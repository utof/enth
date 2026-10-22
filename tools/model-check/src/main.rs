//! **Owner: lane ONX** (R3b; docs/r3.md, "Our own scalar kernels (lane ONX)"); lane EXP added batch inputs, several
//! seeds and `--decode` (the `PyTorch` parity of the exported model).
//!
//! `enth-model-check`: a dev tool for model files (`enth-model-<id>.bin`, `ENTHMODL` format 1). A tooling package,
//! exempt from the layer rules: it reads files and clocks, and runs `enth-solver`'s pure inference on them.
//!
//! ```text
//! enth-model-check info  <model.bin>
//! enth-model-check run   <model.bin> [--seed N[,N...] | --inputs <file>] [--expected <file>] [--tolerance 1e-5]
//!                         [--write <file>] [--decode [--decode-tolerance 1e-4]]
//! enth-model-check bench [--gflop 0.5]... [--runs 5]
//! ```
//!
//! - `info` prints the header, the tables and what [`Net::build`] says: the FLOP recount, the peak memory, or the
//!   refusal (its code and hint, as create would report it).
//! - `run` runs the model on a synthetic input (`--seed`, a comma-separated list for a batch; else the model's
//!   `parity.seeds` or `parity.seed` info entry, else 0) or on `--inputs` (`mel` and `context` in a raw tensor file:
//!   `[3, 128, 64]` and `[5]` for one sample, `[N, 3, 128, 64]` and `[N, 5]` for a batch, whose outputs gain a leading
//!   `N`), prints each output and the outputs' SHA-256 (what an output golden pins), and with `--expected` the largest
//!   absolute difference per output against a raw tensor file (ONNX Runtime's, from `ml/enth_ml/export/parity.py`, or
//!   `PyTorch`'s, from `ml/enth_ml/export/propose.py`): exit 1 above `--tolerance` (default 1e-5). `--write` saves our
//!   outputs as a raw tensor file, so `ml/` can compare them against its own run. `--decode` decodes every hypothesis
//!   row (`OutputLayout::decode`, under the hearing a create job would have for that sample's `context`, as told and
//!   with the key shifted by 3) and, with `--expected`, requires the same drops, the same modules, categoricals and
//!   routes, and values within `--decode-tolerance` (normalised domain, default 1e-4); it also prints how close any
//!   decision comes to flipping and whether the confidence order agrees.
//! - `bench` builds conv-stack models of about each `--gflop` (default 0.5 and 2, docs/r3.md's targets) and times
//!   `Net::run` (the median of `--runs`): information only, wall-clock (docs/benchmarks.md: instruction counts are the
//!   gate). Run it alone on the machine (`flock -x /tmp/enth-quiet.lock`).
//!
//! The raw tensor file (`ENTHTNS1`, `ml/enth_ml/export/tensors.py`): the magic, a `u32` count, then per tensor its
//! name (`u8` length, ASCII), rank (`u8`), dims (`u32` each) and values (`f32`), little-endian.
//!
//! Exit codes: 0 done, 1 a refused model or a parity failure, 2 a usage or file error.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool: reports go to stdout and stderr"
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use enth_solver::create::TargetPerf;
use enth_solver::propose::decode::{Hearing, HearingError, OutputLayout};
use enth_solver::propose::features::{Cue, ModelInput};
use enth_solver::propose::net::{Net, Outputs, synthetic_input};
use enth_solver::propose::pack::{ModelBuilder, ModelPack, OpCode, PackError, Val};

/// One tensor of a raw tensor file.
#[derive(Clone)]
struct Tensor {
    name: String,
    dims: Vec<u32>,
    values: Vec<f32>,
}

fn read_tensors(b: &[u8]) -> Result<Vec<Tensor>, String> {
    if b.len() < 12 || &b[..8] != b"ENTHTNS1" {
        return Err("not a raw tensor file (ENTHTNS1)".to_owned());
    }
    let mut at = 8usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = b.get(at..at + n).ok_or("the tensor file is cut short")?;
        at += n;
        Ok(s)
    };
    let u32_of = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let count = u32_of(take(4)?);
    let mut out = Vec::new();
    for _ in 0..count {
        let n = usize::from(take(1)?[0]);
        let name = String::from_utf8(take(n)?.to_vec()).map_err(|_| "a tensor name is not text")?;
        let rank = usize::from(take(1)?[0]);
        let mut dims = Vec::with_capacity(rank);
        for _ in 0..rank {
            dims.push(u32_of(take(4)?));
        }
        let len = dims
            .iter()
            .try_fold(1usize, |acc, d| acc.checked_mul(*d as usize))
            .ok_or("a tensor's size overflows")?;
        let bytes = take(len.checked_mul(4).ok_or("a tensor's size overflows")?)?;
        let values = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        out.push(Tensor { name, dims, values });
    }
    Ok(out)
}

