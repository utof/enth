//! Lane RC's frozen fixtures for the version-15 codes (docs/challenge.md, "Version 15: the kinds behind the escape"):
//! scorecards, answer codes and an ear result, written once to `tests/fixtures/result/v1/codes.json` and pinned by
//! its SHA-256 ([`CODES_SHA256`]), like code v1's fixture. A code is a promise to every friend who has one: each
//! fixture must decode to its case on every later build, and this build must write the same symbols for it. The
//! regenerate step writes the file only when it is missing; a new layout adds `result/v2/` beside it.
//!
//! The pinned hash was set at F1e by lane F1E, after the judge's layout changes (should-fix S1: an added module's id
//! number in the answer code; S5: the ear rules in the ear result; the F1e review's M3: taps stored from the first
//! tap) and with an ear challenge (kind 3) added, so every version-15 kind has a frozen code here. From F1e on, the
//! file never changes: a new layout adds `result/v2/` beside it.
//!
//! **One exception, 2026-10-26** (owner decision D3, lane ANS): the answer code's target, parameter, place and kind
//! fields were widened in place, because no friend build had carried layout 1 yet. The four answer entries were
//! regenerated (the file was removed and the step above wrote it again) and [`CODES_SHA256`] re-pinned; every other
//! entry is byte for byte what F1e froze, which [`OTHER_KINDS_SHA256`] pins on its own (its value was computed from the
//! F1e file and from the new one: the same). The answers' commitments did not move ([`COMMITMENTS`]).

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture file and writes it when missing; the library does no I/O"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::answer::AnswerCode;
use enth_challenge::ear::code::EarChallengeCode;
use enth_challenge::model::EarSpec;
use enth_challenge::paste::{self, Found};
use enth_challenge::random;
use enth_challenge::result::{
    CardAssists, Commitment, EarAnswer, EarResult, EarRules, Scorecard, ScorecardFields, ScorerSet,
};
use enth_challenge::{ChallengeCode, EngineVersion, RandomSpec, SpecPreset, TargetPrefix};
use enth_music::ear::Game;
use enth_patch::{
    MacroIndex, ModSource, ModuleId, ModuleInstance, ModuleKind, Op, Patch, PatchMeta, Position,
    Route, apply,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The SHA-256 of `result/v1/codes.json`.
const CODES_SHA256: &str = "e8d15567733b8ce0ce1c34f97a5a2261909e5b9349b2e74afc598a01cbbaf344";

/// The SHA-256 of every entry that is not an answer, as `<kind>|<code>\n` lines in file order. It is the same value on
/// the F1e file and on the file after the answer code's widening (D3): scorecards and ear codes did not change.
const OTHER_KINDS_SHA256: &str = "1be486d65aa75fa75c7b7110fd13870565a370a471e53f32c56ab7b5a53070db";

/// The commitments of the four answers of the fixture, measured on the F1e build (9188409, before the widening): a
/// commitment hashes the patch (with the salt and the target), never the code's bits, so a scorecard made before the
/// widening still matches the same answer after it. (name, commitment bits)
const COMMITMENTS: [(&str, u64); 4] = [
    ("answer, level 1, the target", 0x0376_5119_dbcd_d38d),
    ("answer, level 3, the target", 0x0247_d850_d573_9197),
    ("answer, level 5, the target", 0x074b_1d6d_8504_b2b9),
    (
        "answer, level 3, every op kind and personal fields",
        0x06e7_beb3_8097_93ab,
    ),
];

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Fixture {
    schema: String,
    codes: Vec<Entry>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    name: String,
    kind: String,
    symbols: usize,
    code: String,
}

fn path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/result/v1/codes.json")
}

fn short(level: u8, seed: u64) -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(level).unwrap().spec(1, seed)).unwrap()
}

fn target(c: &ChallengeCode) -> Patch {
    random::target(c.spec()).unwrap().patch
}

fn card(
    challenge: ChallengeCode,
    passed: bool,
    assists: CardAssists,
    commitment: u64,
) -> Scorecard {
    Scorecard::from_fields(ScorecardFields {
        challenge,
        engine: EngineVersion {
            major: 0,
            minor: 2,
            patch: 0,
        },
        scorers: ScorerSet::V1,
        sound_tenths: if passed { 934 } else { 712 },
        combined_tenths: if passed { 901 } else { 655 },
        passed,
        submits: if passed { 4 } else { 9 },
        time_ds: if passed { 1610 } else { 4321 },
        assists,
        commitment: Commitment::from_bits(commitment).unwrap(),
    })
    .unwrap()
}

