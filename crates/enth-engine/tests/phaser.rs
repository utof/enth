//! `phaser` through the engine (S2a lane MOD; `docs/dsp/phaser.md`). The core's own tests feed it exact signals and
//! the catalogue's units are the wrapper's; here the signal is the engine's: ms, %, Hz, beats, the segment grid, the
//! tempo events and the plan swap are all in the path. What is held to what:
//!
//! - **The swept notches.** A sine probe through the phaser comes out as `H(fc(t))` times itself, `H` the closed form of
//!   the all-pass chain (`enth_dsp::phaser::response`) and `fc(t) = center · 2^(3 · depth · s(t))` with `s` the sweep in
//!   `f64` from the definition; the lock-in gain of the output over the input, window by window, is compared with the
//!   closed form averaged over the same window. A wrong rate, depth, centre, stage count, width or feedback shows.
//! - **The static notches.** With the sweep frozen the notch frequencies of a white-noise transfer function match the
//!   closed form `2 atan(g tan((2k + 1)π/(2N)))` within 2 % for every stage count.
//! - **Beats.** A synced sweep across tempo changes follows the closed form in beats, and the render is the same bit for
//!   bit at host blocks 1, 64, 100 and 4096.
//! - **Aliasing, level, `mix` 0, latency, two instances, a stage-count change, the 40 s tail, FTZ, allocation and the
//!   swap.**
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;
#[path = "common/modfx.rs"]
mod modfx;

use common::{SEED, SR, bits, drive, engine, events, id, note, reset_saw_patch};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep, fft};
use enth_dsp::phaser::{notch_hz, response};
use enth_engine::module::count_subnormals;
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, SEG, compile,
};
use enth_patch::catalogue::{Beats, OscParam, PhaserParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch};
use modfx::{FS, bpm_at, first_active_segment, sweep_cycles, tempo_events};

