//! **Owner: lane RUI** (the Recreate UI).
//!
//! The score overlay of a round (report 01 §2.6, summary "Feedback"): **no live score by default**; the first
//! Submit puts a Good / Near / Bad ring (✓ ~ ✗, never colour alone) on every control the structure score has a row
//! for, on the **player's** control (`answer_addr`), and the rings then follow the player's edits live (the
//! session's `Rings` jobs). Parameters ring on their knob, switch or segmented control; ST's route rows
//! (`lfo1 -> filter1.cutoff`) on the route's matrix cell or route-list row; enable rows (`osc2.enabled`) on the
//! module's enable toggle ([`RoundView::rings`](enth_session::game::RoundView), read with
//! [`RingAt::parse`](enth_session::game::RingAt::parse)).
//!
//! The `dim_unused` assist (Challenge v1's, offered on the settings page) dims every module none of whose
//! parameters the target randomized ([`RoundView::params`](enth_session::game::RoundView)), and the built-ins'
//! parameters it did not randomize. Recreate never hides anything: "difficulty never locks features".

use std::collections::{BTreeMap, BTreeSet};

use enth_challenge::Verdict;
use enth_patch::{ModuleId, ParamAddr, RouteAddr, Target};
use enth_session::game::{RingAt, RoundView};

use crate::editor::overlay::{Emphasis, Overlay, ParamOverlay};
use crate::theme::Score;

/// The ring a verdict draws; `None` for a verdict from a newer build.
pub fn score_of(verdict: Verdict) -> Option<Score> {
    match verdict {
        Verdict::Good => Some(Score::Good),
        Verdict::Near => Some(Score::Near),
        Verdict::Bad => Some(Score::Bad),
        Verdict::Unknown => None,
    }
}

/// A round's overlay, built from its view each frame ([`RoundOverlay::of`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundOverlay {
    params: BTreeMap<ParamAddr, Score>,
    enables: BTreeMap<ModuleId, Score>,
    routes: BTreeMap<RouteAddr, Score>,
    /// With the `dim_unused` assist: the randomized parameters and the modules they belong to.
    dim: Option<(BTreeSet<ParamAddr>, BTreeSet<ModuleId>)>,
}

impl RoundOverlay {
    /// The overlay of `round`: its rings, and with `dim_unused` (the assist) its randomized parameters. The rings
    /// are empty before the first score is in, so nothing shows before Submit.
    pub fn of(round: &RoundView, dim_unused: bool) -> RoundOverlay {
        let mut o = RoundOverlay::default();
        for ring in &round.rings {
            let Some(score) = score_of(ring.verdict) else {
                continue;
            };
            match &ring.at {
                RingAt::Param(addr) => {
                    o.params.insert(addr.clone(), score);
                }
                RingAt::Route(route) => {
                    o.routes.insert(route.clone(), score);
                }
                RingAt::Enable(id) => {
                    o.enables.insert(id.clone(), score);
                }
            }
        }
        // A report from before `rings` existed (or a shell that fills `knobs` only) still rings its knobs.
        for knob in &round.knobs {
            if let Some(score) = score_of(knob.verdict) {
                o.params.entry(knob.addr.clone()).or_insert(score);
            }
        }
        if dim_unused && !round.params.is_empty() {
            let params: BTreeSet<ParamAddr> = round.params.iter().cloned().collect();
            let modules = params
                .iter()
                .filter_map(|a| match a.target() {
                    Target::Module(id) => Some(id.clone()),
                    Target::Voice | Target::Master => None,
                })
                .collect();
            o.dim = Some((params, modules));
        }
        o
    }

    /// Whether it shows anything at all.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
            && self.enables.is_empty()
            && self.routes.is_empty()
            && self.dim.is_none()
    }
}

impl Overlay for RoundOverlay {
    fn param(&self, addr: &ParamAddr) -> ParamOverlay {
        let emphasis = match (&self.dim, addr.target()) {
            (Some((params, _)), Target::Voice | Target::Master) if !params.contains(addr) => {
                Emphasis::Dimmed
            }
            _ => Emphasis::Normal,
        };
        ParamOverlay {
            emphasis,
            score: self.params.get(addr).copied(),
        }
    }

    fn module(&self, id: &ModuleId) -> Emphasis {
        match &self.dim {
            Some((_, modules)) if !modules.contains(id) => Emphasis::Dimmed,
            _ => Emphasis::Normal,
        }
    }

    fn enable(&self, id: &ModuleId) -> Option<Score> {
        self.enables.get(id).copied()
    }

    fn route(&self, route: &RouteAddr) -> Option<Score> {
        self.routes.get(route).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_session::game::Ring;

    fn ring(at: &str, verdict: Verdict) -> Ring {
        Ring {
            at: RingAt::parse(at).unwrap(),
            verdict,
            randomized: true,
        }
    }

    #[test]
    fn rings_go_to_params_routes_and_toggles_and_nothing_before_a_score() {
        let mut round = RoundView::default();
        assert!(
            RoundOverlay::of(&round, false).is_empty(),
            "no rings before the first score"
        );
        round.rings = vec![
            ring("osc1.wave", Verdict::Good),
            ring("lfo1 -> filter1.cutoff", Verdict::Near),
            ring("osc2.enabled", Verdict::Bad),
        ];
        let o = RoundOverlay::of(&round, false);
        assert_eq!(
            o.param(&"osc1.wave".parse().unwrap()).score,
            Some(Score::Good)
        );
        assert_eq!(o.param(&"osc1.semi".parse().unwrap()).score, None);
        assert_eq!(
            o.route(&"lfo1 -> filter1.cutoff".parse().unwrap()),
            Some(Score::Near)
        );
        assert_eq!(o.enable(&ModuleId::new("osc2").unwrap()), Some(Score::Bad));
        assert_eq!(
            o.module(&ModuleId::new("env1").unwrap()),
            Emphasis::Normal,
            "no dimming without the assist"
        );
    }

    #[test]
    fn the_dim_assist_dims_modules_the_target_left_alone() {
        let mut round = RoundView::default();
        round.params = vec!["osc1.wave".parse().unwrap(), "voice.glide".parse().unwrap()];
        let o = RoundOverlay::of(&round, true);
        assert_eq!(o.module(&ModuleId::new("osc1").unwrap()), Emphasis::Normal);
        assert_eq!(
            o.module(&ModuleId::new("filter1").unwrap()),
            Emphasis::Dimmed
        );
        assert_eq!(
            o.param(&"voice.glide".parse().unwrap()).emphasis,
            Emphasis::Normal
        );
        assert_eq!(
            o.param(&"voice.mode".parse().unwrap()).emphasis,
            Emphasis::Dimmed
        );
        assert_eq!(
            o.param(&"osc1.semi".parse().unwrap()).emphasis,
            Emphasis::Normal,
            "the module's emphasis dims it, not the param's"
        );
    }
}
