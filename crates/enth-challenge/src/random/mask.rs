//! **Owner: lane RG.** The observability mask (design.md §9, report 01 §2.3): what a phrase can reveal of a patch.
//!
//! [`observable`] is pure and works in parameter space only. It classifies every parameter, every route and every
//! module's enable flag of a patch as heard through a phrase:
//!
//! - [`Visibility::Observable`]: a change is audible;
//! - [`Visibility::Gauge`]: a pure gain that loudness normalisation removes, so it never varies alone: the first
//!   enabled generator's level (levels vary only as ratios between sources), `master.level`, `voice.pan` (the scored
//!   signal is mid), `comp.makeup` and `reverb.dry` (wet varies only against dry);
//! - [`Visibility::Nuisance`]: changes the samples but not the sound: phase modes, start phases, seeds;
//! - [`Visibility::Disabled`]: in a disabled module;
//! - [`Visibility::Dead`]: does nothing under this patch and phrase, with the reason ([`Dead`]).
//!
//! The generator randomizes only `Observable` parameters. Lane ST's structure % and the editor's score rings reuse the
//! same mask: comparing a gauge, a nuisance or a dead parameter would punish a difference nobody can hear (metric v1
//! measured `osc1.level` 100 → 50 %, `master.level` −6 dB and a 300 ms glide on one note at a distance of 0.0000).
//!
//! Degenerate pairs: with one note, keytrack equals a cutoff move (both 0.4487 in report 01), so keytrack is dead
//! unless two notes an octave apart are played, and the generator never randomizes both.
//!
//! What the mask does not decide: anything that needs a render (a compressor whose threshold sits above the signal,
//! a cutoff far above the harmonics). Those stay `Observable`.

use std::collections::BTreeMap;

use enth_patch::catalogue::{
    AdsrParam, GlideMode, LfoParam, OscParam, VoiceMode, VoiceParam, Wave,
};
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, RouteAddr, Section, Target,
};

use crate::model::{Phrase, PhraseView};
use crate::phrases::PhraseFacts;

/// How a phrase reveals one parameter, route or enable flag. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    /// A change is audible through the phrase.
    Observable,
    /// A pure gain, removed by loudness normalisation; never varies alone.
    Gauge,
    /// Changes the samples, not the sound (phase, seed).
    Nuisance,
    /// In a disabled module.
    Disabled,
    /// Does nothing under this patch and phrase.
    Dead(Dead),
}

impl Visibility {
    /// Only [`Visibility::Observable`] counts: the generator randomizes it and scoring compares it.
    pub const fn is_observable(self) -> bool {
        matches!(self, Visibility::Observable)
    }

    /// A stable lowercase name for JSON and logs (`observable`, `gauge`, `dead:not_pulse`, …).
    pub const fn name(self) -> &'static str {
        match self {
            Visibility::Observable => "observable",
            Visibility::Gauge => "gauge",
            Visibility::Nuisance => "nuisance",
            Visibility::Disabled => "disabled",
            Visibility::Dead(d) => d.name(),
        }
    }
}

/// Why a parameter does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dead {
    /// `pw` without the pulse wave.
    NotPulse,
    /// `position` or `table` without the table wave.
    NotTable,
    /// `sync` on the first enabled oscillator, the sync master itself.
    SyncMaster,
    /// A generator at level 0 with no route on its level.
    Silent,
    /// A modulator that drives nothing audible (and is not the amplitude envelope).
    Unrouted,
    /// Keytrack without two notes an octave apart (with one note it equals a cutoff move).
    NeedsOctave,
    /// Velocity amounts without two velocities.
    OneVelocity,
    /// Key tracking routes without two keys.
    OneKey,
    /// Glide or glide mode without a phrase that glides.
    NoGlide,
    /// Voice mode, polyphony or LFO scope without notes that tell them apart.
    OneNote,
    /// An attack longer than the shortest held note.
    BeyondHold,
    /// A release longer than the phrase's tail.
    BeyondTail,
    /// A route from the mod wheel or a macro: the phrase never moves them, so it is a fixed offset.
    NoControls,
    /// A route of depth 0.
    ZeroDepth,
    /// A route whose source or destination is hidden or missing.
    HiddenEnd,
}

