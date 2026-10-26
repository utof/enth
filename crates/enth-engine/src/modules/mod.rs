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
//! S2a's reserved kinds (docs/phase-s2.md) got the same treatment from lane S0d (freeze F0m): a stub file each, with
//! every dispatch arm below already written, so a module lane edits only its own file. They are hidden from the Add
//! menu, `enth modules` and the generator until lane INT-S2a clears the flag.
//!
//! | Kind | File | Role | S0d stub | Lane |
//! | --- | --- | --- | --- | --- |
//! | `dist` | `dist.rs` | voice processor (per-voice effect) | pass-through | DST |
//! | `wt` | `wt.rs` | voice processor (generator) | silent | WT (then UNI) |
//! | `ott` | `ott.rs` | global processor | pass-through | OTT |
//! | `comb` | `comb.rs` | voice processor (per-voice effect) | pass-through | FLT |
//! | `formant` | `formant.rs` | voice processor (per-voice effect) | pass-through | FLT |
//! | `mseg` | `mseg.rs` | voice or global modulator (`scope`) | outputs 0 (S2a: built) | MSEG |
//! | `chorus` | `chorus.rs` | global processor | pass-through | MOD |
//! | `phaser` | `phaser.rs` | global processor | pass-through | MOD |
//! | `eq` | `eq.rs` | global processor | pass-through | EQ |
//! | `delay` | `delay.rs` | global processor | pass-through | DLY |
//! | `ladder` | `ladder.rs` | voice processor (per-voice effect) | pass-through | FLT |
//!
//! A lane replaces the body of its own file and keeps the constructor, `Type::new(values: &[f32], cfg:
//! &PrepareCfg) -> Type`, and the role trait implementation(s) (see [`crate::module`] for the rules). Nothing else in
//! the engine changes when a module lands.

pub mod adsr;
pub mod chorus;
pub mod comb;
pub mod comp;
pub mod delay;
pub mod dist;
pub mod eq;
pub mod formant;
pub mod ladder;
pub mod lfo;
pub mod mseg;
pub mod noise;
pub mod osc;
pub mod ott;
pub mod phaser;
pub mod reverb;
pub mod svf;
pub mod wt;

use enth_patch::{ModuleKind, Rate};

pub use adsr::Adsr;
pub use chorus::Chorus;
pub use comb::Comb;
pub use comp::Comp;
pub use delay::Delay;
pub use dist::Dist;
pub use eq::Eq;
pub use formant::Formant;
pub use ladder::Ladder;
pub use lfo::Lfo;
pub use mseg::Mseg;
pub use noise::Noise;
pub use osc::Osc;
pub use ott::Ott;
pub use phaser::Phaser;
pub use reverb::Reverb;
pub use svf::Svf;
pub use wt::Wt;

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
    /// `dist` (S2a, reserved).
    Dist(Dist),
    /// `wt` (S2a, reserved).
    Wt(Wt),
    /// `comb` (S2a, reserved).
    Comb(Comb),
    /// `formant` (S2a, reserved).
    Formant(Formant),
    /// `ladder` (S2a, reserved).
    Ladder(Ladder),
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
    /// `mseg` with `scope = voice`.
    Mseg(Mseg),
}

