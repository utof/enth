//! The stereo phaser behind the `phaser` module (S2a lane MOD): a chain of first-order TPT all-passes with a swept
//! corner, summed with the dry signal. Notes: `docs/dsp/phaser.md`. The sweep helpers are `chorus.rs`'s.
//!
//! Written from Vadim Zavalishin's *The Art of VA Filter Design* (the topology-preserving one-pole and the all-pass
//! made from it, `y = 2·lp − x`), from Julius O. Smith III's *Introduction to Digital Filters* ("Phasing", the notch
//! rule of an all-pass chain) and from the F1m catalogue table (`enth-patch`'s `phaser.rs`). No code is ported.
//!
//! ```text
//! v[n]   = (1 − |fb|) · x[n] + fb · wet[n − 1]
//! wet[n] = A^N (v[n])                         N first-order all-passes of one corner fc
//! y[n]   = (x[n] + wet[n]) / 2                mix 100 %
//! out    = (1 − mix) · x + mix · y
//! fc     = center · 2^(3 · depth · s(t))      s the sine sweep, −1 … 1: depth 100 % is ±3 octaves
//! ```
//!
//! - **The notches** (no feedback) are where the chain's phase is an odd multiple of −π, because the sum of a signal
//!   with a copy of the opposite sign cancels. One stage is `A = (1 − j·u)/(1 + j·u)`, `u = tan(ω/2)/g`,
//!   `g = tan(π·fc/fs)` (the bilinear transform pre-warped so the corner is exactly `fc`), with phase `−2·atan(u)`.
//!   So `N` stages put notches at `ω_k = 2·atan(g·tan((2k + 1)·π/(2N)))` for `(2k + 1) < N`: every two stages make one
//!   notch, an odd count adds a half (a notch at Nyquist). [`notch_hz`] is that closed form.
//! - **The level is bounded.** The loop's input is scaled by `1 − |fb|` and a pure phase has magnitude 1, so
//!   `|wet/x| = (1 − |fb|) / |1 − fb·e^(−jω)·A^N| ≤ 1`: the chain peaks at 0 dB at every feedback, where the bare
//!   `x + fb · wet` loop peaks at `1/(1 − |fb|)` (+40 dB at 99 %). The sum with the dry signal is at most 1.
//! - **The feedback is taken from the previous output**, `wet[n − 1]`: the all-passes pass the input straight through
//!   (`A(∞) ≠ 0`), so a feedback without the unit delay would have no solution at the same sample.
//! - **Modulation.** The coefficient `G = g/(1 + g)` is computed once per block, for the sweep at the block's end
//!   (`exp2` and `tan` through the pinned `libm`), and ramped linearly per sample from the previous block's, so a fast
//!   sweep moves smoothly and the transcendentals cost about 10 instructions per frame. A TPT all-pass is stable for
//!   any coefficient, which is why this and not a direct-form all-pass.
//! - **A change of `stages` crosses one block.** Both counts run for that block (the chain's output after the old
//!   and after the new count are both taps of the same chain) and the output blends linearly from one to the other,
//!   so no click; a removed stage's state is cleared afterwards, an added one starts from rest.
//! - **Determinism.** Per sample `+ − ×` only; the sine is the project's polynomial. No `mul_add`. State is flushed at
//!   the end of every block, the feedback sample on every write.

#![allow(
    clippy::manual_midpoint,
    reason = "0.5·(a + b) is the textbook split; f32::midpoint is wider than it needs to be and its code differs by target"
)]

use crate::chorus::sine_of;
use crate::util::flush::flush;
use crate::util::math;

/// The most stages (the catalogue's maximum).
pub const MAX_STAGES: usize = 12;
/// The largest feedback magnitude (the catalogue's 99 %).
pub const MAX_FEEDBACK: f32 = 0.99;
/// How many octaves a depth of 100 % sweeps either way.
pub const OCTAVES: f32 = 3.0;
/// The lowest corner the sweep computes, in Hz.
pub const MIN_CORNER_HZ: f32 = 1.0;
/// The highest corner as a fraction of the sample rate (21.6 kHz at 48 kHz).
pub const MAX_CORNER_FRACTION: f32 = 0.45;

