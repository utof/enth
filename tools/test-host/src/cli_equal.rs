//! `cli-equal`: render the engine's golden cases through a plugin and compare with the engine's hashes.
//!
//! The cheapest strong test of "the plugin renders exactly like the CLI" (P1b report 04): `tests/golden/cases/*.toml`
//! already say what to play (a patch, notes, automation, a sample rate and a length) and `tests/golden/expected/*.toml`
//! say what the engine gives (one SHA-256). This command plays each case through the plugin and asserts that
//! hash, so a DSP change never churns plugin-level hashes: there are none.
//!
//! What it does for one case:
//!
//! 1. The `[source.patch]` table becomes JSON and goes into the plugin's persisted `patch` field through the
//!    state extension ([`crate::state`]), before activation, as a host loading a project would.
//! 2. The notes become CLAP note events and the automation host parameter events, at `round(t * sample_rate)`
//!    frames, in the engine's own order for events on the same frame (automation first, then each note's on and
//!    off in performance order; `enth_render::render`), with the note's index as its note id. Velocities go
//!    over as the exact `f64` of the case's `f32`.
//! 3. It renders `round(seconds * sample_rate)` frames at each host block size (default 1, 64, 100 and 4096: a
//!    block of 1 and one that does not divide the length are where block-quantised event handling shows), dropping
//!    the part of the last block past the end.
//! 4. It hashes the way the engine does (`hash_samples` in `tests/golden/src/summary.rs`): SHA-256 over the sample
//!    rate (u32), the channel count (u16) and the sample count (u64), all little-endian, then every sample's `f32`
//!    bits, channels interleaved.
//!
//! A case is **equal** when every block size gives the same hash and it is the engine's. It is **block-dependent**
//! when two block sizes disagree (a bug in the plugin whatever the engine says), and **differs** when the
//! block sizes agree on a hash that is not the engine's.
//!
//! What cannot go through a plugin is skipped with its reason, never silently: a source that is not a patch (the
//! harness's `sine`), a render seed other than the default (the plugin has no seed input), and automation of an
//! engine address (the plugin's host parameters are macros and slots; a profile gives the automation for a case
//! as host parameter events, `[automation]` in the profile file). At least `--min-compared` cases must be
//! compared, so a run in which everything was skipped cannot pass.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::path::{Path, PathBuf};

use clack_host::prelude::PluginEntry;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::notes::{self, Kind, Schedule, Scheduled, seconds_to_frame};
use crate::query;
use crate::render::{ProcessThread, RenderConfig, Session, hex, run_job_ext, stats};
use crate::state;
use crate::toml_lite;

/// The render seed the plugin uses (`enth_render::DEFAULT_SEED`, `0x0E17_5EED`). A case with another seed
/// cannot be reproduced through a plugin.
pub const DEFAULT_SEED: u64 = 0x0E17_5EED;

/// The host block sizes cases are rendered at unless told otherwise.
pub const DEFAULT_BLOCKS: [u32; 4] = [1, 64, 100, 4096];

#[derive(Clone, Debug, PartialEq)]
pub struct CaseNote {
    pub key: u8,
    pub velocity: f32,
    pub on_s: f64,
    pub off_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseAutomation {
    pub time_s: f64,
    pub addr: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Patch {
        seed: Option<u64>,
        notes: Vec<CaseNote>,
        automation: Vec<CaseAutomation>,
        /// The `[source.patch]` table as JSON.
        patch: Value,
    },
    /// Anything the plugin cannot play (the harness's `sine`), by kind.
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub name: String,
    pub description: String,
    pub sample_rate: u32,
    pub seconds: f64,
    pub source: Source,
}

impl Case {
    /// Frames in the render: `round(seconds * sample_rate)` in `f64`, as the engine and the harness compute it.
    pub fn frames(&self) -> usize {
        (self.seconds * f64::from(self.sample_rate)).round() as usize
    }
}

fn number(v: &Value, what: &str) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("{what} is not a number"))
}

