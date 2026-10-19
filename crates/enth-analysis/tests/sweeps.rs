//! Report 03's descriptor suites on the toy voice: beat invariance (detuned patch against its detune-0 twin),
//! beating accuracy against theory, false positives on detune-0 patches, and tremolo against detune.
//!
//! The full sweeps are `#[ignore]` and print their tables; run them in release:
//! `cargo test --release -p enth-analysis --test sweeps -- --ignored --nocapture --test-threads 1`.
//! Their results are recorded in `docs/dsp/analysis.md`. The non-ignored tests are fast representative cases from
//! each suite, part of the gate.
//!
//! The sweeps run on the toy voice of `test_signals` (as the research did), so they must be rerun on the real
//! engine once it renders (`docs/phase-1a.md`, lane A).

#![allow(clippy::print_stdout, reason = "the sweeps print their tables")]

mod common;

use std::fmt::Write as _;

use common::{describe, describe_with, pct, quantile, raw_envelope, render};
use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming};
use enth_analysis::envelope::Envelope;
use enth_analysis::test_signals::{
    CutoffLfo, Rng, SR, ToyPatch, ToyPerf, Wave, random_toy, render_toy, render_toy_lfo,
    theory_beat, tremolo,
};

/// Per-descriptor agreement of a detuned patch's envelope with its detune-0 twin (report 03 §F2 tolerances).
#[derive(Default, Clone, Copy, Debug)]
struct Agreement {
    attack: Option<(f64, bool)>,
    sustain: Option<(f64, bool)>,
    decay: Option<(f64, bool)>,
    release: Option<(f64, bool)>,
}

impl Agreement {
    fn all(&self) -> bool {
        [self.attack, self.sustain, self.decay, self.release]
            .iter()
            .all(|v| v.is_none_or(|(_, ok)| ok))
    }
}

fn agree(twin: &Envelope, e: &Envelope) -> Agreement {
    let ea = (e.attack_ms - twin.attack_ms).abs();
    let mut a = Agreement {
        attack: Some((ea, ea <= (0.25 * twin.attack_ms).max(5.0))),
        ..Agreement::default()
    };
    if let (Some(s0), Some(s1)) = (twin.sustain_db, e.sustain_db)
        && s0 > -40.0
    {
        let es = (s1 - s0).abs();
        a.sustain = Some((es, es <= 1.5));
    }
    if let (Some(d0), Some(d1)) = (twin.decay_ms, e.decay_ms)
        && d0 > 20.0
    {
        let ed = libm::log(d1.max(1.0) / d0).abs();
        a.decay = Some((ed, ed <= 0.2));
    }
    if let (Some(r0), Some(r1)) = (twin.release_ms, e.release_ms)
        && r0 > 20.0
        && !twin.release_extrapolated
    {
        let er = libm::log(r1.max(1.0) / r0).abs();
        a.release = Some((er, er <= 0.2));
    }
    a
}

const DETUNES: [f64; 6] = [5.0, 8.0, 12.0, 20.0, 30.0, 45.0];

struct InvRow {
    errs: [Vec<f64>; 4],
    ok: [usize; 4],
    n: [usize; 4],
    all: usize,
}

impl InvRow {
    fn new() -> Self {
        Self {
            errs: Default::default(),
            ok: [0; 4],
            n: [0; 4],
            all: 0,
        }
    }

    fn add(&mut self, a: &Agreement) {
        for (k, v) in [a.attack, a.sustain, a.decay, a.release].iter().enumerate() {
            if let Some((err, ok)) = v {
                self.errs[k].push(*err);
                self.n[k] += 1;
                self.ok[k] += usize::from(*ok);
            }
        }
        self.all += usize::from(a.all());
    }

