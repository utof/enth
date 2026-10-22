//! **Owner: lane DRL** (Drill player, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); written by
//! lane DRL. The reference is docs/ear.md, "Workouts", and docs/learn.md, "Ear skills, bursts and drills".
//!
//! The pure **workout composer**: up to 3 games of 12 to 20 items, about 10 to 15 minutes, chosen by due skills,
//! ending in "done for today" with "one more" optional. **Deterministic:** the same progress file and catalogue on the
//! same day give the same workout, seeds included; the day comes from the shell (days since 1970-01-01, UTC, as
//! Review's), and nothing here reads a clock or draws from a global RNG.
//!
//! - **Due skills choose the games.** Skills are ordered as Review's queue orders them (due first, then the lower box,
//!   the earlier due day, a keyed draw of the day and the skill). Each due skill gets a *vehicle*: of the games that
//!   can drill it, one in a fixed rotation by how many sessions it has had, never the game that drilled it last. So a
//!   due `svf.cutoff` comes back as Spot the change one day, Dial it in another and Frequency finder later: varied
//!   retrieval, which is what generalises (Wright 2010). A game takes up to [`MAX_FOCUS`] skills; each gets a
//!   five-item burst ([`BURST_ITEMS`]) that moves its Review box.
//! - **Each category is met alone before confusable ones mix** (m3/M3, attack/decay, room/decay): two skills the
//!   [`Catalog`] calls confusable share a segment only after each has been drilled once.
//! - **After a break a game starts one level below** where its staircase ended ([`start_level`]), silently: no count
//!   of days missed anywhere.
//! - **It ends.** [`compose`] gives at most [`MAX_SEGMENTS`] segments, and none once the day's workout was played to
//!   its end ([`Progress::finish_workout`]) or nothing is due: *done for today*. [`one_more`] is the optional extra
//!   segment, from the weakest skills.
//! - **Note naming is opt-in and never in the default workout** ([`game::NOTE_NAMING`] is skipped whatever the
//!   catalogue says). No streaks, no loss messages, no daily quests, no currency, no leaderboards: not here, by design.
//!
//! The catalogue is data the shell fills from the build's tables (`enth-music`'s games, ladders and skill slugs; the
//! parameter games' observable controls from `enth-challenge`), so the composer needs no table of its own and a ladder
//! that grows needs no change here.

use std::collections::BTreeSet;

use enth_dsp::util::rng::{derive, fnv1a64};

use crate::drill::{DrillSpec, MAX_ITEMS, MAX_SEED, game, start_level};
use crate::progress::Progress;
use crate::review::{BURST_ITEMS, Day, Skill, Standing, sort_standings, standing};

/// The most games in a workout.
pub const MAX_SEGMENTS: usize = 3;

/// The most skills a game bursts on in one segment (3 bursts of 5 are 15 items).
pub const MAX_FOCUS: usize = 3;

/// A game the composer may use: its id, its ladder, how long a session is, and the skills it can drill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    /// The game's id (`EarSpec::game`).
    pub id: u8,
    /// Its name for the view ("Spot the change").
    pub title: String,
    /// How many levels its ladder has in this build.
    pub levels: u8,
    /// Items in one session: 12 to 20 for most games (a segment never has fewer than five per focus skill).
    pub items: u8,
    /// The skills it can drill: `module.param` for a parameter game, `ear.*` for a music game.
    pub skills: Vec<Skill>,
}

impl GameInfo {
    /// A game entry.
    pub fn new(id: u8, title: &str, levels: u8, items: u8, skills: Vec<Skill>) -> GameInfo {
        GameInfo {
            id,
            title: title.to_owned(),
            levels,
            items,
            skills,
        }
    }
}

/// What the composer chooses from: the games of this build and the pairs of skills that are easily confused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    games: Vec<GameInfo>,
    confusable: Vec<(Skill, Skill)>,
}

impl Catalog {
    /// A catalogue of `games`, in the order the Ear view lists them.
    pub fn new(games: Vec<GameInfo>) -> Catalog {
        Catalog {
            games,
            confusable: Vec::new(),
        }
    }

