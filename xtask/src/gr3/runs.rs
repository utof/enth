//! **Owner: lane EVK** (the append), **`EVb`** (the test runs). Pre-declared by lane S0s (R3).
//!
//! The two files `gr3-run` and `gr3-report` read and write:
//!
//! - **`content/gr3/synth.toml`** (lane DS, frozen): the targets, read leniently (only the fields the kit needs), and
//!   the selection rules: the split, the set (`core`, `rest`, `available`, `full`), and **the freeze's refusals**: a
//!   test run never includes an amendable item whose entry is unfilled (a G1 target whose `sha256` is still
//!   `pending`; content/gr3/README.md, "The freeze", the coordinator's ruling of 2026-10-21), and the manifest
//!   must match `hashes.txt`.
//! - **`content/gr3/runs.jsonl`**: every test run, one JSON line each (the build, the date, the load), **appended
//!   and never dropped or rewritten** (docs/r3.md, "Hygiene"). The tool appends; this file only reads it back for the
//!   report and for a twin's budgets. A run records the renders each checkpoint used, so the render-budget twin
//!   reproduces it on any machine, and its wall-clock conditions, so a timed run is judged as what it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// One `[[target]]` of `synth.toml`, as far as the kit reads it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ManifestTarget {
    /// The id.
    pub id: String,
    /// `test` or `dev`.
    pub split: String,
    /// `generator`, `preset` or `owner-g1`.
    pub kind: String,
    /// Generator level.
    #[serde(default)]
    pub level: Option<u8>,
    /// Generator code.
    #[serde(default)]
    pub code: Option<String>,
    /// Generator fingerprint.
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Generator phrase.
    #[serde(default)]
    pub phrase: Option<u16>,
    /// A patch file, relative to the repository.
    #[serde(default)]
    pub file: Option<String>,
    /// The file's SHA-256 (`pending` until the amendment fills it).
    #[serde(default)]
    pub sha256: Option<String>,
    /// `placeholder` until the amendment fills it.
    #[serde(default)]
    pub status: Option<String>,
    /// A preset's performance.
    #[serde(default)]
    pub perf: Option<PerfToml>,
    /// In the core.
    #[serde(default)]
    pub core: Option<bool>,
}

/// A preset's performance in `synth.toml`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
pub struct PerfToml {
    /// MIDI key.
    pub key: u8,
    /// MIDI velocity.
    pub velocity: u8,
    /// Hold (ms).
    pub hold_ms: u32,
    /// Length (ms).
    pub length_ms: u32,
}

/// `synth.toml`, as far as the kit reads it.
#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    /// `enth.gr3-synth/1`.
    pub schema: String,
    /// The set's version.
    pub set_version: u32,
    /// The targets.
    #[serde(rename = "target")]
    pub targets: Vec<ManifestTarget>,
}

impl ManifestTarget {
    /// An item the one allowed amendment fills and has not filled yet: a test run must leave it out.
    pub fn unfilled_amendable(&self) -> bool {
        self.status.as_deref() == Some("placeholder")
            || self.sha256.as_deref() == Some("pending")
            || (self.kind == "owner-g1" && self.sha256.is_none())
    }

    /// Whether the kit can run it: a generator code or a preset file.
    pub fn runnable(&self) -> bool {
        match self.kind.as_str() {
            "generator" => self.code.is_some(),
            "preset" | "owner-g1" => self.file.is_some() && self.perf.is_some(),
            _ => false,
        }
    }
}

/// Read `synth.toml`.
pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let m: Manifest = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.schema != "enth.gr3-synth/1" {
        return Err(format!(
            "{}: schema {}, not enth.gr3-synth/1 (this kit is older than the set)",
            path.display(),
            m.schema
        ));
    }
    Ok(m)
}

/// Which test targets a run takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Set {
    /// The pre-declared 40-target core (`core = true`).
    Core,
    /// Every filled test target outside the core.
    Rest,
    /// Every filled test target.
    Available,
    /// Every test target: refused while any amendable item is unfilled.
    Full,
}

