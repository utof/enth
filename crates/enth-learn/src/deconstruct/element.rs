//! **Owner: lane DEC.** Elements: what a preset adds to its *base*, as a partition of their differences, and the
//! projection that rebuilds any subset of them (docs/learn.md, "Deconstruct lessons (lane DEC)").
//!
//! **The base** is the preset's own Init: the same modules with the same ids (so every selector, route and link
//! resolves in every step), each at its catalogue defaults, with
//!
//! - one generator sounding: the first enabled one with a level above 0 (a noise patch starts from noise, not from a
//!   saw), every other generator and every effect switched off;
//! - the filters' and modulators' enable flags as the preset has them;
//! - no routes; the `voice` built-ins at their defaults (the amplitude envelope stays the preset's);
//! - the preset's master level and macro positions, which are a gauge and a performance, never an element.
//!
//! A module the preset leaves switched off adds nothing, so its values stay as the preset has them.
//!
//! **Elements** (report 01 §6: "each enabled module, each route, and each group of parameters that differ from
//! Init"): each generator (its switch and its values); each per-voice effect in groups (a filter's tone, its key
//! tracking, its drive); the amplitude envelope; each route, together with its source modulator's values; the
//! voice mode with glide, and the voice pan; each effect (its switch and its values). A modulator's values are
//! shared by all its routes: a part shared by several elements is included when any of them is.
//!
//! **Projection.** Every patch the lesson uses is the preset with the parts of the excluded elements turned back to
//! the base through ops ([`project`]): an ablation is "everything but one", step *k*'s target is "the base's fine
//! detail plus elements 1 to *k*", and the last target is the preset itself, routes in its own summation order.

use std::collections::BTreeSet;

use enth_challenge::v2::Selector;
use enth_patch::catalogue::{ModuleKind, VoiceParam};
use enth_patch::{ModSource, ModuleId, Op, ParamAddr, Patch, RouteAddr, Section, Target, apply};

/// Where an element sits in the signal flow: the order steps are taught in (report 01 §6, "source, filter, amp env,
/// modulators, effects, macros"; the voice settings go between the modulators and the effects, as the studies' cards
/// in content/lessons/PLAN.md place the glide).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flow {
    /// A generator.
    Source,
    /// A per-voice effect (the filter).
    Filter,
    /// The amplitude envelope.
    Amp,
    /// A route from a modulator or a built-in source (an envelope, an LFO, key, velocity, the mod wheel).
    Modulation,
    /// The voice mode, glide and pan.
    Voice,
    /// A global effect (the reverb, the compressor).
    Effects,
    /// A route from a macro knob.
    Macros,
}

impl Flow {
    /// A stable lowercase name, for reports.
    pub fn name(self) -> &'static str {
        match self {
            Flow::Source => "source",
            Flow::Filter => "filter",
            Flow::Amp => "amp",
            Flow::Modulation => "modulation",
            Flow::Voice => "voice",
            Flow::Effects => "effects",
            Flow::Macros => "macros",
        }
    }
}

/// One difference between a preset and its base.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    /// A parameter's value.
    Param(ParamAddr),
    /// A module's enable flag.
    Enable(ModuleId),
    /// A route (with its depth).
    Route(RouteAddr),
}

impl Part {
    /// The selector that lets a player change this part: the parameter, the route, or the whole module for a switch.
    pub fn selector(&self) -> Selector {
        match self {
            Part::Param(a) => Selector::Param(a.clone()),
            Part::Enable(id) => Selector::Module(Target::Module(id.clone())),
            Part::Route(r) => Selector::Route(r.clone()),
        }
    }

    /// Whether `a` and `b` agree on this part.
    pub fn same_in(&self, a: &Patch, b: &Patch) -> bool {
        match self {
            Part::Param(addr) => match (a.value(addr), b.value(addr)) {
                (Some(x), Some(y)) => x.to_bits() == y.to_bits(),
                (x, y) => x.is_none() && y.is_none(),
            },
            Part::Enable(id) => enabled(a, id) == enabled(b, id),
            Part::Route(r) => route_depth(a, r).map(f32::to_bits) == route_depth(b, r).map(f32::to_bits),
        }
    }
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Part::Param(a) => write!(f, "{a}"),
            Part::Enable(id) => write!(f, "{id} on"),
            Part::Route(r) => write!(f, "{r}"),
        }
    }
}

