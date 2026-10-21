//! The persisted forms of lane CT's types, pinned: Challenge v1 and v2 (lane CV2), code v1, `ScoreReport` v1 and
//! `RoundResult` v1.
//!
//! Challenge v2 (R2) writes a challenge at the lowest version that holds it, so the v1 cases below still write the
//! v1 fixtures byte for byte, and every v1 fixture reads as v2 (the migration). `challenge/v2/` holds the cases that
//! need a v2 piece: a draw over a patch start with constraints, a patch target, a seed target with an editable list.
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

use enth_challenge::model::{DEFAULT_PASS_MARK, Start, Target};
use enth_challenge::score::{ModuleStatus, ModuleTerm, ParamScore};
use enth_challenge::spec::MAX_SEED;
use enth_challenge::v2::{
    Constraints, Draw, DrawAt, DrawItem, DrawValue, DrawValues, InlinePatch, Selector,
};
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
    dir_v(kind, 1)
}

fn dir_v(kind: &str, version: u32) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{kind}/v{version}"))
}

fn read(kind: &str, name: &str) -> String {
    read_v(kind, 1, name)
}

fn read_v(kind: &str, version: u32, name: &str) -> String {
    let path = dir_v(kind, version).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn write(kind: &str, name: &str, text: &str) {
    write_v(kind, 1, name, text);
}

fn write_v(kind: &str, version: u32, name: &str, text: &str) {
    let d = dir_v(kind, version);
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

/// The frozen starter (template 0) with ops applied: inline patches whose bytes no editor or engine change moves (its
/// engine stamp is the file's own).
fn starter_with(ops: &[(&str, f32)]) -> enth_patch::Patch {
    let mut p = enth_challenge::random::start(0).unwrap();
    for (addr, value) in ops {
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetParam {
                addr: addr.parse().unwrap(),
                value: *value,
            },
        )
        .unwrap();
    }
    p
}

fn selectors(texts: &[&str]) -> Vec<Selector> {
    texts.iter().map(|t| t.parse().unwrap()).collect()
}

fn v1_scorers(difficulty: u8) -> ScoringSpec {
    ScoringSpec::with_scorers(
        difficulty,
        ScorerId::SOUND_V1,
        ScorerId::STRUCTURE_V1,
        ScorerId::COMBINED_V1,
        ScorerId::TIME_V1,
    )
}

/// Challenge v2's cases (lane CV2): each needs a v2 piece, so each writes `enth.challenge/2`.
fn challenges_v2() -> Vec<(&'static str, Challenge)> {
    let num = DrawValue::Number;
    let text = |t: &str| DrawValue::Text(t.to_owned());
    // Lesson 2.1-style: the cutoff from a set, the mode by key, the resonance on a grid, an envelope route's signed
    // depth; a patch start; the cutoff the step's control, the drive locked.
    let mut draw = Challenge::new(
        Target::Draw(Draw {
            seed: 7,
            items: vec![
                DrawItem {
                    at: DrawAt::Param("filter1.cutoff".parse().unwrap()),
                    values: DrawValues::Set(vec![
                        num(300.0),
                        num(600.0),
                        num(1200.0),
                        text("4kHz"),
                    ]),
                },
                DrawItem {
                    at: DrawAt::Param("filter1.mode".parse().unwrap()),
                    values: DrawValues::Set(vec![text("lp"), text("bp")]),
                },
                DrawItem {
                    at: DrawAt::Param("filter1.resonance".parse().unwrap()),
                    values: DrawValues::Range {
                        low: num(10.0),
                        high: num(70.0),
                        step: Some(num(10.0)),
                    },
                },
                DrawItem {
                    at: DrawAt::Route("env2 -> filter1.cutoff".parse().unwrap()),
                    values: DrawValues::Set(vec![num(-30.0), num(30.0), text("+1oct")]),
                },
            ],
        }),
        Start::Patch {
            patch: InlinePatch::new(starter_with(&[
                ("filter1.cutoff", 2000.0),
                ("env1.decay", 250.0),
            ])),
        },
        PhraseId(1),
        v1_scorers(0),
    );
    draw.constraints = Constraints {
        editable: selectors(&[
            "filter1.cutoff",
            "filter1.mode",
            "filter1.resonance",
            "env2 -> filter1.cutoff",
        ]),
        locked: selectors(&["filter1.drive", "osc1.*"]),
    };
    draw.meta = ChallengeMeta {
        title: "Low-pass cutoff, D1".to_owned(),
        tags: vec!["basics".to_owned(), "2.1".to_owned()],
    };
    // A study's step: an explicit target patch (a macro routed to the cutoff, a slow sweep heard on the long hold)
    // from its own start; the macro and the filter are the step's, the oscillator locked.
    let mut target = starter_with(&[("filter1.cutoff", 1500.0), ("lfo1.rate", 0.5)]);
    enth_patch::apply(
        &mut target,
        enth_patch::Op::InsertRoute {
            index: 0,
            route: enth_patch::Route {
                src: "lfo1".parse().unwrap(),
                dst: "filter1.cutoff".parse().unwrap(),
                depth: 25.0,
            },
        },
    )
    .unwrap();
    let mut patch = Challenge::new(
        Target::Patch(InlinePatch::new(target)),
        Start::Patch {
            patch: InlinePatch::new(starter_with(&[("filter1.cutoff", 1500.0)])),
        },
        enth_challenge::lesson_phrases::LONG_HOLD,
        v1_scorers(50),
    );
    patch.constraints = Constraints {
        editable: selectors(&["lfo1.*", "lfo1 -> filter1.cutoff"]),
        locked: selectors(&["osc1.*", "macro1"]),
    };
    patch.assists.hint_budget = Some(0);
    // A unit test's level-1 round with the oscillator its control (a seed target needs v2 for its constraint). A
    // seed target takes no lock: its draws change with the seed (F1l review M2), so locks go on a draw target.
    let mut seed = recreate(level(1, 42), PhraseId(1));
    seed.constraints = Constraints {
        editable: selectors(&["osc1.*"]),
        locked: Vec::new(),
    };
    vec![("draw", draw), ("patch", patch), ("seed-editable", seed)]
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
            dim_unused: false,
            closest: false,
        },
        agreement: Some(Agreement::AboutRight),
        rating: Some(4),
    }
}

