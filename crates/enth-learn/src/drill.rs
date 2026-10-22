//! **Owner: lane DRL** (Drill player, freeze point F1e; its API is judged there with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); written by lane DRL. The reference is docs/ear.md ("The drill
//! player", "Difficulty and the staircase") and docs/learn.md, "Ear skills, bursts and drills".
//!
//! The pure **drill player**: N questions at a time on a game's ladder, steered by the **3-down/1-up staircase**
//! ([`Staircase`], about 79 % success) for drills and Ear check, while Recreate keeps its 2-down/1-up. Pure like the
//! rest of the crate: no clock (the shell passes the day), no RNG (every draw is keyed from the seed and the item),
//! no I/O. The **run counter stays private**: there is no visible streak.
//!
//! | Piece | What |
//! | --- | --- |
//! | [`Staircase`] | the level of the next item; 3 right in a row make it harder, 1 wrong makes it easier; reversals and an estimate |
//! | [`Drill`] | one game session: [`next_item`](Drill::next_item) names the level and the keyed seed of the next question, [`answer`](Drill::answer) takes the judged answer, [`summary`](Drill::summary) is what the shell records |
//! | [`Burst`] | five items on one skill, tallied from the answers' skills; it moves that skill's Review box (4 or 5 right up, 3 holds, 2 or fewer down) |
//! | [`Summary`] | the session in numbers: the level reached, success, the threshold estimate in levels (and in a physical unit through a [`Ladder`]), the bursts |
//! | [`Drills`] | progress's `drills` field: each game's level, each skill's accuracy and thresholds over time, the practised days |
//! | [`DrillStep`] | the lesson step that plays a drill (`StepKind::Drill`) |
//!
//! # The shell's loop
//!
//! ```text
//! let mut drill = Drill::new(DrillSpec::new(game, levels, items, seed).starting_at(start).focused_on(skills))?;
//! while let Some(item) = drill.next_item() {
//!     // ask the generator for question `item.index` at `item.level` from `item.seed`
//!     // (enth_music::ear::question(game, item.level, item.seed, item.index)), or start a one-question
//!     // session run seeded `item.run_seed()`; play it; judge the answer
//!     drill.answer(Answer::new(item.index, skill_of_the_question, correct))?;
//! }
//! progress.observe_drill(&drill.summary(), today); // bursts move boxes; levels, accuracy, thresholds, the day
//! ```
//!
//! The session's `EarRequest::Result` carries the skill, the level and whether the answer was right: that is an
//! [`Answer`]. A session can stop at any time; [`Drill::summary`] counts what was answered and a burst that did not
//! reach five items moves nothing.
//!
//! # Three additive edits in files that already exist
//!
//! - [`review`](crate::review): `ear.*` skills ([`Skill::ear`]) and 5-item [`Burst`]s
//!   ([`Review::observe_burst`](crate::review::Review::observe_burst)).
//! - [`lesson`](crate::lesson): `StepKind::Drill` ([`DrillStep`]), declared in a pack's `requires.drill`.
//! - [`progress`](crate::progress): the `drills` field ([`Drills`]) and [`Progress::observe_drill`].
//!
//! [`Progress::observe_drill`]: crate::progress::Progress::observe_drill

mod record;
mod staircase;

use std::collections::{BTreeMap, BTreeSet};

use enth_dsp::util::rng::{derive, fnv1a64};

use crate::review::{BURST_ITEMS, Burst, Day, Skill};

pub use record::{Drills, GameDrill, HISTORY_MAX, Point, SkillDrill};
pub use staircase::{DOWN, MAX_AVERAGED, MAX_LEVELS, MIN_REVERSALS, Move, Staircase};

/// The drill schema a lesson's `StepKind::Drill` needs: what a pack declares in `requires.drill`. An older build
/// refuses such a pack with "needs a newer enth" rather than failing inside a lesson.
pub const DRILL_VERSION: u32 = 1;