impl Set {
    /// Parse `core`, `rest`, `available` or `full`.
    pub fn parse(s: &str) -> Result<Set, String> {
        match s {
            "core" => Ok(Set::Core),
            "rest" => Ok(Set::Rest),
            "available" => Ok(Set::Available),
            "full" => Ok(Set::Full),
            _ => Err(format!("--set {s}: one of core, rest, available, full")),
        }
    }

    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Set::Core => "core",
            Set::Rest => "rest",
            Set::Available => "available",
            Set::Full => "full",
        }
    }
}

/// Pick the targets of `split` (`dev` takes every dev target; `test` takes `set`), or exactly `ids` when given, in
/// manifest order. **Refuses** an unfilled amendable item in a test run (the coordinator's ruling of 2026-10-21) and
/// anything the kit cannot run.
pub fn select<'a>(
    m: &'a Manifest,
    split: &str,
    set: Set,
    ids: Option<&[String]>,
) -> Result<Vec<&'a ManifestTarget>, String> {
    let in_split: Vec<&ManifestTarget> = m.targets.iter().filter(|t| t.split == split).collect();
    if in_split.is_empty() {
        return Err(format!("no {split} targets in the manifest"));
    }
    let picked: Vec<&ManifestTarget> = match ids {
        Some(ids) => {
            let mut out = Vec::new();
            for id in ids {
                let t = in_split
                    .iter()
                    .find(|t| &t.id == id)
                    .ok_or_else(|| format!("{id} is not a {split} target"))?;
                out.push(*t);
            }
            out
        }
        None if split == "test" => in_split
            .iter()
            .copied()
            .filter(|t| match set {
                Set::Core => t.core == Some(true),
                Set::Rest => t.core != Some(true) && !t.unfilled_amendable(),
                Set::Available => !t.unfilled_amendable(),
                Set::Full => true,
            })
            .collect(),
        None => in_split,
    };
    if split == "test" {
        let unfilled: Vec<&str> = picked
            .iter()
            .filter(|t| t.unfilled_amendable())
            .map(|t| t.id.as_str())
            .collect();
        if !unfilled.is_empty() {
            return Err(format!(
                "refusing a test run with unfilled amendable items ({}): the one allowed amendment fills them \
                 first (content/gr3/README.md, \"The freeze\"); use --set available or --set rest meanwhile",
                unfilled.join(", ")
            ));
        }
    }
    if let Some(t) = picked.iter().find(|t| !t.runnable()) {
        return Err(format!("{} ({}) cannot run: no code or file", t.id, t.kind));
    }
    if picked.is_empty() {
        return Err("the selection is empty".to_owned());
    }
    Ok(picked)
}

