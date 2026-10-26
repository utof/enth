//! **Owner: lane ST** (structure, combined and time). The structure score, `structure/1` (report 02 §2.5; design.md
//! §6 and §9): how close the player's *patch* is to the target's, over what the round randomized. The reference is
//! docs/challenge.md, "Structure, combined and time".
//!
//! **What it compares.** Only what the generator drew, all of it observable through the challenge's phrase (lane
//! RG's mask, P4): the randomized parameters, the randomized routes and the randomized enable flags. Gauges,
//! nuisances and dead parameters are never compared, because nobody can hear them (metric v1 measured `osc1.level`
//! 100 → 50 %, `master.level` −6 dB and a 300 ms glide on one note at a distance of 0.0000).
//!
//! **How.** The patches' modules pair first ([`super::matching`]: disabled ≡ absent, depth 0 ≡ no route, a silent
//! generator or an unrouted modulator ≡ absent, interchangeable modules pair by content), so each target value is read
//! from the player's partner module, and the ring lands on the player's own control (`answer_addr`). Then:
//!
//! | Item | Distance in steps | Row `addr` |
//! | --- | --- | --- |
//! | a randomized parameter | its just-noticeable steps ([`priors::jnd`](crate::priors::jnd), [`Jnd::steps`](crate::priors::Jnd::steps)), with the folds below; infinite without a partner | the address, `filter1.cutoff` |
//! | a randomized route | depth difference in route steps ([`ROUTE_JND`], 5 points), an absent route at depth 0; infinite when an end's module has no partner | the route, `lfo1 -> filter1.cutoff` |
//! | a randomized enable flag | 0 when the player's patch has the module exactly when the target does, else infinite | `osc2.enabled` |
//! | a module the matching left over | infinite (it scores 0), unless an enable-flag row already says so | `modules` |
//! | a module of another kind of the same family standing in for the target's (lane MTC, S2a: a `wt` for an `osc`, a `ladder` for an `svf`) | its link's credit, 0.25, 0.5 or 0.75 ([`super::matching::families`]); the parameters of the target module are read through the corresponding ones of the substitute and score 0 where it has none | `modules` (status `substituted`) |
//!
//! The folds (equivalences): an oscillator's `semi` and `fine` read as one pitch in cents (+12 st ≡ +11 st and
//! +100 ct); a generator's level reads in dB against the target's first generator's partner (levels count only as
//! ratios between sources); the reverb's `wet` reads against its `dry`.
//!
//! **Each item scores** [`item_score`]: 1 under one step, else `1 − steps/4` (0 from four steps); the ring is
//! [`verdict`]: good under one step, near under three, bad from three. **Structure %** = 100 × the mean item score
//! over every randomized parameter, route and enable flag, and every left-over module not already counted by an
//! enable-flag row (100 when there is nothing to compare). Every item weighs the same in v1; a weighting by measured
//! sensitivity would be `structure/2`. Rows for what the player moved that the target did not randomize
//! (`randomized: false`) and the module list are shown but, except the modules, never counted.
//!
//! **The dispatch hook** (`prepare`, F1r review S5): `score::round` asks this file for the structure scorer a
//! challenge names, after the stub (`structure-stub/1`).

use std::cmp::Ordering;
use std::collections::BTreeSet;

use enth_patch::{ModuleId, ParamAddr, Patch, RouteAddr};

use super::matching::{
    self, Draws, Matcher, ModuleMatching, Side, answer_ref, depth_of, level_ref_of, present,
};
use super::round::{PreparedScorer, Ready};
use super::{
    Direction, ModuleStatus, ModuleTerm, ParamScore, ScoreError, Scorer, ScorerId, StructureScore,
    TargetContext, Verdict,
};
use crate::model::Phrase;
use crate::priors::ROUTE_JND;
use crate::random::{Mask, Visibility, observable};

/// Steps at which an item scores 0.
pub const ZERO_AT_STEPS: f64 = 4.0;

