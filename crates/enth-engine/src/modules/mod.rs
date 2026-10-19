//! One file per module kind, and the enum dispatch by role.
//!
//! [`ModuleImpl::new`] is an exhaustive `match` on [`ModuleKind`], so a catalogue kind without an implementation is a
//! compile error, not a failing test (architecture.md, rule 4). Each kind's file is owned by the lane that writes
//! its DSP; S2a left a clearly marked stub in each:
//!
//! | Kind | File | Role | S2a stub | Lane |
//! | --- | --- | --- | --- | --- |
//! | `osc` | `osc.rs` | voice processor (generator) | reference polyBLEP saw; other waves silent | D1 |
//! | `noise` | `noise.rs` | voice processor (generator) | silent | D1 |
//! | `svf` | `svf.rs` | voice processor (per-voice effect) | pass-through | D2 |
//! | `adsr` | `adsr.rs` | voice modulator | reference envelope (it is every patch's amp env) | D2 |
//! | `lfo` | `lfo.rs` | voice or global modulator (`scope`) | outputs 0 | D2 |
//! | `reverb` | `reverb.rs` | global processor | pass-through | FX1 |
//! | `comp` | `comp.rs` | global processor | pass-through | FX2 |
//!
//! A lane replaces the body of its own file and keeps the constructor, `Type::new(values: &[f32], cfg:
//! &PrepareCfg) -> Type`, and the role trait implementation(s) (see [`crate::module`] for the rules). Nothing else in
//! the engine changes when a module lands.

pub mod adsr;
pub mod comp;
pub mod lfo;
pub mod noise;
pub mod osc;
pub mod reverb;
pub mod svf;

use enth_patch::{ModuleKind, Rate};

pub use adsr::Adsr;
pub use comp::Comp;
pub use lfo::Lfo;
pub use noise::Noise;
pub use osc::Osc;
pub use reverb::Reverb;
pub use svf::Svf;

use crate::module::{
    GlobalCx, GlobalModCx, GlobalModulator, GlobalProcessor, GlobalStart, PrepareCfg, Role,
    VoiceCx, VoiceIdx, VoiceModCx, VoiceModulator, VoiceProcessor, VoiceStart,
};

/// A prepared module, by role. Built by [`ModuleImpl::new`]; the plan compiler sorts the variants into its
/// per-role lists, so the segment loop dispatches without ever meeting a module in the wrong role.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum ModuleImpl {
    /// Generators and per-voice effects.
    VoiceProcessor(VoiceProcessorImpl),
    /// Envelopes and voice LFOs.
    VoiceModulator(VoiceModulatorImpl),
    /// Global LFOs.
    GlobalModulator(GlobalModulatorImpl),
    /// Global effects.
    GlobalProcessor(GlobalProcessorImpl),
}

/// The voice processors.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum VoiceProcessorImpl {
    /// `osc`.
    Osc(Osc),
    /// `noise`.
    Noise(Noise),
    /// `svf`.
    Svf(Svf),
}

/// The voice modulators.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum VoiceModulatorImpl {
    /// `adsr`.
    Adsr(Adsr),
    /// `lfo` with `scope = voice`.
    Lfo(Lfo),
}

/// The global modulators.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum GlobalModulatorImpl {
    /// `lfo` with `scope = global`.
    Lfo(Lfo),
}

/// The global processors.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum GlobalProcessorImpl {
    /// `reverb`.
    Reverb(Reverb),
    /// `comp`.
    Comp(Comp),
}

