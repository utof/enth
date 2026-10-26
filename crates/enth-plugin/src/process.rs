//! The audio block (lanes SK, PL): host values and notes in, `Engine::process`, audio and `NOTE_END` out, the
//! rate-limited background pump and CLAP voice info. Nothing here allocates,
//! frees, locks, logs or does I/O (nice-plug's `assert_process_allocs` aborts debug builds that do).
//!
//! One block, in order:
//!
//! 1. Read the 72 parameters' **modulated** normalised values (atomics) and send each one that changed since the
//!    last block as `EventKind::Macro { index, value }` or `EventKind::Slot { slot, value }` at offset 0: never a
//!    bare `EventKind::Param`, because a plan swap inside the block would misroute an id looked up before it. With
//!    `SAMPLE_ACCURATE_AUTOMATION = true` nice-plug splits the host block at every parameter change, so offset 0 is
//!    the change's own frame and the engine's free-running grid makes the split invisible. After a state load
//!    ([`crate::Shared::rebaseline`]) the values the load restored ([`Restored`]) become "last sent" without being
//!    sent: the loaded patch wins; a value the host set after the load, even in the same block, is still sent. At
//!    activate the "last sent" values are the host's current ones, so nothing is echoed.
//!    **The host's tempo** (S2a, lane TMP) goes in first, from nice-plug's `Transport::tempo`: when the tempo differs
//!    from the one the engine last heard, one `EventKind::Tempo` at offset 0 (applied at the next segment boundary,
//!    like every event), so the plugin's synced modules follow the host's tempo exactly as `enth render --bpm` and a
//!    performance's `tempo_changes` do. The host gives one tempo per block, so a tempo ramp steps at block
//!    boundaries. A host that gives none (or a non-finite or non-positive one) leaves the engine's tempo as it is:
//!    140 BPM until the host gives one. No host parameter is involved; the 72 ids are untouched. **While a Recreate
//!    round plays Target ▶ or Mine ▶** (the A/B switch is not off) the tempo sent is [`DEFAULT_TEMPO_BPM`] instead,
//!    whatever the host says: the round's score, Target ▶ and Mine's loudness render offline at 140, and Mine ▶ plays
//!    the phrase live through this engine, so it must run at the tempo it is judged at. With the switch off, the
//!    host's tempo is sent again.
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
//!
//!    A Recreate round (lane SKR, [`crate::game::Player`]) adds two steps around the render: before it, the A/B
//!    atomics are read, a new target loop is taken from its lock-free hand-off and Mine ▶'s phrase notes for this
//!    block join the event list (on [`crate::game::GAME_PORT`]); after it, Target ▶'s loop and the engine's output
//!    are mixed at one shared position with a 10 ms crossfade and the loudness-match gains. With no round the
//!    player touches nothing.
//! 4. Take the telemetry tap's wake flag and request a repaint (lock-free).
//! 5. Watch `EguiEditorState::is_open()` fall (the editor's `editor_closed` does the session side on the GUI thread;
//!    this covers a build without that hook): disable the tap and leave the editor-closed duties to the next pump.
//! 6. While [`crate::Shared::pump_wanted`] is set, at most once per [`PUMP_INTERVAL_S`] of audio: clear it and
//!    `execute_background(Task::Pump)` (one bounded `try_send`, only while a compile is pending; never
//!    `execute_gui`, which can run inline here).
//! 7. When the running plan's voice count changed (a plan swap), report it (`set_current_voice_capacity`, CLAP voice
//!    info). nice-plug runs the host call on the host's main thread (`request_callback`); a host that calls
//!    `process()` on its main thread gets it inline, so the call is wrapped in `permit_alloc` for the host's own work.
//!
//! The parameter-info rescan after a rename is not requested here: `Shared::settle` asks for it on the thread that
//! renamed (the host's main thread for a state load, the GUI thread for an editor edit), through the fork's
//! `HostRequests` (design.md §4).
//!
//! With the `rt-stats` cargo feature, the block's duration goes into [`crate::rt_stats`]'s histogram; without it
//! no clock is read.

