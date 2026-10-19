//! Op effects and the history glue: how an applied op reaches the engine ([`route`], [`delivery`]), and gestures
//! (lane S1s for the types and the classification; SK for the minimal delivery, S2s for undo, redo and the rest).
//!
//! # The delivery table
//!
//! [`enth_patch::ops::effect`] says what an op changes for the audio side; [`route`] turns that into what the session
//! does, given the patch *before* the op:
//!
//! | `ops::effect` | [`Routing`] | The session | The engine |
//! | --- | --- | --- | --- |
//! | `Value(addr)` | `Value` | `PlanMap::get(addr)` → `Link::send_param(epoch, id, plain)`; nothing if the address is not in the plan (a disabled module: the next plan carries it) | base set at the next segment boundary, smoothed |
//! | `Depth(route)`, old and new depth non-zero | `Depth` | `PlanMap::depth(route)` → `Link::send_depth(epoch, id, depth)`; a recompile instead if the id is `None` | ramps over 10 ms, then exactly the recompiled value |
//! | `Depth(route)`, from or to 0 | `Recompile` | a new plan (the compiler leaves depth-0 routes out) | swap, no crossfade |
//! | `Macro(i)` | `Macro` | `Link::send_macro(i, v)` | smoothed over 10 ms |
//! | `Structural` | `Recompile` | a new plan | swap with state carry (moved at an instant swap); crossfade only if modules were added or removed |
//! | `NoAudio` from `SetSlot` | `Recompile` | a new plan: the slot table is compiled into each plan; plus one host write of the new target's value | slot table updated; the remembered host value is forgotten (its key changed) |
//! | `NoAudio` from `SetLabel`, `SetMeta`, `SetMacroName` | `Silent` | names may change ([`renames_params`]) | — |
//!
//! A transaction with any `Recompile` op is delivered as **one** plan compiled from the patch after it, and its value
//! events are skipped (the plan carries them): [`delivery`]. Otherwise its events go out in op order. Classifying
//! every op against the patch before the transaction is exact in that case, because value, depth and macro ops never
//! change how another op classifies.
//!
//! # Gestures
//!
//! A drag is one [`GestureId`]: one undo step (merged by [`History`](enth_patch::history::History) for edits to one
//! address), and one host gesture per slotted address or macro it touches, opened at the first such edit and closed
//! by [`Session::end_gesture`]. Undo and redo end any open gesture first. An edit with no gesture that touches a
//! slotted address sends a complete begin, set, end to the host.

use enth_patch::history::GestureId;
use enth_patch::{MacroIndex, Op, ParamAddr, Patch, RouteAddr};

use crate::bridge::{GestureStep, HostParam, HostWrite};
use crate::model::{Applied, Session, SessionError, ShellRequest};

/// How one applied op reaches the engine. See the [module docs](self) for the table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Routing {
    /// A value event for this parameter, through the latest sent plan's `PlanMap` and epoch.
    Value(ParamAddr),
    /// A depth event for this route (its depth stays non-zero).
    Depth(RouteAddr),
    /// A macro event.
    Macro(MacroIndex),
    /// A new plan.
    Recompile,
    /// Nothing for the engine.
    Silent,
}

/// How a whole transaction reaches the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// One plan compiled from the patch after the transaction; its value events are skipped.
    Recompile,
    /// Events in op order, each [`Routing::Value`], [`Routing::Depth`], [`Routing::Macro`] or [`Routing::Silent`].
    Events(Vec<Routing>),
}

impl Delivery {
    /// Whether this delivery sends a plan.
    pub fn is_recompile(&self) -> bool {
        matches!(self, Delivery::Recompile)
    }
}

