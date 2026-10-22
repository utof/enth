//! **Owner: lane CV3** (Challenge v3, freeze point F1e); **G4** owns [`ladder`]. Pre-declared by lane S0e (R4/R5,
//! freeze point F0e). The reference is docs/ear.md ("Where R4 lives", "Rendering and playback") and docs/challenge.md,
//! "Challenge v3".
//!
//! What an ear question needs in `enth-challenge` (L4), which `enth-music` (L1) cannot hold: a question to sound, the
//! parameter games' items on v2 draws, the scorers and the ear-challenge code. One file each, so CV3 and G4 never edit
//! each other's:
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`question`] | a question set from an ear target ([`question::prepare`]), what each question plays ([`question::Clip`]: a patch through a phrase, or a [`question::Stimulus`] of a timbre from the frozen pool `ear/timbres-v1/`, notes within C1 to C7, at most 8 at once and 10 s, and an optional EQ band) and its key | CV3 |
//! | [`params`] | the parameter games' items (Spot the change, Dial it in, Ear check) as v2 draws: the options, the control that moves and its distance in parameter steps | CV3 |
//! | [`ladder`] | those games' difficulty ladder in parameter steps, and the *not measured* list for the pairs `sound/1` cannot hear | G4 |
//! | [`scorers`] | the scorers `ear/1`, `ear-value/1` and `taps/1`: answer keys are seeded integers, so an ear result is judged with no render, identically on every platform and engine version | CV3 |
//! | [`code`] | the ear-challenge code (kind 3 behind version 15, an `EarSpec` on a fixed ladder for ear duels) | CV3 |
//!
//! **The two families.** An [`EarSpec`]'s game decides who generates its questions (docs/ear.md, "The ear
//! generator"): games 1 to 3, the parameter games, need the catalogue and are generated here ([`params`]); games 4 and
//! up (Frequency finder, Interval stacks, Chord quality, Rhythm pattern; then progressions, note naming, tap-back) are
//! `enth-music`'s ear generator v1 (lane MUS). [`answer_kind`] is the one registry both families answer through.
//!
//! **The seam with `enth-music`** (wired by lane F1E, the F1e review's M3 and M5). [`EarSpec`] is `enth-music`'s type,
//! re-exported; [`validate`] checks a spec for both families; [`music`] maps `enth_music::ear::EarSpec::validate_music`
//! and `Game::answer_kind(level)` into the one registry; and [`question::prepare`] turns each
//! `enth_music::ear::Question` into a [`question::Stimulus`] (timbre, notes, length, band) and a [`scorers::EarKey`],
//! with its labels (`Options::labels`) and skill (`Question::skill`). One [`question::EqBand`]: `enth-music`'s. What
//! MUS provides: timbre ids into `timbres-v1` (0 sine, 1 saw low-pass, 2 pluck, 3 organ table, 4 pad, 5 noise-burst
//! click, 6 sustained pink noise); notes as (key 24 to 96, velocity 1 to 127, start and hold in ms); `length_ms` with
//! the release tail; the band's integers; the key as an option index (with the option count), or onsets and a window
//! for taps.

pub mod code;
pub mod ladder;
pub mod params;
pub mod question;
pub mod scorers;

use crate::model::AnswerKind;
use crate::spec::MAX_SEED;
use crate::v3::{EarError, MAX_EAR_COUNT};

pub use crate::v3::EarSpec;

/// **The check of an ear spec, for both families** (the F1e review's M3; CV3's checks, moved here when `EarSpec`
/// became `enth-music`'s type): every field in range (`bad_field` for a game, generator, level or count of 0 or a
/// count above [`MAX_EAR_COUNT`]; `out_of_range` for a seed from 2⁵³), then a game, generator and level this build has
/// (`code_too_new`: they are append-only, so one this build lacks is a newer build's). Returns the answer the game asks
/// for at its level: a choice for Spot the change, Ear check and the label games, taps for Tap back, a patch for Dial
/// it in (a Recreate round, so never an ear target's answer). Every challenge, code and session check goes through
/// here; `enth-music`'s `EarSpec::validate_music` knows only the music games.
pub fn validate(spec: &EarSpec) -> Result<AnswerKind, EarError> {
    for (field, value) in [
        ("game", spec.game),
        ("generator", spec.generator),
        ("level", spec.level),
        ("count", spec.count),
    ] {
        if value == 0 {
            return Err(EarError::BadField {
                field,
                value: u64::from(value),
            });
        }
    }
    if spec.count > MAX_EAR_COUNT {
        return Err(EarError::BadField {
            field: "count",
            value: u64::from(spec.count),
        });
    }
    if spec.seed > MAX_SEED {
        return Err(EarError::SeedOutOfRange { seed: spec.seed });
    }
    answer_kind(spec)
}

