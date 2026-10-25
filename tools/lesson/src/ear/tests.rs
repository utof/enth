//! **Owner: lane ECK.** Tests of `ear-check`.
//!
//! Three kinds. **The four seeded faults** (the acceptance): a bad game of each kind is planted and the run fails on
//! it, where a control game that is the same but for the fault passes: a leak a bias bot finds (a position, a length,
//! an energy), an inaudible pair, an unsolvable top level, a drifting hash. **The anchor**: the measuring in `metric.rs`
//! gives `score_round`'s `jnd` on patches under a table phrase, and renders a music clip as the session does. **The
//! shipped games**: the hard checks hold on them, their counterfactuals are what the questions are, and the committed
//! items digest is current. The rest is the staircase's maths and the exit policy.

use enth_challenge::ear::params;
use enth_challenge::ear::question::{Clip as Played, Stimulus};
use enth_challenge::ear::scorers::EarKey;
use enth_challenge::model::{PhraseNote, SCORING_SEED};
use enth_challenge::patch_round::{Compared, prepare_patch_round};
use enth_challenge::phrases::HELD;
use enth_challenge::v2::InlinePatch;
use enth_challenge::{Challenge, ScoringSpec, Start, Target, score_round};
use enth_dsp::util::rng::derive;
use enth_music::ear::{Game, MAX_SEED};
use enth_patch::{Op, ParamAddr};

use super::audibility;
use super::check::{self, Plan};
use super::game::{Cue, EarGame, Item, Layout, Shipped, shipped};
use super::golden;
use super::metric::{self, Playable};
use super::report::{Gate, Ladders, Report};
use super::staircase::{self, Ladder, Row};

// ---------------------------------------------------------------------------------------------------------------
// Planted games

/// A game planted by a test: any items, any number of levels.
struct Planted {
    id: u8,
    levels: u8,
    make: Box<dyn Fn(u8, u64, u32) -> Item + Sync>,
}

impl EarGame for Planted {
    fn id(&self) -> u8 {
        self.id
    }

    fn name(&self) -> String {
        format!("Planted {}", self.id)
    }

    fn levels(&self) -> u8 {
        self.levels
    }

    fn item(&self, level: u8, seed: u64, index: u32) -> Result<Item, String> {
        Ok((self.make)(level, seed, index))
    }
}

/// A single-note stimulus: a clip of a timbre, a key, a velocity and a hold.
fn tone(timbre: u8, key: u8, velocity: u8, hold_ms: u32) -> Played {
    Played::Stimulus(Stimulus {
        timbre,
        notes: vec![PhraseNote {
            key,
            velocity,
            start_ms: 0,
            hold_ms,
        }],
        length_ms: hold_ms + 1000,
        band: None,
    })
}

/// What a bot could read of a tone: the same numbers `Cue::of` takes from a music question.
fn cue(key: u8, velocity: u8, hold_ms: u32) -> Cue {
    let v = f64::from(velocity) / 127.0;
    Cue {
        length_ms: hold_ms + 1000,
        energy: v * v * f64::from(hold_ms) / 1000.0,
        top: key,
        notes: 1,
    }
}

/// An item of `options` clips and cues, the right answer at `key`.
fn item(game: u8, level: u8, seed: u64, index: u32, key: usize, parts: Vec<(Played, Cue)>) -> Item {
    let n = parts.len();
    let (states, cues): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
    Item {
        game,
        level,
        seed,
        index,
        options: n,
        key,
        skill: Some("ear.interval.p5".to_owned()),
        ekey: EarKey::Choice {
            index: u8::try_from(key).unwrap(),
            options: u8::try_from(n).unwrap(),
        },
        layout: Layout::PerOption,
        states,
        cues,
        controls: Vec::new(),
        steps: None,
    }
}

/// A key independent of anything a bot reads: a hash of the item.
fn fair_key(seed: u64, index: u32, n: usize) -> usize {
    (derive(seed, u64::from(index) + 0x004B_4559) % n as u64) as usize
}

/// Three options of equal length and loudness, differing in pitch by an octave and more (so far apart for the ear).
fn spread(key: usize, n: usize) -> Vec<(Played, Cue)> {
    (0..n)
        .map(|j| {
            let _ = key;
            (
                tone(0, 36 + 12 * u8::try_from(j).unwrap(), 100, 600),
                cue(36, 100, 600),
            )
        })
        .collect()
}

