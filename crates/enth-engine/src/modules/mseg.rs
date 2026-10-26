//! The `mseg` module: a multi-segment envelope or drawn LFO, per voice or global (S2a lane MSEG).
//! DSP core: `enth_dsp::mseg`; notes: `docs/dsp/mseg.md`.
//!
//! One type serves both roles: a voice-scope MSEG is a `VoiceModulator` (started at each note-on that restarts the
//! envelopes), a global-scope one a `GlobalModulator`; `ModuleImpl::new` picks the role from the structural `scope`
//! parameter, as for the LFO. Once per 32-frame segment each MSEG moves its position by one increment and writes the
//! shape at the *end* of the segment, `0..1` (`unipolar`) or `-1..1` (`bipolar`) scaled by the velocity amount; the
//! engine ramps from the previous segment's value.
//!
//! - **Length.** `sync` on: `beats` at the engine's tempo (`cx.tempo_bpm`, read each segment); off: `length` in ms,
//!   read per voice because it is modulatable. The position is a fraction of the length, so a tempo or length change
//!   moves the speed and never the point times, and the increment is exact (rounded up, in `u128`): a one-shot ends on
//!   the exact segment (F1m S5).
//! - **Start.** `start_voice` puts a fresh voice at the `phase` knob's position. A *retrigger* (a note-on on a voice
//!   that is still held or releasing: `trigger` without `fresh`, the mono mode) restarts the position at the same
//!   place, as the LFO's one-shot and `adsr` do; a legato note is not a trigger and does not restart it.
//! - **Sustain loop.** The voice's gate is the key: held, the position loops between the loop points (equal points
//!   hold a level); released, it goes on from where it is to the end.
//! - **Global scope (F1m S7, decided here).** A global MSEG is started at the engine's start or reset (and when it
//!   appears in a new plan), like a global LFO. A `loop` goes on from there for ever. A `one_shot` or `sustain_loop`
//!   is an envelope for the phrase, so it restarts at the **first key of a phrase** (a note-on while no key is held;
//!   [`GlobalModulator::keys`]), and its gate is "some key is held": a global sustain loop sustains while any key is
//!   down and plays its release when the last one is let go. It ignores velocity.
//! - **Edits do not click.** The shape is compiled from the parameters once per segment, only when one that matters
//!   has changed. At a change, each voice goes on from where the old shape would have been and glides to the new one
//!   over [`EDIT_GLIDE_MS`] (a linear glide in the output), so dragging a point or switching the mode is smooth. A
//!   restart (a note-on) does not glide: it is a jump, ramped by the engine over one segment, like the LFO's.

use enth_dsp::mseg::{
    Cursor, Mode, POINTS, Point, Shape, length_increment, start_position, synced_increment,
};
use enth_patch::catalogue::{Beats, MSEG_POINT_BASE, MsegMode, MsegParam, Polarity};

use crate::module::{
    GlobalModCx, GlobalModulator, GlobalStart, MAX_VOICES, PrepareCfg, SEG, VoiceIdx, VoiceModCx,
    VoiceModulator, VoiceStart, count_subnormals_f64,
};

/// How long an edit takes to arrive, in milliseconds (a linear glide of the output, whole segments, at least one).
pub const EDIT_GLIDE_MS: f64 = 5.0;

/// A glide smaller than this is not worth starting.
const GLIDE_FLOOR: f64 = 1e-9;

/// What the increment was computed from, so a repeated setting costs a comparison and not a `u128` division.
#[derive(Clone, Copy, Debug, PartialEq)]
enum IncKey {
    /// Nothing yet.
    None,
    /// A tempo (bits of the `f64`) and a length in beats.
    Beats(u64, (u32, u32)),
    /// A length in ms (bits of the `f32`).
    Ms(u32),
}

/// One voice's (or the global MSEG's) running state.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    cursor: Cursor,
    /// The edit glide still to run: added to the output, linearly to zero.
    glide: f64,
    glide_step: f64,
    glide_left: u32,
}

impl Track {
    fn new(pos: u64) -> Track {
        Track {
            cursor: Cursor::new(pos),
            glide: 0.0,
            glide_step: 0.0,
            glide_left: 0,
        }
    }

    fn restart(&mut self, pos: u64) {
        *self = Track::new(pos);
    }
}

/// The MSEG.
#[derive(Clone, Debug)]
pub struct Mseg {
    sample_rate: f32,
    glide_steps: u32,
    /// The shape the parameters describe now, and the one before the last edit (for the glide).
    shape: Shape,
    before_edit: Shape,
    /// The parameter values `shape` was compiled from.
    snapshot: [f32; MsegParam::COUNT],
    inc_key: IncKey,
    inc: u64,
    /// One per voice slot (voice scope).
    voices: [Track; MAX_VOICES],
    /// The one global MSEG (global scope), with what the engine last said about the keys.
    global: Track,
    keys_held: bool,
    phrase_start: bool,
}

