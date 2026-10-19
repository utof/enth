//! `Scale` (Linear, Log, Power) and the plain ⇄ normalised mappings, through the pinned `libm`.
//!
//! **The scale is part of the persisted contract** (design.md §1): route depth is a percentage of the
//! destination's range in this normalised domain, and ML encoding and the randomiser use the same domain. So
//! changing a parameter's scale, range or unit needs a patch migration, and these functions must give the same
//! bits on every platform: all maths runs in `f64` through `libm` (never `std`'s transcendentals, which differ
//! between platforms), then rounds once to `f32`. The engine uses the same functions for modulation, so a
//! change to their arithmetic is also an engine-version change.
//!
//! Guarantees, for a range `min < max` (checked at compile time for every catalogue spec):
//!
//! - `to_norm` clamps its input to `[min, max]` and returns a value in `[0, 1]`, with `to_norm(min) == 0` and
//!   `to_norm(max) == 1` exactly;
//! - `from_norm` clamps its input to `[0, 1]`, with `from_norm(0) == min` and `from_norm(1) == max` exactly, and
//!   never returns `-0.0`;
//! - both are monotonic (non-decreasing), and `to_norm(from_norm(to_norm(x)))` is within one `f32` step of
//!   `to_norm(x)` (the only loss is rounding the normalised value to `f32`).

/// How a continuous parameter's plain range maps onto the normalised 0–1 domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    /// `n = (x − min) / (max − min)`.
    Linear,
    /// `n = ln(x / min) / ln(max / min)`; needs `min > 0`. Equal steps of `n` are equal ratios of `x`, so an octave
    /// is the same depth everywhere: `+1oct` on 20 Hz–20 kHz is 10.034 % of the range.
    Log,
    /// `n = ((x − min) / (max − min))^(1/k)`, i.e. `x = min + (max − min)·n^k`, with `k > 0` and `k ≠ 1`. For
    /// ranges that start at 0, where `Log` is impossible: times such as attack 0–10,000 ms (`k = 4` puts 625 ms
    /// at the middle of the knob).
    Power(f32),
}

impl Scale {
    /// Map a plain value to the normalised domain. The input is clamped to `[min, max]` first.
    pub fn to_norm(self, min: f32, max: f32, x: f32) -> f32 {
        let (lo, hi) = (f64::from(min), f64::from(max));
        let x = f64::from(x).clamp(lo, hi);
        (self.to_norm_f64(lo, hi, x).clamp(0.0, 1.0)) as f32
    }

    /// Map a normalised value back to the plain range. The input is clamped to `[0, 1]` first.
    pub fn from_norm(self, min: f32, max: f32, n: f32) -> f32 {
        if n <= 0.0 {
            return min;
        }
        if n >= 1.0 {
            return max;
        }
        let (lo, hi, n) = (f64::from(min), f64::from(max), f64::from(n));
        let x = match self {
            Scale::Linear => lo + n * (hi - lo),
            Scale::Log => lo * libm::exp(n * libm::log(hi / lo)),
            Scale::Power(k) => lo + (hi - lo) * libm::pow(n, f64::from(k)),
        };
        let x = (x as f32).clamp(min, max);
        crate::units::canonical_zero(x)
    }

    /// The unclamped mapping in `f64`. Outside `[min, max]` it extrapolates where the formula allows (Linear and
    /// Log everywhere, Power above `min`); used to turn musical intervals into route depths.
    pub(crate) fn to_norm_f64(self, lo: f64, hi: f64, x: f64) -> f64 {
        match self {
            Scale::Linear => (x - lo) / (hi - lo),
            Scale::Log => libm::log(x / lo) / libm::log(hi / lo),
            Scale::Power(k) => libm::pow(((x - lo) / (hi - lo)).max(0.0), 1.0 / f64::from(k)),
        }
    }

    /// A stable lowercase name for JSON and docs: `linear`, `log`, `power`.
    pub const fn name(self) -> &'static str {
        match self {
            Scale::Linear => "linear",
            Scale::Log => "log",
            Scale::Power(_) => "power",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_exact() {
        for (s, lo, hi) in [
            (Scale::Linear, -60.0, 6.0),
            (Scale::Log, 20.0, 20_000.0),
            (Scale::Power(4.0), 0.0, 10_000.0),
        ] {
            assert_eq!(s.to_norm(lo, hi, lo), 0.0);
            assert_eq!(s.to_norm(lo, hi, hi), 1.0);
            assert_eq!(s.from_norm(lo, hi, 0.0), lo);
            assert_eq!(s.from_norm(lo, hi, 1.0), hi);
            assert_eq!(s.to_norm(lo, hi, lo - 1.0), 0.0);
            assert_eq!(s.to_norm(lo, hi, hi + 1.0), 1.0);
            assert_eq!(s.from_norm(lo, hi, -0.5), lo);
            assert_eq!(s.from_norm(lo, hi, 1.5), hi);
        }
    }

    #[test]
    fn known_points() {
        // Log 20 Hz–20 kHz: the geometric middle is 632.46 Hz.
        assert!((Scale::Log.from_norm(20.0, 20_000.0, 0.5) - 632.455_5).abs() < 1e-3);
        // Power(4) on 0–10,000 ms puts 625 ms at the middle.
        assert_eq!(Scale::Power(4.0).from_norm(0.0, 10_000.0, 0.5), 625.0);
        assert_eq!(Scale::Linear.from_norm(-100.0, 100.0, 0.5), 0.0);
        assert!(
            Scale::Linear
                .from_norm(-100.0, 100.0, 0.5)
                .is_sign_positive()
        );
    }
}
