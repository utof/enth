//! Parameter smoothers: a linear ramp and a one-pole, both of which settle *exactly* on their target.
//!
//! Two shapes, matching `Smoothing::Linear { ms }` and `Smoothing::OnePole { ms }` in `enth-params`. Both are
//! stepped by the caller (once per 32-frame segment for parameter values, once per sample for audio-rate use), keep
//! their state in `f32`, use no transcendental except `libm`'s `expf` when a coefficient is built, and stay finite
//! and exact: a settled smoother returns the target's own bits, so a parameter that has stopped moving is exactly
//! the value in the patch (`docs/dsp/engine.md`, "settled values are exact").
//!
//! The SVF does not use these types per sample: it ramps its coefficients with plain adds inside its loop
//! (`svf::process_block`), which is the same linear ramp without the bookkeeping. [`ramp_increment`] is the shared
//! arithmetic, so both ramp the same way.

use crate::util::math::exp;

/// A one-pole is settled when it is within this fraction of its target (or of 1.0, near zero): the next step then
/// snaps onto the target. About 2⁻²⁰, far below anything audible, and above where `f32` rounding would keep it
/// creeping towards the target for thousands of steps.
pub const SETTLE_EPS: f32 = 1e-6;

/// The per-step increment of a linear ramp from `from` to `to` in `steps` steps (`steps == 0` counts as one).
/// `from + increment · steps` is `to` up to rounding; [`LinearRamp`] snaps the last step onto `to`.
#[inline]
#[must_use]
pub fn ramp_increment(from: f32, to: f32, steps: u32) -> f32 {
    (to - from) / steps.max(1) as f32
}

/// The one-pole coefficient `exp(−1 / (τ · rate))` for a time constant `ms` (the time to cover 63 % of a step),
/// stepped at `step_rate_hz`. A time constant of zero (or less) gives `0.0`: the output follows the target at once.
#[must_use]
pub fn one_pole_coefficient(ms: f32, step_rate_hz: f32) -> f32 {
    let steps = f64::from(ms) * 0.001 * f64::from(step_rate_hz);
    if steps > 0.0 && steps.is_finite() {
        exp(-1.0 / steps) as f32
    } else {
        0.0
    }
}

/// A linear ramp towards a target in a whole number of steps; the last step lands on the target exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRamp {
    value: f32,
    target: f32,
    inc: f32,
    left: u32,
}

impl LinearRamp {
    /// A settled ramp at `value`.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            inc: 0.0,
            left: 0,
        }
    }

    /// Head for `target`, reaching it after `steps` calls to [`step`](Self::step) (`steps == 0`: at once). Starts
    /// from the present value, wherever the last ramp had got to, so a retarget never jumps.
    pub fn set_target(&mut self, target: f32, steps: u32) {
        self.target = target;
        if steps == 0 {
            self.value = target;
            self.inc = 0.0;
            self.left = 0;
        } else {
            self.inc = ramp_increment(self.value, target, steps);
            self.left = steps;
        }
    }

    /// Jump to `value` and settle there.
    pub fn jump(&mut self, value: f32) {
        *self = Self::new(value);
    }

    /// Advance one step and return the new value.
    #[inline]
    pub fn step(&mut self) -> f32 {
        match self.left {
            0 => {}
            1 => {
                self.left = 0;
                self.value = self.target;
            }
            _ => {
                self.left -= 1;
                self.value += self.inc;
            }
        }
        self.value
    }

    /// The present value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// The value it is heading for.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether it has reached its target.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.left == 0
    }
}

/// A one-pole low-pass on a control value, snapping onto the target once it is within [`SETTLE_EPS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnePole {
    value: f32,
}

