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
            time: f64::from(get(base)),
            level: f64::from(get(base + 1)),
            curve: f64::from(get(base + 2)),
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
    let was = track.cursor;
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
        // Go on from where the old shape (and the glide still running) would be, and glide to the new one. The old
        // shape moves its own copy of the cursor: a mode or loop-point change can send the new one somewhere else.
        let mut shadow = was;
        shadow.advance(before, inc, held);
        let old = before.output(shadow.level(before), velocity);
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

#[cfg(test)]
#[allow(
    clippy::cast_lossless,
    reason = "loop indices in the tests' closed forms"
)]
mod tests {
    use super::*;
    use crate::module::{GlobalParams, NoteState, VoiceParams};
    use enth_patch::ModuleKind;
    use enth_patch::catalogue::{Beats, MsegScope};

    const SR: f32 = 48_000.0;

    /// `(time %, level %, curve %)`.
    type Pt = (f32, f32, f32);

    /// The documented curve, written out here with no code shared with the core.
    fn bend(c: f64, u: f64) -> f64 {
        if c >= 1.0 {
            return 1.0;
        }
        if c <= -1.0 {
            return 0.0;
        }
        let k = (1.0 - c) / (1.0 + c);
        u / (u + k * (1.0 - u))
    }

    /// The documented shape at the share `x` of the length, from `(time, level, curve)` shares.
    fn model(points: &[(f64, f64, f64)], looped: bool, x: f64) -> f64 {
        let n = points.len();
        let mut t = Vec::new();
        let mut latest = 0.0f64;
        for p in points {
            latest = latest.max(p.0);
            t.push(latest);
        }
        if x < t[0] {
            return points[0].1;
        }
        let i = (0..n).rev().find(|&i| t[i] <= x).unwrap();
        let mix = |from: f64, to: f64, f: f64| from * (1.0 - f) + to * f;
        if i + 1 < n {
            let u = (x - t[i]) / (t[i + 1] - t[i]);
            mix(points[i].1, points[i + 1].1, bend(points[i].2, u))
        } else if looped {
            let u = (x - t[i]) / (1.0 - t[i]);
            mix(points[i].1, points[0].1, bend(points[i].2, u))
        } else {
            points[i].1
        }
    }

    fn shares(points: &[Pt]) -> Vec<(f64, f64, f64)> {
        points
            .iter()
            .map(|&(t, l, c)| {
                (
                    f64::from(t) / 100.0,
                    f64::from(l) / 100.0,
                    f64::from(c) / 100.0,
                )
            })
            .collect()
    }

    struct Rig {
        values: Vec<f32>,
        cur: Vec<[f32; MAX_VOICES]>,
        gcur: Vec<f32>,
        notes: NoteState,
        mseg: Mseg,
        tempo: f64,
    }

    impl Rig {
        fn new(set: &[(MsegParam, f32)], points: &[Pt]) -> Rig {
            let mut values = ModuleKind::Mseg.defaults();
            for &(p, v) in set {
                values[p.index()] = v;
            }
            if !points.is_empty() {
                values[MsegParam::Points.index()] = points.len() as f32;
                for (i, &(t, l, c)) in points.iter().enumerate() {
                    let base = MSEG_POINT_BASE + 3 * i;
                    values[base] = t;
                    values[base + 1] = l;
                    values[base + 2] = c;
                }
            }
            let mseg = Mseg::new(&values, &PrepareCfg::new(SR));
            let mut rig = Rig {
                cur: Vec::new(),
                gcur: Vec::new(),
                values,
                notes: NoteState::new(),
                mseg,
                tempo: 120.0,
            };
            rig.sync_cur();
            rig
        }

        fn sync_cur(&mut self) {
            self.cur = self.values.iter().map(|&v| [v; MAX_VOICES]).collect();
            self.gcur = self.values.clone();
        }

        fn set(&mut self, p: MsegParam, v: f32) {
            self.values[p.index()] = v;
            self.sync_cur();
        }

        fn note_on(&mut self, voice: usize, velocity: f32) {
            self.notes.velocity[voice] = velocity;
            self.notes.gate[voice] = true;
            self.notes.trigger[voice] = true;
            self.notes.fresh[voice] = true;
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            VoiceModulator::start_voice(
                &mut self.mseg,
                &VoiceStart {
                    voice,
                    seed: 1,
                    module_key: 0xABCD,
                    note_index: 0,
                    notes: &self.notes,
                    params,
                },
            );
        }

