//! **Owner: lane CV3** (Challenge v3, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "Round types", and docs/challenge.md, "Challenge v3".
//!
//! The ear scorers, each with a [`ScorerId`] beside the old ones (a scorer that would move any score gets a new id,
//! never an edit):
//!
//! | Id | Judges | How |
//! | --- | --- | --- |
//! | `ear/1` | a label or clip index ([`EarAnswer::Choice`]) | exactly, against the seeded key: right is 100 %, wrong 0 % |
//! | `ear-value/1` | a value on a knob or slider ([`EarAnswer::Value`]) | the error in perceptual units ([`ValueScale`]: octaves, doublings of ms, or the unit itself); right within the key's tolerance |
//! | `taps/1` | tap times in ms ([`EarAnswer::Taps`]) | after the best-fit offset and tempo (±10 %), a one-to-one match within the key's window; misses and extras count against: hits / (onsets + extras) |
//!
//! **No render, anywhere.** A judge reads an answer key ([`EarKey`]: integers, and a value's `f32` bits) and an
//! answer; nothing here takes a patch or audio, so an ear result is judged identically on every platform and engine
//! version, and only a new frozen generator version moves a key. Logarithms go through the pinned `libm`, so a value's
//! error has the same bits everywhere. A set ([`judge_set`]) is the mean of its items' percentages in question order,
//! and passes at the challenge's pass mark ([`ScoringSpec::passes`]).
//!
//! The patch round's dispatch ([`crate::score::round`]) refuses an ear target; [`crate::ear::question::EarSet::judge`]
//! is this module's entry point for a prepared set, and [`judge`] for one question.

use core::fmt;

use crate::model::{AnswerKind, ScoringSpec};
use crate::score::ScorerId;

impl ScorerId {
    /// `ear/1`: a choice, judged exactly against its key (lane CV3).
    pub const EAR_V1: ScorerId = ScorerId::new("ear", 1);
    /// `ear-value/1`: a value, judged as an error in perceptual units within the key's tolerance (lane CV3).
    pub const EAR_VALUE_V1: ScorerId = ScorerId::new("ear-value", 1);
    /// `taps/1`: tap times, judged after the best-fit offset and tempo with a one-to-one match (lane CV3).
    pub const TAPS_V1: ScorerId = ScorerId::new("taps", 1);
}

/// The ear scorers this build has.
pub const EAR_SCORERS: [ScorerId; 3] =
    [ScorerId::EAR_V1, ScorerId::EAR_VALUE_V1, ScorerId::TAPS_V1];

/// The most onsets a taps key holds (two bars of sixteenths).
pub const MAX_ONSETS: usize = 32;

/// The most taps an answer may hold (twice [`MAX_ONSETS`]; more is refused, not judged).
pub const MAX_TAPS: usize = 64;

/// The tempo ratios `taps/1` tries, in per mille of the key's tempo: ±10 % in 2.5 % steps.
pub const TAPS_TEMPI: [i64; 9] = [900, 925, 950, 975, 1000, 1025, 1050, 1075, 1100];

/// The scorer this build gives an answer kind: `ear/1`, `ear-value/1`, `taps/1`; `None` for a patch (the sound,
/// structure and combined scorers judge it).
pub fn scorer_for(answer: AnswerKind) -> Option<ScorerId> {
    match answer {
        AnswerKind::Choice => Some(ScorerId::EAR_V1),
        AnswerKind::Value => Some(ScorerId::EAR_VALUE_V1),
        AnswerKind::Taps => Some(ScorerId::TAPS_V1),
        AnswerKind::Patch => None,
    }
}

/// The answer kind an ear scorer judges (by its name, any version).
pub fn answer_of(id: &ScorerId) -> Option<AnswerKind> {
    match id.name() {
        "ear" => Some(AnswerKind::Choice),
        "ear-value" => Some(AnswerKind::Value),
        "taps" => Some(AnswerKind::Taps),
        _ => None,
    }
}

/// How a value's error is measured: the perceptual unit of `ear-value/1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueScale {
    /// A frequency (Hz): `|log2(answer / key)|` octaves. Both must be above 0.
    Octaves,
    /// A time (ms): `|log2((answer + 1) / (key + 1))|`, in doublings, with 1 ms added so times near 0 do not count
    /// every microsecond (the times' just-noticeable step uses the same floor). Both must be at least 0.
    LogMs,
    /// Anything heard on a linear scale in its own unit (semitones, cents, dB, %): `|answer − key|`.
    Linear,
}