/// One item's score from its distance in just-noticeable steps: 1 under one step (inaudible: the ring is good),
/// else `1 − steps/4`, reaching 0 at four steps. So an integer semitone off (exactly one step) scores 0.75, never 1,
/// and structure % is 100 exactly when every counted ring is good.
pub fn item_score(steps: f64) -> f64 {
    if steps < 1.0 {
        1.0
    } else {
        (1.0 - steps / ZERO_AT_STEPS).clamp(0.0, 1.0)
    }
}

/// The ring from the distance in steps: good under one step, near under three, bad from three.
pub fn verdict(steps: f64) -> Verdict {
    if steps < 1.0 {
        Verdict::Good
    } else if steps < 3.0 {
        Verdict::Near
    } else {
        Verdict::Bad
    }
}

/// The structure scorer `id`, prepared on `ctx`; `None` when this file has no scorer by that id (the caller refuses
/// it, `unknown_scorer`).
pub(crate) fn prepare(
    id: &ScorerId,
    ctx: &TargetContext<'_>,
) -> Option<Result<Box<dyn PreparedScorer<StructureScore>>, ScoreError>> {
    (*id == ScorerId::STRUCTURE_V1).then(|| Ready::prepare(StructureV1, ctx))
}

/// `structure/1`: see the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureV1;

impl Scorer for StructureV1 {
    type Prepared = Prepared;
    type Score = StructureScore;

    fn id(&self) -> ScorerId {
        ScorerId::STRUCTURE_V1
    }

    fn prepare(&self, target: &TargetContext<'_>) -> Result<Prepared, ScoreError> {
        Ok(Prepared::new(
            target.target,
            target.phrase,
            &Draws::of(target),
        ))
    }

    fn score(&self, prepared: &Prepared, answer: &Patch) -> Result<StructureScore, ScoreError> {
        Ok(prepared.score(answer))
    }
}

/// What `structure/1` keeps from the target: the matcher (target, phrase, mask, draws) and the items to compare.
#[derive(Clone, Debug)]
pub struct Prepared {
    matcher: Matcher,
    params: Vec<ParamAddr>,
    routes: Vec<RouteAddr>,
    enables: Vec<ModuleId>,
    /// The target's observable parameters that were not randomized, in address order: a player's move there is a
    /// row of its own, never counted.
    others: Vec<ParamAddr>,
}

impl Prepared {
    /// The prepared structure score of `target` heard through `phrase`, comparing `draws` (what a challenge's
    /// [`Draws::of`] gives; the CLI's `compare` of two patches may pass its own).
    pub fn new(target: &Patch, phrase: &Phrase, draws: &Draws) -> Prepared {
        let matcher = Matcher::new(target, phrase, draws);
        let randomized: BTreeSet<&ParamAddr> = draws.params.iter().collect();
        let others = matcher
            .mask()
            .observable_params()
            .filter(|a| !randomized.contains(a))
            .cloned()
            .collect();
        Prepared {
            params: draws.params.clone(),
            routes: draws.routes.clone(),
            enables: draws.enables.clone(),
            others,
            matcher,
        }
    }

    /// The matcher both scores pair modules with.
    pub fn matcher(&self) -> &Matcher {
        &self.matcher
    }

