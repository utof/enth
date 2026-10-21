//! Lane SS: `sound/1` gives the same bits on every platform. The pinned hash covers the in-house loudness of
//! synthetic signals and every number of three `sound/1` scores of a fixed level-4 challenge (its target, its start
//! and a near miss): percent, distance, floor, scale, jnd, the guard's floors and each component's readings and
//! badges. The same hash natively (debug and release) and on wasm32-wasip1 under wasmtime:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime cargo test -p enth-challenge --target wasm32-wasip1 \
//!     --test ss_determinism -- --nocapture
//! ```
//!
//! A change that moves it moves a `sound/1` score: that is a new scorer id (`sound/2`…), never an edit of this pin.
//! The one exception is the engine moving under the same scorer, on purpose: engine 0.3.0 (lane DRV, FU1: the
//! filter drive's make-up gain) moved this pin through the JND scale, whose jittered copies turn `filter1.drive` up
//! a step (now: scale 0.290973, the start 41.981971 %, the near miss 93.637716 %).

use enth_analysis::test_signals::{SR, Wave, oscillator, sine, white_noise};
use enth_challenge::calibrate::step;
use enth_challenge::loudness::integrated_lufs;
use enth_challenge::random;
use enth_challenge::score::sound::SoundV1;
use enth_challenge::{Challenge, Scorer, SoundScore, SpecPreset, TargetContext};
use enth_patch::{Op, ParamAddr};
use sha2::{Digest, Sha256};
use std::fmt::Write;

/// SHA-256 of the bytes below, as hex.
const PINNED: &str = "72666db8e3114279cd951ab65c25b5d0b6f2c271a4ac83a1f7cfe43f1fc6738b";

fn put(h: &mut Sha256, v: f64) {
    h.update(v.to_bits().to_be_bytes());
}

fn put_opt(h: &mut Sha256, v: Option<f64>) {
    match v {
        Some(v) => {
            h.update([1]);
            put(h, v);
        }
        None => h.update([0]),
    }
}

fn put_score(h: &mut Sha256, s: &SoundScore) {
    for v in [s.percent, s.distance, s.floor, s.scale, s.jnd] {
        put(h, v);
    }
    h.update((s.components.len() as u32).to_be_bytes());
    for c in &s.components {
        h.update(format!("{:?}/{:?}/{:?}", c.kind, c.direction, c.closeness).as_bytes());
        put_opt(h, c.target);
        put_opt(h, c.answer);
    }
}

#[test]
fn sound_v1_is_bit_identical_everywhere() {
    let mut h = Sha256::new();
    let signals = [
        sine(SR, 997.0, 0.1, 3.0),
        oscillator(SR, Wave::Saw, 130.81, 2.0, 0.0),
        white_noise(SR, 0.3, 2.0, 7),
        sine(SR, 997.0, 0.5, 0.2),
    ];
    for x in &signals {
        put_opt(&mut h, integrated_lufs(x, 48_000));
    }
    let spec = SpecPreset::by_id(4).unwrap().spec(1, 3);
    let g = random::target(&spec).unwrap();
    let start = random::start(spec.template).unwrap();
    let challenge = Challenge::recreate(spec, g.phrase);
    let prepared = SoundV1
        .prepare(&TargetContext {
            challenge: &challenge,
            target: &g.patch,
            start: &start,
            phrase: g.phrase(),
            randomized: &g.params,
        })
        .unwrap();
    let (pf, wf) = prepared.guard_floors();
    put(&mut h, pf);
    put(&mut h, wf);
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let mut near = g.patch.clone();
    let s = step(near.spec(&cutoff).unwrap()).unwrap();
    let v = s.apply(s.apply(near.value(&cutoff).unwrap(), true), true);
    enth_patch::apply(
        &mut near,
        Op::SetParam {
            addr: cutoff,
            value: v,
        },
    )
    .unwrap();
    for answer in [&g.patch, &start, &near] {
        let score = SoundV1.score(&prepared, answer).unwrap();
        eprintln!(
            "{:.6} % (d {:.6}, floor {:.6}, scale {:.6}, jnd {:.6})",
            score.percent, score.distance, score.floor, score.scale, score.jnd
        );
        put_score(&mut h, &score);
    }
    let got = h.finalize().iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    });
    eprintln!("sound/1 pin: {got}");
    assert_eq!(got, PINNED);
}
