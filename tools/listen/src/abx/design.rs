//! The design step: solve the targets, build the clips, plan the sessions.

use std::collections::HashSet;
use std::io::Write;
use std::time::Instant;

use enth_analysis::loudness::LoudnessMeter;
use spike_metric::distance::{Dist, Scale, analysis_for, median};
use spike_metric::features::Analyzer;
use spike_metric::heuristic::heuristic;
use spike_metric::par::ParEval;
use spike_metric::render::{N_PARAMS, RenderCfg, Wave, render_unit};
use spike_metric::rng::Rng;
use spike_metric::search::{Restarts, Search, SearchCfg, drive};

use super::{
    ClipInfo, Config, Item, Manifest, Role, SCHEMA_MANIFEST, Side, StopAt, Trial, TrialKind,
};
use crate::Res;
use crate::files::{host_threads, load_avg};
use crate::flags;
use crate::loudness::{PEAK_CEILING, TARGET_LUFS, db_to_gain, peak_dbfs};
use crate::wav::Wav;

/// A finished design: the manifest and the clip audio (written to `clips/` by the caller).
#[derive(Debug)]
pub struct Designed {
    /// The key.
    pub manifest: Manifest,
    /// `(clip id, audio)`.
    pub clips: Vec<(String, Wav)>,
}

/// `(total evaluations, seconds)` at which the best distance first crossed a threshold.
type Milestone = Option<(usize, f64)>;

/// A rendered clip before the level match.
struct Draft {
    id: String,
    item: usize,
    role: Role,
    lufs: Option<f64>,
    peak: f32,
    audio: Vec<f32>,
}

/// The fade-out at the end of every clip, so a long release is not cut with a click. No fade-in: the toy voice
/// starts at exactly zero, and a fade-in would cut the transient of a percussive sound.
const FADE_S: f32 = 0.01;

/// The spike's seeded target list: random patches that are audible (`solve.rs`, `make_targets`).
fn make_targets(n: usize, cfg: &RenderCfg, seed: u64) -> Vec<Vec<f64>> {
    let mut buf = Vec::new();
    let mut v = Vec::new();
    let mut rng = Rng::new(seed);
    while v.len() < n {
        let p: Vec<f64> = (0..N_PARAMS).map(|_| rng.uniform()).collect();
        render_unit(cfg, &p, &mut buf);
        let rms = (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt();
        if rms > 1e-3 {
            v.push(p);
        }
    }
    v
}

/// T1 and T3 of distance A for one target: the medians over 16 renders with every continuous parameter jittered by
/// +-1 % and 16 by +-3 % of its range (the spike's `thresholds`, for A only; the draws are the same).
fn thresholds_a(pe: &ParEval, tp: &[f64], ti: usize, scale: &Scale) -> (f64, f64) {
    let mut rng = Rng::new(ti as u64 * 7919 + 1);
    let batch: Vec<Vec<f64>> = (0..32)
        .map(|rep| {
            let pert = if rep < 16 { 0.01 } else { 0.03 };
            tp.iter()
                .enumerate()
                .map(|(i, v)| {
                    if i == 0 {
                        *v
                    } else {
                        (v + pert * (2.0 * rng.uniform() - 1.0)).clamp(0.0, 1.0)
                    }
                })
                .collect()
        })
        .collect();
    let comps = pe.components(&batch);
    let mut d1: Vec<f64> = comps[..16]
        .iter()
        .map(|c| c.distance(Dist::A, scale))
        .collect();
    let mut d3: Vec<f64> = comps[16..]
        .iter()
        .map(|c| c.distance(Dist::A, scale))
        .collect();
    (median(&mut d1), median(&mut d3))
}

/// A clip as rendered: the toy synth, then the fade-out.
pub(super) fn raw_clip(render: &RenderCfg, params: &[f64]) -> Vec<f32> {
    let mut buf = Vec::new();
    render_unit(render, params, &mut buf);
    let fade = ((render.sample_rate * FADE_S) as usize).max(1);
    let n = buf.len();
    for (i, v) in buf.iter_mut().enumerate() {
        *v *= ((n - 1 - i) as f32 / fade as f32).min(1.0);
    }
    buf
}

/// The two most different pairs of targets (unequal waveforms, no target twice): the obviously-different anchors.
fn different_pairs(
    pe: &ParEval,
    targets: &[&[f64]],
    want: usize,
    scale: &Scale,
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for i in 0..targets.len() {
        for j in i + 1..targets.len() {
            pairs.push((targets[i].to_vec(), targets[j].to_vec()));
        }
    }
    let comps = pe.pair_components(&pairs);
    let mut order: Vec<(usize, usize, f64)> = Vec::new();
    let mut k = 0;
    for i in 0..targets.len() {
        for j in i + 1..targets.len() {
            order.push((i, j, comps[k].distance(Dist::A, scale)));
            k += 1;
        }
    }
    order.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, j, d) in order {
        if out.len() == want || d >= spike_metric::distance::SILENT {
            break;
        }
        let waves = Wave::from_unit(targets[i][0]) != Wave::from_unit(targets[j][0]);
        let fresh = out
            .iter()
            .all(|(a, b)| *a != i && *a != j && *b != i && *b != j);
        if waves && fresh {
            out.push((i, j));
        }
    }
    out
}

