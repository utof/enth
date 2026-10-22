//! **Owner: lane MUS** (Music, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The reference is
//! docs/ear.md, "The games" (Rhythm pattern) and "Rhythm input".
//!
//! The 16-step rhythm grid (one bar of sixteenths in 4/4), a pattern as a 16-bit mask, **density**, **swing**, and the
//! **syncopation measure** (Longuet-Higgins and Lee 1984; Fitch and Rosenfeld 2007) that orders the rhythm ladder from
//! low to high syncopation. Integers only. Rhythm pattern is listening only in R4a: pick one of four patterns, then
//! mark it on the grid, scored exactly with no input timing. Tap-back (`taps/1` in `enth-challenge`, after lane TAP's
//! spike) scores inter-onset intervals after a best-fit offset and is not here.
//!
//! **The grid.** Step 0 is the downbeat; steps 0, 4, 8 and 12 are the four beats, 2, 6, 10 and 14 the eighth-note
//! offbeats, and the odd steps the sixteenth offbeats. [`Pattern`] stores step `s` in bit `s` (the least significant
//! bit is the downbeat); its text form is 16 characters, `x` for an onset and `.` for none (`x...x...x.x.....`).
//!
//! **The syncopation measure** is Longuet-Higgins and Lee's, on the metric weights below (0 for the downbeat, -1 for
//! the half bar, -2 for the other beats, -3 for the eighth offbeats and -4 for the sixteenth offbeats): an onset is
//! syncopated when the silence or tie that follows it, up to the next onset, reaches a metrically stronger step than
//! the onset itself, and the value is the difference between the strongest step reached and the onset's. The sum over
//! the bar is the pattern's syncopation; the bar repeats, so the last onsets look past the bar line. It counts steps
//! the sound crosses without a new onset: four on the floor and straight eighths measure 0, a note on step 6 held over
//! the half bar measures 2, and son clave (`x..x..x...x.x...`) and rumba clave (`x..x...x..x.x...`) measure 4 and 5.
//! A pattern with no onsets measures 0 (and the games never draw one).
//!
//! **Swing** delays the second sixteenth of each pair: at `swing_pct` the second step of a pair starts
//! `2 * step_ms * swing_pct / 100` after the pair starts (50 is straight, 66 is triplet swing).

use crate::ear::{NoteSpec, note};
use enth_dsp::util::rng::Rng;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Steps in a bar.
pub const STEPS: u8 = 16;
/// Steps in a beat (a quarter note).
pub const STEPS_PER_BEAT: u8 = 4;
/// Metric weights of the 16 steps (Longuet-Higgins and Lee), strongest 0 down to -4.
pub const WEIGHTS: [i8; 16] = [
    0, -4, -3, -4, -2, -4, -3, -4, -1, -4, -3, -4, -2, -4, -3, -4,
];
/// The straight feel: the second sixteenth of a pair sits exactly half way.
pub const SWING_STRAIGHT: u8 = 50;
/// The largest swing the games use (a dotted feel).
pub const SWING_MAX: u8 = 75;

/// A one-bar pattern on the 16-step grid: bit `s` is set when step `s` has an onset. The JSON form is the mask as a
/// number.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Pattern(pub u16);

impl Pattern {
    /// No onsets (never drawn by the games).
    pub const EMPTY: Self = Self(0);

    /// The pattern with onsets on `steps` (each below 16; others are ignored).
    #[must_use]
    pub const fn from_steps(steps: &[u8]) -> Self {
        let mut mask = 0u16;
        let mut i = 0;
        while i < steps.len() {
            if steps[i] < STEPS {
                mask |= 1 << steps[i];
            }
            i += 1;
        }
        Self(mask)
    }

    /// The pattern of a 16-character grid string, `x` for an onset and `.` for none; `None` for anything else.
    #[must_use]
    pub fn from_grid(grid: &str) -> Option<Self> {
        if grid.len() != usize::from(STEPS) {
            return None;
        }
        let mut mask = 0u16;
        for (s, c) in grid.bytes().enumerate() {
            match c {
                b'x' => mask |= 1 << s,
                b'.' => {}
                _ => return None,
            }
        }
        Some(Self(mask))
    }

    /// The 16-character grid string, `x` for an onset and `.` for none.
    #[must_use]
    pub fn grid(self) -> String {
        (0..STEPS)
            .map(|s| if self.has(s) { 'x' } else { '.' })
            .collect()
    }

