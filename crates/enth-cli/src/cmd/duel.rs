//! **Owner: lane SKD** (`enth duel result|verify`, freeze point F2d), then **EV4** (the rest of the command and the agent
//! tracks, additive).
//!
//! `enth duel` (docs/duels.md, "The flow" and "Verification"; docs/agent-cli.md, "Duels on the command line"): the
//! plugin's duel card on the command line, on the same `enth-challenge` functions, so the CLI checks what the plugin
//! checks. **No network code**: a code is text the user moves.
//!
//! - `enth duel result <PATCH> --code <CODE> [--submits N] [--seconds S] [--hints N] [--salt HEX]` makes a played
//!   patch's two codes (`enth.duel-result/1`): the answer code (`AnswerCode::new`, the patch diffed from the round's
//!   start, its `meta`, labels, macro names and slots stripped), the round prepared once and the **canonical answer**
//!   scored on it (what a receiver re-scores), then the scorecard (`Scorecard::new` with the answer's commitment) and
//!   the share message. The salt is 64 bits of OS randomness (std's `RandomState` keys: the CLI adds no dependency),
//!   **never a clock or a seed**; `--salt` makes the same answer code again from a salt an earlier result printed.
//! - `enth duel verify <TEXT>... [--mine PATCH] [--out FILE]` reads a whole chat message (paste-anything; `-` reads
//!   stdin), and checks every answer code in it on this computer (`enth.duel-verify/1`): decoded (the challenge
//!   regenerated, the ops applied, the tail checked), the round prepared once with the scorecard's scorer set when this
//!   build has it (`verify::scorer_set`), the answer scored, the claim compared ("as reported", another engine or
//!   scorer set flagged, the commitment checked). `--mine` scores your patch as its canonical answer on the same round,
//!   so both numbers come from one machine; `--out` writes their answer's patch (one answer only).
//!
//! The wording is the duel card's: scores are "checked on your computer", times and assists "reported".

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use enth_challenge::answer::AnswerCode;
use enth_challenge::paste::{self, Found};
use enth_challenge::result::{CardAssists, ResultError, Scorecard, ScorerSet};
use enth_challenge::verify::{self, VerifyError};
use enth_challenge::{AssistsUsed, EngineVersion, score_round};

use super::{Reply, Res, game};
use crate::error::{self, CliError};
use crate::patchio;
use crate::schema::{
    DuelCardOut, DuelCheckOut, DuelClaimOut, DuelFoundOut, DuelRefusedOut, DuelReportedOut,
    DuelResultOut, DuelSymbolsOut, DuelVerifyOut, ids,
};

/// `enth duel`'s arguments.
#[derive(Clone, Debug, PartialEq, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Sub,
}

/// `enth duel`'s subcommands.
#[derive(Clone, Debug, PartialEq, clap::Subcommand)]
pub enum Sub {
    /// A played patch's codes: the scorecard to send now, the answer code to send once your friend has played.
    Result {
        /// The patch you played (your answer to the challenge).
        patch: PathBuf,
        /// The challenge code it was played on (case, hyphens and spaces are ignored).
        #[arg(long, value_name = "CODE")]
        code: String,
        /// Submits up to the compared one (the first passing submit, else the best), 1-127. Reported, not checked.
        #[arg(long, value_name = "N", default_value_t = 1)]
        submits: u32,
        /// Seconds to the pass (or the round's active time without one). Reported, not checked.
        #[arg(long, value_name = "S", default_value_t = 0.0)]
        seconds: f64,
        /// Hints used, 0-15. Reported, not checked.
        #[arg(long, value_name = "N", default_value_t = 0)]
        hints: u8,
        /// The salt an earlier `duel result` printed (16 hex digits), to make the same answer code again. Default: 64
        /// bits of OS randomness. Never make one up from a clock or a counter: the answer code carries it in the clear.
        #[arg(long, value_name = "HEX")]
        salt: Option<String>,
    },
    /// Check a friend's codes: every code in the text, each answer re-scored on this computer against their scorecard.
    Verify {
        /// The pasted text (a whole chat message is fine); `-` reads it from stdin.
        #[arg(required = true, num_args = 1.., value_name = "TEXT")]
        text: Vec<String>,
        /// Your own patch, scored as its canonical answer on the same round (both numbers from one machine).
        #[arg(long, value_name = "PATCH")]
        mine: Option<PathBuf>,
        /// Write their answer's patch here (one answer code in the text only).
        #[arg(long, value_name = "PATCH")]
        out: Option<PathBuf>,
    },
}

