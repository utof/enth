//! **Owner: lane OBJ** (R3a, wave 2). The robust distance ([`enth_analysis::robust`], metric id
//! [`METRIC_ROBUST`](enth_analysis::robust::METRIC_ROBUST)) as a create job's [`Objective`]: what a shell passes to
//! [`create::create`](crate::create::create) (or feeds a [`CreateJob`](crate::create::CreateJob)) instead of metric
//! v1's [`MetricTarget`](crate::search::MetricTarget), and records under that id in `enth.create/1`'s settings. TP's
//! API is unchanged: this is one more objective beside metric v1's, built from the same [`Target`] so no shell scores
//! another clip, window, performance or seed than the job renders.
//!
//! ```ignore
//! // In a shell (L8), with rayon: one worker per thread, distances in input order.
//! let it = Intake::new(&interleaved, channels, sample_rate)?;
//! let target = create::Target::from_intake(&it, enth_render::DEFAULT_SEED);
//! let robust = RobustMetricTarget::from_intake(&target, &it.report);
//! struct Pool<'a>(&'a RobustMetricTarget);
//! impl Objective for Pool<'_> {
//!     fn distances(&mut self, c: &[Patch]) -> Vec<f64> {
//!         c.par_iter().map_init(|| self.0.worker(), |w, p| w.distance(p)).collect()
//!     }
//! }
//! let result = create::create(&target, &inputs, &mut Pool(&robust), &cfg, &mut progress)?;
//! // the record's settings: metric = enth_analysis::robust::METRIC_ROBUST
//! ```
//!
//! A worker renders each candidate with the target's performance, longer by
//! [`RobustTarget::extra_samples`] when the upload ends in its floor (the silence after the sound is compared too),
//! its note-on earlier by [`RobustTarget::lead_s`] when a slow attack surfaced late from a confirmed floor (the
//! render starts whole engine segments early and drops them when that reaches before the clip's start), and scores
//! it with [`RobustTarget::distance`]. Like [`MetricWorker`](crate::search::MetricWorker): a candidate
//! that does not render scores [`FAILED`], a distance is a pure function of its candidate (scalar FFT path, no shared
//! state), so a create job gives the same result for any worker count.

use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_analysis::intake::IntakeReport;
use enth_analysis::robust::{Damage, RobustTarget, Settings};
use enth_patch::Patch;
use enth_render::{Performance, RenderSpec, render};

use enth_analysis::intake::trim::SEGMENT_FRAMES;

use crate::create::{SCORE_S, Target};
use crate::search::{FAILED, Objective};

/// A create target prepared for the robust distance: immutable and shareable between workers (one
/// [`RobustWorker`] per thread).
#[derive(Clone, Debug)]
pub struct RobustMetricTarget {
    robust: RobustTarget,
    /// What a candidate plays: the target's performance, longer by the samples compared past the clip, its note-on
    /// earlier by the attack's lead ([`RobustTarget::lead_s`]), all of it later by `skip` samples when that lead
    /// reaches before the clip's start.
    perf: Performance,
    /// Samples dropped from the start of a candidate's render.
    skip: usize,
    spec: RenderSpec,
    cfg: FeatureCfg,
}

impl RobustMetricTarget {
    /// The target with every kind of damage the intake reported ([`Damage::from_report`]): the bandwidth, a confirmed
    /// floor, the hard-clipped spans, and whether the clip ends in the floor (cleared when `target`'s scored window
    /// cuts the clip short). `report` is the report of the intake `target` came from
    /// ([`Target::from_intake`]/[`from_intake_scored`](Target::from_intake_scored)).
    #[must_use]
    pub fn from_intake(target: &Target, report: &IntakeReport) -> RobustMetricTarget {
        RobustMetricTarget::from_intake_with(target, report, &Settings::default())
    }

