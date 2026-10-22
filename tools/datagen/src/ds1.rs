//! **Owner: lane DG** (the dataset generator, R3b).
//!
//! `ds1` (docs/r3.md, "`ds1`: training data from our engine"): generator v1 is unfit as training data (level 1 has about
//! 12 sounds, every phrase starts on C3, `amounts` below 100 pull values towards the start), so a new versioned sampler
//! reuses v1's keyed draws, priors and `observable()` mask from `enth-challenge` and adds a mixture (60 % v1 priors at
//! `amounts` 100 and level 5, 25 % perturbed preset candidates **from the anchor half only**, 15 % uniform over the
//! safe ranges), a phrase sampler, labels in the `ParamSpec` normalised domain with a loss mask, filters (renders below
//! -60 LUFS rejected; clipped renders kept; **never a filter by duration**) and splits by stream key (`r3/train`,
//! `r3/dev`, `r3/test`), never by index ranges. The files are in `ds1/`:
//!
//! | File | Does |
//! | --- | --- |
//! | `sampler.rs` | a sample as a pure function of `(ds1 version, stream, run seed, index)` |
//! | `anchors.rs` | the 11 anchor presets (frozen byte copies) and their projection onto layout 1 |
//! | `jnd.rs` | the loss scale: one just-noticeable step per entry, in the normalised domain |
//! | `pipeline.rs` | render, intake, features, label, and the exclusion tests, per sample |
//! | `clips.rs` | the bridge: clean clips out for the degradation chain, degraded clips back in as shards |
//!
//! ```text
//! enth-datagen ds1 gen     --count N --seed S --out DIR [--stream K] [--workers W] [--shard-size M] [--gr3 DIR] [--force]
//! enth-datagen ds1 bench   --count N [--seed S] [--workers W] [--stream K]         # throughput, stage times, skips
//! enth-datagen ds1 show    --index I [--seed S] [--stream K]                        # one sample, drawn and labelled
//! enth-datagen ds1 anchors                                                           # how far projection moves the presets
//! enth-datagen ds1 audit   --count N [--seed S] [--workers W]                       # structure labels on inaudible modules
//! enth-datagen ds1 render  --count N --seed S --out DIR [--start I] [--workers W]   # clean clips for DEG's chain
//! enth-datagen shards verify DIR | info DIR | features --samples F --clips DIR --out DIR
//! enth-datagen exclude lists | check DIR [--against DIR]...
//! ```
//!
//! A run is the candidates `0..count` of a stream, cut into shards of `--shard-size` consecutive indices; a shard is a
//! pure function of its window, so any number of workers writes the same bytes, a rerun skips the shards whose file
//! still has the recorded hash (`--force` redoes them), and `kept = count - skips` (the manifest counts the skips by
//! code). The module docs of `shards.rs` describe the file; `docs/r3.md` has the contract.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a command-line tool: reports go to stdout and stderr; clocks and files are the tool's"
)]

pub(crate) mod anchors;
pub(crate) mod clips;
pub(crate) mod jnd;
pub(crate) mod pipeline;
pub(crate) mod sampler;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use enth_params::Kind;
use enth_patch::Op;
use enth_solver::propose::decode::OutputLayout;
use enth_solver::propose::features;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::jnd::JndScale;
use self::pipeline::{Clip, NEAR_RADIUS, Pipeline, QUIET_LUFS, References, Timing, label_clip};
use self::sampler::{
    DS1_VERSION, MIXTURE, MUSICAL, RATES, STREAM_DEV, STREAM_TEST, STREAM_TRAIN, Sampler,
};
use crate::exclude::{Exclusions, NearIndex, default_gr3, label_hash};
use crate::shards::{self, MANIFEST_SCHEMA, Manifest, Meta, Rows, ShardStats};

const USAGE: &str = "usage: enth-datagen ds1 <command>
  gen --count N --seed S --out DIR [--stream K] [--workers W] [--shard-size M] [--gr3 DIR] [--force]
  bench --count N [--seed S] [--workers W] [--stream K] [--gr3 DIR]
  show --index I [--seed S] [--stream K] [--gr3 DIR]
  anchors
  audit --count N [--seed S] [--workers W] [--stream K] [--gr3 DIR]
  render --count N --seed S --out DIR [--start I] [--stream K] [--workers W] [--gr3 DIR]";

/// Parsed `--name value` flags (and bare `--switch`es): unknown flags and stray words are errors.
pub(crate) struct Flags {
    values: BTreeMap<String, String>,
    switches: BTreeSet<String>,
}

impl Flags {
    pub(crate) fn parse(
        args: &[String],
        valued: &[&str],
        switches: &[&str],
    ) -> Result<Flags, String> {
        let mut f = Flags {
            values: BTreeMap::new(),
            switches: BTreeSet::new(),
        };
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            if switches.contains(&a) {
                f.switches.insert(a.to_owned());
            } else if valued.contains(&a) {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{a} needs a value"))?;
                f.values.insert(a.to_owned(), v.clone());
                i += 1;
            } else {
                return Err(format!("unknown argument '{a}'"));
            }
            i += 1;
        }
        Ok(f)
    }

    pub(crate) fn str(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub(crate) fn num<T: FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.str(name)
            .map(|v| {
                v.parse::<T>()
                    .map_err(|_| format!("{name}: '{v}' is not a number"))
            })
            .transpose()
    }

    pub(crate) fn need<T: FromStr>(&self, name: &str) -> Result<T, String> {
        self.num(name)?.ok_or_else(|| format!("{name} is required"))
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    pub(crate) fn gr3(&self) -> PathBuf {
        self.str("--gr3").map_or_else(default_gr3, PathBuf::from)
    }

    pub(crate) fn stream(&self) -> String {
        self.str("--stream").unwrap_or(STREAM_TRAIN).to_owned()
    }
}

