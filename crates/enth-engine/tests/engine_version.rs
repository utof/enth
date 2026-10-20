//! The engine version has two homes because of the layers: `enth_engine::ENGINE_VERSION` (L2) is the version of the
//! DSP that the goldens pin, and `enth_patch::file::ENGINE_VERSION` (L1, which cannot depend on the engine) is what
//! a patch with no stamp is written with. They must be the same string, or every patch a fresh plugin instance or
//! the CLI saves names an engine that never rendered it (found by lane INT: 0.1.0 against 0.2.0).
//!
//! A DSP change that bumps `enth_engine::ENGINE_VERSION` fails here until the constant in
//! `crates/enth-patch/src/file/mod.rs` is bumped with it.

use enth_patch::Patch;
use enth_patch::file;

#[test]
fn the_patch_crates_stamp_is_the_engines_version() {
    assert_eq!(
        file::ENGINE_VERSION,
        enth_engine::ENGINE_VERSION,
        "bump `enth_patch::file::ENGINE_VERSION` together with `enth_engine::ENGINE_VERSION`"
    );
}

#[test]
fn a_patch_with_no_stamp_is_written_with_the_engines_version() {
    let init = Patch::init();
    assert!(init.engine.is_empty(), "Patch::init() has no stamp yet");
    assert_eq!(init.to_file().engine, enth_engine::ENGINE_VERSION);
}
