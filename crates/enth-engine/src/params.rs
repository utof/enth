//! Parameter values inside a plan: the flat per-voice and global tables, their smoothers, [`ParamId`], and the
//! read-only views modules get ([`VoiceParams`], [`GlobalParams`]).
//!
//! Every parameter of every module in a plan, plus the built-in `voice.*` and `master.*` (and, in the global table,
//! the eight macros and the mod wheel), has a slot in one of two tables, chosen by the rate of its owner: modules
//! that run per voice (and `voice.*`) in the voice table, global modules (and `master.*`) in the global table. A
//! slot holds:
//!
//! - `base`: the plain value from the patch, or the latest value event (host automation, a UI edit);
//! - a [`Smoother`] that follows `base` in the normalised domain, as the catalogue's `Smoothing` says;
//! - `cur`: this segment's value as modules see it (smoothed, then modulated), per voice in the voice table;
//! - `prev`: the previous segment's `cur`, for per-sample ramps (the SVF interpolates its coefficients from `prev`
//!   to `cur`). On a voice's first segment after a trigger (a fresh voice or a mono retrigger), `prev == cur`.
//!
//! **Settled parameters are exact.** With no smoothing in flight and no route, `cur` is `base` itself, the plain
//! value from the patch, bit for bit; it never goes through `from_norm(to_norm(x))`, which is not guaranteed to
//! round trip. A smoother in flight works in the normalised domain (`ParamSpec::to_norm`/`from_norm`, pinned
//! `libm`) and snaps back to the exact `base` on its last step. A parameter with routes is always
//! `from_norm(n)` with `n = clamp(smoothed_norm + Σ depth · source, 0, 1)` ([`crate::modulation`]).
//!
//! Step 3 of the segment ([`VoiceTable::advance`], [`GlobalTable::advance`]) advances every smoother once and
//! writes `cur` for the parameters without routes; steps 4–6 write `cur` for the routed ones.

use enth_dsp::util::math::exp;
use enth_params::{ParamSpec, Smoothing};
use enth_patch::catalogue::{
    AdsrParam, CompParam, LfoParam, MasterParam, NoiseParam, OscParam, ReverbParam, SvfParam,
    VoiceParam,
};

use crate::module::{MAX_VOICES, SEG, VoiceIdx};

/// Where a parameter lives in a plan. Stable for the life of a plan; a new plan (structural edit) has new ids, which
/// the plan's [`PlanMap`](crate::plan::PlanMap) maps from addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamId {
    /// A slot in the per-voice table (modules that run per voice, and `voice.*`).
    Voice(u16),
    /// A slot in the global table (global modules, and `master.*`).
    Global(u16),
}

/// A parameter's index within its module's table, in spec order. Implemented for the catalogue's index enums
/// (`OscParam`, `SvfParam`, …) and for `usize`, so modules write `params.cur(OscParam::Semi, v)`.
pub trait ParamIndex: Copy {
    /// The index in spec order.
    fn param_index(self) -> usize;
}

impl ParamIndex for usize {
    #[inline(always)]
    fn param_index(self) -> usize {
        self
    }
}

macro_rules! param_index {
    ($($t:ty),* $(,)?) => {
        $(impl ParamIndex for $t {
            #[inline(always)]
            fn param_index(self) -> usize {
                self.index()
            }
        })*
    };
}

param_index!(
    OscParam,
    NoiseParam,
    SvfParam,
    AdsrParam,
    LfoParam,
    ReverbParam,
    CompParam,
    VoiceParam,
    MasterParam,
);

/// One module's parameters in the per-voice table. Cheap to copy.
#[derive(Clone, Copy, Debug)]
pub struct VoiceParams<'a> {
    base: &'a [f32],
    cur: &'a [[f32; MAX_VOICES]],
    prev: &'a [[f32; MAX_VOICES]],
}

impl<'a> VoiceParams<'a> {
    /// A view over caller-owned arrays, one entry per parameter in spec order (for module unit tests and harnesses;
    /// the engine builds views from its tables).
    ///
    /// # Panics
    /// If the three slices differ in length.
    #[must_use]
    pub fn from_slices(
        base: &'a [f32],
        cur: &'a [[f32; MAX_VOICES]],
        prev: &'a [[f32; MAX_VOICES]],
    ) -> Self {
        assert!(
            base.len() == cur.len() && base.len() == prev.len(),
            "one entry per parameter in every slice"
        );
        Self { base, cur, prev }
    }

