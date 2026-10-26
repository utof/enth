//! **Audio routes between generators** (lane RTE, S2a; the seam file lane S0d created at freeze F0m).
//!
//! An audio route feeds one generator's output into an `audio` parameter of another generator, per frame: FM, PM, AM
//! and ring modulation all need it, and `wt`'s `pm` and `am` are its first destinations. The logic lives here; the
//! shared files hold only call sites (`plan.rs`: [`classify`] and [`AudioRoutes::build`] in `compile`, the depth ids;
//! `engine.rs`: step 3 [`AudioRoutes::advance_depths`] and step 7 [`AudioRoutes::render`]; `module.rs`:
//! [`VoiceCx::audio_in`](crate::module::VoiceCx::audio_in) and `pre_level`).
//!
//! The contract (research report 06 §3.1; the F1m judge checks its six points):
//!
//! 1. **Destinations.** An `audio` parameter ([`enth_params::Flags::audio`]; so far `wt.pm` and `wt.am`) accepts only
//!    generator sources, and a generator source targets only `audio` parameters; `enth-patch`'s `validate` and the
//!    `InsertRoute` op enforce it with their own codes, and [`classify`] repeats it for a patch that skipped
//!    `validate` (`not_audio_dest`, `not_audio_source`). Audio routes never enter the control-rate matrix
//!    ([`crate::modulation`]).
//! 2. **Order.** [`AudioRoutes`] sorts the generators topologically by their audio routes (Kahn's algorithm, always
//!    taking the lowest patch index among those ready, so ties keep patch order and a patch without audio routes runs
//!    exactly as before: the engine then never enters [`AudioRoutes::render`]). A cycle, including a route from a
//!    generator to itself, is refused by `compile` as [`CompileError::AudioCycle`] (`audio_cycle`). Hard sync keeps
//!    its rule: the master is the first enabled `osc` in *patch* order and runs before every later generator
//!    ([`AudioRoutes::build`] checks it).
//! 3. **Buffers.** Each generator writes its **pre-level** output (before its `level` ramp) into its own segment
//!    buffer, allocated in `compile`: up to 4 generators × 32 voices × 32 frames × 4 B = 16 KB, plus the same again
//!    for the post-level outputs the voice sum waits on. A carrier reads the buffers of its sources per frame, scaled
//!    by the route depth, through [`AudioIn`]. The **voice sum stays in patch order**: every generator renders into
//!    its own post-level buffer, and the buffers are added in patch order after all of them have run, the same float
//!    operations in the same order as the direct loop, so reordering the run never moves a bit of the sum.
//! 4. **Feedback** is a parameter inside a module, never a route between modules. A self-route is refused like a
//!    cycle.
//! 5. **Carriers.** PM is only for phase-read waves (`wt`), so PM never meets a polyBLEP shape (`docs/dsp/engine.md`).
//! 6. **Masks.** A silent generator hides all but its level, unless an audio route leaves it: a generator whose
//!    `level` is 0 still feeds the routes that leave it, because the pre-level output is what travels
//!    (`docs/dsp/engine.md`, for the matcher and the generator's next version).
//!
//! # Depth
//!
//! The depth of an audio route is the route's `depth`, −100 to +100 %, as for every route: the carrier receives
//! `depth / 100 × the source's pre-level sample`. What that scale means (an index for PM, an amount for AM) is the
//! carrier's. A depth event ramps it over [`DEPTH_SMOOTHING_MS`] like a control route's, per sample within each
//! segment, so a depth sweep never steps. A route with depth 0 is left out of the plan, as for control routes, so a
//! depth change from or to 0 is a recompile. Its [`DepthId`] is `0x8000 +` the route's index among the plan's audio
//! routes ([`AUDIO_DEPTH_BASE`]); control routes count from 0.
//!
//! # The module side
//!
//! A generator that can be a source writes `cx.io[v]` as always, and, when `cx.pre_level` is `Some`, also copies
//! its output *before* the level ramp into `pre[v]` for every active voice (`osc` and `noise` do). A generator that
//! can be a carrier calls [`AudioIn::read`] for each audio parameter it has; that fills a [`Block`] with the sum of
//! the routes reaching the parameter, in patch route order. The engine zeroes `pre` first, so a source that ignores
//! `pre_level` is silent to its routes rather than stale.

use enth_patch::validate::{is_audio_dest, is_generator_source};
use enth_patch::{ModSource, Patch, Route, Section, Target};

use crate::modulation::DEPTH_SMOOTHING_MS;
use crate::module::{Block, MAX_VOICES, NO_SYNC, RAMP, SEG, SyncBus, VoiceBus, VoiceIdx};
use crate::params::{ParamId, ParamIndex};
use crate::plan::{CompileError, DepthId, Plan, PlanMap};

/// The first [`DepthId`] of an audio route: control routes are numbered from 0 (their terms in the matrix), audio
/// routes from here.
pub(crate) const AUDIO_DEPTH_BASE: u16 = 0x8000;

/// The most routes into one carrier that [`AudioIn`] holds. A patch has at most 4 generators, so a carrier has at
/// most 3 sources, each once per audio parameter (`validate` refuses two routes with one source and destination);
/// 16 covers a kind with five audio parameters.
pub const MAX_AUDIO_INPUTS: usize = 16;

