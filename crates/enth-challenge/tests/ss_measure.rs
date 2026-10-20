//! Lane SS's dev measurements of `sound/1` on generated targets (not a gate): the nuisance floor and the JND scale
//! under metric v1 and v1.1, how far single parameter moves land in JND units, and what one `prepare` and one
//! `score` cost. They render a lot, so they are ignored in the gate; run them by hand, in release:
//!
//! ```text
//! cargo test --release -p enth-challenge --test ss_measure -- --ignored --nocapture --test-threads 1
//! ```
//!
//! `SS_MEASURE_N` sets the targets per level (default 8). The numbers in docs/challenge.md ("Sound score (lane SS)")
//! come from here.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_methods,
    reason = "a hand-run measurement: reads its size from the environment, times itself and prints a table"
)]

use enth_analysis::distance::metric_v1;
use enth_analysis::features::{FeatureCfg, FeatureExtractor, Features};
use enth_analysis::pooled::{Pooled, metric_v1_1, pool};
use enth_challenge::calibrate::{self, jitter_set, jittered_params, step};
use enth_challenge::loudness;
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::random;
use enth_challenge::score::sound::{SoundV1, floor_seed};
use enth_challenge::{Category, Phrase, RandomSpec, Scorer, TargetContext};
use enth_dsp::util::rng::derive;
use enth_patch::{Op, Patch};
use enth_render::{RenderSpec, render};
use std::fmt::Write;

fn n() -> u64 {
    std::env::var("SS_MEASURE_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8)
}

fn spec(difficulty: u8, i: u64) -> RandomSpec {
    RandomSpec {
        generator: random::GENERATOR_V1,
        seed: derive(0x5353_4D45_4153, i) >> 11,
        difficulty,
        amounts: [None; Category::COUNT],
        musical: 100,
        template: random::TEMPLATE_STARTER,
    }
}

/// Both metrics' inputs for one render.
struct Both {
    f: Features,
    p: Pooled,
}

fn analyse(fx: &mut FeatureExtractor, patch: &Patch, phrase: &Phrase, seed: u64) -> Both {
    let mid = render(
        patch,
        &phrase.performance(),
        &RenderSpec::new(SCORING_SAMPLE_RATE, seed),
    )
    .unwrap()
    .mid();
    let Some(g) = loudness::scoring_gain(&mid, SCORING_SAMPLE_RATE) else {
        let f = Features {
            silent: true,
            ..Features::default()
        };
        let p = pool(&f);
        return Both { f, p };
    };
    let x: Vec<f32> = mid.iter().map(|v| v * g as f32).collect();
    let f = fx.extract_unnormalised(&x);
    let p = pool(&f);
    Both { f, p }
}

fn d(a: &Both, b: &Both) -> (f64, f64) {
    (metric_v1(&a.f, &b.f), metric_v1_1(&a.p, &b.p))
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        f64::NAN
    } else {
        v[v.len() / 2]
    }
}

#[test]
#[ignore = "renders a few hundred targets; run by hand in release"]
fn floors_scales_and_moves_by_level() {
    let mut fx = FeatureExtractor::new(FeatureCfg::default());
    println!(
        "level seed lufs | v1: floor scale floor/scale | v1.1: floor scale floor/scale | start z v1 v1.1 | moves: \
         z(v1)/z(v1.1) for 3 steps of cutoff, attack, decay, release, fine"
    );
    let mut ratio = ([Vec::new(), Vec::new()], [Vec::new(), Vec::new()]);
    for (level, difficulty) in [(1, 0u8), (2, 25), (3, 50), (4, 75), (5, 100)] {
        let mut per_level = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for i in 0..n() {
            let s = spec(difficulty, i);
            let g = random::target(&s).unwrap();
            let phrase = g.phrase();
            let t = analyse(&mut fx, &g.patch, phrase, SCORING_SEED);
            if t.f.silent {
                println!("{level} {i} silent");
                continue;
            }
            if std::env::var("SS_MEASURE_SHOW").is_ok() {
                for r in &g.randomized {
                    if let random::Drawn::Param(a) = &r.drawn {
                        println!("    {a} = {:?}", g.patch.value(a));
                    } else {
                        println!("    {:?}", r.drawn);
                    }
                }
            }
            let mut floor = (0.0f64, 0.0f64);
            for k in 1..=2 {
                let o = analyse(&mut fx, &g.patch, phrase, floor_seed(k));
                let (a, b) = d(&t, &o);
                floor = (floor.0.max(a), floor.1.max(b));
            }
            let (mut j1, mut j11) = (Vec::new(), Vec::new());
            for p in jitter_set(&g.patch, phrase) {
                let o = analyse(&mut fx, &p, phrase, SCORING_SEED);
                let (a, b) = d(&t, &o);
                j1.push(a);
                j11.push(b);
            }
            let (s1, s11) = (calibrate::scale(&j1), calibrate::scale(&j11));
            let start = random::start(0).unwrap();
            let st = analyse(&mut fx, &start, phrase, SCORING_SEED);
            let (sd1, sd11) = d(&t, &st);
            let z = |dist: f64, fl: f64, sc: f64| calibrate::jnd(dist, fl, sc);
            let mut moves = String::new();
            let (params, _) = jittered_params(&g.patch, phrase);
            for key in ["cutoff", "attack", "decay", "release", "fine"] {
                let Some(addr) = params.iter().find(|a| a.param() == key) else {
                    moves.push_str(" -/-");
                    continue;
                };
                let spec = g.patch.spec(addr).unwrap();
                let st = step(spec).unwrap();
                let mut v = g.patch.value(addr).unwrap();
                for _ in 0..3 {
                    v = st.apply(v, true);
                }
                let v = v.clamp(spec.min(), spec.max());
                let mut p = g.patch.clone();
                enth_patch::apply(
                    &mut p,
                    Op::SetParam {
                        addr: addr.clone(),
                        value: v,
                    },
                )
                .unwrap();
                let o = analyse(&mut fx, &p, phrase, SCORING_SEED);
                let (a, b) = d(&t, &o);
                let _ = write!(
                    moves,
                    " {}:{:.1}/{:.1}",
                    addr,
                    z(a, floor.0, s1),
                    z(b, floor.1, s11)
                );
            }
            let lufs = loudness::integrated_lufs(
                &render(
                    &g.patch,
                    &phrase.performance(),
                    &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
                )
                .unwrap()
                .mid(),
                SCORING_SAMPLE_RATE,
            )
            .unwrap_or(f64::NAN);
            println!(
                "{level} {i} {:.1} | {:.4} {:.4} {:.2} | {:.4} {:.4} {:.2} | {:.1} {:.1} |{moves}",
                lufs,
                floor.0,
                s1,
                floor.0 / s1,
                floor.1,
                s11,
                floor.1 / s11,
                z(sd1, floor.0, s1),
                z(sd11, floor.1, s11)
            );
            per_level.0.push(s1);
            per_level.1.push(s11);
            per_level.2.push(floor.0 / s1);
            per_level.3.push(floor.1 / s11);
            ratio.0[0].push(floor.0 / s1);
            ratio.0[1].push(floor.1 / s11);
            ratio.1[0].push(s1);
            ratio.1[1].push(s11);
        }
        println!(
            "level {level}: median scale v1 {:.4} v1.1 {:.4}; median floor/scale v1 {:.2} v1.1 {:.2}",
            median(&mut per_level.0),
            median(&mut per_level.1),
            median(&mut per_level.2),
            median(&mut per_level.3)
        );
    }
    println!(
        "all: median floor/scale v1 {:.2} v1.1 {:.2}; median scale v1 {:.4} v1.1 {:.4}",
        median(&mut ratio.0[0]),
        median(&mut ratio.0[1]),
        median(&mut ratio.1[0]),
        median(&mut ratio.1[1])
    );
}

