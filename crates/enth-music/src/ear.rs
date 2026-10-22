//! **Owner: lane MUS** (Music, freeze point F1e; the F1e judge fixes the real types). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e). The reference is docs/ear.md, "The ear generator".
//!
//! **Ear generator v1**: an [`EarSpec`] (game, generator, level, seed, count; the seed below 2^53) becomes `count`
//! [`Question`]s. A question is integers all the way down: notes (key, velocity, start and hold in ms), a clip length,
//! a timbre id into `enth-challenge`'s frozen pool, an optional post-EQ band, an answer (a choice index or tap times)
//! and its options. Nothing here is a float, so a question is the same on every platform and engine version.
//!
//! ```text
//! EarSpec  { game: u8, generator: u8, level: u8, seed: u64, count: u8 }
//! Question { timbre, notes: Vec<NoteSpec>, length_ms, post: Option<EqBand>, answer: Answer, options: Options }
//! Answer   { Choice(u8) | Taps { onsets_ms: Vec<u32>, window_ms: u16 } }
//! ```
//!
//! - **Keyed streams.** Item `i` of a game at a level draws from `derive(seed, fnv1a64("<game key>/<i>/<level>"))`
//!   (for example `"interval/7/1"`), so adding a game or an item never moves another's draw, and a staircase or a
//!   fixed ladder can ask for any item at any level without generating the others ([`question`]). A practice session is
//!   (game, seed, settings) plus the answer log. Each game's draw order is written at the top of its ladder file; it is
//!   part of the frozen form.
//! - **Valid by construction.** Every question stays within C1 to C7, has at most 8 notes sounding at once and lasts at
//!   most 10 s, and a choice answer indexes its options ([`Question::validate`]; the proptests run it on every game
//!   and level).
//! - **Options stay put.** A label game lists its options in ascending order of their id (the interval size, the
//!   chord index, the pitch class, the band's frequency), so the buttons do not move between items; the answer's
//!   position is uniform. A pattern game lists its options in draw order.
//! - **Frozen by a golden.** 1,000 specs to a hash, on Linux, Windows and wasm (`tests/ear_golden.rs`, file
//!   `tests/golden/ear-v1.txt`). It freezes at INT4's friend build, the first that shares ear codes; until then the
//!   ladder lanes (G1 to G3, TAP) update the golden on purpose and the commit names the games and why. After it, a
//!   change is ear generator v2.
//!
//! The parameter games (Spot the change, Dial it in, Ear check; ids 1 to 3) are not here: they need the catalogue, and
//! live in `enth-challenge` on v2 draws. Their ids are reserved so the numbering is one list.

use crate::{ladder, rhythm, theory};
use enth_dsp::util::rng::{Rng, derive, fnv1a64};
use serde::{Deserialize, Serialize};
use std::fmt;

/// The version of this generator, in [`EarSpec::generator`].
pub const GENERATOR_V1: u8 = 1;
/// The newest generator this build has (the one new specs use).
pub const LATEST_GENERATOR: u8 = GENERATOR_V1;
/// The largest seed: 2^53 - 1, so the JSON number survives every reader (as `RandomSpec`'s).
pub const MAX_SEED: u64 = (1 << 53) - 1;
/// The most questions in a spec (6 bits in a code).
pub const MAX_COUNT: u8 = 64;
/// The highest level a spec may name (4 bits in a code); each game has fewer ([`Game::max_level`]).
pub const MAX_LEVEL: u8 = 15;
/// The most notes sounding at once in a question.
pub const MAX_NOTES_AT_ONCE: usize = 8;
/// The longest clip, in ms.
pub const MAX_LENGTH_MS: u32 = 10_000;
/// Room after the last note-off for a release, added to every clip: 1 s.
pub const TAIL_MS: u32 = 1000;
/// The most options a question lists (note naming lists up to a full octave).
pub const MAX_OPTIONS: usize = 12;

/// Game id of Spot the change: a parameter game, generated in `enth-challenge`, not here.
pub const SPOT_THE_CHANGE: u8 = 1;
/// Game id of Dial it in: a parameter game, generated in `enth-challenge`.
pub const DIAL_IT_IN: u8 = 2;
/// Game id of Ear check: a parameter game, generated in `enth-challenge`.
pub const EAR_CHECK: u8 = 3;

/// Timbre ids: indices into `enth-challenge`'s frozen, append-only pool `timbres-v1`. A question names a timbre by id
/// and the renderer owns the patch, so ids here are a contract with that pool (docs/ear.md, "The ear generator").
pub mod timbre {
    /// A plain sine.
    pub const SINE: u8 = 0;
    /// A saw through a low-pass.
    pub const SAW_LOW_PASS: u8 = 1;
    /// A pluck.
    pub const PLUCK: u8 = 2;
    /// An organ table.
    pub const ORGAN: u8 = 3;
    /// A pad.
    pub const PAD: u8 = 4;
    /// A noise-burst click (short; not a sustained source).
    pub const CLICK: u8 = 5;
    /// Sustained pink noise, the source of Frequency finder's level 1. **Appended to the pool for this generator**
    /// (the first six are the pool's original list).
    pub const NOISE: u8 = 6;
    /// The highest id this generator uses.
    pub const LAST: u8 = NOISE;
}