impl ModuleImpl {
    /// Prepare a module of `kind` from its patch `values` (one per parameter, in spec order, valid for their specs)
    /// and `cfg`. This is the constructor and the only place that may allocate. The role follows the kind, except
    /// that an `lfo` is a voice or a global modulator by its structural `scope` parameter.
    ///
    /// The `match` is exhaustive on purpose: a new `ModuleKind` does not compile until it has an implementation.
    #[must_use]
    pub fn new(kind: ModuleKind, values: &[f32], cfg: &PrepareCfg) -> ModuleImpl {
        match kind {
            ModuleKind::Osc => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Osc(Osc::new(values, cfg)))
            }
            ModuleKind::Noise => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Noise(Noise::new(values, cfg)))
            }
            ModuleKind::Svf => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Svf(Svf::new(values, cfg)))
            }
            ModuleKind::Adsr => {
                ModuleImpl::VoiceModulator(VoiceModulatorImpl::Adsr(Adsr::new(values, cfg)))
            }
            ModuleKind::Lfo => match kind.rate(values) {
                Rate::Voice => {
                    ModuleImpl::VoiceModulator(VoiceModulatorImpl::Lfo(Lfo::new(values, cfg)))
                }
                Rate::Global => {
                    ModuleImpl::GlobalModulator(GlobalModulatorImpl::Lfo(Lfo::new(values, cfg)))
                }
            },
            ModuleKind::Reverb => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Reverb(Reverb::new(values, cfg)))
            }
            ModuleKind::Comp => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Comp(Comp::new(values, cfg)))
            }
        }
    }

    /// The catalogue kind this implements.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            ModuleImpl::VoiceProcessor(m) => m.kind(),
            ModuleImpl::VoiceModulator(m) => m.kind(),
            ModuleImpl::GlobalModulator(m) => m.kind(),
            ModuleImpl::GlobalProcessor(m) => m.kind(),
        }
    }

    /// The role.
    #[must_use]
    pub fn role(&self) -> Role {
        match self {
            ModuleImpl::VoiceProcessor(_) => Role::VoiceProcessor,
            ModuleImpl::VoiceModulator(_) => Role::VoiceModulator,
            ModuleImpl::GlobalModulator(_) => Role::GlobalModulator,
            ModuleImpl::GlobalProcessor(_) => Role::GlobalProcessor,
        }
    }

    /// Copy `other`'s state if it is the same role and kind (for the plan swap, lane S2b); `false` otherwise.
    /// No allocation.
    pub fn copy_state_from(&mut self, other: &ModuleImpl) -> bool {
        match (self, other) {
            (ModuleImpl::VoiceProcessor(a), ModuleImpl::VoiceProcessor(b)) => a.copy_state(b),
            (ModuleImpl::VoiceModulator(a), ModuleImpl::VoiceModulator(b)) => a.copy_state(b),
            (ModuleImpl::GlobalModulator(a), ModuleImpl::GlobalModulator(b)) => a.copy_state(b),
            (ModuleImpl::GlobalProcessor(a), ModuleImpl::GlobalProcessor(b)) => a.copy_state(b),
            _ => false,
        }
    }

    /// Take `other`'s state, as an instant plan swap does (P1b lane E+): global processors move it
    /// ([`GlobalProcessor::take_state_from`]), the other roles copy. `other` must not run again. `false` (nothing
    /// taken) for another role or kind. No allocation.
    pub fn take_state_from(&mut self, other: &mut ModuleImpl) -> bool {
        match (self, other) {
            (ModuleImpl::GlobalProcessor(a), ModuleImpl::GlobalProcessor(b)) => a.take_state(b),
            (a, b) => a.copy_state_from(b),
        }
    }

    /// The number of subnormal values in the module's state.
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            ModuleImpl::VoiceProcessor(m) => m.subnormals(),
            ModuleImpl::VoiceModulator(m) => m.subnormals(),
            ModuleImpl::GlobalModulator(m) => m.subnormals(),
            ModuleImpl::GlobalProcessor(m) => m.subnormals(),
        }
    }
}

impl VoiceProcessorImpl {
    /// The catalogue kind.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            VoiceProcessorImpl::Osc(_) => ModuleKind::Osc,
            VoiceProcessorImpl::Noise(_) => ModuleKind::Noise,
            VoiceProcessorImpl::Svf(_) => ModuleKind::Svf,
        }
    }

    /// Copy state from the same kind; `false` (and nothing copied) for another kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (VoiceProcessorImpl::Osc(a), VoiceProcessorImpl::Osc(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Noise(a), VoiceProcessorImpl::Noise(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Svf(a), VoiceProcessorImpl::Svf(b)) => a.copy_state_from(b),
            _ => return false,
        }
        true
    }

    /// See [`VoiceProcessor::start_voice`].
    pub fn start_voice(&mut self, start: &VoiceStart<'_>) {
        match self {
            VoiceProcessorImpl::Osc(m) => m.start_voice(start),
            VoiceProcessorImpl::Noise(m) => m.start_voice(start),
            VoiceProcessorImpl::Svf(m) => m.start_voice(start),
        }
    }

    /// See [`VoiceProcessor::process`].
    pub fn process(&mut self, cx: &mut VoiceCx<'_>) {
        match self {
            VoiceProcessorImpl::Osc(m) => m.process(cx),
            VoiceProcessorImpl::Noise(m) => m.process(cx),
            VoiceProcessorImpl::Svf(m) => m.process(cx),
        }
    }

    /// See [`VoiceProcessor::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            VoiceProcessorImpl::Osc(m) => m.subnormals(),
            VoiceProcessorImpl::Noise(m) => m.subnormals(),
            VoiceProcessorImpl::Svf(m) => m.subnormals(),
        }
    }
}

