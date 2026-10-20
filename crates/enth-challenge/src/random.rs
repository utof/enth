//! **Owner: lane RG** (the randomizer). Pre-declared empty by lane S0r.
//!
//! The Recreate generator (design.md §9, report 01 §2.3): a [`RandomSpec`] and a start patch give a target patch that
//! every machine regenerates identically, in parameter space only. Nothing here renders: `random` never reaches
//! `enth-render` (RG's acceptance; `tests/rg_render_free.rs` checks the sources).
//!
//! # How a target is made (generator v1)
//!
//! 1. **The level.** `difficulty` 0-100 picks one of five levels ([`level`]): 0-19 → 1, 20-39 → 2, 40-59 → 3,
//!    60-79 → 4, 80-100 → 5. Each level adds a [`Category`] ([`unlock_level`]): oscillators (1), filter (2),
//!    envelopes (3), modulation (4), effects and structure (5). A category's `amounts` entry overrides the level:
//!    `Some(0)` switches it off, `Some(a)` switches it on and draws the first ⌈n·a/100⌉ entries of its list in
//!    [`priors::v1`](crate::priors) (Basic tier first), with continuous values pulled towards the start by `a/100`
//!    ("how wide"). `None` draws the category's curated default set when the level reaches it.
//! 2. **Keyed streams.** Every value comes from its own stream, `derive(derive(seed, fnv1a64(key)), attempt)`, keyed
//!    by its address (`filter1.cutoff`, `osc2.enabled`, `lfo1->*`), never from a sequence. Adding a parameter, a
//!    module or a category never moves another draw, and a higher difficulty keeps the lower categories' values.
//!    Choices shared by several values (an envelope's pluck/pad archetype, an LFO's destination, the phrase) are
//!    auxiliary streams drawn at attempt 0.
//! 3. **The observability mask** ([`observable`]): only what the phrase reveals is drawn. Gauges, nuisances, dead
//!    and disabled parameters keep the start's value.
//! 4. **Priors and `musical`.** Each draw comes from the hand-written prior with probability `musical` %, otherwise
//!    uniformly over the parameter's observable range (in its normalised scale). Above `musical` 20 the uniform
//!    range is the safe one (resonance at most 80 %, pitch within two octaves, cutoff relative to the note).
//! 5. **Reachable values.** Continuous values are rounded to 3 significant digits, route depths to 5 % steps (never
//!    0), discrete values exact.
//! 6. **The minimum step**, decided on values: in every active category, at least one counted draw must differ from
//!    the start by its just-noticeable step ([`priors::jnd`](crate::priors::jnd)); otherwise the failing keys' attempt
//!    goes up by one and the category is drawn again, up to [`MAX_ROUNDS`] rounds, after which the first counted
//!    parameter is set one step (and a little) from the start. Categories are checked one by one, so k = the number
//!    of active categories, and a category's attempts never depend on a higher one. Draws whose visibility depends
//!    on a level-5 choice (osc 2 and the noise once structure enables them, keytrack and velocity under the octave
//!    pair) do not count, so they never change a lower category's attempts.
//! 7. **Structure over the starter superset** (owner, default yes): at level 5 the generator may enable osc 2 and the
//!    noise (structure), the reverb and the compressor (effects), add routes from env 2, both LFOs and velocity, and
//!    pick the phrase. It never disables a module the start has on, and never adds or removes one.
//! 8. **Headroom.** When the enabled sources sum above 100 %, `master.level` (a gauge, never drawn) is lowered by
//!    their sum in 0.5 dB steps, at most 12 dB.
//!
//! Old generator versions stay frozen: v1's 1,000-spec hash golden is `tests/golden/tests/rg_generator.rs` (in
//! `enth-golden`, so `golden.yml` runs it on every platform), and any change
//! to a prior, a rule or the table is a new generator version.

mod mask;

pub use mask::{Dead, Mask, Visibility, observable};

use enth_dsp::util::rng::{derive, fnv1a64};
use enth_params::{Kind, ParamSpec};
use enth_patch::catalogue::{MasterParam, OscParam, Wave};
use enth_patch::model::MAX_ROUTES;
use enth_patch::{
    ModSource, ModuleId, Op, ParamAddr, Patch, PatchMeta, Rate, Route, RouteAddr, apply,
};

use crate::model::{Phrase, PhraseId};
use crate::phrases::{self, REFERENCE_KEY};
use crate::priors::{self, Jnd};
use crate::spec::{Category, LATEST_GENERATOR, RandomSpec, SpecError};

