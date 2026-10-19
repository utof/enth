//! API compile tests (lane S1s): the traits are object-safe and implementable outside the crate, the session is
//! `Send` (it lives behind the plugin's mutex), the P1b stubs answer with typed errors, the delivery table holds per
//! op kind, gestures balance, and the telemetry reader maps tap order to addresses.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use enth_engine::NoteKey;
use enth_patch::history::History;
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{ModSource, ModuleKind, Op, Patch, PatchMeta, Position, RouteAddr};
use enth_session::apply::{delivery, renames_params, route};
use enth_session::{
    Activation, Applied, Delivery, GestureId, HistoryView, InstanceState, PumpReport, Routing,
    Session, SessionConfig, SessionEdit, SessionError, SessionStatus, SessionView, ShellRequest,
    TelemetryFrame, TelemetryMap, TelemetryReader, TelemetrySource, UiState,
};

const fn assert_send<T: Send>() {}
const fn assert_send_sync<T: Send + Sync + ?Sized>() {}

#[test]
fn the_session_can_live_behind_the_plugins_mutex() {
    assert_send::<Session>();
    assert_send_sync::<dyn TelemetrySource>();
    assert_send::<TelemetryReader>();
    assert_send::<ShellRequest>();
    assert_send::<SessionError>();
}

/// A kittest-style fake: the editor's traits can be implemented without an engine, outside this crate.
struct Recording {
    patch: Patch,
    history: History,
    instance: InstanceState,
    edits: Vec<(Vec<Op>, Option<GestureId>)>,
    next: u64,
}

impl SessionView for Recording {
    fn patch(&self) -> &Patch {
        &self.patch
    }
    fn history(&self) -> HistoryView<'_> {
        HistoryView::new(&self.history)
    }
    fn status(&self) -> SessionStatus {
        let mut s = SessionStatus::default();
        s.gesture_open = self.next > 0;
        s
    }
    fn instance(&self) -> &InstanceState {
        &self.instance
    }
    fn telemetry_map(&self, _epoch: u64) -> Option<&TelemetryMap> {
        None
    }
    fn telemetry_source(&self) -> Option<Arc<dyn TelemetrySource>> {
        None
    }
}

impl SessionEdit for Recording {
    fn begin_gesture(&mut self) -> GestureId {
        self.next += 1;
        GestureId(self.next)
    }
    fn end_gesture(&mut self) {}
    fn edit(&mut self, ops: Vec<Op>, gesture: Option<GestureId>) -> Result<Applied, SessionError> {
        self.edits.push((ops, gesture));
        Ok(Applied::default())
    }
    fn undo(&mut self) -> Result<bool, SessionError> {
        Ok(false)
    }
    fn redo(&mut self) -> Result<bool, SessionError> {
        Ok(false)
    }
    fn set_ui(&mut self, ui: UiState) {
        self.instance.ui = ui;
    }
    fn set_monitor_bypass(&mut self, on: bool) {
        self.instance.monitor.fx_bypass = on;
    }
    fn audition_on(&mut self, key: u8, _velocity: f32) -> Result<NoteKey, SessionError> {
        Ok(NoteKey::new(key, 0))
    }
    fn audition_off(&mut self, _note: NoteKey) -> Result<(), SessionError> {
        Ok(())
    }
    fn audition_all_off(&mut self) -> Result<(), SessionError> {
        Ok(())
    }
}

/// What an editor frame does through the trait object (stand-in for `enth-ui`'s editor).
fn editor_frame(session: &mut dyn SessionEdit) -> Result<Applied, SessionError> {
    let osc = session.patch().generators[0].id.clone();
    let _ = session.history().can_undo();
    let g = session.begin_gesture();
    let addr = enth_patch::ParamAddr::module(osc, "level").unwrap();
    let r = session.edit(vec![Op::SetParam { addr, value: 50.0 }], Some(g));
    session.end_gesture();
    r
}

#[test]
fn the_editor_traits_are_object_safe_and_implementable_elsewhere() {
    let mut fake = Recording {
        patch: Patch::init(),
        history: History::new(),
        instance: InstanceState::default(),
        edits: Vec::new(),
        next: 0,
    };
    assert!(editor_frame(&mut fake).is_ok());
    assert_eq!(fake.edits.len(), 1);
    assert_eq!(fake.edits[0].1, Some(GestureId(1)));

    let mut session = Session::default();
    let views: Vec<Box<dyn SessionView>> = vec![Box::new(Session::default())];
    assert!(!views[0].history().can_undo());
    // The real session's edit is lane SK's: until then it answers with a typed stub error.
    let err = editor_frame(&mut session).unwrap_err();
    assert_eq!(err.code(), "unsupported");
    assert!(matches!(err, SessionError::Unsupported { lane: "SK", .. }));
}

