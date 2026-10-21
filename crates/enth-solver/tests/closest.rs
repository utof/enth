//! Lane CLS: `closest` against the real scorer of a generated round (`docs/dsp/match.md`, "Closest"). A result is a
//! pure function of its inputs and settings, so it is identical for any number of workers (the phase-3 acceptance:
//! "1 and 8 workers give the same result"), and it finds a passing patch nearer to the player's than the target's own
//! settings, moving only the free parameters.

#[path = "../examples/support/round_pool.rs"]
mod round_pool;

use enth_challenge::spec::PRESETS;
use enth_challenge::{PreparedRound, ScoreReport, prepare_round, score_round};
use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_solver::closest::{
    Closest, ClosestCfg, ClosestEval, ClosestJob, ClosestStop, Delta, RoundSerial, closest,
    closest_with, reading,
};

use round_pool::RoundPool;

fn addr(s: &str) -> ParamAddr {
    s.parse().unwrap()
}

fn set(p: &mut Patch, a: &ParamAddr, value: f32) {
    apply(
        p,
        Op::SetParam {
            addr: a.clone(),
            value,
        },
    )
    .unwrap();
}

/// A lesson-like step on a generator v1 round: the target patch with the `free` parameters back at the template's
/// values (the step as it opens: everything else is set, and only these are editable).
struct Step {
    round: PreparedRound,
    free: Vec<ParamAddr>,
    player: Patch,
}

fn step(level: usize, seed: u64, free: &[&str]) -> Step {
    let spec = PRESETS[level - 1].spec(1, 0x434C_0000 + seed);
    let round = prepare_round(&spec).expect("preset rounds prepare");
    let free: Vec<ParamAddr> = free.iter().map(|s| addr(s)).collect();
    let mut player = round.target().clone();
    for a in &free {
        set(
            &mut player,
            a,
            round.start().value(a).expect("start has it"),
        );
    }
    Step {
        round,
        free,
        player,
    }
}

impl Step {
    fn job(&self, budget: usize) -> ClosestJob<'_> {
        ClosestJob {
            round: &self.round,
            patch: &self.player,
            free: &self.free,
            budget,
        }
    }

    fn run(&self, eval: &mut dyn ClosestEval, budget: usize, cfg: &ClosestCfg) -> Closest {
        closest(&self.job(budget), eval, cfg).expect("the job runs")
    }

    /// The knob distance from the player's patch to the target's own settings on the free axes.
    fn target_distance(&self) -> f64 {
        let mut sum = 0.0;
        for a in &self.free {
            let spec = self.player.spec(a).unwrap();
            let d = f64::from(spec.to_norm(self.round.target().value(a).unwrap()))
                - f64::from(spec.to_norm(self.player.value(a).unwrap()));
            sum += d * d;
        }
        sum.sqrt()
    }

    fn report(&self, patch: &Patch) -> ScoreReport {
        score_round(&self.round, patch).expect("scores")
    }
}

/// Level 2, seed 1: the cutoff and the resonance, a start that fails (17.6 %) and a target that passes.
fn cutoff_and_resonance() -> Step {
    step(2, 1, &["filter1.cutoff", "filter1.resonance"])
}

#[test]
fn the_result_is_identical_for_any_worker_count() {
    let s = cutoff_and_resonance();
    let cfg = ClosestCfg::default();
    let serial = s.run(&mut RoundSerial::new(), 150, &cfg);
    assert!(serial.passed, "{serial:?}");
    assert!(serial.renders <= 150 && serial.renders > 1);
    assert!(serial.first_pass_renders.is_some());
    for workers in [1, 2, 3, 8] {
        let pooled = s.run(&mut RoundPool::new(workers), 150, &cfg);
        assert_eq!(pooled, serial, "{workers} workers");
    }
    // The workers' schedule is no input: the same search again is the same.
    assert_eq!(s.run(&mut RoundPool::new(8), 150, &cfg), serial);
}

#[test]
fn the_population_is_a_setting_and_results_follow_it_not_the_pool() {
    let s = cutoff_and_resonance();
    let cfg = ClosestCfg {
        lambda: 8,
        ..ClosestCfg::default()
    };
    let one = s.run(&mut RoundPool::new(1), 100, &cfg);
    let eight = s.run(&mut RoundPool::new(8), 100, &cfg);
    assert_eq!(one, eight);
    // A different population is a different search.
    let other = s.run(
        &mut RoundPool::new(8),
        100,
        &ClosestCfg {
            lambda: 12,
            ..cfg.clone()
        },
    );
    assert_ne!(other, one);
}

