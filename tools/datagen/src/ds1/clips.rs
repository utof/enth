//! **Owner: lane DG.** The bridge between `ds1`'s clean renders and the degradation chain (docs/r3.md, "Who computes
//! what", steps 1 to 3).
//!
//! ```text
//! enth-datagen ds1 render --count N --seed S --out DIR [--start I]     # 1. clean clips: DIR/clips/NNNNNNNNNN.wav, DIR/samples.jsonl
//!   ... DEG's chain and AUG (Python, ffmpeg) write a degraded copy of each clip as a WAV file ...                 # 2.
//! enth-datagen shards features --samples F --clips DIR --out DIR         # 3. the product's intake, features and labels -> shards
//! ```
//!
//! **`samples.jsonl`** has one JSON object per line: `{"stream", "run_seed", "index", "clip"}`, and for a degraded
//! copy an optional `"degrade"` record (DEG's: pool, profile, seed, stages, the pools' hash, the sources used) that
//! the exclusion lists screen. `clip` is a path relative to the directory `--clips` names. `ds1 render` writes the
//! lines for its clean clips; the chain keeps `stream`, `run_seed` and `index` and rewrites `clip` (and adds
//! `degrade`). Every line of one file has the same `stream` and `run_seed`.
//!
//! **What `features` does** for each line: re-draws the sample from `(ds1 version, stream, run seed, index)`
//! (the patch and phrase never travel), screens it against the lists, reads the clip, runs the loudness filter, the
//! **product's intake on the clip as it is** (so a late onset under noise, a missed codec cut or an octave misreading
//! reaches training as it will at inference), `ModelInput::from_intake`, and
//! `OutputLayout::encode(patch, &Hearing::sample(&Cue::from_intake(&intake), notes, length_ms))`, so the label is
//! relative to the key the intake read from **this** clip. A clip the pipeline refuses is skipped and counted by code
//! (`features_silent`, `off_layout`, `rekey`, `quiet`, `intake_*`, ...). A record or a path on a list, or a clip
//! whose bytes are on one, is an error that stops the run: it means the chain used a held-out pool.
//!
//! The same sample, undegraded, gives the same row as `ds1 gen` (a test pins it): the clean render is written as a
//! 32-bit float stereo WAV and the intake's mono mix is the mid channel.
//!
//! **WAV files.** Written: IEEE float, 32 bit, 2 channels, at the sample's rate. Read: PCM 16, 24 or 32 bit and
//! float 32 or 64 bit, any channel count, plain or extensible headers, and a data chunk whose size field is wrong
//! (ffmpeg's pipes write `0xFFFFFFFF`). Standard library only: no new dependency.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a command-line tool: reports go to stdout; the bridge reads and writes files"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use serde_json::{Value, json};

use super::pipeline::{Audio, Sample, Skip};
use super::{Flags, Need, RunInfo, ShardBuilder, pipeline_for, pool, role_of, write_manifest};
use crate::exclude::default_gr3;
use crate::shards::{self, Meta, ShardStats};

// ---------------------------------------------------------------------------------------------------------------
// WAV
// ---------------------------------------------------------------------------------------------------------------

