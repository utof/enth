//! **Owner: lane DG.** The `ds1` v1 sampler: what one sample is, as a pure function of `(ds1 version, stream key, run
//! seed, index)` (docs/r3.md, "`ds1`: training data from our engine").
//!
//! Nothing here renders. A [`Draw`] holds the patch (on layout 1: template 0, differing from it only in layout entries
//! and `osc1.fine`), the phrase, the sample rate and the render seed; `pipeline` renders what a draw labels.
//!
//! **Streams of numbers.** Every concern draws from its own keyed stream, `derive(sample_seed, fnv1a64(key))`
//! (`source`, `phrase`, `rate`, `noise`, `v1/musical`, `uniform/...`), the way generator v1 keys its draws, so adding a
//! concern never moves another's values. A sample seed is `derive(derive(derive(fnv1a64("ds1/1"), fnv1a64(stream)),
//! run seed), index)`: the stream key is part of the seed, so `r3/train`, `r3/dev` and `r3/test` are disjoint by
//! construction (never by index ranges).
//!
//! # The mixture (a starting point to tune on dev)
//!
//! | Source | Share | What |
//! | --- | --- | --- |
//! | `v1` | 60 % | generator v1's priors at difficulty 100 (level 5), `amounts` 100 in every category (structure included), `musical` 100 / 50 / 0 at 60 / 25 / 15 %; template 0. The phrase is the sampler's own, **of the kind v1 chose** (see below) |
//! | `preset` | 25 % | an anchor (the anchor half of the preset candidates only, projected onto the layout), every continuous value moved by a normal step of sigma 0.05-0.2 in the normalised domain, each categorical and structure switch flipped with 4 %, each route's depth moved by the same sigma, removed with 3 %, and 2 % of the empty cells given a route |
//! | `uniform` | 15 % | every layout entry uniform in its normalised domain over a safe range (osc semitones within two octaves, resonance at most 80 %, drive at most 12 dB, envelope times at most 1.5 s, the cutoff above the fundamental for a low-pass, ...), each structure switch 50 %, each route cell 12 % (25 % for velocity to cutoff) with a depth of 5-100 % of either sign |
//!
//! **The phrase** (docs/r3.md): the key uniform over MIDI 24-84 (log-uniform in frequency), velocity 30-127, the hold
//! log-uniform from 0.08 to 4 s, 90 % one note, 5 % an octave pair, 5 % a legato pair, a window of 3-4 s from the
//! onset (3 s plus 0-1 s in quarters). An envelope longer than the hold or the tail is dead under `observable()` and
//! canonicalisation puts the template's value back: that is how the envelope caps follow the hold. **Generator v1's
//! draws depend on the phrase kind** (an octave pair reveals keytrack and velocity, a legato pair glide), so the `v1`
//! source first draws the kind it wants and takes the first of its 64 attempts whose generator phrase has that kind
//! (the generator's table: held, octave pair, legato pair); the sampler then draws a phrase of that kind with its own
//! key, velocities and holds. The kinds keep the generator's facts: an octave pair has two velocities and is detached
//! (the second note starts after the first ended), a legato pair has one velocity, overlaps, and **never spans an
//! octave** (that would reveal keytrack, which a v1 legato draw leaves at the template).
//!
//! **Note noise.** `osc1.fine` is the layout's context entry (decoding sets it from the pitch offset, within +-50 ct):
//! a quarter of the samples are in tune, the rest uniform over +-50 ct in tenths of a cent.
//!
//! **Sample rates**: 48 kHz 50 %, 44.1 kHz 25 %, 22.05 kHz 12.5 % and 16 kHz 12.5 %, rendered natively (the engine
//! runs at the file's rate and the features put their bands in Hz, so nothing is resampled).
#![allow(
    clippy::disallowed_methods,
    reason = "tooling: the sampler's arithmetic goes through enth_dsp::util::math (libm)"
)]

