//! **Owner: lane CT** (Challenge types, freeze point F1r). The reference is docs/challenge.md, "Challenge v1" and
//! "Round results".
//!
//! [`Challenge`] v1 (design.md §6, report 01 §2.5, report 02): what a round asks of the player. In R1 every challenge
//! is a Recreate challenge: the target is generated from a [`RandomSpec`] ([`Target::Seed`]), the answer is a patch,
//! the player hears the target through a [`Phrase`] and starts from a template's patch. The target patch itself is
//! never persisted; a round keeps its [`ChallengeCode`](crate::code::ChallengeCode) and regenerates the target.
//!
//! Persisted, so versioned: the JSON form starts with `"schema": "enth.challenge/<N>"`, a newer version is refused
//! (`challenge_too_new`), unknown fields are refused, and fixtures under `tests/fixtures/challenge/v1/` and `v2/` pin
//! the bytes. Any change to the shape is a version bump with a migration (AGENTS.md); a field is never silently
//! ignored, because ignoring a constraint would play a different game. Every number of the challenge's own is an
//! integer (an inline patch and a draw's values are patch values: [`crate::v2`]).
//!
//! **Challenge v2** (lane CV2, R2; docs/challenge.md, "Challenge v2"): the same types gain a patch target
//! ([`Target::Patch`]), a seeded draw over the start ([`Target::Draw`]), a patch start ([`Start::Patch`]) and
//! [`Constraints`] (`editable`, `locked`), so a lesson's step can be a challenge. v1 reads as v2 with no change of
//! meaning, and a challenge is written at the lowest version that holds it, so a generated round's challenge keeps
//! its v1 bytes. The file forms, the migration and the new pieces live in [`crate::v2`].
//!
//! The round's results ([`RoundResult`], [`Outcome`], [`AssistsUsed`], [`Agreement`]) are here too: lane SG's round
//! produces them and lane GK's round log stores them, with `"schema": "enth.round-result/1"`. A result is an output,
//! read tolerantly (F1r review M3, S6): unknown fields are ignored, a missing new field takes its default and an
//! unknown enum value reads as `unknown`, so a newer build's result (or a round saved by one) still reads.

use enth_patch::Patch;
use enth_render::{Note, Performance};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::code::{ChallengeCode, EngineVersion, TargetPrefix};
use crate::score::{ScoreReport, ScorerId};
use crate::spec::{LATEST_TEMPLATE, PERCENT_MAX, RandomSpec, SpecError};
pub use crate::v2::Constraints;
use crate::v2::{Draw, DrawError, InlinePatch};

/// The challenge schema family.
pub const CHALLENGE_KIND: &str = "enth.challenge";

/// The newest challenge version this build reads and writes (2: Challenge v2, lane CV2). Every version from 1 up
/// reads; a challenge is written at the lowest version that holds it ([`Challenge::version`]).
pub const CHALLENGE_VERSION: u32 = 2;

/// The `schema` string of the newest version, `enth.challenge/2` (a v1-expressible challenge is still written as
/// `enth.challenge/1`: [`crate::v2::SCHEMA_V1`]).
pub const CHALLENGE_SCHEMA: &str = crate::v2::SCHEMA_V2;

/// The `schema` string of a [`RoundResult`]: `enth.round-result/1`.
pub const ROUND_RESULT_SCHEMA: &str = "enth.round-result/1";

/// The sample rate of every scored render, and of the pre-rendered Target ▶ loop (design.md §5: one canonical scoring
/// rate).
pub const SCORING_SAMPLE_RATE: u32 = 48_000;

/// The render seed of every scored render and of the Target ▶ loop, written out (F1r review S12): a scored render
/// must never move because the CLI's default seed did. It equals `enth_render::DEFAULT_SEED` today, so `enth render`
/// of a target with its phrase gives exactly the audio the scorer hears (design.md §5; a test checks the two agree).
/// The nuisance-floor renders of the sound scorer use other seeds, derived from this one.
pub const SCORING_SEED: u64 = 0x0E17_5EED;

/// The sound % at or above which a submit passes, unless a challenge says otherwise. **The mark stays and the curve
/// moves** (F1r review S12): lane SS fits `sound/1`'s curve so that the rating-4.5 point lands on 90, and GR1's refit
/// is `sound/2`, fitted to the same mark. The stubs (`sound-stub/1`) use it as is.
pub const DEFAULT_PASS_MARK: u8 = 90;

/// A challenge: what a round asks of the player. See the [module docs](self).
///
/// JSON, compact and schema first (`Challenge::to_json`); a generated round's challenge is v1 (a v2 example is in
/// [`crate::v2`] and docs/challenge.md):
///
/// ```json
/// {"schema":"enth.challenge/1",
///  "target":{"kind":"seed","spec":{"generator":1,"seed":42,"difficulty":0,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}},
///  "answer":"patch","phrase":1,"start":{"kind":"template","template":0},"constraints":{},
///  "assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},
///  "scoring":{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90},
///  "meta":{"title":"","tags":[]}}
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    /// What the player recreates.
    pub target: Target,
    /// What the player answers with.
    pub answer: AnswerKind,
    /// The phrase the target and the answer are heard and scored through: an id in lane RG's versioned table
    /// ([`crate::phrases`]). For a [`Target::Seed`] the generator picks it, so it is the generator's choice for that
    /// spec.
    pub phrase: PhraseId,
    /// What the player starts from.
    pub start: Start,
    /// What the player is meant to change and what the session refuses. R1 locks nothing; v1 has no constraint.
    pub constraints: Constraints,
    /// The help a round allows.
    pub assists: Assists,
    /// Which scorers, their weights and the pass mark.
    pub scoring: ScoringSpec,
    /// Title and tags, for lists and logs.
    pub meta: ChallengeMeta,
}

