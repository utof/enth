//! The audio block (lanes SK, PL): host values and notes in, `Engine::process`, audio and `NOTE_END` out, the
//! rate-limited background pump, the parameter-info rescan request and CLAP voice info. Nothing here allocates,
//! frees, locks, logs or does I/O (nice-plug's `assert_process_allocs` aborts debug builds that do).
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
//! 2. After a CLAP `reset` (which frees the engine's voices and empties its queue without a report), report every
//!    live host note's `NOTE_END` at frame 0 ([`crate::ledger`]).
//! 3. Translate the block's note events ([`crate::notes::engine_event`]) into the preallocated event list and render
//!    with the monitor bypass handed to the engine first. When the list is full (more than [`EVENT_CAPACITY`]
//!    events), the frames up to the event that did not fit are rendered with the list, and the list starts again
//!    from there: the engine renders the same bits for any split of a block (its grid is free-running), so no host
//!    event is ever dropped. After each chunk, its `NOTE_END`s go out at their frames in the host block, through
//!    the [`crate::ledger`], so every host note ends exactly once (audition notes are not the host's and never
//!    reported). A note-off the host lists ahead of its own note-on on one frame (a zero-length note) is moved
//!    behind it ([`Audio`]'s `push_note`).
//! 4. Take the telemetry tap's wake flag and request a repaint (lock-free).
//! 5. Watch `EguiEditorState::is_open()` fall (the editor's `editor_closed` does the session side on the GUI thread;
//!    this covers a build without that hook): disable the tap and leave the editor-closed duties to the next pump.
//! 6. While [`crate::Shared::pump_wanted`] is set, at most once per [`PUMP_INTERVAL_S`] of audio: clear it and
//!    `execute_background(Task::Pump)` (one bounded `try_send`, only while a compile is pending; never
//!    `execute_gui`, which can run inline here).
//! 7. While [`crate::Shared::names_dirty`] is set (the names themselves were set off the audio thread), at most once
//!    per [`PUMP_INTERVAL_S`]: clear it and ask nice-plug for a parameter-info rescan. nice-plug runs the host call on
//!    the host's main thread (`request_callback`); a host that calls `process()` on its main thread gets it inline,
//!    so the call is wrapped in `permit_alloc` for the host's own work.
//! 8. When the running plan's voice count changed (a plan swap), report it (`set_current_voice_capacity`, CLAP voice
//!    info), the same way.
//!
//! With the `rt-stats` cargo feature, the block's duration goes into [`crate::rt_stats`]'s histogram; without it
//! no clock is read.

use std::sync::atomic::Ordering;

use enth_engine::{Engine, Event, EventKind, MAX_VOICES, NoteKey};
use enth_patch::catalogue::VoiceMode;
use enth_patch::model::{MACROS, SLOTS};
use enth_session::HostValues;
use enth_session::bridge::HOST_PARAMS;
use nice_plug::prelude::*;

use crate::ledger::Ledger;
use crate::params::EnthParams;
use crate::{Enth, Task, notes};

/// The shortest gap between two background pumps `process()` schedules, in seconds of audio.
pub const PUMP_INTERVAL_S: f64 = 0.05;

