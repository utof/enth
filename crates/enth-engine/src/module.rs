//! The module contract: the four role traits, [`PrepareCfg`], the per-segment contexts and the parameter views.
//!
//! Modules are enum-dispatched by role (design.md §3): voice processors (generators and per-voice effects), voice
//! modulators (envelopes, voice LFOs), global modulators (global LFOs) and global processors (effects). The enums
//! and the exhaustive `match ModuleKind` that builds them live in [`crate::modules`]; this file is the contract each
//! module implements.
//!
//! # Rules for every module
//!
//! - **`new` is `prepare`.** A module is built by `Type::new(values, cfg)` from its patch values and a
//!   [`PrepareCfg`]. It may allocate there, sized only from the `PrepareCfg` (never from parameter values; a
//!   structural parameter such as `osc.table` may choose *what* to compute, not how much memory). Everything else
//!   (`start_voice`, `reset`, `process`, `tick`, `copy_state_from`, `subnormals`) never allocates, frees, locks,
//!   logs, does I/O or panics on valid input.
//! - **One call per module per segment.** The engine calls `process`/`tick` once per [`SEG`]-frame segment with the
//!   list of active voices in ascending index; the module loops over them. Per-voice state is structure-of-arrays,
//!   `[T; MAX_VOICES]`, so a later SIMD path can process 4–8 voices per lane group without an API change.
//! - **Parameters come from the views** ([`VoiceParams`], [`GlobalParams`]): plain values in the catalogue's units,
//!   this segment's (`cur`) and the previous segment's (`prev`), for per-sample ramps. Unmodulated, settled
//!   parameters hold the exact plain value from the patch. Read discrete parameters with the catalogue enums'
//!   `from_value` (`Wave::from_value(params.base(OscParam::Wave))`), which never panics.
//! - **Randomness only from the seeds passed in** ([`VoiceStart::seed`], [`GlobalStart::seed`]), through
//!   `enth_dsp::util::rng`. Never from parameter values, the patch hash, a clock or a global RNG.
//! - **The four DSP rules** (AGENTS.md): transcendentals only through `enth_dsp::util::math` (pinned `libm`); no
//!   `mul_add`; a fixed summation order; denormals flushed in code (`enth_dsp::util::flush::flush` on every write
//!   into a feedback path or recursive state, at least once per segment). Never read or set FTZ/DAZ.
//! - **Generators overwrite** `io[v]` for every active voice (the engine sums them in patch order); per-voice
//!   effects transform `io[v]` in place; global processors transform `left`/`right` in place.

use enth_patch::Rate;

pub use crate::params::{GlobalParams, ParamIndex, VoiceParams};
pub use crate::routing::AudioIn;

/// Frames per segment: the processing quantum. Events take effect at segment boundaries, modulation runs once per
/// segment, and the host adapter re-chunks any host block onto this free-running grid (design.md §3, §5).
pub const SEG: usize = 32;

/// Physical voice slots. Polyphony is at most 16 (`voice.polyphony`); the spare slots let a stolen voice fade out
/// while its replacement starts (lane S2b).
pub const MAX_VOICES: usize = 32;

/// A voice slot index, `0..MAX_VOICES`.
pub type VoiceIdx = usize;

/// One segment of one channel.
pub type Block = [f32; SEG];

/// One segment for every voice slot: the per-voice bus, indexed by [`VoiceIdx`].
pub type VoiceBus = [Block; MAX_VOICES];

/// Hard-sync timing, per voice and frame: [`NO_SYNC`] where the sync master's phase did not wrap during that frame,
/// otherwise the fraction of the frame that remained after the wrap (in `[0, 1)`; Plaits' `reset_time`), so a slave
/// can place a band-limited reset between samples.
pub type SyncBus = [Block; MAX_VOICES];

/// "No wrap in this frame" in a [`SyncBus`].
pub const NO_SYNC: f32 = -1.0;

/// `RAMP[i] = (i + 1) / SEG`, exact in `f32`: the weight of a linear per-sample ramp from a segment's previous value
/// to its current one, `prev + (cur − prev)·RAMP[i]`, which reaches `cur` on the last frame. Shared so every module
/// ramps the same way.
pub const RAMP: Block = {
    let mut r = [0.0; SEG];
    let mut i = 0;
    while i < SEG {
        r[i] = (i + 1) as f32 / SEG as f32;
        i += 1;
    }
    r
};