    /// The parameter's base value: the patch value or the latest value event, before smoothing and modulation. The
    /// value to use in `start_voice`, and for discrete parameters (they are never smoothed or modulated).
    #[inline(always)]
    #[must_use]
    pub fn base(&self, p: impl ParamIndex) -> f32 {
        self.base[p.param_index()]
    }

    /// This segment's value for voice `v`, plain units.
    #[inline(always)]
    #[must_use]
    pub fn cur(&self, p: impl ParamIndex, v: VoiceIdx) -> f32 {
        self.cur[p.param_index()][v]
    }

    /// The previous segment's value for voice `v` (equal to `cur` on a voice's first segment).
    #[inline(always)]
    #[must_use]
    pub fn prev(&self, p: impl ParamIndex, v: VoiceIdx) -> f32 {
        self.prev[p.param_index()][v]
    }

    /// The number of parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.base.len()
    }

    /// Whether the module has no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }
}

/// One global module's parameters in the global table. Cheap to copy.
#[derive(Clone, Copy, Debug)]
pub struct GlobalParams<'a> {
    base: &'a [f32],
    cur: &'a [f32],
    prev: &'a [f32],
}

impl<'a> GlobalParams<'a> {
    /// A view over caller-owned slices, one entry per parameter in spec order (for module unit tests).
    ///
    /// # Panics
    /// If the three slices differ in length.
    #[must_use]
    pub fn from_slices(base: &'a [f32], cur: &'a [f32], prev: &'a [f32]) -> Self {
        assert!(
            base.len() == cur.len() && base.len() == prev.len(),
            "one entry per parameter in every slice"
        );
        Self { base, cur, prev }
    }

    /// The base value (see [`VoiceParams::base`]).
    #[inline(always)]
    #[must_use]
    pub fn base(&self, p: impl ParamIndex) -> f32 {
        self.base[p.param_index()]
    }

    /// This segment's value, plain units.
    #[inline(always)]
    #[must_use]
    pub fn cur(&self, p: impl ParamIndex) -> f32 {
        self.cur[p.param_index()]
    }

    /// The previous segment's value (equal to `cur` on the first segment).
    #[inline(always)]
    #[must_use]
    pub fn prev(&self, p: impl ParamIndex) -> f32 {
        self.prev[p.param_index()]
    }

    /// The number of parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.base.len()
    }

    /// Whether the module has no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }
}

/// A contiguous run of slots in one table: one module's parameters, in spec order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn range(self) -> core::ops::Range<usize> {
        self.start..self.start + self.len
    }
}

/// A one-pole smoother is settled once it is this close to its target, in the normalised domain.
const ONE_POLE_SETTLED: f32 = 1e-6;

/// How a [`Smoother`] moves, with its constants for the plan's sample rate.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    /// Steps at the next segment boundary.
    Step,
    /// A linear ramp of `segments` steps (constant time: a new target mid-ramp restarts the full ramp from where
    /// the value is).
    Linear { segments: u32 },
    /// `n += (target − n) · coef` once per segment, until within [`ONE_POLE_SETTLED`].
    OnePole { coef: f32 },
}

impl Motion {
    fn new(smoothing: Smoothing, sample_rate: f32) -> Self {
        let frames = |ms: f32| f64::from(ms) * f64::from(sample_rate) / 1000.0;
        match smoothing {
            Smoothing::None => Motion::Step,
            Smoothing::Linear { ms } => {
                let segments = (frames(ms) / SEG as f64).round().max(1.0);
                Motion::Linear {
                    segments: segments.min(f64::from(u32::MAX)) as u32,
                }
            }
            Smoothing::OnePole { ms } => {
                let n = frames(ms);
                let coef = if n <= SEG as f64 {
                    1.0
                } else {
                    (1.0 - exp(-(SEG as f64) / n)) as f32
                };
                Motion::OnePole { coef }
            }
        }
    }
}

/// Follows one parameter's base value in the normalised domain, once per segment. Settled, it delivers the exact
/// plain `base`; in flight, `from_norm` of its position. Never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Smoother {
    /// The bits of the base value `target` was computed from.
    base_bits: u32,
    /// Where the value is, normalised.
    norm: f32,
    /// `to_norm(base)`.
    target: f32,
    /// Linear: the per-segment increment.
    step: f32,
    /// Segments left for a linear ramp; 1 while a one-pole moves; 0 when settled.
    left: u32,
    motion: Motion,
}