use enth_challenge::code::{ChallengeCode, EngineVersion};
use enth_challenge::model::{PhraseId, PhraseNote, PhraseView};
use enth_challenge::phrases::{HELD, LEGATO_PAIR, OCTAVE_PAIR};
use enth_challenge::random::{self, Generated};
use enth_challenge::spec::{Category, RandomSpec};
use enth_dsp::util::math;
use enth_dsp::util::rng::{Rng, derive, fnv1a64};
use enth_params::ParamSpec;
use enth_patch::{ModSource, Op, ParamAddr, Patch, Route, RouteAddr, apply};
use enth_solver::propose::decode::OutputLayout;

use super::anchors::{self, Anchor};
use crate::exclude::Exclusions;

/// The sampler's version, in every seed, manifest and model header. A change to any draw below is a new version.
pub const DS1_VERSION: u16 = 1;

/// The stream of training samples.
pub const STREAM_TRAIN: &str = "r3/train";
/// The stream of validation samples (never trained on; held-out hygiene tests and dedup run against it).
pub const STREAM_DEV: &str = "r3/dev";
/// The stream of development-curve samples (listed in `exclude-synth.txt`; never trained on).
pub const STREAM_TEST: &str = "r3/test";

/// Generator seeds are cut to this many bits, as the seeds of GR3's targets and the bank's are (a seed fits a code's 40 bits).
pub const SEED_BITS: u32 = 40;
/// The attempts the `v1` source makes to get the phrase kind it wants.
pub const PHRASE_TRIES: u64 = 64;

/// Where a sample's patch comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Generator v1's priors.
    V1 = 0,
    /// A perturbed preset anchor.
    Preset = 1,
    /// Uniform over the safe ranges.
    Uniform = 2,
}

impl Source {
    /// A stable name for the manifest.
    pub fn name(self) -> &'static str {
        match self {
            Source::V1 => "v1",
            Source::Preset => "preset",
            Source::Uniform => "uniform",
        }
    }
}

/// The mixture's weights, in percent.
pub const MIXTURE: [(Source, u32); 3] = [
    (Source::V1, 60),
    (Source::Preset, 25),
    (Source::Uniform, 15),
];
/// `musical` of a `v1` sample, in percent of the `v1` source.
pub const MUSICAL: [(u8, u32); 3] = [(100, 60), (50, 25), (0, 15)];
/// Sample rates in Hz and their weights (sixteenths).
pub const RATES: [(u32, u32); 4] = [(48_000, 8), (44_100, 4), (22_050, 2), (16_000, 2)];

/// What kind of phrase a sample plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Held,
    Octave,
    Legato,
}

impl Kind {
    fn of(id: PhraseId) -> Option<Kind> {
        match id {
            HELD => Some(Kind::Held),
            OCTAVE_PAIR => Some(Kind::Octave),
            LEGATO_PAIR => Some(Kind::Legato),
            _ => None,
        }
    }
}

/// How a draw could fail (counted by code like a skip).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawError {
    /// Generator v1 refused the spec.
    Generate(String),
    /// None of the attempts gave the phrase kind the sample wanted.
    PhraseMatch,
    /// An op failed on the template (a catalogue this sampler was not built for).
    Ops(String),
}

impl DrawError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            DrawError::Generate(_) => "v1_generate",
            DrawError::PhraseMatch => "v1_phrase_match",
            DrawError::Ops(_) => "draw_ops",
        }
    }
}

impl std::fmt::Display for DrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrawError::Generate(m) | DrawError::Ops(m) => f.write_str(m),
            DrawError::PhraseMatch => f.write_str("no attempt gave the phrase kind wanted"),
        }
    }
}

/// What a generator-v1 sample knows of its draw.
#[derive(Clone, Debug, PartialEq)]
pub struct GenInfo {
    /// The spec (generator 1, template 0, difficulty 100, `amounts` 100) that made the patch.
    pub spec: RandomSpec,
    /// Its challenge code, in the long form (a ds1 spec is no preset's, so it has no short one): what the `code` list is
    /// compared with, and what the game plays.
    pub code: Option<String>,
    /// The target fingerprint (64 hex digits), for the `fingerprint` list.
    pub fingerprint: String,
    /// The generated target as v1 made it (before the sampler's note noise): the near-duplicate rule reads it.
    pub generated: Generated,
}

