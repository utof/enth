//! **Owner: lane LCK** (lesson check). Pre-declared by lane S0l (R2).
//!
//! What `cargo xtask lesson-coverage` computes (docs/learn.md, "`lesson-check` and the authoring loop", and "GR2"):
//! which of the ten parameter tables (the seven module kinds, the built-ins `voice` and `master`, and the macro knob;
//! content/lessons/PLAN.md §7.1) and which of their parameters the lessons teach, and with which kinds of check. Pure:
//! the verified packs come in as values, nothing renders.
//!
//! **Taught** means a lesson lists the parameter in `teaches`. **Checked** comes from what each step changes, by
//! `enth_challenge::v2::touches` (the session's table) over the ops between two patches:
//!
//! | Step | What it changes | Kind |
//! | --- | --- | --- |
//! | `do` | its start to its witness (a step with no start of its own carries the patch over); the knobs its predicates read | `S` with a sound check, else `P` |
//! | `recreate` | the drawn parameters and routes (a draw target), or the start to the target (a patch or seed target) | `R` |
//! | `choose` | between its options | `E`, when `sound/1` can compare them; otherwise the step is listed in [`Coverage::unmeasured`] and counts nothing |
//!
//! A route counts for the tables of its source module (an `env2` or `lfo1`, a macro) and of its destination, and a
//! module switched on or off for its own table; neither counts for a parameter.
//!
//! **Holes** ([`Coverage::holes`], each a failure of `lesson-coverage`): a table no lesson teaches, a table with no
//! non-patch check (`S`, `R` or `E`; the ear question counts, Q6), and a Basic-tier parameter no lesson teaches.
//! **Listed apart** (for the owner at GR2, Q6 and Q9): tables and parameters whose only non-patch check is the ear
//! question ("E only"), and taught parameters with no non-patch check at all ("taught, not checked").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use enth_challenge::Challenge;
use enth_challenge::model::Target as ChallengeTarget;
use enth_challenge::v2::{Touch, touches};
use enth_params::{ParamSpec, Tier};
use enth_patch::catalogue::MACRO_VALUE;
use enth_patch::{Builtin, ModSource, ModuleKind, Patch, Target};

use crate::check::Knob;
use crate::lesson::{Example, StepKind};
use crate::pack::{LessonKey, Pack};

/// The macro knob's one parameter, as a table.
static MACRO_PARAMS: [ParamSpec; 1] = [MACRO_VALUE];

/// One of the ten parameter tables: a module kind, a built-in, or the macro knob.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// A module kind (`osc`, `svf`, …).
    Module(ModuleKind),
    /// The built-in `voice`.
    Voice,
    /// The built-in `master`.
    Master,
    /// The macro knob (`macro.value`).
    Macro,
}

impl Group {
    /// Every table, in catalogue order: the listed module kinds, then `voice`, `master` and the macro knob. A
    /// module kind added to the catalogue joins at once, so it is a hole until a lesson teaches it (AGENTS.md: a
    /// user-facing module comes with a lesson). A reserved S2a kind (`ModuleSpec::hidden`) joins when INT-S2a clears
    /// its flag, so coverage never meets a kind without a lesson (docs/phase-s2.md, rule 6).
    pub fn all() -> Vec<Group> {
        ModuleKind::listed()
            .map(Group::Module)
            .chain([Group::Voice, Group::Master, Group::Macro])
            .collect()
    }

    /// The key a lesson's `teaches` uses before the dot (`svf`, `voice`, `macro`).
    pub fn key(self) -> &'static str {
        match self {
            Group::Module(k) => k.key(),
            Group::Voice => Builtin::Voice.key(),
            Group::Master => Builtin::Master.key(),
            Group::Macro => "macro",
        }
    }

    /// Its parameter table.
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            Group::Module(k) => k.params(),
            Group::Voice => Builtin::Voice.params(),
            Group::Master => Builtin::Master.params(),
            Group::Macro => &MACRO_PARAMS,
        }
    }

    /// The table of `target` in `patch` (`None` for a module the patch lacks).
    fn of_target(target: &Target, patch: &Patch) -> Option<Group> {
        match target {
            Target::Module(id) => patch.find(id).map(|(_, m)| Group::Module(m.kind)),
            Target::Voice => Some(Group::Voice),
            Target::Master => Some(Group::Master),
        }
    }

    /// The table of a modulation source in `patch`: a modulator module's kind, or the macro knob.
    fn of_source(src: &ModSource, patch: &Patch) -> Option<Group> {
        match src {
            ModSource::Module(id) => patch.find(id).map(|(_, m)| Group::Module(m.kind)),
            ModSource::Macro(_) => Some(Group::Macro),
            ModSource::Velocity | ModSource::Key | ModSource::ModWheel => None,
        }
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// The kind of a check a step makes (docs/learn.md, "Checks: four kinds").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A patch predicate (`do` with a patch check only).
    Patch,
    /// A sound expectation (`do` with a sound check).
    Sound,
    /// A Recreate round.
    Recreate,
    /// An ear question that `sound/1` can measure.
    Ear,
}

