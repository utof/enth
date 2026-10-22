//! **Owner: lane DRL.** The 3-down/1-up staircase (docs/ear.md, "Difficulty and the staircase").
//!
//! A level runs from 1 (the easiest row of a game's ladder) to the ladder's length; a higher level is harder. After
//! [`DOWN`] right answers in a row the next item is one level harder, and after one wrong answer it is one level
//! easier (Levitt 1971): the track settles where the chance of a right answer is `0.5^(1/3)`, about 79 %, which is the
//! rate the research settled on for training (docs/ear.md; Recreate keeps its 2-down/1-up, about 71 %).
//!
//! - **No visible streak.** The run of right answers that makes the next move is private: nothing reads it, the Ear
//!   view cannot show it and no progress file keeps it. Only the level and the moves it makes are public.
//! - **Pure**: no clock, no RNG. The state is a function of the start and the answers, so a session replays from its
//!   answer log ([`Staircase::replay`]).
//! - **A reversal** is where the track turns, from getting harder to getting easier or back; the level it turned at
//!   is kept. A move that the ends of the ladder swallow (a right run at the top, a wrong answer at the bottom) is
//!   [`Move::Held`], resets the run and is no turn.
//! - **The estimate** ([`Staircase::estimate`]) is the mean level of the last reversals (an even number, at most
//!   twelve), the usual rule for a 3-down/1-up track. A 40-trial track turns about ten times, so most of them count.
//!   Against simulated listeners whose ladder steps are about a just-noticeable difference apart it is within one
//!   level of the true threshold in about nine runs in ten; `tests/drill.rs` states the listeners and the worst
//!   case. A flatter ladder cannot do better with 40 trials, whatever the rule: the data are not there.

/// Right answers in a row that make the next item harder.
pub const DOWN: u8 = 3;

/// The most levels a staircase can have (a ladder is 8 to 15 rows).
pub const MAX_LEVELS: u8 = 32;

/// The fewest reversals that give an estimate.
pub const MIN_REVERSALS: usize = 4;

/// The most reversals an estimate averages (an even number, the last ones): a 40-trial track turns about ten times.
pub const MAX_AVERAGED: usize = 12;

/// What one answer did to the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// The next item is one level harder (3 right in a row).
    Harder,
    /// The next item is one level easier (a wrong answer).
    Easier,
    /// The level stays: a right answer that is not the third in a row, or a move the ladder's end swallowed.
    Held,
}

/// Which way the track last went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    Up,
    Down,
}

/// A 3-down/1-up staircase over levels `1..=levels` (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staircase {
    levels: u8,
    level: u8,
    /// Right answers since the last move: private, so there is no streak to show.
    run: u8,
    way: Option<Way>,
    reversals: Vec<u8>,
    trials: u32,
    right: u32,
    peak: u8,
}

impl Staircase {
    /// A staircase over `levels` rows (clamped to 1 to [`MAX_LEVELS`]) that starts at `start` (clamped into them).
    pub fn new(levels: u8, start: u8) -> Staircase {
        let levels = levels.clamp(1, MAX_LEVELS);
        let level = start.clamp(1, levels);
        Staircase {
            levels,
            level,
            run: 0,
            way: None,
            reversals: Vec::new(),
            trials: 0,
            right: 0,
            peak: level,
        }
    }

    /// The staircase after `answers`, from `start`: the same state as answering them one by one.
    pub fn replay(levels: u8, start: u8, answers: impl IntoIterator<Item = bool>) -> Staircase {
        let mut s = Staircase::new(levels, start);
        for correct in answers {
            s.answer(correct);
        }
        s
    }

    /// The level of the next item, 1 (easiest) to [`levels`](Self::levels).
    pub fn level(&self) -> u8 {
        self.level
    }

    /// How many levels the ladder has.
    pub fn levels(&self) -> u8 {
        self.levels
    }

    /// The hardest level the track has reached.
    pub fn peak(&self) -> u8 {
        self.peak
    }

    /// Answers so far.
    pub fn trials(&self) -> u32 {
        self.trials
    }

    /// Right answers so far.
    pub fn right(&self) -> u32 {
        self.right
    }

    /// The levels the track turned at, in order.
    pub fn reversals(&self) -> &[u8] {
        &self.reversals
    }

    /// Take an answer to the item at [`level`](Self::level): 3 right in a row make the next item harder, a wrong one
    /// makes it easier.
    pub fn answer(&mut self, correct: bool) -> Move {
        self.trials = self.trials.saturating_add(1);
        if correct {
            self.right = self.right.saturating_add(1);
            self.run += 1;
            if self.run < DOWN {
                return Move::Held;
            }
            self.run = 0;
            self.step(Way::Up)
        } else {
            self.run = 0;
            self.step(Way::Down)
        }
    }