/// One thing the preset adds to its base, taught (or folded in as fine detail) as a whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// A stable id within the preset: `osc2`, `filter1`, `filter1.keytrack`, `env1`, `voice`, `voice.pan`,
    /// `reverb1`, `env2 -> filter1.cutoff`. Measurements are keyed by it.
    pub id: String,
    /// Where it sits in the signal flow.
    pub flow: Flow,
    /// What it changes. A modulator's values appear in every route element of that modulator.
    pub parts: Vec<Part>,
    /// The control that names it in text (the cutoff of a filter's tone, a generator's wave).
    pub main: Option<ParamAddr>,
    /// A plain-language name: "the second oscillator", "the filter envelope", "legato and glide".
    pub label: String,
}

impl Element {
    /// A slug of the id for step and file names: lower-case letters, digits and single `-`, at most `max` bytes.
    pub fn slug(&self, max: usize) -> String {
        slug(&self.id, max)
    }
}

/// `text` as a slug: `[a-z0-9]` runs joined by single `-`, at most `max` bytes, never ending in `-`.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.truncate(max);
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn enabled(p: &Patch, id: &ModuleId) -> Option<bool> {
    p.find(id).map(|(_, m)| m.enabled)
}

fn route_depth(p: &Patch, r: &RouteAddr) -> Option<f32> {
    p.routes
        .iter()
        .find(|x| x.src == r.src && x.dst == r.dst)
        .map(|x| x.depth)
}

/// The first generator that sounds on its own: enabled with a level above 0; else the first enabled one.
fn main_generator(preset: &Patch) -> Option<ModuleId> {
    let level = |m: &enth_patch::ModuleInstance| m.get("level").unwrap_or(1.0);
    preset
        .generators
        .iter()
        .find(|m| m.enabled && level(m) > 0.0)
        .or_else(|| preset.generators.iter().find(|m| m.enabled))
        .map(|m| m.id.clone())
}

/// The base's enable flag for a module of the preset (see the [module docs](self)).
fn base_enabled(preset: &Patch, section: Section, id: &ModuleId, now: bool) -> bool {
    match section {
        Section::Generators => main_generator(preset).as_ref() == Some(id),
        Section::Fx => false,
        Section::VoiceFx | Section::Modulators => now,
    }
}

/// The base's value of a parameter: its catalogue default (for the built-in `voice` too).
fn base_value(preset: &Patch, addr: &ParamAddr) -> Option<f32> {
    preset.spec(addr).map(|s| s.default)
}

/// The parameter groups of a per-voice effect: a filter's tone, key tracking and drive are taught apart.
fn voice_fx_groups(kind: ModuleKind) -> &'static [&'static [&'static str]] {
    match kind {
        ModuleKind::Svf => &[&["mode", "cutoff", "resonance"], &["keytrack"], &["drive"]],
        _ => &[],
    }
}

/// The modulator's parameters that differ from their defaults: shared by every route from it.
fn modulator_parts(preset: &Patch, id: &ModuleId) -> Vec<Part> {
    let Some((_, m)) = preset.find(id) else {
        return Vec::new();
    };
    module_param_parts(preset, id, m.kind, None)
}

/// The parameters of a module (all of them, or `only` these keys) whose value differs from the base's.
fn module_param_parts(preset: &Patch, id: &ModuleId, kind: ModuleKind, only: Option<&[&str]>) -> Vec<Part> {
    kind.params()
        .iter()
        .filter(|spec| only.is_none_or(|keys| keys.contains(&spec.key)))
        .filter_map(|spec| ParamAddr::module(id.clone(), spec.key).ok())
        .filter(|addr| {
            matches!((preset.value(addr), base_value(preset, addr)), (Some(v), Some(b)) if v.to_bits() != b.to_bits())
        })
        .map(Part::Param)
        .collect()
}