impl OnePole {
    /// Settled at `value`.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self { value }
    }

    /// Jump to `value`.
    pub fn jump(&mut self, value: f32) {
        self.value = value;
    }

    /// Advance one step towards `target` with coefficient `coef` (from [`one_pole_coefficient`]) and return the new
    /// value. Once within [`SETTLE_EPS`] of the target (relative to `max(|target|, 1)`), it *is* the target.
    #[inline]
    pub fn step(&mut self, target: f32, coef: f32) -> f32 {
        let y = target + (self.value - target) * coef;
        let tol = SETTLE_EPS * target.abs().max(1.0);
        self.value = if (y - target).abs() <= tol { target } else { y };
        self.value
    }

    /// The present value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ramp_is_linear_and_lands_exactly() {
        let mut r = LinearRamp::new(1.0);
        assert!(r.is_settled());
        assert_eq!(r.step().to_bits(), 1.0f32.to_bits());
        r.set_target(3.0, 4);
        assert!(!r.is_settled());
        let got: Vec<f32> = (0..4).map(|_| r.step()).collect();
        assert_eq!(got, [1.5, 2.0, 2.5, 3.0]);
        assert!(r.is_settled());
        assert_eq!(r.step().to_bits(), 3.0f32.to_bits());
        // A target that does not divide evenly still lands on its own bits.
        let mut r = LinearRamp::new(0.1);
        r.set_target(0.7, 7);
        let last = (0..7).map(|_| r.step()).last().unwrap();
        assert_eq!(last.to_bits(), 0.7f32.to_bits());
    }

    #[test]
    fn a_retarget_starts_from_where_the_ramp_is() {
        let mut r = LinearRamp::new(0.0);
        r.set_target(1.0, 10);
        for _ in 0..5 {
            let _ = r.step();
        }
        let mid = r.value();
        r.set_target(0.0, 5);
        assert_eq!(r.step().to_bits(), (mid + (0.0 - mid) / 5.0).to_bits());
        for _ in 0..4 {
            let _ = r.step();
        }
        assert_eq!(r.value().to_bits(), 0.0f32.to_bits());
        r.set_target(2.0, 0);
        assert!(r.is_settled());
        assert_eq!(r.value(), 2.0);
        r.jump(-1.0);
        assert_eq!((r.value(), r.target()), (-1.0, -1.0));
    }

    #[test]
    fn a_one_pole_reaches_63_percent_in_one_time_constant_and_then_settles() {
        // 10 ms constant stepped at 1500 Hz (one step per 32-frame segment at 48 kHz): 15 steps.
        let coef = one_pole_coefficient(10.0, 1500.0);
        assert!((coef - exp(-1.0 / 15.0) as f32).abs() < 1e-7);
        let mut p = OnePole::new(0.0);
        let mut y = 0.0;
        for _ in 0..15 {
            y = p.step(1.0, coef);
        }
        assert!((y - (1.0 - exp(-1.0) as f32)).abs() < 1e-5, "{y}");
        let mut steps = 15;
        while p.value() != 1.0 {
            let _ = p.step(1.0, coef);
            steps += 1;
            assert!(steps < 400, "does not settle");
        }
        assert_eq!(p.step(1.0, coef).to_bits(), 1.0f32.to_bits());
        // Zero time follows at once; a degenerate rate does not produce NaN.
        assert_eq!(one_pole_coefficient(0.0, 1500.0), 0.0);
        assert_eq!(one_pole_coefficient(5.0, 0.0), 0.0);
        assert_eq!(one_pole_coefficient(f32::NAN, 1500.0), 0.0);
        assert_eq!(OnePole::new(5.0).step(-3.0, 0.0), -3.0);
    }

    #[test]
    fn a_one_pole_settles_on_small_and_negative_targets_too() {
        let coef = one_pole_coefficient(5.0, 1500.0);
        for target in [0.0f32, -2.5, 1e-4, 100.0] {
            let mut p = OnePole::new(7.0);
            for _ in 0..2000 {
                let _ = p.step(target, coef);
            }
            assert_eq!(p.value().to_bits(), target.to_bits(), "{target}");
        }
    }
}