/// Run `enth duel`.
pub fn run(args: &Args, stdin: &str) -> Res {
    match &args.cmd {
        Sub::Result {
            patch,
            code,
            submits,
            seconds,
            hints,
            salt,
        } => result(&ResultArgs {
            patch,
            code,
            submits: *submits,
            seconds: *seconds,
            hints: *hints,
            salt: salt.as_deref(),
        }),
        Sub::Verify { text, mine, out } => {
            verify_text(text, stdin, mine.as_deref(), out.as_deref())
        }
    }
}

/// What `duel result` was asked for.
struct ResultArgs<'a> {
    patch: &'a Path,
    code: &'a str,
    submits: u32,
    seconds: f64,
    hints: u8,
    salt: Option<&'a str>,
}

/// 64 bits of OS randomness: std's `RandomState` keys (drawn from the operating system once per thread) through
/// SipHash. Never a clock or a counter (docs/duels.md, "Privacy").
fn fresh_salt() -> u64 {
    let mut h = std::hash::RandomState::new().build_hasher();
    h.write(b"enth-duel-salt");
    h.finish()
}

/// `--salt`: exactly 16 hex digits.
fn parse_salt(text: &str) -> Result<u64, CliError> {
    let t = text.trim();
    let bad = || {
        error::usage(
            "bad_salt",
            format!("--salt {t}: a salt is 16 hex digits"),
            "give the `salt` an earlier `enth duel result` printed, or leave --salt out for a fresh one",
        )
    };
    if t.len() != 16 {
        return Err(bad());
    }
    u64::from_str_radix(t, 16).map_err(|_| bad())
}

/// A version-15 code refused: its own code and hint (exit 3).
fn from_result(e: &ResultError, what: &str) -> CliError {
    error::invalid(e.code(), format!("{what}: {e}"), e.hint())
}

/// A verification refused (exit 3) or a round that could not be scored (exit 4).
fn from_verify(e: &VerifyError) -> CliError {
    match e {
        VerifyError::Round(r) => game::from_round(r),
        VerifyError::Score(s) => game::from_score(s),
        _ => error::invalid(
            e.code(),
            format!("the answer cannot be checked: {e}"),
            e.hint(),
        ),
    }
}

fn assists(a: &CardAssists) -> Vec<String> {
    [
        (a.live_meter, "live meter"),
        (a.dim_unused, "dim unused"),
        (a.closest, "how close"),
        (a.revealed, "target revealed after passing"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, what)| (*what).to_owned())
    .collect()
}

fn reported(card: &Scorecard) -> DuelReportedOut {
    let f = card.fields();
    DuelReportedOut {
        submits: f.submits,
        seconds: f64::from(f.time_ds) / 10.0,
        hints: f.assists.hints,
        assists: assists(&f.assists),
    }
}

/// Symbols in a code's text (hyphens and spaces not counted).
fn symbols(text: &str) -> usize {
    text.chars().filter(char::is_ascii_alphanumeric).count()
}

