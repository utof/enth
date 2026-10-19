//! Signed margins, nuisance noise and the `thin` verdict.
//!
//! - **Margin**: the signed distance from a reading to the nearest edge of the accepted interval, in the feature's
//!   unit, positive inside. For Hz and ms features `margin_pct` gives it relative to that edge as well.
//! - **Noise**: the largest change of the reading over the nuisance re-renders the caller supplies: renders that
//!   differ only in things a listener does not count as design (a common phase shift of the oscillators; a different
//!   inter-oscillator phase only when they are detuned, because at 0 ct that difference is a timbre change; another
//!   noise seed). Rendering is the caller's job; this crate only compares the readings.
//! - **Thin**: |margin| ≤ 2 × noise. A pass that is thin can flip with an inaudible change, so an agent should aim
//!   further in. A reading that is present in some renders and absent in others is `unstable`, and also thin.
//!
//! Report 03 §4.5 (P1a research).

use serde::{Deserialize, Serialize};

/// `thin` when |margin| ≤ `THIN_FACTOR` × noise.
pub const THIN_FACTOR: f64 = 2.0;

/// An accepted interval in the compared domain. Open ends are infinite.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
    /// Whether a reading equal to `lo` (`hi`) passes. Matters only for exact readings such as booleans.
    pub lo_inclusive: bool,
    pub hi_inclusive: bool,
}

impl Interval {
    /// `[lo, hi]`, both ends inclusive.
    #[must_use]
    pub fn closed(lo: f64, hi: f64) -> Self {
        Self {
            lo,
            hi,
            lo_inclusive: true,
            hi_inclusive: true,
        }
    }

    /// `> v` (`>= v` when `inclusive`).
    #[must_use]
    pub fn above(v: f64, inclusive: bool) -> Self {
        Self {
            lo: v,
            hi: f64::INFINITY,
            lo_inclusive: inclusive,
            hi_inclusive: true,
        }
    }

    /// `< v` (`<= v` when `inclusive`).
    #[must_use]
    pub fn below(v: f64, inclusive: bool) -> Self {
        Self {
            lo: f64::NEG_INFINITY,
            hi: v,
            lo_inclusive: true,
            hi_inclusive: inclusive,
        }
    }

    #[must_use]
    pub fn contains(&self, v: f64) -> bool {
        let lo_ok = if self.lo_inclusive {
            v >= self.lo
        } else {
            v > self.lo
        };
        let hi_ok = if self.hi_inclusive {
            v <= self.hi
        } else {
            v < self.hi
        };
        lo_ok && hi_ok
    }

    /// The edge nearest to `v` (the one that sets the margin).
    #[must_use]
    pub fn nearest_edge(&self, v: f64) -> f64 {
        if v - self.lo <= self.hi - v {
            self.lo
        } else {
            self.hi
        }
    }

    /// Signed distance from `v` to the nearest edge, positive inside.
    #[must_use]
    pub fn margin(&self, v: f64) -> f64 {
        (v - self.lo).min(self.hi - v)
    }

    /// How far `v` is outside, 0 inside. For log-scale features pass `log = true`: the shortfall is then the
    /// natural log of the ratio to the edge (nepers), infinite when either is not positive.
    #[must_use]
    pub fn shortfall(&self, v: f64, log: bool) -> f64 {
        if self.contains(v) {
            return 0.0;
        }
        let edge = if v < self.lo { self.lo } else { self.hi };
        if log {
            if v > 0.0 && edge > 0.0 {
                libm::log(v / edge).abs()
            } else {
                f64::INFINITY
            }
        } else {
            (v - edge).abs()
        }
    }
}

/// The margin relative to the nearest edge, in percent: `100 · margin / |edge|`. `None` when the edge is 0 or
/// infinite.
#[must_use]
pub fn margin_pct(iv: &Interval, v: f64) -> Option<f64> {
    let edge = iv.nearest_edge(v);
    (edge.is_finite() && edge != 0.0).then(|| 100.0 * iv.margin(v) / edge.abs())
}

/// Nuisance noise of one reading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Noise {
    /// Largest |change| over the nuisance readings that were present (0 when none was).
    pub max_abs_delta: f64,
    /// The reading was present in some renders and absent in others.
    pub unstable: bool,
}

/// Noise of `value` against the same reading on the nuisance renders. `None` when there are no nuisance renders.
#[must_use]
pub fn noise(value: Option<f64>, nuisance: &[Option<f64>]) -> Option<Noise> {
    if nuisance.is_empty() {
        return None;
    }
    let unstable = nuisance.iter().any(|n| n.is_some() != value.is_some());
    let max_abs_delta = match value {
        Some(v) => nuisance
            .iter()
            .flatten()
            .map(|n| (n - v).abs())
            .fold(0.0, f64::max),
        None => 0.0,
    };
    Some(Noise {
        max_abs_delta,
        unstable,
    })
}

/// `thin` when |margin| ≤ 2 × noise, or when the reading is unstable.
#[must_use]
pub fn is_thin(margin: Option<f64>, noise: Option<Noise>) -> bool {
    match (margin, noise) {
        (_, Some(n)) if n.unstable => true,
        (Some(m), Some(n)) => m.abs() <= THIN_FACTOR * n.max_abs_delta,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margins_are_signed_distances_to_the_nearest_edge() {
        let iv = Interval::above(8.0, false);
        assert_eq!(iv.margin(9.0), 1.0);
        assert_eq!(iv.margin(6.0), -2.0);
        assert_eq!(margin_pct(&iv, 9.0), Some(12.5));
        let w = Interval::closed(3.0, 9.0);
        assert!((w.margin(6.82) - 2.18).abs() < 1e-9);
        assert!((w.margin(8.5) - 0.5).abs() < 1e-12);
        assert_eq!(w.margin(10.0), -1.0);
        assert_eq!(w.nearest_edge(4.0), 3.0);
        let b = Interval::below(-4.0, false);
        assert_eq!(b.margin(-4.5), 0.5);
        assert!(
            !Interval::above(1.0, false).contains(1.0) && Interval::above(1.0, true).contains(1.0)
        );
        assert_eq!(Interval::closed(0.0, 0.0).margin(0.0), 0.0);
        assert_eq!(margin_pct(&Interval::closed(0.0, 0.0), 0.0), None);
    }

    #[test]
    fn shortfall_in_nepers_for_log_features() {
        let iv = Interval::above(2000.0, false);
        assert_eq!(iv.shortfall(2500.0, true), 0.0);
        assert!((iv.shortfall(1000.0, true) - libm::log(2.0)).abs() < 1e-12);
        assert_eq!(iv.shortfall(1000.0, false), 1000.0);
        assert!(iv.shortfall(-1.0, true).is_infinite());
    }

    #[test]
    fn noise_and_thin() {
        let n = noise(Some(9.0), &[Some(8.0), Some(10.5)]).unwrap();
        assert_eq!(
            n,
            Noise {
                max_abs_delta: 1.5,
                unstable: false
            }
        );
        assert!(is_thin(Some(1.0), Some(n)));
        assert!(!is_thin(Some(3.5), Some(n)));
        assert!(!is_thin(Some(1.0), None));
        assert_eq!(noise(Some(1.0), &[]), None);
        let flaky = noise(Some(2.0), &[None, Some(2.1)]).unwrap();
        assert!(flaky.unstable && is_thin(Some(100.0), Some(flaky)));
        assert!(noise(None, &[None, None]).is_some_and(|n| !n.unstable));
    }
}
