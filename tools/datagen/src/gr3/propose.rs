//! **Owner: lane FTL** (R3b's fast loop, 2026-10-25; the scratch tool `ms-devprop` that lanes MS and EVM used, made a
//! command).
//!
//! ```text
//! enth-datagen gr3 proposals --job JOB.json --model enth-model-<id>.bin --out proposals.json [--side side.json]
//! ```
//!
//! A model's proposals for every target of a GR3 job, as the `enth.gr3-proposals/1` file arms B and C read
//! ([`FileProposer`](super::arm::FileProposer)): per target, the kit's own upload ([`Gr3Target::build`],
//! [`Gr3Target::upload`]), FE's intake at the scoring rate, `Target::from_intake_scored` with the job's window (what
//! [`run_target`](super::arm::run_target) builds inside its clock), then `Model::propose` (lane MS): the starts in
//! the model's order (descending confidence), each with its confidence, and the inference time as `charge_s`.
//! `--side` writes what to check against the kit's records: each intake's reading, the model record (`settings.model`:
//! the file's, the input's and the outputs' hashes, the drops), the hypothesis order and the notes.
//!
//! `cargo xtask gr3-run --model M` (dev only) runs this before the arms, so the fast create eval is one command:
//! `cargo xtask gr3-run --arm B --split dev --mode renders --budget 500 --model M --out RUNS`, read with
//! `cargo xtask gr3-report --runs RUNS --a A_RUN --b B_RUN --at 250` (and `--at 500`). A proposals file is a dry
//! run of arms B and C, never a GR3 result: a test run's model must propose inside the clock (lane `EVb`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use enth_analysis::intake::Intake;
use enth_render::DEFAULT_SEED;
use enth_solver::create::{Model, Provenance, Target};
use serde_json::{Value, json};

use super::arm::PROPOSALS_SCHEMA;
use super::job::Job;
use super::target::Gr3Target;

