//! Ear generator v1's golden (lane MUS; docs/ear.md, "The ear generator"; docs/phase-4.md, Acceptance): question-set
//! digests of fixed specs, **one block per (game, level)**, the same on every platform. The file `golden/ear-v1.txt` is
//! compiled in, so the wasm run needs no directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-music --target wasm32-wasip1 --test ear_golden -- --nocapture
//! ```
//!
//! `golden.yml` runs it on Linux (x86-64 and aarch64), Windows and wasm32-wasip1 under wasmtime, and `cargo test`
//! runs it everywhere else.
//!
//! **What a block holds** (the F1e review's should-fix S4, lane F1E). For each (game, level) of this build, games in
//! id order and levels ascending: a `block` line, then [`SPECS`] lines, the `j`-th holding a spec of that game and level
//! with a 53-bit seed from a `SplitMix64` stream keyed by (game, level, `j`) and a count of 1 to 20 (every tenth is
//! the maximum, 64), and the first 16 hex digits of the SHA-256 of the canonical bytes ([`Question::write_canonical`])
//! of all its questions; then a `sum` line, the SHA-256 of the block's full digests. A spec depends only on its game,
//! level and `j`, so **a new level or game appends a block and no existing line changes**: a ladder lane's commit
//! shows exactly the games it added or tuned (MUS first cycled 1,000 specs through every pair, so adding a level
//! rewrote every line). There is no whole-file line, for the same reason; the test prints a hash of the block sums for
//! the native-against-wasm comparison.
//!
//! **Not yet frozen.** Ear generator v1 freezes at INT4's friend build, the first that shares ear codes
//! (docs/phase-4.md, "Open checks"). Until then the ladder lanes (G1 to G3, TAP) change this file **on purpose**: adding
//! a level appends its block, tuning one changes that block alone, and the commit names the games and why. After the
//! freeze a change to an existing block is a new generator version with its own file, never an update.
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

/// Specs per (game, level) block.
const SPECS: u64 = 150;

/// The golden seed of spec `j` of `game` at `level`: one `SplitMix64` output (Vigna's public-domain constants) over
/// `ROOT + key·γ`, the key packing (game, level, `j`), kept to its top 53 bits. It depends on nothing else, so a new
/// block never moves another's seeds.
const fn seed_of(game: u8, level: u8, j: u64) -> u64 {
    let key = ((game as u64) << 40) | ((level as u64) << 32) | j;
    let mut z = ROOT.wrapping_add(key.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

const _: () = assert!(seed_of(255, 15, SPECS) <= MAX_SEED && seed_of(4, 1, 0) <= MAX_SEED);

/// Every (game, level) of this build, games in id order, levels ascending: the blocks, in file order.
fn pairs() -> Vec<(Game, u8)> {
    Game::ALL
        .iter()
        .flat_map(|g| (1..=g.max_level()).map(move |l| (*g, l)))
        .collect()
}

/// Spec `j` of the block of `game` at `level`.
fn spec(game: Game, level: u8, j: u64) -> EarSpec {
    let count = if j % 10 == 9 {
        MAX_COUNT
    } else {
        1 + ((j * 7) % 20) as u8
    };
    EarSpec::new(game, level, seed_of(game.id(), level, j), count)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    })
}

/// The golden file's text, computed, and the hash of the block sums (printed, never stored).
fn compute() -> (String, String) {
    let mut out = String::from(
        "# enth-music ear generator v1: question-set digests, one block per (game, level) (crates/enth-music/tests/ear_golden.rs).\n\
         # block g l; then j seed count d=sha256(canonical bytes of the set)[..16]; then sum g l sha256(the block's full digests).\n\
         # A new level or game appends a block; no existing line changes.\n",
    );
    let mut sums = Sha256::new();
    for (game, level) in pairs() {
        let _ = writeln!(out, "block g={:02} l={level:02}", game.id());
        let mut block = Sha256::new();
        for j in 0..SPECS {
            let s = spec(game, level, j);
            let mut bytes = Vec::new();
            for q in s.questions().expect("a golden spec is valid") {
                q.validate().expect("a golden question is valid");
                q.write_canonical(&mut bytes);
            }
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            block.update(digest);
            let _ = writeln!(
                out,
                "{j:03} {:014x} n={:02} d={}",
                s.seed,
                s.count,
                &hex(&digest)[..16]
            );
        }
        let sum: [u8; 32] = block.finalize().into();
        sums.update(sum);
        let _ = writeln!(out, "sum g={:02} l={level:02} {}", game.id(), hex(&sum));
    }
    let all: [u8; 32] = sums.finalize().into();
    (out, hex(&all))
}

#[test]
fn ear_generator_v1_matches_its_golden() {
    let (got, all) = compute();
    println!("ear generator v1: blocks {all}");
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
    let (text, _) = compute();
    for (game, level) in pairs() {
        assert!(
            text.contains(&format!("block g={:02} l={level:02}\n", game.id())),
            "{game:?} level {level}"
        );
        let specs: Vec<EarSpec> = (0..SPECS).map(|j| spec(game, level, j)).collect();
        assert!(specs.iter().any(|s| s.count == MAX_COUNT));
        assert!(specs.iter().all(|s| s.validate_music().is_ok()));
    }
    // A block's specs depend on its game, level and index alone: the same spec whatever else the build has.
    assert_eq!(
        spec(Game::IntervalStacks, 1, 7),
        spec(Game::IntervalStacks, 1, 7)
    );
    assert_ne!(
        spec(Game::IntervalStacks, 1, 7).seed,
        spec(Game::IntervalStacks, 2, 7).seed
    );
}

#[test]
#[ignore = "writes the golden file; run on purpose"]
fn regenerate() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/ear-v1.txt");
    std::fs::write(path, compute().0).expect("write the golden");
    println!("wrote {path}");
}