/// Generator v1: this module's rules and [`priors::v1`](crate::priors). The versions this build runs are
/// `1..=`[`LATEST_GENERATOR`]; a spec naming another is refused by [`RandomSpec::validate`], never guessed.
pub const GENERATOR_V1: u16 = 1;

const _: () = assert!(
    LATEST_GENERATOR == GENERATOR_V1,
    "a new generator version needs its own table and golden"
);

/// Template 0: the starter superset patch (every P1 module; osc 2, the noise, the compressor and the reverb off; env 2
/// and both LFOs unrouted), frozen as it was at F0r. Templates 1-5 (pluck, pad, bass, lead, keys) are reserved.
pub const TEMPLATE_STARTER: u8 = 0;

/// The most rounds of the minimum-step check per category before the step is forced.
pub const MAX_ROUNDS: u16 = 16;

/// The frozen start of template 0 (a byte copy of `xtask/src/preset/starter.patch.json` at F0r, `513a22d`). It is a
/// separate copy so that later edits to the editor's starter or Init patch never move a target.
const STARTER_V1: &str = include_str!("random/starter-v1.patch.json");

/// The order categories are drawn in: structure first (it decides the phrase and which extra modules are on, which
/// the other categories' observability depends on), then the rest in difficulty order.
const DRAW_ORDER: [Category; 6] = [
    Category::Structure,
    Category::Oscillators,
    Category::Filter,
    Category::Envelopes,
    Category::Modulation,
    Category::Effects,
];

/// The level a difficulty picks: 0-20 → 1, 21-40 → 2, 41-60 → 3, 61-80 → 4, 81-100 → 5 (the presets sit at 0,
/// 25, 50, 75 and 100).
pub const fn level(difficulty: u8) -> u8 {
    let d = if difficulty > 100 { 100 } else { difficulty };
    let l = d.saturating_sub(1) / 20;
    1 + if l > 4 { 4 } else { l }
}

/// The lowest level that randomizes a category by default.
pub const fn unlock_level(category: Category) -> u8 {
    match category {
        Category::Oscillators => 1,
        Category::Filter => 2,
        Category::Envelopes => 3,
        Category::Modulation => 4,
        Category::Effects | Category::Structure => 5,
    }
}

/// Why a target could not be generated.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GenError {
    /// The spec fails [`RandomSpec::validate`] (a percentage above 100, generator 0, or a generator newer than
    /// [`LATEST_GENERATOR`]). The generator refuses it; it never clamps.
    Spec(SpecError),
    /// The spec names a template this build does not have.
    UnknownTemplate {
        /// The template the spec names.
        template: u8,
    },
    /// An op on the start patch failed (the start is not a valid patch, or not one the generator can work on).
    BadStart {
        /// The op's error.
        message: String,
    },
}

impl GenError {
    /// A stable code: the spec's (`out_of_range`, `no_generator`, `code_too_new`), `unknown_template` or
    /// `bad_start`.
    pub fn code(&self) -> &'static str {
        match self {
            GenError::Spec(e) => e.code(),
            GenError::UnknownTemplate { .. } => "unknown_template",
            GenError::BadStart { .. } => "bad_start",
        }
    }

    /// What to do about it.
    pub fn hint(&self) -> String {
        match self {
            GenError::Spec(e) => e.hint(),
            GenError::UnknownTemplate { template } => format!(
                "template {template} does not exist in this build; template {TEMPLATE_STARTER} is the starter patch"
            ),
            GenError::BadStart { .. } => {
                "start from a valid patch, e.g. random::start(0), the starter patch".to_owned()
            }
        }
    }
}

impl core::fmt::Display for GenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GenError::Spec(e) => e.fmt(f),
            GenError::UnknownTemplate { template } => write!(f, "unknown template {template}"),
            GenError::BadStart { message } => {
                write!(f, "cannot generate from this start: {message}")
            }
        }
    }
}

impl std::error::Error for GenError {}

/// One thing the generator drew (whether or not it ended up different from the start): what the overlay rings and
/// what structure % compares.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Drawn {
    /// A parameter value.
    Param(ParamAddr),
    /// A route's depth (the route is added when the start lacks it).
    Route(RouteAddr),
    /// A module's enable flag.
    Enable(ModuleId),
}