fn write_tensors(tensors: &[Tensor]) -> Vec<u8> {
    let mut out = b"ENTHTNS1".to_vec();
    out.extend_from_slice(&(tensors.len() as u32).to_le_bytes());
    for t in tensors {
        out.push(t.name.len() as u8);
        out.extend_from_slice(t.name.as_bytes());
        out.push(t.dims.len() as u8);
        for d in &t.dims {
            out.extend_from_slice(&d.to_le_bytes());
        }
        for v in &t.values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

fn hex(b: &[u8]) -> String {
    use core::fmt::Write as _;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: enth-model-check info <model.bin>\n       enth-model-check run <model.bin> [--seed N[,N...] | --inputs FILE] [--expected FILE] [--tolerance 1e-5] [--write FILE] [--decode [--decode-tolerance 1e-4]]\n       enth-model-check bench [--gflop G]... [--runs N]"
    );
    ExitCode::from(2)
}

fn refused(e: &PackError) -> ExitCode {
    println!("refused: {} ({e})", e.code());
    println!("hint: {}", e.hint());
    ExitCode::from(1)
}

fn load(path: &PathBuf) -> Result<Vec<u8>, ExitCode> {
    std::fs::read(path).map_err(|e| {
        eprintln!("enth-model-check: cannot read {}: {e}", path.display());
        ExitCode::from(2)
    })
}

fn info(path: &PathBuf) -> ExitCode {
    let bytes = match load(path) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let pack = match ModelPack::parse(&bytes) {
        Ok(p) => p,
        Err(e) => return refused(&e),
    };
    let h = pack.header();
    println!(
        "file: {} ({} bytes, sha256 {})",
        path.display(),
        bytes.len(),
        hex(&pack.sha256())
    );
    println!(
        "format {}, op set {}, feature set {}, layout {} (generator {}, template {}), K {}, H {}",
        h.format, h.op_set, h.feature, h.layout, h.generator, h.template, h.hypotheses, h.width
    );
    println!(
        "model id {:?}, engine {:?}, ds1 version {}",
        h.model_id, h.engine, h.ds1
    );
    println!(
        "parameters {}, header FLOPs {} (information only)",
        h.parameters, h.flops
    );
    println!(
        "tables: {} inputs, {} tensors, {} nodes, {} outputs, {} info entries",
        h.inputs, h.tensors, h.nodes, h.outputs, h.info
    );
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for n in pack.nodes() {
        let name = n.op.info().name;
        match counts.iter_mut().find(|(k, _)| *k == name) {
            Some((_, c)) => *c += 1,
            None => counts.push((name, 1)),
        }
    }
    let ops: Vec<String> = counts.iter().map(|(k, c)| format!("{k} {c}")).collect();
    println!("ops: {}", ops.join(", "));
    for o in pack.outputs() {
        println!("output {} {:?}", o.name, o.dims);
    }
    for (k, v) in pack.info() {
        println!("info {k} = {v:?}");
    }
    match Net::build(&pack) {
        Ok(net) => {
            println!(
                "net: {} FLOPs recounted ({:.3} GFLOP), at most {} f32 elements alive at once",
                net.flops(),
                net.flops() as f64 / 1e9,
                net.peak_elements()
            );
            ExitCode::SUCCESS
        }
        Err(e) => refused(&e),
    }
}

struct RunArgs {
    model: PathBuf,
    seeds: Option<Vec<u64>>,
    inputs: Option<PathBuf>,
    expected: Option<PathBuf>,
    tolerance: f32,
    write: Option<PathBuf>,
    decode: bool,
    decode_tolerance: f32,
}

/// The samples of a run: one (`batched` false: outputs keep their model dims) or several (outputs gain a leading `N`).
struct Samples {
    inputs: Vec<ModelInput>,
    batched: bool,
}

fn read_tensor_file(path: &PathBuf) -> Result<Vec<Tensor>, ExitCode> {
    match load(path).map(|b| read_tensors(&b)) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => {
            eprintln!("enth-model-check: {}: {e}", path.display());
            Err(ExitCode::from(2))
        }
        Err(c) => Err(c),
    }
}

