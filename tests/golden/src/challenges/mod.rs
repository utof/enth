//! Golden challenges (lane GC; docs/phase-2.md "Test strategy for the game logic", docs/challenge.md "Golden
//! challenges"): a challenge code, the target it regenerates, and what the scorers say about a few fixed answers,
//! pinned as text and checked on every platform.
//!
//! ```text
//! tests/golden/challenges/
//!   cases/<name>.toml      the input: a code, and recipes for the answers beyond `exact` and `start`
//!   expected/<name>.txt    the pins: the code's facts, the target's fingerprint and render hash, and per scorer set the
//!                          score report of every answer (sound %, structure %, combined %, pass, components, rings)
//! ```
//!
//! Adding a case means adding a `cases/` file and running `cargo xtask golden --challenges --update`, which writes its
//! `expected/` file. Nothing in a test or in CI ever writes.
//!
//! - **What is pinned.** The code (and its decoded settings), the target's fingerprint (never a patch hash: the
//!   fingerprint moves only with the target, [`enth_challenge::fingerprint`]), the target's scored render (a SHA-256
//!   of the Target ▶ audio, to tell a render difference from a scoring difference) and loudness, and for each answer
//!   the whole report. Numbers are compared within [`TOLERANCE`] = 1e-6 (the scores are bit-exact, so the
//!   tolerance stands for the printed digits only).
//! - **The margin.** No pass decision sits within [`MARGIN`] = 1e-3 of the pass mark ([`margin`]); a case that does
//!   fails loudly with the way out, in a check and in an update.
//! - **Frozen per scorer set.** A set of scorers (`sound/1 structure/1 combined/1 time/1`) is a block; a new scorer
//!   is a new block beside the old, written by `--update`, and an existing block never changes ([`text::merge`]).
//!   `--update` refuses a change to anything pinned, and says so: a deliberate re-pin (the engine or a generator
//!   moved, before any code was shared) is deleting the `expected/` file first, which shows in the diff.
//! - **Scorers named explicitly.** [`compute::Scoring`] builds each challenge with [`enth_challenge::model::ScoringSpec::with_scorers`],
//!   so a new default scorer never changes a golden.

pub mod case;
pub mod compute;
pub mod margin;
pub mod propose;
pub mod text;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use case::Case;
use compute::Scoring;
use text::{Doc, diff_block, diff_lines, merge};

/// The tolerance on a pinned number (docs/phase-2.md: scores to 1e-6).
pub const TOLERANCE: f64 = 1e-6;

/// The smallest distance a pass decision may have to the pass mark (docs/phase-2.md: 1e-3).
pub const MARGIN: f64 = 1e-3;

/// The directory holding `cases/` and `expected/`: `tests/golden/challenges` in the checkout this crate was built
/// from.
#[must_use]
pub fn root() -> PathBuf {
    crate::root().join("challenges")
}

/// How to run the challenges.
#[derive(Debug, Clone)]
pub struct Options {
    /// The directory holding `cases/` and `expected/`.
    pub dir: PathBuf,
    /// Write: new cases, new scorer sets and new answers. Never a change to a pinned line.
    pub update: bool,
    /// Run only this case.
    pub only: Option<String>,
}

impl Options {
    /// Compare every case with its pins.
    #[must_use]
    pub fn check() -> Self {
        Self {
            dir: root(),
            update: false,
            only: None,
        }
    }

    /// Write what is new.
    #[must_use]
    pub fn update() -> Self {
        Self {
            update: true,
            ..Self::check()
        }
    }
}

/// What happened to one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Every pin holds (and, in an update, nothing was new).
    Ok,
    /// An update wrote the case's expected file.
    Created,
    /// An update added these to a pinned case.
    Extended(Vec<String>),
    /// The pins do not hold, the case is unusable, or an update was refused: the explanation.
    Failed(String),
}

/// One case's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseReport {
    /// The case's name.
    pub name: String,
    /// What happened.
    pub status: Status,
    /// A digest of every number the case computed, as raw bits (empty when it failed before computing): equal digests
    /// mean equal bits. Information for comparing two runs (native and wasm); the pins use the tolerance.
    pub bits: String,
}