    /// Score the player's patch.
    pub fn score(&self, answer: &Patch) -> StructureScore {
        let target = self.matcher.target();
        let t_mask = self.matcher.mask();
        let a_mask = observable(answer, self.matcher.phrase());
        let matching = self.matcher.pair_with(answer, &a_mask);
        let t_side = Side {
            patch: target,
            level_ref: level_ref_of(target),
        };
        let a_side = Side {
            patch: answer,
            level_ref: answer_ref(target, &matching, answer),
        };
        let reader = Reader {
            target,
            t_mask,
            answer,
            t_side,
            a_side,
            matching: &matching,
        };
        let mut rows = Vec::new();
        for addr in &self.params {
            rows.push(reader.param_row(addr, true));
        }
        for route in &self.routes {
            rows.push(reader.route_row(route, true));
        }
        let mut covered: BTreeSet<(bool, ModuleId)> = BTreeSet::new();
        let mut claimed: BTreeSet<ModuleId> = BTreeSet::new();
        for id in &self.enables {
            if let Some(row) = reader.enable_row(id, &mut covered, &mut claimed) {
                rows.push(row);
            }
        }
        let mut counted: Vec<f64> = rows.iter().map(|r| r.score).collect();
        // What the player moved that the target did not randomize: parameters, then routes.
        for addr in &self.others {
            let row = reader.param_row(addr, false);
            if row.answer_addr.is_some() && row.verdict != Verdict::Good {
                rows.push(row);
            }
        }
        rows.extend(reader.moved_routes(&self.routes, &a_mask));
        let mut modules = Vec::new();
        for (ids, status, patch) in [
            (matching.missing(), ModuleStatus::Missing, target),
            (matching.extra(), ModuleStatus::Extra, answer),
        ] {
            for id in ids {
                if !covered.contains(&(status == ModuleStatus::Extra, id.clone())) {
                    counted.push(0.0);
                }
                modules.push(ModuleTerm {
                    id: id.to_string(),
                    kind: patch
                        .find(id)
                        .map_or_else(String::new, |(_, m)| m.kind.key().to_owned()),
                    status,
                });
            }
        }
        // A family substitute is a module that exists but is not the target's kind: one more item, its link's credit.
        for sub in matching.substitutions() {
            counted.push(sub.credit());
            modules.push(ModuleTerm {
                id: sub.target.to_string(),
                kind: sub.target_kind.key().to_owned(),
                status: ModuleStatus::Substituted,
            });
        }
        let percent = if counted.is_empty() {
            100.0
        } else {
            (100.0 * counted.iter().sum::<f64>() / counted.len() as f64).clamp(0.0, 100.0)
        };
        StructureScore {
            scorer: ScorerId::STRUCTURE_V1,
            percent,
            params: rows,
            modules,
        }
    }
}

/// The two patches and their pairing, for reading rows.
struct Reader<'a> {
    target: &'a Patch,
    t_mask: &'a Mask,
    answer: &'a Patch,
    t_side: Side<'a>,
    a_side: Side<'a>,
    matching: &'a ModuleMatching,
}

