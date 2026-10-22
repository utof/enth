//! **Owner: lane DG** (the dataset generator, R3b). `exclude`: held-out hygiene (docs/r3.md, "GR3 and its hygiene";
//! `content/gr3/README.md`, "How a training pipeline asserts the exclusions").
//!
//! Lane DS froze GR3's held-out sets with their exclusion lists (`content/gr3/exclude-{synth,real,degraded}.txt`).
//! This module reads the three lists, **fails on an entry type it does not know** (so a newer list is never
//! half-read), records their SHA-256 in every manifest, and answers "is this item on a list?" for everything `ds1`
//! touches. `ds1` asserts, for every sample it keeps:
//!
//! | List entry | What is checked | Where |
//! | --- | --- | --- |
//! | `seed-range` | the sample's seed and its generator seed lie outside the closed range (the GR3 block `770000000000..770999999999`) | each sample (a hit is a skip, `excluded_seed`) |
//! | `stream` | a training stream is none of the listed keys (`r3/test`, `gr3/*`; `*` is a prefix) | the run (a hard error) |
//! | `code`, `fingerprint` | a generator-v1 sample is no listed target: its code (long form), its spec against every listed code's, and its fingerprint | each sample (`excluded_code`, `excluded_fingerprint`) |
//! | `preset-stem`, `preset-name`, `file-sha256` | no anchor is a test preset or a byte copy of one | the sampler's construction (a hard error) |
//! | `path` | nothing is read from under a listed path (clips of the bridge, `shards features`) | each clip |
//! | `degrade-pool`, `degrade-variant`, `degrade-profile`, `degrade-seed-range` | every degradation record of a clip says `pool = train` and shows no held-out variant, profile or seed | each clip of the bridge |
//! | `audio-sha256`, `freesound-*` | no clip has these bytes or comes from these sounds | each clip of the bridge (`ds1` renders no recording, so a render never matches) |
//!
//! Beyond the lists: **near duplicates** (the README's rule 3). [`NearIndex`] holds labels (the 102-wide row with its
//! loss mask) and finds one within a radius of the candidate's, in just-noticeable steps (`ds1::jnd`): the same
//! discrete part (categoricals, structure, route presence) and every continuous value both rows carry within the
//! radius. `ds1` builds one from GR3's generator targets and test presets, rendered and read by the product's own
//! intake (`pipeline::gr3_labels`) so that they are in the same convention as a sample's label, and skips a sample
//! `near_gr3_target` that has a neighbour. `exclude check` runs it again over a finished run, and against another
//! run (train against dev and test).
//!
//! ```text
//! enth-datagen exclude lists [--gr3 DIR]                          # the lists' counts and hashes
//! enth-datagen exclude check RUN_DIR [--gr3 DIR] [--against RUN_DIR]...
//! ```
//!
//! `check` re-reads every shard of `RUN_DIR` (hash against the manifest), asserts every stored seed against the lists,
//! re-draws every kept sample from `(ds1 version, stream, run seed, index)` to screen its code and fingerprint again,
//! and, with `--against`, scans the run's labels for near duplicates of the other runs' labels. Exit 0 means clean.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tooling: reads the lists and shards from files"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use enth_solver::propose::decode::OutputLayout;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ds1::jnd::{JndScale, ROUTE_DEPTH_STEP};

/// The lists, in the order their bytes are hashed.
pub const LISTS: [&str; 3] = [
    "exclude-synth.txt",
    "exclude-real.txt",
    "exclude-degraded.txt",
];

/// Every entry type the lists may hold (`content/gr3/README.md`); any other is an error.
pub const KNOWN_TYPES: [&str; 17] = [
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

/// An item that is on a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The entry type (`seed-range`, `code`, ...).
    pub kind: &'static str,
    /// What was checked.
    pub item: String,
    /// The entry it matched, as the list has it.
    pub entry: String,
}

impl fmt::Display for Hit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is on the exclusion lists: {} {}",
            self.item, self.kind, self.entry
        )
    }
}

