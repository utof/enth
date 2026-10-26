//! `wt` end to end (S2a lane WT): what the generated tables, phase modulation, amplitude modulation, ring modulation
//! and feedback *are*, measured against closed forms, not against a previous render (a golden proves "same as
//! before", not "correct"); the aliasing sweeps with the bounds `docs/dsp/wt.md` records; and the module contract
//! (separate state, host-block independence, a self-route refused, a 40 s tail with no subnormals, FTZ-independent
//! bits, the unison knobs inert while `unison` is 1).
//!
//! The rig: a modulator `osc1` (a sine at `level` 0, so it is only heard through the route) and a carrier `wt1`, under
//! a gate envelope, so the amplitude is constant. Both play the same key; the modulator's `semi` sets the frequency
//! ratio. The measurements use `enth_dsp::alias`: a 32,768-sample Blackman-Harris spectrum, a component's amplitude
//! being the square root of the power in ±6 bins around it over the same sum for a unit sine at that frequency.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]
#![allow(
    clippy::print_stderr,
    reason = "the sweeps print the tables docs/dsp/wt.md records (cargo test -p enth-engine --test wt -- --nocapture)"
)]

mod common;

use std::fmt::Write as _;

use common::{SEED, SR, bits, drive, engine, events, id, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, alias_db, midi_hz, power_spectrum, sine};
use enth_dsp::util::math;
use enth_engine::module::count_subnormals;
use enth_engine::{PrepareCfg, compile};
use enth_patch::catalogue::{AdsrParam, OscParam, PhaseMode, Warp, Wave, WtParam, WtTable};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

const SRATE: f64 = SR as f64;
/// Frames rendered per measurement: the attack-free analysis window plus a margin.
const FRAMES: usize = FFT_LEN + 8_192;

// ---------------------------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------------------------

/// `Patch::init()` without generators, under a gate envelope (no attack, decay or release; sustain 100 %).
fn gated() -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 0.0;
    p
}

/// A sine modulator `osc` at `semi` semitones from the key, at level 0 (heard only through its routes), phase 0.
fn modulator(name: &str, semi: f32) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Osc);
    m.values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    m.values[OscParam::PhaseMode.index()] = PhaseMode::Reset.index() as f32;
    m.values[OscParam::Semi.index()] = semi;
    m.values[OscParam::Level.index()] = 0.0;
    m
}

