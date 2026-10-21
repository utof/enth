//! **Owner: lane PLY** (choice checks; docs/learn.md, "Sound and choice checks (lane PLY)").
//!
//! **The audibility gate of an ear question** (`StepKind::Choose`): its options must be pairwise at least
//! [`MIN_JND`] just-noticeable steps apart under `sound/1`'s calibration, or the question cannot be answered by ear
//! (report 03 §9; PLAN.md §1.2). `lesson-check --solve` (lane LCK) calls [`choice_audibility`] on every Choose step
//! and refuses a step that does not [`pass`](Audibility::passes).
//!
//! **How a pair is measured.** For options `a` and `b`, `a` is made the target of the round `enth compare --target a
//! --phrase N b` plays (`prepare_patch_round` over the starter template, the compared parameters those `a` moved off
//! their defaults), and `b` is scored against it: `sound/1`'s distance in JND units, net of `a`'s nuisance floor,
//! with its pitch and waveform guard. Then the other way round. The pair's distance is the **smaller** of the two
//! (the harder direction decides). `sound/1` is loudness-normalised, as the session plays level-matched options;
//! when **both** options switch the level match off (lessons 0.1 and 7.3, where the level is what is taught), their
//! loudness difference is added in units of [`LEVEL_STEP_DB`] (2 dB, `sound/1`'s calibration step for levels).
//!
//! **What it cannot measure.** `sound/1` renders a phrase from the table at the scoring seed, untransposed and with no
//! control curve. A pair whose options differ in phrase, or where either has a transpose, a render seed or a control
//! curve, is reported [`PairJnd::NotMeasured`] with the reason, and a step with such a pair does not pass the gate on
//! its own: `lesson-check` lists it for a listening review (or "taught, not checked", Q9).
//!
//! **Cost.** Each option in a measurable pair is prepared once (`sound/1`'s 19 renders: the target, two floor
//! renders, 16 jittered copies), and each ordered pair scores once (one render): about 0.5 s an option and 30 ms a
//! pair in a release build. An authoring-time check, never run in the plugin.

use std::collections::BTreeMap;
use std::sync::Arc;

use enth_challenge::patch_round::{Compared, prepare_patch_round};
use enth_challenge::score::round::target_lufs;
use enth_challenge::v2::InlinePatch;
use enth_challenge::{Challenge, PreparedRound, ScoringSpec, Start, Target, score_round};
use enth_patch::Patch;

use super::SoundError;
use crate::lesson::Example;
use crate::pack::LessonBundle;
use crate::player::Audition;

/// How far apart, in `sound/1` JND units, every two options of an ear question must be (PLAN.md §1.2: "at least 2
/// just-noticeable steps apart").
pub const MIN_JND: f64 = 2.0;

/// The loudness difference that counts as one just-noticeable step between options played without the level match,
/// in dB (`sound/1`'s calibration step for levels and dB values).
pub const LEVEL_STEP_DB: f64 = 2.0;

/// How far apart two options are.
#[derive(Clone, Debug, PartialEq)]
pub enum PairJnd {
    /// The distance in `sound/1` JND units: the smaller of the two directions, plus the level term when both play
    /// unmatched.
    Measured(f64),
    /// `sound/1` cannot render one of them as it is played; the reason, for `lesson-check`'s report.
    NotMeasured(&'static str),
}

/// Two options and their distance.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionPair {
    /// The first option's index.
    pub a: usize,
    /// The second option's index (`a < b`).
    pub b: usize,
    /// Their distance.
    pub jnd: PairJnd,
}

/// The distances of every pair of an ear question's options, in `(a, b)` order with `a < b`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audibility {
    /// Every pair.
    pub pairs: Vec<OptionPair>,
}

impl Audibility {
    /// Whether every pair was measured and is at least `k` JND apart (the gate is `passes(MIN_JND)`).
    pub fn passes(&self, k: f64) -> bool {
        self.pairs
            .iter()
            .all(|p| matches!(p.jnd, PairJnd::Measured(j) if j >= k))
    }

