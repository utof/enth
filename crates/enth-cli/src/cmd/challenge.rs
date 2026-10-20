//! **Owner: lane SKR, then CL.** `enth challenge`: challenges and codes over `enth_challenge` (lane SKR wrote `new`
//! and `show` for the round skeleton, freeze point F2r; lane CL adds `decode` and the rest).
//!
//! - `enth challenge new [--level N | --code X] [--seed S]`: a challenge from a level's preset (1–5) and a seed (a
//!   fresh 40-bit one when none is given, so the short code stays short), or the challenge a code names. Prints the
//!   code, the settings, the phrase, the target's fingerprint and the challenge JSON (`enth.challenge-new/1`).
//! - `enth challenge show <code> [--reveal] [--out T.patch.json]`: decodes the code, regenerates the target, checks a
//!   long code's fingerprint prefix and engine, and shows the target's summary: the phrase, the scoring, and which
//!   parameters were randomized (addresses only). No answer is shown unless `--reveal` (for debugging): then the
//!   randomized values beside the start's, and `--out` writes the target patch (`enth.challenge-show/1`).
//!
//! Both regenerate the target in parameter space only (no render), the way the game's worker does.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use enth_challenge::random::Generated;
use enth_challenge::spec::{LATEST_GENERATOR, PRESETS, SpecPreset};
use enth_challenge::{
    ChallengeCode, CodeForm, EngineVersion, PreparedRound, RandomSpec, phrases, prepare_round,
};

use super::{Reply, Res};
use crate::error::{self, CliError};
use crate::patchio;
use crate::schema::{
    ChallengeNewOut, ChallengeShowOut, PhraseNoteOut, PhraseOut, RevealOut, RevealedValue,
    ScoringOut, SpecOut, ids,
};

/// `enth challenge`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Sub,
}

/// `enth challenge`'s subcommands.
#[derive(Clone, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum Sub {
    /// A new challenge: a level's preset and a seed, or the challenge a code names.
    New {
        /// The level, 1-5 (code v1's presets; 1 = waveform and octave). Default 1.
        #[arg(long, value_name = "N", conflicts_with = "code")]
        level: Option<u8>,
        /// A challenge code to re-derive instead.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
        /// The seed (at most 2^40 - 1 keeps the short code; default: a fresh one from the clock).
        #[arg(long, value_name = "S", conflicts_with = "code")]
        seed: Option<u64>,
    },
    /// Decode a code and show its target's summary (no answers unless --reveal).
    Show {
        /// The challenge code (case, hyphens and spaces are ignored).
        code: String,
        /// Also show the target's randomized values (debugging: it gives the answer away).
        #[arg(long)]
        reveal: bool,
        /// With --reveal: write the target patch here.
        #[arg(long, value_name = "PATCH", requires = "reveal")]
        out: Option<PathBuf>,
    },
}

/// Run `enth challenge`.
pub fn run(args: &Args) -> Res {
    match &args.cmd {
        Sub::New { level, code, seed } => new(*level, code.as_deref(), *seed),
        Sub::Show { code, reveal, out } => show(code, *reveal, out.as_deref()),
    }
}

/// A code's text read, or an error with the code's own reason and hint (exit 3).
pub(crate) fn decode(text: &str) -> Result<ChallengeCode, CliError> {
    ChallengeCode::decode(text).map_err(|e| {
        error::invalid(
            e.code(),
            format!("the challenge code '{text}' does not read: {e}"),
            e.hint(),
        )
    })
}

/// The round a code names, regenerated and checked against the code (a long code's prefix refuses, another engine
/// warns). Returns the round and the warnings, as notes for humans.
pub(crate) fn regenerate(code: &ChallengeCode) -> Result<(PreparedRound, Vec<String>), CliError> {
    let round = prepare_round(code.spec()).map_err(|e| {
        error::invalid(
            e.code(),
            format!("the challenge's target cannot be regenerated: {e}"),
            e.hint(),
        )
    })?;
    let warnings = code
        .verify(&round.fingerprint(), EngineVersion::CURRENT)
        .map_err(|e| {
            error::invalid(
                e.code(),
                format!("the code does not match the target this build regenerates: {e}"),
                e.hint(),
            )
        })?
        .iter()
        .map(|w| format!("{}: {}", w.code(), w.hint()))
        .collect();
    Ok((round, warnings))
}

/// A fresh 40-bit seed from the wall clock (`SplitMix64`, Vigna's public-domain constants): the CLI is a shell, and a
/// short code holds 40 bits.
fn fresh_seed() -> u64 {
    let mut z = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) & ((1 << 40) - 1)
}

fn spec_out(s: &RandomSpec) -> SpecOut {
    SpecOut {
        generator: s.generator,
        seed: s.seed,
        difficulty: s.difficulty,
        amounts: s.amounts.to_vec(),
        musical: s.musical,
        template: s.template,
    }
}

fn form(code: &ChallengeCode) -> String {
    match code.form() {
        CodeForm::Short => "short",
        CodeForm::Long => "long",
    }
    .to_owned()
}