/// One randomized item: what was drawn, in which category, and whether it counts towards that category's minimum
/// step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Randomized {
    /// The category whose amount drew it.
    pub category: Category,
    /// What was drawn.
    pub drawn: Drawn,
    /// Whether it counts towards the category's minimum step (false for draws whose visibility depends on a
    /// level-5 choice: osc 2 and the noise once enabled, keytrack and velocity under the octave pair, voice mode).
    pub counted: bool,
}

/// How the minimum step went for one active category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The category.
    pub category: Category,
    /// Rounds drawn (1 when the first draw already met the step).
    pub rounds: u16,
    /// Whether some counted draw differs from the start by its just-noticeable step (false only when the category
    /// had nothing observable to count).
    pub met: bool,
    /// Whether the step was forced after [`MAX_ROUNDS`] rounds.
    pub forced: bool,
}

/// A generated target: the patch, the phrase it is heard through, and what was randomized.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    /// The target patch (the start, changed through ops only).
    pub patch: Patch,
    /// The phrase the challenge plays (an id in [`phrases::TABLE`]).
    pub phrase: PhraseId,
    /// The level the difficulty picked (1-5).
    pub level: u8,
    /// Everything drawn, in draw order, with its category.
    pub randomized: Vec<Randomized>,
    /// The randomized parameters, in address order, each once: what `score::TargetContext::randomized` takes (all
    /// observable through the phrase, P4). Routes and enable flags are in [`Generated::randomized`].
    pub params: Vec<ParamAddr>,
    /// The minimum step of each active category, in draw order.
    pub steps: Vec<Step>,
}

impl Generated {
    /// The phrase itself.
    pub fn phrase(&self) -> &'static Phrase {
        phrases::phrase(self.phrase).expect("the generator picks phrases from the table")
    }

    /// The target's observability mask under its phrase (what ST and the overlay reuse).
    pub fn mask(&self) -> Mask {
        observable(&self.patch, self.phrase())
    }

    /// The randomized parameters, in draw order (see [`Generated::params`] for address order).
    pub fn randomized_params(&self) -> impl Iterator<Item = &ParamAddr> {
        self.randomized.iter().filter_map(|r| match &r.drawn {
            Drawn::Param(a) => Some(a),
            _ => None,
        })
    }

    /// The categories that were randomized, in draw order.
    pub fn categories(&self) -> impl Iterator<Item = Category> + '_ {
        self.steps.iter().map(|s| s.category)
    }
}

/// The start patch of a template. Template 0 is the frozen starter superset.
pub fn start(template: u8) -> Result<Patch, GenError> {
    match template {
        TEMPLATE_STARTER => Ok(enth_patch::file::load(STARTER_V1)
            .expect("the frozen starter loads (tested)")
            .patch),
        _ => Err(GenError::UnknownTemplate { template }),
    }
}

/// The target of a spec, from its template's start: [`generate`]`(spec, &`[`start`]`(spec.template)?)`.
pub fn target(spec: &RandomSpec) -> Result<Generated, GenError> {
    generate(spec, &start(spec.template)?)
}

