//! The counterfactual of `ladder::with_answer` (lane ECK; docs/ear.md, "`ear-check`"): for every music game and level,
//! the question rebuilt for its own answer **is** the question, and every other option gives a valid question that
//! sounds different. A ladder change that leaves its builder behind fails here, in the ladder lane's own gate.

use enth_music::ear::{Answer, Game, MAX_SEED, question};
use enth_music::ladder::with_answer::with_answer;

const SEED: u64 = 0x0000_4543_4B5F_7631 & MAX_SEED;
const ITEMS: u32 = 300;

#[test]
fn a_question_rebuilt_for_its_own_answer_is_itself() {
    for game in Game::ALL {
        for level in 1..=game.max_level() {
            for item in 0..ITEMS {
                let q = question(game, level, SEED, item).unwrap();
                let Answer::Choice(answer) = q.answer else {
                    // Tap back: no options, no counterfactual.
                    assert_eq!(with_answer(game, level, &q, 0), None, "{game:?}");
                    continue;
                };
                let same = with_answer(game, level, &q, usize::from(answer));
                assert_eq!(
                    same.as_ref(),
                    Some(&q),
                    "{game:?} level {level} item {item}: the builder no longer reproduces the generator's question; \
                     change `ladder::with_answer` in the same commit as the game's `question`"
                );
            }
        }
    }
}

#[test]
fn every_other_option_is_a_valid_question_that_sounds_different() {
    for game in Game::ALL {
        for level in 1..=game.max_level() {
            for item in 0..ITEMS {
                let q = question(game, level, SEED, item).unwrap();
                if matches!(q.answer, Answer::Taps { .. }) {
                    continue;
                }
                let all: Vec<_> = (0..q.options.ids.len())
                    .map(|j| {
                        with_answer(game, level, &q, j).unwrap_or_else(|| {
                            panic!("{game:?} level {level} item {item}: no counterfactual for option {j}")
                        })
                    })
                    .collect();
                for (j, c) in all.iter().enumerate() {
                    c.validate().unwrap_or_else(|e| {
                        panic!("{game:?} level {level} item {item} option {j}: {e}")
                    });
                    assert_eq!(c.answer, Answer::Choice(j as u8));
                    assert_eq!(c.options, q.options);
                    assert_eq!(c.timbre, q.timbre, "the timbre is context, not the answer");
                }
                for (i, a) in all.iter().enumerate() {
                    for (j, b) in all.iter().enumerate().skip(i + 1) {
                        assert!(
                            a.notes != b.notes || a.post != b.post,
                            "{game:?} level {level} item {item}: options {i} and {j} sound the same"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_missing_option_has_no_counterfactual() {
    let q = question(Game::IntervalStacks, 1, SEED, 0).unwrap();
    assert_eq!(with_answer(Game::IntervalStacks, 1, &q, q.options.ids.len()), None);
}
