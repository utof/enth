//! Solver behaviour on the analytic stand-in evaluator (`common::Synthetic`): fast, and independent of the
//! engine's modules. Determinism hash, render budgets, the no-trade guarantee as a property, which clip each phase
//! renders, and the error paths.

mod common;

use common::{Recorder, Reversed, Short, Synthetic, items, set, two_osc, with_filter};
use enth_patch::{ParamAddr, Patch};
use enth_render::{Performance, RenderSpec};
use enth_solver::{BlockReason, Evaluator, HintCfg, HintError, HintResult, hint};
use proptest::prelude::*;

fn addrs(list: &[&str]) -> Vec<ParamAddr> {
    list.iter().map(|a| a.parse().unwrap()).collect()
}

fn run(
    patch: &Patch,
    perf: &Performance,
    list: &str,
    eval: &mut dyn Evaluator,
    cfg: &HintCfg,
) -> HintResult {
    hint(patch, perf, &RenderSpec::default(), &items(list), eval, cfg).unwrap()
}

/// A lead like the probe's g5: detune too wide for the beating window, sustain too low, release too long.
fn lead() -> (Patch, Performance, &'static str) {
    let mut p = with_filter(two_osc(0.0, 30.0), 6_000.0);
    set(&mut p, "env1.sustain", 50.0);
    set(&mut p, "env1.release", 900.0);
    (
        p,
        Performance::note(69, 0.8, 2.0, 3.0),
        "pitch = A4 ± 10ct, attack < 20ms, brightness in [2kHz, 4kHz], sustain > -4dB, beating in [3Hz, 6Hz], \
         release < 400ms, duration > 2000ms",
    )
}

