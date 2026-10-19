//! Shared test support: raw-number generators of ops against a patch (some deliberately invalid), a builder that
//! makes patches by applying them, and bit-exact comparison.
#![allow(dead_code, reason = "each test binary uses a different part")]

use enth_params::{Kind, ParamSpec};
use enth_patch::addr::{MacroIndex, ModSource, ModuleId, ParamAddr, Position, SlotIndex, Target};
use enth_patch::catalogue::{Builtin, LfoParam, MASTER_PARAMS, ModuleKind, OscParam, VOICE_PARAMS};
use enth_patch::model::{ModuleInstance, Patch, PatchMeta, Route, Section};
use enth_patch::ops::{Op, apply};
use proptest::prelude::*;

pub use enth_patch::canon::identical;

/// Selector, two indices and 32 bits of entropy.
pub type Raw = (u8, u8, u8, u32);

pub fn raw() -> impl Strategy<Value = Raw> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u32>())
}

pub fn raws(range: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Raw>> {
    prop::collection::vec(raw(), range)
}

const STRINGS: &[&str] = &[
    "",
    "Bass",
    "Soft pluck",
    "  spaces  ",
    "日本語",
    "🎹 keys",
    "quote\"back\\slash",
    "tab\tnewline\nreturn\r",
    "\u{0}nul\u{1f}",
    "ÿ ß é ü",
    "a-very-long-label-0123456789-0123456789-0123456789-0123456789-0123456789-0123456789",
];

pub fn text(bits: u32) -> String {
    STRINGS[(bits >> 4) as usize % STRINGS.len()].to_owned()
}

pub fn opt_text(bits: u32) -> Option<String> {
    (bits & 3 != 0).then(|| text(bits))
}

fn order_key(x: f32) -> i64 {
    let b = x.to_bits();
    if b >> 31 == 0 {
        i64::from(b)
    } else {
        -i64::from(b & 0x7fff_ffff)
    }
}

fn from_order_key(k: i64) -> f32 {
    if k >= 0 {
        f32::from_bits(k as u32)
    } else {
        f32::from_bits((-k) as u32 | 0x8000_0000)
    }
}

/// An f32 uniform over the representable values of `[lo, hi]` (so log-uniform for positive ranges), from 32 bits.
pub fn any_f32_in(bits: u32, lo: f32, hi: f32) -> f32 {
    let (klo, khi) = (order_key(lo), order_key(hi));
    let span = (khi - klo + 1) as u64;
    let k = klo + ((u64::from(bits) * span) >> 32) as i64;
    from_order_key(k).clamp(lo, hi)
}

pub fn unit(bits: u32) -> f32 {
    ((bits >> 8) & 0xFF_FFFF) as f32 / 16_777_215.0
}

/// A value for a parameter: mostly valid and spread over the whole range at f32 resolution, with the edge cases
/// (bounds, neighbours of bounds, subnormals, `-0.0`, the top of a periodic range) and about 12 % invalid ones
/// (out of range, `NaN`, infinity, fractional integers, an enum index past the end).
pub fn value_for(spec: &ParamSpec, bits: u32) -> f32 {
    let mode = bits & 0x1f;
    let rest = bits >> 5;
    match spec.kind {
        Kind::Toggle => match mode {
            31 => 0.5,
            30 => 2.0,
            _ => (rest & 1) as f32,
        },
        Kind::Enum(items) => {
            let n = items.len() + usize::from(mode == 31);
            (rest as usize % n) as f32
        }
        Kind::Int { min, max } => match mode {
            31 => (max + 1) as f32,
            30 => min as f32 + 0.5,
            _ => (i64::from(min) + i64::from(rest) % (i64::from(max) - i64::from(min) + 1)) as f32,
        },
        Kind::Float { min, max, .. } => match mode {
            0 => min,
            1 => max,
            2 => spec.default,
            3 if min <= 0.0 && 0.0 <= max => -0.0,
            4 => min.next_up(),
            5 => max.next_down(),
            6 if min <= 1.0e-40 && 1.0e-40 <= max => 1.0e-40,
            7 if spec.flags.periodic => max,
            28 => max + 1.0,
            29 => f32::NAN,
            30 => f32::INFINITY,
            31 => min - 1.0,
            _ => any_f32_in(
                rest.wrapping_mul(0x9E37_79B1).rotate_left(7) ^ rest,
                min,
                max,
            ),
        },
    }
}

/// Values for every parameter of a kind, valid unless `bits` say otherwise for the first one.
pub fn values_for(kind: ModuleKind, bits: u32) -> Vec<f32> {
    kind.params()
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let b = bits.rotate_left(i as u32 * 5).wrapping_mul(0x85EB_CA6B) ^ (i as u32);
            // Keep random modules valid: only the first parameter may be an invalid value, and rarely.
            let v = value_for(spec, if i == 0 { b } else { b & !0x1f | (b & 0x7) });
            if i == 0 || spec.check(v).is_ok() {
                v
            } else {
                spec.default
            }
        })
        .collect()
}

