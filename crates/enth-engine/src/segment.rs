//! The free-running 32-frame segment grid: the one-segment FIFO that re-chunks host blocks, and the queue that holds
//! events until their segment boundary.
//!
//! **Why a FIFO.** With zero latency, the event quantum must equal the on-demand render unit (P1a report 01, §5). The
//! engine renders one [`SEG`]-frame segment whenever the host needs a frame the FIFO does not hold, applying first
//! every event whose frame is at or before that segment's start; then it copies frames out. The grid is counted from
//! the engine's start, never from the start of a host call, so any sequence of host block sizes produces the same
//! segments, the same event timing and the same samples. An event at frame `f` takes effect at
//! `ceil(f / SEG) · SEG`, and events at the same boundary apply in arrival order. (A grid that restarts at each call
//! is why 48-sample blocks differed from 32 and 64 in the Phase 0 spike; the prototype of this adapter matched the
//! offline render in 1,200 of 1,200 cases over host blocks of 1 to 4,096.)
//!
//! **The queue never allocates.** It is a fixed array sized at construction ([`EVENT_CAPACITY`]); when it is full,
//! a new event is dropped and counted (never a `push` that reallocates). The capacity is far above anything a host
//! sends in one block plus one segment's carry-over.

use crate::event::EventKind;
use crate::module::{Block, SEG};

/// Events the queue holds at once: one host block's events plus those carried to the next segment boundary.
pub const EVENT_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug)]
struct Queued {
    frame: u64,
    kind: EventKind,
}

/// Events waiting for their segment boundary, in arrival order. Fixed capacity; never allocates after `new`.
#[derive(Debug)]
pub struct EventQueue {
    buf: Box<[Option<Queued>]>,
    len: usize,
    dropped: u64,
}

impl EventQueue {
    /// An empty queue holding up to `capacity` events (allocates once).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![None; capacity.max(1)].into_boxed_slice(),
            len: 0,
            dropped: 0,
        }
    }

    /// Queue an event due at absolute frame `frame`. Returns `false` (and counts it) if the queue is full.
    pub fn push(&mut self, frame: u64, kind: EventKind) -> bool {
        if let Some(slot) = self.buf.get_mut(self.len) {
            *slot = Some(Queued { frame, kind });
            self.len += 1;
            true
        } else {
            self.dropped += 1;
            false
        }
    }

    /// Hand every event due at or before `frame` to `apply`, in arrival order, and keep the rest in order.
    pub fn drain_due(&mut self, frame: u64, mut apply: impl FnMut(EventKind)) {
        let mut kept = 0;
        for i in 0..self.len {
            let Some(q) = self.buf[i].take() else {
                continue;
            };
            if q.frame <= frame {
                apply(q.kind);
            } else {
                self.buf[kept] = Some(q);
                kept += 1;
            }
        }
        self.len = kept;
    }

    /// Forget every waiting event, keeping the capacity and the dropped count ([`crate::Engine::reset`]). Never
    /// allocates.
    pub fn clear(&mut self) {
        for slot in &mut self.buf[..self.len] {
            *slot = None;
        }
        self.len = 0;
    }

    /// Events waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Events dropped because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }
}

/// One rendered segment of stereo output and how much of it the host has taken.
#[derive(Clone, Debug)]
pub struct Fifo {
    left: Block,
    right: Block,
    read: usize,
}

impl Fifo {
    /// An empty FIFO: the first frame the host asks for triggers a render.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            left: [0.0; SEG],
            right: [0.0; SEG],
            read: SEG,
        }
    }

    /// Whether every frame has been read (the next frame needs a new segment).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.read >= SEG
    }

    /// Store a freshly rendered segment.
    pub fn fill(&mut self, left: &Block, right: &Block) {
        self.left = *left;
        self.right = *right;
        self.read = 0;
    }

    /// Copy up to `left.len()` frames out; returns how many were copied.
    pub fn read_into(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let n = (SEG - self.read).min(left.len()).min(right.len());
        left[..n].copy_from_slice(&self.left[self.read..self.read + n]);
        right[..n].copy_from_slice(&self.right[self.read..self.read + n]);
        self.read += n;
        n
    }
}

impl Default for Fifo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NoteKey;

    fn on(key: u8) -> EventKind {
        EventKind::NoteOn {
            key: NoteKey::new(key, 0),
            velocity: 1.0,
        }
    }

    fn key_of(e: EventKind) -> u8 {
        match e {
            EventKind::NoteOn { key, .. } => key.key as u8,
            _ => 0,
        }
    }

    #[test]
    fn due_events_apply_in_arrival_order_and_the_rest_wait_in_order() {
        let mut q = EventQueue::new(8);
        q.push(40, on(1));
        q.push(10, on(2));
        q.push(32, on(3));
        q.push(70, on(4));
        q.push(32, on(5));
        let mut got = Vec::new();
        q.drain_due(32, |e| got.push(key_of(e)));
        assert_eq!(got, [2, 3, 5]);
        assert_eq!(q.len(), 2);
        got.clear();
        q.drain_due(64, |e| got.push(key_of(e)));
        assert_eq!(got, [1]);
        got.clear();
        q.drain_due(96, |e| got.push(key_of(e)));
        assert_eq!(got, [4]);
        assert!(q.is_empty());
    }

    #[test]
    fn a_full_queue_drops_and_counts() {
        let mut q = EventQueue::new(2);
        assert!(q.push(0, on(1)));
        assert!(q.push(0, on(2)));
        assert!(!q.push(0, on(3)));
        assert_eq!((q.len(), q.dropped(), q.capacity()), (2, 1, 2));
        q.drain_due(0, |_| {});
        assert!(q.push(0, on(4)), "room again after a drain");
    }

    #[test]
    fn the_fifo_hands_out_one_segment_in_any_chunks() {
        let mut f = Fifo::new();
        assert!(f.is_empty());
        let l: Block = core::array::from_fn(|i| i as f32);
        let r: Block = core::array::from_fn(|i| -(i as f32));
        f.fill(&l, &r);
        let (mut ol, mut or) = ([0.0; 40], [0.0; 40]);
        assert_eq!(f.read_into(&mut ol[..5], &mut or[..5]), 5);
        assert_eq!(f.read_into(&mut ol[5..], &mut or[5..]), SEG - 5);
        assert!(f.is_empty());
        assert_eq!(&ol[..SEG], &l[..]);
        assert_eq!(&or[..SEG], &r[..]);
    }
}