impl Dead {
    /// A stable lowercase name (`dead:not_pulse`).
    pub const fn name(self) -> &'static str {
        match self {
            Dead::NotPulse => "dead:not_pulse",
            Dead::NotTable => "dead:not_table",
            Dead::SyncMaster => "dead:sync_master",
            Dead::Silent => "dead:silent",
            Dead::Unrouted => "dead:unrouted",
            Dead::NeedsOctave => "dead:needs_octave",
            Dead::OneVelocity => "dead:one_velocity",
            Dead::OneKey => "dead:one_key",
            Dead::NoGlide => "dead:no_glide",
            Dead::OneNote => "dead:one_note",
            Dead::BeyondHold => "dead:beyond_hold",
            Dead::BeyondTail => "dead:beyond_tail",
            Dead::NoControls => "dead:no_controls",
            Dead::ZeroDepth => "dead:zero_depth",
            Dead::HiddenEnd => "dead:hidden_end",
        }
    }
}

/// The observability of every parameter, route and enable flag of one patch under one phrase. Made by
/// [`observable`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    params: BTreeMap<ParamAddr, Visibility>,
    routes: BTreeMap<RouteAddr, Visibility>,
    modules: BTreeMap<ModuleId, Visibility>,
}

impl Mask {
    /// The visibility of a parameter; `None` if the patch has no such parameter.
    pub fn param(&self, addr: &ParamAddr) -> Option<Visibility> {
        self.params.get(addr).copied()
    }

    /// Whether a parameter exists and is [`Visibility::Observable`].
    pub fn is_observable(&self, addr: &ParamAddr) -> bool {
        self.param(addr).is_some_and(Visibility::is_observable)
    }

    /// The visibility of a route's depth; `None` if the patch has no such route.
    pub fn route(&self, route: &RouteAddr) -> Option<Visibility> {
        self.routes.get(route).copied()
    }

    /// The visibility of a module's enable flag (would switching it change the sound?); `None` if absent.
    pub fn module(&self, id: &ModuleId) -> Option<Visibility> {
        self.modules.get(id).copied()
    }

    /// Every parameter of the patch with its visibility, in address order.
    pub fn params(&self) -> impl Iterator<Item = (&ParamAddr, Visibility)> {
        self.params.iter().map(|(a, v)| (a, *v))
    }

    /// Every route of the patch with its visibility, in address order.
    pub fn routes(&self) -> impl Iterator<Item = (&RouteAddr, Visibility)> {
        self.routes.iter().map(|(a, v)| (a, *v))
    }

    /// Every module's enable flag with its visibility, in id order.
    pub fn modules(&self) -> impl Iterator<Item = (&ModuleId, Visibility)> {
        self.modules.iter().map(|(a, v)| (a, *v))
    }

    /// The observable parameters, in address order.
    pub fn observable_params(&self) -> impl Iterator<Item = &ParamAddr> {
        self.params
            .iter()
            .filter(|(_, v)| v.is_observable())
            .map(|(a, _)| a)
    }
}

/// The observability mask of `patch` heard through `phrase` (see the [module docs](self)). Pure, parameter space
/// only; meant for valid patches (an invalid one gets a best-effort mask).
pub fn observable(patch: &Patch, phrase: &Phrase) -> Mask {
    observable_view(patch, phrase.view())
}