        /// A mono retrigger: a note-on on a voice that is already playing.
        fn retrigger(&mut self, voice: usize, velocity: f32) {
            self.notes.velocity[voice] = velocity;
            self.notes.gate[voice] = true;
            self.notes.trigger[voice] = true;
        }

        fn tick_voice(&mut self, voice: usize) -> f32 {
            let params = VoiceParams::from_slices(&self.values, &self.cur, &self.cur);
            let mut out = [0.0; MAX_VOICES];
            VoiceModulator::tick(
                &mut self.mseg,
                &mut VoiceModCx {
                    voices: &[voice],
                    notes: &self.notes,
                    params,
                    tempo_bpm: self.tempo,
                    out: &mut out,
                },
            );
            self.notes.trigger = [false; MAX_VOICES];
            self.notes.fresh = [false; MAX_VOICES];
            out[voice]
        }

        fn reset_global(&mut self) {
            let params = GlobalParams::from_slices(&self.values, &self.gcur, &self.gcur);
            GlobalModulator::reset(
                &mut self.mseg,
                &GlobalStart {
                    seed: 1,
                    module_key: 0xABCD,
                    params,
                },
            );
        }

        /// One global segment with the keys as given.
        fn tick_global(&mut self, held: bool, phrase_start: bool) -> f32 {
            let params = GlobalParams::from_slices(&self.values, &self.gcur, &self.gcur);
            GlobalModulator::keys(&mut self.mseg, held, phrase_start);
            GlobalModulator::tick(
                &mut self.mseg,
                &mut GlobalModCx {
                    params,
                    tempo_bpm: self.tempo,
                },
            )
        }
    }

    fn close(got: f32, want: f64, tol: f64, what: &str) {
        assert!(
            (f64::from(got) - want).abs() <= tol,
            "{what}: got {got}, want {want}"
        );
    }

    #[test]
    fn the_default_is_a_one_bar_triangle_that_loops() {
        // 120 BPM: a bar is 2 s, 3000 segments. The value after segment k is the triangle at (k + 1) / 3000.
        let mut rig = Rig::new(&[], &[]);
        rig.note_on(0, 1.0);
        let tri = shares(&[(0.0, 0.0, 0.0), (50.0, 100.0, 0.0), (100.0, 0.0, 0.0)]);
        for k in 0..7000 {
            let x = ((k + 1) % 3000) as f64 / 3000.0;
            let y = rig.tick_voice(0);
            close(y, model(&tri, true, x), 1e-6, &format!("segment {k}"));
        }
    }

    #[test]
    fn every_curve_matches_the_documented_formula_within_1e_6() {
        // One shot of 1000 ms (1500 segments), two points, the curve on point 1.
        for c in [
            -100.0f32, -99.0, -75.0, -50.0, -10.0, 0.0, 10.0, 50.0, 75.0, 99.0, 100.0,
        ] {
            let pts: [Pt; 2] = [(0.0, 0.0, c), (100.0, 100.0, 0.0)];
            let mut rig = Rig::new(
                &[
                    (MsegParam::Mode, MsegMode::OneShot.index() as f32),
                    (MsegParam::Sync, 0.0),
                    (MsegParam::Length, 1000.0),
                ],
                &pts,
            );
            rig.note_on(0, 1.0);
            let shape = shares(&pts);
            for k in 0..1500 {
                let y = rig.tick_voice(0);
                let want = if k == 1499 {
                    1.0
                } else {
                    model(&shape, false, (k + 1) as f64 / 1500.0)
                };
                close(y, want, 1e-6, &format!("curve {c}, segment {k}"));
            }
            // The one shot holds the last level.
            for _ in 0..10 {
                assert_eq!(rig.tick_voice(0), 1.0, "curve {c}");
            }
        }
    }

    #[test]
    fn a_zero_curve_is_straight_and_plus_and_minus_100_are_steps() {
        let run = |c: f32| {
            let pts: [Pt; 2] = [(0.0, 20.0, c), (100.0, 80.0, 0.0)];
            let mut rig = Rig::new(
                &[
                    (MsegParam::Mode, MsegMode::OneShot.index() as f32),
                    (MsegParam::Sync, 0.0),
                    (MsegParam::Length, 1000.0),
                ],
                &pts,
            );
            rig.note_on(0, 1.0);
            (0..1500).map(|_| rig.tick_voice(0)).collect::<Vec<_>>()
        };
        for (k, y) in run(0.0).iter().enumerate().take(1499) {
            close(*y, 0.2 + 0.6 * (k + 1) as f64 / 1500.0, 1e-6, "straight");
        }
        // +100 %: the next level at the first segment. -100 %: the first level until the last segment.
        let up = run(100.0);
        assert!(up.iter().all(|&y| (y - 0.8).abs() < 1e-6));
        let hold = run(-100.0);
        assert!(hold[..1499].iter().all(|&y| (y - 0.2).abs() < 1e-6));
        assert_eq!(hold[1499], 0.8);
    }

