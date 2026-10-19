//! The local model behind a hint: each expectation as a value in a search domain with a target band, a
//! finite-difference Jacobian over the free parameters' normalised positions, the bounded Gauss-Newton step on
//! the squared hinge, and the Broyden rank-1 update.
//!
//! Everything here is plain `f64` arithmetic plus `libm`, in fixed loop orders: the same inputs give the same bits
//! on every platform, which is what makes a hint reproducible.

use enth_analysis::descriptors::{Descriptors, Feature};
use enth_analysis::expect::{Expect, Test};

/// The aim margin for pitch judged in cents (the feature's own hint margin is a ratio for Hz).
const PITCH_AIM_CT: f64 = 3.0;
/// The aim margin below full scale for a `clipping = false` item: aim for a peak at or below −1 dBFS.
const CLIP_AIM_DB: f64 = 1.0;
/// Normalised shortfall counted for an item whose reading is missing (or not modelled) and fails.
pub(crate) const MISSING_SHORTFALL: f64 = 1e3;

/// How the solver sees one expectation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Role {
    /// A value in a search domain with an interval to aim inside.
    Modelled(Aim),
    /// `== null` / `!= null`: judged by renders only (no continuous measure to aim with).
    NullTest,
}

/// The search domain and the accepted interval of one modelled item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Aim {
    /// Natural log of the compared value (Hz and ms features), else the value itself.
    pub log: bool,
    /// The item is `loudness.clipping`, aimed at through the sample peak in dBFS (clipping starts at 0 dBFS).
    pub clip_surrogate: bool,
    /// The accepted interval in the search domain (open ends infinite).
    pub lo: f64,
    pub hi: f64,
    /// One aim margin in the search domain: the unit of shortfalls.
    pub scale: f64,
}

/// How much more a passing item's hinge weighs than a failing item's: keeping passing items passing is close to a
/// hard constraint, so a move never buys progress on one item with another item's failure.
pub(crate) const KEEP_WEIGHT: f64 = 100.0;

/// One row of the model at a point: the item's value, the band to aim for and the weight of its squared hinge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Row {
    pub g: f64,
    pub band: (f64, f64),
    pub w: f64,
}

impl Row {
    /// Signed distance from `x` to the band: negative below it, positive above, 0 inside.
    pub(crate) fn dist(&self, x: f64) -> f64 {
        if x < self.band.0 {
            x - self.band.0
        } else if x > self.band.1 {
            x - self.band.1
        } else {
            0.0
        }
    }
}

impl Role {
    pub(crate) fn of(e: &Expect) -> Role {
        let Some(iv) = e.interval() else {
            return Role::NullTest;
        };
        if e.feature == Feature::Clipping {
            let want_clip = iv.lo >= 0.5;
            let (lo, hi) = if want_clip {
                (0.0, f64::INFINITY)
            } else {
                (f64::NEG_INFINITY, 0.0)
            };
            return Role::Modelled(Aim {
                log: false,
                clip_surrogate: true,
                lo,
                hi,
                scale: CLIP_AIM_DB,
            });
        }
        let log = e.log_scale();
        let margin = if e.reference_hz.is_some() {
            PITCH_AIM_CT
        } else {
            e.feature.hint_margin()
        };
        if log {
            // A bound at or below zero on a positive quantity: everything is above (or nothing is below) it.
            let ln = |v: f64| libm::log(v.max(1e-12));
            let lo = if iv.lo.is_finite() && iv.lo > 0.0 {
                ln(iv.lo)
            } else {
                f64::NEG_INFINITY
            };
            let hi = if iv.hi.is_finite() {
                ln(iv.hi)
            } else {
                f64::INFINITY
            };
            Role::Modelled(Aim {
                log,
                clip_surrogate: false,
                lo,
                hi,
                scale: libm::log(margin),
            })
        } else {
            Role::Modelled(Aim {
                log,
                clip_surrogate: false,
                lo: iv.lo,
                hi: iv.hi,
                scale: margin,
            })
        }
    }

    pub(crate) fn aim(&self) -> Option<&Aim> {
        match self {
            Role::Modelled(a) => Some(a),
            Role::NullTest => None,
        }
    }
}

