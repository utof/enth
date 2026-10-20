//! The GR1 numbers: [`build`] turns the merged rounds (and the optional baseline and survey) into a [`Report`], the
//! value both the table and `--json` print. Everything here is a pure function of its inputs: the bootstrap is seeded.
//!
//! What the numbers count (the same words as `docs/gr1.md`):
//!
//! - **ended** rounds: one per round after the restored-round merge, not broken and with a readable result;
//! - **played** rounds: ended rounds with a scored submit or a reveal (the rest are **skipped**: the player moved on
//!   without trying);
//! - **passed**: the result's outcome is `passed`; **assisted**: a hint, the live meter or the reveal was used;
//! - **time to pass**: active time to the first passing submit;
//! - the **recorded score** is the first submit's report, which is the one the vote and the blind rating are about.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::baseline::{self, Cell, Comparison, Group};
use super::logs::{Dispute, Loaded, Problem, Round, Status};
use super::schema::{Agreement, Outcome, Severity};
use super::stats::{self, mean, median, wilson};
use super::survey::{Answer, WantMore};

/// The report's schema.
pub const REPORT_SCHEMA: &str = "enth.gr1-report/1";

/// The proposed reading of the GR1 gate (docs/phase-2.md "The GR1 and GS1 checks"; report 05): a proposal until the
/// owner settles it, so the report prints the thresholds it used and the JSON carries them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Thresholds {
    /// Rounds each player plays.
    pub rounds_each: usize,
    /// "About right" share over all votes for "mostly agree".
    pub agree_overall: f64,
    /// "About right" share each rater must reach.
    pub agree_each: f64,
    /// Votes a player needs to count as a rater.
    pub min_votes: usize,
    /// Share of rounds ended by "another round" for "want more".
    pub want_more: f64,
    /// Rated rounds a player needs for a per-player ρ.
    pub min_rated_for_rho: usize,
    /// Bootstrap resamples for ρ's interval.
    pub bootstrap_reps: usize,
    /// The bootstrap's seed.
    pub bootstrap_seed: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            rounds_each: 20,
            agree_overall: 0.70,
            agree_each: 0.50,
            min_votes: 10,
            want_more: 0.60,
            min_rated_for_rho: 8,
            bootstrap_reps: 2000,
            bootstrap_seed: 0x6772_315F_7265_706F, // "gr1_repo"
        }
    }
}

/// A rate with its sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[allow(
    clippy::struct_field_names,
    reason = "`rate` is the name the JSON report shows"
)]
pub struct Rate {
    /// Trials.
    pub n: usize,
    /// Successes.
    pub k: usize,
    /// `k / n`.
    pub rate: Option<f64>,
    /// 95 % Wilson interval.
    pub ci: Option<(f64, f64)>,
}

fn rate(k: usize, n: usize) -> Rate {
    Rate {
        n,
        k,
        rate: (n > 0).then(|| r4(k as f64 / n as f64)),
        ci: wilson(k, n).map(|(a, b)| (r4(a), r4(b))),
    }
}

fn r4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// A deviation, grouped by message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProblemGroup {
    /// `note`, `warning` or `error`.
    pub severity: &'static str,
    /// What.
    pub message: String,
    /// How many lines.
    pub count: usize,
    /// Where the first one is.
    pub first_at: String,
}

/// What was read.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Input {
    /// Round-log files.
    pub files: usize,
    /// Non-blank lines.
    pub lines: usize,
    /// Lines read into entries.
    pub entries: usize,
    /// Lines skipped as unusable.
    pub skipped_unusable: usize,
    /// Lines skipped for another schema version.
    pub skipped_other_schema: usize,
    /// Errors, warnings and notes found (in all inputs).
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
    /// Notes.
    pub notes: usize,
    /// Lines that look like they hold personal data.
    pub privacy_findings: usize,
    /// The problems, grouped.
    pub problems: Vec<ProblemGroup>,
    /// Where possible personal data was found (place and kind, never the text).
    pub privacy: Vec<String>,
}

/// One player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerRow {
    /// The anonymous id.
    pub id: String,
    /// Log files merged.
    pub files: usize,
    /// Rounds after the restored-round merge (all statuses).
    pub rounds: usize,
    /// Rounds the player tried.
    pub played: usize,
    /// Rounds ended without trying.
    pub skipped: usize,
    /// Broken rounds, and rounds with no readable result.
    pub unusable: usize,
    /// Rounds a host restore logged again (their `end` line count was above one).
    pub restored: usize,
    /// Played rounds that passed.
    pub passed: usize,
    /// Played rounds that ended in a reveal before a pass.
    pub gave_up: usize,
    /// `passed / played`.
    pub pass_rate: Option<f64>,
    /// Played rounds that used a hint, the live meter or the reveal.
    pub assisted: usize,
    /// `assisted / played`.
    pub assisted_share: Option<f64>,
    /// Median active time to pass, in ms.
    pub median_time_to_pass_ms: Option<f64>,
    /// Votes this reader understands.
    pub votes: usize,
    /// Of which "about right".
    pub about_right: usize,
    /// `about_right / votes`.
    pub about_right_share: Option<f64>,
    /// Rounds with a blind rating.
    pub rated: usize,
    /// Disputes.
    pub disputes: usize,
    /// Ended rounds that answered "another round?" with yes.
    pub wanted_more: usize,
    /// `wanted_more / (played + skipped)`.
    pub wanted_more_rate: Option<f64>,
    /// Played and passed per level: `(level, played, passed)`.
    pub by_level: Vec<LevelCount>,
}

/// Played and passed at one level.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelCount {
    /// 1 to 5, or `None` for a code with no readable level.
    pub level: Option<u8>,
    /// Played.
    pub played: usize,
    /// Passed.
    pub passed: usize,
}

/// One level, all players.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelRow {
    /// 1 to 5, or `None` for codes with no readable level.
    pub level: Option<u8>,
    /// Played rounds.
    pub played: usize,
    /// Passed.
    pub passed: usize,
    /// Pass rate with its interval (rounds are the trials; rounds of one player are not independent).
    pub pass: Rate,
    /// Played rounds with no assist.
    pub unassisted_played: usize,
    /// Of which passed.
    pub unassisted_passed: usize,
    /// Played rounds with an assist.
    pub assisted: usize,
    /// Median time to pass, ms.
    pub median_time_to_pass_ms: Option<f64>,
    /// Median time to pass of unassisted passes, ms.
    pub median_time_to_pass_unassisted_ms: Option<f64>,
    /// Median recorded (first-submit) sound %.
    pub median_first_sound: Option<f64>,
    /// Median best sound % over every submit.
    pub median_best_sound: Option<f64>,
}

/// Time to pass, all levels.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TimeToPass {
    /// Passes.
    pub n: usize,
    /// Median, ms.
    pub median_ms: Option<f64>,
    /// Unassisted passes.
    pub unassisted_n: usize,
    /// Median of unassisted passes, ms.
    pub unassisted_median_ms: Option<f64>,
}