impl Challenge {
    /// The Recreate challenge for `spec`, heard through `phrase` (the generator's pick for this spec): the template's
    /// start, no constraints, the default assists, and [`ScoringSpec::recreate`] at the spec's difficulty (this
    /// build's default scorers).
    pub fn recreate(spec: RandomSpec, phrase: PhraseId) -> Challenge {
        Challenge {
            target: Target::Seed(spec),
            answer: AnswerKind::Patch,
            phrase,
            start: Start::Template {
                template: spec.template,
            },
            constraints: Constraints::default(),
            assists: Assists::default(),
            scoring: ScoringSpec::recreate(spec.difficulty),
            meta: ChallengeMeta::default(),
        }
    }

    /// A challenge on `target` from `start`, heard through `phrase` and scored by `scoring`: a patch answer, no
    /// constraint, the default assists, no title (a lesson's step sets the rest).
    pub fn new(target: Target, start: Start, phrase: PhraseId, scoring: ScoringSpec) -> Challenge {
        Challenge {
            target,
            answer: AnswerKind::Patch,
            phrase,
            start,
            constraints: Constraints::default(),
            assists: Assists::default(),
            scoring,
            meta: ChallengeMeta::default(),
        }
    }

    /// The randomizer spec, for a generated target.
    pub fn spec(&self) -> Option<&RandomSpec> {
        match &self.target {
            Target::Seed(spec) => Some(spec),
            Target::Patch(_) | Target::Draw(_) => None,
        }
    }

    /// The draw, for a drawn target.
    pub fn draw(&self) -> Option<&Draw> {
        match &self.target {
            Target::Draw(draw) => Some(draw),
            Target::Seed(_) | Target::Patch(_) => None,
        }
    }

    /// The lowest version that holds this challenge, the one [`to_json`](Self::to_json) writes: 1 for a seed target
    /// with its template's start and no constraint (every R1 challenge, byte for byte as v1 wrote it), else 2.
    pub fn version(&self) -> u32 {
        let v1 = matches!(self.target, Target::Seed(_))
            && matches!(self.start, Start::Template { .. })
            && self.constraints.is_empty()
            && self.answer == AnswerKind::Patch;
        if v1 { 1 } else { 2 }
    }

    /// The same challenge with another seed: a sibling for a redo or a parallel test form. `None` for a patch target,
    /// which has no seed.
    pub fn with_seed(&self, seed: u64) -> Option<Challenge> {
        let mut c = self.clone();
        match &mut c.target {
            Target::Seed(spec) => spec.seed = seed,
            Target::Draw(draw) => draw.seed = seed,
            Target::Patch(_) => return None,
        }
        Some(c)
    }

    /// The patch the player starts from: the template's frozen start ([`crate::random::start`]) or the inline patch.
    /// A template this build does not have is `code_too_new`.
    pub fn start_patch(&self) -> Result<Patch, ChallengeError> {
        match &self.start {
            Start::Template { template } => crate::random::start(*template).map_err(|_| {
                ChallengeError::Spec(SpecError::TemplateTooNew {
                    template: *template,
                    latest: LATEST_TEMPLATE,
                })
            }),
            Start::Patch { patch } => Ok(patch.patch().clone()),
        }
    }

    /// The generator version, for a generated target.
    pub fn generator(&self) -> Option<u16> {
        self.spec().map(|s| s.generator)
    }

    /// The difficulty, 0–100, for a generated target.
    pub fn difficulty(&self) -> Option<u8> {
        self.spec().map(|s| s.difficulty)
    }

    /// The short code, when the spec has one ([`ChallengeCode::short`]).
    pub fn short_code(&self) -> Option<ChallengeCode> {
        self.spec().and_then(|s| ChallengeCode::short(*s))
    }

    /// The code to share or log for a generated target: the short form when the spec has one, otherwise the long form
    /// made with this build's engine and `target`, the regenerated target's fingerprint prefix. A round that started
    /// from a code keeps that code instead, so a long code keeps its maker's engine version.
    pub fn code(&self, target: TargetPrefix) -> Option<ChallengeCode> {
        let spec = *self.spec()?;
        ChallengeCode::short(spec)
            .or_else(|| ChallengeCode::long(spec, EngineVersion::CURRENT, target).ok())
    }

