//! **Owner: lane CT** (this file: the `Scorer` trait, the score types and `ScoreReport`, freeze point F1r).
//! Pre-declared by lane S0r; the sub-modules belong to lanes SS ([`sound`]) and ST ([`structure`], [`combined`]),
//! which implement the traits here. The reference is docs/challenge.md, "Scoring interfaces".
//!
//! Scores (report 02, ADR-0007's 2026-10-20 note, design.md §6 "Scoring is additive"):
//!
//! - **Sound %** ([`SoundScore`], lane SS) decides pass. It is a distance in per-challenge just-noticeable units
//!   mapped by a curve fitted to ratings, with descriptor [`Component`]s beside the headline (pitch in cents,
//!   brightness, attack, envelope, release, noisiness, movement), because the headline barely sees a short attack.
//! - **Structure %** ([`StructureScore`], lane ST) compares the patches over the randomized, observable parameters,
//!   with a Good/Near/Bad [`Verdict`] per parameter for the editor's rings.
//! - **Combined %** ([`Combined`], lane ST) is w·sound + (1 − w)·structure, with w from the challenge's
//!   [`ScoringSpec`]. It never decides pass.
//! - [`ScoreReport`] puts them together with the pass decision ([`ScoringSpec::passes`]): what a submit returns, the
//!   results drawer shows, the round log stores and the golden challenges pin. JSON with
//!   `"schema": "enth.score-report/1"` first.
//!
//! **Scorers are versioned and additive.** Each has a [`ScorerId`] (`sound/1`); a change that moves any score is a new
//! id beside the old one (metric v1.1 is its own id), never an edit, so a stored report keeps meaning what it said.
//!
//! **Scores across machines.** A scorer whose result decides pass must give the same bits on every platform: the
//! scalar FFT path and the in-house `libm` loudness (lane SS), a fixed summation order, no `std` transcendentals.
//! The golden challenges (lane GC) pin scores to 1e-6 and keep a margin of at least 1e-3 from the pass mark.

pub mod combined;
pub mod sound;
pub mod structure;

use std::borrow::Cow;
use std::fmt;

use enth_patch::Patch;
use enth_patch::addr::{AddrError, ParamAddr};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{Challenge, Phrase, ScoringSpec};

/// The `schema` string of a [`ScoreReport`]: `enth.score-report/1`.
pub const SCORE_REPORT_SCHEMA: &str = "enth.score-report/1";

/// A scorer's stable id: a name and a version, written `name/version` (`sound/1`). A scorer that would move any
/// score gets a new version or a new name; old ids keep their meaning.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScorerId {
    name: Cow<'static, str>,
    version: u16,
}

impl ScorerId {
    /// `sound/1`: metric v1 with report 02's fixes (lane SS).
    pub const SOUND_V1: ScorerId = ScorerId::new("sound", 1);
    /// `structure/1` (lane ST).
    pub const STRUCTURE_V1: ScorerId = ScorerId::new("structure", 1);
    /// `combined/1`: w·sound + (1 − w)·structure (lane ST).
    pub const COMBINED_V1: ScorerId = ScorerId::new("combined", 1);
    /// `time/1`: the first passing submit, in editor-visible time (lane ST).
    pub const TIME_V1: ScorerId = ScorerId::new("time", 1);

    /// An id. `name` is lower-case ASCII letters, digits, `-` and `.`; `version` is at least 1 (checked when an id is
    /// parsed, and by [`ScorerId::is_valid`]).
    pub const fn new(name: &'static str, version: u16) -> ScorerId {
        ScorerId {
            name: Cow::Borrowed(name),
            version,
        }
    }

    /// Parse `name/version`.
    pub fn parse(text: &str) -> Option<ScorerId> {
        let (name, version) = text.split_once('/')?;
        if version.is_empty()
            || version.starts_with('0')
            || !version.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let id = ScorerId {
            name: Cow::Owned(name.to_owned()),
            version: version.parse().ok()?,
        };
        id.is_valid().then_some(id)
    }

