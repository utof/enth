//! Same seed -> same SHA-256; scalar = SIMD lanes; goldens for renders and for 1,000 seed -> patch hashes.
//! These run unchanged on Linux, Windows and wasm32-wasip1 (`cargo xtask wasm-hash` and CI), where they
//! compare against the same golden files.

use std::collections::BTreeSet;

use spike_engine::render::{
    PATCH_SEEDS, frames, held_chord, patch_golden_text, phrase, render_cases, render_golden_text,
};
use spike_engine::{Flush, Patch, Path, hash_samples, render};

const SEED: u64 = 0x0E17_5EED;

fn assert_golden(what: &str, regenerate: &str, actual: &str, expected: &str) {
    let a: Vec<&str> = actual.lines().collect();
    let e: Vec<&str> = expected.lines().collect();
    let mut bad = Vec::new();
    for i in 0..a.len().max(e.len()) {
        if a.get(i) != e.get(i) {
            bad.push(format!(
                "  line {}: golden {:?}, got {:?}",
                i + 1,
                e.get(i),
                a.get(i)
            ));
            if bad.len() >= 6 {
                break;
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{what} differ from the golden file:\n{}\nIf the engine or generator changed on purpose, bump ENGINE_VERSION / PATCH_VERSION \
         and regenerate with `{regenerate}`; if only one platform differs, that platform breaks the determinism contract.",
        bad.join("\n")
    );
}

#[test]
fn same_seed_same_hash_twice() {
    for c in render_cases().iter().filter(|c| c.seconds <= 4.0) {
        let n = frames(c.seconds);
        let a = hash_samples(&render(
            &c.patch,
            &c.notes,
            n,
            SEED,
            Path::Scalar,
            Flush::On,
        ));
        let b = hash_samples(&render(
            &c.patch,
            &c.notes,
            n,
            SEED,
            Path::Scalar,
            Flush::On,
        ));
        assert_eq!(a, b, "case {} is not repeatable", c.name);
    }
}

#[test]
fn the_seed_matters() {
    let patch = Patch::p1_like();
    let notes = phrase();
    let n = frames(1.0);
    let a = hash_samples(&render(&patch, &notes, n, 1, Path::Scalar, Flush::On));
    let b = hash_samples(&render(&patch, &notes, n, 2, Path::Scalar, Flush::On));
    assert_ne!(
        a, b,
        "voice phases, detune and reverb jitter must follow the seed"
    );
}

#[test]
fn simd_lanes_equal_scalar() {
    let mut cases: Vec<(String, Patch, Vec<spike_engine::Note>)> = vec![
        ("simple 1 voice".into(), Patch::simple(), held_chord(1, 2.0)),
        (
            "simple 5 voices (partial lane group)".into(),
            Patch::simple(),
            held_chord(5, 2.0),
        ),
        (
            "simple 16 voices".into(),
            Patch::simple(),
            held_chord(16, 2.0),
        ),
        (
            "p1 8 voices held".into(),
            Patch::p1_like(),
            held_chord(8, 2.0),
        ),
        ("p1 phrase".into(), Patch::p1_like(), phrase()),
    ];
    for seed in 0..24u64 {
        cases.push((
            format!("random patch {seed}"),
            Patch::from_seed(seed),
            phrase(),
        ));
    }
    for (name, patch, notes) in cases {
        let n = frames(if name.starts_with("random") { 1.5 } else { 2.0 });
        let scalar = hash_samples(&render(&patch, &notes, n, SEED, Path::Scalar, Flush::On));
        for path in [Path::X4, Path::X8] {
            let lanes = hash_samples(&render(&patch, &notes, n, SEED, path, Flush::On));
            assert_eq!(
                scalar, lanes,
                "{name}: {path:?} differs from the scalar path"
            );
        }
    }
}

#[test]
fn render_hashes_match_the_golden_file() {
    assert_golden(
        "render hashes",
        "cargo run --release -p spike-engine --bin engine-hash -- render > spikes/spike-engine/golden/render.txt",
        &render_golden_text(),
        include_str!("../golden/render.txt"),
    );
}

#[test]
fn patch_hashes_of_1000_seeds_match_the_golden_file() {
    assert_golden(
        "patch hashes",
        "cargo run --release -p spike-engine --bin engine-hash -- patches > spikes/spike-engine/golden/patches.txt",
        &patch_golden_text(),
        include_str!("../golden/patches.txt"),
    );
}

#[test]
fn generated_patches_are_in_range_and_distinct() {
    let mut seen = BTreeSet::new();
    for seed in 0..PATCH_SEEDS {
        let p = Patch::from_seed(seed);
        assert_eq!(
            p,
            Patch::from_seed(seed),
            "generator must be a pure function of the seed"
        );
        assert!((1..=3).contains(&p.osc_count));
        let f = p.filter;
        assert!(
            (40.0..14_500.0).contains(&f.cutoff_hz),
            "seed {seed}: cutoff {}",
            f.cutoff_hz
        );
        assert!((0.5..16.1).contains(&f.q) && (1.0..3.0).contains(&f.drive));
        for e in [p.amp_env, p.filter_env] {
            assert!((1.0..=1024.0).contains(&e.attack_ms) && (5.0..=5120.0).contains(&e.decay_ms));
            assert!(
                (5.0..=5120.0).contains(&e.release_ms) && (0.0..100.0).contains(&e.sustain_pct)
            );
        }
        assert!((0.3..=4.8).contains(&p.reverb.rt60_s) && (-12.0..0.0).contains(&p.master_db));
        assert!(seen.insert(p.hash()), "seed {seed}: duplicate patch hash");
    }
}

/// Random patches are safe for the engine: finite, bounded, no subnormals, audible.
#[test]
fn random_patches_render_clean() {
    for seed in 0..32u64 {
        let patch = Patch::from_seed(seed);
        let out = render(
            &patch,
            &phrase(),
            frames(2.0),
            SEED,
            Path::Scalar,
            Flush::On,
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            out.iter().all(|x| x.is_finite() && !x.is_subnormal()),
            "seed {seed}"
        );
        assert!(peak < 1.0, "seed {seed}: peak {peak}");
        assert!(peak > 1e-4, "seed {seed}: silent render, peak {peak}");
    }
}