pub fn modules(p: &Patch) -> Vec<(Position, &ModuleInstance)> {
    p.modules().collect()
}

fn pick<T>(items: &[T], i: u8) -> Option<&T> {
    if items.is_empty() {
        None
    } else {
        Some(&items[usize::from(i) % items.len()])
    }
}

/// The addresses of every modulatable parameter of the patch's modules, plus `voice.pan`.
pub fn mod_targets(p: &Patch) -> Vec<ParamAddr> {
    let mut out = Vec::new();
    for (_, m) in p.modules() {
        for spec in m.kind.params() {
            if spec.flags.modulatable {
                out.push(ParamAddr::module(m.id.clone(), spec.key).unwrap());
            }
        }
    }
    out.push("voice.pan".parse().unwrap());
    out
}

/// Module parameters whose spec satisfies `pred`.
pub fn params_where(p: &Patch, pred: impl Fn(&ParamSpec) -> bool) -> Vec<ParamAddr> {
    let mut out = Vec::new();
    for (_, m) in p.modules() {
        for spec in m.kind.params() {
            if pred(spec) {
                out.push(ParamAddr::module(m.id.clone(), spec.key).unwrap());
            }
        }
    }
    out
}

pub fn automatable_targets(p: &Patch) -> Vec<ParamAddr> {
    let mut out = Vec::new();
    for (_, m) in p.modules() {
        for spec in m.kind.params() {
            if spec.flags.automatable {
                out.push(ParamAddr::module(m.id.clone(), spec.key).unwrap());
            }
        }
    }
    for s in VOICE_PARAMS.iter().filter(|s| s.flags.automatable) {
        out.push(ParamAddr::new(Target::Voice, s.key).unwrap());
    }
    out.push("master.level".parse().unwrap());
    out
}

fn any_param(p: &Patch, a: u8, b: u8) -> Option<ParamAddr> {
    let ms = modules(p);
    let n = ms.len() + 2;
    match usize::from(a) % n {
        i if i < ms.len() => {
            let m = ms[i].1;
            let spec = &m.kind.params()[usize::from(b) % m.kind.params().len()];
            ParamAddr::module(m.id.clone(), spec.key).ok()
        }
        i if i == ms.len() => {
            let spec = &VOICE_PARAMS[usize::from(b) % VOICE_PARAMS.len()];
            ParamAddr::new(Target::Voice, spec.key).ok()
        }
        _ => {
            let spec = &MASTER_PARAMS[usize::from(b) % MASTER_PARAMS.len()];
            ParamAddr::new(Target::Master, spec.key).ok()
        }
    }
}

