//! Challenge v3's persisted forms, pinned (lane CV3, R4; docs/challenge.md, "Challenge v3").
//!
//! v3 is additive: a challenge is written at the lowest version that holds it, so every v1 and v2 fixture keeps its
//! bytes. `fixtures.rs` (lane CT, then CV2) proves they are still what this build writes and reads; this file pins the
//! files themselves by SHA-256, so no regenerate run can quietly rewrite them, and adds `challenge/v3/`: the cases that
//! need a v3 piece (an ear target with its choice answer). Regenerate the v3 fixtures with `cargo test -p
//! enth-challenge --test cv3_fixtures -- --ignored regenerate`, then read the diff.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::ear::code::EarChallengeCode;
use enth_challenge::ear::params::{EAR_CHECK, SPOT_THE_CHANGE};
use enth_challenge::ear::question;
use enth_challenge::ear::scorers::EarAnswer;
use enth_challenge::model::{AnswerKind, EarSpec, Start, Target};
use enth_challenge::spec::MAX_SEED;
use enth_challenge::{Challenge, ChallengeMeta, PhraseId, ScorerId};
use sha2::{Digest, Sha256};

/// The v1 and v2 challenge fixtures, as main `567e0df` (before Challenge v3) has them.
const V1_V2_SHA256: [(&str, &str); 6] = [
    (
        "v1/custom.json",
        "6f09bc7b0044f4a13b2593d0d839d5a8e62b4fcaded3f24cdbe17273f080a0e5",
    ),
    (
        "v1/level1.json",
        "5639bda41e54bdbf0db9bc98cc85833ababb9f2bd8cc7b6ee0d271f4d88d44ea",
    ),
    (
        "v1/level5.json",
        "5c0cb7e813039454e96dfe7d44c78481254303b755ba697040508ddfd486dbf2",
    ),
    (
        "v2/draw.json",
        "e35bb4d8606a8123ea8367bfdf08b19ba8645af3dc25080903d5247ca1d822c9",
    ),
    (
        "v2/patch.json",
        "57aff26da23617ac15eea5f521327eab126381178055b8e882dcbe375a418c88",
    ),
    (
        "v2/seed-editable.json",
        "e685b7941389def8394a181246426c276f9ccdb94c5958ea1dd9700c3afddbe9",
    ),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/challenge")
}

fn read(name: &str) -> String {
    let path = dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    })
}

fn ear(game: u8, seed: u64, count: u8) -> EarSpec {
    EarSpec {
        game,
        generator: 1,
        level: 1,
        seed,
        count,
    }
}

/// Challenge v3's cases: each has an ear target, so each writes `enth.challenge/3`.
fn challenges_v3() -> Vec<(&'static str, Challenge)> {
    let mut titled = Challenge::ear(ear(SPOT_THE_CHANGE, (1 << 40) - 1, 8)).unwrap();
    titled.meta = ChallengeMeta {
        title: "Spot the change, level 1".to_owned(),
        tags: vec!["ear".to_owned(), "fixture".to_owned()],
    };
    titled.scoring.pass_mark = 75;
    titled.assists.hint_budget = Some(0);
    vec![
        (
            "ear-spot",
            Challenge::ear(ear(SPOT_THE_CHANGE, 42, 16)).unwrap(),
        ),
        (
            "ear-check",
            Challenge::ear(ear(EAR_CHECK, MAX_SEED, 40)).unwrap(),
        ),
        ("ear-titled", titled),
    ]
}

#[test]
#[ignore = "writes the v3 fixtures: run on purpose and review the diff"]
fn regenerate() {
    let d = dir().join("v3");
    fs::create_dir_all(&d).unwrap();
    for (name, c) in challenges_v3() {
        fs::write(
            d.join(format!("{name}.json")),
            format!("{}\n", c.to_json_pretty()),
        )
        .unwrap();
    }
}

/// The acceptance's first line: v1 and v2 fixtures keep their bytes (and `fixtures.rs` reads and writes them).
#[test]
fn the_v1_and_v2_fixture_files_keep_their_bytes() {
    for (name, sha) in V1_V2_SHA256 {
        assert_eq!(
            hex(&Sha256::digest(read(name).as_bytes())),
            sha,
            "{name} changed"
        );
    }
}