impl Smoother {
    /// Settled at `value` (no initial ramp at load).
    pub fn new(spec: &ParamSpec, value: f32, sample_rate: f32) -> Self {
        let n = spec.to_norm(value);
        Self {
            base_bits: value.to_bits(),
            norm: n,
            target: n,
            step: 0.0,
            left: 0,
            motion: Motion::new(spec.smoothing, sample_rate),
        }
    }

    /// Whether the value equals the base exactly (no ramp in flight).
    #[inline(always)]
    pub fn is_settled(&self) -> bool {
        self.left == 0
    }

    /// The position in the normalised domain (`to_norm(base)` when settled): the start of a modulated value.
    #[inline(always)]
    pub fn norm(&self) -> f32 {
        self.norm
    }

    /// Advance one segment towards `base` and return this segment's plain value: `base` itself when settled.
    #[inline(always)]
    pub fn advance(&mut self, spec: &ParamSpec, base: f32) -> f32 {
        if base.to_bits() != self.base_bits {
            self.retarget(spec, base);
        }
        if self.left == 0 {
            return base;
        }
        match self.motion {
            Motion::Step => {
                self.norm = self.target;
                self.left = 0;
            }
            Motion::Linear { .. } => {
                self.left -= 1;
                self.norm = if self.left == 0 {
                    self.target
                } else {
                    self.norm + self.step
                };
            }
            Motion::OnePole { coef } => {
                self.norm += (self.target - self.norm) * coef;
                if (self.target - self.norm).abs() <= ONE_POLE_SETTLED {
                    self.norm = self.target;
                    self.left = 0;
                }
            }
        }
        if self.left == 0 {
            base
        } else {
            spec.from_norm(self.norm)
        }
    }

    fn retarget(&mut self, spec: &ParamSpec, base: f32) {
        self.base_bits = base.to_bits();
        self.target = spec.to_norm(base);
        match self.motion {
            Motion::Step => {
                self.norm = self.target;
                self.left = 0;
            }
            Motion::Linear { segments } => {
                self.left = segments;
                self.step = (self.target - self.norm) / segments as f32;
            }
            Motion::OnePole { .. } => self.left = 1,
        }
    }
}

/// When a table may skip its full pass: nothing written, nothing smoothing, and `prev` already level with `cur`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Activity {
    /// A base value was written (a value event, a carry) since the last pass.
    dirty: bool,
    /// Smoothers in flight after the last pass.
    moving: u32,
    /// The last full pass changed values, so one more brings `prev` level with `cur`.
    sync: bool,
}

impl Activity {
    #[inline(always)]
    fn quiet(self) -> bool {
        !self.dirty && self.moving == 0 && !self.sync
    }

    /// Start a full pass; whether values may change in it.
    #[inline(always)]
    fn begin(&mut self) -> bool {
        let changing = self.dirty || self.moving > 0;
        self.dirty = false;
        changing
    }

    #[inline(always)]
    fn end(&mut self, changing: bool, moving: u32) {
        self.sync = changing;
        self.moving = moving;
    }
}

/// The per-voice table. Sized at compile time; nothing here allocates afterwards.
#[derive(Debug)]
pub(crate) struct VoiceTable {
    pub specs: Vec<&'static ParamSpec>,
    pub base: Vec<f32>,
    pub cur: Vec<[f32; MAX_VOICES]>,
    pub prev: Vec<[f32; MAX_VOICES]>,
    pub smooth: Vec<Smoother>,
    /// The parameter has routes: steps 4–6 write its `cur`.
    pub modulated: Vec<bool>,
    /// The routed parameters' indices.
    pub routed: Vec<usize>,
    activity: Activity,
    sample_rate: f32,
}

