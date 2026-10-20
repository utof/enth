//! **Owner: lane CT**, finalised with lane RG at freeze point F1r. Pre-declared by lane S0r from design.md §9, as the
//! R1/S1 research summary amends it (decision "`RandomSpec` and codes"; report 01 §2.5). The reference is
//! docs/challenge.md, "`RandomSpec`".
//!
//! [`RandomSpec`] is the randomizer's settings: a generator version, a seed and a few percentages. It is what a
//! challenge code carries and what a Recreate [`Challenge`](crate::model::Challenge) targets, so a code, a JSON file
//! and the settings page must reproduce it exactly. **Every field is an integer**: a float that goes through text and
//! back can flip a draw (report 01), so design.md §9's first-draft `f32` knobs are percentages here.
//!
//! The rules the generator (lane RG, [`crate::random`]) follows with it:
//!
//! - **Validate first.** A spec built in code can hold anything; [`RandomSpec::validate`] is the one check (ranges,
//!   and a generator this build runs), and the generator refuses a spec that fails it rather than clamping.
//! - **`generator` picks frozen code.** Versions `1..=`[`LATEST_GENERATOR`] exist in this build, and a version's
//!   output never changes (docs/challenge.md, "Generator versions are frozen"). Freezing a new version bumps
//!   [`LATEST_GENERATOR`] in the same commit that adds its spec → target-hash golden.
//! - **`difficulty` picks the level.** Five levels, each one category more ([`Category::ALL`] order). The presets
//!   below put levels 1–5 at difficulty 0, 25, 50, 75 and 100, so any banding that keeps those five values in
//!   different levels, in order, works; RG's proposed bands are 0–20, 21–40, 41–60, 61–80 and 81–100.
//! - **Short codes need short seeds.** A short code holds a 40-bit seed ([`crate::code::SHORT_SEED_BITS`]), so a
//!   shell that draws a fresh seed for a round keeps 40 bits of it; any `u64` still works through the long code.
//!
//! [`PRESETS`] names the settings a short code can carry. Preset ids are append-only and a preset's settings never
//! change, because an old short code must keep naming the same spec.

use serde::{Deserialize, Serialize};

/// The largest value of a percentage field (`difficulty`, `amounts`, `musical`).
pub const PERCENT_MAX: u8 = 100;

/// The newest generator version this build runs. Lane RG's [`crate::random`] implements every version from 1 to this
/// one, each frozen; a spec or a code that names a newer one is refused (`code_too_new`), because this build cannot
/// regenerate its target.
pub const LATEST_GENERATOR: u16 = 1;

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

/// The randomizer's settings: a seed and a few integers that give the same target on every machine. See the
/// [module docs](self) for the rules the generator follows with it.
///
/// The JSON form is the fields in this order, every number an unsigned integer:
/// `{"generator":1,"seed":42,"difficulty":25,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}`.
/// It has no version of its own: it is persisted only inside a versioned [`Challenge`](crate::model::Challenge) or
/// a [`ChallengeCode`](crate::code::ChallengeCode), and a change to it is a version bump of both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RandomSpec {
    /// The generator version, `1..=`[`LATEST_GENERATOR`]. Old versions stay frozen, with hash goldens, so an old code
    /// keeps its target; a newer version than the build knows is refused (`code_too_new`).
    pub generator: u16,
    /// The seed every draw is keyed from: `derive(derive(seed, fnv1a64(address)), attempt)`. Any `u64`; a short code
    /// holds only the low 40 bits ([`crate::code::SHORT_SEED_BITS`]).
    pub seed: u64,
    /// The one knob, 0–100 (design.md §9's 0..=1 as a percentage): it picks the level, and so which categories are
    /// randomized. RG maps it to the five levels (see the [module docs](self)).
    pub difficulty: u8,
    /// An amount per category, 0–100, indexed by [`Category::index`]. `None` follows `difficulty`; a set amount
    /// overrides how many parameters the category draws, and how wide.
    pub amounts: [Option<u8>; Category::COUNT],
    /// 0–100: 0 draws uniformly, 100 draws from the musical priors only; in between, each draw comes from the prior
    /// with this probability. The same knob later samples ML datasets.
    pub musical: u8,
    /// The archetype: a start patch and a prior set (pluck, pad, bass, lead, keys). 0 means none (the starter
    /// superset); RG numbers the rest, append-only.
    pub template: u8,
}

impl RandomSpec {
    /// The amount set for `category`, or `None` when it follows `difficulty`.
    pub const fn amount(&self, category: Category) -> Option<u8> {
        self.amounts[category.index()]
    }

    /// Check the ranges and the generator version: every percentage at most [`PERCENT_MAX`], and a generator from 1
    /// to [`LATEST_GENERATOR`]. The generator calls this before it draws anything.
    pub fn validate(&self) -> Result<(), SpecError> {
        self.validate_up_to(LATEST_GENERATOR)
    }