/// Reads a case file (`tests/golden/cases/<name>.toml`).
pub fn parse_case(name: &str, text: &str) -> Result<Case, String> {
    let doc = toml_lite::parse(text)?;
    let sample_rate = doc["sample_rate"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("`sample_rate` is missing or not a whole number")?;
    let seconds = number(&doc["seconds"], "`seconds`")?;
    let source = &doc["source"];
    let kind = source["kind"].as_str().ok_or("`source.kind` is missing")?;
    let source = if kind == "patch" {
        let notes = source["notes"]
            .as_array()
            .ok_or("`source.notes` is missing")?
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let key = n["key"]
                    .as_u64()
                    .and_then(|k| u8::try_from(k).ok())
                    .ok_or_else(|| format!("note {i}: `key` is not 0..=255"))?;
                Ok(CaseNote {
                    key,
                    velocity: if n["velocity"].is_null() {
                        0.8
                    } else {
                        number(&n["velocity"], &format!("note {i}: `velocity`"))? as f32
                    },
                    on_s: number(&n["on_s"], &format!("note {i}: `on_s`"))?,
                    off_s: number(&n["off_s"], &format!("note {i}: `off_s`"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let automation = source["automation"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .enumerate()
            .map(|(i, a)| {
                Ok(CaseAutomation {
                    time_s: number(&a["time_s"], &format!("automation {i}: `time_s`"))?,
                    addr: a["addr"]
                        .as_str()
                        .ok_or_else(|| format!("automation {i}: `addr` is missing"))?
                        .to_owned(),
                    value: number(&a["value"], &format!("automation {i}: `value`"))? as f32,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Source::Patch {
            seed: source["seed"].as_u64(),
            notes,
            automation,
            patch: source["patch"].clone(),
        }
    } else {
        Source::Other(kind.to_owned())
    };
    Ok(Case {
        name: name.to_owned(),
        description: doc["description"].as_str().unwrap_or("").to_owned(),
        sample_rate,
        seconds,
        source,
    })
}

/// An expected file (`tests/golden/expected/<name>.toml`): the part this command needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub sample_rate: u32,
    pub frames: usize,
    pub hash: String,
    /// Platforms known to differ, by `{arch}-{os}`: their own hash, or `summary-only`.
    pub platform_hash: BTreeMap<String, String>,
}

pub fn parse_expected(text: &str) -> Result<Expected, String> {
    let doc = toml_lite::parse(text)?;
    Ok(Expected {
        sample_rate: doc["sample_rate"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("`sample_rate` is missing")?,
        frames: doc["frames"].as_u64().ok_or("`frames` is missing")? as usize,
        hash: doc["hash"].as_str().ok_or("`hash` is missing")?.to_owned(),
        platform_hash: doc["platform_hash"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// `x86_64-linux`, as `enth_golden::platform_key` names it.
pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// The engine's golden hash (`tests/golden/src/summary.rs`, `hash_samples`): SHA-256 over the sample rate, the
/// channel count and the number of samples (little-endian `u32`, `u16`, `u64`), then each sample's `f32` bits.
pub fn engine_hash(sample_rate: u32, channels: u16, samples: &[f32]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sample_rate.to_le_bytes());
    hasher.update(channels.to_le_bytes());
    hasher.update((samples.len() as u64).to_le_bytes());
    for s in samples {
        hasher.update(s.to_bits().to_le_bytes());
    }
    hex(&hasher.finalize())
}

/// What `--profile FILE` can say about a plugin (a TOML file; every key is optional).
///
/// ```toml
/// state_field = "patch"                 # the persisted field the patch goes in
/// blocks = [1, 64, 100, 4096]           # host block sizes to render at
/// cases = ["osc-saw-440", "adsr-pad"]   # only these cases
/// [automation]                          # host parameter events standing in for a case's automation
/// engine-chord = "Slot 1=0.3@0.4003, Slot 2=0.9@0.5"
/// ```
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub state_field: Option<String>,
    pub blocks: Option<Vec<u32>>,
    pub cases: Option<Vec<String>>,
    pub automation: BTreeMap<String, String>,
}

pub fn parse_profile(text: &str) -> Result<Profile, String> {
    let doc = toml_lite::parse(text)?;
    let blocks = match doc.get("blocks") {
        None => None,
        Some(v) => Some(
            v.as_array()
                .ok_or("`blocks` is not an array")?
                .iter()
                .map(|b| {
                    b.as_u64()
                        .and_then(|b| u32::try_from(b).ok())
                        .filter(|b| *b > 0)
                        .ok_or_else(|| "`blocks` holds a value that is not a block size".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let cases = match doc.get("cases") {
        None => None,
        Some(v) => Some(
            v.as_array()
                .ok_or("`cases` is not an array")?
                .iter()
                .map(|c| {
                    c.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "`cases` holds a value that is not a name".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let automation = doc["automation"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|s| (k.clone(), s.to_owned()))
                        .ok_or_else(|| format!("[automation] {k}: not a string"))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Profile {
        state_field: doc["state_field"].as_str().map(str::to_owned),
        blocks,
        cases,
        automation,
    })
}

/// The events of a case in the order the engine applies them: automation first, then each note's on and off,
/// stably sorted by frame. Events at or after `frames` are dropped.
///
/// `automation` is already resolved to host parameter events (`frame`, parameter id, value).
pub fn schedule(
    notes: &[CaseNote],
    automation: &[(u64, u32, f64)],
    sample_rate: u32,
    frames: u64,
) -> Schedule {
    let sr = f64::from(sample_rate);
    let mut events: Vec<Scheduled> = Vec::with_capacity(automation.len() + 2 * notes.len());
    for &(frame, id, value) in automation {
        events.push(Scheduled {
            frame,
            kind: Kind::Param { id, value },
        });
    }
    for (i, n) in notes.iter().enumerate() {
        let note_id = u32::try_from(i).unwrap_or(u32::MAX);
        let key = u16::from(n.key);
        events.push(Scheduled {
            frame: seconds_to_frame(n.on_s, sr).unwrap_or(0),
            kind: Kind::NoteOn {
                key,
                note_id,
                velocity: f64::from(n.velocity),
            },
        });
        events.push(Scheduled {
            frame: seconds_to_frame(n.off_s, sr).unwrap_or(0),
            kind: Kind::NoteOff { key, note_id },
        });
    }
    events.retain(|e| e.frame < frames);
    // Stable, and by frame only: the engine's order for events on one frame is the push order above.
    events.sort_by_key(|e| e.frame);
    Schedule { events }
}

/// A fault the host makes on purpose, so a run can show that the check is able to fail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inject {
    #[default]
    None,
    /// Flip the lowest bit of the first sample before hashing.
    FlipBit,
}

impl Inject {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "none" => Ok(Self::None),
            "flip-bit" => Ok(Self::FlipBit),
            other => Err(format!(
                "--inject {other:?}: cli-equal knows `flip-bit` (the hash of the first sample's lowest bit is flipped)"
            )),
        }
    }
}

/// One render of one case at one block size.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub block: u32,
    pub sha256: String,
    pub peak: f32,
    pub non_finite: usize,
}

/// Renders `case` through a new instance at `block` and hashes it the engine's way.
pub fn render_case(
    entry: &PluginEntry,
    id: &CStr,
    case: &Case,
    events: &Schedule,
    state_bytes: &[u8],
    block: u32,
    inject: Inject,
) -> Result<Rendered, String> {
    let frames = case.frames();
    let blocks = frames.div_ceil(block as usize);
    let config = RenderConfig {
        sample_rate: f64::from(case.sample_rate),
        block,
        frames: (blocks * block as usize) as u64,
    };
    let mut session = Session::open_bare(entry, id, Some(state_bytes), config)?;
    session.schedule = events.clone();
    let processor = session.activate()?;
    let out = run_job_ext(session.setup(processor), ProcessThread::Dedicated)?;
    session.close(out.processor);
    let channels = out.channels;
    let mut samples = out.samples;
    samples.truncate(frames * channels);
    if inject == Inject::FlipBit
        && let Some(first) = samples.first_mut()
    {
        *first = f32::from_bits(first.to_bits() ^ 1);
    }
    let s = stats(&samples, channels);
    Ok(Rendered {
        block,
        sha256: engine_hash(case.sample_rate, channels as u16, &samples),
        peak: s.peak,
        non_finite: s.non_finite,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every block size gives the engine's hash (or, with no expected file, one hash).
    Equal,
    /// The block sizes agree, on a hash that is not the engine's.
    Differs,
    /// Two block sizes disagree.
    BlockDependent,
    Skipped,
    /// The case could not be run (the plugin refused the state, `process()` failed, a bad file).
    Error,
}

impl Status {
    pub fn name(&self) -> &'static str {
        match self {
            Status::Equal => "equal",
            Status::Differs => "differs",
            Status::BlockDependent => "block-dependent",
            Status::Skipped => "skipped",
            Status::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CaseReport {
    pub name: String,
    pub status: Status,
    pub reason: Option<String>,
    pub renders: Vec<Rendered>,
    pub expected: Option<String>,
    pub frames: usize,
}

impl CaseReport {
    fn skipped(name: &str, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            status: Status::Skipped,
            reason: Some(reason.into()),
            renders: Vec::new(),
            expected: None,
            frames: 0,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "status": self.status.name(),
            "reason": self.reason,
            "frames": self.frames,
            "expected": self.expected,
            "renders": self.renders.iter().map(|r| json!({
                "block": r.block, "sha256": r.sha256, "peak": r.peak, "non_finite_samples": r.non_finite,
            })).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Case files, or directories of them.
    pub cases: Vec<PathBuf>,
    /// Where `<name>.toml` expected files live; `None` compares block sizes with each other only.
    pub expected_dir: Option<PathBuf>,
    pub only: Vec<String>,
    pub blocks: Vec<u32>,
    pub profile: Profile,
    pub state_field: String,
    pub min_compared: usize,
    /// Write `<name>.toml` expected files from this plugin's own render (the first block size).
    pub write_expected: Option<PathBuf>,
    pub inject: Inject,
}

pub struct Report {
    pub cases: Vec<CaseReport>,
    pub min_compared: usize,
}

impl Report {
    pub fn count(&self, status: &Status) -> usize {
        self.cases.iter().filter(|c| &c.status == status).count()
    }

    /// Cases that were rendered and judged.
    pub fn compared(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| c.status != Status::Skipped && c.status != Status::Error)
            .count()
    }

    pub fn ok(&self) -> bool {
        self.count(&Status::Differs) == 0
            && self.count(&Status::BlockDependent) == 0
            && self.count(&Status::Error) == 0
            && self.compared() >= self.min_compared
    }

    pub fn to_json(&self) -> Value {
        json!({
            "equal": self.count(&Status::Equal),
            "differs": self.count(&Status::Differs),
            "block_dependent": self.count(&Status::BlockDependent),
            "skipped": self.count(&Status::Skipped),
            "errors": self.count(&Status::Error),
            "compared": self.compared(),
            "min_compared": self.min_compared,
            "ok": self.ok(),
            "cases": self.cases.iter().map(CaseReport::to_json).collect::<Vec<_>>(),
        })
    }
}

/// The case files in `paths` (a directory contributes its `*.toml`), sorted by name.
pub fn collect_cases(paths: &[PathBuf]) -> Result<Vec<(String, PathBuf)>, String> {
    let mut found = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
                let p = entry.map_err(|e| e.to_string())?.path();
                if p.extension().is_some_and(|e| e == "toml") {
                    found.push(p);
                }
            }
        } else if path.is_file() {
            found.push(path.clone());
        } else {
            return Err(format!(
                "{}: no such case file or directory",
                path.display()
            ));
        }
    }
    let mut named: Vec<(String, PathBuf)> = found
        .into_iter()
        .map(|p| {
            let name = p
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            (name, p)
        })
        .collect();
    named.sort();
    named.dedup_by(|a, b| a.0 == b.0);
    Ok(named)
}

fn write_expected_file(dir: &Path, name: &str, case: &Case, hash: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = format!(
        "# Written by `enth-test-host cli-equal --write-expected`: this plugin's own render, not the engine's.\nsample_rate = {}\nframes = {}\nhash = \"{hash}\"\n",
        case.sample_rate,
        case.frames()
    );
    std::fs::write(dir.join(format!("{name}.toml")), text)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Runs every case against `entry`.
pub fn run(entry: &PluginEntry, id: &CStr, opts: &Options) -> Result<Report, String> {
    let version = crate::host::plugin_version(entry, id);
    let files = collect_cases(&opts.cases)?;
    // Parameter names for the profile's automation, read from a throwaway instance.
    let params = {
        let config = RenderConfig::new(48_000.0, 64, 0.1)?;
        Session::open_bare(entry, id, None, config)?.params
    };
    let resolve = |name: &str| query::resolve_param(&params, name);
    let mut reports = Vec::new();
    for (name, path) in files {
        if !opts.only.is_empty() && !opts.only.contains(&name) {
            continue;
        }
        if let Some(allowed) = &opts.profile.cases
            && !allowed.contains(&name)
        {
            continue;
        }
        reports.push(run_one(entry, id, opts, &version, &resolve, &name, &path));
    }
    if reports.is_empty() {
        return Err("no case matched; check --cases, --only and the profile's `cases`".to_owned());
    }
    Ok(Report {
        cases: reports,
        min_compared: opts.min_compared,
    })
}

fn run_one(
    entry: &PluginEntry,
    id: &CStr,
    opts: &Options,
    version: &str,
    resolve: &dyn Fn(&str) -> Option<u32>,
    name: &str,
    path: &Path,
) -> CaseReport {
    let error = |reason: String| CaseReport {
        name: name.to_owned(),
        status: Status::Error,
        reason: Some(reason),
        renders: Vec::new(),
        expected: None,
        frames: 0,
    };
    let case = match std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|text| parse_case(name, &text))
    {
        Ok(case) => case,
        Err(e) => return error(e),
    };
    let Source::Patch {
        seed,
        notes,
        automation,
        patch,
    } = &case.source
    else {
        let Source::Other(kind) = &case.source else {
            unreachable!("a source is a patch or another kind");
        };
        return CaseReport::skipped(name, format!("source kind {kind:?} is not a patch"));
    };
    if let Some(seed) = seed
        && *seed != DEFAULT_SEED
    {
        return CaseReport::skipped(
            name,
            format!("render seed {seed}: the plugin always renders with seed {DEFAULT_SEED}"),
        );
    }
    let frames = case.frames();

    // Automation: the profile's host parameter events, or the case has none.
    let mut host_automation = Vec::new();
    if let Some(script) = opts.profile.automation.get(name) {
        match notes::parse(script, f64::from(case.sample_rate), frames as u64, resolve) {
            Ok(parsed) => {
                for e in parsed.events {
                    match e.kind {
                        Kind::Param { id, value } => host_automation.push((e.frame, id, value)),
                        _ => {
                            return error(format!(
                                "profile automation for {name}: only `parameter=value@seconds` entries belong there"
                            ));
                        }
                    }
                }
            }
            Err(e) => return error(format!("profile automation for {name}: {e}")),
        }
    } else if !automation.is_empty() {
        return CaseReport::skipped(
            name,
            format!(
                "{} automation point(s) of engine addresses ({}…) need host parameter events: add `[automation] {name} = \"Slot 1=…@seconds\"` to a --profile",
                automation.len(),
                automation[0].addr
            ),
        );
    }

    let events = schedule(notes, &host_automation, case.sample_rate, frames as u64);
    let patch_text = patch.to_string();
    let state_bytes = match state::wrap_patch(&patch_text, &opts.state_field, version) {
        Ok(b) => b,
        Err(e) => return error(e),
    };

    // The expected file, if there is one for this case.
    let mut expected_hash = None;
    if let Some(dir) = &opts.expected_dir {
        let file = dir.join(format!("{name}.toml"));
        let expected = match std::fs::read_to_string(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
            .and_then(|t| parse_expected(&t))
        {
            Ok(e) => e,
            Err(e) => return error(e),
        };
        if expected.sample_rate != case.sample_rate || expected.frames != frames {
            return error(format!(
                "{}: the expected file says {} frames at {} Hz, the case {} at {}",
                file.display(),
                expected.frames,
                expected.sample_rate,
                frames,
                case.sample_rate
            ));
        }
        match expected
            .platform_hash
            .get(&platform_key())
            .map(String::as_str)
        {
            Some("summary-only") => {
                return CaseReport::skipped(
                    name,
                    format!("{} is compared on its summary only", platform_key()),
                );
            }
            Some(own) => expected_hash = Some(own.to_owned()),
            None => expected_hash = Some(expected.hash),
        }
    }

    let mut renders = Vec::new();
    for &block in &opts.blocks {
        match render_case(entry, id, &case, &events, &state_bytes, block, opts.inject) {
            Ok(r) => renders.push(r),
            Err(e) => return error(format!("block {block}: {e}")),
        }
    }
    let first = renders[0].sha256.clone();
    let consistent = renders.iter().all(|r| r.sha256 == first);
    let (status, reason) = if !consistent {
        (
            Status::BlockDependent,
            Some("the plugin's output depends on the host block size".to_owned()),
        )
    } else if let Some(want) = &expected_hash {
        if *want == first {
            (Status::Equal, None)
        } else {
            (
                Status::Differs,
                Some(format!("engine {want}, plugin {first}")),
            )
        }
    } else {
        (Status::Equal, None)
    };
    if consistent
        && let Some(dir) = &opts.write_expected
        && let Err(e) = write_expected_file(dir, name, &case, &first)
    {
        return error(e);
    }
    CaseReport {
        name: name.to_owned(),
        status,
        reason,
        renders,
        expected: expected_hash,
        frames,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASE: &str = r#"
description = "two notes"
sample_rate = 48000
seconds = 1.0

[source]
kind = "patch"
seed = 7
notes = [
    { key = 60, velocity = 0.5, on_s = 0.0, off_s = 0.5 },
    { key = 64, on_s = 0.25, off_s = 0.25 },
]
automation = [{ time_s = 0.1, addr = "osc1.level", value = 20.0 }]

[source.patch]
schema = 1
voice = { amp_env = "env1" }
"#;

    #[test]
    fn a_case_file_is_read() {
        let case = parse_case("c", CASE).unwrap();
        assert_eq!(case.frames(), 48_000);
        let Source::Patch {
            seed,
            notes,
            automation,
            patch,
        } = &case.source
        else {
            panic!("a patch source");
        };
        assert_eq!(*seed, Some(7));
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].velocity, 0.8, "the default velocity");
        assert_eq!(automation[0].addr, "osc1.level");
        assert_eq!(patch["voice"]["amp_env"], "env1");
        let sine = parse_case(
            "s",
            "sample_rate = 48000\nseconds = 1\n[source]\nkind = \"sine\"\nfrequency_hz = 440.0\namplitude = 0.5\n",
        )
        .unwrap();
        assert_eq!(sine.source, Source::Other("sine".to_owned()));
    }

    #[test]
    fn events_follow_the_engine_order_and_drop_what_is_past_the_end() {
        let notes = [
            CaseNote {
                key: 60,
                velocity: 0.5,
                on_s: 0.0,
                off_s: 0.5,
            },
            // Zero length: on, then off, on the same frame.
            CaseNote {
                key: 64,
                velocity: 1.0,
                on_s: 0.25,
                off_s: 0.25,
            },
            // The off falls after the end of a 0.6 s render.
            CaseNote {
                key: 67,
                velocity: 0.25,
                on_s: 0.5,
                off_s: 2.0,
            },
        ];
        let auto = [(12_000u64, 7u32, 0.5f64)];
        let s = schedule(&notes, &auto, 48_000, 28_800);
        let frames: Vec<u64> = s.events.iter().map(|e| e.frame).collect();
        assert_eq!(
            frames,
            [0, 12_000, 12_000, 12_000, 24_000, 24_000],
            "{:?}",
            s.events
        );
        // On the frame 12,000 the automation comes first, then note 1's on, then its off.
        assert!(matches!(s.events[1].kind, Kind::Param { id: 7, .. }));
        assert!(matches!(s.events[2].kind, Kind::NoteOn { note_id: 1, .. }));
        assert!(matches!(s.events[3].kind, Kind::NoteOff { note_id: 1, .. }));
        // At 24,000 note 0's off precedes note 2's on: performance order, not "offs first".
        assert!(matches!(s.events[4].kind, Kind::NoteOff { note_id: 0, .. }));
        assert!(matches!(s.events[5].kind, Kind::NoteOn { note_id: 2, .. }));
        // Velocities are the exact f64 of the f32.
        assert!(matches!(s.events[0].kind, Kind::NoteOn { velocity, .. } if velocity == 0.5));
        let tenth = schedule(
            &[CaseNote {
                key: 60,
                velocity: 0.9,
                on_s: 0.0,
                off_s: 1.0,
            }],
            &[],
            48_000,
            48_000,
        );
        assert!(
            matches!(tenth.events[0].kind, Kind::NoteOn { velocity, .. } if velocity as f32 == 0.9f32)
        );
    }

    #[test]
    fn the_hash_has_the_engine_s_framing() {
        // Independent of this crate: sha256 of 48000u32le ++ 2u16le ++ 4u64le ++ [1.0, -2.0, 0.5, 0.0] as f32 le bits,
        // computed with Python's hashlib.
        let samples = [1.0f32, -2.0, 0.5, 0.0];
        assert_eq!(
            engine_hash(48_000, 2, &samples),
            "3e12e01b72db298465928ea263069ff93427b1ccfe57f95057d24245c7e7aa94"
        );
    }

    #[test]
    fn an_expected_file_and_a_profile_are_read() {
        let e = parse_expected(
            "sample_rate = 48000\nframes = 96000\nhash = \"abc\"\n[platform_hash]\nx86_64-linux = \"summary-only\"\n[tolerance]\nrms_db = 0.01\n[summary]\nrms_db = [-1.0,\n -2.0]\n",
        )
        .unwrap();
        assert_eq!(
            (e.sample_rate, e.frames, e.hash.as_str()),
            (48_000, 96_000, "abc")
        );
        assert_eq!(e.platform_hash["x86_64-linux"], "summary-only");
        let p = parse_profile(
            "state_field = \"patch\"\nblocks = [64, 100]\ncases = [\"a\"]\n[automation]\na = \"Slot 1=0.5@0.1\"\n",
        )
        .unwrap();
        assert_eq!(p.blocks, Some(vec![64, 100]));
        assert_eq!(p.automation["a"], "Slot 1=0.5@0.1");
        assert!(parse_profile("blocks = [0]").is_err());
    }
}
