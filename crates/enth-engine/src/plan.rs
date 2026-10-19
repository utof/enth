//! `Plan`: the compiled form of a `Patch` that the audio thread runs, and [`compile`], the minimal compiler.
//!
//! A plan holds every enabled module prepared ([`ModuleImpl::new`]) and sorted by role into ordered lists, the flat
//! parameter tables ([`crate::params`]), each module's stable key (`fnv1a64` of its id, for seeds and for the state
//! carry of the plan swap), and the modulator output slots. P1's topology is fixed (four sections), so the "graph"
//! is just ordered lists; there is no general scheduling. Compiling allocates; running a plan never does.
//!
//! [`PlanMap`] maps parameter addresses to the plan's [`ParamId`]s. It stays on the main thread (or in the offline
//! renderer), where an op or an automation point becomes a value event.
//!
//! **S2a's compiler is minimal on purpose.** It refuses what lane S2b builds, rather than ignoring it and rendering
//! something else: modulation routes ([`CompileError::NotYet`]) and the mono and legato voice modes. Seams:
//!
//! - TODO(S2b): compile routes into `(source slot, destination ParamId, depth / 100)` lists in patch route order,
//!   sorted by destination; macros and the mod wheel as global sources, velocity and key as voice sources.
//! - TODO(S2b): mono/legato (the voice manager), glide (`voice.glide`, `voice.glide_mode`).
//! - TODO(S2b): `Plan::carry_state_from(&old)`: for each module whose (key, kind) survives, `copy_state_from`; fresh
//!   modules start their active voices (`start_voice`) or `reset`; the epoch tag on the plan; the 256-frame
//!   crossfade for added or removed modules.
//!
//! Disabled modules are left out (a disabled generator is silent, a disabled effect is bypassed); enabling one is a
//! structural edit. The amplitude envelope always runs, even if disabled (whether disabling it is valid is S1b's
//! `validate`).

use std::collections::BTreeMap;
use std::fmt;

use enth_patch::catalogue::{MASTER_PARAMS, MasterParam, VOICE_PARAMS, VoiceMode, VoiceParam};
use enth_patch::{ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Section};

use crate::module::{MAX_VOICES, PrepareCfg};
use crate::modules::{
    GlobalModulatorImpl, GlobalProcessorImpl, ModuleImpl, VoiceModulatorImpl, VoiceProcessorImpl,
};
use crate::params::{GlobalTable, ParamId, Span, VoiceTable};
use crate::seed::module_key;

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

/// A compiled patch: what the audio thread runs. Build one with [`compile`].
pub struct Plan {
    pub(crate) cfg: PrepareCfg,
    pub(crate) generators: Vec<Slot<VoiceProcessorImpl>>,
    pub(crate) voice_fx: Vec<Slot<VoiceProcessorImpl>>,
    pub(crate) voice_mods: Vec<Slot<VoiceModulatorImpl>>,
    pub(crate) global_mods: Vec<Slot<GlobalModulatorImpl>>,
    pub(crate) fx: Vec<Slot<GlobalProcessorImpl>>,
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
    pub(crate) polyphony: usize,
    /// Voice modulator outputs, per modulator and voice: this segment's and the previous one's.
    pub(crate) vmod_cur: Vec<[f32; MAX_VOICES]>,
    pub(crate) vmod_prev: Vec<[f32; MAX_VOICES]>,
    /// Global modulator outputs.
    pub(crate) gmod_cur: Vec<f32>,
    pub(crate) gmod_prev: Vec<f32>,
}

impl fmt::Debug for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids = |s: &[&ModuleId]| s.iter().map(|i| i.as_str()).collect::<Vec<_>>().join(",");
        f.debug_struct("Plan")
            .field("sample_rate", &self.cfg.sample_rate)
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

    /// The number of subnormal values in every module's state and in the modulator outputs (test support).
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
        voice + vmods + gmods + fx + outs
    }
}

/// Parameter addresses → the plan's parameter ids. Built with the plan; used off the audio thread to turn an op or
/// an automation point into an [`EventKind::Param`](crate::event::EventKind::Param).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanMap {
    params: BTreeMap<ParamAddr, ParamId>,
}

