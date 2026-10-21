//! **Owner: lane DEC.** The pure planner: [`plan`] turns a preset and the caller's measured [`Rank`] of each element
//! into ordered steps (docs/learn.md, "Deconstruct lessons (lane DEC)"; report 03 §9: "`deconstruct::plan(preset,
//! ranks)`, a pure planner fed measured sensitivity").
//!
//! 1. **Prune.** An element whose removal is below one just-noticeable step is *fine detail*: `sound/1` moves less
//!    than [`AUDIBLE_JND`] and no descriptor of the coach's breakdown (brightness, the envelope's times and level,
//!    pitch, noise, beating, tremolo, a sweep, harmonics) moves [`MIN_CHANGE_STEPS`] tolerances beyond its nuisance
//!    noise ([`Rank::audible`]). The descriptors matter because `sound/1` is nearly deaf, at the held note, to slow
//!    beating, the sustain level, attacks under about 60 ms and resonance on plucks (lane C1's finding), and measured
//!    0.00 JND for Glass Pluck's second oscillator and 0.79 for Acid Snap's slide: it would under-rank exactly those.
//!    An element no phrase reveals is fine detail too.
//! 2. **Cap.** At most [`MAX_TAUGHT`] elements are taught (report 01 §6, "at most about 8 steps"); past that, the ones
//!    that contribute least join the fine detail.
//! 3. **Order** by signal flow ([`Flow`]), then by contribution (the larger `sound/1` distance first), then patch order.
//! 4. **Steps.** Fine detail is folded into the lesson's start, which is the base plus every fine element: inaudible
//!    by definition, and it makes the last step's target the preset itself. Step *k* starts from the start plus
//!    elements 1 to *k* − 1 and its target adds element *k*, measured in that context because elements interact
//!    (resonance is silent until the cutoff moves).
//!
//! [`Plan::merge`] is the one change after planning: a step whose element the caller measured as inaudible *in its
//! context* (a detune that only beats against the next oscillator) rides along with the next step.

use std::collections::{BTreeMap, BTreeSet};

use enth_challenge::{Component, Direction, PhraseId};
use enth_patch::Patch;

use super::element::{Element, Flow, elements, project};

/// Below this `sound/1` distance, in just-noticeable steps, an element's removal is not heard by `sound/1`.
pub const AUDIBLE_JND: f64 = 1.0;

/// A change counts toward audibility when it moves at least this many tolerances: two, so one of `sound/1`'s "a bit"
/// rows alone (1.5) is not enough, while a "far" row (3) or a descriptor that moved two tolerances beyond its noise is.
pub const MIN_CHANGE_STEPS: f64 = 2.0;

/// The most elements a lesson teaches, one step each (report 01 §6: "at most about 8 steps").
pub const MAX_TAUGHT: usize = 8;

/// A descriptor a change is read on: the coach's aspects, with movement split by kind where the description tells
/// them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    /// The pitch.
    Pitch,
    /// The brightness (spectral centroid).
    Brightness,
    /// The harmonic content (`sound/1`'s odd-to-all ratio).
    Harmonics,
    /// The attack time.
    Attack,
    /// The decay time.
    Decay,
    /// The held level.
    Sustain,
    /// The release time.
    Release,
    /// The noise share.
    Noisiness,
    /// Detune beating.
    Beating,
    /// An amplitude LFO.
    Tremolo,
    /// A brightness sweep (a filter envelope or LFO).
    Sweep,
    /// Movement of some kind (`sound/1`'s row, which does not say which).
    Movement,
}

/// How the sound *without* something reads against the sound *with* it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Way {
    /// Higher without it (darker with it, for brightness).
    Higher,
    /// Lower without it.
    Lower,
    /// Gone without it: it brings the aspect (a beat, a sweep, a clear pitch).
    Gone,
    /// Only without it: it takes the aspect away.
    New,
}

/// One heard change: an aspect, which way, and how far in the aspect's tolerances (the coach's breakdown).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Change {
    /// What changes.
    pub aspect: Aspect,
    /// Which way, without against with.
    pub way: Way,
    /// How far, in tolerances (`sound/1`'s rows: 1.5 for "a bit", 3 for "far"; a presence change counts 4).
    pub steps: f64,
    /// The reading with it, in the descriptor's unit, when there is one.
    pub with: Option<f64>,
    /// The reading without it.
    pub without: Option<f64>,
}

