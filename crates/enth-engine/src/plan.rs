//! `Plan`: the compiled form of a `Patch` that the audio thread runs, [`compile`], and the state carry of a plan
//! swap.
//!
//! A plan holds every enabled module prepared ([`ModuleImpl::new`]) and sorted by role into ordered lists, the flat
//! parameter tables and their smoothers ([`crate::params`]), the compiled routes ([`crate::modulation`]), each
//! module's stable key (`fnv1a64` of its id, for seeds and for the state carry), the modulator output slots and the
//! output stage's ramps. P1's topology is fixed (four sections), so the "graph" is just ordered lists; there is no
//! general scheduling. Compiling allocates; running a plan never does.
//!
//! [`PlanMap`] maps parameter addresses to the plan's [`ParamId`]s. It stays on the main thread (or in the offline
//! renderer), where an op or an automation point becomes a value event.
//!
//! **Routes** compile into one list of destinations per evaluation stage (see [`crate::modulation`]). The compiler
//! checks what the audio thread relies on (a subset of S1b's `validate`): a module source names a modulator in the
//! patch, the destination exists and is `modulatable`, `|depth| ≤ 100`, and a voice-rate source (velocity, key, a
//! voice-scope modulator) never targets a global destination ([`CompileError::BadRoute`]). A route from or to a
//! disabled module, or with depth 0, contributes nothing and is left out.
//!
//! **Macros and the mod wheel** are nine global-table parameters after `master.*` (spec `MACRO_VALUE`, smoothed over
//! 10 ms); the macros start at the patch's values, the mod wheel at 0.
//!
//! **State carry** ([`Plan::carry_from`], on the audio thread at a swap): every module whose (id, kind) survives
//! copies the old module's state (`copy_state_from`), its modulator outputs and its parameters' running values and
//! smoothers; so do `voice.*`, `master.*`, the macros and the output ramps. Fresh voice modules start every active
//! voice (`start_voice` with the voice's own note index, so the seeds are the ones the note would have had), fresh
//! global modules `reset`. Whether any module was added or removed decides the crossfade.
//!
//! Disabled modules are left out (a disabled generator is silent, a disabled effect is bypassed); enabling one is a
//! structural edit. The amplitude envelope always runs, even if disabled (whether disabling it is valid is S1b's
//! `validate`).

use std::collections::BTreeMap;
use std::fmt;

use enth_dsp::util::math::{constant_power_pan, db_to_gain};
use enth_params::ParamSpec;
use enth_patch::catalogue::{
    MACRO_VALUE, MASTER_PARAMS, MasterParam, VOICE_PARAMS, VoiceMode, VoiceParam,
};
use enth_patch::model::SLOTS;
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Rate, RouteAddr, Section,
};

use crate::modulation::{MACRO_SOURCES, Matrix, MatrixBuilder, Src, Stage, Term};
use crate::module::{GlobalStart, MAX_VOICES, NoteState, PrepareCfg, VoiceIdx, VoiceStart};
use crate::modules::{
    GlobalModulatorImpl, GlobalProcessorImpl, ModuleImpl, VoiceModulatorImpl, VoiceProcessorImpl,
};
use crate::params::{GlobalTable, ParamId, Span, VoiceTable};
use crate::seed::{global_seed, module_key, voice_seed};

/// The specs of the macro and mod-wheel sources in the global table.
static MACRO_SPECS: [ParamSpec; MACRO_SOURCES] = [MACRO_VALUE; MACRO_SOURCES];

/// One module in a plan, with what the engine needs to run it.
pub(crate) struct Slot<M> {
    pub module: M,
    /// The module id (for state carry and diagnostics).
    pub id: ModuleId,
    /// `fnv1a64(id)`: seeds and state carry.
    pub key: u64,
    /// Its parameters in the voice or global table.
    pub params: Span,
}

/// The parameter a host automation slot drives in this plan (P1b lane E+).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlotTarget {
    pub id: ParamId,
    pub spec: &'static ParamSpec,
    /// The patch value (updated by value events from the command ring, so the host's echo of an editor edit
    /// resolves exactly) and its `to_norm`.
    pub plain: f32,
    pub norm: f32,
}

impl SlotTarget {
    /// The base value for the host's normalised value `n`: the exact patch value if `n` is bit-equal to its
    /// `to_norm`, else `from_norm(n)` (`from_norm(to_norm(x))` is not `x` for most values; report 01 F3).
    pub fn resolve(&self, n: f32) -> f32 {
        if n.to_bits() == self.norm.to_bits() {
            self.plain
        } else {
            self.spec.from_norm(n)
        }
    }
}

/// One host automation slot of a plan.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SlotEntry {
    /// `fnv1a64` of the target's address text (never 0), or 0 for an empty slot: a plan swap re-applies a remembered
    /// host value only to a slot whose key is unchanged.
    pub key: u64,
    /// `None` for an empty slot or a target in a disabled module.
    pub target: Option<SlotTarget>,
}

/// A route's depth in a plan, for [`Link::send_depth`](crate::swap::Link::send_depth): from
/// [`PlanMap::depth`]. Valid for the plan whose map gave it (tag it with that plan's epoch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DepthId(pub(crate) u16);

/// The output stage's per-plan ramps: the pan gains per voice and the master gain.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Output {
    /// The pan value the cached gains belong to, per voice.
    pub pan_value: [f32; MAX_VOICES],
    /// This and the previous segment's pan gains, per voice.
    pub pan_cur: [(f32, f32); MAX_VOICES],
    pub pan_prev: [(f32, f32); MAX_VOICES],
    /// `master.level` in dB the cached gain belongs to, the gain, and last segment's gain.
    pub master_db: f32,
    pub master_cur: f32,
    pub master_prev: f32,
}

impl Output {
    fn new(master_db: f32) -> Self {
        let centre = constant_power_pan(0.0);
        let g = db_to_gain(master_db);
        Self {
            pan_value: [0.0; MAX_VOICES],
            pan_cur: [centre; MAX_VOICES],
            pan_prev: [centre; MAX_VOICES],
            master_db,
            master_cur: g,
            master_prev: g,
        }
    }

    fn subnormals(&self) -> usize {
        self.pan_cur
            .iter()
            .chain(&self.pan_prev)
            .filter(|(l, r)| l.is_subnormal() || r.is_subnormal())
            .count()
            + usize::from(self.master_cur.is_subnormal())
            + usize::from(self.master_prev.is_subnormal())
    }
}

