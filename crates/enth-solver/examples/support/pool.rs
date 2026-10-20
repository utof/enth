//! A deterministic worker pool for tests and examples: the stand-in for the rayon-backed objective the CLI builds
//! (`enth_solver::search::objective`). Threads are the shells' business, so this lives outside the library, and it
//! opts out of the workspace's thread ban with a reason.
//!
//! Work is pulled from an atomic counter (a queue, so a slow candidate never idles the other workers) and written
//! back by index, so the result vector is a pure function of the candidates: identical for any worker count.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test and example support: stands in for the shell's thread pool, which the core crates must not spawn or lock"
)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use enth_patch::Patch;
use enth_solver::search::{MetricTarget, MetricWorker, Objective};

pub struct Pool<'a> {
    workers: Vec<Mutex<MetricWorker<'a>>>,
}

impl<'a> Pool<'a> {
    pub fn new(target: &'a MetricTarget, workers: usize) -> Self {
        Pool {
            workers: (0..workers.max(1))
                .map(|_| Mutex::new(target.worker()))
                .collect(),
        }
    }
}

impl Objective for Pool<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        let n = candidates.len();
        let next = AtomicUsize::new(0);
        let mut out = vec![0.0; n];
        thread::scope(|s| {
            let handles: Vec<_> = self
                .workers
                .iter()
                .map(|w| {
                    let next = &next;
                    s.spawn(move || {
                        let mut w = w
                            .lock()
                            .expect("a worker never panics while holding its lock");
                        let mut got = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= n {
                                break;
                            }
                            got.push((i, w.distance(&candidates[i])));
                        }
                        got
                    })
                })
                .collect();
            for h in handles {
                for (i, d) in h.join().expect("a worker does not panic") {
                    out[i] = d;
                }
            }
        });
        out
    }
}