impl ValueScale {
    /// The unit an error is reported in.
    pub fn unit(self) -> &'static str {
        match self {
            ValueScale::Octaves => "oct",
            ValueScale::LogMs => "doublings",
            ValueScale::Linear => "unit",
        }
    }

    /// The error of `answer` against `key` in this scale, or `None` where it cannot be taken (a non-finite value, a
    /// frequency at or below 0, a negative time).
    pub fn error(self, key: f32, answer: f32) -> Option<f64> {
        let (k, a) = (f64::from(key), f64::from(answer));
        if !k.is_finite() || !a.is_finite() {
            return None;
        }
        match self {
            ValueScale::Octaves => (k > 0.0 && a > 0.0).then(|| libm::fabs(libm::log2(a / k))),
            ValueScale::LogMs => {
                (k >= 0.0 && a >= 0.0).then(|| libm::fabs(libm::log2((a + 1.0) / (k + 1.0))))
            }
            ValueScale::Linear => Some(libm::fabs(a - k)),
        }
    }
}

/// A value question's key: the right value (its exact `f32` bits), the scale its error is measured on, and the
/// tolerance within which an answer is right, in thousandths of the scale's unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ValueKey {
    /// How the error is measured.
    pub scale: ValueScale,
    /// The right value's `f32` bits (a draw's value or a generated one; exact on every platform).
    pub target_bits: u32,
    /// The tolerance, in thousandths of the scale's unit (250 = a quarter of an octave on [`ValueScale::Octaves`]).
    pub tolerance_milli: u32,
}

impl ValueKey {
    /// A key for `target`.
    pub fn new(scale: ValueScale, target: f32, tolerance_milli: u32) -> ValueKey {
        ValueKey {
            scale,
            target_bits: target.to_bits(),
            tolerance_milli,
        }
    }

    /// The right value.
    pub fn target(&self) -> f32 {
        f32::from_bits(self.target_bits)
    }

    /// The tolerance in the scale's unit.
    pub fn tolerance(&self) -> f64 {
        f64::from(self.tolerance_milli) / 1000.0
    }
}

/// A tap question's key: the onsets in ms from the start of the response, and the window within which a tap counts as
/// on an onset (after the best-fit offset and tempo).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TapsKey {
    /// The onsets, in ms, strictly increasing; 1 to [`MAX_ONSETS`].
    pub onsets_ms: Vec<u32>,
    /// The window, in ms (at least 1).
    pub window_ms: u16,
}

/// What a question's right answer is: integers (and a value's `f32` bits), so judging needs no render.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EarKey {
    /// The right option's index, of `options` (2 and up).
    Choice {
        /// The right option, 0-based.
        index: u8,
        /// How many options the question offers.
        options: u8,
    },
    /// A value within a tolerance.
    Value(ValueKey),
    /// Onsets within a window.
    Taps(TapsKey),
}

impl EarKey {
    /// The answer kind the key is for.
    pub fn answer_kind(&self) -> AnswerKind {
        match self {
            EarKey::Choice { .. } => AnswerKind::Choice,
            EarKey::Value(_) => AnswerKind::Value,
            EarKey::Taps(_) => AnswerKind::Taps,
        }
    }

    /// The answer that is right for this key (what an oracle player answers; `ear-check`'s and the tests' oracle).
    pub fn right_answer(&self) -> EarAnswer {
        match self {
            EarKey::Choice { index, .. } => EarAnswer::Choice(*index),
            EarKey::Value(v) => EarAnswer::Value(v.target()),
            EarKey::Taps(t) => EarAnswer::Taps(t.onsets_ms.clone()),
        }
    }
}

/// A player's answer to one question, from the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum EarAnswer {
    /// An option's index, 0-based.
    Choice(u8),
    /// A value, in the parameter's or slider's real unit.
    Value(f32),
    /// Tap times in ms, in the order they came (non-decreasing), on any clock: the offset is fitted.
    Taps(Vec<u32>),
}

impl EarAnswer {
    /// The answer kind.
    pub fn answer_kind(&self) -> AnswerKind {
        match self {
            EarAnswer::Choice(_) => AnswerKind::Choice,
            EarAnswer::Value(_) => AnswerKind::Value,
            EarAnswer::Taps(_) => AnswerKind::Taps,
        }
    }
}

/// One question judged.
#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    /// Whether the answer is right (a choice equal to the key; a value within the tolerance; every onset hit and no
    /// extra tap).
    pub right: bool,
    /// The item's score, 0 to 100 (a choice and a value: 100 or 0; taps: hits / (onsets + extras)).
    pub percent: f64,
    /// What was measured.
    pub detail: Detail,
}

