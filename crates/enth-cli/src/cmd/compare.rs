//! **Owner: lane SKR, then CL.** `enth compare <patch>` against a target of one of three kinds. Lane SKR wrote the
//! challenge form for the round skeleton (F2r); lane CL added the reference forms and `enth.compare/2`.
//!
//! - `--code CODE` or `--challenge FILE`: a challenge's target, scored with the scorers the challenge names through
//!   `enth_challenge::score_round`, the same function the game's worker and the golden challenges call. The report
//!   holds the components' direction and closeness, as the game's results drawer shows them, but not the target's
//!   readings (the game never shows those during a round); `--reveal` adds them. Exit 1 when the patch does not pass.
//! - `--target REF.patch.json [--phrase N]`: a reference patch, heard through a phrase of the table (default: the held
//!   note). It is scored by the game's own scorers (`sound/1`, `structure/1`, `combined/1`) on a hand-made round: the
//!   reference stands for the target, the parameters it moved off their defaults (and that the phrase lets you hear)
//!   stand for the randomized ones, and the pass mark is the game's (90). Exit 1 when it does not pass.
//! - `--target REF.wav [--perf ...]`: a recording. There is no patch behind it, so the game's scores (which scale by
//!   re-rendering the target with nuisance changes) do not exist; the answer is the metric distance and the
//!   descriptors that differ. The notes that made it come from `--perf` or the WAV's `enth` chunk. Exit 0.
//!
//! Every form reports `distance`, the metric `enth match` minimises, so a `match` and a `compare` of the same target
//! agree; the two reference forms add `differences`: the descriptors that differ by more than the game tolerates in
//! them, in words.

use std::path::PathBuf;

use enth_analysis::Feature;
use enth_analysis::descriptors::{Describer, Descriptors, NoteTiming};
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_challenge::patch_round::{Compared, prepare_patch_round};
use enth_challenge::phrases::{self, TABLE};
use enth_challenge::v2::InlinePatch;
use enth_challenge::{
    Challenge, Phrase, PreparedRound, RoundError, ScoreReport, ScoringSpec, Start, Target,
    score_round,
};
use enth_patch::Patch;
use enth_render::{Performance, RenderSpec, render};
use enth_solver::coach::Aspect;
use enth_solver::search::{FAILED, MetricTarget};
use enth_solver::sensitivity::{FEATURES, domain_value, tolerance_bounds};
use serde_json::Value;

use super::common::reading;
use super::game::{self, Audio, Resolved, WhichChallenge};
use super::{Reply, Res};
use crate::error::{self, CliError, EXIT_CHECK_FAILED, nearest};
use crate::out::round4;
use crate::patchio;
use crate::perf::{Perf, PerfArgs};
use crate::schema::{CompareOut, CompareTargetOut, DifferenceOut, PhraseRefOut, ids};

/// The metric `distance` is in.
const METRIC: &str = "logmel-l1/1";
/// The most differences listed.
const MAX_DIFFERENCES: usize = 8;
/// A difference under this many tolerance units is not listed.
const MIN_UNITS: f64 = 0.5;
/// A difference of this many tolerance units or more is `high`.
const HIGH_UNITS: f64 = 4.0;
/// A difference of this many tolerance units or more is `medium`.
const MEDIUM_UNITS: f64 = 2.0;

/// What to compare with: exactly one.
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::Args)]
#[group(required = true, multiple = false)]
pub struct Reference {
    /// The challenge's code, as the game shows it (case, hyphens and spaces are ignored).
    #[arg(long, value_name = "CODE")]
    pub code: Option<String>,
    /// A challenge file (`enth.challenge/1` JSON) instead of a code.
    #[arg(long, value_name = "FILE")]
    pub challenge: Option<PathBuf>,
    /// A reference: a patch file, or a WAV (its notes from --perf or the file's `enth` chunk).
    #[arg(long, value_name = "PATCH|WAV")]
    pub target: Option<PathBuf>,
}

/// `enth compare`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// The patch to score (*.patch.json).
    pub patch: PathBuf,
    #[command(flatten)]
    pub reference: Reference,
    /// For a reference patch: the phrase both are heard through, by id or name (`held`, `octave_pair`,
    /// `legato_pair`). Default: held.
    #[arg(long, value_name = "ID|NAME")]
    pub phrase: Option<String>,
    /// For a challenge: also show the target's readings in the report's components (the game never does during a
    /// round: it gives the answer away).
    #[arg(long)]
    pub reveal: bool,
    #[command(flatten)]
    pub perf: PerfArgs,
}

