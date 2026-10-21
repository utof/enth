//! Lane F1S: the F1s judge's M1 and M2 as regressions (the judge's scratch probes `probe_record_after_set_budget` and
//! `probe_render_equivalent_twin`, `~/code/enth-scratch/r23/f1s-judge/probe/judge_probe.rs`).
//!
//! - **M2:** successive halving's rungs sit at each branch's own render milestones, so the budget is only a hard cap:
//!   a run capped by its budget is an exact prefix of a longer one, and the render-equivalent twin of a run stopped
//!   by the clock (the progress callback) is the same job with its renders as the budget. The judge measured a run
//!   capped after 12 rounds (342 renders) against a 342-render budget that ran 18 rounds with other pruning.
//! - **M1:** a record stores the settings the job ran under (from the result), and replays it: the judge measured a
//!   record that replayed 4,987 renders in 235 rounds for a run of 696 in 41.

mod create_support;

use create_support::{cfg, init, inputs, proxy, target, templates};
use enth_patch::Patch;
use enth_solver::create::record::RecordContext;
use enth_solver::create::{
    CreateCfg, CreateJob, CreateRecord, CreateResult, CreateStop, Portfolio, Target, run,
};

/// Run a job over the support's starts, stopping after `stop_after` rounds when given; every patch the objective
/// evaluated, in order, comes back with the result.
fn logged(t: &Target, cfg: &CreateCfg, stop_after: Option<usize>) -> (CreateResult, Vec<Patch>) {
    let init = init();
    let tpl = templates();
    let inputs = inputs(&init, &tpl);
    let a = t.analyse().unwrap();
    let mut job = CreateJob::new(t, &a, Portfolio::build(t, &a, &inputs, cfg), cfg).unwrap();
    let mut log = Vec::new();
    {
        let mut objective = |p: &Patch| {
            log.push(p.clone());
            proxy(p)
        };
        let mut rounds = 0;
        run(&mut job, &mut objective, &mut |_| {
            rounds += 1;
            stop_after.is_none_or(|n| rounds < n)
        })
        .unwrap();
    }
    (job.result(), log)
}

#[test]
fn a_run_capped_by_its_budget_is_an_exact_prefix_of_a_longer_one() {
    let t = target();
    let (long, long_log) = logged(&t, &cfg(3_000), None);
    assert_eq!(long.stop, CreateStop::Budget);
    assert_eq!(long.renders, long_log.len());
    assert!(long.renders > 2_900, "{} renders", long.renders);
    // Each branch's search begins with its start, whose screening distance is reused (should-fix 14): every start is
    // rendered once.
    for s in &long.starts {
        let times = long_log.iter().filter(|p| **p == s.patch).count();
        assert_eq!(times, 1, "{:?} rendered {times} times", s.provenance);
    }
    // From inside the screening round (2 of its 4 starts) to most of the long run.
    for budget in [2, 4, 5, 37, 101, 342, 1_000, 2_222] {
        let (short, log) = logged(&t, &cfg(budget), None);
        assert!(short.renders <= budget, "budget {budget}");
        assert_eq!(short.renders, log.len());
        assert!(
            log[..] == long_log[..log.len()],
            "budget {budget}: the evaluated patches are the longer run's first ones"
        );
        let n = short.rounds.len();
        assert_eq!(
            short.rounds[..n - 1],
            long.rounds[..n - 1],
            "budget {budget}"
        );
        // The last round is the longer run's same round, cut to the whole batches that fit.
        assert!(short.rounds[n - 1].renders <= long.rounds[n - 1].renders);
        assert!(short.rounds[n - 1].live <= long.rounds[n - 1].live);
    }
}

#[test]
fn the_twin_of_a_run_stopped_by_the_clock_is_the_same_job_with_its_renders_as_the_budget() {
    let t = target();
    for rounds in [1, 2, 5, 12, 30] {
        let (capped, capped_log) = logged(&t, &cfg(3_000), Some(rounds));
        assert_eq!(capped.stop, CreateStop::Cancelled);
        assert_eq!(capped.rounds.len(), rounds);
        let (twin, twin_log) = logged(&t, &cfg(capped.renders), None);
        assert_eq!(twin.stop, CreateStop::Budget, "after {rounds} rounds");
        assert_eq!(twin.renders, capped.renders, "after {rounds} rounds");
        assert_eq!(twin.rounds, capped.rounds, "after {rounds} rounds");
        assert_eq!(twin.starts, capped.starts, "after {rounds} rounds");
        assert_eq!(twin.candidates, capped.candidates, "after {rounds} rounds");
        assert!(twin_log == capped_log, "after {rounds} rounds");
    }
}

#[test]
fn a_record_replays_a_run_stopped_by_the_clock_under_the_settings_it_ran_under() {
    let t = target();
    let init = init();
    let tpl = templates();
    let inputs = inputs(&init, &tpl);
    let a = t.analyse().unwrap();
    // The judge's probe: a 5,000-render job, now stopped by the shell's clock rather than re-budgeted.
    let shell_cfg = cfg(5_000);
    let mut job = CreateJob::new(
        &t,
        &a,
        Portfolio::build(&t, &a, &inputs, &shell_cfg),
        &shell_cfg,
    )
    .unwrap();
    let mut rounds = 0;
    run(&mut job, &mut proxy, &mut |_| {
        rounds += 1;
        rounds < 41
    })
    .unwrap();
    let result = job.result();
    assert_eq!(result.cfg, shell_cfg, "the result carries the settings it ran under");
    assert_eq!(result.stop, CreateStop::Cancelled);
    let names: Vec<String> = tpl.iter().map(|x| x.name.clone()).collect();
    let rec = CreateRecord::new(
        &result,
        &RecordContext {
            target: &t,
            analysis: &a,
            intake: None,
            objective: "proxy/0",
            bank: None,
            templates: &names,
        },
    );
    assert_eq!(rec.cfg(), shell_cfg);
    assert_eq!(rec.settings.budget, 5_000);
    assert_eq!(rec.renders, result.renders);
    assert_eq!(rec.replay_cfg().budget, result.renders);

    let mut again = CreateJob::new(&t, &a, rec.starts().unwrap(), &rec.replay_cfg()).unwrap();
    run(&mut again, &mut proxy, &mut |_| true).unwrap();
    let replay = again.result();
    assert_eq!(replay.renders, result.renders);
    assert_eq!(replay.rounds, result.rounds);
    assert_eq!(replay.candidates.len(), result.candidates.len());
    for (x, y) in replay.candidates.iter().zip(&result.candidates) {
        assert_eq!(x.distance, y.distance);
        assert_eq!(x.state, y.state);
        assert_eq!(x.patch.to_json(), y.patch.to_json());
        assert_eq!(x.trace, y.trace);
    }
}