impl Change {
    /// `brightness higher (3.2)`, for reports.
    pub fn words(&self) -> String {
        let a = serde_json::to_value(self.aspect)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let w = match self.way {
            Way::Higher => "higher",
            Way::Lower => "lower",
            Way::Gone => "gone",
            Way::New => "new",
        };
        format!("{a} {w} ({:.1})", self.steps)
    }
}

/// `sound/1`'s descriptor row of "without" (the answer) against "with" (the target) as a change, when it moved.
pub fn change_of(c: &Component) -> Option<Change> {
    use enth_challenge::{Closeness, ComponentKind as K};
    let aspect = match c.kind {
        K::Pitch => Aspect::Pitch,
        K::Brightness => Aspect::Brightness,
        K::Harmonics => Aspect::Harmonics,
        K::Attack => Aspect::Attack,
        K::Envelope => Aspect::Decay,
        K::Sustain => Aspect::Sustain,
        K::Release => Aspect::Release,
        K::Noisiness => Aspect::Noisiness,
        K::Movement => Aspect::Movement,
        K::Unknown => return None,
    };
    let way = match c.direction {
        Direction::Higher => Way::Higher,
        Direction::Lower => Way::Lower,
        Direction::Missing => Way::Gone,
        Direction::Extra => Way::New,
        Direction::Same | Direction::Unknown => return None,
    };
    let steps = match (c.direction, c.closeness) {
        (Direction::Missing | Direction::Extra, _) => 4.0,
        (_, Closeness::Far) => 3.0,
        _ => 1.5,
    };
    Some(Change {
        aspect,
        way,
        steps,
        with: c.target,
        without: c.answer,
    })
}

/// `a` and `b` together, one change per aspect (the furthest), the furthest first.
pub fn merge_changes(a: Vec<Change>, b: Vec<Change>) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for c in a.into_iter().chain(b) {
        match out.iter_mut().find(|x| x.aspect == c.aspect) {
            Some(x) if x.steps < c.steps => *x = c,
            Some(_) => {}
            None => out.push(c),
        }
    }
    out.sort_by(|x, y| y.steps.total_cmp(&x.steps).then(x.aspect.cmp(&y.aspect)));
    out
}

/// What removing one element from the full preset does, measured by the caller (`lesson-gen`, or the plugin's worker
/// through [`super::measure`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Rank {
    /// The phrase it was measured through: the first that reveals it (RG's observability mask); `None` when no
    /// phrase does (a phase, a seed, a module switched on at level 0).
    pub phrase: Option<PhraseId>,
    /// `sound/1`'s distance between the preset and the preset without the element, in just-noticeable steps.
    pub jnd: f64,
    /// The same as a sound %, for reports.
    pub percent: f64,
    /// The heard changes, the furthest first: the descriptors that moved at least two tolerances and beyond their
    /// nuisance noise, and `sound/1`'s harmonic-content row when it moved far (no description reads harmonics).
    pub changes: Vec<Change>,
}

impl Rank {
    /// An element no phrase reveals.
    pub fn unobservable() -> Rank {
        Rank {
            phrase: None,
            jnd: 0.0,
            percent: 100.0,
            changes: Vec::new(),
        }
    }

    /// Whether removing the element is heard: `sound/1` at least [`AUDIBLE_JND`], or a change (see the [module
    /// docs](self)).
    pub fn audible(&self) -> bool {
        self.phrase.is_some() && (self.jnd >= AUDIBLE_JND || self.heard_changes() > 0)
    }

    /// How many changes reach [`MIN_CHANGE_STEPS`].
    pub fn heard_changes(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.steps >= MIN_CHANGE_STEPS)
            .count()
    }
}

/// Whether a descriptor row left its tolerance (or appeared, or disappeared).
pub fn moved(c: &Component) -> bool {
    !matches!(c.direction, Direction::Same | Direction::Unknown)
}

/// Each element's [`Rank`], by element id.
pub type Ranks = BTreeMap<String, Rank>;

