//! Lane REV's shell side: the progress file takes a round's rings into its review boxes under the same lock, re-read
//! and rename as a step's record (`ProgressFile::observe`), each write is counted for the editor's glue
//! (`progress_writes`), and today is a whole UTC day.

#![allow(
    clippy::disallowed_methods,
    reason = "the tests write a temporary progress file"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use enth_challenge::Verdict;
use enth_learn::StepProgress;
use enth_learn::player::StepRecord;
use enth_learn::review::{RoundRings, Skill, SkillRing};

use crate::learn::{PROGRESS_FILE, ProgressFile, progress_writes, today};

fn temp(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "enth-review-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rings(verdict: Verdict) -> RoundRings {
    RoundRings {
        rings: vec![SkillRing {
            skill: Skill::parse("svf.cutoff").unwrap(),
            verdict,
            randomized: true,
        }],
        assisted: false,
    }
}

#[test]
fn a_rounds_rings_move_the_files_box_and_records_keep_merging() {
    let dir = temp("observe");
    let file = ProgressFile::new(Some(dir.join(PROGRESS_FILE)));
    let before = progress_writes();
    file.record(&StepRecord {
        lesson: "basics/cutoff".parse().unwrap(),
        step: "round-1".parse().unwrap(),
        progress: StepProgress {
            done: true,
            ..StepProgress::default()
        },
    })
    .unwrap();
    file.observe(&rings(Verdict::Good), 20_747).unwrap();
    file.observe(&rings(Verdict::Good), 20_749).unwrap();
    assert!(progress_writes() >= before + 3, "every write is counted");
    let skill = Skill::parse("svf.cutoff").unwrap();
    let b = file.load().review.get(&skill);
    assert_eq!((b.level, b.day, b.rounds), (3, 20_749, 2));
    // Another instance's record merges into the same file and keeps the boxes.
    let other = ProgressFile::new(Some(dir.join(PROGRESS_FILE)));
    other
        .record(&StepRecord {
            lesson: "basics/cutoff".parse().unwrap(),
            step: "round-2".parse().unwrap(),
            progress: StepProgress::default(),
        })
        .unwrap();
    let p = file.load();
    assert_eq!(p.review.get(&skill).level, 3);
    assert_eq!(
        p.lesson(&"basics/cutoff".parse().unwrap())
            .unwrap()
            .steps
            .len(),
        2
    );
    let text = fs::read_to_string(dir.join(PROGRESS_FILE)).unwrap();
    assert!(text.contains(r#""svf.cutoff""#), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_learn_folder_drops_the_move() {
    let file = ProgressFile::new(None);
    file.observe(&rings(Verdict::Bad), 20_747).unwrap();
    assert!(file.load().review.is_empty());
}

#[test]
fn today_is_a_whole_utc_day() {
    let day = today();
    assert!(day > 20_000, "{day}: after 2024-10");
    assert!(day < 40_000, "{day}");
}
