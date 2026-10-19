//! enth-render (L3): the offline, deterministic render, `render(patch, performance, spec) → stereo f32`.
//!
//! `render` is a pure function of (patch, performance, sample rate, render seed, engine version) (design.md §5): the
//! same inputs give the same bits on every platform (x86-64 Linux and Windows, wasm32; checked by the golden hashes
//! in CI), in debug and release, with or without FMA, and whatever the host's FTZ/DAZ setting. The host block size
//! is not an input: the renderer feeds the engine in [`DEFAULT_BLOCK`]-frame chunks, and [`render_with_block`] shows
//! that any other chunking gives the same bits.
//!
//! Times in a [`Performance`] are seconds; they become frames as `round(t · sample_rate)` in `f64`, and the engine
//! applies each event at the first 32-frame segment boundary at or after its frame. Events at the same frame apply
//! in this order: automation first, then notes in performance order, each note's on before its off.
//!
//! The render seed ([`RenderSpec::seed`]) is the only source of randomness: free-running oscillator phases and every
//! other random stream derive from it, the module id and the note index, never from the patch. Re-rendering with
//! another seed is how the descriptors measure their noise floor.
//!
//! ```
//! use enth_patch::Patch;
//! use enth_render::{Performance, RenderSpec, render};
//!
//! let out = render(&Patch::init(), &Performance::note(60, 0.8, 0.25, 0.5), &RenderSpec::default()).unwrap();
//! assert_eq!(out.frames(), 24_000);
//! assert!(out.left.iter().any(|x| *x != 0.0));
//! ```

use core::fmt;

use enth_engine::{CompileError, Engine, Event, EventKind, NoteKey, PrepareCfg, compile};
use enth_patch::{ParamAddr, Patch};

/// The render seed used when nothing else is given (the engine spike's, `0x0E17_5EED`). The CLI's `--seed` and a
/// challenge's seed replace it.
pub const DEFAULT_SEED: u64 = 0x0E17_5EED;

/// The canonical scoring rate.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// Frames per `Engine::process` call in [`render`]. Any other value gives the same output.
pub const DEFAULT_BLOCK: usize = 512;

/// The longest render, in seconds.
pub const MAX_SECONDS: f64 = 600.0;

/// How to render: the sample rate and the render seed. Build with [`RenderSpec::new`] or `..Default::default()`
/// (fields may be added, with defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSpec {
    /// Frames per second, 8,000–384,000.
    pub sample_rate: u32,
    /// The render seed: every random stream derives from it, the module id and the note index.
    pub seed: u64,
}

impl RenderSpec {
    /// A spec with this sample rate and seed.
    #[must_use]
    pub const fn new(sample_rate: u32, seed: u64) -> Self {
        Self { sample_rate, seed }
    }
}

impl Default for RenderSpec {
    /// 48 kHz with [`DEFAULT_SEED`].
    fn default() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE, DEFAULT_SEED)
    }
}

/// One note of a performance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// MIDI key, 0–127 (A4 = 69 = 440 Hz).
    pub key: u8,
    /// Velocity, 0–1.
    pub velocity: f32,
    /// Note-on time in seconds, `>= 0`.
    pub on_s: f64,
    /// Note-off time in seconds, `>= on_s`. A note-off at or after the end never happens.
    pub off_s: f64,
}

impl Note {
    /// A note.
    #[must_use]
    pub const fn new(key: u8, velocity: f32, on_s: f64, off_s: f64) -> Self {
        Self {
            key,
            velocity,
            on_s,
            off_s,
        }
    }
}

/// A parameter change at a point in time: the parameter's base value is set to `value` (plain units, clamped to its
/// spec), like host automation. The engine then smooths it as the parameter's spec says (`Smoothing::Linear`
/// ramps of 10 ms for most parameters, 20 ms for `master.level`), so what the voices hear is a ramp, not a jump.
#[derive(Clone, Debug, PartialEq)]
pub struct Automation {
    /// Time in seconds, `>= 0`.
    pub time_s: f64,
    /// The parameter (`filter1.cutoff`, `master.level`).
    pub addr: ParamAddr,
    /// Plain value.
    pub value: f32,
}