    fn print(&mut self, name: &str, total: usize) {
        let mut cells = Vec::new();
        for k in 0..4 {
            let (p50, p90) = (
                quantile(&mut self.errs[k], 0.5),
                quantile(&mut self.errs[k], 0.9),
            );
            cells.push(format!(
                "{p50:6.3} / {p90:6.3}  {:5.1}% (n={})",
                pct(self.ok[k], self.n[k]),
                self.n[k]
            ));
        }
        println!(
            "{name:4} | {} | {:5.1}%",
            cells.join(" | "),
            pct(self.all, total)
        );
    }
}

/// Beat invariance over random toy patches: Raw (no beat removal) against Fit (the descriptors). Report 03's seed.
#[test]
#[ignore = "sweep: 300 random patches; prints the table recorded in docs/dsp/analysis.md"]
fn invariance_sweep() {
    invariance(0x5eed_1234_abcd_0003);
}

/// The same on a different seed: the Huber refits were tuned on report 03's seed, this one checks they generalise.
#[test]
#[ignore = "sweep: 300 other random patches"]
fn invariance_sweep_holdout() {
    invariance(0x0dd_5eed_2026_1019);
}

fn invariance(seed: u64) {
    let mut d = Describer::new(SR);
    let mut rng = Rng(seed);
    let (mut raw_row, mut fit_row) = (InvRow::new(), InvRow::new());
    let (mut total, mut beat_found, mut flagged) = (0, 0, 0);
    for i in 0..300 {
        let (base, perf) = random_toy(&mut rng);
        let det = DETUNES[i % DETUNES.len()];
        let twin = describe(&mut d, &base, &perf);
        let Some(e0) = twin.envelope else { continue };
        total += 1;
        let p1 = ToyPatch {
            detune_ct: det,
            ..base
        };
        let x1 = render(&p1, &perf);
        let d1 = d.describe(&x1, NoteTiming::held(perf.hold_s)).unwrap();
        let e1 = d1.envelope.unwrap();
        beat_found += usize::from(d1.detune.is_some());
        flagged += usize::from(!e1.beat_corrected);
        raw_row.add(&agree(&e0, &raw_envelope(&x1, perf.hold_s)));
        fit_row.add(&agree(&e0, &e1));
    }
    println!(
        "\nbeat invariance: {total} random toy patches, detune 5/8/12/20/30/45 ct against the detune-0 twin"
    );
    println!("beat found on {beat_found}; flagged beat_corrected=false on {flagged}");
    println!(
        "row  | attack |err| ms p50 / p90, ok% | sustain |err| dB | decay |ln ratio| | release |ln ratio| | all four ok"
    );
    raw_row.print("Raw", total);
    fit_row.print("Fit", total);
}

