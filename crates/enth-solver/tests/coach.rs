//! The coach on real renders (`enth-render` + `enth-analysis` through `measure`): a known error is named with its
//! direction, the levels hold their values back, the plan is the same for any worker count, and the render budget
//! is kept.

mod common;

use common::{Recorder, Reversed, Threads, set};
use enth_challenge::model::SCORING_SEED;
use enth_challenge::phrases::{HELD, phrase};
use enth_challenge::random::start;
use enth_patch::{ModSource, ModuleId, Op, ParamAddr, Patch, Route, RouteAddr, apply};
use enth_render::{Performance, RenderSpec};
use enth_solver::coach::{
    Aspect, CoachCfg, CoachPlan, CoachTarget, Difference, HINT_LEVELS, Hint, Turn, Verdict,
};
use enth_solver::{Evaluator, Serial};

fn perf() -> Performance {
    phrase(HELD).unwrap().performance()
}

fn spec() -> RenderSpec {
    RenderSpec::new(48_000, SCORING_SEED)
}

fn addr(a: &str) -> ParamAddr {
    a.parse().unwrap()
}

fn starter() -> Patch {
    start(0).unwrap()
}

/// What a Recreate level 3 randomizes that matters here.
const RANDOMIZED: [&str; 7] = [
    "osc1.semi",
    "filter1.cutoff",
    "filter1.resonance",
    "env1.attack",
    "env1.decay",
    "env1.sustain",
    "env1.release",
];

fn randomized() -> Vec<ParamAddr> {
    RANDOMIZED.iter().map(|a| addr(a)).collect()
}

fn prepare(target: &Patch, eval: &mut dyn Evaluator) -> CoachTarget {
    CoachTarget::prepare(
        target,
        &randomized(),
        &perf(),
        &spec(),
        eval,
        &CoachCfg::default(),
    )
    .unwrap()
}

/// The text with every module id taken out, so that what is left has no digit unless it is a value.
fn without_ids(text: &str, patch: &Patch) -> String {
    let mut out = text.to_string();
    for (_, m) in patch.modules() {
        out = out.replace(m.id.as_str(), "");
    }
    out
}

fn assert_no_value_before_level_4(h: &[Hint], patch: &Patch) {
    for hint in &h[..3] {
        let bare = without_ids(&hint.text, patch);
        assert!(
            !bare.chars().any(|c| c.is_ascii_digit()),
            "level {} shows a number: {}",
            hint.level,
            hint.text
        );
        assert!(hint.range.is_none(), "level {} shows a range", hint.level);
    }
    assert!(h[0].module.is_none() && h[0].param.is_none() && h[0].turn.is_none());
    assert!(h[1].param.is_none() && h[1].turn.is_none());
}

/// One known error: the target is the starter with `param` changed; the player has the starter.
struct Case {
    param: &'static str,
    target_value: f32,
    aspect: Aspect,
}

const CASES: [Case; 5] = [
    Case {
        param: "osc1.semi",
        target_value: 7.0,
        aspect: Aspect::Pitch,
    },
    Case {
        param: "osc1.semi",
        target_value: -5.0,
        aspect: Aspect::Pitch,
    },
    Case {
        param: "filter1.cutoff",
        target_value: 500.0,
        aspect: Aspect::Brightness,
    },
    Case {
        param: "env1.attack",
        target_value: 400.0,
        aspect: Aspect::Attack,
    },
    Case {
        param: "env1.release",
        target_value: 1_200.0,
        aspect: Aspect::Release,
    },
];

