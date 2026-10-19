//! Delivery to the engine: activation, plans and epochs, the `PlanMap`, the bounded queues behind a full command
//! ring, audition notes, and who drains the return ring (lane S1s for the contract, lane SK for the bodies).
//!
//! # The pump
//!
//! [`Session::pump`] is the one place the session talks to the engine's [`Link`]. It runs at the top of every
//! session entry that can send (an edit, undo, redo, a load, activate) and once per editor frame, and does, in order:
//!
//! 1. `Link::collect()`: take back replaced plans and drop them **here**, on the main or GUI thread, never on the
//!    audio thread;
//! 2. if a compile is pending and `Link::in_flight() < MAX_PLANS_IN_FLIGHT`: compile the patch *as it is now*
//!    (invariant I3), mark it with `Plan::forget_host_slots(port.forget_slots)` (a load since the last send, or a
//!    recompiling edit of slotted values), send it, record its epoch, `PlanMap` and `TelemetryMap` (keep the maps of the
//!    last `MAX_PLANS_IN_FLIGHT + 1` epochs; the tap may still publish an older one), and drop queued value events
//!    (the plan carries them);
//! 3. otherwise flush queued value and audition events into the ring while it has room.
//!
//! An entry that leaves a compile pending pumps **again before it returns** (a load marks one, then pumps), so a
//! plan goes out at once whenever the link has room; the pump at the top only collects first, so `in_flight` is
//! fresh.
//!
//! **A `SetSlot`'s host write needs its plan sent.** The edit pushes the write of the new target's value only if
//! the plan that carries the new slot table went out in the same entry. If the compile stays pending, it pushes
//! none: a write reaching `process()` first would go through the old plan's slot table to the slot's *previous*
//! target, while a host left showing the old value is harmless (rule 2 of [`crate::bridge`] keeps a stale host
//! value out of the patch). So host writes still come only from editor edits on the GUI thread. (A sent plan can
//! still wait a few segments in the ring during a crossfade; the previous target then moves for that long and
//! ramps back at the swap. Rare and short; accepted.)
//!
//! # Who drains the return ring when the editor is closed
//!
//! **The session, at its next entry; nobody needs to before that.** Plans are only sent from session entries, at
//! most [`MAX_PLANS_IN_FLIGHT`](enth_engine::MAX_PLANS_IN_FLIGHT) (2) are outstanding, and the return ring holds 4
//! plus a pending slot, so the audio thread can always hand a plan back: with the editor closed and no host call,
//! the cost is at most two retired
//! plans' memory (about 1.6 MB each with a reverb) until the next entry (a state save, a load, activate,
//! deactivate, or the editor's next frame). No timer is needed, and none runs while the editor is closed.
//!
//! The residual case: a compile that is still pending when the entry returns (two plans in flight: the host had
//! stopped calling `process()`) waits for the next entry, and until then the engine plays the previous plan. The
//! shell closes it without a fork patch: after unlocking it sets an atomic "pump wanted" flag when
//! `status().compile_pending`, and `process()`, once the engine has swapped since the flag was set
//! (`EngineStats::swaps` moved), schedules one nice-plug background task (`execute_background`, never inline)
//! that locks the session and calls [`Session::pump`]. See "Plan delivery" in `docs/session.md`.
//!
//! # Never blocks, never drops (invariant I5)
//!
//! - `SendError::InFlight` on a plan: keep the compile pending; the next pump retries.
//! - `SendError::Full` on a value event: queue it (the last value per parameter wins, at most
//!   `SessionConfig::value_queue`); an overflowing queue is cleared and a compile is marked pending instead.
//! - Audition note-offs are queued and never dropped; a note-on that does not fit is refused
//!   ([`SessionError::Busy`]).

use enth_engine::{DepthId, Engine, Link, NoteKey, ParamId, PlanMap};
use enth_patch::MacroIndex;

use crate::model::{Activation, Session, SessionError};

/// The engine side of an active session.
// Lane SK constructs it in `Session::activate`; until then only `status()` and `deactivate()` read it.
#[expect(
    dead_code,
    reason = "constructed by lane SK's Session::activate (P1b stub)"
)]
pub(crate) struct Port {
    /// The main thread's end of the engine's rings.
    pub(crate) link: Link,
    /// Addresses → parameter ids of the latest *sent* plan (value events use it with `link.epoch()`).
    pub(crate) map: PlanMap,
    /// How the host activated the plugin.
    pub(crate) activation: Activation,
    /// A new plan is needed and has not been sent yet.
    pub(crate) compile_pending: bool,
    /// Slots (bit `i` = slot index `i`) whose remembered host value the next plan sent must forget
    /// ([`Plan::forget_host_slots`](enth_engine::Plan::forget_host_slots)), accumulated until a plan goes out; the
    /// pump passes it to that plan and clears it. [`Session::load_patch`] sets every bit (the loaded patch wins over
    /// the old patch's host values); a recompiling edit, undo or redo adds
    /// [`slots_written`](crate::bridge::slots_written) of its ops (the edited value wins until the host's echo of it
    /// arrives, instead of the old host value being re-applied at the swap).
    pub(crate) forget_slots: u64,
    /// Events held back while the command ring is full, in order.
    pub(crate) queue: Vec<Queued>,
    /// The next audition note id (fresh per note, so overlapping auditions of one key stay distinct).
    pub(crate) next_audition_id: i32,
}

