//! Test support for the unit tests (lane S2s): an **active** session without lane SK's `activate`, built by hand
//! around a real engine, plus doubles for the two engine-facing bodies the tests need that are not S2s's (`edit`'s
//! record-and-deliver, and the pump's "send the pending plan"). After lane SK lands, the doubles stay: they are
//! three lines each on top of `Session::deliver` and the `Port`.

use enth_engine::{
    Engine, Event, EventKind, LinkCapacity, MAX_PLANS_IN_FLIGHT, NoteKey, ParamId, PlanMap,
    PrepareCfg, compile,
};
use enth_patch::history::GestureId;
use enth_patch::{ModuleId, Op, ParamAddr, Patch, apply_all};

use crate::link::Port;
use crate::model::{Activation, RENDER_SEED, Session, SessionConfig};

pub(crate) const SR: f32 = 48_000.0;

pub(crate) fn addr(text: &str) -> ParamAddr {
    text.parse().unwrap()
}

pub(crate) fn id(text: &str) -> ModuleId {
    ModuleId::new(text).unwrap()
}

/// The init patch plus a filter and an LFO routed to its cutoff, with three slots: enough to exercise every kind of
/// delivery (value, depth, macro, structural, slot).
pub(crate) fn patch() -> Patch {
    use enth_patch::model::{ModuleInstance, Route, Section};
    use enth_patch::{ModSource, ModuleKind, Position, SlotIndex};
    let slot = |n| SlotIndex::from_one_based(n).unwrap();
    let mut p = Patch::init();
    apply_all(
        &mut p,
        &[
            Op::AddModule {
                at: Position {
                    section: Section::VoiceFx,
                    index: 0,
                },
                module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
            },
            Op::AddModule {
                at: Position {
                    section: Section::Modulators,
                    index: 1,
                },
                module: ModuleInstance::new(id("lfo1"), ModuleKind::Lfo),
            },
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: ModSource::Module(id("lfo1")),
                    dst: addr("filter1.cutoff"),
                    depth: 25.0,
                },
            },
            Op::SetSlot {
                slot: slot(1),
                addr: Some(addr("filter1.cutoff")),
            },
            Op::SetSlot {
                slot: slot(2),
                addr: Some(addr("master.level")),
            },
            Op::SetSlot {
                slot: slot(3),
                addr: Some(addr("filter1.resonance")),
            },
        ],
    )
    .unwrap();
    p
}

/// A session with an engine, active without `Session::activate`: the plan of `patch`, the engine's link, a `Port`.
pub(crate) fn live(patch: Patch) -> (Session, Engine) {
    live_with(patch, SessionConfig::default())
}

pub(crate) fn live_with(patch: Patch, config: SessionConfig) -> (Session, Engine) {
    let mut session = Session::with_config(patch, config);
    let (plan, map) = compile(&session.patch, &PrepareCfg::new(SR)).unwrap();
    let (engine, link) = Engine::with_link(plan, RENDER_SEED, session.config.link);
    session.port = Some(Port {
        link,
        map,
        activation: Activation {
            sample_rate: SR,
            offline: false,
        },
        compile_pending: false,
        forget_slots: 0,
        queue: Vec::new(),
        next_audition_id: 0,
    });
    (session, engine)
}

pub(crate) fn port(session: &Session) -> &Port {
    session.port.as_ref().unwrap()
}

/// Record and deliver a transaction the way an edit does (`Session::edit` is lane SK's): apply, record with the
/// session's open gesture, deliver. Returns what the delivery did.
pub(crate) fn edit(session: &mut Session, ops: Vec<Op>) -> crate::Applied {
    let gesture = session.gestures.open;
    edit_in(session, ops, gesture)
}

pub(crate) fn edit_in(
    session: &mut Session,
    ops: Vec<Op>,
    gesture: Option<GestureId>,
) -> crate::Applied {
    let before = session.patch.clone();
    session
        .history
        .edit(&mut session.patch, ops.clone(), gesture)
        .unwrap();
    session.deliver(&before, &ops)
}

/// What the pump does with a pending compile, minimally: collect returned plans, then compile the patch as it is now,
/// mark it with the slots to forget, send it, and keep its map. (Lane SK's `Session::pump` does this and more;
/// `Session::deliver` calls that one, so with it in place this finds nothing pending.)
pub(crate) fn send_plan(session: &mut Session) -> Option<u64> {
    let patch = &session.patch;
    let port = session.port.as_mut()?;
    port.link.collect();
    if !port.compile_pending || port.link.in_flight() >= MAX_PLANS_IN_FLIGHT {
        return None;
    }
    let (mut plan, map) = compile(patch, &PrepareCfg::new(port.activation.sample_rate)).unwrap();
    plan.forget_host_slots(port.forget_slots);
    let epoch = port.link.send_plan(plan).ok()?;
    port.map = map;
    port.compile_pending = false;
    port.forget_slots = 0;
    port.queue.retain(|q| {
        !matches!(
            q,
            crate::link::Queued::Param { .. }
                | crate::link::Queued::Depth { .. }
                | crate::link::Queued::Macro { .. }
        )
    });
    Some(epoch)
}

/// Run the engine for `blocks` blocks of 64 frames (collecting returned plans on the way), no host events.
pub(crate) fn run(session: &mut Session, engine: &mut Engine, blocks: usize) {
    run_events(session, engine, blocks, &[]);
}

pub(crate) fn run_events(
    session: &mut Session,
    engine: &mut Engine,
    blocks: usize,
    events: &[Event],
) {
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    for k in 0..blocks {
        engine.process(&mut l, &mut r, if k == 0 { events } else { &[] });
        if let Some(port) = session.port.as_mut() {
            port.link.collect();
        }
    }
}

pub(crate) fn note_on(key: u8) -> Event {
    Event {
        offset: 0,
        kind: EventKind::NoteOn {
            key: NoteKey::new(key, 1),
            velocity: 0.8,
        },
    }
}

/// The engine's base value of the parameter at `a`, through the session's current map.
pub(crate) fn base(session: &Session, engine: &Engine, a: &str) -> f32 {
    let id: ParamId = port(session).map.get(&addr(a)).unwrap();
    engine.base(id).unwrap()
}

pub(crate) fn map(session: &Session) -> &PlanMap {
    &port(session).map
}

/// Two plans in flight and none collected: the link cannot take a third, so a compile stays pending whatever the
/// pump does (the F1b "two in flight" case).
pub(crate) fn block_plans(session: &mut Session) {
    for _ in 0..MAX_PLANS_IN_FLIGHT {
        session.port.as_mut().unwrap().compile_pending = true;
        assert!(send_plan(session).is_some());
    }
    assert_eq!(port(session).link.in_flight(), MAX_PLANS_IN_FLIGHT);
}
