//! **Owner: lane ECK** (the counterfactual that `ear-check` needs, added additively in R4's wave 3); each game's builder
//! here is kept by the ladder lane that tunes the game (G1 intervals and pitch, G2 chords, progressions and rhythm,
//! G3 bands), next to the `question` it mirrors. The reference is docs/ear.md, "`ear-check`".
//!
//! **What would this question sound like if its right answer were option `i`?** Everything else held: the root, the
//! timbre, the key, the voicing, the tempo and the band's Q. [`with_answer`] returns that question for a label game,
//! so a tool can hear every option in the context of one item. Three users need it:
//!
//! - `ear-check` (lane ECK) measures every option pair's distance under `sound/1` in the item's own context, plays the
//!   ideal observer against it, and builds the bias bots' cues (the longest and the loudest option);
//! - the Ear view's replay ("A, B and the answer") can play what a wrong option sounds like (lane EUI);
//! - a ladder lane sees at once, in `cargo test -p enth-music` (`tests/with_answer.rs`), when a change to a game's
//!   `question` leaves its builder behind.
//!
//! **The identity test is the contract.** For every game and level, the builder given the question's own answer
//! reproduces that question exactly (`with_answer(q, q.answer) == q`), and every other option gives a valid question
//! whose sound differs. A ladder change that breaks it is a compile-free reminder to change the builder in the same
//! commit. Tap back has no options, so it has no counterfactual (`None`).
//!
//! The builders read the question where they can (the root note, the velocity, the hold, the band's Q) and the
//! level's row only for what the question does not carry (the rhythm game's tempo and keys), so a new level that only
//! changes a row needs no edit here.

use crate::ear::{Answer, EqBand, Game, NoteSpec, Question, TAIL_MS, clip_length_ms, note};
use crate::ladder::rhythm as rhythm_ladder;
use crate::rhythm::{self, Pattern};
use crate::theory;

/// `q` as it would be if its right answer were option `option`; `None` when the game has no options (Tap back), the
/// option does not exist, or the builder cannot reproduce the question's form (a change to the game's `question`
/// that the builder has not followed: the identity test in `tests/with_answer.rs` names it).
#[must_use]
pub fn with_answer(game: Game, level: u8, q: &Question, option: usize) -> Option<Question> {
    let id = *q.options.ids.get(option)?;
    let (notes, post) = match game {
        Game::IntervalStacks => (intervals(q, id)?, None),
        Game::ChordQuality => (chords(q, id)?, None),
        Game::NoteNaming => (pitch(q, id)?, None),
        Game::FrequencyFinder => (q.notes.clone(), Some(band(q, id)?)),
        Game::Progressions => (progression(q, id)?, None),
        Game::RhythmPattern => (pattern(level, id)?, None),
        Game::TapBack => return None,
    };
    let mut length_ms = clip_length_ms(&notes);
    if game == Game::RhythmPattern {
        length_ms = length_ms.max(rhythm_floor_ms(level)?);
    }
    Some(Question {
        timbre: q.timbre,
        notes,
        length_ms,
        post,
        answer: Answer::Choice(u8::try_from(option).ok()?),
        options: q.options.clone(),
    })
}

/// Interval stacks: the root note, then (above a unison) the key `size` semitones up, struck with it.
fn intervals(q: &Question, size: u32) -> Option<Vec<NoteSpec>> {
    let root = *q.notes.first()?;
    let mut notes = vec![root];
    if size > 0 {
        let key = root.key.checked_add(u8::try_from(size).ok()?)?;
        notes.push(note(key, root.velocity, root.start_ms, root.hold_ms));
    }
    Some(notes)
}

/// Chord quality: the chord of quality `id` in root position over the question's lowest key.
fn chords(q: &Question, quality: u32) -> Option<Vec<NoteSpec>> {
    let first = *q.notes.first()?;
    let root = q.notes.iter().map(|n| n.key).min()?;
    let keys = theory::chord_keys(root, u8::try_from(quality).ok()?, 0)?;
    Some(
        keys.into_iter()
            .map(|k| note(k, first.velocity, first.start_ms, first.hold_ms))
            .collect(),
    )
}

/// Note naming: pitch class `pc` in the question's octave.
fn pitch(q: &Question, pc: u32) -> Option<Vec<NoteSpec>> {
    let first = *q.notes.first()?;
    let key = (first.key / 12 * 12).checked_add(u8::try_from(pc).ok().filter(|pc| *pc < 12)?)?;
    Some(vec![note(
        key,
        first.velocity,
        first.start_ms,
        first.hold_ms,
    )])
}

/// Frequency finder: the same band form at the centre `hz`.
fn band(q: &Question, hz: u32) -> Option<EqBand> {
    let b = q.post?;
    Some(EqBand {
        freq_hz: u16::try_from(hz).ok()?,
        ..b
    })
}

/// Progressions: progression `id` voiced in the question's key. The question does not carry its key, so it is
/// found: the one tonic whose voicing of the question's own progression is the question's notes.
fn progression(q: &Question, id: u32) -> Option<Vec<NoteSpec>> {
    let own = progression_notes_to_id(q)?;
    let first = *q.notes.first()?;
    let voice = |tonic: u8, id: u8| -> Option<Vec<NoteSpec>> {
        let p = theory::progression(id)?;
        let mut notes = Vec::new();
        for (i, chord) in theory::voice_progression(tonic, p).iter().enumerate() {
            for key in chord {
                notes.push(note(
                    *key,
                    first.velocity,
                    u32::try_from(i).ok()? * first.hold_ms,
                    first.hold_ms,
                ));
            }
        }
        Some(notes)
    };
    let tonic = (0..12u8).find(|t| voice(*t, own).is_some_and(|n| n == q.notes))?;
    voice(tonic, u8::try_from(id).ok()?)
}

/// The id of the progression a progression question plays: its right answer.
fn progression_notes_to_id(q: &Question) -> Option<u8> {
    let Answer::Choice(i) = q.answer else {
        return None;
    };
    u8::try_from(*q.options.ids.get(usize::from(i))?).ok()
}

/// Rhythm pattern: the count-in clicks, then two bars of pattern `mask`, as the level plays them.
fn pattern(level: u8, mask: u32) -> Option<Vec<NoteSpec>> {
    let l = rhythm_ladder::LEVELS.get(usize::from(level).checked_sub(1)?)?;
    let step_ms = rhythm::step_ms_for_bpm(l.bpm);
    let bar = rhythm::bar_ms(step_ms);
    let bars = l.count_in_bars + l.pattern_bars;
    let mut notes = rhythm::metronome(
        0,
        bars,
        step_ms,
        l.click_key,
        l.click_velocity,
        l.click_hold_ms,
    );
    for b in 0..l.pattern_bars {
        notes.extend(rhythm::bar_notes(
            Pattern(u16::try_from(mask).ok()?),
            (l.count_in_bars + b) * bar,
            step_ms,
            l.swing_pct,
            l.pattern_key,
            l.velocity,
            l.hold_ms,
        ));
    }
    notes.sort_by_key(|n| n.start_ms);
    Some(notes)
}

/// The length the rhythm game gives its clip at least: every bar plus the tail.
fn rhythm_floor_ms(level: u8) -> Option<u32> {
    let l = rhythm_ladder::LEVELS.get(usize::from(level).checked_sub(1)?)?;
    let bar = rhythm::bar_ms(rhythm::step_ms_for_bpm(l.bpm));
    Some((l.count_in_bars + l.pattern_bars) * bar + TAIL_MS)
}
