//! Lane G2B (S2a): the G2 reference patch is one sound in four places.
//!
//! The `g2` bench and the engine's `g2` test build it from `crates/enth-engine/benches/common/g2.rs`; this crate
//! includes the same file. The test host's fixtures `tools/test-host/golden/enth-plugin.g2*.patch.json` (the real-time
//! run) and the golden case `cases/g2-growl.toml` must equal what that file builds, so the budget, the golden and the
//! p99 row measure one patch. After a change to `common/g2.rs`, regenerate the fixtures with
//! `cargo test -p enth-golden --test g2 -- --ignored write_the_fixtures` and update the case's patch by hand (the
//! test names what differs).

#[path = "../../../crates/enth-engine/benches/common/g2.rs"]
mod g2_patch;

use std::path::PathBuf;

use enth_golden::patch::patch_from_file;
use enth_golden::{Case, Source, root};
use enth_patch::Patch;
use g2_patch::{g2_patch, module_added, value_edit};

fn fixture_dir() -> PathBuf {
    root().join("../../tools/test-host/golden")
}

/// A file carries the engine version that wrote it (`Patch::init()` has none), which a bump of `ENGINE_VERSION` moves
/// without touching the sound, so it is not compared.
fn without_engine(mut p: Patch) -> Patch {
    p.engine = String::new();
    p
}

fn fixture(name: &str) -> Patch {
    let path = fixture_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let loaded = enth_patch::file::load(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    without_engine(loaded.patch)
}

const FIXTURES: [&str; 3] = [
    "enth-plugin.g2.patch.json",
    "enth-plugin.g2-value.patch.json",
    "enth-plugin.g2-lfo3.patch.json",
];

fn bench_patches() -> [Patch; 3] {
    [g2_patch(), value_edit(), module_added()]
}

#[test]
fn the_test_host_fixtures_are_the_bench_patches() {
    for (name, patch) in FIXTURES.into_iter().zip(bench_patches()) {
        assert!(
            fixture(name) == without_engine(patch),
            "tools/test-host/golden/{name} is not the bench patch: regenerate it (see the header of this file)"
        );
    }
}

#[test]
#[ignore = "writes the test host's G2 fixtures"]
fn write_the_fixtures() {
    for (name, patch) in FIXTURES.into_iter().zip(bench_patches()) {
        std::fs::write(fixture_dir().join(name), patch.to_json()).expect("write the fixture");
    }
}

#[test]
fn the_golden_case_is_the_bench_patch() {
    let case: Case = toml::from_str(
        &std::fs::read_to_string(root().join("cases/g2-growl.toml")).expect("cases/g2-growl.toml"),
    )
    .expect("the case parses");
    let Source::Patch { patch, .. } = &case.source else {
        panic!("g2-growl is a patch case");
    };
    let from_case = without_engine(patch_from_file(patch).expect("the case's patch is valid"));
    assert!(
        from_case == without_engine(g2_patch()),
        "tests/golden/cases/g2-growl.toml is not the bench patch: edit its [source.patch] to match \
         crates/enth-engine/benches/common/g2.rs"
    );
}