    /// [`validate`](Self::validate) against another newest generator, so the code tests can cover every `u16`.
    pub(crate) fn validate_up_to(&self, latest: u16) -> Result<(), SpecError> {
        if self.generator == 0 {
            return Err(SpecError::NoGenerator);
        }
        if self.generator > latest {
            return Err(SpecError::GeneratorTooNew {
                generator: self.generator,
                latest,
            });
        }
        let percent = |field: &'static str, value: u8| {
            if value > PERCENT_MAX {
                Err(SpecError::OutOfRange { field, value })
            } else {
                Ok(())
            }
        };
        percent("difficulty", self.difficulty)?;
        percent("musical", self.musical)?;
        for (c, a) in Category::ALL.iter().zip(self.amounts) {
            if let Some(a) = a {
                percent(AMOUNT_FIELDS[c.index()], a)?;
            }
        }
        Ok(())
    }

    /// The preset whose settings this spec has (difficulty, amounts, musical and template all equal), if any. Only
    /// such a spec can travel in a short code.
    pub fn preset(&self) -> Option<&'static SpecPreset> {
        PRESETS.iter().find(|p| p.matches(self))
    }
}

/// The field names [`SpecError::OutOfRange`] reports for each amount, in [`Category::ALL`] order.
const AMOUNT_FIELDS: [&str; Category::COUNT] = [
    "amounts.oscillators",
    "amounts.filter",
    "amounts.envelopes",
    "amounts.modulation",
    "amounts.effects",
    "amounts.structure",
];

/// Why a [`RandomSpec`] is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpecError {
    /// `generator` is 0, which names no generator.
    NoGenerator,
    /// `generator` is newer than this build runs.
    GeneratorTooNew {
        /// The version the spec names.
        generator: u16,
        /// The newest this build runs ([`LATEST_GENERATOR`]).
        latest: u16,
    },
    /// A percentage is above [`PERCENT_MAX`].
    OutOfRange {
        /// The field (`difficulty`, `musical`, `amounts.filter`, …).
        field: &'static str,
        /// Its value.
        value: u8,
    },
}

impl SpecError {
    /// A stable snake-case code. A newer generator shares `code_too_new` with a newer code version: the remedy is the
    /// same.
    pub fn code(&self) -> &'static str {
        match self {
            SpecError::NoGenerator => "no_generator",
            SpecError::GeneratorTooNew { .. } => "code_too_new",
            SpecError::OutOfRange { .. } => "out_of_range",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            SpecError::NoGenerator => "generator versions start at 1".to_owned(),
            SpecError::GeneratorTooNew { generator, latest } => format!(
                "made with generator {generator}, and this enth knows up to {latest}: update enth to play it"
            ),
            SpecError::OutOfRange { field, .. } => {
                format!("{field} is a percentage, 0 to {PERCENT_MAX}")
            }
        }
    }
}

impl core::fmt::Display for SpecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpecError::NoGenerator => f.write_str("the spec names generator 0"),
            SpecError::GeneratorTooNew { generator, latest } => write!(
                f,
                "the spec names generator {generator}, newer than this build's {latest}"
            ),
            SpecError::OutOfRange { field, value } => {
                write!(f, "{field} is {value}, above {PERCENT_MAX}")
            }
        }
    }
}

impl std::error::Error for SpecError {}

/// Named settings a short code can carry by id: everything in a [`RandomSpec`] except the generator and the seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpecPreset {
    /// The id a short code carries, 1–255. Append-only: an id is never reused or redefined.
    pub id: u8,
    /// What the settings page calls it.
    pub name: &'static str,
    /// [`RandomSpec::difficulty`].
    pub difficulty: u8,
    /// [`RandomSpec::amounts`].
    pub amounts: [Option<u8>; Category::COUNT],
    /// [`RandomSpec::musical`].
    pub musical: u8,
    /// [`RandomSpec::template`].
    pub template: u8,
}

impl SpecPreset {
    /// The preset with this id, if this build has it.
    pub fn by_id(id: u8) -> Option<&'static SpecPreset> {
        PRESETS.iter().find(|p| p.id == id)
    }

    /// The spec these settings give with `generator` and `seed`.
    pub const fn spec(&self, generator: u16, seed: u64) -> RandomSpec {
        RandomSpec {
            generator,
            seed,
            difficulty: self.difficulty,
            amounts: self.amounts,
            musical: self.musical,
            template: self.template,
        }
    }

    /// Whether `spec` has exactly these settings.
    pub fn matches(&self, spec: &RandomSpec) -> bool {
        spec.difficulty == self.difficulty
            && spec.amounts == self.amounts
            && spec.musical == self.musical
            && spec.template == self.template
    }
}

/// The presets of code v1, frozen: the five levels with every amount following the difficulty, musical priors only and
/// the starter superset as the start. New presets (archetypes, a "chaos" level) are appended with new ids.
pub const PRESETS: [SpecPreset; 5] = [
    level_preset(1, "Level 1", 0),
    level_preset(2, "Level 2", 25),
    level_preset(3, "Level 3", 50),
    level_preset(4, "Level 4", 75),
    level_preset(5, "Level 5", 100),
];

