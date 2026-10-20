//! What a search minimises: a distance from a candidate patch to the target.
//!
//! The seam is [`Objective`]: a batch of candidate patches in, one distance each out, **in input order, each a pure
//! function of its candidate**. The solver is a core crate and spawns no threads (`AGENTS.md`), so parallelism is the
//! implementer's: [`MetricWorker`] is the serial default, and a caller with a pool builds one worker per thread from
//! one shared [`MetricTarget`] (see below). Lane SS's sound scorer plugs in the same way, as a closure or a type
//! implementing [`Objective`].
//!
//! # The default: metric v1
//!
//! [`MetricTarget`] holds the target's metric v1 features (`logmel-l1/1`, ADR-0007: both clips scaled to −23 LUFS,
//! mean absolute log-mel difference at three resolutions) and what is played; a [`MetricWorker`] renders a candidate
//! with that performance, extracts its features on the scalar FFT path and returns `metric_v1`. A silent candidate
//! scores [`SILENT`](enth_analysis::distance::SILENT); a candidate that does not render scores [`FAILED`].
//!
//! ```ignore
//! // In the CLI (L8), which may use rayon: one worker per thread, results in input order.
//! struct Pool<'a>(&'a enth_solver::search::MetricTarget);
//! impl enth_solver::search::Objective for Pool<'_> {
//!     fn distances(&mut self, candidates: &[enth_patch::Patch]) -> Vec<f64> {
//!         use rayon::prelude::*;
//!         candidates
//!             .par_iter()
//!             .map_init(|| self.0.worker(), |w, p| w.distance(p))
//!             .collect()
//!     }
//! }
//! ```
//!
//! Because `collect` on an indexed parallel iterator keeps the input order and a distance does not depend on which
//! worker computed it (scalar FFT path, no shared state), a search gives the same result for any number of workers,
//! provided its population `lambda` does not depend on that number
//! ([`SearchCfg::lambda`](super::SearchCfg::lambda)).

use enth_analysis::distance::metric_v1;
use enth_analysis::features::{FeatureCfg, FeatureExtractor, Features};
use enth_analysis::{Describer, Descriptors};
use enth_analysis::descriptors::Want;
use enth_patch::Patch;
use enth_render::{Performance, RenderSpec, render};

use crate::eval::{EvalError, note_timing};

/// The distance of a candidate that could not be rendered (above every real distance, and above
/// [`SILENT`](enth_analysis::distance::SILENT)).
pub const FAILED: f64 = 1.0e4;

/// Distances of candidate patches to a target; lower is better, 0 is identical.
///
/// Contract: `distances(c)` returns exactly `c.len()` values, `out[i]` for `c[i]`, and `out[i]` depends only on
/// `c[i]` (not on the batch, the order of work or the worker that ran it). A non-finite value counts as failed.
pub trait Objective {
    /// The distance of every candidate, in input order.
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64>;
}

/// A closure that scores one patch is an objective (evaluated one after the other, in order).
impl<F: FnMut(&Patch) -> f64> Objective for F {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        candidates.iter().map(&mut *self).collect()
    }
}

/// The target of a metric v1 search: its features, its descriptors (for the heuristic start) and what is played.
/// Immutable and shareable between workers.
#[derive(Clone, Debug)]
pub struct MetricTarget {
    features: Features,
    descriptors: Option<Descriptors>,
    perf: Performance,
    spec: RenderSpec,
    cfg: FeatureCfg,
}

impl MetricTarget {
    /// The target of a clip: `mid` is the clip as the scorer hears it (`(L + R) / 2` of a render of `perf` at
    /// `spec`). Its descriptors are measured with the first note's timing; `None` when the analysis refuses the clip
    /// (too short, not finite, silent below the gate).
    #[must_use]
    pub fn from_audio(mid: &[f32], perf: &Performance, spec: &RenderSpec) -> MetricTarget {
        let cfg = FeatureCfg {
            sample_rate: f64::from(spec.sample_rate),
            ..FeatureCfg::default()
        };
        let features = FeatureExtractor::new(cfg).extract(mid);
        let descriptors = Describer::new(f64::from(spec.sample_rate))
            .describe_some(mid, note_timing(perf), Want::ALL)
            .ok()
            .filter(|d| !d.silent);
        MetricTarget {
            features,
            descriptors,
            perf: perf.clone(),
            spec: *spec,
            cfg,
        }
    }