    /// Whether step `step` (0 to 15) has an onset.
    #[must_use]
    pub const fn has(self, step: u8) -> bool {
        step < STEPS && (self.0 >> step) & 1 == 1
    }

    /// The number of onsets.
    #[must_use]
    pub const fn density(self) -> u32 {
        self.0.count_ones()
    }

    /// The steps with an onset, ascending.
    pub fn steps(self) -> impl Iterator<Item = u8> {
        (0..STEPS).filter(move |s| self.has(*s))
    }

    /// The number of steps at which the two patterns differ (Hamming distance).
    #[must_use]
    pub const fn distance(self, other: Self) -> u32 {
        (self.0 ^ other.0).count_ones()
    }

    /// The syncopation measure, see the [module docs](self).
    #[must_use]
    pub fn syncopation(self) -> u32 {
        syncopation(self)
    }

    /// The onset times in ms from the start of the bar, with `step_ms` per sixteenth and `swing_pct` swing
    /// ([`onset_ms`]).
    #[must_use]
    pub fn onsets_ms(self, step_ms: u32, swing_pct: u8) -> Vec<u32> {
        self.steps()
            .map(|s| onset_ms(s, step_ms, swing_pct))
            .collect()
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.grid())
    }
}

/// The metric weight of `step` (0 to 15), 0 down to -4.
#[must_use]
pub const fn weight(step: u8) -> i8 {
    WEIGHTS[(step % STEPS) as usize]
}

/// The syncopation of `pattern`: the sum over its onsets of how far the silence or tie after the onset climbs the
/// metric hierarchy above the onset (module docs). 0 for a pattern with no onsets or with every onset on an even
/// step.
#[must_use]
pub fn syncopation(pattern: Pattern) -> u32 {
    let mut total = 0u32;
    for s in pattern.steps() {
        // The strongest step the sound crosses before the next onset (the bar repeats).
        let mut strongest = i8::MIN;
        let mut next = (s + 1) % STEPS;
        while next != s && !pattern.has(next) {
            strongest = strongest.max(weight(next));
            next = (next + 1) % STEPS;
        }
        let gain = i32::from(strongest) - i32::from(weight(s));
        if strongest != i8::MIN && gain > 0 {
            total += gain as u32;
        }
    }
    total
}

/// The length of one sixteenth at `bpm` quarter notes per minute, in whole ms: `15000 / bpm` (150 at 100 bpm).
#[must_use]
pub const fn step_ms_for_bpm(bpm: u32) -> u32 {
    15_000 / bpm
}

/// The time of step `step` (0 to 15) in ms from the start of the bar: `step_ms` per sixteenth, the second step of each
/// pair delayed to `2 * step_ms * swing_pct / 100` after the pair starts (`swing_pct` 50 is straight). Integer
/// arithmetic, rounding down.
#[must_use]
pub const fn onset_ms(step: u8, step_ms: u32, swing_pct: u8) -> u32 {
    let pair_start = (step / 2) as u32 * 2 * step_ms;
    if step.is_multiple_of(2) {
        pair_start
    } else {
        pair_start + 2 * step_ms * swing_pct as u32 / 100
    }
}

/// The length of a bar in ms at `step_ms` per sixteenth.
#[must_use]
pub const fn bar_ms(step_ms: u32) -> u32 {
    STEPS as u32 * step_ms
}

/// The metronome of `bars` bars starting at `start_ms`: a click on each of the 4 beats of each bar, `step_ms` per
/// sixteenth, on `key` at `velocity`, each held `hold_ms`.
#[must_use]
pub fn metronome(
    start_ms: u32,
    bars: u32,
    step_ms: u32,
    key: u8,
    velocity: u8,
    hold_ms: u32,
) -> Vec<NoteSpec> {
    (0..bars * u32::from(STEPS / STEPS_PER_BEAT))
        .map(|beat| {
            note(
                key,
                velocity,
                start_ms + beat * u32::from(STEPS_PER_BEAT) * step_ms,
                hold_ms,
            )
        })
        .collect()
}

