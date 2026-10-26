//! The DSP core of the `dist` module (S2a lane DST; `docs/dsp/dist.md`): five shaping curves with first-order
//! antiderivative anti-aliasing (ADAA-1) and zero-latency IIR oversampling ([`crate::resample`]), a bitcrusher and a
//! sample-and-hold downsampler, and the filters around them: the `keep_lows` crossover, a DC blocker, the `tone`
//! low-pass and the phase-matched dry path that `mix` blends against.
//!
//! **Signal path** (per voice, `f64` inside, the bus is `f32`):
//!
//! ```text
//!   x ─┬─ crossover (LR4 at keep_lows) ─ lo ───────────────────────── PhaseChain ─┐
//!      │                              └ hi ─ ×drive +bias ─ curve ─ DC block ──────┴─ + ─ tone LP ─ wet ─┐
//!      │                                                                                                 ├─ blend ─ out
//!      └─ (lo + hi, or x) ─ PhaseChain ─ tone all-pass ─ dry ────────────────────────────────────────────┘
//! ```
//!
//! - **Curves** ([`Mode`]): `soft` is `tanh`, `hard` a clamp to ±1, `asymmetric` `tanh` for positive and `½·tanh(2u)`
//!   for negative input (equal slope at 0, limits +1 and −½), `sine_fold` is `sin(u)` (first fold at `|u|` = π/2, then
//!   every π) and `triangle_fold` a triangle wave of period 4 with unit slope (folds at `|u|` = 1, 3, 5, …). Every
//!   curve has slope 1 at 0 and a peak of 1, so changing mode changes the character and not the level of a quiet
//!   signal. `drive` is a gain of `10^(dB/20)` and `bias` an offset of `bias/100` added *after* it, so 100 % puts the
//!   curve's own unit level on the signal at any drive.
//! - **ADAA-1** (Parker, Zavalishin and Le Bivic, *Reducing the aliasing of nonlinear waveshaping using
//!   continuous-time convolution*, DAFx-16, 2016; Bilbao, Esqueda, Parker and Välimäki, *Antiderivative antialiasing
//!   for memoryless nonlinearities*, IEEE SPL 24(7), 2017): `y[n] = (F(u[n]) − F(u[n−1])) / (u[n] − u[n−1])` with the
//!   closed-form antiderivative `F` of each curve, and the curve at the midpoint when the two inputs are closer than
//!   `1e-5` (a constant input then returns the curve exactly). In a linear stretch of the curve this is the average of
//!   two samples: a half-sample delay and a `cos(ω/2)` droop of the wet path, which the dry path copies in phase.
//! - **Oversampling** `off`, `x2`, `x4`: the driven signal goes through the half-band IIR stages of
//!   [`crate::resample`], the ADAA curve runs at the high rate and the result is decimated. `bitcrush` and
//!   `downsample` are not oversampled (their aliasing is the effect).
//! - **`bitcrush`**: the driven signal is clamped to ±1 (an ADC's range) and rounded to `2^(bits−1)` steps per side
//!   (mid-tread, so silence stays silence; fractional `bits` allowed). **`downsample`**: the clamped driven signal is
//!   held for `fs/rate` samples (a zero-order hold on a phase accumulator); at `rate` at or above the engine's rate
//!   every sample is taken and the hold is the identity.
//! - **`keep_lows`**: a fourth-order Linkwitz-Riley crossover (Linkwitz, *Active crossover networks for noncoincident
//!   drivers*, JAES 24(1), 1976; two cascaded 2nd-order Butterworth sections in Simper's trapezoidal state-variable
//!   form, Cytomic, *Linear trapezoidal integrated SVF*, 2013). The low band skips the curve and is added back after
//!   the DC blocker through the same phase chain as the dry path, so in the linear limit low plus shaped high is the
//!   crossover's all-pass; at the 20 Hz minimum there is no split.
//! - **`tone`**: a one-pole low-pass in the trapezoidal form (Zavalishin, *The Art of VA Filter Design*, 2018):
//!   `|H| = 1/√(1 + (tan(ω/2)/g)²)`, `g = tan(π·tone/fs)`, −3 dB at `tone` up to the bilinear warp. At the 20 kHz
//!   maximum the filter is out of the path.
//! - **The dry path**: the wet path's linear stages shift phase (the oversampler's all-pass branches, the ADAA
//!   half-sample, the crossover, the tone filter), so `mix` blends against the input put through all-pass filters of
//!   the same phase: a fixed chain of first-order sections per oversampling factor ([`PhaseChain`], fitted to within
//!   a few degrees from 50 Hz to 12 kHz), the crossover's own sum, and a first-order all-pass whose group delay
//!   matches the tone filter's at DC (`2·lp − x` with the filter's corner doubled). At `mix` 0 the voice is not
//!   processed at all and the bus is untouched, bit for bit; the blend fades in over 8 segments when `mix` leaves 0
//!   so that the one step from the raw input to the phase-matched dry cannot click.
//!
//! **Determinism** (AGENTS.md): `f64` throughout, libm through `util::math`, no fused operations, a fixed order of
//! evaluation, every recursive state flushed once per block (`util::flush`), nothing reads CPU flags. Nothing here
//! allocates.
//!
//! Nothing is ported: the structures are from the papers above and the equations of each stage are written out in
//! `docs/dsp/dist.md`.

use core::f64::consts::{LN_2, PI, SQRT_2};

use crate::resample::{Oversampler, PHASE_SECTIONS, PhaseChain, phase_chain_coefs};
use crate::util::flush::flush64;
use crate::util::math::{DB_TO_LOG2, cos, exp, exp2, log, sin, tan, tanh};

/// `keep_lows` values at or below this (the parameter's minimum, in Hz) have no crossover.
pub const KEEP_LOWS_OFF_HZ: f32 = 20.0;
/// `tone` values at or above this (the parameter's maximum, in Hz) take the low-pass out of the path.
pub const TONE_OPEN_HZ: f32 = 20_000.0;
/// The DC blocker's corner (Hz): far below the lowest note, far above anything `bias` modulation does.
pub const DC_BLOCK_HZ: f64 = 5.0;
/// Inputs of the ADAA step closer than this use the curve at their midpoint.
pub const ADAA_EPS: f64 = 1.0e-5;
/// The mix fade moves this much of its 0..1 range per block.
pub const FADE_STEP: f64 = 0.125;

/// The distortion mode (the F1m `mode` values, in order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `tanh(u)`.
    Soft,
    /// `clamp(u, −1, 1)`.
    Hard,
    /// `tanh(u)` for `u ≥ 0`, `½·tanh(2u)` below.
    Asymmetric,
    /// `sin(u)`.
    SineFold,
    /// A triangle wave of period 4 and unit slope at 0.
    TriangleFold,
    /// Clamp to ±1 and rounding to `2^(bits−1)` steps per side.
    Bitcrush,
    /// Clamp to ±1 and a sample-and-hold at `rate`.
    Downsample,
}