#[test]
#[ignore = "times prepare and score; run by hand in release"]
fn prepare_and_score_cost() {
    for (level, difficulty) in [(1, 0u8), (3, 50), (5, 100)] {
        let g = random::target(&spec(difficulty, 1)).unwrap();
        let challenge = enth_challenge::Challenge::recreate(spec(difficulty, 1), g.phrase);
        let start = random::start(0).unwrap();
        let ctx = TargetContext {
            challenge: &challenge,
            target: &g.patch,
            start: &start,
            phrase: g.phrase(),
            randomized: &g.params,
        };
        let t0 = std::time::Instant::now();
        let prepared = SoundV1.prepare(&ctx).unwrap();
        let t1 = std::time::Instant::now();
        let s = SoundV1.score(&prepared, &start).unwrap();
        let t2 = std::time::Instant::now();
        println!(
            "level {level}: prepare {:.0} ms, score {:.0} ms; start {:.1} % (d {:.4}, floor {:.4}, scale {:.4})",
            (t1 - t0).as_secs_f64() * 1000.0,
            (t2 - t1).as_secs_f64() * 1000.0,
            s.percent,
            s.distance,
            s.floor,
            s.scale
        );
    }
}

#[test]
#[ignore = "prepares and scores a few dozen rounds; run by hand in release"]
fn the_start_by_level() {
    use enth_challenge::score::sound::component_text;
    println!(
        "level seed: start % (jnd = mel + guard) | readings target/start: pitch ct, odd share | components"
    );
    for (level, difficulty) in [(1, 0u8), (2, 25), (3, 50), (4, 75), (5, 100)] {
        let (mut passed, mut total) = (0, 0);
        for i in 0..n() {
            let s = spec(difficulty, i);
            let g = random::target(&s).unwrap();
            let challenge = enth_challenge::Challenge::recreate(s, g.phrase);
            let start = random::start(0).unwrap();
            let ctx = TargetContext {
                challenge: &challenge,
                target: &g.patch,
                start: &start,
                phrase: g.phrase(),
                randomized: &g.params,
            };
            let prepared = SoundV1.prepare(&ctx).unwrap();
            let sc = SoundV1.score(&prepared, &start).unwrap();
            let mel = calibrate::jnd(sc.distance, sc.floor, sc.scale);
            let t = prepared.target_clip().readings;
            let pick = |k| {
                sc.components
                    .iter()
                    .find(|c| c.kind == k)
                    .and_then(|c| c.answer)
            };
            let texts: Vec<String> = sc.components.iter().map(component_text).collect();
            total += 1;
            if sc.percent >= 90.0 {
                passed += 1;
            }
            println!(
                "{level} {i}: {:.1} % ({:.2} = {:.2} + {:.2}) | {:?}/{:?} {:?}/{:?} | floors {:?} | {}",
                sc.percent,
                sc.jnd,
                mel,
                sc.jnd - mel,
                t.pitch.map(f64::round),
                pick(enth_challenge::ComponentKind::Pitch).map(f64::round),
                t.harmonics.map(|v| (v * 100.0).round() / 100.0),
                pick(enth_challenge::ComponentKind::Harmonics).map(|v| (v * 100.0).round() / 100.0),
                prepared.guard_floors(),
                texts.join("; ")
            );
        }
        println!("level {level}: the start passes {passed} of {total}");
    }
}