/// Why an element gets no step of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FineWhy {
    /// Removing it is below one just-noticeable step.
    BelowJnd,
    /// No phrase reveals it.
    Unobservable,
    /// The caller gave no rank for it.
    NotMeasured,
    /// Audible, but past the [`MAX_TAUGHT`] most contributing elements.
    OverCap,
}

impl FineWhy {
    /// A stable snake-case name.
    pub fn name(self) -> &'static str {
        match self {
            FineWhy::BelowJnd => "below_jnd",
            FineWhy::Unobservable => "unobservable",
            FineWhy::NotMeasured => "not_measured",
            FineWhy::OverCap => "over_cap",
        }
    }
}

/// One step: the elements it adds (one, or more after a [`Plan::merge`]) and the phrase it is heard through.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanStep {
    /// Indices into [`Plan::elements`], in teaching order.
    pub elements: Vec<usize>,
    /// The phrase of its round and its examples.
    pub phrase: PhraseId,
}

/// A deconstruct lesson's skeleton: the elements, which are fine detail, and the steps in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The preset.
    pub preset: Patch,
    /// Every element of the preset, in patch order ([`elements`]).
    pub elements: Vec<Element>,
    /// The ranks the plan was made from.
    pub ranks: Ranks,
    /// The elements folded into the lesson's start, with why, in patch order.
    pub fine: Vec<(usize, FineWhy)>,
    /// The steps, in teaching order.
    pub steps: Vec<PlanStep>,
}

impl Plan {
    /// The elements included before step `k` (the fine detail and every earlier step's).
    pub fn included_before(&self, k: usize) -> BTreeSet<usize> {
        self.fine
            .iter()
            .map(|(i, _)| *i)
            .chain(
                self.steps
                    .iter()
                    .take(k)
                    .flat_map(|s| s.elements.iter().copied()),
            )
            .collect()
    }

    /// The lesson's start: the base with the fine detail (step 0's start).
    pub fn start_patch(&self) -> Patch {
        self.step_start(0)
    }

    /// Step `k`'s start: the lesson's start plus the elements of steps 0 to `k` − 1.
    pub fn step_start(&self, k: usize) -> Patch {
        project(&self.preset, &self.elements, &self.included_before(k))
    }

    /// Step `k`'s target: its start plus its own elements. The last step's target is the preset.
    pub fn step_target(&self, k: usize) -> Patch {
        project(&self.preset, &self.elements, &self.included_before(k + 1))
    }

    /// The preset without element `i` (everything else included): an ablation.
    pub fn without(&self, i: usize) -> Patch {
        let all: BTreeSet<usize> = (0..self.elements.len()).filter(|j| *j != i).collect();
        project(&self.preset, &self.elements, &all)
    }

    /// Fold step `k` into the next one (into the previous one when it is the last), so its elements are taught
    /// together with a neighbour's. The merged step keeps the phrase of its element with the larger contribution.
    /// Does nothing when the plan has one step or `k` is out of range.
    pub fn merge(&mut self, k: usize) {
        if self.steps.len() < 2 || k >= self.steps.len() {
            return;
        }
        let (into, from) = if k + 1 < self.steps.len() {
            (k + 1, k)
        } else {
            (k - 1, k)
        };
        let moved = self.steps.remove(from);
        let into = if into > from { into - 1 } else { into };
        let target = &mut self.steps[into];
        let jnd = |i: &usize| self.ranks.get(&self.elements[*i].id).map_or(0.0, |r| r.jnd);
        let best_new = moved.elements.iter().map(jnd).fold(0.0, f64::max);
        let best_old = target.elements.iter().map(jnd).fold(0.0, f64::max);
        if best_new > best_old {
            target.phrase = moved.phrase;
        }
        if from < into + 1 {
            let mut els = moved.elements;
            els.extend(target.elements.iter().copied());
            target.elements = els;
        } else {
            target.elements.extend(moved.elements);
        }
    }
}

