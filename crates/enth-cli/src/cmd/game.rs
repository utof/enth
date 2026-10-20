//! **Owner: lane CL.** What the game commands share (`suggest`, `match`, and `compare` and `challenge` once lane SKR's
//! versions are polished): naming a challenge by code or file and regenerating its round, the worker pool, and the
//! translation of the round's, the coach's and the search's errors into `enth.error/1`.
//!
//! A challenge is named by `--code` (what the game shows; case, hyphens and spaces ignored) or `--challenge FILE`
//! (an `enth.challenge/1` document). Either way the round is regenerated from the spec with the same
//! `enth_challenge` functions the game's worker calls, so the CLI judges exactly what the plugin judges.

use std::path::{Path, PathBuf};

use enth_challenge::model::Phrase;
use enth_challenge::phrases::{self, HELD};
use enth_challenge::{
    Challenge, ChallengeCode, EngineVersion, PreparedRound, RoundError, ScoreError, ScoreReport,
    SpecError, prepare_challenge, prepare_round,
};
use enth_patch::Patch;
use enth_render::DEFAULT_SEED;
use enth_solver::coach::CoachError;
use enth_solver::search::SearchError;
use enth_solver::sensitivity::SensitivityError;

use crate::error::{self, CliError};
use crate::perf::{Perf, Source};
use crate::schema::ScoreOut;
use crate::{patchio, templates, wav};

/// The most workers `--workers` accepts: rayon's pool is cheap, but a typo should not ask for a million threads.
pub const MAX_WORKERS: usize = 256;

/// Which challenge: a code, or a file.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
pub struct WhichChallenge {
    /// The challenge's code, as the game shows it (case, hyphens and spaces are ignored).
    #[arg(long, value_name = "CODE", required_unless_present = "challenge")]
    pub code: Option<String>,
    /// A challenge file (`enth.challenge/1` JSON) instead of a code.
    #[arg(long, value_name = "FILE", conflicts_with = "code")]
    pub challenge: Option<PathBuf>,
}

/// A challenge's round, regenerated.
pub struct Resolved {
    /// The round: target, start, phrase, randomized parameters, scorers.
    pub round: PreparedRound,
    /// The challenge's code text, when it has one.
    pub code: Option<String>,
    /// Notes for humans (another engine than the code's).
    pub notes: Vec<String>,
}

/// Regenerate the round a code or a challenge file names.
pub fn resolve(which: &WhichChallenge) -> Result<Resolved, CliError> {
    match (&which.challenge, &which.code) {
        (Some(file), _) => from_file(file),
        (None, Some(text)) => from_code(text),
        (None, None) => Err(error::usage(
            "usage",
            "a challenge is needed: --code <code> or --challenge <file>",
            "`enth challenge new` makes one and prints its code",
        )),
    }
}

/// The round a code names, regenerated and checked against the code (a long code's fingerprint prefix refuses,
/// another engine warns).
pub fn from_code(text: &str) -> Result<Resolved, CliError> {
    let code = decode(text)?;
    let (round, notes) = regenerate(&code)?;
    Ok(Resolved {
        round,
        code: Some(code.to_string()),
        notes,
    })
}

/// The round a decoded code names, regenerated and checked against the code: a long code's fingerprint prefix
/// refuses (`target_mismatch`), another engine warns. Returns the round and the warnings, as notes for humans.
pub fn regenerate(code: &ChallengeCode) -> Result<(PreparedRound, Vec<String>), CliError> {
    let round = prepare_round(code.spec()).map_err(|e| from_round(&e))?;
    let notes = code
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
    Ok((round, notes))
}

fn from_file(file: &Path) -> Result<Resolved, CliError> {
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
    let round = prepare_challenge(challenge).map_err(|e| from_round(&e))?;
    let code = round
        .challenge()
        .code(round.fingerprint().prefix())
        .map(|c| c.to_string());
    Ok(Resolved {
        round,
        code,
        notes: Vec::new(),
    })
}

