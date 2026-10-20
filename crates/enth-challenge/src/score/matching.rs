//! **Owner: lane ST** (module matching; pre-declared by lane F1R from the F1r review, S5). Lane SS maps the player's
//! modules onto the target's before the scoring render (ADR-0007's 2026-10-20 note, fix 2: seeds follow module ids),
//! and lane ST's structure score and the editor's rings use the same pairing. The reference is docs/challenge.md,
//! "Structure, combined and time".
//!
//! # The contract (stable since lane ST's first commit)
//!
//! - [`match_modules`]`(target, answer) -> `[`ModuleMatching`]: the context-free pairing of two patches (heard
//!   through the held note, nothing randomized), for `enth compare` on two arbitrary patches.
//! - [`Matcher`]: the pairing of one round. [`Matcher::for_round`]`(ctx)` once per challenge (in a scorer's
//!   `prepare`), then [`Matcher::pair`]`(answer)` per submit. **Lane SS and lane ST both use it**, so the scored
//!   render's relabelling and the rings agree.
//! - [`ModuleMatching`]: the pairs, the target modules the answer lacks ([`missing`](ModuleMatching::missing)) and
//!   the answer's extra ones ([`extra`](ModuleMatching::extra)); address maps both ways
//!   ([`answer_addr`](ModuleMatching::answer_addr), [`answer_route`](ModuleMatching::answer_route),
//!   [`target_addr`](ModuleMatching::target_addr)); and [`relabel`](ModuleMatching::relabel), the answer with its
//!   modules renamed to their partners' ids. [`Matcher::relabel`] also puts its routes in the target's summation
//!   order: **what SS renders** (`let m = matcher.pair(answer); let scored = matcher.relabel(&m, answer);`).
//!
//! Pure and deterministic: parameter space only, no render, no float that depends on the platform (`libm` only).
//!
//! # Equivalences: what counts as the same structure
//!
//! The rules of `enth_patch::canon` (disabled ≡ absent, depth 0 ≡ no route, generators and modulators commute),
//! applied to the patches as they are, so the pairing keeps the player's real ids (where the rings go):
//!
//! - **A module takes part only when it is present:** enabled, and for a generator not silent (level 0 with no
//!   route on its level), for a modulator live (the amplitude envelope, or the source of a route of non-zero depth
//!   onto something heard: the observability mask's module visibility). A disabled module, a silent generator, an
//!   unrouted LFO and a route of depth 0 are absent: they cost nothing and pair with nothing.
//! - **Modules pair per kind**, oscillators with oscillators, so a permutation of interchangeable modules (osc 1 and
//!   osc 2 swapped, re-added under another id, reordered) pairs by content, not by id.
//! - **The amplitude envelopes pair with each other**, whatever their values: the role, not the shape, decides.
//!
//! # The pairing: the most favourable reading
//!
//! Per kind, an exact minimum-cost assignment (dynamic programming over subsets; a section holds at most 8 modules),
//! pairing as many modules as the smaller side has. The cost of a pair is lexicographic:
//!
//! 1. **the structure loss** on what the round randomized: Σ (1 − sᵢ) over the target module's randomized
//!    parameters and the randomized routes it is the source of, with sᵢ the structure score's per-item score
//!    ([`super::structure::item_score`]); a target module left unpaired loses 1 per such item, plus 1 when its
//!    enable flag was drawn. So the pairing is the one that scores best for the player;
//! 2. the same loss over the other parameters either side's mask calls observable, and the other routes (a sync
//!    master paired with a non-master counts 1 here);
//! 3. how many pairs change id.
//!
//! Remaining ties go to the lexicographically first assignment (the target's modules in patch order, each taking the
//! earliest answer module that keeps the optimum), so equal costs keep ids and order. Kinds are paired in a fixed
//! order: generators twice (levels count as ratios to the target's first generator, whose partner the first round
//! finds), then the effects, then the modulators twice (a route's destination maps through the pairs found so far).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use enth_params::ParamSpec;
use enth_patch::addr::Target;
use enth_patch::canon::identical;
use enth_patch::catalogue::{ModuleKind, OscParam};
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ParamAddr, Patch, Route, RouteAddr, Section,
};

use super::TargetContext;
use super::structure::item_score;
use crate::model::Phrase;
use crate::phrases;
use crate::priors::{self, Jnd, ROUTE_JND};
use crate::random::{self, Drawn, Mask, Visibility, observable};

/// How the answer's modules pair with the target's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleMatching {
    /// (target id, answer id), in the target's module order.
    pairs: Vec<(ModuleId, ModuleId)>,
    /// Target modules with no partner, in the target's order.
    missing: Vec<ModuleId>,
    /// Answer modules with no partner, in the answer's order.
    extra: Vec<ModuleId>,
}

impl ModuleMatching {
    /// The answer's module paired with the target's `id`.
    pub fn answer_of(&self, id: &ModuleId) -> Option<&ModuleId> {
        self.pairs.iter().find(|(t, _)| t == id).map(|(_, a)| a)
    }

    /// The target's module paired with the answer's `id`.
    pub fn target_of(&self, id: &ModuleId) -> Option<&ModuleId> {
        self.pairs.iter().find(|(_, a)| a == id).map(|(t, _)| t)
    }

    /// Every pair, (target id, answer id), in the target's module order.
    pub fn pairs(&self) -> &[(ModuleId, ModuleId)] {
        &self.pairs
    }

    /// Present target modules the answer lacks, in the target's order.
    pub fn missing(&self) -> &[ModuleId] {
        &self.missing
    }

    /// Present answer modules the target lacks, in the answer's order.
    pub fn extra(&self) -> &[ModuleId] {
        &self.extra
    }

    /// Whether every present module pairs with the module of the same id on the other side, and none is left over.
    pub fn is_identity(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.pairs.iter().all(|(t, a)| t == a)
    }

    /// The answer's address for a target address: the same parameter on the paired module (voice and master
    /// addresses map to themselves); `None` when the module has no partner.
    pub fn answer_addr(&self, target: &ParamAddr) -> Option<ParamAddr> {
        match target.target() {
            Target::Module(id) => {
                ParamAddr::module(self.answer_of(id)?.clone(), target.param()).ok()
            }
            _ => Some(target.clone()),
        }
    }

    /// The target's address for an answer address (the reverse of [`answer_addr`](Self::answer_addr)).
    pub fn target_addr(&self, answer: &ParamAddr) -> Option<ParamAddr> {
        match answer.target() {
            Target::Module(id) => {
                ParamAddr::module(self.target_of(id)?.clone(), answer.param()).ok()
            }
            _ => Some(answer.clone()),
        }
    }

