//! **Owner: lane CV3** (Challenge v3, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/ear.md, "Rendering and playback", and docs/challenge.md, "Challenge v3".
//!
//! An ear target becomes questions ([`prepare`]): each says what to play ([`Clip`]s), what to offer
//! ([`EarOption`]s), its answer key ([`EarKey`]) and the Review skill it trains. The keys come from the generators in
//! parameter space and integers, never from a render, so a set is judged ([`EarSet::judge`]) with no render.
//!
//! **What a clip is.** A parameter game's clip is a patch heard through the held phrase (the base, or the base with one
//! control moved: [`super::params`]). A music game's clip is a [`Stimulus`]: a timbre from the frozen pool, the
//! question's own notes and length, and an optional [`EqBand`]. Every clip renders offline at the scoring rate and
//! seed ([`Clip::render`]), about 15 to 30 ms each, and plays through the Target slot. **No engine change.**
//!
//! **The timbre pool `timbres-v1/`** is our own patches, frozen and append-only: an id is an index into
//! [`TIMBRES_V1`], and a file's bytes never change (a test pins each file's SHA-256), so a new timbre is a new id at
//! the end. Each is the starter (template 0) with a few values changed, polyphony 8, filter 1 in place (the band-pass
//! band sets it):
//!
//! | Id | Name | What |
//! | --- | --- | --- |
//! | 0 | sine | one harmonic through an open filter |
//! | 1 | saw low-pass | a saw through a 1.5 kHz low-pass |
//! | 2 | pluck | a saw whose low-pass closes fast after the attack, a decaying amp |
//! | 3 | organ table | the organ wavetable at mid position |
//! | 4 | pad | two detuned saws, soft low-pass, slow attack and release |
//! | 5 | noise click | a 30 ms burst of band-passed white noise (rhythms) |
//! | 6 | pink noise | sustained pink noise through an open filter (the band games' source; lane MUS's Frequency finder) |
//!
//! **The band.** [`EqBand`] is integers (a kind, Hz, centi-dB and thousandths of Q), so a question stays exact. A
//! band-pass band is the synth's own filter, Frequency finder's svf form: it sets the timbre's filter 1 to band-pass at
//! the band's frequency, with the resonance whose Q it names (`Q = 0.5 · 50^(resonance/100)`, docs/dsp/svf.md),
//! and the engine renders it. A peaking or shelf band is applied after the engine by `enth-dsp`'s offline RBJ biquad
//! (lane BQ, planned in `enth-dsp/src/biquad.rs`): until BQ lands, [`Clip::render`] refuses such a clip
//! (`needs_biquad`) rather than play it without its band.
//!
//! **Level matching** (within 0.5 LU unless level is the cue, docs/ear.md) is playback's: [`crate::loudness`] measures
//! a rendered clip, and the shell sets the gains.

use core::fmt;

use enth_patch::{Op, ParamAddr, Patch};
use enth_render::{Performance, RenderOutput, RenderSpec};

use super::params::{self, DIAL_IT_IN, EAR_CHECK, EAR_CHECK_INTERVALS, SPOT_THE_CHANGE};
use super::scorers::{self, EarAnswer, EarKey, EarScore, EarScoreError};
use crate::model::{
    AnswerKind, Challenge, ChallengeError, PhraseId, PhraseNote, PhraseView, SCORING_SAMPLE_RATE,
    SCORING_SEED, ScoringSpec, Target,
};
use crate::v3::EarSpec;

/// A timbre id: an index into [`TIMBRES_V1`].
pub type TimbreId = u8;

/// Timbre 0: a sine.
pub const SINE: TimbreId = 0;
/// Timbre 1: a saw through a low-pass.
pub const SAW_LOW_PASS: TimbreId = 1;
/// Timbre 2: a pluck.
pub const PLUCK: TimbreId = 2;
/// Timbre 3: the organ wavetable.
pub const ORGAN: TimbreId = 3;
/// Timbre 4: a pad.
pub const PAD: TimbreId = 4;
/// Timbre 5: a noise-burst click.
pub const CLICK: TimbreId = 5;
/// Timbre 6: sustained pink noise.
pub const PINK_NOISE: TimbreId = 6;

/// The frozen, append-only timbre pool: (name, patch file), in id order.
pub const TIMBRES_V1: [(&str, &str); 7] = [
    ("sine", include_str!("timbres-v1/0-sine.patch.json")),
    (
        "saw low-pass",
        include_str!("timbres-v1/1-saw-lp.patch.json"),
    ),
    ("pluck", include_str!("timbres-v1/2-pluck.patch.json")),
    ("organ table", include_str!("timbres-v1/3-organ.patch.json")),
    ("pad", include_str!("timbres-v1/4-pad.patch.json")),
    ("noise click", include_str!("timbres-v1/5-click.patch.json")),
    ("pink noise", include_str!("timbres-v1/6-pink.patch.json")),
];

