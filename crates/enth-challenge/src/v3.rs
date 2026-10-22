//! **Owner: lane CV3** (Challenge v3, freeze point F1e; the F1e judge reviews it with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e). The reference is docs/challenge.md, "Challenge v3", and
//! docs/ear.md, "Where R4 lives".
//!
//! Challenge v2 has `Target::Draw` and `Target::Patch`, and every answer is a patch. R4's games need a target that is
//! not a patch and answers that are not patches. v3 adds, to the same types in [`crate::model`]:
//!
//! | Piece | JSON | What |
//! | --- | --- | --- |
//! | `Target::Ear(EarSpec)` | `{"kind":"ear","spec":{"game":1,"generator":1,"level":1,"seed":42,"count":16}}` | a question set from a frozen ear generator ([`EarSpec`]): the parameter games' items in [`crate::ear::params`], the music games' in `enth-music` |
//! | `AnswerKind::{Choice, Value, Taps}` | `"choice"`, `"value"`, `"taps"` | a label or clip index, judged exactly (`ear/1`); a value on a knob or slider, judged as an error in perceptual units (`ear-value/1`); tap times, judged on their timing after a best-fit offset and tempo (`taps/1`). `Patch` stays the default |
//!
//! **Additive, like v2.** v1 and v2 files read as before, through their own readers, and a challenge is written at
//! the lowest version that holds it ([`Challenge::version`]): 3 only with an ear target or a non-patch answer, so no
//! R1 to R3 challenge changes its bytes, and the golden challenges and generator v1's draws do not move. A v2 file
//! holding a v3 piece is refused (`bad_challenge`), as a v1 file holding a v2 piece is; a v3 file holding a
//! v1- or v2-expressible challenge reads, and is written back at its lowest version.
//!
//! **Strict, like every target.** An ear target's questions carry their own notes and timbres, so the challenge's
//! own phrase, start and constraints have one spelling each: phrase 0, the template 0 start (never played) and no
//! constraints (`bad_ear_target`). Its answer is the one its game asks for (`bad_answer`), never a patch: an ear
//! target is judged with no render. And a choice, value or taps answer needs an ear target (`bad_answer`): the
//! parameter games' items are v2 draws inside the ear generator, not challenges of their own (docs/ear.md).
//!
//! **Scoring a non-patch answer.** A choice, value or taps answer has one scorer, its ear scorer
//! ([`crate::ear::scorers`]). It fills the `sound` slot (the score that decides pass) and the `structure` and
//! `combined` slots repeat it, with a sound weight of 100, so `combined = sound`; the time rule stays `time/1`
//! (`bad_scoring` otherwise). The form is the same six keys as before, so nothing is hidden and nothing is lost on a
//! write ([`ScoringSpec::ear`] builds it).
//!
//! The question-to-sound side is [`crate::ear`]; the ear-challenge code (kind 3 behind version 15) is
//! [`crate::ear::code`].

use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ear::scorers;
use crate::model::{
    AnswerKind, Assists, Challenge, ChallengeError, ChallengeMeta, PhraseId, ScoringSpec, Start,
    Target,
};
use crate::score::ScorerId;
use crate::spec::MAX_SEED;
use crate::v2::Constraints;

/// The `schema` string of a v3 challenge.
pub const SCHEMA_V3: &str = "enth.challenge/3";

/// The phrase of an ear target: 0, no table's phrase (its questions carry their own notes).
pub const EAR_PHRASE: PhraseId = PhraseId(0);

/// The start of an ear target: template 0, never played (the questions carry their own timbres).
pub const EAR_TEMPLATE: u8 = 0;

/// The most questions one ear target asks (a 40-trial Ear check fits; a set of 64 renders in about 2-4 s).
pub const MAX_EAR_COUNT: u8 = 64;

/// The pass mark of [`ScoringSpec::ear`]: 80 % of the set right, about the 79 % the drills train at (3-down/1-up,
/// docs/ear.md). A challenge keeps its own mark, so a later default never changes an old challenge's pass.
pub const EAR_PASS_MARK: u8 = 80;