    /// The answer's route for a target route: both ends on their partners (velocity, key, the mod wheel, macros, and
    /// voice and master destinations map to themselves); `None` when an end's module has no partner.
    pub fn answer_route(&self, target: &RouteAddr) -> Option<RouteAddr> {
        let src = match &target.src {
            ModSource::Module(id) => ModSource::Module(self.answer_of(id)?.clone()),
            other => other.clone(),
        };
        Some(RouteAddr {
            src,
            dst: self.answer_addr(&target.dst)?,
        })
    }

    /// The target's route for an answer route (the reverse of [`answer_route`](Self::answer_route)).
    pub fn target_route(&self, answer: &RouteAddr) -> Option<RouteAddr> {
        let src = match &answer.src {
            ModSource::Module(id) => ModSource::Module(self.target_of(id)?.clone()),
            other => other.clone(),
        };
        Some(RouteAddr {
            src,
            dst: self.target_addr(&answer.dst)?,
        })
    }

    /// The answer with every paired module renamed to its partner's id: what lane SS renders, so random start phases
    /// and noise (seeded by module id) match the target's (ADR-0007's 2026-10-20 note, fix 2).
    ///
    /// - Unpaired modules keep their ids unless a partner took it; then they get the lowest free `<prefix><n>`.
    /// - Routes, the amplitude envelope and slots follow the new ids.
    /// - The commutative sections (generators, modulators) are put in the target's order (paired modules first, in
    ///   the target's order; the rest after, in the answer's). Generators keep their order when the answer uses hard
    ///   sync (the first oscillator is the sync master). `voice_fx` and `fx` are chains: never reordered.
    /// - Routes keep the answer's order here; [`Matcher::relabel`] also puts them in the target's summation order,
    ///   so an answer that equals the target up to ids and order renders the same samples.
    ///
    /// A derived copy for scoring, never the player's patch (which changes only through ops). A valid answer gives a
    /// valid patch.
    pub fn relabel(&self, answer: &Patch) -> Patch {
        let mut new_id: BTreeMap<ModuleId, ModuleId> = BTreeMap::new();
        let mut taken: BTreeSet<ModuleId> = BTreeSet::new();
        for (t, a) in &self.pairs {
            if answer.find(a).is_some() {
                new_id.insert(a.clone(), t.clone());
                taken.insert(t.clone());
            }
        }
        let mut clashing = Vec::new();
        for (_, m) in answer.modules() {
            if new_id.contains_key(&m.id) {
                continue;
            }
            if taken.insert(m.id.clone()) {
                new_id.insert(m.id.clone(), m.id.clone());
            } else {
                clashing.push(m);
            }
        }
        for m in clashing {
            let prefix = m.kind.spec().id_prefix;
            // A section holds at most 8 modules, so one of the first 64 numbers is free.
            let fresh = (1u32..=64)
                .filter_map(|n| ModuleId::numbered(prefix, n).ok())
                .find(|id| !taken.contains(id))
                .expect("a section is small, so a free number exists");
            taken.insert(fresh.clone());
            new_id.insert(m.id.clone(), fresh);
        }
        let rename = |id: &ModuleId| new_id.get(id).cloned().unwrap_or_else(|| id.clone());
        let paired: BTreeMap<&ModuleId, usize> = self
            .pairs
            .iter()
            .enumerate()
            .map(|(i, (t, _))| (t, i))
            .collect();
        let sync = uses_sync(answer);
        let mut out = answer.clone();
        for section in Section::ALL {
            let mut modules: Vec<ModuleInstance> = answer
                .section(section)
                .iter()
                .map(|m| ModuleInstance {
                    id: rename(&m.id),
                    ..m.clone()
                })
                .collect();
            let commutes = match section {
                Section::Generators => !sync,
                Section::Modulators => true,
                Section::VoiceFx | Section::Fx => false,
            };
            if commutes {
                // Paired modules (now under their partner's id) by the target's order, the rest after; stable.
                modules.sort_by_key(|m| paired.get(&m.id).copied().unwrap_or(usize::MAX));
            }
            *out.section_mut(section) = modules;
        }
        let rename_src = |src: &ModSource| match src {
            ModSource::Module(id) => ModSource::Module(rename(id)),
            other => other.clone(),
        };
        let rename_addr = |addr: &ParamAddr| match addr.target() {
            Target::Module(id) => {
                ParamAddr::module(rename(id), addr.param()).unwrap_or_else(|_| addr.clone())
            }
            _ => addr.clone(),
        };
        out.routes = answer
            .routes
            .iter()
            .map(|r| Route {
                src: rename_src(&r.src),
                dst: rename_addr(&r.dst),
                depth: r.depth,
            })
            .collect();
        out.voice.amp_env = rename(&answer.voice.amp_env);
        for addr in out.slots.iter_mut().flatten() {
            *addr = rename_addr(addr);
        }
        out
    }
}

/// Pair the answer's modules with the target's, with nothing known about a round: every observable parameter weighs
/// the same, heard through the held note (phrase 1). See the [module docs](self). For a round, use
/// [`Matcher::for_round`], which weighs what the round randomized first.
pub fn match_modules(target: &Patch, answer: &Patch) -> ModuleMatching {
    let held = phrases::phrase(phrases::HELD).expect("the held phrase is in the table");
    Matcher::new(target, held, &Draws::default()).pair(answer)
}

/// What the generator drew for a round: the parameters ([`TargetContext::randomized`]), and the routes and enable
/// flags it drew, which `TargetContext` does not carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draws {
    /// The randomized parameters, in address order.
    pub params: Vec<ParamAddr>,
    /// The randomized routes that are heard in the target (non-zero depth), in route order.
    pub routes: Vec<RouteAddr>,
    /// The modules whose enable flag was drawn (on or off), in id order.
    pub enables: Vec<ModuleId>,
}

