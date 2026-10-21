//! **Owner: lane REV** (Review, after LUI; on the cut list). Pre-declared by lane S0l (R2). The reference is
//! docs/learn.md, "Review (lane REV)", and report 01 §3 step 6 and §5.
//!
//! **Leitner boxes per skill**, fed by the Good, Near and Bad rings of every Recreate round, and **the Review queue**:
//! 3–5 seeded sibling rounds of the challenges of passed lessons, the weakest due skills first. No streaks, no
//! `fsrs` (it pulls in ndarray, rayon and rand for about 50 skills), no model to train. Pure: no clock (the shell
//! passes the [`Day`]), no RNG (sibling seeds are keyed draws of the day, the step and the skill).
//!
//! # Skills and boxes
//!
//! A [`Skill`] is a module kind and a parameter key, `svf.cutoff` (`voice.glide`, `master.level` for the built-ins),
//! read off the round's target patch, so the player's module ids never matter. Each skill sits in a box, 1 to
//! [`BOXES`], and is due [`INTERVAL_DAYS`] after the round that last rang it:
//!
//! | Ring (the round's first submit) | Box |
//! | --- | --- |
//! | Good | up one, **unless the round was assisted** (a level-4 hint, a Reveal or "How close am I?") or only a row the target left alone rang it |
//! | Near | stays |
//! | Bad | down one (never below 1) |
//!
//! A round's rings are its **first submit's** structure rows ([`RoundRings::from_structure`]): after it the rings show
//! live, so later submits are feedback, not evidence. When several rows ring one skill (two oscillators' `wave`), the
//! worst verdict moves it. Routes and enable flags are not skills. A skill no round has rung counts as box 1, due.
//!
//! # The progress field
//!
//! [`Review`] is progress's `review` field (additive, no schema bump; docs/learn.md, "Progress v1"):
//!
//! ```json
//! "review":{"skills":{"svf.cutoff":{"box":2,"day":20747,"rounds":3}}}
//! ```
//!
//! `day` is the day of the last round that rang the skill (days since 1970-01-01, UTC, as the shell counts them) and
//! `rounds` how many rounds have. It is read leniently, as the rest of progress is: unknown fields at any level are
//! kept and written back, and an entry this build cannot read is kept verbatim. [`Review::merge`] is a join: per
//! skill, the entry with more rounds wins (then the later day, the higher box, the greater kept fields' text), so two
//! files merge the same either way round. The shell writes by re-reading the file and applying a round to it
//! ([`Progress::observe`](crate::progress::Progress::observe)), so one writer never loses a move.
//!
//! # The queue
//!
//! [`queue`] is a pure function of the progress, the installed lessons and the day: the same progress file on the
//! same day gives the same items, seeds included. Candidates are the Recreate steps of **completed** lessons whose
//! challenge can make a sibling (a draw or a seed target; a patch target has no seed). Skills are ordered due first,
//! then by box, then by due day (a skill no round has rung first), then by a keyed draw of the day and the skill (equal
//! skills take turns from day to day); each skill takes one candidate that exercises it (none twice), until
//! [`MAX_ITEMS`]; when fewer than [`MIN_ITEMS`] skills are due, the weakest of the rest top the queue up. Each item is
//! the step's challenge with a new seed ([`Challenge::with_seed`]), so it is a problem, never the round the lesson
//! played. Review gives no hint on the first try: the session's review round refuses one before the first submit.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use enth_challenge::model::Target;
use enth_challenge::spec::MAX_SEED;
use enth_challenge::v2::DrawAt;
use enth_challenge::{Challenge, StructureScore, Verdict};
use enth_dsp::util::rng::{derive, fnv1a64};
use enth_patch::{Builtin, ParamAddr, Patch, Target as PatchTarget};
use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::Value;

use crate::lesson::StepKind;
use crate::pack::{LessonBundle, LessonKey, StepId};
use crate::progress::{Extra, Merged, Progress, merge_extra};

/// A day: days since 1970-01-01 (UTC), as the shell counts them. Review reads no clock.
pub type Day = u32;

/// How many Leitner boxes: 1 (weakest) to 5.
pub const BOXES: u8 = 5;

/// Days from the round that last rang a skill until it is due again, by box (1 to [`BOXES`]).
pub const INTERVAL_DAYS: [u32; BOXES as usize] = [1, 2, 4, 8, 16];

/// The fewest items a queue holds when the passed lessons have that many reviewable steps.
pub const MIN_ITEMS: usize = 3;

/// The most items a queue holds.
pub const MAX_ITEMS: usize = 5;

