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
//!
//! **Tempo sync and one-shot (S2a, lane TMP).** [`Lfo::set_synced`] sets the increment from a cycle length in beats
//! and a tempo instead of a rate in Hz ([`synced_increment`]): the same accumulator, so the phase is exact in beats
//! (the only error is the `f64` rounding of the increment) and continuous across a tempo change, which the engine
//! applies at a segment boundary. [`Lfo::set_one_shot`] makes the LFO run exactly one cycle from the phase it had when
//! it was armed, then hold the value it reached at the end of the cycle (the left limit at the start phase: a saw
//! up holds +1, not the −1 it would wrap to). The unsynced, free-running path is the one above, bit for bit.

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
    cycles_to_increment(f64::from(rate_hz) * step as f64 / f64::from(sample_rate))
}

/// `round(cycles · 2⁶⁴)` for a number of cycles per step; 0 for anything not finite or not positive.
fn cycles_to_increment(cycles: f64) -> u64 {
    if cycles > 0.0 && cycles.is_finite() {
        // `as` saturates, and f64 rounds the product to 53 bits.
        (cycles * TWO_POW_64 + 0.5) as u64
    } else {
        0
    }
}

/// The phase increment of one control step of `step` frames when one cycle lasts `num / den` beats at `bpm` beats
/// per minute: `round(bpm / 60 · (den / num) · step / fs · 2⁶⁴)`. A quarter note is `(1, 1)`, a dotted eighth `(3, 4)`.
/// The cycle length stays a ratio of integers until the one division, so the increment is the `f64` rounding of the
/// exact value (about 1e-16 relative), and the phase after any number of steps at one tempo is exact in beats. A
/// tempo that is not finite or not positive stops the LFO, as a non-positive rate does.
#[must_use]
pub fn synced_increment(bpm: f64, cycle_beats: (u32, u32), sample_rate: f32, step: usize) -> u64 {
    let (num, den) = cycle_beats;
    if num == 0 {
        return 0;
    }
    let cycles =
        bpm * f64::from(den) * step as f64 / (60.0 * f64::from(num) * f64::from(sample_rate));
    cycles_to_increment(cycles)
}

/// What the cached increment was computed from, so a repeated setting costs a comparison and not a division.
#[derive(Clone, Copy, Debug, PartialEq)]
enum IncKey {
    /// Nothing yet.
    None,
    /// A rate: the bits of the `f32` Hz value.
    Hz(u32),
    /// A tempo and a cycle length: the bits of the `f64` BPM and the cycle in beats.
    Beats(u64, (u32, u32)),
}

/// The one-shot state.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shot {
    /// Free-running (the default, and the only state of an unarmed LFO).
    Off,
    /// One cycle from `start`: `left` is the phase still to travel, or a whole cycle (2⁶⁴, which a `u64` cannot
    /// hold) while `fresh`.
    Run { start: u64, left: u64, fresh: bool },
    /// The cycle is over: the phase is parked just before `start` and the value is held.
    Hold,
}

