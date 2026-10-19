//! The engine: the segment loop behind the host adapter.
//!
//! [`Engine::process`] takes any number of frames and the host block's events, and fills the output from a
//! one-segment FIFO ([`crate::segment`]), rendering a new segment whenever it runs dry. Every segment runs the same
//! fixed steps in the same order, which is where the determinism lives (design.md §3):
//!
//! 1. poll the command ring ([`crate::swap`]): plan swaps (state carried, a 256-frame crossfade when modules were
//!    added or removed; remembered host slot values re-applied), epoch-tagged value and depth events (stale ones
//!    dropped), macros, events sent from the main thread (audition notes);
//! 2. apply the events due at this boundary, in arrival order (notes through the voice manager, parameter values,
//!    slot values, macros and their per-voice offsets, the mod wheel, chokes), then advance glides one segment;
//! 3. advance the smoothers (normalised domain; settled parameters deliver the exact plain value) and the route
//!    depth ramps;
//! 4. global modulators, in patch order, each after its own modulated parameters;
//! 5. voice modulators, in patch order, each after its own modulated parameters, over the active voices;
//! 6. the remaining modulated parameters: `n = clamp(smoothed_norm + Σ depth·src, 0, 1)` in patch route order;
//! 7. voices: generators sum into each voice's bus in patch order (the first enabled `osc` writes the hard-sync
//!    timing that later generators read), then `voice_fx` in order, then the amp stage (ramped per sample from the
//!    amp envelope's previous to current value, times a steal fade) with the constant-power pan, into the stereo sum
//!    in ascending voice order;
//! 8. `fx` in order (a processor whose state is silent and whose input is zero is skipped; the monitor bypass fades
//!    `time_based` ones out over 20 ms and then skips them), then `master.level`;
//! 9. end finished voices (past the attack and below −100 dB, or a steal fade over); modules flush their own state
//!    below 1e-15;
//! 10. write the FIFO.
//!
//! During a crossfade, steps 3–8 run for both plans (the replaced one first) over the same notes, and the output is
//! `old + (new − old)·w` with a raised-cosine `w` over 256 frames: an equal-gain fade, exact where both plans agree.
//! At the end of `process` an enabled telemetry tap ([`crate::tap`]) gets the block's values; it only reads.
//!
//! `process` never allocates, frees, locks, logs or does I/O, and it does not panic on any input: mismatched
//! channel lengths, out-of-range event offsets, unknown parameter ids, NaN values, a full event queue, full rings
//! and a full return ring are all handled (clamped, ignored, kept or dropped and counted).

use core::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use enth_dsp::util::math::{constant_power_pan, cos, db_to_gain};
use enth_patch::catalogue::{GlideMode, MasterParam, VoiceParam};
use enth_patch::model::SLOTS;

use crate::event::{Event, EventKind, NoteKey};
use crate::modulation::{MACRO_SOURCES, MOD_WHEEL, POLY_MACROS, PolyOffsets, Sources};
use crate::module::{
    Block, GlobalCx, GlobalModCx, MAX_VOICES, NO_SYNC, NoteState, RAMP, SEG, SyncBus, VoiceBus,
    VoiceCx, VoiceIdx, VoiceModCx,
};
use crate::params::ParamId;
use crate::plan::Plan;
use crate::segment::{EVENT_CAPACITY, EventQueue, Fifo};
use crate::swap::{AudioLink, Cmd, Link, LinkCapacity, rings};
use crate::tap::{Tap, store_f32};
use crate::voice::{Glide, NoteEnd, NoteOn, VoiceManager, VoiceState, glide_segments};

/// Frames a plan-swap crossfade lasts (only when modules were added or removed).
pub const CROSSFADE_FRAMES: usize = 256;

const CROSSFADE_SEGMENTS: usize = CROSSFADE_FRAMES / SEG;

/// How long the monitor bypass fades a `time_based` effect out (on engage) or in (on release), ADR-0006:
/// `max(1, round(20 · sr / 1000 / 32))` segments, 30 at 48 kHz.
pub const MONITOR_BYPASS_FADE_MS: f32 = 20.0;

/// Scratch buses, boxed so the engine stays small to move.
struct Buses {
    voice: VoiceBus,
    scratch: VoiceBus,
    sync: SyncBus,
    left: Block,
    right: Block,
    /// The replaced plan's output during a crossfade.
    old_left: Block,
    old_right: Block,
    /// A time-based effect's input while the monitor bypass fades it.
    dry_left: Block,
    dry_right: Block,
    /// The crossfade weight per frame: `0.5 − 0.5·cos(π·(i + 1)/256)`, rising to exactly 1.
    xfade: [f32; CROSSFADE_FRAMES],
    /// Per-voice macro offsets (CLAP polyphonic modulation), `[macro][voice]`.
    poly: PolyOffsets,
}

/// Counters for things the engine handled without failing. `non_exhaustive` (F1b review): counters are added
/// without breaking readers; build one with `Default` outside the crate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineStats {
    /// Note-ons since the engine started (the next note index).
    pub notes: u64,
    /// Note-ons dropped. Always 0 since lane S2b: a full engine steals instead.
    pub dropped_notes: u64,
    /// Events dropped because the event queue was full.
    pub dropped_events: u64,
    /// Segments rendered.
    pub segments: u64,
    /// Voices faded out over 64 frames to make room for a note (S2b).
    pub stolen_voices: u64,
    /// Voices cut without a fade because all 32 slots were busy (a burst of more than 16 steals in 64 frames).
    pub cut_voices: u64,
    /// Value and depth events dropped because they named another plan's epoch (from the command ring, or an
    /// `EventKind::EpochParam`).
    pub stale_events: u64,
    /// Plans swapped in.
    pub swaps: u64,
    /// Swaps that crossfaded (a module was added or removed).
    pub crossfades: u64,
    /// Voice ends that did not fit one `process` call's report.
    pub dropped_note_ends: u64,
}