/// The event list's capacity: nice-plug's input capacity plus all 72 parameters. A block with more events is
/// rendered in chunks (each chunk ends at the frame of the event that did not fit), so none is dropped.
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
    /// The host notes handed to the engine and not reported ended yet ([`Ledger`]).
    ledger: Ledger,
    /// A CLAP `reset` emptied the engine: the next block reports every live note at frame 0.
    reset_pending: bool,
    /// Frames since the last parameter-info rescan request.
    since_rescan: u64,
    /// Rescan requests made (tests).
    rescans: u64,
    /// The voice count last reported to the host (CLAP voice info).
    voices: u32,
    /// Times the event list filled up inside a block, so the block was rendered in chunks (tests).
    chunks: u64,
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
            ledger: Ledger::new(),
            reset_pending: false,
            since_rescan: 1,
            rescans: 0,
            voices: 0,
            chunks: 0,
        }
    }

    /// At activate (main thread, before the engine plays): the host's current values become the "last sent" ones,
    /// so nothing is echoed and the engine starts from the patch; the pump interval follows the sample rate.
    ///
    /// `voices` is the voice count reported to the host at activation ([`voice_count`]; 0 while silent).
    pub fn activate(&mut self, params: &EnthParams, sample_rate: f32, voices: u32) {
        params.read_modulated(&mut self.last);
        self.now = self.last;
        self.pump_interval = ((PUMP_INTERVAL_S * f64::from(sample_rate)).round() as u64).max(1);
        self.since_pump = self.pump_interval;
        self.since_rescan = self.pump_interval;
        // A new engine has no voices: the old one's notes are forgotten with it (the host forgets its voices at
        // deactivation, and a `NOTE_END` can only be sent from `process()`).
        self.ledger.clear();
        self.reset_pending = false;
        self.voices = voices;
    }

    /// At a CLAP `reset` (audio thread), which frees the engine's voices and empties its queue without a report:
    /// the next block reports every live host note at frame 0 (hosts that track voices would otherwise wait for
    /// them forever).
    pub fn note_reset(&mut self) {
        self.reset_pending = true;
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

    /// Step 3's push: one host note event into the list. A note-off the host listed ahead of the note-on with the
    /// same note id on the same frame (a zero-length note) moves behind it, so the note starts and ends instead of
    /// starting after its own note-off and hanging forever: the order `enth render` gives a zero-length note. The
    /// caller has made room for one event; the move adds none.
    fn push_note(&mut self, e: Event) {
        if let EventKind::NoteOn { key, .. } = e.kind
            && key.note_id >= 0
        {
            let same_frame = self
                .events
                .iter()
                .rposition(|p| p.offset != e.offset)
                .map_or(0, |i| i + 1);
            let own_off = self.events[same_frame..].iter().position(|p| {
                matches!(p.kind, EventKind::NoteOff { key: off, .. }
                    if off.note_id == key.note_id && (off.key == -1 || off.key == key.key))
            });
            if let Some(i) = own_off {
                let off = self.events.remove(same_frame + i);
                self.events.push(e);
                self.events.push(off);
                return;
            }
        }
        self.events.push(e);
    }

    /// The events this block hands the engine (tests).
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Parameter-info rescans requested so far (tests).
    pub fn rescans(&self) -> u64 {
        self.rescans
    }

    /// The voice count last reported to the host (tests).
    pub fn voices(&self) -> u32 {
        self.voices
    }

    /// Times the event list filled up inside a block (tests).
    pub fn chunks(&self) -> u64 {
        self.chunks
    }
}

impl Enth {
    /// One audio block; see the [module docs](self).
    pub(crate) fn process_block(
        &mut self,
        buffer: &mut Buffer<'_>,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        #[cfg(feature = "rt-stats")]
        let started = std::time::Instant::now();
        let frames = buffer.samples();
        let Some(engine) = self.engine.as_mut() else {
            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }
            return ProcessStatus::KeepAlive;
        };
        // The only layout is stereo out (`AUDIO_IO_LAYOUTS`).
        let [left, right, ..] = buffer.as_slice() else {
            return ProcessStatus::KeepAlive;
        };
        let audio = &mut self.audio;
        audio.events.clear();

        // 1. Host values.
        self.params.read_modulated(&mut audio.now);
        let adopt = self.shared.rebaseline.swap(false, Ordering::AcqRel);
        audio.host_events(adopt);

        // 2. The notes a CLAP `reset` dropped end at frame 0, before anything this block reports.
        if audio.reset_pending {
            audio.reset_pending = false;
            audio.ledger.drain(|key| send_end(context, key, 0));
        }

        // 3. Notes, rendered in chunks when the list fills up, so no host event is ever dropped.
        engine.set_monitor_bypass(self.shared.monitor_bypass.load(Ordering::Acquire));
        let last = frames.saturating_sub(1);
        let mut start = 0;
        while let Some(event) = context.next_event() {
            let Some(e) = notes::engine_event(&event) else {
                continue;
            };
            let at = (e.offset as usize).min(last).max(start);
            if audio.events.len() >= EVENT_CAPACITY {
                render_chunk(
                    engine,
                    &mut audio.ledger,
                    (&mut left[start..at], &mut right[start..at]),
                    &audio.events,
                    start,
                    context,
                );
                audio.events.clear();
                audio.chunks = audio.chunks.saturating_add(1);
                start = at;
            }
            let offset = (at - start) as u32;
            if let EventKind::NoteOn { key, .. } = e.kind {
                audio
                    .ledger
                    .started(key, engine.position() + u64::from(offset));
            }
            audio.push_note(Event {
                offset,
                kind: e.kind,
            });
        }
        render_chunk(
            engine,
            &mut audio.ledger,
            (&mut left[start..frames], &mut right[start..frames]),
            &audio.events,
            start,
            context,
        );

