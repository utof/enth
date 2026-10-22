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
//! **The seam with `enth-music`, until MUS's generator is merged.** This build has no music game yet, so games 4 and
//! up read as `code_too_new` ("update enth"). Wiring them is mechanical, in this file and [`question`] only: [`music`]
//! maps `enth_music::ear::EarSpec::validate` and `Game::answer_kind(level)` here, and [`question::prepare`] turns each
//! `enth_music::ear::Question` into a [`question::Stimulus`] (timbre, notes, length, band) and a
//! [`scorers::EarKey`], with its labels and skill. What MUS provides, agreed with lane MUS: the fields of
//! [`EarSpec`] field for field; timbre ids into `timbres-v1` (0 sine, 1 saw low-pass, 2 pluck, 3 organ table, 4 pad,
//! 5 noise-burst click, 6 sustained pink noise); notes as (key 24 to 96, velocity 1 to 127, start and hold in ms);
//! `length_ms` with the release tail; the band as [`question::EqBand`]'s integers; the key as an option index (with the
//! option count), or onsets and a window for taps.

pub mod code;
pub mod ladder;
pub mod params;
pub mod question;
pub mod scorers;

use crate::model::AnswerKind;
use crate::v3::{EarError, EarSpec};

/// The answer `spec`'s game asks for at its level, after checking that this build has the game, the generator and the
/// level (`code_too_new` otherwise). The fields' own ranges are [`EarSpec::validate`]'s, which calls this.
pub fn answer_kind(spec: &EarSpec) -> Result<AnswerKind, EarError> {
    if params::is_params_game(spec.game) {
        params::answer_kind(spec)
    } else {
        music::answer_kind(spec)
    }
}

/// The music games' side of the registry: `enth-music`'s ear generator (lane MUS).
pub mod music {
    use crate::model::AnswerKind;
    use crate::v3::{EarError, EarSpec};

    /// The answer a music game asks for. **No music game is merged in this build** (lane MUS builds ear generator
    /// v1 in parallel), so every game from 4 up is `code_too_new` here; F1E maps `enth_music::ear::EarSpec::validate`
    /// (an unknown game, generator or level is `code_too_new`) and `Game::answer_kind(level)` (choice or taps; a value
    /// with G3's slider level) into this function.
    pub fn answer_kind(spec: &EarSpec) -> Result<AnswerKind, EarError> {
        Err(EarError::GameTooNew { game: spec.game })
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
        assert_eq!(music.validate().unwrap_err().code(), "code_too_new");
    }
}