/// Beating against theory on report 03's grid (378 cells).
#[test]
#[ignore = "sweep: 378 cells; prints the table recorded in docs/dsp/analysis.md"]
fn beat_accuracy_sweep() {
    let mut d = Describer::new(SR);
    let notes = [
        ("C2", 65.41),
        ("C3", 130.81),
        ("A3", 220.0),
        ("C5", 523.25),
        ("A4", 440.0),
        ("C6", 1046.5),
    ];
    let dets = [3.0, 5.0, 8.0, 12.0, 20.0, 30.0, 50.0];
    let (mut within, mut wrong, mut absent, mut tremolo_fp, mut lfo_fp) = (0, 0, 0, 0, 0);
    let mut wrong_cells = Vec::new();
    let mut lfo_cells = Vec::new();
    println!("\nbeating accuracy, % error vs f0·(2^(d/2400) − 2^(−d/2400)); '-' = absent");
    println!("cells: notes C2 C3 A3 C5 A4 C6 × detune 3 5 8 12 20 30 50 ct");
    for shape in ["held", "bell", "fenv"] {
        for wave in Wave::ALL {
            let mut line = format!("{shape:5} {:8}", wave.name());
            for (name, f0) in notes {
                for det in dets {
                    let p = grid_patch(shape, wave, det);
                    let perf = ToyPerf {
                        note_hz: f0,
                        dur_s: 3.0,
                        hold_s: 2.0,
                    };
                    let r = describe(&mut d, &p, &perf);
                    tremolo_fp += usize::from(r.modulation.amp_lfo.is_some());
                    // No cell has a filter LFO: a cyclic sweep here would be the beat read as a filter.
                    if let Some(s) = r.modulation.filter_sweep.filter(|s| s.hz.is_some()) {
                        lfo_fp += 1;
                        lfo_cells.push(format!(
                            "{shape} {} {name} {det} ct: {s:?}, beat {:?}",
                            wave.name(),
                            r.detune.map(|b| b.beating_hz)
                        ));
                    }
                    let th = theory_beat(f0, det);
                    match r.detune {
                        None => {
                            absent += 1;
                            line.push_str("    -");
                        }
                        Some(b) => {
                            let e = (b.beating_hz / th - 1.0) * 100.0;
                            if e.abs() <= 3.0 {
                                within += 1;
                            } else {
                                wrong += 1;
                                wrong_cells.push(format!(
                                    "{shape} {} {name} {det} ct: {:.3} Hz vs {th:.3}",
                                    wave.name(),
                                    b.beating_hz
                                ));
                            }
                            write!(line, " {e:+4.0}").unwrap();
                        }
                    }
                }
            }
            println!("{line}");
        }
    }
    println!(
        "within 3 %: {within}, wrong: {wrong}, absent: {absent}; tremolo reported on a detuned cell: {tremolo_fp}; \
         filter LFO reported: {lfo_fp}"
    );
    for c in wrong_cells {
        println!("  wrong: {c}");
    }
    for c in lfo_cells {
        println!("  filter LFO: {c}");
    }
}

fn grid_patch(shape: &str, wave: Wave, det: f64) -> ToyPatch {
    let base = ToyPatch {
        wave,
        detune_ct: det,
        cutoff_hz: 8000.0,
        ..ToyPatch::default()
    };
    match shape {
        "bell" => ToyPatch {
            sustain_db: -60.0,
            decay_ms: 2000.0,
            attack_ms: 10.0,
            ..base
        },
        "fenv" => ToyPatch {
            env_amount_oct: 3.0,
            cutoff_hz: 600.0,
            sustain_db: -3.0,
            ..base
        },
        _ => ToyPatch {
            sustain_db: 0.0,
            ..base
        },
    }
}

/// Detune-0 patches must report neither beating nor tremolo.
#[test]
#[ignore = "sweep: 400 random detune-0 patches"]
fn false_positive_sweep() {
    let mut d = Describer::new(SR);
    let mut rng = Rng(0x000f_a15e_0003);
    let (mut total, mut beats, mut trem, mut lfo, mut env) = (0, 0, 0, 0, 0);
    for _ in 0..400 {
        let (p, perf) = random_toy(&mut rng);
        let r = describe(&mut d, &p, &perf);
        if r.silent {
            continue;
        }
        total += 1;
        if let Some(b) = r.detune {
            beats += 1;
            println!(
                "false beat: {:.2} Hz, {:.1} dB, prominence {:.1} | {p:?} {perf:?}",
                b.beating_hz, b.depth_db, b.prominence
            );
        }
        if let Some(t) = r.modulation.amp_lfo {
            trem += 1;
            println!(
                "false tremolo: {:.2} Hz, {:.1} dB | {p:?} {perf:?}",
                t.hz, t.depth_db
            );
        }
        // The toy has no LFO: a cyclic sweep is false. A one-shot sweep is real when the filter envelope moves the
        // brightness by half an octave (counted, not judged).
        if let Some(s) = r.modulation.filter_sweep {
            if s.hz.is_some() {
                lfo += 1;
                println!("false filter LFO: {s:?} | {p:?} {perf:?}");
            } else {
                env += 1;
            }
        }
    }
    println!(
        "\ndetune-0 random patches: {total}; beating reported: {beats}; tremolo reported: {trem}; filter LFO \
         reported: {lfo}; one-shot sweeps: {env}"
    );
}