/// Assists, all players.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssistSummary {
    /// Played rounds.
    pub played: usize,
    /// Of which assisted.
    pub assisted: usize,
    /// `assisted / played`.
    pub share: Option<f64>,
    /// Rounds that used a hint.
    pub with_hint: usize,
    /// Hints revealed in all.
    pub hints: u64,
    /// Rounds that revealed the target.
    pub revealed: usize,
    /// Rounds that had the live meter.
    pub live_meter: usize,
    /// Passes that were assisted.
    pub assisted_passes: usize,
}

/// The vote's counts and the "mostly agree" reading.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgreementSummary {
    /// Rounds with a recorded score.
    pub scored_rounds: usize,
    /// Votes this reader understands.
    pub votes: usize,
    /// Felt too low.
    pub too_low: usize,
    /// About right.
    pub about_right: usize,
    /// Felt too high.
    pub too_high: usize,
    /// Votes from a newer build.
    pub unknown: usize,
    /// Scored rounds with no vote.
    pub no_vote: usize,
    /// `votes / scored_rounds`.
    pub response_rate: Option<f64>,
    /// Share "about right" among votes.
    pub about_right_share: Option<f64>,
    /// Share "too low".
    pub too_low_share: Option<f64>,
    /// Share "too high".
    pub too_high_share: Option<f64>,
    /// Median recorded sound % among rounds voted too low.
    pub median_sound_too_low: Option<f64>,
    /// Median recorded sound % among rounds voted about right.
    pub median_sound_about_right: Option<f64>,
    /// Median recorded sound % among rounds voted too high.
    pub median_sound_too_high: Option<f64>,
    /// Per player.
    pub players: Vec<PlayerAgreement>,
}

/// One player's votes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerAgreement {
    /// The player.
    pub id: String,
    /// Votes.
    pub votes: usize,
    /// "About right" votes.
    pub about_right: usize,
    /// Share.
    pub share: Option<f64>,
    /// Enough votes to be a rater.
    pub rater: bool,
}

/// ρ with its interval.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RhoOut {
    /// Pairs.
    pub n: usize,
    /// Spearman's ρ.
    pub rho: Option<f64>,
    /// Lower end of the 95 % bootstrap interval.
    pub ci_lo: Option<f64>,
    /// Upper end.
    pub ci_hi: Option<f64>,
}

fn rho_out(x: &[f64], y: &[f64], t: &Thresholds) -> RhoOut {
    let r = stats::spearman_ci(x, y, t.bootstrap_reps, t.bootstrap_seed);
    RhoOut {
        n: r.n,
        rho: r.rho.map(r4),
        ci_lo: r.ci.map(|c| r4(c.0)),
        ci_hi: r.ci.map(|c| r4(c.1)),
    }
}

/// The blind rating against the score.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RatingSummary {
    /// Rounds with a rating and a recorded score.
    pub n: usize,
    /// Mean rating.
    pub mean_rating: Option<f64>,
    /// ρ of the rating against the recorded sound %.
    pub sound: RhoOut,
    /// ρ against structure %.
    pub structure: RhoOut,
    /// ρ against combined %.
    pub combined: RhoOut,
    /// The median recorded sound % at each rating (the curve to refit).
    pub by_rating: Vec<RatingRow>,
    /// Per player (only players with enough rated rounds).
    pub players: Vec<PlayerRho>,
}

/// One rating's row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RatingRow {
    /// 1 to 5.
    pub rating: u8,
    /// Rounds.
    pub n: usize,
    /// Median recorded sound %.
    pub median_sound: Option<f64>,
}

/// One player's ρ.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerRho {
    /// The player.
    pub id: String,
    /// Rated rounds.
    pub n: usize,
    /// ρ against sound %.
    pub rho: Option<f64>,
}

/// "Want more".
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WantMoreSummary {
    /// Ended rounds (played and skipped).
    pub rounds: Rate,
    /// Per player: ended rounds and the "another round" yes share.
    pub players: Vec<PlayerWant>,
    /// The end-of-session answers, when given.
    pub survey: Vec<SurveyRow>,
    /// Players with a log and no survey answer, when a survey was given.
    pub unanswered: Vec<String>,
}

/// One player's "another round?" answers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerWant {
    /// The player.
    pub id: String,
    /// Ended rounds and yes answers.
    pub rounds: Rate,
}

/// One survey answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SurveyRow {
    /// The player.
    pub id: String,
    /// yes, maybe or no.
    pub want_more: String,
    /// OS, if given.
    pub os: Option<String>,
    /// DAW, if given.
    pub daw: Option<String>,
    /// The note, if any.
    pub note: Option<String>,
    /// The id matches a player with a log.
    pub known_player: bool,
}

/// A score dispute, with its round.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisputeRow {
    /// The player.
    pub player: String,
    /// The round's serial within its plugin instance.
    pub serial: u64,
    /// The code.
    pub code: String,
    /// The level.
    pub level: Option<u8>,
    /// How the round ended, when it has an `end` line.
    pub outcome: Option<String>,
    /// The recorded scores (sound, structure, combined), when the round has them.
    pub scores: Option<[f64; 3]>,
    /// The vote, if any.
    pub vote: Option<String>,
    /// The note.
    pub note: String,
    /// The round has an `end` line (a crash leaves a dispute alone).
    pub has_end: bool,
}

/// A value and how often it was seen.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Counted {
    /// The value.
    pub value: String,
    /// How many rounds.
    pub count: usize,
}

fn counted(map: BTreeMap<String, usize>) -> Vec<Counted> {
    map.into_iter()
        .map(|(value, count)| Counted { value, count })
        .collect()
}

/// Things worth checking before the numbers are trusted.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Checks {
    /// Distinct codes played.
    pub codes: usize,
    /// Codes every player played.
    pub codes_played_by_all: usize,
    /// Scorer ids seen on recorded scores, with counts.
    pub scorers: Vec<Counted>,
    /// Engines seen (on the lines and on the reports), with counts.
    pub engines: Vec<Counted>,
    /// More than one set of scorer ids: scores and pass rates of different scorers do not mix.
    pub mixed_scorers: bool,
    /// More than one engine.
    pub mixed_engines: bool,
    /// Codes whose target fingerprint differs between rounds: the same code gave different targets.
    pub fingerprint_drift: Vec<String>,
    /// Codes read as long codes with hand-set amounts, which are not one of the five levels.
    pub custom_codes: usize,
    /// Played rounds whose code has no readable level.
    pub unplaced_rounds: usize,
}

/// A GR1 criterion's reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// Reaches the threshold.
    Met,
    /// Does not.
    NotMet,
    /// Not enough data to say.
    Insufficient,
}

impl Reading {
    /// For the table.
    pub fn text(self) -> &'static str {
        match self {
            Reading::Met => "met",
            Reading::NotMet => "NOT MET",
            Reading::Insufficient => "no data",
        }
    }
}

/// One line of the gate table.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GateRow {
    /// A stable id: `players`, `rounds`, `agree`, `want_more`, `agent`.
    pub id: &'static str,
    /// The check.
    pub check: String,
    /// The proposed reading.
    pub reading: String,
    /// What the logs say.
    pub value: String,
    /// Where that leaves it.
    pub status: Reading,
}