/// [`observable`] for a phrase built at run time (lane F3S, R3b: `ds1`'s sampled phrases, a create job's inferred
/// performance), whose notes are not `'static`. One implementation: a table phrase and a view with the same notes
/// get the same mask.
pub fn observable_view(patch: &Patch, phrase: PhraseView<'_>) -> Mask {
    let mut mask = Mask {
        params: BTreeMap::new(),
        routes: BTreeMap::new(),
        modules: BTreeMap::new(),
    };
    let first_generator = patch.generators.iter().find(|m| m.enabled).map(|m| &m.id);
    let first_osc = patch
        .generators
        .iter()
        .find(|m| m.enabled && m.kind == ModuleKind::Osc)
        .map(|m| &m.id);

    // The built-ins.
    for (i, spec) in enth_patch::catalogue::VOICE_PARAMS.iter().enumerate() {
        let v = voice_visibility(patch, phrase, i);
        mask.params.insert(
            ParamAddr::new(Target::Voice, spec.key).expect("voice key"),
            v,
        );
    }
    for spec in &enth_patch::catalogue::MASTER_PARAMS {
        mask.params.insert(
            ParamAddr::new(Target::Master, spec.key).expect("master key"),
            Visibility::Gauge,
        );
    }

    // Generators and effects: their visibility does not depend on the modulators.
    for section in [Section::Generators, Section::VoiceFx, Section::Fx] {
        for m in patch.section(section) {
            mask.modules.insert(m.id.clone(), Visibility::Observable);
            for (i, spec) in m.kind.params().iter().enumerate() {
                let v = if m.enabled {
                    module_visibility(patch, phrase, m, i, first_generator, first_osc)
                } else {
                    Visibility::Disabled
                };
                mask.params.insert(addr(m, spec.key), v);
            }
        }
    }

    // Modulators: live when they are the amplitude envelope or drive something audible, to a fixpoint (a modulator
    // may drive another modulator).
    let mut live: Vec<&ModuleId> = Vec::new();
    loop {
        let before = live.len();
        for m in patch.modulators.iter().filter(|m| m.enabled) {
            if live.contains(&&m.id) {
                continue;
            }
            if patch.voice.amp_env == m.id || drives_something(patch, phrase, &mask, &live, &m.id) {
                live.push(&m.id);
            }
        }
        if live.len() == before {
            break;
        }
    }
    for m in &patch.modulators {
        let would_live =
            patch.voice.amp_env == m.id || drives_something(patch, phrase, &mask, &live, &m.id);
        mask.modules.insert(
            m.id.clone(),
            if would_live {
                Visibility::Observable
            } else {
                Visibility::Dead(Dead::Unrouted)
            },
        );
        for (i, spec) in m.kind.params().iter().enumerate() {
            let v = if !m.enabled {
                Visibility::Disabled
            } else if !live.contains(&&m.id) {
                Visibility::Dead(Dead::Unrouted)
            } else {
                modulator_visibility(phrase, m, i)
            };
            mask.params.insert(addr(m, spec.key), v);
        }
    }

    // Routes, now that every destination is classified.
    for r in &patch.routes {
        let v = route_visibility(patch, phrase, &mask, r.depth, &r.src, &r.dst);
        mask.routes.insert(
            RouteAddr {
                src: r.src.clone(),
                dst: r.dst.clone(),
            },
            v,
        );
    }
    mask
}

fn addr(m: &ModuleInstance, key: &str) -> ParamAddr {
    ParamAddr::module(m.id.clone(), key).expect("catalogue key")
}

/// Whether a value is audible on its own: observable or a gauge (a route onto a gauge, e.g. tremolo on a lone
/// oscillator's level, is still heard).
const fn heard(v: Visibility) -> bool {
    matches!(v, Visibility::Observable | Visibility::Gauge)
}

fn voice_visibility(patch: &Patch, phrase: PhraseView<'_>, index: usize) -> Visibility {
    let mode = VoiceMode::from_value(patch.voice.get(VoiceParam::Mode)).unwrap_or(VoiceMode::Poly);
    let glide_mode =
        GlideMode::from_value(patch.voice.get(VoiceParam::GlideMode)).unwrap_or(GlideMode::Always);
    let glide_heard = phrase.has_pitch_change()
        && (glide_mode == GlideMode::Always || phrase.has_overlapping_pitch_change());
    match VoiceParam::ALL[index] {
        VoiceParam::Mode => {
            if phrase.notes.len() >= 2 {
                Visibility::Observable
            } else {
                Visibility::Dead(Dead::OneNote)
            }
        }
        VoiceParam::Polyphony => {
            let poly = patch.voice.get(VoiceParam::Polyphony);
            if mode == VoiceMode::Poly && phrase.max_simultaneous() as f32 > poly {
                Visibility::Observable
            } else {
                Visibility::Dead(Dead::OneNote)
            }
        }
        VoiceParam::Glide => {
            if glide_heard {
                Visibility::Observable
            } else {
                Visibility::Dead(Dead::NoGlide)
            }
        }
        VoiceParam::GlideMode => {
            // The two modes differ only on a pitch change after every key was released.
            if glide_heard
                && patch.voice.get(VoiceParam::Glide) > 0.0
                && phrase.has_detached_pitch_change()
            {
                Visibility::Observable
            } else {
                Visibility::Dead(Dead::NoGlide)
            }
        }
        VoiceParam::Pan => Visibility::Gauge,
    }
}

