//! The golden challenges (lane GC; docs/phase-2.md "Test strategy for the game logic", docs/challenge.md "Golden
//! challenges"): 25 challenge codes under `tests/golden/challenges/`, each with the target's fingerprint and render,
//! and the full score report of three fixed answers (the target, the start, a near miss) under `sound/1`,
//! `structure/1` and `combined/1`, named explicitly. It lives in `enth-golden`, so `golden.yml` runs it on Linux and
//! Windows (debug and release) and under wasmtime on every push, and on macOS weekly.
//!
//! - `golden_challenges_match` recomputes every case and compares it with its pins (numbers within 1e-6).
//! - The other tests read the pins only: they cover the game (every level, the three phrases, both code forms, the
//!   special targets), hold the invariants the plan states (the target scores exactly 100, the start never passes),
//!   and re-check the margin.
//! - The margin assertion and the update path's refusals are tested with deliberately bad inputs, in a scratch directory.
//!
//! The wasm run reads the cases at the path baked in at build time, so it preopens that directory:
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUNNER="/path/to/wasmtime run --dir /path/to/enth/tests/golden" \
//!   cargo test -p enth-golden --target wasm32-wasip1 --test gc_challenges -- --nocapture
//! ```
//!
//! Add or extend a case with `cargo xtask golden --challenges --update` (see `challenges/mod.rs`).

#![allow(
    clippy::print_stdout,
    reason = "prints the bits digest for the native-vs-wasm comparison"
)]

use std::collections::BTreeMap;

use enth_golden::challenges::case::Case;
use enth_golden::challenges::text::Doc;
use enth_golden::challenges::{self, MARGIN, Options};

/// Every case name, with its parsed case file and its pins.
fn pinned() -> Vec<(String, Case, Doc)> {
    let dir = challenges::root();
    let mut names: Vec<String> = std::fs::read_dir(dir.join("cases"))
        .expect("cases/ exists")
        .filter_map(|e| {
            let path = e.expect("a directory entry").path();
            (path.extension()? == "toml").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let case = Case::parse(
                &std::fs::read_to_string(dir.join(format!("cases/{name}.toml"))).expect("a case"),
            )
            .expect("a case parses");
            let doc = Doc::parse(
                &std::fs::read_to_string(dir.join(format!("expected/{name}.txt")))
                    .expect("every case has its pins"),
            )
            .expect("pins parse");
            (name, case, doc)
        })
        .collect()
}

/// `key=value` words of a line.
fn kv(line: &str) -> BTreeMap<&str, &str> {
    line.split_whitespace()
        .filter_map(|w| w.split_once('='))
        .collect()
}

fn header<'a>(doc: &'a Doc, key: &str) -> &'a str {
    doc.header
        .iter()
        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(' ')))
        .unwrap_or_else(|| panic!("no `{key}` line in {:?}", doc.header))
}

/// The `report` line of an answer: sound %, structure %, combined %, passed.
fn report<'a>(doc: &'a Doc, block: usize, answer: &str) -> BTreeMap<&'a str, &'a str> {
    let a = doc.blocks[block]
        .answers
        .iter()
        .find(|a| a.name == answer)
        .unwrap_or_else(|| panic!("no answer {answer}"));
    kv(a.lines
        .iter()
        .find(|l| l.starts_with("report "))
        .expect("a report line"))
}

#[test]
fn golden_challenges_match() {
    let report = challenges::run(&Options::check()).expect("the harness runs");
    println!("{}", report.render().lines().last().unwrap_or_default());
    assert!(report.is_ok(), "\n{}", report.render());
}

/// The set covers what the plan lists: five levels with four seeds each, the three phrases, both code forms, and the
/// four special targets.
#[test]
fn the_cases_cover_the_game() {
    let all = pinned();
    assert!(all.len() >= 24, "{} cases", all.len());
    let mut levels = [0usize; 6];
    let mut phrases = [0usize; 4];
    let (mut short, mut long) = (0, 0);
    let (mut musical0, mut structure, mut max, mut quiet, mut silent) = (0, 0, 0, 0, 0);
    for (name, _, doc) in &all {
        levels[header(doc, "level").parse::<usize>().unwrap()] += 1;
        phrases[header(doc, "phrase").parse::<usize>().unwrap()] += 1;
        match header(doc, "form").split(' ').next().unwrap() {
            "short" => short += 1,
            "long" => long += 1,
            other => panic!("{name}: form {other}"),
        }
        let spec = kv(header(doc, "spec"));
        let amounts: Vec<&str> = spec["amounts"].split(',').collect();
        musical0 += usize::from(spec["musical"] == "0");
        structure += usize::from(amounts[5] != "-" && header(doc, "level") != "5");
        max += usize::from(amounts.iter().all(|a| *a == "100") && spec["difficulty"] == "100");
        let lufs = header(doc, "target").strip_prefix("lufs=").unwrap();
        if lufs == "silent" {
            silent += 1;
        } else {
            let lufs: f64 = lufs.parse().unwrap();
            quiet += usize::from((-70.0..-60.0).contains(&lufs));
        }
    }
    assert!(levels[1..].iter().all(|n| *n >= 4), "levels {levels:?}");
    assert!(
        phrases[1] >= 4 && phrases[2] >= 2 && phrases[3] >= 2,
        "phrases {phrases:?}"
    );
    assert!(short >= 20 && long >= 4, "{short} short, {long} long");
    assert!(
        musical0 >= 1 && structure >= 1 && max >= 1 && quiet >= 1 && silent >= 1,
        "musical 0: {musical0}, structure: {structure}, max: {max}, near-silent: {quiet}, silent: {silent}"
    );
}

