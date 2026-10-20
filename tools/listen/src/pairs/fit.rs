//! The curve fit: ratings against `sound/1`'s JND distance.
//!
//! **The model.** `rating(z) = 1 + 4 / (1 + (z / c)^q)`: 5 (indistinguishable) at z = 0, falling to 1 (very different)
//! far away, 3 at z = c. It is the sound score's curve written on the rating scale, so the fit maps onto a `Curve`
//! directly: rating 3 is the 50 % point, `z50 = c`; rating 4.5 is the pass mark, at `z = c · 7^(−1/q)`, and putting it
//! on 90 % gives `k = q · ln 9 / ln 7`. (`sound/1` was made the same way from two points of session 1, rating 4.5 and
//! rating 3, read off a logistic fit; ADR-0007's 2026-10-20 notes.)
//!
//! **The fit.** Least squares on the ratings (one point per test pair: its mean rating over the sessions played), over
//! `ln c` in [ln 0.05, ln 200] and `q` in [0.2, 8]: a 41 × 40 grid, then six rounds of an 11 × 11 grid around the best
//! point, each a third the size of the last.
//! Deterministic, no dependencies. A bootstrap over pairs gives percentile 95 % intervals. A best point on the edge
//! of the box is flagged: the data do not pin the curve (all pairs rated alike, say).

use serde::{Deserialize, Serialize};
use spike_metric::stats::{percentile_ci, resamples};

/// The search box.
const LN_C: (f64, f64) = (-2.995_732_273_553_991, 5.298_317_366_548_036); // ln 0.05, ln 200
const Q: (f64, f64) = (0.2, 8.0);

/// `rating(z) = 1 + 4 / (1 + (z / c)^q)` (see the [module docs](self)).
#[must_use]
pub fn rating_model(z: f64, c: f64, q: f64) -> f64 {
    if z <= 0.0 {
        return 5.0;
    }
    1.0 + 4.0 / (1.0 + libm::pow(z / c, q))
}

/// The least-squares fit of the model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RatingFit {
    /// The distance rated 3.
    pub c: f64,
    /// The steepness on the rating scale.
    pub q: f64,
    /// Sum of squared rating errors.
    pub sse: f64,
    /// Points fitted.
    pub n: usize,
    /// The best point is on the edge of the search box.
    pub at_bound: bool,
}

impl RatingFit {
    /// The distance rated 4.5: the proposed pass point.
    #[must_use]
    pub fn z_pass(&self) -> f64 {
        self.c * libm::pow(7.0, -1.0 / self.q)
    }

    /// The sound-% curve's steepness: `k = q · ln 9 / ln 7`.
    #[must_use]
    pub fn k(&self) -> f64 {
        self.q * libm::log(9.0) / libm::log(7.0)
    }
}

fn sse(z: &[f64], r: &[f64], ln_c: f64, q: f64) -> f64 {
    let c = libm::exp(ln_c);
    z.iter()
        .zip(r)
        .map(|(z, r)| {
            let e = rating_model(*z, c, q) - r;
            e * e
        })
        .sum()
}

/// Fit the model to distances `z` and ratings `r` (equal lengths). `None` with fewer than 3 points or no finite
/// distance.
#[must_use]
pub fn fit_ratings(z: &[f64], r: &[f64]) -> Option<RatingFit> {
    fit_from(z, r, None)
}

/// Points per side of a refinement grid, and refinement rounds.
const REFINE_POINTS: usize = 11;
const REFINE_ROUNDS: usize = 6;

