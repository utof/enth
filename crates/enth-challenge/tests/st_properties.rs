//! Structure properties P8 and P9 (lane ST; docs/phase-2.md "Test strategy for the game logic", docs/challenge.md
//! "Structure, combined and time"). Fixed seed in the gate (`PROPTEST_RNG_SEED`) and proptest's default 256 cases;
//! 10,000 nightly.
//!
//! - **P8** Structure equivalences score 100: the target against itself; a module off ≡ absent (a disabled or silent
//!   module, whatever its values, added or not); depth 0 ≡ no route; permuted and relabelled identical modules
//!   (ids swapped, sections reordered, the canonical form); enharmonic tunings; levels as ratios; gauges and
//!   nuisances. A disabled extra module costs nothing on any answer, not only the target.
//! - **P9** Structure is monotone along any single parameter toward the target: an error injected into one
//!   observable parameter or route of the target, growing, never raises the score.
//! - Determinism: the same inputs give the same bits (the native = wasm check is `enth-golden`'s `st_structure`).

use enth_challenge::random;
use enth_challenge::score::matching::Matcher;
use enth_challenge::score::{ScorerId, StructureScore, Verdict};
use enth_challenge::spec::MAX_SEED;
use enth_challenge::{
    Category, Challenge, PreparedRound, RandomSpec, prepare_challenge, score_structure,
};
use enth_patch::canon::{canonical_hash, canonicalise};
use enth_patch::model::MAX_GENERATORS;
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ModuleKind, Op, ParamAddr, Patch, Position, Route,
    RouteAddr, Section,
};
use proptest::prelude::*;

fn amounts() -> impl Strategy<Value = [Option<u8>; Category::COUNT]> {
    proptest::array::uniform6(prop_oneof![6 => Just(None), 1 => (0u8..=100).prop_map(Some)])
}

/// Any spec of generator v1 on the starter, with the higher levels drawn more often (they randomize more).
fn specs() -> impl Strategy<Value = RandomSpec> {
    (
        0..=MAX_SEED,
        prop_oneof![1 => 0u8..=100, 2 => 61u8..=100],
        0u8..=100,
        amounts(),
    )
        .prop_map(|(seed, difficulty, musical, amounts)| RandomSpec {
            generator: random::GENERATOR_V1,
            seed,
            difficulty,
            amounts,
            musical,
            template: random::TEMPLATE_STARTER,
        })
}

/// The round of `spec` with the sound stub: these tests read structure only, and `sound/1`'s `prepare` renders
/// (its floor and JND scale), which would make 256 cases slow.
fn round(spec: &RandomSpec) -> PreparedRound {
    let generated = random::target(spec).unwrap();
    let mut challenge = Challenge::recreate(*spec, generated.phrase);
    challenge.scoring.sound = ScorerId::SOUND_STUB;
    prepare_challenge(challenge).unwrap()
}

fn score(round: &PreparedRound, answer: &Patch) -> StructureScore {
    enth_patch::validate::validate(answer).unwrap();
    let s = score_structure(round, answer).unwrap();
    assert_eq!(s.scorer, ScorerId::STRUCTURE_V1);
    assert!((0.0..=100.0).contains(&s.percent), "{}", s.percent);
    s
}

fn apply(p: &mut Patch, op: &Op) {
    enth_patch::apply(p, op).unwrap();
}

fn set(p: &mut Patch, addr: &ParamAddr, value: f32) {
    apply(
        p,
        &Op::SetParam {
            addr: addr.clone(),
            value,
        },
    );
}

fn depth(p: &Patch, r: &RouteAddr) -> f32 {
    p.routes
        .iter()
        .find(|x| x.src == r.src && x.dst == r.dst)
        .map_or(0.0, |x| x.depth)
}

fn set_depth(p: &mut Patch, r: &RouteAddr, depth: f32) {
    if p.routes.iter().any(|x| x.src == r.src && x.dst == r.dst) {
        apply(
            p,
            &Op::SetDepth {
                route: r.clone(),
                depth,
            },
        );
    } else {
        apply(
            p,
            &Op::InsertRoute {
                index: p.routes.len(),
                route: Route {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                    depth,
                },
            },
        );
    }
}