const USAGE: &str = "usage: enth-datagen gr3 proposals --job JOB.json --model enth-model-<id>.bin --out proposals.json [--side side.json]";

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// `gr3 proposals`.
pub fn run(args: &[String]) -> Result<(), String> {
    let job_path = arg(args, "--job").ok_or(USAGE)?;
    let model_path = arg(args, "--model").ok_or(USAGE)?;
    let out = arg(args, "--out").ok_or(USAGE)?;
    let text =
        fs::read_to_string(job_path).map_err(|e| format!("cannot read the job {job_path}: {e}"))?;
    let job: Job = serde_json::from_str(&text).map_err(|e| format!("the job {job_path}: {e}"))?;
    let bytes =
        fs::read(model_path).map_err(|e| format!("cannot read the model {model_path}: {e}"))?;
    let tb = Instant::now();
    let model = Model::parse(&bytes).map_err(|e| {
        format!(
            "the model {model_path} is refused: {e} ({}); hint: {}",
            e.code(),
            e.hint()
        )
    })?;
    let build_s = tb.elapsed().as_secs_f64();
    eprintln!(
        "gr3 proposals: model {} ({} hypotheses, {:.3} GFLOP, built in {build_s:.2} s), {} targets",
        model.id(),
        model.hypotheses(),
        model.net().flops() as f64 / 1e9,
        job.targets.len()
    );
    let (file, side) = proposals(&job, &model, |line| eprintln!("{line}"))?;
    write_json(Path::new(out), &file)?;
    if let Some(side_path) = arg(args, "--side") {
        let mut side = side;
        side["build_s"] = json!(build_s);
        write_json(Path::new(side_path), &side)?;
    }
    eprintln!("gr3 proposals: wrote {out}");
    Ok(())
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The proposals file (`enth.gr3-proposals/1`) and the side file for `job`'s targets, in the job's order; `log` gets
/// one line per target.
pub fn proposals(
    job: &Job,
    model: &Model,
    mut log: impl FnMut(String),
) -> Result<(Value, Value), String> {
    let mut targets = BTreeMap::new();
    let mut sides = BTreeMap::new();
    for t in &job.targets {
        let g = Gr3Target::build(t).map_err(|e| format!("{}: {e}", t.id))?;
        let upload = g.upload().map_err(|e| format!("{}: {e}", t.id))?;
        let intake = Intake::new(&upload, 1, enth_challenge::model::SCORING_SAMPLE_RATE)
            .map_err(|e| format!("{}: intake: {e}", t.id))?;
        let ct = Target::from_intake_scored(&intake, DEFAULT_SEED, job.cfg.score_s);
        let ti = Instant::now();
        let mp = model.propose(&intake, &ct);
        let infer_s = ti.elapsed().as_secs_f64();
        let mut starts = Vec::with_capacity(mp.starts.len());
        let mut order = Vec::with_capacity(mp.starts.len());
        for s in &mp.starts {
            let (hypothesis, confidence) = match &s.provenance {
                Provenance::Model {
                    hypothesis,
                    confidence,
                    ..
                } => (*hypothesis, *confidence),
                _ => return Err(format!("{}: a model start without its provenance", t.id)),
            };
            let patch = serde_json::to_value(s.patch.to_file()).map_err(|e| e.to_string())?;
            starts.push(json!({ "patch": patch, "confidence": confidence }));
            order.push(json!([hypothesis, confidence]));
        }
        log(format!(
            "{}: {} starts, {} dropped, {:.0} ms{}",
            t.id,
            mp.starts.len(),
            mp.dropped.len(),
            infer_s * 1000.0,
            if mp.notes.is_empty() {
                String::new()
            } else {
                format!("; {}", mp.notes.join(" | "))
            }
        ));
        targets.insert(
            t.id.clone(),
            json!({ "charge_s": infer_s, "starts": starts }),
        );
        let r = &intake.report;
        sides.insert(
            t.id.clone(),
            json!({
                "intake": { "key": r.perf.key, "on_s": r.perf.on_s, "hold_s": r.perf.hold_s, "duration_s": ct.perf.duration_s,
                            "f0_hz": r.perf.f0_hz, "bandwidth_hz": r.degradation.bandwidth_hz },
                "model": serde_json::to_value(mp.record()).map_err(|e| e.to_string())?,
                "order": order,
                "notes": mp.notes,
                "infer_s": infer_s,
            }),
        );
    }
    let file = json!({ "schema": PROPOSALS_SCHEMA, "model": model.id(), "targets": targets });
    let side = json!({
        "model": model.id(),
        "file_sha256": enth_solver::create::record::hex(&model.sha256()),
        "job": job.run_id,
        "targets": sides,
    });
    Ok((file, side))
}

#[cfg(test)]
mod tests {
    //! On the committed tiny model (`exp_tiny`, lane EXP: propose v0's shape, K = 8, random weights) and one cheap dev
    //! target: the file is what `Model::propose` gives, in its order, it reads back through the kit's own
    //! `FileProposer`, and arm C renders exactly its starts.
    use std::path::PathBuf;

    use super::super::arm::{self, Ctx, FileProposer, Proposer};
    use super::super::job::{Arm, CfgJob, Job, Kind, Mode, SCHEMA, TargetJob};
    use super::super::templates;
    use super::*;

    const TINY: &[u8] =
        include_bytes!("../../../../crates/enth-solver/tests/fixtures/propose/exp_tiny.bin");

    fn job(arm: Arm, proposals: Option<PathBuf>) -> Job {
        Job {
            schema: SCHEMA.into(),
            run_id: "ftl-test".into(),
            out: PathBuf::from("/dev/null"),
            patches_out: None,
            arm,
            mode: Mode::Renders,
            seconds: None,
            budget: Some(40),
            marks: vec![],
            workers: 1,
            lock: None,
            cfg: CfgJob {
                score_s: 1.0,
                ..CfgJob::default()
            },
            bank: None,
            bank_entries: None,
            proposals,
            templates: None,
            split: "dev".into(),
            header: Value::Null,
            // `gen-d-l1-01` of content/gr3/synth.toml (dev), as in the kit's own tests.
            targets: vec![TargetJob {
                id: "gen-d-l1-01".into(),
                split: "dev".into(),
                kind: Kind::Generator,
                level: Some(1),
                phrase: Some(1),
                core: false,
                code: Some("1040-1PD3-SPQ1-H4".into()),
                fingerprint: Some(
                    "eb30ccd04b3edf5f6f5ad62c140e2613f8c33c685fced0afd518c144bdd2eac5".into(),
                ),
                file: None,
                sha256: None,
                perf: None,
                budget: None,
            }],
        }
    }

    fn strip_timing(mut v: Value) -> Value {
        for t in v["targets"].as_object_mut().unwrap().values_mut() {
            t.as_object_mut().unwrap().remove("charge_s");
            t.as_object_mut().unwrap().remove("infer_s");
        }
        v
    }

    #[test]
    fn proposals_are_the_models_starts_in_its_order_and_read_back() {
        let model = Model::parse(TINY).unwrap();
        let j = job(Arm::C, None);
        let mut lines = Vec::new();
        let (file, side) = proposals(&j, &model, |l| lines.push(l)).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(file["schema"], PROPOSALS_SCHEMA);
        assert_eq!(file["model"], model.id());
        // The long way round: the kit's intake and target, then Model::propose.
        let t = Gr3Target::build(&j.targets[0]).unwrap();
        let intake = Intake::new(
            &t.upload().unwrap(),
            1,
            enth_challenge::model::SCORING_SAMPLE_RATE,
        )
        .unwrap();
        let ct = Target::from_intake_scored(&intake, DEFAULT_SEED, j.cfg.score_s);
        let mp = model.propose(&intake, &ct);
        let starts = file["targets"]["gen-d-l1-01"]["starts"].as_array().unwrap();
        assert_eq!(starts.len(), mp.starts.len());
        assert_eq!(mp.starts.len() + mp.dropped.len(), model.hypotheses());
        let mut last = f64::INFINITY;
        for (s, m) in starts.iter().zip(&mp.starts) {
            assert_eq!(s["patch"], serde_json::to_value(m.patch.to_file()).unwrap());
            if let Some(c) = s["confidence"].as_f64() {
                assert!(c <= last, "descending confidence");
                last = c;
            }
        }
        assert_eq!(
            side["targets"]["gen-d-l1-01"]["model"],
            serde_json::to_value(mp.record()).unwrap()
        );
        // Deterministic but for the timing.
        let (again, _) = proposals(&j, &model, |_| {}).unwrap();
        assert_eq!(strip_timing(again), strip_timing(file.clone()));

        // The kit reads it back, and arm C renders exactly its starts.
        let dir = std::env::temp_dir().join(format!("enth-ftl-proposals-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("proposals.json");
        write_json(&path, &file).unwrap();
        let fp = FileProposer::load(&path).unwrap();
        assert_eq!(fp.id(), format!("file:{}", model.id()));
        let analysis = ct.analyse().unwrap();
        let back = fp.propose("gen-d-l1-01", &ct, &analysis).unwrap();
        assert_eq!(back.len(), mp.starts.len());
        for (b, m) in back.iter().zip(&mp.starts) {
            assert_eq!(b.patch, m.patch);
        }
        let (init, tpl) = templates().unwrap();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let ctx = Ctx {
            init: &init,
            templates: &tpl,
            bank: None,
            pool: &pool,
            proposer: Some(&fp),
        };
        let jc = job(Arm::C, Some(path));
        let (rec, _) = arm::run_target(&jc, &jc.targets[0], &ctx).unwrap();
        // Arm C's portfolio keeps each distinct patch once (`Portfolio::push`), and renders each once.
        let mut distinct: Vec<&enth_patch::Patch> = Vec::new();
        for s in &mp.starts {
            if !distinct.contains(&&s.patch) {
                distinct.push(&s.patch);
            }
        }
        assert!(
            !distinct.is_empty(),
            "the tiny model proposes something for this target"
        );
        assert_eq!(rec.fin.starts, distinct.len());
        assert_eq!(rec.fin.renders, distinct.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_and_refusals() {
        assert!(run(&[]).unwrap_err().contains("usage"));
        let dir = std::env::temp_dir().join(format!("enth-ftl-refusal-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let job_path = dir.join("job.json");
        fs::write(&job_path, "{}").unwrap();
        let bad = dir.join("bad.bin");
        fs::write(&bad, b"not a model").unwrap();
        let args = |m: &Path| {
            vec![
                "--job".to_owned(),
                job_path.display().to_string(),
                "--model".to_owned(),
                m.display().to_string(),
                "--out".to_owned(),
                dir.join("p.json").display().to_string(),
            ]
        };
        // A job file that is not a job.
        assert!(run(&args(&bad)).unwrap_err().contains("the job"));
        // A job, and a file that is not a model: refused with the pack's code and hint.
        fs::write(
            &job_path,
            serde_json::to_string(&job(Arm::B, None)).unwrap(),
        )
        .unwrap();
        let err = run(&args(&bad)).unwrap_err();
        assert!(err.contains("refused") && err.contains("hint"), "{err}");
        assert!(!dir.join("p.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