/// SHA-256 of a file's bytes, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// Check `synth.toml` against `hashes.txt` (the freeze): a test run never runs on a changed manifest.
pub fn check_freeze(gr3: &Path) -> Result<String, String> {
    let hashes = std::fs::read_to_string(gr3.join("hashes.txt"))
        .map_err(|e| format!("cannot read {}: {e}", gr3.join("hashes.txt").display()))?;
    let want: BTreeMap<&str, &str> = hashes
        .lines()
        .filter_map(|l| l.split_once("  ").map(|(h, f)| (f.trim(), h.trim())))
        .collect();
    let got = sha256_file(&gr3.join("synth.toml"))?;
    match want.get("synth.toml") {
        Some(w) if *w == got => Ok(got),
        Some(w) => Err(format!(
            "content/gr3/synth.toml has SHA-256 {got}, but hashes.txt pins {w}: the freeze is broken"
        )),
        None => Err("hashes.txt does not pin synth.toml".to_owned()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// runs.jsonl, read back
// ---------------------------------------------------------------------------------------------------------------

/// A checkpoint's scores, as recorded.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct ScoreRec {
    /// `sound/1` %.
    pub percent: f64,
    /// JND units.
    pub jnd: f64,
    /// Passed (≥ 90 %).
    pub passed: bool,
    /// `structure/1` %.
    #[serde(default)]
    pub structure: Option<f64>,
    /// MFCC MSE (the independent metric).
    #[serde(default)]
    pub mfcc: Option<f64>,
}

/// A checkpoint, as recorded.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CheckpointRec {
    /// The mark (s or renders).
    pub mark: f64,
    /// When its round ended (s).
    #[serde(default)]
    pub t_s: f64,
    /// Renders then.
    pub renders: usize,
    /// The objective distance (null when no candidate existed yet).
    #[serde(default)]
    pub objective: Option<f64>,
    /// The best patch's hash.
    #[serde(default)]
    pub patch_sha256: String,
    /// The best candidate's start (`heuristic`, `template`, `knn`, `model`; `none` before any candidate).
    #[serde(default)]
    pub provenance: String,
    /// The scores.
    #[serde(default)]
    pub score: Option<ScoreRec>,
}

/// A target's identity, as recorded.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TargetIdRec {
    /// The id.
    pub id: String,
    /// The kind.
    #[serde(default)]
    pub kind: String,
    /// The level.
    #[serde(default)]
    pub level: Option<u8>,
    /// The stratum.
    #[serde(default)]
    pub stratum: String,
    /// In the core.
    #[serde(default)]
    pub core: bool,
}

/// The machine over a target, as recorded.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
pub struct MachineRec {
    /// Wall seconds.
    #[serde(default)]
    pub seconds: f64,
    /// Other processes' cores.
    #[serde(default)]
    pub others_cores: Option<f64>,
    /// Load before.
    #[serde(default)]
    pub load1_before: Option<f64>,
    /// Locked.
    #[serde(default)]
    pub locked: bool,
    /// Lock wait.
    #[serde(default)]
    pub lock_wait_s: f64,
}

/// A job's end, as recorded.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FinalRec {
    /// Renders.
    #[serde(default)]
    pub renders: usize,
    /// Renders per second.
    #[serde(default)]
    pub renders_per_s: f64,
    /// The branches' provenance kinds.
    #[serde(default)]
    pub branch_provenance: Vec<String>,
}

/// The intake, as recorded.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct IntakeRec {
    /// Warning kinds.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One `target` line.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TargetLine {
    /// The run.
    pub run_id: String,
    /// The arm.
    pub arm: String,
    /// `wall` or `renders`.
    pub mode: String,
    /// The cap.
    pub cap: f64,
    /// The target.
    pub target: TargetIdRec,
    /// The machine.
    #[serde(default)]
    pub machine: MachineRec,
    /// The intake.
    #[serde(default)]
    pub intake: IntakeRec,
    /// The end.
    #[serde(default, rename = "final")]
    pub fin: FinalRec,
    /// The checkpoints.
    pub checkpoints: Vec<CheckpointRec>,
}

impl TargetLine {
    /// The checkpoint at `mark` (exactly), if the run had one.
    pub fn at(&self, mark: f64) -> Option<&CheckpointRec> {
        self.checkpoints.iter().find(|c| c.mark == mark)
    }

    /// The checkpoint at the cap (the last).
    pub fn at_cap(&self) -> Option<&CheckpointRec> {
        self.checkpoints.last()
    }

    /// The first mark whose score passed, or `None` (censored at the cap).
    pub fn time_to_pass(&self) -> Option<f64> {
        self.checkpoints
            .iter()
            .find(|c| c.score.is_some_and(|s| s.passed))
            .map(|c| c.mark)
    }
}

