//! The blind owner check: one final sound per goal, level-matched to -23 LUFS (peak-limited at -1 dBFS), under
//! shuffled numbers, shown with the brief only. The owner answers "matches the brief?" yes or no, with an optional
//! 1-5; G1 needs at least 7 of 10 yes. The key (which number is which trial) stays in `blind-key.json` in the run
//! directory, and the answers go under `_local/` (never committed). Building the set plays nothing; `listen` plays
//! only when the owner starts it, and `--dry-run` never plays and answers synthetically.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::goal::EvalSet;
use super::run::{Env, Track, TrialResult};
use super::util::{self, Rng, Wav};

/// The loudness every clip is matched to (EBU R128 integrated, as in the other listening tools).
pub const TARGET_LUFS: f64 = -23.0;
/// No clip peaks above this after matching.
pub const PEAK_CEILING_DBFS: f64 = -1.0;

/// One trial per goal: the best passing run with the median renders (a seeded pick among ties), else the run
/// closest to passing (fewest failed items, then the smallest shortfall).
pub fn pick<'a>(rs: &[&'a TrialResult], rng: &mut Rng) -> Option<&'a TrialResult> {
    let with_wav: Vec<&&TrialResult> = rs.iter().filter(|r| r.final_wav.is_some()).collect();
    let mut passing: Vec<&&TrialResult> = with_wav.iter().copied().filter(|r| r.pass).collect();
    if !passing.is_empty() {
        passing.sort_by_key(|r| (r.renders, r.trial));
        let median = passing[(passing.len() - 1) / 2].renders;
        let ties: Vec<&&TrialResult> = passing
            .into_iter()
            .filter(|r| r.renders == median)
            .collect();
        return Some(**ties.get(rng.below(ties.len()))?);
    }
    let shortfall = |r: &TrialResult| -> f64 {
        r.margins
            .iter()
            .filter(|m| !m.pass)
            .map(|m| m.margin_pct.map_or(1.0, |p| p.abs() / 100.0))
            .sum()
    };
    with_wav
        .into_iter()
        .min_by(|a, b| {
            (a.failed.len(), shortfall(a))
                .partial_cmp(&(b.failed.len(), shortfall(b)))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

/// The gain in dB that brings a clip to the target loudness without passing the peak ceiling; `None` when the
/// loudness is not finite (a silent clip).
pub fn gain_db(lufs: f64, peak_dbfs: f64) -> Option<(f64, bool)> {
    if !lufs.is_finite() {
        return None;
    }
    let wanted = TARGET_LUFS - lufs;
    let allowed = if peak_dbfs.is_finite() {
        PEAK_CEILING_DBFS - peak_dbfs
    } else {
        f64::INFINITY
    };
    Some((wanted.min(allowed), wanted > allowed))
}

fn loudness_of(env: &Env, wav: &Path) -> Option<(f64, f64)> {
    let out = Command::new(&env.enth)
        .arg("describe")
        .arg(wav)
        .output()
        .ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let l = v.get("loudness")?;
    Some((l.get("lufs_i")?.as_f64()?, l.get("peak_dbfs")?.as_f64()?))
}

/// Build `<run_dir>/blind/NN.wav` and `<run_dir>/blind-key.json`; returns how many clips.
pub fn build(
    env: &Env,
    set: &EvalSet,
    run_dir: &Path,
    results: &[TrialResult],
    seed: u64,
) -> Result<usize, String> {
    let track = if results.iter().any(|r| r.track == Track::Given) {
        Track::Given
    } else {
        Track::SelfWritten
    };
    let mut goals: Vec<&str> = results.iter().map(|r| r.goal.as_str()).collect();
    goals.sort_unstable();
    goals.dedup();
    let mut rng = Rng(seed);
    let mut picked: Vec<&TrialResult> = Vec::new();
    for g in goals {
        let rs: Vec<&TrialResult> = results
            .iter()
            .filter(|r| r.goal == g && r.track == track)
            .collect();
        if let Some(r) = pick(&rs, &mut rng) {
            picked.push(r);
        }
    }
    if picked.is_empty() {
        return Err("no trial left a WAV".to_owned());
    }
    rng.shuffle(&mut picked);
    let out_dir = run_dir.join("blind");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut key = Vec::new();
    for (i, r) in picked.iter().enumerate() {
        let src = PathBuf::from(r.final_wav.as_deref().unwrap_or_default());
        let bytes = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        let mut wav: Wav =
            util::parse_wav(&bytes).map_err(|e| format!("{}: {e}", src.display()))?;
        let (lufs, peak) = loudness_of(env, &src).unwrap_or((f64::NAN, f64::NAN));
        let (gain, limited) = gain_db(lufs, peak).unwrap_or((0.0, false));
        let k = 10f32.powf(gain as f32 / 20.0);
        for s in &mut wav.samples {
            *s *= k;
        }
        let name = format!("{:02}.wav", i + 1);
        std::fs::write(out_dir.join(&name), util::wav_bytes(&wav)).map_err(|e| e.to_string())?;
        key.push(json!({
            "file": name, "goal": r.goal, "brief": set.goals.iter().find(|g| g.id == r.goal).map(|g| g.brief.as_str()),
            "track": r.track.name(), "trial": r.trial, "pass": r.pass,
            "renders": r.renders, "lufs_before": util::round4(lufs), "peak_before_dbfs": util::round4(peak),
            "gain_db": util::round4(gain), "peak_limited": limited, "wav": r.final_wav,
        }));
    }
    let n = key.len();
    util::write_json(
        &run_dir.join("blind-key.json"),
        &json!({"schema": "enth.agent-eval.blind-key/1", "seed": seed, "target_lufs": TARGET_LUFS,
                "note": "the owner must not open this file before answering", "clips": key}),
    )?;
    Ok(n)
}

/// The clips in presented order with their briefs: the owner sees the text and no numbers.
fn clips_of(key: &Value) -> Vec<(String, String)> {
    key["clips"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    (
                        c["file"].as_str().unwrap_or("?").to_owned(),
                        c["brief"].as_str().unwrap_or("?").to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `agent-eval listen <dir> [--player CMD] [--dry-run]`.
pub fn listen(dir: &Path, root: &Path, player: &str, dry_run: bool) -> Result<(), String> {
    let key_path = dir.join("blind-key.json");
    let text =
        std::fs::read_to_string(&key_path).map_err(|e| format!("{}: {e}", key_path.display()))?;
    let key: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let clips = clips_of(&key);
    let run_name = dir
        .file_name()
        .map_or_else(|| "run".to_owned(), |n| n.to_string_lossy().into_owned());
    let answers_path = if dry_run {
        dir.join("listen-dry-run.json")
    } else {
        let d = root.join("_local/agent-eval");
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        d.join(format!("{run_name}.answers.json"))
    };
    let mut answers: Vec<Value> = std::fs::read_to_string(&answers_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["answers"].as_array().cloned())
        .unwrap_or_default();
    println!(
        "blind listen: {} sounds, level-matched to {TARGET_LUFS} LUFS. Say whether each one matches its text. \
y = matches, n = does not (add 1-5 for how well, e.g. `y 4`), r = replay, q = quit (answers are saved).{}",
        clips.len(),
        if dry_run {
            "\nDRY RUN: nothing is played and the answers are synthetic."
        } else {
            ""
        }
    );
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    for (i, (file, brief)) in clips.iter().enumerate() {
        if answers.iter().any(|a| a["file"] == file.as_str()) {
            continue;
        }
        let wav = dir.join("blind").join(file);
        println!("\n{} of {}: \"{brief}\"", i + 1, clips.len());
        let answer = if dry_run {
            json!({"file": file, "matches": i % 3 != 0, "rating": 3, "synthetic": true})
        } else {
            let mut parsed: Option<(bool, Option<u8>)> = None;
            play(player, &wav)?;
            while parsed.is_none() {
                println!("matches? [y/n 1-5, r replay, q quit]");
                let Some(Ok(line)) = lines.next() else {
                    return finish(&answers, &answers_path, clips.len(), dry_run);
                };
                let mut words = line.split_whitespace();
                match words.next() {
                    Some("y" | "Y") => {
                        parsed = Some((true, words.next().and_then(|r| r.parse().ok())));
                    }
                    Some("n" | "N") => {
                        parsed = Some((false, words.next().and_then(|r| r.parse().ok())));
                    }
                    Some("r" | "R") => play(player, &wav)?,
                    Some("q" | "Q") => {
                        return finish(&answers, &answers_path, clips.len(), dry_run);
                    }
                    _ => {}
                }
            }
            let (matches, rating) = parsed.unwrap_or((false, None));
            json!({"file": file, "matches": matches, "rating": rating})
        };
        answers.push(answer);
        save(&answers, &answers_path, dry_run)?;
    }
    finish(&answers, &answers_path, clips.len(), dry_run)
}

fn play(player: &str, wav: &Path) -> Result<(), String> {
    let mut parts = player.split_whitespace();
    let program = parts.next().ok_or("empty --player")?;
    let status = Command::new(program)
        .args(parts)
        .arg(wav)
        .status()
        .map_err(|e| format!("cannot run `{program}`: {e}. Use --player to name another player"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{player}` failed on {}", wav.display()))
    }
}

fn save(answers: &[Value], path: &Path, dry_run: bool) -> Result<(), String> {
    util::write_json(
        path,
        &json!({"schema": "enth.agent-eval.answers/1", "synthetic": dry_run, "answers": answers}),
    )
}

fn finish(answers: &[Value], path: &Path, total: usize, dry_run: bool) -> Result<(), String> {
    save(answers, path, dry_run)?;
    let yes = answers.iter().filter(|a| a["matches"] == true).count();
    println!(
        "\n{yes} of {} answered match their text ({} sounds in the set). G1 needs at least 7 of 10.{} Answers: {}",
        answers.len(),
        total,
        if dry_run {
            " (SYNTHETIC: a dry run says nothing about the sounds.)"
        } else {
            ""
        },
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_eval::run::{Margin, TokensOut};

    fn trial(trial: u32, pass: bool, renders: u32, failed: usize, margin_pct: f64) -> TrialResult {
        TrialResult {
            schema: String::new(),
            eval_version: 1,
            goal: "g".into(),
            track: Track::Given,
            trial,
            runner: "x".into(),
            model: None,
            date: String::new(),
            claude_code_version: None,
            status: "ok".into(),
            tokens: TokensOut::default(),
            cost_usd: None,
            cost_is_estimate: true,
            turns: None,
            wall_s: 0.0,
            calls: 0,
            invalid_calls: 0,
            refused_calls: 0,
            renders,
            pass,
            thin: 0,
            margins: (0..failed)
                .map(|_| Margin {
                    expect: "e".into(),
                    pass: false,
                    got: Value::Null,
                    margin: None,
                    margin_pct: Some(margin_pct),
                    unit: None,
                    thin: false,
                })
                .collect(),
            failed: vec!["e".into(); failed],
            own_expect: None,
            own_pass: None,
            own_error: None,
            final_patch: None,
            final_wav: Some(format!("w{trial}.wav")),
            patch_source: String::new(),
            produced_by_enth: true,
            dir: String::new(),
            warnings: vec![],
        }
    }

    #[test]
    fn the_pick_prefers_the_passing_run_with_median_renders() {
        let t = [
            trial(1, true, 3, 0, 0.0),
            trial(2, true, 9, 0, 0.0),
            trial(3, true, 5, 0, 0.0),
            trial(4, false, 2, 1, 5.0),
        ];
        let refs: Vec<&TrialResult> = t.iter().collect();
        assert_eq!(pick(&refs, &mut Rng(1)).unwrap().trial, 3);
    }

    #[test]
    fn with_no_passing_run_the_closest_to_passing_is_picked() {
        let t = [
            trial(1, false, 20, 2, 10.0),
            trial(2, false, 20, 1, 40.0),
            trial(3, false, 20, 1, 5.0),
        ];
        let refs: Vec<&TrialResult> = t.iter().collect();
        assert_eq!(pick(&refs, &mut Rng(1)).unwrap().trial, 3);
        assert!(pick(&[], &mut Rng(1)).is_none());
    }

    #[test]
    fn gain_matches_loudness_and_respects_the_peak_ceiling() {
        let (g, limited) = gain_db(-30.0, -12.0).unwrap();
        assert_eq!((g, limited), (7.0, false));
        let (g, limited) = gain_db(-30.0, -3.0).unwrap();
        assert_eq!((g, limited), (2.0, true));
        assert!(gain_db(f64::NEG_INFINITY, 0.0).is_none());
    }
}