/// A compiled patch: what the audio thread runs. Build one with [`compile`].
pub struct Plan {
    pub(crate) cfg: PrepareCfg,
    /// Set by [`Link::send_plan`](crate::swap::Link::send_plan); 0 for a plan the engine starts with.
    pub(crate) epoch: u64,
    pub(crate) generators: Vec<Slot<VoiceProcessorImpl>>,
    pub(crate) voice_fx: Vec<Slot<VoiceProcessorImpl>>,
    pub(crate) voice_mods: Vec<Slot<VoiceModulatorImpl>>,
    pub(crate) global_mods: Vec<Slot<GlobalModulatorImpl>>,
    pub(crate) fx: Vec<Slot<GlobalProcessorImpl>>,
    /// Per `fx` slot: the kind is `time_based` (the monitor bypass may mute it).
    pub(crate) fx_time_based: Vec<bool>,
    /// Index of the amplitude envelope in `voice_mods`.
    pub(crate) amp_env: usize,
    /// Index of the hard-sync master in `generators`: the first enabled `osc`.
    pub(crate) sync_master: Option<usize>,
    pub(crate) voice_table: VoiceTable,
    pub(crate) global_table: GlobalTable,
    /// `voice.*` in the voice table.
    pub(crate) voice_builtin: Span,
    /// `master.*` in the global table.
    pub(crate) master_builtin: Span,
    /// The eight macros and the mod wheel in the global table.
    pub(crate) macros: Span,
    pub(crate) mode: VoiceMode,
    pub(crate) polyphony: usize,
    pub(crate) matrix: Matrix,
    /// Voice modulator outputs, per modulator and voice: this segment's and the previous one's.
    pub(crate) vmod_cur: Vec<[f32; MAX_VOICES]>,
    pub(crate) vmod_prev: Vec<[f32; MAX_VOICES]>,
    /// Global modulator outputs.
    pub(crate) gmod_cur: Vec<f32>,
    pub(crate) gmod_prev: Vec<f32>,
    pub(crate) out: Output,
    /// The host automation slots (P1b lane E+).
    pub(crate) slots: [SlotEntry; SLOTS],
    /// The routed parameters in telemetry order: the voice table's, then the global table's (P1b lane E+).
    pub(crate) tap_params: Vec<ParamId>,
    /// Slots (bit `i` = slot index `i`) whose remembered host value this plan forgets instead of re-applying when
    /// it swaps in ([`Plan::forget_host_slots`]; F1b review).
    pub(crate) forget_slots: u64,
}

impl fmt::Debug for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids = |s: &[&ModuleId]| s.iter().map(|i| i.as_str()).collect::<Vec<_>>().join(",");
        f.debug_struct("Plan")
            .field("sample_rate", &self.cfg.sample_rate)
            .field("epoch", &self.epoch)
            .field(
                "generators",
                &ids(&self.generators.iter().map(|s| &s.id).collect::<Vec<_>>()),
            )
            .field(
                "voice_fx",
                &ids(&self.voice_fx.iter().map(|s| &s.id).collect::<Vec<_>>()),
            )
            .field(
                "voice_mods",
                &ids(&self.voice_mods.iter().map(|s| &s.id).collect::<Vec<_>>()),
            )
            .field(
                "global_mods",
                &ids(&self.global_mods.iter().map(|s| &s.id).collect::<Vec<_>>()),
            )
            .field(
                "fx",
                &ids(&self.fx.iter().map(|s| &s.id).collect::<Vec<_>>()),
            )
            .field("mode", &self.mode)
            .field("polyphony", &self.polyphony)
            .finish_non_exhaustive()
    }
}

impl Plan {
    /// The configuration the modules were prepared with.
    #[must_use]
    pub fn cfg(&self) -> PrepareCfg {
        self.cfg
    }

    /// The plan's epoch: 0 for the plan an engine starts with, then the number
    /// [`Link::send_plan`](crate::swap::Link::send_plan) assigned.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The voice mode (`voice.mode`).
    #[must_use]
    pub fn mode(&self) -> VoiceMode {
        self.mode
    }

    /// The polyphony limit (`voice.polyphony`).
    #[must_use]
    pub fn polyphony(&self) -> usize {
        self.polyphony
    }

    /// The ids of the modules that run, in segment order: global modulators, voice modulators, generators,
    /// per-voice effects, global effects.
    #[must_use]
    pub fn module_ids(&self) -> Vec<&ModuleId> {
        self.global_mods
            .iter()
            .map(|s| &s.id)
            .chain(self.voice_mods.iter().map(|s| &s.id))
            .chain(self.generators.iter().map(|s| &s.id))
            .chain(self.voice_fx.iter().map(|s| &s.id))
            .chain(self.fx.iter().map(|s| &s.id))
            .collect()
    }

    /// The routed (modulated) parameters, in the order the telemetry tap publishes their values
    /// ([`crate::tap::Tap`]): the voice table's routed parameters, then the global table's. Main thread: map them to
    /// addresses with [`PlanMap::addr`].
    #[must_use]
    pub fn tap_params(&self) -> &[ParamId] {
        &self.tap_params
    }

    /// The modulators, in the order the telemetry tap publishes their outputs: the voice modulators (patch order),
    /// then the global ones.
    #[must_use]
    pub fn tap_modulators(&self) -> Vec<&ModuleId> {
        self.voice_mods
            .iter()
            .map(|s| &s.id)
            .chain(self.global_mods.iter().map(|s| &s.id))
            .collect()
    }

    /// The parameter host slot `slot` (0-based) drives in this plan, if any (an empty slot, or a target in a disabled
    /// module, has none).
    #[must_use]
    pub fn slot_target(&self, slot: usize) -> Option<ParamId> {
        self.slots.get(slot).and_then(|e| e.target).map(|t| t.id)
    }

    /// When this plan swaps in, **forget** the remembered host value
    /// ([`Engine::host_slot`](crate::Engine::host_slot)) of every slot in `slots` (bit `i` = slot index `i`; bits
    /// accumulate) instead of re-applying it, so the plan's own patch value wins even where the slot keeps its
    /// target (F1b review; docs/session.md). The session passes `u64::MAX` for a plan compiled from a loaded patch,
    /// and the slots of the addresses a recompiling transaction sets, so the old host value neither carries into a
    /// loaded patch nor undoes an editor edit until the host's echo arrives. Host automation that moves afterwards
    /// applies and is remembered as usual. A plan an engine starts with has nothing to forget.
    pub fn forget_host_slots(&mut self, slots: u64) {
        self.forget_slots |= slots;
    }

    /// The slots this plan forgets when it swaps in ([`Plan::forget_host_slots`]).
    #[must_use]
    pub fn forgotten_host_slots(&self) -> u64 {
        self.forget_slots
    }

    /// A route's current depth, as a percentage (a depth event ramps it).
    #[must_use]
    pub fn route_depth(&self, id: DepthId) -> Option<f32> {
        self.matrix.depth(usize::from(id.0)).map(|d| d * 100.0)
    }