    /// Check what the types cannot, and every load runs it:
    ///
    /// - a seed target: a valid spec (a generator and template this build has, a seed below 2⁵³) and its template's
    ///   start (`start_mismatch`);
    /// - a template start that this build has (`code_too_new`);
    /// - a draw target: its draw against the start ([`Draw::validate`]: items on the start, values that resolve,
    ///   sets a step apart);
    /// - constraints whose selectors name things the start has, and no lock on anything the target varies (the
    ///   round could not be passed), both `bad_selector`;
    /// - a phrase id of either phrase table (`phrase_too_new` for an id this build does not have; 0 is `bad_field`);
    /// - the scoring percentages.
    pub fn validate(&self) -> Result<(), ChallengeError> {
        match (&self.target, &self.start) {
            (Target::Seed(spec), start) => {
                spec.validate().map_err(ChallengeError::Spec)?;
                match start {
                    Start::Template { template } if *template == spec.template => {}
                    Start::Template { template } => {
                        return Err(ChallengeError::StartMismatch {
                            start: *template,
                            spec: spec.template,
                        });
                    }
                    Start::Patch { .. } => {
                        return Err(ChallengeError::StartNotTemplate {
                            spec: spec.template,
                        });
                    }
                }
            }
            (_, Start::Template { template }) if *template > LATEST_TEMPLATE => {
                return Err(ChallengeError::Spec(SpecError::TemplateTooNew {
                    template: *template,
                    latest: LATEST_TEMPLATE,
                }));
            }
            _ => {}
        }
        if self.phrase.0 == 0 {
            return Err(ChallengeError::BadField {
                field: "phrase",
                value: 0,
            });
        }
        if crate::phrases::entry(self.phrase).is_none() {
            return Err(ChallengeError::UnknownPhrase {
                phrase: self.phrase.0,
            });
        }
        // The start patch only when something is checked against it, so a v1 challenge loads nothing.
        if self.draw().is_some() || !self.constraints.is_empty() {
            let start = self.start_patch()?;
            if let Some(draw) = self.draw() {
                draw.validate(&start).map_err(ChallengeError::Draw)?;
            }
            self.constraints.check_against(&start)?;
            if !self.constraints.locked.is_empty() {
                crate::v2::check_locks(self, &start)?;
            }
        }
        self.scoring.validate()
    }

    /// The JSON text: compact, schema first, byte-deterministic.
    pub fn to_json(&self) -> String {
        // A struct of strings, integers, booleans and enums: serialization cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The JSON text, indented (fixtures and `enth challenge show`).
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Read a challenge of any version this build has: parse, check the schema (`missing_schema`, `bad_schema`,
    /// `challenge_too_new`), decode (unknown fields refused; v1 through its frozen shapes), migrate v1 to the current
    /// types (no change of meaning), then [`validate`](Self::validate) ([`crate::v2::read`]). The embedded serde path
    /// runs the same reader.
    pub fn from_json(text: &str) -> Result<Challenge, ChallengeError> {
        let value: Value = serde_json::from_str(text).map_err(|e| ChallengeError::BadJson {
            message: e.to_string(),
        })?;
        crate::v2::read(value)
    }
}

/// What the player recreates (design.md §6). R1 has generated targets only; Challenge v2 adds a patch and a draw
/// (lessons). Audio and expectation targets ("create from sound", text goals) are later variants, each a version
/// bump. JSON (`{"kind":…}`, unknown fields refused) is in [`crate::v2`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    /// Generated from the spec by the frozen generator it names: `{"kind":"seed","spec":{…}}` (v1).
    Seed(RandomSpec),
    /// An explicit target patch, held inline: `{"kind":"patch","patch":{…patch file…}}` (v2).
    Patch(InlinePatch),
    /// A seeded draw over the start: `{"kind":"draw","seed":7,"items":[…]}` (v2; [`Draw`]).
    Draw(Draw),
}

/// What the player answers with. R1 answers with a patch; choice and value answers (ear training) come later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AnswerKind {
    /// Rebuild the sound: the player's patch is the answer.
    Patch,
}

/// An id in lane RG's versioned phrase table ([`crate::phrases`]), 1 and up. A phrase's notes never change once
/// released: a different phrase is a new id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhraseId(pub u16);

/// The notes and length a target is heard and scored through (one held note; an octave pair at two velocities; a
/// legato pair). Integers only, so a phrase is exact; [`Phrase::performance`] turns it into a render
/// [`Performance`]. Phrases live in code, in lane RG's table, and are referred to by [`PhraseId`], never persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Phrase {
    /// The notes, in the order they sound (same-time notes play in this order).
    pub notes: &'static [PhraseNote],
    /// The length of the render, in ms: the notes plus room for the release tail.
    pub length_ms: u32,
}

/// One note of a [`Phrase`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhraseNote {
    /// MIDI key, 0–127 (A4 = 69).
    pub key: u8,
    /// MIDI velocity, 1–127; the render gets `velocity / 127`.
    pub velocity: u8,
    /// Note-on, in ms from the start.
    pub start_ms: u32,
    /// How long the note is held, in ms.
    pub hold_ms: u32,
}

impl Phrase {
    /// The render performance: each note at `start_ms / 1000` s with velocity `velocity / 127`, released after
    /// `hold_ms`, in a render of `length_ms`. Plain IEEE divisions of integers, so the same on every platform.
    pub fn performance(&self) -> Performance {
        Performance {
            notes: self
                .notes
                .iter()
                .map(|n| {
                    let on = f64::from(n.start_ms) / 1000.0;
                    let off = f64::from(n.start_ms + n.hold_ms) / 1000.0;
                    Note::new(n.key, f32::from(n.velocity) / 127.0, on, off)
                })
                .collect(),
            duration_s: f64::from(self.length_ms) / 1000.0,
            ..Performance::default()
        }
    }

    /// Check the table entry: at least one note, keys at most 127, velocities 1–127, every note starting before the
    /// end. Lane RG's table test runs it on every phrase.
    pub fn validate(&self) -> Result<(), PhraseError> {
        if self.notes.is_empty() {
            return Err(PhraseError::NoNotes);
        }
        for (index, n) in self.notes.iter().enumerate() {
            if n.key > 127 {
                return Err(PhraseError::BadKey { index });
            }
            if n.velocity == 0 || n.velocity > 127 {
                return Err(PhraseError::BadVelocity { index });
            }
            if n.start_ms >= self.length_ms || n.start_ms.checked_add(n.hold_ms).is_none() {
                return Err(PhraseError::OutsideTheRender { index });
            }
        }
        Ok(())
    }
}