/// The corner frequency of the all-passes for a sweep value `s` (`−1 … 1`): `center · 2^(3 · depth · s)`, clamped to
/// [`MIN_CORNER_HZ`] … `0.45 · sample_rate`. `depth` is a fraction (`0 … 1`). A NaN gives the minimum.
#[must_use]
pub fn corner_hz(center_hz: f32, depth: f32, sweep: f32, sample_rate: f32) -> f32 {
    let f = center_hz * math::exp2f(OCTAVES * depth * sweep);
    if f >= MIN_CORNER_HZ {
        f.min(MAX_CORNER_FRACTION * sample_rate)
    } else {
        MIN_CORNER_HZ
    }
}

/// The TPT one-pole's coefficient for a corner: `G = g/(1 + g)`, `g = tan(π·fc/fs)`, in `(0, 1)`.
#[must_use]
pub fn coefficient(corner_hz: f32, sample_rate: f32) -> f32 {
    let g = math::tanf(core::f32::consts::PI * corner_hz / sample_rate);
    g / (1.0 + g)
}

/// The notch frequencies, in Hz, of `stages` all-passes with corner `fc_hz` and no feedback, lowest first: the closed
/// form `fs/π · atan(g · tan((2k + 1)·π/(2N)))`. An odd count's last notch is Nyquist.
#[must_use]
pub fn notch_hz(stages: usize, corner_hz: f64, sample_rate: f64) -> Vec<f64> {
    use core::f64::consts::PI;
    let g = math::tan(PI * corner_hz / sample_rate);
    let n = stages as f64;
    let mut out = Vec::new();
    let mut k = 0usize;
    while (2 * k + 1) <= stages {
        let theta = (2 * k + 1) as f64 * PI / (2.0 * n);
        let f = if 2 * k + 1 == stages {
            sample_rate / 2.0
        } else {
            sample_rate / PI * libm::atan(g * math::tan(theta))
        };
        out.push(f);
        k += 1;
    }
    out
}

/// The exact complex response `(re, im)` of a static phaser at `freq_hz`, `mix` 100 %: `H = (1 + W)/2`,
/// `W = (1 − |fb|)·A / (1 − fb·e^(−jω)·A)`, `A = ((1 − j·u)/(1 + j·u))^N`, in `f64`. The closed form the tests hold
/// the core to.
#[must_use]
pub fn response(
    stages: usize,
    corner_hz: f64,
    feedback: f64,
    freq_hz: f64,
    sample_rate: f64,
) -> (f64, f64) {
    use core::f64::consts::PI;
    let fb = feedback.clamp(-f64::from(MAX_FEEDBACK), f64::from(MAX_FEEDBACK));
    let g = math::tan(PI * corner_hz / sample_rate);
    let w = 2.0 * PI * freq_hz / sample_rate;
    let u = math::tan(0.5 * w) / g;
    // (1 − ju)/(1 + ju) = e^(−2j·atan u)
    let phase = -2.0 * stages as f64 * libm::atan(u);
    let (a_re, a_im) = (math::cos(phase), math::sin(phase));
    // z⁻¹·A
    let (c, s) = (math::cos(-w), math::sin(-w));
    let (za_re, za_im) = (c * a_re - s * a_im, c * a_im + s * a_re);
    let (d_re, d_im) = (1.0 - fb * za_re, -fb * za_im);
    let den = d_re * d_re + d_im * d_im;
    let a = 1.0 - fb.abs();
    let (n_re, n_im) = (a * a_re, a * a_im);
    let (w_re, w_im) = (
        (n_re * d_re + n_im * d_im) / den,
        (n_im * d_re - n_re * d_im) / den,
    );
    ((1.0 + w_re) / 2.0, w_im / 2.0)
}