impl Draws {
    /// What `ctx`'s generator drew. For a generated challenge the generator runs again on the start (pure, about
    /// 0.3 ms) and its draws are read when it gives `ctx.target` exactly; otherwise (a hand-made context) the routes
    /// and enable flags are what the target changed from the start.
    pub fn of(ctx: &TargetContext<'_>) -> Draws {
        let mut params = ctx.randomized.to_vec();
        params.sort();
        params.dedup();
        let generated = ctx
            .challenge
            .spec()
            .and_then(|spec| random::generate(spec, ctx.start).ok())
            .filter(|g| identical(&g.patch, ctx.target));
        let (mut routes, mut enables): (Vec<RouteAddr>, Vec<ModuleId>) = match generated {
            Some(g) => (
                g.randomized
                    .iter()
                    .filter_map(|r| match &r.drawn {
                        Drawn::Route(a) => Some(a.clone()),
                        _ => None,
                    })
                    .collect(),
                g.randomized
                    .iter()
                    .filter_map(|r| match &r.drawn {
                        Drawn::Enable(id) => Some(id.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            None => (
                ctx.target
                    .routes
                    .iter()
                    .filter(|r| {
                        !ctx.start
                            .routes
                            .iter()
                            .any(|s| s.src == r.src && s.dst == r.dst && s.depth == r.depth)
                    })
                    .map(|r| RouteAddr {
                        src: r.src.clone(),
                        dst: r.dst.clone(),
                    })
                    .collect(),
                ctx.target
                    .modules()
                    .filter(|(_, m)| {
                        ctx.start
                            .find(&m.id)
                            .is_some_and(|(_, s)| s.kind == m.kind && s.enabled != m.enabled)
                    })
                    .map(|(_, m)| m.id.clone())
                    .collect(),
            ),
        };
        let mask = observable(ctx.target, ctx.phrase);
        routes.retain(|r| mask.route(r).is_some_and(Visibility::is_observable));
        routes.sort();
        routes.dedup();
        enables.sort();
        enables.dedup();
        Draws {
            params,
            routes,
            enables,
        }
    }
}

/// The module pairing of one round: the target, its phrase and mask, and what was randomized, kept once per
/// challenge; [`pair`](Matcher::pair) per submit. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Matcher {
    target: Patch,
    phrase: Phrase,
    mask: Mask,
    params: BTreeSet<ParamAddr>,
    routes: BTreeSet<RouteAddr>,
    enables: BTreeSet<ModuleId>,
}

impl Matcher {
    /// A matcher for `target` heard through `phrase`, weighing `draws` first.
    pub fn new(target: &Patch, phrase: &Phrase, draws: &Draws) -> Matcher {
        Matcher {
            target: target.clone(),
            phrase: *phrase,
            mask: observable(target, phrase),
            params: draws.params.iter().cloned().collect(),
            routes: draws.routes.iter().cloned().collect(),
            enables: draws.enables.iter().cloned().collect(),
        }
    }

    /// The matcher of a round: [`Matcher::new`] with the context's target, phrase and [`Draws::of`]`(ctx)`. What
    /// lane SS keeps in its `prepare` so its relabelling pairs modules exactly as the structure score does.
    pub fn for_round(ctx: &TargetContext<'_>) -> Matcher {
        Matcher::new(ctx.target, ctx.phrase, &Draws::of(ctx))
    }

    /// The target.
    pub fn target(&self) -> &Patch {
        &self.target
    }

    /// The phrase both patches are heard through.
    pub fn phrase(&self) -> &Phrase {
        &self.phrase
    }

    /// The target's observability mask under the phrase.
    pub fn mask(&self) -> &Mask {
        &self.mask
    }

    /// Pair `answer`'s modules with the target's.
    pub fn pair(&self, answer: &Patch) -> ModuleMatching {
        let mask = observable(answer, &self.phrase);
        self.pair_with(answer, &mask)
    }

    /// The answer relabelled onto the target's ids ([`ModuleMatching::relabel`]), with its routes in the target's
    /// summation order: the patch lane SS renders.
    pub fn relabel(&self, matching: &ModuleMatching, answer: &Patch) -> Patch {
        let mut out = matching.relabel(answer);
        let index: BTreeMap<(&ModSource, &ParamAddr), usize> = self
            .target
            .routes
            .iter()
            .enumerate()
            .map(|(i, r)| ((&r.src, &r.dst), i))
            .collect();
        let mut keyed: Vec<(usize, Route)> = out
            .routes
            .drain(..)
            .map(|r| {
                (
                    index.get(&(&r.src, &r.dst)).copied().unwrap_or(usize::MAX),
                    r,
                )
            })
            .collect();
        keyed.sort_by_key(|(i, _)| *i);
        out.routes = keyed.into_iter().map(|(_, r)| r).collect();
        out
    }

    /// [`pair`](Self::pair) with the answer's mask already computed (the structure score needs it too).
    pub(crate) fn pair_with(&self, answer: &Patch, a_mask: &Mask) -> ModuleMatching {
        let target = &self.target;
        let t_present: Vec<&ModuleInstance> = target
            .modules()
            .map(|(_, m)| m)
            .filter(|m| present(target, &self.mask, m))
            .collect();
        let a_present: Vec<&ModuleInstance> = answer
            .modules()
            .map(|(_, m)| m)
            .filter(|m| present(answer, a_mask, m))
            .collect();
        let mut pairs: BTreeMap<ModuleId, ModuleId> = BTreeMap::new();
        // The amplitude envelopes are each other's partners.
        let t_amp = t_present
            .iter()
            .find(|m| m.id == target.voice.amp_env && m.kind == ModuleKind::Adsr);
        let a_amp = a_present
            .iter()
            .find(|m| m.id == answer.voice.amp_env && m.kind == ModuleKind::Adsr);
        let forced = match (t_amp, a_amp) {
            (Some(t), Some(a)) => {
                pairs.insert(t.id.clone(), a.id.clone());
                Some((t.id.clone(), a.id.clone()))
            }
            _ => None,
        };
        let job = Job {
            matcher: self,
            answer,
            a_mask,
            t_present: &t_present,
            a_present: &a_present,
            forced: forced.as_ref(),
            sync: uses_sync(target) || uses_sync(answer),
        };
        // Generators twice: first with each patch's first generator as its level reference, then with the partner
        // of the target's.
        let t_ref = level_ref_of(target);
        let mut a_ref = level_ref_of(answer);
        for round in 0..2 {
            for kind in [ModuleKind::Osc, ModuleKind::Noise] {
                job.assign(kind, t_ref, a_ref, &mut pairs);
            }
            if round == 0 {
                a_ref = partner_ref(t_ref, &pairs, answer);
            }
        }
        for kind in [ModuleKind::Svf, ModuleKind::Reverb, ModuleKind::Comp] {
            job.assign(kind, t_ref, a_ref, &mut pairs);
        }
        for _ in 0..2 {
            for kind in [ModuleKind::Adsr, ModuleKind::Lfo] {
                job.assign(kind, t_ref, a_ref, &mut pairs);
            }
        }
        let mut out = ModuleMatching::default();
        for m in &t_present {
            match pairs.get(&m.id) {
                Some(a) => out.pairs.push((m.id.clone(), a.clone())),
                None => out.missing.push(m.id.clone()),
            }
        }
        let taken: BTreeSet<&ModuleId> = pairs.values().collect();
        out.extra = a_present
            .iter()
            .filter(|m| !taken.contains(&m.id))
            .map(|m| m.id.clone())
            .collect();
        out
    }
}

/// One pairing in progress: the two patches, what is present, and the forced pair.
struct Job<'a> {
    matcher: &'a Matcher,
    answer: &'a Patch,
    a_mask: &'a Mask,
    t_present: &'a [&'a ModuleInstance],
    a_present: &'a [&'a ModuleInstance],
    forced: Option<&'a (ModuleId, ModuleId)>,
    sync: bool,
}