/// FNV-1a over the parts of a result that matter (moves, verdicts, counters), with every number as its bits.
fn digest(r: &HintResult) -> u64 {
    use std::fmt::Write as _;
    let mut text = String::new();
    if let Some(f) = &r.fix {
        for m in &f.moves {
            let _ = write!(
                text,
                "move {} {:08x} {:08x};",
                m.addr,
                m.from.to_bits(),
                m.to.to_bits()
            );
        }
        for a in &f.after {
            let got = a.got.map_or(0, f64::to_bits);
            let _ = write!(text, "after {} {} {got:016x};", a.expect, a.pass);
        }
        let _ = write!(
            text,
            "{} {} {};",
            f.pass_all, f.single_param_enough, f.rounded
        );
    }
    for b in &r.blocked {
        let _ = write!(
            text,
            "blocked {} {} {:?} {:?};",
            b.item,
            b.reason.code(),
            b.at_bound
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            b.conflicts_with
        );
    }
    let _ = write!(text, "{:?}", r.stats);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[test]
fn determinism_hash() {
    // Pinned. The solver is serial, `f64` + `libm`, in fixed loop orders: this value is the same on every platform
    // and for any evaluator that keeps the job order. A deliberate change to the algorithm updates it (and says
    // why in the commit message); anything else changing it is a determinism bug.
    let (p, perf, list) = lead();
    let cfg = HintCfg {
        only: Some(addrs(&[
            "osc1.fine",
            "osc2.fine",
            "filter1.cutoff",
            "env1.attack",
            "env1.decay",
            "env1.sustain",
            "env1.release",
            "master.level",
        ])),
        ..HintCfg::default()
    };
    let a = run(&p, &perf, list, &mut Synthetic, &cfg);
    let b = run(&p, &perf, list, &mut Reversed(Synthetic), &cfg);
    assert_eq!(a, b, "the order of work inside a batch changed the result");
    let fix = a.fix.as_ref().expect("the lead gets a fix");
    assert!(fix.pass_all, "{a:#?}");
    assert_eq!(digest(&a), 0x12e9_a364_9e51_23e8, "{a:#?}");
}

#[test]
fn passing_lists_cost_one_render() {
    let mut rec = Recorder::new(Synthetic);
    let r = run(
        &Patch::init(),
        &Performance::note(69, 0.8, 1.0, 2.0),
        "attack < 20ms",
        &mut rec,
        &HintCfg::default(),
    );
    assert!(r.fix.is_none() && r.blocked.is_empty());
    assert_eq!((r.stats.renders, rec.jobs), (1, 1));
}

#[test]
fn performance_limits_cost_no_probes() {
    // The note is never released inside the clip: no release to measure, whatever the patch.
    let mut rec = Recorder::new(Synthetic);
    let r = run(
        &Patch::init(),
        &Performance::note(69, 0.8, 3.0, 2.0),
        "release < 300ms",
        &mut rec,
        &HintCfg::default(),
    );
    assert_eq!(rec.jobs, 1);
    assert_eq!(r.blocked.len(), 1);
    assert_eq!(r.blocked[0].reason, BlockReason::Performance);
}

#[test]
fn envelope_and_beating_items_probe_with_the_full_performance() {
    let (p, perf, list) = lead();
    let mut rec = Recorder::new(Synthetic);
    let r = run(&p, &perf, list, &mut rec, &HintCfg::default());
    assert_eq!(r.stats.short_renders, 0);
    assert!(rec.clip_lengths.iter().all(|d| *d == perf.duration_s));
}

#[test]
fn spectral_items_probe_on_a_short_clip_and_verify_on_the_full_one() {
    let p = with_filter(Patch::init(), 6_000.0);
    let perf = Performance::note(69, 0.8, 2.0, 3.0);
    let mut rec = Recorder::new(Synthetic);
    let r = run(
        &p,
        &perf,
        "brightness < 2kHz, pitch = A4 ± 20ct",
        &mut rec,
        &HintCfg::default(),
    );
    let fix = r.fix.as_ref().expect("a filter can darken the sound");
    assert!(fix.pass_all);
    let short = rec.clip_lengths.iter().filter(|d| **d == 0.5).count();
    assert_eq!(short, r.stats.short_renders);
    assert_eq!(
        short, r.stats.probe_renders,
        "every probe, and only probes, on the short clip"
    );
    // The base, the verified steps and the final render use the goal's performance.
    let full = rec.clip_lengths.iter().filter(|d| **d == 3.0).count();
    assert_eq!(full, r.stats.renders - short, "{:?}", rec.clip_lengths);
}

#[test]
fn errors() {
    let p = Patch::init();
    let perf = Performance::note(69, 0.8, 1.0, 2.0);
    let spec = RenderSpec::default();
    let cfg = HintCfg::default();
    let one = items("attack > 30ms");
    let e = hint(&p, &perf, &spec, &[], &mut Synthetic, &cfg).unwrap_err();
    assert_eq!(e, HintError::NoItems);
    let zero = HintCfg {
        max_renders: 0,
        ..HintCfg::default()
    };
    assert_eq!(
        hint(&p, &perf, &spec, &one, &mut Synthetic, &zero).unwrap_err(),
        HintError::NoBudget
    );
    let silent = Performance {
        duration_s: 1.0,
        ..Performance::default()
    };
    assert_eq!(
        hint(&p, &silent, &spec, &one, &mut Synthetic, &cfg).unwrap_err(),
        HintError::NoNotes
    );
    let e = hint(&p, &perf, &spec, &one, &mut Short, &cfg).unwrap_err();
    assert_eq!(e.code(), "evaluator_contract");
}

#[test]
fn moves_print_as_the_values_they_set() {
    let (p, perf, list) = lead();
    let r = run(&p, &perf, list, &mut Synthetic, &HintCfg::default());
    let fix = r.fix.unwrap();
    for m in &fix.moves {
        assert_eq!(
            m.spec.parse(&m.to_text()).unwrap().to_bits(),
            m.to.to_bits()
        );
        assert_eq!(fix.patch.value(&m.addr), Some(m.to));
    }
    assert_eq!(fix.ops().len(), fix.moves.len());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// For any budget and any list: the budget holds, every evaluation is counted, a fix never turns a passing item
    /// into a failing one, and its `after` is what a fresh evaluation of the fixed patch says.
    #[test]
    fn budget_holds_and_nothing_is_traded(
        budget in 1usize..28,
        fine2 in -60.0f32..60.0,
        sustain in 0.0f32..100.0,
        release in 0.0f32..3000.0,
        beat_lo in 0.5f64..8.0,
        beat_w in 0.5f64..6.0,
        pitch_tol in 2.0f64..40.0,
        dur in 300.0f64..3000.0,
        sus_db in -30.0f64..-1.0,
    ) {
        let mut p = two_osc(0.0, fine2);
        set(&mut p, "env1.sustain", sustain);
        set(&mut p, "env1.release", release);
        let perf = Performance::note(69, 0.8, 1.5, 2.5);
        let list = format!(
            "pitch = A4 ± {pitch_tol}ct, beating in [{beat_lo}Hz, {}Hz], sustain > {sus_db}dB, duration > {dur}ms, \
             release < 500ms",
            beat_lo + beat_w
        );
        let cfg = HintCfg { max_renders: budget, ..HintCfg::default() };
        let mut rec = Recorder::new(Synthetic);
        let r = run(&p, &perf, &list, &mut rec, &cfg);
        let s = &r.stats;
        prop_assert!(s.renders <= budget, "{s:?}");
        prop_assert_eq!(s.renders, rec.jobs);
        prop_assert_eq!(s.batches, rec.batches);
        prop_assert_eq!(
            s.renders,
            s.base_renders + s.probe_renders + s.absent_renders + s.verify_renders + s.final_renders
        );
        if let Some(fix) = &r.fix {
            for (b, a) in r.before.iter().zip(&fix.after) {
                prop_assert!(!b.pass || a.pass, "{} passed before and fails after", b.expect);
            }
            let again = run(&fix.patch, &perf, &list, &mut Synthetic, &cfg);
            let flags: Vec<bool> = again.before.iter().map(|x| x.pass).collect();
            let after: Vec<bool> = fix.after.iter().map(|x| x.pass).collect();
            prop_assert_eq!(flags, after);
            prop_assert!(fix.after.iter().filter(|x| x.pass).count() > r.before.iter().filter(|x| x.pass).count()
                || fix.moves.iter().any(|m| m.from != m.to));
        }
        // A blocked item is a failing one.
        for b in &r.blocked {
            let failing = r.fix.as_ref().map_or(!r.before[b.item].pass, |f| !f.after[b.item].pass);
            prop_assert!(failing, "{b:?}");
        }
    }
}