/// `p` with modules `a` and `b` trading ids everywhere (routes, the amplitude envelope, slots): the same patch.
fn swap_ids(p: &Patch, a: &str, b: &str) -> Patch {
    let (a, b) = (ModuleId::new(a).unwrap(), ModuleId::new(b).unwrap());
    let swap = |id: &ModuleId| {
        if *id == a {
            b.clone()
        } else if *id == b {
            a.clone()
        } else {
            id.clone()
        }
    };
    let swap_addr = |addr: &ParamAddr| match addr.module_id() {
        Some(id) => ParamAddr::module(swap(id), addr.param()).unwrap(),
        None => addr.clone(),
    };
    let mut out = p.clone();
    for section in Section::ALL {
        for m in out.section_mut(section) {
            m.id = swap(&m.id);
        }
    }
    for r in &mut out.routes {
        if let ModSource::Module(id) = &r.src {
            r.src = ModSource::Module(swap(id));
        }
        r.dst = swap_addr(&r.dst);
    }
    out.voice.amp_env = swap(&out.voice.amp_env);
    for addr in out.slots.iter_mut().flatten() {
        *addr = swap_addr(addr);
    }
    out
}

fn uses_sync(p: &Patch) -> bool {
    p.generators
        .iter()
        .any(|m| m.enabled && m.kind == ModuleKind::Osc && m.get("sync") == Some(1.0))
}

/// Patches that equal the target structurally, each named.
fn equivalents(target: &Patch) -> Vec<(&'static str, Patch)> {
    let mut out = Vec::new();
    out.push(("itself", target.clone()));
    out.push((
        "ids swapped",
        swap_ids(
            &swap_ids(&swap_ids(target, "osc1", "osc2"), "lfo1", "lfo2"),
            "env1",
            "env2",
        ),
    ));
    let mut reordered = swap_ids(target, "osc1", "osc2");
    if !uses_sync(target) {
        reordered.generators.reverse();
    }
    reordered.modulators.reverse();
    out.push(("ids swapped and sections reordered", reordered));
    out.push(("canonical form", canonicalise(target)));
    // A module off is absent, whatever its values.
    let mut off = target.clone();
    for (_, m) in target.modules().filter(|(_, m)| !m.enabled) {
        for (i, spec) in m.kind.params().iter().enumerate() {
            let v = if i % 2 == 0 { spec.max() } else { spec.min() };
            set(
                &mut off,
                &ParamAddr::module(m.id.clone(), spec.key).unwrap(),
                v,
            );
        }
    }
    out.push(("disabled modules changed", off));
    if target.generators.len() < MAX_GENERATORS {
        for (name, enabled, level) in [
            ("an extra disabled oscillator", false, 100.0),
            ("an extra silent oscillator", true, 0.0),
        ] {
            let mut extra = target.clone();
            let mut m = ModuleInstance::new(ModuleId::new("osc9").unwrap(), ModuleKind::Osc);
            m.enabled = enabled;
            m.values[enth_patch::catalogue::OscParam::Level.index()] = level;
            m.values[enth_patch::catalogue::OscParam::Wave.index()] = 2.0;
            apply(
                &mut extra,
                &Op::AddModule {
                    at: Position {
                        section: Section::Generators,
                        index: target.generators.len(),
                    },
                    module: m,
                },
            );
            out.push((name, extra));
        }
    }
    // Depth 0 is no route.
    let lfo2 = RouteAddr {
        src: "lfo2".parse().unwrap(),
        dst: "filter1.cutoff".parse().unwrap(),
    };
    if depth(target, &lfo2) == 0.0 && target.routes.len() < enth_patch::model::MAX_ROUTES {
        let mut zero = target.clone();
        set_depth(&mut zero, &lfo2, 0.0);
        out.push(("a depth-0 route", zero));
    }
    // Enharmonic: 100·semi + fine kept.
    let mut enharmonic = target.clone();
    for id in ["osc1", "osc2"] {
        let semi: ParamAddr = format!("{id}.semi").parse().unwrap();
        let fine: ParamAddr = format!("{id}.fine").parse().unwrap();
        let (s, f) = (target.value(&semi).unwrap(), target.value(&fine).unwrap());
        let (s2, f2) = if f <= 0.0 {
            (s - 1.0, f + 100.0)
        } else {
            (s + 1.0, f - 100.0)
        };
        if (-48.0..=48.0).contains(&s2) {
            set(&mut enharmonic, &semi, s2);
            set(&mut enharmonic, &fine, f2);
        }
    }
    out.push(("enharmonic tunings", enharmonic));
    // Levels are ratios: every source 6 dB down.
    let mut quieter = target.clone();
    for m in &target.generators {
        let addr = ParamAddr::module(m.id.clone(), "level").unwrap();
        set(&mut quieter, &addr, 0.5 * m.get("level").unwrap());
    }
    out.push(("every source 6 dB down", quieter));
    // Gauges and nuisances.
    let mut gauges = target.clone();
    set(&mut gauges, &"master.level".parse().unwrap(), -9.0);
    set(&mut gauges, &"voice.pan".parse().unwrap(), 30.0);
    for key in ["osc1.phase", "osc2.phase", "lfo1.phase"] {
        set(&mut gauges, &key.parse().unwrap(), 90.0);
    }
    set(&mut gauges, &"osc1.seed".parse().unwrap(), 777.0);
    out.push(("gauges and nuisances moved", gauges));
    out
}

