//! Patch v1 in memory: `Patch`, `ModuleInstance`, `Route`, `Macro`, `VoiceSettings`, `PatchMeta`, sections and
//! limits (design.md §2).
//!
//! A patch is plain data with stable ids and never contains engine state. It holds **plain values in real units**
//! (Hz, ms, dB, cents, %), one dense `Vec<f32>` per module in its kind's spec order. The on-disk form is
//! [`PatchFile`](crate::file::PatchFile) (sparse in, dense out); converting between the two, validation, ops,
//! canonicalisation and hashes are in their own modules (lane S1b).
//!
//! # Invariants of a valid patch
//!
//! Fields are public so that ops, the file layer, tests and generators can build any patch, including invalid ones
//! to test validation. **Product code changes a patch only through `Op`s** (AGENTS.md), and `validate` (S1b)
//! checks these invariants at every boundary (file load, after ops in debug builds, before compiling a plan):
//!
//! 1. Module ids are unique across all sections (their grammar and reserved words are enforced by [`ModuleId`]).
//! 2. Each module sits in its kind's section (`ModuleKind::section`), and each section holds at most its limit
//!    ([`Section::max_len`]).
//! 3. `values.len()` equals the kind's parameter count, and every value is valid for its spec
//!    ([`ParamSpec::is_valid`](enth_params::ParamSpec::is_valid): finite, in range, integral where discrete, never
//!    `-0.0`). The same holds for `voice.values` and `master`.
//! 4. Routes: the source exists (a modulator in this patch, or a built-in source), the destination exists and is
//!    `modulatable`, `|depth| <= 100`, no two routes share (src, dst), at most [`MAX_ROUTES`], and a voice-rate
//!    source never targets a global destination (`route_rate_mismatch`; see [`Patch::source_rate`] and
//!    [`Patch::target_rate`]).
//! 5. `voice.amp_env` names an `adsr` module in `modulators`.
//! 6. Every slot points at an existing `automatable` parameter; macro values are in 0–1.

use core::fmt;

use enth_params::ParamSpec;

use crate::addr::{ModSource, ModuleId, ParamAddr, Position, Target};
use crate::catalogue::{
    Builtin, MACRO_VALUE, MASTER_PARAMS, MasterParam, ModuleKind, ModuleSpec, Rate, VOICE_PARAMS,
    VoiceParam,
};

/// Most generators per patch (summed per voice).
pub const MAX_GENERATORS: usize = 4;
/// Most per-voice effects (a serial chain).
pub const MAX_VOICE_FX: usize = 4;
/// Most global effects (a serial chain).
pub const MAX_FX: usize = 4;
/// Most modulators.
pub const MAX_MODULATORS: usize = 8;
/// Most modulation routes.
pub const MAX_ROUTES: usize = 64;
/// Number of macros (host parameters 1–8).
pub const MACROS: usize = 8;
/// Number of host automation slots (design.md §4).
pub const SLOTS: usize = 64;

/// The four module sections, in their fixed order (the order of the file and of `canonical_hash`).
///
/// Sections, not lanes: the per-voice/global split is explicit. A later `lanes: Vec<Lane { poly, modules }>` is
/// a pure migration from `voice_fx` and `fx` (design.md §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// Per voice, parallel: summed in order into the voice bus.
    Generators,
    /// Per voice, serial: after the generator sum and before the amp stage.
    VoiceFx,
    /// Global, serial, stereo: after the voice sum and before `master`.
    Fx,
    /// Evaluated in order, global ones first; a route from a later modulator reads its previous-segment value.
    Modulators,
}

impl Section {
    /// Every section, in file order.
    pub const ALL: [Section; 4] = [
        Section::Generators,
        Section::VoiceFx,
        Section::Fx,
        Section::Modulators,
    ];

