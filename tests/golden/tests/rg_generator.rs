//! The Recreate generator's golden, version 1 (lane RG; design.md §9, docs/phase-2.md "Target determinism",
//! docs/challenge.md "Generator versions are frozen"): 1,000 (spec → target hash) pairs, the same on every platform.
//! It lives in `enth-golden` so that `golden.yml` runs it on Linux and Windows (debug and release) and under wasmtime
//! on every push, and on macOS weekly, with no job of its own.
//!
//! A spec here is a seed (53 bits, a spec's limit), a difficulty, a `musical` amount and sometimes a per-category
//! amount, from the starter template. Each line holds the spec, the phrase the generator picked and the first 16 hex
//! digits of the target's **fingerprint** (`enth_challenge::fingerprint`: the generator, the phrase and every draw by
//! name and value, which is what a long code's prefix and a saved round check); the last line is the SHA-256 of all
//! 1,000 full fingerprints.
//!
//! **Why the fingerprint, not a patch hash** (F1r review M1, 2026-10-20). The first version of this file pinned
//! `canonical_hash` and `exact_hash`, whose bytes hold the patch schema number and every value by position: the first
//! schema bump or new catalogue parameter would have moved all 1,000 lines without any v1 target changing. The
//! fingerprint holds neither, so this file moves only when a v1 target does. It was regenerated once, on purpose,
//! for that switch (lane F1R; the seeds went from 64 to 53 bits in the same change, F1r review S4).
//!
//! **Frozen with generator v1.** A change to a prior, a rule, the slot table, the phrase table, the frozen starter or
//! a catalogue field v1 reads (`random::tests::the_catalogue_fields_v1_reads_are_pinned`) changes these
//! fingerprints: that is a new generator version (a new file beside this one, and `LATEST_GENERATOR` bumped in the
//! same commit), never an update of this file. It changes only when a v1 target broke its own documented rules (say
//! so in the commit), or when the fingerprint format gets a new magic.
//!
//! The file is compiled in, so the wasm run needs no directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-golden --target wasm32-wasip1 --test rg_generator -- --nocapture
//! ```
//!
//! Regenerate (only for the reasons above, and review the diff):
//! `cargo test -p enth-golden --test rg_generator -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]

use enth_challenge::random::{self, Generated};
use enth_challenge::spec::{MAX_SEED, PRESETS};
use enth_challenge::{Category, RandomSpec};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

const EXPECTED: &str = include_str!("../generator/v1.txt");

/// The seed every golden spec's seed derives from (`RG_v1` in ASCII).
const ROOT: u64 = 0x0052_475F_7631;

/// The `i`-th golden seed: one `SplitMix64` output (Vigna's public-domain constants) over `ROOT + i·γ`, kept to its
/// top 53 bits (a spec's seed is at most `MAX_SEED`).
const fn seed_of(i: u64) -> u64 {
    let mut z = ROOT.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

const _: () = assert!(seed_of(0) <= MAX_SEED && seed_of(999) <= MAX_SEED);

/// The `i`-th golden spec. Every fourth is one of code v1's presets (the five levels, cycling) with a 40-bit seed,
/// exactly what a short code can carry, which has no hash prefix and relies on this golden. The rest spread the
/// difficulty over every value, `musical` over 0, 20, 21, 50 and 100, and set one amount in some (to 0 in some).
fn spec(i: u64) -> RandomSpec {
    if i.is_multiple_of(4) {
        return PRESETS[((i / 4) % 5) as usize].spec(random::GENERATOR_V1, seed_of(i) >> (53 - 40));
    }
    let mut amounts = [None; Category::COUNT];
    if i % 9 == 2 {
        amounts[((i / 9) % 6) as usize] = Some(((i * 53) % 101) as u8);
    }
    if i % 13 == 6 {
        amounts[((i / 13) % 6) as usize] = Some(0);
    }
    RandomSpec {
        generator: random::GENERATOR_V1,
        seed: seed_of(i),
        difficulty: ((i * 37) % 101) as u8,
        amounts,
        musical: [100, 0, 50, 100, 21, 20, 100][(i % 7) as usize],
        template: random::TEMPLATE_STARTER,
    }
}

fn line(i: u64, s: &RandomSpec, g: &Generated, fingerprint: &str) -> String {
    let amounts: Vec<String> = s
        .amounts
        .iter()
        .map(|a| a.map_or("-".to_owned(), |a| a.to_string()))
        .collect();
    format!(
        "{i:04} {:014x} d={:03} m={:03} a={} p={} f={}",
        s.seed,
        s.difficulty,
        s.musical,
        amounts.join(","),
        g.phrase.0,
        &fingerprint[..16]
    )
}

/// The golden file's text, computed.
fn compute() -> String {
    let mut out = String::from(
        "# enth-challenge generator v1: 1,000 RandomSpec -> target fingerprint golden (tests/golden/tests/rg_generator.rs).\n\
         # index seed d=difficulty m=musical a=amounts (osc,filter,env,mod,fx,structure) p=phrase \
         f=fingerprint[..16]; last line: sha256 of the 1,000 full fingerprints\n",
    );
    let mut all = Sha256::new();
    for i in 0..1_000 {
        let s = spec(i);
        let g = random::target(&s).unwrap();
        let fingerprint = g.fingerprint.to_hex();
        all.update(fingerprint.as_bytes());
        out.push_str(&line(i, &s, &g, &fingerprint));
        out.push('\n');
    }
    let digest: [u8; 32] = all.finalize().into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    let _ = writeln!(out, "all {hex}");
    out
}

#[test]
fn generator_v1_matches_its_golden() {
    let got = compute();
    println!("rg generator v1: {}", got.lines().last().unwrap());
    if got == EXPECTED {
        return;
    }
    let diffs: Vec<String> = got
        .lines()
        .zip(EXPECTED.lines())
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("  want {b}\n  got  {a}"))
        .collect();
    panic!(
        "{} of {} golden lines differ (a v1 target changed: that is a new generator version, see the file's \
         docs):\n{}",
        diffs.len(),
        EXPECTED.lines().count(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The golden covers what it claims: every level, both priors, the amounts, every phrase.
#[test]
fn the_golden_specs_cover_the_generator() {
    let mut levels = [0usize; 6];
    let mut phrases = [0usize; 4];
    let (mut amounts, mut uniform, mut presets) = (0, 0, 0);
    for i in 0..1_000 {
        let s = spec(i);
        levels[usize::from(random::level(s.difficulty))] += 1;
        let g = random::target(&s).unwrap();
        phrases[usize::from(g.phrase.0)] += 1;
        amounts += usize::from(s.amounts.iter().any(Option::is_some));
        uniform += usize::from(s.musical == 0);
        presets += usize::from(s.preset().is_some());
    }
    assert!(levels[1..].iter().all(|n| *n > 150), "{levels:?}");
    assert!(
        phrases[0] == 0 && phrases[1..].iter().all(|n| *n > 20),
        "{phrases:?}"
    );
    assert!(
        amounts > 100 && uniform > 80 && presets >= 250,
        "{amounts} {uniform} {presets}"
    );
}

#[test]
#[ignore = "rewrites tests/golden/generator/v1.txt; run by hand, for the reasons in the file's docs"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the golden file; the library does no I/O"
)]
fn regenerate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("generator/v1.txt");
    std::fs::write(path, compute()).unwrap();
}