/// The global modulators.
#[allow(
    clippy::large_enum_variant,
    reason = "one value per module, stored in the plan's Vecs; boxing would add an indirection to every dispatch"
)]
pub enum GlobalModulatorImpl {
    /// `lfo` with `scope = global`.
    Lfo(Lfo),
    /// `mseg` with `scope = global`.
    Mseg(Mseg),
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
    /// `ott` (S2a, reserved).
    Ott(Ott),
    /// `chorus` (S2a, reserved).
    Chorus(Chorus),
    /// `phaser` (S2a, reserved).
    Phaser(Phaser),
    /// `eq` (S2a, reserved).
    Eq(Eq),
    /// `delay` (S2a, reserved).
    Delay(Delay),
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
            ModuleKind::Dist => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Dist(Dist::new(values, cfg)))
            }
            ModuleKind::Wt => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Wt(Wt::new(values, cfg)))
            }
            ModuleKind::Comb => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Comb(Comb::new(values, cfg)))
            }
            ModuleKind::Formant => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Formant(Formant::new(values, cfg)))
            }
            ModuleKind::Ladder => {
                ModuleImpl::VoiceProcessor(VoiceProcessorImpl::Ladder(Ladder::new(values, cfg)))
            }
            ModuleKind::Mseg => match kind.rate(values) {
                Rate::Voice => {
                    ModuleImpl::VoiceModulator(VoiceModulatorImpl::Mseg(Mseg::new(values, cfg)))
                }
                Rate::Global => {
                    ModuleImpl::GlobalModulator(GlobalModulatorImpl::Mseg(Mseg::new(values, cfg)))
                }
            },
            ModuleKind::Ott => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Ott(Ott::new(values, cfg)))
            }
            ModuleKind::Chorus => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Chorus(Chorus::new(values, cfg)))
            }
            ModuleKind::Phaser => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Phaser(Phaser::new(values, cfg)))
            }
            ModuleKind::Eq => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Eq(Eq::new(values, cfg)))
            }
            ModuleKind::Delay => {
                ModuleImpl::GlobalProcessor(GlobalProcessorImpl::Delay(Delay::new(values, cfg)))
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
            VoiceProcessorImpl::Dist(_) => ModuleKind::Dist,
            VoiceProcessorImpl::Wt(_) => ModuleKind::Wt,
            VoiceProcessorImpl::Comb(_) => ModuleKind::Comb,
            VoiceProcessorImpl::Formant(_) => ModuleKind::Formant,
            VoiceProcessorImpl::Ladder(_) => ModuleKind::Ladder,
        }
    }

    /// Copy state from the same kind; `false` (and nothing copied) for another kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (VoiceProcessorImpl::Osc(a), VoiceProcessorImpl::Osc(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Noise(a), VoiceProcessorImpl::Noise(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Svf(a), VoiceProcessorImpl::Svf(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Dist(a), VoiceProcessorImpl::Dist(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Wt(a), VoiceProcessorImpl::Wt(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Comb(a), VoiceProcessorImpl::Comb(b)) => a.copy_state_from(b),
            (VoiceProcessorImpl::Formant(a), VoiceProcessorImpl::Formant(b)) => {
                a.copy_state_from(b);
            }
            (VoiceProcessorImpl::Ladder(a), VoiceProcessorImpl::Ladder(b)) => a.copy_state_from(b),
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
            VoiceProcessorImpl::Dist(m) => m.start_voice(start),
            VoiceProcessorImpl::Wt(m) => m.start_voice(start),
            VoiceProcessorImpl::Comb(m) => m.start_voice(start),
            VoiceProcessorImpl::Formant(m) => m.start_voice(start),
            VoiceProcessorImpl::Ladder(m) => m.start_voice(start),
        }
    }

    /// See [`VoiceProcessor::process`].
    pub fn process(&mut self, cx: &mut VoiceCx<'_>) {
        match self {
            VoiceProcessorImpl::Osc(m) => m.process(cx),
            VoiceProcessorImpl::Noise(m) => m.process(cx),
            VoiceProcessorImpl::Svf(m) => m.process(cx),
            VoiceProcessorImpl::Dist(m) => m.process(cx),
            VoiceProcessorImpl::Wt(m) => m.process(cx),
            VoiceProcessorImpl::Comb(m) => m.process(cx),
            VoiceProcessorImpl::Formant(m) => m.process(cx),
            VoiceProcessorImpl::Ladder(m) => m.process(cx),
        }
    }

    /// See [`VoiceProcessor::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            VoiceProcessorImpl::Osc(m) => m.subnormals(),
            VoiceProcessorImpl::Noise(m) => m.subnormals(),
            VoiceProcessorImpl::Svf(m) => m.subnormals(),
            VoiceProcessorImpl::Dist(m) => m.subnormals(),
            VoiceProcessorImpl::Wt(m) => m.subnormals(),
            VoiceProcessorImpl::Comb(m) => m.subnormals(),
            VoiceProcessorImpl::Formant(m) => m.subnormals(),
            VoiceProcessorImpl::Ladder(m) => m.subnormals(),
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
            VoiceModulatorImpl::Mseg(_) => ModuleKind::Mseg,
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
            (VoiceModulatorImpl::Mseg(a), VoiceModulatorImpl::Mseg(b)) => {
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
            VoiceModulatorImpl::Mseg(m) => VoiceModulator::start_voice(m, start),
        }
    }

    /// See [`VoiceModulator::tick`].
    pub fn tick(&mut self, cx: &mut VoiceModCx<'_>) {
        match self {
            VoiceModulatorImpl::Adsr(m) => VoiceModulator::tick(m, cx),
            VoiceModulatorImpl::Lfo(m) => VoiceModulator::tick(m, cx),
            VoiceModulatorImpl::Mseg(m) => VoiceModulator::tick(m, cx),
        }
    }

    /// See [`VoiceModulator::voice_finished`].
    #[must_use]
    pub fn voice_finished(&self, v: VoiceIdx) -> bool {
        match self {
            VoiceModulatorImpl::Adsr(m) => m.voice_finished(v),
            VoiceModulatorImpl::Lfo(m) => m.voice_finished(v),
            VoiceModulatorImpl::Mseg(m) => m.voice_finished(v),
        }
    }

    /// See [`VoiceModulator::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            VoiceModulatorImpl::Adsr(m) => VoiceModulator::subnormals(m),
            VoiceModulatorImpl::Lfo(m) => VoiceModulator::subnormals(m),
            VoiceModulatorImpl::Mseg(m) => VoiceModulator::subnormals(m),
        }
    }
}