fn phrase_out(g: &Generated) -> PhraseOut {
    let p = g.phrase();
    PhraseOut {
        id: g.phrase.0,
        name: phrases::entry(g.phrase)
            .map(|e| e.name.to_owned())
            .unwrap_or_default(),
        length_ms: p.length_ms,
        notes: p
            .notes
            .iter()
            .map(|n| PhraseNoteOut {
                key: n.key,
                velocity: n.velocity,
                start_ms: n.start_ms,
                hold_ms: n.hold_ms,
            })
            .collect(),
    }
}

fn scoring_out(round: &PreparedRound) -> ScoringOut {
    let s = &round.challenge().scoring;
    ScoringOut {
        sound: s.sound.to_string(),
        structure: s.structure.to_string(),
        combined: s.combined.to_string(),
        time: s.time.to_string(),
        sound_weight: s.sound_weight,
        pass_mark: s.pass_mark,
    }
}

/// The code of a level's preset with `seed`: the short form, or the long one (with the target's prefix) for a seed
/// from 2^40 up.
fn code_for(level: u8, seed: u64) -> Result<ChallengeCode, CliError> {
    let preset = SpecPreset::by_id(level).ok_or_else(|| {
        error::invalid(
            "out_of_range",
            format!("there is no level {level}"),
            format!("levels go from 1 to {}", PRESETS.len()),
        )
    })?;
    let spec = preset.spec(LATEST_GENERATOR, seed);
    if let Some(code) = ChallengeCode::short(spec) {
        return Ok(code);
    }
    let round = prepare_round(&spec).map_err(|e| {
        error::invalid(e.code(), format!("the settings are refused: {e}"), e.hint())
    })?;
    round
        .challenge()
        .code(round.fingerprint().prefix())
        .ok_or_else(|| {
            error::invalid(
                "out_of_range",
                "the settings have no code",
                "seeds go up to 2^53 - 1; use one below 2^40 for a short code",
            )
        })
}

fn new(level: Option<u8>, code: Option<&str>, seed: Option<u64>) -> Res {
    let code = if let Some(text) = code {
        decode(text)?
    } else {
        code_for(level.unwrap_or(1), seed.unwrap_or_else(fresh_seed))?
    };
    let (round, warnings) = regenerate(&code)?;
    let g = round.generated();
    let fingerprint = round.fingerprint().to_hex();
    let text = code.to_string();
    let out = ChallengeNewOut {
        schema: ids::CHALLENGE_NEW.to_owned(),
        code: text.clone(),
        form: form(&code),
        level: g.level,
        spec: spec_out(code.spec()),
        phrase: phrase_out(g),
        fingerprint_prefix: fingerprint[..8].to_owned(),
        fingerprint,
        engine: EngineVersion::CURRENT.to_string(),
        challenge: serde_json::from_str(&round.challenge().to_json()).unwrap_or_default(),
        warnings: warnings.clone(),
        next: format!("enth challenge show {text}"),
    };
    let mut reply = Reply::new(&out).columns(&["code", "level", "fingerprint_prefix"]);
    for w in &warnings {
        reply = reply.note(w);
    }
    Ok(reply)
}

fn show(text: &str, reveal: bool, out: Option<&std::path::Path>) -> Res {
    let code = decode(text)?;
    let (round, warnings) = regenerate(&code)?;
    let g = round.generated();
    let fingerprint = round.fingerprint().to_hex();
    let revealed = if reveal {
        let values = g
            .params
            .iter()
            .map(|addr| RevealedValue {
                addr: addr.to_string(),
                start: round.start().value(addr),
                target: round.target().value(addr),
            })
            .collect();
        let wrote = match out {
            Some(path) => {
                patchio::write(path, round.target())?;
                Some(patchio::show(path))
            }
            None => None,
        };
        Some(RevealOut {
            values,
            target: serde_json::from_str(&round.target().to_json()).unwrap_or_default(),
            wrote,
        })
    } else {
        None
    };
    let out = ChallengeShowOut {
        schema: ids::CHALLENGE_SHOW.to_owned(),
        code: code.to_string(),
        form: form(&code),
        level: g.level,
        spec: spec_out(code.spec()),
        phrase: phrase_out(g),
        scoring: scoring_out(&round),
        randomized: g.params.iter().map(ToString::to_string).collect(),
        draws: g.randomized.len(),
        fingerprint_prefix: fingerprint[..8].to_owned(),
        fingerprint,
        engine: code.engine().map(|e| e.to_string()),
        warnings: warnings.clone(),
        reveal: revealed,
        hint: if reveal {
            "the target's values are shown: this round is spoiled for you".to_owned()
        } else {
            format!(
                "`enth compare <patch> --code {code}` scores a patch against it; --reveal shows the answer"
            )
        },
    };
    let mut reply = Reply::new(&out).columns(&["code", "level", "draws"]);
    for w in &warnings {
        reply = reply.note(w);
    }
    Ok(reply)
}