impl std::error::Error for Hit {}

/// The three exclusion lists, parsed.
#[derive(Clone, Debug, Default)]
pub struct Exclusions {
    seed_ranges: Vec<(u64, u64)>,
    degrade_seed_ranges: Vec<(u64, u64)>,
    streams: Vec<String>,
    paths: Vec<String>,
    codes: BTreeSet<String>,
    fingerprints: BTreeSet<String>,
    preset_stems: BTreeSet<String>,
    preset_names: BTreeSet<String>,
    file_sha256: BTreeSet<String>,
    audio_sha256: BTreeSet<String>,
    freesound_ids: BTreeSet<String>,
    freesound_packs: BTreeSet<String>,
    freesound_uploaders: BTreeSet<String>,
    degrade_pools: BTreeSet<String>,
    degrade_variants: BTreeSet<String>,
    degrade_profiles: BTreeSet<String>,
    pools_sha256: Option<String>,
    /// Entries per type.
    pub counts: BTreeMap<String, usize>,
    /// SHA-256 of each list's bytes, in [`LISTS`] order.
    pub file_hashes: Vec<(String, String)>,
    /// SHA-256 of the three lists' bytes in order.
    pub sha256: [u8; 32],
}

fn range(name: &str, n: usize, value: &str) -> Result<(u64, u64), String> {
    value
        .split_once("..")
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
        .filter(|(lo, hi)| lo <= hi)
        .ok_or_else(|| format!("{name}:{n}: bad range '{value}'"))
}

impl Exclusions {
    /// Read `dir/exclude-{synth,real,degraded}.txt`.
    ///
    /// # Errors
    /// A missing list, or one [`Exclusions::parse`] refuses.
    pub fn load(dir: &Path) -> Result<Exclusions, String> {
        let mut lists = Vec::new();
        for name in LISTS {
            let path = dir.join(name);
            let bytes =
                fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            lists.push((name, bytes));
        }
        Exclusions::parse(&lists)
    }

    /// Parse lists given as `(name, bytes)`.
    ///
    /// # Errors
    /// A list that is not UTF-8, an unknown entry type (the builder is older than the list) or a bad range.
    pub fn parse(lists: &[(&str, Vec<u8>)]) -> Result<Exclusions, String> {
        let mut ex = Exclusions::default();
        let mut all = Sha256::new();
        for (name, bytes) in lists {
            all.update(bytes);
            ex.file_hashes
                .push(((*name).to_owned(), crate::ds1::anchors::sha256_hex(bytes)));
            let text = std::str::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8"))?;
            for (i, raw) in text.lines().enumerate() {
                let line = raw.split('#').next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                let n = i + 1;
                let (kind, value) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
                let value = value.trim();
                if !KNOWN_TYPES.contains(&kind) {
                    return Err(format!(
                        "{name}:{n}: unknown entry type '{kind}': this builder is older than the list"
                    ));
                }
                if value.is_empty() {
                    return Err(format!("{name}:{n}: '{kind}' has no value"));
                }
                *ex.counts.entry(kind.to_owned()).or_default() += 1;
                let v = value.to_owned();
                match kind {
                    "seed-range" => ex.seed_ranges.push(range(name, n, value)?),
                    "degrade-seed-range" => ex.degrade_seed_ranges.push(range(name, n, value)?),
                    "stream" => ex.streams.push(v),
                    "path" => ex.paths.push(v),
                    "code" => {
                        ex.codes.insert(v);
                    }
                    "fingerprint" => {
                        ex.fingerprints.insert(v.to_ascii_lowercase());
                    }
                    "preset-stem" => {
                        ex.preset_stems.insert(v);
                    }
                    "preset-name" => {
                        ex.preset_names.insert(v);
                    }
                    "file-sha256" => {
                        ex.file_sha256.insert(v.to_ascii_lowercase());
                    }
                    "audio-sha256" => {
                        ex.audio_sha256.insert(v.to_ascii_lowercase());
                    }
                    "freesound-id" => {
                        ex.freesound_ids.insert(v);
                    }
                    "freesound-pack" => {
                        ex.freesound_packs.insert(v);
                    }
                    "freesound-uploader" => {
                        ex.freesound_uploaders.insert(v);
                    }
                    "degrade-pool" => {
                        ex.degrade_pools.insert(v);
                    }
                    "degrade-variant" => {
                        ex.degrade_variants.insert(v);
                    }
                    "degrade-profile" => {
                        ex.degrade_profiles.insert(v);
                    }
                    "pools-sha256" => ex.pools_sha256 = Some(v.to_ascii_lowercase()),
                    _ => {}
                }
            }
        }
        ex.sha256 = all.finalize().into();
        Ok(ex)
    }