        // 4. Telemetry wake.
        if let Some(tap) = engine.tap()
            && tap.take_wake()
        {
            self.repaint.request_repaint();
        }

        // 5. The editor closed.
        let open = self.editor_state.is_open();
        if audio.editor_open && !open {
            if let Some(tap) = engine.tap() {
                tap.set_enabled(false);
            }
            self.shared.editor_closed.store(true, Ordering::Release);
            self.shared.pump_wanted.store(true, Ordering::Release);
        }
        audio.editor_open = open;

        // 6. The background pump, rate-limited.
        let frames = frames as u64;
        audio.since_pump = audio.since_pump.saturating_add(frames);
        if audio.since_pump >= audio.pump_interval
            && self.shared.pump_wanted.swap(false, Ordering::AcqRel)
        {
            audio.since_pump = 0;
            context.execute_background(Task::Pump);
        }

        // 7. Host parameter names changed (set off this thread by `Shared::with_session`): ask for a rescan,
        //    rate-limited like the pump.
        audio.since_rescan = audio.since_rescan.saturating_add(frames);
        if audio.since_rescan >= audio.pump_interval
            && self.shared.names_dirty.swap(false, Ordering::AcqRel)
        {
            audio.since_rescan = 0;
            // nice-plug queues its own rescan task and calls the host's `request_callback`; only a host that calls
            // `process()` on its main thread gets the rescan inline, and the host's work in it may allocate.
            nice_plug::util::permit_alloc(|| context.request_param_info_rescan());
            audio.rescans = audio.rescans.saturating_add(1);
        }

        // 8. CLAP voice info: the plan's voice count, when a plan swap changed it.
        let voices = voice_count(engine);
        if voices != audio.voices {
            audio.voices = voices;
            // Like the rescan: nice-plug's own main-thread task, inline only on a host's main thread.
            nice_plug::util::permit_alloc(|| context.set_current_voice_capacity(voices));
        }

        #[cfg(feature = "rt-stats")]
        self.shared.rt_stats.record(started.elapsed(), frames);
        ProcessStatus::KeepAlive
    }
}

/// Render one chunk of a block (`out` starts at frame `start` of the host block; `events`' offsets count from
/// there) and report its `NOTE_END`s at their frames in the host block: the engine's reports for live host notes,
/// then the live notes the ledger's sweep finds ended without a report, at the chunk's last frame.
fn render_chunk(
    engine: &mut Engine,
    ledger: &mut Ledger,
    out: (&mut [f32], &mut [f32]),
    events: &[Event],
    start: usize,
    context: &mut impl ProcessContext<Enth>,
) {
    let (left, right) = out;
    let last = (start + left.len().saturating_sub(1)) as u32;
    engine.process(left, right, events);
    for end in engine.note_ends() {
        if ledger.ended(end.key) {
            send_end(context, end.key, start as u32 + end.offset);
        }
    }
    ledger.sweep(engine, |key| send_end(context, key, last));
}

/// One `NOTE_END` to the host (a full host queue only drops the report; nothing else to do here).
fn send_end(context: &mut impl ProcessContext<Enth>, key: NoteKey, timing: u32) {
    if let Some(end) = notes::note_end(key, timing) {
        let _ = context.try_send_event(end);
    }
}

/// The voice count CLAP voice info reports for the running plan: its polyphony in poly mode, one in mono and
/// legato.
pub fn voice_count(engine: &Engine) -> u32 {
    match engine.plan().mode() {
        VoiceMode::Poly => engine.plan().polyphony().clamp(1, MAX_VOICES) as u32,
        _ => 1,
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
