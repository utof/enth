//! The modulated-delay core of S2a lane MOD: the stereo chorus, ensemble and flanger behind the `chorus` module, and
//! the sweep helpers the `phaser` shares. Notes: `docs/dsp/chorus.md`.
//!
//! Written from the equations of Julius O. Smith III's *Physical Audio Signal Processing* ("Flanging", "Chorus" and
//! "Fractional delay" in the *Delay Effects* chapter), from the F1m catalogue table (`enth-patch`'s `chorus.rs`) and
//! from the structure of the Plaits and Rings ensemble the table names (three taps 120° apart on a slow and a fast
//! sweep). No code is ported.
//!
//! ```text
//! tap(t)  = delay · (1 + depth · s(t))                       s(t) the sweep, −1 … 1
//! v[n]    = (1 − |fb|) · x[n] + fb · wet[n]                  wet[n] = mean of the taps read from v at lag tap(n)
//! y[n]    = (x[n] + wet[n]) / 2                              mix 100 %
//! out     = (1 − mix) · x + mix · y
//! ```
//!
//! - **Zero latency.** The dry signal is never delayed: `y[n]` uses `x[n]` directly. The wet tap's delay is clamped to
//!   at least one sample, so a flanger reaches its zero-delay null (the first comb notch at Nyquist, where the comb
//!   leaves the audible band) without delaying the output; a true through-zero flanger would need the dry path
//!   delayed, which is latency (D10, ADR-0006) and is not a mode.
//! - **The loop is the F1m one, exactly, down to one sample.** A four-point Lagrange read of the tap `τ ≥ 1` reaches
//!   the sample being written (lag 0) when `⌊τ⌋ = 1`. That term is solved, not delayed: with `c0` the lag-0 weight,
//!   `v = ((1 − |fb|)·x + fb·r) / (1 − fb·c0)` and `wet = c0·v + r`, `r` the sum of the lags ≥ 1. `|fb·c0| ≤ 0.064`,
//!   so the divisor stays within 0.93 … 1.07.
//! - **The level is bounded.** Lagrange interpolation has magnitude at most 1 at every fraction (tested), so
//!   `|wet/x| = (1 − |fb|)·|K| / |1 − fb·K| ≤ 1` (`K` the tap's kernel): the loop peaks at 0 dB at every feedback, where
//!   the bare `x + fb·loop` peaks at `1/(1 − |fb|)`. The sum with the dry signal is `|1 + wet/x|/2 ≤ 1`.
//! - **The sweep** is a 64-bit phase accumulator advanced by an integer increment per block, so a synced sweep is
//!   exact in beats ([`lfo::synced_increment`](crate::lfo::synced_increment)). The sample inside the block is
//!   [`block_phase`], exact and landing on the next block's start. The right channel runs [`lag_for_width`] behind
//!   the left. Chorus is a sine, flanger a triangle (the tape-flanger sweep), ensemble three taps at 0°, 120° and 240°
//!   on `0.9·sine(slow) + 0.1·sine(8 × slow)`.
//! - **Determinism.** Per sample: `+ − × ÷` and integer index maths; the sine is the project's polynomial
//!   ([`osc::sine`](crate::osc::sine)). No `mul_add`. Every write into the line is flushed.

#![allow(
    clippy::manual_midpoint,
    reason = "0.5·(a + b) is the textbook split; f32::midpoint is wider than it needs to be and its code differs by target"
)]

use crate::osc::sine;
use crate::util::flush::flush;

/// The catalogue's longest `delay`, in ms.
pub const MAX_DELAY_MS: f32 = 30.0;
/// The longest a tap can be: twice the longest `delay` (a depth of 100 % doubles it), in ms.
pub const MAX_TAP_MS: f32 = 60.0;
/// The shortest tap, in samples: the zero-delay point of the sweep.
pub const MIN_DELAY: f32 = 1.0;
/// The largest feedback magnitude (the catalogue's 99 %).
pub const MAX_FEEDBACK: f32 = 0.99;
/// How much faster the ensemble's fast sweep runs than its slow one.
pub const FAST_RATIO: u64 = 8;
/// The fast sweep's share of the ensemble's sweep (the slow one has `1 − FAST_SHARE`).
pub const FAST_SHARE: f32 = 0.1;
/// 120° as a phase offset: `2⁶⁴ / 3`, rounded.
pub const THIRD_TURN: u64 = 0x5555_5555_5555_5555;
/// Taps held back from the end of the line: the interpolator reads lags up to `⌊τ⌋ + 2`.
const GUARD: usize = 4;

/// The longest tap in samples at `sample_rate`.
#[must_use]
pub fn max_tap_samples(sample_rate: f32) -> f32 {
    MAX_TAP_MS * 0.001 * sample_rate
}

/// The length of one channel's delay line at `sample_rate`: a power of two holding the longest tap and the
/// interpolator's reach, so reads wrap with a mask. Sized from the sample rate alone, never from parameter values.
#[must_use]
pub fn line_len(sample_rate: f32) -> usize {
    (max_tap_samples(sample_rate).ceil() as usize + GUARD + 2).next_power_of_two()
}

/// The algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// One sine-swept tap per channel.
    Chorus,
    /// Three taps per channel on a slow and a fast sweep.
    Ensemble,
    /// One triangle-swept tap per channel.
    Flanger,
}

