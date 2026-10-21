//! Lane REV's acceptance (docs/phase-3.md): **the Leitner moves as properties** (fixed seed: `PROPTEST_RNG_SEED` in
//! `.cargo/config.toml`, 256 cases in the gate), **a skill's box follows its rings**, and **the due queue is
//! deterministic for a given progress file**. Plus the `review` field: its JSON, its fixture, and its merge as a join
//! (commutative, associative, idempotent), alone and inside `Progress::merge`.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads the basics pack and a fixture from the repository; the library does no I/O"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use enth_challenge::model::{Start, Target};
use enth_challenge::score::ParamScore;
use enth_challenge::v2::{Draw, DrawAt, DrawItem, DrawValue, DrawValues, InlinePatch};
use enth_challenge::{
    Challenge, Direction, PhraseId, ScorerId, ScoringSpec, StructureScore, Verdict,
};
use enth_learn::pack::{LessonKey, StepId, lesson_path};
use enth_learn::player::StepRecord;
use enth_learn::review::{
    self, BOXES, Day, MAX_ITEMS, MIN_ITEMS, Review, RoundRings, Skill, SkillBox, SkillRing, moved,
};
use enth_learn::{Lesson, LessonBundle, Manifest, Pack, Progress, StepKind, StepProgress};
use enth_patch::{Op, ParamAddr};
use proptest::prelude::*;
use serde_json::Value;

// ---------------------------------------------------------------------------------------------------------------
// Strategies

const SKILLS: [&str; 4] = ["svf.cutoff", "osc.wave", "adsr.attack", "voice.glide"];

fn verdict() -> impl Strategy<Value = Verdict> {
    prop_oneof![
        4 => Just(Verdict::Good),
        4 => Just(Verdict::Near),
        4 => Just(Verdict::Bad),
        1 => Just(Verdict::Unknown),
    ]
}

