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
const CODES_SHA256: &str = "82dc0247779d9bf1cca228976be67b48941ce3505cc9e44e16b37bcb18c5e19d";

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
