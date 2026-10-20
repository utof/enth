//! The persisted forms of lane CT's types, pinned: Challenge v1, code v1, `ScoreReport` v1 and `RoundResult` v1.
//!
//! Fixtures live in `tests/fixtures/<kind>/v<N>/`. They are the bytes this build writes for the cases below, and they
//! must decode to those cases: a change to a format fails here. A code fixture is a promise to every player who has a
//! code, so `code/v1/codes.json` is never regenerated: its SHA-256 is pinned below ([`CODES_V1_SHA256`]), and the
//! regenerate test writes it only when it is missing. A new code version adds `code/v2/` beside it. (Code v1 was
//! re-frozen once before any code was shared, at the F1r review: the GF(32) check, 53-bit seeds, template 0.)
//! When another format changes on purpose (a new version), add `v<N+1>/` fixtures, keep the old ones as migration
//! inputs, and regenerate the current ones with `cargo test -p enth-challenge --test fixtures -- --ignored
//! regenerate`, then read the diff.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::model::DEFAULT_PASS_MARK;
use enth_challenge::score::{ModuleStatus, ModuleTerm, ParamScore};
use enth_challenge::spec::MAX_SEED;
use enth_challenge::{
    Agreement, AssistsUsed, Category, Challenge, ChallengeCode, ChallengeMeta, Closeness, CodeForm,
    Combined, Component, ComponentKind, Direction, EngineVersion, Outcome, PhraseId, RandomSpec,
    RoundResult, ScoreReport, ScorerId, ScoringSpec, SoundScore, SpecPreset, StructureScore,
    TargetPrefix, Verdict,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The SHA-256 of `code/v1/codes.json`, frozen with code v1 (T4 of the F1r review).
const CODES_V1_SHA256: &str = "e962d821c8122b0ca6db17cb3408a721f14984653a4238348f3cf31021d75bfe";

fn dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{kind}/v1"))
}

fn read(kind: &str, name: &str) -> String {
    let path = dir(kind).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn write(kind: &str, name: &str, text: &str) {
    let d = dir(kind);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(name), format!("{text}\n")).unwrap();
}

// ---------------------------------------------------------------------------------------------------------------
// Cases

fn level(id: u8, seed: u64) -> RandomSpec {
    SpecPreset::by_id(id).unwrap().spec(1, seed)
}

fn custom() -> RandomSpec {
    RandomSpec {
        generator: 1,
        seed: MAX_SEED,
        difficulty: 40,
        amounts: [None, Some(100), None, None, Some(0), None],
        musical: 70,
        template: 0,
    }
}

/// A challenge with the v1 scorer ids, whatever this build's default ([`ScoringSpec::recreate`]) names: the fixture
/// pins the format, not the default.
fn recreate(spec: RandomSpec, phrase: PhraseId) -> Challenge {
    let mut c = Challenge::recreate(spec, phrase);
    c.scoring = ScoringSpec::with_scorers(
        spec.difficulty,
        ScorerId::SOUND_V1,
        ScorerId::STRUCTURE_V1,
        ScorerId::COMBINED_V1,
        ScorerId::TIME_V1,
    );
    c
}

fn challenges() -> Vec<(&'static str, Challenge)> {
    let mut assisted = recreate(custom(), PhraseId(3));
    assisted.assists.live_meter = true;
    assisted.assists.dim_unused = true;
    assisted.assists.hint_budget = Some(2);
    assisted.meta = ChallengeMeta {
        title: "Custom, with assists".to_owned(),
        tags: vec!["fixture".to_owned(), "r1".to_owned()],
    };
    vec![
        ("level1", recreate(level(1, 42), PhraseId(1))),
        ("level5", recreate(level(5, (1 << 40) - 1), PhraseId(2))),
        ("custom", assisted),
    ]
}

/// One code fixture: the inputs of both forms and the texts they give.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeCase {
    name: String,
    spec: RandomSpec,
    engine: String,
    target: String,
    short: Option<String>,
    long: String,
}