fn unique_id(rng: &mut Rng, prefix: &str, used: &mut HashSet<String>) -> String {
    loop {
        let id = format!("{prefix}{:06x}", rng.next_u64() & 0xff_ffff);
        if used.insert(id.clone()) {
            return id;
        }
    }
}

/// `n` booleans, as many true as false (the extra one random), in random order.
fn balanced(rng: &mut Rng, n: usize) -> Vec<bool> {
    let flip = rng.uniform() < 0.5;
    let mut v: Vec<bool> = (0..n).map(|i| (i % 2 == 0) != flip).collect();
    rng.shuffle(&mut v);
    v
}

/// Plan every session. Deterministic in `cfg.seed` and the clip ids.
fn plan_trials(
    cfg: &Config,
    items: &[Item],
    diff: &[(usize, usize)],
    rng: &mut Rng,
    used: &mut HashSet<String>,
) -> Vec<Trial> {
    let n = items.len();
    let sessions = cfg.trials_per_target;
    // per item, the order of target/solution in A and the side of X, each balanced over the sessions
    let target_in_a: Vec<Vec<bool>> = (0..n).map(|_| balanced(rng, sessions)).collect();
    let x_in_a: Vec<Vec<bool>> = (0..n).map(|_| balanced(rng, sessions)).collect();
    let side = |in_a: bool| if in_a { Side::A } else { Side::B };
    let mut all = Vec::new();
    for s in 0..sessions {
        let mut trials: Vec<Trial> = Vec::new();
        let blank = |kind, item, item_b, a: &str, b: &str, x_is| Trial {
            session: s as u32 + 1,
            n: 0,
            id: String::new(),
            kind,
            item,
            item_b,
            a: a.to_string(),
            b: b.to_string(),
            x_is,
        };
        for (i, it) in items.iter().enumerate() {
            let (a, b) = if target_in_a[i][s] {
                (&it.target_clip, &it.solution_clip)
            } else {
                (&it.solution_clip, &it.target_clip)
            };
            trials.push(blank(TrialKind::Real, i, None, a, b, side(x_in_a[i][s])));
        }
        let mut chosen: Vec<usize> = Vec::new();
        while chosen.len() < cfg.n_null.min(n) {
            let i = rng.below(n);
            if !chosen.contains(&i) {
                chosen.push(i);
            }
        }
        for i in chosen {
            let c = &items[i].target_clip;
            let x = side(rng.uniform() < 0.5);
            trials.push(blank(TrialKind::Null, i, None, c, c, x));
        }
        for &(i, j) in diff.iter().take(cfg.n_diff) {
            let (first, second) = if rng.uniform() < 0.5 { (i, j) } else { (j, i) };
            let x = side(rng.uniform() < 0.5);
            trials.push(blank(
                TrialKind::Different,
                first,
                Some(second),
                &items[first].target_clip,
                &items[second].target_clip,
                x,
            ));
        }
        rng.shuffle(&mut trials);
        for (k, t) in trials.iter_mut().enumerate() {
            t.n = k + 1;
            t.id = unique_id(rng, "t", used);
        }
        all.extend(trials);
    }
    all
}