/// The ear games' ids, the numbers of `enth-music`'s `EarSpec::game` (1 to 3 are the parameter games, generated in
/// `enth-challenge`; 4 to 10 the music games).
pub mod game {
    /// Spot the change.
    pub const SPOT_THE_CHANGE: u8 = 1;
    /// Dial it in.
    pub const DIAL_IT_IN: u8 = 2;
    /// Ear check.
    pub const EAR_CHECK: u8 = 3;
    /// Frequency finder.
    pub const FREQUENCY_FINDER: u8 = 4;
    /// Interval stacks.
    pub const INTERVAL_STACKS: u8 = 5;
    /// Chord quality.
    pub const CHORD_QUALITY: u8 = 6;
    /// Rhythm pattern.
    pub const RHYTHM_PATTERN: u8 = 7;
    /// R4b's progressions.
    pub const PROGRESSIONS: u8 = 8;
    /// R4b's note naming: opt-in, never in the default workout.
    pub const NOTE_NAMING: u8 = 9;
    /// R4b's tap-back.
    pub const TAP_BACK: u8 = 10;
}

/// The highest game id this build knows.
pub const MAX_GAME: u8 = game::TAP_BACK;

/// The most items in one drill (the ear generator's count is 6 bits).
pub const MAX_ITEMS: u8 = 64;

/// The most skills one drill bursts on at once.
pub const MAX_FOCUS: usize = 4;

/// The largest seed: 2^53 - 1, so a JSON number survives every reader (as the ear generator's).
pub const MAX_SEED: u64 = (1 << 53) - 1;

/// A game not played for more than this many days starts its next session one level below where its staircase
/// ended: a warm-up. No message counts the days.
pub const BREAK_DAYS: u32 = 7;

/// Items in a drill step that does not say (`StepKind::Drill`).
pub const DEFAULT_STEP_ITEMS: u8 = 10;

// ---------------------------------------------------------------------------------------------------------------
// Units and ladders
// ---------------------------------------------------------------------------------------------------------------

/// The unit a threshold is shown in: physical wherever the game has one ("progress as thresholds in physical units").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    /// A level of the game's ladder (the music games have no physical axis): higher is better.
    Level,
    /// Semitones: a cutoff or a pitch difference.
    Semitones,
    /// Cents: a detune.
    Cents,
    /// Milliseconds: an attack, a decay, a delay.
    Ms,
    /// Decibels: a level or an EQ gain.
    Db,
    /// Octaves: a band's place.
    Octaves,
    /// A ratio ("decay changes of × 1.2"), 1 being no change.
    Ratio,
    /// A percentage of a control's range.
    Percent,
}

impl Unit {
    /// Every unit.
    pub const ALL: [Unit; 8] = [
        Unit::Level,
        Unit::Semitones,
        Unit::Cents,
        Unit::Ms,
        Unit::Db,
        Unit::Octaves,
        Unit::Ratio,
        Unit::Percent,
    ];

    /// The word in the progress file.
    pub fn key(self) -> &'static str {
        match self {
            Unit::Level => "level",
            Unit::Semitones => "st",
            Unit::Cents => "ct",
            Unit::Ms => "ms",
            Unit::Db => "db",
            Unit::Octaves => "oct",
            Unit::Ratio => "x",
            Unit::Percent => "pct",
        }
    }

    /// Read a unit's word.
    pub fn from_key(key: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|u| u.key() == key)
    }

    /// What the Ear view prints after a value.
    pub fn label(self) -> &'static str {
        match self {
            Unit::Level => "levels",
            Unit::Semitones => "semitones",
            Unit::Cents => "cents",
            Unit::Ms => "ms",
            Unit::Db => "dB",
            Unit::Octaves => "octaves",
            Unit::Ratio => "×",
            Unit::Percent => "%",
        }
    }

    /// Whether a smaller value is a finer difference heard, and so better: every physical unit. A level is better
    /// the higher it is.
    pub fn better_is_lower(self) -> bool {
        self != Unit::Level
    }
}

/// What a ladder's levels mean in a physical unit: level `n` asks the listener to hear a difference of `values[n - 1]`.
/// A game's ladder (the lanes G1 to G4, measured by `ear-check`) gives it; the drill's threshold estimate, a fraction
/// of a level, becomes a value in the unit by interpolating between the neighbouring levels (linearly, in the
/// ladder's own numbers: give them in the unit the listener thinks in).
#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    unit: Unit,
    values: Vec<f64>,
}