/// The longest part of a skill's text (a module kind or a parameter key).
const MAX_PART: usize = 32;

// ---------------------------------------------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------------------------------------------

/// A skill: a module kind and a parameter key, `svf.cutoff` (see the [module docs](self)). Text of two parts of
/// lowercase letters, digits and `_`, so a newer build's module kind is just another skill.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Skill(String);

/// Whether `part` may be one half of a skill.
fn valid_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= MAX_PART
        && part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Skill {
    /// The skill of a module kind (`svf`, or `voice`, `master`) and a parameter key (`cutoff`).
    pub fn new(kind: &str, key: &str) -> Option<Skill> {
        (valid_part(kind) && valid_part(key)).then(|| Skill(format!("{kind}.{key}")))
    }

    /// Read a skill's text (`svf.cutoff`); `None` when it is not two valid parts.
    pub fn parse(text: &str) -> Option<Skill> {
        let (kind, key) = text.split_once('.')?;
        Skill::new(kind, key)
    }

    /// The skill `addr` exercises in `patch`: its module's kind (or the built-in) and the parameter key. `None` when
    /// `patch` has no such module.
    pub fn of(patch: &Patch, addr: &ParamAddr) -> Option<Skill> {
        let kind = match addr.target() {
            PatchTarget::Module(id) => patch.find(id)?.1.kind.key(),
            PatchTarget::Voice => Builtin::Voice.key(),
            PatchTarget::Master => Builtin::Master.key(),
        };
        Skill::new(kind, addr.param())
    }

    /// The text, `svf.cutoff`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The module kind (`svf`, `voice`).
    pub fn kind(&self) -> &str {
        self.0.split_once('.').map_or("", |(k, _)| k)
    }

    /// The parameter key (`cutoff`).
    pub fn key(&self) -> &str {
        self.0.split_once('.').map_or("", |(_, k)| k)
    }
}

impl fmt::Display for Skill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Boxes
// ---------------------------------------------------------------------------------------------------------------

/// The Leitner move of one ring: Good up one box (only when `may_rise`), Near stays, Bad down one; always within
/// 1 to [`BOXES`]. An unknown verdict (a newer build's) stays.
pub fn moved(level: u8, verdict: Verdict, may_rise: bool) -> u8 {
    let level = level.clamp(1, BOXES);
    match verdict {
        Verdict::Good if may_rise => (level + 1).min(BOXES),
        Verdict::Bad => (level - 1).max(1),
        _ => level,
    }
}

/// The days a skill in box `level` waits until it is due again.
pub fn interval(level: u8) -> u32 {
    INTERVAL_DAYS[usize::from(level.clamp(1, BOXES) - 1)]
}

/// One skill's box (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillBox {
    /// The box, 1 to [`BOXES`] (`"box"`).
    pub level: u8,
    /// The day of the last round that rang the skill.
    pub day: Day,
    /// How many rounds have rung it: the merge keeps the entry with more.
    pub rounds: u32,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl Default for SkillBox {
    /// A skill no round has rung: box 1, due at once.
    fn default() -> Self {
        SkillBox {
            level: 1,
            day: 0,
            rounds: 0,
            extra: Extra::new(),
        }
    }
}

impl SkillBox {
    /// The day it is due again: the last round's day plus its box's interval (day 0 for a skill no round has rung).
    pub fn due_day(&self) -> Day {
        if self.rounds == 0 {
            return 0;
        }
        self.day.saturating_add(interval(self.level))
    }

    /// Whether it is due on `today`.
    pub fn is_due(&self, today: Day) -> bool {
        today >= self.due_day()
    }

    /// The merge's order: more rounds, then the later day, the higher box, the greater kept fields' text.
    fn rank(&self) -> (u32, Day, u8, String) {
        let extra = serde_json::to_string(&self.extra).unwrap_or_default();
        (self.rounds, self.day, self.level, extra)
    }

    fn read(value: &Value) -> Option<SkillBox> {
        let obj = value.as_object()?;
        let mut b = SkillBox::default();
        let mut has_box = false;
        for (k, v) in obj {
            match k.as_str() {
                "box" => {
                    b.level = u8::try_from(v.as_u64()?)
                        .ok()
                        .filter(|l| (1..=BOXES).contains(l))?;
                    has_box = true;
                }
                "day" => b.day = Day::try_from(v.as_u64()?).ok()?,
                "rounds" => b.rounds = u32::try_from(v.as_u64()?).ok()?,
                _ => {
                    b.extra.insert(k.clone(), v.clone());
                }
            }
        }
        has_box.then_some(b)
    }
}

