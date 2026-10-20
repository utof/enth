//! `enth match`: the search on the command line. The output is a pure function of the target, the start, the free
//! set, the seed and the budget, so it is identical for any worker count; the ops it prints replay with `enth apply`;
//! and every error carries a code and a hint. Budgets are tiny (the probe is about 20 renders): these tests check the
//! plumbing, `enth-solver`'s tests check the search.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;
mod gamekit;

use common::{Scratch, round_trip, run};
use enth_cli::schema::{ErrorOut, MatchOut};

/// The pluck, and a target made of it by moving three parameters.
fn pluck_and_target(s: &Scratch) -> (String, String) {
    let start = s.new_patch("pluck_1osc", "start.patch.json");
    let target = s.path("target.patch.json");
    let o = run(&[
        "apply",
        &start,
        "--set",
        "filter1.cutoff=600Hz,filter1.resonance=40%,env1.release=900ms",
        "--out",
        &target,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    (start, target)
}

const FREE: &str = "filter1.cutoff,filter1.resonance,env1.release";

#[test]
fn the_output_is_the_same_for_any_worker_count() {
    let s = Scratch::new("match_workers");
    let (start, target) = pluck_and_target(&s);
    let args = |workers: Option<&'static str>| {
        let mut a = vec![
            "match", "--target", &target, "--start", &start, "--free", FREE, "--budget", "30",
        ];
        if let Some(w) = workers {
            a.extend(["--workers", w]);
        }
        run(&a)
    };
    let one = args(Some("1"));
    assert_eq!(one.code, 0, "{}", one.stdout);
    let many = args(Some("3"));
    assert_eq!(many.stdout, one.stdout, "--workers 3");
    let default = args(None);
    assert_eq!(default.stdout, one.stdout, "default workers");
    // And the same again: nothing is time or order dependent.
    assert_eq!(args(Some("2")).stdout, one.stdout, "--workers 2");
}

#[test]
fn a_code_is_the_same_for_any_worker_count_too() {
    let code = gamekit::code(1, 7);
    let at = |w: &str| {
        run(&[
            "match",
            "--target",
            &code,
            "--budget",
            "26",
            "--level",
            "1",
            "--workers",
            w,
        ])
    };
    let (one, four) = (at("1"), at("4"));
    assert_eq!(one.code, 0, "{}", one.stdout);
    assert_eq!(four.stdout, one.stdout);
}

#[test]
fn the_search_improves_on_the_start_and_its_ops_replay() {
    let s = Scratch::new("match_replay");
    let (start, target) = pluck_and_target(&s);
    let best = s.path("best.patch.json");
    let o = run(&[
        "match", "--target", &target, "--start", &start, "--free", FREE, "--budget", "40", "--out",
        &best,
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let m: MatchOut = round_trip(&o);

    assert_eq!(m.schema, "enth.match/1");
    assert_eq!(
        (m.target.kind.as_str(), m.free.how.as_str()),
        ("patch", "listed")
    );
    assert_eq!(m.free.count, 3);
    assert_eq!(
        m.free.addrs,
        ["env1.release", "filter1.cutoff", "filter1.resonance"]
    );
    let start_distance = m.start_distance.expect("the start was measured");
    assert!(
        m.distance <= start_distance,
        "{} > {start_distance}",
        m.distance
    );
    assert!(
        m.distance < start_distance,
        "nothing improved: {}",
        o.stdout
    );
    assert!(m.renders <= 40 && m.renders >= 1, "{}", m.renders);
    assert!(m.score.is_none(), "a patch target has no game score");
    assert_eq!(m.wrote.as_deref(), Some(best.as_str()));
    // Only free parameters moved.
    assert!(!m.ops.is_empty());
    for op in &m.ops {
        let addr = op["set"].as_str().unwrap();
        assert!(
            m.free.addrs.iter().any(|a| a == addr),
            "{addr} moved but was not free"
        );
    }
    assert_eq!(m.changes.len(), m.ops.len());

    // The ops replay: `enth apply <start> --ops <ops>` writes the file `--out` wrote.
    let replay = s.path("replay.patch.json");
    let ops = serde_json::to_string(&m.ops).unwrap();
    let o = run(&["apply", &start, "--ops", &ops, "--out", &replay]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    assert_eq!(
        std::fs::read_to_string(&replay).unwrap(),
        std::fs::read_to_string(&best).unwrap()
    );
}

#[test]
fn a_goal_ends_the_search_early() {
    let s = Scratch::new("match_goal");
    let (start, target) = pluck_and_target(&s);
    let o = run(&[
        "match",
        "--target",
        &target,
        "--start",
        &start,
        "--free",
        "filter1.cutoff",
        "--budget",
        "100",
        "--stop-at",
        "0.02",
    ]);
    let m: MatchOut = round_trip(&o);
    assert_eq!(m.stop, "goal", "{}", o.stdout);
    assert!(m.renders < 100, "{}", m.renders);
    assert!(m.distance <= 0.02, "{}", m.distance);
    assert_eq!(m.settings.stop_at, Some(0.02));
    assert_eq!(m.settings.budget, 100);
    // A goal the start already meets ends it after the probe.
    let o = run(&[
        "match",
        "--target",
        &target,
        "--start",
        &start,
        "--free",
        FREE,
        "--budget",
        "400",
        "--stop-at",
        "5",
    ]);
    let m: MatchOut = round_trip(&o);
    assert_eq!(
        (m.stop.as_str(), m.runs, m.ops.len()),
        ("goal", 0, 0),
        "{}",
        o.stdout
    );
}

#[test]
fn a_wav_is_matched_with_the_performance_it_carries() {
    let s = Scratch::new("match_wav");
    let (start, target) = pluck_and_target(&s);
    let wav = s.path("target.wav");
    let o = run(&["render", &target, "--out", &wav]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let o = run(&[
        "match",
        "--target",
        &wav,
        "--start",
        &start,
        "--free",
        "filter1.cutoff",
        "--budget",
        "30",
        "--stop-at",
        "0.02",
    ]);
    let m: MatchOut = round_trip(&o);
    assert_eq!(m.target.kind, "wav");
    assert!(m.score.is_none() && m.target.fingerprint.is_none());
    assert!(m.distance <= m.start_distance.unwrap());
    assert!(m.stop == "goal" || m.stop == "budget", "{}", m.stop);

    // A WAV that does not say how it was played cannot be matched without --perf.
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
    for i in 0..24_000 {
        w.write_sample((8000.0 * (f64::from(i) * 0.03).sin()) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    let o = run(&[
        "match", "--target", &plain, "--start", &start, "--budget", "5",
    ]);
    let e: ErrorOut = round_trip(&o);
    assert_eq!(
        (o.code, e.error.as_str()),
        (2, "perf_required"),
        "{}",
        o.stdout
    );
    assert!(e.hint.contains("--perf"), "{}", e.hint);
    let o = run(&[
        "match",
        "--target",
        &plain,
        "--start",
        &start,
        "--free",
        "filter1.cutoff",
        "--budget",
        "5",
        "--perf",
        "note=C3,vel=100,hold=0.25s,dur=0.5s",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
}

#[test]
fn a_code_reports_the_games_own_score() {
    let code = gamekit::code(1, 7);
    let o = run(&[
        "match", "--target", &code, "--free", "drawn", "--budget", "1",
    ]);
    let m: MatchOut = round_trip(&o);
    assert_eq!(m.target.kind, "code");
    assert!(m.target.fingerprint.is_some());
    assert_eq!(m.start, "challenge start");
    let (score, before) = (m.score.unwrap(), m.start_score.unwrap());
    assert_eq!(score, before, "a budget of one render moves nothing");
    assert!(score.sound > 0.0 && score.sound < 100.0, "{score:?}");
    assert_eq!(score.pass_mark, 90);
    assert!(!score.passed);
    assert_eq!(m.stop, "budget");
    assert!(m.ops.is_empty() && m.changes.is_empty());
}

#[test]
fn the_free_set_is_named_in_four_ways() {
    let code = gamekit::code(2, 3);
    let round = gamekit::round(&code);
    let free = |extra: &[&str]| -> MatchOut {
        let mut a = vec!["match", "--target", &code, "--budget", "1"];
        a.extend_from_slice(extra);
        let o = run(&a);
        assert_eq!(o.code, 0, "{extra:?}: {}", o.stdout);
        round_trip(&o)
    };
    // Observable parameters by default; levels widen the set; drawn is the code's own.
    let default = free(&[]);
    assert_eq!(default.free.how, "observable");
    let (l1, l2, l3) = (
        free(&["--level", "1"]),
        free(&["--level", "2"]),
        free(&["--level", "3"]),
    );
    assert!(l1.free.count < l2.free.count && l2.free.count < l3.free.count);
    assert!(l3.free.count <= default.free.count);
    for l in [&l1, &l2] {
        assert!(
            l.free.addrs.iter().all(|a| default.free.addrs.contains(a)),
            "{l:?}"
        );
    }
    assert_eq!(l1.free.how, "level 1");
    let drawn = free(&["--free", "drawn"]);
    assert_eq!(drawn.free.how, "drawn");
    assert!(!drawn.free.addrs.is_empty());
    for a in &drawn.free.addrs {
        assert!(
            round.params().iter().any(|p| &p.to_string() == a),
            "{a} is not one of the randomized parameters"
        );
    }
    // Module ids and addresses, in any order, duplicates once.
    let listed = free(&["--free", "filter1.cutoff,osc1,filter1.cutoff"]);
    assert_eq!(listed.free.how, "listed");
    assert!(listed.free.addrs.contains(&"filter1.cutoff".to_owned()));
    assert!(listed.free.addrs.iter().any(|a| a.starts_with("osc1.")));
    let mut sorted = listed.free.addrs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, listed.free.addrs, "sorted, each once");
    assert_eq!(
        listed.free.count,
        listed.free.continuous + listed.free.categorical
    );
}

#[test]
fn errors_have_a_code_and_a_hint() {
    let s = Scratch::new("match_errors");
    let (start, target) = pluck_and_target(&s);
    let missing = s.path("nope.wav");
    let missing_patch = s.path("nope.patch.json");
    let code = gamekit::code(1, 7);
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (vec!["match"], 2, "usage"),
        (
            vec!["match", "--target", &target, "--budget", "0"],
            2,
            "bad_budget",
        ),
        (
            vec!["match", "--target", &target, "--stop-at=-1"],
            2,
            "bad_stop_at",
        ),
        (
            vec!["match", "--target", &target, "--max-seconds", "0"],
            2,
            "bad_seconds",
        ),
        (
            vec!["match", "--target", &target, "--workers", "0"],
            2,
            "bad_workers",
        ),
        (
            vec!["match", "--target", &target, "--level", "9"],
            2,
            "bad_level",
        ),
        (
            vec![
                "match", "--target", &target, "--level", "1", "--free", "osc1",
            ],
            2,
            "usage",
        ),
        (
            vec![
                "match",
                "--target",
                &target,
                "--start",
                &start,
                "--free",
                "filter1.cutof",
            ],
            3,
            "",
        ),
        (
            vec![
                "match", "--target", &target, "--start", &start, "--free", "lowpass9",
            ],
            3,
            "",
        ),
        (
            vec![
                "match", "--target", &target, "--start", &start, "--free", "drawn",
            ],
            2,
            "bad_free",
        ),
        (vec!["match", "--target", &missing], 5, "file_not_found"),
        (
            vec!["match", "--target", &target, "--start", &missing_patch],
            5,
            "file_not_found",
        ),
        (vec!["match", "--target", "not-a-code"], 3, ""),
        (vec!["match", "--target", &code, "--perf", "note=C3"], 2, ""),
    ];
    for (args, exit, error) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(o.code, exit, "{args:?}: {}", o.stdout);
        assert_eq!(e.exit, exit, "{args:?}");
        if !error.is_empty() {
            assert_eq!(e.error, error, "{args:?}");
        }
        assert_ne!(e.error, "");
        assert_ne!(e.hint, "", "{args:?}: {}", o.stdout);
    }
    // A typo in a free parameter says what was meant.
    let o = run(&[
        "match",
        "--target",
        &target,
        "--start",
        &start,
        "--free",
        "filter1.cutof",
    ]);
    let e: ErrorOut = round_trip(&o);
    assert!(
        e.did_you_mean.contains(&"filter1.cutoff".to_owned()),
        "{}",
        o.stdout
    );
}

#[test]
fn the_terminal_summary() {
    let s = Scratch::new("match_tty");
    let (start, target) = pluck_and_target(&s);
    let o = common::run_tty(&[
        "match",
        "--target",
        &target,
        "--start",
        &start,
        "--free",
        "filter1.cutoff",
        "--budget",
        "30",
        "--stop-at",
        "0.02",
    ]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    for part in [
        "match patch",
        "free:",
        "distance:",
        "renders",
        "stopped on goal",
        "filter1.cutoff",
    ] {
        assert!(o.stdout.contains(part), "{part}: {}", o.stdout);
    }
}

#[test]
fn snapshots() {
    let s = Scratch::new("match_snap");
    let (start, target) = pluck_and_target(&s);
    let o = run(&[
        "match",
        "--target",
        &target,
        "--start",
        &start,
        "--free",
        "filter1.cutoff",
        "--budget",
        "30",
        "--stop-at",
        "0.02",
    ]);
    s.settings().bind(|| {
        insta::assert_snapshot!(
            "match_patch",
            gamekit::record(
                &s,
                &o,
                &["/settings/budget", "/settings/stop_at"],
                &["/changes", "/ops", "/trace/steps"]
            )
        );
    });
    let code = gamekit::code(1, 7);
    let o = run(&["match", "--target", &code, "--level", "1", "--budget", "1"]);
    s.settings().bind(|| {
        insta::assert_snapshot!("match_code", gamekit::record(&s, &o, &[], &[]));
    });
    let o = run(&["match", "--target", &target, "--budget", "0"]);
    s.settings().bind(|| {
        insta::assert_snapshot!("err_match_bad_budget", gamekit::record(&s, &o, &[], &[]));
    });
}
