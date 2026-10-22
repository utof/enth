//! The proposal model in the plugin (lane `PUIb`, R3b; feature `create`): where model files are looked for and which is
//! chosen, a refusal that leaves the job to search alone and says why, a job whose starts include the model's
//! hypotheses, the same candidates for any worker count with the model, and a model parsed once until its files
//! change.
//!
//! No trained model is used: lane EXP's committed tiny fixture (`exp_tiny.bin`, 29.5 KB, propose v0's architecture at
//! 10,768 parameters, randomly initialised) proposes in the jobs, and hand-built `ModelBuilder` files (a linear head on
//! the mel's mean and the context) test the choice between files. The tests that run jobs hold `tests_create`'s
//! `SERIAL` lock, because its tests read the process's thread names.

#![allow(
    clippy::disallowed_methods,
    reason = "test files and timing; their bits do not matter"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use enth_session::create::{
    CreateIntake, CreateJobId, CreatePost, CreateRequest, CreateRun, CreateSettings, CreateSource,
    CreateStop,
};
use enth_solver::create::{Candidate, Provenance};
use enth_solver::propose::pack::{ModelBuilder, OpCode, Val};

use crate::create::model::{self, Models};
use crate::create::{self, Create};
use crate::tests_create::SERIAL;

/// Lane EXP's tiny fixture: propose v0's architecture, K = 8, model id `exp-tiny`.
const EXP_TINY: &[u8] = include_bytes!("../../enth-solver/tests/fixtures/propose/exp_tiny.bin");
const EXP_TINY_ID: &str = "exp-tiny";