    /// The number of subnormal values in every module's state, the modulator outputs and the output ramps (test
    /// support).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        let voice: usize = self
            .generators
            .iter()
            .chain(&self.voice_fx)
            .map(|s| s.module.subnormals())
            .sum();
        let vmods: usize = self.voice_mods.iter().map(|s| s.module.subnormals()).sum();
        let gmods: usize = self.global_mods.iter().map(|s| s.module.subnormals()).sum();
        let fx: usize = self.fx.iter().map(|s| s.module.subnormals()).sum();
        let outs: usize = self
            .vmod_cur
            .iter()
            .chain(&self.vmod_prev)
            .map(|a| crate::module::count_subnormals(a))
            .sum::<usize>()
            + crate::module::count_subnormals(&self.gmod_cur)
            + crate::module::count_subnormals(&self.gmod_prev);
        voice + vmods + gmods + fx + outs + self.out.subnormals()
    }

    /// Reset every global module (engine start).
    pub(crate) fn reset_globals(&mut self, render_seed: u64) {
        let Plan {
            global_mods,
            fx,
            global_table,
            ..
        } = self;
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

    /// Back to the state a freshly compiled plan started by [`Engine::new`](crate::Engine::new) holds, keeping the
    /// base values ([`crate::Engine::reset`]): parameters settled on their bases, depth ramps landed, modulator
    /// outputs at 0, the output ramps at the current master level, every global module reset. Voice modules need
    /// nothing: every voice is freed, and `start_voice` resets a slot. Never allocates.
    pub(crate) fn reset_state(&mut self, render_seed: u64) {
        self.voice_table.settle();
        self.global_table.settle();
        self.matrix.settle_depths();
        for out in self.vmod_cur.iter_mut().chain(self.vmod_prev.iter_mut()) {
            *out = [0.0; MAX_VOICES];
        }
        self.gmod_cur.fill(0.0);
        self.gmod_prev.fill(0.0);
        let level = self.global_table.base[self.master_builtin.start + MasterParam::Level.index()];
        self.out = Output::new(level);
        self.reset_globals(render_seed);
    }

    /// A value from the command ring set parameter `id` to `value` (already applied to the table): a slot that
    /// targets it now holds that as its patch value, so the host's echo (`to_norm` of it) resolves exactly.
    pub(crate) fn note_patch_value(&mut self, id: ParamId, value: f32) {
        for entry in &mut self.slots {
            if let Some(t) = entry.target.as_mut()
                && t.id == id
                && !value.is_nan()
            {
                t.plain = t.spec.clamp(value);
                t.norm = t.spec.to_norm(t.plain);
            }
        }
    }

    /// Reset the `fx` slots whose kind is `time_based` (the monitor bypass engaged: they restart from silence).
    pub(crate) fn reset_time_based(&mut self, render_seed: u64) {
        let Plan {
            fx,
            fx_time_based,
            global_table,
            ..
        } = self;
        for (slot, &tb) in fx.iter_mut().zip(fx_time_based.iter()) {
            if tb {
                slot.module.reset(&GlobalStart {
                    seed: global_seed(render_seed, slot.key),
                    module_key: slot.key,
                    params: global_table.view(slot.params),
                });
            }
        }
    }

    /// A fresh voice in slot `v` (its notes already set): every voice module starts it with its seed, and the
    /// slot's modulator outputs restart from 0 (the amp stage ramps up from silence).
    pub(crate) fn start_voice(&mut self, v: VoiceIdx, notes: &NoteState, render_seed: u64) {
        for out in &mut self.vmod_cur {
            out[v] = 0.0;
        }
        let Plan {
            generators,
            voice_fx,
            voice_mods,
            voice_table,
            ..
        } = self;
        for slot in generators.iter_mut().chain(voice_fx.iter_mut()) {
            slot.module
                .start_voice(&voice_start(slot, v, notes, voice_table, render_seed));
        }
        for slot in voice_mods.iter_mut() {
            slot.module
                .start_voice(&voice_start(slot, v, notes, voice_table, render_seed));
        }
    }

    /// Carry the running state of `old` (the plan being replaced) into this one (see the module docs). Called on the
    /// audio thread; never allocates. Returns whether a module was added or removed (the swap then crossfades).
    ///
    /// Whether the swap crossfades is decided first (pairing the five role lists is cheap). Without a crossfade the
    /// old plan never runs again, so each surviving effect *takes* its state ([`GlobalProcessorImpl::take_state`]:
    /// the reverb moves its delay lines instead of copying 1.6 MB); with one, both plans run for 256 frames and the
    /// state is copied. Either way the new plan ends up with the same state, so the output is the same bits.
    pub(crate) fn carry_from(
        &mut self,
        old: &mut Plan,
        active: &[VoiceIdx],
        notes: &NoteState,
        render_seed: u64,
    ) -> bool {
        let changed = pair(&self.generators, &old.generators, VoiceProcessorImpl::kind).1
            || pair(&self.voice_fx, &old.voice_fx, VoiceProcessorImpl::kind).1
            || pair(&self.voice_mods, &old.voice_mods, VoiceModulatorImpl::kind).1
            || pair(
                &self.global_mods,
                &old.global_mods,
                GlobalModulatorImpl::kind,
            )
            .1
            || pair(&self.fx, &old.fx, GlobalProcessorImpl::kind).1;
        let move_fx = !changed;
        let Plan {
            generators,
            voice_fx,
            voice_mods,
            global_mods,
            fx,
            voice_table,
            global_table,
            vmod_cur,
            vmod_prev,
            gmod_cur,
            gmod_prev,
            ..
        } = self;

        for (list, old_list) in [
            (&mut *generators, &old.generators),
            (&mut *voice_fx, &old.voice_fx),
        ] {
            let (pairs, _) = pair(list, old_list, VoiceProcessorImpl::kind);
            for (i, slot) in list.iter_mut().enumerate() {
                if let Some(j) = pairs[i] {
                    let o = &old_list[j];
                    slot.module.copy_state(&o.module);
                    voice_table.carry(slot.params, &old.voice_table, o.params);
                } else {
                    for &v in active {
                        slot.module.start_voice(&voice_start(
                            slot,
                            v,
                            notes,
                            voice_table,
                            render_seed,
                        ));
                    }
                }
            }
        }

        let (pairs, _) = pair(voice_mods, &old.voice_mods, VoiceModulatorImpl::kind);
        for (i, slot) in voice_mods.iter_mut().enumerate() {
            if let Some(j) = pairs[i] {
                let o = &old.voice_mods[j];
                slot.module.copy_state(&o.module);
                voice_table.carry(slot.params, &old.voice_table, o.params);
                vmod_cur[i] = old.vmod_cur[j];
                vmod_prev[i] = old.vmod_prev[j];
            } else {
                for &v in active {
                    slot.module
                        .start_voice(&voice_start(slot, v, notes, voice_table, render_seed));
                }
            }
        }

        let (pairs, _) = pair(global_mods, &old.global_mods, GlobalModulatorImpl::kind);
        for (i, slot) in global_mods.iter_mut().enumerate() {
            if let Some(j) = pairs[i] {
                let o = &old.global_mods[j];
                slot.module.copy_state(&o.module);
                global_table.carry(slot.params, &old.global_table, o.params);
                gmod_cur[i] = old.gmod_cur[j];
                gmod_prev[i] = old.gmod_prev[j];
            } else {
                slot.module.reset(&GlobalStart {
                    seed: global_seed(render_seed, slot.key),
                    module_key: slot.key,
                    params: global_table.view(slot.params),
                });
            }
        }

        let (pairs, _) = pair(fx, &old.fx, GlobalProcessorImpl::kind);
        for (i, slot) in fx.iter_mut().enumerate() {
            if let Some(j) = pairs[i] {
                let o_params = old.fx[j].params;
                if move_fx {
                    slot.module.take_state(&mut old.fx[j].module);
                } else {
                    slot.module.copy_state(&old.fx[j].module);
                }
                global_table.carry(slot.params, &old.global_table, o_params);
            } else {
                slot.module.reset(&GlobalStart {
                    seed: global_seed(render_seed, slot.key),
                    module_key: slot.key,
                    params: global_table.view(slot.params),
                });
            }
        }

        voice_table.carry(self.voice_builtin, &old.voice_table, old.voice_builtin);
        global_table.carry(self.master_builtin, &old.global_table, old.master_builtin);
        global_table.carry(self.macros, &old.global_table, old.macros);
        self.out = old.out;
        changed
    }
}

