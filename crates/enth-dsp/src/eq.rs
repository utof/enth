//! **Owner: lane EQ** (S2a wave 4, `docs/dsp/eq.md`). The DSP core of the `eq` module: a six-band parametric
//! equaliser built from trapezoidal-integrator (TPT) state-variable sections, with per-segment coefficient ramps.
//!
//! **The path.** Six sections in series, in the catalogue's table order, the same for both channels: a **low cut**
//! (12 dB/octave Butterworth high-pass), a **low shelf**, two **bells**, a **high shelf** and a **high cut** (12
//! dB/octave Butterworth low-pass). Every band is the response of Robert Bristow-Johnson's *Audio EQ Cookbook*
//! (`HPF`, `lowShelf`, `peakingEQ`, `highShelf`, `LPF`), the digital filter the bilinear transform of its analog
//! prototype gives with the corner pre-warped, which is the filter `enth-dsp`'s [`biquad`](crate::biquad)
//! designs. The shelves use the cookbook's slope `S = 1` (`Q = 1/√2`, its steepest monotonic shelf), because the
//! catalogue gives them a corner and a gain and no shape; the cuts are `Q = 1/√2` (Butterworth). **The cuts are
//! always in the path**: nothing switches them in or out at the ends of their ranges, because both are modulatable
//! and a stage that appeared at a range end would click as a source crossed it. A band at 0 dB is the identity
//! (`out = in`, bit for bit) but keeps tracking the signal, so it can be raised without a swell.
//!
//! **Why a state-variable section and not [`biquad::Biquad`](crate::biquad::Biquad).** The two realise the same
//! transfer function when the controls are still (a test compares them), but they behave differently while the
//! controls move. The biquad's transposed direct form II holds sums of past inputs and outputs, so changing its
//! coefficients re-reads that state with different numbers: a high-Q bell hopping between 20 Hz and 20 kHz every
//! segment pumps energy into it and blows up to infinity in a few seconds (measured: lane EQ's first version,
//! `docs/dsp/eq.md`, "Why not the biquad"). The TPT section keeps the two integrator voltages of an analog
//! state-variable filter. Its state update is `[[2a1−1, −2a2], [2a2, 1−2a3]]` with `a1 = 1/(1 + g(g + k))`, `a2 =
//! g·a1`, `a3 = g·a2`, whose largest singular value is exactly 1 for every `g > 0` and `k ≥ 0`: a rotation by `2
//! atan g`, damped. So **no sequence of corner, gain or Q changes can feed it energy**, whatever the modulation
//! rate (Andrew Simper, *Linear Trapezoidal Integrated State Variable Filter*, Cytomic, 2013, typo fix 2016; the
//! equations only, no code). The section is `v3 = in − ic2; v1 = a1·ic1 + a2·v3; v2 = ic2 + a2·ic1 + a3·v3; ic1 =
//! 2·v1 − ic1; ic2 = 2·v2 − ic2; out = m0·in + m1·v1 + m2·v2`, and each band is a choice of `(g, k, m0, m1, m2)`
//! ([`Params`]).
//!
//! **Segments and ramps.** The engine calls [`Eq::process`] once per [`SEGMENT`] of 32 frames with two sets of
//! [`Controls`]: the previous segment's and this one's. Each band's parameters at the start of the segment are the
//! design of the previous controls and at its end the design of the new ones, and `g`, `k`, `m0`, `m1`, `m2` move
//! linearly per sample between them (a precomputed increment, five adds, one division for the coefficients),
//! reaching the new design on the last frame. Every frame between is a real section (`g > 0`, `k ≥ 0`), so the
//! guarantee above holds along the ramp. The ramp starts from the design of the *previous controls*, not from
//! anything the module remembers, so the output depends only on the state, the input and the two control sets:
//! skipping a silent segment (the engine's idle rule) is the same as running it. A band whose controls did not
//! change designs nothing (the last design is cached by its inputs), so a static EQ costs no transcendental; a
//! moving band costs one design per segment.
//!
//! **Mix.** `out = wet·m + dry·(1 − m)`, with `m` ramped per sample from the previous segment's mix to this one's.
//! At `m = 1` the output is the wet signal, and at `m = 0` the *dry samples are put back*, so the input passes bit
//! for bit (NaN, subnormals and `-0.0` included). The sections keep running while the mix is 0, so their state
//! follows the signal and the mix can come up without a transient.
//!
//! **Precision and determinism.** Design and state are `f64`, input and output `f32` (a low band has `g` near
//! 1e-3). Between the sections the signal is rounded to `f32`, a relative 6e-8 per section. The only
//! transcendentals are `pow`, `tan` and `sqrt`-class operations from the pinned `libm` through
//! [`util::math`](crate::util::math), once per design; everything else is `+ − × ÷`, written in one fixed order with
//! no fused operation. The integrator states are flushed once per segment and the output of every section per
//! sample ([`flush64`]); a section whose state turns non-finite (a NaN in the input) is cleared, so a bad block does
//! not poison the equaliser for good.
//!
//! The equations and the reference are in `docs/dsp/eq.md`. The cookbook is a set of formulas and the paper is
//! public domain: nothing is copied.

use core::f64::consts::{FRAC_1_SQRT_2, PI};

use crate::biquad::{MAX_FREQ_FRACTION, MAX_GAIN_DB, MAX_Q, MIN_FREQ_HZ, MIN_Q};
use crate::util::flush::flush64;
use crate::util::math;

/// Frames per [`Eq::process`] call: the engine's segment. The ramps run over exactly this many frames.
pub const SEGMENT: usize = 32;

/// The number of bands in series.
pub const BANDS: usize = 6;

/// The quality factor of both shelves: the cookbook's slope `S = 1` in its `Q` form (`1/√2`).
pub const SHELF_Q: f64 = FRAC_1_SQRT_2;

/// The quality factor of both cuts: Butterworth, 12 dB/octave (`k = √2`).
pub const CUT_Q: f64 = FRAC_1_SQRT_2;

/// `RAMP[i] = (i + 1) / SEGMENT`, exact in `f32`: the weight of the mix ramp, the engine's own table.
const RAMP: [f32; SEGMENT] = {
    let mut r = [0.0; SEGMENT];
    let mut i = 0;
    while i < SEGMENT {
        r[i] = (i + 1) as f32 / SEGMENT as f32;
        i += 1;
    }
    r
};

/// One audio segment of one channel.
pub type Segment = [f32; SEGMENT];

/// The six bands, in signal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// 12 dB/octave high-pass.
    LowCut,
    /// Low shelf.
    LowShelf,
    /// First bell.
    Bell1,
    /// Second bell.
    Bell2,
    /// High shelf.
    HighShelf,
    /// 12 dB/octave low-pass.
    HighCut,
}

impl Band {
    /// All six, in signal order.
    pub const ALL: [Band; BANDS] = [
        Band::LowCut,
        Band::LowShelf,
        Band::Bell1,
        Band::Bell2,
        Band::HighShelf,
        Band::HighCut,
    ];
}

/// The equaliser's settings in the catalogue's plain units (Hz, dB, Q, percent), one field per catalogue parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// Low cut corner, Hz (10 Hz … 1 kHz).
    pub low_cut_hz: f32,
    /// Low shelf corner, Hz (20 Hz … 1 kHz).
    pub low_freq_hz: f32,
    /// Low shelf gain, dB (±24).
    pub low_gain_db: f32,
    /// First bell centre, Hz (20 Hz … 20 kHz).
    pub bell1_freq_hz: f32,
    /// First bell gain, dB (±24).
    pub bell1_gain_db: f32,
    /// First bell Q (0.1 … 20).
    pub bell1_q: f32,
    /// Second bell centre, Hz.
    pub bell2_freq_hz: f32,
    /// Second bell gain, dB.
    pub bell2_gain_db: f32,
    /// Second bell Q.
    pub bell2_q: f32,
    /// High shelf corner, Hz (1 kHz … 20 kHz).
    pub high_freq_hz: f32,
    /// High shelf gain, dB.
    pub high_gain_db: f32,
    /// High cut corner, Hz (1 kHz … 20 kHz).
    pub high_cut_hz: f32,
    /// Dry/wet, percent: 0 passes the input bit for bit.
    pub mix_pct: f32,
}

/// What a band's design depends on: frequency, gain and Q (unused entries 0). Equal keys give equal designs.
type Key = [f32; 3];

impl Controls {
    /// The catalogue's defaults: every gain at 0 dB, the cuts at the ends of their ranges, mix 100 %.
    pub const DEFAULT: Self = Self {
        low_cut_hz: 10.0,
        low_freq_hz: 100.0,
        low_gain_db: 0.0,
        bell1_freq_hz: 400.0,
        bell1_gain_db: 0.0,
        bell1_q: 1.0,
        bell2_freq_hz: 2_500.0,
        bell2_gain_db: 0.0,
        bell2_q: 1.0,
        high_freq_hz: 8_000.0,
        high_gain_db: 0.0,
        high_cut_hz: 20_000.0,
        mix_pct: 100.0,
    };

