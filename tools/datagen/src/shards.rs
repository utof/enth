//! **Owner: lane DG** (the dataset generator, R3b). `shards`: the files lane TR reads (docs/r3.md, "`ds1`" and "The
//! model contract (F3s)", "Who computes what").
//!
//! A **shard** is a pure function of the sampler version, its stream key, the run seed and its index range, so it
//! can be re-rendered bit for bit and 5 to 10 % of the shards are re-rendered per epoch instead of cached: shard `k`
//! of a run covers the candidate indices `k * shard_size .. (k + 1) * shard_size` of the stream, and keeps the ones
//! that were not skipped. The same shard comes out for any worker count. The CPU is the bottleneck, so a shard is
//! also the unit of resumption (`shard-000042.json` is written after `shard-000042.bin`, and a rerun skips a shard
//! whose file still has the recorded SHA-256).
//!
//! # The file (format 1)
//!
//! Little-endian throughout; arrays are structure-of-arrays, so `numpy.fromfile` (or `array.array`) reads each one
//! with a single call.
//!
//! ```text
//! bytes 0..8    magic "ENTHSHRD"
//! bytes 8..12   u32 format = 1
//! bytes 12..16  u32 H, the length of the header JSON
//! bytes 16..16+H   the header: UTF-8 JSON, padded with spaces up to byte 1024
//! byte 1024...  the arrays, in the order the header lists them, each starting at a multiple of 64
//! ```
//!
//! The header (`enth.ds1-shard/1`) has the sample count `n` and, per array, its `name`, numpy `dtype`, `shape`, `offset`
//! (from the start of the file) and `bytes`:
//!
//! | Array | dtype | Shape | What |
//! | --- | --- | --- | --- |
//! | `mel` | `<f4` | `[n, 3, 128, 64]` | feature set 1's log-mel (resolution, block of 1/64 s, band) |
//! | `context` | `<f4` | `[n, 5]` | the told key, the pitch offset, pitched, the hold, the band limit |
//! | `label` | `<f4` | `[n, 102]` | layout 1's row: normalised continuous values, one-hot categoricals, structure and route presence as 0 or 1, route depth as `(depth / 100 + 1) / 2` |
//! | `mask` | `<f4` | `[n, 102]` | 1.0 where the entry carries a loss, else 0.0 |
//! | `index` | `<u8` | `[n]` | the sample's index in its stream |
//! | `seed` | `<u8` | `[n]` | the sample's seed (`sample_seed`) |
//! | `gen_seed` | `<u8` | `[n]` | the generator-v1 seed of a `v1` sample, else 0 |
//! | `rate` | `<u4` | `[n]` | the clip's sample rate (Hz) |
//! | `source` | `\|u1` | `[n]` | 0 `v1`, 1 `preset`, 2 `uniform` |
//!
//! The run's `manifest.json` (`enth.ds1-shards/1`) holds what a model's header copies (ds1 version, feature set,
//! layout id and SHA-256, generator, template, engine), the 102 entry names, the per-entry JND scale (and the
//! 33 x 17 table behind it), the seeds, the counts of skips by code, and per shard the file, counts and SHA-256.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tooling: reads and writes the shard files"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The file's first eight bytes.
pub const MAGIC: [u8; 8] = *b"ENTHSHRD";
/// The shard format.
pub const FORMAT: u32 = 1;
/// Where the arrays start: the header is padded up to here.
pub const DATA_START: usize = 1024;
/// Every array starts at a multiple of this many bytes.
pub const ALIGN: usize = 64;
/// `mel`'s values per sample: `[3, 128, 64]`.
pub const MEL_LEN: usize = 3 * 128 * 64;
/// `context`'s values per sample.
pub const CONTEXT_LEN: usize = 5;
/// `label`'s and `mask`'s values per sample: layout 1's width.
pub const ROW_LEN: usize = 102;

/// The shard header's schema.
pub const HEADER_SCHEMA: &str = "enth.ds1-shard/1";
/// The manifest's schema.
pub const MANIFEST_SCHEMA: &str = "enth.ds1-shards/1";

/// The samples of a shard, array by array.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    /// `[n, 3, 128, 64]`.
    pub mel: Vec<f32>,
    /// `[n, 5]`.
    pub context: Vec<f32>,
    /// `[n, 102]`.
    pub label: Vec<f32>,
    /// `[n, 102]`, 0.0 or 1.0.
    pub mask: Vec<f32>,
    /// `[n]`.
    pub index: Vec<u64>,
    /// `[n]`.
    pub seed: Vec<u64>,
    /// `[n]`.
    pub gen_seed: Vec<u64>,
    /// `[n]`.
    pub rate: Vec<u32>,
    /// `[n]`.
    pub source: Vec<u8>,
}

