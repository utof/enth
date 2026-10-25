//! **Owner: lane ECK.** The games as `ear-check` sees them: an [`EarGame`] draws [`Item`]s, and an item holds what the
//! checks need, whatever family made it.
//!
//! An item is a question **and what each option would have sounded like**. That is the point of the tool: to ask whether
//! an option can be told from the right answer, the checks need the clip of every option in the context of this one item
//! (its root, timbre, key, voicing and the rest held). The music games get it from `enth_music::ladder::with_answer`
//! (the identity test in `crates/enth-music/tests/with_answer.rs` is its contract; each item is checked against it again
//! here). Spot the change gets it from the public parameter-game API: the clip of option `j` is the base with control
//! `j` moved by the level's steps in the direction the right answer moved (`ear_check_on` draws the same move). Ear check's
//! options are the position of the odd interval, so the clips are the base and the odd one.
//!
//! The ear games of `enth-challenge` and `enth-music` stay the source of truth: an item's `ekey` and clips are the
//! `EarQuestion` the session plays, and every check that judges does so through the real scorer (`ear/1`).

use enth_challenge::ear::EarSpec;
use enth_challenge::ear::params;
use enth_challenge::ear::question::{self, Clip as Played};
use enth_challenge::ear::scorers::EarKey;
use enth_music::ear::{self as music, Answer, Game as Music, Question};
use enth_music::ladder::with_answer::with_answer;

/// How an item's states relate to its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One clip per option: `states[j]` is what the question sounds like if option `j` is right (the music games).
    PerOption,
    /// A reference, then a changed clip per option (Spot the change): `states[0]` is the reference A, `states[1 + j]` is
    /// B if control `j` moved.
    Changed,
    /// Ear check: `states[0]` is the base interval and `states[1]` the odd one; every option is a position of the odd
    /// one, so every pair of options differs by exactly this pair.
    OddOne,
}

/// What an option offers a player who is not listening, read from the notes (no render).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    /// The clip's length in ms.
    pub length_ms: u32,
    /// An energy proxy: the sum over notes of `(velocity / 127)^2 * hold in s`.
    pub energy: f64,
    /// The highest key.
    pub top: u8,
    /// How many notes.
    pub notes: usize,
}

impl Cue {
    /// The cue of a music question's notes.
    #[must_use]
    pub fn of(q: &Question) -> Cue {
        Cue {
            length_ms: q.length_ms,
            energy: q
                .notes
                .iter()
                .map(|n| {
                    let v = f64::from(n.velocity) / 127.0;
                    v * v * f64::from(n.hold_ms) / 1000.0
                })
                .sum(),
            top: q.notes.iter().map(|n| n.key).max().unwrap_or(0),
            notes: q.notes.len(),
        }
    }

    /// No cue: every option is the same to a bot that cannot hear (the parameter games' clips share a phrase).
    pub const NONE: Cue = Cue {
        length_ms: 0,
        energy: 0.0,
        top: 0,
        notes: 0,
    };
}

/// One drawn question with the clip of every option.
#[derive(Clone, Debug)]
pub struct Item {
    /// The game's id (`EarSpec::game`).
    pub game: u8,
    /// The level.
    pub level: u8,
    /// The seed.
    pub seed: u64,
    /// The item's index.
    pub index: u32,
    /// How many options (0 for Tap back).
    pub options: usize,
    /// The right option's index.
    pub key: usize,
    /// The Review skill the question exercises.
    pub skill: Option<String>,
    /// The key the real judge uses.
    pub ekey: EarKey,
    /// How the states relate to the options.
    pub layout: Layout,
    /// The clips ([`Layout`]).
    pub states: Vec<Played>,
    /// Per option, what a bot can read from the notes.
    pub cues: Vec<Cue>,
    /// For a parameter game, the controls the options name (Spot the change: one per option; Ear check: the one control).
    pub controls: Vec<String>,
    /// For a parameter game, how far the moved control moved, in parameter steps.
    pub steps: Option<f64>,
}

impl Item {
    /// The pair of states that tells option `i` from option `j`.
    #[must_use]
    pub fn option_pair(&self, i: usize, j: usize) -> (usize, usize) {
        match self.layout {
            Layout::PerOption => (i, j),
            Layout::Changed => (1 + i, 1 + j),
            Layout::OddOne => (0, 1),
        }
    }

    /// The pairs of states that show the player **a change at all**: Spot the change's A against each B, Ear check's
    /// base against the odd one. Empty for the music games, whose options are the states.
    #[must_use]
    pub fn change_pairs(&self) -> Vec<(usize, usize)> {
        match self.layout {
            Layout::PerOption => Vec::new(),
            Layout::Changed => (0..self.options).map(|j| (0, 1 + j)).collect(),
            Layout::OddOne => vec![(0, 1)],
        }
    }
}