/// Why a [`Phrase`] is not a valid table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhraseError {
    /// It has no notes.
    NoNotes,
    /// A key above 127.
    BadKey {
        /// The note.
        index: usize,
    },
    /// A velocity of 0 or above 127.
    BadVelocity {
        /// The note.
        index: usize,
    },
    /// A note that starts at or after the end of the render, or whose end overflows.
    OutsideTheRender {
        /// The note.
        index: usize,
    },
}

/// What the player starts from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Start {
    /// A template's start patch (lane RG's templates; 0 is the starter superset): `{"kind":"template","template":0}`.
    /// For a [`Target::Seed`] it is the spec's own template.
    Template {
        /// The template id ([`RandomSpec::template`]).
        template: u8,
    },
    /// The step's own start patch, held inline: `{"kind":"patch","patch":{…patch file…}}` (v2). Not for a seed
    /// target, whose generator draws over its template's start.
    Patch {
        /// The patch.
        patch: InlinePatch,
    },
}

/// The help a round allows (report 01 §2.5). Using any marks the round as assisted ([`AssistsUsed`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assists {
    /// Show the sound score live while editing (an assist GR1 tries; off by default: score on submit).
    pub live_meter: bool,
    /// Dim the controls the target did not randomize.
    pub dim_unused: bool,
    /// How many hints may be revealed: `null` for no limit, 0 for none.
    pub hint_budget: Option<u8>,
}

/// The scorers [`ScoringSpec::recreate`] names in this build: sound, structure, combined, time. **Stubs until lanes
/// SS and ST land theirs** (F1r review S12): the stubs score in parameter space under their own ids, so no stub
/// score is ever stored under `sound/1`. The lane that lands a scorer switches its entry here in the same commit
/// (all landed: SS `sound/1`; ST `structure/1` and `combined/1`).
pub const RECREATE_SCORERS: [ScorerId; 4] = [
    ScorerId::SOUND_V1,
    ScorerId::STRUCTURE_V1,
    ScorerId::COMBINED_V1,
    ScorerId::TIME_V1,
];

/// Which scorers a challenge uses, how they combine and where pass is (report 02; design.md §6, "Scoring is
/// additive"). Only sound decides pass.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoringSpec {
    /// The sound scorer: `sound/1` (lane SS's fitted curve), `sound/2` (GR1's refit) or `sound-stub/1`.
    pub sound: ScorerId,
    /// The structure scorer: `structure/1` (lane ST) or `structure-stub/1`.
    pub structure: ScorerId,
    /// The combined score: `combined/1` (lane ST) or `combined-stub/1`.
    pub combined: ScorerId,
    /// The time rule (lane ST): `time/1`, the first passing submit in editor-visible time.
    pub time: ScorerId,
    /// The weight of sound in the combined score, in percent: combined = w·sound + (1 − w)·structure.
    pub sound_weight: u8,
    /// The sound % at or above which a submit passes.
    pub pass_mark: u8,
}

impl ScoringSpec {
    /// The Recreate default at `difficulty` (0–100): this build's [`RECREATE_SCORERS`], the sound weight rising
    /// linearly from 50 % at difficulty 0 to 80 % at 100 (where many routes are valid and structure is ambiguous),
    /// and [`DEFAULT_PASS_MARK`].
    pub fn recreate(difficulty: u8) -> ScoringSpec {
        let [sound, structure, combined, time] = RECREATE_SCORERS;
        ScoringSpec::with_scorers(difficulty, sound, structure, combined, time)
    }

    /// [`recreate`](Self::recreate)'s weights and pass mark with named scorers (the golden challenges and the tests
    /// pin the v1 ids this way, whatever this build's default is).
    pub fn with_scorers(
        difficulty: u8,
        sound: ScorerId,
        structure: ScorerId,
        combined: ScorerId,
        time: ScorerId,
    ) -> ScoringSpec {
        let d = u32::from(difficulty.min(PERCENT_MAX));
        ScoringSpec {
            sound,
            structure,
            combined,
            time,
            sound_weight: (50 + 30 * d / 100) as u8,
            pass_mark: DEFAULT_PASS_MARK,
        }
    }

    /// Whether a sound % passes: at or above the pass mark.
    pub fn passes(&self, sound_percent: f64) -> bool {
        sound_percent >= f64::from(self.pass_mark)
    }

    /// Both percentages at most 100.
    pub fn validate(&self) -> Result<(), ChallengeError> {
        for (field, value) in [
            ("scoring.sound_weight", self.sound_weight),
            ("scoring.pass_mark", self.pass_mark),
        ] {
            if value > PERCENT_MAX {
                return Err(ChallengeError::BadField {
                    field,
                    value: u64::from(value),
                });
            }
        }
        Ok(())
    }
}

/// Title and tags, for lists, dailies and logs; empty for a generated round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeMeta {
    /// A title.
    pub title: String,
    /// Tags.
    pub tags: Vec<String>,
}

