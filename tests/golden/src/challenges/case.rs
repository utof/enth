//! A golden challenge's input: `challenges/cases/<name>.toml`.
//!
//! ```toml
//! description = "Level 2, seed 22: a filtered held note"
//! code = "1080-1000-0000-PJ"            # what a player pastes, in canonical form (short or long)
//! scorings = ["v1"]                     # optional; the scorer sets to pin (default: ["v1"])
//!
//! [answers.near]                        # up to two more answers besides `exact` and `start`
//! base = "target"                       # "target" (default) or "start"
//! set = { "osc1.semi" = 7.0 }           # then, in this order: enable, set, mul, add, depth
//! mul = { "filter1.cutoff" = 1.25 }     # times the base's value
//! add = { "env1.sustain" = -5.0 }       # plus
//! enable = { osc2 = false }
//! depth = { "lfo1 -> filter1.cutoff" = 40.0 }
//! ```
//!
//! Values are plain numbers in the parameter's real unit (Hz, ms, dB, %, semitones). `exact` is the target patch and
//! `start` the template's start patch; both always exist.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A case file.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// One line on what the case guards.
    pub description: String,
    /// The challenge code, canonical text: the generator's settings, and for a long code the engine version and the
    /// target-fingerprint prefix.
    pub code: String,
    /// The scorer sets to pin, by name (see [`crate::challenges::SETS`]). `["v1"]` when absent.
    #[serde(default = "default_scorings")]
    pub scorings: Vec<String>,
    /// The answers besides `exact` (the target) and `start`, by name.
    #[serde(default)]
    pub answers: BTreeMap<String, Recipe>,
}

fn default_scorings() -> Vec<String> {
    vec!["v1".to_owned()]
}

/// How an answer patch is made: a base, and edits to it.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// What the edits apply to.
    #[serde(default)]
    pub base: Base,
    /// Module enable flags (`osc2 = false`).
    #[serde(default)]
    pub enable: BTreeMap<String, bool>,
    /// Parameters set to a plain value.
    #[serde(default)]
    pub set: BTreeMap<String, f64>,
    /// Parameters multiplied: the new value is the base's times the factor.
    #[serde(default)]
    pub mul: BTreeMap<String, f64>,
    /// Parameters moved: the new value is the base's plus the amount.
    #[serde(default)]
    pub add: BTreeMap<String, f64>,
    /// Route depths set (`"lfo1 -> filter1.cutoff" = 40.0`), in percent.
    #[serde(default)]
    pub depth: BTreeMap<String, f64>,
}

/// The patch a recipe starts from.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Base {
    /// The generated target.
    #[default]
    Target,
    /// The template's start patch (what a round begins with).
    Start,
}

/// Names the harness reserves for its own answers.
pub const RESERVED: [&str; 2] = ["exact", "start"];

impl Case {
    /// Parse a case file's text.
    ///
    /// # Errors
    /// Invalid TOML, an unknown key, a reserved or malformed answer name, or more than two extra answers.
    pub fn parse(text: &str) -> Result<Case, String> {
        let case: Case = toml::from_str(text).map_err(|e| e.to_string())?;
        if case.answers.len() > 2 {
            return Err("a case pins at most two answers besides exact and start".to_owned());
        }
        for name in case.answers.keys() {
            if RESERVED.contains(&name.as_str()) || !super::valid_name(name) {
                return Err(format!(
                    "answer name `{name}`: lowercase letters, digits and '-', and not exact or start"
                ));
            }
        }
        Ok(case)
    }
}