/// Generate the target of `spec` from `start` (see the [module docs](self)). Pure and deterministic: the same spec
/// and start give the same target, bit for bit, on every machine.
pub fn generate(spec: &RandomSpec, start: &Patch) -> Result<Generated, GenError> {
    spec.validate().map_err(GenError::Spec)?;
    match spec.generator {
        GENERATOR_V1 => generate_with(spec, start, priors::v1::SLOTS),
        // `validate` admits only 1..=LATEST_GENERATOR, which the const assertion above keeps at 1.
        generator => Err(GenError::Spec(SpecError::GeneratorTooNew {
            generator,
            latest: LATEST_GENERATOR,
        })),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The machinery behind `generate`, shared with the priors table
// ---------------------------------------------------------------------------------------------------------------

/// The keyed draws of one slot: `derive(derive(seed, fnv1a64(key)), attempt)`.
pub(crate) struct Draws<'a> {
    seed: u64,
    key: &'a str,
    attempt: u64,
}

impl Draws<'_> {
    fn bits_of(&self, key: &str, attempt: u64) -> u64 {
        derive(derive(self.seed, fnv1a64(key.as_bytes())), attempt)
    }

    /// 24 random bits as a uniform in `[0, 1)`, exact in `f32` (the engine's `unit_f32`).
    const fn unit_of(bits: u64) -> f32 {
        (bits >> 40) as f32 / 16_777_216.0
    }

    /// The slot's main uniform in `[0, 1)`.
    pub(crate) fn u(&self) -> f32 {
        Self::unit_of(self.bits_of(self.key, self.attempt))
    }

    /// A second uniform of this slot (`<key>/<sub>`), at the slot's attempt.
    pub(crate) fn sub(&self, sub: &str) -> f32 {
        Self::unit_of(self.bits_of(&format!("{}/{sub}", self.key), self.attempt))
    }

    /// An auxiliary uniform shared by several slots (`name`), at attempt 0: an archetype or a destination that must
    /// stay the same while the slots that use it redraw.
    pub(crate) fn shared(&self, name: &str) -> f32 {
        Self::unit_of(self.bits_of(name, 0))
    }

    /// A weighted choice by an integer draw (exact on every platform): `items` are (value, weight).
    pub(crate) fn pick_by(bits: u64, items: &[(f32, u32)]) -> f32 {
        let total: u64 = items.iter().map(|(_, w)| u64::from(*w)).sum();
        let mut r = if total == 0 { 0 } else { bits % total };
        for (v, w) in items {
            if r < u64::from(*w) {
                return *v;
            }
            r -= u64::from(*w);
        }
        items.last().map_or(0.0, |(v, _)| *v)
    }

    /// A weighted choice from the slot's main stream.
    pub(crate) fn pick(&self, items: &[(f32, u32)]) -> f32 {
        Self::pick_by(self.bits_of(self.key, self.attempt), items)
    }

    /// A weighted choice from an auxiliary stream shared by several slots, at attempt 0.
    pub(crate) fn pick_shared(&self, name: &str, items: &[(f32, u32)]) -> f32 {
        Self::pick_by(self.bits_of(name, 0), items)
    }

    /// Whether this draw comes from the prior: true with probability `musical` %, from `<key>/musical`.
    fn use_prior(&self, musical: u8) -> bool {
        self.bits_of(&format!("{}/musical", self.key), self.attempt) % 100 < u64::from(musical)
    }
}

/// What a slot's functions see: the target so far, the phrase, and whether the safety bounds are on.
pub(crate) struct Ctx<'a> {
    pub(crate) patch: &'a Patch,
    pub(crate) phrase_id: PhraseId,
    pub(crate) phrase: &'static Phrase,
    /// `musical > 20`: uniform draws keep to the safe ranges.
    pub(crate) safe: bool,
}

impl Ctx<'_> {
    /// The current value at an address, if the patch has it.
    pub(crate) fn value(&self, addr: &str) -> Option<f32> {
        self.patch.value(&addr.parse().ok()?)
    }

    /// The wave of an oscillator.
    pub(crate) fn wave(&self, id: &str) -> Option<Wave> {
        self.value(&format!("{id}.{}", OscParam::Wave.key()))
            .and_then(Wave::from_value)
    }

    /// The sounding fundamental of the reference note through osc 1 (`440 · 2^((key − 69 + semi) / 12)`), in Hz:
    /// what the filter priors are relative to.
    pub(crate) fn f0(&self) -> f64 {
        let semi = f64::from(self.value("osc1.semi").unwrap_or(0.0));
        440.0 * libm::pow(2.0, (f64::from(REFERENCE_KEY) - 69.0 + semi) / 12.0)
    }

    /// The destination of the first route from `src`, if any.
    pub(crate) fn route_dst(&self, src: &str) -> Option<String> {
        let src: ModSource = src.parse().ok()?;
        self.patch
            .routes
            .iter()
            .find(|r| r.src == src)
            .map(|r| r.dst.to_string())
    }
}

/// The range a uniform draw covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Domain {
    /// A continuous value between two plain values, uniform in the parameter's normalised scale.
    Range(f32, f32),
    /// One of these item indices (or integer values), each equally likely.
    Items(&'static [u8]),
}

