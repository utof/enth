//! Lane DRL's `drills` progress field (docs/learn.md, "Ear skills, bursts and drills"): its JSON, its fixture, and its
//! merge as a **join** (commutative, associative, idempotent) alone and inside `Progress::merge`, as REV's tests do for
//! `review`; the bounded threshold history; a drill's effect on the file; and that a file without drills keeps its
//! bytes. Fixed seed: `PROPTEST_RNG_SEED` in `.cargo/config.toml`, 256 cases in the gate.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads fixture files from the repository; the library does no I/O"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use enth_learn::Progress;
use enth_learn::drill::{
    Answer, Drill, DrillSpec, Drills, GameDrill, HISTORY_MAX, Ladder, Measured, Point, SkillDrill,
    Summary, Tally, Unit, game,
};
use enth_learn::review::{BOXES, Burst, Day, Skill};
use proptest::prelude::*;
use serde_json::Value;

// ---------------------------------------------------------------------------------------------------------------
// Strategies

const SKILLS: [&str; 5] = [
    "svf.cutoff",
    "osc.wave",
    "ear.interval.M3",
    "ear.chord.min",
    "ear.band.1k",
];

fn skill() -> impl Strategy<Value = Skill> {
    prop::sample::select(SKILLS.to_vec()).prop_map(|s| Skill::parse(s).unwrap())
}

fn unit() -> impl Strategy<Value = Unit> {
    prop::sample::select(Unit::ALL.to_vec())
}

fn point() -> impl Strategy<Value = Point> {
    (unit(), 0_u32..20_000).prop_map(|(unit, milli)| Point { unit, milli })
}

fn kept() -> impl Strategy<Value = BTreeMap<String, Value>> {
    prop::collection::btree_map(
        prop::sample::select(vec!["future".to_owned(), "votes".to_owned()]),
        prop_oneof![
            (0_u8..3).prop_map(Value::from),
            Just(serde_json::json!({"x": 1}))
        ],
        0..2,
    )
}

fn game_drill() -> impl Strategy<Value = GameDrill> {
    (
        1_u8..=12,
        0_u8..4,
        20_700_u32..20_760,
        1_u32..8,
        0_u32..200,
        0_u32..200,
        kept(),
    )
        .prop_map(
            |(level, above, day, sessions, items, right, extra)| GameDrill {
                level,
                best: level + above,
                day,
                sessions,
                items,
                right,
                extra,
            },
        )
}

fn skill_drill() -> impl Strategy<Value = SkillDrill> {
    (
        1_u8..=10,
        20_700_u32..20_760,
        1_u32..8,
        0_u32..200,
        0_u32..200,
        prop::collection::btree_map(20_690_u32..20_760, point(), 0..6),
        kept(),
    )
        .prop_map(
            |(game, day, sessions, items, right, thresholds, extra)| SkillDrill {
                game,
                day,
                sessions,
                items,
                right,
                thresholds,
                extra,
            },
        )
}

/// A `drills` field as a build writes and re-reads it: no kept entry under a key a readable one holds, no kept part
/// beside a readable one.
fn normal(mut d: Drills) -> Drills {
    let games: BTreeSet<String> = d.games.keys().map(u8::to_string).collect();
    d.unread_games.retain(|k, _| !games.contains(k));
    let skills: BTreeSet<String> = d.skills.keys().map(|s| s.as_str().to_owned()).collect();
    d.unread_skills.retain(|k, _| !skills.contains(k));
    if !d.games.is_empty() || !d.unread_games.is_empty() {
        d.extra.remove("games");
    }
    if !d.skills.is_empty() || !d.unread_skills.is_empty() {
        d.extra.remove("skills");
    }
    if !d.days.is_empty() {
        d.extra.remove("days");
    }
    if d.workout_day > 0 {
        d.extra.remove("workout");
    }
    d
}