/// A question set from a frozen ear generator: the game, the generator version, the level, the seed and how many
/// questions. Integers only, the seed below 2⁵³. JSON `{"game":1,"generator":1,"level":1,"seed":42,"count":16}`,
/// unknown fields refused.
///
/// **A stand-in for `enth_music::ear::EarSpec`** (lane MUS, agreed field for field: the same fields in the same order,
/// the same derives and JSON). When MUS's type is merged, this one becomes `pub use enth_music::ear::EarSpec`, and
/// nothing else changes (docs/challenge.md, "Challenge v3": the seam with `enth-music`).
///
/// Games keep docs/ear.md's numbers: 1 Spot the change, 2 Dial it in and 3 Ear check are the parameter games
/// ([`crate::ear::params`]); 4 Frequency finder, 5 Interval stacks, 6 Chord quality, 7 Rhythm pattern and 8 up are
/// `enth-music`'s. Item `i` draws from its own keyed stream (`derive(seed, fnv1a64("game/i/level"))`), so a
/// question never depends on another, and every player of a spec hears the same set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarSpec {
    /// The game (docs/ear.md's number, 1 and up).
    pub game: u8,
    /// The ear generator version, 1 and up. A released version never changes its questions.
    pub generator: u8,
    /// The level on the game's ladder, 1 and up.
    pub level: u8,
    /// The seed, at most 2⁵³ − 1 (a code keeps 40 bits of it).
    pub seed: u64,
    /// How many questions, 1 to [`MAX_EAR_COUNT`].
    pub count: u8,
}

impl EarSpec {
    /// Check the spec: every field in range (`bad_field`, `out_of_range` for the seed), then a game, generator and
    /// level this build has (`code_too_new`: game ids, generator versions and levels are append-only, so one this
    /// build lacks is a newer build's).
    pub fn validate(&self) -> Result<(), EarError> {
        self.answer().map(|_| ())
    }

    /// The answer the spec's game asks for at its level, after [`validate`](Self::validate)'s checks: a choice for
    /// Spot the change and Ear check, a patch for Dial it in (a Recreate round, so never an ear target's answer).
    pub fn answer(&self) -> Result<AnswerKind, EarError> {
        for (field, value) in [
            ("game", self.game),
            ("generator", self.generator),
            ("level", self.level),
            ("count", self.count),
        ] {
            if value == 0 {
                return Err(EarError::BadField {
                    field,
                    value: u64::from(value),
                });
            }
        }
        if self.count > MAX_EAR_COUNT {
            return Err(EarError::BadField {
                field: "count",
                value: u64::from(self.count),
            });
        }
        if self.seed > MAX_SEED {
            return Err(EarError::SeedOutOfRange { seed: self.seed });
        }
        crate::ear::answer_kind(self)
    }
}

/// Why an [`EarSpec`] is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarError {
    /// A field no encoder writes: a game, generator, level or count of 0, or a count above [`MAX_EAR_COUNT`]
    /// (`bad_field`).
    BadField {
        /// The field.
        field: &'static str,
        /// Its value.
        value: u64,
    },
    /// A seed from 2⁵³ (`out_of_range`).
    SeedOutOfRange {
        /// The seed.
        seed: u64,
    },
    /// A game this build does not have (`code_too_new`): game ids are append-only, so it is a newer build's.
    GameTooNew {
        /// The game.
        game: u8,
    },
    /// An ear generator version this build does not have for the game (`code_too_new`).
    GeneratorTooNew {
        /// The game.
        game: u8,
        /// The version asked for.
        generator: u8,
        /// The newest this build has for the game.
        latest: u8,
    },
    /// A level above the top of the game's ladder in this build (`code_too_new`).
    LevelTooNew {
        /// The game.
        game: u8,
        /// The level asked for.
        level: u8,
        /// The top level this build has.
        top: u8,
    },
}

impl EarError {
    /// A stable snake-case code. A newer game, generator or level is `code_too_new`, as a newer Recreate generator
    /// is.
    pub fn code(&self) -> &'static str {
        match self {
            EarError::BadField { .. } => "bad_field",
            EarError::SeedOutOfRange { .. } => "out_of_range",
            EarError::GameTooNew { .. }
            | EarError::GeneratorTooNew { .. }
            | EarError::LevelTooNew { .. } => "code_too_new",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            EarError::BadField { field, .. } => {
                format!("the ear game's {field} is out of range: the challenge is damaged")
            }
            EarError::SeedOutOfRange { .. } => "a seed is below 2^53".to_owned(),
            EarError::GameTooNew { .. }
            | EarError::GeneratorTooNew { .. }
            | EarError::LevelTooNew { .. } => {
                "made by a newer enth: update enth to play it".to_owned()
            }
        }
    }
}