    /// The key used in files and positions (`"voice_fx"`).
    pub const fn key(self) -> &'static str {
        match self {
            Section::Generators => "generators",
            Section::VoiceFx => "voice_fx",
            Section::Fx => "fx",
            Section::Modulators => "modulators",
        }
    }

    /// The section with this key.
    pub fn from_key(key: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The most modules this section may hold.
    pub const fn max_len(self) -> usize {
        match self {
            Section::Generators => MAX_GENERATORS,
            Section::VoiceFx => MAX_VOICE_FX,
            Section::Fx => MAX_FX,
            Section::Modulators => MAX_MODULATORS,
        }
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// One module in a patch.
///
/// Invariant (checked by `validate`): `values` has one entry per parameter of `kind`, in spec order, each valid
/// for its spec; enum values are item indices, toggles 0 or 1.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleInstance {
    /// Stable, unique id (`"filter1"`); routes, slots and seeds refer to it.
    pub id: ModuleId,
    /// The catalogue kind.
    pub kind: ModuleKind,
    /// A display label for the UI; not hashed, never changes the sound.
    pub label: Option<String>,
    /// A disabled module is bypassed (a generator is silent); `canonical_hash` treats it as absent.
    pub enabled: bool,
    /// Plain values, dense, in the kind's spec order.
    pub values: Vec<f32>,
}

impl ModuleInstance {
    /// A new, enabled module with every parameter at its default.
    pub fn new(id: ModuleId, kind: ModuleKind) -> Self {
        ModuleInstance {
            id,
            kind,
            label: None,
            enabled: true,
            values: kind.defaults(),
        }
    }

    /// The catalogue descriptor of this module's kind.
    pub const fn spec(&self) -> &'static ModuleSpec {
        self.kind.spec()
    }

    /// The value of the parameter with this key.
    pub fn get(&self, key: &str) -> Option<f32> {
        self.kind
            .param_index(key)
            .and_then(|i| self.values.get(i).copied())
    }

    /// Whether this module runs per voice or globally (an LFO follows its `scope`).
    pub fn rate(&self) -> Rate {
        self.kind.rate(&self.values)
    }
}

/// A modulation route: `src` moves `dst` by `depth` percent of the destination's range, in its normalised domain:
/// `n = clamp(to_norm(base) + Σ depth/100 · src, 0, 1)` (design.md §2). Routes sum in patch order.
///
/// Polarity belongs to the source, so a route carries only a depth. Invariant: `-100 <= depth <= 100`
/// ([`enth_params::DEPTH`]); `canonical_hash` drops depth-0 routes.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The modulation source.
    pub src: ModSource,
    /// The destination parameter; must be `modulatable`.
    pub dst: ParamAddr,
    /// Percent of the destination's range, −100..=100.
    pub depth: f32,
}

/// One of the eight macros: a global modulation source with a knob position, exposed to the host.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Macro {
    /// Knob position, 0–1 ([`MACRO_VALUE`]).
    pub value: f32,
    /// A display name ("Brightness"); not hashed.
    pub name: Option<String>,
}

/// The built-in `voice`: its parameters and the amplitude envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceSettings {
    /// Plain values in [`VOICE_PARAMS`] order (index with [`VoiceParam`]).
    pub values: [f32; VoiceParam::COUNT],
    /// The `adsr` module in `modulators` that shapes each voice's amplitude. A voice ends when this envelope, past
    /// its attack, is below −100 dB. Removing that module is an error (`in_use_as_amp_env`).
    pub amp_env: ModuleId,
}

impl VoiceSettings {
    /// Default voice parameters with the given amplitude envelope.
    pub fn new(amp_env: ModuleId) -> Self {
        VoiceSettings {
            values: core::array::from_fn(|i| VOICE_PARAMS[i].default),
            amp_env,
        }
    }

    /// The value of one voice parameter.
    pub const fn get(&self, p: VoiceParam) -> f32 {
        self.values[p.index()]
    }
}

/// Descriptive metadata. Not hashed, never changes the sound.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PatchMeta {
    /// Patch name ("Soft pluck"); empty if unnamed.
    pub name: String,
    /// Author; empty if unknown.
    pub author: String,
    /// Free text.
    pub description: String,
    /// Tags ("pluck", "bass").
    pub tags: Vec<String>,
}

