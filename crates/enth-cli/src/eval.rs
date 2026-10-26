//! The evaluator: render a patch with a performance and describe the result, many at once on rayon, results in
//! input order (so they are identical for any worker count). `describe`, `check` and `step` use it for the judged
//! render and the nuisance re-renders; the hint solver (through [`crate::fix`]) uses [`Parallel`] for its probes.
//! Both go through [`enth_solver::measure`] (render, then describe the mid signal with the first note's timing on
//! the scalar FFT path), so a hint's `before` is the CLI's own reading. Core crates spawn no threads: this is where
//! the parallelism comes from (AGENTS.md).

use enth_analysis::descriptors::Want;
use enth_analysis::{Descriptors, NoteTiming};
use enth_patch::Patch;
use enth_render::{DEFAULT_SAMPLE_RATE, Performance, RenderOutput, RenderSpec, render};
use enth_solver::{EvalError, Job, measure, note_timing};
use rayon::prelude::*;

use crate::error::{self, CliError};
use crate::perf::Perf;

/// Offsets that derive the two nuisance render seeds from the judged one. Oscillators in `free` phase mode (the
/// default) take their start phases from the render seed, so a re-render with another seed is the same sound with
/// other phases: how far a reading moves across them is its measurement noise.
const NUISANCE_OFFSETS: [u64; 2] = [0x9E37_79B9_7F4A_7C15, 0xC2B2_AE3D_27D4_EB4F];

/// Renders and describes patches with one performance.
#[derive(Clone, Debug)]
pub struct Evaluator {
    perf: Performance,
    seed: u64,
    want: Want,
}

impl Evaluator {
    /// An evaluator for `perf` at the judged render `seed`, computing the descriptor groups in `want` (pitch, the
    /// envelope and beating are always measured).
    pub fn new(perf: &Perf, seed: u64, want: Want) -> Self {
        Evaluator {
            perf: perf.to_render(),
            seed,
            want,
        }
    }

    /// This evaluator with the tempo `bpm` (already checked by [`crate::perf::parse_bpm`]) for tempo-synced modules.
    #[must_use]
    pub fn with_tempo(mut self, bpm: f64) -> Self {
        self.perf.tempo_bpm = bpm;
        self
    }

    /// The judged render seed.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The seeds of the two nuisance re-renders.
    pub fn nuisance_seeds(&self) -> [u64; 2] {
        NUISANCE_OFFSETS.map(|o| self.seed.wrapping_add(o))
    }

    /// The performance, as rendered.
    pub fn performance(&self) -> &Performance {
        &self.perf
    }

    /// The note timing the envelope is read against: the first note and its note-off
    /// ([`enth_solver::note_timing`], the one `measure` uses).
    pub fn timing(&self) -> NoteTiming {
        note_timing(&self.perf)
    }

    /// The sample rate of every render (48 kHz).
    pub fn sample_rate(&self) -> u32 {
        DEFAULT_SAMPLE_RATE
    }

    /// Render one patch at a seed.
    pub fn render(&self, patch: &Patch, seed: u64) -> Result<RenderOutput, CliError> {
        render(
            patch,
            &self.perf,
            &RenderSpec::new(DEFAULT_SAMPLE_RATE, seed),
        )
        .map_err(|e| error::from_render(&e))
    }

    /// The solver's job for one render: `patch` at `seed`, describing at least the groups in `want`.
    fn job(&self, patch: &Patch, seed: u64, want: Want) -> Job<'_> {
        Job {
            patch: patch.clone(),
            perf: &self.perf,
            spec: RenderSpec::new(DEFAULT_SAMPLE_RATE, seed),
            want,
        }
    }

    /// Render and describe each `(patch, seed)` job in parallel; results in input order.
    pub fn describe_jobs(&self, jobs: &[(&Patch, u64)]) -> Vec<Result<Descriptors, CliError>> {
        self.describe_jobs_with(jobs, self.want)
    }

    /// [`Evaluator::describe_jobs`] computing the descriptor groups in `want` instead of the evaluator's own.
    pub fn describe_jobs_with(
        &self,
        jobs: &[(&Patch, u64)],
        want: Want,
    ) -> Vec<Result<Descriptors, CliError>> {
        let jobs: Vec<Job<'_>> = jobs
            .iter()
            .map(|(patch, seed)| self.job(patch, *seed, want))
            .collect();
        measure_all(&jobs)
            .into_iter()
            .map(|r| r.map_err(|e| error::from_eval(&e)))
            .collect()
    }
}

/// The solver's [`enth_solver::Evaluator`]: [`measure`] for every job on rayon, results in job order. A hint is
/// identical for any worker count because each result is a pure function of its job and `collect` keeps order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parallel;

/// [`measure`] for every job on rayon, results in job order.
fn measure_all(jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
    jobs.par_iter()
        .map_init(|| None, |d, job| measure(job, d))
        .collect()
}

impl enth_solver::Evaluator for Parallel {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        measure_all(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_input_order_and_repeat() {
        let perf = Perf::default_perf();
        let ev = Evaluator::new(&perf, enth_render::DEFAULT_SEED, Want::CORE);
        let a = Patch::init();
        let mut b = Patch::init();
        b.voice.values[0] = 0.0; // poly, unchanged: the same patch twice under another name
        let jobs = [(&a, 1), (&b, 2), (&a, 1)];
        let r = ev.describe_jobs(&jobs);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], r[2]);
        let again = ev.describe_jobs(&jobs);
        assert_eq!(r, again);
    }
}
