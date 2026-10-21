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
//! - **steep:** everything from 1/3 octave above the edge to the top is at least 30 dB below the 1/6 octave under it
//!   (at least 60 dB/octave; a 4-pole filter on a saw falls about 30);
//! - **full below:** the octave under the edge holds at least three partials (local maxima) within 35 dB of its
//!   loudest, so an isolated partial (a sine, a high square's few harmonics) is not an edge;
//! - **no bump:** the 1/6 octave under the edge is at most 10 dB above the median of those partials, so a resonant
//!   filter peak, which also falls steeply, is not an edge.
//!
//! The highest edge from 2 to 16 kHz that passes all three wins, and the cut-off is the last bin near it still within
//! 20 dB of the level under the edge. Edges within 1/3 octave of Nyquist cannot be tested, and edges above 16 kHz
//! (a 128 kbps MP3 and better) cost the search nothing audible: both count as the full band.

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
const DROP_DB: f64 = 30.0;
/// Fullness: at least this many partials (local maxima) in the octave under the edge within this many dB of its
/// loudest.
const FULL_DB: f64 = 35.0;
const FULL_PARTIALS: usize = 3;
/// No bump: the level under the edge at most this far above the median of those partials.
const BUMP_DB: f64 = 10.0;
/// The cut-off is the last bin within this many dB of the level under the edge.
const EDGE_DB: f64 = 20.0;
/// Analysis frame (s), rounded up to a power of two in samples.
const FRAME_S: f64 = 0.04;

/// The cut-off of a steep low-pass in `x[span]` (Hz), or `None` for the full band.
pub(crate) fn estimate(
    x: &[f32],
    sr: f64,
    span: (usize, usize),
    planner: &mut FftPlanner,
) -> Option<f64> {
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
    // Edge e sits between band e-1 and band e; it needs the octave under it and bands from e+4 to the top.
    let mut found = None;
    for e in 12..nb.saturating_sub(4) {
        if edges[e] < MIN_CUTOFF_HZ || edges[e] > MAX_CUTOFF_HZ {
            continue;
        }
        let under = bands[e - 2].max(bands[e - 1]);
        let above = bands[e + 4..].iter().copied().fold(-300.0, f64::max);
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
        if strong.len() < FULL_PARTIALS {
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
    Some(((last as f64 + 0.5) * bin).min(nyq))
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
        estimate(x, SR, (0, x.len()), &mut FftPlanner::new(FftPath::Scalar))
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
}