impl Mode {
    /// The taps per channel.
    #[must_use]
    pub const fn taps(self) -> usize {
        match self {
            Mode::Chorus | Mode::Flanger => 1,
            Mode::Ensemble => 3,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The sweep (shared with the phaser)
// ---------------------------------------------------------------------------------------------------------------

/// The cycle position `[0, 1)` of a 64-bit phase: its top 24 bits, exact in `f32`.
#[inline(always)]
#[must_use]
pub fn unit(phase: u64) -> f32 {
    (phase >> 40) as u32 as f32 * (1.0 / 16_777_216.0)
}

/// A sine sweep of a phase, `−1 … 1`, starting at 0 and rising.
#[inline(always)]
#[must_use]
pub fn sine_of(phase: u64) -> f32 {
    sine(unit(phase))
}

/// A triangle sweep of a phase, `−1 … 1`, in phase with [`sine_of`].
#[inline(always)]
#[must_use]
pub fn triangle_of(phase: u64) -> f32 {
    let x = unit(phase);
    if x < 0.25 {
        4.0 * x
    } else if x < 0.75 {
        2.0 - 4.0 * x
    } else {
        4.0 * x - 4.0
    }
}

/// The ensemble's sweep for tap `tap` (0, 1 or 2): `0.9·sine(phase + tap·120°) + 0.1·sine(8·phase + tap·120°)`,
/// `−1 … 1`. Each sweep takes the tap's 120° steps on its own phase.
#[inline(always)]
#[must_use]
pub fn ensemble_of(phase: u64, tap: u64) -> f32 {
    let off = tap.wrapping_mul(THIRD_TURN);
    let slow = sine_of(phase.wrapping_add(off));
    let fast = sine_of(phase.wrapping_mul(FAST_RATIO).wrapping_add(off));
    (1.0 - FAST_SHARE) * slow + FAST_SHARE * fast
}

/// Half a turn of the sweep in counts of 2⁻⁶⁴ cycle (exact in `f64`).
const HALF_TURN: f64 = (1u64 << 63) as f64;

/// The right channel's lag behind the left, as a phase: `width` percent of 180° of the sweep (`0 … 100`; NaN is 0).
#[must_use]
pub fn lag_for_width(width_percent: f32) -> u64 {
    let w = if width_percent.is_nan() {
        0.0
    } else {
        f64::from(width_percent.clamp(0.0, 100.0))
    };
    // 180° is 2⁶³ counts.
    (w / 100.0 * HALF_TURN) as u64
}

/// The phase of frame `i` of a block of `N` frames that starts at `start`, when the block advances the sweep by `inc`:
/// `start + ⌊inc·(i + 1)/N⌋`, exact in integers, equal to `start + inc` on the last frame (so the block boundary
/// carries no rounding).
#[inline(always)]
#[must_use]
pub fn block_phase<const N: usize>(start: u64, inc: u64, i: usize) -> u64 {
    let n = N as u64;
    let k = i as u64 + 1;
    start
        .wrapping_add((inc / n).wrapping_mul(k))
        .wrapping_add((inc % n) * k / n)
}

/// The four-point Lagrange weights for the fraction `x` in `[0, 1)` between the taps at lags `m` and `m + 1`, for taps
/// ordered by lag `m − 1, m, m + 1, m + 2`. They sum to 1 and `weights(0)` is `[0, 1, 0, 0]`.
#[inline(always)]
#[must_use]
pub fn weights(x: f32) -> [f32; 4] {
    let xm1 = x - 1.0;
    let xm2 = x - 2.0;
    let xp1 = x + 1.0;
    [
        -x * xm1 * xm2 * (1.0 / 6.0),
        xp1 * xm1 * xm2 * 0.5,
        -xp1 * x * xm2 * 0.5,
        xp1 * x * xm1 * (1.0 / 6.0),
    ]
}

/// A tap's whole lag and weights: `τ` clamped to `[1, dmax]` (a NaN falls to 1).
#[inline(always)]
fn place(tau: f32, dmax: f32) -> (usize, [f32; 4]) {
    let t = tau.max(MIN_DELAY).min(dmax);
    let whole = t as usize;
    (whole, weights(t - whole as f32))
}

/// One tap read from `line` (`w` is the slot about to be written, so lag 1 is the previous sample): the sum over the
/// lags `≥ 1` and the weight of lag 0 (nonzero only when the tap is between 1 and 2 samples).
#[inline(always)]
fn read(line: &[f32], w: usize, mask: usize, tau: f32, dmax: f32) -> (f32, f32) {
    let (m, c) = place(tau, dmax);
    let at = |lag: usize| line[w.wrapping_sub(lag) & mask];
    if m == 1 {
        (c[1] * at(1) + c[2] * at(2) + c[3] * at(3), c[0])
    } else {
        (
            c[0] * at(m - 1) + c[1] * at(m) + c[2] * at(m + 1) + c[3] * at(m + 2),
            0.0,
        )
    }
}

/// The exact complex response `(re, im)` of one static tap (chorus or flanger mode, no modulation) at `freq_hz`, for a
/// tap of `tau` samples (clamped to `[1, 4096]`), `feedback` (a fraction, clamped to ±[`MAX_FEEDBACK`]) and `mix`
/// 100 %: `H = (1 + (1 − |fb|)·K / (1 − fb·K)) / 2`, `K(ω) = Σ c_k·e^(−jω(m − 1 + k))`, in `f64`. It is the closed form
/// the tests hold the core to, and the numbers `docs/dsp/chorus.md` quotes.
#[must_use]
pub fn response(tau: f64, feedback: f64, freq_hz: f64, sample_rate: f64) -> (f64, f64) {
    use crate::util::math::{cos, sin};
    let fb = feedback.clamp(-f64::from(MAX_FEEDBACK), f64::from(MAX_FEEDBACK));
    let t = tau.clamp(f64::from(MIN_DELAY), 4096.0);
    let m = t.floor();
    let c = weights((t - m) as f32);
    let w = 2.0 * core::f64::consts::PI * freq_hz / sample_rate;
    let (mut k_re, mut k_im) = (0.0, 0.0);
    for (k, ck) in c.iter().enumerate() {
        let ph = -w * (m - 1.0 + k as f64);
        k_re += f64::from(*ck) * cos(ph);
        k_im += f64::from(*ck) * sin(ph);
    }
    // wet = a·K / (1 − fb·K)
    let a = 1.0 - fb.abs();
    let (d_re, d_im) = (1.0 - fb * k_re, -fb * k_im);
    let den = d_re * d_re + d_im * d_im;
    let (n_re, n_im) = (a * k_re, a * k_im);
    let (w_re, w_im) = (
        (n_re * d_re + n_im * d_im) / den,
        (n_im * d_re - n_re * d_im) / den,
    );
    ((1.0 + w_re) / 2.0, w_im / 2.0)
}

// ---------------------------------------------------------------------------------------------------------------
// The core
// ---------------------------------------------------------------------------------------------------------------

/// What one block needs: the settings at its start (the previous block's end) and at its end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The algorithm.
    pub mode: Mode,
    /// How far the sweep advances over this block, in counts of 2⁻⁶⁴ cycle.
    pub phase_inc: u64,
    /// The right channel's lag behind the left ([`lag_for_width`]).
    pub lag: u64,
    /// The centre delay in samples at the start and the end.
    pub delay: [f32; 2],
    /// The sweep depth as a fraction of the delay (`0 … 1`) at the start and the end.
    pub depth: [f32; 2],
    /// The feedback as a fraction (`−0.99 … 0.99`) at the start and the end.
    pub feedback: [f32; 2],
    /// The mix as a fraction (`0 … 1`) at the start and the end.
    pub mix: [f32; 2],
}

#[inline(always)]
fn sane_fraction(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(lo, hi) }
}

const CHORUS: u8 = 0;
const ENSEMBLE: u8 = 1;
const FLANGER: u8 = 2;

/// The stereo modulated-delay core: two delay lines, one write position and one sweep.
#[derive(Clone, Debug)]
pub struct Chorus {
    sample_rate: f32,
    lines: [Vec<f32>; 2],
    write: u32,
    phase: u64,
}

impl Chorus {
    /// A silent core for `sample_rate`: allocates the lines ([`line_len`] samples each). The only allocation.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let len = line_len(sample_rate);
        Self {
            sample_rate,
            lines: [vec![0.0; len], vec![0.0; len]],
            write: 0,
            phase: 0,
        }
    }

