//! **Owner: lane G2** (ladder file; level 1 is lane MUS's). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "The games".
//!
//! The ladder of **Rhythm pattern** (listening only): a one-bar pattern, pick one of four, then mark it on the 16-step
//! grid. The syncopation index runs from low to high, then density and swing. Judged exactly, with no input timing.
//!
//! **Level 1 (lane MUS): four eighth-note patterns, hardly syncopated.** The clip is a bar of metronome clicks (a
//! count-in) and then two bars of the pattern over the clicks, at 100 bpm (a sixteenth is 150 ms), 8.2 s with its tail.
//! The four options are patterns on the even steps (the eighth-note grid) that always strike the downbeat, have 3 to 6
//! onsets, measure 0 or 1 on the syncopation index (67 patterns qualify) and differ pairwise at 3 steps or more; the
//! answer is one of them, uniformly.
//! The options are listed in draw order. To mark the pattern on the grid, the expected mask is `options.ids[answer]`.
//!
//! **Draw order** (part of the frozen form): the four patterns ([`crate::rhythm::draw_patterns`]: one `next_u32` per
//! pattern, more if an attempt starts over), then the answer's index.

use crate::ear::{Answer, OptionKind, Options, Question, TAIL_MS, below, clip_length_ms, timbre};
use crate::rhythm::{self, PatternRule};
use enth_dsp::util::rng::Rng;

/// One row of the ladder.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    /// The tempo in quarter notes per minute.
    pub bpm: u32,
    /// The swing, 50 straight ([`rhythm::SWING_STRAIGHT`]).
    pub swing_pct: u8,
    /// What each option pattern must satisfy.
    pub rule: PatternRule,
    /// How many patterns a question lists.
    pub options: usize,
    /// Bars of metronome alone before the pattern.
    pub count_in_bars: u32,
    /// Bars of the pattern.
    pub pattern_bars: u32,
    /// The timbre of the pattern and the clicks (the clicks sit at `click_key`, the pattern at `pattern_key`).
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
        density: (3, 6),
        syncopation: (0, 1),
        min_distance: 3,
    },
    options: 4,
    count_in_bars: 1,
    pattern_bars: 2,
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
    let patterns = rhythm::draw_patterns(rng, &l.rule, l.options);
    let answer = below(rng, patterns.len());
    let step_ms = rhythm::step_ms_for_bpm(l.bpm);
    let bar = rhythm::bar_ms(step_ms);
    let bars = l.count_in_bars + l.pattern_bars;
    let mut notes = rhythm::metronome(
        0,
        bars,
        step_ms,
        l.click_key,
        l.click_velocity,
        l.click_hold_ms,
    );
    for b in 0..l.pattern_bars {
        notes.extend(rhythm::bar_notes(
            patterns[answer],
            (l.count_in_bars + b) * bar,
            step_ms,
            l.swing_pct,
            l.pattern_key,
            l.velocity,
            l.hold_ms,
        ));
    }
    notes.sort_by_key(|n| n.start_ms);
    Question {
        timbre: l.timbre,
        length_ms: clip_length_ms(&notes).max(bars * bar + TAIL_MS),
        notes,
        post: None,
        answer: Answer::Choice(answer as u8),
        options: Options {
            kind: OptionKind::Pattern,
            ids: patterns.iter().map(|p| u32::from(p.0)).collect(),
        },
    }
}