impl Mode {
    /// The modes that are not oversampled and have no curve: the converters.
    #[must_use]
    pub fn is_digital(self) -> bool {
        matches!(self, Mode::Bitcrush | Mode::Downsample)
    }

    /// The static transfer curve at DC: the value of the curve at `u` (the converter's clamp for the digital modes).
    #[must_use]
    pub fn transfer(self, u: f64) -> f64 {
        match self {
            Mode::Soft => SoftCurve::f(u),
            Mode::Hard => HardCurve::f(u),
            Mode::Asymmetric => AsymCurve::f(u),
            Mode::SineFold => SineCurve::f(u),
            Mode::TriangleFold => TriCurve::f(u),
            Mode::Bitcrush | Mode::Downsample => u.clamp(-1.0, 1.0),
        }
    }

    /// The antiderivative of [`Mode::transfer`] (zero at 0 where it has no other constant).
    fn anti(self, u: f64) -> f64 {
        match self {
            Mode::Soft => SoftCurve::anti(u),
            Mode::Hard => HardCurve::anti(u),
            Mode::Asymmetric => AsymCurve::anti(u),
            Mode::SineFold => SineCurve::anti(u),
            Mode::TriangleFold => TriCurve::anti(u),
            Mode::Bitcrush | Mode::Downsample => 0.0,
        }
    }
}

/// A shaping curve and its antiderivative in closed form.
pub trait Curve {
    /// The curve.
    fn f(u: f64) -> f64;
    /// An antiderivative of [`Curve::f`] (constants are irrelevant to ADAA).
    fn anti(u: f64) -> f64;
}

/// `ln cosh(u)`, stable for any `u`: `|u| + ln(1 + e^(−2|u|)) − ln 2`.
#[inline(always)]
fn ln_cosh(u: f64) -> f64 {
    let a = u.abs();
    if a > 20.0 {
        a - LN_2
    } else {
        a + log(1.0 + exp(-2.0 * a)) - LN_2
    }
}

/// `tanh`.
pub struct SoftCurve;
impl Curve for SoftCurve {
    #[inline(always)]
    fn f(u: f64) -> f64 {
        tanh(u)
    }
    #[inline(always)]
    fn anti(u: f64) -> f64 {
        ln_cosh(u)
    }
}

/// The clamp.
pub struct HardCurve;
impl Curve for HardCurve {
    #[inline(always)]
    fn f(u: f64) -> f64 {
        u.clamp(-1.0, 1.0)
    }
    #[inline(always)]
    fn anti(u: f64) -> f64 {
        let a = u.abs();
        if a <= 1.0 { 0.5 * u * u } else { a - 0.5 }
    }
}

/// `tanh` above 0, `½·tanh(2u)` below.
pub struct AsymCurve;
impl Curve for AsymCurve {
    #[inline(always)]
    fn f(u: f64) -> f64 {
        if u >= 0.0 {
            tanh(u)
        } else {
            0.5 * tanh(2.0 * u)
        }
    }
    #[inline(always)]
    fn anti(u: f64) -> f64 {
        if u >= 0.0 {
            ln_cosh(u)
        } else {
            0.25 * ln_cosh(2.0 * u)
        }
    }
}

/// `sin(u)`.
pub struct SineCurve;
impl Curve for SineCurve {
    #[inline(always)]
    fn f(u: f64) -> f64 {
        sin(u)
    }
    #[inline(always)]
    fn anti(u: f64) -> f64 {
        -cos(u)
    }
}

/// The triangle wave of period 4: `u` on `[−1, 1]`, `2 − u` on `[1, 3]`, and so on.
pub struct TriCurve;
impl TriCurve {
    /// `(u + 1) mod 4`, in `[0, 4)`.
    #[inline(always)]
    fn phase(u: f64) -> f64 {
        let w = u + 1.0;
        w - 4.0 * (w * 0.25).floor()
    }
}
impl Curve for TriCurve {
    #[inline(always)]
    fn f(u: f64) -> f64 {
        let r = Self::phase(u);
        if r < 2.0 { r - 1.0 } else { 3.0 - r }
    }
    #[inline(always)]
    fn anti(u: f64) -> f64 {
        // The integral over one period is 0, so the antiderivative is periodic too.
        let r = Self::phase(u);
        if r < 2.0 {
            0.5 * r * r - r
        } else {
            3.0 * r - 0.5 * r * r - 4.0
        }
    }
}

/// The state of one ADAA-1 stage: the previous input and the antiderivative there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Adaa {
    u1: f64,
    f1: f64,
}

impl Adaa {
    /// One sample through the curve `C`.
    #[inline(always)]
    pub fn step<C: Curve>(&mut self, u: f64) -> f64 {
        let fu = C::anti(u);
        let d = u - self.u1;
        let y = if d.abs() < ADAA_EPS {
            C::f(0.5 * u + 0.5 * self.u1)
        } else {
            (fu - self.f1) / d
        };
        self.u1 = u;
        self.f1 = fu;
        y
    }

    #[inline(always)]
    fn flush(&mut self) {
        self.u1 = flush64(self.u1);
        self.f1 = flush64(self.f1);
    }

    fn subnormals(&self) -> usize {
        usize::from(self.u1.is_subnormal()) + usize::from(self.f1.is_subnormal())
    }
}

/// The nonlinear stage of one voice: the ADAA curve inside the oversampler, or the converter's hold state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shaper {
    adaa: Adaa,
    os: Oversampler,
    /// The held value of `downsample`.
    hold: f64,
    /// The phase accumulator of `downsample`, in samples.
    acc: f64,
}

/// What the digital modes read per sample (interpolated across the block).
#[derive(Clone, Copy)]
struct Dig {
    level: f64,
    inv: f64,
    step: f64,
}

impl Shaper {
    /// A stage at rest.
    pub const ZERO: Self = Self {
        adaa: Adaa { u1: 0.0, f1: 0.0 },
        os: Oversampler::ZERO,
        hold: 0.0,
        acc: 0.0,
    };

    /// One base-rate sample through curve `C` at `K` times the rate (`K` = 1, 2 or 4).
    #[inline(always)]
    pub fn step<C: Curve, const K: usize>(&mut self, u: f64) -> f64 {
        let adaa = &mut self.adaa;
        self.os.process::<K>(u, &mut |s| adaa.step::<C>(s))
    }