/// `ds1`: see the module docs.
pub fn run(args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("gen") => generate(rest),
        Some("bench") => bench(rest),
        Some("show") => show(rest),
        Some("anchors") => anchors_cmd(rest),
        Some("audit") => audit(rest),
        Some("render") => clips::render_cmd(rest),
        _ => Err(USAGE.to_owned()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------------------------------------------

/// `r3/train` and `r3/dev` are training streams; `r3/test` is the development-curve stream (listed so that nothing
/// trains on it, and drawn here so that it can be made); any other key is a training stream.
pub(crate) fn role_of(stream: &str) -> &'static str {
    match stream {
        STREAM_TEST => "test",
        STREAM_DEV => "dev",
        _ => "train",
    }
}

/// A listed stream may be drawn only as the test stream itself.
pub(crate) fn check_stream(ex: &Exclusions, stream: &str) -> Result<(), String> {
    match ex.stream(stream) {
        Some(hit) if stream != STREAM_TEST => Err(format!("refusing to draw: {hit}")),
        _ => Ok(()),
    }
}

pub(crate) fn pool(workers: usize) -> Result<rayon::ThreadPool, String> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers.max(1))
        .thread_name(|i| format!("enth-ds1-{i}"))
        .build()
        .map_err(|e| format!("cannot start {workers} threads: {e}"))
}

/// How much of GR3's references a command needs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    /// Labels and generator targets (a run, a bench).
    Labels,
    /// The generator targets only (a render).
    Generators,
    /// None (show, audit).
    Nothing,
}

pub(crate) fn pipeline_for(
    pool: &rayon::ThreadPool,
    stream: &str,
    seed: u64,
    gr3: &Path,
    need: Need,
) -> Result<Pipeline, String> {
    let ex = Exclusions::load(gr3)?;
    check_stream(&ex, stream)?;
    let sampler = Sampler::new(Some(&ex))?;
    let refs = match need {
        Need::Labels => pool.install(|| References::build(gr3, &ex, sampler.layout()))?,
        Need::Generators => References::generators(&ex, sampler.layout())?,
        Need::Nothing => References::none(sampler.layout()),
    };
    Ok(Pipeline::new(sampler, ex, refs, stream, seed))
}

/// The names of the 102 entries of layout 1, in row order.
pub(crate) fn entry_names(layout: &OutputLayout) -> Vec<String> {
    let mut names = vec![String::new(); layout.width()];
    for e in layout.continuous() {
        names[e.offset] = e.addr.to_string();
    }
    for e in layout.categorical() {
        let items: Vec<&str> = match e.spec.kind {
            Kind::Enum(items) => items.iter().map(|i| i.key).collect(),
            _ => vec!["off", "on"],
        };
        for (i, item) in items.iter().enumerate() {
            names[e.offset + i] = format!("{}={item}", e.addr);
        }
    }
    for e in layout.enables() {
        names[e.offset] = format!("{}.enabled", e.id);
    }
    for c in layout.routes() {
        names[c.presence] = format!("route {}->{} present", c.src, c.dst);
        names[c.depth] = format!("route {}->{} depth", c.src, c.dst);
    }
    names
}

/// The blocks of a manifest that describe the run's contract, shared by `gen` and the bridge.
pub(crate) struct Description {
    pub feature_set: Value,
    pub layout: Value,
    pub jnd: Value,
    pub arrays: Value,
    pub sampler: Value,
    pub exclusions: Value,
}

pub(crate) fn describe(p: &Pipeline) -> Description {
    let layout = p.sampler.layout();
    let scale = JndScale::build(layout);
    let spec = &features::SPEC_V1;
    let inputs: Vec<Value> = spec
        .inputs
        .iter()
        .map(|i| json!({"name": i.name, "dims": i.dims}))
        .collect();
    let anchors: Vec<Value> = p
        .sampler
        .anchors()
        .iter()
        .map(|a| json!({"stem": a.stem, "name": a.name, "sha256": a.sha256}))
        .collect();
    let s = &p.refs.summary;
    let mut exclusions = p.ex.summary();
    if let Some(o) = exclusions.as_object_mut() {
        o.insert(
            "references".to_owned(),
            json!({
                "generator_targets": s.generator_targets,
                "preset_targets": s.preset_targets,
                "labelled": s.labelled,
                "unlabelled": s.unlabelled,
                "missing_files": s.missing_files,
                "near_radius_steps": NEAR_RADIUS,
            }),
        );
    }
    Description {
        feature_set: json!({
            "id": spec.id,
            "name": spec.name,
            "inputs": inputs,
            "window_s": features::WINDOW_S,
            "target_lufs": features::TARGET_LUFS,
            "context": [
                "key - 60 (the key the intake reads)",
                "pitch offset (semitones), 0 when unpitched",
                "pitched (0 or 1)",
                "hold (s), capped at 4",
                "band limit (log2 of the detected low-pass over 20 kHz)",
            ],
        }),
        layout: json!({
            "id": layout.id(),
            "name": enth_solver::propose::decode::LAYOUT_V1_NAME,
            "generator": layout.generator(),
            "template": layout.template_id(),
            "width": layout.width(),
            "sha256": anchors::hex(&layout.sha256()),
            "entries": entry_names(layout),
        }),
        jnd: json!({
            "scale": scale.scale,
            "table": scale.table.iter().map(|r| r.to_vec()).collect::<Vec<_>>(),
            "table_points": jnd::POINTS,
            "note": "scale[i] is one just-noticeable step of entry i in the normalised domain (a categorical, structure or presence entry: 1); table[j][q] is the step's width for continuous entry j at normalised value q/16; see docs/r3.md",
        }),
        arrays: json!(
            shards::ARRAYS
                .iter()
                .map(|(n, d, per)| json!({"name": n, "dtype": d, "per_sample_shape": per}))
                .collect::<Vec<_>>()
        ),
        sampler: json!({
            "version": DS1_VERSION,
            "mixture_percent": MIXTURE.iter().map(|(s, w)| (s.name(), w)).collect::<BTreeMap<_, _>>(),
            "musical_percent": MUSICAL.iter().map(|(m, w)| (m.to_string(), w)).collect::<BTreeMap<_, _>>(),
            "rates_sixteenths": RATES.iter().map(|(r, w)| (r.to_string(), w)).collect::<BTreeMap<_, _>>(),
            "quiet_lufs": QUIET_LUFS,
            "anchors": anchors,
        }),
        exclusions,
    }
}