/// What the plan states of every pin: the target scores exactly 100 and passes, the start never passes, and both
/// decisions occur among the near misses on every level.
#[test]
fn the_pins_hold_the_plans_invariants() {
    let mut near_passes = [0usize; 6];
    let mut near_fails = [0usize; 6];
    for (name, case, doc) in pinned() {
        assert!(!doc.blocks.is_empty(), "{name}: no scorer set pinned");
        let level: usize = header(&doc, "level").parse().unwrap();
        for (b, block) in doc.blocks.iter().enumerate() {
            let exact = report(&doc, b, "exact");
            for column in ["sound", "structure", "combined"] {
                assert_eq!(
                    exact[column].parse::<f64>().unwrap(),
                    100.0,
                    "{name} {}: the target must score exactly 100 % ({column})",
                    block.key()
                );
            }
            assert_eq!(exact["passed"], "true", "{name}: the target passes");
            let start = report(&doc, b, "start");
            assert_eq!(
                start["passed"],
                "false",
                "{name} {}: the start must fail (sound {})",
                block.key(),
                start["sound"]
            );
            for answer in case.answers.keys() {
                assert!(
                    block.answers.iter().any(|a| &a.name == answer),
                    "{name}: `{answer}` is not pinned under {}",
                    block.key()
                );
            }
            if block.key().starts_with("sound/1 ") && case.answers.contains_key("near") {
                let near = report(&doc, b, "near");
                if near["passed"] == "true" {
                    near_passes[level] += 1;
                } else {
                    near_fails[level] += 1;
                }
            }
        }
    }
    for level in 1..=5 {
        assert!(
            near_passes[level] >= 1 && near_fails[level] >= 1,
            "level {level}: {} near misses pass, {} fail; both decisions should occur on every level",
            near_passes[level],
            near_fails[level]
        );
    }
}

/// The margin, read back from the files: no pinned sound % is within MARGIN of its pass mark, whatever the
/// computation that wrote it said.
#[test]
fn no_pin_is_within_the_margin_of_the_pass_mark() {
    let mut closest = f64::INFINITY;
    for (name, _, doc) in pinned() {
        for (b, block) in doc.blocks.iter().enumerate() {
            let mark: f64 = kv(&block.heading)["pass_mark"].parse().unwrap();
            for answer in &block.answers {
                let sound: f64 = report(&doc, b, &answer.name)["sound"].parse().unwrap();
                let distance = (sound - mark).abs();
                closest = closest.min(distance);
                assert!(
                    distance >= MARGIN,
                    "{name} / {}: sound {sound} is within {MARGIN} of the pass mark {mark}",
                    answer.name
                );
            }
        }
    }
    println!("closest pinned sound % to the pass mark: {closest:.6}");
}

// ---------------------------------------------------------------------------------------------------------------
// The margin and the update path, on deliberately bad input (native only: they write to a scratch directory).

#[cfg(not(target_family = "wasm"))]
mod scratch {
    use std::path::{Path, PathBuf};

    use enth_challenge::score::ScorerId;
    use enth_challenge::score_round;
    use enth_golden::challenges::case::{Case, Recipe};
    use enth_golden::challenges::compute::{self, Scoring, apply_recipe};
    use enth_golden::challenges::text::Doc;
    use enth_golden::challenges::{self, MARGIN, Options, Status};

