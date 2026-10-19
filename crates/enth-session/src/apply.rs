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

use crate::bridge::{
    GestureStep, HostParam, HostWrite, host_layout_changed, host_updates, slots_written,
};
use crate::link::{Port, Queued};
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

    /// Undo one step and deliver its inverse ops like an edit (it also clears `unloaded_patch`). Ends an open
    /// gesture first: a drag in flight becomes its undo step, closed, and this call undoes it whole. `Ok(false)` if
    /// there is nothing to undo. Lane S2s.
    pub fn undo(&mut self) -> Result<bool, SessionError> {
        self.history_step(Step::Undo)
            .map(|applied| applied.is_some())
    }

    /// Redo one step and deliver it like an edit (it also clears `unloaded_patch`). Ends an open gesture first.
    /// `Ok(false)` if there was nothing to redo. Lane S2s.
    pub fn redo(&mut self) -> Result<bool, SessionError> {
        self.history_step(Step::Redo)
            .map(|applied| applied.is_some())
    }

    /// One undo or redo step with its delivery. `Ok(None)` if there was nothing to do; on `Err` (the patch was changed
    /// behind the history's back so that the step no longer applies) nothing changed and nothing was delivered.
    pub(crate) fn history_step(&mut self, step: Step) -> Result<Option<Applied>, SessionError> {
        // A drag in flight ends first: its undo step is closed (or annulled, if it ended where it started) and its
        // host gestures get their `End`s, so the step below undoes the whole drag and begins and ends stay balanced.
        self.end_gesture();
        self.pump();
        let before = self.patch.clone();
        let moved = match step {
            Step::Undo => self.history.undo(&mut self.patch),
            Step::Redo => self.history.redo(&mut self.patch),
        };
        match moved {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(e) => {
                let err = SessionError::Edit(e);
                self.last_error = Some(err.clone());
                return Err(err);
            }
        }
        // The ops that just changed the patch: an undone entry's inverse is the newest on the redo stack, a redone
        // entry's forward the newest on the undo stack.
        let ops = match step {
            Step::Undo => self.history.redo_stack().last().map(|e| e.inverse.clone()),
            Step::Redo => self.history.undo_stack().last().map(|e| e.forward.clone()),
        }
        .unwrap_or_default();
        self.unloaded_patch = None; // the user now owns the init patch that stood in for an unreadable one
        self.last_error = None;
        Ok(Some(self.deliver(&before, &ops)))
    }

    /// Deliver a transaction that has just changed the patch from `before` ([`delivery`]): the engine's events or one
    /// plan, the host writes of the slots and macros it moved, and the requests for the shell. The same rules as an
    /// edit; used by undo and redo (lane S2s).
    ///
    /// - **Engine** (active only): a `Recompile` marks a plan pending with the slots it writes to forget
    ///   ([`slots_written`]); events go out in op order through the latest sent plan's `PlanMap` and epoch, once per
    ///   parameter (the final value), behind any events already waiting for room, and a full command ring queues
    ///   them (the last per parameter; a queue that overflows becomes a plan instead). A depth the plan has no term
    ///   for becomes a plan too. Then it pumps, so a pending plan goes out at once unless two are in flight.
    /// - **Host**: each slotted parameter or macro the ops moved gets one complete begin, set, end
    ///   ([`host_updates`]); a `SetSlot`'s write only if its plan went out (a write reaching `process()` first would
    ///   go through the old slot table to the previous target; detached there is no `process()`).
    /// - **Shell**: [`ShellRequest::RenameParams`] when the host-visible names or targets changed
    ///   ([`host_layout_changed`]), then the host writes, then [`ShellRequest::PatchEdited`].
    pub(crate) fn deliver(&mut self, before: &Patch, ops: &[Op]) -> Applied {
        let mut applied = Applied::default();
        let mut plan_sent = true;
        if self.port.is_some() {
            let wants_plan = match delivery(before, ops) {
                Delivery::Recompile => true,
                Delivery::Events(routes) => !self.send_events(&routes, &mut applied),
            };
            if wants_plan {
                applied.recompile = true;
                if let Some(port) = &mut self.port {
                    port.compile_pending = true;
                    port.forget_slots |= slots_written(&self.patch, ops);
                }
            }
            self.pump(); // a pending plan goes out at once if the link has room
            plan_sent = self.port.as_ref().is_none_or(|p| !p.compile_pending);
        }
        if ops.iter().any(renames_params) && host_layout_changed(before, &self.patch) {
            applied.renamed = true;
            self.requests.push(ShellRequest::RenameParams);
        }
        applied.host_writes = self.push_host_writes(ops, plan_sent);
        self.requests.push(ShellRequest::PatchEdited);
        applied
    }

    /// Hand the engine the events of a transaction that needs no plan, or report that it does after all. Returns
    /// `false` if a plan is needed (a depth with no term in the plan, or the value queue overflowed).
    fn send_events(&mut self, routes: &[Routing], applied: &mut Applied) -> bool {
        let value_queue = self.config.value_queue;
        let Some(port) = &mut self.port else {
            return true;
        };
        let after = &self.patch;
        let epoch = port.link.epoch();
        let mut commands: Vec<Queued> = Vec::with_capacity(routes.len());
        for route in routes {
            match route {
                Routing::Value(addr) => {
                    // An address the plan does not have (a disabled module's) is carried by the next plan.
                    if let (Some(id), Some(value)) = (port.map.get(addr), after.value(addr)) {
                        put_latest(&mut commands, Queued::Param { epoch, id, value });
                    }
                }
                Routing::Depth(route) => {
                    let depth = after
                        .routes
                        .iter()
                        .find(|r| r.src == route.src && r.dst == route.dst)
                        .map(|r| r.depth);
                    match (port.map.depth(route), depth) {
                        (Some(id), Some(depth)) => {
                            put_latest(&mut commands, Queued::Depth { epoch, id, depth });
                        }
                        _ => return false,
                    }
                }
                Routing::Macro(index) => put_latest(
                    &mut commands,
                    Queued::Macro {
                        index: *index,
                        value: after.macros[index.get()].value,
                    },
                ),
                _ => {}
            }
        }
        for command in commands {
            applied.events += 1;
            if !send_or_queue(port, value_queue, command) {
                return false;
            }
        }
        true
    }

    /// Push the host writes `ops` imply, one begin, set, end per parameter (the last value if several ops moved it).
    /// Returns how many parameters were set.
    fn push_host_writes(&mut self, ops: &[Op], plan_sent: bool) -> u32 {
        let mut writes: Vec<(HostParam, f32)> = Vec::new();
        let mut one = Vec::new();
        for op in ops {
            if matches!(op, Op::SetSlot { .. }) && !plan_sent {
                continue;
            }
            one.clear();
            host_updates(&self.patch, op, &mut one);
            for &(param, norm) in &one {
                match writes.iter_mut().find(|(p, _)| *p == param) {
                    Some(w) => w.1 = norm,
                    None => writes.push((param, norm)),
                }
            }
        }
        for &(param, norm) in &writes {
            for step in [GestureStep::Begin, GestureStep::Set(norm), GestureStep::End] {
                self.requests
                    .push(ShellRequest::Host(HostWrite { param, step }));
            }
        }
        u32::try_from(writes.len()).unwrap_or(u32::MAX)
    }
}