/// A game `ear-check` plays.
pub trait EarGame: Sync {
    /// The game's id (`EarSpec::game`).
    fn id(&self) -> u8;
    /// Its name.
    fn name(&self) -> String;
    /// How many levels this build's ladder has.
    fn levels(&self) -> u8;
    /// Item `index` of `level` at `seed`.
    ///
    /// # Errors
    /// The game cannot draw it (named for the report).
    fn item(&self, level: u8, seed: u64, index: u32) -> Result<Item, String>;
    /// How many items a seed has at most (a parameter game numbers its items by a byte).
    fn items_per_seed(&self) -> u32 {
        u32::MAX
    }
    /// Whether this is a game of the build (its skills resolve through `enth_challenge::ear::find_item`, so bursts can be
    /// checked); a game planted by a test is not.
    fn real(&self) -> bool {
        false
    }
}

/// A shipped game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shipped {
    /// Game 1.
    Spot,
    /// Game 3.
    Check,
    /// A music game (4 to 10).
    Music(Music),
}

/// The shipped ear games, in id order. Dial it in (game 2) is a Recreate round, not an ear target: it has no options and
/// is judged by `sound/1`, so `ear-check` has nothing to say about it beyond coverage.
#[must_use]
pub fn shipped() -> Vec<Shipped> {
    let mut v = vec![Shipped::Spot, Shipped::Check];
    v.extend(Music::ALL.iter().map(|g| Shipped::Music(*g)));
    v
}

impl EarGame for Shipped {
    fn id(&self) -> u8 {
        match self {
            Shipped::Spot => params::SPOT_THE_CHANGE,
            Shipped::Check => params::EAR_CHECK,
            Shipped::Music(g) => g.id(),
        }
    }

    fn name(&self) -> String {
        match self {
            Shipped::Spot => "Spot the change".to_owned(),
            Shipped::Check => "Ear check".to_owned(),
            Shipped::Music(g) => g.title().to_owned(),
        }
    }

    fn levels(&self) -> u8 {
        match self {
            Shipped::Spot => params::top_level(params::SPOT_THE_CHANGE),
            Shipped::Check => params::top_level(params::EAR_CHECK),
            Shipped::Music(g) => g.max_level(),
        }
    }

    fn item(&self, level: u8, seed: u64, index: u32) -> Result<Item, String> {
        match self {
            Shipped::Spot => spot_item(level, seed, index),
            Shipped::Check => check_item(level, seed, index),
            Shipped::Music(g) => music_item(*g, level, seed, index),
        }
    }

    fn items_per_seed(&self) -> u32 {
        match self {
            Shipped::Spot | Shipped::Check => enth_challenge::ear::MAX_PARAMS_SCAN,
            Shipped::Music(_) => u32::MAX,
        }
    }

    fn real(&self) -> bool {
        true
    }
}

fn spec_of(game: u8, level: u8, seed: u64) -> EarSpec {
    EarSpec {
        game,
        generator: params::PARAMS_GENERATOR,
        level,
        seed,
        count: 1,
    }
}

fn byte(index: u32) -> Result<u8, String> {
    u8::try_from(index)
        .map_err(|_| format!("a parameter game's items are numbered 0 to 255, not {index}"))
}

/// A music game's item with the clip of every option.
fn music_item(game: Music, level: u8, seed: u64, index: u32) -> Result<Item, String> {
    let q = music::question(game, level, seed, index)
        .map_err(|e| format!("{} level {level} item {index}: {e}", game.title()))?;
    let eq = question::music_question(0, &q).map_err(|e| e.to_string())?;
    let n = q.options.ids.len();
    let key = match &q.answer {
        Answer::Choice(i) => usize::from(*i),
        Answer::Taps { .. } => 0,
    };
    let mut states = Vec::with_capacity(n);
    let mut cues = Vec::with_capacity(n);
    for j in 0..n {
        let cf = with_answer(game, level, &q, j).ok_or_else(|| {
            format!(
                "{} level {level} item {index}: `ladder::with_answer` has no question for option {j}: the game's `question` \
                 changed and its counterfactual builder did not follow (crates/enth-music/src/ladder/with_answer.rs)",
                game.title()
            )
        })?;
        let played = question::music_question(0, &cf).map_err(|e| e.to_string())?;
        states.push(played.clips[0].clone());
        cues.push(Cue::of(&cf));
    }
    if n > 0 && states[key] != eq.clips[0] {
        return Err(format!(
            "{} level {level} item {index}: the counterfactual for the right answer is not the question \
             (`ladder::with_answer`'s identity contract)",
            game.title()
        ));
    }
    Ok(Item {
        game: game.id(),
        level,
        seed,
        index,
        options: n,
        key,
        skill: eq.skill.clone(),
        ekey: eq.key.clone(),
        layout: Layout::PerOption,
        states,
        cues,
        controls: Vec::new(),
        steps: None,
    })
}