/// How the monitor bypass treats the `time_based` effects in one segment.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bypass {
    /// Not bypassed: they run as usual.
    Off,
    /// Fully bypassed: they are the identity and are not called.
    On,
    /// Fading: the output is `wet·g + dry·(1 − g)`, `g` ramping per sample from `g0` to `g1` (1 = not bypassed).
    Fade { g0: f32, g1: f32 },
}

/// The real-time engine. Build it with [`Engine::new`] (offline: no command ring) or [`Engine::with_link`] (real
/// time: plus the main thread's [`Link`]) from a compiled [`Plan`] (allocates); then call [`Engine::process`] from
/// the audio thread (never allocates or frees).
pub struct Engine {
    render_seed: u64,
    plan: Box<Plan>,
    /// The replaced plan while a crossfade runs, and the crossfade's segment (`CROSSFADE_SEGMENTS`: over, waiting
    /// for room on the return ring).
    old: Option<Box<Plan>>,
    xfade_seg: usize,
    link: Option<AudioLink>,
    voices: VoiceManager,
    notes: NoteState,
    glide: Glide,
    buses: Box<Buses>,
    fifo: Fifo,
    queue: EventQueue,
    /// Absolute frame of the next host frame.
    position: u64,
    /// Absolute frame of the next segment's start (a multiple of `SEG`).
    next_segment: u64,
    segments: u64,
    /// The monitor bypass as asked.
    monitor_bypass: bool,
    /// Where the bypass fade is: `bypass_segments` = effects fully on, 0 = fully bypassed.
    bypass_level: u32,
    bypass_segments: u32,
    /// The last host value of each automation slot (NaN: none), re-applied after a plan swap.
    host_slots: [f32; SLOTS],
    /// Some per-voice macro offset is non-zero, and whether that needs recomputing.
    poly_active: bool,
    poly_dirty: bool,
    tap: Option<Arc<Tap>>,
    /// Consecutive silent samples written to the tap's scope since the last sound.
    tap_silent: usize,
    stale_events: u64,
    swaps: u64,
    crossfades: u64,
}

impl Engine {
    /// An engine running `plan`, with every random stream derived from `render_seed`, and no command ring (the
    /// offline renderer's engine: it never swaps). Global modules start here. Allocates (the buses, the event
    /// queue); `process` never does.
    #[must_use]
    pub fn new(plan: Plan, render_seed: u64) -> Self {
        let mut plan = Box::new(plan);
        plan.reset_globals(render_seed);
        let voices = VoiceManager::with_mode(plan.mode, plan.polyphony);
        let xfade = core::array::from_fn(|i| {
            let t = (i + 1) as f64 / CROSSFADE_FRAMES as f64;
            (0.5 - 0.5 * cos(core::f64::consts::PI * t)) as f32
        });
        let fade_frames =
            f64::from(MONITOR_BYPASS_FADE_MS) * f64::from(plan.cfg.sample_rate) / 1000.0;
        let bypass_segments = (fade_frames / SEG as f64).round().clamp(1.0, 1e6) as u32;
        Self {
            render_seed,
            plan,
            old: None,
            xfade_seg: 0,
            link: None,
            voices,
            notes: NoteState::new(),
            glide: Glide::new(),
            buses: Box::new(Buses {
                voice: [[0.0; SEG]; MAX_VOICES],
                scratch: [[0.0; SEG]; MAX_VOICES],
                sync: [[NO_SYNC; SEG]; MAX_VOICES],
                left: [0.0; SEG],
                right: [0.0; SEG],
                old_left: [0.0; SEG],
                old_right: [0.0; SEG],
                dry_left: [0.0; SEG],
                dry_right: [0.0; SEG],
                xfade,
                poly: [[0.0; MAX_VOICES]; POLY_MACROS],
            }),
            fifo: Fifo::new(),
            queue: EventQueue::new(EVENT_CAPACITY),
            position: 0,
            next_segment: 0,
            segments: 0,
            monitor_bypass: false,
            bypass_level: bypass_segments,
            bypass_segments,
            host_slots: [f32::NAN; SLOTS],
            poly_active: false,
            poly_dirty: false,
            tap: None,
            tap_silent: 0,
            stale_events: 0,
            swaps: 0,
            crossfades: 0,
        }
    }

    /// An engine as [`Engine::new`], plus its command and return rings: the [`Link`] is the main thread's end
    /// (send plans and value events, collect replaced plans). Allocates the rings.
    #[must_use]
    pub fn with_link(plan: Plan, render_seed: u64, capacity: LinkCapacity) -> (Self, Link) {
        let epoch = plan.epoch;
        let mut engine = Self::new(plan, render_seed);
        let (audio, link) = rings(capacity, epoch);
        engine.link = Some(audio);
        (engine, link)
    }

    /// Attach a telemetry tap ([`crate::tap`]), replacing any previous one. **Main thread**, before the engine goes
    /// to the audio thread: replacing a tap drops a reference, which must not be the audio thread's job. The engine
    /// publishes into it at the end of every `process` call while it is enabled; it never changes the output.
    pub fn attach_tap(&mut self, tap: Arc<Tap>) {
        self.tap = Some(tap);
        self.tap_silent = 0;
    }

    /// The attached telemetry tap.
    #[must_use]
    pub fn tap(&self) -> Option<&Arc<Tap>> {
        self.tap.as_ref()
    }