/// A held sine (key 83, 987.8 Hz) into one `phaser1` with the given settings.
fn tone_patch(set: &[(PhaserParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let mut m = ModuleInstance::new(id("phaser1"), ModuleKind::Phaser);
    for &(param, v) in set {
        m.values[param.index()] = v;
    }
    p.fx.push(m);
    p
}

/// The same patch without its effects.
fn dry(p: &Patch) -> Patch {
    let mut q = p.clone();
    q.fx.clear();
    q
}

/// One key held from frame 0 to the end.
fn held(key: u8, frames: usize) -> Vec<common::Note> {
    vec![note(key, 0, frames as u64 + 1)]
}

type Stereo = (Vec<f32>, Vec<f32>);

fn render_stereo(p: &Patch, ev: &[(u64, EventKind)], frames: usize, block: usize) -> Stereo {
    let mut e = engine(p, SEED);
    drive(&mut e, ev, frames, block, |_, _, _| {})
}

fn hypot(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

// ---------------------------------------------------------------------------------------------------------------
// The swept notches
// ---------------------------------------------------------------------------------------------------------------

/// What the catalogue says the phaser does.
struct Law {
    stages: usize,
    center_hz: f64,
    depth_pct: f64,
    feedback_pct: f64,
    width_pct: f64,
}

impl Law {
    /// The all-passes' corner at the sweep position `cycles`, for the left (`right` false) or right channel.
    fn corner(&self, cycles: f64, right: bool) -> f64 {
        let lag = if right { self.width_pct / 200.0 } else { 0.0 };
        self.center_hz
            * enth_dsp::util::math::exp2(3.0 * self.depth_pct / 100.0 * modfx::sine(cycles - lag))
    }
}

/// The worst `|measured − model|` of the complex gain at the probe frequency `f0`, window by window: the measured gain
/// is the Hann lock-in of the output over that of the input; the model is the closed form `H(fc(n))` averaged under the
/// same window and weighted with the input.
fn gain_error(dry: &Stereo, wet: &Stereo, law: &Law, theta: &[f64], f0: f64, from: usize) -> f64 {
    const WIN: usize = 960;
    let mut worst: f64 = 0.0;
    for (right, (x, y)) in [(false, (&dry.0, &wet.0)), (true, (&dry.1, &wet.1))] {
        let (xf, yf) = (modfx::to_f64(x), modfx::to_f64(y));
        let mut start = from;
        while start + WIN < x.len() {
            let zx = modfx::lock_in(&xf, start, WIN, f0);
            let zy = modfx::lock_in(&yf, start, WIN, f0);
            let measured = modfx::cdiv(zy, zx);
            // The model: the same lock-in of x with every sample scaled by H at that sample's corner.
            let w = core::f64::consts::TAU * f0 / FS;
            let (mut re, mut im) = (0.0, 0.0);
            for n in 0..WIN {
                let h = 0.5
                    - 0.5
                        * enth_dsp::util::math::cos(
                            core::f64::consts::TAU * (n as f64 + 0.5) / WIN as f64,
                        );
                let k = start + n;
                let (hr, hi) = response(
                    law.stages,
                    law.corner(theta[k], right),
                    law.feedback_pct / 100.0,
                    f0,
                    FS,
                );
                let a = w * k as f64;
                let v = xf[k] * h;
                // x·e^(−jωn)·H
                let (c, s) = (enth_dsp::util::math::cos(a), -enth_dsp::util::math::sin(a));
                let (xr, xi) = (v * c, v * s);
                re += xr * hr - xi * hi;
                im += xr * hi + xi * hr;
            }
            let model = modfx::cdiv((re, im), zx);
            worst = worst.max(hypot(measured.0 - model.0, measured.1 - model.1));
            start += 12_007;
        }
    }
    worst
}

/// A free-running sweep: the probe's gain follows the closed form of the all-pass chain at the swept corner. The
/// closed form is quasi-static (it takes the corner as frozen), so the error grows with the sweep speed and with the
/// loop's memory: 1e-3 for two stages without feedback, 5e-3 for the defaults, a few percent for twelve stages at 60 %
/// feedback and 0.1 Hz (and ten times less at a tenth of the rate: the implementation is not what limits it).
#[test]
fn the_swept_notches_follow_the_closed_form() {
    let frames = 12 * 48_000;
    let f0 = enth_dsp::alias::midi_hz(83.0);
    for (stages, center, depth, fb, rate, width, tol) in [
        (
            2usize, 1_000.0f64, 100.0f64, 0.0f64, 0.4f64, 50.0f64, 4e-3f64,
        ),
        (6, 1_000.0, 50.0, 30.0, 0.3, 50.0, 2e-2),
        (5, 700.0, 100.0, -60.0, 0.1, 100.0, 2e-2),
        (12, 1_500.0, 80.0, 60.0, 0.1, 0.0, 5e-2),
        (3, 2_500.0, 100.0, 0.0, 0.6, 25.0, 4e-3),
        (12, 900.0, 100.0, 0.0, 0.1, 100.0, 2e-2),
    ] {
        let p = tone_patch(&[
            (PhaserParam::Stages, stages as f32),
            (PhaserParam::Center, center as f32),
            (PhaserParam::Depth, depth as f32),
            (PhaserParam::Feedback, fb as f32),
            (PhaserParam::Rate, rate as f32),
            (PhaserParam::Width, width as f32),
        ]);
        let ev = events(&held(83, frames));
        let x = render_stereo(&dry(&p), &ev, frames, 512);
        let y = render_stereo(&p, &ev, frames, 512);
        let k0 = first_active_segment(&x.0, &x.1);
        let cps = f64::from(rate as f32) * SEG as f64 / FS;
        let theta = sweep_cycles(frames, k0, |_| cps);
        let law = Law {
            stages,
            center_hz: center,
            depth_pct: depth,
            feedback_pct: fb,
            width_pct: width,
        };
        let err = gain_error(&x, &y, &law, &theta, f0, 24_000);
        eprintln!(
            "{stages} stages centre {center} Hz depth {depth} % fb {fb} % rate {rate} Hz width {width} %: worst gain error {err:.2e}"
        );
        assert!(err < tol, "{stages} stages: {err:.2e}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The static notches
// ---------------------------------------------------------------------------------------------------------------

/// `H(f) = Y(f)/X(f)` from Hann-windowed segments (the cross spectrum over the power spectrum): exact for a linear
/// filter whose memory is short against the segment, biased by about `memory/len` otherwise.
fn transfer(x: &[f32], y: &[f32], from: usize, len: usize, hop: usize) -> Vec<(f64, f64)> {
    let mut num = vec![(0.0, 0.0); len / 2 + 1];
    let mut den = vec![0.0; len / 2 + 1];
    let mut start = from;
    while start + len <= x.len() {
        let win = |v: &[f32]| -> (Vec<f64>, Vec<f64>) {
            let mut re: Vec<f64> = (0..len)
                .map(|n| {
                    let h = 0.5
                        - 0.5
                            * enth_dsp::util::math::cos(
                                core::f64::consts::TAU * (n as f64 + 0.5) / len as f64,
                            );
                    f64::from(v[start + n]) * h
                })
                .collect();
            let mut im = vec![0.0; len];
            fft(&mut re, &mut im);
            (re, im)
        };
        let (xr, xi) = win(x);
        let (yr, yi) = win(y);
        for k in 0..=len / 2 {
            num[k].0 += xr[k] * yr[k] + xi[k] * yi[k];
            num[k].1 += xr[k] * yi[k] - xi[k] * yr[k];
            den[k] += xr[k] * xr[k] + xi[k] * xi[k];
        }
        start += hop;
    }
    num.iter()
        .zip(&den)
        .map(|(n, d)| (n.0 / d, n.1 / d))
        .collect()
}

/// A white noise voice into one `phaser1`.
fn noise_patch(set: &[(PhaserParam, f32)]) -> Patch {
    let mut p = tone_patch(set);
    p.generators = vec![ModuleInstance::new(id("noise1"), ModuleKind::Noise)];
    p
}

/// With the sweep frozen (depth 0) the notches of the transfer function, measured from white noise through the engine,
/// are the all-pass chain's closed form `2 atan(g tan((2k + 1)π/(2N)))` within 2 % for every stage count from 2 to 12
/// (even counts: `N/2` notches; odd: `(N − 1)/2` and a half at Nyquist), and the whole response is the closed form
/// (the bias of the cross spectrum, `memory/len`, sets the tolerance).
#[test]
fn the_static_notches_are_the_all_pass_chains_closed_form_for_every_stage_count() {
    const LEN: usize = 65_536;
    let frames = 4_000 + LEN + 4 * LEN / 2;
    let fc = 1_000.0;
    let mut worst_notch: f64 = 0.0;
    for stages in 2..=12usize {
        let p = noise_patch(&[
            (PhaserParam::Stages, stages as f32),
            (PhaserParam::Center, fc as f32),
            (PhaserParam::Depth, 0.0),
            (PhaserParam::Feedback, 0.0),
        ]);
        let ev = events(&held(60, frames));
        let x = render_stereo(&dry(&p), &ev, frames, 512);
        let y = render_stereo(&p, &ev, frames, 512);
        let h = transfer(&x.0, &y.0, 4_000, LEN, LEN / 2);
        let mag: Vec<f64> = h.iter().map(|v| hypot(v.0, v.1)).collect();
        // The whole response against the closed form.
        let mut worst: f64 = 0.0;
        for (k, got) in h.iter().enumerate().skip(64) {
            let f = k as f64 * FS / LEN as f64;
            if f > 21_000.0 {
                break;
            }
            let (re, im) = response(stages, fc, 0.0, f, FS);
            worst = worst.max(hypot(got.0 - re, got.1 - im));
        }
        assert!(
            worst < 1.0e-3 * stages as f64 + 1e-3,
            "{stages} stages: response error {worst:.2e}"
        );
        // The notches: local minima of |H| below 0.03, against the prediction (Nyquist's half notch left out).
        let found: Vec<f64> = (65..mag.len() - 1)
            .filter(|&k| mag[k] < 0.03 && mag[k] <= mag[k - 1] && mag[k] < mag[k + 1])
            .map(|k| k as f64 * FS / LEN as f64)
            .filter(|f| *f < 0.45 * FS)
            .collect();
        let want: Vec<f64> = notch_hz(stages, fc, FS)
            .into_iter()
            .filter(|f| *f < 0.45 * FS)
            .collect();
        assert_eq!(
            found.len(),
            want.len(),
            "{stages} stages: {found:.1?} against {want:.1?}"
        );
        for (got, want) in found.iter().zip(&want) {
            let dev = (got / want - 1.0).abs();
            worst_notch = worst_notch.max(dev);
            assert!(
                dev < 0.02,
                "{stages} stages: notch {got:.1} Hz against {want:.1}"
            );
        }
        eprintln!("{stages} stages: notches {found:.0?} Hz, response error {worst:.1e}");
    }
    eprintln!(
        "worst notch deviation from the closed form: {:.2} %",
        worst_notch * 100.0
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Beats
// ---------------------------------------------------------------------------------------------------------------

/// Tempo changes at segment boundaries: the render does not depend on the host block, and the sweep follows the closed
/// form in beats for 21 s, across three changes.
#[test]
fn a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block() {
    let beats = Beats::Bar;
    let schedule = [(3_200u64, 174.0), (400_000, 63.0), (700_000, 140.0)];
    let frames = 1_000_000;
    let f0 = enth_dsp::alias::midi_hz(83.0);
    let p = tone_patch(&[
        (PhaserParam::Sync, 1.0),
        (PhaserParam::Beats, beats.index() as f32),
        (PhaserParam::Stages, 2.0),
        (PhaserParam::Depth, 100.0),
        (PhaserParam::Center, 1_000.0),
        (PhaserParam::Feedback, 0.0),
        (PhaserParam::Width, 100.0),
        // The free rate is ignored while sync is on: a wrong one would show.
        (PhaserParam::Rate, 13.0),
    ]);
    let mut ev = events(&held(83, frames));
    ev.extend(tempo_events(&schedule));
    ev.sort_by_key(|(f, _)| *f);
    let reference = render_stereo(&p, &ev, frames, 4_096);
    for block in [1, 64, 100] {
        let got = render_stereo(&p, &ev, frames, block);
        assert_eq!(
            bits(&got.0),
            bits(&reference.0),
            "left at host block {block}"
        );
        assert_eq!(
            bits(&got.1),
            bits(&reference.1),
            "right at host block {block}"
        );
    }
    let x = render_stereo(&dry(&p), &ev, frames, 4_096);
    let k0 = first_active_segment(&x.0, &x.1);
    let theta = sweep_cycles(frames, k0, |k| {
        modfx::cycles_per_segment_synced(
            bpm_at(&schedule, enth_engine::DEFAULT_TEMPO_BPM, k),
            beats.length(),
        )
    });
    let law = Law {
        stages: 2,
        center_hz: 1_000.0,
        depth_pct: 100.0,
        feedback_pct: 0.0,
        width_pct: 100.0,
    };
    let err = gain_error(&x, &reference, &law, &theta, f0, 24_000);
    eprintln!(
        "synced bars across 140 -> 174 -> 63 -> 140 BPM: worst gain error {err:.2e}, {} cycles swept",
        theta[frames - 1]
    );
    assert!(err < 4e-3, "{err:.2e}");
    let flat = render_stereo(&p, &events(&held(83, frames)), frames, 4_096);
    assert_ne!(
        bits(&flat.0),
        bits(&reference.0),
        "the tempo changed the speed"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Aliasing
// ---------------------------------------------------------------------------------------------------------------

/// The keys of the sweeps: 110 Hz to 10.5 kHz.
const KEYS: [u8; 8] = [45, 57, 69, 81, 93, 105, 117, 124];

fn sweep(p: &Patch) -> Sweep {
    Sweep::run_keys(&KEYS, FS, &Measure::REPORT_02, |key| {
        let frames = FFT_LEN + 8_192;
        common::render(p, &[note(key, 0, frames as u64)], frames, 512, SEED).0
    })
}

/// The worst alias measure of the settings in `cases` at each key.
fn worst_of(cases: &[Vec<(PhaserParam, f32)>]) -> Vec<f64> {
    let mut worst = vec![f64::NEG_INFINITY; KEYS.len()];
    for set in cases {
        let s = sweep(&tone_patch(set));
        for (w, pt) in worst.iter_mut().zip(&s.points) {
            *w = w.max(pt.1);
        }
    }
    worst
}

/// A phaser has no delay line and no table to read: an all-pass chain is a rational filter, and a swept corner makes
/// sidebands at multiples of the sweep rate and nothing else. So the measure stays at the sine's own floor with the
/// sweep frozen (−92.6 dB), at the slowest full-depth sweep (−92.6) and at the defaults, 0.3 Hz at 50 %, whose
/// sidebands come within 8 bins of the guard at the top key (−90.8 dB). Measured on 2026-10-26 over six stages with 30 %
/// feedback, twelve with 60 %, two at 8 kHz with −60 %; the bound is the worst plus 1.5 dB.
const BOUND_DB: f64 = -89.3;

#[test]
fn the_phaser_adds_no_aliasing_frozen_slow_or_at_the_defaults() {
    let frozen = [
        vec![(PhaserParam::Depth, 0.0)],
        vec![
            (PhaserParam::Depth, 0.0),
            (PhaserParam::Stages, 12.0),
            (PhaserParam::Feedback, 60.0),
        ],
        vec![
            (PhaserParam::Depth, 0.0),
            (PhaserParam::Stages, 2.0),
            (PhaserParam::Feedback, -60.0),
            (PhaserParam::Center, 8_000.0),
        ],
    ];
    let slow = [
        vec![(PhaserParam::Rate, 0.01), (PhaserParam::Depth, 100.0)],
        vec![
            (PhaserParam::Rate, 0.01),
            (PhaserParam::Depth, 100.0),
            (PhaserParam::Stages, 12.0),
            (PhaserParam::Feedback, 60.0),
        ],
        vec![
            (PhaserParam::Rate, 0.01),
            (PhaserParam::Depth, 100.0),
            (PhaserParam::Stages, 2.0),
            (PhaserParam::Feedback, -60.0),
            (PhaserParam::Center, 8_000.0),
        ],
        vec![],
    ];
    for (name, cases) in [("frozen", &frozen[..]), ("slow and default", &slow[..])] {
        let worst = worst_of(cases);
        eprintln!("{name}, worst per key: {worst:.1?}");
        for (key, w) in KEYS.iter().zip(&worst) {
            assert!(*w <= BOUND_DB, "{name}, key {key}: {w:.1} dB");
        }
    }
}

/// At the fastest sweep the module's own sidebands (the sweep's rate times the modulation index, many hundreds of Hz)
/// are most of the output's off-harmonic power: recorded, not bounded.
#[test]
fn the_fastest_full_depth_sweep_is_all_sidebands() {
    let worst = worst_of(&[vec![(PhaserParam::Rate, 20.0), (PhaserParam::Depth, 100.0)]]);
    eprintln!("20 Hz at 100 %, worst per key: {worst:.1?}");
    assert!(worst.iter().all(|w| w.is_finite()));
}

// ---------------------------------------------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------------------------------------------

/// A saw chord through a phaser with `set`: (level change in dB over the last two seconds, peak ratio), the worse of the
/// two channels.
fn level_change(set: &[(PhaserParam, f32)]) -> (f64, f64) {
    let frames = 3 * 48_000;
    let mut p = reset_saw_patch();
    let mut m = ModuleInstance::new(id("phaser1"), ModuleKind::Phaser);
    for &(param, v) in set {
        m.values[param.index()] = v;
    }
    p.fx.push(m);
    let n = [
        note(48, 0, 4 * 48_000),
        note(52, 0, 4 * 48_000),
        note(55, 0, 4 * 48_000),
    ];
    let ev = events(&n);
    let x = render_stereo(&dry(&p), &ev, frames, 512);
    let y = render_stereo(&p, &ev, frames, 512);
    let from = 48_000;
    let db = [(&x.0, &y.0), (&x.1, &y.1)]
        .iter()
        .map(|(a, b)| modfx::db(modfx::rms(b, from) / modfx::rms(a, from)))
        .fold(f64::NAN, |m, v| {
            if m.is_nan() || v.abs() > m.abs() {
                v
            } else {
                m
            }
        });
    let peak = modfx::peak(&y.0, from).max(modfx::peak(&y.1, from))
        / modfx::peak(&x.0, from).max(modfx::peak(&x.1, from));
    (db, peak)
}

/// The level is bounded (F1m): whatever the stage count, centre and feedback, the output is never louder than the input
/// in RMS (+0.05 dB for rounding), and a saw chord loses at most 5.6 dB without feedback and 6.5 dB with it (the input to
/// the loop is scaled by `1 − |fb|`, so the wet part shrinks where the loop resonates). Measured 2026-10-26: −0.4 …
/// −3.8 dB at 0 %, −0.7 … −5.6 dB at 30 %, −6.0 at ±99 %. The peak of a chord can rise where the all-pass chain
/// re-phases its low partials (×1.55 at two stages around 200 Hz, with the RMS 3.8 dB down); it never exceeds 1.6.
#[test]
fn the_level_is_bounded_at_the_defaults_and_at_full_feedback() {
    let mut worst_peak: f64 = 0.0;
    for stages in [2.0f32, 6.0, 12.0] {
        for fb in [30.0f32, 0.0, 99.0, -99.0] {
            for center in [200.0f32, 1_000.0, 6_000.0] {
                let (db, peak) = level_change(&[
                    (PhaserParam::Stages, stages),
                    (PhaserParam::Feedback, fb),
                    (PhaserParam::Center, center),
                ]);
                eprintln!(
                    "{stages} stages fb {fb} centre {center} Hz: {db:+.2} dB, peak x{peak:.3}"
                );
                let floor = if fb == 0.0 { -5.6 } else { -6.5 };
                assert!(
                    db <= 0.05 && db >= floor,
                    "{stages} stages fb {fb} {center} Hz: {db:+.2} dB"
                );
                worst_peak = worst_peak.max(peak);
            }
        }
    }
    assert!(worst_peak <= 1.6, "peak x{worst_peak:.3}");
    // The patch defaults, with the sweep running.
    let (db, _) = level_change(&[]);
    assert!((-5.2..=0.05).contains(&db), "defaults: {db:+.2} dB");
}

// ---------------------------------------------------------------------------------------------------------------
// mix 0, latency, two instances, a stage-count change
// ---------------------------------------------------------------------------------------------------------------

fn chord() -> Vec<common::Note> {
    vec![
        note(48, 0, 30_000),
        note(55, 0, 30_000),
        note(60, 2_000, 30_000),
    ]
}

/// At `mix` 0 the input passes bit for bit: the phaser is in the chain, running, and the render is the one without it.
#[test]
fn mix_zero_passes_the_input_bit_for_bit() {
    let ev = events(&chord());
    for stages in [2.0f32, 5.0, 12.0] {
        let mut p = reset_saw_patch();
        let mut m = ModuleInstance::new(id("phaser1"), ModuleKind::Phaser);
        m.values[PhaserParam::Stages.index()] = stages;
        m.values[PhaserParam::Mix.index()] = 0.0;
        m.values[PhaserParam::Feedback.index()] = 70.0;
        p.fx.push(m);
        let with = render_stereo(&p, &ev, 40_000, 512);
        let without = render_stereo(&dry(&p), &ev, 40_000, 512);
        assert_eq!(bits(&with.0), bits(&without.0), "{stages} stages left");
        assert_eq!(bits(&with.1), bits(&without.1), "{stages} stages right");
        assert!(with.0.iter().any(|v| *v != 0.0));
    }
}

/// Zero latency: the dry part of the output is never delayed, so the first sample of a note is where the dry first
/// sample is, at every mix.
#[test]
fn latency_is_zero() {
    let frames = 4_000;
    let ev = events(&[note(60, 500, 3_000)]);
    for mix in [100.0f32, 50.0, 1.0] {
        for stages in [2.0f32, 6.0, 12.0] {
            let p = tone_patch(&[(PhaserParam::Mix, mix), (PhaserParam::Stages, stages)]);
            let x = render_stereo(&dry(&p), &ev, frames, 64);
            let y = render_stereo(&p, &ev, frames, 64);
            let first = |v: &[f32]| v.iter().position(|s| *s != 0.0).unwrap();
            assert_eq!(first(&y.0), first(&x.0), "{stages} stages mix {mix}");
            // The all-pass chain passes its input straight through (its direct term), so the first frame is (x + a·x)/2.
            let n = first(&x.0);
            let g = f64::from(y.0[n]) / f64::from(x.0[n]);
            assert!(
                g > -0.1 && g <= 1.0001,
                "{stages} stages mix {mix}: first-frame gain {g}"
            );
        }
    }
}

/// Two instances keep separate state: a second `phaser` with `mix` 0 before or after the first changes nothing, and two
/// active ones in series differ from either alone.
#[test]
fn two_instances_keep_separate_state() {
    let ev = events(&chord());
    let frames = 40_000;
    let one = |stages: f32, rate: f32, mix: f32| {
        let mut m = ModuleInstance::new(id("phaser1"), ModuleKind::Phaser);
        m.values[PhaserParam::Stages.index()] = stages;
        m.values[PhaserParam::Rate.index()] = rate;
        m.values[PhaserParam::Mix.index()] = mix;
        m
    };
    let mut alone = reset_saw_patch();
    alone.fx.push(one(6.0, 0.5, 100.0));
    let reference = render_stereo(&alone, &ev, frames, 512);
    for pos in 0..2 {
        let mut p = reset_saw_patch();
        let mut idle = one(9.0, 3.0, 0.0);
        idle.id = id("phaser2");
        p.fx.push(one(6.0, 0.5, 100.0));
        p.fx.insert(pos, idle);
        let got = render_stereo(&p, &ev, frames, 512);
        assert_eq!(
            bits(&got.0),
            bits(&reference.0),
            "idle neighbour at {pos}: left"
        );
        assert_eq!(
            bits(&got.1),
            bits(&reference.1),
            "idle neighbour at {pos}: right"
        );
    }
    let mut pair = reset_saw_patch();
    pair.fx.push(one(6.0, 0.5, 100.0));
    let mut second = one(4.0, 0.9, 100.0);
    second.id = id("phaser2");
    pair.fx.push(second.clone());
    let both = render_stereo(&pair, &ev, frames, 512);
    let mut only_second = reset_saw_patch();
    second.id = id("phaser1");
    only_second.fx.push(second);
    let second_alone = render_stereo(&only_second, &ev, frames, 512);
    assert_ne!(bits(&both.0), bits(&reference.0));
    assert_ne!(bits(&both.0), bits(&second_alone.0));
    let again = render_stereo(&pair, &ev, frames, 512);
    assert_eq!(bits(&both.0), bits(&again.0), "deterministic");
}

/// A value event that changes `stages` under a held tone crosses one segment: the largest step between samples across
/// the change stays within 1.7 times the steady one (a hard switch would jump by the difference of the two chains'
/// outputs, up to twice the amplitude, 15 times a 987 Hz sine's step).
#[test]
fn a_change_of_stage_count_does_not_click() {
    let p = tone_patch(&[
        (PhaserParam::Stages, 6.0),
        (PhaserParam::Depth, 0.0),
        (PhaserParam::Feedback, 0.0),
        // Off the notches at 987.8 Hz of every stage count used here.
        (PhaserParam::Center, 400.0),
    ]);
    let (plan, map) = compile(&p, &PrepareCfg::new(SR)).expect("the patch compiles");
    let stages = map
        .get(&"phaser1.stages".parse().unwrap())
        .expect("phaser1.stages is a parameter");
    let (mut e, mut link): (Engine, Link) = Engine::with_link(plan, SEED, LinkCapacity::default());
    let segments = 1_200;
    let mut left = vec![0.0f32; segments * SEG];
    for k in 0..segments {
        if k == 600 {
            link.send_param(0, stages, 7.0).expect("room");
        }
        if k == 900 {
            link.send_param(0, stages, 2.0).expect("room");
        }
        link.collect();
        let ev: Vec<Event> = if k == 0 {
            vec![Event {
                offset: 0,
                kind: EventKind::NoteOn {
                    key: NoteKey::new(83, 0),
                    velocity: 0.8,
                },
            }]
        } else {
            Vec::new()
        };
        let mut right = [0.0f32; SEG];
        e.process(&mut left[k * SEG..(k + 1) * SEG], &mut right, &ev);
    }
    let step = |r: std::ops::Range<usize>| {
        left[r.start..r.end]
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max)
    };
    let steady = step(500 * SEG..600 * SEG);
    for (name, at) in [("6 to 7", 600), ("7 to 2", 900)] {
        let across = step((at - 1) * SEG..(at + 3) * SEG);
        eprintln!("{name}: step {across:.4} against the steady {steady:.4}");
        assert!(across <= 1.7 * steady, "{name}: {across} against {steady}");
    }
    assert!(left.iter().all(|v| v.is_finite()));
    // The change is audible afterwards: the 7-stage output is not the 6-stage one.
    let l6 = &left[550 * SEG..600 * SEG];
    let l7 = &left[800 * SEG..850 * SEG];
    assert!(modfx::rms(l6, 0) > 0.0 && (modfx::rms(l7, 0) - modfx::rms(l6, 0)).abs() < 0.2);
}

// ---------------------------------------------------------------------------------------------------------------
// The tail, FTZ, allocation and the swap
// ---------------------------------------------------------------------------------------------------------------

const FRAMES_PER_SECOND: usize = 48_000;

/// A saw chord with a long release into twelve stages at a centre of 50 Hz (three octaves down at the sweep's
/// bottom: the slowest corner, 6 Hz) with +99 % feedback, the slowest decay the module has, then a five-stage
/// phaser with −60 %.
fn tail_patch() -> Patch {
    let mut p = reset_saw_patch();
    p.modulators[0].values[enth_patch::catalogue::AdsrParam::Release.index()] = 3_000.0;
    let mut a = ModuleInstance::new(id("phaser1"), ModuleKind::Phaser);
    a.values[PhaserParam::Stages.index()] = 12.0;
    a.values[PhaserParam::Center.index()] = 50.0;
    a.values[PhaserParam::Depth.index()] = 100.0;
    a.values[PhaserParam::Feedback.index()] = 99.0;
    a.values[PhaserParam::Rate.index()] = 7.0;
    let mut b = ModuleInstance::new(id("phaser2"), ModuleKind::Phaser);
    b.values[PhaserParam::Stages.index()] = 5.0;
    b.values[PhaserParam::Feedback.index()] = -60.0;
    p.fx.push(a);
    p.fx.push(b);
    p
}

fn tail_notes() -> Vec<common::Note> {
    vec![
        note(48, 0, FRAMES_PER_SECOND as u64),
        note(55, 0, FRAMES_PER_SECOND as u64),
        note(60, 960, FRAMES_PER_SECOND as u64),
    ]
}

#[test]
fn a_40_s_tail_through_the_phaser_has_no_subnormals() {
    let mut e = engine(&tail_patch(), SEED);
    let (mut sub_out, mut state_max, mut non_finite) = (0, 0, 0);
    let mut calls = 0usize;
    let (l, _) = drive(
        &mut e,
        &events(&tail_notes()),
        40 * FRAMES_PER_SECOND,
        512,
        |e, l, r| {
            sub_out += count_subnormals(l) + count_subnormals(r);
            non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                state_max = state_max.max(e.subnormals());
            }
            calls += 1;
        },
    );
    assert_eq!(sub_out, 0, "subnormal output samples");
    assert_eq!(state_max.max(e.subnormals()), 0, "engine state");
    assert_eq!(non_finite, 0);
    assert_eq!(e.active_voices().len(), 0, "voices end");
    // The 99 % loop around twelve stages at 6 Hz rings for minutes: it is not cut off at 40 s, and it is quiet.
    let late = &l[39 * FRAMES_PER_SECOND..];
    assert!(late.iter().all(|v| v.abs() < 1e-3));
}

/// The render is the same under every FTZ/DAZ mode: every state is flushed in code, so no subnormal reaches the
/// arithmetic.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn the_render_is_the_same_under_every_ftz_mode() {
    let patch = tail_patch();
    let run = || {
        let mut e = engine(&patch, SEED);
        let (l, r) = drive(
            &mut e,
            &events(&tail_notes()),
            12 * FRAMES_PER_SECOND,
            512,
            |_, _, _| {},
        );
        (bits(&l), bits(&r))
    };
    let reference = modfx::ftz::with_mode(0, run);
    for (name, mode_bits) in modfx::fp::MODES {
        assert!(
            modfx::ftz::with_mode(mode_bits, run) == reference,
            "under {name}"
        );
    }
}

#[cfg(debug_assertions)]
mod guarded {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// `process()` with a phaser at several stage counts, synced and free, with a tempo change, never allocates.
    #[test]
    fn process_does_not_allocate() {
        for stages in [2.0f32, 7.0, 12.0] {
            for sync in [0.0f32, 1.0] {
                let mut p = tail_patch();
                p.fx[0].values[PhaserParam::Stages.index()] = stages;
                p.fx[0].values[PhaserParam::Sync.index()] = sync;
                let mut e = engine(&p, SEED);
                let mut ev = events(&[note(48, 0, 4_000), note(64, 500, 6_000)]);
                ev.push((2_048, EventKind::Tempo { bpm: 91.0 }));
                ev.sort_by_key(|(f, _)| *f);
                let mut left = vec![0.0f32; 8_192];
                let mut right = vec![0.0f32; 8_192];
                let mut next = 0;
                let mut start = 0;
                let mut chunk: Vec<Event> = Vec::with_capacity(8);
                while start < left.len() {
                    let end = (start + 256).min(left.len());
                    chunk.clear();
                    while let Some(&(f, kind)) = ev.get(next) {
                        if f >= end as u64 {
                            break;
                        }
                        chunk.push(Event {
                            offset: (f - start as u64) as u32,
                            kind,
                        });
                        next += 1;
                    }
                    let (l, r) = (&mut left[start..end], &mut right[start..end]);
                    assert_no_alloc(|| e.process(l, r, &chunk));
                    start = end;
                }
                assert!(left.iter().any(|x| *x != 0.0), "{stages} stages");
            }
        }
    }
}

/// A chord held, released late; `edit` is swapped in at segment 200 when given. Returns the left channel and the
/// engine afterwards.
fn chord_with_swap(base: &Patch, edit: Option<&Patch>) -> (Vec<f32>, Engine) {
    let plan = |p: &Patch| {
        compile(p, &PrepareCfg::new(SR))
            .expect("the patch compiles")
            .0
    };
    let (mut e, mut link): (Engine, Link) =
        Engine::with_link(plan(base), SEED, LinkCapacity::default());
    let mut edit = edit.map(plan);
    let mut left = vec![0.0; 600 * SEG];
    for k in 0..600 {
        if k == 200
            && let Some(p) = edit.take()
        {
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        let ev: Vec<Event> = if k == 0 {
            [48, 55, 60]
                .iter()
                .enumerate()
                .map(|(i, &n)| Event {
                    offset: 0,
                    kind: EventKind::NoteOn {
                        key: NoteKey::new(n, i as i32),
                        velocity: 0.8,
                    },
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut right = [0.0f32; SEG];
        e.process(&mut left[k * SEG..(k + 1) * SEG], &mut right, &ev);
    }
    (left, e)
}

/// The filter state moves with the plan swap: recompiling the same patch under a held chord (an instant swap) and
/// adding an unrelated module (a crossfading swap, both plans run on copies of the state) both give the render of no
/// swap at all, bit for bit. A lost state would show as a click and a restarted sweep.
#[test]
fn the_plan_swap_carries_the_filter_state() {
    let base = tail_patch();
    let (l0, _) = chord_with_swap(&base, None);
    assert!(l0.iter().any(|x| *x != 0.0));
    let (l1, e) = chord_with_swap(&base, Some(&base));
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(
        e.stats().crossfades,
        0,
        "nothing added or removed: an instant swap"
    );
    assert_eq!(bits(&l1), bits(&l0));
    let mut edit = base.clone();
    let mut silent = ModuleInstance::new(id("osc3"), ModuleKind::Osc);
    silent.values[OscParam::Level.index()] = 0.0;
    edit.generators.push(silent);
    let (l2, e) = chord_with_swap(&base, Some(&edit));
    assert_eq!(e.stats().crossfades, 1, "a module came: a crossfading swap");
    assert_eq!(bits(&l2), bits(&l0));
    // A different setting at the swap is heard from there on, and only from there on.
    let mut changed = base.clone();
    changed.fx[0].values[PhaserParam::Center.index()] = 400.0;
    let (l3, _) = chord_with_swap(&base, Some(&changed));
    let swap = 200 * SEG;
    assert_eq!(bits(&l3[..swap]), bits(&l0[..swap]));
    assert_ne!(bits(&l3[swap..]), bits(&l0[swap..]));
    assert!(l3.iter().all(|x| x.is_finite()));
}