/// What the engine does with a route, from its two ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteClass {
    /// A modulation route: the control-rate matrix (the compiler checks the rest).
    Control,
    /// An audio route: [`AudioRoutes::build`].
    Audio,
    /// A route the engine cannot run, with the reason `compile` reports (stable wording of
    /// [`CompileError::BadRoute`]).
    Reject(&'static str),
}

/// Sort a route into the matrix or the audio routes, or refuse it: a generator source goes to an `audio` parameter of
/// a generator and nowhere else (`not_audio_dest`); an `audio` parameter takes nothing but a generator
/// (`not_audio_source`). Anything else, including an end that does not exist, is left to the control-route checks
/// (`unknown_source`, `not_a_modulator`, `unknown_destination`, `not_modulatable`).
pub(crate) fn classify(patch: &Patch, r: &Route) -> RouteClass {
    let to_audio = is_audio_dest(patch, &r.dst);
    if is_generator_source(patch, &r.src) {
        if patch.spec(&r.dst).is_none() {
            return RouteClass::Reject("unknown_destination");
        }
        return if to_audio && dst_is_generator(patch, &r.dst) {
            RouteClass::Audio
        } else {
            RouteClass::Reject("not_audio_dest")
        };
    }
    let source_resolves = match &r.src {
        ModSource::Module(id) => patch
            .find(id)
            .is_some_and(|(pos, _)| pos.section == Section::Modulators),
        _ => true,
    };
    if to_audio && source_resolves {
        RouteClass::Reject("not_audio_source")
    } else {
        RouteClass::Control
    }
}

fn dst_is_generator(patch: &Patch, dst: &enth_patch::ParamAddr) -> bool {
    matches!(dst.target(), Target::Module(id)
        if patch.find(id).is_some_and(|(pos, _)| pos.section == Section::Generators))
}

/// One audio route as the plan holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Term {
    /// The source and the carrier, as indices into `Plan::generators` (patch order).
    src: usize,
    dst: usize,
    /// The carrier's parameter, as an index within its module's table.
    param: usize,
    /// The depth as a fraction (`depth / 100`) this segment ends on, and the previous segment's.
    cur: f32,
    prev: f32,
}

/// A depth event in flight (see `Matrix::set_depth`: the same ramp, kept here so `modulation.rs` stays untouched).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Ramp {
    target: f32,
    step: f32,
    /// Segments left; 0 when settled.
    left: u32,
}

/// The audio routes of a plan, with the run order and the buffers they need. Empty (the `Default`) for a patch
/// without audio routes, which is the state of every patch that exists today.
#[derive(Debug, Default)]
pub(crate) struct AudioRoutes {
    /// The routes sorted by carrier, patch route order within one carrier (a stable sort).
    terms: Vec<Term>,
    /// Per term: its patch route index (the tag [`AudioRoutes::tags`] reports).
    tags: Vec<usize>,
    /// Per term: its depth ramp.
    ramps: Vec<Ramp>,
    /// Per generator: its range in `terms`.
    by_dst: Vec<core::ops::Range<usize>>,
    /// Per generator: some route leaves it, so it is asked for its pre-level output.
    is_source: Vec<bool>,
    /// The generators in the order they run.
    order: Vec<usize>,
    /// Per generator: the pre-level output of this segment, for the routes that leave it.
    pre: Vec<VoiceBus>,
    /// Per generator: the output that goes to the voice sum.
    post: Vec<VoiceBus>,
    ramp_segments: u32,
}

/// The two ends of an enabled audio route, before sorting.
struct Spec {
    src: usize,
    dst: usize,
    param: usize,
    depth: f32,
    route: usize,
}

/// A generator order that respects `edges` (`(source, carrier)` pairs over `n` generators): Kahn's algorithm,
/// always taking the lowest index among those ready. `Err(i)` is the index of an edge that lies on a cycle.
fn run_order(n: usize, edges: &[(usize, usize)]) -> Result<Vec<usize>, usize> {
    let mut waiting = vec![0usize; n];
    for &(_, d) in edges {
        waiting[d] += 1;
    }
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let Some(g) = (0..n).find(|&g| !done[g] && waiting[g] == 0) else {
            // Everything left waits on something left: a cycle. An edge between two of them whose carrier can reach
            // its source is on it (a node downstream of the cycle is not).
            let left = |g: usize| !done[g];
            let on_cycle = edges
                .iter()
                .position(|&(s, d)| left(s) && left(d) && reaches(edges, &done, d, s))
                .unwrap_or(0);
            return Err(on_cycle);
        };
        done[g] = true;
        order.push(g);
        for &(s, d) in edges {
            if s == g {
                waiting[d] -= 1;
            }
        }
    }
    Ok(order)
}

/// Whether `to` can be reached from `from` along `edges` between the generators not `done` (a node reaches itself).
fn reaches(edges: &[(usize, usize)], done: &[bool], from: usize, to: usize) -> bool {
    let mut seen = vec![false; done.len()];
    let mut stack = vec![from];
    while let Some(at) = stack.pop() {
        if at == to {
            return true;
        }
        if core::mem::replace(&mut seen[at], true) {
            continue;
        }
        stack.extend(
            edges
                .iter()
                .filter(|&&(s, d)| s == at && !done[d])
                .map(|&(_, d)| d),
        );
    }
    false
}