/// One LFO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    phase: u64,
    inc: u64,
    inc_key: IncKey,
    shot: Shot,
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
            inc_key: IncKey::None,
            shot: Shot::Off,
            a,
            b,
            rng,
        }
    }

    /// Set the rate, recomputing the increment only when it changed.
    #[inline]
    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32, step: usize) {
        let key = IncKey::Hz(rate_hz.to_bits());
        if key != self.inc_key || self.inc == 0 {
            self.inc_key = key;
            self.inc = phase_increment(rate_hz, sample_rate, step);
        }
    }

    /// Set the speed from a tempo and a cycle length in beats (`(num, den)` beats per cycle), recomputing the
    /// increment only when either changed. The phase is continuous: a tempo change moves the speed from the next
    /// step on.
    #[inline]
    pub fn set_synced(&mut self, bpm: f64, cycle_beats: (u32, u32), sample_rate: f32, step: usize) {
        let key = IncKey::Beats(bpm.to_bits(), cycle_beats);
        if key != self.inc_key || self.inc == 0 {
            self.inc_key = key;
            self.inc = synced_increment(bpm, cycle_beats, sample_rate, step);
        }
    }

    /// Arm or disarm the one-shot mode; call it every step with the parameter's value. Armed (from free-running),
    /// the LFO starts one cycle from the phase it has now; the engine arms a voice's LFO at its first step, so the
    /// cycle starts at the note's start phase. Armed already, nothing changes (a cycle in flight is not restarted,
    /// and a held value stays held). Disarmed, it runs again from where it is.
    #[inline]
    pub fn set_one_shot(&mut self, on: bool) {
        match (on, self.shot) {
            (true, Shot::Off) => {
                self.shot = Shot::Run {
                    start: self.phase,
                    left: 0,
                    fresh: true,
                };
            }
            (false, Shot::Run { .. } | Shot::Hold) => self.shot = Shot::Off,
            _ => {}
        }
    }

    /// Whether a one-shot cycle has run its course and the value is held.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.shot == Shot::Hold
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
        match self.shot {
            Shot::Off => {
                let (p, wrapped) = self.phase.overflowing_add(self.inc);
                self.phase = p;
                if wrapped {
                    self.a = self.b;
                    self.b = self.rng.bipolar_f32();
                }
            }
            Shot::Run { start, left, fresh } => self.step_one_shot(start, left, fresh),
            Shot::Hold => {}
        }
        self.value(shape)
    }

    /// One step of a running one-shot cycle. A step that reaches the start phase again ends the cycle: the phase
    /// parks one count before `start` (the left limit, so a saw up holds +1) and the value is held from then on. The
    /// random stream moves on at a wrap only when the wrap comes before the end of the cycle, so a cycle that starts
    /// at phase 0 holds its own value and never draws the next cycle's.
    fn step_one_shot(&mut self, start: u64, left: u64, fresh: bool) {
        let inc = self.inc;
        if !fresh && inc >= left {
            // The wrap is `2⁶⁴ − phase` away (a whole cycle from phase 0); the end of the cycle is `left` away.
            let to_wrap = 0u64.wrapping_sub(self.phase);
            if self.phase != 0 && to_wrap < left {
                self.a = self.b;
                self.b = self.rng.bipolar_f32();
            }
            self.phase = start.wrapping_sub(1);
            self.shot = Shot::Hold;
            return;
        }
        let (p, wrapped) = self.phase.overflowing_add(inc);
        self.phase = p;
        if wrapped {
            self.a = self.b;
            self.b = self.rng.bipolar_f32();
        }
        // Still short of the end. A whole cycle (2⁶⁴) less `inc` is `0 − inc` in 64 bits; a stopped LFO (`inc` 0) has
        // travelled nothing, so a fresh cycle stays whole.
        let (left, fresh) = match (inc, fresh) {
            (0, _) => (left, fresh),
            (_, true) => (0u64.wrapping_sub(inc), false),
            (_, false) => (left - inc, false),
        };
        self.shot = Shot::Run { start, left, fresh };
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

    /// `phase_increment`'s exact value for integer inputs, in 128-bit arithmetic: `round(bpm · den · step · 2⁶⁴ /
    /// (60 · num · fs))`.
    fn exact_synced(bpm: u128, num: u128, den: u128, fs: u128, step: u128) -> u128 {
        let top = bpm * den * step * (1u128 << 64);
        let bottom = 60 * num * fs;
        (top + bottom / 2) / bottom
    }

    #[test]
    fn the_synced_increment_is_the_exact_ratio_to_f64_rounding() {
        for bpm in [20u128, 90, 128, 140, 174, 999] {
            for (num, den) in [
                (1u32, 12u32),
                (1, 8),
                (1, 3),
                (3, 4),
                (1, 1),
                (3, 2),
                (8, 3),
                (16, 1),
            ] {
                for fs in [44_100u128, 48_000, 96_000] {
                    let got = synced_increment(bpm as f64, (num, den), fs as f32, 32);
                    let want = exact_synced(bpm, u128::from(num), u128::from(den), fs, 32);
                    let off = (got as i128 - want as i128).unsigned_abs();
                    // Half a count of rounding to an integer, plus a few units of f64's 53-bit precision.
                    assert!(
                        off as f64 <= 1.0 + 4e-16 * want as f64,
                        "{bpm} BPM {num}/{den} beats at {fs}: {got} vs {want}"
                    );
                }
            }
        }
        // A cycle of one beat at 120 BPM and 48 kHz with 32-frame steps: 1/750 of a cycle per step.
        let inc = synced_increment(120.0, (1, 1), 48_000.0, 32) as f64;
        assert!((inc / TWO_POW_64 - 1.0 / 750.0).abs() < 1e-15);
        // A rate of bpm / 60 / beats Hz is the same speed as the unsynced path, to rounding.
        let hz = (140.0f64 / 60.0 / (2.0 / 3.0)) as f32;
        let a = phase_increment(hz, 48_000.0, 32) as f64;
        let b = synced_increment(140.0, (2, 3), 48_000.0, 32) as f64;
        assert!(
            (a / b - 1.0).abs() < 1e-6,
            "f32 rate rounding only: {a} {b}"
        );
        // Nothing sensible: stopped.
        assert_eq!(synced_increment(0.0, (1, 1), SR, 32), 0);
        assert_eq!(synced_increment(-140.0, (1, 1), SR, 32), 0);
        assert_eq!(synced_increment(f64::NAN, (1, 1), SR, 32), 0);
        assert_eq!(synced_increment(f64::INFINITY, (1, 1), SR, 32), 0);
        assert_eq!(synced_increment(140.0, (0, 1), SR, 32), 0);
    }

    #[test]
    fn a_synced_lfo_is_exact_in_beats_over_ten_minutes() {
        // Cycles after N steps at one tempo: wraps + the phase fraction, against bpm / 60 / beats · seconds.
        for (bpm, beats) in [
            (140.0f64, (1u32, 1u32)),
            (140.0, (2, 3)),
            (174.0, (3, 4)),
            (90.0, (1, 4)),
            (128.5, (16, 1)),
        ] {
            let mut l = Lfo::new(1, StartPhase::Cycles(0.0));
            l.set_synced(bpm, beats, SR, 32);
            let steps = 600 * 1500; // 600 s at 48 kHz in 32-frame steps
            let (mut wraps, mut last) = (0u64, l.phase());
            for _ in 0..steps {
                let _ = l.tick(Shape::Sine);
                if l.phase() < last {
                    wraps += 1;
                }
                last = l.phase();
            }
            let cycles = wraps as f64 + l.phase() as f64 / TWO_POW_64;
            let want = bpm / 60.0 * f64::from(beats.1) / f64::from(beats.0) * 600.0;
            assert!(
                (cycles / want - 1.0).abs() < 1e-12,
                "{bpm} BPM {beats:?}: {cycles} cycles vs {want}"
            );
        }
    }

    #[test]
    fn a_tempo_change_keeps_the_phase_and_changes_the_speed() {
        let mut l = Lfo::new(1, StartPhase::Cycles(0.3));
        l.set_synced(120.0, (1, 1), SR, 32);
        let inc = l.increment();
        l.set_synced(120.0, (1, 1), SR, 32);
        assert_eq!(l.increment(), inc, "cached");
        let _ = l.tick(Shape::Sine);
        let ph = l.phase();
        l.set_synced(240.0, (1, 1), SR, 32);
        assert_eq!(l.phase(), ph, "continuous");
        assert!(
            (l.increment() as f64 / inc as f64 - 2.0).abs() < 1e-12,
            "double speed"
        );
        // Back to Hz and back to beats: each setting wins over the cache of the other.
        l.set_rate(2.0, SR, 32);
        assert_eq!(l.increment(), phase_increment(2.0, SR, 32));
        l.set_synced(240.0, (1, 1), SR, 32);
        assert_eq!(l.increment(), synced_increment(240.0, (1, 1), SR, 32));
    }

    /// 1/64 of a cycle per step: 64 steps make one cycle exactly.
    fn lfo_64() -> Lfo {
        let mut l = Lfo::new(1, StartPhase::Cycles(0.0));
        l.set_rate(23.4375, SR, 32); // 23.4375 · 32 / 48000 = 1/64
        assert_eq!(l.increment(), 1 << 58);
        l
    }

    #[test]
    fn one_shot_runs_one_cycle_then_holds() {
        for shape in [
            Shape::Sine,
            Shape::Triangle,
            Shape::SawUp,
            Shape::SawDown,
            Shape::Square,
        ] {
            let mut free = lfo_64();
            let mut shot = lfo_64();
            shot.set_one_shot(true);
            for k in 1..=63 {
                shot.set_one_shot(true);
                let (f, s) = (free.tick(shape), shot.tick(shape));
                assert_eq!(
                    f.to_bits(),
                    s.to_bits(),
                    "{shape:?} step {k} is the free-running value"
                );
                assert!(!shot.is_held());
            }
            // The 64th step completes the cycle: the held value is the end of the cycle, not the wrapped start.
            let end = shot.tick(shape);
            assert!(shot.is_held(), "{shape:?}");
            let want = match shape {
                Shape::Sine | Shape::Triangle => 0.0,
                Shape::SawUp => 1.0,
                Shape::SawDown | Shape::Square => -1.0,
                _ => unreachable!(),
            };
            assert!(
                (end - want).abs() < 1e-5,
                "{shape:?}: held {end}, want {want}"
            );
            for _ in 0..500 {
                assert_eq!(shot.tick(shape).to_bits(), end.to_bits(), "{shape:?} holds");
            }
        }
    }

    #[test]
    fn one_shot_from_a_start_phase_ends_where_it_started() {
        // Start at a quarter turn: the saw rises to 1, wraps, rises to just under the start again and holds -0.5.
        let mut l = Lfo::new(1, StartPhase::Cycles(0.25));
        l.set_rate(23.4375, SR, 32);
        l.set_one_shot(true);
        let mut ys = Vec::new();
        for _ in 0..70 {
            l.set_one_shot(true);
            ys.push(l.tick(Shape::SawUp));
        }
        assert!(l.is_held());
        assert!((ys[63] + 0.5).abs() < 1e-5, "{}", ys[63]);
        assert!(ys[62] < ys[63], "still rising into the end of the cycle");
        assert!(
            ys[..47].windows(2).all(|w| w[1] > w[0]),
            "rises to the top of the cycle"
        );
        assert_eq!(ys[47], -1.0, "step 48 is the wrap, back to the bottom");
        assert!(
            ys[47..64].windows(2).all(|w| w[1] > w[0]),
            "rising again to the start phase"
        );
        assert!(ys[63..].iter().all(|y| y.to_bits() == ys[63].to_bits()));
        // A cycle that does not divide into whole steps: the last step is partial, the hold is the same.
        let mut odd = Lfo::new(1, StartPhase::Cycles(0.25));
        odd.set_rate(21.3, SR, 32);
        odd.set_one_shot(true);
        let mut held_at = None;
        let steps: Vec<f32> = (0..200)
            .map(|k| {
                let y = odd.tick(Shape::SawUp);
                if odd.is_held() && held_at.is_none() {
                    held_at = Some(k);
                }
                y
            })
            .collect();
        // 21.3 Hz is 0.01420 of a cycle per step: the cycle ends in step 71 (index 70), part way through it.
        assert_eq!(held_at, Some(70));
        assert!((steps[70] + 0.5).abs() < 1e-5, "{}", steps[70]);
        assert!(
            steps[70..]
                .iter()
                .all(|y| y.to_bits() == steps[70].to_bits())
        );
        assert!(
            steps[69] < steps[70],
            "the last step still rises into the end of the cycle"
        );
    }

    #[test]
    fn one_shot_random_shapes_hold_their_own_cycle() {
        // From phase 0 nothing is drawn at the end: sample & hold holds the cycle's first value, and smooth random
        // ends on the second.
        let mut r = Rng::new(7);
        let (a, b) = (r.bipolar_f32(), r.bipolar_f32());
        let mut sh = Lfo::new(7, StartPhase::Cycles(0.0));
        sh.set_rate(23.4375, SR, 32);
        sh.set_one_shot(true);
        let ys: Vec<f32> = (0..200).map(|_| sh.tick(Shape::SampleHold)).collect();
        assert!(ys.iter().all(|y| *y == a), "held {a}: {ys:?}");
        let mut sm = Lfo::new(7, StartPhase::Cycles(0.0));
        sm.set_rate(23.4375, SR, 32);
        sm.set_one_shot(true);
        let zs: Vec<f32> = (0..200).map(|_| sm.tick(Shape::SmoothRandom)).collect();
        assert!((zs[63] - b).abs() < 1e-5, "{} vs {b}", zs[63]);
        assert!(zs[63..].iter().all(|z| z.to_bits() == zs[63].to_bits()));
        // From half a cycle the wrap comes mid-cycle and draws once: the hold is the second value.
        let mut mid = Lfo::new(7, StartPhase::Cycles(0.5));
        mid.set_rate(23.4375, SR, 32);
        mid.set_one_shot(true);
        let ms: Vec<f32> = (0..200).map(|_| mid.tick(Shape::SampleHold)).collect();
        assert_eq!(*ms.last().unwrap(), b);
        assert_eq!(ms[0], a);
    }

    #[test]
    fn disarming_runs_again_and_arming_starts_a_new_cycle_from_here() {
        let mut l = lfo_64();
        l.set_one_shot(true);
        for _ in 0..70 {
            let _ = l.tick(Shape::SawUp);
        }
        assert!(l.is_held());
        let parked = l.phase();
        l.set_one_shot(false);
        assert!(!l.is_held());
        let y = l.tick(Shape::SawUp);
        assert_eq!(l.phase(), parked.wrapping_add(1 << 58));
        assert!(
            (y + 0.96875).abs() < 1e-5,
            "{y}: a step into the cycle (x = 1/64) after the parked phase"
        );
        // Arming mid-run: one more full cycle from this phase.
        let from = l.phase();
        l.set_one_shot(true);
        for k in 1..=64 {
            let _ = l.tick(Shape::Sine);
            assert_eq!(l.is_held(), k == 64, "step {k}");
        }
        assert_eq!(l.phase(), from.wrapping_sub(1));
        // A stopped LFO never completes a cycle.
        let mut stopped = Lfo::new(1, StartPhase::Cycles(0.0));
        stopped.set_rate(0.0, SR, 32);
        stopped.set_one_shot(true);
        for _ in 0..100 {
            let _ = stopped.tick(Shape::Sine);
        }
        assert!(!stopped.is_held());
        stopped.set_rate(23.4375, SR, 32);
        for _ in 0..64 {
            let _ = stopped.tick(Shape::Sine);
        }
        assert!(stopped.is_held(), "a whole cycle after it starts moving");
    }

    #[test]
    fn one_shot_is_off_by_default_and_changes_nothing_when_off() {
        let mut a = lfo_64();
        let mut b = lfo_64();
        for _ in 0..500 {
            b.set_one_shot(false);
            assert_eq!(
                a.tick(Shape::Triangle).to_bits(),
                b.tick(Shape::Triangle).to_bits()
            );
        }
        assert!(!b.is_held());
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
