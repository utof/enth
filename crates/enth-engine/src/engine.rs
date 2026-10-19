//! The engine: the segment loop behind the host adapter.
//!
//! [`Engine::process`] takes any number of frames and the host block's events, and fills the output from a
//! one-segment FIFO ([`crate::segment`]), rendering a new segment whenever it runs dry. Every segment runs the same
//! fixed steps in the same order, which is where the determinism lives (design.md §3):
//!
//! 1. poll the command ring (plan swaps, epoch-tagged value events) — TODO(S2b);
//! 2. apply the events due at this boundary, in arrival order (notes, parameter values);
//! 3. advance the base-value smoothers — S2a copies `base` into `cur` (TODO(S2b): smoothing);
//! 4. global modulators, in patch order;
//! 5. voice modulators, in patch order, each over the active voices in ascending order;
//! 6. modulated parameter values — TODO(S2b): `n = clamp(base_norm + Σ depth·src)` in patch route order;
//! 7. voices: generators sum into each voice's bus in patch order (the first enabled `osc` writes the hard-sync
//!    timing that later generators read), then `voice_fx` in order, then the amp stage
//!    (ramped per sample from the amp envelope's previous to current value) with the constant-power pan, into the
//!    stereo sum in ascending voice order;
//! 8. `fx` in order (a processor whose state is silent and whose input is zero is skipped), then `master.level`;
//! 9. end finished voices (past the attack and below −100 dB); modules flush their own state below 1e-15;
//! 10. write the FIFO.
//!
//! `process` never allocates, frees, locks, logs or does I/O, and it does not panic on any input: mismatched
//! channel lengths, out-of-range event offsets, unknown parameter ids, NaN values and a full event queue are all
//! handled (clamped, ignored or dropped and counted).

use enth_dsp::util::math::{constant_power_pan, db_to_gain};
use enth_patch::catalogue::{MasterParam, VoiceParam};

use crate::event::{Event, EventKind, NoteKey};
use crate::module::{
    Block, GlobalCx, GlobalModCx, GlobalStart, MAX_VOICES, NO_SYNC, NoteState, RAMP, SEG, SyncBus,
    VoiceBus, VoiceCx, VoiceIdx, VoiceModCx, VoiceStart,
};
use crate::params::ParamId;
use crate::plan::Plan;
use crate::seed::{global_seed, voice_seed};
use crate::segment::{EVENT_CAPACITY, EventQueue, Fifo};
use crate::voice::VoiceManager;

/// Scratch buses, boxed so the engine stays small to move.
struct Buses {
    voice: VoiceBus,
    scratch: VoiceBus,
    sync: SyncBus,
    left: Block,
    right: Block,
}

/// Counters for things the engine handled without failing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Note-ons since the engine started (the next note index).
    pub notes: u64,
    /// Note-ons dropped because polyphony was full (S2a has no stealing).
    pub dropped_notes: u64,
    /// Events dropped because the event queue was full.
    pub dropped_events: u64,
    /// Segments rendered.
    pub segments: u64,
}

/// The real-time engine. Build it with [`Engine::new`] from a compiled [`Plan`] (allocates); then call
/// [`Engine::process`] from the audio thread (never allocates).
pub struct Engine {
    render_seed: u64,
    plan: Box<Plan>,
    voices: VoiceManager,
    notes: NoteState,
    buses: Box<Buses>,
    /// Per voice: the pan value the cached gains belong to, and the gains of this and the previous segment.
    pan_value: [f32; MAX_VOICES],
    pan_cur: [(f32, f32); MAX_VOICES],
    pan_prev: [(f32, f32); MAX_VOICES],
    /// `master.level` in dB the cached gain belongs to, the gain, and last segment's gain.
    master_db: f32,
    master_cur: f32,
    master_prev: f32,
    fifo: Fifo,
    queue: EventQueue,
    /// Absolute frame of the next host frame.
    position: u64,
    /// Absolute frame of the next segment's start (a multiple of `SEG`).
    next_segment: u64,
    segments: u64,
}

impl Engine {
    /// An engine running `plan`, with every random stream derived from `render_seed`. Global modules start here.
    /// Allocates (the buses, the event queue); `process` never does.
    #[must_use]
    pub fn new(plan: Plan, render_seed: u64) -> Self {
        let mut plan = Box::new(plan);
        let voices = VoiceManager::new(plan.polyphony);
        {
            let Plan {
                global_mods,
                fx,
                global_table,
                ..
            } = &mut *plan;
            for slot in global_mods.iter_mut() {
                slot.module.reset(&GlobalStart {
                    seed: global_seed(render_seed, slot.key),
                    module_key: slot.key,
                    params: global_table.view(slot.params),
                });
            }
            for slot in fx.iter_mut() {
                slot.module.reset(&GlobalStart {
                    seed: global_seed(render_seed, slot.key),
                    module_key: slot.key,
                    params: global_table.view(slot.params),
                });
            }
        }
        let master_db =
            plan.global_table.cur[plan.master_builtin.start + MasterParam::Level.index()];
        let master = db_to_gain(master_db);
        let centre = constant_power_pan(0.0);
        Self {
            render_seed,
            plan,
            voices,
            notes: NoteState::new(),
            buses: Box::new(Buses {
                voice: [[0.0; SEG]; MAX_VOICES],
                scratch: [[0.0; SEG]; MAX_VOICES],
                sync: [[NO_SYNC; SEG]; MAX_VOICES],
                left: [0.0; SEG],
                right: [0.0; SEG],
            }),
            pan_value: [0.0; MAX_VOICES],
            pan_cur: [centre; MAX_VOICES],
            pan_prev: [centre; MAX_VOICES],
            master_db,
            master_cur: master,
            master_prev: master,
            fifo: Fifo::new(),
            queue: EventQueue::new(EVENT_CAPACITY),
            position: 0,
            next_segment: 0,
            segments: 0,
        }
    }

