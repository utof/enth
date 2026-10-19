//! Folding the host's values against the plan's compiled value (lane S2s, F1b review issue 4): the patch ends on the
//! exact plain value the engine plays, also when the host returns to `to_norm` of the value the plan holds after
//! earlier folds moved the patch.

use enth_engine::{Event, EventKind};
use enth_params::ParamSpec;
use enth_patch::Op;
use proptest::prelude::*;

use super::kit::{addr, base, edit, live, patch, port, run_events, send_plan};
use crate::Session;
use crate::bridge::HostValues;

const CUTOFF: &str = "filter1.cutoff";

fn spec(session: &Session) -> &'static ParamSpec {
    session.patch().spec(&addr(CUTOFF)).unwrap()
}

/// A cutoff in Hz whose trip plain → normalised → plain does not come back to it: the "one rounding step" is real.
fn awkward_cutoff() -> f32 {
    let spec = patch().spec(&addr(CUTOFF)).unwrap();
    (300..9000)
        .map(|hz| hz as f32 + 0.5)
        .find(|&v| spec.from_norm(spec.to_norm(v)) != v && spec.to_norm(v) < 0.9)
        .expect("log scales have such values")
}

/// A live session whose plan was compiled with the cutoff at `p0`, and the host values that match it.
fn with_cutoff(p0: f32) -> (Session, enth_engine::Engine, HostValues) {
    let mut p = patch();
    enth_patch::apply(
        &mut p,
        Op::SetParam {
            addr: addr(CUTOFF),
            value: p0,
        },
    )
    .unwrap();
    let (mut session, engine) = live(p);
    let host = session.host_values();
    assert!(session.fold_host(&host).baseline, "the first fold records");
    (session, engine, host)
}

/// The host moves slot 1 to `n`: the shell sends the engine a slot event, the editor folds it.
fn host_moves(
    session: &mut Session,
    engine: &mut enth_engine::Engine,
    host: &mut HostValues,
    n: f32,
) {
    host.slots[0] = n;
    session.fold_host(host);
    let event = Event {
        offset: 0,
        kind: EventKind::Slot { slot: 0, value: n },
    };
    run_events(session, engine, 3, &[event]);
}

fn cutoff_bits(session: &Session) -> u32 {
    session.patch().value(&addr(CUTOFF)).unwrap().to_bits()
}

#[test]
fn a_host_returning_to_the_compiled_value_leaves_the_patch_on_the_value_the_engine_plays() {
    let p0 = awkward_cutoff();
    let (mut session, mut engine, mut host) = with_cutoff(p0);
    let n0 = spec(&session).to_norm(p0);
    assert_ne!(
        spec(&session).from_norm(n0),
        p0,
        "the premise: from_norm(to_norm(p0)) is not p0"
    );

    // Away: the engine plays from_norm(n1), the patch follows.
    host_moves(&mut session, &mut engine, &mut host, n0 + 0.2);
    assert_ne!(cutoff_bits(&session), p0.to_bits());
    assert_eq!(
        base(&session, &engine, CUTOFF).to_bits(),
        cutoff_bits(&session)
    );
    assert_eq!(session.host.memos.len(), 1, "the plan still holds p0");
    assert_eq!(session.host.memos[0].engine, p0);

    // Back to exactly to_norm(p0): the engine's slot table resolves it to p0 itself, and so does the fold.
    host_moves(&mut session, &mut engine, &mut host, n0);
    assert_eq!(base(&session, &engine, CUTOFF).to_bits(), p0.to_bits());
    assert_eq!(
        cutoff_bits(&session),
        p0.to_bits(),
        "no rounding step between them"
    );
    assert!(session.host.memos.is_empty(), "patch and plan agree again");
}

#[test]
fn without_an_engine_the_fold_uses_the_patchs_own_value() {
    let p0 = awkward_cutoff();
    let mut p = patch();
    enth_patch::apply(
        &mut p,
        Op::SetParam {
            addr: addr(CUTOFF),
            value: p0,
        },
    )
    .unwrap();
    let mut session = Session::new(p);
    let mut host = session.host_values();
    session.fold_host(&host);
    let n0 = host.slots[0];
    host.slots[0] = n0 + 0.2;
    session.fold_host(&host);
    host.slots[0] = n0;
    session.fold_host(&host);
    assert!(
        session.host.memos.is_empty(),
        "nothing to remember without a plan"
    );
    // The patch rule (rule 3 of the module docs): back at to_norm(p0) the patch takes from_norm of it.
    assert_eq!(
        cutoff_bits(&session),
        spec(&session).from_norm(n0).to_bits()
    );
}

