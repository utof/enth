//! LFO core: a 64-bit phase accumulator, seven shapes, seeded sample-and-hold and smooth random (P1a lane D2).
//! Notes: `docs/dsp/lfo.md`.
//!
//! The phase is an integer. The top 32 bits are the phase of the cycle (the "u32 phase" of report 02) and the low
//! 32 bits carry the fraction of an increment that a 32-bit accumulator would lose, so the rate error is the `f64`
//! rounding of the increment (about 1e-16) and not 1e-5 at the lowest rate: a 32-bit accumulator at 0.01 Hz with
//! 32-frame steps has an increment of 28,633.1 counts, whose rounding alone is up to 1.7e-5. The phase never drifts,
//! never goes inexact, and is the same on every platform.
//!
//! The LFO is stepped once per control step: [`Lfo::tick`] advances the phase by one increment, then evaluates the
//! shape at the new phase, so the value it returns is the value at the *end* of the step (what the engine's module
//! contract asks of a modulator). A shape is evaluated from the top 24 bits of the phase, `x ∈ [0, 1)`, exactly
//! representable in `f32`.
//!
//! Shapes (all bipolar, −1..1; the module maps to unipolar):
//!
//! | shape | `x = 0` | formula |
//! | --- | --- | --- |
//! | sine | 0 rising | `sin(2π·x)` |
//! | triangle | 0 rising, in phase with the sine | `4x`, `2 − 4x`, `4x − 4` on quarters |
//! | saw up | −1 | `2x − 1` |
//! | saw down | +1 | `1 − 2x` |
//! | square | +1 | `+1` for `x < ½`, else `−1` |
//! | sample & hold | the cycle's drawn value | one new random value per cycle |
//! | smooth random | the cycle's drawn value | smoothstep from this cycle's value to the next |
//!
//! Randomness comes only from the [`Rng`] passed to [`Lfo::new`] (the engine derives its seed from the render seed,
//! the module id and the note index, or from the module's own `seed` in the seeded mode), never from the patch.
//! The draw order is fixed: the start phase if random, then the cycle's value `a`, then the next cycle's `b`, then
//! one draw per wrap.

use crate::util::math::sinf;
use crate::util::rng::Rng;

/// 2⁶⁴ as an `f64`.
const TWO_POW_64: f64 = 4_294_967_296.0 * 4_294_967_296.0;

/// 2⁻²⁴.
const INV_2_24: f32 = 1.0 / 16_777_216.0;

/// The LFO waveform, in the catalogue's order (`LfoShape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Sine.
    Sine,
    /// Triangle, in phase with the sine.
    Triangle,
    /// Rising ramp.
    SawUp,
    /// Falling ramp.
    SawDown,
    /// Square.
    Square,
    /// A new random value each cycle, held.
    SampleHold,
    /// Random values joined by smoothstep curves.
    SmoothRandom,
}

/// Where the cycle starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StartPhase {
    /// At this many cycles (`0.25` is a quarter turn); the fractional part is used.
    Cycles(f64),
    /// A uniformly random phase from the seed.
    Random,
}

/// The phase increment of one control step of `step` frames at `rate_hz`: `round(rate · step / fs · 2⁶⁴)`,
/// saturating at 2⁶⁴ − 1 for absurd rates. Not finite or non-positive rates stop the LFO.
#[must_use]
pub fn phase_increment(rate_hz: f32, sample_rate: f32, step: usize) -> u64 {
    let cycles = f64::from(rate_hz) * step as f64 / f64::from(sample_rate);
    if cycles > 0.0 && cycles.is_finite() {
        // `as` saturates, and f64 rounds the product to 53 bits.
        (cycles * TWO_POW_64 + 0.5) as u64
    } else {
        0
    }
}

/// One LFO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    phase: u64,
    inc: u64,
    rate_bits: u32,
    a: f32,
    b: f32,
    rng: Rng,
}

impl Lfo {
    /// An LFO that starts at `start`, drawing its randomness from `seed`. The rate is zero until [`set_rate`](
    /// Self::set_rate).
    #[must_use]
    pub fn new(seed: u64, start: StartPhase) -> Self {
        let mut rng = Rng::new(seed);
        let phase = match start {
            StartPhase::Cycles(c) => {
                let f = c - c.floor();
                // `f` is in [0, 1); the product is below 2⁶⁴ except for rounding right under 1.0.
                (f * TWO_POW_64) as u64
            }
            StartPhase::Random => u64::from(rng.next_u32()) << 32,
        };
        let a = rng.bipolar_f32();
        let b = rng.bipolar_f32();
        Self {
            phase,
            inc: 0,
            rate_bits: 0,
            a,
            b,
            rng,
        }
    }

