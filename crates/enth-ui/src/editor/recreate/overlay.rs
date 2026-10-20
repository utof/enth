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
//!
//! **The latest hint points at what its text names** (lane HINT, FU1; [`RoundView::hint`](enth_session::game::RoundView),
//! the session's [`CoachHint`] kept on the coach's ladder, docs/dsp/hints.md): an outline in the value accent, never a
//! value. It follows the level ladder:
//!
//! | Level | The text says | Marked |
//! | --- | --- | --- |
//! | 1 | which aspect is off, and how | nothing (an aspect is not a control) |
//! | 2 | which module (or region) | the module's card; nothing for a region (the voice settings, the modulation routes, general advice) |
//! | 3 | which parameter, and which way | the parameter's control, with ↑ or ↓ when the text says raise or lower (a categorical parameter or a route's destination gets the outline alone); the card instead while "More" hides the control |
//! | 4 | a value range (a route: its source too) | as level 3; a route hint also outlines its source module's card |
//!
//! The mark clears when the next hint replaces it, at a reveal, and when the round ends (no round view, no overlay).

use std::collections::{BTreeMap, BTreeSet};

use enth_challenge::Verdict;
use enth_patch::{ModSource, ModuleId, ParamAddr, RouteAddr, Target};
use enth_session::game::{CoachHint, HintTurn, RingAt, RoundView};

use crate::editor::overlay::{Emphasis, Overlay, ParamOverlay, Pointer};
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
    /// The control the latest hint points at (lane HINT), and how.
    hint_param: Option<(ParamAddr, Pointer)>,
    /// The modules the latest hint outlines whole (lane HINT).
    hint_modules: BTreeSet<ModuleId>,
}

/// The pointer for a hint's turn: ↑ or ↓ when the text says raise or lower, else the outline alone.
fn pointer(turn: Option<HintTurn>) -> Pointer {
    match turn {
        Some(HintTurn::Raise) => Pointer::Raise,
        Some(HintTurn::Lower) => Pointer::Lower,
        Some(HintTurn::Unknown) | None => Pointer::Here,
    }
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
        // The latest hint, while the round can still be played (a reveal shows the answer instead).
        if round.phase.is_open()
            && let Some(hint) = &round.hint
        {
            o.point(hint);
        }
        o
    }

    /// Mark what `hint` names, on the level ladder (see the [module docs](self)).
    fn point(&mut self, hint: &CoachHint) {
        let hint = hint.clone().laddered();
        match (&hint.param, &hint.module) {
            (Some(param), _) => self.hint_param = Some((param.clone(), pointer(hint.turn))),
            (None, Some(module)) => {
                self.hint_modules.insert(module.clone());
            }
            (None, None) => {}
        }
        if let Some(RouteAddr {
            src: ModSource::Module(id),
            ..
        }) = &hint.route
        {
            self.hint_modules.insert(id.clone());
        }
    }

    /// Whether it shows anything at all.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
            && self.enables.is_empty()
            && self.routes.is_empty()
            && self.dim.is_none()
            && self.hint_param.is_none()
            && self.hint_modules.is_empty()
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
            hint: self
                .hint_param
                .as_ref()
                .filter(|(a, _)| a == addr)
                .map(|(_, p)| *p),
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

    fn module_hint(&self, id: &ModuleId) -> bool {
        self.hint_modules.contains(id)
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

    fn hinted(level: u8) -> RoundView {
        let mut round = RoundView::default();
        round.phase = enth_session::game::Phase::Playing;
        round.hint = Some(
            CoachHint::new(level, "a hint")
                .with_aspect("sweep", None)
                .with_module(ModuleId::new("filter1").unwrap())
                .with_param("filter1.cutoff".parse().unwrap(), Some(HintTurn::Raise))
                .with_route("lfo1 -> filter1.cutoff".parse().unwrap()),
        );
        round
    }

    /// Lane HINT (FU1): each level marks what its text names, and never more, even from an over-filled hint.
    #[test]
    fn a_hint_marks_what_its_level_names() {
        let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
        let filter = ModuleId::new("filter1").unwrap();
        let lfo = ModuleId::new("lfo1").unwrap();
        let o = RoundOverlay::of(&hinted(1), false);
        assert!(o.is_empty(), "level 1 names an aspect, not a control");
        let o = RoundOverlay::of(&hinted(2), false);
        assert!(o.module_hint(&filter) && !o.module_hint(&lfo));
        assert_eq!(o.param(&cutoff).hint, None);
        let o = RoundOverlay::of(&hinted(3), false);
        assert_eq!(o.param(&cutoff).hint, Some(Pointer::Raise));
        assert_eq!(o.param(&"filter1.resonance".parse().unwrap()).hint, None);
        assert!(
            !o.module_hint(&filter),
            "the control, not its card, once the text names it"
        );
        let o = RoundOverlay::of(&hinted(4), false);
        assert_eq!(o.param(&cutoff).hint, Some(Pointer::Raise));
        assert!(o.module_hint(&lfo), "a route's source at level 4");
        // A categorical parameter (no turn) gets the outline alone.
        let mut round = hinted(3);
        round.hint = Some(
            CoachHint::new(3, "change Wave")
                .with_param("osc1.wave".parse().unwrap(), None),
        );
        let o = RoundOverlay::of(&round, false);
        assert_eq!(
            o.param(&"osc1.wave".parse().unwrap()).hint,
            Some(Pointer::Here)
        );
        // A reveal, or no round in play, marks nothing.
        for phase in [
            enth_session::game::Phase::Revealed,
            enth_session::game::Phase::Preparing,
            enth_session::game::Phase::Broken,
        ] {
            let mut round = hinted(4);
            round.phase = phase;
            assert!(RoundOverlay::of(&round, false).is_empty(), "{phase:?}");
        }
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