    /// The steady state for a constant input `u0` in `mode`; returns the output there.
    fn prime(&mut self, mode: Mode, u0: f64, level: f64, inv: f64) -> f64 {
        match mode {
            Mode::Bitcrush => {
                let w = (u0.clamp(-1.0, 1.0) * level).round() * inv;
                self.hold = w;
                self.acc = 0.0;
                w
            }
            Mode::Downsample => {
                let w = u0.clamp(-1.0, 1.0);
                self.hold = w;
                self.acc = 0.0;
                w
            }
            _ => {
                let w = mode.transfer(u0);
                self.adaa = Adaa {
                    u1: u0,
                    f1: mode.anti(u0),
                };
                self.os.prime(u0, w);
                w
            }
        }
    }

    #[inline(always)]
    fn flush(&mut self) {
        self.adaa.flush();
        self.os.flush();
        self.hold = flush64(self.hold);
        self.acc = flush64(self.acc);
    }

    fn subnormals(&self) -> usize {
        self.adaa.subnormals()
            + self.os.subnormals()
            + usize::from(self.hold.is_subnormal())
            + usize::from(self.acc.is_subnormal())
    }
}

/// `bitcrush` for one sample.
#[inline(always)]
fn crush(_sh: &mut Shaper, u: f64, d: &Dig) -> f64 {
    (u.clamp(-1.0, 1.0) * d.level).round() * d.inv
}

/// `downsample` for one sample.
#[inline(always)]
fn hold(sh: &mut Shaper, u: f64, d: &Dig) -> f64 {
    sh.acc += d.step;
    if sh.acc >= 1.0 {
        sh.acc -= 1.0;
        sh.hold = u.clamp(-1.0, 1.0);
    }
    sh.hold
}

/// A clipping or folding curve for one sample.
#[inline(always)]
fn clip<C: Curve, const K: usize>(sh: &mut Shaper, u: f64, _d: &Dig) -> f64 {
    sh.step::<C, K>(u)
}

/// One second-order trapezoidal state-variable filter with `Q = 1/√2` (Simper's form): the building block of the
/// crossover.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Svf2 {
    ic1: f64,
    ic2: f64,
}

/// Coefficients of [`Svf2`] for one corner.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SvfCoefs {
    a1: f64,
    a2: f64,
    a3: f64,
}

impl SvfCoefs {
    fn new(hz: f64, sample_rate: f64) -> Self {
        let g = tan(PI * hz.min(0.45 * sample_rate) / sample_rate);
        let a1 = 1.0 / (1.0 + g * (g + SQRT_2));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2 }
    }
}

impl Svf2 {
    /// `(low-pass, high-pass)` of one sample.
    #[inline(always)]
    fn step(&mut self, c: &SvfCoefs, x: f64) -> (f64, f64) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, x - SQRT_2 * v1 - v2)
    }

    fn prime(&mut self, x: f64) {
        self.ic1 = 0.0;
        self.ic2 = x;
    }

    fn flush(&mut self) {
        self.ic1 = flush64(self.ic1);
        self.ic2 = flush64(self.ic2);
    }

    fn subnormals(&self) -> usize {
        usize::from(self.ic1.is_subnormal()) + usize::from(self.ic2.is_subnormal())
    }
}

/// A fourth-order Linkwitz-Riley split: `lo + hi` is the second-order Butterworth all-pass of the input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Crossover {
    first: Svf2,
    low: Svf2,
    high: Svf2,
}

impl Crossover {
    #[inline(always)]
    fn step(&mut self, c: &SvfCoefs, x: f64) -> (f64, f64) {
        let (lp1, hp1) = self.first.step(c, x);
        let (lo, _) = self.low.step(c, lp1);
        let (_, hi) = self.high.step(c, hp1);
        (lo, hi)
    }

    /// Steady state of a constant input: all of it in the low band.
    fn prime(&mut self, x: f64) {
        self.first.prime(x);
        self.low.prime(x);
        self.high.prime(0.0);
    }

    fn flush(&mut self) {
        self.first.flush();
        self.low.flush();
        self.high.flush();
    }

    fn subnormals(&self) -> usize {
        self.first.subnormals() + self.low.subnormals() + self.high.subnormals()
    }
}

/// A one-pole high-pass: `y[n] = x[n] − x[n−1] + R·y[n−1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct DcBlock {
    x1: f64,
    y1: f64,
}

impl DcBlock {
    #[inline(always)]
    fn step(&mut self, r: f64, x: f64) -> f64 {
        let y = x - self.x1 + r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    fn flush(&mut self) {
        self.x1 = flush64(self.x1);
        self.y1 = flush64(self.y1);
    }

    fn subnormals(&self) -> usize {
        usize::from(self.x1.is_subnormal()) + usize::from(self.y1.is_subnormal())
    }
}

/// A one-pole trapezoidal low-pass; `g` is `G = tan/(1 + tan)` of the corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct OnePole {
    s: f64,
}

impl OnePole {
    #[inline(always)]
    fn lp(&mut self, g: f64, x: f64) -> f64 {
        let v = g * (x - self.s);
        let y = v + self.s;
        self.s = y + v;
        y
    }

    fn flush(&mut self) {
        self.s = flush64(self.s);
    }

    fn subnormals(self) -> usize {
        usize::from(self.s.is_subnormal())
    }
}

/// What the block needs that does not change from segment to segment: mode, oversampling factor, crossover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    mode: Mode,
    factor: usize,
    split: bool,
    xo: SvfCoefs,
    pc: [f64; PHASE_SECTIONS],
    pc_n: usize,
    dc_r: f64,
}

impl Setup {
    /// For `mode`, oversampling `factor` (1, 2 or 4; anything else is 1), the `keep_lows` corner in Hz and the sample
    /// rate. The digital modes ignore `factor`.
    #[must_use]
    pub fn new(mode: Mode, factor: usize, keep_lows_hz: f32, sample_rate: f32) -> Self {
        let fs = f64::from(sample_rate);
        let factor = if mode.is_digital() {
            1
        } else {
            match factor {
                2 => 2,
                4 => 4,
                _ => 1,
            }
        };
        let (pc, pc_n) = if mode.is_digital() {
            ([0.0; PHASE_SECTIONS], 0)
        } else {
            phase_chain_coefs(factor)
        };
        Self {
            mode,
            factor,
            split: keep_lows_hz > KEEP_LOWS_OFF_HZ,
            xo: SvfCoefs::new(f64::from(keep_lows_hz), fs),
            pc,
            pc_n,
            dc_r: 1.0 - 2.0 * PI * DC_BLOCK_HZ / fs,
        }
    }

    /// What a voice's stages were primed for: a change of it re-primes the shaper and the phase chains.
    fn key(&self) -> (Mode, usize, bool) {
        (self.mode, self.factor, self.split)
    }

    /// The mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether the crossover is in the path.
    #[must_use]
    pub fn split(&self) -> bool {
        self.split
    }
}