impl Rows {
    /// The number of samples.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// One sample's label row and mask.
    pub fn row(&self, i: usize) -> (&[f32], &[f32]) {
        (
            &self.label[i * ROW_LEN..(i + 1) * ROW_LEN],
            &self.mask[i * ROW_LEN..(i + 1) * ROW_LEN],
        )
    }
}

/// What the header says of the shard besides its arrays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// The sampler's version.
    pub ds1: u16,
    /// The stream key (`r3/train`).
    pub stream: String,
    /// The run seed.
    pub run_seed: u64,
    /// The shard's number.
    pub shard: u32,
    /// The first candidate index it covers.
    pub first_index: u64,
    /// How many candidate indices it covers (some are skipped).
    pub candidates: u32,
}

/// One array in the header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrayInfo {
    /// Its name.
    pub name: String,
    /// The numpy dtype string.
    pub dtype: String,
    /// The shape, `n` first.
    pub shape: Vec<u64>,
    /// Where it starts, in bytes from the start of the file.
    pub offset: u64,
    /// How long it is, in bytes.
    pub bytes: u64,
}

/// The header block as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Header {
    schema: String,
    format: u32,
    #[serde(flatten)]
    meta: Meta,
    n: u64,
    arrays: Vec<ArrayInfo>,
}

/// The arrays' names, dtypes and per-sample shapes, in file order.
pub const ARRAYS: [(&str, &str, &[u64]); 9] = [
    ("mel", "<f4", &[3, 128, 64]),
    ("context", "<f4", &[CONTEXT_LEN as u64]),
    ("label", "<f4", &[ROW_LEN as u64]),
    ("mask", "<f4", &[ROW_LEN as u64]),
    ("index", "<u8", &[]),
    ("seed", "<u8", &[]),
    ("gen_seed", "<u8", &[]),
    ("rate", "<u4", &[]),
    ("source", "|u1", &[]),
];

fn width(dtype: &str) -> usize {
    match dtype {
        "<f4" | "<u4" => 4,
        "<u8" => 8,
        _ => 1,
    }
}

fn align(x: usize) -> usize {
    x.div_ceil(ALIGN) * ALIGN
}

fn infos(n: usize) -> Vec<ArrayInfo> {
    let mut at = DATA_START;
    ARRAYS
        .iter()
        .map(|(name, dtype, per)| {
            let mut shape = vec![n as u64];
            shape.extend_from_slice(per);
            let bytes = shape.iter().product::<u64>() as usize * width(dtype);
            let info = ArrayInfo {
                name: (*name).to_owned(),
                dtype: (*dtype).to_owned(),
                shape,
                offset: at as u64,
                bytes: bytes as u64,
            };
            at = align(at + bytes);
            info
        })
        .collect()
}

/// The shard file's bytes: deterministic, a pure function of `meta` and `rows`.
///
/// # Panics
/// If `rows` is not consistent (array lengths that do not match the sample count).
pub fn encode(meta: &Meta, rows: &Rows) -> Vec<u8> {
    let n = rows.len();
    assert_eq!(rows.mel.len(), n * MEL_LEN, "mel");
    assert_eq!(rows.context.len(), n * CONTEXT_LEN, "context");
    assert_eq!(rows.label.len(), n * ROW_LEN, "label");
    assert_eq!(rows.mask.len(), n * ROW_LEN, "mask");
    assert!(
        [
            rows.seed.len(),
            rows.gen_seed.len(),
            rows.rate.len(),
            rows.source.len()
        ]
        .iter()
        .all(|l| *l == n)
    );
    let arrays = infos(n);
    let header = Header {
        schema: HEADER_SCHEMA.to_owned(),
        format: FORMAT,
        meta: meta.clone(),
        n: n as u64,
        arrays: arrays.clone(),
    };
    let json = serde_json::to_vec(&header).expect("the header serialises");
    assert!(
        16 + json.len() <= DATA_START,
        "the header outgrew its block"
    );
    let end = arrays
        .last()
        .map_or(DATA_START, |a| (a.offset + a.bytes) as usize);
    let mut out = vec![b' '; DATA_START];
    out[0..8].copy_from_slice(&MAGIC);
    out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
    out[12..16].copy_from_slice(&(json.len() as u32).to_le_bytes());
    out[16..16 + json.len()].copy_from_slice(&json);
    out.resize(end, 0);
    let mut put = |name: &str, bytes: Vec<u8>| {
        let a = arrays.iter().find(|a| a.name == name).expect("listed");
        out[a.offset as usize..a.offset as usize + bytes.len()].copy_from_slice(&bytes);
    };
    let f32s = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let u64s = |v: &[u64]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    put("mel", f32s(&rows.mel));
    put("context", f32s(&rows.context));
    put("label", f32s(&rows.label));
    put("mask", f32s(&rows.mask));
    put("index", u64s(&rows.index));
    put("seed", u64s(&rows.seed));
    put("gen_seed", u64s(&rows.gen_seed));
    put(
        "rate",
        rows.rate.iter().flat_map(|x| x.to_le_bytes()).collect(),
    );
    put("source", rows.source.clone());
    out
}

