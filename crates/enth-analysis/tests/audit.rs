//! The descriptor audit of `docs/spikes/agent-probe.md` as regression tests: sweep one toy-voice parameter at a
//! time and check the reading against theory, then re-judge the Phase 0 trial's final patches (report 03 §F1).
//!
//! The audit's numbers were read on the spike's raw 8 ms RMS envelope; the theory columns still hold for the
//! beat-corrected one on clean sounds, and the detuned rows are where the new descriptors must differ from the
//! spike's: they read like the detune-0 twin.

mod common;

use common::{describe, goal_list, nuisance_phases, trial_patches};
use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming};
use enth_analysis::expect::{CheckReport, check, parse_list};
use enth_analysis::test_signals::{SR, ToyPatch, ToyPerf, Wave, render_toy, theory_beat};
use enth_analysis::units::cents;

fn long() -> ToyPerf {
    ToyPerf {
        note_hz: 220.0,
        dur_s: 2.6,
        hold_s: 2.0,
    }
}

fn short() -> ToyPerf {
    ToyPerf {
        note_hz: 220.0,
        dur_s: 1.2,
        hold_s: 1.0,
    }
}

fn env(r: &Descriptors) -> enth_analysis::envelope::Envelope {
    r.envelope.expect("not silent")
}

#[test]
fn attack_tracks_the_ramp() {
    // agent-probe: 10 / 100 / 500 ms read 11 / 88 / 441 ms: 0.88 of the ramp (1 % to -1 dB of a linear ramp);
    // the toy's filter adds a little at the start, the 8 ms window rounds short ramps up
    let mut d = Describer::new(SR);
    for (a, lo, hi) in [
        (10.0, 8.0, 13.0),
        (100.0, 80.0, 95.0),
        (500.0, 400.0, 460.0),
    ] {
        let r = describe(
            &mut d,
            &ToyPatch {
                attack_ms: a,
                sustain_db: 0.0,
                ..ToyPatch::default()
            },
            &long(),
        );
        let got = env(&r).attack_ms;
        assert!((lo..=hi).contains(&got), "amp.attack {a} ms read {got} ms");
    }
}

#[test]
fn sustain_reads_the_programmed_level() {
    // agent-probe (amp.decay 100 ms): 0 / -6 / -20 / -45 dB read -0.4 / -5.7 / -19.3 / -44.2. Report 03's calibration
    // (amp.decay 300 ms): -0.1 / -5.7 / -19.5 / -44.5. The 8 ms RMS window and the 5 ms robust peak blunt the peak of
    // a fast decay, so the sustain reads up to about 1.2 dB high after a decay spike with a time constant under 30 ms
    // (amp.decay 100 ms in the toy); with 300 ms it is within 0.6 dB.
    let mut d = Describer::new(SR);
    for (s, decay, tol) in [
        (0.0, 300.0, 0.6),
        (-6.0, 300.0, 0.6),
        (-20.0, 300.0, 0.6),
        (-45.0, 300.0, 0.6),
        (-20.0, 100.0, 1.3),
    ] {
        let p = ToyPatch {
            sustain_db: s,
            decay_ms: decay,
            ..ToyPatch::default()
        };
        let got = env(&describe(&mut d, &p, &long())).sustain_db.unwrap();
        assert!(
            (got - s).abs() < tol,
            "amp.sustain {s} dB (decay {decay} ms) read {got} dB"
        );
    }
    // sustain off reads the floor
    let p = ToyPatch {
        sustain_db: -60.0,
        decay_ms: 100.0,
        ..ToyPatch::default()
    };
    assert!(env(&describe(&mut d, &p, &long())).sustain_db.unwrap() <= -79.0);
}

#[test]
fn release_is_the_minus_40_db_time() {
    // the toy's release has tau = R/4, so -40 dB comes after 4.6 tau = 1.15 R (agent-probe: 116 / 571 / 2280 ms)
    let mut d = Describer::new(SR);
    for r in [100.0, 500.0, 1000.0] {
        let p = ToyPatch {
            release_ms: r,
            sustain_db: -3.0,
            ..ToyPatch::default()
        };
        let got = env(&describe(
            &mut d,
            &p,
            &ToyPerf {
                dur_s: 3.5,
                ..long()
            },
        ))
        .release_ms
        .unwrap();
        assert!(
            (got / (1.15 * r) - 1.0).abs() < 0.05,
            "amp.release {r} ms read {got} ms"
        );
    }
}