/// The control values of one voice at one instant, as the block uses them (interpolated linearly across a block
/// from the previous block's to this one's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctl {
    gain: f64,
    bias: f64,
    level: f64,
    inv_level: f64,
    step: f64,
    tone_open: bool,
    tone_g: f64,
    tone_ga: f64,
    mix: f64,
}

impl Ctl {
    /// From the parameter values (`drive` in dB, `bias` in %, `bits`, `rate` and `tone` in Hz, `mix` in %) and the
    /// sample rate.
    #[must_use]
    pub fn new(
        drive_db: f32,
        bias_pct: f32,
        bits: f32,
        rate_hz: f32,
        tone_hz: f32,
        mix_pct: f32,
        sample_rate: f32,
    ) -> Self {
        let fs = f64::from(sample_rate);
        let level = exp2(f64::from(bits) - 1.0);
        let tone_open = tone_hz >= TONE_OPEN_HZ;
        let (tone_g, tone_ga) = if tone_open {
            (1.0, 1.0)
        } else {
            let g = tan(PI * f64::from(tone_hz).min(0.45 * fs) / fs);
            // The all-pass of the dry path has the corner doubled: equal group delay at DC.
            let ga = 2.0 * g;
            (g / (1.0 + g), ga / (1.0 + ga))
        };
        Self {
            gain: exp2(f64::from(drive_db) * DB_TO_LOG2),
            bias: f64::from(bias_pct) * 0.01,
            level,
            inv_level: 1.0 / level,
            step: (f64::from(rate_hz) / fs).min(1.0),
            tone_open,
            tone_g,
            tone_ga,
            mix: f64::from(mix_pct) * 0.01,
        }
    }

    /// The mix as a fraction.
    #[must_use]
    pub fn mix(&self) -> f64 {
        self.mix
    }
}

#[inline(always)]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// One voice of `dist`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    xo: Crossover,
    sh: Shaper,
    dc: DcBlock,
    pc_lo: PhaseChain,
    pc_dry: PhaseChain,
    tone: OnePole,
    tone_ap: OnePole,
    /// How much of the processed blend is heard (0 = the bus untouched).
    fade: f64,
    /// Nothing is running: the next block that needs the voice primes its states.
    cold: bool,
    /// The voice has not yet run a block since it started.
    fresh: bool,
    /// What the shaper and phase chains were primed for.
    key: (Mode, usize, bool),
}

impl Default for Voice {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Voice {
    /// A voice that has not run.
    pub const ZERO: Self = Self {
        xo: Crossover {
            first: Svf2 { ic1: 0.0, ic2: 0.0 },
            low: Svf2 { ic1: 0.0, ic2: 0.0 },
            high: Svf2 { ic1: 0.0, ic2: 0.0 },
        },
        sh: Shaper::ZERO,
        dc: DcBlock { x1: 0.0, y1: 0.0 },
        pc_lo: PhaseChain::ZERO,
        pc_dry: PhaseChain::ZERO,
        tone: OnePole { s: 0.0 },
        tone_ap: OnePole { s: 0.0 },
        fade: 0.0,
        cold: true,
        fresh: true,
        key: (Mode::Soft, 1, false),
    };

    /// A fresh voice starts: forget everything.
    pub fn reset(&mut self) {
        *self = Self::ZERO;
    }

    /// Prime the stages that depend on the structure (shaper, crossover, phase chains) for a constant input `x0`.
    /// Returns the shaper's output there.
    fn prime_structure(&mut self, su: &Setup, a: &Ctl, x0: f64) -> f64 {
        let (lo0, hi0) = if su.split { (x0, 0.0) } else { (0.0, x0) };
        if su.split {
            self.xo.prime(x0);
        }
        self.pc_lo.prime(lo0);
        self.pc_dry.prime(x0);
        self.key = su.key();
        let u0 = a.gain * hi0 + a.bias;
        self.sh.prime(su.mode, u0, a.level, a.inv_level)
    }

    /// Render one block in place. `a` holds the controls at the end of the previous block (the same as `b` for a
    /// fresh voice), `b` at the end of this one.
    pub fn process(&mut self, su: &Setup, a: &Ctl, b: &Ctl, io: &mut [f32]) {
        if io.is_empty() {
            return;
        }
        if a.mix == 0.0 && b.mix == 0.0 && self.fade == 0.0 {
            self.cold = true;
            self.fresh = false;
            return;
        }
        let x0 = f64::from(io[0]);
        if self.cold {
            let w0 = self.prime_structure(su, a, x0);
            let lo0 = if su.split { x0 } else { 0.0 };
            self.dc = DcBlock { x1: w0, y1: 0.0 };
            self.tone = OnePole { s: lo0 };
            self.tone_ap = OnePole { s: x0 };
            if self.fresh {
                self.fade = 1.0;
            }
            self.cold = false;
        } else if self.key != su.key() {
            self.prime_structure(su, a, x0);
        }
        self.fresh = false;
        let c0 = self.fade;
        let c1 = if b.mix > 0.0 {
            (c0 + FADE_STEP).min(1.0)
        } else {
            (c0 - FADE_STEP).max(0.0)
        };
        self.fade = c1;
        match (su.mode, su.factor) {
            (Mode::Soft, 1) => run(self, su, a, b, (c0, c1), io, clip::<SoftCurve, 1>),
            (Mode::Soft, 2) => run(self, su, a, b, (c0, c1), io, clip::<SoftCurve, 2>),
            (Mode::Soft, _) => run(self, su, a, b, (c0, c1), io, clip::<SoftCurve, 4>),
            (Mode::Hard, 1) => run(self, su, a, b, (c0, c1), io, clip::<HardCurve, 1>),
            (Mode::Hard, 2) => run(self, su, a, b, (c0, c1), io, clip::<HardCurve, 2>),
            (Mode::Hard, _) => run(self, su, a, b, (c0, c1), io, clip::<HardCurve, 4>),
            (Mode::Asymmetric, 1) => run(self, su, a, b, (c0, c1), io, clip::<AsymCurve, 1>),
            (Mode::Asymmetric, 2) => run(self, su, a, b, (c0, c1), io, clip::<AsymCurve, 2>),
            (Mode::Asymmetric, _) => run(self, su, a, b, (c0, c1), io, clip::<AsymCurve, 4>),
            (Mode::SineFold, 1) => run(self, su, a, b, (c0, c1), io, clip::<SineCurve, 1>),
            (Mode::SineFold, 2) => run(self, su, a, b, (c0, c1), io, clip::<SineCurve, 2>),
            (Mode::SineFold, _) => run(self, su, a, b, (c0, c1), io, clip::<SineCurve, 4>),
            (Mode::TriangleFold, 1) => run(self, su, a, b, (c0, c1), io, clip::<TriCurve, 1>),
            (Mode::TriangleFold, 2) => run(self, su, a, b, (c0, c1), io, clip::<TriCurve, 2>),
            (Mode::TriangleFold, _) => run(self, su, a, b, (c0, c1), io, clip::<TriCurve, 4>),
            (Mode::Bitcrush, _) => run(self, su, a, b, (c0, c1), io, crush),
            (Mode::Downsample, _) => run(self, su, a, b, (c0, c1), io, hold),
        }
        self.flush();
    }

