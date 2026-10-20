//! Golden-render harness (docs/architecture.md: `tests/golden`; driven by `cargo xtask golden [--update]`).
//!
//! A *case* is a text file, `cases/<name>.toml`: a sample rate, a length and a [`source::Source`]. Its *expected* file,
//! `expected/<name>.toml`, holds one SHA-256 of the samples that is valid on every platform, plus a feature summary
//! ([`summary::Summary`]) and a tolerance. Adding a case means adding two text files and no code, so lanes never
//! conflict. Design (P1a report 04, from the Phase 0 harness):
//!
//! - **One hash per case, bit-exact on every platform.** A platform known to differ gets a `[platform_hash]` entry,
//!   either its own hash or `"summary-only"` (summary within tolerance instead). Use that only for platforms that
//!   have not run yet (aarch64, macOS); a difference on x86-64 or wasm is a determinism bug.
//! - **Every mismatching case is reported in one run**, each with a summary diff (RMS per 10 ms, centroid) that says
//!   whether the sound changed or only the last bits did.
//! - **`--update` is refused under `CI`** and prints the list of changed cases, which belongs in the commit message
//!   together with the reason. Reviewers reject golden updates that do not say why.
//! - No WAVs in git: hashes and summaries only.

pub mod challenges;
pub mod patch;
pub mod source;
pub mod summary;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use source::{Rendered, Source};
pub use summary::{Summary, Tolerance, hash_samples, summarise};

/// `[platform_hash]` value meaning "this platform is compared on the summary only".
pub const SUMMARY_ONLY: &str = "summary-only";

/// A case file: `cases/<name>.toml`.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// One line on what the case guards.
    pub description: String,
    pub sample_rate: u32,
    /// Length of the render. Frames = `round(seconds * sample_rate)`, computed in f64.
    pub seconds: f64,
    pub source: Source,
}

impl Case {
    #[must_use]
    pub fn frames(&self) -> usize {
        (self.seconds * f64::from(self.sample_rate)).round() as usize
    }
}

/// An expected file: `expected/<name>.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub sample_rate: u32,
    pub frames: usize,
    /// SHA-256 of the samples, valid on every platform.
    pub hash: String,
    /// Platforms known to differ, keyed by [`platform_key`]: their own hash, or [`SUMMARY_ONLY`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub platform_hash: BTreeMap<String, String>,
    pub tolerance: Tolerance,
    pub summary: Summary,
}

/// What a render measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    pub sample_rate: u32,
    pub frames: usize,
    pub hash: String,
    pub summary: Summary,
}

impl Measured {
    /// Hash every channel (interleaved) with the channel count; summarise the mono mix.
    #[must_use]
    pub fn new(sample_rate: u32, rendered: &Rendered) -> Self {
        Self {
            sample_rate,
            frames: rendered.frames(),
            hash: hash_samples(sample_rate, rendered.channels, &rendered.samples),
            summary: summarise(sample_rate, &rendered.mono()),
        }
    }
}

/// `x86_64-linux`, `x86_64-windows`, `aarch64-macos`, `wasm32-wasi`, ...
/// (`std::env::consts::OS` is empty on `wasm32-wasip1`, so WASI is named explicitly.)
#[must_use]
pub fn platform_key() -> String {
    let os = if cfg!(target_os = "wasi") {
        "wasi"
    } else {
        std::env::consts::OS
    };
    format!("{}-{os}", std::env::consts::ARCH)
}

/// The directory holding `cases/` and `expected/`: `tests/golden` in the checkout this crate was built from.
#[must_use]
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Platforms allowed to write the shared `hash` on `--update`. Others (aarch64, macOS) can only add a
/// `[platform_hash]` override, so a run on a machine that has not proven bit-exactness cannot move the shared hash.
#[must_use]
pub fn is_reference_platform(key: &str) -> bool {
    key.starts_with("x86_64-") || key.starts_with("wasm32-")
}

// ---------------------------------------------------------------------------------------------------------------
// Comparison