    /// The closest measured pair, if any.
    pub fn closest(&self) -> Option<&OptionPair> {
        self.pairs
            .iter()
            .filter_map(|p| match p.jnd {
                PairJnd::Measured(j) => Some((p, j)),
                PairJnd::NotMeasured(_) => None,
            })
            .min_by(|(_, x), (_, y)| x.total_cmp(y))
            .map(|(p, _)| p)
    }

    /// The pairs that keep it from passing at `k`: measured closer than `k`, or not measured.
    pub fn failing(&self, k: f64) -> impl Iterator<Item = &OptionPair> {
        self.pairs
            .iter()
            .filter(move |p| !matches!(p.jnd, PairJnd::Measured(j) if j >= k))
    }
}

/// The options of a Choose step (or the examples of a Listen step) as the player resolves them: each with its pack
/// patch. An example whose patch the bundle lacks is left out (the pack reader refuses such a pack).
pub fn auditions(bundle: &LessonBundle, examples: &[Example]) -> Vec<Audition> {
    examples
        .iter()
        .filter_map(|e| {
            Some(Audition {
                label: e.label.clone(),
                patch: Arc::clone(bundle.patch(&e.patch)?),
                phrase: e.phrase,
                transpose: e.transpose,
                level_match: e.level_match,
                seed: e.seed,
                curve: e.curve.clone(),
            })
        })
        .collect()
}

/// Measure every pair of `options` (see the [module docs](self)). Deterministic: the same distances on every
/// platform (`sound/1` runs on the scalar FFT path and the in-house loudness meter).
///
/// # Errors
/// [`SoundError`] when an option's round cannot be prepared or scored (its patch does not render).
pub fn choice_audibility(options: &[Audition]) -> Result<Audibility, SoundError> {
    let mut rounds: Vec<Option<PreparedRound>> = options.iter().map(|_| None).collect();
    let mut loudness: BTreeMap<usize, Option<f64>> = BTreeMap::new();
    let mut pairs = Vec::new();
    for a in 0..options.len() {
        for b in a + 1..options.len() {
            let (x, y) = (&options[a], &options[b]);
            let jnd = if let Some(why) = not_measurable(x, y) {
                PairJnd::NotMeasured(why)
            } else {
                let ab = jnd_against(round_of(&mut rounds, options, a)?, &y.patch)?;
                let ba = jnd_against(round_of(&mut rounds, options, b)?, &x.patch)?;
                let mut jnd = ab.min(ba);
                if !x.level_match && !y.level_match {
                    let la = lufs_of(&mut loudness, options, a)?;
                    let lb = lufs_of(&mut loudness, options, b)?;
                    jnd += level_jnd(la, lb);
                }
                PairJnd::Measured(jnd)
            };
            pairs.push(OptionPair { a, b, jnd });
        }
    }
    Ok(Audibility { pairs })
}

/// Why `sound/1` cannot compare two options as they are played, if it cannot.
fn not_measurable(x: &Audition, y: &Audition) -> Option<&'static str> {
    if x.phrase != y.phrase {
        return Some("the options play different phrases");
    }
    if x.transpose != 0 || y.transpose != 0 {
        return Some("an option is transposed");
    }
    if x.seed.is_some() || y.seed.is_some() {
        return Some("an option has its own render seed");
    }
    if x.curve.is_some() || y.curve.is_some() {
        return Some("an option plays a control curve");
    }
    None
}

/// The round with option `i` as its target, prepared once.
fn round_of<'r>(
    rounds: &'r mut [Option<PreparedRound>],
    options: &[Audition],
    i: usize,
) -> Result<&'r PreparedRound, SoundError> {
    if rounds[i].is_none() {
        let o = &options[i];
        let challenge = Challenge::new(
            Target::Patch(InlinePatch::new(Patch::clone(&o.patch))),
            Start::Template { template: 0 },
            o.phrase,
            ScoringSpec::recreate(0),
        );
        let round = prepare_patch_round(challenge, Compared::OffDefaults).map_err(|e| {
            SoundError::Score {
                message: format!("option {:?}: {e}", o.label),
            }
        })?;
        rounds[i] = Some(round);
    }
    Ok(rounds[i]
        .as_ref()
        .unwrap_or_else(|| unreachable!("prepared above")))
}