/// One sample, drawn: everything but the render.
#[derive(Clone, Debug)]
pub struct Draw {
    /// The index within the stream.
    pub index: u64,
    /// The sample's seed (see the module docs).
    pub seed: u64,
    /// Where the patch comes from.
    pub source: Source,
    /// The anchor's index in [`Sampler::anchors`] (a `preset` sample).
    pub anchor: Option<usize>,
    /// The generator-v1 draw (a `v1` sample).
    pub generator: Option<GenInfo>,
    /// The patch: template 0 with layout entries changed, and `osc1.fine` the note noise. **Not yet canonicalised.**
    pub patch: Patch,
    /// The notes of the phrase (the first at 0 ms).
    pub notes: Vec<PhraseNote>,
    /// The render's length (ms).
    pub length_ms: u32,
    /// The render's sample rate (Hz).
    pub rate: u32,
    /// The engine's render seed.
    pub render_seed: u64,
}

impl Draw {
    /// The phrase as a view.
    pub fn phrase(&self) -> PhraseView<'_> {
        PhraseView {
            notes: &self.notes,
            length_ms: self.length_ms,
        }
    }
}

/// A sample's seed: a pure function of the version, the stream key, the run seed and the index.
pub fn sample_seed(stream: &str, run_seed: u64, index: u64) -> u64 {
    let version = fnv1a64(format!("ds1/{DS1_VERSION}").as_bytes());
    derive(
        derive(derive(version, fnv1a64(stream.as_bytes())), run_seed),
        index,
    )
}

fn rng(seed: u64, key: &str) -> Rng {
    Rng::new(derive(seed, fnv1a64(key.as_bytes())))
}

/// A weighted choice (integer weights, exact on every platform).
fn pick<T: Copy>(rng: &mut Rng, items: &[(T, u32)]) -> T {
    let total: u64 = items.iter().map(|(_, w)| u64::from(*w)).sum();
    let mut r = rng.below(total);
    for (v, w) in items {
        if r < u64::from(*w) {
            return *v;
        }
        r -= u64::from(*w);
    }
    items[items.len() - 1].0
}

/// Log-uniform over `[lo, hi]`.
fn log_uniform(rng: &mut Rng, lo: f64, hi: f64) -> f64 {
    lo * math::pow(hi / lo, rng.unit_f64())
}

/// A standard normal (Box-Muller; `libm` for the logarithm and cosine).
fn gauss(rng: &mut Rng) -> f64 {
    let u1 = 1.0 - rng.unit_f64();
    let u2 = rng.unit_f64();
    (-2.0 * math::log(u1)).sqrt() * math::cos(2.0 * std::f64::consts::PI * u2)
}

fn ops_error(e: impl std::fmt::Display) -> DrawError {
    DrawError::Ops(e.to_string())
}

fn set(p: &mut Patch, addr: &ParamAddr, value: f32) -> Result<(), DrawError> {
    apply(
        p,
        Op::SetParam {
            addr: addr.clone(),
            value,
        },
    )
    .map(drop)
    .map_err(ops_error)
}

fn set_route(
    p: &mut Patch,
    src: &ModSource,
    dst: &ParamAddr,
    depth: Option<f32>,
) -> Result<(), DrawError> {
    let existing = p.routes.iter().any(|r| &r.src == src && &r.dst == dst);
    let op = match (existing, depth) {
        (false, None) => return Ok(()),
        (true, None) => Op::RemoveRoute {
            route: RouteAddr {
                src: src.clone(),
                dst: dst.clone(),
            },
        },
        (true, Some(depth)) => Op::SetDepth {
            route: RouteAddr {
                src: src.clone(),
                dst: dst.clone(),
            },
            depth,
        },
        (false, Some(depth)) => Op::InsertRoute {
            index: p.routes.len(),
            route: Route {
                src: src.clone(),
                dst: dst.clone(),
                depth,
            },
        },
    };
    apply(p, op).map(drop).map_err(ops_error)
}