impl AudioRoutes {
    /// Compile the audio routes `routes` (indices into `patch.routes`, each classified [`RouteClass::Audio`]) against
    /// the plan's generators: the terms, the run order and the buffers. A route with depth 0, or from or to a
    /// disabled generator, contributes nothing.
    ///
    /// # Errors
    /// [`CompileError::AudioCycle`] when the enabled generators' routes loop, and [`CompileError::BadRoute`] for a
    /// carrier with more than [`MAX_AUDIO_INPUTS`] routes or an audio route into the hard-sync master that would run
    /// it after a later generator (no kind has an audio parameter on an `osc` today; the check keeps the rule
    /// explicit if one gets one).
    pub(crate) fn build(
        patch: &Patch,
        plan: &Plan,
        map: &PlanMap,
        routes: &[usize],
    ) -> Result<Self, CompileError> {
        let generator =
            |id: &enth_patch::ModuleId| plan.generators.iter().position(|s| &s.id == id);
        let mut specs: Vec<Spec> = Vec::new();
        for &route in routes {
            let r = &patch.routes[route];
            let (ModSource::Module(src), Target::Module(dst)) = (&r.src, r.dst.target()) else {
                continue;
            };
            let (Some(src), Some(dst), Some(ParamId::Voice(index))) =
                (generator(src), generator(dst), map.get(&r.dst))
            else {
                continue; // from or to a disabled generator
            };
            if r.depth == 0.0 {
                continue;
            }
            specs.push(Spec {
                src,
                dst,
                param: usize::from(index) - plan.generators[dst].params.start,
                depth: r.depth / 100.0,
                route,
            });
        }
        if specs.is_empty() {
            return Ok(Self::default());
        }
        let n = plan.generators.len();
        let bad = |route: usize, reason| {
            let r = &patch.routes[route];
            CompileError::BadRoute {
                route: format!("{} -> {}", r.src, r.dst),
                reason,
            }
        };

        let edges: Vec<(usize, usize)> = specs.iter().map(|s| (s.src, s.dst)).collect();
        let order = run_order(n, &edges).map_err(|i| {
            let r = &patch.routes[specs[i].route];
            CompileError::AudioCycle {
                route: format!("{} -> {}", r.src, r.dst),
            }
        })?;
        if let Some(m) = plan.sync_master {
            let at = |g: usize| order.iter().position(|&o| o == g);
            if (m + 1..n).any(|g| at(g) < at(m)) {
                let route = specs.iter().find(|s| s.dst == m).map_or(0, |s| s.route);
                return Err(bad(route, "audio_into_sync_master"));
            }
        }

        // Stable: one carrier's routes stay in patch route order, which is the order they sum in.
        specs.sort_by_key(|s| s.dst);
        let by_dst: Vec<core::ops::Range<usize>> = (0..n)
            .map(|g| specs.partition_point(|s| s.dst < g)..specs.partition_point(|s| s.dst <= g))
            .collect();
        if let Some(range) = by_dst.iter().find(|r| r.end - r.start > MAX_AUDIO_INPUTS) {
            return Err(bad(specs[range.start].route, "too_many_audio_routes"));
        }
        let mut is_source = vec![false; n];
        for s in &specs {
            is_source[s.src] = true;
        }
        let frames = f64::from(DEPTH_SMOOTHING_MS) * f64::from(plan.cfg.sample_rate) / 1000.0;
        Ok(Self {
            terms: specs
                .iter()
                .map(|s| Term {
                    src: s.src,
                    dst: s.dst,
                    param: s.param,
                    cur: s.depth,
                    prev: s.depth,
                })
                .collect(),
            tags: specs.iter().map(|s| s.route).collect(),
            ramps: specs
                .iter()
                .map(|s| Ramp {
                    target: s.depth,
                    ..Ramp::default()
                })
                .collect(),
            by_dst,
            is_source,
            order,
            pre: vec![[[0.0; SEG]; MAX_VOICES]; n],
            post: vec![[[0.0; SEG]; MAX_VOICES]; n],
            ramp_segments: (frames / SEG as f64)
                .round()
                .clamp(1.0, f64::from(u32::MAX)) as u32,
        })
    }