/// The most modules in one role list (the modulators section's limit).
const MAX_ROLE_SLOTS: usize = 8;

/// Match the slots of a new role list to the old one by (id, kind): `pairs[i]` is the old index of new slot `i`.
/// The flag says whether a module was added or removed.
fn pair<M>(
    new: &[Slot<M>],
    old: &[Slot<M>],
    kind: impl Fn(&M) -> ModuleKind,
) -> ([Option<usize>; MAX_ROLE_SLOTS], bool) {
    let mut pairs = [None; MAX_ROLE_SLOTS];
    let mut used = [false; MAX_ROLE_SLOTS];
    let mut changed = new.len() > MAX_ROLE_SLOTS || old.len() > MAX_ROLE_SLOTS;
    for (i, s) in new.iter().enumerate().take(MAX_ROLE_SLOTS) {
        let found = old
            .iter()
            .enumerate()
            .take(MAX_ROLE_SLOTS)
            .position(|(j, o)| {
                !used[j] && o.key == s.key && o.id == s.id && kind(&o.module) == kind(&s.module)
            });
        match found {
            Some(j) => {
                used[j] = true;
                pairs[i] = Some(j);
            }
            None => changed = true,
        }
    }
    if used.iter().take(old.len().min(MAX_ROLE_SLOTS)).any(|u| !u) {
        changed = true;
    }
    (pairs, changed)
}

fn voice_start<'a, M>(
    slot: &Slot<M>,
    v: VoiceIdx,
    notes: &'a NoteState,
    voice_table: &'a VoiceTable,
    render_seed: u64,
) -> VoiceStart<'a> {
    let note_index = notes.note_index[v];
    VoiceStart {
        voice: v,
        seed: voice_seed(render_seed, slot.key, note_index),
        module_key: slot.key,
        note_index,
        notes,
        params: voice_table.view(slot.params),
    }
}

/// Parameter addresses → the plan's parameter ids. Built with the plan; used off the audio thread to turn an op or
/// an automation point into an [`EventKind::Param`](crate::event::EventKind::Param) (or a
/// [`Link::send_param`](crate::swap::Link::send_param) with the plan's epoch), and a route into a [`DepthId`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanMap {
    params: BTreeMap<ParamAddr, ParamId>,
    routes: BTreeMap<RouteAddr, DepthId>,
}

impl PlanMap {
    /// The id of the parameter at `addr`, if it is in the plan (a disabled module's parameters are not).
    #[must_use]
    pub fn get(&self, addr: &ParamAddr) -> Option<ParamId> {
        self.params.get(addr).copied()
    }

    /// The address of parameter `id` (a linear search; main thread, e.g. to label the telemetry tap's values).
    #[must_use]
    pub fn addr(&self, id: ParamId) -> Option<&ParamAddr> {
        self.params.iter().find(|(_, i)| **i == id).map(|(a, _)| a)
    }

    /// The depth of route `route` in the plan, for [`Link::send_depth`](crate::swap::Link::send_depth) (P1b lane
    /// E+). `None` when the plan has no term for it: a route with depth 0 (left out of the plan), or from or to a
    /// disabled module. A depth change from or to 0 is therefore a recompile, which keeps settled values exact (a
    /// routed parameter is always `from_norm(n)`).
    #[must_use]
    pub fn depth(&self, route: &RouteAddr) -> Option<DepthId> {
        self.routes.get(route).copied()
    }

    /// Every address in the plan with its id, in address order.
    pub fn iter(&self) -> impl Iterator<Item = (&ParamAddr, ParamId)> {
        self.params.iter().map(|(a, &id)| (a, id))
    }

    /// The number of parameters in the plan.
    #[must_use]
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Whether the plan has no parameters (never: `voice.*` and `master.*` are always there).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }
}

/// Why a patch cannot be compiled. Codes are stable snake case.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompileError {
    /// The sample rate is outside 8–384 kHz or not finite.
    UnsupportedSampleRate(f32),
    /// A module's value list does not match its kind's parameter count.
    WrongValueCount {
        /// Module id (or `voice`/`master`).
        module: String,
        /// Parameters the kind has.
        expected: usize,
        /// Values the patch holds.
        found: usize,
    },
    /// A value is not valid for its spec (non-finite, out of range, not integral, or `-0.0`).
    InvalidValue {
        /// The address.
        addr: String,
        /// The value.
        value: f32,
    },
    /// Two modules share an id.
    DuplicateId(String),
    /// A module sits in a section its kind does not belong to.
    WrongSection {
        /// Module id.
        module: String,
        /// Its kind.
        kind: ModuleKind,
        /// Where it was found.
        section: Section,
    },
    /// A section holds more modules than its limit.
    TooMany {
        /// The section.
        section: Section,
        /// Its limit.
        max: usize,
    },
    /// `voice.amp_env` does not name a per-voice `adsr` in `modulators`.
    BadAmpEnv(String),
    /// A feature a later lane builds (the message names the lane). No longer returned since S2b; kept for
    /// compatibility.
    NotYet {
        /// What is missing.
        feature: &'static str,
        /// The lane that builds it.
        lane: &'static str,
    },
    /// A modulation route the engine cannot run (lane S2b): an unknown or non-modulator source, an unknown or
    /// non-modulatable destination, a depth outside ±100 %, or a voice-rate source on a global destination.
    BadRoute {
        /// The route, as text (`velocity -> osc1.level`).
        route: String,
        /// Why (stable wording: `unknown_source`, `not_a_modulator`, `unknown_destination`, `not_modulatable`,
        /// `depth_out_of_range`, `route_rate_mismatch`).
        reason: &'static str,
    },
}