impl Ladder {
    /// A ladder from the difference each level asks, level 1 first (finer differences at the harder levels, so the
    /// numbers fall). `None` for no level or a number that is not finite and non-negative.
    pub fn new(unit: Unit, values: Vec<f64>) -> Option<Ladder> {
        (!values.is_empty() && values.iter().all(|v| v.is_finite() && *v >= 0.0))
            .then_some(Ladder { unit, values })
    }

    /// The identity ladder: a game with no physical axis reports its threshold in levels.
    pub fn levels(count: u8) -> Ladder {
        Ladder {
            unit: Unit::Level,
            values: (1..=count.max(1)).map(f64::from).collect(),
        }
    }

    /// The unit the values are in.
    pub fn unit(&self) -> Unit {
        self.unit
    }

    /// The value at a (fractional) level, clamped to the ends of the ladder.
    pub fn value_at(&self, level: f64) -> f64 {
        let last = (self.values.len() - 1) as f64;
        let at = (level - 1.0).clamp(0.0, last);
        let lo = at.floor();
        let frac = at - lo;
        let a = self.values[lo as usize];
        let b = self.values[(lo as usize + 1).min(self.values.len() - 1)];
        a + (b - a) * frac
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The lesson step
// ---------------------------------------------------------------------------------------------------------------

/// A lesson step that plays a drill (`StepKind::Drill`, declared in a pack's `requires.drill`): a game, optionally a
/// skill to burst on, a start level and a number of items. **Playing it to the end passes the step, whatever the
/// score**: no content is gated on chords, intervals or note naming, and a drill's score feeds the staircase and the
/// boxes, never the lesson.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrillStep {
    /// The game's id (`EarSpec::game`, 1 to [`MAX_GAME`]).
    pub game: u8,
    /// The level the staircase starts at (default 1).
    pub level: u8,
    /// How many items (default [`DEFAULT_STEP_ITEMS`]).
    pub items: u8,
    /// A skill to burst on, when the step is about one.
    pub skill: Option<Skill>,
}

impl DrillStep {
    /// A step playing `game` from level 1 with the default number of items.
    pub fn new(game: u8) -> DrillStep {
        DrillStep {
            game,
            level: 1,
            items: DEFAULT_STEP_ITEMS,
            skill: None,
        }
    }

    /// What is wrong with the step, as a field name and a message: an unknown game, a level or item count out of
    /// range, a burst skill that does not fit the items.
    pub fn problem(&self) -> Option<(&'static str, String)> {
        if !(1..=MAX_GAME).contains(&self.game) {
            return Some((
                "game",
                format!(
                    "game {} is not an ear game of this build (1 to {MAX_GAME})",
                    self.game
                ),
            ));
        }
        if !(1..=MAX_LEVELS).contains(&self.level) {
            return Some(("level", format!("is 1 to {MAX_LEVELS}")));
        }
        if !(1..=MAX_ITEMS).contains(&self.items) {
            return Some(("items", format!("is 1 to {MAX_ITEMS}")));
        }
        if self.skill.is_some() && self.items < BURST_ITEMS {
            return Some((
                "items",
                format!("a step that bursts on a skill asks at least {BURST_ITEMS} items"),
            ));
        }
        None
    }

    /// The drill this step plays, on a ladder of `levels` rows, with the ear generator `generator` and `seed`
    /// (clamped to the seed range). The step's level is the start, clamped to the ladder.
    pub fn spec(&self, generator: u8, levels: u8, seed: u64) -> DrillSpec {
        DrillSpec {
            game: self.game,
            generator,
            seed: seed & MAX_SEED,
            items: self.items,
            levels,
            start: self.level.min(levels.max(1)),
            focus: self.skill.iter().cloned().collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The drill
// ---------------------------------------------------------------------------------------------------------------

/// Why a drill refused something.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrillError {
    /// The spec is out of range; the message names the field.
    BadSpec(&'static str),
    /// Every item is answered.
    Done,
    /// An answer for an item that is not the open one (a stale result): refused, harmlessly.
    Stale {
        /// The item that is open.
        expected: u32,
        /// The item the answer was for.
        got: u32,
    },
}

impl std::fmt::Display for DrillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrillError::BadSpec(what) => write!(f, "a drill's {what}"),
            DrillError::Done => f.write_str("every item of the drill is answered"),
            DrillError::Stale { expected, got } => {
                write!(f, "item {got} is not the open one ({expected})")
            }
        }
    }
}

impl std::error::Error for DrillError {}

/// What a drill is asked for. The generator, the game and the seed are the ear generator's: item `i` of the drill is
/// `question(game, level, seed, i)` at the level the staircase chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrillSpec {
    /// The game's id (`EarSpec::game`, 1 to [`MAX_GAME`]).
    pub game: u8,
    /// The ear generator's version, echoed in every [`Item`].
    pub generator: u8,
    /// The seed every item is keyed from, at most [`MAX_SEED`].
    pub seed: u64,
    /// How many items (1 to [`MAX_ITEMS`]).
    pub items: u8,
    /// How many levels the game's ladder has in this build.
    pub levels: u8,
    /// The level the staircase starts at (1 to `levels`).
    pub start: u8,
    /// The skills to burst on: items are asked for them round-robin, five each, then free ones fill the rest.
    pub focus: Vec<Skill>,
}

