//! `eq` through the engine (S2a lane EQ; `docs/dsp/eq.md`): the response of the whole chain against the Audio EQ
//! Cookbook's formulas written out here, the spectral aliasing of static settings, the zipper skirt of a swept band,
//! stability under a square LFO on every control (the fastest, hardest modulation the engine delivers), `mix` 0
//! passing the bus untouched, the idle skip, two instances with separate state, the 40 s tail with zero
//! subnormals, the render under every FTZ mode, the plan swap, and `process` under the no-allocation guard.
//!
//! The per-sample claims (the designs against the cookbook over the table's ranges, the processed response of every
//! band type, the passivity of the section update, the single-parameter and pair stability sweeps) are in
//! `enth_dsp::eq`'s unit tests, which can drive the core with exact signals; here the signal is a held `osc` sine,
//! saw chord or noise through the real parameter tables.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]
#![allow(
    clippy::manual_midpoint,
    reason = "the cookbook's formulas are written as the cookbook writes them, `(1 + cos w0) / 2` and all"
)]

mod common;

use common::{SEED, SR, bits, drive, engine, events, id, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep, midi_hz, power_spectrum};
use enth_dsp::util::math;
use enth_engine::module::{
    Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG, count_subnormals,
};
use enth_engine::modules::eq::Eq;
use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, NoteKey, compile};
use enth_patch::catalogue::{
    AdsrParam, EqParam, LfoParam, LfoScope, LfoShape, OscParam, Polarity, Wave,
};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

// ---------------------------------------------------------------------------------------------------------------
// The reference: the Audio EQ Cookbook, written here
// ---------------------------------------------------------------------------------------------------------------

/// The cookbook's biquad, normalised: `(b0, b1, b2, a1, a2)`.
type Rbj = [f64; 5];

#[derive(Clone, Copy)]
enum Kind {
    HighPass,
    LowPass,
    Peak,
    LowShelf,
    HighShelf,
}

/// RBJ's `HPF`, `LPF` (Q = 1/sqrt 2), `peakingEQ` (Q), `lowShelf` and `highShelf` (slope S = 1).
fn rbj(kind: Kind, f0: f64, db: f64, q: f64, sr: f64) -> Rbj {
    let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
    let (s, c) = (math::sin(w0), math::cos(w0));
    let a = math::pow(10.0, db / 40.0);
    let (b0, b1, b2, a0, a1, a2);
    match kind {
        Kind::LowPass => {
            let alpha = s / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
            (b0, b1, b2) = ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0);
            (a0, a1, a2) = (1.0 + alpha, -2.0 * c, 1.0 - alpha);
        }
        Kind::HighPass => {
            let alpha = s / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
            (b0, b1, b2) = ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0);
            (a0, a1, a2) = (1.0 + alpha, -2.0 * c, 1.0 - alpha);
        }
        Kind::Peak => {
            let alpha = s / (2.0 * q);
            (b0, b1, b2) = (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a);
            (a0, a1, a2) = (1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
        }
        Kind::LowShelf | Kind::HighShelf => {
            let slope = 1.0;
            let alpha = s / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
            let t = 2.0 * a.sqrt() * alpha;
            if matches!(kind, Kind::LowShelf) {
                b0 = a * ((a + 1.0) - (a - 1.0) * c + t);
                b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * c);
                b2 = a * ((a + 1.0) - (a - 1.0) * c - t);
                a0 = (a + 1.0) + (a - 1.0) * c + t;
                a1 = -2.0 * ((a - 1.0) + (a + 1.0) * c);
                a2 = (a + 1.0) + (a - 1.0) * c - t;
            } else {
                b0 = a * ((a + 1.0) + (a - 1.0) * c + t);
                b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * c);
                b2 = a * ((a + 1.0) + (a - 1.0) * c - t);
                a0 = (a + 1.0) - (a - 1.0) * c + t;
                a1 = 2.0 * ((a - 1.0) - (a + 1.0) * c);
                a2 = (a + 1.0) - (a - 1.0) * c - t;
            }
        }
    }
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// `|H|` of a normalised biquad at `f`, in dB.
fn magnitude_db(c: &Rbj, f: f64, sr: f64) -> f64 {
    let w = 2.0 * core::f64::consts::PI * f / sr;
    let (c1, s1, c2, s2) = (
        math::cos(w),
        math::sin(w),
        math::cos(2.0 * w),
        math::sin(2.0 * w),
    );
    let (nr, ni) = (c[0] + c[1] * c1 + c[2] * c2, -(c[1] * s1 + c[2] * s2));
    let (dr, di) = (1.0 + c[3] * c1 + c[4] * c2, -(c[3] * s1 + c[4] * s2));
    10.0 * math::log10((nr * nr + ni * ni) / (dr * dr + di * di))
}