    /// Render `left.len()` frames (the shorter channel's length; the rest of the longer one is zeroed) with this
    /// block's `events`, whose offsets count from `left[0]` (an offset past the block is taken as its last frame).
    /// Events take effect at the first segment boundary at or after their frame; the block size never matters.
    #[cfg_attr(enth_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], events: &[Event]) {
        self.voices.clear_ends();
        let n = left.len().min(right.len());
        let last = n.saturating_sub(1) as u64;
        for e in events {
            let frame = self.position + u64::from(e.offset).min(last);
            self.queue.push(frame, e.kind);
        }
        let mut i = 0;
        while i < n {
            if self.fifo.is_empty() {
                self.render_segment();
            }
            i += self.fifo.read_into(&mut left[i..n], &mut right[i..n]);
        }
        left[n..].fill(0.0);
        right[n..].fill(0.0);
        self.voices.place_ends(self.position, n);
        self.position += n as u64;
        if self.tap.as_deref().is_some_and(Tap::is_enabled) {
            self.publish(&left[..n], &right[..n]);
        }
    }

    /// Back to the state of a freshly built engine with the same plan, keeping the base values (CLAP `reset`, which
    /// the host calls on the audio thread; P1b lane E+). Every voice is freed (no `NOTE_END` is reported), every
    /// module's state is cleared (global modules reset, parameters settled on their bases, depth ramps landed), the
    /// FIFO and the event queue are emptied, and the position, the segment grid and the note index restart at 0, so
    /// what follows renders exactly like [`Engine::new`] with a plan compiled from a patch holding those bases (the
    /// determinism contract counts events from engine creation or reset). Kept: the plan, its epoch, the base values
    /// (host and editor values), the remembered slot values, the commands queued in the ring, the monitor bypass
    /// (snapped, no fade), the tap, and the link counters (stale events, swaps, crossfades) and dropped events. A
    /// crossfade in progress ends: the replaced plan goes back on the return ring, or waits in the pending slot, never
    /// freed here. Never allocates.
    pub fn reset(&mut self) {
        if self.old.is_some() {
            self.xfade_seg = CROSSFADE_SEGMENTS;
            self.retire_old();
        }
        self.plan.reset_state(self.render_seed);
        self.voices = VoiceManager::with_mode(self.plan.mode, self.plan.polyphony);
        self.notes = NoteState::new();
        self.glide = Glide::new();
        self.fifo = Fifo::new();
        self.queue.clear();
        self.position = 0;
        self.next_segment = 0;
        self.segments = 0;
        self.buses.poly = [[0.0; MAX_VOICES]; POLY_MACROS];
        self.poly_active = false;
        self.poly_dirty = false;
        self.bypass_level = if self.monitor_bypass {
            0
        } else {
            self.bypass_segments
        };
        self.tap_silent = 0;
    }

    /// The monitor bypass (the plugin's global FX bypass, a monitoring aid): the effects whose kind is flagged
    /// `time_based` in the catalogue (the reverb in P1) fade out over 20 ms ([`MONITOR_BYPASS_FADE_MS`]) to the
    /// identity, are reset and stop processing (they cost nothing); releasing it fades them back in from silence
    /// over 20 ms (ADR-0006). Reversing mid-fade turns the fade around. Before the first segment (and right after
    /// [`Engine::reset`]) the switch is immediate: there is nothing to fade. It is engine state, not patch state,
    /// and `enth-render` never sets it, so renders and scores cannot see it (design.md §5). Takes effect at the next
    /// segment boundary. Cheap and allocation-free: call it from the audio thread before `process`.
    pub fn set_monitor_bypass(&mut self, on: bool) {
        self.monitor_bypass = on;
        if self.segments == 0 {
            self.bypass_level = if on { 0 } else { self.bypass_segments };
        }
    }

    /// Whether the monitor bypass is on (as asked; the fade may still run).
    #[must_use]
    pub fn monitor_bypass(&self) -> bool {
        self.monitor_bypass
    }

    /// Whether the monitor bypass fade is running.
    #[must_use]
    pub fn is_bypass_fading(&self) -> bool {
        self.bypass_level != 0 && self.bypass_level != self.bypass_segments
    }

    /// The playing voices (held, releasing or fading), ascending.
    #[must_use]
    pub fn active_voices(&self) -> &[VoiceIdx] {
        self.voices.active()
    }

    /// The voices that ended during the last `process` call, with the frame they went silent and that frame as an
    /// offset clamped into the call's block ([`NoteEnd::offset`], the timing for CLAP `NOTE_END`).
    #[must_use]
    pub fn note_ends(&self) -> &[NoteEnd] {
        self.voices.ends()
    }

    /// The absolute frame of the next host frame: frames processed since [`Engine::new`] or [`Engine::reset`]. A
    /// block passed to `process` starts here; its events' offsets count from it.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.position
    }

    /// The voice manager (slot states and keys).
    #[must_use]
    pub fn voices(&self) -> &VoiceManager {
        &self.voices
    }

    /// The notes as modules see them (pitch, velocity, gate per slot).
    #[must_use]
    pub fn notes(&self) -> &NoteState {
        &self.notes
    }

    /// The running plan's base value of parameter `id` (the patch value or the latest value event), before smoothing
    /// and modulation; `None` for an unknown id.
    #[must_use]
    pub fn base(&self, id: ParamId) -> Option<f32> {
        match id {
            ParamId::Voice(i) => self.plan.voice_table.base.get(usize::from(i)).copied(),
            ParamId::Global(i) => self.plan.global_table.base.get(usize::from(i)).copied(),
        }
    }

    /// The running plan's value of parameter `id` in the last segment, as modules saw it (smoothed and modulated),
    /// for voice slot `voice` (ignored for a global parameter); `None` for an unknown id or slot. A slot that is not
    /// playing holds a stale value.
    #[must_use]
    pub fn value(&self, id: ParamId, voice: VoiceIdx) -> Option<f32> {
        match id {
            ParamId::Voice(i) => self
                .plan
                .voice_table
                .cur
                .get(usize::from(i))
                .and_then(|row| row.get(voice))
                .copied(),
            ParamId::Global(i) => self.plan.global_table.cur.get(usize::from(i)).copied(),
        }
    }

    /// The per-voice offset of macro `index` (0–7) on voice slot `voice` (CLAP polyphonic modulation).
    #[must_use]
    pub fn macro_offset(&self, index: usize, voice: VoiceIdx) -> Option<f32> {
        self.buses
            .poly
            .get(index)
            .and_then(|row| row.get(voice))
            .copied()
    }

    /// The last host value of automation slot `slot` (0-based), if the engine remembers one.
    #[must_use]
    pub fn host_slot(&self, slot: usize) -> Option<f32> {
        self.host_slots.get(slot).copied().filter(|v| !v.is_nan())
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            notes: self.voices.note_count(),
            dropped_notes: self.voices.dropped_notes(),
            dropped_events: self.queue.dropped(),
            segments: self.segments,
            stolen_voices: self.voices.stolen(),
            cut_voices: self.voices.cut(),
            stale_events: self.stale_events,
            swaps: self.swaps,
            crossfades: self.crossfades,
            dropped_note_ends: self.voices.dropped_ends(),
        }
    }

    /// The render seed every random stream derives from.
    #[must_use]
    pub fn render_seed(&self) -> u64 {
        self.render_seed
    }

    /// The plan being run.
    #[must_use]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The epoch of the plan being run. Tag `EventKind::EpochParam` with the epoch of the plan whose
    /// [`PlanMap`](crate::PlanMap) gave the id (in a real-time engine the plan can change inside `process`).
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.plan.epoch
    }

    /// Whether a crossfade between two plans is running.
    #[must_use]
    pub fn is_crossfading(&self) -> bool {
        self.old.is_some() && self.xfade_seg < CROSSFADE_SEGMENTS
    }

    /// The plans the engine holds: the running one, plus a replaced one that is crossfading or waiting for room on
    /// the return ring (never dropped on the audio thread).
    #[must_use]
    pub fn plans_held(&self) -> usize {
        1 + usize::from(self.old.is_some())
            + usize::from(self.link.as_ref().is_some_and(AudioLink::has_pending))
    }

    /// The number of subnormal values in the engine's state: every module of every plan held, the modulator outputs
    /// and the output ramps (test support for the release-tail invariant; walks the state, does not allocate).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.plan.subnormals() + self.old.as_ref().map_or(0, |p| p.subnormals())
    }

    /// One segment: steps 1–10 (see the module docs).
    fn render_segment(&mut self) {
        let frame = self.next_segment;
        // 1. Commands.
        self.poll_commands(frame);

        // 2. Events due at this boundary, in arrival order; then the glides.
        self.begin_notes();
        {
            let (queue, mut cx) = self.note_cx(frame);
            queue.drain_due(frame, |kind| cx.apply(kind));
        }
        let mut active_buf = [0; MAX_VOICES];
        let n_active = {
            let a = self.voices.active();
            active_buf[..a.len()].copy_from_slice(a);
            a.len()
        };
        let active = &active_buf[..n_active];
        self.glide.advance(active, &mut self.notes);
        if self.poly_dirty {
            self.poly_dirty = false;
            self.poly_active = self
                .buses
                .poly
                .iter()
                .any(|row| active.iter().any(|&v| row[v] != 0.0));
        }

        // The monitor bypass fade moves one segment towards its target.
        let (b0, b1) = self.advance_bypass();
        let bypass = if b0 == self.bypass_segments && b1 == self.bypass_segments {
            Bypass::Off
        } else if b0 == 0 && b1 == 0 {
            Bypass::On
        } else {
            let n = self.bypass_segments as f32;
            Bypass::Fade {
                g0: b0 as f32 / n,
                g1: b1 as f32 / n,
            }
        };

        // 3–8, per plan: the replaced one first while it crossfades.
        let buses = &mut *self.buses;
        let cx = SegmentCx {
            active,
            notes: &self.notes,
            voices: &self.voices,
            bypass,
            poly: self.poly_active.then_some(&buses.poly),
        };
        let mut out = Out {
            voice: &mut buses.voice,
            scratch: &mut buses.scratch,
            sync: &mut buses.sync,
            left: &mut buses.left,
            right: &mut buses.right,
            dry_left: &mut buses.dry_left,
            dry_right: &mut buses.dry_right,
        };
        let crossfading = self.old.is_some() && self.xfade_seg < CROSSFADE_SEGMENTS;
        if crossfading && let Some(old) = self.old.as_deref_mut() {
            render_plan(old, &cx, &mut out);
            buses.old_left = *out.left;
            buses.old_right = *out.right;
        }
        render_plan(&mut self.plan, &cx, &mut out);
        if crossfading {
            let w = &buses.xfade[self.xfade_seg * SEG..(self.xfade_seg + 1) * SEG];
            for (((l, r), (ol, or)), w) in out
                .left
                .iter_mut()
                .zip(out.right.iter_mut())
                .zip(buses.old_left.iter().zip(&buses.old_right))
                .zip(w)
            {
                *l = ol + (*l - ol) * w;
                *r = or + (*r - or) * w;
            }
            self.xfade_seg += 1;
        }
        debug_assert!(
            out.left
                .iter()
                .chain(out.right.iter())
                .all(|x| x.is_finite()),
            "non-finite output sample"
        );
        if b0 > 0 && b1 == 0 {
            // The bypass fade-out is over: the time-based effects stop here, and restart from silence when released.
            self.plan.reset_time_based(self.render_seed);
            if let Some(old) = self.old.as_deref_mut() {
                old.reset_time_based(self.render_seed);
            }
        }

        // 9. End finished voices: the amp envelope of every plan that runs is done, or the steal fade is over.
        {
            let plan = &self.plan;
            let old = self.old.as_deref().filter(|_| crossfading);
            let any = self.voices.end_segment(frame, |v| {
                plan.voice_mods[plan.amp_env].module.voice_finished(v)
                    && old.is_none_or(|o| o.voice_mods[o.amp_env].module.voice_finished(v))
            });
            if any {
                for &v in active {
                    if self.voices.state(v) == VoiceState::Free {
                        self.notes.gate[v] = false;
                        for row in &mut self.buses.poly {
                            row[v] = 0.0;
                        }
                        self.poly_dirty = true;
                    }
                }
            }
        }
        if self.old.is_some() && self.xfade_seg >= CROSSFADE_SEGMENTS {
            self.retire_old();
        }

        // 10. The FIFO.
        self.fifo.fill(&self.buses.left, &self.buses.right);
        self.next_segment += SEG as u64;
        self.segments += 1;
    }

    /// The bypass level at the start and at the end of this segment, after moving one step towards the target.
    fn advance_bypass(&mut self) -> (u32, u32) {
        let target = if self.monitor_bypass {
            0
        } else {
            self.bypass_segments
        };
        let l0 = self.bypass_level;
        let l1 = match l0.cmp(&target) {
            core::cmp::Ordering::Less => l0 + 1,
            core::cmp::Ordering::Greater => l0 - 1,
            core::cmp::Ordering::Equal => l0,
        };
        self.bypass_level = l1;
        (l0, l1)
    }

    /// What step 2 (and an event from the command ring) needs, borrowed from the engine; plus the event queue.
    fn note_cx(&mut self, frame: u64) -> (&mut EventQueue, NoteCx<'_>) {
        let Self {
            queue,
            plan,
            old,
            voices,
            notes,
            glide,
            render_seed,
            buses,
            host_slots,
            poly_dirty,
            stale_events,
            ..
        } = self;
        (
            queue,
            NoteCx {
                plan,
                old: old.as_deref_mut(),
                voices,
                notes,
                glide,
                render_seed: *render_seed,
                frame,
                host_slots,
                poly: &mut buses.poly,
                poly_dirty,
                stale_events,
            },
        )
    }

    /// Step 1: read commands until the ring is empty, a plan has to wait (a crossfade runs) or a replaced plan waits
    /// for room on the return ring (back-pressure: the commands behind it stay queued, in order).
    fn poll_commands(&mut self, frame: u64) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        if link.flush_pending() {
            loop {
                if link.next_is_plan() && self.old.is_some() {
                    break;
                }
                // Back-pressure for events from the ring (audition notes; F1b review): one that finds the event
                // queue full waits in the ring, with the commands behind it, until a segment has applied the due
                // events. Host events are due within the block, so the queue drains; an audition note-off is never
                // dropped (a dropped one would be a stuck note).
                if link.next_is_event() && self.queue.len() >= self.queue.capacity() {
                    break;
                }
                let Some(cmd) = link.pop() else {
                    break;
                };
                match cmd {
                    Cmd::Plan(new) => {
                        self.swap(new, &mut link);
                        if link.has_pending() {
                            break;
                        }
                    }
                    Cmd::Param { epoch, id, value } => {
                        if epoch == self.plan.epoch {
                            set_param(&mut self.plan, id, value);
                            self.plan.note_patch_value(id, value);
                        } else {
                            self.stale_events += 1;
                        }
                    }
                    Cmd::Macro { index, value } => {
                        set_macro(&mut self.plan, usize::from(index), value);
                        if let Some(old) = self.old.as_deref_mut() {
                            set_macro(old, usize::from(index), value);
                        }
                    }
                    Cmd::Depth { epoch, id, depth } => {
                        if epoch == self.plan.epoch {
                            self.plan.matrix.set_depth(usize::from(id.0), depth);
                        } else {
                            self.stale_events += 1;
                        }
                    }
                    // Queued behind the host events already due here, so step 2 applies it in order. The check
                    // above leaves it in the ring while the queue is full, so it is never dropped.
                    Cmd::Event(kind) => {
                        self.queue.push(frame, kind);
                    }
                }
            }
        }
        self.link = Some(link);
    }

    /// Swap `new` in at this boundary: carry the running state, re-apply the remembered host slot values, adopt its
    /// voice mode, and either crossfade (a module was added or removed) or hand the old plan back at once.
    fn swap(&mut self, mut new: Box<Plan>, link: &mut AudioLink) {
        let changed = new.carry_from(
            &mut self.plan,
            self.voices.active(),
            &self.notes,
            self.render_seed,
        );
        // Host automation survives an edit: a slot that still names the same parameter gets its host value again
        // (the exact patch value if the host's value is `to_norm` of it). A slot that now names another parameter,
        // or none, forgets it: the host sends a new value when it moves. So does a slot the plan names in
        // `Plan::forget_host_slots` (a loaded patch, an edited value): the plan's patch value wins.
        let forget = new.forget_slots;
        for (i, h) in self.host_slots.iter_mut().enumerate() {
            if h.is_nan() {
                continue;
            }
            let key = new.slots[i].key;
            if forget & (1 << i) == 0 && key != 0 && key == self.plan.slots[i].key {
                apply_slot(&mut new, i, *h);
            } else {
                *h = f32::NAN;
            }
        }
        self.voices.configure(new.mode, new.polyphony);
        let old = core::mem::replace(&mut self.plan, new);
        self.swaps += 1;
        if changed {
            self.old = Some(old);
            self.xfade_seg = 0;
            self.crossfades += 1;
        } else if let Err(old) = link.give_back(old) {
            // Unreachable (no swap while a plan waits); keep it rather than free it here.
            self.old = Some(old);
            self.xfade_seg = CROSSFADE_SEGMENTS;
        }
    }

    /// The crossfade is over: hand the replaced plan back (or keep it until the pending slot frees).
    fn retire_old(&mut self) {
        let Some(link) = self.link.as_mut() else {
            return; // no ring, so no swap and no old plan; never drop one here
        };
        if let Some(old) = self.old.take()
            && let Err(old) = link.give_back(old)
        {
            self.old = Some(old);
        }
    }

    /// Clear the per-segment note flags and carry the pitch over.
    fn begin_notes(&mut self) {
        let n = &mut self.notes;
        n.trigger = [false; MAX_VOICES];
        n.fresh = [false; MAX_VOICES];
        n.release = [false; MAX_VOICES];
        n.pitch_prev = n.pitch_cur;
    }

    /// End of `process`: publish this block into the enabled tap (reads engine state only).
    fn publish(&mut self, left: &[f32], right: &[f32]) {
        let stats = self.stats();
        let Self {
            tap,
            tap_silent,
            plan,
            voices,
            notes,
            ..
        } = self;
        let Some(tap) = tap.as_deref() else { return };
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let (pl, pr) = (peak(left), peak(right));
        let active = voices.active();
        if pl == 0.0 && pr == 0.0 && active.is_empty() {
            if *tap_silent >= tap.scope_len() {
                return; // silence after silence: nothing new, the editor may sleep
            }
            *tap_silent += left.len();
        } else {
            *tap_silent = 0;
        }
        tap.write_scope(left, right);

        // The display voice: the newest held voice, else the newest releasing one, else the newest fading one.
        let newest = |state: VoiceState| {
            active
                .iter()
                .copied()
                .filter(|&v| voices.state(v) == state)
                .max_by_key(|&v| notes.note_index[v])
        };
        let display = newest(VoiceState::Held)
            .or_else(|| newest(VoiceState::Released))
            .or_else(|| newest(VoiceState::Stolen));
        let held = active
            .iter()
            .filter(|&&v| voices.state(v) == VoiceState::Held)
            .count();

        tap.begin(); // the frame's seqlock opens: the stores below are one publish (`Tap::finish` closes it)
        let mut dropped = 0u64;
        let params = plan.tap_params();
        for (k, &id) in params.iter().enumerate() {
            let Some(slot) = tap.params.get(k) else {
                dropped += 1;
                continue;
            };
            let value = match id {
                ParamId::Voice(i) => {
                    let i = usize::from(i);
                    display.map_or(plan.voice_table.base[i], |v| plan.voice_table.cur[i][v])
                }
                ParamId::Global(i) => plan.global_table.cur[usize::from(i)],
            };
            store_f32(slot, value);
        }
        let mods = plan
            .vmod_cur
            .iter()
            .map(|row| display.map_or(0.0, |v| row[v]))
            .chain(plan.gmod_cur.iter().copied());
        let mut n_mods = 0;
        for (k, value) in mods.enumerate() {
            match tap.modulators.get(k) {
                Some(slot) => {
                    store_f32(slot, value);
                    n_mods += 1;
                }
                None => dropped += 1,
            }
        }
        for (slot, &value) in tap
            .macros
            .iter()
            .zip(&plan.global_table.cur[plan.macros.range()])
        {
            store_f32(slot, value);
        }
        store_f32(&tap.peaks[0], pl);
        store_f32(&tap.peaks[1], pr);
        tap.epoch.store(plan.epoch, Relaxed);
        tap.params_len
            .store(params.len().min(tap.params.len()) as u32, Relaxed);
        tap.modulators_len.store(n_mods as u32, Relaxed);
        tap.voices.store(active.len() as u32, Relaxed);
        tap.held.store(held as u32, Relaxed);
        tap.display_voice
            .store(display.map_or(u32::MAX, |v| v as u32), Relaxed);
        tap.display_note
            .store(display.map_or(0, |v| notes.note_index[v]), Relaxed);
        tap.store_stats(&stats);
        if dropped > 0 {
            tap.dropped.fetch_add(dropped, Relaxed);
        }
        tap.finish();
    }
}