const fn level_preset(id: u8, name: &'static str, difficulty: u8) -> SpecPreset {
    SpecPreset {
        id,
        name,
        difficulty,
        amounts: [None; Category::COUNT],
        musical: PERCENT_MAX,
        template: 0,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        Category, LATEST_GENERATOR, PERCENT_MAX, PRESETS, RandomSpec, SpecError, SpecPreset,
    };

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

    proptest! {
        /// The spec is integer-only: for any valid spec, every number in the JSON form is an unsigned integer, and
        /// the text reads back to the same spec.
        #[test]
        fn any_spec_is_integers_only_and_round_trips(
            seed in any::<u64>(),
            difficulty in 0..=PERCENT_MAX,
            amounts in proptest::array::uniform6(proptest::option::of(0..=PERCENT_MAX)),
            musical in 0..=PERCENT_MAX,
            template in any::<u8>(),
        ) {
            let s = RandomSpec { generator: LATEST_GENERATOR, seed, difficulty, amounts, musical, template };
            prop_assert_eq!(s.validate(), Ok(()));
            let json = serde_json::to_string(&s).unwrap();
            let mut found = Vec::new();
            numbers(&serde_json::from_str(&json).unwrap(), &mut found);
            prop_assert!(found.iter().all(serde_json::Number::is_u64), "{}", json);
            prop_assert_eq!(serde_json::from_str::<RandomSpec>(&json).unwrap(), s);
        }
    }

    #[test]
    fn the_json_form_is_pinned() {
        assert_eq!(
            serde_json::to_string(&spec()).unwrap(),
            r#"{"generator":1,"seed":18446744073709551615,"difficulty":40,"amounts":[null,100,null,null,0,null],"musical":100,"template":0}"#
        );
    }

    #[test]
    fn floats_and_unknown_fields_are_refused() {
        let float = r#"{"generator":1,"seed":1,"difficulty":40.5,"amounts":[null,null,null,null,null,null],"musical":100,"template":0}"#;
        assert!(serde_json::from_str::<RandomSpec>(float).is_err());
        let extra = r#"{"generator":1,"seed":1,"difficulty":40,"amounts":[null,null,null,null,null,null],"musical":100,"template":0,"x":1}"#;
        assert!(serde_json::from_str::<RandomSpec>(extra).is_err());
        let short = r#"{"generator":1,"seed":1,"difficulty":40,"amounts":[null],"musical":100,"template":0}"#;
        assert!(serde_json::from_str::<RandomSpec>(short).is_err());
    }

    #[test]
    fn validate_checks_ranges_and_the_generator() {
        assert_eq!(spec().validate(), Ok(()));
        let mut s = spec();
        s.generator = 0;
        assert_eq!(s.validate(), Err(SpecError::NoGenerator));
        s.generator = LATEST_GENERATOR + 1;
        let e = s.validate().unwrap_err();
        assert_eq!(e.code(), "code_too_new");
        assert!(e.hint().contains("update enth"), "{}", e.hint());
        let mut s = spec();
        s.difficulty = 101;
        assert_eq!(
            s.validate(),
            Err(SpecError::OutOfRange {
                field: "difficulty",
                value: 101
            })
        );
        let mut s = spec();
        s.amounts[Category::Effects.index()] = Some(200);
        assert_eq!(
            s.validate(),
            Err(SpecError::OutOfRange {
                field: "amounts.effects",
                value: 200
            })
        );
        let mut s = spec();
        s.musical = 255;
        assert_eq!(s.validate().unwrap_err().code(), "out_of_range");
    }

    #[test]
    fn presets_are_frozen() {
        // Code v1's preset table. Changing a row changes what old short codes name: never edit, only append.
        let table: Vec<_> = PRESETS
            .iter()
            .map(|p| (p.id, p.difficulty, p.amounts, p.musical, p.template))
            .collect();
        let none = [None; Category::COUNT];
        assert_eq!(
            table,
            [
                (1, 0, none, 100, 0),
                (2, 25, none, 100, 0),
                (3, 50, none, 100, 0),
                (4, 75, none, 100, 0),
                (5, 100, none, 100, 0),
            ]
        );
    }

    #[test]
    fn preset_specs_find_their_preset() {
        for p in &PRESETS {
            let s = p.spec(LATEST_GENERATOR, 7);
            assert_eq!(s.validate(), Ok(()));
            assert_eq!(s.preset(), Some(p));
            assert_eq!(SpecPreset::by_id(p.id), Some(p));
        }
        assert_eq!(spec().preset(), None);
        assert_eq!(SpecPreset::by_id(0), None);
        assert_eq!(SpecPreset::by_id(6), None);
        let ids: Vec<u8> = PRESETS.iter().map(|p| p.id).collect();
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(ids, sorted, "preset ids are unique and in order");
    }
}