/// What a slot draws.
#[derive(Clone, Copy)]
pub(crate) enum What {
    /// A parameter value: the musical prior (plain units) and the uniform domain.
    Param {
        addr: &'static str,
        prior: fn(&Ctx<'_>, &Draws<'_>) -> f32,
        uniform: fn(&Ctx<'_>) -> Domain,
    },
    /// Whether a module the start has off is switched on: on with `percent` % under the prior, 50 % uniformly.
    Enable { id: &'static str, percent: u8 },
    /// A route from `src`: its destination (a prior or a uniform choice, or none) and its musical depth in % (the
    /// uniform depth is 5-100 % of either sign).
    Route {
        src: &'static str,
        dst: fn(&Ctx<'_>, &Draws<'_>, bool) -> Option<&'static str>,
        depth: fn(&Ctx<'_>, &Draws<'_>, &str) -> f32,
    },
}

/// One entry of a generator's table. Its position in the table is its rank within its category.
#[derive(Clone, Copy)]
pub(crate) struct Slot {
    pub(crate) what: What,
    pub(crate) category: Category,
    /// In the category's default set (drawn when its amount is `None`).
    pub(crate) default: bool,
    /// Counts towards the category's minimum step.
    pub(crate) step: bool,
    /// An extra gate beyond the mask (e.g. glide only under the legato pair).
    pub(crate) when: fn(&Ctx<'_>) -> bool,
}

impl Slot {
    /// The stream key: the address, `<id>.enabled` or `<src>->*`.
    fn key(&self) -> String {
        match &self.what {
            What::Param { addr, .. } => (*addr).to_owned(),
            What::Enable { id, .. } => format!("{id}.enabled"),
            What::Route { src, .. } => format!("{src}->*"),
        }
    }
}

/// `true`: the slot needs no gate beyond the mask.
pub(crate) const fn always(_: &Ctx<'_>) -> bool {
    true
}

/// How much of a category to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Amount {
    /// The curated default set, full width.
    Default,
    /// The first `n` entries of the category's list, with continuous values pulled towards the start by `width`.
    First { n: usize, width: f32 },
}

fn amount(spec: &RandomSpec, level: u8, category: Category, list_len: usize) -> Option<Amount> {
    match spec.amounts[category.index()] {
        None if level >= unlock_level(category) => Some(Amount::Default),
        None | Some(0) => None,
        Some(a) => Some(Amount::First {
            n: (list_len * usize::from(a)).div_ceil(100),
            width: f32::from(a) / 100.0,
        }),
    }
}

/// The phrase: drawn from `phrase` (attempt 0) when structure is randomized, else the held note.
fn choose_phrase(seed: u64, musical: u8) -> PhraseId {
    let d = Draws {
        seed,
        key: "phrase",
        attempt: 0,
    };
    let items: &[(f32, u32)] = if d.use_prior(musical) {
        &priors::v1::PHRASE_WEIGHTS
    } else {
        &[(1.0, 1), (2.0, 1), (3.0, 1)]
    };
    PhraseId(d.pick(items) as u16)
}

fn op(patch: &mut Patch, op: Op) -> Result<(), GenError> {
    apply(patch, op).map(drop).map_err(|e| GenError::BadStart {
        message: e.to_string(),
    })
}

/// Round to 3 significant digits, in `f64` through `libm`, then to `f32`; `-0.0` becomes `0.0`.
pub(crate) fn sig3(x: f32) -> f32 {
    if x == 0.0 || !x.is_finite() {
        return 0.0;
    }
    let x = f64::from(x);
    let mag = libm::floor(libm::log10(libm::fabs(x)));
    let q = libm::pow(10.0, mag - 2.0);
    let r = (libm::round(x / q) * q) as f32;
    if r == 0.0 { 0.0 } else { r }
}

/// A route depth in 5 % steps, within ±100, never 0 (a depth-0 route is no route).
pub(crate) fn depth_step(depth: f32) -> f32 {
    let q = (libm::round(f64::from(depth) / 5.0) * 5.0).clamp(-100.0, 100.0) as f32;
    if q == 0.0 {
        if depth < 0.0 { -5.0 } else { 5.0 }
    } else {
        q
    }
}

/// A reachable value for `spec`: discrete values exact, continuous ones to 3 significant digits, in range.
pub(crate) fn quantize(spec: &ParamSpec, v: f32) -> f32 {
    if spec.is_discrete() {
        spec.clamp(v)
    } else {
        spec.clamp(sig3(spec.clamp(v)))
    }
}

/// A uniform draw over a domain.
fn uniform_value(spec: &ParamSpec, domain: Domain, d: &Draws<'_>) -> f32 {
    match domain {
        Domain::Items(items) if !items.is_empty() => {
            let i = d.pick(&items.iter().map(|v| (f32::from(*v), 1)).collect::<Vec<_>>());
            spec.clamp(i)
        }
        Domain::Items(_) => spec.default,
        Domain::Range(lo, hi) => {
            let (a, b) = (spec.to_norm(lo), spec.to_norm(hi));
            spec.from_norm(a + (b - a) * d.u())
        }
    }
}

/// One step and a little from `from`, in the direction that stays in range: the forced minimum step.
fn forced_value(spec: &ParamSpec, jnd: Jnd, domain: Domain, from: f32) -> Option<f32> {
    if spec.is_discrete() {
        let items: Vec<f32> = match domain {
            Domain::Items(items) => items.iter().map(|v| f32::from(*v)).collect(),
            Domain::Range(lo, hi) => {
                let (lo, hi) = (lo as i32, hi as i32);
                (lo..=hi).map(|v| v as f32).collect()
            }
        };
        return items.into_iter().find(|v| *v != from);
    }
    [1.0, -1.0].into_iter().find_map(|dir| {
        let v = quantize(spec, jnd.beyond(from, dir));
        jnd.exceeded(from, v).then_some(v)
    })
}

/// What one slot drew: what, and whether it differs from the start by its step.
struct Outcome {
    drawn: Drawn,
    passes: bool,
}

/// The state shared by every slot draw of one target.
struct Gen<'a> {
    seed: u64,
    musical: u8,
    phrase_id: PhraseId,
    phrase: &'static Phrase,
    start: &'a Patch,
    patch: Patch,
    mask: Option<Mask>,
}

impl Gen<'_> {
    fn ensure_mask(&mut self) {
        if self.mask.is_none() {
            self.mask = Some(observable(&self.patch, self.phrase));
        }
    }