    /// Say that `a` and `b` are easily confused (m3 and M3): they mix only after each was met alone.
    #[must_use]
    pub fn confusing(mut self, a: Skill, b: Skill) -> Catalog {
        self.confusable.push((a, b));
        self
    }

    /// The games, in order.
    pub fn games(&self) -> &[GameInfo] {
        &self.games
    }

    /// A game by id.
    pub fn game(&self, id: u8) -> Option<&GameInfo> {
        self.games.iter().find(|g| g.id == id)
    }

    /// Whether `a` and `b` are easily confused.
    fn confuses(&self, a: &Skill, b: &Skill) -> bool {
        self.confusable
            .iter()
            .any(|(x, y)| (x == a && y == b) || (x == b && y == a))
    }

    /// The games the default workout may use: not the opt-in ones, and only those with a skill to drill.
    fn vehicles(&self) -> impl Iterator<Item = &GameInfo> {
        self.games
            .iter()
            .filter(|g| g.id != game::NOTE_NAMING && !g.skills.is_empty())
    }

    /// Every skill some default game can drill.
    fn skills(&self) -> BTreeSet<&Skill> {
        self.vehicles().flat_map(|g| g.skills.iter()).collect()
    }
}

/// One game of a workout: which game, which skills it bursts on, and where its staircase starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// The game's id.
    pub game: u8,
    /// Its name for the view.
    pub title: String,
    /// The skills to burst on, weakest first (1 to [`MAX_FOCUS`]), five items each.
    pub focus: Vec<Skill>,
    /// How many items (the game's session length, at least five per focus skill).
    pub items: u8,
    /// How many levels the game's ladder has.
    pub levels: u8,
    /// The level the staircase starts at: where it ended last time, or one below after a break.
    pub start_level: u8,
    /// This segment starts one level below because of a break. The view says nothing about it.
    pub warm_up: bool,
    /// The drill's seed, a keyed draw of the day, the game and the skills.
    pub seed: u64,
}

impl Segment {
    /// The drill this segment plays, with ear generator `generator`.
    pub fn drill_spec(&self, generator: u8) -> DrillSpec {
        DrillSpec {
            game: self.game,
            generator,
            seed: self.seed,
            items: self.items,
            levels: self.levels,
            start: self.start_level,
            focus: self.focus.clone(),
        }
    }
}

/// A day's workout (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workout {
    /// The day it was composed for.
    pub day: Day,
    /// The games to play, in order: at most [`MAX_SEGMENTS`], none when it is done for today.
    pub segments: Vec<Segment>,
    /// Some segment starts a level below (a warm-up after a break).
    pub warm_up: bool,
    /// Nothing is left: the day's workout was played to its end, or no skill is due. The view says "done for today".
    pub done_for_today: bool,
}

impl Workout {
    /// How many items the workout asks in all.
    pub fn items(&self) -> u32 {
        self.segments.iter().map(|s| u32::from(s.items)).sum()
    }
}

/// The workout for `progress` on `today`: the same progress, catalogue and day give the same workout.
pub fn compose(progress: &Progress, catalog: &Catalog, today: Day) -> Workout {
    let done = progress.drills.workout_day == today && today > 0;
    let segments = if done {
        Vec::new()
    } else {
        let mut due: Vec<Standing> = standings(progress, catalog, today)
            .into_iter()
            .filter(|s| s.due)
            .collect();
        sort_standings(&mut due, today);
        assemble(progress, catalog, today, &due, MAX_SEGMENTS)
    };
    Workout {
        day: today,
        warm_up: segments.iter().any(|s| s.warm_up),
        done_for_today: segments.is_empty(),
        segments,
    }
}

/// The optional extra after "done for today": one more segment from the weakest skills (due ones first, then the
/// lowest boxes). `None` only when the catalogue has nothing to drill.
pub fn one_more(progress: &Progress, catalog: &Catalog, today: Day) -> Option<Segment> {
    let mut all = standings(progress, catalog, today);
    sort_standings(&mut all, today);
    assemble(progress, catalog, today, &all, 1)
        .into_iter()
        .next()
}

