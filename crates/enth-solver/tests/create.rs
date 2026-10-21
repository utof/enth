//! Lane TP: "create from sound" (`enth_solver::create`). TP's acceptance (docs/phase-3.md): K branches; identical
//! results for 1 and 8 workers; the schedule does not depend on wall time. Plus the schedule's own rules (the
//! heuristic start always kept, the rungs in renders, the hard budget) and the replay from a record.

#[path = "../examples/support/pool.rs"]
mod pool;

use std::cell::Cell;

use enth_analysis::descriptors::Want;
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_analysis::{Describer, Descriptors};
use enth_challenge::random;
use enth_challenge::spec::PRESETS;
use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_render::{Performance, RenderSpec, render};
use enth_solver::create::record::RecordContext;
use enth_solver::create::{
    CandidateState, CreateCfg, CreateJob, CreateRecord, CreateResult, CreateStop, Inputs,
    Portfolio, Provenance, Target, TargetPerf, Template, create, run,
};
use enth_solver::nearest::{
    BANK_FORMAT, Bank, BankEntry, BankHeader, EMBEDDING_VERSION, REF_NOTE, UNPITCHED, embed,
    reference_hz,
};
use enth_solver::search::{MetricTarget, Objective};

/// A low rate and a short note keep a debug build's renders cheap.
const SR: u32 = 22_050;

fn set(p: &mut Patch, addr: &str, v: f32) {
    apply(
        p,
        Op::SetParam {
            addr: addr.parse().unwrap(),
            value: v,
        },
    )
    .unwrap();
}

fn template(name: &str, text: &str) -> Template {
    Template {
        name: name.to_owned(),
        patch: enth_patch::file::load(text).unwrap().patch,
    }
}

fn templates() -> Vec<Template> {
    vec![
        template(
            "pluck_1osc",
            include_str!("../../../content/templates/pluck_1osc.patch.json"),
        ),
        template(
            "pad_2osc",
            include_str!("../../../content/templates/pad_2osc.patch.json"),
        ),
        template(
            "bass_sub",
            include_str!("../../../content/templates/bass_sub.patch.json"),
        ),
    ]
}

fn init() -> Patch {
    enth_patch::file::load(include_str!("../../../content/templates/init.patch.json"))
        .unwrap()
        .patch
}

/// The upload: a rendered pluck-like sine, with a few ms of pre-roll before the note as FE's intake keeps.
fn target() -> Target {
    let mut p = init();
    set(&mut p, "osc1.wave", 0.0);
    set(&mut p, "env1.attack", 4.0);
    set(&mut p, "env1.decay", 180.0);
    set(&mut p, "env1.sustain", 30.0);
    set(&mut p, "env1.release", 120.0);
    let perf = TargetPerf {
        key: 57,
        velocity: 100.0 / 127.0,
        on_s: 32.0 * 4.0 / f64::from(SR),
        hold_s: 0.3,
        duration_s: 0.7,
        f0_hz: None,
    };
    let mut t = Target {
        audio: Vec::new(),
        sample_rate: SR,
        render_seed: 5,
        perf,
        bandwidth_hz: None,
        noise_floor_dbfs: None,
    };
    t.audio = render(&p, &t.performance(), &t.render_spec())
        .unwrap()
        .mid();
    t
}

/// A small bank of generator-v1 patches (levels 2 and 5), rendered at the bank's reference note.
fn small_bank() -> Bank {
    let perf = Performance::note(REF_NOTE, 0.8, 0.6, 1.0);
    let spec = RenderSpec::new(SR, 3);
    let cfg = FeatureCfg {
        sample_rate: f64::from(SR),
        ..FeatureCfg::default()
    };
    let mut fx = FeatureExtractor::new(cfg);
    let mut describer = Describer::new(f64::from(SR));
    let mut entries = Vec::new();
    for (i, preset) in [1usize, 4, 1, 4, 4].into_iter().enumerate() {
        let rspec = PRESETS[preset].spec(1, 0x5450_0000 + i as u64);
        let g = random::target(&rspec).unwrap();
        let mid = render(&g.patch, &perf, &spec).unwrap().mid();
        let f = fx.extract(&mid);
        let desc: Option<Descriptors> = describer
            .describe_some(&mid, enth_solver::note_timing(&perf), Want::CORE)
            .ok();
        entries.push(BankEntry {
            seed: rspec.seed,
            difficulty: rspec.difficulty,
            musical: rspec.musical,
            pitch_cents: UNPITCHED,
            fingerprint_prefix: g.fingerprint.prefix().0,
            embedding: embed(
                &f,
                &cfg,
                0.0,
                reference_hz(desc.as_ref()),
                f64::from(SR) / 2.0,
            ),
        });
    }
    Bank::new(
        BankHeader {
            format: BANK_FORMAT,
            generator: 1,
            template: 0,
            embedding: EMBEDDING_VERSION,
            note: REF_NOTE,
            velocity: 0.8,
            hold_s: 0.6,
            duration_s: 1.0,
            sample_rate: SR,
            render_seed: 3,
            exclusions_sha256: [0; 32],
        },
        entries,
    )
}

