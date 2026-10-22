//! Ear generator v1's properties (lane MUS; docs/phase-4.md, Acceptance): every generated question is valid (C1 to C7,
//! at most 8 notes at once, at most 10 s, integers only), every answer key is one of its options, a spec reproduces
//! itself, and the JSON forms are fixed. The golden is `ear_golden.rs`.

use enth_music::ear::{
    self, Answer, AnswerKind, EarError, EarSpec, Game, MAX_COUNT, MAX_LENGTH_MS, MAX_NOTES_AT_ONCE,
    MAX_SEED, OptionKind, Question,
};
use enth_music::ladder;
use enth_music::rhythm::Pattern;
use enth_music::theory::{KEY_MAX, KEY_MIN};
use proptest::prelude::*;

fn game_level(g: usize, l: u8) -> (Game, u8) {
    let game = Game::ALL[g % Game::ALL.len()];
    (game, 1 + l % game.max_level())
}

proptest! {
    /// Every question of every game and level is valid, and a choice answer is one of its options.
    #[test]
    fn every_question_is_valid(g in 0usize..7, l in 0u8..16, seed in 0u64..=MAX_SEED, item in 0u32..100_000) {
        let (game, level) = game_level(g, l);
        let q = ear::question(game, level, seed, item).unwrap();
        prop_assert_eq!(q.validate(), Ok(()));
        prop_assert!(q.notes.iter().all(|n| (KEY_MIN..=KEY_MAX).contains(&n.key)));
        prop_assert!(q.max_simultaneous() <= MAX_NOTES_AT_ONCE);
        prop_assert!(q.length_ms <= MAX_LENGTH_MS);
        match &q.answer {
            Answer::Choice(i) => {
                prop_assert_eq!(game.answer_kind(level), AnswerKind::Choice);
                prop_assert!(usize::from(*i) < q.options.ids.len());
                prop_assert!(q.answer_label().is_some());
            }
            Answer::Taps { onsets_ms, window_ms } => {
                prop_assert_eq!(game.answer_kind(level), AnswerKind::Taps);
                prop_assert_eq!(q.options.kind, OptionKind::None);
                prop_assert!(onsets_ms.iter().all(|t| *t < u32::from(*window_ms)));
            }
        }
        prop_assert!(q.skill().is_some());
    }

    /// A spec reproduces itself, item by item and as a set, and its JSON form round trips.
    #[test]
    fn a_spec_is_deterministic(g in 0usize..7, l in 0u8..16, seed in 0u64..=MAX_SEED, count in 1u8..=MAX_COUNT) {
        let (game, level) = game_level(g, l);
        let spec = EarSpec::new(game, level, seed, count);
        let all = spec.questions().unwrap();
        prop_assert_eq!(all.len(), usize::from(count));
        prop_assert_eq!(&all, &spec.questions().unwrap());
        for (i, q) in all.iter().enumerate() {
            prop_assert_eq!(q, &spec.question(i as u32).unwrap());
        }
        let json = serde_json::to_string(&spec).unwrap();
        prop_assert_eq!(serde_json::from_str::<EarSpec>(&json).unwrap(), spec);
        let q = &all[0];
        let json = serde_json::to_string(q).unwrap();
        prop_assert_eq!(&serde_json::from_str::<Question>(&json).unwrap(), q);
    }

    /// `validate` accepts exactly the specs that generate.
    #[test]
    fn validate_agrees_with_generation(game in 0u8..=12, generator in 0u8..=3, level in 0u8..=17, seed in prop_oneof![0u64..1000, (MAX_SEED - 2)..=(MAX_SEED + 2)], count in 0u8..=66) {
        let spec = EarSpec { game, generator, level, seed, count };
        let valid = spec.validate().is_ok();
        prop_assert_eq!(valid, spec.questions().is_ok());
        if valid {
            prop_assert!((4..=10).contains(&game) && generator == 1 && level >= 1 && seed <= MAX_SEED);
        }
    }

    /// Canonical bytes are injective on what differs: two different questions never share bytes.
    #[test]
    fn canonical_bytes_tell_questions_apart(g in 0usize..7, seed in 0u64..=MAX_SEED, a in 0u32..50, b in 0u32..50) {
        let (game, level) = game_level(g, 0);
        let qa = ear::question(game, level, seed, a).unwrap();
        let qb = ear::question(game, level, seed, b).unwrap();
        let (mut ba, mut bb) = (Vec::new(), Vec::new());
        qa.write_canonical(&mut ba);
        qb.write_canonical(&mut bb);
        prop_assert_eq!(qa == qb, ba == bb);
    }
}

