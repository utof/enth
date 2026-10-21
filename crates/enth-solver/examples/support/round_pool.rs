//! A deterministic worker pool for `closest`, for tests and examples: the stand-in for the rayon-backed evaluator the
//! plugin worker and the CLI build (`enth_solver::closest`'s docs). Threads are the shells' business, so this lives
//! outside the library, and it opts out of the workspace's thread ban with a reason.
//!
//! Work is pulled from an atomic counter (a queue, so a slow candidate never idles the other workers) and written
//! back by index, so the readings are a pure function of the candidates: identical for any worker count. A round is
//! `Send + Sync` and scoring takes `&self`, so the workers share it and keep no state of their own.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test and example support: stands in for the shell's thread pool, which the core crates must not spawn or lock"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use enth_challenge::PreparedRound;
use enth_patch::Patch;
use enth_solver::closest::{ClosestEval, Reading, reading};

pub struct RoundPool {
    workers: usize,
}

impl RoundPool {
    pub fn new(workers: usize) -> Self {
        RoundPool {
            workers: workers.max(1),
        }
    }
}

impl ClosestEval for RoundPool {
    fn readings(&mut self, round: &PreparedRound, candidates: &[Patch]) -> Vec<Option<Reading>> {
        let n = candidates.len();
        let next = AtomicUsize::new(0);
        let mut out: Vec<Option<Reading>> = vec![None; n];
        thread::scope(|s| {
            let handles: Vec<_> = (0..self.workers.min(n.max(1)))
                .map(|_| {
                    let next = &next;
                    s.spawn(move || {
                        let mut got = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= n {
                                break;
                            }
                            got.push((i, reading(round, &candidates[i])));
                        }
                        got
                    })
                })
                .collect();
            for h in handles {
                for (i, r) in h.join().expect("a worker does not panic") {
                    out[i] = r;
                }
            }
        });
        out
    }
}