impl fmt::Display for EarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EarError::BadField { field, value } => write!(f, "ear {field} {value} is not valid"),
            EarError::SeedOutOfRange { seed } => write!(f, "seed {seed} is 2^53 or more"),
            EarError::GameTooNew { game } => write!(f, "ear game {game} is not in this build"),
            EarError::GeneratorTooNew {
                game,
                generator,
                latest,
            } => write!(
                f,
                "ear generator {generator} of game {game} is newer than this build's {latest}"
            ),
            EarError::LevelTooNew { game, level, top } => write!(
                f,
                "level {level} of ear game {game} is above this build's top level {top}"
            ),
        }
    }
}

impl std::error::Error for EarError {}

/// Whether `c` needs Challenge v3: an ear target, or a choice, value or taps answer.
pub(crate) fn needs_v3(c: &Challenge) -> bool {
    matches!(c.target, Target::Ear(_)) || c.answer != AnswerKind::Patch
}

/// [`Challenge::validate`] for a challenge that [`needs_v3`]: an ear target with its strict phrase, start and
/// constraints, the answer its game asks for, and its ear scorer in every scoring slot.
pub(crate) fn validate(c: &Challenge) -> Result<(), ChallengeError> {
    let Target::Ear(spec) = &c.target else {
        return Err(ChallengeError::Answer {
            reason: format!(
                "a {} answer needs an ear target (a draw, patch or seed target is answered with a patch)",
                answer_text(c.answer)
            ),
        });
    };
    let asked = spec.answer().map_err(ChallengeError::Ear)?;
    if c.phrase != EAR_PHRASE {
        return Err(ChallengeError::EarTarget {
            reason: format!(
                "the phrase is {}: an ear target's questions carry their own notes, so its phrase is 0",
                c.phrase.0
            ),
        });
    }
    if c.start
        != (Start::Template {
            template: EAR_TEMPLATE,
        })
    {
        return Err(ChallengeError::EarTarget {
            reason: "an ear target starts from template 0 (its questions carry their own timbres)"
                .to_owned(),
        });
    }
    if !c.constraints.is_empty() {
        return Err(ChallengeError::EarTarget {
            reason: "an ear target has nothing to edit, so it takes no constraint".to_owned(),
        });
    }
    if asked == AnswerKind::Patch {
        return Err(ChallengeError::Answer {
            reason: format!(
                "game {} asks for a patch: its rounds are v2 draw challenges (a constrained Recreate round, \
                 ear::params::dial_it_in), not an ear target",
                spec.game
            ),
        });
    }
    if c.answer != asked {
        return Err(ChallengeError::Answer {
            reason: format!(
                "game {} at level {} asks for a {} answer, not a {} answer",
                spec.game,
                spec.level,
                answer_text(asked),
                answer_text(c.answer)
            ),
        });
    }
    check_ear_scoring(&c.scoring, c.answer)?;
    c.scoring.validate()
}

/// The scoring of a non-patch answer: its ear scorer (any version of the right name) in the sound, structure and
/// combined slots, a sound weight of 100 and `time/1`.
fn check_ear_scoring(s: &ScoringSpec, answer: AnswerKind) -> Result<(), ChallengeError> {
    let id = scorers::scorer_for(answer);
    let name = id.as_ref().map_or("", |id| id.name());
    let bad = |reason: String| Err(ChallengeError::Scoring { reason });
    if s.sound.name() != name {
        return bad(format!(
            "a {} answer is scored by {name}, not {}",
            answer_text(answer),
            s.sound
        ));
    }
    if s.structure != s.sound || s.combined != s.sound {
        return bad(format!(
            "a {} answer has one scorer: structure and combined repeat {}",
            answer_text(answer),
            s.sound
        ));
    }
    if s.sound_weight != 100 {
        return bad(format!(
            "a {} answer is weighted 100 % (its scorer is the whole score), not {} %",
            answer_text(answer),
            s.sound_weight
        ));
    }
    if s.time != ScorerId::TIME_V1 {
        return bad(format!("the time rule is time/1, not {}", s.time));
    }
    Ok(())
}

/// An answer kind as its JSON text.
fn answer_text(a: AnswerKind) -> &'static str {
    match a {
        AnswerKind::Patch => "patch",
        AnswerKind::Choice => "choice",
        AnswerKind::Value => "value",
        AnswerKind::Taps => "taps",
    }
}

