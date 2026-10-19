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
//!    (invariant I3), send it, record its epoch and `PlanMap`, and drop queued value events (the plan carries them);
//! 3. otherwise flush queued value and audition events into the ring while it has room.
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
//! The residual case: a compile that is still pending when the editor closes (the audio thread had not returned a
//! plan, for example because the host stopped calling `process()`) waits for the next entry, and until then the
//! engine plays the previous plan. See "open questions" in `docs/session.md`.
//!
//! # Never blocks, never drops (invariant I5)
//!
//! - `SendError::InFlight` on a plan: keep the compile pending; the next pump retries.
//! - `SendError::Full` on a value event: queue it (the last value per parameter wins, at most
//!   `SessionConfig::value_queue`); an overflowing queue is cleared and a compile is marked pending instead.
//! - Audition note-offs are queued and never dropped; a note-on that does not fit is refused
//!   ([`SessionError::Busy`]).

use enth_engine::{Engine, Link, NoteKey, ParamId, PlanMap};
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
    /// Compile the current patch for `activation`, build the engine with [`RENDER_SEED`](crate::model::RENDER_SEED)
    /// and a link ([`Engine::with_link`]), attach the telemetry tap (lane E+), and keep the link and the plan's map.
    /// The shell moves the returned engine to the audio thread; the previous engine, if any, is dropped by the shell
    /// on the main thread. **Host main thread** (nice-plug `initialize`/`activate`). Lane SK implements it.
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
    /// blocks. Detached, there is nothing to do. Lane SK implements the active case.
    pub fn pump(&mut self) -> PumpReport {
        PumpReport::default()
    }

    /// Play an audition note: `NoteKey { port: AUDITION_PORT, channel: 0, key, note_id: fresh }`
    /// ([`AUDITION_PORT`](crate::model::AUDITION_PORT)) through
    /// `Link::send_event` (lane E+). Refused while inactive ([`SessionError::Inactive`]), offline
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
