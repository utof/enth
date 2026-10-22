//! Lane DRL's workout composer (docs/phase-4.md): **the same workout for the same progress file and day** (seeds
//! included), games chosen by due skills with a varied vehicle, at most three of them, a warm-up one level down after a
//! break, each category met alone before confusable ones mix, note naming never in the default workout, and a workout
//! that *ends* ("done for today") with "one more" on request. A property drives the whole loop (compose, play,
//! observe, finish) and checks the rules at every step.

use std::collections::{BTreeMap, BTreeSet};

use enth_learn::Progress;
use enth_learn::drill::{Answer, BREAK_DAYS, Drill, MAX_ITEMS, MAX_SEED, Summary, game};
use enth_learn::review::{BURST_ITEMS, Burst, Day, Skill};
use enth_learn::workout::{Catalog, GameInfo, MAX_FOCUS, MAX_SEGMENTS, Segment, compose, one_more};
use proptest::prelude::*;

const TODAY: Day = 20_750;

fn s(text: &str) -> Skill {
    Skill::parse(text).unwrap()
}

fn params() -> Vec<Skill> {
    vec![
        s("svf.cutoff"),
        s("osc.wave"),
        s("adsr.attack"),
        s("voice.glide"),
    ]
}

/// A catalogue like the shell's: the parameter games over parameter skills, the music games over `ear.*` skills, the
/// opt-in note naming, and one confusable pair.
fn catalog() -> Catalog {
    Catalog::new(vec![
        GameInfo::new(game::SPOT_THE_CHANGE, "Spot the change", 12, 15, params()),
        GameInfo::new(game::DIAL_IT_IN, "Dial it in", 10, 12, params()),
        GameInfo::new(
            game::EAR_CHECK,
            "Ear check",
            12,
            20,
            vec![s("svf.cutoff"), s("adsr.attack")],
        ),
        GameInfo::new(
            game::FREQUENCY_FINDER,
            "Frequency finder",
            10,
            12,
            vec![s("svf.cutoff")],
        ),
        GameInfo::new(
            game::INTERVAL_STACKS,
            "Interval stacks",
            12,
            15,
            vec![
                s("ear.interval.m3"),
                s("ear.interval.M3"),
                s("ear.interval.P5"),
            ],
        ),
        GameInfo::new(
            game::CHORD_QUALITY,
            "Chord quality",
            10,
            15,
            vec![s("ear.chord.min"), s("ear.chord.maj")],
        ),
        GameInfo::new(
            game::NOTE_NAMING,
            "Note naming",
            8,
            20,
            vec![s("ear.note.c")],
        ),
    ])
    .confusing(s("ear.interval.m3"), s("ear.interval.M3"))
}

/// Play `seg` as the shell would: every item answered `right` or wrong, the focus skills answered in turn, and the
/// summary applied to the progress file on `day`.
fn play(p: &mut Progress, seg: &Segment, day: Day, right: bool) {
    let mut d = Drill::new(seg.drill_spec(1)).unwrap();
    while let Some(item) = d.next_item() {
        d.answer(Answer::new(item.index, item.focus.clone(), right))
            .unwrap();
    }
    p.observe_drill(&d.summary(), day);
}