impl ScoringSpec {
    /// The scoring of a choice, value or taps answer: its ear scorer ([`crate::ear::scorers::scorer_for`]) in the
    /// sound, structure and combined slots, a sound weight of 100, `time/1` and [`EAR_PASS_MARK`]. `None` for a
    /// patch answer, which [`ScoringSpec::recreate`] scores.
    pub fn ear(answer: AnswerKind) -> Option<ScoringSpec> {
        let id = scorers::scorer_for(answer)?;
        Some(ScoringSpec {
            sound: id.clone(),
            structure: id.clone(),
            combined: id,
            time: ScorerId::TIME_V1,
            sound_weight: 100,
            pass_mark: EAR_PASS_MARK,
        })
    }
}

impl Challenge {
    /// The ear challenge of `spec`: the answer its game asks for, phrase 0, the template 0 start, no constraint, the
    /// default assists, [`ScoringSpec::ear`] and no title; validated. A spec this build cannot play is refused as
    /// [`validate`](Self::validate) refuses it (`code_too_new` for a newer game, generator or level; `bad_answer` for
    /// Dial it in, whose rounds are v2 draw challenges).
    pub fn ear(spec: EarSpec) -> Result<Challenge, ChallengeError> {
        let answer = spec.answer().map_err(ChallengeError::Ear)?;
        let scoring = ScoringSpec::ear(answer).ok_or_else(|| ChallengeError::Answer {
            reason: format!(
                "game {} asks for a patch: its rounds are v2 draw challenges (ear::params::dial_it_in)",
                spec.game
            ),
        })?;
        let c = Challenge {
            target: Target::Ear(spec),
            answer,
            phrase: EAR_PHRASE,
            start: Start::Template {
                template: EAR_TEMPLATE,
            },
            constraints: Constraints::default(),
            assists: Assists::default(),
            scoring,
            meta: ChallengeMeta::default(),
        };
        c.validate()?;
        Ok(c)
    }

    /// The ear spec, for an ear target.
    pub fn ear_spec(&self) -> Option<&EarSpec> {
        match &self.target {
            Target::Ear(spec) => Some(spec),
            Target::Seed(_) | Target::Patch(_) | Target::Draw(_) => None,
        }
    }
}

/// The v3 file: the same fields as v2's, with the target's and the answer's v3 forms.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV3 {
    #[serde(rename = "schema")]
    _schema: String,
    target: Target,
    answer: AnswerKind,
    phrase: PhraseId,
    start: Start,
    constraints: Constraints,
    assists: Assists,
    scoring: ScoringSpec,
    meta: ChallengeMeta,
}

/// Decode a v3 file (the caller has checked the schema and validates the result).
pub(crate) fn read(value: Value) -> Result<Challenge, ChallengeError> {
    let f = serde_json::from_value::<FileV3>(value).map_err(|e| ChallengeError::BadChallenge {
        message: e.to_string(),
    })?;
    Ok(Challenge {
        target: f.target,
        answer: f.answer,
        phrase: f.phrase,
        start: f.start,
        constraints: f.constraints,
        assists: f.assists,
        scoring: f.scoring,
        meta: f.meta,
    })
}