/// Every element of `preset`, in patch order (generators, per-voice effects, the amplitude envelope, routes in their
/// summation order, the voice, effects). An element always has at least one part, and every difference between the
/// preset and its [`base`] belongs to one (a modulator's values to every route from it).
pub fn elements(preset: &Patch) -> Vec<Element> {
    let mut out = Vec::new();
    let amp = preset.voice.amp_env.clone();
    // Generators: the switch and every value.
    for m in &preset.generators {
        if !m.enabled {
            continue;
        }
        let mut parts = Vec::new();
        if !base_enabled(preset, Section::Generators, &m.id, m.enabled) {
            parts.push(Part::Enable(m.id.clone()));
        }
        parts.extend(module_param_parts(preset, &m.id, m.kind, None));
        if parts.is_empty() {
            continue;
        }
        let main = ["wave", "color", "semi", "fine", "level"]
            .iter()
            .filter_map(|k| ParamAddr::module(m.id.clone(), k).ok())
            .find(|a| parts.contains(&Part::Param(a.clone())))
            .or_else(|| {
                parts.iter().find_map(|p| match p {
                    Part::Param(a) => Some(a.clone()),
                    _ => None,
                })
            });
        out.push(Element {
            id: m.id.to_string(),
            flow: Flow::Source,
            label: generator_label(preset, &m.id, m.kind),
            parts,
            main,
        });
    }
    // Per-voice effects: in groups.
    for m in &preset.voice_fx {
        if !m.enabled {
            continue;
        }
        let groups = voice_fx_groups(m.kind);
        if groups.is_empty() {
            let parts = module_param_parts(preset, &m.id, m.kind, None);
            if !parts.is_empty() {
                out.push(Element {
                    id: m.id.to_string(),
                    flow: Flow::Filter,
                    main: first_param(&parts),
                    label: format!("the {}", m.kind.spec().name.to_lowercase()),
                    parts,
                });
            }
            continue;
        }
        for (g, keys) in groups.iter().enumerate() {
            let parts = module_param_parts(preset, &m.id, m.kind, Some(keys));
            if parts.is_empty() {
                continue;
            }
            let (id, label, main_key) = match (m.kind, g) {
                (ModuleKind::Svf, 0) => (m.id.to_string(), "the filter".to_owned(), "cutoff"),
                (ModuleKind::Svf, 1) => (format!("{}.keytrack", m.id), "the filter's key tracking".to_owned(), "keytrack"),
                (ModuleKind::Svf, _) => (format!("{}.drive", m.id), "the filter's drive".to_owned(), "drive"),
                _ => (format!("{}.{}", m.id, keys[0]), format!("the {}", m.kind.spec().name.to_lowercase()), keys[0]),
            };
            let main = ParamAddr::module(m.id.clone(), main_key)
                .ok()
                .filter(|a| parts.contains(&Part::Param(a.clone())))
                .or_else(|| first_param(&parts));
            out.push(Element {
                id,
                flow: Flow::Filter,
                parts,
                main,
                label,
            });
        }
    }
    // The amplitude envelope.
    if let Some((_, m)) = preset.find(&amp) {
        let parts = module_param_parts(preset, &amp, m.kind, None);
        if !parts.is_empty() {
            out.push(Element {
                id: amp.to_string(),
                flow: Flow::Amp,
                main: first_param(&parts),
                label: "the amplitude envelope".to_owned(),
                parts,
            });
        }
    }
    // Routes, in summation order, each with its source modulator's values.
    for r in &preset.routes {
        let addr = RouteAddr {
            src: r.src.clone(),
            dst: r.dst.clone(),
        };
        let mut parts = vec![Part::Route(addr.clone())];
        if let ModSource::Module(src) = &r.src {
            parts.extend(modulator_parts(preset, src));
        }
        let flow = if matches!(r.src, ModSource::Macro(_)) {
            Flow::Macros
        } else {
            Flow::Modulation
        };
        out.push(Element {
            id: addr.to_string(),
            flow,
            label: route_label(preset, &addr),
            main: Some(r.dst.clone()),
            parts,
        });
    }
    // The voice: mode with glide, then pan.
    let voice = |p: VoiceParam| ParamAddr::voice(p);
    let mode_parts: Vec<Part> = [
        VoiceParam::Mode,
        VoiceParam::Polyphony,
        VoiceParam::Glide,
        VoiceParam::GlideMode,
    ]
    .into_iter()
    .map(voice)
    .filter(|a| differs_from_base(preset, a))
    .map(Part::Param)
    .collect();
    if !mode_parts.is_empty() {
        out.push(Element {
            id: "voice".to_owned(),
            flow: Flow::Voice,
            label: voice_label(preset),
            main: Some(voice(VoiceParam::Mode)),
            parts: mode_parts,
        });
    }
    let pan = voice(VoiceParam::Pan);
    if differs_from_base(preset, &pan) {
        out.push(Element {
            id: "voice.pan".to_owned(),
            flow: Flow::Voice,
            label: "the voice pan".to_owned(),
            main: Some(pan.clone()),
            parts: vec![Part::Param(pan)],
        });
    }
    // Effects: the switch and every value.
    for m in &preset.fx {
        if !m.enabled {
            continue;
        }
        let mut parts = vec![Part::Enable(m.id.clone())];
        parts.extend(module_param_parts(preset, &m.id, m.kind, None));
        out.push(Element {
            id: m.id.to_string(),
            flow: Flow::Effects,
            main: first_param(&parts),
            label: format!("the {}", m.kind.spec().name.to_lowercase()),
            parts,
        });
    }
    out
}