fn module_visibility(
    patch: &Patch,
    phrase: PhraseView<'_>,
    m: &ModuleInstance,
    index: usize,
    first_generator: Option<&ModuleId>,
    first_osc: Option<&ModuleId>,
) -> Visibility {
    let key = m.kind.params()[index].key;
    let is_first_generator = first_generator == Some(&m.id);
    // A silent generator hides everything but its level (unless a route moves the level).
    let silent = |level_key: &str| {
        m.get(level_key) == Some(0.0)
            && !patch.routes.iter().any(|r| {
                r.depth != 0.0 && r.dst.module_id() == Some(&m.id) && r.dst.param() == level_key
            })
    };
    match m.kind {
        ModuleKind::Osc => {
            let wave = Wave::from_value(m.values[OscParam::Wave.index()]);
            match OscParam::ALL[index] {
                OscParam::Level if is_first_generator => Visibility::Gauge,
                OscParam::Level => Visibility::Observable,
                _ if silent("level") => Visibility::Dead(Dead::Silent),
                OscParam::PhaseMode | OscParam::Phase | OscParam::Seed => Visibility::Nuisance,
                OscParam::Pw if wave != Some(Wave::Pulse) => Visibility::Dead(Dead::NotPulse),
                OscParam::Position | OscParam::Table if wave != Some(Wave::Table) => {
                    Visibility::Dead(Dead::NotTable)
                }
                OscParam::Sync if first_osc == Some(&m.id) => Visibility::Dead(Dead::SyncMaster),
                _ => Visibility::Observable,
            }
        }
        ModuleKind::Noise => match key {
            "level" if is_first_generator => Visibility::Gauge,
            "level" => Visibility::Observable,
            _ if silent("level") => Visibility::Dead(Dead::Silent),
            _ => Visibility::Observable,
        },
        ModuleKind::Svf => match key {
            "keytrack" if !phrase.has_octave_pair() => Visibility::Dead(Dead::NeedsOctave),
            _ => Visibility::Observable,
        },
        ModuleKind::Comp => match key {
            "makeup" => Visibility::Gauge,
            _ => Visibility::Observable,
        },
        ModuleKind::Reverb => match key {
            "dry" => Visibility::Gauge,
            _ => Visibility::Observable,
        },
        // Modulators are classified by `modulator_visibility`.
        ModuleKind::Adsr | ModuleKind::Lfo => Visibility::Observable,
    }
}

fn modulator_visibility(phrase: PhraseView<'_>, m: &ModuleInstance, index: usize) -> Visibility {
    match m.kind {
        ModuleKind::Adsr => match AdsrParam::ALL[index] {
            AdsrParam::Velocity if !phrase.has_two_velocities() => {
                Visibility::Dead(Dead::OneVelocity)
            }
            AdsrParam::Attack if m.values[index] > phrase.min_hold_ms() as f32 => {
                Visibility::Dead(Dead::BeyondHold)
            }
            AdsrParam::Release if m.values[index] > phrase.tail_ms() as f32 => {
                Visibility::Dead(Dead::BeyondTail)
            }
            _ => Visibility::Observable,
        },
        ModuleKind::Lfo => match LfoParam::ALL[index] {
            LfoParam::PhaseMode | LfoParam::Phase | LfoParam::Seed => Visibility::Nuisance,
            LfoParam::Scope if phrase.notes.len() < 2 => Visibility::Dead(Dead::OneNote),
            _ => Visibility::Observable,
        },
        _ => Visibility::Observable,
    }
}

/// Whether modulator `id` has a route of non-zero depth onto something heard: a parameter of a generator, an
/// effect or a built-in that is heard, or a heard parameter of a modulator already known to be live.
fn drives_something(
    patch: &Patch,
    phrase: PhraseView<'_>,
    mask: &Mask,
    live: &[&ModuleId],
    id: &ModuleId,
) -> bool {
    patch.routes.iter().any(|r| {
        r.depth != 0.0
            && matches!(&r.src, ModSource::Module(s) if s == id)
            && match r.dst.module_id() {
                Some(target) if patch.modulators.iter().any(|m| &m.id == target) => {
                    live.contains(&target)
                        && patch.find(target).is_some_and(|(_, m)| {
                            m.kind
                                .param_index(r.dst.param())
                                .is_some_and(|i| heard(modulator_visibility(phrase, m, i)))
                        })
                }
                _ => mask.param(&r.dst).is_some_and(heard),
            }
    })
}

