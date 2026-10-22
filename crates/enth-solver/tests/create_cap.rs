//! Lane EVM (R3b): the model's share cap (`CreateCfg::model_cap`, docs/r3.md, "The model's share cap"). Lane `PUIb`
//! found the model's hypotheses taking 6 of 7 free branches on `gen-d-l2-01`, and successive halving pruning the
//! k-NN neighbour that would have reached 0.018 (0.101 with the model). With the cap, a model's starts hold at most
//! half of the places, rounded up, at screening and at every rung, while other starts or branches can fill the rest.
//! Checked here: the cap holds at screening and at every rung and binds where it should (it is off in R3b's first
//! behaviour), it never shrinks a selection, a job without model starts is unchanged, the result is the same for 1
//! and 6 workers, and the record says the cap only when a model joined, replaying either way (its JSON side is in
//! enth-datagen's `tests/create_record.rs`, which has `serde_json`).

#[path = "../examples/support/pool.rs"]
mod pool;

mod create_support;

use create_support::{init, proxy, set, target, target_patch, templates};
use enth_patch::Patch;
use enth_solver::create::record::RecordContext;
use enth_solver::create::{
    CandidateState, CreateCfg, CreateJob, CreateRecord, CreateResult, Inputs, Portfolio, Progress,
    Provenance, Start, Target, run,
};

/// Eight "hypotheses", each closer to the upload than any template or the heuristic start (the decay a little off,
/// by more for a later one): a model that crowds every other source out of the branches.
fn hypotheses() -> Vec<Start> {
    (0..8)
        .map(|k| {
            let mut p = target_patch();
            set(
                &mut p,
                "env1.decay",
                180.0 * (1.0 + 0.04 * (k as f32 + 1.0)),
            );
            Start {
                patch: p,
                provenance: Provenance::Model {
                    model: "evm-cap".to_owned(),
                    hypothesis: k,
                    confidence: Some(1.0 / (k as f64 + 2.0)),
                },
            }
        })
        .collect()
}

/// K 8 with short rungs: screening, then 8 → 4 → 2 branches within a small budget.
fn cfg(model_cap: bool) -> CreateCfg {
    CreateCfg {
        k: 8,
        budget: 900,
        seed: 5,
        first_rung: 20,
        model_cap,
        ..CreateCfg::default()
    }
}

fn is_model(p: &Provenance) -> bool {
    matches!(p, Provenance::Model { .. })
}

fn live(state: CandidateState) -> bool {
    matches!(state, CandidateState::Proposed | CandidateState::Searching)
}

/// At each rung's start (and right after screening): (live branches, live model branches, live others).
#[derive(Debug, Default)]
struct Rungs(Vec<(Option<usize>, usize, usize, usize)>);

impl Rungs {
    fn see(&mut self, p: &Progress) {
        if p.branches.is_empty() || self.0.last().is_some_and(|r| r.0 == p.rung) {
            return;
        }
        let alive: Vec<_> = p.branches.iter().filter(|b| live(b.state)).collect();
        let models = alive.iter().filter(|b| is_model(&b.provenance)).count();
        self.0
            .push((p.rung, alive.len(), models, alive.len() - models));
    }
}

/// A job on the proxy objective (cheap and deterministic), with the rungs it went through.
fn proxy_job(proposals: &[Start], cfg: &CreateCfg) -> (CreateResult, Rungs) {
    let target = target();
    let analysis = target.analyse().unwrap();
    let (init, templates) = (init(), templates());
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: None,
        proposals,
    };
    let portfolio = Portfolio::build(&target, &analysis, &inputs, cfg);
    let mut job = CreateJob::new(&target, &analysis, portfolio, cfg).unwrap();
    let mut rungs = Rungs::default();
    run(&mut job, &mut |p: &Patch| proxy(p), &mut |j: &CreateJob| {
        rungs.see(&j.progress());
        true
    })
    .unwrap();
    (job.result(), rungs)
}

/// The branches each screening kept, by kind.
fn branch_kinds(r: &CreateResult) -> (usize, usize) {
    let models = r
        .candidates
        .iter()
        .filter(|c| is_model(&c.provenance))
        .count();
    (models, r.candidates.len() - models)
}

#[test]
fn the_cap_holds_at_screening_and_at_every_rung_and_binds() {
    let hyps = hypotheses();
    // The portfolio: the heuristic start, three templates, the eight hypotheses (12 starts for K 8).
    let (capped, rungs) = proxy_job(&hyps, &cfg(true));
    assert_eq!(capped.starts.len(), 12);
    assert_eq!(capped.candidates.len(), 8, "the cap never shrinks K");
    assert_eq!(
        branch_kinds(&capped),
        (4, 4),
        "screening: 4 of 8 places for the model"
    );
    assert_eq!(capped.candidates[0].provenance, Provenance::Heuristic);
    // Every rung starts with the model holding at most half of the live branches, rounded up.
    assert!(rungs.0.len() >= 3, "{rungs:?}");
    for &(rung, n, models, others) in &rungs.0 {
        assert!(
            models <= n.div_ceil(2) || others == 0,
            "rung {rung:?}: {models} model branches of {n} live ({rungs:?})"
        );
    }
    let at = |r: usize| rungs.0.iter().find(|x| x.0 == Some(r)).copied().unwrap();
    assert_eq!(at(1).1, 4, "8 → 4 after the first rung");
    assert_eq!(at(1).2, 2, "2 of the 4 survivors from the model");
    assert_eq!(at(2).1, 2, "4 → 2 finalists");
    assert_eq!(at(2).2, 1, "1 of the 2 finalists from the model");

    // Off (R3b's first behaviour): the hypotheses take 7 of the 7 places next to the heuristic start and both final
    // places, which is what the cap is for.
    let (plain, rungs) = proxy_job(&hyps, &cfg(false));
    assert_eq!(branch_kinds(&plain), (7, 1));
    let at = |r: usize| rungs.0.iter().find(|x| x.0 == Some(r)).copied().unwrap();
    assert_eq!(at(2).2, 2, "uncapped, both finalists are the model's");
}