impl Kind {
    /// `P`, `S`, `R`, `E`.
    pub fn letter(self) -> char {
        match self {
            Kind::Patch => 'P',
            Kind::Sound => 'S',
            Kind::Recreate => 'R',
            Kind::Ear => 'E',
        }
    }

    /// Whether it is a non-patch check (GR2 asks one per module).
    pub fn is_non_patch(self) -> bool {
        self != Kind::Patch
    }
}

/// The kinds in a set, as letters in `P S R E` order (`"S R E"`, `"-"` for none).
pub fn letters(kinds: &BTreeSet<Kind>) -> String {
    if kinds.is_empty() {
        return "-".to_owned();
    }
    kinds
        .iter()
        .map(|k| k.letter().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One parameter: where it is taught and how it is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamCoverage {
    /// `svf.cutoff`.
    pub key: String,
    /// Its catalogue tier.
    pub tier: Tier,
    /// The lessons whose `teaches` lists it.
    pub taught_in: BTreeSet<LessonKey>,
    /// The kinds of the steps that change it.
    pub kinds: BTreeSet<Kind>,
}

impl ParamCoverage {
    /// Its non-patch kinds.
    pub fn non_patch(&self) -> BTreeSet<Kind> {
        self.kinds
            .iter()
            .copied()
            .filter(|k| k.is_non_patch())
            .collect()
    }
}

/// One table: its lessons, the kinds of check that reach it (through a parameter, a route or the module itself), and
/// its parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupCoverage {
    /// The table.
    pub group: Group,
    /// The lessons that teach any of its parameters.
    pub lessons: BTreeSet<LessonKey>,
    /// Every kind of step that changes something of it.
    pub kinds: BTreeSet<Kind>,
    /// Its parameters, in catalogue order.
    pub params: Vec<ParamCoverage>,
}

impl GroupCoverage {
    /// Its non-patch kinds.
    pub fn non_patch(&self) -> BTreeSet<Kind> {
        self.kinds
            .iter()
            .copied()
            .filter(|k| k.is_non_patch())
            .collect()
    }
}

/// A step, named for a report: `pack/lesson#step`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StepRef {
    /// The lesson.
    pub lesson: LessonKey,
    /// The step's id.
    pub step: String,
}

impl fmt::Display for StepRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.lesson, self.step)
    }
}

/// A hole: a reason `lesson-coverage` fails.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hole {
    /// No lesson teaches any parameter of the table.
    NoLesson(Group),
    /// No step makes a non-patch check on the table.
    NoCheck(Group),
    /// No lesson teaches this Basic-tier parameter.
    Untaught(String),
}

impl fmt::Display for Hole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hole::NoLesson(g) => write!(f, "{g}: no lesson teaches any of its parameters"),
            Hole::NoCheck(g) => write!(
                f,
                "{g}: no non-patch check (a sound check, a Recreate round or an ear question) reaches it"
            ),
            Hole::Untaught(p) => write!(f, "{p}: a Basic-tier parameter no lesson teaches"),
        }
    }
}

/// The coverage of a set of packs. Build it with [`Coverage::of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    /// Every table, in [`Group::all`] order.
    pub groups: Vec<GroupCoverage>,
    /// Ear questions `sound/1` cannot compare (options that differ in phrase, or play transposed, with their own
    /// seed or a control curve): they count as no check, and need a listening review.
    pub unmeasured: Vec<StepRef>,
    /// The lessons read.
    pub lessons: usize,
}

impl Coverage {
    /// The coverage of `packs`.
    pub fn of(packs: &[&Pack]) -> Coverage {
        let mut acc = Acc::new();
        let mut lessons = 0;
        for pack in packs {
            for lesson in pack.lessons() {
                lessons += 1;
                let key = LessonKey {
                    pack: pack.id().clone(),
                    lesson: lesson.id.clone(),
                };
                for t in &lesson.teaches {
                    acc.taught(t.as_str(), &key);
                }
                acc.steps(pack, lesson, &key);
            }
        }
        Coverage {
            groups: acc.groups,
            unmeasured: acc.unmeasured,
            lessons,
        }
    }

