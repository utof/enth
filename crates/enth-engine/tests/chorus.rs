//! `chorus` through the engine (S2a lane MOD; `docs/dsp/chorus.md`). The core's own tests feed it exact signals; here
//! the signal is the engine's: the catalogue's units (ms, %, Hz, beats), the segment grid, the tempo events and the
//! plan swap are all in the path. What is held to what:
//!
//! - **The delay law.** The wet part of the output (`2·y − x`, with `x` the same patch without the chorus) is compared
//!   sample by sample with `x` read at `n − τ(n)` by a 64-point windowed sinc, `τ = delay · (1 + depth · s)`, `s` the
//!   sweep in `f64` from the definition: a wrong delay, depth, rate, width or sweep shape shows at once.
//! - **The comb.** With the sweep frozen (depth 0) the chorus is a linear filter, `H = (1 + K/(1 − fb·K))/2`; the
//!   transfer function measured from white noise matches the closed form at every bin, notches included.
//! - **Beats.** A synced sweep across tempo changes follows the closed form in beats, and the render is the same bit
//!   for bit at host blocks 1, 64, 100 and 4096.
//! - **Aliasing, level, `mix` 0, latency, two instances, the 40 s tail, FTZ, allocation and the swap.**
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;
#[path = "common/modfx.rs"]
mod modfx;

use common::{SEED, SR, bits, drive, engine, events, id, note, reset_saw_patch};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep, fft};
use enth_dsp::chorus::response;
use enth_engine::module::count_subnormals;
use enth_engine::{
    Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, SEG, compile,
};
use enth_patch::catalogue::{Beats, ChorusMode, ChorusParam, OscParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch};
use modfx::{FS, bpm_at, first_active_segment, sinc_at, sweep_cycles, tempo_events};