/// The lowest key a stimulus plays: C1 (MIDI 24).
pub const LOWEST_KEY: u8 = 24;
/// The highest key a stimulus plays: C7 (MIDI 96).
pub const HIGHEST_KEY: u8 = 96;
/// The most notes sounding at once.
pub const MAX_AT_ONCE: usize = 8;
/// The longest stimulus, in ms (the release tail included).
pub const MAX_LENGTH_MS: u32 = 10_000;
/// The most notes one stimulus holds.
pub const MAX_NOTES: usize = 64;

/// The timbre `id` as a patch, or `None` for an id this build's pool lacks (a newer build's).
pub fn timbre(id: TimbreId) -> Option<Patch> {
    let (_, text) = TIMBRES_V1.get(usize::from(id))?;
    // The pool's files load (tested); a failure here is a broken build, reported as a missing timbre.
    enth_patch::file::load(text).ok().map(|f| f.patch)
}

/// One EQ band of a question, in integers: **`enth-music`'s type** (the F1e review's M3: one `EqBand`, the one with
/// serde, which a question carries). The kind ([`EqKind`]), the centre (peak, band-pass) or corner (shelf) frequency,
/// 20 to 20,000 Hz; the gain in hundredths of a dB, ±2,400 (±24 dB), never 0 for a peak or a shelf (a flat band is no
/// band) and 0 for a band-pass; the quality factor in thousandths (1,414 is an octave-wide peak), 500 to 25,000 for a
/// band-pass (the svf's Q range), 100 to 25,000 otherwise. What this crate needs of it is [`BandExt`].
pub use enth_music::ear::{EqBand, EqKind};

/// What rendering needs of an [`EqBand`]: its values in real units, where it is applied, and the ranges above.
pub trait BandExt {
    /// The frequency in Hz.
    fn freq(&self) -> f32;
    /// The gain in dB (`gain_cdb / 100`, one IEEE division: the same bits everywhere).
    fn gain_db(&self) -> f32;
    /// The quality factor (`q_milli / 1000`).
    fn q(&self) -> f32;
    /// Whether the band is applied after the engine (a peak or a shelf: the biquad) rather than in the timbre.
    fn is_post(&self) -> bool;
    /// The svf resonance (%) whose Q is this band's: `100 · ln(2Q) / ln 50`, quantized as generator v1 quantizes
    /// (three significant digits), in 0 to 100.
    fn resonance(&self) -> f32;
    /// Check the ranges in [`EqBand`]'s docs.
    ///
    /// # Errors
    /// [`StimulusError::Band`] naming what is out of range.
    fn check(&self) -> Result<(), StimulusError>;
}

impl BandExt for EqBand {
    fn freq(&self) -> f32 {
        f32::from(self.freq_hz)
    }

    fn gain_db(&self) -> f32 {
        f32::from(self.gain_cdb) / 100.0
    }

    fn q(&self) -> f32 {
        f32::from(self.q_milli) / 1000.0
    }

    fn is_post(&self) -> bool {
        self.kind != EqKind::BandPass
    }

    fn resonance(&self) -> f32 {
        let q = f64::from(self.q_milli) / 1000.0;
        let r = 100.0 * libm::log(2.0 * q) / libm::log(50.0);
        crate::random::sig3(r.clamp(0.0, 100.0) as f32)
    }

    fn check(&self) -> Result<(), StimulusError> {
        let bad = |reason: String| Err(StimulusError::Band { reason });
        if !(20..=20_000).contains(&self.freq_hz) {
            return bad(format!("{} Hz is outside 20 Hz to 20 kHz", self.freq_hz));
        }
        if self.kind == EqKind::BandPass {
            if self.gain_cdb != 0 {
                return bad("a band-pass band has no gain".to_owned());
            }
            if !(500..=25_000).contains(&self.q_milli) {
                return bad(format!(
                    "a band-pass Q of {} is outside the svf's 0.5 to 25",
                    self.q()
                ));
            }
        } else {
            if self.gain_cdb == 0 || !(-2400..=2400).contains(&self.gain_cdb) {
                return bad(format!(
                    "a gain of {} dB (a peak or shelf boosts or cuts, at most 24 dB)",
                    self.gain_db()
                ));
            }
            if !(100..=25_000).contains(&self.q_milli) {
                return bad(format!("a Q of {} is outside 0.1 to 25", self.q()));
            }
        }
        Ok(())
    }
}

/// A music question's sound: a timbre, notes and a length, all integers, and an optional band.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stimulus {
    /// The timbre ([`TIMBRES_V1`]).
    pub timbre: TimbreId,
    /// The notes, in start order (key C1 to C7, velocity 1 to 127, start and hold in ms).
    pub notes: Vec<PhraseNote>,
    /// The render's length in ms, the release tail included (at most 10 s).
    pub length_ms: u32,
    /// The band, if any.
    pub band: Option<EqBand>,
}