/// An event waiting for room in the command ring.
#[expect(dead_code, reason = "queued by lane SK's delivery (P1b stub)")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Queued {
    /// A parameter value for the plan with `epoch`.
    Param { epoch: u64, id: ParamId, value: f32 },
    /// A route depth (percent) for the plan with `epoch`.
    Depth { epoch: u64, id: DepthId, depth: f32 },
    /// A macro value.
    Macro { index: MacroIndex, value: f32 },
    /// An audition note-on.
    NoteOn { key: NoteKey, velocity: f32 },
    /// An audition note-off (never dropped).
    NoteOff { key: NoteKey },
}

/// What one [`Session::pump`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PumpReport {
    /// Replaced plans taken back from the return ring (and dropped here).
    pub collected: usize,
    /// The epoch of the plan this pump sent, if it sent one.
    pub sent_plan: Option<u64>,
    /// Queued events moved into the command ring.
    pub flushed: usize,
}

impl Session {
    /// Activate: on the **host main thread** (nice-plug `initialize`/`activate`), in this order:
    ///
    /// 1. [`enth_engine::compile`] the current patch for `activation.sample_rate`;
    /// 2. [`Engine::with_link`] with [`RENDER_SEED`](crate::model::RENDER_SEED) and `SessionConfig::link`;
    /// 3. create a [`Tap`](enth_engine::Tap) with `SessionConfig::tap` and [`Engine::attach_tap`] it (before the
    ///    engine leaves this thread); keep it as the [`TelemetrySource`](crate::telemetry::TelemetrySource), disabled;
    /// 4. `Engine::set_monitor_bypass` with the persisted bypass: on a fresh engine the switch is immediate, so a
    ///    project that loads with the bypass on starts without the reverb;
    /// 5. keep the link, the plan's map and its [`TelemetryMap`](crate::telemetry::TelemetryMap) (epoch
    ///    `link.epoch()`).
    ///
    /// The shell moves the returned engine to the audio thread; the previous engine, if any, is dropped by the shell
    /// on the main thread. A second activate replaces the port (deactivate first is not required). Lane SK
    /// implements it.
    pub fn activate(&mut self, activation: Activation) -> Result<Engine, SessionError> {
        let _ = activation;
        Err(SessionError::unsupported("activate", "SK"))
    }

    /// Drop the link, the plan map and the telemetry reader (plans still in the return ring are dropped here). The
    /// shell drops the engine on the main thread right after. **Host main thread** (nice-plug `deactivate`).
    pub fn deactivate(&mut self) {
        self.port = None;
        self.telemetry = None;
        self.telemetry_maps.clear();
    }

    /// Whether an engine is active.
    pub fn is_active(&self) -> bool {
        self.port.is_some()
    }

    /// Collect replaced plans, send a pending compile, flush queued events (see the [module docs](self)). Never
    /// blocks. Detached, there is nothing to do. Lane SK implements the active case. Callers: every session entry
    /// that can send (an edit, undo, redo, a load, a save, activate), once per editor frame, and the shell's
    /// background "pump wanted" task while the editor is closed. It pushes no
    /// [`ShellRequest`](crate::model::ShellRequest), so a pump on the main or a background thread has nothing to
    /// forward.
    pub fn pump(&mut self) -> PumpReport {
        PumpReport::default()
    }

    /// Play an audition note: `NoteKey { port: AUDITION_PORT, channel: 0, key, note_id: fresh }`
    /// ([`AUDITION_PORT`](crate::model::AUDITION_PORT)) through `Link::send_event(EventKind::NoteOn { .. })`. Refused while inactive ([`SessionError::Inactive`]), offline
    /// ([`SessionError::Offline`]) or with a full queue ([`SessionError::Busy`]). Lane SK implements the active case.
    pub fn audition_on(&mut self, key: u8, velocity: f32) -> Result<NoteKey, SessionError> {
        let Some(port) = &self.port else {
            return Err(SessionError::Inactive);
        };
        if port.activation.offline {
            return Err(SessionError::Offline);
        }
        let _ = (key, velocity);
        Err(SessionError::unsupported("audition_on", "SK"))
    }

    /// Release an audition note (queued if the ring is full; never dropped).
    pub fn audition_off(&mut self, note: NoteKey) -> Result<(), SessionError> {
        if self.port.is_none() {
            return Err(SessionError::Inactive);
        }
        let _ = note;
        Err(SessionError::unsupported("audition_off", "SK"))
    }

    /// Release every audition note (a wildcard note-off on [`AUDITION_PORT`](crate::model::AUDITION_PORT)).
    pub fn audition_all_off(&mut self) -> Result<(), SessionError> {
        if self.port.is_none() {
            return Err(SessionError::Inactive);
        }
        Err(SessionError::unsupported("audition_all_off", "SK"))
    }
}
