//! The [`Session`] and the types the editor and the shell see: the read model ([`SessionView`]), the edit trait
//! ([`SessionEdit`]), the shell's request outbox ([`ShellRequest`]), status, configuration and errors (lane S1s).
//!
//! # Who calls what (the threading contract in brief; full text in `docs/session.md`)
//!
//! - `Session` is a plain single-threaded value: every method takes `&self` or `&mut self`, nothing locks, sleeps,
//!   spawns, reads a clock or does I/O. The plugin wraps it in **one** `parking_lot::Mutex`, shared by the host's
//!   main thread and baseview's GUI thread (different threads in baseview 0.4.1). **The audio thread never touches
//!   it**, and nothing is locked from `process()`.
//! - Host main thread: [`Session::activate`] / [`Session::deactivate`], state save and load
//!   ([`Session::save_patch`], [`Session::save_instance`], [`Session::load_patch`], [`Session::load_instance`]).
//! - GUI thread, once per editor frame, in this order: [`Session::pump`], [`Session::fold_host`], the editor's reads
//!   and edits through [`SessionEdit`], then [`Session::drain_requests`] into a local buffer, **unlock**, and only
//!   then forward [`ShellRequest::Host`] writes to nice-plug's `ParamSetter` (a host may call back into the plugin
//!   synchronously, for example to read state, which would deadlock on a held lock).
//! - The return ring is drained by the session itself at every entry that can send ([`Session::pump`] runs at the
//!   top of each of them, and again before one returns with a compile pending). At most
//!   [`MAX_PLANS_IN_FLIGHT`](enth_engine::MAX_PLANS_IN_FLIGHT) plans are ever outstanding and the ring holds more
//!   than that, so an idle session (editor closed, no host calls) never stalls the audio thread; it only holds up to
//!   two retired plans' memory until its next entry. A compile left pending is pumped by the shell's background
//!   task (nice-plug's `execute_background`, armed from `process()`; see [`crate::link`]).
//! - nice-plug's background worker may also lock the session, only to call [`Session::pump`].

use std::sync::Arc;

use enth_engine::{CompileError, LinkCapacity, TapCapacity};
use enth_patch::history::{Entry, GestureId, History};
use enth_patch::ops::TxError;
use enth_patch::{Op, Patch};

use crate::apply::Gestures;
use crate::bridge::{HostSync, HostWrite};
use crate::link::Port;
use crate::state::{InstanceState, StateError, UiState};
use crate::telemetry::{TelemetryMap, TelemetrySource};

/// The render seed every session-built engine uses: the CLI's default, never per instance and never the patch hash
/// (docs/design.md §5), so a plugin render equals `enth render` under the stated conditions.
pub const RENDER_SEED: u64 = enth_render::DEFAULT_SEED;

/// The `NoteKey::port` of the editor's audition notes. The shell filters `NOTE_END`s on this port (they are not the
/// host's notes); `-1` would be the wildcard, so it is a fixed positive port no host uses.
pub const AUDITION_PORT: i16 = 0x7F00;

/// Sizes the session works with. Every queue is bounded: nothing grows without limit, nothing blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// The engine's command and return rings ([`enth_engine::Engine::with_link`]).
    pub link: LinkCapacity,
    /// The telemetry tap's sizes (routed parameters, modulators, scope samples).
    pub tap: TapCapacity,
    /// Undo steps kept ([`History::with_cap`]).
    pub history_cap: usize,
    /// Value events held back while the command ring is full (the last per parameter is kept). When this
    /// overflows, the session drops the queue and recompiles instead: a plan carries every value.
    pub value_queue: usize,
    /// Audition events held back while the command ring is full. Note-offs are never dropped (a lost note-off is a
    /// stuck note); a note-on that does not fit is refused with [`SessionError::Busy`].
    pub audition_queue: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            link: LinkCapacity::default(),
            tap: TapCapacity::default(),
            history_cap: History::CAP,
            value_queue: 256,
            audition_queue: 64,
        }
    }
}

/// How the host activated the plugin ([`Session::activate`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Activation {
    /// Frames per second; plans are compiled for it.
    pub sample_rate: f32,
    /// The host renders offline (nice-plug `ProcessMode::Offline`): audition notes are refused, so an offline
    /// bounce never contains an editor note.
    pub offline: bool,
}