    /// These controls with a non-finite value replaced by its default and the mix kept in `0..=100`. The designs
    /// clamp frequencies, gains and Q themselves, so a finite value is never changed here but the mix.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let d = Self::DEFAULT;
        let f = |x: f32, default: f32| if x.is_finite() { x } else { default };
        Self {
            low_cut_hz: f(self.low_cut_hz, d.low_cut_hz),
            low_freq_hz: f(self.low_freq_hz, d.low_freq_hz),
            low_gain_db: f(self.low_gain_db, d.low_gain_db),
            bell1_freq_hz: f(self.bell1_freq_hz, d.bell1_freq_hz),
            bell1_gain_db: f(self.bell1_gain_db, d.bell1_gain_db),
            bell1_q: f(self.bell1_q, d.bell1_q),
            bell2_freq_hz: f(self.bell2_freq_hz, d.bell2_freq_hz),
            bell2_gain_db: f(self.bell2_gain_db, d.bell2_gain_db),
            bell2_q: f(self.bell2_q, d.bell2_q),
            high_freq_hz: f(self.high_freq_hz, d.high_freq_hz),
            high_gain_db: f(self.high_gain_db, d.high_gain_db),
            high_cut_hz: f(self.high_cut_hz, d.high_cut_hz),
            mix_pct: f(self.mix_pct, d.mix_pct).clamp(0.0, 100.0),
        }
    }

    /// The inputs of `band`'s design.
    #[must_use]
    fn key(&self, band: Band) -> Key {
        match band {
            Band::LowCut => [self.low_cut_hz, 0.0, 0.0],
            Band::LowShelf => [self.low_freq_hz, self.low_gain_db, 0.0],
            Band::Bell1 => [self.bell1_freq_hz, self.bell1_gain_db, self.bell1_q],
            Band::Bell2 => [self.bell2_freq_hz, self.bell2_gain_db, self.bell2_q],
            Band::HighShelf => [self.high_freq_hz, self.high_gain_db, 0.0],
            Band::HighCut => [self.high_cut_hz, 0.0, 0.0],
        }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What one section needs: the pre-warped corner `g = tan(π·f/fs)`, the damping `k`, and the output mix
/// `out = m0·in + m1·band + m2·low`. Interpolated linearly during a ramp; every point on the line between two
/// designs has `g > 0` and `k ≥ 0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// `tan(π·f/fs)`.
    pub g: f64,
    /// Damping, `1/Q` for a cut or shelf, `1/(Q·A)` for a bell.
    pub k: f64,
    /// `[m0, m1, m2]`.
    pub m: [f64; 3],
}

impl Params {
    /// The section that passes its input unchanged and holds its state (`g = 0` is a rotation by nothing). A design
    /// returns it only for an argument it cannot use (a non-finite value); a band at 0 dB keeps its real corner, so
    /// its state follows the signal while the output is the input.
    pub const IDENTITY: Self = Self {
        g: 0.0,
        k: 0.0,
        m: [1.0, 0.0, 0.0],
    };

    /// Whether the output is exactly the input.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.m == [1.0, 0.0, 0.0]
    }

    /// `(a1, a2, a3)` of the section.
    #[inline(always)]
    fn coefficients(&self) -> (f64, f64, f64) {
        let a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        let a2 = self.g * a1;
        (a1, a2, self.g * a2)
    }

    /// The magnitude response in dB at `freq_hz`: the analog prototype `(m0·D + m1·s + m2)/D`,
    /// `D = s² + k·s + 1`, at the pre-warped `s = j·tan(π·f/fs)/g`. Valid below Nyquist.
    #[must_use]
    pub fn magnitude_db(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        if self.is_identity() {
            return 0.0;
        }
        let x = math::tan(PI * (freq_hz / sample_rate)) / self.g;
        // D = 1 − x² + j·k·x;  N = m0·D + j·m1·x + m2.
        let (dr, di) = (1.0 - x * x, self.k * x);
        let nr = self.m[0] * dr + self.m[2];
        let ni = self.m[0] * di + self.m[1] * x;
        10.0 * math::log10((nr * nr + ni * ni) / (dr * dr + di * di))
    }
}

/// The inputs a design needs, cleaned: a frequency in `1 Hz … 0.49·fs`, else `None` (a sample rate too low to hold
/// 1 Hz, or a non-finite value: the identity).
fn corner(freq_hz: f64, sample_rate: f64) -> Option<f64> {
    let highest = MAX_FREQ_FRACTION * sample_rate;
    if !(freq_hz.is_finite() && sample_rate.is_finite()) || highest <= MIN_FREQ_HZ {
        return None;
    }
    Some(math::tan(
        PI * (freq_hz.clamp(MIN_FREQ_HZ, highest) / sample_rate),
    ))
}

/// The cookbook's `HPF` at `Q = 1/√2`: a 12 dB/octave Butterworth high-pass with its corner (−3 dB) at `freq_hz`,
/// clamped to `1 Hz … 0.49·sample_rate`. A non-finite argument designs the identity.
#[must_use]
pub fn highpass(freq_hz: f64, sample_rate: f64) -> Params {
    corner(freq_hz, sample_rate).map_or(Params::IDENTITY, |g| {
        let k = CUT_Q.recip();
        Params {
            g,
            k,
            m: [1.0, -k, -1.0],
        }
    })
}

/// The cookbook's `LPF` at `Q = 1/√2`: a 12 dB/octave Butterworth low-pass with its corner (−3 dB) at `freq_hz`,
/// clamped as [`highpass`].
#[must_use]
pub fn lowpass(freq_hz: f64, sample_rate: f64) -> Params {
    corner(freq_hz, sample_rate).map_or(Params::IDENTITY, |g| Params {
        g,
        k: CUT_Q.recip(),
        m: [0.0, 0.0, 1.0],
    })
}

/// `A = 10^(dB/40)` for a gain clamped to ±[`MAX_GAIN_DB`]; `None` for a non-finite gain.
fn amplitude(gain_db: f64) -> Option<f64> {
    gain_db
        .is_finite()
        .then(|| math::pow(10.0, gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB) / 40.0))
}

/// The cookbook's `peakingEQ`: a bell of `gain_db` at its centre and unity far away. `q` is clamped to the
/// biquad's `0.01 … 1000`. A gain of exactly 0 is the identity in output (`m = [1, 0, 0]`); a non-finite argument
/// designs [`Params::IDENTITY`].
#[must_use]
pub fn bell(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Params {
    let (Some(g), Some(a)) = (corner(freq_hz, sample_rate), amplitude(gain_db)) else {
        return Params::IDENTITY;
    };
    if !q.is_finite() {
        return Params::IDENTITY;
    }
    let k = 1.0 / (q.clamp(MIN_Q, MAX_Q) * a);
    Params {
        g,
        k,
        m: [1.0, k * (a * a - 1.0), 0.0],
    }
}

/// The cookbook's `lowShelf` in its `Q` form: `gain_db` below the corner, unity above, half the gain in dB at the
/// corner. A gain of exactly 0 is the identity in output; a non-finite argument designs [`Params::IDENTITY`].
#[must_use]
pub fn low_shelf(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Params {
    let (Some(g), Some(a)) = (corner(freq_hz, sample_rate), amplitude(gain_db)) else {
        return Params::IDENTITY;
    };
    if !q.is_finite() {
        return Params::IDENTITY;
    }
    let k = 1.0 / q.clamp(MIN_Q, MAX_Q);
    Params {
        g: g / a.sqrt(),
        k,
        m: [1.0, k * (a - 1.0), a * a - 1.0],
    }
}

/// The cookbook's `highShelf` in its `Q` form: unity below the corner, `gain_db` above, half the gain in dB at the
/// corner. A gain of exactly 0 is the identity in output; a non-finite argument designs [`Params::IDENTITY`].
#[must_use]
pub fn high_shelf(freq_hz: f64, gain_db: f64, q: f64, sample_rate: f64) -> Params {
    let (Some(g), Some(a)) = (corner(freq_hz, sample_rate), amplitude(gain_db)) else {
        return Params::IDENTITY;
    };
    if !q.is_finite() {
        return Params::IDENTITY;
    }
    let k = 1.0 / q.clamp(MIN_Q, MAX_Q);
    Params {
        g: g * a.sqrt(),
        k,
        m: [a * a, k * (1.0 - a) * a, 1.0 - a * a],
    }
}

/// The section `band` has for the design inputs `key` at `sample_rate`.
fn design(band: Band, key: Key, sample_rate: f64) -> Params {
    let (f, g, q) = (f64::from(key[0]), f64::from(key[1]), f64::from(key[2]));
    match band {
        Band::LowCut => highpass(f, sample_rate),
        Band::LowShelf => low_shelf(f, g, SHELF_Q, sample_rate),
        Band::Bell1 | Band::Bell2 => bell(f, g, q, sample_rate),
        Band::HighShelf => high_shelf(f, g, SHELF_Q, sample_rate),
        Band::HighCut => lowpass(f, sample_rate),
    }
}

/// The six sections for `controls` at `sample_rate`, in signal order: what the equaliser's bands are when settled.
/// For tests and later for displays.
#[must_use]
pub fn settled(controls: &Controls, sample_rate: f64) -> [Params; BANDS] {
    let c = controls.sanitized();
    Band::ALL.map(|band| design(band, c.key(band), sample_rate))
}

/// The magnitude response of the six settled sections in series, in dB, at `freq_hz` (below Nyquist).
#[must_use]
pub fn magnitude_db(controls: &Controls, freq_hz: f64, sample_rate: f64) -> f64 {
    settled(controls, sample_rate)
        .iter()
        .map(|p| p.magnitude_db(freq_hz, sample_rate))
        .sum()
}

/// The two integrator voltages of both channels of one band: lane 0 is the left channel and lane 1 the right. The
/// two channels run in lock step through the same arithmetic, written lane by lane so the compiler can put the pair
/// in one vector register; every operation is a plain IEEE `+ − ×`, so the bits are the same as the scalar form on
/// every target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct State {
    ic1: [f64; 2],
    ic2: [f64; 2],
}

impl State {
    /// One frame (left, right) through the section with coefficients `(a1, a2, a3)` and output mix `m`.
    #[inline(always)]
    fn tick(&mut self, x: [f32; 2], a: (f64, f64, f64), m: &[f64; 3]) -> [f32; 2] {
        let v0 = [f64::from(x[0]), f64::from(x[1])];
        let v3 = [v0[0] - self.ic2[0], v0[1] - self.ic2[1]];
        let v1 = [
            a.0 * self.ic1[0] + a.1 * v3[0],
            a.0 * self.ic1[1] + a.1 * v3[1],
        ];
        let v2 = [
            self.ic2[0] + a.1 * self.ic1[0] + a.2 * v3[0],
            self.ic2[1] + a.1 * self.ic1[1] + a.2 * v3[1],
        ];
        self.ic1 = [2.0 * v1[0] - self.ic1[0], 2.0 * v1[1] - self.ic1[1]];
        self.ic2 = [2.0 * v2[0] - self.ic2[0], 2.0 * v2[1] - self.ic2[1]];
        let y0 = (m[0] * v0[0] + m[1] * v1[0]) + m[2] * v2[0];
        let y1 = (m[0] * v0[1] + m[1] * v1[1]) + m[2] * v2[1];
        [flush64(y0) as f32, flush64(y1) as f32]
    }

    fn at_rest(&self) -> bool {
        self.ic1 == [0.0; 2] && self.ic2 == [0.0; 2]
    }

    /// Flush the voltages; clear a channel's if they are not finite.
    fn settle(&mut self) {
        for c in 0..2 {
            if self.ic1[c].is_finite() && self.ic2[c].is_finite() {
                self.ic1[c] = flush64(self.ic1[c]);
                self.ic2[c] = flush64(self.ic2[c]);
            } else {
                self.ic1[c] = 0.0;
                self.ic2[c] = 0.0;
            }
        }
    }

    /// The number of subnormal voltages (test support).
    fn subnormals(&self) -> usize {
        let sub = |x: f64| usize::from(x != 0.0 && x.abs() < f64::MIN_POSITIVE);
        self.ic1.iter().chain(&self.ic2).map(|&x| sub(x)).sum()
    }
}

/// One band: the two channels' states and the last design (cached by its inputs).
#[derive(Clone, Debug)]
struct Section {
    state: State,
    /// The inputs of the cached design. NaN until the first design, so the first lookup always misses.
    key: Key,
    params: Params,
}

impl Section {
    const fn new() -> Self {
        Self {
            state: State {
                ic1: [0.0; 2],
                ic2: [0.0; 2],
            },
            key: [f32::NAN; 3],
            params: Params::IDENTITY,
        }
    }

    /// Run the band over one segment, ramping from the design of `from` to the design of `to`.
    fn run(
        &mut self,
        band: Band,
        from: Key,
        to: Key,
        sample_rate: f64,
        left: &mut Segment,
        right: &mut Segment,
    ) {
        let p0 = if self.key == from {
            self.params
        } else {
            design(band, from, sample_rate)
        };
        let p1 = if to == from {
            p0
        } else {
            design(band, to, sample_rate)
        };
        self.key = to;
        self.params = p1;

        if p0 == p1 {
            let a = p1.coefficients();
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                [*l, *r] = self.state.tick([*l, *r], a, &p1.m);
            }
        } else {
            // 1 / SEGMENT is a power of two, so the scaling is exact.
            let k = 1.0 / SEGMENT as f64;
            let step = Params {
                g: (p1.g - p0.g) * k,
                k: (p1.k - p0.k) * k,
                m: [
                    (p1.m[0] - p0.m[0]) * k,
                    (p1.m[1] - p0.m[1]) * k,
                    (p1.m[2] - p0.m[2]) * k,
                ],
            };
            let mut p = p0;
            let last = SEGMENT - 1;
            for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                if i == last {
                    // The last frame is exactly the new design, whatever the increments added up to.
                    p = p1;
                } else {
                    p.g += step.g;
                    p.k += step.k;
                    p.m[0] += step.m[0];
                    p.m[1] += step.m[1];
                    p.m[2] += step.m[2];
                }
                let a = p.coefficients();
                [*l, *r] = self.state.tick([*l, *r], a, &p.m);
            }
        }
        self.state.settle();
    }
}

