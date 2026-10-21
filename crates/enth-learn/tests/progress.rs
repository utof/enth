//! `Progress::merge`'s laws as properties (fixed seed: `PROPTEST_RNG_SEED` in `.cargo/config.toml`, 256 cases in the
//! gate): the union of steps, the max of bests, commutative, associative, idempotent, and it never shrinks either
//! side, kept fields from newer builds included. Plus the JSON round trip and `record` as a one-step merge.

use std::collections::BTreeMap;

use enth_learn::pack::{LessonKey, StepId};
use enth_learn::player::StepRecord;
use enth_learn::progress::LessonProgress;
use enth_learn::{Progress, StepProgress};
use proptest::prelude::*;
use serde_json::Value;

/// Small key spaces, so two progress files share lessons, steps and kept keys often.
const PACKS: [&str; 2] = ["basics", "shaping"];
const LESSONS: [&str; 3] = ["cutoff", "waves", "glide"];
const STEPS: [&str; 3] = ["hear", "try", "which"];
const EXTRA_KEYS: [&str; 2] = ["time_ms", "review"];

/// A kept field's value: any JSON.
fn kept_value() -> impl Strategy<Value = Value> {
    prop_oneof![unreadable_value(), Just(serde_json::json!({"box": 2}))]
}

/// A value no build reads as a step or a lesson (not an object), as an unread entry holds.
fn unreadable_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (0_u8..4).prop_map(Value::from),
        prop::sample::select(vec!["new", "knew"]).prop_map(Value::from),
    ]
}

fn extra() -> impl Strategy<Value = BTreeMap<String, Value>> {
    prop::collection::btree_map(
        prop::sample::select(EXTRA_KEYS.to_vec()).prop_map(str::to_owned),
        kept_value(),
        0..2,
    )
}

fn step() -> impl Strategy<Value = StepProgress> {
    (
        any::<bool>(),
        any::<bool>(),
        prop::option::of(0_u8..=100),
        extra(),
    )
        .prop_map(|(done, clean, best, extra)| StepProgress {
            done: done || clean,
            clean,
            best,
            extra,
        })
}

fn lesson() -> impl Strategy<Value = LessonProgress> {
    (
        prop::collection::btree_map(prop::sample::select(STEPS.to_vec()), step(), 0..3),
        // Unreadable step entries: under a step key (another copy may read it) or a key no build reads.
        prop::collection::btree_map(
            prop::sample::select(vec!["try", "Not-A-Step"]).prop_map(str::to_owned),
            unreadable_value(),
            0..2,
        ),
        extra(),
    )
        .prop_map(|(steps, mut unread, extra)| {
            let steps: BTreeMap<StepId, StepProgress> = steps
                .into_iter()
                .map(|(k, v)| (StepId::new(k).unwrap(), v))
                .collect();
            unread.retain(|k, _| !steps.keys().any(|id| id.as_str() == k));
            LessonProgress {
                steps,
                unread,
                extra,
            }
        })
}

fn progress() -> impl Strategy<Value = Progress> {
    (
        prop::collection::btree_map(
            (
                prop::sample::select(PACKS.to_vec()),
                prop::sample::select(LESSONS.to_vec()),
            ),
            lesson(),
            0..4,
        ),
        prop::collection::btree_map(
            prop::sample::select(vec!["basics/cutoff", "Bad Key"]).prop_map(str::to_owned),
            unreadable_value(),
            0..2,
        ),
        extra(),
        1_u32..=3,
    )
        .prop_map(|(lessons, mut unread, extra, version)| {
            let lessons: BTreeMap<LessonKey, LessonProgress> = lessons
                .into_iter()
                .map(|((p, l), v)| (format!("{p}/{l}").parse().unwrap(), v))
                .collect();
            unread.retain(|k, _| !lessons.keys().any(|key| key.to_string() == *k));
            Progress {
                lessons,
                unread,
                extra,
                version,
            }
        })
}

fn merged(a: &Progress, b: &Progress) -> Progress {
    let mut m = a.clone();
    m.merge(b);
    m
}