impl Job<'_> {
    /// (Re)pair the modules of one kind, with the other kinds' pairs as they stand.
    fn assign(
        &self,
        kind: ModuleKind,
        t_ref: LevelRef<'_>,
        a_ref: LevelRef<'_>,
        pairs: &mut BTreeMap<ModuleId, ModuleId>,
    ) {
        let is_forced = |id: &ModuleId, side: usize| {
            self.forced
                .is_some_and(|f| if side == 0 { &f.0 == id } else { &f.1 == id })
        };
        let ts: Vec<&ModuleInstance> = self
            .t_present
            .iter()
            .copied()
            .filter(|m| m.kind == kind && !is_forced(&m.id, 0))
            .collect();
        let answers: Vec<&ModuleInstance> = self
            .a_present
            .iter()
            .copied()
            .filter(|m| m.kind == kind && !is_forced(&m.id, 1))
            .collect();
        if ts.is_empty() {
            return;
        }
        // Costs read the other kinds' pairs (route destinations) as they stood before this kind is redone.
        let snapshot = pairs.clone();
        let reverse: BTreeMap<&ModuleId, &ModuleId> =
            snapshot.iter().map(|(t, a)| (a, t)).collect();
        let t_side = Side {
            patch: &self.matcher.target,
            level_ref: t_ref,
        };
        let a_side = Side {
            patch: self.answer,
            level_ref: a_ref,
        };
        let costs: Vec<Vec<Cost>> = ts
            .iter()
            .map(|t| {
                answers
                    .iter()
                    .map(|a| self.pair_cost(&t_side, t, &a_side, a, &snapshot, &reverse))
                    .collect()
            })
            .collect();
        let skips: Vec<Cost> = ts.iter().map(|t| self.skip_cost(t)).collect();
        for t in &ts {
            pairs.remove(&t.id);
        }
        for (t, choice) in ts.iter().zip(assign(&costs, &skips, answers.len())) {
            if let Some(j) = choice {
                pairs.insert(t.id.clone(), answers[j].id.clone());
            }
        }
    }

    /// The cost of pairing `t` with `a` (see the [module docs](self)).
    fn pair_cost(
        &self,
        t_side: &Side<'_>,
        t: &ModuleInstance,
        a_side: &Side<'_>,
        a: &ModuleInstance,
        pairs: &BTreeMap<ModuleId, ModuleId>,
        reverse: &BTreeMap<&ModuleId, &ModuleId>,
    ) -> Cost {
        let m = self.matcher;
        let mut cost = Cost {
            renamed: u32::from(t.id != a.id),
            ..Cost::ZERO
        };
        for (i, spec) in t.kind.params().iter().enumerate() {
            let (Ok(t_addr), Ok(a_addr)) = (
                ParamAddr::module(t.id.clone(), spec.key),
                ParamAddr::module(a.id.clone(), spec.key),
            ) else {
                continue;
            };
            let focus = m.params.contains(&t_addr);
            if !focus && !m.mask.is_observable(&t_addr) && !self.a_mask.is_observable(&a_addr) {
                continue;
            }
            let (steps, _) = module_steps(t_side, t, a_side, a, i);
            let loss = 1.0 - item_score(steps);
            if focus {
                cost.primary += loss;
            } else {
                cost.secondary += loss;
            }
        }
        if t.kind == ModuleKind::Osc
            && self.sync
            && is_sync_master(&m.target, &t.id) != is_sync_master(self.answer, &a.id)
        {
            cost.secondary += 1.0;
        }
        // Incoming routes, by their sources' kind (whose pairing may not be known yet): twins that differ only in
        // what modulates them (tremolo on one oscillator, vibrato on the other) are not interchangeable, and a
        // randomized route onto a module weighs on its pairing like its parameters do.
        let (focus, rest) = signature_loss(
            &incoming(&m.target, &m.mask, &t.id, &m.routes),
            &incoming(self.answer, self.a_mask, &a.id, &BTreeSet::new()),
        );
        cost.primary += focus;
        cost.secondary += rest;
        if t.kind.section() == Section::Modulators {
            let t_src = ModSource::Module(t.id.clone());
            let a_src = ModSource::Module(a.id.clone());
            for r in m
                .target
                .routes
                .iter()
                .filter(|r| r.src == t_src && r.depth != 0.0)
            {
                let addr = RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                };
                let focus = m.routes.contains(&addr);
                if !focus && !m.mask.route(&addr).is_some_and(Visibility::is_observable) {
                    continue;
                }
                let a_depth = map_dst(&r.dst, pairs)
                    .and_then(|dst| depth_of(self.answer, &a_src, &dst))
                    .unwrap_or(0.0);
                let loss = 1.0 - item_score(ROUTE_JND.steps(r.depth, a_depth));
                if focus {
                    cost.primary += loss;
                } else {
                    cost.secondary += loss;
                }
            }
            for r in self
                .answer
                .routes
                .iter()
                .filter(|r| r.src == a_src && r.depth != 0.0)
            {
                let addr = RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                };
                if !self
                    .a_mask
                    .route(&addr)
                    .is_some_and(Visibility::is_observable)
                {
                    continue;
                }
                let counted = unmap_dst(&r.dst, reverse)
                    .is_some_and(|dst| depth_of(&m.target, &t_src, &dst).is_some());
                if !counted {
                    cost.secondary += 1.0 - item_score(ROUTE_JND.steps(0.0, r.depth));
                }
            }
        }
        cost
    }

    /// The cost of leaving target module `t` unpaired: every item it carries is lost.
    fn skip_cost(&self, t: &ModuleInstance) -> Cost {
        let m = self.matcher;
        let mut cost = Cost::ZERO;
        for spec in t.kind.params() {
            let Ok(addr) = ParamAddr::module(t.id.clone(), spec.key) else {
                continue;
            };
            if m.params.contains(&addr) {
                cost.primary += 1.0;
            } else if m.mask.is_observable(&addr) {
                cost.secondary += 1.0;
            }
        }
        let src = ModSource::Module(t.id.clone());
        for r in m
            .target
            .routes
            .iter()
            .filter(|r| r.src == src && r.depth != 0.0)
        {
            let addr = RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            };
            if m.routes.contains(&addr) {
                cost.primary += 1.0;
            } else if m.mask.route(&addr).is_some_and(Visibility::is_observable) {
                cost.secondary += 1.0;
            }
        }
        for (_, _, _, randomized) in incoming(&m.target, &m.mask, &t.id, &m.routes) {
            if randomized {
                cost.primary += 1.0;
            } else {
                cost.secondary += 1.0;
            }
        }
        if m.enables.contains(&t.id) {
            cost.primary += 1.0;
        }
        cost
    }
}