/// A route depth in 5 % steps, never 0, within +-100.
fn depth_step(depth: f32) -> f32 {
    let d = (depth / 5.0).round() * 5.0;
    let d = d.clamp(-100.0, 100.0);
    if d == 0.0 {
        if depth < 0.0 { -5.0 } else { 5.0 }
    } else {
        d
    }
}

/// The sampler: layout 1, the anchors and nothing else (a sample is a pure function of its key).
#[derive(Clone, Debug)]
pub struct Sampler {
    layout: OutputLayout,
    anchors: Vec<Anchor>,
}

impl Sampler {
    /// The sampler for layout 1. When `exclusions` is given, asserts that no anchor is on a list.
    ///
    /// # Errors
    /// An anchor that does not project, or one a list names (a hard error: a test preset must never be an anchor).
    pub fn new(exclusions: Option<&Exclusions>) -> Result<Sampler, String> {
        let layout = OutputLayout::v1();
        let anchors = anchors::load(&layout)?;
        if let Some(ex) = exclusions {
            for a in &anchors {
                if let Some(hit) = ex.preset(a.stem, &a.name, &a.sha256) {
                    return Err(format!("anchor {}: {hit}", a.stem));
                }
            }
        }
        Ok(Sampler { layout, anchors })
    }

    /// Layout 1.
    pub fn layout(&self) -> &OutputLayout {
        &self.layout
    }