fn tremolo_case(d: &mut Describer, note: f64, det: f64, rate: f64, depth: f64) -> Descriptors {
    let p = ToyPatch {
        detune_ct: det,
        sustain_db: 0.0,
        cutoff_hz: 6000.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: note,
        dur_s: 3.0,
        hold_s: 2.0,
    };
    let mut x = render_toy(SR, &p, &perf, common::PHASES, 0.0);
    if rate > 0.0 {
        tremolo(&mut x, SR, rate, depth);
    }
    d.describe(&x, NoteTiming::held(2.0)).unwrap()
}

/// Tremolo (an amplitude LFO after the voice) against detune beating.
#[test]
#[ignore = "sweep: report 03's 5 tremolo cases plus a 72-case grid"]
fn tremolo_sweep() {
    let mut d = Describer::new(SR);
    println!("\nreport 03 cases:");
    for (note, det, rate, depth) in [
        (220.0, 0.0, 5.0, 0.5),
        (220.0, 0.0, 3.0, 0.8),
        (440.0, 0.0, 6.0, 0.3),
        (220.0, 20.0, 0.0, 0.0),
        (130.81, 0.0, 4.0, 0.6),
    ] {
        let r = tremolo_case(&mut d, note, det, rate, depth);
        println!(
            "note {note} detune {det} ct, tremolo {rate} Hz depth {depth}: beating {:?}, tremolo {:?}",
            r.detune.map(|b| (b.beating_hz * 100.0).round() / 100.0),
            r.modulation.amp_lfo.map(|t| (
                (t.hz * 100.0).round() / 100.0,
                (t.depth_db * 10.0).round() / 10.0
            ))
        );
    }
    let (mut n, mut beat_fp, mut found, mut accurate, mut sweep_fp) = (0, 0, 0, 0, 0);
    for note in [110.0, 220.0, 440.0, 880.0] {
        for rate in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0] {
            for depth in [0.3, 0.5, 0.8] {
                let r = tremolo_case(&mut d, note, 0.0, rate, depth);
                n += 1;
                beat_fp += usize::from(r.detune.is_some());
                // An amplitude LFO moves no brightness.
                sweep_fp += usize::from(r.modulation.filter_sweep.is_some());
                if let Some(t) = r.modulation.amp_lfo {
                    found += 1;
                    accurate += usize::from((t.hz / rate - 1.0).abs() <= 0.03);
                } else {
                    println!("  missed: note {note} rate {rate} depth {depth}");
                }
            }
        }
    }
    println!(
        "grid (notes 110/220/440/880 × rates 2-8 Hz × depths 0.3/0.5/0.8): {n} cases; reported as beating {beat_fp}; tremolo found {found}, within 3 % {accurate}; filter sweep reported {sweep_fp}"
    );
}

