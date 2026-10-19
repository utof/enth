//! The audio block (lanes SK, PL): host values and notes in, `Engine::process`, audio and `NOTE_END` out, and the
//! rate-limited background pump. Nothing here allocates, frees, locks, logs or does I/O (nice-plug's
//! `assert_process_allocs` aborts debug builds that do).
//!
//! One block, in order:
//!
//! 1. Read the 72 parameters' **modulated** normalised values (atomics) and send each one that changed since the
//!    last block as `EventKind::Macro { index, value }` or `EventKind::Slot { slot, value }` at offset 0: never a
//!    bare `EventKind::Param`, because a plan swap inside the block would misroute an id looked up before it. With
//!    `SAMPLE_ACCURATE_AUTOMATION = true` nice-plug splits the host block at every parameter change, so offset 0 is
//!    the change's own frame and the engine's free-running grid makes the split invisible. After a state load
//!    ([`crate::Shared::rebaseline`]) the restored values are adopted without being sent: the loaded patch wins.
//!    At activate the "last sent" values are the host's current ones, so nothing is echoed.
//! 2. Translate the block's note events ([`crate::notes::engine_event`]) into the preallocated event list; events
//!    beyond its capacity are dropped (it holds nice-plug's input capacity plus all 72 parameters).
//! 3. Hand the engine the monitor bypass, render.
//! 4. Report `NOTE_END`: first the voices a CLAP `reset` freed (the engine reports none for them), then the block's
//!    own ends at `NoteEnd::offset`; audition notes are not the host's and are skipped.
//! 5. Take the telemetry tap's wake flag and request a repaint (lock-free).
//! 6. Watch `EguiEditorState::is_open()` fall (stock nice-plug-egui never calls `editor_closed`): disable the tap
//!    and leave the editor-closed duties to the next pump.
//! 7. While [`crate::Shared::pump_wanted`] is set, at most once per [`PUMP_INTERVAL_S`] of audio: clear it and
//!    `execute_background(Task::Pump)` (one bounded `try_send`, only while a compile is pending; never
//!    `execute_gui`, which can run inline here).

use std::sync::atomic::Ordering;

use enth_engine::{Engine, Event, EventKind, MAX_VOICES, NoteKey};
use enth_patch::model::{MACROS, SLOTS};
use enth_session::HostValues;
use enth_session::bridge::HOST_PARAMS;
use nice_plug::prelude::*;

use crate::params::EnthParams;
use crate::{Enth, Task, notes};

/// The shortest gap between two background pumps `process()` schedules, in seconds of audio.
pub const PUMP_INTERVAL_S: f64 = 0.05;

/// The event list's capacity: every host event nice-plug can deliver in one block, plus all 72 parameters.
pub const EVENT_CAPACITY: usize = <Enth as Plugin>::INPUT_EVENT_CAPACITY + HOST_PARAMS;

/// The audio thread's own state. Allocated once (at construction); `process()` only reuses it.
pub struct Audio {
    /// The host values the engine last heard (modulated).
    last: HostValues,
    /// This block's host values.
    now: HostValues,
    /// This block's engine events.
    events: Vec<Event>,
    /// [`PUMP_INTERVAL_S`] in frames at the active sample rate.
    pump_interval: u64,
    /// Frames since the last background pump was scheduled.
    since_pump: u64,
    /// The editor was open at the last block.
    editor_open: bool,
    /// Voices a CLAP `reset` freed without a report, for the next block's `NOTE_END`s.
    reset_ends: [NoteKey; MAX_VOICES],
    reset_len: usize,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// Fresh state; the event list is allocated here, never in `process()`.
    pub fn new() -> Self {
        Audio {
            last: HostValues::default(),
            now: HostValues::default(),
            events: Vec::with_capacity(EVENT_CAPACITY),
            pump_interval: 1,
            since_pump: 1,
            editor_open: false,
            reset_ends: [NoteKey::new(0, -1); MAX_VOICES],
            reset_len: 0,
        }
    }

    /// At activate (main thread, before the engine plays): the host's current values become the "last sent" ones,
    /// so nothing is echoed and the engine starts from the patch; the pump interval follows the sample rate.
    pub fn activate(&mut self, params: &EnthParams, sample_rate: f32) {
        params.read_modulated(&mut self.last);
        self.now = self.last;
        self.pump_interval = ((PUMP_INTERVAL_S * f64::from(sample_rate)).round() as u64).max(1);
        self.since_pump = self.pump_interval;
        self.reset_len = 0;
    }

