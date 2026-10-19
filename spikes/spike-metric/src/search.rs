//! Restarting ask-tell search over the 10-parameter template: CMA-ES on the nine continuous parameters, with the
//! categorical waveform fixed per restart and switched at restarts (cycling in the heuristic's order when there is one),
//! IPOP (or BIPOP-style) population schedules, and an optional heuristic start.
//!
//! Pure and single-threaded: [`Search::ask`] hands out candidates, the caller evaluates them however it likes
//! ([`BatchEval`]; results must come back in input order) and returns the fitness to [`Search::tell`]. No clocks,
//! no I/O, no global RNG. Wall-clock caps are the caller's job (stop calling `ask`).

use crate::cmaes::{Cmaes, default_lambda};
use crate::render::{N_PARAMS, Wave};
use crate::rng::Rng;

/// Continuous parameters: everything except the waveform.
pub const N_CONT: usize = N_PARAMS - 1;

/// The caller owns evaluation. Must return one fitness per input, in input order.
pub trait BatchEval {
    fn eval(&mut self, xs: &[Vec<f64>]) -> Vec<f64>;
}

/// Heuristic start derived from the target audio alone.
#[derive(Clone, Debug)]
pub struct Start {
    pub wave_order: [Wave; 3],
    /// The nine continuous parameters, normalised.
    pub cont: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restarts {
    /// Population doubles after every full waveform cycle (3 restarts).
    Ipop,
    /// Large (IPOP-schedule) and small-population/small-step restarts, whichever regime has used fewer evaluations.
    Bipop,
}

#[derive(Clone, Debug)]
pub struct SearchCfg {
    pub seed: u64,
    pub start: Option<Start>,
    /// Initial step size around the heuristic start.
    pub sigma_start: f64,
    /// Initial step size for random restarts.
    pub sigma_random: f64,
    /// Population size of the first run; 0 = Hansen's default for nine dimensions.
    pub lambda0: usize,
    /// Evaluation cap (checked after each generation).
    pub budget: usize,
    /// Stop as soon as the best fitness is at or below this.
    pub stop_at: Option<f64>,
    pub restarts: Restarts,
}

impl Default for SearchCfg {
    fn default() -> Self {
        Self {
            seed: 1,
            start: None,
            sigma_start: 0.15,
            sigma_random: 0.3,
            lambda0: 0,
            budget: 5000,
            stop_at: None,
            restarts: Restarts::Ipop,
        }
    }
}

/// Population size for `workers` parallel evaluations: Hansen's default rounded up to a multiple of the worker
/// count, so every generation fills the work queue (8 workers: 16).
#[must_use]
pub fn lambda_for_workers(workers: usize) -> usize {
    let base = default_lambda(N_CONT);
    if workers <= 1 {
        base
    } else {
        base.div_ceil(workers) * workers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    StartPoint,
    Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Regime {
    Large,
    Small,
}

pub struct Search {
    cfg: SearchCfg,
    lambda0: usize,
    rng: Rng,
    es: Option<Cmaes>,
    phase: Phase,
    wave: Wave,
    regime: Regime,
    restarts: usize,
    run_evals: usize,
    evals_large: usize,
    evals_small: usize,
    evals: usize,
    best_f: f64,
    best_x: Vec<f64>,
    start_f: Option<f64>,
    pending: Vec<Vec<f64>>,
}

impl Search {
    #[must_use]
    pub fn new(cfg: SearchCfg) -> Self {
        let lambda0 = if cfg.lambda0 > 0 {
            cfg.lambda0
        } else {
            default_lambda(N_CONT)
        };
        let phase = if cfg.start.is_some() {
            Phase::StartPoint
        } else {
            Phase::Generation
        };
        Self {
            rng: Rng::new(cfg.seed ^ 0x00AB_CDEF),
            lambda0,
            es: None,
            phase,
            wave: Wave::Saw,
            regime: Regime::Large,
            restarts: 0,
            run_evals: 0,
            evals_large: 0,
            evals_small: 0,
            evals: 0,
            best_f: f64::INFINITY,
            best_x: vec![0.5; N_PARAMS],
            start_f: None,
            pending: Vec::new(),
            cfg,
        }
    }

    fn wave_order(&self) -> [Wave; 3] {
        self.cfg.start.as_ref().map_or(Wave::ALL, |s| s.wave_order)
    }

    fn full(wave: Wave, cont: &[f64]) -> Vec<f64> {
        let mut v = Vec::with_capacity(N_PARAMS);
        v.push(wave.unit());
        v.extend_from_slice(cont);
        v
    }

    fn begin_run(&mut self) {
        self.wave = self.wave_order()[self.restarts % 3];
        self.run_evals = 0;
        let cycle = self.restarts / 3;
        let big = self.lambda0 << cycle.min(20);
        let heuristic_mean = self
            .cfg
            .start
            .as_ref()
            .filter(|_| self.restarts < 3)
            .map(|st| st.cont.clone());
        let (mean, sigma, lambda, regime) = if let Some(mean) = heuristic_mean {
            (mean, self.cfg.sigma_start, self.lambda0, Regime::Large)
        } else {
            let mean: Vec<f64> = (0..N_CONT).map(|_| self.rng.uniform()).collect();
            let small = self.cfg.restarts == Restarts::Bipop
                && self.restarts >= 3
                && self.evals_small < self.evals_large;
            if small {
                // Hansen 2009: lambda_small = lambda0 * (0.5 * lambda_large / lambda0)^(U^2), sigma0 * 10^(-2 U')
                let (u, u2) = (self.rng.uniform(), self.rng.uniform());
                let ratio = 0.5 * big as f64 / self.lambda0 as f64;
                let l = (self.lambda0 as f64 * libm::pow(ratio, u * u)) as usize;
                (
                    mean,
                    self.cfg.sigma_random * libm::pow(10.0, -2.0 * u2),
                    l.max(4),
                    Regime::Small,
                )
            } else {
                (mean, self.cfg.sigma_random, big, Regime::Large)
            }
        };
        self.regime = regime;
        let seed = self.rng.next_u64();
        self.es = Some(Cmaes::new(
            mean,
            sigma,
            lambda,
            vec![0.0; N_CONT],
            vec![1.0; N_CONT],
            seed,
        ));
    }

    fn end_run(&mut self) {
        match self.regime {
            Regime::Large => self.evals_large += self.run_evals,
            Regime::Small => self.evals_small += self.run_evals,
        }
        self.restarts += 1;
        self.es = None;
    }

    /// Candidates (full 10-parameter normalised vectors) to evaluate next. Call [`Search::tell`] with their fitness.
    pub fn ask(&mut self) -> Vec<Vec<f64>> {
        if self.phase == Phase::StartPoint {
            let st = self
                .cfg
                .start
                .as_ref()
                .expect("start phase implies a start");
            self.pending = vec![Self::full(st.wave_order[0], &st.cont)];
            return self.pending.clone();
        }
        if self.es.is_none() {
            self.begin_run();
        }
        let wave = self.wave;
        let xs = self.es.as_mut().expect("run begun").ask();
        self.pending = xs.iter().map(|c| Self::full(wave, c)).collect();
        self.pending.clone()
    }

    /// Fitness (lower is better) of the batch returned by the last [`Search::ask`], in the same order.
    pub fn tell(&mut self, fitness: &[f64]) {
        assert_eq!(
            fitness.len(),
            self.pending.len(),
            "tell() needs one fitness per asked point"
        );
        for (x, &f) in self.pending.iter().zip(fitness) {
            self.evals += 1;
            if f < self.best_f {
                self.best_f = f;
                self.best_x.clone_from(x);
            }
        }
        if self.phase == Phase::StartPoint {
            self.start_f = Some(fitness[0]);
            self.phase = Phase::Generation;
            return;
        }
        self.run_evals += fitness.len();
        let es = self.es.as_mut().expect("run begun");
        es.tell(fitness);
        if es.stop_reason().is_some() && !self.finished() {
            self.end_run();
        }
    }

    /// The best fitness reached the `stop_at` goal.
    #[must_use]
    pub fn reached_goal(&self) -> bool {
        self.cfg.stop_at.is_some_and(|t| self.best_f <= t)
    }

    #[must_use]
    pub fn finished(&self) -> bool {
        self.reached_goal() || self.evals >= self.cfg.budget
    }

    #[must_use]
    pub fn best_fitness(&self) -> f64 {
        self.best_f
    }

    #[must_use]
    pub fn best_x(&self) -> &[f64] {
        &self.best_x
    }

    #[must_use]
    pub fn evals(&self) -> usize {
        self.evals
    }

    #[must_use]
    pub fn restarts(&self) -> usize {
        self.restarts
    }

    /// Distance of the heuristic start point itself (NaN without a heuristic start).
    #[must_use]
    pub fn start_fitness(&self) -> f64 {
        self.start_f.unwrap_or(f64::NAN)
    }
}

/// Run several searches side by side (e.g. heuristic start and random start), evaluating their generations in one
/// combined batch so the caller's work queue stays full. Stops when any search reaches its goal, when every search has
/// used its budget, or when `after_batch` returns false (the caller's wall-clock cap lives there).
pub fn drive<E: BatchEval + ?Sized>(
    searches: &mut [Search],
    ev: &mut E,
    mut after_batch: impl FnMut(&[Search]) -> bool,
) {
    loop {
        if searches.iter().any(Search::reached_goal) {
            break;
        }
        let active: Vec<usize> = (0..searches.len())
            .filter(|&i| !searches[i].finished())
            .collect();
        if active.is_empty() {
            break;
        }
        let asked: Vec<Vec<Vec<f64>>> = active.iter().map(|&i| searches[i].ask()).collect();
        let flat: Vec<Vec<f64>> = asked.iter().flatten().cloned().collect();
        let fit = ev.eval(&flat);
        let mut off = 0;
        for (k, &i) in active.iter().enumerate() {
            let n = asked[k].len();
            searches[i].tell(&fit[off..off + n]);
            off += n;
        }
        if !after_batch(searches) {
            break;
        }
    }
}