    /// The sample rate the lines were sized for.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The bytes the lines hold.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.lines.iter().map(|l| l.len() * 4).sum()
    }

    /// Silence the lines and restart the sweep at phase 0. Does not allocate.
    pub fn reset(&mut self) {
        for l in &mut self.lines {
            l.fill(0.0);
        }
        self.write = 0;
        self.phase = 0;
    }

    /// Wake from silence: the sweep restarts at phase 0, so what follows does not depend on how many blocks were
    /// skipped. Call it only when [`is_state_zero`](Self::is_state_zero).
    pub fn restart_from_silence(&mut self) {
        self.phase = 0;
    }

    /// The sweep's phase (test support).
    #[must_use]
    pub fn phase(&self) -> u64 {
        self.phase
    }

    /// Whether both lines hold only zeros (so silent input gives silent output).
    #[must_use]
    pub fn is_state_zero(&self) -> bool {
        self.lines.iter().all(|l| l.iter().all(|x| *x == 0.0))
    }

    /// The number of subnormal values in the lines (test support).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.iter().filter(|x| x.is_subnormal()).count())
            .sum()
    }

    /// Copy the whole state of `other` (a plan swap that crossfades, where both plans run). `false`, and nothing
    /// copied, if the two were sized for different sample rates. No allocation.
    pub fn copy_state_from(&mut self, other: &Self) -> bool {
        if self.sample_rate.to_bits() != other.sample_rate.to_bits() {
            return false;
        }
        for (a, b) in self.lines.iter_mut().zip(&other.lines) {
            a.copy_from_slice(b);
        }
        self.write = other.write;
        self.phase = other.phase;
        true
    }

    /// Exchange the whole state with `other` in O(1) (an instant plan swap: the old plan never runs again). `false`,
    /// and nothing moved, if the two were sized for different sample rates.
    pub fn swap_state(&mut self, other: &mut Self) -> bool {
        if self.sample_rate.to_bits() != other.sample_rate.to_bits() {
            return false;
        }
        core::mem::swap(self, other);
        true
    }

    /// Process one block of `N` frames in place. The delay, depth, feedback and mix ramp linearly per sample from the
    /// block's start values to its end values. At mix 0 at both ends the block is not written, so the input passes bit
    /// for bit while the lines keep running.
    pub fn process_block<const N: usize>(
        &mut self,
        seg: &Segment,
        left: &mut [f32; N],
        right: &mut [f32; N],
    ) {
        let write = seg.mix.map(|m| sane_fraction(m, 0.0, 1.0)) != [0.0, 0.0];
        match (seg.mode, write) {
            (Mode::Chorus, true) => self.run::<N, CHORUS, true>(seg, left, right),
            (Mode::Chorus, false) => self.run::<N, CHORUS, false>(seg, left, right),
            (Mode::Ensemble, true) => self.run::<N, ENSEMBLE, true>(seg, left, right),
            (Mode::Ensemble, false) => self.run::<N, ENSEMBLE, false>(seg, left, right),
            (Mode::Flanger, true) => self.run::<N, FLANGER, true>(seg, left, right),
            (Mode::Flanger, false) => self.run::<N, FLANGER, false>(seg, left, right),
        }
        self.phase = self.phase.wrapping_add(seg.phase_inc);
    }

    #[inline(always)]
    fn run<const N: usize, const MODE: u8, const WRITE: bool>(
        &mut self,
        seg: &Segment,
        left: &mut [f32; N],
        right: &mut [f32; N],
    ) {
        let [line_l, line_r] = &mut self.lines;
        let mask = line_l.len() - 1;
        let dmax = (line_l.len() - GUARD) as f32;
        let inv_n = 1.0 / N as f32;
        let d = seg.delay.map(|x| x.max(MIN_DELAY).min(dmax));
        let depth = seg.depth.map(|x| sane_fraction(x, 0.0, 1.0));
        let fb = seg
            .feedback
            .map(|x| sane_fraction(x, -MAX_FEEDBACK, MAX_FEEDBACK));
        let mix = seg.mix.map(|x| sane_fraction(x, 0.0, 1.0));
        let mut w = self.write;
        for i in 0..N {
            let r = (i + 1) as f32 * inv_n;
            let tap = Tap {
                delay: d[0] + (d[1] - d[0]) * r,
                depth: depth[0] + (depth[1] - depth[0]) * r,
                feedback: fb[0] + (fb[1] - fb[0]) * r,
                dmax,
            };
            let m = mix[0] + (mix[1] - mix[0]) * r;
            let p = block_phase::<N>(self.phase, seg.phase_inc, i);
            let wi = w as usize;
            let (xl, xr) = (left[i], right[i]);
            let wet_l = channel::<MODE>(line_l, wi, mask, p, &tap, xl);
            let wet_r = channel::<MODE>(line_r, wi, mask, p.wrapping_sub(seg.lag), &tap, xr);
            if WRITE {
                left[i] = (1.0 - m) * xl + m * (0.5 * (xl + wet_l));
                right[i] = (1.0 - m) * xr + m * (0.5 * (xr + wet_r));
            }
            w = w.wrapping_add(1);
        }
        self.write = w;
    }
}

/// One sample's tap settings (shared by both channels).
struct Tap {
    delay: f32,
    depth: f32,
    feedback: f32,
    dmax: f32,
}

