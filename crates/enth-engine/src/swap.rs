//! The plan swap's plumbing: one `rtrb` command ring (plans and epoch-tagged value events, in order) and a return
//! ring with a pending slot, so the audio thread never frees anything (P1a lane S2b; design.md §3).
//!
//! ```text
//! main thread                                   audio thread (Engine::process, step 1 of each segment)
//! Link::send_plan(plan)  ──Cmd::Plan──▶ ┐
//! Link::send_param(e, id, v) ──Cmd────▶ │
//! Link::send_depth(e, d, pct) ──Cmd───▶ ├─ command ring (ordered) ──▶ poll: swap / apply / drop stale
//! Link::send_macro(i, v) ──Cmd────────▶ │
//! Link::send_event(kind) ──Cmd────────▶ ┘   (audition notes: applied like host events, at step 1)
//! Link::collect() ◀── return ring (+ pending slot) ◀── the replaced plan, after its crossfade
//! ```
//!
//! - **One ring for plans and values**, so a value event can never overtake the plan that defines its ids. Each
//!   value event carries the epoch of the plan its [`ParamId`] came from; the engine drops it (and counts it in
//!   `EngineStats::stale_events`) unless that is the plan it runs.
//! - **The return ring has a pending slot.** If it is full when a plan comes back, the engine keeps the plan and
//!   stops reading commands (events stay queued behind it, in order) until the main thread has made room; nothing
//!   is dropped, so nothing is freed on the audio thread.
//! - **At most two plans in flight** ([`MAX_PLANS_IN_FLIGHT`]): [`Link::send_plan`] gives the plan back while two
//!   sent plans have not yet returned their predecessors; the session coalesces edits meanwhile. A returned plan is
//!   the acknowledgement.
//! - While a crossfade runs (256 frames, only when modules were added or removed), the next plan waits in the ring.
//!
//! The engine end lives in [`crate::Engine`] (built with [`Engine::with_link`](crate::Engine::with_link)); the
//! offline renderer builds an engine without a link and never swaps.

use rtrb::{Consumer, Producer, PushError, RingBuffer};

use crate::event::EventKind;
use crate::params::ParamId;
use crate::plan::{DepthId, Plan};

/// Plans that may be sent and not yet acknowledged (returned) at once.
pub const MAX_PLANS_IN_FLIGHT: usize = 2;

/// A command from the main thread, in arrival order.
pub(crate) enum Cmd {
    /// Swap this plan in (state carried, crossfade if modules were added or removed).
    Plan(Box<Plan>),
    /// Set a parameter's base value; ignored unless `epoch` is the running plan's.
    Param { epoch: u64, id: ParamId, value: f32 },
    /// Set a macro (0-based) or, at index 8, the mod wheel; 0–1, clamped. Plan-independent.
    Macro { index: u8, value: f32 },
    /// Move a route's depth (percent) over 10 ms; ignored unless `epoch` is the running plan's (P1b lane E+).
    Depth { epoch: u64, id: DepthId, depth: f32 },
    /// An event applied at step 1 as if the host had sent it at that boundary (audition notes; P1b lane E+).
    Event(EventKind),
}

/// Ring sizes for [`Engine::with_link`](crate::Engine::with_link).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkCapacity {
    /// Commands the main thread can queue ahead of the audio thread.
    pub commands: usize,
    /// Old plans the audio thread can hand back before the main thread collects them (the pending slot holds one
    /// more).
    pub returns: usize,
}

impl Default for LinkCapacity {
    fn default() -> Self {
        Self {
            commands: 1024,
            returns: 4,
        }
    }
}

/// Why a send failed. The value comes back, so nothing is lost and nothing is dropped.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError<T> {
    /// The command ring is full: retry later (the audio thread empties it every segment).
    Full(T),
    /// Two plans are in flight: collect returned plans first, and coalesce edits meanwhile.
    InFlight(T),
}

