//! **Owner: lane ONX** (R3b; docs/r3.md, "Our own scalar kernels (lane ONX)").
//!
//! `enth-model-check`: a dev tool for model files (`enth-model-<id>.bin`, `ENTHMODL` format 1). A tooling package,
//! exempt from the layer rules: it reads files and clocks, and runs `enth-solver`'s pure inference on them.
//!
//! ```text
//! enth-model-check info  <model.bin>
//! enth-model-check run   <model.bin> [--seed N | --inputs <file>] [--expected <file>] [--tolerance 1e-5] [--write <file>]
//! enth-model-check bench [--gflop 0.5]... [--runs 5]
//! ```
//!
//! - `info` prints the header, the tables and what [`Net::build`] says: the FLOP recount, the peak memory, or the
//!   refusal (its code and hint, as create would report it).
//! - `run` runs the model once on a synthetic input (`--seed`, else the model's `parity.seed` info entry, else 0) or on
//!   `--inputs` (`mel` and `context` in a raw tensor file), prints each output and the outputs' SHA-256 (what an
//!   output golden pins), and with `--expected` the largest absolute difference per output against a raw tensor file
//!   (ONNX Runtime's, from `ml/enth_ml/export/parity.py`, or `PyTorch`'s): exit 1 above `--tolerance` (default 1e-5).
//!   `--write` saves our outputs as a raw tensor file, so `ml/` can compare them against its own run.
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

use enth_solver::propose::features::ModelInput;
use enth_solver::propose::net::{Net, synthetic_input};
use enth_solver::propose::pack::{ModelBuilder, ModelPack, OpCode, PackError, Val};

/// One tensor of a raw tensor file.
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
        "usage: enth-model-check info <model.bin>\n       enth-model-check run <model.bin> [--seed N | --inputs FILE] [--expected FILE] [--tolerance 1e-5] [--write FILE]\n       enth-model-check bench [--gflop G]... [--runs N]"
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
    seed: Option<u64>,
    inputs: Option<PathBuf>,
    expected: Option<PathBuf>,
    tolerance: f32,
    write: Option<PathBuf>,
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
    let input = if let Some(path) = &a.inputs {
        let t = match load(path).map(|b| read_tensors(&b)) {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                eprintln!("enth-model-check: {}: {e}", path.display());
                return ExitCode::from(2);
            }
            Err(c) => return c,
        };
        let get = |name: &str| t.iter().find(|x| x.name == name).map(|x| x.values.clone());
        let (Some(mel), Some(context)) = (get("mel"), get("context")) else {
            eprintln!(
                "enth-model-check: {} needs tensors mel and context",
                path.display()
            );
            return ExitCode::from(2);
        };
        let Ok(context) = <[f32; 5]>::try_from(context.as_slice()) else {
            eprintln!(
                "enth-model-check: context has {} values, not 5",
                context.len()
            );
            return ExitCode::from(2);
        };
        println!("input: {}", path.display());
        ModelInput { mel, context }
    } else {
        let seed = a
            .seed
            .or_else(|| pack.info_value("parity.seed").and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        println!("input: synthetic, seed {seed}");
        synthetic_input(seed)
    };
    let started = Instant::now();
    let out = match net.run(&input) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("enth-model-check: {} ({e})", e.code());
            return ExitCode::from(2);
        }
    };
    let ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "ran {} ({:.3} GFLOP) in {ms:.1} ms (wall clock, information only)",
        a.model.display(),
        net.flops() as f64 / 1e9
    );
    for o in out.all() {
        let finite = o.values.iter().all(|v| v.is_finite());
        println!(
            "output {} {:?}{}",
            o.name,
            o.dims,
            if finite { "" } else { " NON-FINITE" }
        );
    }
    println!("outputs sha256 {}", hex(&out.sha256()));
    if let Some(path) = &a.write {
        let tensors: Vec<Tensor> = out
            .all()
            .iter()
            .map(|o| Tensor {
                name: o.name.clone(),
                dims: o.dims.clone(),
                values: o.values.clone(),
            })
            .collect();
        if let Err(e) = std::fs::write(path, write_tensors(&tensors)) {
            eprintln!("enth-model-check: cannot write {}: {e}", path.display());
            return ExitCode::from(2);
        }
        println!("wrote {}", path.display());
    }
    let Some(path) = &a.expected else {
        return ExitCode::SUCCESS;
    };
    let expected = match load(path).map(|b| read_tensors(&b)) {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            eprintln!("enth-model-check: {}: {e}", path.display());
            return ExitCode::from(2);
        }
        Err(c) => return c,
    };
    let mut worst = 0.0f32;
    let mut ok = true;
    for t in &expected {
        let Some(ours) = out.get(&t.name) else {
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
                seed: None,
                inputs: None,
                expected: None,
                tolerance: 1e-5,
                write: None,
            };
            let mut it = rest[1..].iter();
            while let Some(flag) = it.next() {
                let Some(value) = it.next() else {
                    return usage();
                };
                match flag.as_str() {
                    "--seed" => match value.parse() {
                        Ok(s) => a.seed = Some(s),
                        Err(_) => return usage(),
                    },
                    "--inputs" => a.inputs = Some(PathBuf::from(value)),
                    "--expected" => a.expected = Some(PathBuf::from(value)),
                    "--write" => a.write = Some(PathBuf::from(value)),
                    "--tolerance" => match value.parse() {
                        Ok(t) => a.tolerance = t,
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