impl Stimulus {
    /// Check what docs/ear.md promises of every generated question: a timbre this build has (`code_too_new`
    /// otherwise), 1 to 64 notes within C1 to C7 with velocities 1 to 127, each held at least 1 ms and ending within
    /// the length, in start order, never two at once on one key, at most 8 at once, at most 10 s; a valid band.
    pub fn validate(&self) -> Result<(), StimulusError> {
        if usize::from(self.timbre) >= TIMBRES_V1.len() {
            return Err(StimulusError::TimbreTooNew {
                timbre: self.timbre,
            });
        }
        let bad = |reason: String| Err(StimulusError::Notes { reason });
        if self.length_ms == 0 || self.length_ms > MAX_LENGTH_MS {
            return bad(format!(
                "a length of {} ms (1 to {MAX_LENGTH_MS})",
                self.length_ms
            ));
        }
        if self.notes.is_empty() || self.notes.len() > MAX_NOTES {
            return bad(format!("{} notes (1 to {MAX_NOTES})", self.notes.len()));
        }
        for (i, n) in self.notes.iter().enumerate() {
            if !(LOWEST_KEY..=HIGHEST_KEY).contains(&n.key) {
                return bad(format!("note {i}: key {} is outside C1 to C7", n.key));
            }
            if n.velocity == 0 || n.velocity > 127 {
                return bad(format!("note {i}: velocity {}", n.velocity));
            }
            let end = n.start_ms.checked_add(n.hold_ms);
            if n.hold_ms == 0 || end.is_none_or(|e| e > self.length_ms) {
                return bad(format!("note {i} does not end within the clip"));
            }
            if i > 0 && self.notes[i - 1].start_ms > n.start_ms {
                return bad(format!("note {i} starts before note {}", i - 1));
            }
        }
        // At most 8 at once, and never two at once on one key: a sweep over the note-ons.
        for (i, n) in self.notes.iter().enumerate() {
            let sounding: Vec<&PhraseNote> = self.notes[..=i]
                .iter()
                .filter(|m| m.start_ms + m.hold_ms > n.start_ms)
                .collect();
            if sounding.len() > MAX_AT_ONCE {
                return bad(format!(
                    "{} notes at once at {} ms",
                    sounding.len(),
                    n.start_ms
                ));
            }
            if sounding.iter().filter(|m| m.key == n.key).count() > 1 {
                return bad(format!("two notes at once on key {}", n.key));
            }
        }
        if let Some(band) = &self.band {
            band.check()?;
        }
        Ok(())
    }

    /// The patch it plays: the timbre, with a band-pass band set in its filter 1 (band-pass mode, the band's
    /// frequency, the resonance of its Q, no key tracking, no drive).
    pub fn patch(&self) -> Result<Patch, StimulusError> {
        self.validate()?;
        let mut patch = timbre(self.timbre).ok_or(StimulusError::TimbreTooNew {
            timbre: self.timbre,
        })?;
        if let Some(band) = self.band.filter(|b| b.kind == EqKind::BandPass) {
            let bp = enth_patch::catalogue::FilterMode::Bp.index();
            for (key, value) in [
                ("mode", f32::from(bp as u8)),
                ("cutoff", band.freq()),
                ("resonance", band.resonance()),
                ("keytrack", 0.0),
                ("drive", 0.0),
            ] {
                let addr: ParamAddr =
                    format!("filter1.{key}")
                        .parse()
                        .map_err(|e| StimulusError::Notes {
                            reason: format!("filter1.{key}: {e}"),
                        })?;
                enth_patch::apply(&mut patch, Op::SetParam { addr, value }).map_err(|e| {
                    StimulusError::Band {
                        reason: e.to_string(),
                    }
                })?;
            }
        }
        Ok(patch)
    }

    /// The render performance of its notes (as a phrase's: `velocity / 127`, times in seconds).
    pub fn performance(&self) -> Performance {
        PhraseView {
            notes: &self.notes,
            length_ms: self.length_ms,
        }
        .performance()
    }

    /// The band applied after the engine (a peak or a shelf), if any.
    pub fn post(&self) -> Option<EqBand> {
        self.band.filter(BandExt::is_post)
    }
}

/// Why a stimulus cannot be played.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StimulusError {
    /// A timbre id this build's pool lacks (`code_too_new`: the pool is append-only, so a newer build's).
    TimbreTooNew {
        /// The id.
        timbre: TimbreId,
    },
    /// Notes or a length outside the promises (`bad_question`).
    Notes {
        /// What is wrong.
        reason: String,
    },
    /// A band outside its ranges (`bad_question`).
    Band {
        /// What is wrong.
        reason: String,
    },
    /// A peak or shelf band, which the offline biquad renders: lane BQ's, not built yet (`needs_biquad`).
    NeedsBiquad,
    /// The render failed (`render_failed`).
    Render {
        /// The renderer's message.
        message: String,
    },
}