/// Turn raw numbers into an op against the current patch. Some ops are deliberately invalid (unknown ids,
/// out-of-range values, duplicate routes, wrong sections, removing the amp envelope…).
pub fn make_op(p: &Patch, (c, a, b, bits): Raw, fresh: &mut u32) -> Op {
    let ms = modules(p);
    let kind = ModuleKind::ALL[usize::from(a) % ModuleKind::ALL.len()];
    let unknown = || ModuleId::new("nope").unwrap();
    let some_id = |i: u8| {
        if bits & 0x70_0000 == 0 {
            return unknown();
        }
        pick(&ms, i).map_or_else(unknown, |(_, m)| m.id.clone())
    };
    // A module that routes or slots refer to, if there is one (removing it exercises the cascade).
    let linked: Vec<ModuleId> = ms
        .iter()
        .map(|(_, m)| m.id.clone())
        .filter(|id| {
            p.routes.iter().any(|r| {
                r.dst.module_id() == Some(id) || matches!(&r.src, ModSource::Module(s) if s == id)
            }) || p.slots.iter().flatten().any(|a| a.module_id() == Some(id))
        })
        .collect();
    match c % 20 {
        0..=2 => {
            *fresh += 1;
            let section = if b % 11 == 0 {
                Section::ALL[usize::from(a) % 4]
            } else {
                kind.section()
            };
            let len = p.section(kind.section()).len();
            let mut m = ModuleInstance::new(ModuleId::new(&format!("m{fresh}")).unwrap(), kind);
            m.values = values_for(kind, bits);
            m.enabled = bits & 0x100 == 0;
            m.label = if bits & 0x200 != 0 {
                opt_text(bits >> 10)
            } else {
                None
            };
            Op::AddModule {
                at: Position {
                    section,
                    index: usize::from(b) % (len + 2),
                },
                module: m,
            }
        }
        3 | 4 => Op::RemoveModule {
            id: if b % 2 == 0 {
                pick(&linked, a).cloned().unwrap_or_else(|| some_id(a))
            } else {
                some_id(a)
            },
        },
        5..=8 => match any_param(p, a, b) {
            Some(addr) => {
                let spec = p.spec(&addr).expect("generated address resolves");
                Op::SetParam {
                    value: value_for(spec, bits),
                    addr,
                }
            }
            None => Op::SetParam {
                addr: "osc1.semi".parse().unwrap(),
                value: 1.0,
            },
        },
        9..=11 => {
            let mut srcs: Vec<ModSource> = ms
                .iter()
                .filter(|(pos, _)| pos.section == Section::Modulators || bits & 0x400 != 0)
                .map(|(_, m)| ModSource::Module(m.id.clone()))
                .collect();
            srcs.extend([ModSource::Velocity, ModSource::Key, ModSource::ModWheel]);
            srcs.push(ModSource::Macro(
                MacroIndex::new(usize::from(b) % 8).unwrap(),
            ));
            let dsts = mod_targets(p);
            let mut src = pick(&srcs, a).unwrap().clone();
            let mut dst = pick(&dsts, b.wrapping_add(a)).unwrap().clone();
            if bits & 0x1800 == 0 {
                // a destination that routes cannot drive
                let others = params_where(p, |s| !s.flags.modulatable);
                if let Some(d) = pick(&others, b) {
                    dst = d.clone();
                }
            } else if bits & 0x6000 == 0
                && let Some(r) = pick(&p.routes, a)
            {
                // a route that is already there
                (src, dst) = (r.src.clone(), r.dst.clone());
            }
            let depth = match bits & 7 {
                0 => 0.0,
                1 => 100.0,
                2 => -100.0,
                3 => -0.0,
                4 => 100.5,
                _ => any_f32_in(bits >> 3, -100.0, 100.0),
            };
            let index = usize::from(b) % (p.routes.len() + 2);
            Op::InsertRoute {
                index,
                route: Route { src, dst, depth },
            }
        }
        12 => match pick(&p.routes, a) {
            Some(r) => Op::RemoveRoute {
                route: enth_patch::addr::RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
            },
            None => Op::RemoveRoute {
                route: "velocity -> voice.pan".parse().unwrap(),
            },
        },
        13 => match pick(&p.routes, a) {
            Some(r) => Op::SetDepth {
                route: enth_patch::addr::RouteAddr {
                    src: r.src.clone(),
                    dst: r.dst.clone(),
                },
                depth: any_f32_in(bits, -100.0, 100.0),
            },
            None => Op::SetMacro {
                index: MacroIndex::new(usize::from(a) % 8).unwrap(),
                value: unit(bits),
            },
        },
        14 => match pick(&ms, a) {
            Some((_, m)) => {
                let sec = if b % 5 == 0 {
                    Section::ALL[usize::from(b / 5) % 4]
                } else {
                    m.kind.section()
                };
                let len = p.section(sec).len();
                Op::Move {
                    id: m.id.clone(),
                    to: Position {
                        section: sec,
                        index: usize::from(b) % (len + usize::from(b % 7 == 0)).max(1),
                    },
                }
            }
            None => Op::SetMacro {
                index: MacroIndex::new(0).unwrap(),
                value: 0.5,
            },
        },
        15 => Op::SetEnabled {
            id: some_id(a),
            enabled: b % 3 != 0,
        },
        16 => {
            let envs: Vec<_> = ms
                .iter()
                .filter(|(_, m)| m.kind == ModuleKind::Adsr || b % 7 == 0)
                .collect();
            let id = pick(&envs, a).map_or_else(unknown, |(_, m)| m.id.clone());
            Op::SetAmpEnv { id }
        }
        17 => {
            let index = MacroIndex::new(usize::from(a) % 8).unwrap();
            if b % 2 == 0 {
                Op::SetMacro {
                    index,
                    value: if bits.trailing_zeros() >= 4 {
                        1.5
                    } else {
                        unit(bits)
                    },
                }
            } else {
                Op::SetMacroName {
                    index,
                    name: opt_text(bits),
                }
            }
        }
        18 => {
            {
                let targets = automatable_targets(p);
                let addr = if bits.trailing_zeros() >= 4 {
                    // sometimes something that is not automatable
                    pick(&params_where(p, |s| !s.flags.automatable), b).cloned()
                } else if bits & 0xf0 == 0 {
                    None
                } else {
                    pick(&targets, a).cloned()
                };
                Op::SetSlot {
                    slot: SlotIndex::new(usize::from(a) % 64).unwrap(),
                    addr,
                }
            }
        }
        _ => match b % 3 {
            0 => Op::SetLabel {
                id: some_id(a),
                label: opt_text(bits),
            },
            1 => Op::SetMeta {
                meta: PatchMeta {
                    name: text(bits),
                    author: text(bits >> 3),
                    description: text(bits >> 6),
                    tags: (0..(bits >> 12) % 3)
                        .map(|i| text(bits >> (i * 4)))
                        .collect(),
                },
            },
            _ => Op::SetParam {
                addr: ParamAddr::voice(enth_patch::catalogue::VoiceParam::Polyphony),
                value: value_for(&VOICE_PARAMS[1], bits),
            },
        },
    }
}

