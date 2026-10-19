//! Live feedback (lane ED2): the engine's telemetry in the editor, and when the editor draws its next frame.
//!
//! **Telemetry.** One [`TelemetryReader`] per open of the window ([`Live::open`], from the shell's `build()`), enabled
//! while the window is open and disabled when it closes ([`Live::close`]); while the session is detached, the reader
//! is made when the session first has a source. Every frame [`Live::refresh`] copies the tap's latest values (it never
//! waits for the audio thread: the tap is a seqlock with a bounded retry). The editor reads them through [`Live`]:
//! knob rings (`ParamView::live`), the tiles' levels (`SourceView::level`), the scope, the spectrum and the meters.
//! Nothing published yet (generation 0) reads as no value, never as zeros.
//!
//! **Repaint gating** ([`Live::schedule`], at the end of every frame). egui draws a frame on input; the editor asks
//! for one more only while visible live data changes:
//!
//! - the frame hashes every live value it drew **where it can be seen** (a control or tile scrolled out of its region
//!   does not count; the window being open is the shell's part) into a fingerprint, quantised to well under a pixel;
//! - the fingerprint changed, or the tap published since the last frame: the next frame comes after
//!   `motion.animated_interval()` (30 Hz), never sooner;
//! - nothing changed: the editor sleeps on the tap ([`TelemetryReader::sleep`]) and asks for nothing, so an idle
//!   editor draws **0 frames**. The next publish with new data sets the tap's wake flag, which the shell's `process()`
//!   turns into a repaint (lock-free); input wakes it as always.
//!
//! The engine's quiet rule (docs/dsp/engine.md) keeps the tap from publishing in silence unless a shown value changed,
//! so silence is idle too, unless something really moves (a global LFO).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use egui::{Context, Id};
use enth_patch::{ModuleId, ParamAddr};
use enth_session::{SessionView, TelemetryReader};

/// Index of the mod wheel among the tap's macro values.
const MOD_WHEEL: usize = 8;

/// How finely a live value counts as changed: 1/512 of a control's range, about a tenth of a pixel on a knob's ring.
const QUANTUM: u16 = 512;

/// The editor's live state: the telemetry reader and what the last frame drew.
#[derive(Default)]
pub struct Live {
    reader: Option<TelemetryReader>,
    /// The window is open (between the shell's `build()` and its close).
    open: bool,
    /// The tap published since the frame before.
    moved: bool,
    /// The fingerprint of the visible live values the last frame drew.
    last: u64,
    /// The newest scope samples, oldest first (allocated once per open).
    scope: Vec<f32>,
    /// How many scope samples the tap had written at the last read.
    scope_written: u64,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("open", &self.open)
            .field("reader", &self.reader.is_some())
            .field("moved", &self.moved)
            .finish_non_exhaustive()
    }
}

impl Live {
    /// The window opened (the shell's `build()`): a fresh reader over the session's tap, enabled.
    pub fn open(&mut self, session: &dyn SessionView, scope_samples: usize) {
        self.close();
        self.open = true;
        self.scope = vec![0.0; scope_samples];
        self.scope_written = 0;
        self.last = 0;
        self.attach(session);
    }

    /// The window closed: the tap stops publishing.
    pub fn close(&mut self) {
        if let Some(reader) = self.reader.as_mut() {
            reader.set_enabled(false);
        }
        self.reader = None;
        self.open = false;
        self.moved = false;
    }

    /// Whether the window is open (live feedback runs).
    pub fn is_open(&self) -> bool {
        self.open
    }

    fn attach(&mut self, session: &dyn SessionView) {
        if self.reader.is_none()
            && let Some(source) = session.telemetry_source()
        {
            let mut reader = TelemetryReader::new(source);
            reader.set_enabled(true);
            self.reader = Some(reader);
        }
    }

    /// Read the tap's latest values (once per frame, under the shell's lock; never waits for the audio thread).
    pub fn refresh(&mut self, session: &dyn SessionView) {
        if !self.open {
            return;
        }
        self.attach(session);
        let Some(reader) = self.reader.as_mut() else {
            self.moved = false;
            return;
        };
        self.moved = reader.refresh(session);
        if self.moved && !self.scope.is_empty() {
            self.scope_written = reader.scope(&mut self.scope);
        }
    }

    /// The reader, when something was published.
    fn published(&self) -> Option<&TelemetryReader> {
        self.reader.as_ref().filter(|r| r.frame().generation > 0)
    }

    /// The modulated value of a routed parameter now (plain units, the display voice's), if the tap has it.
    pub fn value(&self, addr: &ParamAddr) -> Option<f32> {
        self.published()?.live_value(addr)
    }

    /// A modulator's output now (the display voice's for a voice modulator).
    pub fn modulator(&self, id: &ModuleId) -> Option<f32> {
        self.published()?.modulator(id)
    }

    /// The mod wheel's smoothed value, 0–1.
    pub fn wheel(&self) -> Option<f32> {
        self.published().map(|r| r.frame().macros[MOD_WHEEL])
    }

    /// The last published block's peak per channel (linear), if anything was published.
    pub fn peaks(&self) -> Option<[f32; 2]> {
        self.published().map(TelemetryReader::peaks)
    }

    /// Playing voices.
    pub fn voices(&self) -> Option<u32> {
        self.published().map(TelemetryReader::voices)
    }

    /// The newest scope samples (oldest first) and how many the tap has written in all, if anything was published.
    pub fn scope(&self) -> Option<(&[f32], u64)> {
        self.published()
            .map(|_| (self.scope.as_slice(), self.scope_written))
    }

    /// The tap published since the frame before.
    pub fn moved(&self) -> bool {
        self.moved
    }

    /// End of a frame: ask egui for the next one only while visible live data changes (module docs). Returns the
    /// delay asked for, `None` for none (the editor sleeps on the tap).
    pub fn schedule(
        &mut self,
        ctx: &Context,
        fingerprint: u64,
        interval: Duration,
    ) -> Option<Duration> {
        let changed = fingerprint != self.last;
        self.last = fingerprint;
        let reader = self.reader.as_ref().filter(|_| self.open)?;
        let delay = if changed || self.moved || !reader.sleep() {
            Some(interval)
        } else {
            None
        };
        if let Some(delay) = delay {
            ctx.request_repaint_after(delay);
        }
        delay
    }
}

/// The visible live values of one frame, hashed (module docs).
#[derive(Default)]
pub struct Fingerprint {
    hasher: DefaultHasher,
}

impl Fingerprint {
    /// Count `value` (0–1) under `key` if the widget `id` drew this frame where it can be seen.
    pub fn visible(&mut self, ctx: &Context, id: Id, key: impl Hash, value: f32) {
        if is_visible(ctx, id) {
            self.add(key, value);
        }
    }

    /// Count `value` (0–1) under `key` unconditionally (a component that knows it is visible).
    pub fn add(&mut self, key: impl Hash, value: f32) {
        key.hash(&mut self.hasher);
        ((value * f32::from(QUANTUM)).round() as i32).hash(&mut self.hasher);
    }

    /// Count a raw number (a counter) under `key`.
    pub fn count(&mut self, key: impl Hash, n: u64) {
        key.hash(&mut self.hasher);
        n.hash(&mut self.hasher);
    }

    pub fn finish(&self) -> u64 {
        self.hasher.finish()
    }
}

/// Whether the widget `id` was drawn this frame inside its clip rectangle (not scrolled out of its region).
pub fn is_visible(ctx: &Context, id: Id) -> bool {
    ctx.read_response(id)
        .is_some_and(|r| r.interact_rect.is_positive())
}