/// The baseline beside the humans, serialized.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BaselineOut {
    /// Distinct codes in the baseline files.
    pub codes: usize,
    /// Of those, codes a human played.
    pub codes_played_by_humans: usize,
    /// Every human round, by best sound %.
    pub humans_all: GroupOut,
    /// The human rounds on the baseline's codes.
    pub humans_same_codes: Option<GroupOut>,
    /// The agents, then the floors.
    pub arms: Vec<GroupOut>,
}

/// A group of the baseline tables.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroupOut {
    /// The label printed.
    pub label: String,
    /// The arm id (`humans` for people).
    pub id: String,
    /// Distinct codes.
    pub codes: usize,
    /// Pooled single-attempt results.
    pub overall: CellOut,
    /// By level.
    pub by_level: Vec<LevelCell>,
    /// Median renders per attempt.
    pub median_renders: Option<f64>,
    /// Median tokens per attempt.
    pub median_tokens: Option<f64>,
    /// Median cost per attempt, US dollars.
    pub median_cost_usd: Option<f64>,
    /// Total cost, US dollars.
    pub total_cost_usd: Option<f64>,
}

/// A group's results at one level.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelCell {
    /// 1 to 5, or `None` for codes with no readable level.
    pub level: Option<u8>,
    /// The results.
    #[serde(flatten)]
    pub cell: CellOut,
}

/// One cell of the baseline tables.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CellOut {
    /// Attempts.
    pub n: usize,
    /// Passes.
    pub passes: usize,
    /// pass@1.
    pub rate: Option<f64>,
    /// 95 % Wilson interval.
    pub ci: Option<(f64, f64)>,
    /// Median sound %.
    pub median_sound: Option<f64>,
}

fn cell_out(c: &Cell) -> CellOut {
    CellOut {
        n: c.n,
        passes: c.passes,
        rate: c.rate.map(r4),
        ci: c.ci.map(|(a, b)| (r4(a), r4(b))),
        median_sound: c.median_sound.map(r2),
    }
}

fn group_out(g: &Group) -> GroupOut {
    GroupOut {
        label: g.label.clone(),
        id: g.id.clone(),
        codes: g.codes,
        overall: cell_out(&g.overall),
        by_level: g
            .by_level
            .iter()
            .map(|(l, c)| LevelCell {
                level: *l,
                cell: cell_out(c),
            })
            .collect(),
        median_renders: g.median_renders.map(r2),
        median_tokens: g.median_tokens.map(r2),
        median_cost_usd: g.median_cost_usd.map(r4),
        total_cost_usd: g.total_cost_usd.map(r4),
    }
}

fn baseline_out(c: &Comparison) -> BaselineOut {
    BaselineOut {
        codes: c.codes,
        codes_played_by_humans: c.codes_played_by_humans,
        humans_all: group_out(&c.humans_all),
        humans_same_codes: c.humans_same_codes.as_ref().map(group_out),
        arms: c.arms.iter().map(group_out).collect(),
    }
}

/// The whole report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    /// [`REPORT_SCHEMA`].
    pub schema: &'static str,
    /// What was read.
    pub input: Input,
    /// The reading of the gate that was used.
    pub thresholds: Thresholds,
    /// The gate table.
    pub gate: Vec<GateRow>,
    /// One row per player.
    pub players: Vec<PlayerRow>,
    /// One row per level.
    pub levels: Vec<LevelRow>,
    /// Time to pass.
    pub time_to_pass: TimeToPass,
    /// Assists.
    pub assists: AssistSummary,
    /// The vote.
    pub agreement: AgreementSummary,
    /// The blind rating against the score.
    pub rating: RatingSummary,
    /// Want more.
    pub want_more: WantMoreSummary,
    /// Disputes.
    pub disputes: Vec<DisputeRow>,
    /// Things to check.
    pub checks: Checks,
    /// The agent baseline beside the humans, when baseline files were given.
    pub baseline: Option<BaselineOut>,
}

/// Everything [`build`] reads besides the log rounds.
#[derive(Default)]
pub struct Extras {
    /// Baseline attempts.
    pub attempts: Vec<baseline::Attempt>,
    /// Survey answers.
    pub survey: Vec<Answer>,
    /// Problems found while reading the baseline and survey files.
    pub problems: Vec<Problem>,
    /// Baseline files read.
    pub baseline_files: usize,
    /// Whether a survey file was given.
    pub survey_given: bool,
}

fn group_problems(problems: &[Problem]) -> Vec<ProblemGroup> {
    let mut groups: Vec<ProblemGroup> = Vec::new();
    for p in problems {
        // Group by message with line numbers masked out of the counts: messages never include them.
        match groups
            .iter_mut()
            .find(|g| g.severity == p.severity.name() && g.message == p.message)
        {
            Some(g) => g.count += 1,
            None => groups.push(ProblemGroup {
                severity: p.severity.name(),
                message: p.message.clone(),
                count: 1,
                first_at: p.place.clone(),
            }),
        }
    }
    groups.sort_by(|a, b| {
        let rank = |s: &str| match s {
            "error" => 0,
            "warning" => 1,
            _ => 2,
        };
        rank(a.severity).cmp(&rank(b.severity))
    });
    groups
}

