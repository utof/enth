//! **Owner: lane EVK.** Pre-declared by lane S0s (R3).
//!
//! `cargo xtask gr3-run`: the four arms over a set of `content/gr3/` (docs/r3.md, "GR3 and its hygiene"; docs/gr3.md
//! has the protocol). They share the intake, the start set, the free set, the workers and the seeds: **A**, R3a's
//! best configuration tuned on dev (the baseline must be the best search, not GR1's 1,500-render floor); **B**, model
//! plus search from the same budget; **C**, the model's proposals unrefined (a diagnostic); **D**, A given the true
//! template (it separates structure error from parameter error). B's clock includes intake, features and inference.
//!
//! This file is the thin driver: it reads the manifest, picks the targets, **checks the freeze** (the manifest against
//! `hashes.txt`; no unfilled amendable item in a test run; a clean tree for a test run, so its build is a commit),
//! writes the job (`enth.gr3-job/1`), builds `enth-datagen` under `flock -s` and runs `enth-datagen gr3 run`, which
//! does the work with the engine crates and appends to the runs file. **Tuning happens on dev only** (dev runs go to
//! a scratch file, never to `content/gr3/runs.jsonl`); **every test run is appended to `content/gr3/runs.jsonl`**, and
//! a test run is never repeated to pick a winner.
//!
//! Do not wrap `gr3-run` in `flock`: in wall-clock mode the tool takes the quiet lock exclusively per target, and a
//! shared lock held by its own parent would deadlock it.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::runs::{self, Set};

const USAGE: &str = "usage: cargo xtask gr3-run --arm A|B|C|D [--split test|dev] [--set core|rest|available|full]
       [--ids ID,ID...] [--mode wall|renders] [--seconds 60] [--budget N] [--twin-of RUN_ID [--at MARK]]
       [--workers 8] [--bank PATH|none] [--bank-entries N] [--proposals FILE (dev only)] [--model FILE (dev only)]
       [--k 8] [--first-rung 50] [--eta 2] [--finalists 2] [--neighbours 16] [--min-spread 0.25]
       [--lambda 0] [--seed 1] [--score-seconds 3] [--marks M,M...] [--out PATH (dev only)] [--runs PATH]
       [--note TEXT] [--no-lock] [--dry-run] [--tool PATH]
       [--endpoint RENDERS --equivalent-seconds S --rate-source TEXT]   (a render run standing for a timed one)
       cargo xtask gr3-run --rate-probe   (the idle-rate probe: 8 dev targets, 60 s each under flock -x, ~10 min)
Defaults: test split, core set, wall-clock mode at 60 s, 8 workers, the bank `cargo xtask bank` writes,
create's own settings (docs/gr3.md lists the tuned defaults). Test runs append to content/gr3/runs.jsonl.
--model (arms B and C, dev): `enth-datagen gr3 proposals` writes the model's proposals for the job's targets first
(target/gr3/proposals/RUN_ID.json), then the arms run on them: the fast create eval (ml/README.md, section Fast loop).";

/// The options.
#[derive(Debug, Default)]
struct Opts {
    pairs: Vec<(String, String)>,
    flags: Vec<String>,
}

const FLAGS: [&str; 4] = ["--no-lock", "--dry-run", "--help", "--rate-probe"];

/// The idle-rate probe's targets (`--rate-probe`): eight dev targets across the levels, a two-note phrase and three
/// presets (a pad with reverb, a kick, a lead), so the probe sees short dry clips and long wet ones.
pub const PROBE_IDS: [&str; 8] = [
    "gen-d-l2-02",
    "gen-d-l3-03",
    "gen-d-l4-02",
    "gen-d-l5-03",
    "gen-d-l5-05",
    "hand-d-velvet-pad-a",
    "hand-d-thump-kick-a",
    "hand-d-solar-lead-a",
];