#[test]
fn a_detached_session_reports_its_state() {
    let session = Session::default();
    let status = session.status();
    assert_eq!(status, SessionStatus::default());
    assert!(!session.is_active());
    assert!(session.telemetry_source().is_none());
    assert!(session.telemetry_map(0).is_none());
    assert!(identical(session.patch(), &Patch::init()));
    assert_eq!(session.instance(), &InstanceState::default());
    let config = SessionConfig::default();
    assert_eq!(session.config(), &config);
    assert!(format!("{session:?}").starts_with("Session"));
}

fn identical(a: &Patch, b: &Patch) -> bool {
    enth_patch::canon::identical(a, b)
}

#[test]
fn the_return_ring_always_fits_every_outstanding_plan() {
    let mut config = SessionConfig::default();
    config.link.returns = 0;
    let session = Session::with_config(Patch::init(), config);
    assert!(session.config().link.returns >= enth_engine::MAX_PLANS_IN_FLIGHT);
}

#[test]
fn the_stubs_answer_with_typed_errors() {
    let mut session = Session::default();
    let activation = Activation {
        sample_rate: 48_000.0,
        offline: false,
    };
    let Err(err) = session.activate(activation) else {
        panic!("activate is a stub until lane SK")
    };
    assert!(matches!(
        err,
        SessionError::Unsupported {
            what: "activate",
            lane: "SK"
        }
    ));
    assert_eq!(session.pump(), PumpReport::default());
    assert!(matches!(
        session.undo(),
        Err(SessionError::Unsupported { lane: "S2s", .. })
    ));
    assert!(matches!(
        session.redo(),
        Err(SessionError::Unsupported { lane: "S2s", .. })
    ));
    assert_eq!(session.audition_on(60, 1.0), Err(SessionError::Inactive));
    assert_eq!(
        session.audition_off(NoteKey::new(60, 0)),
        Err(SessionError::Inactive)
    );
    assert_eq!(session.audition_all_off(), Err(SessionError::Inactive));
    session.deactivate();
    assert!(!session.is_active());
    for e in [
        err,
        SessionError::Inactive,
        SessionError::Busy,
        SessionError::Offline,
    ] {
        assert_ne!(e.hint(), "");
        assert_ne!(e.to_string(), "");
    }
}

#[test]
fn gestures_balance_and_end_is_idempotent() {
    let mut session = Session::default();
    let a = session.begin_gesture();
    assert!(session.status().gesture_open);
    let b = session.begin_gesture();
    assert_ne!(a, b, "a new gesture ends the open one");
    session.end_gesture();
    session.end_gesture();
    assert!(!session.status().gesture_open);
    assert!(
        !session.history().can_undo(),
        "a gesture with no edit leaves no undo step"
    );
    assert_eq!(
        session.drain_requests().count(),
        0,
        "no host gesture was opened"
    );
}

#[test]
fn the_monitor_bypass_is_reported_once_per_change() {
    let mut session = Session::default();
    session.set_monitor_bypass(false);
    session.set_monitor_bypass(true);
    session.set_monitor_bypass(true);
    let requests: Vec<_> = session.drain_requests().collect();
    assert_eq!(requests, vec![ShellRequest::MonitorBypass(true)]);
    assert!(session.instance().monitor.fx_bypass);
    let ui = UiState {
        layout: "flow".to_owned(),
        ..UiState::default()
    };
    session.set_ui(ui.clone());
    assert_eq!(session.instance().ui, ui);
    assert_eq!(session.status().requests, 0);
}