/// One heard route onto a module, as its pairing sees it: the source's kind, the parameter, the depth, and whether
/// the round randomized it.
type Incoming<'p> = (&'p str, &'static str, f32, bool);

/// The heard routes onto module `id`, in patch order (`focus`: the randomized routes).
fn incoming<'p>(
    patch: &'p Patch,
    mask: &Mask,
    id: &ModuleId,
    focus: &BTreeSet<RouteAddr>,
) -> Vec<Incoming<'p>> {
    patch
        .routes
        .iter()
        .filter(|r| r.depth != 0.0 && r.dst.module_id() == Some(id))
        .filter_map(|r| {
            let addr = RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            };
            mask.route(&addr)
                .is_some_and(Visibility::is_observable)
                .then(|| (r, focus.contains(&addr)))
        })
        .map(|(r, focus)| {
            let class = match &r.src {
                ModSource::Module(s) => patch.find(s).map_or("", |(_, m)| m.kind.key()),
                ModSource::Velocity => "velocity",
                ModSource::Key => "key",
                ModSource::ModWheel => "mod_wheel",
                ModSource::Macro(_) => "macro",
            };
            (class, r.dst.param(), r.depth, focus)
        })
        .collect()
}

/// The route loss between two lists of incoming routes: each target route against the closest unused answer route
/// of the same source kind and parameter (absent: depth 0), then each unused answer route against depth 0. Returns
/// (the loss on randomized target routes, the rest).
fn signature_loss(t: &[Incoming<'_>], a: &[Incoming<'_>]) -> (f64, f64) {
    let loss = |x: f32, y: f32| 1.0 - item_score(ROUTE_JND.steps(x, y));
    let mut used = vec![false; a.len()];
    let (mut focus, mut rest) = (0.0, 0.0);
    for &(class, param, depth, randomized) in t {
        let total = if randomized { &mut focus } else { &mut rest };
        let best = a
            .iter()
            .enumerate()
            .filter(|(j, r)| !used[*j] && r.0 == class && r.1 == param)
            .min_by(|(i, x), (j, y)| loss(depth, x.2).total_cmp(&loss(depth, y.2)).then(i.cmp(j)));
        match best {
            Some((j, r)) => {
                used[j] = true;
                *total += loss(depth, r.2);
            }
            None => *total += loss(depth, 0.0),
        }
    }
    for (j, r) in a.iter().enumerate() {
        if !used[j] {
            rest += loss(0.0, r.2);
        }
    }
    (focus, rest)
}

fn map_dst(dst: &ParamAddr, pairs: &BTreeMap<ModuleId, ModuleId>) -> Option<ParamAddr> {
    match dst.target() {
        Target::Module(id) => ParamAddr::module(pairs.get(id)?.clone(), dst.param()).ok(),
        _ => Some(dst.clone()),
    }
}

fn unmap_dst(dst: &ParamAddr, reverse: &BTreeMap<&ModuleId, &ModuleId>) -> Option<ParamAddr> {
    match dst.target() {
        Target::Module(id) => ParamAddr::module((*reverse.get(id)?).clone(), dst.param()).ok(),
        _ => Some(dst.clone()),
    }
}

/// The depth of the route `src → dst`, when the patch has it with a non-zero depth.
pub(crate) fn depth_of(patch: &Patch, src: &ModSource, dst: &ParamAddr) -> Option<f32> {
    patch
        .routes
        .iter()
        .find(|r| &r.src == src && &r.dst == dst && r.depth != 0.0)
        .map(|r| r.depth)
}

// ---------------------------------------------------------------------------------------------------------------
// Presence and references
// ---------------------------------------------------------------------------------------------------------------

/// Whether a module takes part in matching (see the [module docs](self)): enabled, and for a generator not silent,
/// for a modulator live under `mask`.
pub(crate) fn present(patch: &Patch, mask: &Mask, m: &ModuleInstance) -> bool {
    m.enabled
        && match m.kind.section() {
            Section::Generators => !silent(patch, m),
            Section::Modulators => mask.module(&m.id) == Some(Visibility::Observable),
            Section::VoiceFx | Section::Fx => true,
        }
}

/// A generator at level 0 with no route of non-zero depth on its level.
fn silent(patch: &Patch, m: &ModuleInstance) -> bool {
    m.get("level") == Some(0.0)
        && !patch
            .routes
            .iter()
            .any(|r| r.depth != 0.0 && r.dst.module_id() == Some(&m.id) && r.dst.param() == "level")
}

/// Whether any enabled oscillator has hard sync on (the first enabled oscillator is then the sync master).
fn uses_sync(patch: &Patch) -> bool {
    patch.generators.iter().any(|m| {
        m.enabled
            && m.kind == ModuleKind::Osc
            && m.values
                .get(OscParam::Sync.index())
                .is_some_and(|&v| v != 0.0)
    })
}

fn is_sync_master(patch: &Patch, id: &ModuleId) -> bool {
    patch
        .generators
        .iter()
        .find(|m| m.enabled && m.kind == ModuleKind::Osc)
        .is_some_and(|m| &m.id == id)
}

/// The generator the other generators' levels are read against in one patch (levels count only as ratios between
/// sources; observability mask: the first enabled generator's level is a gauge), and its level. A missing reference
/// reads as level 0: absent ≡ silent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LevelRef<'a> {
    id: Option<&'a ModuleId>,
    level: f32,
}

/// The patch's first enabled generator as its level reference.
pub(crate) fn level_ref_of(patch: &Patch) -> LevelRef<'_> {
    let first = patch.generators.iter().find(|m| m.enabled);
    LevelRef {
        id: first.map(|m| &m.id),
        level: first.and_then(|m| m.get("level")).unwrap_or(0.0),
    }
}

