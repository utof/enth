//! **Owner: lane SKR, then CL.** `enth challenge`: challenges and codes over `enth_challenge` (lane SKR wrote `new`
//! and `show` for the round skeleton, freeze point F2r; lane CL added `decode` and custom settings).
//!
//! - `enth challenge new [--level N | --difficulty D | --code X] [--seed S] [--amount CAT=PCT[,..]] [--musical PCT]
//!   [--template N] [--generator N]`: a challenge from a level's preset (1-5) or a difficulty, a seed (a fresh 40-bit
//!   one when none is given, so the short code stays short), and any overrides of the preset's settings, or the
//!   challenge a code names. Settings that are no preset travel in a long code (the engine and the target's
//!   fingerprint prefix ride along). Prints the code, the settings, the phrase, the target's fingerprint and the
//!   challenge JSON (`enth.challenge-new/2`).
//! - `enth challenge show <code> [--reveal] [--out T.patch.json]`: decodes the code, regenerates the target, checks a
//!   long code's fingerprint prefix and engine, and shows the target's summary: the phrase, the scoring, and which
//!   parameters were randomized (addresses only). No answer is shown unless `--reveal` (for debugging): then the
//!   randomized values beside the start's, and `--out` writes the target patch (`enth.challenge-show/2`).
//! - `enth challenge decode <code>`: what a code says, without regenerating anything: the form, the preset or the
//!   custom settings, the engine and the target prefix of a long code (`enth.challenge-decode/1`).
//!
//! `new` and `show` regenerate the target in parameter space only (no render), the way the game's worker does.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use enth_challenge::random::Generated;
use enth_challenge::spec::{LATEST_GENERATOR, PERCENT_MAX, PRESETS, SpecPreset};
use enth_challenge::{
    Category, ChallengeCode, CodeForm, EngineVersion, PreparedRound, RandomSpec, phrases,
    prepare_round,
};

use super::game;
use super::{Reply, Res};
use crate::error::{self, CliError, nearest};
use crate::patchio;
use crate::schema::{
    ChallengeDecodeOut, ChallengeNewOut, ChallengeShowOut, PhraseNoteOut, PhraseOut, PresetOut,
    RevealOut, RevealedValue, ScoringOut, SpecOut, ids,
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
    /// A new challenge: a level's preset or a difficulty, a seed and any overrides; or the challenge a code names.
    New {
        /// The level, 1-5 (code v1's presets; 1 = waveform and octave). Default 1.
        #[arg(long, value_name = "N", conflicts_with_all = ["code", "difficulty"])]
        level: Option<u8>,
        /// A challenge code to re-derive instead.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
        /// The seed (at most 2^40 - 1 keeps the short code; default: a fresh one from the clock).
        #[arg(long, value_name = "S", conflicts_with = "code")]
        seed: Option<u64>,
        /// Any difficulty, 0-100, instead of a level (levels are 0, 25, 50, 75 and 100). Any other value is a
        /// custom setting, so the code is a long one.
        #[arg(long, value_name = "D", conflicts_with_all = ["code", "level"])]
        difficulty: Option<u8>,
        /// An amount for a category, 0-100, overriding what the difficulty gives: `filter=80` or
        /// `oscillators=100,effects=0` (the categories: oscillators, filter, envelopes, modulation, effects,
        /// structure). A custom setting: a long code.
        #[arg(
            long,
            value_name = "CAT=PCT",
            value_delimiter = ',',
            conflicts_with = "code"
        )]
        amount: Vec<String>,
        /// 0-100: 100 draws from the musical priors only, 0 uniformly (the presets use 100). A custom setting when
        /// not 100.
        #[arg(long, value_name = "PCT", conflicts_with = "code")]
        musical: Option<u8>,
        /// The start template, 0 = the starter superset (the only one in this build).
        #[arg(long, value_name = "N", conflicts_with = "code")]
        template: Option<u8>,
        /// The generator version (this build runs 1; a version's targets never change).
        #[arg(long, value_name = "N", conflicts_with = "code")]
        generator: Option<u16>,
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
    /// Read a code without regenerating its target: the form, the settings, the engine.
    Decode {
        /// The challenge code (case, hyphens and spaces are ignored).
        code: String,
    },
}

/// What `challenge new` was asked for.
struct NewArgs<'a> {
    level: Option<u8>,
    code: Option<&'a str>,
    seed: Option<u64>,
    difficulty: Option<u8>,
    amount: &'a [String],
    musical: Option<u8>,
    template: Option<u8>,
    generator: Option<u16>,
}

/// Run `enth challenge`.
pub fn run(args: &Args) -> Res {
    match &args.cmd {
        Sub::New {
            level,
            code,
            seed,
            difficulty,
            amount,
            musical,
            template,
            generator,
        } => new(&NewArgs {
            level: *level,
            code: code.as_deref(),
            seed: *seed,
            difficulty: *difficulty,
            amount,
            musical: *musical,
            template: *template,
            generator: *generator,
        }),
        Sub::Show { code, reveal, out } => show(code, *reveal, out.as_deref()),
        Sub::Decode { code } => decode(code),
    }
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

/// The category's name in `--amount` and in the output.
const fn category_name(c: Category) -> &'static str {
    match c {
        Category::Oscillators => "oscillators",
        Category::Filter => "filter",
        Category::Envelopes => "envelopes",
        Category::Modulation => "modulation",
        Category::Effects => "effects",
        Category::Structure => "structure",
    }
}