impl VoiceTable {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            specs: Vec::new(),
            base: Vec::new(),
            cur: Vec::new(),
            prev: Vec::new(),
            smooth: Vec::new(),
            modulated: Vec::new(),
            routed: Vec::new(),
            activity: Activity::default(),
            sample_rate,
        }
    }

    /// Append one module's values (already validated) and return their span.
    pub fn push(&mut self, specs: &'static [ParamSpec], values: &[f32]) -> Span {
        let start = self.base.len();
        for (spec, &v) in specs.iter().zip(values) {
            self.specs.push(spec);
            self.base.push(v);
            self.cur.push([v; MAX_VOICES]);
            self.prev.push([v; MAX_VOICES]);
            self.smooth.push(Smoother::new(spec, v, self.sample_rate));
            self.modulated.push(false);
        }
        Span {
            start,
            len: specs.len(),
        }
    }

    pub fn view(&self, span: Span) -> VoiceParams<'_> {
        let r = span.range();
        VoiceParams {
            base: &self.base[r.clone()],
            cur: &self.cur[r.clone()],
            prev: &self.prev[r],
        }
    }

    /// Step 3: advance every smoother one segment; for the active voices, `prev ← cur` (or this segment's value
    /// for a voice that `reset` flags: a trigger restarts its ramps) and, for parameters without routes,
    /// `cur ← value` (the exact base when settled). Routed parameters get `cur` in steps 4–6.
    ///
    /// When the table is quiet (no value written, no smoother moving, `prev` level with `cur`), every active voice
    /// already holds the settled value, so only restarting voices are written (their slot may hold a value from
    /// before a change it missed) and the routed parameters' `prev ← cur`: the same values as the full pass.
    pub fn advance(&mut self, voices: &[VoiceIdx], reset: &[bool; MAX_VOICES]) {
        if self.activity.quiet() {
            if voices.iter().any(|&v| reset[v]) {
                for p in 0..self.base.len() {
                    if !self.modulated[p] {
                        let b = self.base[p];
                        let (cur, prev) = (&mut self.cur[p], &mut self.prev[p]);
                        for &v in voices {
                            if reset[v] {
                                cur[v] = b;
                                prev[v] = b;
                            }
                        }
                    }
                }
            }
            for &p in &self.routed {
                let (cur, prev) = (&self.cur[p], &mut self.prev[p]);
                for &v in voices {
                    prev[v] = cur[v];
                }
            }
            return;
        }
        let changing = self.activity.begin();
        let mut moving = 0;
        for p in 0..self.base.len() {
            let value = self.smooth[p].advance(self.specs[p], self.base[p]);
            moving += u32::from(!self.smooth[p].is_settled());
            let (cur, prev) = (&mut self.cur[p], &mut self.prev[p]);
            if self.modulated[p] {
                for &v in voices {
                    prev[v] = cur[v];
                }
            } else {
                for &v in voices {
                    prev[v] = if reset[v] { value } else { cur[v] };
                    cur[v] = value;
                }
            }
        }
        self.activity.end(changing, moving);
    }

    /// Mark parameter `p` as routed (compile time).
    pub fn route(&mut self, p: usize) {
        if !self.modulated[p] {
            self.modulated[p] = true;
            self.routed.push(p);
        }
    }

    /// Set a base value from a value event, clamped to the spec (NaN is ignored: it is a bug upstream). The
    /// smoother follows from the next segment.
    pub fn set(&mut self, index: usize, value: f32) {
        if let (Some(spec), Some(base)) = (self.specs.get(index), self.base.get_mut(index))
            && !value.is_nan()
        {
            *base = spec.clamp(value);
            self.activity.dirty = true;
        }
    }

    /// Carry the running state (values per voice and the smoother) of `len` parameters from `old` (the same specs,
    /// from the plan being replaced). The base stays this plan's: if it differs, the smoother ramps to it.
    pub fn carry(&mut self, dst: Span, old: &VoiceTable, src: Span) {
        let n = dst.len.min(src.len);
        for i in 0..n {
            let (d, s) = (dst.start + i, src.start + i);
            self.cur[d] = old.cur[s];
            self.prev[d] = old.prev[s];
            self.smooth[d] = old.smooth[s];
        }
        self.activity.dirty = true;
        self.activity.sync = true;
    }
}

/// The global table.
#[derive(Debug)]
pub(crate) struct GlobalTable {
    pub specs: Vec<&'static ParamSpec>,
    pub base: Vec<f32>,
    pub cur: Vec<f32>,
    pub prev: Vec<f32>,
    pub smooth: Vec<Smoother>,
    /// The parameter has routes: steps 4 and 6 write its `cur`.
    pub modulated: Vec<bool>,
    /// The routed parameters' indices.
    pub routed: Vec<usize>,
    activity: Activity,
    sample_rate: f32,
}