/// The music games this crate generates, with the ids they share with the parameter games (1 to 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Game {
    /// Name the band a noise or saw is filtered to (ladder file `ladder::bands`, lane G3).
    FrequencyFinder = 4,
    /// Name the interval of two notes (`ladder::intervals`, lane G1).
    IntervalStacks = 5,
    /// Name the quality of a chord (`ladder::chords`, lane G2).
    ChordQuality = 6,
    /// Pick the pattern you heard, then mark it on the grid (`ladder::rhythm`, lane G2).
    RhythmPattern = 7,
    /// R4b: name the progression (`ladder::progressions`, lane G2).
    Progressions = 8,
    /// R4b, opt-in and never in the default workout: name the note (`ladder::pitch`, lane G1).
    NoteNaming = 9,
    /// R4b, after the TAP spike: tap the pattern back (`ladder::taps`, lane TAP).
    TapBack = 10,
}

/// What a question is answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnswerKind {
    /// An index into the options ([`Answer::Choice`]).
    Choice,
    /// Tap times ([`Answer::Taps`]).
    Taps,
}

impl Game {
    /// Every music game, in id order.
    pub const ALL: [Self; 7] = [
        Self::FrequencyFinder,
        Self::IntervalStacks,
        Self::ChordQuality,
        Self::RhythmPattern,
        Self::Progressions,
        Self::NoteNaming,
        Self::TapBack,
    ];

    /// The id in [`EarSpec::game`].
    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// The game with this id, or `None` for the parameter games (1 to 3) and ids this build does not know.
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            4 => Some(Self::FrequencyFinder),
            5 => Some(Self::IntervalStacks),
            6 => Some(Self::ChordQuality),
            7 => Some(Self::RhythmPattern),
            8 => Some(Self::Progressions),
            9 => Some(Self::NoteNaming),
            10 => Some(Self::TapBack),
            _ => None,
        }
    }

    /// The word in the keyed stream and in logs: `"band"`, `"interval"`, `"chord"`, `"rhythm"`, `"progression"`,
    /// `"pitch"`, `"tap"`. Frozen: it is part of every draw's key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::FrequencyFinder => "band",
            Self::IntervalStacks => "interval",
            Self::ChordQuality => "chord",
            Self::RhythmPattern => "rhythm",
            Self::Progressions => "progression",
            Self::NoteNaming => "pitch",
            Self::TapBack => "tap",
        }
    }

    /// The name the UI prints.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::FrequencyFinder => "Frequency finder",
            Self::IntervalStacks => "Interval stacks",
            Self::ChordQuality => "Chord quality",
            Self::RhythmPattern => "Rhythm pattern",
            Self::Progressions => "Progressions",
            Self::NoteNaming => "Note naming",
            Self::TapBack => "Tap back",
        }
    }

    /// The number of levels this build's ladder has for the game (levels are `1..=max_level()`).
    #[must_use]
    pub const fn max_level(self) -> u8 {
        match self {
            Self::FrequencyFinder => ladder::bands::MAX_LEVEL,
            Self::IntervalStacks => ladder::intervals::MAX_LEVEL,
            Self::ChordQuality => ladder::chords::MAX_LEVEL,
            Self::RhythmPattern => ladder::rhythm::MAX_LEVEL,
            Self::Progressions => ladder::progressions::MAX_LEVEL,
            Self::NoteNaming => ladder::pitch::MAX_LEVEL,
            Self::TapBack => ladder::taps::MAX_LEVEL,
        }
    }

    /// What the game's questions at `level` are answered with.
    #[must_use]
    pub const fn answer_kind(self, _level: u8) -> AnswerKind {
        match self {
            Self::TapBack => AnswerKind::Taps,
            _ => AnswerKind::Choice,
        }
    }
}

/// The settings of a set of questions. Persisted inside `enth-challenge`'s `Target::Ear` (challenge v3) and in the
/// ear-challenge code, so the JSON form is these fields in this order, every number an unsigned integer:
/// `{"game":5,"generator":1,"level":1,"seed":42,"count":20}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarSpec {
    /// The game id: [`Game::id`] (4 to 10 here; 1 to 3 are the parameter games in `enth-challenge`).
    pub game: u8,
    /// The generator version, 1 to [`LATEST_GENERATOR`]. Old versions stay frozen with goldens; a newer one than the
    /// build has is refused.
    pub generator: u8,
    /// The level, 1 to the game's [`Game::max_level`] in this build (and at most [`MAX_LEVEL`]).
    pub level: u8,
    /// The seed every draw is keyed from, at most [`MAX_SEED`].
    pub seed: u64,
    /// How many questions, 1 to [`MAX_COUNT`].
    pub count: u8,
}