/// What one block needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The stage count at the start and at the end of the block (`1 … 12`); they differ in the block where it changes.
    pub stages: [usize; 2],
    /// How far the sweep advances over this block, in counts of 2⁻⁶⁴ cycle.
    pub phase_inc: u64,
    /// The right channel's lag behind the left ([`lag_for_width`](crate::chorus::lag_for_width)).
    pub lag: u64,
    /// The sweep's centre in Hz at the end of the block.
    pub center_hz: f32,
    /// The sweep's depth as a fraction (`0 … 1`) at the end of the block.
    pub depth: f32,
    /// The feedback as a fraction (`−0.99 … 0.99`) at the start and the end.
    pub feedback: [f32; 2],
    /// The mix as a fraction (`0 … 1`) at the start and the end.
    pub mix: [f32; 2],
}

#[inline(always)]
fn sane_fraction(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(lo, hi) }
}

/// The stereo phaser: the all-pass states, the last wet sample and the sweep.
#[derive(Clone, Debug, PartialEq)]
pub struct Phaser {
    sample_rate: f32,
    states: [[f32; MAX_STAGES]; 2],
    wet: [f32; 2],
    /// The coefficients at the end of the previous block (the ramp's start); meaningful once `primed`.
    g: [f32; 2],
    primed: bool,
    phase: u64,
}