fn first_param(parts: &[Part]) -> Option<ParamAddr> {
    parts.iter().find_map(|p| match p {
        Part::Param(a) => Some(a.clone()),
        _ => None,
    })
}

fn differs_from_base(preset: &Patch, addr: &ParamAddr) -> bool {
    matches!((preset.value(addr), base_value(preset, addr)), (Some(v), Some(b)) if v.to_bits() != b.to_bits())
}

fn ordinal(n: usize) -> &'static str {
    ["first", "second", "third", "fourth", "fifth"]
        .get(n)
        .copied()
        .unwrap_or("next")
}

fn generator_label(preset: &Patch, id: &ModuleId, kind: ModuleKind) -> String {
    match kind {
        ModuleKind::Noise => "the noise".to_owned(),
        _ => {
            let n = preset
                .generators
                .iter()
                .filter(|m| m.kind == kind)
                .position(|m| &m.id == id)
                .unwrap_or(0);
            format!("the {} oscillator", ordinal(n))
        }
    }
}

fn voice_label(preset: &Patch) -> String {
    let get = |p: VoiceParam| preset.voice.get(p);
    let mode = enth_patch::catalogue::VoiceMode::from_value(get(VoiceParam::Mode));
    let glide = get(VoiceParam::Glide) > 0.0;
    match (mode, glide) {
        (Some(enth_patch::catalogue::VoiceMode::Legato), true) => "legato and glide".to_owned(),
        (Some(enth_patch::catalogue::VoiceMode::Legato), false) => "legato mode".to_owned(),
        (Some(enth_patch::catalogue::VoiceMode::Mono), true) => "mono mode and glide".to_owned(),
        (Some(enth_patch::catalogue::VoiceMode::Mono), false) => "mono mode".to_owned(),
        (_, true) => "glide".to_owned(),
        _ => "the voice settings".to_owned(),
    }
}