/// Compute the report.
pub fn build(loaded: &Loaded, extras: &Extras, t: &Thresholds) -> Report {
    let all_problems: Vec<Problem> = loaded
        .problems
        .iter()
        .chain(&extras.problems)
        .cloned()
        .collect();
    let count = |s: Severity| all_problems.iter().filter(|p| p.severity == s).count();
    let input = Input {
        files: loaded.stats.files,
        lines: loaded.stats.lines,
        entries: loaded.stats.entries,
        skipped_unusable: loaded.stats.unusable,
        skipped_other_schema: loaded.stats.other_schema,
        errors: count(Severity::Error),
        warnings: count(Severity::Warn),
        notes: count(Severity::Note),
        privacy_findings: loaded.privacy.len(),
        problems: group_problems(&all_problems),
        privacy: loaded
            .privacy
            .iter()
            .map(|p| format!("{}: {}", p.place, p.message))
            .collect(),
    };

    let rounds: Vec<&Round> = loaded.rounds.iter().collect();
    let played: Vec<&Round> = rounds.iter().copied().filter(|r| r.played()).collect();

    let players: Vec<PlayerRow> = loaded
        .players
        .iter()
        .map(|id| player_row(id, loaded))
        .collect();
    let levels = level_rows(&played);
    let passes: Vec<&Round> = played.iter().copied().filter(|r| r.passed()).collect();
    let times = |rs: &[&Round]| -> Vec<f64> {
        rs.iter()
            .filter_map(|r| r.time_to_pass_ms())
            .map(|t| t as f64)
            .collect()
    };
    let unassisted_passes: Vec<&Round> = passes.iter().copied().filter(|r| !r.assisted()).collect();
    let time_to_pass = TimeToPass {
        n: passes.len(),
        median_ms: median(&times(&passes)),
        unassisted_n: unassisted_passes.len(),
        unassisted_median_ms: median(&times(&unassisted_passes)),
    };
    let assisted_rounds: Vec<&Round> = played.iter().copied().filter(|r| r.assisted()).collect();
    let assists = AssistSummary {
        played: played.len(),
        assisted: assisted_rounds.len(),
        share: (!played.is_empty()).then(|| r4(assisted_rounds.len() as f64 / played.len() as f64)),
        with_hint: played.iter().filter(|r| r.assists().hints > 0).count(),
        hints: played.iter().map(|r| r.assists().hints).sum(),
        revealed: played.iter().filter(|r| r.assists().revealed).count(),
        live_meter: played.iter().filter(|r| r.assists().live_meter).count(),
        assisted_passes: passes.iter().filter(|r| r.assisted()).count(),
    };
    let agreement = agreement_summary(&rounds, loaded, t);
    let rating = rating_summary(&played, loaded, t);
    let want_more = want_more_summary(&rounds, loaded, extras);
    let disputes = dispute_rows(&loaded.disputes, &rounds);
    let checks = checks(&played, &rounds, loaded);
    let comparison = baseline::compare(&extras.attempts, &loaded.rounds);
    let gate = gate_rows(
        loaded,
        &players,
        &agreement,
        &want_more,
        comparison.as_ref(),
        t,
    );

    Report {
        schema: REPORT_SCHEMA,
        input,
        thresholds: t.clone(),
        gate,
        players,
        levels,
        time_to_pass,
        assists,
        agreement,
        rating,
        want_more,
        disputes,
        checks,
        baseline: comparison.as_ref().map(baseline_out),
    }
}

fn player_row(id: &str, loaded: &Loaded) -> PlayerRow {
    let mine: Vec<&Round> = loaded.rounds.iter().filter(|r| r.player == id).collect();
    let played: Vec<&Round> = mine.iter().copied().filter(|r| r.played()).collect();
    let skipped = mine
        .iter()
        .filter(|r| r.status() == Status::Skipped)
        .count();
    let unusable = mine
        .iter()
        .filter(|r| matches!(r.status(), Status::Broken | Status::NoResult))
        .count();
    let passed: Vec<&Round> = played.iter().copied().filter(|r| r.passed()).collect();
    let assisted = played.iter().filter(|r| r.assisted()).count();
    let times: Vec<f64> = passed
        .iter()
        .filter_map(|r| r.time_to_pass_ms())
        .map(|t| t as f64)
        .collect();
    let votes: Vec<&Agreement> = mine
        .iter()
        .filter_map(|r| r.vote())
        .filter(|v| v.is_known())
        .collect();
    let about_right = votes
        .iter()
        .filter(|v| ***v == Agreement::AboutRight)
        .count();
    let ended = played.len() + skipped;
    let wanted = mine
        .iter()
        .filter(|r| matches!(r.status(), Status::Played | Status::Skipped) && r.wanted_more())
        .count();
    let levels: BTreeSet<Option<u8>> = played.iter().map(|r| r.level).collect();
    let mut order: Vec<Option<u8>> = levels.iter().copied().flatten().map(Some).collect();
    if levels.contains(&None) {
        order.push(None);
    }
    PlayerRow {
        id: id.to_owned(),
        files: loaded.files.get(id).map_or(0, Vec::len),
        rounds: mine.len(),
        played: played.len(),
        skipped,
        unusable,
        restored: mine.iter().filter(|r| r.end_lines > 1).count(),
        passed: passed.len(),
        gave_up: played
            .iter()
            .filter(|r| {
                r.result
                    .as_ref()
                    .is_some_and(|x| x.outcome == Outcome::GaveUp)
            })
            .count(),
        pass_rate: (!played.is_empty()).then(|| r4(passed.len() as f64 / played.len() as f64)),
        assisted,
        assisted_share: (!played.is_empty()).then(|| r4(assisted as f64 / played.len() as f64)),
        median_time_to_pass_ms: median(&times),
        votes: votes.len(),
        about_right,
        about_right_share: (!votes.is_empty()).then(|| r4(about_right as f64 / votes.len() as f64)),
        rated: mine
            .iter()
            .filter(|r| r.rating().is_some() && r.first().is_some())
            .count(),
        disputes: loaded.disputes.iter().filter(|d| d.player == id).count(),
        wanted_more: wanted,
        wanted_more_rate: (ended > 0).then(|| r4(wanted as f64 / ended as f64)),
        by_level: order
            .into_iter()
            .map(|level| {
                let at: Vec<&&Round> = played.iter().filter(|r| r.level == level).collect();
                LevelCount {
                    level,
                    played: at.len(),
                    passed: at.iter().filter(|r| r.passed()).count(),
                }
            })
            .collect(),
    }
}

fn level_rows(played: &[&Round]) -> Vec<LevelRow> {
    let levels: BTreeSet<Option<u8>> = played.iter().map(|r| r.level).collect();
    let mut order: Vec<Option<u8>> = levels.iter().copied().flatten().map(Some).collect();
    if levels.contains(&None) {
        order.push(None);
    }
    order
        .into_iter()
        .map(|level| {
            let at: Vec<&Round> = played
                .iter()
                .copied()
                .filter(|r| r.level == level)
                .collect();
            let passed: Vec<&Round> = at.iter().copied().filter(|r| r.passed()).collect();
            let unassisted: Vec<&Round> = at.iter().copied().filter(|r| !r.assisted()).collect();
            let times = |rs: &[&Round]| -> Vec<f64> {
                rs.iter()
                    .filter_map(|r| r.time_to_pass_ms())
                    .map(|t| t as f64)
                    .collect()
            };
            let unassisted_passed: Vec<&Round> =
                unassisted.iter().copied().filter(|r| r.passed()).collect();
            LevelRow {
                level,
                played: at.len(),
                passed: passed.len(),
                pass: rate(passed.len(), at.len()),
                unassisted_played: unassisted.len(),
                unassisted_passed: unassisted_passed.len(),
                assisted: at.len() - unassisted.len(),
                median_time_to_pass_ms: median(&times(&passed)),
                median_time_to_pass_unassisted_ms: median(&times(&unassisted_passed)),
                median_first_sound: median(
                    &at.iter()
                        .filter_map(|r| r.first().map(|f| f.sound))
                        .collect::<Vec<_>>(),
                )
                .map(r2),
                median_best_sound: median(
                    &at.iter().filter_map(|r| r.best_sound()).collect::<Vec<_>>(),
                )
                .map(r2),
            }
        })
        .collect()
}