/// `mel` `[3, 128, 64]` and `context` `[5]` (one sample), or `[N, 3, 128, 64]` and `[N, 5]` (a batch) from a tensor file.
fn samples_from_file(path: &PathBuf) -> Result<Samples, ExitCode> {
    let t = read_tensor_file(path)?;
    let bad = |why: &str| {
        eprintln!("enth-model-check: {}: {why}", path.display());
        ExitCode::from(2)
    };
    let (Some(mel), Some(ctx)) = (
        t.iter().find(|x| x.name == "mel"),
        t.iter().find(|x| x.name == "context"),
    ) else {
        return Err(bad("needs tensors mel and context"));
    };
    let (mel_len, batched) = match mel.dims.as_slice() {
        [3, 128, 64] => (3 * 128 * 64, false),
        [_, 3, 128, 64] => (3 * 128 * 64, true),
        _ => return Err(bad("mel is [3, 128, 64] or [N, 3, 128, 64]")),
    };
    let n = if batched { mel.dims[0] as usize } else { 1 };
    let want: &[u32] = if batched { &[n as u32, 5] } else { &[5] };
    if ctx.dims != want || mel.values.len() != n * mel_len {
        return Err(bad("context is [5] or [N, 5], one row per mel"));
    }
    let inputs = (0..n)
        .map(|i| {
            let mut context = [0.0f32; 5];
            context.copy_from_slice(&ctx.values[i * 5..(i + 1) * 5]);
            ModelInput {
                mel: mel.values[i * mel_len..(i + 1) * mel_len].to_vec(),
                context,
            }
        })
        .collect();
    Ok(Samples { inputs, batched })
}

/// The outputs of every sample as tensors: as the model declares them, or with a leading `N` for a batch.
fn combine(runs: &[Outputs], batched: bool) -> Vec<Tensor> {
    let first = runs[0].all();
    (0..first.len())
        .map(|j| {
            let mut dims = first[j].dims.clone();
            if batched {
                dims.insert(0, runs.len() as u32);
            }
            Tensor {
                name: first[j].name.clone(),
                dims,
                values: runs
                    .iter()
                    .flat_map(|r| r.all()[j].values.iter().copied())
                    .collect(),
            }
        })
        .collect()
}

/// Hypothesis indices from the most to the least confident (ties: the lower index), as create orders proposals.
fn order(conf: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..conf.len()).collect();
    idx.sort_by(|&a, &b| conf[b].total_cmp(&conf[a]).then(a.cmp(&b)));
    idx
}

/// The hearing a create job would have for a sample whose model input is told `context`: the intake's cue rebuilt from
/// the context (key `60 + context[0]`, the pitch offset when `context[2]` says pitched, the hold capped at 4 s), a
/// scored window of 3 s from an onset 7 ms in, and one note on the told key plus `key_shift` (a `--perf` override).
fn job_hearing(context: &[f32; 5], key_shift: i32) -> Result<Hearing, HearingError> {
    let key = (context[0].round() as i32 + 60).clamp(0, 127);
    let pitched = context[2] > 0.5;
    let f0_hz = pitched
        .then(|| 440.0 * ((f64::from(key) - 69.0) / 12.0 + f64::from(context[1]) / 12.0).exp2());
    let hold_s = f64::from(context[3]).clamp(0.08, 4.0);
    let cue = Cue {
        key: key as u8,
        f0_hz,
        on_s: 0.007,
        hold_s,
        bandwidth_hz: None,
    };
    let perf = TargetPerf {
        key: (key + key_shift).clamp(0, 127) as u8,
        velocity: 100.0 / 127.0,
        on_s: 0.007,
        hold_s,
        duration_s: 3.0,
        f0_hz,
    };
    Hearing::job(&cue, &perf)
}