/// The one owner of the live patch (docs/architecture.md, L6). See the [module docs](self) for the threading
/// contract and `docs/session.md` for the invariants.
///
/// Invariants (each has, or gets, a test in the owning lane):
/// - **I1, one writer:** every patch change is an [`Op`] applied here. Editor edits, undo and redo are recorded in
///   the history; host folds ([`Session::fold_host`]) and loads are not.
/// - **I2, delivery follows the op:** an applied transaction reaches the engine as [`crate::apply::delivery`]
///   says, after the transaction succeeded; a rejected one delivers nothing and changes nothing.
/// - **I3, lazy plans:** at most one compile is pending; it is compiled from the patch at the moment it can be sent,
///   so a plan always reflects the patch as it was when sent.
/// - **I4, epochs:** value events carry the epoch of the latest *sent* plan and use that plan's `PlanMap`.
/// - **I5, never blocks, never drops:** a full command ring queues values (bounded); an overflowing queue becomes a
///   recompile.
/// - **I6, the patch wins:** host values are folded only when they changed since the session last saw them, and
///   only when they truly differ from the patch ([`crate::bridge::fold_value`]).
pub struct Session {
    pub(crate) patch: Patch,
    pub(crate) history: History,
    pub(crate) instance: InstanceState,
    pub(crate) config: SessionConfig,
    /// The engine side while active (lane SK fills it in).
    pub(crate) port: Option<Port>,
    /// The open gesture and the host gestures it opened.
    pub(crate) gestures: Gestures,
    /// What the session last saw of the host's values.
    pub(crate) host: HostSync,
    /// The outbox the shell drains.
    pub(crate) requests: Vec<ShellRequest>,
    /// The last error a delivery or a load hit (shown in the status line), cleared by the next successful delivery.
    pub(crate) last_error: Option<SessionError>,
    /// The `patch` field text that did not load, while the session holds the fallback init patch in its place.
    /// [`Session::save_patch`] writes it back verbatim, so opening a project in an older build (`schema_too_new`) or
    /// hitting a damaged field never overwrites the stored patch on the next host save. Cleared by a successful
    /// load and by the first recorded change (an edit, undo or redo: SK's `edit` and S2s's undo and redo clear it), after which
    /// the user's own patch is saved.
    pub(crate) unloaded_patch: Option<String>,
    /// The `enth` field text that was refused as too new for this build (`schema_too_new`), while the session holds
    /// the default instance state in its place. [`Session::save_instance`] writes it back verbatim, so a project saved
    /// by a newer enth keeps its newer preferences through an older build's save. Cleared by a successful
    /// [`Session::load_instance`] and by the first change of the monitor bypass or the editor preferences (lane S2s).
    pub(crate) unloaded_instance: Option<String>,
    /// The telemetry tap's reader side, while active.
    pub(crate) telemetry: Option<Arc<dyn TelemetrySource>>,
    /// Address maps for the epochs the tap may still be publishing (newest last).
    pub(crate) telemetry_maps: Vec<TelemetryMap>,
    /// The Recreate round's state that is not saved (lane SG; the saved part is `instance.game`): see
    /// [`crate::game`].
    pub(crate) game: crate::game::GameLive,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("status", &self.status())
            .field("requests", &self.requests.len())
            .finish_non_exhaustive()
    }
}

impl Default for Session {
    /// A session on [`Patch::init`].
    fn default() -> Self {
        Session::new(Patch::init())
    }
}

impl Session {
    /// A detached session owning `patch`, with an empty history (the patch counts as saved). Edits work while
    /// detached; [`Session::activate`] compiles the patch as it is then.
    pub fn new(patch: Patch) -> Self {
        Session::with_config(patch, SessionConfig::default())
    }