/// A cheap stand-in for render + metric: how far a few parameters are from the upload's, on their normalised axes;
/// a patch missing one pays 1.
fn proxy(p: &Patch) -> f64 {
    let goal = [
        ("osc1.wave", 0.0),
        ("env1.attack", 4.0),
        ("env1.decay", 180.0),
        ("env1.sustain", 30.0),
        ("env1.release", 120.0),
        ("osc1.semi", 0.0),
    ];
    goal.iter()
        .map(|(a, g)| {
            let addr: ParamAddr = a.parse().unwrap();
            match (p.spec(&addr), p.value(&addr)) {
                (Some(s), Some(v)) => f64::from((s.to_norm(v) - s.to_norm(*g)).abs()),
                _ => 1.0,
            }
        })
        .sum()
}

fn cheap_cfg(budget: usize) -> CreateCfg {
    CreateCfg {
        k: 4,
        budget,
        seed: 11,
        neighbours: 3,
        min_spread: 0.0,
        ..CreateCfg::default()
    }
}

fn run_with(
    target: &Target,
    bank: Option<&Bank>,
    objective: &mut dyn Objective,
    cfg: &CreateCfg,
    progress: &mut dyn FnMut(&CreateJob) -> bool,
) -> CreateResult {
    let init = init();
    let templates = templates();
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank,
    };
    create(target, &inputs, objective, cfg, progress).unwrap()
}

#[test]
fn k_branches_and_identical_results_for_1_and_8_workers() {
    let t = target();
    let bank = small_bank();
    let metric = MetricTarget::from_audio(&t.audio, &t.performance(), &t.render_spec());
    let cfg = CreateCfg {
        k: 4,
        budget: 260,
        seed: 3,
        neighbours: 3,
        min_spread: 0.0,
        ..CreateCfg::default()
    };
    let one = run_with(
        &t,
        Some(&bank),
        &mut pool::Pool::new(&metric, 1),
        &cfg,
        &mut |_| true,
    );
    let eight = run_with(
        &t,
        Some(&bank),
        &mut pool::Pool::new(&metric, 8),
        &cfg,
        &mut |_| true,
    );
    assert_eq!(one, eight, "the worker count changes nothing");

    // K branches, from a screened portfolio of the heuristic, three templates and three neighbours.
    assert_eq!(one.candidates.len(), 4);
    assert_eq!(
        one.starts.len(),
        7,
        "{:?}",
        one.starts
            .iter()
            .map(|s| s.provenance.kind())
            .collect::<Vec<_>>()
    );
    assert!(one.starts.iter().all(|s| s.screen_distance.is_some()));
    assert!(
        one.starts
            .iter()
            .any(|s| matches!(s.provenance, Provenance::Knn { .. }))
    );
    assert_eq!(one.candidates[0].provenance, Provenance::Heuristic);
    assert!(one.renders <= cfg.budget);
    assert_eq!(one.rounds[0].rung, None, "the screening round");
    assert_eq!(
        one.candidates
            .iter()
            .filter(|c| c.state == CandidateState::Pruned)
            .count(),
        2,
        "4 branches: 2 survive rung 0 (the finalists)"
    );
    let best = one.best().unwrap();
    assert!(
        best.distance <= best.proposal_distance.unwrap(),
        "search never loses its start"
    );
}

