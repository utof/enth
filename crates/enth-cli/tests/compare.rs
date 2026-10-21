//! `enth compare` against a reference: a challenge's target (the report keeps the target's readings back, as the game
//! does during a round), a reference patch (the game's own scorers on a hand-made round) and a recording (a distance
//! and the descriptors that differ). Every form reports the metric distance `enth match` minimises.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;
mod gamekit;

use common::{Scratch, round_trip, run};
use enth_cli::schema::{CompareOut, ErrorOut};

/// The pluck, and a darker, longer one made of it.
fn pluck_and_dark(s: &Scratch) -> (String, String) {
    let pluck = s.new_patch("pluck_1osc", "pluck.patch.json");
    let dark = s.path("dark.patch.json");
    let o = run(&[
        "apply",
        &pluck,
        "--set",
        "filter1.cutoff=150Hz,filter1.resonance=40%",
        "--out",
        &dark,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    (pluck, dark)
}

fn compare(args: &[&str]) -> (enth_cli::Output, CompareOut) {
    let mut a = vec!["compare"];
    a.extend_from_slice(args);
    let o = run(&a);
    assert!(o.code == 0 || o.code == 1, "{args:?}: {}", o.stdout);
    let out = round_trip::<CompareOut>(&o);
    (o, out)
}

#[test]
fn a_challenge_report_holds_directions_but_not_the_targets_readings() {
    let s = Scratch::new("compare_reveal");
    let code = gamekit::code(1, 7);
    let patch = s.new_patch("init", "p.patch.json");
    let (o, hidden) = compare(&["--code", &code, &patch]);
    assert_eq!((o.code, hidden.mode.as_str()), (1, "challenge"));
    assert_eq!(hidden.passed, Some(false));
    assert!(
        hidden.differences.is_empty(),
        "the game never names the target's readings"
    );
    let report = hidden.report.clone().unwrap();
    let components = report["sound"]["components"].as_array().unwrap();
    assert!(!components.is_empty(), "no components: {report}");
    for c in components {
        assert!(
            c["direction"].is_string() && c["closeness"].is_string(),
            "{c}"
        );
        assert!(
            c.get("target").is_none() && c.get("answer").is_none(),
            "{c}"
        );
    }
    // --reveal brings them back.
    let (_, shown) = compare(&["--code", &code, &patch, "--reveal"]);
    let components = shown.report.unwrap()["sound"]["components"].clone();
    assert!(
        components
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c.get("target").is_some()),
        "{components}"
    );
    // Nothing else changes.
    assert_eq!(
        (shown.sound, shown.distance),
        (hidden.sound, hidden.distance)
    );
    // The distance is the one `match` reports for the same start.
    let start = gamekit::round(&code).start().to_json_pretty();
    let start = s.write("start.patch.json", &start);
    let (_, at_start) = compare(&["--code", &code, &start]);
    let m = run(&[
        "match", "--target", &code, "--budget", "1", "--free", "drawn",
    ]);
    let m: enth_cli::schema::MatchOut = round_trip(&m);
    assert_eq!(Some(at_start.distance), m.start_distance);
}

#[test]
fn a_patch_compared_with_itself_is_perfect() {
    let s = Scratch::new("compare_self");
    let (pluck, _) = pluck_and_dark(&s);
    let (o, out) = compare(&[&pluck, "--target", &pluck]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    assert_eq!(
        (out.mode.as_str(), out.target.kind.as_str()),
        ("patch", "patch")
    );
    assert_eq!(
        (out.sound, out.structure, out.combined),
        (Some(100.0), Some(100.0), Some(100.0))
    );
    assert_eq!((out.passed, out.pass_mark), (Some(true), Some(90)));
    assert_eq!(out.distance, 0.0);
    assert!(out.differences.is_empty(), "{:?}", out.differences);
    assert_eq!(out.scorers, ["sound/1", "structure/1", "combined/1"]);
    assert_eq!(
        out.target.phrase.as_ref().map(|p| p.name.as_str()),
        Some("held")
    );
    assert!(
        out.target.code.is_none() && out.target.fingerprint.is_none() && out.target.perf.is_none()
    );
    // The same call twice gives the same bytes.
    assert_eq!(
        run(&["compare", &pluck, "--target", &pluck]).stdout,
        o.stdout
    );
}

#[test]
fn a_different_patch_fails_and_the_differences_say_how() {
    let s = Scratch::new("compare_different");
    let (pluck, dark) = pluck_and_dark(&s);
    // The reference is the pluck; ours is the dark one: it is darker, and it fails.
    let (o, out) = compare(&[&dark, "--target", &pluck]);
    assert_eq!(o.code, 1, "does not pass: exit 1\n{}", o.stdout);
    assert_eq!(out.passed, Some(false));
    assert!(out.sound.unwrap() < 90.0 && out.distance > 0.0);
    let d = out
        .differences
        .iter()
        .find(|d| d.aspect == "brightness")
        .unwrap_or_else(|| panic!("no brightness difference: {:?}", out.differences));
    assert_eq!(
        (d.direction.as_str(), d.feature.as_str()),
        ("lower", "spectrum.centroid_hz")
    );
    assert!(d.perceptual.contains("darker"), "{}", d.perceptual);
    assert!(d.ours.is_some() && d.target.is_some() && d.units.unwrap() >= 0.5);
    // The most different first; at most eight; `severity` follows the units.
    assert!(out.differences.len() <= 8);
    let units: Vec<f64> = out.differences.iter().filter_map(|d| d.units).collect();
    assert!(units.windows(2).all(|w| w[0] >= w[1]), "{units:?}");
    // The other way round it reads brighter.
    let (_, back) = compare(&[&pluck, "--target", &dark]);
    let b = back
        .differences
        .iter()
        .find(|d| d.aspect == "brightness")
        .unwrap();
    assert_eq!(b.direction, "higher");
    assert!(b.perceptual.contains("brighter"), "{}", b.perceptual);
}

#[test]
fn a_phrase_is_named_by_id_or_name() {
    let s = Scratch::new("compare_phrase");
    let (pluck, _) = pluck_and_dark(&s);
    let by_name = run(&[
        "compare",
        &pluck,
        "--target",
        &pluck,
        "--phrase",
        "octave_pair",
    ]);
    let by_id = run(&["compare", &pluck, "--target", &pluck, "--phrase", "2"]);
    assert_eq!(by_name.code, 0, "{}", by_name.stdout);
    assert_eq!(by_name.stdout, by_id.stdout);
    let out: CompareOut = round_trip(&by_name);
    let phrase = out.target.phrase.unwrap();
    assert_eq!((phrase.id, phrase.name.as_str()), (2, "octave_pair"));
    let o = run(&["compare", &pluck, "--target", &pluck, "--phrase", "octave"]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(
        (o.code, e.error.as_str()),
        (2, "unknown_phrase"),
        "{}",
        o.stdout
    );
    assert!(
        e.did_you_mean.contains(&"octave_pair".to_owned()),
        "{:?}",
        e.did_you_mean
    );
    // The lesson phrases (ids 4-7) resolve too (lane SKL; the F1l review's nit 9).
    let chord = run(&["compare", &pluck, "--target", &pluck, "--phrase", "chord"]);
    let four = run(&["compare", &pluck, "--target", &pluck, "--phrase", "4"]);
    assert_eq!(chord.code, 0, "{}", chord.stdout);
    assert_eq!(chord.stdout, four.stdout);
    let out: CompareOut = round_trip(&chord);
    let phrase = out.target.phrase.unwrap();
    assert_eq!((phrase.id, phrase.name.as_str()), (4, "chord"));
    assert!(
        e.known.contains(&"staccato_run".to_owned()),
        "{:?}",
        e.known
    );
}

#[test]
fn a_recording_gets_a_distance_and_the_differences_but_no_score() {
    let s = Scratch::new("compare_wav");
    let (pluck, dark) = pluck_and_dark(&s);
    let wav = s.path("pluck.wav");
    assert_eq!(run(&["render", &pluck, "--out", &wav]).code, 0);
    // The patch that made it: identical to the sample.
    let (o, same) = compare(&[&pluck, "--target", &wav]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    assert_eq!(
        (same.mode.as_str(), same.target.kind.as_str()),
        ("wav", "wav")
    );
    assert_eq!(same.distance, 0.0);
    assert!(same.differences.is_empty(), "{:?}", same.differences);
    assert!(same.sound.is_none() && same.structure.is_none() && same.combined.is_none());
    assert!(same.passed.is_none() && same.pass_mark.is_none() && same.margin.is_none());
    assert!(same.scorers.is_empty() && same.report.is_none());
    let perf = same.target.perf.clone().expect("the notes that made it");
    assert_eq!(perf.source, "wav");
    // Another patch: a distance, the differences, still exit 0 (there is no pass).
    let (o, other) = compare(&[&dark, "--target", &wav]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    assert!(other.distance > 0.0);
    let b = other
        .differences
        .iter()
        .find(|d| d.aspect == "brightness")
        .expect("brightness differs");
    assert_eq!(b.direction, "lower");
    assert!(other.hint.contains("enth match"), "{}", other.hint);
    // The distance is the one `match` starts from.
    let m = run(&[
        "match",
        "--target",
        &wav,
        "--start",
        &dark,
        "--free",
        "filter1.cutoff",
        "--budget",
        "1",
    ]);
    let m: enth_cli::schema::MatchOut = round_trip(&m);
    assert_eq!(m.start_distance, Some(other.distance));
}

#[test]
fn a_recording_without_its_notes_needs_perf() {
    let s = Scratch::new("compare_wav_perf");
    let (pluck, _) = pluck_and_dark(&s);
    let plain = s.path("plain.wav");
    let mut w = hound::WavWriter::create(
        &plain,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..48_000 {
        w.write_sample((8000.0 * (f64::from(i) * 0.03).sin()) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    let o = run(&["compare", &pluck, "--target", &plain]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(
        (o.code, e.error.as_str()),
        (2, "perf_required"),
        "{}",
        o.stdout
    );
    assert!(e.hint.contains("--perf"));
    let (o, out) = compare(&[
        &pluck,
        "--target",
        &plain,
        "--perf",
        "note=C3,vel=100,hold=0.5s,dur=1s",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    assert_eq!(out.target.perf.unwrap().source, "flag");
    assert!(out.distance > 0.0);
}

#[test]
fn errors_have_a_code_and_a_hint() {
    let s = Scratch::new("compare_errors");
    let (pluck, _) = pluck_and_dark(&s);
    let code = gamekit::code(1, 7);
    let missing = s.path("nope.patch.json");
    let text = s.write("notes.txt", "not a patch");
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (vec!["compare", &pluck], 2, "usage"),
        (
            vec!["compare", &pluck, "--code", &code, "--target", &pluck],
            2,
            "usage",
        ),
        (
            vec!["compare", &pluck, "--code", &code, "--perf", "note=C3"],
            2,
            "perf_fixed",
        ),
        (
            vec!["compare", &pluck, "--target", &pluck, "--perf", "note=C3"],
            2,
            "perf_fixed",
        ),
        (
            vec!["compare", &pluck, "--code", &code, "--phrase", "held"],
            2,
            "phrase_fixed",
        ),
        (
            vec!["compare", &pluck, "--target", &missing],
            5,
            "file_not_found",
        ),
        (
            vec!["compare", &missing, "--target", &pluck],
            5,
            "file_not_found",
        ),
        (vec!["compare", &pluck, "--target", &text], 3, ""),
        (vec!["compare", &pluck, "--code", "not-a-code"], 3, ""),
    ];
    for (args, exit, error) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(o.code, exit, "{args:?}: {}", o.stdout);
        if !error.is_empty() {
            assert_eq!(e.error, error, "{args:?}");
        }
        assert_ne!(e.hint, "", "{args:?}");
    }
}

#[test]
fn the_terminal_summary() {
    let s = Scratch::new("compare_tty");
    let (pluck, dark) = pluck_and_dark(&s);
    let o = common::run_tty(&["compare", &dark, "--target", &pluck]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    for part in [
        "compare ",
        "sound ",
        "distance ",
        "the pass mark 90",
        "brightness",
    ] {
        assert!(o.stdout.contains(part), "{part}: {}", o.stdout);
    }
}

#[test]
fn snapshots() {
    let s = Scratch::new("compare_snap");
    let (pluck, dark) = pluck_and_dark(&s);
    let code = gamekit::code(1, 7);
    let init = s.new_patch("init", "init.patch.json");
    // The report and the differences come from the DSP: the shape is pinned, the findings are not.
    let hide = ["/report", "/differences"];
    let wav = s.path("pluck.wav");
    assert_eq!(run(&["render", &pluck, "--out", &wav]).code, 0);
    let cases = [
        ("compare_challenge", vec!["compare", &init, "--code", &code]),
        ("compare_patch", vec!["compare", &dark, "--target", &pluck]),
        ("compare_wav", vec!["compare", &dark, "--target", &wav]),
        (
            "err_compare_phrase",
            vec!["compare", &pluck, "--target", &pluck, "--phrase", "octave"],
        ),
    ];
    for (name, args) in cases {
        let o = run(&args);
        s.settings().bind(|| {
            insta::assert_snapshot!(name, gamekit::record(&s, &o, &[], &hide));
        });
    }
}