impl Opts {
    fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts::default();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if FLAGS.contains(&a.as_str()) {
                o.flags.push(a.clone());
                i += 1;
            } else if a.starts_with("--") {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))?;
                o.pairs.push((a.clone(), v.clone()));
                i += 2;
            } else {
                return Err(format!("unexpected argument {a}\n{USAGE}"));
            }
        }
        Ok(o)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    fn num<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("{name}: not a number: {v}")),
        }
    }

    fn check_known(&self) -> Result<(), String> {
        const KNOWN: [&str; 31] = [
            "--arm",
            "--split",
            "--set",
            "--ids",
            "--mode",
            "--seconds",
            "--budget",
            "--twin-of",
            "--at",
            "--workers",
            "--bank",
            "--bank-entries",
            "--proposals",
            "--model",
            "--k",
            "--first-rung",
            "--eta",
            "--finalists",
            "--neighbours",
            "--min-spread",
            "--lambda",
            "--seed",
            "--score-seconds",
            "--marks",
            "--out",
            "--runs",
            "--note",
            "--tool",
            "--endpoint",
            "--equivalent-seconds",
            "--rate-source",
        ];
        for (k, _) in &self.pairs {
            if !KNOWN.contains(&k.as_str()) {
                return Err(format!("unknown option {k}\n{USAGE}"));
            }
        }
        Ok(())
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The build a run records: the commit, whether the tree differs from it (the runs file itself aside), the
/// toolchain.
fn build_info(root: &Path) -> (Value, bool) {
    let commit = git(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    let status = git(root, &["status", "--porcelain"]).unwrap_or_default();
    let dirty: Vec<&str> = status
        .lines()
        .filter(|l| !l.ends_with("content/gr3/runs.jsonl"))
        .collect();
    let rustc = Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    (
        json!({
            "commit": commit,
            "dirty": !dirty.is_empty(),
            "dirty_files": dirty.iter().take(20).map(|l| l.get(3..).unwrap_or(l)).collect::<Vec<_>>(),
            "rustc": rustc,
            "profile": "release",
        }),
        !dirty.is_empty(),
    )
}

fn load1() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn now_id() -> String {
    let out = Command::new("date")
        .args(["-u", "+%Y%m%dT%H%M%SZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{out}-{:04x}", nanos & 0xffff)
}

/// Per-target budgets of a twin: each target's renders at `at` (or at the cap) in run `of`.
fn twin_budgets(
    runs_path: &Path,
    of: &str,
    at: Option<f64>,
) -> Result<(Vec<(String, usize)>, Value), String> {
    let (runs, _) = runs::read_runs(runs_path)?;
    let header = runs
        .headers
        .iter()
        .find(|r| r.run_id == of)
        .ok_or_else(|| format!("no run {of} in {}", runs_path.display()))?;
    let lines = runs
        .targets
        .get(of)
        .ok_or_else(|| format!("run {of} has no target lines"))?;
    let mut out = Vec::new();
    for t in lines {
        let c = match at {
            Some(m) => t.at(m),
            None => t.at_cap(),
        }
        .ok_or_else(|| format!("run {of}, {}: no checkpoint at {at:?}", t.target.id))?;
        out.push((t.target.id.clone(), c.renders.max(1)));
    }
    Ok((out, header.raw.clone()))
}

/// `cargo xtask gr3-run`.
pub fn gr3_run(args: &[String]) -> Result<(), String> {
    let mut o = Opts::parse(args)?;
    if o.flag("--rate-probe") {
        // The idle-rate probe (docs/gr3.md): arm A, wall-clock 60 s under the exclusive lock, 8 workers, on
        // PROBE_IDS of dev, into a scratch file. Its renders at 60 s are the machine's idle rate.
        for (k, v) in [
            ("--arm", "A".to_owned()),
            ("--split", "dev".to_owned()),
            ("--ids", PROBE_IDS.join(",")),
            ("--mode", "wall".to_owned()),
            ("--seconds", "60".to_owned()),
            ("--workers", "8".to_owned()),
            ("--note", "idle-rate probe".to_owned()),
        ] {
            if o.get(k).is_none() {
                o.pairs.push((k.to_owned(), v));
            }
        }
        if o.get("--out").is_none() {
            let out = target_dir(&root()).join("gr3/rate-probe.jsonl");
            o.pairs
                .push(("--out".to_owned(), out.display().to_string()));
        }
    }
    if o.flag("--help") {
        println!("{USAGE}");
        return Ok(());
    }
    o.check_known()?;
    let root = root();
    let gr3 = runs::gr3_dir(&root);
    let tdir = target_dir(&root);

    let twin = match o.get("--twin-of") {
        Some(of) => {
            let runs_path = o
                .get("--runs")
                .map_or_else(|| gr3.join("runs.jsonl"), PathBuf::from);
            let at = o
                .get("--at")
                .map(str::parse::<f64>)
                .transpose()
                .map_err(|_| "--at: not a number")?;
            Some((of.to_owned(), twin_budgets(&runs_path, of, at)?))
        }
        None => None,
    };
    // A twin copies its run's arm, split, settings and bank unless told otherwise.
    let base = twin.as_ref().map(|(_, (_, h))| h.clone());
    let from_base = |key: &str| base.as_ref().and_then(|b| b.get(key)).cloned();

    let arm = o
        .get("--arm")
        .map(str::to_owned)
        .or_else(|| from_base("arm").and_then(|v| v.as_str().map(str::to_owned)))
        .ok_or_else(|| format!("--arm is required\n{USAGE}"))?;
    if !["A", "B", "C", "D"].contains(&arm.as_str()) {
        return Err(format!("--arm {arm}: one of A, B, C, D"));
    }
    let split = o
        .get("--split")
        .map(str::to_owned)
        .or_else(|| from_base("split").and_then(|v| v.as_str().map(str::to_owned)))
        .unwrap_or_else(|| "test".to_owned());
    if split != "test" && split != "dev" {
        return Err(format!("--split {split}: test or dev"));
    }
    let set = Set::parse(o.get("--set").unwrap_or("core"))?;
    let mode = if twin.is_some() {
        "renders".to_owned()
    } else {
        o.get("--mode").unwrap_or("wall").to_owned()
    };
    if mode != "wall" && mode != "renders" {
        return Err(format!("--mode {mode}: wall or renders"));
    }
    let test = split == "test";

    // ---- the hygiene of a test run ----
    if test {
        if o.get("--out").is_some() {
            return Err(
                "a test run always appends to content/gr3/runs.jsonl: --out is for dev runs"
                    .to_owned(),
            );
        }
        if o.get("--proposals").is_some() || o.get("--model").is_some() {
            return Err(
                "a proposals file is a dry run of arms B and C, never a GR3 result: use it on dev"
                    .to_owned(),
            );
        }
    }
    // --model: the proposals file is written by `enth-datagen gr3 proposals` before the arms run (lane FTL).
    let model = o.get("--model").map(PathBuf::from);
    if let Some(m) = &model {
        if o.get("--proposals").is_some() {
            return Err("--model writes the proposals file itself: drop --proposals".to_owned());
        }
        if arm != "B" && arm != "C" {
            return Err(format!(
                "--model is for arms B and C (arm {arm} uses no proposals)"
            ));
        }
        if !m.is_file() {
            return Err(format!("no model file at {}", m.display()));
        }
    }
    let synth_sha = runs::check_freeze(&gr3)?;
    let manifest = runs::read_manifest(&gr3.join("synth.toml"))?;
    let ids: Option<Vec<String>> = match (&twin, o.get("--ids")) {
        (Some((_, (b, _))), None) => Some(b.iter().map(|(id, _)| id.clone()).collect()),
        (_, Some(s)) => Some(s.split(',').map(|x| x.trim().to_owned()).collect()),
        (None, None) => None,
    };
    let picked = runs::select(&manifest, &split, set, ids.as_deref())?;
    let (build, dirty) = build_info(&root);
    if test && dirty && !o.flag("--dry-run") {
        return Err(
            "a test run needs a clean tree (its build must be a commit): commit or stash first"
                .to_owned(),
        );
    }

    // ---- settings ----
    let base_cfg = from_base("cfg").unwrap_or(Value::Null);
    let cfg_num = |opt: &str, key: &str, default: f64| -> Result<f64, String> {
        match o.get(opt) {
            Some(v) => v.parse().map_err(|_| format!("{opt}: not a number: {v}")),
            None => Ok(base_cfg.get(key).and_then(Value::as_f64).unwrap_or(default)),
        }
    };
    let cfg = json!({
        "k": cfg_num("--k", "k", 8.0)? as usize,
        "first_rung": cfg_num("--first-rung", "first_rung", 50.0)? as usize,
        "eta": cfg_num("--eta", "eta", 2.0)? as usize,
        "finalists": cfg_num("--finalists", "finalists", 2.0)? as usize,
        "neighbours": cfg_num("--neighbours", "neighbours", 16.0)? as usize,
        "min_spread": cfg_num("--min-spread", "min_spread", 0.25)?,
        "lambda": cfg_num("--lambda", "lambda", 0.0)? as usize,
        "seed": cfg_num("--seed", "seed", 1.0)? as u64,
        "score_s": cfg_num("--score-seconds", "score_s", 3.0)?,
    });
    let workers: usize = o.num(
        "--workers",
        from_base("workers")
            .and_then(|v| v.as_u64())
            .map_or(8, |v| v as usize),
    )?;
    let base_bank = from_base("bank");
    let bank: Option<PathBuf> = match o.get("--bank") {
        Some("none") => None,
        Some(p) => Some(PathBuf::from(p)),
        None => match &base_bank {
            Some(Value::Null) => None,
            Some(b) => b.get("path").and_then(Value::as_str).map(PathBuf::from),
            None => Some(tdir.join("bank/enth-bank-v1.bin")),
        },
    };
    if let Some(b) = &bank
        && !b.is_file()
    {
        return Err(format!(
            "no bank at {}: build it with `cargo xtask bank`, pass --bank PATH, or run without one (--bank none)",
            b.display()
        ));
    }
    let bank_entries: Option<usize> = match o.get("--bank-entries") {
        Some(v) => Some(
            v.parse()
                .map_err(|_| format!("--bank-entries: not a number: {v}"))?,
        ),
        None => base_bank
            .as_ref()
            .and_then(|b| b.get("entries"))
            .and_then(Value::as_u64)
            .map(|v| v as usize),
    };
    let seconds: f64 = o.num("--seconds", 60.0)?;
    let budget: Option<usize> = o
        .get("--budget")
        .map(str::parse)
        .transpose()
        .map_err(|_| "--budget: not a number")?;
    if mode == "renders" && budget.is_none() && twin.is_none() {
        return Err("render mode needs --budget N (or --twin-of RUN_ID)".to_owned());
    }
    let marks: Vec<f64> = match o.get("--marks") {
        Some(s) => s
            .split(',')
            .map(|x| {
                x.trim()
                    .parse()
                    .map_err(|_| format!("--marks: bad number {x}"))
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    // A render-capped run that stands for a wall-clock one records the assumption, so the conversion can be applied
    // (or replaced by a measured idle rate) later without running again.
    let equivalence = match o.get("--equivalent-seconds") {
        Some(sec) => {
            if mode != "renders" {
                return Err("--equivalent-seconds is for render mode".to_owned());
            }
            let sec: f64 = sec
                .parse()
                .map_err(|_| format!("--equivalent-seconds: not a number: {sec}"))?;
            let endpoint: usize = o.num("--endpoint", budget.unwrap_or(0))?;
            if endpoint == 0 || sec <= 0.0 {
                return Err(
                    "--equivalent-seconds needs a positive time and an --endpoint (renders)"
                        .to_owned(),
                );
            }
            let source = o.get("--rate-source").ok_or(
                "--equivalent-seconds needs --rate-source (where the assumed rate comes from)",
            )?;
            json!({
                "kind": "estimate",
                "seconds": sec,
                "renders": endpoint,
                "rate_renders_per_s": endpoint as f64 / sec,
                "source": source,
            })
        }
        None => Value::Null,
    };
    let out = if test {
        gr3.join("runs.jsonl")
    } else {
        o.get("--out")
            .map_or_else(|| tdir.join("gr3/dev-runs.jsonl"), PathBuf::from)
    };
    let set_name = if ids.is_some() {
        "ids"
    } else if test {
        set.name()
    } else {
        "dev"
    };
    let cap = if mode == "wall" {
        format!("wall{seconds}")
    } else if twin.is_some() {
        "twin".to_owned()
    } else {
        format!("renders{}", budget.unwrap_or(0))
    };
    let run_id = format!("{}-{arm}-{cap}-{split}-{set_name}", now_id());
    let twin_of = twin.as_ref().map(|(of, _)| of.clone());
    let proposals_out = model
        .as_ref()
        .map(|_| tdir.join(format!("gr3/proposals/{run_id}.json")));
    let proposals: Option<String> = proposals_out
        .as_ref()
        .map(|p| p.display().to_string())
        .or_else(|| o.get("--proposals").map(str::to_owned));
    let budgets: std::collections::BTreeMap<String, usize> = twin
        .map(|(_, (b, _))| b.into_iter().collect())
        .unwrap_or_default();

    let targets: Vec<Value> = picked
        .iter()
        .map(|t| {
            let mut v = json!({
                "id": t.id,
                "split": t.split,
                "kind": if t.kind == "generator" { "generator" } else { "preset" },
                "core": t.core.unwrap_or(false),
            });
            if let Some(l) = t.level {
                v["level"] = json!(l);
            }
            if let Some(p) = t.phrase {
                v["phrase"] = json!(p);
            }
            if let Some(c) = &t.code {
                v["code"] = json!(c);
            }
            if let Some(f) = &t.fingerprint {
                v["fingerprint"] = json!(f);
            }
            if let Some(f) = &t.file {
                v["file"] = json!(root.join(f));
            }
            if let Some(s) = &t.sha256 {
                v["sha256"] = json!(s);
            }
            if let Some(p) = &t.perf {
                v["perf"] = json!(p);
            }
            if let Some(b) = budgets.get(&t.id) {
                v["budget"] = json!(b);
            }
            v
        })
        .collect();
    let header = json!({
        "build": build,
        "set": {
            "file": "content/gr3/synth.toml",
            "set_version": manifest.set_version,
            "sha256": synth_sha,
            "set": set_name,
            "ids": ids,
        },
        "twin_of": twin_of,
        "command": std::iter::once("cargo xtask gr3-run".to_owned()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "),
        "note": o.get("--note"),
        "equivalence": equivalence,
        "load1_at_start": load1(),
    });
    let job = json!({
        "schema": "enth.gr3-job/1",
        "run_id": run_id,
        "out": out,
        "patches_out": tdir.join(format!("gr3/patches/{run_id}.jsonl")),
        "arm": arm,
        "mode": mode,
        "seconds": (mode == "wall").then_some(seconds),
        "budget": budget,
        "marks": marks,
        "workers": workers,
        "lock": (mode == "wall" && !o.flag("--no-lock")).then(|| PathBuf::from("/tmp/enth-quiet.lock")),
        "cfg": cfg,
        "bank": bank,
        "bank_entries": bank_entries,
        "proposals": proposals,
        "split": split,
        "header": header,
        "targets": targets,
    });
    let job_path = tdir.join(format!("gr3/jobs/{run_id}.json"));
    std::fs::create_dir_all(job_path.parent().unwrap_or(&tdir))
        .map_err(|e| format!("cannot create {}: {e}", tdir.display()))?;
    std::fs::write(
        &job_path,
        serde_json::to_string_pretty(&job).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("cannot write {}: {e}", job_path.display()))?;
    println!(
        "gr3-run: {run_id}: arm {arm}, {mode}, {} targets of {split}, {workers} workers, out {}",
        picked.len(),
        out.display()
    );
    println!("gr3-run: job {}", job_path.display());
    if let (Some(m), Some(p)) = (&model, &proposals_out) {
        println!(
            "gr3-run: proposals of {} -> {} (enth-datagen gr3 proposals)",
            m.display(),
            p.display()
        );
    }
    if o.flag("--dry-run") {
        for t in &picked {
            println!("  {}", t.id);
        }
        return Ok(());
    }

    // ---- build (shared lock), then run (the tool locks per target in wall mode) ----
    if let Some(tool) = o.get("--tool") {
        // A frozen tool binary (a long sweep keeps one build however the tree moves meanwhile).
        if let (Some(m), Some(p)) = (&model, &proposals_out) {
            run_proposals(Path::new(tool), &job_path, m, p)?;
        }
        return run_tool(Path::new(tool), &job_path, &out);
    }
    let status = Command::new("flock")
        .args(["-s", "/tmp/enth-quiet.lock"])
        .arg(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "build",
            "--quiet",
            "--release",
            "--package",
            "enth-datagen",
            "--bin",
            "enth-datagen",
        ])
        .status()
        .map_err(|e| format!("cannot run flock and cargo build: {e}"))?;
    if !status.success() {
        return Err("building enth-datagen failed (report above)".to_owned());
    }
    let bin = tdir.join("release/enth-datagen");
    if let (Some(m), Some(p)) = (&model, &proposals_out) {
        run_proposals(&bin, &job_path, m, p)?;
    }
    run_tool(&bin, &job_path, &out)
}

/// `enth-datagen gr3 proposals`: the model's proposals for the job's targets, before the arms run (`--model`).
fn run_proposals(bin: &Path, job_path: &Path, model: &Path, out: &Path) -> Result<(), String> {
    let side = out.with_extension("side.json");
    let status = Command::new(bin)
        .args(["gr3", "proposals", "--job"])
        .arg(job_path)
        .arg("--model")
        .arg(model)
        .arg("--out")
        .arg(out)
        .arg("--side")
        .arg(&side)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err("enth-datagen gr3 proposals: failed (report above)".to_owned())
    }
}

fn run_tool(bin: &Path, job_path: &Path, out: &Path) -> Result<(), String> {
    let status = Command::new(bin)
        .args(["gr3", "run", "--job"])
        .arg(job_path)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "enth-datagen gr3 run: failed (report above; whatever ran is recorded in {})",
            out.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    //! `--model` (lane FTL): refused where a proposals file is (a test run), with `--proposals`, for arms without
    //! proposals, and for a missing file; each refusal comes before any file of `content/gr3/` is read.
    use super::gr3_run;

    fn run(args: &[&str]) -> String {
        let v: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        gr3_run(&v).unwrap_err()
    }

    #[test]
    fn model_refusals() {
        assert!(run(&["--arm", "B", "--model", "m.bin"]).contains("never a GR3 result"));
        assert!(
            run(&[
                "--arm",
                "B",
                "--split",
                "dev",
                "--model",
                "m.bin",
                "--proposals",
                "p.json"
            ])
            .contains("drop --proposals")
        );
        assert!(
            run(&["--arm", "A", "--split", "dev", "--model", "m.bin"]).contains("arms B and C")
        );
        assert!(
            run(&[
                "--arm",
                "C",
                "--split",
                "dev",
                "--model",
                "/nonexistent/enth-model.bin"
            ])
            .contains("no model file")
        );
        assert!(run(&["--arm", "B", "--modle", "m.bin"]).contains("unknown option --modle"));
    }
}