    /// The name (`sound`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Whether the name and version follow the rules of [`ScorerId::new`].
    pub fn is_valid(&self) -> bool {
        !self.name.is_empty()
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && self.version >= 1
    }
}

impl fmt::Display for ScorerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.name, self.version)
    }
}

impl Serialize for ScorerId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ScorerId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        ScorerId::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format_args!(
                "{text:?} is not a scorer id (name/version, e.g. sound/1)"
            ))
        })
    }
}

/// Everything a scorer may know before the player answers, once per challenge.
#[derive(Clone, Copy, Debug)]
pub struct TargetContext<'a> {
    /// The challenge (its scoring spec and difficulty).
    pub challenge: &'a Challenge,
    /// The target patch, regenerated from the challenge (never persisted).
    pub target: &'a Patch,
    /// The patch the player started from.
    pub start: &'a Patch,
    /// The phrase both patches are rendered with ([`Phrase::performance`], at
    /// [`SCORING_SAMPLE_RATE`](crate::model::SCORING_SAMPLE_RATE) with
    /// [`SCORING_SEED`](crate::model::SCORING_SEED)).
    pub phrase: &'a Phrase,
    /// The parameters the generator randomized, all observable through the phrase (lane RG's mask), in address
    /// order. Structure compares these; the rings go on these.
    pub randomized: &'a [ParamAddr],
}

/// A scorer: sound (lane SS) or structure (lane ST). Pure: no I/O, no threads, no clocks; renders go through
/// `enth-render` on the caller's thread, and the shell's worker runs it off the UI and audio threads.
///
/// [`prepare`](Scorer::prepare) runs once per challenge (the target's renders, its nuisance floor, the per-challenge
/// JND scale: about 0.1–1 s); [`score`](Scorer::score) runs per submit (or per settled edit with the live meter) on
/// the player's patch. Both are deterministic: the same inputs give the same bits on every platform.
pub trait Scorer: Send + Sync {
    /// What `prepare` computes from the target, reused by every `score`.
    type Prepared: Send + Sync;
    /// What `score` returns: [`SoundScore`] or [`StructureScore`].
    type Score: Clone + fmt::Debug + Send + Sync;

    /// This scorer's id: what a [`ScoringSpec`] names and every score it returns carries.
    fn id(&self) -> ScorerId;

    /// The per-challenge work.
    fn prepare(&self, target: &TargetContext<'_>) -> Result<Self::Prepared, ScoreError>;

    /// Score the player's patch.
    fn score(&self, prepared: &Self::Prepared, answer: &Patch) -> Result<Self::Score, ScoreError>;
}

/// The combined score (lane ST): one headline number from the sound and structure scores. It never decides pass.
pub trait Combiner: Send + Sync {
    /// This combiner's id (`combined/1`).
    fn id(&self) -> ScorerId;

    /// The combined score, with the weight from `scoring`.
    fn combine(
        &self,
        scoring: &ScoringSpec,
        sound: &SoundScore,
        structure: &StructureScore,
    ) -> Combined;
}

/// Why a scorer could not score.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScoreError {
    /// A render failed (an invalid patch, a bad performance).
    Render {
        /// The renderer's message.
        message: String,
    },
    /// The analysis failed.
    Analysis {
        /// The analyser's message.
        message: String,
    },
    /// The inputs cannot be scored by this scorer (a silent target, a challenge kind it does not handle).
    Unscorable {
        /// Why.
        reason: String,
    },
}

impl ScoreError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            ScoreError::Render { .. } => "render_failed",
            ScoreError::Analysis { .. } => "analysis_failed",
            ScoreError::Unscorable { .. } => "unscorable",
        }
    }
}