fn route_visibility(
    patch: &Patch,
    phrase: PhraseView<'_>,
    mask: &Mask,
    depth: f32,
    src: &ModSource,
    dst: &ParamAddr,
) -> Visibility {
    if depth == 0.0 {
        return Visibility::Dead(Dead::ZeroDepth);
    }
    match src {
        ModSource::Module(id) => match patch.find(id) {
            None => return Visibility::Dead(Dead::HiddenEnd),
            Some((_, m)) if !m.enabled => return Visibility::Disabled,
            Some(_) => {}
        },
        ModSource::Velocity if !phrase.has_two_velocities() => {
            return Visibility::Dead(Dead::OneVelocity);
        }
        ModSource::Key if !phrase.has_pitch_change() => return Visibility::Dead(Dead::OneKey),
        ModSource::ModWheel | ModSource::Macro(_) => return Visibility::Dead(Dead::NoControls),
        ModSource::Velocity | ModSource::Key => {}
    }
    // Every destination is classified by now, modulators included (a modulator that is not live is `Unrouted`).
    match mask.param(dst) {
        Some(Visibility::Disabled) => Visibility::Disabled,
        Some(v) if heard(v) => Visibility::Observable,
        _ => Visibility::Dead(Dead::HiddenEnd),
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::model::PhraseNote;
    use crate::phrases::{self, TABLE};
    use crate::random::{start, target};
    use crate::spec::{Category, RandomSpec};

    /// A run-time phrase (owned notes) with a table phrase's notes gets exactly the table phrase's mask.
    #[test]
    fn a_view_with_a_table_phrases_notes_gets_its_mask() {
        for e in &TABLE {
            let notes: Vec<PhraseNote> = e.phrase.notes.to_vec();
            let view = PhraseView {
                notes: &notes,
                length_ms: e.phrase.length_ms,
            };
            assert_eq!(view.validate(), Ok(()));
            assert_eq!(view.performance(), e.phrase.performance());
            for seed in 0..12u64 {
                let spec = RandomSpec {
                    generator: 1,
                    seed: 7_300_000 + seed,
                    difficulty: 100,
                    amounts: [None; Category::COUNT],
                    musical: [100, 50, 0][seed as usize % 3],
                    template: 0,
                };
                let patch = target(&spec).unwrap().patch;
                assert_eq!(
                    observable_view(&patch, view),
                    observable(&patch, &e.phrase),
                    "{} seed {seed}",
                    e.name
                );
            }
        }
    }

    /// A run-time phrase reveals what its own facts reveal: a 2 s attack is heard under a 2.8 s hold (not under the
    /// held phrase's 1.5 s), and a release under a short note with a long tail.
    #[test]
    fn a_run_time_phrase_reveals_what_its_notes_reveal() {
        let mut p = start(0).unwrap();
        let attack: ParamAddr = "env1.attack".parse().unwrap();
        let release: ParamAddr = "env1.release".parse().unwrap();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetParam {
                addr: attack.clone(),
                value: 2000.0,
            },
        )
        .unwrap();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetParam {
                addr: release.clone(),
                value: 2000.0,
            },
        )
        .unwrap();
        let held = phrases::phrase(phrases::HELD).unwrap();
        assert_eq!(
            observable(&p, held).param(&attack),
            Some(Visibility::Dead(Dead::BeyondHold))
        );
        let pad = [PhraseNote {
            key: 52,
            velocity: 90,
            start_ms: 0,
            hold_ms: 2800,
        }];
        let m = observable_view(
            &p,
            PhraseView {
                notes: &pad,
                length_ms: 4000,
            },
        );
        assert!(m.is_observable(&attack));
        assert_eq!(m.param(&release), Some(Visibility::Dead(Dead::BeyondTail)));
        let short = [PhraseNote {
            hold_ms: 300,
            ..pad[0]
        }];
        let m = observable_view(
            &p,
            PhraseView {
                notes: &short,
                length_ms: 3000,
            },
        );
        assert!(m.is_observable(&release));
        assert_eq!(m.param(&attack), Some(Visibility::Dead(Dead::BeyondHold)));
    }
}