impl StimulusError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            StimulusError::TimbreTooNew { .. } => "code_too_new",
            StimulusError::Notes { .. } | StimulusError::Band { .. } => "bad_question",
            StimulusError::NeedsBiquad => "needs_biquad",
            StimulusError::Render { .. } => "render_failed",
        }
    }
}

impl fmt::Display for StimulusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StimulusError::TimbreTooNew { timbre } => {
                write!(f, "timbre {timbre} is not in this build's pool")
            }
            StimulusError::Notes { reason } | StimulusError::Band { reason } => f.write_str(reason),
            StimulusError::NeedsBiquad => f.write_str(
                "a peak or shelf band needs the offline biquad (lane BQ), which this build lacks",
            ),
            StimulusError::Render { message } => write!(f, "the render failed: {message}"),
        }
    }
}

impl std::error::Error for StimulusError {}

/// What a question plays: one clip of a set of clips.
#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    /// A patch heard through a table phrase (a parameter game's A, B or interval).
    Patch {
        /// The patch.
        patch: Box<Patch>,
        /// The phrase.
        phrase: PhraseId,
    },
    /// A music game's stimulus.
    Stimulus(Stimulus),
}

impl Clip {
    /// Render the clip offline at [`SCORING_SAMPLE_RATE`] with [`SCORING_SEED`], as the Target slot plays it. A
    /// stimulus with a peak or shelf band is refused until lane BQ's biquad lands (`needs_biquad`).
    pub fn render(&self) -> Result<RenderOutput, StimulusError> {
        let (patch, perf) = match self {
            Clip::Patch { patch, phrase } => {
                let p = crate::phrases::phrase(*phrase).ok_or_else(|| StimulusError::Notes {
                    reason: format!("phrase {} is not in this build's tables", phrase.0),
                })?;
                ((**patch).clone(), p.performance())
            }
            Clip::Stimulus(s) => {
                if s.post().is_some() {
                    return Err(StimulusError::NeedsBiquad);
                }
                (s.patch()?, s.performance())
            }
        };
        enth_render::render(
            &patch,
            &perf,
            &RenderSpec::new(SCORING_SAMPLE_RATE, SCORING_SEED),
        )
        .map_err(|e| StimulusError::Render {
            message: e.to_string(),
        })
    }
}

/// One option of a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarOption {
    /// What the button says: the editor's own knob name for a control ("Cutoff"; with the module's id when two
    /// options share a name), "1", "2", "3" for Ear check's intervals, the music generator's label otherwise.
    pub label: String,
    /// The control the option names, for a parameter game (the editor highlights its knob).
    pub control: Option<ParamAddr>,
}

/// One question of an ear set.
#[derive(Clone, Debug, PartialEq)]
pub struct EarQuestion {
    /// The question's index in its set.
    pub index: u8,
    /// What to play, in order (Spot the change: A then B; Ear check: three intervals; a music game: one stimulus).
    pub clips: Vec<Clip>,
    /// What to offer, in order: a choice answer is an index into them.
    pub options: Vec<EarOption>,
    /// The right answer.
    pub key: EarKey,
    /// The Review skill it trains (`svf.cutoff`, `ear.interval.m3`), if any.
    pub skill: Option<String>,
    /// For a parameter game, how far the moved control moved, in its parameter steps.
    pub steps: Option<f64>,
}

/// A prepared ear target: its questions, and how they are judged.
#[derive(Clone, Debug, PartialEq)]
pub struct EarSet {
    spec: EarSpec,
    answer: AnswerKind,
    scoring: ScoringSpec,
    questions: Vec<EarQuestion>,
}

impl EarSet {
    /// The spec.
    pub fn spec(&self) -> &EarSpec {
        &self.spec
    }

    /// The answer kind every question takes.
    pub fn answer(&self) -> AnswerKind {
        self.answer
    }

    /// The scoring (the challenge's).
    pub fn scoring(&self) -> &ScoringSpec {
        &self.scoring
    }

    /// The questions, in order.
    pub fn questions(&self) -> &[EarQuestion] {
        &self.questions
    }

    /// The answer keys, in question order.
    pub fn keys(&self) -> Vec<EarKey> {
        self.questions.iter().map(|q| q.key.clone()).collect()
    }

    /// Judge the answers (one per question, `None` for none given) with the challenge's scorer and pass mark: no
    /// render ([`scorers::judge_set`]).
    pub fn judge(&self, answers: &[Option<EarAnswer>]) -> Result<EarScore, EarScoreError> {
        scorers::judge_set(&self.scoring, &self.keys(), answers)
    }
}

