//! Lane MT: the match search against the real metric. A search is a pure function of its inputs and seed, so it is
//! identical for any number of workers (`docs/phase-2.md`: "identical results for 1 and 8 workers"), and it finds
//! the randomizer's targets from a heuristic start.

#[path = "../examples/support/pool.rs"]
mod pool;

use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::random;
use enth_challenge::spec::PRESETS;
use enth_patch::Patch;
use enth_render::{Performance, RenderSpec};
use enth_solver::heuristic::Heuristic;
use enth_solver::search::{
    FreeSet, MetricTarget, SearchCfg, SearchResult, StopCause, search, search_metric,
};

/// The randomizer's target for `(preset index, seed offset)`: its patch, the start patch of its template, and the
/// parameters it drew.
fn drawn(preset: usize, i: u64) -> (random::Generated, Patch, FreeSet) {
    let rspec = PRESETS[preset].spec(1, 0x4D54_0000 + i);
    let generated = random::target(&rspec).expect("preset specs generate");
    let start = random::start(rspec.template).expect("template 0 exists");
    let free =
        FreeSet::new(&start, &generated.params).expect("drawn parameters exist in the start");
    (generated, start, free)
}

/// One note, half a second held: enough for the descriptors, cheap to render a few hundred times unoptimised.
fn short_note() -> Performance {
    Performance::note(48, 0.8, 0.3, 0.5)
}

fn run(
    start: &Patch,
    free: &FreeSet,
    target: &MetricTarget,
    cfg: &SearchCfg,
    workers: usize,
) -> SearchResult {
    let heuristic = target
        .descriptors()
        .map(|d| Heuristic::new(d, target.performance()));
    let mut objective = pool::Pool::new(target, workers);
    search(start, free, heuristic.as_ref(), &mut objective, cfg).expect("search runs")
}

#[test]
fn the_result_is_identical_for_any_worker_count() {
    // A short note keeps the renders cheap in a debug build; level 2 has the categorical and the continuous axes both.
    let (generated, start, free) = drawn(1, 3);
    let perf = short_note();
    let target = MetricTarget::from_patch(&generated.patch, &perf, &RenderSpec::default())
        .expect("generated targets render");
    assert!(
        target.descriptors().is_some(),
        "the heuristic path is part of what is compared"
    );
    let cfg = SearchCfg {
        seed: 9,
        budget: 60,
        ..SearchCfg::default()
    };

    let serial = search_metric(&start, &free, &target, &cfg).expect("search runs");
    assert!(serial.renders <= cfg.budget);
    assert!(
        serial.heuristic_distance.is_some(),
        "the heuristic changed the start"
    );
    assert!(serial.runs >= 1, "the budget reaches the CMA-ES runs");
    assert!(serial.distance < serial.start_distance);

    for workers in [1, 2, 3, 8] {
        let pooled = run(&start, &free, &target, &cfg, workers);
        assert_eq!(pooled, serial, "{workers} workers");
    }
    // The workers' schedule is no input: the same search again is the same.
    assert_eq!(run(&start, &free, &target, &cfg, 8), serial);
}

#[test]
fn the_population_is_a_setting_and_results_follow_it_not_the_pool() {
    let (generated, start, free) = drawn(1, 3);
    let perf = short_note();
    let target = MetricTarget::from_patch(&generated.patch, &perf, &RenderSpec::default())
        .expect("generated targets render");
    let cfg = SearchCfg {
        seed: 2,
        budget: 40,
        lambda: 8,
        ..SearchCfg::default()
    };
    let one = run(&start, &free, &target, &cfg, 1);
    let eight = run(&start, &free, &target, &cfg, 8);
    assert_eq!(one, eight);
    // A different population is a different search.
    let other = run(
        &start,
        &free,
        &target,
        &SearchCfg {
            lambda: 12,
            ..cfg.clone()
        },
        8,
    );
    assert_ne!(other.trace, one.trace);
}

#[test]
fn the_search_improves_on_the_start_and_the_trace_only_improves() {
    let (generated, start, free) = drawn(1, 3);
    let target = MetricTarget::from_patch(&generated.patch, &short_note(), &RenderSpec::default())
        .expect("renders");
    let cfg = SearchCfg {
        budget: 200,
        stop_at: Some(0.02),
        ..SearchCfg::default()
    };
    let found = run(&start, &free, &target, &cfg, 4);
    assert_eq!(found.stop, StopCause::Goal, "{found:?}");
    assert!(found.distance <= 0.02);
    assert!(
        found.start_distance > 1.0,
        "the init patch is far from the target"
    );
    assert!(found.renders <= 200);
    let bests: Vec<f64> = found.trace.iter().map(|t| t.best).collect();
    assert!(bests.windows(2).all(|w| w[1] <= w[0]), "{bests:?}");
    assert_eq!(found.trace.last().map(|t| t.renders), Some(found.renders));
    // The best patch really is that far from the target.
    let mut worker = target.worker();
    assert_eq!(worker.distance(&found.best), found.distance);
}

/// Level 2, seed 0x4D540000: the pitch is a needle in the metric (a tenth of a semitone off costs as much as a wrong
/// waveform) and the descriptors measure it to a cent. The search keeps the measured pitch while CMA-ES works on
/// the rest, and releases it only if that fails (`docs/dsp/match.md`). Before that, this target stayed at 0.0686
/// after 1,500 renders. The synthetic version of this runs with the unit tests (`search::tests`); this one renders
/// 574 scoring-length clips, about 3 s in a release build and minutes in a debug one:
/// `cargo test --release -p enth-solver --test match -- --ignored`.
#[test]
#[ignore = "renders about 600 scoring-length clips: minutes in a debug build; run with --release -- --ignored"]
fn a_pitch_needle_is_found_because_the_measured_pitch_is_kept() {
    let (generated, start, free) = drawn(1, 0);
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    let perf = generated.phrase().performance();
    let target = MetricTarget::from_patch(&generated.patch, &perf, &spec).expect("renders");
    let cfg = SearchCfg {
        budget: 1500,
        stop_at: Some(0.005),
        ..SearchCfg::default()
    };
    let found = run(&start, &free, &target, &cfg, 4);
    assert_eq!(found.stop, StopCause::Goal, "{found:?}");
    assert!(found.distance <= 0.005, "{}", found.distance);
    let semi = |p: &Patch| p.value(&"osc1.semi".parse().unwrap()).unwrap();
    assert!(
        (semi(&found.best) - semi(&generated.patch)).abs() < 0.01,
        "found {} for {}",
        semi(&found.best),
        semi(&generated.patch)
    );
}