/// The six cookbook sections for `settings` laid over the defaults, in dB at `f`.
fn cookbook_db(settings: &[(EqParam, f32)], f: f64) -> f64 {
    let mut v = ModuleKind::Eq.defaults();
    for &(p, x) in settings {
        v[p.index()] = x;
    }
    let g = |p: EqParam| f64::from(v[p.index()]);
    let sr = f64::from(SR);
    let sections = [
        rbj(Kind::HighPass, g(EqParam::LowCut), 0.0, 0.0, sr),
        rbj(
            Kind::LowShelf,
            g(EqParam::LowFreq),
            g(EqParam::LowGain),
            0.0,
            sr,
        ),
        rbj(
            Kind::Peak,
            g(EqParam::Bell1Freq),
            g(EqParam::Bell1Gain),
            g(EqParam::Bell1Q),
            sr,
        ),
        rbj(
            Kind::Peak,
            g(EqParam::Bell2Freq),
            g(EqParam::Bell2Gain),
            g(EqParam::Bell2Q),
            sr,
        ),
        rbj(
            Kind::HighShelf,
            g(EqParam::HighFreq),
            g(EqParam::HighGain),
            0.0,
            sr,
        ),
        rbj(Kind::LowPass, g(EqParam::HighCut), 0.0, 0.0, sr),
    ];
    sections.iter().map(|c| magnitude_db(c, f, sr)).sum()
}

// ---------------------------------------------------------------------------------------------------------------
// Patches and measurements
// ---------------------------------------------------------------------------------------------------------------

/// A sine oscillator into one `eq` with `settings` over its defaults.
fn eq_patch(settings: &[(EqParam, f32)]) -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let mut m = ModuleInstance::new(id("eq1"), ModuleKind::Eq);
    for &(param, v) in settings {
        m.values[param.index()] = v;
    }
    p.fx.push(m);
    p
}

/// The same sine without the `eq`.
fn sine_patch() -> Patch {
    let mut p = eq_patch(&[]);
    p.fx.clear();
    p
}

/// `key` held through `patch`, the left channel.
fn held(patch: &Patch, key: u8, frames: usize) -> Vec<f32> {
    render(patch, &[note(key, 0, frames as u64)], frames, 512, SEED).0
}

/// The power of the tone at `f0` in the last `FFT_LEN` samples of `x`: the sum over the window's main lobe.
fn tone_power(x: &[f32], f0: f64) -> f64 {
    let p = power_spectrum(&x[x.len() - FFT_LEN..]);
    let bin = f64::from(SR) / FFT_LEN as f64;
    let c = (f0 / bin).round() as usize;
    p[c - 5..=c + 5].iter().sum()
}

/// The gain the `eq` gave a held sine at `key`, in dB: the tone's power with it over the tone's power without.
fn measured_gain_db(settings: &[(EqParam, f32)], key: u8) -> f64 {
    let frames = 3 * FFT_LEN;
    let with = held(&eq_patch(settings), key, frames);
    let without = held(&sine_patch(), key, frames);
    10.0 * math::log10(
        tone_power(&with, midi_hz(f64::from(key))) / tone_power(&without, midi_hz(f64::from(key))),
    )
}

// ---------------------------------------------------------------------------------------------------------------
// The response
// ---------------------------------------------------------------------------------------------------------------

type Settings = &'static [(EqParam, f32)];

const SETTINGS: [(&str, Settings); 8] = [
    ("defaults", &[]),
    (
        "bell +12 dB at 440 Hz, Q 2",
        &[
            (EqParam::Bell1Freq, 440.0),
            (EqParam::Bell1Gain, 12.0),
            (EqParam::Bell1Q, 2.0),
        ],
    ),
    (
        "bell -18 dB at 1 kHz Q 8 and bell +9 dB at 5 kHz Q 0.7",
        &[
            (EqParam::Bell1Freq, 1_000.0),
            (EqParam::Bell1Gain, -18.0),
            (EqParam::Bell1Q, 8.0),
            (EqParam::Bell2Freq, 5_000.0),
            (EqParam::Bell2Gain, 9.0),
            (EqParam::Bell2Q, 0.7),
        ],
    ),
    (
        "low shelf +10 dB at 200 Hz and high shelf -8 dB at 6 kHz",
        &[
            (EqParam::LowFreq, 200.0),
            (EqParam::LowGain, 10.0),
            (EqParam::HighFreq, 6_000.0),
            (EqParam::HighGain, -8.0),
        ],
    ),
    (
        "low cut 250 Hz and high cut 5 kHz",
        &[(EqParam::LowCut, 250.0), (EqParam::HighCut, 5_000.0)],
    ),
    (
        "low-mid cut and air boost",
        &[
            (EqParam::Bell1Freq, 300.0),
            (EqParam::Bell1Gain, -5.0),
            (EqParam::Bell1Q, 1.2),
            (EqParam::HighFreq, 10_000.0),
            (EqParam::HighGain, 6.0),
        ],
    ),
    (
        "extreme gains: low +24, bell -24 at Q 20, bell +24 at Q 0.1, high -24",
        &[
            (EqParam::LowFreq, 300.0),
            (EqParam::LowGain, 24.0),
            (EqParam::Bell1Freq, 700.0),
            (EqParam::Bell1Gain, -24.0),
            (EqParam::Bell1Q, 20.0),
            (EqParam::Bell2Freq, 2_000.0),
            (EqParam::Bell2Gain, 24.0),
            (EqParam::Bell2Q, 0.1),
            (EqParam::HighFreq, 8_000.0),
            (EqParam::HighGain, -24.0),
        ],
    ),
    (
        "cuts at the middle of their ranges",
        &[(EqParam::LowCut, 100.0), (EqParam::HighCut, 2_000.0)],
    ),
];