/// A patch in memory (design.md §2). See the [module docs](self) for the invariants of a valid patch.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// The engine version the patch was authored on (`"0.1.0"`); the schema number lives in the file layer.
    pub engine: String,
    /// Name, author, tags, description.
    pub meta: PatchMeta,
    /// The built-in `voice`.
    pub voice: VoiceSettings,
    /// The built-in `master`, in [`MASTER_PARAMS`] order (index with [`MasterParam`]); applied after `fx`.
    pub master: [f32; MasterParam::COUNT],
    /// Per voice, parallel, summed in this order.
    pub generators: Vec<ModuleInstance>,
    /// Per voice, serial, after the generator sum and before the amp stage.
    pub voice_fx: Vec<ModuleInstance>,
    /// Global, serial, stereo, after the voice sum.
    pub fx: Vec<ModuleInstance>,
    /// Envelopes and LFOs, evaluated in this order.
    pub modulators: Vec<ModuleInstance>,
    /// Modulation routes; the order is the summation order.
    pub routes: Vec<Route>,
    /// The eight macros (index with [`MacroIndex::get`](crate::addr::MacroIndex::get)).
    pub macros: [Macro; MACROS],
    /// Host automation slots, each pointing at an automatable parameter or empty (design.md §4); index with
    /// [`SlotIndex::get`](crate::addr::SlotIndex::get).
    pub slots: [Option<ParamAddr>; SLOTS],
}

impl Patch {
    /// The smallest valid patch: one saw oscillator (`osc1`) and one envelope (`env1`) as the amplitude envelope,
    /// everything else at its default, no routes, macros at 0, no slots, `engine` empty (the file layer stamps
    /// it). Templates (`enth new --template …`) build on this through ops.
    pub fn init() -> Patch {
        let osc1 = ModuleId::new("osc1").expect("valid id");
        let env1 = ModuleId::new("env1").expect("valid id");
        Patch {
            engine: String::new(),
            meta: PatchMeta::default(),
            voice: VoiceSettings::new(env1.clone()),
            master: core::array::from_fn(|i| MASTER_PARAMS[i].default),
            generators: vec![ModuleInstance::new(osc1, ModuleKind::Osc)],
            voice_fx: Vec::new(),
            fx: Vec::new(),
            modulators: vec![ModuleInstance::new(env1, ModuleKind::Adsr)],
            routes: Vec::new(),
            macros: core::array::from_fn(|_| Macro {
                value: MACRO_VALUE.default,
                name: None,
            }),
            slots: core::array::from_fn(|_| None),
        }
    }

    /// The modules of one section, in order.
    pub fn section(&self, section: Section) -> &[ModuleInstance] {
        match section {
            Section::Generators => &self.generators,
            Section::VoiceFx => &self.voice_fx,
            Section::Fx => &self.fx,
            Section::Modulators => &self.modulators,
        }
    }

    /// The modules of one section, mutable (for ops and the file layer).
    pub fn section_mut(&mut self, section: Section) -> &mut Vec<ModuleInstance> {
        match section {
            Section::Generators => &mut self.generators,
            Section::VoiceFx => &mut self.voice_fx,
            Section::Fx => &mut self.fx,
            Section::Modulators => &mut self.modulators,
        }
    }

    /// Every module with its position, sections in file order.
    pub fn modules(&self) -> impl Iterator<Item = (Position, &ModuleInstance)> {
        Section::ALL.into_iter().flat_map(move |s| {
            self.section(s)
                .iter()
                .enumerate()
                .map(move |(index, m)| (Position { section: s, index }, m))
        })
    }

    /// The module with this id, and where it is.
    pub fn find(&self, id: &ModuleId) -> Option<(Position, &ModuleInstance)> {
        self.modules().find(|(_, m)| &m.id == id)
    }