/// The lowest supported sample rate.
pub const MIN_SAMPLE_RATE: f32 = 8_000.0;
/// The highest supported sample rate.
pub const MAX_SAMPLE_RATE: f32 = 384_000.0;

/// Everything a module may size its allocations from (design.md §3): the only input to `new` besides the patch
/// values. Never parameter values. New fields are added with a default in [`PrepareCfg::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrepareCfg {
    /// Frames per second, within [`MIN_SAMPLE_RATE`]`..=`[`MAX_SAMPLE_RATE`].
    pub sample_rate: f32,
}

impl PrepareCfg {
    /// A configuration for `sample_rate`.
    #[must_use]
    pub const fn new(sample_rate: f32) -> Self {
        Self { sample_rate }
    }

    /// Whether the engine supports this configuration.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.sample_rate.is_finite()
            && (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate)
    }
}

/// What every voice module sees of the notes, per voice slot (structure of arrays). Written by the engine at the
/// segment boundary (step 2), read-only for modules. New fields may be added (additive only after F2).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteState {
    /// Pitch at the start of this segment, in semitones (MIDI note number; fractional once glide or tuning apply).
    pub pitch_prev: [f32; MAX_VOICES],
    /// Pitch at the end of this segment. Equal to `pitch_prev` unless the voice glides (lane S2b).
    pub pitch_cur: [f32; MAX_VOICES],
    /// Note-on velocity, 0–1.
    pub velocity: [f32; MAX_VOICES],
    /// The key is held.
    pub gate: [bool; MAX_VOICES],
    /// A note-on landed on this voice at this segment's start: a fresh voice, or (lane S2b) a mono retrigger.
    /// Envelopes start their attack from their current level.
    pub trigger: [bool; MAX_VOICES],
    /// The voice was freshly started at this segment's start (`start_voice` ran): there is no previous segment.
    pub fresh: [bool; MAX_VOICES],
    /// The gate fell at this segment's start.
    pub release: [bool; MAX_VOICES],
    /// The note index of the voice's note: note-ons counted from engine reset, starting at 0.
    pub note_index: [u64; MAX_VOICES],
}

impl NoteState {
    /// Every voice silent and released.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pitch_prev: [0.0; MAX_VOICES],
            pitch_cur: [0.0; MAX_VOICES],
            velocity: [0.0; MAX_VOICES],
            gate: [false; MAX_VOICES],
            trigger: [false; MAX_VOICES],
            fresh: [false; MAX_VOICES],
            release: [false; MAX_VOICES],
            note_index: [0; MAX_VOICES],
        }
    }
}

impl Default for NoteState {
    fn default() -> Self {
        Self::new()
    }
}

/// Passed to `start_voice` when a fresh voice starts in slot `voice` (step 2 of the segment).
///
/// Reset every per-voice state of slot `voice` (it may hold a previous note's values) and draw this note's randomness
/// from `seed`. Parameters: read [`VoiceParams::base`]; `cur`/`prev` for this voice are computed later in the segment.
pub struct VoiceStart<'a> {
    /// The voice slot.
    pub voice: VoiceIdx,
    /// `derive(derive(render_seed, module_key), note_index)` (`crate::seed::voice_seed`).
    pub seed: u64,
    /// `fnv1a64(module id)`: for a `seeded` phase mode, use `crate::seed::seeded_voice_seed(param_seed, module_key,
    /// note_index)`, the same rule with the module's own `seed` parameter in place of the render seed.
    pub module_key: u64,
    /// The note index (note-ons since engine reset).
    pub note_index: u64,
    /// The notes, already updated for this voice (pitch, velocity, gate).
    pub notes: &'a NoteState,
    /// This module's parameters.
    pub params: VoiceParams<'a>,
}

/// Passed to a global module's `reset` when it starts (engine start, or a fresh module in a new plan).
pub struct GlobalStart<'a> {
    /// `derive(render_seed, module_key)` (`crate::seed::global_seed`).
    pub seed: u64,
    /// `fnv1a64(module id)`.
    pub module_key: u64,
    /// This module's parameters (settled at their patch values).
    pub params: GlobalParams<'a>,
}