/// What a manifest records of the run besides the pipeline.
pub(crate) struct RunInfo<'a> {
    /// `clean` or `clips`.
    pub kind: &'a str,
    /// Candidates (clean) or lines (clips).
    pub count: u64,
    /// Candidates or lines per shard.
    pub shard_size: u32,
    /// Notes that are not errors.
    pub warnings: Vec<String>,
    /// The bridge's block, else null.
    pub bridge: Value,
}

fn manifest_of(p: &Pipeline, info: RunInfo<'_>, shards: Vec<ShardStats>) -> Manifest {
    let d = describe(p);
    let mut skips: BTreeMap<String, u64> = BTreeMap::new();
    let mut sources: BTreeMap<String, u64> = BTreeMap::new();
    let mut kept = 0u64;
    for s in &shards {
        kept += u64::from(s.kept);
        for (k, v) in &s.skips {
            *skips.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &s.sources {
            *sources.entry(k.clone()).or_default() += u64::from(*v);
        }
    }
    Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        kind: info.kind.to_owned(),
        ds1: DS1_VERSION,
        stream: p.stream.clone(),
        role: role_of(&p.stream).to_owned(),
        run_seed: p.run_seed,
        count: info.count,
        shard_size: info.shard_size,
        kept,
        skips,
        sources,
        engine: enth_patch::file::ENGINE_VERSION.to_owned(),
        feature_set: d.feature_set,
        layout: d.layout,
        jnd: d.jnd,
        arrays: d.arrays,
        sampler: d.sampler,
        exclusions: d.exclusions,
        bridge: info.bridge,
        warnings: info.warnings,
        shards,
    }
}

/// Write `manifest.json` (pretty, with sorted keys: the same bytes for the same run) and return its SHA-256.
pub(crate) fn write_manifest(
    p: &Pipeline,
    out: &Path,
    info: RunInfo<'_>,
    shards: Vec<ShardStats>,
) -> Result<(Manifest, String), String> {
    let m = manifest_of(p, info, shards);
    let mut bytes = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    shards::write_atomic(&out.join("manifest.json"), &bytes)?;
    Ok((m, shards::sha256_hex(&bytes)))
}

// ---------------------------------------------------------------------------------------------------------------
// Shards
// ---------------------------------------------------------------------------------------------------------------

/// What identifies a run for resuming: anything that moves a shard's bytes.
fn run_key(p: &Pipeline, shard_size: u32) -> String {
    let d = describe(p);
    let text = format!(
        "ds1/{}|{}|{}|{}|{}|{}|{}|{}|{}",
        DS1_VERSION,
        p.stream,
        p.run_seed,
        shard_size,
        d.exclusions["sha256"],
        d.layout["sha256"],
        d.feature_set["id"],
        enth_patch::file::ENGINE_VERSION,
        enth_analysis::intake::INTAKE_VERSION,
    );
    shards::sha256_hex(text.as_bytes())
}

#[derive(Serialize, Deserialize)]
struct Sidecar {
    run: String,
    stats: ShardStats,
}

/// Accumulates one shard's samples.
#[derive(Default)]
pub(crate) struct ShardBuilder {
    pub rows: Rows,
    pub skips: BTreeMap<String, u64>,
    pub sources: BTreeMap<String, u32>,
    seen: HashSet<[u8; 32]>,
}

impl ShardBuilder {
    pub(crate) fn skip(&mut self, code: &str) {
        *self.skips.entry(code.to_owned()).or_default() += 1;
    }

    /// Add a labelled sample, unless a sample with the same masked label is already here.
    pub(crate) fn push(&mut self, s: &pipeline::Sample) {
        let l = &s.labelled;
        if !self.seen.insert(label_hash(&l.label, &l.mask)) {
            self.skip("duplicate_label");
            return;
        }
        let r = &mut self.rows;
        r.mel.extend_from_slice(&l.mel);
        r.context.extend_from_slice(&l.context);
        r.label.extend_from_slice(&l.label);
        r.mask.extend_from_slice(&l.mask);
        r.index.push(s.index);
        r.seed.push(s.seed);
        r.gen_seed.push(s.gen_seed);
        r.rate.push(s.rate);
        r.source.push(s.source as u8);
        *self.sources.entry(s.source.name().to_owned()).or_default() += 1;
    }