/// Every drillable skill's standing on `today` (a skill never rung is box 1 and due).
fn standings(progress: &Progress, catalog: &Catalog, today: Day) -> Vec<Standing> {
    catalog
        .skills()
        .into_iter()
        .map(|skill| standing(progress, skill, today))
        .collect()
}

/// Whether `skill` was drilled or rung at least once: it has been "met alone".
fn met(progress: &Progress, skill: &Skill) -> bool {
    progress
        .review
        .skills
        .get(skill)
        .is_some_and(|b| b.rounds > 0)
        || progress
            .drills
            .skills
            .get(skill)
            .is_some_and(|d| d.sessions > 0)
}

/// The game that drills `skill` today: of the games that can, one in a fixed rotation by the skill's sessions so far,
/// moved on a place when it would be the game that drilled it last.
fn vehicle<'a>(progress: &Progress, catalog: &'a Catalog, skill: &Skill) -> Option<&'a GameInfo> {
    let mut games: Vec<&GameInfo> = catalog
        .vehicles()
        .filter(|g| g.skills.contains(skill))
        .collect();
    if games.is_empty() {
        return None;
    }
    games.sort_by_key(|g| g.id);
    let known = progress.drills.skills.get(skill);
    let sessions = u64::from(known.map_or(0, |d| d.sessions));
    let n = games.len() as u64;
    let mut at = (fnv1a64(skill.as_str().as_bytes()) % n + sessions) % n;
    if n > 1 && known.is_some_and(|d| d.sessions > 0 && games[at as usize].id == d.game) {
        at = (at + 1) % n;
    }
    Some(games[at as usize])
}

/// A segment being filled.
struct Draft<'a> {
    info: &'a GameInfo,
    focus: Vec<Skill>,
}

/// Group `order` (skills, weakest first) into at most `max` segments by their vehicles.
fn assemble(
    progress: &Progress,
    catalog: &Catalog,
    today: Day,
    order: &[Standing],
    max: usize,
) -> Vec<Segment> {
    let mut drafts: Vec<Draft<'_>> = Vec::new();
    for s in order {
        let Some(info) = vehicle(progress, catalog, &s.skill) else {
            continue;
        };
        if let Some(at) = drafts.iter().position(|d| d.info.id == info.id) {
            let d = &mut drafts[at];
            let mixes = d.focus.iter().all(|c| {
                !catalog.confuses(c, &s.skill) || (met(progress, c) && met(progress, &s.skill))
            });
            if d.focus.len() < MAX_FOCUS && mixes {
                d.focus.push(s.skill.clone());
            }
        } else if drafts.len() < max {
            drafts.push(Draft {
                info,
                focus: vec![s.skill.clone()],
            });
        }
    }
    drafts
        .into_iter()
        .map(|d| segment(progress, today, d))
        .collect()
}

/// A draft as a segment: its length, its start and its seed.
fn segment(progress: &Progress, today: Day, d: Draft<'_>) -> Segment {
    let (start, warm_up) = start_level(&progress.drills, d.info.id, d.info.levels, today);
    let burst_items = u8::try_from(d.focus.len() * usize::from(BURST_ITEMS)).unwrap_or(MAX_ITEMS);
    let names: Vec<&str> = d.focus.iter().map(Skill::as_str).collect();
    let key = format!("{}/{}", d.info.id, names.join(","));
    let seed = derive(
        derive(u64::from(today), fnv1a64(b"workout")),
        fnv1a64(key.as_bytes()),
    ) & MAX_SEED;
    Segment {
        game: d.info.id,
        title: d.info.title.clone(),
        items: d.info.items.max(burst_items).min(MAX_ITEMS),
        levels: d.info.levels,
        start_level: start,
        warm_up,
        seed,
        focus: d.focus,
    }
}