    /// Whether the plan has no audio routes: the engine then runs its plain generator loop.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Each route's [`DepthId`] with its patch route index.
    pub(crate) fn tags(&self) -> impl Iterator<Item = (DepthId, usize)> + '_ {
        self.tags
            .iter()
            .enumerate()
            .map(|(k, &route)| (DepthId(AUDIO_DEPTH_BASE + k as u16), route))
    }

    /// The generators in the order they run (test support).
    #[cfg(test)]
    pub(crate) fn order(&self) -> &[usize] {
        &self.order
    }

    /// Route `k`'s current depth as a fraction, for tests and telemetry.
    pub(crate) fn depth(&self, k: usize) -> Option<f32> {
        self.terms.get(k).map(|t| t.cur)
    }

    /// A depth event: route `k`'s depth moves to `depth_pct / 100` (clamped to ±100 %; NaN and unknown routes are
    /// ignored) over [`DEPTH_SMOOTHING_MS`], linearly; a new depth mid-ramp restarts the ramp from where the depth
    /// is. Computed as the compiler computes a depth, so once the ramp is over the term holds the bits a recompile
    /// would give.
    pub(crate) fn set_depth(&mut self, k: usize, depth_pct: f32) {
        let (Some(t), Some(r)) = (self.terms.get_mut(k), self.ramps.get_mut(k)) else {
            return;
        };
        if depth_pct.is_nan() {
            return;
        }
        let target = depth_pct.clamp(-100.0, 100.0) / 100.0;
        if r.left == 0 && t.cur.to_bits() == target.to_bits() {
            return;
        }
        r.target = target;
        r.left = self.ramp_segments;
        r.step = (target - t.cur) / self.ramp_segments as f32;
    }

    /// Step 3: every route's depth of the previous segment becomes `prev`, and the ramps in flight move one step.
    /// Within a segment the carrier ramps from `prev` to `cur` per sample.
    pub(crate) fn advance_depths(&mut self) {
        for (t, r) in self.terms.iter_mut().zip(&mut self.ramps) {
            t.prev = t.cur;
            if r.left > 0 {
                r.left -= 1;
                if r.left == 0 {
                    t.cur = r.target;
                } else {
                    t.cur += r.step;
                }
            }
        }
    }

    /// Land every depth ramp on its target at once ([`crate::Engine::reset`]).
    pub(crate) fn settle_depths(&mut self) {
        for (t, r) in self.terms.iter_mut().zip(&mut self.ramps) {
            if r.left > 0 {
                r.left = 0;
                t.cur = r.target;
            }
            t.prev = t.cur;
        }
    }

    /// Step 7 for a plan with audio routes: run the generators in route order, each into its own buffer, then add
    /// the buffers into `voice` in patch order. `process(g, ports)` renders generator `g` (an index into
    /// `Plan::generators`, which are in patch order) with the buffers in `ports`. `voice` must be zeroed for the
    /// active voices; `sync` is the hard-sync bus, handled here as the plain loop handles it.
    pub(crate) fn render(
        &mut self,
        active: &[VoiceIdx],
        sync_master: Option<usize>,
        voice: &mut VoiceBus,
        sync: &mut SyncBus,
        mut process: impl FnMut(usize, Ports<'_>),
    ) {
        let Self {
            terms,
            by_dst,
            is_source,
            order,
            pre,
            post,
            ..
        } = self;
        for &g in order.iter() {
            let (lo, rest) = pre.split_at_mut(g);
            let (Some((mine, hi)), Some(io)) = (rest.split_first_mut(), post.get_mut(g)) else {
                continue;
            };
            let pre_level = if is_source[g] {
                for &v in active {
                    mine[v] = [0.0; SEG];
                }
                Some(mine)
            } else {
                None
            };
            let mut audio_in = AudioIn::new();
            for t in &terms[by_dst[g].clone()] {
                let bus = match t.src.cmp(&g) {
                    core::cmp::Ordering::Less => &lo[t.src],
                    core::cmp::Ordering::Greater => &hi[t.src - g - 1],
                    core::cmp::Ordering::Equal => continue,
                };
                audio_in.push_route(t.param, bus, t.prev, t.cur);
            }
            let (sync_out, sync_in) = match sync_master {
                Some(m) if g == m => {
                    for &v in active {
                        sync[v] = [NO_SYNC; SEG];
                    }
                    (Some(&mut *sync), None)
                }
                Some(m) if g > m => (None, Some(&*sync)),
                _ => (None, None),
            };
            process(
                g,
                Ports {
                    io,
                    sync_out,
                    sync_in,
                    audio_in: (!audio_in.is_empty()).then_some(audio_in),
                    pre_level,
                },
            );
        }
        for bus in post.iter() {
            for &v in active {
                for (acc, x) in voice[v].iter_mut().zip(&bus[v]) {
                    *acc += *x;
                }
            }
        }
    }
}

/// What [`AudioRoutes::render`] hands `process` for one generator: the parts of its
/// [`VoiceCx`](crate::module::VoiceCx) the routes decide.
pub(crate) struct Ports<'a> {
    pub io: &'a mut VoiceBus,
    pub sync_out: Option<&'a mut SyncBus>,
    pub sync_in: Option<&'a SyncBus>,
    pub audio_in: Option<AudioIn<'a>>,
    pub pre_level: Option<&'a mut VoiceBus>,
}

/// One route into a carrier: the source's pre-level buffer, the carrier's parameter and the depth fraction at the
/// start and the end of the segment.
#[derive(Clone, Copy, Debug)]
struct Input<'a> {
    param: usize,
    bus: &'a VoiceBus,
    prev: f32,
    cur: f32,
}

/// The audio a carrier receives this segment ([`VoiceCx::audio_in`](crate::module::VoiceCx::audio_in)): the routes
/// that reach it, each the pre-level output of another generator, scaled by the route's depth. Built by the engine;
/// [`AudioIn::new`] and [`AudioIn::push_route`] build one for a module's unit test.
#[derive(Clone, Copy, Debug)]
pub struct AudioIn<'a> {
    inputs: [Option<Input<'a>>; MAX_AUDIO_INPUTS],
    len: usize,
}