impl Serialize for SkillBox {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("box", &self.level)?;
        m.serialize_entry("day", &self.day)?;
        m.serialize_entry("rounds", &self.rounds)?;
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rings
// ---------------------------------------------------------------------------------------------------------------

/// One ring of a round, on a skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRing {
    /// The skill its parameter exercises.
    pub skill: Skill,
    /// The ring.
    pub verdict: Verdict,
    /// The target randomized the parameter (`false`: a control the player moved that the target left alone).
    pub randomized: bool,
}

/// A Recreate round as Review reads it: its first submit's rings by skill, and whether it was assisted (a level-4
/// hint, a Reveal or "How close am I?": then no box moves up). The session builds one when a played round ends; the
/// shell applies it to the progress file with the day ([`Progress::observe`](crate::progress::Progress::observe)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundRings {
    /// The rings, in the structure score's row order.
    pub rings: Vec<SkillRing>,
    /// The round was assisted.
    pub assisted: bool,
}

impl RoundRings {
    /// The rings of a structure score (the round's first submit) on `target`, the round's target patch: one per
    /// parameter row with a verdict this build knows. Route and enable-flag rows, rows whose address does not parse
    /// and rows on a module the target lacks are skipped.
    pub fn from_structure(
        structure: &StructureScore,
        target: &Patch,
        assisted: bool,
    ) -> RoundRings {
        let rings = structure
            .params
            .iter()
            .filter(|p| p.verdict != Verdict::Unknown)
            .filter(|p| !p.addr.contains("->") && !p.addr.ends_with(".enabled"))
            .filter_map(|p| {
                let addr = p.param_addr().ok()?;
                Some(SkillRing {
                    skill: Skill::of(target, &addr)?,
                    verdict: p.verdict,
                    randomized: p.randomized,
                })
            })
            .collect();
        RoundRings { rings, assisted }
    }

    /// One move per skill: its worst verdict (Bad over Near over Good), and whether it may rise (the round was not
    /// assisted and a randomized row rang it).
    pub fn moves(&self) -> BTreeMap<Skill, (Verdict, bool)> {
        let mut out: BTreeMap<Skill, (Verdict, bool)> = BTreeMap::new();
        for r in self.rings.iter().filter(|r| r.verdict != Verdict::Unknown) {
            let rise = !self.assisted && r.randomized;
            out.entry(r.skill.clone())
                .and_modify(|(v, may)| {
                    *v = (*v).max(r.verdict);
                    *may |= rise;
                })
                .or_insert((r.verdict, rise));
        }
        out
    }

    /// No ring to move a box.
    pub fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The progress field
// ---------------------------------------------------------------------------------------------------------------

/// Progress's `review` field: a box per skill (see the [module docs](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    /// The boxes by skill.
    pub skills: BTreeMap<Skill, SkillBox>,
    /// Skill entries this build could not decode, kept (never under a key `skills` also holds).
    pub unread: Extra,
    /// Fields from a newer build, kept.
    pub extra: Extra,
}

impl Review {
    /// Nothing to write: no box and nothing kept (progress then omits the field, so a file without one keeps its
    /// bytes).
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.unread.is_empty() && self.extra.is_empty()
    }

    /// A skill's box (a skill no round has rung: box 1, due at once).
    pub fn get(&self, skill: &Skill) -> SkillBox {
        self.skills.get(skill).cloned().unwrap_or_default()
    }

    /// Apply one round on `today`: each skill it rang moves by [`moved`], takes the day (never earlier than the one
    /// it had) and counts the round. A readable entry replaces a kept unreadable one under the same skill.
    pub fn observe(&mut self, rings: &RoundRings, today: Day) {
        for (skill, (verdict, may_rise)) in rings.moves() {
            self.unread.remove(skill.as_str());
            let b = self.skills.entry(skill).or_default();
            b.level = moved(b.level, verdict, may_rise);
            b.day = b.day.max(today);
            b.rounds = b.rounds.saturating_add(1);
        }
    }

    /// The join with `other`: per skill the entry that ranks higher (more rounds, the later day, the higher box, the
    /// greater kept fields' text), kept fields united. Commutative, associative, idempotent.
    pub fn merge(&mut self, other: &Review) {
        for (skill, theirs) in &other.skills {
            match self.skills.get_mut(skill) {
                Some(mine) => {
                    if theirs.rank() > mine.rank() {
                        mine.clone_from(theirs);
                    }
                }
                None => {
                    self.skills.insert(skill.clone(), theirs.clone());
                }
            }
        }
        merge_extra(&mut self.unread, &other.unread);
        self.unread
            .retain(|k, _| !self.skills.keys().any(|s| s.as_str() == k));
        merge_extra(&mut self.extra, &other.extra);
    }

    /// Read the field leniently: `None` when it is not an object (progress then keeps it verbatim).
    pub fn read(value: &Value) -> Option<Review> {
        let obj = value.as_object()?;
        let mut r = Review::default();
        for (k, v) in obj {
            match (k.as_str(), v.as_object()) {
                ("skills", Some(skills)) => {
                    for (text, entry) in skills {
                        match (Skill::parse(text), SkillBox::read(entry)) {
                            (Some(skill), Some(b)) => {
                                r.skills.insert(skill, b);
                            }
                            _ => {
                                r.unread.insert(text.clone(), entry.clone());
                            }
                        }
                    }
                }
                _ => {
                    r.extra.insert(k.clone(), v.clone());
                }
            }
        }
        Some(r)
    }
}