/// Run `enth compare`.
pub fn run(args: &Args, stdin: &str) -> Res {
    let r = &args.reference;
    match (&r.code, &r.challenge, &r.target) {
        (_, _, Some(target)) => reference(args, target, stdin),
        (code, challenge, None) => challenge_form(
            args,
            &WhichChallenge {
                code: code.clone(),
                challenge: challenge.clone(),
            },
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A challenge
// ---------------------------------------------------------------------------------------------------------------

fn challenge_form(args: &Args, which: &WhichChallenge) -> Res {
    if args.phrase.is_some() {
        return Err(error::usage(
            "phrase_fixed",
            "--phrase names the phrase of a reference patch; a challenge has its own",
            "drop --phrase, or compare with --target <patch>",
        ));
    }
    if !args.perf.is_empty() {
        return Err(perf_fixed());
    }
    let (patch, mut notes) = patchio::load(&args.patch)?;
    let Resolved {
        round,
        code,
        notes: round_notes,
    } = game::resolve(which)?;
    notes.extend(round_notes);
    let report = score_round(&round, &patch).map_err(|e| game::from_score(&e))?;
    let phrase_perf = round.phrase().performance();
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    let distance = distance_to(
        &MetricTarget::from_patch(round.target(), &phrase_perf, &spec)
            .map_err(|e| error::from_eval(&e))?,
        &patch,
    )?;
    let kind = if which.challenge.is_some() {
        "challenge"
    } else {
        "code"
    };
    let source = match (&which.challenge, &which.code) {
        (Some(file), _) => patchio::show(file),
        (None, Some(text)) => text.clone(),
        (None, None) => String::new(),
    };
    let target = CompareTargetOut {
        kind: kind.to_owned(),
        source,
        code,
        fingerprint: Some(round.fingerprint().to_hex()),
        phrase: Some(phrase_ref(&round)),
        perf: None,
    };
    let mut report_json = serde_json::to_value(&report).unwrap_or_default();
    if !args.reveal {
        hide_readings(&mut report_json);
    }
    let hint = if report.passed {
        "it passes: sound % is at or above the pass mark".to_owned()
    } else {
        "not yet: the report's components say which way and how far each aspect is off; `enth suggest` gives a graded hint, `enth match` searches"
            .to_owned()
    };
    Ok(finish(Parts {
        mode: "challenge",
        patch_path: &args.patch,
        target,
        distance,
        report: Some(report),
        report_json: Some(report_json),
        differences: Vec::new(),
        hint,
        notes,
    }))
}

/// A phrase's id and name.
fn phrase_ref(round: &PreparedRound) -> PhraseRefOut {
    let id = round.challenge().phrase;
    PhraseRefOut {
        id: id.0,
        name: phrases::entry(id)
            .map(|e| e.name.to_owned())
            .unwrap_or_default(),
    }
}

/// The report without the target's and the answer's readings: what the game shows during a round is each
/// component's direction and closeness.
fn hide_readings(report: &mut Value) {
    let Some(components) = report
        .pointer_mut("/sound/components")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for c in components {
        if let Some(m) = c.as_object_mut() {
            m.remove("target");
            m.remove("answer");
        }
    }
}

fn perf_fixed() -> CliError {
    error::usage(
        "perf_fixed",
        "a challenge or a reference patch fixes the performance: its phrase is what both are played through",
        "drop --perf/--note/--vel/--hold/--dur, or name a WAV with --target",
    )
}

// ---------------------------------------------------------------------------------------------------------------
// A reference
// ---------------------------------------------------------------------------------------------------------------

fn reference(args: &Args, target: &std::path::Path, stdin: &str) -> Res {
    let bytes = patchio::read_bytes(target)?;
    let flag = Perf::from_args(&args.perf, stdin)?;
    if patchio::is_wav(&bytes) {
        if args.phrase.is_some() {
            return Err(error::usage(
                "phrase_fixed",
                "--phrase names the phrase of a reference patch; a WAV is played with its own notes",
                "pass the notes with --perf, or drop --phrase",
            ));
        }
        wav_reference(args, target, &bytes, flag)
    } else {
        if flag.is_some() {
            return Err(perf_fixed());
        }
        patch_reference(args, target, &bytes)
    }
}

/// `--phrase`: an id or a name of the table; the held note by default.
fn pick_phrase(arg: Option<&str>) -> Result<&'static phrases::Entry, CliError> {
    let Some(text) = arg.map(str::trim) else {
        return Ok(&TABLE[0]);
    };
    let found = text.parse::<u16>().map_or_else(
        |_| TABLE.iter().find(|e| e.name == text),
        |n| TABLE.iter().find(|e| e.id.0 == n),
    );
    found.ok_or_else(|| {
        let names: Vec<&str> = TABLE.iter().map(|e| e.name).collect();
        error::usage(
            "unknown_phrase",
            format!("'{text}' is not a phrase"),
            format!(
                "the phrases are {}",
                TABLE
                    .iter()
                    .map(|e| format!("{} ({})", e.id.0, e.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .suggest(nearest(text, names.iter().copied()))
        .known(names.iter().map(|n| (*n).to_owned()).collect())
    })
}

fn patch_reference(args: &Args, target_path: &std::path::Path, bytes: &[u8]) -> Res {
    let name = patchio::show(target_path);
    let entry = pick_phrase(args.phrase.as_deref())?;
    let (patch, mut notes) = patchio::load(&args.patch)?;
    let (reference, ref_notes) = patchio::parse(bytes, &name)?;
    notes.extend(ref_notes);

    // The hand-made round (`enth_challenge::patch_round`, lane CV2): the reference is the target, the parameters it
    // moved off their defaults and that the phrase lets you hear are the compared ones, and the starter superset is
    // the start.
    let challenge = Challenge::new(
        Target::Patch(InlinePatch::new(reference.clone())),
        Start::Template { template: 0 },
        entry.id,
        ScoringSpec::recreate(0),
    );
    let round = prepare_patch_round(challenge, Compared::OffDefaults).map_err(|e| match e {
        // A scorer that cannot prepare on the reference is reported as scoring it always was.
        RoundError::Score(e) => game::from_score(&e),
        e => error::failed(
            "start_failed",
            format!("the round on the reference cannot be prepared: {e}"),
            "this is a bug; report it",
        ),
    })?;
    let report = score_round(&round, &patch).map_err(|e| game::from_score(&e))?;

    let perf = entry.phrase.performance();
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    let distance = distance_to(
        &MetricTarget::from_patch(&reference, &perf, &spec).map_err(|e| error::from_eval(&e))?,
        &patch,
    )?;
    let differences = {
        let ours = describe_phrase(&patch, &entry.phrase)?;
        let theirs = describe_phrase(&reference, &entry.phrase)?;
        differences(&ours, &theirs)
    };
    let target = CompareTargetOut {
        kind: "patch".to_owned(),
        source: name,
        code: None,
        fingerprint: None,
        phrase: Some(PhraseRefOut {
            id: entry.id.0,
            name: entry.name.to_owned(),
        }),
        perf: None,
    };
    let report_json = serde_json::to_value(&report).unwrap_or_default();
    let hint = if report.passed {
        "it passes: sound % is at or above the pass mark (90)".to_owned()
    } else {
        "not yet: `differences` says which way each descriptor is off; `enth match --target <ref> --start <patch>` searches for the parameters"
            .to_owned()
    };
    Ok(finish(Parts {
        mode: "patch",
        patch_path: &args.patch,
        target,
        distance,
        report: Some(report),
        report_json: Some(report_json),
        differences,
        hint,
        notes,
    }))
}

fn wav_reference(
    args: &Args,
    target_path: &std::path::Path,
    bytes: &[u8],
    flag: Option<Perf>,
) -> Res {
    let name = patchio::show(target_path);
    let (patch, notes) = patchio::load(&args.patch)?;
    let Audio {
        mono,
        sample_rate,
        perf,
        seed,
    } = game::load_audio(bytes, &name, flag, "a comparison")?;
    let performance = perf.to_render();
    let spec = RenderSpec::new(sample_rate, seed);
    let distance = distance_to(
        &MetricTarget::from_audio(&mono, &performance, &spec),
        &patch,
    )?;

    let sr = f64::from(sample_rate);
    let theirs = Describer::new(sr)
        .describe(&mono, perf.timing())
        .map_err(|e| error::from_analysis(&e))?;
    let ours = describe_render(&patch, &performance, &spec, perf.timing())?;
    let differences = differences(&ours, &theirs);

    let target = CompareTargetOut {
        kind: "wav".to_owned(),
        source: name,
        code: None,
        fingerprint: None,
        phrase: None,
        perf: Some(perf.to_out()),
    };
    let hint = if differences.is_empty() {
        "no descriptor differs by more than the game tolerates; `distance` says how close the whole sound is (0 is identical)"
            .to_owned()
    } else {
        "`differences` says which way each descriptor is off; `enth match --target <wav> --start <patch>` searches for the parameters"
            .to_owned()
    };
    Ok(finish(Parts {
        mode: "wav",
        patch_path: &args.patch,
        target,
        distance,
        report: None,
        report_json: None,
        differences,
        hint,
        notes,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Measuring
// ---------------------------------------------------------------------------------------------------------------

/// Metric v1 between the target and `patch` played with the target's performance.
fn distance_to(metric: &MetricTarget, patch: &Patch) -> Result<f64, CliError> {
    let d = metric.worker().distance(patch);
    if d >= FAILED {
        return Err(error::failed(
            "render_failed",
            "the patch could not be rendered, or its render has no sound to measure",
            "check it with `enth render`",
        ));
    }
    Ok(d)
}

/// `patch` rendered with `perf` and described with the note's timing.
fn describe_render(
    patch: &Patch,
    perf: &Performance,
    spec: &RenderSpec,
    timing: NoteTiming,
) -> Result<Descriptors, CliError> {
    let out = render(patch, perf, spec).map_err(|e| error::from_render(&e))?;
    Describer::new(f64::from(out.sample_rate))
        .describe(&out.mid(), timing)
        .map_err(|e| error::from_analysis(&e))
}

/// `patch` played through `phrase` at the scoring rate and seed, described over the phrase's first note (from its
/// start to the next note-on), as `sound/1` reads its components.
fn describe_phrase(patch: &Patch, phrase: &Phrase) -> Result<Descriptors, CliError> {
    let spec = RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED);
    let out = render(patch, &phrase.performance(), &spec).map_err(|e| error::from_render(&e))?;
    let mid = out.mid();
    let sr = f64::from(SCORING_SAMPLE_RATE);
    let Some(first) = phrase.notes.first() else {
        return Describer::new(sr)
            .describe(&mid, NoteTiming::held(0.0))
            .map_err(|e| error::from_analysis(&e));
    };
    let end = phrase
        .notes
        .iter()
        .skip(1)
        .map(|n| n.start_ms)
        .filter(|ms| *ms > first.start_ms)
        .min()
        .map_or(mid.len(), |ms| {
            ((f64::from(ms) / 1000.0 * sr) as usize).min(mid.len())
        });
    let on_s = f64::from(first.start_ms) / 1000.0;
    let off_s = f64::from(first.start_ms + first.hold_ms) / 1000.0;
    Describer::new(sr)
        .describe(
            &mid[..end],
            NoteTiming {
                on_s,
                off_s: Some(off_s),
            },
        )
        .map_err(|e| error::from_analysis(&e))
}

// ---------------------------------------------------------------------------------------------------------------
// Differences
// ---------------------------------------------------------------------------------------------------------------

/// The aspect's words for a patch that reads `direction` relative to the reference.
fn perceptual(aspect: Aspect, direction: &str) -> String {
    let (higher, lower, noun) = match aspect {
        Aspect::Pitch => (
            "ours is higher in pitch",
            "ours is lower in pitch",
            "a pitch",
        ),
        Aspect::Brightness => ("ours is brighter", "ours is darker", "brightness"),
        Aspect::Attack => (
            "ours attacks more slowly",
            "ours attacks faster",
            "an attack",
        ),
        Aspect::Decay => ("ours decays more slowly", "ours decays faster", "a decay"),
        Aspect::Sustain => (
            "ours holds a higher level",
            "ours holds a lower level",
            "a sustain level",
        ),
        Aspect::Release => (
            "ours rings on longer after the note ends",
            "ours stops sooner after the note ends",
            "a release",
        ),
        Aspect::Noisiness => ("ours is noisier", "ours is cleaner", "noise"),
        Aspect::Beating => (
            "ours beats faster (more chorused)",
            "ours beats slower",
            "detune beating",
        ),
        Aspect::Tremolo => ("ours wobbles faster", "ours wobbles slower", "tremolo"),
        Aspect::Sweep => (
            "ours sweeps its filter more",
            "ours sweeps its filter less",
            "a filter sweep",
        ),
        _ => ("ours reads higher", "ours reads lower", "this"),
    };
    match direction {
        "higher" => format!("{higher} than the target"),
        "lower" => format!("{lower} than the target"),
        "missing" => format!("the target has {noun} that ours lacks"),
        _ => format!("ours has {noun} that the target lacks"),
    }
}

/// The descriptors of the game's list ([`FEATURES`]) that differ between the patch's render and the reference's by
/// at least half a tolerance unit ([`tolerance_bounds`]`.0` in the descriptor's compare domain), or that one has and the
/// other lacks; the most different first, one per aspect.
fn differences(ours: &Descriptors, theirs: &Descriptors) -> Vec<DifferenceOut> {
    let mut found: Vec<(f64, DifferenceOut)> = Vec::new();
    let mut seen: Vec<Aspect> = Vec::new();
    for feature in FEATURES {
        let Some(aspect) = Aspect::from_feature(feature) else {
            continue;
        };
        if seen.contains(&aspect) {
            continue;
        }
        let (a, t) = (ours.get(feature), theirs.get(feature));
        let (direction, units) = match (a, t) {
            (None, None) => continue,
            (None, Some(_)) => ("missing", None),
            (Some(_), None) => ("extra", None),
            (Some(_), Some(_)) => {
                let (Some(da), Some(dt)) =
                    (domain_value(feature, ours), domain_value(feature, theirs))
                else {
                    continue;
                };
                let units = (da - dt).abs() / tolerance_bounds(feature).0;
                if units < MIN_UNITS {
                    continue;
                }
                (if da > dt { "higher" } else { "lower" }, Some(units))
            }
        };
        seen.push(aspect);
        let severity = match units {
            None => "high",
            Some(u) if u >= HIGH_UNITS => "high",
            Some(u) if u >= MEDIUM_UNITS => "medium",
            Some(_) => "low",
        };
        found.push((
            units.unwrap_or(f64::INFINITY),
            DifferenceOut {
                feature: feature_path(feature),
                aspect: aspect.code().to_owned(),
                direction: direction.to_owned(),
                ours: a.map(|v| reading(feature, v)),
                target: t.map(|v| reading(feature, v)),
                units: units.map(round4),
                severity: severity.to_owned(),
                perceptual: perceptual(aspect, direction),
            },
        ));
    }
    found.sort_by(|x, y| y.0.total_cmp(&x.0));
    found
        .into_iter()
        .take(MAX_DIFFERENCES)
        .map(|(_, d)| d)
        .collect()
}

fn feature_path(feature: Feature) -> String {
    feature.path().to_owned()
}

// ---------------------------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------------------------

/// What every form hands to [`finish`].
struct Parts<'a> {
    mode: &'static str,
    patch_path: &'a std::path::Path,
    target: CompareTargetOut,
    distance: f64,
    /// The game's report, for a challenge or a reference patch.
    report: Option<ScoreReport>,
    /// The report as printed.
    report_json: Option<Value>,
    differences: Vec<DifferenceOut>,
    hint: String,
    notes: Vec<String>,
}

fn finish(p: Parts<'_>) -> Reply {
    let r = p.report.as_ref();
    let out = CompareOut {
        schema: ids::COMPARE.to_owned(),
        mode: p.mode.to_owned(),
        patch: patchio::show(p.patch_path),
        target: p.target,
        distance: p.distance,
        metric: METRIC.to_owned(),
        sound: r.map(|r| r.sound.percent),
        structure: r.map(|r| r.structure.percent),
        combined: r.map(|r| r.combined.percent),
        pass_mark: r.map(|r| r.pass_mark),
        passed: r.map(|r| r.passed),
        margin: r.map(ScoreReport::margin),
        scorers: r.map_or_else(Vec::new, |r| {
            vec![
                r.sound.scorer.to_string(),
                r.structure.scorer.to_string(),
                r.combined.scorer.to_string(),
            ]
        }),
        report: p.report_json,
        differences: p.differences,
        hint: p.hint,
    };
    let mut reply = Reply::new(&out)
        .columns(&["sound", "structure", "combined", "distance", "passed"])
        .text(summary(&out));
    if r.is_some_and(|r| !r.passed) {
        reply = reply.code(EXIT_CHECK_FAILED);
    }
    for n in &p.notes {
        reply = reply.note(n);
    }
    reply
}

/// What a terminal prints.
fn summary(o: &CompareOut) -> String {
    use std::fmt::Write as _;
    let mut s = format!(
        "compare {} with {} {}\n",
        o.patch, o.target.kind, o.target.source
    );
    if let (Some(sound), Some(structure), Some(combined), Some(mark), Some(passed)) =
        (o.sound, o.structure, o.combined, o.pass_mark, o.passed)
    {
        let _ = writeln!(
            s,
            "  sound {}%, structure {}%, combined {}% ({} the pass mark {mark})",
            round4(sound),
            round4(structure),
            round4(combined),
            if passed { "passes" } else { "below" },
        );
    }
    let _ = writeln!(s, "  distance {} ({})", round4(o.distance), o.metric);
    for d in &o.differences {
        let _ = writeln!(s, "  {:<6} {}: {}", d.severity, d.aspect, d.perceptual);
    }
    let _ = writeln!(s, "{}", o.hint);
    s
}