fn code_cases() -> Vec<CodeCase> {
    let engines = [
        ("0.2.0", EngineVersion::parse("0.2.0").unwrap()),
        ("255.255.255", EngineVersion::parse("255.255.255").unwrap()),
    ];
    let mut every_amount = custom();
    every_amount.amounts = [Some(0), Some(1), Some(50), Some(99), Some(100), Some(7)];
    every_amount.seed = 0x0012_3456_789a_bcde;
    let mut zero = level(1, 0);
    zero.musical = 0;
    let specs = [
        ("level1-seed0", level(1, 0)),
        ("level2-seed42", level(2, 42)),
        ("level3-seed-max-short", level(3, (1 << 40) - 1)),
        ("level4-seed-2^40", level(4, 1 << 40)),
        ("level4-seed-short", level(4, 0x00ab_cdef_0123)),
        ("level5-seed-max", level(5, MAX_SEED)),
        ("level5-seed7", level(5, 7)),
        ("custom", custom()),
        ("every-amount", every_amount),
        ("musical-0", zero),
    ];
    let targets = [0u32, 0x0123_abcd, u32::MAX];
    specs
        .iter()
        .enumerate()
        .map(|(i, (name, spec))| {
            let (engine_text, engine) = engines[i % engines.len()];
            let target = TargetPrefix(targets[i % targets.len()]);
            CodeCase {
                name: (*name).to_owned(),
                spec: *spec,
                engine: engine_text.to_owned(),
                target: target.to_string(),
                short: ChallengeCode::short(*spec).map(|c| c.encode()),
                long: ChallengeCode::long(*spec, engine, target).unwrap().encode(),
            }
        })
        .collect()
}

fn report() -> ScoreReport {
    let scoring = ScoringSpec::recreate(50);
    let sound = SoundScore {
        scorer: ScorerId::SOUND_V1,
        percent: 91.25,
        distance: 0.3125,
        floor: 0.0625,
        scale: 0.5,
        jnd: 0.5,
        components: vec![
            Component {
                kind: ComponentKind::Pitch,
                direction: Direction::Same,
                closeness: Closeness::Close,
                target: Some(-2100.0),
                answer: Some(-2098.5),
            },
            Component {
                kind: ComponentKind::Brightness,
                direction: Direction::Higher,
                closeness: Closeness::ABit,
                target: Some(1200.0),
                answer: Some(1530.5),
            },
            Component {
                kind: ComponentKind::Harmonics,
                direction: Direction::Lower,
                closeness: Closeness::Close,
                target: Some(0.5),
                answer: Some(0.48),
            },
            Component {
                kind: ComponentKind::Sustain,
                direction: Direction::Higher,
                closeness: Closeness::ABit,
                target: Some(-12.0),
                answer: Some(-6.5),
            },
            Component {
                kind: ComponentKind::Movement,
                direction: Direction::Missing,
                closeness: Closeness::Far,
                target: Some(4.5),
                answer: None,
            },
        ],
    };
    let structure = StructureScore {
        scorer: ScorerId::STRUCTURE_V1,
        percent: 75.5,
        params: vec![
            ParamScore {
                addr: "filter1.cutoff".to_owned(),
                answer_addr: Some("filter1.cutoff".to_owned()),
                score: 0.5,
                verdict: Verdict::Near,
                direction: Direction::Higher,
                randomized: true,
            },
            ParamScore {
                addr: "osc1.wave".to_owned(),
                answer_addr: Some("osc2.wave".to_owned()),
                score: 1.0,
                verdict: Verdict::Good,
                direction: Direction::Same,
                randomized: true,
            },
            ParamScore {
                addr: "osc2.level".to_owned(),
                answer_addr: None,
                score: 0.0,
                verdict: Verdict::Bad,
                direction: Direction::Missing,
                randomized: true,
            },
            ParamScore {
                addr: "env1.release".to_owned(),
                answer_addr: Some("env1.release".to_owned()),
                score: 0.25,
                verdict: Verdict::Bad,
                direction: Direction::Lower,
                randomized: false,
            },
        ],
        modules: vec![ModuleTerm {
            id: "lfo1".to_owned(),
            kind: "lfo".to_owned(),
            status: ModuleStatus::Missing,
        }],
    };
    let combined = Combined {
        scorer: ScorerId::COMBINED_V1,
        percent: 85.737_5,
        sound_weight: scoring.sound_weight,
    };
    let mut r = ScoreReport::new(&scoring, sound, structure, combined);
    // Pinned, not this build's engine: the fixture pins the format.
    r.engine = EngineVersion::parse("0.2.0");
    r
}

fn round() -> RoundResult {
    RoundResult {
        code: ChallengeCode::short(level(2, 42)).unwrap(),
        outcome: Outcome::Passed,
        first_submit: Some(report()),
        submits: 2,
        time_to_pass_ms: Some(95_250),
        active_ms: 120_000,
        assists: AssistsUsed {
            hints: 1,
            live_meter: false,
            revealed: false,
        },
        agreement: Some(Agreement::AboutRight),
        rating: Some(4),
    }
}