use std::sync::atomic::{AtomicU32, Ordering, fence};

use enth_engine::tempo::{self, DEFAULT_TEMPO_BPM};
use enth_engine::{Engine, Event, EventKind, MAX_VOICES, NoteKey};
use enth_patch::catalogue::VoiceMode;
use enth_patch::model::{MACROS, SLOTS};
use enth_session::HostValues;
use enth_session::bridge::HOST_PARAMS;
use enth_session::game::Listen;
use nice_plug::prelude::*;

use crate::ledger::Ledger;
use crate::params::EnthParams;
use crate::{Enth, Task, notes};

/// The shortest gap between two background pumps `process()` schedules, in seconds of audio.
pub const PUMP_INTERVAL_S: f64 = 0.05;

/// The event list's capacity: nice-plug's input capacity plus all 72 parameters and the tempo event. A block with
/// more events is rendered in chunks (each chunk ends at the frame of the event that did not fit), so none is
/// dropped.
pub const EVENT_CAPACITY: usize = <Enth as Plugin>::INPUT_EVENT_CAPACITY + HOST_PARAMS + 1;

/// The host values nice-plug restored with a state, as the `patch` field's load saw them (host main thread), for the
/// next block to take as its "last sent" values ([`crate::Shared::rebaseline`]). Only these are adopted silently: a
/// value the host sets after the load (REAPER sends an automation lane's value in the first block after it loads a
/// project) differs from them and reaches the engine. A small seqlock: the main thread writes (one writer, the
/// field's `set`), the audio thread reads; a read that overlaps a write is retried at the next block.
pub struct Restored {
    generation: AtomicU32,
    values: [AtomicU32; HOST_PARAMS],
}

impl Default for Restored {
    fn default() -> Self {
        Self::new()
    }
}