impl Phaser {
    /// A silent phaser for `sample_rate`. It holds no heap memory.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            states: [[0.0; MAX_STAGES]; 2],
            wet: [0.0; 2],
            g: [0.0; 2],
            primed: false,
            phase: 0,
        }
    }

    /// The sample rate.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Silence everything and restart the sweep at phase 0.
    pub fn reset(&mut self) {
        *self = Self::new(self.sample_rate);
    }

    /// Wake from silence: the sweep restarts at phase 0 and the first block starts its coefficient ramp at its own
    /// end value, so what follows does not depend on how many blocks were skipped. Call it only when
    /// [`is_state_zero`](Self::is_state_zero).
    pub fn restart_from_silence(&mut self) {
        self.phase = 0;
        self.primed = false;
    }

    /// The sweep's phase (test support).
    #[must_use]
    pub fn phase(&self) -> u64 {
        self.phase
    }

    /// Whether the states and the last wet sample are exactly zero (so silent input gives silent output).
    #[must_use]
    pub fn is_state_zero(&self) -> bool {
        self.wet == [0.0; 2] && self.states.iter().all(|s| s.iter().all(|x| *x == 0.0))
    }

    /// The number of subnormal values in the state (test support).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        self.states
            .iter()
            .flatten()
            .chain(&self.wet)
            .filter(|x| x.is_subnormal())
            .count()
    }

    /// Process one block of `N` frames in place. The coefficient, feedback and mix ramp linearly per sample. At mix 0
    /// at both ends the block is not written, so the input passes bit for bit while the chain keeps running.
    pub fn process_block<const N: usize>(
        &mut self,
        seg: &Segment,
        left: &mut [f32; N],
        right: &mut [f32; N],
    ) {
        let write = seg.mix.map(|m| sane_fraction(m, 0.0, 1.0)) != [0.0, 0.0];
        if write {
            self.run::<N, true>(seg, left, right);
        } else {
            self.run::<N, false>(seg, left, right);
        }
        self.phase = self.phase.wrapping_add(seg.phase_inc);
    }

    #[inline(always)]
    fn run<const N: usize, const WRITE: bool>(
        &mut self,
        seg: &Segment,
        left: &mut [f32; N],
        right: &mut [f32; N],
    ) {
        let counts = seg.stages.map(|s| s.clamp(1, MAX_STAGES));
        let end = self.phase.wrapping_add(seg.phase_inc);
        let depth = sane_fraction(seg.depth, 0.0, 1.0);
        let g1 = [
            coefficient(
                corner_hz(seg.center_hz, depth, sine_of(end), self.sample_rate),
                self.sample_rate,
            ),
            coefficient(
                corner_hz(
                    seg.center_hz,
                    depth,
                    sine_of(end.wrapping_sub(seg.lag)),
                    self.sample_rate,
                ),
                self.sample_rate,
            ),
        ];
        let g0 = if self.primed { self.g } else { g1 };
        let fb = seg
            .feedback
            .map(|x| sane_fraction(x, -MAX_FEEDBACK, MAX_FEEDBACK));
        let mix = seg.mix.map(|x| sane_fraction(x, 0.0, 1.0));
        let inv_n = 1.0 / N as f32;
        let run = counts[0].max(counts[1]);
        let changing = counts[0] != counts[1];
        for i in 0..N {
            let r = (i + 1) as f32 * inv_n;
            let f = fb[0] + (fb[1] - fb[0]) * r;
            let a = 1.0 - f.abs();
            let m = mix[0] + (mix[1] - mix[0]) * r;
            let xs = [left[i], right[i]];
            let mut outs = [0.0f32; 2];
            for ch in 0..2 {
                let g = g0[ch] + (g1[ch] - g0[ch]) * r;
                let s = &mut self.states[ch];
                let mut y = a * xs[ch] + f * self.wet[ch];
                let (mut tap_old, mut tap_new) = (y, y);
                for (k, sk) in s.iter_mut().enumerate().take(run) {
                    let v = (y - *sk) * g;
                    let lp = v + *sk;
                    *sk = lp + v;
                    y = 2.0 * lp - y;
                    if k + 1 == counts[0] {
                        tap_old = y;
                    }
                    if k + 1 == counts[1] {
                        tap_new = y;
                    }
                }
                let wet = if changing {
                    (1.0 - r) * tap_old + r * tap_new
                } else {
                    tap_new
                };
                self.wet[ch] = flush(wet);
                outs[ch] = (1.0 - m) * xs[ch] + m * (0.5 * (xs[ch] + wet));
            }
            if WRITE {
                left[i] = outs[0];
                right[i] = outs[1];
            }
        }
        // Flush the states once per block; a removed stage starts from rest if it comes back.
        for s in &mut self.states {
            for (k, x) in s.iter_mut().enumerate() {
                *x = if k < counts[1] { flush(*x) } else { 0.0 };
            }
        }
        self.g = g1;
        self.primed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chorus::lag_for_width;
    use crate::util::rng::Rng;

    const N: usize = 32;
    const SR: f32 = 48_000.0;

    fn sq(x: f64) -> f64 {
        x * x
    }

    fn seg(stages: usize, center: f32, depth: f32, fb: f32, mix: f32) -> Segment {
        Segment {
            stages: [stages; 2],
            phase_inc: 0,
            lag: 0,
            center_hz: center,
            depth,
            feedback: [fb; 2],
            mix: [mix; 2],
        }
    }

    fn run(p: &mut Phaser, seg: &Segment, left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
        assert!(left.len().is_multiple_of(N) && left.len() == right.len());
        let (mut ol, mut or) = (left.to_vec(), right.to_vec());
        for (l, r) in ol
            .as_chunks_mut::<N>()
            .0
            .iter_mut()
            .zip(or.as_chunks_mut::<N>().0.iter_mut())
        {
            p.process_block::<N>(seg, l, r);
        }
        (ol, or)
    }

    fn impulse(len: usize) -> Vec<f32> {
        let mut x = vec![0.0; len];
        x[0] = 1.0;
        x
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

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    #[test]
    fn the_response_matches_the_closed_form_for_every_stage_count() {
        for stages in 1..=MAX_STAGES {
            for &(fc, fb) in &[
                (1_000.0f32, 0.0f32),
                (250.0, 0.0),
                (6_000.0, 0.5),
                (1_000.0, -0.9),
                (80.0, 0.99),
                (3_000.0, -0.99),
            ] {
                // The loop's time constant is its group delay (N · 2/ω_c + 1 samples) over 1 − |fb|; the impulse
                // response is cut at 14 of them (e^-14), and a case whose window would pass 2^18 is left out.
                let wc = 2.0 * core::f64::consts::PI * f64::from(fc) / f64::from(SR);
                let tc = (stages as f64 * 2.0 / wc + 1.0) / (1.0 - f64::from(fb.abs()));
                let len = ((14.0 * tc) as usize).next_power_of_two().max(8192);
                if len > 1 << 18 {
                    continue;
                }
                let mut p = Phaser::new(SR);
                let x = impulse(len);
                let (y, _) = run(&mut p, &seg(stages, fc, 0.0, fb, 1.0), &x, &x);
                let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
                for &f in &[40.0f64, 300.0, 1_000.0, 2_900.0, 7_000.0, 15_000.0] {
                    let (re, im) = dft(&h, f, f64::from(SR));
                    let (cre, cim) =
                        response(stages, f64::from(fc), f64::from(fb), f, f64::from(SR));
                    let err = (sq(re - cre) + sq(im - cim)).sqrt();
                    assert!(
                        err < 2e-4,
                        "{stages} stages fc {fc} fb {fb} at {f} Hz: {err}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_notches_of_a_static_phaser_are_the_all_pass_chains_closed_form() {
        // Measure the deepest points of the impulse response's spectrum and compare with the closed form: within 2 %
        // is the acceptance, and the core is within a small fraction of that.
        let len = 16_384;
        let mut worst: f64 = 0.0;
        for stages in 2..=MAX_STAGES {
            for &fc in &[150.0f32, 1_000.0, 4_000.0] {
                let mut p = Phaser::new(SR);
                let x = impulse(len);
                let (y, _) = run(&mut p, &seg(stages, fc, 0.0, 0.0, 1.0), &x, &x);
                let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
                for want in notch_hz(stages, f64::from(fc), f64::from(SR)) {
                    if want > 20_000.0 {
                        continue; // Nyquist's half notch
                    }
                    // Scan a log grid ±15 % round the prediction for the deepest point, then refine it.
                    let (mut best, mut best_f) = (f64::INFINITY, want);
                    let mut f = want * 0.85;
                    while f < want * 1.15 {
                        let (re, im) = dft(&h[..8192], f, f64::from(SR));
                        let m = libm::hypot(re, im);
                        if m < best {
                            best = m;
                            best_f = f;
                        }
                        f += want * 0.001;
                    }
                    let err = (best_f / want - 1.0).abs();
                    worst = worst.max(err);
                    assert!(
                        err < 0.02,
                        "{stages} stages fc {fc}: notch {want:.1} Hz measured {best_f:.1}"
                    );
                    assert!(best < 0.02, "the notch is deep: {best}");
                }
            }
        }
        assert!(
            worst < 0.005,
            "the closed form is much tighter than 2 %: {worst}"
        );
    }

    #[test]
    fn the_analog_corner_rule_holds_for_a_low_corner() {
        // Below fs/20 the digital notches are the analog ones, fc · tan((2k + 1)π/(2N)).
        let n = notch_hz(6, 200.0, 48_000.0);
        let analog =
            |k: usize| 200.0 * math::tan((2 * k + 1) as f64 * core::f64::consts::PI / 12.0);
        for (k, f) in n.iter().take(2).enumerate() {
            assert!(
                (f / analog(k) - 1.0).abs() < 0.01,
                "{k}: {f} vs {}",
                analog(k)
            );
        }
        assert_eq!(notch_hz(6, 1000.0, 48_000.0).len(), 3);
        assert_eq!(notch_hz(5, 1000.0, 48_000.0).len(), 3);
        assert_eq!(notch_hz(5, 1000.0, 48_000.0)[2], 24_000.0);
        assert_eq!(notch_hz(2, 1000.0, 48_000.0).len(), 1);
    }

    #[test]
    fn the_level_is_bounded_at_every_feedback_and_stage_count() {
        let mut worst: f64 = 0.0;
        for stages in 1..=MAX_STAGES {
            for &fb in &[0.0, 0.3, 0.9, 0.99, -0.3, -0.9, -0.99] {
                for &fc in &[5.0, 50.0, 1_000.0, 8_000.0, 20_000.0] {
                    for j in 0..=2400 {
                        let f = 24_000.0 * f64::from(j) / 2400.0;
                        let (re, im) = response(stages, fc, fb, f, 48_000.0);
                        worst = worst.max(libm::hypot(re, im));
                    }
                }
            }
        }
        assert!(worst <= 1.0 + 1e-9, "peak gain {worst}");
        assert!(worst > 0.999);
    }

    #[test]
    fn white_noise_never_gains_level_at_the_extreme_feedbacks() {
        let mut rng = Rng::new(3);
        let x: Vec<f32> = (0..96_000).map(|_| rng.bipolar_f32()).collect();
        let e_in: f64 = x.iter().map(|v| sq(f64::from(*v))).sum();
        for stages in [2usize, 5, 12] {
            for fb in [0.99f32, -0.99, 0.0] {
                let mut p = Phaser::new(SR);
                let mut s = seg(stages, 1_000.0, 1.0, fb, 1.0);
                s.phase_inc = 1 << 52;
                s.lag = lag_for_width(100.0);
                let (l, r) = run(&mut p, &s, &x, &x);
                for ch in [&l, &r] {
                    let e: f64 = ch.iter().map(|v| sq(f64::from(*v))).sum();
                    assert!(e <= e_in * 1.002, "{stages} stages fb {fb}: {e} vs {e_in}");
                    assert!(ch.iter().all(|v| v.is_finite() && v.abs() < 4.0));
                }
            }
        }
    }

    #[test]
    fn the_corner_sweeps_three_octaves_either_way_at_full_depth() {
        let c = |s: f32, d: f32| corner_hz(1_000.0, d, s, SR);
        assert_eq!(c(0.0, 1.0), 1_000.0);
        assert!((c(1.0, 1.0) - 8_000.0).abs() < 0.01);
        assert!((c(-1.0, 1.0) - 125.0).abs() < 0.01);
        assert!((c(1.0, 0.5) / 1_000.0 - 2.828_427).abs() < 1e-3); // 2^1.5
        assert_eq!(c(1.0, 0.0), 1_000.0);
        // The ends are clamped; a NaN falls to the minimum.
        assert!((corner_hz(10_000.0, 1.0, 1.0, SR) - 0.45 * SR).abs() < 1e-3);
        assert_eq!(corner_hz(50.0, 1.0, -1.0, SR), 50.0 / 8.0);
        assert!((corner_hz(f32::NAN, 1.0, 1.0, SR) - MIN_CORNER_HZ).abs() < 1e-6);
        let g = coefficient(1_000.0, SR);
        let t = math::tan(core::f64::consts::PI * 1_000.0 / 48_000.0);
        assert!((f64::from(g) - t / (1.0 + t)).abs() < 1e-7);
    }

    #[test]
    fn the_notches_move_with_the_sweep() {
        // At a quarter of the cycle the sine is +1, so a depth of 100 % puts every corner three octaves up: the
        // measured notches are the closed form at 8 × the centre. Freeze the sweep there.
        let mut p = Phaser::new(SR);
        p.phase = (1 << 62) - 1; // the block's end phase is one count later: s = 1
        let mut s = seg(6, 500.0, 1.0, 0.0, 1.0);
        s.phase_inc = 1;
        let x = impulse(8192);
        let (y, _) = run(&mut p, &s, &x, &x);
        let h: Vec<f64> = y.iter().map(|v| f64::from(*v)).collect();
        // The ramp from the first block's start (primed at its end value) is flat, so every block has the same corner.
        for want in notch_hz(6, 4_000.0, 48_000.0) {
            let (mut best, mut best_f) = (f64::INFINITY, want);
            let mut f = want * 0.9;
            while f < want * 1.1 {
                let (re, im) = dft(&h, f, 48_000.0);
                if libm::hypot(re, im) < best {
                    best = libm::hypot(re, im);
                    best_f = f;
                }
                f += want * 0.001;
            }
            assert!((best_f / want - 1.0).abs() < 0.01, "{best_f} vs {want}");
        }
    }

    #[test]
    fn the_dry_signal_is_never_delayed() {
        // The first output sample is the half sum of the input and the chain's first sample (the all-pass's direct term).
        let mut p = Phaser::new(SR);
        let x = impulse(64);
        let (y, _) = run(&mut p, &seg(6, 1_000.0, 0.0, 0.0, 1.0), &x, &x);
        assert!(y[0] > 0.0 && y[0] < 1.0, "{}", y[0]);
        let g = f64::from(coefficient(1_000.0, SR));
        // An all-pass stage's direct term is 2G − 1 for the TPT form: the chain's first sample is its sixth power.
        let d = 2.0 * g - 1.0;
        let want = 0.5 * (1.0 + sq(d * d * d));
        assert!((f64::from(y[0]) - want).abs() < 1e-6, "{} vs {want}", y[0]);
    }

    #[test]
    fn mix_zero_leaves_the_block_alone_while_the_chain_runs() {
        let mut rng = Rng::new(5);
        let x: Vec<f32> = (0..4096).map(|_| rng.bipolar_f32()).collect();
        let mut s = seg(7, 800.0, 0.7, 0.6, 0.0);
        s.phase_inc = 1 << 55;
        let mut a = Phaser::new(SR);
        let (y0, _) = run(&mut a, &s, &x, &x);
        assert_eq!(bits(&y0), bits(&x), "mix 0 passes the input bit for bit");
        s.mix = [1.0; 2];
        let next: Vec<f32> = (0..4096).map(|_| rng.bipolar_f32()).collect();
        let (wa, _) = run(&mut a, &s, &next, &next);
        let mut b = Phaser::new(SR);
        s.mix = [1.0; 2];
        let _ = run(&mut b, &s, &x, &x);
        let (wb, _) = run(&mut b, &s, &next, &next);
        assert_eq!(bits(&wa), bits(&wb));
        let mut c = Phaser::new(SR);
        let mut z = [-0.0f32; N];
        let mut z2 = z;
        c.process_block::<N>(&seg(6, 1_000.0, 0.5, 0.3, 0.0), &mut z, &mut z2);
        assert_eq!(z[0].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn the_right_channel_lags_the_left_and_width_zero_is_mono() {
        let mut rng = Rng::new(8);
        let x: Vec<f32> = (0..8192).map(|_| rng.bipolar_f32()).collect();
        let mut s = seg(6, 1_000.0, 1.0, 0.4, 1.0);
        s.phase_inc = 1 << 54;
        let mut mono = Phaser::new(SR);
        let (l, r) = run(&mut mono, &s, &x, &x);
        assert_eq!(bits(&l), bits(&r), "no lag: both channels alike");
        s.lag = lag_for_width(100.0);
        let mut wide = Phaser::new(SR);
        let (l2, r2) = run(&mut wide, &s, &x, &x);
        assert_eq!(
            bits(&l),
            bits(&l2),
            "the left channel does not move with the lag"
        );
        assert_ne!(bits(&r2), bits(&l2));
    }

    #[test]
    fn a_change_of_stage_count_crosses_one_block_without_a_click() {
        let n = 4800;
        let sine: Vec<f32> = (0..n)
            .map(|i| 0.5 * math::sinf(2.0 * core::f32::consts::PI * 200.0 * i as f32 / SR))
            .collect();
        let jump = |y: &[f32]| {
            y.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let mut steady = Phaser::new(SR);
        let (ys, _) = run(&mut steady, &seg(6, 1_000.0, 0.0, 0.0, 1.0), &sine, &sine);
        let mut p = Phaser::new(SR);
        let (head, _) = run(
            &mut p,
            &seg(6, 1_000.0, 0.0, 0.0, 1.0),
            &sine[..2400],
            &sine[..2400],
        );
        let mut change = seg(6, 1_000.0, 0.0, 0.0, 1.0);
        change.stages = [6, 7];
        let (mid, _) = run(
            &mut p,
            &change,
            &sine[2400..2400 + N],
            &sine[2400..2400 + N],
        );
        let (tail, _) = run(
            &mut p,
            &seg(7, 1_000.0, 0.0, 0.0, 1.0),
            &sine[2400 + N..],
            &sine[2400 + N..],
        );
        let all: Vec<f32> = head.iter().chain(&mid).chain(&tail).copied().collect();
        let (steady_jump, switch_jump) = (jump(&ys), jump(&all));
        assert!(
            switch_jump < 2.0 * steady_jump + 0.02,
            "{switch_jump} against {steady_jump}"
        );
        // Without the crossfade the added stage's rest state is a step: show that a hard switch would be much worse.
        let mut hard = Phaser::new(SR);
        let _ = run(
            &mut hard,
            &seg(6, 1_000.0, 0.0, 0.0, 1.0),
            &sine[..2400],
            &sine[..2400],
        );
        let (h2, _) = run(
            &mut hard,
            &seg(7, 1_000.0, 0.0, 0.0, 1.0),
            &sine[2400..],
            &sine[2400..],
        );
        assert!(
            jump(&h2) > switch_jump,
            "the blend helps: {} against {switch_jump}",
            jump(&h2)
        );
        // The removed stage is cleared and the state stays zero-able.
        let mut q = Phaser::new(SR);
        let mut down = seg(12, 1_000.0, 0.0, 0.0, 1.0);
        let _ = run(&mut q, &down, &sine[..960], &sine[..960]);
        down.stages = [12, 3];
        let _ = run(&mut q, &down, &sine[960..960 + N], &sine[960..960 + N]);
        assert!(q.states.iter().all(|s| s[3..].iter().all(|x| *x == 0.0)));
    }

    #[test]
    fn two_phasers_keep_separate_state_and_the_channels_are_independent() {
        let mut rng = Rng::new(21);
        let xl: Vec<f32> = (0..2048).map(|_| rng.bipolar_f32()).collect();
        let xr = vec![0.0f32; 2048];
        let mut s = seg(6, 1_000.0, 0.8, 0.5, 1.0);
        s.phase_inc = 1 << 54;
        let (mut a, mut b) = (Phaser::new(SR), Phaser::new(SR));
        let (al, ar) = run(&mut a, &s, &xl, &xr);
        let (bl, _) = run(&mut b, &s, &xl, &xl);
        assert_eq!(bits(&al), bits(&bl));
        assert!(ar.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_tail_decays_to_exact_zero_without_subnormals() {
        // The slowest case: a 12-stage chain at a corner of 6 Hz (center 50 Hz, three octaves down) with feedback.
        let mut p = Phaser::new(SR);
        let x = impulse(64);
        let mut s = seg(12, 50.0, 1.0, 0.5, 1.0);
        s.phase_inc = 3 << 59; // freeze near the bottom of the sweep: s = −1 at 3/4 of the cycle
        p.phase = 3 << 62;
        s.phase_inc = 0;
        let _ = run(&mut p, &s, &x, &x);
        let silence = vec![0.0f32; 48_000];
        let mut zero_at = None;
        for second in 0..40 {
            let (l, r) = run(&mut p, &s, &silence, &silence);
            assert!(
                l.iter()
                    .chain(&r)
                    .all(|v| !v.is_subnormal() && v.is_finite())
            );
            assert_eq!(p.count_subnormals(), 0);
            if zero_at.is_none() && p.is_state_zero() {
                zero_at = Some(second);
            }
        }
        assert!(
            zero_at.is_some(),
            "the state flushes to exactly zero within 40 s"
        );
    }

    #[test]
    fn nan_parameters_fall_back() {
        let mut p = Phaser::new(SR);
        let mut s = seg(6, f32::NAN, f32::NAN, f32::NAN, 1.0);
        s.stages = [0, 99];
        let x = vec![0.5f32; 256];
        let (l, _) = run(&mut p, &s, &x, &x);
        assert!(l.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn restart_from_silence_forgets_the_sweep_and_the_ramp() {
        let mut p = Phaser::new(SR);
        let x = vec![0.1f32; 256];
        let mut s = seg(6, 1_000.0, 1.0, 0.0, 1.0);
        s.phase_inc = 1 << 55;
        let _ = run(&mut p, &s, &x, &x);
        assert_ne!(p.phase(), 0);
        p.reset();
        assert_eq!(p.phase(), 0);
        assert!(p.is_state_zero());
        let mut q = Phaser::new(SR);
        q.phase = 12345;
        q.restart_from_silence();
        let (a, _) = run(&mut p, &s, &x, &x);
        let (b, _) = run(&mut q, &s, &x, &x);
        assert_eq!(bits(&a), bits(&b));
    }
}