impl Default for AudioIn<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> AudioIn<'a> {
    /// No routes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inputs: [None; MAX_AUDIO_INPUTS],
            len: 0,
        }
    }

    /// Add a route into parameter `param` from `source`'s pre-level output, with the depth (a fraction: the route's
    /// percentage over 100) at the start and the end of the segment; the scale ramps between them per sample.
    /// Returns `false` (and adds nothing) when [`MAX_AUDIO_INPUTS`] routes are in.
    pub fn push_route(
        &mut self,
        param: impl ParamIndex,
        source: &'a VoiceBus,
        depth_prev: f32,
        depth_cur: f32,
    ) -> bool {
        let Some(slot) = self.inputs.get_mut(self.len) else {
            return false;
        };
        *slot = Some(Input {
            param: param.param_index(),
            bus: source,
            prev: depth_prev,
            cur: depth_cur,
        });
        self.len += 1;
        true
    }

    /// Whether no route reaches this carrier.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether some route reaches parameter `param`.
    #[must_use]
    pub fn has(&self, param: impl ParamIndex) -> bool {
        let p = param.param_index();
        self.routes().any(|i| i.param == p)
    }

    /// Fill `out` with the audio reaching parameter `param` for voice `v`: the sum over the routes into it, in patch
    /// route order, of `depth × the source's pre-level sample`, per frame, the depth ramping from the previous
    /// segment's to this one's. Returns `false`, leaving `out` as it was, when no route reaches `param`.
    pub fn read(&self, param: impl ParamIndex, v: VoiceIdx, out: &mut Block) -> bool {
        let p = param.param_index();
        let mut first = true;
        for input in self.routes().filter(|i| i.param == p) {
            let x = &input.bus[v];
            if input.prev.to_bits() == input.cur.to_bits() {
                let d = input.cur;
                if first {
                    for (o, s) in out.iter_mut().zip(x) {
                        *o = *s * d;
                    }
                } else {
                    for (o, s) in out.iter_mut().zip(x) {
                        *o += *s * d;
                    }
                }
            } else {
                let step = input.cur - input.prev;
                if first {
                    for ((o, s), w) in out.iter_mut().zip(x).zip(RAMP) {
                        *o = *s * (input.prev + step * w);
                    }
                } else {
                    for ((o, s), w) in out.iter_mut().zip(x).zip(RAMP) {
                        *o += *s * (input.prev + step * w);
                    }
                }
            }
            first = false;
        }
        !first
    }

    fn routes(&self) -> impl Iterator<Item = &Input<'a>> {
        self.inputs[..self.len].iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{NoteState, PrepareCfg, VoiceCx};
    use crate::plan::compile;
    use enth_patch::catalogue::{NoiseParam, OscParam, Wave};
    use enth_patch::{ModuleId, ModuleInstance, ModuleKind, RouteAddr};

    const SEED: u64 = 0x5EED;
    const PM: usize = 1; // WtParam::Pm, in spec order

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s).unwrap()
    }

    fn route(src: &str, dst: &str, depth: f32) -> Route {
        Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        }
    }

    /// `ids` as generators in patch order (`wt…` is a `wt`, `osc…` an `osc`, `noise…` a `noise`), with `routes`.
    fn patch(ids: &[&str], routes: &[Route]) -> Patch {
        let mut p = Patch::init();
        p.generators.clear();
        for name in ids {
            let kind = if name.starts_with("wt") {
                ModuleKind::Wt
            } else if name.starts_with("noise") {
                ModuleKind::Noise
            } else {
                ModuleKind::Osc
            };
            p.generators.push(ModuleInstance::new(id(name), kind));
        }
        p.routes = routes.to_vec();
        p
    }

    fn order_of(p: &Patch) -> Vec<usize> {
        let (plan, _) = compile(p, &PrepareCfg::new(48_000.0)).expect("compiles");
        plan.audio.order().to_vec()
    }

    fn bad_reason(p: &Patch) -> &'static str {
        match compile(p, &PrepareCfg::new(48_000.0)) {
            Err(CompileError::BadRoute { reason, .. }) => reason,
            other => panic!("expected a bad route, got {:?}", other.err()),
        }
    }

    #[test]
    fn run_order_is_kahn_with_the_lowest_index_first() {
        assert_eq!(run_order(3, &[]), Ok(vec![0, 1, 2]));
        // 1 feeds 0: 1 first, then the rest in index order.
        assert_eq!(run_order(2, &[(1, 0)]), Ok(vec![1, 0]));
        // 2 feeds 0: 1 and 2 are ready, 1 is lower; then 0 is ready and lower than nothing else.
        assert_eq!(run_order(3, &[(2, 0)]), Ok(vec![1, 2, 0]));
        // A chain against patch order, and a diamond.
        assert_eq!(run_order(3, &[(2, 1), (1, 0)]), Ok(vec![2, 1, 0]));
        assert_eq!(
            run_order(4, &[(3, 1), (3, 2), (1, 0), (2, 0)]),
            Ok(vec![3, 1, 2, 0])
        );
    }

    #[test]
    fn run_order_names_an_edge_on_the_cycle() {
        assert_eq!(run_order(1, &[(0, 0)]), Err(0));
        assert_eq!(run_order(2, &[(0, 1), (1, 0)]), Err(0));
        // 0 -> 1 is fine (0 has nothing waiting); the loop is 1 <-> 2, and the edge into it is not named.
        assert_eq!(run_order(3, &[(0, 1), (1, 2), (2, 1)]), Err(1));
        // 3 hangs off the loop and is never ready, but it is not on it.
        assert_eq!(run_order(4, &[(1, 2), (2, 1), (2, 3)]), Err(0));
    }

    #[test]
    fn the_carrier_runs_after_its_source_wherever_the_patch_puts_them() {
        // B feeds A: B first, though A is first in the patch.
        let p = patch(&["wt1", "osc1"], &[route("osc1", "wt1.pm", 50.0)]);
        assert_eq!(order_of(&p), [1, 0]);
        // A chain against patch order: osc1 -> wt2 -> wt1.
        let p = patch(
            &["wt1", "wt2", "osc1"],
            &[route("wt2", "wt1.pm", 20.0), route("osc1", "wt2.am", 30.0)],
        );
        assert_eq!(order_of(&p), [2, 1, 0]);
    }

    #[test]
    fn ties_keep_patch_order() {
        // Two carriers fed by the same source: after it they run in patch order.
        let p = patch(
            &["wt1", "osc1", "wt2"],
            &[route("osc1", "wt2.pm", 10.0), route("osc1", "wt1.pm", 10.0)],
        );
        assert_eq!(order_of(&p), [1, 0, 2]);
        // Generators the routes do not constrain keep their relative patch order.
        let p = patch(
            &["wt1", "osc1", "osc2", "wt2"],
            &[route("osc2", "wt1.pm", 10.0)],
        );
        assert_eq!(order_of(&p), [1, 2, 0, 3]);
    }

    #[test]
    fn a_patch_without_audio_routes_has_no_audio_state() {
        let p = patch(&["wt1", "osc1"], &[]);
        let (plan, map) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        assert!(plan.audio.is_empty());
        assert!(plan.audio.pre.is_empty() && plan.audio.post.is_empty());
        assert_eq!(
            map.depth(&RouteAddr {
                src: "osc1".parse().unwrap(),
                dst: "wt1.pm".parse().unwrap(),
            }),
            None
        );
    }

    #[test]
    fn loops_and_self_routes_are_audio_cycles() {
        for routes in [
            vec![route("wt1", "wt1.pm", 10.0)],
            vec![route("wt1", "wt2.pm", 10.0), route("wt2", "wt1.am", 10.0)],
        ] {
            let p = patch(&["wt1", "wt2"], &routes);
            let e = compile(&p, &PrepareCfg::new(48_000.0)).unwrap_err();
            assert_eq!(e.code(), "audio_cycle", "{e}");
            let CompileError::AudioCycle { route } = &e else {
                panic!("{e:?}")
            };
            assert!(route.starts_with("wt"), "{route}");
            assert!(e.to_string().contains("loop"), "{e}");
        }
        // A disabled generator leaves the loop open for the engine (validate refuses it all the same).
        let mut p = patch(
            &["wt1", "wt2"],
            &[route("wt1", "wt2.pm", 10.0), route("wt2", "wt1.am", 10.0)],
        );
        p.generators[1].enabled = false;
        let (plan, _) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        assert!(plan.audio.is_empty());
    }

    #[test]
    fn the_engine_refuses_the_pairings_the_contract_forbids() {
        // A generator into a parameter that is not an audio input.
        for dst in ["wt1.level", "osc2.level", "voice.pan", "master.level"] {
            let p = patch(&["osc1", "osc2", "wt1"], &[route("osc1", dst, 10.0)]);
            assert_eq!(bad_reason(&p), "not_audio_dest", "{dst}");
        }
        // Anything but a generator into an audio input.
        for src in ["env1", "velocity", "key", "modwheel", "macro1"] {
            let p = patch(&["osc1", "wt1"], &[route(src, "wt1.pm", 10.0)]);
            assert_eq!(bad_reason(&p), "not_audio_source", "{src}");
        }
        let p = patch(&["osc1", "wt1"], &[route("osc1", "wt9.pm", 10.0)]);
        assert_eq!(bad_reason(&p), "unknown_destination");
        let p = patch(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 101.0)]);
        assert_eq!(bad_reason(&p), "depth_out_of_range");
        // Disabled ends and depth 0 contribute nothing, and are no error.
        let mut p = patch(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 10.0)]);
        p.generators[1].enabled = false;
        assert!(
            compile(&p, &PrepareCfg::new(48_000.0))
                .unwrap()
                .0
                .audio
                .is_empty()
        );
        let p = patch(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 0.0)]);
        assert!(
            compile(&p, &PrepareCfg::new(48_000.0))
                .unwrap()
                .0
                .audio
                .is_empty()
        );
    }

    fn one_voice() -> (NoteState, [usize; 1]) {
        let mut n = NoteState::new();
        n.pitch_prev[0] = 57.0;
        n.pitch_cur[0] = 57.0;
        n.velocity[0] = 0.8;
        n.gate[0] = true;
        n.trigger[0] = true;
        n.fresh[0] = true;
        (n, [0])
    }

    /// Render `segments` segments of `plan` through [`AudioRoutes::render`] with the real modules, the way the engine
    /// does, but recording what each carrier received for parameter `PM` (generator, voice 0's block), the order the
    /// generators ran in, and the buses afterwards.
    struct Seen {
        order: Vec<usize>,
        pm: Vec<(usize, Block)>,
        voice: Block,
    }

    fn run(plan: &mut Plan, segments: usize) -> Vec<Seen> {
        let (mut notes, active) = one_voice();
        plan.start_voice(0, &notes, SEED);
        let mut out = Vec::new();
        for _ in 0..segments {
            let Plan {
                generators,
                voice_table,
                audio,
                sync_master,
                ..
            } = plan;
            voice_table.advance(&active, &notes.trigger);
            audio.advance_depths();
            let mut voice: VoiceBus = [[0.0; SEG]; MAX_VOICES];
            let mut sync: SyncBus = [[NO_SYNC; SEG]; MAX_VOICES];
            let mut seen = Seen {
                order: Vec::new(),
                pm: Vec::new(),
                voice: [0.0; SEG],
            };
            audio.render(&active, *sync_master, &mut voice, &mut sync, |g, ports| {
                seen.order.push(g);
                if let Some(input) = &ports.audio_in {
                    let mut block = [f32::NAN; SEG];
                    if input.read(PM, 0, &mut block) {
                        seen.pm.push((g, block));
                    }
                }
                let slot = &mut generators[g];
                slot.module.process(&mut VoiceCx {
                    voices: &active,
                    notes: &notes,
                    params: voice_table.view(slot.params),
                    io: ports.io,
                    sync_out: ports.sync_out,
                    sync_in: ports.sync_in,
                    audio_in: ports.audio_in,
                    pre_level: ports.pre_level,
                });
            });
            seen.voice = voice[0];
            out.push(seen);
            notes.trigger[0] = false;
            notes.fresh[0] = false;
        }
        out
    }

    #[test]
    fn a_route_into_wt_pm_delivers_the_pre_level_samples_scaled_by_depth() {
        let mut p = patch(&["wt1", "osc1"], &[route("osc1", "wt1.pm", 60.0)]);
        let osc = &mut p.generators[1].values;
        osc[OscParam::Wave.index()] = Wave::Sine.index() as f32;
        osc[OscParam::Level.index()] = 50.0;
        let (mut plan, map) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        assert_eq!(plan.audio.order(), [1, 0]);
        let seen = run(&mut plan, 3);
        let depth = 60.0f32 / 100.0;
        for (k, s) in seen.iter().enumerate() {
            assert_eq!(s.order, [1, 0], "the source first");
            let [(g, pm)] = s.pm.as_slice() else {
                panic!("one carrier saw the route")
            };
            assert_eq!(*g, 0, "the carrier is wt1, the first generator");
            // The source's buffers: pre-level in `pre`, post-level (the level 50 % applied) in `post`.
            let pre = &plan.audio.pre[1][0];
            let post = &plan.audio.post[1][0];
            if k == seen.len() - 1 {
                for i in 0..SEG {
                    assert_eq!(pm[i].to_bits(), (pre[i] * depth).to_bits(), "frame {i}");
                    assert_eq!(post[i].to_bits(), (pre[i] * 0.5).to_bits(), "frame {i}");
                }
                assert!(pre.iter().any(|x| x.abs() > 0.1), "the oscillator sounds");
            }
            assert!(pm.iter().all(|x| x.is_finite()));
        }
        // The route is a depth event target with an id of its own.
        let key = RouteAddr {
            src: "osc1".parse().unwrap(),
            dst: "wt1.pm".parse().unwrap(),
        };
        let id = map.depth(&key).expect("an audio route has a depth id");
        assert!(id.0 >= AUDIO_DEPTH_BASE);
        let shown = plan.route_depth(id).expect("the depth reads back");
        assert!((shown - 60.0).abs() < 1e-4, "{shown}");
    }

    #[test]
    fn a_silent_generator_still_feeds_its_routes() {
        // Level 0: nothing reaches the voice, but the route carries the full oscillator (the FM operator pattern).
        let mut p = patch(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 100.0)]);
        p.generators[0].values[OscParam::Level.index()] = 0.0;
        let (mut plan, _) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        let seen = run(&mut plan, 2);
        let last = seen.last().unwrap();
        assert!(
            last.voice.iter().all(|x| *x == 0.0),
            "level 0 is silent in the voice"
        );
        let [(_, pm)] = last.pm.as_slice() else {
            panic!("the carrier saw the route")
        };
        assert!(
            pm.iter().any(|x| x.abs() > 0.1),
            "yet the route carries the oscillator"
        );
    }

    #[test]
    fn noise_is_a_source_too_and_two_routes_into_one_parameter_sum_in_route_order() {
        let mut p = patch(
            &["noise1", "osc1", "wt1"],
            &[
                route("osc1", "wt1.pm", 30.0),
                route("noise1", "wt1.pm", -20.0),
            ],
        );
        p.generators[1].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
        p.generators[0].values[NoiseParam::Level.index()] = 25.0;
        let (mut plan, _) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        let seen = run(&mut plan, 2);
        let last = seen.last().unwrap();
        let [(g, pm)] = last.pm.as_slice() else {
            panic!("one carrier")
        };
        assert_eq!(*g, 2);
        let (noise, osc) = (&plan.audio.pre[0][0], &plan.audio.pre[1][0]);
        for i in 0..SEG {
            // Patch route order: osc1's route first, then noise1's.
            let want = osc[i] * 0.3 + noise[i] * -0.2;
            assert_eq!(pm[i].to_bits(), want.to_bits(), "frame {i}");
        }
        assert!(
            noise.iter().any(|x| x.abs() > 0.1),
            "noise level 25 % is not applied to the route"
        );
    }

    #[test]
    fn the_voice_sum_stays_in_patch_order() {
        // Run order is [osc2, osc1... ]: the same generators with and without a route into a silent wt sum alike.
        let mut with = patch(&["wt1", "osc1", "osc2"], &[route("osc2", "wt1.pm", 40.0)]);
        with.generators[2].values[OscParam::Fine.index()] = 11.0;
        let mut without = with.clone();
        without.routes.clear();
        let (mut a, _) = compile(&with, &PrepareCfg::new(48_000.0)).unwrap();
        let (mut b, _) = compile(&without, &PrepareCfg::new(48_000.0)).unwrap();
        assert!(!a.audio.is_empty() && b.audio.is_empty());
        let seen = run(&mut a, 6);
        // The plain path, as the engine's loop writes it.
        let (mut notes, active) = one_voice();
        b.start_voice(0, &notes, SEED);
        for s in &seen {
            let Plan {
                generators,
                voice_table,
                ..
            } = &mut b;
            voice_table.advance(&active, &notes.trigger);
            let mut voice: VoiceBus = [[0.0; SEG]; MAX_VOICES];
            let mut scratch: VoiceBus = [[0.0; SEG]; MAX_VOICES];
            for slot in generators.iter_mut() {
                slot.module.process(&mut VoiceCx {
                    voices: &active,
                    notes: &notes,
                    params: voice_table.view(slot.params),
                    io: &mut scratch,
                    sync_out: None,
                    sync_in: None,
                    audio_in: None,
                    pre_level: None,
                });
                for (acc, x) in voice[0].iter_mut().zip(&scratch[0]) {
                    *acc += *x;
                }
            }
            for (i, (got, want)) in s.voice.iter().zip(&voice[0]).enumerate() {
                assert_eq!(got.to_bits(), want.to_bits(), "frame {i}");
            }
            notes.trigger[0] = false;
            notes.fresh[0] = false;
        }
    }

    #[test]
    fn the_voice_sum_is_added_in_patch_order_whatever_the_run_order() {
        // Values chosen so that f32 addition is not associative: in patch order ((1e8 + 3) + 3) stays 1e8, while
        // adding the two 3s first (the run order below) gives 1e8 + 8.
        let p = patch(&["wt1", "osc1", "osc2"], &[route("osc2", "wt1.pm", 40.0)]);
        let (mut plan, _) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        assert_eq!(plan.audio.order(), [1, 2, 0]);
        let values = [1.0e8f32, 3.0, 3.0];
        let mut voice: VoiceBus = [[0.0; SEG]; MAX_VOICES];
        let mut sync: SyncBus = [[NO_SYNC; SEG]; MAX_VOICES];
        let mut ran = Vec::new();
        plan.audio
            .render(&[0], None, &mut voice, &mut sync, |g, ports| {
                ran.push(g);
                ports.io[0] = [values[g]; SEG];
            });
        assert_eq!(ran, [1, 2, 0]);
        let want = ((0.0f32 + values[0]) + values[1]) + values[2];
        assert_eq!(want, 1.0e8, "patch order loses both 3s");
        assert!(voice[0].iter().all(|x| x.to_bits() == want.to_bits()));
    }

    #[test]
    fn audio_in_ramps_the_depth_and_sums() {
        let mut a: VoiceBus = [[0.0; SEG]; MAX_VOICES];
        let mut b: VoiceBus = [[0.0; SEG]; MAX_VOICES];
        for i in 0..SEG {
            a[3][i] = 1.0 + i as f32;
            b[3][i] = -2.0;
        }
        let mut input = AudioIn::new();
        assert!(input.is_empty() && !input.has(PM));
        assert!(input.push_route(PM, &a, 0.25, 0.75));
        assert!(input.push_route(PM, &b, 0.5, 0.5));
        assert!(input.push_route(0usize, &b, 1.0, 1.0));
        assert!(input.has(PM) && input.has(0usize) && !input.has(2usize));
        let mut out = [f32::NAN; SEG];
        assert!(input.read(PM, 3, &mut out));
        for i in 0..SEG {
            let d = 0.25 + (0.75 - 0.25) * RAMP[i];
            let want = a[3][i] * d + b[3][i] * 0.5;
            assert_eq!(out[i].to_bits(), want.to_bits(), "frame {i}");
        }
        let mut untouched = [7.0; SEG];
        assert!(!input.read(2usize, 3, &mut untouched));
        assert_eq!(untouched, [7.0; SEG]);
        let mut full = AudioIn::new();
        for _ in 0..MAX_AUDIO_INPUTS {
            assert!(full.push_route(PM, &a, 1.0, 1.0));
        }
        assert!(!full.push_route(PM, &a, 1.0, 1.0));
    }

    #[test]
    fn a_depth_event_ramps_per_sample_and_lands_on_the_compiled_bits() {
        let p = patch(&["osc1", "wt1"], &[route("osc1", "wt1.pm", 80.0)]);
        let (mut plan, map) = compile(&p, &PrepareCfg::new(48_000.0)).unwrap();
        let key = RouteAddr {
            src: "osc1".parse().unwrap(),
            dst: "wt1.pm".parse().unwrap(),
        };
        let id = map.depth(&key).unwrap();
        plan.set_route_depth(id, 20.0);
        // 10 ms at 48 kHz is 480 frames = 15 segments.
        let mut seen_prev_ne_cur = false;
        for _ in 0..15 {
            plan.audio.advance_depths();
            let t = plan.audio.terms[0];
            seen_prev_ne_cur |= t.prev.to_bits() != t.cur.to_bits();
        }
        assert!(seen_prev_ne_cur, "the segments in between ramp per sample");
        assert_eq!(
            plan.audio.terms[0].cur.to_bits(),
            (20.0f32 / 100.0).to_bits()
        );
        plan.audio.advance_depths();
        let t = plan.audio.terms[0];
        assert_eq!(
            t.prev.to_bits(),
            t.cur.to_bits(),
            "settled: a constant scale"
        );
        // NaN is ignored, the range is clamped, and an unknown id does nothing.
        plan.set_route_depth(id, f32::NAN);
        plan.set_route_depth(DepthId(AUDIO_DEPTH_BASE + 9), 5.0);
        plan.set_route_depth(id, 250.0);
        plan.audio.settle_depths();
        assert_eq!(plan.route_depth(id), Some(100.0));
        assert_eq!(plan.route_depth(DepthId(AUDIO_DEPTH_BASE + 9)), None);
    }
}