/// `sound/1`'s distance of `answer` to the round's target, in JND units.
fn jnd_against(round: &PreparedRound, answer: &Patch) -> Result<f64, SoundError> {
    score_round(round, answer)
        .map(|r| r.sound.jnd)
        .map_err(|e| SoundError::Score {
            message: e.to_string(),
        })
}

/// Option `i`'s integrated loudness through its phrase (`None` when silent), measured once.
fn lufs_of(
    cache: &mut BTreeMap<usize, Option<f64>>,
    options: &[Audition],
    i: usize,
) -> Result<Option<f64>, SoundError> {
    if let Some(l) = cache.get(&i) {
        return Ok(*l);
    }
    let o = &options[i];
    let phrase = enth_challenge::phrases::phrase(o.phrase)
        .ok_or(SoundError::UnknownPhrase { phrase: o.phrase.0 })?;
    let l = target_lufs(&o.patch, phrase).map_err(|e| SoundError::Score {
        message: e.to_string(),
    })?;
    cache.insert(i, l);
    Ok(l)
}

/// The loudness difference in JND units: `|Δ LUFS| / LEVEL_STEP_DB`; a silent option against a sounding one is
/// infinitely far, two silent ones are the same.
fn level_jnd(a: Option<f64>, b: Option<f64>) -> f64 {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() / LEVEL_STEP_DB,
        (None, None) => 0.0,
        _ => f64::INFINITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_challenge::phrases::HELD;
    use enth_patch::{Op, ParamAddr};

    fn option(label: &str, patch: Patch) -> Audition {
        Audition {
            label: label.to_owned(),
            patch: Arc::new(patch),
            phrase: HELD,
            transpose: 0,
            level_match: true,
            seed: None,
            curve: None,
        }
    }

    fn with(addr: &str, value: f32) -> Patch {
        let mut p = Patch::init();
        enth_patch::apply(
            &mut p,
            Op::SetParam {
                addr: addr.parse::<ParamAddr>().unwrap(),
                value,
            },
        )
        .unwrap();
        p
    }

    #[test]
    fn distinct_waves_pass_and_twins_do_not() {
        let saw = option("saw", Patch::init());
        let sine = option("sine", with("osc1.wave", 0.0));
        let twin = option("saw again", Patch::init());
        let a = choice_audibility(&[saw.clone(), sine.clone()]).unwrap();
        assert_eq!(a.pairs.len(), 1);
        assert!(a.passes(MIN_JND), "{a:?}");
        let b = choice_audibility(&[saw, sine, twin]).unwrap();
        assert_eq!(b.pairs.len(), 3);
        assert!(!b.passes(MIN_JND), "two identical options");
        let closest = b.closest().unwrap();
        assert_eq!((closest.a, closest.b), (0, 2));
        assert_eq!(b.failing(MIN_JND).count(), 1);
        assert_eq!(choice_audibility(&[]).unwrap(), Audibility::default());
    }

    #[test]
    fn level_counts_only_without_the_level_match() {
        let loud = with("master.level", 0.0);
        let quiet = with("master.level", -12.0);
        let matched =
            choice_audibility(&[option("loud", loud.clone()), option("quiet", quiet.clone())])
                .unwrap();
        assert!(
            !matched.passes(MIN_JND),
            "level-matched, a level change is inaudible: {matched:?}"
        );
        let mut a = option("loud", loud);
        let mut b = option("quiet", quiet);
        a.level_match = false;
        b.level_match = false;
        let unmatched = choice_audibility(&[a, b]).unwrap();
        let PairJnd::Measured(j) = unmatched.pairs[0].jnd else {
            panic!("measured")
        };
        assert!(j >= 12.0 / LEVEL_STEP_DB - 1.0, "about 6 JND of level: {j}");
        assert!(unmatched.passes(MIN_JND));
    }

    #[test]
    fn what_sound_1_cannot_play_is_not_measured() {
        let mut low = option("low", Patch::init());
        low.transpose = -12;
        let high = option("high", Patch::init());
        let a = choice_audibility(&[low, high]).unwrap();
        assert!(matches!(a.pairs[0].jnd, PairJnd::NotMeasured(_)));
        assert!(!a.passes(MIN_JND));
        assert_eq!(level_jnd(None, Some(-20.0)), f64::INFINITY);
        assert_eq!(level_jnd(None, None), 0.0);
    }
}
