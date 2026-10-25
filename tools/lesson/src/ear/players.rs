//! **Owner: lane ECK.** The players `ear-check` plays every game with (docs/ear.md, "GR4").
//!
//! | Player | Hears | What its score says |
//! | --- | --- | --- |
//! | **key oracle** | the key itself (`EarKey::right_answer`) | 100 %, or the judge, the key or the option count disagree |
//! | **ideal observer** (`audibility.rs`, [`observer`]) | every option's `sound/1` distance to the right clip, to 1 JND | what a perfect listener scores: at least 95 % at the top level |
//! | **random** | nothing, draws uniformly | chance |
//! | **deaf** | nothing, answers by the item's number (`index mod options`) | chance: the key does not follow the order of play |
//! | **bias bots** | the *options*, never the clip: their position (first, last) and what the option's clip is made of (the longest, the loudest, the highest, the one with the most notes) | chance +- 3 %: a bot that beats chance has found a leak (the right answer is not independent of the property) |
//!
//! A bias bot picks the extreme option of one ordering and breaks ties by a random draw, so a property the options share
//! (every clip the same length) leaves it at chance, and one the generator ties to the answer shows at once. The
//! properties are read from the notes (no render), so they run over 10,000 items per level in a second; the loudest
//! option **as rendered** (`sound/1`'s pre-gain loudness) runs on the measured sample, with a binomial allowance
//! ([`rendered_loudest`]).
//!
//! Every answer goes through the real judge (`ear/1`, [`scorers::judge`]); draws come from `enth-dsp`'s pinned `Rng`,
//! keyed from the run's seed, so a run repeats exactly.

use enth_challenge::ear::scorers::{self, EarAnswer};
use enth_challenge::score::ScorerId;
use enth_dsp::util::rng::Rng;

use super::audibility::Measured;
use super::game::Item;

/// A player that does not look at the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Player {
    /// Reads the key.
    Oracle,
    /// Draws uniformly.
    Random,
    /// Answers `index mod options`.
    Deaf,
    /// Always the first option.
    First,
    /// Always the last option.
    Last,
    /// The longest option's clip.
    Longest,
    /// The option with the most energy in its notes (velocity squared times hold).
    Loudest,
    /// The option whose highest note is highest.
    Highest,
    /// The option with the most notes.
    Densest,
}

impl Player {
    /// Every player, the oracle first.
    pub const ALL: [Player; 9] = [
        Player::Oracle,
        Player::Random,
        Player::Deaf,
        Player::First,
        Player::Last,
        Player::Longest,
        Player::Loudest,
        Player::Highest,
        Player::Densest,
    ];

    /// The player's name in reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Player::Oracle => "oracle",
            Player::Random => "random",
            Player::Deaf => "deaf",
            Player::First => "first",
            Player::Last => "last",
            Player::Longest => "longest",
            Player::Loudest => "loudest",
            Player::Highest => "highest",
            Player::Densest => "densest",
        }
    }

    /// Whether the player is a bias bot (it must stay at chance +- 3 %).
    #[must_use]
    pub fn is_bias_bot(self) -> bool {
        !matches!(self, Player::Oracle | Player::Random | Player::Deaf)
    }
}

/// The index of the largest value; ties (within 1e-9, relative) are broken by a draw.
fn extreme(values: &[f64], rng: &mut Rng) -> usize {
    let top = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let slack = 1e-9 * top.abs().max(1.0);
    let ties: Vec<usize> = values
        .iter()
        .enumerate()
        .filter(|(_, v)| (top - **v).abs() <= slack)
        .map(|(i, _)| i)
        .collect();
    ties[rng.below(ties.len() as u64) as usize]
}

/// The option `player` picks for `item` (`item.options > 0`).
pub fn pick(player: Player, item: &Item, rng: &mut Rng) -> usize {
    let n = item.options;
    let by = |f: &dyn Fn(&super::game::Cue) -> f64, rng: &mut Rng| {
        let v: Vec<f64> = item.cues.iter().map(f).collect();
        extreme(&v, rng)
    };
    match player {
        Player::Oracle => match item.ekey.right_answer() {
            EarAnswer::Choice(i) => usize::from(i),
            _ => 0,
        },
        Player::Random => rng.below(n as u64) as usize,
        Player::Deaf => item.index as usize % n,
        Player::First => 0,
        Player::Last => n - 1,
        Player::Longest => by(&|c| f64::from(c.length_ms), rng),
        Player::Loudest => by(&|c| c.energy, rng),
        Player::Highest => by(&|c| f64::from(c.top), rng),
        Player::Densest => by(&|c| c.notes as f64, rng),
    }
}

/// What the judge made of one player's answers.
#[derive(Clone, Debug, PartialEq)]
pub struct Tally {
    /// The player.
    pub name: &'static str,
    /// Right answers.
    pub right: u64,
    /// Items played.
    pub items: u64,
    /// The expected score of a uniform guesser on the same items: the mean of `1 / options`.
    pub chance: f64,
}