    /// A one-shot of two points that holds 0 and jumps to 1 at the very end (`-100 %` on point 1).
    fn hold_then_jump(set: &[(MsegParam, f32)]) -> Rig {
        let mut all = vec![(MsegParam::Mode, MsegMode::OneShot.index() as f32)];
        all.extend_from_slice(set);
        Rig::new(&all, &[(0.0, 0.0, -100.0), (100.0, 100.0, 0.0)])
    }

    #[test]
    fn a_one_shot_ends_on_the_exact_segment_for_a_synced_and_a_ms_length() {
        let beats = |b: Beats| (MsegParam::Beats, b.index() as f32);
        let synced = |bpm: f64, b: Beats, n: usize| {
            let mut rig = hold_then_jump(&[beats(b)]);
            rig.tempo = bpm;
            (rig, n, format!("{b:?} at {bpm} BPM"))
        };
        let ms = |length: f32, n: usize| {
            let rig = hold_then_jump(&[(MsegParam::Sync, 0.0), (MsegParam::Length, length)]);
            (rig, n, format!("{length} ms"))
        };
        let cases = vec![
            synced(120.0, Beats::Quarter, 750),
            synced(90.0, Beats::Quarter, 1000),
            synced(150.0, Beats::Quarter, 600),
            synced(100.0, Beats::Quarter, 900),
            synced(75.0, Beats::Quarter, 1200),
            synced(60.0, Beats::Bar, 6000),
            synced(120.0, Beats::Eighth, 375),
            synced(120.0, Beats::QuarterDotted, 1125),
            ms(10.0, 15),
            ms(500.0, 750),
            ms(1000.0, 1500),
            ms(60_000.0, 90_000),
        ];
        for (mut rig, n, what) in cases {
            rig.note_on(0, 1.0);
            for k in 0..n + 3 {
                let y = rig.tick_voice(0);
                let want = if k + 1 >= n { 1.0 } else { 0.0 };
                assert_eq!(y, want, "{what}: segment {k} of {n}");
            }
        }
    }

    #[test]
    fn a_synced_one_shot_ends_on_the_exact_segment_across_tempo_changes() {
        // A quarter note is bpm / 90000 of its length per segment: the runs below add up to 90000 exactly, so the
        // shape ends at segment 800 (index 799), whatever the order of the changes.
        let runs: [(f64, usize); 5] = [
            (120.0, 150),
            (60.0, 300),
            (240.0, 100),
            (90.0, 200),
            (240.0, 50),
        ];
        assert_eq!(
            runs.iter().map(|&(b, n)| b as usize * n).sum::<usize>(),
            90_000
        );
        let mut rig = hold_then_jump(&[(MsegParam::Beats, Beats::Quarter.index() as f32)]);
        rig.note_on(0, 1.0);
        let mut k = 0;
        for (bpm, count) in runs {
            rig.tempo = bpm;
            for _ in 0..count {
                let y = rig.tick_voice(0);
                assert_eq!(y, if k == 799 { 1.0 } else { 0.0 }, "segment {k}");
                k += 1;
            }
        }
        assert_eq!(rig.tick_voice(0), 1.0);
    }

    #[test]
    fn a_tempo_change_moves_the_speed_and_never_a_point() {
        // A looping saw 0 → 1 over a quarter note, sampled at 120 BPM and then 60 BPM: the position carries over.
        let pts: [Pt; 2] = [(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)];
        let mut rig = Rig::new(&[(MsegParam::Beats, Beats::Quarter.index() as f32)], &pts);
        rig.note_on(0, 1.0);
        for _ in 0..300 {
            rig.tick_voice(0);
        }
        rig.tempo = 60.0;
        // 300 segments at 120 BPM is 0.4 of the length; 1500 segments per length at 60 BPM.
        for k in 0..1000 {
            let y = rig.tick_voice(0);
            let x = (0.4 + (k + 1) as f64 / 1500.0).fract();
            close(y, x, 1e-6, &format!("segment {k} after the change"));
        }
    }

