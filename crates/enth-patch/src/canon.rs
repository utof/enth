//! Canonicalisation, [`exact_hash`] and [`canonical_hash`] (P1a lane S1b).
//!
//! Two hashes, two questions:
//!
//! - **[`exact_hash`]: "is this the same patch?"** Covers everything that can change a render, in the order and
//!   under the ids the patch has: module ids (seeds follow ids), order, disabled modules, depth-0 routes, route
//!   order, every value as raw f32 bits. It leaves out what never changes the sound: the engine stamp, meta, labels,
//!   macro names and slots. It keys render caches and goldens.
//! - **[`canonical_hash`]: "is this the same patch up to symmetry?"** It hashes [`canonicalise`]`(p)`: the patch with
//!   the differences that cannot be heard taken out. It keys dedup, leaderboard entries and the `structure` score.
//!
//! **A canonical-equal pair is not promised to render bit-identically**: f32 addition of three or more generators
//! is not associative (a reorder can change the last bit), and seeds follow module ids (a relabel changes them).
//! Canonical equality means "the same sound up to rounding and random phase". Never assert
//! `render(p) == render(canonicalise(p))` bit for bit.
//!
//! # What [`canonicalise`] does
//!
//! 1. Drops disabled modules, and with them every route that touches one. A disabled module is bypassed (a
//!    generator is silent), which is the same as absent.
//! 2. Drops routes of depth 0.
//! 3. Sorts only the **commutative sections**, `generators` (they are summed) and `modulators` (evaluated in an
//!    order that does not change what they mean; docs/design.md §2). `voice_fx` and `fx` are serial chains and keep
//!    their order.
//! 4. Relabels ids as `<prefix><n>` in the final order (prefixes from `ModuleSpec::id_prefix`), rewriting routes
//!    and `voice.amp_env`.
//! 5. Sorts routes by (destination, source), so the summation order is gone (rounding only).
//! 6. Clears what does not change the sound: engine stamp, meta, labels, macro names, slots. Macro **values** stay.
//! 7. Stores `-0.0` as `0.0` and the top of a periodic range as its bottom, like the ops do.
//!
//! **Hard sync (the exception to step 3).** The first *enabled* `osc` in `generators` is the master of every
//! `osc` with `sync` on (`catalogue::OscParam::Sync`). So when any enabled oscillator has `sync` on, that first
//! oscillator is marked and sorts first among the oscillators; the other generators still sort freely. Two patches
//! that differ in which oscillator is the sync master therefore hash differently, and patches that differ only in
//! the order of the *other* oscillators hash the same. With no sync anywhere, all oscillators commute.
//!
//! # How the order is chosen
//!
//! A sort key made of parameter bits cannot order two modules whose parameters are equal but whose routes differ
//! (`lfo1` drives the filter, `lfo2` the oscillator), and modulators can modulate modulators, so the labels are
//! circular. So the order comes from **colour refinement** (1-WL): every module starts with a colour made of its
//! kind, section, parameter bits and flags; each round folds in the colours of its routes' other ends, labelled by
//! destination parameter and depth. Modules that still share a colour at the end are either interchangeable
//! (identical, with identical routes: a **twin**, swapping them changes nothing) or hard cases for 1-WL (two
//! triangles against a hexagon of identical LFOs). Both are settled by **individualisation**: pick one module of the
//! first tied class, give it a unique colour, refine again, and repeat, trying every non-twin choice and keeping the
//! smallest resulting encoding. The result depends only on the structure of the patch, never on its ids or the
//! order it was written in. Twins are skipped, so identical modules cost nothing; the search is also capped at
//! [`LEAF_BUDGET`] complete orderings (never reached by a real patch: it takes 8 modules in a rotation-symmetric
//! knot of non-twin ties), after which the best ordering found so far wins and invariance is no longer promised.
//!
//! # Encoding
//!
//! Both hashes are SHA-256 of a byte string: a magic (`enth-patch-e1` / `enth-patch-c1`), the schema number, the
//! voice values and `amp_env`, `master`, the eight macro values, then each section in file order (module id, kind
//! discriminant, enabled flag, value bits), then the routes. Strings are length-prefixed UTF-8, numbers
//! little-endian, f32 as bits. **The encoding and [`ModuleKind`] discriminants are a persisted contract** (render
//! caches and goldens key on it): a change needs a new magic.

