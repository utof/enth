//! **Owner: lane PLY** (sound checks; docs/learn.md, "Sound and choice checks (lane PLY)").
//!
//! A sound check is evaluated by a pure function the shell's worker calls: [`evaluate_sounds`] renders the patch (with
//! each check's macro pins applied, [`SoundCheck::pinned`]) playing the check's phrase, describes the mid signal and
//! judges the expectation items with `enth-analysis`'s checker. It takes the patch of a
//! [`CheckRequest`](crate::player::CheckRequest) and its `sounds`, and its [`CheckVerdict`] goes back to the player
//! through `Player::check_done` with the request's revision.
//!
//! **The seam.** `enth-render` and `enth-analysis` sit at L3, below this crate (L5), so the render happens here and
//! the shell passes nothing in: the plugin's worker, `enth lesson check` and `lesson-check` all get the same verdict
//! for the same patch. The steps are the ones `enth_solver::measure` takes (this crate cannot call its same-layer
//! sibling): render at [`SCORING_SAMPLE_RATE`] and [`SCORING_SEED`], take the mid channel `(L + R) / 2`, describe it
//! on the scalar FFT path with the first note's timing, computing only the descriptor groups the items need
//! ([`Want::for_features`]). So a check reads what `enth check --phrase N` reads at the default seed.
//!
//! **Cost.** [`evaluate_sounds`] renders once per sound check (3–5 s of audio, about 15–30 ms), the plugin's budget.
//! [`evaluate_sounds_report`] with [`SoundOptions::nuisance`] adds `enth check`'s two nuisance re-renders per sound
//! check, so each item also carries its noise and the `thin` verdict (an inaudible change could flip it), which
//! `lesson-check` needs for its margin rule; the pass/fail verdict is the judged render's either way.

use enth_analysis::descriptors::{AnalysisError, Want};
use enth_analysis::expect::{CheckReport, Expect, parse_item};
use enth_analysis::{Describer, Descriptors, Feature, NoteTiming};
use enth_challenge::PhraseId;
use enth_challenge::model::{SCORING_SAMPLE_RATE, SCORING_SEED};
use enth_patch::Patch;
use enth_render::{Performance, RenderSpec, render};

use super::SoundCheck;
use crate::player::{CheckVerdict, ItemVerdict};

/// The offsets from the judged seed to the two nuisance render seeds: the ones `enth check` uses (`enth-cli`'s
/// evaluator), so a nuisance-checked item reads the same noise and `thin` verdict there and here. Oscillators in
/// `free` phase mode take their start phases from the render seed, so a nuisance render is the same sound with other
/// phases.
pub const NUISANCE_OFFSETS: [u64; 2] = [0x9E37_79B9_7F4A_7C15, 0xC2B2_AE3D_27D4_EB4F];

/// How [`evaluate_sounds_report`] measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SoundOptions {
    /// Also render each check at the two nuisance seeds ([`NUISANCE_OFFSETS`]), so every item carries its noise and
    /// `thin` verdict. Three renders per sound check instead of one.
    pub nuisance: bool,
}

/// One sound check's full result: `enth-analysis`'s report on the judged render, items in the check's order.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundReport {
    /// The phrase it played.
    pub phrase: PhraseId,
    /// The items as written, in order (the report's `results` echo them in canonical form).
    pub expect: Vec<String>,
    /// Per item: pass, value, margin, and with nuisance renders the noise and `thin`.
    pub report: CheckReport,
}

/// Why a sound check could not be evaluated. A check that ran and failed is a verdict, not an error.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SoundError {
    /// The phrase is not in this build (a lesson's checks are validated when the pack loads, so only a hand-made
    /// request meets it).
    UnknownPhrase {
        /// The id.
        phrase: u16,
    },
    /// An expectation item does not parse (also refused when the pack loads).
    BadExpect {
        /// The item.
        item: String,
        /// The parser's message.
        message: String,
    },
    /// The patch does not render.
    Render {
        /// `enth-render`'s code.
        code: &'static str,
        /// Its message.
        message: String,
    },
    /// The analysis refused the rendered signal.
    Analysis {
        /// Its message.
        message: String,
    },
}

