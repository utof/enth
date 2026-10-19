//! The host's notes that the shell handed the engine and has not reported ended yet (lane PL), so every host
//! note-on gets **exactly one** `NOTE_END`, which is what CLAP hosts that track voices (and poly-mod) rely on.
//!
//! The engine reports a voice's end ([`Engine::note_ends`]), but some notes end without a report:
//!
//! - more than `enth_engine::voice::NOTE_END_CAPACITY` (256) voices ending in one `Engine::process` call (an event
//!   flood): the rest are counted, not reported;
//! - in mono and legato the one voice takes the new note's key, so the note it played before is never reported (and
//!   a return to a held key brings an already-replaced key back);
//! - a note-on still waiting for its segment boundary when a CLAP `reset` empties the engine's queue, or one the
//!   engine's queue had no room for.
//!
//! So the ledger keeps every live host note (its tuple and the absolute frame of its note-on). After each render:
//!
//! 1. each engine report whose note is live is sent and the note forgotten; a report for a note that is not live
//!    (already reported, or an editor audition note) is not sent again;
//! 2. a **sweep** sends and forgets every live note whose note-on the engine has applied (the segment boundary at or
//!    after its frame has been rendered) and that no active voice plays any more.
//!
//! At a reset every live note is reported at frame 0 of the next block. Allocation-free after construction: a
//! fixed-capacity list, `swap_remove` to forget. Costs one scan of the active voices per live note per chunk
//! (a few dozen comparisons in normal playing).

use enth_engine::segment::EVENT_CAPACITY as ENGINE_EVENTS;
use enth_engine::{Engine, MAX_VOICES, NoteKey, SEG};

/// Live notes the ledger tracks at most: everything the engine's event queue can hold plus every voice. A note-on
/// past it is played but not tracked (counted in [`Ledger::untracked`]).
pub const CAPACITY: usize = ENGINE_EVENTS + MAX_VOICES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Live {
    key: NoteKey,
    /// The absolute engine frame of its note-on.
    frame: u64,
}

/// See the [module docs](self).
#[derive(Debug)]
pub struct Ledger {
    live: Vec<Live>,
    untracked: u64,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

impl Ledger {
    /// Empty; allocates its list once, here.
    #[must_use]
    pub fn new() -> Self {
        Ledger {
            live: Vec::with_capacity(CAPACITY),
            untracked: 0,
        }
    }

    /// Forget everything (a new engine: the host forgets its voices at deactivation).
    pub fn clear(&mut self) {
        self.live.clear();
    }

    /// A host note-on handed to the engine at absolute frame `frame`.
    pub fn started(&mut self, key: NoteKey, frame: u64) {
        if self.live.len() < CAPACITY {
            self.live.push(Live { key, frame });
        } else {
            self.untracked += 1;
        }
    }

