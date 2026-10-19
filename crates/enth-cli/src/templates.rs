//! The templates `enth new --template` starts from, embedded from `content/templates/*.patch.json` (sparse patch
//! files: only values that differ from the defaults). Templates are part of the product: the agent evals start
//! from them.
//!
//! The engine at freeze F2 refuses modulation routes and the mono and legato voice modes (`not_yet`, lane S2b), so
//! the templates use neither yet. Once S2b is merged, the natural next step is a filter envelope on `pluck_1osc`
//! (`env2 -> filter1.cutoff`), a slow LFO on the pad's cutoff, and `voice.mode = mono` with a filter blip on
//! `bass_sub`; each is one `add` and one `route` op.

use enth_patch::{Patch, file};

/// One template.
pub struct Template {
    /// The name given to `--template`.
    pub name: &'static str,
    /// What it is, in one line.
    pub summary: &'static str,
    /// The patch file.
    pub text: &'static str,
}

/// Every template, in the order `enth new --list` prints them.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "init",
        summary: "one saw through the amplitude envelope: a blank start",
        text: include_str!("../../../content/templates/init.patch.json"),
    },
    Template {
        name: "pluck_1osc",
        summary: "a saw through a bright low-pass, fast attack, short decay, no sustain: a short pluck",
        text: include_str!("../../../content/templates/pluck_1osc.patch.json"),
    },
    Template {
        name: "pad_2osc",
        summary: "two detuned saws, a soft low-pass, a slow swell, a long release, some reverb",
        text: include_str!("../../../content/templates/pad_2osc.patch.json"),
    },
    Template {
        name: "bass_sub",
        summary: "a saw over a sine an octave below, a dark resonant low-pass, fast attack, full sustain",
        text: include_str!("../../../content/templates/bass_sub.patch.json"),
    },
];

/// The template named `name`.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name.trim())
}

/// The names, for errors.
pub fn names() -> Vec<String> {
    TEMPLATES.iter().map(|t| t.name.to_owned()).collect()
}

impl Template {
    /// The patch (templates are checked by the tests, so this cannot fail in a released build).
    pub fn patch(&self) -> Patch {
        file::load(self.text)
            .unwrap_or_else(|e| unreachable!("template {} is invalid: {e}", self.name))
            .patch
    }
}

/// Module ids of a patch, in section order.
pub fn module_ids(p: &Patch) -> Vec<String> {
    p.modules().map(|(_, m)| m.id.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_loads_and_validates() {
        for t in TEMPLATES {
            let loaded = file::load(t.text).unwrap_or_else(|e| panic!("{}: {e}", t.name));
            assert!(loaded.notes.is_empty(), "{}", t.name);
            enth_patch::validate::validate(&loaded.patch).unwrap();
            assert_ne!(t.summary, "");
        }
        assert_eq!(names(), ["init", "pluck_1osc", "pad_2osc", "bass_sub"]);
    }

    #[test]
    fn init_is_the_patch_crates_init() {
        let mut p = find("init").unwrap().patch();
        p.meta = Patch::init().meta;
        assert_eq!(p, Patch::init());
    }
}