impl fmt::Display for ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScoreError::Render { message } | ScoreError::Analysis { message } => {
                f.write_str(message)
            }
            ScoreError::Unscorable { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for ScoreError {}

/// How close the player's audio is to the target's (lane SS; ADR-0007's 2026-10-20 note). It decides pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundScore {
    /// The scorer that made it (`sound/1`).
    pub scorer: ScorerId,
    /// Sound %, 0–100: `100 / (1 + (jnd / z50)^k)` with the fitted curve.
    pub percent: f64,
    /// The raw metric distance between the two scored renders.
    pub distance: f64,
    /// The target's own nuisance floor: its distance to itself rendered at other seeds.
    pub floor: f64,
    /// The per-challenge JND scale: the median distance of the target against renders with every active parameter
    /// moved by one just-noticeable step.
    pub scale: f64,
    /// The distance in JND units: `max(0, distance − floor) / scale`.
    pub jnd: f64,
    /// The descriptor rows shown beside the headline, in [`ComponentKind`] order, one per kind at most.
    pub components: Vec<Component>,
}

/// One descriptor row of a [`SoundScore`]: the coaching text ("brighter than the target", "attack too slow") and the
/// agent's `compare` output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Which descriptor.
    pub kind: ComponentKind,
    /// The player's reading relative to the target's.
    pub direction: Direction,
    /// How far apart they are, in the descriptor's own tolerance (`check`'s defaults to start: 25 ct, 10 % for times
    /// and Hz, 2 dB).
    pub closeness: Closeness,
    /// The target's reading in [`ComponentKind::unit`], when it has one. Never shown during a round (no peeking).
    pub target: Option<f64>,
    /// The player's reading, when it has one.
    pub answer: Option<f64>,
}

/// The descriptor a [`Component`] reads (report 02 §2.4; ADR-0007's 2026-10-20 note).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Pitch: f0 in cents relative to A4 (440 Hz), so a difference reads in cents.
    Pitch,
    /// Brightness: the spectral centroid, Hz.
    Brightness,
    /// Attack time, ms. Less reliable on low notes (a 3 ms attack reads 15 ms at A1).
    Attack,
    /// Decay and sustain: the decay time to the sustain level, ms.
    Envelope,
    /// Release time, ms.
    Release,
    /// Noisiness: the noise ratio, 0–1.
    Noisiness,
    /// Movement (beating, tremolo, a filter sweep): the rate of the strongest, Hz; no reading when there is none.
    Movement,
}

impl ComponentKind {
    /// Every kind, in display order.
    pub const ALL: [ComponentKind; 7] = [
        ComponentKind::Pitch,
        ComponentKind::Brightness,
        ComponentKind::Attack,
        ComponentKind::Envelope,
        ComponentKind::Release,
        ComponentKind::Noisiness,
        ComponentKind::Movement,
    ];

    /// The unit of the readings.
    pub fn unit(self) -> &'static str {
        match self {
            ComponentKind::Pitch => "ct",
            ComponentKind::Brightness | ComponentKind::Movement => "Hz",
            ComponentKind::Attack | ComponentKind::Envelope | ComponentKind::Release => "ms",
            ComponentKind::Noisiness => "ratio",
        }
    }
}

/// The player's reading relative to the target's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Within tolerance.
    Same,
    /// Higher: sharper, brighter, slower attack, longer decay or release, noisier, faster movement.
    Higher,
    /// Lower.
    Lower,
    /// The target has it and the answer does not (no pitch, no movement).
    Missing,
    /// The answer has it and the target does not.
    Extra,
}

/// How far a reading is from the target's, in the descriptor's tolerance: the close / a bit / far badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Closeness {
    /// Within the tolerance.
    Close,
    /// Within a few tolerances.
    ABit,
    /// Further.
    Far,
}

