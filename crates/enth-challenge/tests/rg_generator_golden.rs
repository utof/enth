//! Generator v1's golden (lane RG; design.md §9, docs/phase-2.md "Target determinism"): 1,000 (spec → target
//! hash) pairs, the same on every platform. A spec here is a seed, a difficulty, a `musical` amount and sometimes a
//! per-category amount; the target hash is `enth_patch::canon::exact_hash` of the generated patch (its first 16 hex
//! digits on each line), with the phrase id beside it, and the last line is the SHA-256 of all 1,000 full hashes.
//!
//! **Frozen with generator v1.** A change to a prior, a rule, the slot table, the phrase table or the frozen starter
//! changes these hashes: that is a new generator version, never an update of this file. The file changes only when
//! a v1 target was wrong by its own documented rules (say so in the commit), or when `exact_hash`'s encoding gets a
//! new magic.
//!
//! Native and wasm32 must agree (the file is compiled in, so no directory needs opening):
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER=/path/to/wasmtime \
//!   cargo test -p enth-challenge --target wasm32-wasip1 --test rg_generator_golden -- --nocapture
//! ```
//!
//! Regenerate (only for the reasons above, and review the diff):
//! `cargo test -p enth-challenge --test rg_generator_golden -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary hash for the native-vs-wasm comparison"
)]

use enth_challenge::random::{self, Generated};
use enth_challenge::{Category, RandomSpec};
use enth_dsp::util::rng::derive;
use enth_patch::canon::exact_hash;
use sha2::{Digest, Sha256};

const EXPECTED: &str = include_str!("golden/rg-generator-v1.txt");

/// The seed every golden spec's seed derives from ("RG_v1").
const ROOT: u64 = 0x5247_5F76_31;

/// The `i`-th golden spec: difficulties spread over every level and both ends, `musical` at 0, 20, 21, 50 and 100,
/// and every ninth and thirteenth spec with one amount set (every 13th to 0).
fn spec(i: u64) -> RandomSpec {
    let mut amounts = [None; Category::COUNT];
    if i % 9 == 4 {
        amounts[((i / 9) % 6) as usize] = Some(((i * 53) % 101) as u8);
    }
    if i % 13 == 6 {
        amounts[((i / 13) % 6) as usize] = Some(0);
    }
    RandomSpec {
        generator: random::GENERATOR_V1,
        seed: derive(ROOT, i),
        difficulty: ((i * 37) % 101) as u8,
        amounts,
        musical: [100, 100, 0, 50, 100, 21, 20, 100][(i % 8) as usize],
        template: random::TEMPLATE_STARTER,
    }
}

fn line(i: u64, s: &RandomSpec, g: &Generated, hash: &str) -> String {
    let amounts: Vec<String> = s
        .amounts
        .iter()
        .map(|a| a.map_or("-".to_owned(), |a| a.to_string()))
        .collect();
    format!(
        "{i:04} {:016x} d={:03} m={:03} a={} p={} {}",
        s.seed,
        s.difficulty,
        s.musical,
        amounts.join(","),
        g.phrase.as_u8(),
        &hash[..16]
    )
}

/// The golden file's text, computed.
fn compute() -> String {
    let mut out = String::from(
        "# enth-challenge generator v1: 1,000 RandomSpec -> target golden (tests/rg_generator_golden.rs).\n\
         # index seed d=difficulty m=musical a=amounts per category (osc,filter,env,mod,fx,structure) p=phrase \
         exact_hash[..16]\n",
    );
    let mut all = Sha256::new();
    for i in 0..1_000 {
        let s = spec(i);
        let g = random::target(&s).unwrap();
        let hash = exact_hash(&g.patch).to_hex();
        all.update(hash.as_bytes());
        out.push_str(&line(i, &s, &g, &hash));
        out.push('\n');
    }
    let digest: [u8; 32] = all.finalize().into();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    out.push_str(&format!("all {hex}\n"));
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
    let mut phrases = [0usize; 3];
    let (mut amounts, mut uniform) = (0, 0);
    for i in 0..1_000 {
        let s = spec(i);
        levels[usize::from(random::level(s.difficulty))] += 1;
        let g = random::target(&s).unwrap();
        phrases[usize::from(g.phrase.as_u8())] += 1;
        amounts += usize::from(s.amounts.iter().any(Option::is_some));
        uniform += usize::from(s.musical == 0);
    }
    assert!(levels[1..].iter().all(|n| *n > 150), "{levels:?}");
    assert!(phrases.iter().all(|n| *n > 20), "{phrases:?}");
    assert!(amounts > 150 && uniform > 100, "{amounts} {uniform}");
}

#[test]
#[ignore = "rewrites tests/golden/rg-generator-v1.txt; run by hand, for the reasons in the file's docs"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the golden file; the library does no I/O"
)]
fn regenerate() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/rg-generator-v1.txt");
    std::fs::write(path, compute()).unwrap();
}