    /// Every hole, tables first.
    pub fn holes(&self) -> Vec<Hole> {
        let mut out = Vec::new();
        for g in &self.groups {
            if g.lessons.is_empty() {
                out.push(Hole::NoLesson(g.group));
            }
            if g.non_patch().is_empty() {
                out.push(Hole::NoCheck(g.group));
            }
        }
        for g in &self.groups {
            for p in &g.params {
                if p.tier == Tier::Basic && p.taught_in.is_empty() {
                    out.push(Hole::Untaught(p.key.clone()));
                }
            }
        }
        out
    }

    /// The tables whose only non-patch check is the ear question (Q6: listed apart).
    pub fn e_only_groups(&self) -> Vec<Group> {
        self.groups
            .iter()
            .filter(|g| g.non_patch() == BTreeSet::from([Kind::Ear]))
            .map(|g| g.group)
            .collect()
    }

    /// The parameters whose only non-patch check is the ear question.
    pub fn e_only_params(&self) -> Vec<&ParamCoverage> {
        self.params()
            .filter(|p| p.non_patch() == BTreeSet::from([Kind::Ear]))
            .collect()
    }

    /// The taught parameters no non-patch check reaches (Q9: "taught, not checked", for the owner to accept at GR2).
    pub fn taught_not_checked(&self) -> Vec<&ParamCoverage> {
        self.params()
            .filter(|p| !p.taught_in.is_empty() && p.non_patch().is_empty())
            .collect()
    }

    /// The parameters no lesson teaches (a hole when Basic-tier).
    pub fn untaught(&self) -> Vec<&ParamCoverage> {
        self.params().filter(|p| p.taught_in.is_empty()).collect()
    }

    /// Every parameter, table by table.
    pub fn params(&self) -> impl Iterator<Item = &ParamCoverage> {
        self.groups.iter().flat_map(|g| g.params.iter())
    }

    /// One parameter by key (`svf.cutoff`).
    pub fn param(&self, key: &str) -> Option<&ParamCoverage> {
        self.params().find(|p| p.key == key)
    }

    /// One table.
    pub fn group(&self, group: Group) -> Option<&GroupCoverage> {
        self.groups.iter().find(|g| g.group == group)
    }
}

/// The running count.
struct Acc {
    groups: Vec<GroupCoverage>,
    index: BTreeMap<Group, usize>,
    unmeasured: Vec<StepRef>,
}