/// Every v1 and v2 fixture still reads, and is written back byte for byte at its own version, through both readers.
#[test]
fn the_v1_and_v2_fixtures_read_and_write_as_before() {
    for (name, _) in V1_V2_SHA256 {
        let text = read(name);
        let c = Challenge::from_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let version = if name.starts_with("v1/") { 1 } else { 2 };
        assert_eq!(c.version(), version, "{name}");
        assert_eq!(c.to_json_pretty() + "\n", text, "{name}");
        assert_eq!(
            serde_json::from_str::<Challenge>(&text).unwrap(),
            c,
            "{name}"
        );
    }
}

#[test]
fn challenge_v3_fixtures_are_what_this_build_writes_and_reads() {
    for (name, c) in challenges_v3() {
        let text = read(&format!("v3/{name}.json"));
        assert_eq!(c.version(), 3, "{name}");
        assert!(text.contains("\"enth.challenge/3\""), "{name}");
        assert_eq!(text, format!("{}\n", c.to_json_pretty()), "{name}");
        assert_eq!(Challenge::from_json(&text), Ok(c.clone()), "{name}");
        assert_eq!(
            serde_json::from_str::<Challenge>(&text).unwrap(),
            c,
            "{name}"
        );
        assert_eq!(Challenge::from_json(&c.to_json()), Ok(c.clone()), "{name}");
        assert!(matches!(c.target, Target::Ear(_)), "{name}");
        assert_eq!(c.answer, AnswerKind::Choice, "{name}");
        assert_eq!(c.phrase, PhraseId(0), "{name}");
        assert_eq!(c.start, Start::Template { template: 0 }, "{name}");
        assert_eq!(c.scoring.sound, ScorerId::EAR_V1, "{name}");
        // Older readers refuse it: no v1 or v2 reader sees a v3 piece.
        for older in ["enth.challenge/2", "enth.challenge/1"] {
            assert_eq!(
                Challenge::from_json(&text.replacen("enth.challenge/3", older, 1))
                    .unwrap_err()
                    .code(),
                "bad_challenge",
                "{name} as {older}"
            );
        }
    }
}

/// Every v3 fixture prepares its question set with no render, and its oracle scores 100 % while no answers fail.
#[test]
fn challenge_v3_fixtures_prepare_and_judge_with_no_render() {
    for (name, c) in challenges_v3() {
        let set = question::prepare(&c).unwrap_or_else(|e| panic!("{name}: {e}"));
        let count = usize::from(c.ear_spec().unwrap().count);
        assert_eq!(set.questions().len(), count, "{name}");
        let oracle: Vec<Option<EarAnswer>> =
            set.keys().iter().map(|k| Some(k.right_answer())).collect();
        let score = set.judge(&oracle).unwrap();
        assert_eq!(score.percent, 100.0, "{name}");
        assert!(score.passed, "{name}");
        let none = set.judge(&vec![None; count]).unwrap();
        assert_eq!((none.right, none.passed), (0, false), "{name}");
    }
}

/// A fixture's spec, when its seed fits a code, travels as the ear-challenge code and comes back as the same game.
#[test]
fn the_fixtures_ear_codes_round_trip() {
    for (name, c) in challenges_v3() {
        let spec = *c.ear_spec().unwrap();
        match EarChallengeCode::new(spec) {
            Some(code) => {
                let back = EarChallengeCode::decode(&code.encode()).unwrap();
                assert_eq!(back.spec(), &spec, "{name}");
                assert_eq!(
                    back.challenge().unwrap().target,
                    c.target,
                    "{name}: the code carries the game"
                );
            }
            None => assert!(spec.seed >= 1 << 40, "{name}: only a long seed has no code"),
        }
    }
}

/// No float in a v3 challenge: its own fields stay integers, as v1's and v2's do.
#[test]
fn a_v3_challenge_holds_integers_only() {
    fn floats(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Number(n) => n.is_f64(),
            serde_json::Value::Array(a) => a.iter().any(floats),
            serde_json::Value::Object(o) => o.values().any(floats),
            _ => false,
        }
    }
    for (name, c) in challenges_v3() {
        let v: serde_json::Value = serde_json::from_str(&c.to_json()).unwrap();
        assert!(!floats(&v), "{name}");
    }
}