/// `--amount filter=80,effects=0` read into the spec's per-category amounts.
fn apply_amounts(spec: &mut RandomSpec, items: &[String]) -> Result<(), CliError> {
    for item in items.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
        let names: Vec<&str> = Category::ALL.iter().map(|c| category_name(*c)).collect();
        let Some((name, pct)) = item.split_once('=') else {
            return Err(error::usage(
                "bad_amount",
                format!("--amount '{item}': an amount is CATEGORY=PERCENT"),
                format!(
                    "for example --amount filter=80; the categories are {}",
                    names.join(", ")
                ),
            ));
        };
        let (name, pct) = (name.trim().to_lowercase(), pct.trim().trim_end_matches('%'));
        let Some(category) = Category::ALL.iter().find(|c| category_name(**c) == name) else {
            return Err(error::invalid(
                "unknown_category",
                format!("'{name}' is not a category"),
                format!("the categories are {}", names.join(", ")),
            )
            .suggest(nearest(&name, names.iter().copied()))
            .known(names.iter().map(|n| (*n).to_owned()).collect()));
        };
        let Ok(pct) = pct.parse::<u8>() else {
            return Err(error::invalid(
                "out_of_range",
                format!("--amount {name}={pct}: an amount is a whole percentage"),
                format!("0 to {PERCENT_MAX}"),
            ));
        };
        spec.amounts[category.index()] = Some(pct);
    }
    Ok(())
}

/// The settings `challenge new` was asked for: a level's preset (default level 1) or a difficulty on the first
/// preset's footing, then the overrides.
fn spec_for(a: &NewArgs<'_>) -> Result<RandomSpec, CliError> {
    let seed = a.seed.unwrap_or_else(fresh_seed);
    let generator = a.generator.unwrap_or(LATEST_GENERATOR);
    let mut spec = if let Some(d) = a.difficulty {
        let mut spec = PRESETS[0].spec(generator, seed);
        spec.difficulty = d;
        spec
    } else {
        let level = a.level.unwrap_or(1);
        let preset = SpecPreset::by_id(level).ok_or_else(|| {
            error::invalid(
                "out_of_range",
                format!("there is no level {level}"),
                format!("levels go from 1 to {}", PRESETS.len()),
            )
        })?;
        preset.spec(generator, seed)
    };
    apply_amounts(&mut spec, a.amount)?;
    if let Some(m) = a.musical {
        spec.musical = m;
    }
    if let Some(t) = a.template {
        spec.template = t;
    }
    spec.validate().map_err(|e| game::from_spec_error(&e))?;
    Ok(spec)
}

/// The code of `spec`: the short form when it is a preset with a short seed, else the long one (with the target's
/// fingerprint prefix).
fn code_for(spec: RandomSpec) -> Result<ChallengeCode, CliError> {
    if let Some(code) = ChallengeCode::short(spec) {
        return Ok(code);
    }
    let round = prepare_round(&spec).map_err(|e| game::from_round(&e))?;
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

fn preset_name(spec: &RandomSpec) -> Option<String> {
    spec.preset().map(|p| p.name.to_owned())
}

fn new(a: &NewArgs<'_>) -> Res {
    let code = if let Some(text) = a.code {
        game::decode(text)?
    } else {
        code_for(spec_for(a)?)?
    };
    let (round, warnings) = game::regenerate(&code)?;
    let g = round.generated();
    let fingerprint = round.fingerprint().to_hex();
    let text = code.to_string();
    let out = ChallengeNewOut {
        schema: ids::CHALLENGE_NEW.to_owned(),
        code: text.clone(),
        form: form(&code),
        level: g.level,
        spec: spec_out(code.spec()),
        preset: preset_name(code.spec()),
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

fn decode(text: &str) -> Res {
    let code = game::decode(text)?;
    let spec = code.spec();
    let canonical = code.to_string();
    let out = ChallengeDecodeOut {
        schema: ids::CHALLENGE_DECODE.to_owned(),
        code: canonical.clone(),
        form: form(&code),
        preset: spec.preset().map(|p| PresetOut {
            id: p.id,
            name: p.name.to_owned(),
        }),
        spec: spec_out(spec),
        categories: Category::ALL
            .iter()
            .map(|c| category_name(*c).to_owned())
            .collect(),
        engine: code.engine().map(|e| e.to_string()),
        engine_current: code.engine().map(|e| e == EngineVersion::CURRENT),
        target_prefix: code.target().map(|t| format!("{:08x}", t.0)),
        hint: format!(
            "`enth challenge show {canonical}` regenerates the target and summarises it; `enth challenge new --code {canonical}` prints the challenge"
        ),
    };
    Ok(Reply::new(&out).columns(&["code", "form"]))
}

fn show(text: &str, reveal: bool, out: Option<&std::path::Path>) -> Res {
    let code = game::decode(text)?;
    let (round, warnings) = game::regenerate(&code)?;
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
        preset: preset_name(code.spec()),
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