/// How close the player's patch is to the target's (lane ST), over the randomized parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureScore {
    /// The scorer that made it (`structure/1`).
    pub scorer: ScorerId,
    /// Structure %, 0–100.
    pub percent: f64,
    /// One row per randomized parameter, in address order: what the editor's rings show.
    pub params: Vec<ParamScore>,
    /// Modules the matching left over: missing from the answer, or extra in it.
    pub modules: Vec<ModuleTerm>,
}

/// One randomized parameter's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamScore {
    /// The parameter's address on the target (`filter1.cutoff`), as text so an old report still reads after a
    /// catalogue change; [`ParamScore::param_addr`] parses it.
    pub addr: String,
    /// 0–1: 1 within the parameter's tolerance, falling to 0 at four tolerances.
    pub score: f64,
    /// The ring on the control.
    pub verdict: Verdict,
}

impl ParamScore {
    /// The address, parsed against this build's catalogue.
    pub fn param_addr(&self) -> Result<ParamAddr, AddrError> {
        self.addr.parse()
    }
}

/// The ring on a control after a submit (the editor's `Overlay`: Good, Near, Bad).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Within tolerance.
    Good,
    /// Close.
    Near,
    /// Off.
    Bad,
}

/// A module the structure matching could not pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleTerm {
    /// The module's id: the target's for a missing module, the player's for an extra one.
    pub id: String,
    /// Its kind (`osc`, `svf`).
    pub kind: String,
    /// Missing or extra.
    pub status: ModuleStatus,
}

/// Which side a [`ModuleTerm`]'s module is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleStatus {
    /// The target has it and the answer does not.
    Missing,
    /// The answer has it and the target does not.
    Extra,
}

/// The combined score (lane ST): one headline number. It never decides pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Combined {
    /// The combiner that made it (`combined/1`).
    pub scorer: ScorerId,
    /// Combined %, 0–100.
    pub percent: f64,
    /// The sound weight used, in percent ([`ScoringSpec::sound_weight`]).
    pub sound_weight: u8,
}

/// What a submit returns: the three scores and the pass decision. JSON with `"schema": "enth.score-report/1"` first;
/// every percentage finite and within 0–100, and `passed` agrees with `pass_mark` (checked when a report is read).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(into = "ScoreReportFile", try_from = "ScoreReportFile")]
pub struct ScoreReport {
    /// Sound: decides pass.
    pub sound: SoundScore,
    /// Structure.
    pub structure: StructureScore,
    /// Combined.
    pub combined: Combined,
    /// The challenge's pass mark ([`ScoringSpec::pass_mark`]).
    pub pass_mark: u8,
    /// Whether the submit passed: sound % at or above the pass mark.
    pub passed: bool,
}

impl ScoreReport {
    /// A report, with the pass decided by `scoring` from the sound score alone.
    pub fn new(
        scoring: &ScoringSpec,
        sound: SoundScore,
        structure: StructureScore,
        combined: Combined,
    ) -> ScoreReport {
        let passed = scoring.passes(sound.percent);
        ScoreReport {
            sound,
            structure,
            combined,
            pass_mark: scoring.pass_mark,
            passed,
        }
    }

    /// Sound % minus the pass mark: positive passes. The golden challenges keep it at least 1e-3 away from 0.
    pub fn margin(&self) -> f64 {
        self.sound.percent - f64::from(self.pass_mark)
    }