#[test]
#[allow(
    clippy::print_stderr,
    reason = "the worst deviation docs/dsp/eq.md records (cargo test -p enth-engine --test eq -- --nocapture)"
)]
fn the_engine_response_is_within_a_tenth_of_a_decibel_of_the_cookbook() {
    let keys = [36u8, 48, 60, 69, 76, 84, 93, 100, 108, 116, 124];
    let mut worst = 0.0f64;
    for (name, settings) in SETTINGS {
        let mut worst_here = 0.0f64;
        for key in keys {
            let f = midi_hz(f64::from(key));
            let got = measured_gain_db(settings, key);
            let want = cookbook_db(settings, f);
            let d = (got - want).abs();
            assert!(
                d < 0.1,
                "{name} at {f:.0} Hz: {got:.3} dB against the cookbook's {want:.3}"
            );
            worst_here = worst_here.max(d);
        }
        eprintln!("{name}: worst difference {worst_here:.4} dB");
        worst = worst.max(worst_here);
    }
    eprintln!(
        "engine response against the cookbook: worst {worst:.4} dB over {} settings",
        SETTINGS.len()
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Aliasing
// ---------------------------------------------------------------------------------------------------------------

/// The keys of the sweeps: 110 Hz to 3.5 kHz.
const KEYS: [u8; 6] = [45, 57, 69, 81, 93, 105];

/// An equaliser is linear, so a static setting adds no aliasing: the alias measure of a sine through it is the
/// floor of the engine's own arithmetic (the oscillator, the envelope and the 32-bit bus), and it comes out the same
/// with or without the `eq`. The worst measure over [`KEYS`] with `Measure::REPORT_02` is -92.7 dB (the engine's
/// floor) for the three settings below and for no `eq` at all; the bound is that plus 1.5 dB
/// (`docs/dsp/aliasing.md`).
const FLOOR_BOUND_DB: f64 = -91.2;

const STATIC_SETTINGS: [(&str, Settings); 3] = [
    ("defaults", &[]),
    (
        "air boost and low-mid cut",
        &[
            (EqParam::Bell1Freq, 300.0),
            (EqParam::Bell1Gain, -5.0),
            (EqParam::Bell1Q, 1.2),
            (EqParam::HighFreq, 10_000.0),
            (EqParam::HighGain, 6.0),
        ],
    ),
    (
        "four bands at the table's corners",
        &[
            (EqParam::LowFreq, 300.0),
            (EqParam::LowGain, 24.0),
            (EqParam::Bell1Freq, 700.0),
            (EqParam::Bell1Gain, -24.0),
            (EqParam::Bell1Q, 20.0),
            (EqParam::Bell2Freq, 2_000.0),
            (EqParam::Bell2Gain, 24.0),
            (EqParam::Bell2Q, 0.1),
            (EqParam::HighFreq, 8_000.0),
            (EqParam::HighGain, -24.0),
        ],
    ),
];

fn alias_sweep(p: &Patch) -> Sweep {
    Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |k| {
        held(p, k, FFT_LEN + 8_192)
    })
}

#[test]
#[allow(
    clippy::print_stderr,
    reason = "the table docs/dsp/eq.md records (cargo test -p enth-engine --test eq -- --nocapture)"
)]
fn static_settings_alias_no_more_than_the_engine_floor() {
    let floor = alias_sweep(&sine_patch());
    eprintln!("no eq (the engine's floor):\n{}", floor.table());
    floor.assert_below(FLOOR_BOUND_DB, "no eq");
    for (name, settings) in STATIC_SETTINGS {
        let s = alias_sweep(&eq_patch(settings));
        eprintln!("{name}:\n{}", s.table());
        s.assert_below(FLOOR_BOUND_DB, name);
        // And point by point no more than 3 dB above the floor of the same key: the equaliser adds nothing.
        for (got, base) in s.points.iter().zip(&floor.points) {
            assert!(
                got.1 < base.1 + 3.0,
                "{name} at {:.0} Hz: {:.1} against {:.1}",
                got.0,
                got.1,
                base.1
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Modulation
// ---------------------------------------------------------------------------------------------------------------

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

/// A global LFO `lfo1` of `shape` at `rate_hz`, `polarity`.
fn global_lfo(shape: LfoShape, rate_hz: f32, polarity: Polarity) -> ModuleInstance {
    let mut lfo = ModuleInstance::new(id("lfo1"), ModuleKind::Lfo);
    lfo.values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
    lfo.values[LfoParam::Shape.index()] = shape.index() as f32;
    lfo.values[LfoParam::Rate.index()] = rate_hz;
    lfo.values[LfoParam::Polarity.index()] = polarity.index() as f32;
    lfo
}

/// The strongest bin in three bands 2-4, 4-8 and 8-16 kHz above the carrier, over the carrier's strongest bin, in dB
/// (the MSEG page's measure: `docs/dsp/mseg.md`).
fn skirt_db(patch: &Patch, key: u8) -> [f64; 3] {
    let frames = 4 * FFT_LEN;
    let l = held(patch, key, frames);
    let p = power_spectrum(&l[frames - FFT_LEN..]);
    let bin = f64::from(SR) / FFT_LEN as f64;
    let f0 = midi_hz(f64::from(key));
    let at = |hz: f64| (hz / bin).round() as usize;
    let peak = p[at(f0) - 8..=at(f0) + 8]
        .iter()
        .fold(0.0f64, |m, &x| m.max(x));
    [(2_000.0, 4_000.0), (4_000.0, 8_000.0), (8_000.0, 16_000.0)].map(|(lo, hi)| {
        let worst = p[at(f0 + lo)..=at(f0 + hi)]
            .iter()
            .fold(0.0f64, |m, &x| m.max(x));
        10.0 * math::log10(worst / peak)
    })
}

/// A bell on a sine, swept by a global LFO: the bell's centre, its gain or its Q.
fn swept(target: EqParam, key: u8, rate_hz: f32, depth: f32) -> Patch {
    // A boost of 15 dB and Q 4 on the carrier, moved slowly enough that the carrier's own amplitude and phase
    // modulation stays within tens of Hz: whatever lies kHz away is the stepping of the controls.
    let f0 = midi_hz(f64::from(key)) as f32;
    let mut p = eq_patch(&[
        (EqParam::Bell1Freq, f0),
        (EqParam::Bell1Gain, 15.0),
        (EqParam::Bell1Q, 4.0),
    ]);
    p.modulators
        .push(global_lfo(LfoShape::Sine, rate_hz, Polarity::Bipolar));
    p.routes
        .push(route("lfo1", &format!("eq1.{}", target.spec().key), depth));
    p
}

/// The skirts of a swept bell are bounded: the ramps inside the segment turn the 32-frame steps of the parameter
/// into a smooth line (the steps put sidebands at multiples of 1.5 kHz; `enth_dsp::eq` measures ramped against
/// stepped). A bell of +15 dB and Q 4 on the carrier, its centre (+-0.24 decades), gain (+-12 dB) or Q (about
/// 0.7..20) swept by a global sine LFO at 5 and at 25 Hz, over two carriers.
#[test]
#[allow(
    clippy::print_stderr,
    reason = "the table docs/dsp/eq.md records (cargo test -p enth-engine --test eq -- --nocapture)"
)]
fn a_swept_bell_has_a_deep_skirt() {
    let mut worst = [[f64::NEG_INFINITY; 3]; 2];
    for (r, rate) in [5.0f32, 25.0].into_iter().enumerate() {
        for (target, depth) in [
            (EqParam::Bell1Freq, 8.0),
            (EqParam::Bell1Gain, 25.0),
            (EqParam::Bell1Q, 30.0),
        ] {
            for key in [69u8, 93] {
                let got = skirt_db(&swept(target, key, rate, depth), key);
                eprintln!(
                    "  {} swept at {rate} Hz, key {key}: {:.1} / {:.1} / {:.1}",
                    target.spec().key,
                    got[0],
                    got[1],
                    got[2]
                );
                for (w, g) in worst[r].iter_mut().zip(got) {
                    *w = w.max(g);
                }
            }
        }
        eprintln!(
            "  worst at {rate} Hz: {:.1} / {:.1} / {:.1}",
            worst[r][0], worst[r][1], worst[r][2]
        );
    }
    for (w, b) in worst.iter().zip(SKIRT_BOUNDS) {
        for (g, b) in w.iter().zip(b) {
            assert!(*g <= b, "{worst:?} against {SKIRT_BOUNDS:?}");
        }
    }
}

/// The recorded skirt bounds, dB re the carrier in the 2-4, 4-8 and 8-16 kHz bands above it, for the 5 Hz and the
/// 25 Hz sweeps: the worst measure of each plus 2 dB (measured 2026-10-27: -110.5 / -117.4 / -128.6 and -82.6 /
/// -89.5 / -100.8).
const SKIRT_BOUNDS: [[f64; 3]; 2] = [[-108.5, -115.4, -126.6], [-80.6, -87.5, -98.8]];

// ---------------------------------------------------------------------------------------------------------------
// Stability under the hardest modulation the engine delivers
// ---------------------------------------------------------------------------------------------------------------

/// A held chord of saws and noise through an `eq` whose settings are `base`; with `lfo` a unipolar square wave at
/// 50 Hz (the LFO's top rate; each edge is a jump over the control's whole range, which the engine delivers as a
/// 10 ms smoothed line and the core ramps inside every segment) at full depth on `target`, starting from its
/// minimum.
fn stress_patch(base: &[(EqParam, f32)], lfo: Option<EqParam>) -> Patch {
    let mut p = Patch::init();
    p.generators
        .push(ModuleInstance::new(id("noise1"), ModuleKind::Noise));
    let mut m = ModuleInstance::new(id("eq1"), ModuleKind::Eq);
    for &(param, v) in base {
        m.values[param.index()] = v;
    }
    p.fx.push(m);
    if let Some(target) = lfo {
        p.modulators
            .push(global_lfo(LfoShape::Square, 50.0, Polarity::Unipolar));
        p.routes
            .push(route("lfo1", &format!("eq1.{}", target.spec().key), 100.0));
    }
    p
}

fn peak_after(patch: &Patch, from: usize, frames: usize) -> f32 {
    let chord = [
        note(36, 0, frames as u64),
        note(43, 0, frames as u64),
        note(55, 0, frames as u64),
    ];
    let (l, r, _) = render(patch, &chord, frames, 512, SEED);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    l[from..]
        .iter()
        .chain(&r[from..])
        .fold(0.0f32, |m, x| m.max(x.abs()))
}

#[test]
#[allow(
    clippy::print_stderr,
    reason = "the table docs/dsp/eq.md records (cargo test -p enth-engine --test eq -- --nocapture)"
)]
fn a_square_lfo_on_any_control_stays_within_a_few_times_the_frozen_ends() {
    let frames = 4 * 48_000;
    let from = 24_000;
    // (the modulated control, the settings that make it matter most)
    let cases: [(EqParam, Vec<(EqParam, f32)>); 8] = [
        (EqParam::LowCut, vec![]),
        (EqParam::LowFreq, vec![(EqParam::LowGain, 24.0)]),
        (EqParam::LowGain, vec![]),
        (
            EqParam::Bell1Freq,
            vec![(EqParam::Bell1Gain, 24.0), (EqParam::Bell1Q, 20.0)],
        ),
        (
            EqParam::Bell1Gain,
            vec![(EqParam::Bell1Freq, 100.0), (EqParam::Bell1Q, 20.0)],
        ),
        (
            EqParam::Bell1Q,
            vec![(EqParam::Bell1Freq, 100.0), (EqParam::Bell1Gain, 24.0)],
        ),
        (EqParam::HighFreq, vec![(EqParam::HighGain, 24.0)]),
        (EqParam::HighCut, vec![]),
    ];
    let mut worst = 0.0f32;
    for (target, base) in cases {
        let spec = target.spec();
        let with_value = |v: f32| -> Vec<(EqParam, f32)> {
            let mut s = base.clone();
            s.retain(|(p, _)| *p != target);
            s.push((target, v));
            s
        };
        // The LFO is unipolar from the minimum: the control toggles between its minimum and its maximum.
        let moving = peak_after(
            &stress_patch(&with_value(spec.min()), Some(target)),
            from,
            frames,
        );
        let frozen = [spec.min(), spec.max()]
            .into_iter()
            .map(|v| peak_after(&stress_patch(&with_value(v), None), from, frames))
            .fold(0.0f32, f32::max);
        eprintln!(
            "{}: moving peak {moving:.2}, frozen ends {frozen:.2}, ratio {:.2}",
            spec.key,
            moving / frozen
        );
        assert!(
            moving < 4.0 * frozen,
            "{}: {moving} against {frozen}",
            spec.key
        );
        worst = worst.max(moving / frozen);
    }
    eprintln!("square LFO at 50 Hz on one control: worst moving/frozen ratio {worst:.2}");
}