/// Why a challenge is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChallengeError {
    /// Not JSON.
    BadJson {
        /// The parser's message.
        message: String,
    },
    /// No `schema` field.
    MissingSchema,
    /// A `schema` that is not `enth.challenge/<N>`.
    BadSchema {
        /// What it was, as JSON.
        found: String,
    },
    /// Newer than this build reads.
    TooNew {
        /// Its version.
        found: u32,
        /// The newest this build reads ([`CHALLENGE_VERSION`]).
        supported: u32,
    },
    /// The right schema, but a missing, unknown or mistyped field.
    BadChallenge {
        /// The decoder's message.
        message: String,
    },
    /// The spec is refused (a newer generator is `code_too_new`).
    Spec(SpecError),
    /// The start template is not the spec's.
    StartMismatch {
        /// The start's template.
        start: u8,
        /// The spec's.
        spec: u8,
    },
    /// A field out of range (phrase 0, a percentage above 100).
    BadField {
        /// The field.
        field: &'static str,
        /// Its value.
        value: u64,
    },
    /// A seed target with a patch start: the generator draws over its template's start (`start_mismatch`).
    StartNotTemplate {
        /// The spec's template.
        spec: u8,
    },
    /// A phrase id neither phrase table has: a newer build's (ids are append-only), `phrase_too_new`.
    UnknownPhrase {
        /// The id.
        phrase: u16,
    },
    /// The draw is refused (`bad_draw`, `draw_too_close`, `out_of_range`).
    Draw(DrawError),
    /// A constraint's selector names nothing on the start, or is listed twice (`bad_selector`).
    Selector {
        /// The selector, as text.
        selector: String,
        /// Why.
        reason: String,
    },
}

impl ChallengeError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            ChallengeError::BadJson { .. } => "bad_json",
            ChallengeError::MissingSchema => "missing_schema",
            ChallengeError::BadSchema { .. } => "bad_schema",
            ChallengeError::TooNew { .. } => "challenge_too_new",
            ChallengeError::BadChallenge { .. } => "bad_challenge",
            ChallengeError::Spec(e) => e.code(),
            ChallengeError::StartMismatch { .. } | ChallengeError::StartNotTemplate { .. } => {
                "start_mismatch"
            }
            ChallengeError::BadField { .. } => "bad_field",
            ChallengeError::UnknownPhrase { .. } => "phrase_too_new",
            ChallengeError::Draw(e) => e.code(),
            ChallengeError::Selector { .. } => "bad_selector",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            ChallengeError::BadJson { .. } | ChallengeError::BadChallenge { .. } => {
                "the challenge file is damaged".to_owned()
            }
            ChallengeError::MissingSchema | ChallengeError::BadSchema { .. } => {
                format!(
                    "a challenge starts with \"schema\": \"{}\" or \"{CHALLENGE_SCHEMA}\"",
                    crate::v2::SCHEMA_V1
                )
            }
            ChallengeError::TooNew { .. } => {
                "made by a newer enth: update enth to open it".to_owned()
            }
            ChallengeError::Spec(e) => e.hint(),
            ChallengeError::StartMismatch { .. } | ChallengeError::StartNotTemplate { .. } => {
                "a generated challenge starts from its spec's template".to_owned()
            }
            ChallengeError::BadField { field, .. } => format!("{field} is out of range"),
            ChallengeError::UnknownPhrase { .. } => {
                "made by a newer enth (a phrase this build does not have): update enth to open it".to_owned()
            }
            ChallengeError::Draw(e) => e.hint(),
            ChallengeError::Selector { .. } => {
                "a selector names a parameter, module (filter1.*), route or macro of the start patch".to_owned()
            }
        }
    }
}

impl core::fmt::Display for ChallengeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ChallengeError::BadJson { message } | ChallengeError::BadChallenge { message } => {
                f.write_str(message)
            }
            ChallengeError::MissingSchema => f.write_str("the challenge has no schema"),
            ChallengeError::BadSchema { found } => write!(f, "schema {found} is not a challenge"),
            ChallengeError::TooNew { found, supported } => write!(
                f,
                "challenge version {found} is newer than this build's {supported}"
            ),
            ChallengeError::Spec(e) => write!(f, "{e}"),
            ChallengeError::StartMismatch { start, spec } => write!(
                f,
                "the start is template {start}, the spec's template is {spec}"
            ),
            ChallengeError::BadField { field, value } => write!(f, "{field} is {value}"),
            ChallengeError::StartNotTemplate { spec } => write!(
                f,
                "the start is a patch, the spec's start is template {spec}"
            ),
            ChallengeError::UnknownPhrase { phrase } => {
                write!(f, "phrase {phrase} is not in this build's tables")
            }
            ChallengeError::Draw(e) => write!(f, "the draw: {e}"),
            ChallengeError::Selector { selector, reason } => {
                write!(f, "selector '{selector}': {reason}")
            }
        }
    }
}

impl std::error::Error for ChallengeError {}

// ---------------------------------------------------------------------------------------------------------------
// Round results

/// How a round ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A submit passed (sound % at or above the pass mark).
    Passed,
    /// The round ended without a passing submit and without a reveal.
    NotPassed,
    /// The player revealed the target before passing.
    GaveUp,
    /// An outcome from a newer build (read tolerantly; never written by this one).
    #[serde(other)]
    Unknown,
}

/// The help a round used. Any of it marks the round as assisted. Read tolerantly: a newer build's assist is ignored,
/// and a field this build added later (`dim_unused`) defaults when missing, so every older result still reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistsUsed {
    /// Hints revealed.
    pub hints: u8,
    /// Whether the live meter was on at any time.
    pub live_meter: bool,
    /// Whether the target was revealed (also [`Outcome::GaveUp`] when it came before a pass).
    pub revealed: bool,
    /// Whether the `dim_unused` assist ([`Assists::dim_unused`]: the modules the target leaves alone are dimmed) was
    /// on at any time while the round could be played (lane HINT, FU1; additive). Written only when true, so a round
    /// without it keeps the bytes it had before the field existed.
    #[serde(skip_serializing_if = "is_false")]
    pub dim_unused: bool,
}

/// `skip_serializing_if` for an additive flag that is written only when set.
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl AssistsUsed {
    /// Whether the round was assisted.
    pub fn any(&self) -> bool {
        self.hints > 0 || self.live_meter || self.revealed || self.dim_unused
    }
}