impl VoiceModulatorImpl {
    /// The catalogue kind.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            VoiceModulatorImpl::Adsr(_) => ModuleKind::Adsr,
            VoiceModulatorImpl::Lfo(_) => ModuleKind::Lfo,
        }
    }

    /// Copy state from the same kind; `false` (and nothing copied) for another kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (VoiceModulatorImpl::Adsr(a), VoiceModulatorImpl::Adsr(b)) => {
                VoiceModulator::copy_state_from(a, b);
            }
            (VoiceModulatorImpl::Lfo(a), VoiceModulatorImpl::Lfo(b)) => {
                VoiceModulator::copy_state_from(a, b);
            }
            _ => return false,
        }
        true
    }

    /// See [`VoiceModulator::start_voice`].
    pub fn start_voice(&mut self, start: &VoiceStart<'_>) {
        match self {
            VoiceModulatorImpl::Adsr(m) => VoiceModulator::start_voice(m, start),
            VoiceModulatorImpl::Lfo(m) => VoiceModulator::start_voice(m, start),
        }
    }

    /// See [`VoiceModulator::tick`].
    pub fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        match self {
            VoiceModulatorImpl::Adsr(m) => VoiceModulator::tick(m, cx),
            VoiceModulatorImpl::Lfo(m) => VoiceModulator::tick(m, cx),
        }
    }

    /// See [`VoiceModulator::voice_finished`].
    #[must_use]
    pub fn voice_finished(&self, v: VoiceIdx) -> bool {
        match self {
            VoiceModulatorImpl::Adsr(m) => m.voice_finished(v),
            VoiceModulatorImpl::Lfo(m) => m.voice_finished(v),
        }
    }

    /// See [`VoiceModulator::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            VoiceModulatorImpl::Adsr(m) => VoiceModulator::subnormals(m),
            VoiceModulatorImpl::Lfo(m) => VoiceModulator::subnormals(m),
        }
    }
}

impl GlobalModulatorImpl {
    /// The catalogue kind.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            GlobalModulatorImpl::Lfo(_) => ModuleKind::Lfo,
        }
    }

    /// Copy state from the same kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (GlobalModulatorImpl::Lfo(a), GlobalModulatorImpl::Lfo(b)) => {
                GlobalModulator::copy_state_from(a, b);
            }
        }
        true
    }

    /// See [`GlobalModulator::reset`].
    pub fn reset(&mut self, start: &GlobalStart<'_>) {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::reset(m, start),
        }
    }

    /// See [`GlobalModulator::tick`].
    pub fn tick(&mut self, cx: &mut GlobalModCx<'_>) -> f32 {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::tick(m, cx),
        }
    }

    /// See [`GlobalModulator::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::subnormals(m),
        }
    }
}