// ---------------------------------------------------------------------------------------------------------------
// The module contract
// ---------------------------------------------------------------------------------------------------------------

/// `mix` 0 passes the bus bit for bit, whatever the bands do (the sections keep running).
#[test]
fn mix_zero_passes_the_engine_output_bit_for_bit() {
    let busy = [
        (EqParam::LowGain, 24.0),
        (EqParam::Bell1Gain, -24.0),
        (EqParam::Bell1Q, 20.0),
        (EqParam::Bell2Gain, 24.0),
        (EqParam::HighGain, -24.0),
        (EqParam::LowCut, 400.0),
        (EqParam::Mix, 0.0),
    ];
    // The bell is swept by a global LFO while the mix is 0, so the sections do move; the other patch has no `eq`
    // and no LFO at all.
    let with = stress_patch(&busy, Some(EqParam::Bell1Freq));
    let mut without = with.clone();
    without.fx.clear();
    without.routes.clear();
    without.modulators.retain(|m| m.kind != ModuleKind::Lfo);
    let chord = [
        note(36, 0, 20_000),
        note(43, 2_000, 20_000),
        note(55, 4_000, 18_000),
    ];
    let (l0, r0, _) = render(&without, &chord, 30_000, 512, SEED);
    let (l1, r1, _) = render(&with, &chord, 30_000, 512, SEED);
    assert!(l0.iter().any(|x| x.abs() > 0.01));
    assert_eq!(bits(&l1), bits(&l0));
    assert_eq!(bits(&r1), bits(&r0));
}