impl EarSpec {
    /// A spec for the latest generator.
    #[must_use]
    pub const fn new(game: Game, level: u8, seed: u64, count: u8) -> Self {
        Self {
            game: game.id(),
            generator: LATEST_GENERATOR,
            level,
            seed,
            count,
        }
    }

    /// Check the spec against this build: a generator it has, a game it generates, a level of that game's ladder, a
    /// seed below 2^53 and a count of 1 to [`MAX_COUNT`]. Returns the game. Every error means "this build cannot play
    /// this spec" (a newer build's code reads `code_too_new`).
    pub fn validate(&self) -> Result<Game, EarError> {
        if self.generator == 0 || self.generator > LATEST_GENERATOR {
            return Err(EarError::UnknownGenerator(self.generator));
        }
        let game = Game::from_id(self.game).ok_or(EarError::UnknownGame(self.game))?;
        if self.level == 0 || self.level > game.max_level() || self.level > MAX_LEVEL {
            return Err(EarError::LevelUnavailable {
                game: self.game,
                level: self.level,
                max: game.max_level(),
            });
        }
        if self.seed > MAX_SEED {
            return Err(EarError::SeedTooLarge(self.seed));
        }
        if self.count == 0 || self.count > MAX_COUNT {
            return Err(EarError::BadCount(self.count));
        }
        Ok(game)
    }

    /// Question `item` of the set, at the spec's level (`item < count`).
    pub fn question(&self, item: u32) -> Result<Question, EarError> {
        let game = self.validate()?;
        if item >= u32::from(self.count) {
            return Err(EarError::ItemOutOfRange {
                item,
                count: self.count,
            });
        }
        question(game, self.level, self.seed, item)
    }

    /// All `count` questions, in order.
    pub fn questions(&self) -> Result<Vec<Question>, EarError> {
        let game = self.validate()?;
        (0..u32::from(self.count))
            .map(|item| question(game, self.level, self.seed, item))
            .collect()
    }
}

/// Why a spec cannot be generated by this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarError {
    /// A generator version this build does not have (0 or newer than [`LATEST_GENERATOR`]).
    UnknownGenerator(u8),
    /// A game id that is not a music game of this build (the parameter games, 1 to 3, are generated elsewhere).
    UnknownGame(u8),
    /// A level outside the game's ladder.
    LevelUnavailable {
        /// The game id.
        game: u8,
        /// The level asked for.
        level: u8,
        /// The highest level this build has for the game.
        max: u8,
    },
    /// A seed above [`MAX_SEED`].
    SeedTooLarge(u64),
    /// A count of 0 or above [`MAX_COUNT`].
    BadCount(u8),
    /// An item index at or past the spec's count.
    ItemOutOfRange {
        /// The index asked for.
        item: u32,
        /// The spec's count.
        count: u8,
    },
}

impl fmt::Display for EarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownGenerator(g) => write!(f, "ear generator {g} is not in this build"),
            Self::UnknownGame(g) => write!(f, "game {g} is not a music game of this build"),
            Self::LevelUnavailable { game, level, max } => {
                write!(
                    f,
                    "game {game} has no level {level} in this build (levels 1 to {max})"
                )
            }
            Self::SeedTooLarge(s) => write!(f, "seed {s} is above 2^53 - 1"),
            Self::BadCount(c) => write!(f, "{c} questions: a set has 1 to {MAX_COUNT}"),
            Self::ItemOutOfRange { item, count } => write!(f, "item {item} of a set of {count}"),
        }
    }
}

impl std::error::Error for EarError {}

/// The keyed stream of item `item` of `game` at `level`: `derive(seed, fnv1a64("<game key>/<item>/<level>"))`.
#[must_use]
pub fn stream(game: Game, level: u8, seed: u64, item: u32) -> Rng {
    let key = format!("{}/{item}/{level}", game.key());
    Rng::new(derive(seed, fnv1a64(key.as_bytes())))
}

/// Question `item` of `game` at `level`, drawn from the keyed stream of `seed`. It depends on nothing else, so a
/// staircase, a fixed ladder (an ear duel: item `i`'s level a function of `i`) or a replay asks for any item at any
/// level without generating the rest.
pub fn question(game: Game, level: u8, seed: u64, item: u32) -> Result<Question, EarError> {
    if level == 0 || level > game.max_level() || level > MAX_LEVEL {
        return Err(EarError::LevelUnavailable {
            game: game.id(),
            level,
            max: game.max_level(),
        });
    }
    if seed > MAX_SEED {
        return Err(EarError::SeedTooLarge(seed));
    }
    let mut rng = stream(game, level, seed, item);
    Ok(match game {
        Game::FrequencyFinder => ladder::bands::question(level, &mut rng),
        Game::IntervalStacks => ladder::intervals::question(level, &mut rng),
        Game::ChordQuality => ladder::chords::question(level, &mut rng),
        Game::RhythmPattern => ladder::rhythm::question(level, &mut rng),
        Game::Progressions => ladder::progressions::question(level, &mut rng),
        Game::NoteNaming => ladder::pitch::question(level, &mut rng),
        Game::TapBack => ladder::taps::question(level, &mut rng),
    })
}

