//! **Metric v1.1** (`logmel-pool85-l1/1`, lane SS, R1/S1; ADR-0007's 2026-10-20 note, fix 1): metric v1's log-mel
//! with the mel magnitudes pooled over about 85 ms before the log, then the same L1. A separate id beside
//! [`METRIC_V1`](crate::distance::METRIC_V1); metric v1 is untouched.
//!
//! **Why.** Frame-wise log-mel L1 reads the free-running start phase of an oscillator and the realisation of noise
//! as a difference: on six real-engine patches, a render at another seed cost 0.04–0.28 under v1. Pooling the
//! magnitudes over ~85 ms first averages the phase beating and the noise frames out, so that floor drops 2–20×
//! (a single saw 0.04–0.15 → 0.002–0.007) while a cutoff move keeps its size (a pad's cutoff × 1.26: 0.192 → 0.193),
//! and on the owner's session-1 pairs, re-rendered exactly, ρ is 0.81 against v1's 0.78 (report 02 §2.3; the
//! experiment is `~/code/enth-scratch/2026-10-20-r1-s1/02/exp`).
//!
//! **Definition.** For each of [`FFT_SIZES`](crate::features::FFT_SIZES) (hop n/2), consecutive, non-overlapping
//! blocks of [`POOL_FRAMES`] frames (4, 8 and 16: 85.3 ms each at 48 kHz); per block and mel band, the mean of the
//! frames' mel magnitudes (`exp` of [`Features::logmel`]: the magnitude plus v1's 1e-5 floor), then its natural log.
//! A trailing partial block is dropped, except that a clip shorter than one block is one block. The distance is the
//! mean absolute difference per resolution over the blocks both clips have, averaged over the resolutions;
//! [`SILENT`](crate::distance::SILENT) when either clip is silent. Report 02's experiment pooled the magnitudes (the
//! ADR's text says "power"); this follows the experiment, so its measurements hold.
//!
//! Pure, `libm` only (`exp`, `log`), sums in a fixed order: the same bits on every platform for the same
//! [`Features`].

use crate::distance::SILENT;
use crate::features::{BANDS, Features};

/// Stable id of metric v1.1. A change to its definition gets a new id.
pub const METRIC_V1_1: &str = "logmel-pool85-l1/1";

/// Frames per pooling block at each resolution of [`FFT_SIZES`](crate::features::FFT_SIZES): about 85 ms at
/// 48 kHz (4 × 1024, 8 × 512 and 16 × 256 samples of hop).
pub const POOL_FRAMES: [usize; 3] = [4, 8, 16];

/// A clip's pooled log-mel: what [`metric_v1_1`] compares. Compute it once per clip with [`pool`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pooled {
    /// Per resolution, `blocks × BANDS` natural logs of the mean mel magnitude.
    pub logmel: Vec<Vec<f64>>,
    /// The clip was silent ([`Features::silent`]).
    pub silent: bool,
}

/// Pool a clip's [`Features`] (see the [module docs](self)).
#[must_use]
pub fn pool(f: &Features) -> Pooled {
    if f.silent {
        return Pooled {
            logmel: Vec::new(),
            silent: true,
        };
    }
    let logmel = f
        .logmel
        .iter()
        .enumerate()
        .map(|(r, lm)| {
            let w = POOL_FRAMES.get(r).copied().unwrap_or(1).max(1);
            let frames = lm.len() / BANDS;
            let blocks = if frames == 0 { 0 } else { (frames / w).max(1) };
            let mut out = Vec::with_capacity(blocks * BANDS);
            for b in 0..blocks {
                let (lo, hi) = (b * w, ((b + 1) * w).min(frames));
                for band in 0..BANDS {
                    let mut s = 0.0f64;
                    for frame in lo..hi {
                        s += libm::exp(f64::from(lm[frame * BANDS + band]));
                    }
                    out.push(libm::log(s / (hi - lo) as f64));
                }
            }
            out
        })
        .collect();
    Pooled {
        logmel,
        silent: false,
    }
}