/// What step 2 needs to apply note and value events.
struct NoteCx<'a> {
    plan: &'a mut Plan,
    old: Option<&'a mut Plan>,
    voices: &'a mut VoiceManager,
    notes: &'a mut NoteState,
    glide: &'a mut Glide,
    render_seed: u64,
    frame: u64,
    host_slots: &'a mut [f32; SLOTS],
    poly: &'a mut PolyOffsets,
    poly_dirty: &'a mut bool,
    stale_events: &'a mut u64,
}

impl NoteCx<'_> {
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { key, velocity } => self.note_on(key, velocity),
            EventKind::NoteOff { key, .. } => self.note_off(key),
            EventKind::Param { id, value } => set_param(self.plan, id, value),
            EventKind::Macro { index, value } => {
                if usize::from(index) < MOD_WHEEL {
                    self.set_macro(usize::from(index), value);
                }
            }
            EventKind::ModWheel { value } => self.set_macro(MOD_WHEEL, value),
            EventKind::Choke { key } => self.voices.choke(key, |_| {}),
            EventKind::Slot { slot, value } => self.set_slot(usize::from(slot), value),
            EventKind::EpochParam { epoch, id, value } => {
                if epoch == self.plan.epoch {
                    set_param(self.plan, id, value);
                } else {
                    *self.stale_events += 1;
                }
            }
            EventKind::MacroVoice { key, index, offset } => {
                self.set_macro_offset(key, usize::from(index), offset);
            }
        }
    }

    fn set_macro(&mut self, index: usize, value: f32) {
        set_macro(self.plan, index, value);
        if let Some(old) = self.old.as_deref_mut() {
            set_macro(old, index, value);
        }
    }

    /// Remember the host's value of slot `slot` and apply it through each running plan's slot table.
    fn set_slot(&mut self, slot: usize, value: f32) {
        if slot >= SLOTS || value.is_nan() {
            return;
        }
        let n = value.clamp(0.0, 1.0);
        self.host_slots[slot] = n;
        apply_slot(self.plan, slot, n);
        if let Some(old) = self.old.as_deref_mut() {
            apply_slot(old, slot, n);
        }
    }

    fn set_macro_offset(&mut self, key: NoteKey, index: usize, offset: f32) {
        if index >= POLY_MACROS || offset.is_nan() {
            return;
        }
        let offset = offset.clamp(-1.0, 1.0);
        for &v in self.voices.active() {
            if self.voices.state(v) != VoiceState::Free && key.matches(&self.voices.key(v)) {
                self.poly[index][v] = offset;
                *self.poly_dirty = true;
            }
        }
    }

    /// `voice.glide` in segments and `voice.glide_mode`, from the running plan's base values.
    fn glide_settings(&self) -> (u32, GlideMode) {
        let p = &*self.plan;
        let base = &p.voice_table.base[p.voice_builtin.range()];
        let segments = glide_segments(base[VoiceParam::Glide.index()], p.cfg.sample_rate);
        let mode =
            GlideMode::from_value(base[VoiceParam::GlideMode.index()]).unwrap_or(GlideMode::Always);
        (segments, mode)
    }

    fn note_on(&mut self, key: NoteKey, velocity: f32) {
        let (note_index, action) = self.voices.note_on(key, self.frame);
        let v = action.voice();
        let pitch = f32::from(key.key.clamp(0, 127));
        let velocity = if velocity.is_nan() {
            0.0
        } else {
            velocity.clamp(0.0, 1.0)
        };
        // A note landed on the voice: its polyphonic offsets belong to the note it played before.
        for row in self.poly.iter_mut() {
            if row[v] != 0.0 {
                row[v] = 0.0;
                *self.poly_dirty = true;
            }
        }
        let (segments, mode) = self.glide_settings();
        let from = self.glide.origin(action, mode, segments, self.notes);
        let n = &mut *self.notes;
        match action {
            NoteOn::Start { .. } => {
                n.velocity[v] = velocity;
                n.gate[v] = true;
                n.trigger[v] = true;
                n.fresh[v] = true;
                n.release[v] = false;
                n.note_index[v] = note_index;
                self.glide.go(v, pitch, from, segments, true, n);
                self.plan.start_voice(v, n, self.render_seed);
                if let Some(old) = self.old.as_deref_mut() {
                    old.start_voice(v, n, self.render_seed);
                }
            }
            NoteOn::Retrigger { .. } => {
                n.velocity[v] = velocity;
                n.gate[v] = true;
                n.trigger[v] = true;
                n.release[v] = false;
                self.glide.go(v, pitch, from, segments, false, n);
            }
            NoteOn::Legato { .. } => self.glide.go(v, pitch, from, segments, false, n),
        }
    }

    fn note_off(&mut self, key: NoteKey) {
        let notes = &mut *self.notes;
        let back = self.voices.note_off(key, |v| {
            notes.gate[v] = false;
            notes.release[v] = true;
        });
        // Mono and legato: back to the newest held key, gliding (the keys overlap) if glide is on.
        if let Some((v, k)) = back {
            let (segments, _) = self.glide_settings();
            let from = (segments > 0).then_some(self.notes.pitch_cur[v]);
            let pitch = f32::from(k.key.clamp(0, 127));
            self.glide.go(v, pitch, from, segments, false, self.notes);
        }
    }
}

