//! Sensitivity on real renders: the probes move the descriptors the way the catalogue says, the result is the same
//! for any worker count, and the render budget is kept.

mod common;

use common::{Recorder, Reversed, Short, Threads, set};
use enth_analysis::Feature;
use enth_challenge::model::SCORING_SEED;
use enth_challenge::phrases::{HELD, phrase};
use enth_challenge::random::start;
use enth_patch::{ParamAddr, Patch};
use enth_render::{Performance, RenderSpec};
use enth_solver::sensitivity::{
    Change, FEATURES, INERT_BELOW, Sensitivity, SensitivityCfg, SensitivityError, sensitivity,
    tolerance_bounds,
};
use enth_solver::{Evaluator, Serial};

fn perf() -> Performance {
    phrase(HELD).unwrap().performance()
}

fn spec() -> RenderSpec {
    RenderSpec::new(48_000, SCORING_SEED)
}

fn addrs(list: &[&str]) -> Vec<ParamAddr> {
    list.iter().map(|a| a.parse().unwrap()).collect()
}

fn starter() -> Patch {
    start(0).unwrap()
}

const PARAMS: [&str; 8] = [
    "osc1.semi",
    "osc1.wave",
    "filter1.cutoff",
    "filter1.resonance",
    "env1.attack",
    "env1.sustain",
    "env1.release",
    "env2.decay",
];

fn run(eval: &mut dyn Evaluator, cfg: &SensitivityCfg) -> Sensitivity {
    sensitivity(&starter(), &addrs(&PARAMS), &perf(), &spec(), eval, cfg).unwrap()
}

#[test]
fn a_probe_moves_the_descriptor_the_catalogue_says() {
    let s = run(&mut Serial::new(), &SensitivityCfg::default());
    let p = |a: &str| s.param(&a.parse().unwrap()).unwrap();

    // One semitone is 100 cents of pitch, in the direction of the probe.
    let semi = p("osc1.semi");
    let cents = semi.delta(Feature::Pitch).unwrap();
    assert!((libm::fabs(cents) - 100.0).abs() < 2.0, "{cents}");
    assert_eq!(cents.signum(), semi.step.signum());
    assert!(semi.reach() >= 1.0 && !semi.inert());

    // The cutoff sits above the middle of its log axis, so the probe goes down: darker.
    let cutoff = p("filter1.cutoff");
    assert!(cutoff.step < 0.0);
    assert!(cutoff.delta(Feature::Brightness).unwrap() < 0.0);
    assert!(cutoff.slope(Feature::Brightness).unwrap() > 0.0);

    // The attack probe goes up (100 ms is near the bottom of its log axis): a slower attack, and nothing else moves
    // much. (Under about 10 ms the attack descriptor does not resolve a step: 5 ms and 8 ms both read 5.)
    let mut slow = starter();
    set(&mut slow, "env1.attack", 100.0);
    let a = sensitivity(
        &slow,
        &addrs(&["env1.attack"]),
        &perf(),
        &spec(),
        &mut Serial::new(),
        &SensitivityCfg::default(),
    )
    .unwrap();
    let attack = &a.params[0];
    assert!(attack.step > 0.0 && attack.to > attack.from);
    assert!(attack.delta(Feature::Attack).unwrap() > 0.2);
    assert_eq!(attack.delta(Feature::Pitch), Some(0.0));

    // The release probe moves the release time only.
    let release = p("env1.release");
    assert!(release.delta(Feature::Release).unwrap().abs() > 0.1);
    assert_eq!(release.delta(Feature::Attack), Some(0.0));

    // The sustain level moves the sustain descriptor, in dB.
    let sustain = p("env1.sustain");
    assert!(sustain.delta(Feature::Sustain).unwrap().abs() > 0.5);

    // Descriptors the starter does not have are absent in both renders.
    assert_eq!(
        cutoff.effect(Feature::Beating).unwrap().change,
        Change::Absent
    );
}

#[test]
fn a_parameter_that_drives_nothing_is_inert() {
    let s = run(&mut Serial::new(), &SensitivityCfg::default());
    // env2 is routed nowhere in the starter.
    let env2 = s.param(&"env2.decay".parse().unwrap()).unwrap();
    assert!(
        env2.inert() && env2.reach() < INERT_BELOW,
        "{}",
        env2.reach()
    );
    let cutoff = s.param(&"filter1.cutoff".parse().unwrap()).unwrap();
    assert!(!cutoff.inert());
    // A live parameter has a normalised tolerance within the range; a dead one has none.
    let tol = cutoff.tolerance_norm().unwrap();
    assert!(tol > 0.0 && tol <= 1.0, "{tol}");
}