/// The parameters that make the shape: everything but the length (`sync`, `length`, `beats`), the `scope` and the
/// start `phase`, and of the points only those used.
fn shape_differs(a: &[f32; MsegParam::COUNT], get: impl Fn(usize) -> f32) -> bool {
    let fixed = [
        MsegParam::Mode,
        MsegParam::Polarity,
        MsegParam::Velocity,
        MsegParam::Points,
        MsegParam::LoopStart,
        MsegParam::LoopEnd,
    ];
    if fixed.iter().any(|p| a[p.index()] != get(p.index())) {
        return true;
    }
    let used = points_used(get(MsegParam::Points.index()));
    (MSEG_POINT_BASE..MSEG_POINT_BASE + 3 * used).any(|i| a[i] != get(i))
}

fn points_used(value: f32) -> usize {
    (value as usize).clamp(2, POINTS)
}

/// Compile the shape from the parameter values (`get(index)`, in spec order).
fn compile(get: impl Fn(usize) -> f32) -> Shape {
    let mode = match MsegMode::from_value(get(MsegParam::Mode.index())) {
        Some(MsegMode::OneShot) => Mode::OneShot,
        Some(MsegMode::SustainLoop) => Mode::SustainLoop,
        Some(MsegMode::Loop) | None => Mode::Loop,
    };
    let n = points_used(get(MsegParam::Points.index()));
    let mut points = [Point {
        time: 0.0,
        level: 0.0,
        curve: 0.0,
    }; POINTS];
    for (i, point) in points.iter_mut().enumerate().take(n) {
        let base = MSEG_POINT_BASE + 3 * i;
        *point = Point {
            time: f64::from(get(base)) / 100.0,
            level: f64::from(get(base + 1)) / 100.0,
            curve: f64::from(get(base + 2)) / 100.0,
        };
    }
    let bipolar = Polarity::from_value(get(MsegParam::Polarity.index())) == Some(Polarity::Bipolar);
    Shape::new(mode, &points[..n])
        .with_loop(
            get(MsegParam::LoopStart.index()) as usize,
            get(MsegParam::LoopEnd.index()) as usize,
        )
        .with_output(bipolar, f64::from(get(MsegParam::Velocity.index())) / 100.0)
}

fn snapshot_of(get: impl Fn(usize) -> f32) -> [f32; MsegParam::COUNT] {
    core::array::from_fn(get)
}

/// One length in beats for a stored `beats` value (a bar for a value that is no item, which cannot happen).
fn cycle_beats(value: f32) -> (u32, u32) {
    Beats::from_value(value).unwrap_or(Beats::Bar).length()
}

impl Mseg {
    /// Prepare an MSEG for the patch `values`.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let get = |i: usize| values.get(i).copied().unwrap_or(0.0);
        let shape = compile(get);
        let glide_steps =
            (EDIT_GLIDE_MS * f64::from(cfg.sample_rate) / 1000.0 / SEG as f64).round() as u32;
        Self {
            sample_rate: cfg.sample_rate,
            glide_steps: glide_steps.max(1),
            shape,
            before_edit: shape,
            snapshot: snapshot_of(get),
            inc_key: IncKey::None,
            inc: 0,
            voices: [Track::new(0); MAX_VOICES],
            global: Track::new(0),
            keys_held: false,
            phrase_start: false,
        }
    }

    /// Take the shape from the parameters now, with no glide (a start or a reset, not an edit).
    fn adopt(&mut self, get: impl Fn(usize) -> f32 + Copy) {
        self.shape = compile(get);
        self.before_edit = self.shape;
        self.snapshot = snapshot_of(get);
    }

    /// Recompile the shape if a parameter that matters changed since the last segment; whether it did.
    fn refresh(&mut self, get: impl Fn(usize) -> f32 + Copy) -> bool {
        if !shape_differs(&self.snapshot, get) {
            return false;
        }
        self.before_edit = self.shape;
        self.shape = compile(get);
        self.snapshot = snapshot_of(get);
        true
    }

    /// The position increment for this segment, recomputed only when the tempo and length changed.
    fn increment(
        inc_key: &mut IncKey,
        inc: &mut u64,
        sample_rate: f32,
        synced: Option<(f64, (u32, u32))>,
        length_ms: f32,
    ) -> u64 {
        let key = match synced {
            Some((bpm, beats)) => IncKey::Beats(bpm.to_bits(), beats),
            None => IncKey::Ms(length_ms.to_bits()),
        };
        if key != *inc_key {
            *inc_key = key;
            *inc = match synced {
                Some((bpm, beats)) => synced_increment(bpm, beats, sample_rate, SEG),
                None => length_increment(length_ms, sample_rate, SEG),
            };
        }
        *inc
    }
}