    /// The rings: each randomized parameter's address and verdict, in address order.
    pub fn verdicts(&self) -> impl Iterator<Item = (&str, Verdict)> {
        self.structure
            .params
            .iter()
            .map(|p| (p.addr.as_str(), p.verdict))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreReportFile {
    schema: String,
    sound: SoundScore,
    structure: StructureScore,
    combined: Combined,
    pass_mark: u8,
    passed: bool,
}

impl From<ScoreReport> for ScoreReportFile {
    fn from(r: ScoreReport) -> Self {
        ScoreReportFile {
            schema: SCORE_REPORT_SCHEMA.to_owned(),
            sound: r.sound,
            structure: r.structure,
            combined: r.combined,
            pass_mark: r.pass_mark,
            passed: r.passed,
        }
    }
}

impl TryFrom<ScoreReportFile> for ScoreReport {
    type Error = String;

    fn try_from(f: ScoreReportFile) -> Result<Self, Self::Error> {
        if f.schema != SCORE_REPORT_SCHEMA {
            return Err(format!(
                "schema {:?}: this build reads {SCORE_REPORT_SCHEMA}",
                f.schema
            ));
        }
        let percent = |what: &str, p: f64| {
            if p.is_finite() && (0.0..=100.0).contains(&p) {
                Ok(())
            } else {
                Err(format!("{what} {p} is not a percentage"))
            }
        };
        percent("sound", f.sound.percent)?;
        percent("structure", f.structure.percent)?;
        percent("combined", f.combined.percent)?;
        if f.pass_mark > 100 || f.combined.sound_weight > 100 {
            return Err("pass_mark and sound_weight are percentages".to_owned());
        }
        if f.passed != (f.sound.percent >= f64::from(f.pass_mark)) {
            return Err("passed must agree with the sound % and the pass mark".to_owned());
        }
        Ok(ScoreReport {
            sound: f.sound,
            structure: f.structure,
            combined: f.combined,
            pass_mark: f.pass_mark,
            passed: f.passed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scorer_ids_read_and_write_as_name_slash_version() {
        assert_eq!(ScorerId::SOUND_V1.to_string(), "sound/1");
        assert_eq!(ScorerId::parse("sound/1"), Some(ScorerId::SOUND_V1));
        assert_eq!(
            ScorerId::parse("logmel-pool85-l1/12").map(|i| i.version()),
            Some(12)
        );
        for bad in [
            "sound",
            "sound/0",
            "sound/01",
            "Sound/1",
            "/1",
            "sound/",
            "so und/1",
            "sound/1/2",
            "sound/x",
        ] {
            assert_eq!(ScorerId::parse(bad), None, "{bad}");
        }
        let json = serde_json::to_string(&ScorerId::TIME_V1).unwrap();
        assert_eq!(json, "\"time/1\"");
        assert_eq!(
            serde_json::from_str::<ScorerId>(&json).unwrap(),
            ScorerId::TIME_V1
        );
        assert!(serde_json::from_str::<ScorerId>("\"time\"").is_err());
        for id in [
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        ] {
            assert!(id.is_valid(), "{id}");
        }
    }

    #[test]
    fn every_component_kind_has_a_unit() {
        for k in ComponentKind::ALL {
            assert_ne!(k.unit(), "");
        }
        let mut sorted = ComponentKind::ALL;
        sorted.sort();
        assert_eq!(sorted, ComponentKind::ALL, "ALL is in declaration order");
    }

    /// A scorer that ignores its inputs: the trait is implementable as declared, and its scores combine.
    struct Fixed;

    impl Scorer for Fixed {
        type Prepared = ();
        type Score = SoundScore;

        fn id(&self) -> ScorerId {
            ScorerId::SOUND_V1
        }

        fn prepare(&self, _: &TargetContext<'_>) -> Result<(), ScoreError> {
            Ok(())
        }

        fn score(&self, (): &(), _: &Patch) -> Result<SoundScore, ScoreError> {
            Ok(SoundScore {
                scorer: self.id(),
                percent: 95.0,
                distance: 0.1,
                floor: 0.0,
                scale: 0.1,
                jnd: 1.0,
                components: Vec::new(),
            })
        }
    }

    #[test]
    fn a_scorer_is_object_safe_enough_to_box() {
        let s: Box<dyn Scorer<Prepared = (), Score = SoundScore>> = Box::new(Fixed);
        let score = s.score(&(), &Patch::init()).unwrap();
        assert_eq!(score.scorer, ScorerId::SOUND_V1);
    }
}