    /// Set the rate, recomputing the increment only when it changed.
    #[inline]
    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32, step: usize) {
        let bits = rate_hz.to_bits();
        if bits != self.rate_bits || self.inc == 0 {
            self.rate_bits = bits;
            self.inc = phase_increment(rate_hz, sample_rate, step);
        }
    }

    /// The raw 64-bit phase (test support): the cycle position is `phase / 2⁶⁴`.
    #[must_use]
    pub fn phase(&self) -> u64 {
        self.phase
    }

    /// The phase increment per step.
    #[must_use]
    pub fn increment(&self) -> u64 {
        self.inc
    }

    /// Advance one control step and return the bipolar value (−1..1) of `shape` at the new phase. When the phase
    /// wraps, the cycle's random value moves on (one draw).
    #[inline]
    pub fn tick(&mut self, shape: Shape) -> f32 {
        let (p, wrapped) = self.phase.overflowing_add(self.inc);
        self.phase = p;
        if wrapped {
            self.a = self.b;
            self.b = self.rng.bipolar_f32();
        }
        self.value(shape)
    }

    /// The value of `shape` at the present phase, without advancing.
    #[inline]
    #[must_use]
    pub fn value(&self, shape: Shape) -> f32 {
        // The top 24 bits of the phase: exact in f32, in [0, 1).
        let x = (self.phase >> 40) as u32 as f32 * INV_2_24;
        match shape {
            Shape::Sine => sinf(core::f32::consts::TAU * x),
            Shape::Triangle => {
                if x < 0.25 {
                    4.0 * x
                } else if x < 0.75 {
                    2.0 - 4.0 * x
                } else {
                    4.0 * x - 4.0
                }
            }
            Shape::SawUp => 2.0 * x - 1.0,
            Shape::SawDown => 1.0 - 2.0 * x,
            Shape::Square => {
                if x < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Shape::SampleHold => self.a,
            Shape::SmoothRandom => {
                let s = x * x * (3.0 - 2.0 * x);
                self.a + (self.b - self.a) * s
            }
        }
    }

    /// Whether the LFO's `f32` state holds a subnormal (test support; it never should).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.a.is_subnormal() || self.b.is_subnormal()
    }
}

