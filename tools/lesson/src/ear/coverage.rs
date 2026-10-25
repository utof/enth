//! **Owner: lane ECK.** GR4's coverage bullet (docs/ear.md): **every observable Basic-tier P1 parameter is in game 1 or
//! 2**, and **bursts move Review boxes** (as REV's properties).
//!
//! **Parameters.** The parameter games draw from a pool of controls per level (`ear::params::level(game, level).pool`);
//! the union over the levels of Spot the change and Dial it in must hold every Basic-tier parameter that is observable
//! through the games' held phrase on the games' base patch (template 0, plus the built-ins `voice` and `master`). A
//! control that cannot be heard on the base (lane RG's mask) cannot be asked, and is not required. G4's ladder grows the
//! pools; until then this lists what is missing.
//!
//! **Bursts.** A burst is five items on one skill (`enth_learn::review::BURST_ITEMS`), found by
//! `enth_challenge::ear::find_item` from a seed. For each skill a (game, level) asks in the sample: the five items are
//! found, each asks the skill (the question's own `skill()`), and a Good burst moves the skill's box up and a Bad one
//! down, by `Review::observe_burst`, which is the function the drill's summary uses.

use enth_challenge::ear::{self, EarSpec, params, question};
use enth_challenge::random;
use enth_learn::review::{BURST_ITEMS, Burst, Review, Skill, burst_verdict, moved};
use enth_params::Tier;
use enth_patch::{Builtin, ParamAddr, Target};

/// The Basic-tier parameters observable on the games' base, which of them the pools hold, and which are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Basic {
    /// Every Basic-tier parameter heard through the held phrase on the base.
    pub observable: Vec<String>,
    /// Those in a pool of game 1 or 2 at some level.
    pub covered: Vec<String>,
    /// Those in no pool.
    pub missing: Vec<String>,
}

/// Compute [`Basic`].
///
/// # Errors
/// The base or the pools do not parse.
pub fn basic_tier() -> Result<Basic, String> {
    let base = params::base();
    let phrase = params::phrase();
    let mask = random::observable(&base, phrase);
    let mut observable: Vec<ParamAddr> = Vec::new();
    for (_, inst) in base.modules() {
        for spec in inst.kind.params() {
            if spec.learn.tier == Tier::Basic {
                let addr: ParamAddr = format!("{}.{}", inst.id, spec.key)
                    .parse()
                    .map_err(|e| format!("{}.{}: {e}", inst.id, spec.key))?;
                if mask.is_observable(&addr) {
                    observable.push(addr);
                }
            }
        }
    }
    for builtin in [Builtin::Voice, Builtin::Master] {
        let target = if builtin == Builtin::Voice {
            Target::Voice
        } else {
            Target::Master
        };
        for spec in builtin.params() {
            if spec.learn.tier == Tier::Basic
                && let Ok(addr) = ParamAddr::new(target.clone(), spec.key)
                && mask.is_observable(&addr)
            {
                observable.push(addr);
            }
        }
    }
    let mut pooled: Vec<ParamAddr> = Vec::new();
    for game in [params::SPOT_THE_CHANGE, params::DIAL_IT_IN] {
        for level in 1..=params::top_level(game) {
            if let Some(l) = params::level(game, level) {
                for text in l.pool {
                    pooled.push(text.parse().map_err(|e| format!("{text}: {e}"))?);
                }
            }
        }
    }
    let name = |a: &ParamAddr| a.to_string();
    let (covered, missing): (Vec<_>, Vec<_>) = observable.iter().partition(|a| pooled.contains(a));
    Ok(Basic {
        observable: observable.iter().map(name).collect(),
        covered: covered.into_iter().map(name).collect(),
        missing: missing.into_iter().map(name).collect(),
    })
}

/// The problems with bursts on `skills` (the Review skills a game's questions at a level exercise, each with seeds a
/// question of it was drawn from), for the game and level (empty: all good).
#[must_use]
pub fn bursts(game: u8, level: u8, skills: &[(String, Vec<u64>)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (text, seeds) in skills.iter().map(|(t, s)| (t.as_str(), s)) {
        let Some(skill) = Skill::parse(text) else {
            problems.push(format!("the skill {text:?} is not a Review skill key"));
            continue;
        };
        for &seed in seeds {
            let spec = EarSpec {
                game,
                generator: 1,
                level,
                seed,
                count: 1,
            };
            let mut from = 0_u32;
            for burst_item in 0..BURST_ITEMS {
                let Some(item) = ear::find_item(&spec, text, from) else {
                    problems.push(format!(
                        "{text}: no item {burst_item} of a burst from seed {seed}"
                    ));
                    break;
                };
                match question::questions_at(&spec, item) {
                    Ok(qs) if qs.first().and_then(|q| q.skill.as_deref()) == Some(text) => {}
                    Ok(_) => problems.push(format!(
                        "{text}: find_item gave item {item}, which asks another skill"
                    )),
                    Err(e) => problems.push(format!("{text}: item {item}: {e}")),
                }
                from = item + 1;
            }
        }
        // A burst moves its box the way Review says: a Good one up, a Bad one down (never below 1), a Near one holds.
        for (right, start) in [
            (BURST_ITEMS, 1_u8),
            (BURST_ITEMS, 3),
            (0, 3),
            (0, 1),
            (3, 2),
        ] {
            let mut review = Review::default();
            if start > 1 {
                for _ in 1..start {
                    review.observe_burst(&Burst::new(skill.clone(), BURST_ITEMS), 1);
                }
            }
            let before = review.get(&skill).level;
            review.observe_burst(&Burst::new(skill.clone(), right), 2);
            let after = review.get(&skill).level;
            let want = moved(before, burst_verdict(right), true);
            if after != want || (right == BURST_ITEMS && before < 5 && after <= before) {
                problems.push(format!(
                    "{text}: a burst of {right} right moved box {before} to {after}, not {want}"
                ));
            }
        }
    }
    problems
}
