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
    assert!(out.passed);
    assert_eq!(out.sound, 100.0);
    assert_eq!(out.report["schema"], "enth.score-report/1");
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
        near(out.sound, want.sound.percent),
        "{} {}",
        out.sound,
        want.sound.percent
    );
    assert!(near(out.structure, want.structure.percent));
    assert!(near(out.combined, want.combined.percent));
    assert_eq!(out.passed, want.passed);
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
    assert_eq!(by_file.code.as_deref(), Some(code.as_str()));
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
