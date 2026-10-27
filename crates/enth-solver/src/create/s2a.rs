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

    fn sketches() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/templates/s2a/sketches");
        let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| {
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    /// One second of C3 at 24 kHz: finite, audible and not wild.
    fn plays(patch: &Patch, hold_s: f64, duration_s: f64) {
        use enth_render::{Performance, RenderSpec, render};
        let perf = Performance::note(48, 100.0 / 127.0, hold_s, duration_s);
        let out = render(patch, &perf, &RenderSpec::new(24_000, 1))
            .unwrap()
            .mid();
        assert!(out.iter().all(|x| x.is_finite()), "non-finite samples");
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "silent (peak {peak})");
        assert!(peak < 8.0, "wild (peak {peak})");
    }

    #[test]
    fn every_s2a_template_plays() {
        for t in s2a_templates() {
            plays(&t.patch, 0.6, 1.0);
        }
    }

    /// Report 04's sketches S1 to S10 (and S7b, S10b), as reference patches: they load, validate and play. S4m, the
    /// metallic variant of S4 as the report writes it, needs six voice effects and is refused: the chain-depth
    /// evidence for `MAX_VOICE_FX` (phase-s2.md's open check); the cap is the coordinator's to change.
    #[test]
    fn the_sketches_load_and_play_and_the_metallic_variant_exceeds_the_voice_fx_cap() {
        let all = sketches();
        assert_eq!(all.len(), 13);
        let mut played = 0;
        for (name, text) in &all {
            if name.starts_with("s04m") {
                let err = match file::load(text) {
                    Err(e) => format!("{e:?}"),
                    Ok(l) => match validate(&l.patch) {
                        Err(e) => format!("{e:?}"),
                        Ok(_) => panic!("{name} should be refused: it has six voice effects"),
                    },
                };
                assert!(
                    err.contains("voice_fx"),
                    "{name}: the refusal names voice_fx: {err}"
                );
                // The six voice effects the report's metallic variant lists.
                for id in ["dist1", "filter1", "formant1", "dist2", "dist3", "comb1"] {
                    assert!(text.contains(&format!("\"id\": \"{id}\"")), "{name}: {id}");
                }
                continue;
            }
            let loaded = file::load(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(loaded.notes.is_empty(), "{name}: {:?}", loaded.notes);
            validate(&loaded.patch).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (hold, dur) = if name.starts_with("s10b") {
                (3.0, 4.0)
            } else {
                (0.6, 1.0)
            };
            plays(&loaded.patch, hold, dur);
            played += 1;
        }
        assert_eq!(played, 12);
    }

    /// Each template leaves the search a space of a size it can use: the continuous and categorical axes the job
    /// would search under the dev-target performance (C3, held 1.5 s of 3 s).
    #[test]
    fn the_templates_leave_a_searchable_space() {
        use super::super::{TargetPerf, free_set};
        let perf = TargetPerf {
            key: 48,
            velocity: 100.0 / 127.0,
            on_s: 0.0,
            hold_s: 1.5,
            duration_s: 3.0,
            f0_hz: None,
        };
        for t in s2a_templates() {
            let free = free_set(&t.patch, &perf).unwrap();
            let (c, k) = (free.continuous().len(), free.categorical().len());
            assert!(
                (10..=160).contains(&(c + k)),
                "{}: {c} continuous and {k} categorical axes",
                t.name
            );
        }
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