    /// [`Session::new`] with explicit sizes. The return ring is raised to at least
    /// [`MAX_PLANS_IN_FLIGHT`](enth_engine::MAX_PLANS_IN_FLIGHT) slots, so the audio thread can always hand every
    /// outstanding plan back without anyone draining it (the threading contract relies on that).
    pub fn with_config(patch: Patch, mut config: SessionConfig) -> Self {
        config.link.returns = config.link.returns.max(enth_engine::MAX_PLANS_IN_FLIGHT);
        Session {
            patch,
            history: History::with_cap(config.history_cap),
            instance: InstanceState::default(),
            config,
            port: None,
            gestures: Gestures::default(),
            host: HostSync::default(),
            requests: Vec::new(),
            last_error: None,
            unloaded_patch: None,
            unloaded_instance: None,
            telemetry: None,
            telemetry_maps: Vec::new(),
            game: crate::game::GameLive::default(),
        }
    }

    /// The live patch. Read it every editor frame; never cache it across frames.
    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    /// The undo and redo stacks, read-only.
    pub fn history(&self) -> HistoryView<'_> {
        HistoryView(&self.history)
    }

    /// The per-instance state outside the patch (monitor bypass, editor preferences).
    pub fn instance(&self) -> &InstanceState {
        &self.instance
    }

    /// The sizes this session was built with.
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// What the session is doing, for the status line and tests.
    pub fn status(&self) -> SessionStatus {
        let mut s = SessionStatus::default();
        if let Some(port) = &self.port {
            s.active = Some(port.activation);
            s.epoch = port.link.epoch();
            s.plans_in_flight = port.link.in_flight();
            s.compile_pending = port.compile_pending;
            s.queued_events = port.queue.len();
        }
        s.requests = self.requests.len();
        s.gesture_open = self.gestures.open.is_some();
        s.last_error.clone_from(&self.last_error);
        s.patch_unloaded = self.unloaded_patch.is_some();
        s.instance_unloaded = self.unloaded_instance.is_some();
        s
    }

    /// Take everything the shell must do on the session's behalf, oldest first. The shell drains this after each
    /// editor frame and after each host call into the session, **after releasing the session lock**, then acts:
    /// see [`ShellRequest`]. Requests are bounded by one frame's edits; the outbox is never dropped.
    pub fn drain_requests(&mut self) -> std::vec::Drain<'_, ShellRequest> {
        self.requests.drain(..)
    }

    /// Replace the editor preferences (persisted in the `enth` field, not in the patch; not an undo step).
    pub fn set_ui(&mut self, ui: UiState) {
        if self.instance.ui != ui {
            self.unloaded_instance = None; // the user's own preferences now (a newer build's text is replaced)
        }
        self.instance.ui = ui;
    }

    /// Turn the monitor FX bypass on or off (ADR-0006: per-instance monitoring state outside the patch, so renders
    /// and scores never see it). Pushes [`ShellRequest::MonitorBypass`] when it changes; the shell stores it into
    /// the atomic `process()` reads. Not an undo step.
    pub fn set_monitor_bypass(&mut self, on: bool) {
        if self.instance.monitor.fx_bypass != on {
            self.unloaded_instance = None;
            self.instance.monitor.fx_bypass = on;
            self.requests.push(ShellRequest::MonitorBypass(on));
        }
    }

    /// The address map of the telemetry tap's values for `epoch`, if the session still knows that plan. The tap
    /// may publish an older epoch for a few blocks after a plan is sent; a reader hides values with no map.
    pub fn telemetry_map(&self, epoch: u64) -> Option<&TelemetryMap> {
        self.telemetry_maps.iter().rev().find(|m| m.epoch == epoch)
    }

    /// The telemetry tap's reader side, while active. Clone it into a
    /// [`TelemetryReader`](crate::telemetry::TelemetryReader) once per editor open.
    pub fn telemetry_source(&self) -> Option<Arc<dyn TelemetrySource>> {
        self.telemetry.clone()
    }
}

/// The history, read-only: what the header's Undo and Redo buttons and their tooltips need.
#[derive(Clone, Copy)]
pub struct HistoryView<'a>(pub(crate) &'a History);

impl<'a> HistoryView<'a> {
    /// A view of any history (for fakes that implement [`SessionView`]).
    pub fn new(history: &'a History) -> Self {
        HistoryView(history)
    }

    /// Whether Undo has something to do.
    pub fn can_undo(&self) -> bool {
        self.0.can_undo()
    }

    /// Whether Redo has something to do.
    pub fn can_redo(&self) -> bool {
        self.0.can_redo()
    }