    /// Render `left.len()` frames (the shorter channel's length; the rest of the longer one is zeroed) with this
    /// block's `events`, whose offsets count from `left[0]` (an offset past the block is taken as its last frame).
    /// Events take effect at the first segment boundary at or after their frame; the block size never matters.
    #[cfg_attr(enth_rtsan, sanitize(realtime = "nonblocking"))]
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], events: &[Event]) {
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

    /// The playing voices, ascending.
    #[must_use]
    pub fn active_voices(&self) -> &[VoiceIdx] {
        self.voices.active()
    }

    /// The counters.
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            notes: self.voices.note_count(),
            dropped_notes: self.voices.dropped_notes(),
            dropped_events: self.queue.dropped(),
            segments: self.segments,
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

    /// The number of subnormal values in the engine's state: every module, the modulator outputs and the engine's
    /// own ramps (test support for the release-tail invariant; walks the state, does not allocate).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        let own = self
            .pan_cur
            .iter()
            .chain(&self.pan_prev)
            .filter(|(l, r)| l.is_subnormal() || r.is_subnormal())
            .count()
            + usize::from(self.master_cur.is_subnormal())
            + usize::from(self.master_prev.is_subnormal());
        self.plan.subnormals() + own
    }

    /// One segment: steps 1–10 (see the module docs).
    fn render_segment(&mut self) {
        let frame = self.next_segment;
        // 1. Commands: TODO(S2b) poll the command ring (plan swap with state carry, epoch-tagged value events).

        // 2. Events due at this boundary, in arrival order.
        self.begin_notes();
        {
            let Self {
                queue,
                plan,
                voices,
                notes,
                render_seed,
                ..
            } = self;
            queue.drain_due(frame, |kind| {
                apply_event(kind, plan, voices, notes, *render_seed);
            });
        }
        let mut active_buf = [0; MAX_VOICES];
        let n_active = {
            let a = self.voices.active();
            active_buf[..a.len()].copy_from_slice(a);
            a.len()
        };
        let active = &active_buf[..n_active];

        let Self {
            plan,
            notes,
            buses,
            pan_value,
            pan_cur,
            pan_prev,
            master_db,
            master_cur,
            master_prev,
            ..
        } = self;
        let plan = &mut **plan;
        let buses = &mut **buses;

        // 3. Smoothers. S2a: no smoothing, `cur = base` (settled values are exact).
        plan.voice_table.advance(active, &notes.fresh);
        plan.global_table.advance();

        // 4. Global modulators, in patch order.
        for (i, slot) in plan.global_mods.iter_mut().enumerate() {
            plan.gmod_prev[i] = plan.gmod_cur[i];
            plan.gmod_cur[i] = slot.module.tick(&mut GlobalModCx {
                params: plan.global_table.view(slot.params),
            });
        }

        // 5. Voice modulators, in patch order, over the active voices.
        for (i, slot) in plan.voice_mods.iter_mut().enumerate() {
            for &v in active {
                plan.vmod_prev[i][v] = plan.vmod_cur[i][v];
            }
            slot.module.tick(&mut VoiceModCx {
                voices: active,
                notes,
                params: plan.voice_table.view(slot.params),
                out: &mut plan.vmod_cur[i],
            });
        }

        // 6. Modulated parameters: TODO(S2b). The S2a compiler refuses routes, so there is nothing to add.

        // 7. Voices.
        for &v in active {
            buses.voice[v] = [0.0; SEG];
        }
        for (g, slot) in plan.generators.iter_mut().enumerate() {
            let (sync_out, sync_in) = match plan.sync_master {
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
                params: plan.voice_table.view(slot.params),
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
        for slot in &mut plan.voice_fx {
            slot.module.process(&mut VoiceCx {
                voices: active,
                notes,
                params: plan.voice_table.view(slot.params),
                io: &mut buses.voice,
                sync_out: None,
                sync_in: None,
            });
        }
        buses.left = [0.0; SEG];
        buses.right = [0.0; SEG];
        let amp = plan.amp_env;
        let pan_index = plan.voice_builtin.start + VoiceParam::Pan.index();
        for &v in active {
            let pan = plan.voice_table.cur[pan_index][v];
            if notes.fresh[v] || pan.to_bits() != pan_value[v].to_bits() {
                pan_value[v] = pan;
                pan_cur[v] = constant_power_pan(pan * 0.01);
            }
            let (pl, pr) = pan_cur[v];
            let (ql, qr) = if notes.fresh[v] {
                (pl, pr)
            } else {
                pan_prev[v]
            };
            pan_prev[v] = (pl, pr);
            let a0 = plan.vmod_prev[amp][v];
            let a1 = plan.vmod_cur[amp][v];
            let (l0, r0) = (a0 * ql, a0 * qr);
            let (dl, dr) = (a1 * pl - l0, a1 * pr - r0);
            let voice = &buses.voice[v];
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

        // 8. Global effects in order, then the master level.
        for slot in &mut plan.fx {
            if slot.module.is_silent() && is_zero(&buses.left) && is_zero(&buses.right) {
                continue;
            }
            slot.module.process(&mut GlobalCx {
                params: plan.global_table.view(slot.params),
                left: &mut buses.left,
                right: &mut buses.right,
            });
        }
        let level = plan.global_table.cur[plan.master_builtin.start + MasterParam::Level.index()];
        *master_prev = *master_cur;
        if level.to_bits() != master_db.to_bits() {
            *master_db = level;
            *master_cur = db_to_gain(level);
        }
        let (g0, g1) = (*master_prev, *master_cur);
        if g0 != 1.0 || g1 != 1.0 {
            let dg = g1 - g0;
            for ((l, r), w) in buses.left.iter_mut().zip(&mut buses.right).zip(RAMP) {
                let g = g0 + dg * w;
                *l *= g;
                *r *= g;
            }
        }

        // 9. End finished voices. (Modules flush their own state; nothing in the engine recurses.)
        let amp_env = &plan.voice_mods[amp].module;
        for &v in active {
            if amp_env.voice_finished(v) {
                self.voices.end(v);
                notes.gate[v] = false;
            }
        }

        // 10. The FIFO.
        debug_assert!(
            buses.left.iter().chain(&buses.right).all(|x| x.is_finite()),
            "non-finite output sample"
        );
        self.fifo.fill(&buses.left, &buses.right);
        self.next_segment += SEG as u64;
        self.segments += 1;
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

fn is_zero(block: &Block) -> bool {
    block.iter().all(|x| *x == 0.0)
}

/// Step 2 for one event.
fn apply_event(
    kind: EventKind,
    plan: &mut Plan,
    voices: &mut VoiceManager,
    notes: &mut NoteState,
    render_seed: u64,
) {
    match kind {
        EventKind::NoteOn { key, velocity } => {
            note_on(plan, voices, notes, render_seed, key, velocity);
        }
        EventKind::NoteOff { key, .. } => voices.note_off(key, |v| {
            notes.gate[v] = false;
            notes.release[v] = true;
        }),
        EventKind::Param { id, value } => match id {
            ParamId::Voice(i) => plan.voice_table.set(usize::from(i), value),
            ParamId::Global(i) => plan.global_table.set(usize::from(i), value),
        },
    }
}

fn note_on(
    plan: &mut Plan,
    voices: &mut VoiceManager,
    notes: &mut NoteState,
    render_seed: u64,
    key: NoteKey,
    velocity: f32,
) {
    let (note_index, slot) = voices.note_on(key);
    let Some(v) = slot else {
        return; // TODO(S2b): steal instead of dropping.
    };
    let pitch = f32::from(key.key.clamp(0, 127));
    notes.pitch_prev[v] = pitch;
    notes.pitch_cur[v] = pitch;
    notes.velocity[v] = if velocity.is_nan() {
        0.0
    } else {
        velocity.clamp(0.0, 1.0)
    };
    notes.gate[v] = true;
    notes.trigger[v] = true;
    notes.fresh[v] = true;
    notes.release[v] = false;
    notes.note_index[v] = note_index;
    // A fresh voice's modulator outputs start from 0 (the amp stage ramps up from silence).
    for out in &mut plan.vmod_cur {
        out[v] = 0.0;
    }
    let Plan {
        generators,
        voice_fx,
        voice_mods,
        voice_table,
        ..
    } = plan;
    for slot in generators.iter_mut().chain(voice_fx.iter_mut()) {
        slot.module.start_voice(&VoiceStart {
            voice: v,
            seed: voice_seed(render_seed, slot.key, note_index),
            module_key: slot.key,
            note_index,
            notes,
            params: voice_table.view(slot.params),
        });
    }
    for slot in voice_mods.iter_mut() {
        slot.module.start_voice(&VoiceStart {
            voice: v,
            seed: voice_seed(render_seed, slot.key, note_index),
            module_key: slot.key,
            note_index,
            notes,
            params: voice_table.view(slot.params),
        });
    }
}