    #[test]
    fn velocity_scales_by_one_minus_amount_times_one_minus_velocity() {
        // A held level of 80 % (two points at 80 %), unipolar and bipolar.
        for (amount, velocity) in [
            (0.0f32, 0.25f32),
            (50.0, 0.25),
            (100.0, 0.25),
            (100.0, 1.0),
            (30.0, 0.0),
        ] {
            let pts: [Pt; 2] = [(0.0, 80.0, 0.0), (100.0, 80.0, 0.0)];
            let scale = 1.0 - f64::from(amount) / 100.0 * (1.0 - f64::from(velocity));
            for (polarity, level) in [(Polarity::Unipolar, 0.8), (Polarity::Bipolar, 0.6)] {
                let mut rig = Rig::new(
                    &[
                        (MsegParam::Velocity, amount),
                        (MsegParam::Polarity, polarity.index() as f32),
                    ],
                    &pts,
                );
                rig.note_on(0, velocity);
                let y = rig.tick_voice(0);
                close(y, level * scale, 1e-6, &format!("{amount} % at {velocity}"));
            }
        }
        // Bipolar is 2 · level − 1: a level of 0 is -1 and of 1 is +1.
        let pts: [Pt; 2] = [(0.0, 0.0, 0.0), (100.0, 100.0, 100.0)];
        let mut rig = Rig::new(
            &[
                (MsegParam::Polarity, Polarity::Bipolar.index() as f32),
                (MsegParam::Mode, MsegMode::OneShot.index() as f32),
            ],
            &pts,
        );
        rig.note_on(0, 1.0);
        let first = rig.tick_voice(0);
        close(first, 2.0 / 3000.0 - 1.0, 1e-6, "bipolar start");
    }

    #[test]
    fn the_start_phase_starts_the_shape_part_way() {
        // A one-shot saw over 1000 ms starting at 180°: half the length is left, 750 segments.
        let pts: [Pt; 2] = [(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)];
        let mut rig = Rig::new(
            &[
                (MsegParam::Mode, MsegMode::OneShot.index() as f32),
                (MsegParam::Sync, 0.0),
                (MsegParam::Length, 1000.0),
                (MsegParam::Phase, 180.0),
            ],
            &pts,
        );
        rig.note_on(0, 1.0);
        for k in 0..750 {
            let y = rig.tick_voice(0);
            let want = if k == 749 {
                1.0
            } else {
                0.5 + (k + 1) as f64 / 1500.0
            };
            close(y, want, 1e-6, &format!("segment {k}"));
        }
    }

    #[test]
    fn a_retrigger_restarts_the_shape_and_a_legato_note_does_not() {
        let pts: [Pt; 2] = [(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)];
        let mut rig = Rig::new(
            &[
                (MsegParam::Mode, MsegMode::OneShot.index() as f32),
                (MsegParam::Sync, 0.0),
                (MsegParam::Length, 1000.0),
            ],
            &pts,
        );
        rig.note_on(0, 1.0);
        for _ in 0..600 {
            rig.tick_voice(0);
        }
        // Legato: no trigger, the shape goes on.
        let y = rig.tick_voice(0);
        close(y, 601.0 / 1500.0, 1e-6, "legato");
        // Mono retrigger: the shape starts again.
        rig.retrigger(0, 1.0);
        let y = rig.tick_voice(0);
        close(y, 1.0 / 1500.0, 1e-6, "retrigger");
        let y = rig.tick_voice(0);
        close(y, 2.0 / 1500.0, 1e-6, "after the retrigger");
    }

    #[test]
    fn an_edit_glides_to_the_new_shape_over_eight_segments() {
        // Hold 50 % (sustain loop, both points equal); raise point 1 and 2 to 100 %: the output moves in eight equal
        // steps from where it was, starting in the segment of the edit.
        let pts: [Pt; 2] = [(0.0, 50.0, 0.0), (100.0, 50.0, 0.0)];
        let mut rig = Rig::new(
            &[
                (MsegParam::Mode, MsegMode::SustainLoop.index() as f32),
                (MsegParam::LoopStart, 1.0),
                (MsegParam::LoopEnd, 2.0),
            ],
            &pts,
        );
        rig.note_on(0, 1.0);
        for _ in 0..100 {
            assert_eq!(rig.tick_voice(0), 0.5);
        }
        rig.values[MSEG_POINT_BASE + 1] = 100.0;
        rig.values[MSEG_POINT_BASE + 4] = 100.0;
        rig.sync_cur();
        for j in 0..=8 {
            let y = rig.tick_voice(0);
            close(
                y,
                0.5 + 0.5 * f64::from(j) / 8.0,
                1e-6,
                &format!("step {j}"),
            );
        }
        for _ in 0..20 {
            assert_eq!(rig.tick_voice(0), 1.0);
        }
    }