/// The most items [`find_skill_item`] looks at.
pub const MAX_SKILL_SCAN: u32 = 4096;

/// The first item at or after `from` whose question exercises `skill` (a key like `ear.interval.m3`, as
/// [`Question::skill`] names it), with its index; `None` if the game never asks it at this level within
/// [`MAX_SKILL_SCAN`] items (or the level is not in this build). Items are keyed streams, so scanning generates only
/// what it looks at, and the result is a function of the seed and `from`: a burst of 5 items on one skill is five calls,
/// each starting after the last (`from = item + 1`), and two players given the same seed get the same burst.
#[must_use]
pub fn find_skill_item(
    game: Game,
    level: u8,
    seed: u64,
    skill: &str,
    from: u32,
) -> Option<(u32, Question)> {
    question(game, level, seed, from).ok()?;
    (from..from.saturating_add(MAX_SKILL_SCAN)).find_map(|item| {
        let q = question(game, level, seed, item).ok()?;
        (q.skill().as_deref() == Some(skill)).then_some((item, q))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The question
// ---------------------------------------------------------------------------------------------------------------

/// One note of a question: the same four integers as `enth-challenge`'s `PhraseNote`, so a question becomes a phrase
/// one to one. The note sounds from `start_ms` to `start_ms + hold_ms` (half open) and then releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteSpec {
    /// MIDI key, C1 (24) to C7 (96); C4 = 60.
    pub key: u8,
    /// MIDI velocity, 1 to 127.
    pub velocity: u8,
    /// Note-on, in ms from the start of the clip.
    pub start_ms: u32,
    /// How long the note is held, in ms (at least 1).
    pub hold_ms: u32,
}

impl NoteSpec {
    /// The end of the held note, in ms.
    #[must_use]
    pub const fn end_ms(&self) -> u32 {
        self.start_ms + self.hold_ms
    }
}

/// A note.
#[must_use]
pub const fn note(key: u8, velocity: u8, start_ms: u32, hold_ms: u32) -> NoteSpec {
    NoteSpec {
        key,
        velocity,
        start_ms,
        hold_ms,
    }
}

/// The length of a clip holding `notes`: the last note-off plus [`TAIL_MS`] for the release.
#[must_use]
pub fn clip_length_ms(notes: &[NoteSpec]) -> u32 {
    notes.iter().map(NoteSpec::end_ms).max().unwrap_or(0) + TAIL_MS
}

/// The most notes sounding at once (half-open intervals: a note ending when another starts does not overlap it).
#[must_use]
pub fn max_simultaneous(notes: &[NoteSpec]) -> usize {
    notes
        .iter()
        .map(|a| {
            notes
                .iter()
                .filter(|b| b.start_ms <= a.start_ms && a.start_ms < b.end_ms())
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// What a post-EQ band does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum EqKind {
    /// A peaking boost or cut (the biquad's RBJ peak).
    Peak = 0,
    /// A low shelf.
    LowShelf = 1,
    /// A high shelf.
    HighShelf = 2,
    /// The synth's own svf band-pass at the centre, `q_milli` its quality, no gain: Frequency finder's svf form
    /// (docs/ear.md keeps it after the EQ form lands, because it teaches the synth's own filter).
    BandPass = 3,
}

/// A band applied after the render (Frequency finder), integers only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    /// What the band does.
    pub kind: EqKind,
    /// The centre (the corner of a shelf), in Hz, 20 to 20,000.
    pub freq_hz: u16,
    /// The gain in hundredths of a dB (0 for a band-pass).
    pub gain_cdb: i16,
    /// The quality factor times 1000 (8000 is Q 8), at least 1.
    pub q_milli: u16,
}

/// What an option's id means, so labels and skills come from the tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum OptionKind {
    /// No options (an open answer: taps).
    None = 0,
    /// Ids are interval sizes in semitones ([`theory::INTERVALS`]).
    Interval = 1,
    /// Ids are chord qualities ([`theory::CHORDS`]).
    Chord = 2,
    /// Ids are pitch classes 0 to 11.
    PitchClass = 3,
    /// Ids are band centres in Hz.
    Band = 4,
    /// Ids are rhythm patterns: the 16-bit mask ([`rhythm::Pattern`]). To mark the pattern on the grid, the expected
    /// mask is `options.ids[answer]`.
    Pattern = 5,
    /// Ids are progressions ([`theory::PROGRESSIONS`]).
    Progression = 6,
}

/// The options of a choice question: what they are and their ids, in the order the UI lists them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// What the ids mean.
    pub kind: OptionKind,
    /// The ids, distinct.
    pub ids: Vec<u32>,
}

impl Options {
    /// No options.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            kind: OptionKind::None,
            ids: Vec::new(),
        }
    }

    /// The label of option `index`: `P5`, `minor`, `F#`, `1 kHz`, `x...x...`, `I-IV-V-I`.
    #[must_use]
    pub fn label(&self, index: usize) -> Option<String> {
        let id = *self.ids.get(index)?;
        match self.kind {
            OptionKind::None => None,
            OptionKind::Interval => {
                theory::interval(u8::try_from(id).ok()?).map(|i| i.short.to_owned())
            }
            OptionKind::Chord => theory::chord(u8::try_from(id).ok()?).map(|c| c.name.to_owned()),
            OptionKind::PitchClass => theory::NAMES_SHARP
                .get(id as usize)
                .map(|n| (*n).to_owned()),
            OptionKind::Band => Some(band_label(id)),
            OptionKind::Pattern => Some(rhythm::Pattern(u16::try_from(id).ok()?).grid()),
            OptionKind::Progression => {
                theory::progression(u8::try_from(id).ok()?).map(|p| p.name.to_owned())
            }
        }
    }

    /// Every label, in order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        (0..self.ids.len()).filter_map(|i| self.label(i)).collect()
    }
}