/// Plan the deconstruct lesson of `preset` from the caller's `ranks` (see the [module docs](self)). Pure: no render.
pub fn plan(preset: &Patch, ranks: &Ranks) -> Plan {
    let els = elements(preset);
    let mut fine = Vec::new();
    let mut taught = Vec::new();
    for (i, e) in els.iter().enumerate() {
        match ranks.get(&e.id) {
            None => fine.push((i, FineWhy::NotMeasured)),
            Some(r) if r.phrase.is_none() => fine.push((i, FineWhy::Unobservable)),
            Some(r) if !r.audible() => fine.push((i, FineWhy::BelowJnd)),
            Some(_) => taught.push(i),
        }
    }
    let contribution = |i: usize| ranks.get(&els[i].id).map_or(0.0, contribution_of);
    if taught.len() > MAX_TAUGHT {
        let mut by = taught.clone();
        by.sort_by(|a, b| contribution(*b).total_cmp(&contribution(*a)).then(a.cmp(b)));
        let cut: BTreeSet<usize> = by[MAX_TAUGHT..].iter().copied().collect();
        taught.retain(|i| !cut.contains(i));
        fine.extend(cut.into_iter().map(|i| (i, FineWhy::OverCap)));
        fine.sort_by_key(|(i, _)| *i);
    }
    taught.sort_by(|a, b| {
        els[*a]
            .flow
            .cmp(&els[*b].flow)
            .then(contribution(*b).total_cmp(&contribution(*a)))
            .then(a.cmp(b))
    });
    let steps = taught
        .into_iter()
        .map(|i| PlanStep {
            elements: vec![i],
            phrase: ranks[&els[i].id]
                .phrase
                .unwrap_or(enth_challenge::phrases::HELD),
        })
        .collect();
    Plan {
        preset: preset.clone(),
        elements: els,
        ranks: ranks.clone(),
        fine,
        steps,
    }
}

/// How much an element contributes, for ordering: its `sound/1` distance, plus half a step for each heard change, so an element `sound/1` hears weakly but the coach reads still ranks above one neither hears.
pub fn contribution_of(rank: &Rank) -> f64 {
    rank.jnd + 0.5 * rank.heard_changes() as f64
}