    /// The target of a patch: render it with `perf` and `spec`, then [`from_audio`](Self::from_audio).
    ///
    /// # Errors
    /// [`EvalError::Render`] if the patch does not render, [`EvalError::SampleRate`] for a rate the analysis cannot
    /// handle.
    pub fn from_patch(
        patch: &Patch,
        perf: &Performance,
        spec: &RenderSpec,
    ) -> Result<MetricTarget, EvalError> {
        if !(8_000..192_000).contains(&spec.sample_rate) {
            return Err(EvalError::SampleRate(spec.sample_rate));
        }
        let out = render(patch, perf, spec).map_err(EvalError::Render)?;
        Ok(MetricTarget::from_audio(&out.mid(), perf, spec))
    }

    /// The target's descriptors, when they could be measured.
    #[must_use]
    pub fn descriptors(&self) -> Option<&Descriptors> {
        self.descriptors.as_ref()
    }

    /// What is played.
    #[must_use]
    pub fn performance(&self) -> &Performance {
        &self.perf
    }

    /// How it is rendered.
    #[must_use]
    pub fn render_spec(&self) -> &RenderSpec {
        &self.spec
    }

    /// The target's metric v1 features.
    #[must_use]
    pub fn features(&self) -> &Features {
        &self.features
    }

    /// A worker: its own feature extractor (FFT plans, filter banks, scratch buffers), so one per thread.
    #[must_use]
    pub fn worker(&self) -> MetricWorker<'_> {
        MetricWorker {
            target: self,
            fx: FeatureExtractor::new(self.cfg),
        }
    }
}

/// Scores candidates against a [`MetricTarget`] with metric v1, one after the other. It is the serial
/// [`Objective`]; give each thread its own.
pub struct MetricWorker<'a> {
    target: &'a MetricTarget,
    fx: FeatureExtractor,
}

impl MetricWorker<'_> {
    /// Metric v1 between the target and `patch` rendered with the target's performance: [`FAILED`] if the patch
    /// does not render.
    pub fn distance(&mut self, patch: &Patch) -> f64 {
        let Ok(out) = render(patch, &self.target.perf, &self.target.spec) else {
            return FAILED;
        };
        let features = self.fx.extract(&out.mid());
        let d = metric_v1(&self.target.features, &features);
        if d.is_finite() { d } else { FAILED }
    }
}

impl core::fmt::Debug for MetricWorker<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MetricWorker").finish_non_exhaustive()
    }
}

impl Objective for MetricWorker<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        candidates.iter().map(|p| self.distance(p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn workers_move_between_threads_and_the_target_is_shared() {
        assert_send::<MetricWorker<'static>>();
        assert_sync::<MetricTarget>();
    }

    #[test]
    fn metric_zero_for_the_target_itself_and_positive_for_another_patch() {
        let perf = Performance::note(60, 0.8, 0.3, 0.6);
        let spec = RenderSpec::default();
        let target_patch = Patch::init();
        let target = MetricTarget::from_patch(&target_patch, &perf, &spec).unwrap();
        assert!(target.descriptors().is_some());
        let mut w = target.worker();
        assert_eq!(w.distance(&target_patch), 0.0);
        let mut other = Patch::init();
        enth_patch::apply(
            &mut other,
            enth_patch::Op::SetParam {
                addr: "osc1.wave".parse().unwrap(),
                value: 0.0,
            },
        )
        .unwrap();
        let ds = w.distances(&[target_patch, other]);
        assert_eq!(ds[0], 0.0);
        assert!(ds[1] > 0.1, "a sine is far from a saw: {}", ds[1]);
    }

    #[test]
    fn closures_are_objectives_and_keep_input_order() {
        let mut calls = 0;
        let mut f = |p: &Patch| {
            calls += 1;
            f64::from(p.value(&"osc1.semi".parse().unwrap()).unwrap()) + f64::from(calls) * 100.0
        };
        let ps = vec![Patch::init(), Patch::init(), Patch::init()];
        assert_eq!(f.distances(&ps), vec![100.0, 200.0, 300.0]);
    }
}