/// A value event for the running plan (clamped; NaN and unknown ids ignored).
fn set_param(plan: &mut Plan, id: ParamId, value: f32) {
    match id {
        ParamId::Voice(i) => plan.voice_table.set(usize::from(i), value),
        ParamId::Global(i) => plan.global_table.set(usize::from(i), value),
    }
}

/// Host value `n` (0–1) of slot `slot`, through the plan's slot table.
fn apply_slot(plan: &mut Plan, slot: usize, n: f32) {
    if let Some(t) = plan.slots.get(slot).and_then(|e| e.target) {
        set_param(plan, t.id, t.resolve(n));
    }
}

/// Macro `index` (0–7) or the mod wheel (8), 0–1, clamped.
fn set_macro(plan: &mut Plan, index: usize, value: f32) {
    if index < MACRO_SOURCES {
        plan.global_table.set(plan.macros.start + index, value);
    }
}

/// What steps 3–8 read, shared by both plans during a crossfade.
struct SegmentCx<'a> {
    active: &'a [VoiceIdx],
    notes: &'a NoteState,
    /// For the steal fades.
    voices: &'a VoiceManager,
    /// The monitor bypass for the time-based effects.
    bypass: Bypass,
    /// Per-voice macro offsets, when any is non-zero.
    poly: Option<&'a PolyOffsets>,
}

