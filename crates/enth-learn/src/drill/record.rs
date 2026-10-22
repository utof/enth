//! **Owner: lane DRL.** Progress's `drills` field (additive, still `enth.progress/1`; docs/learn.md, "Ear skills,
//! bursts and drills"): what the Ear view shows and the workout composer reads.
//!
//! ```json
//! "drills":{
//!   "games":{"5":{"level":3,"best":4,"day":20747,"sessions":4,"items":57,"right":44}},
//!   "skills":{"svf.cutoff":{"game":3,"day":20747,"sessions":2,"items":80,"right":62,"thresholds":[[20740,"st",2400],[20747,"st",2100]]}},
//!   "days":[20740,20747],
//!   "workout":20747
//! }
//! ```
//!
//! - **`games`** (by game id, the numbers of `EarSpec::game`): the level the game's staircase ended at (the next
//!   session starts there, or one below after a break), the hardest level ever reached, the day last played, and
//!   how many sessions, items and right answers.
//! - **`skills`**: per skill the game that last drilled it (the composer varies the vehicle), accuracy over all its
//!   items, and **thresholds over time in physical units**: one point a day, `[day, unit, thousandths]`, the better
//!   of a day's measures, the latest [`HISTORY_MAX`] days kept.
//! - **`days`**: the days practised, a cumulative calendar that is never lost.
//! - **`workout`**: the last day a workout was played to its end, which is what makes the composer's workout *end*
//!   ("done for today"); omitted until one is.
//!
//! **Read as leniently as the rest of progress**: unknown fields at any level are kept and written back; an entry
//! this build cannot decode is kept verbatim; a part that is not the expected JSON shape is kept until a readable one
//! replaces it. **Its merge is a join** (commutative, associative, idempotent; `tests/drills.rs` proves it as
//! properties): per game and per skill the entry with more sessions wins (then the later day and so on, down to the
//! kept text), the threshold series unite (a day's better measure, then the latest [`HISTORY_MAX`]), and the
//! practised days unite. No streak is stored or computed anywhere: a calendar of days practised cannot be broken.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::Value;

use super::{Summary, Unit};
use crate::progress::{Extra, Merged, merge_extra};
use crate::review::{Day, Skill};

/// The most threshold points kept per skill: one a day, the latest.
pub const HISTORY_MAX: usize = 64;

/// One threshold measure: a value in a unit, in thousandths (`2400` is 2.4 semitones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// The unit the value is in.
    pub unit: Unit,
    /// The value in thousandths of the unit.
    pub milli: u32,
}

impl Point {
    /// The thousandths of `value`, clamped to what the file holds.
    pub fn from_value(unit: Unit, value: f64) -> Point {
        let milli = (value * 1000.0).round().clamp(0.0, f64::from(u32::MAX));
        Point {
            unit,
            milli: milli as u32,
        }
    }

    /// The value in the unit.
    pub fn value(&self) -> f64 {
        f64::from(self.milli) / 1000.0
    }

    /// Whether `self` is the one to keep over `other` on the same day: the better measure when the units agree
    /// (smaller for a physical unit, greater for a level), else the greater unit text. A total order, so the merge
    /// stays a join.
    fn beats(self, other: Point) -> bool {
        if self.unit == other.unit {
            if self.unit.better_is_lower() {
                self.milli < other.milli
            } else {
                self.milli > other.milli
            }
        } else {
            self.unit.key() > other.unit.key()
        }
    }
}

/// One game's staircase standing (progress's `drills.games`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameDrill {
    /// The level the staircase ended at in the latest session: where the next one starts.
    pub level: u8,
    /// The hardest level ever reached.
    pub best: u8,
    /// The day last played.
    pub day: Day,
    /// How many sessions have been played.
    pub sessions: u32,
    /// Items asked over all sessions.
    pub items: u32,
    /// Right answers over all sessions.
    pub right: u32,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl Default for GameDrill {
    /// A game never played: level 1.
    fn default() -> Self {
        GameDrill {
            level: 1,
            best: 1,
            day: 0,
            sessions: 0,
            items: 0,
            right: 0,
            extra: Extra::new(),
        }
    }
}

impl GameDrill {
    /// The merge's order: more sessions, then the later day, the higher level, best, items, right, the greater kept
    /// fields' text.
    fn rank(&self) -> (u32, Day, u8, u8, u32, u32, String) {
        let extra = serde_json::to_string(&self.extra).unwrap_or_default();
        (
            self.sessions,
            self.day,
            self.level,
            self.best,
            self.items,
            self.right,
            extra,
        )
    }