impl Aim {
    /// The item's reading in the search domain, `None` when absent (or not positive on a log axis).
    pub(crate) fn value(&self, e: &Expect, d: &Descriptors) -> Option<f64> {
        if self.clip_surrogate {
            return d.get(Feature::Peak).filter(|v| v.is_finite());
        }
        let c = e.compared(d)?;
        if self.log {
            (c > 0.0).then(|| libm::log(c))
        } else {
            Some(c)
        }
    }

    /// The model row of this item at value `g`. A failing item aims one margin inside its interval with weight
    /// `1/margin²`. A passing item only has to stay inside: its band edge is half a margin inside the interval, or
    /// at its current value when it is thinner than that, with [`KEEP_WEIGHT`] times the weight.
    pub(crate) fn row(&self, g: f64, passing: bool) -> Row {
        let s = if passing {
            0.5 * self.scale
        } else {
            self.scale
        };
        let (mut lo, mut hi) = (self.lo + s, self.hi - s);
        if passing {
            lo = lo.min(g.max(self.lo));
            hi = hi.max(g.min(self.hi));
        }
        if lo > hi {
            let mid = if lo.is_finite() && hi.is_finite() {
                f64::midpoint(lo, hi)
            } else if lo.is_finite() {
                lo
            } else {
                hi
            };
            lo = mid;
            hi = mid;
        }
        let w = 1.0 / (self.scale * self.scale);
        Row {
            g,
            band: (lo, hi),
            w: if passing { KEEP_WEIGHT * w } else { w },
        }
    }
}

/// How far an item is from passing, in aim margins (0 when it passes); [`MISSING_SHORTFALL`] when it fails with
/// no reading or no continuous measure.
pub(crate) fn shortfall(e: &Expect, role: &Role, d: &Descriptors, pass: bool) -> f64 {
    if pass {
        return 0.0;
    }
    let Some(aim) = role.aim() else {
        return MISSING_SHORTFALL;
    };
    if aim.clip_surrogate {
        // Clipping starts at 0 dBFS; count how far the peak is on the wrong side, plus a little.
        let peak = d.get(Feature::Peak).unwrap_or(0.0);
        let want_clip = matches!(e.test, Test::Within(iv) if iv.lo >= 0.5);
        let over = if want_clip { -peak } else { peak };
        return (over.max(0.0) + 0.01) / aim.scale;
    }
    let s = e.shortfall(d);
    if s.is_finite() {
        // A failing reading inside the interval (a lower bound against `<`) still counts as failing.
        (s / aim.scale).max(0.01)
    } else {
        MISSING_SHORTFALL
    }
}

/// A dense row-major matrix: `rows × cols`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Matrix {
    pub(crate) fn zeros(rows: usize, cols: usize) -> Matrix {
        Matrix {
            rows,
            cols,
            data: vec![0.0; rows * cols],
        }
    }

    pub(crate) fn at(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }

    pub(crate) fn set(&mut self, r: usize, c: usize, v: f64) {
        self.data[r * self.cols + c] = v;
    }

    pub(crate) fn row(&self, r: usize) -> &[f64] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }

    /// `row(r) · x`.
    pub(crate) fn dot_row(&self, r: usize, x: &[f64]) -> f64 {
        self.row(r).iter().zip(x).map(|(a, b)| a * b).sum()
    }
}

/// Broyden's rank-1 update: make row `r` of `j` reproduce the observed change `dg` for the step `du`
/// (`J += (dg − J·du) duᵀ / (duᵀdu)`). Rows without an observation (`None`) are left alone.
pub(crate) fn broyden(j: &mut Matrix, du: &[f64], dg: &[Option<f64>]) {
    let nn: f64 = du.iter().map(|x| x * x).sum();
    if nn <= 1e-12 {
        return;
    }
    for (r, d) in dg.iter().enumerate() {
        let Some(d) = d else { continue };
        let err = d - j.dot_row(r, du);
        for (c, x) in du.iter().enumerate() {
            let v = j.at(r, c) + err * x / nn;
            j.set(r, c, v);
        }
    }
}