/// A `run` line: kept whole, the report reads a few fields.
#[derive(Clone, Debug, Deserialize)]
pub struct RunLine {
    /// The run.
    pub run_id: String,
    /// The arm.
    pub arm: String,
    /// The mode.
    pub mode: String,
    /// The cap in seconds.
    #[serde(default)]
    pub seconds: Option<f64>,
    /// The budget.
    #[serde(default)]
    pub budget: Option<usize>,
    /// Workers.
    #[serde(default)]
    pub workers: usize,
    /// The split.
    #[serde(default)]
    pub split: String,
    /// The date.
    #[serde(default)]
    pub date: String,
    /// The whole line.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

/// Everything in a runs file, by run id, in file order.
#[derive(Debug, Default)]
pub struct Runs {
    /// The run lines, in file order.
    pub headers: Vec<RunLine>,
    /// Target lines, per run id.
    pub targets: BTreeMap<String, Vec<TargetLine>>,
    /// `target_failed` lines, per run id.
    pub failed: BTreeMap<String, Vec<serde_json::Value>>,
    /// `run_end` lines, per run id.
    pub ended: BTreeMap<String, serde_json::Value>,
}

/// Read a runs file; a line of another schema or record kind is skipped with a note, never an error, so a newer
/// writer's lines never stop an older report.
pub fn read_runs(path: &Path) -> Result<(Runs, Vec<String>), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_runs(&text)
}

/// Parse the text of a runs file.
pub fn parse_runs(text: &str) -> Result<(Runs, Vec<String>), String> {
    let mut runs = Runs::default();
    let mut notes = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: not JSON: {e}", i + 1))?;
        if v.get("schema").and_then(|s| s.as_str()) != Some("enth.gr3-run/1") {
            notes.push(format!("line {}: another schema, skipped", i + 1));
            continue;
        }
        let run_id = v
            .get("run_id")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_owned();
        match v.get("record").and_then(|s| s.as_str()) {
            Some("run") => {
                let mut r: RunLine = serde_json::from_value(v.clone())
                    .map_err(|e| format!("line {}: a run line: {e}", i + 1))?;
                r.raw = v;
                runs.headers.push(r);
            }
            Some("target") => {
                let t: TargetLine = serde_json::from_value(v)
                    .map_err(|e| format!("line {}: a target line: {e}", i + 1))?;
                runs.targets.entry(run_id).or_default().push(t);
            }
            Some("target_failed") => runs.failed.entry(run_id).or_default().push(v),
            Some("run_end") => {
                runs.ended.insert(run_id, v);
            }
            other => notes.push(format!("line {}: record {other:?} skipped", i + 1)),
        }
    }
    Ok((runs, notes))
}