/// Mean absolute pooled log-mel difference, averaged over the resolutions; blocks beyond the shorter clip are
/// ignored.
#[must_use]
pub fn pooled_l1(a: &Pooled, b: &Pooled) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.logmel.iter().zip(&b.logmel) {
        let n = x.len().min(y.len());
        let mut acc = 0.0f64;
        for (p, q) in x.iter().zip(y) {
            acc += (p - q).abs();
        }
        s += acc / n.max(1) as f64;
    }
    s / a.logmel.len().max(1) as f64
}

/// Metric v1.1: [`pooled_l1`], or [`SILENT`] when either clip is silent. 0 for identical audio.
#[must_use]
pub fn metric_v1_1(a: &Pooled, b: &Pooled) -> f64 {
    if a.silent || b.silent {
        SILENT
    } else {
        pooled_l1(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::metric_v1;
    use crate::features::{FFT_SIZES, FeatureCfg, FeatureExtractor};
    use crate::test_signals::{SR, ToyPatch, ToyPerf, render_toy};

    fn toy(p: &ToyPatch, phases: (f32, f32)) -> Vec<f32> {
        render_toy(
            SR,
            p,
            &ToyPerf {
                note_hz: 220.0,
                dur_s: 1.0,
                hold_s: 0.7,
            },
            phases,
            0.0,
        )
    }

    #[test]
    fn blocks_are_85_ms_and_shapes_follow_the_frames() {
        for (n, w) in FFT_SIZES.iter().zip(POOL_FRAMES) {
            let ms = (w * n / 2) as f64 / SR * 1000.0;
            assert!((ms - 85.33).abs() < 0.01, "{ms}");
        }
        let mut fx = FeatureExtractor::new(FeatureCfg::default());
        let f = fx.extract(&toy(&ToyPatch::default(), (0.0, 0.37)));
        let p = pool(&f);
        for (r, lm) in f.logmel.iter().enumerate() {
            assert_eq!(
                p.logmel[r].len(),
                (lm.len() / BANDS / POOL_FRAMES[r]) * BANDS
            );
        }
        // a constant spectrum pools to itself
        let flat = Features {
            logmel: vec![vec![-2.0; 4 * BANDS]],
            ..Features::default()
        };
        let pf = pool(&flat);
        assert!(pf.logmel[0].iter().all(|v| (v + 2.0).abs() < 1e-6));
    }

    #[test]
    fn identical_is_zero_silent_is_far_and_it_is_symmetric() {
        let mut fx = FeatureExtractor::new(FeatureCfg::default());
        let a = pool(&fx.extract(&toy(&ToyPatch::default(), (0.0, 0.37))));
        assert_eq!(metric_v1_1(&a, &a), 0.0);
        let silent = pool(&fx.extract(&vec![0.0; 48_000]));
        assert!(silent.silent);
        assert!(metric_v1_1(&a, &silent) >= SILENT);
        let b = pool(&fx.extract(&toy(
            &ToyPatch {
                cutoff_hz: 4000.0,
                ..ToyPatch::default()
            },
            (0.0, 0.37),
        )));
        assert_eq!(metric_v1_1(&a, &b), metric_v1_1(&b, &a));
    }

    #[test]
    fn pooling_cuts_the_phase_floor_and_keeps_parameter_moves() {
        // The voice at another common start phase: v1 reads the frames' phase, v1.1 much less; a cutoff move of a
        // third of an octave keeps its size.
        let mut fx = FeatureExtractor::new(FeatureCfg::default());
        let base = ToyPatch::default();
        let fa = fx.extract(&toy(&base, (0.0, 0.0)));
        let fb = fx.extract(&toy(&base, (0.37, 0.37)));
        let fc = fx.extract(&toy(
            &ToyPatch {
                cutoff_hz: 2520.0,
                ..base
            },
            (0.0, 0.0),
        ));
        let (pa, pb, pc) = (pool(&fa), pool(&fb), pool(&fc));
        let (v1_floor, v11_floor) = (metric_v1(&fa, &fb), metric_v1_1(&pa, &pb));
        let (v1_move, v11_move) = (metric_v1(&fa, &fc), metric_v1_1(&pa, &pc));
        assert!(
            v11_floor < 0.5 * v1_floor,
            "floor {v1_floor} -> {v11_floor}"
        );
        assert!(v11_move > 0.6 * v1_move, "move {v1_move} -> {v11_move}");
        assert!(v11_move > 4.0 * v11_floor, "{v11_move} {v11_floor}");
    }
}
