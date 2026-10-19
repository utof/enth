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
//! `status().compile_pending`, and `process()`, while the flag is set and at most once per 50 ms of audio, clears
//! it and schedules one nice-plug background task (`execute_background`, never inline) that locks the session,
//! calls [`Session::pump`] and sets the flag again if a compile is still pending (F1b decision 1). See "Plan
//! delivery" in `docs/session.md` and `docs/plugin.md`.
//!
//! # Never blocks, never drops (invariant I5)
//!
//! - `SendError::InFlight` on a plan: keep the compile pending; the next pump retries.
//! - `SendError::Full` on a value event: queue it (the last value per parameter wins, at most
//!   `SessionConfig::value_queue`); an overflowing queue is cleared and a compile is marked pending instead.
//! - Audition note-offs are queued and never dropped; a note-on that does not fit is refused
//!   ([`SessionError::Busy`]).

use std::sync::Arc;

use enth_engine::{
    DepthId, Engine, EventKind, Link, MAX_PLANS_IN_FLIGHT, NoteKey, ParamId, PlanMap, PrepareCfg,
    SendError, Tap, compile,
};
use enth_patch::{MacroIndex, Patch};

use crate::model::{AUDITION_PORT, Activation, RENDER_SEED, Session, SessionError};
use crate::telemetry::{TelemetryMap, TelemetrySource};

/// Telemetry maps kept: the latest sent plan's and the ones the tap may still publish for a few blocks.
const TELEMETRY_MAPS: usize = MAX_PLANS_IN_FLIGHT + 1;

/// The engine side of an active session.
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
    /// An audition note-on was sent since the last [`Session::audition_all_off`], so a release has something to do.
    pub(crate) auditioned: bool,
}

/// An event waiting for room in the command ring.
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

impl Queued {
    /// A value event (the last per target is kept; a plan carries it), as opposed to an audition note.
    fn is_value(&self) -> bool {
        matches!(
            self,
            Queued::Param { .. } | Queued::Depth { .. } | Queued::Macro { .. }
        )
    }
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

impl Port {
    /// Try to put one event into the command ring.
    fn try_send(&mut self, event: Queued) -> Result<(), SendError<()>> {
        match event {
            Queued::Param { epoch, id, value } => self.link.send_param(epoch, id, value),
            Queued::Depth { epoch, id, depth } => self.link.send_depth(epoch, id, depth),
            Queued::Macro { index, value } => self.link.send_macro(index.get(), value),
            Queued::NoteOn { key, velocity } => {
                self.link.send_event(EventKind::NoteOn { key, velocity })
            }
            Queued::NoteOff { key } => self
                .link
                .send_event(EventKind::NoteOff { key, velocity: 0.0 }),
        }
    }

    /// Send an audition event, or queue it. A note-on that does not fit the audition queue is refused
    /// ([`SessionError::Busy`]); a note-off is always queued (never dropped).
    fn send_audition(&mut self, event: Queued, audition_queue: usize) -> Result<(), SessionError> {
        if self.queue.is_empty() && self.try_send(event).is_ok() {
            return Ok(());
        }
        let notes = self.queue.iter().filter(|q| !q.is_value()).count();
        if matches!(event, Queued::NoteOn { .. }) && notes >= audition_queue {
            return Err(SessionError::Busy);
        }
        self.queue.push(event);
        Ok(())
    }