/// What a judge measured, per kind.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Detail {
    /// No answer was given: wrong, 0 %.
    Unanswered,
    /// A choice.
    Choice {
        /// The answer.
        answer: u8,
        /// The key.
        key: u8,
    },
    /// A value's error, in the scale's unit, and the tolerance.
    Value {
        /// The error.
        error: f64,
        /// The tolerance.
        tolerance: f64,
        /// The scale.
        scale: ValueScale,
    },
    /// Taps.
    Taps(TapsJudged),
}

/// What `taps/1` measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapsJudged {
    /// Onsets matched by a tap within the window.
    pub hits: u16,
    /// Onsets with no tap.
    pub misses: u16,
    /// Taps on no onset.
    pub extras: u16,
    /// The summed absolute timing error of the hits, in ms, after the fit.
    pub error_ms: u64,
    /// The fitted offset: the tap time that the anchor onset maps to, minus that onset.
    pub offset_ms: i64,
    /// The fitted tempo, in per mille of the key's ([`TAPS_TEMPI`]).
    pub tempo_permille: u16,
}

impl TapsJudged {
    /// The mean absolute timing error of the hits, in ms (0 with no hit).
    pub fn mean_error_ms(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.error_ms as f64 / f64::from(self.hits)
        }
    }
}

/// Why a question cannot be judged. Never "wrong": a wrong answer is a [`Judged`] with `right` false.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarScoreError {
    /// A scorer this build does not have (`unknown_scorer`): a newer build's.
    UnknownScorer {
        /// The id.
        id: ScorerId,
    },
    /// The scorer, the key and the answer are not of one kind (`bad_answer`).
    Mismatch {
        /// The scorer.
        id: ScorerId,
        /// The key's kind.
        key: AnswerKind,
        /// The answer's kind.
        answer: AnswerKind,
    },
    /// An answer no shell sends: an index beyond the options, a value the scale cannot measure, too many or
    /// unordered taps (`bad_answer`).
    BadAnswer {
        /// What is wrong.
        reason: String,
    },
    /// A key no generator makes: fewer than 2 options or an index beyond them, no onsets or unordered ones, a window of
    /// 0 (`bad_key`).
    BadKey {
        /// What is wrong.
        reason: String,
    },
    /// A set judged with another number of answers than questions (`bad_answer`).
    Count {
        /// The questions.
        keys: usize,
        /// The answers.
        answers: usize,
    },
}

impl EarScoreError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            EarScoreError::UnknownScorer { .. } => "unknown_scorer",
            EarScoreError::Mismatch { .. }
            | EarScoreError::BadAnswer { .. }
            | EarScoreError::Count { .. } => "bad_answer",
            EarScoreError::BadKey { .. } => "bad_key",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            EarScoreError::UnknownScorer { .. } => {
                "made by a newer enth (a scorer this build does not have): update enth".to_owned()
            }
            EarScoreError::Mismatch { .. }
            | EarScoreError::BadAnswer { .. }
            | EarScoreError::Count { .. } => {
                "the answer does not fit the question: one answer per question, of the kind it asks for".to_owned()
            }
            EarScoreError::BadKey { .. } => {
                "the question's key is damaged: please report it with the challenge".to_owned()
            }
        }
    }
}

impl fmt::Display for EarScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EarScoreError::UnknownScorer { id } => write!(f, "scorer {id} is not in this build"),
            EarScoreError::Mismatch { id, key, answer } => write!(
                f,
                "{id} cannot judge a {answer:?} answer against a {key:?} key"
            ),
            EarScoreError::BadAnswer { reason } | EarScoreError::BadKey { reason } => {
                f.write_str(reason)
            }
            EarScoreError::Count { keys, answers } => {
                write!(f, "{answers} answers for {keys} questions")
            }
        }
    }
}

impl std::error::Error for EarScoreError {}