fn result(a: &ResultArgs<'_>) -> Res {
    if !(1..=127).contains(&a.submits) {
        return Err(error::usage(
            "bad_submits",
            format!("--submits {}: a scorecard says 1 to 127 submits", a.submits),
            "the submits up to the compared one: the first passing submit, else the best",
        ));
    }
    if !(a.seconds.is_finite() && a.seconds >= 0.0) {
        return Err(error::usage(
            "bad_seconds",
            format!("--seconds {}: a time is 0 seconds or more", a.seconds),
            "the time to the pass, in seconds (reported, never checked)",
        ));
    }
    if a.hints > 15 {
        return Err(error::usage(
            "bad_hints",
            format!("--hints {}: a scorecard says 0 to 15 hints", a.hints),
            "15 means 15 or more",
        ));
    }
    let salt = match a.salt {
        Some(text) => parse_salt(text)?,
        None => fresh_salt(),
    };
    let code = game::decode(a.code)?;
    let (patch, mut warnings) = patchio::load(a.patch)?;
    let answer = AnswerCode::new(code, salt, &patch).map_err(|e| {
        let mut err = from_result(&e, "the patch cannot be put in an answer code");
        if e.code() == "unencodable" {
            err = err.in_file(patchio::show(a.patch));
        }
        err
    })?;
    // This build's current scorer set: the one its own rounds are judged with.
    let round = verify::prepare(&answer, ScorerSet::current()).map_err(|e| from_verify(&e))?;
    let report = score_round(&round, answer.answer()).map_err(|e| game::from_score(&e))?;
    let used = AssistsUsed {
        hints: a.hints,
        ..AssistsUsed::default()
    };
    let time_ms = (a.seconds * 1000.0).round().min(u64::MAX as f64) as u64;
    let card = Scorecard::new(
        code,
        &report,
        &round.challenge().scoring,
        a.submits,
        time_ms,
        &used,
        answer.commitment(),
    )
    .map_err(|e| from_result(&e, "the scorecard cannot be made"))?;
    warnings.extend(
        code.verify(&round.fingerprint(), EngineVersion::CURRENT)
            .unwrap_or_default()
            .iter()
            .map(|w| format!("{}: {}", w.code(), w.hint())),
    );
    let [first, second] = card.share_lines();
    let scorecard = card.encode();
    let text = answer.encode();
    let out = DuelResultOut {
        schema: ids::DUEL_RESULT.to_owned(),
        code: code.to_string(),
        patch: patchio::show(a.patch),
        score: game::score_out(&report),
        message: vec![first, second, scorecard.clone()],
        symbols: DuelSymbolsOut {
            scorecard: symbols(&scorecard),
            answer: answer.symbols(),
        },
        scorecard,
        answer: text.clone(),
        salt: format!("{salt:016x}"),
        reported: reported(&card),
        warnings: warnings.clone(),
        next: format!(
            "send the message now (it spoils nothing); once your friend has played, send the answer code; they check it with `enth duel verify '{text}'`"
        ),
    };
    let mut reply = Reply::new(&out).columns(&["code", "scorecard", "answer"]);
    for w in &warnings {
        reply = reply.note(w);
    }
    if !report.passed {
        reply = reply.note("the patch does not pass: the scorecard says \"not passed\"");
    }
    Ok(reply)
}

/// The text `verify` reads: the arguments joined by line breaks, `-` replaced by stdin.
fn joined(text: &[String], stdin: &str) -> String {
    text.iter()
        .map(|t| if t == "-" { stdin } else { t.as_str() })
        .collect::<Vec<&str>>()
        .join("\n")
}

fn found_out(found: &Found) -> DuelFoundOut {
    let kind = match found {
        Found::Challenge(_) => "challenge",
        Found::Scorecard(_) => "scorecard",
        Found::Answer(_) => "answer",
        Found::EarChallenge(_) => "ear_challenge",
        Found::EarResult(_) => "ear_result",
        _ => "other",
    };
    DuelFoundOut {
        kind: kind.to_owned(),
        code: found.encode(),
    }
}

fn card_out(card: &Scorecard) -> DuelCardOut {
    let f = card.fields();
    DuelCardOut {
        challenge: f.challenge.to_string(),
        lines: card.share_lines().to_vec(),
        sound: card.sound_percent(),
        combined: card.combined_percent(),
        passed: f.passed,
        engine: f.engine.to_string(),
        engine_here: EngineVersion::CURRENT.to_string(),
        scorers: f.scorers.index(),
        scorers_known: f.scorers.is_known(),
        reported: reported(card),
    }
}