    fn apply(&mut self, o: Op) -> Result<(), GenError> {
        self.mask = None;
        op(&mut self.patch, o)
    }

    /// Draw one slot at `attempt` (or set it to `forced`), apply it, and say what it drew. `None`: the slot does not
    /// apply to this patch and phrase (absent, hidden, gated).
    fn draw(
        &mut self,
        slot: &Slot,
        attempt: u64,
        width: f32,
        forced: Option<f32>,
    ) -> Result<Option<Outcome>, GenError> {
        self.ensure_mask();
        match self.plan(slot, attempt, width, forced) {
            Some((o, outcome)) => {
                self.apply(o)?;
                Ok(Some(outcome))
            }
            None => Ok(None),
        }
    }

    /// The op one slot's draw makes, and what it drew (no change yet). Needs the mask of the current patch.
    fn plan(
        &self,
        slot: &Slot,
        attempt: u64,
        width: f32,
        forced: Option<f32>,
    ) -> Option<(Op, Outcome)> {
        let mask = self.mask.as_ref().expect("ensure_mask ran");
        let key = slot.key();
        let d = Draws {
            seed: self.seed,
            key: &key,
            attempt,
        };
        let ctx = Ctx {
            patch: &self.patch,
            phrase_id: self.phrase_id,
            phrase: self.phrase,
            safe: self.musical > 20,
        };
        if !(slot.when)(&ctx) {
            return None;
        }
        match &slot.what {
            What::Param {
                addr,
                prior,
                uniform,
            } => {
                let addr: ParamAddr = addr.parse().expect("table addresses parse (tested)");
                let spec = self.patch.spec(&addr)?;
                let from = self.start.value(&addr)?;
                if !mask.is_observable(&addr) {
                    return None;
                }
                let v = forced.unwrap_or_else(|| {
                    let domain = uniform(&ctx);
                    let raw = if d.use_prior(self.musical) {
                        prior(&ctx, &d)
                    } else {
                        uniform_value(spec, domain, &d)
                    };
                    let raw = if width < 1.0 && matches!(spec.kind, Kind::Float { .. }) {
                        let (s, t) = (spec.to_norm(from), spec.to_norm(spec.clamp(raw)));
                        spec.from_norm(s + width * (t - s))
                    } else {
                        raw
                    };
                    // A narrow amount pulls towards the start, which may sit outside the observable or safe
                    // range (a 2 kHz high-pass two octaves below C3): keep the value inside it.
                    let raw = match domain {
                        Domain::Range(lo, hi) => raw.clamp(lo.min(hi), hi.max(lo)),
                        Domain::Items(_) => raw,
                    };
                    quantize(spec, raw)
                });
                let passes = priors::jnd(spec).exceeded(from, v);
                Some((
                    Op::SetParam {
                        addr: addr.clone(),
                        value: v,
                    },
                    Outcome {
                        drawn: Drawn::Param(addr),
                        passes,
                    },
                ))
            }
            What::Enable { id, percent } => {
                let id = ModuleId::new(id).expect("table ids are valid (tested)");
                // Only a module the start has off, and whose switch would be heard.
                if self.start.find(&id).is_none_or(|(_, m)| m.enabled)
                    || !mask.module(&id).is_some_and(Visibility::is_observable)
                {
                    return None;
                }
                let on = match forced {
                    Some(v) => v != 0.0,
                    None if d.use_prior(self.musical) => {
                        let p = u32::from(*percent);
                        d.pick(&[(1.0, p), (0.0, 100 - p)]) != 0.0
                    }
                    None => d.pick(&[(1.0, 1), (0.0, 1)]) != 0.0,
                };
                // Drawn off, it stays off: still drawn (in play), but no op.
                let o = Op::SetEnabled {
                    id: id.clone(),
                    enabled: on,
                };
                Some((
                    o,
                    Outcome {
                        drawn: Drawn::Enable(id),
                        passes: on,
                    },
                ))
            }
            What::Route { src, dst, depth } => {
                let src: ModSource = src.parse().expect("table sources parse (tested)");
                if let ModSource::Module(id) = &src
                    && !self.patch.find(id).is_some_and(|(_, m)| m.enabled)
                {
                    return None;
                }
                let dst_key = format!("{key}/dst");
                let aux = Draws {
                    seed: self.seed,
                    key: &dst_key,
                    attempt: 0,
                };
                let dst_text = dst(&ctx, &d, aux.use_prior(self.musical))?;
                let dst: ParamAddr = dst_text.parse().expect("table destinations parse (tested)");
                let spec = self.patch.spec(&dst)?;
                let heard = matches!(
                    mask.param(&dst),
                    Some(Visibility::Observable | Visibility::Gauge)
                );
                let rates_ok = match (
                    self.patch.source_rate(&src),
                    self.patch.target_rate(dst.target()),
                ) {
                    (Some(s), Some(t)) => !(s == Rate::Voice && t == Rate::Global),
                    _ => false,
                };
                if !spec.flags.modulatable || !heard || !rates_ok {
                    return None;
                }
                let from = self
                    .start
                    .routes
                    .iter()
                    .find(|r| r.src == src && r.dst == dst)
                    .map_or(0.0, |r| r.depth);
                let v = match forced {
                    Some(v) => v,
                    None => depth_step(if d.use_prior(self.musical) {
                        depth(&ctx, &d, dst_text)
                    } else {
                        // 5-100 % in 5 % steps, either sign.
                        let magnitude = 5.0 * (1.0 + (d.u() * 20.0).floor()).min(20.0);
                        if d.sub("sign") < 0.5 {
                            -magnitude
                        } else {
                            magnitude
                        }
                    }),
                };
                let route = RouteAddr {
                    src: src.clone(),
                    dst: dst.clone(),
                };
                let o = if self
                    .patch
                    .routes
                    .iter()
                    .any(|r| r.src == src && r.dst == dst)
                {
                    Op::SetDepth {
                        route: route.clone(),
                        depth: v,
                    }
                } else if self.patch.routes.len() < MAX_ROUTES {
                    Op::InsertRoute {
                        index: self.patch.routes.len(),
                        route: Route { src, dst, depth: v },
                    }
                } else {
                    return None;
                };
                Some((
                    o,
                    Outcome {
                        drawn: Drawn::Route(route),
                        passes: priors::ROUTE_JND.exceeded(from, v),
                    },
                ))
            }
        }
    }