/// A route's name by what it does: "the filter envelope", "vibrato", "tremolo", "pan by pitch", "the Bright macro".
fn route_label(preset: &Patch, r: &RouteAddr) -> String {
    let dst_kind = match r.dst.target() {
        Target::Module(id) => preset.find(id).map(|(_, m)| m.kind),
        _ => None,
    };
    let param = r.dst.param();
    let dst_is = |k: ModuleKind, keys: &[&str]| dst_kind == Some(k) && keys.contains(&param);
    match &r.src {
        ModSource::Macro(i) => match preset.macros[i.get()].name.as_deref() {
            Some(name) => format!("the {name} macro"),
            None => format!("macro {}", i.one_based()),
        },
        ModSource::Key if param == "pan" => "pan by pitch".to_owned(),
        ModSource::Key => format!("key tracking on {}", r.dst),
        ModSource::Velocity => format!("velocity on {}", r.dst),
        ModSource::ModWheel => format!("the mod wheel on {}", r.dst),
        ModSource::Module(src) => {
            let src_kind = preset.find(src).map(|(_, m)| m.kind);
            let is_env = src_kind == Some(ModuleKind::Adsr);
            if dst_is(ModuleKind::Svf, &["cutoff"]) {
                if is_env { "the filter envelope" } else { "the filter LFO" }.to_owned()
            } else if dst_is(ModuleKind::Osc, &["semi", "fine"]) {
                if is_env { "the pitch envelope" } else { "vibrato" }.to_owned()
            } else if dst_kind.is_some_and(|k| matches!(k, ModuleKind::Osc | ModuleKind::Noise)) && param == "level" {
                let what = generator_label(preset, r.dst.module_id().unwrap_or(src), dst_kind.unwrap_or(ModuleKind::Osc));
                if is_env {
                    format!("an envelope on {what}")
                } else {
                    format!("tremolo on {what}")
                }
            } else if r.dst.target() == &Target::Voice && param == "pan" {
                if is_env { "a pan envelope" } else { "auto-pan" }.to_owned()
            } else {
                format!("{src} on {}", r.dst)
            }
        }
    }
}

/// The preset's base (see the [module docs](self)): [`project`] with nothing included.
pub fn base(preset: &Patch) -> Patch {
    let els = elements(preset);
    project(preset, &els, &BTreeSet::new())
}

