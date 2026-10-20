//! **Owner: lane CT**, which finalises it with lane RG at freeze point F1r. Pre-declared by lane S0r from design.md §9,
//! as the R1/S1 research summary amends it (decision "`RandomSpec` and codes"; report 01 §2.5).
//!
//! **A stub.** The fields are fixed early so that RG can write the generator against them while CT writes Challenge
//! v1 and the codes (both in wave 2). CT and RG settle the names, the ranges and the version at F1r, where a judge
//! reviews them with the other persisted types. Until then nothing persists a `RandomSpec`, so it has no version or
//! fixture of its own yet.
//!
//! Every field is an integer: a code, a JSON file and the UI must reproduce a spec exactly, and a text → float round
//! trip can flip a draw (report 01). design.md §9's `f32` knobs become percentages, and the generator version joins
//! the spec, so a code names the generator that made its target.

use serde::{Deserialize, Serialize};

/// What a challenge randomizes (design.md §9), in difficulty order: level 1 randomizes the oscillators, and each
/// level adds the next category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Waveform, tuning, level.
    Oscillators,
    /// Filter type, cutoff, resonance.
    Filter,
    /// The amp and filter envelopes.
    Envelopes,
    /// LFOs and modulation routes.
    Modulation,
    /// Effects and their settings.
    Effects,
    /// Which modules exist and how they are routed (enable flags and routes over the starter superset in R1).
    Structure,
}

impl Category {
    /// Every category, in difficulty order. [`RandomSpec::amounts`] is indexed in this order.
    pub const ALL: [Category; 6] = [
        Category::Oscillators,
        Category::Filter,
        Category::Envelopes,
        Category::Modulation,
        Category::Effects,
        Category::Structure,
    ];

    /// How many categories there are.
    pub const COUNT: usize = Category::ALL.len();

    /// This category's position in [`Category::ALL`] and in [`RandomSpec::amounts`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The randomizer's settings: a seed and a few integers that give the same target on every machine (a stub until
/// F1r; see the [module docs](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RandomSpec {
    /// The generator version. Old versions stay frozen, with hash goldens, so an old code keeps its target; a code
    /// that names a newer generator than the build knows is refused (`code_too_new`).
    pub generator: u16,
    /// The seed every draw is keyed from: `derive(derive(seed, fnv1a64(address)), attempt)`.
    pub seed: u64,
    /// The one knob, 0-100 (design.md §9's 0..=1 as a percentage): it picks the level, and so which categories are
    /// randomized. RG maps it to the five levels.
    pub difficulty: u8,
    /// An amount per category, 0-100, indexed by [`Category::index`]. `None` follows `difficulty`; a set amount
    /// overrides how many parameters the category draws, and how wide.
    pub amounts: [Option<u8>; Category::COUNT],
    /// 0-100: 0 draws uniformly, 100 draws from the musical priors only; in between, each draw comes from the prior
    /// with this probability. The same knob later samples ML datasets.
    pub musical: u8,
    /// The archetype: a start patch and a prior set (pluck, pad, bass, lead, keys). 0 means none (the starter
    /// superset); RG numbers the rest.
    pub template: u8,
}

#[cfg(test)]
mod tests {
    use super::{Category, RandomSpec};

    fn spec() -> RandomSpec {
        RandomSpec {
            generator: 1,
            seed: u64::MAX,
            difficulty: 40,
            amounts: [None, Some(100), None, None, Some(0), None],
            musical: 100,
            template: 0,
        }
    }

    #[test]
    fn categories_are_in_difficulty_order() {
        for (i, c) in Category::ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
        }
        assert_eq!(Category::COUNT, 6);
    }

    /// Every number in the JSON form, nested or not.
    fn numbers(v: &serde_json::Value, out: &mut Vec<serde_json::Number>) {
        match v {
            serde_json::Value::Number(n) => out.push(n.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|i| numbers(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| numbers(i, out)),
            _ => {}
        }
    }

    /// Integers only: every number in the JSON form is an unsigned integer, and a `u64::MAX` seed survives.
    #[test]
    fn a_spec_round_trips_through_json_as_integers() {
        let json = serde_json::to_string(&spec()).unwrap();
        let mut found = Vec::new();
        numbers(&serde_json::from_str(&json).unwrap(), &mut found);
        assert_eq!(found.len(), 7, "{json}");
        assert!(found.iter().all(serde_json::Number::is_u64), "{json}");
        let back: RandomSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec());
    }
}
