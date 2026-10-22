//! Lane MS (R3b): a model's hypotheses as create starts (docs/r3.md, "How the Kₘ hypotheses join a create job").
//! MS's acceptance (docs/phase-3.md): **deterministic given its proposals**. Hand-built model files (ONX's
//! `ModelBuilder`), no trained model: the input and the cue come from the intake before any override, each row decodes
//! under the job's performance, non-finite rows and confidences and decode errors are dropped and said, the starts go
//! in descending confidence after the k-NN neighbours, the heuristic start is kept, the same model and input give the
//! same starts and the same result for 1 and 8 workers, and the record names the model and replays without it.

#[path = "../examples/support/pool.rs"]
mod pool;

mod create_support;
mod propose_support;

use create_support::{init, target_patch, templates};
use enth_analysis::intake::Intake;
use enth_patch::{ParamAddr, Patch};
use enth_render::{DEFAULT_SEED, Note, Performance, RenderSpec, render};
use enth_solver::create::record::{DroppedRecord, ModelRecord, RecordContext};
use enth_solver::create::{
    CreateCfg, CreateJob, CreateRecord, CreateResult, Inputs, Model, ModelProposals, Portfolio,
    Provenance, Start, Target, TargetPerf, run,
};
use enth_solver::propose::decode::{Hearing, OutputLayout};
use enth_solver::propose::features::{Cue, ModelInput};
use enth_solver::propose::net::Net;
use enth_solver::propose::pack::{ModelBuilder, ModelPack, OpCode, Val};
use propose_support::{hex, values};

/// A low rate keeps a debug build's renders cheap.
const SR: u32 = 22_050;
/// Kₘ of the fixture models.
const KM: u32 = 8;
/// Layout 1's width.
const W: u32 = 102;

/// A fractional part, for spreading the fixture's biases.
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// Per hypothesis, a bias row that decodes to a plausible patch which changes with `k`: continuous values within
/// 0.2–0.8 of their range, one categorical item lifted per group (a different one per hypothesis), `osc2` and the
/// noise on in some hypotheses (the compressor and the reverb never, to keep renders cheap), one route present.
fn bias_rows(layout: &OutputLayout) -> Vec<f32> {
    let mut out = vec![0.0f32; (KM * W) as usize];
    for k in 0..KM as usize {
        let row = &mut out[k * W as usize..(k + 1) * W as usize];
        for (i, e) in layout.continuous().iter().enumerate() {
            row[e.offset] = 0.2 + 0.6 * frac(0.37 * k as f32 + 0.61 * i as f32);
        }
        for (g, e) in layout.categorical().iter().enumerate() {
            let n = e.len();
            for item in 0..n {
                row[e.offset + item] = if item == (k + g) % n { 1.0 } else { -1.0 };
            }
        }
        for e in layout.enables() {
            let on = match e.id.as_str() {
                "osc2" => k % 2 == 0,
                "noise1" => k % 3 == 1,
                _ => false,
            };
            row[e.offset] = if on { 1.0 } else { -1.0 };
        }
        for (c, cell) in layout.routes().iter().enumerate() {
            row[cell.presence] = if c == (3 * k) % 16 { 1.0 } else { -1.0 };
            row[cell.depth] = 0.8;
        }
    }
    out
}

/// A K = 8 model whose hypotheses and confidences depend on the clip (the mel's global average) and on the context.
/// `faulty` drives hypothesis 2's first value and confidence 5 past `f32::MAX` (an infinity), as an exporter bug or an
/// overflowing graph would.
fn fixture(id: &str, faulty: bool) -> Vec<u8> {
    let layout = OutputLayout::v1();
    let mut b = ModelBuilder::new(id, KM as u16);
    let g = b.node(OpCode::GlobalAvgPool, &[Val::MEL], &[]); // [3, 1, 1]
    let g = b.node(OpCode::Reshape, &[g], &[3]);
    let x = b.node(OpCode::Concat, &[g, Val::CONTEXT], &[0]); // [8]
    let w = b.tensor_f16(
        "hyp.w",
        &[KM * W, 8],
        &values(31, (KM * W * 8) as usize, 1.0 / 128.0),
    );
    let bias = b.tensor("hyp.b", &[KM * W], &bias_rows(&layout));
    let h = b.node(OpCode::Linear, &[x, w, bias], &[]);
    let mut h = b.node(OpCode::Reshape, &[h], &[KM as i32, W as i32]);
    let wc = b.tensor("conf.w", &[KM, 8], &values(32, (KM * 8) as usize, 0.25));
    let bc = b.tensor("conf.b", &[KM], &values(33, KM as usize, 2.0));
    let mut c = b.node(OpCode::Linear, &[x, wc, bc], &[]);
    if faulty {
        let mut add = vec![0.0f32; (KM * W) as usize];
        let mut mul = vec![1.0f32; (KM * W) as usize];
        add[2 * W as usize] = 3.0e38;
        mul[2 * W as usize] = 4.0;
        let a = b.tensor("bad.add", &[KM, W], &add);
        let m = b.tensor("bad.mul", &[KM, W], &mul);
        let s = b.node(OpCode::Add, &[h, a], &[]);
        h = b.node(OpCode::Mul, &[s, m], &[]);
        let mut add = vec![0.0f32; KM as usize];
        let mut mul = vec![1.0f32; KM as usize];
        add[5] = 3.0e38;
        mul[5] = 4.0;
        let a = b.tensor("bad.cadd", &[KM], &add);
        let m = b.tensor("bad.cmul", &[KM], &mul);
        let s = b.node(OpCode::Add, &[c, a], &[]);
        c = b.node(OpCode::Mul, &[s, m], &[]);
    }
    b.output("hypotheses", h, &[KM, W]);
    b.output("confidence", c, &[KM]);
    b.build()
}