#[test]
fn a_spec_has_a_fixed_json_form() {
    let spec = EarSpec::new(Game::IntervalStacks, 1, 42, 20);
    let json = r#"{"game":5,"generator":1,"level":1,"seed":42,"count":20}"#;
    assert_eq!(serde_json::to_string(&spec).unwrap(), json);
    assert_eq!(serde_json::from_str::<EarSpec>(json).unwrap(), spec);
    // Strict like RandomSpec: unknown fields, floats and missing fields are refused.
    for bad in [
        r#"{"game":5,"generator":1,"level":1,"seed":42,"count":20,"x":1}"#,
        r#"{"game":5,"generator":1,"level":1,"seed":42.0,"count":20}"#,
        r#"{"game":5,"generator":1,"level":1,"seed":42}"#,
        r#"{"game":5,"generator":1,"level":1,"seed":-1,"count":20}"#,
        r#"{"game":256,"generator":1,"level":1,"seed":42,"count":20}"#,
    ] {
        assert!(serde_json::from_str::<EarSpec>(bad).is_err(), "{bad}");
    }
}

#[test]
fn refused_specs_say_why() {
    let ok = EarSpec::new(Game::TapBack, 1, 1, 1);
    assert!(ok.validate().is_ok());
    let level_past_the_ladder = EarSpec {
        level: Game::TapBack.max_level() + 1,
        ..ok
    };
    assert_eq!(
        level_past_the_ladder.validate(),
        Err(EarError::LevelUnavailable {
            game: 10,
            level: Game::TapBack.max_level() + 1,
            max: Game::TapBack.max_level()
        })
    );
    // The parameter games are generated in enth-challenge, not here.
    for id in [ear::SPOT_THE_CHANGE, ear::DIAL_IT_IN, ear::EAR_CHECK] {
        assert_eq!(
            EarSpec { game: id, ..ok }.validate(),
            Err(EarError::UnknownGame(id))
        );
    }
    assert!(EarError::BadCount(0).to_string().contains("1 to 64"));
}

/// Level 1 of each ladder file: what the module docs promise.
#[test]
fn level_one_of_each_game_is_what_its_ladder_says() {
    for item in 0..400 {
        // Intervals: P1 or P8, listed P1 then P8; a unison is one note, an octave two.
        let q = ear::question(Game::IntervalStacks, 1, 5, item).unwrap();
        assert_eq!(q.options.ids, [0, 12]);
        let Answer::Choice(i) = q.answer else {
            panic!()
        };
        let size = q.options.ids[usize::from(i)];
        assert_eq!(q.notes.len(), if size == 0 { 1 } else { 2 });
        assert!(q.notes.iter().all(|n| n.start_ms == 0 && n.hold_ms == 2000));
        if size == 12 {
            assert_eq!(q.notes[1].key, q.notes[0].key + 12);
        }
        assert!((48..=72).contains(&q.notes[0].key));

        // Chords: major or minor in root position.
        let q = ear::question(Game::ChordQuality, 1, 5, item).unwrap();
        assert_eq!(q.options.ids, [0, 1]);
        let Answer::Choice(i) = q.answer else {
            panic!()
        };
        let third = q.notes[1].key - q.notes[0].key;
        assert_eq!(third, if i == 0 { 4 } else { 3 });
        assert_eq!(q.notes[2].key - q.notes[0].key, 7);

        // Pitch: two pitch classes a fourth or more apart, the note is the answer.
        let q = ear::question(Game::NoteNaming, 1, 5, item).unwrap();
        let (a, b) = (q.options.ids[0], q.options.ids[1]);
        assert!(a < b && (b - a).min(12 - (b - a)) >= 5);
        let Answer::Choice(i) = q.answer else {
            panic!()
        };
        assert_eq!(
            u32::from(q.notes[0].key) % 12,
            q.options.ids[usize::from(i)]
        );
        assert_eq!(q.notes.len(), 1);

        // Bands: two centres at least three octaves apart, the post band is the answer, in the svf form.
        let q = ear::question(Game::FrequencyFinder, 1, 5, item).unwrap();
        let (lo, hi) = (q.options.ids[0], q.options.ids[1]);
        assert!(hi >= 8 * lo);
        let Answer::Choice(i) = q.answer else {
            panic!()
        };
        let band = q.post.unwrap();
        assert_eq!(u32::from(band.freq_hz), q.options.ids[usize::from(i)]);
        assert_eq!(band.kind, ear::EqKind::BandPass);
        assert_eq!(q.timbre, ear::timbre::NOISE);

        // Progressions: the notes are the answer's voicing; four chords of 1.2 s.
        let q = ear::question(Game::Progressions, 1, 5, item).unwrap();
        assert_eq!(q.options.ids, [0, 1]);
        assert_eq!(q.length_ms, 4 * 1200 + ear::TAIL_MS);
        assert!(
            q.notes
                .iter()
                .all(|n| n.hold_ms == 1200 && n.start_ms % 1200 == 0)
        );
    }
}