/// The bytes of an IEEE-float 32-bit WAV file of `interleaved` frames of `channels`.
///
/// # Panics
/// If `channels` is 0 or the file would not fit a 32-bit size field.
pub fn wav_f32(rate: u32, channels: u16, interleaved: &[f32]) -> Vec<u8> {
    assert!(channels > 0);
    let data = u32::try_from(interleaved.len() * 4).expect("the clip fits a WAV file");
    let mut out = Vec::with_capacity(44 + interleaved.len() * 4);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 4).to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in interleaved {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A decoded WAV file.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    /// Frames per second.
    pub rate: u32,
    /// Channels per frame.
    pub channels: u16,
    /// Interleaved samples in -1..1 (more for float files that exceed it).
    pub samples: Vec<f32>,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Decode a WAV file (see the module docs for what is read).
///
/// # Errors
/// What is wrong: not RIFF/WAVE, no `fmt ` or `data` chunk, a format that is not read.
pub fn read_wav(b: &[u8]) -> Result<Wav, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_owned());
    }
    let mut at = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = le32(b, at + 4).ok_or("truncated chunk")? as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(b.len());
        match id {
            b"fmt " => {
                let tag = le16(b, body).ok_or("truncated fmt chunk")?;
                let channels = le16(b, body + 2).ok_or("truncated fmt chunk")?;
                let rate = le32(b, body + 4).ok_or("truncated fmt chunk")?;
                let bits = le16(b, body + 14).ok_or("truncated fmt chunk")?;
                // WAVE_FORMAT_EXTENSIBLE: the real tag is the first two bytes of the sub-format GUID.
                let tag = if tag == 0xFFFE {
                    le16(b, body + 24).ok_or("truncated extensible fmt chunk")?
                } else {
                    tag
                };
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => {
                data = Some(&b[body..end]);
                break;
            }
            _ => {}
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 {
        return Err("zero channels".to_owned());
    }
    let samples: Vec<f32> = match (tag, bits) {
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| f32::from(i16::from_le_bytes(*c)) / 32_768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| (f64::from(i32::from_le_bytes(*c)) / 2_147_483_648.0) as f32)
            .collect(),
        (3, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
        (3, 64) => data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c) as f32)
            .collect(),
        _ => {
            return Err(format!(
                "WAV format {tag} at {bits} bits is not read (PCM 16/24/32, float 32/64)"
            ));
        }
    };
    let whole = samples.len() / usize::from(channels) * usize::from(channels);
    let mut samples = samples;
    samples.truncate(whole);
    Ok(Wav {
        rate,
        channels,
        samples,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------------------------------------------

/// `ds1 render`: clean clips for DEG's chain (the module docs, step 1).
pub fn render_cmd(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(
        args,
        &[
            "--count",
            "--seed",
            "--out",
            "--start",
            "--stream",
            "--workers",
            "--gr3",
        ],
        &[],
    )?;
    let count: u64 = f.need("--count")?;
    let seed: u64 = f.need("--seed")?;
    let start: u64 = f.num("--start")?.unwrap_or(0);
    let workers: usize = f.num("--workers")?.unwrap_or(1);
    let out = PathBuf::from(f.str("--out").ok_or("--out is required")?);
    let stream = f.stream();
    let pool = pool(workers)?;
    let p = pipeline_for(&pool, &stream, seed, &f.gr3(), Need::Generators)?;
    let dir = out.join("clips");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let t = Instant::now();
    let done: Vec<Result<Value, Skip>> = pool.install(|| {
        (start..start + count)
            .into_par_iter()
            .map(|i| {
                let (d, r) = p.render_candidate(i)?;
                let name = format!("clips/{i:010}.wav");
                let bytes = wav_f32(d.rate, 2, &r.out.interleaved());
                fs::write(out.join(&name), bytes).map_err(|e| Skip {
                    code: "write",
                    detail: e.to_string(),
                })?;
                Ok(json!({
                    "clip": name,
                    "index": i,
                    "rate": d.rate,
                    "run_seed": seed,
                    "source": d.source.name(),
                    "stream": stream,
                }))
            })
            .collect()
    });
    let mut lines = String::new();
    let mut skips: BTreeMap<String, u64> = BTreeMap::new();
    let mut kept = 0u64;
    for r in done {
        match r {
            Ok(v) => {
                lines.push_str(&v.to_string());
                lines.push('\n');
                kept += 1;
            }
            Err(s) => *skips.entry(s.code.to_owned()).or_default() += 1,
        }
    }
    shards::write_atomic(&out.join("samples.jsonl"), lines.as_bytes())?;
    println!(
        "wrote {kept} clean clips of {count} candidates to {} in {:.1} s on {workers} workers; skips {}",
        dir.display(),
        t.elapsed().as_secs_f64(),
        serde_json::to_string(&skips).map_err(|e| e.to_string())?
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// features
// ---------------------------------------------------------------------------------------------------------------

struct Line {
    index: u64,
    clip: PathBuf,
}

fn relative_clip(v: &Value) -> Result<PathBuf, String> {
    let rel = Path::new(v.as_str().ok_or("'clip' must be a string")?);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!(
            "'clip' {} must be a path below --clips",
            rel.display()
        ));
    }
    Ok(rel.to_path_buf())
}

/// `shards features`: degraded clips to shards (the module docs, step 3).
pub fn features(args: &[String]) -> Result<(), String> {
    let f = Flags::parse(
        args,
        &[
            "--samples",
            "--clips",
            "--out",
            "--shard-size",
            "--workers",
            "--gr3",
        ],
        &[],
    )?;
    let samples = PathBuf::from(f.str("--samples").ok_or("--samples is required")?);
    let clips = PathBuf::from(f.str("--clips").ok_or("--clips is required")?);
    let out = PathBuf::from(f.str("--out").ok_or("--out is required")?);
    let size: u32 = f.num("--shard-size")?.unwrap_or(64);
    let workers: usize = f.num("--workers")?.unwrap_or(1);
    if size == 0 {
        return Err("--shard-size must be at least 1".to_owned());
    }
    let text = fs::read(&samples).map_err(|e| format!("cannot read {}: {e}", samples.display()))?;
    let samples_sha = shards::sha256_hex(&text);
    let text = String::from_utf8(text).map_err(|_| "the samples file is not UTF-8".to_owned())?;

    // Parse every line before any work: one stream, one run seed, and no held-out record or path.
    let mut stream: Option<String> = None;
    let mut run_seed: Option<u64> = None;
    let mut parsed: Vec<(Line, Value)> = Vec::new();
    for (n, raw) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| format!("{}:{}: {e}", samples.display(), n + 1))?;
        let at = |e: String| format!("{}:{}: {e}", samples.display(), n + 1);
        let s = v["stream"]
            .as_str()
            .ok_or_else(|| at("no 'stream'".to_owned()))?
            .to_owned();
        let seed = v["run_seed"]
            .as_u64()
            .ok_or_else(|| at("no 'run_seed'".to_owned()))?;
        if *stream.get_or_insert_with(|| s.clone()) != s || *run_seed.get_or_insert(seed) != seed {
            return Err(at(
                "a samples file holds one stream and one run seed".to_owned()
            ));
        }
        let index = v["index"]
            .as_u64()
            .ok_or_else(|| at("no 'index'".to_owned()))?;
        let clip = relative_clip(&v["clip"]).map_err(at)?;
        parsed.push((Line { index, clip }, v["degrade"].clone()));
    }
    let (Some(stream), Some(run_seed)) = (stream, run_seed) else {
        return Err(format!("{} holds no samples", samples.display()));
    };

    let pool = pool(workers)?;
    let p = pipeline_for(&pool, &stream, run_seed, &f.gr3(), Need::Labels)?;
    let repo = fs::canonicalize(default_gr3().join("../..")).ok();
    let mut warnings: Vec<String> = Vec::new();
    for (n, (line, record)) in parsed.iter().enumerate() {
        if !record.is_null() {
            match p.ex.degrade_record(record) {
                Ok(w) => warnings.extend(w),
                Err(hit) => {
                    return Err(format!(
                        "{}: line {}: the degradation record {hit}",
                        samples.display(),
                        n + 1
                    ));
                }
            }
        }
        let abs = clips.join(&line.clip);
        if let (Some(repo), Ok(abs)) = (&repo, fs::canonicalize(&abs))
            && let Ok(rel) = abs.strip_prefix(repo)
            && let Some(hit) = p.ex.path(&rel.to_string_lossy())
        {
            return Err(format!("{}: line {}: {hit}", samples.display(), n + 1));
        }
    }
    warnings.sort();
    warnings.dedup();

    let total = parsed.len() as u64;
    let n_shards = total.div_ceil(u64::from(size)) as u32;
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let run =
        shards::sha256_hex(format!("clips|{}|{samples_sha}|{size}", super::DS1_VERSION).as_bytes());
    let t = Instant::now();
    let done: Vec<Result<ShardStats, String>> = pool.install(|| {
        (0..n_shards)
            .into_par_iter()
            .map(|k| {
                let first = u64::from(k) * u64::from(size);
                let end = (first + u64::from(size)).min(total);
                let mut b = ShardBuilder::default();
                for (line, _) in &parsed[first as usize..end as usize] {
                    match clip_sample(&p, &clips, line)? {
                        Ok(s) => b.push(&s),
                        Err(skip) => b.skip(skip.code),
                    }
                }
                let meta = Meta {
                    ds1: super::DS1_VERSION,
                    stream: stream.clone(),
                    run_seed,
                    shard: k,
                    first_index: first,
                    candidates: (end - first) as u32,
                };
                b.finish(&out, &run, &meta)
            })
            .collect()
    });
    let wall = t.elapsed().as_secs_f64();
    let stats = done.into_iter().collect::<Result<Vec<_>, _>>()?;
    let bridge = json!({
        "samples_sha256": samples_sha,
        "lines": total,
        "note": "shard i holds lines [i * shard_size, ...) of the samples file; first_index is the line, row 'index' the sample's index in its stream",
    });
    let (m, sha) = write_manifest(
        &p,
        &out,
        RunInfo {
            kind: "clips",
            count: total,
            shard_size: size,
            warnings,
            bridge,
        },
        stats,
    )?;
    println!(
        "wrote {} shards to {}: kept {} of {} clips ({} stream) in {wall:.1} s",
        m.shards.len(),
        out.display(),
        m.kept,
        m.count,
        role_of(&stream)
    );
    println!(
        "skips by code: {}",
        serde_json::to_string(&m.skips).map_err(|e| e.to_string())?
    );
    println!("manifest sha256: {sha}");
    for w in &m.warnings {
        println!("warning: {w}");
    }
    Ok(())
}