impl CompileError {
    /// A stable snake-case code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            CompileError::UnsupportedSampleRate(_) => "unsupported_sample_rate",
            CompileError::WrongValueCount { .. } => "wrong_value_count",
            CompileError::InvalidValue { .. } => "invalid_value",
            CompileError::DuplicateId(_) => "duplicate_id",
            CompileError::WrongSection { .. } => "wrong_section",
            CompileError::TooMany { .. } => "too_many",
            CompileError::BadAmpEnv(_) => "bad_amp_env",
            CompileError::NotYet { .. } => "not_yet",
            CompileError::BadRoute { .. } => "bad_route",
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::UnsupportedSampleRate(sr) => {
                write!(f, "sample rate {sr} Hz is outside 8000..=384000")
            }
            CompileError::WrongValueCount {
                module,
                expected,
                found,
            } => write!(
                f,
                "{module} has {found} values; its kind has {expected} parameters"
            ),
            CompileError::InvalidValue { addr, value } => {
                write!(f, "{addr} = {value:e} is not a valid value")
            }
            CompileError::DuplicateId(id) => write!(f, "module id '{id}' is used twice"),
            CompileError::WrongSection {
                module,
                kind,
                section,
            } => write!(f, "{module} is a {kind} and cannot sit in {section}"),
            CompileError::TooMany { section, max } => {
                write!(f, "{section} holds more than {max} modules")
            }
            CompileError::BadAmpEnv(id) => write!(
                f,
                "voice.amp_env = '{id}' must name a per-voice adsr in modulators"
            ),
            CompileError::NotYet { feature, lane } => write!(
                f,
                "{feature} are not implemented in the engine yet (lane {lane})"
            ),
            CompileError::BadRoute { route, reason } => {
                write!(f, "route '{route}' cannot run: {reason}")
            }
        }
    }
}

impl std::error::Error for CompileError {}

fn check_values(
    name: &str,
    specs: &'static [enth_params::ParamSpec],
    values: &[f32],
) -> Result<(), CompileError> {
    if values.len() != specs.len() {
        return Err(CompileError::WrongValueCount {
            module: name.to_owned(),
            expected: specs.len(),
            found: values.len(),
        });
    }
    for (spec, &value) in specs.iter().zip(values) {
        if !spec.is_valid(value) {
            return Err(CompileError::InvalidValue {
                addr: format!("{name}.{}", spec.key),
                value,
            });
        }
    }
    Ok(())
}

/// Compile a patch for `cfg`. Allocates (modules are prepared here); the result runs without allocating.
///
/// # Errors
/// An invalid patch (the checks are a subset of S1b's `validate`, enough to never hand the audio thread a bad
/// value), an unsupported sample rate, or a route the engine cannot run ([`CompileError::BadRoute`]).
pub fn compile(patch: &Patch, cfg: &PrepareCfg) -> Result<(Plan, PlanMap), CompileError> {
    if !cfg.is_supported() {
        return Err(CompileError::UnsupportedSampleRate(cfg.sample_rate));
    }
    check_values("voice", &VOICE_PARAMS, &patch.voice.values)?;
    check_values("master", &MASTER_PARAMS, &patch.master)?;
    for (i, m) in patch.macros.iter().enumerate() {
        if !MACRO_VALUE.is_valid(m.value) {
            return Err(CompileError::InvalidValue {
                addr: format!("macro{}", i + 1),
                value: m.value,
            });
        }
    }
    let mode = VoiceMode::from_value(patch.voice.get(VoiceParam::Mode)).unwrap_or(VoiceMode::Poly);
    let mut ids: Vec<&str> = Vec::new();
    for section in Section::ALL {
        let modules = patch.section(section);
        if modules.len() > section.max_len() {
            return Err(CompileError::TooMany {
                section,
                max: section.max_len(),
            });
        }
        for m in modules {
            if m.kind.section() != section {
                return Err(CompileError::WrongSection {
                    module: m.id.to_string(),
                    kind: m.kind,
                    section,
                });
            }
            if ids.contains(&m.id.as_str()) {
                return Err(CompileError::DuplicateId(m.id.to_string()));
            }
            ids.push(m.id.as_str());
            check_values(m.id.as_str(), m.kind.params(), &m.values)?;
        }
    }
    let amp_env_id = &patch.voice.amp_env;
    let amp_env_ok = patch
        .modulators
        .iter()
        .any(|m| &m.id == amp_env_id && m.kind == ModuleKind::Adsr && m.rate() == Rate::Voice);
    if !amp_env_ok {
        return Err(CompileError::BadAmpEnv(amp_env_id.to_string()));
    }

    let mut voice_table = VoiceTable::new(cfg.sample_rate);
    let mut global_table = GlobalTable::new(cfg.sample_rate);
    let mut map = PlanMap::default();
    let voice_builtin = voice_table.push(&VOICE_PARAMS, &patch.voice.values);
    for p in VoiceParam::ALL {
        map.params.insert(
            ParamAddr::voice(*p),
            ParamId::Voice((voice_builtin.start + p.index()) as u16),
        );
    }
    let master_builtin = global_table.push(&MASTER_PARAMS, &patch.master);
    for p in MasterParam::ALL {
        map.params.insert(
            ParamAddr::master(*p),
            ParamId::Global((master_builtin.start + p.index()) as u16),
        );
    }
    let mut macro_values = [0.0; MACRO_SOURCES];
    for (v, m) in macro_values.iter_mut().zip(&patch.macros) {
        *v = m.value;
    }
    let macros = global_table.push(&MACRO_SPECS, &macro_values);

    let master_db = patch.master[MasterParam::Level.index()];
    let mut plan = Plan {
        cfg: *cfg,
        epoch: 0,
        generators: Vec::new(),
        voice_fx: Vec::new(),
        voice_mods: Vec::new(),
        global_mods: Vec::new(),
        fx: Vec::new(),
        fx_time_based: Vec::new(),
        amp_env: 0,
        sync_master: None,
        voice_table: VoiceTable::new(cfg.sample_rate),
        global_table: GlobalTable::new(cfg.sample_rate),
        voice_builtin,
        master_builtin,
        macros,
        mode,
        polyphony: patch.voice.get(VoiceParam::Polyphony) as usize,
        matrix: Matrix::default(),
        vmod_cur: Vec::new(),
        vmod_prev: Vec::new(),
        gmod_cur: Vec::new(),
        gmod_prev: Vec::new(),
        out: Output::new(master_db),
        slots: [SlotEntry::default(); SLOTS],
        tap_params: Vec::new(),
        forget_slots: 0,
    };

    for section in Section::ALL {
        for m in patch.section(section) {
            let is_amp_env = section == Section::Modulators && &m.id == amp_env_id;
            if !m.enabled && !is_amp_env {
                continue;
            }
            add_module(
                &mut plan,
                &mut voice_table,
                &mut global_table,
                &mut map,
                section,
                m,
            );
            if is_amp_env {
                plan.amp_env = plan.voice_mods.len() - 1;
            }
        }
    }
    plan.sync_master = plan
        .generators
        .iter()
        .position(|g| matches!(g.module, VoiceProcessorImpl::Osc(_)));
    plan.fx_time_based = plan
        .fx
        .iter()
        .map(|s| s.module.kind().spec().time_based)
        .collect();
    plan.matrix = compile_routes(patch, &plan, &map, &mut voice_table, &mut global_table)?;
    for (term, route) in plan.matrix.tags() {
        let r = &patch.routes[route];
        let addr = RouteAddr {
            src: r.src.clone(),
            dst: r.dst.clone(),
        };
        map.routes.insert(addr, DepthId(term as u16));
    }
    plan.tap_params = voice_table
        .routed
        .iter()
        .map(|&p| ParamId::Voice(p as u16))
        .chain(
            global_table
                .routed
                .iter()
                .map(|&p| ParamId::Global(p as u16)),
        )
        .collect();
    plan.slots = compile_slots(patch, &map);
    plan.voice_table = voice_table;
    plan.global_table = global_table;
    plan.vmod_cur = vec![[0.0; MAX_VOICES]; plan.voice_mods.len()];
    plan.vmod_prev = vec![[0.0; MAX_VOICES]; plan.voice_mods.len()];
    plan.gmod_cur = vec![0.0; plan.global_mods.len()];
    plan.gmod_prev = vec![0.0; plan.global_mods.len()];
    Ok((plan, map))
}