/// A module and the parameter values it sees.
struct Rig {
    module: Eq,
    prev: Vec<f32>,
    cur: Vec<f32>,
}

impl Rig {
    fn new(sample_rate: f32, vals: &[f32]) -> Self {
        let mut rig = Self {
            module: Eq::new(vals, &PrepareCfg::new(sample_rate)),
            prev: vals.to_vec(),
            cur: vals.to_vec(),
        };
        let params = GlobalParams::from_slices(vals, vals, vals);
        rig.module.reset(&GlobalStart {
            seed: 1,
            module_key: 2,
            params,
        });
        rig
    }

    /// Run one segment with `cur` as the new values (the old `cur` becomes `prev`).
    fn run(&mut self, cur: &[f32], l: &mut Block, r: &mut Block) {
        self.prev.clone_from(&self.cur);
        self.cur.clear();
        self.cur.extend_from_slice(cur);
        let params = GlobalParams::from_slices(&self.cur, &self.cur, &self.prev);
        self.module.process(&mut GlobalCx {
            tempo_bpm: enth_engine::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: l,
            right: r,
        });
    }

    /// The way the engine drives it: an idle module with silent input is skipped, but the values still advance.
    fn run_like_the_engine(&mut self, cur: &[f32], l: &mut Block, r: &mut Block) -> bool {
        let zero = |b: &Block| b.iter().all(|x| *x == 0.0);
        if self.module.is_silent() && zero(l) && zero(r) {
            self.prev.clone_from(&self.cur);
            self.cur.clear();
            self.cur.extend_from_slice(cur);
            return false;
        }
        self.run(cur, l, r);
        true
    }
}