/// `250 Hz`, `1 kHz`, `1.5 kHz`: a band centre for a label.
#[must_use]
pub fn band_label(hz: u32) -> String {
    if hz < 1000 {
        format!("{hz} Hz")
    } else if hz.is_multiple_of(1000) {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{}.{} kHz", hz / 1000, (hz % 1000) / 100)
    }
}

/// `250`, `1k`, `8k`: a band centre in a skill key (`ear.band.1k`).
#[must_use]
pub fn band_slug(hz: u32) -> String {
    if hz >= 1000 && hz.is_multiple_of(1000) {
        format!("{}k", hz / 1000)
    } else {
        hz.to_string()
    }
}

/// The right answer of a question.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// The index into [`Question::options`] of the right option, judged exactly.
    Choice(u8),
    /// The pattern to tap back: onset times in ms from the start of the response bar, strictly increasing, inside
    /// the window; `window_ms` is the bar. Scored by `taps/1` in `enth-challenge` on the intervals between taps.
    Taps {
        /// The onsets.
        onsets_ms: Vec<u32>,
        /// The length of the response window.
        window_ms: u16,
    },
}

/// One question: what to play, and the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    /// The timbre: an index into `enth-challenge`'s frozen pool (see [`timbre`]).
    pub timbre: u8,
    /// The stimulus, in the order they sound.
    pub notes: Vec<NoteSpec>,
    /// The length of the clip in ms, last note-off plus the release tail; at most [`MAX_LENGTH_MS`].
    pub length_ms: u32,
    /// A band applied after the render, if the game has one.
    pub post: Option<EqBand>,
    /// The right answer.
    pub answer: Answer,
    /// The options (none for taps).
    pub options: Options,
}

/// Why a question is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuestionError {
    /// No notes.
    NoNotes,
    /// A key outside C1 to C7.
    KeyOutOfRange {
        /// The note.
        index: usize,
    },
    /// A velocity of 0 or above 127.
    BadVelocity {
        /// The note.
        index: usize,
    },
    /// A zero hold, a note ending after the clip, or an end that overflows.
    BadTiming {
        /// The note.
        index: usize,
    },
    /// More notes at once than [`MAX_NOTES_AT_ONCE`].
    TooManyAtOnce(usize),
    /// A clip longer than [`MAX_LENGTH_MS`].
    TooLong(u32),
    /// A timbre id beyond [`timbre::LAST`].
    UnknownTimbre(u8),
    /// A band outside 20 Hz to 20 kHz, with a zero Q, or a gain beyond +-24 dB.
    BadBand,
    /// Options that are not distinct, are too few or too many for a choice, or are listed for an open answer.
    BadOptions,
    /// A choice that is not one of the options.
    AnswerNotAnOption,
    /// Taps out of order, outside the window, or a window past the clip.
    BadTaps,
}

impl fmt::Display for QuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid ear question: {self:?}")
    }
}

impl std::error::Error for QuestionError {}