    /// The step Undo would revert (its `forward` ops name what it did: "Undo Filter 1 › Cutoff").
    pub fn next_undo(&self) -> Option<&'a Entry> {
        self.0.undo_stack().last()
    }

    /// The step Redo would re-apply.
    pub fn next_redo(&self) -> Option<&'a Entry> {
        self.0.redo_stack().last()
    }

    /// Whether the patch differs from the last saved state (an editor preset save, or a load).
    pub fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }
}

impl std::fmt::Debug for HistoryView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryView")
            .field("undo", &self.0.undo_stack().len())
            .field("redo", &self.0.redo_stack().len())
            .field("dirty", &self.0.is_dirty())
            .finish()
    }
}

/// A snapshot of what the session is doing. Build one with `Default` and set fields (it is `non_exhaustive`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a status snapshot: independent facts the editor shows, not a state machine; the F1b API keeps its fields"
)]
pub struct SessionStatus {
    /// `Some` while the host has the plugin active (an engine exists).
    pub active: Option<Activation>,
    /// The epoch of the latest plan sent (0 before any).
    pub epoch: u64,
    /// Plans sent whose predecessors have not come back yet (at most `MAX_PLANS_IN_FLIGHT`).
    pub plans_in_flight: usize,
    /// An edit needs a new plan that could not be sent yet (two in flight, or a full ring); the next pump sends it.
    pub compile_pending: bool,
    /// Events held back because the command ring was full.
    pub queued_events: usize,
    /// Requests waiting for the shell.
    pub requests: usize,
    /// Whether a gesture (a drag) is open.
    pub gesture_open: bool,
    /// The last delivery or load error, until the next successful delivery.
    pub last_error: Option<SessionError>,
    /// The stored patch did not load and the session plays the init patch instead; until the first edit, a save
    /// writes the stored text back unchanged ([`crate::state::LoadReport::fell_back`]).
    pub patch_unloaded: bool,
    /// The stored `enth` field was written by a newer build and the session uses the default preferences instead;
    /// until the first change of the monitor bypass or the editor preferences, a save writes the stored text back
    /// unchanged ([`crate::state::LoadReport`]).
    pub instance_unloaded: bool,
}

/// What an applied edit did (for tests and the status line). Build one with `Default` (it is `non_exhaustive`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    /// Value, depth and macro events handed to the link or queued for the next pump.
    pub events: u32,
    /// The edit needs a new plan: sent now, or pending until the link has room.
    pub recompile: bool,
    /// [`ShellRequest::Host`] writes pushed for the host (slotted addresses and macros).
    pub host_writes: u32,
    /// Host parameter names changed ([`ShellRequest::RenameParams`] pushed).
    pub renamed: bool,
}

/// Something the shell must do for the session, because the session knows no host and does no I/O. Drained with
/// [`Session::drain_requests`]; act on them **after unlocking** the session.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ShellRequest {
    /// Tell the host a parameter moved, as one step of a gesture: nice-plug's `ParamSetter::begin_set_parameter`,
    /// `set_parameter_normalized` and `end_set_parameter`, so automation records in write and touch modes. Only
    /// edits made through [`SessionEdit`] (the editor, on the GUI thread) produce these. Begins and ends are
    /// balanced per gesture.
    Host(HostWrite),
    /// Host parameter names may have changed (a slot assignment, a module label, a macro name, a load): set the
    /// display names on the main or GUI thread (with the fork's rename patch), refresh the slot value-text table,
    /// and ask the host to rescan parameter info from `process()` through the shell's own flag.
    RenameParams,
    /// The monitor FX bypass changed: store it into the atomic `process()` reads.
    MonitorBypass(bool),
    /// The patch changed through the editor (edits, undo, redo), which the host cannot see. Coalesce per frame; the
    /// shell marks the host project dirty through the fork's `HostRequests::mark_dirty()` (patch 6).
    PatchEdited,
}