#[test]
fn the_schedule_depends_on_renders_never_on_wall_time() {
    let t = target();
    let cfg = cheap_cfg(900);
    let plain = run_with(&t, None, &mut proxy, &cfg, &mut |_| true);

    // The same job with a slow, uneven objective and a slow progress callback: wall time changes, nothing else.
    let calls = Cell::new(0u64);
    let mut slow = |p: &Patch| {
        calls.set(calls.get() + 1);
        if calls.get().is_multiple_of(97) {
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        proxy(p)
    };
    let slowed = run_with(&t, None, &mut slow, &cfg, &mut |_| {
        std::thread::sleep(std::time::Duration::from_millis(1));
        true
    });
    assert_eq!(plain, slowed);

    // A wall-clock cap only truncates: stopping after round 5 gives a prefix of the full run.
    let mut rounds = 0;
    let capped = run_with(&t, None, &mut proxy, &cfg, &mut |_| {
        rounds += 1;
        rounds < 5
    });
    assert_eq!(capped.stop, CreateStop::Cancelled);
    assert_eq!(capped.rounds[..], plain.rounds[..capped.rounds.len()]);
    for (c, full) in capped.candidates.iter().zip(&plain.candidates) {
        assert_eq!(
            c.trace[..],
            full.trace[..c.trace.len()],
            "branch {}",
            c.branch
        );
    }
}

#[test]
fn successive_halving_runs_in_renders_and_the_budget_is_hard() {
    let t = target();
    let cfg = CreateCfg {
        k: 8,
        budget: 3_000,
        ..cheap_cfg(3_000)
    };
    // 1 heuristic + 3 templates: 4 starts, all screened in round 0, and 4 branches.
    let r = run_with(&t, None, &mut proxy, &cfg, &mut |_| true);
    assert_eq!(r.candidates.len(), 4);
    assert_eq!((r.rounds[0].rung, r.rounds[0].renders), (None, 4));
    assert!(r.rounds[1..].iter().all(|x| x.rung.is_some()));
    assert!(r.renders <= cfg.budget);
    // Live branches never grow from one rung to the next: 4, then at most 2 (half, at least the finalists), then 2.
    let max_live = |rung| {
        r.rounds
            .iter()
            .filter(|x| x.rung == Some(rung))
            .map(|x| x.live)
            .max()
            .unwrap_or(0)
    };
    assert!(
        max_live(0) <= 4 && max_live(1) <= 2 && max_live(2) <= 2,
        "{:?}",
        r.rounds
    );
    // Rung 0 spends about 10 % of what screening left (299 of 2,996): it ends at the first round boundary past its
    // shares, so by less than one batch per branch later.
    let end0 = r
        .rounds
        .iter()
        .filter(|x| x.rung == Some(0))
        .map(|x| x.renders)
        .max()
        .unwrap();
    assert!(
        (4 + 299..4 + 299 + 4 * 64).contains(&end0),
        "rung 0 ended at {end0}"
    );
    assert_eq!(
        r.candidates
            .iter()
            .filter(|c| c.state == CandidateState::Pruned)
            .count(),
        2
    );

    for budget in [1, 7, 37, 101, 333] {
        let r = run_with(&t, None, &mut proxy, &cheap_cfg(budget), &mut |_| true);
        assert!(
            r.renders <= budget,
            "{} renders for a budget of {budget}",
            r.renders
        );
        assert!(
            matches!(r.stop, CreateStop::Budget | CreateStop::Exhausted),
            "{:?}",
            r.stop
        );
    }
}

#[test]
fn the_shell_may_set_the_budget_after_screening_and_a_replay_under_it_is_the_same() {
    let t = target();
    let init = init();
    let templates = templates();
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: None,
    };
    let analysis = t.analyse().unwrap();
    let cfg = cheap_cfg(5_000);
    let mut job = CreateJob::new(
        &t,
        &analysis,
        Portfolio::build(&analysis, &inputs, &cfg),
        &cfg,
    )
    .unwrap();
    assert!(job.budget_open());
    let screen = job.ask();
    assert!(!job.budget_open(), "not while a batch is out");
    let d: Vec<f64> = screen.iter().map(proxy).collect();
    job.tell(&d).unwrap();
    // The shell timed the screening round and can afford 700 renders in all.
    assert!(job.budget_open());
    job.set_budget(700).unwrap();
    run(&mut job, &mut proxy, &mut |_| true).unwrap();
    assert!(!job.budget_open());
    assert_eq!(job.set_budget(900).unwrap_err().code(), "bad_create_cfg");
    let shell = job.result();
    assert!(shell.renders <= 700);

    let replay = run_with(&t, None, &mut proxy, &cheap_cfg(700), &mut |_| true);
    assert_eq!(replay, shell);
}

