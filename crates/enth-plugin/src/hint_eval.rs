//! **Owner: lane HINT** (FU1). The hint job's evaluator: `enth_solver::measure` for every job of a batch on a few
//! scoped threads, results written back by job index (docs/plugin.md, "The Recreate round"; docs/dsp/hints.md,
//! "Determinism and parallelism").
//!
//! Core crates spawn no threads, so the coach's renders (lane SN's `CoachTarget::for_round` and `CoachTarget::plan`)
//! go through the caller's `enth_solver::Evaluator`; this is the plugin's. It needs no new crate (the plugin has no
//! rayon): `std::thread::scope`, one shared counter as the work queue, and each worker's results kept with their job
//! index. **A hint is identical for any worker count**: each result is a pure function of its job (the scalar FFT
//! path, so not of which [`Describer`] computed it), and the batch is reassembled in job order, whatever order the
//! workers finished in. `the_worker_count_never_changes_the_hint` checks it against the serial evaluator.
//!
//! # How many workers
//!
//! [`hint_workers`]: half the logical CPUs, at least 1 and at most [`MAX_HINT_WORKERS`] (4). The reasons:
//!
//! - **The host's audio thread must never starve.** The workers run at normal priority. A host that gives its audio
//!   threads real-time priority is safe either way, but REAPER on Linux without rtkit, and many hosts' extra audio
//!   worker threads, run at normal priority too, and during a hint the player is often listening: Mine ▶ renders
//!   live in `process()`. Half the logical CPUs is about the physical cores, and leaves the other SMT siblings (and
//!   at least one whole core on any machine with two or more) to the host's audio, GUI and main threads.
//! - **More does not pay.** The coach's parallel batches are the target's sensitivity (1 + the randomized continuous
//!   parameters, about 6–20 renders) and the solver's probe batch (one render per free parameter, 6–11 for a
//!   two-oscillator patch); its verification steps are serial (each depends on the last). Past four workers the
//!   serial steps dominate (docs/plugin.md's latency table).
//! - **RAM.** Each worker holds one render (a few MB for a 2–5 s stereo phrase at 48 kHz) and its describer.
//!
//! The calling thread (the game worker, `enth-game`) is one of the workers, so a batch uses `n − 1` new threads,
//! named `enth-hint-<i>`, for the batch only; a batch of one job, or one worker, runs inline with no thread at all.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

use enth_analysis::{Describer, Descriptors};
use enth_solver::{EvalError, Evaluator, Job, measure};

/// The most threads a hint uses, the calling thread included (see the [module docs](self)).
pub const MAX_HINT_WORKERS: usize = 4;

/// How many workers a hint uses on a machine with `cpus` logical CPUs: half of them, at least 1 and at most
/// [`MAX_HINT_WORKERS`] (see the [module docs](self)).
pub fn hint_workers(cpus: usize) -> usize {
    (cpus / 2).clamp(1, MAX_HINT_WORKERS)
}

/// The plugin's `enth_solver::Evaluator` for hints: `measure` on up to `n` threads, results in job order. Each worker
/// keeps its [`Describer`] across batches.
#[derive(Debug)]
pub struct Workers {
    /// One describer slot per worker; slot 0 is the calling thread's.
    describers: Vec<Option<Describer>>,
}

impl Workers {
    /// `n` workers (at least one; [`MAX_HINT_WORKERS`] is the shell's cap, not this type's: tests and the latency
    /// measurement ask for any count).
    pub fn new(n: usize) -> Workers {
        Workers {
            describers: (0..n.max(1)).map(|_| None).collect(),
        }
    }

    /// [`hint_workers`] for this machine's logical CPUs (`std::thread::available_parallelism`, 1 if unknown).
    pub fn for_this_machine() -> Workers {
        let cpus = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        Workers::new(hint_workers(cpus))
    }

    /// How many workers a batch may use.
    pub fn count(&self) -> usize {
        self.describers.len()
    }
}

impl Default for Workers {
    fn default() -> Self {
        Workers::for_this_machine()
    }
}