    /// Before a CLAP `reset` frees the engine's voices: remember them, so the next block reports their
    /// `NOTE_END`s (hosts that track voices would otherwise wait for them forever).
    pub fn note_reset(&mut self, engine: &Engine) {
        for &v in engine.active_voices() {
            if self.reset_len < MAX_VOICES {
                self.reset_ends[self.reset_len] = engine.voices().key(v);
                self.reset_len += 1;
            }
        }
    }

    /// Step 1: the changed host values as engine events, or none after a load (adopted silently).
    fn host_events(&mut self, adopt: bool) {
        if !adopt {
            for i in 0..MACROS {
                let value = self.now.macros[i];
                if value.to_bits() != self.last.macros[i].to_bits() {
                    self.events.push(Event {
                        offset: 0,
                        kind: EventKind::Macro {
                            index: i as u8,
                            value,
                        },
                    });
                }
            }
            for i in 0..SLOTS {
                let value = self.now.slots[i];
                if value.to_bits() != self.last.slots[i].to_bits() {
                    self.events.push(Event {
                        offset: 0,
                        kind: EventKind::Slot {
                            slot: i as u8,
                            value,
                        },
                    });
                }
            }
        }
        self.last = self.now;
    }

    /// The events this block hands the engine (tests).
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}

impl Enth {
    /// One audio block; see the [module docs](self).
    pub(crate) fn process_block(
        &mut self,
        buffer: &mut Buffer<'_>,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let frames = buffer.samples() as u64;
        let Some(engine) = self.engine.as_mut() else {
            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }
            return ProcessStatus::KeepAlive;
        };
        let audio = &mut self.audio;
        audio.events.clear();

        // 1. Host values.
        self.params.read_modulated(&mut audio.now);
        let adopt = self.shared.rebaseline.swap(false, Ordering::AcqRel);
        audio.host_events(adopt);

        // 2. Notes.
        while let Some(event) = context.next_event() {
            if audio.events.len() < audio.events.capacity()
                && let Some(e) = notes::engine_event(&event)
            {
                audio.events.push(e);
            }
        }

        // 3. Render.
        engine.set_monitor_bypass(self.shared.monitor_bypass.load(Ordering::Acquire));
        // The only layout is stereo out (`AUDIO_IO_LAYOUTS`).
        if let [left, right, ..] = buffer.as_slice() {
            engine.process(left, right, &audio.events);
        }

        // 4. NOTE_END.
        for &key in &audio.reset_ends[..audio.reset_len] {
            if let Some(end) = notes::note_end(key, 0) {
                // A full host queue only drops the report; nothing else to do here.
                let _ = context.try_send_event(end);
            }
        }
        audio.reset_len = 0;
        for end in engine.note_ends() {
            if let Some(end) = notes::reported_end(end) {
                let _ = context.try_send_event(end);
            }
        }

        // 5. Telemetry wake.
        if let Some(tap) = engine.tap()
            && tap.take_wake()
        {
            self.repaint.request_repaint();
        }

        // 6. The editor closed.
        let open = self.editor_state.is_open();
        if audio.editor_open && !open {
            if let Some(tap) = engine.tap() {
                tap.set_enabled(false);
            }
            self.shared.editor_closed.store(true, Ordering::Release);
            self.shared.pump_wanted.store(true, Ordering::Release);
        }
        audio.editor_open = open;

        // 7. The background pump, rate-limited.
        audio.since_pump = audio.since_pump.saturating_add(frames);
        if audio.since_pump >= audio.pump_interval
            && self.shared.pump_wanted.swap(false, Ordering::AcqRel)
        {
            audio.since_pump = 0;
            context.execute_background(Task::Pump);
        }
        ProcessStatus::KeepAlive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_host_values_become_events_and_a_load_adopts_them() {
        let mut audio = Audio::new();
        audio.now.macros[2] = 0.25;
        audio.now.slots[63] = 0.9;
        audio.host_events(false);
        assert_eq!(
            audio.events(),
            [
                Event {
                    offset: 0,
                    kind: EventKind::Macro {
                        index: 2,
                        value: 0.25
                    }
                },
                Event {
                    offset: 0,
                    kind: EventKind::Slot {
                        slot: 63,
                        value: 0.9
                    }
                },
            ]
        );
        audio.events.clear();
        audio.host_events(false);
        assert_eq!(audio.events(), [], "unchanged values send nothing");
        audio.now.slots[0] = 0.1;
        audio.host_events(true);
        assert_eq!(audio.events(), [], "values restored by a load are adopted");
        assert_eq!(audio.last.slots[0], 0.1);
    }

    #[test]
    fn the_event_list_fits_every_host_event_and_every_parameter() {
        let audio = Audio::new();
        assert!(audio.events.capacity() >= EVENT_CAPACITY);
        assert_eq!(EVENT_CAPACITY, 1024 + 72);
    }
}