/// Prepare an ear challenge's questions: validate it, then generate each question from its game's family (games 1 to
/// 3 here, 4 and up `enth-music`'s ear generator v1). Not an ear target: `bad_answer`. No render: clips are rendered
/// when played.
pub fn prepare(challenge: &Challenge) -> Result<EarSet, ChallengeError> {
    challenge.validate()?;
    let Target::Ear(spec) = &challenge.target else {
        return Err(ChallengeError::Answer {
            reason: "only an ear target has ear questions".to_owned(),
        });
    };
    Ok(EarSet {
        spec: *spec,
        answer: challenge.answer,
        scoring: challenge.scoring.clone(),
        questions: questions(spec)?,
    })
}

/// The questions of `spec`, in order (its answer keys need no render).
pub fn questions(spec: &EarSpec) -> Result<Vec<EarQuestion>, ChallengeError> {
    super::validate(spec).map_err(ChallengeError::Ear)?;
    match spec.game {
        SPOT_THE_CHANGE => (0..spec.count).map(|i| spot_question(spec, i)).collect(),
        EAR_CHECK => (0..spec.count).map(|i| check_question(spec, i)).collect(),
        DIAL_IT_IN => Err(ChallengeError::Answer {
            reason: "Dial it in's rounds are v2 draw challenges (ear::params::dial_it_in)"
                .to_owned(),
        }),
        // The music games: `enth-music`'s ear generator v1 (`super::validate` has checked the game and level).
        _ => spec
            .questions()
            .map_err(|e| ChallengeError::Ear(super::music::refusal(spec, e)))?
            .iter()
            .enumerate()
            .map(|(i, q)| music_question(i as u8, q))
            .collect(),
    }
}

/// A music question as an ear question (the F1e review's M5): its [`Stimulus`] (the timbre, the notes as a phrase's,
/// the length and the band), its options' labels (`Options::labels`), its key (a choice index of the option count, or
/// the onsets and window of a tap answer) and its skill (`Question::skill`). The stimulus is checked here
/// ([`Stimulus::validate`]), so a question this crate would not play is refused before it is asked.
pub fn music_question(
    index: u8,
    q: &enth_music::ear::Question,
) -> Result<EarQuestion, ChallengeError> {
    let stimulus = Stimulus {
        timbre: q.timbre,
        notes: q
            .notes
            .iter()
            .map(|n| PhraseNote {
                key: n.key,
                velocity: n.velocity,
                start_ms: n.start_ms,
                hold_ms: n.hold_ms,
            })
            .collect(),
        length_ms: q.length_ms,
        band: q.post,
    };
    let bad = |reason: String| ChallengeError::Answer { reason };
    stimulus
        .validate()
        .map_err(|e| bad(format!("music question {index}: {e}")))?;
    let options: Vec<EarOption> = q
        .options
        .labels()
        .into_iter()
        .map(|label| EarOption {
            label,
            control: None,
        })
        .collect();
    let key = match &q.answer {
        enth_music::ear::Answer::Choice(i) => EarKey::Choice {
            index: *i,
            options: u8::try_from(q.options.ids.len())
                .map_err(|_| bad(format!("music question {index}: too many options")))?,
        },
        enth_music::ear::Answer::Taps {
            onsets_ms,
            window_ms,
        } => EarKey::Taps(scorers::TapsKey {
            onsets_ms: onsets_ms.clone(),
            window_ms: *window_ms,
        }),
    };
    Ok(EarQuestion {
        index,
        clips: vec![Clip::Stimulus(stimulus)],
        options,
        key,
        skill: q.skill(),
        steps: None,
    })
}

/// A Spot the change question: A, then B; the options' knob names; the moved option's index.
fn spot_question(spec: &EarSpec, index: u8) -> Result<EarQuestion, ChallengeError> {
    let item = params::spot_the_change(spec, index).map_err(params::as_challenge_error)?;
    let base = params::base();
    let b = item
        .draw
        .draw(&base, params::phrase())
        .map_err(ChallengeError::Draw)?
        .patch;
    let moved = &item.options[usize::from(item.key)];
    Ok(EarQuestion {
        index,
        clips: vec![
            Clip::Patch {
                patch: Box::new(base.clone()),
                phrase: params::PARAMS_PHRASE,
            },
            Clip::Patch {
                patch: Box::new(b),
                phrase: params::PARAMS_PHRASE,
            },
        ],
        options: knob_options(&base, &item.options),
        key: EarKey::Choice {
            index: item.key,
            options: item.options.len() as u8,
        },
        skill: skill_of(&base, moved),
        steps: Some(item.steps),
    })
}