fn drills_field() -> impl Strategy<Value = Drills> {
    let unreadable = || {
        prop_oneof![
            Just(serde_json::json!({"level": 0})),
            Just(serde_json::json!({"game": 0})),
            Just(Value::from("high")),
            Just(Value::from(7))
        ]
    };
    (
        prop::collection::btree_map(1_u8..=10, game_drill(), 0..4),
        prop::collection::btree_map(skill(), skill_drill(), 0..4),
        prop::collection::btree_set(20_700_u32..20_760, 0..5),
        prop_oneof![Just(0_u32), 20_700_u32..20_760],
        // Entries no build reads: a key that is not a game id, or a readable key with a part that is not the shape.
        prop::collection::btree_map(
            prop::sample::select(vec![
                "3".to_owned(),
                "05".to_owned(),
                "x".to_owned(),
                "300".to_owned(),
            ]),
            unreadable(),
            0..3,
        ),
        prop::collection::btree_map(
            prop::sample::select(vec![
                "svf.cutoff".to_owned(),
                "Not A Skill".to_owned(),
                "ear.chord.min".to_owned(),
            ]),
            unreadable(),
            0..3,
        ),
        // Fields from a newer build, and parts of the wrong shape.
        prop::collection::btree_map(
            prop::sample::select(vec![
                "future".to_owned(),
                "votes".to_owned(),
                "games".to_owned(),
                "skills".to_owned(),
                "days".to_owned(),
                "workout".to_owned(),
            ]),
            prop_oneof![(0_u8..3).prop_map(Value::from), Just(Value::from("later"))],
            0..3,
        ),
    )
        .prop_map(
            |(games, skills, days, workout_day, unread_games, unread_skills, mut extra)| {
                // A number under `workout` would be read as the day (and `0` as none): only text is a part of the wrong shape.
                if let Some(v) = extra.get_mut("workout") {
                    *v = Value::from("later");
                }
                normal(Drills {
                    games,
                    skills,
                    days,
                    workout_day,
                    unread_games,
                    unread_skills,
                    extra,
                })
            },
        )
}

fn with_drills(drills: Drills) -> Progress {
    let mut p = Progress::new();
    p.drills = drills;
    p
}

fn merged<T: Clone>(a: &T, b: &T, merge: impl Fn(&mut T, &T)) -> T {
    let mut m = a.clone();
    merge(&mut m, b);
    m
}

/// A summary of a finished drill of `game`: `items` answered, `right` of them right, bursts and a measure.
fn summary(
    game: u8,
    items: u32,
    right: u32,
    end_level: u8,
    skills: &[(&str, u32, u32)],
    measured: Option<(&str, Point)>,
) -> Summary {
    Summary {
        game,
        items,
        right,
        start_level: 1,
        end_level,
        peak_level: end_level.max(3),
        reversals: 6,
        threshold: None,
        bursts: Vec::new(),
        skills: skills
            .iter()
            .map(|(s, right, total)| {
                (
                    Skill::parse(s).unwrap(),
                    Tally {
                        right: *right,
                        total: *total,
                    },
                )
            })
            .collect(),
        measured: measured.map(|(s, point)| Measured {
            skill: Skill::parse(s).unwrap(),
            point,
        }),
    }
}

fn summary_strategy() -> impl Strategy<Value = Summary> {
    (
        1_u8..=10,
        1_u32..40,
        1_u8..=12,
        prop::collection::btree_map(skill(), (0_u32..6, 1_u32..8), 0..3),
        prop::option::of((skill(), point())),
    )
        .prop_map(|(game, items, end_level, skills, measured)| Summary {
            game,
            items,
            right: items / 2,
            start_level: 1,
            end_level,
            peak_level: end_level + 1,
            reversals: 4,
            threshold: None,
            bursts: Vec::new(),
            skills: skills
                .into_iter()
                .map(|(s, (right, total))| {
                    (
                        s,
                        Tally {
                            right: right.min(total),
                            total,
                        },
                    )
                })
                .collect(),
            measured: measured.map(|(skill, point)| Measured { skill, point }),
        })
}

// ---------------------------------------------------------------------------------------------------------------
// The field: merge, round trip

