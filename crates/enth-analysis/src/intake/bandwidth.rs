//! **Owner: lane FE** (the intake, freeze point F1s).
//!
//! The effective bandwidth of an upload (docs/r3.md, "The intake"): where its spectrum stops, which is the low-pass of
//! a codec (a 64 kbps MP3 cuts near 11 kHz; the search copied it, with a cutoff of 4.58 kHz), a resampled file, or a
//! phone codec. The estimate is within 1/3 octave (FE's acceptance). A file recorded at a low rate simply has nothing
//! above its Nyquist frequency: there is no resampler in the product path, and that is not reported as a cut-off.
//! Lane OBJ's robust distance (and TP's k-NN) compare only the band below the cut-off.
//!
//! A cut-off is told from an instrument's own roll-off by its shape, on the energy-weighted power spectrum of the
//! sounding part (Hann frames of about 40 ms, the peak bin of each 1/12-octave band from 1 kHz up):
//!
//! - **steep:** everything from 1/3 octave above the edge to the top is at least 20 dB below the 1/6 octave under it
//!   (at least 40 dB/octave over that half octave; a 4-pole filter on a saw falls about 30 dB/octave, so 15 dB there,
//!   and a 48 kbps MP3 leaves its top 25 to 30 dB down, not more);
//! - **full below:** the octave under the edge holds at least three partials (local maxima) within 35 dB of its
//!   loudest, so an isolated partial (a sine, a high square's few harmonics) is not an edge;
//! - **no bump:** the 1/6 octave under the edge is at most 6 dB above the median of those partials, so a resonant
//!   filter peak, which also falls steeply, is not an edge.
//!
//! The level under the edge must also be within 80 dB of the loudest bin (lower is rounding noise, or nothing worth
//! masking). The highest edge from 2 kHz to 16 kHz (and to 3/4 of Nyquist) that passes wins, judged on what lies
//! above it up to 0.9 × Nyquist, where a render's own filters fall steeply (the bilinear transform puts their zero at
//! Nyquist); the cut-off is the last bin near it still within 20 dB of the level under the edge, reported up to
//! 16 kHz. Higher cut-offs (a 128 kbps MP3 and better) cost the search nothing audible and count as the full band.

use super::welch;
use crate::fft::FftPlanner;

/// Lowest cut-off searched (Hz): phone codecs are the narrowest real cases (about 3.4 kHz).
const MIN_CUTOFF_HZ: f64 = 2000.0;
/// Highest cut-off searched (Hz). A 128 kbps MP3 cuts near 16 kHz; above that a cut-off costs the search nothing
/// audible, and a render's own filters fall steeply towards Nyquist (the bilinear transform puts their zero there).
const MAX_CUTOFF_HZ: f64 = 16_000.0;
/// Bands start here (Hz), so the octave under the lowest edge is covered.
const BANDS_FROM_HZ: f64 = 1000.0;
const BANDS_PER_OCTAVE: f64 = 12.0;
/// The drop, in dB, from the 1/6 octave under the edge to everything from 1/3 octave above it.
const DROP_DB: f64 = 20.0;
/// Fullness: at least this many partials (local maxima) in the octave under the edge within this many dB of its
/// loudest.
const FULL_DB: f64 = 35.0;
const FULL_PARTIALS: usize = 3;
/// Fullness when the decoder says the file is lossy ([`IntakeCfg::codec`](super::IntakeCfg::codec), lane SK3): the
/// codec has likely low-passed it, so two partials under the edge are enough (a high note's sparse spectrum holds
/// fewer than three in the octave under an MP3's cut-off). An isolated partial (a sine) is still not an edge.
const FULL_PARTIALS_LOSSY: usize = 2;
/// No bump: the level under the edge at most this far above the median of those partials.
const BUMP_DB: f64 = 6.0;
/// What is under an edge is within this many dB of the loudest bin (lower is rounding noise, or nothing worth
/// masking).
const RANGE_DB: f64 = 80.0;
/// Edges up to this share of Nyquist, judged on what is above them up to `NYQUIST_SHARE` of it.
const EDGE_SHARE: f64 = 0.75;
const NYQUIST_SHARE: f64 = 0.9;
/// The cut-off is the last bin within this many dB of the level under the edge.
const EDGE_DB: f64 = 20.0;
/// Analysis frame (s), rounded up to a power of two in samples.
const FRAME_S: f64 = 0.04;