impl Serialize for Review {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        if !self.skills.is_empty() || !self.unread.is_empty() {
            m.serialize_entry("skills", &Merged(&self.skills, &self.unread))?;
        }
        for (k, v) in &self.extra {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------------------------------------------

/// The skills a challenge exercises: a draw's parameter items (kinds from its start), or a seed target's generated
/// parameters (kinds from the target). Empty for a patch target, which has no sibling, and for a challenge this build
/// cannot generate.
pub fn challenge_skills(challenge: &Challenge) -> BTreeSet<Skill> {
    match &challenge.target {
        Target::Draw(draw) => {
            let Ok(start) = challenge.start_patch() else {
                return BTreeSet::new();
            };
            draw.items
                .iter()
                .filter_map(|item| match &item.at {
                    DrawAt::Param(addr) => Skill::of(&start, addr),
                    DrawAt::Route(_) => None,
                })
                .collect()
        }
        Target::Seed(spec) => match enth_challenge::random::target(spec) {
            Ok(generated) => generated
                .params
                .iter()
                .filter_map(|addr| Skill::of(&generated.patch, addr))
                .collect(),
            Err(_) => BTreeSet::new(),
        },
        _ => BTreeSet::new(),
    }
}

/// A Recreate step of a passed lesson that Review can draw a sibling of.
#[derive(Clone, Debug)]
struct Candidate<'a> {
    lesson: LessonKey,
    step: StepId,
    challenge: &'a Challenge,
    skills: BTreeSet<Skill>,
}

/// The reviewable steps: every Recreate step of a completed lesson with a draw or seed target that exercises a skill,
/// each step once, in the lessons' order.
fn candidates<'a>(progress: &Progress, lessons: &'a [LessonBundle]) -> Vec<Candidate<'a>> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for bundle in lessons {
        let key = bundle.key();
        if !progress
            .lesson(&key)
            .is_some_and(|p| p.completed(&bundle.lesson))
        {
            continue;
        }
        for step in &bundle.lesson.steps {
            let StepKind::Recreate { challenge, .. } = &step.kind else {
                continue;
            };
            if challenge.draw().is_none() && challenge.spec().is_none() {
                continue;
            }
            if !seen.insert((key.clone(), step.id.clone())) {
                continue;
            }
            let skills = challenge_skills(challenge);
            if !skills.is_empty() {
                out.push(Candidate {
                    lesson: key.clone(),
                    step: step.id.clone(),
                    challenge,
                    skills,
                });
            }
        }
    }
    out
}

/// A skill's standing on a day, as the queue orders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// The skill.
    pub skill: Skill,
    /// Its box.
    pub level: u8,
    /// The day it is due (0 when no round has rung it).
    pub due_day: Day,
    /// Due on the queue's day.
    pub due: bool,
}

impl Standing {
    /// The queue's order on `today`: due first, then the lower box, the earlier due day, then a keyed draw of the day
    /// and the skill (so equal skills take turns from day to day instead of by name), then the name.
    fn order(&self, today: Day) -> (bool, u8, Day, u64, &str) {
        let turn = derive(u64::from(today), fnv1a64(self.skill.as_str().as_bytes()));
        (
            !self.due,
            self.level,
            self.due_day,
            turn,
            self.skill.as_str(),
        )
    }
}

/// One Review item: a seeded sibling of a passed lesson's Recreate step, for the weak skill it drills.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewItem {
    /// The skill it was picked for.
    pub skill: Skill,
    /// The skill's box.
    pub level: u8,
    /// The skill is due (`false`: an item that tops the queue up to [`MIN_ITEMS`]).
    pub due: bool,
    /// The lesson the step belongs to.
    pub lesson: LessonKey,
    /// The Recreate step.
    pub step: StepId,
    /// The sibling's seed.
    pub seed: u64,
    /// The step's challenge with that seed ([`Challenge::with_seed`]): what the round plays.
    pub challenge: Challenge,
}