impl SoundError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            SoundError::UnknownPhrase { .. } => "unknown_phrase",
            SoundError::BadExpect { .. } => "bad_expect",
            SoundError::Render { .. } => "render_failed",
            SoundError::Analysis { .. } => "analysis_failed",
        }
    }

    /// What to do about it, in one line. Whatever the error, answer the player with a failed verdict
    /// (`CheckVerdict::default()`), so the step does not wait on a check that never comes.
    pub fn hint(&self) -> String {
        match self {
            SoundError::UnknownPhrase { .. } | SoundError::BadExpect { .. } => {
                "the lesson's check is broken: run `cargo xtask lesson-check`".to_owned()
            }
            SoundError::Render { .. } | SoundError::Analysis { .. } => {
                "the patch could not be heard; change it and the check runs again".to_owned()
            }
        }
    }
}

impl std::fmt::Display for SoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SoundError::UnknownPhrase { phrase } => {
                write!(f, "phrase {phrase} is not in this build")
            }
            SoundError::BadExpect { item, message } => write!(f, "expectation {item:?}: {message}"),
            SoundError::Render { message, .. } => write!(f, "render failed: {message}"),
            SoundError::Analysis { message } => write!(f, "analysis failed: {message}"),
        }
    }
}

impl std::error::Error for SoundError {}

/// Evaluate `sounds` on `patch`: each check renders `sound.pinned(patch)` through its phrase and judges its items
/// (see the [module docs](self)). The verdict passes when every item of every check holds on the judged render (an
/// empty list passes); its items come in order, each as written with its signed margin in the feature's unit. One
/// render per check, no nuisance renders.
///
/// # Errors
/// [`SoundError`] when a phrase or item is unknown, the patch does not render, or the analysis refuses the signal.
/// The caller still answers the player, with a failed verdict.
pub fn evaluate_sounds(sounds: &[SoundCheck], patch: &Patch) -> Result<CheckVerdict, SoundError> {
    let reports = evaluate_sounds_report(sounds, patch, SoundOptions::default())?;
    Ok(verdict(&reports))
}

/// [`evaluate_sounds`] with the full report per check, and optionally the nuisance renders (`lesson-check`'s margin
/// rule reads `thin` and the noise).
///
/// # Errors
/// As [`evaluate_sounds`].
pub fn evaluate_sounds_report(
    sounds: &[SoundCheck],
    patch: &Patch,
    options: SoundOptions,
) -> Result<Vec<SoundReport>, SoundError> {
    let mut describer = None;
    sounds
        .iter()
        .map(|sound| evaluate_one(sound, patch, options, &mut describer))
        .collect()
}

/// The player's verdict from full reports: every item of every check, in order.
pub fn verdict(reports: &[SoundReport]) -> CheckVerdict {
    let items: Vec<ItemVerdict> = reports
        .iter()
        .flat_map(|r| {
            r.report
                .results
                .iter()
                .zip(&r.expect)
                .map(|(res, item)| ItemVerdict {
                    item: item.clone(),
                    passed: res.pass,
                    margin: res.margin,
                })
        })
        .collect();
    CheckVerdict {
        passed: reports.iter().all(|r| r.report.pass),
        items,
    }
}

