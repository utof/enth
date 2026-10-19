//! `listen`: the owner's listening test, and its `--dry-run`.
//!
//! Workflow (about 2 x 15 minutes, ideally on different days):
//!   spike-metric listen --dir ~/enth-listen --session 1
//!   spike-metric listen --dir ~/enth-listen --session 2
//!   spike-metric listen --dir ~/enth-listen --analyze
//! `--dry-run` does all of it in `DIR/dry-run` with synthetic answers and never plays audio.

use crate::util::{Res, host_threads, read_json, write_json};
use clap::Args as ClapArgs;
use spike_metric::distance::{Dist, Scale};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::listen::{
    Answer, AnswerFile, DesignCfg, Manifest, SCHEMA_MANIFEST, analyze, design, stimulus,
    synthetic_answers,
};
use spike_metric::par::ParEval;
use spike_metric::render::RenderCfg;
use spike_metric::wav::encode_wav_i16;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(ClapArgs)]
pub struct Args {
    /// Where the stimuli, manifest and answers live.
    #[arg(long, default_value = "listen-test")]
    dir: PathBuf,
    /// Rate one session (1 or 2) interactively, playing each pair with `--player`.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    session: Option<u8>,
    /// Compute Kendall tau-b and Spearman (with CIs) per distance from the answer files.
    #[arg(long)]
    analyze: bool,
    /// Prepare stimuli in DIR/dry-run, answer both sessions with synthetic ratings, analyse. Plays nothing.
    #[arg(long)]
    dry_run: bool,
    /// Design seed. A directory keeps the seed it was prepared with.
    #[arg(long, default_value_t = 20_261_019)]
    seed: u64,
    /// Candidate pairs generated before the distance-A quantile binning.
    #[arg(long, default_value_t = 600)]
    pool: usize,
    /// Threads for the design step (0 = up to 8).
    #[arg(long, default_value_t = 0)]
    workers: usize,
    /// Command that plays one WAV file.
    #[arg(long, default_value = "pw-play")]
    player: String,
    #[arg(long, default_value_t = 2000)]
    bootstrap: usize,
    /// 0.5 s clips instead of 2 s (for tests).
    #[arg(long, hide = true)]
    quick: bool,
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

fn answers_path(dir: &Path, session: u8) -> PathBuf {
    dir.join(format!("answers-session{session}.json"))
}

fn wav_path(dir: &Path, session: u8, id: &str) -> PathBuf {
    dir.join(format!("session{session}"))
        .join(format!("{id}.wav"))
}

/// Design (or load) the manifest and make sure every WAV exists.
fn prepare(dir: &Path, args: &Args) -> Res<Manifest> {
    std::fs::create_dir_all(dir)?;
    let render = if args.quick {
        RenderCfg::short()
    } else {
        RenderCfg::default()
    };
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let mut an = Analyzer::new(analysis);
    let mp = manifest_path(dir);
    let manifest: Manifest = if mp.exists() {
        let m: Manifest = read_json(&mp)?;
        if m.schema != SCHEMA_MANIFEST {
            return Err(format!(
                "{} has schema {}, expected {SCHEMA_MANIFEST}",
                mp.display(),
                m.schema
            )
            .into());
        }
        println!(
            "using the stimuli designed earlier in {} (seed {})",
            dir.display(),
            m.seed
        );
        m
    } else {
        let workers = if args.workers > 0 {
            args.workers
        } else {
            host_threads().min(8)
        };
        let scale = Scale::calibrate(&mut an, &render, 42, 50);
        let pe = ParEval::new(workers, render, analysis, Dist::A, scale);
        let cfg = DesignCfg {
            seed: args.seed,
            pool: args.pool,
            ..DesignCfg::default()
        };
        println!(
            "designing {} pairs from a pool of {} on {workers} threads ...",
            cfg.n_test, cfg.pool
        );
        let t = Instant::now();
        let m = design(&cfg, &render, &scale, analysis.target_lufs, &|pairs| {
            pe.pair_components(pairs)
        });
        println!("designed in {:.1} s", t.elapsed().as_secs_f64());
        write_json(&mp, &m)?;
        m
    };
    for s in 1..=2u8 {
        std::fs::create_dir_all(dir.join(format!("session{s}")))?;
        for t in &manifest.sessions[usize::from(s) - 1] {
            let p = wav_path(dir, s, &t.id);
            if !p.exists() {
                let audio = stimulus(&mut an, &manifest.render, &manifest.pairs[t.pair], t.swap);
                std::fs::write(
                    &p,
                    encode_wav_i16(&audio, manifest.render.sample_rate as u32),
                )?;
            }
        }
    }
    Ok(manifest)
}

fn load_answers(dir: &Path, session: u8) -> Res<Option<AnswerFile>> {
    let p = answers_path(dir, session);
    if !p.exists() {
        return Ok(None);
    }
    let f: AnswerFile = read_json(&p)?;
    if f.session != session {
        return Err(format!("{} is for session {}", p.display(), f.session).into());
    }
    Ok(Some(f))
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    match std::io::stdin().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim().to_lowercase()),
    }
}

