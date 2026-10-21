//! **Owner: lane TP** (the k-NN bank builder, R3a). Pre-declared by lane S0s (R3); driven by `cargo xtask bank`.
//!
//! `bank`: builds the k-NN retrieval bank that `enth_solver::nearest` reads (docs/r3.md, "The start portfolio and k-NN
//! retrieval"; the format and the embedding live in `nearest`, so this package only builds and checks the file).
//!
//! ```text
//! cargo xtask bank [--entries 20000] [--out PATH] [--threads 3] [--gr3 content/gr3]
//! cargo xtask bank check PATH          # re-reads a bank, regenerates every entry, re-asserts the exclusions
//! cargo xtask bank cost [--evals 24]   # the per-evaluation cost table (render + features, 1 worker and 8)
//! cargo xtask bank probe PATH          # does retrieval help? start distances of k-NN against templates and chance
//! ```
//!
//! # What it does
//!
//! 1. **Reads `content/gr3/exclude-{synth,real,degraded}.txt`** (lane DS) and hashes them; an unknown entry type is an
//!    error, so a newer list is never half-read (the README's rule 1). Their SHA-256 goes into the bank's header.
//! 2. **Draws candidate specs from its own stream**, `r3/bank/v1`: candidate `i` gets the seed
//!    `derive(fnv1a64("r3/bank/v1"), i)` cut to 40 bits (so every spec has a short code), a level preset 2–5 (weights
//!    1 : 2 : 3 : 4, level 5 most: it is the most varied) and `musical` 100 (¾) or 50 (¼), all by keyed draws.
//!    Generator v1, template 0 (the starter superset). Level 1 is left out: it has only 14 distinct targets and all
//!    are GR3 targets.
//! 3. **Filters each candidate, before rendering**, against the lists: a seed inside a `seed-range`, a short code on
//!    a `code` line, a target `fingerprint`, and a **near duplicate of a GR3 target** (DS's rule 3, against every
//!    generator target on the list, test and dev, at any level: the same phrase, the same enabled modules, the same
//!    routes with depths under 5 points apart, and every randomized value within its just-noticeable step). Then
//!    duplicates of an earlier bank entry (same fingerprint) are dropped. Each skip is counted in the manifest.
//! 4. **Renders** each survivor at the bank's reference performance (C3, velocity 0.8, held 1 s, 2 s at 48 kHz), drops
//!    the silent ones (below −70 LUFS: nothing to match; never a filter by duration), measures the pitch and embeds
//!    it (`nearest::embed`), on a rayon pool of `--threads` workers. Results are taken in candidate order, so the
//!    bank is the same bytes for any thread count.
//! 5. **Asserts** the finished bank once more, independently: every entry regenerates to its fingerprint prefix, no
//!    seed is in a listed range, no code or fingerprint is listed, no two entries share a fingerprint, the stream key
//!    matches no listed stream. The bank holds generator specs only, so the preset, path, audio, Freesound and
//!    degradation entries cannot match; the manifest says so.
//! 6. **Writes** the bank (`nearest::Bank::to_bytes`) and a manifest (`enth.bank-manifest/1`, JSON) beside it.
//!
//! Like `enth-lesson-tool` this is tooling, exempt from the layer rules: it reads files, reads the clock (for the
//! manifest's build time and the cost table, information only) and runs threads.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tooling: reads and writes files, times itself, runs a rayon pool"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use enth_analysis::Describer;
use enth_analysis::descriptors::Want;
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_challenge::code::ChallengeCode;
use enth_challenge::priors::{ROUTE_JND, jnd};
use enth_challenge::random::{self, Generated};
use enth_challenge::spec::{PRESETS, RandomSpec};
use enth_dsp::util::rng::{derive, fnv1a64};
use enth_patch::{Op, Patch, apply, file};
use enth_render::{Performance, RenderSpec, render};
use enth_solver::nearest::{
    BANK_FORMAT, Bank, BankEntry, BankHeader, EMBEDDING_ID, EMBEDDING_VERSION, ENTRY_LEN, REF_NOTE,
    embed, pitch_cents, reference_hz,
};
use enth_solver::search::{MetricTarget, Objective};
use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The bank's own stream key (not `r3/test`, `r3/dev` or any `gr3/*` stream: the builder asserts it).
const STREAM: &str = "r3/bank/v1";
/// The default size of the shipped bank.
const DEFAULT_ENTRIES: usize = 20_000;
/// The reference performance: C3, velocity 0.8, held 1 s, in a 2 s render.
const VELOCITY: f32 = 0.8;
const HOLD_S: f32 = 1.0;
const DURATION_S: f32 = 2.0;
/// The render rate and seed.
const SAMPLE_RATE: u32 = 48_000;
const RENDER_SEED: u64 = enth_render::DEFAULT_SEED;
/// The most candidates per batch.
const CHUNK: usize = 512;
/// Seeds are cut to this many bits, so every spec has a short code.
const SEED_BITS: u32 = 40;
/// The exclusion lists, in the order their bytes are hashed.
const LISTS: [&str; 3] = [
    "exclude-synth.txt",
    "exclude-real.txt",
    "exclude-degraded.txt",
];
/// Every entry type the lists may hold (content/gr3/README.md); any other is an error.
const KNOWN_TYPES: [&str; 17] = [
    "seed-range",
    "stream",
    "code",
    "fingerprint",
    "preset-stem",
    "preset-name",
    "file-sha256",
    "path",
    "freesound-id",
    "freesound-pack",
    "freesound-uploader",
    "audio-sha256",
    "degrade-pool",
    "degrade-variant",
    "degrade-profile",
    "degrade-seed-range",
    "pools-sha256",
];

