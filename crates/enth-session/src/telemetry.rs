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
//! - [`TelemetrySource`] is the seam: the session codes against it, `enth_engine::tap::Tap` implements it once E+
//!   is merged (a few delegating lines in this file), and kittest and the dev window can fake it without an engine.
//! - [`TelemetryFrame`] mirrors E+'s `TapFrame` field for field (branch `p1b/eplus` at `a7cb53f`), so it can become
//!   a re-export after the merge without changing editor code.

use std::sync::Arc;

use enth_engine::EngineStats;
use enth_patch::{ModuleId, ParamAddr};

use crate::model::SessionView;

/// Macro sources in a frame: the eight macros, then the mod wheel (index 8).
pub const MACRO_SOURCES: usize = 9;

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

/// One copy of the tap's values. Mirrors `enth_engine::tap::TapFrame` (lane E+).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TelemetryFrame {
    /// The generation the values were read at (0: nothing published yet).
    pub generation: u64,
    /// The epoch of the plan whose order `params` and `modulators` follow.
    pub epoch: u64,
    /// Playing voices (held, releasing or fading).
    pub voices: u32,
    /// Voices with their key down.
    pub held: u32,
    /// The voice slot the per-voice values describe (the newest held voice, else the newest releasing one).
    pub display_voice: Option<u32>,
    /// The note index of that voice's note.
    pub display_note: u64,
    /// Routed parameter values in plain units, in the plan's tap order; the first `params_len` are valid.
    pub params: Vec<f32>,
    /// How many of `params` are valid.
    pub params_len: usize,
    /// Modulator outputs in the plan's tap order; the first `modulators_len` are valid.
    pub modulators: Vec<f32>,
    /// How many of `modulators` are valid.
    pub modulators_len: usize,
    /// The eight macros and the mod wheel (index 8), smoothed, 0–1.
    pub macros: [f32; MACRO_SOURCES],
    /// The last published block's peak per channel.
    pub peaks: [f32; 2],
    /// The engine's counters.
    pub stats: EngineStats,
    /// Values that did not fit the tap's capacities, summed over every publish.
    pub dropped: u64,
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
pub struct TelemetryReader {
    source: Arc<dyn TelemetrySource>,
    frame: TelemetryFrame,
    map: Option<TelemetryMap>,
    seen: u64,
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
    /// A reader over `source` (allocates its frame once).
    pub fn new(source: Arc<dyn TelemetrySource>) -> Self {
        let frame = source.frame();
        TelemetryReader {
            source,
            frame,
            map: None,
            seen: 0,
        }
    }

    /// Read the latest values; when the frame's epoch differs from the cached map's, fetch that epoch's map from the
    /// session (cloned: allocates only when the plan changed). Returns whether the generation moved since the last
    /// refresh (new data to draw).
    pub fn refresh(&mut self, session: &dyn SessionView) -> bool {
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
