//! The engine: the segment loop behind the host adapter.
//!
//! [`Engine::process`] takes any number of frames and the host block's events, and fills the output from a
//! one-segment FIFO ([`crate::segment`]), rendering a new segment whenever it runs dry. Every segment runs the same
//! fixed steps in the same order, which is where the determinism lives (design.md §3):
//!
//! 1. poll the command ring ([`crate::swap`]): plan swaps (state carried, a 256-frame crossfade when modules were
//!    added or removed), epoch-tagged value events (stale ones dropped), macros;
//! 2. apply the events due at this boundary, in arrival order (notes through the voice manager, parameter values,
//!    macros, the mod wheel, chokes), then advance glides one segment;
//! 3. advance the smoothers (normalised domain; settled parameters deliver the exact plain value);
//! 4. global modulators, in patch order, each after its own modulated parameters;
//! 5. voice modulators, in patch order, each after its own modulated parameters, over the active voices;
//! 6. the remaining modulated parameters: `n = clamp(smoothed_norm + Σ depth·src, 0, 1)` in patch route order;
//! 7. voices: generators sum into each voice's bus in patch order (the first enabled `osc` writes the hard-sync
//!    timing that later generators read), then `voice_fx` in order, then the amp stage (ramped per sample from the
//!    amp envelope's previous to current value, times a steal fade) with the constant-power pan, into the stereo sum
//!    in ascending voice order;
//! 8. `fx` in order (a processor whose state is silent and whose input is zero is skipped; with the monitor bypass
//!    on, `time_based` ones are skipped), then `master.level`;
//! 9. end finished voices (past the attack and below −100 dB, or a steal fade over); modules flush their own state
//!    below 1e-15;
//! 10. write the FIFO.
//!
//! During a crossfade, steps 3–8 run for both plans (the replaced one first) over the same notes, and the output is
//! `old + (new − old)·w` with a raised-cosine `w` over 256 frames: an equal-gain fade, exact where both plans agree.
//!
//! `process` never allocates, frees, locks, logs or does I/O, and it does not panic on any input: mismatched
//! channel lengths, out-of-range event offsets, unknown parameter ids, NaN values, a full event queue, full rings
//! and a full return ring are all handled (clamped, ignored, kept or dropped and counted).

use enth_dsp::util::math::{constant_power_pan, cos, db_to_gain};
use enth_patch::catalogue::{GlideMode, MasterParam, VoiceParam};

use crate::event::{Event, EventKind, NoteKey};
use crate::modulation::{MACRO_SOURCES, MOD_WHEEL, Sources};
use crate::module::{
    Block, GlobalCx, GlobalModCx, MAX_VOICES, NO_SYNC, NoteState, RAMP, SEG, SyncBus, VoiceBus,
    VoiceCx, VoiceIdx, VoiceModCx,
};
use crate::params::ParamId;
use crate::plan::Plan;
use crate::segment::{EVENT_CAPACITY, EventQueue, Fifo};
use crate::swap::{AudioLink, Cmd, Link, LinkCapacity, rings};
use crate::voice::{Glide, NoteEnd, NoteOn, VoiceManager, VoiceState, glide_segments};

/// Frames a plan-swap crossfade lasts (only when modules were added or removed).
pub const CROSSFADE_FRAMES: usize = 256;

const CROSSFADE_SEGMENTS: usize = CROSSFADE_FRAMES / SEG;

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
    /// The crossfade weight per frame: `0.5 − 0.5·cos(π·(i + 1)/256)`, rising to exactly 1.
    xfade: [f32; CROSSFADE_FRAMES],
}

