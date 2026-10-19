//! Seeds from (render seed, module id, note index); never from parameter values or the patch hash (design.md §5).
//!
//! ```text
//! module_key = fnv1a64(module_id bytes)                       stable when other modules come and go
//! per voice:          seed = derive(derive(render_seed, module_key), note_index)
//! per global module:  seed = derive(render_seed, module_key)
//! seeded phase mode:  seed = derive(derive(param_seed, module_key), note_index)
//! ```
//!
//! `render_seed` comes from the render spec or the challenge (the CLI has a default and `--seed`); `note_index`
//! counts note-ons since engine reset, from 0. Moving a knob never reseeds anything: the seed-stability test in
//! `enth-render` guards it (a 1e-3 relative move of any continuous parameter moves the 10 ms RMS envelope by under
//! 0.05 dB). A patch that wants a different random character uses a module's explicit `seed` parameter with the
//! `seeded` phase mode, which replaces the render seed in the voice rule, so the patch alone fixes it.
//!
//! The arithmetic (`SplitMix64`, `derive`, `fnv1a64`) is in `enth_dsp::util::rng`.

use enth_dsp::util::rng::{derive, fnv1a64};
use enth_patch::ModuleId;

/// The stable key of a module id: `fnv1a64` of its bytes.
#[must_use]
pub fn module_key(id: &ModuleId) -> u64 {
    fnv1a64(id.as_str().as_bytes())
}

/// The seed of a global module (global LFO, effect): `derive(render_seed, module_key)`.
#[must_use]
pub const fn global_seed(render_seed: u64, module_key: u64) -> u64 {
    derive(render_seed, module_key)
}

/// The seed of one note in a per-voice module: `derive(derive(render_seed, module_key), note_index)`.
#[must_use]
pub const fn voice_seed(render_seed: u64, module_key: u64, note_index: u64) -> u64 {
    derive(derive(render_seed, module_key), note_index)
}

/// The seed of one note in a per-voice module in the `seeded` phase mode: the voice rule with the module's own
/// `seed` parameter (an integer 0–65,535 stored in the patch) in place of the render seed, so a different render
/// seed does not move it.
#[must_use]
pub const fn seeded_voice_seed(param_seed: u32, module_key: u64, note_index: u64) -> u64 {
    voice_seed(param_seed as u64, module_key, note_index)
}

/// The global counterpart of [`seeded_voice_seed`]: `derive(param_seed, module_key)`.
#[must_use]
pub const fn seeded_global_seed(param_seed: u32, module_key: u64) -> u64 {
    global_seed(param_seed as u64, module_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    #[test]
    fn keys_follow_ids_only() {
        assert_eq!(module_key(&id("osc1")), fnv1a64(b"osc1"));
        assert_ne!(module_key(&id("osc1")), module_key(&id("osc2")));
    }

    #[test]
    fn the_rule_is_pinned() {
        // A change here changes every seeded render: an engine-version change and new goldens.
        let k = module_key(&id("osc1"));
        assert_eq!(global_seed(0x0E17_5EED, k), 0x2931_f78f_4edb_0cc0);
        assert_eq!(voice_seed(0x0E17_5EED, k, 3), 0x12dd_a249_22a4_ad74);
        assert_eq!(
            seeded_voice_seed(7, k, 3),
            derive(derive(7, k), 3),
            "seeded mode replaces the render seed"
        );
        assert_eq!(seeded_global_seed(7, k), derive(7, k));
    }

    #[test]
    fn streams_are_distinct_per_module_note_and_render_seed() {
        let a = module_key(&id("osc1"));
        let b = module_key(&id("osc2"));
        let mut seen = std::collections::BTreeSet::new();
        for rs in [0, 1, 0x0E17_5EED] {
            for k in [a, b] {
                assert!(seen.insert(global_seed(rs, k)));
                for n in 0..64 {
                    assert!(
                        seen.insert(voice_seed(rs, k, n)),
                        "collision at {rs} {k} {n}"
                    );
                }
            }
        }
    }
}