    /// The forced value of a slot, for the minimum-step fallback (`None`: nothing to force).
    fn forced(&self, slot: &Slot) -> Option<f32> {
        match &slot.what {
            What::Param { addr, uniform, .. } => {
                let addr: ParamAddr = addr.parse().ok()?;
                let spec = self.patch.spec(&addr)?;
                let from = self.start.value(&addr)?;
                let ctx = Ctx {
                    patch: &self.patch,
                    phrase_id: self.phrase_id,
                    phrase: self.phrase,
                    safe: self.musical > 20,
                };
                forced_value(spec, priors::jnd(spec), uniform(&ctx), from)
            }
            What::Enable { .. } => Some(1.0),
            What::Route { .. } => None,
        }
    }
}

/// [`generate`] over any table (the tests add an extra parameter to check keyed-stream stability, P6).
pub(crate) fn generate_with(
    spec: &RandomSpec,
    start: &Patch,
    slots: &[Slot],
) -> Result<Generated, GenError> {
    spec.validate().map_err(GenError::Spec)?;
    let level = level(spec.difficulty);
    let lists: Vec<(Category, Vec<&Slot>)> = DRAW_ORDER
        .iter()
        .map(|c| (*c, slots.iter().filter(|s| s.category == *c).collect()))
        .collect();
    let plans: Vec<(Category, Option<Amount>)> = lists
        .iter()
        .map(|(c, list)| (*c, amount(spec, level, *c, list.len())))
        .collect();
    let structure = plans
        .iter()
        .any(|(c, a)| *c == Category::Structure && a.is_some());
    let phrase_id = if structure {
        choose_phrase(spec.seed, spec.musical)
    } else {
        phrases::HELD
    };
    let mut g = Gen {
        seed: spec.seed,
        musical: spec.musical,
        phrase_id,
        phrase: phrases::phrase(phrase_id).expect("the phrase weights name table ids (tested)"),
        start,
        patch: start.clone(),
        mask: None,
    };
    g.apply(Op::SetMeta {
        meta: PatchMeta::default(),
    })?;

    let mut randomized = Vec::new();
    let mut steps = Vec::new();
    for ((category, list), (_, plan)) in lists.iter().zip(&plans) {
        let Some(plan) = plan else { continue };
        let (chosen, width): (Vec<&Slot>, f32) = match *plan {
            Amount::Default => (list.iter().copied().filter(|s| s.default).collect(), 1.0),
            Amount::First { n, width } => (list.iter().copied().take(n).collect(), width),
        };
        let snapshot = g.patch.clone();
        let mut attempts = vec![0u64; chosen.len()];
        let mut forced: Vec<Option<f32>> = vec![None; chosen.len()];
        let mut rounds = 0u16;
        let (outcomes, met, was_forced) = loop {
            rounds += 1;
            g.patch = snapshot.clone();
            g.mask = None;
            let mut outcomes: Vec<(usize, Outcome)> = Vec::new();
            for (i, slot) in chosen.iter().enumerate() {
                if let Some(o) = g.draw(slot, attempts[i], width, forced[i])? {
                    outcomes.push((i, o));
                }
            }
            let counted: Vec<&(usize, Outcome)> =
                outcomes.iter().filter(|(i, _)| chosen[*i].step).collect();
            let was_forced = forced.iter().any(Option::is_some);
            if counted.is_empty() || counted.iter().any(|(_, o)| o.passes) || was_forced {
                let met = counted.iter().any(|(_, o)| o.passes);
                break (outcomes, met, was_forced);
            }
            if rounds == MAX_ROUNDS {
                // Force the first counted slot that has a forced value, and draw the round once more with it.
                let target = counted
                    .iter()
                    .map(|(i, _)| *i)
                    .find_map(|i| g.forced(chosen[i]).map(|v| (i, v)));
                match target {
                    Some((i, v)) => forced[i] = Some(v),
                    None => break (outcomes, false, false),
                }
                continue;
            }
            for (i, o) in &counted {
                if !o.passes {
                    attempts[*i] += 1;
                }
            }
        };
        randomized.extend(outcomes.into_iter().map(|(i, o)| Randomized {
            category: *category,
            drawn: o.drawn,
            counted: chosen[i].step,
        }));
        steps.push(Step {
            category: *category,
            rounds,
            met,
            forced: was_forced,
        });
    }

    headroom(&mut g)?;
    let mut params: Vec<ParamAddr> = randomized
        .iter()
        .filter_map(|r| match &r.drawn {
            Drawn::Param(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    params.sort();
    params.dedup();
    Ok(Generated {
        patch: g.patch,
        phrase: phrase_id,
        level,
        randomized,
        params,
        steps,
    })
}

/// Lower `master.level` by the enabled sources' summed level when it is above 100 %, in 0.5 dB steps, at most 12
/// dB (design.md §9: the generator may set the gauge for headroom).
fn headroom(g: &mut Gen<'_>) -> Result<(), GenError> {
    let sum: f64 = g
        .patch
        .generators
        .iter()
        .filter(|m| m.enabled)
        .filter_map(|m| m.get("level"))
        .map(|l| f64::from(l) / 100.0)
        .sum();
    if sum <= 1.0 {
        return Ok(());
    }
    let db = (libm::ceil(20.0 * libm::log10(sum) * 2.0) / 2.0).min(12.0);
    let addr = ParamAddr::master(MasterParam::Level);
    let current = g.patch.master[MasterParam::Level.index()];
    let value = g
        .patch
        .spec(&addr)
        .expect("master.level")
        .clamp(current.min(-db as f32));
    if value != current {
        g.apply(Op::SetParam { addr, value })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