    /// The spec of the parameter an address names, if the target exists and has that parameter.
    pub fn spec(&self, addr: &ParamAddr) -> Option<&'static ParamSpec> {
        match addr.target() {
            Target::Module(id) => self.find(id)?.1.kind.param(addr.param()),
            Target::Voice => Builtin::Voice
                .param_index(addr.param())
                .map(|i| &VOICE_PARAMS[i]),
            Target::Master => Builtin::Master
                .param_index(addr.param())
                .map(|i| &MASTER_PARAMS[i]),
        }
    }

    /// The current plain value at an address.
    pub fn value(&self, addr: &ParamAddr) -> Option<f32> {
        match addr.target() {
            Target::Module(id) => self.find(id)?.1.get(addr.param()),
            Target::Voice => Builtin::Voice
                .param_index(addr.param())
                .map(|i| self.voice.values[i]),
            Target::Master => Builtin::Master
                .param_index(addr.param())
                .map(|i| self.master[i]),
        }
    }

    /// The rate of a modulation source: velocity and key are per voice, the mod wheel and macros global, and a
    /// module follows [`ModuleInstance::rate`]. `None` if the source module does not exist.
    pub fn source_rate(&self, src: &ModSource) -> Option<Rate> {
        match src {
            ModSource::Velocity | ModSource::Key => Some(Rate::Voice),
            ModSource::ModWheel | ModSource::Macro(_) => Some(Rate::Global),
            ModSource::Module(id) => self.find(id).map(|(_, m)| m.rate()),
        }
    }

    /// The rate of a destination's target: `voice.*`, generators and `voice_fx` are per voice; `master.*` and `fx`
    /// are global; a modulator follows its own rate. A voice-rate source may drive a voice-rate target only; a
    /// global source may drive either. `None` if the target module does not exist.
    pub fn target_rate(&self, target: &Target) -> Option<Rate> {
        match target {
            Target::Voice => Some(Rate::Voice),
            Target::Master => Some(Rate::Global),
            Target::Module(id) => self.find(id).map(|(_, m)| m.rate()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{AdsrParam, OscParam, Wave};

    #[test]
    fn init_patch_is_minimal_and_default() {
        let p = Patch::init();
        assert_eq!(p.generators.len(), 1);
        assert_eq!(p.modulators.len(), 1);
        assert_eq!(p.voice.amp_env.as_str(), "env1");
        assert_eq!(
            p.generators[0].values[OscParam::Wave.index()],
            Wave::Saw.index() as f32
        );
        assert_eq!(p.modulators[0].values[AdsrParam::Attack.index()], 5.0);
        assert_eq!(p.voice.get(VoiceParam::Polyphony), 8.0);
        assert_eq!(p.master[MasterParam::Level.index()], 0.0);
        assert!(p.slots.iter().all(Option::is_none));
    }

    #[test]
    fn lookups() {
        let p = Patch::init();
        let addr: ParamAddr = "osc1.semi".parse().unwrap();
        assert_eq!(p.value(&addr), Some(0.0));
        assert_eq!(p.spec(&addr).map(|s| s.key), Some("semi"));
        let (pos, m) = p.find(&ModuleId::new("env1").unwrap()).unwrap();
        assert_eq!(
            (pos.section, pos.index, m.kind),
            (Section::Modulators, 0, ModuleKind::Adsr)
        );
        assert_eq!(p.value(&"voice.polyphony".parse().unwrap()), Some(8.0));
        assert_eq!(p.value(&"master.level".parse().unwrap()), Some(0.0));
        // `cutoff` is a module key, but osc1 has none.
        assert_eq!(p.value(&"osc1.cutoff".parse().unwrap()), None);
        assert_eq!(p.source_rate(&ModSource::Velocity), Some(Rate::Voice));
        assert_eq!(p.source_rate(&"env1".parse().unwrap()), Some(Rate::Voice));
        assert_eq!(p.target_rate(&Target::Master), Some(Rate::Global));
        assert_eq!(p.modules().count(), 2);
    }
}