/// The most items [`find_item`] looks at in a parameter game (its items are numbered by a byte).
pub const MAX_PARAMS_SCAN: u32 = 256;

/// **The one resolver of a burst's focus, over both families** (should-fix S3): the first item at or after `from`
/// of `spec`'s game, level and seed whose question exercises `skill` (a Review key: `ear.interval.m3`, `svf.cutoff`),
/// or `None` when the game never asks it within its scan (or this build cannot play the spec). Items are keyed
/// streams, so the answer is a function of the spec and `from`, and two players given the same spec get the same
/// burst. The spec's `count` is not a bound: a focused run asks item `first` onwards.
///
/// | Game | How |
/// | --- | --- |
/// | 4 and up (music) | `enth_music::ear::find_skill_item`: the right answer's skill, over up to 4,096 items |
/// | 1 Spot the change | the moved option's control (`svf.cutoff`), over items `from` to 255 |
/// | 2 Dial it in | the round's one editable control, over items `from` to 255 |
/// | 3 Ear check | a set measures one control ([`params::ear_check_control`]): `from` itself when that control is the skill's, else `None`; a staircase on a chosen control is [`params::ear_check_on`] |
pub fn find_item(spec: &EarSpec, skill: &str, from: u32) -> Option<u32> {
    validate(spec).ok()?;
    if !params::is_params_game(spec.game) {
        let game = enth_music::ear::Game::from_id(spec.game)?;
        return enth_music::ear::find_skill_item(game, spec.level, spec.seed, skill, from)
            .map(|(item, _)| item);
    }
    let base = params::base();
    let skill_of = |addr: &enth_patch::ParamAddr| question::control_skill(&base, addr);
    match spec.game {
        params::EAR_CHECK => {
            let control = params::ear_check_control(spec).ok()?;
            (from < MAX_PARAMS_SCAN && skill_of(&control).as_deref() == Some(skill)).then_some(from)
        }
        params::SPOT_THE_CHANGE => (from..MAX_PARAMS_SCAN).find(|&i| {
            params::spot_the_change(spec, i as u8).is_ok_and(|item| {
                skill_of(&item.options[usize::from(item.key)]).as_deref() == Some(skill)
            })
        }),
        params::DIAL_IT_IN => (from..MAX_PARAMS_SCAN).find(|&i| {
            params::dial_it_in(spec, i as u8).is_ok_and(|round| {
                round.constraints.editable.iter().any(|sel| match sel {
                    crate::v2::Selector::Param(addr) => skill_of(addr).as_deref() == Some(skill),
                    _ => false,
                })
            })
        }),
        _ => None,
    }
}

/// The answer `spec`'s game asks for at its level, after checking that this build has the game, the generator and the
/// level (`code_too_new` otherwise). The fields' own ranges are [`validate`]'s, which calls this.
pub fn answer_kind(spec: &EarSpec) -> Result<AnswerKind, EarError> {
    if params::is_params_game(spec.game) {
        params::answer_kind(spec)
    } else {
        music::answer_kind(spec)
    }
}

/// The music games' side of the registry: `enth-music`'s ear generator v1 (lane MUS), wired by lane F1E (the F1e
/// review's M5).
pub mod music {
    use enth_music::ear::{self as music, EarError as MusicError};

    use crate::model::AnswerKind;
    use crate::v3::{EarError, EarSpec};

    /// The answer a music game asks for at the spec's level: `enth-music`'s check of the game, the generator and the
    /// level (each unknown one `code_too_new`: they are append-only), then `Game::answer_kind(level)` (a choice, or taps
    /// for Tap back). Range errors are [`super::validate`]'s, which runs first.
    pub fn answer_kind(spec: &EarSpec) -> Result<AnswerKind, EarError> {
        let game = spec.validate_music().map_err(|e| refusal(spec, e))?;
        Ok(match game.answer_kind(spec.level) {
            music::AnswerKind::Choice => AnswerKind::Choice,
            music::AnswerKind::Taps => AnswerKind::Taps,
        })
    }