use std::collections::BTreeMap;

use enth_params::ParamSpec;
use sha2::{Digest, Sha256};

use crate::addr::{ModSource, ModuleId, ParamAddr, Target};
use crate::catalogue::{MACRO_VALUE, MASTER_PARAMS, ModuleKind, OscParam, VOICE_PARAMS};
use crate::file::SCHEMA;
use crate::model::{Macro, ModuleInstance, Patch, PatchMeta, Route, Section};

/// How many complete orderings [`canonicalise`] tries at most before taking the best so far.
pub const LEAF_BUDGET: usize = 4096;

const MAGIC_EXACT: &[u8] = b"enth-patch-e1";
const MAGIC_CANON: &[u8] = b"enth-patch-c1";
const MAGIC_FULL: &[u8] = b"enth-patch-f1";

/// A SHA-256 patch hash.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PatchHash(pub [u8; 32]);

impl PatchHash {
    /// 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        use core::fmt::Write;
        self.0.iter().fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// The first 16 hex digits, enough to tell hashes apart in output.
    pub fn short(&self) -> String {
        self.to_hex()[..16].to_owned()
    }
}

impl core::fmt::Display for PatchHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl core::fmt::Debug for PatchHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PatchHash({})", self.short())
    }
}

fn sha(bytes: &[u8]) -> PatchHash {
    PatchHash(Sha256::digest(bytes).into())
}

/// The hash of a patch as it is: ids, order, disabled modules and depth-0 routes included. See the
/// [module docs](self).
pub fn exact_hash(p: &Patch) -> PatchHash {
    sha(&encode(p, MAGIC_EXACT, false))
}

/// The hash of [`canonicalise`]`(p)`: equal for patches that differ only by symmetry. See the
/// [module docs](self).
pub fn canonical_hash(p: &Patch) -> PatchHash {
    sha(&canonical_form(p).1)
}

/// The canonical form of a patch (see the [module docs](self)): idempotent, and the same for every patch that
/// differs only by ids, the order of interchangeable modules, disabled modules, depth-0 routes, meta, labels,
/// macro names and slots. Meant for valid patches; an invalid one gets a best-effort result.
pub fn canonicalise(p: &Patch) -> Patch {
    canonical_form(p).0
}

/// Bit-exact structural equality of two patches, **everything included** (engine stamp, meta, labels, macro names,
/// slots): f32 values compare by bits, so `-0.0 != 0.0` and a NaN equals itself. This is the "exact structural
/// equality" the tests mean; `Patch`'s derived `==` compares floats by value.
pub fn identical(a: &Patch, b: &Patch) -> bool {
    encode(a, MAGIC_FULL, true) == encode(b, MAGIC_FULL, true)
}

// ---------------------------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------------------------

struct Enc(Vec<u8>);

impl Enc {
    fn u8(&mut self, x: u8) {
        self.0.push(x);
    }
    fn u32(&mut self, x: usize) {
        self.0.extend_from_slice(&(x as u32).to_le_bytes());
    }
    fn f32(&mut self, x: f32) {
        self.0.extend_from_slice(&x.to_bits().to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len());
        self.0.extend_from_slice(s.as_bytes());
    }
    fn opt_str(&mut self, s: Option<&str>) {
        match s {
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
            None => self.u8(0),
        }
    }
}

fn encode_src(e: &mut Enc, src: &ModSource) {
    match src {
        ModSource::Module(id) => {
            e.u8(0);
            e.str(id.as_str());
        }
        ModSource::Velocity => e.u8(1),
        ModSource::Key => e.u8(2),
        ModSource::ModWheel => e.u8(3),
        ModSource::Macro(i) => {
            e.u8(4);
            e.u8(i.get() as u8);
        }
    }
}

fn encode_addr(e: &mut Enc, a: &ParamAddr) {
    match a.target() {
        Target::Module(id) => {
            e.u8(0);
            e.str(id.as_str());
        }
        Target::Voice => e.u8(1),
        Target::Master => e.u8(2),
    }
    e.str(a.param());
}

