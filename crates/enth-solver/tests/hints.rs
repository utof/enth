//! Hints on real renders (`enth-render` + `enth-analysis` through `measure`).
//!
//! The cases use only what any oscillator and envelope implementation provides (two detuned oscillators beat at
//! their frequency difference and sound at their mean pitch; a held note lasts at least its hold), and assert
//! properties (no item traded, a verified fix, the right verdict and parameter) rather than exact values, so they
//! survive lanes D1/D2 replacing the reference saw and envelope.

mod common;

use std::sync::OnceLock;

use common::{Recorder, Threads, items, two_osc};
use enth_analysis::Expect;
use enth_patch::{ParamAddr, Patch};
use enth_render::{Performance, RenderSpec};
use enth_solver::{BlockReason, HintCfg, HintResult, Serial, hint};

fn perf() -> Performance {
    Performance::note(69, 0.8, 1.5, 2.0)
}

/// Like the probe's g5 conflict: the beating is too fast, and lowering one oscillator's detune alone would drag
/// the pitch out of its window; both oscillators have to move.
fn conflict() -> (Patch, Vec<Expect>) {
    (
        two_osc(-15.0, 15.0),
        items("pitch = A4 ± 6ct, beating in [3Hz, 6Hz], attack < 20ms"),
    )
}

fn conflict_serial() -> &'static HintResult {
    static R: OnceLock<HintResult> = OnceLock::new();
    R.get_or_init(|| {
        let (p, list) = conflict();
        hint(
            &p,
            &perf(),
            &RenderSpec::default(),
            &list,
            &mut Serial::new(),
            &HintCfg::default(),
        )
        .unwrap()
    })
}

fn addr(a: &str) -> ParamAddr {
    a.parse().unwrap()
}

#[test]
fn conflicting_goals_get_one_fix_that_trades_nothing() {
    let r = conflict_serial();
    assert!(
        r.before[0].pass && !r.before[1].pass && r.before[2].pass,
        "{:#?}",
        r.before
    );
    let fix = r.fix.as_ref().expect("a fix");
    for (b, a) in r.before.iter().zip(&fix.after) {
        assert!(
            !b.pass || a.pass,
            "{} passed before and fails after",
            b.expect
        );
    }
    assert!(fix.pass_all, "{fix:#?}");
    let moved: Vec<String> = fix.moves.iter().map(|m| m.addr.to_string()).collect();
    assert!(
        moved.contains(&"osc1.fine".to_string()) && moved.contains(&"osc2.fine".to_string()),
        "both oscillators move: {moved:?}"
    );
    assert!(!fix.single_param_enough && r.blocked.is_empty());
    assert!(r.stats.renders <= HintCfg::default().max_renders);
    // Verified: a fresh render of the fixed patch reads what the fix reports.
    let again = hint(
        &fix.patch,
        &perf(),
        &RenderSpec::default(),
        &conflict().1,
        &mut Serial::new(),
        &HintCfg::default(),
    )
    .unwrap();
    assert_eq!(again.before, fix.after);
    assert_eq!(
        again.stats.renders, 1,
        "everything passes, so only the base is rendered"
    );
}

#[test]
fn one_parameter_alone_would_trade_so_the_hint_refuses() {
    // Only osc2.fine may move, and the pitch window is tight: fixing the beating with it would break the pitch.
    let (p, _) = conflict();
    let list = items("pitch = A4 ± 3ct, beating in [3Hz, 6Hz], attack < 20ms");
    let cfg = HintCfg {
        only: Some(vec![addr("osc2.fine")]),
        ..HintCfg::default()
    };
    let r = hint(
        &p,
        &perf(),
        &RenderSpec::default(),
        &list,
        &mut Serial::new(),
        &cfg,
    )
    .unwrap();
    assert!(r.before[0].pass && !r.before[1].pass, "{:#?}", r.before);
    if let Some(fix) = &r.fix {
        assert!(
            fix.after[0].pass && fix.after[2].pass,
            "nothing traded: {fix:#?}"
        );
        assert!(!fix.after[1].pass, "{fix:#?}");
    }
    let b = r
        .blocked
        .iter()
        .find(|b| b.item == 1)
        .expect("the beating is blocked");
    assert_eq!(b.reason, BlockReason::Conflict, "{b:#?}");
    assert_eq!(b.conflicts_with, vec![0], "{b:#?}");
}

