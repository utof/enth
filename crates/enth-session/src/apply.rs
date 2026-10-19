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
    /// [`crate::model::SessionEdit::edit`]. In order: pump (collect), treat a `gesture` that is not the open one as
    /// `None` (a drag that outlived a load, which clears the history and restarts its gesture ids), classify against
    /// the patch before, apply (on error nothing changes and nothing is delivered), record (merged into the gesture's
    /// step), clear the kept text of a patch that did not load (the user now owns the init patch), deliver
    /// ([`Session::deliver`]), and push the host writes, renames and "patch edited" for the shell.
    pub fn edit(
        &mut self,
        ops: Vec<Op>,
        gesture: Option<GestureId>,
    ) -> Result<Applied, SessionError> {
        self.pump();
        let gesture = gesture.filter(|g| self.gestures.open == Some(*g));
        if ops.is_empty() {
            return Ok(Applied::default());
        }
        let routing = delivery(&self.patch, &ops);
        let inverse = enth_patch::apply_all(&mut self.patch, &ops)?;
        self.unloaded_patch = None;
        let applied = self.deliver(routing, &ops, gesture.is_some());
        self.history.record(ops, inverse, gesture);
        Ok(applied)
    }

    /// Deliver a transaction that has just been applied to the patch (invariant I2), the shared tail of an edit, an
    /// undo and a redo: `routing` is [`delivery`] of `ops` against the patch **before** them, `in_gesture` whether a
    /// valid gesture is open (host writes then join its host gestures instead of a complete begin, set, end).
    ///
    /// - A recompile marks a compile (adding [`crate::bridge::slots_written`] to the slots the plan forgets), and
    ///   pumps so the plan goes out at once when the link has room; value events are skipped (the plan carries them).
    /// - Otherwise each value, depth and macro event goes to the link with the latest sent plan's epoch and
    ///   `PlanMap` (queued behind a full ring; an overflowing queue becomes a compile). An address missing from the
    ///   map (a disabled module) sends nothing; a depth with no id recompiles.
    /// - Host writes ([`crate::bridge::host_updates`]), the last per parameter: a `SetSlot`'s only if its plan went
    ///   out in this entry (or no engine is active, so nothing can misroute it); see [`crate::link`].
    /// - [`ShellRequest::RenameParams`] when names may change, [`ShellRequest::PatchEdited`] once.
    ///
    /// Clears `last_error` (a successful delivery). Detached, only the requests are pushed.
    pub(crate) fn deliver(&mut self, routing: Delivery, ops: &[Op], in_gesture: bool) -> Applied {
        let mut applied = Applied::default();
        let value_queue = self.config.value_queue;
        match routing {
            Delivery::Recompile => {
                applied.recompile = true;
                let forget = crate::bridge::slots_written(&self.patch, ops);
                if let Some(port) = &mut self.port {
                    port.compile_pending = true;
                    port.forget_slots |= forget;
                }
            }
            Delivery::Events(routes) => {
                for r in routes {
                    match r {
                        Routing::Value(addr) => {
                            let Some(port) = &mut self.port else { continue };
                            if let (Some(id), Some(value)) =
                                (port.map.get(&addr), self.patch.value(&addr))
                            {
                                let epoch = port.link.epoch();
                                port.send_value(
                                    crate::link::Queued::Param { epoch, id, value },
                                    value_queue,
                                );
                                applied.events += 1;
                            }
                        }
                        Routing::Depth(route) => {
                            let Some(port) = &mut self.port else { continue };
                            let depth = self
                                .patch
                                .routes
                                .iter()
                                .find(|x| x.src == route.src && x.dst == route.dst)
                                .map(|x| x.depth);
                            if let (Some(id), Some(depth)) = (port.map.depth(&route), depth) {
                                let epoch = port.link.epoch();
                                port.send_value(
                                    crate::link::Queued::Depth { epoch, id, depth },
                                    value_queue,
                                );
                                applied.events += 1;
                            } else {
                                // No term for it in the plan (a disabled module): a new plan carries it.
                                port.compile_pending = true;
                                applied.recompile = true;
                            }
                        }
                        Routing::Macro(index) => {
                            let Some(port) = &mut self.port else { continue };
                            let value = self.patch.macros[index.get()].value;
                            port.send_value(
                                crate::link::Queued::Macro { index, value },
                                value_queue,
                            );
                            applied.events += 1;
                        }
                        // `delivery` never puts a recompile into events; treat one as a plan to be safe.
                        Routing::Recompile => {
                            if let Some(port) = &mut self.port {
                                port.compile_pending = true;
                            }
                            applied.recompile = true;
                        }
                        Routing::Silent => {}
                    }
                }
            }
        }
        // An entry that leaves a compile pending pumps again, so the plan goes out now when the link has room.
        let plan_sent = match &self.port {
            Some(port) if port.compile_pending => {
                self.pump();
                self.port.as_ref().is_some_and(|p| !p.compile_pending)
            }
            Some(_) => applied.recompile,
            None => true,
        };

        let mut writes: Vec<(HostParam, f32)> = Vec::new();
        for op in ops {
            if matches!(op, Op::SetSlot { .. }) && !plan_sent {
                continue;
            }
            crate::bridge::host_updates(&self.patch, op, &mut writes);
        }
        // The last write per parameter wins (a transaction may set one address twice).
        let mut last: Vec<(HostParam, f32)> = Vec::with_capacity(writes.len());
        for (param, norm) in writes {
            match last.iter_mut().find(|(p, _)| *p == param) {
                Some(entry) => entry.1 = norm,
                None => last.push((param, norm)),
            }
        }
        for (param, norm) in last {
            applied.host_writes += 1;
            if in_gesture {
                if !self.gestures.host_open.contains(&param) {
                    self.gestures.host_open.push(param);
                    self.requests.push(ShellRequest::Host(HostWrite {
                        param,
                        step: GestureStep::Begin,
                    }));
                }
                self.requests.push(ShellRequest::Host(HostWrite {
                    param,
                    step: GestureStep::Set(norm),
                }));
            } else if self.gestures.host_open.contains(&param) {
                // A host gesture a load interrupted is still open (its `End` comes with the next `end_gesture`): set
                // inside it rather than nest a second begin and end.
                self.requests.push(ShellRequest::Host(HostWrite {
                    param,
                    step: GestureStep::Set(norm),
                }));
            } else {
                for step in [GestureStep::Begin, GestureStep::Set(norm), GestureStep::End] {
                    self.requests
                        .push(ShellRequest::Host(HostWrite { param, step }));
                }
            }
        }
        if ops.iter().any(renames_params) {
            applied.renamed = true;
            self.requests.push(ShellRequest::RenameParams);
        }
        if !self.requests.contains(&ShellRequest::PatchEdited) {
            self.requests.push(ShellRequest::PatchEdited);
        }
        self.last_error = None;
        applied
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