    /// The engine reported `key` ended: `true` (and forgotten) if it was live, `false` if it was already reported
    /// or is not the host's.
    pub fn ended(&mut self, key: NoteKey) -> bool {
        match self.live.iter().position(|l| l.key == key) {
            Some(i) => {
                self.live.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// After a render: forget, and pass to `report`, every live note whose note-on the engine has applied and that
    /// no active voice plays.
    pub fn sweep(&mut self, engine: &Engine, mut report: impl FnMut(NoteKey)) {
        let position = engine.position();
        let seg = SEG as u64;
        let voices = engine.voices();
        let mut i = 0;
        while i < self.live.len() {
            let l = self.live[i];
            let applied = position > l.frame.div_ceil(seg) * seg;
            let playing = || {
                engine
                    .active_voices()
                    .iter()
                    .any(|&v| voices.key(v) == l.key)
            };
            if applied && !playing() {
                self.live.swap_remove(i);
                report(l.key);
            } else {
                i += 1;
            }
        }
    }

    /// Forget every live note, passing each to `report` (the voices a reset freed, and notes it dropped from the
    /// engine's queue).
    pub fn drain(&mut self, mut report: impl FnMut(NoteKey)) {
        for l in self.live.drain(..) {
            report(l.key);
        }
    }

    /// Live notes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Whether no note is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Note-ons played but not tracked because the ledger was full.
    #[must_use]
    pub fn untracked(&self) -> u64 {
        self.untracked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_engine::{Event, EventKind};

    fn engine(mode: &str) -> Engine {
        let mut patch = enth_patch::Patch::init();
        if mode != "poly" {
            let value = if mode == "mono" { 1.0 } else { 2.0 };
            enth_patch::apply(
                &mut patch,
                enth_patch::Op::SetParam {
                    addr: "voice.mode".parse().unwrap(),
                    value,
                },
            )
            .unwrap();
        }
        let cfg = enth_engine::PrepareCfg::new(48_000.0);
        let (plan, _) = enth_engine::compile(&patch, &cfg).unwrap();
        Engine::new(plan, 1)
    }

    fn run(
        engine: &mut Engine,
        ledger: &mut Ledger,
        frames: usize,
        events: &[Event],
        out: &mut Vec<NoteKey>,
    ) {
        for e in events {
            if let EventKind::NoteOn { key, .. } = e.kind {
                ledger.started(key, engine.position() + u64::from(e.offset));
            }
        }
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        engine.process(&mut l, &mut r, events);
        for end in engine.note_ends() {
            if ledger.ended(end.key) {
                out.push(end.key);
            }
        }
        ledger.sweep(engine, |k| out.push(k));
    }

    fn note(offset: u32, key: u8, id: i32, on: bool) -> Event {
        let key = NoteKey::new(key, id);
        Event {
            offset,
            kind: if on {
                EventKind::NoteOn { key, velocity: 0.8 }
            } else {
                EventKind::NoteOff { key, velocity: 0.0 }
            },
        }
    }

    #[test]
    fn a_queued_note_is_not_swept_before_its_segment_and_a_poly_note_ends_once() {
        let mut e = engine("poly");
        let mut ledger = Ledger::new();
        let mut ends = Vec::new();
        // A note-on at frame 5 of a 10-frame block: its boundary (32) is not rendered yet.
        run(&mut e, &mut ledger, 10, &[note(5, 60, 0, true)], &mut ends);
        assert_eq!((ledger.len(), ends.len()), (1, 0));
        run(
            &mut e,
            &mut ledger,
            100,
            &[note(50, 60, 0, false)],
            &mut ends,
        );
        for _ in 0..200 {
            run(&mut e, &mut ledger, 480, &[], &mut ends);
        }
        assert_eq!(ends, [NoteKey::new(60, 0)]);
        assert!(ledger.is_empty());
    }

    #[test]
    fn in_mono_the_replaced_note_ends_when_the_voice_takes_the_new_one_and_never_twice() {
        for mode in ["mono", "legato"] {
            let mut e = engine(mode);
            let mut ledger = Ledger::new();
            let mut ends = Vec::new();
            run(&mut e, &mut ledger, 64, &[note(0, 60, 0, true)], &mut ends);
            run(&mut e, &mut ledger, 64, &[note(0, 64, 1, true)], &mut ends);
            assert_eq!(
                ends,
                [NoteKey::new(60, 0)],
                "{mode}: the voice plays note 1 now"
            );
            // Releasing note 1 returns the voice to the held note 0, which the host was told ended: no second end.
            run(&mut e, &mut ledger, 64, &[note(0, 64, 1, false)], &mut ends);
            run(&mut e, &mut ledger, 64, &[note(0, 60, 0, false)], &mut ends);
            for _ in 0..200 {
                run(&mut e, &mut ledger, 480, &[], &mut ends);
            }
            assert_eq!(ends, [NoteKey::new(60, 0), NoteKey::new(64, 1)], "{mode}");
            assert!(ledger.is_empty());
        }
    }

    #[test]
    fn more_ends_than_the_engine_reports_in_one_call_are_still_reported() {
        let mut e = engine("poly");
        let mut ledger = Ledger::new();
        let mut ends = Vec::new();
        // 600 notes in one segment: most are cut at once, far past the engine's 256 reports per call.
        let events: Vec<Event> = (0..600)
            .flat_map(|i| {
                [
                    note(0, (i % 100) as u8, i, true),
                    note(0, (i % 100) as u8, i, false),
                ]
            })
            .collect();
        run(&mut e, &mut ledger, 64, &events, &mut ends);
        for _ in 0..200 {
            run(&mut e, &mut ledger, 480, &[], &mut ends);
        }
        assert!(
            e.stats().dropped_note_ends > 0,
            "the engine's report overflowed"
        );
        let mut ids: Vec<i32> = ends.iter().map(|k| k.note_id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..600).collect::<Vec<_>>());
    }

    #[test]
    fn drain_reports_everything_once() {
        let mut ledger = Ledger::new();
        ledger.started(NoteKey::new(1, 1), 0);
        ledger.started(NoteKey::new(2, 2), 40);
        let mut out = Vec::new();
        ledger.drain(|k| out.push(k));
        assert_eq!(out.len(), 2);
        assert!(ledger.is_empty());
        assert!(!ledger.ended(NoteKey::new(1, 1)));
    }
}