#[test]
fn without_model_starts_the_cap_changes_nothing() {
    let (on, _) = proxy_job(&[], &cfg(true));
    let (off, _) = proxy_job(&[], &cfg(false));
    assert_eq!(on.starts, off.starts);
    assert_eq!(on.candidates, off.candidates);
    assert_eq!(on.renders, off.renders);
    assert_eq!(on.rounds, off.rounds);
}

#[test]
fn too_few_others_leave_the_places_to_the_model() {
    // K 12 over 12 starts: the cap leaves 6 places for the model, the 4 others take 4, and the 2 places left go
    // back to the model, so every start is a branch.
    let cfg = CreateCfg { k: 12, ..cfg(true) };
    let (r, _) = proxy_job(&hypotheses(), &cfg);
    assert_eq!(r.candidates.len(), 12);
    assert_eq!(branch_kinds(&r), (8, 4));
}

/// The same job on real renders (the debug build's cheap target) over a pool of `workers`.
fn rendered_job(target: &Target, proposals: &[Start], workers: usize) -> CreateResult {
    let cfg = CreateCfg {
        budget: 420,
        ..cfg(true)
    };
    let analysis = target.analyse().unwrap();
    let (init, templates) = (init(), templates());
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: None,
        proposals,
    };
    let portfolio = Portfolio::build(target, &analysis, &inputs, &cfg);
    let mut job = CreateJob::new(target, &analysis, portfolio, &cfg).unwrap();
    let metric = target.metric_target();
    run(
        &mut job,
        &mut pool::Pool::new(&metric, workers),
        &mut |_| true,
    )
    .unwrap();
    job.result()
}

#[test]
fn capped_jobs_are_the_same_for_1_and_6_workers_and_the_cap_holds() {
    let target = target();
    let hyps = hypotheses();
    let one = rendered_job(&target, &hyps, 1);
    let six = rendered_job(&target, &hyps, 6);
    assert_eq!(one, six, "the worker count changes nothing");
    let (models, others) = branch_kinds(&one);
    assert_eq!(models + others, 8);
    assert!(models <= 4, "{models} model branches of 8");
    // The branches pruning kept (the ones not pruned): at most half from the model while others were kept.
    let kept: Vec<_> = one
        .candidates
        .iter()
        .filter(|c| c.state != CandidateState::Pruned)
        .collect();
    let kept_models = kept.iter().filter(|c| is_model(&c.provenance)).count();
    assert!(
        kept_models <= kept.len().div_ceil(2) || kept_models == kept.len(),
        "{kept_models} of {} kept",
        kept.len()
    );
}

fn record_of(result: &CreateResult, target: &Target) -> CreateRecord {
    let analysis = target.analyse().unwrap();
    let names: Vec<String> = templates().iter().map(|t| t.name.clone()).collect();
    CreateRecord::new(
        result,
        &RecordContext {
            target,
            analysis: &analysis,
            intake: None,
            objective: "proxy/0",
            bank: None,
            templates: &names,
        },
    )
}

/// Replays a record from its starts and settings on the proxy objective.
fn replay(record: &CreateRecord, target: &Target) -> CreateResult {
    let analysis = target.analyse().unwrap();
    let mut job = CreateJob::new(
        target,
        &analysis,
        record.starts().unwrap(),
        &record.replay_cfg(),
    )
    .unwrap();
    run(&mut job, &mut |p: &Patch| proxy(p), &mut |_| true).unwrap();
    job.result()
}

/// The result with every patch's engine stamp cleared (a patch file records the engine that wrote it).
fn unstamped(mut r: CreateResult) -> CreateResult {
    for s in &mut r.starts {
        s.patch.engine.clear();
    }
    for c in &mut r.candidates {
        c.proposal.engine.clear();
        c.patch.engine.clear();
    }
    r
}

#[test]
fn the_record_says_the_cap_only_when_a_model_joined_and_replays_either_way() {
    let target = target();
    let hyps = hypotheses();

    // A model joined, cap on: `settings.model_cap` is true, and the record replays the capped run.
    let (capped, _) = proxy_job(&hyps, &cfg(true));
    let record = record_of(&capped, &target);
    assert_eq!(record.settings.model_cap, Some(true));
    assert!(record.cfg().model_cap);
    let again = replay(&record, &target);
    assert_eq!(
        unstamped(again.clone()).candidates,
        unstamped(capped.clone()).candidates
    );
    assert_eq!(again.renders, capped.renders);

    // A record written before the cap (model starts, no `model_cap`: serde's default) reads back uncapped and
    // replays that run. The JSON side (the key absent or present) is in enth-datagen's tests/create_record.rs.
    let (plain, _) = proxy_job(&hyps, &cfg(false));
    let mut old = record_of(&plain, &target);
    assert_eq!(old.settings.model_cap, Some(false));
    old.settings.model_cap = None;
    assert!(!old.cfg().model_cap, "absent with model starts: uncapped");
    let again = replay(&old, &target);
    assert_eq!(unstamped(again).candidates, unstamped(plain).candidates);

    // No model: no `model_cap` (a record without one is what it was), and the settings read back as the job's.
    let (none, _) = proxy_job(&[], &cfg(true));
    let record = record_of(&none, &target);
    assert_eq!(record.settings.model_cap, None);
    assert_eq!(record.cfg(), cfg(true));
}
