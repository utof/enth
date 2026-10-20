//! `enth suggest`: the coach's graded hints. A level shows only what its level allows (the levels before level 4
//! hold no number and no value of the target's), the answer is the same for any worker count, and the errors carry a
//! code and a hint.
#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write scratch files"
)]

mod common;
mod gamekit;

use common::{Scratch, json, round_trip, run};
use enth_cli::Output;
use enth_cli::schema::{ErrorOut, SuggestOut};

/// `enth suggest` of `patch` at `level`, asserting success.
fn suggest(patch: &str, code: &str, level: u8) -> (Output, SuggestOut) {
    let lv = level.to_string();
    let o = run(&["suggest", patch, "--code", code, "--level", &lv]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let s = round_trip::<SuggestOut>(&o);
    (o, s)
}

/// The start patch of a round, written to `file`.
fn start_patch(s: &Scratch, code: &str, file: &str) -> String {
    let round = gamekit::round(code);
    s.write(file, &round.start().to_json_pretty())
}

#[test]
fn the_levels_reveal_in_order() {
    let s = Scratch::new("suggest_levels");
    let code = gamekit::code(1, 7);
    let patch = s.new_patch("init", "p.patch.json");
    let l: Vec<SuggestOut> = (1..=4).map(|n| suggest(&patch, &code, n).1).collect();

    for (i, h) in l.iter().enumerate() {
        assert_eq!(usize::from(h.level), i + 1);
        assert_eq!(h.levels, 4);
        assert_eq!(h.schema, "enth.suggest/1");
        assert_eq!(h.code.as_deref(), Some(code.as_str()));
        assert!(!h.text.is_empty());
        assert_ne!(h.hint, "");
        assert!(!h.close, "init is far from the target: {}", h.text);
    }
    // Level 1: which aspect, in which direction. Nothing else.
    assert!(l[0].aspect.is_some() && l[0].difference.is_some());
    assert!(
        l[0].module.is_none()
            && l[0].param.is_none()
            && l[0].turn.is_none()
            && l[0].range.is_none()
    );
    // Level 2 adds the module, level 3 the parameter (and the direction of the turn, when it has one).
    assert!(l[1].module.is_some() && l[1].param.is_none() && l[1].range.is_none());
    assert!(l[2].module.is_some() && l[2].param.is_some() && l[2].range.is_none());
    // The module is the parameter's owner.
    let owner = l[2].param.as_deref().unwrap().rsplit_once('.').unwrap().0;
    assert_eq!(l[2].module.as_deref(), Some(owner));
    // The same measured fix underlies every level: the aspect and the module do not change between levels.
    assert_eq!(l[0].aspect, l[3].aspect);
    assert_eq!(l[1].module, l[3].module);
    assert_eq!(l[2].param, l[3].param);
    // Level 4 names the parameter and says how far: a range for a knob, the value for a choice.
    assert!(l[3].param.is_some());
    if let Some(r) = &l[3].range {
        assert_eq!(r.len(), 2, "{r:?}");
    }
}

#[test]
fn a_hint_below_level_4_never_prints_the_targets_values() {
    let s = Scratch::new("suggest_leaks");
    // A level 1 and a level 3 challenge: the second randomizes envelopes and the filter too.
    for (level, seed) in [(1, 7), (2, 3), (3, 11)] {
        let code = gamekit::code(level, seed);
        let round = gamekit::round(&code);
        let values = gamekit::target_values(&round);
        assert!(!values.is_empty(), "{code}: no drawn parameters");
        let patch = start_patch(&s, &code, &format!("start-{level}.patch.json"));
        for n in 1..=3 {
            let (o, h) = suggest(&patch, &code, n);
            // No target value as the CLI prints them.
            for (addr, shown) in &values {
                assert!(
                    !o.stdout.contains(&format!("\"{shown}\"")) && !h.text.contains(shown.as_str()),
                    "{code} level {n} prints {addr} = {shown}: {}",
                    o.stdout
                );
            }
            // No number at all but the level and the render count.
            let nums = gamekit::numbers(&json(&o));
            assert!(
                nums.iter()
                    .all(|k| ["level", "levels", "renders"].contains(&k.as_str())),
                "{code} level {n}: numbers under {nums:?}: {}",
                o.stdout
            );
            // And none in the words: with the module ids taken out, a hint has no digit.
            let mut bare = h.text.clone();
            for (_, m) in round.target().modules().chain(round.start().modules()) {
                bare = bare.replace(m.id.as_str(), "");
            }
            assert!(
                !bare.chars().any(|c| c.is_ascii_digit()),
                "{code} level {n} shows a number: {}",
                h.text
            );
            // The fix the solver found is never printed: only the hint's own level of it.
            assert!(h.range.is_none(), "{code} level {n} shows a range");
        }
    }
}

#[test]
fn the_answer_is_the_same_for_any_worker_count() {
    let s = Scratch::new("suggest_workers");
    let code = gamekit::code(2, 3);
    let patch = start_patch(&s, &code, "p.patch.json");
    let base = run(&[
        "suggest",
        &patch,
        "--code",
        &code,
        "--level",
        "3",
        "--workers",
        "1",
    ]);
    assert_eq!(base.code, 0, "{}", base.stdout);
    for workers in ["2", "5"] {
        let o = run(&[
            "suggest",
            &patch,
            "--code",
            &code,
            "--level",
            "3",
            "--workers",
            workers,
        ]);
        assert_eq!(o.stdout, base.stdout, "--workers {workers}");
    }
    let default = run(&["suggest", &patch, "--code", &code, "--level", "3"]);
    assert_eq!(default.stdout, base.stdout, "default workers");
}

#[test]
fn a_patch_that_already_matches_is_close() {
    let s = Scratch::new("suggest_close");
    let code = gamekit::code(1, 7);
    let round = gamekit::round(&code);
    let patch = s.write("target.patch.json", &round.target().to_json_pretty());
    for n in [1, 4] {
        let (_, h) = suggest(&patch, &code, n);
        assert!(h.close, "level {n}: {}", h.text);
        assert!(h.module.is_none() && h.param.is_none() && h.range.is_none());
        assert!(h.hint.contains("nothing measured is off"), "{}", h.hint);
    }
}

#[test]
fn a_challenge_file_works_like_its_code() {
    let s = Scratch::new("suggest_file");
    let code = gamekit::code(1, 7);
    let round = gamekit::round(&code);
    let file = s.write("c.challenge.json", &round.challenge().to_json_pretty());
    let patch = s.new_patch("init", "p.patch.json");
    let by_code = run(&["suggest", &patch, "--code", &code, "--level", "2"]);
    let by_file = run(&["suggest", &patch, "--challenge", &file, "--level", "2"]);
    assert_eq!(by_file.code, 0, "{}", by_file.stdout);
    let (a, b): (SuggestOut, SuggestOut) = (round_trip(&by_code), round_trip(&by_file));
    // The same hint; only the way the challenge was named differs (a seed-only file has no short code to echo).
    assert_eq!(
        (a.aspect, a.difference, a.module),
        (b.aspect, b.difference, b.module)
    );
}

#[test]
fn the_terminal_text_is_the_hint() {
    let s = Scratch::new("suggest_tty");
    let code = gamekit::code(1, 7);
    let patch = s.new_patch("init", "p.patch.json");
    let o = common::run_tty(&["suggest", &patch, "--code", &code, "--level", "2"]);
    assert_eq!(o.code, 0, "{}", o.stdout);
    let (_, h) = suggest(&patch, &code, 2);
    assert_eq!(o.stdout.trim_end(), h.text);
}

#[test]
fn errors_have_a_code_and_a_hint() {
    let s = Scratch::new("suggest_errors");
    let code = gamekit::code(1, 7);
    let patch = s.new_patch("init", "p.patch.json");
    let missing = s.path("nope.patch.json");
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (
            vec!["suggest", &patch, "--code", &code, "--level", "0"],
            2,
            "bad_level",
        ),
        (
            vec!["suggest", &patch, "--code", &code, "--level", "5"],
            2,
            "bad_level",
        ),
        (
            vec!["suggest", &patch, "--code", &code, "--workers", "0"],
            2,
            "bad_workers",
        ),
        (vec!["suggest", &patch], 2, "usage"),
        (
            vec!["suggest", &patch, "--code", &code, "--challenge", &patch],
            2,
            "usage",
        ),
        (vec!["suggest", &patch, "--code", "not-a-code"], 3, ""),
        (vec!["suggest", &patch, "--challenge", &patch], 3, ""),
        (
            vec!["suggest", &missing, "--code", &code],
            5,
            "file_not_found",
        ),
    ];
    for (args, exit, error) in cases {
        let o = run(&args);
        let e: ErrorOut = round_trip(&o);
        assert_eq!(o.code, exit, "{args:?}: {}", o.stdout);
        assert_eq!(e.exit, exit);
        if !error.is_empty() {
            assert_eq!(e.error, error, "{args:?}");
        }
        assert_ne!(e.error, "");
        assert_ne!(e.hint, "", "{args:?}: {}", o.stdout);
    }
}

#[test]
fn snapshots() {
    let s = Scratch::new("suggest_snap");
    let code = gamekit::code(1, 7);
    let patch = s.new_patch("init", "p.patch.json");
    for n in [1, 3, 4] {
        let lv = n.to_string();
        let o = run(&["suggest", &patch, "--code", &code, "--level", &lv]);
        s.settings().bind(|| {
            insta::assert_snapshot!(
                format!("suggest_level{n}"),
                gamekit::record(&s, &o, &["/level", "/levels"], &[])
            );
        });
    }
}