fn plan() -> Plan {
    Plan {
        seed: 0x0000_4543_4B5F_5445 & MAX_SEED,
        audible_items: 0,
        bot_items: 2500,
        runs: 60,
        jobs: 2,
        ladders: Ladders::Auto,
    }
}

fn run(games: &[&dyn EarGame], plan: &Plan) -> Report {
    check::check_games(games, plan).report
}

fn names(r: &Report, ladders: Ladders) -> Vec<(&'static str, &str)> {
    r.failures(ladders)
        .into_iter()
        .map(|c| (c.name, c.cell.as_str()))
        .collect()
}

fn fails(r: &Report, name: &str) -> bool {
    r.failures(Ladders::Auto).iter().any(|c| c.name == name)
}

// ---------------------------------------------------------------------------------------------------------------
// 1. A leak a bias bot finds

fn fair_game() -> Planted {
    Planted {
        id: 40,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let key = fair_key(seed, index, 3);
            item(40, level, seed, index, key, spread(key, 3))
        }),
    }
}

#[test]
fn a_fair_game_passes_every_hard_check() {
    let r = run(&[&fair_game()], &plan());
    assert!(
        r.failures(Ladders::Auto).is_empty(),
        "{}",
        r.summary(Ladders::Auto)
    );
}

#[test]
fn a_bias_bot_finds_a_position_leak() {
    // The right answer is always the first option.
    let game = Planted {
        id: 41,
        levels: 1,
        make: Box::new(|level, seed, index| item(41, level, seed, index, 0, spread(0, 3))),
    };
    let r = run(&[&game], &plan());
    let bias = r
        .failures(Ladders::Auto)
        .into_iter()
        .find(|c| c.name == "bias");
    assert!(
        bias.is_some_and(|c| c.detail.contains("first")),
        "{}",
        r.summary(Ladders::Auto)
    );
    // The key oracle and the judge are still fine: the game is solvable, and unfair.
    assert!(!fails(&r, "oracle"));
}

#[test]
fn a_bias_bot_finds_a_length_leak() {
    // The right answer is always the longest option; its position is fair, and so is its energy (the cue keeps one).
    let game = Planted {
        id: 42,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let key = fair_key(seed, index, 3);
            let parts = (0..3)
                .map(|j| {
                    let hold = if j == key { 1800 } else { 600 };
                    let mut c = cue(48, 100, 600);
                    c.length_ms = hold + 1000;
                    (tone(0, 48, 100, hold), c)
                })
                .collect();
            item(42, level, seed, index, key, parts)
        }),
    };
    let r = run(&[&game], &plan());
    let bias = r
        .failures(Ladders::Auto)
        .into_iter()
        .find(|c| c.name == "bias");
    assert!(
        bias.is_some_and(|c| c.detail.contains("longest")),
        "{}",
        r.summary(Ladders::Auto)
    );
}

#[test]
fn a_bias_bot_finds_a_loudness_leak() {
    // The right answer is always the loudest option (by velocity); its position is fair.
    let game = Planted {
        id: 43,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let key = fair_key(seed, index, 4);
            let parts = (0..4)
                .map(|j| {
                    let vel = if j == key { 120 } else { 70 };
                    (tone(0, 48, vel, 600), cue(48, vel, 600))
                })
                .collect();
            item(43, level, seed, index, key, parts)
        }),
    };
    let r = run(&[&game], &plan());
    let bias = r
        .failures(Ladders::Auto)
        .into_iter()
        .find(|c| c.name == "bias");
    assert!(
        bias.is_some_and(|c| c.detail.contains("loudest")),
        "{}",
        r.summary(Ladders::Auto)
    );
}

#[test]
fn a_key_that_follows_the_order_of_play_is_found_by_the_deaf_player() {
    // The right answer cycles with the item number, which a player who hears nothing can follow.
    let game = Planted {
        id: 44,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let key = index as usize % 3;
            item(44, level, seed, index, key, spread(key, 3))
        }),
    };
    let r = run(&[&game], &plan());
    assert!(fails(&r, "chance"), "{}", r.summary(Ladders::Auto));
}

#[test]
fn a_key_the_judge_refuses_fails_structure() {
    let game = Planted {
        id: 45,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let mut it = item(45, level, seed, index, 1, spread(1, 3));
            it.ekey = EarKey::Choice {
                index: 7,
                options: 3,
            };
            it
        }),
    };
    let r = run(&[&game], &plan());
    assert!(fails(&r, "structure"), "{}", r.summary(Ladders::Auto));
}