fn gave_up() -> RoundResult {
    RoundResult {
        // Pinned, not this build's engine (as in `report`): the fixture pins the format, so an engine bump (lane
        // DRV's 0.3.0) does not rewrite it.
        code: ChallengeCode::long(
            custom(),
            EngineVersion::parse("0.2.0").unwrap(),
            TargetPrefix(0xdead_beef),
        )
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
            dim_unused: false,
            closest: false,
        },
        agreement: None,
        rating: None,
    }
}

/// A round passed with the `dim_unused` assist and nothing else (lane HINT, FU1): assisted by the dimming alone.
fn dimmed() -> RoundResult {
    RoundResult {
        code: ChallengeCode::short(level(3, 7)).unwrap(),
        outcome: Outcome::Passed,
        first_submit: Some(report()),
        submits: 1,
        time_to_pass_ms: Some(61_000),
        active_ms: 64_500,
        assists: AssistsUsed {
            dim_unused: true,
            ..AssistsUsed::default()
        },
        agreement: Some(Agreement::TooLow),
        rating: Some(3),
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
    for (name, c) in challenges_v2() {
        write_v("challenge", 2, &format!("{name}.json"), &c.to_json_pretty());
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
    write(
        "round",
        "dimmed.json",
        &serde_json::to_string_pretty(&dimmed()).unwrap(),
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

/// Challenge v1's fixtures read as v2 (lane CV2's acceptance): the migrated challenge is the v1 one with no change
/// of meaning (a seed target, its template's start, no constraint), it is still written as v1, and the embedded serde
/// path reads it too.
#[test]
fn challenge_v1_fixtures_read_as_v2() {
    for (name, c) in challenges() {
        let text = read("challenge", &format!("{name}.json"));
        assert!(text.contains("\"enth.challenge/1\""), "{name}");
        let read = Challenge::from_json(&text).unwrap();
        assert_eq!(read.version(), 1, "{name}");
        assert!(matches!(read.target, Target::Seed(_)), "{name}");
        assert!(
            matches!(read.start, Start::Template { template: 0 }),
            "{name}"
        );
        assert!(read.constraints.is_empty(), "{name}");
        assert_eq!(
            serde_json::from_str::<Challenge>(&text).unwrap(),
            c,
            "{name}"
        );
        assert_eq!(
            read.to_json_pretty() + "\n",
            text,
            "{name}: still written as v1"
        );
    }
}

#[test]
fn challenge_v2_fixtures_are_what_this_build_writes_and_reads() {
    for (name, c) in challenges_v2() {
        let text = read_v("challenge", 2, &format!("{name}.json"));
        assert_eq!(c.version(), 2, "{name}");
        assert!(text.contains("\"enth.challenge/2\""), "{name}");
        assert_eq!(text, format!("{}\n", c.to_json_pretty()), "{name}");
        assert_eq!(Challenge::from_json(&text), Ok(c.clone()), "{name}");
        assert_eq!(
            serde_json::from_str::<Challenge>(&text).unwrap(),
            c,
            "{name}"
        );
        assert_eq!(Challenge::from_json(&c.to_json()), Ok(c.clone()), "{name}");
        // A v1 build refuses it by its schema: no v1 reader sees a v2 piece.
        assert_eq!(
            Challenge::from_json(&text.replacen("enth.challenge/2", "enth.challenge/1", 1))
                .unwrap_err()
                .code(),
            "bad_challenge",
            "{name}"
        );
    }
}

/// Every v2 fixture prepares a round whose target scores 100, and the draw's target is pinned by its fingerprint.
#[test]
fn challenge_v2_fixtures_prepare() {
    for (name, c) in challenges_v2() {
        let round =
            enth_challenge::prepare_challenge(c.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(round.challenge(), &c, "{name}");
        if name == "draw" {
            assert_eq!(
                round.fingerprint().to_hex(),
                DRAW_FINGERPRINT,
                "the draw fixture's target moved: a draw rule changed"
            );
        }
    }
}

/// The draw fixture's target fingerprint: a change to how a draw picks, places or quantizes values moves it.
const DRAW_FINGERPRINT: &str = "03f82ae2982232db9506ab8eb290a1f62e2123ab64a7118952e611c16aba1045";

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
    for (name, r) in [
        ("passed", round()),
        ("gave-up", gave_up()),
        ("dimmed", dimmed()),
    ] {
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

/// Old results still read (lane HINT, FU1): `passed.json` and `gave-up.json` were written before `AssistsUsed` had
/// `dim_unused`, and they are still exactly what this build writes for a round without the dim assist (the field is
/// written only when set), so every older round log keeps its bytes and reads with the flag off. `dimmed.json` is a
/// round assisted by the dimming alone.
#[test]
fn round_results_from_before_the_dim_assist_read_and_keep_their_bytes() {
    for name in ["passed", "gave-up"] {
        let text = read("round", &format!("{name}.json"));
        assert!(!text.contains("dim_unused"), "{name} predates the field");
        let r: RoundResult = serde_json::from_str(&text).unwrap();
        assert!(!r.assists.dim_unused, "{name}");
    }
    let text = read("round", "dimmed.json");
    let r: RoundResult = serde_json::from_str(&text).unwrap();
    assert!(r.assists.dim_unused && r.assists.any());
    assert_eq!((r.assists.hints, r.assists.revealed), (0, false));
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