/// The notes of `pattern` for one bar starting at `bar_start_ms`, on `key` at `velocity`, each held `hold_ms`, with
/// `step_ms` per sixteenth and `swing_pct` swing.
#[must_use]
pub fn bar_notes(
    pattern: Pattern,
    bar_start_ms: u32,
    step_ms: u32,
    swing_pct: u8,
    key: u8,
    velocity: u8,
    hold_ms: u32,
) -> Vec<NoteSpec> {
    pattern
        .steps()
        .map(|s| {
            note(
                key,
                velocity,
                bar_start_ms + onset_ms(s, step_ms, swing_pct),
                hold_ms,
            )
        })
        .collect()
}

/// What a drawn pattern must satisfy: where onsets may fall, the density and syncopation range, and how far it must be
/// from the patterns already drawn for the same question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternRule {
    /// The steps an onset may take (a mask).
    pub allowed: u16,
    /// The steps that always have an onset (a subset of `allowed`; the downbeat, usually).
    pub anchor: u16,
    /// The fewest and most onsets, inclusive.
    pub density: (u8, u8),
    /// The lowest and highest syncopation, inclusive.
    pub syncopation: (u32, u32),
    /// The least Hamming distance from every pattern already in the question.
    pub min_distance: u32,
}

impl PatternRule {
    /// Whether `pattern` meets the rule against the patterns `others` already drawn.
    #[must_use]
    pub fn accepts(&self, pattern: Pattern, others: &[Pattern]) -> bool {
        let d = pattern.density();
        let s = pattern.syncopation();
        pattern.0 & !self.allowed == 0
            && pattern.0 & self.anchor == self.anchor
            && (u32::from(self.density.0)..=u32::from(self.density.1)).contains(&d)
            && (self.syncopation.0..=self.syncopation.1).contains(&s)
            && others
                .iter()
                .all(|o| pattern.distance(*o) >= self.min_distance)
    }
}

/// Draw one pattern meeting `rule` against the patterns `others` already drawn, or `None` if none exists. One
/// `next_u32` from `rng` picks a start among the subsets of the free steps, and a deterministic scan through the subsets
/// (in counting order, wrapping) finds the first that the rule accepts, so the draw ends after at most `2^free` checks.
pub fn try_draw_pattern(rng: &mut Rng, rule: &PatternRule, others: &[Pattern]) -> Option<Pattern> {
    let free = rule.allowed & !rule.anchor;
    let mut cand = (rng.next_u32() as u16) & free;
    for _ in 0..(1u32 << free.count_ones()) {
        let p = Pattern(cand | rule.anchor);
        if rule.accepts(p, others) {
            return Some(p);
        }
        // The next subset of `free` in counting order (wraps to 0 after the full set).
        cand = (cand | !free).wrapping_add(1) & free;
    }
    None
}

/// How many times [`draw_patterns`] starts over when a draw cannot be completed.
pub const MAX_REDRAWS: u32 = 32;