/// The rhythm and tap ladders' rules hold for every draw (no draw fell back to an unchecked pattern), and the tap
/// answer is the pattern's own onsets.
#[test]
fn rhythm_draws_meet_their_rules() {
    let l = &ladder::rhythm::LEVELS[0];
    let t = &ladder::taps::LEVELS[0];
    for item in 0..3000 {
        let q = ear::question(Game::RhythmPattern, 1, 11, item).unwrap();
        assert_eq!(q.options.ids.len(), 4);
        let patterns: Vec<Pattern> = q.options.ids.iter().map(|m| Pattern(*m as u16)).collect();
        for (i, p) in patterns.iter().enumerate() {
            assert!(l.rule.accepts(*p, &patterns[..i]), "item {item}: {p}");
            assert!(p.syncopation() <= 1, "low syncopation");
            assert_eq!(p.0 & !0x5555, 0, "even steps only");
        }
        assert!(q.length_ms <= 8300, "{}", q.length_ms);

        let q = ear::question(Game::TapBack, 1, 11, item).unwrap();
        let Answer::Taps {
            onsets_ms,
            window_ms,
        } = &q.answer
        else {
            panic!()
        };
        assert_eq!(*window_ms, 2400);
        assert_eq!(onsets_ms[0], 0, "the downbeat is anchored");
        assert!((3..=5).contains(&onsets_ms.len()));
        let step = 150;
        let pattern = Pattern::from_steps(
            &onsets_ms
                .iter()
                .map(|t| (t / step) as u8)
                .collect::<Vec<_>>(),
        );
        assert!(t.rule.accepts(pattern, &[]), "item {item}");
        // The pattern notes in the clip are the onsets one bar after the count-in.
        let heard: Vec<u32> = q
            .notes
            .iter()
            .filter(|n| n.key == t.pattern_key)
            .map(|n| n.start_ms - 2400)
            .collect();
        assert_eq!(&heard, onsets_ms);
    }
}

/// No game leaks its answer through its position or its timbre: over many items the answer's index is spread evenly
/// and every timbre of the level shows up with every answer (docs/ear.md, "GR4": bias bots stay at chance).
#[test]
fn answers_are_spread_over_positions_and_timbres() {
    for game in Game::ALL {
        if game == Game::TapBack {
            continue;
        }
        let n = 4000u32;
        let mut by_index = [0u32; 12];
        let mut by_timbre_and_index = std::collections::BTreeMap::<(u8, u8), u32>::new();
        let mut options = 0;
        for item in 0..n {
            let q = ear::question(game, 1, 2024, item).unwrap();
            options = q.options.ids.len();
            let Answer::Choice(i) = q.answer else {
                panic!()
            };
            by_index[usize::from(i)] += 1;
            *by_timbre_and_index.entry((q.timbre, i)).or_default() += 1;
        }
        let expect = n / options as u32;
        for (i, c) in by_index.iter().take(options).enumerate() {
            assert!(
                c.abs_diff(expect) * 100 <= expect * 8,
                "{game:?}: answer index {i} came {c} times of {n} (about {expect} expected)"
            );
        }
        // Every timbre appears with every answer index, so the timbre says nothing about the answer.
        let timbres: std::collections::BTreeSet<u8> =
            by_timbre_and_index.keys().map(|(t, _)| *t).collect();
        for t in &timbres {
            for i in 0..options as u8 {
                assert!(
                    by_timbre_and_index.get(&(*t, i)).copied().unwrap_or(0) > 0,
                    "{game:?}: timbre {t} never goes with answer {i}"
                );
            }
        }
    }
}

/// Each level-1 game draws a spread of content, so a session is not one question over and over.
#[test]
fn level_one_has_variety() {
    for game in Game::ALL {
        let distinct: std::collections::BTreeSet<Vec<u8>> = (0..300)
            .map(|item| {
                let mut b = Vec::new();
                ear::question(game, 1, 77, item)
                    .unwrap()
                    .write_canonical(&mut b);
                b
            })
            .collect();
        assert!(
            distinct.len() >= 12,
            "{game:?} drew only {} distinct questions in 300",
            distinct.len()
        );
    }
}

/// A burst on one skill: the same seed gives the same items, each of them exercising the skill.
#[test]
fn a_burst_on_a_skill_scans_the_keyed_items() {
    let mut from = 0;
    let mut items = Vec::new();
    for _ in 0..5 {
        let (item, q) =
            ear::find_skill_item(Game::IntervalStacks, 1, 99, "ear.interval.p8", from).unwrap();
        assert!(item >= from);
        assert_eq!(q.skill().as_deref(), Some("ear.interval.p8"));
        assert_eq!(q, ear::question(Game::IntervalStacks, 1, 99, item).unwrap());
        items.push(item);
        from = item + 1;
    }
    assert!(items.windows(2).all(|w| w[0] < w[1]));
    // Every skill a level can ask is found.
    for game in Game::ALL {
        let q = ear::question(game, 1, 5, 0).unwrap();
        let skill = q.skill().unwrap();
        assert_eq!(
            ear::find_skill_item(game, 1, 5, &skill, 0).map(|(i, _)| i),
            Some(0),
            "{game:?}"
        );
    }
    // A skill the level never asks, and a level the build lacks.
    assert_eq!(
        ear::find_skill_item(Game::IntervalStacks, 1, 99, "ear.interval.m3", 0),
        None
    );
    assert_eq!(
        ear::find_skill_item(Game::IntervalStacks, 9, 99, "ear.interval.p8", 0),
        None
    );
}