/// The editor's read model: what `enth-ui` reads every frame. [`Session`] implements it; a kittest fake may too.
///
/// The view-model is rebuilt from [`SessionView::patch`] every frame (about 0.23 M instructions for the G1 patch,
/// research report 02), so nothing here is cached.
pub trait SessionView {
    /// The live patch.
    fn patch(&self) -> &Patch;
    /// The undo and redo stacks.
    fn history(&self) -> HistoryView<'_>;
    /// What the session is doing.
    fn status(&self) -> SessionStatus;
    /// Monitor bypass and editor preferences.
    fn instance(&self) -> &InstanceState;
    /// The address map for the telemetry values of plan `epoch`.
    fn telemetry_map(&self, epoch: u64) -> Option<&TelemetryMap>;
    /// The telemetry tap's reader side, while active.
    fn telemetry_source(&self) -> Option<Arc<dyn TelemetrySource>>;
}

/// The editor's edit trait: widgets emit intents, and the editor turns them into these calls **at the end of the
/// frame** (no widget borrows the session). [`Session`] implements it.
///
/// Rules the editor keeps (research report 02, D5):
/// - a drag is one gesture: [`SessionEdit::begin_gesture`] at drag start, the same id on every edit, and
///   [`SessionEdit::end_gesture`] when the pointer is released **anywhere** (a widget that vanished never reports
///   its drag end), and again when the editor is rebuilt (every `build()` is a fresh open);
/// - a structural control commits at drag end, not per step;
/// - single clicks, text entry and menu items are one edit with no gesture.
pub trait SessionEdit: SessionView {
    /// Open a gesture (ends any open one first). Host gestures for slotted addresses and macros open lazily, at the
    /// first edit that touches them.
    fn begin_gesture(&mut self) -> GestureId;
    /// End the open gesture, if any: one undo step (none if the patch is back where it started), and an `End` for
    /// every host gesture it opened. Idempotent.
    fn end_gesture(&mut self);
    /// Apply `ops` as one transaction, record it in the history (merged into `gesture`'s step when given), and
    /// deliver it. On error nothing changes and nothing is delivered. A `gesture` that is not the open one (a drag
    /// that outlived a load, which clears the history and restarts its gesture ids) counts as no gesture.
    fn edit(&mut self, ops: Vec<Op>, gesture: Option<GestureId>) -> Result<Applied, SessionError>;
    /// Undo one step and deliver it. `Ok(false)` if there was nothing to undo. Ends any open gesture.
    fn undo(&mut self) -> Result<bool, SessionError>;
    /// Redo one step and deliver it. `Ok(false)` if there was nothing to redo. Ends any open gesture.
    fn redo(&mut self) -> Result<bool, SessionError>;
    /// Replace the editor preferences.
    fn set_ui(&mut self, ui: UiState);
    /// Turn the monitor FX bypass on or off.
    fn set_monitor_bypass(&mut self, on: bool);
    /// Play an audition note (the editor's keyboard): key 0–127, velocity 0–1. Returns the note's key for the
    /// note-off. Refused while inactive, offline, or when the queue is full.
    fn audition_on(&mut self, key: u8, velocity: f32)
    -> Result<enth_engine::NoteKey, SessionError>;
    /// Release an audition note. Never dropped.
    fn audition_off(&mut self, note: enth_engine::NoteKey) -> Result<(), SessionError>;
    /// Release every audition note: the editor calls it when it closes; the session calls it on deactivate and load.
    fn audition_all_off(&mut self) -> Result<(), SessionError>;
}

impl SessionView for Session {
    fn patch(&self) -> &Patch {
        Session::patch(self)
    }

    fn history(&self) -> HistoryView<'_> {
        Session::history(self)
    }

    fn status(&self) -> SessionStatus {
        Session::status(self)
    }

    fn instance(&self) -> &InstanceState {
        Session::instance(self)
    }

    fn telemetry_map(&self, epoch: u64) -> Option<&TelemetryMap> {
        Session::telemetry_map(self, epoch)
    }

    fn telemetry_source(&self) -> Option<Arc<dyn TelemetrySource>> {
        Session::telemetry_source(self)
    }
}

impl SessionEdit for Session {
    fn begin_gesture(&mut self) -> GestureId {
        Session::begin_gesture(self)
    }

    fn end_gesture(&mut self) {
        Session::end_gesture(self);
    }

    fn edit(&mut self, ops: Vec<Op>, gesture: Option<GestureId>) -> Result<Applied, SessionError> {
        Session::edit(self, ops, gesture)
    }