    /// Encode, hash and write the shard (`shard-NNNNNN.bin`, then its `.json` sidecar).
    pub(crate) fn finish(self, out: &Path, run: &str, meta: &Meta) -> Result<ShardStats, String> {
        let bytes = shards::encode(meta, &self.rows);
        let file = shards::shard_name(meta.shard);
        let stats = ShardStats {
            file: file.clone(),
            shard: meta.shard,
            first_index: meta.first_index,
            candidates: meta.candidates,
            kept: self.rows.len() as u32,
            bytes: bytes.len() as u64,
            sha256: shards::sha256_hex(&bytes),
            skips: self.skips,
            sources: self.sources,
        };
        shards::write_atomic(&out.join(&file), &bytes)?;
        let side = Sidecar {
            run: run.to_owned(),
            stats: stats.clone(),
        };
        let json = serde_json::to_vec(&side).map_err(|e| e.to_string())?;
        shards::write_atomic(&out.join(file.replace(".bin", ".json")), &json)?;
        Ok(stats)
    }
}

/// The shard recorded in `out` for window `meta`, when its file still has the recorded bytes.
fn resumed(out: &Path, run: &str, meta: &Meta) -> Option<ShardStats> {
    let file = shards::shard_name(meta.shard);
    let side: Sidecar =
        serde_json::from_slice(&fs::read(out.join(file.replace(".bin", ".json"))).ok()?).ok()?;
    let s = &side.stats;
    let same = side.run == run
        && s.file == file
        && s.shard == meta.shard
        && s.first_index == meta.first_index
        && s.candidates == meta.candidates;
    let bytes = fs::read(out.join(&file)).ok()?;
    (same && bytes.len() as u64 == s.bytes && shards::sha256_hex(&bytes) == s.sha256)
        .then_some(side.stats)
}

/// Shard `k` of a run: candidates `k * size .. min((k + 1) * size, count)`.
fn compute_shard(
    p: &Pipeline,
    out: &Path,
    run: &str,
    k: u32,
    size: u32,
    count: u64,
) -> Result<(ShardStats, Timing, bool), String> {
    let first = u64::from(k) * u64::from(size);
    let end = (first + u64::from(size)).min(count);
    let meta = Meta {
        ds1: DS1_VERSION,
        stream: p.stream.clone(),
        run_seed: p.run_seed,
        shard: k,
        first_index: first,
        candidates: (end - first) as u32,
    };
    if let Some(stats) = resumed(out, run, &meta) {
        return Ok((stats, Timing::default(), true));
    }
    let mut b = ShardBuilder::default();
    let mut timing = Timing::default();
    for index in first..end {
        let (result, t) = p.sample(index);
        timing.add(&t);
        match result {
            Ok(s) => b.push(&s),
            Err(skip) => b.skip(skip.code),
        }
    }
    Ok((b.finish(out, run, &meta)?, timing, false))
}