fn agreement_summary(rounds: &[&Round], loaded: &Loaded, t: &Thresholds) -> AgreementSummary {
    let scored: Vec<&Round> = rounds
        .iter()
        .copied()
        .filter(|r| matches!(r.status(), Status::Played) && r.first().is_some())
        .collect();
    let with = |want: Agreement| -> Vec<&Round> {
        scored
            .iter()
            .copied()
            .filter(|r| r.vote() == Some(&want))
            .collect()
    };
    let (low, right, high) = (
        with(Agreement::TooLow),
        with(Agreement::AboutRight),
        with(Agreement::TooHigh),
    );
    let unknown = scored
        .iter()
        .filter(|r| r.vote().is_some_and(|v| !v.is_known()))
        .count();
    let votes = low.len() + right.len() + high.len();
    let no_vote = scored.iter().filter(|r| r.vote().is_none()).count();
    let share = |k: usize| (votes > 0).then(|| r4(k as f64 / votes as f64));
    let med = |rs: &[&Round]| {
        median(
            &rs.iter()
                .filter_map(|r| r.first().map(|f| f.sound))
                .collect::<Vec<_>>(),
        )
        .map(r2)
    };
    let players = loaded
        .players
        .iter()
        .map(|id| {
            let mine: Vec<&&Round> = scored.iter().filter(|r| &r.player == id).collect();
            let v = mine
                .iter()
                .filter(|r| r.vote().is_some_and(Agreement::is_known))
                .count();
            let ar = mine
                .iter()
                .filter(|r| r.vote() == Some(&Agreement::AboutRight))
                .count();
            PlayerAgreement {
                id: id.clone(),
                votes: v,
                about_right: ar,
                share: (v > 0).then(|| r4(ar as f64 / v as f64)),
                rater: v >= t.min_votes,
            }
        })
        .collect();
    AgreementSummary {
        scored_rounds: scored.len(),
        votes,
        too_low: low.len(),
        about_right: right.len(),
        too_high: high.len(),
        unknown,
        no_vote,
        response_rate: (!scored.is_empty())
            .then(|| r4((votes + unknown) as f64 / scored.len() as f64)),
        about_right_share: share(right.len()),
        too_low_share: share(low.len()),
        too_high_share: share(high.len()),
        median_sound_too_low: med(&low),
        median_sound_about_right: med(&right),
        median_sound_too_high: med(&high),
        players,
    }
}

fn rating_summary(played: &[&Round], loaded: &Loaded, t: &Thresholds) -> RatingSummary {
    let rated: Vec<(&Round, u8)> = played
        .iter()
        .filter_map(|r| r.rating().filter(|_| r.first().is_some()).map(|x| (*r, x)))
        .collect();
    let ratings: Vec<f64> = rated.iter().map(|(_, x)| f64::from(*x)).collect();
    let column = |f: fn(&super::schema::ScoreView) -> f64| -> Vec<f64> {
        rated.iter().filter_map(|(r, _)| r.first().map(f)).collect()
    };
    let (sound, structure, combined) = (
        column(|s| s.sound),
        column(|s| s.structure),
        column(|s| s.combined),
    );
    let by_rating = (1..=super::schema::RATING_MAX)
        .filter_map(|rating| {
            let at: Vec<f64> = rated
                .iter()
                .filter(|(_, x)| *x == rating)
                .filter_map(|(r, _)| r.first().map(|f| f.sound))
                .collect();
            (!at.is_empty()).then(|| RatingRow {
                rating,
                n: at.len(),
                median_sound: median(&at).map(r2),
            })
        })
        .collect();
    let per_player = loaded
        .players
        .iter()
        .filter_map(|id| {
            let mine: Vec<&(&Round, u8)> = rated.iter().filter(|(r, _)| &r.player == id).collect();
            if mine.len() < t.min_rated_for_rho {
                return None;
            }
            let x: Vec<f64> = mine.iter().map(|(_, x)| f64::from(*x)).collect();
            let y: Vec<f64> = mine
                .iter()
                .filter_map(|(r, _)| r.first().map(|f| f.sound))
                .collect();
            Some(PlayerRho {
                id: id.clone(),
                n: mine.len(),
                rho: stats::spearman(&x, &y).map(r4),
            })
        })
        .collect();
    RatingSummary {
        n: rated.len(),
        mean_rating: mean(&ratings).map(r2),
        sound: rho_out(&ratings, &sound, t),
        structure: rho_out(&ratings, &structure, t),
        combined: rho_out(&ratings, &combined, t),
        by_rating,
        players: per_player,
    }
}