/// The inputs of one model step.
pub(crate) struct Problem<'a> {
    /// One row per item (`None`: the item is not steered).
    pub rows: &'a [Option<Row>],
    /// Jacobian, items × parameters.
    pub j: &'a Matrix,
    /// Current normalised positions.
    pub u: &'a [f64],
    /// Ridge on the step (Levenberg-Marquardt damping).
    pub lambda: f64,
}

/// The bounded Gauss-Newton step: minimise `Σ wᵢ·dist(gᵢ + Jᵢ·Δu, bandᵢ)² + λ‖Δu‖²` subject to
/// `0 ≤ u + Δu ≤ 1`, by a projected active-set iteration (parameters at a bound whose gradient pushes outward are
/// held for that iteration). Returns `Δu`.
pub(crate) fn step(p: &Problem<'_>) -> Vec<f64> {
    let n = p.u.len();
    let mut du = vec![0.0f64; n];
    for _ in 0..60 {
        let mut grad = vec![0.0f64; n];
        let mut a = vec![0.0f64; n * n];
        for (r, row) in p.rows.iter().enumerate() {
            let Some(row) = row else { continue };
            let res = row.dist(row.g + p.j.dot_row(r, &du));
            if res == 0.0 {
                continue;
            }
            let jr = p.j.row(r);
            for (i, ji) in jr.iter().enumerate() {
                if *ji == 0.0 {
                    continue;
                }
                grad[i] += row.w * ji * res;
                for (k, jk) in jr.iter().enumerate() {
                    a[i * n + k] += row.w * ji * jk;
                }
            }
        }
        for i in 0..n {
            grad[i] += p.lambda * du[i];
            a[i * n + i] += p.lambda;
        }
        let held: Vec<bool> = (0..n)
            .map(|i| {
                let x = p.u[i] + du[i];
                (x <= 0.0 && grad[i] > 0.0) || (x >= 1.0 && grad[i] < 0.0)
            })
            .collect();
        let idx: Vec<usize> = (0..n).filter(|&i| !held[i]).collect();
        if idx.is_empty() {
            break;
        }
        let m = idx.len();
        let mut sub = vec![0.0f64; m * m];
        let mut rhs = vec![0.0f64; m];
        for (r, &i) in idx.iter().enumerate() {
            rhs[r] = -grad[i];
            for (c, &k) in idx.iter().enumerate() {
                sub[r * m + c] = a[i * n + k];
            }
        }
        let delta = solve(&mut sub, &mut rhs, m);
        let mut moved = 0.0f64;
        for (r, &i) in idx.iter().enumerate() {
            let next = (du[i] + delta[r]).clamp(-p.u[i], 1.0 - p.u[i]);
            moved = moved.max((next - du[i]).abs());
            du[i] = next;
        }
        if moved < 1e-9 {
            break;
        }
    }
    du
}

/// The model's cost at `du` (the objective of [`step`] without the ridge).
#[cfg(test)]
pub(crate) fn cost(p: &Problem<'_>, du: &[f64]) -> f64 {
    p.rows
        .iter()
        .enumerate()
        .filter_map(|(r, row)| {
            let row = (*row)?;
            let d = row.dist(row.g + p.j.dot_row(r, du));
            Some(row.w * d * d)
        })
        .sum()
}