#[test]
fn the_nearest_passing_patch_passes_and_is_nearer_than_the_target() {
    let s = cutoff_and_resonance();
    let found = s.run(&mut RoundPool::new(4), 200, &ClosestCfg::default());
    assert!(found.passed);
    assert_eq!(found.stop, ClosestStop::Converged);
    assert!(found.start_percent < 90.0, "the step opens failing");
    // The round's own scorer agrees with the numbers returned.
    let report = s.report(&found.patch);
    assert!(report.passed);
    assert_eq!(report.sound.percent, found.percent);
    // It is nearer than the target's own settings, which pass too (and the answer is not the player's own patch).
    assert!(found.distance > 0.0);
    assert!(
        found.distance <= s.target_distance(),
        "{} against the target's {}",
        found.distance,
        s.target_distance()
    );
    // Only the free parameters moved, and the deltas say how.
    let mut expected = s.player.clone();
    for d in &found.deltas {
        assert!(s.free.contains(&d.addr));
        assert_eq!(d.from, s.player.value(&d.addr).unwrap());
        set(&mut expected, &d.addr, d.to);
    }
    assert_eq!(found.patch, expected);
    assert_eq!(found.deltas.len(), s.free.len());
    assert!(found.deltas.iter().any(Delta::moved));
    // The cutoff is the parameter that needed to move: from the open template value towards the target's.
    let cutoff = found
        .deltas
        .iter()
        .find(|d| d.addr == addr("filter1.cutoff"))
        .unwrap();
    let target = s.round.target().value(&cutoff.addr).unwrap();
    assert!((cutoff.to - target).abs() < (cutoff.from - target).abs());
}

#[test]
fn a_start_that_passes_costs_one_render_and_is_the_answer() {
    // The target itself passes: a lesson step whose player is already there.
    let s = cutoff_and_resonance();
    let player = s.round.target().clone();
    let job = ClosestJob {
        round: &s.round,
        patch: &player,
        free: &s.free,
        budget: 100,
    };
    let found = closest(&job, &mut RoundPool::new(4), &ClosestCfg::default()).unwrap();
    assert_eq!(found.stop, ClosestStop::StartPasses);
    assert!(found.passed);
    assert_eq!(found.renders, 1);
    assert_eq!(found.patch, player);
    assert_eq!(found.distance, 0.0);
    assert!(found.deltas.iter().all(|d| !d.moved()));
    assert_eq!(found.percent, found.start_percent);
}

#[test]
fn the_budget_is_hard_and_a_callback_cancels() {
    let s = cutoff_and_resonance();
    let cfg = ClosestCfg::default();
    for budget in [1, 3, 10, 25] {
        let found = s.run(&mut RoundPool::new(3), budget, &cfg);
        assert!(
            found.renders <= budget,
            "budget {budget}: {}",
            found.renders
        );
    }
    let mut seen = Vec::new();
    let found = closest_with(&s.job(200), &mut RoundPool::new(2), &cfg, &mut |p| {
        seen.push(p.renders);
        seen.len() < 2
    })
    .unwrap();
    assert_eq!(found.stop, ClosestStop::Cancelled);
    assert_eq!(seen.len(), 2);
    assert_eq!(found.renders, seen[1]);
    // What it returns is a scored candidate, never worse than the player's own patch.
    assert!(found.percent >= found.start_percent);
}

#[test]
fn a_round_the_free_set_cannot_pass_returns_the_nearest_to_passing() {
    // Only the resonance is free, but the target also differs in the cutoff: no pass exists in the free set.
    let s = step(2, 1, &["filter1.cutoff", "filter1.resonance"]);
    let narrow = [addr("filter1.resonance")];
    let job = ClosestJob {
        round: &s.round,
        patch: &s.player,
        free: &narrow,
        budget: 30,
    };
    let found = closest(&job, &mut RoundPool::new(4), &ClosestCfg::default()).unwrap();
    assert!(!found.passed);
    assert_eq!(found.stop, ClosestStop::Budget);
    assert_eq!(found.renders, 30, "the budget is used, and not exceeded");
    assert!(found.percent >= found.start_percent);
    assert!(!s.report(&found.patch).passed);
}

#[test]
fn the_reading_is_the_rounds_own_verdict() {
    let s = cutoff_and_resonance();
    for patch in [&s.player, s.round.target()] {
        let r = reading(&s.round, patch).unwrap();
        let report = s.report(patch);
        assert_eq!(r.passed, report.passed);
        assert_eq!(r.percent, report.sound.percent);
        assert_eq!(r.jnd, report.sound.jnd);
    }
}

#[test]
fn a_waveform_among_the_free_parameters_is_searched_too() {
    // Level 2 also randomizes the waveform. The player has the wrong one and the cutoff open: both are free.
    let spec = PRESETS[1].spec(1, 0x434C_0000 + 1);
    let round = prepare_round(&spec).unwrap();
    let wave = addr("osc1.wave");
    let free = vec![wave.clone(), addr("filter1.cutoff")];
    let mut player = round.target().clone();
    set(
        &mut player,
        &free[1],
        round.start().value(&free[1]).unwrap(),
    );
    // A different waveform than the target's.
    let target_wave = round.target().value(&wave).unwrap();
    set(
        &mut player,
        &wave,
        if target_wave == 0.0 { 1.0 } else { 0.0 },
    );
    assert!(!reading(&round, &player).unwrap().passed);
    let job = ClosestJob {
        round: &round,
        patch: &player,
        free: &free,
        budget: 250,
    };
    let one = closest(&job, &mut RoundSerial::new(), &ClosestCfg::default()).unwrap();
    let eight = closest(&job, &mut RoundPool::new(8), &ClosestCfg::default()).unwrap();
    assert_eq!(one, eight);
    assert!(one.passed, "{one:?}");
    assert!(one.deltas[0].categorical);
    assert!(one.deltas[0].moved(), "the wrong waveform is changed");
}