/// Filter LFOs (a sine on the toy's cutoff) of every rate, depth and brightness: found, and at the right rate.
#[test]
#[ignore = "sweep: 240 filter-LFO cases"]
fn filter_lfo_sweep() {
    let mut d = Describer::new(SR);
    let (mut n, mut found, mut accurate) = (0, 0, 0);
    let mut depths: Vec<(f64, f64)> = Vec::new();
    for note in [110.0, 220.0, 440.0, 880.0] {
        for cutoff in [600.0, 2000.0] {
            for rate in [0.75, 1.5, 3.0, 6.0, 12.0] {
                for oct in [0.25, 0.5, 1.0, 2.0, 3.0, 4.0] {
                    let p = ToyPatch {
                        cutoff_hz: cutoff,
                        resonance_q: 1.2,
                        sustain_db: 0.0,
                        ..ToyPatch::default()
                    };
                    let perf = ToyPerf {
                        note_hz: note,
                        dur_s: 4.0,
                        hold_s: 3.5,
                    };
                    let lfo = CutoffLfo {
                        rate_hz: rate,
                        depth_oct: oct,
                    };
                    let x = render_toy_lfo(SR, &p, &perf, common::PHASES, 0.0, Some(lfo));
                    let r = d.describe(&x, NoteTiming::held(perf.hold_s)).unwrap();
                    n += 1;
                    match r
                        .modulation
                        .filter_sweep
                        .and_then(|s| s.hz.map(|h| (h, s.depth_oct)))
                    {
                        Some((hz, depth)) => {
                            found += 1;
                            if (hz / rate - 1.0).abs() <= 0.03 {
                                accurate += 1;
                            } else {
                                println!(
                                    "  wrong: note {note} cutoff {cutoff} rate {rate} +-{oct} oct: {hz:.3} Hz"
                                );
                            }
                            depths.push((oct, depth));
                        }
                        None => println!(
                            "  missed: note {note} cutoff {cutoff} rate {rate} +-{oct} oct: {:?}",
                            r.modulation.filter_sweep
                        ),
                    }
                }
            }
        }
    }
    println!(
        "filter LFO grid (notes 110/220/440/880 × cutoff 600/2000 Hz × rates 0.75-12 Hz × +-0.25-4 oct): {n} \
         cases; found {found}, within 3 % {accurate}"
    );
    for oct in [0.25, 0.5, 1.0, 2.0, 3.0, 4.0] {
        let mut v: Vec<f64> = depths.iter().filter(|d| d.0 == oct).map(|d| d.1).collect();
        if !v.is_empty() {
            let (p10, p50, p90) = (
                quantile(&mut v, 0.1),
                quantile(&mut v, 0.5),
                quantile(&mut v, 0.9),
            );
            println!(
                "  cutoff +-{oct} oct: centroid swing p10 {p10:.2}, p50 {p50:.2}, p90 {p90:.2} oct"
            );
        }
    }
}

/// Calibration on clean and on beating sounds (report 03's audit table): the 25 ct column must match the 0 ct one.
#[test]
#[ignore = "prints the calibration table"]
fn calibration_table() {
    let mut d = Describer::new(SR);
    let base = ToyPatch {
        cutoff_hz: 6000.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 220.0,
        dur_s: 4.0,
        hold_s: 2.5,
    };
    let show =
        |d: &mut Describer, label: String, p: ToyPatch, pick: &dyn Fn(&Envelope) -> Option<f64>| {
            let r0 = describe(d, &p, &perf);
            let p25 = ToyPatch {
                detune_ct: 25.0,
                ..p
            };
            let x25 = render(&p25, &perf);
            let r25 = d.describe(&x25, NoteTiming::held(perf.hold_s)).unwrap();
            let raw25 = raw_envelope(&x25, perf.hold_s);
            let f = |v: Option<f64>| v.map_or("-".to_string(), |x| format!("{x:.1}"));
            println!(
                "{label:32} | {:>7} | raw {:>7} / fit {:>7}  (beat {:?})",
                f(pick(&r0.envelope.unwrap())),
                f(pick(&raw25)),
                f(pick(&r25.envelope.unwrap())),
                r25.detune.map(|b| (b.beating_hz * 100.0).round() / 100.0)
            );
        };
    println!("\nprogrammed (saw A3, cutoff 6 kHz)  | 0 ct    | 25 ct (3.2 Hz beat)");
    for a in [2.0, 10.0, 100.0, 500.0, 1000.0] {
        show(
            &mut d,
            format!("amp.attack {a} ms -> attack"),
            ToyPatch {
                attack_ms: a,
                sustain_db: 0.0,
                ..base
            },
            &|e| Some(e.attack_ms),
        );
    }
    for s in [0.0, -3.0, -6.0, -12.0, -20.0, -45.0] {
        show(
            &mut d,
            format!("amp.sustain {s} dB -> sustain"),
            ToyPatch {
                sustain_db: s,
                ..base
            },
            &|e| e.sustain_db,
        );
    }
    for dc in [100.0, 500.0, 2000.0] {
        for s in [-6.0, -20.0, -60.0] {
            show(
                &mut d,
                format!("amp.decay {dc} ms (S {s}) -> decay"),
                ToyPatch {
                    decay_ms: dc,
                    sustain_db: s,
                    ..base
                },
                &|e| e.decay_ms,
            );
        }
    }
    for r in [100.0, 500.0, 1000.0] {
        show(
            &mut d,
            format!("amp.release {r} ms -> release"),
            ToyPatch {
                release_ms: r,
                sustain_db: -3.0,
                ..base
            },
            &|e| e.release_ms,
        );
    }
}