/// The cut-off of a steep low-pass in `x[span]` (Hz), or `None` for the full band. `lossy`: the decoder says the file
/// went through a lossy codec, so two partials under an edge are enough ([`FULL_PARTIALS_LOSSY`]).
pub(crate) fn estimate(
    x: &[f32],
    sr: f64,
    span: (usize, usize),
    lossy: bool,
    planner: &mut FftPlanner,
) -> Option<f64> {
    let full_partials = if lossy {
        FULL_PARTIALS_LOSSY
    } else {
        FULL_PARTIALS
    };
    let n = ((sr * FRAME_S) as usize).next_power_of_two().max(256);
    let (pw, _) = welch(planner, x, span.0, span.1, n);
    let bin = sr / n as f64;
    let nyq = sr / 2.0;
    let db: Vec<f64> = pw
        .iter()
        .map(|p| 10.0 * libm::log10(p.max(1e-30)))
        .collect();
    // Band edges from 1 kHz to Nyquist, and each band's peak bin level.
    let mut edges = Vec::new();
    let mut k = 0.0;
    loop {
        let f = BANDS_FROM_HZ * libm::exp2(k / BANDS_PER_OCTAVE);
        if f >= nyq {
            break;
        }
        edges.push(f);
        k += 1.0;
    }
    edges.push(nyq);
    let bands: Vec<f64> = edges
        .windows(2)
        .map(|w| {
            let a = (libm::ceil(w[0] / bin) as usize).min(db.len() - 1);
            let b = (libm::ceil(w[1] / bin) as usize).clamp(a + 1, db.len());
            db[a..b].iter().copied().fold(-300.0, f64::max)
        })
        .collect();
    let nb = bands.len();
    // The loudest bin from 30 Hz up: what is under an edge must be within `RANGE_DB` of it.
    let loudest = db[((30.0 / bin) as usize).min(db.len() - 1)..]
        .iter()
        .copied()
        .fold(-300.0, f64::max);
    // Bands that reach past 0.9 × Nyquist are left out of "above": a render's own filters fall steeply there.
    let top = edges
        .iter()
        .position(|&f| f > NYQUIST_SHARE * nyq)
        .unwrap_or(nb)
        .min(nb);
    // Edge e sits between band e-1 and band e; it needs the octave under it and bands from e+4 to the top.
    let mut found = None;
    for e in 12..top.saturating_sub(4) {
        if edges[e] < MIN_CUTOFF_HZ || edges[e] > MAX_CUTOFF_HZ.min(EDGE_SHARE * nyq) {
            continue;
        }
        let under = bands[e - 2].max(bands[e - 1]);
        if under < loudest - RANGE_DB {
            continue;
        }
        let above = bands[e + 4..top].iter().copied().fold(-300.0, f64::max);
        if under - above < DROP_DB {
            continue;
        }
        // The partials of the octave under the edge: local maxima over ±2 bins.
        let (oa, ob) = (
            (libm::ceil(edges[e] / 2.0 / bin) as usize).max(2),
            (libm::ceil(edges[e] / bin) as usize).min(db.len() - 2),
        );
        let parts: Vec<f64> = (oa..ob)
            .filter(|&k| {
                db[k] > db[k - 1] && db[k] >= db[k + 1] && db[k] >= db[k - 2] && db[k] >= db[k + 2]
            })
            .map(|k| db[k])
            .collect();
        let top = parts.iter().copied().fold(-300.0, f64::max);
        let mut strong: Vec<f64> = parts.into_iter().filter(|&p| p >= top - FULL_DB).collect();
        if strong.len() < full_partials {
            continue;
        }
        strong.sort_by(f64::total_cmp);
        if under > strong[strong.len() / 2] + BUMP_DB {
            continue;
        }
        found = Some((e, under));
    }
    let (e, under) = found?;
    let a = libm::ceil(edges[e - 2] / bin) as usize;
    let b = (libm::ceil(edges[e + 4] / bin) as usize).min(db.len());
    let last = (a..b).rev().find(|&i| db[i] >= under - EDGE_DB)?;
    let fc = ((last as f64 + 0.5) * bin).min(nyq);
    (fc <= MAX_CUTOFF_HZ).then_some(fc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::{FftPath, FftPlanner};
    use crate::test_signals::{
        SR, ToyPatch, ToyPerf, Wave, oscillator, render_toy, sine, white_noise,
    };
    use realfft::num_complex::Complex32;

    /// A brick-wall low-pass at `fc`, by zeroing the bins of one whole-signal FFT above it.
    fn brick_wall(x: &[f32], fc: f64) -> Vec<f32> {
        let n = x.len() & !1;
        let mut pl = FftPlanner::new(FftPath::Scalar);
        let (fw, inv) = (pl.forward(n), pl.inverse(n));
        let mut buf = x[..n].to_vec();
        let mut spec = vec![Complex32::new(0.0, 0.0); fw.bins()];
        fw.forward(&mut buf, &mut spec, &mut fw.make_scratch());
        let cut = (fc / (SR / n as f64)) as usize;
        for c in spec.iter_mut().skip(cut) {
            *c = Complex32::new(0.0, 0.0);
        }
        let mut out = vec![0.0; n];
        inv.inverse(&mut spec, &mut out, &mut inv.make_scratch());
        out.iter().map(|v| v / n as f32).collect()
    }

    fn bw(x: &[f32]) -> Option<f64> {
        estimate(x, SR, (0, x.len()), false, &mut FftPlanner::new(FftPath::Scalar))
    }

    fn bw_lossy(x: &[f32]) -> Option<f64> {
        estimate(x, SR, (0, x.len()), true, &mut FftPlanner::new(FftPath::Scalar))
    }

    fn bright(note_hz: f64) -> Vec<f32> {
        let p = ToyPatch {
            cutoff_hz: 18_000.0,
            sustain_db: -3.0,
            ..ToyPatch::default()
        };
        let perf = ToyPerf {
            note_hz,
            dur_s: 1.0,
            hold_s: 0.8,
        };
        render_toy(SR, &p, &perf, (0.0, 0.37), 0.0)
    }

    #[test]
    fn a_brick_wall_is_found_within_a_third_of_an_octave() {
        for note in [65.4, 220.0, 523.25] {
            let x = bright(note);
            assert!(bw(&x).is_none(), "{note}: an open filter is the full band");
            for fc in [3400.0, 8000.0, 11_000.0, 16_000.0] {
                let got = bw(&brick_wall(&x, fc)).unwrap_or_else(|| panic!("{note} {fc}"));
                let oct = libm::log2(got / fc).abs();
                assert!(oct < 1.0 / 3.0, "{note} {fc}: {got}");
            }
        }
        // noise too
        let n = white_noise(SR, 0.3, 1.0, 4);
        let got = bw(&brick_wall(&n, 11_000.0)).unwrap();
        assert!(libm::log2(got / 11_000.0).abs() < 0.1, "{got}");
    }

    #[test]
    fn dark_sounds_sines_and_resonances_are_not_cut_offs() {
        for cutoff in [300.0, 1000.0, 4000.0] {
            let p = ToyPatch {
                cutoff_hz: cutoff,
                ..ToyPatch::default()
            };
            let x = render_toy(SR, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
            assert!(bw(&x).is_none(), "filter at {cutoff}");
            let q = ToyPatch {
                cutoff_hz: cutoff,
                resonance_q: 3.0,
                ..ToyPatch::default()
            };
            let y = render_toy(SR, &q, &ToyPerf::default(), (0.0, 0.37), 0.0);
            assert!(
                bw(&y).is_none(),
                "resonant filter at {cutoff}: {:?}",
                bw(&y)
            );
        }
        for f in [440.0, 2500.0, 6000.0] {
            assert!(bw(&sine(SR, f, 0.5, 1.0)).is_none(), "sine {f}");
        }
        let sq = oscillator(SR, Wave::Square, 1500.0, 1.0, 0.0);
        assert!(bw(&sq).is_none(), "a high square");
    }

    /// Lane SK3 (F1s should-fix 2): a high note keeps two partials in the octave under an MP3-like cut-off, which the
    /// clean rule (three) does not trust; the decoder's lossy hint does. What has no edge stays without one.
    #[test]
    fn the_lossy_hint_finds_a_high_notes_cut_off_and_nothing_else() {
        let x = bright(2500.0);
        let cut = brick_wall(&x, 11_000.0);
        assert_eq!(bw(&cut), None, "the clean rule leaves a sparse high note alone");
        let got = bw_lossy(&cut).expect("the lossy hint finds the cut-off");
        assert!(libm::log2(got / 11_000.0).abs() < 1.0 / 3.0, "{got}");
        assert!(bw_lossy(&x).is_none(), "an open filter is the full band");
        for f in [440.0, 2500.0, 6000.0] {
            assert!(bw_lossy(&sine(SR, f, 0.5, 1.0)).is_none(), "sine {f}");
        }
        for note in [65.4, 220.0, 523.25] {
            let low = bright(note);
            for fc in [3400.0, 8000.0, 11_000.0, 16_000.0] {
                let clean = bw(&brick_wall(&low, fc));
                let hinted = bw_lossy(&brick_wall(&low, fc));
                assert_eq!(clean, hinted, "{note} {fc}: a full spectrum reads the same either way");
            }
        }
    }
}