impl Question {
    /// The invariants of docs/ear.md: keys C1 to C7, velocities 1 to 127, every note inside the clip, at most 8 notes
    /// at once, at most 10 s, a known timbre, a sane band, a choice that is one of 2 to [`MAX_OPTIONS`] distinct
    /// options (or taps in order inside their window and no options).
    pub fn validate(&self) -> Result<(), QuestionError> {
        if self.notes.is_empty() {
            return Err(QuestionError::NoNotes);
        }
        if self.length_ms > MAX_LENGTH_MS {
            return Err(QuestionError::TooLong(self.length_ms));
        }
        for (index, n) in self.notes.iter().enumerate() {
            if !(theory::KEY_MIN..=theory::KEY_MAX).contains(&n.key) {
                return Err(QuestionError::KeyOutOfRange { index });
            }
            if n.velocity == 0 || n.velocity > 127 {
                return Err(QuestionError::BadVelocity { index });
            }
            match n.start_ms.checked_add(n.hold_ms) {
                Some(end) if n.hold_ms > 0 && end <= self.length_ms => {}
                _ => return Err(QuestionError::BadTiming { index }),
            }
        }
        let at_once = max_simultaneous(&self.notes);
        if at_once > MAX_NOTES_AT_ONCE {
            return Err(QuestionError::TooManyAtOnce(at_once));
        }
        if self.timbre > timbre::LAST {
            return Err(QuestionError::UnknownTimbre(self.timbre));
        }
        if let Some(b) = &self.post
            && (!(20..=20_000).contains(&b.freq_hz)
                || b.q_milli == 0
                || b.gain_cdb.unsigned_abs() > 2400)
        {
            return Err(QuestionError::BadBand);
        }
        match &self.answer {
            Answer::Choice(i) => {
                let ids = &self.options.ids;
                let mut sorted = ids.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if self.options.kind == OptionKind::None
                    || !(2..=MAX_OPTIONS).contains(&ids.len())
                    || sorted.len() != ids.len()
                {
                    return Err(QuestionError::BadOptions);
                }
                if usize::from(*i) >= ids.len() {
                    return Err(QuestionError::AnswerNotAnOption);
                }
            }
            Answer::Taps {
                onsets_ms,
                window_ms,
            } => {
                if self.options.kind != OptionKind::None || !self.options.ids.is_empty() {
                    return Err(QuestionError::BadOptions);
                }
                let ordered = onsets_ms.windows(2).all(|w| w[0] < w[1]);
                let inside = onsets_ms.iter().all(|t| *t < u32::from(*window_ms));
                if onsets_ms.is_empty() || !ordered || !inside || *window_ms == 0 {
                    return Err(QuestionError::BadTaps);
                }
            }
        }
        Ok(())
    }

    /// The most notes sounding at once.
    #[must_use]
    pub fn max_simultaneous(&self) -> usize {
        max_simultaneous(&self.notes)
    }

    /// The Review skill this question exercises, from the right answer: `ear.interval.m3`, `ear.chord.min`,
    /// `ear.band.1k`, `ear.pitch.cs`, `ear.progression.i-iv-v-i`, `ear.rhythm.pattern`, `ear.rhythm.taps`. The drill
    /// player files a burst's results under it.
    #[must_use]
    pub fn skill(&self) -> Option<String> {
        let index = match &self.answer {
            Answer::Taps { .. } => return Some("ear.rhythm.taps".to_owned()),
            Answer::Choice(i) => usize::from(*i),
        };
        let id = *self.options.ids.get(index)?;
        match self.options.kind {
            OptionKind::None => None,
            OptionKind::Interval => theory::interval(u8::try_from(id).ok()?)
                .map(|i| format!("ear.interval.{}", i.skill)),
            OptionKind::Chord => {
                theory::chord(u8::try_from(id).ok()?).map(|c| format!("ear.chord.{}", c.skill))
            }
            OptionKind::PitchClass => theory::PITCH_SLUGS
                .get(id as usize)
                .map(|s| format!("ear.pitch.{s}")),
            OptionKind::Band => Some(format!("ear.band.{}", band_slug(id))),
            OptionKind::Pattern => Some("ear.rhythm.pattern".to_owned()),
            OptionKind::Progression => theory::progression(u8::try_from(id).ok()?)
                .map(|p| format!("ear.progression.{}", p.skill)),
        }
    }

    /// The right option's label (`P5`), for feedback after an answer; `None` for taps.
    #[must_use]
    pub fn answer_label(&self) -> Option<String> {
        match &self.answer {
            Answer::Choice(i) => self.options.label(usize::from(*i)),
            Answer::Taps { .. } => None,
        }
    }