/// A fresh, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("enth-model-test-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A K = 8 model with this id and `ds1` version: a linear head on the mel's per-resolution mean and the context, its
/// confidences from the same input. `extra` adds an info entry (another length on disk).
fn small_model(id: &str, ds1: u16, extra: Option<&str>) -> Vec<u8> {
    const KM: u32 = 8;
    const W: u32 = 102;
    let mut b = ModelBuilder::new(id, KM as u16);
    b.provenance("0.3.0", 0, ds1, [0; 32]);
    let g = b.node(OpCode::GlobalAvgPool, &[Val::MEL], &[]); // [3, 1, 1]
    let g = b.node(OpCode::Reshape, &[g], &[3]);
    let x = b.node(OpCode::Concat, &[g, Val::CONTEXT], &[0]); // [8]
    let w: Vec<f32> = (0..KM * W * 8)
        .map(|i| ((i % 17) as f32 - 8.0) / 512.0 + 1.0 / 1024.0)
        .collect();
    let w = b.tensor("hyp.w", &[KM * W, 8], &w);
    let bias: Vec<f32> = (0..KM * W).map(|i| 0.25 + (i % 7) as f32 / 14.0).collect();
    let bias = b.tensor("hyp.b", &[KM * W], &bias);
    let h = b.node(OpCode::Linear, &[x, w, bias], &[]);
    let h = b.node(OpCode::Reshape, &[h], &[KM as i32, W as i32]);
    let wc = b.tensor("conf.w", &[KM, 8], &[1.0 / 64.0; (KM * 8) as usize]);
    let bc: Vec<f32> = (0..KM).map(|k| k as f32 / 8.0 + 0.5).collect();
    let bc = b.tensor("conf.b", &[KM], &bc);
    let c = b.node(OpCode::Linear, &[x, wc, bc], &[]);
    b.output("hypotheses", h, &[KM, W]);
    b.output("confidence", c, &[KM]);
    if let Some(v) = extra {
        b.info("test.extra", v);
    }
    b.build()
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn model_files_are_looked_for_beside_the_plugin_then_in_the_models_folder() {
    struct Env(Vec<(&'static str, String)>);
    impl crate::presets::Environment for Env {
        fn var(&self, name: &str) -> Option<std::ffi::OsString> {
            self.0
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.into())
        }
        fn read_text(&self, _path: &Path) -> Option<String> {
            None
        }
    }
    let dir = folder("dirs");
    let env = Env(vec![("ENTH_MODELS_DIR", dir.display().to_string())]);
    let dirs = model::model_dirs(&env, crate::presets::Os::Linux);
    assert_eq!(dirs.last(), Some(&dir));
    #[cfg(unix)]
    assert_eq!(dirs.len(), 2, "beside this binary first: {dirs:?}");
    // The bank's folders: the bank is looked for in the same two places.
    let banks = create::bank_paths(&env, crate::presets::Os::Linux);
    let bank_dirs: Vec<PathBuf> = banks
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    assert_eq!(bank_dirs, dirs);
    // `<Documents>/enth/Models` without the variable.
    let home = Env(vec![("HOME", "/home/someone".to_owned())]);
    assert_eq!(
        model::model_dirs(&home, crate::presets::Os::MacOs).last(),
        Some(&PathBuf::from("/home/someone/Documents/enth/Models"))
    );
    // Which names are model files.
    for (name, is) in [
        ("enth-model-propose-v0.bin", true),
        ("enth-model-x.bin", true),
        ("ENTH-MODEL-X.BIN", true),
        ("enth-model-.bin", false),
        ("enth-bank-v1.bin", false),
        ("enth-model-x.bin.part", false),
        ("model-x.bin", false),
    ] {
        assert_eq!(model::is_model_file(name), is, "{name}");
    }
}

#[test]
fn the_first_folder_with_a_usable_model_wins_then_the_newest_header_then_the_last_name() {
    let (a, b) = (folder("choose-a"), folder("choose-b"));
    write(&b, "enth-model-zzz.bin", &small_model("in-b", 9, None));
    // In one folder: the newer header (ds1 2) beats the greater name; equal headers go to the greater name.
    write(&a, "enth-model-a.bin", &small_model("newer-data", 2, None));
    write(&a, "enth-model-b.bin", &small_model("older-data", 1, None));
    let found = model::lookup(&[a.clone(), b.clone()]);
    let loaded = found.model.as_ref().expect("a model");
    assert_eq!(loaded.model.id(), "newer-data");
    assert_eq!(loaded.path, a.join("enth-model-a.bin"));
    assert_eq!(found.line, "Model: newer-data (enth-model-a.bin)");
    assert!(
        found.notes[0].starts_with("model: newer-data from "),
        "{:?}",
        found.notes
    );
    assert!(
        found
            .notes
            .iter()
            .any(|n| n.contains("enth-model-b.bin was not used")),
        "{:?}",
        found.notes
    );
    write(&a, "enth-model-c.bin", &small_model("same-data", 2, None));
    let found = model::lookup(&[a.clone(), b.clone()]);
    assert_eq!(found.model.as_ref().unwrap().model.id(), "same-data");
    // The same folders give the same choice, whatever order the files were written in.
    for _ in 0..3 {
        assert_eq!(
            model::lookup(&[a.clone(), b.clone()])
                .model
                .unwrap()
                .model
                .id(),
            "same-data"
        );
    }
    // The other folder first: its model wins, whatever the first folder's headers.
    let found = model::lookup(&[b.clone(), a.clone()]);
    assert_eq!(found.model.as_ref().unwrap().model.id(), "in-b");
    // A folder whose files are all refused passes the choice to the next, and its refusals are noted.
    let junk = folder("choose-junk");
    write(&junk, "enth-model-junk.bin", b"not a model at all");
    let found = model::lookup(&[junk.clone(), b.clone()]);
    assert_eq!(found.model.as_ref().unwrap().model.id(), "in-b");
    assert!(
        found
            .notes
            .iter()
            .any(|n| n.contains("enth-model-junk.bin was refused (not_a_model")),
        "{:?}",
        found.notes
    );
}

#[test]
fn a_missing_or_refused_model_means_search_alone_and_says_why() {
    // Nothing there.
    let empty = folder("refused-empty");
    let found = model::lookup(std::slice::from_ref(&empty));
    assert!(found.model.is_none());
    assert_eq!(
        found.line,
        "No model: search alone (no enth-model-*.bin found)"
    );
    assert!(
        found
            .notes
            .iter()
            .any(|n| n.contains("no model file") && n.contains(&empty.display().to_string())),
        "where it looked: {:?}",
        found.notes
    );
    // One damaged file: its code and hint.
    let dir = folder("refused-one");
    let mut damaged = EXP_TINY.to_vec();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x40;
    write(&dir, "enth-model-damaged.bin", &damaged);
    let found = model::lookup(std::slice::from_ref(&dir));
    assert!(found.model.is_none());
    assert_eq!(
        found.line,
        "No model: enth-model-damaged.bin was refused (model_hash); search alone"
    );
    assert!(
        found
            .notes
            .iter()
            .any(|n| n.contains("(model_hash: ") && n.contains("download it again")),
        "{:?}",
        found.notes
    );
    // Two: counted.
    write(&dir, "enth-model-short.bin", &EXP_TINY[..300]);
    let found = model::lookup(std::slice::from_ref(&dir));
    assert_eq!(
        found.line,
        "No model: 2 model files were refused; search alone"
    );
    assert!(
        found
            .notes
            .iter()
            .any(|n| n.contains("enth-model-short.bin was refused (model_truncated")),
        "{:?}",
        found.notes
    );
}

// --- Jobs --------------------------------------------------------------------------------------------------------

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/create")
        .join(name)
}

fn shell(models: &Path, workers: usize) -> Create {
    let c = Create::new();
    c.set_bank_paths(Some(Vec::new()));
    c.set_model_dirs(Some(vec![models.to_path_buf()]));
    c.set_workers(workers);
    c
}

fn job(id: u64, k: usize, budget: usize) -> CreateRequest {
    CreateRequest::Run(CreateRun {
        id: CreateJobId(id),
        source: CreateSource::file(fixture("tone.wav")),
        settings: CreateSettings {
            candidates: k,
            seconds: 240.0,
            budget: Some(budget),
            seed: 1,
        },
    })
}

/// A job's intake post and its end: its candidates, stop and notes.
struct Ran {
    intake: CreateIntake,
    stop: CreateStop,
    candidates: Vec<Candidate>,
    notes: Vec<String>,
}

fn run_to_the_end(c: &Create, id: u64, k: usize, budget: usize) -> Ran {
    c.act(vec![job(id, k, budget)]);
    let t0 = Instant::now();
    let mut posts = Vec::new();
    while t0.elapsed() < Duration::from_secs(240) {
        posts.extend(c.take_posts());
        if posts.iter().any(|(i, p)| {
            *i == CreateJobId(id)
                && matches!(p, CreatePost::Done { .. } | CreatePost::Failed { .. })
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let intake = posts
        .iter()
        .find_map(|(_, p)| match p {
            CreatePost::Intake(i) => Some((**i).clone()),
            _ => None,
        })
        .expect("an intake post");
    match posts.pop() {
        Some((
            _,
            CreatePost::Done {
                stop,
                candidates,
                notes,
                ..
            },
        )) => Ran {
            intake,
            stop,
            candidates,
            notes,
        },
        other => panic!("the job ends with Done: {other:?}"),
    }
}

fn model_branches(candidates: &[Candidate]) -> usize {
    candidates
        .iter()
        .filter(|c| matches!(c.provenance, Provenance::Model { .. }))
        .count()
}

/// **A job with a model has model starts** (`PUIb`'s acceptance, "branches from the model"): the intake post names the
/// model, the model's hypotheses join the portfolio (K = 12 keeps every start of the 4 + 8 as a branch, so every
/// decoded hypothesis shows as a candidate), each with its model, hypothesis and confidence, and the end says what
/// the model gave.
#[test]
fn a_job_with_a_model_has_branches_from_the_model_and_says_which_model() {
    let _serial = SERIAL.lock();
    let dir = folder("job-model");
    write(&dir, "enth-model-tiny.bin", EXP_TINY);
    let c = shell(&dir, 2);
    let ran = run_to_the_end(&c, 1, 12, 60);
    assert_eq!(
        ran.intake.model.as_deref(),
        Some("Model: exp-tiny (enth-model-tiny.bin)")
    );
    assert!(
        ran.intake
            .notes
            .iter()
            .any(|n| n.starts_with("model: exp-tiny from ")),
        "{:?}",
        ran.intake.notes
    );
    assert_eq!(ran.stop, CreateStop::Budget);
    let from_model: Vec<&Candidate> = ran
        .candidates
        .iter()
        .filter(|c| matches!(c.provenance, Provenance::Model { .. }))
        .collect();
    assert!(!from_model.is_empty(), "{:?}", ran.candidates);
    for c in &from_model {
        let Provenance::Model {
            model,
            hypothesis,
            confidence,
        } = &c.provenance
        else {
            unreachable!()
        };
        assert_eq!(model, EXP_TINY_ID);
        assert!(*hypothesis < 8);
        assert!(confidence.is_some_and(f64::is_finite));
        assert!(c.distance.is_finite());
    }
    // Every decoded hypothesis became a start, and here a branch.
    let summary = ran
        .notes
        .iter()
        .find(|n| n.starts_with("model exp-tiny: "))
        .unwrap_or_else(|| panic!("the end says what the model gave: {:?}", ran.notes));
    let n = from_model.len();
    assert!(
        summary.contains(&format!(
            "{n} of its 8 hypotheses became starts, {n} of them branches (inference "
        )),
        "{summary}"
    );
    println!("{summary}");
    let dropped = ran.notes.iter().filter(|n| n.contains("dropped (")).count();
    assert!(n + dropped <= 8, "{:?}", ran.notes);
    assert_eq!(c.model_loads(), 1);
}

/// **Deterministic with the model** for any worker count: the same file, model and settings give the same candidates
/// (patches, distances, provenance and traces) with 1 and 3 workers, and with the screening round's cut (K = 6 of 12
/// starts).
#[test]
fn the_same_file_and_model_give_the_same_candidates_for_any_worker_count() {
    let _serial = SERIAL.lock();
    let dir = folder("job-determinism");
    write(&dir, "enth-model-tiny.bin", EXP_TINY);
    let one = run_to_the_end(&shell(&dir, 1), 1, 6, 90);
    let three = run_to_the_end(&shell(&dir, 3), 1, 6, 90);
    assert_eq!(one.candidates.len(), 6);
    assert_eq!(one.candidates, three.candidates);
    let summary = |r: &Ran| -> String {
        r.notes
            .iter()
            .find(|n| n.starts_with("model exp-tiny: "))
            .map(|n| n.split(" (inference").next().unwrap_or_default().to_owned())
            .unwrap_or_default()
    };
    assert_eq!(summary(&one), summary(&three));
    println!(
        "{}; {} model branches of 6",
        summary(&one),
        model_branches(&one.candidates)
    );
}

/// **Refusal → R3a with a note**: a damaged model file leaves the job to search alone (no model start), and the intake
/// post and the notes say which file was refused and why.
#[test]
fn a_refused_model_leaves_the_job_to_search_alone_and_says_why() {
    let _serial = SERIAL.lock();
    let dir = folder("job-refused");
    let mut damaged = EXP_TINY.to_vec();
    damaged[1000] ^= 1;
    write(&dir, "enth-model-damaged.bin", &damaged);
    let c = shell(&dir, 2);
    let ran = run_to_the_end(&c, 1, 4, 40);
    assert_eq!(
        ran.intake.model.as_deref(),
        Some("No model: enth-model-damaged.bin was refused (model_hash); search alone")
    );
    assert!(
        ran.intake
            .notes
            .iter()
            .any(|n| n.contains("enth-model-damaged.bin was refused (model_hash")),
        "{:?}",
        ran.intake.notes
    );
    assert_eq!(ran.stop, CreateStop::Budget);
    assert_eq!(ran.candidates.len(), 4, "the heuristic and the templates");
    assert_eq!(model_branches(&ran.candidates), 0);
    assert!(
        !ran.notes.iter().any(|n| n.starts_with("model ")),
        "{:?}",
        ran.notes
    );
}

/// **Parsed once**: two jobs on unchanged files parse the model once; a file replaced in the folder is used from the
/// next job; a file removed leaves the job to search alone.
#[test]
fn the_model_is_parsed_once_and_again_only_when_its_files_change() {
    let _serial = SERIAL.lock();
    let dir = folder("job-cache");
    let path = write(&dir, "enth-model-small.bin", &small_model("first", 1, None));
    let c = shell(&dir, 2);
    let a = run_to_the_end(&c, 1, 4, 20);
    let b = run_to_the_end(&c, 2, 4, 20);
    assert_eq!(
        a.intake.model.as_deref(),
        Some("Model: first (enth-model-small.bin)")
    );
    assert_eq!(b.intake.model, a.intake.model);
    assert_eq!(c.model_loads(), 1, "two jobs, one parse");
    std::fs::write(&path, small_model("second", 1, Some("a longer file"))).unwrap();
    let d = run_to_the_end(&c, 3, 4, 20);
    assert_eq!(
        d.intake.model.as_deref(),
        Some("Model: second (enth-model-small.bin)")
    );
    assert_eq!(c.model_loads(), 2);
    std::fs::remove_file(&path).unwrap();
    let e = run_to_the_end(&c, 4, 4, 20);
    assert_eq!(
        e.intake.model.as_deref(),
        Some("No model: search alone (no enth-model-*.bin found)")
    );
    assert_eq!(c.model_loads(), 3);
    // The cache on its own: unchanged folders are not read again.
    let mut models = Models::default();
    let (_, first) = models.get(std::slice::from_ref(&dir));
    let (_, again) = models.get(std::slice::from_ref(&dir));
    assert!(first && !again);
}