/// The byte string a hash is taken of. `full` appends what never changes the sound (labels, macro names, slots,
/// meta, engine) so [`identical`] can compare everything.
fn encode(p: &Patch, magic: &[u8], full: bool) -> Vec<u8> {
    let mut e = Enc(Vec::with_capacity(512));
    e.0.extend_from_slice(magic);
    e.u32(SCHEMA as usize);
    e.u8(p.voice.values.len() as u8);
    for &v in &p.voice.values {
        e.f32(v);
    }
    e.str(p.voice.amp_env.as_str());
    e.u8(p.master.len() as u8);
    for &v in &p.master {
        e.f32(v);
    }
    e.u8(p.macros.len() as u8);
    for m in &p.macros {
        e.f32(m.value);
    }
    for section in Section::ALL {
        let modules = p.section(section);
        e.u32(modules.len());
        for m in modules {
            e.str(m.id.as_str());
            e.u8(m.kind as u8);
            e.u8(u8::from(m.enabled));
            e.u32(m.values.len());
            for &v in &m.values {
                e.f32(v);
            }
        }
    }
    e.u32(p.routes.len());
    for r in &p.routes {
        encode_src(&mut e, &r.src);
        encode_addr(&mut e, &r.dst);
        e.f32(r.depth);
    }
    if full {
        e.str(&p.engine);
        e.str(&p.meta.name);
        e.str(&p.meta.author);
        e.str(&p.meta.description);
        e.u32(p.meta.tags.len());
        for t in &p.meta.tags {
            e.str(t);
        }
        for section in Section::ALL {
            for m in p.section(section) {
                e.opt_str(m.label.as_deref());
            }
        }
        for m in &p.macros {
            e.opt_str(m.name.as_deref());
        }
        for (n, slot) in p.slots.iter().enumerate() {
            if let Some(a) = slot {
                e.u8(1);
                e.u8(n as u8);
                encode_addr(&mut e, a);
            }
        }
        e.u8(0);
    }
    e.0
}

// ---------------------------------------------------------------------------------------------------------------
// Colour refinement
// ---------------------------------------------------------------------------------------------------------------

