//! The hand-made fixtures in `xtask/fixtures/gr2/`, made up (no real person), with the numbers worked out by hand
//! below. Three families (`osc`, taught before the mid-test; `filter` and `fx`, after it), a four-lesson tour across
//! two packs, and four ids:
//!
//! | Id | What is in the files |
//! | --- | --- |
//! | `p1` | the owner, full protocol, pre-test on form A: osc fails then passes (better), filter passes with a hint before (not clean) and scores 75 after 80 (same), fx passes cleanly both times at 90 and 95 (same); a delayed osc item that passes after the limit (not kept); two notes coded specific, one vague, an uncoded debrief thing with no lesson |
//! | `p2` | full protocol, pre-test on form B: osc passes, then is revealed (worse); filter 40 then 55 (better; the post round was restored by the host, and its later `end` line counts); fx with no submit in time, then a clean pass (better); a practice round on a pre-test code later (ignored: the first played round counts); a debrief thing coded specific for cutoff, an uncoded note |
//! | `p3` | the tour only, no round log: one note coded specific, one uncoded |
//! | `p4` | a round log with a test code and no answers file (a finding; not counted) |
//!
//! `cargo xtask gr2-report xtask/fixtures/gr2/people --forms xtask/fixtures/gr2/forms.toml --lessons
//! xtask/fixtures/gr2/lessons` prints `report.txt`; after a deliberate change to the tables, regenerate it with
//! `cargo test -p xtask gr2::fixtures::regenerate -- --ignored` and read the diff.

use std::path::PathBuf;

use super::input::{Coded, Form, Protocol};
use super::numbers::{self, Inputs, Report, Thresholds, Verdict};
use super::{Options, header, load, render};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gr2")
}

fn options() -> Options {
    Options {
        dirs: vec![dir().join("people")],
        forms: dir().join("forms.toml"),
        lessons: dir().join("lessons"),
        owner: "p1".to_owned(),
        thresholds: Thresholds::default(),
        json: false,
        notes: false,
        validate: false,
        strict: false,
    }
}