/// The upload: the pluck-like sine of `create_support`, a note at 50 ms held 0.3 s, in a 1 s file.
fn intake() -> Intake {
    let perf = Performance {
        notes: vec![Note::new(57, 100.0 / 127.0, 0.05, 0.35)],
        duration_s: 1.0,
        ..Performance::default()
    };
    let audio = render(&target_patch(), &perf, &RenderSpec::new(SR, DEFAULT_SEED))
        .unwrap()
        .mid();
    Intake::new(&audio, 1, SR).unwrap()
}

/// What the starts must be, computed the long way: the intake's input and cue, one inference, each row decoded under
/// `perf`, the decoded ones in descending confidence (ties to the lower hypothesis).
fn expected(bytes: &[u8], intake: &Intake, perf: &TargetPerf) -> Vec<(usize, f32, Patch)> {
    let pack = ModelPack::parse(bytes).unwrap();
    let net = Net::build(&pack).unwrap();
    let out = net.run(&ModelInput::from_intake(intake).unwrap()).unwrap();
    let hearing = Hearing::job(&Cue::from_intake(intake), perf).unwrap();
    let mut rows: Vec<(usize, f32, Patch)> = (0..KM as usize)
        .filter_map(|k| {
            let c = out.confidence()[k];
            let p = pack.layout().decode(out.hypothesis(k), &hearing).ok()?;
            c.is_finite().then_some((k, c, p))
        })
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    rows
}

fn model_of(start: &Start) -> (String, usize, f64) {
    match &start.provenance {
        Provenance::Model {
            model,
            hypothesis,
            confidence,
        } => (model.clone(), *hypothesis, confidence.unwrap()),
        other => panic!("not a model start: {other:?}"),
    }
}

fn value(p: &Patch, addr: &str) -> f32 {
    p.value(&addr.parse::<ParamAddr>().unwrap()).unwrap()
}

#[test]
fn the_hypotheses_join_in_descending_confidence_decoded_under_the_job() {
    let bytes = fixture("ms-fixture", false);
    let model = Model::parse(&bytes).unwrap();
    let pack = ModelPack::parse(&bytes).unwrap();
    assert_eq!(
        (model.id(), model.sha256(), model.hypotheses()),
        ("ms-fixture", pack.sha256(), 8)
    );
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let mp = model.propose(&intake, &target);
    let want = expected(&bytes, &intake, &target.perf);
    assert_eq!(want.len(), 8, "every fixture hypothesis decodes");
    assert!(
        mp.dropped.is_empty() && mp.notes.is_empty(),
        "{:?}",
        mp.notes
    );
    assert_eq!(mp.starts.len(), want.len());
    for (s, (k, c, p)) in mp.starts.iter().zip(&want) {
        assert_eq!(model_of(s), ("ms-fixture".to_owned(), *k, f64::from(*c)));
        assert_eq!(&s.patch, p, "hypothesis {k}");
    }
    let order: Vec<f64> = mp.starts.iter().map(|s| model_of(s).2).collect();
    assert!(order.windows(2).all(|w| w[0] >= w[1]), "{order:?}");
    let ks: Vec<usize> = mp.starts.iter().map(|s| model_of(s).1).collect();
    assert_ne!(
        ks,
        (0..8).collect::<Vec<_>>(),
        "the fixture's confidence reorders them"
    );
    let distinct: Vec<&Patch> = mp.starts.iter().map(|s| &s.patch).collect();
    assert!(
        distinct
            .iter()
            .enumerate()
            .all(|(i, p)| !distinct[..i].contains(p)),
        "the hypotheses differ"
    );
    // The record: the model's id, the file's SHA-256, the input's and the outputs' hashes.
    let input = ModelInput::from_intake(&intake).unwrap();
    let outputs = Net::build(&pack).unwrap().run(&input).unwrap();
    assert_eq!(
        mp.record(),
        ModelRecord {
            id: "ms-fixture".to_owned(),
            sha256: hex(&pack.sha256()),
            input_sha256: Some(hex(&input.sha256())),
            outputs_sha256: Some(hex(&outputs.sha256())),
            hypotheses: 8,
            dropped: Vec::new(),
        }
    );
    // A refused file is the shell's to report: create then runs on R3a's starts.
    let refused = Model::parse(b"not a model at all").unwrap_err();
    assert_eq!(refused.code(), "not_a_model");
}