/// The buses steps 3–8 write.
struct Out<'a> {
    voice: &'a mut VoiceBus,
    scratch: &'a mut VoiceBus,
    sync: &'a mut SyncBus,
    left: &'a mut Block,
    right: &'a mut Block,
    dry_left: &'a mut Block,
    dry_right: &'a mut Block,
}

/// Steps 3–8 for one plan; the stereo result is left in `buses.left`/`buses.right`.
fn render_plan(plan: &mut Plan, cx: &SegmentCx<'_>, buses: &mut Out<'_>) {
    let (active, notes) = (cx.active, cx.notes);
    let reset = &notes.trigger;
    let Plan {
        generators,
        voice_fx,
        voice_mods,
        global_mods,
        fx,
        fx_time_based,
        amp_env,
        sync_master,
        voice_table,
        global_table,
        voice_builtin,
        master_builtin,
        macros,
        matrix,
        vmod_cur,
        vmod_prev,
        gmod_cur,
        gmod_prev,
        out,
        ..
    } = plan;

    // 3. Smoothers (a trigger restarts the voice's ramps) and route depth ramps.
    voice_table.advance(active, reset);
    global_table.advance();
    matrix.advance_depths();
    let routed = !matrix.is_empty();
    let mut macro_values = [0.0; MACRO_SOURCES];
    macro_values.copy_from_slice(&global_table.cur[macros.range()]);

    // 4. Global modulators, in patch order, each after its own modulated parameters.
    for (i, slot) in global_mods.iter_mut().enumerate() {
        if routed {
            let src = Sources {
                notes,
                vmod: vmod_cur,
                gmod: gmod_cur,
                macros: macro_values,
                poly: cx.poly,
            };
            matrix.eval_global_mod(i, global_table, &src);
        }
        gmod_prev[i] = gmod_cur[i];
        gmod_cur[i] = slot.module.tick(&mut GlobalModCx {
            params: global_table.view(slot.params),
        });
    }

    // 5. Voice modulators, in patch order, over the active voices.
    for (i, slot) in voice_mods.iter_mut().enumerate() {
        if routed {
            let src = Sources {
                notes,
                vmod: vmod_cur,
                gmod: gmod_cur,
                macros: macro_values,
                poly: cx.poly,
            };
            matrix.eval_voice_mod(i, voice_table, &src, active, reset);
        }
        for &v in active {
            vmod_prev[i][v] = vmod_cur[i][v];
        }
        slot.module.tick(&mut VoiceModCx {
            voices: active,
            notes,
            params: voice_table.view(slot.params),
            out: &mut vmod_cur[i],
        });
    }

    // 6. The remaining modulated parameters.
    if routed {
        let src = Sources {
            notes,
            vmod: vmod_cur,
            gmod: gmod_cur,
            macros: macro_values,
            poly: cx.poly,
        };
        matrix.eval_processors(voice_table, global_table, &src, active, reset);
    }

    // 7. Voices.
    for &v in active {
        buses.voice[v] = [0.0; SEG];
    }
    for (g, slot) in generators.iter_mut().enumerate() {
        let (sync_out, sync_in) = match *sync_master {
            Some(m) if g == m => {
                for &v in active {
                    buses.sync[v] = [NO_SYNC; SEG];
                }
                (Some(&mut *buses.sync), None)
            }
            Some(m) if g > m => (None, Some(&*buses.sync)),
            _ => (None, None),
        };
        slot.module.process(&mut VoiceCx {
            voices: active,
            notes,
            params: voice_table.view(slot.params),
            io: buses.scratch,
            sync_out,
            sync_in,
        });
        for &v in active {
            for (acc, x) in buses.voice[v].iter_mut().zip(&buses.scratch[v]) {
                *acc += *x;
            }
        }
    }
    for slot in voice_fx.iter_mut() {
        slot.module.process(&mut VoiceCx {
            voices: active,
            notes,
            params: voice_table.view(slot.params),
            io: buses.voice,
            sync_out: None,
            sync_in: None,
        });
    }
    *buses.left = [0.0; SEG];
    *buses.right = [0.0; SEG];
    let amp = *amp_env;
    let pan_index = voice_builtin.start + VoiceParam::Pan.index();
    for &v in active {
        let pan = voice_table.cur[pan_index][v];
        if notes.fresh[v] || pan.to_bits() != out.pan_value[v].to_bits() {
            out.pan_value[v] = pan;
            out.pan_cur[v] = constant_power_pan(pan * 0.01);
        }
        let (pl, pr) = out.pan_cur[v];
        let (ql, qr) = if notes.fresh[v] {
            (pl, pr)
        } else {
            out.pan_prev[v]
        };
        out.pan_prev[v] = (pl, pr);
        let a0 = vmod_prev[amp][v];
        let a1 = vmod_cur[amp][v];
        let (l0, r0) = (a0 * ql, a0 * qr);
        let (dl, dr) = (a1 * pl - l0, a1 * pr - r0);
        let voice = &buses.voice[v];
        if let Some((f0, f1)) = cx.voices.fade(v) {
            // A steal fade: the amp-and-pan ramp times a linear fade to silence over 64 frames.
            let df = f1 - f0;
            for (((l, r), x), w) in buses
                .left
                .iter_mut()
                .zip(buses.right.iter_mut())
                .zip(voice)
                .zip(RAMP)
            {
                let f = f0 + df * w;
                *l += x * ((l0 + dl * w) * f);
                *r += x * ((r0 + dr * w) * f);
            }
        } else {
            for (((l, r), x), w) in buses
                .left
                .iter_mut()
                .zip(buses.right.iter_mut())
                .zip(voice)
                .zip(RAMP)
            {
                *l += x * (l0 + dl * w);
                *r += x * (r0 + dr * w);
            }
        }
    }

    // 8. Global effects in order (time-based ones faded out and skipped by the monitor bypass), then the master
    //    level.
    for (slot, &time_based) in fx.iter_mut().zip(fx_time_based.iter()) {
        let bypass = if time_based { cx.bypass } else { Bypass::Off };
        if bypass == Bypass::On
            || (slot.module.is_silent() && is_zero(buses.left) && is_zero(buses.right))
        {
            continue;
        }
        if matches!(bypass, Bypass::Fade { .. }) {
            *buses.dry_left = *buses.left;
            *buses.dry_right = *buses.right;
        }
        slot.module.process(&mut GlobalCx {
            params: global_table.view(slot.params),
            left: buses.left,
            right: buses.right,
        });
        if let Bypass::Fade { g0, g1 } = bypass {
            // wet·g + dry·(1 − g): exactly the dry signal at g = 0 and the wet one at g = 1.
            let dg = g1 - g0;
            for ((((l, r), dl), dr), w) in buses
                .left
                .iter_mut()
                .zip(buses.right.iter_mut())
                .zip(buses.dry_left.iter())
                .zip(buses.dry_right.iter())
                .zip(RAMP)
            {
                let g = g0 + dg * w;
                *l = *l * g + dl * (1.0 - g);
                *r = *r * g + dr * (1.0 - g);
            }
        }
    }
    let level = global_table.cur[master_builtin.start + MasterParam::Level.index()];
    out.master_prev = out.master_cur;
    if level.to_bits() != out.master_db.to_bits() {
        out.master_db = level;
        out.master_cur = db_to_gain(level);
    }
    let (g0, g1) = (out.master_prev, out.master_cur);
    if g0 != 1.0 || g1 != 1.0 {
        let dg = g1 - g0;
        for ((l, r), w) in buses.left.iter_mut().zip(buses.right.iter_mut()).zip(RAMP) {
            let g = g0 + dg * w;
            *l *= g;
            *r *= g;
        }
    }
}

fn is_zero(block: &Block) -> bool {
    block.iter().all(|x| *x == 0.0)
}