#[test]
fn a_skill_that_is_not_a_review_key_fails_structure() {
    let game = Planted {
        id: 46,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let mut it = item(
                46,
                level,
                seed,
                index,
                fair_key(seed, index, 3),
                spread(0, 3),
            );
            it.skill = Some("not a skill".to_owned());
            it
        }),
    };
    let r = run(&[&game], &plan());
    assert!(fails(&r, "structure"), "{}", r.summary(Ladders::Auto));
}

// ---------------------------------------------------------------------------------------------------------------
// 2. An inaudible pair, and 3. an unsolvable top level

/// Options differing in timbre (sine, saw, pink noise): far apart under `sound/1` at any level.
fn timbres(n: usize) -> Vec<(Played, Cue)> {
    [0_u8, 1, 6]
        .iter()
        .take(n)
        .map(|t| (tone(*t, 48, 100, 700), cue(48, 100, 700)))
        .collect()
}

/// A two-level game whose second level offers `second` options per item.
fn two_levels(id: u8, second: fn(usize, usize) -> Vec<(Played, Cue)>) -> Planted {
    Planted {
        id,
        levels: 2,
        make: Box::new(move |level, seed, index| {
            let key = fair_key(seed, index, 3);
            let parts = if level == 1 {
                timbres(3)
            } else {
                second(key, 3)
            };
            item(id, level, seed, index, key, parts)
        }),
    }
}

fn measuring() -> Plan {
    Plan {
        audible_items: 3,
        ..plan()
    }
}

#[test]
fn a_game_of_far_apart_options_is_audible_and_solvable() {
    let game = two_levels(50, |_, n| timbres(n));
    let r = check::check_games(&[&game], &measuring());
    let failed = names(&r.report, Ladders::Auto);
    assert!(
        !failed
            .iter()
            .any(|(n, _)| ["audibility", "observer"].contains(n)),
        "{}",
        r.report.summary(Ladders::Auto)
    );
    // Every level measured: three pairs per item, none unmeasured.
    for c in &r.cells {
        let s = c.summary.as_ref().expect("measured");
        assert_eq!(s.unmeasured.0, 0);
        assert!(
            s.min.is_some_and(|m| m >= audibility::floor_for(c.level)),
            "{c:?}"
        );
    }
}

#[test]
fn an_inaudible_pair_is_found() {
    // At level 2 two of the three options are the same clip.
    let game = two_levels(51, |_, _| {
        let a = (tone(0, 48, 100, 700), cue(48, 100, 700));
        vec![a.clone(), a, (tone(6, 48, 100, 700), cue(48, 100, 700))]
    });
    let r = check::check_games(&[&game], &measuring());
    let found = r
        .report
        .failures(Ladders::Auto)
        .into_iter()
        .find(|c| c.name == "audibility" && c.cell.ends_with("L2"));
    // The two-level game is a delivered ladder, so the failure is hard (a one-level stub only reports, below).
    assert!(found.is_some(), "{}", r.report.summary(Ladders::Auto));
    assert!(
        !r.report
            .failures(Ladders::Report)
            .iter()
            .any(|c| c.name == "audibility")
    );
}

#[test]
fn a_one_level_stub_reports_an_inaudible_pair_without_failing() {
    let game = Planted {
        id: 53,
        levels: 1,
        make: Box::new(|level, seed, index| {
            let a = (tone(0, 48, 100, 700), cue(48, 100, 700));
            item(
                53,
                level,
                seed,
                index,
                fair_key(seed, index, 2),
                vec![a.clone(), a],
            )
        }),
    };
    let r = check::check_games(&[&game], &measuring());
    assert!(
        r.report.failures(Ladders::Auto).is_empty(),
        "{}",
        r.report.summary(Ladders::Auto)
    );
    assert!(
        r.report
            .reported(Ladders::Auto)
            .iter()
            .any(|c| c.name == "audibility")
    );
    // --strict hardens it, and --ladders report softens a delivered ladder.
    assert!(
        r.report
            .failures(Ladders::Enforce)
            .iter()
            .any(|c| c.name == "audibility")
    );
}