    /// The anchors, projected.
    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }

    /// The sample of `(stream, run_seed, index)`.
    ///
    /// # Errors
    /// [`DrawError`]: generator v1 refused a spec, none of its attempts gave the phrase kind wanted, or an op failed.
    pub fn draw(&self, stream: &str, run_seed: u64, index: u64) -> Result<Draw, DrawError> {
        let seed = sample_seed(stream, run_seed, index);
        let source = pick(&mut rng(seed, "source"), &MIXTURE);
        let kind = pick(
            &mut rng(seed, "phrase/kind"),
            &[(Kind::Held, 90), (Kind::Octave, 5), (Kind::Legato, 5)],
        );
        let (notes, length_ms) = phrase(seed, kind);
        let first_key = notes[0].key;
        let rate = pick(&mut rng(seed, "rate"), &RATES);
        let render_seed = derive(seed, fnv1a64(b"render"));

        let (mut patch, anchor, generator) = match source {
            Source::V1 => {
                let (p, g) = Self::v1(seed, kind)?;
                (p, None, Some(g))
            }
            Source::Preset => {
                let (p, i) = self.preset(seed)?;
                (p, Some(i), None)
            }
            Source::Uniform => (self.uniform(seed, first_key)?, None, None),
        };
        let fine: ParamAddr = "osc1.fine".parse().map_err(ops_error)?;
        set(&mut patch, &fine, note_noise(seed))?;
        Ok(Draw {
            index,
            seed,
            source,
            anchor,
            generator,
            patch,
            notes,
            length_ms,
            rate,
            render_seed,
        })
    }

    /// Generator v1's patch, from the first of its attempts whose phrase has the kind wanted.
    fn v1(seed: u64, want: Kind) -> Result<(Patch, GenInfo), DrawError> {
        let musical = pick(&mut rng(seed, "v1/musical"), &MUSICAL);
        let base = derive(seed, fnv1a64(b"v1/spec"));
        for attempt in 0..PHRASE_TRIES {
            let spec = RandomSpec {
                generator: random::GENERATOR_V1,
                seed: derive(base, attempt) & ((1u64 << SEED_BITS) - 1),
                difficulty: 100,
                amounts: [Some(100); Category::COUNT],
                musical,
                template: random::TEMPLATE_STARTER,
            };
            let g = random::target(&spec).map_err(|e| DrawError::Generate(e.to_string()))?;
            if Kind::of(g.phrase) == Some(want) {
                let patch = g.patch.clone();
                let info = GenInfo {
                    code: ChallengeCode::long(spec, EngineVersion::CURRENT, g.fingerprint.prefix())
                        .ok()
                        .map(|c| c.encode()),
                    fingerprint: g.fingerprint.to_hex(),
                    spec,
                    generated: g,
                };
                return Ok((patch, info));
            }
        }
        Err(DrawError::PhraseMatch)
    }

    /// A perturbed anchor.
    fn preset(&self, seed: u64) -> Result<(Patch, usize), DrawError> {
        let mut r = rng(seed, "preset");
        let i = r.below(self.anchors.len() as u64) as usize;
        let sigma = 0.05 + 0.15 * r.unit_f64();
        let mut p = self.anchors[i].projected.clone();
        let l = &self.layout;
        for e in l.continuous() {
            let n = f64::from(e.spec.to_norm(p.value(&e.addr).unwrap_or(e.template)));
            let moved = (n + sigma * gauss(&mut r)).clamp(0.0, 1.0) as f32;
            set(&mut p, &e.addr, e.spec.from_norm(moved))?;
        }
        for e in l.categorical() {
            if r.below(100) < 4 {
                let items = e.len() as u64;
                let now = p.value(&e.addr).unwrap_or(e.template) as u64;
                let other = (now + 1 + r.below(items - 1)) % items;
                set(&mut p, &e.addr, other as f32)?;
            }
        }
        for e in l.enables() {
            if r.below(100) < 4 {
                let on = p.find(&e.id).is_some_and(|(_, m)| m.enabled);
                apply(
                    &mut p,
                    Op::SetEnabled {
                        id: e.id.clone(),
                        enabled: !on,
                    },
                )
                .map_err(ops_error)?;
            }
        }
        for c in l.routes() {
            let now = p
                .routes
                .iter()
                .find(|rt| rt.src == c.src && rt.dst == c.dst)
                .map(|rt| rt.depth);
            let depth = match now {
                Some(_) if r.below(100) < 3 => None,
                Some(d) => {
                    let n = f64::from(f32::midpoint(d / 100.0, 1.0));
                    let n = (n + sigma * gauss(&mut r)).clamp(0.0, 1.0);
                    Some(depth_step(((2.0 * n - 1.0) * 100.0) as f32))
                }
                None if r.below(100) < 2 => {
                    let mag = 10.0 + 50.0 * r.unit_f32();
                    Some(depth_step(if r.below(2) == 0 { mag } else { -mag }))
                }
                None => None,
            };
            set_route(&mut p, &c.src, &c.dst, depth)?;
        }
        Ok((p, i))
    }

    /// Every layout entry uniform over its safe range.
    fn uniform(&self, seed: u64, first_key: u8) -> Result<Patch, DrawError> {
        let mut r = rng(seed, "uniform");
        let l = &self.layout;
        let mut p = l.template().clone();
        for e in l.enables() {
            if r.below(2) == 0 {
                apply(
                    &mut p,
                    Op::SetEnabled {
                        id: e.id.clone(),
                        enabled: true,
                    },
                )
                .map_err(ops_error)?;
            }
        }
        for e in l.categorical() {
            let addr = e.addr.to_string();
            // The two random LFO shapes follow the render seed: a nuisance, never drawn (as in generator v1).
            let items = if addr.rsplit('.').next() == Some("shape") {
                5
            } else {
                e.len() as u64
            };
            set(&mut p, &e.addr, r.below(items) as f32)?;
        }
        for e in l.continuous() {
            let (lo, hi) = safe_range(&e.addr.to_string(), e.spec, &p, first_key);
            let (nlo, nhi) = (e.spec.to_norm(lo), e.spec.to_norm(hi));
            let n = nlo + (nhi - nlo) * r.unit_f32();
            set(&mut p, &e.addr, e.spec.from_norm(n))?;
        }
        for c in l.routes() {
            let percent = if c.src == ModSource::Velocity { 25 } else { 12 };
            if r.below(100) < percent {
                let mag = 5.0 * (1 + r.below(20)) as f32;
                let depth = if r.below(2) == 0 { mag } else { -mag };
                set_route(&mut p, &c.src, &c.dst, Some(depth))?;
            }
        }
        Ok(p)
    }
}