/// Read a shard file.
///
/// # Errors
/// A file that is not format 1 or whose header and arrays disagree.
pub fn decode(bytes: &[u8]) -> Result<(Meta, Rows), String> {
    if bytes.len() < DATA_START || bytes[0..8] != MAGIC {
        return Err("not a shard file".to_owned());
    }
    let format = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    if format != FORMAT {
        return Err(format!("shard format {format}; this reader reads {FORMAT}"));
    }
    let hlen = u32::from_le_bytes(bytes[12..16].try_into().expect("4 bytes")) as usize;
    if 16 + hlen > DATA_START {
        return Err("the header is longer than its block".to_owned());
    }
    let header: Header =
        serde_json::from_slice(&bytes[16..16 + hlen]).map_err(|e| e.to_string())?;
    if header.schema != HEADER_SCHEMA || header.format != FORMAT {
        return Err(format!("unknown shard header {}", header.schema));
    }
    let n = header.n as usize;
    if header.arrays != infos(n) {
        return Err("the header's arrays are not format 1's".to_owned());
    }
    let end = header
        .arrays
        .last()
        .map_or(DATA_START, |a| (a.offset + a.bytes) as usize);
    if bytes.len() != end {
        return Err(format!(
            "the file is {} bytes; the header says {end}",
            bytes.len()
        ));
    }
    let slice = |name: &str| -> &[u8] {
        let a = header
            .arrays
            .iter()
            .find(|a| a.name == name)
            .expect("listed");
        &bytes[a.offset as usize..(a.offset + a.bytes) as usize]
    };
    let f32s = |name: &str| -> Vec<f32> {
        slice(name)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    };
    let u64s = |name: &str| -> Vec<u64> {
        slice(name)
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_le_bytes(*c))
            .collect()
    };
    let rows = Rows {
        mel: f32s("mel"),
        context: f32s("context"),
        label: f32s("label"),
        mask: f32s("mask"),
        index: u64s("index"),
        seed: u64s("seed"),
        gen_seed: u64s("gen_seed"),
        rate: slice("rate")
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect(),
        source: slice("source").to_vec(),
    };
    Ok((header.meta, rows))
}

/// SHA-256 of bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::ds1::anchors::hex(&Sha256::digest(bytes))
}

/// The file name of shard `k`.
pub fn shard_name(k: u32) -> String {
    format!("shard-{k:06}.bin")
}

/// What a run records of one shard (also its `.json` sidecar).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardStats {
    /// The file's name.
    pub file: String,
    /// The shard's number.
    pub shard: u32,
    /// The first candidate index.
    pub first_index: u64,
    /// The candidate indices covered.
    pub candidates: u32,
    /// The samples kept.
    pub kept: u32,
    /// The file's size.
    pub bytes: u64,
    /// The file's SHA-256 (lowercase hex).
    pub sha256: String,
    /// The candidates skipped, by code.
    pub skips: BTreeMap<String, u64>,
    /// The samples kept, by source (`v1`, `preset`, `uniform`).
    pub sources: BTreeMap<String, u32>,
}