/// One segment of one track: advance, read the output and add the edit glide. `edited` says the shape changed this
/// segment; `glide_ok` is false for a track that was just started or restarted (a jump, not an edit).
#[allow(
    clippy::too_many_arguments,
    reason = "one hot function over disjoint pieces of the module's state, split so the borrows stay disjoint"
)]
#[inline]
fn step(
    shape: &Shape,
    before: &Shape,
    edited: bool,
    glide_ok: bool,
    glide_steps: u32,
    track: &mut Track,
    inc: u64,
    held: bool,
    velocity: f64,
) -> f32 {
    track.cursor.advance(shape, inc, held);
    let out = shape.output(track.cursor.level(shape), velocity);
    if track.glide_left > 0 {
        track.glide_left -= 1;
        track.glide = if track.glide_left == 0 {
            0.0
        } else {
            track.glide - track.glide_step
        };
    }
    if edited && glide_ok {
        // Go on from where the old shape (and the glide still running) would be, and glide to the new one.
        let old = before.output(track.cursor.level(before), velocity);
        let jump = old + track.glide - out;
        if jump.abs() > GLIDE_FLOOR {
            track.glide = jump;
            track.glide_step = jump / f64::from(glide_steps);
            track.glide_left = glide_steps;
        } else {
            track.glide = 0.0;
            track.glide_left = 0;
        }
    }
    (out + track.glide) as f32
}

impl VoiceModulator for Mseg {
    fn start_voice(&mut self, start: &VoiceStart<'_>) {
        let pos = start_position(start.params.base(MsegParam::Phase.index()));
        self.voices[start.voice] = Track::new(pos);
    }

    fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        let p = cx.params;
        let get = |i: usize| p.base(i);
        let edited = self.refresh(get);
        let synced = (p.base(MsegParam::Sync.index()) != 0.0)
            .then(|| (cx.tempo_bpm, cycle_beats(p.base(MsegParam::Beats.index()))));
        let restart_at = start_position(p.base(MsegParam::Phase.index()));
        let Self {
            sample_rate,
            glide_steps,
            shape,
            before_edit,
            inc_key,
            inc,
            voices,
            ..
        } = self;
        for &v in cx.voices {
            let track = &mut voices[v];
            // A mono retrigger (a note-on on a voice that is still held or releasing) is a `trigger` without a fresh
            // voice: the shape starts again, as an envelope's attack does.
            let restarted = cx.notes.trigger[v] && !cx.notes.fresh[v];
            if restarted {
                track.restart(restart_at);
            }
            let fresh = cx.notes.fresh[v];
            let increment = Self::increment(
                inc_key,
                inc,
                *sample_rate,
                synced,
                p.cur(MsegParam::Length.index(), v),
            );
            cx.out[v] = step(
                shape,
                before_edit,
                edited,
                !(fresh || restarted),
                *glide_steps,
                track,
                increment,
                cx.notes.gate[v],
                f64::from(cx.notes.velocity[v]),
            );
        }
    }

    fn voice_finished(&self, _v: VoiceIdx) -> bool {
        false
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.clone_from(other);
    }

    fn subnormals(&self) -> usize {
        let glides: [f64; MAX_VOICES + 1] =
            core::array::from_fn(|i| self.voices.get(i).map_or(self.global.glide, |t| t.glide));
        count_subnormals_f64(&glides)
    }
}

impl GlobalModulator for Mseg {
    fn reset(&mut self, start: &GlobalStart<'_>) {
        let p = start.params;
        self.adopt(|i| p.base(i));
        self.global = Track::new(start_position(p.base(MsegParam::Phase.index())));
        self.keys_held = false;
        self.phrase_start = false;
        self.inc_key = IncKey::None;
    }

    fn keys(&mut self, held: bool, phrase_start: bool) {
        self.keys_held = held;
        self.phrase_start = phrase_start;
    }

    fn tick(&mut self, cx: &mut GlobalModCx<'_>) -> f32 {
        let p = cx.params;
        let get = |i: usize| p.base(i);
        let edited = self.refresh(get);
        let synced = (p.base(MsegParam::Sync.index()) != 0.0)
            .then(|| (cx.tempo_bpm, cycle_beats(p.base(MsegParam::Beats.index()))));
        // An envelope restarts with the phrase; a loop runs on from the engine's start.
        let restarted = self.phrase_start && self.shape.mode() != Mode::Loop;
        self.phrase_start = false;
        if restarted {
            self.global
                .restart(start_position(p.base(MsegParam::Phase.index())));
        }
        let increment = Self::increment(
            &mut self.inc_key,
            &mut self.inc,
            self.sample_rate,
            synced,
            p.cur(MsegParam::Length.index()),
        );
        step(
            &self.shape,
            &self.before_edit,
            edited,
            !restarted,
            self.glide_steps,
            &mut self.global,
            increment,
            self.keys_held,
            1.0,
        )
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.clone_from(other);
    }

    fn subnormals(&self) -> usize {
        VoiceModulator::subnormals(self)
    }
}