/// The base with `control` moved by `steps`, as Spot the change would have drawn it had that control been the one that
/// moved (`ear_check_on` draws the same `move_control`, keyed by the direction). The wanted direction is the right
/// answer's; a control that cannot move that way at that distance (a time already at its longest) moves the other, as
/// the generator redraws it.
fn moved_clip(
    spec: &EarSpec,
    control: &enth_patch::ParamAddr,
    steps: u8,
    up: bool,
) -> Result<Played, String> {
    let base = params::base();
    let at = base
        .value(control)
        .ok_or_else(|| format!("{control} is not on the base"))?;
    let mut other = None;
    for probe in 0..64u8 {
        let odd = params::ear_check_on(control, steps, spec, probe).map_err(|e| e.to_string())?;
        let patch = odd
            .draw
            .draw(&base, params::phrase())
            .map_err(|e| e.to_string())?
            .patch;
        let value = patch
            .value(control)
            .ok_or_else(|| format!("{control} is not on the base"))?;
        let clip = Played::Patch {
            patch: Box::new(patch),
            phrase: params::PARAMS_PHRASE,
        };
        if (value > at) == up {
            return Ok(clip);
        }
        other.get_or_insert(clip);
    }
    other.ok_or_else(|| format!("no probe moved {control}"))
}

fn spot_item(level: u8, seed: u64, index: u32) -> Result<Item, String> {
    let idx = byte(index)?;
    let spec = spec_of(params::SPOT_THE_CHANGE, level, seed);
    let lvl = params::level(spec.game, level)
        .ok_or_else(|| format!("Spot the change has no level {level}"))?;
    let eq = question::questions_at(&spec, u32::from(idx))
        .map_err(|e| e.to_string())?
        .remove(0);
    let item = params::spot_the_change(&spec, idx).map_err(|e| e.to_string())?;
    let base = params::base();
    let key = usize::from(item.key);
    let moved = &item.options[key];
    let Played::Patch { patch: b_key, .. } = &eq.clips[1] else {
        return Err("Spot the change's B is not a patch clip".to_owned());
    };
    let at = base
        .value(moved)
        .ok_or_else(|| format!("{moved} is not on the base"))?;
    let up = b_key
        .value(moved)
        .ok_or_else(|| format!("{moved} is not on B"))?
        > at;
    let mut states = vec![eq.clips[0].clone()];
    for (j, control) in item.options.iter().enumerate() {
        states.push(if j == key {
            eq.clips[1].clone()
        } else {
            moved_clip(&spec, control, lvl.steps, up)?
        });
    }
    Ok(Item {
        game: spec.game,
        level,
        seed,
        index,
        options: item.options.len(),
        key,
        skill: eq.skill.clone(),
        ekey: eq.key.clone(),
        layout: Layout::Changed,
        states,
        cues: vec![Cue::NONE; item.options.len()],
        controls: item.options.iter().map(ToString::to_string).collect(),
        steps: Some(item.steps),
    })
}

fn check_item(level: u8, seed: u64, index: u32) -> Result<Item, String> {
    let idx = byte(index)?;
    let spec = spec_of(params::EAR_CHECK, level, seed);
    let eq = question::questions_at(&spec, u32::from(idx))
        .map_err(|e| e.to_string())?
        .remove(0);
    let trial = params::ear_check(&spec, idx).map_err(|e| e.to_string())?;
    let n = usize::from(params::EAR_CHECK_INTERVALS);
    let odd = usize::from(trial.position);
    Ok(Item {
        game: spec.game,
        level,
        seed,
        index,
        options: n,
        key: odd,
        skill: eq.skill.clone(),
        ekey: eq.key.clone(),
        layout: Layout::OddOne,
        states: vec![eq.clips[(odd + 1) % n].clone(), eq.clips[odd].clone()],
        cues: vec![Cue::NONE; n],
        controls: vec![trial.control.to_string()],
        steps: Some(trial.steps),
    })
}

/// An Ear check trial on `control` at `steps` (a staircase's trial: the control and distance are the caller's).
///
/// # Errors
/// The trial cannot be drawn.
pub fn check_trial(
    control: &enth_patch::ParamAddr,
    steps: u8,
    seed: u64,
    index: u32,
) -> Result<(Played, Played), String> {
    let idx = byte(index)?;
    let spec = spec_of(params::EAR_CHECK, 1, seed);
    let trial = params::ear_check_on(control, steps, &spec, idx).map_err(|e| e.to_string())?;
    let base = params::base();
    let odd = trial
        .draw
        .draw(&base, params::phrase())
        .map_err(|e| e.to_string())?
        .patch;
    let clip = |p: enth_patch::Patch| Played::Patch {
        patch: Box::new(p),
        phrase: params::PARAMS_PHRASE,
    };
    Ok((clip(base), clip(odd)))
}
