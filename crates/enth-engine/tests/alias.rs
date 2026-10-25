//! The spectral aliasing harness (`enth_dsp::alias`, lane S0d, freeze F0m) through the engine: the pattern S2a's
//! module lanes copy into their own test files (`docs/dsp/aliasing.md`), calibrated here against the numbers
//! `docs/dsp/osc.md` records for the oscillators, plus an effect measured the way the S2a effects will be: a sine
//! into a nonlinearity (the filter's drive) at a sweep of keys.

mod common;

use common::{SEED, SR, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep};
use enth_patch::catalogue::{OscParam, SvfParam, Wave};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, Patch};

/// `key` held through `patch`, long enough that the harness's window (the last 32,768 samples) starts after the
/// attack: the left channel.
fn held(patch: &Patch, key: u8) -> Vec<f32> {
    let frames = FFT_LEN + 8_192;
    let (left, _, _) = render(patch, &[note(key, 0, frames as u64)], frames, 512, SEED);
    left
}

/// `Patch::init()` with its oscillator on a sine and one module of `kind` added in its section, at its defaults: an
/// effect's test patch (a `voice_fx` kind processes each voice, an `fx` kind the voice sum).
fn sine_into(kind: ModuleKind) -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let id = ModuleId::new(&format!("{}1", kind.spec().id_prefix)).unwrap();
    p.section_mut(kind.section())
        .push(ModuleInstance::new(id, kind));
    p
}

/// Calibration: the harness through the engine reproduces osc.md's saw (-40.6 dB at key 69, -28.8 dB at key 105).
#[test]
fn the_saw_measures_what_osc_md_records() {
    let saw = Patch::init();
    let sweep = Sweep::run_keys(&[69, 105], f64::from(SR), &Measure::REPORT_02, |key| {
        held(&saw, key)
    });
    eprintln!("polyBLEP saw through the engine:\n{}", sweep.table());
    assert!((sweep.points[0].1 + 40.6).abs() < 1.0, "{}", sweep.table());
    assert!((sweep.points[1].1 + 28.8).abs() < 1.0, "{}", sweep.table());
    sweep.assert_below(-27.0, "the polyBLEP saw (osc.md's ceiling)");
}

/// The sine the S2a effects are fed is clean: whatever an effect adds between the harmonics is the effect's.
#[test]
fn the_engine_sine_is_a_clean_test_input() {
    let mut sine = Patch::init();
    sine.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    let sweep = Sweep::run_keys(
        &[33, 57, 81, 105],
        f64::from(SR),
        &Measure::REPORT_02,
        |key| held(&sine, key),
    );
    eprintln!("engine sine:\n{}", sweep.table());
    sweep.assert_below(-90.0, "the engine's sine");
}

/// An effect, measured as an S2a effect lane will: a sine into the filter's drive (a per-sample saturator at 24 dB),
/// keys over five octaves. The saturator runs at the base rate with no oversampling, so its folded harmonics show at
/// the top key; the bound records today's numbers with a margin. A `dist` lane's test is the same with
/// `sine_into(ModuleKind::Dist)` and its own bound.
#[test]
fn a_driven_filter_is_measured_like_an_s2a_effect() {
    let mut p = sine_into(ModuleKind::Svf);
    let svf = &mut p.voice_fx[0].values;
    svf[SvfParam::Drive.index()] = 24.0;
    svf[SvfParam::Cutoff.index()] = 20_000.0;
    let sweep = Sweep::run_keys(
        &[45, 57, 69, 81, 93],
        f64::from(SR),
        &Measure::REPORT_02,
        |key| held(&p, key),
    );
    eprintln!("svf, 24 dB drive, sine in:\n{}", sweep.table());
    sweep.assert_below(BOUND_SVF_DRIVE_DB, "svf drive 24 dB, sine in");
}

/// Measured on 2026-10-25 (S0d): -100.5, -93.9, -58.2, -37.3 and -25.2 dB at 110, 220, 440, 880 and 1,760 Hz; the
/// bound is the worst plus 1.5 dB (`docs/dsp/aliasing.md`).
const BOUND_SVF_DRIVE_DB: f64 = -23.7;