/// Whether a step's elements all sit in one flow (for text: "the filter" rather than a list).
pub fn flows(plan: &Plan, step: &PlanStep) -> Vec<Flow> {
    let mut out: Vec<Flow> = step
        .elements
        .iter()
        .map(|i| plan.elements[*i].flow)
        .collect();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_challenge::phrases::{HELD, LEGATO_PAIR};
    use enth_challenge::{Closeness, ComponentKind};
    use enth_patch::{Op, ParamAddr, RouteAddr, apply};

    fn acid() -> Patch {
        let mut p = super::super::element::tests::starter();
        let set = |p: &mut Patch, a: &str, v: f32| {
            apply(
                p,
                Op::SetParam {
                    addr: a.parse::<ParamAddr>().unwrap(),
                    value: v,
                },
            )
            .unwrap();
        };
        set(&mut p, "filter1.cutoff", 320.0);
        set(&mut p, "filter1.resonance", 55.0);
        set(&mut p, "filter1.drive", 6.0);
        set(&mut p, "env2.sustain", 0.0);
        set(&mut p, "voice.glide", 70.0);
        set(&mut p, "voice.mode", 2.0);
        for (r, d) in [
            ("env2 -> filter1.cutoff", 50.0),
            ("macro1 -> filter1.cutoff", 15.0),
        ] {
            let route: RouteAddr = r.parse().unwrap();
            let index = p.routes.len();
            apply(
                &mut p,
                Op::InsertRoute {
                    index,
                    route: enth_patch::Route {
                        src: route.src,
                        dst: route.dst,
                        depth: d,
                    },
                },
            )
            .unwrap();
        }
        p
    }

    fn rank(phrase: PhraseId, jnd: f64, moved: &[ComponentKind]) -> Rank {
        Rank {
            phrase: Some(phrase),
            jnd,
            percent: 100.0 / (1.0 + jnd),
            changes: moved
                .iter()
                .filter_map(|k| {
                    change_of(&Component {
                        kind: *k,
                        direction: Direction::Higher,
                        closeness: Closeness::Far,
                        target: Some(1.0),
                        answer: Some(2.0),
                    })
                })
                .collect(),
        }
    }

    #[test]
    fn prunes_orders_by_flow_then_contribution_and_ends_on_the_preset() {
        let p = acid();
        let mut ranks = Ranks::new();
        ranks.insert(
            "filter1".into(),
            rank(HELD, 9.0, &[ComponentKind::Brightness]),
        );
        ranks.insert("filter1.drive".into(), rank(HELD, 0.4, &[]));
        ranks.insert("env2 -> filter1.cutoff".into(), rank(HELD, 5.0, &[]));
        ranks.insert("macro1 -> filter1.cutoff".into(), rank(HELD, 0.0, &[]));
        // sound/1 hears the glide only weakly, but a descriptor moves far: it is taught.
        ranks.insert(
            "voice".into(),
            rank(LEGATO_PAIR, 0.3, &[ComponentKind::Pitch]),
        );
        let plan = plan(&p, &ranks);
        let order: Vec<&str> = plan
            .steps
            .iter()
            .map(|s| plan.elements[s.elements[0]].id.as_str())
            .collect();
        assert_eq!(order, ["filter1", "env2 -> filter1.cutoff", "voice"]);
        assert_eq!(plan.steps[2].phrase, LEGATO_PAIR);
        let fine: Vec<(&str, FineWhy)> = plan
            .fine
            .iter()
            .map(|(i, w)| (plan.elements[*i].id.as_str(), *w))
            .collect();
        assert_eq!(
            fine,
            [
                ("filter1.drive", FineWhy::BelowJnd),
                ("macro1 -> filter1.cutoff", FineWhy::BelowJnd)
            ]
        );
        assert_eq!(
            plan.step_target(plan.steps.len() - 1),
            p,
            "the last target is the preset"
        );
        for k in 1..plan.steps.len() {
            assert_eq!(plan.step_start(k), plan.step_target(k - 1));
        }
        // The fine detail is in the lesson's start already.
        let drive: ParamAddr = "filter1.drive".parse().unwrap();
        assert_eq!(plan.start_patch().value(&drive), Some(6.0));
        assert_eq!(
            plan.start_patch().routes.len(),
            1,
            "the macro route, folded in"
        );
    }

    #[test]
    fn an_element_with_no_rank_or_no_phrase_is_fine_detail_and_the_cap_keeps_the_strongest() {
        let p = acid();
        let mut ranks = Ranks::new();
        ranks.insert("filter1".into(), Rank::unobservable());
        let plan1 = plan(&p, &ranks);
        assert_eq!(plan1.steps, Vec::new());
        assert!(plan1.fine.iter().any(|(_, w)| *w == FineWhy::Unobservable));
        assert!(plan1.fine.iter().any(|(_, w)| *w == FineWhy::NotMeasured));
        assert_eq!(
            plan1.start_patch(),
            p,
            "nothing taught: the start is the preset"
        );

        let els = elements(&p);
        let mut ranks = Ranks::new();
        for (n, e) in els.iter().enumerate() {
            ranks.insert(e.id.clone(), rank(HELD, 2.0 + n as f64, &[]));
        }
        assert!(
            els.len() <= MAX_TAUGHT,
            "the fixture is small; the cap is a property below"
        );
        let all = plan(&p, &ranks);
        assert_eq!(all.steps.len(), els.len());
    }

    #[test]
    fn merging_folds_a_step_into_its_neighbour_and_keeps_the_targets() {
        let p = acid();
        let mut ranks = Ranks::new();
        ranks.insert("filter1".into(), rank(HELD, 9.0, &[]));
        ranks.insert("env2 -> filter1.cutoff".into(), rank(HELD, 5.0, &[]));
        ranks.insert("voice".into(), rank(LEGATO_PAIR, 3.0, &[]));
        let mut pl = plan(&p, &ranks);
        let last = pl.step_target(2);
        pl.merge(0);
        assert_eq!(pl.steps.len(), 2);
        assert_eq!(pl.steps[0].elements.len(), 2);
        assert_eq!(pl.step_target(1), last);
        pl.merge(1);
        assert_eq!(pl.steps.len(), 1);
        assert_eq!(pl.steps[0].elements.len(), 3);
        assert_eq!(
            pl.steps[0].phrase, HELD,
            "the filter's phrase: it contributes most"
        );
        pl.merge(0);
        assert_eq!(pl.steps.len(), 1, "one step cannot merge");
    }
}