/// `bank`: build, check or time.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check") => check(
            args.get(1).ok_or("usage: bank check <bank file>")?,
            &args[2..],
        ),
        Some("cost") => cost(&args[1..]),
        Some("probe") => probe(
            args.get(1).ok_or("usage: bank probe <bank file>")?,
            &args[2..],
        ),
        _ => build(args),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------------------------------

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn num<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> Result<T, String> {
    match opt(args, name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{name}: not a number: {v}")),
    }
}

fn pool(threads: usize) -> Result<rayon::ThreadPool, String> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .thread_name(|i| format!("enth-bank-{i}"))
        .build()
        .map_err(|e| format!("cannot start {threads} threads: {e}"))
}

// ---------------------------------------------------------------------------------------------------------------
// The exclusion lists
// ---------------------------------------------------------------------------------------------------------------

/// The entries of `content/gr3/exclude-*.txt` the bank can be checked against, and the lists' hashes.
#[derive(Debug, Default)]
struct Exclusions {
    seed_ranges: Vec<(u64, u64)>,
    streams: Vec<String>,
    codes: BTreeSet<String>,
    fingerprints: BTreeSet<String>,
    /// Entries per type, for the manifest.
    counts: BTreeMap<String, usize>,
    /// SHA-256 of each list's bytes, and of the three in order.
    file_sha256: Vec<(String, String)>,
    sha256: [u8; 32],
}

fn hex(bytes: &[u8]) -> String {
    enth_solver::create::record::hex(bytes)
}

fn read_exclusions(dir: &Path) -> Result<Exclusions, String> {
    let mut ex = Exclusions::default();
    let mut all = Sha256::new();
    for name in LISTS {
        let path = dir.join(name);
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        all.update(&bytes);
        ex.file_sha256
            .push((name.to_owned(), hex(&Sha256::digest(&bytes))));
        let text = String::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8"))?;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (kind, value) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let value = value.trim();
            if !KNOWN_TYPES.contains(&kind) {
                return Err(format!(
                    "{name}:{}: unknown entry type '{kind}': this builder is older than the list",
                    n + 1
                ));
            }
            *ex.counts.entry(kind.to_owned()).or_default() += 1;
            match kind {
                "seed-range" => {
                    let (lo, hi) = value
                        .split_once("..")
                        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                        .ok_or_else(|| format!("{name}:{}: bad seed-range '{value}'", n + 1))?;
                    ex.seed_ranges.push((lo, hi));
                }
                "stream" => ex.streams.push(value.to_owned()),
                "code" => {
                    ex.codes.insert(value.to_owned());
                }
                "fingerprint" => {
                    ex.fingerprints.insert(value.to_ascii_lowercase());
                }
                _ => {}
            }
        }
    }
    ex.sha256 = all.finalize().into();
    Ok(ex)
}