/// A patch with a name, labels, a macro name and a slot set, a module added under a custom id, a route from it, a
/// module removed and another moved, and env 2 as the amplitude envelope: every op kind the answer code carries.
fn structural() -> Patch {
    let mut p = random::start(0).unwrap();
    let mut lfo = ModuleInstance::new(ModuleId::new("wobble").unwrap(), ModuleKind::Lfo);
    lfo.values[1] = 5.5;
    lfo.label = Some("Wobble".to_owned());
    let ops = vec![
        Op::AddModule {
            at: Position {
                section: ModuleKind::Lfo.section(),
                index: p.modulators.len(),
            },
            module: lfo,
        },
        Op::InsertRoute {
            index: 0,
            route: Route {
                src: ModSource::Module(ModuleId::new("wobble").unwrap()),
                dst: "filter1.cutoff".parse().unwrap(),
                depth: 35.0,
            },
        },
        Op::InsertRoute {
            index: 1,
            route: Route {
                src: ModSource::Macro(MacroIndex::new(2).unwrap()),
                dst: "osc1.fine".parse().unwrap(),
                depth: -12.5,
            },
        },
        Op::SetMacro {
            index: MacroIndex::new(2).unwrap(),
            value: 0.75,
        },
        Op::RemoveModule {
            id: ModuleId::new("noise1").unwrap(),
        },
        Op::Move {
            id: ModuleId::new("osc2").unwrap(),
            to: Position {
                section: enth_patch::Section::Generators,
                index: 0,
            },
        },
        Op::SetAmpEnv {
            id: ModuleId::new("env2").unwrap(),
        },
        Op::SetEnabled {
            id: ModuleId::new("reverb1").unwrap(),
            enabled: true,
        },
        Op::SetParam {
            addr: "filter1.cutoff".parse().unwrap(),
            value: 812.3456,
        },
        Op::SetMeta {
            meta: PatchMeta {
                name: "Mine".to_owned(),
                author: "A Real Name".to_owned(),
                description: String::new(),
                tags: vec![],
            },
        },
    ];
    for op in ops {
        apply(&mut p, &op).unwrap();
    }
    p
}

/// Every fixture case, built by this build.
fn cases() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut push = |name: &str, kind: &str, code: String| {
        out.push((name.to_owned(), kind.to_owned(), code));
    };
    push(
        "scorecard, level 2, passed",
        "scorecard",
        card(
            short(2, 42),
            true,
            CardAssists::default(),
            0x0012_3456_789A_BCDE,
        )
        .encode(),
    );
    push(
        "scorecard, level 5, not passed, hints and live meter",
        "scorecard",
        card(
            short(5, (1 << 40) - 1),
            false,
            CardAssists {
                hints: 3,
                live_meter: true,
                ..CardAssists::default()
            },
            (1 << 60) - 1,
        )
        .encode(),
    );
    let custom = RandomSpec {
        generator: 1,
        seed: 9_007_199_254_740_991,
        difficulty: 40,
        amounts: [None, Some(100), None, None, Some(0), None],
        musical: 70,
        template: 0,
    };
    let long =
        ChallengeCode::long(custom, EngineVersion::CURRENT, TargetPrefix(0x0BAD_F00D)).unwrap();
    push(
        "scorecard, custom settings, every assist",
        "scorecard",
        card(
            long,
            true,
            CardAssists {
                hints: 15,
                live_meter: true,
                dim_unused: true,
                closest: true,
                revealed: true,
            },
            42,
        )
        .encode(),
    );
    for (level, seed) in [(1, 42), (3, 42), (5, 42)] {
        let c = short(level, seed);
        push(
            &format!("answer, level {level}, the target"),
            "answer",
            AnswerCode::new(c, 0xC0FF_EE00_1234_5678, &target(&c))
                .unwrap()
                .encode(),
        );
    }
    let c = short(3, 7);
    push(
        "answer, level 3, every op kind and personal fields",
        "answer",
        AnswerCode::new(c, 1, &structural()).unwrap().encode(),
    );
    let ear = EarResult::new(
        EarSpec {
            game: 1,
            generator: 1,
            level: 3,
            seed: 123_456_789,
            count: 5,
        },
        EarRules::V1,
        vec![
            Some(EarAnswer::Choice(0)),
            Some(EarAnswer::Choice(5)),
            None,
            Some(EarAnswer::Value(1.25)),
            Some(EarAnswer::Taps(vec![0, 240, 510, 760])),
        ],
        61_500,
        1,
        false,
    )
    .unwrap();
    push("ear result, five answers", "ear-result", ear.encode());
    let ear_challenge =
        EarChallengeCode::new(EarSpec::new(Game::IntervalStacks, 1, 123_456_789, 20)).unwrap();
    push(
        "ear challenge, interval stacks, level 1",
        "ear-challenge",
        ear_challenge.encode(),
    );
    out
}

fn fixture() -> Fixture {
    Fixture {
        schema: "enth.result-codes-fixture/1".to_owned(),
        codes: cases()
            .into_iter()
            .map(|(name, kind, code)| Entry {
                name,
                kind,
                symbols: code.bytes().filter(|&b| b != b'-').count(),
                code,
            })
            .collect(),
    }
}

/// The file's text: pretty JSON with a final newline.
fn render(f: &Fixture) -> String {
    let mut s = serde_json::to_string_pretty(f).unwrap();
    s.push('\n');
    s
}

/// Writes the fixture when it is missing (never over an existing one).
#[test]
fn regenerate_writes_the_fixture_only_when_missing() {
    let path = path();
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, render(&fixture())).unwrap();
    }
}