/// Check a measurement against its expected file on `platform`. `Err` carries the full multi-line explanation.
pub fn compare(expected: &Expected, got: &Measured, platform: &str) -> Result<(), String> {
    if expected.sample_rate != got.sample_rate || expected.frames != got.frames {
        return Err(format!(
            "length changed: expected {} frames at {} Hz, got {} frames at {} Hz",
            expected.frames, expected.sample_rate, got.frames, got.sample_rate
        ));
    }
    let want = expected
        .platform_hash
        .get(platform)
        .map_or(expected.hash.as_str(), String::as_str);
    if want == SUMMARY_ONLY {
        let diff = SummaryDiff::new(expected, got);
        return if diff.within_tolerance {
            Ok(())
        } else {
            Err(format!(
                "{platform} is compared on the summary only, and the summary moved beyond the tolerance:\n{}",
                diff.text
            ))
        };
    }
    if want == got.hash {
        return Ok(());
    }
    let diff = SummaryDiff::new(expected, got);
    let verdict = if diff.within_tolerance {
        "the summary is within tolerance, so only low-order bits moved (a platform or build difference, or a tiny numeric change)"
    } else {
        "the summary moved beyond the tolerance: the sound changed"
    };
    Err(format!(
        "hash differs on {platform}\n  expected {want}\n  got      {}\n  {verdict}\n{}",
        got.hash, diff.text
    ))
}

/// The difference between two summaries, as text for a human or an agent.
#[derive(Debug)]
pub struct SummaryDiff {
    pub within_tolerance: bool,
    pub text: String,
}

impl SummaryDiff {
    #[must_use]
    pub fn new(expected: &Expected, got: &Measured) -> Self {
        let rms_step_ms = 10.0;
        let centroid_step_ms =
            summary::CENTROID_WINDOW as f64 / f64::from(got.sample_rate) * 1000.0;
        let (rms_text, rms_ok) = diff_series(
            "rms_db",
            "dB",
            rms_step_ms,
            &expected.summary.rms_db,
            &got.summary.rms_db,
            expected.tolerance.rms_db,
        );
        let (centroid_text, centroid_ok) = diff_series(
            "centroid_hz",
            "Hz",
            centroid_step_ms,
            &expected.summary.centroid_hz,
            &got.summary.centroid_hz,
            expected.tolerance.centroid_hz,
        );
        Self {
            within_tolerance: rms_ok && centroid_ok,
            text: format!("{rms_text}\n{centroid_text}"),
        }
    }
}

fn diff_series(
    label: &str,
    unit: &str,
    step_ms: f64,
    expected: &[f64],
    got: &[f64],
    tolerance: f64,
) -> (String, bool) {
    if expected.len() != got.len() {
        return (
            format!(
                "  {label}: {} points expected, {} got",
                expected.len(),
                got.len()
            ),
            false,
        );
    }
    let over: Vec<usize> = (0..expected.len())
        .filter(|&i| (expected[i] - got[i]).abs() > tolerance)
        .collect();
    let (worst, max_dev) = (0..expected.len())
        .map(|i| (i, (expected[i] - got[i]).abs()))
        .fold((0, 0.0_f64), |a, b| if b.1 > a.1 { b } else { a });
    if over.is_empty() {
        return (
            format!("  {label}: max deviation {max_dev:.3} {unit} (tolerance {tolerance} {unit})"),
            true,
        );
    }
    let mut text = format!(
        "  {label}: {} of {} points differ by more than {tolerance} {unit}; worst {max_dev:.3} {unit} at {:.0} ms",
        over.len(),
        expected.len(),
        worst as f64 * step_ms
    );
    for &i in over.iter().take(3) {
        let _ = write!(
            text,
            "\n    t = {:.0} ms: expected {}, got {}",
            i as f64 * step_ms,
            expected[i],
            got[i]
        );
    }
    if over.len() > 3 {
        let _ = write!(text, "\n    ... and {} more", over.len() - 3);
    }
    (text, false)
}

// ---------------------------------------------------------------------------------------------------------------
// Running every case

/// How to run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory with `cases/` and `expected/`.
    pub dir: PathBuf,
    /// Rewrite the expected files instead of comparing.
    pub update: bool,
    /// The platform to compare for; [`platform_key`] unless a test overrides it.
    pub platform: String,
}

impl Options {
    /// Compare every case in this checkout.
    #[must_use]
    pub fn check() -> Self {
        Self {
            dir: root(),
            update: false,
            platform: platform_key(),
        }
    }

    /// Rewrite the expected files in this checkout.
    #[must_use]
    pub fn update() -> Self {
        Self {
            update: true,
            ..Self::check()
        }
    }
}

/// What happened to one case.
#[derive(Debug, PartialEq)]
pub enum Status {
    Ok,
    /// `--update` created the expected file.
    Created,
    /// `--update` changed the expected file.
    Updated,
    Failed(String),
}

#[derive(Debug)]
pub struct CaseReport {
    pub name: String,
    pub status: Status,
}

