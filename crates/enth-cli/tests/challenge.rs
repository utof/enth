//! `enth challenge new|show` and `enth compare` (lane SKR's first versions, F2r; lane CL finishes them). Each output
//! is deserialized back into its published type (the typed round trip). The snapshots pin codes, settings, phrases
//! and target fingerprints, which depend on the frozen generator only (no render); the scorer ids are filtered,
//! because a code's challenge names this build's default scorers, which lanes SS and ST switch from the stubs.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;

use common::{Scratch, json, record, round_trip, run};
use enth_challenge::spec::{LATEST_GENERATOR, SpecPreset};
use enth_cli::schema::{ChallengeNewOut, ChallengeShowOut, CompareOut, ErrorOut};
use insta::assert_snapshot;

/// Snapshot settings: the scratch directory and the scorer ids filtered.
fn settings(s: &Scratch) -> insta::Settings {
    let mut settings = s.settings();
    settings.add_filter(r"(sound|structure|combined)(-stub)?/\d+", "[scorer]");
    settings
}

fn level(n: u8, seed: u64) -> enth_challenge::RandomSpec {
    SpecPreset::by_id(n).unwrap().spec(LATEST_GENERATOR, seed)
}

#[test]
fn challenge_new_prints_the_code_settings_and_fingerprint() {
    let s = Scratch::new("challenge_new");
    let o = run(&["challenge", "new", "--level", "2", "--seed", "42"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: ChallengeNewOut = round_trip(&o);
    let round = enth_challenge::prepare_round(&level(2, 42)).unwrap();
    assert_eq!(out.fingerprint, round.fingerprint().to_hex());
    assert_eq!(out.form, "short");
    assert_eq!(out.level, 2);
    assert_eq!(out.challenge["schema"], "enth.challenge/1");
    // The code reads back to the same settings, and `new --code` gives the same challenge.
    let again = run(&["challenge", "new", "--code", &out.code]);
    let again: ChallengeNewOut = round_trip(&again);
    assert_eq!(again, out);
    settings(&s).bind(|| assert_snapshot!("challenge_new", record(&s, &o, false)));
    // Without a seed, a fresh 40-bit one: still a short code.
    let fresh: ChallengeNewOut = round_trip(&run(&["challenge", "new", "--level", "5"]));
    assert_eq!(fresh.form, "short");
    assert!(fresh.spec.seed < 1 << 40);
}

#[test]
fn a_seed_from_2_pow_40_gets_the_long_code_with_the_fingerprint_prefix() {
    let seed = (1u64 << 40).to_string();
    let out: ChallengeNewOut = round_trip(&run(&["challenge", "new", "--seed", &seed]));
    assert_eq!(out.form, "long");
    let show: ChallengeShowOut = round_trip(&run(&["challenge", "show", &out.code]));
    assert_eq!(show.fingerprint, out.fingerprint);
    assert_eq!(show.engine.as_deref(), Some(out.engine.as_str()));
}

#[test]
fn challenge_show_summarises_the_target_without_the_answer() {
    let s = Scratch::new("challenge_show");
    let code = enth_challenge::ChallengeCode::short(level(3, 7))
        .unwrap()
        .to_string();
    // Lower case and without hyphens reads the same.
    let typed = code.to_lowercase().replace('-', "");
    let o = run(&["challenge", "show", &typed]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: ChallengeShowOut = round_trip(&o);
    assert_eq!(out.code, code);
    assert!(out.reveal.is_none(), "no answer by default");
    assert_ne!(out.randomized, Vec::<String>::new());
    let round = enth_challenge::prepare_round(&level(3, 7)).unwrap();
    // The answer is nowhere: no `reveal`, and the randomized parameters are addresses only.
    let v = json(&o);
    assert!(v["reveal"].is_null());
    assert!(
        v["randomized"]
            .as_array()
            .unwrap()
            .iter()
            .all(serde_json::Value::is_string)
    );
    assert_eq!(
        out.randomized,
        round
            .params()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    settings(&s).bind(|| assert_snapshot!("challenge_show", record(&s, &o, false)));

    // --reveal shows the values and writes the target.
    let target = s.path("target.patch.json");
    let r = run(&["challenge", "show", &code, "--reveal", "--out", &target]);
    let revealed: ChallengeShowOut = round_trip(&r);
    let reveal = revealed.reveal.unwrap();
    assert_eq!(reveal.values.len(), round.params().len());
    let (written, _) = enth_patch::file::load(&std::fs::read_to_string(&target).unwrap())
        .map(|l| (l.patch, l.notes))
        .unwrap();
    let mut want = round.target().clone();
    written.engine.clone_into(&mut want.engine);
    assert_eq!(written, want);
}

#[test]
fn bad_codes_are_refused_with_their_reason() {
    let good = enth_challenge::ChallengeCode::short(level(2, 42))
        .unwrap()
        .to_string();
    let last = good.chars().last().unwrap();
    let other = if last == '0' { '1' } else { '0' };
    let bad_check = format!("{}{other}", &good[..good.len() - 1]);
    let bad_symbol = format!("{}*", &good[..good.len() - 1]);
    for (text, code) in [
        ("", "empty_code"),
        (bad_symbol.as_str(), "bad_symbol"),
        (bad_check.as_str(), "bad_check"),
    ] {
        let o = run(&["challenge", "show", text]);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(
            (o.code, e.error.as_str()),
            (3, code),
            "{text}: {}",
            o.stdout
        );
        assert_ne!(e.hint, "");
    }
    let o = run(&["challenge", "new", "--level", "9"]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!((o.code, e.error.as_str()), (3, "out_of_range"));
}

#[test]
fn compare_scores_a_patch_against_a_code_with_score_round() {
    let s = Scratch::new("compare");
    let code = enth_challenge::ChallengeCode::short(level(1, 42))
        .unwrap()
        .to_string();
    let target = s.path("target.patch.json");
    assert_eq!(
        run(&["challenge", "show", &code, "--reveal", "--out", &target]).code,
        0
    );
    // The target itself passes with 100 %: exit 0.
    let o = run(&["compare", &target, "--code", &code]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let out: CompareOut = round_trip(&o);
    assert_eq!(out.passed, Some(true));
    assert_eq!(out.sound, Some(100.0));
    assert_eq!(out.mode, "challenge");
    assert_eq!(out.distance, 0.0, "the target is at distance 0 from itself");
    assert_eq!(
        out.report.as_ref().unwrap()["schema"],
        "enth.score-report/1"
    );
    // What score_round gives for the same patch and challenge, field for field.
    let round = enth_challenge::prepare_round(&level(1, 42)).unwrap();
    let start = s.path("start.patch.json");
    enth_cli_write(&start, round.start());
    let o = run(&["compare", &start, "--code", &code]);
    assert_eq!(o.code, 1, "the start does not pass: exit 1\n{}", o.stdout);
    let out: CompareOut = round_trip(&o);
    let want = enth_challenge::score_round(&round, round.start()).unwrap();
    // The CLI prints numbers rounded (`out::normalise`).
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    assert!(
        near(out.sound.unwrap(), want.sound.percent),
        "{:?} {}",
        out.sound,
        want.sound.percent
    );
    assert!(near(out.structure.unwrap(), want.structure.percent));
    assert!(near(out.combined.unwrap(), want.combined.percent));
    assert_eq!(out.passed, Some(want.passed));
    assert_eq!(
        out.scorers,
        [
            want.sound.scorer.to_string(),
            want.structure.scorer.to_string(),
            want.combined.scorer.to_string()
        ]
    );
    // A challenge file gives the same score as its code.
    let new: ChallengeNewOut = round_trip(&run(&["challenge", "new", "--code", &code]));
    let file = s.write("c.challenge.json", &new.challenge.to_string());
    let by_file: CompareOut = round_trip(&run(&["compare", &start, "--challenge", &file]));
    assert_eq!(by_file.sound, out.sound);
    assert_eq!(by_file.target.code.as_deref(), Some(code.as_str()));
    assert_eq!(by_file.target.kind, "challenge");
    // Neither a code nor a challenge: a usage error.
    let o = run(&["compare", &start]);
    assert_eq!(json(&o)["exit"], 2, "{}", o.stdout);
}

/// Write a patch file the way `enth` does.
fn enth_cli_write(path: &str, patch: &enth_patch::Patch) {
    let mut p = patch.clone();
    enth_patch::file::ENGINE_VERSION.clone_into(&mut p.engine);
    std::fs::write(path, p.to_json_pretty() + "\n").unwrap();
}

#[test]
fn the_built_commands_are_in_help() {
    let help = run(&["--help"]).stdout;
    for cmd in ["compare", "challenge"] {
        assert!(help.contains(&format!("  {cmd} ")), "{cmd}:\n{help}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// decode, custom settings (lane CL)
// ---------------------------------------------------------------------------------------------------------------

use enth_cli::schema::ChallengeDecodeOut;

#[test]
fn decode_reads_a_short_code_without_regenerating() {
    let s = Scratch::new("challenge_decode");
    let code = enth_challenge::ChallengeCode::short(level(3, 7))
        .unwrap()
        .to_string();
    let typed = code.to_lowercase().replace('-', " ");
    let o = run(&["challenge", "decode", &typed]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let d: ChallengeDecodeOut = round_trip(&o);
    assert_eq!(d.schema, "enth.challenge-decode/1");
    assert_eq!(d.code, code);
    assert_eq!(d.form, "short");
    let preset = d.preset.clone().expect("a level is a preset");
    assert_eq!((preset.id, preset.name.as_str()), (3, "Level 3"));
    assert_eq!(
        (d.spec.seed, d.spec.difficulty, d.spec.generator),
        (7, 50, 1)
    );
    assert_eq!(d.spec.amounts, vec![None; 6]);
    assert_eq!(d.categories.len(), 6);
    assert!(d.engine.is_none() && d.engine_current.is_none() && d.target_prefix.is_none());
    settings(&s).bind(|| assert_snapshot!("challenge_decode", record(&s, &o, false)));
}

#[test]
fn decode_reads_a_long_code_and_its_prefix_matches_new() {
    let seed = (1u64 << 40).to_string();
    let new: ChallengeNewOut = round_trip(&run(&["challenge", "new", "--seed", &seed]));
    let d: ChallengeDecodeOut = round_trip(&run(&["challenge", "decode", &new.code]));
    assert_eq!(d.form, "long");
    assert_eq!(d.engine.as_deref(), Some(new.engine.as_str()));
    assert_eq!(d.engine_current, Some(true));
    assert_eq!(
        d.target_prefix.as_deref(),
        Some(new.fingerprint_prefix.as_str())
    );
    assert_eq!(d.spec, new.spec);
}

#[test]
fn custom_settings_travel_in_a_long_code() {
    let o = run(&[
        "challenge",
        "new",
        "--difficulty",
        "33",
        "--amount",
        "filter=80,effects=0",
        "--musical",
        "60",
        "--seed",
        "5",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let new: ChallengeNewOut = round_trip(&o);
    assert_eq!(new.form, "long");
    assert!(new.preset.is_none());
    assert_eq!(new.spec.difficulty, 33);
    assert_eq!(new.spec.musical, 60);
    assert_eq!(
        new.spec.amounts,
        vec![None, Some(80), None, None, Some(0), None]
    );
    // The code reads back to the same settings, `new --code` gives the same challenge, and `show` regenerates it
    // (a long code carries the target's fingerprint prefix, which is checked).
    let d: ChallengeDecodeOut = round_trip(&run(&["challenge", "decode", &new.code]));
    assert_eq!(d.spec, new.spec);
    assert!(d.preset.is_none());
    assert_eq!(
        d.target_prefix.as_deref(),
        Some(new.fingerprint_prefix.as_str())
    );
    let again: ChallengeNewOut = round_trip(&run(&["challenge", "new", "--code", &new.code]));
    assert_eq!(again, new);
    let shown: ChallengeShowOut = round_trip(&run(&["challenge", "show", &new.code]));
    assert_eq!(shown.fingerprint, new.fingerprint);
    assert!(shown.preset.is_none());
    // The game's own function agrees about the target.
    let spec = enth_challenge::ChallengeCode::decode(&new.code).unwrap();
    let round = enth_challenge::prepare_round(spec.spec()).unwrap();
    assert_eq!(round.fingerprint().to_hex(), new.fingerprint);
}

#[test]
fn a_level_with_an_override_is_custom_and_a_levels_difficulty_is_the_level() {
    let custom: ChallengeNewOut = round_trip(&run(&[
        "challenge",
        "new",
        "--level",
        "2",
        "--musical",
        "50",
        "--seed",
        "3",
    ]));
    assert_eq!(
        (
            custom.form.as_str(),
            custom.preset,
            custom.spec.difficulty,
            custom.spec.musical
        ),
        ("long", None, 25, 50)
    );
    let by_level: ChallengeNewOut =
        round_trip(&run(&["challenge", "new", "--level", "2", "--seed", "3"]));
    let by_difficulty: ChallengeNewOut = round_trip(&run(&[
        "challenge",
        "new",
        "--difficulty",
        "25",
        "--seed",
        "3",
    ]));
    assert_eq!(by_level.preset.as_deref(), Some("Level 2"));
    assert_eq!(by_difficulty.code, by_level.code);
    assert_eq!(by_difficulty.form, "short");
    // Amounts spelled as a percentage and in any case.
    let spelled: ChallengeNewOut = round_trip(&run(&[
        "challenge",
        "new",
        "--amount",
        "Filter=80%",
        "--seed",
        "3",
    ]));
    assert_eq!(spelled.spec.amounts[1], Some(80));
}

#[test]
fn custom_settings_are_checked_and_errors_say_what_is_valid() {
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (
            vec!["challenge", "new", "--amount", "filtr=80"],
            3,
            "unknown_category",
        ),
        (
            vec!["challenge", "new", "--amount", "filter"],
            2,
            "bad_amount",
        ),
        (
            vec!["challenge", "new", "--amount", "filter=high"],
            3,
            "out_of_range",
        ),
        (
            vec!["challenge", "new", "--amount", "filter=101"],
            3,
            "out_of_range",
        ),
        (
            vec!["challenge", "new", "--difficulty", "101"],
            3,
            "out_of_range",
        ),
        (
            vec!["challenge", "new", "--musical", "101"],
            3,
            "out_of_range",
        ),
        (
            vec!["challenge", "new", "--template", "9"],
            3,
            "code_too_new",
        ),
        (
            vec!["challenge", "new", "--generator", "9"],
            3,
            "code_too_new",
        ),
        (
            vec!["challenge", "new", "--generator", "0"],
            3,
            "no_generator",
        ),
        (
            vec!["challenge", "new", "--level", "2", "--difficulty", "10"],
            2,
            "usage",
        ),
        (
            vec![
                "challenge",
                "new",
                "--code",
                "1040-1000-0000-7M",
                "--difficulty",
                "10",
            ],
            2,
            "usage",
        ),
        (
            vec![
                "challenge",
                "new",
                "--code",
                "1040-1000-0000-7M",
                "--amount",
                "filter=1",
            ],
            2,
            "usage",
        ),
        (vec!["challenge", "decode"], 2, "usage"),
        (vec!["challenge", "decode", ""], 3, "empty_code"),
        (
            vec!["challenge", "decode", "1040-1000-0000-7N"],
            3,
            "bad_check",
        ),
    ];
    for (args, exit, error) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(
            (o.code, e.error.as_str()),
            (exit, error),
            "{args:?}: {}",
            o.stdout
        );
        assert_ne!(e.hint, "", "{args:?}");
    }
    let o = run(&["challenge", "new", "--amount", "filtr=80"]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(e.did_you_mean, ["filter"]);
}

#[test]
fn the_new_and_show_outputs_say_their_preset() {
    let s = Scratch::new("challenge_preset");
    let o = run(&["challenge", "new", "--level", "4", "--seed", "11"]);
    let new: ChallengeNewOut = round_trip(&o);
    assert_eq!(new.preset.as_deref(), Some("Level 4"));
    assert_eq!(new.schema, "enth.challenge-new/2");
    let shown: ChallengeShowOut = round_trip(&run(&["challenge", "show", &new.code]));
    assert_eq!(
        (shown.preset.as_deref(), shown.schema.as_str()),
        (Some("Level 4"), "enth.challenge-show/2")
    );
    let custom = run(&["challenge", "new", "--difficulty", "33", "--seed", "5"]);
    settings(&s).bind(|| assert_snapshot!("challenge_new_custom", record(&s, &custom, false)));
}
