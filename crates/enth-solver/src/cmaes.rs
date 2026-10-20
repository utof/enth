//! **Owner: lane MT** (match). Filled from lane S0r's stub.
//!
//! CMA-ES (Hansen 2016, "The CMA Evolution Strategy: A Tutorial", default parameters, positive weights), ported from
//! the Phase 0 metric spike (`spikes/spike-metric/src/cmaes.rs`, in-repo, MIT; `docs/spikes/metric.md`) into the
//! solver for `enth match` and the solver floors on golden challenges. Box bounds are handled by clamping plus a
//! quadratic penalty on the distance between the sample and its clamped point.
//!
//! It is an **ask-tell** optimiser: [`Cmaes::ask`] samples a generation, the caller evaluates it however it likes (a
//! rayon pool, a worker per core, one thread) and hands the fitness values back **in the same order** to
//! [`Cmaes::tell`]. The optimiser never sees a thread, a clock or a file, so a run is a pure function of its inputs:
//!
//! - **Deterministic.** The only randomness is a [`Rng`] seeded by the caller (`SplitMix64`, `enth_dsp::util::rng`),
//!   drawn in a fixed order; normals are Box-Muller through `libm`, and every other operation is `+ − × ÷ √` in a
//!   fixed summation order (no `mul_add`, no `powi`, no reassociation), so the stream of candidates is the same on
//!   every platform and for any worker count.
//! - **Bounded.** [`Cmaes::ask`] returns points already clamped into the box, so a caller can render them as they
//!   are.
//! - **Budgeted by the caller.** There is no evaluation counter here: the caller stops asking. [`Cmaes::stop_reason`]
//!   only says when the *run* is worth abandoning (collapsed, ill-conditioned or stalled), which is when
//!   [`search`](crate::search) restarts it with a larger population (IPOP).
//!
//! ```
//! use enth_solver::cmaes::Cmaes;
//!
//! // Minimise a sphere centred at (0.3, 0.7) inside the unit square.
//! let mut es = Cmaes::new(vec![0.5, 0.5], 0.3, 8, vec![0.0; 2], vec![1.0; 2], 42).unwrap();
//! let mut best = f64::INFINITY;
//! for _ in 0..60 {
//!     let xs = es.ask();
//!     let fit: Vec<f64> = xs.iter().map(|x| (x[0] - 0.3) * (x[0] - 0.3) + (x[1] - 0.7) * (x[1] - 0.7)).collect();
//!     best = fit.iter().copied().fold(best, f64::min);
//!     es.tell(&fit).unwrap();
//! }
//! assert!(best < 1e-6);
//! ```

#![allow(
    clippy::needless_range_loop,
    reason = "dense matrix code indexes several arrays by the same i, j, k, as in Hansen's tutorial; iterator forms hide the algebra"
)]

use core::f64::consts::PI;
use core::fmt;

use enth_dsp::util::rng::Rng;

/// Restart when `sigma * max(D)` falls below this (the search has collapsed onto a point).
pub const TOL_X: f64 = 1e-4;
/// Restart when the covariance condition number exceeds this.
pub const MAX_CONDITION: f64 = 1e14;
/// Restart when the best penalised fitness of a generation hasn't improved by 1 % for this many generations.
pub const STALL_GENERATIONS: usize = 15;

/// The fitness a non-finite value is replaced by, so a failed evaluation ranks last instead of poisoning the sort
/// and the penalty arithmetic.
pub const WORST_FITNESS: f64 = 1e300;

/// Why a run should be abandoned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// `sigma · max(D)` fell below [`TOL_X`]: the distribution collapsed onto a point.
    TolX,
    /// The covariance condition number exceeded [`MAX_CONDITION`] (or the step size is no longer finite).
    Condition,
    /// The best fitness of a generation hasn't improved by 1 % for [`STALL_GENERATIONS`] generations.
    Stall,
}

/// A refused setup or a mismatched [`Cmaes::tell`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CmaesError {
    /// No dimensions.
    Empty,
    /// A population smaller than 4.
    Lambda,
    /// The bounds do not have one entry per dimension, or `lower >= upper` (or not finite) somewhere.
    Bounds,
    /// The start point or the step size is not finite (the step must be positive).
    Start,
    /// [`Cmaes::tell`] got a different number of fitness values than the last [`Cmaes::ask`] returned (or none was
    /// asked).
    FitnessCount {
        /// What `ask` returned.
        expected: usize,
        /// What `tell` got.
        got: usize,
    },
}