    /// Move one level in `way` (up is harder), noting a reversal when the track turns.
    fn step(&mut self, way: Way) -> Move {
        let to = match way {
            Way::Up => self.level.saturating_add(1).min(self.levels),
            Way::Down => self.level.saturating_sub(1).max(1),
        };
        if to == self.level {
            return Move::Held;
        }
        if self.way.is_some_and(|last| last != way) {
            self.reversals.push(self.level);
        }
        self.way = Some(way);
        self.level = to;
        self.peak = self.peak.max(to);
        match way {
            Way::Up => Move::Harder,
            Way::Down => Move::Easier,
        }
    }

    /// The threshold in levels: the mean level of the last reversals (an even number, at most [`MAX_AVERAGED`]);
    /// `None` until there are [`MIN_REVERSALS`]. The level at which a right answer is about 79 % likely. It is a
    /// fraction of a level; the caller turns it into a physical unit with a [`Ladder`](super::Ladder).
    pub fn estimate(&self) -> Option<f64> {
        if self.reversals.len() < MIN_REVERSALS {
            return None;
        }
        let n = self.reversals.len().min(MAX_AVERAGED) & !1;
        let tail = &self.reversals[self.reversals.len() - n..];
        let sum: u32 = tail.iter().map(|&l| u32::from(l)).sum();
        Some(f64::from(sum) / n as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `answers` (`1` right, `0` wrong) and return the moves.
    fn moves(s: &mut Staircase, answers: &str) -> Vec<Move> {
        answers.chars().map(|c| s.answer(c == '1')).collect()
    }

    #[test]
    fn three_right_make_it_harder_and_one_wrong_makes_it_easier() {
        let mut s = Staircase::new(8, 4);
        assert_eq!(moves(&mut s, "111"), [Move::Held, Move::Held, Move::Harder]);
        assert_eq!(s.level(), 5);
        assert_eq!(moves(&mut s, "0"), [Move::Easier]);
        assert_eq!(s.level(), 4);
        // A wrong answer resets the run: two right, a wrong, two right is no move up.
        let mut s = Staircase::new(8, 4);
        assert!(moves(&mut s, "11011").iter().all(|m| *m != Move::Harder));
        assert_eq!(s.level(), 3);
    }

    #[test]
    fn the_ends_of_the_ladder_swallow_moves() {
        let mut s = Staircase::new(3, 3);
        assert_eq!(moves(&mut s, "111"), [Move::Held; 3]);
        assert_eq!(s.level(), 3, "no level above the top");
        let mut s = Staircase::new(3, 1);
        assert_eq!(moves(&mut s, "0"), [Move::Held]);
        assert_eq!(s.level(), 1, "no level below the bottom");
        assert!(s.reversals().is_empty(), "a swallowed move is no turn");
    }

    #[test]
    fn reversals_are_the_levels_the_track_turns_at() {
        // 4 -> (111) 5 -> (0) 4 turns at 5; -> (111) 5 turns at 4.
        let mut s = Staircase::new(8, 4);
        moves(&mut s, "111");
        moves(&mut s, "0");
        moves(&mut s, "111");
        assert_eq!(s.reversals(), [5, 4]);
        assert_eq!(s.peak(), 5);
        // Two wrong in a row go the same way: no turn.
        moves(&mut s, "00");
        assert_eq!(s.reversals(), [5, 4, 5]);
        assert_eq!(s.level(), 3);
    }

    #[test]
    fn the_estimate_averages_an_even_tail_of_the_last_twelve_reversals() {
        let mut s = Staircase::new(12, 6);
        assert_eq!(s.estimate(), None);
        // A zig-zag between 5 and 6: the first wrong answer starts the track, then each turn is a reversal
        // (5, 6, 5, 6, ...), so k rounds give 2k - 1 of them.
        for _ in 0..2 {
            moves(&mut s, "0");
            moves(&mut s, "111");
        }
        assert_eq!(s.reversals(), [5, 6, 5]);
        assert_eq!(s.estimate(), None, "three reversals are too few");
        moves(&mut s, "0");
        moves(&mut s, "111");
        assert_eq!(s.reversals(), [5, 6, 5, 6, 5]);
        assert_eq!(
            s.estimate(),
            Some(5.5),
            "five reversals: the last four are averaged"
        );
        // The track walks down four levels and zig-zags between 2 and 3: after six rounds the old reversals are
        // out of the last twelve.
        moves(&mut s, "0000");
        for _ in 0..6 {
            moves(&mut s, "111");
            moves(&mut s, "0");
        }
        assert_eq!(s.reversals().len(), 18);
        assert_eq!(s.estimate(), Some(2.5), "only the last twelve count");
    }

    #[test]
    fn replay_is_the_same_as_answering() {
        let answers = "11101110110111011101";
        let mut a = Staircase::new(10, 3);
        for c in answers.chars() {
            a.answer(c == '1');
        }
        assert_eq!(
            Staircase::replay(10, 3, answers.chars().map(|c| c == '1')),
            a
        );
    }

    #[test]
    fn out_of_range_inputs_are_clamped() {
        let s = Staircase::new(0, 0);
        assert_eq!((s.levels(), s.level()), (1, 1));
        let s = Staircase::new(200, 250);
        assert_eq!((s.levels(), s.level()), (MAX_LEVELS, MAX_LEVELS));
    }
}