impl Exclusions {
    fn seed_listed(&self, seed: u64) -> bool {
        self.seed_ranges
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&seed))
    }

    fn stream_listed(&self, key: &str) -> bool {
        self.streams.iter().any(|s| match s.strip_suffix('*') {
            Some(prefix) => key.starts_with(prefix),
            None => key == s,
        })
    }

    /// The GR3 generator targets on the list (test and dev), regenerated from their codes.
    fn targets(&self) -> Result<Vec<Generated>, String> {
        self.codes
            .iter()
            .map(|c| {
                let code = ChallengeCode::decode(c).map_err(|e| format!("listed code {c}: {e}"))?;
                random::target(code.spec()).map_err(|e| format!("listed code {c}: {e}"))
            })
            .collect()
    }
}

/// DS's near-duplicate rule (content/gr3/synth.toml, selection rule 3), applied across levels: the same phrase, the
/// same enabled modules, the same routes with depths under one route step apart, and every value either target
/// randomized within its just-noticeable step.
fn near_duplicate(a: &Generated, b: &Generated) -> bool {
    if a.phrase != b.phrase {
        return false;
    }
    let enabled = |p: &Patch| -> Vec<(String, bool)> {
        p.modules()
            .map(|(_, m)| (m.id.as_str().to_owned(), m.enabled))
            .collect()
    };
    if enabled(&a.patch) != enabled(&b.patch) {
        return false;
    }
    if a.patch.routes.len() != b.patch.routes.len() {
        return false;
    }
    for ra in &a.patch.routes {
        let Some(rb) = b
            .patch
            .routes
            .iter()
            .find(|r| r.src == ra.src && r.dst == ra.dst)
        else {
            return false;
        };
        if ROUTE_JND.exceeded(ra.depth, rb.depth) {
            return false;
        }
    }
    a.params.iter().chain(&b.params).all(|addr| {
        match (a.patch.spec(addr), a.patch.value(addr), b.patch.value(addr)) {
            (Some(spec), Some(va), Some(vb)) => !jnd(spec).exceeded(va, vb),
            _ => false,
        }
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------------------------------------------

/// Candidate `i`'s spec: its seed, level preset and `musical`, all keyed draws from the bank's stream.
fn candidate_spec(i: u64) -> RandomSpec {
    let base = derive(fnv1a64(STREAM.as_bytes()), i);
    let seed = base & ((1u64 << SEED_BITS) - 1);
    let level_draw = derive(base, 1) % 10;
    let preset = match level_draw {
        0 => 1,     // level 2: 1 in 10
        1..=2 => 2, // level 3: 2 in 10
        3..=5 => 3, // level 4: 3 in 10
        _ => 4,     // level 5: 4 in 10
    };
    let mut spec = PRESETS[preset].spec(1, seed);
    if derive(base, 2).is_multiple_of(4) {
        spec.musical = 50;
    }
    spec
}

/// Why a candidate was skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Skip {
    SeedRange,
    Code,
    Fingerprint,
    NearDuplicate,
    Duplicate,
    Silent,
    Generate,
}

impl Skip {
    fn name(self) -> &'static str {
        match self {
            Skip::SeedRange => "seed_range",
            Skip::Code => "excluded_code",
            Skip::Fingerprint => "excluded_fingerprint",
            Skip::NearDuplicate => "near_duplicate_of_gr3_target",
            Skip::Duplicate => "duplicate_of_earlier_entry",
            Skip::Silent => "silent",
            Skip::Generate => "does_not_generate",
        }
    }
}

fn screen(i: u64, ex: &Exclusions, gr3: &[Generated]) -> Result<(RandomSpec, Generated), Skip> {
    let spec = candidate_spec(i);
    if ex.seed_listed(spec.seed) {
        return Err(Skip::SeedRange);
    }
    if let Some(code) = ChallengeCode::short(spec)
        && ex.codes.contains(&code.encode())
    {
        return Err(Skip::Code);
    }
    let g = random::target(&spec).map_err(|_| Skip::Generate)?;
    if ex.fingerprints.contains(&g.fingerprint.to_hex()) {
        return Err(Skip::Fingerprint);
    }
    if gr3.iter().any(|t| near_duplicate(&g, t)) {
        return Err(Skip::NearDuplicate);
    }
    Ok((spec, g))
}

fn reference_perf() -> Performance {
    Performance::note(REF_NOTE, VELOCITY, f64::from(HOLD_S), f64::from(DURATION_S))
}

fn feature_cfg() -> FeatureCfg {
    FeatureCfg {
        sample_rate: f64::from(SAMPLE_RATE),
        ..FeatureCfg::default()
    }
}

/// Render and embed one bank patch; `None` when it is silent.
fn measure(spec: &RandomSpec, g: &Generated) -> Option<BankEntry> {
    let perf = reference_perf();
    let out = render(&g.patch, &perf, &RenderSpec::new(SAMPLE_RATE, RENDER_SEED)).ok()?;
    let mid = out.mid();
    let cfg = feature_cfg();
    let features = FeatureExtractor::new(cfg).extract(&mid);
    if features.silent {
        return None;
    }
    let desc = Describer::new(f64::from(SAMPLE_RATE))
        .describe_some(&mid, enth_solver::note_timing(&perf), Want::CORE)
        .ok();
    Some(BankEntry {
        seed: spec.seed,
        difficulty: spec.difficulty,
        musical: spec.musical,
        pitch_cents: pitch_cents(desc.as_ref(), REF_NOTE),
        fingerprint_prefix: g.fingerprint.prefix().0,
        embedding: embed(
            &features,
            &cfg,
            0.0,
            reference_hz(desc.as_ref()),
            f64::from(SAMPLE_RATE) / 2.0,
        ),
    })
}

fn header(ex: &Exclusions) -> BankHeader {
    BankHeader {
        format: BANK_FORMAT,
        generator: 1,
        template: 0,
        embedding: EMBEDDING_VERSION,
        note: REF_NOTE,
        velocity: VELOCITY,
        hold_s: HOLD_S,
        duration_s: DURATION_S,
        sample_rate: SAMPLE_RATE,
        render_seed: RENDER_SEED,
        exclusions_sha256: ex.sha256,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct Manifest {
    schema: &'static str,
    file: String,
    sha256: String,
    bytes: usize,
    entries: usize,
    entry_bytes: usize,
    embedding: &'static str,
    stream: &'static str,
    seed_rule: String,
    spec_mix: &'static str,
    generator: u16,
    template: u8,
    reference: ReferenceOut,
    candidates_tried: u64,
    skipped: BTreeMap<&'static str, usize>,
    gr3_targets_checked: usize,
    exclusions: ExclusionsOut,
    asserted: Vec<&'static str>,
    threads: usize,
    build_seconds: f64,
}

#[derive(Serialize)]
struct ReferenceOut {
    note: u8,
    velocity: f32,
    hold_s: f32,
    duration_s: f32,
    sample_rate: u32,
    render_seed: u64,
}

#[derive(Serialize)]
struct ExclusionsOut {
    sha256: String,
    files: BTreeMap<String, String>,
    entries: BTreeMap<String, usize>,
}

fn build(args: &[String]) -> Result<(), String> {
    let entries: usize = num(args, "--entries", DEFAULT_ENTRIES)?;
    let threads: usize = num(args, "--threads", 3)?;
    let gr3 = opt(args, "--gr3").map_or_else(|| workspace().join("content/gr3"), PathBuf::from);
    let out = opt(args, "--out").map_or_else(
        || {
            std::env::var_os("CARGO_TARGET_DIR")
                .map_or_else(|| workspace().join("target"), PathBuf::from)
                .join("bank/enth-bank-v1.bin")
        },
        PathBuf::from,
    );
    if entries == 0 || entries > u32::MAX as usize {
        return Err("--entries must be 1 to 4294967295".to_owned());
    }
    let t0 = Instant::now();
    let ex = read_exclusions(&gr3)?;
    if ex.stream_listed(STREAM) {
        return Err(format!(
            "the bank's stream {STREAM} is on an exclusion list"
        ));
    }
    let targets = ex.targets()?;
    let pool = pool(threads)?;
    eprintln!(
        "bank: {entries} entries, {threads} threads, {} GR3 targets to stay away from",
        targets.len()
    );

    let mut bank: Vec<BankEntry> = Vec::with_capacity(entries);
    let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
    let mut skipped: BTreeMap<Skip, usize> = BTreeMap::new();
    let mut next = 0u64;
    while bank.len() < entries {
        // Candidates are taken in order, so the batch size changes only how many extra renders the last batch makes.
        let need = entries - bank.len();
        let chunk = CHUNK.min(need + need / 8 + 8) as u64;
        let ids: Vec<u64> = (next..next + chunk).collect();
        next += chunk;
        let screened: Vec<Result<(RandomSpec, Generated), Skip>> =
            pool.install(|| ids.par_iter().map(|&i| screen(i, &ex, &targets)).collect());
        let mut fresh: Vec<(RandomSpec, Generated)> = Vec::new();
        for r in screened {
            match r {
                Ok((spec, g)) => {
                    if seen.insert(g.fingerprint.0) {
                        fresh.push((spec, g));
                    } else {
                        *skipped.entry(Skip::Duplicate).or_default() += 1;
                    }
                }
                Err(why) => *skipped.entry(why).or_default() += 1,
            }
        }
        let measured: Vec<Option<BankEntry>> =
            pool.install(|| fresh.par_iter().map(|(s, g)| measure(s, g)).collect());
        for m in measured {
            match m {
                Some(e) if bank.len() < entries => bank.push(e),
                Some(_) => {}
                None => *skipped.entry(Skip::Silent).or_default() += 1,
            }
        }
        eprintln!(
            "bank: {} / {entries} after {next} candidates ({:.0} s)",
            bank.len(),
            t0.elapsed().as_secs_f64()
        );
    }
    let bank = Bank::new(header(&ex), bank);
    assert_bank(&bank, &ex, &pool)?;
    let bytes = bank.to_bytes();
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    fs::write(&out, &bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let manifest = Manifest {
        schema: "enth.bank-manifest/1",
        file: out
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        sha256: hex(&Sha256::digest(&bytes)),
        bytes: bytes.len(),
        entries: bank.len(),
        entry_bytes: ENTRY_LEN,
        embedding: EMBEDDING_ID,
        stream: STREAM,
        seed_rule: format!(
            "candidate i: derive(fnv1a64(\"{STREAM}\"), i) cut to {SEED_BITS} bits; candidates in order, the first {entries} that pass"
        ),
        spec_mix: "generator 1, template 0; level presets 2/3/4/5 at 1:2:3:4; musical 100 (3/4) or 50 (1/4)",
        generator: 1,
        template: 0,
        reference: ReferenceOut {
            note: REF_NOTE,
            velocity: VELOCITY,
            hold_s: HOLD_S,
            duration_s: DURATION_S,
            sample_rate: SAMPLE_RATE,
            render_seed: RENDER_SEED,
        },
        candidates_tried: next,
        skipped: skipped.iter().map(|(k, v)| (k.name(), *v)).collect(),
        gr3_targets_checked: targets.len(),
        exclusions: ExclusionsOut {
            sha256: hex(&ex.sha256),
            files: ex.file_sha256.iter().cloned().collect(),
            entries: ex.counts.clone(),
        },
        asserted: ASSERTED.to_vec(),
        threads,
        build_seconds: t0.elapsed().as_secs_f64(),
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    let mpath = out.with_extension("json");
    fs::write(&mpath, json).map_err(|e| format!("cannot write {}: {e}", mpath.display()))?;
    println!(
        "{} ({} entries, {} bytes, sha256 {}) and {} in {:.1} s",
        out.display(),
        bank.len(),
        bytes.len(),
        manifest.sha256,
        mpath.display(),
        manifest.build_seconds
    );
    Ok(())
}

/// What [`assert_bank`] checks, for the manifest.
const ASSERTED: [&str; 7] = [
    "every entry regenerates to its fingerprint prefix (generator v1, template 0)",
    "no seed inside a listed seed-range",
    "no short code on a code line",
    "no target fingerprint on a fingerprint line",
    "no two entries share a fingerprint",
    "the stream key matches no listed stream",
    "generator specs only: no preset, file, path, audio, Freesound or degradation item can be in the bank",
];

/// The independent pass over a finished bank (see the [module docs](self), step 5). Fails on the first violation,
/// naming the entry.
fn assert_bank(bank: &Bank, ex: &Exclusions, pool: &rayon::ThreadPool) -> Result<(), String> {
    if ex.stream_listed(STREAM) {
        return Err(format!("the bank's stream {STREAM} is listed"));
    }
    let fingerprints: Vec<Result<[u8; 32], String>> = pool.install(|| {
        (0..bank.len())
            .into_par_iter()
            .map(|i| {
                let spec = bank.spec(i);
                let e = &bank.entries()[i];
                if ex.seed_listed(e.seed) {
                    return Err(format!(
                        "entry {i}: seed {} is in a listed seed-range",
                        e.seed
                    ));
                }
                if let Some(code) = ChallengeCode::short(spec)
                    && ex.codes.contains(&code.encode())
                {
                    return Err(format!("entry {i}: code {} is listed", code.encode()));
                }
                let g = random::target(&spec).map_err(|err| format!("entry {i}: {err}"))?;
                if g.fingerprint.prefix().0 != e.fingerprint_prefix {
                    return Err(format!("entry {i}: regenerates another target"));
                }
                if ex.fingerprints.contains(&g.fingerprint.to_hex()) {
                    return Err(format!(
                        "entry {i}: fingerprint {} is listed",
                        g.fingerprint.to_hex()
                    ));
                }
                Ok(g.fingerprint.0)
            })
            .collect()
    });
    let mut seen = BTreeSet::new();
    for (i, f) in fingerprints.into_iter().enumerate() {
        if !seen.insert(f?) {
            return Err(format!("entry {i}: the same target as an earlier entry"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------------------------------------------

fn check(path: &str, args: &[String]) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let bank = Bank::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let gr3 = opt(args, "--gr3").map_or_else(|| workspace().join("content/gr3"), PathBuf::from);
    let threads: usize = num(args, "--threads", 3)?;
    let ex = read_exclusions(&gr3)?;
    if bank.header().exclusions_sha256 != ex.sha256 {
        return Err(format!(
            "{path} was built against other exclusion lists ({}; these are {}): rebuild it",
            hex(&bank.header().exclusions_sha256),
            hex(&ex.sha256)
        ));
    }
    assert_bank(&bank, &ex, &pool(threads)?)?;
    println!(
        "{path}: {} entries, sha256 {}, every assertion holds",
        bank.len(),
        hex(&Sha256::digest(&bytes))
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------------------------------------------

/// A pool objective over metric workers, one per rayon thread, results by index (what the CLI's `enth match` does).
struct PoolObjective<'a> {
    target: &'a MetricTarget,
}

impl Objective for PoolObjective<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        candidates
            .par_iter()
            .map_init(
                || self.target.worker(),
                enth_solver::search::MetricWorker::distance,
            )
            .collect()
    }
}

fn template(text: &str) -> Result<Patch, String> {
    file::load(text)
        .map(|l| l.patch)
        .map_err(|e| format!("a template does not load: {e}"))
}

/// The starter superset with every module on: two oscillators, the noise, the filter, the compressor and the reverb.
fn everything_on() -> Result<Patch, String> {
    let mut p = random::start(0).map_err(|e| e.to_string())?;
    let ids: Vec<_> = p
        .generators
        .iter()
        .chain(&p.voice_fx)
        .chain(&p.fx)
        .filter(|m| !m.enabled)
        .map(|m| m.id.clone())
        .collect();
    for id in ids {
        apply(&mut p, Op::SetEnabled { id, enabled: true }).map_err(|e| e.to_string())?;
    }
    Ok(p)
}

fn cost(args: &[String]) -> Result<(), String> {
    let evals: usize = num(args, "--evals", 24)?;
    let workers: Vec<usize> = opt(args, "--workers")
        .unwrap_or("1,8")
        .split(',')
        .map(|w| {
            w.trim()
                .parse()
                .map_err(|_| format!("--workers: bad number {w}"))
        })
        .collect::<Result<_, _>>()?;
    let mut patches: Vec<(String, Vec<Patch>)> = vec![
        (
            "init (1 osc, dry)".to_owned(),
            vec![template(include_str!(
                "../../../content/templates/init.patch.json"
            ))?],
        ),
        (
            "pad_2osc (2 osc, filter, LFO, reverb)".to_owned(),
            vec![template(include_str!(
                "../../../content/templates/pad_2osc.patch.json"
            ))?],
        ),
        (
            "starter, every module on (2 osc, noise, filter, comp, reverb)".to_owned(),
            vec![everything_on()?],
        ),
    ];
    let level5: Vec<Patch> = (0..16)
        .filter_map(|i| {
            random::target(&candidate_spec_level5(i))
                .ok()
                .map(|g| g.patch)
        })
        .collect();
    patches.push((
        "generator level 5 (16 bank-style patches)".to_owned(),
        level5,
    ));

    let heads: Vec<String> = workers
        .iter()
        .map(|w| format!("{w} worker(s): evals/s"))
        .collect();
    println!(
        "| Patch | Clip | ms per evaluation (1 worker) | {} |",
        heads.join(" | ")
    );
    println!("| --- | --- | --- |{}", " --- |".repeat(workers.len()));
    for seconds in [2.0f64, 5.0] {
        let perf = Performance::note(48, 100.0 / 127.0, seconds * 0.5, seconds);
        let spec = RenderSpec::new(SAMPLE_RATE, RENDER_SEED);
        for (name, set) in &patches {
            // The target: the first patch of the set, so the metric runs its full path.
            let out = render(&set[0], &perf, &spec).map_err(|e| e.to_string())?;
            let target = MetricTarget::from_audio(&out.mid(), &perf, &spec);
            let mut ms = f64::NAN;
            let mut cells = Vec::new();
            for &w in &workers {
                let p = pool(w)?;
                let n = evals * w.max(1);
                let batch: Vec<Patch> = (0..n).map(|i| set[i % set.len()].clone()).collect();
                // One warm-up evaluation per worker, then the timed batch.
                p.install(|| PoolObjective { target: &target }.distances(&batch[..w.min(n)]));
                let t = Instant::now();
                let d = p.install(|| PoolObjective { target: &target }.distances(&batch));
                let s = t.elapsed().as_secs_f64();
                if d.iter().any(|x| !x.is_finite()) {
                    return Err(format!("{name}: a non-finite distance"));
                }
                if w == 1 {
                    ms = 1000.0 * s / n as f64;
                }
                cells.push(format!("{:.0}", n as f64 / s));
            }
            println!(
                "| {name} | {seconds:.0} s | {ms:.1} | {} |",
                cells.join(" | ")
            );
        }
    }
    Ok(())
}

/// The cost table's generator patches: level 5 specs from the bank's stream.
fn candidate_spec_level5(i: u64) -> RandomSpec {
    let mut spec = candidate_spec(i);
    spec.difficulty = PRESETS[4].difficulty;
    spec
}

// ---------------------------------------------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------------------------------------------

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        f64::NAN
    } else {
        v[v.len() / 2]
    }
}

/// Does retrieval help? For held-aside generator targets (the bank's own stream far past any bank index, played at
/// another note than the bank's), the metric v1 distance of each kind of start before any search: the init
/// template, the best template, the nearest neighbour, the best of the 8 nearest, and the best of 8 random entries.
fn probe(path: &str, args: &[String]) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let bank = Bank::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let targets: u64 = num(args, "--targets", 24)?;
    let note: u8 = num(args, "--note", 55)?;
    let threads: usize = num(args, "--threads", 3)?;
    let templates = [
        template(include_str!("../../../content/templates/init.patch.json"))?,
        template(include_str!(
            "../../../content/templates/pluck_1osc.patch.json"
        ))?,
        template(include_str!(
            "../../../content/templates/pad_2osc.patch.json"
        ))?,
        template(include_str!(
            "../../../content/templates/bass_sub.patch.json"
        ))?,
    ];
    let in_bank: BTreeSet<u64> = bank.entries().iter().map(|e| e.seed).collect();
    let rows: Vec<Option<[f64; 5]>> = pool(threads)?.install(|| {
        (0..targets)
            .into_par_iter()
            .map(|i| {
                let spec = candidate_spec(1_000_000_000 + i);
                if in_bank.contains(&spec.seed) {
                    return None;
                }
                let g = random::target(&spec).ok()?;
                let perf = Performance::note(note, 100.0 / 127.0, 0.75, 1.5);
                let rspec = RenderSpec::new(SAMPLE_RATE, RENDER_SEED);
                let mid = render(&g.patch, &perf, &rspec).ok()?.mid();
                let t = enth_solver::create::Target {
                    audio: mid.clone(),
                    sample_rate: SAMPLE_RATE,
                    render_seed: RENDER_SEED,
                    perf: enth_solver::create::TargetPerf {
                        key: note,
                        velocity: 100.0 / 127.0,
                        on_s: 0.0,
                        hold_s: 0.75,
                        duration_s: 1.5,
                        f0_hz: None,
                    },
                    bandwidth_hz: None,
                    noise_floor_dbfs: None,
                };
                let analysis = t.analyse().ok()?;
                let metric = MetricTarget::from_audio(&mid, &perf, &rspec);
                let mut w = metric.worker();
                let tpl: Vec<f64> = templates.iter().map(|p| w.distance(p)).collect();
                let near = bank.nearest(&analysis.embedding, 8, 0.0);
                let knn: Vec<f64> = near
                    .iter()
                    .filter_map(|n| bank.patch(n.index).ok())
                    .map(|p| w.distance(&p))
                    .collect();
                let random: Vec<f64> = (0..8u64)
                    .filter_map(|k| {
                        let idx = (derive(derive(0x5052_4F42, i), k) % bank.len() as u64) as usize;
                        bank.patch(idx).ok()
                    })
                    .map(|p| w.distance(&p))
                    .collect();
                let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
                Some([
                    tpl[0],
                    min(&tpl),
                    knn.first().copied().unwrap_or(f64::INFINITY),
                    min(&knn),
                    min(&random),
                ])
            })
            .collect()
    });
    let rows: Vec<[f64; 5]> = rows.into_iter().flatten().collect();
    let col = |k: usize| median(rows.iter().map(|r| r[k]).collect());
    let wins = rows.iter().filter(|r| r[3] < r[1]).count();
    println!(
        "{} targets (bank stream, levels 2-5, note {note}, 1.5 s), metric v1 before any search:",
        rows.len()
    );
    println!("| Start | Median distance |");
    println!("| --- | --- |");
    println!("| init template | {:.3} |", col(0));
    println!("| best of the 4 templates | {:.3} |", col(1));
    println!("| nearest neighbour | {:.3} |", col(2));
    println!("| best of the 8 nearest | {:.3} |", col(3));
    println!("| best of 8 random bank entries | {:.3} |", col(4));
    println!(
        "best of 8 nearest beats the best template on {wins} of {} targets",
        rows.len()
    );
    Ok(())
}
