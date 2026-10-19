//! Parameter values inside a plan: the flat per-voice and global tables, [`ParamId`], and the read-only views
//! modules get ([`VoiceParams`], [`GlobalParams`]).
//!
//! Every parameter of every module in a plan, plus the built-in `voice.*` and `master.*`, has a slot in one of two
//! tables, chosen by the rate of its owner: modules that run per voice (and `voice.*`) in the voice table, global
//! modules (and `master.*`) in the global table. A slot holds:
//!
//! - `base`: the plain value from the patch, or the latest value event (host automation, a UI edit);
//! - `cur`: this segment's value as modules see it, per voice in the voice table;
//! - `prev`: the previous segment's `cur`, for per-sample ramps (the SVF interpolates its coefficients from `prev`
//!   to `cur`). For a voice that starts this segment, `prev == cur`.
//!
//! **Settled parameters are exact.** With no smoothing in flight and no modulation, `cur` is `base` itself, the plain
//! value from the patch, bit for bit; it never goes through `from_norm(to_norm(x))`, which is not guaranteed to round
//! trip. Smoothing and modulation (lane S2b) work in the normalised domain (`ParamSpec::to_norm`/`from_norm`, pinned
//! `libm`) and write `cur`; when a smoother settles it snaps back to the exact `base`.
//!
//! In S2a, step 3 of the segment copies `base` into `cur` (no smoothing yet) and step 6 has nothing to add (the S2a
//! compiler refuses routes). Seams for S2b are marked `TODO(S2b)`.

use enth_params::ParamSpec;
use enth_patch::catalogue::{
    AdsrParam, CompParam, LfoParam, MasterParam, NoiseParam, OscParam, ReverbParam, SvfParam,
    VoiceParam,
};

use crate::module::{MAX_VOICES, VoiceIdx};

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

/// The per-voice table. Sized at compile time; nothing here allocates afterwards.
#[derive(Debug)]
pub(crate) struct VoiceTable {
    pub specs: Vec<&'static ParamSpec>,
    pub base: Vec<f32>,
    pub cur: Vec<[f32; MAX_VOICES]>,
    pub prev: Vec<[f32; MAX_VOICES]>,
}

impl VoiceTable {
    pub fn new() -> Self {
        Self {
            specs: Vec::new(),
            base: Vec::new(),
            cur: Vec::new(),
            prev: Vec::new(),
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

    /// Step 3 in S2a: `prev ← cur` (or the new value for a fresh voice), `cur ← base`, for the active voices.
    // TODO(S2b): smoothers advance here in the normalised domain (a settled smoother delivers `base` exactly), and
    // step 6 adds the routes: `n = clamp(to_norm(smoothed) + Σ depth/100 · src, 0, 1)`, `cur = from_norm(n)`.
    pub fn advance(&mut self, voices: &[VoiceIdx], fresh: &[bool; MAX_VOICES]) {
        for ((base, cur), prev) in self.base.iter().zip(&mut self.cur).zip(&mut self.prev) {
            for &v in voices {
                prev[v] = if fresh[v] { *base } else { cur[v] };
                cur[v] = *base;
            }
        }
    }

    /// Set a base value from a value event, clamped to the spec (NaN is ignored: it is a bug upstream).
    pub fn set(&mut self, index: usize, value: f32) {
        if let (Some(spec), Some(base)) = (self.specs.get(index), self.base.get_mut(index))
            && !value.is_nan()
        {
            *base = spec.clamp(value);
        }
    }
}

/// The global table.
#[derive(Debug)]
pub(crate) struct GlobalTable {
    pub specs: Vec<&'static ParamSpec>,
    pub base: Vec<f32>,
    pub cur: Vec<f32>,
    pub prev: Vec<f32>,
}

impl GlobalTable {
    pub fn new() -> Self {
        Self {
            specs: Vec::new(),
            base: Vec::new(),
            cur: Vec::new(),
            prev: Vec::new(),
        }
    }

    pub fn push(&mut self, specs: &'static [ParamSpec], values: &[f32]) -> Span {
        let start = self.base.len();
        for (spec, &v) in specs.iter().zip(values) {
            self.specs.push(spec);
            self.base.push(v);
            self.cur.push(v);
            self.prev.push(v);
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

    /// Step 3 in S2a: `prev ← cur`, `cur ← base`.
    // TODO(S2b): smoothing and modulation, as in `VoiceTable::advance`.
    pub fn advance(&mut self) {
        for ((base, cur), prev) in self.base.iter().zip(&mut self.cur).zip(&mut self.prev) {
            *prev = *cur;
            *cur = *base;
        }
    }

    /// Set a base value from a value event (see [`VoiceTable::set`]).
    pub fn set(&mut self, index: usize, value: f32) {
        if let (Some(spec), Some(base)) = (self.specs.get(index), self.base.get_mut(index))
            && !value.is_nan()
        {
            *base = spec.clamp(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::ModuleKind;

    #[test]
    fn settled_values_are_the_patch_values_bit_for_bit() {
        let mut t = VoiceTable::new();
        let mut values = ModuleKind::Svf.defaults();
        values[SvfParam::Cutoff.index()] = 1234.567;
        let span = t.push(ModuleKind::Svf.params(), &values);
        let fresh = [false; MAX_VOICES];
        for _ in 0..3 {
            t.advance(&[0, 3, 7], &fresh);
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
    fn a_value_event_moves_cur_and_keeps_prev_for_one_segment() {
        let mut t = VoiceTable::new();
        let span = t.push(ModuleKind::Svf.params(), &ModuleKind::Svf.defaults());
        let fresh = [false; MAX_VOICES];
        t.advance(&[2], &fresh);
        t.set(span.start + SvfParam::Cutoff.index(), 500.0);
        t.advance(&[2], &fresh);
        let view = t.view(span);
        assert_eq!(view.prev(SvfParam::Cutoff, 2), 2000.0);
        assert_eq!(view.cur(SvfParam::Cutoff, 2), 500.0);
        // A fresh voice gets no ramp from a stale value.
        let mut fresh = [false; MAX_VOICES];
        fresh[5] = true;
        t.set(span.start + SvfParam::Cutoff.index(), 700.0);
        t.advance(&[2, 5], &fresh);
        let view = t.view(span);
        assert_eq!(view.prev(SvfParam::Cutoff, 5), 700.0);
        assert_eq!(view.prev(SvfParam::Cutoff, 2), 500.0);
    }

    #[test]
    fn value_events_are_clamped_and_nan_is_ignored() {
        let mut t = GlobalTable::new();
        let span = t.push(ModuleKind::Comp.params(), &ModuleKind::Comp.defaults());
        let i = span.start + CompParam::Ratio.index();
        t.set(i, 1000.0);
        t.advance();
        assert_eq!(t.view(span).cur(CompParam::Ratio), 20.0);
        t.set(i, f32::NAN);
        t.set(99, 1.0); // out of range: ignored
        t.advance();
        assert_eq!(t.view(span).cur(CompParam::Ratio), 20.0);
        assert_eq!(t.view(span).prev(CompParam::Ratio), 20.0);
    }
}