fn focus_names(seg: &Segment) -> Vec<&str> {
    seg.focus.iter().map(Skill::as_str).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Determinism

#[test]
fn the_same_progress_file_and_day_give_the_same_workout() {
    let cat = catalog();
    let mut p = Progress::new();
    let w = compose(&p, &cat, TODAY);
    assert!(!w.done_for_today);
    play(&mut p, &w.segments[0], TODAY, true);
    // Read the file again, as the shell does on another day or another machine.
    let reread = Progress::from_json(&p.to_json()).unwrap();
    let a = compose(&p, &cat, TODAY + 1);
    let b = compose(&reread, &cat, TODAY + 1);
    let c = compose(&p.clone(), &catalog(), TODAY + 1);
    assert_eq!(a, b, "a re-read file");
    assert_eq!(a, c, "a rebuilt catalogue");
    assert_eq!(compose(&p, &cat, TODAY + 1), a, "composing twice");
    let other = compose(&p, &cat, TODAY + 2);
    assert_ne!(
        a.segments.iter().map(|g| g.seed).collect::<Vec<_>>(),
        other.segments.iter().map(|g| g.seed).collect::<Vec<_>>(),
        "another day draws other items"
    );
}

#[test]
fn a_workout_has_at_most_three_games_of_distinct_vehicles_with_their_own_seeds() {
    let w = compose(&Progress::new(), &catalog(), TODAY);
    assert_eq!(
        w.segments.len(),
        MAX_SEGMENTS,
        "a new player has everything due"
    );
    let games: BTreeSet<u8> = w.segments.iter().map(|g| g.game).collect();
    assert_eq!(games.len(), w.segments.len(), "one segment a game");
    let seeds: BTreeSet<u64> = w.segments.iter().map(|g| g.seed).collect();
    assert_eq!(seeds.len(), w.segments.len());
    for seg in &w.segments {
        assert!(seg.seed <= MAX_SEED);
        assert_ne!(seg.game, game::NOTE_NAMING, "note naming is opt-in");
        assert!(!seg.focus.contains(&s("ear.note.c")));
        assert!((1..=MAX_FOCUS).contains(&seg.focus.len()), "{seg:?}");
        assert!(usize::from(seg.items) >= seg.focus.len() * usize::from(BURST_ITEMS));
        assert!(seg.items <= MAX_ITEMS);
        assert_eq!(
            seg.start_level, 1,
            "a game never played starts at the bottom"
        );
        assert!(!seg.warm_up);
        seg.drill_spec(1).validate().unwrap();
    }
    assert!(
        w.items() >= 24 && w.items() <= 60,
        "about ten to fifteen minutes: {} items",
        w.items()
    );
    assert!(!w.warm_up && !w.done_for_today);
}

#[test]
fn a_game_takes_three_skills_and_the_fourth_waits() {
    let cat = Catalog::new(vec![GameInfo::new(
        game::SPOT_THE_CHANGE,
        "Spot the change",
        12,
        15,
        params(),
    )]);
    let w = compose(&Progress::new(), &cat, TODAY);
    assert_eq!(w.segments.len(), 1);
    assert_eq!(w.segments[0].focus.len(), MAX_FOCUS);
    assert_eq!(w.segments[0].items, 15, "three bursts of five");
}

// ---------------------------------------------------------------------------------------------------------------
// Due skills choose the games

#[test]
fn a_due_skill_comes_back_in_another_game_each_time() {
    let cat = Catalog::new(vec![
        GameInfo::new(
            game::SPOT_THE_CHANGE,
            "Spot the change",
            12,
            15,
            vec![s("svf.cutoff")],
        ),
        GameInfo::new(
            game::DIAL_IT_IN,
            "Dial it in",
            10,
            12,
            vec![s("svf.cutoff")],
        ),
        GameInfo::new(game::EAR_CHECK, "Ear check", 12, 20, vec![s("svf.cutoff")]),
        GameInfo::new(
            game::FREQUENCY_FINDER,
            "Frequency finder",
            10,
            12,
            vec![s("svf.cutoff")],
        ),
    ]);
    let mut p = Progress::new();
    let mut used = Vec::new();
    for k in 0..8 {
        let day = TODAY + k;
        let w = compose(&p, &cat, day);
        assert_eq!(w.segments.len(), 1, "day {k}");
        let seg = &w.segments[0];
        assert_eq!(focus_names(seg), ["svf.cutoff"]);
        // Wrong answers keep the box at 1: the skill is due again tomorrow.
        play(&mut p, seg, day, false);
        used.push(seg.game);
    }
    for pair in used.windows(2) {
        assert_ne!(
            pair[0], pair[1],
            "never the game that drilled it last: {used:?}"
        );
    }
    let distinct: BTreeSet<u8> = used.iter().copied().collect();
    assert!(distinct.len() >= 3, "varied retrieval: {used:?}");
}

#[test]
fn only_due_skills_are_asked() {
    let cat = catalog();
    let mut p = Progress::new();
    // Every skill of the default games rung today: box 2 is not due for two days.
    for g in cat.games() {
        for skill in &g.skills {
            p.observe_burst(&Burst::new(skill.clone(), 5), TODAY);
        }
    }
    let w = compose(&p, &cat, TODAY);
    assert!(
        w.segments.is_empty() && w.done_for_today,
        "nothing is due: done for today"
    );
    // Two days on, they are all due again.
    let w = compose(&p, &cat, TODAY + 2);
    assert!(!w.done_for_today);
    // Only one skill is due: only it is asked, and in a game that can drill it.
    let mut p = Progress::new();
    for g in cat.games() {
        for skill in &g.skills {
            if skill.as_str() != "ear.chord.maj" {
                p.observe_burst(&Burst::new(skill.clone(), 5), TODAY);
            }
        }
    }
    let w = compose(&p, &cat, TODAY);
    assert_eq!(w.segments.len(), 1);
    assert_eq!(focus_names(&w.segments[0]), ["ear.chord.maj"]);
    assert_eq!(w.segments[0].game, game::CHORD_QUALITY);
}

#[test]
fn note_naming_is_never_in_the_default_workout() {
    let only = Catalog::new(vec![GameInfo::new(
        game::NOTE_NAMING,
        "Note naming",
        8,
        20,
        vec![s("ear.note.c"), s("ear.note.d")],
    )]);
    let p = Progress::new();
    let w = compose(&p, &only, TODAY);
    assert!(
        w.done_for_today,
        "a catalogue of note naming alone has no default workout"
    );
    assert!(one_more(&p, &only, TODAY).is_none());
    // It also stays out of a full catalogue's "one more".
    let full = catalog();
    for k in 0..20 {
        let seg = one_more(&p, &full, TODAY + k).unwrap();
        assert_ne!(seg.game, game::NOTE_NAMING);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Confusable skills, breaks, and the end

#[test]
fn confusable_skills_are_met_alone_before_they_mix() {
    let cat = Catalog::new(vec![GameInfo::new(
        game::INTERVAL_STACKS,
        "Interval stacks",
        12,
        15,
        vec![s("ear.interval.m3"), s("ear.interval.M3")],
    )])
    .confusing(s("ear.interval.m3"), s("ear.interval.M3"));
    let mut p = Progress::new();
    let first = compose(&p, &cat, TODAY);
    assert_eq!(
        first.segments[0].focus.len(),
        1,
        "met alone: {:?}",
        first.segments[0].focus
    );
    play(&mut p, &first.segments[0], TODAY, false);
    let second = compose(&p, &cat, TODAY + 1);
    assert_eq!(
        second.segments[0].focus.len(),
        1,
        "the other one is met alone next"
    );
    assert_ne!(second.segments[0].focus, first.segments[0].focus);
    play(&mut p, &second.segments[0], TODAY + 1, false);
    let third = compose(&p, &cat, TODAY + 2);
    assert_eq!(
        third.segments[0].focus.len(),
        2,
        "both met: they mix now ({:?})",
        third.segments[0].focus
    );
}

/// A finished drill of `game` that ended at `end_level`, with no skill answered.
fn ended_at(game: u8, end_level: u8) -> Summary {
    Summary {
        game,
        items: 15,
        right: 12,
        start_level: 1,
        end_level,
        peak_level: end_level,
        reversals: 4,
        threshold: None,
        bursts: Vec::new(),
        skills: BTreeMap::new(),
        measured: None,
    }
}

fn intervals_only() -> Catalog {
    Catalog::new(vec![GameInfo::new(
        game::INTERVAL_STACKS,
        "Interval stacks",
        12,
        15,
        vec![s("ear.interval.m3")],
    )])
}

#[test]
fn after_a_break_a_game_starts_one_level_down_silently() {
    let cat = intervals_only();
    let mut p = Progress::new();
    p.observe_drill(&ended_at(game::INTERVAL_STACKS, 6), TODAY);
    let at = |day: Day| {
        let w = compose(&p, &cat, day);
        assert_eq!(w.segments.len(), 1, "day {day}");
        (w.segments[0].start_level, w.segments[0].warm_up, w.warm_up)
    };
    assert_eq!(at(TODAY + 1), (6, false, false), "the next day carries on");
    assert_eq!(
        at(TODAY + BREAK_DAYS),
        (6, false, false),
        "a week is no break"
    );
    assert_eq!(
        at(TODAY + BREAK_DAYS + 1),
        (5, true, true),
        "more than a week: a warm-up one level down"
    );
    assert_eq!(
        at(TODAY + 400),
        (5, true, true),
        "however long: one level, not a reset"
    );
    // The warm-up leaves no trace in the file: the staircase ended at 6 and the next session decides again.
    assert_eq!(p.drills.game(game::INTERVAL_STACKS).level, 6);
}

#[test]
fn a_game_at_the_bottom_has_no_warm_up() {
    let cat = intervals_only();
    let mut p = Progress::new();
    p.observe_drill(&ended_at(game::INTERVAL_STACKS, 1), TODAY);
    let w = compose(&p, &cat, TODAY + 30);
    assert_eq!(w.segments.len(), 1);
    assert_eq!(
        (w.segments[0].start_level, w.segments[0].warm_up, w.warm_up),
        (1, false, false)
    );
    // A ladder that shrank below where the staircase ended starts at its top.
    let mut p = Progress::new();
    p.observe_drill(&ended_at(game::INTERVAL_STACKS, 30), TODAY);
    let w = compose(&p, &cat, TODAY + 1);
    assert_eq!(w.segments[0].start_level, w.segments[0].levels);
}

#[test]
fn a_workout_ends_and_one_more_is_optional() {
    let cat = catalog();
    let mut p = Progress::new();
    let w = compose(&p, &cat, TODAY);
    for seg in &w.segments {
        play(&mut p, seg, TODAY, true);
    }
    // Everything asked is answered, but the workout is only over once the shell says so.
    p.finish_workout(TODAY);
    let done = compose(&p, &cat, TODAY);
    assert!(done.done_for_today && done.segments.is_empty() && !done.warm_up);
    assert_eq!(done.items(), 0);
    // The shell's file keeps it.
    let reread = Progress::from_json(&p.to_json()).unwrap();
    assert!(compose(&reread, &cat, TODAY).done_for_today);
    // Tomorrow is another day.
    assert!(!compose(&p, &cat, TODAY + 3).done_for_today);
    // One more: a single segment, the same every time, from the weakest skills.
    let extra = one_more(&p, &cat, TODAY).expect("a catalogue with games has an extra");
    assert_eq!(Some(&extra), one_more(&reread, &cat, TODAY).as_ref());
    assert!(extra.items <= MAX_ITEMS);
    extra.drill_spec(1).validate().unwrap();
    // The calendar of days practised cannot be broken: it holds the day, and nothing counts a missing one.
    assert!(p.drills.practised(TODAY));
}

#[test]
fn a_finished_workout_on_day_zero_is_no_marker() {
    // Day 0 means "never" in the file; a shell that has no clock yet composes a workout, not "done".
    let mut p = Progress::new();
    p.finish_workout(0);
    assert!(!compose(&p, &catalog(), 0).done_for_today);
}

// ---------------------------------------------------------------------------------------------------------------
// The whole loop

proptest! {
    /// Compose, play some segments (right or wrong), maybe finish, on days that skip: at every step the workout
    /// obeys its rules and is the same when composed again from a re-read file.
    #[test]
    fn composed_workouts_obey_their_rules_through_a_whole_loop(
        steps in prop::collection::vec((0_u32..12, any::<bool>(), any::<bool>(), 0_usize..4), 1..14),
    ) {
        let cat = catalog();
        let mut p = Progress::new();
        let mut day = 20_700;
        for (gap, right, finish, played) in steps {
            day += gap;
            let w = compose(&p, &cat, day);
            let reread = Progress::from_json(&p.to_json()).unwrap();
            prop_assert_eq!(&w, &compose(&reread, &cat, day));
            prop_assert_eq!(w.day, day);
            prop_assert!(w.segments.len() <= MAX_SEGMENTS);
            prop_assert_eq!(w.done_for_today, w.segments.is_empty());
            prop_assert_eq!(w.warm_up, w.segments.iter().any(|g| g.warm_up));
            let mut games = BTreeSet::new();
            let mut skills = BTreeSet::new();
            for seg in &w.segments {
                prop_assert!(games.insert(seg.game), "one segment a game");
                prop_assert_ne!(seg.game, game::NOTE_NAMING);
                prop_assert!((1..=MAX_FOCUS).contains(&seg.focus.len()));
                let info = cat.game(seg.game).unwrap();
                for f in &seg.focus {
                    prop_assert!(info.skills.contains(f), "{} cannot drill {}", info.title, f);
                    prop_assert!(skills.insert(f.clone()), "a skill once a workout");
                }
                prop_assert!(usize::from(seg.items) >= seg.focus.len() * usize::from(BURST_ITEMS));
                prop_assert!(seg.items <= MAX_ITEMS);
                prop_assert!((1..=seg.levels).contains(&seg.start_level));
                prop_assert!(seg.seed <= MAX_SEED);
                prop_assert!(seg.drill_spec(1).validate().is_ok());
            }
            for seg in w.segments.iter().take(played) {
                play(&mut p, seg, day, right);
            }
            if finish {
                p.finish_workout(day);
                prop_assert!(compose(&p, &cat, day).done_for_today);
            }
        }
    }
}
