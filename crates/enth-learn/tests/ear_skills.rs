//! Lane DRL's additions to Review (docs/learn.md, "Ear skills, bursts and drills"): `ear.*` skills (a dotted path with
//! the case kept, so `ear.interval.M3` and `ear.interval.m3` are two skills), how they print and serialize, and
//! `burst_standings`, the due skills with a box that no Recreate step in the queue can drill.

use enth_learn::Progress;
use enth_learn::review::{self, Burst, Day, EAR_KIND, Queue, Skill, standing};
use proptest::prelude::*;

const TODAY: Day = 20_750;

fn s(text: &str) -> Skill {
    Skill::parse(text).unwrap()
}

#[test]
fn ear_skills_are_dotted_paths_with_the_case_kept() {
    for text in [
        "ear.interval.M3",
        "ear.interval.m3",
        "ear.chord.min",
        "ear.band.1k",
        "ear.progression.i-iv-v-i",
        "ear.note.names",
        "ear.a.b.c",
    ] {
        let skill = Skill::parse(text).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(skill.as_str(), text);
        assert!(skill.is_ear() && skill.kind() == EAR_KIND);
    }
    assert_ne!(
        s("ear.interval.M3"),
        s("ear.interval.m3"),
        "a major and a minor third"
    );
    assert_eq!(Skill::ear("interval.M3"), Some(s("ear.interval.M3")));
    assert_eq!(Skill::new("ear", "chord.min"), Some(s("ear.chord.min")));
    for text in [
        "ear",
        "ear.",
        // One part after `ear.` is refused (should-fix S6): an R2 or R3 build would read `ear.note` as a parameter
        // `note` of a module kind `ear` (two valid lowercase parts), and schedule a module that does not exist.
        "ear.note",
        "ear.x",
        "ear..m3",
        "ear.a.b.c.d",
        "ear.interval M3",
        "ear.interval.é",
        "ear.interval/M3",
        &format!("ear.{}", "x".repeat(33)),
    ] {
        assert_eq!(Skill::parse(text), None, "{text}");
    }
    // The old grammar is as it was: two lowercase parts, so a third part is no parameter skill.
    assert_eq!(Skill::parse("svf.cut.off"), None);
    assert_eq!(Skill::parse("SVF.cutoff"), None);
    assert!(!s("svf.cutoff").is_ear());
}

#[test]
fn ear_skills_print_without_a_table_and_serialize_as_text() {
    assert_eq!(s("ear.interval.M3").label(), "Interval M3");
    assert_eq!(s("ear.chord.min").label(), "Chord min");
    assert_eq!(s("ear.band.1k").label(), "Band 1k");
    assert_eq!(s("ear.rhythm.taps").label(), "Rhythm taps");
    assert!(
        s("svf.cutoff").label().contains("Cutoff"),
        "{}",
        s("svf.cutoff").label()
    );
    let json = serde_json::to_string(&s("ear.interval.M3")).unwrap();
    assert_eq!(json, r#""ear.interval.M3""#);
    assert_eq!(
        serde_json::from_str::<Skill>(&json).unwrap(),
        s("ear.interval.M3")
    );
    assert!(serde_json::from_str::<Skill>(r#""Not A Skill""#).is_err());
    assert!(serde_json::from_str::<Skill>("7").is_err());
}

proptest! {
    #[test]
    fn a_valid_ear_path_survives_text_and_json(path in "[A-Za-z0-9_-]{1,32}(\\.[A-Za-z0-9_-]{1,32}){1,2}") {
        let skill = Skill::ear(&path).unwrap();
        prop_assert_eq!(Skill::parse(skill.as_str()), Some(skill.clone()));
        let json = serde_json::to_string(&skill).unwrap();
        prop_assert_eq!(serde_json::from_str::<Skill>(&json).unwrap(), skill);
    }

    #[test]
    fn nothing_else_is_an_ear_path(path in "[ -~]{0,40}") {
        let ok = (2..=3).contains(&path.split('.').count())
            && path.split('.').all(|p| !p.is_empty() && p.len() <= 32
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
        prop_assert_eq!(Skill::ear(&path).is_some(), ok, "{}", path);
    }
}

#[test]
fn burst_standings_are_the_due_skills_with_a_box_that_the_queue_does_not_hold() {
    let mut p = Progress::new();
    let day = TODAY - 2;
    p.observe_burst(&Burst::new(s("ear.interval.M3"), 1), day); // box 1, due the day after
    p.observe_burst(&Burst::new(s("ear.chord.min"), 5), day); // box 2, due 2 days on
    for _ in 0..2 {
        p.observe_burst(&Burst::new(s("ear.band.1k"), 5), day); // box 3, due 4 days on
    }
    p.observe_burst(&Burst::new(s("svf.cutoff"), 1), day); // a parameter skill, in the queue below
    let empty = review::queue(&Progress::new(), &[], TODAY);
    assert!(empty.is_empty());

    let names = |q: &Queue| -> Vec<String> {
        review::burst_standings(&p, q, TODAY)
            .iter()
            .map(|st| st.skill.to_string())
            .collect()
    };
    let all = names(&empty);
    assert_eq!(
        all.len(),
        3,
        "due ones only: the box-3 skill waits ({all:?})"
    );
    assert_eq!(
        all[2], "ear.chord.min",
        "the lower boxes first; the two box-1 skills tie and take turns by a keyed draw"
    );
    let mut first_two = all[..2].to_vec();
    first_two.sort();
    assert_eq!(first_two, ["ear.interval.M3", "svf.cutoff"]);
    let held = Queue {
        items: Vec::new(),
        skills: vec![standing(&p, &s("svf.cutoff"), TODAY)],
    };
    assert_eq!(
        names(&held),
        ["ear.interval.M3", "ear.chord.min"],
        "a skill the queue holds is a Recreate step's, not a burst's"
    );
    // A skill nobody has rung has no box: the composer starts those from its catalogue, Review does not list them.
    assert!(!names(&empty).contains(&"ear.note.names".to_owned()));
    // Later, the box-3 skill is due too, and the order stays the queue's.
    let later: Vec<String> = review::burst_standings(&p, &empty, TODAY + 10)
        .iter()
        .map(|st| st.skill.to_string())
        .collect();
    assert_eq!(later.len(), 4);
    assert_eq!(
        later.last().map(String::as_str),
        Some("ear.band.1k"),
        "the highest box last: {later:?}"
    );
}

#[test]
fn burst_standings_are_the_same_for_the_same_file_and_day() {
    let mut p = Progress::new();
    for (k, name) in [
        "ear.interval.M3",
        "ear.interval.m3",
        "ear.chord.min",
        "ear.chord.maj",
    ]
    .iter()
    .enumerate()
    {
        p.observe_burst(
            &Burst::new(s(name), 1),
            TODAY - 3 - u32::try_from(k).unwrap(),
        );
    }
    let q = review::queue(&Progress::new(), &[], TODAY);
    let a = review::burst_standings(&p, &q, TODAY);
    let b = review::burst_standings(&Progress::from_json(&p.to_json()).unwrap(), &q, TODAY);
    assert_eq!(a, b);
    assert_eq!(a.len(), 4);
}