/// The Review queue on a day ([`queue`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    /// The items, weakest due skill first: at most [`MAX_ITEMS`], at least [`MIN_ITEMS`] when that many reviewable
    /// steps exist.
    pub items: Vec<ReviewItem>,
    /// Every skill the reviewable steps exercise, in the queue's order.
    pub skills: Vec<Standing>,
}

impl Queue {
    /// How many skills of the passed lessons are due (what the Review button shows).
    pub fn due(&self) -> usize {
        self.skills.iter().filter(|s| s.due).count()
    }

    /// No reviewable step: nothing to play.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A sibling's seed for a step and a skill on a day: a keyed draw (never the global RNG), within the challenge
/// seeds' range, and never the step's own seed.
pub fn sibling_seed(today: Day, lesson: &LessonKey, step: &StepId, skill: &Skill, own: u64) -> u64 {
    let place = format!("{lesson}#{step}");
    let seed = derive(
        derive(u64::from(today), fnv1a64(place.as_bytes())),
        fnv1a64(skill.as_str().as_bytes()),
    ) & MAX_SEED;
    if seed == own {
        (seed + 1) & MAX_SEED
    } else {
        seed
    }
}

/// The seed a challenge plays now (a draw's or a spec's), if it has one.
fn own_seed(challenge: &Challenge) -> Option<u64> {
    challenge
        .draw()
        .map(|d| d.seed)
        .or_else(|| challenge.spec().map(|s| s.seed))
}

/// The Review queue for `progress` on `today`, from `lessons` (every installed lesson): see the [module docs](self).
/// Deterministic: the same progress, lessons and day give the same queue.
pub fn queue(progress: &Progress, lessons: &[LessonBundle], today: Day) -> Queue {
    let candidates = candidates(progress, lessons);
    let mut skills: Vec<Standing> = candidates
        .iter()
        .flat_map(|c| c.skills.iter())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|skill| {
            let b = progress.review.get(skill);
            Standing {
                skill: skill.clone(),
                level: b.level,
                due_day: b.due_day(),
                due: b.is_due(today),
            }
        })
        .collect();
    skills.sort_by(|a, b| a.order(today).cmp(&b.order(today)));
    let mut items: Vec<ReviewItem> = Vec::new();
    let mut used: BTreeSet<(&LessonKey, &StepId)> = BTreeSet::new();
    for s in &skills {
        if items.len() >= MAX_ITEMS || (!s.due && items.len() >= MIN_ITEMS) {
            break;
        }
        let options: Vec<&Candidate<'_>> = candidates
            .iter()
            .filter(|c| c.skills.contains(&s.skill) && !used.contains(&(&c.lesson, &c.step)))
            .collect();
        if options.is_empty() {
            continue;
        }
        let n = options.len() as u64;
        let first = derive(u64::from(today), fnv1a64(s.skill.as_str().as_bytes())) % n;
        for k in 0..n {
            let c = options[((first + k) % n) as usize];
            let own = own_seed(c.challenge).unwrap_or(u64::MAX);
            let seed = sibling_seed(today, &c.lesson, &c.step, &s.skill, own);
            if let Ok(challenge) = c.challenge.with_seed(seed) {
                used.insert((&c.lesson, &c.step));
                items.push(ReviewItem {
                    skill: s.skill.clone(),
                    level: s.level,
                    due: s.due,
                    lesson: c.lesson.clone(),
                    step: c.step.clone(),
                    seed,
                    challenge,
                });
                break;
            }
        }
    }
    Queue { items, skills }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_read_and_print_two_parts() {
        let s = Skill::parse("svf.cutoff").unwrap();
        assert_eq!(
            (s.kind(), s.key(), s.to_string()),
            ("svf", "cutoff", "svf.cutoff".to_owned())
        );
        for bad in [
            "svf",
            ".cutoff",
            "svf.",
            "Svf.cutoff",
            "svf.cut.off",
            "svf.cut off",
        ] {
            assert_eq!(Skill::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_interval_doubles_by_box() {
        assert_eq!(
            (1..=BOXES).map(interval).collect::<Vec<_>>(),
            [1, 2, 4, 8, 16]
        );
        assert_eq!(interval(0), 1, "clamped");
        assert_eq!(interval(9), 16, "clamped");
    }
}