/// A held sine (key 83, 987.8 Hz) into one `chorus1` with the given settings.
fn tone_patch(mode: ChorusMode, set: &[(ChorusParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let mut m = ModuleInstance::new(id("chorus1"), ModuleKind::Chorus);
    m.values[ChorusParam::Mode.index()] = mode.index() as f32;
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

fn libm_hypot(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

fn render_stereo(p: &Patch, ev: &[(u64, EventKind)], frames: usize, block: usize) -> Stereo {
    let mut e = engine(p, SEED);
    drive(&mut e, ev, frames, block, |_, _, _| {})
}

// ---------------------------------------------------------------------------------------------------------------
// The delay law
// ---------------------------------------------------------------------------------------------------------------

/// What the catalogue says one tap does.
struct Law {
    mode: ChorusMode,
    delay_ms: f64,
    depth_pct: f64,
    width_pct: f64,
}

impl Law {
    /// The tap's delay in samples at the sweep position `cycles`, for the left (`right` false) or right channel.
    fn tau(&self, cycles: f64, right: bool) -> f64 {
        // The right channel runs `width` × 180° behind the left.
        let lag = if right { self.width_pct / 200.0 } else { 0.0 };
        let c = cycles - lag;
        let s = match self.mode {
            ChorusMode::Flanger => modfx::triangle(c),
            _ => modfx::sine(c),
        };
        (self.delay_ms * 0.001 * FS * (1.0 + self.depth_pct / 100.0 * s)).max(1.0)
    }
}

/// The worst `|wet − x(n − τ(n))|` over the checked windows, relative to the dry peak. `theta[n]` is the sweep in
/// cycles at frame `n`.
fn law_error(dry: &Stereo, wet: &Stereo, law: &Law, theta: &[f64], from: usize) -> f64 {
    let peak = modfx::peak(&dry.0, 0);
    let mut worst: f64 = 0.0;
    for (right, (x, y)) in [(false, (&dry.0, &wet.0)), (true, (&dry.1, &wet.1))] {
        let mut start = from;
        while start + 512 < x.len() {
            for n in start..start + 512 {
                let w_meas = 2.0 * f64::from(y[n]) - f64::from(x[n]);
                let w_model = sinc_at(x, n as f64 - law.tau(theta[n], right));
                worst = worst.max((w_meas - w_model).abs());
            }
            start += 9_973;
        }
    }
    worst / peak
}

/// A free-running sweep: the delay follows `delay · (1 + depth · s(rate · t))` to a few millionths of the peak.
#[test]
fn the_swept_delay_follows_the_catalogue_law() {
    let frames = 5 * 48_000;
    for (mode, delay_ms, depth, rate, width) in [
        (ChorusMode::Chorus, 7.0, 50.0, 0.5, 100.0),
        (ChorusMode::Chorus, 12.5, 100.0, 2.0, 40.0),
        (ChorusMode::Flanger, 3.0, 60.0, 1.0, 100.0),
        (ChorusMode::Flanger, 0.5, 80.0, 0.7, 0.0),
    ] {
        let p = tone_patch(
            mode,
            &[
                (ChorusParam::Delay, delay_ms as f32),
                (ChorusParam::Depth, depth as f32),
                (ChorusParam::Rate, rate as f32),
                (ChorusParam::Width, width as f32),
            ],
        );
        let n = held(83, frames);
        let ev = events(&n);
        let x = render_stereo(&dry(&p), &ev, frames, 512);
        let y = render_stereo(&p, &ev, frames, 512);
        let k0 = first_active_segment(&x.0, &x.1);
        let cps = f64::from(rate as f32) * SEG as f64 / FS;
        let theta = sweep_cycles(frames, k0, |_| cps);
        let law = Law {
            mode,
            delay_ms,
            depth_pct: depth,
            width_pct: width,
        };
        let err = law_error(&x, &y, &law, &theta, 2_000);
        eprintln!(
            "{mode:?} delay {delay_ms} ms depth {depth} % rate {rate} Hz width {width} %: worst error {err:.2e} of the peak"
        );
        assert!(err < 1e-4, "{mode:?}: {err:.2e}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The comb
// ---------------------------------------------------------------------------------------------------------------

/// `H(f) = Y(f)/X(f)` estimated from Hann-windowed segments of `len` samples every `hop` (the cross spectrum over the
/// power spectrum, summed over the segments): exact for a linear filter whose memory is short against `len`.
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
            // conj(X)·Y
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

/// A white noise voice into one `chorus1`.
fn noise_patch(mode: ChorusMode, set: &[(ChorusParam, f32)]) -> Patch {
    let mut p = tone_patch(mode, set);
    p.generators = vec![ModuleInstance::new(id("noise1"), ModuleKind::Noise)];
    p
}

/// With the sweep frozen (depth 0) the chorus is a linear filter and its transfer function, measured from white noise
/// through the engine, is the closed form `(1 + (1 − |fb|)·K/(1 − fb·K))/2`: the flanger's comb notches at odd
/// multiples of `1/(2τ)`, the feedback's peaks, the interpolator's droop at the top of the band. The estimate from
/// windowed segments is biased by the filter's memory over the segment (about `m/N` of the response: 0.1 % to 0.4 % at
/// 65,536 samples), which sets the tolerance; the exact checks of the delay and feedback are in the next test.
#[test]
fn the_static_comb_matches_the_closed_form_through_the_engine() {
    const LEN: usize = 65_536;
    let frames = 4_000 + LEN + 6 * LEN / 2;
    for (mode, delay_ms, fb) in [
        (ChorusMode::Flanger, 1.0, 0.0),
        (ChorusMode::Flanger, 0.37, 60.0),
        (ChorusMode::Flanger, 0.8, -50.0),
        (ChorusMode::Chorus, 2.0, 0.0),
    ] {
        let p = noise_patch(
            mode,
            &[
                (ChorusParam::Depth, 0.0),
                (ChorusParam::Delay, delay_ms as f32),
                (ChorusParam::Feedback, fb as f32),
            ],
        );
        let ev = events(&held(60, frames));
        let x = render_stereo(&dry(&p), &ev, frames, 512);
        let y = render_stereo(&p, &ev, frames, 512);
        let h = transfer(&x.0, &y.0, 4_000, LEN, LEN / 2);
        let tau = delay_ms * 0.001 * FS;
        let mut worst: f64 = 0.0;
        for (k, got) in h.iter().enumerate().skip(64) {
            let f = k as f64 * FS / LEN as f64;
            if f > 21_000.0 {
                break;
            }
            let (re, im) = response(tau, fb / 100.0, f, FS);
            worst = worst.max(libm_hypot(got.0 - re, got.1 - im));
        }
        eprintln!("{mode:?} {delay_ms} ms fb {fb} %: worst |H - closed form| {worst:.2e}");
        assert!(worst < 8e-3, "{mode:?} {delay_ms} ms fb {fb}: {worst:.2e}");
        // The first notch of the comb (no feedback): the deepest point of |H| below 1.5/(2τ) is at 1/(2τ).
        if fb == 0.0 {
            let want = 1.0 / (2.0 * delay_ms * 0.001);
            let (k, depth) = h
                .iter()
                .enumerate()
                .skip(64)
                .take_while(|(k, _)| (*k as f64) * FS / (LEN as f64) < 1.5 * want)
                .map(|(k, v)| (k, (v.0 * v.0 + v.1 * v.1).sqrt()))
                .fold((0, f64::INFINITY), |a, b| if b.1 < a.1 { b } else { a });
            let f = k as f64 * FS / LEN as f64;
            assert!(
                (f / want - 1.0).abs() < 0.01,
                "first notch {f} Hz, want {want}"
            );
            assert!(depth < 0.03, "and it is deep: {depth}");
        }
    }
}

/// The feedback loop through the engine, in the time domain: with a sine in, the wet part is
/// `(1 − |fb|)·Σ fb^(j−1)·x(n − j·τ)`, the delay of each trip read by the windowed sinc. This pins `delay` (ms to
/// samples) and `feedback` (%, sign) exactly; the interpolator compounded `j` times differs from one long sinc read by
/// a few millionths.
#[test]
fn the_feedback_loop_is_the_geometric_series_of_the_delay() {
    let frames = 2 * 48_000;
    for (mode, delay_ms, fb) in [
        (ChorusMode::Flanger, 1.7, 60.0),
        (ChorusMode::Flanger, 0.4, -85.0),
        (ChorusMode::Chorus, 9.0, 40.0),
    ] {
        let p = tone_patch(
            mode,
            &[
                (ChorusParam::Depth, 0.0),
                (ChorusParam::Delay, delay_ms as f32),
                (ChorusParam::Feedback, fb as f32),
            ],
        );
        let ev = events(&held(83, frames));
        let x = render_stereo(&dry(&p), &ev, frames, 512);
        let y = render_stereo(&p, &ev, frames, 512);
        let tau = delay_ms * 0.001 * FS;
        let f = fb / 100.0;
        let mut worst: f64 = 0.0;
        for n in (60_000..60_000 + 400).chain(90_000..90_000 + 400) {
            let mut w = 0.0;
            let mut gain = 1.0 - f64::abs(f);
            for j in 1..=220 {
                w += gain * sinc_at(&x.0, n as f64 - f64::from(j) * tau);
                gain *= f;
                if gain.abs() < 1e-9 {
                    break;
                }
            }
            let w_meas = 2.0 * f64::from(y.0[n]) - f64::from(x.0[n]);
            worst = worst.max((w_meas - w).abs());
        }
        let rel = worst / modfx::peak(&x.0, 0);
        eprintln!("{mode:?} {delay_ms} ms fb {fb} %: worst error {rel:.2e} of the peak");
        assert!(rel < 5e-4, "{mode:?} {delay_ms} ms fb {fb}: {rel:.2e}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Beats
// ---------------------------------------------------------------------------------------------------------------

/// Tempo changes at segment boundaries (frame, bpm): the render does not depend on the host block, and the sweep
/// follows the closed form in beats for 21 s, across three changes, to the same few millionths as a free-running one.
#[test]
fn a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block() {
    let beats = Beats::Quarter;
    let schedule = [(3_200u64, 174.0), (400_000, 63.0), (700_000, 140.0)];
    let frames = 1_000_000;
    let p = tone_patch(
        ChorusMode::Chorus,
        &[
            (ChorusParam::Sync, 1.0),
            (ChorusParam::Beats, beats.index() as f32),
            (ChorusParam::Depth, 50.0),
            (ChorusParam::Delay, 7.0),
            (ChorusParam::Width, 100.0),
            // The free rate is ignored while sync is on: a wrong one would show.
            (ChorusParam::Rate, 13.0),
        ],
    );
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
        mode: ChorusMode::Chorus,
        delay_ms: 7.0,
        depth_pct: 50.0,
        width_pct: 100.0,
    };
    let err = law_error(&x, &reference, &law, &theta, 2_000);
    eprintln!(
        "synced quarter notes across 140 -> 174 -> 63 -> 140 BPM: worst error {err:.2e} of the peak, {} cycles swept",
        theta[frames - 1]
    );
    assert!(err < 1e-4, "{err:.2e}");
    // The tempo really changed the speed: a render at the initial tempo throughout differs.
    let flat = render_stereo(&p, &events(&held(83, frames)), frames, 4_096);
    assert_ne!(bits(&flat.0), bits(&reference.0));
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
fn worst_of(cases: &[(ChorusMode, Vec<(ChorusParam, f32)>)]) -> Vec<f64> {
    let mut worst = vec![f64::NEG_INFINITY; KEYS.len()];
    for (mode, set) in cases {
        let s = sweep(&tone_patch(*mode, set));
        for (w, pt) in worst.iter_mut().zip(&s.points) {
            *w = w.max(pt.1);
        }
    }
    worst
}

/// With the sweep frozen the chorus makes nothing a sine did not have: the measure is the window's floor, the same
/// as the oscillator's own (worst −92.6 dB measured on 2026-10-26; the bound is that plus 1.5 dB,
/// `docs/dsp/aliasing.md`).
const FROZEN_BOUND_DB: f64 = -91.1;

/// A sweep at the slowest rate, 0.01 Hz, and the full depth: the delay's own sidebands stay inside the measure's guard
/// bins (the Doppler shift of a 7 ms delay at 100 % is 4.4e-4 of the frequency), and what lies outside it is the
/// four-point Lagrange read's: the error of the interpolator depends on the fraction, the fraction cycles at the
/// Doppler rate, so a tone gets sidebands at ± that rate down by the interpolation error at its frequency (which grows
/// as the fourth power of the frequency: −66 dB at 3.5 kHz, −48 dB at 7 kHz, −34 dB at 10.5 kHz in the bounds below).
/// Measured on 2026-10-26 over the chorus, the ensemble and a flanger with feedback; each bound is the worst plus
/// 1.5 dB.
const MOVING_BOUNDS_DB: [f64; 8] = [-101.9, -95.0, -91.9, -91.0, -89.0, -65.8, -48.4, -33.6];

#[test]
fn a_frozen_sweep_adds_no_aliasing_in_any_mode() {
    let worst = worst_of(&[
        (ChorusMode::Chorus, vec![(ChorusParam::Depth, 0.0)]),
        (ChorusMode::Ensemble, vec![(ChorusParam::Depth, 0.0)]),
        (
            ChorusMode::Flanger,
            vec![
                (ChorusParam::Depth, 0.0),
                (ChorusParam::Delay, 2.0),
                (ChorusParam::Feedback, 60.0),
            ],
        ),
    ]);
    eprintln!("frozen, worst per key: {worst:.1?}");
    for (key, w) in KEYS.iter().zip(&worst) {
        assert!(*w <= FROZEN_BOUND_DB, "key {key}: {w:.1} dB");
    }
}

#[test]
fn the_slowest_full_depth_sweep_stays_below_the_recorded_bounds() {
    let worst = worst_of(&[
        (
            ChorusMode::Chorus,
            vec![(ChorusParam::Rate, 0.01), (ChorusParam::Depth, 100.0)],
        ),
        (
            ChorusMode::Ensemble,
            vec![(ChorusParam::Rate, 0.01), (ChorusParam::Depth, 100.0)],
        ),
        (
            ChorusMode::Flanger,
            vec![
                (ChorusParam::Rate, 0.01),
                (ChorusParam::Depth, 100.0),
                (ChorusParam::Delay, 2.0),
                (ChorusParam::Feedback, 60.0),
            ],
        ),
        (
            ChorusMode::Chorus,
            vec![
                (ChorusParam::Rate, 0.01),
                (ChorusParam::Depth, 100.0),
                (ChorusParam::Delay, 3.0),
                (ChorusParam::Width, 0.0),
            ],
        ),
    ]);
    eprintln!("moving, worst per key: {worst:.1?}");
    for ((key, w), bound) in KEYS.iter().zip(&worst).zip(MOVING_BOUNDS_DB) {
        assert!(*w <= bound, "key {key}: {w:.1} dB against {bound:.1}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------------------------------------------

/// A saw chord through a chorus with `set`: (level change in dB over the last two seconds, peak ratio), the worse of
/// the two channels.
fn level_change(mode: ChorusMode, set: &[(ChorusParam, f32)]) -> (f64, f64) {
    let frames = 3 * 48_000;
    let mut p = reset_saw_patch();
    let mut m = ModuleInstance::new(id("chorus1"), ModuleKind::Chorus);
    m.values[ChorusParam::Mode.index()] = mode.index() as f32;
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

/// The level is bounded (F1m): whatever `mode`, `delay` and `feedback`, the output never gets louder than the input
/// (+0.05 dB for rounding), and a saw chord loses at most 3.6 dB with a chorus or flanger, 5.6 dB with the ensemble
/// (whose wet part is the mean of three decorrelated taps) and 6.5 dB with feedback (the input to the loop is scaled
/// by `1 − |fb|`). Measured 2026-10-26: chorus −2.3 … −3.2 dB, flanger −2.3 … −3.0, ensemble −2.8 … −4.8, and −6.0 at
/// ±99 %; the peak never rose above 0.91 of the dry chord's.
#[test]
fn the_level_is_bounded_at_the_defaults_and_at_full_feedback() {
    for mode in [
        ChorusMode::Chorus,
        ChorusMode::Ensemble,
        ChorusMode::Flanger,
    ] {
        for fb in [0.0f32, 99.0, -99.0] {
            for delay in [1.0f32, 7.0, 30.0] {
                let (db, peak) = level_change(
                    mode,
                    &[(ChorusParam::Feedback, fb), (ChorusParam::Delay, delay)],
                );
                eprintln!("{mode:?} fb {fb} delay {delay} ms: {db:+.2} dB, peak x{peak:.3}");
                let floor = if fb != 0.0 {
                    -6.5
                } else if mode == ChorusMode::Ensemble {
                    -5.6
                } else {
                    -3.6
                };
                assert!(
                    db <= 0.05 && db >= floor,
                    "{mode:?} fb {fb} {delay} ms: {db:+.2} dB"
                );
                assert!(peak <= 1.07, "{mode:?} fb {fb} {delay} ms: peak x{peak:.3}");
            }
        }
    }
    // The defaults: a freshly added chorus is within the documented 3 dB of the input (the ensemble is not: −4.8 dB).
    let (db, _) = level_change(ChorusMode::Chorus, &[]);
    assert!(db > -3.3, "{db}");
}

// ---------------------------------------------------------------------------------------------------------------
// mix 0, latency, two instances
// ---------------------------------------------------------------------------------------------------------------

fn chord() -> Vec<common::Note> {
    vec![
        note(48, 0, 30_000),
        note(55, 0, 30_000),
        note(60, 2_000, 30_000),
    ]
}

/// At `mix` 0 the input passes bit for bit: the chorus is in the chain, running, and the render is the one without it.
#[test]
fn mix_zero_passes_the_input_bit_for_bit() {
    let ev = events(&chord());
    for mode in [
        ChorusMode::Chorus,
        ChorusMode::Ensemble,
        ChorusMode::Flanger,
    ] {
        let mut p = reset_saw_patch();
        let mut m = ModuleInstance::new(id("chorus1"), ModuleKind::Chorus);
        m.values[ChorusParam::Mode.index()] = mode.index() as f32;
        m.values[ChorusParam::Mix.index()] = 0.0;
        m.values[ChorusParam::Feedback.index()] = 70.0;
        p.fx.push(m);
        let with = render_stereo(&p, &ev, 40_000, 512);
        let without = render_stereo(&dry(&p), &ev, 40_000, 512);
        assert_eq!(bits(&with.0), bits(&without.0), "{mode:?} left");
        assert_eq!(bits(&with.1), bits(&without.1), "{mode:?} right");
        assert!(with.0.iter().any(|v| *v != 0.0));
    }
}

/// Zero latency: the dry part of the output is never delayed, so the first sample of a note is where the dry first
/// sample is, at every mix, and the output at that frame is the dry sample blended with the wet (nothing read yet).
#[test]
fn latency_is_zero() {
    let frames = 4_000;
    let ev = events(&[note(60, 500, 3_000)]);
    for mix in [100.0f32, 50.0, 1.0] {
        for mode in [
            ChorusMode::Chorus,
            ChorusMode::Ensemble,
            ChorusMode::Flanger,
        ] {
            let p = tone_patch(mode, &[(ChorusParam::Mix, mix), (ChorusParam::Delay, 0.1)]);
            let x = render_stereo(&dry(&p), &ev, frames, 64);
            let y = render_stereo(&p, &ev, frames, 64);
            let first = |v: &[f32]| v.iter().position(|s| *s != 0.0).unwrap();
            assert_eq!(first(&y.0), first(&x.0), "{mode:?} mix {mix}");
            // Nothing is audible before the note, and the first frames are the dry ones scaled by 1 − mix/2 at most.
            let n = first(&x.0);
            let g = f64::from(y.0[n]) / f64::from(x.0[n]);
            assert!(
                g > 0.45 && g <= 1.0001,
                "{mode:?} mix {mix}: first-frame gain {g}"
            );
        }
    }
}

/// Two instances keep separate state: a second `chorus` with `mix` 0 before or after the first changes nothing (it
/// passes its input bit for bit, whatever its own lines hold), and two active ones in series differ from either alone.
#[test]
fn two_instances_keep_separate_state() {
    let ev = events(&chord());
    let frames = 40_000;
    let one = |mode: ChorusMode, rate: f32, mix: f32| {
        let mut m = ModuleInstance::new(id("chorus1"), ModuleKind::Chorus);
        m.values[ChorusParam::Mode.index()] = mode.index() as f32;
        m.values[ChorusParam::Rate.index()] = rate;
        m.values[ChorusParam::Mix.index()] = mix;
        m
    };
    let mut alone = reset_saw_patch();
    alone.fx.push(one(ChorusMode::Chorus, 0.5, 100.0));
    let reference = render_stereo(&alone, &ev, frames, 512);
    for pos in 0..2 {
        let mut p = reset_saw_patch();
        let mut idle = one(ChorusMode::Flanger, 3.0, 0.0);
        idle.id = id("chorus2");
        p.fx.push(one(ChorusMode::Chorus, 0.5, 100.0));
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
    pair.fx.push(one(ChorusMode::Chorus, 0.5, 100.0));
    let mut second = one(ChorusMode::Ensemble, 0.9, 100.0);
    second.id = id("chorus2");
    pair.fx.push(second.clone());
    let both = render_stereo(&pair, &ev, frames, 512);
    let mut only_second = reset_saw_patch();
    second.id = id("chorus1");
    only_second.fx.push(second);
    let second_alone = render_stereo(&only_second, &ev, frames, 512);
    assert_ne!(bits(&both.0), bits(&reference.0));
    assert_ne!(bits(&both.0), bits(&second_alone.0));
    let again = render_stereo(&pair, &ev, frames, 512);
    assert_eq!(bits(&both.0), bits(&again.0), "deterministic");
}

// ---------------------------------------------------------------------------------------------------------------
// The tail, FTZ, allocation and the swap
// ---------------------------------------------------------------------------------------------------------------

const FRAMES_PER_SECOND: usize = 48_000;

/// A saw chord with a long release into a flanger at the longest delay and +99 % feedback (the slowest decay the
/// module has: the loop takes about 100 s to fall to 1e-15), then an ensemble.
fn tail_patch() -> Patch {
    let mut p = reset_saw_patch();
    p.modulators[0].values[enth_patch::catalogue::AdsrParam::Release.index()] = 3_000.0;
    let mut a = ModuleInstance::new(id("chorus1"), ModuleKind::Chorus);
    a.values[ChorusParam::Mode.index()] = ChorusMode::Flanger.index() as f32;
    a.values[ChorusParam::Delay.index()] = 30.0;
    a.values[ChorusParam::Feedback.index()] = 99.0;
    a.values[ChorusParam::Rate.index()] = 7.0;
    let mut b = ModuleInstance::new(id("chorus2"), ModuleKind::Chorus);
    b.values[ChorusParam::Mode.index()] = ChorusMode::Ensemble.index() as f32;
    b.values[ChorusParam::Feedback.index()] = -60.0;
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
fn a_40_s_tail_through_the_chorus_has_no_subnormals() {
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
    // The 99 % loop is still ringing at 40 s, quietly: it is not cut off.
    let late = &l[39 * FRAMES_PER_SECOND..];
    assert!(late.iter().any(|v| *v != 0.0), "the tail is still there");
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

    /// `process()` with a chorus in every mode, synced and free, with a tempo change, never allocates.
    #[test]
    fn process_does_not_allocate() {
        for mode in [
            ChorusMode::Chorus,
            ChorusMode::Ensemble,
            ChorusMode::Flanger,
        ] {
            for sync in [0.0f32, 1.0] {
                let mut p = tail_patch();
                p.fx[0].values[ChorusParam::Mode.index()] = mode.index() as f32;
                p.fx[0].values[ChorusParam::Sync.index()] = sync;
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
                assert!(left.iter().any(|x| *x != 0.0), "{mode:?}");
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

/// The delay lines move with the plan swap: recompiling the same patch under a held chord (an instant swap, the new
/// plan takes the old one's lines) and adding an unrelated module (a crossfading swap, both plans run on copies of
/// the lines) both give the render of no swap at all, bit for bit. A lost line would show as a gap in the sweep.
#[test]
fn the_plan_swap_carries_the_delay_lines() {
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
    // A different chorus setting at the swap is heard from there on, and only from there on.
    let mut changed = base.clone();
    changed.fx[0].values[ChorusParam::Delay.index()] = 20.0;
    let (l3, _) = chord_with_swap(&base, Some(&changed));
    let swap = 200 * SEG;
    assert_eq!(bits(&l3[..swap]), bits(&l0[..swap]));
    assert_ne!(bits(&l3[swap..]), bits(&l0[swap..]));
    assert!(l3.iter().all(|x| x.is_finite()));
}

// ---------------------------------------------------------------------------------------------------------------
// The cost of the copy a crossfading swap makes (information, not a gate)
// ---------------------------------------------------------------------------------------------------------------

/// Times `Chorus::copy_state_from`, the memcpy of a crossfading plan swap, at the sample rates the lines are sized for,
/// against the reverb's 1.6 MB of lines, warm and with the caches flushed in between (the state after a sleep: research
/// 03 found a paced copy 2 to 4 times slower than a back-to-back one). Run with
/// `flock -x /tmp/enth-quiet.lock cargo test -p enth-engine --release --test chorus swap_copy -- --ignored --nocapture`.
/// Wall-clock time is information only (docs/benchmarks.md, "Measurement hygiene"); `docs/dsp/chorus.md` quotes it.
#[test]
#[ignore = "timing, information only: run it by hand under the exclusive lock"]
fn swap_copy_cost_is_microseconds() {
    use enth_dsp::chorus::Chorus;
    use std::time::Instant;

    fn quantiles(mut us: Vec<f64>) -> (f64, f64, f64) {
        us.sort_by(f64::total_cmp);
        let at = |q: f64| us[((us.len() - 1) as f64 * q) as usize];
        (at(0.5), at(0.99), us[us.len() - 1])
    }
    // 32 MiB read between the cold copies, enough to evict an L2 and most of an L3 slice.
    let mut flush = vec![1u8; 32 << 20];
    let mut evict = |k: usize| {
        for v in flush.iter_mut().step_by(64) {
            *v = v.wrapping_add(k as u8);
        }
        std::hint::black_box(&flush);
    };
    for (label, sr) in [
        ("chorus 48 kHz (32 KiB)", 48_000.0f32),
        ("chorus 96 kHz (64 KiB)", 96_000.0),
        ("chorus 192 kHz (128 KiB)", 192_000.0),
    ] {
        let src = Chorus::new(sr);
        let mut dst = Chorus::new(sr);
        for cold in [false, true] {
            let mut us = Vec::new();
            for k in 0..2_000 {
                if cold {
                    evict(k);
                }
                let t = Instant::now();
                assert!(dst.copy_state_from(std::hint::black_box(&src)));
                us.push(t.elapsed().as_secs_f64() * 1e6);
            }
            let (p50, p99, max) = quantiles(us);
            eprintln!(
                "{label}, {}: p50 {p50:.1} us, p99 {p99:.1} us, max {max:.1} us",
                if cold { "cold" } else { "warm" }
            );
        }
    }
    // The reverb's six delay rings, 1.6 MB: the baseline the swap harness measured (research 03: 148 to 249 us alone).
    let src = vec![0.5f32; 400_000];
    let mut dst = vec![0.0f32; 400_000];
    for cold in [false, true] {
        let mut us = Vec::new();
        for k in 0..400 {
            if cold {
                evict(k);
            }
            let t = Instant::now();
            dst.copy_from_slice(std::hint::black_box(&src));
            std::hint::black_box(&dst);
            us.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let (p50, p99, max) = quantiles(us);
        eprintln!(
            "reverb 1.6 MB, {}: p50 {p50:.1} us, p99 {p99:.1} us, max {max:.1} us",
            if cold { "cold" } else { "warm" }
        );
    }
}