/// What is played: notes, automation and the length of the render. Build with `..Default::default()` (fields may
/// be added, with defaults), or [`Performance::note`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Performance {
    /// The notes, in any order (same-frame events follow this order).
    pub notes: Vec<Note>,
    /// Parameter changes, in any order.
    pub automation: Vec<Automation>,
    /// Length of the render in seconds (frames = `round(duration_s · sample_rate)`).
    pub duration_s: f64,
}

impl Performance {
    /// One note at time 0, held for `hold_s`, in a render of `duration_s`.
    #[must_use]
    pub fn note(key: u8, velocity: f32, hold_s: f64, duration_s: f64) -> Self {
        Self {
            notes: vec![Note::new(key, velocity, 0.0, hold_s)],
            automation: Vec::new(),
            duration_s,
        }
    }
}

/// Counters from a render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Note-ons played.
    pub notes: u64,
    /// Note-ons dropped because no voice could be given. Always 0 in practice: when polyphony is full the engine
    /// steals a voice (the oldest released one, else the oldest held one), counted in the engine's `stolen_voices`.
    pub dropped_notes: u64,
    /// Events dropped because the engine's event queue was full (never, for sane performances).
    pub dropped_events: u64,
    /// Voices still sounding at the end.
    pub voices_at_end: usize,
}

/// A stereo render.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOutput {
    /// Frames per second.
    pub sample_rate: u32,
    /// Left channel.
    pub left: Vec<f32>,
    /// Right channel (same length).
    pub right: Vec<f32>,
    /// Counters.
    pub stats: RenderStats,
}

impl RenderOutput {
    /// Frames per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// Interleaved `L R L R …` (for WAV files and hashes).
    #[must_use]
    pub fn interleaved(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.left.len() * 2);
        for (l, r) in self.left.iter().zip(&self.right) {
            out.push(*l);
            out.push(*r);
        }
        out
    }

    /// The mid signal `(L + R) / 2` (for mono analysis).
    #[must_use]
    pub fn mid(&self) -> Vec<f32> {
        self.left
            .iter()
            .zip(&self.right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    }

    /// The largest absolute sample on either channel.
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.left
            .iter()
            .chain(&self.right)
            .fold(0.0f32, |m, x| m.max(x.abs()))
    }
}

/// Why a render could not run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RenderError {
    /// The patch does not compile.
    Compile(CompileError),
    /// The spec is out of range.
    BadSpec(String),
    /// The performance has a bad time, key or velocity.
    BadPerformance(String),
    /// An automation address names no parameter of the patch.
    UnknownParam(String),
}

impl RenderError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            RenderError::Compile(e) => e.code(),
            RenderError::BadSpec(_) => "bad_spec",
            RenderError::BadPerformance(_) => "bad_performance",
            RenderError::UnknownParam(_) => "unknown_param",
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Compile(e) => write!(f, "{e}"),
            RenderError::BadSpec(m) | RenderError::BadPerformance(m) => f.write_str(m),
            RenderError::UnknownParam(a) => write!(
                f,
                "automation names '{a}', which is not a parameter of this patch"
            ),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<CompileError> for RenderError {
    fn from(e: CompileError) -> Self {
        RenderError::Compile(e)
    }
}

/// Render `perf` through `patch` (see the crate docs).
///
/// # Errors
/// A patch that does not compile, a sample rate outside 8–384 kHz, or a bad performance (non-finite or negative
/// times, an off before its on, a key above 127, a velocity outside 0–1, a length over [`MAX_SECONDS`], automation
/// of an unknown parameter or with a non-finite value).
pub fn render(
    patch: &Patch,
    perf: &Performance,
    spec: &RenderSpec,
) -> Result<RenderOutput, RenderError> {
    render_with_block(patch, perf, spec, DEFAULT_BLOCK)
}