impl Reader<'_> {
    /// A parameter's row: the target's value against the player's partner's.
    fn param_row(&self, addr: &ParamAddr, randomized: bool) -> ParamScore {
        let answer_addr = self.matching.answer_addr(addr).or_else(|| {
            // A target module that is itself absent (never for a generated target) reads against the same id.
            let id = addr.module_id()?;
            let (_, tm) = self.target.find(id)?;
            (!present(self.target, self.t_mask, tm)
                && self.answer.find(id).is_some_and(|(_, m)| m.kind == tm.kind))
            .then(|| addr.clone())
        });
        let reading = answer_addr
            .as_ref()
            .and_then(|a| matching::steps(&self.t_side, addr, &self.a_side, a));
        let (steps, direction) = match reading {
            Some((steps, order)) => (steps, direction_of(steps, order)),
            None => (f64::INFINITY, Direction::Missing),
        };
        ParamScore {
            addr: addr.to_string(),
            answer_addr: reading.and(answer_addr).map(|a| a.to_string()),
            score: item_score(steps),
            verdict: verdict(steps),
            direction,
            randomized,
        }
    }

    /// A route's row: the target's depth against the player's route between the partners (absent: depth 0).
    fn route_row(&self, route: &RouteAddr, randomized: bool) -> ParamScore {
        let t_depth = depth_of(self.target, &route.src, &route.dst).unwrap_or(0.0);
        let mapped = self.matching.answer_route(route);
        let a_depth = mapped
            .as_ref()
            .and_then(|r| depth_of(self.answer, &r.src, &r.dst));
        let (steps, direction) = if mapped.is_none() {
            (f64::INFINITY, Direction::Missing)
        } else {
            let a = a_depth.unwrap_or(0.0);
            let steps = ROUTE_JND.steps(t_depth, a);
            let direction = if steps < 1.0 {
                Direction::Same
            } else if a_depth.is_none() {
                Direction::Missing
            } else if t_depth == 0.0 {
                Direction::Extra
            } else {
                direction_of(steps, a.partial_cmp(&t_depth).unwrap_or(Ordering::Equal))
            };
            (steps, direction)
        };
        ParamScore {
            addr: route.to_string(),
            answer_addr: a_depth.and(mapped).map(|r| r.to_string()),
            score: item_score(steps),
            verdict: verdict(steps),
            direction,
            randomized,
        }
    }

    /// An enable flag's row: good when the player's patch has the module exactly when the target does. A target
    /// module that is on and unpaired is missing; one that is off is matched against an extra module of its kind.
    fn enable_row(
        &self,
        id: &ModuleId,
        covered: &mut BTreeSet<(bool, ModuleId)>,
        claimed: &mut BTreeSet<ModuleId>,
    ) -> Option<ParamScore> {
        let (_, tm) = self.target.find(id)?;
        let same_id = || {
            self.answer
                .find(id)
                .filter(|(_, m)| m.kind == tm.kind)
                .map(|_| id.clone())
        };
        let (good, direction, answer_id) = if present(self.target, self.t_mask, tm) {
            if let Some(a) = self.matching.answer_of(id) {
                (true, Direction::Same, Some(a.clone()))
            } else {
                covered.insert((false, id.clone()));
                (false, Direction::Missing, same_id())
            }
        } else {
            let extra = self.matching.extra().iter().find(|e| {
                !claimed.contains(*e) && self.answer.find(e).is_some_and(|(_, m)| m.kind == tm.kind)
            });
            match extra {
                Some(e) => {
                    claimed.insert(e.clone());
                    covered.insert((true, e.clone()));
                    (false, Direction::Extra, Some(e.clone()))
                }
                None => (true, Direction::Same, same_id()),
            }
        };
        let steps = if good { 0.0 } else { f64::INFINITY };
        Some(ParamScore {
            addr: format!("{id}.enabled"),
            answer_addr: answer_id.map(|a| format!("{a}.enabled")),
            score: item_score(steps),
            verdict: verdict(steps),
            direction,
            randomized: true,
        })
    }

    /// Routes the player moved that the target did not randomize, by the target's address: the target's other heard
    /// routes, and the player's heard routes between paired modules. Shown when a step or more apart; never counted.
    fn moved_routes(&self, randomized: &[RouteAddr], a_mask: &Mask) -> Vec<ParamScore> {
        let skip: BTreeSet<&RouteAddr> = randomized.iter().collect();
        let mut seen: BTreeSet<RouteAddr> = BTreeSet::new();
        for r in &self.target.routes {
            let addr = RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            };
            if r.depth != 0.0
                && !skip.contains(&addr)
                && self
                    .t_mask
                    .route(&addr)
                    .is_some_and(Visibility::is_observable)
                && self.matching.answer_route(&addr).is_some()
            {
                seen.insert(addr);
            }
        }
        for r in &self.answer.routes {
            let addr = RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            };
            if r.depth == 0.0 || !a_mask.route(&addr).is_some_and(Visibility::is_observable) {
                continue;
            }
            if let Some(t) = self.matching.target_route(&addr)
                && !skip.contains(&t)
            {
                seen.insert(t);
            }
        }
        seen.into_iter()
            .map(|addr| self.route_row(&addr, false))
            .filter(|row| row.verdict != Verdict::Good)
            .collect()
    }
}

fn direction_of(steps: f64, order: Ordering) -> Direction {
    if steps < 1.0 {
        Direction::Same
    } else {
        match order {
            Ordering::Greater => Direction::Higher,
            Ordering::Less => Direction::Lower,
            // A discrete value that moved without an order (never: indices compare).
            Ordering::Equal => Direction::Same,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_score_one_under_a_step_and_zero_from_four() {
        assert_eq!(item_score(0.0), 1.0);
        assert_eq!(item_score(0.999), 1.0);
        assert_eq!(item_score(1.0), 0.75);
        assert_eq!(item_score(2.0), 0.5);
        assert_eq!(item_score(3.0), 0.25);
        assert_eq!(item_score(4.0), 0.0);
        assert_eq!(item_score(f64::INFINITY), 0.0);
        assert_eq!(
            [verdict(0.5), verdict(1.0), verdict(2.99), verdict(3.0)],
            [Verdict::Good, Verdict::Near, Verdict::Near, Verdict::Bad]
        );
    }
}