impl Restored {
    /// Defaults (never read before a first `store`: [`crate::Shared::rebaseline`] is set only after one).
    pub fn new() -> Self {
        Restored {
            generation: AtomicU32::new(0),
            values: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// The restored values (host main thread, before `rebaseline` is set with `Release`).
    pub fn store(&self, host: &HostValues) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        fence(Ordering::Release);
        for (slot, v) in self
            .values
            .iter()
            .zip(host.macros.iter().chain(&host.slots))
        {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Read them into `out` (audio thread): `false` if a store overlapped the read (`out` is then partly written).
    fn load_into(&self, out: &mut HostValues) -> bool {
        let before = self.generation.load(Ordering::Acquire);
        if before % 2 == 1 {
            return false;
        }
        for (v, slot) in out
            .macros
            .iter_mut()
            .chain(out.slots.iter_mut())
            .zip(&self.values)
        {
            *v = f32::from_bits(slot.load(Ordering::Relaxed));
        }
        fence(Ordering::Acquire);
        self.generation.load(Ordering::Relaxed) == before
    }
}

/// The audio thread's own state. Allocated once (at construction); `process()` only reuses it.
pub struct Audio {
    /// The host values the engine last heard (modulated).
    last: HostValues,
    /// This block's host values.
    now: HostValues,
    /// This block's engine events.
    events: Vec<Event>,
    /// The tempo the engine last heard, in BPM ([`DEFAULT_TEMPO_BPM`] for a new or reset engine).
    tempo: f64,
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
            tempo: DEFAULT_TEMPO_BPM,
            pump_interval: 1,
            since_pump: 1,
            editor_open: false,
            ledger: Ledger::new(),
            reset_pending: false,
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
        // A new engine has no voices: the old one's notes are forgotten with it (the host forgets its voices at
        // deactivation, and a `NOTE_END` can only be sent from `process()`).
        self.ledger.clear();
        self.reset_pending = false;
        self.voices = voices;
        self.tempo = DEFAULT_TEMPO_BPM;
    }

    /// At a CLAP `reset` (audio thread), which frees the engine's voices and empties its queue without a report:
    /// the next block reports every live host note at frame 0 (hosts that track voices would otherwise wait for
    /// them forever).
    pub fn note_reset(&mut self) {
        self.reset_pending = true;
        // `Engine::reset` returns the engine's tempo to the default: the next block tells it the host's again.
        self.tempo = DEFAULT_TEMPO_BPM;
    }

    /// Step 1's tempo: the host's tempo from this block's transport as one `Tempo` event at offset 0 when it differs
    /// from what the engine last heard (see [`wanted_tempo`]).
    fn tempo_event(&mut self, host: Option<f64>) {
        let want = wanted_tempo(host, self.tempo);
        if want.to_bits() != self.tempo.to_bits() {
            self.tempo = want;
            self.events.push(Event {
                offset: 0,
                kind: EventKind::Tempo { bpm: want },
            });
        }
    }

    /// Step 1: the host values that changed since the last block as engine events. After a load, `restored` (the
    /// values the load restored) replaces "last sent" first, so those are adopted silently and only a value the
    /// host changed after the load is sent.
    fn host_events(&mut self, restored: Option<&HostValues>) {
        if let Some(r) = restored {
            self.last = *r;
        }
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

        // 1. The host's tempo, then its values. While a Recreate round plays Target ▶ or Mine ▶ the tempo is the
        //    default one, not the host's: the round's score and Target ▶ render offline at `DEFAULT_TEMPO_BPM`, and
        //    Mine ▶ plays the phrase live through this engine, so a synced LFO must run at the tempo it is judged at
        //    (F1m judge, M2). Off, the host's tempo again.
        let ab = self.shared.game.ab.load();
        audio.tempo_event(if ab.listen == Listen::Off {
            context.transport().tempo
        } else {
            Some(DEFAULT_TEMPO_BPM)
        });
        self.params.read_modulated(&mut audio.now);
        if self.shared.rebaseline.swap(false, Ordering::AcqRel) {
            let mut restored = HostValues::default();
            if self.shared.restored.load_into(&mut restored) {
                audio.host_events(Some(&restored));
            } else {
                // A second load is writing its values: send nothing now, adopt them next block.
                self.shared.rebaseline.store(true, Ordering::Release);
            }
        } else {
            audio.host_events(None);
        }

        // 1b. A Recreate round (lane SKR): the A/B switch, a new target loop, and Mine ▶'s phrase notes for this block
        //     (on their own port, so the ledger never reports them). Nothing at all while no round plays.
        self.player.begin(ab, frames, &mut audio.events);

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

        // 3b. The A/B mix (lane SKR): Target ▶'s loop and Mine ▶'s engine output, crossfaded at one position; the
        //     block is left untouched while no round plays, so renders still equal `enth render`.
        self.player.mix(left, right);
        // 3c. A create candidate's audition (lane SK3, feature `create`): added once, from its start; one atomic load
        //     per block while none plays.
        #[cfg(feature = "create")]
        self.audition.mix(left, right);

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

        // 7. CLAP voice info: the plan's voice count, when a plan swap changed it. nice-plug queues its own main-thread
        //    task and calls the host's `request_callback`; only a host that calls `process()` on its main thread gets
        //    the host call inline, and the host's work in it may allocate.
        let voices = voice_count(engine);
        if voices != audio.voices {
            audio.voices = voices;
            nice_plug::util::permit_alloc(|| context.set_current_voice_capacity(voices));
        }

        #[cfg(feature = "rt-stats")]
        self.shared.rt_stats.record(started.elapsed(), frames);
        ProcessStatus::KeepAlive
    }
}

/// The tempo the engine should have for a block whose transport reports `host` (nice-plug's `Transport::tempo`),
/// given the tempo `held` it last heard: a usable host tempo (finite and positive) clamped into the engine's range
/// ([`tempo::MIN_TEMPO_BPM`] to [`tempo::MAX_TEMPO_BPM`]); a missing or unusable one keeps `held`.
pub fn wanted_tempo(host: Option<f64>, held: f64) -> f64 {
    match host {
        Some(bpm) if bpm.is_finite() && bpm > 0.0 => tempo::sanitize(bpm).unwrap_or(held),
        _ => held,
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
        audio.host_events(None);
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
        audio.host_events(None);
        assert_eq!(audio.events(), [], "unchanged values send nothing");
        // A load restored slot 1 = 0.1 and macro 1 = 0.6; the host then set macro 1 to 0.3 before this block.
        let mut restored = audio.now;
        restored.slots[0] = 0.1;
        restored.macros[0] = 0.6;
        audio.now = restored;
        audio.now.macros[0] = 0.3;
        audio.host_events(Some(&restored));
        assert_eq!(
            audio.events(),
            [Event {
                offset: 0,
                kind: EventKind::Macro {
                    index: 0,
                    value: 0.3
                }
            }],
            "the restored slot 1 is adopted, the later macro move is sent"
        );
        assert_eq!(audio.last.slots[0], 0.1);
    }

    #[test]
    fn the_restored_values_cross_threads_whole() {
        let r = Restored::new();
        let mut host = HostValues::default();
        host.slots[63] = 0.25;
        host.macros[7] = 0.75;
        r.store(&host);
        let mut out = HostValues::default();
        assert!(r.load_into(&mut out));
        assert_eq!(out, host);
        // A store in progress (odd generation) is not read.
        r.generation.fetch_add(1, Ordering::AcqRel);
        assert!(!r.load_into(&mut out));
    }

    #[test]
    fn the_event_list_fits_every_host_event_and_every_parameter() {
        let audio = Audio::new();
        assert!(audio.events.capacity() >= EVENT_CAPACITY);
        assert_eq!(
            EVENT_CAPACITY,
            1024 + 72 + 1,
            "input events, 72 parameters, the tempo"
        );
    }

    #[test]
    fn the_host_tempo_maps_to_the_engines_with_140_until_the_host_gives_one() {
        // A usable host tempo is taken (clamped into 20..=999); nothing usable keeps what the engine has.
        assert_eq!(wanted_tempo(Some(128.0), 140.0), 128.0);
        assert_eq!(
            wanted_tempo(Some(19.0), 140.0).to_bits(),
            tempo::MIN_TEMPO_BPM.to_bits()
        );
        assert_eq!(
            wanted_tempo(Some(5000.0), 140.0).to_bits(),
            tempo::MAX_TEMPO_BPM.to_bits()
        );
        assert_eq!(wanted_tempo(None, 140.0), 140.0);
        assert_eq!(
            wanted_tempo(None, 90.0),
            90.0,
            "a host that stops saying keeps the last tempo"
        );
        for junk in [0.0, -120.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(wanted_tempo(Some(junk), 90.0), 90.0, "{junk}");
        }
    }

    #[test]
    fn a_tempo_event_goes_out_only_when_the_host_tempo_changed() {
        let mut audio = Audio::new();
        let tempo = |bpm: f64| Event {
            offset: 0,
            kind: EventKind::Tempo { bpm },
        };
        // A new engine runs at 140: no host tempo and a host at 140 send nothing.
        audio.tempo_event(None);
        audio.tempo_event(Some(140.0));
        assert_eq!(audio.events(), []);
        audio.tempo_event(Some(174.0));
        assert_eq!(audio.events(), [tempo(174.0)]);
        audio.tempo_event(Some(174.0));
        audio.tempo_event(None);
        audio.tempo_event(Some(f64::NAN));
        assert_eq!(
            audio.events().len(),
            1,
            "unchanged or unusable sends nothing"
        );
        // A CLAP reset puts the engine back at 140, so the host's tempo is sent again (a new activation does the same).
        audio.events.clear();
        audio.note_reset();
        audio.tempo_event(Some(174.0));
        assert_eq!(audio.events(), [tempo(174.0)]);
    }
}
