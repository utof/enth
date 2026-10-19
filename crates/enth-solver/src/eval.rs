//! Evaluation: one candidate patch, rendered and described. The solver decides *what* to measure; the caller's
//! [`Evaluator`] decides *how* (one thread, rayon, a worker pool), because core crates spawn no threads.
//!
//! The contract that makes results identical for any worker count: an evaluator returns one result per job, **in
//! job order**, and each result is a pure function of its job. The simplest way to honour it is to call
//! [`measure`] for every job, which is what [`Serial`] does and what a rayon evaluator should do:
//!
//! ```ignore
//! // In the CLI (L8), which may use rayon:
//! struct Rayon;
//! impl enth_solver::Evaluator for Rayon {
//!     fn evaluate(&mut self, jobs: &[enth_solver::Job<'_>]) -> Vec<Result<Descriptors, enth_solver::EvalError>> {
//!         use rayon::prelude::*;
//!         jobs.par_iter().map_init(|| None, |d, job| enth_solver::measure(job, d)).collect()
//!     }
//! }
//! ```
//!
//! `collect` on an indexed parallel iterator keeps the input order, and the scalar FFT path makes a
//! [`Describer`]'s output independent of which instance (or thread) computed it.

use core::fmt;

use enth_analysis::descriptors::{AnalysisError, Want};
use enth_analysis::{Describer, Descriptors, NoteTiming};
use enth_patch::Patch;
use enth_render::{Performance, RenderError, RenderSpec, render};

/// One evaluation: render `patch` playing `perf` with `spec`, then describe the mid signal, computing at least
/// the descriptor groups in `want`.
#[derive(Clone, Debug)]
pub struct Job<'a> {
    /// The candidate patch.
    pub patch: Patch,
    /// What is played (the goal's performance, or a short clip of it for spectral-only lists).
    pub perf: &'a Performance,
    /// Sample rate and render seed.
    pub spec: RenderSpec,
    /// The optional descriptor groups to compute ([`Want::for_features`] of the expectation list).
    pub want: Want,
}

/// Why one evaluation failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EvalError {
    /// The render failed (the patch does not compile, or the performance or spec is bad).
    Render(RenderError),
    /// The analysis refused the signal.
    Analysis(AnalysisError),
    /// The sample rate is outside what the analysis supports (8–192 kHz, whole Hz).
    SampleRate(u32),
}

impl EvalError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            EvalError::Render(e) => e.code(),
            EvalError::Analysis(_) => "analysis_failed",
            EvalError::SampleRate(_) => "bad_sample_rate",
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Render(e) => write!(f, "render failed: {e}"),
            EvalError::Analysis(e) => write!(f, "analysis failed: {e}"),
            EvalError::SampleRate(sr) => write!(
                f,
                "sample rate {sr} Hz cannot be analysed (8000 to 191999 Hz)"
            ),
        }
    }
}

impl std::error::Error for EvalError {}

/// Renders and describes batches of candidate patches. Implemented by the caller, which owns parallelism.
///
/// Contract: `evaluate(jobs)` returns exactly `jobs.len()` results, `out[i]` for `jobs[i]`, and each result
/// depends only on its job (not on the batch, the order of work or the worker that ran it). Call [`measure`] per
/// job to get that for free.
pub trait Evaluator {
    /// Evaluate every job; results in job order.
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>>;
}

/// The single-threaded evaluator: [`measure`] for each job in turn, reusing one [`Describer`].
#[derive(Debug, Default)]
pub struct Serial {
    describer: Option<Describer>,
}

impl Serial {
    /// A serial evaluator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Evaluator for Serial {
    fn evaluate(&mut self, jobs: &[Job<'_>]) -> Vec<Result<Descriptors, EvalError>> {
        jobs.iter()
            .map(|job| measure(job, &mut self.describer))
            .collect()
    }
}

/// The note timing the descriptors use for a performance: the first note (earliest on, ties in performance
/// order), with its note-off even when that falls at or after the end of the clip (the release is then unknown).
/// No notes: from 0 with an unknown note-off.
#[must_use]
pub fn note_timing(perf: &Performance) -> NoteTiming {
    let first = perf
        .notes
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| a.on_s.total_cmp(&b.on_s).then(i.cmp(j)))
        .map(|(_, n)| n);
    match first {
        Some(n) => NoteTiming {
            on_s: n.on_s,
            off_s: Some(n.off_s),
        },
        None => NoteTiming::unknown_off(),
    }
}

/// Render and describe one job: `render`, then [`Describer::describe_some`] on the mid signal `(L + R) / 2` with
/// [`note_timing`]. `describer` is created on first use (and re-created for another sample rate), so a worker
/// keeps one across jobs. On the scalar FFT path the result does not depend on which describer computed it.
///
/// # Errors
/// A render that fails, a signal the analysis refuses, or a sample rate the analysis does not support.
pub fn measure(job: &Job<'_>, describer: &mut Option<Describer>) -> Result<Descriptors, EvalError> {
    let sr = job.spec.sample_rate;
    if !(8_000..192_000).contains(&sr) {
        return Err(EvalError::SampleRate(sr));
    }
    let out = render(&job.patch, job.perf, &job.spec).map_err(EvalError::Render)?;
    let d = match describer {
        Some(d) if d.sample_rate() == f64::from(sr) => d,
        slot => slot.insert(Describer::new(f64::from(sr))),
    };
    d.describe_some(&out.mid(), note_timing(job.perf), job.want)
        .map_err(EvalError::Analysis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_render::Note;

    #[test]
    fn timing_takes_the_first_note() {
        let perf = Performance {
            notes: vec![
                Note::new(60, 0.8, 0.5, 1.0),
                Note::new(62, 0.8, 0.25, 2.0),
                Note::new(64, 0.8, 0.25, 3.0),
            ],
            automation: Vec::new(),
            duration_s: 3.0,
        };
        let t = note_timing(&perf);
        assert_eq!((t.on_s, t.off_s), (0.25, Some(2.0)));
        assert_eq!(
            note_timing(&Performance::default()),
            NoteTiming::unknown_off()
        );
    }

    #[test]
    fn bad_sample_rate_is_an_error_not_a_panic() {
        let perf = Performance::note(69, 0.8, 0.1, 0.2);
        let job = Job {
            patch: Patch::init(),
            perf: &perf,
            spec: RenderSpec::new(384_000, 1),
            want: Want::CORE,
        };
        let r = measure(&job, &mut None);
        assert_eq!(r.unwrap_err().code(), "bad_sample_rate");
    }
}