impl GlobalTable {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            specs: Vec::new(),
            base: Vec::new(),
            cur: Vec::new(),
            prev: Vec::new(),
            smooth: Vec::new(),
            modulated: Vec::new(),
            routed: Vec::new(),
            activity: Activity::default(),
            sample_rate,
        }
    }

    pub fn push(&mut self, specs: &'static [ParamSpec], values: &[f32]) -> Span {
        let start = self.base.len();
        for (spec, &v) in specs.iter().zip(values) {
            self.specs.push(spec);
            self.base.push(v);
            self.cur.push(v);
            self.prev.push(v);
            self.smooth.push(Smoother::new(spec, v, self.sample_rate));
            self.modulated.push(false);
        }
        Span {
            start,
            len: specs.len(),
        }
    }

    pub fn view(&self, span: Span) -> GlobalParams<'_> {
        let r = span.range();
        GlobalParams {
            base: &self.base[r.clone()],
            cur: &self.cur[r.clone()],
            prev: &self.prev[r],
        }
    }

    /// Step 3: advance every smoother; `prev ← cur`; `cur ← value` for parameters without routes. Quiet (see
    /// [`VoiceTable::advance`]), only the routed parameters' `prev ← cur`.
    pub fn advance(&mut self) {
        if self.activity.quiet() {
            for &p in &self.routed {
                self.prev[p] = self.cur[p];
            }
            return;
        }
        let changing = self.activity.begin();
        let mut moving = 0;
        for p in 0..self.base.len() {
            let value = self.smooth[p].advance(self.specs[p], self.base[p]);
            moving += u32::from(!self.smooth[p].is_settled());
            self.prev[p] = self.cur[p];
            if !self.modulated[p] {
                self.cur[p] = value;
            }
        }
        self.activity.end(changing, moving);
    }

    /// Mark parameter `p` as routed (compile time).
    pub fn route(&mut self, p: usize) {
        if !self.modulated[p] {
            self.modulated[p] = true;
            self.routed.push(p);
        }
    }

    /// Set a base value from a value event (see [`VoiceTable::set`]).
    pub fn set(&mut self, index: usize, value: f32) {
        if let (Some(spec), Some(base)) = (self.specs.get(index), self.base.get_mut(index))
            && !value.is_nan()
        {
            *base = spec.clamp(value);
            self.activity.dirty = true;
        }
    }

    /// See [`VoiceTable::carry`].
    pub fn carry(&mut self, dst: Span, old: &GlobalTable, src: Span) {
        let n = dst.len.min(src.len);
        for i in 0..n {
            let (d, s) = (dst.start + i, src.start + i);
            self.cur[d] = old.cur[s];
            self.prev[d] = old.prev[s];
            self.smooth[d] = old.smooth[s];
        }
        self.activity.dirty = true;
        self.activity.sync = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;
    const NONE: [bool; MAX_VOICES] = [false; MAX_VOICES];

    #[test]
    fn settled_values_are_the_patch_values_bit_for_bit() {
        let mut t = VoiceTable::new(SR);
        let mut values = ModuleKind::Svf.defaults();
        values[SvfParam::Cutoff.index()] = 1234.567;
        let span = t.push(ModuleKind::Svf.params(), &values);
        for _ in 0..3 {
            t.advance(&[0, 3, 7], &NONE);
        }
        let view = t.view(span);
        for v in [0, 3, 7] {
            assert_eq!(
                view.cur(SvfParam::Cutoff, v).to_bits(),
                1234.567f32.to_bits()
            );
            assert_eq!(
                view.prev(SvfParam::Cutoff, v).to_bits(),
                1234.567f32.to_bits()
            );
        }
        assert_eq!(view.base(SvfParam::Cutoff), 1234.567);
        assert_eq!(view.len(), SvfParam::COUNT);
    }

    #[test]
    fn a_value_event_ramps_linearly_in_the_normalised_domain_and_lands_exactly() {
        // svf.cutoff: Log 20 Hz–20 kHz, linear smoothing over 10 ms = 15 segments at 48 kHz.
        let spec = SvfParam::Cutoff.spec();
        let mut t = VoiceTable::new(SR);
        let span = t.push(ModuleKind::Svf.params(), &ModuleKind::Svf.defaults());
        let i = span.start + SvfParam::Cutoff.index();
        t.advance(&[2], &NONE);
        t.set(i, 500.0);
        let (n0, n1) = (spec.to_norm(2000.0), spec.to_norm(500.0));
        let mut last = 2000.0f32;
        for k in 1..=15 {
            t.advance(&[2], &NONE);
            let view = t.view(span);
            let (prev, cur) = (
                view.prev(SvfParam::Cutoff, 2),
                view.cur(SvfParam::Cutoff, 2),
            );
            assert_eq!(prev.to_bits(), last.to_bits(), "prev is last segment's cur");
            if k < 15 {
                let expected = n0 + (n1 - n0) * k as f32 / 15.0;
                assert!((spec.to_norm(cur) - expected).abs() < 1e-5, "segment {k}");
                assert!(cur < last && cur > 500.0, "monotonic, segment {k}: {cur}");
            } else {
                assert_eq!(cur.to_bits(), 500.0f32.to_bits(), "lands on the exact base");
            }
            last = cur;
        }
        t.advance(&[2], &NONE);
        assert_eq!(t.view(span).prev(SvfParam::Cutoff, 2), 500.0);
        assert!(t.smooth[i].is_settled());
    }

    #[test]
    fn a_new_target_mid_ramp_restarts_the_full_ramp_from_where_it_is() {
        let mut t = GlobalTable::new(SR);
        let span = t.push(ModuleKind::Comp.params(), &ModuleKind::Comp.defaults());
        let i = span.start + CompParam::Mix.index();
        assert_ne!(
            t.specs[i].smoothing,
            Smoothing::None,
            "comp.mix is smoothed"
        );
        t.set(i, 0.0);
        for _ in 0..5 {
            t.advance();
        }
        let mid = t.cur[i];
        t.set(i, 100.0);
        let mut segments = 0;
        while !t.smooth[i].is_settled() {
            t.advance();
            segments += 1;
            assert!(t.cur[i] >= mid);
        }
        assert_eq!(segments, 15, "constant time");
        assert_eq!(t.cur[i], 100.0);
    }

    #[test]
    fn unsmoothed_parameters_step_and_a_trigger_restarts_the_ramp() {
        let mut t = VoiceTable::new(SR);
        let span = t.push(ModuleKind::Adsr.params(), &ModuleKind::Adsr.defaults());
        let attack = span.start + AdsrParam::Attack.index();
        assert_eq!(t.specs[attack].smoothing, Smoothing::None);
        t.advance(&[1, 4], &NONE);
        t.set(attack, 50.0);
        let mut reset = NONE;
        reset[4] = true;
        t.advance(&[1, 4], &reset);
        let view = t.view(span);
        assert_eq!(view.cur(AdsrParam::Attack, 1), 50.0, "steps at once");
        assert_eq!(view.prev(AdsrParam::Attack, 1), 5.0);
        assert_eq!(
            view.prev(AdsrParam::Attack, 4),
            50.0,
            "a triggered voice gets no ramp from a stale value"
        );
    }

    #[test]
    fn one_pole_smoothing_settles_on_the_exact_base() {
        let spec: &'static ParamSpec = Box::leak(Box::new(
            (*SvfParam::Cutoff.spec()).smoothing(Smoothing::OnePole { ms: 5.0 }),
        ));
        let mut s = Smoother::new(spec, 100.0, SR);
        let mut v = s.advance(spec, 1000.0);
        assert!(v > 100.0 && v < 1000.0);
        let mut n = 1;
        while !s.is_settled() {
            let w = s.advance(spec, 1000.0);
            assert!(w >= v);
            v = w;
            n += 1;
            assert!(n < 10_000);
        }
        assert_eq!(v.to_bits(), 1000.0f32.to_bits());
    }

    #[test]
    fn value_events_are_clamped_and_nan_is_ignored() {
        let mut t = GlobalTable::new(SR);
        let span = t.push(ModuleKind::Comp.params(), &ModuleKind::Comp.defaults());
        let i = span.start + CompParam::Attack.index();
        t.set(i, 1000.0);
        t.advance();
        assert_eq!(t.view(span).cur(CompParam::Attack), 100.0);
        t.set(i, f32::NAN);
        t.set(99, 1.0); // out of range: ignored
        t.advance();
        assert_eq!(t.view(span).cur(CompParam::Attack), 100.0);
        assert_eq!(t.view(span).prev(CompParam::Attack), 100.0);
    }
}