    fn hit(kind: &'static str, item: impl Into<String>, entry: impl Into<String>) -> Hit {
        Hit {
            kind,
            item: item.into(),
            entry: entry.into(),
        }
    }

    /// The listed challenge codes, sorted: the GR3 generator targets (test and dev) that training must not contain.
    pub fn codes(&self) -> impl Iterator<Item = &String> {
        self.codes.iter()
    }

    /// A seed in a listed closed range (`seed-range`), whatever generator, spec or stream it came from.
    pub fn seed(&self, seed: u64) -> Option<Hit> {
        self.seed_ranges
            .iter()
            .find(|&&(lo, hi)| (lo..=hi).contains(&seed))
            .map(|(lo, hi)| Self::hit("seed-range", format!("seed {seed}"), format!("{lo}..{hi}")))
    }

    /// A stream key on the list (`stream`; a trailing `*` is a prefix).
    pub fn stream(&self, key: &str) -> Option<Hit> {
        self.streams
            .iter()
            .find(|s| match s.strip_suffix('*') {
                Some(prefix) => key.starts_with(prefix),
                None => key == s.as_str(),
            })
            .map(|s| Self::hit("stream", format!("stream {key}"), s.clone()))
    }

    /// A challenge code on the list (`code`).
    pub fn code(&self, code: &str) -> Option<Hit> {
        self.codes
            .contains(code)
            .then(|| Self::hit("code", format!("code {code}"), code))
    }

    /// A target fingerprint (64 lowercase hex digits) on the list (`fingerprint`).
    pub fn fingerprint(&self, hex: &str) -> Option<Hit> {
        let h = hex.to_ascii_lowercase();
        self.fingerprints
            .contains(&h)
            .then(|| Self::hit("fingerprint", format!("fingerprint {h}"), h))
    }

    /// A preset by file stem, `meta.name` and the SHA-256 of its file (`preset-stem`, `preset-name`, `file-sha256`).
    pub fn preset(&self, stem: &str, name: &str, sha256: &str) -> Option<Hit> {
        let item = format!("preset {stem} ({name})");
        if self.preset_stems.contains(stem) {
            return Some(Self::hit("preset-stem", item, stem));
        }
        if self.preset_names.contains(name) {
            return Some(Self::hit("preset-name", item, name));
        }
        let h = sha256.to_ascii_lowercase();
        self.file_sha256
            .contains(&h)
            .then(|| Self::hit("file-sha256", item, h))
    }

    /// A path under a listed `path` pattern (`**` after a directory is "anything below"; `~` is the home directory).
    pub fn path(&self, path: &str) -> Option<Hit> {
        let home = std::env::var("HOME").unwrap_or_default();
        self.paths
            .iter()
            .find(|p| {
                let pat = match p.strip_prefix("~/") {
                    Some(rest) if !home.is_empty() => format!("{home}/{rest}"),
                    _ => (*p).clone(),
                };
                match pat.strip_suffix("/**") {
                    Some(dir) => path == dir || path.starts_with(&format!("{dir}/")),
                    None => path == pat,
                }
            })
            .map(|p| Self::hit("path", format!("path {path}"), p.clone()))
    }

