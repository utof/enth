//! **Owner: lane PR** (factory presets; it follows lane PBC's preset format). Pre-declared by lane S0r (R1/S1) as a
//! stub that refuses to run.
//!
//! `cargo xtask preset-add FILE --name NAME --tags T1,T2`: load a drafted patch (an agent's, from the CLI), set its
//! meta (name, tags from the factory vocabulary), stamp the engine version, and write exactly `Patch::to_json()`
//! under `content/presets/`, which `tests/presets.rs` requires (report 03 §2.4). It sits beside `extract-presets`
//! (`preset.rs`) without editing it.

/// `cargo xtask preset-add`: not built yet.
pub fn preset_add(_args: &[String]) -> Result<(), String> {
    Err("preset-add is not built yet (R1/S1 lane PR; docs/phase-2.md)".to_owned())
}