/// A player's answer: the target with a few observable parameters moved to random values.
fn answer_from(target: &Patch, round: &PreparedRound, picks: &[(usize, f64)]) -> Patch {
    let mask = round.generated().mask();
    let params: Vec<ParamAddr> = mask.observable_params().cloned().collect();
    let mut answer = target.clone();
    for &(i, u) in picks {
        let addr = &params[i % params.len()];
        let spec = target.spec(addr).unwrap();
        set(&mut answer, addr, value_at(spec, u));
    }
    answer
}

/// The value at fraction `u` of a parameter's range (an item index for a discrete one).
fn value_at(spec: &enth_params::ParamSpec, u: f64) -> f32 {
    let (lo, hi) = (f64::from(spec.min()), f64::from(spec.max()));
    let v = lo + u * (hi - lo);
    if spec.is_discrete() {
        v.round() as f32
    } else {
        v as f32
    }
}

proptest! {
    /// P8: the target scores 100 against itself, and so does every equivalent patch, with every ring good and no
    /// module left over.
    #[test]
    fn p8_equivalent_patches_score_100(spec in specs()) {
        let round = round(&spec);
        let target = round.target().clone();
        for (name, answer) in equivalents(&target) {
            let s = score(&round, &answer);
            prop_assert_eq!(s.percent, 100.0, "{}: {:?}", name, s);
            prop_assert!(s.modules.is_empty(), "{}: {:?}", name, s.modules);
            prop_assert!(
                s.params.iter().all(|p| p.verdict == Verdict::Good),
                "{}: {:?}", name, s.params
            );
            if name == "itself" {
                prop_assert!(s.params.iter().all(|p| p.randomized && p.answer_addr.as_deref() == Some(p.addr.as_str())));
                prop_assert_eq!(
                    s.params.iter().filter(|p| p.param_addr().is_ok()).count(),
                    round.params().len()
                );
            }
        }
    }

    /// P8: a disabled extra module costs nothing on any answer, and an enabled one costs on the target. (On another
    /// answer an enabled extra may raise the score: when it fits the target better than the player's own module,
    /// the matching reads it as the partner, and the player's module becomes the extra one.)
    #[test]
    fn p8_a_disabled_extra_module_costs_nothing(
        spec in specs(),
        picks in proptest::collection::vec((0usize..64, 0.0f64..=1.0), 0..4),
        wave in 0.0f64..=1.0,
    ) {
        let round = round(&spec);
        let answer = answer_from(round.target(), &round, &picks);
        let base = score(&round, &answer);
        prop_assume!(answer.generators.len() < MAX_GENERATORS);
        let mut m = ModuleInstance::new(ModuleId::new("osc9").unwrap(), ModuleKind::Osc);
        m.values[0] = value_at(ModuleKind::Osc.params().first().unwrap(), wave);
        let at = Position { section: Section::Generators, index: answer.generators.len() };
        let mut off = answer.clone();
        m.enabled = false;
        apply(&mut off, &Op::AddModule { at, module: m.clone() });
        prop_assert_eq!(score(&round, &off).percent.to_bits(), base.percent.to_bits());
        let mut on = round.target().clone();
        m.enabled = true;
        apply(&mut on, &Op::AddModule { at, module: m });
        let with = score(&round, &on);
        prop_assert!(with.percent < 100.0, "an enabled extra oscillator costs: {:?}", with);
        prop_assert_eq!(with.modules.len(), 1);
    }

    /// P9: an error injected into one observable parameter of the target, growing towards any value, never raises
    /// structure %.
    #[test]
    fn p9_monotone_along_a_parameter(spec in specs(), pick in 0usize..1000, u in 0.0f64..=1.0) {
        let round = round(&spec);
        let target = round.target().clone();
        let mask = round.generated().mask();
        let params: Vec<ParamAddr> = mask.observable_params().cloned().collect();
        let addr = &params[pick % params.len()];
        let spec = target.spec(addr).unwrap();
        let (t, v) = (target.value(addr).unwrap(), value_at(spec, u));
        let steps: &[f32] = if spec.is_discrete() { &[0.0, 1.0] } else { &[0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0] };
        let mut last = f64::INFINITY;
        for &lambda in steps {
            let mut answer = target.clone();
            let x = if lambda == 1.0 { v } else { t + lambda * (v - t) };
            set(&mut answer, addr, x);
            let s = score(&round, &answer).percent;
            prop_assert!(s <= last, "{} at {} ({} → {}): {} > {}", addr, lambda, t, v, s, last);
            last = s;
        }
    }

    /// P9 for routes: a randomized route's depth moved away from the target's, up to 0 when it changes sign (at
    /// depth 0 the route is absent, and so may be its modulator), never raises structure %.
    #[test]
    fn p9_monotone_along_a_route(spec in specs(), pick in 0usize..1000, u in -1.0f64..=1.0) {
        let round = round(&spec);
        let target = round.target().clone();
        let mask = round.generated().mask();
        let routes: Vec<RouteAddr> = mask
            .routes()
            .filter(|(_, v)| v.is_observable())
            .map(|(r, _)| r.clone())
            .collect();
        if routes.is_empty() {
            return Ok(());
        }
        let route = &routes[pick % routes.len()];
        let t = depth(&target, route);
        let mut v = (100.0 * u) as f32;
        if v * t < 0.0 {
            v = 0.0;
        }
        let mut last = f64::INFINITY;
        for lambda in [0.0f32, 0.2, 0.5, 0.8, 1.0] {
            let mut answer = target.clone();
            set_depth(&mut answer, route, t + lambda * (v - t));
            let s = score(&round, &answer).percent;
            prop_assert!(s <= last, "{} at {}: {} > {}", route, lambda, s, last);
            last = s;
        }
    }

    /// Deterministic: preparing and scoring twice gives the same bits, and a relabelled equivalent answer is, for
    /// lane SS, the target again (the same canonical form, every partner under the target's id).
    #[test]
    fn determinism_and_relabelling(
        spec in specs(),
        picks in proptest::collection::vec((0usize..64, 0.0f64..=1.0), 0..4),
    ) {
        let a = round(&spec);
        let b = round(&spec);
        let answer = answer_from(a.target(), &a, &picks);
        prop_assert_eq!(score(&a, &answer), score(&b, &answer));
        let target = a.target();
        let ctx_matcher = matcher_for(&a);
        for (name, equivalent) in equivalents(target) {
            let m = ctx_matcher.pair(&equivalent);
            let relabelled = ctx_matcher.relabel(&m, &equivalent);
            prop_assert!(enth_patch::validate::validate(&relabelled).is_ok(), "{}", name);
            // Pure relabellings and reorderings come back as the target itself.
            if matches!(name, "itself" | "ids swapped" | "ids swapped and sections reordered" | "disabled modules changed") {
                prop_assert_eq!(canonical_hash(&relabelled), canonical_hash(target), "{}", name);
            }
            for (t, _) in m.pairs() {
                let tm = target.find(t).unwrap().1;
                let rm = relabelled.find(t).unwrap().1;
                prop_assert_eq!(tm.kind, rm.kind, "{}", name);
            }
        }
    }
}