/// The refusal of a v3 piece in an older file: `bad_challenge`, as a v2 piece in a v1 file is.
pub(crate) fn too_new_for(schema: &str) -> ChallengeError {
    ChallengeError::BadChallenge {
        message: format!(
            "an ear target or a choice, value or taps answer is a Challenge v3 piece: a {schema} file cannot hold it"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ear::params::SPOT_THE_CHANGE;
    use crate::model::CHALLENGE_VERSION;

    fn spot(seed: u64) -> EarSpec {
        EarSpec {
            game: SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed,
            count: 16,
        }
    }

    #[test]
    fn the_json_form_is_pinned() {
        let c = Challenge::ear(spot(42)).unwrap();
        assert_eq!(c.version(), 3);
        assert_eq!(
            c.to_json(),
            concat!(
                r#"{"schema":"enth.challenge/3","#,
                r#""target":{"kind":"ear","spec":{"game":1,"generator":1,"level":1,"seed":42,"count":16}},"#,
                r#""answer":"choice","phrase":0,"start":{"kind":"template","template":0},"constraints":{},"#,
                r#""assists":{"live_meter":false,"dim_unused":false,"hint_budget":null},"#,
                r#""scoring":{"sound":"ear/1","structure":"ear/1","combined":"ear/1","time":"time/1","sound_weight":100,"pass_mark":80},"#,
                r#""meta":{"title":"","tags":[]}}"#
            )
        );
        assert_eq!(CHALLENGE_VERSION, 3);
    }

    #[test]
    fn an_ear_challenge_round_trips_through_both_paths() {
        let c = Challenge::ear(spot(7)).unwrap();
        assert_eq!(Challenge::from_json(&c.to_json()), Ok(c.clone()));
        assert_eq!(Challenge::from_json(&c.to_json_pretty()), Ok(c.clone()));
        assert_eq!(serde_json::from_str::<Challenge>(&c.to_json()).unwrap(), c);
        assert_eq!(c.ear_spec(), Some(&spot(7)));
        assert_eq!(c.spec(), None);
        assert_eq!(c.draw(), None);
        assert_eq!(c.code(crate::TargetPrefix(0)), None);
        assert_eq!(c.short_code(), None);
    }

    #[test]
    fn a_v2_file_cannot_hold_a_v3_piece() {
        let c = Challenge::ear(spot(1)).unwrap();
        for schema in ["enth.challenge/2", "enth.challenge/1"] {
            let text = c.to_json().replacen(SCHEMA_V3, schema, 1);
            assert_eq!(
                Challenge::from_json(&text).unwrap_err().code(),
                "bad_challenge",
                "{schema}"
            );
            assert!(
                serde_json::from_str::<Challenge>(&text).is_err(),
                "{schema}"
            );
        }
        // A choice answer alone (on a draw target) is a v3 piece too: a v2 file refuses it before validation.
        let level1 = crate::model::Challenge::recreate(
            crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 42),
            PhraseId(1),
        );
        let text = level1
            .to_json()
            .replacen("\"enth.challenge/1\"", "\"enth.challenge/2\"", 1)
            .replacen("\"answer\":\"patch\"", "\"answer\":\"choice\"", 1);
        let e = Challenge::from_json(&text).unwrap_err();
        assert_eq!(e.code(), "bad_challenge", "{e}");
    }

    #[test]
    fn a_v3_file_reads_older_challenges_and_writes_them_back_at_their_version() {
        let level1 = crate::model::Challenge::recreate(
            crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 42),
            PhraseId(1),
        );
        let text = level1
            .to_json()
            .replacen("\"enth.challenge/1\"", "\"enth.challenge/3\"", 1);
        let read = Challenge::from_json(&text).unwrap();
        assert_eq!(read, level1);
        assert_eq!(read.version(), 1);
        assert!(
            read.to_json()
                .starts_with("{\"schema\":\"enth.challenge/1\"")
        );
        let newer = level1
            .to_json()
            .replacen("\"enth.challenge/1\"", "\"enth.challenge/4\"", 1);
        assert_eq!(
            Challenge::from_json(&newer).unwrap_err(),
            ChallengeError::TooNew {
                found: 4,
                supported: 3
            }
        );
    }

    #[test]
    fn an_ear_target_is_strict() {
        let good = Challenge::ear(spot(3)).unwrap();
        let refuse = |c: Challenge, code: &str| {
            let e = c.validate().unwrap_err();
            assert_eq!(e.code(), code, "{e}");
            // The file path refuses it too.
            assert_eq!(Challenge::from_json(&c.to_json()).unwrap_err().code(), code);
        };
        // Phrase 0, and only 0.
        for phrase in [1, 2, 4, 9] {
            let mut c = good.clone();
            c.phrase = PhraseId(phrase);
            refuse(c, "bad_ear_target");
        }
        // The template 0 start, and only it.
        let mut c = good.clone();
        c.start = Start::Template { template: 1 };
        refuse(c, "bad_ear_target");
        let mut c = good.clone();
        c.start = Start::Patch {
            patch: crate::v2::InlinePatch::new(crate::random::start(0).unwrap()),
        };
        refuse(c, "bad_ear_target");
        // No constraint.
        let mut c = good.clone();
        c.constraints.editable = vec!["filter1.cutoff".parse().unwrap()];
        refuse(c, "bad_ear_target");
        // Never a patch answer, and only the game's answer.
        for answer in [AnswerKind::Patch, AnswerKind::Value, AnswerKind::Taps] {
            let mut c = good.clone();
            c.answer = answer;
            refuse(c, "bad_answer");
        }
        // Its scorer in every slot, weighted 100, with time/1.
        let mut c = good.clone();
        c.scoring = ScoringSpec::recreate(0);
        refuse(c, "bad_scoring");
        let mut c = good.clone();
        c.scoring.structure = ScorerId::STRUCTURE_V1;
        refuse(c, "bad_scoring");
        let mut c = good.clone();
        c.scoring.sound_weight = 50;
        refuse(c, "bad_scoring");
        let mut c = good.clone();
        c.scoring.time = ScorerId::new("time", 2);
        refuse(c, "bad_scoring");
        let mut c = good.clone();
        c.scoring.pass_mark = 101;
        refuse(c, "bad_field");
        // A newer ear scorer of the right name is a later build's to judge, not a malformed challenge.
        let mut c = good;
        let ear2 = ScorerId::new("ear", 2);
        c.scoring.sound = ear2.clone();
        c.scoring.structure = ear2.clone();
        c.scoring.combined = ear2;
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn a_non_patch_answer_needs_an_ear_target() {
        let level1 = crate::model::Challenge::recreate(
            crate::spec::SpecPreset::by_id(1).unwrap().spec(1, 42),
            PhraseId(1),
        );
        for answer in [AnswerKind::Choice, AnswerKind::Value, AnswerKind::Taps] {
            let mut c = level1.clone();
            c.answer = answer;
            c.scoring = ScoringSpec::ear(answer).unwrap();
            assert_eq!(c.version(), 3);
            assert_eq!(c.validate().unwrap_err().code(), "bad_answer");
            assert_eq!(
                Challenge::from_json(&c.to_json()).unwrap_err().code(),
                "bad_answer"
            );
        }
    }

    #[test]
    fn ear_specs_are_checked() {
        let code = |s: EarSpec| s.validate().map_err(|e| e.code());
        assert_eq!(code(spot(MAX_SEED)), Ok(()));
        assert_eq!(
            code(EarSpec {
                seed: MAX_SEED + 1,
                ..spot(0)
            }),
            Err("out_of_range")
        );
        for zero in [
            EarSpec { game: 0, ..spot(0) },
            EarSpec {
                generator: 0,
                ..spot(0)
            },
            EarSpec {
                level: 0,
                ..spot(0)
            },
            EarSpec {
                count: 0,
                ..spot(0)
            },
            EarSpec {
                count: MAX_EAR_COUNT + 1,
                ..spot(0)
            },
        ] {
            assert_eq!(code(zero), Err("bad_field"), "{zero:?}");
        }
        assert_eq!(
            code(EarSpec {
                count: MAX_EAR_COUNT,
                ..spot(0)
            }),
            Ok(())
        );
        // Newer games, generators and levels are a newer build's.
        assert_eq!(
            code(EarSpec {
                game: 200,
                ..spot(0)
            }),
            Err("code_too_new")
        );
        assert_eq!(
            code(EarSpec {
                generator: 2,
                ..spot(0)
            }),
            Err("code_too_new")
        );
        assert_eq!(
            code(EarSpec {
                level: 99,
                ..spot(0)
            }),
            Err("code_too_new")
        );
        // A spec's JSON is strict.
        assert!(
            serde_json::from_str::<EarSpec>(
                r#"{"game":1,"generator":1,"level":1,"seed":1,"count":1,"extra":0}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<EarSpec>(r#"{"game":1,"generator":1,"level":1,"seed":1}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<EarSpec>(
                r#"{"game":1,"generator":1,"level":1,"seed":1.0,"count":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn dial_it_in_is_no_ear_target() {
        let spec = EarSpec {
            game: crate::ear::params::DIAL_IT_IN,
            ..spot(1)
        };
        assert_eq!(spec.answer(), Ok(AnswerKind::Patch));
        assert_eq!(Challenge::ear(spec).unwrap_err().code(), "bad_answer");
    }

    #[test]
    fn a_sibling_keeps_the_game_and_changes_the_seed() {
        let c = Challenge::ear(spot(1)).unwrap();
        let s = c.with_seed(99).unwrap();
        assert_eq!(s.ear_spec().unwrap().seed, 99);
        assert_eq!(s.ear_spec().unwrap().game, SPOT_THE_CHANGE);
        assert_eq!(
            c.with_seed(MAX_SEED + 1).unwrap_err().code(),
            "out_of_range"
        );
    }
}