    /// [`RobustMetricTarget::from_intake`] with other [`Settings`] (experiments; no metric id).
    #[doc(hidden)]
    #[must_use]
    pub fn from_intake_with(
        target: &Target,
        report: &IntakeReport,
        settings: &Settings,
    ) -> RobustMetricTarget {
        let damage =
            Damage::from_report(report).windowed(target.perf.duration_s, report.perf.duration_s);
        RobustMetricTarget::with_damage(target, &damage, settings)
    }

    /// The target with only what [`Target`] carries (the bandwidth and a confirmed floor): no clipped spans, and the
    /// silence after the sound is not compared. For a target that did not come from an intake.
    #[must_use]
    pub fn from_target(target: &Target) -> RobustMetricTarget {
        let damage = Damage {
            bandwidth_hz: target.bandwidth_hz,
            noise_floor_dbfs: target.noise_floor_dbfs,
            ..Damage::default()
        };
        RobustMetricTarget::with_damage(target, &damage, &Settings::default())
    }

    /// The target with an explicit [`Damage`] (clip time of `target.audio`).
    #[must_use]
    pub fn with_damage(
        target: &Target,
        damage: &Damage,
        settings: &Settings,
    ) -> RobustMetricTarget {
        let cfg = target.feature_cfg();
        let mut fx = FeatureExtractor::new(cfg);
        let robust = RobustTarget::with_settings(&mut fx, &target.audio, damage, SCORE_S, settings);
        let sr = f64::from(target.sample_rate);
        let mut perf = target.performance();
        perf.duration_s += robust.extra_samples() as f64 / sr;
        let mut skip = 0;
        let lead = robust.lead_s();
        if lead > 0.0
            && let Some(first) = perf.notes.first().map(|n| n.on_s)
        {
            let on = first - lead;
            if on < 0.0 {
                // start the render whole engine segments earlier and drop them
                skip = (libm::ceil(-on * sr) as usize).div_ceil(SEGMENT_FRAMES) * SEGMENT_FRAMES;
            }
            let shift = skip as f64 / sr;
            for (i, n) in perf.notes.iter_mut().enumerate() {
                if i == 0 {
                    n.on_s = on + shift;
                } else {
                    n.on_s += shift;
                }
                n.off_s += shift;
            }
            perf.duration_s += shift;
        }
        RobustMetricTarget {
            robust,
            perf,
            skip,
            spec: target.render_spec(),
            cfg,
        }
    }

    /// The prepared distance.
    #[must_use]
    pub fn robust(&self) -> &RobustTarget {
        &self.robust
    }

    /// What every candidate plays (the target's performance, maybe longer: see the [module docs](self)).
    #[must_use]
    pub fn performance(&self) -> &Performance {
        &self.perf
    }

    /// A worker: its own feature extractor, so one per thread.
    #[must_use]
    pub fn worker(&self) -> RobustWorker<'_> {
        RobustWorker {
            target: self,
            fx: FeatureExtractor::new(self.cfg),
        }
    }
}

/// Scores candidates against a [`RobustMetricTarget`], one after the other; give each thread its own.
pub struct RobustWorker<'a> {
    target: &'a RobustMetricTarget,
    fx: FeatureExtractor,
}

impl RobustWorker<'_> {
    /// The robust distance from the target to `patch` rendered with the target's performance: [`FAILED`] if the
    /// patch does not render.
    pub fn distance(&mut self, patch: &Patch) -> f64 {
        let Ok(out) = render(patch, &self.target.perf, &self.target.spec) else {
            return FAILED;
        };
        let mid = out.mid();
        let pooled = self
            .target
            .robust
            .candidate(&mut self.fx, mid.get(self.target.skip..).unwrap_or(&[]));
        let d = self.target.robust.distance(&pooled);
        if d.is_finite() { d } else { FAILED }
    }
}

impl core::fmt::Debug for RobustWorker<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RobustWorker").finish_non_exhaustive()
    }
}

impl Objective for RobustWorker<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        candidates.iter().map(|p| self.distance(p)).collect()
    }
}