/// The round's matcher, as lane SS builds it from the scorer context.
fn matcher_for(round: &PreparedRound) -> Matcher {
    let ctx = enth_challenge::TargetContext {
        challenge: round.challenge(),
        target: round.target(),
        start: round.start(),
        phrase: round.phrase(),
        randomized: round.params(),
    };
    Matcher::for_round(&ctx)
}

/// The rings land on the player's controls: with the oscillators' ids swapped, a ring for the target's `osc1` goes
/// on the player's `osc2`.
#[test]
fn rings_follow_the_players_modules() {
    let spec = enth_challenge::SpecPreset::by_id(5).unwrap().spec(1, 3);
    // A level-5 target with osc 2 on.
    let (round, _) = (1..200)
        .map(|seed| {
            let mut s = spec;
            s.seed = seed;
            (round(&s), seed)
        })
        .find(|(r, _)| {
            r.target()
                .find(&ModuleId::new("osc2").unwrap())
                .unwrap()
                .1
                .enabled
        })
        .unwrap();
    let swapped = swap_ids(round.target(), "osc1", "osc2");
    let s = score(&round, &swapped);
    assert_eq!(s.percent, 100.0);
    let osc1_wave = s.params.iter().find(|p| p.addr == "osc1.wave").unwrap();
    assert_eq!(osc1_wave.answer_addr.as_deref(), Some("osc2.wave"));
    assert!(round.params().iter().any(|a| a.to_string() == "osc1.wave"));
}