/// Solve `a·x = b` (`n × n`, row-major) by Gaussian elimination with partial pivoting. Singular directions get 0.
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> Vec<f64> {
    for c in 0..n {
        let mut piv = c;
        for r in c + 1..n {
            if a[r * n + c].abs() > a[piv * n + c].abs() {
                piv = r;
            }
        }
        if piv != c {
            for k in 0..n {
                a.swap(c * n + k, piv * n + k);
            }
            b.swap(c, piv);
        }
        let d = a[c * n + c];
        if d.abs() < 1e-14 {
            continue;
        }
        for r in 0..n {
            if r == c {
                continue;
            }
            let f = a[r * n + c] / d;
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r * n + k] -= f * a[c * n + k];
            }
            b[r] -= f * b[c];
        }
    }
    (0..n)
        .map(|i| {
            let d = a[i * n + i];
            if d.abs() < 1e-14 { 0.0 } else { b[i] / d }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::expect::parse_item;

    fn aim(text: &str) -> Aim {
        *Role::of(&parse_item(text).unwrap()).aim().unwrap()
    }

    #[test]
    fn bands_aim_inside_for_failing_items_and_hold_passing_ones() {
        let a = aim("brightness > 2kHz");
        assert!(a.log && (a.lo - libm::log(2000.0)).abs() < 1e-12 && a.hi.is_infinite());
        let r = a.row(libm::log(1000.0), false);
        assert!((r.band.0 - libm::log(2200.0)).abs() < 1e-9 && r.band.1.is_infinite());
        let s = aim("sustain > -4dB");
        assert_eq!(s.row(-6.0, false).band, (-2.5, f64::INFINITY));
        // Passing comfortably: half a margin inside. Passing thinly: the current value is the edge.
        assert_eq!(s.row(0.0, true).band, (-3.25, f64::INFINITY));
        assert_eq!(s.row(-3.9, true).band, (-3.9, f64::INFINITY));
        assert_eq!(s.row(0.0, true).w, KEEP_WEIGHT * s.row(-6.0, false).w);
        let w = aim("beating in [3Hz, 3.1Hz]");
        let b = w.row(1.0, false).band;
        assert_eq!(
            b.0, b.1,
            "a window narrower than the margin collapses to its middle"
        );
        let p = aim("pitch = A4 ± 10ct");
        assert_eq!((p.row(20.0, false).band, p.scale), ((-7.0, 7.0), 3.0));
        let c = aim("clipping = false");
        assert!(c.clip_surrogate && c.row(3.0, false).band.1 == -1.0);
        assert_eq!(
            Role::of(&parse_item("tremolo == null").unwrap()),
            Role::NullTest
        );
    }

    fn row(lo: f64, hi: f64, g: f64) -> Row {
        Row {
            g,
            band: (lo, hi),
            w: 100.0,
        }
    }

    #[test]
    fn step_reaches_the_band_and_keeps_others_inside() {
        // Item 0 needs +1 along p0; item 1 sits at its edge and moves with p0 too, so the step must use p1 to hold
        // it.
        let rows = [
            Some(row(1.0, f64::INFINITY, 0.0)),
            Some(row(f64::NEG_INFINITY, 0.0, 0.0)),
        ];
        let mut j = Matrix::zeros(2, 2);
        j.set(0, 0, 4.0);
        j.set(1, 0, 2.0);
        j.set(1, 1, 2.0);
        let u = [0.5, 0.5];
        let p = Problem {
            rows: &rows,
            j: &j,
            u: &u,
            lambda: 1e-6,
        };
        let du = step(&p);
        assert!(cost(&p, &du) < 1e-6, "{du:?}");
        assert!(du[0] > 0.24 && du[1] < -0.24, "{du:?}");
        // Bounds hold.
        let tight = Problem {
            u: &[0.9, 0.95],
            ..p
        };
        let du = step(&tight);
        assert!(du[0] <= 0.1 + 1e-12 && 0.95 + du[1] >= -1e-12);
    }

    #[test]
    fn broyden_matches_the_observed_change() {
        let mut j = Matrix::zeros(2, 2);
        j.set(0, 0, 1.0);
        let du = [0.1, 0.2];
        broyden(&mut j, &du, &[Some(0.5), None]);
        assert!((j.dot_row(0, &du) - 0.5).abs() < 1e-12);
        assert_eq!(j.row(1), &[0.0, 0.0]);
    }

    #[test]
    fn solve_handles_pivoting_and_singular_systems() {
        let mut a = vec![0.0, 2.0, 1.0, 1.0];
        let mut b = vec![4.0, 3.0];
        assert_eq!(solve(&mut a, &mut b, 2), vec![1.0, 2.0]);
        let mut a = vec![1.0, 0.0, 0.0, 0.0];
        let mut b = vec![2.0, 5.0];
        assert_eq!(solve(&mut a, &mut b, 2), vec![2.0, 0.0]);
    }
}