/// A `wt` at its defaults (a sine at full level) with `set` applied.
fn wt(name: &str, set: &[(WtParam, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Wt);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    m
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

/// `osc1` (at `semi`) routed with `depth` % into `wt1.pm`; `wt1` has `set`.
fn pm_rig(semi: f32, depth: f32, set: &[(WtParam, f32)]) -> Patch {
    let mut p = gated();
    p.generators.push(modulator("osc1", semi));
    p.generators.push(wt("wt1", set));
    p.routes.push(route("osc1", "wt1.pm", depth));
    p
}

/// The same with the route into `wt1.am`.
fn am_rig(semi: f32, depth: f32, set: &[(WtParam, f32)]) -> Patch {
    let mut p = gated();
    p.generators.push(modulator("osc1", semi));
    p.generators.push(wt("wt1", set));
    p.routes.push(route("osc1", "wt1.am", depth));
    p
}

/// A lone `wt1`.
fn lone(set: &[(WtParam, f32)]) -> Patch {
    let mut p = gated();
    p.generators.push(wt("wt1", set));
    p
}

/// `key` held through `patch`: the left channel.
fn held(patch: &Patch, key: u8) -> Vec<f32> {
    let (left, _, _) = render(patch, &[note(key, 0, FRAMES as u64)], FRAMES, 512, SEED);
    left
}

/// Power in ±6 bins around `f` Hz in the last `FFT_LEN` samples of `x`.
fn lobe_power(x: &[f32], f: f64) -> f64 {
    let p = power_spectrum(&x[x.len() - FFT_LEN..]);
    let c = (f / (SRATE / FFT_LEN as f64)).round() as isize;
    (c - 6..=c + 6)
        .filter(|&i| i >= 0 && (i as usize) < p.len())
        .map(|i| p[i as usize])
        .sum()
}

/// The amplitude of the component at `f` Hz, against a unit sine: the same lobe sum of a clean `f64` sine at `f`.
fn amplitude(x: &[f32], f: f64) -> f64 {
    let reference = sine(f, 1.0, SRATE, FFT_LEN);
    (lobe_power(x, f) / lobe_power(&reference, f)).sqrt()
}

fn db(ratio: f64) -> f64 {
    20.0 * math::log10(ratio.max(1e-30))
}

// ---------------------------------------------------------------------------------------------------------------
// Phase modulation: sidebands at carrier ± n × modulator with amplitudes |J_n(β)|
// ---------------------------------------------------------------------------------------------------------------

/// `J_n(x)` from its power series, in the test, with its own loop (independent of the DSP's).
fn bessel(n: i32, x: f64) -> f64 {
    let n = n.unsigned_abs() as i32;
    let mut sum = 0.0f64;
    let mut term = 1.0f64; // (x/2)^(2m+n) / (m! (m+n)!) built up from m = 0
    for i in 1..=n {
        term *= 0.5 * x / f64::from(i);
    }
    for m in 0..80 {
        sum += term;
        term *= -(0.25 * x * x) / (f64::from(m + 1) * f64::from(m + 1 + n));
    }
    sum
}

/// The key and modulator semitones of the closed-form rigs: a carrier at B4 (493.9 Hz) and a modulator 25 semitones
/// below (116.6 Hz): a ratio near 0.236, irrational enough that no sideband up to |n| = 8 lands on another.
const KEY: u8 = 71;
const MOD_SEMI: f32 = -25.0;

fn carrier_and_modulator_hz() -> (f64, f64) {
    (
        midi_hz(f64::from(KEY)),
        midi_hz(f64::from(KEY) + f64::from(MOD_SEMI)),
    )
}

/// The reference level of the unmodulated carrier through the rig (the pan law and the envelope included).
fn carrier_amplitude(set: &[(WtParam, f32)]) -> f64 {
    let (fc, _) = carrier_and_modulator_hz();
    amplitude(&held(&lone(set), KEY), fc)
}

/// The sidebands of a sine carrier phase-modulated by a sine, against `|J_n(β)|`, for `n` in `-4..=4`.
fn check_bessel(patch: &Patch, beta: f64, what: &str) {
    let (fc, fm) = carrier_and_modulator_hz();
    let x = held(patch, KEY);
    let reference = carrier_amplitude(&[]);
    let mut worst = 0.0f64;
    let mut table = String::new();
    for n in -4..=4 {
        let want = bessel(n, beta).abs();
        let got = amplitude(&x, fc + f64::from(n) * fm) / reference;
        let err = db(got) - db(want);
        worst = worst.max(err.abs());
        writeln!(
            table,
            "  n {n:>2}: J {want:.5} ({:>7.2} dB)  measured {got:.5} ({:>7.2} dB)  error {err:+.3} dB",
            db(want),
            db(got),
        )
        .unwrap();
    }
    eprintln!("{what} (β = {beta}): worst {worst:.3} dB\n{table}");
    assert!(
        worst < 0.5,
        "{what}: sidebands off the Bessel functions by {worst:.3} dB\n{table}"
    );
}

#[test]
fn pm_sidebands_are_the_bessel_functions() {
    for beta in [1.0f32, 2.5] {
        let p = pm_rig(MOD_SEMI, 100.0, &[(WtParam::PmIndex, beta)]);
        check_bessel(&p, f64::from(beta), "pm_index");
    }
}

#[test]
fn the_index_is_pm_index_times_pm_trim_times_the_route_depth() {
    // β = 5 × 50 % × 100 % and β = 5 × 100 % × 50 % are both 2.5; so is β = 2.5 with everything at 100 %.
    let a = pm_rig(
        MOD_SEMI,
        100.0,
        &[(WtParam::PmIndex, 5.0), (WtParam::Pm, 50.0)],
    );
    check_bessel(&a, 2.5, "pm trim 50 %");
    let b = pm_rig(MOD_SEMI, 50.0, &[(WtParam::PmIndex, 5.0)]);
    check_bessel(&b, 2.5, "route depth 50 %");
    // A negative depth is the same spectrum (J_n magnitudes) with the sidebands' signs swapped.
    let c = pm_rig(MOD_SEMI, -100.0, &[(WtParam::PmIndex, 2.5)]);
    check_bessel(&c, 2.5, "route depth -100 %");
}

#[test]
fn pm_at_index_zero_or_without_a_route_is_the_plain_carrier() {
    let plain = held(&lone(&[]), KEY);
    let zero = held(&pm_rig(MOD_SEMI, 100.0, &[(WtParam::PmIndex, 0.0)]), KEY);
    assert_eq!(bits(&plain), bits(&zero), "index 0");
    let off = held(&pm_rig(MOD_SEMI, 100.0, &[(WtParam::Pm, 0.0)]), KEY);
    assert_eq!(bits(&plain), bits(&off), "pm trim 0 %");
    // No route: the two values do nothing whatever they are.
    let mut p = lone(&[(WtParam::PmIndex, 10.0), (WtParam::Pm, 100.0)]);
    p.generators.insert(0, modulator("osc1", MOD_SEMI));
    assert_eq!(bits(&plain), bits(&held(&p, KEY)), "no route");
}

// ---------------------------------------------------------------------------------------------------------------
// Amplitude modulation and ring modulation
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn unipolar_am_is_the_carrier_and_two_sidebands_at_half_the_index() {
    let (fc, fm) = carrier_and_modulator_hz();
    // gain = am/100 + depth/100·sin: carrier am/100, sidebands depth/200 each, so sidebands/carrier = depth/(2·am).
    for (am, depth) in [
        (100.0f32, 80.0f32),
        (100.0, 30.0),
        (50.0, 40.0),
        (60.0, 60.0),
    ] {
        let x = held(&am_rig(MOD_SEMI, depth, &[(WtParam::Am, am)]), KEY);
        let reference = carrier_amplitude(&[]);
        let carrier = amplitude(&x, fc) / reference;
        let want_carrier = f64::from(am) / 100.0;
        assert!(
            (db(carrier) - db(want_carrier)).abs() < 0.1,
            "am {am} depth {depth}: carrier {carrier:.5}, want {want_carrier}"
        );
        let want_side = f64::from(depth) / 200.0;
        for sign in [-1.0, 1.0] {
            let side = amplitude(&x, fc + sign * fm) / reference;
            let err = db(side) - db(want_side);
            eprintln!(
                "am {am} depth {depth}: sideband {sign:+} {side:.5} (want {want_side:.5}) {err:+.4} dB"
            );
            assert!(
                err.abs() < 0.1,
                "am {am} depth {depth}: sideband off by {err:.3} dB"
            );
        }
        // Nothing else: the components at ±2 modulator frequencies and the modulator itself are below −80 dB.
        for f in [fc + 2.0 * fm, fc - 2.0 * fm, fm] {
            assert!(
                db(amplitude(&x, f) / reference) < -80.0,
                "am {am} depth {depth}: stray at {f:.1}"
            );
        }
    }
}

#[test]
fn am_at_zero_percent_is_ring_modulation_with_no_carrier() {
    let (fc, fm) = carrier_and_modulator_hz();
    let reference = carrier_amplitude(&[]);
    for depth in [100.0f32, 60.0, -100.0] {
        let x = held(&am_rig(MOD_SEMI, depth, &[(WtParam::Am, 0.0)]), KEY);
        // sin(c)·d·sin(m) = d/2·(cos(c − m) − cos(c + m)): two sidebands of |d|/2, nothing at the carrier.
        let want = f64::from(depth.abs()) / 200.0;
        for sign in [-1.0, 1.0] {
            let side = amplitude(&x, fc + sign * fm) / reference;
            assert!(
                (db(side) - db(want)).abs() < 0.1,
                "depth {depth}: sideband {side:.5}, want {want:.5}"
            );
        }
        let carrier = db(amplitude(&x, fc) / reference);
        let side = db(want);
        eprintln!(
            "ring mod depth {depth}: carrier {carrier:.1} dB re the plain carrier ({:.1} dB re a sideband)",
            carrier - side
        );
        assert!(
            carrier < -60.0,
            "depth {depth}: the carrier is only {carrier:.1} dB down"
        );
        assert!(
            carrier - side < -60.0,
            "depth {depth}: {carrier:.1} dB against a sideband"
        );
    }
}

#[test]
fn am_without_a_route_scales_the_output_like_a_second_level() {
    let plain = held(&lone(&[]), KEY);
    let half = held(&lone(&[(WtParam::Am, 50.0)]), KEY);
    let level = held(&lone(&[(WtParam::Level, 50.0)]), KEY);
    assert!(plain.iter().any(|x| x.abs() > 0.3));
    for ((p, h), l) in plain.iter().zip(&half).zip(&level) {
        assert_eq!(h.to_bits(), l.to_bits());
        assert!((p * 0.5 - h).abs() < 1e-6);
    }
    let silent = held(&lone(&[(WtParam::Am, 0.0)]), KEY);
    assert!(
        silent.iter().all(|&x| x == 0.0),
        "am 0 % without a route is silent"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Feedback
// ---------------------------------------------------------------------------------------------------------------

/// The distance of the first ten harmonics' amplitudes (against the first) from a saw's `1/n`, as an rms in dB.
fn distance_from_saw(x: &[f32], f0: f64) -> f64 {
    let a1 = amplitude(x, f0);
    let sum: f64 = (2..=10)
        .map(|n| {
            let have = db(amplitude(x, f0 * f64::from(n)) / a1).max(-100.0);
            let saw = -db(f64::from(n));
            (have - saw) * (have - saw)
        })
        .sum();
    (sum / 9.0).sqrt()
}

#[test]
fn feedback_moves_the_spectrum_monotonically_toward_a_saws_slope() {
    for key in [45u8, 57] {
        let f0 = midi_hz(f64::from(key));
        let mut last = f64::INFINITY;
        let mut row = String::new();
        for fb in (0..=10).map(|i| i as f32 * 10.0) {
            let x = held(&lone(&[(WtParam::Feedback, fb)]), key);
            let d = distance_from_saw(&x, f0);
            write!(row, " {fb:.0}%: {d:.2}").unwrap();
            assert!(
                d < last,
                "key {key}: feedback {fb} % is {d:.2} dB from a saw, after {last:.2}\n{row}"
            );
            last = d;
        }
        eprintln!("key {key} distance from a saw's 1/n (dB rms, harmonics 2-10):{row}");
        assert!(
            last < 3.0,
            "key {key}: 100 % feedback is {last:.2} dB from a saw"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Aliasing
// ---------------------------------------------------------------------------------------------------------------

/// Alias power over harmonic power of `patch` at `key`, against the fundamental `f0`.
fn alias_of(patch: &Patch, key: u8, f0: f64) -> f64 {
    alias_db(&held(patch, key), f0, SRATE, &Measure::REPORT_02)
}

const SWEEP_KEYS: [u8; 7] = [24, 36, 48, 60, 72, 84, 96];

/// One row of an aliasing table: a label, the swept parameter's value, the alias dB at each key.
struct Row {
    label: String,
    param: f32,
    values: Vec<f64>,
}

/// Measure `rows` of `keys` with `measure(param, key)`.
fn measure_rows(
    keys: &[u8],
    params: &[f32],
    label: impl Fn(f32) -> String,
    measure: impl Fn(f32, u8) -> f64,
) -> Vec<Row> {
    params
        .iter()
        .map(|&param| Row {
            label: label(param),
            param,
            values: keys.iter().map(|&k| measure(param, k)).collect(),
        })
        .collect()
}

/// Print the table (the one `docs/dsp/wt.md` records) and fail on any cell at or above its bound.
fn check_table(title: &str, keys: &[u8], rows: &[Row], bound: impl Fn(u8, f32) -> f64) {
    let mut table = format!("{title}\n  {:<24}", "");
    for k in keys {
        write!(table, "{:>8}", format!("key {k}")).unwrap();
    }
    table.push('\n');
    let mut failures = String::new();
    for row in rows {
        write!(table, "  {:<24}", row.label).unwrap();
        for (&k, &v) in keys.iter().zip(&row.values) {
            write!(table, "{v:>8.1}").unwrap();
            let limit = bound(k, row.param);
            if v >= limit {
                writeln!(
                    failures,
                    "  {} at key {k}: {v:.1} dB, bound {limit:.1}",
                    row.label
                )
                .unwrap();
            }
        }
        table.push('\n');
    }
    eprintln!("{table}");
    assert!(
        failures.is_empty(),
        "{title}: above the recorded bound\n{failures}{table}"
    );
}

// The bounds below are the worst measured point of each region plus a margin of 1-2 dB (`docs/dsp/wt.md` has the
// tables). Regions, because aliasing here is a property of the *setting*: a high index or a hard feedback is a wide
// spectrum, and a wide spectrum on a high note folds over at any sample rate (PM's Carson bandwidth 2(I + 1)·f_m).

/// Alias dB below which every wave table sits, at every key 24-105 and position 0/50/100 %.
const BOUND_TABLES_DB: f64 = -90.0;

#[test]
fn every_table_stays_inside_its_bound_across_the_keyboard() {
    let keys = [24u8, 36, 48, 60, 72, 84, 96, 105];
    let mut rows = Vec::new();
    for (name, table) in [
        ("basic", WtTable::Basic),
        ("fm", WtTable::Fm),
        ("fm_octave", WtTable::FmOctave),
        ("formant", WtTable::Formant),
        ("metal", WtTable::Metal),
    ] {
        for pos in [0.0f32, 50.0, 100.0] {
            let p = lone(&[
                (WtParam::Table, table.index() as f32),
                (WtParam::Position, pos),
            ]);
            rows.push(Row {
                label: format!("{name} pos {pos:.0} %"),
                param: pos,
                values: keys
                    .iter()
                    .map(|&k| alias_of(&p, k, midi_hz(f64::from(k))))
                    .collect(),
            });
        }
    }
    check_table("tables, alias dB re the harmonics", &keys, &rows, |_, _| {
        BOUND_TABLES_DB
    });
}

/// The PM sweep's bound by region: any index up to key 72 (C5, 523 Hz); an index up to 2.5 above that; and a ceiling
/// over the rest, the fold-over of a modulator 2:1 or 1:1 at index 5-10 on a note above 2 kHz.
fn pm_bound(key: u8, index: f32) -> f64 {
    if key <= 72 {
        -91.0
    } else if index <= 2.5 {
        -60.0
    } else {
        2.0
    }
}

#[test]
fn the_pm_index_sweep_stays_inside_its_recorded_bound() {
    // Carrier = the sine table; the modulator a sine at 2:1, 1:1 and 1:2 of the carrier, so the output has period
    // 1, 1 and 2 carrier cycles: the fundamental to pass is `f_c`, `f_c` and `f_c / 2`.
    for (semi, ratio, divisor) in [
        (12.0f32, "2:1", 1.0f64),
        (0.0, "1:1", 1.0),
        (-12.0, "1:2", 2.0),
    ] {
        // The measure reads 20 Hz - 20 kHz: leave out a fundamental below it.
        let keys: Vec<u8> = SWEEP_KEYS
            .iter()
            .copied()
            .filter(|&k| midi_hz(f64::from(k)) / divisor >= 25.0)
            .collect();
        let rows = measure_rows(
            &keys,
            &[0.0, 1.0, 2.5, 5.0, 10.0],
            |i| format!("index {i}"),
            |index, k| {
                alias_of(
                    &pm_rig(semi, 100.0, &[(WtParam::PmIndex, index)]),
                    k,
                    midi_hz(f64::from(k)) / divisor,
                )
            },
        );
        check_table(
            &format!("PM, modulator {ratio}, alias dB re the harmonics"),
            &keys,
            &rows,
            pm_bound,
        );
    }
}

/// The feedback sweep's bound by region: any feedback up to key 48 (131 Hz); up to 25 % anywhere; up to 50 % to key
/// 84; and above that the aliasing of a near-saw's edge (osc.md's polyBLEP saw: -40.6 dB at 440 Hz, -28.8 dB at
/// 3,520 Hz), here -22 dB at most.
fn feedback_bound(key: u8, feedback: f32) -> f64 {
    if key <= 48 || feedback <= 25.0 || (feedback <= 50.0 && key <= 84) {
        -90.0
    } else {
        -21.0
    }
}

#[test]
fn the_feedback_sweep_stays_inside_its_recorded_bound() {
    let rows = measure_rows(
        &SWEEP_KEYS,
        &[0.0, 25.0, 50.0, 75.0, 100.0],
        |fb| format!("feedback {fb} %"),
        |fb, k| alias_of(&lone(&[(WtParam::Feedback, fb)]), k, midi_hz(f64::from(k))),
    );
    check_table(
        "feedback on the sine table, alias dB re the harmonics",
        &SWEEP_KEYS,
        &rows,
        feedback_bound,
    );
}

/// A warp's recorded ceiling. `bend` is a smooth map and the mip level follows its slope; `asymmetric` and `squeeze`
/// have kinks, `quantize` and a `sync` ratio that is not a whole number have jumps, which a read-through-a-map
/// warp does not smooth; `sync` at ±100 % is a ratio of 8, so every jump lands on the table's own period.
fn warp_bound(warp: Warp, amount: f32) -> f64 {
    match warp {
        Warp::Off => -90.0,
        Warp::Bend => -23.0,
        Warp::Asymmetric => -14.0,
        Warp::Squeeze => -5.0,
        Warp::Sync if amount.abs() == 100.0 => -90.0,
        Warp::Sync => -28.0,
        Warp::Quantize => -8.0,
    }
}

#[test]
fn the_warp_sweep_stays_inside_its_recorded_bound() {
    for (name, warp) in [
        ("bend", Warp::Bend),
        ("asymmetric", Warp::Asymmetric),
        ("squeeze", Warp::Squeeze),
        ("sync", Warp::Sync),
        ("quantize", Warp::Quantize),
    ] {
        let rows = measure_rows(
            &SWEEP_KEYS,
            &[50.0, -100.0, 100.0],
            |a| format!("{name} {a:+.0} %"),
            |amount, k| {
                let p = lone(&[
                    (WtParam::Table, WtTable::Formant.index() as f32),
                    (WtParam::Position, 50.0),
                    (WtParam::Warp, warp.index() as f32),
                    (WtParam::WarpAmount, amount),
                ]);
                alias_of(&p, k, midi_hz(f64::from(k)))
            },
        );
        check_table(
            &format!("{name} on the formant table (position 50 %), alias dB re the harmonics"),
            &SWEEP_KEYS,
            &rows,
            |_, amount| warp_bound(warp, amount),
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The module contract
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_wt_as_a_source_feeds_another_wt_and_the_two_keep_separate_state() {
    // wt1 is a silent FM operator (level 0) one octave up, feeding wt2.pm. An `osc` sine at the same pitch is the
    // same operator up to the table's interpolation error.
    let mut p = gated();
    p.generators
        .push(wt("wt1", &[(WtParam::Semi, 12.0), (WtParam::Level, 0.0)]));
    p.generators.push(wt("wt2", &[(WtParam::PmIndex, 3.0)]));
    p.routes.push(route("wt1", "wt2.pm", 100.0));
    let mut q = gated();
    q.generators.push(modulator("osc1", 12.0));
    q.generators.push(wt("wt2", &[(WtParam::PmIndex, 3.0)]));
    q.routes.push(route("osc1", "wt2.pm", 100.0));
    let a = held(&p, 57);
    let b = held(&q, 57);
    assert!(a.iter().any(|x| x.abs() > 0.3), "it sounds");
    let worst = a
        .iter()
        .zip(&b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(
        worst < 2e-4,
        "a wt operator equals an osc sine operator: {worst}"
    );

    // And without a route the two modules are independent: the sum of their separate renders is the joint render.
    let set1 = [
        (WtParam::Semi, 7.0),
        (WtParam::Table, WtTable::Formant.index() as f32),
        (WtParam::Feedback, 40.0),
    ];
    let set2 = [
        (WtParam::Table, WtTable::Metal.index() as f32),
        (WtParam::Position, 60.0),
        (WtParam::Feedback, 70.0),
    ];
    let mut both = gated();
    both.generators.push(wt("wt1", &set1));
    both.generators.push(wt("wt2", &set2));
    let mut one = gated();
    one.generators.push(wt("wt1", &set1));
    let mut two = gated();
    two.generators.push(wt("wt2", &set2));
    let (j, x, y) = (held(&both, 52), held(&one, 52), held(&two, 52));
    for i in 0..j.len() {
        assert!((j[i] - (x[i] + y[i])).abs() < 1e-6, "sample {i}");
    }
}

#[test]
fn a_wt_routed_to_itself_is_refused() {
    let mut p = gated();
    p.generators.push(wt("wt1", &[]));
    p.routes.push(route("wt1", "wt1.pm", 50.0));
    let err = compile(&p, &PrepareCfg::new(SR)).expect_err("a loop");
    assert_eq!(err.code(), "audio_cycle");
}

#[test]
fn with_unison_off_detune_blend_and_spread_do_nothing() {
    // `unison` 1 is the single oscillator whatever the other unison knobs say (lane UNI's tests do the copies).
    let plain = lone(&[(WtParam::Feedback, 30.0)]);
    let knobs = lone(&[
        (WtParam::Feedback, 30.0),
        (WtParam::Unison, 1.0),
        (WtParam::Detune, 80.0),
        (WtParam::Blend, 40.0),
        (WtParam::Spread, 100.0),
    ]);
    let (l1, r1, _) = render(&plain, &[note(55, 0, 6_000)], 8_000, 512, SEED);
    let (l8, r8, _) = render(&knobs, &[note(55, 0, 6_000)], 8_000, 512, SEED);
    assert!(l1.iter().any(|x| x.abs() > 0.1));
    assert_eq!(bits(&l1), bits(&l8));
    assert_eq!(bits(&r1), bits(&r8));
}

/// A busy patch: `osc1` modulating a `wt1` that feeds back, warps and is AM'd by a second modulator, and a second `wt`
/// as a plain voice.
fn busy() -> Patch {
    let mut p = gated();
    p.generators.push(modulator("osc1", 7.0));
    p.generators.push(modulator("osc2", -12.0));
    p.generators.push(wt(
        "wt1",
        &[
            (WtParam::Table, WtTable::Fm.index() as f32),
            (WtParam::Position, 35.0),
            (WtParam::Warp, Warp::Asymmetric.index() as f32),
            (WtParam::WarpAmount, 40.0),
            (WtParam::PmIndex, 4.0),
            (WtParam::Feedback, 35.0),
            (WtParam::Am, 70.0),
        ],
    ));
    p.generators
        .push(wt("wt2", &[(WtParam::Semi, -12.0), (WtParam::Level, 50.0)]));
    p.routes.push(route("osc1", "wt1.pm", 80.0));
    p.routes.push(route("osc2", "wt1.am", 25.0));
    p
}

#[test]
fn the_host_block_size_is_not_an_input() {
    let p = busy();
    let notes = [
        note(48, 0, 9_000),
        note(55, 1_500, 9_000),
        note(64, 2_001, 7_000),
    ];
    let (reference, rr, _) = render(&p, &notes, 12_000, 512, SEED);
    assert!(reference.iter().any(|x| x.abs() > 0.1));
    for block in [1usize, 7, 32, 64, 100, 4_096] {
        let (l, r, _) = render(&p, &notes, 12_000, block, SEED);
        assert_eq!(bits(&l), bits(&reference), "left at block {block}");
        assert_eq!(bits(&r), bits(&rr), "right at block {block}");
    }
}

fn tail_notes() -> Vec<common::Note> {
    vec![
        note(36, 0, SR as u64),
        note(48, 0, SR as u64),
        note(60, 960, SR as u64),
        note(84, 1920, SR as u64 + 4800),
    ]
}

/// The busy patch with a long release, plus feedback at its maximum.
fn tail_patch() -> Patch {
    let mut p = busy();
    p.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    p.generators[2].values[WtParam::Feedback.index()] = 100.0;
    p
}

#[test]
fn a_40_s_release_tail_has_no_subnormals_and_every_voice_ends() {
    let mut e = engine(&tail_patch(), SEED);
    let (mut output, mut state, mut non_finite) = (0usize, 0usize, 0usize);
    let mut calls = 0usize;
    drive(
        &mut e,
        &events(&tail_notes()),
        40 * SR as usize,
        512,
        |e, l, r| {
            output += count_subnormals(l) + count_subnormals(r);
            non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                state = state.max(e.subnormals());
            }
            calls += 1;
        },
    );
    state = state.max(e.subnormals());
    assert_eq!(output, 0, "subnormal output samples");
    assert_eq!(state, 0, "subnormal values in the engine state");
    assert_eq!(non_finite, 0, "non-finite samples");
    assert!(
        e.active_voices().is_empty(),
        "every voice ends below -100 dB"
    );
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
    pub const MODES: [(&str, u64); 3] = [("IEEE", 0), ("FTZ", FTZ), ("FTZ+DAZ", FTZ | DAZ)];
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
    pub const MODES: [(&str, u64); 2] = [("IEEE", 0), ("FZ", FZ)];
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn the_render_is_the_same_under_every_ftz_mode() {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }
    let run = || {
        let mut e = engine(&tail_patch(), SEED);
        let (l, r) = drive(
            &mut e,
            &events(&tail_notes()),
            6 * SR as usize,
            512,
            |_, _, _| {},
        );
        (bits(&l), bits(&r))
    };
    let with_mode = |mode: u64| {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | mode);
        std::hint::black_box(run())
    };
    let reference = with_mode(0);
    for (name, mode) in fp::MODES {
        assert!(with_mode(mode) == reference, "the render under {name}");
    }
}