/// The context of a voice processor's `process`.
pub struct VoiceCx<'a> {
    /// Active voices, ascending (the fixed voice order).
    pub voices: &'a [VoiceIdx],
    /// The notes.
    pub notes: &'a NoteState,
    /// This module's parameters.
    pub params: VoiceParams<'a>,
    /// Generators: overwrite `io[v]` for every active `v`. Per-voice effects: transform `io[v]` in place.
    pub io: &'a mut VoiceBus,
    /// `Some` only for the sync master, the first enabled `osc` in `generators`: write, for every active voice and
    /// frame, where its phase wrapped ([`SyncBus`]). The engine fills it with [`NO_SYNC`] first.
    pub sync_out: Option<&'a mut SyncBus>,
    /// `Some` for the generators after the sync master (when there is one): the master's wraps for this segment, for
    /// an `osc` with `sync` on. `None` for the master itself, for per-voice effects, and when no `osc` is enabled.
    pub sync_in: Option<&'a SyncBus>,
    /// Generators that are a carrier: `Some` when an audio route reaches this module (lane RTE): the pre-level
    /// outputs of the generators that feed it, scaled by the route depths. Read an `audio` parameter's input with
    /// [`AudioIn::read`]. `None` for a module no audio route reaches, and for per-voice effects.
    pub audio_in: Option<AudioIn<'a>>,
    /// Generators that are a source: `Some` when an audio route leaves this module (lane RTE), zeroed for the active
    /// voices. Copy the output *before* the `level` ramp into `pre[v]` for every active voice. `None` otherwise.
    ///
    /// What a route carries from a **stereo** generator (lane UNI) is its one mono signal: the mix of its copies
    /// before the spread, which is also what a plan without the stereo voice path would render, whatever `spread` is.
    pub pre_level: Option<&'a mut VoiceBus>,
    /// A stereo generator's right channel (lane UNI): `Some` only for a generator whose `is_stereo` is true, in a plan
    /// with the stereo voice path. Overwrite `io_r[v]` for every active voice, beside `io[v]` which is then the left
    /// channel. `None` for every other module, and for a stereo generator in a mono plan: it writes its mono mix to
    /// `io` alone. A generator that is not stereo never sees it; the engine adds its `io` to both channels.
    pub io_r: Option<&'a mut VoiceBus>,
}

/// The context of a voice modulator's `tick`.
pub struct VoiceModCx<'a> {
    /// Active voices, ascending.
    pub voices: &'a [VoiceIdx],
    /// The notes.
    pub notes: &'a NoteState,
    /// This module's parameters.
    pub params: VoiceParams<'a>,
    /// The tempo in beats per minute, as of this segment ([`crate::tempo`]).
    pub tempo_bpm: f64,
    /// Write this segment's output for every active voice: the value at the *end* of the segment (envelopes 0–1,
    /// a bipolar LFO −1..1). The engine keeps the previous value for ramps.
    pub out: &'a mut [f32; MAX_VOICES],
}

/// The context of a global modulator's `tick`.
pub struct GlobalModCx<'a> {
    /// This module's parameters.
    pub params: GlobalParams<'a>,
    /// The tempo in beats per minute, as of this segment ([`crate::tempo`]).
    pub tempo_bpm: f64,
}

/// The context of a global processor's `process`: the stereo bus after the voice sum (and earlier effects).
pub struct GlobalCx<'a> {
    /// This module's parameters.
    pub params: GlobalParams<'a>,
    /// The tempo in beats per minute, as of this segment ([`crate::tempo`]).
    pub tempo_bpm: f64,
    /// Left channel, in place.
    pub left: &'a mut Block,
    /// Right channel, in place.
    pub right: &'a mut Block,
}