#[test]
fn a_known_error_is_named_with_its_parameter_and_direction() {
    for case in &CASES {
        let player = starter();
        let mut target = starter();
        set(&mut target, case.param, case.target_value);
        let ct = prepare(&target, &mut Serial::new());
        let plan = ct
            .plan(&player, &mut Serial::new(), &CoachCfg::default())
            .unwrap();
        let name = format!("{} -> {}", case.param, case.target_value);
        assert_eq!(plan.verdict, Verdict::Measured, "{name}: {plan:#?}");
        let want = addr(case.param);
        let from = player.value(&want).unwrap();
        let turn = if case.target_value > from {
            Turn::Raise
        } else {
            Turn::Lower
        };
        let spec = player.spec(&want).unwrap();
        for level in 1..=HINT_LEVELS {
            let h = plan.hint(level);
            assert_eq!(h.level, level);
            assert_eq!(h.aspect, Some(case.aspect), "{name} L{level}: {}", h.text);
            assert!(h.measured, "{name} L{level}");
        }
        let (h2, h3, h4) = (plan.hint(2), plan.hint(3), plan.hint(4));
        assert_eq!(h2.module.as_ref(), want.module_id(), "{name}: {}", h2.text);
        assert_eq!(h3.param.as_ref(), Some(&want), "{name}: {}", h3.text);
        assert_eq!(h3.turn, Some(turn), "{name}: {}", h3.text);
        let expected_diff = if turn == Turn::Raise {
            Difference::Lower
        } else {
            Difference::Higher
        };
        let d = plan.hint(1).difference.unwrap();
        // Time and level aspects read the other way round from the knob (a longer attack is a slower one), so
        // only pitch and brightness fix the sign of the difference.
        if matches!(case.aspect, Aspect::Pitch | Aspect::Brightness) {
            assert_eq!(d, expected_diff, "{name}: {}", plan.hint(1).text);
        }
        let (lo, hi) = h4.range.expect("a level-4 range");
        assert!(
            lo <= case.target_value && case.target_value <= hi,
            "{name}: the range {lo}..{hi} misses the target's value; {}",
            h4.text
        );
        assert!(
            h4.text.contains(&spec.display(lo).to_string()),
            "{name}: {}",
            h4.text
        );
        assert!(
            hi - lo > 0.0 && hi - lo <= 0.3 * (spec.max() - spec.min()),
            "{name}: {lo}..{hi}"
        );
    }
}

#[test]
fn the_levels_hold_the_value_back() {
    for case in &CASES {
        let mut target = starter();
        set(&mut target, case.param, case.target_value);
        let ct = prepare(&target, &mut Serial::new());
        let plan = ct
            .plan(&starter(), &mut Serial::new(), &CoachCfg::default())
            .unwrap();
        assert_eq!(plan.hints.len(), usize::from(HINT_LEVELS));
        assert_no_value_before_level_4(&plan.hints, &target);
        // The target's value, in the forms a text could show it, is nowhere in levels 1 to 3.
        let spec = target.spec(&addr(case.param)).unwrap();
        for form in [
            spec.display(case.target_value).to_string(),
            spec.display_rounded(case.target_value, 2).to_string(),
            format!("{}", case.target_value),
        ] {
            for h in &plan.hints[..3] {
                assert!(
                    !h.text.contains(&form),
                    "L{}: {} shows {form}",
                    h.level,
                    h.text
                );
            }
        }
        // Level 4 gives a span, never the single value.
        let (lo, hi) = plan.hint(4).range.unwrap();
        assert!(lo < hi, "{lo}..{hi}");
    }
}

#[test]
fn a_waveform_difference_is_named_but_its_value_waits_for_level_4() {
    let mut target = starter();
    set(&mut target, "osc1.wave", 1.0); // triangle: the start is a saw
    let ct = CoachTarget::prepare(
        &target,
        &[addr("osc1.wave"), addr("osc1.semi")],
        &perf(),
        &spec(),
        &mut Serial::new(),
        &CoachCfg::default(),
    )
    .unwrap();
    let plan = ct
        .plan(&starter(), &mut Serial::new(), &CoachCfg::default())
        .unwrap();
    assert_eq!(plan.verdict, Verdict::Structure, "{plan:#?}");
    assert_no_value_before_level_4(&plan.hints, &target);
    let (h3, h4) = (plan.hint(3), plan.hint(4));
    assert_eq!(h3.param, Some(addr("osc1.wave")));
    assert!(!h3.measured && h3.turn.is_none() && h3.range.is_none());
    assert!(!h3.text.contains("triangle"), "{}", h3.text);
    assert!(h4.text.contains("triangle"), "{}", h4.text);
}