/// Every case's result.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Whether this was an update.
    pub update: bool,
    /// In case-name order.
    pub cases: Vec<CaseReport>,
    /// Expected files with no case. Pins are never deleted for you.
    pub stale: Vec<String>,
}

impl Report {
    /// Whether every case holds and nothing is stale.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.stale.is_empty()
            && self
                .cases
                .iter()
                .all(|c| !matches!(c.status, Status::Failed(_)))
    }

    /// One digest over every case's bits digest, in case order: equal digests on two machines (or native and wasm)
    /// mean every number agrees to the last bit.
    #[must_use]
    pub fn bits(&self) -> String {
        let mut h = Sha256::new();
        for c in &self.cases {
            h.update(c.name.as_bytes());
            h.update(c.bits.as_bytes());
        }
        h.finalize().iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    /// The report as text, for a terminal or a failed assertion.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut failed = 0;
        for c in &self.cases {
            match &c.status {
                Status::Ok => {
                    let _ = writeln!(out, "  ok       {}", c.name);
                }
                Status::Created => {
                    let _ = writeln!(out, "  created  {}", c.name);
                }
                Status::Extended(what) => {
                    let _ = writeln!(out, "  extended {}: {}", c.name, what.join("; "));
                }
                Status::Failed(why) => {
                    failed += 1;
                    let _ = writeln!(out, "  FAILED   {}\n{}", c.name, indent(why));
                }
            }
        }
        for name in &self.stale {
            let _ = writeln!(
                out,
                "  STALE    expected/{name}.txt has no cases/{name}.toml; a pin is never deleted for you: delete it by hand if the case is gone for good"
            );
        }
        let _ = writeln!(
            out,
            "golden challenges: {} case(s), {failed} failed, {} stale; bits {}",
            self.cases.len(),
            self.stale.len(),
            self.bits()
        );
        if failed > 0 && !self.update {
            out.push_str(
                "A pin that moved under the same scorer ids is a regression to fix (the scorer, the generator or the engine \
                 changed), not a file to update. A new scorer or a new case is added with `cargo xtask golden --challenges \
                 --update`.\n",
            );
        }
        out
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("      {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Names are lowercase letters, digits and `-`.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
}

fn names(dir: &Path, extension: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().is_some_and(|e| e == extension)
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Run every case (or `opts.only`).
///
/// # Errors
/// A problem with the harness itself (no `cases/` directory, an update under `CI`); a case that fails is reported in
/// the [`Report`], not here.
pub fn run(opts: &Options) -> Result<Report, String> {
    if opts.update && std::env::var_os("CI").is_some() {
        return Err("refusing to update golden challenges under CI; run `cargo xtask golden --challenges --update` locally and commit the diff with the reason".to_owned());
    }
    let cases_dir = opts.dir.join("cases");
    let expected_dir = opts.dir.join("expected");
    let all = names(&cases_dir, "toml")?;
    if all.is_empty() {
        return Err(format!("no golden challenges in {}", cases_dir.display()));
    }
    if let Some(only) = &opts.only
        && !all.contains(only)
    {
        return Err(format!("no case `{only}` in {}", cases_dir.display()));
    }
    let mut report = Report {
        update: opts.update,
        ..Report::default()
    };
    for name in &all {
        if opts.only.as_ref().is_some_and(|o| o != name) {
            continue;
        }
        let (status, bits) = match try_case(opts, &cases_dir, &expected_dir, name) {
            Ok((status, bits)) => (status, bits),
            Err(why) => (Status::Failed(why), String::new()),
        };
        report.cases.push(CaseReport {
            name: name.clone(),
            status,
            bits,
        });
    }
    if opts.only.is_none() && expected_dir.is_dir() {
        for name in names(&expected_dir, "txt")? {
            if !all.contains(&name) {
                report.stale.push(name);
            }
        }
    }
    Ok(report)
}

/// The title comment of an expected file.
fn title(name: &str) -> String {
    format!(
        "enth golden challenge `{name}` (challenges/cases/{name}.toml). Pinned: the code's facts, the target's fingerprint \
         and render, and per scorer set every answer's report. Nothing here changes under the same scorer ids; written \
         only by `cargo xtask golden --challenges --update`."
    )
}

fn try_case(
    opts: &Options,
    cases_dir: &Path,
    expected_dir: &Path,
    name: &str,
) -> Result<(Status, String), String> {
    if !valid_name(name) {
        return Err("case names are lowercase letters, digits and '-'".to_owned());
    }
    let case_path = cases_dir.join(format!("{name}.toml"));
    let text =
        std::fs::read_to_string(&case_path).map_err(|e| format!("{}: {e}", case_path.display()))?;
    let case = Case::parse(&text).map_err(|e| format!("{}: {e}", case_path.display()))?;
    let expected_path = expected_dir.join(format!("{name}.txt"));
    let pinned = match std::fs::read_to_string(&expected_path) {
        Ok(text) => {
            Some(Doc::parse(&text).map_err(|e| format!("{}: {e}", expected_path.display()))?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", expected_path.display())),
    };

    // The scorer sets to compute: the case's, then every set already pinned (a pin is always checked).
    let mut scorings: Vec<Scoring> = Vec::new();
    for set in &case.scorings {
        let scoring = Scoring::named(set)
            .ok_or_else(|| format!("scorings: unknown set `{set}` (v1, stub)"))?;
        if !scorings.contains(&scoring) {
            scorings.push(scoring);
        }
    }
    let case_sets = scorings.len();
    if let Some(pinned) = &pinned {
        for block in &pinned.blocks {
            let scoring = Scoring::from_key(&block.key())
                .ok_or_else(|| format!("pinned block `{}` names no scorers", block.key()))?;
            if !scorings.contains(&scoring) {
                scorings.push(scoring);
            }
        }
    }

    let subject = compute::subject(&case)?;
    let computed = compute::document(name, &case, &subject, &scorings)?;
    let (fresh, bits) = (computed.doc, computed.bits);

    match (opts.update, pinned) {
        (false, None) => Err(format!(
            "{} is missing; run `cargo xtask golden --challenges --update` and commit it",
            expected_path.display()
        )),
        (false, Some(pinned)) => {
            let mut problems = diff_lines("header", &pinned.header, &fresh.header, 6);
            for scoring in &scorings[..case_sets] {
                if pinned.block(&scoring.key()).is_none() {
                    problems.push(format!(
                        "the scorer set `{}` is not pinned yet; run `cargo xtask golden --challenges --update`",
                        scoring.key()
                    ));
                }
            }
            for old in &pinned.blocks {
                if let Some(new) = fresh.block(&old.key()) {
                    problems.extend(diff_block(old, new, 6));
                }
            }
            if problems.is_empty() {
                Ok((Status::Ok, bits))
            } else {
                Err(problems.join("\n"))
            }
        }
        (true, None) => {
            write(&expected_path, &fresh.render(&title(name)))?;
            Ok((Status::Created, bits))
        }
        (true, Some(pinned)) => {
            let merged = merge(&pinned, &fresh)
                .map_err(|problems| format!("{}\n{}", problems.join("\n"), text::FROZEN))?;
            if merged == pinned {
                return Ok((Status::Ok, bits));
            }
            let added = added(&pinned, &merged);
            write(&expected_path, &merged.render(&title(name)))?;
            Ok((Status::Extended(added), bits))
        }
    }
}

fn added(old: &Doc, new: &Doc) -> Vec<String> {
    let mut out = Vec::new();
    for block in &new.blocks {
        match old.block(&block.key()) {
            None => out.push(format!("scorer set {}", block.key())),
            Some(was) => {
                for a in &block.answers {
                    if !was.answers.iter().any(|w| w.name == a.name) {
                        out.push(format!("answer {} under {}", a.name, block.key()));
                    }
                }
            }
        }
    }
    out
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}