/// The slot table: each of `patch.slots` resolved against the plan's parameters. A target that is not in the plan
/// (a disabled module) or not automatable (a structural parameter: a patch that skipped `validate`) drives nothing.
fn compile_slots(patch: &Patch, map: &PlanMap) -> [SlotEntry; SLOTS] {
    let mut slots = [SlotEntry::default(); SLOTS];
    for (entry, addr) in slots.iter_mut().zip(&patch.slots) {
        let Some(addr) = addr else { continue };
        entry.key = enth_dsp::util::rng::fnv1a64(addr.to_string().as_bytes()).max(1);
        let (Some(id), Some(spec), Some(plain)) =
            (map.get(addr), patch.spec(addr), patch.value(addr))
        else {
            continue;
        };
        if !spec.flags.automatable || spec.flags.structural {
            continue;
        }
        entry.target = Some(SlotTarget {
            id,
            spec,
            plain,
            norm: spec.to_norm(plain),
        });
    }
    slots
}

/// Resolve the patch's routes against the plan, in patch order, into the matrix (and mark the routed parameters).
fn compile_routes(
    patch: &Patch,
    plan: &Plan,
    map: &PlanMap,
    voice_table: &mut VoiceTable,
    global_table: &mut GlobalTable,
) -> Result<Matrix, CompileError> {
    let mut voice_stage = vec![Stage::Processors; voice_table.base.len()];
    for (i, s) in plan.voice_mods.iter().enumerate() {
        for p in s.params.range() {
            voice_stage[p] = Stage::VoiceMod(i as u16);
        }
    }
    let mut global_stage = vec![Stage::Processors; global_table.base.len()];
    for (i, s) in plan.global_mods.iter().enumerate() {
        for p in s.params.range() {
            global_stage[p] = Stage::GlobalMod(i as u16);
        }
    }
    let mut builder = MatrixBuilder::default();
    for (route_index, r) in patch.routes.iter().enumerate() {
        let bad = |reason| CompileError::BadRoute {
            route: format!("{} -> {}", r.src, r.dst),
            reason,
        };
        if !r.depth.is_finite() || r.depth.abs() > 100.0 {
            return Err(bad("depth_out_of_range"));
        }
        let src = match &r.src {
            ModSource::Velocity => Some(Src::Velocity),
            ModSource::Key => Some(Src::Key),
            ModSource::ModWheel => Some(Src::ModWheel),
            ModSource::Macro(i) => Some(Src::Macro(i.get() as u8)),
            ModSource::Module(id) => {
                let Some((_, m)) = patch.find(id) else {
                    return Err(bad("unknown_source"));
                };
                if m.kind.section() != Section::Modulators {
                    return Err(bad("not_a_modulator"));
                }
                // A disabled modulator is not in the plan: the route contributes nothing.
                plan.voice_mods
                    .iter()
                    .position(|s| &s.id == id)
                    .map(|i| Src::VoiceMod(i as u16))
                    .or_else(|| {
                        plan.global_mods
                            .iter()
                            .position(|s| &s.id == id)
                            .map(|i| Src::GlobalMod(i as u16))
                    })
            }
        };
        let Some(spec) = patch.spec(&r.dst) else {
            return Err(bad("unknown_destination"));
        };
        if !spec.flags.modulatable {
            return Err(bad("not_modulatable"));
        }
        if patch.source_rate(&r.src) == Some(Rate::Voice)
            && patch.target_rate(r.dst.target()) == Some(Rate::Global)
        {
            return Err(bad("route_rate_mismatch"));
        }
        let (Some(src), Some(dst)) = (src, map.get(&r.dst)) else {
            continue; // from or to a disabled module
        };
        if r.depth == 0.0 {
            continue;
        }
        let term = Term {
            src,
            depth: r.depth / 100.0,
        };
        match dst {
            ParamId::Voice(i) => {
                let i = usize::from(i);
                builder.add_tagged(voice_stage[i], true, i, term, route_index);
            }
            ParamId::Global(i) => {
                if src.is_voice_rate() {
                    return Err(bad("route_rate_mismatch"));
                }
                let i = usize::from(i);
                builder.add_tagged(global_stage[i], false, i, term, route_index);
            }
        }
    }
    Ok(builder.build(
        plan.voice_mods.len(),
        plan.global_mods.len(),
        voice_table,
        global_table,
    ))
}

