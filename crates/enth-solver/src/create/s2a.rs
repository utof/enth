//! **Owner: lane TPL** (S2a wave 5). The S2a create templates: `growl`, `reese`, `wobble` and `fm_bass`
//! (`content/templates/s2a/`; docs/s2a-templates.md).
//!
//! Create searches only parameter values, so its templates decide which structures it can reach. These four use the
//! S2a kinds (`wt`, `dist`, `comb`, `formant`, `mseg`, a synced `lfo`, `ott`, `phaser`, `chorus`…), which stay hidden
//! until INT-S2a clears their flags. So there are two loaders:
//!
//! - [`s2a_templates`] returns all four, whatever the flags say: tests and measurements (the GR3 kit's `templates`
//!   job option) call it;
//! - [`enabled_s2a_templates`] returns the ones create may propose to users today: a template whose every module kind
//!   is visible ([`ModuleKind::is_hidden`]). While any of its kinds is hidden a template is left out, so today's
//!   create gets exactly the starts it had before this module existed (bit for bit), and INT-S2a switches the
//!   templates on by clearing the flags, with no edit here.
//!
//! The callers (`enth create`, the plugin's Create view, the GR3 kit) append [`enabled_s2a_templates`] to the
//! templates they already pass.

use enth_patch::{ModuleKind, Patch, file};

use super::Template;

/// The S2a templates: name, one-line summary, the patch file (embedded from `content/templates/s2a/`).
pub const S2A_TEMPLATES: [(&str, &str, &str); 4] = [
    (
        "growl",
        "an FM wavetable growl: a phase-modulated wt with a looping MSEG on index and position, distortion, a band-pass, a formant and a fold, OTT",
        include_str!("../../../../content/templates/s2a/growl.patch.json"),
    ),
    (
        "reese",
        "two detuned saws and a sine sub, a bar-long filter sweep, distortion that spares the lows, a phaser",
        include_str!("../../../../content/templates/s2a/reese.patch.json"),
    ),
    (
        "wobble",
        "two saw-and-pulse oscillators through a low-pass swept by a tempo-synced LFO, distortion, OTT",
        include_str!("../../../../content/templates/s2a/wobble.patch.json"),
    ),
    (
        "fm_bass",
        "a wavetable phase-modulated by a sine, an envelope on the index, a low-pass, distortion, a compressor",
        include_str!("../../../../content/templates/s2a/fm_bass.patch.json"),
    ),
];

/// The names of [`S2A_TEMPLATES`], in order.
#[must_use]
pub fn s2a_names() -> Vec<&'static str> {
    S2A_TEMPLATES.iter().map(|t| t.0).collect()
}

/// The patch of one embedded template.
///
/// # Panics
/// If the embedded file does not load: the tests load every one, so a released build cannot.
fn load(name: &str, text: &str) -> Patch {
    file::load(text)
        .unwrap_or_else(|e| panic!("the S2a template {name} does not load: {e}"))
        .patch
}

/// All four S2a templates, hidden kinds or not: for tests and measurements.
#[must_use]
pub fn s2a_templates() -> Vec<Template> {
    S2A_TEMPLATES
        .iter()
        .map(|(name, _, text)| Template {
            name: (*name).to_owned(),
            patch: load(name, text),
        })
        .collect()
}

/// The S2a template named `name`.
#[must_use]
pub fn s2a_template(name: &str) -> Option<Template> {
    S2A_TEMPLATES
        .iter()
        .find(|t| t.0 == name)
        .map(|(n, _, text)| Template {
            name: (*n).to_owned(),
            patch: load(n, text),
        })
}

/// Whether every module kind of `patch` is visible (none hidden).
#[must_use]
pub fn all_kinds_visible(patch: &Patch) -> bool {
    patch
        .modules()
        .all(|(_, m): (_, &enth_patch::ModuleInstance)| !ModuleKind::is_hidden(m.kind))
}

/// The S2a templates create may propose today: those whose every module kind is visible. Empty while the S2a kinds
/// are hidden, which keeps create's starts exactly as they were.
#[must_use]
pub fn enabled_s2a_templates() -> Vec<Template> {
    s2a_templates()
        .into_iter()
        .filter(|t| all_kinds_visible(&t.patch))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::validate::validate;

    #[test]
    fn every_s2a_template_loads_and_validates() {
        let all = s2a_templates();
        assert_eq!(
            all.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["growl", "reese", "wobble", "fm_bass"]
        );
        for (t, (_, summary, text)) in all.iter().zip(S2A_TEMPLATES) {
            let loaded = file::load(text).unwrap_or_else(|e| panic!("{}: {e}", t.name));
            assert!(loaded.notes.is_empty(), "{}: {:?}", t.name, loaded.notes);
            validate(&t.patch).unwrap_or_else(|e| panic!("{}: {e:?}", t.name));
            assert!(!summary.is_empty());
            assert!(s2a_template(&t.name).is_some_and(|x| x == *t));
        }
        assert!(s2a_template("pluck_1osc").is_none());
    }

    /// While an S2a kind is hidden no S2a template is enabled, so create's starts do not change. When INT-S2a clears
    /// the last flag a template uses, this test is the place to change (it then expects those templates).
    #[test]
    fn no_s2a_template_is_enabled_while_its_kinds_are_hidden() {
        for t in s2a_templates() {
            let hidden: Vec<_> = t
                .patch
                .modules()
                .filter(|(_, m)| m.kind.is_hidden())
                .map(|(_, m)| m.kind.key())
                .collect();
            assert!(
                !hidden.is_empty(),
                "{} uses no S2a kind: it would not need this loader",
                t.name
            );
            assert_eq!(all_kinds_visible(&t.patch), hidden.is_empty());
        }
        let enabled = enabled_s2a_templates();
        let expected: Vec<String> = s2a_templates()
            .into_iter()
            .filter(|t| t.patch.modules().all(|(_, m)| !m.kind.is_hidden()))
            .map(|t| t.name)
            .collect();
        assert_eq!(
            enabled.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            expected
        );
        assert!(enabled.is_empty(), "the S2a kinds are hidden until INT-S2a");
    }
}