// ---- fast representative cases (part of the gate) ----

#[test]
fn detuned_twins_read_like_their_detune_0_twin() {
    let mut d = Describer::new(SR);
    let lead = ToyPerf {
        note_hz: 440.0,
        dur_s: 2.5,
        hold_s: 2.0,
    };
    let bass = ToyPerf {
        note_hz: 220.0,
        dur_s: 2.5,
        hold_s: 2.0,
    };
    let cases = [
        // g5-like lead at 26.84 ct, a 12 ct square with a slow attack, a 30 ct saw with a long decay
        (
            ToyPatch {
                detune_ct: 26.84,
                cutoff_hz: 14_000.0,
                attack_ms: 5.0,
                decay_ms: 40.0,
                sustain_db: -6.0,
                release_ms: 150.0,
                ..ToyPatch::default()
            },
            lead,
        ),
        (
            ToyPatch {
                wave: Wave::Square,
                detune_ct: 12.0,
                attack_ms: 120.0,
                decay_ms: 400.0,
                sustain_db: -10.0,
                ..ToyPatch::default()
            },
            lead,
        ),
        (
            ToyPatch {
                detune_ct: 30.0,
                cutoff_hz: 4000.0,
                attack_ms: 2.0,
                decay_ms: 800.0,
                sustain_db: -20.0,
                release_ms: 300.0,
                ..ToyPatch::default()
            },
            bass,
        ),
    ];
    for (p, perf) in cases {
        let twin = describe(
            &mut d,
            &ToyPatch {
                detune_ct: 0.0,
                ..p
            },
            &perf,
        )
        .envelope
        .unwrap();
        let x = render(&p, &perf);
        let r = d.describe(&x, NoteTiming::held(perf.hold_s)).unwrap();
        let e = r.envelope.unwrap();
        let a = agree(&twin, &e);
        assert!(r.detune.is_some() && e.beat_corrected, "{p:?}: {r:?}");
        assert!(a.all(), "{p:?}\n twin {twin:?}\n fit  {e:?}\n {a:?}");
    }
}

#[test]
fn beating_cells_read_theory() {
    let mut d = Describer::new(SR);
    for (shape, wave, f0, det) in [
        ("held", Wave::Saw, 130.81, 20.0),
        ("bell", Wave::Square, 440.0, 12.0),
        ("fenv", Wave::Triangle, 523.25, 30.0),
        ("held", Wave::Saw, 130.81, 5.0),
    ] {
        let r = describe(
            &mut d,
            &grid_patch(shape, wave, det),
            &ToyPerf {
                note_hz: f0,
                dur_s: 2.5,
                hold_s: 2.0,
            },
        );
        let th = theory_beat(f0, det);
        let b = r
            .detune
            .unwrap_or_else(|| panic!("{shape} {wave:?} {f0} {det}: no beat"));
        assert!(
            (b.beating_hz / th - 1.0).abs() <= 0.03,
            "{shape} {wave:?} {f0} {det}: {} vs {th}",
            b.beating_hz
        );
        assert!(r.modulation.amp_lfo.is_none());
        // 5 ct at C3 beats at 0.38 Hz: less than one period in the held part, so it is flagged, not corrected
        assert_eq!(
            r.envelope.unwrap().beat_corrected,
            det > 5.0,
            "{shape} {f0} {det}"
        );
    }
}