/// The answer's partner of the target's reference generator as the answer's level reference (level 0 without one).
pub(crate) fn partner_ref<'a>(
    t_ref: LevelRef<'_>,
    pairs: &BTreeMap<ModuleId, ModuleId>,
    answer: &'a Patch,
) -> LevelRef<'a> {
    let partner = t_ref
        .id
        .and_then(|id| pairs.get(id))
        .and_then(|id| answer.find(id))
        .map(|(_, m)| m);
    LevelRef {
        id: partner.map(|m| &m.id),
        level: partner.and_then(|m| m.get("level")).unwrap_or(0.0),
    }
}

/// The answer's level reference for a finished matching (what the structure score reads levels against).
pub(crate) fn answer_ref<'a>(
    target: &Patch,
    matching: &ModuleMatching,
    answer: &'a Patch,
) -> LevelRef<'a> {
    let pairs: BTreeMap<ModuleId, ModuleId> = matching.pairs.iter().cloned().collect();
    partner_ref(level_ref_of(target), &pairs, answer)
}

// ---------------------------------------------------------------------------------------------------------------
// Readings: one value of the target against the answer's, in just-noticeable steps
// ---------------------------------------------------------------------------------------------------------------

/// One patch as a reading sees it: the patch and the level its generators' levels are read against.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Side<'a> {
    pub patch: &'a Patch,
    pub level_ref: LevelRef<'a>,
}

/// How far the answer's value at `a_addr` is from the target's at `t_addr`, in just-noticeable steps
/// ([`priors::jnd`]), and where the answer lies (`Greater`: higher). `None` when either side lacks the address or the
/// modules differ in kind. The folds of the [structure score](super::structure) apply.
pub(crate) fn steps(
    t: &Side<'_>,
    t_addr: &ParamAddr,
    a: &Side<'_>,
    a_addr: &ParamAddr,
) -> Option<(f64, Ordering)> {
    if let (Target::Module(ti), Target::Module(ai)) = (t_addr.target(), a_addr.target()) {
        let tm = t.patch.find(ti)?.1;
        let am = a.patch.find(ai)?.1;
        if tm.kind != am.kind {
            return None;
        }
        let index = tm.kind.param_index(t_addr.param())?;
        Some(module_steps(t, tm, a, am, index))
    } else {
        let spec = t.patch.spec(t_addr)?;
        Some(plain_steps(
            spec,
            t.patch.value(t_addr)?,
            a.patch.value(a_addr)?,
        ))
    }
}

/// [`steps`] for parameter `index` of two modules of the same kind, with the folds:
///
/// - an oscillator's `semi` and `fine` read as one pitch, 100·semi + fine cents (semi in semitone steps, fine in
///   cent steps), so enharmonic tunings (+12 st against +11 st and +100 ct) are equal;
/// - a generator's `level` reads in dB relative to its patch's reference generator (levels vary only as ratios);
/// - the reverb's `wet` reads relative to its `dry` (wet varies only against dry).
pub(crate) fn module_steps(
    t: &Side<'_>,
    tm: &ModuleInstance,
    a: &Side<'_>,
    am: &ModuleInstance,
    index: usize,
) -> (f64, Ordering) {
    let spec = &tm.kind.params()[index];
    let plain = || plain_steps(spec, tm.values[index], am.values[index]);
    match (tm.kind, spec.key, priors::jnd(spec)) {
        (ModuleKind::Osc, "semi" | "fine", Jnd::Abs(step)) => {
            let cents = |m: &ModuleInstance| {
                100.0 * f64::from(m.values[OscParam::Semi.index()])
                    + f64::from(m.values[OscParam::Fine.index()])
            };
            let per = if spec.key == "semi" { 100.0 } else { 1.0 };
            linear(cents(am) - cents(tm), per * f64::from(step))
        }
        (ModuleKind::Osc | ModuleKind::Noise, "level", Jnd::Db { db, floor }) => {
            let rel = |side: &Side<'_>, m: &ModuleInstance| {
                if side.level_ref.id == Some(&m.id) {
                    0.0
                } else {
                    let l = f64::from(m.get("level").unwrap_or(0.0)) + f64::from(floor);
                    let r = f64::from(side.level_ref.level) + f64::from(floor);
                    20.0 * libm::log10(l / r)
                }
            };
            linear(rel(a, am) - rel(t, tm), f64::from(db))
        }
        (ModuleKind::Reverb, "wet", Jnd::Abs(step)) => {
            let wet = |m: &ModuleInstance| {
                f64::from(m.get("wet").unwrap_or(0.0)) - f64::from(m.get("dry").unwrap_or(0.0))
            };
            linear(wet(am) - wet(tm), f64::from(step))
        }
        _ => plain(),
    }
}

/// A difference `d` in a unit whose step is `step`.
fn linear(d: f64, step: f64) -> (f64, Ordering) {
    (
        libm::fabs(d) / step,
        d.partial_cmp(&0.0).unwrap_or(Ordering::Equal),
    )
}

/// Two plain values in steps of the parameter's just-noticeable step (a periodic one wraps).
fn plain_steps(spec: &ParamSpec, t: f32, a: f32) -> (f64, Ordering) {
    let order = a.partial_cmp(&t).unwrap_or(Ordering::Equal);
    let steps = match priors::jnd(spec) {
        Jnd::Abs(step) if spec.flags.periodic && t != a => {
            let range = f64::from(spec.max()) - f64::from(spec.min());
            let d = libm::fmod(libm::fabs(f64::from(a) - f64::from(t)), range);
            d.min(range - d) / f64::from(step)
        }
        jnd => jnd.steps(t, a),
    };
    (steps, order)
}

// ---------------------------------------------------------------------------------------------------------------
// The assignment
// ---------------------------------------------------------------------------------------------------------------

/// A pair's cost, compared lexicographically: the structure loss on randomized items, the loss on the rest, renames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cost {
    primary: f64,
    secondary: f64,
    renamed: u32,
}

impl Cost {
    const ZERO: Cost = Cost {
        primary: 0.0,
        secondary: 0.0,
        renamed: 0,
    };

    fn plus(self, other: Cost) -> Cost {
        Cost {
            primary: self.primary + other.primary,
            secondary: self.secondary + other.secondary,
            renamed: self.renamed + other.renamed,
        }
    }

    fn order(&self, other: &Cost) -> Ordering {
        self.primary
            .total_cmp(&other.primary)
            .then(self.secondary.total_cmp(&other.secondary))
            .then(self.renamed.cmp(&other.renamed))
    }
}

/// Above this many columns the exact search would be too big (never in a valid patch: a section holds at most 8
/// modules), and a greedy pass takes over.
const MAX_EXACT: usize = 12;