impl<T> SendError<T> {
    /// The value that was not sent.
    pub fn into_inner(self) -> T {
        match self {
            SendError::Full(t) | SendError::InFlight(t) => t,
        }
    }
}

/// The main thread's end of an engine's rings: send plans and value events, collect old plans. Not for the audio
/// thread (sending a plan boxes it; collecting drops plans).
pub struct Link {
    cmds: Producer<Cmd>,
    returns: Consumer<Box<Plan>>,
    epoch: u64,
    in_flight: usize,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("epoch", &self.epoch)
            .field("in_flight", &self.in_flight)
            .finish_non_exhaustive()
    }
}

impl Link {
    /// Send a plan; it takes effect at the next segment boundary the audio thread reaches (after any crossfade in
    /// progress). Returns its epoch: tag value events for its [`PlanMap`](crate::PlanMap)'s ids with it.
    ///
    /// # Errors
    /// [`SendError::InFlight`] with two plans in flight, [`SendError::Full`] if the command ring is full; the plan
    /// comes back (boxed).
    pub fn send_plan(&mut self, plan: Plan) -> Result<u64, SendError<Box<Plan>>> {
        let mut plan = Box::new(plan);
        if self.in_flight >= MAX_PLANS_IN_FLIGHT {
            return Err(SendError::InFlight(plan));
        }
        // Only this producer fills the ring, so a slot that is free now stays free until the push.
        if self.cmds.is_full() {
            return Err(SendError::Full(plan));
        }
        let epoch = self.epoch + 1;
        plan.epoch = epoch;
        let pushed = self.cmds.push(Cmd::Plan(plan));
        debug_assert!(pushed.is_ok(), "a free slot was checked just before");
        self.epoch = epoch;
        self.in_flight += 1;
        Ok(epoch)
    }

    /// Set a parameter of the plan with `epoch` (the id comes from that plan's [`PlanMap`](crate::PlanMap)). The
    /// engine ignores it if it runs another plan by then.
    ///
    /// # Errors
    /// [`SendError::Full`] if the command ring is full.
    pub fn send_param(&mut self, epoch: u64, id: ParamId, value: f32) -> Result<(), SendError<()>> {
        self.cmds
            .push(Cmd::Param { epoch, id, value })
            .map_err(|_| SendError::Full(()))
    }

    /// Move the depth of a route of the plan with `epoch` (the id comes from that plan's
    /// [`PlanMap::depth`](crate::PlanMap::depth)) to `depth` percent (−100..100, clamped; NaN is ignored), without
    /// a recompile (P1b lane E+): the depth ramps linearly over 10 ms and then holds exactly the value a recompiled
    /// plan would. The engine ignores it (counted as stale) if it runs another plan by then. A change from or to 0
    /// must be a recompile (`PlanMap::depth` has no id for a depth-0 route).
    ///
    /// # Errors
    /// [`SendError::Full`] if the command ring is full.
    pub fn send_depth(&mut self, epoch: u64, id: DepthId, depth: f32) -> Result<(), SendError<()>> {
        self.cmds
            .push(Cmd::Depth { epoch, id, depth })
            .map_err(|_| SendError::Full(()))
    }

    /// Send an event from the main thread: it applies at the start of the next segment the audio thread renders, as
    /// if the host had sent it at that boundary (P1b lane E+); if the engine's event queue is full then, it waits in
    /// the ring until there is room (never dropped). For **audition notes** (the editor's keyboard,
    /// Recreate), which play whether or not the host transport runs; give them their own `NoteKey::port` so their
    /// `NOTE_END`s can be told apart. Offline engines have no link, so renders cannot see these by construction.
    /// Prefer [`Link::send_param`] for values (an `EventKind::Param` here names whatever plan runs then; an
    /// [`EventKind::EpochParam`] or [`EventKind::Slot`] is safe).
    ///
    /// # Errors
    /// [`SendError::Full`] if the command ring is full.
    pub fn send_event(&mut self, kind: EventKind) -> Result<(), SendError<()>> {
        self.cmds
            .push(Cmd::Event(kind))
            .map_err(|_| SendError::Full(()))
    }