/// The plain range a uniform draw covers for one continuous entry (generator v1's safe ranges where it has them).
fn safe_range(addr: &str, spec: &ParamSpec, p: &Patch, first_key: u8) -> (f32, f32) {
    let value = |a: &str| a.parse::<ParamAddr>().ok().and_then(|a| p.value(&a));
    match addr {
        "osc1.semi" | "osc2.semi" => (-24.0, 24.0),
        "osc2.level" | "noise1.level" => (5.0, 100.0),
        "filter1.resonance" => (0.0, 80.0),
        "filter1.drive" => (0.0, 12.0),
        "env1.attack" | "env1.decay" | "env1.release" | "env2.attack" | "env2.decay"
        | "env2.release" => (0.0, 1_500.0),
        "lfo1.rate" | "lfo2.rate" => (0.25, 20.0),
        "voice.glide" => (5.0, 1_000.0),
        "reverb1.wet" => (-60.0, 0.0),
        "reverb1.decay" => (100.0, 8_000.0),
        "filter1.cutoff" => {
            let semi = value("osc1.semi").unwrap_or(0.0);
            let f0 = (440.0
                * math::pow(2.0, (f64::from(first_key) - 69.0 + f64::from(semi)) / 12.0))
                as f32;
            match value("filter1.mode").map_or(0, |m| m as u8) {
                0 => (f0.max(20.0), 20_000.0),
                1 => ((0.5 * f0).max(20.0), 12_000.0),
                2 => (20.0, (8.0 * f0).clamp(40.0, 20_000.0)),
                _ => ((0.5 * f0).max(20.0), 16_000.0),
            }
        }
        _ => (spec.min(), spec.max()),
    }
}

/// `osc1.fine`: a quarter in tune, the rest uniform over +-50 ct in tenths of a cent.
fn note_noise(seed: u64) -> f32 {
    let mut r = rng(seed, "noise");
    if r.below(4) == 0 {
        return 0.0;
    }
    let tenths = r.below(1001) as f32 - 500.0;
    tenths / 10.0
}