#[test]
fn discrete_parameters_and_unknown_addresses_are_listed_apart() {
    let mut list = addrs(&PARAMS);
    list.push("osc2.fine".parse().unwrap()); // the starter has an osc2 (disabled): continuous
    list.push("osc9.fine".parse().unwrap()); // there is no osc9
    let s = sensitivity(
        &starter(),
        &list,
        &perf(),
        &spec(),
        &mut Serial::new(),
        &SensitivityCfg::default(),
    )
    .unwrap();
    let names = |v: &[ParamAddr]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(names(&s.discrete), ["osc1.wave", "osc9.fine"]);
    assert!(s.param(&"osc2.fine".parse().unwrap()).is_some());
    assert_eq!(s.unprobed.len(), 0, "{:?}", s.unprobed);
    // Each parameter once, even when listed twice.
    let twice = [list.clone(), list].concat();
    let again = sensitivity(
        &starter(),
        &twice,
        &perf(),
        &spec(),
        &mut Serial::new(),
        &SensitivityCfg::default(),
    )
    .unwrap();
    assert_eq!(again, s);
}

#[test]
fn tolerances_stay_within_their_bounds() {
    let s = run(&mut Serial::new(), &SensitivityCfg::default());
    for f in FEATURES {
        let (lo, hi) = tolerance_bounds(f);
        let t = s.tolerance(f);
        assert!(lo <= t && t <= hi, "{f:?}: {t} outside {lo}..{hi}");
    }
    // Pitch: the semitone probe alone is far past 25 cents, so the tolerance is capped.
    assert_eq!(s.tolerance(Feature::Pitch), 25.0);
    // Something nothing moves sits at its floor.
    assert_eq!(
        s.tolerance(Feature::Beating),
        tolerance_bounds(Feature::Beating).0
    );
}

#[test]
fn the_result_is_pure_and_the_same_for_any_worker_count() {
    let cfg = SensitivityCfg::default();
    let serial = run(&mut Serial::new(), &cfg);
    assert_eq!(run(&mut Serial::new(), &cfg), serial, "twice");
    for n in [1, 4, 8] {
        assert_eq!(run(&mut Threads(n), &cfg), serial, "{n} workers");
    }
    assert_eq!(run(&mut Reversed(Serial::new()), &cfg), serial, "reversed");
}

#[test]
fn the_render_budget_is_kept_and_tagged_parameters_go_first() {
    for budget in [1, 2, 4, 9, 48] {
        let cfg = SensitivityCfg {
            max_renders: budget,
            ..SensitivityCfg::default()
        };
        let mut rec = Recorder::new(Serial::new());
        let s = run(&mut rec, &cfg);
        assert!(rec.jobs <= budget, "budget {budget}: {} jobs", rec.jobs);
        assert_eq!(s.renders, rec.jobs);
        assert_eq!(rec.batches, 1, "one batch");
        // Every continuous parameter is either probed or listed as left out.
        assert_eq!(
            s.params.len() + s.unprobed.len(),
            PARAMS.len() - 1,
            "{budget}"
        );
        assert_eq!(
            s.params.len(),
            (budget - 1).min(PARAMS.len() - 1),
            "{budget}"
        );
    }
    // With room for three probes, the three probed carry a `perceptual` tag.
    let cfg = SensitivityCfg {
        max_renders: 4,
        ..SensitivityCfg::default()
    };
    let s = run(&mut Serial::new(), &cfg);
    assert!(
        s.params.iter().all(|p| p.spec.perceptual.is_some()),
        "{:?}",
        s.params
            .iter()
            .map(|p| p.addr.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn bad_inputs_are_errors_not_panics() {
    let p = starter();
    let list = addrs(&["filter1.cutoff"]);
    let cfg = SensitivityCfg::default();
    let none = SensitivityCfg {
        max_renders: 0,
        ..cfg.clone()
    };
    let e = sensitivity(&p, &list, &perf(), &spec(), &mut Serial::new(), &none).unwrap_err();
    assert_eq!(e, SensitivityError::NoBudget);
    let silent = Performance::default();
    let e = sensitivity(&p, &list, &silent, &spec(), &mut Serial::new(), &cfg).unwrap_err();
    assert_eq!(e.code(), "no_notes");
    let e = sensitivity(&p, &list, &perf(), &spec(), &mut Short, &cfg).unwrap_err();
    assert_eq!(e.code(), "evaluator_contract");
    // A render that cannot be measured is reported as such.
    let bad = RenderSpec::new(384_000, 1);
    let e = sensitivity(&p, &list, &perf(), &bad, &mut Serial::new(), &cfg).unwrap_err();
    assert_eq!(e.code(), "bad_sample_rate");
    // A patch with its cutoff at the top of the range still gets a probe, downward.
    let mut high = starter();
    set(&mut high, "filter1.cutoff", 20_000.0);
    let s = sensitivity(&high, &list, &perf(), &spec(), &mut Serial::new(), &cfg).unwrap();
    assert!(s.params[0].to < s.params[0].from);
}