#[test]
fn decay_is_a_threshold_time_in_db() {
    // sustain off: the level must fall to -40 dB + 8 dB (the 20 % rule) = -32 dB re peak: tau·ln(10^(32/20)) = 3.7 tau,
    // tau = D/4, so about 0.92 D (report 03's calibration: 2000 ms read 1867)
    let mut d = Describer::new(SR);
    let p = ToyPatch {
        decay_ms: 2000.0,
        sustain_db: -60.0,
        cutoff_hz: 6000.0,
        ..ToyPatch::default()
    };
    let got = env(&describe(
        &mut d,
        &p,
        &ToyPerf {
            dur_s: 3.0,
            hold_s: 2.5,
            ..long()
        },
    ))
    .decay_ms
    .unwrap();
    assert!((1700.0..2000.0).contains(&got), "{got}");
    // with a sustain within 1 dB of the peak there is no decay stage
    let p = ToyPatch {
        sustain_db: -0.5,
        ..ToyPatch::default()
    };
    assert_eq!(env(&describe(&mut d, &p, &short())).decay_ms, Some(0.0));
}

#[test]
fn brightness_follows_cutoff_and_waveform() {
    let mut d = Describer::new(SR);
    let mut prev = 0.0;
    for c in [200.0, 800.0, 3200.0, 12_800.0] {
        let r = describe(
            &mut d,
            &ToyPatch {
                cutoff_hz: c,
                ..ToyPatch::default()
            },
            &short(),
        );
        let got = r.spectrum.unwrap().centroid_hz;
        assert!(got > prev, "cutoff {c}: centroid {got} not above {prev}");
        prev = got;
    }
    // agent-probe at 4 kHz: saw 1672 > square 1466 > triangle 501 Hz
    let c: Vec<f64> = Wave::ALL
        .iter()
        .map(|&w| {
            describe(
                &mut d,
                &ToyPatch {
                    wave: w,
                    cutoff_hz: 4000.0,
                    ..ToyPatch::default()
                },
                &short(),
            )
            .spectrum
            .unwrap()
            .centroid_hz
        })
        .collect();
    assert!(c[0] > c[1] && c[1] > c[2], "{c:?}");
    assert!(
        (c[0] - 1672.0).abs() < 100.0 && (c[2] - 501.0).abs() < 60.0,
        "{c:?}"
    );
}

#[test]
fn pitch_is_right_from_c2_to_c6() {
    let mut d = Describer::new(SR);
    for (i, f) in [65.406, 130.813, 261.626, 523.251, 1046.5]
        .into_iter()
        .enumerate()
    {
        let wave = Wave::ALL[i % 3];
        let r = describe(
            &mut d,
            &ToyPatch {
                wave,
                cutoff_hz: 8000.0,
                ..ToyPatch::default()
            },
            &ToyPerf {
                note_hz: f,
                dur_s: 1.0,
                hold_s: 0.8,
            },
        );
        let p = r.pitch.unwrap();
        assert!(cents(p.f0_hz, f).abs() < 5.0, "{wave:?} {f}: {p:?}");
    }
}

#[test]
fn detune_reads_its_beat_rate() {
    // agent-probe: 20 / 30 / 50 ct at C3 and 10 / 20 / 30 / 50 ct at A4, within about 2 % of theory
    let mut d = Describer::new(SR);
    for (f0, det) in [(130.81, 20.0), (130.81, 50.0), (440.0, 10.0), (440.0, 30.0)] {
        let r = describe(
            &mut d,
            &ToyPatch {
                detune_ct: det,
                sustain_db: 0.0,
                ..ToyPatch::default()
            },
            &ToyPerf {
                note_hz: f0,
                ..long()
            },
        );
        let th = theory_beat(f0, det);
        let got = r
            .detune
            .unwrap_or_else(|| panic!("{f0} {det}: no beat"))
            .beating_hz;
        assert!(
            (got / th - 1.0).abs() < 0.03,
            "{f0} Hz {det} ct: {got} vs {th}"
        );
    }
}

#[test]
fn detuned_attack_is_not_a_quarter_beat() {
    // The probe's bug: with sustain near 0 dB the first beat peak set the "peak", so attack read ~1/(4 beat) whatever
    // amp.attack was (g5: 5 ms read 35 ms at 31 ct). Report 03's calibration at 25 ct: raw 97 ms, fit 4 ms.
    let mut d = Describer::new(SR);
    for a in [2.0, 10.0] {
        let p = ToyPatch {
            attack_ms: a,
            sustain_db: 0.0,
            cutoff_hz: 6000.0,
            ..ToyPatch::default()
        };
        let twin = env(&describe(&mut d, &p, &long())).attack_ms;
        let x = render_toy(
            SR,
            &ToyPatch {
                detune_ct: 25.0,
                ..p
            },
            &long(),
            common::PHASES,
            0.0,
        );
        let fit = env(&d.describe(&x, NoteTiming::held(2.0)).unwrap()).attack_ms;
        let raw = common::raw_envelope(&x, 2.0).attack_ms;
        assert!(
            (fit - twin).abs() <= 2.0,
            "attack {a}: fit {fit} twin {twin}"
        );
        assert!(
            raw > 50.0,
            "the raw envelope should show the probe's bug: {raw}"
        );
    }
}

