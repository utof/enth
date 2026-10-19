//! Ask-tell CMA-ES (Hansen 2016, "The CMA Evolution Strategy: A Tutorial", default parameters, positive weights) with
//! box bounds handled by clamping plus a quadratic penalty.
//!
//! Single-threaded and pure: the caller evaluates the candidates however it likes (threads, GPU, ...) and returns
//! fitness values in input order. No clocks, no I/O, no global RNG; transcendentals come from `libm`.

use crate::rng::Rng;

/// Restart when `sigma * max(D)` falls below this (the search has collapsed onto a point).
pub const TOL_X: f64 = 1e-4;
/// Restart when the covariance condition number exceeds this.
pub const MAX_CONDITION: f64 = 1e14;
/// Restart when the best penalised fitness of a generation hasn't improved by 1 % for this many generations.
pub const STALL_GENERATIONS: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    TolX,
    Condition,
    Stall,
}

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
    for i in 0..n {
        v[i][i] = 1.0;
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

/// Hansen's default population size for dimension `n`: `4 + floor(3 ln n)`.
#[must_use]
pub fn default_lambda(n: usize) -> usize {
    4 + libm::floor(3.0 * libm::log(n as f64)) as usize
}

impl Cmaes {
    /// `mean`: start point; `sigma`: initial step size (in the units of the bounds); `lower`/`upper`: per-coordinate box.
    #[must_use]
    pub fn new(
        mean: Vec<f64>,
        sigma: f64,
        lambda: usize,
        lower: Vec<f64>,
        upper: Vec<f64>,
        seed: u64,
    ) -> Self {
        let n = mean.len();
        assert!(
            n > 0 && lambda >= 4 && lower.len() == n && upper.len() == n,
            "bad CMA-ES setup"
        );
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
        let c1 = 2.0 / ((nf + 1.3).powi(2) + mueff);
        let cmu = (1.0 - c1).min(2.0 * (mueff - 2.0 + 1.0 / mueff) / ((nf + 2.0).powi(2) + mueff));
        let damps = 1.0 + 2.0 * (((mueff - 1.0) / (nf + 1.0)).sqrt() - 1.0).max(0.0) + cs;
        let chi_n = nf.sqrt() * (1.0 - 1.0 / (4.0 * nf) + 1.0 / (21.0 * nf * nf));
        let mut cov = vec![vec![0.0; n]; n];
        let mut b = vec![vec![0.0; n]; n];
        for i in 0..n {
            cov[i][i] = 1.0;
            b[i][i] = 1.0;
        }
        Self {
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
        }
    }

    #[must_use]
    pub fn lambda(&self) -> usize {
        self.lambda
    }

    #[must_use]
    pub fn mean(&self) -> &[f64] {
        &self.mean
    }

    #[must_use]
    pub fn sigma(&self) -> f64 {
        self.sigma
    }

    #[must_use]
    pub fn generation(&self) -> usize {
        self.generation
    }

    /// Weight of the quadratic penalty `sum (raw - clamped)^2` added to the fitness of out-of-bounds samples.
    pub fn set_penalty_weight(&mut self, w: f64) {
        self.penalty_weight = w;
    }

    /// Sample `lambda` candidates. The returned points are inside the bounds (clamped); evaluate them and pass the
    /// fitness (lower is better) to [`Cmaes::tell`] in the same order.
    pub fn ask(&mut self) -> Vec<Vec<f64>> {
        let n = self.n;
        self.raw.clear();
        self.clamped.clear();
        for _ in 0..self.lambda {
            let z: Vec<f64> = (0..n).map(|_| self.rng.normal()).collect();
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

    /// Report the fitness of the batch returned by the last [`Cmaes::ask`] and update the distribution.
    pub fn tell(&mut self, fitness: &[f64]) {
        assert_eq!(
            fitness.len(),
            self.raw.len(),
            "tell() needs one fitness value per asked point"
        );
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
                f + self.penalty_weight * pen
            })
            .collect();
        let mut idx: Vec<usize> = (0..fit.len()).collect();
        idx.sort_by(|&a, &b| fit[a].total_cmp(&fit[b]));
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
        let hsig =
            psn / (1.0 - (1.0 - self.cs).powi(2 * self.generation as i32)).sqrt() / self.chi_n
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
    }

    /// Whether this run should be abandoned (collapsed, ill-conditioned or stalled).
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        let dmax = self.d.iter().copied().fold(0.0, f64::max);
        let dmin = self.d.iter().copied().fold(f64::INFINITY, f64::min);
        if self.sigma * dmax < TOL_X {
            Some(StopReason::TolX)
        } else if (dmax / dmin).powi(2) > MAX_CONDITION {
            Some(StopReason::Condition)
        } else if self.generation - self.last_improve_gen > STALL_GENERATIONS {
            Some(StopReason::Stall)
        } else {
            None
        }
    }
}