/// For each row, its column or `None`, pairing `min(rows, columns)` of them at the least total cost: the pairs'
/// costs plus `skip[i]` for each unpaired row. Exact (dynamic programming over the used columns); ties go to the
/// lexicographically first choice (each row its earliest column that keeps the optimum, unpaired last).
fn assign(costs: &[Vec<Cost>], skip: &[Cost], columns: usize) -> Vec<Option<usize>> {
    let rows = costs.len();
    if rows == 0 || columns == 0 {
        return vec![None; rows];
    }
    let pairs = rows.min(columns);
    let skips = rows - pairs;
    if columns > MAX_EXACT {
        return greedy(costs, columns, pairs);
    }
    let size = 1usize << columns;
    let at = |i: usize, mask: usize| i * size + mask;
    let mut best: Vec<Option<Cost>> = vec![None; (rows + 1) * size];
    for mask in 0..size {
        if mask.count_ones() as usize == pairs {
            best[at(rows, mask)] = Some(Cost::ZERO);
        }
    }
    for i in (0..rows).rev() {
        for mask in 0..size {
            let used = mask.count_ones() as usize;
            if used > i || i - used > skips {
                continue;
            }
            let mut b: Option<Cost> = None;
            let mut consider = |c: Cost| {
                if b.is_none_or(|x| c.order(&x) == Ordering::Less) {
                    b = Some(c);
                }
            };
            for (j, cost) in costs[i].iter().enumerate() {
                let bit = 1usize << j;
                if mask & bit == 0
                    && let Some(rest) = best[at(i + 1, mask | bit)]
                {
                    consider(cost.plus(rest));
                }
            }
            if i - used < skips
                && let Some(rest) = best[at(i + 1, mask)]
            {
                consider(skip[i].plus(rest));
            }
            best[at(i, mask)] = b;
        }
    }
    let mut out = vec![None; rows];
    let mut mask = 0usize;
    for (i, slot) in out.iter_mut().enumerate() {
        let Some(goal) = best[at(i, mask)] else {
            break;
        };
        for (j, cost) in costs[i].iter().enumerate() {
            let bit = 1usize << j;
            if mask & bit == 0
                && let Some(rest) = best[at(i + 1, mask | bit)]
                && cost.plus(rest).order(&goal) == Ordering::Equal
            {
                *slot = Some(j);
                mask |= bit;
                break;
            }
        }
    }
    out
}