/// `load average` from the OS, for the throughput notes.
pub(crate) fn load_average() -> String {
    fs::read_to_string("/proc/loadavg").ok().map_or_else(
        || "unknown".to_owned(),
        |t| t.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

fn generate(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(
        args,
        &[
            "--count",
            "--seed",
            "--out",
            "--stream",
            "--workers",
            "--shard-size",
            "--gr3",
        ],
        &["--force"],
    )?;
    let count: u64 = f.need("--count")?;
    let seed: u64 = f.need("--seed")?;
    let out = PathBuf::from(f.str("--out").ok_or("--out is required")?);
    let workers: usize = f.num("--workers")?.unwrap_or(1);
    let size: u32 = f.num("--shard-size")?.unwrap_or(64);
    if count == 0 || size == 0 {
        return Err("--count and --shard-size must be at least 1".to_owned());
    }
    let stream = f.stream();
    let started = Instant::now();
    let pool = pool(workers)?;
    let p = pipeline_for(&pool, &stream, seed, &f.gr3(), Need::Labels)?;
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    if f.has("--force") {
        for k in 0..count.div_ceil(u64::from(size)) {
            let _ =
                fs::remove_file(out.join(shards::shard_name(k as u32).replace(".bin", ".json")));
        }
    }
    let setup_s = started.elapsed().as_secs_f64();
    let run = run_key(&p, size);
    let n = count.div_ceil(u64::from(size)) as u32;
    eprintln!(
        "ds1 v{DS1_VERSION}: {count} candidates of {stream} (run seed {seed}) in {n} shards of {size}, {workers} workers; references ready in {setup_s:.1} s ({} labelled)",
        p.refs.near.len()
    );
    let t0 = Instant::now();
    let done: Vec<Result<(ShardStats, Timing, bool), String>> = pool.install(|| {
        (0..n)
            .into_par_iter()
            .map(|k| {
                let r = compute_shard(&p, &out, &run, k, size, count);
                if let Ok((s, _, resumed)) = &r {
                    eprintln!(
                        "shard {}/{n}: kept {} of {}{}",
                        k + 1,
                        s.kept,
                        s.candidates,
                        if *resumed { " (resumed)" } else { "" }
                    );
                }
                r
            })
            .collect()
    });
    let wall = t0.elapsed().as_secs_f64();
    let mut stats = Vec::new();
    let mut timing = Timing::default();
    let mut computed = 0u64;
    for r in done {
        let (s, t, resumed) = r?;
        if !resumed {
            computed += u64::from(s.candidates);
        }
        timing.add(&t);
        stats.push(s);
    }
    let (m, sha) = write_manifest(
        &p,
        &out,
        RunInfo {
            kind: "clean",
            count,
            shard_size: size,
            warnings: Vec::new(),
            bridge: Value::Null,
        },
        stats,
    )?;
    println!(
        "wrote {} shards to {}: kept {} of {} candidates",
        m.shards.len(),
        out.display(),
        m.kept,
        m.count
    );
    println!(
        "skips by code: {}",
        serde_json::to_string(&m.skips).map_err(|e| e.to_string())?
    );
    println!(
        "kept by source: {}",
        serde_json::to_string(&m.sources).map_err(|e| e.to_string())?
    );
    println!("manifest sha256: {sha}");
    if computed > 0 {
        println!(
            "computed {computed} candidates in {wall:.1} s on {workers} workers: {:.1} candidates/s, {:.1} samples/s kept (load average {})",
            computed as f64 / wall,
            m.kept as f64 / wall * (computed as f64 / m.count as f64),
            load_average()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// bench
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Agg {
    kept: u64,
    skips: BTreeMap<String, u64>,
    sources: BTreeMap<String, u64>,
    rates: BTreeMap<u32, u64>,
    timing: Timing,
}

impl Agg {
    fn merge(mut self, other: Agg) -> Agg {
        self.kept += other.kept;
        for (k, v) in other.skips {
            *self.skips.entry(k).or_default() += v;
        }
        for (k, v) in other.sources {
            *self.sources.entry(k).or_default() += v;
        }
        for (k, v) in other.rates {
            *self.rates.entry(k).or_default() += v;
        }
        self.timing.add(&other.timing);
        self
    }
}

fn bench(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(
        args,
        &[
            "--count",
            "--seed",
            "--stream",
            "--workers",
            "--gr3",
            "--start",
        ],
        &[],
    )?;
    let count: u64 = f.need("--count")?;
    let seed: u64 = f.num("--seed")?.unwrap_or(1);
    let start: u64 = f.num("--start")?.unwrap_or(0);
    let workers: usize = f.num("--workers")?.unwrap_or(1);
    let pool = pool(workers)?;
    let t = Instant::now();
    let p = pipeline_for(&pool, &f.stream(), seed, &f.gr3(), Need::Labels)?;
    let refs_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let agg = pool.install(|| {
        (start..start + count)
            .into_par_iter()
            .map(|i| {
                let (r, t) = p.sample(i);
                let mut a = Agg {
                    timing: t,
                    ..Agg::default()
                };
                match r {
                    Ok(s) => {
                        a.kept = 1;
                        *a.sources.entry(s.source.name().to_owned()).or_default() += 1;
                        *a.rates.entry(s.rate).or_default() += 1;
                    }
                    Err(skip) => *a.skips.entry(skip.code.to_owned()).or_default() += 1,
                }
                a
            })
            .reduce(Agg::default, Agg::merge)
    });
    let wall = t.elapsed().as_secs_f64();
    let n = count as f64;
    println!(
        "references: {refs_s:.1} s ({} generator targets, {} presets, {} labelled)",
        p.refs.summary.generator_targets, p.refs.summary.preset_targets, p.refs.summary.labelled
    );
    println!(
        "{count} candidates in {wall:.2} s on {workers} workers (load average {}): {:.1} candidates/s, {:.1} samples/s kept ({} kept)",
        load_average(),
        n / wall,
        agg.kept as f64 / wall,
        agg.kept
    );
    println!(
        "cpu seconds per candidate by stage (ms): draw {:.1}, render {:.1}, intake {:.1}, features {:.1}, label {:.1}; total {:.1}",
        1e3 * agg.timing.draw / n,
        1e3 * agg.timing.render / n,
        1e3 * agg.timing.intake / n,
        1e3 * agg.timing.features / n,
        1e3 * agg.timing.label / n,
        1e3 * agg.timing.total() / n
    );
    println!("kept by source: {:?}", agg.sources);
    println!("kept by rate: {:?}", agg.rates);
    println!("skips by code: {:?}", agg.skips);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------------------------------------------

fn show(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(args, &["--index", "--seed", "--stream", "--gr3"], &[])?;
    let index: u64 = f.need("--index")?;
    let seed: u64 = f.num("--seed")?.unwrap_or(1);
    let stream = f.stream();
    let ex = Exclusions::load(&f.gr3())?;
    check_stream(&ex, &stream)?;
    let sampler = Sampler::new(Some(&ex))?;
    let d = sampler
        .draw(&stream, seed, index)
        .map_err(|e| format!("{}: {e}", e.code()))?;
    let layout = sampler.layout();
    let changed: Vec<String> = {
        let mut out = Vec::new();
        for e in layout.continuous().iter().chain(layout.categorical()) {
            let v = d.patch.value(&e.addr).unwrap_or(e.template);
            if v != e.template {
                out.push(format!("{}: {} -> {v}", e.addr, e.template));
            }
        }
        out
    };
    let on: Vec<String> = layout
        .enables()
        .iter()
        .filter(|e| d.patch.find(&e.id).is_some_and(|(_, m)| m.enabled))
        .map(|e| e.id.to_string())
        .collect();
    let routes: Vec<String> = d
        .patch
        .routes
        .iter()
        .map(|r| format!("{}->{} {}", r.src, r.dst, r.depth))
        .collect();
    println!("{}", serde_json::to_string_pretty(&json!({
        "index": d.index,
        "seed": d.seed,
        "source": d.source.name(),
        "anchor": d.anchor.map(|i| sampler.anchors()[i].stem),
        "generator": d.generator.as_ref().map(|g| json!({"seed": g.spec.seed, "code": g.code, "fingerprint": g.fingerprint})),
        "rate": d.rate,
        "length_ms": d.length_ms,
        "notes": d.notes.iter().map(|n| json!({"key": n.key, "velocity": n.velocity, "start_ms": n.start_ms, "hold_ms": n.hold_ms})).collect::<Vec<_>>(),
        "modules_on": on,
        "changed_from_template": changed,
        "routes": routes,
    })).map_err(|e| e.to_string())?);
    let clip = Clip {
        patch: &d.patch,
        notes: &d.notes,
        length_ms: d.length_ms,
        rate: d.rate,
        render_seed: d.render_seed,
    };
    match label_clip(layout, &clip) {
        Err(skip) => println!("skipped: {skip}"),
        Ok(l) => {
            println!(
                "heard: {:.1} LUFS, peak {:.3}; the model is told key {} (context {:?}), cue {:?}",
                l.lufs, l.peak, l.cue.key, l.context, l.cue
            );
            let names = entry_names(layout);
            let carried = l.mask.iter().filter(|m| **m > 0.0).count();
            println!("label: {carried} of {} entries carry a loss", names.len());
            for (i, name) in names.iter().enumerate() {
                if l.mask[i] > 0.0 && l.label[i] != 0.0 {
                    println!("  {name} = {}", l.label[i]);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// anchors
// ---------------------------------------------------------------------------------------------------------------

fn anchors_cmd(args: &[String]) -> Result<(), String> {
    Flags::parse(args, &[], &[])?;
    let layout = OutputLayout::v1();
    let anchors = anchors::load(&layout)?;
    println!(
        "| anchor | outside-layout params reset | of those audible (> 1 JND) | grid-less routes dropped | layout values clamped | pooled log-mel distance (nepers) |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    let moved: Vec<anchors::Moved> = anchors
        .par_iter()
        .map(|a| anchors::measure(&layout, a))
        .collect();
    for m in &moved {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            m.stem,
            m.reset.len(),
            m.reset_audible,
            m.dropped_routes.len(),
            m.clamped,
            m.nepers.map_or("silent".to_owned(), |n| format!("{n:.3}"))
        );
    }
    let nepers: Vec<f64> = moved.iter().filter_map(|m| m.nepers).collect();
    if !nepers.is_empty() {
        let mut sorted = nepers.clone();
        sorted.sort_by(f64::total_cmp);
        println!(
            "\nmedian {:.3} nepers, max {:.3}, {} of {} within one near-render step (0.125)",
            sorted[sorted.len() / 2],
            sorted[sorted.len() - 1],
            nepers.iter().filter(|n| **n <= 0.125).count(),
            nepers.len()
        );
    }
    for m in &moved {
        println!("\n{}:", m.stem);
        for r in &m.reset {
            println!("  reset {r}");
        }
        for r in &m.dropped_routes {
            println!("  dropped route {r}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// audit
// ---------------------------------------------------------------------------------------------------------------

/// A source and a module: the audit's table key.
type Key = (&'static str, String);

/// Samples whose structure switch is on, per module: how many, and the distances (nepers) between the render and the
/// render with that module switched off.
#[derive(Default)]
struct ModuleAudit {
    on: u64,
    off: u64,
    distances: Vec<f64>,
    silent: u64,
}

fn audit(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(
        args,
        &["--count", "--seed", "--stream", "--workers", "--gr3"],
        &[],
    )?;
    let count: u64 = f.need("--count")?;
    let seed: u64 = f.num("--seed")?.unwrap_or(1);
    let workers: usize = f.num("--workers")?.unwrap_or(1);
    let pool = pool(workers)?;
    let p = pipeline_for(&pool, &f.stream(), seed, &f.gr3(), Need::Nothing)?;
    let layout = p.sampler.layout();
    let sources = [
        sampler::Source::V1,
        sampler::Source::Preset,
        sampler::Source::Uniform,
    ];
    let rows: Vec<Vec<(Key, Option<f64>, bool)>> = pool.install(|| {
        (0..count)
            .into_par_iter()
            .map(|i| {
                let mut out = Vec::new();
                let Ok((d, r)) = p.render_candidate(i) else {
                    return out;
                };
                let perf = d.phrase().performance();
                let Some(full) = anchors::embedding(&r.patch, &perf, d.rate) else {
                    return out;
                };
                for e in layout.enables() {
                    let key = (d.source.name(), e.id.to_string());
                    let is_on = r.patch.find(&e.id).is_some_and(|(_, m)| m.enabled);
                    if !is_on {
                        out.push((key, None, false));
                        continue;
                    }
                    let mut off = r.patch.clone();
                    let applied = enth_patch::apply(
                        &mut off,
                        Op::SetEnabled {
                            id: e.id.clone(),
                            enabled: false,
                        },
                    );
                    let dist = applied
                        .ok()
                        .and_then(|_| anchors::embedding(&off, &perf, d.rate))
                        .and_then(|b| full.distance(&b));
                    out.push((key, dist, true));
                }
                out
            })
            .collect()
    });
    let mut table: BTreeMap<Key, ModuleAudit> = BTreeMap::new();
    for (key, dist, on) in rows.into_iter().flatten() {
        let a = table.entry(key).or_default();
        if on {
            a.on += 1;
            match dist {
                Some(d) => a.distances.push(d),
                None => a.silent += 1,
            }
        } else {
            a.off += 1;
        }
    }
    println!(
        "A structure label says 'on' when the module is enabled; it is inaudible when switching it off moves the pooled log-mel by under 0.125 nepers (one just-noticeable step; the bank's near-render step).\n"
    );
    println!(
        "| source | module | samples | on | on and inaudible | share of the on labels | median distance when on (nepers) |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let mut total_on = 0u64;
    let mut total_quiet = 0u64;
    for src in sources {
        let (mut son, mut squiet) = (0u64, 0u64);
        for ((s, module), a) in table.iter().filter(|((s, _), _)| *s == src.name()) {
            let quiet = a.distances.iter().filter(|d| **d < 0.125).count() as u64 + a.silent;
            let mut d = a.distances.clone();
            d.sort_by(f64::total_cmp);
            let median = d
                .get(d.len() / 2)
                .map_or("-".to_owned(), |m| format!("{m:.3}"));
            println!(
                "| {s} | {module} | {} | {} | {} | {} | {} |",
                a.on + a.off,
                a.on,
                quiet,
                share(quiet, a.on),
                median
            );
            son += a.on;
            squiet += quiet;
        }
        println!(
            "| {} | all | | {son} | {squiet} | {} | |",
            src.name(),
            share(squiet, son)
        );
        total_on += son;
        total_quiet += squiet;
    }
    println!(
        "\nall sources: {total_quiet} of {total_on} 'on' labels are inaudible ({})",
        share(total_quiet, total_on)
    );
    Ok(())
}

fn share(n: u64, of: u64) -> String {
    if of == 0 {
        "-".to_owned()
    } else {
        format!("{:.1} %", 100.0 * n as f64 / of as f64)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// exclude check
// ---------------------------------------------------------------------------------------------------------------

/// `exclude check RUN_DIR`: re-read a finished run and assert the exclusions again, independently of the pass that
/// made it (see `exclude.rs`). Exit 0 means clean.
pub(crate) fn check_run(dir: &Path, gr3: &Path, against: &[PathBuf]) -> Result<(), String> {
    let m = shards::verify(dir)?;
    let ex = Exclusions::load(gr3)?;
    let sha = anchors::hex(&ex.sha256);
    if m.exclusions["sha256"] != json!(sha) {
        return Err(format!(
            "{} was made against other exclusion lists ({}; these are {sha}): rerun it",
            dir.display(),
            m.exclusions["sha256"]
        ));
    }
    if m.ds1 != DS1_VERSION {
        return Err(format!(
            "{} is ds1 v{}; this build checks v{DS1_VERSION}",
            dir.display(),
            m.ds1
        ));
    }
    check_stream(&ex, &m.stream)?;
    if m.role != role_of(&m.stream) {
        return Err(format!(
            "the manifest's role '{}' is not the stream's '{}'",
            m.role,
            role_of(&m.stream)
        ));
    }
    let pool = pool(rayon::current_num_threads())?;
    let p = pool.install(|| pipeline_for(&pool, &m.stream, m.run_seed, gr3, Need::Labels))?;
    let (checked, duplicates, run_rows) = check_rows(&p, &m, dir, !against.is_empty())?;
    for other in against {
        let o = shards::verify(other)?;
        if o.stream == m.stream && o.run_seed == m.run_seed {
            return Err(format!(
                "{} is the same stream and seed as {}",
                other.display(),
                dir.display()
            ));
        }
        let mut near = NearIndex::new(p.sampler.layout(), NEAR_RADIUS);
        let mut n = 0usize;
        for s in &o.shards {
            let bytes = fs::read(other.join(&s.file)).map_err(|e| format!("{}: {e}", s.file))?;
            let (_, rows) = shards::decode(&bytes).map_err(|e| format!("{}: {e}", s.file))?;
            for i in 0..rows.len() {
                let (l, mk) = rows.row(i);
                near.add(format!("{}:{}", o.stream, rows.index[i]), l, mk);
                n += 1;
            }
        }
        let hits: Vec<String> = run_rows
            .par_iter()
            .filter_map(|(l, mk)| {
                near.near(l, mk)
                    .map(|(id, d)| format!("{id} ({d:.2} steps)"))
            })
            .take_any(5)
            .collect();
        if !hits.is_empty() {
            return Err(format!(
                "{} has labels within {NEAR_RADIUS} step of {} ({} samples): {}",
                dir.display(),
                other.display(),
                n,
                hits.join(", ")
            ));
        }
    }
    println!(
        "clean: {checked} samples of {} ({} stream {}, run seed {}): no listed seed, code or fingerprint, none within {NEAR_RADIUS} step of GR3's {} labelled targets ({} generator targets by rule 3){}; {duplicates} exact label duplicates across shards (dedup is within a shard)",
        dir.display(),
        m.role,
        m.stream,
        m.run_seed,
        p.refs.summary.labelled,
        p.refs.summary.generator_targets,
        if against.is_empty() {
            String::new()
        } else {
            format!(
                ", none within {NEAR_RADIUS} step of {} other run(s)",
                against.len()
            )
        }
    );
    Ok(())
}

/// The per-sample assertions of `exclude check` on a verified run: stored seeds, a re-draw from the key, the lists and
/// rule 3 again, and the label-space test against GR3's references. Returns the samples checked, the exact label
/// duplicates across shards and (when asked) the labels, for the `--against` scan.
type Labels = Vec<(Vec<f32>, Vec<f32>)>;

fn check_rows(
    p: &Pipeline,
    m: &Manifest,
    dir: &Path,
    keep_rows: bool,
) -> Result<(u64, u64, Labels), String> {
    let clean = m.kind == "clean";
    let mut seen_labels: HashSet<[u8; 32]> = HashSet::new();
    let mut duplicates = 0u64;
    let mut checked = 0u64;
    let mut run_rows: Labels = Vec::new();
    for s in &m.shards {
        let bytes = fs::read(dir.join(&s.file)).map_err(|e| format!("{}: {e}", s.file))?;
        let (_, rows) = shards::decode(&bytes).map_err(|e| format!("{}: {e}", s.file))?;
        for i in 0..rows.len() {
            let (label, mask) = rows.row(i);
            let at = format!("{}: sample {}", s.file, rows.index[i]);
            if let Some(hit) = p.ex.seed(rows.seed[i]) {
                return Err(format!("{at}: {hit}"));
            }
            if rows.gen_seed[i] != 0
                && let Some(hit) = p.ex.seed(rows.gen_seed[i])
            {
                return Err(format!("{at}: {hit}"));
            }
            // The sample is re-drawn from its key: the stored seeds, rate and source must be what the key gives,
            // and the draw must pass the lists again (codes, fingerprints and rule 3).
            let d = p
                .sampler
                .draw(&m.stream, m.run_seed, rows.index[i])
                .map_err(|e| format!("{at} no longer draws: {e}"))?;
            let gen_seed = d.generator.as_ref().map_or(0, |g| g.spec.seed);
            if d.seed != rows.seed[i]
                || gen_seed != rows.gen_seed[i]
                || d.source as u8 != rows.source[i]
                || (clean && d.rate != rows.rate[i])
            {
                return Err(format!(
                    "{at} is not what its key draws (the sampler moved?)"
                ));
            }
            p.screen(&d).map_err(|e| format!("{at}: {e}"))?;
            if let Some((id, dist)) = p.refs.near.near(label, mask) {
                return Err(format!("{at} is within {dist:.2} steps of GR3 target {id}"));
            }
            if !seen_labels.insert(label_hash(label, mask)) {
                duplicates += 1;
            }
            if keep_rows {
                run_rows.push((label.to_vec(), mask.to_vec()));
            }
            checked += 1;
        }
    }
    Ok((checked, duplicates, run_rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_strict() {
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let f = Flags::parse(&v(&["--count", "5", "--force"]), &["--count"], &["--force"]).unwrap();
        assert_eq!(f.need::<u64>("--count").unwrap(), 5);
        assert!(f.has("--force"));
        assert!(Flags::parse(&v(&["--nope"]), &["--count"], &[]).is_err());
        assert!(Flags::parse(&v(&["--count"]), &["--count"], &[]).is_err());
        assert!(Flags::parse(&v(&["stray"]), &["--count"], &[]).is_err());
        let f = Flags::parse(&v(&["--count", "x"]), &["--count"], &[]).unwrap();
        assert!(f.need::<u64>("--count").is_err());
        assert!(f.need::<u64>("--seed").is_err());
    }

    #[test]
    fn the_entry_names_cover_the_row_once() {
        let layout = OutputLayout::v1();
        let names = entry_names(&layout);
        assert_eq!(names.len(), 102);
        assert!(names.iter().all(|n| !n.is_empty()), "{names:?}");
        let unique: BTreeSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), 102);
        assert_eq!(names[0], layout.continuous()[0].addr.to_string());
        assert_eq!(features::CONTEXT, 5);
        assert_eq!(
            (features::RESOLUTIONS, features::FRAMES, features::MEL_BANDS),
            (3, 128, 64)
        );
    }

    fn lists(synth: &str) -> Exclusions {
        Exclusions::parse(&[
            ("exclude-synth.txt", synth.as_bytes().to_vec()),
            ("exclude-real.txt", Vec::new()),
            ("exclude-degraded.txt", Vec::new()),
        ])
        .unwrap()
    }

    fn bare_pipeline(ex: Exclusions, seed: u64) -> Pipeline {
        let sampler = Sampler::new(Some(&ex)).unwrap();
        let refs = References::generators(&ex, sampler.layout()).unwrap();
        Pipeline::new(sampler, ex, refs, STREAM_TRAIN, seed)
    }

    #[test]
    fn the_check_finds_an_excluded_sample_in_a_finished_shard() {
        let dir = std::env::temp_dir().join(format!("enth-ds1-check-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = bare_pipeline(lists(""), 31);
        let run = run_key(&p, 12);
        let (stats, _, _) = compute_shard(&p, &dir, &run, 0, 12, 12).unwrap();
        let info = RunInfo {
            kind: "clean",
            count: 12,
            shard_size: 12,
            warnings: Vec::new(),
            bridge: Value::Null,
        };
        let (m, _) = write_manifest(&p, &dir, info, vec![stats]).unwrap();
        let (checked, _, _) = check_rows(&p, &m, &dir, false).unwrap();
        assert_eq!(checked, m.kept);
        assert!(checked >= 8);
        shards::verify(&dir).unwrap();
        // The same shard, checked against lists that name one of its generator samples (by seed, code, fingerprint).
        let (_, rows) =
            shards::decode(&fs::read(dir.join(shards::shard_name(0))).unwrap()).unwrap();
        let g = rows
            .index
            .iter()
            .find_map(|i| p.sampler.draw(STREAM_TRAIN, 31, *i).ok()?.generator)
            .expect("a generator sample among 12");
        for entry in [
            format!("seed-range {0}..{0}", g.spec.seed),
            format!("code {}", g.code.clone().unwrap()),
            format!("fingerprint {}", g.fingerprint),
        ] {
            let q = bare_pipeline(lists(&entry), 31);
            let err = check_rows(&q, &m, &dir, false).unwrap_err();
            assert!(
                err.contains("sample") && err.contains("exclu"),
                "{entry}: {err}"
            );
        }
        // A sample that is not what its key draws is caught: the same shard under another run seed.
        let mut moved = m.clone();
        moved.run_seed = 32;
        assert!(
            check_rows(&p, &moved, &dir, false)
                .unwrap_err()
                .contains("not what its key draws")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listed_stream_is_refused_except_the_test_stream() {
        let ex = Exclusions::load(&default_gr3()).unwrap();
        assert!(check_stream(&ex, STREAM_TRAIN).is_ok());
        assert!(check_stream(&ex, STREAM_DEV).is_ok());
        assert!(check_stream(&ex, STREAM_TEST).is_ok());
        assert!(check_stream(&ex, "gr3/dev").is_err());
        assert_eq!(role_of(STREAM_TEST), "test");
        assert_eq!(role_of("r3/anything"), "train");
    }
}