fn job_inputs<'a>(
    init: &'a Patch,
    templates: &'a [enth_solver::create::Template],
    proposals: &'a [Start],
) -> Inputs<'a> {
    Inputs {
        init,
        templates,
        bank: None,
        proposals,
    }
}

fn cfg() -> CreateCfg {
    CreateCfg {
        k: 12,
        budget: 360,
        seed: 7,
        first_rung: 20,
        ..CreateCfg::default()
    }
}

fn run_job(target: &Target, proposals: &[Start], workers: usize) -> CreateResult {
    let analysis = target.analyse().unwrap();
    let (init, templates) = (init(), templates());
    let inputs = job_inputs(&init, &templates, proposals);
    let portfolio = Portfolio::build(target, &analysis, &inputs, &cfg());
    let mut job = CreateJob::new(target, &analysis, portfolio, &cfg()).unwrap();
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
fn the_same_model_and_input_give_the_same_starts_and_result_for_1_and_8_workers() {
    let bytes = fixture("ms-fixture", false);
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let model = Model::parse(&bytes).unwrap();
    let mp = model.propose(&intake, &target);
    // A second model built from the same bytes proposes the same starts, bit for bit.
    let again = Model::parse(&bytes).unwrap().propose(&intake, &target);
    assert_eq!(mp, again);

    let one = run_job(&target, &mp.starts, 1);
    let eight = run_job(&target, &again.starts, 8);
    assert_eq!(one, eight, "the worker count changes nothing");

    // The portfolio: the heuristic start, the three templates, then the hypotheses in descending confidence.
    let kinds: Vec<&str> = one.starts.iter().map(|s| s.provenance.kind()).collect();
    assert_eq!(
        kinds[..4],
        ["heuristic", "template", "template", "template"]
    );
    assert!(kinds[4..].iter().all(|k| *k == "model"), "{kinds:?}");
    assert_eq!(one.starts.len(), 4 + mp.starts.len());
    for (s, p) in one.starts[4..].iter().zip(&mp.starts) {
        assert_eq!((&s.patch, &s.provenance), (&p.patch, &p.provenance));
    }
    // Every start is screened once; with K at 12 each becomes a branch, the heuristic's first.
    assert!(one.starts.iter().all(|s| s.screen_distance.is_some()));
    assert_eq!(one.rounds[0].renders, one.starts.len());
    assert_eq!(one.candidates[0].provenance, Provenance::Heuristic);
    let searched_models = one
        .candidates
        .iter()
        .filter(|c| c.provenance.kind() == "model" && c.renders > 0)
        .count();
    assert!(searched_models > 0, "model branches are searched");
    assert!(one.renders <= cfg().budget);
}

#[test]
fn the_record_names_the_model_and_replays_without_it() {
    let bytes = fixture("ms-fixture", false);
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let mp = Model::parse(&bytes).unwrap().propose(&intake, &target);
    let result = run_job(&target, &mp.starts, 8);
    let analysis = target.analyse().unwrap();
    let names: Vec<String> = templates().iter().map(|t| t.name.clone()).collect();
    let mut record = CreateRecord::new(
        &result,
        &RecordContext {
            target: &target,
            analysis: &analysis,
            intake: Some(&intake.report),
            objective: "logmel-l1/1",
            bank: None,
            templates: &names,
        },
    );
    assert_eq!(record.settings.model, None, "the shell adds the model");
    record.settings.model = Some(mp.record());
    assert_eq!(
        record.settings.model.as_ref().map(|m| m.id.as_str()),
        Some("ms-fixture")
    );
    // The recorded starts (the hypotheses as patches) replay the run with no model file and no inference.
    let mut job = CreateJob::new(
        &target,
        &analysis,
        record.starts().unwrap(),
        &record.replay_cfg(),
    )
    .unwrap();
    let metric = target.metric_target();
    run(&mut job, &mut pool::Pool::new(&metric, 3), &mut |_| true).unwrap();
    let (replay, result) = (unstamped(job.result()), unstamped(result));
    assert_eq!(replay.starts, result.starts);
    assert_eq!(replay.candidates, result.candidates);
    assert_eq!(replay.renders, result.renders);
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
fn an_override_changes_the_decoding_never_the_input() {
    let bytes = fixture("ms-fixture", false);
    let model = Model::parse(&bytes).unwrap();
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let base = model.propose(&intake, &target);
    // `--note` an octave up: the model is told the intake's key and hears the same clip; decoding re-keys the semis.
    let mut up = target.clone();
    up.perf.key += 12;
    let moved = model.propose(&intake, &up);
    assert_eq!(moved.input_sha256(), base.input_sha256());
    assert_eq!(moved.outputs_sha256(), base.outputs_sha256());
    let want = expected(&bytes, &intake, &up.perf);
    assert_eq!(moved.starts.len(), want.len());
    for ((m, b), (k, _, p)) in moved.starts.iter().zip(&base.starts).zip(&want) {
        assert_eq!(model_of(m).1, *k);
        assert_eq!(model_of(b).1, *k, "the same outputs, the same order");
        assert_eq!(&m.patch, p);
        assert_eq!(
            value(&m.patch, "osc1.semi"),
            value(&b.patch, "osc1.semi") - 12.0
        );
        assert_eq!(
            value(&m.patch, "osc1.fine"),
            value(&b.patch, "osc1.fine"),
            "tuned from the intake's pitch"
        );
    }
    // Far from the told key, every hypothesis re-keys past ±48 semitones and is dropped, and said.
    let input = ModelInput::from_intake(&intake).unwrap();
    let cue = Cue {
        key: 20,
        ..Cue::from_intake(&intake)
    };
    let far = TargetPerf {
        key: 127,
        ..target.perf
    };
    let none = model.propose_input(&input, &cue, &far);
    assert_eq!(none.starts, []);
    assert_eq!(none.dropped.len(), 8);
    assert!(none.dropped.iter().all(|d| d.code == "hypothesis_rekey"));
    assert_eq!(none.notes.len(), 8);
    assert!(none.notes[0].contains("hypothesis 0 dropped (hypothesis_rekey)"));
}

#[test]
fn non_finite_rows_and_confidences_are_dropped_and_said() {
    let bytes = fixture("ms-faulty", true);
    let model = Model::parse(&bytes).unwrap();
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let mp: ModelProposals = model.propose(&intake, &target);
    let dropped: Vec<(usize, &str)> = mp.dropped.iter().map(|d| (d.hypothesis, d.code)).collect();
    assert_eq!(
        dropped,
        [(2, "hypothesis_non_finite"), (5, "confidence_non_finite")]
    );
    assert_eq!(mp.starts.len(), 6);
    assert!(mp.starts.iter().all(|s| ![2, 5].contains(&model_of(s).1)));
    assert_eq!(mp.notes.len(), 2);
    assert_eq!(
        mp.record().dropped,
        [
            DroppedRecord {
                hypothesis: 2,
                code: "hypothesis_non_finite".to_owned()
            },
            DroppedRecord {
                hypothesis: 5,
                code: "confidence_non_finite".to_owned()
            },
        ]
    );
    assert_eq!(
        mp.starts.len(),
        expected(&bytes, &intake, &target.perf).len()
    );
}

#[test]
fn a_silent_window_proposes_nothing_and_says_why() {
    let model = Model::parse(&fixture("ms-fixture", false)).unwrap();
    let intake = intake();
    let target = Target::from_intake(&intake, DEFAULT_SEED);
    let silent = vec![0.0f32; SR as usize];
    let mp = model.propose_clip(&silent, SR, &Cue::from_intake(&intake), &target.perf);
    assert_eq!((mp.starts.len(), mp.dropped.len()), (0, 0));
    assert_eq!(mp.notes.len(), 1);
    assert!(mp.notes[0].contains("features_silent"), "{}", mp.notes[0]);
    let r = mp.record();
    assert_eq!((r.input_sha256, r.outputs_sha256), (None, None));
    assert_eq!(r.id, "ms-fixture");
}

/// `settings.model`'s wire names: a change here is a schema change (`enth.create/1` grows only additively).
#[test]
fn the_model_records_wire_names_are_pinned() {
    let record = ModelRecord {
        id: "propose-v0".to_owned(),
        sha256: "ab".repeat(32),
        input_sha256: Some("cd".repeat(32)),
        outputs_sha256: None,
        hypotheses: 8,
        dropped: vec![DroppedRecord {
            hypothesis: 3,
            code: "hypothesis_rekey".to_owned(),
        }],
    };
    insta::assert_json_snapshot!("model_record", record);
}