/// A code's text read, or an error with the code's own reason and hint (exit 3).
pub fn decode(text: &str) -> Result<ChallengeCode, CliError> {
    ChallengeCode::decode(text).map_err(|e| {
        error::invalid(
            e.code(),
            format!("the challenge code '{text}' does not read: {e}"),
            e.hint(),
        )
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

/// A round that could not be regenerated: exit 3 for a spec this build refuses, 4 when a scorer could not render.
pub fn from_round(e: &RoundError) -> CliError {
    if matches!(e, RoundError::SilentTarget) {
        // R1/S1 lane INT: any silent answer would score 100 %, so the round is refused (exit 3).
        return error::invalid(
            e.code(),
            format!("the challenge cannot be played: {e}, so there is nothing to recreate"),
            e.hint(),
        );
    }
    let message = format!("the challenge's target cannot be regenerated: {e}");
    match e.code() {
        "render_failed" | "analysis_failed" => error::failed(e.code(), message, e.hint()),
        code => error::invalid(code, message, e.hint()),
    }
}

/// Settings the generator refuses (exit 3).
pub fn from_spec_error(e: &SpecError) -> CliError {
    error::invalid(e.code(), format!("the settings are refused: {e}"), e.hint())
}

/// A patch that could not be scored (exit 4).
pub fn from_score(e: &ScoreError) -> CliError {
    error::failed(
        e.code(),
        format!("the patch could not be scored: {e}"),
        e.hint(),
    )
}

/// A coach that could not answer (exit 4): the target or the player's patch would not render or describe.
pub fn from_coach(e: &CoachError) -> CliError {
    use enth_solver::HintError;
    let (code, why) = match e {
        CoachError::Target(SensitivityError::Base(inner))
        | CoachError::Hint(HintError::Base(inner)) => {
            return error::from_eval(inner);
        }
        CoachError::Silent => (
            "silent_target",
            "the challenge's target is silent, so there is nothing to coach towards",
        ),
        CoachError::Target(inner) => (inner.code(), "the target could not be measured"),
        CoachError::Hint(inner) => (inner.code(), "the hint could not run"),
        _ => ("coach_failed", "the coach could not answer"),
    };
    error::failed(
        code,
        format!("{why}: {e}"),
        "this is a bug in the coach; report it with the code and the patch",
    )
}

/// A search that could not run: a free set the start lacks (exit 3), a bad setting (2), or a bug (4).
pub fn from_search(e: &SearchError) -> CliError {
    match e {
        SearchError::NoFreeParameters => error::invalid(e.code(), e.to_string(), e.hint()),
        SearchError::UnknownParam { addr } => {
            error::invalid(e.code(), e.to_string(), e.hint()).at(addr.clone())
        }
        SearchError::BadCfg(_) => error::usage(e.code(), e.to_string(), e.hint()),
        _ => error::failed(e.code(), e.to_string(), e.hint()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Scores
// ---------------------------------------------------------------------------------------------------------------

/// The percentages of a report, for output.
pub fn score_out(report: &ScoreReport) -> ScoreOut {
    ScoreOut {
        sound: report.sound.percent,
        structure: report.structure.percent,
        combined: report.combined.percent,
        pass_mark: report.pass_mark,
        passed: report.passed,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Workers
// ---------------------------------------------------------------------------------------------------------------

/// Check `--workers`: at least 1, at most [`MAX_WORKERS`].
pub fn check_workers(workers: Option<usize>) -> Result<Option<usize>, CliError> {
    match workers {
        Some(n) if n == 0 || n > MAX_WORKERS => Err(error::usage(
            "bad_workers",
            format!("--workers {n}: the number of worker threads must be 1 to {MAX_WORKERS}"),
            "leave --workers out to use every core; the result is the same for any number",
        )),
        other => Ok(other),
    }
}

/// Run `f` on a rayon pool of `workers` threads (the global pool, one thread per core, without). Every parallel
/// step of the game commands returns its results by index, so the output is identical for any number.
pub fn with_workers<T: Send>(
    workers: Option<usize>,
    f: impl FnOnce() -> T + Send,
) -> Result<T, CliError> {
    let Some(n) = check_workers(workers)? else {
        return Ok(f());
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .map_err(|e| {
            error::failed(
                "pool_failed",
                format!("cannot start {n} worker threads: {e}"),
                "try fewer workers, or none for the default",
            )
        })?;
    Ok(pool.install(f))
}

// ---------------------------------------------------------------------------------------------------------------
// Targets that are not challenges
// ---------------------------------------------------------------------------------------------------------------

/// The `init` template: what a search starts from when nothing else is named.
pub fn init_patch() -> Patch {
    templates::find("init")
        .unwrap_or_else(|| unreachable!("the init template exists"))
        .patch()
}

/// The held-note phrase (phrase 1): what a patch or WAV target is heard through when no phrase is named.
pub fn held() -> &'static Phrase {
    phrases::phrase(HELD).unwrap_or_else(|| unreachable!("the held phrase exists"))
}

/// A WAV ready to compare with: the mid signal, its rate, the performance that made it and the render seed.
pub struct Audio {
    /// The mean of the channels.
    pub mono: Vec<f32>,
    /// Frames per second.
    pub sample_rate: u32,
    /// The notes that made it (`--perf` over the `enth` chunk), with the clip's length.
    pub perf: Perf,
    /// The seed it was rendered with when the file says (oscillators in free phase mode start where the target's
    /// did), else the default.
    pub seed: u64,
}

/// Read a WAV target. Its performance comes from the flags, else from the `enth` chunk `enth render` writes; a WAV
/// that says neither is refused (`perf_required`), because comparing means playing the same notes. `who` names the
/// command for the hint.
pub fn load_audio(
    bytes: &[u8],
    name: &str,
    flag: Option<Perf>,
    who: &str,
) -> Result<Audio, CliError> {
    let d = wav::decode(bytes).map_err(|why| {
        error::failed(
            "unsupported_wav",
            format!("cannot read {name}: {why}"),
            format!("{who} reads 8-32 bit integer or 32-bit float WAVs at 8-192 kHz"),
        )
        .in_file(name)
    })?;
    if !(8_000..192_000).contains(&d.sample_rate) {
        return Err(error::failed(
            "unsupported_wav",
            format!(
                "{name} is at {} Hz; the analysis runs at 8 to 192 kHz",
                d.sample_rate
            ),
            "resample the file",
        )
        .in_file(name));
    }
    let dur_s = d.mono.len() as f64 / f64::from(d.sample_rate);
    let mut perf = if let Some(p) = flag {
        p
    } else if let Some(meta) = &d.meta {
        Perf::from_file(&meta.perf, Source::Wav)?
    } else {
        return Err(error::usage(
            "perf_required",
            format!(
                "{name} has no enth chunk, so the notes that made it are unknown, and {who} plays them on every candidate"
            ),
            "pass the performance: --perf note=A3,vel=100,hold=1.5s,dur=3s (or --note, --hold, --dur); files written by `enth render` carry theirs",
        )
        .in_file(name));
    };
    perf.dur_s = dur_s;
    let seed = d.meta.as_ref().map_or(DEFAULT_SEED, |m| m.seed);
    Ok(Audio {
        mono: d.mono,
        sample_rate: d.sample_rate,
        perf,
        seed,
    })
}