/// An Ear check question: three intervals, one moved; the options are the positions.
fn check_question(spec: &EarSpec, index: u8) -> Result<EarQuestion, ChallengeError> {
    let trial = params::ear_check(spec, index).map_err(params::as_challenge_error)?;
    let base = params::base();
    let odd = trial
        .draw
        .draw(&base, params::phrase())
        .map_err(ChallengeError::Draw)?
        .patch;
    let clips = (0..EAR_CHECK_INTERVALS)
        .map(|p| Clip::Patch {
            patch: Box::new(if p == trial.position {
                odd.clone()
            } else {
                base.clone()
            }),
            phrase: params::PARAMS_PHRASE,
        })
        .collect();
    Ok(EarQuestion {
        index,
        clips,
        options: (1..=EAR_CHECK_INTERVALS)
            .map(|n| EarOption {
                label: n.to_string(),
                control: None,
            })
            .collect(),
        key: EarKey::Choice {
            index: trial.position,
            options: EAR_CHECK_INTERVALS,
        },
        skill: skill_of(&base, &trial.control),
        steps: Some(trial.steps),
    })
}

/// The editor's knob names for `controls`, with the module's id in front where two share a name.
fn knob_options(base: &Patch, controls: &[ParamAddr]) -> Vec<EarOption> {
    let name = |a: &ParamAddr| base.spec(a).map_or(a.param(), |s| s.name);
    controls
        .iter()
        .map(|a| {
            let shared = controls.iter().filter(|b| name(b) == name(a)).count() > 1;
            let label = match (shared, a.target()) {
                (true, enth_patch::Target::Module(id)) => format!("{id} {}", name(a)),
                _ => name(a).to_owned(),
            };
            EarOption {
                label,
                control: Some(a.clone()),
            }
        })
        .collect()
}