/// [`render`] with the engine fed `block` frames per call (at least 1). The output is the same for every block size;
/// this exists so tests can show it.
///
/// # Errors
/// As [`render`].
pub fn render_with_block(
    patch: &Patch,
    perf: &Performance,
    spec: &RenderSpec,
    block: usize,
) -> Result<RenderOutput, RenderError> {
    let sr = f64::from(spec.sample_rate);
    let cfg = PrepareCfg::new(spec.sample_rate as f32);
    if !cfg.is_supported() {
        return Err(RenderError::BadSpec(format!(
            "sample_rate {} is outside 8000..=384000",
            spec.sample_rate
        )));
    }
    let d = perf.duration_s;
    if !(d.is_finite() && (0.0..=MAX_SECONDS).contains(&d)) {
        return Err(RenderError::BadPerformance(format!(
            "duration_s {d} is outside 0..={MAX_SECONDS}"
        )));
    }
    let frames = (d * sr).round() as usize;
    let (plan, map) = compile(patch, &cfg)?;
    let to_frame = |t: f64| (t * sr).round() as u64;

    // (frame, event); a stable sort by frame keeps the order below for same-frame events.
    let mut events: Vec<(u64, EventKind)> =
        Vec::with_capacity(perf.automation.len() + 2 * perf.notes.len());
    for a in &perf.automation {
        if !(a.time_s.is_finite() && a.time_s >= 0.0) {
            return Err(RenderError::BadPerformance(format!(
                "automation of {} at time {}: times are finite and >= 0",
                a.addr, a.time_s
            )));
        }
        if !a.value.is_finite() {
            return Err(RenderError::BadPerformance(format!(
                "automation of {} to {}: values are finite",
                a.addr, a.value
            )));
        }
        match map.get(&a.addr) {
            Some(id) => events.push((to_frame(a.time_s), EventKind::Param { id, value: a.value })),
            // A parameter of a disabled module: it exists, and changing it changes nothing.
            None if patch.spec(&a.addr).is_some() => {}
            None => return Err(RenderError::UnknownParam(a.addr.to_string())),
        }
    }
    for (i, n) in perf.notes.iter().enumerate() {
        let times_ok =
            n.on_s.is_finite() && n.off_s.is_finite() && n.on_s >= 0.0 && n.off_s >= n.on_s;
        if !times_ok || n.key > 127 || !(0.0..=1.0).contains(&n.velocity) {
            return Err(RenderError::BadPerformance(format!(
                "note {i}: key {} velocity {} on {} s off {} s (keys 0-127, velocity 0-1, 0 <= on <= off)",
                n.key, n.velocity, n.on_s, n.off_s
            )));
        }
        let key = NoteKey::new(n.key, i32::try_from(i).unwrap_or(i32::MAX));
        events.push((
            to_frame(n.on_s),
            EventKind::NoteOn {
                key,
                velocity: n.velocity,
            },
        ));
        events.push((to_frame(n.off_s), EventKind::NoteOff { key, velocity: 0.0 }));
    }
    events.sort_by_key(|(frame, _)| *frame);

    let mut engine = Engine::new(plan, spec.seed);
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let block = block.max(1);
    let mut chunk: Vec<Event> = Vec::new();
    let mut next = 0;
    let mut start = 0;
    while start < frames {
        let end = (start + block).min(frames);
        chunk.clear();
        while let Some(&(frame, kind)) = events.get(next) {
            if frame >= end as u64 {
                break;
            }
            chunk.push(Event {
                offset: (frame - start as u64) as u32,
                kind,
            });
            next += 1;
        }
        engine.process(&mut left[start..end], &mut right[start..end], &chunk);
        start = end;
    }
    let s = engine.stats();
    Ok(RenderOutput {
        sample_rate: spec.sample_rate,
        left,
        right,
        stats: RenderStats {
            notes: s.notes,
            dropped_notes: s.dropped_notes,
            dropped_events: s.dropped_events,
            voices_at_end: engine.active_voices().len(),
        },
    })
}
