//! **Owner: lane CT** (F1r review S5, S12). Stub scorers, so the round can be wired, tested and played before lanes SS
//! and ST land the real ones: `sound-stub/1`, `structure-stub/1` and `combined-stub/1`.
//!
//! **Not a sound score.** Every stub works in parameter space, with no render: each randomized parameter's distance
//! from the target in just-noticeable steps ([`crate::priors::jnd`], generator v1's table), through the placeholder
//! module matching ([`super::matching`]). A parameter scores 1 within one step, falling linearly to 0 at four;
//! sound % and structure % are both the mean over the randomized parameters, and combined is w·sound + (1 − w)·
//! structure. The target itself scores 100 everywhere (P7). Their ids are their own, so a stub score is never
//! stored, logged or pinned as `sound/1`.

use enth_patch::{ParamAddr, Patch};

use super::matching::{self, ModuleMatching};
use super::{
    Closeness, Combined, Combiner, Component, ComponentKind, Direction, ModuleStatus, ModuleTerm,
    ParamScore, ScoreError, Scorer, ScorerId, SoundScore, StructureScore, TargetContext, Verdict,
};
use crate::model::ScoringSpec;
use crate::priors;
use crate::random;

/// The steps at which a parameter scores 0.
const ZERO_AT_STEPS: f64 = 4.0;

/// What both stubs keep from the target: the patches and which parameters to compare.
#[derive(Clone, Debug)]
pub struct StubPrepared {
    target: Patch,
    /// The randomized parameters, in address order.
    randomized: Vec<ParamAddr>,
    /// The target's observable parameters that were not randomized, in address order: a player's move there is a
    /// row of its own.
    others: Vec<ParamAddr>,
}

fn prepare(ctx: &TargetContext<'_>) -> StubPrepared {
    let mask = random::observable(ctx.target, ctx.phrase);
    let mut others: Vec<ParamAddr> = mask
        .observable_params()
        .filter(|a| !ctx.randomized.contains(a))
        .cloned()
        .collect();
    others.sort();
    StubPrepared {
        target: ctx.target.clone(),
        randomized: ctx.randomized.to_vec(),
        others,
    }
}

/// How far the answer's value is from the target's, in just-noticeable steps, and in which direction. `None` when
/// the answer has no matching parameter.
fn compare(
    prepared: &StubPrepared,
    matching: &ModuleMatching,
    addr: &ParamAddr,
    answer: &Patch,
) -> (Option<ParamAddr>, f64, Direction) {
    let (Some(spec), Some(t)) = (prepared.target.spec(addr), prepared.target.value(addr)) else {
        return (None, ZERO_AT_STEPS, Direction::Missing);
    };
    let Some(answer_addr) = matching.answer_addr(addr) else {
        return (None, ZERO_AT_STEPS, Direction::Missing);
    };
    let Some(a) = answer.value(&answer_addr) else {
        return (None, ZERO_AT_STEPS, Direction::Missing);
    };
    let steps = priors::jnd(spec).steps(t, a);
    let direction = if steps < 1.0 {
        Direction::Same
    } else if a > t {
        Direction::Higher
    } else {
        Direction::Lower
    };
    (Some(answer_addr), steps, direction)
}

/// 1 within one step, falling linearly to 0 at [`ZERO_AT_STEPS`].
fn score_of(steps: f64) -> f64 {
    if steps < 1.0 {
        1.0
    } else {
        ((ZERO_AT_STEPS - steps) / (ZERO_AT_STEPS - 1.0)).clamp(0.0, 1.0)
    }
}

fn verdict_of(steps: f64) -> Verdict {
    if steps < 1.0 {
        Verdict::Good
    } else if steps < 3.0 {
        Verdict::Near
    } else {
        Verdict::Bad
    }
}

/// The rows: every randomized parameter, then every other observable one the player moved by a step or more.
fn rows(prepared: &StubPrepared, answer: &Patch) -> (Vec<ParamScore>, Vec<f64>, ModuleMatching) {
    let matching = matching::match_modules(&prepared.target, answer);
    let mut out = Vec::new();
    let mut steps = Vec::new();
    for addr in &prepared.randomized {
        let (answer_addr, s, direction) = compare(prepared, &matching, addr, answer);
        steps.push(s);
        out.push(ParamScore {
            addr: addr.to_string(),
            answer_addr: answer_addr.map(|a| a.to_string()),
            score: score_of(s),
            verdict: verdict_of(s),
            direction,
            randomized: true,
        });
    }
    for addr in &prepared.others {
        let (answer_addr, s, direction) = compare(prepared, &matching, addr, answer);
        if answer_addr.is_some() && s >= 1.0 {
            out.push(ParamScore {
                addr: addr.to_string(),
                answer_addr: answer_addr.map(|a| a.to_string()),
                score: score_of(s),
                verdict: verdict_of(s),
                direction,
                randomized: false,
            });
        }
    }
    (out, steps, matching)
}

/// 100 × the mean score of the randomized rows (100 when nothing was randomized).
fn percent(rows: &[ParamScore]) -> f64 {
    let scores: Vec<f64> = rows
        .iter()
        .filter(|r| r.randomized)
        .map(|r| r.score)
        .collect();
    if scores.is_empty() {
        100.0
    } else {
        100.0 * scores.iter().sum::<f64>() / scores.len() as f64
    }
}

