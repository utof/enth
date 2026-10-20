//! The Init patch a fresh instance plays (lane AM, R1/S1; report 03 §2.2, summary "Init patch").
//!
//! Init is the starter patch, gain-staged: every P1 module, osc 1 into the filter and env 1 on the amp, the extras
//! (osc 2, the noise, the compressor, the reverb) switched off and env 2 and both LFOs unrouted, so a newcomer toggles
//! a card on instead of building from nothing, and the disabled modules cost no CPU (they are not in the plan) and no
//! structure score in Recreate (`canonical_hash` drops them). The master sits at −12 dB: a four-note chord on the
//! ungain-staged starter peaked at up to +7.9 dBFS (report 04), and Init must stay at or below −1 dBFS (the test
//! below renders the worst of several chords and render seeds).
//!
//! The patch is the factory preset `content/presets/init.patch.json`, exactly `Patch::to_json()` (the presets test
//! checks it), so the preset browser's "Init" and a fresh instance are the same sound, and `cargo xtask starter-project`
//! loads it on its four tracks. `Patch::init()` stays the minimal patch of the CLI, the session's own default and the
//! tests; a saved state that does not load still falls back to it (the session's rule, docs/session.md).

use enth_patch::Patch;

/// The Init preset, as the plugin saves it.
pub const INIT_PATCH: &str = include_str!("../../../content/presets/init.patch.json");

/// The patch a fresh instance plays. Its engine stamp is cleared, so a fresh instance's first save carries this
/// build's engine version (docs/plugin.md, "Maintaining the fixtures"), not the version the preset was written by. A
/// preset that did not load (a broken build: the presets test fails first) gives the minimal init patch, never a
/// panic.
pub fn patch() -> Patch {
    let mut patch = enth_patch::file::load(INIT_PATCH).map_or_else(|_| Patch::init(), |l| l.patch);
    patch.engine.clear();
    patch
}

#[cfg(test)]
mod tests {
    use enth_patch::{ModuleKind, Section};
    use enth_render::{DEFAULT_SEED, Note, Performance, RenderSpec, render};

    use super::*;

    /// −1 dBFS as a linear peak: 10^(−1/20).
    const MINUS_1_DBFS: f32 = 0.891_250_9;

    #[test]
    fn init_holds_every_p1_module_with_the_extras_off() {
        let p = patch();
        assert_eq!(p.meta.name, "Init");
        assert!(
            p.engine.is_empty(),
            "a fresh instance stamps its own engine version"
        );
        for &kind in ModuleKind::ALL {
            assert!(
                p.modules().any(|(_, m)| m.kind == kind),
                "{kind:?} is in Init"
            );
        }
        let on: Vec<&str> = p
            .modules()
            .filter(|(_, m)| m.enabled)
            .map(|(_, m)| m.id.as_str())
            .collect();
        assert_eq!(on, ["osc1", "filter1", "env1", "env2", "lfo1", "lfo2"]);
        assert_eq!(p.routes, []);
        assert_eq!(p.voice.amp_env.as_str(), "env1");
        assert_eq!(p.section(Section::Generators).len(), 3);
        assert!(enth_patch::validate::validate(&p).is_ok());
    }

    #[test]
    fn a_four_note_chord_on_init_peaks_at_or_below_minus_1_dbfs() {
        let p = patch();
        let chords: [(&str, [u8; 4]); 4] = [
            ("C4 E4 G4 C5", [60, 64, 67, 72]),
            ("C2 C3 G3 C4", [36, 48, 55, 60]),
            ("A4 C5 E5 A5", [69, 72, 76, 81]),
            ("C4 C#4 D4 D#4", [60, 61, 62, 63]),
        ];
        // Free-running oscillator phases follow the render seed: the default seed and seven more.
        let seeds = [DEFAULT_SEED, 1, 2, 3, 5, 8, 13, 21];
        let mut worst = (0.0f32, "", 0u64);
        for (name, keys) in chords {
            for seed in seeds {
                let performance = Performance {
                    notes: keys.iter().map(|&k| Note::new(k, 1.0, 0.0, 1.5)).collect(),
                    duration_s: 2.0,
                    ..Performance::default()
                };
                let peak = render(&p, &performance, &RenderSpec::new(48_000, seed))
                    .expect("Init renders")
                    .peak();
                if peak > worst.0 {
                    worst = (peak, name, seed);
                }
            }
        }
        let (peak, chord, seed) = worst;
        assert!(
            peak <= MINUS_1_DBFS,
            "{chord} (seed {seed}) peaks at {peak:.3} (linear), over -1 dBFS ({MINUS_1_DBFS})"
        );
        // Gain-staged, not silenced: the loudest chord is within 6 dB of the limit.
        assert!(
            peak >= MINUS_1_DBFS / 2.0,
            "{chord} (seed {seed}) peaks at only {peak:.3}"
        );
    }

    #[test]
    fn a_fresh_instance_plays_init_and_saves_it_with_this_builds_stamp() {
        let plugin = crate::Enth::default();
        let shared = plugin.shared();
        assert!(enth_patch::canon::identical(
            shared.session.lock().patch(),
            &patch()
        ));
        let saved = nice_plug::prelude::Params::serialize_fields(plugin.host_params().as_ref());
        let saved = enth_patch::file::load(&saved["patch"]).unwrap().patch;
        assert_eq!(saved.engine, enth_patch::file::ENGINE_VERSION);
        assert_eq!(saved.meta.name, "Init");
    }
}