impl Tally {
    /// A new, empty tally.
    #[must_use]
    pub fn new(name: &'static str) -> Tally {
        Tally {
            name,
            right: 0,
            items: 0,
            chance: 0.0,
        }
    }

    /// The share of items right.
    #[must_use]
    pub fn accuracy(&self) -> f64 {
        if self.items == 0 {
            0.0
        } else {
            self.right as f64 / self.items as f64
        }
    }

    /// The expected score of a uniform guesser, as a share.
    #[must_use]
    pub fn chance(&self) -> f64 {
        if self.items == 0 {
            0.0
        } else {
            self.chance / self.items as f64
        }
    }

    /// How far above (or below) chance the player scored.
    #[must_use]
    pub fn edge(&self) -> f64 {
        self.accuracy() - self.chance()
    }

    /// The standard error of the player's score at chance, for a binomial allowance.
    #[must_use]
    pub fn sigma(&self) -> f64 {
        if self.items == 0 {
            return 0.0;
        }
        let p = self.chance();
        (p * (1.0 - p) / self.items as f64).sqrt()
    }

    fn add(&mut self, right: bool, options: usize) {
        self.items += 1;
        self.right += u64::from(right);
        self.chance += 1.0 / options.max(1) as f64;
    }
}

/// Judge a choice through the real scorer, `ear/1`. An answer the judge refuses is wrong (and reported by the caller as
/// a failure through [`judged`]).
fn judge(item: &Item, answer: usize) -> Result<bool, String> {
    let a = u8::try_from(answer).map_err(|_| format!("option {answer} does not fit a byte"))?;
    scorers::judge(&ScorerId::EAR_V1, &item.ekey, &EarAnswer::Choice(a))
        .map(|j| j.right)
        .map_err(|e| e.to_string())
}

/// A fresh set of tallies, one per [`Player`].
#[must_use]
pub fn tallies() -> Vec<Tally> {
    Player::ALL.iter().map(|p| Tally::new(p.name())).collect()
}

/// Play every player over one item (one with options), through the judge, into `tallies`.
///
/// # Errors
/// The judge refused an answer (a key and an option count that disagree): named for the report.
pub fn play_item(item: &Item, rng: &mut Rng, tallies: &mut [Tally]) -> Result<(), String> {
    if item.options == 0 {
        return Ok(());
    }
    for (p, t) in Player::ALL.iter().zip(tallies) {
        let answer = pick(*p, item, rng);
        let right = judge(item, answer).map_err(|e| {
            format!(
                "game {} level {} item {} (seed {}): {e}",
                item.game, item.level, item.index, item.seed
            )
        })?;
        t.add(right, item.options);
    }
    Ok(())
}

/// Tap back's key through the judge: the oracle answers the key's own onsets.
///
/// # Errors
/// The judge refuses the key's own answer, or does not call it right.
pub fn tap_oracle(item: &Item) -> Result<(), String> {
    let j = scorers::judge(&ScorerId::TAPS_V1, &item.ekey, &item.ekey.right_answer())
        .map_err(|e| e.to_string())?;
    if j.right {
        Ok(())
    } else {
        Err(format!(
            "item {}: the judge does not accept the key's own taps",
            item.index
        ))
    }
}

/// The ideal observer's answer to a measured item: it picks uniformly among the options it cannot tell from the right
/// answer's clip (within one JND), or among all of them when it does not hear the change at all.
pub fn observer(item: &Item, m: &Measured, rng: &mut Rng) -> usize {
    let n = item.options;
    if matches!(m.detect(item.layout), Some(Ok(d)) if d < 1.0) {
        return rng.below(n as u64) as usize;
    }
    let mut set = vec![item.key];
    set.extend(
        (0..n).filter(|&j| j != item.key && m.distance(item.key, j).is_some_and(|d| d < 1.0)),
    );
    set[rng.below(set.len() as u64) as usize]
}

/// The ideal observer through the judge over the measured items.
///
/// # Errors
/// The judge refused an answer.
pub fn play_observer(items: &[(Item, Measured)], rng: &mut Rng) -> Result<Tally, String> {
    let mut t = Tally::new("observer");
    for (item, m) in items {
        if item.options == 0 {
            continue;
        }
        let right = judge(item, observer(item, m, rng))?;
        t.add(right, item.options);
    }
    Ok(t)
}

/// The option that renders loudest: `sound/1`'s pre-gain loudness of each option's clip (ties and unknown by a draw).
pub fn rendered_loudest(m: &Measured, rng: &mut Rng) -> usize {
    let v: Vec<f64> = m
        .loudness
        .iter()
        .map(|l| l.unwrap_or(f64::NEG_INFINITY))
        .collect();
    extreme(&v, rng)
}

/// The loudest-as-rendered bot over the measured items, through the judge.
///
/// # Errors
/// The judge refused an answer.
pub fn play_rendered_loudest(items: &[(Item, Measured)], rng: &mut Rng) -> Result<Tally, String> {
    let mut t = Tally::new("loudest-render");
    for (item, m) in items {
        if item.options == 0 {
            continue;
        }
        let right = judge(item, rendered_loudest(m, rng))?;
        t.add(right, item.options);
    }
    Ok(t)
}