#[test]
fn detune_0_patches_do_not_beat() {
    let mut d = Describer::new(SR);
    let mut rng = Rng(0x000f_a15e_0003);
    for _ in 0..3 {
        let (p, perf) = random_toy(&mut rng);
        let r = describe(
            &mut d,
            &p,
            &ToyPerf {
                dur_s: 2.0,
                hold_s: 1.5,
                ..perf
            },
        );
        assert!(
            r.detune.is_none() && r.modulation.amp_lfo.is_none(),
            "{p:?}: {r:?}"
        );
    }
}

#[test]
fn tremolo_is_told_from_detune() {
    let mut d = Describer::new(SR);
    // the last case is a fast rate on a low note: harmonic 2 would wobble at 16 Hz, outside the demodulator band
    for (note, rate, depth) in [(220.0, 3.0, 0.8), (130.81, 4.0, 0.6), (110.0, 8.0, 0.8)] {
        let r = tremolo_case(&mut d, note, 0.0, rate, depth);
        assert!(r.detune.is_none(), "{note} {rate}: {r:?}");
        let t = r
            .modulation
            .amp_lfo
            .unwrap_or_else(|| panic!("{note} {rate}: no tremolo"));
        assert!((t.hz / rate - 1.0).abs() <= 0.03, "{t:?}");
    }
    let r = tremolo_case(&mut d, 220.0, 20.0, 0.0, 0.0);
    assert!(r.modulation.amp_lfo.is_none());
    assert!((r.detune.unwrap().beating_hz / theory_beat(220.0, 20.0) - 1.0).abs() < 0.03);
}

#[test]
fn clipping_bends_brightness_and_master_gain_undoes_it() {
    // report 03 §F4: the g5 run 3 intermediate step (saw, 24.6 ct, cutoff 4.5 kHz, Q 4, A4) peaks above full scale
    let mut d = Describer::new(SR);
    let p = ToyPatch {
        detune_ct: 24.6,
        cutoff_hz: 4500.0,
        resonance_q: 4.0,
        attack_ms: 5.0,
        decay_ms: 100.0,
        sustain_db: 0.0,
        release_ms: 150.0,
        ..ToyPatch::default()
    };
    let perf = ToyPerf {
        note_hz: 440.0,
        dur_s: 2.5,
        hold_s: 2.0,
    };
    let loud = describe_with(&mut d, &p, &perf, common::PHASES, 0.0);
    let quiet = describe_with(&mut d, &p, &perf, common::PHASES, -12.0);
    assert!(
        loud.loudness.clipping && loud.loudness.peak_dbfs > 5.0,
        "{:?}",
        loud.loudness
    );
    assert!(
        loud.loudness.clipped_pct > 0.5 && loud.loudness.true_peak_dbtp >= loud.loudness.peak_dbfs
    );
    assert!(
        !quiet.loudness.clipping
            && (loud.loudness.peak_dbfs - quiet.loudness.peak_dbfs - 12.0).abs() < 1e-3
    );
    // on the float render a gain is just a gain: every other descriptor is unchanged by the master level
    let (cl, cq) = (
        loud.spectrum.unwrap().centroid_hz,
        quiet.spectrum.unwrap().centroid_hz,
    );
    assert!((cl / cq - 1.0).abs() < 1e-3, "{cl} vs {cq}");
    // ...whereas clamping the same render to full scale, as a 16-bit file would, moves brightness up
    let mut x = render_toy(SR, &p, &perf, common::PHASES, 0.0);
    for s in &mut x {
        *s = s.clamp(-1.0, 1.0);
    }
    let clamped = d.describe(&x, NoteTiming::held(2.0)).unwrap();
    assert!(
        clamped.spectrum.unwrap().centroid_hz > 1.1 * cq,
        "{:?} vs {cq}",
        clamped.spectrum
    );
}