/// One channel's sample: read the taps at sweep phase `pc`, solve the loop for `v`, write it, return the wet signal.
#[inline(always)]
fn channel<const MODE: u8>(
    line: &mut [f32],
    w: usize,
    mask: usize,
    pc: u64,
    tap: &Tap,
    x: f32,
) -> f32 {
    let (rsum, c0) = if MODE == ENSEMBLE {
        let (mut rs, mut cs) = (0.0, 0.0);
        for k in 0..3u64 {
            let s = ensemble_of(pc, k);
            let (rk, ck) = read(line, w, mask, tap.delay * (1.0 + tap.depth * s), tap.dmax);
            rs += rk;
            cs += ck;
        }
        (rs * (1.0 / 3.0), cs * (1.0 / 3.0))
    } else {
        let s = if MODE == FLANGER {
            triangle_of(pc)
        } else {
            sine_of(pc)
        };
        read(line, w, mask, tap.delay * (1.0 + tap.depth * s), tap.dmax)
    };
    let f = tap.feedback;
    let drive = (1.0 - f.abs()) * x + f * rsum;
    let (v, wet) = if c0 == 0.0 {
        (drive, rsum)
    } else {
        let v = drive / (1.0 - f * c0);
        (v, c0 * v + rsum)
    };
    line[w & mask] = flush(v);
    wet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::math;
    use crate::util::rng::Rng;

    const N: usize = 32;
    const SR: f32 = 48_000.0;
    const TWO_64: f64 = 2.0 * HALF_TURN;

    fn sq(x: f64) -> f64 {
        x * x
    }

    fn seg(mode: Mode, delay: f32, depth: f32, fb: f32, mix: f32) -> Segment {
        Segment {
            mode,
            phase_inc: 0,
            lag: 0,
            delay: [delay; 2],
            depth: [depth; 2],
            feedback: [fb; 2],
            mix: [mix; 2],
        }
    }

    /// Run `seg` over `left`/`right` (lengths a multiple of 32), a block at a time.
    fn run(core: &mut Chorus, seg: &Segment, left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
        assert!(left.len().is_multiple_of(N) && left.len() == right.len());
        let (mut ol, mut or) = (left.to_vec(), right.to_vec());
        for (l, r) in ol
            .as_chunks_mut::<N>()
            .0
            .iter_mut()
            .zip(or.as_chunks_mut::<N>().0.iter_mut())
        {
            core.process_block::<N>(seg, l, r);
        }
        (ol, or)
    }

    fn impulse(len: usize, at: usize) -> Vec<f32> {
        let mut x = vec![0.0; len];
        x[at] = 1.0;
        x
    }

    /// The wet signal of a mix-100 % output: `2y − x`.
    fn wet_of(y: &[f32], x: &[f32]) -> Vec<f64> {
        y.iter()
            .zip(x)
            .map(|(y, x)| 2.0 * f64::from(*y) - f64::from(*x))
            .collect()
    }

    /// The first moment of an impulse's wet response over its sum: the tap's delay in samples (Lagrange reproduces a
    /// ramp). With a static tap the sum is 1; a moving tap stretches or squeezes the impulse (the Doppler effect), so
    /// the sum is `1/(1 − dτ/dn)` and only the ratio is the delay.
    fn centroid_and_sum(wet: &[f64], at: usize) -> (f64, f64) {
        let (mut m0, mut m1) = (0.0, 0.0);
        for (n, w) in wet.iter().enumerate().skip(at) {
            m0 += w;
            m1 += w * (n - at) as f64;
        }
        (m1 / m0, m0)
    }

    /// [`centroid_and_sum`] of a static tap, whose kernel sums to one.
    fn centroid(wet: &[f64], at: usize) -> f64 {
        let (c, sum) = centroid_and_sum(wet, at);
        assert!((sum - 1.0).abs() < 1e-6, "the kernel sums to one: {sum}");
        c
    }

    fn dft(x: &[f64], f: f64, sr: f64) -> (f64, f64) {
        let w = 2.0 * core::f64::consts::PI * f / sr;
        let (mut re, mut im) = (0.0, 0.0);
        for (n, v) in x.iter().enumerate() {
            re += v * math::cos(w * n as f64);
            im -= v * math::sin(w * n as f64);
        }
        (re, im)
    }

    #[test]
    fn the_weights_sum_to_one_and_reproduce_cubics() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let c = weights(x);
            let sum: f64 = c.iter().map(|v| f64::from(*v)).sum();
            assert!((sum - 1.0).abs() < 1e-6, "{x}: {sum}");
            for p in 1..=3 {
                let got: f64 = c
                    .iter()
                    .enumerate()
                    .map(|(k, v)| f64::from(*v) * libm::pow(k as f64, f64::from(p)))
                    .sum();
                let want = libm::pow(1.0 + f64::from(x), f64::from(p));
                assert!(
                    (got - want).abs() < 1e-5,
                    "x {x} power {p}: {got} vs {want}"
                );
            }
        }
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn the_interpolator_never_amplifies() {
        // The loop's stability and the level bound both rest on |K(ω)| ≤ 1 for every fraction.
        let mut worst: f64 = 0.0;
        for i in 0..=200 {
            let c = weights(i as f32 / 200.0);
            for j in 0..=400 {
                let w = core::f64::consts::PI * f64::from(j) / 400.0;
                let (mut re, mut im) = (0.0, 0.0);
                for (k, ck) in c.iter().enumerate() {
                    re += f64::from(*ck) * math::cos(w * k as f64);
                    im -= f64::from(*ck) * math::sin(w * k as f64);
                }
                worst = worst.max((re * re + im * im).sqrt());
            }
        }
        assert!(worst <= 1.0 + 1e-6, "the largest |K| is {worst}");
    }

    #[test]
    fn block_phase_is_exact_and_lands_on_the_next_block() {
        let mut rng = Rng::new(7);
        for _ in 0..200 {
            let (start, inc) = (rng.next_u64(), rng.next_u64());
            assert_eq!(block_phase::<N>(start, inc, N - 1), start.wrapping_add(inc));
            let mut last = start;
            for i in 0..N {
                let p = block_phase::<N>(start, inc, i);
                // Steps are ⌊inc/N⌋ or one more.
                let step = p.wrapping_sub(last);
                assert!(
                    step == inc / N as u64 || step == inc / N as u64 + 1,
                    "{step}"
                );
                last = p;
            }
        }
    }

    #[test]
    fn the_sweeps_are_unit_and_in_phase() {
        for i in 0..4096u64 {
            let p = i << 52;
            let x = f64::from(unit(p));
            let want = math::sin(2.0 * core::f64::consts::PI * x);
            assert!((f64::from(sine_of(p)) - want).abs() < 2e-6);
            let tri = f64::from(triangle_of(p));
            assert!((-1.0..=1.0).contains(&tri));
            // The triangle and the sine have the same sign and cross zero together.
            assert!(tri * want >= -1e-6);
            for k in 0..3 {
                assert!(ensemble_of(p, k).abs() <= 1.0 + 1e-6);
            }
        }
        assert_eq!(sine_of(0), 0.0);
        assert_eq!(triangle_of(0), 0.0);
        assert!((sine_of(1 << 62) - 1.0).abs() < 1e-6);
        assert_eq!(triangle_of(1 << 62), 1.0);
        assert_eq!(triangle_of(3 << 62), -1.0);
    }

    #[test]
    fn width_is_a_share_of_half_a_cycle() {
        assert_eq!(lag_for_width(0.0), 0);
        assert_eq!(lag_for_width(100.0), 1 << 63);
        assert_eq!(lag_for_width(50.0), 1 << 62);
        assert_eq!(lag_for_width(f32::NAN), 0);
        assert_eq!(lag_for_width(-5.0), 0);
        assert_eq!(lag_for_width(500.0), 1 << 63);
    }

    #[test]
    fn a_static_tap_has_the_closed_form_response_at_every_feedback() {
        // The impulse response must have decayed within the window: 0.99^(n/τ) is small only for short taps.
        let len = 65_536;
        for &tau in &[1.0f32, 1.3, 1.99, 2.0, 5.5, 100.25, 336.0] {
            for &fb in &[0.0f32, 0.5, -0.5, 0.9, 0.99, -0.99] {
                if tau > 6.0 && fb.abs() > 0.6 {
                    continue;
                }
                let mut core = Chorus::new(SR);
                let x = impulse(len, 0);
                let (y, _) = run(&mut core, &seg(Mode::Chorus, tau, 0.0, fb, 1.0), &x, &x);
                let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
                for &f in &[50.0f64, 400.0, 1_500.0, 6_000.0, 12_000.0, 21_000.0] {
                    let (re, im) = dft(&h, f, f64::from(SR));
                    let (cre, cim) = response(f64::from(tau), f64::from(fb), f, f64::from(SR));
                    let err = (sq(re - cre) + sq(im - cim)).sqrt();
                    assert!(err < 3e-5, "tau {tau} fb {fb} {f} Hz: {err}");
                }
            }
        }
    }

    #[test]
    fn the_level_is_bounded_at_every_feedback_and_delay() {
        // |H| = |1 + a·K/(1 − fb·K)| / 2 ≤ 1 everywhere: no resonance and no summing lobe above 0 dB.
        let mut worst: f64 = 0.0;
        for &fb in &[0.0, 0.3, 0.5, 0.9, 0.99, -0.3, -0.5, -0.9, -0.99] {
            for &tau in &[1.0, 1.5, 2.0, 3.7, 20.2, 336.0, 1000.5, 2880.0] {
                for j in 0..=2400 {
                    let f = 24_000.0 * f64::from(j) / 2400.0;
                    let (re, im) = response(tau, fb, f, 48_000.0);
                    worst = worst.max((re * re + im * im).sqrt());
                }
            }
        }
        // The f32 weights sum to 1 within 6e-8, which the loop magnifies by 1/(1 − |fb|) = 100 at 99 %.
        assert!(worst <= 1.0 + 1e-5, "peak gain {worst}");
        assert!(worst > 0.999, "the bound is reached (DC): {worst}");
    }

    #[test]
    fn white_noise_never_gains_level_at_the_extreme_feedbacks() {
        let mut rng = Rng::new(3);
        let n = 96_000;
        let x: Vec<f32> = (0..n).map(|_| rng.bipolar_f32()).collect();
        let e_in: f64 = x.iter().map(|v| sq(f64::from(*v))).sum();
        for mode in [Mode::Chorus, Mode::Ensemble, Mode::Flanger] {
            for fb in [0.99f32, -0.99, 0.5, 0.0] {
                for delay in [1.0f32, 5.0, 48.0, 336.0] {
                    let mut core = Chorus::new(SR);
                    let mut s = seg(mode, delay, 0.8, fb, 1.0);
                    s.phase_inc = 1 << 53;
                    s.lag = lag_for_width(100.0);
                    let (l, _) = run(&mut core, &s, &x, &x);
                    let e_out: f64 = l.iter().map(|v| sq(f64::from(*v))).sum();
                    assert!(
                        e_out <= e_in * 1.001,
                        "{mode:?} fb {fb} delay {delay}: {e_out} vs {e_in}"
                    );
                    assert!(l.iter().all(|v| v.is_finite() && v.abs() < 4.0));
                }
            }
        }
    }

    #[test]
    fn a_full_scale_saw_keeps_its_level_at_the_defaults_and_at_99_percent() {
        // 100 Hz saw at full scale. The level bound: the RMS never rises and falls by at most about 3 dB; the peak
        // rises only by the cubic interpolator's overshoot at the saw's edge (at most 1/16 of the step, half of it
        // after the sum with the dry signal).
        let n = 48_000;
        let x: Vec<f32> = (0..n)
            .map(|i| 2.0 * ((i as f32 * 100.0 / 48_000.0) % 1.0) - 1.0)
            .collect();
        for mode in [Mode::Chorus, Mode::Ensemble, Mode::Flanger] {
            for fb in [0.0f32, 0.99, -0.99] {
                let mut core = Chorus::new(SR);
                let mut s = seg(mode, 336.0, 0.5, fb, 1.0);
                s.phase_inc = lag_for_width(1.0) / 4;
                let (l, _) = run(&mut core, &s, &x, &x);
                let peak = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak <= 1.07, "{mode:?} fb {fb}: peak {peak}");
                let rms = |v: &[f32]| {
                    (v.iter().map(|a| sq(f64::from(*a))).sum::<f64>() / v.len() as f64).sqrt()
                };
                let db = 20.0 * math::log10(rms(&l[4800..]) / rms(&x[4800..]));
                // At the defaults (no feedback) the sum with the dry signal costs at most about 3 dB; a feedback of 99 %
                // sends most of the signal into narrow teeth, so the wet path adds little and the sum is about half.
                // The ensemble's mean of three decorrelated copies is weaker still (about 5 dB on a bright saw).
                let floor = match (fb == 0.0, mode) {
                    (true, Mode::Ensemble) => -5.6,
                    (true, _) => -3.6,
                    (false, _) => -6.5,
                };
                assert!(
                    (floor..=0.05).contains(&db),
                    "{mode:?} fb {fb}: level change {db:.2} dB"
                );
            }
        }
    }

    #[test]
    fn the_impulse_delay_follows_the_sweep_exactly() {
        // Freeze the sweep (increment 0) at 64 phases over a cycle and measure the delay of an impulse: the first moment
        // of the wet response is the tap, delay · (1 + depth · s).
        let (delay, depth) = (336.0f32, 0.5f32);
        for (mode, sweep) in [(Mode::Chorus, 0), (Mode::Flanger, 1)] {
            let mut seen = (f64::INFINITY, f64::NEG_INFINITY);
            for i in 0..64u64 {
                let phase = i << 58;
                let mut core = Chorus::new(SR);
                core.phase = phase;
                let x = impulse(1024, 0);
                let (y, _) = run(&mut core, &seg(mode, delay, depth, 0.0, 1.0), &x, &x);
                let tau = centroid(&wet_of(&y, &x), 0);
                let s = if sweep == 0 {
                    sine_of(phase)
                } else {
                    triangle_of(phase)
                };
                let want = f64::from(delay) * (1.0 + f64::from(depth) * f64::from(s));
                assert!(
                    (tau - want).abs() < 2e-4,
                    "{mode:?} phase {i}/64: {tau} vs {want}"
                );
                seen = (seen.0.min(tau), seen.1.max(tau));
            }
            // The swing is exactly delay ± depth·delay (the sine reaches ±1 at the quarter points of the 64).
            assert!(
                (seen.0 - 168.0).abs() < 1e-3 && (seen.1 - 504.0).abs() < 1e-3,
                "{mode:?} {seen:?}"
            );
        }
    }

    #[test]
    fn at_full_depth_the_delay_swings_between_none_and_double() {
        // 100 %: a sine sweep takes the tap from 2·delay down to the 1-sample clamp.
        let delay = 100.0f32;
        let lo = {
            let mut core = Chorus::new(SR);
            core.phase = 3 << 62; // sine = −1
            let x = impulse(512, 0);
            let (y, _) = run(&mut core, &seg(Mode::Chorus, delay, 1.0, 0.0, 1.0), &x, &x);
            centroid(&wet_of(&y, &x), 0)
        };
        let hi = {
            let mut core = Chorus::new(SR);
            core.phase = 1 << 62; // sine = +1
            let x = impulse(512, 0);
            let (y, _) = run(&mut core, &seg(Mode::Chorus, delay, 1.0, 0.0, 1.0), &x, &x);
            centroid(&wet_of(&y, &x), 0)
        };
        assert!((lo - 1.0).abs() < 1e-5, "the clamp: {lo}");
        assert!((hi - 200.0).abs() < 1e-3, "double: {hi}");
    }

    #[test]
    fn the_ensemble_runs_three_taps_a_third_of_a_turn_apart() {
        let (delay, depth) = (300.0f32, 0.6f32);
        for i in 0..40u64 {
            let phase = i << 58 | 0x3039;
            let mut core = Chorus::new(SR);
            core.phase = phase;
            let x = impulse(2048, 0);
            let (y, _) = run(
                &mut core,
                &seg(Mode::Ensemble, delay, depth, 0.0, 1.0),
                &x,
                &x,
            );
            let wet = wet_of(&y, &x);
            let mean: f64 = (0..3)
                .map(|k| {
                    f64::from(delay) * (1.0 + f64::from(depth) * f64::from(ensemble_of(phase, k)))
                })
                .sum::<f64>()
                / 3.0;
            let tau = centroid(&wet, 0);
            assert!((tau - mean).abs() < 5e-4, "{i}: {tau} vs {mean}");
        }
        // The taps differ (that is the ensemble), and the sweep stays inside ±1.
        let s: Vec<f32> = (0..3).map(|k| ensemble_of(1 << 60, k)).collect();
        assert!((s[0] - s[1]).abs() > 0.05 && (s[1] - s[2]).abs() > 0.05);
    }

    #[test]
    fn the_right_channel_lags_the_left_by_the_width() {
        let delay = 200.0f32;
        for (width, cycles) in [(0.0f32, 0.0f64), (50.0, 0.25), (100.0, 0.5)] {
            let mut core = Chorus::new(SR);
            core.phase = 7 << 59;
            let x = impulse(1024, 0);
            let mut s = seg(Mode::Chorus, delay, 0.7, 0.0, 1.0);
            s.lag = lag_for_width(width);
            let (yl, yr) = run(&mut core, &s, &x, &x);
            let p = 7.0 / 32.0;
            let sweep = |c: f64| math::sin(2.0 * core::f64::consts::PI * c);
            let want_l = 200.0 * (1.0 + 0.7 * sweep(p));
            let want_r = 200.0 * (1.0 + 0.7 * sweep(p - cycles));
            assert!((centroid(&wet_of(&yl, &x), 0) - want_l).abs() < 1e-3);
            assert!(
                (centroid(&wet_of(&yr, &x), 0) - want_r).abs() < 1e-3,
                "width {width}"
            );
            if width == 0.0 {
                assert_eq!(yl, yr, "width 0 is mono");
            }
        }
    }

    #[test]
    fn a_moving_sweep_keeps_the_delay_inside_its_range() {
        // A 1 Hz sweep (a cycle is 48,000 samples): impulses every 2,048 samples over two cycles.
        let (delay, depth) = (336.0f32, 0.5f32);
        // 2⁶⁴ / 48,000 per sample, times the 32 samples of a block.
        let inc = (TWO_64 / 48_000.0 * N as f64) as u64;
        let spacing = 2048;
        let n = 96_000 / spacing * spacing;
        let mut x = vec![0.0; n];
        for k in 0..n / spacing {
            x[k * spacing] = 1.0;
        }
        let mut core = Chorus::new(SR);
        let mut s = seg(Mode::Chorus, delay, depth, 0.0, 1.0);
        s.phase_inc = inc;
        let (y, _) = run(&mut core, &s, &x, &x);
        let wet = wet_of(&y, &x);
        let mut seen = (f64::INFINITY, f64::NEG_INFINITY);
        for k in 0..n / spacing - 1 {
            let at = k * spacing;
            let window = &wet[at..at + 700];
            let (tau, sum) = centroid_and_sum(window, 0);
            assert!(
                (sum - 1.0).abs() < 0.04,
                "Doppler stretches the kernel by the sweep's slope only: {sum}"
            );
            seen = (seen.0.min(tau), seen.1.max(tau));
            // The sweep at the output time n_out = at + tau (solve n − τ(n) = at by iteration).
            let mut t = f64::from(delay);
            for _ in 0..8 {
                let phase = ((at as f64 + t) * inc as f64 / 32.0).rem_euclid(TWO_64);
                t = f64::from(delay) * (1.0 + f64::from(depth) * f64::from(sine_of(phase as u64)));
            }
            assert!((tau - t).abs() < 0.12, "impulse {k}: {tau} vs {t}");
        }
        assert!(seen.0 < 175.0 && seen.1 > 497.0, "{seen:?}");
        assert!(
            seen.0 > 167.5 && seen.1 < 504.5,
            "never outside delay ± depth: {seen:?}"
        );
    }

    #[test]
    fn the_dry_signal_is_never_delayed() {
        // Latency 0: the first output sample of an impulse is the impulse's own (half of it, plus the lag-0 share of a
        // tap between one and two samples). Nothing precedes it.
        for &delay in &[1.0f32, 1.4, 2.0, 10.0, 336.0] {
            for mode in [Mode::Chorus, Mode::Ensemble, Mode::Flanger] {
                let mut core = Chorus::new(SR);
                let x = impulse(256, 100);
                let (y, _) = run(&mut core, &seg(mode, delay, 0.0, 0.0, 1.0), &x, &x);
                assert!(
                    y[..100].iter().all(|v| *v == 0.0),
                    "{mode:?} {delay}: nothing before"
                );
                assert!(
                    (0.46..=0.54).contains(&y[100]),
                    "{mode:?} {delay}: {}",
                    y[100]
                );
            }
        }
    }

    #[test]
    fn the_flanger_reaches_its_zero_delay_null_without_delaying_the_output() {
        // The triangle at s = −1 with depth 100 %: the wet tap is clamped to one sample, so the wet signal is the
        // input one sample late and the first comb notch sits at Nyquist.
        let mut core = Chorus::new(SR);
        core.phase = 3 << 62;
        let x = impulse(64, 10);
        let (y, _) = run(&mut core, &seg(Mode::Flanger, 20.0, 1.0, 0.0, 1.0), &x, &x);
        assert_eq!(y[10], 0.5);
        assert_eq!(y[11], 0.5);
        assert!(y[12..].iter().all(|v| *v == 0.0));
        let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
        let (re, im) = dft(&h, 24_000.0, 48_000.0);
        assert!(
            libm::hypot(re, im) < 1e-6,
            "the comb's first notch is at Nyquist"
        );
        let (re, im) = dft(&h, 100.0, 48_000.0);
        assert!(libm::hypot(re, im) > 0.99, "and DC passes");
    }

    #[test]
    fn the_flanger_notches_are_the_odd_multiples_of_half_over_tau() {
        // Without feedback the comb's notches are at (2k + 1) / (2τ); measure the minima of the spectrum.
        let len = 16_384;
        for &tau in &[24.0f32, 48.0, 96.5, 144.25] {
            let mut core = Chorus::new(SR);
            let x = impulse(len, 0);
            let (y, _) = run(&mut core, &seg(Mode::Flanger, tau, 0.0, 0.0, 1.0), &x, &x);
            let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
            for k in 0..3 {
                let want = (2.0 * f64::from(k) + 1.0) * 48_000.0 / (2.0 * f64::from(tau));
                if want > 9_000.0 {
                    continue;
                }
                // The deepest bin within ±10 % of the prediction, refined by a parabola on the dB values.
                let (mut best, mut best_f) = (f64::INFINITY, 0.0);
                let mut f = want * 0.9;
                while f < want * 1.1 {
                    let (re, im) = dft(&h[..4096], f, 48_000.0);
                    if libm::hypot(re, im) < best {
                        best = libm::hypot(re, im);
                        best_f = f;
                    }
                    f += want * 0.0005;
                }
                assert!(
                    (best_f / want - 1.0).abs() < 0.01,
                    "tau {tau} notch {k}: {best_f} vs {want}"
                );
            }
        }
    }

    #[test]
    fn feedback_through_a_tap_between_one_and_two_samples_matches_a_reference_solution() {
        // The lag-0 term is solved, not delayed: compare with a direct f64 simulation that iterates the loop
        // equation to convergence at each sample.
        let mut rng = Rng::new(11);
        let n = 4096;
        let x: Vec<f32> = (0..n).map(|_| rng.bipolar_f32()).collect();
        for &(tau, fb) in &[(1.0f64, 0.9f64), (1.37, 0.99), (1.99, -0.99), (1.5, 0.5)] {
            let mut core = Chorus::new(SR);
            let (y, _) = run(
                &mut core,
                &seg(Mode::Chorus, tau as f32, 0.0, fb as f32, 1.0),
                &x,
                &x,
            );
            let m = tau.floor();
            let c = weights((tau - m) as f32);
            let a = 1.0 - fb.abs();
            let mut v = vec![0.0f64; n];
            for i in 0..n {
                let at = |lag: usize| if i >= lag { v[i - lag] } else { 0.0 };
                let r = f64::from(c[1]) * at(1) + f64::from(c[2]) * at(2) + f64::from(c[3]) * at(3);
                let mut vi = 0.0;
                for _ in 0..60 {
                    let wet = f64::from(c[0]) * vi + r;
                    vi = a * f64::from(x[i]) + fb * wet;
                }
                v[i] = vi;
                let wet = f64::from(c[0]) * vi + r;
                let want = 0.5 * (f64::from(x[i]) + wet);
                assert!(
                    (f64::from(y[i]) - want).abs() < 2e-5,
                    "tau {tau} fb {fb} n {i}: {} vs {want}",
                    y[i]
                );
            }
        }
    }

    #[test]
    fn mix_zero_leaves_the_block_alone_while_the_lines_run() {
        let mut rng = Rng::new(5);
        let n = 4096;
        let x: Vec<f32> = (0..n).map(|_| rng.bipolar_f32()).collect();
        let mut s = seg(Mode::Ensemble, 300.0, 0.5, 0.6, 0.0);
        s.phase_inc = 1 << 55;
        let mut a = Chorus::new(SR);
        let (y0, _) = run(&mut a, &s, &x, &x);
        assert_eq!(
            y0.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "mix 0 passes the input bit for bit"
        );
        // It ran underneath: switching the mix to 100 % gives what a core that was wet all along gives.
        s.mix = [1.0; 2];
        let next: Vec<f32> = (0..n).map(|_| rng.bipolar_f32()).collect();
        let (wet_a, _) = run(&mut a, &s, &next, &next);
        let mut b = Chorus::new(SR);
        let _ = run(&mut b, &s, &x, &x);
        let (wet_b, _) = run(&mut b, &s, &next, &next);
        assert_eq!(
            wet_a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            wet_b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        // And negative zero passes untouched.
        let mut c = Chorus::new(SR);
        let mut z = [-0.0f32; N];
        let mut z2 = z;
        c.process_block::<N>(&seg(Mode::Chorus, 100.0, 0.5, 0.0, 0.0), &mut z, &mut z2);
        assert_eq!(z[0].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn the_mix_is_a_linear_blend_and_a_full_mix_is_the_effect() {
        let mut rng = Rng::new(9);
        let x: Vec<f32> = (0..2048).map(|_| rng.bipolar_f32()).collect();
        let mut full = Chorus::new(SR);
        let mut half = Chorus::new(SR);
        let (yf, _) = run(&mut full, &seg(Mode::Chorus, 90.0, 0.5, 0.3, 1.0), &x, &x);
        let (yh, _) = run(&mut half, &seg(Mode::Chorus, 90.0, 0.5, 0.3, 0.5), &x, &x);
        for i in 0..x.len() {
            let want = 0.5 * x[i] + 0.5 * yf[i];
            assert!((yh[i] - want).abs() < 1e-6, "{i}");
        }
    }

    #[test]
    fn two_cores_keep_separate_state_and_the_stereo_channels_are_independent() {
        let mut rng = Rng::new(21);
        let xl: Vec<f32> = (0..2048).map(|_| rng.bipolar_f32()).collect();
        let xr = vec![0.0f32; 2048];
        let mut s = seg(Mode::Ensemble, 300.0, 0.5, 0.4, 1.0);
        s.phase_inc = 1 << 54;
        let (mut a, mut b) = (Chorus::new(SR), Chorus::new(SR));
        let (al, ar) = run(&mut a, &s, &xl, &xr);
        let (bl, _) = run(&mut b, &s, &xl, &xl);
        assert_eq!(al, bl, "the left channel does not hear the right");
        assert!(
            ar.iter().all(|v| *v == 0.0),
            "a silent right channel stays silent"
        );
        let mut c = Chorus::new(SR);
        let _ = run(&mut c, &s, &xr, &xr);
        assert!(c.is_state_zero());
    }

    #[test]
    fn the_state_drains_and_is_reported_zero() {
        let mut core = Chorus::new(SR);
        assert!(core.is_state_zero());
        let x = impulse(64, 3);
        let _ = run(&mut core, &seg(Mode::Chorus, 100.0, 0.5, 0.0, 1.0), &x, &x);
        assert!(!core.is_state_zero());
        let silence = vec![0.0f32; line_len(SR) + 64];
        let _ = run(
            &mut core,
            &seg(Mode::Chorus, 100.0, 0.5, 0.0, 1.0),
            &silence,
            &silence,
        );
        assert!(
            core.is_state_zero(),
            "a line with no feedback is empty a line length later"
        );
        assert_eq!(core.count_subnormals(), 0);
    }

    #[test]
    fn a_99_percent_tail_decays_to_exact_zero_without_subnormals() {
        let mut core = Chorus::new(SR);
        let x = impulse(64, 0);
        let mut s = seg(Mode::Chorus, 2.0, 0.0, 0.99, 1.0);
        let _ = run(&mut core, &s, &x, &x);
        s.phase_inc = 1 << 52;
        let silence = vec![0.0f32; 48_000];
        for _ in 0..16 {
            let (l, r) = run(&mut core, &s, &silence, &silence);
            assert!(
                l.iter()
                    .chain(&r)
                    .all(|v| !v.is_subnormal() && v.is_finite())
            );
            assert_eq!(core.count_subnormals(), 0);
        }
        assert!(
            core.is_state_zero(),
            "flushed to exactly zero within 16 s at a 2-sample loop"
        );
    }

    #[test]
    fn copy_and_swap_carry_the_whole_state() {
        let mut rng = Rng::new(17);
        let x: Vec<f32> = (0..2048).map(|_| rng.bipolar_f32()).collect();
        let mut s = seg(Mode::Chorus, 150.0, 0.5, 0.5, 1.0);
        s.phase_inc = 1 << 55;
        let mut a = Chorus::new(SR);
        let _ = run(&mut a, &s, &x, &x);
        let mut copy = Chorus::new(SR);
        assert!(copy.copy_state_from(&a));
        let mut moved = Chorus::new(SR);
        let mut donor = a.clone();
        assert!(moved.swap_state(&mut donor));
        assert!(
            donor.is_state_zero(),
            "the donor now holds the fresh core's empty lines"
        );
        let next: Vec<f32> = (0..2048).map(|_| rng.bipolar_f32()).collect();
        let (ya, _) = run(&mut a, &s, &next, &next);
        let (yc, _) = run(&mut copy, &s, &next, &next);
        let (ym, _) = run(&mut moved, &s, &next, &next);
        assert_eq!(ya, yc);
        assert_eq!(ya, ym);
        assert!(
            !Chorus::new(44_100.0).copy_state_from(&a),
            "a different sample rate refuses"
        );
    }

    #[test]
    fn the_lines_are_sized_from_the_sample_rate_alone() {
        for (sr, len) in [
            (8_000.0f32, 512),
            (44_100.0, 4096),
            (48_000.0, 4096),
            (96_000.0, 8192),
            (192_000.0, 16_384),
            (384_000.0, 32_768),
        ] {
            assert_eq!(line_len(sr), len, "{sr}");
            assert!(line_len(sr) as f32 >= max_tap_samples(sr) + 4.0);
            assert_eq!(Chorus::new(sr).allocated_bytes(), 2 * len * 4);
        }
    }

    #[test]
    fn nan_parameters_fall_back_instead_of_breaking_the_line() {
        let mut core = Chorus::new(SR);
        let mut s = seg(Mode::Ensemble, f32::NAN, f32::NAN, f32::NAN, 1.0);
        s.delay = [f32::NAN, 1e9];
        let x = vec![0.5f32; 256];
        let (l, _) = run(&mut core, &s, &x, &x);
        assert!(l.iter().all(|v| v.is_finite()));
    }
}