    fn read(value: &Value) -> Option<GameDrill> {
        let obj = value.as_object()?;
        let mut g = GameDrill::default();
        let mut has_level = false;
        for (k, v) in obj {
            match k.as_str() {
                "level" => {
                    g.level = u8::try_from(v.as_u64()?).ok().filter(|l| *l >= 1)?;
                    has_level = true;
                }
                "best" => g.best = u8::try_from(v.as_u64()?).ok().filter(|l| *l >= 1)?,
                "day" => g.day = Day::try_from(v.as_u64()?).ok()?,
                "sessions" => g.sessions = u32::try_from(v.as_u64()?).ok()?,
                "items" => g.items = u32::try_from(v.as_u64()?).ok()?,
                "right" => g.right = u32::try_from(v.as_u64()?).ok()?,
                _ => {
                    g.extra.insert(k.clone(), v.clone());
                }
            }
        }
        g.best = g.best.max(g.level);
        has_level.then_some(g)
    }
}

impl Serialize for GameDrill {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("level", &self.level)?;
        m.serialize_entry("best", &self.best)?;
        m.serialize_entry("day", &self.day)?;
        m.serialize_entry("sessions", &self.sessions)?;
        m.serialize_entry("items", &self.items)?;
        m.serialize_entry("right", &self.right)?;
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// One skill's drill standing (progress's `drills.skills`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillDrill {
    /// The game that last drilled it: the composer varies the vehicle from day to day.
    pub game: u8,
    /// The day last drilled.
    pub day: Day,
    /// How many drill sessions have asked it.
    pub sessions: u32,
    /// Items asked on it over all sessions.
    pub items: u32,
    /// Right answers on it.
    pub right: u32,
    /// Its thresholds over time, one point a day, at most [`HISTORY_MAX`], by day.
    pub thresholds: BTreeMap<Day, Point>,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl SkillDrill {
    /// A skill first drilled in `game`.
    pub fn new(game: u8) -> SkillDrill {
        SkillDrill {
            game,
            day: 0,
            sessions: 0,
            items: 0,
            right: 0,
            thresholds: BTreeMap::new(),
            extra: Extra::new(),
        }
    }

    /// The latest threshold point, if any.
    pub fn latest(&self) -> Option<(Day, Point)> {
        self.thresholds.iter().next_back().map(|(d, p)| (*d, *p))
    }

    /// The merge's order for the scalar part: more sessions, then the later day, the greater game, items, right, the
    /// greater kept fields' text.
    fn rank(&self) -> (u32, Day, u8, u32, u32, String) {
        let extra = serde_json::to_string(&self.extra).unwrap_or_default();
        (
            self.sessions,
            self.day,
            self.game,
            self.items,
            self.right,
            extra,
        )
    }

    /// Note a measure on `day`: the better of the day's, then only the latest [`HISTORY_MAX`] days stay.
    pub(crate) fn note(&mut self, day: Day, point: Point) {
        match self.thresholds.get_mut(&day) {
            Some(mine) => {
                if point.beats(*mine) {
                    *mine = point;
                }
            }
            None => {
                self.thresholds.insert(day, point);
            }
        }
        self.trim();
    }

    /// Keep the latest [`HISTORY_MAX`] days.
    fn trim(&mut self) {
        while self.thresholds.len() > HISTORY_MAX {
            self.thresholds.pop_first();
        }
    }

    /// The join: the scalar part (and the kept fields) of the entry that ranks higher, the threshold series united
    /// (a day's better measure) and cut to the latest [`HISTORY_MAX`] days.
    fn merge(&mut self, other: &SkillDrill) {
        let mut thresholds = std::mem::take(&mut self.thresholds);
        for (day, point) in &other.thresholds {
            match thresholds.get_mut(day) {
                Some(mine) => {
                    if point.beats(*mine) {
                        *mine = *point;
                    }
                }
                None => {
                    thresholds.insert(*day, *point);
                }
            }
        }
        if other.rank() > self.rank() {
            self.clone_from(other);
        }
        self.thresholds = thresholds;
        self.trim();
    }

    fn read(value: &Value) -> Option<SkillDrill> {
        let obj = value.as_object()?;
        let mut d = SkillDrill::new(0);
        let mut has_game = false;
        for (k, v) in obj {
            match k.as_str() {
                "game" => {
                    d.game = u8::try_from(v.as_u64()?).ok().filter(|g| *g >= 1)?;
                    has_game = true;
                }
                "day" => d.day = Day::try_from(v.as_u64()?).ok()?,
                "sessions" => d.sessions = u32::try_from(v.as_u64()?).ok()?,
                "items" => d.items = u32::try_from(v.as_u64()?).ok()?,
                "right" => d.right = u32::try_from(v.as_u64()?).ok()?,
                "thresholds" => {
                    for p in v.as_array()? {
                        let p = p.as_array().filter(|p| p.len() == 3)?;
                        let day = Day::try_from(p[0].as_u64()?).ok()?;
                        let unit = Unit::from_key(p[1].as_str()?)?;
                        let milli = u32::try_from(p[2].as_u64()?).ok()?;
                        d.note(day, Point { unit, milli });
                    }
                }
                _ => {
                    d.extra.insert(k.clone(), v.clone());
                }
            }
        }
        has_game.then_some(d)
    }
}

impl Serialize for SkillDrill {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("game", &self.game)?;
        m.serialize_entry("day", &self.day)?;
        m.serialize_entry("sessions", &self.sessions)?;
        m.serialize_entry("items", &self.items)?;
        m.serialize_entry("right", &self.right)?;
        if !self.thresholds.is_empty() {
            let points: Vec<(Day, &str, u32)> = self
                .thresholds
                .iter()
                .map(|(d, p)| (*d, p.unit.key(), p.milli))
                .collect();
            m.serialize_entry("thresholds", &points)?;
        }
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// Progress's `drills` field (see the [module docs](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drills {
    /// Each game's staircase standing.
    pub games: BTreeMap<u8, GameDrill>,
    /// Each skill's drill standing.
    pub skills: BTreeMap<Skill, SkillDrill>,
    /// The days practised.
    pub days: BTreeSet<Day>,
    /// The last day a workout was played to its end (0: never): the composer's "done for today".
    pub workout_day: Day,
    /// Game entries this build could not decode, kept (never under a key `games` also holds).
    pub unread_games: Extra,
    /// Skill entries this build could not decode, kept (never under a key `skills` also holds).
    pub unread_skills: Extra,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

/// A game id as the file spells it: the plain number.
fn game_key(text: &str) -> Option<u8> {
    let n: u8 = text.parse().ok()?;
    (n >= 1 && n.to_string() == text).then_some(n)
}

impl Drills {
    /// Nothing to write: no entry, no day and nothing kept (progress then omits the field, so a file without drills
    /// keeps its bytes).
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
            && self.skills.is_empty()
            && self.days.is_empty()
            && self.workout_day == 0
            && self.unread_games.is_empty()
            && self.unread_skills.is_empty()
            && self.extra.is_empty()
    }