/// The fit, from the whole box (`start` = `None`: a 41 × 40 grid first) or refined around `start` (`(ln c, q)`; the
/// bootstrap starts each resample at the full fit, which is about 10 times cheaper).
fn fit_from(z: &[f64], r: &[f64], start: Option<(f64, f64)>) -> Option<RatingFit> {
    let pts: Vec<(f64, f64)> = z
        .iter()
        .zip(r)
        .filter(|(z, r)| z.is_finite() && r.is_finite())
        .map(|(z, r)| (*z, *r))
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let (z, r): (Vec<f64>, Vec<f64>) = pts.into_iter().unzip();
    let grid = |lo: f64, hi: f64, n: usize| -> Vec<f64> {
        (0..n)
            .map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64)
            .collect()
    };
    let search = |lcs: &[f64], qs: &[f64], best: &mut (f64, f64, f64)| {
        for lc in lcs {
            for q in qs {
                let e = sse(&z, &r, *lc, *q);
                if e < best.0 {
                    *best = (e, *lc, *q);
                }
            }
        }
    };
    // From the coarse grid, refine at its spacing; from a start, at 4 times that (the first refinement spans
    // +-4 in ln c and +-3 in q, so a resample whose best point is far from the full fit's still finds it).
    let (mut dl, mut dq, mut best) = if let Some((lc, q)) = start {
        (
            (LN_C.1 - LN_C.0) / 10.0,
            (Q.1 - Q.0) / 13.0,
            (sse(&z, &r, lc, q), lc, q),
        )
    } else {
        let mut b = (f64::INFINITY, 0.0, 1.0);
        search(&grid(LN_C.0, LN_C.1, 41), &grid(Q.0, Q.1, 40), &mut b);
        ((LN_C.1 - LN_C.0) / 40.0, (Q.1 - Q.0) / 39.0, b)
    };
    let half = (REFINE_POINTS / 2) as f64;
    for _ in 0..REFINE_ROUNDS {
        let (lc, q) = (best.1, best.2);
        let lcs = grid(
            (lc - half * dl).max(LN_C.0),
            (lc + half * dl).min(LN_C.1),
            REFINE_POINTS,
        );
        let qs = grid(
            (q - half * dq).max(Q.0),
            (q + half * dq).min(Q.1),
            REFINE_POINTS,
        );
        search(&lcs, &qs, &mut best);
        dl /= 3.0;
        dq /= 3.0;
    }
    let edge = |v: f64, (lo, hi): (f64, f64)| (v - lo).abs() < 1e-9 || (hi - v).abs() < 1e-9;
    Some(RatingFit {
        c: libm::exp(best.1),
        q: best.2,
        sse: best.0,
        n: z.len(),
        at_bound: edge(best.1, LN_C) || edge(best.2, Q),
    })
}

/// A 95 % interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    /// Lower bound.
    pub lo: f64,
    /// Upper bound.
    pub hi: f64,
}

/// The proposed curve, with bootstrap intervals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveFit {
    /// The least-squares fit.
    pub fit: RatingFit,
    /// The 50 % point (`c`).
    pub z50: f64,
    /// The sound-% curve's steepness.
    pub k: f64,
    /// The distance rated 4.5: where the pass mark would sit.
    pub z_pass: f64,
    /// Root mean square rating error.
    pub rmse: f64,
    /// Bootstrap resamples behind the intervals.
    pub reps: usize,
    /// 95 % interval of `z50`.
    pub z50_ci: Interval,
    /// 95 % interval of `k`.
    pub k_ci: Interval,
    /// 95 % interval of `z_pass`.
    pub z_pass_ci: Interval,
}