proptest! {
    #[test]
    fn drills_merge_is_commutative(a in drills_field(), b in drills_field()) {
        prop_assert_eq!(merged(&a, &b, Drills::merge), merged(&b, &a, Drills::merge));
    }

    #[test]
    fn drills_merge_is_associative(a in drills_field(), b in drills_field(), c in drills_field()) {
        prop_assert_eq!(
            merged(&merged(&a, &b, Drills::merge), &c, Drills::merge),
            merged(&a, &merged(&b, &c, Drills::merge), Drills::merge)
        );
    }

    #[test]
    fn drills_merge_is_idempotent_and_empty_is_its_identity(a in drills_field()) {
        prop_assert_eq!(merged(&a, &a, Drills::merge), a.clone());
        prop_assert_eq!(merged(&a, &Drills::default(), Drills::merge), a.clone());
        prop_assert_eq!(merged(&Drills::default(), &a, Drills::merge), a);
    }

    #[test]
    fn progress_merge_stays_a_join_with_drills(a in drills_field(), b in drills_field(), c in drills_field()) {
        let (a, b, c) = (with_drills(a), with_drills(b), with_drills(c));
        prop_assert_eq!(merged(&a, &b, Progress::merge), merged(&b, &a, Progress::merge));
        prop_assert_eq!(
            merged(&merged(&a, &b, Progress::merge), &c, Progress::merge),
            merged(&a, &merged(&b, &c, Progress::merge), Progress::merge)
        );
        prop_assert_eq!(merged(&a, &a, Progress::merge), a);
    }

    /// The shell applies a drill to the file it re-read; a stale copy merged with the result keeps the change.
    #[test]
    fn a_drill_survives_a_merge_with_the_file_before_it(
        a in drills_field(), s in summary_strategy(), today in 20_760_u32..20_800, finish in any::<bool>(),
    ) {
        let before = with_drills(a);
        let mut after = before.clone();
        after.observe_drill(&s, today);
        if finish {
            after.finish_workout(today);
        }
        prop_assert_eq!(merged(&before, &after, Progress::merge), after.clone());
        prop_assert_eq!(merged(&after, &before, Progress::merge), after);
    }

    #[test]
    fn the_field_round_trips_after_lessons(a in drills_field()) {
        let p = with_drills(a);
        let text = p.to_json();
        prop_assert_eq!(Progress::from_json(&text).unwrap(), p.clone());
        prop_assert!(text.starts_with(r#"{"schema":"enth.progress/1","lessons":{}"#), "{}", text);
        prop_assert_eq!(text.contains(r#""drills":"#), !p.drills.is_empty(), "omitted while empty");
    }

    /// The history is the latest `HISTORY_MAX` days of the union of both sides, a day keeping its better measure.
    #[test]
    fn the_history_is_the_latest_days_of_the_union(
        a in prop::collection::btree_map(20_600_u32..20_760, 0_u32..9_000, 0..100),
        b in prop::collection::btree_map(20_600_u32..20_760, 0_u32..9_000, 0..100),
    ) {
        let series = |m: &BTreeMap<Day, u32>| {
            let mut e = SkillDrill::new(3);
            e.sessions = 1;
            e.thresholds = m.iter().map(|(d, v)| (*d, Point { unit: Unit::Semitones, milli: *v })).collect();
            let mut d = Drills::default();
            d.skills.insert(Skill::parse("svf.cutoff").unwrap(), e);
            d
        };
        let m = merged(&series(&a), &series(&b), Drills::merge);
        let got: BTreeMap<Day, u32> = m.skills.values().next().unwrap().thresholds.iter().map(|(d, p)| (*d, p.milli)).collect();
        let mut want: BTreeMap<Day, u32> = a.clone();
        for (d, v) in &b {
            // Semitones: the smaller is the finer difference, the better.
            let e = want.entry(*d).or_insert(*v);
            *e = (*e).min(*v);
        }
        while want.len() > HISTORY_MAX {
            want.pop_first();
        }
        prop_assert_eq!(got, want);
    }

    /// One drill moves a game's level to where its staircase ended, and counts: sessions, items, right answers, the
    /// day, and one more session per skill answered.
    #[test]
    fn a_drill_counts_in_the_field(a in drills_field(), s in summary_strategy(), today in 20_760_u32..20_800) {
        let mut d = a.clone();
        let mut p = with_drills(a.clone());
        p.observe_drill(&s, today);
        d.observe(&s, today);
        prop_assert_eq!(&p.drills, &d);
        let g = d.game(s.game);
        let before = a.game(s.game);
        prop_assert_eq!(g.level, s.end_level);
        prop_assert_eq!(g.sessions, before.sessions + 1);
        prop_assert_eq!(g.items, before.items + s.items);
        prop_assert_eq!(g.right, before.right + s.right);
        prop_assert!(g.best >= before.best.max(s.end_level));
        prop_assert!(d.practised(today));
        for (skill, tally) in &s.skills {
            let was = a.skills.get(skill).map_or((0, 0, 0), |e| (e.sessions, e.items, e.right));
            let now = &d.skills[skill];
            prop_assert_eq!((now.sessions, now.items, now.right), (was.0 + 1, was.1 + tally.total, was.2 + tally.right));
            prop_assert_eq!(now.game, s.game);
        }
    }
}

#[test]
fn a_drill_with_no_answer_changes_nothing() {
    let mut p = Progress::new();
    p.observe_drill(&summary(game::INTERVAL_STACKS, 0, 0, 3, &[], None), 20_747);
    assert!(p.drills.is_empty());
    assert_eq!(p.to_json(), Progress::new().to_json());
}

#[test]
fn a_measure_a_day_keeps_the_better_one_and_a_skill_counts_one_session() {
    let st = |milli| Point {
        unit: Unit::Semitones,
        milli,
    };
    let mut p = Progress::new();
    let cutoff = Skill::parse("svf.cutoff").unwrap();
    p.observe_drill(
        &summary(
            game::EAR_CHECK,
            20,
            16,
            4,
            &[("svf.cutoff", 9, 10)],
            Some(("svf.cutoff", st(2400))),
        ),
        20_740,
    );
    p.observe_drill(
        &summary(
            game::EAR_CHECK,
            20,
            17,
            5,
            &[],
            Some(("svf.cutoff", st(3100))),
        ),
        20_740,
    );
    p.observe_drill(
        &summary(
            game::EAR_CHECK,
            20,
            18,
            6,
            &[("svf.cutoff", 9, 10)],
            Some(("svf.cutoff", st(2100))),
        ),
        20_747,
    );
    let e = &p.drills.skills[&cutoff];
    assert_eq!(
        e.thresholds
            .iter()
            .map(|(d, p)| (*d, p.milli))
            .collect::<Vec<_>>(),
        [(20_740, 2400), (20_747, 2100)],
        "the better of a day's, one a day"
    );
    assert_eq!(e.latest().map(|(d, p)| (d, p.value())), Some((20_747, 2.1)));
    assert_eq!(
        (e.sessions, e.items, e.right),
        (3, 20, 18),
        "a measure alone counts as a session of its own"
    );
    let g = p.drills.game(game::EAR_CHECK);
    assert_eq!(
        (g.level, g.sessions, g.items, g.right, g.day),
        (6, 3, 60, 51, 20_747)
    );
    assert_eq!(
        p.drills.days.iter().copied().collect::<Vec<_>>(),
        [20_740, 20_747]
    );
    assert_eq!(p.drills.month(20_747), [20_740, 20_747]);
    assert_eq!(
        p.drills.month(20_790),
        Vec::<Day>::new(),
        "the calendar shows the month ending today"
    );
}

#[test]
fn a_threshold_becomes_a_value_in_the_ladders_unit() {
    // A drill of 40 answers whose track turned often enough, turned into semitones by a ladder of cutoff steps.
    let ladder = Ladder::new(
        Unit::Semitones,
        vec![12.0, 8.0, 6.0, 4.0, 3.0, 2.0, 1.5, 1.0, 0.7, 0.5],
    )
    .unwrap();
    let mut d = Drill::new(DrillSpec::new(game::DIAL_IT_IN, 10, 40, 7).starting_at(5)).unwrap();
    let mut right = [false, true, true, true].into_iter().cycle();
    while let Some(item) = d.next_item() {
        d.answer(Answer::new(item.index, None, right.next().unwrap()))
            .unwrap();
    }
    let s = d.summary();
    let level = s
        .threshold
        .expect("40 answers on a zig-zag track turn often enough");
    let s = s.measuring(Skill::parse("svf.cutoff").unwrap(), &ladder);
    let m = s.measured.clone().expect("measured");
    assert_eq!(m.point.unit, Unit::Semitones);
    assert!((m.point.value() - ladder.value_at(level)).abs() < 0.001);
    let mut p = Progress::new();
    p.observe_drill(&s, 20_747);
    let (day, point) = p.drills.skills[&m.skill].latest().unwrap();
    assert_eq!((day, point), (20_747, m.point));
}

#[test]
fn bursts_move_the_review_boxes_inside_the_progress_file() {
    let m3 = Skill::parse("ear.interval.M3").unwrap();
    let mut p = Progress::new();
    let mut s = summary(
        game::INTERVAL_STACKS,
        10,
        9,
        4,
        &[("ear.interval.M3", 5, 5)],
        None,
    );
    s.bursts = vec![Burst::new(m3.clone(), 5)];
    p.observe_drill(&s, 20_740);
    assert_eq!(p.review.get(&m3).level, 2, "five right is Good: box 1 to 2");
    s.bursts = vec![Burst::new(m3.clone(), 3)];
    p.observe_drill(&s, 20_742);
    assert_eq!(p.review.get(&m3).level, 2, "three right is Near: it holds");
    s.bursts = vec![Burst::new(m3.clone(), 1)];
    p.observe_drill(&s, 20_744);
    assert_eq!(p.review.get(&m3).level, 1, "one right is Bad: it falls");
    assert!(p.review.get(&m3).level <= BOXES);
    assert_eq!(p.review.get(&m3).rounds, 3);
}

// ---------------------------------------------------------------------------------------------------------------
// Reading: lenient, nothing lost

#[test]
fn a_drills_value_that_is_not_an_object_is_kept_until_a_readable_one_replaces_it() {
    let mut p =
        Progress::from_json(r#"{"schema":"enth.progress/1","lessons":{},"drills":7}"#).unwrap();
    assert!(p.drills.is_empty());
    assert_eq!(
        p.to_json(),
        r#"{"schema":"enth.progress/1","lessons":{},"drills":7}"#
    );
    p.observe_drill(&summary(game::SPOT_THE_CHANGE, 10, 8, 2, &[], None), 20_747);
    assert!(!p.to_json().contains(r#""drills":7"#));
    assert_eq!(
        p.to_json(),
        r#"{"schema":"enth.progress/1","lessons":{},"drills":{"games":{"1":{"level":2,"best":3,"day":20747,"sessions":1,"items":10,"right":8}},"days":[20747]}}"#
    );
}

#[test]
fn unknown_fields_and_unreadable_entries_are_kept_verbatim() {
    let text = r#"{"schema":"enth.progress/1","lessons":{},"drills":{"days":[20740,20747],"future":{"a":[1,2]},"games":{"05":{"level":3},"3":{"best":2},"5":{"best":4,"day":20747,"items":57,"level":3,"note":"x","right":44,"sessions":4}},"skills":{"Not A Skill":{"game":1},"svf.cutoff":{"day":20747,"game":3,"items":80,"right":62,"sessions":2,"thresholds":[[20740,"st",2400],[20747,"st",2100]],"tag":true},"ear.chord.min":{"game":0}},"workout":20747}}"#;
    let p = Progress::from_json(text).unwrap();
    let d = &p.drills;
    assert_eq!(d.games.keys().copied().collect::<Vec<_>>(), [5]);
    assert_eq!(
        d.unread_games
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["05", "3"]
    );
    assert_eq!(d.skills.len(), 1);
    assert_eq!(
        d.unread_skills
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["Not A Skill", "ear.chord.min"]
    );
    assert_eq!(d.workout_day, 20_747);
    assert_eq!(d.extra["future"], serde_json::json!({"a": [1, 2]}));
    assert_eq!(d.games[&5].extra["note"], "x");
    // Written back as it was read (in this build's key order), nothing lost.
    let again: Value = serde_json::from_str(&p.to_json()).unwrap();
    let wanted: Value = serde_json::from_str(text).unwrap();
    assert_eq!(again, wanted);
    assert_eq!(Progress::from_json(&p.to_json()).unwrap(), p);
}

#[test]
fn a_game_key_is_the_plain_number() {
    let p = Progress::from_json(
        r#"{"schema":"enth.progress/1","lessons":{},"drills":{"games":{"+5":{"level":2},"05":{"level":2},"5 ":{"level":2},"0":{"level":2},"10":{"level":2}}}}"#,
    )
    .unwrap();
    assert_eq!(p.drills.games.keys().copied().collect::<Vec<_>>(), [10]);
    assert_eq!(p.drills.unread_games.len(), 4);
}

#[test]
fn a_file_without_drills_keeps_its_bytes() {
    let dir = common::fixture_dir("progress");
    let mut seen = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        if text.contains(r#""drills""#) {
            continue;
        }
        let p = Progress::from_json(&text).unwrap();
        assert!(p.drills.is_empty(), "{}", path.display());
        assert!(!p.to_json().contains("drills"), "{}", path.display());
        seen += 1;
    }
    assert!(seen >= 3, "the existing fixtures were read: {seen}");
}

// ---------------------------------------------------------------------------------------------------------------
// The fixture

/// `tests/fixtures/progress/v1/drills.json`: the field as this build writes it, pinned (regenerate with
/// `cargo test -p enth-learn --test drills -- --ignored regenerate`, then read the diff).
fn drills_fixture() -> Progress {
    let st = |milli| Point {
        unit: Unit::Semitones,
        milli,
    };
    let mut p = Progress::new();
    let m3 = Skill::parse("ear.interval.M3").unwrap();
    let mut s = summary(
        game::INTERVAL_STACKS,
        15,
        12,
        3,
        &[("ear.interval.M3", 5, 5), ("ear.interval.m3", 4, 5)],
        None,
    );
    s.bursts = vec![
        Burst::new(m3, 5),
        Burst::new(Skill::parse("ear.interval.m3").unwrap(), 4),
    ];
    p.observe_drill(&s, 20_745);
    p.observe_drill(
        &summary(
            game::EAR_CHECK,
            20,
            16,
            4,
            &[("svf.cutoff", 8, 10)],
            Some(("svf.cutoff", st(2400))),
        ),
        20_745,
    );
    p.observe_drill(
        &summary(
            game::EAR_CHECK,
            20,
            17,
            5,
            &[("svf.cutoff", 9, 10)],
            Some(("svf.cutoff", st(2100))),
        ),
        20_747,
    );
    p.finish_workout(20_747);
    p
}

fn fixture_path() -> PathBuf {
    common::fixture_dir("progress").join("drills.json")
}

#[test]
fn the_drills_field_is_what_this_build_writes() {
    let p = drills_fixture();
    let text = fs::read_to_string(fixture_path()).unwrap();
    assert_eq!(text.trim_end(), p.to_json());
    assert_eq!(Progress::from_json(&text).unwrap(), p);
    assert_eq!(p.drills.game(game::INTERVAL_STACKS).level, 3);
    assert_eq!(p.drills.game(game::EAR_CHECK).sessions, 2);
    assert_eq!(p.drills.workout_day, 20_747);
    let cutoff = &p.drills.skills[&Skill::parse("svf.cutoff").unwrap()];
    assert_eq!(cutoff.thresholds.len(), 2);
    assert!(
        p.review
            .skills
            .contains_key(&Skill::parse("ear.interval.M3").unwrap()),
        "bursts reached review"
    );
}

#[test]
#[ignore = "rewrites the fixture; run by hand, then read the diff"]
fn regenerate() {
    fs::write(fixture_path(), drills_fixture().to_json() + "\n").unwrap();
}
