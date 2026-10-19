//! The three candidate distances between a target's features and a candidate's.
//!
//! * A: multi-resolution log-mel L1.
//! * B: A + crest-envelope L1 + energy-weighted centroid-track L1, each divided by its median over random pairs.
//! * C: MFCC (c0..c12) mean squared error.
//!
//! All are 0 for identical audio. Pure functions of two [`Features`] values.

use crate::features::{AnalysisCfg, Analyzer, Features};
use crate::render::{N_PARAMS, RenderCfg, render_unit};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

/// Distance returned when either side is silent.
pub const SILENT: f64 = 1e3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dist {
    A,
    B,
    C,
}

impl Dist {
    pub const ALL: [Dist; 3] = [Dist::A, Dist::B, Dist::C];
}

/// Per-component divisors for the composite B.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub logmel: f64,
    pub env: f64,
    pub centroid: f64,
}

impl Scale {
    /// Median component distances over 50 seeded random pairs of the toy synth at the default settings
    /// (`Scale::calibrate(.., 42, 50)`, measured 2026-10-19; recompute for other settings).
    pub const DEFAULT: Scale = Scale {
        logmel: 1.7,
        env: 9.0,
        centroid: 0.6,
    };

    /// Median of each component over `n` random pairs `(i, i + 7 mod n)` of seeded random patches.
    pub fn calibrate(an: &mut Analyzer, cfg: &RenderCfg, seed: u64, n: usize) -> Scale {
        let mut rng = Rng::new(seed);
        let mut buf = Vec::new();
        let feats: Vec<Features> = (0..n)
            .map(|_| {
                let p: Vec<f64> = (0..N_PARAMS).map(|_| rng.uniform()).collect();
                render_unit(cfg, &p, &mut buf);
                an.features(&buf)
            })
            .collect();
        let (mut a, mut b, mut c) = (vec![], vec![], vec![]);
        for i in 0..n {
            let (x, y) = (&feats[i], &feats[(i + 7) % n]);
            if x.silent || y.silent {
                continue;
            }
            a.push(d_logmel(x, y));
            b.push(d_env(x, y));
            c.push(d_centroid(x, y));
        }
        Scale {
            logmel: median(&mut a),
            env: median(&mut b),
            centroid: median(&mut c),
        }
    }
}

/// Median of a slice (sorts it); NaN for an empty slice.
pub fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[must_use]
pub fn d_logmel(a: &Features, b: &Features) -> f64 {
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

#[must_use]
pub fn d_env(a: &Features, b: &Features) -> f64 {
    let n = a.env_db.len().min(b.env_db.len()).max(1);
    a.env_db
        .iter()
        .zip(&b.env_db)
        .map(|(p, q)| f64::from((p - q).abs()))
        .sum::<f64>()
        / n as f64
}

/// Centroid-track L1 (octaves), weighted by the mean of the two frames' energy shares.
#[must_use]
pub fn d_centroid(a: &Features, b: &Features) -> f64 {
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

#[must_use]
pub fn d_mfcc(a: &Features, b: &Features) -> f64 {
    let n = a.mfcc.len().min(b.mfcc.len()).max(1);
    a.mfcc
        .iter()
        .zip(&b.mfcc)
        .map(|(p, q)| f64::from(p - q).powi(2))
        .sum::<f64>()
        / n as f64
}

/// Median absolute f0 difference in cents over frames voiced in both; `None` if f0 was not extracted or never voiced.
#[must_use]
pub fn d_f0_cents(a: &Features, b: &Features) -> Option<f64> {
    let mut v: Vec<f64> = a
        .f0_hz
        .iter()
        .zip(&b.f0_hz)
        .filter(|(p, q)| **p > 0.0 && **q > 0.0)
        .map(|(p, q)| (1200.0 * libm::log2(f64::from(*p) / f64::from(*q))).abs())
        .collect();
    (!v.is_empty()).then(|| median(&mut v))
}

/// Per-component distances for coaching text ("brighter than target", "attack too fast") and for the listen analysis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Components {
    pub logmel: f64,
    pub env: f64,
    pub centroid: f64,
    pub mfcc: f64,
    pub f0_cents: Option<f64>,
}

#[must_use]
pub fn components(a: &Features, b: &Features) -> Components {
    if a.silent || b.silent {
        return Components {
            logmel: SILENT,
            env: SILENT,
            centroid: SILENT,
            mfcc: SILENT,
            f0_cents: None,
        };
    }
    Components {
        logmel: d_logmel(a, b),
        env: d_env(a, b),
        centroid: d_centroid(a, b),
        mfcc: d_mfcc(a, b),
        f0_cents: d_f0_cents(a, b),
    }
}

impl Components {
    #[must_use]
    pub fn distance(&self, kind: Dist, sc: &Scale) -> f64 {
        match kind {
            Dist::A => self.logmel,
            Dist::B => {
                (self.logmel / sc.logmel + self.env / sc.env + self.centroid / sc.centroid) / 3.0
            }
            Dist::C => self.mfcc,
        }
    }
}

#[must_use]
pub fn distance(kind: Dist, a: &Features, b: &Features, sc: &Scale) -> f64 {
    if a.silent || b.silent {
        return SILENT;
    }
    match kind {
        Dist::A => d_logmel(a, b),
        Dist::B => {
            (d_logmel(a, b) / sc.logmel + d_env(a, b) / sc.env + d_centroid(a, b) / sc.centroid)
                / 3.0
        }
        Dist::C => d_mfcc(a, b),
    }
}

/// Convenience for callers that only have audio: normalise, extract and compare.
pub fn audio_distance(an: &mut Analyzer, kind: Dist, a: &[f32], b: &[f32], sc: &Scale) -> f64 {
    let (fa, fb) = (an.features(a), an.features(b));
    distance(kind, &fa, &fb, sc)
}

/// Default analysis settings, re-exported for callers that build an [`Analyzer`] from a [`RenderCfg`].
#[must_use]
pub fn analysis_for(cfg: &RenderCfg) -> AnalysisCfg {
    AnalysisCfg {
        sample_rate: cfg.sample_rate,
        ..AnalysisCfg::default()
    }
}