fn gave_up() -> RoundResult {
    RoundResult {
        code: ChallengeCode::long(custom(), EngineVersion::CURRENT, TargetPrefix(0xdead_beef))
            .unwrap(),
        outcome: Outcome::GaveUp,
        first_submit: None,
        submits: 0,
        time_to_pass_ms: None,
        active_ms: 300_000,
        assists: AssistsUsed {
            hints: 0,
            live_meter: true,
            revealed: true,
        },
        agreement: None,
        rating: None,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tests

#[test]
#[ignore = "writes the fixtures: run on purpose and review the diff"]
fn regenerate() {
    for (name, c) in challenges() {
        write("challenge", &format!("{name}.json"), &c.to_json_pretty());
    }
    // Code v1 is frozen: written only when missing (a new code version adds `code/v2/`).
    if !dir("code").join("codes.json").exists() {
        write(
            "code",
            "codes.json",
            &serde_json::to_string_pretty(&code_cases()).unwrap(),
        );
    }
    write(
        "score",
        "report.json",
        &serde_json::to_string_pretty(&report()).unwrap(),
    );
    write(
        "round",
        "passed.json",
        &serde_json::to_string_pretty(&round()).unwrap(),
    );
    write(
        "round",
        "gave-up.json",
        &serde_json::to_string_pretty(&gave_up()).unwrap(),
    );
}

#[test]
fn challenge_v1_fixtures_are_what_this_build_writes_and_reads() {
    for (name, c) in challenges() {
        let text = read("challenge", &format!("{name}.json"));
        assert_eq!(text, format!("{}\n", c.to_json_pretty()), "{name}");
        assert_eq!(Challenge::from_json(&text), Ok(c.clone()), "{name}");
        assert_eq!(Challenge::from_json(&c.to_json()), Ok(c), "{name}");
    }
}

/// T4 (F1r review): code v1's fixture is frozen by its SHA-256, so no regenerate run can quietly rewrite it.
#[test]
fn code_v1_fixture_bytes_are_pinned() {
    let digest: [u8; 32] = Sha256::digest(read("code", "codes.json").as_bytes()).into();
    let hex = digest.iter().fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    assert_eq!(hex, CODES_V1_SHA256, "code/v1/codes.json is frozen");
}

#[test]
fn code_v1_fixtures_decode_to_their_specs_and_encode_back() {
    let cases: Vec<CodeCase> = serde_json::from_str(&read("code", "codes.json")).unwrap();
    assert_eq!(cases, code_cases(), "code/v1/codes.json is frozen");
    for case in &cases {
        let engine = EngineVersion::parse(&case.engine).unwrap();
        let target = TargetPrefix(u32::from_str_radix(&case.target, 16).unwrap());
        let long = ChallengeCode::decode(&case.long).unwrap();
        assert_eq!(long.spec(), &case.spec, "{}", case.name);
        assert_eq!(long.form(), CodeForm::Long);
        assert_eq!(long.engine(), Some(engine));
        assert_eq!(long.target(), Some(target));
        assert_eq!(long.encode(), case.long);
        match &case.short {
            Some(text) => {
                let short = ChallengeCode::decode(text).unwrap();
                assert_eq!(short.spec(), &case.spec, "{}", case.name);
                assert_eq!(short.form(), CodeForm::Short);
                assert_eq!(&short.encode(), text);
                assert_eq!(text.len(), 17, "14 symbols and 3 hyphens");
            }
            None => assert!(ChallengeCode::short(case.spec).is_none(), "{}", case.name),
        }
    }
    // The fixture covers both forms and every preset.
    assert!(cases.iter().any(|c| c.short.is_none()));
    for p in 1..=5 {
        assert!(
            cases
                .iter()
                .any(|c| c.short.is_some() && c.spec.preset().map(|s| s.id) == Some(p)),
            "preset {p}"
        );
    }
}

#[test]
fn score_report_v1_fixture_is_what_this_build_writes_and_reads() {
    let text = read("score", "report.json");
    let r = report();
    assert_eq!(
        text,
        format!("{}\n", serde_json::to_string_pretty(&r).unwrap())
    );
    assert_eq!(serde_json::from_str::<ScoreReport>(&text).unwrap(), r);
    assert!(text.starts_with("{\n  \"schema\": \"enth.score-report/1\""));
    assert!(r.passed, "91.25 passes at {DEFAULT_PASS_MARK}");
    assert_eq!(r.margin(), 1.25);
    assert_eq!(
        r.verdicts().collect::<Vec<_>>(),
        [
            ("filter1.cutoff", Verdict::Near),
            ("osc1.wave", Verdict::Good),
            ("osc2.level", Verdict::Bad)
        ],
        "the rings: randomized rows only"
    );
    assert_eq!(
        r.structure.params[1]
            .answer_param_addr()
            .unwrap()
            .to_string(),
        "osc2.wave"
    );
    assert_eq!(
        r.structure.params[0].param_addr().unwrap().to_string(),
        "filter1.cutoff"
    );
}

#[test]
fn score_reports_that_contradict_themselves_are_refused() {
    let json = serde_json::to_string(&report()).unwrap();
    for (from, to) in [
        ("\"passed\":true", "\"passed\":false"),
        ("\"percent\":91.25", "\"percent\":101.0"),
        ("\"percent\":91.25", "\"percent\":-1.0"),
        (
            "\"schema\":\"enth.score-report/1\"",
            "\"schema\":\"enth.score-report/2\"",
        ),
        ("\"sound/1\"", "\"sound\""),
    ] {
        assert!(json.contains(from), "{from}");
        let bad = json.replacen(from, to, 1);
        assert!(serde_json::from_str::<ScoreReport>(&bad).is_err(), "{to}");
    }
}

/// S6 (F1r review): a report is an output, read tolerantly. A newer build's extra fields and enum values read
/// (unknown values as `unknown`), and a report written before the S6 fields existed still reads, with defaults.
#[test]
fn score_reports_read_tolerantly() {
    let json = serde_json::to_string(&report()).unwrap();
    let newer = json
        .replacen("\"pass_mark\":90", "\"pass_mark\":90,\"extra\":1", 1)
        .replacen("\"kind\":\"pitch\"", "\"kind\":\"formants\"", 1)
        .replacen("\"direction\":\"same\"", "\"direction\":\"wobbly\"", 1)
        .replacen("\"closeness\":\"close\"", "\"closeness\":\"spot_on\"", 1)
        .replacen("\"verdict\":\"near\"", "\"verdict\":\"almost\"", 1)
        .replacen("\"status\":\"missing\"", "\"status\":\"moved\"", 1)
        .replacen("\"jnd\":0.5", "\"jnd\":0.5,\"pooled\":true", 1);
    let r: ScoreReport = serde_json::from_str(&newer).unwrap();
    assert_eq!(r.sound.components[0].kind, ComponentKind::Unknown);
    assert_eq!(r.sound.components[0].direction, Direction::Unknown);
    assert_eq!(r.sound.components[0].closeness, Closeness::Unknown);
    assert_eq!(r.structure.params[0].verdict, Verdict::Unknown);
    assert_eq!(r.structure.modules[0].status, ModuleStatus::Unknown);
    // Before S6: no answer address, direction, randomized flag or engine.
    let old = json
        .replace(",\"answer_addr\":\"filter1.cutoff\"", "")
        .replace(",\"direction\":\"higher\",\"randomized\":true", "")
        .replace(",\"engine\":\"0.2.0\"", "");
    assert_ne!(old, json);
    let r: ScoreReport = serde_json::from_str(&old).unwrap();
    let p = &r.structure.params[0];
    assert_eq!(
        (p.answer_addr.as_deref(), p.direction, p.randomized),
        (None, Direction::Unknown, true)
    );
    assert_eq!(r.engine, None);
}

#[test]
fn round_result_v1_fixtures_are_what_this_build_writes_and_reads() {
    for (name, r) in [("passed", round()), ("gave-up", gave_up())] {
        let text = read("round", &format!("{name}.json"));
        assert_eq!(
            text,
            format!("{}\n", serde_json::to_string_pretty(&r).unwrap()),
            "{name}"
        );
        assert_eq!(serde_json::from_str::<RoundResult>(&text).unwrap(), r);
    }
    let json = serde_json::to_string(&round()).unwrap();
    let inconsistent = json.replacen("\"time_to_pass_ms\":95250", "\"time_to_pass_ms\":null", 1);
    assert!(serde_json::from_str::<RoundResult>(&inconsistent).is_err());
}

/// The JSON forms hold no float except the scores: every number in a challenge is an integer.
#[test]
fn a_challenge_holds_integers_only() {
    fn numbers(v: &serde_json::Value, out: &mut Vec<serde_json::Number>) {
        match v {
            serde_json::Value::Number(n) => out.push(n.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|i| numbers(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| numbers(i, out)),
            _ => {}
        }
    }
    for (name, c) in challenges() {
        let mut found = Vec::new();
        numbers(&serde_json::from_str(&c.to_json()).unwrap(), &mut found);
        assert!(found.iter().all(serde_json::Number::is_u64), "{name}");
    }
    let mut spec = custom();
    spec.amounts[Category::Structure.index()] = Some(5);
    assert!(Challenge::recreate(spec, PhraseId(1)).validate().is_ok());
}