#[derive(Debug)]
pub struct Report {
    pub platform: String,
    pub update: bool,
    pub cases: Vec<CaseReport>,
    /// Expected files with no case. Removed on `--update`, a failure otherwise.
    pub stale: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        let no_failures = self
            .cases
            .iter()
            .all(|c| !matches!(c.status, Status::Failed(_)));
        no_failures && (self.stale.is_empty() || self.update)
    }

    /// Names of the cases `--update` created or changed (for the commit message).
    #[must_use]
    pub fn changed(&self) -> Vec<&str> {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, Status::Created | Status::Updated))
            .map(|c| c.name.as_str())
            .collect()
    }

    /// The report, ready to print.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "golden: {} case(s) on {}{}\n",
            self.cases.len(),
            self.platform,
            if self.update { " (update)" } else { "" }
        );
        for c in &self.cases {
            match &c.status {
                Status::Ok => {
                    let _ = writeln!(out, "  ok       {}", c.name);
                }
                Status::Created => {
                    let _ = writeln!(out, "  created  {}", c.name);
                }
                Status::Updated => {
                    let _ = writeln!(out, "  updated  {}", c.name);
                }
                Status::Failed(why) => {
                    let _ = writeln!(out, "  FAILED   {}", c.name);
                    for line in why.lines() {
                        let _ = writeln!(out, "           {line}");
                    }
                }
            }
        }
        for s in &self.stale {
            let verb = if self.update { "removed" } else { "STALE" };
            let _ = writeln!(
                out,
                "  {verb:<8} expected/{s}.toml has no case in cases/{s}.toml"
            );
        }
        let failed = self
            .cases
            .iter()
            .filter(|c| matches!(c.status, Status::Failed(_)))
            .count();
        if self.update {
            let changed = self.changed();
            if changed.is_empty() && self.stale.is_empty() {
                out.push_str("golden: nothing changed\n");
            } else {
                out.push_str(
                    "golden: expected files changed. Put this list, and why the sound changed, in the commit message:\n",
                );
                for name in changed {
                    let _ = writeln!(out, "  - {name}");
                }
                for name in &self.stale {
                    let _ = writeln!(out, "  - {name} (removed)");
                }
            }
        }
        if failed > 0 {
            let _ = writeln!(
                out,
                "golden: {failed} of {} case(s) failed. If the change is deliberate: `cargo xtask golden --update`, then commit with the reason.",
                self.cases.len()
            );
        } else if !self.is_ok() {
            out.push_str(
                "golden: stale expected files; delete them or run `cargo xtask golden --update`\n",
            );
        } else if !self.update {
            out.push_str("golden: ok\n");
        }
        out
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
}