    /// `enth-music`'s refusal as this crate's.
    pub(crate) fn refusal(spec: &EarSpec, e: MusicError) -> EarError {
        match e {
            MusicError::UnknownGenerator(generator) => EarError::GeneratorTooNew {
                game: spec.game,
                generator,
                latest: music::LATEST_GENERATOR,
            },
            MusicError::LevelUnavailable { game, level, max } => EarError::LevelTooNew {
                game,
                level,
                top: max,
            },
            MusicError::SeedTooLarge(seed) => EarError::SeedOutOfRange { seed },
            MusicError::BadCount(count) => EarError::BadField {
                field: "count",
                value: u64::from(count),
            },
            // An unknown game, and any refusal a later enth-music adds: a newer build's.
            _ => EarError::GameTooNew { game: spec.game },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_families_split_at_game_4() {
        for game in 1..=3 {
            assert!(params::is_params_game(game));
        }
        for game in [0, 4, 5, 6, 7, 8, 10, 255] {
            assert!(!params::is_params_game(game), "{game}");
        }
        let music = EarSpec {
            game: 5,
            generator: 1,
            level: 1,
            seed: 1,
            count: 20,
        };
        assert_eq!(validate(&music), Ok(AnswerKind::Choice));
        // Every music game's level 1 is in this build; a level past its ladder, a newer generator and a newer game
        // are a newer build's.
        for game in enth_music::ear::Game::ALL {
            let spec = EarSpec::new(game, 1, 7, 10);
            let want = if game == enth_music::ear::Game::TapBack {
                AnswerKind::Taps
            } else {
                AnswerKind::Choice
            };
            assert_eq!(validate(&spec), Ok(want), "{game:?}");
            let top = game.max_level();
            for newer in [
                EarSpec {
                    level: top + 1,
                    ..spec
                },
                EarSpec {
                    generator: enth_music::ear::LATEST_GENERATOR + 1,
                    ..spec
                },
            ] {
                assert_eq!(
                    validate(&newer).unwrap_err().code(),
                    "code_too_new",
                    "{newer:?}"
                );
            }
        }
        assert_eq!(
            validate(&EarSpec { game: 11, ..music }).unwrap_err().code(),
            "code_too_new"
        );
    }

    /// Should-fix S3: one resolver finds a burst's item in either family, and a run from that item asks it first.
    #[test]
    fn find_item_resolves_a_focus_in_both_families() {
        // A music game: the found item's question exercises the skill, and nothing before it from `from` does.
        let spec = EarSpec::new(enth_music::ear::Game::IntervalStacks, 1, 4242, 1);
        let first = question::questions_at(&spec, 0).unwrap()[0]
            .skill
            .clone()
            .unwrap();
        let other = (1..200)
            .find_map(|i| {
                let q =
                    enth_music::ear::question(enth_music::ear::Game::IntervalStacks, 1, 4242, i)
                        .ok()?;
                q.skill().filter(|s| *s != first)
            })
            .unwrap();
        let at = find_item(&spec, &other, 0).unwrap();
        assert!(at > 0);
        let run = question::questions_at(&spec, at).unwrap();
        assert_eq!(run[0].skill.as_deref(), Some(other.as_str()));
        assert_eq!(run[0].index, 0);
        assert_eq!(find_item(&spec, &other, at), Some(at));
        assert_eq!(
            find_item(&spec, "ear.chord.min", 0),
            None,
            "a skill the game never asks"
        );
        // Spot the change: the moved control's skill.
        let spot = EarSpec {
            game: params::SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed: 77,
            count: 1,
        };
        let skill = question::questions_at(&spot, 9).unwrap()[0]
            .skill
            .clone()
            .unwrap();
        let at = find_item(&spot, &skill, 0).unwrap();
        assert!(at <= 9);
        assert_eq!(
            question::questions_at(&spot, at).unwrap()[0]
                .skill
                .as_deref(),
            Some(skill.as_str())
        );
        assert_eq!(find_item(&spot, &skill, MAX_PARAMS_SCAN), None);
        // Ear check: the set's one control, or nothing.
        let check = EarSpec {
            game: params::EAR_CHECK,
            ..spot
        };
        let control = question::questions_at(&check, 0).unwrap()[0]
            .skill
            .clone()
            .unwrap();
        assert_eq!(find_item(&check, &control, 3), Some(3));
        assert_eq!(find_item(&check, "ear.interval.m3", 3), None);
        // A spec this build cannot play finds nothing.
        assert_eq!(find_item(&EarSpec { game: 99, ..spot }, &skill, 0), None);
        // A parameter game's run stops at item 255.
        assert_eq!(
            question::questions_at(&EarSpec { count: 2, ..spot }, 255)
                .unwrap_err()
                .code(),
            "bad_answer"
        );
    }
}