/// Off-target answers get the rows the editor needs: the start of a level-3 round has bad rings, directions and
/// player addresses, and a removed filter is a missing module.
#[test]
fn the_start_and_a_missing_filter() {
    let spec = enth_challenge::SpecPreset::by_id(3).unwrap().spec(1, 11);
    let round = round(&spec);
    let start = score(&round, round.start());
    assert!(start.percent < 100.0);
    assert!(start.params.iter().any(|p| p.verdict != Verdict::Good));
    assert!(
        start
            .params
            .iter()
            .all(|p| p.direction != enth_challenge::Direction::Unknown)
    );
    let mut no_filter = round.target().clone();
    apply(
        &mut no_filter,
        &Op::SetEnabled {
            id: ModuleId::new("filter1").unwrap(),
            enabled: false,
        },
    );
    let s = score(&round, &no_filter);
    assert!(s.percent < 100.0);
    // The filter is missing, and so is env 2 when it only drove the filter (it now drives nothing heard).
    assert_eq!(s.modules[0].id, "filter1");
    assert_eq!(s.modules[0].kind, "svf");
    assert!(
        s.modules
            .iter()
            .all(|m| m.status == enth_challenge::score::ModuleStatus::Missing),
        "{:?}",
        s.modules
    );
    let cutoff = s
        .params
        .iter()
        .find(|p| p.addr == "filter1.cutoff")
        .unwrap();
    assert_eq!(cutoff.answer_addr, None);
    assert_eq!(cutoff.direction, enth_challenge::Direction::Missing);
}

/// For lane SS: an answer that equals the target up to ids and order, relabelled by the round's matcher, renders the
/// target's samples bit for bit (random start phases and noise follow module ids, and routes sum in the target's
/// order), where the answer as played does not.
#[test]
fn relabelled_equivalents_render_the_target_bit_for_bit() {
    use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
    let render_spec = enth_render::RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    let mut unrelabelled_differs = false;
    for (level, seed) in [(5, 1), (5, 2), (5, 7), (4, 3)] {
        let spec = enth_challenge::SpecPreset::by_id(level)
            .unwrap()
            .spec(1, seed);
        let round = round(&spec);
        let target = round.target();
        let mut answer = swap_ids(&swap_ids(target, "osc1", "osc2"), "lfo1", "lfo2");
        if !uses_sync(target) {
            answer.generators.reverse();
        }
        answer.modulators.reverse();
        answer.routes.reverse();
        let matcher = matcher_for(&round);
        let scored = matcher.relabel(&matcher.pair(&answer), &answer);
        let perf = round.phrase().performance();
        let a = enth_render::render(target, &perf, &render_spec).unwrap();
        let b = enth_render::render(&scored, &perf, &render_spec).unwrap();
        let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert!(
            bits(&a.left) == bits(&b.left) && bits(&a.right) == bits(&b.right),
            "level {level} seed {seed}"
        );
        let c = enth_render::render(&answer, &perf, &render_spec).unwrap();
        unrelabelled_differs |= bits(&a.left) != bits(&c.left);
    }
    assert!(unrelabelled_differs, "the cases exercise the relabelling");
}