/// Whether `big` holds everything `small` does, at least as far along.
fn covers(big: &Progress, small: &Progress) -> bool {
    small.lessons.iter().all(|(key, l)| {
        big.lessons.get(key).is_some_and(|bl| {
            l.steps.iter().all(|(id, s)| {
                bl.steps.get(id).is_some_and(|bs| {
                    (bs.done || !s.done)
                        && (bs.clean || !s.clean)
                        && bs.best >= s.best
                        && s.extra.keys().all(|k| bs.extra.contains_key(k))
                })
            }) && l.extra.keys().all(|k| bl.extra.contains_key(k))
        })
    }) && small.extra.keys().all(|k| big.extra.contains_key(k))
        && big.version >= small.version
}

proptest! {
    #[test]
    fn merge_is_commutative(a in progress(), b in progress()) {
        prop_assert_eq!(merged(&a, &b), merged(&b, &a));
    }

    #[test]
    fn merge_is_associative(a in progress(), b in progress(), c in progress()) {
        prop_assert_eq!(merged(&merged(&a, &b), &c), merged(&a, &merged(&b, &c)));
    }

    #[test]
    fn merge_is_idempotent_and_empty_is_its_identity(a in progress()) {
        prop_assert_eq!(merged(&a, &a), a.clone());
        prop_assert_eq!(merged(&a, &Progress::new()), a.clone());
        prop_assert_eq!(merged(&Progress::new(), &a), a);
    }

    #[test]
    fn merge_never_shrinks_either_side(a in progress(), b in progress()) {
        let m = merged(&a, &b);
        prop_assert!(covers(&m, &a));
        prop_assert!(covers(&m, &b));
        // The union of steps, and the max of bests.
        for (key, l) in a.lessons.iter().chain(b.lessons.iter()) {
            for id in l.steps.keys() {
                let best = |p: &Progress| p.step(key, id).and_then(|s| s.best);
                prop_assert_eq!(best(&m), best(&a).max(best(&b)));
            }
        }
    }

    #[test]
    fn json_round_trips(a in progress()) {
        let text = a.to_json();
        let head = format!(r#"{{"schema":"enth.progress/{}","lessons":"#, a.version);
        prop_assert_eq!(Progress::from_json(&text).unwrap(), a);
        prop_assert!(text.starts_with(&head), "schema first, at the highest version read");
    }

    #[test]
    fn record_is_a_one_step_merge(a in progress(), s in step(), l in 0_usize..3, st in 0_usize..3) {
        let lesson: LessonKey = format!("basics/{}", LESSONS[l]).parse().unwrap();
        let step = StepId::new(STEPS[st]).unwrap();
        let mut by_record = a.clone();
        by_record.record(&StepRecord { lesson: lesson.clone(), step: step.clone(), progress: s.clone() });
        let mut one = Progress::new();
        one.lessons.entry(lesson).or_default().steps.insert(step, s);
        prop_assert_eq!(by_record, merged(&a, &one));
    }
}

#[test]
fn two_writers_keep_each_others_progress() {
    // Two instances read the same file, each records a step, each writes by re-reading and merging.
    let base = Progress::from_json(r#"{"schema":"enth.progress/1","lessons":{}}"#).unwrap();
    let record = |lesson: &str, step: &str, best: Option<u8>| StepRecord {
        lesson: lesson.parse().unwrap(),
        step: StepId::new(step).unwrap(),
        progress: StepProgress {
            done: true,
            clean: true,
            best,
            extra: BTreeMap::new(),
        },
    };
    let mut first = base.clone();
    first.record(&record("basics/cutoff", "try", Some(91)));
    let mut second = base;
    second.record(&record("basics/waves", "hear", None));
    // First writes; second re-reads the file first wrote, merges and writes.
    let on_disk = first.to_json();
    let mut written = Progress::from_json(&on_disk).unwrap();
    written.merge(&second);
    let back = Progress::from_json(&written.to_json()).unwrap();
    assert!(covers(&back, &first) && covers(&back, &second));
}
