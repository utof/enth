//! Five golden challenges (lane SKR, freeze point F2r; lane GC grows them to about 24 under `tests/golden/challenges/`
//! with the real scorers' columns): one per level, each pinned as its **code**, then its **target fingerprint**, then
//! the **stub scores** of three fixed answers (docs/phase-2.md, "Golden challenges"). It lives in `enth-golden`, so
//! `golden.yml` runs it on Linux and Windows (debug and release) and under wasmtime on every push, and on macOS
//! weekly.
//!
//! - The code is the short code of a level's preset with a fixed seed: code v1 is frozen, so its text never changes.
//! - The fingerprint is the regenerated target's (generator v1, frozen): it moves only if a v1 target moves.
//! - The scores are `enth_challenge::score_round` with the challenge's scorers named explicitly as the stubs
//!   (`sound-stub/1`, `structure-stub/1`, `combined-stub/1`; parameter space, no render), so they stay pinned when
//!   lanes SS and ST switch this build's defaults; their scorers add columns of their own (scorers are additive,
//!   design.md §6). Each to 1e-6, with pass or fail, and a margin of at least 1e-3 to the pass mark, so no golden
//!   sits where a last-bit difference flips it.
//!
//! The answers: `exact` (the target), `start` (the template's start patch, what a round begins with) and `near` (the
//! start with every randomized parameter but the last set to the target's: one knob away).
//!
//! The file is compiled in, so the wasm run needs no directory. Regenerate only on purpose (a new answer or case;
//! review the diff): `cargo test -p enth-golden --test skr_challenges -- --ignored regenerate`.

#![allow(
    clippy::print_stdout,
    reason = "prints the summary for the native-vs-wasm comparison"
)]

use std::fmt::Write as _;

use enth_challenge::model::ScoringSpec;
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_challenge::{
    Challenge, ChallengeCode, PreparedRound, ScoreReport, ScorerId, prepare_challenge, score_round,
};
use enth_patch::{Op, Patch};

const EXPECTED: &str = include_str!("../challenges/skr-v1.txt");

/// The five cases: (level, seed).
const CASES: [(u8, u64); 5] = [(1, 1), (2, 22), (3, 333), (4, 4_444), (5, 55_555)];

/// The smallest distance a pinned sound % may have to the pass mark.
const MARGIN: f64 = 1e-3;

/// The challenge of a case, scored by the stubs (named, whatever this build's defaults are).
fn round(level: u8, seed: u64) -> (ChallengeCode, PreparedRound) {
    let spec = SpecPreset::by_id(level)
        .unwrap()
        .spec(LATEST_GENERATOR, seed);
    let code = ChallengeCode::short(spec).unwrap();
    let generated = enth_challenge::random::target(&spec).unwrap();
    let mut challenge = Challenge::recreate(spec, generated.phrase);
    challenge.scoring = ScoringSpec::with_scorers(
        spec.difficulty,
        ScorerId::SOUND_STUB,
        ScorerId::STRUCTURE_STUB,
        ScorerId::COMBINED_STUB,
        ScorerId::TIME_V1,
    );
    (code, prepare_challenge(challenge).unwrap())
}

/// The three fixed answers.
fn answers(round: &PreparedRound) -> [(&'static str, Patch); 3] {
    let mut near = round.start().clone();
    let params = round.params();
    for addr in &params[..params.len().saturating_sub(1)] {
        let value = round.target().value(addr).unwrap();
        enth_patch::apply(
            &mut near,
            Op::SetParam {
                addr: addr.clone(),
                value,
            },
        )
        .unwrap();
    }
    [
        ("exact", round.target().clone()),
        ("start", round.start().clone()),
        ("near", near),
    ]
}

fn line(name: &str, r: &ScoreReport) -> String {
    format!(
        "  {name:<5} sound={:.6} structure={:.6} combined={:.6} pass={}",
        r.sound.percent, r.structure.percent, r.combined.percent, r.passed
    )
}

/// The golden file's text, computed.
fn compute() -> String {
    let mut out = String::from(
        "# enth golden challenges, lane SKR's five (tests/golden/tests/skr_challenges.rs): per case the short code,\n\
         # the level and seed, the phrase and the target fingerprint, then the stub scores of three answers.\n",
    );
    for (level, seed) in CASES {
        let (code, round) = round(level, seed);
        let _ = writeln!(
            out,
            "{code} level={level} seed={seed} phrase={} f={}",
            round.challenge().phrase.0,
            round.fingerprint().to_hex()
        );
        for (name, answer) in answers(&round) {
            let report = score_round(&round, &answer).unwrap();
            out.push_str(&line(name, &report));
            out.push('\n');
        }
    }
    out
}

#[test]
fn the_five_golden_challenges_match() {
    let got = compute();
    println!("{got}");
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
        "{} of {} golden challenge lines differ:\n{}",
        diffs.len(),
        EXPECTED.lines().count(),
        diffs.join("\n")
    );
}

/// The pins hold what they claim: the codes decode to their settings, the target scores 100 and passes, no sound %
/// sits within the margin of the pass mark, and every level is covered.
#[test]
fn the_golden_challenges_are_well_formed() {
    let mut levels = Vec::new();
    for (level, seed) in CASES {
        let (code, round) = round(level, seed);
        let decoded = ChallengeCode::decode(&code.to_string()).unwrap();
        assert_eq!(decoded.spec(), code.spec());
        assert!(
            decoded
                .verify(&round.fingerprint(), enth_challenge::EngineVersion::CURRENT)
                .is_ok()
        );
        levels.push(round.generated().level);
        for (name, answer) in answers(&round) {
            let report = score_round(&round, &answer).unwrap();
            let mark = f64::from(report.pass_mark);
            assert!(
                (report.sound.percent - mark).abs() >= MARGIN || report.sound.percent == 100.0,
                "{code} {name}: sound {} is within {MARGIN} of the pass mark",
                report.sound.percent
            );
            if name == "exact" {
                assert!(report.passed && report.sound.percent == 100.0, "{code}");
            }
        }
    }
    assert_eq!(levels, [1, 2, 3, 4, 5]);
}

#[test]
#[ignore = "rewrites tests/golden/challenges/skr-v1.txt; run by hand, on purpose"]
#[allow(
    clippy::disallowed_methods,
    reason = "the regenerate test writes the golden file; the library does no I/O"
)]
fn regenerate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("challenges/skr-v1.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, compute()).unwrap();
}