/// The patch every test builds on: `init()` stamped with an engine version.
pub fn base_patch() -> Patch {
    let mut p = Patch::init();
    "0.1.0".clone_into(&mut p.engine);
    p
}

/// A patch made by applying raw ops to the base patch (failures skipped).
pub fn build(raws: &[Raw]) -> Patch {
    let mut p = base_patch();
    let mut fresh = 0;
    for r in raws {
        let op = make_op(&p, *r, &mut fresh);
        let _ = apply(&mut p, &op);
    }
    p
}

/// Does the patch have an enabled oscillator with sync on?
pub fn sync_active(p: &Patch) -> bool {
    p.generators
        .iter()
        .any(|m| m.enabled && m.kind == ModuleKind::Osc && m.values[OscParam::Sync.index()] != 0.0)
}

pub fn assert_typed(e: &enth_patch::PatchError) {
    assert!(!e.code().is_empty(), "{e:?}");
    assert!(
        enth_patch::error::CODES.contains(&e.code()),
        "{} is not in CODES",
        e.code()
    );
    assert!(!e.message().is_empty(), "{e:?}");
    assert!(!e.hint().is_empty(), "{e:?}");
    let _ = e.did_you_mean();
    let _ = Builtin::ALL;
}

// ---------------------------------------------------------------------------------------------------------------
// Symmetries: relabelling ids and reordering what commutes
// ---------------------------------------------------------------------------------------------------------------