    /// A game's standing (a game never played: level 1, no sessions).
    pub fn game(&self, id: u8) -> GameDrill {
        self.games.get(&id).cloned().unwrap_or_default()
    }

    /// Whether `day` was practised.
    pub fn practised(&self, day: Day) -> bool {
        self.days.contains(&day)
    }

    /// The days practised within the month of 30 days ending on `today`, oldest first: the Ear view's calendar.
    pub fn month(&self, today: Day) -> Vec<Day> {
        self.days
            .range(today.saturating_sub(29)..=today)
            .copied()
            .collect()
    }

    /// Apply a finished drill on `today` (see [`Progress::observe_drill`](crate::progress::Progress::observe_drill)).
    /// A drill with no answer leaves everything alone.
    pub fn observe(&mut self, s: &Summary, today: Day) {
        if s.items == 0 {
            return;
        }
        self.days.insert(today);
        self.unread_games.remove(&s.game.to_string());
        let g = self.games.entry(s.game).or_default();
        g.level = s.end_level.max(1);
        g.best = g.best.max(s.peak_level).max(g.level);
        g.day = g.day.max(today);
        g.sessions = g.sessions.saturating_add(1);
        g.items = g.items.saturating_add(s.items);
        g.right = g.right.saturating_add(s.right);
        for (skill, tally) in &s.skills {
            let e = self.touch(skill, s.game, today);
            e.items = e.items.saturating_add(tally.total);
            e.right = e.right.saturating_add(tally.right);
        }
        if let Some(m) = &s.measured {
            // The measure rides on the skill's session when it was asked, else it counts as one of its own.
            if !s.skills.contains_key(&m.skill) {
                self.touch(&m.skill, s.game, today);
            }
            if let Some(e) = self.skills.get_mut(&m.skill) {
                e.note(today, m.point);
            }
        }
        self.drop_kept();
    }

    /// Note that the day's workout was played to its end on `today`.
    pub fn finish_workout(&mut self, today: Day) {
        self.workout_day = self.workout_day.max(today);
        self.days.insert(today);
        self.drop_kept();
    }