/// A per-voice module that makes or shapes audio: generators (`osc`, `noise`) and per-voice effects (`svf`).
pub trait VoiceProcessor {
    /// A fresh voice starts in slot `start.voice`: reset that slot's state.
    fn start_voice(&mut self, start: &VoiceStart<'_>);
    /// Render one segment for every active voice.
    fn process(&mut self, cx: &mut VoiceCx<'_>);
    /// Copy all state from `other` (the same kind, prepared with the same `PrepareCfg`), for the plan swap (lane
    /// S2b): afterwards both produce identical output for identical input. No allocation.
    fn copy_state_from(&mut self, other: &Self);
    /// The number of subnormal values in the module's state (test support for the release-tail invariant).
    fn subnormals(&self) -> usize;
}

/// A per-voice modulation source: envelopes (`adsr`) and voice-scope LFOs.
pub trait VoiceModulator {
    /// A fresh voice starts in slot `start.voice`: reset that slot's state.
    fn start_voice(&mut self, start: &VoiceStart<'_>);
    /// Advance one segment for every active voice and write `cx.out[v]`.
    fn tick(&mut self, cx: &mut VoiceModCx<'_>);
    /// Whether voice `v` is over: past its attack and below −100 dB
    /// ([`VOICE_END_LEVEL`](enth_dsp::util::math::VOICE_END_LEVEL)). Only the amplitude envelope's answer ends a
    /// voice; a modulator that never ends one returns `false`.
    fn voice_finished(&self, v: VoiceIdx) -> bool;
    /// See [`VoiceProcessor::copy_state_from`].
    fn copy_state_from(&mut self, other: &Self);
    /// See [`VoiceProcessor::subnormals`].
    fn subnormals(&self) -> usize;
}

/// A modulation source shared by all voices: global-scope LFOs.
pub trait GlobalModulator {
    /// Start from scratch (engine start or a fresh module in a new plan).
    fn reset(&mut self, start: &GlobalStart<'_>);
    /// Advance one segment and return the value at its end.
    fn tick(&mut self, cx: &mut GlobalModCx<'_>) -> f32;
    /// See [`VoiceProcessor::copy_state_from`].
    fn copy_state_from(&mut self, other: &Self);
    /// See [`VoiceProcessor::subnormals`].
    fn subnormals(&self) -> usize;
}

/// A stereo effect after the voice sum: `reverb`, `comp`.
pub trait GlobalProcessor {
    /// Start from scratch (engine start or a fresh module in a new plan).
    fn reset(&mut self, start: &GlobalStart<'_>);
    /// Process one segment of the stereo bus in place.
    fn process(&mut self, cx: &mut GlobalCx<'_>);
    /// `true` only when processing a segment of exact zeros would output exact zeros and leave the state unchanged
    /// (all state flushed to zero, no fade in flight). The engine then skips the module while its input is silent,
    /// which must be bit-identical to running it.
    fn is_silent(&self) -> bool;
    /// See [`VoiceProcessor::copy_state_from`].
    fn copy_state_from(&mut self, other: &Self);
    /// Take all state from `other`, which is never run again (an instant plan swap, P1b lane E+): afterwards this
    /// module produces exactly what `other` would have produced, and `other` may hold anything (for example this
    /// module's old, freshly prepared state). The default copies. A module with a large state (the reverb's 1.6 MB of
    /// delay lines) overrides it with an O(1) exchange: nothing is freed on the audio thread, because the old plan
    /// goes back to the main thread with whatever `other` now holds. No allocation.
    fn take_state_from(&mut self, other: &mut Self) {
        self.copy_state_from(other);
    }
    /// See [`VoiceProcessor::subnormals`].
    fn subnormals(&self) -> usize;
}

/// The four roles, as data (for the plan compiler and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Generators and per-voice effects.
    VoiceProcessor,
    /// Envelopes and voice LFOs.
    VoiceModulator,
    /// Global LFOs.
    GlobalModulator,
    /// Global effects.
    GlobalProcessor,
}

impl Role {
    /// Whether modules of this role run once per voice.
    #[must_use]
    pub const fn rate(self) -> Rate {
        match self {
            Role::VoiceProcessor | Role::VoiceModulator => Rate::Voice,
            Role::GlobalModulator | Role::GlobalProcessor => Rate::Global,
        }
    }
}

/// Count the subnormal values in a slice of `f32` state (for [`VoiceProcessor::subnormals`] and friends).
#[must_use]
pub fn count_subnormals(values: &[f32]) -> usize {
    values.iter().filter(|x| x.is_subnormal()).count()
}

/// Count the subnormal values in a slice of `f64` state.
#[must_use]
pub fn count_subnormals_f64(values: &[f64]) -> usize {
    values.iter().filter(|x| x.is_subnormal()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_exact_and_ends_at_one() {
        let (first, last) = (RAMP[0], RAMP[SEG - 1]);
        assert_eq!(first.to_bits(), (1.0f32 / 32.0).to_bits());
        assert_eq!(last.to_bits(), 1.0f32.to_bits());
        for (i, r) in RAMP.iter().enumerate() {
            assert_eq!(f64::from(*r), (i + 1) as f64 / SEG as f64);
        }
    }

    #[test]
    fn prepare_cfg_range() {
        assert!(PrepareCfg::new(48_000.0).is_supported());
        assert!(PrepareCfg::new(8_000.0).is_supported());
        assert!(!PrepareCfg::new(7_999.0).is_supported());
        assert!(!PrepareCfg::new(f32::NAN).is_supported());
        assert!(!PrepareCfg::new(400_000.0).is_supported());
    }
}