impl DrillSpec {
    /// A drill of `items` questions on `game`'s ladder of `levels` rows, from level 1, generator 1, with no focus.
    pub fn new(game: u8, levels: u8, items: u8, seed: u64) -> DrillSpec {
        DrillSpec {
            game,
            generator: 1,
            seed,
            items,
            levels,
            start: 1,
            focus: Vec::new(),
        }
    }

    /// Start the staircase at `level`.
    #[must_use]
    pub fn starting_at(mut self, level: u8) -> DrillSpec {
        self.start = level;
        self
    }

    /// Burst on these skills.
    #[must_use]
    pub fn focused_on(mut self, skills: Vec<Skill>) -> DrillSpec {
        self.focus = skills;
        self
    }

    /// Use ear generator `generator`.
    #[must_use]
    pub fn with_generator(mut self, generator: u8) -> DrillSpec {
        self.generator = generator;
        self
    }

    /// Check the spec.
    pub fn validate(&self) -> Result<(), DrillError> {
        let bad = |what| Err(DrillError::BadSpec(what));
        if !(1..=MAX_GAME).contains(&self.game) {
            return bad("game is not an ear game of this build");
        }
        if self.generator == 0 {
            return bad("generator is 1 or more");
        }
        if self.seed > MAX_SEED {
            return bad("seed is below 2^53");
        }
        if !(1..=MAX_ITEMS).contains(&self.items) {
            return bad("items are 1 to 64");
        }
        if !(1..=MAX_LEVELS).contains(&self.levels) {
            return bad("ladder has 1 to 32 levels");
        }
        if !(1..=self.levels).contains(&self.start) {
            return bad("start level is on the ladder");
        }
        if self.focus.len() > MAX_FOCUS {
            return bad("focus is at most 4 skills");
        }
        let distinct: BTreeSet<&Skill> = self.focus.iter().collect();
        if distinct.len() != self.focus.len() {
            return bad("focus names a skill twice");
        }
        if self.focus.len() * usize::from(BURST_ITEMS) > usize::from(self.items) {
            return bad("items hold five per focus skill");
        }
        Ok(())
    }

    /// The skill item `index` is asked for, if any: the focus skills in turn for the first five of each.
    fn focus_for(&self, index: u32) -> Option<&Skill> {
        let burst_items = self.focus.len() as u32 * u32::from(BURST_ITEMS);
        (index < burst_items).then(|| &self.focus[index as usize % self.focus.len()])
    }
}

/// The next question to ask: which one, at what level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Its number in the drill (0 is the first).
    pub index: u32,
    /// The game's id.
    pub game: u8,
    /// The ear generator's version.
    pub generator: u8,
    /// The staircase's level for it: the ladder row the generator draws from.
    pub level: u8,
    /// The drill's seed: the question is `question(game, level, seed, index)`.
    pub seed: u64,
    /// The skill the drill wants this item to be about (a burst's), when the game can give it.
    pub focus: Option<Skill>,
}

impl Item {
    /// A seed for a one-question run of the session (`EarPlan { count: 1 }`, whose item index is 0): keyed from the
    /// seed and the item's number, so two items never share a draw.
    pub fn run_seed(&self) -> u64 {
        derive(
            self.seed,
            fnv1a64(format!("item/{}", self.index).as_bytes()),
        ) & MAX_SEED
    }
}