/// Counters for things the engine handled without failing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Value events from the command ring dropped because they named another plan's epoch.
    pub stale_events: u64,
    /// Plans swapped in.
    pub swaps: u64,
    /// Swaps that crossfaded (a module was added or removed).
    pub crossfades: u64,
    /// Voice ends that did not fit one `process` call's report.
    pub dropped_note_ends: u64,
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
    /// The monitor bypass as asked, and as applied at the last segment boundary.
    monitor_bypass: bool,
    bypass_applied: bool,
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
                xfade,
            }),
            fifo: Fifo::new(),
            queue: EventQueue::new(EVENT_CAPACITY),
            position: 0,
            next_segment: 0,
            segments: 0,
            monitor_bypass: false,
            bypass_applied: false,
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
        self.position += n as u64;
    }

    /// Mute the effects whose kind is flagged `time_based` in the catalogue (the reverb in P1): the plugin's global
    /// FX bypass, a monitoring aid. It is engine state, not patch state, and `enth-render` never sets it, so renders
    /// and scores cannot see it (design.md §5, ADR-0006). Takes effect at the next segment boundary; engaging it
    /// resets those effects, so releasing it starts them from silence instead of resuming a stale tail. Cheap and
    /// allocation-free: call it from the audio thread before `process`.
    pub fn set_monitor_bypass(&mut self, on: bool) {
        self.monitor_bypass = on;
    }

    /// Whether the monitor bypass is on.
    #[must_use]
    pub fn monitor_bypass(&self) -> bool {
        self.monitor_bypass
    }

    /// The playing voices (held, releasing or fading), ascending.
    #[must_use]
    pub fn active_voices(&self) -> &[VoiceIdx] {
        self.voices.active()
    }

    /// The voices that ended during the last `process` call, with the frame they went silent (for CLAP `NOTE_END`).
    #[must_use]
    pub fn note_ends(&self) -> &[NoteEnd] {
        self.voices.ends()
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

    /// The epoch of the plan being run: host automation (`EventKind::Param`) must use ids from its
    /// [`PlanMap`](crate::PlanMap).
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
        self.poll_commands();
        if self.monitor_bypass && !self.bypass_applied {
            self.plan.reset_time_based(self.render_seed);
            if let Some(old) = self.old.as_deref_mut() {
                old.reset_time_based(self.render_seed);
            }
        }
        self.bypass_applied = self.monitor_bypass;

        // 2. Events due at this boundary, in arrival order; then the glides.
        self.begin_notes();
        {
            let Self {
                queue,
                plan,
                old,
                voices,
                notes,
                glide,
                render_seed,
                ..
            } = self;
            let mut cx = NoteCx {
                plan,
                old: old.as_deref_mut(),
                voices,
                notes,
                glide,
                render_seed: *render_seed,
                frame,
            };
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

        // 3–8, per plan: the replaced one first while it crossfades.
        let cx = SegmentCx {
            active,
            notes: &self.notes,
            voices: &self.voices,
            bypass: self.monitor_bypass,
        };
        let buses = &mut *self.buses;
        let crossfading = self.old.is_some() && self.xfade_seg < CROSSFADE_SEGMENTS;
        if crossfading && let Some(old) = self.old.as_deref_mut() {
            render_plan(old, &cx, buses);
            buses.old_left = buses.left;
            buses.old_right = buses.right;
        }
        render_plan(&mut self.plan, &cx, buses);
        if crossfading {
            let w = &buses.xfade[self.xfade_seg * SEG..(self.xfade_seg + 1) * SEG];
            for (((l, r), (ol, or)), w) in buses
                .left
                .iter_mut()
                .zip(&mut buses.right)
                .zip(buses.old_left.iter().zip(&buses.old_right))
                .zip(w)
            {
                *l = ol + (*l - ol) * w;
                *r = or + (*r - or) * w;
            }
            self.xfade_seg += 1;
        }
        debug_assert!(
            buses.left.iter().chain(&buses.right).all(|x| x.is_finite()),
            "non-finite output sample"
        );

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

    /// Step 1: read commands until the ring is empty, a plan has to wait (a crossfade runs) or a replaced plan waits
    /// for room on the return ring (back-pressure: the commands behind it stay queued, in order).
    fn poll_commands(&mut self) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        if link.flush_pending() {
            loop {
                if link.next_is_plan() && self.old.is_some() {
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
                }
            }
        }
        self.link = Some(link);
    }

    /// Swap `new` in at this boundary: carry the running state, adopt its voice mode, and either crossfade (a
    /// module was added or removed) or hand the old plan back at once.
    fn swap(&mut self, mut new: Box<Plan>, link: &mut AudioLink) {
        let changed = new.carry_from(
            &self.plan,
            self.voices.active(),
            &self.notes,
            self.render_seed,
        );
        if self.monitor_bypass {
            new.reset_time_based(self.render_seed);
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
        }
    }

    fn set_macro(&mut self, index: usize, value: f32) {
        set_macro(self.plan, index, value);
        if let Some(old) = self.old.as_deref_mut() {
            set_macro(old, index, value);
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
    bypass: bool,
}

/// Steps 3–8 for one plan; the stereo result is left in `buses.left`/`buses.right`.
fn render_plan(plan: &mut Plan, cx: &SegmentCx<'_>, buses: &mut Buses) {
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

    // 3. Smoothers (a trigger restarts the voice's ramps).
    voice_table.advance(active, reset);
    global_table.advance();
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
                (Some(&mut buses.sync), None)
            }
            Some(m) if g > m => (None, Some(&buses.sync)),
            _ => (None, None),
        };
        slot.module.process(&mut VoiceCx {
            voices: active,
            notes,
            params: voice_table.view(slot.params),
            io: &mut buses.scratch,
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
            io: &mut buses.voice,
            sync_out: None,
            sync_in: None,
        });
    }
    buses.left = [0.0; SEG];
    buses.right = [0.0; SEG];
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
                .zip(&mut buses.right)
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
                .zip(&mut buses.right)
                .zip(voice)
                .zip(RAMP)
            {
                *l += x * (l0 + dl * w);
                *r += x * (r0 + dr * w);
            }
        }
    }

    // 8. Global effects in order (time-based ones muted by the monitor bypass), then the master level.
    for (slot, &time_based) in fx.iter_mut().zip(fx_time_based.iter()) {
        if (cx.bypass && time_based)
            || (slot.module.is_silent() && is_zero(&buses.left) && is_zero(&buses.right))
        {
            continue;
        }
        slot.module.process(&mut GlobalCx {
            params: global_table.view(slot.params),
            left: &mut buses.left,
            right: &mut buses.right,
        });
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
        for ((l, r), w) in buses.left.iter_mut().zip(&mut buses.right).zip(RAMP) {
            let g = g0 + dg * w;
            *l *= g;
            *r *= g;
        }
    }
}

fn is_zero(block: &Block) -> bool {
    block.iter().all(|x| *x == 0.0)
}