fn skill() -> impl Strategy<Value = Skill> {
    prop::sample::select(SKILLS.to_vec()).prop_map(|s| Skill::parse(s).unwrap())
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

fn skill_box() -> impl Strategy<Value = SkillBox> {
    (1..=BOXES, 20_700_u32..20_760, 0_u32..6, kept()).prop_map(|(level, day, rounds, extra)| {
        SkillBox {
            level,
            day,
            rounds,
            extra,
        }
    })
}

fn review_field() -> impl Strategy<Value = Review> {
    (
        prop::collection::btree_map(skill(), skill_box(), 0..4),
        // Unreadable entries: under a skill another copy may read, or a key no build reads as a skill.
        prop::collection::btree_map(
            prop::sample::select(vec!["svf.cutoff".to_owned(), "Not A Skill".to_owned()]),
            prop_oneof![
                Just(serde_json::json!({"box": 9})),
                Just(Value::from("high"))
            ],
            0..2,
        ),
        kept(),
    )
        .prop_map(|(skills, mut unread, extra)| {
            unread.retain(|k, _| !skills.keys().any(|s| s.as_str() == k));
            Review {
                skills,
                unread,
                extra,
            }
        })
}

fn rings() -> impl Strategy<Value = RoundRings> {
    (
        prop::collection::vec(
            (skill(), verdict(), any::<bool>()).prop_map(|(skill, verdict, randomized)| {
                SkillRing {
                    skill,
                    verdict,
                    randomized,
                }
            }),
            0..5,
        ),
        any::<bool>(),
    )
        .prop_map(|(rings, assisted)| RoundRings { rings, assisted })
}

fn with_review(review: Review) -> Progress {
    let mut p = Progress::new();
    p.review = review;
    p
}

fn merged<T: Clone>(a: &T, b: &T, merge: impl Fn(&mut T, &T)) -> T {
    let mut m = a.clone();
    merge(&mut m, b);
    m
}

// ---------------------------------------------------------------------------------------------------------------
// The Leitner moves

proptest! {
    #[test]
    fn a_move_stays_in_the_boxes_and_is_one_box_at_most(level in 0_u8..8, v in verdict(), rise in any::<bool>()) {
        let from = level.clamp(1, BOXES);
        let to = moved(level, v, rise);
        prop_assert!((1..=BOXES).contains(&to));
        prop_assert!(to.abs_diff(from) <= 1);
        let expected = match v {
            Verdict::Good if rise => (from + 1).min(BOXES),
            Verdict::Bad => (from - 1).max(1),
            _ => from,
        };
        prop_assert_eq!(to, expected, "Good up, Near stays, Bad down");
    }

    #[test]
    fn a_box_never_rises_when_it_may_not(level in 1..=BOXES, v in verdict()) {
        prop_assert!(moved(level, v, false) <= level);
    }

    #[test]
    fn a_better_ring_never_leaves_a_lower_box(level in 1..=BOXES, rise in any::<bool>()) {
        let good = moved(level, Verdict::Good, rise);
        let near = moved(level, Verdict::Near, rise);
        let bad = moved(level, Verdict::Bad, rise);
        prop_assert!(good >= near && near >= bad);
    }

    /// A skill's box follows its rings: after any sequence of rounds, it is the fold of [`moved`] over them from box
    /// 1, rising only for an unassisted round on a randomized parameter; it counts every round that rang it and keeps
    /// the last one's day.
    #[test]
    fn a_skills_box_follows_its_rings(
        rounds in prop::collection::vec((verdict(), any::<bool>(), any::<bool>(), 0_u32..3), 0..40),
    ) {
        let skill = Skill::parse("svf.cutoff").unwrap();
        let mut review = Review::default();
        let (mut level, mut counted, mut day, mut last) = (1_u8, 0_u32, 20_000_u32, None);
        for (verdict, randomized, assisted, step) in rounds {
            day += step;
            let round = RoundRings {
                rings: vec![SkillRing { skill: skill.clone(), verdict, randomized }],
                assisted,
            };
            review.observe(&round, day);
            if verdict != Verdict::Unknown {
                level = moved(level, verdict, randomized && !assisted);
                counted += 1;
                last = Some(day);
            }
            let b = review.get(&skill);
            prop_assert_eq!(b.level, level);
            prop_assert_eq!(b.rounds, counted);
            prop_assert_eq!(b.day, last.unwrap_or(0));
            prop_assert_eq!(review.skills.contains_key(&skill), last.is_some());
        }
    }

    #[test]
    fn an_assisted_round_never_raises_a_box(r in review_field(), mut round in rings(), today in 20_700_u32..20_800) {
        round.assisted = true;
        let mut after = r.clone();
        after.observe(&round, today);
        for (skill, b) in &after.skills {
            prop_assert!(b.level <= r.get(skill).level, "{skill}");
        }
    }

    #[test]
    fn the_worst_ring_of_a_skill_moves_it(r in review_field(), round in rings(), today in 20_700_u32..20_800) {
        // One ring per skill, its worst verdict, rising when any randomized row may: the same moves.
        let mut one = RoundRings { rings: Vec::new(), assisted: round.assisted };
        for (skill, (verdict, rise)) in round.moves() {
            one.rings.push(SkillRing { skill, verdict, randomized: rise });
        }
        let mut a = r.clone();
        a.observe(&round, today);
        let mut b = r;
        b.observe(&one, today);
        prop_assert_eq!(a, b);
    }
}

#[test]
fn rings_come_from_the_first_submits_parameter_rows_on_the_target() {
    let target = enth_challenge::random::start(0).unwrap();
    let row = |addr: &str, verdict, randomized| ParamScore {
        addr: addr.to_owned(),
        answer_addr: Some(addr.to_owned()),
        score: 0.5,
        verdict,
        direction: Direction::Higher,
        randomized,
    };
    let structure = StructureScore {
        scorer: ScorerId::STRUCTURE_V1,
        percent: 50.0,
        params: vec![
            row("filter1.cutoff", Verdict::Good, true),
            row("osc1.wave", Verdict::Bad, true),
            row("osc2.wave", Verdict::Near, true),
            row("voice.glide", Verdict::Near, false),
            row("lfo1 -> filter1.cutoff", Verdict::Bad, true),
            row("filter1.enabled", Verdict::Bad, true),
            row("filter1.cutoff", Verdict::Unknown, true),
            row("nosuch9.cutoff", Verdict::Bad, true),
        ],
        modules: Vec::new(),
    };
    let rings = RoundRings::from_structure(&structure, &target, false);
    let text: Vec<(String, Verdict, bool)> = rings
        .rings
        .iter()
        .map(|r| (r.skill.to_string(), r.verdict, r.randomized))
        .collect();
    assert_eq!(
        text,
        [
            ("svf.cutoff".to_owned(), Verdict::Good, true),
            ("osc.wave".to_owned(), Verdict::Bad, true),
            ("osc.wave".to_owned(), Verdict::Near, true),
            ("voice.glide".to_owned(), Verdict::Near, false),
        ],
        "routes, enable flags, unknown verdicts and modules the target lacks are not skills"
    );
    let moves = rings.moves();
    assert_eq!(
        moves[&Skill::parse("osc.wave").unwrap()],
        (Verdict::Bad, true),
        "the worst ring"
    );
    assert_eq!(
        moves[&Skill::parse("voice.glide").unwrap()],
        (Verdict::Near, false)
    );
    let mut review = Review::default();
    review.observe(&rings, 20_747);
    let level = |s: &str| review.get(&Skill::parse(s).unwrap()).level;
    assert_eq!(
        (level("svf.cutoff"), level("osc.wave"), level("voice.glide")),
        (2, 1, 1)
    );
    let mut assisted = Review::default();
    assisted.observe(
        &RoundRings {
            assisted: true,
            ..rings
        },
        20_747,
    );
    assert_eq!(
        assisted.get(&Skill::parse("svf.cutoff").unwrap()).level,
        1,
        "an assisted Good holds"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The field: JSON, fixture, merge

proptest! {
    #[test]
    fn review_merge_is_commutative(a in review_field(), b in review_field()) {
        prop_assert_eq!(merged(&a, &b, Review::merge), merged(&b, &a, Review::merge));
    }

    #[test]
    fn review_merge_is_associative(a in review_field(), b in review_field(), c in review_field()) {
        prop_assert_eq!(
            merged(&merged(&a, &b, Review::merge), &c, Review::merge),
            merged(&a, &merged(&b, &c, Review::merge), Review::merge)
        );
    }

    #[test]
    fn review_merge_is_idempotent_and_empty_is_its_identity(a in review_field()) {
        prop_assert_eq!(merged(&a, &a, Review::merge), a.clone());
        prop_assert_eq!(merged(&a, &Review::default(), Review::merge), a.clone());
        prop_assert_eq!(merged(&Review::default(), &a, Review::merge), a);
    }

    #[test]
    fn progress_merge_stays_a_join_with_review(a in review_field(), b in review_field(), c in review_field()) {
        let (a, b, c) = (with_review(a), with_review(b), with_review(c));
        prop_assert_eq!(merged(&a, &b, Progress::merge), merged(&b, &a, Progress::merge));
        prop_assert_eq!(
            merged(&merged(&a, &b, Progress::merge), &c, Progress::merge),
            merged(&a, &merged(&b, &c, Progress::merge), Progress::merge)
        );
        prop_assert_eq!(merged(&a, &a, Progress::merge), a);
    }

    /// The shell applies a round to the file it re-read; a stale copy merged with the result keeps the move.
    #[test]
    fn a_round_survives_a_merge_with_the_file_before_it(a in review_field(), round in rings(), today in 20_760_u32..20_800) {
        let before = with_review(a);
        let mut after = before.clone();
        after.observe(&round, today);
        prop_assert_eq!(merged(&before, &after, Progress::merge), after.clone());
        prop_assert_eq!(merged(&after, &before, Progress::merge), after);
    }

    #[test]
    fn the_field_round_trips_after_lessons(a in review_field()) {
        let p = with_review(a);
        let text = p.to_json();
        prop_assert_eq!(Progress::from_json(&text).unwrap(), p.clone());
        prop_assert!(text.starts_with(r#"{"schema":"enth.progress/1","lessons":{}"#), "{}", text);
        prop_assert_eq!(text.contains(r#""review":"#), !p.review.is_empty(), "omitted while empty");
    }
}

#[test]
fn a_review_value_that_is_not_an_object_is_kept_until_a_readable_one_replaces_it() {
    let mut p =
        Progress::from_json(r#"{"schema":"enth.progress/1","lessons":{},"review":7}"#).unwrap();
    assert!(p.review.is_empty());
    assert_eq!(p.extra["review"], 7);
    assert_eq!(
        p.to_json(),
        r#"{"schema":"enth.progress/1","lessons":{},"review":7}"#
    );
    let round = RoundRings {
        rings: vec![SkillRing {
            skill: Skill::parse("svf.cutoff").unwrap(),
            verdict: Verdict::Good,
            randomized: true,
        }],
        assisted: false,
    };
    p.observe(&round, 20_747);
    assert!(!p.extra.contains_key("review"));
    assert_eq!(
        p.to_json(),
        r#"{"schema":"enth.progress/1","lessons":{},"review":{"skills":{"svf.cutoff":{"box":2,"day":20747,"rounds":1}}}}"#
    );
}

/// `tests/fixtures/progress/v1/review.json`: the field as this build writes it, pinned (regenerate with
/// `cargo test -p enth-learn --test review -- --ignored regenerate`, then read the diff).
fn review_fixture() -> Progress {
    let mut p = Progress::new();
    p.record(&StepRecord {
        lesson: "basics/cutoff".parse().unwrap(),
        step: StepId::new("round-1").unwrap(),
        progress: StepProgress {
            done: true,
            clean: true,
            best: Some(91),
            extra: BTreeMap::new(),
        },
    });
    let ring = |skill: &str, verdict, randomized| SkillRing {
        skill: Skill::parse(skill).unwrap(),
        verdict,
        randomized,
    };
    p.observe(
        &RoundRings {
            rings: vec![
                ring("svf.cutoff", Verdict::Good, true),
                ring("osc.wave", Verdict::Bad, true),
            ],
            assisted: false,
        },
        20_745,
    );
    p.observe(
        &RoundRings {
            rings: vec![
                ring("svf.cutoff", Verdict::Good, true),
                ring("voice.glide", Verdict::Near, false),
            ],
            assisted: false,
        },
        20_747,
    );
    p
}

fn fixture_path() -> PathBuf {
    common::fixture_dir("progress").join("review.json")
}

#[test]
fn the_review_field_is_what_this_build_writes() {
    let p = review_fixture();
    let text = fs::read_to_string(fixture_path()).unwrap();
    assert_eq!(text.trim_end(), p.to_json());
    assert_eq!(Progress::from_json(&text).unwrap(), p);
    let skill = |s: &str| p.review.get(&Skill::parse(s).unwrap());
    assert_eq!(
        (
            skill("svf.cutoff").level,
            skill("svf.cutoff").rounds,
            skill("svf.cutoff").day
        ),
        (3, 2, 20_747)
    );
    assert_eq!(skill("svf.cutoff").due_day(), 20_751, "box 3 waits 4 days");
    assert!(skill("osc.wave").is_due(20_746), "box 1 waits 1 day");
}

#[test]
#[ignore = "rewrites the fixture; run by hand, then read the diff"]
fn regenerate() {
    fs::write(fixture_path(), review_fixture().to_json() + "\n").unwrap();
}

// ---------------------------------------------------------------------------------------------------------------
// The queue

const TODAY: Day = 20_747;

/// A draw over the starter template (template 0): `items` as `(param, set)`.
fn draw(seed: u64, items: &[(&str, Vec<DrawValue>)]) -> Challenge {
    Challenge::new(
        Target::Draw(Draw {
            seed,
            items: items
                .iter()
                .map(|(param, set)| DrawItem {
                    at: DrawAt::Param(param.parse().unwrap()),
                    values: DrawValues::Set(set.clone()),
                })
                .collect(),
        }),
        Start::Template { template: 0 },
        PhraseId(1),
        scoring(),
    )
}

fn scoring() -> ScoringSpec {
    ScoringSpec::with_scorers(
        0,
        ScorerId::SOUND_V1,
        ScorerId::STRUCTURE_V1,
        ScorerId::COMBINED_V1,
        ScorerId::TIME_V1,
    )
}

fn n(v: f32) -> DrawValue {
    DrawValue::Number(v)
}

/// A lesson of Recreate steps (`round-1`, …), one per challenge.
fn lesson(id: &str, challenges: &[Challenge]) -> String {
    let steps: Vec<String> = challenges
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                r#"{{"id":"round-{}","kind":"recreate","text":"text.md","challenge":{}}}"#,
                i + 1,
                c.to_json()
            )
        })
        .collect();
    format!(
        r#"{{"schema":"enth.lesson/1","id":"{id}","title":"{id}","tier":"basic","objective":"By ear.","teaches":["svf.cutoff"],"steps":[{}]}}"#,
        steps.join(",")
    )
}

/// A pack built in code: a lesson per id, each its Recreate steps.
fn bundles(lessons: &[(&str, Vec<Challenge>)]) -> Vec<LessonBundle> {
    let json: Vec<(String, String)> = lessons
        .iter()
        .map(|(id, cs)| ((*id).to_owned(), lesson(id, cs)))
        .collect();
    let refs: Vec<(&str, &str)> = json.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let pack = Pack::from_files(&common::test_files(&refs)).unwrap();
    pack.manifest()
        .lessons
        .iter()
        .map(|id| pack.bundle(id).unwrap())
        .collect()
}

/// Every step of `bundle` done.
fn pass(progress: &mut Progress, bundle: &LessonBundle) {
    for step in &bundle.lesson.steps {
        progress.record(&StepRecord {
            lesson: bundle.key(),
            step: step.id.clone(),
            progress: StepProgress {
                done: true,
                ..StepProgress::default()
            },
        });
    }
}

fn set_box(progress: &mut Progress, skill: &str, level: u8, day: Day) {
    progress.review.skills.insert(
        Skill::parse(skill).unwrap(),
        SkillBox {
            level,
            day,
            rounds: 1,
            extra: BTreeMap::new(),
        },
    );
}

/// The four-lesson pack: cutoff, resonance, wave, and a patch-target lesson that has no sibling.
fn small_pack() -> Vec<LessonBundle> {
    let mut target = enth_challenge::random::start(0).unwrap();
    enth_patch::apply(
        &mut target,
        &Op::SetParam {
            addr: "filter1.cutoff".parse::<ParamAddr>().unwrap(),
            value: 900.0,
        },
    )
    .unwrap();
    let fixed = Challenge::new(
        Target::Patch(InlinePatch::new(target)),
        Start::Template { template: 0 },
        PhraseId(1),
        scoring(),
    );
    bundles(&[
        (
            "cutoff",
            vec![
                draw(
                    1,
                    &[("filter1.cutoff", vec![n(300.0), n(1200.0), n(4000.0)])],
                ),
                draw(
                    2,
                    &[("filter1.cutoff", vec![n(300.0), n(1200.0), n(4000.0)])],
                ),
            ],
        ),
        (
            "resonance",
            vec![draw(
                3,
                &[("filter1.resonance", vec![n(10.0), n(50.0), n(90.0)])],
            )],
        ),
        (
            "wave",
            vec![draw(
                4,
                &[(
                    "osc1.wave",
                    vec![
                        DrawValue::Text("sine".to_owned()),
                        DrawValue::Text("triangle".to_owned()),
                    ],
                )],
            )],
        ),
        ("fixed", vec![fixed]),
    ])
}

#[test]
fn the_queue_is_empty_until_a_lesson_is_passed() {
    let lessons = small_pack();
    let q = review::queue(&Progress::new(), &lessons, TODAY);
    assert!(q.is_empty() && q.due() == 0);
    // A lesson with only a patch target has no sibling to review.
    let mut p = Progress::new();
    pass(&mut p, &lessons[3]);
    assert!(review::queue(&p, &lessons, TODAY).is_empty());
}

#[test]
fn the_weakest_due_skill_comes_first_and_each_item_is_a_sibling() {
    let lessons = small_pack();
    let mut p = Progress::new();
    for b in &lessons {
        pass(&mut p, b);
    }
    // svf.cutoff box 3 rung 4 days ago (due), svf.resonance box 1 rung yesterday (due), osc.wave box 2 today (not).
    set_box(&mut p, "svf.cutoff", 3, TODAY - 4);
    set_box(&mut p, "svf.resonance", 1, TODAY - 1);
    set_box(&mut p, "osc.wave", 2, TODAY);
    let q = review::queue(&p, &lessons, TODAY);
    assert_eq!(q.due(), 2);
    let picked: Vec<(&str, bool, &str)> = q
        .items
        .iter()
        .map(|i| (i.skill.as_str(), i.due, i.lesson.lesson.as_str()))
        .collect();
    assert_eq!(
        picked,
        [
            ("svf.resonance", true, "resonance"),
            ("svf.cutoff", true, "cutoff"),
            ("osc.wave", false, "wave"),
        ],
        "box 1 before box 3; the not-due skill tops the queue up to {MIN_ITEMS}"
    );
    for item in &q.items {
        let bundle = lessons.iter().find(|b| b.key() == item.lesson).unwrap();
        let step = bundle
            .lesson
            .steps
            .iter()
            .find(|s| s.id == item.step)
            .unwrap();
        let StepKind::Recreate { challenge, .. } = &step.kind else {
            panic!("a Recreate step");
        };
        assert_ne!(
            Some(item.seed),
            challenge.draw().map(|d| d.seed),
            "never the step's own round"
        );
        assert_eq!(item.challenge, challenge.with_seed(item.seed).unwrap());
        assert!(review::challenge_skills(&item.challenge).contains(&item.skill));
    }
}

#[test]
fn not_due_skills_wait_once_three_are_due() {
    let lessons = bundles(&[
        (
            "a",
            vec![draw(1, &[("filter1.cutoff", vec![n(300.0), n(4000.0)])])],
        ),
        (
            "b",
            vec![draw(1, &[("filter1.resonance", vec![n(10.0), n(90.0)])])],
        ),
        (
            "c",
            vec![draw(1, &[("filter1.drive", vec![n(0.0), n(24.0)])])],
        ),
        (
            "d",
            vec![draw(1, &[("env1.attack", vec![n(5.0), n(300.0)])])],
        ),
    ]);
    let mut p = Progress::new();
    for b in &lessons {
        pass(&mut p, b);
    }
    set_box(&mut p, "adsr.attack", 1, TODAY);
    let q = review::queue(&p, &lessons, TODAY);
    assert_eq!(q.due(), 3, "three skills no round has rung are due");
    assert_eq!(q.items.len(), 3);
    assert!(q.items.iter().all(|i| i.due));
}

#[test]
fn an_unfinished_lesson_is_not_reviewed() {
    let lessons = small_pack();
    let mut p = Progress::new();
    pass(&mut p, &lessons[0]);
    p.record(&StepRecord {
        lesson: lessons[1].key(),
        step: StepId::new("round-1").unwrap(),
        progress: StepProgress::default(),
    });
    let q = review::queue(&p, &lessons, TODAY);
    let lessons_in: BTreeSet<&str> = q.items.iter().map(|i| i.lesson.lesson.as_str()).collect();
    assert_eq!(lessons_in, BTreeSet::from(["cutoff"]));
    assert_eq!(
        q.items.len(),
        1,
        "one skill, one item: a step is never played twice in a queue"
    );
}

/// `content/lessons/basics/`, as `cargo xtask pack` writes it (the authoring layout read through the same types).
fn basics() -> Vec<LessonBundle> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/lessons/basics");
    let text = fs::read_to_string(dir.join("pack.toml")).unwrap();
    let manifest = Manifest::from_json(&common::toml_as_json(&text)).unwrap();
    let mut files = BTreeMap::new();
    files.insert(
        "CREDITS.md".to_owned(),
        fs::read(dir.join("CREDITS.md")).unwrap(),
    );
    for entry in fs::read_dir(dir.join("patches")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        files.insert(format!("patches/{name}"), fs::read(&path).unwrap());
    }
    for id in &manifest.lessons {
        for entry in fs::read_dir(dir.join("lessons").join(id.as_str())).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name == "lesson.toml" {
                let lesson =
                    Lesson::from_json(&common::toml_as_json(&fs::read_to_string(&path).unwrap()))
                        .unwrap();
                files.insert(lesson_path(id), lesson.to_json().into_bytes());
            } else {
                files.insert(format!("lessons/{id}/{name}"), fs::read(&path).unwrap());
            }
        }
    }
    let pack = Pack::from_files(&common::seal(&manifest, files)).unwrap();
    manifest
        .lessons
        .iter()
        .map(|id| pack.bundle(id).unwrap())
        .collect()
}

/// The acceptance: the same progress file on the same day gives the same queue, read again from its text or not.
#[test]
fn the_due_queue_is_deterministic_for_a_given_progress_file() {
    let lessons = basics();
    let mut p = Progress::new();
    for b in &lessons {
        pass(&mut p, b);
    }
    set_box(&mut p, "svf.cutoff", 4, TODAY - 20);
    set_box(&mut p, "osc.wave", 2, TODAY - 1);
    set_box(&mut p, "svf.resonance", 5, TODAY);
    let text = p.to_json();
    let a = review::queue(&Progress::from_json(&text).unwrap(), &lessons, TODAY);
    let b = review::queue(&Progress::from_json(&text).unwrap(), &lessons, TODAY);
    assert_eq!(a, b);
    assert_eq!(a, review::queue(&p, &lessons, TODAY));
    assert!(
        (MIN_ITEMS..=MAX_ITEMS).contains(&a.items.len()),
        "{}",
        a.items.len()
    );
    let steps: BTreeSet<(&LessonKey, &StepId)> =
        a.items.iter().map(|i| (&i.lesson, &i.step)).collect();
    assert_eq!(steps.len(), a.items.len(), "no step twice");
    // Due skills first, weakest first; a due skill with a box has waited its interval.
    let order: Vec<(bool, u8)> = a.items.iter().map(|i| (!i.due, i.level)).collect();
    assert!(order.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
    assert!(a.skills.iter().all(|s| s.due == (s.due_day <= TODAY)));
    assert!(
        a.items.iter().all(|i| i.skill.as_str() != "svf.resonance"),
        "box 5, rung today: not due"
    );
    // Another day can draw other siblings; the same day never does.
    let later = review::queue(&p, &lessons, TODAY + 30);
    assert_eq!(later, review::queue(&p, &lessons, TODAY + 30));
    assert!(later.due() >= a.due(), "more is due a month later");
}