/// One line of the samples file as a sample: the draw, the clip, the label. `Err(String)` stops the run (a held-out
/// clip); `Ok(Err(skip))` is counted.
fn clip_sample(
    p: &super::Pipeline,
    clips: &Path,
    line: &Line,
) -> Result<Result<Sample, Skip>, String> {
    let skip = |code: &'static str, detail: String| Ok(Err(Skip { code, detail }));
    let d = match p.sampler.draw(&p.stream, p.run_seed, line.index) {
        Ok(d) => d,
        Err(e) => return skip(e.code(), e.to_string()),
    };
    if let Err(s) = p.screen(&d) {
        return Ok(Err(s));
    }
    let path = clips.join(&line.clip);
    let Ok(bytes) = fs::read(&path) else {
        return skip("clip_unreadable", path.display().to_string());
    };
    if let Some(hit) = p.ex.audio(&shards::sha256_hex(&bytes)) {
        return Err(format!("{}: {hit}", path.display()));
    }
    let wav = match read_wav(&bytes) {
        Ok(w) => w,
        Err(e) => return skip("clip_unreadable", format!("{}: {e}", path.display())),
    };
    let audio = Audio {
        interleaved: &wav.samples,
        channels: wav.channels,
        rate: wav.rate,
    };
    Ok(p.label_file(&d, &audio).map(|labelled| Sample {
        index: d.index,
        seed: d.seed,
        gen_seed: d.generator.as_ref().map_or(0, |g| g.spec.seed),
        rate: wav.rate,
        source: d.source,
        labelled,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_float_wav_round_trips() {
        let x: Vec<f32> = (0..20).map(|i| i as f32 * 0.05 - 0.4).collect();
        let w = read_wav(&wav_f32(22_050, 2, &x)).unwrap();
        assert_eq!((w.rate, w.channels), (22_050, 2));
        assert_eq!(w.samples, x);
    }

    fn header(
        tag: u16,
        channels: u16,
        rate: u32,
        bits: u16,
        data: &[u8],
        size_field: u32,
    ) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * u32::from(bits) / 8).to_le_bytes());
        b.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"LIST\x04\0\0\0abcd");
        b.extend_from_slice(b"data");
        b.extend_from_slice(&size_field.to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn pcm_files_and_ffmpeg_pipe_headers_are_read() {
        let pcm16: Vec<u8> = [0i16, 16_384, -32_768, 32_767]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let w = read_wav(&header(1, 1, 8_000, 16, &pcm16, pcm16.len() as u32)).unwrap();
        assert_eq!(w.samples, vec![0.0, 0.5, -1.0, 32_767.0 / 32_768.0]);
        // A data size of 0xFFFFFFFF (a pipe) reads to the end of the file.
        let w = read_wav(&header(1, 2, 16_000, 16, &pcm16, u32::MAX)).unwrap();
        assert_eq!((w.channels, w.samples.len()), (2, 4));
        let pcm24: Vec<u8> = [0x40_0000i32, -0x40_0000]
            .iter()
            .flat_map(|s| s.to_le_bytes()[..3].to_vec())
            .collect();
        let w = read_wav(&header(1, 1, 8_000, 24, &pcm24, pcm24.len() as u32)).unwrap();
        assert_eq!(w.samples, vec![0.5, -0.5]);
        let pcm32: Vec<u8> = [i32::MIN, 0x4000_0000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let w = read_wav(&header(1, 1, 8_000, 32, &pcm32, pcm32.len() as u32)).unwrap();
        assert_eq!(w.samples, vec![-1.0, 0.5]);
        let f64s: Vec<u8> = [0.25f64, -0.75]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let w = read_wav(&header(3, 1, 8_000, 64, &f64s, f64s.len() as u32)).unwrap();
        assert_eq!(w.samples, vec![0.25, -0.75]);
    }

    #[test]
    fn what_is_not_a_wav_is_refused() {
        assert!(read_wav(b"nope").is_err());
        assert!(
            read_wav(&header(2, 1, 8_000, 4, &[0, 0], 2))
                .unwrap_err()
                .contains("not read")
        );
        assert!(read_wav(&wav_f32(8_000, 1, &[0.0])[..30]).is_err());
    }
}
