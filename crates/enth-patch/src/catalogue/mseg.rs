//! `mseg`: a multi-segment envelope or LFO with **16 fixed points** (time, level and a curve each), of which the first
//! `points` are used; a length in ms or synced in beats; loop, one-shot and sustain-loop modes; per voice or global.
//! It is also enth's envelope for times above 10 s (decision 2). Lane MSEG owns its DSP, `docs/dsp/mseg.md` and its
//! golden cases (docs/phase-s2.md).
//!
//! **A reserved S2a kind** (lane S0d, freeze F0m): its key, id prefix, section and discriminant (12) are fixed and
//! persisted; it is hidden ([`ModuleSpec::hidden`](super::ModuleSpec::hidden)) from the Add menu, `enth modules` and
//! the generator until lane INT-S2a clears the flag. **The parameter table below is lane CAT's F1m table**: after F1m
//! its keys, units, ranges, scales, defaults and flags change only through the coordinator (docs/dsp/catalogue.md,
//! "S2a decisions": why 16 points, why the MSEG covers long envelopes).
//!
//! # The shape, as the table's meanings assume it (lane MSEG implements it)
//!
//! - **Length:** `beats` of the render's tempo when `sync` is on, `length` in ms when it is off. Point times are shares
//!   of it, so a 1-bar shape on a 1/16 grid has its points at multiples of 6.25 %.
//! - **Points:** the first `points` points (2–16) are the shape; the rest are ignored. A point's effective time is its
//!   `time` or the previous point's effective time, whichever is later, so points never cross (equal times make a
//!   vertical edge). Before point 1's time the output holds point 1's level; after the last point it holds the last
//!   level, except in `loop` mode, where the last segment heads back to point 1's level at the end of the length so
//!   the loop joins.
//! - **Curves:** `curve<n>` bends the segment *leaving* point n: 0 % straight, positive moves early (fast then slow),
//!   negative late; **+100 % jumps to the next level at once and −100 % holds, then jumps at the end**, so 16 points
//!   make a 16-step pattern. Report 05's rational curve `x / (x + k(1 − x))`, `k` from the curve once per segment
//!   (`k` = 1 straight, 0 and ∞ the two steps), needs no per-sample `pow`.
//! - **Modes:** `loop` repeats the whole length; `one_shot` runs once and holds the last level; `sustain_loop` runs to
//!   point `loop_end`, then loops `loop_start` … `loop_end` while the note is held (equal points hold that level, a
//!   sustain), and at note-off goes on from where it is to the end. `loop_start` and `loop_end` above `points` count as
//!   the last point; `loop_start` above `loop_end` counts as `loop_end`.
//! - **Start:** each note-on that restarts the voice's envelopes (the voice mode decides, as for `adsr`) restarts the
//!   shape at `phase` (a share of the length). A **global** MSEG (`scope`) starts at the render start and never sees a
//!   note-off, so `sustain_loop` loops for ever; only a global MSEG may modulate effects.
//! - **Output:** unipolar 0..1 (the levels as drawn) or bipolar −1..1 (`2·level − 1`), scaled by note velocity as
//!   `velocity` says (like `adsr`'s). Evaluated once per 32-frame segment, like every modulator.
//!
//! `scope` needs lane MSEG to give `ModuleKind::rate` an `mseg` arm (as `lfo` has) and the engine a global modulator
//! arm; until then an MSEG runs per voice whatever `scope` says, and validation treats it as a voice source, so no
//! global route can reach a stub.
//!
//! At its defaults it is a 1-bar synced triangle (0 → 100 % → 0), looping, unipolar, per voice: harmless until routed,
//! and an attack-hold-release envelope when switched to `sustain_loop` (it holds at point 2).

use enth_params::{ParamSpec, Scale, Unit, enum_items, params};

use super::lfo::{POLARITIES, Polarity};
use super::{BEATS, Beats};

/// The number of points every MSEG has (decision 1: 16, the most the answer code's 63 parameters leave room for with
/// the other 11 parameters, and a 1/16 grid over a bar).
pub const MSEG_POINTS: usize = 16;

/// The parameters before the first point (`mode` … `loop_end`).
pub const MSEG_POINT_BASE: usize = 11;