    const L1: &str = "l1-s1";

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("enth-gc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cases")).expect("create cases/");
        dir
    }

    fn options(dir: &Path, update: bool) -> Options {
        Options {
            dir: dir.to_owned(),
            update,
            only: None,
        }
    }

    fn real_case(name: &str) -> String {
        std::fs::read_to_string(challenges::root().join(format!("cases/{name}.toml")))
            .expect("a real case")
    }

    fn status(report: &challenges::Report, name: &str) -> challenges::Status {
        report
            .cases
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the report"))
            .status
            .clone()
    }

    /// The pulse width offset (percent points above the target's) at which level 1 seed 4's sound % is within `within`
    /// of the pass mark, found by bisection: a near miss placed on purpose where the margin rule must refuse it.
    /// (Pulse width, not pitch: the sound % steps by a few thousandths when a pitch offset crosses a decision in the
    /// analysis, so a pitch bisection can straddle the pass mark without landing inside the band.)
    fn pulse_offset_at_the_pass_mark(case: &Case, within: f64) -> f64 {
        let subject = compute::subject(case).expect("the case decodes");
        let scoring = Scoring::named("v1").expect("v1");
        let (round, _, mark) = compute::round(&subject, &scoring).expect("prepares");
        let mark = f64::from(mark);
        let percent = |x: f64| {
            let mut recipe = Recipe::default();
            recipe.add.insert("osc1.pw".to_owned(), x);
            let patch = apply_recipe(&recipe, &subject).expect("applies");
            score_round(&round, &patch).expect("scores").sound.percent
        };
        // 10 points wider passes (95.5 %), 20 fails (61.7 %); the score falls smoothly in between.
        let (mut lo, mut hi) = (10.0, 20.0);
        assert!(percent(lo) > mark && percent(hi) < mark);
        for _ in 0..50 {
            let mid = f64::midpoint(lo, hi);
            let p = percent(mid);
            if (p - mark).abs() < within {
                return mid;
            }
            if p > mark {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        panic!("no pulse width offset puts the sound % within {within} of the pass mark");
    }

    #[test]
    fn the_margin_refuses_an_answer_too_close_to_the_pass_mark() {
        let real = Case::parse(&real_case("l1-s4")).expect("the case parses");
        let x = pulse_offset_at_the_pass_mark(&real, MARGIN / 4.0);
        let dir = dir("margin");
        let text = format!(
            "description = \"deliberately within the margin\"\ncode = \"{}\"\n\n[answers.near]\nadd = {{ \"osc1.pw\" = {x:?} }}\n",
            real.code
        );
        std::fs::write(dir.join("cases/too-close.toml"), text).expect("write case");

        // A check says so, with the numbers and the way out.
        let report = challenges::run(&options(&dir, false)).expect("runs");
        let Status::Failed(why) = status(&report, "too-close") else {
            panic!("a case within the margin must fail");
        };
        for part in [
            "too-close / near",
            "pass mark 90",
            "Pick another answer",
            "--propose",
        ] {
            assert!(why.contains(part), "missing `{part}` in:\n{why}");
        }

        // An update refuses to pin it: nothing is written.
        if std::env::var_os("CI").is_none() {
            let report = challenges::run(&options(&dir, true)).expect("runs");
            assert!(matches!(status(&report, "too-close"), Status::Failed(_)));
            assert!(!dir.join("expected/too-close.txt").exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same check at the boundary, on a real report: exactly the margin passes, a hair under does not.
    #[test]
    fn the_margin_is_one_thousandth_either_side() {
        let case = Case::parse(&real_case(L1)).expect("the case parses");
        let subject = compute::subject(&case).expect("the case decodes");
        let stub = Scoring::named("stub").expect("stub");
        let (round, _, _) = compute::round(&subject, &stub).expect("prepares");
        let mut report = score_round(&round, round.target()).expect("scores");
        let mark = f64::from(report.pass_mark);
        for (percent, ok) in [
            (mark + MARGIN, true),
            (mark + MARGIN * 0.99, false),
            (mark, false),
            (mark - MARGIN * 0.99, false),
            (mark - MARGIN * 1.01, true),
            (100.0, true),
            (0.0, true),
        ] {
            report.sound.percent = percent;
            report.passed = percent >= mark;
            let got = challenges::margin::check("c", "a", &report);
            assert_eq!(got.is_ok(), ok, "{percent}: {got:?}");
        }
    }

    #[test]
    fn the_update_path_extends_and_refuses() {
        if std::env::var_os("CI").is_some() {
            let dir = dir("ci");
            std::fs::write(dir.join(format!("cases/{L1}.toml")), real_case(L1)).unwrap();
            let why = challenges::run(&options(&dir, true)).expect_err("CI must refuse --update");
            assert!(
                why.contains("refusing to update golden challenges under CI"),
                "{why}"
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let dir = dir("update");
        let case_path = dir.join(format!("cases/{L1}.toml"));
        let expected_path = dir.join(format!("expected/{L1}.txt"));
        std::fs::write(&case_path, real_case(L1)).unwrap();

        // 1. A new case is created, and then holds.
        let created = challenges::run(&options(&dir, true)).expect("runs");
        assert_eq!(status(&created, L1), Status::Created);
        assert!(challenges::run(&options(&dir, false)).unwrap().is_ok());
        // The pins equal the real case's: this build writes what is committed.
        let real =
            std::fs::read_to_string(challenges::root().join(format!("expected/{L1}.txt"))).unwrap();
        let first = std::fs::read_to_string(&expected_path).unwrap();
        assert_eq!(first, real, "a fresh update writes the committed pins");

        // 2. Updating again changes nothing.
        let again = challenges::run(&options(&dir, true)).expect("runs");
        assert_eq!(status(&again, L1), Status::Ok);
        assert_eq!(std::fs::read_to_string(&expected_path).unwrap(), first);

        // 3. A scorer set added to the case is a new block; the old one is kept byte for byte.
        let with_stub =
            real_case(L1).replace("description", "scorings = [\"v1\", \"stub\"]\ndescription");
        std::fs::write(&case_path, with_stub).unwrap();
        let before = challenges::run(&options(&dir, false)).expect("runs");
        let Status::Failed(why) = status(&before, L1) else {
            panic!("an unpinned scorer set must fail a check");
        };
        assert!(
            why.contains("sound-stub/1") && why.contains("not pinned yet"),
            "{why}"
        );
        let extended = challenges::run(&options(&dir, true)).expect("runs");
        let Status::Extended(added) = status(&extended, L1) else {
            panic!("an update must add the stub set");
        };
        assert_eq!(
            added,
            ["scorer set sound-stub/1 structure-stub/1 combined-stub/1 time/1"]
        );
        let second = std::fs::read_to_string(&expected_path).unwrap();
        assert!(
            second.starts_with(first.trim_end()),
            "the old pins stay as they were"
        );
        assert!(challenges::run(&options(&dir, false)).unwrap().is_ok());
        let doc = Doc::parse(&second).unwrap();
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(
            doc.blocks[0].key(),
            format!(
                "{} {} {} {}",
                ScorerId::SOUND_V1,
                ScorerId::STRUCTURE_V1,
                ScorerId::COMBINED_V1,
                ScorerId::TIME_V1
            )
        );

        // 4. A pinned number that moves is refused, in a check and in an update, and the file is left alone.
        let tampered = second.replacen("sound=94.918204140", "sound=94.718204140", 1);
        assert_ne!(tampered, second);
        std::fs::write(&expected_path, &tampered).unwrap();
        let moved = challenges::run(&options(&dir, false)).expect("runs");
        let Status::Failed(why) = status(&moved, L1) else {
            panic!("a moved pin must fail a check");
        };
        assert!(
            why.contains("sound=94.718204140") && why.contains("sound=94.918204140"),
            "{why}"
        );
        let refused = challenges::run(&options(&dir, true)).expect("runs");
        let Status::Failed(why) = status(&refused, L1) else {
            panic!("an update must not move a pin");
        };
        assert!(why.contains("a pinned result never changes"), "{why}");
        assert_eq!(std::fs::read_to_string(&expected_path).unwrap(), tampered);

        // 5. A recipe that changes a pinned answer is refused the same way.
        std::fs::write(&expected_path, &second).unwrap();
        std::fs::write(&case_path, real_case(L1).replace("0.1 }", "0.11 }")).unwrap();
        let edited = challenges::run(&options(&dir, true)).expect("runs");
        assert!(
            matches!(status(&edited, L1), Status::Failed(_)),
            "{:?}",
            status(&edited, L1)
        );
        assert_eq!(std::fs::read_to_string(&expected_path).unwrap(), second);

        // 6. A pin whose case is gone is stale, and is not deleted for you.
        std::fs::copy(&expected_path, dir.join("expected/gone.txt")).unwrap();
        std::fs::write(&case_path, real_case(L1)).unwrap();
        let stale = challenges::run(&options(&dir, true)).expect("runs");
        assert_eq!(stale.stale, ["gone"]);
        assert!(!stale.is_ok());
        assert!(dir.join("expected/gone.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_case_fails_that_case_only() {
        let dir = dir("malformed");
        std::fs::write(dir.join("cases/a-bad.toml"), "code = 5\n").unwrap();
        std::fs::write(
            dir.join("cases/b-badcode.toml"),
            "description = \"x\"\ncode = \"1040-1000-0000-1S\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cases/c-lower.toml"),
            "description = \"x\"\ncode = \"1040-1000-0000-1r\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cases/d-reserved.toml"),
            "description = \"x\"\ncode = \"1040-1000-0000-1R\"\n[answers.exact]\nbase = \"start\"\n",
        )
        .unwrap();
        let report = challenges::run(&options(&dir, false)).expect("runs");
        for (name, part) in [
            ("a-bad", "code"),
            ("b-badcode", "check"),
            ("c-lower", "canonical"),
            ("d-reserved", "not exact or start"),
        ] {
            let Status::Failed(why) = status(&report, name) else {
                panic!("{name} must fail");
            };
            assert!(why.contains(part), "{name}: {why}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