    /// Set macro `index` (0-based, 0–7) to `value` (0–1, clamped by the engine). An index outside 0–7 sends
    /// nothing.
    ///
    /// # Errors
    /// [`SendError::Full`] if the command ring is full.
    pub fn send_macro(&mut self, index: usize, value: f32) -> Result<(), SendError<()>> {
        if index >= crate::modulation::MOD_WHEEL {
            return Ok(());
        }
        self.push_macro(index as u8, value)
    }

    /// Set the mod wheel (0–1).
    ///
    /// # Errors
    /// [`SendError::Full`] if the command ring is full.
    pub fn send_mod_wheel(&mut self, value: f32) -> Result<(), SendError<()>> {
        self.push_macro(crate::modulation::MOD_WHEEL as u8, value)
    }

    fn push_macro(&mut self, index: u8, value: f32) -> Result<(), SendError<()>> {
        self.cmds
            .push(Cmd::Macro { index, value })
            .map_err(|_| SendError::Full(()))
    }

    /// Take back the plans the engine has replaced, and drop them here. Returns how many came back.
    pub fn collect(&mut self) -> usize {
        let mut n = 0;
        while let Ok(plan) = self.returns.pop() {
            drop(plan);
            n += 1;
        }
        self.in_flight = self.in_flight.saturating_sub(n);
        n
    }

    /// The epoch of the latest plan sent (0 before any).
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Plans sent whose predecessors have not come back yet.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Free command slots.
    #[must_use]
    pub fn free_slots(&self) -> usize {
        self.cmds.slots()
    }
}

/// The audio thread's end. Never allocates or frees.
pub(crate) struct AudioLink {
    cmds: Consumer<Cmd>,
    returns: Producer<Box<Plan>>,
    /// A replaced plan the return ring had no room for.
    pending: Option<Box<Plan>>,
}

/// Both ends of a fresh pair of rings (allocates; main thread).
pub(crate) fn rings(capacity: LinkCapacity, epoch: u64) -> (AudioLink, Link) {
    let (cmd_tx, cmd_rx) = RingBuffer::new(capacity.commands.max(1));
    let (ret_tx, ret_rx) = RingBuffer::new(capacity.returns.max(1));
    (
        AudioLink {
            cmds: cmd_rx,
            returns: ret_tx,
            pending: None,
        },
        Link {
            cmds: cmd_tx,
            returns: ret_rx,
            epoch,
            in_flight: 0,
        },
    )
}

impl AudioLink {
    /// Retry handing back the pending plan; `true` if the slot is empty afterwards.
    pub fn flush_pending(&mut self) -> bool {
        if let Some(plan) = self.pending.take()
            && let Err(PushError::Full(plan)) = self.returns.push(plan)
        {
            self.pending = Some(plan);
            return false;
        }
        true
    }

    /// Hand a replaced plan back: onto the return ring, or into the pending slot if the ring is full. The engine
    /// calls it only with the slot empty (it reads no command while the slot is occupied); otherwise the plan comes
    /// back as `Err` for the caller to keep.
    pub fn give_back(&mut self, plan: Box<Plan>) -> Result<(), Box<Plan>> {
        if self.pending.is_some() {
            return Err(plan);
        }
        if let Err(PushError::Full(plan)) = self.returns.push(plan) {
            self.pending = Some(plan);
        }
        Ok(())
    }

    /// Whether a plan waits in the pending slot.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the next command is a plan.
    pub fn next_is_plan(&self) -> bool {
        matches!(self.cmds.peek(), Ok(Cmd::Plan(_)))
    }

    /// Whether the next command is an event (`Link::send_event`).
    pub fn next_is_event(&self) -> bool {
        matches!(self.cmds.peek(), Ok(Cmd::Event(_)))
    }

    /// The next command, if any.
    pub fn pop(&mut self) -> Option<Cmd> {
        self.cmds.pop().ok()
    }
}