#[test]
fn what_a_fold_remembers_ends_with_the_plan_or_with_any_other_change() {
    let p0 = awkward_cutoff();
    let moved = |session: &mut Session, engine: &mut enth_engine::Engine, host: &mut HostValues| {
        let n0 = spec(session).to_norm(p0);
        host_moves(session, engine, host, n0 + 0.2);
        assert_eq!(session.host.memos.len(), 1);
    };

    // A plan sent: it is compiled from the patch, so the next fold starts from the patch's value.
    let (mut session, mut engine, mut host) = with_cutoff(p0);
    moved(&mut session, &mut engine, &mut host);
    session.port.as_mut().unwrap().compile_pending = true;
    assert_eq!(send_plan(&mut session), Some(1));
    session.fold_host(&host);
    assert!(session.host.memos.is_empty());

    // An edit of that value: the engine got an event, so its value is the patch's.
    let (mut session, mut engine, mut host) = with_cutoff(p0);
    moved(&mut session, &mut engine, &mut host);
    edit(
        &mut session,
        vec![Op::SetParam {
            addr: addr(CUTOFF),
            value: 1500.0,
        }],
    );
    session.fold_host(&host);
    assert!(session.host.memos.is_empty());

    // A load replaces both.
    let (mut session, mut engine, mut host) = with_cutoff(p0);
    moved(&mut session, &mut engine, &mut host);
    let text = session.patch().to_json();
    session.load_patch(&text);
    assert!(session.host.memos.is_empty());

    // A deactivation: nothing to remember.
    let (mut session, mut engine, mut host) = with_cutoff(p0);
    moved(&mut session, &mut engine, &mut host);
    session.deactivate();
    session.fold_host(&host);
    assert!(session.host.memos.is_empty());
}

#[derive(Clone, Debug)]
enum Step {
    /// The host sends a normalised value (anywhere).
    Host(f32),
    /// The host returns to `to_norm` of the value the plan was compiled with.
    HostHome,
    /// The editor sets the cutoff; the host's echo of it arrives.
    Edit(u32),
    /// A plan goes out (a structural edit elsewhere).
    Plan,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => (0.0f32..=1.0).prop_map(Step::Host),
        3 => Just(Step::HostHome),
        2 => (100u32..9000).prop_map(Step::Edit),
        1 => Just(Step::Plan),
    ]
}

proptest! {
    /// The invariant the memo exists for: after every host move, edit, echo and plan swap, the patch's cutoff and the
    /// engine's base value are the same bits.
    #[test]
    fn the_patch_and_the_engine_never_part_by_a_rounding_step(steps in prop::collection::vec(step(), 0..40)) {
        let p0 = awkward_cutoff();
        let (mut session, mut engine, mut host) = with_cutoff(p0);
        let mut home = spec(&session).to_norm(p0);
        for step in steps {
            match step {
                Step::Host(n) => host_moves(&mut session, &mut engine, &mut host, n),
                Step::HostHome => host_moves(&mut session, &mut engine, &mut host, home),
                Step::Edit(hz) => {
                    let value = hz as f32;
                    edit(&mut session, vec![Op::SetParam { addr: addr(CUTOFF), value }]);
                    // The shell writes to_norm(value) to the host and the host echoes it.
                    let echo = spec(&session).to_norm(value);
                    home = echo; // the plan's value is now the edited one
                    host_moves(&mut session, &mut engine, &mut host, echo);
                }
                Step::Plan => {
                    session.port.as_mut().unwrap().compile_pending = true;
                    prop_assert!(send_plan(&mut session).is_some());
                    // The compiled value is the patch's now.
                    home = spec(&session).to_norm(session.patch().value(&addr(CUTOFF)).unwrap());
                    run_events(&mut session, &mut engine, 4, &[]);
                    session.fold_host(&host);
                }
            }
            prop_assert_eq!(
                base(&session, &engine, CUTOFF).to_bits(),
                cutoff_bits(&session),
                "after {:?}", step
            );
        }
        prop_assert!(port(&session).link.epoch() < 100);
    }
}