fn judge(d: &mut Describer, p: &ToyPatch, perf: &ToyPerf, list: &str) -> CheckReport {
    let items = parse_list(list).unwrap();
    let main = describe(d, p, perf);
    let nuisance: Vec<Descriptors> = nuisance_phases(p.detune_ct > 0.0)
        .iter()
        .map(|&ph| common::describe_with(d, p, perf, ph, 0.0))
        .collect();
    check(&items, &main, &nuisance)
}

fn row(r: &CheckReport, item: &str) -> enth_analysis::expect::ItemResult {
    r.results
        .iter()
        .find(|x| x.expect == item)
        .unwrap_or_else(|| panic!("{item} not in {r:?}"))
        .clone()
}

/// The trial's two real failures under corrected descriptors (report 03 §F1): g4 run 3 fails `attack>8ms`
/// (programmed 6 ms; the spike read 38 ms) and g5 run 1 fails `sustain>-4dB` (programmed -6 dB; the spike read -2.0).
#[test]
fn trial_failures_are_caught() {
    let mut d = Describer::new(SR);
    let trials = trial_patches();
    let (_, g4, perf4) = trials.iter().find(|t| t.0 == "g4-bell-run3").unwrap();
    let r = judge(&mut d, g4, perf4, goal_list("g4"));
    let a = row(&r, "envelope.attack_ms > 8");
    assert!(!a.pass && a.got.unwrap() < 8.0, "{a:?}");
    assert!(
        row(&r, "detune.beating_hz < 8").pass && row(&r, "envelope.decay_ms > 600").pass,
        "{r:?}"
    );
    let (_, g5, perf5) = trials
        .iter()
        .find(|t| t.0 == "g5-supersaw-lead-run1")
        .unwrap();
    let r = judge(&mut d, g5, perf5, goal_list("g5"));
    let s = row(&r, "envelope.sustain_db > -4");
    assert!(
        !s.pass && s.got.unwrap() < -4.0 && s.got.unwrap() > -5.5,
        "{s:?}"
    );
    assert_eq!(r.failed, 1, "{r:?}");
}

/// The whole trial re-judged (report 03 §F1): 13 of 15 pass, 3 of them within noise. Here g5 run 3's
/// `sustain>-4dB` reads -4.04 dB, a thin fail instead of the report's thin pass at -4.0 (this crate measures the
/// float render, the research the 16-bit file): so 12 or 13 pass, the two real failures always fail, and every
/// other failing item is thin.
#[test]
#[ignore = "45 renders of 2 to 6 s; run in release"]
fn trial_rejudged() {
    let mut d = Describer::new(SR);
    let mut passed = 0;
    let mut failing = Vec::new();
    let mut thin = Vec::new();
    for (name, p, perf) in trial_patches() {
        let r = judge(&mut d, &p, &perf, goal_list(name));
        passed += usize::from(r.pass);
        for x in &r.results {
            let line = format!(
                "{name}: {} got {:?} margin {:?} noise {:?}",
                x.expect, x.got, x.margin, x.noise
            );
            println!(
                "{line}{}{}",
                if x.pass { "" } else { "  FAIL" },
                if x.thin { "  thin" } else { "" }
            );
            if !x.pass {
                failing.push((line.clone(), x.thin));
            }
            if x.thin {
                thin.push(line);
            }
        }
    }
    let hard = [
        "g4-bell-run3: envelope.attack_ms > 8",
        "g5-supersaw-lead-run1: envelope.sustain_db > -4",
    ];
    for h in hard {
        assert!(
            failing.iter().any(|(f, _)| f.starts_with(h)),
            "{h} should fail: {failing:#?}"
        );
    }
    for (f, is_thin) in &failing {
        assert!(
            *is_thin || hard.iter().any(|h| f.starts_with(h)),
            "unexpected clear failure: {f}"
        );
    }
    assert!(
        (12..=13).contains(&passed),
        "{passed} passed; failing: {failing:#?}"
    );
    assert!(
        thin.iter()
            .any(|t| t.starts_with("g4-bell-run1: envelope.attack_ms > 8")),
        "{thin:#?}"
    );
}