/// Decodes every hypothesis row of `ours` (and of `expected`, when given) under a job's hearing and compares the
/// patches. Returns whether every row agrees: the same drops, and in every kept row the same modules, categoricals and
/// routes, and every continuous value (and route depth) within `tolerance` in the normalised domain.
fn decode_report(
    layout: &OutputLayout,
    ours: &Tensor,
    expected: Option<&Tensor>,
    contexts: &[[f32; 5]],
    tolerance: f32,
) -> bool {
    let h = layout.width();
    let rows_per = ours.dims[ours.dims.len() - 2] as usize;
    let mut all_ok = true;
    for (mode, shift) in [("as told", 0), ("key +3 (a perf override)", 3)] {
        let (mut rows, mut exact, mut close, mut dropped, mut mixed, mut discrete, mut over) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let mut worst = 0.0f32;
        let mut first_problem: Option<String> = None;
        for (n, context) in contexts.iter().enumerate() {
            let Ok(hearing) = job_hearing(context, shift) else {
                continue;
            };
            for k in 0..rows_per {
                let at = (n * rows_per + k) * h;
                let a = layout.decode(&ours.values[at..at + h], &hearing);
                rows += 1;
                let Some(exp) = expected else {
                    if a.is_err() {
                        dropped += 1;
                    } else {
                        exact += 1;
                    }
                    continue;
                };
                let b = layout.decode(&exp.values[at..at + h], &hearing);
                let (pa, pb) = match (a, b) {
                    (Err(x), Err(y)) if x.code() == y.code() => {
                        dropped += 1;
                        continue;
                    }
                    (Ok(pa), Ok(pb)) => (pa, pb),
                    (x, y) => {
                        mixed += 1;
                        first_problem.get_or_insert_with(|| {
                            format!(
                                "sample {n} hypothesis {k}: ours {:?}, expected {:?}",
                                x.map(|_| "a patch").map_err(|e| e.code()),
                                y.map(|_| "a patch").map_err(|e| e.code())
                            )
                        });
                        continue;
                    }
                };
                if pa == pb {
                    exact += 1;
                    continue;
                }
                let mut why = None;
                let mut row_worst = 0.0f32;
                let modules_a: Vec<_> = pa
                    .modules()
                    .map(|(_, m)| (m.id.clone(), m.enabled))
                    .collect();
                let modules_b: Vec<_> = pb
                    .modules()
                    .map(|(_, m)| (m.id.clone(), m.enabled))
                    .collect();
                if modules_a != modules_b {
                    why = Some("the modules switched on differ".to_owned());
                }
                for e in layout.categorical() {
                    if pa.value(&e.addr) != pb.value(&e.addr) {
                        why.get_or_insert_with(|| format!("{} differs", e.addr));
                    }
                }
                if pa.routes.len() == pb.routes.len() {
                    for (x, y) in pa.routes.iter().zip(&pb.routes) {
                        if x.src != y.src || x.dst != y.dst {
                            why.get_or_insert_with(|| "the routes differ".to_owned());
                        }
                        row_worst = row_worst.max((x.depth - y.depth).abs() / 100.0);
                    }
                } else {
                    why.get_or_insert_with(|| "the routes differ".to_owned());
                }
                for e in layout.continuous() {
                    if let (Some(x), Some(y)) = (pa.value(&e.addr), pb.value(&e.addr)) {
                        row_worst = row_worst.max((e.spec.to_norm(x) - e.spec.to_norm(y)).abs());
                    }
                }
                worst = worst.max(row_worst);
                if let Some(w) = why {
                    discrete += 1;
                    first_problem.get_or_insert_with(|| format!("sample {n} hypothesis {k}: {w}"));
                } else if row_worst > tolerance {
                    over += 1;
                    first_problem.get_or_insert_with(|| {
                        format!("sample {n} hypothesis {k}: a value is {row_worst:.3e} apart")
                    });
                } else {
                    close += 1;
                }
            }
        }
        let ok = mixed == 0 && discrete == 0 && over == 0;
        all_ok &= ok;
        if expected.is_some() {
            println!(
                "decode, {mode}: {rows} rows: {exact} bit-identical patches, {close} equal in modules, categoricals and routes with values within {tolerance:.0e}, {dropped} dropped by both, {discrete} differ in a decision, {over} over the tolerance, {mixed} dropped by one only; worst normalised difference {worst:.3e}: {}",
                if ok { "pass" } else { "FAIL" }
            );
        } else {
            println!("decode, {mode}: {rows} rows: {exact} decoded, {dropped} dropped");
        }
        if let Some(p) = first_problem {
            println!("  first problem: {p}");
        }
    }
    all_ok
}