impl fmt::Display for CmaesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmaesError::Empty => f.write_str("CMA-ES needs at least one dimension"),
            CmaesError::Lambda => f.write_str("CMA-ES needs a population of at least 4"),
            CmaesError::Bounds => {
                f.write_str("CMA-ES bounds need one finite `lower < upper` pair per dimension")
            }
            CmaesError::Start => {
                f.write_str("CMA-ES needs a finite start point and a positive step")
            }
            CmaesError::FitnessCount { expected, got } => write!(
                f,
                "tell() needs one fitness value per asked point: expected {expected}, got {got}"
            ),
        }
    }
}

impl std::error::Error for CmaesError {}

/// Ask-tell CMA-ES over a box. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct Cmaes {
    n: usize,
    lambda: usize,
    mu: usize,
    weights: Vec<f64>,
    mueff: f64,
    cc: f64,
    cs: f64,
    c1: f64,
    cmu: f64,
    damps: f64,
    chi_n: f64,
    mean: Vec<f64>,
    sigma: f64,
    cov: Vec<Vec<f64>>,
    b: Vec<Vec<f64>>,
    d: Vec<f64>,
    pc: Vec<f64>,
    ps: Vec<f64>,
    generation: usize,
    lower: Vec<f64>,
    upper: Vec<f64>,
    penalty_weight: f64,
    rng: Rng,
    /// Last asked batch: unclamped samples (what the distribution produced) and clamped points (what was evaluated).
    raw: Vec<Vec<f64>>,
    clamped: Vec<Vec<f64>>,
    run_best: f64,
    last_improve_gen: usize,
}

/// Eigen-decomposition of a symmetric matrix by cyclic Jacobi rotations: `(eigenvalues, eigenvectors as columns)`.
fn jacobi_eigen(a: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = a.len();
    let mut m = a.to_vec();
    let mut v = vec![vec![0.0; n]; n];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _sweep in 0..50 {
        let mut off = 0.0;
        for i in 0..n {
            for j in i + 1..n {
                off += m[i][j] * m[i][j];
            }
        }
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if m[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
                let t = if theta == 0.0 {
                    1.0
                } else {
                    theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
                };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (mkp, mkq) = (m[k][p], m[k][q]);
                    m[k][p] = c * mkp - s * mkq;
                    m[k][q] = s * mkp + c * mkq;
                }
                for k in 0..n {
                    let (mpk, mqk) = (m[p][k], m[q][k]);
                    m[p][k] = c * mpk - s * mqk;
                    m[q][k] = s * mpk + c * mqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k][p], v[k][q]);
                    v[k][p] = c * vkp - s * vkq;
                    v[k][q] = s * vkp + c * vkq;
                }
            }
        }
    }
    ((0..n).map(|i| m[i][i]).collect(), v)
}

/// A standard normal by Box-Muller through `libm`, so the same stream gives the same numbers on every platform.
fn normal(rng: &mut Rng) -> f64 {
    let u1 = rng.unit_f64().max(1e-300);
    let u2 = rng.unit_f64();
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * PI * u2)
}

/// Hansen's default population size for dimension `n`: `4 + floor(3 ln n)`.
#[must_use]
pub fn default_lambda(n: usize) -> usize {
    4 + libm::floor(3.0 * libm::log(n.max(1) as f64)) as usize
}