impl GlobalProcessorImpl {
    /// The catalogue kind.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            GlobalProcessorImpl::Reverb(_) => ModuleKind::Reverb,
            GlobalProcessorImpl::Comp(_) => ModuleKind::Comp,
        }
    }

    /// Copy state from the same kind; `false` (and nothing copied) for another kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (GlobalProcessorImpl::Reverb(a), GlobalProcessorImpl::Reverb(b)) => {
                a.copy_state_from(b);
            }
            (GlobalProcessorImpl::Comp(a), GlobalProcessorImpl::Comp(b)) => a.copy_state_from(b),
            _ => return false,
        }
        true
    }

    /// Take the state of the same kind from `other`, which is never run again (see
    /// [`GlobalProcessor::take_state_from`]); `false` (and nothing taken) for another kind.
    pub fn take_state(&mut self, other: &mut Self) -> bool {
        match (self, other) {
            (GlobalProcessorImpl::Reverb(a), GlobalProcessorImpl::Reverb(b)) => {
                a.take_state_from(b);
            }
            (GlobalProcessorImpl::Comp(a), GlobalProcessorImpl::Comp(b)) => a.take_state_from(b),
            _ => return false,
        }
        true
    }

    /// See [`GlobalProcessor::reset`].
    pub fn reset(&mut self, start: &GlobalStart<'_>) {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.reset(start),
            GlobalProcessorImpl::Comp(m) => m.reset(start),
        }
    }

    /// See [`GlobalProcessor::process`].
    pub fn process(&mut self, cx: &mut GlobalCx<'_>) {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.process(cx),
            GlobalProcessorImpl::Comp(m) => m.process(cx),
        }
    }

    /// See [`GlobalProcessor::is_silent`].
    #[must_use]
    pub fn is_silent(&self) -> bool {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.is_silent(),
            GlobalProcessorImpl::Comp(m) => m.is_silent(),
        }
    }

    /// See [`GlobalProcessor::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.subnormals(),
            GlobalProcessorImpl::Comp(m) => m.subnormals(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Section;
    use enth_patch::catalogue::{LfoParam, LfoScope};

    /// Every value of every structural parameter that changes the role: for P1a, only `lfo.scope`.
    fn variants(kind: ModuleKind) -> Vec<Vec<f32>> {
        let mut out = vec![kind.defaults()];
        if kind == ModuleKind::Lfo {
            let mut global = kind.defaults();
            global[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
            out.push(global);
        }
        out
    }

    /// The registry test: every catalogue kind has an implementation (also a compile error, thanks to the exhaustive
    /// match), it reports the kind it was built for, its role agrees with the catalogue's section and rate, and
    /// every role variant is reachable from some kind ("and vice versa").
    #[test]
    fn every_kind_has_an_implementation_in_the_role_its_section_implies() {
        let cfg = PrepareCfg::new(48_000.0);
        let mut seen = std::collections::BTreeSet::new();
        for &kind in ModuleKind::ALL {
            for values in variants(kind) {
                let m = ModuleImpl::new(kind, &values, &cfg);
                assert_eq!(m.kind(), kind);
                let role = m.role();
                let expected = match (kind.section(), kind.rate(&values)) {
                    (Section::Generators | Section::VoiceFx, _) => Role::VoiceProcessor,
                    (Section::Fx, _) => Role::GlobalProcessor,
                    (Section::Modulators, Rate::Voice) => Role::VoiceModulator,
                    (Section::Modulators, Rate::Global) => Role::GlobalModulator,
                };
                assert_eq!(role, expected, "{kind}");
                assert_eq!(role.rate(), kind.rate(&values), "{kind}");
                seen.insert((role, kind));
                assert_eq!(m.subnormals(), 0, "{kind} starts clean");
            }
        }
        // Each (role, kind) pair below is one variant of a role enum: 3 + 2 + 1 + 2.
        let expected: std::collections::BTreeSet<_> = [
            (Role::VoiceProcessor, ModuleKind::Osc),
            (Role::VoiceProcessor, ModuleKind::Noise),
            (Role::VoiceProcessor, ModuleKind::Svf),
            (Role::VoiceModulator, ModuleKind::Adsr),
            (Role::VoiceModulator, ModuleKind::Lfo),
            (Role::GlobalModulator, ModuleKind::Lfo),
            (Role::GlobalProcessor, ModuleKind::Reverb),
            (Role::GlobalProcessor, ModuleKind::Comp),
        ]
        .into_iter()
        .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn state_copies_only_between_the_same_kind() {
        let cfg = PrepareCfg::new(48_000.0);
        let mut osc = ModuleImpl::new(ModuleKind::Osc, &ModuleKind::Osc.defaults(), &cfg);
        let osc2 = ModuleImpl::new(ModuleKind::Osc, &ModuleKind::Osc.defaults(), &cfg);
        let svf = ModuleImpl::new(ModuleKind::Svf, &ModuleKind::Svf.defaults(), &cfg);
        let rev = ModuleImpl::new(ModuleKind::Reverb, &ModuleKind::Reverb.defaults(), &cfg);
        assert!(osc.copy_state_from(&osc2));
        assert!(!osc.copy_state_from(&svf));
        assert!(!osc.copy_state_from(&rev));
    }
}
