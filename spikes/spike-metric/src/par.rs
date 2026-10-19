//! Parallel evaluator over a work queue (spike-only: this is the one module that uses threads, standing in for the
//! rayon-backed evaluator the CLI passes to the solver).
//!
//! Each worker thread owns an [`Analyzer`] and a render buffer. Items are pulled from an atomic counter (a work
//! queue, not static chunks, so a slow item never idles the other workers) and written back by index, so the result
//! vector is a pure function of the inputs: identical for any worker count.

use crate::distance::{Components, Dist, Scale, components, distance};
use crate::features::{AnalysisCfg, Analyzer, Features};
use crate::render::{RenderCfg, render_unit};
use crate::search::BatchEval;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub struct Worker {
    pub analyzer: Analyzer,
    pub buf: Vec<f32>,
}

pub struct ParEval {
    pool: rayon::ThreadPool,
    workers: Vec<Mutex<Worker>>,
    render: RenderCfg,
    target: Features,
    kind: Dist,
    scale: Scale,
}

impl ParEval {
    /// `n_workers` threads (at least 1). The target starts silent; set it with [`ParEval::set_target`].
    #[must_use]
    pub fn new(
        n_workers: usize,
        render: RenderCfg,
        analysis: AnalysisCfg,
        kind: Dist,
        scale: Scale,
    ) -> Self {
        let n = n_workers.max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .expect("thread pool");
        let workers = (0..n)
            .map(|_| {
                Mutex::new(Worker {
                    analyzer: Analyzer::new(analysis),
                    buf: Vec::new(),
                })
            })
            .collect();
        Self {
            pool,
            workers,
            render,
            target: Features {
                silent: true,
                ..Features::default()
            },
            kind,
            scale,
        }
    }

    #[must_use]
    pub fn n_workers(&self) -> usize {
        self.workers.len()
    }

    #[must_use]
    pub fn render_cfg(&self) -> &RenderCfg {
        &self.render
    }

    pub fn set_kind(&mut self, kind: Dist, scale: Scale) {
        self.kind = kind;
        self.scale = scale;
    }

    /// Render the normalised parameter vector `p` and use its features as the target.
    pub fn set_target(&mut self, p: &[f64]) {
        let mut w = self.workers[0].lock().expect("worker lock");
        let Worker { analyzer, buf } = &mut *w;
        render_unit(&self.render, p, buf);
        self.target = analyzer.features(buf);
    }

    #[must_use]
    pub fn target(&self) -> &Features {
        &self.target
    }

    /// Run `f(worker, index)` for every index in `0..n` over the work queue; results come back in index order.
    pub fn map<T, F>(&self, n: usize, f: F) -> Vec<T>
    where
        T: Send + Sync,
        F: Fn(&mut Worker, usize) -> T + Sync,
    {
        let next = AtomicUsize::new(0);
        let slots: Vec<OnceLock<T>> = (0..n).map(|_| OnceLock::new()).collect();
        self.pool.broadcast(|ctx| {
            let mut w = self.workers[ctx.index()].lock().expect("worker lock");
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                assert!(
                    slots[i].set(f(&mut w, i)).is_ok(),
                    "each index is claimed once"
                );
            }
        });
        slots
            .into_iter()
            .map(|s| s.into_inner().expect("every slot filled"))
            .collect()
    }

    /// Per-component distances from the target to each candidate.
    pub fn components(&self, xs: &[Vec<f64>]) -> Vec<Components> {
        self.map(xs.len(), |w, i| {
            render_unit(&self.render, &xs[i], &mut w.buf);
            let f = w.analyzer.features(&w.buf);
            components(&self.target, &f)
        })
    }

    /// Per-component distances within each `(a, b)` pair (used to design the listening test).
    pub fn pair_components(&self, pairs: &[(Vec<f64>, Vec<f64>)]) -> Vec<Components> {
        self.map(pairs.len(), |w, i| {
            render_unit(&self.render, &pairs[i].0, &mut w.buf);
            let fa = w.analyzer.features(&w.buf);
            render_unit(&self.render, &pairs[i].1, &mut w.buf);
            let fb = w.analyzer.features(&w.buf);
            components(&fa, &fb)
        })
    }
}

impl BatchEval for ParEval {
    fn eval(&mut self, xs: &[Vec<f64>]) -> Vec<f64> {
        let (target, kind, scale) = (&self.target, self.kind, &self.scale);
        let render = &self.render;
        self.map(xs.len(), |w, i| {
            render_unit(render, &xs[i], &mut w.buf);
            let f = w.analyzer.features(&w.buf);
            distance(kind, target, &f, scale)
        })
    }
}
