//! The design: score each test pair's ladder of step counts with `sound/1`, keep the one whose distance lands in the
//! planned bin, measure the checks, and render every clip level-matched as the game's A/B is.
//!
//! **Choosing a step count.** For a test pair the design scores the answer at every [`LADDER`] step count of its
//! move (17 scores of about 40 ms after one `prepare` of 0.3–2 s) and keeps the count whose JND distance is inside the
//! planned bin, closest to the bin's centre in log distance (the smaller count on a tie). When no count lands in the
//! bin (a `one` or `pitch` move whose parameter barely matters, or saturates), the `all` move's ladder is scored too
//! and used if one of its counts lands; otherwise the closest count of either is kept and the pair's bin is where it
//! landed. Renders are bit-identical on every platform, so the choice is the same on every machine for one engine
//! version; a DSP change can move it, which is why the plan, not the choice, is pinned.
//!
//! **Level matching.** Both sounds are rendered through the target's phrase at the scoring rate and seed, in stereo.
//! The game's A/B match (`enth_challenge::loudness::ab_gains_db` on each side's 48 kHz mid channel, through the
//! in-house BS.1770 meter) brings the louder one down to the quieter one; one more gain, the same for both, puts the
//! pair at −23 LUFS, so every trial plays at one level. Then a 10 ms fade-out, and if either clip would peak above
//! [`PEAK_CEILING`], both are scaled down together.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use enth_challenge::calibrate::{self, Curve};
use enth_challenge::model::{DEFAULT_PASS_MARK, SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::random::{self, Generated};
use enth_challenge::score::sound::{GUARD_PITCH_CT, GUARD_WAVE, SoundPrepared, SoundV1};
use enth_challenge::{
    Challenge, ChallengeCode, ComponentKind, Phrase, RandomSpec, Scorer, ScorerId, TargetContext,
    loudness,
};
use enth_patch::Patch;
use enth_render::{RenderSpec, render};

use super::perturb::answer_patch;
use super::plan::{Plan, PlannedPair};
use super::{
    BIN_CENTRE, BIN_LOWER, ClipInfo, Config, FADE_S, LADDER, Manifest, Measured, MoveKind,
    PEAK_CEILING, Pair, PairRole, SCHEMA_MANIFEST, bin_of,
};
use crate::flags;
use crate::wav::{Wav, encode};

/// A finished design: the manifest, and what the owner should know about it.
#[derive(Clone, Debug)]
pub struct Designed {
    /// The key to every trial.
    pub manifest: Manifest,
    /// Pairs that landed outside their planned bin, checks that are not what they should be.
    pub warnings: Vec<String>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The target of `spec`, and `sound/1` prepared on it (the same context the game's round builds).
fn prepare(spec: &RandomSpec) -> Result<(Generated, SoundPrepared), String> {
    let start = random::start(spec.template).map_err(err)?;
    let g = random::generate(spec, &start).map_err(err)?;
    let challenge = Challenge::recreate(*spec, g.phrase);
    let ctx = TargetContext {
        challenge: &challenge,
        target: &g.patch,
        start: &start,
        phrase: g.phrase(),
        randomized: &g.params,
    };
    let prepared = SoundV1.prepare(&ctx).map_err(err)?;
    if prepared.target_clip().lufs.is_none() {
        return Err(format!(
            "the target of seed {} (level spec {:?}) is silent; use another --seed",
            spec.seed, spec.difficulty
        ));
    }
    Ok((g, prepared))
}

/// What `sound/1` says about `answer` against its prepared target, with the JND distance split into the metric's part
/// and the two guard terms.
///
/// # Errors
/// The render or analysis failed.
pub fn measure_pair(prepared: &SoundPrepared, answer: &Patch) -> Result<Measured, String> {
    let s = SoundV1.score(prepared, answer).map_err(err)?;
    let reading = |k: ComponentKind| {
        let c = s.components.iter().find(|c| c.kind == k)?;
        Some((c.answer? - c.target?).abs())
    };
    let (pitch_floor_ct, odd_floor) = prepared.guard_floors();
    let pitch_delta_ct = reading(ComponentKind::Pitch);
    let odd_delta = reading(ComponentKind::Harmonics);
    Ok(Measured {
        distance: s.distance,
        floor: s.floor,
        scale: s.scale,
        z_metric: calibrate::jnd(s.distance, s.floor, s.scale),
        pitch_term: pitch_delta_ct.map_or(0.0, |d| (d - pitch_floor_ct).max(0.0) / GUARD_PITCH_CT),
        wave_term: odd_delta.map_or(0.0, |d| (d - odd_floor).max(0.0) / GUARD_WAVE),
        jnd: s.jnd,
        percent: s.percent,
        passes: s.percent >= f64::from(DEFAULT_PASS_MARK),
        pitch_delta_ct,
        pitch_floor_ct,
        odd_delta,
        odd_floor,
    })
}

fn log_gap(jnd: f64, centre: f64) -> f64 {
    (libm::log(jnd.max(1e-3)) - libm::log(centre)).abs()
}

/// The ladder entry to keep for `bin`: inside the bin and closest to its centre (the smaller step count on a tie);
/// `None` when no entry is inside.
#[must_use]
pub fn choose(ladder: &[f64], bin: usize) -> Option<usize> {
    let centre = BIN_CENTRE[bin];
    ladder
        .iter()
        .enumerate()
        .filter(|(_, z)| bin_of(**z) == bin)
        .min_by(|(i, a), (j, b)| {
            log_gap(**a, centre)
                .total_cmp(&log_gap(**b, centre))
                .then(i.cmp(j))
        })
        .map(|(i, _)| i)
}

/// The ladder entry closest to `bin`'s centre, inside or not.
fn closest(ladder: &[f64], bin: usize) -> usize {
    let centre = BIN_CENTRE[bin];
    ladder
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| {
            log_gap(**a, centre)
                .total_cmp(&log_gap(**b, centre))
                .then(i.cmp(j))
        })
        .map_or(0, |(i, _)| i)
}

fn ladder(
    g: &Generated,
    prepared: &SoundPrepared,
    kind: MoveKind,
    param: Option<&str>,
    sign_seed: u64,
) -> Result<Vec<Measured>, String> {
    LADDER
        .iter()
        .map(|m| {
            measure_pair(
                prepared,
                &answer_patch(&g.patch, g.phrase(), kind, param, sign_seed, *m),
            )
        })
        .collect()
}

fn short_code(spec: &RandomSpec) -> Option<String> {
    ChallengeCode::short(*spec).map(|c| c.encode())
}

/// The target and the answer of a designed pair, and the phrase both play: what its clips are rendered from.
///
/// # Errors
/// The generator refused a spec (never for a plan's specs).
pub fn sounds(pair: &Pair) -> Result<(Patch, Patch, &'static Phrase), String> {
    let p = &pair.plan;
    let g = random::target(&p.target).map_err(err)?;
    let answer = match p.role {
        PairRole::Same => g.patch.clone(),
        PairRole::Different => {
            let other = p
                .other
                .ok_or("an obviously different pair has a second spec")?;
            random::target(&other).map_err(err)?.patch
        }
        PairRole::Test => answer_patch(
            &g.patch,
            g.phrase(),
            pair.kind.unwrap_or(MoveKind::All),
            pair.moved_param(),
            p.sign_seed,
            pair.steps.unwrap_or(0.0),
        ),
    };
    Ok((g.patch.clone(), answer, g.phrase()))
}

fn design_pair(p: &PlannedPair) -> Result<(Pair, Vec<String>), String> {
    let (g, prepared) = prepare(&p.target)?;
    let mut warnings = Vec::new();
    let code = short_code(&p.target);
    let (kind, steps, ladder_jnd, measured) = match p.role {
        PairRole::Same => (None, None, Vec::new(), measure_pair(&prepared, &g.patch)?),
        PairRole::Different => {
            let other = p
                .other
                .ok_or("an obviously different pair has a second spec")?;
            let o = random::target(&other).map_err(err)?;
            let m = measure_pair(&prepared, &o.patch)?;
            if m.jnd < BIN_LOWER[BIN_LOWER.len() - 1] {
                warnings.push(format!(
                    "pair {}: sound/1 puts the obviously different check only {:.1} JND apart",
                    p.index, m.jnd
                ));
            }
            (None, None, Vec::new(), m)
        }
        PairRole::Test => {
            let planned = p.kind.unwrap_or(MoveKind::All);
            let bin = p.planned_bin.unwrap_or(0);
            let first = ladder(&g, &prepared, planned, p.param.as_deref(), p.sign_seed)?;
            let zs: Vec<f64> = first.iter().map(|m| m.jnd).collect();
            let mut used = (planned, choose(&zs, bin), first, zs);
            if used.1.is_none() && planned != MoveKind::All {
                let all = ladder(&g, &prepared, MoveKind::All, None, p.sign_seed)?;
                let zs_all: Vec<f64> = all.iter().map(|m| m.jnd).collect();
                let mut both = used.3.clone();
                both.extend_from_slice(&zs_all);
                if let Some(i) = choose(&zs_all, bin) {
                    used = (MoveKind::All, Some(i), all, both);
                } else {
                    let (ci, ca) = (closest(&used.3, bin), closest(&zs_all, bin));
                    let centre = BIN_CENTRE[bin];
                    used = if log_gap(zs_all[ca], centre) < log_gap(used.3[ci], centre) {
                        (MoveKind::All, Some(ca), all, both)
                    } else {
                        (planned, Some(ci), used.2, both)
                    };
                }
            }
            let (kind, idx, measured, ladder_jnd) = used;
            let idx = idx.unwrap_or_else(|| closest(&ladder_jnd[..LADDER.len()], bin));
            let m = measured[idx].clone();
            if bin_of(m.jnd) != bin {
                warnings.push(format!(
                    "pair {}: planned bin {bin}, landed in bin {} ({:.2} JND)",
                    p.index,
                    bin_of(m.jnd),
                    m.jnd
                ));
            }
            (Some(kind), Some(LADDER[idx]), ladder_jnd, m)
        }
    };
    let bin = bin_of(measured.jnd);
    Ok((
        Pair {
            plan: p.clone(),
            code,
            other_code: p.other.as_ref().and_then(short_code),
            kind,
            steps,
            ladder_jnd,
            measured,
            bin,
        },
        warnings,
    ))
}

fn render_stereo(patch: &Patch, phrase: &Phrase) -> Result<(Vec<f32>, Vec<f32>), String> {
    let out = render(
        patch,
        &phrase.performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
    )
    .map_err(err)?;
    Ok((out.left, out.right))
}

/// The two clips of a pair as the listener hears them (target first), and their clip records without ids. See the
/// [module docs](self) for the level matching. An identical pair is the target twice.
///
/// # Errors
/// A render failed.
pub fn pair_audio(
    target: &Patch,
    answer: &Patch,
    phrase: &Phrase,
) -> Result<[(Wav, ClipInfo); 2], String> {
    let sr = SCORING_SAMPLE_RATE;
    let raw = [
        render_stereo(target, phrase)?,
        render_stereo(answer, phrase)?,
    ];
    let lufs: Vec<Option<f64>> = raw
        .iter()
        .map(|(l, r)| {
            let mid: Vec<f32> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
            loudness::integrated_lufs(&mid, sr)
        })
        .collect();
    let (gt, ga) = loudness::ab_gains_db(lufs[0], lufs[1]);
    let quieter = match (lufs[0], lufs[1]) {
        (Some(t), Some(a)) => Some(t.min(a)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    };
    let offset = quieter.map_or(0.0, |q| loudness::gain_db(q, loudness::SCORING_LUFS));
    let mut gains = [gt + offset, ga + offset];
    let fade = ((f64::from(sr) * f64::from(FADE_S)) as usize).max(1);
    let mut wavs: Vec<Wav> = raw
        .into_iter()
        .zip(gains)
        .map(|((l, r), g)| {
            let mut w = Wav::stereo(sr, l, r);
            w.scale(loudness::db_to_amp(g) as f32);
            let n = w.frames();
            for c in &mut w.channels {
                for (i, v) in c.iter_mut().enumerate() {
                    *v *= ((n - 1 - i) as f32 / fade as f32).min(1.0);
                }
            }
            w
        })
        .collect();
    let peak = wavs.iter().map(Wav::peak).fold(0.0f32, f32::max);
    if peak > PEAK_CEILING {
        let k = PEAK_CEILING / peak;
        for w in &mut wavs {
            w.scale(k);
        }
        for g in &mut gains {
            *g += 20.0 * libm::log10(f64::from(k));
        }
    }
    let mut out = wavs
        .into_iter()
        .zip(gains)
        .zip(lufs)
        .enumerate()
        .map(|(i, ((w, g), l))| {
            let refs: Vec<&[f32]> = w.channels.iter().map(Vec::as_slice).collect();
            let info = ClipInfo {
                id: String::new(),
                pair: 0,
                target: i == 0,
                raw_lufs: l,
                gain_db: g,
                peak_dbfs: 20.0 * libm::log10(f64::from(w.peak().max(1e-9))),
                seconds: w.seconds(),
                flags: flags::check(&refs, sr),
            };
            (w, info)
        });
    let a = out.next().ok_or("two clips")?;
    let b = out.next().ok_or("two clips")?;
    Ok([a, b])
}

/// Render a designed pair's clips into `clips_dir` (only the missing ones unless `all`), and return their records.
///
/// # Errors
/// A render or a file write failed.
pub fn write_clips(pair: &Pair, clips_dir: &Path, all: bool) -> Result<Vec<ClipInfo>, String> {
    let p = &pair.plan;
    let ids = if p.role == PairRole::Same {
        vec![p.target_clip.clone()]
    } else {
        vec![p.target_clip.clone(), p.answer_clip.clone()]
    };
    let missing = ids
        .iter()
        .any(|id| !clips_dir.join(format!("{id}.wav")).exists());
    if !all && !missing {
        return Ok(Vec::new());
    }
    let (target, answer, phrase) = sounds(pair)?;
    let clips = pair_audio(&target, &answer, phrase)?;
    let mut infos = Vec::new();
    for (id, (wav, mut info)) in ids.iter().zip(clips) {
        info.id.clone_from(id);
        info.pair = p.index;
        let path = clips_dir.join(format!("{id}.wav"));
        if all || !path.exists() {
            std::fs::write(&path, encode(&wav)).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        infos.push(info);
    }
    Ok(infos)
}

/// Run `f` on `0..n` on up to `workers` threads; results in index order, the same for any worker count.
fn par_map<T: Send>(n: usize, workers: usize, f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers.clamp(1, n.max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut got = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        got.push((i, f(i)));
                    }
                    got
                })
            })
            .collect();
        for h in handles {
            for (i, v) in h.join().expect("a design worker panicked") {
                out[i] = Some(v);
            }
        }
    });
    out.into_iter()
        .map(|v| v.expect("every index ran"))
        .collect()
}

