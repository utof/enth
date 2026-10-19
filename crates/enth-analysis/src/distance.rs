//! Distances between two clips' [`Features`]. **Metric v1** (provisional, `docs/adr/0007-metric-v1.md`): both
//! clips scaled to −23 LUFS, then the mean absolute difference of natural-log mel spectra at FFT 2048, 1024 and 512
//! (hop n/2, 64 bands from 30 Hz to 16 kHz), averaged over the three resolutions. Crest envelope, centroid track,
//! MFCC and f0 in cents are reported beside it as [`Components`], never added into it.
//!
//! Ported from `spikes/spike-metric/src/distance.rs` (in-repo, MIT; distance "A" there). Pure functions of two
//! [`Features`] values; extract them with [`FeatureExtractor`] on the scalar FFT path when the distance decides
//! pass/fail.

use crate::features::{FeatureExtractor, Features};
use crate::units::cents;
use serde::{Deserialize, Serialize};

/// Stable id of metric v1. A change to its definition gets a new id.
pub const METRIC_V1: &str = "logmel-l1/1";

/// Distance returned when either side is silent.
pub const SILENT: f64 = 1e3;

/// Mean absolute log-mel difference, averaged over the resolutions; frames beyond the shorter clip are ignored.
#[must_use]
pub fn logmel_l1(a: &Features, b: &Features) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.logmel.iter().zip(&b.logmel) {
        let n = x.len().min(y.len()).max(1);
        s += x
            .iter()
            .zip(y)
            .map(|(p, q)| f64::from((p - q).abs()))
            .sum::<f64>()
            / n as f64;
    }
    s / a.logmel.len().max(1) as f64
}

/// Metric v1: [`logmel_l1`], or [`SILENT`] when either clip is silent. 0 for identical audio.
#[must_use]
pub fn metric_v1(a: &Features, b: &Features) -> f64 {
    if a.silent || b.silent {
        SILENT
    } else {
        logmel_l1(a, b)
    }
}

/// Crest-envelope L1 (dB per 10 ms frame).
#[must_use]
pub fn envelope_l1(a: &Features, b: &Features) -> f64 {
    let n = a.crest_db.len().min(b.crest_db.len()).max(1);
    a.crest_db
        .iter()
        .zip(&b.crest_db)
        .map(|(p, q)| f64::from((p - q).abs()))
        .sum::<f64>()
        / n as f64
}

/// Centroid-track L1 in octaves, weighted by the mean of the two frames' energy shares.
#[must_use]
pub fn centroid_l1(a: &Features, b: &Features) -> f64 {
    let n = a.centroid.len().min(b.centroid.len());
    (0..n)
        .map(|i| {
            f64::from(
                f32::midpoint(a.centroid_w[i], b.centroid_w[i])
                    * (a.centroid[i] - b.centroid[i]).abs(),
            )
        })
        .sum()
}

/// MFCC mean squared error.
#[must_use]
pub fn mfcc_mse(a: &Features, b: &Features) -> f64 {
    let n = a.mfcc.len().min(b.mfcc.len()).max(1);
    a.mfcc
        .iter()
        .zip(&b.mfcc)
        .map(|(p, q)| f64::from(p - q) * f64::from(p - q))
        .sum::<f64>()
        / n as f64
}

/// Median absolute f0 difference in cents over frames voiced in both; `None` without f0 tracks or voiced frames.
#[must_use]
pub fn f0_cents(a: &Features, b: &Features) -> Option<f64> {
    let mut v: Vec<f64> = a
        .f0_hz
        .iter()
        .zip(&b.f0_hz)
        .filter(|(p, q)| **p > 0.0 && **q > 0.0)
        .map(|(p, q)| cents(f64::from(*p), f64::from(*q)).abs())
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// Every component, for coaching text ("brighter than the target", "attack too fast").
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Components {
    /// Metric v1.
    pub logmel: f64,
    pub envelope: f64,
    pub centroid: f64,
    pub mfcc: f64,
    pub f0_cents: Option<f64>,
}

#[must_use]
pub fn components(a: &Features, b: &Features) -> Components {
    if a.silent || b.silent {
        return Components {
            logmel: SILENT,
            envelope: SILENT,
            centroid: SILENT,
            mfcc: SILENT,
            f0_cents: None,
        };
    }
    Components {
        logmel: logmel_l1(a, b),
        envelope: envelope_l1(a, b),
        centroid: centroid_l1(a, b),
        mfcc: mfcc_mse(a, b),
        f0_cents: f0_cents(a, b),
    }
}

/// Convenience for callers with audio only: extract both and return metric v1.
pub fn audio_metric_v1(fx: &mut FeatureExtractor, a: &[f32], b: &[f32]) -> f64 {
    let (fa, fb) = (fx.extract(a), fx.extract(b));
    metric_v1(&fa, &fb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::FeatureCfg;
    use crate::test_signals::{SR, ToyPatch, ToyPerf, Wave, render_toy};

    fn toy(p: &ToyPatch) -> Vec<f32> {
        render_toy(
            SR,
            p,
            &ToyPerf {
                note_hz: 220.0,
                dur_s: 0.6,
                hold_s: 0.45,
            },
            (0.0, 0.37),
            0.0,
        )
    }

    #[test]
    fn identical_audio_scores_zero_and_loudness_is_normalised() {
        let mut fx = FeatureExtractor::new(FeatureCfg {
            with_f0: true,
            ..FeatureCfg::default()
        });
        let x = toy(&ToyPatch::default());
        let fa = fx.extract(&x);
        assert_eq!(metric_v1(&fa, &fa), 0.0);
        let quieter: Vec<f32> = x.iter().map(|v| v * 0.3).collect();
        let fq = fx.extract(&quieter);
        assert!(metric_v1(&fa, &fq) < 1e-3, "{}", metric_v1(&fa, &fq));
        let c = components(&fa, &fq);
        assert!(c.f0_cents.unwrap() < 1.0 && c.envelope < 0.05, "{c:?}");
        let silent = fx.extract(&vec![0.0; x.len()]);
        assert!(metric_v1(&fa, &silent) >= SILENT);
    }

    #[test]
    fn bigger_changes_score_farther() {
        let mut fx = FeatureExtractor::new(FeatureCfg::default());
        let base = ToyPatch::default();
        let fa = fx.extract(&toy(&base));
        let small = fx.extract(&toy(&ToyPatch {
            cutoff_hz: 2100.0,
            ..base
        }));
        let large = fx.extract(&toy(&ToyPatch {
            cutoff_hz: 6000.0,
            ..base
        }));
        let wave = fx.extract(&toy(&ToyPatch {
            wave: Wave::Triangle,
            ..base
        }));
        let (ds, dl, dw) = (
            metric_v1(&fa, &small),
            metric_v1(&fa, &large),
            metric_v1(&fa, &wave),
        );
        assert!(ds < dl && ds < dw, "{ds} {dl} {dw}");
        assert!((metric_v1(&small, &fa) - ds).abs() < 1e-12, "symmetric");
    }
}