fn read() -> String {
    let path = path();
    if !path.exists() {
        regenerate_writes_the_fixture_only_when_missing();
    }
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn this_build_writes_the_frozen_codes() {
    let file: Fixture = serde_json::from_str(&read()).unwrap();
    assert_eq!(file, fixture());
}

#[test]
fn every_frozen_code_decodes_to_its_kind() {
    let file: Fixture = serde_json::from_str(&read()).unwrap();
    for e in &file.codes {
        let found = paste::decode(&e.code).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        let kind = match &found {
            Found::Scorecard(_) => "scorecard",
            Found::Answer(_) => "answer",
            Found::EarResult(_) => "ear-result",
            Found::EarChallenge(_) => "ear-challenge",
            _ => "other",
        };
        assert_eq!(kind, e.kind, "{}", e.name);
        assert_eq!(found.encode(), e.code, "{}", e.name);
        // An R1-R3 build's decoder: "update enth" up to 64 symbols, "bad length" above.
        let old = ChallengeCode::decode(&e.code).unwrap_err().code();
        assert_eq!(
            old,
            if e.symbols <= 64 {
                "code_too_new"
            } else {
                "bad_length"
            },
            "{}",
            e.name
        );
    }
    // The personal fields of the structural case never reach its code.
    let structural = file
        .codes
        .iter()
        .find(|e| e.name.contains("personal"))
        .unwrap();
    let Ok(Found::Answer(a)) = paste::decode(&structural.code) else {
        panic!("the structural answer decodes");
    };
    assert_eq!(a.answer().meta.author, "");
    assert!(a.answer().find(&ModuleId::new("wobble").unwrap()).is_none());
    assert!(a.answer().find(&ModuleId::new("lfo3").unwrap()).is_some());
    assert_eq!(a.answer().voice.amp_env.as_str(), "env2");
}

#[test]
fn the_fixture_is_frozen() {
    let hex = Sha256::digest(read().as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    assert_eq!(hex, CODES_SHA256, "result/v1/codes.json is frozen");
}

#[test]
fn the_other_kinds_are_what_f1e_froze() {
    let file: Fixture = serde_json::from_str(&read()).unwrap();
    let mut sha = Sha256::new();
    for e in file.codes.iter().filter(|e| e.kind != "answer") {
        sha.update(format!("{}|{}\n", e.kind, e.code).as_bytes());
    }
    let hex = sha.finalize().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    assert_eq!(hex, OTHER_KINDS_SHA256, "a scorecard or ear code changed");
}

/// The four answers of the fixture, by name, made by this build.
fn fixture_answers() -> Vec<(&'static str, AnswerCode)> {
    let level = |name, level, seed| {
        let c = short(level, seed);
        (
            name,
            AnswerCode::new(c, 0xC0FF_EE00_1234_5678, &target(&c)).unwrap(),
        )
    };
    vec![
        level("answer, level 1, the target", 1, 42),
        level("answer, level 3, the target", 3, 42),
        level("answer, level 5, the target", 5, 42),
        (
            "answer, level 3, every op kind and personal fields",
            AnswerCode::new(short(3, 7), 1, &structural()).unwrap(),
        ),
    ]
}

/// The answer code was widened in place (D3), and a commitment hashes the patch, not the code's bits: the same answer
/// (salt, challenge, patch) gives the commitment the F1e build gave, so a scorecard made then still matches it.
#[test]
fn the_commitments_did_not_move_with_the_widening() {
    let file: Fixture = serde_json::from_str(&read()).unwrap();
    let answers = fixture_answers();
    assert_eq!(answers.len(), COMMITMENTS.len());
    for ((name, bits), (n, made)) in COMMITMENTS.iter().zip(&answers) {
        assert_eq!(name, n);
        assert_eq!(made.commitment().bits(), *bits, "{name}");
        // The frozen code decodes to the same answer, and so to the same commitment.
        let entry = file.codes.iter().find(|e| e.name == *name).unwrap();
        let Ok(Found::Answer(back)) = paste::decode(&entry.code) else {
            panic!("{name} decodes");
        };
        assert_eq!(back.commitment(), made.commitment(), "{name}");
    }
}

/// A scorecard that carries an F1e-era commitment accepts the widened code's answer: `verify` says the commitment
/// matches (the claim's other fields are the fixture's made-up numbers, and are not what is asked here).
#[test]
fn a_scorecard_made_before_the_widening_still_matches_its_answer() {
    let (name, bits) = COMMITMENTS[0];
    let file: Fixture = serde_json::from_str(&read()).unwrap();
    let entry = file.codes.iter().find(|e| e.name == name).unwrap();
    let Ok(Found::Answer(answer)) = paste::decode(&entry.code) else {
        panic!("{name} decodes");
    };
    let card = card(*answer.challenge(), true, CardAssists::default(), bits);
    let verified = enth_challenge::verify::verify(&answer, Some(&card)).unwrap();
    assert!(verified.claim.unwrap().commitment_matches);
}