    fn flush(&mut self) {
        self.xo.flush();
        self.sh.flush();
        self.dc.flush();
        self.pc_lo.flush();
        self.pc_dry.flush();
        self.tone.flush();
        self.tone_ap.flush();
    }

    /// How many state values are subnormal (test support for the release-tail invariant).
    #[must_use]
    pub fn subnormals(&self) -> usize {
        self.xo.subnormals()
            + self.sh.subnormals()
            + self.dc.subnormals()
            + self.pc_lo.subnormals()
            + self.pc_dry.subnormals()
            + self.tone.subnormals()
            + self.tone_ap.subnormals()
            + usize::from(self.fade.is_subnormal())
    }
}

/// The sample loop, generic over the shaper so that each mode and factor compiles to straight code. `fade` is the
/// blend with the raw input at the start and end of the block (1, 1 in steady state).
#[inline(always)]
fn run<F>(v: &mut Voice, su: &Setup, a: &Ctl, b: &Ctl, fade: (f64, f64), io: &mut [f32], shape: F)
where
    F: Fn(&mut Shaper, f64, &Dig) -> f64,
{
    let inv_n = 1.0 / io.len() as f64;
    let open = a.tone_open && b.tone_open;
    let full = fade.0 == 1.0 && fade.1 == 1.0;
    for (i, slot) in io.iter_mut().enumerate() {
        let t = (i + 1) as f64 * inv_n;
        let x = f64::from(*slot);
        let (lo, hi) = if su.split {
            v.xo.step(&su.xo, x)
        } else {
            (0.0, x)
        };
        let u = lerp(a.gain, b.gain, t) * hi + lerp(a.bias, b.bias, t);
        let dig = Dig {
            level: lerp(a.level, b.level, t),
            inv: lerp(a.inv_level, b.inv_level, t),
            step: lerp(a.step, b.step, t),
        };
        let w = shape(&mut v.sh, u, &dig);
        let mut wet = v.dc.step(su.dc_r, w);
        let d0 = if su.split {
            wet += v.pc_lo.step(&su.pc, su.pc_n, lo);
            lo + hi
        } else {
            x
        };
        let mut dry = v.pc_dry.step(&su.pc, su.pc_n, d0);
        if open {
            v.tone.s = wet;
            v.tone_ap.s = dry;
        } else {
            wet = v.tone.lp(lerp(a.tone_g, b.tone_g, t), wet);
            dry = 2.0 * v.tone_ap.lp(lerp(a.tone_ga, b.tone_ga, t), dry) - dry;
        }
        let m = lerp(a.mix, b.mix, t);
        let blend = (1.0 - m) * dry + m * wet;
        let out = if full {
            blend
        } else {
            x + lerp(fade.0, fade.1, t) * (blend - x)
        };
        *slot = out as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libm::{atan2, hypot, log10};

    const SR: f32 = 48_000.0;

    /// Every curve against a high-precision reference formula and its antiderivative against a central difference.
    #[test]
    fn antiderivatives_differentiate_to_the_curves() {
        fn check<C: Curve>(name: &str) {
            let h = 1.0e-6;
            for i in -300..=300 {
                let u = f64::from(i) * 0.0137 + 0.003;
                let d = (C::anti(u + h) - C::anti(u - h)) / (2.0 * h);
                // Skip the corners of the piecewise-linear curves, where a central difference straddles a kink.
                if (C::f(u + h) - C::f(u - h)).abs() > 1.0e-4 {
                    continue;
                }
                assert!(
                    (d - C::f(u)).abs() < 1.0e-6,
                    "{name} at {u}: {d} vs {}",
                    C::f(u)
                );
            }
        }
        check::<SoftCurve>("soft");
        check::<HardCurve>("hard");
        check::<AsymCurve>("asymmetric");
        check::<SineCurve>("sine_fold");
        check::<TriCurve>("triangle_fold");
    }

    #[test]
    fn the_curves_match_their_formulas_and_have_unit_slope_and_peak() {
        for i in -2000..=2000 {
            let u = f64::from(i) * 0.005;
            assert!((SoftCurve::f(u) - libm::tanh(u)).abs() < 1e-15);
            assert_eq!(HardCurve::f(u), u.clamp(-1.0, 1.0));
            let asym = if u >= 0.0 {
                libm::tanh(u)
            } else {
                0.5 * libm::tanh(2.0 * u)
            };
            assert!((AsymCurve::f(u) - asym).abs() < 1e-15);
            assert!((SineCurve::f(u) - libm::sin(u)).abs() < 1e-15);
            // Triangle against the explicit period-4 definition.
            let w = (u + 1.0).rem_euclid(4.0);
            let tri = if w < 2.0 { w - 1.0 } else { 3.0 - w };
            assert!((TriCurve::f(u) - tri).abs() < 1e-12, "tri at {u}");
        }
        // Slope 1 at 0 everywhere (one-sided for the asymmetric curve), peak 1.
        for mode in [
            Mode::Soft,
            Mode::Hard,
            Mode::Asymmetric,
            Mode::SineFold,
            Mode::TriangleFold,
        ] {
            let e = 1.0e-7;
            assert!((mode.transfer(e) - e).abs() < 1e-12, "{mode:?}");
            assert!((mode.transfer(-e) + e).abs() < 1e-12, "{mode:?}");
        }
    }

    #[test]
    fn folds_fold_at_their_thresholds() {
        // Sine: rises to 1 at pi/2, falls to -1 at 3pi/2, and so on.
        assert!((SineCurve::f(PI * 0.5) - 1.0).abs() < 1e-15);
        assert!(SineCurve::f(PI * 0.5 + 0.2) < SineCurve::f(PI * 0.5));
        assert!((SineCurve::f(PI * 1.5) + 1.0).abs() < 1e-15);
        // Triangle: unit slope up to 1, then back down to -1 at 3, up to 1 at 5.
        assert_eq!(TriCurve::f(1.0), 1.0);
        assert_eq!(TriCurve::f(0.5), 0.5);
        assert_eq!(TriCurve::f(1.5), 0.5);
        assert_eq!(TriCurve::f(2.0), 0.0);
        assert_eq!(TriCurve::f(3.0), -1.0);
        assert_eq!(TriCurve::f(5.0), 1.0);
        assert_eq!(TriCurve::f(-1.0), -1.0);
        assert_eq!(TriCurve::f(-3.0), 1.0);
        // Every fold stays within the unit range at any input.
        for i in -5000..=5000 {
            let u = f64::from(i) * 0.37;
            assert!(TriCurve::f(u).abs() <= 1.0 + 1e-12 && SineCurve::f(u).abs() <= 1.0);
        }
    }

    /// The static curve: a constant input to the ADAA stage inside the oversampler returns the curve exactly (the
    /// oversampler's DC gain is 1), within 1e-6 over a sweep, at 1x, 2x and 4x.
    #[test]
    fn a_constant_input_returns_the_static_curve() {
        fn sweep<C: Curve, const K: usize>(mode: Mode) {
            let mut worst = 0f64;
            for i in -120..=120 {
                let u = f64::from(i) * 0.05;
                let mut sh = Shaper::ZERO;
                let mut y = 0.0;
                for _ in 0..400 {
                    y = sh.step::<C, K>(u);
                }
                worst = worst.max((y - mode.transfer(u)).abs());
            }
            assert!(worst < 1e-6, "{mode:?} x{K}: {worst}");
        }
        sweep::<SoftCurve, 1>(Mode::Soft);
        sweep::<SoftCurve, 2>(Mode::Soft);
        sweep::<SoftCurve, 4>(Mode::Soft);
        sweep::<HardCurve, 1>(Mode::Hard);
        sweep::<HardCurve, 2>(Mode::Hard);
        sweep::<HardCurve, 4>(Mode::Hard);
        sweep::<AsymCurve, 1>(Mode::Asymmetric);
        sweep::<AsymCurve, 2>(Mode::Asymmetric);
        sweep::<AsymCurve, 4>(Mode::Asymmetric);
        sweep::<SineCurve, 1>(Mode::SineFold);
        sweep::<SineCurve, 2>(Mode::SineFold);
        sweep::<SineCurve, 4>(Mode::SineFold);
        sweep::<TriCurve, 1>(Mode::TriangleFold);
        sweep::<TriCurve, 2>(Mode::TriangleFold);
        sweep::<TriCurve, 4>(Mode::TriangleFold);
    }

    #[test]
    fn adaa_of_a_straight_line_is_the_two_sample_average() {
        // Inside the clamp's linear stretch the antiderivative quotient is exactly (u + u1)/2.
        let mut st = Adaa::default();
        let mut prev = 0.0;
        for i in 0..50 {
            let u = 0.01 * f64::from(i % 7) - 0.03;
            let y = st.step::<HardCurve>(u);
            assert!((y - (0.5 * u + 0.5 * prev)).abs() < 1e-9, "{i}: {y}");
            prev = u;
        }
    }

    #[test]
    fn adaa_matches_the_midpoint_across_the_fallback_threshold() {
        // The quotient and the midpoint rule agree to far better than the threshold's own error either side of it.
        for &(u, d) in &[
            (0.4, 2.0e-5),
            (0.4, 0.5e-5),
            (-1.3, 2.0e-5),
            (3.0, 0.5e-5),
            (0.0, 2.0e-5),
        ] {
            let mut a = Adaa::default();
            let _ = a.step::<SoftCurve>(u);
            let y = a.step::<SoftCurve>(u + d);
            let mid = SoftCurve::f(u + 0.5 * d);
            assert!((y - mid).abs() < 1e-8, "{u} {d}: {y} vs {mid}");
        }
    }

    fn ctl(drive: f32, bias: f32, bits: f32, rate: f32, tone: f32, mix: f32) -> Ctl {
        Ctl::new(drive, bias, bits, rate, tone, mix, SR)
    }

    /// The defaults of the catalogue (soft, 6 dB, tone open, mix 100).
    fn default_ctl() -> Ctl {
        ctl(6.0, 0.0, 8.0, 6_000.0, 20_000.0, 100.0)
    }

    /// Render `n` samples of `f(i)` through a fresh voice with fixed controls, in blocks of 32.
    fn render(su: &Setup, c: &Ctl, n: usize, f: impl Fn(usize) -> f64) -> Vec<f32> {
        let mut v = Voice::ZERO;
        let mut out = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            let mut block = [0f32; 32];
            for (k, s) in block.iter_mut().enumerate() {
                *s = f(i + k) as f32;
            }
            v.process(su, c, c, &mut block);
            out.extend_from_slice(&block);
            i += 32;
        }
        out
    }

    /// Amplitude and phase of the sinusoid of `hz` in `y[from..]`, by projecting on sin and cos over whole periods.
    fn tone_of(y: &[f32], hz: f64, from: usize) -> (f64, f64) {
        let w = 2.0 * PI * hz / f64::from(SR);
        let (mut re, mut im) = (0.0, 0.0);
        // Whole periods only (a window of 24000 samples holds an integer number of cycles of any multiple of 2 Hz).
        let cycles = ((y.len() - from) as f64 * hz / f64::from(SR) + 1.0e-6).floor();
        let n = (cycles * f64::from(SR) / hz).round() as usize;
        for (k, &s) in y[from..from + n].iter().enumerate() {
            let a = w * (from + k) as f64;
            re += f64::from(s) * cos(a);
            im += f64::from(s) * sin(a);
        }
        (2.0 * hypot(re, im) / n as f64, atan2(im, re))
    }

    fn db(x: f64) -> f64 {
        20.0 * log10(x)
    }

    /// The open decision of the design: ADAA stays on at 2x and 4x. A sine at 16x drive into each curve, aliasing
    /// measured with the harness, with and without the antiderivative quotient inside the oversampler (the plain
    /// curve at the high rate): the quotient lowers aliasing by 10 to 23 dB for the clips and folds at every factor
    /// (`docs/dsp/dist.md` has the table).
    #[test]
    fn adaa_lowers_aliasing_inside_the_oversampler_too() {
        use crate::alias::{Measure, alias_db};
        fn run<C: Curve, const K: usize>(adaa: bool, hz: f64, gain: f64) -> f64 {
            let w = 2.0 * PI * hz / f64::from(SR);
            let mut sh = Shaper::ZERO;
            let n: usize = 32_768 + 4_096;
            let y: Vec<f32> = (0..n)
                .map(|i| {
                    let u = gain * sin(w * i as f64);
                    let o = if adaa {
                        sh.step::<C, K>(u)
                    } else {
                        sh.os.process::<K>(u, &mut |s| C::f(s))
                    };
                    o as f32
                })
                .collect();
            alias_db(&y[4_096..], hz, f64::from(SR), &Measure::REPORT_02)
        }
        fn gain_db<C: Curve, const K: usize>(hz: f64) -> f64 {
            run::<C, K>(false, hz, 16.0) - run::<C, K>(true, hz, 16.0)
        }
        for hz in [1_760.0, 3_520.0] {
            for (name, g1, g2, g4) in [
                (
                    "soft",
                    gain_db::<SoftCurve, 1>(hz),
                    gain_db::<SoftCurve, 2>(hz),
                    99.0,
                ),
                (
                    "hard",
                    gain_db::<HardCurve, 1>(hz),
                    gain_db::<HardCurve, 2>(hz),
                    gain_db::<HardCurve, 4>(hz),
                ),
                (
                    "tri",
                    gain_db::<TriCurve, 1>(hz),
                    gain_db::<TriCurve, 2>(hz),
                    gain_db::<TriCurve, 4>(hz),
                ),
            ] {
                for (k, g) in [(1, g1), (2, g2), (4, g4)] {
                    assert!(g > 8.0, "{name} {hz} Hz x{k}: ADAA gains only {g:.1} dB");
                }
            }
        }
    }

    #[test]
    fn mix_zero_is_the_input_bit_for_bit_and_costs_nothing() {
        for mode in [Mode::Soft, Mode::Bitcrush, Mode::TriangleFold] {
            for factor in [1, 2, 4] {
                let su = Setup::new(mode, factor, 200.0, SR);
                let c = ctl(24.0, 30.0, 4.0, 3_000.0, 3_000.0, 0.0);
                let input = |i: usize| 0.8 * sin(0.37 * i as f64) + 0.1 * sin(1.7 * i as f64);
                let out = render(&su, &c, 4096, input);
                for (i, &y) in out.iter().enumerate() {
                    assert_eq!(
                        (input(i) as f32).to_bits(),
                        y.to_bits(),
                        "{mode:?} {factor}x at {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn mix_half_is_flat_below_the_knee() {
        // Drive 0 dB and a level far below the knee: the wet path is linear, the dry path matches its phase, so the
        // blend is flat. Open tone, no split, at x2 and x4 (the default is x2).
        for factor in [2, 4] {
            let su = Setup::new(Mode::Soft, factor, 20.0, SR);
            let c = ctl(0.0, 0.0, 8.0, 6_000.0, 20_000.0, 50.0);
            for hz in [
                50.0, 100.0, 300.0, 1_000.0, 3_000.0, 6_000.0, 8_000.0, 10_000.0,
            ] {
                let w = 2.0 * PI * hz / f64::from(SR);
                let y = render(&su, &c, 24_000, |i| 0.01 * sin(w * i as f64));
                let (amp, _) = tone_of(&y, hz, 12_000);
                let g = db(amp / 0.01);
                assert!(g.abs() < 0.5, "{factor}x {hz} Hz: {g} dB");
            }
        }
    }

    #[test]
    fn tone_follows_its_one_pole_response() {
        let su = Setup::new(Mode::Soft, 2, 20.0, SR);
        for tone in [500.0f32, 2_000.0, 4_000.0] {
            let c = ctl(0.0, 0.0, 8.0, 6_000.0, tone, 100.0);
            let g = tan(PI * f64::from(tone) / f64::from(SR));
            for hz in [
                f64::from(tone) * 0.25,
                f64::from(tone),
                f64::from(tone) * 2.0,
            ] {
                let w = 2.0 * PI * hz / f64::from(SR);
                let y = render(&su, &c, 24_000, |i| 0.01 * sin(w * i as f64));
                let (amp, _) = tone_of(&y, hz, 12_000);
                let r = tan(w * 0.5) / g;
                let want = 1.0 / (1.0 + r * r).sqrt();
                let err = db(amp / 0.01) - db(want);
                assert!(
                    err.abs() < 0.5,
                    "tone {tone} at {hz}: {err} dB (measured {})",
                    db(amp / 0.01)
                );
            }
        }
    }

    #[test]
    fn the_crossover_is_lr4_and_sums_to_an_all_pass() {
        let c = SvfCoefs::new(500.0, f64::from(SR));
        let g = tan(PI * 500.0 / f64::from(SR));
        for hz in [20.0, 100.0, 250.0, 500.0, 1_000.0, 4_000.0] {
            let w = 2.0 * PI * hz / f64::from(SR);
            let mut xo = Crossover::default();
            let (mut lo_e, mut hi_e, mut sum_e, mut in_e) = (0.0, 0.0, 0.0, 0.0);
            for i in 0..48_000 {
                let x = sin(w * f64::from(i));
                let (lo, hi) = xo.step(&c, x);
                if i >= 24_000 {
                    lo_e += lo * lo;
                    hi_e += hi * hi;
                    sum_e += (lo + hi) * (lo + hi);
                    in_e += x * x;
                }
            }
            let r = tan(w * 0.5) / g;
            let r4 = r * r * r * r;
            let lo = (lo_e / in_e).sqrt();
            let hi = (hi_e / in_e).sqrt();
            assert!(
                (db(lo) - db(1.0 / (1.0 + r4))).abs() < 0.01,
                "{hz} lo {}",
                db(lo)
            );
            assert!(
                (db(hi) - db(r4 / (1.0 + r4))).abs() < 0.01,
                "{hz} hi {}",
                db(hi)
            );
            assert!(db((sum_e / in_e).sqrt()).abs() < 0.01, "{hz} sum");
        }
    }

    #[test]
    fn keep_lows_passes_the_low_band_and_distorts_the_rest() {
        // A loud low sine through heavy hard clipping is untouched when it lies far below the crossover, and
        // clipped when there is no split.
        let hz = 40.0;
        let w = 2.0 * PI * hz / f64::from(SR);
        let c = ctl(24.0, 0.0, 8.0, 6_000.0, 20_000.0, 100.0);
        let split = Setup::new(Mode::Hard, 2, 400.0, SR);
        let y = render(&split, &c, 48_000, |i| 0.5 * sin(w * i as f64));
        let (amp, _) = tone_of(&y, hz, 24_000);
        // lo = 1/(1 + r^4) with r about 0.1: 0.9999; the high band leaks 1e-4 x 16 into a clamp that is not reached.
        assert!(db(amp / 0.5).abs() < 0.1, "{} dB", db(amp / 0.5));
        let none = Setup::new(Mode::Hard, 2, 20.0, SR);
        let y = render(&none, &c, 48_000, |i| 0.5 * sin(w * i as f64));
        let (amp, _) = tone_of(&y, hz, 24_000);
        assert!(
            amp > 1.1 * 0.5,
            "a hard clip at 24 dB squares the wave: {amp}"
        );
    }

    #[test]
    fn bias_makes_even_harmonics() {
        let hz = 440.0;
        let w = 2.0 * PI * hz / f64::from(SR);
        let su = Setup::new(Mode::Soft, 2, 20.0, SR);
        for (bias, even_present) in [(0.0f32, false), (35.0, true)] {
            let c = ctl(12.0, bias, 8.0, 6_000.0, 20_000.0, 100.0);
            let y = render(&su, &c, 48_000, |i| 0.6 * sin(w * i as f64));
            let (h1, _) = tone_of(&y, hz, 24_000);
            let (h2, _) = tone_of(&y, 2.0 * hz, 24_000);
            let (h3, _) = tone_of(&y, 3.0 * hz, 24_000);
            assert!(h3 > 1e-3 * h1, "odd harmonics are there");
            if even_present {
                assert!(h2 > 0.01 * h1, "bias {bias}: second {} dB", db(h2 / h1));
            } else {
                assert!(h2 < 1e-6 * h1, "no bias: second {} dB", db(h2 / h1));
            }
        }
    }

    #[test]
    fn bitcrush_quantises_and_downsample_holds() {
        let su = Setup::new(Mode::Bitcrush, 2, 20.0, SR);
        let c = ctl(0.0, 0.0, 3.0, 6_000.0, 20_000.0, 100.0);
        let y = render(&su, &c, 4096, |i| 0.9 * sin(0.05 * i as f64));
        // 3 bits: 4 steps per side, so the values are multiples of 1/4 (after the DC blocker, which is 0 on average
        // for a symmetric wave; compare the shaper directly instead).
        let mut sh = Shaper::ZERO;
        let d = Dig {
            level: 4.0,
            inv: 0.25,
            step: 1.0,
        };
        for i in -40..=40 {
            let u = f64::from(i) * 0.03;
            let q = crush(&mut sh, u, &d);
            assert!(
                (q * 4.0 - (q * 4.0).round()).abs() < 1e-12 && q.abs() <= 1.0,
                "{u} {q}"
            );
        }
        assert!(y.iter().all(|s| s.abs() <= 1.2));
        // Downsample at a quarter of the rate holds four samples.
        let mut sh = Shaper::ZERO;
        let d = Dig {
            level: 1.0,
            inv: 1.0,
            step: 0.25,
        };
        let held: Vec<f64> = (0..16)
            .map(|i| hold(&mut sh, f64::from(i) * 0.05, &d))
            .collect();
        for chunk in held[3..15].chunks(4) {
            assert!(chunk.windows(2).all(|w| w[0] == w[1]), "{chunk:?}");
        }
        // The top of the rate range is the identity.
        let mut sh = Shaper::ZERO;
        let d = Dig {
            level: 1.0,
            inv: 1.0,
            step: 1.0,
        };
        for i in 0..40 {
            let u = f64::from(i % 9) * 0.1 - 0.4;
            assert_eq!(hold(&mut sh, u, &d), u);
        }
    }

    #[test]
    fn instances_keep_separate_state() {
        let su = Setup::new(Mode::Asymmetric, 4, 100.0, SR);
        let c = default_ctl();
        let mut a = Voice::ZERO;
        let mut b = Voice::ZERO;
        let mut quiet = [0.0f32; 32];
        let mut loud = [0.0f32; 32];
        for (k, s) in loud.iter_mut().enumerate() {
            *s = (0.9 * sin(0.2 * k as f64)) as f32;
        }
        a.process(&su, &c, &c, &mut loud);
        b.process(&su, &c, &c, &mut quiet);
        assert!(quiet.iter().all(|&s| s == 0.0));
        assert_ne!(a, b);
    }

    #[test]
    fn a_long_tail_has_no_subnormals() {
        let su = Setup::new(Mode::Soft, 4, 300.0, SR);
        let c = ctl(12.0, 20.0, 8.0, 6_000.0, 4_000.0, 70.0);
        let mut v = Voice::ZERO;
        let mut block = [0f32; 32];
        for blk in 0..40 {
            for (k, s) in block.iter_mut().enumerate() {
                *s = (0.8 * sin(0.1 * (blk * 32 + k) as f64)) as f32;
            }
            v.process(&su, &c, &c, &mut block);
        }
        for _ in 0..(40 * 48_000 / 32) {
            block = [0.0; 32];
            v.process(&su, &c, &c, &mut block);
            assert_eq!(v.subnormals(), 0);
        }
    }

    #[test]
    fn a_fresh_voice_has_no_start_transient_from_a_constant_input() {
        // Priming puts every filter in its steady state: a constant input (as an oscillator's first samples can be)
        // produces no step response through the DC blocker, the tone filter or the oversampler.
        let su = Setup::new(Mode::Soft, 2, 200.0, SR);
        let c = ctl(6.0, 25.0, 8.0, 6_000.0, 3_000.0, 100.0);
        let y = render(&su, &c, 640, |_| 0.3);
        // After priming the wet path is the clean low band (the crossover is on): its steady value is the input.
        for (i, &s) in y.iter().enumerate() {
            assert!((f64::from(s) - 0.3).abs() < 1e-6, "{i}: {s}");
        }
    }

    #[test]
    fn the_mix_fades_in_when_it_leaves_zero() {
        let su = Setup::new(Mode::Hard, 2, 20.0, SR);
        let off = ctl(12.0, 0.0, 8.0, 6_000.0, 20_000.0, 0.0);
        let on = ctl(12.0, 0.0, 8.0, 6_000.0, 20_000.0, 100.0);
        let mut v = Voice::ZERO;
        let x = |i: usize| (0.7 * sin(0.3 * i as f64)) as f32;
        let seg = |v: &mut Voice, base: usize, a: &Ctl, b: &Ctl| {
            let mut blk = [0f32; 32];
            for (k, s) in blk.iter_mut().enumerate() {
                *s = x(base + k);
            }
            v.process(&su, a, b, &mut blk);
            blk
        };
        // Bypassed: exact.
        let blk = seg(&mut v, 0, &off, &off);
        assert!(
            blk.iter()
                .enumerate()
                .all(|(k, &s)| s.to_bits() == x(k).to_bits())
        );
        // Leaving 0: the first output sample is still the raw input to within the first fade step.
        let blk = seg(&mut v, 32, &off, &on);
        assert!((f64::from(blk[0]) - f64::from(x(32))).abs() < 0.2);
        // After 8 more blocks the fade is complete.
        for k in 2..12 {
            let _ = seg(&mut v, 32 * k, &on, &on);
        }
        assert_eq!(v.fade, 1.0);
        // And going back to 0 fades out and then bypasses exactly again.
        for k in 12..30 {
            let _ = seg(&mut v, 32 * k, &on, &off);
        }
        let blk = seg(&mut v, 32 * 31, &off, &off);
        assert!(
            blk.iter()
                .enumerate()
                .all(|(k, &s)| s.to_bits() == x(32 * 31 + k).to_bits())
        );
    }
}