#[test]
fn the_delivery_table_per_op_kind() {
    let p = common::custom_patch();
    let a = common::addr;
    let lfo_to_cutoff = RouteAddr {
        src: ModSource::Module(common::id("lfo1")),
        dst: a("filter1.cutoff"),
    };
    let velocity_to_level = RouteAddr {
        src: ModSource::Velocity,
        dst: a("osc1.level"),
    };
    let mut zero_depth = p.clone();
    enth_patch::apply(
        &mut zero_depth,
        Op::InsertRoute {
            index: 1,
            route: Route {
                src: velocity_to_level.src.clone(),
                dst: velocity_to_level.dst.clone(),
                depth: 0.0,
            },
        },
    )
    .unwrap();

    let cases: Vec<(&Patch, Op, Routing)> = vec![
        (
            &p,
            Op::SetParam {
                addr: a("filter1.cutoff"),
                value: 900.0,
            },
            Routing::Value(a("filter1.cutoff")),
        ),
        (
            &p,
            Op::SetParam {
                addr: a("master.level"),
                value: -6.0,
            },
            Routing::Value(a("master.level")),
        ),
        (
            &p,
            Op::SetParam {
                addr: a("voice.polyphony"),
                value: 4.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetParam {
                addr: a("voice.mode"),
                value: 1.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetParam {
                addr: a("lfo1.scope"),
                value: 1.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetParam {
                addr: a("nosuch1.cutoff"),
                value: 1.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetDepth {
                route: lfo_to_cutoff.clone(),
                depth: 50.0,
            },
            Routing::Depth(lfo_to_cutoff.clone()),
        ),
        (
            &p,
            Op::SetDepth {
                route: lfo_to_cutoff.clone(),
                depth: 0.0,
            },
            Routing::Recompile,
        ),
        (
            &zero_depth,
            Op::SetDepth {
                route: velocity_to_level.clone(),
                depth: 30.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetDepth {
                route: velocity_to_level.clone(),
                depth: 30.0,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetMacro {
                index: common::macro_index(2),
                value: 0.5,
            },
            Routing::Macro(common::macro_index(2)),
        ),
        (
            &p,
            Op::SetSlot {
                slot: common::slot(3),
                addr: Some(a("filter1.resonance")),
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetSlot {
                slot: common::slot(1),
                addr: None,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetLabel {
                id: common::id("filter1"),
                label: None,
            },
            Routing::Silent,
        ),
        (
            &p,
            Op::SetMeta {
                meta: PatchMeta::default(),
            },
            Routing::Silent,
        ),
        (
            &p,
            Op::SetMacroName {
                index: common::macro_index(1),
                name: None,
            },
            Routing::Silent,
        ),
        (
            &p,
            Op::AddModule {
                at: Position {
                    section: Section::Fx,
                    index: 0,
                },
                module: ModuleInstance::new(common::id("verb1"), ModuleKind::Reverb),
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::RemoveModule {
                id: common::id("filter1"),
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::Move {
                id: common::id("lfo1"),
                to: Position {
                    section: Section::Modulators,
                    index: 0,
                },
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::InsertRoute {
                index: 0,
                route: Route {
                    src: ModSource::Velocity,
                    dst: a("osc1.level"),
                    depth: 10.0,
                },
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::RemoveRoute {
                route: lfo_to_cutoff.clone(),
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetEnabled {
                id: common::id("filter1"),
                enabled: false,
            },
            Routing::Recompile,
        ),
        (
            &p,
            Op::SetAmpEnv {
                id: common::id("env1"),
            },
            Routing::Recompile,
        ),
    ];
    for (patch, op, expected) in cases {
        assert_eq!(route(patch, &op), expected, "{op:?}");
    }
}

#[test]
fn a_transaction_with_any_structural_op_is_one_plan() {
    let p = common::custom_patch();
    let value = Op::SetParam {
        addr: common::addr("filter1.cutoff"),
        value: 900.0,
    };
    let structural = Op::SetEnabled {
        id: common::id("filter1"),
        enabled: false,
    };
    let label = Op::SetLabel {
        id: common::id("filter1"),
        label: None,
    };
    assert_eq!(
        delivery(&p, &[value.clone(), label.clone()]),
        Delivery::Events(vec![
            Routing::Value(common::addr("filter1.cutoff")),
            Routing::Silent
        ])
    );
    assert!(delivery(&p, &[value, structural, label.clone()]).is_recompile());
    assert_eq!(delivery(&p, &[]), Delivery::Events(Vec::new()));
    assert!(renames_params(&label));
    assert!(!renames_params(&Op::SetMacro {
        index: common::macro_index(1),
        value: 0.1
    }));
}

/// A fake tap: the editor and kittest can drive telemetry without an engine.
struct FakeTap {
    generation: AtomicU64,
    enabled: AtomicBool,
    frame: TelemetryFrame,
}

impl TelemetrySource for FakeTap {
    fn frame(&self) -> TelemetryFrame {
        TelemetryFrame {
            params: vec![0.0; 4],
            modulators: vec![0.0; 2],
            ..TelemetryFrame::default()
        }
    }
    fn read(&self, out: &mut TelemetryFrame) {
        out.clone_from(&self.frame);
        out.generation = self.generation.load(Ordering::Relaxed);
    }
    fn scope(&self, out: &mut [f32]) -> u64 {
        out.fill(0.0);
        out.len() as u64
    }
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
    fn sleep(&self, seen: u64) -> bool {
        self.generation() == seen
    }
    fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }
}

/// A view whose telemetry map knows epoch 3 only.
struct MapOnly(TelemetryMap, Session);

impl SessionView for MapOnly {
    fn patch(&self) -> &Patch {
        self.1.patch()
    }
    fn history(&self) -> HistoryView<'_> {
        self.1.history()
    }
    fn status(&self) -> SessionStatus {
        self.1.status()
    }
    fn instance(&self) -> &InstanceState {
        self.1.instance()
    }
    fn telemetry_map(&self, epoch: u64) -> Option<&TelemetryMap> {
        (epoch == self.0.epoch).then_some(&self.0)
    }
    fn telemetry_source(&self) -> Option<Arc<dyn TelemetrySource>> {
        None
    }
}

#[test]
fn the_telemetry_reader_maps_tap_order_to_addresses() {
    let cutoff = common::addr("filter1.cutoff");
    let level = common::addr("osc1.level");
    let mut frame = TelemetryFrame {
        epoch: 3,
        voices: 2,
        params: vec![1500.0, 80.0, 0.0, 0.0],
        params_len: 2,
        modulators: vec![0.5, 0.0],
        modulators_len: 1,
        peaks: [0.25, 0.5],
        ..TelemetryFrame::default()
    };
    let tap = Arc::new(FakeTap {
        generation: AtomicU64::new(7),
        enabled: AtomicBool::new(false),
        frame: frame.clone(),
    });
    let view = MapOnly(
        TelemetryMap {
            epoch: 3,
            params: vec![cutoff.clone(), level.clone()],
            modulators: vec![common::id("lfo1")],
        },
        Session::default(),
    );
    let source: Arc<dyn TelemetrySource> = tap.clone();
    source.set_enabled(true);
    assert!(tap.enabled.load(Ordering::Relaxed));
    let mut reader = TelemetryReader::new(source);
    assert!(reader.refresh(&view), "generation 7 is new");
    assert_eq!(reader.live_value(&cutoff), Some(1500.0));
    assert_eq!(reader.live_value(&level), Some(80.0));
    assert_eq!(reader.live_value(&common::addr("master.level")), None);
    assert_eq!(reader.modulator(&common::id("lfo1")), Some(0.5));
    assert_eq!(reader.voices(), 2);
    assert_eq!(reader.peaks(), [0.25, 0.5]);
    assert!(!reader.refresh(&view), "nothing new");
    assert!(reader.sleep());
    let mut scope = [1.0; 8];
    assert_eq!(reader.scope(&mut scope), 8);

    // A plan the session has no map for: values are hidden, never shown on the wrong knob.
    frame.epoch = 4;
    let tap4 = Arc::new(FakeTap {
        generation: AtomicU64::new(8),
        enabled: AtomicBool::new(false),
        frame,
    });
    let mut reader = TelemetryReader::new(tap4);
    reader.refresh(&view);
    assert_eq!(reader.live_value(&cutoff), None);
    assert_eq!(reader.modulator(&common::id("lfo1")), None);
    assert_eq!(reader.frame().epoch, 4);
}

#[test]
fn a_real_plan_and_tap_fit_the_reader() {
    use enth_engine::{
        Engine, Event, EventKind, LinkCapacity, NoteKey, PrepareCfg, Tap, TapCapacity, compile,
    };

    let patch = common::custom_patch();
    let (plan, map) = compile(&patch, &PrepareCfg::new(48_000.0)).unwrap();
    let (mut engine, _link) =
        Engine::with_link(plan, enth_session::RENDER_SEED, LinkCapacity::default());
    let telemetry = TelemetryMap::from_plan(engine.epoch(), engine.plan(), &map);
    let cutoff = common::addr("filter1.cutoff");
    assert!(telemetry.param_index(&cutoff).is_some(), "{telemetry:?}");
    assert!(telemetry.modulator_index(&common::id("lfo1")).is_some());
    assert!(telemetry.modulator_index(&common::id("env1")).is_some());

    let tap = Tap::new(TapCapacity::default());
    engine.attach_tap(tap.clone());
    let source: Arc<dyn TelemetrySource> = tap;
    source.set_enabled(true);

    let (mut left, mut right) = (vec![0.0; 256], vec![0.0; 256]);
    let on = [Event {
        offset: 0,
        kind: EventKind::NoteOn {
            key: NoteKey::new(60, 0),
            velocity: 1.0,
        },
    }];
    engine.process(&mut left, &mut right, &on);
    engine.process(&mut left, &mut right, &[]);

    let view = MapOnly(telemetry, Session::new(patch));
    let mut reader = TelemetryReader::new(source);
    assert!(reader.refresh(&view));
    assert_eq!(reader.voices(), 1);
    let live = reader
        .live_value(&cutoff)
        .expect("the routed cutoff is published");
    assert!((20.0..=20_000.0).contains(&live), "{live}");
    assert!(reader.modulator(&common::id("env1")).is_some());
}