    /// Move queued events into the ring, in order, while it has room. Returns how many moved.
    fn flush(&mut self) -> usize {
        let mut sent = 0;
        while let Some(&event) = self.queue.first() {
            if self.try_send(event).is_err() {
                break;
            }
            self.queue.remove(0);
            sent += 1;
        }
        sent
    }
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
    /// on the main thread. A second activate replaces the port (deactivate first is not required): audition notes
    /// and queued events of the old engine go with it. The shell folds the host's values first
    /// ([`Session::fold_host`]), so the engine starts from the patch the host sees, and records its own "last seen"
    /// host values right after, so nothing is echoed. On error the session stays as it was (detached if it was).
    pub fn activate(&mut self, activation: Activation) -> Result<Engine, SessionError> {
        let cfg = PrepareCfg::new(activation.sample_rate);
        let (plan, map) = compile(&self.patch, &cfg)?;
        let (mut engine, link) = Engine::with_link(plan, RENDER_SEED, self.config.link);
        let tap = Tap::new(self.config.tap);
        engine.attach_tap(Arc::clone(&tap));
        engine.set_monitor_bypass(self.instance.monitor.fx_bypass);
        self.host.memos.clear(); // a new engine starts its epochs at 0 again (the fold's memos are keyed by epoch)
        self.telemetry_maps.clear();
        self.telemetry_maps
            .push(TelemetryMap::from_plan(engine.epoch(), engine.plan(), &map));
        let source: Arc<dyn TelemetrySource> = tap;
        self.telemetry = Some(source);
        self.port = Some(Port {
            link,
            map,
            activation,
            compile_pending: false,
            forget_slots: 0,
            queue: Vec::with_capacity(self.config.value_queue + self.config.audition_queue),
            next_audition_id: 0,
            auditioned: false,
        });
        Ok(engine)
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
    /// blocks. Detached, there is nothing to do. Callers: every session entry that can send (an edit, undo, redo, a
    /// load, a save, activate), once per editor frame, and the shell's background "pump wanted" task while the
    /// editor is closed. It pushes no [`ShellRequest`](crate::model::ShellRequest), so a pump on the main or a
    /// background thread has nothing to forward.
    ///
    /// A compile is sent only when fewer than [`MAX_PLANS_IN_FLIGHT`] plans are outstanding and the command ring has
    /// room; the plan is compiled from the patch **as it is now** (I3), so queued value events are dropped (the plan
    /// carries them) and audition events stay queued behind it.
    pub fn pump(&mut self) -> PumpReport {
        let mut report = PumpReport::default();
        let Session {
            port,
            patch,
            telemetry_maps,
            last_error,
            ..
        } = self;
        let Some(port) = port.as_mut() else {
            return report;
        };
        report.collected = port.link.collect();
        if port.compile_pending
            && port.link.in_flight() < MAX_PLANS_IN_FLIGHT
            && port.link.free_slots() > 0
        {
            match send_compiled(port, patch) {
                Ok(Some((epoch, map))) => {
                    telemetry_maps.push(map);
                    if telemetry_maps.len() > TELEMETRY_MAPS {
                        let excess = telemetry_maps.len() - TELEMETRY_MAPS;
                        telemetry_maps.drain(..excess);
                    }
                    report.sent_plan = Some(epoch);
                }
                Ok(None) => {}
                Err(e) => {
                    // A valid patch always compiles; retrying every pump would not help. The next edit marks a
                    // new compile.
                    port.compile_pending = false;
                    *last_error = Some(e);
                }
            }
        }
        report.flushed = port.flush();
        report
    }

    /// Play an audition note: `NoteKey { port: AUDITION_PORT, channel: 0, key, note_id: fresh }`
    /// ([`AUDITION_PORT`](crate::model::AUDITION_PORT)) through `Link::send_event(EventKind::NoteOn { .. })`, queued
    /// behind earlier events when the ring is full. Refused while inactive ([`SessionError::Inactive`]), offline
    /// ([`SessionError::Offline`]) or with a full audition queue ([`SessionError::Busy`]). `key` is clamped to
    /// 0–127 and `velocity` to 0–1 (NaN is 0).
    pub fn audition_on(&mut self, key: u8, velocity: f32) -> Result<NoteKey, SessionError> {
        let audition_queue = self.config.audition_queue;
        let Some(port) = &mut self.port else {
            return Err(SessionError::Inactive);
        };
        if port.activation.offline {
            return Err(SessionError::Offline);
        }
        let note = NoteKey {
            port: AUDITION_PORT,
            channel: 0,
            key: i16::from(key.min(127)),
            note_id: port.next_audition_id,
        };
        let velocity = if velocity.is_nan() {
            0.0
        } else {
            velocity.clamp(0.0, 1.0)
        };
        port.send_audition(
            Queued::NoteOn {
                key: note,
                velocity,
            },
            audition_queue,
        )?;
        // Note ids stay non-negative (−1 is the wildcard).
        port.next_audition_id = port.next_audition_id.wrapping_add(1) & i32::MAX;
        port.auditioned = true;
        Ok(note)
    }

    /// Release an audition note (queued if the ring is full; never dropped).
    pub fn audition_off(&mut self, note: NoteKey) -> Result<(), SessionError> {
        let audition_queue = self.config.audition_queue;
        let Some(port) = &mut self.port else {
            return Err(SessionError::Inactive);
        };
        port.send_audition(Queued::NoteOff { key: note }, audition_queue)
    }

    /// Release every audition note (a wildcard note-off on [`AUDITION_PORT`](crate::model::AUDITION_PORT)), if any
    /// was played since the last release.
    pub fn audition_all_off(&mut self) -> Result<(), SessionError> {
        let audition_queue = self.config.audition_queue;
        let Some(port) = &mut self.port else {
            return Err(SessionError::Inactive);
        };
        if !port.auditioned {
            return Ok(());
        }
        let all = NoteKey {
            port: AUDITION_PORT,
            channel: -1,
            key: -1,
            note_id: -1,
        };
        port.send_audition(Queued::NoteOff { key: all }, audition_queue)?;
        port.auditioned = false;
        Ok(())
    }
}

/// Compile `patch` for the port's sample rate, mark it with the slots to forget, and send it. `Ok(None)` when the
/// link refused it (the compile stays pending). On success the port takes the plan's map, the pending flags clear,
/// and queued value events are dropped (the plan carries them); returns the epoch and the plan's telemetry map.
fn send_compiled(
    port: &mut Port,
    patch: &Patch,
) -> Result<Option<(u64, TelemetryMap)>, SessionError> {
    let cfg = PrepareCfg::new(port.activation.sample_rate);
    let (mut plan, map) = compile(patch, &cfg)?;
    plan.forget_host_slots(port.forget_slots);
    let next = port.link.epoch() + 1;
    let telemetry = TelemetryMap::from_plan(next, &plan, &map);
    match port.link.send_plan(plan) {
        Ok(epoch) => {
            debug_assert_eq!(epoch, next, "Link numbers plans consecutively");
            port.map = map;
            port.compile_pending = false;
            port.forget_slots = 0;
            port.queue.retain(|q| !q.is_value());
            Ok(Some((epoch, telemetry)))
        }
        // The plan comes back and is dropped here, on the calling thread; the next pump compiles afresh.
        Err(SendError::InFlight(_) | SendError::Full(_)) => Ok(None),
    }
}
