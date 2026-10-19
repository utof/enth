//! Reader types for the engine's read-only telemetry tap: modulated parameter values for the knob rings, modulator
//! outputs, voice counts, peaks, the scope, and the sleep/wake handshake for repaint gating (lane S1s for the
//! types, ED2 for their use).
//!
//! ```text
//! audio thread: Engine::process ── publish ──▶ tap (atomics) ◀── TelemetrySource::read ── TelemetryReader (editor)
//!                                                                 TelemetryMap (epoch → addresses) ◀── Session
//! ```
//!
//! - The tap itself is lane E+'s (`enth_engine::tap::Tap`): atomics written at the end of `process()` while
//!   enabled, so renders never change and nothing blocks. The editor wants the latest value, not every value.
//! - Values arrive in the order of the plan that published them (`Plan::tap_params`, `Plan::tap_modulators`). The
//!   session keeps a [`TelemetryMap`] per recently sent plan, mapping that order to addresses; a
//!   [`TelemetryReader`] hides values whose epoch has no map (one stale frame is hidden, never shown on the wrong
//!   knob).
//! - [`TelemetrySource`] is the seam the editor codes against: [`enth_engine::Tap`] implements it (below), and
//!   kittest and the dev window can fake it without an engine (a real `Tap`'s values can only be written by an
//!   engine). [`TelemetryFrame`] is the engine's [`TapFrame`](enth_engine::TapFrame).
//! - Who does what: the session creates the tap at activate and attaches it to the engine on the main thread,
//!   before the engine goes to the audio thread (lane SK); the shell enables it while the editor is open and shown
//!   ([`TelemetrySource::set_enabled`]) and, on the audio thread after `process()`, takes the wake flag through
//!   `Engine::tap()` and `Tap::take_wake` to request a repaint; the editor reads through a [`TelemetryReader`].

use std::sync::Arc;

use enth_engine::{Plan, PlanMap, Tap, TapFrame};
use enth_patch::{ModuleId, ParamAddr};

use crate::model::SessionView;

/// One copy of the tap's values: the engine's [`TapFrame`] (epoch, voices, display voice, routed values and
/// modulator outputs in the plan's tap order with their valid lengths, macros, peaks, stats, dropped count).
pub type TelemetryFrame = TapFrame;

/// The reader side of a telemetry tap. Every method is lock-free and allocation-free except
/// [`TelemetrySource::frame`], and none ever waits for the audio thread.
pub trait TelemetrySource: Send + Sync {
    /// A frame sized for this source (allocates once, at editor open).
    fn frame(&self) -> TelemetryFrame;
    /// Copy the latest published values into `out` (sized by [`TelemetrySource::frame`]; never allocates).
    fn read(&self, out: &mut TelemetryFrame);
    /// Copy the newest scope samples into `out`, oldest first; returns how many samples were ever written, so a
    /// reader can tell new data from old.
    fn scope(&self, out: &mut [f32]) -> u64;
    /// Bumped by every publish that brought new data.
    fn generation(&self) -> u64;
    /// The editor drew generation `seen` and will sleep: returns `false` if newer data is already there (draw
    /// again); otherwise the next publish with new data sets the wake flag the shell turns into a repaint.
    fn sleep(&self, seen: u64) -> bool;
    /// Publish only while enabled: the shell enables it while the editor is open and shown.
    fn set_enabled(&self, on: bool);
}

/// Which address each published value belongs to, for one plan. Built by the session when it sends a plan (from
/// `Plan::tap_params` through `PlanMap::addr`, and `Plan::tap_modulators`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryMap {
    /// The plan's epoch.
    pub epoch: u64,
    /// The address of each routed value, in tap order.
    pub params: Vec<ParamAddr>,
    /// The module of each modulator output, in tap order.
    pub modulators: Vec<ModuleId>,
}

impl TelemetryMap {
    /// The map for a plan the session is about to send (or has sent) as `epoch`: `Plan::tap_params` through
    /// `PlanMap::addr`, and `Plan::tap_modulators`. Index `i` must stay tap index `i`, so a routed parameter with no
    /// address (never, for a plan and the map compiled with it) ends the list there: it and the values after it are
    /// hidden rather than shown on the wrong knob.
    pub fn from_plan(epoch: u64, plan: &Plan, map: &PlanMap) -> TelemetryMap {
        TelemetryMap {
            epoch,
            params: plan
                .tap_params()
                .iter()
                .map_while(|&id| map.addr(id).cloned())
                .collect(),
            modulators: plan.tap_modulators().into_iter().cloned().collect(),
        }
    }

    /// The tap index of a routed parameter.
    pub fn param_index(&self, addr: &ParamAddr) -> Option<usize> {
        self.params.iter().position(|a| a == addr)
    }

    /// The tap index of a modulator.
    pub fn modulator_index(&self, id: &ModuleId) -> Option<usize> {
        self.modulators.iter().position(|m| m == id)
    }
}

