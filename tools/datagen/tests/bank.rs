//! Lane TP: `enth-datagen bank`, the k-NN bank builder. The bank is the same bytes for any thread count, `bank check`
//! re-asserts the GR3 exclusions on a finished file, and a bank holding a GR3 seed or target fingerprint is refused.
#![allow(
    clippy::disallowed_methods,
    reason = "the test runs the tool's own binary and reads the files it writes"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use enth_challenge::random;
use enth_challenge::spec::PRESETS;
use enth_solver::nearest::{
    BANK_FORMAT, Bank, BankEntry, BankHeader, DIMS, EMBEDDING_VERSION, Embedding, UNPITCHED,
};
use sha2::{Digest, Sha256};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_enth-datagen"))
        .args(args)
        .output()
        .expect("the tool runs")
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bank-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn gr3() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/gr3")
}

fn ok(out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_bank_is_the_same_for_any_thread_count_and_passes_its_check() {
    let (a, b) = (scratch("t1.bin"), scratch("t3.bin"));
    ok(&run(&[
        "bank",
        "--entries",
        "6",
        "--threads",
        "1",
        "--out",
        a.to_str().unwrap(),
    ]));
    ok(&run(&[
        "bank",
        "--entries",
        "6",
        "--threads",
        "3",
        "--out",
        b.to_str().unwrap(),
    ]));
    let (x, y) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    assert_eq!(x, y, "thread count changed the bank");
    let bank = Bank::parse(&x).unwrap();
    assert_eq!(bank.len(), 6);
    assert_eq!(bank.header().embedding, EMBEDDING_VERSION);
    // Levels 2-5 only, every seed short-code sized and outside GR3's block.
    for e in bank.entries() {
        assert!([25, 50, 75, 100].contains(&e.difficulty), "{e:?}");
        assert!(e.seed < 1 << 40);
        assert!(!(770_000_000_000..=770_999_999_999).contains(&e.seed));
    }
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(a.with_extension("json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "enth.bank-manifest/1");
    assert_eq!(manifest["entries"], 6);
    assert_eq!(
        manifest["sha256"],
        enth_solver::create::record::hex(&Sha256::digest(&x))
    );
    assert!(
        manifest["exclusions"]["entries"]["fingerprint"]
            .as_u64()
            .unwrap()
            > 100
    );
    ok(&run(&["bank", "check", a.to_str().unwrap()]));
}

/// The SHA-256 the builder records for the three lists.
fn lists_sha256() -> [u8; 32] {
    let mut h = Sha256::new();
    for name in [
        "exclude-synth.txt",
        "exclude-real.txt",
        "exclude-degraded.txt",
    ] {
        h.update(std::fs::read(gr3().join(name)).unwrap());
    }
    h.finalize().into()
}

fn entry_for(seed: u64, preset: usize) -> BankEntry {
    let spec = PRESETS[preset].spec(1, seed);
    let g = random::target(&spec).unwrap();
    BankEntry {
        seed,
        difficulty: spec.difficulty,
        musical: spec.musical,
        pitch_cents: UNPITCHED,
        fingerprint_prefix: g.fingerprint.prefix().0,
        embedding: Embedding { codes: [60; DIMS] },
    }
}

fn header(exclusions_sha256: [u8; 32]) -> BankHeader {
    BankHeader {
        format: BANK_FORMAT,
        generator: 1,
        template: 0,
        embedding: EMBEDDING_VERSION,
        note: 48,
        velocity: 0.8,
        hold_s: 1.0,
        duration_s: 2.0,
        sample_rate: 48_000,
        render_seed: 1,
        exclusions_sha256,
    }
}

fn check_refuses(entry: BankEntry, why: &str, file: &str) {
    let path = scratch(file);
    std::fs::write(
        &path,
        Bank::new(header(lists_sha256()), vec![entry]).to_bytes(),
    )
    .unwrap();
    let out = run(&["bank", "check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(why), "{err}");
}

#[test]
fn a_bank_holding_a_gr3_seed_or_target_is_refused() {
    // A GR3 test seed (level 2's first): inside the listed seed range.
    check_refuses(entry_for(770_000_020_001, 1), "seed-range", "seed.bin");
    // Level 1 has 14 distinct targets, all GR3 targets: some seed outside the block regenerates one of them.
    let listed = std::fs::read_to_string(gr3().join("exclude-synth.txt")).unwrap();
    // (Its code must not be listed itself: the list also names GR1's and the goldens' codes.)
    let seed = (1..2_000u64)
        .find(|&s| {
            let spec = PRESETS[0].spec(1, s);
            let g = random::target(&spec).unwrap();
            let code = enth_challenge::ChallengeCode::short(spec).unwrap().encode();
            listed.contains(&g.fingerprint.to_hex()) && !listed.contains(&code)
        })
        .expect("a level-1 seed regenerates a GR3 target");
    check_refuses(entry_for(seed, 0), "fingerprint", "fingerprint.bin");
}

#[test]
fn a_bank_built_against_other_lists_is_refused() {
    let path = scratch("stale.bin");
    std::fs::write(
        &path,
        Bank::new(header([9; 32]), vec![entry_for(12_345, 4)]).to_bytes(),
    )
    .unwrap();
    let out = run(&["bank", "check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("other exclusion lists"));
}