/// A judged answer, the drill's input (the session's `EarRequest::Result` has the fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The item answered: the open one, or the answer is stale.
    pub item: u32,
    /// The skill the question was about (`ear.interval.M3`, `svf.cutoff`); `None` for a question with none. This
    /// is the question's own skill, which may differ from the focus the item asked for.
    pub skill: Option<Skill>,
    /// The answer matched the key.
    pub correct: bool,
}

impl Answer {
    /// A judged answer to item `item`.
    pub fn new(item: u32, skill: Option<Skill>, correct: bool) -> Answer {
        Answer {
            item,
            skill,
            correct,
        }
    }
}

/// Right answers and answers, for one skill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Right answers.
    pub right: u32,
    /// Answers.
    pub total: u32,
}

/// A threshold in a physical unit, measured by a drill ([`Summary::measuring`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The skill the threshold is of.
    pub skill: Skill,
    /// The value, in thousandths of its unit.
    pub point: Point,
}

/// A drill in numbers: what the shell records ([`Progress::observe_drill`](crate::progress::Progress::observe_drill))
/// and the Ear view shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// The game.
    pub game: u8,
    /// Items answered.
    pub items: u32,
    /// Right answers.
    pub right: u32,
    /// The level the staircase started at.
    pub start_level: u8,
    /// The level it ended at (where the game's next session starts).
    pub end_level: u8,
    /// The hardest level it reached.
    pub peak_level: u8,
    /// How many times the track turned.
    pub reversals: u16,
    /// The threshold in levels (a fraction), when the track turned often enough ([`Staircase::estimate`]).
    pub threshold: Option<f64>,
    /// The bursts that reached five items, in the order they completed.
    pub bursts: Vec<Burst>,
    /// Every skill answered, with its right answers and answers.
    pub skills: BTreeMap<Skill, Tally>,
    /// A threshold in a physical unit, when the shell measured one ([`measuring`](Self::measuring)).
    pub measured: Option<Measured>,
}

impl Summary {
    /// The share of right answers in percent (0 for no answers). The Ear view shows it per session; the staircase
    /// holds it near 79.
    pub fn success_pct(&self) -> u32 {
        self.right
            .saturating_mul(100)
            .saturating_add(self.items / 2)
            .checked_div(self.items)
            .unwrap_or(0)
    }

    /// Record the threshold of `skill` in the unit of `ladder`: the estimate in levels, turned into a value, when
    /// the drill has one (an Ear check on one skill). With no estimate nothing is measured and the summary is
    /// returned as it was.
    #[must_use]
    pub fn measuring(mut self, skill: Skill, ladder: &Ladder) -> Summary {
        if let Some(level) = self.threshold {
            self.measured = Some(Measured {
                skill,
                point: Point::from_value(ladder.unit(), ladder.value_at(level)),
            });
        }
        self
    }
}

/// One game session. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Drill {
    spec: DrillSpec,
    stair: Staircase,
    answered: u32,
    right: u32,
    skills: BTreeMap<Skill, Tally>,
    /// Bursts in progress by skill: right answers and answers so far; one ends at [`BURST_ITEMS`] answers.
    open_bursts: BTreeMap<Skill, (u8, u8)>,
    bursts: Vec<Burst>,
    /// Skills whose burst is finished: a skill bursts once a session, so a long session on one skill cannot climb
    /// several boxes in a sitting.
    done: BTreeSet<Skill>,
}

impl Drill {
    /// Start a drill (`Err` when the spec is out of range).
    pub fn new(spec: DrillSpec) -> Result<Drill, DrillError> {
        spec.validate()?;
        let stair = Staircase::new(spec.levels, spec.start);
        Ok(Drill {
            spec,
            stair,
            answered: 0,
            right: 0,
            skills: BTreeMap::new(),
            open_bursts: BTreeMap::new(),
            bursts: Vec::new(),
            done: BTreeSet::new(),
        })
    }

    /// The drill after `answers`: the same state as answering them one by one (a session is its spec and its answer
    /// log).
    pub fn replay(
        spec: DrillSpec,
        answers: impl IntoIterator<Item = Answer>,
    ) -> Result<Drill, DrillError> {
        let mut d = Drill::new(spec)?;
        for a in answers {
            d.answer(a)?;
        }
        Ok(d)
    }