    fn undo(&mut self) -> Result<bool, SessionError> {
        Session::undo(self)
    }

    fn redo(&mut self) -> Result<bool, SessionError> {
        Session::redo(self)
    }

    fn set_ui(&mut self, ui: UiState) {
        Session::set_ui(self, ui);
    }

    fn set_monitor_bypass(&mut self, on: bool) {
        Session::set_monitor_bypass(self, on);
    }

    fn audition_on(
        &mut self,
        key: u8,
        velocity: f32,
    ) -> Result<enth_engine::NoteKey, SessionError> {
        Session::audition_on(self, key, velocity)
    }

    fn audition_off(&mut self, note: enth_engine::NoteKey) -> Result<(), SessionError> {
        Session::audition_off(self, note)
    }

    fn audition_all_off(&mut self) -> Result<(), SessionError> {
        Session::audition_all_off(self)
    }
}

/// Why a session call failed. Every variant has a stable snake-case [`code`](SessionError::code) and a one-line
/// [`hint`](SessionError::hint), like the CLI's errors.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SessionError {
    /// The edit was rejected; the patch, the history and the engine are unchanged.
    Edit(TxError),
    /// The patch did not compile for the engine. A valid patch always compiles, so this is a bug.
    Compile(CompileError),
    /// A saved state did not load (see [`crate::state::LoadReport`] for what the session did instead).
    State(StateError),
    /// The call needs an active engine ([`Session::activate`] first).
    Inactive,
    /// The command ring and the bounded queue behind it are full: try again next frame. Only audition note-ons get
    /// this; edits never fail for lack of room (they queue or recompile).
    Busy,
    /// Refused while the host renders offline (audition notes).
    Offline,
    /// Not implemented yet: a P1b stub that a later lane fills in.
    Unsupported {
        /// What was called.
        what: &'static str,
        /// The lane that implements it.
        lane: &'static str,
    },
}

impl SessionError {
    /// A stable snake-case code. An edit or state error passes its inner code through (`out_of_range`,
    /// `schema_too_new`), so the editor and the CLI speak one vocabulary.
    pub fn code(&self) -> &'static str {
        match self {
            SessionError::Edit(e) => e.error.code(),
            SessionError::Compile(e) => e.code(),
            SessionError::State(e) => e.code(),
            SessionError::Inactive => "inactive",
            SessionError::Busy => "busy",
            SessionError::Offline => "offline",
            SessionError::Unsupported { .. } => "unsupported",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            SessionError::Edit(e) => e.error.hint(),
            SessionError::Compile(_) => {
                "a valid patch always compiles: report this with the patch file".to_owned()
            }
            SessionError::State(e) => e.hint(),
            SessionError::Inactive => "the host has not activated the plugin yet".to_owned(),
            SessionError::Busy => "the audio thread is behind: try again".to_owned(),
            SessionError::Offline => "audition is off while the host renders offline".to_owned(),
            SessionError::Unsupported { lane, .. } => format!("not built yet (lane {lane})"),
        }
    }

    #[allow(
        dead_code,
        reason = "no stub is left once lane SK lands; the next one to appear uses it"
    )]
    pub(crate) const fn unsupported(what: &'static str, lane: &'static str) -> Self {
        SessionError::Unsupported { what, lane }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Edit(e) => write!(f, "edit rejected: {e}"),
            SessionError::Compile(e) => write!(f, "the patch does not compile: {e}"),
            SessionError::State(e) => write!(f, "state not loaded: {e}"),
            SessionError::Inactive => f.write_str("the engine is not active"),
            SessionError::Busy => f.write_str("the command ring is full"),
            SessionError::Offline => f.write_str("refused while rendering offline"),
            SessionError::Unsupported { what, lane } => {
                write!(f, "{what} is not implemented yet (lane {lane})")
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl From<TxError> for SessionError {
    fn from(e: TxError) -> Self {
        SessionError::Edit(e)
    }
}

impl From<CompileError> for SessionError {
    fn from(e: CompileError) -> Self {
        SessionError::Compile(e)
    }
}

impl From<StateError> for SessionError {
    fn from(e: StateError) -> Self {
        SessionError::State(e)
    }
}