/// Judge one answer with the scorer `id` (no render). An unknown scorer is `unknown_scorer`; a scorer, key and answer
/// of different kinds are `bad_answer`.
pub fn judge(id: &ScorerId, key: &EarKey, answer: &EarAnswer) -> Result<Judged, EarScoreError> {
    if !EAR_SCORERS.contains(id) {
        return Err(EarScoreError::UnknownScorer { id: id.clone() });
    }
    let kind = answer_of(id);
    if kind != Some(key.answer_kind()) || kind != Some(answer.answer_kind()) {
        return Err(EarScoreError::Mismatch {
            id: id.clone(),
            key: key.answer_kind(),
            answer: answer.answer_kind(),
        });
    }
    match (key, answer) {
        (EarKey::Choice { index, options }, EarAnswer::Choice(a)) => {
            judge_choice(*index, *options, *a)
        }
        (EarKey::Value(k), EarAnswer::Value(a)) => judge_value(k, *a),
        (EarKey::Taps(k), EarAnswer::Taps(t)) => judge_taps(k, t),
        // The kinds were checked above.
        _ => Err(EarScoreError::Mismatch {
            id: id.clone(),
            key: key.answer_kind(),
            answer: answer.answer_kind(),
        }),
    }
}

/// `ear/1`: right when the index is the key's.
fn judge_choice(index: u8, options: u8, answer: u8) -> Result<Judged, EarScoreError> {
    if options < 2 || index >= options {
        return Err(EarScoreError::BadKey {
            reason: format!("a choice key {index} of {options} options"),
        });
    }
    if answer >= options {
        return Err(EarScoreError::BadAnswer {
            reason: format!("option {answer} of {options} (0-based)"),
        });
    }
    let right = answer == index;
    Ok(Judged {
        right,
        percent: if right { 100.0 } else { 0.0 },
        detail: Detail::Choice { answer, key: index },
    })
}

/// `ear-value/1`: right when the error in the key's scale is within its tolerance.
fn judge_value(key: &ValueKey, answer: f32) -> Result<Judged, EarScoreError> {
    if key.scale.error(key.target(), key.target()).is_none() {
        return Err(EarScoreError::BadKey {
            reason: format!("a value key {} that its scale cannot measure", key.target()),
        });
    }
    let error = key
        .scale
        .error(key.target(), answer)
        .ok_or_else(|| EarScoreError::BadAnswer {
            reason: format!("the value {answer} cannot be measured on its scale"),
        })?;
    let tolerance = key.tolerance();
    let right = error <= tolerance;
    Ok(Judged {
        right,
        percent: if right { 100.0 } else { 0.0 },
        detail: Detail::Value {
            error,
            tolerance,
            scale: key.scale,
        },
    })
}

/// `taps/1`: the best fit over anchors (each onset against each tap) and [`TAPS_TEMPI`], each scored by a one-to-one
/// monotone match within the window; the most hits win, then the least summed error, then the first fit found.
fn judge_taps(key: &TapsKey, taps: &[u32]) -> Result<Judged, EarScoreError> {
    let onsets = &key.onsets_ms;
    if onsets.is_empty() || onsets.len() > MAX_ONSETS || key.window_ms == 0 {
        return Err(EarScoreError::BadKey {
            reason: format!(
                "a taps key with {} onsets (1 to {MAX_ONSETS}) and a window of {} ms",
                onsets.len(),
                key.window_ms
            ),
        });
    }
    if onsets.windows(2).any(|w| w[0] >= w[1]) {
        return Err(EarScoreError::BadKey {
            reason: "a taps key's onsets are not strictly increasing".to_owned(),
        });
    }
    if taps.len() > MAX_TAPS {
        return Err(EarScoreError::BadAnswer {
            reason: format!("{} taps (at most {MAX_TAPS})", taps.len()),
        });
    }
    if taps.windows(2).any(|w| w[0] > w[1]) {
        return Err(EarScoreError::BadAnswer {
            reason: "the taps are not in time order".to_owned(),
        });
    }
    let n = onsets.len();
    let mut best = Fit {
        hits: 0,
        error: 0,
        offset: 0,
        tempo: 1000,
    };
    let window = i64::from(key.window_ms);
    let mut predicted = vec![0i64; n];
    'fit: for &anchor in onsets {
        for &tap in taps {
            for &tempo in &TAPS_TEMPI {
                for (x, p) in predicted.iter_mut().enumerate() {
                    let d = i64::from(onsets[x]) - i64::from(anchor);
                    *p = i64::from(tap) + div_round(d * tempo, 1000);
                }
                let (hits, error) = best_match(&predicted, taps, window);
                if hits > best.hits || (hits == best.hits && hits > 0 && error < best.error) {
                    best = Fit {
                        hits,
                        error,
                        offset: i64::from(tap) - i64::from(anchor),
                        tempo,
                    };
                    if hits == n && error == 0 {
                        // Nothing fits better, and the first best fit is the one kept.
                        break 'fit;
                    }
                }
            }
        }
    }
    let hits = best.hits;
    let misses = n - hits;
    let extras = taps.len() - hits;
    let percent = 100.0 * hits as f64 / (n + extras) as f64;
    let judged = TapsJudged {
        hits: hits as u16,
        misses: misses as u16,
        extras: extras as u16,
        error_ms: best.error,
        offset_ms: best.offset,
        tempo_permille: best.tempo as u16,
    };
    Ok(Judged {
        right: misses == 0 && extras == 0,
        percent,
        detail: Detail::Taps(judged),
    })
}