fn verify_text(text: &[String], stdin: &str, mine: Option<&Path>, out: Option<&Path>) -> Res {
    let pasted = paste::paste(&joined(text, stdin));
    let refused: Vec<DuelRefusedOut> = pasted
        .refused
        .iter()
        .map(|r| DuelRefusedOut {
            text: r.text.clone(),
            error: r.error.code().to_owned(),
            hint: r.error.hint(),
        })
        .collect();
    if pasted.found.is_empty() {
        return Err(match pasted.refused.first() {
            Some(r) => error::invalid(
                r.error.code(),
                format!(
                    "'{}' looks like a code and does not read: {}",
                    r.text, r.error
                ),
                r.error.hint(),
            ),
            None => error::invalid(
                "no_code",
                "no code in the text",
                "paste the whole message your friend sent: their scorecard, their answer code, or both",
            ),
        });
    }
    let cards: Vec<&Scorecard> = pasted
        .found
        .iter()
        .filter_map(|f| match f {
            Found::Scorecard(c) => Some(c),
            _ => None,
        })
        .collect();
    let answers: Vec<&AnswerCode> = pasted
        .found
        .iter()
        .filter_map(|f| match f {
            Found::Answer(a) => Some(&**a),
            _ => None,
        })
        .collect();
    let mine = match mine {
        Some(path) => Some(patchio::load(path)?.0),
        None => None,
    };
    if out.is_some() && answers.len() != 1 {
        return Err(error::usage(
            "one_answer",
            format!(
                "--out writes one answer's patch; the text holds {}",
                answers.len()
            ),
            "paste one answer code with --out",
        ));
    }
    let mut checks = Vec::new();
    for answer in &answers {
        let card = cards
            .iter()
            .copied()
            .find(|c| c.challenge() == answer.challenge());
        let round =
            verify::prepare(answer, verify::scorer_set(card)).map_err(|e| from_verify(&e))?;
        let v = verify::verify_prepared(&round, answer, card).map_err(|e| from_verify(&e))?;
        let mine = match &mine {
            Some(patch) => {
                let canonical = AnswerCode::new(*answer.challenge(), 0, patch)
                    .map_or_else(|_| patch.clone(), |c| c.answer().clone());
                Some(game::score_out(
                    &score_round(&round, &canonical).map_err(|e| game::from_score(&e))?,
                ))
            }
            None => None,
        };
        checks.push(DuelCheckOut {
            challenge: answer.challenge().to_string(),
            symbols: answer.symbols(),
            ops: answer.ops(),
            theirs: game::score_out(&v.report),
            scorers: v.scorers.index(),
            claim: v.claim.map(|c| DuelClaimOut {
                claimed_sound: f64::from(c.claimed_sound_tenths) / 10.0,
                sound: f64::from(c.sound_tenths) / 10.0,
                claimed_combined: f64::from(c.claimed_combined_tenths) / 10.0,
                combined: f64::from(c.combined_tenths) / 10.0,
                claimed_passed: c.claimed_passed,
                passed: c.passed,
                as_reported: c.as_reported(),
                engine_differs: c.engine_differs(),
                scorers_differ: c.scorers_differ(),
                commitment_matches: c.commitment_matches,
            }),
            mine,
            warnings: v
                .warnings
                .iter()
                .map(|w| format!("{}: {}", w.code(), w.hint()))
                .collect(),
        });
    }
    let wrote = match (out, answers.first()) {
        (Some(path), Some(answer)) => {
            patchio::write(path, answer.answer())?;
            Some(patchio::show(path))
        }
        _ => None,
    };
    let next = if answers.is_empty() && !cards.is_empty() {
        "their scorecard is as reported by their app: swap answer codes once you have both played, then `enth duel verify` their answer".to_owned()
    } else if answers.is_empty() {
        "no answer code in the text: paste their answer code to check it".to_owned()
    } else {
        "the scores above were checked on your computer; times and assists are reported by their app, and enth can't check them".to_owned()
    };
    let out = DuelVerifyOut {
        schema: ids::DUEL_VERIFY.to_owned(),
        found: pasted.found.iter().map(found_out).collect(),
        refused,
        cards: cards.iter().map(|c| card_out(c)).collect(),
        checks,
        wrote,
        next,
    };
    Ok(Reply::new(&out).columns(&["found", "checks"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_salt_is_16_hex_digits_and_fresh_ones_differ() {
        assert_eq!(
            parse_salt("0123456789abcdef").unwrap(),
            0x0123_4567_89AB_CDEF
        );
        assert!(parse_salt("123").is_err());
        assert!(parse_salt("0123456789abcdeg").is_err());
        assert_ne!(fresh_salt(), fresh_salt());
    }
}