/// The invariance sweep's failures, one line each, for tuning.
#[test]
#[ignore = "prints the invariance sweep's failing patches"]
fn invariance_failures() {
    let mut d = Describer::new(SR);
    let mut rng = Rng(0x5eed_1234_abcd_0003);
    for i in 0..300 {
        let (base, perf) = random_toy(&mut rng);
        let det = DETUNES[i % DETUNES.len()];
        let twin = describe(&mut d, &base, &perf);
        let Some(e0) = twin.envelope else { continue };
        let p1 = ToyPatch {
            detune_ct: det,
            ..base
        };
        let r1 = describe(&mut d, &p1, &perf);
        let e1 = r1.envelope.unwrap();
        let a = agree(&e0, &e1);
        if !a.all() {
            let bad = |v: Option<(f64, bool)>, c: &'static str| {
                if v.is_some_and(|(_, ok)| !ok) { c } else { "" }
            };
            println!(
                "#{i:3} {}{}{}{} {:8} f0 {:6.1} det {det:4} beat {:5.2} per {:4.1} | A{:5.0} D{:5.0} S{:6.1} R{:4.0} cut {:5.0} fenv {:.1} | twin A{:4.0} D{:?} S{:?} | fit A{:4.0} D{:?} S{:?} rip {:.1}",
                bad(a.attack, "A"),
                bad(a.decay, "D"),
                bad(a.sustain, "S"),
                bad(a.release, "R"),
                base.wave.name(),
                perf.note_hz,
                r1.detune.map_or(0.0, |b| b.beating_hz),
                e1.beat_periods.unwrap_or(0.0),
                base.attack_ms,
                base.decay_ms,
                base.sustain_db,
                base.release_ms,
                base.cutoff_hz,
                base.env_amount_oct,
                e0.attack_ms,
                e0.decay_ms,
                e0.sustain_db.map(|s| (s * 10.0).round() / 10.0),
                e1.attack_ms,
                e1.decay_ms,
                e1.sustain_db.map(|s| (s * 10.0).round() / 10.0),
                e1.ripple_db
            );
        }
    }
}

/// Note-off inference for WAVs without their performance (report 03 §F5): 200 random patches with holds of 0.8 to
/// 2.6 s, a third of them detuned by 20 ct.
#[test]
#[ignore = "sweep: 200 random patches"]
fn note_off_inference_sweep() {
    let mut rng = Rng(0x40_1d);
    let mut errs = Vec::new();
    let (mut n_sus, mut none_sus, mut n_dec, mut none_dec) = (0, 0, 0, 0);
    for i in 0..200 {
        let (mut p, mut perf) = random_toy(&mut rng);
        perf.hold_s = 0.8 + 1.8 * rng.next_f64();
        perf.dur_s = perf.hold_s + 1.2;
        if i % 3 == 0 {
            p.detune_ct = 20.0;
        }
        let x = render(&p, &perf);
        let inferred = enth_analysis::envelope::infer_note_off(&x, SR);
        let (e, _) = enth_analysis::envelope::rms_envelope(&x, SR);
        let peak = e.iter().copied().fold(0.0, f64::max);
        let at_off = e[((perf.hold_s * 1000.0) as usize).min(e.len() - 1)];
        let sustained =
            at_off > 0.0316 * peak && p.release_ms < 0.6 * 1000.0 * (perf.dur_s - perf.hold_s);
        if sustained {
            n_sus += 1;
            match inferred {
                Some(h) => errs.push((h - perf.hold_s) * 1000.0),
                None => none_sus += 1,
            }
        } else {
            n_dec += 1;
            none_dec += usize::from(inferred.is_none());
        }
    }
    let within = errs.iter().filter(|e| e.abs() <= 30.0).count();
    let found = errs.len();
    println!(
        "\nnote-off inference: {n_sus} patches still sounding at note-off: found {found} (within 30 ms: {within}), none {none_sus}; error median {:.0} ms, p10 {:.0}, p90 {:.0}",
        quantile(&mut errs, 0.5),
        quantile(&mut errs, 0.1),
        quantile(&mut errs, 0.9)
    );
    println!("{n_dec} patches already decayed at note-off: no knee reported on {none_dec}");
}