    /// What the drill was asked for.
    pub fn spec(&self) -> &DrillSpec {
        &self.spec
    }

    /// Items answered so far.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// Whether every item is answered.
    pub fn is_done(&self) -> bool {
        self.answered >= u32::from(self.spec.items)
    }

    /// The level of the next item.
    pub fn level(&self) -> u8 {
        self.stair.level()
    }

    /// The open item: the same until it is answered. `None` once every item is answered.
    pub fn next_item(&self) -> Option<Item> {
        if self.is_done() {
            return None;
        }
        Some(Item {
            index: self.answered,
            game: self.spec.game,
            generator: self.spec.generator,
            level: self.stair.level(),
            seed: self.spec.seed,
            focus: self.spec.focus_for(self.answered).cloned(),
        })
    }

    /// Take the judged answer to the open item. The staircase moves, the answer joins its skill's tally and the
    /// skill's burst; the move is returned for the view's replay line, never as a count.
    pub fn answer(&mut self, answer: Answer) -> Result<Move, DrillError> {
        if self.is_done() {
            return Err(DrillError::Done);
        }
        if answer.item != self.answered {
            return Err(DrillError::Stale {
                expected: self.answered,
                got: answer.item,
            });
        }
        self.answered += 1;
        self.right += u32::from(answer.correct);
        let moved = self.stair.answer(answer.correct);
        if let Some(skill) = answer.skill {
            let t = self.skills.entry(skill.clone()).or_default();
            t.total += 1;
            t.right += u32::from(answer.correct);
            if !self.done.contains(&skill) {
                let b = self.open_bursts.entry(skill.clone()).or_default();
                b.1 += 1;
                b.0 += u8::from(answer.correct);
                if b.1 >= BURST_ITEMS {
                    let right = b.0;
                    self.open_bursts.remove(&skill);
                    self.done.insert(skill.clone());
                    self.bursts.push(Burst::new(skill, right));
                }
            }
        }
        Ok(moved)
    }