    /// Audio bytes on the list (`audio-sha256`).
    pub fn audio(&self, sha256: &str) -> Option<Hit> {
        let h = sha256.to_ascii_lowercase();
        self.audio_sha256
            .contains(&h)
            .then(|| Self::hit("audio-sha256", format!("audio {h}"), h))
    }

    /// A Freesound sound by id or pack (`freesound-id`, `freesound-pack`), or by uploader (`freesound-uploader`, which
    /// the README calls advisory).
    #[allow(
        dead_code,
        reason = "ds1 renders its own patches and no recording has a Freesound id; the lists' real-audio entries are asserted here for the bridge's records and a later real-audio lane"
    )]
    pub fn freesound(&self, id: &str, pack: Option<&str>, uploader: Option<&str>) -> Option<Hit> {
        if self.freesound_ids.contains(id) {
            return Some(Self::hit("freesound-id", format!("sound {id}"), id));
        }
        if let Some(p) = pack.filter(|p| self.freesound_packs.contains(*p)) {
            return Some(Self::hit("freesound-pack", format!("sound {id}"), p));
        }
        uploader
            .filter(|u| self.freesound_uploaders.contains(*u))
            .map(|u| Self::hit("freesound-uploader", format!("sound {id}"), u))
    }

    /// A degradation record (`enth.degrade-record/1` or `enth.augment-record/1`, parsed) of a clip that goes into
    /// training: `pool` must be `train` (and not a listed pool), its profile, variants and seed must not be listed,
    /// and no recorded source may be a listed file or audio hash. Returns warnings (a pools file other than the one the
    /// lists were frozen from: the names may then be stale).
    ///
    /// # Errors
    /// The first [`Hit`].
    pub fn degrade_record(&self, record: &Value) -> Result<Vec<String>, Hit> {
        let mut warnings = Vec::new();
        let pool = record.get("pool").and_then(Value::as_str);
        match pool {
            Some("train") => {}
            Some(p) => {
                let entry = if self.degrade_pools.contains(p) {
                    p.to_owned()
                } else {
                    "train only".to_owned()
                };
                return Err(Self::hit("degrade-pool", format!("pool {p}"), entry));
            }
            None => warnings.push("the record names no pool".to_owned()),
        }
        if let Some(p) = record.get("profile").and_then(Value::as_str)
            && self.degrade_profiles.contains(p)
        {
            return Err(Self::hit("degrade-profile", format!("profile {p}"), p));
        }
        if let Some(seed) = record.get("seed").and_then(Value::as_u64) {
            if let Some((lo, hi)) = self
                .degrade_seed_ranges
                .iter()
                .find(|&&(lo, hi)| (lo..=hi).contains(&seed))
            {
                return Err(Self::hit(
                    "degrade-seed-range",
                    format!("seed {seed}"),
                    format!("{lo}..{hi}"),
                ));
            }
            if let Some(h) = self.seed(seed) {
                return Err(h);
            }
        }
        for stage in record
            .get("stages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(v) = stage.get("variant").and_then(Value::as_str)
                && self.degrade_variants.contains(v)
            {
                return Err(Self::hit("degrade-variant", format!("variant {v}"), v));
            }
        }
        for source in record
            .get("sources_used")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(h) = source
                .get("sha256")
                .and_then(Value::as_str)
                .and_then(|s| self.audio(s))
            {
                return Err(h);
            }
            if let Some(h) = source
                .get("path")
                .and_then(Value::as_str)
                .and_then(|s| self.path(s))
            {
                return Err(h);
            }
        }
        if let (Some(frozen), Some(have)) = (
            self.pools_sha256.as_deref(),
            record
                .get("pools")
                .and_then(|p| p.get("sha256"))
                .and_then(Value::as_str),
        ) && frozen != have.to_ascii_lowercase()
        {
            warnings.push(format!(
                "pools.toml is {have}, not {frozen} that the lists were frozen from: held-out names may be stale"
            ));
        }
        Ok(warnings)
    }

    /// What a manifest records of the lists.
    pub fn summary(&self) -> Value {
        let files: BTreeMap<&str, &str> = self
            .file_hashes
            .iter()
            .map(|(n, h)| (n.as_str(), h.as_str()))
            .collect();
        json!({
            "sha256": crate::ds1::anchors::hex(&self.sha256),
            "files": files,
            "entries": self.counts,
            "asserted": [
                "seed-range: every sample's seed and generator seed",
                "stream: the run's stream key (training streams only)",
                "code, fingerprint: every generator-v1 sample",
                "preset-stem, preset-name, file-sha256: every anchor, at construction",
                "near duplicates: every sample against GR3's generator targets and test presets, in label space",
            ],
            "not_applicable": "path, audio-sha256, freesound-* and degrade-*: ds1 renders its own patches (the bridge checks clips and records)",
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Labels: exact hashes and near duplicates
// ---------------------------------------------------------------------------------------------------------------

/// SHA-256 of a **masked canonical label**: the mask, then the label with every value the mask carries no loss on
/// zeroed. Two samples with the same hash train the model on the same target (dedup's first step).
pub fn label_hash(label: &[f32], mask: &[f32]) -> [u8; 32] {
    let mut h = Sha256::new();
    for m in mask {
        h.update(m.to_le_bytes());
    }
    for (v, m) in label.iter().zip(mask) {
        let v = if *m > 0.0 { *v } else { 0.0 };
        h.update(v.to_le_bytes());
    }
    h.finalize().into()
}

/// Labels to find near duplicates in (see the module docs).
#[derive(Clone, Debug)]
pub struct NearIndex {
    radius: f32,
    jnd: JndScale,
    /// `(offset, table row)` of the continuous entries.
    continuous: Vec<(usize, usize)>,
    /// `(offset, len)` of the categorical groups.
    categorical: Vec<(usize, usize)>,
    enables: Vec<usize>,
    /// `(presence, depth)` of the route grid.
    routes: Vec<(usize, usize)>,
    buckets: HashMap<u32, Vec<usize>>,
    items: Vec<(String, Vec<f32>, Vec<f32>)>,
}

impl NearIndex {
    /// An empty index for `layout`; two labels are near when they differ by at most `radius` steps.
    pub fn new(layout: &OutputLayout, radius: f32) -> NearIndex {
        NearIndex {
            radius,
            jnd: JndScale::build(layout),
            continuous: layout
                .continuous()
                .iter()
                .enumerate()
                .map(|(i, e)| (e.offset, i))
                .collect(),
            categorical: layout
                .categorical()
                .iter()
                .map(|e| (e.offset, e.len()))
                .collect(),
            enables: layout.enables().iter().map(|e| e.offset).collect(),
            routes: layout
                .routes()
                .iter()
                .map(|c| (c.presence, c.depth))
                .collect(),
            buckets: HashMap::new(),
            items: Vec::new(),
        }
    }

    /// The bucket of a label: its structure switches and route presence (which near labels share).
    fn key(&self, label: &[f32]) -> u32 {
        let mut k = 0u32;
        for &o in &self.enables {
            k = (k << 1) | u32::from(label[o] > 0.5);
        }
        for &(p, _) in &self.routes {
            k = (k << 1) | u32::from(label[p] > 0.5);
        }
        k
    }

    /// Add a label with its loss mask under a name.
    pub fn add(&mut self, id: impl Into<String>, label: &[f32], mask: &[f32]) {
        let k = self.key(label);
        self.buckets.entry(k).or_default().push(self.items.len());
        self.items.push((id.into(), label.to_vec(), mask.to_vec()));
    }

    /// How many labels the index holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The worst difference in steps between two labels, or `None` when they differ by more than the radius in any
    /// entry both carry a loss on, or in a discrete entry.
    fn compare(&self, a: (&[f32], &[f32]), b: (&[f32], &[f32])) -> Option<f32> {
        let ((la, ma), (lb, mb)) = (a, b);
        for &(o, len) in &self.categorical {
            if ma[o] > 0.0 && mb[o] > 0.0 && argmax(&la[o..o + len]) != argmax(&lb[o..o + len]) {
                return None;
            }
        }
        let mut worst = 0.0f32;
        for &(p, d) in &self.routes {
            if (la[p] > 0.5) != (lb[p] > 0.5) {
                return None;
            }
            if la[p] > 0.5 && ma[d] > 0.0 && mb[d] > 0.0 {
                let steps = (la[d] - lb[d]).abs() / ROUTE_DEPTH_STEP;
                if steps > self.radius {
                    return None;
                }
                worst = worst.max(steps);
            }
        }
        for &(o, i) in &self.continuous {
            if ma[o] > 0.0 && mb[o] > 0.0 {
                let steps = (la[o] - lb[o]).abs() / self.jnd.at(i, lb[o]).max(1e-6);
                if steps > self.radius {
                    return None;
                }
                worst = worst.max(steps);
            }
        }
        Some(worst)
    }

    /// The nearest item within the radius: its name and the worst difference in steps.
    pub fn near(&self, label: &[f32], mask: &[f32]) -> Option<(&str, f32)> {
        let k = self.key(label);
        let mut best: Option<(&str, f32)> = None;
        for &i in self.buckets.get(&k)? {
            let (id, l, m) = &self.items[i];
            // The structure switches are part of the bucket; they are exact.
            if let Some(d) = self.compare((label, mask), (l, m))
                && best.is_none_or(|(_, b)| d < b)
            {
                best = Some((id.as_str(), d));
            }
        }
        best
    }
}

fn argmax(v: &[f32]) -> usize {
    let mut best = 0;
    for (i, x) in v.iter().enumerate() {
        if *x > v[best] {
            best = i;
        }
    }
    best
}

// ---------------------------------------------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------------------------------------------

fn opts(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

/// The default `content/gr3`.
pub fn default_gr3() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/gr3")
}

/// `exclude`: `lists` or `check`.
pub fn run(args: &[String]) -> Result<(), String> {
    let gr3 = opts(args, "--gr3")
        .pop()
        .map_or_else(default_gr3, PathBuf::from);
    match args.first().map(String::as_str) {
        Some("lists") => {
            let ex = Exclusions::load(&gr3)?;
            println!("{}", serde_json::to_string_pretty(&ex.summary()).map_err(|e| e.to_string())?);
            Ok(())
        }
        Some("check") => {
            let run_dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("usage: exclude check RUN_DIR [--gr3 DIR] [--against RUN_DIR]...")?;
            let against: Vec<PathBuf> = opts(args, "--against")
                .into_iter()
                .map(PathBuf::from)
                .collect();
            crate::ds1::check_run(Path::new(run_dir), &gr3, &against)
        }
        _ => Err("usage: exclude lists [--gr3 DIR] | exclude check RUN_DIR [--gr3 DIR] [--against RUN_DIR]...".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists(synth: &str, real: &str, degraded: &str) -> Exclusions {
        Exclusions::parse(&[
            (LISTS[0], synth.as_bytes().to_vec()),
            (LISTS[1], real.as_bytes().to_vec()),
            (LISTS[2], degraded.as_bytes().to_vec()),
        ])
        .unwrap()
    }

    const SYNTH: &str = "\
# a comment
seed-range 770000000000..770999999999  # GR3's block
stream r3/test
stream gr3/*
path content/gr3/**
path content/presets/bass.patch.json
code 1040-1PD3-S7ER-H4  # gen-t-l1-01
fingerprint FA5E007ADB3B25E883D1FDEA886412096F7EC38923B68A1BF68007AC9B65EE9F
preset-stem acid-snap
preset-name Acid Snap
file-sha256 AE40658B07EAE698D9B2D797CDA7F0ED95C36F683E873DB472562A2A8441FFBE  # a path
";

    #[test]
    fn every_entry_type_is_enforced() {
        let ex = lists(
            SYNTH,
            "freesound-id 213657\nfreesound-pack 55\nfreesound-uploader sgossner\naudio-sha256 AB\n",
            "",
        );
        // seeds: the closed range, both ends
        assert!(ex.seed(770_000_000_000).is_some());
        assert!(ex.seed(770_999_999_999).is_some());
        assert!(ex.seed(769_999_999_999).is_none());
        assert!(ex.seed(771_000_000_000).is_none());
        // streams: exact and prefix
        assert!(ex.stream("r3/test").is_some());
        assert!(ex.stream("gr3/anything").is_some());
        assert!(ex.stream("r3/train").is_none());
        assert!(ex.stream("r3/test2").is_none());
        // codes and fingerprints, case-insensitively for hex
        assert!(ex.code("1040-1PD3-S7ER-H4").is_some());
        assert!(ex.code("1040-1PD3-S7ER-J2").is_none());
        assert!(
            ex.fingerprint("fa5e007adb3b25e883d1fdea886412096f7ec38923b68a1bf68007ac9b65ee9f")
                .is_some()
        );
        // presets by stem, by name and by file hash
        assert_eq!(
            ex.preset("acid-snap", "x", "00").unwrap().kind,
            "preset-stem"
        );
        assert_eq!(
            ex.preset("renamed", "Acid Snap", "00").unwrap().kind,
            "preset-name"
        );
        assert_eq!(
            ex.preset(
                "renamed",
                "Other",
                "ae40658b07eae698d9b2d797cda7f0ed95c36f683e873db472562a2a8441ffbe"
            )
            .unwrap()
            .kind,
            "file-sha256"
        );
        assert!(ex.preset("warm-reese", "Warm Reese", "00").is_none());
        // paths: below a directory, exact files
        assert!(ex.path("content/gr3/presets/aurora.patch.json").is_some());
        assert!(ex.path("content/gr3").is_some());
        assert!(ex.path("content/gr3x/a").is_none());
        assert!(ex.path("content/presets/bass.patch.json").is_some());
        // real audio
        assert!(ex.audio("ab").is_some());
        assert!(ex.freesound("213657", None, None).is_some());
        assert!(ex.freesound("1", Some("55"), None).is_some());
        assert!(ex.freesound("1", None, Some("sgossner")).is_some());
        assert!(ex.freesound("1", Some("56"), Some("x")).is_none());
    }

    #[test]
    fn an_unknown_entry_type_or_a_bad_range_is_an_error() {
        let bad = Exclusions::parse(&[(LISTS[0], b"future-type x".to_vec())]).unwrap_err();
        assert!(bad.contains("unknown entry type 'future-type'"), "{bad}");
        let bad = Exclusions::parse(&[(LISTS[0], b"seed-range 9..1".to_vec())]).unwrap_err();
        assert!(bad.contains("bad range"), "{bad}");
        let bad = Exclusions::parse(&[(LISTS[0], b"code".to_vec())]).unwrap_err();
        assert!(bad.contains("no value"), "{bad}");
    }

    #[test]
    fn a_degradation_record_is_screened() {
        let ex = lists(
            "",
            "",
            "degrade-pool heldout\ndegrade-profile mp3-heldout\ndegrade-seed-range 770100000000..770299999999\ndegrade-variant heldout/room/hall\npools-sha256 AA\n",
        );
        let ok = json!({"pool":"train","profile":null,"seed":7,"stages":[{"stage":"room","variant":"train/room/small"}],"pools":{"sha256":"aa"}});
        assert_eq!(ex.degrade_record(&ok).unwrap(), Vec::<String>::new());
        let stale = json!({"pool":"train","seed":7,"pools":{"sha256":"bb"}});
        assert_eq!(ex.degrade_record(&stale).unwrap().len(), 1);
        for (rec, kind) in [
            (json!({"pool":"heldout"}), "degrade-pool"),
            (
                json!({"pool":"train","profile":"mp3-heldout"}),
                "degrade-profile",
            ),
            (
                json!({"pool":"train","seed":770_100_000_001u64}),
                "degrade-seed-range",
            ),
            (
                json!({"pool":"train","stages":[{"variant":"heldout/room/hall"}]}),
                "degrade-variant",
            ),
        ] {
            assert_eq!(ex.degrade_record(&rec).unwrap_err().kind, kind, "{rec}");
        }
        let src = lists("", "audio-sha256 AB\n", "");
        let rec = json!({"pool":"train","sources_used":[{"sha256":"ab"}]});
        assert_eq!(src.degrade_record(&rec).unwrap_err().kind, "audio-sha256");
    }

    #[test]
    fn the_real_lists_parse_and_hold_gr3s_block() {
        let ex = Exclusions::load(&default_gr3()).unwrap();
        assert!(ex.seed(770_000_000_000).is_some() && ex.seed(770_999_999_999).is_some());
        assert!(ex.stream("r3/test").is_some() && ex.stream("gr3/dev").is_some());
        assert!(ex.stream("r3/train").is_none() && ex.stream("r3/dev").is_none());
        assert!(ex.counts["code"] >= 150 && ex.counts["fingerprint"] >= 150);
        assert_eq!(ex.counts["preset-stem"], 11);
        assert_eq!(ex.counts["file-sha256"], 11);
    }

    #[test]
    fn label_hash_sees_only_what_the_mask_carries() {
        let (a, b) = (vec![1.0, 2.0, 3.0], vec![1.0, 9.0, 3.0]);
        let mask = vec![1.0, 0.0, 1.0];
        assert_eq!(label_hash(&a, &mask), label_hash(&b, &mask));
        assert_ne!(label_hash(&a, &mask), label_hash(&a, &[1.0, 1.0, 1.0]));
        let c = vec![1.0, 2.0, 4.0];
        assert_ne!(label_hash(&a, &mask), label_hash(&c, &mask));
    }

    fn row(layout: &OutputLayout) -> (Vec<f32>, Vec<f32>) {
        let mut v = vec![0.0f32; layout.width()];
        for e in layout.continuous() {
            v[e.offset] = 0.5;
        }
        for e in layout.categorical() {
            v[e.offset] = 1.0;
        }
        for c in layout.routes() {
            v[c.depth] = 0.5;
        }
        (v, vec![1.0; layout.width()])
    }

    #[test]
    fn a_near_label_is_found_within_the_radius_and_not_beyond() {
        let layout = OutputLayout::v1();
        let jnd = JndScale::build(&layout);
        let mut index = NearIndex::new(&layout, 1.0);
        let (label, mask) = row(&layout);
        index.add("target", &label, &mask);
        assert_eq!(index.len(), 1);
        // The same label: distance 0.
        assert_eq!(index.near(&label, &mask), Some(("target", 0.0)));
        // 0.8 steps away in one value: near.
        let i = layout
            .continuous()
            .iter()
            .position(|e| e.addr.to_string() == "filter1.cutoff")
            .unwrap();
        let o = layout.continuous()[i].offset;
        let mut near = label.clone();
        near[o] += 0.8 * jnd.at(i, 0.5);
        let (id, d) = index.near(&near, &mask).unwrap();
        assert_eq!(id, "target");
        assert!((d - 0.8).abs() < 0.05, "{d}");
        // 1.5 steps away: not near.
        let mut far = label.clone();
        far[o] += 1.5 * jnd.at(i, 0.5);
        assert!(index.near(&far, &mask).is_none());
        // A value the candidate's mask does not carry is not compared.
        let mut masked = mask.clone();
        masked[o] = 0.0;
        assert!(index.near(&far, &masked).is_some());
        // A different categorical item is not near, however close the rest.
        let mut other = label.clone();
        let wave = layout
            .categorical()
            .iter()
            .find(|e| e.addr.to_string() == "osc1.wave")
            .unwrap();
        other[wave.offset] = 0.0;
        other[wave.offset + 1] = 2.0;
        assert!(index.near(&other, &mask).is_none());
        // A different structure switch lands in another bucket.
        let mut on = label.clone();
        on[layout.enables()[0].offset] = 1.0;
        assert!(index.near(&on, &mask).is_none());
    }
}