fn toml_names(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

fn read_expected(path: &Path) -> Result<Option<Expected>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The new contents of an expected file after `--update` on `platform`, or `Err` when this platform may not create it.
fn updated(old: Option<&Expected>, got: &Measured, platform: &str) -> Result<Expected, String> {
    let mut new = old.cloned().unwrap_or_else(|| Expected {
        sample_rate: got.sample_rate,
        frames: got.frames,
        hash: got.hash.clone(),
        platform_hash: BTreeMap::new(),
        tolerance: Tolerance::default(),
        summary: got.summary.clone(),
    });
    if is_reference_platform(platform) {
        new.hash.clone_from(&got.hash);
        // The shared hash was just proven on a reference platform; drop an override only if it now agrees.
        new.platform_hash
            .retain(|_, v| v == SUMMARY_ONLY || *v != got.hash);
    } else if old.is_none() {
        return Err(format!(
            "{platform} may not create a golden: create it with `cargo xtask golden --update` on x86-64 (Linux, Windows) or wasm first"
        ));
    } else if new.hash == got.hash {
        new.platform_hash.remove(platform);
    } else {
        new.platform_hash
            .insert(platform.to_owned(), got.hash.clone());
    }
    new.sample_rate = got.sample_rate;
    new.frames = got.frames;
    new.summary = got.summary.clone();
    Ok(new)
}

/// An expected file as text: scalars first, then the two series in rows of ten so a long render stays readable.
/// (`toml::to_string_pretty` puts every number on its own line; `toml::to_string` puts them all on one.)
#[must_use]
pub fn to_toml(e: &Expected) -> String {
    fn series(out: &mut String, key: &str, values: &[f64]) {
        if values.is_empty() {
            let _ = writeln!(out, "{key} = []");
            return;
        }
        let _ = writeln!(out, "{key} = [");
        for row in values.chunks(10) {
            let cells: Vec<String> = row.iter().map(|v| format!("{v:?}")).collect();
            let _ = writeln!(out, "    {},", cells.join(", "));
        }
        out.push_str("]\n");
    }
    let mut out = String::new();
    let _ = writeln!(out, "sample_rate = {}", e.sample_rate);
    let _ = writeln!(out, "frames = {}", e.frames);
    let _ = writeln!(out, "hash = {:?}", e.hash);
    if !e.platform_hash.is_empty() {
        out.push_str("\n[platform_hash]\n");
        for (platform, hash) in &e.platform_hash {
            let _ = writeln!(out, "{platform:?} = {hash:?}");
        }
    }
    let _ = writeln!(
        out,
        "\n[tolerance]\nrms_db = {:?}\ncentroid_hz = {:?}",
        e.tolerance.rms_db, e.tolerance.centroid_hz
    );
    out.push_str("\n[summary]\n");
    series(&mut out, "rms_db", &e.summary.rms_db);
    series(&mut out, "centroid_hz", &e.summary.centroid_hz);
    out
}

/// Run every case in `opts.dir`.
///
/// # Errors
/// A problem with the harness itself (no `cases/` directory, `--update` under `CI`); a case that fails is reported in
/// the [`Report`], not here.
pub fn run(opts: &Options) -> Result<Report, String> {
    if opts.update && std::env::var_os("CI").is_some() {
        return Err(
            "refusing to update goldens under CI; run `cargo xtask golden --update` locally and commit the diff with the reason"
                .to_owned(),
        );
    }
    let cases_dir = opts.dir.join("cases");
    let expected_dir = opts.dir.join("expected");
    let names = toml_names(&cases_dir)?;
    if names.is_empty() {
        return Err(format!("no golden cases in {}", cases_dir.display()));
    }
    let mut report = Report {
        platform: opts.platform.clone(),
        update: opts.update,
        cases: Vec::new(),
        stale: Vec::new(),
    };
    for name in &names {
        let status = run_case(opts, &cases_dir, &expected_dir, name);
        report.cases.push(CaseReport {
            name: name.clone(),
            status,
        });
    }
    if expected_dir.is_dir() {
        for name in toml_names(&expected_dir)? {
            if !names.contains(&name) {
                if opts.update {
                    let path = expected_dir.join(format!("{name}.toml"));
                    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                }
                report.stale.push(name);
            }
        }
    }
    Ok(report)
}

fn run_case(opts: &Options, cases_dir: &Path, expected_dir: &Path, name: &str) -> Status {
    try_case(opts, cases_dir, expected_dir, name).unwrap_or_else(Status::Failed)
}

fn try_case(
    opts: &Options,
    cases_dir: &Path,
    expected_dir: &Path,
    name: &str,
) -> Result<Status, String> {
    if !valid_name(name) {
        return Err("case names are lowercase letters, digits and '-'".to_owned());
    }
    let path = cases_dir.join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let case: Case = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if !(8_000..=384_000).contains(&case.sample_rate) {
        return Err(format!(
            "sample_rate {} is outside 8000..=384000",
            case.sample_rate
        ));
    }
    if !(case.seconds.is_finite() && case.seconds > 0.0 && case.seconds <= 120.0) {
        return Err(format!("seconds {} is outside (0, 120]", case.seconds));
    }
    let rendered = case.source.render(case.sample_rate, case.frames())?;
    let got = Measured::new(case.sample_rate, &rendered);
    let expected_path = expected_dir.join(format!("{name}.toml"));
    let old = read_expected(&expected_path)?;
    if opts.update {
        let new = updated(old.as_ref(), &got, &opts.platform)?;
        if old.as_ref() == Some(&new) {
            return Ok(Status::Ok);
        }
        std::fs::create_dir_all(expected_dir)
            .and_then(|()| std::fs::write(&expected_path, to_toml(&new)))
            .map_err(|e| format!("{}: {e}", expected_path.display()))?;
        return Ok(if old.is_some() {
            Status::Updated
        } else {
            Status::Created
        });
    }
    let old = old.ok_or_else(|| {
        format!(
            "{} is missing; run `cargo xtask golden --update` and commit it",
            expected_path.display()
        )
    })?;
    compare(&old, &got, &opts.platform)?;
    Ok(Status::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(hash: &str, rms: &[f64]) -> Measured {
        Measured {
            sample_rate: 48_000,
            frames: 4800,
            hash: hash.to_owned(),
            summary: Summary {
                rms_db: rms.to_vec(),
                centroid_hz: vec![440.0],
            },
        }
    }

    fn expected(hash: &str, rms: &[f64]) -> Expected {
        let m = measured(hash, rms);
        Expected {
            sample_rate: m.sample_rate,
            frames: m.frames,
            hash: m.hash,
            platform_hash: BTreeMap::new(),
            tolerance: Tolerance::default(),
            summary: m.summary,
        }
    }

    #[test]
    fn equal_hashes_pass_without_looking_at_the_summary() {
        let e = expected("aa", &[-6.0, -6.0]);
        assert!(compare(&e, &measured("aa", &[-60.0, -60.0]), "x86_64-linux").is_ok());
    }

    #[test]
    fn a_hash_change_inside_the_tolerance_is_called_low_order_bits() {
        let e = expected("aa", &[-6.0, -6.0]);
        let why = compare(&e, &measured("bb", &[-6.001, -6.0]), "x86_64-linux").unwrap_err();
        assert!(why.contains("only low-order bits moved"), "{why}");
        assert!(why.contains("expected aa"), "{why}");
    }

    #[test]
    fn a_hash_change_beyond_the_tolerance_names_the_frames() {
        let e = expected("aa", &[-6.0, -6.0, -6.0, -6.0, -6.0]);
        let why = compare(
            &e,
            &measured("bb", &[-6.0, -3.0, -3.0, -3.0, -3.0]),
            "x86_64-linux",
        )
        .unwrap_err();
        assert!(why.contains("the sound changed"), "{why}");
        assert!(why.contains("4 of 5 points"), "{why}");
        assert!(why.contains("t = 10 ms: expected -6, got -3"), "{why}");
        assert!(why.contains("... and 1 more"), "{why}");
    }

    #[test]
    fn a_length_change_is_reported_first() {
        let e = expected("aa", &[-6.0]);
        let mut m = measured("aa", &[-6.0]);
        m.frames = 4801;
        let why = compare(&e, &m, "x86_64-linux").unwrap_err();
        assert!(why.starts_with("length changed"), "{why}");
    }

    #[test]
    fn a_platform_override_replaces_the_shared_hash() {
        let mut e = expected("aa", &[-6.0]);
        e.platform_hash.insert("aarch64-macos".into(), "cc".into());
        assert!(compare(&e, &measured("cc", &[-6.0]), "aarch64-macos").is_ok());
        assert!(compare(&e, &measured("aa", &[-6.0]), "aarch64-macos").is_err());
        assert!(compare(&e, &measured("aa", &[-6.0]), "x86_64-linux").is_ok());
    }

    #[test]
    fn summary_only_platforms_pass_on_the_summary() {
        let mut e = expected("aa", &[-6.0]);
        e.platform_hash
            .insert("aarch64-macos".into(), SUMMARY_ONLY.into());
        assert!(compare(&e, &measured("zz", &[-6.004]), "aarch64-macos").is_ok());
        let why = compare(&e, &measured("zz", &[-5.0]), "aarch64-macos").unwrap_err();
        assert!(why.contains("summary only"), "{why}");
    }

    #[test]
    fn only_reference_platforms_write_the_shared_hash() {
        let old = expected("aa", &[-6.0]);
        let got = measured("bb", &[-6.0]);
        assert_eq!(
            updated(Some(&old), &got, "x86_64-linux").unwrap().hash,
            "bb"
        );
        let mac = updated(Some(&old), &got, "aarch64-macos").unwrap();
        assert_eq!(mac.hash, "aa");
        assert_eq!(mac.platform_hash["aarch64-macos"], "bb");
        assert!(updated(None, &got, "aarch64-macos").is_err());
        // An override that now agrees with the shared hash disappears.
        let agreeing = updated(Some(&mac), &measured("aa", &[-6.0]), "aarch64-macos").unwrap();
        assert!(agreeing.platform_hash.is_empty());
    }

    #[test]
    fn the_expected_file_text_reads_back_to_the_same_value() {
        let mut e = expected(
            "aa",
            &(0..23).map(|i| -f64::from(i) / 8.0).collect::<Vec<_>>(),
        );
        e.platform_hash
            .insert("aarch64-macos".into(), SUMMARY_ONLY.into());
        e.summary.centroid_hz = Vec::new();
        let text = to_toml(&e);
        assert!(text.lines().all(|l| l.len() < 100), "{text}");
        assert_eq!(toml::from_str::<Expected>(&text).unwrap(), e, "{text}");
    }

    #[test]
    fn case_names_are_restricted() {
        assert!(valid_name("osc-saw-1v"));
        assert!(!valid_name("Osc"));
        assert!(!valid_name("-x"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
    }
}