/// Solve the targets and plan the sessions. Prints progress (no results: they could unblind the owner) to `out`.
///
/// # Errors
/// Nothing in the design can fail except writing progress; the result is `Res` for symmetry with the callers.
pub fn design(cfg: &Config, out: &mut dyn Write) -> Res<Designed> {
    let t_design = Instant::now();
    let load_start = load_avg();
    let render = cfg.render;
    let analysis = analysis_for(&render);
    let mut an = Analyzer::new(analysis);
    let scale = Scale::calibrate(&mut an, &render, 42, 50);
    let mut pe = ParEval::new(cfg.workers, render, analysis, Dist::A, scale);
    let targets = make_targets(cfg.n_targets, &render, cfg.seed);
    writeln!(
        out,
        "solving {} targets on {} workers (cap {:.0} s each, at most {} evaluations) ...",
        cfg.n_targets, cfg.workers, cfg.time_cap_s, cfg.budget
    )?;
    let mut rng = Rng::new(cfg.seed ^ 0x5eed_ab11_0000_0001);
    let mut used: HashSet<String> = HashSet::new();
    let mut items: Vec<Item> = Vec::new();
    for (ti, params) in targets.iter().enumerate() {
        pe.set_target(params);
        let (t1, t3) = thresholds_a(&pe, params, ti, &scale);
        let audio = {
            let mut b = Vec::new();
            render_unit(&render, params, &mut b);
            b
        };
        let stop = match cfg.stop_at {
            StopAt::T1 => Some(t1),
            StopAt::T3 => Some(t3),
            StopAt::Never => None,
        };
        let mut searches = vec![Search::new(SearchCfg {
            seed: cfg.search_seed + ti as u64,
            start: Some(heuristic(&render, &audio)),
            sigma_start: cfg.sigma_start,
            sigma_random: 0.3,
            lambda0: cfg.lambda,
            budget: cfg.budget,
            stop_at: stop,
            restarts: Restarts::Ipop,
        })];
        let t0 = Instant::now();
        let (mut e1, mut e3): (Milestone, Milestone) = (None, None);
        drive(&mut searches, &mut pe, |ss| {
            let best = ss
                .iter()
                .map(Search::best_fitness)
                .fold(f64::INFINITY, f64::min);
            let total: usize = ss.iter().map(Search::evals).sum();
            let t = t0.elapsed().as_secs_f64();
            if e3.is_none() && best <= t3 {
                e3 = Some((total, t));
            }
            if e1.is_none() && best <= t1 {
                e1 = Some((total, t));
            }
            cfg.time_cap_s <= 0.0 || t < cfg.time_cap_s
        });
        let wall = t0.elapsed().as_secs_f64();
        let s = &searches[0];
        let best_x = s.best_x().to_vec();
        let dist = pe.components(std::slice::from_ref(&best_x))[0].distance(Dist::A, &scale);
        let param_err = (1..N_PARAMS)
            .map(|i| (params[i] - best_x[i]).abs())
            .sum::<f64>()
            / (N_PARAMS - 1) as f64;
        items.push(Item {
            index: ti,
            target: params.clone(),
            wave_ok: Wave::from_unit(params[0]) == Wave::from_unit(best_x[0]),
            solution: best_x,
            t1,
            t3,
            start_distance: Some(s.start_fitness()).filter(|v| v.is_finite()),
            distance: dist,
            evals: s.evals(),
            restarts: s.restarts(),
            wall_s: wall,
            reached_t1: e1.is_some(),
            reached_t3: e3.is_some(),
            evals_to_t1: e1.map(|e| e.0),
            wall_to_t1_s: e1.map(|e| e.1),
            param_err,
            target_clip: unique_id(&mut rng, "c", &mut used),
            solution_clip: unique_id(&mut rng, "c", &mut used),
        });
        writeln!(
            out,
            "  target {}/{} done in {wall:.1} s ({} evaluations)",
            ti + 1,
            cfg.n_targets,
            s.evals()
        )?;
    }
    let design_wall_s = t_design.elapsed().as_secs_f64();
    let load_end = load_avg();

    let refs: Vec<&[f64]> = items.iter().map(|i| i.target.as_slice()).collect();
    let diff = different_pairs(&pe, &refs, cfg.n_diff, &scale);

    // clips: each loudness-matched on its own (enth-analysis), then one shared factor keeps every sample under the ceiling
    let sr = render.sample_rate as u32;
    let mut meter = LoudnessMeter::new(sr);
    let mut raws: Vec<Draft> = Vec::new();
    for it in &items {
        for (role, id, params) in [
            (Role::Target, &it.target_clip, &it.target),
            (Role::Solution, &it.solution_clip, &it.solution),
        ] {
            let audio = raw_clip(&render, params);
            let raw_peak = audio.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let lufs = meter.integrated(&audio);
            raws.push(Draft {
                id: id.clone(),
                item: it.index,
                role,
                lufs,
                peak: raw_peak,
                audio,
            });
        }
    }
    let matched = |r: &Draft| r.lufs.map_or(1.0, |l| db_to_gain(TARGET_LUFS - l));
    let peak_after = raws
        .iter()
        .map(|r| f64::from(r.peak) * matched(r))
        .fold(0.0, f64::max);
    let shared = if peak_after > f64::from(PEAK_CEILING) {
        f64::from(PEAK_CEILING) / peak_after
    } else {
        1.0
    };
    let mut clips = Vec::new();
    let mut infos = Vec::new();
    for r in raws {
        let gain = matched(&r) * shared;
        let wav = Wav::mono(
            sr,
            r.audio
                .iter()
                .map(|v| (f64::from(*v) * gain) as f32)
                .collect(),
        );
        infos.push(ClipInfo {
            id: r.id.clone(),
            item: r.item,
            role: r.role,
            raw_lufs: r.lufs,
            gain,
            peak_dbfs: peak_dbfs(&[&wav.channels[0]]),
            flags: flags::check(&[&wav.channels[0]], sr),
        });
        clips.push((r.id, wav));
    }

    let trials = plan_trials(cfg, &items, &diff, &mut rng, &mut used);
    Ok(Designed {
        manifest: Manifest {
            schema: SCHEMA_MANIFEST.to_string(),
            config: cfg.clone(),
            host_threads: host_threads(),
            load: [load_start, load_end],
            design_wall_s,
            items,
            clips: infos,
            trials,
        },
        clips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_has_equal_halves() {
        let mut rng = Rng::new(3);
        for n in 1..12 {
            let v = balanced(&mut rng, n);
            let t = v.iter().filter(|b| **b).count();
            assert!(t == n / 2 || t == n.div_ceil(2), "{n}: {t}");
        }
    }

    #[test]
    fn the_target_list_is_the_spikes() {
        // the first target of the seeded list is audible, and the list is a prefix-stable function of the seed
        let cfg = RenderCfg::short();
        let a = make_targets(3, &cfg, 20_261_019);
        let b = make_targets(5, &cfg, 20_261_019);
        assert_eq!(a[..], b[..3]);
        assert_ne!(a, make_targets(3, &cfg, 7));
    }
}