#[test]
fn the_heuristic_start_is_always_kept() {
    let t = target();
    let bank = small_bank();
    // An objective that hates the init template: the heuristic start screens worst, and is kept anyway.
    let init = init();
    let mut hate = |p: &Patch| if *p == init { 99.0 } else { proxy(p) };
    let cfg = CreateCfg {
        k: 2,
        ..cheap_cfg(200)
    };
    let r = run_with(&t, Some(&bank), &mut hate, &cfg, &mut |_| true);
    assert_eq!(r.candidates.len(), 2);
    assert!(r.starts.len() > 2);
    assert_eq!(r.starts[0].provenance, Provenance::Heuristic);
    assert_eq!(r.starts[0].branch, Some(0));
    let kept = r.starts.iter().filter(|s| s.branch.is_some()).count();
    assert_eq!(kept, 2);
}

#[test]
fn a_record_replays_the_run_and_keeps_every_start() {
    let t = target();
    let bank = small_bank();
    let cfg = cheap_cfg(400);
    let init = init();
    let templates = templates();
    let inputs = Inputs {
        init: &init,
        templates: &templates,
        bank: Some(&bank),
    };
    let analysis = t.analyse().unwrap();
    let portfolio = Portfolio::build(&analysis, &inputs, &cfg);
    let mut job = CreateJob::new(&t, &analysis, portfolio, &cfg).unwrap();
    run(&mut job, &mut proxy, &mut |_| true).unwrap();
    let result = job.result();

    let names: Vec<String> = templates.iter().map(|x| x.name.clone()).collect();
    let record = CreateRecord::new(
        &result,
        &RecordContext {
            target: &t,
            analysis: &analysis,
            cfg: &cfg,
            objective: "proxy/0",
            bank: Some(&bank),
            templates: &names,
        },
    );
    assert_eq!(record.schema, "enth.create/1");
    assert_eq!(record.starts.len(), result.starts.len());
    assert_eq!(record.candidates.len(), cfg.k);
    assert_eq!(record.settings.bank.as_ref().unwrap().entries, bank.len());
    assert_eq!(record.input.audio_sha256.len(), 64);
    assert_eq!(record.cfg(), cfg);

    // Replay from the recorded starts alone (no bank, no templates): the same run.
    let replayed_starts = record.starts().unwrap();
    let mut again = CreateJob::new(&t, &analysis, replayed_starts, &record.cfg()).unwrap();
    run(&mut again, &mut proxy, &mut |_| true).unwrap();
    // A patch file stamps an unstamped patch with the current engine version (a record, never an input).
    assert_eq!(unstamped(again.result()), unstamped(result));
}

/// The result with every patch's engine stamp cleared.
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
fn a_short_clip_is_kept_and_searched_without_the_heuristic() {
    // 10 ms: too short to describe, never rejected.
    let mut t = target();
    t.audio.truncate((f64::from(SR) * 0.010) as usize);
    t.perf.hold_s = 0.005;
    t.perf.duration_s = 0.010;
    let r = run_with(&t, None, &mut proxy, &cheap_cfg(120), &mut |_| true);
    assert_eq!(r.candidates.len(), 4);
    assert!(
        r.notes.iter().any(|n| n.contains("too short")),
        "{:?}",
        r.notes
    );
}

#[test]
fn a_job_moves_to_the_shells_thread_and_a_bank_is_shared() {
    fn send<T: Send>() {}
    fn send_sync<T: Send + Sync>() {}
    send::<CreateJob>();
    send_sync::<Bank>();
    send_sync::<Target>();
    send_sync::<CreateResult>();
}

#[test]
fn bad_targets_and_settings_are_errors_not_panics() {
    let mut t = target();
    t.audio.clear();
    assert_eq!(t.analyse().unwrap_err().code(), "bad_target");
    let t = target();
    let init = init();
    let inputs = Inputs {
        init: &init,
        templates: &[],
        bank: None,
    };
    for cfg in [
        CreateCfg {
            k: 0,
            ..CreateCfg::default()
        },
        CreateCfg {
            budget: 0,
            ..CreateCfg::default()
        },
        CreateCfg {
            rungs: [0.0; 3],
            ..CreateCfg::default()
        },
        CreateCfg {
            lambda: 2,
            ..CreateCfg::default()
        },
    ] {
        let e = create(&t, &inputs, &mut proxy, &cfg, &mut |_| true).unwrap_err();
        assert_eq!(e.code(), "bad_create_cfg");
    }
    let analysis = t.analyse().unwrap();
    match CreateJob::new(&t, &analysis, Portfolio::default(), &CreateCfg::default()) {
        Err(e) => assert_eq!(e.code(), "no_starts"),
        Ok(_) => panic!("an empty portfolio is an error"),
    }
}