#[test]
fn an_unsolvable_top_level_is_found() {
    // Level 1 offers four far apart timbres; level 2 four options whose clips are all the same, so an ideal observer
    // who has the right clip in hand still guesses among four.
    let game = Planted {
        id: 54,
        levels: 2,
        make: Box::new(|level, seed, index| {
            let key = fair_key(seed, index, 3);
            let parts = if level == 1 {
                timbres(3)
            } else {
                (0..3)
                    .map(|_| (tone(6, 48, 100, 700), cue(48, 100, 700)))
                    .collect()
            };
            item(54, level, seed, index, key, parts)
        }),
    };
    let r = check::check_games(&[&game], &measuring());
    let failed: Vec<_> = names(&r.report, Ladders::Auto);
    assert!(
        failed.contains(&("observer", "Planted 54 L2")),
        "{}",
        r.report.summary(Ladders::Auto)
    );
    // Only the top level is asked to be solvable, and level 1 is fine.
    assert!(
        !failed
            .iter()
            .any(|(n, c)| *n == "observer" && c.ends_with("L1")),
        "{failed:?}"
    );
    let observer = r
        .cells
        .iter()
        .find(|c| c.level == 2)
        .and_then(|c| c.observer.as_ref())
        .expect("measured");
    assert!(observer.accuracy() < 0.5, "{observer:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// 4. A drifting hash

#[test]
fn the_music_golden_is_current() {
    let found = golden::check_music_real(golden::MUSIC_GOLDEN);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_generator_that_drifted_is_found() {
    // Interval stacks now strikes its notes a velocity louder: every question of the game changes.
    let drifted = |game: Game, level: u8, seed: u64, item: u32| {
        let mut q = enth_music::ear::question(game, level, seed, item)?;
        if game == Game::IntervalStacks
            && let Some(n) = q.notes.first_mut()
        {
            n.velocity = n.velocity.saturating_sub(1);
        }
        Ok(q)
    };
    let found = golden::check_music(golden::MUSIC_GOLDEN, &drifted);
    assert_ne!(found.len(), 0);
    assert!(
        found.iter().all(|f| f.cell.starts_with("g=05")),
        "only the game that moved: {:?}",
        found.iter().take(3).collect::<Vec<_>>()
    );
}

#[test]
fn a_golden_file_that_was_edited_is_found() {
    // One digit of one digest.
    let mut text = golden::MUSIC_GOLDEN.to_owned();
    let spec = text.find("\n000 ").unwrap();
    let at = spec + text[spec..].find(" d=").unwrap() + 3;
    let c = text.as_bytes()[at];
    text.replace_range(at..=at, if c == b'0' { "1" } else { "0" });
    let found = golden::check_music_real(&text);
    assert!(
        found.iter().any(|f| f.detail.contains("digests to")),
        "{found:?}"
    );
    // A block the build has and the file does not.
    let cut = golden::MUSIC_GOLDEN.rfind("block g=").unwrap();
    let found = golden::check_music_real(&golden::MUSIC_GOLDEN[..cut]);
    assert!(
        found.iter().any(|f| f.detail.contains("no block for it")),
        "{found:?}"
    );
}

#[test]
fn the_items_digest_notices_a_change() {
    let computed = vec![(5_u8, 1_u8, "00".repeat(32))];
    let found = golden::compare_items(
        &format!("g=05 l=01 sha256={}\n", "11".repeat(32)),
        &computed,
    );
    assert_eq!(found.len(), 1);
    assert!(found[0].detail.contains("digest to"));
    // A level the file lacks, a line the build lacks.
    let found = golden::compare_items("", &computed);
    assert!(found[0].detail.contains("no line for it"));
    let found = golden::compare_items(
        &format!("g=06 l=01 sha256={}\n", "11".repeat(32)),
        &computed,
    );
    assert!(found.iter().any(|f| f.detail.contains("does not have")));
}

#[test]
fn the_committed_items_digest_is_current() {
    let games = golden::cells();
    let computed: Vec<(u8, u8, String)> = games
        .iter()
        .map(|(g, l)| (g.id(), *l, golden::items_digest(g, *l).unwrap()))
        .collect();
    let found = golden::compare_items(golden::ITEMS_GOLDEN, &computed);
    assert!(
        found.is_empty(),
        "regenerate on purpose with `cargo xtask ear-check --update-golden`, and say which games and why: {found:#?}"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The anchor

#[test]
fn the_metric_is_sound_1s_on_patches_under_a_table_phrase() {
    let base = params::base();
    let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
    let at = base.value(&cutoff).unwrap();
    let mut moved = base.clone();
    enth_patch::apply(
        &mut moved,
        Op::SetParam {
            addr: cutoff,
            value: at * 1.5,
        },
    )
    .unwrap();
    let challenge = Challenge::new(
        Target::Patch(InlinePatch::new(base.clone())),
        Start::Template { template: 0 },
        HELD,
        ScoringSpec::recreate(0),
    );
    let round = prepare_patch_round(challenge, Compared::OffDefaults).unwrap();
    let want = score_round(&round, &moved).unwrap().sound.jnd;
    let clip = |p: enth_patch::Patch| {
        Playable::of(&Played::Patch {
            patch: Box::new(p),
            phrase: HELD,
        })
        .unwrap()
    };
    let target = clip(base);
    let calib = metric::calibrate(&target).unwrap();
    let got = metric::jnd_between(
        &calib,
        &metric::hear(&target).unwrap(),
        &metric::hear(&clip(moved)).unwrap(),
    );
    assert!(want > 0.0, "the moved cutoff is heard");
    assert!(
        (want - got).abs() <= 1e-9 * want.max(1.0),
        "sound/1 says {want}, ear-check says {got}"
    );
}

#[test]
fn a_music_clip_renders_as_the_session_renders_it() {
    // The scoring seed, a stimulus with a post band (Frequency finder) and one without.
    for game in [Game::FrequencyFinder, Game::IntervalStacks] {
        let item = Shipped::Music(game).item(1, 11, 3).unwrap();
        let played = &item.states[item.key];
        let want = played.render().unwrap().mid();
        let got = Playable::of(played).unwrap().mid(SCORING_SEED).unwrap();
        assert_eq!(want, got, "{}", game.title());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The shipped games

#[test]
fn the_shipped_games_hold_every_hard_check() {
    let games = shipped();
    let refs: Vec<&dyn EarGame> = games.iter().map(|g| g as &dyn EarGame).collect();
    let r = run(&refs, &plan());
    assert!(
        r.failures(Ladders::Auto).is_empty(),
        "{}",
        r.summary(Ladders::Auto)
    );
}

#[test]
fn every_option_of_a_music_question_has_a_clip_and_the_right_one_is_the_question() {
    for game in Game::ALL {
        let g = Shipped::Music(game);
        for level in 1..=game.max_level() {
            for index in 0..40 {
                let item = g.item(level, 0x5EED, index).unwrap();
                if item.options == 0 {
                    assert_eq!(item.states.len(), 0);
                    continue;
                }
                assert_eq!(item.states.len(), item.options, "{}", game.title());
                assert_eq!(item.cues.len(), item.options);
            }
        }
    }
}

#[test]
fn spot_the_change_counterfactuals_move_the_named_control() {
    let g = Shipped::Spot;
    for index in 0..20 {
        let item = g.item(1, 77, index).unwrap();
        let base = params::base();
        assert_eq!(item.layout, Layout::Changed);
        assert_eq!(item.states.len(), item.options + 1);
        for (j, control) in item.controls.iter().enumerate() {
            let addr: ParamAddr = control.parse().unwrap();
            let Played::Patch { patch, .. } = &item.states[1 + j] else {
                panic!("a patch clip")
            };
            assert_ne!(
                patch.value(&addr),
                base.value(&addr),
                "option {j} ({control}) moved nothing"
            );
            // And only that control: every other control on the base is where it was.
            for (k, other) in item.controls.iter().enumerate().filter(|(k, _)| *k != j) {
                let a: ParamAddr = other.parse().unwrap();
                assert_eq!(
                    patch.value(&a),
                    base.value(&a),
                    "option {j} also moved option {k}"
                );
            }
        }
    }
}

#[test]
fn an_ear_check_item_is_the_base_and_the_odd_interval() {
    let item = Shipped::Check.item(1, 5, 2).unwrap();
    assert_eq!(item.layout, Layout::OddOne);
    assert_eq!((item.options, item.states.len()), (3, 2));
    assert_ne!(item.states[0], item.states[1]);
}

#[test]
fn the_results_do_not_depend_on_how_many_threads_run_them() {
    let game = fair_game();
    let plan_a = Plan {
        audible_items: 2,
        bot_items: 3000,
        jobs: 1,
        ..plan()
    };
    let plan_b = Plan {
        jobs: 3,
        ..plan_a.clone()
    };
    let (a, b) = (
        check::check_games(&[&game], &plan_a),
        check::check_games(&[&game], &plan_b),
    );
    assert_eq!(format!("{:?}", a.cells), format!("{:?}", b.cells));
}

// ---------------------------------------------------------------------------------------------------------------
// The staircase

/// A ladder of `n` levels falling by `per_level` JND each from `top`, of three options.
fn ladder(n: usize, top: f64, per_level: f64) -> Ladder {
    Ladder {
        name: "test".to_owned(),
        rows: (0..n)
            .map(|i| Row {
                jnd: (top - per_level * i as f64).max(0.0),
                options: 3,
            })
            .collect(),
    }
}

#[test]
fn the_listener_is_right_at_chance_when_deaf_and_certain_when_it_hears() {
    for m in [2, 3, 4, 8] {
        assert!((staircase::p_afc(0.0, m) - 1.0 / m as f64).abs() < 1e-4);
        assert!(staircase::p_afc(8.0, m) > 0.999);
    }
    // One JND is the distance at which an ideal two-alternative observer is right 75 % of the time.
    assert!((staircase::p_afc(staircase::D_PRIME_AT_JND, 2) - 0.75).abs() < 5e-3);
}

#[test]
fn a_ladder_that_gets_harder_by_whole_jnds_is_placed_within_one_level() {
    // 14 levels from 13 JND down to 0: one JND a level (steep), and low enough at the bottom for every acuity to
    // have its threshold on the ladder.
    let e = staircase::evaluate(&ladder(14, 13.0, 1.0), 200, 7).expect("14 levels");
    assert!(!e.flat && e.monotone);
    assert!(e.in_range >= staircase::MIN_IN_RANGE, "{e:?}");
    assert!(e.within_one >= 0.90 && e.worst_one.0 >= 0.75, "{e:?}");
    assert!(
        (0.75..=0.85).contains(&e.success.0) && (0.75..=0.85).contains(&e.success.1),
        "{e:?}"
    );
    assert!(e.passes(), "{e:?}");
}

#[test]
fn a_flat_ladder_is_judged_within_two_levels() {
    let e = staircase::evaluate(&ladder(12, 6.0, 0.3), 200, 7).expect("12 levels");
    assert!(e.flat, "{e:?}");
    assert!(e.within_two >= e.within_one);
}

#[test]
fn a_ladder_whose_harder_level_is_easier_is_not_monotone() {
    let mut l = ladder(10, 20.0, 2.0);
    l.rows[6].jnd = 15.0;
    let e = staircase::evaluate(&l, 100, 7).unwrap();
    assert!(!e.monotone && !e.passes());
}

#[test]
fn a_ladder_nobody_hears_has_no_listener() {
    let e = staircase::evaluate(&ladder(10, 0.2, 0.0), 100, 7).unwrap();
    assert_eq!(e.in_range, 0);
    assert!(!e.passes());
}

#[test]
fn a_short_ladder_is_not_simulated_and_the_simulation_repeats() {
    assert!(staircase::evaluate(&ladder(3, 9.0, 2.0), 50, 1).is_none());
    let l = ladder(10, 20.0, 2.0);
    assert_eq!(
        staircase::evaluate(&l, 80, 3),
        staircase::evaluate(&l, 80, 3)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The exit policy

#[test]
fn the_exit_policy_is_hard_for_leaks_and_drift_and_for_ladders_once_delivered() {
    let mut r = Report::default();
    r.check("bias", "g L1", false, Gate::Hard, 1, "a leak");
    r.check("audibility", "stub L1", false, Gate::Ladder, 1, "a stub");
    r.check(
        "audibility",
        "ladder L2",
        false,
        Gate::Ladder,
        5,
        "a delivered ladder",
    );
    r.check("bias-render", "g L1", false, Gate::Note, 5, "a note");
    r.check("oracle", "g L1", true, Gate::Hard, 1, "fine");
    let cells = |l: Ladders| {
        names(&r, l)
            .into_iter()
            .map(|(_, c)| c.to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(cells(Ladders::Auto), ["g L1", "ladder L2"]);
    assert_eq!(cells(Ladders::Enforce), ["g L1", "stub L1", "ladder L2"]);
    assert_eq!(cells(Ladders::Report), ["g L1"]);
    assert_eq!(r.reported(Ladders::Auto).len(), 2);
}