/// `preset` with every part of the elements not in `included` turned back to the base, through ops (`SetParam`,
/// `SetEnabled`, `RemoveRoute`), so routes keep the preset's summation order. A part shared by several elements stays
/// when any of them is included. The metadata stays the preset's; the caller names the result.
pub fn project(preset: &Patch, elements: &[Element], included: &BTreeSet<usize>) -> Patch {
    let keep: BTreeSet<&Part> = included
        .iter()
        .filter_map(|i| elements.get(*i))
        .flat_map(|e| e.parts.iter())
        .collect();
    let drop: BTreeSet<&Part> = elements
        .iter()
        .flat_map(|e| e.parts.iter())
        .filter(|p| !keep.contains(p))
        .collect();
    let mut out = preset.clone();
    for part in drop {
        let op = match part {
            Part::Param(addr) => base_value(preset, addr).map(|value| Op::SetParam {
                addr: addr.clone(),
                value,
            }),
            Part::Enable(id) => preset.find(id).map(|(pos, m)| Op::SetEnabled {
                id: id.clone(),
                enabled: base_enabled(preset, pos.section, id, m.enabled),
            }),
            Part::Route(r) => Some(Op::RemoveRoute { route: r.clone() }),
        };
        if let Some(op) = op {
            // Every op here names something the preset has with a valid value, so it applies; a refusal would
            // leave that part as the preset has it, which the planner's tests would show.
            let _ = apply(&mut out, op);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn patch(ops: &[&str]) -> Patch {
        let mut p = starter();
        for line in ops {
            let (lhs, value) = line.split_once('=').unwrap();
            let lhs = lhs.trim();
            if let Ok(route) = lhs.parse::<RouteAddr>() {
                let index = p.routes.len();
                apply(
                    &mut p,
                    Op::InsertRoute {
                        index,
                        route: enth_patch::Route {
                            src: route.src,
                            dst: route.dst,
                            depth: value.trim().parse().unwrap(),
                        },
                    },
                )
                .unwrap();
            } else if let Some(id) = lhs.strip_suffix(".enabled") {
                apply(
                    &mut p,
                    Op::SetEnabled {
                        id: ModuleId::new(id).unwrap(),
                        enabled: value.trim() == "on",
                    },
                )
                .unwrap();
            } else {
                let addr: ParamAddr = lhs.parse().unwrap();
                let spec = p.spec(&addr).unwrap();
                let value = spec.parse(value.trim()).unwrap();
                apply(&mut p, Op::SetParam { addr, value }).unwrap();
            }
        }
        p
    }

    /// The starter: every P1 module, osc 2, noise, compressor and reverb off.
    pub(crate) fn starter() -> Patch {
        enth_patch::file::load(include_str!("../../../../xtask/src/preset/starter.patch.json"))
            .unwrap()
            .patch
    }

    #[test]
    fn the_starter_has_no_elements_and_is_its_own_base() {
        let s = starter();
        assert!(elements(&s).is_empty());
        assert_eq!(base(&s), s);
    }

    #[test]
    fn elements_partition_the_differences_and_projection_rebuilds_the_preset() {
        let p = patch(&[
            "osc2.enabled=on",
            "osc2.wave=pulse",
            "osc2.semi=12",
            "filter1.cutoff=320",
            "filter1.resonance=55",
            "filter1.keytrack=40",
            "env1.decay=650",
            "env2.sustain=0",
            "env2 -> filter1.cutoff=45",
            "macro1 -> filter1.cutoff=15",
            "voice.mode=legato",
            "voice.glide=70",
            "reverb1.enabled=on",
            "reverb1.wet=-16",
        ]);
        let els = elements(&p);
        let ids: Vec<&str> = els.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "osc2",
                "filter1",
                "filter1.keytrack",
                "env1",
                "env2 -> filter1.cutoff",
                "macro1 -> filter1.cutoff",
                "voice",
                "reverb1"
            ]
        );
        let all: BTreeSet<usize> = (0..els.len()).collect();
        assert_eq!(project(&p, &els, &all), p, "everything included is the preset");
        let b = base(&p);
        assert!(elements(&b).is_empty(), "the base adds nothing");
        // The base keeps the module set, turns the extras off and drops the routes.
        assert!(b.routes.is_empty());
        assert!(!b.find(&ModuleId::new("osc2").unwrap()).unwrap().1.enabled);
        assert!(!b.find(&ModuleId::new("reverb1").unwrap()).unwrap().1.enabled);
        assert_eq!(b.master, p.master);
        // One element back on its own: only its parts differ from the base.
        let env2 = ids.iter().position(|i| *i == "env2 -> filter1.cutoff").unwrap();
        let one = project(&p, &els, &BTreeSet::from([env2]));
        assert_eq!(one.routes.len(), 1);
        let sustain: ParamAddr = "env2.sustain".parse().unwrap();
        assert_eq!(one.value(&sustain), Some(0.0), "the route brings its envelope's values");
        assert_eq!(elements(&one).len(), 1);
    }

    #[test]
    fn a_noise_patch_starts_from_noise_and_a_shared_modulator_stays_while_one_route_uses_it() {
        let p = patch(&[
            "osc1.level=0",
            "noise1.enabled=on",
            "env2.decay=70",
            "env2 -> osc1.semi=37.5",
            "env2 -> noise1.level=12",
        ]);
        let b = base(&p);
        let noise = ModuleId::new("noise1").unwrap();
        let osc1 = ModuleId::new("osc1").unwrap();
        assert!(b.find(&noise).unwrap().1.enabled, "noise is the sounding generator");
        assert!(!b.find(&osc1).unwrap().1.enabled);
        let els = elements(&p);
        let routes: Vec<usize> = els
            .iter()
            .enumerate()
            .filter(|(_, e)| e.flow == Flow::Modulation)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(routes.len(), 2);
        let decay: ParamAddr = "env2.decay".parse().unwrap();
        let only_second = project(&p, &els, &BTreeSet::from([routes[1]]));
        assert_eq!(only_second.value(&decay), Some(70.0));
        assert_eq!(only_second.routes.len(), 1);
        let neither = project(&p, &els, &BTreeSet::new());
        assert_eq!(neither.value(&decay), Some(300.0));
    }

    #[test]
    fn slugs_are_step_ids() {
        assert_eq!(slug("env2 -> filter1.cutoff", 32), "env2-filter1-cutoff");
        assert_eq!(slug("voice.pan", 32), "voice-pan");
        assert_eq!(slug("a--b-", 32), "a-b");
        assert_eq!(slug("abcdef", 3), "abc");
        assert!(crate::pack::StepId::new(&format!("build-{}", slug("macro4 -> comp1.makeup", 26))).is_ok());
    }
}