/// A candidate fit of `taps/1`.
struct Fit {
    hits: usize,
    error: u64,
    offset: i64,
    tempo: i64,
}

/// `a / b` rounded half away from zero (`b` > 0).
fn div_round(a: i64, b: i64) -> i64 {
    if a >= 0 {
        (a + b / 2) / b
    } else {
        -((-a + b / 2) / b)
    }
}

/// The one-to-one monotone match of predicted onsets and taps that hits the most onsets within `window`, then has the
/// least summed absolute error: a longest-common-subsequence table over (hits, −error).
fn best_match(predicted: &[i64], taps: &[u32], window: i64) -> (usize, u64) {
    let m = taps.len();
    // `row[y]` is the best (hits, error) for the onsets so far against the first `y` taps.
    let mut prev = vec![(0usize, 0u64); m + 1];
    let mut row = vec![(0usize, 0u64); m + 1];
    for &p in predicted {
        row[0] = (0, 0);
        for y in 1..=m {
            let mut cell = better(prev[y], row[y - 1]);
            let d = (p - i64::from(taps[y - 1])).unsigned_abs();
            if d <= window.unsigned_abs() {
                let (h, e) = prev[y - 1];
                cell = better(cell, (h + 1, e + d));
            }
            row[y] = cell;
        }
        core::mem::swap(&mut prev, &mut row);
    }
    prev[m]
}

/// The better of two (hits, error) pairs: more hits, then less error; the first on a tie.
fn better(a: (usize, u64), b: (usize, u64)) -> (usize, u64) {
    if b.0 > a.0 || (b.0 == a.0 && b.1 < a.1) {
        b
    } else {
        a
    }
}

/// A set judged: per question, and the whole.
#[derive(Clone, Debug, PartialEq)]
pub struct EarScore {
    /// The scorer that judged it.
    pub scorer: ScorerId,
    /// Each question, in order.
    pub items: Vec<Judged>,
    /// How many are right.
    pub right: u16,
    /// The mean of the items' percentages, in question order.
    pub percent: f64,
    /// Whether the percentage reaches the challenge's pass mark.
    pub passed: bool,
}

/// Judge a set with the challenge's scoring: every question's key against its answer (`None`: not answered, wrong at
/// 0 %), with the scorer in the `sound` slot (an ear challenge repeats it in every slot), and the pass mark. No render.
pub fn judge_set(
    scoring: &ScoringSpec,
    keys: &[EarKey],
    answers: &[Option<EarAnswer>],
) -> Result<EarScore, EarScoreError> {
    if keys.len() != answers.len() || keys.is_empty() {
        return Err(EarScoreError::Count {
            keys: keys.len(),
            answers: answers.len(),
        });
    }
    let id = &scoring.sound;
    let mut items = Vec::with_capacity(keys.len());
    for (key, answer) in keys.iter().zip(answers) {
        items.push(if let Some(a) = answer {
            judge(id, key, a)?
        } else {
            if !EAR_SCORERS.contains(id) {
                return Err(EarScoreError::UnknownScorer { id: id.clone() });
            }
            Judged {
                right: false,
                percent: 0.0,
                detail: Detail::Unanswered,
            }
        });
    }
    let right = items.iter().filter(|j| j.right).count() as u16;
    let percent = items.iter().map(|j| j.percent).sum::<f64>() / items.len() as f64;
    Ok(EarScore {
        scorer: id.clone(),
        right,
        percent,
        passed: scoring.passes(percent),
        items,
    })
}