/// Which way a history step goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Undo,
    Redo,
}

/// Add `command` to `list`, replacing an earlier one for the same target (the last value per parameter wins).
pub(crate) fn put_latest(list: &mut Vec<Queued>, command: Queued) {
    match list.iter_mut().find(|q| same_target(q, &command)) {
        Some(slot) => *slot = command,
        None => list.push(command),
    }
}

/// Whether two commands set the same thing on the same plan (so the later one replaces the earlier).
fn same_target(a: &Queued, b: &Queued) -> bool {
    match (a, b) {
        (
            Queued::Param {
                epoch: e, id: i, ..
            },
            Queued::Param {
                epoch: f, id: j, ..
            },
        ) => e == f && i == j,
        (
            Queued::Depth {
                epoch: e, id: i, ..
            },
            Queued::Depth {
                epoch: f, id: j, ..
            },
        ) => e == f && i == j,
        (Queued::Macro { index: i, .. }, Queued::Macro { index: j, .. }) => i == j,
        _ => false,
    }
}

/// Whether a queued command is a value event (audition notes are not, and are never dropped).
fn is_value(q: &Queued) -> bool {
    matches!(
        q,
        Queued::Param { .. } | Queued::Depth { .. } | Queued::Macro { .. }
    )
}

/// Hand `command` to the engine, or hold it. Events already waiting go first (their order is kept), and a command
/// the full ring refuses waits behind them, the last per parameter; the pump flushes the queue as the ring empties.
/// Returns `false` if the queue of value events overflowed `cap`: it is dropped, every value of it is in the patch,
/// and the caller marks a plan pending instead (audition events in the queue are never dropped). Never blocks.
pub(crate) fn send_or_queue(port: &mut Port, cap: usize, command: Queued) -> bool {
    if port.queue.is_empty() {
        let sent = match command {
            Queued::Param { epoch, id, value } => port.link.send_param(epoch, id, value),
            Queued::Depth { epoch, id, depth } => port.link.send_depth(epoch, id, depth),
            Queued::Macro { index, value } => port.link.send_macro(index.get(), value),
            // Audition events are the pump's and the audition calls', not a transaction's.
            Queued::NoteOn { .. } | Queued::NoteOff { .. } => return true,
        };
        if sent.is_ok() {
            return true;
        }
    }
    put_latest(&mut port.queue, command);
    let values = port.queue.iter().filter(|q| is_value(q)).count();
    if values > cap {
        port.queue.retain(|q| !is_value(q));
        port.forget_slots = u64::MAX;
        return false;
    }
    true
}
