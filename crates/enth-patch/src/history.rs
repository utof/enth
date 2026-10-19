//! Undo and redo over inverse ops; drags merge only edits to the same address (P1a lane S1b).
//!
//! [`History`] is pure data: two stacks of [`Entry`]s (`forward` and `inverse` op lists, as [`apply_all`] returns
//! them) plus the bookkeeping for gestures and the dirty flag. `enth-session` owns one and drives it; host
//! automation applies `SetParam` with [`apply`](crate::ops::apply) directly and records nothing.
//!
//! # Gestures and merging
//!
//! A knob drag is a *gesture*: many `SetParam`s, one undo step. Callers pass the same [`GestureId`] for every edit
//! of a drag ([`History::begin_gesture`] hands them out). Consecutive records of one gesture fold into one entry:
//!
//! - **same address:** a single `SetParam` that follows a `SetParam` on the same address replaces it: the entry
//!   keeps the *first* inverse (the value before the drag) and takes the *last* forward. The same holds for a
//!   `SetDepth` on the same route (a depth drag) and a `SetMacro` on the same macro (P1b, F1b review), so a drag
//!   costs one op pair however many frames it lasts;
//! - **another address (or any other op):** concatenated, forward in order and inverse in reverse, so nothing is
//!   lost. (Merging across addresses loses undo data: the history property test caught exactly that in the first
//!   draft.)
//!
//! A record with no gesture, or with another one, starts a new entry. [`History::end_gesture`] **annuls** a gesture
//! that ended where it started (the patch equals what it was at `begin_gesture`, bit for bit): its entry is dropped,
//! so wiggling a knob back to its start leaves no undo step.
//!
//! Calling undo or redo (even with nothing to undo) ends any open gesture. Recording clears the redo stack. The undo stack holds at most
//! [`History::CAP`] entries (oldest dropped). `mark_saved` / `is_dirty` follow the save point through undo, redo
//! and merging.
//!
//! The entries' `inverse` lists must be what [`apply_all`] returned for their `forward` (the merge relies on the
//! first inverse op undoing the last forward op).

use crate::canon::identical;
use crate::model::Patch;
use crate::ops::{Op, TxError, apply_all};

/// Names one gesture (a knob drag). Hand them out with [`History::begin_gesture`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GestureId(pub u64);

/// One undo step.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The ops that did it, in order.
    pub forward: Vec<Op>,
    /// The ops that undo it, in order (what [`apply_all`] returned).
    pub inverse: Vec<Op>,
    /// The gesture it belongs to, if any.
    pub gesture: Option<GestureId>,
    before: u64,
    after: u64,
}

struct OpenGesture {
    id: GestureId,
    start: Patch,
    start_state: u64,
}

/// Undo and redo stacks with gesture merging. See the [module docs](self).
pub struct History {
    done: Vec<Entry>,
    undone: Vec<Entry>,
    cap: usize,
    next_gesture: u64,
    open: Option<OpenGesture>,
    /// The gesture whose entry is the last one on `done` and may still take merges.
    merging: Option<GestureId>,
    clock: u64,
    current: u64,
    saved: Option<u64>,
}

impl Default for History {
    fn default() -> Self {
        History::new()
    }
}

impl History {
    /// The most undo steps kept.
    pub const CAP: usize = 1000;

    /// An empty history; its (empty) state counts as saved.
    pub fn new() -> Self {
        History::with_cap(Self::CAP)
    }

    /// An empty history keeping at most `cap` undo steps (at least 1).
    pub fn with_cap(cap: usize) -> Self {
        History {
            done: Vec::new(),
            undone: Vec::new(),
            cap: cap.max(1),
            next_gesture: 0,
            open: None,
            merging: None,
            clock: 0,
            current: 0,
            saved: Some(0),
        }
    }

    /// Start a gesture. `patch` is the state before it, kept to annul a gesture that ends where it started.
    pub fn begin_gesture(&mut self, patch: &Patch) -> GestureId {
        let id = GestureId(self.next_gesture);
        self.next_gesture += 1;
        self.open = Some(OpenGesture {
            id,
            start: patch.clone(),
            start_state: self.current,
        });
        self.merging = None;
        id
    }