    /// `skill`'s entry, counted as one more session of `game` on `today`.
    fn touch(&mut self, skill: &Skill, game: u8, today: Day) -> &mut SkillDrill {
        self.unread_skills.remove(skill.as_str());
        let e = self
            .skills
            .entry(skill.clone())
            .or_insert_with(|| SkillDrill::new(game));
        e.game = game;
        e.day = e.day.max(today);
        e.sessions = e.sessions.saturating_add(1);
        e
    }

    /// The join with `other` (see the [module docs](self)).
    pub fn merge(&mut self, other: &Drills) {
        for (id, theirs) in &other.games {
            match self.games.get_mut(id) {
                Some(mine) => {
                    if theirs.rank() > mine.rank() {
                        mine.clone_from(theirs);
                    }
                }
                None => {
                    self.games.insert(*id, theirs.clone());
                }
            }
        }
        for (skill, theirs) in &other.skills {
            match self.skills.get_mut(skill) {
                Some(mine) => mine.merge(theirs),
                None => {
                    self.skills.insert(skill.clone(), theirs.clone());
                }
            }
        }
        self.days.extend(other.days.iter().copied());
        self.workout_day = self.workout_day.max(other.workout_day);
        merge_extra(&mut self.unread_games, &other.unread_games);
        merge_extra(&mut self.unread_skills, &other.unread_skills);
        merge_extra(&mut self.extra, &other.extra);
        self.drop_kept();
    }

    /// A readable part replaces a kept one that was not (written once, never twice), and a kept entry gives way to
    /// a readable one under the same key.
    fn drop_kept(&mut self) {
        self.unread_games
            .retain(|k, _| !self.games.keys().any(|id| id.to_string() == *k));
        self.unread_skills
            .retain(|k, _| !self.skills.keys().any(|s| s.as_str() == k));
        if !self.games.is_empty() || !self.unread_games.is_empty() {
            self.extra.remove("games");
        }
        if !self.skills.is_empty() || !self.unread_skills.is_empty() {
            self.extra.remove("skills");
        }
        if !self.days.is_empty() {
            self.extra.remove("days");
        }
        if self.workout_day > 0 {
            self.extra.remove("workout");
        }
    }

    /// Read the field leniently: `None` when it is not an object (progress then keeps it verbatim).
    pub fn read(value: &Value) -> Option<Drills> {
        let obj = value.as_object()?;
        let mut d = Drills::default();
        for (k, v) in obj {
            match (k.as_str(), v) {
                ("games", Value::Object(games)) => {
                    for (key, entry) in games {
                        match (game_key(key), GameDrill::read(entry)) {
                            (Some(id), Some(g)) => {
                                d.games.insert(id, g);
                            }
                            _ => {
                                d.unread_games.insert(key.clone(), entry.clone());
                            }
                        }
                    }
                }
                ("skills", Value::Object(skills)) => {
                    for (text, entry) in skills {
                        match (Skill::parse(text), SkillDrill::read(entry)) {
                            (Some(skill), Some(e)) => {
                                d.skills.insert(skill, e);
                            }
                            _ => {
                                d.unread_skills.insert(text.clone(), entry.clone());
                            }
                        }
                    }
                }
                ("days", Value::Array(days)) => {
                    let parsed: Option<BTreeSet<Day>> = days
                        .iter()
                        .map(|v| v.as_u64().and_then(|n| Day::try_from(n).ok()))
                        .collect();
                    match parsed {
                        Some(set) => d.days = set,
                        None => {
                            d.extra.insert(k.clone(), v.clone());
                        }
                    }
                }
                ("workout", Value::Number(n))
                    if n.as_u64().is_some_and(|n| Day::try_from(n).is_ok()) =>
                {
                    d.workout_day = n.as_u64().and_then(|n| Day::try_from(n).ok()).unwrap_or(0);
                }
                _ => {
                    d.extra.insert(k.clone(), v.clone());
                }
            }
        }
        d.drop_kept();
        Some(d)
    }
}

impl Serialize for Drills {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        if !self.games.is_empty() || !self.unread_games.is_empty() {
            m.serialize_entry("games", &Merged(&self.games, &self.unread_games))?;
        }
        if !self.skills.is_empty() || !self.unread_skills.is_empty() {
            m.serialize_entry("skills", &Merged(&self.skills, &self.unread_skills))?;
        }
        if !self.days.is_empty() {
            m.serialize_entry("days", &self.days)?;
        }
        if self.workout_day > 0 {
            m.serialize_entry("workout", &self.workout_day)?;
        }
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}