/// Each row in turn takes its cheapest free column (earliest on ties) until `pairs` are made.
fn greedy(costs: &[Vec<Cost>], columns: usize, pairs: usize) -> Vec<Option<usize>> {
    let mut used = vec![false; columns];
    let mut made = 0;
    costs
        .iter()
        .map(|row| {
            if made == pairs {
                return None;
            }
            let j = (0..columns)
                .filter(|&j| !used[j])
                .min_by(|&x, &y| row[x].order(&row[y]).then(x.cmp(&y)))?;
            used[j] = true;
            made += 1;
            Some(j)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Op;

    fn starter() -> Patch {
        random::start(0).unwrap()
    }

    fn set(p: &mut Patch, addr: &str, value: f32) {
        enth_patch::apply(
            p,
            &Op::SetParam {
                addr: addr.parse().unwrap(),
                value,
            },
        )
        .unwrap();
    }

    fn enable(p: &mut Patch, id: &str, enabled: bool) {
        enth_patch::apply(
            p,
            &Op::SetEnabled {
                id: ModuleId::new(id).unwrap(),
                enabled,
            },
        )
        .unwrap();
    }

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    fn route(p: &mut Patch, src: &str, dst: &str, depth: f32) {
        enth_patch::apply(
            p,
            &Op::InsertRoute {
                index: p.routes.len(),
                route: Route {
                    src: src.parse().unwrap(),
                    dst: dst.parse().unwrap(),
                    depth,
                },
            },
        )
        .unwrap();
    }

    fn side(p: &Patch) -> Side<'_> {
        Side {
            patch: p,
            level_ref: level_ref_of(p),
        }
    }

    #[test]
    fn a_patch_matches_itself_and_addresses_carry_over() {
        let p = Patch::init();
        let m = match_modules(&p, &p);
        assert_eq!((m.missing().len(), m.extra().len()), (0, 0));
        assert_eq!(m.pairs().len(), p.modules().count());
        assert!(m.is_identity());
        let addr: ParamAddr = "env1.attack".parse().unwrap();
        assert_eq!(m.answer_addr(&addr), Some(addr.clone()));
        assert_eq!(m.target_addr(&addr), Some(addr));
        let voice: ParamAddr = "voice.glide".parse().unwrap();
        assert_eq!(m.answer_addr(&voice), Some(voice));
        let empty = Patch {
            generators: Vec::new(),
            ..Patch::init()
        };
        let m = match_modules(&p, &empty);
        assert_eq!(m.missing(), &[id("osc1")]);
        let osc: ParamAddr = "osc1.semi".parse().unwrap();
        assert_eq!(m.answer_addr(&osc), None);
    }

    #[test]
    fn disabled_silent_and_unrouted_modules_are_absent() {
        let start = starter();
        let m = match_modules(&start, &start);
        // osc2, the noise, the compressor and the reverb are off; env2 and both LFOs drive nothing.
        let paired: Vec<&str> = m.pairs().iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(paired, ["osc1", "filter1", "env1"]);
        let mut answer = start.clone();
        enable(&mut answer, "osc2", true);
        assert_eq!(match_modules(&start, &answer).extra(), &[id("osc2")]);
        set(&mut answer, "osc2.level", 0.0);
        assert!(
            match_modules(&start, &answer).is_identity(),
            "a silent generator is absent"
        );
        let mut routed = start.clone();
        route(&mut routed, "lfo1", "filter1.cutoff", 0.0);
        assert!(
            match_modules(&start, &routed).is_identity(),
            "depth 0 is no route"
        );
    }

    #[test]
    fn swapped_oscillators_pair_by_content() {
        let mut target = starter();
        enable(&mut target, "osc2", true);
        set(&mut target, "osc1.wave", 2.0);
        set(&mut target, "osc2.semi", 12.0);
        // The answer has the same two oscillators under each other's ids.
        let mut answer = starter();
        enable(&mut answer, "osc2", true);
        set(&mut answer, "osc2.wave", 2.0);
        set(&mut answer, "osc1.semi", 12.0);
        let m = match_modules(&target, &answer);
        assert_eq!(m.answer_of(&id("osc1")), Some(&id("osc2")));
        assert_eq!(m.answer_of(&id("osc2")), Some(&id("osc1")));
        assert_eq!(
            m.answer_addr(&"osc1.wave".parse().unwrap()),
            Some("osc2.wave".parse().unwrap())
        );
        // Relabelled, the answer is the target up to the order of the commutative sections.
        let relabelled = m.relabel(&answer);
        enth_patch::validate::validate(&relabelled).unwrap();
        assert_eq!(
            enth_patch::canon::canonical_hash(&relabelled),
            enth_patch::canon::canonical_hash(&target)
        );
        assert_eq!(relabelled.generators[0].id, id("osc1"));
        assert_eq!(relabelled.generators[0].get("wave"), Some(2.0));
    }

    #[test]
    fn identical_twins_keep_their_ids() {
        let mut target = starter();
        enable(&mut target, "osc2", true);
        let m = match_modules(&target, &target);
        assert!(m.is_identity(), "{m:?}");
    }

    #[test]
    fn the_amplitude_envelopes_pair_whatever_their_shape() {
        let mut target = starter();
        set(&mut target, "env1.attack", 900.0);
        // env2 drives the filter in both, and the answer's env2 looks like the target's env1.
        route(&mut target, "env2", "filter1.cutoff", 40.0);
        let mut answer = target.clone();
        set(&mut answer, "env1.attack", 5.0);
        set(&mut answer, "env2.attack", 900.0);
        let m = match_modules(&target, &answer);
        assert_eq!(m.answer_of(&id("env1")), Some(&id("env1")));
        assert_eq!(m.answer_of(&id("env2")), Some(&id("env2")));
    }

    #[test]
    fn relabel_renames_clashing_extras_and_follows_routes() {
        let target = starter();
        // The answer's only oscillator is osc2 (osc1 off): it pairs with the target's osc1, and its disabled osc1
        // must give up the id.
        let mut answer = starter();
        enable(&mut answer, "osc2", true);
        enable(&mut answer, "osc1", false);
        route(&mut answer, "lfo1", "osc2.fine", 10.0);
        let m = match_modules(&target, &answer);
        assert_eq!(m.answer_of(&id("osc1")), Some(&id("osc2")));
        let r = m.relabel(&answer);
        enth_patch::validate::validate(&r).unwrap();
        let osc1 = r.find(&id("osc1")).unwrap().1;
        assert!(osc1.enabled, "the answer's osc2 is now osc1");
        let other = r.find(&id("osc2")).unwrap().1;
        assert!(
            !other.enabled,
            "the answer's disabled osc1 took the free id"
        );
        assert_eq!(r.routes[0].dst, "osc1.fine".parse().unwrap());
        // The extra LFO is not the target's: it keeps its id.
        assert_eq!(m.extra(), &[id("lfo1")]);
        assert_eq!(r.routes[0].src, "lfo1".parse().unwrap());
    }

    #[test]
    fn the_assignment_is_exact_and_breaks_ties_in_order() {
        let c = |p: f64| Cost {
            primary: p,
            ..Cost::ZERO
        };
        // A greedy pass would give row 0 column 0 (cost 1) and row 1 column 1 (cost 10).
        let costs = vec![vec![c(1.0), c(2.0)], vec![c(2.0), c(10.0)]];
        assert_eq!(assign(&costs, &[c(9.0); 2], 2), [Some(1), Some(0)]);
        // Ties: the first row takes the first column.
        let costs = vec![vec![c(1.0), c(1.0)], vec![c(1.0), c(1.0)]];
        assert_eq!(assign(&costs, &[c(9.0); 2], 2), [Some(0), Some(1)]);
        // More rows than columns: the row that loses least when left over is skipped.
        let costs = vec![vec![c(0.0)], vec![c(0.0)], vec![c(0.0)]];
        assert_eq!(
            assign(&costs, &[c(3.0), c(1.0), c(2.0)], 1),
            [Some(0), None, None]
        );
        assert_eq!(
            assign(&costs, &[c(1.0), c(3.0), c(2.0)], 1),
            [None, Some(0), None]
        );
        // Fewer rows than columns: every row is paired.
        let costs = vec![vec![c(5.0), c(1.0), c(3.0)]];
        assert_eq!(assign(&costs, &[c(0.0)], 3), [Some(1)]);
        assert_eq!(assign(&[], &[], 3), Vec::<Option<usize>>::new());
        assert_eq!(assign(&costs, &[c(0.0)], 0), [None]);
    }

    #[test]
    fn readings_fold_pitch_levels_and_wet() {
        let start = starter();
        let mut a = start.clone();
        set(&mut a, "osc1.semi", 11.0);
        set(&mut a, "osc1.fine", 100.0);
        let mut t = start.clone();
        set(&mut t, "osc1.semi", 12.0);
        let semi: ParamAddr = "osc1.semi".parse().unwrap();
        let fine: ParamAddr = "osc1.fine".parse().unwrap();
        assert_eq!(steps(&side(&t), &semi, &side(&a), &semi).unwrap().0, 0.0);
        assert_eq!(steps(&side(&t), &fine, &side(&a), &fine).unwrap().0, 0.0);
        set(&mut a, "osc1.fine", 0.0);
        let (s, o) = steps(&side(&t), &semi, &side(&a), &semi).unwrap();
        assert_eq!((s, o), (1.0, Ordering::Less));
        // Levels are ratios: both oscillators 6 dB down is the same mix.
        let mut t2 = start.clone();
        enable(&mut t2, "osc2", true);
        set(&mut t2, "osc2.level", 50.0);
        let mut a2 = t2.clone();
        set(&mut a2, "osc1.level", 50.0);
        set(&mut a2, "osc2.level", 24.5);
        let level: ParamAddr = "osc2.level".parse().unwrap();
        let (s, _) = steps(&side(&t2), &level, &side(&a2), &level).unwrap();
        assert!(s < 0.05, "{s}: the 1 % floor keeps it from being exact");
        // Without the ratio, 50 % against 24.5 % would be two steps.
        assert!(priors::jnd(t2.spec(&level).unwrap()).steps(50.0, 24.5) > 1.9);
        // Wet against dry.
        let mut t3 = start.clone();
        enable(&mut t3, "reverb1", true);
        let mut a3 = t3.clone();
        set(&mut a3, "reverb1.wet", -12.0);
        set(&mut a3, "reverb1.dry", -6.0);
        let wet: ParamAddr = "reverb1.wet".parse().unwrap();
        assert_eq!(steps(&side(&t3), &wet, &side(&a3), &wet).unwrap().0, 0.0);
    }
}