/// Draw `n` patterns, each accepted by `rule` against the ones before it. A greedy draw can paint itself into a corner
/// (the first patterns leave none at the required distance), so a failed attempt is thrown away and the draw starts
/// over, up to [`MAX_REDRAWS`] times; each attempt draws from `rng` in turn, so the outcome is a function of the seed.
/// If every attempt fails (a rule that cannot be met; the ladders' tests check theirs never do) the last attempt is
/// padded with the anchor pattern.
pub fn draw_patterns(rng: &mut Rng, rule: &PatternRule, n: usize) -> Vec<Pattern> {
    let mut out: Vec<Pattern> = Vec::with_capacity(n);
    for _ in 0..MAX_REDRAWS {
        out.clear();
        while out.len() < n {
            match try_draw_pattern(rng, rule, &out) {
                Some(p) => out.push(p),
                None => break,
            }
        }
        if out.len() == n {
            return out;
        }
    }
    out.resize(n, Pattern(rule.anchor));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(grid: &str) -> Pattern {
        Pattern::from_grid(grid).unwrap()
    }

    #[test]
    fn the_grid_text_round_trips() {
        assert_eq!(p("x...x...x.x.....").0, 0x0511);
        assert_eq!(p("x...x...x.x.....").grid(), "x...x...x.x.....");
        assert_eq!(p("x...............").0, 1);
        assert_eq!(p("...............x").0, 0x8000);
        assert_eq!(p("x...x...x.x.....").to_string(), "x...x...x.x.....");
        assert_eq!(Pattern::from_grid("x..."), None);
        assert_eq!(Pattern::from_grid("x...............x"), None);
        assert_eq!(Pattern::from_grid("x..?............"), None);
        assert_eq!(
            Pattern::from_steps(&[0, 4, 8, 16, 99]),
            p("x...x...x.......")
        );
        assert_eq!(
            p("x...x...x.x.....").steps().collect::<Vec<_>>(),
            [0, 4, 8, 10]
        );
        assert_eq!(p("x...x...x.x.....").density(), 4);
        assert_eq!(p("x...x...x.x.....").distance(p("x...............")), 3);
    }

    #[test]
    fn the_weights_are_the_metric_hierarchy() {
        for s in 0..STEPS {
            let want = match s {
                0 => 0,
                8 => -1,
                4 | 12 => -2,
                2 | 6 | 10 | 14 => -3,
                _ => -4,
            };
            assert_eq!(weight(s), want, "step {s}");
        }
        assert_eq!(weight(16), 0, "wraps");
    }

    #[test]
    fn syncopation_counts_the_strongest_step_a_sound_crosses() {
        // Four on the floor and straight eighths cross nothing stronger than the onset that holds them.
        assert_eq!(syncopation(p("x...x...x...x...")), 0);
        assert_eq!(syncopation(p("x.x.x.x.x.x.x.x.")), 0);
        assert_eq!(syncopation(Pattern::EMPTY), 0);
        // An eighth-note offbeat held over the half bar (step 6 over step 8) is syncopated even on the even grid.
        assert_eq!(syncopation(p("x...x.x...x.x.x.")), 2);
        // A note on beat 2 held through the half bar (4 over 8).
        assert_eq!(syncopation(p("x...x...........")), 1);
        // An offbeat held across the beat after it.
        assert_eq!(
            syncopation(p("x.x.x.x.x.x.x.xx")),
            0,
            "an onset on 15 sits right before the downbeat onset"
        );
        assert!(syncopation(p("x.....x.........")) > 0);
        // Clave patterns: the measure's values for the two classics (Toussaint's tabulated 4 and 5).
        assert_eq!(syncopation(p("x..x..x...x.x...")), 4, "son clave");
        assert_eq!(syncopation(p("x..x...x..x.x...")), 5, "rumba clave");
        // A single onset looks past the bar line to the strongest step of the other 15.
        assert_eq!(
            syncopation(p("..x.............")),
            3,
            "w(2) = -3 and the downbeat is 0"
        );
        assert_eq!(syncopation(p("x...............")), 0);
    }

    #[test]
    fn syncopation_never_decreases_when_an_onset_moves_off_the_beat() {
        // The beat 0..4..8..12 shifted to the sixteenth before each beat is more syncopated than on the beat.
        let on = p("x...x...x...x...");
        let off = p("...x...x...x...x");
        assert!(syncopation(off) > syncopation(on));
        // A bar of all sixteenths has nothing to cross.
        assert_eq!(syncopation(Pattern(0xFFFF)), 0);
    }

    #[test]
    fn timing_is_exact_integers() {
        assert_eq!(step_ms_for_bpm(100), 150);
        assert_eq!(step_ms_for_bpm(120), 125);
        assert_eq!(bar_ms(150), 2400);
        // Straight: every step on the grid.
        for s in 0..STEPS {
            assert_eq!(onset_ms(s, 150, SWING_STRAIGHT), u32::from(s) * 150);
        }
        // Triplet swing (66 %): the second sixteenth of each pair at two thirds (rounded down) of the pair.
        assert_eq!(onset_ms(0, 150, 66), 0);
        assert_eq!(onset_ms(1, 150, 66), 198);
        assert_eq!(onset_ms(2, 150, 66), 300);
        assert_eq!(onset_ms(3, 150, 66), 498);
        assert_eq!(onset_ms(15, 150, 66), 14 * 150 + 198);
        assert_eq!(p("x...x...........").onsets_ms(150, 50), [0, 600]);
        // Even at the largest swing a step stays inside its pair.
        for s in (1..STEPS).step_by(2) {
            assert!(onset_ms(s, 150, SWING_MAX) < u32::from(s + 1) * 150);
            assert!(onset_ms(s, 150, SWING_MAX) >= onset_ms(s, 150, SWING_STRAIGHT));
        }
    }
}