#[test]
fn an_unreachable_item_is_blocked_with_the_parameter_at_its_bound() {
    // A held note lasts at least its hold (1.5 s): with the level held up, the duration cannot drop below 500 ms.
    let list = items("sustain > -6dB, duration < 500ms");
    let r = hint(
        &Patch::init(),
        &Performance::note(57, 0.8, 1.5, 2.0),
        &RenderSpec::default(),
        &list,
        &mut Serial::new(),
        &HintCfg::default(),
    )
    .unwrap();
    assert!(r.before[0].pass && !r.before[1].pass, "{:#?}", r.before);
    let b = r
        .blocked
        .iter()
        .find(|b| b.item == 1)
        .expect("duration is blocked");
    assert!(
        matches!(b.reason, BlockReason::Performance | BlockReason::AtBound),
        "{b:#?}"
    );
    assert!(b.at_bound.contains(&addr("env1.release")), "{b:#?}");
    assert!(b.why.contains("env1.release"), "{}", b.why);
    if let Some(fix) = &r.fix {
        assert!(fix.after[0].pass, "nothing traded: {fix:#?}");
    }
}

#[test]
fn an_absent_descriptor_the_structure_cannot_make_is_blocked() {
    // One oscillator cannot beat, whatever its tuning.
    let r = hint(
        &Patch::init(),
        &perf(),
        &RenderSpec::default(),
        &items("beating in [3Hz, 9Hz]"),
        &mut Serial::new(),
        &HintCfg::default(),
    )
    .unwrap();
    assert!(r.fix.is_none(), "{:#?}", r.fix);
    assert_eq!(r.blocked.len(), 1);
    assert_eq!(r.blocked[0].reason, BlockReason::Absent);
    assert!(
        r.stats.absent_renders > 0,
        "it looked along osc1.fine first"
    );
    assert!(
        r.blocked[0].why.contains("osc1.fine"),
        "{}",
        r.blocked[0].why
    );
}

#[test]
fn an_absent_descriptor_is_found_along_the_parameter_that_moves_it() {
    // Two oscillators at 0 ct: no beating yet. Detuning one makes it appear.
    let r = hint(
        &two_osc(0.0, 0.0),
        &perf(),
        &RenderSpec::default(),
        &items("beating in [3Hz, 9Hz], pitch = A4 ± 20ct"),
        &mut Serial::new(),
        &HintCfg::default(),
    )
    .unwrap();
    let fix = r.fix.as_ref().expect("a fix");
    assert!(fix.pass_all, "{fix:#?}");
    assert!(
        fix.moves.iter().all(|m| m.addr.param() == "fine"),
        "{:?}",
        fix.moves
    );
}

#[test]
fn the_worker_count_does_not_change_the_hint() {
    let (p, list) = conflict();
    let serial = conflict_serial();
    for n in [1, 4] {
        let r = hint(
            &p,
            &perf(),
            &RenderSpec::default(),
            &list,
            &mut Threads(n),
            &HintCfg::default(),
        )
        .unwrap();
        assert_eq!(&r, serial, "{n} workers");
    }
}

#[test]
fn the_render_budget_is_never_exceeded() {
    let (p, list) = conflict();
    for budget in [1, 2, 5, 9] {
        let cfg = HintCfg {
            max_renders: budget,
            ..HintCfg::default()
        };
        let mut rec = Recorder::new(Serial::new());
        let r = hint(&p, &perf(), &RenderSpec::default(), &list, &mut rec, &cfg).unwrap();
        assert!(r.stats.renders <= budget, "{budget}: {:?}", r.stats);
        assert_eq!(r.stats.renders, rec.jobs);
        if let Some(fix) = &r.fix {
            for (b, a) in r.before.iter().zip(&fix.after) {
                assert!(!b.pass || a.pass, "{budget}: {} traded", b.expect);
            }
        }
    }
}