/// The player's vote on the score after a round: GR1's "mostly agree" measure and the dispute path (design.md §9,
/// "Disputes"). It is saved inside a round, so a value from a newer build reads as [`Agreement::Unknown`] and never
/// makes the saved round unreadable (F1r review M3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agreement {
    /// "Felt too low".
    TooLow,
    /// "About right".
    AboutRight,
    /// "Felt too high".
    TooHigh,
    /// A vote from a newer build (read tolerantly; the editor never offers it).
    #[serde(other)]
    Unknown,
}

/// The blind rating's scale: 1 ("nothing like it") to 5 ("I can't tell them apart"), asked before the score shows
/// (ADR-0007's 2026-10-20 note, review point (a); F1r review S9), so GR1 can refit the curve.
pub const RATING_MAX: u8 = 5;

/// What one round produced, for the results drawer and the round log. Times are editor-visible milliseconds from the
/// shell's clock (core crates read none). JSON with `"schema": "enth.round-result/1"` first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(into = "RoundResultFile", try_from = "RoundResultFile")]
pub struct RoundResult {
    /// The challenge, as the code the round started from.
    pub code: ChallengeCode,
    /// How it ended.
    pub outcome: Outcome,
    /// The recorded score: the first submit's report. `None` when the round ended before any submit.
    pub first_submit: Option<ScoreReport>,
    /// How many times the player submitted.
    pub submits: u32,
    /// Editor-visible time from the start to the first passing submit; `None` without a pass.
    pub time_to_pass_ms: Option<u64>,
    /// Editor-visible time from the start to the end of the round.
    pub active_ms: u64,
    /// The help used.
    pub assists: AssistsUsed,
    /// The player's vote on the score, if they gave one.
    pub agreement: Option<Agreement>,
    /// The player's blind rating of the first submit, 1–[`RATING_MAX`], given before its score showed; `None`
    /// without one.
    pub rating: Option<u8>,
}

/// The on-disk shape. Read tolerantly: unknown fields are ignored and the fields added after the first draft
/// (`rating`) default when missing.
#[derive(Serialize, Deserialize)]
struct RoundResultFile {
    schema: String,
    code: ChallengeCode,
    outcome: Outcome,
    first_submit: Option<ScoreReport>,
    submits: u32,
    time_to_pass_ms: Option<u64>,
    active_ms: u64,
    assists: AssistsUsed,
    agreement: Option<Agreement>,
    #[serde(default)]
    rating: Option<u8>,
}

impl From<RoundResult> for RoundResultFile {
    fn from(r: RoundResult) -> Self {
        RoundResultFile {
            schema: ROUND_RESULT_SCHEMA.to_owned(),
            code: r.code,
            outcome: r.outcome,
            first_submit: r.first_submit,
            submits: r.submits,
            time_to_pass_ms: r.time_to_pass_ms,
            active_ms: r.active_ms,
            assists: r.assists,
            agreement: r.agreement,
            rating: r.rating,
        }
    }
}

impl TryFrom<RoundResultFile> for RoundResult {
    type Error = String;