/// The descriptor row a parameter feeds, by its module kind and key (the stub's stand-in for SS's descriptors).
fn kind_of(addr: &str) -> Option<ComponentKind> {
    let (module, key) = addr.split_once('.')?;
    let kind = module.trim_end_matches(|c: char| c.is_ascii_digit());
    Some(match (kind, key) {
        ("osc", "semi" | "fine") => ComponentKind::Pitch,
        ("osc", "wave" | "pw" | "position" | "table") => ComponentKind::Harmonics,
        ("filter", _) => ComponentKind::Brightness,
        ("env", "attack") => ComponentKind::Attack,
        ("env", "decay") => ComponentKind::Envelope,
        ("env", "sustain") => ComponentKind::Sustain,
        ("env", "release") => ComponentKind::Release,
        ("noise", _) => ComponentKind::Noisiness,
        ("lfo", _) => ComponentKind::Movement,
        _ => return None,
    })
}

/// The sound stub (`sound-stub/1`): see the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundStub;

impl Scorer for SoundStub {
    type Prepared = StubPrepared;
    type Score = SoundScore;

    fn id(&self) -> ScorerId {
        ScorerId::SOUND_STUB
    }

    fn prepare(&self, target: &TargetContext<'_>) -> Result<StubPrepared, ScoreError> {
        Ok(prepare(target))
    }

    fn score(&self, prepared: &StubPrepared, answer: &Patch) -> Result<SoundScore, ScoreError> {
        let (rows, steps, _) = rows(prepared, answer);
        let capped: Vec<f64> = steps.iter().map(|s| s.min(ZERO_AT_STEPS)).collect();
        let distance = if capped.is_empty() {
            0.0
        } else {
            capped.iter().sum::<f64>() / capped.len() as f64
        };
        // One row per kind, from its farthest randomized parameter.
        let mut components = Vec::new();
        for kind in ComponentKind::ALL {
            let worst = rows
                .iter()
                .zip(&steps)
                .filter(|(r, _)| r.randomized && kind_of(&r.addr) == Some(kind))
                .max_by(|a, b| a.1.total_cmp(b.1));
            if let Some((row, s)) = worst {
                components.push(Component {
                    kind,
                    direction: row.direction,
                    closeness: if *s < 1.0 {
                        Closeness::Close
                    } else if *s < 3.0 {
                        Closeness::ABit
                    } else {
                        Closeness::Far
                    },
                    target: None,
                    answer: None,
                });
            }
        }
        Ok(SoundScore {
            scorer: self.id(),
            percent: percent(&rows),
            distance,
            floor: 0.0,
            scale: 1.0,
            jnd: distance,
            components,
        })
    }
}

/// The structure stub (`structure-stub/1`): see the [module docs](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureStub;

impl Scorer for StructureStub {
    type Prepared = StubPrepared;
    type Score = StructureScore;

    fn id(&self) -> ScorerId {
        ScorerId::STRUCTURE_STUB
    }

    fn prepare(&self, target: &TargetContext<'_>) -> Result<StubPrepared, ScoreError> {
        Ok(prepare(target))
    }

    fn score(&self, prepared: &StubPrepared, answer: &Patch) -> Result<StructureScore, ScoreError> {
        let (params, _, matching) = rows(prepared, answer);
        let kind_of = |patch: &Patch, id: &enth_patch::ModuleId| {
            patch
                .find(id)
                .map_or_else(String::new, |(_, m)| m.kind.key().to_owned())
        };
        let mut modules: Vec<ModuleTerm> = matching
            .missing()
            .iter()
            .map(|id| ModuleTerm {
                id: id.to_string(),
                kind: kind_of(&prepared.target, id),
                status: ModuleStatus::Missing,
            })
            .collect();
        modules.extend(matching.extra().iter().map(|id| ModuleTerm {
            id: id.to_string(),
            kind: kind_of(answer, id),
            status: ModuleStatus::Extra,
        }));
        Ok(StructureScore {
            scorer: self.id(),
            percent: percent(&params),
            params,
            modules,
        })
    }
}

/// The combined stub (`combined-stub/1`): w·sound + (1 − w)·structure, w the challenge's sound weight.
#[derive(Clone, Copy, Debug, Default)]
pub struct CombinedStub;

impl Combiner for CombinedStub {
    fn id(&self) -> ScorerId {
        ScorerId::COMBINED_STUB
    }

    fn combine(
        &self,
        scoring: &ScoringSpec,
        sound: &SoundScore,
        structure: &StructureScore,
    ) -> Combined {
        let w = f64::from(scoring.sound_weight.min(100)) / 100.0;
        Combined {
            scorer: self.id(),
            percent: (w * sound.percent + (1.0 - w) * structure.percent).clamp(0.0, 100.0),
            sound_weight: scoring.sound_weight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_fall_from_one_step_to_four() {
        assert_eq!(score_of(0.0), 1.0);
        assert_eq!(score_of(0.99), 1.0);
        assert_eq!(score_of(1.0), 1.0);
        assert_eq!(score_of(2.5), 0.5);
        assert_eq!(score_of(4.0), 0.0);
        assert_eq!(score_of(f64::INFINITY), 0.0);
        assert_eq!(
            [verdict_of(0.5), verdict_of(2.0), verdict_of(3.0)],
            [Verdict::Good, Verdict::Near, Verdict::Bad]
        );
    }

    #[test]
    fn parameters_feed_their_component() {
        assert_eq!(kind_of("osc1.semi"), Some(ComponentKind::Pitch));
        assert_eq!(kind_of("osc2.wave"), Some(ComponentKind::Harmonics));
        assert_eq!(kind_of("filter1.cutoff"), Some(ComponentKind::Brightness));
        assert_eq!(kind_of("env2.sustain"), Some(ComponentKind::Sustain));
        assert_eq!(kind_of("lfo1.rate"), Some(ComponentKind::Movement));
        assert_eq!(kind_of("voice.mode"), None);
        assert_eq!(kind_of("reverb1.wet"), None);
    }
}