    /// The canonical bytes the golden hashes: every field in a fixed order, integers big-endian, lengths as `u32`.
    /// Frozen with ear generator v1; a field added to the types is a new generator.
    pub fn write_canonical(&self, out: &mut Vec<u8>) {
        fn put_u32(out: &mut Vec<u8>, v: u32) {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.push(self.timbre);
        put_u32(out, self.notes.len() as u32);
        for n in &self.notes {
            out.extend_from_slice(&[n.key, n.velocity]);
            put_u32(out, n.start_ms);
            put_u32(out, n.hold_ms);
        }
        put_u32(out, self.length_ms);
        match &self.post {
            None => out.push(0),
            Some(b) => {
                out.push(1);
                out.push(b.kind as u8);
                out.extend_from_slice(&b.freq_hz.to_be_bytes());
                out.extend_from_slice(&b.gain_cdb.to_be_bytes());
                out.extend_from_slice(&b.q_milli.to_be_bytes());
            }
        }
        match &self.answer {
            Answer::Choice(i) => out.extend_from_slice(&[0, *i]),
            Answer::Taps {
                onsets_ms,
                window_ms,
            } => {
                out.push(1);
                put_u32(out, onsets_ms.len() as u32);
                for t in onsets_ms {
                    put_u32(out, *t);
                }
                out.extend_from_slice(&window_ms.to_be_bytes());
            }
        }
        out.push(self.options.kind as u8);
        put_u32(out, self.options.ids.len() as u32);
        for id in &self.options.ids {
            put_u32(out, *id);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Helpers for the ladder files
// ---------------------------------------------------------------------------------------------------------------

/// A uniform index below `len` (`len > 0`).
pub fn below(rng: &mut Rng, len: usize) -> usize {
    rng.below(len as u64) as usize
}

/// A uniform element of a non-empty slice.
///
/// # Panics
///
/// Panics on an empty slice (the ladders' tables are never empty).
pub fn pick<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[below(rng, items.len())]
}

/// Draw an answer uniformly from `pool` (distinct ids), then `n - 1` distractors without replacement, and list the
/// `n` options in ascending order: returns the options and the answer's index among them. Draws `n` times from `rng`
/// (the answer first); `n` is clamped to the pool's size. Because the answer is uniform and the others are a uniform
/// subset of the rest, its position is uniform over `0..n`.
pub fn pick_options(rng: &mut Rng, pool: &[u32], n: usize) -> (Vec<u32>, u8) {
    let mut rest = pool.to_vec();
    let answer = rest.swap_remove(below(rng, rest.len()));
    let mut ids = vec![answer];
    while ids.len() < n.min(pool.len()) {
        ids.push(rest.swap_remove(below(rng, rest.len())));
    }
    ids.sort_unstable();
    let index = ids.iter().position(|id| *id == answer).unwrap_or(0);
    (ids, index as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_ids_are_the_docs_numbers() {
        assert_eq!(Game::FrequencyFinder.id(), 4);
        assert_eq!(Game::IntervalStacks.id(), 5);
        assert_eq!(Game::ChordQuality.id(), 6);
        assert_eq!(Game::RhythmPattern.id(), 7);
        assert_eq!(Game::Progressions.id(), 8);
        assert_eq!(Game::NoteNaming.id(), 9);
        assert_eq!(Game::TapBack.id(), 10);
        for g in Game::ALL {
            assert_eq!(Game::from_id(g.id()), Some(g));
            assert!(g.max_level() >= 1 && g.max_level() <= MAX_LEVEL);
        }
        for id in [0, SPOT_THE_CHANGE, DIAL_IT_IN, EAR_CHECK, 11, 255] {
            assert_eq!(Game::from_id(id), None);
        }
        let keys: Vec<&str> = Game::ALL.iter().map(|g| g.key()).collect();
        assert_eq!(
            keys,
            [
                "band",
                "interval",
                "chord",
                "rhythm",
                "progression",
                "pitch",
                "tap"
            ]
        );
    }

    #[test]
    fn specs_are_checked_against_the_build() {
        let ok = EarSpec::new(Game::IntervalStacks, 1, 42, 20);
        assert_eq!(ok.validate(), Ok(Game::IntervalStacks));
        assert_eq!(ok.generator, GENERATOR_V1);
        let bad = |f: fn(&mut EarSpec)| {
            let mut s = ok;
            f(&mut s);
            s.validate().unwrap_err()
        };
        assert_eq!(bad(|s| s.generator = 0), EarError::UnknownGenerator(0));
        assert_eq!(bad(|s| s.generator = 2), EarError::UnknownGenerator(2));
        assert_eq!(bad(|s| s.game = 1), EarError::UnknownGame(1));
        assert_eq!(bad(|s| s.game = 3), EarError::UnknownGame(3));
        assert_eq!(bad(|s| s.game = 11), EarError::UnknownGame(11));
        assert!(matches!(
            bad(|s| s.level = 0),
            EarError::LevelUnavailable { level: 0, .. }
        ));
        assert!(matches!(
            bad(|s| s.level = 16),
            EarError::LevelUnavailable { level: 16, .. }
        ));
        assert_eq!(
            bad(|s| s.seed = MAX_SEED + 1),
            EarError::SeedTooLarge(MAX_SEED + 1)
        );
        assert_eq!(bad(|s| s.count = 0), EarError::BadCount(0));
        assert_eq!(
            bad(|s| s.count = MAX_COUNT + 1),
            EarError::BadCount(MAX_COUNT + 1)
        );
        let mut top = ok;
        top.seed = MAX_SEED;
        top.count = MAX_COUNT;
        assert!(top.validate().is_ok());
        assert_eq!(
            ok.question(20),
            Err(EarError::ItemOutOfRange {
                item: 20,
                count: 20
            })
        );
        assert!(ok.question(19).is_ok());
        assert_eq!(ok.questions().unwrap().len(), 20);
    }

    #[test]
    fn items_are_keyed_streams() {
        // The same item gives the same question whatever else was generated; a different level or seed another.
        let all = EarSpec::new(Game::ChordQuality, 1, 7, 10)
            .questions()
            .unwrap();
        let alone = question(Game::ChordQuality, 1, 7, 6).unwrap();
        assert_eq!(all[6], alone);
        let one = EarSpec::new(Game::ChordQuality, 1, 7, 1)
            .questions()
            .unwrap();
        assert_eq!(one[0], all[0], "count does not move item 0");
        assert_ne!(
            stream(Game::ChordQuality, 1, 7, 6).next_u64(),
            stream(Game::ChordQuality, 1, 8, 6).next_u64()
        );
        assert_ne!(
            stream(Game::ChordQuality, 1, 7, 6).next_u64(),
            stream(Game::ChordQuality, 1, 7, 7).next_u64()
        );
        assert_ne!(
            stream(Game::ChordQuality, 1, 7, 6).next_u64(),
            stream(Game::IntervalStacks, 1, 7, 6).next_u64()
        );
        // The key is the documented text.
        let mut r = stream(Game::IntervalStacks, 1, 99, 7);
        let want = derive(99, fnv1a64(b"interval/7/1"));
        assert_eq!(r.next_u64(), Rng::new(want).next_u64());
    }

    #[test]
    fn labels_and_skills_come_from_the_tables() {
        let q = question(Game::IntervalStacks, 1, 3, 0).unwrap();
        let skill = q.skill().unwrap();
        assert!(
            skill == "ear.interval.p1" || skill == "ear.interval.p8",
            "{skill}"
        );
        assert_eq!(q.options.labels().len(), q.options.ids.len());
        assert!(q.answer_label().is_some());
        assert_eq!(band_label(250), "250 Hz");
        assert_eq!(band_label(1000), "1 kHz");
        assert_eq!(band_label(1500), "1.5 kHz");
        assert_eq!(band_slug(1000), "1k");
        assert_eq!(band_slug(250), "250");
        let options = Options {
            kind: OptionKind::Pattern,
            ids: vec![0x0511],
        };
        assert_eq!(options.label(0).as_deref(), Some("x...x...x.x....."));
        assert_eq!(options.label(1), None);
    }

    #[test]
    fn pick_options_lists_ascending_with_a_uniform_answer_position() {
        let mut counts = [0u32; 3];
        let mut rng = Rng::new(1);
        for _ in 0..3000 {
            let (ids, i) = pick_options(&mut rng, &[5, 1, 9, 3, 7], 3);
            assert_eq!(ids.len(), 3);
            assert!(ids.windows(2).all(|w| w[0] < w[1]));
            counts[usize::from(i)] += 1;
        }
        for c in counts {
            assert!((900..=1100).contains(&c), "{counts:?}");
        }
        // Clamped to the pool.
        let (ids, _) = pick_options(&mut rng, &[2, 4], 5);
        assert_eq!(ids, [2, 4]);
    }

    #[test]
    fn validation_catches_each_kind_of_bad_question() {
        let good = question(Game::IntervalStacks, 1, 1, 0).unwrap();
        assert_eq!(good.validate(), Ok(()));
        let mutate = |f: &dyn Fn(&mut Question)| {
            let mut q = good.clone();
            f(&mut q);
            q.validate().unwrap_err()
        };
        assert_eq!(mutate(&|q| q.notes.clear()), QuestionError::NoNotes);
        assert_eq!(
            mutate(&|q| q.notes[0].key = 23),
            QuestionError::KeyOutOfRange { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.notes[0].key = 97),
            QuestionError::KeyOutOfRange { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.notes[0].velocity = 0),
            QuestionError::BadVelocity { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.notes[0].velocity = 128),
            QuestionError::BadVelocity { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.notes[0].hold_ms = 0),
            QuestionError::BadTiming { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.notes[0].start_ms = q.length_ms),
            QuestionError::BadTiming { index: 0 }
        );
        assert_eq!(
            mutate(&|q| q.length_ms = 10_001),
            QuestionError::TooLong(10_001)
        );
        assert_eq!(mutate(&|q| q.timbre = 7), QuestionError::UnknownTimbre(7));
        assert_eq!(
            mutate(&|q| q.notes = vec![note(60, 100, 0, 1000); 9]),
            QuestionError::TooManyAtOnce(9)
        );
        assert_eq!(
            mutate(&|q| q.answer = Answer::Choice(2)),
            QuestionError::AnswerNotAnOption
        );
        assert_eq!(
            mutate(&|q| q.options.ids = vec![0]),
            QuestionError::BadOptions
        );
        assert_eq!(
            mutate(&|q| q.options.ids = vec![12, 12]),
            QuestionError::BadOptions
        );
        assert_eq!(
            mutate(&|q| q.options.kind = OptionKind::None),
            QuestionError::BadOptions
        );
        assert_eq!(
            mutate(&|q| q.post = Some(EqBand {
                kind: EqKind::Peak,
                freq_hz: 10,
                gain_cdb: 0,
                q_milli: 1
            })),
            QuestionError::BadBand
        );
        assert_eq!(
            mutate(&|q| q.answer = Answer::Taps {
                onsets_ms: vec![0, 500],
                window_ms: 2400
            }),
            QuestionError::BadOptions
        );
        // Eight at once is fine, nine is not; back-to-back notes do not overlap.
        let eight = vec![note(60, 100, 0, 1000); 8];
        assert_eq!(max_simultaneous(&eight), 8);
        assert_eq!(
            max_simultaneous(&[note(60, 100, 0, 500), note(62, 100, 500, 500)]),
            1
        );
    }
}