fn add_module(
    plan: &mut Plan,
    voice_table: &mut VoiceTable,
    global_table: &mut GlobalTable,
    map: &mut PlanMap,
    section: Section,
    m: &ModuleInstance,
) {
    let key = module_key(&m.id);
    let specs = m.kind.params();
    let module = ModuleImpl::new(m.kind, &m.values, &plan.cfg);
    let (params, voice_rate) = match &module {
        ModuleImpl::VoiceProcessor(_) | ModuleImpl::VoiceModulator(_) => {
            (voice_table.push(specs, &m.values), true)
        }
        ModuleImpl::GlobalModulator(_) | ModuleImpl::GlobalProcessor(_) => {
            (global_table.push(specs, &m.values), false)
        }
    };
    for (i, spec) in specs.iter().enumerate() {
        if let Ok(addr) = ParamAddr::module(m.id.clone(), spec.key) {
            let index = (params.start + i) as u16;
            let id = if voice_rate {
                ParamId::Voice(index)
            } else {
                ParamId::Global(index)
            };
            map.params.insert(addr, id);
        }
    }
    let id = m.id.clone();
    match module {
        ModuleImpl::VoiceProcessor(module) => {
            let slot = Slot {
                module,
                id,
                key,
                params,
            };
            if section == Section::Generators {
                plan.generators.push(slot);
            } else {
                plan.voice_fx.push(slot);
            }
        }
        ModuleImpl::VoiceModulator(module) => plan.voice_mods.push(Slot {
            module,
            id,
            key,
            params,
        }),
        ModuleImpl::GlobalModulator(module) => plan.global_mods.push(Slot {
            module,
            id,
            key,
            params,
        }),
        ModuleImpl::GlobalProcessor(module) => plan.fx.push(Slot {
            module,
            id,
            key,
            params,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Route;
    use enth_patch::catalogue::{AdsrParam, LfoParam, LfoScope, OscParam};

    fn cfg() -> PrepareCfg {
        PrepareCfg::new(48_000.0)
    }

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    fn full_patch() -> Patch {
        let mut p = Patch::init();
        p.generators
            .push(ModuleInstance::new(id("noise1"), ModuleKind::Noise));
        p.voice_fx
            .push(ModuleInstance::new(id("filter1"), ModuleKind::Svf));
        let mut lfo2 = ModuleInstance::new(id("lfo2"), ModuleKind::Lfo);
        lfo2.values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
        p.modulators
            .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
        p.modulators.push(lfo2);
        p.fx.push(ModuleInstance::new(id("comp1"), ModuleKind::Comp));
        p.fx.push(ModuleInstance::new(id("reverb1"), ModuleKind::Reverb));
        p
    }

    fn route(src: &str, dst: &str, depth: f32) -> Route {
        Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        }
    }

    #[test]
    fn modules_land_in_their_role_lists_in_patch_order() {
        let (plan, map) = compile(&full_patch(), &cfg()).unwrap();
        let ids: Vec<&str> = plan.module_ids().iter().map(|i| i.as_str()).collect();
        assert_eq!(
            ids,
            [
                "lfo2", "env1", "lfo1", "osc1", "noise1", "filter1", "comp1", "reverb1"
            ]
        );
        assert_eq!(plan.voice_mods[plan.amp_env].id.as_str(), "env1");
        assert_eq!(plan.polyphony(), 8);
        assert_eq!(plan.mode(), VoiceMode::Poly);
        assert_eq!(plan.epoch(), 0);
        assert_eq!(plan.sync_master, Some(0), "osc1 drives hard sync");
        assert_eq!(
            plan.fx_time_based,
            [false, true],
            "the reverb is time-based"
        );
        // Every parameter is addressable, in the table its owner's rate implies.
        assert!(matches!(
            map.get(&"filter1.cutoff".parse().unwrap()),
            Some(ParamId::Voice(_))
        ));
        assert!(matches!(
            map.get(&"lfo2.rate".parse().unwrap()),
            Some(ParamId::Global(_))
        ));
        assert!(matches!(
            map.get(&"reverb1.wet".parse().unwrap()),
            Some(ParamId::Global(_))
        ));
        assert!(matches!(
            map.get(&"voice.pan".parse().unwrap()),
            Some(ParamId::Voice(_))
        ));
        assert!(matches!(
            map.get(&"master.level".parse().unwrap()),
            Some(ParamId::Global(_))
        ));
        let expected: usize = full_patch()
            .modules()
            .map(|(_, m)| m.values.len())
            .sum::<usize>()
            + VoiceParam::COUNT
            + MasterParam::COUNT;
        assert_eq!(map.len(), expected);
        // The macros and the mod wheel sit after master.*, not in the map.
        assert_eq!(plan.macros.len, MACRO_SOURCES);
        assert_eq!(
            plan.macros.start,
            plan.master_builtin.start + MasterParam::COUNT
        );
    }

    #[test]
    fn disabled_modules_are_left_out_but_the_amp_env_always_runs() {
        let mut p = full_patch();
        p.generators[1].enabled = false;
        p.modulators[0].enabled = false; // env1, the amp env
        let (plan, map) = compile(&p, &cfg()).unwrap();
        let ids: Vec<&str> = plan.module_ids().iter().map(|i| i.as_str()).collect();
        assert!(!ids.contains(&"noise1"));
        // The first *enabled* osc is the sync master; with osc1 disabled there is none here.
        let mut q = full_patch();
        q.generators[0].enabled = false;
        assert_eq!(compile(&q, &cfg()).unwrap().0.sync_master, None);
        q.generators
            .insert(0, ModuleInstance::new(id("osc9"), ModuleKind::Osc));
        q.generators[0].enabled = true;
        let (plan2, _) = compile(&q, &cfg()).unwrap();
        assert_eq!(
            plan2.generators[plan2.sync_master.unwrap()].id.as_str(),
            "osc9"
        );
        assert!(ids.contains(&"env1"));
        assert_eq!(map.get(&"noise1.level".parse().unwrap()), None);
    }

    #[test]
    fn routes_and_voice_modes_compile() {
        let mut p = full_patch();
        p.routes.push(route("velocity", "osc1.level", 50.0));
        p.routes.push(route("lfo1", "filter1.cutoff", -20.0));
        p.routes.push(route("lfo2", "reverb1.wet", 10.0));
        p.routes.push(route("macro3", "lfo1.rate", 100.0));
        p.routes.push(route("modwheel", "voice.pan", 100.0));
        p.routes.push(route("key", "env1.sustain", 0.0)); // depth 0: left out
        p.voice.values[VoiceParam::Mode.index()] = VoiceMode::Legato.index() as f32;
        let (plan, map) = compile(&p, &cfg()).unwrap();
        assert_eq!(plan.mode(), VoiceMode::Legato);
        let routed = |a: &str| match map.get(&a.parse().unwrap()).unwrap() {
            ParamId::Voice(i) => plan.voice_table.modulated[usize::from(i)],
            ParamId::Global(i) => plan.global_table.modulated[usize::from(i)],
        };
        for a in [
            "osc1.level",
            "filter1.cutoff",
            "reverb1.wet",
            "lfo1.rate",
            "voice.pan",
        ] {
            assert!(routed(a), "{a}");
        }
        assert!(!routed("env1.sustain"), "a depth-0 route is left out");
        assert!(!routed("osc1.semi"));
        assert!(!plan.matrix.is_empty());
    }

    #[test]
    fn routes_the_engine_cannot_run_are_errors() {
        let check = |r: Route, reason: &str| {
            let mut p = full_patch();
            p.routes.push(r);
            let e = compile(&p, &cfg()).unwrap_err();
            assert_eq!(e.code(), "bad_route");
            assert!(
                matches!(&e, CompileError::BadRoute { reason: got, .. } if *got == reason),
                "{e}"
            );
        };
        check(
            route("velocity", "reverb1.wet", 10.0),
            "route_rate_mismatch",
        );
        check(route("lfo2", "master.level", 10.0), "not_modulatable");
        check(route("lfo2", "filter9.cutoff", 10.0), "unknown_destination");
        check(route("env1", "comp1.mix", 10.0), "route_rate_mismatch");
        check(route("lfo9", "osc1.level", 10.0), "unknown_source");
        check(route("osc1", "osc1.level", 10.0), "not_a_modulator");
        check(route("lfo1", "osc1.wave", 10.0), "not_modulatable");
        check(route("lfo1", "osc1.level", 101.0), "depth_out_of_range");
        check(route("lfo1", "osc1.level", f32::NAN), "depth_out_of_range");
        // From or to a disabled module: nothing, and no error.
        let mut p = full_patch();
        p.modulators[1].enabled = false; // lfo1
        p.routes.push(route("lfo1", "osc1.level", 10.0));
        p.generators[1].enabled = false; // noise1
        p.routes.push(route("lfo2", "noise1.level", 10.0));
        let (plan, _) = compile(&p, &cfg()).unwrap();
        assert!(plan.matrix.is_empty());
    }

    #[test]
    fn invalid_patches_are_errors_not_panics() {
        let mut p = Patch::init();
        p.modulators[0].values[AdsrParam::Attack.index()] = f32::NAN;
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "invalid_value");

        let mut p = Patch::init();
        p.generators[0].values.pop();
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "wrong_value_count");

        let mut p = Patch::init();
        p.voice.amp_env = id("env9");
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "bad_amp_env");

        let mut p = Patch::init();
        p.generators
            .push(ModuleInstance::new(id("env1"), ModuleKind::Osc));
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "duplicate_id");

        let mut p = Patch::init();
        p.fx.push(ModuleInstance::new(id("osc2"), ModuleKind::Osc));
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "wrong_section");

        let mut p = Patch::init();
        for i in 2..=5 {
            p.generators
                .push(ModuleInstance::new(id(&format!("osc{i}")), ModuleKind::Osc));
        }
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "too_many");

        let mut p = Patch::init();
        p.macros[2].value = 1.5;
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "invalid_value");

        assert_eq!(
            compile(&Patch::init(), &PrepareCfg::new(1000.0))
                .unwrap_err()
                .code(),
            "unsupported_sample_rate"
        );
    }

    #[test]
    fn state_carries_by_id_and_kind() {
        let base = full_patch();
        let (mut a, _) = compile(&base, &cfg()).unwrap();
        a.voice_table.cur[a.voice_builtin.start + VoiceParam::Pan.index()][3] = 0.25;
        a.vmod_cur[a.amp_env][3] = 0.5;
        let notes = NoteState::new();
        // The same patch: nothing added or removed.
        let (mut b, _) = compile(&base, &cfg()).unwrap();
        assert!(!b.carry_from(&mut a, &[3], &notes, 1));
        assert_eq!(b.vmod_cur[b.amp_env][3], 0.5);
        assert_eq!(
            b.voice_table.cur[b.voice_builtin.start + VoiceParam::Pan.index()][3],
            0.25
        );
        // An added LFO, a removed noise, a moved osc: changed, and the survivors still carry.
        let mut edited = base.clone();
        edited.generators.remove(1);
        edited
            .modulators
            .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
        edited.generators[0].values[OscParam::Semi.index()] = 7.0;
        let (mut c, _) = compile(&edited, &cfg()).unwrap();
        assert!(c.carry_from(&mut a, &[3], &notes, 1));
        assert_eq!(c.vmod_cur[c.amp_env][3], 0.5);
        // A scope change moves an LFO between role lists: removed and added.
        let mut scoped = base.clone();
        scoped.modulators[1].values[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
        let (mut d, _) = compile(&scoped, &cfg()).unwrap();
        assert!(d.carry_from(&mut a, &[3], &notes, 1));
    }

    /// Feed the plan's reverb a burst so its delay lines hold a tail.
    fn excite_reverb(plan: &mut Plan) -> usize {
        let i = plan
            .fx
            .iter()
            .position(|s| s.module.kind() == ModuleKind::Reverb)
            .unwrap();
        let Plan {
            fx, global_table, ..
        } = plan;
        for k in 0..20 {
            let mut left = [0.0f32; crate::module::SEG];
            let mut right = [0.0f32; crate::module::SEG];
            if k < 4 {
                left[3] = 0.5;
                right[17] = -0.25;
            }
            let span = fx[i].params;
            fx[i].module.process(&mut crate::module::GlobalCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                params: global_table.view(span),
                left: &mut left,
                right: &mut right,
            });
        }
        i
    }

    fn reverb_is_empty(plan: &Plan, i: usize) -> bool {
        match &plan.fx[i].module {
            GlobalProcessorImpl::Reverb(r) => r.core().is_state_zero(),
            _ => unreachable!("index of the reverb"),
        }
    }

    #[test]
    fn an_instant_swap_moves_the_effect_state_and_a_crossfade_copies_it() {
        let base = full_patch();
        let notes = NoteState::new();
        let (mut a, _) = compile(&base, &cfg()).unwrap();
        let i = excite_reverb(&mut a);
        assert!(!reverb_is_empty(&a, i));
        // Instant (nothing added or removed): the new plan takes the lines, the old one is left with the new plan's
        // fresh (zeroed) lines, which the main thread drops with it.
        let (mut b, _) = compile(&base, &cfg()).unwrap();
        assert!(!b.carry_from(&mut a, &[], &notes, 1));
        assert!(!reverb_is_empty(&b, i), "the tail moved to the new plan");
        assert!(reverb_is_empty(&a, i), "moved, not copied");
        // A crossfade runs both plans: the state is copied and the old plan keeps its own.
        let i = excite_reverb(&mut a);
        let mut edited = base.clone();
        edited
            .modulators
            .push(ModuleInstance::new(id("lfo3"), ModuleKind::Lfo));
        let (mut c, _) = compile(&edited, &cfg()).unwrap();
        assert!(c.carry_from(&mut a, &[], &notes, 1));
        assert!(!reverb_is_empty(&c, i));
        assert!(!reverb_is_empty(&a, i), "copied: the old plan still runs");
    }
}