/// Map a bipolar value to 0..1.
#[inline(always)]
#[must_use]
pub fn unipolar(y: f32) -> f32 {
    0.5 + 0.5 * y
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn lfo_at(rate: f32, start: StartPhase, seed: u64) -> Lfo {
        let mut l = Lfo::new(seed, start);
        l.set_rate(rate, SR, 32);
        l
    }

    #[test]
    fn the_increment_is_exact_for_binary_fractions() {
        // 1500 Hz at 48 kHz with 32-frame steps is exactly one cycle per step: 2⁶⁴ saturates; 750 Hz is one half.
        assert_eq!(phase_increment(750.0, SR, 32), 1 << 63);
        assert_eq!(phase_increment(375.0, SR, 32), 1 << 62);
        assert_eq!(phase_increment(0.0, SR, 32), 0);
        assert_eq!(phase_increment(-1.0, SR, 32), 0);
        assert_eq!(phase_increment(f32::NAN, SR, 32), 0);
        assert_eq!(phase_increment(f32::INFINITY, SR, 32), 0);
        assert_eq!(phase_increment(1.0e9, SR, 32), u64::MAX);
    }

    #[test]
    fn rate_error_over_60_seconds_is_far_below_1e_5() {
        // 60 s is 90,000 steps. Total cycles = wraps + phase fraction, compared with rate · 60.
        for rate in [0.01f32, 0.037, 0.5, 2.0, 5.0, 13.3, 50.0] {
            let mut l = lfo_at(rate, StartPhase::Cycles(0.0), 1);
            let mut wraps = 0u64;
            let mut last = l.phase();
            for _ in 0..90_000 {
                let _ = l.tick(Shape::Sine);
                if l.phase() < last {
                    wraps += 1;
                }
                last = l.phase();
            }
            let cycles = wraps as f64 + l.phase() as f64 / TWO_POW_64;
            let want = f64::from(rate) * 60.0;
            let err = (cycles - want).abs() / want;
            assert!(err < 1e-9, "{rate} Hz: {cycles} vs {want} ({err:e})");
        }
        // A 32-bit accumulator with a rounded increment would not make 1e-5 at 0.01 Hz: the old design's error.
        let inc32 = (0.01f64 * 32.0 / 48_000.0 * 4_294_967_296.0).round();
        let err32 = (inc32 / (0.01 * 32.0 / 48_000.0 * 4_294_967_296.0) - 1.0).abs();
        assert!(err32 < 2e-5);
    }

    #[test]
    fn shapes_hit_their_landmarks() {
        let at = |c: f64, shape: Shape| Lfo::new(1, StartPhase::Cycles(c)).value(shape);
        assert!(at(0.0, Shape::Sine).abs() < 1e-6);
        assert!((at(0.25, Shape::Sine) - 1.0).abs() < 1e-6);
        assert!((at(0.75, Shape::Sine) + 1.0).abs() < 1e-6);
        assert_eq!(at(0.0, Shape::Triangle), 0.0);
        assert_eq!(at(0.25, Shape::Triangle), 1.0);
        assert_eq!(at(0.5, Shape::Triangle), 0.0);
        assert_eq!(at(0.75, Shape::Triangle), -1.0);
        assert_eq!(at(0.0, Shape::SawUp), -1.0);
        assert_eq!(at(0.5, Shape::SawUp), 0.0);
        assert_eq!(at(0.0, Shape::SawDown), 1.0);
        assert_eq!(at(0.25, Shape::SawDown), 0.5);
        assert_eq!(at(0.49, Shape::Square), 1.0);
        assert_eq!(at(0.5, Shape::Square), -1.0);
        // The triangle is in phase with the sine: same sign everywhere, equal at the quarter points.
        for i in 0..64 {
            let c = f64::from(i) / 64.0;
            let (s, t) = (at(c, Shape::Sine), at(c, Shape::Triangle));
            assert!(s * t >= -1e-6, "{c}: {s} {t}");
        }
        // Every deterministic shape stays in range.
        for shape in [
            Shape::Sine,
            Shape::Triangle,
            Shape::SawUp,
            Shape::SawDown,
            Shape::Square,
        ] {
            for i in 0..1000 {
                let y = at(f64::from(i) / 1000.0, shape);
                assert!((-1.0..=1.0).contains(&y), "{shape:?} {y}");
            }
        }
        // A full turn is zero turns.
        assert_eq!(
            Lfo::new(1, StartPhase::Cycles(1.0)).phase(),
            Lfo::new(1, StartPhase::Cycles(0.0)).phase()
        );
    }

    #[test]
    fn tick_returns_the_value_at_the_end_of_the_step() {
        // 375 Hz is a quarter turn per step at 48 kHz: the first sine value is at x = 0.25.
        let mut l = lfo_at(375.0, StartPhase::Cycles(0.0), 1);
        assert!((l.tick(Shape::Sine) - 1.0).abs() < 1e-6);
        assert!(l.tick(Shape::Sine).abs() < 1e-5);
        assert!((l.tick(Shape::Sine) + 1.0).abs() < 1e-6);
        assert!(l.tick(Shape::Sine).abs() < 1e-5);
        assert_eq!(l.phase(), 0, "four quarter turns return to the start");
    }

    #[test]
    fn sample_and_hold_draws_once_per_cycle_from_the_seed() {
        // 187.5 Hz is exactly 1/8 of a turn per 32-frame step at 48 kHz: a wrap every eighth step.
        let rate = 187.5;
        let draw = |seed: u64| {
            let mut l = lfo_at(rate, StartPhase::Cycles(0.0), seed);
            (0..100)
                .map(|_| l.tick(Shape::SampleHold))
                .collect::<Vec<_>>()
        };
        let a = draw(1);
        let b = draw(1);
        let c = draw(2);
        assert_eq!(a, b, "same seed, same values");
        assert_ne!(a, c, "another seed, other values");
        let mut distinct = Vec::new();
        for w in a.windows(2) {
            if w[0] != w[1] {
                distinct.push(w[1]);
            }
        }
        assert_eq!(
            distinct.len(),
            12,
            "{a:?}: a change at each of the twelve wraps in 100 steps"
        );
        assert!(a.iter().all(|v| (-1.0..1.0).contains(v)));
        // The first value is the first draw of the stream (no start-phase draw in the Cycles mode).
        let mut r = Rng::new(1);
        assert_eq!(a[0], r.bipolar_f32());
    }

    #[test]
    fn smooth_random_is_continuous_and_passes_through_the_drawn_values() {
        let mut l = lfo_at(93.75, StartPhase::Cycles(0.0), 9); // exactly 1/16 of a turn per step
        let ys: Vec<f32> = (0..400).map(|_| l.tick(Shape::SmoothRandom)).collect();
        let mut worst = 0.0f32;
        for w in ys.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
        // 16 steps per cycle and values within ±1: a smoothstep moves at most 1.5·Δ/16 per step.
        assert!(worst < 2.0 * 1.5 / 16.0 + 1e-3, "{worst}");
        assert!(ys.iter().all(|y| (-1.0..=1.0).contains(y)));
        // At a wrap the curve ends on the cycle's second draw, which then becomes the first.
        let mut r = Rng::new(9);
        let (_a, b) = (r.bipolar_f32(), r.bipolar_f32());
        assert!((ys[15] - b).abs() < 1e-5, "{} vs {b}", ys[15]);
    }

    #[test]
    fn a_random_start_phase_comes_from_the_seed_alone() {
        let p = |seed| Lfo::new(seed, StartPhase::Random).phase();
        assert_eq!(p(5), p(5));
        assert_ne!(p(5), p(6));
        let mut r = Rng::new(5);
        assert_eq!(p(5), u64::from(r.next_u32()) << 32);
        // And not from the rate: setting it changes nothing about the start.
        let mut l = Lfo::new(5, StartPhase::Random);
        let before = l.phase();
        l.set_rate(7.0, SR, 32);
        assert_eq!(l.phase(), before);
    }

    #[test]
    fn set_rate_caches_and_a_rate_change_keeps_the_phase() {
        let mut l = lfo_at(2.0, StartPhase::Cycles(0.3), 1);
        let inc = l.increment();
        l.set_rate(2.0, SR, 32);
        assert_eq!(l.increment(), inc);
        let ph = l.phase();
        l.set_rate(4.0, SR, 32);
        assert_eq!(l.phase(), ph, "continuous phase");
        assert!(l.increment() > inc);
        // A zero rate stops it and the cache does not hide a later change.
        l.set_rate(0.0, SR, 32);
        assert_eq!(l.increment(), 0);
        let _ = l.tick(Shape::Sine);
        assert_eq!(l.phase(), ph);
        l.set_rate(2.0, SR, 32);
        assert_eq!(l.increment(), inc);
    }

    #[test]
    fn unipolar_maps_minus_one_one_to_zero_one() {
        assert_eq!(unipolar(-1.0), 0.0);
        assert_eq!(unipolar(0.0), 0.5);
        assert_eq!(unipolar(1.0), 1.0);
    }

    /// FNV-1a over the bit patterns of 20 s of control-rate output, per shape: pinned so a change is deliberate.
    fn fnv(xs: &[f32]) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for x in xs {
            for b in x.to_bits().to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn output_hashes_are_pinned_per_shape() {
        let mut got = Vec::new();
        for shape in [
            Shape::Sine,
            Shape::Triangle,
            Shape::SawUp,
            Shape::SawDown,
            Shape::Square,
            Shape::SampleHold,
            Shape::SmoothRandom,
        ] {
            let mut l = lfo_at(3.7, StartPhase::Random, 0x0E17_5EED);
            let ys: Vec<f32> = (0..30_000).map(|_| l.tick(shape)).collect();
            got.push(fnv(&ys));
        }
        assert_eq!(got, PINNED_HASHES, "{got:#018x?}");
    }

    const PINNED_HASHES: [u64; 7] = [
        0xf248e911e47bc7cb,
        0x76a67c25ddb74d2c,
        0x00048aff6becefb7,
        0x9b1b53abc829f7b7,
        0x085a2e107e685fa5,
        0x30d3d952d5550e6e,
        0xa28108fb0ef3e6a3,
    ];
}