impl PlanMap {
    /// The id of the parameter at `addr`, if it is in the plan (a disabled module's parameters are not).
    #[must_use]
    pub fn get(&self, addr: &ParamAddr) -> Option<ParamId> {
        self.params.get(addr).copied()
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
    /// A feature a later lane builds (the message names the lane).
    NotYet {
        /// What is missing.
        feature: &'static str,
        /// The lane that builds it.
        lane: &'static str,
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
/// value), an unsupported sample rate, or a feature S2a does not build yet ([`CompileError::NotYet`]).
pub fn compile(patch: &Patch, cfg: &PrepareCfg) -> Result<(Plan, PlanMap), CompileError> {
    if !cfg.is_supported() {
        return Err(CompileError::UnsupportedSampleRate(cfg.sample_rate));
    }
    check_values("voice", &VOICE_PARAMS, &patch.voice.values)?;
    check_values("master", &MASTER_PARAMS, &patch.master)?;
    let mode = VoiceMode::from_value(patch.voice.get(VoiceParam::Mode));
    if mode != Some(VoiceMode::Poly) {
        return Err(CompileError::NotYet {
            feature: "mono and legato voice modes",
            lane: "S2b",
        });
    }
    if !patch.routes.is_empty() {
        return Err(CompileError::NotYet {
            feature: "modulation routes",
            lane: "S2b",
        });
    }
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
    let amp_env_ok = patch.modulators.iter().any(|m| {
        &m.id == amp_env_id && m.kind == ModuleKind::Adsr && m.rate() == enth_patch::Rate::Voice
    });
    if !amp_env_ok {
        return Err(CompileError::BadAmpEnv(amp_env_id.to_string()));
    }

    let mut voice_table = VoiceTable::new();
    let mut global_table = GlobalTable::new();
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

    let mut plan = Plan {
        cfg: *cfg,
        generators: Vec::new(),
        voice_fx: Vec::new(),
        voice_mods: Vec::new(),
        global_mods: Vec::new(),
        fx: Vec::new(),
        amp_env: 0,
        sync_master: None,
        voice_table: VoiceTable::new(),
        global_table: GlobalTable::new(),
        voice_builtin,
        master_builtin,
        polyphony: patch.voice.get(VoiceParam::Polyphony) as usize,
        vmod_cur: Vec::new(),
        vmod_prev: Vec::new(),
        gmod_cur: Vec::new(),
        gmod_prev: Vec::new(),
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
    plan.voice_table = voice_table;
    plan.global_table = global_table;
    plan.vmod_cur = vec![[0.0; MAX_VOICES]; plan.voice_mods.len()];
    plan.vmod_prev = vec![[0.0; MAX_VOICES]; plan.voice_mods.len()];
    plan.gmod_cur = vec![0.0; plan.global_mods.len()];
    plan.gmod_prev = vec![0.0; plan.global_mods.len()];
    Ok((plan, map))
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
    use enth_patch::catalogue::{AdsrParam, LfoParam, LfoScope};
    use enth_patch::{ModSource, Route};

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
        assert_eq!(plan.sync_master, Some(0), "osc1 drives hard sync");
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
    fn s2b_features_are_refused_not_ignored() {
        let mut p = Patch::init();
        p.routes.push(Route {
            src: ModSource::Velocity,
            dst: "osc1.level".parse().unwrap(),
            depth: 50.0,
        });
        let e = compile(&p, &cfg()).unwrap_err();
        assert_eq!(e.code(), "not_yet");
        assert!(e.to_string().contains("S2b"), "{e}");
        let mut p = Patch::init();
        p.voice.values[VoiceParam::Mode.index()] = VoiceMode::Legato.index() as f32;
        assert_eq!(compile(&p, &cfg()).unwrap_err().code(), "not_yet");
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

        assert_eq!(
            compile(&Patch::init(), &PrepareCfg::new(1000.0))
                .unwrap_err()
                .code(),
            "unsupported_sample_rate"
        );
    }
}