/// How close any of our rows comes to a decision flipping: the smallest `|logit|` of a switch or a route's presence,
/// and the smallest gap between a categorical group's best and second-best logit.
fn decision_margin(layout: &OutputLayout, rows: &Tensor) -> (f32, f32) {
    let h = layout.width();
    let mut sign = f32::INFINITY;
    let mut gap = f32::INFINITY;
    for row in rows.values.chunks_exact(h) {
        for e in layout.enables() {
            sign = sign.min(row[e.offset].abs());
        }
        for c in layout.routes() {
            sign = sign.min(row[c.presence].abs());
        }
        for e in layout.categorical() {
            let mut v: Vec<f32> = row[e.offset..e.offset + e.len()].to_vec();
            v.sort_by(|a, b| b.total_cmp(a));
            gap = gap.min(v[0] - v[1]);
        }
    }
    (sign, gap)
}

fn run(a: &RunArgs) -> ExitCode {
    let bytes = match load(&a.model) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let pack = match ModelPack::parse(&bytes) {
        Ok(p) => p,
        Err(e) => return refused(&e),
    };
    let net = match Net::build(&pack) {
        Ok(n) => n,
        Err(e) => return refused(&e),
    };
    let samples = if let Some(path) = &a.inputs {
        let s = match samples_from_file(path) {
            Ok(s) => s,
            Err(c) => return c,
        };
        println!("input: {} ({} samples)", path.display(), s.inputs.len());
        s
    } else {
        let seeds = a
            .seeds
            .clone()
            .or_else(|| {
                pack.info_value("parity.seeds")
                    .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            })
            .or_else(|| {
                pack.info_value("parity.seed")
                    .and_then(|s| s.parse().ok())
                    .map(|s| vec![s])
            })
            .unwrap_or_else(|| vec![0]);
        println!("input: synthetic, seed {seeds:?}");
        Samples {
            batched: seeds.len() > 1,
            inputs: seeds.iter().map(|s| synthetic_input(*s)).collect(),
        }
    };
    let started = Instant::now();
    let mut runs = Vec::with_capacity(samples.inputs.len());
    for input in &samples.inputs {
        match net.run(input) {
            Ok(o) => runs.push(o),
            Err(e) => {
                eprintln!("enth-model-check: {} ({e})", e.code());
                return ExitCode::from(2);
            }
        }
    }
    if runs.is_empty() {
        eprintln!("enth-model-check: no input sample");
        return ExitCode::from(2);
    }
    let ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "ran {} ({:.3} GFLOP) on {} sample(s) in {ms:.1} ms, {:.1} ms each (wall clock, information only)",
        a.model.display(),
        net.flops() as f64 / 1e9,
        runs.len(),
        ms / runs.len() as f64
    );
    let out = combine(&runs, samples.batched);
    for o in &out {
        let finite = o.values.iter().all(|v| v.is_finite());
        println!(
            "output {} {:?}{}",
            o.name,
            o.dims,
            if finite { "" } else { " NON-FINITE" }
        );
    }
    if samples.batched {
        println!(
            "outputs sha256 of sample 0 {} (hash a --write file for the whole batch)",
            hex(&runs[0].sha256())
        );
    } else {
        println!("outputs sha256 {}", hex(&runs[0].sha256()));
    }
    if let Some(path) = &a.write {
        if let Err(e) = std::fs::write(path, write_tensors(&out)) {
            eprintln!("enth-model-check: cannot write {}: {e}", path.display());
            return ExitCode::from(2);
        }
        println!("wrote {}", path.display());
    }
    let expected = match &a.expected {
        Some(path) => match read_tensor_file(path) {
            Ok(t) => Some(t),
            Err(c) => return c,
        },
        None => None,
    };
    let mut ok = true;
    if let Some(expected) = &expected {
        let mut worst = 0.0f32;
        for t in expected {
            let Some(ours) = out.iter().find(|o| o.name == t.name) else {
                println!("parity {}: the model has no such output", t.name);
                ok = false;
                continue;
            };
            if ours.dims != t.dims {
                println!(
                    "parity {}: dims {:?}, expected {:?}",
                    t.name, ours.dims, t.dims
                );
                ok = false;
                continue;
            }
            let mut w = 0.0f32;
            for (x, y) in ours.values.iter().zip(&t.values) {
                let d = (x - y).abs();
                w = if d.is_nan() { f32::INFINITY } else { w.max(d) };
            }
            println!("parity {}: max |ours - expected| = {w:.3e}", t.name);
            worst = worst.max(w);
        }
        ok &= worst <= a.tolerance;
        println!(
            "parity: max {worst:.3e} against a tolerance of {:.0e}: {}",
            a.tolerance,
            if ok { "pass" } else { "FAIL" }
        );
    }
    if a.decode {
        let h = pack.header();
        let layout = match OutputLayout::build(h.layout, h.generator, h.template) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("enth-model-check: the layout does not build: {e}");
                return ExitCode::from(2);
            }
        };
        let Some(hyp) = out.iter().find(|o| o.name == "hypotheses") else {
            eprintln!("enth-model-check: the model has no hypotheses output");
            return ExitCode::from(2);
        };
        let contexts: Vec<[f32; 5]> = samples.inputs.iter().map(|i| i.context).collect();
        let exp_hyp = expected
            .as_ref()
            .and_then(|e| e.iter().find(|t| t.name == "hypotheses"));
        if expected.is_some() && exp_hyp.is_none() {
            println!("decode: the expected file has no hypotheses");
            ok = false;
        }
        let (sign, gap) = decision_margin(&layout, hyp);
        println!(
            "decision margins in our rows: smallest |logit| of a switch or a route {sign:.3e}, smallest gap between a categorical's top two {gap:.3e} (a difference of this size between the two runs could flip a decision)"
        );
        ok &= decode_report(&layout, hyp, exp_hyp, &contexts, a.decode_tolerance);
        let exp_conf = expected
            .as_ref()
            .and_then(|e| e.iter().find(|t| t.name == "confidence"));
        if let (Some(ec), Some(oc)) = (exp_conf, out.iter().find(|t| t.name == "confidence")) {
            let k = oc.dims[oc.dims.len() - 1] as usize;
            let n = oc.values.len() / k;
            let (mut same, mut top1) = (0usize, 0usize);
            let mut tightest = f32::INFINITY;
            for i in 0..n {
                let (x, y) = (
                    &oc.values[i * k..(i + 1) * k],
                    &ec.values[i * k..(i + 1) * k],
                );
                let (ox, oy) = (order(x), order(y));
                same += usize::from(ox == oy);
                top1 += usize::from(ox[0] == oy[0]);
                for w in ox.windows(2) {
                    tightest = tightest.min(x[w[0]] - x[w[1]]);
                }
            }
            println!(
                "confidence order: {same} of {n} samples rank all {k} hypotheses identically, {top1} the same first (the smallest gap between neighbours in our ranking is {tightest:.3e})"
            );
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// A conv stack of about `gflop`: a 3 × 3 stem to `c` channels on `mel`, then 3 × 3 convolutions `c` → `c` at
/// stride 1 with `Relu`s, global pooling and the K = 8 heads. Returns the model's bytes.
fn conv_stack(gflop: f64, c: u32) -> Vec<u8> {
    let mut b = ModelBuilder::new("bench-conv-stack", 8);
    let mut seed = 1u64;
    let mut values = |n: usize, scale: f32| {
        let mut s = seed;
        seed += 1;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((s >> 40) as i32 - (1 << 23)) as f32 / (1 << 23) as f32 * scale
            })
            .collect::<Vec<f32>>()
    };
    let pad = [1, 1, 1, 1, 1, 1, 1, 1, 1];
    let w = b.tensor_f16("stem", &[c, 3, 3, 3], &values((c * 27) as usize, 0.25));
    let mut x = b.node(OpCode::Conv2d, &[Val::MEL, w], &pad);
    let per_layer = 2.0 * f64::from(c * c) * 128.0 * 64.0 * 9.0;
    let stem = 2.0 * f64::from(c) * 128.0 * 64.0 * 27.0;
    let layers = ((gflop * 1e9 - stem) / per_layer).round().max(0.0) as usize;
    for i in 0..layers {
        let w = b.tensor_f16(
            &format!("conv{i}"),
            &[c, c, 3, 3],
            &values((c * c * 9) as usize, 0.0625),
        );
        let r = b.node(OpCode::Relu, &[x], &[]);
        x = b.node(OpCode::Conv2d, &[r, w], &pad);
    }
    let g = b.node(OpCode::GlobalAvgPool, &[x], &[]);
    let f = b.node(OpCode::Reshape, &[g], &[c as i32]);
    let feat = b.node(OpCode::Concat, &[f, Val::CONTEXT], &[0]);
    let w = b.tensor_f16(
        "hyp",
        &[8 * 102, c + 5],
        &values((816 * (c + 5)) as usize, 0.25),
    );
    let h = b.node(OpCode::Linear, &[feat, w], &[]);
    let r = b.node(OpCode::Reshape, &[h], &[8, 102]);
    let w = b.tensor("conf", &[8, c + 5], &values((8 * (c + 5)) as usize, 0.25));
    let conf = b.node(OpCode::Linear, &[feat, w], &[]);
    b.output("hypotheses", r, &[8, 102])
        .output("confidence", conf, &[8]);
    b.build()
}