/// The run manifest (`manifest.json`): see the module docs. The descriptive blocks are JSON values so that a model's
/// exporter copies them without a schema of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`MANIFEST_SCHEMA`].
    pub schema: String,
    /// `clean`: shards of the sampler's own renders, a window of candidate indices each; `clips`: shards of files
    /// (degraded copies of renders) that `shards features` read, a run of lines of the samples file each.
    pub kind: String,
    /// The sampler's version.
    pub ds1: u16,
    /// The stream key.
    pub stream: String,
    /// `train`, `dev` or `test`.
    pub role: String,
    /// The run seed.
    pub run_seed: u64,
    /// The candidates the run covers (`--count`).
    pub count: u64,
    /// Candidates per shard.
    pub shard_size: u32,
    /// Samples kept in all.
    pub kept: u64,
    /// Candidates skipped in all, by code.
    pub skips: BTreeMap<String, u64>,
    /// Samples kept in all, by source.
    pub sources: BTreeMap<String, u64>,
    /// The engine that rendered the clips.
    pub engine: String,
    /// Feature set 1: id, name, shapes.
    pub feature_set: Value,
    /// Layout 1: id, name, generator, template, width, SHA-256, entry names.
    pub layout: Value,
    /// The loss scale: `scale` [102], `table` [33][17].
    pub jnd: Value,
    /// The arrays of a shard.
    pub arrays: Value,
    /// The sampler: mixture, rates, phrase, note noise, anchors.
    pub sampler: Value,
    /// The exclusion lists' hashes, what is asserted, and the GR3 references.
    pub exclusions: Value,
    /// For a `clips` run: the samples file's hash and line count; else null.
    #[serde(default)]
    pub bridge: Value,
    /// Things a reader should know that are not errors (a degradation record with a stale pool hash, ...).
    pub warnings: Vec<String>,
    /// The shards.
    pub shards: Vec<ShardStats>,
}

/// Write `bytes` to `path` through a temporary file, so a crash never leaves half a shard under its name.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("cannot rename {}: {e}", tmp.display()))
}

/// Read `dir/manifest.json`.
pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let path = dir.join("manifest.json");
    let text =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let m: Manifest =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.schema != MANIFEST_SCHEMA {
        return Err(format!(
            "{}: schema {}, not {MANIFEST_SCHEMA}",
            path.display(),
            m.schema
        ));
    }
    Ok(m)
}