/// A tiny deterministic generator (splitmix64) for shuffles that must be reproducible from a proptest-drawn seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// Rename every module through `f` (ids in sections, routes, slots, the amplitude envelope).
pub fn rename(p: &Patch, f: impl Fn(&ModuleId) -> ModuleId) -> Patch {
    let mut q = p.clone();
    let map_addr = |a: &ParamAddr| match a.target() {
        Target::Module(id) => ParamAddr::new(Target::Module(f(id)), a.param()).unwrap(),
        _ => a.clone(),
    };
    for section in Section::ALL {
        for m in q.section_mut(section) {
            m.id = f(&m.id);
        }
    }
    for r in &mut q.routes {
        if let ModSource::Module(id) = &r.src {
            r.src = ModSource::Module(f(id));
        }
        r.dst = map_addr(&r.dst);
    }
    for s in q.slots.iter_mut().flatten() {
        *s = map_addr(s);
    }
    q.voice.amp_env = f(&q.voice.amp_env);
    q
}

/// A copy with every module under a new, unrelated id (a random bijection onto `q<n>`).
pub fn relabel(p: &Patch, rng: &mut Rng) -> Patch {
    let ids: Vec<ModuleId> = p.modules().map(|(_, m)| m.id.clone()).collect();
    let mut names: Vec<usize> = (0..ids.len()).collect();
    rng.shuffle(&mut names);
    rename(p, |id| {
        let i = ids.iter().position(|x| x == id).unwrap();
        ModuleId::new(&format!("q{}", names[i] + 1)).unwrap()
    })
}

/// A copy with `generators` and `modulators` shuffled and `routes` shuffled. With hard sync active, the first enabled
/// oscillator stays the first enabled oscillator (the one rule of order in the generators), the rest move freely.
pub fn reorder(p: &Patch, rng: &mut Rng) -> Patch {
    let mut q = p.clone();
    let master = p
        .generators
        .iter()
        .find(|m| m.enabled && m.kind == ModuleKind::Osc)
        .map(|m| m.id.clone());
    rng.shuffle(&mut q.generators);
    rng.shuffle(&mut q.modulators);
    rng.shuffle(&mut q.routes);
    if sync_active(p)
        && let Some(master) = master
    {
        let first = q
            .generators
            .iter()
            .position(|m| m.enabled && m.kind == ModuleKind::Osc)
            .unwrap();
        let at = q.generators.iter().position(|m| m.id == master).unwrap();
        q.generators.swap(first, at);
    }
    q
}

/// Relabel and reorder: the same patch, written as differently as the format allows.
pub fn scramble(p: &Patch, seed: u64) -> Patch {
    let mut rng = Rng(seed);
    let q = reorder(p, &mut rng);
    relabel(&q, &mut rng)
}

/// Add some oscillators (enabled, random values) and switch hard sync on for one of the enabled ones, through ops.
pub fn with_synced_oscs(p: &Patch, bits: u32, extra: usize) -> Patch {
    let mut q = p.clone();
    for i in 0..extra {
        let id = q.free_id(ModuleKind::Osc);
        let mut m = ModuleInstance::new(id, ModuleKind::Osc);
        m.values = values_for(
            ModuleKind::Osc,
            bits.rotate_left(i as u32 * 7) ^ 0x0505_0505,
        );
        let at = Position {
            section: Section::Generators,
            index: q.generators.len(),
        };
        let op = Op::AddModule { at, module: m };
        let _ = apply(&mut q, &op);
    }
    let oscs: Vec<ModuleId> = q
        .generators
        .iter()
        .filter(|m| m.enabled && m.kind == ModuleKind::Osc)
        .map(|m| m.id.clone())
        .collect();
    if let Some(id) = oscs.get(bits as usize % oscs.len().max(1)) {
        let addr = ParamAddr::module(id.clone(), OscParam::Sync.spec().key).unwrap();
        let _ = apply(&mut q, &Op::SetParam { addr, value: 1.0 });
    }
    q
}