impl Cmaes {
    /// A new run.
    ///
    /// `mean` is the start point, `sigma` the initial step size in the units of the bounds (0.3 is a third of a unit
    /// box), `lambda` the population per generation (at least 4; [`default_lambda`] is Hansen's), `lower` and `upper`
    /// the box, and `seed` the run's only source of randomness.
    ///
    /// # Errors
    /// [`CmaesError`] for no dimensions, `lambda < 4`, bounds that do not match or are not ordered, or a start point
    /// or step that is not finite.
    pub fn new(
        mean: Vec<f64>,
        sigma: f64,
        lambda: usize,
        lower: Vec<f64>,
        upper: Vec<f64>,
        seed: u64,
    ) -> Result<Self, CmaesError> {
        let n = mean.len();
        if n == 0 {
            return Err(CmaesError::Empty);
        }
        if lambda < 4 {
            return Err(CmaesError::Lambda);
        }
        if lower.len() != n
            || upper.len() != n
            || lower
                .iter()
                .zip(&upper)
                .any(|(l, u)| !(l.is_finite() && u.is_finite() && l < u))
        {
            return Err(CmaesError::Bounds);
        }
        if !(sigma.is_finite() && sigma > 0.0 && mean.iter().all(|m| m.is_finite())) {
            return Err(CmaesError::Start);
        }
        let mu = lambda / 2;
        let mut weights: Vec<f64> = (0..mu)
            .map(|i| libm::log(f64::midpoint(lambda as f64, 1.0)) - libm::log((i + 1) as f64))
            .collect();
        let sw: f64 = weights.iter().sum();
        for w in &mut weights {
            *w /= sw;
        }
        let mueff = 1.0 / weights.iter().map(|w| w * w).sum::<f64>();
        let nf = n as f64;
        let cc = (4.0 + mueff / nf) / (nf + 4.0 + 2.0 * mueff / nf);
        let cs = (mueff + 2.0) / (nf + mueff + 5.0);
        let c1 = 2.0 / ((nf + 1.3) * (nf + 1.3) + mueff);
        let cmu =
            (1.0 - c1).min(2.0 * (mueff - 2.0 + 1.0 / mueff) / ((nf + 2.0) * (nf + 2.0) + mueff));
        let damps = 1.0 + 2.0 * (((mueff - 1.0) / (nf + 1.0)).sqrt() - 1.0).max(0.0) + cs;
        let chi_n = nf.sqrt() * (1.0 - 1.0 / (4.0 * nf) + 1.0 / (21.0 * nf * nf));
        let mut cov = vec![vec![0.0; n]; n];
        let mut b = vec![vec![0.0; n]; n];
        for i in 0..n {
            cov[i][i] = 1.0;
            b[i][i] = 1.0;
        }
        Ok(Self {
            n,
            lambda,
            mu,
            weights,
            mueff,
            cc,
            cs,
            c1,
            cmu,
            damps,
            chi_n,
            mean,
            sigma,
            cov,
            b,
            d: vec![1.0; n],
            pc: vec![0.0; n],
            ps: vec![0.0; n],
            generation: 0,
            lower,
            upper,
            penalty_weight: 1.0,
            rng: Rng::new(seed),
            raw: Vec::new(),
            clamped: Vec::new(),
            run_best: f64::INFINITY,
            last_improve_gen: 0,
        })
    }