fn bench(gflops: &[f64], runs: usize) -> ExitCode {
    for g in gflops {
        let bytes = conv_stack(*g, 32);
        let pack = match ModelPack::parse(&bytes) {
            Ok(p) => p,
            Err(e) => return refused(&e),
        };
        let net = match Net::build(&pack) {
            Ok(n) => n,
            Err(e) => return refused(&e),
        };
        let input = synthetic_input(0);
        let mut times = Vec::with_capacity(runs);
        let mut hash = String::new();
        for _ in 0..runs.max(1) {
            let started = Instant::now();
            let out = net.run(&input).expect("a synthetic input fits");
            times.push(started.elapsed().as_secs_f64());
            hash = hex(&out.sha256());
        }
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        let flops = net.flops() as f64;
        println!(
            "conv stack, 32 channels on [3, 128, 64]: {:.3} GFLOP ({} parameters): median {:.0} ms of {} runs ({:.2} GFLOP/s), outputs sha256 {}",
            flops / 1e9,
            pack.header().parameters,
            median * 1e3,
            times.len(),
            flops / median / 1e9,
            &hash[..16]
        );
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        return usage();
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "info" if rest.len() == 1 => info(&PathBuf::from(&rest[0])),
        "run" if !rest.is_empty() => {
            let mut a = RunArgs {
                model: PathBuf::from(&rest[0]),
                seeds: None,
                inputs: None,
                expected: None,
                tolerance: 1e-5,
                write: None,
                decode: false,
                decode_tolerance: 1e-4,
            };
            let mut it = rest[1..].iter();
            while let Some(flag) = it.next() {
                if flag == "--decode" {
                    a.decode = true;
                    continue;
                }
                let Some(value) = it.next() else {
                    return usage();
                };
                match flag.as_str() {
                    "--seed" => {
                        let seeds: Result<Vec<u64>, _> =
                            value.split(',').map(|s| s.trim().parse()).collect();
                        match seeds {
                            Ok(s) if !s.is_empty() => a.seeds = Some(s),
                            _ => return usage(),
                        }
                    }
                    "--inputs" => a.inputs = Some(PathBuf::from(value)),
                    "--expected" => a.expected = Some(PathBuf::from(value)),
                    "--write" => a.write = Some(PathBuf::from(value)),
                    "--tolerance" => match value.parse() {
                        Ok(t) => a.tolerance = t,
                        Err(_) => return usage(),
                    },
                    "--decode-tolerance" => match value.parse() {
                        Ok(t) => a.decode_tolerance = t,
                        Err(_) => return usage(),
                    },
                    _ => return usage(),
                }
            }
            run(&a)
        }
        "bench" => {
            let mut gflops = Vec::new();
            let mut runs = 5;
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let Some(value) = it.next() else {
                    return usage();
                };
                match (flag.as_str(), value.parse::<f64>()) {
                    ("--gflop", Ok(g)) if g > 0.0 => gflops.push(g),
                    ("--runs", Ok(n)) if n >= 1.0 => runs = n as usize,
                    _ => return usage(),
                }
            }
            if gflops.is_empty() {
                gflops = vec![0.5, 2.0];
            }
            bench(&gflops, runs)
        }
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lane EXP's `PyTorch` fixture, the one `enth-solver`'s `propose_parity` test checks in CI.
    const MODEL: &[u8] =
        include_bytes!("../../../crates/enth-solver/tests/fixtures/propose/exp_tiny.bin");
    const EXPECTED: &[u8] =
        include_bytes!("../../../crates/enth-solver/tests/fixtures/propose/exp_tiny.expected");

    /// The fixture's outputs on its own seeds, batched as `run` combines them.
    fn fixture_run() -> (ModelPack, Vec<ModelInput>, Vec<Tensor>) {
        let pack = ModelPack::parse(MODEL).unwrap();
        let net = Net::build(&pack).unwrap();
        let inputs: Vec<ModelInput> = pack
            .info_value("parity.seeds")
            .unwrap()
            .split(',')
            .map(|s| synthetic_input(s.parse().unwrap()))
            .collect();
        let runs: Vec<Outputs> = inputs.iter().map(|i| net.run(i).unwrap()).collect();
        let out = combine(&runs, true);
        (pack, inputs, out)
    }

    #[test]
    fn order_ranks_by_descending_confidence_and_ties_go_to_the_lower_index() {
        assert_eq!(order(&[0.5, 2.0, 0.5, -1.0]), [1, 0, 2, 3]);
        assert_eq!(order(&[1.0]), [0]);
    }

    #[test]
    fn tensor_files_round_trip() {
        let t = vec![
            Tensor {
                name: "a".to_owned(),
                dims: vec![2, 3],
                values: vec![0.0, 1.5, -2.0, 3.25, 4.0, 5.0],
            },
            Tensor {
                name: "hypotheses".to_owned(),
                dims: vec![1],
                values: vec![-0.125],
            },
        ];
        let back = read_tensors(&write_tensors(&t)).unwrap();
        assert_eq!(back.len(), 2);
        for (x, y) in t.iter().zip(&back) {
            assert_eq!((&x.name, &x.dims, &x.values), (&y.name, &y.dims, &y.values));
        }
        assert!(read_tensors(b"NOTATENS\0\0\0\0").is_err());
    }

    #[test]
    fn batched_outputs_gain_a_leading_dimension() {
        let (_, inputs, out) = fixture_run();
        assert_eq!(out[0].name, "hypotheses");
        assert_eq!(out[0].dims, [inputs.len() as u32, 8, 102]);
        assert_eq!(out[1].dims, [inputs.len() as u32, 8]);
        assert_eq!(out[0].values.len(), inputs.len() * 8 * 102);
    }

    #[test]
    fn the_pytorch_fixture_agrees_and_decodes_the_same() {
        let (pack, inputs, out) = fixture_run();
        let expected = read_tensors(EXPECTED).unwrap();
        for t in &expected {
            let ours = out.iter().find(|o| o.name == t.name).unwrap();
            assert_eq!(ours.dims, t.dims);
            let worst = ours
                .values
                .iter()
                .zip(&t.values)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "{}: {worst:.3e}", t.name);
        }
        let h = pack.header();
        let layout = OutputLayout::build(h.layout, h.generator, h.template).unwrap();
        let contexts: Vec<[f32; 5]> = inputs.iter().map(|i| i.context).collect();
        let exp_hyp = expected.iter().find(|t| t.name == "hypotheses");
        assert!(decode_report(&layout, &out[0], exp_hyp, &contexts, 1e-4));
    }

    #[test]
    fn a_flipped_switch_is_a_decode_failure() {
        let (pack, inputs, out) = fixture_run();
        let h = pack.header();
        let layout = OutputLayout::build(h.layout, h.generator, h.template).unwrap();
        let contexts: Vec<[f32; 5]> = inputs.iter().map(|i| i.context).collect();
        let mut bad = out[0].clone();
        let at = layout.enables()[0].offset;
        for row in bad.values.chunks_exact_mut(layout.width()) {
            row[at] = if row[at] > 0.0 { -50.0 } else { 50.0 };
        }
        assert!(!decode_report(
            &layout,
            &out[0],
            Some(&bad),
            &contexts,
            1e-4
        ));
        let (sign, gap) = decision_margin(&layout, &bad);
        assert!(sign >= 0.0 && gap >= 0.0);
    }

    #[test]
    fn a_job_hears_the_contexts_key_pitch_and_hold() {
        let unpitched = job_hearing(&[-2.0, 0.3, 0.0, 99.0, 0.0], 0).unwrap();
        assert_eq!(unpitched.told_key(), 58);
        assert_eq!(unpitched.pitch_offset(), None);
        let pitched = job_hearing(&[0.0, 0.25, 1.0, 0.01, 0.0], 3).unwrap();
        assert_eq!(pitched.told_key(), 60);
        assert_eq!(pitched.shift(), 3);
        let offset = pitched.pitch_offset().unwrap();
        assert!((offset - 0.25).abs() < 1e-9, "{offset}");
    }
}