fn want_more_summary(rounds: &[&Round], loaded: &Loaded, extras: &Extras) -> WantMoreSummary {
    let ended: Vec<&Round> = rounds
        .iter()
        .copied()
        .filter(|r| matches!(r.status(), Status::Played | Status::Skipped))
        .collect();
    let yes = ended.iter().filter(|r| r.wanted_more()).count();
    let players = loaded
        .players
        .iter()
        .map(|id| {
            let mine: Vec<&&Round> = ended.iter().filter(|r| &r.player == id).collect();
            PlayerWant {
                id: id.clone(),
                rounds: rate(mine.iter().filter(|r| r.wanted_more()).count(), mine.len()),
            }
        })
        .collect();
    let survey: Vec<SurveyRow> = extras
        .survey
        .iter()
        .map(|a| SurveyRow {
            id: a.player.clone(),
            want_more: a.want_more.as_str().to_owned(),
            os: a.os.clone(),
            daw: a.daw.clone(),
            note: a.note.clone(),
            known_player: loaded.players.contains(&a.player),
        })
        .collect();
    let unanswered = if extras.survey_given {
        loaded
            .players
            .iter()
            .filter(|p| !extras.survey.iter().any(|a| &a.player == *p))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    WantMoreSummary {
        rounds: rate(yes, ended.len()),
        players,
        survey,
        unanswered,
    }
}

fn dispute_rows(disputes: &[Dispute], rounds: &[&Round]) -> Vec<DisputeRow> {
    disputes
        .iter()
        .map(|d| {
            let round = rounds.iter().find(|r| {
                r.player == d.player
                    && r.key == d.key
                    && r.code_norm == super::code::normalize(&d.code)
            });
            let first = round.and_then(|r| r.first());
            DisputeRow {
                player: d.player.clone(),
                serial: d.key.serial,
                code: d.code.clone(),
                level: round
                    .and_then(|r| r.level)
                    .or_else(|| super::code::decode(&d.code).and_then(|c| c.level)),
                outcome: round
                    .and_then(|r| r.result.as_ref())
                    .map(|r| r.outcome.as_str().to_owned()),
                scores: first.map(|f| [r2(f.sound), r2(f.structure), r2(f.combined)]),
                vote: round.and_then(|r| r.vote()).map(|v| v.as_str().to_owned()),
                note: d.note.clone(),
                has_end: d.has_end,
            }
        })
        .collect()
}

fn checks(played: &[&Round], rounds: &[&Round], loaded: &Loaded) -> Checks {
    let codes: BTreeSet<&str> = played.iter().map(|r| r.code_norm.as_str()).collect();
    let by_all = codes
        .iter()
        .filter(|code| {
            loaded.players.iter().all(|p| {
                played
                    .iter()
                    .any(|r| &r.player == p && r.code_norm == **code)
            })
        })
        .count();
    let mut scorers: BTreeMap<String, usize> = BTreeMap::new();
    let mut engines: BTreeMap<String, usize> = BTreeMap::new();
    for r in rounds {
        if let Some(f) = r.first() {
            let ids: Vec<&str> = f
                .scorers
                .iter()
                .map(|s| s.as_deref().unwrap_or("?"))
                .collect();
            *scorers.entry(ids.join(" ")).or_default() += 1;
            if let Some(e) = &f.engine {
                *engines.entry(e.clone()).or_default() += 1;
            }
        } else if let Some(e) = &r.engine {
            *engines.entry(e.clone()).or_default() += 1;
        }
    }
    let mut prints: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for r in rounds {
        if let Some(fp) = &r.fingerprint {
            prints
                .entry(r.code_norm.as_str())
                .or_default()
                .insert(fp.as_str());
        }
    }
    let drift: Vec<String> = prints
        .into_iter()
        .filter(|(_, fps)| fps.len() > 1)
        .filter_map(|(code, _)| {
            rounds
                .iter()
                .find(|r| r.code_norm == code)
                .map(|r| r.code.clone())
        })
        .collect();
    Checks {
        codes: codes.len(),
        codes_played_by_all: by_all,
        mixed_scorers: scorers.len() > 1,
        mixed_engines: engines.len() > 1,
        scorers: counted(scorers),
        engines: counted(engines),
        fingerprint_drift: drift,
        custom_codes: played.iter().filter(|r| r.custom).count(),
        unplaced_rounds: played.iter().filter(|r| r.level.is_none()).count(),
    }
}

fn pct(x: f64) -> String {
    format!("{:.0} %", x * 100.0)
}

fn gate_rows(
    loaded: &Loaded,
    players: &[PlayerRow],
    agreement: &AgreementSummary,
    want_more: &WantMoreSummary,
    baseline: Option<&Comparison>,
    t: &Thresholds,
) -> Vec<GateRow> {
    let mut rows = Vec::new();

    rows.push(GateRow {
        id: "players",
        check: "You and your friends play".to_owned(),
        reading: "at least 2 players (1 is the owner-only fallback)".to_owned(),
        value: format!("{} player(s)", loaded.players.len()),
        status: match loaded.players.len() {
            0 => Reading::Insufficient,
            1 => Reading::NotMet,
            _ => Reading::Met,
        },
    });

    let short: Vec<String> = players
        .iter()
        .filter(|p| p.played < t.rounds_each)
        .map(|p| {
            let skipped = if p.skipped > 0 {
                format!(", {} skipped", p.skipped)
            } else {
                String::new()
            };
            format!("{} {}/{}{skipped}", p.id, p.played, t.rounds_each)
        })
        .collect();
    rows.push(GateRow {
        id: "rounds",
        check: format!("{} rounds each, logged", t.rounds_each),
        reading: format!("every player has at least {} played rounds", t.rounds_each),
        value: if players.is_empty() {
            "no rounds".to_owned()
        } else if short.is_empty() {
            format!(
                "all {} players have {} or more (fewest {})",
                players.len(),
                t.rounds_each,
                players.iter().map(|p| p.played).min().unwrap_or(0)
            )
        } else {
            format!("short: {}", short.join(", "))
        },
        status: if players.is_empty() {
            Reading::Insufficient
        } else if short.is_empty() {
            Reading::Met
        } else {
            Reading::NotMet
        },
    });

    let raters: Vec<&PlayerAgreement> = agreement.players.iter().filter(|p| p.rater).collect();
    let below: Vec<String> = raters
        .iter()
        .filter(|p| p.share.is_some_and(|s| s < t.agree_each))
        .map(|p| p.id.clone())
        .collect();
    let agree_status = match agreement.about_right_share {
        _ if raters.is_empty() => Reading::Insufficient,
        Some(s) if s >= t.agree_overall && below.is_empty() => Reading::Met,
        _ => Reading::NotMet,
    };
    let thin = agreement.players.len() - raters.len();
    rows.push(GateRow {
        id: "agree",
        check: "Mostly agree with the score".to_owned(),
        reading: format!(
            "at least {} \"about right\" overall and no rater under {} (a rater has {} votes or more)",
            pct(t.agree_overall),
            pct(t.agree_each),
            t.min_votes
        ),
        value: match agreement.about_right_share {
            None => "no votes".to_owned(),
            Some(s) => format!(
                "{} about right over {} votes{}{}",
                pct(s),
                agreement.votes,
                if below.is_empty() { String::new() } else { format!("; under {}: {}", pct(t.agree_each), below.join(", ")) },
                if thin > 0 { format!("; {thin} player(s) with too few votes to rate") } else { String::new() }
            ),
        },
        status: agree_status,
    });

    let want = &want_more.rounds;
    rows.push(GateRow {
        id: "want_more",
        check: "Want more".to_owned(),
        reading: format!(
            "\"another round?\" answered yes on at least {} of rounds",
            pct(t.want_more)
        ),
        value: match want.rate {
            None => "no rounds".to_owned(),
            Some(r) => {
                let answers: Vec<&SurveyRow> = want_more.survey.iter().collect();
                let yes = answers
                    .iter()
                    .filter(|a| a.want_more == WantMore::Yes.as_str())
                    .count();
                let survey = if answers.is_empty() {
                    String::new()
                } else {
                    format!("; end-of-session: {yes} of {} said yes", answers.len())
                };
                format!("{} ({} of {} rounds){survey}", pct(r), want.k, want.n)
            }
        },
        status: match want.rate {
            None => Reading::Insufficient,
            Some(r) if r >= t.want_more => Reading::Met,
            Some(_) => Reading::NotMet,
        },
    });

    rows.push(GateRow {
        id: "agent",
        check: "The agent's solve rate is recorded as a baseline".to_owned(),
        reading: "an agent arm beside the no-op and CMA-ES floors".to_owned(),
        value: match baseline {
            None => "no baseline file given".to_owned(),
            Some(c) => {
                let agent = c
                    .arms
                    .iter()
                    .find(|g| g.id != baseline::ARM_NOOP && g.id != baseline::ARM_CMAES);
                let floors = [baseline::ARM_NOOP, baseline::ARM_CMAES]
                    .iter()
                    .filter(|f| c.has_arm(f))
                    .count();
                match agent {
                    Some(g) => format!(
                        "{}: pass@1 {} over {} attempts on {} codes; floors present: {floors} of 2",
                        g.label,
                        g.overall.rate.map_or("n/a".to_owned(), pct),
                        g.overall.n,
                        g.codes
                    ),
                    None => format!("floors only ({floors} of 2); no agent arm"),
                }
            }
        },
        status: match baseline {
            None => Reading::Insufficient,
            Some(c)
                if c.has_agent()
                    && c.has_arm(baseline::ARM_NOOP)
                    && c.has_arm(baseline::ARM_CMAES) =>
            {
                Reading::Met
            }
            Some(_) => Reading::NotMet,
        },
    });
    rows
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use serde_json::{Value, json};

    use super::*;
    use crate::gr1_report::code::encode_short;
    use crate::gr1_report::fixtures::report_of;

    /// One round to write: the few things the numbers care about.
    #[derive(Clone)]
    struct R {
        serial: u64,
        level: u8,
        outcome: &'static str,
        sound: f64,
        vote: Option<&'static str>,
        rating: Option<u8>,
        ended_by: &'static str,
        hints: u8,
        revealed: bool,
    }

    impl R {
        fn new(serial: u64) -> R {
            R {
                serial,
                level: 2,
                outcome: "passed",
                sound: 92.0,
                vote: Some("about_right"),
                rating: Some(4),
                ended_by: "new_round",
                hints: 0,
                revealed: false,
            }
        }
        fn vote(mut self, v: &'static str) -> R {
            self.vote = Some(v);
            self
        }
        fn ended(mut self, e: &'static str) -> R {
            self.ended_by = e;
            self
        }
        fn level(mut self, l: u8) -> R {
            self.level = l;
            self
        }
        fn rating(mut self, r: Option<u8>) -> R {
            self.rating = r;
            self
        }
        fn sound(mut self, s: f64) -> R {
            self.sound = s;
            self
        }
        fn fail(mut self) -> R {
            self.outcome = "not_passed";
            self.sound = 40.0;
            self
        }

        fn json(&self) -> Value {
            let passed = self.outcome == "passed";
            json!({
                "schema": "enth.round-log/1", "event": "end",
                "round": {"serial": self.serial, "nonce": 1000 + self.serial},
                "code": encode_short(self.level, 1, 500 + self.serial),
                "ended_by": self.ended_by, "broken": false,
                "result": {
                    "schema": "enth.round-result/1", "outcome": self.outcome,
                    "first_submit": {
                        "sound": {"scorer": "sound/1", "percent": self.sound},
                        "structure": {"scorer": "structure/1", "percent": 50.0},
                        "combined": {"scorer": "combined/1", "percent": 60.0},
                        "pass_mark": 90, "passed": passed, "engine": "0.2.0"
                    },
                    "submits": 1, "time_to_pass_ms": if passed { json!(60_000 + self.serial * 1000) } else { Value::Null },
                    "active_ms": 100_000,
                    "assists": {"hints": self.hints, "live_meter": false, "revealed": self.revealed},
                    "agreement": self.vote, "rating": self.rating
                },
                "submits": [{"at_ms": 1000, "passed": passed, "sound": (self.sound * 100.0).round(), "structure": 5000, "combined": 6000}],
                "edits": 3, "patch": "{}"
            })
        }
    }

    fn log(rounds: &[R]) -> String {
        rounds.iter().fold(String::new(), |mut out, r| {
            let _ = writeln!(out, "{}", r.json());
            out
        })
    }

    fn report(players: &[(&str, Vec<R>)]) -> Report {
        let texts: Vec<(String, String)> = players
            .iter()
            .map(|(id, rs)| ((*id).to_owned(), log(rs)))
            .collect();
        let logs: Vec<(&str, &str, &str)> = texts
            .iter()
            .map(|(id, t)| (id.as_str(), "x.jsonl", t.as_str()))
            .collect();
        report_of(&logs, &[], None)
    }

    /// `votes` rounds, `right` of them "about right", the rest "too low".
    fn voter(right: usize, votes: usize) -> Vec<R> {
        (0..votes)
            .map(|i| R::new(i as u64 + 1).vote(if i < right { "about_right" } else { "too_low" }))
            .collect()
    }

    fn agree(r: &Report) -> Reading {
        r.gate.iter().find(|g| g.id == "agree").unwrap().status
    }

    #[test]
    fn mostly_agree_needs_70_percent_overall_and_no_rater_under_50() {
        // 9/10 and 6/10: 75 % overall, nobody under 50 %.
        assert_eq!(
            agree(&report(&[("a", voter(9, 10)), ("b", voter(6, 10))])),
            Reading::Met
        );
        // Exactly 70 % overall and exactly 50 % for the weakest rater: both thresholds are "at least".
        assert_eq!(
            agree(&report(&[("a", voter(9, 10)), ("b", voter(5, 10))])),
            Reading::Met
        );
        // 65 % overall.
        assert_eq!(
            agree(&report(&[("a", voter(7, 10)), ("b", voter(6, 10))])),
            Reading::NotMet
        );
        // 70 % overall, but one rater is at 40 %.
        let r = report(&[("a", voter(10, 10)), ("b", voter(4, 10))]);
        assert_eq!(agree(&r), Reading::NotMet);
        assert!(
            r.gate
                .iter()
                .find(|g| g.id == "agree")
                .unwrap()
                .value
                .contains("under 50 %: b"),
            "{:?}",
            r.gate
        );
    }

    #[test]
    fn a_player_with_too_few_votes_is_not_a_rater_and_no_raters_means_no_data() {
        // `c` voted 3 times, all "too low": not a rater, so the floor does not apply to it, but it is said.
        let r = report(&[("a", voter(9, 10)), ("b", voter(8, 10)), ("c", voter(0, 3))]);
        assert_eq!(agree(&r), Reading::Met);
        assert!(
            r.gate
                .iter()
                .find(|g| g.id == "agree")
                .unwrap()
                .value
                .contains("1 player(s) with too few votes")
        );
        assert!(
            r.agreement
                .players
                .iter()
                .find(|p| p.id == "c")
                .is_some_and(|p| !p.rater)
        );
        // Nobody with enough votes: no reading at all.
        assert_eq!(agree(&report(&[("a", voter(5, 5))])), Reading::Insufficient);
        assert_eq!(agree(&report(&[])), Reading::Insufficient);
    }

    #[test]
    fn want_more_is_the_share_of_rounds_ended_by_another_round() {
        // 6 of 10 is exactly the threshold.
        let rounds = |yes: usize| -> Vec<R> {
            (0..10)
                .map(|i| {
                    R::new(i + 1).ended(if (i as usize) < yes {
                        "new_round"
                    } else {
                        "keep"
                    })
                })
                .collect()
        };
        let status = |yes| {
            report(&[("a", rounds(yes))])
                .gate
                .iter()
                .find(|g| g.id == "want_more")
                .unwrap()
                .status
        };
        assert_eq!(status(6), Reading::Met);
        assert_eq!(status(5), Reading::NotMet);
        let r = report(&[("a", rounds(6)), ("b", rounds(2))]);
        assert_eq!((r.want_more.rounds.k, r.want_more.rounds.n), (8, 20));
        assert_eq!(
            r.want_more
                .players
                .iter()
                .map(|p| p.rounds.k)
                .collect::<Vec<_>>(),
            [6, 2]
        );
    }

    #[test]
    fn rounds_each_counts_played_rounds_not_skipped_or_broken_ones() {
        let mut rs: Vec<R> = (0..20).map(|i| R::new(i + 1)).collect();
        let r = report(&[("a", rs.clone())]);
        assert_eq!(
            r.gate.iter().find(|g| g.id == "rounds").unwrap().status,
            Reading::Met
        );
        // One round ends without a submit and without a reveal: skipped, so 19 played.
        let mut skipped = R::new(21).json();
        skipped["result"]["submits"] = json!(0);
        skipped["result"]["first_submit"] = Value::Null;
        skipped["result"]["time_to_pass_ms"] = Value::Null;
        skipped["result"]["outcome"] = json!("not_passed");
        skipped["submits"] = json!([]);
        let mut broken = R::new(22).json();
        broken["broken"] = json!(true);
        broken["result"] = Value::Null;
        rs.pop();
        let text = format!("{}{skipped}\n{broken}\n", log(&rs));
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        let p = &r.players[0];
        assert_eq!((p.rounds, p.played, p.skipped, p.unusable), (21, 19, 1, 1));
        let gate = r.gate.iter().find(|g| g.id == "rounds").unwrap();
        assert_eq!(gate.status, Reading::NotMet);
        assert_eq!(gate.value, "short: a 19/20, 1 skipped");
    }

    #[test]
    fn a_code_with_no_readable_level_is_counted_apart_not_dropped() {
        let mut odd = R::new(3).json();
        odd["code"] = json!("ZZZZ-ZZZZ-ZZZZ-ZZ");
        odd["result"].as_object_mut().unwrap().remove("code");
        let text = format!("{}{odd}\n", log(&[R::new(1).level(1), R::new(2).level(5)]));
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        let levels: Vec<Option<u8>> = r.levels.iter().map(|l| l.level).collect();
        assert_eq!(
            levels,
            [Some(1), Some(5), None],
            "the unplaced level sorts last"
        );
        assert_eq!(r.checks.unplaced_rounds, 1);
        assert_eq!(r.players[0].played, 3);
    }

    #[test]
    fn the_blind_rating_needs_variation_and_a_score_to_have_a_rho() {
        // Every rating is 4: no rank correlation, and no panic.
        let rs: Vec<R> = (0..12)
            .map(|i| R::new(i + 1).sound(50.0 + i as f64))
            .collect();
        let r = report(&[("a", rs)]);
        assert_eq!(
            (r.rating.n, r.rating.sound.rho, r.rating.sound.ci_lo),
            (12, None, None)
        );
        // A perfectly monotone relation is +1.
        let rs: Vec<R> = (0..12)
            .map(|i| {
                R::new(i + 1)
                    .sound(10.0 + 7.0 * i as f64)
                    .rating(Some((i / 3 + 1) as u8))
            })
            .collect();
        let r = report(&[("a", rs)]);
        assert!(r.rating.sound.rho.unwrap() > 0.95, "{:?}", r.rating.sound);
        // Rounds with no rating do not count as pairs.
        let rs: Vec<R> = (0..12).map(|i| R::new(i + 1).rating(None)).collect();
        assert_eq!(report(&[("a", rs)]).rating.n, 0);
    }

    #[test]
    fn nothing_to_report_is_not_a_panic() {
        let r = report(&[]);
        assert!(r.players.is_empty() && r.levels.is_empty() && r.disputes.is_empty());
        assert_eq!(
            (r.agreement.votes, r.agreement.about_right_share),
            (0, None)
        );
        assert_eq!((r.time_to_pass.n, r.time_to_pass.median_ms), (0, None));
        assert_eq!(r.want_more.rounds.rate, None);
        assert!(r.baseline.is_none());
        assert!(
            r.gate
                .iter()
                .filter(|g| g.id != "players")
                .all(|g| g.status == Reading::Insufficient)
        );
    }

    #[test]
    fn the_same_code_with_two_targets_is_flagged_and_mixed_scorers_and_engines_too() {
        let mut a = R::new(1).json();
        let mut b = R::new(2).json();
        // Same code, two fingerprints.
        let code = encode_short(3, 1, 1);
        for (v, fp) in [(&mut a, "ab".repeat(32)), (&mut b, "cd".repeat(32))] {
            v["code"] = json!(code);
            v["fingerprint"] = json!(fp);
        }
        b["result"]["first_submit"]["sound"]["scorer"] = json!("sound/2");
        b["result"]["first_submit"]["engine"] = json!("0.3.0");
        let text = format!("{a}\n{b}\n");
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        assert_eq!(r.checks.fingerprint_drift, [code]);
        assert!(r.checks.mixed_scorers && r.checks.mixed_engines);
        // One fingerprint per code is clean.
        let r = report(&[("a", vec![R::new(1)])]);
        assert!(r.checks.fingerprint_drift.is_empty() && !r.checks.mixed_scorers);
    }

    #[test]
    fn an_unknown_outcome_with_a_time_to_pass_counts_as_a_pass() {
        let mut v = R::new(1).json();
        v["result"]["outcome"] = json!("passed_with_style");
        let text = format!("{v}\n");
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        assert_eq!(r.players[0].passed, 1);
        // Without a time it is not one.
        v["result"]["time_to_pass_ms"] = Value::Null;
        let text = format!("{v}\n");
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        assert_eq!(r.players[0].passed, 0);
    }

    #[test]
    fn survey_answers_are_matched_to_players_and_the_silent_ones_listed() {
        let text = log(&[R::new(1)]);
        let survey = [
            json!({"schema": "enth.gr1-survey/1", "player": "a", "want_more": "yes"}),
            json!({"schema": "enth.gr1-survey/1", "player": "ghost", "want_more": "no"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let r = report_of(
            &[("a", "a.jsonl", &text), ("b", "b.jsonl", &text)],
            &[],
            Some(&survey),
        );
        assert_eq!(r.want_more.unanswered, ["b"]);
        assert_eq!(
            r.want_more
                .survey
                .iter()
                .map(|s| s.known_player)
                .collect::<Vec<_>>(),
            [true, false]
        );
        // Without a survey file nobody is "unanswered".
        let r = report_of(&[("a", "a.jsonl", &text)], &[], None);
        assert!(
            r.want_more.unanswered.is_empty(),
            "{:?}",
            r.want_more.unanswered
        );
    }

    #[test]
    fn pass_rate_time_and_assists_come_from_played_rounds_by_level() {
        let rs = vec![
            R::new(1).level(1),
            R::new(2).level(1).fail(),
            R {
                hints: 2,
                ..R::new(3).level(1)
            },
            R {
                revealed: true,
                outcome: "gave_up",
                ..R::new(4).level(1)
            },
        ];
        let r = report(&[("a", rs)]);
        let l1 = &r.levels[0];
        assert_eq!(
            (
                l1.played,
                l1.passed,
                l1.assisted,
                l1.unassisted_played,
                l1.unassisted_passed
            ),
            (4, 2, 2, 2, 1)
        );
        assert_eq!(l1.pass.rate, Some(0.5));
        // The two passes took 61 s and 63 s.
        assert_eq!(l1.median_time_to_pass_ms, Some(62_000.0));
        assert_eq!(l1.median_time_to_pass_unassisted_ms, Some(61_000.0));
        assert_eq!(
            (r.players[0].gave_up, r.assists.hints, r.assists.revealed),
            (1, 2, 1)
        );
    }
}