fn evaluate_one(
    sound: &SoundCheck,
    patch: &Patch,
    options: SoundOptions,
    describer: &mut Option<Describer>,
) -> Result<SoundReport, SoundError> {
    let phrase =
        enth_challenge::phrases::phrase(sound.phrase).ok_or(SoundError::UnknownPhrase {
            phrase: sound.phrase.0,
        })?;
    let items: Vec<Expect> = sound
        .expect
        .iter()
        .map(|item| {
            parse_item(item).map_err(|e| SoundError::BadExpect {
                item: item.clone(),
                message: e.message,
            })
        })
        .collect::<Result<_, _>>()?;
    let features: Vec<Feature> = items.iter().map(|e| e.feature).collect();
    let want = Want::for_features(&features);
    let perf = phrase.performance();
    let heard = sound.pinned(patch);
    let main = measure(&heard, &perf, SCORING_SEED, want, describer)?;
    let nuisance: Vec<Descriptors> = if options.nuisance {
        NUISANCE_OFFSETS
            .iter()
            .map(|o| {
                measure(
                    &heard,
                    &perf,
                    SCORING_SEED.wrapping_add(*o),
                    want,
                    describer,
                )
            })
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    Ok(SoundReport {
        phrase: sound.phrase,
        expect: sound.expect.clone(),
        report: enth_analysis::check(&items, &main, &nuisance),
    })
}

/// Render `patch` playing `perf` at the scoring rate and `seed`, and describe the mid signal with the first note's
/// timing: `enth_solver::measure`'s steps. `describer` is made on first use and kept across renders.
pub(crate) fn measure(
    patch: &Patch,
    perf: &Performance,
    seed: u64,
    want: Want,
    describer: &mut Option<Describer>,
) -> Result<Descriptors, SoundError> {
    let out = render(patch, perf, &RenderSpec::new(SCORING_SAMPLE_RATE, seed)).map_err(|e| {
        SoundError::Render {
            code: e.code(),
            message: e.to_string(),
        }
    })?;
    let d = describer.get_or_insert_with(|| Describer::new(f64::from(SCORING_SAMPLE_RATE)));
    d.describe_some(&out.mid(), note_timing(perf), want)
        .map_err(|e: AnalysisError| SoundError::Analysis {
            message: e.to_string(),
        })
}

/// The first note's timing (earliest on, ties in performance order), with its note-off: `enth_solver::note_timing`.
fn note_timing(perf: &Performance) -> NoteTiming {
    let first = perf
        .notes
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
        .map(|(_, n)| n);
    match first {
        Some(n) => NoteTiming {
            on_s: n.on_s,
            off_s: Some(n.off_s),
        },
        None => NoteTiming::unknown_off(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{MacroKnob, MacroPin, Plain};
    use enth_patch::MacroIndex;

    fn sound(expect: &[&str]) -> SoundCheck {
        SoundCheck {
            phrase: enth_challenge::phrases::HELD,
            expect: expect.iter().map(|s| (*s).to_owned()).collect(),
            macros: Vec::new(),
        }
    }

    #[test]
    fn a_bright_init_passes_brightness_above_and_fails_below() {
        let p = Patch::init();
        let v = evaluate_sounds(&[sound(&["brightness>500Hz"])], &p).unwrap();
        assert!(v.passed, "{v:?}");
        assert_eq!(v.items.len(), 1);
        assert_eq!(v.items[0].item, "brightness>500Hz");
        assert!(v.items[0].margin.is_some_and(|m| m > 0.0));
        let v = evaluate_sounds(
            &[sound(&["brightness>500Hz"]), sound(&["brightness<500Hz"])],
            &p,
        )
        .unwrap();
        assert!(!v.passed, "one item of the second check fails");
        assert_eq!(
            v.items.iter().map(|i| i.passed).collect::<Vec<_>>(),
            [true, false]
        );
        assert!(v.items[1].margin.is_some_and(|m| m < 0.0));
    }

    #[test]
    fn evaluation_is_deterministic_and_nuisance_only_adds_noise() {
        let p = Patch::init();
        let s = [sound(&["brightness>500Hz", "attack<50ms"])];
        let a = evaluate_sounds_report(&s, &p, SoundOptions::default()).unwrap();
        let b = evaluate_sounds_report(&s, &p, SoundOptions::default()).unwrap();
        assert_eq!(a, b);
        let n = evaluate_sounds_report(&s, &p, SoundOptions { nuisance: true }).unwrap();
        assert_eq!(
            verdict(&a),
            verdict(&n),
            "the verdict is the judged render's"
        );
        assert!(n[0].report.results.iter().all(|r| r.noise.is_some()));
        assert!(a[0].report.results.iter().all(|r| r.noise.is_none()));
    }

    #[test]
    fn macro_pins_change_what_the_check_hears() {
        // macro1 takes osc1.level down by its whole range: pinned at 1 the patch is silent.
        let mut p = Patch::init();
        let route: enth_patch::RouteAddr = "macro1 -> osc1.level".parse().unwrap();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::InsertRoute {
                index: 0,
                route: enth_patch::Route {
                    src: route.src,
                    dst: route.dst,
                    depth: -100.0,
                },
            },
        )
        .unwrap();
        let loud = sound(&["loudness>-30LUFS"]);
        let mut pinned = loud.clone();
        pinned.macros.push(MacroPin {
            knob: MacroKnob(MacroIndex::new(0).unwrap()),
            value: Plain::new("1"),
        });
        assert!(
            evaluate_sounds(std::slice::from_ref(&loud), &p)
                .unwrap()
                .passed
        );
        assert!(!evaluate_sounds(&[pinned], &p).unwrap().passed);
    }

    #[test]
    fn errors_are_named() {
        let p = Patch::init();
        let mut s = sound(&["brightness>500Hz"]);
        s.phrase = PhraseId(999);
        let e = evaluate_sounds(&[s], &p).unwrap_err();
        assert_eq!(e.code(), "unknown_phrase");
        let e = evaluate_sounds(&[sound(&["brightnes>1kHz"])], &p).unwrap_err();
        assert_eq!(e.code(), "bad_expect");
        assert_ne!(e.hint(), "");
        let v = evaluate_sounds(&[], &p).unwrap();
        assert!(v.passed && v.items.is_empty(), "nothing to hear passes");
    }
}