fn run_session(dir: &Path, manifest: &Manifest, session: u8, player: &str) -> Res<()> {
    let trials = &manifest.sessions[usize::from(session) - 1];
    let mut file = load_answers(dir, session)?.unwrap_or_else(|| AnswerFile::new(session, false));
    if file.synthetic {
        return Err(format!(
            "{} holds synthetic answers; use a different --dir",
            answers_path(dir, session).display()
        )
        .into());
    }
    let done = file.answers.len();
    println!();
    println!(
        "Listening test, session {session} of 2: {} trials, about 15 minutes.",
        trials.len()
    );
    println!(
        "  - Use headphones. Set the volume once and do not touch it again, in this session or the next."
    );
    println!("  - Each trial plays two sounds (about 4.5 s). Rate how similar they sound:");
    println!("      1 = very different   2   3   4   5 = indistinguishable");
    println!(
        "  - r = replay the pair, q = quit (progress is saved; run the same command to resume)."
    );
    println!(
        "  - Do not open manifest.json: it holds the metric's distances and would unblind you."
    );
    if done > 0 {
        println!("  - Resuming after {done} answered trials.");
    }
    for t in trials {
        if file.answers.iter().any(|a| a.id == t.id) {
            continue;
        }
        let path = wav_path(dir, session, &t.id);
        let started = Instant::now();
        let mut replays = 0u32;
        loop {
            println!();
            println!("[{}/{}] playing ...", t.n, trials.len());
            let mut cmd = Command::new(player);
            if player == "pw-play" {
                // pw-play defaults to media.role "Music", and PipeWire's stream-restore may keep that role muted
                // (it did on the owner's machine). A role of our own starts unmuted and touches no system setting.
                cmd.arg("--media-role=Production");
            }
            let status = cmd
                .arg(&path)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| format!("cannot run {player:?}: {e} (set --player to a command that plays one WAV file)"))?;
            if !status.success() {
                return Err(format!("{player} exited with {status}").into());
            }
            let rating = loop {
                print!("rating 1-5 (r = replay, q = quit): ");
                std::io::stdout().flush()?;
                match read_line().as_deref() {
                    None | Some("q") => {
                        println!(
                            "saved {} answers; run the same command to resume.",
                            file.answers.len()
                        );
                        return Ok(());
                    }
                    Some("r") => {
                        replays += 1;
                        break None;
                    }
                    Some(s @ ("1" | "2" | "3" | "4" | "5")) => break s.parse::<u8>().ok(),
                    Some(_) => println!("type 1, 2, 3, 4, 5, r or q"),
                }
            };
            if let Some(rating) = rating {
                file.answers.push(Answer {
                    n: t.n,
                    id: t.id.clone(),
                    rating,
                    replays,
                    ms: started.elapsed().as_millis() as u64,
                });
                write_json(&answers_path(dir, session), &file)?;
                break;
            }
        }
    }
    println!();
    println!(
        "Session {session} done: {} answers saved to {}.",
        file.answers.len(),
        answers_path(dir, session).display()
    );
    if session == 1 {
        println!("Next: session 2 (same command with --session 2), ideally on another day.");
    } else {
        println!("Next: analyse with --analyze.");
    }
    Ok(())
}

fn run_analysis(dir: &Path, manifest: &Manifest, bootstrap: usize) -> Res<()> {
    let mut files = Vec::new();
    for s in 1..=2u8 {
        if let Some(f) = load_answers(dir, s)? {
            println!(
                "session {s}: {} of {} answers",
                f.answers.len(),
                manifest.sessions[usize::from(s) - 1].len()
            );
            files.push(f);
        }
    }
    if files.is_empty() {
        return Err("no answer files yet; run --session 1 first".into());
    }
    let report = analyze(manifest, &files, bootstrap, manifest.seed ^ 0xb007);
    print!("{}", report.to_text());
    let out = dir.join("listen-report.json");
    write_json(&out, &report)?;
    println!("wrote {}", out.display());
    Ok(())
}

pub fn run(args: &Args) -> Res<()> {
    let dir = if args.dry_run {
        args.dir.join("dry-run")
    } else {
        args.dir.clone()
    };
    let manifest = prepare(&dir, args)?;
    println!(
        "{} trials per session ({} test pairs + {} anchors), stimuli in {}",
        manifest.sessions[0].len(),
        manifest.pairs.iter().filter(|p| p.bin.is_some()).count(),
        manifest.pairs.iter().filter(|p| p.bin.is_none()).count(),
        dir.display()
    );
    if args.dry_run {
        println!(
            "dry run: nothing is played; both sessions are answered with synthetic ratings derived from the distances"
        );
        for s in 1..=2u8 {
            write_json(
                &answers_path(&dir, s),
                &synthetic_answers(&manifest, s, manifest.seed),
            )?;
        }
        return run_analysis(&dir, &manifest, args.bootstrap);
    }
    if let Some(s) = args.session {
        run_session(&dir, &manifest, s, &args.player)?;
    }
    if args.analyze {
        run_analysis(&dir, &manifest, args.bootstrap)?;
    }
    if args.session.is_none() && !args.analyze {
        println!(
            "prepared. Next: `spike-metric listen --dir {} --session 1`",
            dir.display()
        );
    }
    Ok(())
}