    /// End the open gesture. If `patch` equals the state at `begin_gesture`, the gesture's undo step is dropped.
    pub fn end_gesture(&mut self, patch: &Patch) {
        self.merging = None;
        let Some(open) = self.open.take() else {
            return;
        };
        let alone = self
            .done
            .last()
            .is_some_and(|e| e.gesture == Some(open.id) && e.before == open.start_state);
        if alone
            && identical(patch, &open.start)
            && let Some(e) = self.done.pop()
        {
            self.current = e.before;
        }
    }

    /// Record a transaction that has been applied: `forward` and the `inverse` that [`apply_all`] returned. A new
    /// record clears the redo stack. Empty transactions are ignored.
    pub fn record(&mut self, forward: Vec<Op>, inverse: Vec<Op>, gesture: Option<GestureId>) {
        if forward.is_empty() && inverse.is_empty() {
            return;
        }
        self.undone.clear();
        self.clock += 1;
        let after = self.clock;
        if let Some(g) = gesture
            && self.merging == Some(g)
            && let Some(last) = self.done.last_mut()
            && last.gesture == Some(g)
        {
            merge(last, forward, inverse);
            last.after = after;
            self.current = after;
            return;
        }
        self.done.push(Entry {
            forward,
            inverse,
            gesture,
            before: self.current,
            after,
        });
        self.current = after;
        self.merging = gesture;
        if self.done.len() > self.cap {
            self.done.remove(0);
        }
    }

    /// Apply `ops` atomically and record them. On error nothing changes.
    pub fn edit(
        &mut self,
        patch: &mut Patch,
        ops: Vec<Op>,
        gesture: Option<GestureId>,
    ) -> Result<(), TxError> {
        let inverse = apply_all(patch, &ops)?;
        self.record(ops, inverse, gesture);
        Ok(())
    }

    /// Undo the last step. `Ok(false)` if there is nothing to undo; on `Err` (the patch was changed behind the
    /// history's back so that the inverse no longer applies) nothing changes.
    pub fn undo(&mut self, patch: &mut Patch) -> Result<bool, TxError> {
        self.open = None;
        self.merging = None;
        let Some(entry) = self.done.pop() else {
            return Ok(false);
        };
        if let Err(e) = apply_all(patch, &entry.inverse) {
            self.done.push(entry);
            return Err(e);
        }
        self.current = entry.before;
        self.undone.push(entry);
        Ok(true)
    }

    /// Redo the last undone step. `Ok(false)` if there is nothing to redo.
    pub fn redo(&mut self, patch: &mut Patch) -> Result<bool, TxError> {
        self.open = None;
        self.merging = None;
        let Some(entry) = self.undone.pop() else {
            return Ok(false);
        };
        if let Err(e) = apply_all(patch, &entry.forward) {
            self.undone.push(entry);
            return Err(e);
        }
        self.current = entry.after;
        self.done.push(entry);
        Ok(true)
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// The undo steps, oldest first.
    pub fn undo_stack(&self) -> &[Entry] {
        &self.done
    }

    /// The redo steps, the next one to redo last.
    pub fn redo_stack(&self) -> &[Entry] {
        &self.undone
    }

    /// Remember the current state as the saved one.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current);
    }

    /// Whether the patch differs from the last saved state (through undo and redo; a saved state that was dropped
    /// from the history stays dirty).
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current)
    }

    /// Forget everything (a new or freshly loaded patch); the new state counts as saved.
    pub fn clear(&mut self) {
        *self = History::with_cap(self.cap);
    }
}

fn merge(last: &mut Entry, mut forward: Vec<Op>, mut inverse: Vec<Op>) {
    // One value op that follows a value op on the same target: a knob (`SetParam` per address), a depth handle
    // (`SetDepth` per route) or a macro knob (`SetMacro` per index). Each one's inverse is a single op of its own
    // kind, so the entry's first inverse still undoes the last forward.
    let same_address = match (forward.as_slice(), last.forward.last()) {
        ([Op::SetParam { addr: a, .. }], Some(Op::SetParam { addr: b, .. })) => a == b,
        ([Op::SetDepth { route: a, .. }], Some(Op::SetDepth { route: b, .. })) => a == b,
        ([Op::SetMacro { index: a, .. }], Some(Op::SetMacro { index: b, .. })) => a == b,
        _ => false,
    };
    if same_address {
        // Keep the first inverse (the value before the drag), take the latest forward.
        if let (Some(new), Some(slot)) = (forward.pop(), last.forward.last_mut()) {
            *slot = new;
        }
    } else {
        last.forward.append(&mut forward);
        inverse.append(&mut last.inverse);
        last.inverse = inverse;
    }
}