    /// The session in numbers, at any time (a session that stopped early counts what was answered; a burst short of
    /// five items is dropped, since fewer answers are too guessable to move a box).
    pub fn summary(&self) -> Summary {
        Summary {
            game: self.spec.game,
            items: self.answered,
            right: self.right,
            start_level: self.spec.start,
            end_level: self.stair.level(),
            peak_level: self.stair.peak(),
            reversals: u16::try_from(self.stair.reversals().len()).unwrap_or(u16::MAX),
            threshold: self.stair.estimate(),
            bursts: self.bursts.clone(),
            skills: self.skills.clone(),
            measured: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Starting a session
// ---------------------------------------------------------------------------------------------------------------

/// Where a game's next session starts on `today`, and whether that is a warm-up: the level its staircase ended at,
/// or **one level below it after a break** (more than [`BREAK_DAYS`] since the game was last played). A game never
/// played starts at level 1, and no start is above `levels`. The warm-up is silent: no message counts the days.
pub fn start_level(drills: &Drills, game: u8, levels: u8, today: Day) -> (u8, bool) {
    let g = drills.game(game);
    let top = levels.max(1);
    if g.sessions == 0 {
        return (1, false);
    }
    let after_break = today.saturating_sub(g.day) > BREAK_DAYS;
    let level = if after_break {
        g.level.saturating_sub(1).max(1)
    } else {
        g.level
    };
    (level.min(top), after_break && g.level > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(text: &str) -> Skill {
        Skill::parse(text).unwrap()
    }

    #[test]
    fn units_have_one_word_each() {
        for u in Unit::ALL {
            assert_eq!(Unit::from_key(u.key()), Some(u));
        }
        assert_eq!(Unit::from_key("furlong"), None);
        assert!(Unit::Semitones.better_is_lower() && !Unit::Level.better_is_lower());
    }

    #[test]
    fn a_ladder_interpolates_and_clamps() {
        let l = Ladder::new(Unit::Semitones, vec![8.0, 4.0, 2.0, 1.0]).unwrap();
        assert_eq!(l.value_at(1.0), 8.0);
        assert_eq!(l.value_at(1.5), 6.0);
        assert_eq!(l.value_at(3.0), 2.0);
        assert_eq!(l.value_at(0.0), 8.0, "clamped below");
        assert_eq!(l.value_at(9.0), 1.0, "clamped above");
        assert!(Ladder::new(Unit::Ms, vec![]).is_none());
        assert!(Ladder::new(Unit::Ms, vec![1.0, f64::NAN]).is_none());
        assert_eq!(Ladder::levels(5).value_at(3.5), 3.5);
    }

    #[test]
    fn a_drill_steps_through_its_items_and_refuses_stale_answers() {
        let spec = DrillSpec::new(game::INTERVAL_STACKS, 8, 4, 77).starting_at(3);
        let mut d = Drill::new(spec).unwrap();
        let first = d.next_item().unwrap();
        assert_eq!((first.index, first.level, first.game), (0, 3, 5));
        assert_eq!(d.next_item().unwrap(), first, "the open item stays open");
        assert_eq!(
            d.answer(Answer::new(1, None, true)),
            Err(DrillError::Stale {
                expected: 0,
                got: 1
            })
        );
        for i in 0..4 {
            assert_eq!(d.next_item().unwrap().index, i);
            d.answer(Answer::new(i, None, true)).unwrap();
        }
        assert!(d.is_done() && d.next_item().is_none());
        assert_eq!(d.answer(Answer::new(4, None, true)), Err(DrillError::Done));
        let s = d.summary();
        assert_eq!((s.items, s.right, s.success_pct()), (4, 4, 100));
        assert_eq!(
            (s.start_level, s.end_level),
            (3, 4),
            "3 right in a row went up"
        );
    }

    #[test]
    fn five_items_on_a_skill_make_one_burst() {
        let m3 = skill("ear.interval.m3");
        let maj3 = skill("ear.interval.M3");
        let spec = DrillSpec::new(game::INTERVAL_STACKS, 8, 12, 5)
            .focused_on(vec![m3.clone(), maj3.clone()]);
        let mut d = Drill::new(spec).unwrap();
        let mut asked = Vec::new();
        while let Some(item) = d.next_item() {
            asked.push(item.focus.clone());
            // m3 always right; M3 right twice in five.
            let (s, correct) = match item.focus.clone() {
                Some(s) if s == m3 => (Some(s), true),
                Some(s) => {
                    let n = asked.iter().filter(|f| **f == Some(maj3.clone())).count();
                    (Some(s), n <= 2)
                }
                None => (None, true),
            };
            d.answer(Answer::new(item.index, s, correct)).unwrap();
        }
        // Round-robin for the first ten items, then free ones.
        assert_eq!(asked[0], Some(m3.clone()));
        assert_eq!(asked[1], Some(maj3.clone()));
        assert_eq!(asked[10], None);
        let s = d.summary();
        assert_eq!(
            s.bursts,
            vec![Burst::new(m3.clone(), 5), Burst::new(maj3.clone(), 2)],
            "in the order they completed"
        );
        assert_eq!(s.skills[&m3], Tally { right: 5, total: 5 });
    }

    #[test]
    fn a_skill_bursts_once_a_session_and_a_short_burst_moves_nothing() {
        let s = skill("svf.cutoff");
        let mut d = Drill::new(DrillSpec::new(game::EAR_CHECK, 8, 12, 1)).unwrap();
        for i in 0..4 {
            d.answer(Answer::new(i, Some(s.clone()), true)).unwrap();
        }
        assert!(d.summary().bursts.is_empty(), "four answers are no burst");
        for i in 4..12 {
            d.answer(Answer::new(i, Some(s.clone()), true)).unwrap();
        }
        let sum = d.summary();
        assert_eq!(sum.bursts, [Burst::new(s.clone(), 5)], "one burst, not two");
        assert_eq!(
            sum.skills[&s].total, 12,
            "every answer still counts for accuracy"
        );
    }

    #[test]
    fn specs_are_checked() {
        let ok = DrillSpec::new(5, 8, 10, 1);
        assert!(ok.validate().is_ok());
        for (bad, what) in [
            (
                DrillSpec {
                    game: 0,
                    ..ok.clone()
                },
                "game",
            ),
            (
                DrillSpec {
                    game: 11,
                    ..ok.clone()
                },
                "game",
            ),
            (
                DrillSpec {
                    items: 0,
                    ..ok.clone()
                },
                "items",
            ),
            (
                DrillSpec {
                    items: 65,
                    ..ok.clone()
                },
                "items",
            ),
            (
                DrillSpec {
                    levels: 0,
                    ..ok.clone()
                },
                "ladder",
            ),
            (
                DrillSpec {
                    start: 9,
                    ..ok.clone()
                },
                "start",
            ),
            (
                DrillSpec {
                    seed: MAX_SEED + 1,
                    ..ok.clone()
                },
                "seed",
            ),
            (
                DrillSpec {
                    generator: 0,
                    ..ok.clone()
                },
                "generator",
            ),
        ] {
            let Err(DrillError::BadSpec(msg)) = bad.validate() else {
                panic!("{what}");
            };
            assert!(msg.contains(what), "{msg} / {what}");
        }
        let two = vec![skill("ear.chord.min"), skill("ear.chord.maj")];
        assert!(ok.clone().focused_on(two.clone()).validate().is_ok());
        assert!(
            DrillSpec::new(5, 8, 9, 1)
                .focused_on(two.clone())
                .validate()
                .is_err(),
            "five per focus skill"
        );
        assert!(
            ok.clone()
                .focused_on(vec![two[0].clone(), two[0].clone()])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_drill_replays_from_its_answer_log() {
        let spec = DrillSpec::new(game::CHORD_QUALITY, 6, 20, 9).starting_at(2);
        let mut live = Drill::new(spec.clone()).unwrap();
        let mut log = Vec::new();
        for i in 0..20 {
            let a = Answer::new(i, Some(skill("ear.chord.min")), i % 4 != 3);
            live.answer(a.clone()).unwrap();
            log.push(a);
        }
        assert_eq!(Drill::replay(spec, log).unwrap(), live);
    }

    #[test]
    fn run_seeds_differ_per_item_and_stay_in_range() {
        let d = Drill::new(DrillSpec::new(5, 8, 3, MAX_SEED)).unwrap();
        let a = d.next_item().unwrap();
        let mut d2 = d.clone();
        d2.answer(Answer::new(0, None, true)).unwrap();
        let b = d2.next_item().unwrap();
        assert_ne!(a.run_seed(), b.run_seed());
        assert!(a.run_seed() <= MAX_SEED && b.run_seed() <= MAX_SEED);
    }

    #[test]
    fn a_break_starts_one_level_below_and_says_nothing() {
        let mut drills = Drills::default();
        assert_eq!(start_level(&drills, 5, 8, 100), (1, false), "never played");
        let mut s = DrillSpec::new(5, 8, 3, 1).starting_at(4);
        s.focus.clear();
        let mut d = Drill::new(s).unwrap();
        for i in 0..3 {
            d.answer(Answer::new(i, None, true)).unwrap();
        }
        let sum = d.summary();
        assert_eq!(sum.end_level, 5);
        drills.observe(&sum, 100);
        assert_eq!(start_level(&drills, 5, 8, 101), (5, false));
        assert_eq!(
            start_level(&drills, 5, 8, 100 + BREAK_DAYS),
            (5, false),
            "a week is no break"
        );
        assert_eq!(start_level(&drills, 5, 8, 100 + BREAK_DAYS + 1), (4, true));
        assert_eq!(
            start_level(&drills, 5, 3, 101),
            (3, false),
            "never above the ladder"
        );
    }

    #[test]
    fn steps_are_checked() {
        let mut s = DrillStep::new(5);
        assert_eq!(s.problem(), None);
        s.game = 0;
        assert_eq!(s.problem().unwrap().0, "game");
        s.game = 5;
        s.items = 3;
        s.skill = Some(skill("ear.interval.M3"));
        assert_eq!(s.problem().unwrap().0, "items");
        s.items = 10;
        s.level = 40;
        assert_eq!(s.problem().unwrap().0, "level");
        s.level = 2;
        assert_eq!(s.problem(), None);
        let spec = s.spec(1, 8, MAX_SEED + 5);
        assert!(spec.validate().is_ok(), "{spec:?}");
        assert_eq!(spec.focus, [skill("ear.interval.M3")]);
    }
}
