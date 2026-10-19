//! `process()` never allocates: every block runs inside `assert_no_alloc` (debug builds only: the crate
//! removes `AllocDisabler` in release unless a feature is set). A negative control proves the guard bites.
#![cfg(debug_assertions)]

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use spike_engine::render::{frames, held_chord, phrase};
use spike_engine::{BLOCK, Engine, Flush, Note, Patch, Path};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn run_guarded(patch: &Patch, notes: &[Note], seconds: f32, path: Path) {
    let mut engine = Engine::new(patch, notes, 3, path, Flush::On); // prepare: may allocate
    let mut out = vec![0.0f32; frames(seconds)];
    for block in out.chunks_mut(BLOCK) {
        assert_no_alloc(|| engine.process(block));
    }
    assert!(
        out.iter().any(|x| *x != 0.0),
        "the render should not be silent"
    );
}

#[test]
fn process_does_not_allocate() {
    for path in [Path::Scalar, Path::X4, Path::X8] {
        run_guarded(&Patch::simple(), &held_chord(16, 1.0), 1.0, path);
        run_guarded(&Patch::p1_like(), &held_chord(8, 1.0), 1.0, path);
        run_guarded(&Patch::p1_like(), &phrase(), 3.0, path);
    }
}

#[test]
fn process_does_not_allocate_for_random_patches() {
    for seed in 0..16u64 {
        run_guarded(&Patch::from_seed(seed), &phrase(), 1.0, Path::X8);
    }
}

/// Negative control: an allocation inside the guard aborts the process, so run it in a child.
#[test]
#[cfg(not(target_family = "wasm"))]
fn an_allocation_inside_the_guard_is_detected() {
    if std::env::var_os("SPIKE_ENGINE_NOALLOC_CHILD").is_some() {
        assert_no_alloc(|| {
            let v: Vec<f32> = Vec::with_capacity(16);
            std::hint::black_box(v);
        });
        return;
    }
    let exe = std::env::current_exe().expect("test executable path");
    let status = std::process::Command::new(exe)
        .args([
            "an_allocation_inside_the_guard_is_detected",
            "--exact",
            "--nocapture",
        ])
        .env("SPIKE_ENGINE_NOALLOC_CHILD", "1")
        .output()
        .expect("spawn child test");
    assert!(
        !status.status.success(),
        "an allocation inside assert_no_alloc was not detected"
    );
}