/// The repository's `content/gr3`.
pub fn gr3_dir(root: &Path) -> PathBuf {
    root.join("content/gr3")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
schema = "enth.gr3-synth/1"
set_version = 1

[[target]]
id = "gen-t-1"
split = "test"
kind = "generator"
level = 1
code = "X"
phrase = 1
core = true

[[target]]
id = "gen-t-2"
split = "test"
kind = "generator"
level = 2
code = "Y"
phrase = 2
core = false

[[target]]
id = "hand-t-a"
split = "test"
kind = "preset"
file = "content/gr3/presets/a.patch.json"
sha256 = "ab"
perf = { key = 60, velocity = 100, hold_ms = 2000, length_ms = 3000 }
core = true

[[target]]
id = "g1-bass-a"
split = "test"
kind = "owner-g1"
file = "content/presets/bass.patch.json"
sha256 = "pending"
status = "placeholder"
perf = { key = 36, velocity = 100, hold_ms = 2000, length_ms = 3000 }
core = false

[[target]]
id = "gen-d-1"
split = "dev"
kind = "generator"
level = 1
code = "Z"
phrase = 1
"#;

    fn m() -> Manifest {
        toml::from_str(MANIFEST).unwrap()
    }

    fn ids(v: &[&ManifestTarget]) -> Vec<String> {
        v.iter().map(|t| t.id.clone()).collect()
    }

    #[test]
    fn sets_pick_what_they_say() {
        let m = m();
        assert_eq!(
            ids(&select(&m, "test", Set::Core, None).unwrap()),
            ["gen-t-1", "hand-t-a"]
        );
        assert_eq!(
            ids(&select(&m, "test", Set::Rest, None).unwrap()),
            ["gen-t-2"]
        );
        assert_eq!(
            ids(&select(&m, "test", Set::Available, None).unwrap()),
            ["gen-t-1", "gen-t-2", "hand-t-a"]
        );
        assert_eq!(
            ids(&select(&m, "dev", Set::Core, None).unwrap()),
            ["gen-d-1"]
        );
    }

    #[test]
    fn an_unfilled_amendable_item_is_refused_in_a_test_run() {
        let m = m();
        let e = select(&m, "test", Set::Full, None).unwrap_err();
        assert!(e.contains("g1-bass-a") && e.contains("amendment"), "{e}");
        let e = select(&m, "test", Set::Core, Some(&["g1-bass-a".to_owned()])).unwrap_err();
        assert!(e.contains("unfilled amendable"), "{e}");
        // Filled, it runs.
        let mut m2 = m.clone();
        let g1 = m2.targets.iter_mut().find(|t| t.id == "g1-bass-a").unwrap();
        g1.sha256 = Some("cd".to_owned());
        g1.status = Some("extracted".to_owned());
        assert_eq!(select(&m2, "test", Set::Full, None).unwrap().len(), 4);
        // A dev id is not a test id.
        assert!(select(&m, "test", Set::Core, Some(&["gen-d-1".to_owned()])).is_err());
    }

    #[test]
    fn the_real_manifest_reads_and_its_core_is_forty_without_a_placeholder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let m = read_manifest(&gr3_dir(&root).join("synth.toml")).unwrap();
        let core = select(&m, "test", Set::Core, None).unwrap();
        assert_eq!(core.len(), 40);
        assert!(core.iter().all(|t| t.kind != "owner-g1"));
        assert_eq!(core.iter().filter(|t| t.kind == "generator").count(), 32);
        assert_eq!(select(&m, "dev", Set::Core, None).unwrap().len(), 30);
        // While the G1 sounds are placeholders: the full set is refused, 114 are available, 74 are the rest.
        if m.targets.iter().any(ManifestTarget::unfilled_amendable) {
            assert!(select(&m, "test", Set::Full, None).is_err());
            assert_eq!(select(&m, "test", Set::Available, None).unwrap().len(), 114);
            assert_eq!(select(&m, "test", Set::Rest, None).unwrap().len(), 74);
        }
        check_freeze(&gr3_dir(&root)).unwrap();
    }

    #[test]
    fn runs_files_read_leniently() {
        let text = r#"{"schema":"enth.gr3-run/1","record":"run","run_id":"r1","arm":"A","mode":"wall","seconds":60.0,"workers":8,"split":"test","date":"2026-10-21T00:00:00Z","cfg":{}}
{"schema":"enth.gr3-run/1","record":"target","run_id":"r1","arm":"A","mode":"wall","cap":60.0,"target":{"id":"t1","kind":"generator","level":1,"stratum":"gen-1note","core":true},"checkpoints":[{"mark":10.0,"renders":500,"objective":0.5,"score":{"percent":80.0,"jnd":2.0,"passed":false}},{"mark":60.0,"renders":3000,"objective":0.2,"score":{"percent":95.0,"jnd":0.5,"passed":true}}],"newer_field":1}
{"schema":"enth.gr3-run/2","record":"target"}
{"schema":"enth.gr3-run/1","record":"target_failed","run_id":"r1","target":"t2","error":"x"}
{"schema":"enth.gr3-run/1","record":"run_end","run_id":"r1"}
"#;
        let (runs, notes) = parse_runs(text).unwrap();
        assert_eq!(runs.headers.len(), 1);
        let t = &runs.targets["r1"][0];
        assert_eq!(t.at(60.0).unwrap().renders, 3000);
        assert_eq!(t.time_to_pass(), Some(60.0));
        assert_eq!(runs.failed["r1"].len(), 1);
        assert!(runs.ended.contains_key("r1"));
        assert_eq!(notes.len(), 1);
    }
}
