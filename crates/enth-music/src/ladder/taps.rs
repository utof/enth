//! **Owner: lane TAP** (the tap-back round, after EUI and FK4). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "Rhythm input".
//!
//! The ladder of the **tap-back** round (rhythm, syncopation), built only after lane TAP's timing spike, and on the tap
//! path the spike chooses (editor stamps first; host MIDI only with the owner's OK). `taps/1` in `enth-challenge`
//! scores it, on inter-onset intervals after a best-fit offset. Tap-along is deferred: it needs a calibration screen.
//!
//! **Level 1 (lane MUS, ahead of the spike so the answer shape exists): a short eighth-note pattern.** The clip is a
//! bar of metronome clicks (the count-in) and then the pattern, one bar over the clicks, at 100 bpm. The pattern is on
//! the even steps, strikes the downbeat, has 3 to 5 onsets and no syncopation. The answer is its onset times in ms from
//! the start of the pattern's bar ([`Answer::Taps`]; the response window is that bar, 2.4 s), to be tapped back after
//! its own count-in. No options.
//!
//! **Draw order** (part of the frozen form): the pattern ([`crate::rhythm::draw_patterns`], one `next_u32`).

use crate::ear::{Answer, Options, Question, TAIL_MS, clip_length_ms, timbre};
use crate::rhythm::{self, PatternRule};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The tempo in quarter notes per minute.
    pub bpm: u32,
    /// The swing, 50 straight ([`rhythm::SWING_STRAIGHT`]).
    pub swing_pct: u8,
    /// What the pattern must satisfy.
    pub rule: PatternRule,
    /// Bars of metronome alone before the pattern.
    pub count_in_bars: u32,
    /// The timbre of the pattern and the clicks.
    pub timbre: u8,
    /// The pattern's key.
    pub pattern_key: u8,
    /// The metronome's key.
    pub click_key: u8,
    /// The pattern's velocity.
    pub velocity: u8,
    /// The metronome's velocity.
    pub click_velocity: u8,
    /// The pattern notes' hold, in ms.
    pub hold_ms: u32,
    /// The clicks' hold, in ms.
    pub click_hold_ms: u32,
}

/// The ladder, level `n` at index `n - 1`.
pub const LEVELS: [Level; 1] = [Level {
    bpm: 100,
    swing_pct: rhythm::SWING_STRAIGHT,
    rule: PatternRule {
        allowed: 0x5555,
        anchor: 0x0001,
        density: (3, 5),
        syncopation: (0, 0),
        min_distance: 0,
    },
    count_in_bars: 1,
    timbre: timbre::PLUCK,
    pattern_key: 60,
    click_key: 84,
    velocity: 100,
    click_velocity: 60,
    hold_ms: 100,
    click_hold_ms: 60,
}];

/// The number of levels.
pub const MAX_LEVEL: u8 = LEVELS.len() as u8;

/// A question at `level` (1 to [`MAX_LEVEL`]; the caller checks), drawing from `rng`.
pub fn question(level: u8, rng: &mut Rng) -> Question {
    let l = &LEVELS[usize::from(level.clamp(1, MAX_LEVEL)) - 1];
    let pattern = rhythm::draw_patterns(rng, &l.rule, 1)[0];
    let step_ms = rhythm::step_ms_for_bpm(l.bpm);
    let bar = rhythm::bar_ms(step_ms);
    let bars = l.count_in_bars + 1;
    let mut notes = rhythm::metronome(
        0,
        bars,
        step_ms,
        l.click_key,
        l.click_velocity,
        l.click_hold_ms,
    );
    notes.extend(rhythm::bar_notes(
        pattern,
        l.count_in_bars * bar,
        step_ms,
        l.swing_pct,
        l.pattern_key,
        l.velocity,
        l.hold_ms,
    ));
    notes.sort_by_key(|n| n.start_ms);
    Question {
        timbre: l.timbre,
        length_ms: clip_length_ms(&notes).max(bars * bar + TAIL_MS),
        notes,
        post: None,
        answer: Answer::Taps {
            onsets_ms: pattern.onsets_ms(step_ms, l.swing_pct),
            window_ms: u16::try_from(bar).unwrap_or(u16::MAX),
        },
        options: Options::none(),
    }
}
