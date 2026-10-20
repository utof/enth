//! **Owner: lane ST** (structure, combined and time). The combined score, `combined/1`, and the time rule, `time/1`
//! (report 02 §2.6; design.md §6 and §9). The reference is docs/challenge.md, "Structure, combined and time".
//!
//! **`combined/1`** is one headline number: `(w·sound + (100 − w)·structure) / 100`, with `w` the challenge's
//! [`ScoringSpec::sound_weight`] in percent. The weight lives in the challenge, so a later default never moves an
//! old score; the Recreate default ([`ScoringSpec::recreate`], lane CT) is `w = 50 + 30·difficulty/100` (integer
//! division): 50 % at difficulty 0, where structure is unambiguous and ADR-0007 asks to lean on it, 80 % at 100,
//! where many routes are valid. Written with the integer weight first, so `65·90 + 35·80 = 8650` is exact.
//! **It never decides pass:** [`ScoreReport::new`](super::ScoreReport::new) decides pass from sound alone, whatever
//! combined says.
//!
//! **`time/1`** is the first passing **submit** (Syntorial-style: a live pass light would give the answer away), in
//! the editor-visible milliseconds the shell's clock gave the session (core crates read no clock). The session (lane
//! SG) records each submit's time and keeps the rule; [`time_to_pass`] is the rule as a pure function, for the
//! session, the CLI and the round log's reader, and [`first_passing`] replays any timed log ("you were at a pass at
//! 1:42").
//!
//! **The dispatch hook** ([`combiner`], F1r review S5): `score::round` asks this file for the combiner a challenge
//! names, after the stub (`combined-stub/1`). `time/1` is checked by `score::round` itself.

use super::{Combined, Combiner, ScorerId, SoundScore, StructureScore};
use crate::model::ScoringSpec;

/// The combiner `id`; `None` when this file has none by that id (the caller refuses it, `unknown_scorer`).
pub(crate) fn combiner(id: &ScorerId) -> Option<Box<dyn Combiner>> {
    (*id == ScorerId::COMBINED_V1).then(|| Box::new(CombinedV1) as Box<dyn Combiner>)
}

/// `combined/1`: see the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct CombinedV1;

impl CombinedV1 {
    /// The combined % of a sound % and a structure % at sound weight `w` percent (above 100 reads as 100). A
    /// non-finite input stays non-finite, so a broken score is refused downstream rather than hidden.
    pub fn percent(sound_weight: u8, sound: f64, structure: f64) -> f64 {
        let w = f64::from(sound_weight.min(100));
        ((w * sound + (100.0 - w) * structure) / 100.0).clamp(0.0, 100.0)
    }
}

impl Combiner for CombinedV1 {
    fn id(&self) -> ScorerId {
        ScorerId::COMBINED_V1
    }

    fn combine(
        &self,
        scoring: &ScoringSpec,
        sound: &SoundScore,
        structure: &StructureScore,
    ) -> Combined {
        Combined {
            scorer: self.id(),
            percent: CombinedV1::percent(scoring.sound_weight, sound.percent, structure.percent),
            sound_weight: scoring.sound_weight.min(100),
        }
    }
}

/// One submit as the session recorded it: when (editor-visible ms since the round started, from the shell's clock)
/// and whether it passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Submit {
    /// Editor-visible milliseconds since the round started.
    pub at_ms: u64,
    /// Whether the submit passed (sound % at or above the pass mark).
    pub passed: bool,
}

/// `time/1`: the time of the first passing submit, in submit order; `None` without a pass. Later submits never
/// change it, passing or not.
pub fn time_to_pass(submits: impl IntoIterator<Item = Submit>) -> Option<u64> {
    first_passing(submits.into_iter().map(|s| (s.at_ms, s)), |s| s.passed)
}

