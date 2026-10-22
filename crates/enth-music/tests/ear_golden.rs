//! Ear generator v1's golden (lane MUS; docs/ear.md, "The ear generator"; docs/phase-4.md, Acceptance): 1,000
//! `EarSpec`s to one hash, the same on every platform. The file `golden/ear-v1.txt` is compiled in, so the wasm run
//! needs no directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-music --target wasm32-wasip1 --test ear_golden -- --nocapture
//! ```
//!
//! `golden.yml` runs it on Linux (x86-64 and aarch64), Windows and wasm32-wasip1 under wasmtime, and `cargo test`
//! runs it everywhere else.
//!
//! **What a line holds.** The `i`-th spec cycles through every (game, level) pair of this build in order (so a spec of
//! each game and level is always in the file), with a 53-bit seed from a `SplitMix64` stream of a fixed root, and a
//! count of 1 to 20 (every tenth is the maximum, 64). The line prints the spec and the first 16 hex digits of the
//! SHA-256 of the canonical bytes ([`Question::write_canonical`]) of all its questions; the last line is the SHA-256
//! of all 1,000 full digests.
//!
//! **Not yet frozen.** Ear generator v1 freezes at INT4's friend build, the first that shares ear codes
//! (docs/phase-4.md, "Open checks"). Until then the ladder lanes (G1 to G3, TAP) change this file **on purpose**
//! whenever they add or tune a level: adding a level changes every line (the pairs cycle), and the commit names the
//! games and why. After the freeze a change here is a new generator version with its own file, never an update.
//!
//! Regenerate (only for the reasons above, and review the diff):
//! `cargo test -p enth-music --test ear_golden -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]
#![allow(
    clippy::disallowed_methods,
    reason = "only the ignored `regenerate` test writes the golden file; a test, not library code"
)]

use enth_music::ear::{EarSpec, Game, MAX_COUNT, MAX_SEED};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const EXPECTED: &str = include_str!("golden/ear-v1.txt");

/// The seed every golden spec's seed derives from (`EAR_v1` in ASCII).
const ROOT: u64 = 0x0000_4541_525F_7631;

/// The `i`-th golden seed: one `SplitMix64` output (Vigna's public-domain constants) over `ROOT + i·γ`, kept to its
/// top 53 bits.
const fn seed_of(i: u64) -> u64 {
    let mut z = ROOT.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

const _: () = assert!(seed_of(0) <= MAX_SEED && seed_of(999) <= MAX_SEED);

/// Every (game, level) of this build, games in id order, levels ascending.
fn pairs() -> Vec<(Game, u8)> {
    Game::ALL
        .iter()
        .flat_map(|g| (1..=g.max_level()).map(move |l| (*g, l)))
        .collect()
}

/// The `i`-th golden spec.
fn spec(i: u64) -> EarSpec {
    let pairs = pairs();
    let (game, level) = pairs[(i % pairs.len() as u64) as usize];
    let count = if i % 10 == 9 {
        MAX_COUNT
    } else {
        1 + ((i * 7) % 20) as u8
    };
    EarSpec::new(game, level, seed_of(i), count)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    })
}

/// The golden file's text, computed.
fn compute() -> String {
    let mut out = String::from(
        "# enth-music ear generator v1: 1,000 EarSpec -> question-set digest golden (crates/enth-music/tests/ear_golden.rs).\n\
         # index seed game level count d=sha256(canonical bytes of the set)[..16]; last line: sha256 of the 1,000 full digests\n",
    );
    let mut all = Sha256::new();
    for i in 0..1_000u64 {
        let s = spec(i);
        let mut bytes = Vec::new();
        for q in s.questions().expect("a golden spec is valid") {
            q.validate().expect("a golden question is valid");
            q.write_canonical(&mut bytes);
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        all.update(digest);
        let _ = writeln!(
            out,
            "{i:04} {:014x} g={:02} l={:02} n={:02} d={}",
            s.seed,
            s.game,
            s.level,
            s.count,
            &hex(&digest)[..16]
        );
    }
    let digest: [u8; 32] = all.finalize().into();
    let _ = writeln!(out, "all {}", hex(&digest));
    out
}

#[test]
fn ear_generator_v1_matches_its_golden() {
    let got = compute();
    println!("ear generator v1: {}", got.lines().last().unwrap());
    // A Windows checkout may turn the file's LF into CRLF (.gitattributes pins LF; this keeps the test honest anyway).
    let expected = EXPECTED.replace("\r\n", "\n");
    if got == expected {
        return;
    }
    let diffs: Vec<String> = got
        .lines()
        .zip(expected.lines())
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("  want {b}\n  got  {a}"))
        .collect();
    panic!(
        "{} of {} golden lines differ (a v1 question changed: until INT4's friend build freezes the generator, \
         regenerate on purpose and say which games and why; after it, that is ear generator v2):\n{}",
        diffs.len(),
        expected.lines().count(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_golden_covers_every_game_and_level() {
    let specs: Vec<EarSpec> = (0..1_000).map(spec).collect();
    for (game, level) in pairs() {
        assert!(
            specs
                .iter()
                .any(|s| s.game == game.id() && s.level == level),
            "{game:?} level {level}"
        );
    }
    assert!(specs.iter().any(|s| s.count == MAX_COUNT));
    assert!(specs.iter().all(|s| s.validate().is_ok()));
}

#[test]
#[ignore = "writes the golden file; run on purpose"]
fn regenerate() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/ear-v1.txt");
    std::fs::write(path, compute()).expect("write the golden");
    println!("wrote {path}");
}