    fn try_from(f: RoundResultFile) -> Result<Self, Self::Error> {
        if f.schema != ROUND_RESULT_SCHEMA {
            return Err(format!(
                "schema {:?}: this build reads {ROUND_RESULT_SCHEMA}",
                f.schema
            ));
        }
        // A newer build's outcome cannot be checked against the time; every other one is.
        let passed = f.outcome == Outcome::Passed;
        if f.outcome != Outcome::Unknown && passed != f.time_to_pass_ms.is_some() {
            return Err("a passed round has a time to pass, and only a passed round".to_owned());
        }
        if f.first_submit.is_some() != (f.submits > 0) {
            return Err("a round has a first submit exactly when it has submits".to_owned());
        }
        if f.rating.is_some_and(|r| r == 0 || r > RATING_MAX) {
            return Err(format!("a rating is 1 to {RATING_MAX}"));
        }
        Ok(RoundResult {
            code: f.code,
            outcome: f.outcome,
            first_submit: f.first_submit,
            submits: f.submits,
            time_to_pass_ms: f.time_to_pass_ms,
            active_ms: f.active_ms,
            assists: f.assists,
            agreement: f.agreement,
            rating: f.rating,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Category, LATEST_GENERATOR, SpecPreset};

    /// Level 1 with the v1 scorer ids, whatever this build's default is.
    fn level1() -> Challenge {
        let spec = SpecPreset::by_id(1).unwrap().spec(1, 42);
        let mut c = Challenge::recreate(spec, PhraseId(1));
        c.scoring = ScoringSpec::with_scorers(
            0,
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        );
        c
    }

    #[test]
    fn the_json_form_is_pinned() {
        assert_eq!(
            level1().to_json(),
            concat!(
                r#"{"schema":"enth.challenge/1","#,
                r#""target":{"kind":"seed","spec":{"generator":1,"seed":42,"difficulty":0,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}},"#,
                r#""answer":"patch","phrase":1,"start":{"kind":"template","template":0},"constraints":{},"#,
                r#""assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},"#,
                r#""scoring":{"sound":"sound/1","structure":"structure/1","combined":"combined/1","time":"time/1","sound_weight":50,"pass_mark":90},"#,
                r#""meta":{"title":"","tags":[]}}"#
            )
        );
    }

    #[test]
    fn a_challenge_round_trips_through_both_paths() {
        let c = level1();
        let json = c.to_json();
        assert_eq!(Challenge::from_json(&json), Ok(c.clone()));
        assert_eq!(serde_json::from_str::<Challenge>(&json).unwrap(), c);
        assert_eq!(Challenge::from_json(&c.to_json_pretty()), Ok(c));
    }

    #[test]
    fn schemas_are_checked() {
        let json = level1().to_json();
        let with = |schema: &str| json.replacen("\"enth.challenge/1\"", schema, 1);
        // Version 2 is this build's newest (Challenge v2, lane CV2): a v2 file may hold a seed challenge too, and it
        // reads to the same challenge (written back as v1, the lowest version that holds it).
        assert_eq!(
            Challenge::from_json(&with("\"enth.challenge/2\"")),
            Ok(level1())
        );
        assert_eq!(
            Challenge::from_json(&with("\"enth.challenge/3\"")).unwrap_err(),
            ChallengeError::TooNew {
                found: 3,
                supported: 2
            }
        );
        for bad in [
            "\"enth.challenge/0\"",
            "\"enth.challenge/01\"",
            "\"enth.challenge/\"",
            "\"enth.patch/1\"",
            "1",
        ] {
            assert_eq!(
                Challenge::from_json(&with(bad)).unwrap_err().code(),
                "bad_schema",
                "{bad}"
            );
        }
        let none = json.replacen("\"schema\":\"enth.challenge/1\",", "", 1);
        assert_eq!(
            Challenge::from_json(&none),
            Err(ChallengeError::MissingSchema)
        );
        assert_eq!(Challenge::from_json("[").unwrap_err().code(), "bad_json");
        assert_eq!(
            Challenge::from_json("[]").unwrap_err().code(),
            "bad_challenge"
        );
        // The embedded path runs the same reader: a newer version is refused, an older one migrates.
        assert!(serde_json::from_str::<Challenge>(&with("\"enth.challenge/3\"")).is_err());
        assert_eq!(
            serde_json::from_str::<Challenge>(&with("\"enth.challenge/2\"")).unwrap(),
            level1()
        );
    }

    #[test]
    fn unknown_fields_and_floats_are_refused() {
        let json = level1().to_json();
        let extra = json.replacen("\"meta\":", "\"locked\":[],\"meta\":", 1);
        assert_eq!(
            Challenge::from_json(&extra).unwrap_err().code(),
            "bad_challenge"
        );
        let float = json.replacen("\"seed\":42", "\"seed\":42.0", 1);
        assert_eq!(
            Challenge::from_json(&float).unwrap_err().code(),
            "bad_challenge"
        );
        let inner = json.replacen("\"tags\":[]", "\"tags\":[],\"x\":1", 1);
        assert_eq!(
            Challenge::from_json(&inner).unwrap_err().code(),
            "bad_challenge"
        );
        let start = json.replacen("\"template\":0}", "\"template\":0,\"x\":1}", 1);
        assert_ne!(start, json);
        assert_eq!(
            Challenge::from_json(&start).unwrap_err().code(),
            "bad_challenge"
        );
        let target = json.replacen("\"kind\":\"seed\",", "\"kind\":\"seed\",\"x\":1,", 1);
        assert_eq!(
            Challenge::from_json(&target).unwrap_err().code(),
            "bad_challenge"
        );
    }

    #[test]
    fn validation_runs_on_load() {
        let mut c = level1();
        c.start = Start::Template { template: 2 };
        assert_eq!(c.validate().unwrap_err().code(), "start_mismatch");
        assert_eq!(
            Challenge::from_json(&c.to_json()).unwrap_err().code(),
            "start_mismatch"
        );
        let mut c = level1();
        c.phrase = PhraseId(0);
        assert_eq!(c.validate().unwrap_err().code(), "bad_field");
        let mut c = level1();
        c.scoring.pass_mark = 101;
        assert_eq!(c.validate().unwrap_err().code(), "bad_field");
        let mut c = level1();
        c.target = Target::Seed(RandomSpec {
            generator: LATEST_GENERATOR + 1,
            ..*c.spec().unwrap()
        });
        assert_eq!(c.validate().unwrap_err().code(), "code_too_new");
        assert_eq!(
            Challenge::from_json(&c.to_json()).unwrap_err().code(),
            "code_too_new"
        );
    }

    #[test]
    fn accessors_and_codes() {
        let c = level1();
        assert_eq!(c.generator(), Some(1));
        assert_eq!(c.difficulty(), Some(0));
        let short = c.short_code().unwrap();
        assert_eq!(c.code(TargetPrefix(5)), Some(short));
        let mut spec = *c.spec().unwrap();
        spec.amounts[Category::Filter.index()] = Some(10);
        let custom = Challenge::recreate(spec, PhraseId(2));
        assert_eq!(custom.short_code(), None);
        let long = custom.code(TargetPrefix(5)).unwrap();
        assert_eq!(long.spec(), &spec);
        assert_eq!(long.target(), Some(TargetPrefix(5)));
        assert_eq!(long.engine(), Some(EngineVersion::CURRENT));
    }

    #[test]
    fn the_sound_weight_runs_from_50_to_80() {
        assert_eq!(ScoringSpec::recreate(0).sound_weight, 50);
        assert_eq!(ScoringSpec::recreate(50).sound_weight, 65);
        assert_eq!(ScoringSpec::recreate(100).sound_weight, 80);
        assert_eq!(ScoringSpec::recreate(255).sound_weight, 80);
        let s = ScoringSpec::recreate(0);
        assert!(s.passes(90.0));
        assert!(!s.passes(89.999));
    }

    #[test]
    fn phrases_render_as_performances() {
        static NOTES: [PhraseNote; 2] = [
            PhraseNote {
                key: 48,
                velocity: 127,
                start_ms: 0,
                hold_ms: 1500,
            },
            PhraseNote {
                key: 60,
                velocity: 64,
                start_ms: 2000,
                hold_ms: 500,
            },
        ];
        static SILENT: [PhraseNote; 1] = [PhraseNote {
            key: 60,
            velocity: 0,
            start_ms: 0,
            hold_ms: 1,
        }];
        let p = Phrase {
            notes: &NOTES,
            length_ms: 4000,
        };
        assert_eq!(p.validate(), Ok(()));
        let perf = p.performance();
        assert_eq!(perf.duration_s, 4.0);
        assert_eq!(perf.notes.len(), 2);
        assert_eq!(perf.notes[0].velocity, 1.0);
        assert_eq!(perf.notes[0].off_s, 1.5);
        assert_eq!(perf.notes[1].on_s, 2.0);
        assert_eq!(perf.notes[1].velocity, 64.0 / 127.0);
        assert_eq!(
            Phrase {
                notes: &[],
                length_ms: 1
            }
            .validate(),
            Err(PhraseError::NoNotes)
        );
        assert_eq!(
            Phrase {
                notes: &SILENT,
                length_ms: 10
            }
            .validate(),
            Err(PhraseError::BadVelocity { index: 0 })
        );
    }

    /// S12 (F1r review): the scoring seed is a literal that cannot follow the CLI's default silently, and it agrees
    /// with it today; the Recreate default names the stubs until SS and ST land.
    #[test]
    fn the_scoring_seed_is_a_literal_equal_to_the_default_seed() {
        assert_eq!(SCORING_SEED, enth_render::DEFAULT_SEED);
        let s = ScoringSpec::recreate(25);
        assert_eq!(
            [
                s.sound.clone(),
                s.structure.clone(),
                s.combined.clone(),
                s.time.clone()
            ],
            RECREATE_SCORERS
        );
        assert_eq!(s.pass_mark, DEFAULT_PASS_MARK);
        assert_eq!(
            Challenge::recreate(SpecPreset::by_id(2).unwrap().spec(1, 1), PhraseId(1)).scoring,
            s
        );
    }

    /// M3 (F1r review): a vote or an outcome from a newer build reads as `unknown`, and a result written by a newer
    /// build with an extra field still reads.
    #[test]
    fn newer_votes_outcomes_and_fields_read_tolerantly() {
        assert_eq!(
            serde_json::from_str::<Agreement>("\"disagree\"").unwrap(),
            Agreement::Unknown
        );
        assert_eq!(
            serde_json::from_str::<Outcome>("\"timed_out\"").unwrap(),
            Outcome::Unknown
        );
        let r = RoundResult {
            code: ChallengeCode::short(SpecPreset::by_id(1).unwrap().spec(1, 3)).unwrap(),
            outcome: Outcome::NotPassed,
            first_submit: None,
            submits: 0,
            time_to_pass_ms: None,
            active_ms: 10,
            assists: AssistsUsed::default(),
            agreement: None,
            rating: Some(4),
        };
        let json = serde_json::to_string(&r).unwrap();
        let newer = json
            .replacen(
                "\"agreement\":null",
                "\"agreement\":\"disagree\",\"streak\":3",
                1,
            )
            .replacen("\"outcome\":\"not_passed\"", "\"outcome\":\"timed_out\"", 1)
            .replacen("\"hints\":0", "\"hints\":0,\"slow_motion\":true", 1);
        assert_ne!(newer, json);
        let back: RoundResult = serde_json::from_str(&newer).unwrap();
        assert_eq!(
            (back.agreement, back.outcome, back.rating),
            (Some(Agreement::Unknown), Outcome::Unknown, Some(4))
        );
        let old = json.replacen(",\"rating\":4", "", 1);
        assert_ne!(old, json);
        assert_eq!(
            serde_json::from_str::<RoundResult>(&old).unwrap().rating,
            None
        );
        for bad in ["0", "6"] {
            let text = json.replacen("\"rating\":4", &format!("\"rating\":{bad}"), 1);
            assert!(serde_json::from_str::<RoundResult>(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn assists_used() {
        assert!(!AssistsUsed::default().any());
        assert!(
            AssistsUsed {
                hints: 1,
                ..AssistsUsed::default()
            }
            .any()
        );
        let dimmed = AssistsUsed {
            dim_unused: true,
            ..AssistsUsed::default()
        };
        assert!(dimmed.any(), "the dim assist marks the round assisted");
    }

    /// `dim_unused` is additive (lane HINT, FU1): written only when set, so a result without it keeps its bytes, and a
    /// result written before the field existed reads with it false.
    #[test]
    fn the_dim_assist_is_written_only_when_used_and_old_results_read() {
        let plain = serde_json::to_string(&AssistsUsed::default()).unwrap();
        assert_eq!(plain, r#"{"hints":0,"live_meter":false,"revealed":false}"#);
        let dimmed = AssistsUsed {
            dim_unused: true,
            ..AssistsUsed::default()
        };
        let text = serde_json::to_string(&dimmed).unwrap();
        assert_eq!(
            text,
            r#"{"hints":0,"live_meter":false,"revealed":false,"dim_unused":true}"#
        );
        assert_eq!(serde_json::from_str::<AssistsUsed>(&text).unwrap(), dimmed);
        let old: AssistsUsed =
            serde_json::from_str(r#"{"hints":2,"live_meter":false,"revealed":true}"#).unwrap();
        assert!(!old.dim_unused && old.any());
    }
}