fn noise_segment(seed: &mut u64, amp: f32) -> Block {
    let mut b = [0.0f32; SEG];
    for v in &mut b {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *v = ((*seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * amp;
    }
    b
}

fn settings(set: &[(EqParam, f32)]) -> Vec<f32> {
    let mut v = ModuleKind::Eq.defaults();
    for &(p, x) in set {
        v[p.index()] = x;
    }
    v
}

/// A burst, then silence while the controls keep moving: the engine's skip (an idle module with silent input is not
/// called) gives the same bits as calling it every segment, and the same state at the end.
#[test]
fn an_idle_eq_skipped_by_the_engine_gives_the_same_bits_as_one_that_is_always_called() {
    let a = settings(&[
        (EqParam::Bell1Gain, 12.0),
        (EqParam::Bell1Q, 6.0),
        (EqParam::HighGain, 6.0),
        (EqParam::Mix, 80.0),
    ]);
    let b = settings(&[
        (EqParam::Bell1Gain, -9.0),
        (EqParam::Bell1Freq, 3_000.0),
        (EqParam::LowCut, 90.0),
        (EqParam::Mix, 100.0),
    ]);
    let mut always = Rig::new(48_000.0, &a);
    let mut skipping = Rig::new(48_000.0, &a);
    let mut seed = 7u64;
    let (mut skipped, mut ran_idle) = (0usize, 0usize);
    for k in 0..(60 * 48_000 / SEG) {
        let (mut l, mut r) = if k < 200 {
            (noise_segment(&mut seed, 0.8), noise_segment(&mut seed, 0.8))
        } else {
            ([0.0; SEG], [0.0; SEG])
        };
        let (mut l2, mut r2) = (l, r);
        let cur = if (k / 40) % 2 == 0 { &a } else { &b };
        always.run(cur, &mut l, &mut r);
        if !skipping.run_like_the_engine(cur, &mut l2, &mut r2) {
            skipped += 1;
            if always.module.is_silent() {
                ran_idle += 1;
            }
        }
        assert_eq!(bits(&l), bits(&l2), "left at segment {k}");
        assert_eq!(bits(&r), bits(&r2), "right at segment {k}");
    }
    assert!(
        skipped > 1_000,
        "the tail ends and the module is skipped: {skipped}"
    );
    assert!(ran_idle > 1_000);
    assert!(always.module.is_silent() && skipping.module.is_silent());
}

/// Two instances are two states: interleaved on different signals and settings each gives what it gives alone.
#[test]
fn two_instances_keep_separate_state() {
    let va = settings(&[
        (EqParam::Bell1Gain, 14.0),
        (EqParam::Bell1Q, 9.0),
        (EqParam::Bell1Freq, 600.0),
    ]);
    let vb = settings(&[
        (EqParam::LowGain, -12.0),
        (EqParam::HighCut, 3_000.0),
        (EqParam::Mix, 70.0),
    ]);
    let mut solo_a = Rig::new(48_000.0, &va);
    let mut solo_b = Rig::new(44_100.0, &vb);
    let mut both_a = Rig::new(48_000.0, &va);
    let mut both_b = Rig::new(44_100.0, &vb);
    let (mut sa, mut sb) = (11u64, 22u64);
    for k in 0..4_000 {
        let (mut la, mut ra) = (noise_segment(&mut sa, 0.7), noise_segment(&mut sa, 0.7));
        let (mut lb, mut rb) = (noise_segment(&mut sb, 0.3), noise_segment(&mut sb, 0.3));
        let (mut la2, mut ra2, mut lb2, mut rb2) = (la, ra, lb, rb);
        solo_a.run(&va, &mut la, &mut ra);
        both_a.run(&va, &mut la2, &mut ra2);
        solo_b.run(&vb, &mut lb, &mut rb);
        both_b.run(&vb, &mut lb2, &mut rb2);
        assert_eq!(bits(&la), bits(&la2), "A at segment {k}");
        assert_eq!(bits(&lb), bits(&lb2), "B at segment {k}");
        assert_eq!(bits(&ra), bits(&ra2));
        assert_eq!(bits(&rb), bits(&rb2));
    }
    // And they differ from each other (the test is not vacuous).
    let (mut l, mut r) = ([0.5; SEG], [0.5; SEG]);
    let (mut l2, mut r2) = (l, r);
    both_a.run(&va, &mut l, &mut r);
    both_b.run(&vb, &mut l2, &mut r2);
    assert_ne!(bits(&l), bits(&l2));
}

/// Two `eq` modules in one patch run in order, each with its own state: the pair differs from either alone and a
/// second render is bit-identical.
#[test]
fn two_modules_in_one_patch() {
    let mut p = eq_patch(&[(EqParam::Bell1Gain, 12.0), (EqParam::Bell1Q, 3.0)]);
    let mut second = ModuleInstance::new(id("eq2"), ModuleKind::Eq);
    second.values[EqParam::HighCut.index()] = 3_000.0;
    second.values[EqParam::LowGain.index()] = 9.0;
    p.fx.push(second);
    let mut alone = p.clone();
    alone.fx.truncate(1);
    let n = [note(48, 0, 20_000), note(60, 0, 20_000)];
    let (l_pair, _, _) = render(&p, &n, 24_000, 512, SEED);
    let (l_alone, _, _) = render(&alone, &n, 24_000, 512, SEED);
    assert!(l_pair.iter().all(|s| s.is_finite()));
    assert!(l_pair.iter().any(|s| s.abs() > 0.05));
    assert_ne!(bits(&l_pair), bits(&l_alone));
    let (again, _, _) = render(&p, &n, 24_000, 512, SEED);
    assert_eq!(bits(&l_pair), bits(&again));
}

// ---------------------------------------------------------------------------------------------------------------
// The tail, the FTZ modes
// ---------------------------------------------------------------------------------------------------------------

const FRAMES_PER_SECOND: usize = 48_000;

/// A chord with a long release through the longest-ringing setting the table allows (a +24 dB Q 20 bell at 20 Hz
/// with a +24 dB low shelf there), swept by a global LFO, for the tail and FTZ tests.
fn tail_patch() -> Patch {
    let mut p = eq_patch(&[
        (EqParam::Bell1Freq, 20.0),
        (EqParam::Bell1Gain, 24.0),
        (EqParam::Bell1Q, 20.0),
        (EqParam::LowFreq, 20.0),
        (EqParam::LowGain, 24.0),
        (EqParam::Bell2Freq, 3_000.0),
        (EqParam::Bell2Gain, -12.0),
        (EqParam::Bell2Q, 8.0),
    ]);
    p.generators[0].values[OscParam::Wave.index()] = Wave::Saw.index() as f32;
    p.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    p.modulators
        .push(global_lfo(LfoShape::SmoothRandom, 3.0, Polarity::Bipolar));
    p.routes.push(route("lfo1", "eq1.bell2_freq", 40.0));
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
fn a_40_s_tail_through_eq_has_no_subnormals() {
    let mut e = engine(&tail_patch(), SEED);
    let (mut sub_out, mut state_max, mut non_finite) = (0, 0, 0);
    let mut calls = 0usize;
    drive(
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
    assert_eq!(non_finite, 0, "non-finite samples");
    assert_eq!(e.active_voices().len(), 0, "voices must end");
}

#[cfg(target_arch = "x86_64")]
mod fp {
    use core::arch::asm;
    pub const FTZ: u64 = 1 << 15;
    pub const DAZ: u64 = 1 << 6;
    pub const MASK: u64 = FTZ | DAZ;
    pub fn get() -> u64 {
        let mut v: u32 = 0;
        // SAFETY: stmxcsr stores the 32-bit MXCSR to the address in the register; `v` is a valid u32.
        unsafe { asm!("stmxcsr [{}]", in(reg) &raw mut v, options(nostack, preserves_flags)) };
        u64::from(v)
    }
    pub fn set(v: u64) {
        let v = v as u32;
        // SAFETY: ldmxcsr loads MXCSR from a valid u32; only the FTZ/DAZ bits differ from the current value.
        unsafe { asm!("ldmxcsr [{}]", in(reg) &raw const v, options(nostack, preserves_flags)) };
    }
    pub const MODES: [(&str, u64); 3] = [
        ("IEEE (FTZ off)", 0),
        ("FTZ only (nice-plug)", FTZ),
        ("FTZ+DAZ", FTZ | DAZ),
    ];
}

#[cfg(target_arch = "aarch64")]
mod fp {
    use core::arch::asm;
    pub const FZ: u64 = 1 << 24;
    pub const MASK: u64 = FZ;
    pub fn get() -> u64 {
        let v: u64;
        // SAFETY: reads the FPCR system register.
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nostack, preserves_flags)) };
        v
    }
    pub fn set(v: u64) {
        // SAFETY: writes the FPCR system register; only FZ differs from the current value.
        unsafe { asm!("msr fpcr, {}", in(reg) v, options(nostack)) };
    }
    pub const MODES: [(&str, u64); 2] = [("IEEE (FZ off)", 0), ("FZ on", FZ)];
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod ftz {
    use super::*;

    /// Restores the previous FTZ/DAZ bits even if the closure panics.
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }

    fn with_mode<R>(bits: u64, f: impl FnOnce() -> R) -> R {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | bits);
        // Through `black_box` before the guard restores the old mode, so the optimiser cannot sink the arithmetic
        // past the restore (tests/tail.rs).
        std::hint::black_box(f())
    }

    /// The render of the tail patch is the same under every FTZ/DAZ mode: every state is flushed in code, so no
    /// subnormal ever reaches the arithmetic.
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
        let reference = with_mode(0, run);
        for (name, mode_bits) in fp::MODES {
            assert!(with_mode(mode_bits, run) == reference, "under {name}");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The plan swap
// ---------------------------------------------------------------------------------------------------------------

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
    let key = |k: u8, i: i32| NoteKey::new(k, i);
    let mut left = vec![0.0; 600 * SEG];
    for k in 0..600 {
        if k == 200
            && let Some(p) = edit.take()
        {
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        let ev: Vec<Event> = match k {
            0 => [48, 55, 60]
                .iter()
                .enumerate()
                .map(|(i, &n)| Event {
                    offset: 0,
                    kind: EventKind::NoteOn {
                        key: key(n, i as i32),
                        velocity: 0.8,
                    },
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut right = [0.0f32; SEG];
        e.process(&mut left[k * SEG..(k + 1) * SEG], &mut right, &ev);
    }
    (left, e)
}

#[test]
#[allow(
    clippy::print_stderr,
    reason = "the sizes docs/dsp/eq.md records (cargo test -p enth-engine --test eq -- --nocapture)"
)]
fn the_plan_swap_carries_the_eq_state() {
    let mut base = tail_patch();
    base.modulators[0].values[AdsrParam::Release.index()] = 300.0;
    let (l0, _) = chord_with_swap(&base, None);
    assert!(l0.iter().any(|x| *x != 0.0));
    // The same patch again: an instant swap, and the state moves with the module (by id and kind): the same bits.
    let (l1, e) = chord_with_swap(&base, Some(&base));
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 0);
    assert_eq!(bits(&l1), bits(&l0));
    // A value edit is an instant swap too, and the render is the new setting's own from then on, continuous at the
    // swap (the state carried).
    let mut edited = base.clone();
    edited.fx[0].values[EqParam::Bell2Gain.index()] = -9.0;
    let (l2, e) = chord_with_swap(&base, Some(&edited));
    assert_eq!(e.stats().crossfades, 0);
    let swap = 200 * SEG;
    assert_eq!(
        bits(&l0[..swap]),
        bits(&l2[..swap]),
        "nothing changes before the swap"
    );
    let jump = (l2[swap] - l2[swap - 1]).abs();
    let step = (l0[swap] - l0[swap - 1]).abs();
    eprintln!(
        "edit at the swap: first-sample step {jump:.4} against {step:.4} in the unedited render"
    );
    assert!(jump < step + 0.05);
    // A module added elsewhere (an unrouted voice LFO) makes the swap crossfade, and the surviving `eq`'s state is
    // copied into the new plan: the render stays on the unswapped one to within the crossfade's own rounding.
    let mut added = base.clone();
    added
        .modulators
        .push(ModuleInstance::new(id("lfo2"), ModuleKind::Lfo));
    let (l3, e) = chord_with_swap(&base, Some(&added));
    assert_eq!(e.stats().crossfades, 1, "the added module crossfades");
    let gap = l3
        .iter()
        .zip(&l0)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    eprintln!(
        "crossfading swap: largest difference from the unswapped render {gap:.2e}; the module's state is {} bytes",
        core::mem::size_of::<enth_dsp::eq::Eq>()
    );
    assert!(gap < 1e-3, "{gap}");
}

// ---------------------------------------------------------------------------------------------------------------
// No allocation
// ---------------------------------------------------------------------------------------------------------------

#[cfg(debug_assertions)]
mod guarded {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// `process()` with an `eq` that is being swept, with a note starting mid-render, never allocates.
    #[test]
    fn process_does_not_allocate() {
        let mut p = tail_patch();
        p.routes.push(route("lfo1", "eq1.bell1_q", 30.0));
        p.routes.push(route("lfo1", "eq1.low_cut", 20.0));
        p.routes.push(route("lfo1", "eq1.mix", 10.0));
        let mut e = engine(&p, SEED);
        let ev = events(&[note(48, 0, 4_000), note(64, 500, 6_000)]);
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
        assert!(left.iter().any(|x| *x != 0.0));
    }

    /// The module on its own: a segment through `Eq::process` under the guard, then a copy of its state.
    #[test]
    fn the_module_and_its_state_copy_do_not_allocate() {
        let vals = settings(&[(EqParam::Bell1Gain, 12.0), (EqParam::HighCut, 9_000.0)]);
        let mut a = Rig::new(48_000.0, &vals);
        let mut b = Eq::new(&vals, &PrepareCfg::new(48_000.0));
        let (mut l, mut r) = ([0.3; SEG], [-0.2; SEG]);
        assert_no_alloc(|| {
            for _ in 0..16 {
                a.run(&vals, &mut l, &mut r);
            }
            b.copy_state_from(&a.module);
        });
        assert!(!b.is_silent());
    }
}