    #[test]
    fn a_global_loop_runs_from_the_reset_whatever_the_keys_do() {
        let mut rig = Rig::new(
            &[
                (MsegParam::Scope, MsegScope::Global.index() as f32),
                (MsegParam::Beats, Beats::Quarter.index() as f32),
            ],
            &[(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)],
        );
        rig.reset_global();
        for k in 0..2000 {
            // A phrase start in the middle changes nothing for a loop.
            let y = rig.tick_global(k % 400 < 200, k == 333);
            close(
                y,
                ((k + 1) % 750) as f64 / 750.0,
                1e-6,
                &format!("segment {k}"),
            );
        }
    }

    #[test]
    fn a_global_one_shot_restarts_at_the_first_key_of_a_phrase_only() {
        let mut rig = Rig::new(
            &[
                (MsegParam::Scope, MsegScope::Global.index() as f32),
                (MsegParam::Mode, MsegMode::OneShot.index() as f32),
                (MsegParam::Beats, Beats::Quarter.index() as f32),
            ],
            &[(0.0, 0.0, 0.0), (100.0, 100.0, 0.0)],
        );
        rig.reset_global();
        // From the reset it runs its 750 segments and holds.
        for k in 0..800 {
            let y = rig.tick_global(false, false);
            close(
                y,
                ((k + 1) as f64 / 750.0).min(1.0),
                1e-6,
                &format!("segment {k}"),
            );
        }
        // The first key of a phrase restarts it; the segment of the restart is already one step in.
        let y = rig.tick_global(true, true);
        close(y, 1.0 / 750.0, 1e-6, "restart");
        for k in 1..300 {
            let y = rig.tick_global(true, false);
            close(
                y,
                (k + 1) as f64 / 750.0,
                1e-6,
                &format!("held, segment {k}"),
            );
        }
    }

    #[test]
    fn a_global_sustain_loop_loops_while_any_key_is_down_and_plays_out_after() {
        // 0 → 100 % (point 2, 20 %), hold 100 % until 40 % (point 3), then down to 0 at 100 %; loop on points 2..3.
        let pts: [Pt; 4] = [
            (0.0, 0.0, 0.0),
            (20.0, 100.0, 0.0),
            (40.0, 100.0, 0.0),
            (100.0, 0.0, 0.0),
        ];
        let mut rig = Rig::new(
            &[
                (MsegParam::Scope, MsegScope::Global.index() as f32),
                (MsegParam::Mode, MsegMode::SustainLoop.index() as f32),
                (MsegParam::LoopStart, 2.0),
                (MsegParam::LoopEnd, 3.0),
                (MsegParam::Beats, Beats::Quarter.index() as f32),
            ],
            &pts,
        );
        rig.reset_global();
        // Settle the reset run first (no key): it plays the whole shape once and holds the end.
        for _ in 0..800 {
            rig.tick_global(false, false);
        }
        // A key: restart, rise for 150 segments, then loop 2..3 (a flat 100 %) for as long as the key is down.
        let first = rig.tick_global(true, true);
        close(first, 1.0 / 750.0 / 0.2, 1e-6, "rising");
        for _ in 0..2000 {
            let y = rig.tick_global(true, false);
            assert!((0.0..=1.0).contains(&y));
        }
        close(
            rig.tick_global(true, false),
            1.0,
            1e-9,
            "looping on the flat part",
        );
        // Released: it goes on from where it is, down to 0 over the rest of the length, and holds the end.
        let mut last = 1.0f32;
        let mut ended = false;
        for _ in 0..1000 {
            let y = rig.tick_global(false, false);
            assert!(y <= last + 1e-6, "falls");
            last = y;
            ended |= y == 0.0;
        }
        assert!(ended, "it reaches the end");
        assert_eq!(rig.tick_global(false, false), 0.0);
    }

    #[test]
    fn nonsense_values_never_produce_a_non_finite_output() {
        let mut rig = Rig::new(&[(MsegParam::Sync, 0.0)], &[]);
        rig.note_on(0, 1.0);
        for length in [10.0f32, 60_000.0, f32::NAN, 0.0, -5.0, f32::INFINITY] {
            rig.values[MsegParam::Length.index()] = length;
            rig.sync_cur();
            for _ in 0..50 {
                assert!(rig.tick_voice(0).is_finite(), "length {length}");
            }
        }
        rig.tempo = f64::NAN;
        rig.set(MsegParam::Sync, 1.0);
        for _ in 0..50 {
            assert!(rig.tick_voice(0).is_finite());
        }
    }
}