fn report() -> (Report, String) {
    let o = options();
    let l = load(&o).unwrap();
    let r = numbers::compute(&Inputs {
        forms: &l.forms,
        lessons: &l.lessons,
        tour: &l.tour,
        people: &l.people,
        rounds: &l.rounds,
        owner: &o.owner,
        thresholds: o.thresholds,
    });
    let text = render::render(&r, &header(&l));
    (r, text)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_files_load_with_the_orphan_named_and_nothing_private() {
    let l = load(&options()).unwrap();
    let ids: Vec<&str> = l.people.iter().map(|p| p.answers.id.as_str()).collect();
    assert_eq!(ids, ["p1", "p2", "p3"]);
    assert_eq!(l.tour.len(), 4);
    assert_eq!(l.lessons.len(), 4);
    assert!(l.privacy.is_empty());
    assert!(
        l.findings
            .iter()
            .any(|f| f.place == "p4" && f.message.contains("no answers file"))
    );
    // p2's restored round: two `end` lines, one round.
    assert_eq!(l.rounds.stats.lines, 20);
    assert_eq!(
        l.rounds.rounds.iter().filter(|r| r.player == "p2").count(),
        9
    );
}

#[test]
fn each_item_is_scored_by_hand() {
    let (r, _) = report();
    let p1 = &r.people[0];
    let osc = &p1.families[0];
    let pre = osc.pre.as_ref().unwrap();
    assert!(!pre.clean && close(pre.structure, 62.0) && pre.in_time);
    assert!(osc.post.as_ref().unwrap().clean);
    let delayed = osc.delayed.as_ref().unwrap();
    // A pass at 200 s is after the 180 s limit: not clean; the best structure within the limit is 70.
    assert!(!delayed.clean && close(delayed.structure, 70.0));
    assert_eq!(
        (osc.change, osc.mid_change, osc.kept),
        (Some(1), Some(1), Some(-1))
    );
    let filter = &p1.families[1];
    let fpre = filter.pre.as_ref().unwrap();
    // A pass with a hint is not a clean pass.
    assert!(!fpre.clean && fpre.assisted && close(fpre.structure, 80.0));
    assert_eq!((filter.change, filter.mid_change), (Some(0), Some(0)));
    let fx = &p1.families[2];
    assert_eq!((fx.change, fx.mid_change), (Some(0), None));

    let p2 = &r.people[1];
    assert_eq!(p2.pre_form, Some(Form::B));
    let osc = &p2.families[0];
    // Revealed on the post-test: worse than a clean pre-test pass.
    assert_eq!(osc.change, Some(-1));
    assert!(osc.post.as_ref().unwrap().assisted);
    let filter = &p2.families[1];
    // The pre-test is the first played round (40), not the later practice pass; the post-test is the restored
    // round's later `end` line (55, not 30).
    assert!(close(filter.pre.as_ref().unwrap().structure, 40.0));
    assert!(close(filter.post.as_ref().unwrap().structure, 55.0));
    assert_eq!(filter.change, Some(1));
    let fx = &p2.families[2];
    let fpre = fx.pre.as_ref().unwrap();
    assert!(!fpre.in_time && close(fpre.structure, 0.0));
    assert_eq!((fx.change, fx.mid_change), (Some(1), Some(1)));
}

#[test]
fn the_people_rows_match_the_hand_count() {
    let (r, _) = report();
    let row = |i: usize| &r.people[i];
    // (tour done, lessons done, new, knew, named, uncoded, better, same, worse, missing)
    let got: Vec<_> = (0..3)
        .map(|i| {
            let p = row(i);
            (
                p.tour_done,
                p.lessons_done,
                p.voted_new,
                p.voted_knew,
                p.named.len(),
                p.uncoded,
                p.improved,
                p.same,
                p.worse,
                p.missing,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (3, 3, 2, 2, 2, 0, 1, 2, 0, 0),
            (4, 4, 3, 0, 2, 1, 2, 0, 1, 0),
            (3, 3, 2, 2, 1, 1, 0, 0, 0, 0),
        ]
    );
    assert_eq!(row(0).named, ["basics/cutoff", "basics/waveforms"]);
    assert_eq!(row(1).named, ["basics/cutoff", "basics/waveforms"]);
    assert!(row(0).owner && row(0).passed_after_failing);
    assert!(row(1).passed_after_failing);
    assert_eq!(row(2).protocol, Protocol::Tour);
    assert_eq!(row(2).families.len(), 0);
    assert!(close(row(0).confidence.unwrap(), 11.0 / 3.0));
    assert!(close(row(1).years.unwrap(), 1.5));
}

#[test]
fn the_families_and_the_mid_test_match_the_hand_count() {
    let (r, _) = report();
    // (people, better, same, worse, mean change, mean structure change, pre clean, post clean, before mid, mid people, mid better)
    let f: Vec<_> = r
        .families
        .iter()
        .map(|f| {
            (
                f.people,
                f.improved,
                f.same,
                f.worse,
                f.mean_change.unwrap(),
                f.mean_structure_change.unwrap(),
                f.pre_clean,
                f.post_clean,
                f.taught_before_mid,
                f.mid_people,
                f.mid_improved,
            )
        })
        .collect();
    // osc: +1 and -1; structure (100 - 62) and (40 - 100): mean -11.
    assert_eq!((f[0].0, f[0].1, f[0].2, f[0].3), (2, 1, 0, 1));
    assert!(close(f[0].4, 0.0) && close(f[0].5, -11.0));
    assert_eq!((f[0].6, f[0].7, f[0].8, f[0].9, f[0].10), (1, 1, 1, 2, 1));
    // filter: 0 and +1; (75 - 80) and (55 - 40): mean 5.
    assert_eq!((f[1].0, f[1].1, f[1].2, f[1].3), (2, 1, 1, 0));
    assert!(close(f[1].4, 0.5) && close(f[1].5, 5.0));
    assert_eq!((f[1].6, f[1].7, f[1].8, f[1].9, f[1].10), (0, 0, 0, 1, 0));
    // fx: 0 and +1; (95 - 90) and (92 - 0): mean 48.5.
    assert_eq!((f[2].0, f[2].1, f[2].2, f[2].3), (2, 1, 1, 0));
    assert!(close(f[2].4, 0.5) && close(f[2].5, 48.5));
    assert_eq!((f[2].6, f[2].7, f[2].8, f[2].9, f[2].10), (1, 2, 0, 1, 1));
    // Taught before the mid-test: osc (p1 better, p2 same). Not yet: filter (p1 same), fx (p2 better).
    assert_eq!(
        (
            r.mid.taught_pairs,
            r.mid.taught_improved,
            r.mid.untaught_pairs,
            r.mid.untaught_improved
        ),
        (2, 1, 2, 1)
    );
    // Families better: p1 1, p2 2 -> median 1.5. Both have more better than worse: P(X >= 2 | n = 2) = 1/4.
    assert!(close(r.median_improved.unwrap(), 1.5));
    let s = r.sign_test.as_ref().unwrap();
    assert_eq!((s.better, s.worse, s.ties), (2, 0, 0));
    assert!(close(s.p_one_sided, 0.25));
}

#[test]
fn the_lessons_match_the_hand_count() {
    let (r, _) = report();
    // (key, tour #, done, new, knew, easy, ok, hard, named, notes, ear clean, ear done)
    let got: Vec<_> = r
        .lessons
        .iter()
        .map(|l| {
            (
                l.key.as_str(),
                l.tour,
                l.done,
                l.new,
                l.knew,
                l.too_easy,
                l.ok,
                l.too_hard,
                l.named,
                l.notes,
                l.ear_clean,
                l.ear_done,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("basics/intro", Some(1), 3, 0, 2, 0, 0, 0, 0, 0, 0, 0),
            ("basics/waveforms", Some(2), 3, 2, 1, 1, 1, 1, 2, 3, 2, 3),
            ("basics/cutoff", Some(3), 2, 3, 0, 1, 2, 0, 2, 3, 1, 2),
            ("space/reverb", Some(4), 2, 2, 1, 0, 3, 0, 1, 3, 2, 2),
        ]
    );
}

#[test]
fn the_criteria_match_the_hand_count() {
    let (r, _) = report();
    let v: Vec<_> = r.criteria.iter().map(|c| (c.id, c.verdict)).collect();
    // named: p1 has 2 coded specific and nothing uncoded, so it cannot reach 3: not met. families: median 1.5 < 2.
    // owner: p1 named something and passed osc after failing it.
    assert_eq!(
        v,
        [
            ("named", Verdict::Fail),
            ("families", Verdict::Fail),
            ("owner", Verdict::Pass)
        ]
    );
    assert_eq!(
        r.criteria[0].detail,
        "lessons with a named new thing: p1 2, p2 2 (+1 uncoded), p3 1 (+1 uncoded)"
    );
    assert_eq!(
        r.criteria[1].detail,
        "median families improved 1.5 of 3 over 2 participant(s); no family worse on average"
    );
    assert!(
        r.findings
            .iter()
            .any(|f| f.starts_with("p4 played test codes but has no answers file"))
    );
}

#[test]
fn criteria_wait_for_missing_items_and_uncoded_notes() {
    let o = options();
    let mut l = load(&o).unwrap();
    // p1 codes nothing yet (every note uncoded) and p2 is missing a family: everything waits.
    for n in &mut l.people[0].answers.notes {
        n.coded = None;
    }
    let mut rounds = l.rounds;
    rounds
        .rounds
        .retain(|r| !(r.player == "p2" && r.code_norm.starts_with("10C0")));
    let thresholds = Thresholds {
        lessons_named: 2,
        families_improved: 1,
    };
    let r = numbers::compute(&Inputs {
        forms: &l.forms,
        lessons: &l.lessons,
        tour: &l.tour,
        people: &l.people,
        rounds: &rounds,
        owner: "p1",
        thresholds,
    });
    let v: Vec<_> = r.criteria.iter().map(|c| c.verdict).collect();
    // named: p1 0 coded + 3 uncoded lessons (could reach 2), p3 1 + 1: pending. families: p2 lacks fx: pending.
    // owner: no specific note yet, but three uncoded: pending.
    assert_eq!(v, [Verdict::Pending, Verdict::Pending, Verdict::Pending]);
    assert_eq!(r.people[1].missing, 1);
    assert!(r.criteria[1].detail.contains("1 pre/post pair(s) missing"));
    assert!(
        r.notes
            .iter()
            .filter(|n| n.person == "p1")
            .all(|n| n.coded.is_none())
    );
    assert!(r.notes.iter().any(|n| n.coded == Some(Coded::Specific)));
}

#[test]
fn the_sign_test_is_the_binomial_tail() {
    assert!(close(numbers::sign_p(5, 5), 1.0 / 32.0));
    assert!(close(numbers::sign_p(4, 5), 6.0 / 32.0));
    assert!(close(numbers::sign_p(0, 3), 1.0));
    assert!(close(numbers::sign_p(1, 1), 0.5));
}

#[test]
fn the_report_text_is_the_committed_one() {
    let (_, text) = report();
    let want = std::fs::read_to_string(dir().join("report.txt")).unwrap();
    assert_eq!(
        text, want,
        "the GR2 report's text changed: regenerate xtask/fixtures/gr2/report.txt (see the module docs) and read the diff"
    );
}

#[test]
fn the_json_report_carries_its_schema() {
    let (r, _) = report();
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["schema"], "enth.gr2-report/1");
    assert_eq!(v["people"][1]["families"][0]["change"], -1);
    assert_eq!(v["criteria"][2]["verdict"], "pass");
}

#[test]
#[ignore = "rewrites xtask/fixtures/gr2/report.txt; run on purpose and read the diff"]
fn regenerate() {
    let (_, text) = report();
    std::fs::write(dir().join("report.txt"), text).unwrap();
}