/// The editor's telemetry reader: one per editor open, refreshed once per frame under the session lock.
///
/// It follows the session's source: when the host re-activates the plugin while the editor is open (a sample-rate
/// change, REAPER's restart at the start of an offline render), the session builds a new engine and a new tap, and
/// [`TelemetryReader::refresh`] switches to it, drops the old plan's map (a new engine counts epochs from 0 again,
/// so an epoch alone cannot tell two engines' plans apart) and re-applies [`TelemetryReader::set_enabled`].
pub struct TelemetryReader {
    source: Arc<dyn TelemetrySource>,
    frame: TelemetryFrame,
    map: Option<TelemetryMap>,
    seen: u64,
    enabled: bool,
}

impl std::fmt::Debug for TelemetryReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryReader")
            .field("frame", &self.frame)
            .field("map", &self.map)
            .finish_non_exhaustive()
    }
}

impl TelemetryReader {
    /// A reader over `source` (allocates its frame once). It does not enable the source: call
    /// [`TelemetryReader::set_enabled`] while the editor is shown.
    pub fn new(source: Arc<dyn TelemetrySource>) -> Self {
        let frame = source.frame();
        TelemetryReader {
            source,
            frame,
            map: None,
            seen: 0,
            enabled: false,
        }
    }

    /// Publish only while the editor is open and shown: forwarded to the source, and re-applied to a new source
    /// after a re-activation. Call it with `false` when the editor closes or hides.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.source.set_enabled(on);
    }

    /// Read the latest values; when the frame's epoch differs from the cached map's, fetch that epoch's map from the
    /// session (cloned: allocates only when the plan changed). Switches to the session's source first if it has a
    /// new one (allocating a new frame once). Returns whether the generation moved since the last refresh (new data
    /// to draw).
    pub fn refresh(&mut self, session: &dyn SessionView) -> bool {
        if let Some(source) = session.telemetry_source()
            && !same_source(&source, &self.source)
        {
            self.frame = source.frame();
            self.map = None;
            self.seen = 0;
            if self.enabled {
                self.source.set_enabled(false);
                source.set_enabled(true);
            }
            self.source = source;
        }
        self.source.read(&mut self.frame);
        if self.map.as_ref().map(|m| m.epoch) != Some(self.frame.epoch) {
            self.map = session.telemetry_map(self.frame.epoch).cloned();
        }
        let moved = self.frame.generation != self.seen;
        self.seen = self.frame.generation;
        moved
    }

    /// The last frame read.
    pub fn frame(&self) -> &TelemetryFrame {
        &self.frame
    }

    /// The live (modulated) value of a routed parameter, in plain units, for the display voice. `None` if the
    /// parameter is not routed in the plan that published, or that plan has no map.
    pub fn live_value(&self, addr: &ParamAddr) -> Option<f32> {
        let i = self.map.as_ref()?.param_index(addr)?;
        (i < self.frame.params_len)
            .then(|| self.frame.params.get(i).copied())
            .flatten()
    }

    /// A modulator's output for the display voice (voice modulators) or as is (global ones).
    pub fn modulator(&self, id: &ModuleId) -> Option<f32> {
        let i = self.map.as_ref()?.modulator_index(id)?;
        (i < self.frame.modulators_len)
            .then(|| self.frame.modulators.get(i).copied())
            .flatten()
    }

    /// Playing voices.
    pub fn voices(&self) -> u32 {
        self.frame.voices
    }

    /// The last block's peak per channel.
    pub fn peaks(&self) -> [f32; 2] {
        self.frame.peaks
    }

    /// The newest scope samples, oldest first ([`TelemetrySource::scope`]).
    pub fn scope(&self, out: &mut [f32]) -> u64 {
        self.source.scope(out)
    }

    /// Nothing visible is changing: sleep on the generation last read. `false` means new data arrived meanwhile.
    pub fn sleep(&self) -> bool {
        self.source.sleep(self.seen)
    }
}

/// Whether two sources are the same object (data pointers only: the vtable half of a `dyn` pointer may differ
/// between codegen units for one object).
fn same_source(a: &Arc<dyn TelemetrySource>, b: &Arc<dyn TelemetrySource>) -> bool {
    std::ptr::eq(Arc::as_ptr(a).cast::<()>(), Arc::as_ptr(b).cast::<()>())
}

impl TelemetrySource for Tap {
    fn frame(&self) -> TelemetryFrame {
        Tap::frame(self)
    }

    fn read(&self, out: &mut TelemetryFrame) {
        Tap::read(self, out);
    }

    fn scope(&self, out: &mut [f32]) -> u64 {
        Tap::scope(self, out)
    }

    fn generation(&self) -> u64 {
        Tap::generation(self)
    }

    fn sleep(&self, seen: u64) -> bool {
        Tap::sleep(self, seen)
    }

    fn set_enabled(&self, on: bool) {
        Tap::set_enabled(self, on);
    }
}