/// The equaliser: six sections per channel, ramped per segment. `Clone` copies all of it (about 0.5 KB), which is
/// the plan swap's whole cost.
#[derive(Clone, Debug)]
pub struct Eq {
    sample_rate: f64,
    sections: [Section; BANDS],
}

impl Eq {
    /// An equaliser for `sample_rate` with every state cleared.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: f64::from(sample_rate),
            sections: [const { Section::new() }; BANDS],
        }
    }

    /// Clear every state (a restart). The design cache stays: it is a function of its inputs.
    pub fn reset(&mut self) {
        for s in &mut self.sections {
            s.state = State::default();
        }
    }

    /// Process one segment in place. `from` is the previous segment's controls and `to` this segment's: the bands
    /// ramp from the first to the second (equal sets run static), the mix likewise.
    pub fn process(
        &mut self,
        from: &Controls,
        to: &Controls,
        left: &mut Segment,
        right: &mut Segment,
    ) {
        let (from, to) = (from.sanitized(), to.sanitized());
        let mix = [from.mix_pct / 100.0, to.mix_pct / 100.0];
        let all_wet = mix == [1.0, 1.0];
        let (dry_left, dry_right) = (*left, *right);

        for (section, band) in self.sections.iter_mut().zip(Band::ALL) {
            section.run(
                band,
                from.key(band),
                to.key(band),
                self.sample_rate,
                left,
                right,
            );
        }

        if all_wet {
            return;
        }
        if mix == [0.0, 0.0] {
            *left = dry_left;
            *right = dry_right;
            return;
        }
        let d = mix[1] - mix[0];
        for (i, w) in RAMP.iter().enumerate() {
            let m = mix[0] + d * w;
            let dry = 1.0 - m;
            left[i] = left[i] * m + dry_left[i] * dry;
            right[i] = right[i] * m + dry_right[i] * dry;
        }
    }

    /// `true` when every state is exactly zero: a segment of silence would then give silence and change nothing.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.sections.iter().all(|s| s.state.at_rest())
    }

    /// The number of subnormal values in the state (test support: the flushes keep it at 0).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        self.sections.iter().map(|s| s.state.subnormals()).sum()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::manual_midpoint,
        reason = "the cookbook's formulas are written as the cookbook writes them, `(1 + cos w0) / 2` and all"
    )]

    use super::*;
    use crate::biquad::{Biquad, Coeffs, Kind};
    use crate::util::rng::Rng;
    use core::f64::consts::TAU;

    const SR: f64 = 48_000.0;

    /// A setter for one control, and one for the frequency, gain and Q of a band.
    type SetOne = fn(&mut Controls, f32);
    type SetThree = fn(&mut Controls, f32, f32, f32);

    // -----------------------------------------------------------------------------------------------------------
    // The reference: the Audio EQ Cookbook's formulas, written here and nowhere else
    // -----------------------------------------------------------------------------------------------------------

    /// The cookbook's biquad as `(b0, b1, b2, a0, a1, a2)`, un-normalised.
    type Rbj = [f64; 6];

    fn rbj_lowpass(f0: f64, sr: f64) -> Rbj {
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (c, alpha) = (libm::cos(w0), libm::sin(w0) / (2.0 * FRAC_1_SQRT_2));
        [
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        ]
    }

    fn rbj_highpass(f0: f64, sr: f64) -> Rbj {
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (c, alpha) = (libm::cos(w0), libm::sin(w0) / (2.0 * FRAC_1_SQRT_2));
        [
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        ]
    }

    fn rbj_peaking(f0: f64, db: f64, q: f64, sr: f64) -> Rbj {
        let a = libm::pow(10.0, db / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (c, alpha) = (libm::cos(w0), libm::sin(w0) / (2.0 * q));
        [
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        ]
    }

    /// The shelves in the cookbook's slope form with `S = 1`, which is not the `Q` form the code uses:
    /// `alpha = sin(w0)/2 · sqrt((A + 1/A)(1/S − 1) + 2)`.
    fn shelf_alpha(a: f64, w0: f64, slope: f64) -> f64 {
        libm::sin(w0) / 2.0 * libm::sqrt((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0)
    }

    fn rbj_lowshelf(f0: f64, db: f64, sr: f64) -> Rbj {
        let a = libm::pow(10.0, db / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (c, sa) = (libm::cos(w0), 2.0 * libm::sqrt(a) * shelf_alpha(a, w0, 1.0));
        [
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        ]
    }

    fn rbj_highshelf(f0: f64, db: f64, sr: f64) -> Rbj {
        let a = libm::pow(10.0, db / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / sr;
        let (c, sa) = (libm::cos(w0), 2.0 * libm::sqrt(a) * shelf_alpha(a, w0, 1.0));
        [
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        ]
    }

    /// `20·log10 |H(e^{jw})|` of a cookbook biquad at `f`, evaluated from the un-normalised coefficients.
    fn rbj_db(b: &Rbj, f: f64, sr: f64) -> f64 {
        let w = 2.0 * core::f64::consts::PI * f / sr;
        let (c1, s1, c2, s2) = (
            libm::cos(w),
            -libm::sin(w),
            libm::cos(2.0 * w),
            -libm::sin(2.0 * w),
        );
        let num = [b[0] + b[1] * c1 + b[2] * c2, b[1] * s1 + b[2] * s2];
        let den = [b[3] + b[4] * c1 + b[5] * c2, b[4] * s1 + b[5] * s2];
        10.0 * libm::log10(
            (num[0] * num[0] + num[1] * num[1]) / (den[0] * den[0] + den[1] * den[1]),
        )
    }

    /// The cookbook's six bands for `c` in series: the response of the whole equaliser.
    fn cookbook_db(c: &Controls, f: f64, sr: f64) -> f64 {
        let bands = [
            rbj_highpass(f64::from(c.low_cut_hz), sr),
            rbj_lowshelf(f64::from(c.low_freq_hz), f64::from(c.low_gain_db), sr),
            rbj_peaking(
                f64::from(c.bell1_freq_hz),
                f64::from(c.bell1_gain_db),
                f64::from(c.bell1_q),
                sr,
            ),
            rbj_peaking(
                f64::from(c.bell2_freq_hz),
                f64::from(c.bell2_gain_db),
                f64::from(c.bell2_q),
                sr,
            ),
            rbj_highshelf(f64::from(c.high_freq_hz), f64::from(c.high_gain_db), sr),
            rbj_lowpass(f64::from(c.high_cut_hz), sr),
        ];
        bands.iter().map(|b| rbj_db(b, f, sr)).sum()
    }

    /// `n` frequencies, log-spaced from `lo` to `hi`.
    fn log_grid(lo: f64, hi: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| lo * libm::pow(hi / lo, i as f64 / (n - 1) as f64))
            .collect()
    }

    // -----------------------------------------------------------------------------------------------------------
    // Running the core
    // -----------------------------------------------------------------------------------------------------------

    /// Run `input` (any length, zero-padded to whole segments) through the core with fixed controls.
    fn run_static(eq: &mut Eq, c: &Controls, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len().next_multiple_of(SEGMENT));
        for chunk in input.chunks(SEGMENT) {
            let (mut l, mut r) = ([0.0f32; SEGMENT], [0.0f32; SEGMENT]);
            l[..chunk.len()].copy_from_slice(chunk);
            r[..chunk.len()].copy_from_slice(chunk);
            eq.process(c, c, &mut l, &mut r);
            assert!(
                l.iter().zip(&r).all(|(a, b)| a.to_bits() == b.to_bits()),
                "both channels get the same sound from the same input"
            );
            out.extend_from_slice(&l);
        }
        out
    }

    /// The impulse response of the core under `c`, run until the last segments are below `1e-10` or `cap` samples.
    /// Returns it and whether it settled.
    fn impulse_response(c: &Controls, sr: f32, cap: usize) -> (Vec<f32>, bool) {
        let mut eq = Eq::new(sr);
        let mut x = vec![0.0f32; 4096];
        x[0] = 1.0;
        let mut h = run_static(&mut eq, c, &x);
        let zeros = vec![0.0f32; 4096];
        while h.len() < cap {
            let chunk = run_static(&mut eq, c, &zeros);
            let quiet = chunk.iter().all(|v| v.abs() < 1e-10);
            h.extend_from_slice(&chunk);
            if quiet {
                return (h, true);
            }
        }
        (h, false)
    }

    /// `|DTFT(h)|` in dB at `f` (a rotating phasor, re-synchronised from `sin`/`cos` every 1,024 samples).
    fn dtft_db(h: &[f32], f: f64, sr: f64) -> f64 {
        let w = TAU * (f / sr);
        let (rc, rs) = (libm::cos(w), -libm::sin(w));
        let (mut re, mut im) = (0.0f64, 0.0f64);
        let (mut pc, mut ps) = (1.0f64, 0.0f64);
        for (n, &v) in h.iter().enumerate() {
            if n % 1024 == 0 {
                let ph = w * n as f64;
                pc = libm::cos(ph);
                ps = -libm::sin(ph);
            }
            let v = f64::from(v);
            re += v * pc;
            im += v * ps;
            (pc, ps) = (pc * rc - ps * rs, pc * rs + ps * rc);
        }
        10.0 * libm::log10(re * re + im * im)
    }

    fn noise(n: usize, seed: u64, amp: f32) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| rng.bipolar_f32() * amp).collect()
    }

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// Controls with one band set and the rest at their defaults.
    fn with(f: impl FnOnce(&mut Controls)) -> Controls {
        let mut c = Controls::DEFAULT;
        f(&mut c);
        c
    }

    // -----------------------------------------------------------------------------------------------------------
    // Acceptance: within 0.1 dB of the cookbook
    // -----------------------------------------------------------------------------------------------------------

    #[test]
    fn the_designed_sections_equal_the_cookbooks_formulas() {
        // Coefficient level, every band type over the table's ranges at three sample rates: the response of the
        // designed section against the cookbook's formulas, which this test writes out on its own.
        let mut worst = 0.0f64;
        let mut cases = 0;
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            for f0 in [
                20.0f32, 63.0, 100.0, 250.0, 1_000.0, 4_000.0, 12_000.0, 20_000.0,
            ] {
                for gain in [-24.0f32, -12.0, -3.0, -0.1, 0.1, 3.0, 12.0, 24.0] {
                    for q in [0.1f32, 0.5, 0.7, 1.0, 4.0, 10.0, 20.0] {
                        let probes = log_grid(10.0, 0.499 * sr, 40);
                        let sections = [
                            (
                                with(|c| c.bell1_freq_hz = f0).tap(|c| {
                                    c.bell1_gain_db = gain;
                                    c.bell1_q = q;
                                }),
                                "bell",
                            ),
                            (
                                with(|c| c.low_freq_hz = f0.min(1_000.0))
                                    .tap(|c| c.low_gain_db = gain),
                                "low shelf",
                            ),
                            (
                                with(|c| c.high_freq_hz = f0.max(1_000.0))
                                    .tap(|c| c.high_gain_db = gain),
                                "high shelf",
                            ),
                        ];
                        for (c, what) in sections {
                            for &f in &probes {
                                let got = magnitude_db(&c, f, sr);
                                let want = cookbook_db(&c, f, sr);
                                worst = worst.max((got - want).abs());
                                cases += 1;
                                assert!(
                                    (got - want).abs() < 1e-6,
                                    "{what} {f0} Hz {gain} dB Q {q} sr {sr} at {f}: {got} vs {want}"
                                );
                            }
                        }
                    }
                }
            }
            // The cuts over their whole ranges.
            for fc in log_grid(10.0, 1_000.0, 25) {
                let c = with(|c| c.low_cut_hz = fc as f32);
                for f in log_grid(5.0, 0.499 * sr, 40) {
                    let (got, want) = (magnitude_db(&c, f, sr), cookbook_db(&c, f, sr));
                    worst = worst.max((got - want).abs());
                    assert!((got - want).abs() < 1e-6, "low cut {fc} at {f}");
                    cases += 1;
                }
            }
            for fc in log_grid(1_000.0, 20_000.0, 25) {
                let c = with(|c| c.high_cut_hz = fc as f32);
                for f in log_grid(5.0, 0.499 * sr, 40) {
                    let (got, want) = (magnitude_db(&c, f, sr), cookbook_db(&c, f, sr));
                    worst = worst.max((got - want).abs());
                    assert!((got - want).abs() < 1e-6, "high cut {fc} at {f}");
                    cases += 1;
                }
            }
        }
        assert!(cases > 40_000, "{cases}");
        println!("max |designed - cookbook| over {cases} points: {worst:.3e} dB");
    }

    trait Tap: Sized {
        fn tap(self, f: impl FnOnce(&mut Self)) -> Self;
    }
    impl Tap for Controls {
        fn tap(mut self, f: impl FnOnce(&mut Self)) -> Self {
            f(&mut self);
            self
        }
    }

    /// The processed impulse response of the whole core against the cookbook cascade, for `c`.
    fn processed_worst(c: &Controls, sr: f32, probes: &[f64]) -> Option<f64> {
        let (h, settled) = impulse_response(c, sr, 1 << 19);
        if !settled {
            return None;
        }
        let mut worst = 0.0f64;
        for &f in probes {
            let got = dtft_db(&h, f, f64::from(sr));
            let want = cookbook_db(c, f, f64::from(sr));
            worst = worst.max((got - want).abs());
        }
        Some(worst)
    }

    #[test]
    fn the_processed_response_is_within_a_tenth_of_a_decibel_of_the_cookbook() {
        // The impulse response through `Eq::process` (the ramps, the f32 hand-offs and the flushes included), read at
        // 40 frequencies, for each band type alone over gains and Qs across the table's ranges.
        let sr = 48_000.0f32;
        let probes = log_grid(20.0, 0.45 * f64::from(sr), 39);
        let mut worst = 0.0f64;
        let (mut tested, mut skipped) = (0, 0);
        let mut check = |c: Controls, what: String| {
            let mut p = probes.clone();
            p.push(f64::from(c.bell1_freq_hz));
            match processed_worst(&c, sr, &p) {
                None => skipped += 1,
                Some(w) => {
                    tested += 1;
                    worst = worst.max(w);
                    assert!(w < 0.1, "{what}: {w} dB from the cookbook");
                }
            }
        };
        for f0 in [20.0f32, 100.0, 1_000.0, 8_000.0, 16_000.0, 20_000.0] {
            for gain in [-24.0f32, -6.0, 6.0, 24.0] {
                for q in [0.1f32, 0.7, 4.0, 20.0] {
                    check(
                        with(|c| c.bell1_freq_hz = f0).tap(|c| {
                            c.bell1_gain_db = gain;
                            c.bell1_q = q;
                        }),
                        format!("bell {f0} Hz {gain} dB Q {q}"),
                    );
                }
                check(
                    with(|c| c.low_freq_hz = f0.min(1_000.0)).tap(|c| c.low_gain_db = gain),
                    format!("low shelf {f0} Hz {gain} dB"),
                );
                check(
                    with(|c| c.high_freq_hz = f0.max(1_000.0)).tap(|c| c.high_gain_db = gain),
                    format!("high shelf {f0} Hz {gain} dB"),
                );
            }
        }
        for fc in [10.0f32, 20.0, 80.0, 300.0, 1_000.0] {
            check(with(|c| c.low_cut_hz = fc), format!("low cut {fc} Hz"));
        }
        for fc in [1_000.0f32, 3_000.0, 8_000.0, 16_000.0, 20_000.0] {
            check(with(|c| c.high_cut_hz = fc), format!("high cut {fc} Hz"));
        }
        assert!(
            tested >= 150 && skipped <= 10,
            "tested {tested}, skipped {skipped}"
        );
        println!(
            "max |processed - cookbook| over {tested} settings: {worst:.3e} dB ({skipped} too long to settle)"
        );
    }

    #[test]
    fn all_six_bands_at_once_stay_within_a_tenth_of_a_decibel() {
        // Random settings across the table's ranges, every band in play: the cascade against the product of the
        // cookbook's sections.
        let mut rng = Rng::new(0xE0E0_0001);
        let mut unit = move || f64::from(rng.bipolar_f32() * 0.5 + 0.5);
        let log = |u: f64, lo: f64, hi: f64| lo * libm::pow(hi / lo, u);
        let mut worst = 0.0f64;
        let mut tested = 0;
        for sr in [44_100.0f32, 48_000.0, 96_000.0] {
            let probes = log_grid(20.0, 0.45 * f64::from(sr), 41);
            for _ in 0..24 {
                let c = Controls {
                    low_cut_hz: log(unit(), 10.0, 1_000.0) as f32,
                    low_freq_hz: log(unit(), 20.0, 1_000.0) as f32,
                    low_gain_db: (unit() * 48.0 - 24.0) as f32,
                    bell1_freq_hz: log(unit(), 20.0, 20_000.0) as f32,
                    bell1_gain_db: (unit() * 48.0 - 24.0) as f32,
                    bell1_q: log(unit(), 0.1, 20.0) as f32,
                    bell2_freq_hz: log(unit(), 20.0, 20_000.0) as f32,
                    bell2_gain_db: (unit() * 48.0 - 24.0) as f32,
                    bell2_q: log(unit(), 0.1, 20.0) as f32,
                    high_freq_hz: log(unit(), 1_000.0, 20_000.0) as f32,
                    high_gain_db: (unit() * 48.0 - 24.0) as f32,
                    high_cut_hz: log(unit(), 1_000.0, 20_000.0) as f32,
                    mix_pct: 100.0,
                };
                if let Some(w) = processed_worst(&c, sr, &probes) {
                    tested += 1;
                    worst = worst.max(w);
                    assert!(w < 0.1, "{c:?} at {sr}: {w} dB");
                }
            }
        }
        assert!(tested >= 50, "{tested}");
        println!("max |processed - cookbook| over {tested} full cascades: {worst:.3e} dB");
    }

    #[test]
    fn the_defaults_are_flat_where_the_table_says() {
        // docs/dsp/eq.md: the cuts at their ends are out of the way, not bypassed.
        let d = Controls::DEFAULT;
        for sr in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            // From 40 Hz to 5 kHz the response is within 0.02 dB of flat at every rate (the high cut is warped
            // flatter than its analog twin at the lower rates and meets it at 192 kHz: -0.02 dB at 5 kHz).
            for f in log_grid(40.0, 5_000.0, 30) {
                assert!(magnitude_db(&d, f, sr).abs() < 0.02, "{f} Hz at {sr}");
            }
            // The corners: -3 dB at 20 kHz exactly (the cookbook pre-warps the corner), and the low cut's value
            // at 20 Hz is the Butterworth's -0.26 dB.
            assert!(
                (magnitude_db(&d, 20_000.0, sr) + 3.0103).abs() < 0.01,
                "{sr}"
            );
            assert!((magnitude_db(&d, 20.0, sr) + 0.2633).abs() < 0.005, "{sr}");
        }
        // Every gain at zero is the identity in output for the middle four bands, and the cuts are not.
        let c = settled(&d, SR);
        for (i, section) in c.iter().enumerate() {
            assert_eq!(section.is_identity(), (1..=4).contains(&i), "band {i}");
        }
    }

    // -----------------------------------------------------------------------------------------------------------
    // The wrap: nothing of the biquad changed, the core is the biquad plus ramps
    // -----------------------------------------------------------------------------------------------------------

    #[test]
    fn a_static_core_matches_bqs_biquads_to_rounding() {
        // The same transfer function by another structure: the core with fixed controls against BQ's `Biquad`
        // sections on the coefficients `Coeffs::design` gives (the cuts from the cookbook's HPF and LPF), run one
        // after another in `f64`. They agree to the rounding of the two structures (about 1e-7 of the peak).
        let c = Controls {
            low_cut_hz: 80.0,
            low_gain_db: 6.0,
            bell1_gain_db: -9.0,
            bell1_q: 3.0,
            bell2_gain_db: 4.5,
            high_gain_db: -5.0,
            high_cut_hz: 15_000.0,
            ..Controls::DEFAULT
        };
        let input = noise(20_000, 5, 0.5);
        let got = run_static(&mut Eq::new(48_000.0), &c, &input);
        let rbj = |b: Rbj| Coeffs {
            b0: b[0] / b[3],
            b1: b[1] / b[3],
            b2: b[2] / b[3],
            a1: b[4] / b[3],
            a2: b[5] / b[3],
        };
        let sections = [
            rbj(rbj_highpass(80.0, SR)),
            Coeffs::design(Kind::LowShelf, 100.0, 6.0, SHELF_Q, SR),
            Coeffs::design(Kind::Peak, 400.0, -9.0, 3.0, SR),
            Coeffs::design(Kind::Peak, 2_500.0, 4.5, 1.0, SR),
            Coeffs::design(Kind::HighShelf, 8_000.0, -5.0, SHELF_Q, SR),
            rbj(rbj_lowpass(15_000.0, SR)),
        ];
        let mut chain: Vec<Biquad> = sections.into_iter().map(Biquad::new).collect();
        let want: Vec<f32> = input
            .iter()
            .map(|&x| chain.iter_mut().fold(x, |v, b| b.tick(v)))
            .collect();
        let peak = want.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let worst = got
            .iter()
            .zip(&want)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        println!("core vs BQ's biquads: worst difference {worst:.2e} on a peak of {peak:.2}");
        assert!(worst < 1e-5 * peak, "{worst} vs peak {peak}");
    }

    #[test]
    fn a_band_at_zero_db_is_the_identity_bit_for_bit_and_keeps_tracking() {
        // The middle four bands of the defaults: the output of each is its input, exactly, while its integrators
        // follow the signal (so raising the gain later starts from a warm section, not from silence).
        let mut section = Section::new();
        let input: [f32; SEGMENT] = noise(SEGMENT, 8, 0.7).try_into().unwrap();
        let (mut l, mut r) = (input, input);
        for _ in 0..10 {
            l = input;
            r = input;
            section.run(
                Band::Bell1,
                [400.0, 0.0, 1.0],
                [400.0, 0.0, 1.0],
                SR,
                &mut l,
                &mut r,
            );
            assert_eq!(bits(&l), bits(&input));
            assert_eq!(bits(&r), bits(&input));
        }
        assert!(
            !section.state.at_rest(),
            "a zero-gain band still tracks the signal"
        );
        // Raised to +6 dB on the next segment it ramps from the identity with its state already warm.
        section.run(
            Band::Bell1,
            [400.0, 0.0, 1.0],
            [400.0, 6.0, 1.0],
            SR,
            &mut l,
            &mut r,
        );
        assert!(l.iter().all(|v| v.is_finite()));
    }

    // -----------------------------------------------------------------------------------------------------------
    // The mix
    // -----------------------------------------------------------------------------------------------------------

    #[test]
    fn mix_zero_passes_the_input_bit_for_bit() {
        let c = Controls {
            bell1_gain_db: 18.0,
            low_gain_db: -12.0,
            mix_pct: 0.0,
            ..Controls::DEFAULT
        };
        let mut eq = Eq::new(48_000.0);
        let mut input = noise(SEGMENT * 40, 21, 0.9);
        // The awkward values: NaN, infinity, -0.0, subnormals, the largest finite value.
        input[3] = f32::NAN;
        input[70] = f32::INFINITY;
        input[71] = -0.0;
        input[72] = f32::MIN_POSITIVE / 8.0;
        input[73] = -f32::MIN_POSITIVE / 3.0;
        input[74] = f32::MAX;
        input[300] = f32::NEG_INFINITY;
        let out = run_static(&mut eq, &c, &input);
        assert_eq!(bits(&out[..input.len()]), bits(&input));
    }

    #[test]
    fn mix_one_is_the_wet_signal_and_half_is_their_average() {
        let wet = Controls {
            bell1_gain_db: 12.0,
            high_gain_db: -8.0,
            ..Controls::DEFAULT
        };
        let input = noise(SEGMENT * 20, 22, 0.5);
        let full = run_static(&mut Eq::new(48_000.0), &wet, &input);
        let same = run_static(
            &mut Eq::new(48_000.0),
            &Controls {
                mix_pct: 100.0,
                ..wet
            },
            &input,
        );
        assert_eq!(bits(&full), bits(&same));
        let half = run_static(
            &mut Eq::new(48_000.0),
            &Controls {
                mix_pct: 50.0,
                ..wet
            },
            &input,
        );
        for ((h, w), d) in half.iter().zip(&full).zip(&input) {
            let want = f64::from(*w) * 0.5 + f64::from(*d) * 0.5;
            assert!((f64::from(*h) - want).abs() < 1e-6, "{h} vs {want}");
        }
    }

    #[test]
    fn the_mix_ramps_across_a_segment_and_the_filters_stay_warm() {
        // From 0 % to 100 % over one segment: the first frame is nearly dry, the last frame fully wet, and the wet
        // path has been running all along (it equals a fully wet run on every frame where m = 1).
        let wet = Controls {
            low_cut_hz: 400.0,
            bell1_gain_db: 12.0,
            ..Controls::DEFAULT
        };
        let dry = Controls {
            mix_pct: 0.0,
            ..wet
        };
        let input = noise(SEGMENT * 6, 23, 0.5);
        let mut a = Eq::new(48_000.0);
        let mut b = Eq::new(48_000.0);
        let mut got = Vec::new();
        let mut reference = Vec::new();
        for (k, chunk) in input.chunks(SEGMENT).enumerate() {
            let (from, to) = match k.cmp(&3) {
                core::cmp::Ordering::Equal => (&dry, &wet),
                core::cmp::Ordering::Less => (&dry, &dry),
                core::cmp::Ordering::Greater => (&wet, &wet),
            };
            let (mut l, mut r) = ([0.0f32; SEGMENT], [0.0f32; SEGMENT]);
            l.copy_from_slice(chunk);
            r.copy_from_slice(chunk);
            a.process(from, to, &mut l, &mut r);
            got.extend_from_slice(&l);
            let (mut l, mut r) = ([0.0f32; SEGMENT], [0.0f32; SEGMENT]);
            l.copy_from_slice(chunk);
            r.copy_from_slice(chunk);
            b.process(&wet, &wet, &mut l, &mut r);
            reference.extend_from_slice(&l);
        }
        // Before the ramp: dry. During: between. After: exactly the always-wet run (state followed the signal).
        assert_eq!(bits(&got[..3 * SEGMENT]), bits(&input[..3 * SEGMENT]));
        assert_eq!(bits(&got[4 * SEGMENT..]), bits(&reference[4 * SEGMENT..]));
        let k = 3 * SEGMENT;
        let m0 = 1.0 / SEGMENT as f64;
        let want0 = f64::from(reference[k]) * m0 + f64::from(input[k]) * (1.0 - m0);
        assert!((f64::from(got[k]) - want0).abs() < 1e-6);
        assert_eq!(
            got[k + SEGMENT - 1].to_bits(),
            reference[k + SEGMENT - 1].to_bits()
        );
    }

    // -----------------------------------------------------------------------------------------------------------
    // Ramps and stability
    // -----------------------------------------------------------------------------------------------------------

    /// The segment of `input` through the core's chain, written out with the ramp spelled by hand: for each band,
    /// `g`, `k` and the mix coefficients move linearly from the old design to the new one, one step per frame, and
    /// the section is the textbook TPT recurrence.
    fn by_hand(from: &Controls, to: &Controls, input: &[f32; SEGMENT]) -> [f32; SEGMENT] {
        let (a, b) = (settled(from, SR), settled(to, SR));
        let mut out = *input;
        for band in 0..BANDS {
            let (mut ic1, mut ic2) = (0.0f64, 0.0f64);
            for (i, x) in out.iter_mut().enumerate() {
                let w = (i + 1) as f64 / SEGMENT as f64;
                let lerp = |p: f64, q: f64| p + (q - p) * w;
                let g = lerp(a[band].g, b[band].g);
                let k = lerp(a[band].k, b[band].k);
                let m = [0, 1, 2].map(|j| lerp(a[band].m[j], b[band].m[j]));
                let a1 = 1.0 / (1.0 + g * (g + k));
                let (a2, a3) = (g * a1, g * g * a1);
                let v0 = f64::from(*x);
                let v3 = v0 - ic2;
                let v1 = a1 * ic1 + a2 * v3;
                let v2 = ic2 + a2 * ic1 + a3 * v3;
                ic1 = 2.0 * v1 - ic1;
                ic2 = 2.0 * v2 - ic2;
                *x = (m[0] * v0 + m[1] * v1 + m[2] * v2) as f32;
            }
        }
        out
    }

    #[test]
    fn a_ramp_walks_from_the_old_design_to_the_new_one_linearly() {
        let from = with(|c| c.bell1_gain_db = 6.0).tap(|c| c.low_cut_hz = 40.0);
        let to = with(|c| {
            c.bell1_gain_db = 18.0;
            c.bell1_freq_hz = 900.0;
            c.high_gain_db = -6.0;
            c.low_cut_hz = 90.0;
        });
        let input: [f32; SEGMENT] = noise(SEGMENT, 31, 0.5).try_into().unwrap();
        let mut eq = Eq::new(48_000.0);
        let (mut l, mut r) = (input, input);
        eq.process(&from, &to, &mut l, &mut r);
        let want = by_hand(&from, &to, &input);
        for (i, (g, w)) in l.iter().zip(&want).enumerate() {
            assert!(
                (g - w).abs() <= 1e-6 * w.abs().max(1.0),
                "frame {i}: {g} vs {w}"
            );
        }
        assert_eq!(bits(&l), bits(&r));
        // The design cached for the next segment is the new one, bit for bit.
        for (band, want) in settled(&to, SR).iter().enumerate() {
            assert_eq!(&eq.sections[band].params, want, "band {band}");
        }
    }

    #[test]
    fn equal_controls_run_static_and_unequal_ones_do_not() {
        let a = with(|c| c.bell1_gain_db = 9.0);
        let b = with(|c| c.bell1_gain_db = 9.5);
        let input: [f32; SEGMENT] = noise(SEGMENT, 32, 0.5).try_into().unwrap();
        let (mut l1, mut r1) = (input, input);
        let (mut l2, mut r2) = (input, input);
        Eq::new(48_000.0).process(&a, &a, &mut l1, &mut r1);
        Eq::new(48_000.0).process(&a, &b, &mut l2, &mut r2);
        assert_ne!(bits(&l1), bits(&l2));
        // A ramp between two equal designs is the static section (the increments are exactly zero).
        let (mut l3, mut r3) = (input, input);
        let mut eq = Eq::new(48_000.0);
        eq.process(&a, &a, &mut l3, &mut r3);
        assert_eq!(bits(&l1), bits(&l3));
    }

    #[test]
    fn skipping_a_silent_segment_is_the_same_as_running_it() {
        // The engine skips an idle module on a silent input. The ramp starts from the design of the previous
        // controls, so a skipped segment leaves nothing behind that a run would have changed.
        let quiet = with(|c| c.bell1_gain_db = 10.0);
        let moved = with(|c| {
            c.bell1_gain_db = -14.0;
            c.bell1_freq_hz = 3_000.0;
            c.low_cut_hz = 200.0;
        });
        let input: [f32; SEGMENT] = noise(SEGMENT, 33, 0.5).try_into().unwrap();
        let mut ran = Eq::new(48_000.0);
        let mut skipped = Eq::new(48_000.0);
        // `ran` goes through the silent segment with the controls moving, `skipped` does not see it.
        let (mut l, mut r) = ([0.0f32; SEGMENT], [0.0f32; SEGMENT]);
        ran.process(&quiet, &moved, &mut l, &mut r);
        assert!(l.iter().all(|x| x.to_bits() == 0));
        assert!(ran.is_idle() && skipped.is_idle());
        // The next segment has the same (previous, current) controls for both: `moved` held.
        let (mut l1, mut r1) = (input, input);
        let (mut l2, mut r2) = (input, input);
        ran.process(&moved, &moved, &mut l1, &mut r1);
        skipped.process(&moved, &moved, &mut l2, &mut r2);
        assert_eq!(bits(&l1), bits(&l2));
        assert_eq!(bits(&r1), bits(&r2));
    }

    /// The largest singular value of the section's state update `[[2a1−1, −2a2], [2a2, 1−2a3]]`.
    fn update_norm(p: &Params) -> f64 {
        let a1 = 1.0 / (1.0 + p.g * (p.g + p.k));
        let (a2, a3) = (p.g * a1, p.g * p.g * a1);
        let m = [[2.0 * a1 - 1.0, -2.0 * a2], [2.0 * a2, 1.0 - 2.0 * a3]];
        let t = m[0][0] * m[0][0] + m[0][1] * m[0][1] + m[1][0] * m[1][0] + m[1][1] * m[1][1];
        let d = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        libm::sqrt((t + libm::sqrt((t * t - 4.0 * d * d).max(0.0))) / 2.0)
    }

    #[test]
    fn every_section_the_table_allows_is_passive_and_so_is_every_point_of_a_ramp() {
        // The structural guarantee behind the stability row: the state update of a TPT section never expands (its
        // largest singular value is 1 for every g > 0 and k >= 0), so no sequence of controls can feed it energy.
        // Checked at the corners of the table's ranges at five sample rates, and along the straight line between
        // pairs of designs, which is what a ramp walks.
        let freqs = [10.0f32, 20.0, 100.0, 1_000.0, 8_000.0, 20_000.0];
        let gains = [-24.0f32, -0.5, 0.0, 0.5, 24.0];
        let qs = [0.1f32, 1.0, 20.0];
        let mut count = 0;
        let mut worst = 0.0f64;
        for sr in [8_000.0, 32_000.0, 44_100.0, 96_000.0, 192_000.0] {
            let mut all: Vec<Params> = Vec::new();
            for &f in &freqs {
                all.push(highpass(f64::from(f), sr));
                all.push(lowpass(f64::from(f), sr));
                for &g in &gains {
                    all.push(low_shelf(f64::from(f), f64::from(g), SHELF_Q, sr));
                    all.push(high_shelf(f64::from(f), f64::from(g), SHELF_Q, sr));
                    for &q in &qs {
                        all.push(bell(f64::from(f), f64::from(g), f64::from(q), sr));
                    }
                }
            }
            for p in &all {
                assert!(
                    p.g > 0.0 && p.g.is_finite() && p.k > 0.0 && p.k.is_finite(),
                    "{p:?} at {sr}"
                );
                worst = worst.max(update_norm(p));
            }
            for (i, a) in all.iter().enumerate() {
                for b in all.iter().skip(i % 17).step_by(17) {
                    for w in [0.0625, 0.25, 0.5, 0.75, 0.9375] {
                        let p = Params {
                            g: a.g + (b.g - a.g) * w,
                            k: a.k + (b.k - a.k) * w,
                            m: [0.0; 3],
                        };
                        worst = worst.max(update_norm(&p));
                        count += 1;
                    }
                }
            }
        }
        assert!(count > 10_000, "{count}");
        println!(
            "{count} ramp points; the largest singular value of any state update is {worst:.15}"
        );
        // The norm is computed from the square root of a difference that is zero when the section is lossless, so
        // it carries about 1e-11 of rounding itself.
        assert!(worst < 1.0 + 1e-9, "{worst}");
    }

    /// Controls drawn from the corners and the middle of every range: the harshest thing a modulator can do.
    fn extreme(rng: &mut Rng) -> Controls {
        let mut pick = |lo: f32, hi: f32| {
            let u = rng.bipolar_f32();
            if u < -0.33 {
                lo
            } else if u > 0.33 {
                hi
            } else if lo > 0.0 {
                // The geometric middle of a log range.
                libm::sqrt(f64::from(lo) * f64::from(hi)) as f32
            } else {
                // The middle of a gain range.
                0.0
            }
        };
        Controls {
            low_cut_hz: pick(10.0, 1_000.0),
            low_freq_hz: pick(20.0, 1_000.0),
            low_gain_db: pick(-24.0, 24.0),
            bell1_freq_hz: pick(20.0, 20_000.0),
            bell1_gain_db: pick(-24.0, 24.0),
            bell1_q: pick(0.1, 20.0),
            bell2_freq_hz: pick(20.0, 20_000.0),
            bell2_gain_db: pick(-24.0, 24.0),
            bell2_q: pick(0.1, 20.0),
            high_freq_hz: pick(1_000.0, 20_000.0),
            high_gain_db: pick(-24.0, 24.0),
            high_cut_hz: pick(1_000.0, 20_000.0),
            mix_pct: 100.0,
        }
    }

    /// Drive `eq` with `input` in segments, the controls coming from `controls(k)` for segment `k` (previous and
    /// current), and return the left channel.
    fn run_moving(
        eq: &mut Eq,
        input: &[f32],
        mut controls: impl FnMut(usize) -> Controls,
    ) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        let mut prev = controls(0);
        for (k, chunk) in input.as_chunks::<SEGMENT>().0.iter().enumerate() {
            let cur = controls(k + 1);
            let mut l: Segment = *chunk;
            let mut r = l;
            eq.process(&prev, &cur, &mut l, &mut r);
            out.extend_from_slice(&l);
            prev = cur;
        }
        out
    }

    fn peak_of(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    fn power_of(v: &[f32]) -> f64 {
        v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / v.len() as f64
    }

    #[test]
    fn one_parameter_jumping_between_its_extremes_every_segment_is_bounded_by_the_frozen_ends() {
        // The brief's stability row, one control at a time: the control picks one of its two extremes at random for
        // every segment (a jump of the whole range 1500 times a second, ramped across each segment), under full-scale
        // noise. A transposed-direct-form biquad with ramped coefficients pumps here (the bell sweep went to
        // infinity, the low shelf to 10x); the TPT section stays within a few times the louder of the two frozen
        // ends. Measured worst ratio 2.4 (bell1 frequency, +24 dB, Q 20); the bound is 4.
        let sr = 48_000.0f32;
        let input = noise(SEGMENT * 1500 * 5, 143, 1.0);
        let cases: [(&str, f32, f32, SetOne, Controls); 8] = [
            (
                "low_cut",
                10.0,
                1_000.0,
                |c, v| c.low_cut_hz = v,
                Controls::DEFAULT,
            ),
            (
                "low_freq +24",
                20.0,
                1_000.0,
                |c, v| c.low_freq_hz = v,
                with(|c| c.low_gain_db = 24.0),
            ),
            (
                "low_gain",
                -24.0,
                24.0,
                |c, v| c.low_gain_db = v,
                Controls::DEFAULT,
            ),
            (
                "bell1_freq +24 Q20",
                20.0,
                20_000.0,
                |c, v| c.bell1_freq_hz = v,
                with(|c| {
                    c.bell1_gain_db = 24.0;
                    c.bell1_q = 20.0;
                }),
            ),
            (
                "bell1_gain Q20",
                -24.0,
                24.0,
                |c, v| c.bell1_gain_db = v,
                with(|c| {
                    c.bell1_q = 20.0;
                    c.bell1_freq_hz = 100.0;
                }),
            ),
            (
                "bell1_q +24",
                0.1,
                20.0,
                |c, v| c.bell1_q = v,
                with(|c| {
                    c.bell1_gain_db = 24.0;
                    c.bell1_freq_hz = 100.0;
                }),
            ),
            (
                "high_freq +24",
                1_000.0,
                20_000.0,
                |c, v| c.high_freq_hz = v,
                with(|c| c.high_gain_db = 24.0),
            ),
            (
                "high_cut",
                1_000.0,
                20_000.0,
                |c, v| c.high_cut_hz = v,
                Controls::DEFAULT,
            ),
        ];
        let mut worst = 0.0f32;
        for (name, lo, hi, set, base) in cases {
            let mut rng = Rng::new(0xE0E0_3000);
            let controls: Vec<Controls> = (0..=input.len() / SEGMENT)
                .map(|_| {
                    let mut c = base;
                    set(&mut c, if rng.bipolar_f32() < 0.0 { lo } else { hi });
                    c
                })
                .collect();
            let out = run_moving(&mut Eq::new(sr), &input, |k| controls[k]);
            assert!(out.iter().all(|v| v.is_finite()), "{name}");
            let moving = peak_of(&out[2_000..]);
            let frozen = [lo, hi].iter().fold(0.0f32, |m, &v| {
                let mut c = base;
                set(&mut c, v);
                m.max(peak_of(&run_static(&mut Eq::new(sr), &c, &input)[2_000..]))
            });
            println!(
                "{name}: moving peak {moving:.1}, frozen ends {frozen:.1}, ratio {:.2}",
                moving / frozen
            );
            assert!(moving < 4.0 * frozen, "{name}: {moving} against {frozen}");
            worst = worst.max(moving / frozen);
        }
        println!("one parameter at a time: worst moving/frozen ratio {worst:.2}");
    }

    #[test]
    fn every_pair_of_corner_settings_of_a_band_alternating_every_segment_stays_bounded() {
        // Each band alone (the others at their defaults): every pair of settings from a grid of the table's corners
        // and middles (frequency x gain x Q) alternates segment by segment for a second of full-scale noise. The
        // peak stays within 16x the louder frozen setting; the worst pair measured 9.3, a +24 dB Q 1 bell at 20 Hz
        // against a 0 dB Q 20 bell at 20 kHz (the ramp passes through a narrow +21 dB bell in the middle).
        let sr = 48_000.0f32;
        let input = noise(SEGMENT * 1500, 160, 1.0);
        let kinds: [(&str, SetThree, [f32; 3]); 6] = [
            (
                "low_cut",
                |c, f, _, _| c.low_cut_hz = f,
                [10.0, 100.0, 1_000.0],
            ),
            (
                "low shelf",
                |c, f, g, _| {
                    c.low_freq_hz = f;
                    c.low_gain_db = g;
                },
                [20.0, 140.0, 1_000.0],
            ),
            (
                "bell1",
                |c, f, g, q| {
                    c.bell1_freq_hz = f;
                    c.bell1_gain_db = g;
                    c.bell1_q = q;
                },
                [20.0, 632.0, 20_000.0],
            ),
            (
                "bell2",
                |c, f, g, q| {
                    c.bell2_freq_hz = f;
                    c.bell2_gain_db = g;
                    c.bell2_q = q;
                },
                [20.0, 632.0, 20_000.0],
            ),
            (
                "high shelf",
                |c, f, g, _| {
                    c.high_freq_hz = f;
                    c.high_gain_db = g;
                },
                [1_000.0, 4_500.0, 20_000.0],
            ),
            (
                "high_cut",
                |c, f, _, _| c.high_cut_hz = f,
                [1_000.0, 4_500.0, 20_000.0],
            ),
        ];
        let mut overall = 0.0f32;
        for (name, set, freqs) in kinds {
            let mut settings: Vec<Controls> = Vec::new();
            for &f in &freqs {
                for g in [-24.0f32, 0.0, 24.0] {
                    for q in [0.1f32, 1.0, 20.0] {
                        let mut c = Controls::DEFAULT;
                        set(&mut c, f, g, q);
                        if !settings.contains(&c) {
                            settings.push(c);
                        }
                    }
                }
            }
            let frozen: Vec<f32> = settings
                .iter()
                .map(|c| peak_of(&run_static(&mut Eq::new(sr), c, &input)[2_000..]))
                .collect();
            let mut worst = 0.0f32;
            for i in 0..settings.len() {
                for j in (i + 1)..settings.len() {
                    let out = run_moving(&mut Eq::new(sr), &input, |k| {
                        if k % 2 == 0 { settings[i] } else { settings[j] }
                    });
                    assert!(out.iter().all(|v| v.is_finite()), "{name}: {i} {j}");
                    let ratio = peak_of(&out[2_000..]) / frozen[i].max(frozen[j]);
                    assert!(
                        ratio < 16.0,
                        "{name}: {:?} and {:?} give {ratio}",
                        settings[i],
                        settings[j]
                    );
                    worst = worst.max(ratio);
                }
            }
            println!(
                "{name}: {} settings, worst alternating ratio {worst:.2}",
                settings.len()
            );
            overall = overall.max(worst);
        }
        println!("worst alternating ratio over every band: {overall:.2}");
    }

    #[test]
    fn all_bands_jumping_to_random_extremes_every_segment_stay_finite_and_stationary() {
        // The harshest case: every band, every segment, a corner or the middle of its range, under full-scale noise
        // for 20 s at each sample rate. A cascade of four +24 dB sections is loud when frozen (the sum of |h| of
        // the loudest of 40 random corner settings is the first number below), and modulation adds the energy of
        // each switch, so the bound here is a regression guard and not a derived limit: measured peaks 2.6e3 to
        // 1.3e4, the guard 5e4. What the test pins is that it is stationary: the second half is no louder than
        // 4x the first (a pumping section grows without limit).
        let mut rng = Rng::new(0xE0E0_0002);
        let mut frozen_worst = 0.0f64;
        for _ in 0..40 {
            let c = extreme(&mut rng);
            let (h, _) = impulse_response(&c, 48_000.0, 1 << 17);
            frozen_worst = frozen_worst.max(h.iter().map(|v| f64::from(v.abs())).sum::<f64>());
        }
        let mut peak_seen = 0.0f32;
        for (sr, seed) in [
            (32_000.0f32, 1u64),
            (44_100.0, 2),
            (48_000.0, 3),
            (96_000.0, 4),
            (192_000.0, 5),
        ] {
            let mut crng = Rng::new(0xE0E0_1000 + seed);
            let segments = (20.0 * sr / SEGMENT as f32) as usize;
            let input = noise(segments * SEGMENT, 40 + seed, 1.0);
            let mut eq = Eq::new(sr);
            let cs: Vec<Controls> = (0..=segments).map(|_| extreme(&mut crng)).collect();
            let out = run_moving(&mut eq, &input, |k| cs[k]);
            assert!(out.iter().all(|v| v.is_finite()), "non-finite at {sr}");
            let peak = peak_of(&out);
            peak_seen = peak_seen.max(peak);
            assert!(peak < 5.0e4, "peak {peak} at {sr}");
            let half = out.len() / 2;
            let (first, second) = (power_of(&out[..half]), power_of(&out[half..]));
            println!("sr {sr}: peak {peak:.0}, power {first:.0} then {second:.0}");
            assert!(second < 4.0 * first, "{first} then {second} at {sr}");
        }
        println!(
            "random extremes every segment: worst peak {peak_seen:.0} for a +-1 noise input; the loudest frozen corner setting's sum of |h| is {frozen_worst:.1}"
        );
    }

    #[test]
    fn range_corners_held_static_are_bounded_by_their_own_impulse_response() {
        // Every corner of the table's ranges, held: |y| <= sum|h| · max|x| for a linear time-invariant section chain
        // (a rigorous bound, so a blow-up cannot hide). One second of full-scale noise each.
        let mut rng = Rng::new(0xE0E0_0003);
        let mut cases = 0;
        for sr in [44_100.0f32, 48_000.0, 96_000.0] {
            for _ in 0..24 {
                let c = extreme(&mut rng);
                let (h, settled) = impulse_response(&c, sr, 1 << 19);
                if !settled {
                    continue;
                }
                let bound = h.iter().map(|v| f64::from(v.abs())).sum::<f64>();
                let input = noise(sr as usize / 32 * 32, 55 + cases, 1.0);
                let out = run_static(&mut Eq::new(sr), &c, &input);
                let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(
                    f64::from(peak) <= bound * 1.001 + 1e-6,
                    "{c:?} at {sr}: {peak} vs {bound}"
                );
                cases += 1;
            }
        }
        assert!(cases >= 40, "{cases}");
    }

    #[test]
    fn a_sine_through_a_swept_band_has_no_zipper_that_a_ramp_does_not_remove() {
        // A bell swept by an 80 Hz sine in the log domain between 300 Hz and 6 kHz at +18 dB, Q 8, on a 1 kHz sine
        // (the controls change every 32 frames, as in the engine). Stepped (previous = current, so each segment
        // jumps) the output carries a skirt of edges at the segment rate; ramped it is much cleaner. The skirt is
        // the strongest bin 1.2 to 14 kHz above the carrier, in dB re the carrier's strongest bin.
        let sr = 48_000.0f32;
        let frames = 4 * crate::alias::FFT_LEN;
        let input: Vec<f32> = crate::alias::sine(1_000.0, 0.5, f64::from(sr), frames);
        let bell = |k: usize| {
            let t = k as f64 * SEGMENT as f64 / f64::from(sr);
            let f = 1_350.0 * libm::pow(20.0, 0.5 * libm::sin(TAU * 80.0 * t) - 0.0)
                / libm::sqrt(20.0)
                * 1.0;
            Controls {
                bell1_freq_hz: (f * 1.0).clamp(300.0, 6_000.0) as f32,
                bell1_gain_db: 18.0,
                bell1_q: 8.0,
                ..Controls::DEFAULT
            }
        };
        let skirt = |out: &[f32]| {
            let p = crate::alias::power_spectrum(&out[out.len() - crate::alias::FFT_LEN..]);
            let bin = f64::from(sr) / crate::alias::FFT_LEN as f64;
            let at = |hz: f64| (hz / bin).round() as usize;
            let peak = p[at(1_000.0) - 8..=at(1_000.0) + 8]
                .iter()
                .fold(0.0f64, |m, &x| m.max(x));
            let worst = p[at(2_200.0)..=at(15_000.0)]
                .iter()
                .fold(0.0f64, |m, &x| m.max(x));
            10.0 * libm::log10(worst / peak)
        };
        let ramped = run_moving(&mut Eq::new(sr), &input, bell);
        // Stepped: each segment is processed with previous = current, so the coefficients jump at its first frame.
        let mut stepped_eq = Eq::new(sr);
        let mut stepped = Vec::with_capacity(frames);
        for (k, chunk) in input.as_chunks::<SEGMENT>().0.iter().enumerate() {
            let c = bell(k + 1);
            let mut l: Segment = *chunk;
            let mut r = l;
            stepped_eq.process(&c, &c, &mut l, &mut r);
            stepped.extend_from_slice(&l);
        }
        let (a, b) = (skirt(&ramped), skirt(&stepped));
        println!("zipper skirt: ramped {a:.1} dB, stepped {b:.1} dB re the carrier");
        assert!(a < b - 15.0, "ramped {a} vs stepped {b}");
    }

    #[test]
    fn a_nan_in_the_input_does_not_poison_the_equaliser() {
        let c = Controls {
            bell1_gain_db: 12.0,
            low_cut_hz: 100.0,
            ..Controls::DEFAULT
        };
        let clean = noise(SEGMENT * 30, 61, 0.5);
        let mut dirty = clean.clone();
        dirty[5 * SEGMENT + 3] = f32::NAN;
        dirty[9 * SEGMENT] = f32::INFINITY;
        let mut a = Eq::new(48_000.0);
        let out = run_static(&mut a, &c, &dirty);
        let fresh = run_static(&mut Eq::new(48_000.0), &c, &clean);
        // The segments with a bad sample are lost; the section is cleared at their end, so from the next clean one
        // on the output is that of a fresh equaliser fed the same samples from there.
        let resume = 10 * SEGMENT;
        assert!(out[resume..].iter().all(|v| v.is_finite()));
        let mut b = Eq::new(48_000.0);
        let fresh_tail = run_static(&mut b, &c, &clean[resume..]);
        assert_eq!(
            bits(&out[resume..SEGMENT * 30]),
            bits(&fresh_tail[..SEGMENT * 30 - resume])
        );
        assert!(fresh.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn latency_is_zero() {
        // The first output sample of an impulse is the product of the sections' direct terms (the cookbook's
        // b0/a0 each): nothing is delayed.
        let mut x = vec![0.0f32; SEGMENT];
        x[0] = 1.0;
        for c in [
            Controls::DEFAULT,
            with(|c| {
                c.bell1_gain_db = 12.0;
                c.low_cut_hz = 200.0;
                c.high_gain_db = -6.0;
            }),
        ] {
            let out = run_static(&mut Eq::new(48_000.0), &c, &x);
            let sr = 48_000.0;
            let direct = |b: Rbj| b[0] / b[3];
            let b0 = direct(rbj_highpass(f64::from(c.low_cut_hz), sr))
                * direct(rbj_lowshelf(
                    f64::from(c.low_freq_hz),
                    f64::from(c.low_gain_db),
                    sr,
                ))
                * direct(rbj_peaking(
                    f64::from(c.bell1_freq_hz),
                    f64::from(c.bell1_gain_db),
                    f64::from(c.bell1_q),
                    sr,
                ))
                * direct(rbj_peaking(
                    f64::from(c.bell2_freq_hz),
                    f64::from(c.bell2_gain_db),
                    f64::from(c.bell2_q),
                    sr,
                ))
                * direct(rbj_highshelf(
                    f64::from(c.high_freq_hz),
                    f64::from(c.high_gain_db),
                    sr,
                ))
                * direct(rbj_lowpass(f64::from(c.high_cut_hz), sr));
            assert!(b0 > 0.1, "{b0}");
            assert!((f64::from(out[0]) - b0).abs() < 1e-6, "{} vs {b0}", out[0]);
        }
    }

    #[test]
    fn a_clone_continues_exactly_where_the_original_would() {
        let c = with(|c| c.bell1_gain_db = 9.0).tap(|c| c.high_gain_db = -6.0);
        let first = noise(SEGMENT * 20, 71, 0.5);
        let second = noise(SEGMENT * 20, 72, 0.5);
        let mut a = Eq::new(48_000.0);
        run_static(&mut a, &c, &first);
        let mut b = Eq::new(48_000.0);
        b.clone_from(&a);
        let out_a = run_static(&mut a, &c, &second);
        let out_b = run_static(&mut b, &c, &second);
        assert_eq!(bits(&out_a), bits(&out_b));
    }

    #[test]
    fn a_long_tail_has_no_subnormals_and_reaches_exact_zero() {
        // 40 s of silence after a loud burst, for a typical setting and for the longest ring the table allows (a
        // +24 dB, Q 20 bell at 20 Hz with a +24 dB shelf there: its poles decay with a time constant of about
        // 1.3 s): never a subnormal in the state or the output, and the state reaches exact zero (the 1e-15 flush)
        // inside the 40 s, so the engine can skip the module.
        let sr = 48_000.0f32;
        let burst = noise(SEGMENT * 100, 81, 1.0);
        for c in [
            with(|c| {
                c.low_cut_hz = 60.0;
                c.bell1_gain_db = 12.0;
                c.bell1_q = 6.0;
                c.high_gain_db = 6.0;
            }),
            with(|c| {
                c.bell1_freq_hz = 20.0;
                c.bell1_gain_db = 24.0;
                c.bell1_q = 20.0;
                c.low_gain_db = 24.0;
                c.low_freq_hz = 20.0;
            }),
        ] {
            let mut eq = Eq::new(sr);
            run_static(&mut eq, &c, &burst);
            assert!(!eq.is_idle());
            let silence = vec![0.0f32; SEGMENT];
            let mut idle_at = None;
            for k in 0..(40.0 * sr / SEGMENT as f32) as usize {
                let out = run_static(&mut eq, &c, &silence);
                assert!(
                    out.iter()
                        .all(|v| *v == 0.0 || v.abs() >= f32::MIN_POSITIVE),
                    "subnormal output at segment {k}"
                );
                if k % 500 == 0 {
                    assert_eq!(eq.count_subnormals(), 0, "subnormal state at segment {k}");
                }
                if idle_at.is_none() && eq.is_idle() {
                    idle_at = Some(k);
                }
            }
            assert_eq!(eq.count_subnormals(), 0);
            let k = idle_at.unwrap_or_else(|| panic!("never idle: {c:?}"));
            println!("idle after {:.1} s of silence", (k * SEGMENT) as f32 / sr);
            assert!(eq.is_idle(), "{c:?}");
        }
    }

    #[test]
    fn controls_with_non_finite_values_are_the_defaults_and_the_mix_is_clamped() {
        let bad = Controls {
            low_cut_hz: f32::NAN,
            bell1_gain_db: f32::INFINITY,
            bell2_q: f32::NEG_INFINITY,
            high_cut_hz: f32::NAN,
            mix_pct: 250.0,
            ..Controls::DEFAULT
        };
        let s = bad.sanitized();
        assert_eq!(s.low_cut_hz, 10.0);
        assert_eq!(s.bell1_gain_db, 0.0);
        assert_eq!(s.bell2_q, 1.0);
        assert_eq!(s.high_cut_hz, 20_000.0);
        assert_eq!(s.mix_pct, 100.0);
        assert_eq!(
            Controls {
                mix_pct: -5.0,
                ..Controls::DEFAULT
            }
            .sanitized()
            .mix_pct,
            0.0
        );
        let input = noise(SEGMENT * 4, 91, 0.5);
        let out = run_static(&mut Eq::new(48_000.0), &bad, &input);
        let want = run_static(&mut Eq::new(48_000.0), &Controls::DEFAULT, &input);
        assert_eq!(bits(&out), bits(&want));
    }
}