impl CurveFit {
    /// Fit `z` and `r` and bootstrap over points (`reps` resamples from `seed`).
    #[must_use]
    pub fn new(z: &[f64], r: &[f64], reps: usize, seed: u64) -> Option<CurveFit> {
        let fit = fit_ratings(z, r)?;
        let mut z50 = Vec::new();
        let mut k = Vec::new();
        let mut zp = Vec::new();
        for ix in resamples(z.len(), reps, seed) {
            let zs: Vec<f64> = ix.iter().map(|i| z[*i]).collect();
            let rs: Vec<f64> = ix.iter().map(|i| r[*i]).collect();
            if let Some(f) = fit_from(&zs, &rs, Some((libm::log(fit.c), fit.q))) {
                z50.push(f.c);
                k.push(f.k());
                zp.push(f.z_pass());
            }
        }
        let ci = |v: &[f64]| {
            if v.is_empty() {
                Interval {
                    lo: f64::NAN,
                    hi: f64::NAN,
                }
            } else {
                let (lo, hi) = percentile_ci(v);
                Interval { lo, hi }
            }
        };
        Some(CurveFit {
            fit,
            z50: fit.c,
            k: fit.k(),
            z_pass: fit.z_pass(),
            rmse: libm::sqrt(fit.sse / fit.n as f64),
            reps,
            z50_ci: ci(&z50),
            k_ci: ci(&k),
            z_pass_ci: ci(&zp),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sound/1`'s curve on the rating scale: c = z50 = 5.11, q = k · ln 7 / ln 9.
    fn sound_v1() -> (f64, f64) {
        let c = enth_challenge::calibrate::Curve::SOUND_V1;
        (c.z50, c.k * libm::log(7.0) / libm::log(9.0))
    }

    #[test]
    fn the_model_maps_onto_the_sound_curve() {
        let (c, q) = sound_v1();
        let f = RatingFit {
            c,
            q,
            sse: 0.0,
            n: 0,
            at_bound: false,
        };
        let curve = enth_challenge::calibrate::Curve::SOUND_V1;
        assert!((f.k() - curve.k).abs() < 1e-12);
        assert!((rating_model(f.z_pass(), c, q) - 4.5).abs() < 1e-12);
        assert!((curve.percent(f.z_pass()) - 90.0).abs() < 1e-9);
        assert!((f.z_pass() - 0.989).abs() < 1e-3, "{}", f.z_pass());
        assert!((rating_model(c, c, q) - 3.0).abs() < 1e-12);
        assert_eq!(rating_model(0.0, c, q), 5.0);
    }

    #[test]
    fn a_noise_free_curve_is_recovered() {
        let (c, q) = sound_v1();
        let z: Vec<f64> = (0..40)
            .map(|i| 0.1 * libm::pow(1.2, f64::from(i)))
            .collect();
        let r: Vec<f64> = z.iter().map(|z| rating_model(*z, c, q)).collect();
        let f = fit_ratings(&z, &r).unwrap();
        assert!(
            (f.c - c).abs() / c < 1e-3 && (f.q - q).abs() / q < 1e-3,
            "{f:?}"
        );
        assert!(!f.at_bound && f.sse < 1e-3, "{f:?}");
    }

    #[test]
    fn flat_or_tiny_data_are_flagged() {
        assert!(fit_ratings(&[1.0, 2.0], &[5.0, 4.0]).is_none());
        let f = fit_ratings(&[0.5, 1.0, 2.0, 4.0], &[5.0, 5.0, 5.0, 5.0]).unwrap();
        assert!(f.at_bound, "{f:?}");
    }

    #[test]
    fn the_bootstrap_brackets_the_estimate() {
        let (c, q) = sound_v1();
        let z: Vec<f64> = (0..30)
            .map(|i| 0.2 * libm::pow(1.18, f64::from(i)))
            .collect();
        // deterministic "noise": alternate +-0.4
        let r: Vec<f64> = z
            .iter()
            .enumerate()
            .map(|(i, z)| rating_model(*z, c, q) + if i % 2 == 0 { 0.4 } else { -0.4 })
            .collect();
        let f = CurveFit::new(&z, &r, 200, 7).unwrap();
        assert!(f.z50_ci.lo <= f.z50 && f.z50 <= f.z50_ci.hi, "{f:?}");
        assert!(
            f.z_pass_ci.lo <= f.z_pass && f.z_pass <= f.z_pass_ci.hi,
            "{f:?}"
        );
        assert!((f.z50 - c).abs() / c < 0.2, "{f:?}");
    }
}