enum_items! {
    /// How an MSEG plays its shape.
    pub enum MsegMode in MSEG_MODES {
        /// Repeats the whole length.
        Loop = ("loop", "Loop", "Repeats the shape for as long as the note lasts: a drawn LFO for wobbles and rhythms."),
        /// Runs once, then holds the last level.
        OneShot = ("one_shot", "One-shot", "Plays the shape once from the note's start, then holds the last level: an envelope of any length."),
        /// Loops a region while the note is held, then plays to the end.
        SustainLoop = ("sustain_loop", "Sustain loop", "Plays to the loop end, loops between loop start and loop end while the note is held, then plays the rest after release."),
    }
}

enum_items! {
    /// Whether one MSEG runs per voice or one is shared by all voices (as `lfo`'s scope).
    pub enum MsegScope in MSEG_SCOPES {
        /// One MSEG per voice, started at each note-on.
        Voice = ("voice", "Per voice", "Each note gets its own MSEG, started at note-on."),
        /// One MSEG for the whole engine, started at the render start.
        Global = ("global", "Global", "One MSEG shared by every note, running from the start; the only kind that may modulate effects."),
    }
}

params! {
    /// Parameters of the `mseg` module, in spec order: 11, then the 16 points (time, level, curve each).
    pub enum MsegParam in MSEG_PARAMS {
        Mode = ParamSpec::enumeration("mode", "Mode", &MSEG_MODES, MsegMode::Loop.index())
            .basic("How the shape plays: loop (a drawn LFO), one-shot (an envelope) or sustain loop (loops while the note is held)."),
        Sync = ParamSpec::toggle("sync", "Tempo sync", true)
            .basic("On: the shape's length is beats of the song's tempo. Off: it is length, in milliseconds."),
        Length = ParamSpec::float("length", "Length", Unit::Ms, 10.0, 60_000.0, Scale::Log, 1_000.0)
            .modulatable()
            .intermediate("How long one pass through the shape takes when tempo sync is off, up to a minute (long risers and swells).")
            .moves("envelope.decay_ms"),
        Beats = ParamSpec::enumeration("beats", "Beats", &BEATS, Beats::Bar.index())
            .basic("How long one pass through the shape takes when tempo sync is on, as a note division: 1/16 to 16 bars."),
        Polarity = ParamSpec::enumeration("polarity", "Polarity", &POLARITIES, Polarity::Unipolar.index())
            .intermediate("Unipolar outputs the levels as drawn (0 to 1); bipolar maps them to -1 to +1, swinging around the destination's value."),
        Scope = ParamSpec::enumeration("scope", "Scope", &MSEG_SCOPES, MsegScope::Voice.index())
            .structural()
            .intermediate("One MSEG per voice or one global MSEG; only a global MSEG can modulate effects. Changing it rebuilds the plan."),
        Phase = ParamSpec::float("phase", "Start", Unit::Degrees, 0.0, 360.0, Scale::Linear, 0.0)
            .periodic()
            .advanced("Where in the shape each note starts, as a share of the length (180deg starts halfway)."),
        Velocity = ParamSpec::float("velocity", "Velocity", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .intermediate("How much note velocity scales the output: 0 % ignores velocity, 100 % follows it fully."),
        Points = ParamSpec::int("points", "Points", Unit::None, 2, 16, 3)
            .intermediate("How many of the 16 points make the shape; the rest are ignored."),
        LoopStart = ParamSpec::int("loop_start", "Loop start", Unit::None, 1, 16, 2)
            .advanced("The point the sustain loop jumps back to while the note is held."),
        LoopEnd = ParamSpec::int("loop_end", "Loop end", Unit::None, 1, 16, 2)
            .advanced("The point where the sustain loop turns back while the note is held; equal to loop start, the level is held (a sustain)."),
        Time1 = ParamSpec::float("time1", "Point 1 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Where point 1 sits, as a share of the length; a point never sits before the one before it."),
        Level1 = ParamSpec::float("level1", "Point 1 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 1's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve1 = ParamSpec::float("curve1", "Point 1 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 1 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time2 = ParamSpec::float("time2", "Point 2 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 50.0)
            .advanced("Where point 2 sits, as a share of the length; a point never sits before the one before it."),
        Level2 = ParamSpec::float("level2", "Point 2 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Point 2's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve2 = ParamSpec::float("curve2", "Point 2 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 2 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time3 = ParamSpec::float("time3", "Point 3 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 3 sits, as a share of the length; a point never sits before the one before it."),
        Level3 = ParamSpec::float("level3", "Point 3 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 3's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve3 = ParamSpec::float("curve3", "Point 3 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 3 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time4 = ParamSpec::float("time4", "Point 4 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 4 sits, as a share of the length; a point never sits before the one before it."),
        Level4 = ParamSpec::float("level4", "Point 4 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 4's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve4 = ParamSpec::float("curve4", "Point 4 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 4 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time5 = ParamSpec::float("time5", "Point 5 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 5 sits, as a share of the length; a point never sits before the one before it."),
        Level5 = ParamSpec::float("level5", "Point 5 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 5's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve5 = ParamSpec::float("curve5", "Point 5 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 5 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time6 = ParamSpec::float("time6", "Point 6 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 6 sits, as a share of the length; a point never sits before the one before it."),
        Level6 = ParamSpec::float("level6", "Point 6 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 6's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve6 = ParamSpec::float("curve6", "Point 6 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 6 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time7 = ParamSpec::float("time7", "Point 7 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 7 sits, as a share of the length; a point never sits before the one before it."),
        Level7 = ParamSpec::float("level7", "Point 7 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 7's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve7 = ParamSpec::float("curve7", "Point 7 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 7 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time8 = ParamSpec::float("time8", "Point 8 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 8 sits, as a share of the length; a point never sits before the one before it."),
        Level8 = ParamSpec::float("level8", "Point 8 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 8's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve8 = ParamSpec::float("curve8", "Point 8 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 8 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time9 = ParamSpec::float("time9", "Point 9 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 9 sits, as a share of the length; a point never sits before the one before it."),
        Level9 = ParamSpec::float("level9", "Point 9 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 9's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve9 = ParamSpec::float("curve9", "Point 9 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 9 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time10 = ParamSpec::float("time10", "Point 10 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 10 sits, as a share of the length; a point never sits before the one before it."),
        Level10 = ParamSpec::float("level10", "Point 10 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 10's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve10 = ParamSpec::float("curve10", "Point 10 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 10 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time11 = ParamSpec::float("time11", "Point 11 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 11 sits, as a share of the length; a point never sits before the one before it."),
        Level11 = ParamSpec::float("level11", "Point 11 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 11's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve11 = ParamSpec::float("curve11", "Point 11 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 11 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time12 = ParamSpec::float("time12", "Point 12 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 12 sits, as a share of the length; a point never sits before the one before it."),
        Level12 = ParamSpec::float("level12", "Point 12 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 12's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve12 = ParamSpec::float("curve12", "Point 12 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 12 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time13 = ParamSpec::float("time13", "Point 13 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 13 sits, as a share of the length; a point never sits before the one before it."),
        Level13 = ParamSpec::float("level13", "Point 13 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 13's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve13 = ParamSpec::float("curve13", "Point 13 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 13 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time14 = ParamSpec::float("time14", "Point 14 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 14 sits, as a share of the length; a point never sits before the one before it."),
        Level14 = ParamSpec::float("level14", "Point 14 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 14's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve14 = ParamSpec::float("curve14", "Point 14 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 14 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time15 = ParamSpec::float("time15", "Point 15 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 15 sits, as a share of the length; a point never sits before the one before it."),
        Level15 = ParamSpec::float("level15", "Point 15 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 15's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve15 = ParamSpec::float("curve15", "Point 15 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 15 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
        Time16 = ParamSpec::float("time16", "Point 16 time", Unit::Percent, 0.0, 100.0, Scale::Linear, 100.0)
            .advanced("Where point 16 sits, as a share of the length; a point never sits before the one before it."),
        Level16 = ParamSpec::float("level16", "Point 16 level", Unit::Percent, 0.0, 100.0, Scale::Linear, 0.0)
            .advanced("Point 16's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar)."),
        Curve16 = ParamSpec::float("curve16", "Point 16 curve", Unit::Percent, -100.0, 100.0, Scale::Linear, 0.0)
            .advanced("The bend of the segment from point 16 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps."),
    }
}

// Point n's time, level and curve are at `MSEG_POINT_BASE + 3·(n − 1)` and the two after it.
const _: () = {
    assert!(MsegParam::Time1.index() == MSEG_POINT_BASE);
    assert!(MsegParam::Curve16.index() == MSEG_POINT_BASE + 3 * MSEG_POINTS - 1);
    assert!(MsegParam::COUNT == MSEG_POINT_BASE + 3 * MSEG_POINTS);
};