/// Take jobs off the shared counter until none is left, measuring each with `describer`.
fn drain<'j>(
    jobs: &'j [Job<'j>],
    next: &AtomicUsize,
    describer: &mut Option<Describer>,
) -> Vec<(usize, Result<Descriptors, EvalError>)> {
    let mut done = Vec::new();
    loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        let Some(job) = jobs.get(i) else {
            return done;
        };
        done.push((i, measure(job, describer)));
    }
}

impl Evaluator for Workers {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        let n = self.describers.len().min(jobs.len());
        if n <= 1 {
            let d = &mut self.describers[0];
            return jobs.iter().map(|job| measure(job, d)).collect();
        }
        let next = AtomicUsize::new(0);
        let mut slots: Vec<Option<Result<Descriptors, EvalError>>> =
            (0..jobs.len()).map(|_| None).collect();
        let (own, others) = self.describers[..n].split_at_mut(1);
        std::thread::scope(|s| {
            let next = &next;
            // A thread that cannot be spawned leaves its share to the others (and the inline pass below).
            let spawned: Vec<_> = others
                .iter_mut()
                .enumerate()
                .filter_map(|(w, d)| {
                    std::thread::Builder::new()
                        .name(format!("enth-hint-{}", w + 1))
                        .spawn_scoped(s, move || drain(jobs, next, d))
                        .ok()
                })
                .collect();
            let mut put = |done: Vec<(usize, Result<Descriptors, EvalError>)>| {
                for (i, r) in done {
                    slots[i] = Some(r);
                }
            };
            put(drain(jobs, next, &mut own[0]));
            for handle in spawned {
                match handle.join() {
                    Ok(done) => put(done),
                    // A panicking render: the game worker's `catch_unwind` turns it into a failed job.
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
        });
        let d = &mut self.describers[0];
        slots
            .into_iter()
            .zip(jobs)
            .map(|(r, job)| r.unwrap_or_else(|| measure(job, d)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::descriptors::Want;
    use enth_patch::{Op, Patch};
    use enth_render::{Performance, RenderSpec};
    use enth_solver::Serial;

    #[test]
    fn the_worker_count_is_half_the_cpus_within_one_and_four() {
        let counts: Vec<usize> = [0, 1, 2, 3, 4, 6, 8, 12, 16, 64]
            .into_iter()
            .map(hint_workers)
            .collect();
        assert_eq!(counts, [1, 1, 1, 1, 2, 3, 4, 4, 4, 4]);
        assert!((1..=MAX_HINT_WORKERS).contains(&Workers::for_this_machine().count()));
        assert_eq!(Workers::new(0).count(), 1, "never zero");
    }

    /// Jobs in a batch come back in job order with the serial evaluator's descriptors, for any worker count, more
    /// workers than jobs and a batch of one included; a job that fails stays at its index.
    #[test]
    fn results_come_back_in_job_order_for_any_worker_count() {
        let perf = Performance::note(60, 1.0, 0.2, 0.3);
        let mut patches = Vec::new();
        for fine in [0.0_f32, 7.0, -12.0, 3.0] {
            let mut p = Patch::init();
            let set = Op::SetParam {
                addr: "osc1.fine".parse().unwrap(),
                value: fine,
            };
            enth_patch::apply(&mut p, &set).unwrap();
            patches.push(p);
        }
        let jobs: Vec<Job<'_>> = patches
            .iter()
            .enumerate()
            .map(|(i, p)| Job {
                patch: p.clone(),
                perf: &perf,
                // The second job asks for a sample rate `measure` refuses: its error stays at its index.
                spec: RenderSpec::new(if i == 1 { 1_000 } else { 48_000 }, 7),
                want: Want::CORE,
            })
            .collect();
        let serial = Serial::new().evaluate(&jobs);
        assert!(serial[1].is_err() && serial[0].is_ok());
        for n in [1, 2, 3, 8] {
            let mut w = Workers::new(n);
            assert_eq!(w.evaluate(&jobs), serial, "{n} workers");
            assert_eq!(w.evaluate(&jobs[..1]), serial[..1], "{n} workers, one job");
            assert_eq!(w.evaluate(&[]), [], "{n} workers, no job");
        }
    }
}