/// A phrase of `kind`: the notes (the first at 0 ms) and the render's length in ms.
fn phrase(seed: u64, kind: Kind) -> (Vec<PhraseNote>, u32) {
    let mut r = rng(seed, "phrase");
    let length_ms = 3_000 + 250 * r.below(5) as u32;
    let hold = |r: &mut Rng, lo: f64, hi: f64| (1000.0 * log_uniform(r, lo, hi)).round() as u32;
    let velocity = |r: &mut Rng| 30 + r.below(98) as u8;
    let note = |key: u8, velocity: u8, start_ms: u32, hold_ms: u32| PhraseNote {
        key,
        velocity,
        start_ms,
        hold_ms,
    };
    match kind {
        Kind::Held => {
            let key = 24 + r.below(61) as u8;
            let v = velocity(&mut r);
            let h = hold(&mut r, 0.08, 4.0);
            (vec![note(key, v, 0, h)], length_ms)
        }
        Kind::Octave => {
            let key = 24 + r.below(49) as u8;
            let va = velocity(&mut r);
            let ha = hold(&mut r, 0.08, 1.5);
            let gap = hold(&mut r, 0.05, 0.5);
            let vb = if va > 78 {
                va - 15 - r.below(u64::from(va) - 45) as u8
            } else {
                va + 15 + r.below(113 - u64::from(va)) as u8
            };
            let hb = hold(&mut r, 0.08, 1.5);
            (
                vec![note(key, va, 0, ha), note(key + 12, vb, ha + gap, hb)],
                length_ms,
            )
        }
        Kind::Legato => {
            let key = 36 + r.below(37) as u8;
            let interval: i16 = pick(
                &mut r,
                &[
                    (2, 2),
                    (3, 2),
                    (4, 2),
                    (5, 2),
                    (7, 3),
                    (9, 1),
                    (-2, 2),
                    (-3, 2),
                    (-4, 2),
                    (-5, 2),
                    (-7, 3),
                    (-9, 1),
                ],
            );
            let v = velocity(&mut r);
            let ha = hold(&mut r, 0.2, 1.5);
            let overlap = 0.4 + 0.5 * r.unit_f64();
            let start_b = (f64::from(ha) * overlap).floor() as u32;
            let hb = hold(&mut r, 0.08, 1.5);
            let key_b = (i16::from(key) + interval) as u8;
            (
                vec![note(key, v, 0, ha), note(key_b, v, start_b, hb)],
                length_ms,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use enth_challenge::phrases::PhraseFacts;
    use enth_solver::propose::decode::Hearing;
    use enth_solver::propose::features::Cue;

    use super::*;

    fn sampler() -> Sampler {
        Sampler::new(None).unwrap()
    }

    #[test]
    fn a_sample_is_a_pure_function_of_its_key() {
        let s = sampler();
        for i in 0..20 {
            let a = s.draw(STREAM_TRAIN, 7, i).unwrap();
            let b = s.draw(STREAM_TRAIN, 7, i).unwrap();
            assert_eq!(a.seed, b.seed);
            assert_eq!(a.patch, b.patch);
            assert_eq!(
                (a.notes.clone(), a.length_ms, a.rate),
                (b.notes.clone(), b.length_ms, b.rate)
            );
        }
        // The key is the whole key.
        let base = s.draw(STREAM_TRAIN, 7, 3).unwrap();
        assert_ne!(base.seed, s.draw(STREAM_TRAIN, 7, 4).unwrap().seed);
        assert_ne!(base.seed, s.draw(STREAM_TRAIN, 8, 3).unwrap().seed);
        assert_ne!(base.seed, s.draw(STREAM_DEV, 7, 3).unwrap().seed);
        assert_ne!(base.seed, s.draw(STREAM_TEST, 7, 3).unwrap().seed);
    }

    #[test]
    fn streams_are_disjoint_by_construction() {
        // No seed repeats across the three streams (same run seed, same indices): the stream key is in the seed.
        let mut seen = std::collections::BTreeSet::new();
        for stream in [STREAM_TRAIN, STREAM_DEV, STREAM_TEST] {
            for i in 0..5_000 {
                assert!(seen.insert(sample_seed(stream, 1, i)), "{stream} {i}");
            }
        }
    }

    #[test]
    fn the_mixture_rates_and_phrases_follow_the_plan() {
        let s = sampler();
        let n = 3_000u64;
        let mut sources: BTreeMap<&str, u32> = BTreeMap::new();
        let mut rates: BTreeMap<u32, u32> = BTreeMap::new();
        let (mut pairs, mut musical) = (0u32, BTreeMap::new());
        for i in 0..n {
            let d = s.draw(STREAM_TRAIN, 11, i).unwrap();
            *sources.entry(d.source.name()).or_default() += 1;
            *rates.entry(d.rate).or_default() += 1;
            if d.notes.len() == 2 {
                pairs += 1;
            }
            if let Some(g) = &d.generator {
                *musical.entry(g.spec.musical).or_insert(0u32) += 1;
                assert_eq!(g.spec.generator, 1);
                assert_eq!(g.spec.difficulty, 100);
                assert_eq!(g.spec.template, 0);
                assert!(g.spec.seed < 1 << SEED_BITS);
            }
            // The phrase is valid and in the plan's ranges.
            assert_eq!(d.phrase().validate(), Ok(()), "{i}");
            assert!((3_000..=4_000).contains(&d.length_ms));
            for n in &d.notes {
                assert!((24..=96).contains(&n.key), "{n:?}");
                assert!((30..=127).contains(&n.velocity), "{n:?}");
                assert!(n.hold_ms >= 80, "{n:?}");
            }
            assert!(d.notes[0].start_ms == 0 && d.notes[0].hold_ms <= 4_000);
            assert!(d.notes[0].key <= 84);
        }
        let share = |c: u32| f64::from(c) / n as f64;
        assert!((share(sources["v1"]) - 0.60).abs() < 0.04, "{sources:?}");
        assert!(
            (share(sources["preset"]) - 0.25).abs() < 0.04,
            "{sources:?}"
        );
        assert!(
            (share(sources["uniform"]) - 0.15).abs() < 0.04,
            "{sources:?}"
        );
        assert!((share(rates[&48_000]) - 0.50).abs() < 0.04, "{rates:?}");
        assert!((share(rates[&44_100]) - 0.25).abs() < 0.04, "{rates:?}");
        assert!((share(rates[&22_050]) - 0.125).abs() < 0.03, "{rates:?}");
        assert!((share(rates[&16_000]) - 0.125).abs() < 0.03, "{rates:?}");
        assert!((share(pairs) - 0.10).abs() < 0.03, "{pairs}");
        let v1 = f64::from(sources["v1"]);
        assert!(
            (f64::from(musical[&100]) / v1 - 0.60).abs() < 0.05,
            "{musical:?}"
        );
        assert!(
            (f64::from(musical[&0]) / v1 - 0.15).abs() < 0.05,
            "{musical:?}"
        );
    }

    #[test]
    fn a_pair_reveals_what_its_kind_says() {
        let s = sampler();
        let (mut octave, mut legato) = (0, 0);
        for i in 0..4_000 {
            let d = s.draw(STREAM_TRAIN, 5, i).unwrap();
            let view = d.phrase();
            if d.notes.len() == 2 {
                if view.has_octave_pair() {
                    octave += 1;
                    assert!(
                        view.has_two_velocities() && !view.has_overlapping_pitch_change(),
                        "{:?}",
                        d.notes
                    );
                } else {
                    legato += 1;
                    assert!(view.has_overlapping_pitch_change(), "{:?}", d.notes);
                }
            }
        }
        assert!(octave > 100 && legato > 100, "{octave} {legato}");
    }

    #[test]
    fn every_draw_is_on_the_layout() {
        // Whatever the source, the canonical patch encodes: other modules, a heard parameter outside the layout away
        // from the template's value or a route off the grid would be `off_layout`.
        let s = sampler();
        let l = s.layout();
        let mut by_source: BTreeMap<&str, u32> = BTreeMap::new();
        for i in 0..900 {
            let d = s.draw(STREAM_TRAIN, 3, i).unwrap();
            let mut canon = d.patch.clone();
            l.canonicalise(&mut canon, d.phrase()).unwrap();
            let cue = Cue {
                key: d.notes[0].key,
                f0_hz: None,
                on_s: 0.005,
                hold_s: f64::from(d.notes[0].hold_ms) / 1000.0,
                bandwidth_hz: None,
            };
            let hearing = Hearing::sample(&cue, &d.notes, d.length_ms).unwrap();
            let labels = l.encode(&canon, &hearing);
            assert!(
                labels.is_ok(),
                "{} ({}): {:?}",
                i,
                d.source.name(),
                labels.err()
            );
            *by_source.entry(d.source.name()).or_default() += 1;
        }
        assert_eq!(by_source.len(), 3, "{by_source:?}");
    }

    #[test]
    fn note_noise_stays_within_fifty_cents_and_a_quarter_is_in_tune() {
        let s = sampler();
        let fine: ParamAddr = "osc1.fine".parse().unwrap();
        let mut zero = 0;
        for i in 0..2_000 {
            let d = s.draw(STREAM_TRAIN, 9, i).unwrap();
            let v = d.patch.value(&fine).unwrap();
            assert!((-50.0..=50.0).contains(&v), "{v}");
            if v == 0.0 {
                zero += 1;
            }
        }
        assert!((400..700).contains(&zero), "{zero}");
    }

    #[test]
    fn a_v1_sample_gets_the_phrase_kind_it_wanted() {
        let s = sampler();
        for i in 0..400 {
            let d = s.draw(STREAM_TRAIN, 13, i).unwrap();
            if let Some(g) = &d.generator {
                let t = random::target(&g.spec).unwrap();
                assert_eq!(t.fingerprint.to_hex(), g.fingerprint);
                let kind = match d.notes.len() {
                    1 => HELD,
                    _ if d.phrase().has_octave_pair() => OCTAVE_PAIR,
                    _ => LEGATO_PAIR,
                };
                assert_eq!(t.phrase, kind, "{i}");
            }
        }
    }

    #[test]
    fn the_anchors_are_not_on_the_lists_and_the_sampler_says_so() {
        let ex = Exclusions::load(&crate::exclude::default_gr3()).unwrap();
        let s = Sampler::new(Some(&ex)).unwrap();
        assert_eq!(s.anchors().len(), 11);
    }
}