/// Whether `id` is an ear scorer this build has and judges a key's own right answer as right, on one key of its kind
/// (the round's known-scorers test).
pub fn judges_its_own_key(id: &ScorerId) -> bool {
    let key = match answer_of(id) {
        Some(AnswerKind::Choice) => EarKey::Choice {
            index: 1,
            options: 3,
        },
        Some(AnswerKind::Value) => EarKey::Value(ValueKey::new(ValueScale::Octaves, 440.0, 250)),
        Some(AnswerKind::Taps) => EarKey::Taps(TapsKey {
            onsets_ms: vec![0, 250, 500, 875],
            window_ms: 60,
        }),
        _ => return false,
    };
    judge(id, &key, &key.right_answer()).is_ok_and(|j| j.right && j.percent == 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn choice(index: u8, options: u8) -> EarKey {
        EarKey::Choice { index, options }
    }

    fn taps(onsets: &[u32], window: u16) -> EarKey {
        EarKey::Taps(TapsKey {
            onsets_ms: onsets.to_vec(),
            window_ms: window,
        })
    }

    fn tapped(key: &EarKey, t: &[u32]) -> TapsJudged {
        let j = judge(&ScorerId::TAPS_V1, key, &EarAnswer::Taps(t.to_vec())).unwrap();
        let Detail::Taps(d) = j.detail else {
            panic!("{j:?}")
        };
        d
    }

    #[test]
    fn the_ids_are_the_documented_ones() {
        assert_eq!(ScorerId::EAR_V1.to_string(), "ear/1");
        assert_eq!(ScorerId::EAR_VALUE_V1.to_string(), "ear-value/1");
        assert_eq!(ScorerId::TAPS_V1.to_string(), "taps/1");
        for id in EAR_SCORERS {
            assert!(crate::score::is_known(&id), "{id}");
            assert!(id.is_valid());
            assert!(judges_its_own_key(&id), "{id}");
            assert_eq!(scorer_for(answer_of(&id).unwrap()), Some(id));
        }
        assert_eq!(scorer_for(AnswerKind::Patch), None);
        assert!(!judges_its_own_key(&ScorerId::SOUND_V1));
    }

    #[test]
    fn a_choice_is_judged_exactly() {
        let key = choice(2, 4);
        for a in 0..4 {
            let j = judge(&ScorerId::EAR_V1, &key, &EarAnswer::Choice(a)).unwrap();
            assert_eq!(j.right, a == 2);
            assert_eq!(j.percent, if a == 2 { 100.0 } else { 0.0 });
        }
        let e = judge(&ScorerId::EAR_V1, &key, &EarAnswer::Choice(4)).unwrap_err();
        assert_eq!(e.code(), "bad_answer");
        for bad in [choice(0, 1), choice(3, 3)] {
            let e = judge(&ScorerId::EAR_V1, &bad, &EarAnswer::Choice(0)).unwrap_err();
            assert_eq!(e.code(), "bad_key", "{bad:?}");
        }
    }

    #[test]
    fn kinds_and_ids_must_agree() {
        let key = choice(0, 2);
        for (id, answer) in [
            (ScorerId::EAR_VALUE_V1, EarAnswer::Choice(0)),
            (ScorerId::EAR_V1, EarAnswer::Value(1.0)),
            (ScorerId::TAPS_V1, EarAnswer::Taps(vec![0])),
        ] {
            assert_eq!(judge(&id, &key, &answer).unwrap_err().code(), "bad_answer");
        }
        for id in [
            ScorerId::new("ear", 2),
            ScorerId::SOUND_V1,
            ScorerId::new("taps", 9),
        ] {
            assert_eq!(
                judge(&id, &key, &EarAnswer::Choice(0)).unwrap_err().code(),
                "unknown_scorer",
                "{id}"
            );
        }
    }

    #[test]
    fn a_value_is_judged_in_perceptual_units() {
        // A quarter of an octave around 1 kHz.
        let key = EarKey::Value(ValueKey::new(ValueScale::Octaves, 1000.0, 250));
        let at = |v: f32| judge(&ScorerId::EAR_VALUE_V1, &key, &EarAnswer::Value(v)).unwrap();
        assert!(at(1000.0).right);
        assert!(at(1180.0).right); // 0.239 oct
        assert!(at(848.0).right); // 0.238 oct
        assert!(!at(1200.0).right); // 0.263 oct
        assert!(!at(500.0).right);
        let Detail::Value { error, .. } = at(2000.0).detail else {
            panic!()
        };
        assert_eq!(error, 1.0);
        assert_eq!(
            judge(&ScorerId::EAR_VALUE_V1, &key, &EarAnswer::Value(0.0))
                .unwrap_err()
                .code(),
            "bad_answer"
        );
        assert_eq!(
            judge(&ScorerId::EAR_VALUE_V1, &key, &EarAnswer::Value(f32::NAN))
                .unwrap_err()
                .code(),
            "bad_answer"
        );
        // Times: doublings with a 1 ms floor; 0 ms is a valid answer.
        let ms = EarKey::Value(ValueKey::new(ValueScale::LogMs, 99.0, 500));
        let at = |v: f32| judge(&ScorerId::EAR_VALUE_V1, &ms, &EarAnswer::Value(v)).unwrap();
        assert!(at(140.0).right); // log2(141/100) = 0.496
        assert!(!at(150.0).right);
        assert!(!at(0.0).right);
        // Linear units: dB.
        let db = EarKey::Value(ValueKey::new(ValueScale::Linear, -6.0, 1500));
        let at = |v: f32| judge(&ScorerId::EAR_VALUE_V1, &db, &EarAnswer::Value(v)).unwrap();
        assert!(at(-4.5).right);
        assert!(!at(-4.0).right);
        // A key its scale cannot measure is the generator's bug, not a wrong answer.
        let bad = EarKey::Value(ValueKey::new(ValueScale::Octaves, -1.0, 100));
        assert_eq!(
            judge(&ScorerId::EAR_VALUE_V1, &bad, &EarAnswer::Value(1.0))
                .unwrap_err()
                .code(),
            "bad_key"
        );
    }

    #[test]
    fn taps_cancel_a_constant_latency_and_a_small_tempo_change() {
        let key = taps(&[0, 250, 500, 875, 1000, 1500], 40);
        // Exact, then late by any constant: right.
        for late in [0u32, 33, 120, 5000] {
            let t: Vec<u32> = [0u32, 250, 500, 875, 1000, 1500]
                .iter()
                .map(|x| x + late)
                .collect();
            let d = tapped(&key, &t);
            assert_eq!((d.hits, d.misses, d.extras), (6, 0, 0), "{late}");
            assert_eq!(d.error_ms, 0, "{late}");
        }
        // 8 % slower: every onset still hit after the tempo fit.
        let slow: Vec<u32> = [0u32, 250, 500, 875, 1000, 1500]
            .iter()
            .map(|x| 300 + x * 108 / 100)
            .collect();
        let d = tapped(&key, &slow);
        assert_eq!((d.hits, d.misses, d.extras), (6, 0, 0), "{d:?}");
        assert!(d.tempo_permille >= 1050, "{d:?}");
        // 25 % slower is out of the tempo range: not every onset can be hit.
        let far: Vec<u32> = [0u32, 250, 500, 875, 1000, 1500]
            .iter()
            .map(|x| x * 125 / 100)
            .collect();
        assert!(tapped(&key, &far).hits < 6);
        // Jitter inside the window is fine, and counted.
        let j = tapped(&key, &[10, 230, 515, 880, 990, 1530]);
        assert_eq!(j.hits, 6, "{j:?}");
        assert!(j.error_ms > 0);
    }

    #[test]
    fn misses_and_extras_count_against() {
        let key = taps(&[0, 500, 1000, 1500], 50);
        let all = judge(
            &ScorerId::TAPS_V1,
            &key,
            &EarAnswer::Taps(vec![0, 500, 1000, 1500]),
        )
        .unwrap();
        assert!(all.right);
        assert_eq!(all.percent, 100.0);
        let missed = judge(
            &ScorerId::TAPS_V1,
            &key,
            &EarAnswer::Taps(vec![0, 500, 1500]),
        )
        .unwrap();
        assert!(!missed.right);
        assert_eq!(missed.percent, 75.0);
        let extra = judge(
            &ScorerId::TAPS_V1,
            &key,
            &EarAnswer::Taps(vec![0, 250, 500, 1000, 1500]),
        )
        .unwrap();
        assert!(!extra.right);
        assert_eq!(extra.percent, 80.0); // 4 hits / (4 onsets + 1 extra)
        let none = judge(&ScorerId::TAPS_V1, &key, &EarAnswer::Taps(Vec::new())).unwrap();
        assert_eq!((none.right, none.percent), (false, 0.0));
        // One tap matches at most one onset.
        let one = tapped(&key, &[500]);
        assert_eq!((one.hits, one.misses, one.extras), (1, 3, 0));
        // Unordered or too many taps, and damaged keys, are refused.
        assert_eq!(
            judge(&ScorerId::TAPS_V1, &key, &EarAnswer::Taps(vec![500, 0]))
                .unwrap_err()
                .code(),
            "bad_answer"
        );
        assert_eq!(
            judge(
                &ScorerId::TAPS_V1,
                &key,
                &EarAnswer::Taps(vec![0; MAX_TAPS + 1])
            )
            .unwrap_err()
            .code(),
            "bad_answer"
        );
        for bad in [taps(&[], 50), taps(&[0, 0], 50), taps(&[0, 10], 0)] {
            assert_eq!(
                judge(&ScorerId::TAPS_V1, &bad, &EarAnswer::Taps(vec![0]))
                    .unwrap_err()
                    .code(),
                "bad_key"
            );
        }
    }

    #[test]
    fn a_set_is_the_mean_of_its_items_and_passes_at_the_mark() {
        let scoring = ScoringSpec::ear(AnswerKind::Choice).unwrap();
        let keys: Vec<EarKey> = (0..10).map(|i| choice(i % 3, 3)).collect();
        let mut answers: Vec<Option<EarAnswer>> =
            keys.iter().map(|k| Some(k.right_answer())).collect();
        let all = judge_set(&scoring, &keys, &answers).unwrap();
        assert_eq!((all.right, all.percent, all.passed), (10, 100.0, true));
        answers[0] = None;
        answers[1] = Some(EarAnswer::Choice(2));
        let eight = judge_set(&scoring, &keys, &answers).unwrap();
        assert_eq!((eight.right, eight.percent, eight.passed), (8, 80.0, true));
        assert_eq!(eight.items[0].detail, Detail::Unanswered);
        answers[2] = None;
        let seven = judge_set(&scoring, &keys, &answers).unwrap();
        assert!(!seven.passed);
        assert_eq!(
            judge_set(&scoring, &keys, &answers[..9])
                .unwrap_err()
                .code(),
            "bad_answer"
        );
        assert_eq!(
            judge_set(&scoring, &[], &[]).unwrap_err().code(),
            "bad_answer"
        );
    }

    proptest! {
        /// The right answer is always right, at 100 %, for every key of every kind.
        #[test]
        fn the_right_answer_is_right(index in 0u8..6, extra in 1u8..5, hz in 20.0f32..20000.0, tol in 1u32..2000,
                                     gaps in proptest::collection::vec(1u32..800, 1..16), late in 0u32..10_000,
                                     window in 1u16..100) {
            let options = index + extra + 1;
            let mut onsets = vec![0u32];
            for g in &gaps {
                onsets.push(onsets.last().unwrap() + g);
            }
            let keys = [
                EarKey::Choice { index, options },
                EarKey::Value(ValueKey::new(ValueScale::Octaves, hz, tol)),
                EarKey::Value(ValueKey::new(ValueScale::LogMs, hz, tol)),
                EarKey::Value(ValueKey::new(ValueScale::Linear, -hz, tol)),
                EarKey::Taps(TapsKey { onsets_ms: onsets.clone(), window_ms: window }),
            ];
            for key in keys {
                let id = scorer_for(key.answer_kind()).unwrap();
                let j = judge(&id, &key, &key.right_answer()).unwrap();
                prop_assert!(j.right && j.percent == 100.0, "{key:?}: {j:?}");
            }
            // Tapping the onsets late by a constant changes nothing.
            let key = EarKey::Taps(TapsKey { onsets_ms: onsets.clone(), window_ms: window });
            let shifted = EarAnswer::Taps(onsets.iter().map(|x| x + late).collect());
            let j = judge(&ScorerId::TAPS_V1, &key, &shifted).unwrap();
            prop_assert!(j.right, "{j:?}");
        }

        /// Judging never panics, and a percentage is always within 0-100.
        #[test]
        fn judging_is_total(onsets in proptest::collection::vec(0u32..5000, 0..12),
                            t in proptest::collection::vec(0u32..5000, 0..24), window in 0u16..200,
                            v in proptest::num::f32::ANY, k in proptest::num::f32::ANY) {
            let mut onsets = onsets;
            onsets.sort_unstable();
            onsets.dedup();
            let mut t = t;
            t.sort_unstable();
            let key = EarKey::Taps(TapsKey { onsets_ms: onsets, window_ms: window });
            if let Ok(j) = judge(&ScorerId::TAPS_V1, &key, &EarAnswer::Taps(t)) {
                prop_assert!((0.0..=100.0).contains(&j.percent));
            }
            let key = EarKey::Value(ValueKey::new(ValueScale::Octaves, k, 250));
            if let Ok(j) = judge(&ScorerId::EAR_VALUE_V1, &key, &EarAnswer::Value(v)) {
                prop_assert!(j.percent == 0.0 || j.percent == 100.0);
            }
        }
    }
}
