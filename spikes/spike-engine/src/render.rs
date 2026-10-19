//! The offline renderer, the reference performances and the golden cases.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::engine::{Engine, Flush, Note, Path};
use crate::patch::{self, Patch};
use crate::{BLOCK, ENGINE_VERSION, SAMPLE_RATE};

/// Seed of all golden renders.
pub const GOLDEN_SEED: u64 = 0x0E17_5EED;
/// Seeds `0..PATCH_SEEDS` are hashed as random patches.
pub const PATCH_SEEDS: u64 = 1000;

/// Render `frames` mono samples in fixed blocks of [`BLOCK`]. Allocates the output once.
#[must_use]
pub fn render(
    patch: &Patch,
    notes: &[Note],
    frames: usize,
    seed: u64,
    path: Path,
    flush: Flush,
) -> Vec<f32> {
    let mut engine = Engine::new(patch, notes, seed, path, flush);
    let mut out = vec![0.0f32; frames];
    for block in out.chunks_mut(BLOCK) {
        engine.process(block);
    }
    out
}

/// Frames in `seconds` at the canonical rate.
#[must_use]
pub fn frames(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE) as usize
}

/// SHA-256 over (sample rate, channels, frame count) and the little-endian bits of every sample.
/// Hashing quantised samples would let one rounding-boundary flip change the digest, so raw bits it is.
///
/// # Panics
/// On a NaN or infinite sample: those are engine bugs, and NaN payloads are not deterministic.
#[must_use]
pub fn hash_samples(samples: &[f32]) -> String {
    let mut h = Sha256::new();
    h.update((SAMPLE_RATE as u32).to_le_bytes());
    h.update(1u16.to_le_bytes());
    h.update((samples.len() as u64).to_le_bytes());
    for s in samples {
        assert!(s.is_finite(), "non-finite sample in render");
        h.update(s.to_bits().to_le_bytes());
    }
    patch::hex(&h.finalize())
}

/// `voices` notes held from frame 0 for `seconds`: the speed template (worst case, every voice sounding).
#[must_use]
pub fn held_chord(voices: usize, seconds: f32) -> Vec<Note> {
    let len = frames(seconds);
    (0..voices)
        .map(|v| Note {
            start: 0,
            len,
            pitch: 36.0 + ((v * 7) % 36) as f32,
            vel: 0.8 + 0.2 * ((v % 3) as f32 / 2.0),
        })
        .collect()
}

/// Eight staggered notes, each held 1 s: voices start, release and die, then the reverb rings out.
#[must_use]
pub fn phrase() -> Vec<Note> {
    const PITCHES: [f32; 8] = [48.0, 55.0, 60.0, 64.0, 67.0, 72.0, 76.0, 79.0];
    PITCHES
        .iter()
        .enumerate()
        .map(|(i, &pitch)| Note {
            start: 3072 * i,
            len: 48_000,
            pitch,
            vel: 0.6 + 0.05 * i as f32,
        })
        .collect()
}

/// Four notes held 1 s then released: the long-tail performance (denormal and FTZ tests).
#[must_use]
pub fn tail_notes() -> Vec<Note> {
    [48.0, 55.0, 60.0, 64.0]
        .iter()
        .enumerate()
        .map(|(i, &pitch)| Note {
            start: i * 2400,
            len: 48_000,
            pitch,
            vel: 1.0,
        })
        .collect()
}

/// The P1-like patch with fast decays, so a naive (unflushed) engine reaches the subnormal range within
/// about 5 s instead of 20: the negative control of the FTZ and subnormal tests.
#[must_use]
pub fn fast_decay_patch() -> Patch {
    let mut p = Patch::p1_like();
    p.amp_env.release_ms = 40.0;
    p.filter_env.release_ms = 40.0;
    p.reverb.rt60_s = 0.5;
    p
}

/// One golden render case.
pub struct Case {
    pub name: &'static str,
    pub patch: Patch,
    pub notes: Vec<Note>,
    pub seconds: f32,
}

/// The render goldens. All are rendered with the scalar path; the lane paths are tested for equality.
#[must_use]
pub fn render_cases() -> Vec<Case> {
    let mut cases = vec![
        Case {
            name: "simple-1v-2s",
            patch: Patch::simple(),
            notes: held_chord(1, 2.0),
            seconds: 2.0,
        },
        Case {
            name: "simple-16v-2s",
            patch: Patch::simple(),
            notes: held_chord(16, 2.0),
            seconds: 2.0,
        },
        Case {
            name: "p1-8v-held-2s",
            patch: Patch::p1_like(),
            notes: held_chord(8, 2.0),
            seconds: 2.0,
        },
        Case {
            name: "p1-phrase-4s",
            patch: Patch::p1_like(),
            notes: phrase(),
            seconds: 4.0,
        },
        Case {
            name: "p1-tail-40s",
            patch: Patch::p1_like(),
            notes: tail_notes(),
            seconds: 40.0,
        },
        Case {
            name: "fast-decay-tail-12s",
            patch: fast_decay_patch(),
            notes: tail_notes(),
            seconds: 12.0,
        },
    ];
    for seed in 0..4u64 {
        let name = [
            "random-seed-0-phrase-2s",
            "random-seed-1-phrase-2s",
            "random-seed-2-phrase-2s",
            "random-seed-3-phrase-2s",
        ][seed as usize];
        cases.push(Case {
            name,
            patch: Patch::from_seed(seed),
            notes: phrase(),
            seconds: 2.0,
        });
    }
    cases
}

/// `render/<case> <sha256>` lines for every golden case (with the engine version in the header comment).
#[must_use]
pub fn render_golden_text() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# engine {ENGINE_VERSION}, patch {}; scalar path, Flush::On, seed {GOLDEN_SEED:#x}",
        patch::PATCH_VERSION
    );
    for c in render_cases() {
        let out = render(
            &c.patch,
            &c.notes,
            frames(c.seconds),
            GOLDEN_SEED,
            Path::Scalar,
            Flush::On,
        );
        let _ = writeln!(s, "render/{} {}", c.name, hash_samples(&out));
    }
    s
}

/// `patch/<seed> <sha256>` for seeds 0..[`PATCH_SEEDS`].
#[must_use]
pub fn patch_golden_text() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# patch version {}; Patch::from_seed(seed).hash()",
        patch::PATCH_VERSION
    );
    for seed in 0..PATCH_SEEDS {
        let _ = writeln!(s, "patch/{seed} {}", Patch::from_seed(seed).hash());
    }
    s
}