impl Acc {
    fn new() -> Acc {
        let groups: Vec<GroupCoverage> = Group::all()
            .into_iter()
            .map(|group| GroupCoverage {
                group,
                lessons: BTreeSet::new(),
                kinds: BTreeSet::new(),
                params: group
                    .params()
                    .iter()
                    .map(|spec| ParamCoverage {
                        key: format!("{}.{}", group.key(), spec.key),
                        tier: spec.learn.tier,
                        taught_in: BTreeSet::new(),
                        kinds: BTreeSet::new(),
                    })
                    .collect(),
            })
            .collect();
        let index = groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.group, i))
            .collect();
        Acc {
            groups,
            index,
            unmeasured: Vec::new(),
        }
    }

    fn group_mut(&mut self, group: Group) -> Option<&mut GroupCoverage> {
        let i = *self.index.get(&group)?;
        self.groups.get_mut(i)
    }

    /// A lesson's `teaches` entry (`svf.cutoff`).
    fn taught(&mut self, key: &str, lesson: &LessonKey) {
        let Some((group, _)) = key.split_once('.') else {
            return;
        };
        let Some(group) = Group::all().into_iter().find(|g| g.key() == group) else {
            return;
        };
        let Some(g) = self.group_mut(group) else {
            return;
        };
        if let Some(p) = g.params.iter_mut().find(|p| p.key == key) {
            p.taught_in.insert(lesson.clone());
            g.lessons.insert(lesson.clone());
        }
    }

    /// A step changed parameter `param` of `group` with a check of `kind`.
    fn param(&mut self, group: Group, param: &str, kind: Kind) {
        let key = format!("{}.{param}", group.key());
        if let Some(g) = self.group_mut(group) {
            g.kinds.insert(kind);
            if let Some(p) = g.params.iter_mut().find(|p| p.key == key) {
                p.kinds.insert(kind);
            }
        }
    }

    /// A step reached `group` other than through a parameter (a route, the module's switch).
    fn group(&mut self, group: Group, kind: Kind) {
        if let Some(g) = self.group_mut(group) {
            g.kinds.insert(kind);
        }
    }

    /// A touch, resolved against `before` (the patch as the earlier ops leave it) or `after` (an added module).
    fn touch(&mut self, touch: &Touch, before: &Patch, after: &Patch, kind: Kind) {
        let group_of =
            |t: &Target| Group::of_target(t, before).or_else(|| Group::of_target(t, after));
        match touch {
            Touch::Param(addr) => {
                if let Some(g) = group_of(addr.target()) {
                    self.param(g, addr.param(), kind);
                }
            }
            Touch::Macro(_) => self.param(Group::Macro, MACRO_VALUE.key, kind),
            Touch::Route(route) => {
                let src = Group::of_source(&route.src, before)
                    .or_else(|| Group::of_source(&route.src, after));
                for g in [src, group_of(route.dst.target())].into_iter().flatten() {
                    self.group(g, kind);
                }
            }
            Touch::Module(target) => {
                if let Some(g) = group_of(target) {
                    self.group(g, kind);
                }
            }
        }
    }

    /// Everything that differs between `a` and `b`, by the session's table.
    fn between(&mut self, a: &Patch, b: &Patch, kind: Kind) {
        let Some(ops) = enth_patch::ops_between(a, b) else {
            return;
        };
        let mut scratch = a.clone();
        for op in &ops {
            let before = scratch.clone();
            if enth_patch::apply(&mut scratch, op).is_err() {
                break;
            }
            for t in touches(&before, op).unwrap_or_default() {
                self.touch(&t, &before, &scratch, kind);
            }
        }
    }

    /// A lesson's steps, with the patch each one starts from.
    fn steps(&mut self, pack: &Pack, lesson: &crate::Lesson, key: &LessonKey) {
        let mut carried: Option<Patch> = None;
        for step in &lesson.steps {
            let own = step
                .view
                .start
                .as_ref()
                .and_then(|r| pack.patch(r))
                .map(|p| Patch::clone(p));
            let start = own.or_else(|| carried.clone());
            match &step.kind {
                // A drill is a game, not a lesson on a control: it covers nothing here (GR4 counts the games' coverage).
                StepKind::Read | StepKind::Listen { .. } | StepKind::Drill(_) => carried = start,
                StepKind::Do { check, witness } => {
                    let kind = if check.sounds().is_empty() {
                        Kind::Patch
                    } else {
                        Kind::Sound
                    };
                    let witness = pack.patch(witness).map(|p| Patch::clone(p));
                    if let (Some(s), Some(w)) = (&start, &witness) {
                        self.between(s, w, kind);
                    }
                    let reference = start.as_ref().or(witness.as_ref());
                    for knob in check.knobs() {
                        match knob {
                            Knob::Param(addr) => {
                                if let Some(g) =
                                    reference.and_then(|p| Group::of_target(addr.target(), p))
                                {
                                    self.param(g, addr.param(), Kind::Patch);
                                }
                            }
                            Knob::Macro(_) => {
                                self.param(Group::Macro, MACRO_VALUE.key, Kind::Patch);
                            }
                        }
                    }
                    carried = witness.or(start);
                }
                StepKind::Recreate { challenge, .. } => {
                    self.round(challenge);
                    carried = None;
                }
                StepKind::Choose { options, .. } => {
                    if measurable(options) {
                        let patches: Vec<Patch> = options
                            .iter()
                            .filter_map(|o| pack.patch(&o.patch).map(|p| Patch::clone(p)))
                            .collect();
                        if let Some((first, rest)) = patches.split_first() {
                            for other in rest {
                                self.between(first, other, Kind::Ear);
                            }
                        }
                    } else {
                        self.unmeasured.push(StepRef {
                            lesson: key.clone(),
                            step: step.id.to_string(),
                        });
                    }
                    carried = start;
                }
            }
        }
    }

    /// A Recreate round: what its target varies.
    fn round(&mut self, challenge: &Challenge) {
        let Ok(start) = challenge.start_patch() else {
            return;
        };
        match &challenge.target {
            ChallengeTarget::Draw(draw) => {
                for item in &draw.items {
                    self.touch(&item.at.touch(), &start, &start, Kind::Recreate);
                }
            }
            ChallengeTarget::Patch(target) => self.between(&start, target.patch(), Kind::Recreate),
            ChallengeTarget::Seed(spec) => {
                if let Ok(generated) = enth_challenge::random::generate(spec, &start) {
                    self.between(&start, &generated.patch, Kind::Recreate);
                }
            }
            // A target kind this build does not know counts nothing (`Target` is `non_exhaustive`).
            _ => {}
        }
    }
}

/// Whether `sound/1` can compare an ear question's options as they are played: one phrase, untransposed, at the
/// scoring seed, with no control curve (the rule of [`crate::check::choice_audibility`]).
pub fn measurable(options: &[Example]) -> bool {
    let Some(first) = options.first() else {
        return false;
    };
    options.iter().all(|o| {
        o.phrase == first.phrase && o.transpose == 0 && o.seed.is_none() && o.curve.is_none()
    })
}