/// Design every pair of `plan` on `workers` threads and write the clips under `clips_dir`.
///
/// # Errors
/// A render, an analysis or a file write failed, or a target is silent.
pub fn design(plan: &Plan, cfg: &Config, clips_dir: &Path) -> Result<Designed, String> {
    let started = Instant::now();
    std::fs::create_dir_all(clips_dir).map_err(|e| format!("{}: {e}", clips_dir.display()))?;
    let done = par_map(plan.pairs.len(), cfg.workers, &|i| -> Result<_, String> {
        let (pair, warnings) = design_pair(&plan.pairs[i])?;
        let clips = write_clips(&pair, clips_dir, true)?;
        Ok((pair, warnings, clips))
    });
    let mut pairs = Vec::new();
    let mut clips = Vec::new();
    let mut warnings = Vec::new();
    for r in done {
        let (p, w, c) = r?;
        pairs.push(p);
        warnings.extend(w);
        clips.extend(c);
    }
    for c in &clips {
        if !c.flags.is_empty() {
            warnings.push(format!(
                "clip {} (pair {}): {} (kept)",
                c.id,
                c.pair,
                flags::summary(&c.flags)
            ));
        }
    }
    let curve = Curve::SOUND_V1;
    Ok(Designed {
        manifest: Manifest {
            schema: SCHEMA_MANIFEST.to_string(),
            config: cfg.clone(),
            plan_hash: plan.hash(),
            engine: enth_patch::file::ENGINE_VERSION.to_string(),
            scorer: ScorerId::SOUND_V1.to_string(),
            curve_z50: curve.z50,
            curve_k: curve.k,
            pass_mark: f64::from(DEFAULT_PASS_MARK),
            host_threads: crate::files::host_threads(),
            design_wall_s: started.elapsed().as_secs_f64(),
            pairs,
            clips,
            trials: plan.trials.clone(),
        },
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_keeps_the_in_bin_entry_nearest_the_centre() {
        let ladder = [0.1, 0.4, 0.6, 0.9, 1.3, 1.9, 3.0];
        assert_eq!(choose(&ladder, 0), Some(1)); // 0.4 is nearer 0.3 than 0.1 in log distance
        assert_eq!(choose(&ladder, 1), Some(2)); // 0.6 and 0.9 against 0.7
        assert_eq!(choose(&ladder, 2), Some(4));
        assert_eq!(choose(&ladder, 3), Some(6));
        assert_eq!(choose(&ladder, 5), None);
        assert_eq!(closest(&ladder, 5), 6);
        assert_eq!(
            choose(&[0.0, 0.0], 0),
            Some(0),
            "ties go to the smaller count"
        );
    }

    #[test]
    fn par_map_keeps_the_order_for_any_worker_count() {
        for w in [1, 2, 5, 40] {
            assert_eq!(par_map(7, w, &|i| i * i), vec![0, 1, 4, 9, 16, 25, 36]);
        }
        assert_eq!(par_map(0, 3, &|i| i), Vec::<usize>::new());
    }
}