#[test]
fn a_missing_filter_sweep_points_at_the_route() {
    let mut target = starter();
    set(&mut target, "env2.attack", 600.0);
    set(&mut target, "env2.decay", 1_200.0);
    set(&mut target, "env2.sustain", 0.0);
    set(&mut target, "filter1.cutoff", 400.0);
    let route = Route {
        src: ModSource::Module(ModuleId::new("env2").unwrap()),
        dst: addr("filter1.cutoff"),
        depth: 60.0,
    };
    apply(
        &mut target,
        &Op::InsertRoute {
            index: 0,
            route: route.clone(),
        },
    )
    .unwrap();
    let mut player = starter();
    set(&mut player, "filter1.cutoff", 400.0);
    let ct = CoachTarget::prepare(
        &target,
        &[
            addr("filter1.cutoff"),
            addr("env2.decay"),
            addr("env2.attack"),
        ],
        &perf(),
        &spec(),
        &mut Serial::new(),
        &CoachCfg::default(),
    )
    .unwrap();
    let plan = ct
        .plan(&player, &mut Serial::new(), &CoachCfg::default())
        .unwrap();
    assert!(plan.failing.contains(&Aspect::Sweep), "{plan:#?}");
    assert_eq!(plan.verdict, Verdict::Structure, "{plan:#?}");
    assert_no_value_before_level_4(&plan.hints, &target);
    let (h2, h3, h4) = (plan.hint(2), plan.hint(3), plan.hint(4));
    assert!(h2.text.contains("modulation routes"), "{}", h2.text);
    assert_eq!(h3.param, Some(addr("filter1.cutoff")));
    assert!(
        !h3.text.contains("env2"),
        "the source waits for level 4: {}",
        h3.text
    );
    assert!(
        h4.text.contains("env2") && h4.text.contains('%'),
        "{}",
        h4.text
    );
    let _ = RouteAddr {
        src: route.src,
        dst: route.dst,
    };
}

#[test]
fn a_matching_patch_gets_one_render_and_the_same_words_at_every_level() {
    let mut target = starter();
    set(&mut target, "filter1.cutoff", 900.0);
    let ct = prepare(&target, &mut Serial::new());
    let mut rec = Recorder::new(Serial::new());
    let plan = ct.plan(&target, &mut rec, &CoachCfg::default()).unwrap();
    assert_eq!(plan.verdict, Verdict::Close);
    assert_eq!(plan.renders(), 1);
    assert_eq!(rec.jobs, 1);
    assert_eq!(plan.failing.len(), 0, "{:?}", plan.failing);
    assert!(plan.hints.iter().all(|h| h.text == plan.hints[0].text));
    assert!(
        plan.hints
            .iter()
            .all(|h| h.aspect.is_none() && h.range.is_none())
    );
}

#[test]
fn the_levels_climb_within_one_plan() {
    let mut target = starter();
    set(&mut target, "filter1.cutoff", 500.0);
    let ct = prepare(&target, &mut Serial::new());
    let plan = ct
        .plan(&starter(), &mut Serial::new(), &CoachCfg::default())
        .unwrap();
    let levels: Vec<u8> = plan.hints.iter().map(|h| h.level).collect();
    assert_eq!(levels, [1, 2, 3, 4]);
    // `hint(level)` clamps.
    assert_eq!(plan.hint(0).level, 1);
    assert_eq!(plan.hint(9).level, HINT_LEVELS);
}