    /// Dimensions.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.n
    }

    /// Population per generation.
    #[must_use]
    pub fn lambda(&self) -> usize {
        self.lambda
    }

    /// The distribution's mean: the weighted centre of the best candidates of the last generation.
    #[must_use]
    pub fn mean(&self) -> &[f64] {
        &self.mean
    }

    /// The current step size.
    #[must_use]
    pub fn sigma(&self) -> f64 {
        self.sigma
    }

    /// Generations told so far.
    #[must_use]
    pub fn generation(&self) -> usize {
        self.generation
    }

    /// Weight of the quadratic penalty `Σ (raw − clamped)²` added to the fitness of out-of-bounds samples (default 1).
    /// Scale it with the fitness: a distance that runs 0.1–0.5 wants a weight near 1, a fitness in the thousands a
    /// weight in the thousands.
    pub fn set_penalty_weight(&mut self, w: f64) {
        self.penalty_weight = w;
    }

    /// Sample `lambda` candidates, in a fixed order. The returned points are inside the bounds (clamped); evaluate
    /// them and pass the fitness (lower is better) to [`Cmaes::tell`] in the same order.
    pub fn ask(&mut self) -> Vec<Vec<f64>> {
        let n = self.n;
        self.raw.clear();
        self.clamped.clear();
        for _ in 0..self.lambda {
            let z: Vec<f64> = (0..n).map(|_| normal(&mut self.rng)).collect();
            let mut x = self.mean.clone();
            for i in 0..n {
                let mut acc = 0.0;
                for j in 0..n {
                    acc += self.b[i][j] * self.d[j] * z[j];
                }
                x[i] += self.sigma * acc;
            }
            let c: Vec<f64> = (0..n)
                .map(|i| x[i].clamp(self.lower[i], self.upper[i]))
                .collect();
            self.raw.push(x);
            self.clamped.push(c);
        }
        self.clamped.clone()
    }

    /// Report the fitness of the batch returned by the last [`Cmaes::ask`] and update the distribution. A
    /// non-finite fitness counts as [`WORST_FITNESS`].
    ///
    /// # Errors
    /// [`CmaesError::FitnessCount`] if `fitness` does not have one value per asked point; nothing changes.
    pub fn tell(&mut self, fitness: &[f64]) -> Result<(), CmaesError> {
        if fitness.len() != self.raw.len() || self.raw.is_empty() {
            return Err(CmaesError::FitnessCount {
                expected: self.raw.len(),
                got: fitness.len(),
            });
        }
        let n = self.n;
        let fit: Vec<f64> = fitness
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let pen: f64 = self.raw[i]
                    .iter()
                    .zip(&self.clamped[i])
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                let f = if f.is_finite() { *f } else { WORST_FITNESS };
                f + self.penalty_weight * pen
            })
            .collect();
        let mut idx: Vec<usize> = (0..fit.len()).collect();
        idx.sort_by(|&a, &b| fit[a].total_cmp(&fit[b]).then(a.cmp(&b)));
        let gen_best = fit[idx[0]];
        if gen_best < self.run_best * 0.99 {
            self.run_best = gen_best;
            self.last_improve_gen = self.generation;
        }
        let old = self.mean.clone();
        let mut new_mean = vec![0.0; n];
        for k in 0..self.mu {
            for i in 0..n {
                new_mean[i] += self.weights[k] * self.raw[idx[k]][i];
            }
        }
        self.mean = new_mean;
        let y: Vec<f64> = (0..n)
            .map(|i| (self.mean[i] - old[i]) / self.sigma)
            .collect();
        // C^{-1/2} y = B D^-1 B^T y
        let mut bty = vec![0.0; n];
        for j in 0..n {
            for i in 0..n {
                bty[j] += self.b[i][j] * y[i];
            }
            bty[j] /= self.d[j];
        }
        let mut cinv_y = vec![0.0; n];
        for i in 0..n {
            for j in 0..n {
                cinv_y[i] += self.b[i][j] * bty[j];
            }
        }
        let csn = (self.cs * (2.0 - self.cs) * self.mueff).sqrt();
        for i in 0..n {
            self.ps[i] = (1.0 - self.cs) * self.ps[i] + csn * cinv_y[i];
        }
        self.generation += 1;
        let psn = self.ps.iter().map(|v| v * v).sum::<f64>().sqrt();
        let hsig = psn
            / (1.0 - libm::pow(1.0 - self.cs, 2.0 * self.generation as f64)).sqrt()
            / self.chi_n
            < 1.4 + 2.0 / (n as f64 + 1.0);
        let hs = if hsig { 1.0 } else { 0.0 };
        let ccn = (self.cc * (2.0 - self.cc) * self.mueff).sqrt();
        for i in 0..n {
            self.pc[i] = (1.0 - self.cc) * self.pc[i] + hs * ccn * y[i];
        }
        let artmp: Vec<Vec<f64>> = (0..self.mu)
            .map(|k| {
                (0..n)
                    .map(|i| (self.raw[idx[k]][i] - old[i]) / self.sigma)
                    .collect()
            })
            .collect();
        for i in 0..n {
            for j in 0..=i {
                let mut rmu = 0.0;
                for k in 0..self.mu {
                    rmu += self.weights[k] * artmp[k][i] * artmp[k][j];
                }
                let v = (1.0 - self.c1 - self.cmu) * self.cov[i][j]
                    + self.c1
                        * (self.pc[i] * self.pc[j]
                            + (1.0 - hs) * self.cc * (2.0 - self.cc) * self.cov[i][j])
                    + self.cmu * rmu;
                self.cov[i][j] = v;
                self.cov[j][i] = v;
            }
        }
        self.sigma *= libm::exp((self.cs / self.damps) * (psn / self.chi_n - 1.0));
        let (ev, b) = jacobi_eigen(&self.cov);
        self.d = ev.iter().map(|e| e.max(1e-20).sqrt()).collect();
        self.b = b;
        self.raw.clear();
        self.clamped.clear();
        Ok(())
    }

    /// Whether this run should be abandoned (collapsed, ill-conditioned or stalled).
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        let dmax = self.d.iter().copied().fold(0.0, f64::max);
        let dmin = self.d.iter().copied().fold(f64::INFINITY, f64::min);
        let cond = (dmax / dmin) * (dmax / dmin);
        if !self.sigma.is_finite() || !cond.is_finite() || cond > MAX_CONDITION {
            Some(StopReason::Condition)
        } else if self.sigma * dmax < TOL_X {
            Some(StopReason::TolX)
        } else if self.generation - self.last_improve_gen > STALL_GENERATIONS {
            Some(StopReason::Stall)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a run by hand: `gens` generations, returning (best x, best f, every fitness in order).
    fn run(
        es: &mut Cmaes,
        gens: usize,
        mut f: impl FnMut(&[f64]) -> f64,
    ) -> (Vec<f64>, f64, Vec<f64>) {
        let mut best = (Vec::new(), f64::INFINITY);
        let mut all = Vec::new();
        for _ in 0..gens {
            let xs = es.ask();
            let fit: Vec<f64> = xs.iter().map(|x| f(x)).collect();
            for (x, &v) in xs.iter().zip(&fit) {
                if v < best.1 {
                    best = (x.clone(), v);
                }
            }
            all.extend(fit.iter().copied());
            es.tell(&fit).unwrap();
        }
        (best.0, best.1, all)
    }

    fn sphere(x: &[f64]) -> f64 {
        x.iter().map(|v| v * v).sum()
    }

    fn rosenbrock(x: &[f64]) -> f64 {
        x.windows(2)
            .map(|w| {
                100.0 * (w[1] - w[0] * w[0]) * (w[1] - w[0] * w[0]) + (1.0 - w[0]) * (1.0 - w[0])
            })
            .sum()
    }

    #[test]
    fn minimises_the_sphere_to_tolerance() {
        let n = 5;
        let mut es = Cmaes::new(
            vec![2.0; n],
            1.0,
            default_lambda(n),
            vec![-5.0; n],
            vec![5.0; n],
            7,
        )
        .unwrap();
        let (x, f, _) = run(&mut es, 150, sphere);
        assert!(f < 1e-12, "sphere best {f}");
        assert!(x.iter().all(|v| v.abs() < 1e-5), "{x:?}");
    }

    #[test]
    fn minimises_rosenbrock_to_tolerance() {
        let n = 4;
        let mut es = Cmaes::new(vec![-1.0; n], 0.5, 12, vec![-3.0; n], vec![3.0; n], 11).unwrap();
        let (x, f, _) = run(&mut es, 600, rosenbrock);
        assert!(f < 1e-8, "rosenbrock best {f}");
        assert!(x.iter().all(|v| (v - 1.0).abs() < 1e-3), "{x:?}");
    }

    #[test]
    fn minimises_an_ill_conditioned_ellipsoid() {
        // Axes scaled by 1, 10, 100: the covariance has to learn the shape.
        let n = 3;
        let f = |x: &[f64]| -> f64 {
            x.iter()
                .enumerate()
                .map(|(i, v)| {
                    let s = [1.0, 10.0, 100.0][i];
                    (s * v) * (s * v)
                })
                .sum()
        };
        let mut es = Cmaes::new(vec![1.0; n], 0.5, 10, vec![-5.0; n], vec![5.0; n], 3).unwrap();
        let (_, best, _) = run(&mut es, 300, f);
        assert!(best < 1e-10, "ellipsoid best {best}");
    }

    #[test]
    fn the_same_seed_gives_the_same_run_bit_for_bit() {
        let make =
            |seed| Cmaes::new(vec![0.5; 6], 0.3, 10, vec![0.0; 6], vec![1.0; 6], seed).unwrap();
        let (mut a, mut b, mut c) = (make(5), make(5), make(6));
        let (xa, fa, all_a) = run(&mut a, 40, rosenbrock);
        let (xb, fb, all_b) = run(&mut b, 40, rosenbrock);
        let (_, _, all_c) = run(&mut c, 40, rosenbrock);
        assert_eq!(xa, xb);
        assert_eq!(fa.to_bits(), fb.to_bits());
        assert_eq!(
            all_a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            all_b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        assert_ne!(
            all_a[0].to_bits(),
            all_c[0].to_bits(),
            "another seed, another stream"
        );
        assert_eq!(a.mean(), b.mean());
        assert_eq!(a.sigma().to_bits(), b.sigma().to_bits());
    }

    #[test]
    fn samples_stay_inside_the_box_and_an_outside_optimum_lands_on_the_bound() {
        let n = 3;
        let mut es = Cmaes::new(vec![0.5; n], 0.4, 10, vec![0.0; n], vec![1.0; n], 9).unwrap();
        // The optimum (2, 2, 2) is outside the unit cube: the best feasible point is the corner (1, 1, 1).
        let mut best = (vec![], f64::INFINITY);
        for _ in 0..120 {
            let xs = es.ask();
            assert!(xs.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
            let fit: Vec<f64> = xs
                .iter()
                .map(|x| x.iter().map(|v| (v - 2.0) * (v - 2.0)).sum())
                .collect();
            for (x, &v) in xs.iter().zip(&fit) {
                if v < best.1 {
                    best = (x.clone(), v);
                }
            }
            es.tell(&fit).unwrap();
        }
        assert!(best.0.iter().all(|v| *v > 0.999), "{:?}", best.0);
    }

    #[test]
    fn a_collapsed_run_asks_to_be_restarted() {
        let n = 2;
        let mut es = Cmaes::new(vec![0.5; n], 0.2, 8, vec![0.0; n], vec![1.0; n], 1).unwrap();
        let mut stopped = None;
        for _ in 0..2000 {
            let xs = es.ask();
            let fit: Vec<f64> = xs.iter().map(|x| sphere(x)).collect();
            es.tell(&fit).unwrap();
            if let Some(r) = es.stop_reason() {
                stopped = Some(r);
                break;
            }
        }
        assert!(stopped.is_some(), "a converged run must signal a restart");
        // A flat landscape stalls instead.
        let mut flat = Cmaes::new(vec![0.5; n], 0.2, 8, vec![0.0; n], vec![1.0; n], 1).unwrap();
        let mut reason = None;
        for _ in 0..(STALL_GENERATIONS + 4) {
            let xs = flat.ask();
            flat.tell(&vec![1.0; xs.len()]).unwrap();
            reason = flat.stop_reason().or(reason);
        }
        assert_eq!(reason, Some(StopReason::Stall));
    }

    #[test]
    fn bad_setups_and_mismatched_tells_are_errors_not_panics() {
        let ok = || Cmaes::new(vec![0.5; 2], 0.3, 8, vec![0.0; 2], vec![1.0; 2], 1);
        assert!(ok().is_ok());
        assert_eq!(
            Cmaes::new(vec![], 0.3, 8, vec![], vec![], 1).unwrap_err(),
            CmaesError::Empty
        );
        assert_eq!(
            Cmaes::new(vec![0.5; 2], 0.3, 3, vec![0.0; 2], vec![1.0; 2], 1).unwrap_err(),
            CmaesError::Lambda
        );
        assert_eq!(
            Cmaes::new(vec![0.5; 2], 0.3, 8, vec![0.0; 2], vec![1.0], 1).unwrap_err(),
            CmaesError::Bounds
        );
        assert_eq!(
            Cmaes::new(vec![0.5; 2], 0.3, 8, vec![1.0; 2], vec![0.0; 2], 1).unwrap_err(),
            CmaesError::Bounds
        );
        assert_eq!(
            Cmaes::new(vec![f64::NAN; 2], 0.3, 8, vec![0.0; 2], vec![1.0; 2], 1).unwrap_err(),
            CmaesError::Start
        );
        assert_eq!(
            Cmaes::new(vec![0.5; 2], 0.0, 8, vec![0.0; 2], vec![1.0; 2], 1).unwrap_err(),
            CmaesError::Start
        );
        let mut es = ok().unwrap();
        assert!(matches!(
            es.tell(&[1.0; 8]),
            Err(CmaesError::FitnessCount {
                expected: 0,
                got: 8
            })
        ));
        es.ask();
        assert!(matches!(
            es.tell(&[1.0; 3]),
            Err(CmaesError::FitnessCount {
                expected: 8,
                got: 3
            })
        ));
        // Failed evaluations rank last and never poison the state.
        let mut fit = vec![1.0; 8];
        fit[2] = f64::NAN;
        fit[5] = f64::INFINITY;
        assert!(es.tell(&fit).is_ok());
        assert!(es.mean().iter().all(|v| v.is_finite()) && es.sigma().is_finite());
    }

    #[test]
    fn default_lambda_is_hansens() {
        assert_eq!(default_lambda(1), 4);
        assert_eq!(default_lambda(2), 6);
        assert_eq!(default_lambda(9), 10);
        assert_eq!(default_lambda(10), 10);
        assert_eq!(default_lambda(30), 14);
    }
}
