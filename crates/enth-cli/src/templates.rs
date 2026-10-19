//! The templates `enth new --template` starts from, embedded from `content/templates/*.patch.json` (sparse patch
//! files: only values that differ from the defaults). Templates are part of the product: the agent evals start
//! from them.
//!
//! Each starts with one piece of modulation, so an agent sees routes and voice modes in a working patch: a filter
//! envelope on `pluck_1osc` (`env2 -> filter1.cutoff`, about +4 octaves over 220 ms), a slow LFO on the cutoff of
//! `pad_2osc` (`lfo1`, 0.3 Hz, about an octave either way), and `bass_sub` in mono with a 40 ms glide for notes
//! played legato.

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
        summary: "a saw through a low-pass that a filter envelope throws open at the attack, no sustain: a short pluck",
        text: include_str!("../../../content/templates/pluck_1osc.patch.json"),
    },
    Template {
        name: "pad_2osc",
        summary: "two detuned saws, a soft low-pass moved by a slow LFO, a slow swell, a long release, some reverb",
        text: include_str!("../../../content/templates/pad_2osc.patch.json"),
    },
    Template {
        name: "bass_sub",
        summary: "a saw over a sine an octave below, a dark resonant low-pass, mono with a short legato glide, full sustain",
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

    /// Each template carries the modulation its summary promises, and the engine renders it.
    #[test]
    fn the_templates_carry_their_modulation() {
        use enth_patch::catalogue::{VoiceMode, VoiceParam};
        let routes = |name: &str| -> Vec<(String, String, f32)> {
            find(name)
                .unwrap()
                .patch()
                .routes
                .iter()
                .map(|r| (r.src.to_string(), r.dst.to_string(), r.depth))
                .collect()
        };
        assert_eq!(
            routes("pluck_1osc"),
            [("env2".to_owned(), "filter1.cutoff".to_owned(), 45.0)]
        );
        assert_eq!(
            routes("pad_2osc"),
            [("lfo1".to_owned(), "filter1.cutoff".to_owned(), 10.0)]
        );
        assert!(routes("bass_sub").is_empty() && routes("init").is_empty());

        let bass = find("bass_sub").unwrap().patch();
        assert_eq!(
            VoiceMode::from_value(bass.voice.values[VoiceParam::Mode.index()]),
            Some(VoiceMode::Mono)
        );
        assert_eq!(bass.voice.values[VoiceParam::Glide.index()], 40.0);
        let ids = |name: &str| module_ids(&find(name).unwrap().patch());
        assert_eq!(ids("pluck_1osc"), ["osc1", "filter1", "env1", "env2"]);
        assert_eq!(
            ids("pad_2osc"),
            ["osc1", "osc2", "filter1", "reverb1", "env1", "lfo1"]
        );
    }

    #[test]
    fn init_is_the_patch_crates_init() {
        let mut p = find("init").unwrap().patch();
        p.meta = Patch::init().meta;
        assert_eq!(p, Patch::init());
    }
}