fn digest(plan: &CoachPlan) -> Vec<(u8, String)> {
    plan.hints
        .iter()
        .map(|h| (h.level, h.text.clone()))
        .collect()
}

#[test]
fn the_worker_count_does_not_change_the_hints() {
    let mut target = starter();
    set(&mut target, "filter1.cutoff", 500.0);
    set(&mut target, "env1.attack", 300.0);
    let mut player = starter();
    set(&mut player, "osc1.semi", 3.0);
    let cfg = CoachCfg::default();
    let ct = prepare(&target, &mut Serial::new());
    let serial = ct.plan(&player, &mut Serial::new(), &cfg).unwrap();
    // The same twice, on 1 and 4 real threads, and with a batch evaluated back to front.
    assert_eq!(ct.plan(&player, &mut Serial::new(), &cfg).unwrap(), serial);
    for n in [1, 4, 8] {
        let t = prepare(&target, &mut Threads(n));
        assert_eq!(t.expects(), ct.expects(), "{n} workers: the list");
        assert_eq!(t.items(), ct.items(), "{n} workers: the items");
        assert_eq!(
            t.sensitivity(),
            ct.sensitivity(),
            "{n} workers: the sensitivity"
        );
        let p = t.plan(&player, &mut Threads(n), &cfg).unwrap();
        assert_eq!(p, serial, "{n} workers");
        assert_eq!(digest(&p), digest(&serial));
    }
    let rev = prepare(&target, &mut Reversed(Serial::new()));
    assert_eq!(rev.sensitivity(), ct.sensitivity());
    assert_eq!(
        rev.plan(&player, &mut Reversed(Serial::new()), &cfg)
            .unwrap(),
        serial
    );
}

#[test]
fn the_render_budget_is_kept() {
    let mut target = starter();
    set(&mut target, "filter1.cutoff", 500.0);
    set(&mut target, "env1.attack", 300.0);
    let mut player = starter();
    set(&mut player, "osc1.semi", 3.0);
    for (prepare_budget, plan_budget) in [(1, 1), (2, 2), (5, 3), (48, 8), (48, 24)] {
        let mut cfg = CoachCfg::default();
        cfg.sensitivity.max_renders = prepare_budget;
        cfg.hint.max_renders = plan_budget;
        let mut rec = Recorder::new(Serial::new());
        let ct =
            CoachTarget::prepare(&target, &randomized(), &perf(), &spec(), &mut rec, &cfg).unwrap();
        assert!(
            rec.jobs <= prepare_budget,
            "prepare {prepare_budget}: {} jobs",
            rec.jobs
        );
        assert_eq!(ct.sensitivity().renders, rec.jobs);
        let mut rec = Recorder::new(Serial::new());
        let plan = ct.plan(&player, &mut rec, &cfg).unwrap();
        assert!(
            rec.jobs <= plan_budget,
            "plan {plan_budget}: {} jobs",
            rec.jobs
        );
        assert_eq!(plan.renders(), rec.jobs);
        // Whatever the budget, there are four hints and the first names an aspect.
        assert_eq!(plan.hints.len(), usize::from(HINT_LEVELS));
        assert!(plan.hint(1).aspect.is_some());
    }
}

#[test]
fn the_one_shot_costs_at_most_the_two_budgets() {
    use enth_challenge::{RandomSpec, prepare_round};
    use enth_solver::coach::coach_hint;
    let rs = RandomSpec {
        generator: 1,
        seed: 42,
        difficulty: 50,
        amounts: [None; 6],
        musical: 100,
        template: 0,
    };
    let round = prepare_round(&rs).unwrap();
    let cfg = CoachCfg::default();
    let mut rec = Recorder::new(Serial::new());
    let h = coach_hint(&round, round.start(), 3, &mut rec, &cfg).unwrap();
    assert_eq!(h.level, 3);
    assert!(
        rec.jobs <= cfg.sensitivity.max_renders + cfg.hint.max_renders,
        "{} jobs",
        rec.jobs
    );
    // The round's target itself is close.
    let ct = CoachTarget::for_round(&round, &mut Serial::new(), &cfg).unwrap();
    let plan = ct.plan(round.target(), &mut Serial::new(), &cfg).unwrap();
    assert_eq!(plan.verdict, Verdict::Close);
}