/// Like [`build`], but also returns the ops that applied (in order, from `base_patch()`).
pub fn build_ops(raws: &[Raw]) -> (Vec<Op>, Patch) {
    let mut p = base_patch();
    let mut fresh = 0;
    let mut ops = Vec::new();
    for r in raws {
        let op = make_op(&p, *r, &mut fresh);
        if apply(&mut p, &op).is_ok() {
            ops.push(op);
        }
    }
    (ops, p)
}

// ---------------------------------------------------------------------------------------------------------------
// Hand-built patches
// ---------------------------------------------------------------------------------------------------------------

/// `n` identical LFOs and a ring of routes `lfo(i) -> lfo(next(i)).rate` given as (from, to) pairs.
pub fn lfo_graph(n: usize, edges: &[(usize, usize)]) -> Patch {
    let mut p = base_patch();
    for i in 0..n {
        let id = ModuleId::new(&format!("lfo{}", i + 1)).unwrap();
        let m = ModuleInstance::new(id, ModuleKind::Lfo);
        let at = Position {
            section: Section::Modulators,
            index: p.modulators.len(),
        };
        apply(&mut p, &Op::AddModule { at, module: m }).unwrap();
    }
    for &(a, b) in edges {
        let src = ModSource::Module(ModuleId::new(&format!("lfo{}", a + 1)).unwrap());
        let dst = ParamAddr::module(
            ModuleId::new(&format!("lfo{}", b + 1)).unwrap(),
            LfoParam::Rate.key(),
        )
        .unwrap();
        let route = Route {
            src,
            dst,
            depth: 20.0,
        };
        let index = p.routes.len();
        apply(&mut p, &Op::InsertRoute { index, route }).unwrap();
    }
    enth_patch::validate::validate(&p).unwrap();
    p
}

/// Two LFOs with the same parameters but different jobs: one drives `filter1.cutoff`, the other `osc1.semi`;
/// `first_drives_filter` says which of `lfo1`, `lfo2` has which job. Both spellings are the same patch up to naming.
pub fn two_lfos(first_drives_filter: bool) -> Patch {
    let mut p = base_patch();
    for id in ["lfo1", "lfo2"] {
        let m = ModuleInstance::new(ModuleId::new(id).unwrap(), ModuleKind::Lfo);
        let at = Position {
            section: Section::Modulators,
            index: p.modulators.len(),
        };
        apply(&mut p, &Op::AddModule { at, module: m }).unwrap();
    }
    let svf = ModuleInstance::new(ModuleId::new("filter1").unwrap(), ModuleKind::Svf);
    apply(
        &mut p,
        &Op::AddModule {
            at: Position {
                section: Section::VoiceFx,
                index: 0,
            },
            module: svf,
        },
    )
    .unwrap();
    let (a, b) = if first_drives_filter {
        ("lfo1", "lfo2")
    } else {
        ("lfo2", "lfo1")
    };
    for (src, dst) in [(a, "filter1.cutoff"), (b, "osc1.semi")] {
        let route = Route {
            src: ModSource::Module(ModuleId::new(src).unwrap()),
            dst: dst.parse().unwrap(),
            depth: 30.0,
        };
        let index = p.routes.len();
        apply(&mut p, &Op::InsertRoute { index, route }).unwrap();
    }
    p
}

/// The largest number of significant digits of any number in JSON text (outside strings). 9 is enough for any
/// `f32`; the shortest text that reads back to the same bits never needs more.
pub fn max_digits(json: &str) -> usize {
    let (mut best, mut in_string, mut escaped) = (0, false, false);
    let mut token = String::new();
    let flush = |token: &mut String, best: &mut usize| {
        if !token.is_empty() {
            let t = token.trim_start_matches('-');
            let mantissa = t.split(['e', 'E']).next().unwrap_or("");
            let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
            let digits = digits.trim_start_matches('0').trim_end_matches('0');
            *best = (*best).max(digits.len());
            token.clear();
        }
    };
    for c in json.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            flush(&mut token, &mut best);
            in_string = true;
        } else if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E') {
            token.push(c);
        } else {
            flush(&mut token, &mut best);
        }
    }
    flush(&mut token, &mut best);
    best
}