/// Classify `op` against the patch **before** it is applied (the depth rule needs the old depth). An op that
/// would not apply classifies as `Recompile`, the safe answer, like [`enth_patch::ops::effect`].
pub fn route(before: &Patch, op: &Op) -> Routing {
    use enth_patch::ops::Effect;
    match enth_patch::ops::effect(before, op) {
        Effect::Value(addr) => Routing::Value(addr),
        Effect::Macro(index) => Routing::Macro(index),
        Effect::Depth(route) => {
            let new = match op {
                Op::SetDepth { depth, .. } => Some(*depth),
                _ => None,
            };
            let old = before
                .routes
                .iter()
                .find(|r| r.src == route.src && r.dst == route.dst)
                .map(|r| r.depth);
            match (old, new) {
                (Some(old), Some(new)) if old != 0.0 && new != 0.0 => Routing::Depth(route),
                _ => Routing::Recompile,
            }
        }
        Effect::NoAudio => match op {
            Op::SetSlot { .. } => Routing::Recompile,
            _ => Routing::Silent,
        },
        // `Structural` and any effect added later: a new plan is always correct.
        _ => Routing::Recompile,
    }
}

/// Classify a transaction against the patch before it: one plan if any op needs one, else its events in order.
pub fn delivery(before: &Patch, ops: &[Op]) -> Delivery {
    let mut events = Vec::with_capacity(ops.len());
    for op in ops {
        match route(before, op) {
            Routing::Recompile => return Delivery::Recompile,
            r => events.push(r),
        }
    }
    Delivery::Events(events)
}

/// Whether applying `op` may change a host parameter's display name: a slot assignment, a module label (slot names
/// use it), a macro name, or adding or removing a module (its slots come and go). Conservative: a rename request is
/// cheap and rare.
pub fn renames_params(op: &Op) -> bool {
    matches!(
        op,
        Op::SetSlot { .. }
            | Op::SetLabel { .. }
            | Op::SetMacroName { .. }
            | Op::AddModule { .. }
            | Op::RemoveModule { .. }
    )
}

/// The open gesture and the host gestures it opened.
#[derive(Debug, Default)]
pub(crate) struct Gestures {
    /// The history gesture, while a drag runs.
    pub(crate) open: Option<GestureId>,
    /// Host parameters this gesture has sent `Begin` for, in order; ending the gesture sends `End` for each.
    pub(crate) host_open: Vec<HostParam>,
}

impl Session {
    /// Open a gesture for a drag (ends any open one first). See [`crate::model::SessionEdit::begin_gesture`].
    pub fn begin_gesture(&mut self) -> GestureId {
        self.end_gesture();
        let id = self.history.begin_gesture(&self.patch);
        self.gestures.open = Some(id);
        id
    }

    /// End the open gesture: its undo step (dropped if the patch is back where it started) and an `End` for each host
    /// gesture it opened. Idempotent; the editor calls it on every pointer release and on every `build()`.
    pub fn end_gesture(&mut self) {
        if self.gestures.open.take().is_some() {
            self.history.end_gesture(&self.patch);
        }
        for param in self.gestures.host_open.drain(..) {
            self.requests.push(ShellRequest::Host(HostWrite {
                param,
                step: GestureStep::End,
            }));
        }
    }

    /// Apply `ops` as one recorded transaction and deliver it ([`delivery`]). See
    /// [`crate::model::SessionEdit::edit`]. Lane SK implements it: pump first (collect), treat a `gesture` that is
    /// not `self.gestures.open` as `None`, apply and record, clear `unloaded_patch` (the user now owns the init
    /// patch), deliver (a recompile adds [`crate::bridge::slots_written`] to `Port::forget_slots`), push the host
    /// writes ([`crate::bridge::host_updates`]; a `SetSlot`'s only if its plan was sent, see [`crate::link`]), and
    /// pump again if a compile is left pending.
    pub fn edit(
        &mut self,
        ops: Vec<Op>,
        gesture: Option<GestureId>,
    ) -> Result<Applied, SessionError> {
        let _ = (ops, gesture);
        Err(SessionError::unsupported("edit", "SK"))
    }

    /// Undo one step and deliver its inverse ops like an edit (it also clears `unloaded_patch`). Lane S2s
    /// implements it.
    pub fn undo(&mut self) -> Result<bool, SessionError> {
        self.end_gesture();
        Err(SessionError::unsupported("undo", "S2s"))
    }

    /// Redo one step and deliver it like an edit (it also clears `unloaded_patch`). Lane S2s implements it.
    pub fn redo(&mut self) -> Result<bool, SessionError> {
        self.end_gesture();
        Err(SessionError::unsupported("redo", "S2s"))
    }
}
