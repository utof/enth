//! **Owner: lane SKR, then CL.** `enth compare <patch> --code <code>` (or `--challenge FILE`): score a patch against
//! a challenge's target with the scorers the challenge names, through `enth_challenge::score_round`, the same function
//! the game's worker and the golden challenges call. Until lanes SS and ST land, the default scorers are the stubs
//! (`sound-stub/1`, `structure-stub/1`, `combined-stub/1`: parameter space, no render); a code's challenge switches to
//! the real ones by itself when they do. Lane SKR wrote this first version for the round skeleton (F2r); lane CL
//! finishes it (`enth.compare/1`).
//!
//! Exit 0 when the patch passes (sound % at or above the pass mark), 1 when it does not, like `enth check`.

use std::path::{Path, PathBuf};

use enth_challenge::{Challenge, prepare_challenge, score_round};

use super::{Reply, Res};
use crate::error::{self, EXIT_CHECK_FAILED};
use crate::patchio;
use crate::schema::{CompareOut, ids};

/// `enth compare`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// The patch to score (*.patch.json).
    pub patch: PathBuf,
    /// The challenge's code.
    #[arg(long, value_name = "CODE", required_unless_present = "challenge")]
    pub code: Option<String>,
    /// A challenge file (`enth.challenge/1` JSON) instead of a code.
    #[arg(long, value_name = "FILE", conflicts_with = "code")]
    pub challenge: Option<PathBuf>,
}

/// Run `enth compare`.
pub fn run(args: &Args) -> Res {
    compare(&args.patch, args.code.as_deref(), args.challenge.as_deref())
}

fn compare(patch_path: &Path, code: Option<&str>, file: Option<&Path>) -> Res {
    let (patch, mut notes) = patchio::load(patch_path)?;
    let (round, code_text) = if let Some(file) = file {
        let bytes = patchio::read_bytes(file)?;
        let text = String::from_utf8_lossy(&bytes);
        let challenge = Challenge::from_json(&text).map_err(|e| {
            error::invalid(
                e.code(),
                format!("{} is not a challenge: {e}", patchio::show(file)),
                e.hint(),
            )
            .in_file(patchio::show(file))
        })?;
        let round = prepare_challenge(challenge).map_err(|e| {
            error::invalid(
                e.code(),
                format!("the challenge's target cannot be regenerated: {e}"),
                e.hint(),
            )
        })?;
        let code = round
            .challenge()
            .code(round.fingerprint().prefix())
            .map(|c| c.to_string());
        (round, code)
    } else {
        let text = code.unwrap_or_default();
        let code = super::challenge::decode(text)?;
        let (round, warnings) = super::challenge::regenerate(&code)?;
        notes.extend(warnings);
        (round, Some(code.to_string()))
    };
    let report = score_round(&round, &patch).map_err(|e| {
        error::failed(
            e.code(),
            format!("the patch could not be scored: {e}"),
            e.hint(),
        )
    })?;
    let out = CompareOut {
        schema: ids::COMPARE.to_owned(),
        patch: patchio::show(patch_path),
        code: code_text,
        fingerprint: round.fingerprint().to_hex(),
        sound: report.sound.percent,
        structure: report.structure.percent,
        combined: report.combined.percent,
        pass_mark: report.pass_mark,
        passed: report.passed,
        margin: report.margin(),
        scorers: vec![
            report.sound.scorer.to_string(),
            report.structure.scorer.to_string(),
            report.combined.scorer.to_string(),
        ],
        report: serde_json::to_value(&report).unwrap_or_default(),
        hint: if report.passed {
            "it passes: sound % is at or above the pass mark".to_owned()
        } else {
            "not yet: the report's components say what differs (`enth challenge show <code>` for the phrase)"
                .to_owned()
        },
    };
    let mut reply = Reply::new(&out).columns(&["sound", "structure", "combined", "passed"]);
    if !report.passed {
        reply = reply.code(EXIT_CHECK_FAILED);
    }
    for n in &notes {
        reply = reply.note(n);
    }
    Ok(reply)
}