#[test]
fn the_coach_list_is_the_targets_own_descriptors() {
    let mut target = starter();
    set(&mut target, "filter1.cutoff", 900.0);
    let ct = prepare(&target, &mut Serial::new());
    let aspects: Vec<Aspect> = ct.items().iter().map(|i| i.aspect).collect();
    for a in [
        Aspect::Pitch,
        Aspect::Brightness,
        Aspect::Attack,
        Aspect::Sustain,
        Aspect::Release,
    ] {
        assert!(aspects.contains(&a), "{a:?} in {aspects:?}");
    }
    // The starter has no movement, so beating, tremolo and sweep are `== null`.
    for (item, e) in ct.items().iter().zip(ct.expects()) {
        assert_eq!(item.feature, e.feature);
        if matches!(
            item.aspect,
            Aspect::Beating | Aspect::Tremolo | Aspect::Sweep
        ) {
            assert!(
                item.target.is_none() && e.canonical.ends_with("== null"),
                "{}",
                e.canonical
            );
        } else {
            assert!(
                item.target.is_some() && item.tolerance > 0.0,
                "{}",
                e.canonical
            );
        }
    }
    // The target passes its own list.
    let mut rec = Recorder::new(Serial::new());
    let plan = ct.plan(&target, &mut rec, &CoachCfg::default()).unwrap();
    assert!(
        plan.result.before.iter().all(|r| r.pass),
        "{:#?}",
        plan.result.before
    );
}

#[test]
fn real_rounds_never_show_a_target_value_before_level_4() {
    use enth_challenge::{RandomSpec, prepare_round};
    use enth_params::Kind;
    let cfg = CoachCfg::default();
    for (seed, difficulty) in [(42, 0), (7, 25), (42, 50), (3, 75), (11, 100)] {
        let rs = RandomSpec {
            generator: 1,
            seed,
            difficulty,
            amounts: [None; 6],
            musical: 100,
            template: 0,
        };
        let round = prepare_round(&rs).unwrap();
        let name = format!("seed {seed} difficulty {difficulty}");
        let ct = CoachTarget::for_round(&round, &mut Serial::new(), &cfg).unwrap();
        let plan = ct.plan(round.start(), &mut Serial::new(), &cfg).unwrap();
        assert_eq!(plan.hints.len(), usize::from(HINT_LEVELS), "{name}");
        assert!(plan.renders() <= cfg.hint.max_renders, "{name}");
        assert!(
            plan.hint(1).aspect.is_some(),
            "{name}: the start differs from the target"
        );
        assert_no_value_before_level_4(&plan.hints, round.target());
        // No randomized parameter's value, shown as the editor shows it, is in levels 1 to 3.
        for a in round.params() {
            let (Some(sp), Some(v)) = (round.target().spec(a), round.target().value(a)) else {
                continue;
            };
            let forms = match sp.kind {
                Kind::Float { .. } => vec![
                    sp.display(v).to_string(),
                    sp.display_rounded(v, 3).to_string(),
                ],
                _ => vec![sp.display(v).to_string()],
            };
            for form in forms.iter().filter(|f| f.len() >= 4) {
                for h in &plan.hints[..3] {
                    assert!(
                        !h.text.contains(form.as_str()),
                        "{name} L{}: {} shows {form} ({a})",
                        h.level,
                        h.text
                    );
                }
            }
        }
        // And the same words the second time.
        let again = ct.plan(round.start(), &mut Threads(4), &cfg).unwrap();
        assert_eq!(again, plan, "{name}");
    }
}