/// A 64-bit mixing step (splitmix64's finaliser). Colours only steer the search, so quality matters for speed, not
/// for correctness: every choice the search makes is settled by comparing complete encodings.
fn mix(h: u64, x: u64) -> u64 {
    let mut z = (h ^ x).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn mix_str(h: u64, s: &str) -> u64 {
    s.bytes()
        .fold(mix(h, s.len() as u64), |h, b| mix(h, u64::from(b)))
}

/// A parameter value as the canonical form stores it: `-0.0` as `0.0`, the top of a periodic range as its bottom.
fn normalised(spec: &ParamSpec, v: f32) -> f32 {
    if v == 0.0 {
        0.0
    } else if spec.flags.periodic && v == spec.max() {
        spec.min()
    } else {
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Src {
    Node(usize),
    Velocity,
    Key,
    ModWheel,
    Macro(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dst {
    Node(usize),
    Voice,
    Master,
}

/// A kept route, with module ids replaced by node numbers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    src: Src,
    dst: Dst,
    param: &'static str,
    depth_bits: u32,
}

impl Edge {
    fn swapped(&self, u: usize, v: usize) -> Edge {
        let sw = |n: usize| {
            if n == u {
                v
            } else if n == v {
                u
            } else {
                n
            }
        };
        Edge {
            src: match self.src {
                Src::Node(n) => Src::Node(sw(n)),
                other => other,
            },
            dst: match self.dst {
                Dst::Node(n) => Dst::Node(sw(n)),
                other => other,
            },
            ..self.clone()
        }
    }

    fn label(&self) -> u64 {
        mix(mix_str(0x1ABE1, self.param), u64::from(self.depth_bits))
    }
}

/// One enabled module and what identifies it independent of its id.
struct Node<'a> {
    section: Section,
    module: &'a ModuleInstance,
    is_amp_env: bool,
    is_sync_master: bool,
    /// Everything that makes two nodes interchangeable: section, kind, position, flags and value bits.
    fingerprint: Vec<u32>,
}

/// An end of a route as seen from a node: another node's colour, or a constant for a built-in.
#[derive(Clone, Copy)]
enum End {
    Node(usize),
    Const(u64),
}

struct Graph<'a> {
    patch: &'a Patch,
    nodes: Vec<Node<'a>>,
    edges: Vec<Edge>,
    out: Vec<Vec<(u64, End)>>,
    inn: Vec<Vec<(u64, End)>>,
}

impl<'a> Graph<'a> {
    fn new(p: &'a Patch) -> Self {
        let sync_on = p.generators.iter().any(|m| {
            m.enabled
                && m.kind == ModuleKind::Osc
                && m.values
                    .get(OscParam::Sync.index())
                    .is_some_and(|&v| v != 0.0)
        });
        let sync_master = sync_on
            .then(|| {
                p.generators
                    .iter()
                    .find(|m| m.enabled && m.kind == ModuleKind::Osc)
                    .map(|m| &m.id)
            })
            .flatten();
        let mut nodes: Vec<Node<'a>> = Vec::new();
        let mut index: BTreeMap<&ModuleId, usize> = BTreeMap::new();
        for section in Section::ALL {
            let fixed = matches!(section, Section::VoiceFx | Section::Fx);
            for (pos, m) in p.section(section).iter().filter(|m| m.enabled).enumerate() {
                let is_amp_env = m.id == p.voice.amp_env;
                let is_sync_master = sync_master == Some(&m.id);
                let fixed_pos = fixed.then_some(pos);
                let mut fp = vec![
                    section as u32,
                    m.kind as u32,
                    fixed_pos.map_or(u32::MAX, |x| x as u32),
                    u32::from(is_amp_env),
                    u32::from(is_sync_master),
                ];
                fp.extend(
                    m.values
                        .iter()
                        .zip(m.kind.params())
                        .map(|(&v, spec)| normalised(spec, v).to_bits()),
                );
                index.insert(&m.id, nodes.len());
                nodes.push(Node {
                    section,
                    module: m,
                    is_amp_env,
                    is_sync_master,
                    fingerprint: fp,
                });
            }
        }
        let mut edges = Vec::new();
        for r in &p.routes {
            if r.depth == 0.0 {
                continue;
            }
            let src = match &r.src {
                ModSource::Module(id) => match index.get(id) {
                    Some(&n) => Src::Node(n),
                    None => continue,
                },
                ModSource::Velocity => Src::Velocity,
                ModSource::Key => Src::Key,
                ModSource::ModWheel => Src::ModWheel,
                ModSource::Macro(i) => Src::Macro(i.get() as u8),
            };
            let dst = match r.dst.target() {
                Target::Module(id) => match index.get(id) {
                    Some(&n) => Dst::Node(n),
                    None => continue,
                },
                Target::Voice => Dst::Voice,
                Target::Master => Dst::Master,
            };
            edges.push(Edge {
                src,
                dst,
                param: r.dst.param(),
                depth_bits: r.depth.to_bits(),
            });
        }
        let mut out = vec![Vec::new(); nodes.len()];
        let mut inn = vec![Vec::new(); nodes.len()];
        for e in &edges {
            let label = e.label();
            let dst_end = match e.dst {
                Dst::Node(n) => End::Node(n),
                Dst::Voice => End::Const(0xD57_0001),
                Dst::Master => End::Const(0xD57_0002),
            };
            let src_end = match e.src {
                Src::Node(n) => End::Node(n),
                Src::Velocity => End::Const(0x5C0_0001),
                Src::Key => End::Const(0x5C0_0002),
                Src::ModWheel => End::Const(0x5C0_0003),
                Src::Macro(i) => End::Const(0x5C0_0100 + u64::from(i)),
            };
            if let Src::Node(n) = e.src {
                out[n].push((label, dst_end));
            }
            if let Dst::Node(n) = e.dst {
                inn[n].push((label, src_end));
            }
        }
        Graph {
            patch: p,
            nodes,
            edges,
            out,
            inn,
        }
    }

    fn colour0(&self) -> Vec<u64> {
        self.nodes
            .iter()
            .map(|n| {
                n.fingerprint
                    .iter()
                    .fold(0xC0107, |h, &x| mix(h, u64::from(x)))
            })
            .collect()
    }

    fn end_colour(end: End, colours: &[u64]) -> u64 {
        match end {
            End::Node(n) => colours[n],
            End::Const(c) => c,
        }
    }

    /// Refine the colouring until it stops splitting classes.
    fn refine(&self, colours: &mut Vec<u64>) {
        let mut classes = distinct(colours);
        for _ in 0..=colours.len() {
            let next: Vec<u64> = (0..colours.len())
                .map(|v| {
                    let side = |ends: &[(u64, End)]| {
                        let mut xs: Vec<u64> = ends
                            .iter()
                            .map(|&(label, end)| mix(label, Self::end_colour(end, colours)))
                            .collect();
                        xs.sort_unstable();
                        xs
                    };
                    let mut h = mix(colours[v], 1);
                    for x in side(&self.out[v]) {
                        h = mix(h, x);
                    }
                    h = mix(h, 2);
                    for x in side(&self.inn[v]) {
                        h = mix(h, x);
                    }
                    h
                })
                .collect();
            let n = distinct(&next);
            *colours = next;
            if n == classes {
                break;
            }
            classes = n;
        }
    }

    /// Whether swapping nodes `u` and `v` is a symmetry of the patch: same attributes, same routes.
    fn are_twins(&self, u: usize, v: usize) -> bool {
        if self.nodes[u].fingerprint != self.nodes[v].fingerprint {
            return false;
        }
        let mut a = self.edges.clone();
        let mut b: Vec<Edge> = self.edges.iter().map(|e| e.swapped(u, v)).collect();
        a.sort_unstable();
        b.sort_unstable();
        a == b
    }
}

fn distinct(colours: &[u64]) -> usize {
    let mut c = colours.to_vec();
    c.sort_unstable();
    c.dedup();
    c.len()
}

// ---------------------------------------------------------------------------------------------------------------
// The search over orderings
// ---------------------------------------------------------------------------------------------------------------

struct Search<'g, 'a> {
    g: &'g Graph<'a>,
    best: Option<(Vec<u8>, Patch)>,
    leaves: usize,
}

impl Search<'_, '_> {
    fn explore(&mut self, mut colours: Vec<u64>) {
        self.g.refine(&mut colours);
        let mut cells: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (v, &c) in colours.iter().enumerate() {
            cells.entry(c).or_default().push(v);
        }
        match cells.into_values().find(|cell| cell.len() > 1) {
            None => self.leaf(&colours),
            Some(cell) => {
                let mut tried: Vec<usize> = Vec::new();
                for &v in &cell {
                    if tried.iter().any(|&u| self.g.are_twins(u, v)) {
                        continue;
                    }
                    if self.leaves >= LEAF_BUDGET && self.best.is_some() {
                        return;
                    }
                    tried.push(v);
                    let mut next = colours.clone();
                    next[v] = mix(next[v], 0x1D1D);
                    self.explore(next);
                }
            }
        }
    }

    fn leaf(&mut self, colours: &[u64]) {
        self.leaves += 1;
        let g = self.g;
        // Final order: fixed sections as written; commutative ones by (kind, sync master first, colour).
        let mut orders: [Vec<usize>; 4] = Default::default();
        for (v, n) in g.nodes.iter().enumerate() {
            orders[n.section as usize].push(v);
        }
        for (section, order) in Section::ALL.into_iter().zip(&mut orders) {
            if matches!(section, Section::Generators | Section::Modulators) {
                order.sort_by_key(|&v| {
                    (
                        g.nodes[v].module.kind as u8,
                        !g.nodes[v].is_sync_master,
                        colours[v],
                    )
                });
            }
        }
        let (patch, bytes) = build(g, &orders);
        if self.best.as_ref().is_none_or(|(b, _)| bytes < *b) {
            self.best = Some((bytes, patch));
        }
    }
}

/// The canonical patch for one ordering of the nodes, and its encoding.
fn build(g: &Graph<'_>, orders: &[Vec<usize>; 4]) -> (Patch, Vec<u8>) {
    let p = g.patch;
    let mut new_id: Vec<Option<ModuleId>> = vec![None; g.nodes.len()];
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut sections: [Vec<ModuleInstance>; 4] = Default::default();
    for (section, order) in Section::ALL.into_iter().zip(orders) {
        for &v in order {
            let m = g.nodes[v].module;
            let prefix = m.kind.spec().id_prefix;
            let n = counts.entry(prefix).or_insert(0);
            *n += 1;
            let id = ModuleId::numbered(prefix, *n)
                .expect("a catalogue prefix and a small number form a valid id");
            new_id[v] = Some(id.clone());
            sections[section as usize].push(ModuleInstance {
                id,
                kind: m.kind,
                label: None,
                enabled: true,
                values: m
                    .values
                    .iter()
                    .zip(m.kind.params())
                    .map(|(&v, spec)| normalised(spec, v))
                    .collect(),
            });
        }
    }
    let id_of = |n: usize| new_id[n].clone().expect("every node is placed");
    let mut routes: Vec<Route> = g
        .edges
        .iter()
        .map(|e| {
            let target = match e.dst {
                Dst::Node(n) => Target::Module(id_of(n)),
                Dst::Voice => Target::Voice,
                Dst::Master => Target::Master,
            };
            Route {
                src: match e.src {
                    Src::Node(n) => ModSource::Module(id_of(n)),
                    Src::Velocity => ModSource::Velocity,
                    Src::Key => ModSource::Key,
                    Src::ModWheel => ModSource::ModWheel,
                    Src::Macro(i) => ModSource::Macro(
                        crate::addr::MacroIndex::new(usize::from(i)).expect("a macro index"),
                    ),
                },
                dst: ParamAddr::new(target, e.param).expect("the key came from an address"),
                depth: f32::from_bits(e.depth_bits),
            }
        })
        .collect();
    routes.sort_by(|a, b| {
        (&a.dst, &a.src, a.depth.to_bits()).cmp(&(&b.dst, &b.src, b.depth.to_bits()))
    });
    let amp_env = g
        .nodes
        .iter()
        .position(|n| n.is_amp_env)
        .map_or_else(|| p.voice.amp_env.clone(), id_of);
    let [generators, voice_fx, fx, modulators] = sections;
    let mut voice = p.voice.clone();
    voice.amp_env = amp_env;
    for (v, spec) in voice.values.iter_mut().zip(&VOICE_PARAMS) {
        *v = normalised(spec, *v);
    }
    let mut master = p.master;
    for (v, spec) in master.iter_mut().zip(&MASTER_PARAMS) {
        *v = normalised(spec, *v);
    }
    let patch = Patch {
        engine: String::new(),
        meta: PatchMeta::default(),
        voice,
        master,
        generators,
        voice_fx,
        fx,
        modulators,
        routes,
        macros: core::array::from_fn(|i| Macro {
            value: normalised(&MACRO_VALUE, p.macros[i].value),
            name: None,
        }),
        slots: core::array::from_fn(|_| None),
    };
    let bytes = encode(&patch, MAGIC_CANON, false);
    (patch, bytes)
}

fn canonical_form(p: &Patch) -> (Patch, Vec<u8>) {
    let (patch, bytes, _) = canonical_search(p);
    (patch, bytes)
}

fn canonical_search(p: &Patch) -> (Patch, Vec<u8>, usize) {
    let g = Graph::new(p);
    let mut search = Search {
        g: &g,
        best: None,
        leaves: 0,
    };
    let colours = g.colour0();
    search.explore(colours);
    let (bytes, patch) = search
        .best
        .unwrap_or_else(|| unreachable!("the first descent always reaches a complete ordering"));
    (patch, bytes, search.leaves)
}

/// How many complete orderings [`canonicalise`] tried for this patch (at most [`LEAF_BUDGET`]): 1 for a patch whose
/// modules colour refinement tells apart, or that only has interchangeable twins; more for symmetric knots of
/// non-twin modules. A diagnostic for tests and for noticing a pathological patch, not something to branch on.
pub fn canonical_effort(p: &Patch) -> usize {
    canonical_search(p).2
}