impl GlobalModulatorImpl {
    /// The catalogue kind.
    #[must_use]
    pub fn kind(&self) -> ModuleKind {
        match self {
            GlobalModulatorImpl::Lfo(_) => ModuleKind::Lfo,
            GlobalModulatorImpl::Mseg(_) => ModuleKind::Mseg,
        }
    }

    /// Copy state from the same kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (GlobalModulatorImpl::Lfo(a), GlobalModulatorImpl::Lfo(b)) => {
                GlobalModulator::copy_state_from(a, b);
            }
            (GlobalModulatorImpl::Mseg(a), GlobalModulatorImpl::Mseg(b)) => {
                GlobalModulator::copy_state_from(a, b);
            }
            _ => return false,
        }
        true
    }

    /// See [`GlobalModulator::reset`].
    pub fn reset(&mut self, start: &GlobalStart<'_>) {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::reset(m, start),
            GlobalModulatorImpl::Mseg(m) => GlobalModulator::reset(m, start),
        }
    }

    /// See [`GlobalModulator::keys`].
    pub fn keys(&mut self, held: bool, phrase_start: bool) {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::keys(m, held, phrase_start),
            GlobalModulatorImpl::Mseg(m) => GlobalModulator::keys(m, held, phrase_start),
        }
    }

    /// See [`GlobalModulator::tick`].
    pub fn tick(&mut self, cx: &mut GlobalModCx<'_>) -> f32 {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::tick(m, cx),
            GlobalModulatorImpl::Mseg(m) => GlobalModulator::tick(m, cx),
        }
    }

    /// See [`GlobalModulator::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            GlobalModulatorImpl::Lfo(m) => GlobalModulator::subnormals(m),
            GlobalModulatorImpl::Mseg(m) => GlobalModulator::subnormals(m),
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
            GlobalProcessorImpl::Ott(_) => ModuleKind::Ott,
            GlobalProcessorImpl::Chorus(_) => ModuleKind::Chorus,
            GlobalProcessorImpl::Phaser(_) => ModuleKind::Phaser,
            GlobalProcessorImpl::Eq(_) => ModuleKind::Eq,
            GlobalProcessorImpl::Delay(_) => ModuleKind::Delay,
        }
    }

    /// Copy state from the same kind; `false` (and nothing copied) for another kind.
    pub fn copy_state(&mut self, other: &Self) -> bool {
        match (self, other) {
            (GlobalProcessorImpl::Reverb(a), GlobalProcessorImpl::Reverb(b)) => {
                a.copy_state_from(b);
            }
            (GlobalProcessorImpl::Comp(a), GlobalProcessorImpl::Comp(b)) => a.copy_state_from(b),
            (GlobalProcessorImpl::Ott(a), GlobalProcessorImpl::Ott(b)) => a.copy_state_from(b),
            (GlobalProcessorImpl::Chorus(a), GlobalProcessorImpl::Chorus(b)) => {
                a.copy_state_from(b);
            }
            (GlobalProcessorImpl::Phaser(a), GlobalProcessorImpl::Phaser(b)) => {
                a.copy_state_from(b);
            }
            (GlobalProcessorImpl::Eq(a), GlobalProcessorImpl::Eq(b)) => a.copy_state_from(b),
            (GlobalProcessorImpl::Delay(a), GlobalProcessorImpl::Delay(b)) => a.copy_state_from(b),
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
            (GlobalProcessorImpl::Ott(a), GlobalProcessorImpl::Ott(b)) => a.take_state_from(b),
            (GlobalProcessorImpl::Chorus(a), GlobalProcessorImpl::Chorus(b)) => {
                a.take_state_from(b);
            }
            (GlobalProcessorImpl::Phaser(a), GlobalProcessorImpl::Phaser(b)) => {
                a.take_state_from(b);
            }
            (GlobalProcessorImpl::Eq(a), GlobalProcessorImpl::Eq(b)) => a.take_state_from(b),
            (GlobalProcessorImpl::Delay(a), GlobalProcessorImpl::Delay(b)) => a.take_state_from(b),
            _ => return false,
        }
        true
    }

    /// See [`GlobalProcessor::reset`].
    pub fn reset(&mut self, start: &GlobalStart<'_>) {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.reset(start),
            GlobalProcessorImpl::Comp(m) => m.reset(start),
            GlobalProcessorImpl::Ott(m) => m.reset(start),
            GlobalProcessorImpl::Chorus(m) => m.reset(start),
            GlobalProcessorImpl::Phaser(m) => m.reset(start),
            GlobalProcessorImpl::Eq(m) => m.reset(start),
            GlobalProcessorImpl::Delay(m) => m.reset(start),
        }
    }

    /// See [`GlobalProcessor::process`].
    pub fn process(&mut self, cx: &mut GlobalCx<'_>) {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.process(cx),
            GlobalProcessorImpl::Comp(m) => m.process(cx),
            GlobalProcessorImpl::Ott(m) => m.process(cx),
            GlobalProcessorImpl::Chorus(m) => m.process(cx),
            GlobalProcessorImpl::Phaser(m) => m.process(cx),
            GlobalProcessorImpl::Eq(m) => m.process(cx),
            GlobalProcessorImpl::Delay(m) => m.process(cx),
        }
    }

    /// See [`GlobalProcessor::is_silent`].
    #[must_use]
    pub fn is_silent(&self) -> bool {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.is_silent(),
            GlobalProcessorImpl::Comp(m) => m.is_silent(),
            GlobalProcessorImpl::Ott(m) => m.is_silent(),
            GlobalProcessorImpl::Chorus(m) => m.is_silent(),
            GlobalProcessorImpl::Phaser(m) => m.is_silent(),
            GlobalProcessorImpl::Eq(m) => m.is_silent(),
            GlobalProcessorImpl::Delay(m) => m.is_silent(),
        }
    }

    /// See [`GlobalProcessor::subnormals`].
    #[must_use]
    pub fn subnormals(&self) -> usize {
        match self {
            GlobalProcessorImpl::Reverb(m) => m.subnormals(),
            GlobalProcessorImpl::Comp(m) => m.subnormals(),
            GlobalProcessorImpl::Ott(m) => m.subnormals(),
            GlobalProcessorImpl::Chorus(m) => m.subnormals(),
            GlobalProcessorImpl::Phaser(m) => m.subnormals(),
            GlobalProcessorImpl::Eq(m) => m.subnormals(),
            GlobalProcessorImpl::Delay(m) => m.subnormals(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::Section;
    use enth_patch::catalogue::{LfoParam, LfoScope, MsegParam, MsegScope};

    /// Every value of every structural parameter that changes the role: for P1a, only `lfo.scope`.
    fn variants(kind: ModuleKind) -> Vec<Vec<f32>> {
        let mut out = vec![kind.defaults()];
        if kind == ModuleKind::Lfo {
            let mut global = kind.defaults();
            global[LfoParam::Scope.index()] = LfoScope::Global.index() as f32;
            out.push(global);
        }
        if kind == ModuleKind::Mseg {
            let mut global = kind.defaults();
            global[MsegParam::Scope.index()] = MsegScope::Global.index() as f32;
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
        // Each (role, kind) pair below is one variant of a role enum: 3 + 2 + 1 + 2, plus S2a's reserved kinds
        // (5 + 2 + 5).
        let expected: std::collections::BTreeSet<_> = [
            (Role::VoiceProcessor, ModuleKind::Osc),
            (Role::VoiceProcessor, ModuleKind::Noise),
            (Role::VoiceProcessor, ModuleKind::Svf),
            (Role::VoiceModulator, ModuleKind::Adsr),
            (Role::VoiceModulator, ModuleKind::Lfo),
            (Role::GlobalModulator, ModuleKind::Lfo),
            (Role::GlobalProcessor, ModuleKind::Reverb),
            (Role::GlobalProcessor, ModuleKind::Comp),
            (Role::VoiceProcessor, ModuleKind::Dist),
            (Role::VoiceProcessor, ModuleKind::Wt),
            (Role::VoiceProcessor, ModuleKind::Comb),
            (Role::VoiceProcessor, ModuleKind::Formant),
            (Role::VoiceProcessor, ModuleKind::Ladder),
            (Role::VoiceModulator, ModuleKind::Mseg),
            (Role::GlobalModulator, ModuleKind::Mseg),
            (Role::GlobalProcessor, ModuleKind::Ott),
            (Role::GlobalProcessor, ModuleKind::Chorus),
            (Role::GlobalProcessor, ModuleKind::Phaser),
            (Role::GlobalProcessor, ModuleKind::Eq),
            (Role::GlobalProcessor, ModuleKind::Delay),
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