/// The time of the first entry of a timed log that `passes`, in log order: [`time_to_pass`] over submits, or a
/// replay of the edit log through the scorer ("you were at a pass at 1:42").
pub fn first_passing<T>(
    log: impl IntoIterator<Item = (u64, T)>,
    mut passes: impl FnMut(&T) -> bool,
) -> Option<u64> {
    log.into_iter().find(|(_, x)| passes(x)).map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::ScoreReport;

    fn sound(percent: f64) -> SoundScore {
        SoundScore {
            scorer: ScorerId::SOUND_V1,
            percent,
            distance: 0.0,
            floor: 0.0,
            scale: 1.0,
            jnd: 0.0,
            components: Vec::new(),
        }
    }

    fn structure(percent: f64) -> StructureScore {
        StructureScore {
            scorer: ScorerId::STRUCTURE_V1,
            percent,
            params: Vec::new(),
            modules: Vec::new(),
        }
    }

    fn spec(difficulty: u8) -> ScoringSpec {
        ScoringSpec::with_scorers(
            difficulty,
            ScorerId::SOUND_V1,
            ScorerId::STRUCTURE_V1,
            ScorerId::COMBINED_V1,
            ScorerId::TIME_V1,
        )
    }

    #[test]
    fn combined_weighs_sound_from_half_to_four_fifths_by_difficulty() {
        let c = |d: u8, s: f64, t: f64| CombinedV1.combine(&spec(d), &sound(s), &structure(t));
        assert_eq!(c(0, 90.0, 70.0).percent, 80.0);
        assert_eq!(c(0, 90.0, 70.0).sound_weight, 50);
        assert_eq!(
            c(50, 90.0, 80.0).percent,
            86.5,
            "65·90 + 35·80 = 8650, exactly"
        );
        assert_eq!(c(100, 100.0, 0.0).percent, 80.0);
        assert_eq!(c(25, 100.0, 100.0).percent, 100.0);
        assert_eq!(c(75, 0.0, 0.0).percent, 0.0);
        assert_eq!(
            CombinedV1::percent(200, 40.0, 0.0),
            40.0,
            "a weight above 100 reads as 100"
        );
        assert!(CombinedV1::percent(50, f64::NAN, 10.0).is_nan());
        assert_eq!(
            combiner(&ScorerId::COMBINED_V1).unwrap().id(),
            ScorerId::COMBINED_V1
        );
        assert!(combiner(&ScorerId::COMBINED_STUB).is_none());
    }

    #[test]
    fn combined_never_decides_pass() {
        let s = spec(0);
        // A perfect patch that sounds off does not pass; a different patch that sounds right does.
        let off = ScoreReport::new(
            &s,
            sound(89.0),
            structure(100.0),
            CombinedV1.combine(&s, &sound(89.0), &structure(100.0)),
        );
        assert!(off.combined.percent > 90.0 && !off.passed);
        let right = ScoreReport::new(
            &s,
            sound(90.0),
            structure(0.0),
            CombinedV1.combine(&s, &sound(90.0), &structure(0.0)),
        );
        assert!(right.combined.percent < 90.0 && right.passed);
        assert_eq!(right.check_against(&s), Ok(()));
    }

    #[test]
    fn time_is_the_first_passing_submit() {
        let at = |at_ms, passed| Submit { at_ms, passed };
        assert_eq!(time_to_pass([]), None);
        assert_eq!(time_to_pass([at(3_000, false), at(6_000, false)]), None);
        assert_eq!(
            time_to_pass([at(3_000, false), at(61_500, true), at(70_000, true)]),
            Some(61_500)
        );
        assert_eq!(time_to_pass([at(10, true), at(20, false)]), Some(10));
        // A replay of an edit log through any pass test.
        let log = [
            (1_000u64, 40.0),
            (2_000, 89.9),
            (102_000, 90.0),
            (110_000, 95.0),
        ];
        assert_eq!(first_passing(log, |s| *s >= 90.0), Some(102_000));
    }
}