/// Re-hash and re-read every shard of a run.
///
/// # Errors
/// The first shard whose hash, header or arrays are wrong.
pub fn verify(dir: &Path) -> Result<Manifest, String> {
    let m = read_manifest(dir)?;
    let mut kept = 0u64;
    let mut skips: BTreeMap<String, u64> = BTreeMap::new();
    let mut sources: BTreeMap<String, u64> = BTreeMap::new();
    let clean = m.kind == "clean";
    for s in &m.shards {
        let path = dir.join(&s.file);
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if sha256_hex(&bytes) != s.sha256 || bytes.len() as u64 != s.bytes {
            return Err(format!("{}: the bytes are not the manifest's", s.file));
        }
        let (meta, rows) = decode(&bytes).map_err(|e| format!("{}: {e}", s.file))?;
        if meta.ds1 != m.ds1
            || meta.stream != m.stream
            || meta.run_seed != m.run_seed
            || meta.shard != s.shard
        {
            return Err(format!("{}: its header is not the run's", s.file));
        }
        if rows.len() != s.kept as usize
            || meta.first_index != s.first_index
            || meta.candidates != s.candidates
        {
            return Err(format!("{}: its counts are not the manifest's", s.file));
        }
        let skipped: u64 = s.skips.values().sum();
        if skipped + u64::from(s.kept) != u64::from(s.candidates) {
            return Err(format!("{}: kept + skipped is not the candidates", s.file));
        }
        if clean {
            for (i, idx) in rows.index.iter().enumerate() {
                if !(s.first_index..s.first_index + u64::from(s.candidates)).contains(idx) {
                    return Err(format!(
                        "{}: sample {i} has index {idx} outside the shard",
                        s.file
                    ));
                }
            }
            if rows.index.windows(2).any(|w| w[0] >= w[1]) {
                return Err(format!("{}: indices are not increasing", s.file));
            }
        }
        let by_source: u32 = s.sources.values().sum();
        if by_source != s.kept {
            return Err(format!(
                "{}: the sources do not add up to the samples kept",
                s.file
            ));
        }
        if rows
            .mel
            .iter()
            .chain(&rows.context)
            .chain(&rows.label)
            .any(|v| !v.is_finite())
        {
            return Err(format!("{}: a non-finite feature or label", s.file));
        }
        if rows.mask.iter().any(|m| *m != 0.0 && *m != 1.0) {
            return Err(format!("{}: a mask value that is not 0 or 1", s.file));
        }
        kept += u64::from(s.kept);
        for (k, v) in &s.skips {
            *skips.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &s.sources {
            *sources.entry(k.clone()).or_default() += u64::from(*v);
        }
    }
    if kept != m.kept || skips != m.skips || sources != m.sources {
        return Err("the manifest's totals are not its shards' sums".to_owned());
    }
    Ok(m)
}

/// `shards`: `verify DIR`, `info DIR`, or `features` (the bridge from degraded clips).
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("verify") => {
            let dir = args.get(1).ok_or("usage: shards verify RUN_DIR")?;
            let m = verify(&PathBuf::from(dir))?;
            println!(
                "ok: {} shards, {} samples kept of {} candidates ({} stream {}, run seed {}), every hash and header checked",
                m.shards.len(),
                m.kept,
                m.count,
                m.role,
                m.stream,
                m.run_seed
            );
            Ok(())
        }
        Some("info") => {
            let dir = args.get(1).ok_or("usage: shards info RUN_DIR")?;
            let m = read_manifest(&PathBuf::from(dir))?;
            let mut v = serde_json::to_value(&m).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                for k in ["jnd", "layout", "shards", "sampler"] {
                    o.remove(k);
                }
            }
            println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
            Ok(())
        }
        Some("features") => crate::ds1::clips::features(&args[1..]),
        _ => Err("usage: shards verify RUN_DIR | info RUN_DIR | features --samples FILE --clips DIR --out DIR".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Rows {
        let mut r = Rows::default();
        for i in 0..n {
            r.mel
                .extend((0..MEL_LEN).map(|j| (i * 7 + j) as f32 * 0.001 - 3.0));
            r.context.extend([i as f32, -0.25, 1.0, 1.5, -0.5]);
            r.label.extend((0..ROW_LEN).map(|j| (j % 3) as f32 * 0.5));
            r.mask.extend((0..ROW_LEN).map(|j| f32::from(j % 2 == 0)));
            r.index.push(100 + i as u64);
            r.seed.push(u64::MAX - i as u64);
            r.gen_seed.push(if i % 2 == 0 { 0 } else { 1 << 39 });
            r.rate.push([48_000, 44_100, 22_050, 16_000][i % 4]);
            r.source.push((i % 3) as u8);
        }
        r
    }

    fn meta() -> Meta {
        Meta {
            ds1: 1,
            stream: "r3/train".to_owned(),
            run_seed: 7,
            shard: 3,
            first_index: 96,
            candidates: 32,
        }
    }

    #[test]
    fn a_shard_round_trips_and_is_aligned() {
        for n in [0, 1, 5] {
            let r = rows(n);
            let bytes = encode(&meta(), &r);
            assert_eq!(bytes, encode(&meta(), &r), "deterministic");
            let (m, back) = decode(&bytes).unwrap();
            assert_eq!(m, meta());
            assert_eq!(back, r);
            for a in infos(n) {
                assert_eq!(a.offset % ALIGN as u64, 0, "{}", a.name);
            }
        }
    }

    #[test]
    fn the_layout_matches_what_the_docs_say() {
        let bytes = encode(&meta(), &rows(2));
        assert_eq!(&bytes[0..8], b"ENTHSHRD");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        let h = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let v: Value = serde_json::from_slice(&bytes[16..16 + h]).unwrap();
        assert_eq!(v["schema"], "enth.ds1-shard/1");
        assert_eq!(v["n"], 2);
        let mel = &v["arrays"][0];
        assert_eq!(mel["name"], "mel");
        assert_eq!(mel["dtype"], "<f4");
        assert_eq!(mel["shape"], serde_json::json!([2, 3, 128, 64]));
        assert_eq!(mel["offset"], 1024);
        // A reader with no help from this crate: mel's first value, little-endian f32.
        let first = f32::from_le_bytes(bytes[1024..1028].try_into().unwrap());
        assert_eq!(first, -3.0);
        // The padding between the header and the data is spaces, so the block is valid UTF-8 to byte 1024.
        assert!(std::str::from_utf8(&bytes[16..1024]).is_ok());
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let bytes = encode(&meta(), &rows(2));
        assert!(decode(&bytes[..500]).is_err());
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(decode(&bad).is_err());
        let mut newer = bytes.clone();
        newer[8] = 2;
        assert!(decode(&newer).unwrap_err().contains("format 2"));
    }
}