/// The Review skill of a control: its module's kind (or the built-in) and its key (`svf.cutoff`), as `enth-learn`'s
/// `Skill::of`.
fn skill_of(patch: &Patch, addr: &ParamAddr) -> Option<String> {
    let kind = match addr.target() {
        enth_patch::Target::Module(id) => patch.find(id)?.1.kind.key(),
        enth_patch::Target::Voice => enth_patch::Builtin::Voice.key(),
        enth_patch::Target::Master => enth_patch::Builtin::Master.key(),
    };
    Some(format!("{kind}.{}", addr.param()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::EarSpec;
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    fn note(key: u8, start_ms: u32, hold_ms: u32) -> PhraseNote {
        PhraseNote {
            key,
            velocity: 100,
            start_ms,
            hold_ms,
        }
    }

    fn stim(notes: Vec<PhraseNote>) -> Stimulus {
        Stimulus {
            timbre: SINE,
            notes,
            length_ms: 3000,
            band: None,
        }
    }

    /// The pool's bytes, frozen: a change to a released timbre moves every question that plays it. A new timbre is a
    /// new file at the end, with its hash added here.
    #[test]
    fn the_timbre_pool_is_frozen() {
        const SHA256: [&str; 7] = [
            "21c5fe5862bb5edc9ebf32450ed47746bca03adfab64584de2356cd748793f36",
            "6b0e561b0446295be61bd4b3d5b6dbed603f51af833db1f97248b15f7fa348b2",
            "db52edc0a5f3e38a5d70c379e5d279f580fc2cd3e7dae67a394bdfda23ca0f6d",
            "38412298700b578f4b1ecaf2318d7ff8a6bce985509f6d88c412c5cb74898464",
            "63ddeb5afc49ea9740b9e607a492e7e70c42a6d7dcc6d60d704a7190eb856d0c",
            "9e8bf2303cf9d1acaf3dec2a8574f98d724ff717f9782079e2763c23607cf1da",
            "ff778bd6ff314cc667bb4fda59feb991071c6b8d56b5d62adc84a1cd5c87c48e",
        ];
        for (i, (name, text)) in TIMBRES_V1.iter().enumerate() {
            let hex = Sha256::digest(text.as_bytes())
                .iter()
                .fold(String::new(), |mut s, b| {
                    let _ = write!(s, "{b:02x}");
                    s
                });
            assert_eq!(hex, SHA256[i], "timbre {i} ({name}) is frozen");
        }
    }

    #[test]
    fn every_timbre_loads_is_polyphonic_and_sounds() {
        for (i, (name, _)) in TIMBRES_V1.iter().enumerate() {
            let p = timbre(i as u8).unwrap_or_else(|| panic!("{name}"));
            assert!(
                p.voice.get(enth_patch::catalogue::VoiceParam::Polyphony) >= 8.0,
                "{name}"
            );
            assert!(
                p.find(&"filter1".parse().unwrap()).is_some(),
                "{name} has filter 1"
            );
            let s = Stimulus {
                timbre: i as u8,
                ..stim(vec![note(60, 0, 1000)])
            };
            let out = Clip::Stimulus(s).render().unwrap();
            let lufs = crate::loudness::integrated_lufs(&out.mid(), SCORING_SAMPLE_RATE);
            assert!(lufs.is_some_and(|l| l > -50.0), "{name}: {lufs:?}");
        }
        assert_eq!(timbre(TIMBRES_V1.len() as u8), None);
    }

    #[test]
    fn a_stimulus_keeps_its_promises() {
        assert_eq!(stim(vec![note(60, 0, 1000)]).validate(), Ok(()));
        // A chord of 8 is fine, 9 is not; a unison is one note.
        let chord: Vec<PhraseNote> = (0..8).map(|i| note(48 + i * 3, 0, 1000)).collect();
        assert_eq!(stim(chord.clone()).validate(), Ok(()));
        let mut nine = chord;
        nine.push(note(80, 0, 1000));
        assert_eq!(stim(nine).validate().unwrap_err().code(), "bad_question");
        assert_eq!(
            stim(vec![note(60, 0, 1000), note(60, 500, 1000)])
                .validate()
                .unwrap_err()
                .code(),
            "bad_question"
        );
        // The same key again after it ended is fine.
        assert_eq!(
            stim(vec![note(60, 0, 500), note(60, 500, 500)]).validate(),
            Ok(())
        );
        for bad in [
            stim(vec![note(23, 0, 100)]),
            stim(vec![note(97, 0, 100)]),
            stim(vec![note(60, 0, 0)]),
            stim(vec![note(60, 2500, 600)]),
            stim(vec![note(60, 500, 100), note(62, 0, 100)]),
            stim(vec![]),
            Stimulus {
                length_ms: 10_001,
                ..stim(vec![note(60, 0, 100)])
            },
            Stimulus {
                notes: vec![PhraseNote {
                    velocity: 0,
                    ..note(60, 0, 100)
                }],
                ..stim(vec![])
            },
        ] {
            assert_eq!(
                bad.validate().unwrap_err().code(),
                "bad_question",
                "{bad:?}"
            );
        }
        let newer = Stimulus {
            timbre: 7,
            ..stim(vec![note(60, 0, 100)])
        };
        assert_eq!(newer.validate().unwrap_err().code(), "code_too_new");
    }

    #[test]
    fn a_band_pass_band_sets_the_timbres_filter() {
        let band = EqBand {
            kind: EqKind::BandPass,
            freq_hz: 500,
            gain_cdb: 0,
            q_milli: 8000,
        };
        assert_eq!(band.resonance(), 70.9);
        let s = Stimulus {
            timbre: PINK_NOISE,
            band: Some(band),
            ..stim(vec![note(60, 0, 2000)])
        };
        let p = s.patch().unwrap();
        let at = |k: &str| p.value(&format!("filter1.{k}").parse().unwrap()).unwrap();
        assert_eq!(at("cutoff"), 500.0);
        assert_eq!(at("resonance"), 70.9);
        assert_eq!(at("mode"), 1.0);
        assert_eq!(s.post(), None);
        assert!(Clip::Stimulus(s).render().is_ok());
        // A peak waits for the biquad.
        let peak = Stimulus {
            band: Some(EqBand {
                kind: EqKind::Peak,
                freq_hz: 1000,
                gain_cdb: 600,
                q_milli: 1414,
            }),
            ..stim(vec![note(60, 0, 2000)])
        };
        assert_eq!(peak.post().map(|b| b.gain_db()), Some(6.0));
        assert_eq!(
            Clip::Stimulus(peak).render().unwrap_err().code(),
            "needs_biquad"
        );
        for bad in [
            EqBand {
                gain_cdb: 100,
                ..band
            },
            EqBand {
                q_milli: 400,
                ..band
            },
            EqBand {
                freq_hz: 19,
                ..band
            },
            EqBand {
                kind: EqKind::Peak,
                gain_cdb: 0,
                ..band
            },
            EqBand {
                kind: EqKind::HighShelf,
                gain_cdb: 2401,
                ..band
            },
        ] {
            assert_eq!(bad.check().unwrap_err().code(), "bad_question", "{bad:?}");
        }
    }

    fn spot(seed: u64) -> Challenge {
        Challenge::ear(EarSpec {
            game: SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed,
            count: 16,
        })
        .unwrap()
    }

    #[test]
    fn a_spot_the_change_set_prepares_and_its_oracle_scores_100() {
        let set = prepare(&spot(42)).unwrap();
        assert_eq!(set.questions().len(), 16);
        for q in set.questions() {
            assert_eq!(q.clips.len(), 2);
            assert_eq!(q.options.len(), 2);
            assert!(
                q.options
                    .iter()
                    .all(|o| o.control.is_some() && !o.label.is_empty())
            );
            assert!(q.skill.is_some());
        }
        let oracle: Vec<Option<EarAnswer>> =
            set.keys().iter().map(|k| Some(k.right_answer())).collect();
        let score = set.judge(&oracle).unwrap();
        assert_eq!(
            (score.right, score.percent, score.passed),
            (16, 100.0, true)
        );
        let none: Vec<Option<EarAnswer>> = vec![None; 16];
        assert!(!set.judge(&none).unwrap().passed);
        // Renders A and B: they differ.
        let q = &set.questions()[0];
        let a = q.clips[0].render().unwrap().mid();
        let b = q.clips[1].render().unwrap().mid();
        assert_ne!(a, b);
    }

    #[test]
    fn an_ear_check_set_prepares() {
        let c = Challenge::ear(EarSpec {
            game: EAR_CHECK,
            generator: 1,
            level: 1,
            seed: 5,
            count: 10,
        })
        .unwrap();
        let set = prepare(&c).unwrap();
        let skills: Vec<&Option<String>> = set.questions().iter().map(|q| &q.skill).collect();
        assert!(
            skills.windows(2).all(|w| w[0] == w[1]),
            "one control per set"
        );
        for q in set.questions() {
            let EarKey::Choice { index, options } = q.key else {
                panic!()
            };
            assert_eq!(options, 3);
            assert_eq!(q.clips.len(), 3);
            for (p, clip) in q.clips.iter().enumerate() {
                let Clip::Patch { patch, .. } = clip else {
                    panic!()
                };
                assert_eq!(**patch != params::base(), p == usize::from(index));
            }
        }
    }

    #[test]
    fn only_an_ear_target_prepares() {
        let level1 = Challenge::recreate(
            crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 42),
            PhraseId(1),
        );
        assert_eq!(prepare(&level1).unwrap_err().code(), "bad_answer");
    }

    /// Keys need no render: 1,000 Spot the change keys come from parameter space alone (this test renders nothing).
    #[test]
    fn keys_need_no_render() {
        let spec = EarSpec {
            game: SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed: 77,
            count: 50,
        };
        let mut n = 0;
        for seed in 0..20 {
            let qs = questions(&EarSpec { seed, ..spec }).unwrap();
            n += qs.len();
        }
        assert_eq!(n, 1000);
    }

    /// The F1e review's M5: level 1 of every music game prepares through this crate (`enth-music`'s questions as
    /// stimuli and keys, no render), every stimulus is one this crate plays, the labels and skill come through, and an
    /// oracle that answers each key's right answer scores 100 % with the challenge's own scorer.
    #[test]
    fn every_music_game_prepares_at_level_1_and_its_oracle_scores_100() {
        for game in enth_music::ear::Game::ALL {
            for seed in [0u64, 1, 42, 9_007_199_254_740_991] {
                let spec = EarSpec::new(game, 1, seed, 20);
                let challenge = Challenge::ear(spec).unwrap_or_else(|e| panic!("{game:?}: {e}"));
                let set = prepare(&challenge).unwrap_or_else(|e| panic!("{game:?} {seed}: {e}"));
                assert_eq!(set.questions().len(), 20);
                for (i, q) in set.questions().iter().enumerate() {
                    let music = enth_music::ear::question(game, 1, seed, i as u32).unwrap();
                    assert_eq!(q.index as usize, i);
                    assert_eq!(q.skill, music.skill(), "{game:?}");
                    let [Clip::Stimulus(stimulus)] = q.clips.as_slice() else {
                        panic!("{game:?}: one stimulus");
                    };
                    assert_eq!(stimulus.validate(), Ok(()));
                    assert_eq!(stimulus.timbre, music.timbre);
                    assert_eq!(stimulus.length_ms, music.length_ms);
                    assert_eq!(stimulus.notes.len(), music.notes.len());
                    assert_eq!(
                        q.options
                            .iter()
                            .map(|o| o.label.clone())
                            .collect::<Vec<_>>(),
                        music.options.labels()
                    );
                    match (&q.key, game.answer_kind(1)) {
                        (EarKey::Choice { options, .. }, enth_music::ear::AnswerKind::Choice) => {
                            assert_eq!(usize::from(*options), q.options.len());
                        }
                        (EarKey::Taps(_), enth_music::ear::AnswerKind::Taps) => {}
                        (key, kind) => panic!("{game:?}: {key:?} for {kind:?}"),
                    }
                }
                let oracle: Vec<Option<EarAnswer>> =
                    set.keys().iter().map(|k| Some(k.right_answer())).collect();
                let score = set.judge(&oracle).unwrap();
                assert_eq!(
                    (score.right, score.percent, score.passed),
                    (20, 100.0, true),
                    "{game:?}"
                );
                // A wrong first answer costs a question (a choice) or its share (taps).
                let mut one_off = oracle.clone();
                one_off[0] = None;
                assert!(set.judge(&one_off).unwrap().percent < 100.0);
            }
        }
    }
}
