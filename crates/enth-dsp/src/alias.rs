//! The spectral aliasing harness that S2a's module lanes share (lane S0d, freeze F0m; `docs/dsp/aliasing.md`).
//!
//! A golden hash cannot see aliasing: a module that folds a harmonic back below Nyquist is as deterministic as one
//! that does not. So each S2a module lane measures it, as `docs/dsp/osc.md` did for the oscillators, with **research
//! report 02's measure**: the power outside the harmonics of the fundamental over the power on them, in dB, between
//! 20 Hz and 20 kHz, from 32,768 samples under a 4-term Blackman-Harris window, every bin within ±8 bins of a
//! harmonic `k·f0` counting as signal. Lower is cleaner: the polyBLEP saw at 440 Hz measures −40.6 dB, the
//! wavetable oscillator below −85 dB.
//!
//! How a lane uses it:
//!
//! - **An effect** (`dist`, `comb`, `ott`, …): feed it a sine ([`sine`] here, or an `osc` sine in an engine patch)
//!   at each frequency of a sweep ([`log_sweep`], or MIDI keys through [`midi_hz`]) and measure the output against
//!   the sine's frequency. A static nonlinearity makes only harmonics of `f0`, so whatever lands between them is
//!   aliasing (or noise). A modulated effect also makes sidebands at `f0 ± rate`: measure it with its modulation
//!   depth at 0, or at a modulation rate that is a harmonic of `f0`.
//! - **A generator** (`wt`, and `wt` with PM from another generator): render it at each pitch of a sweep and measure
//!   against its fundamental. With a modulator at a frequency ratio `p/q` (in lowest terms), the output is periodic
//!   in `f0 / q`: pass that as the fundamental.
//! - **Record a bound per module**: the worst value of the sweep plus a margin, in the lane's test (through
//!   [`Sweep::assert_below`]) and in its `docs/dsp/<kind>.md`, beside the measured table.
//!
//! [`Sweep::run`] measures one rendered block per frequency, analysing its **last** [`Measure::fft_len`] samples, so
//! a render that starts with an attack or a filter settling only needs to be long enough. Everything is `f64`
//! arithmetic with the pinned `libm` and a fixed loop order, so a measurement is identical on every platform; a bound
//! still keeps a margin, because a module's output may legitimately move by an ulp between engine versions.

use core::f64::consts::PI;

use crate::util::math;

/// Report 02's analysis length: 32,768 samples (0.68 s at 48 kHz, a 1.46 Hz bin).
pub const FFT_LEN: usize = 32_768;

/// What counts as signal and what as aliasing. [`Measure::REPORT_02`] is the measure `docs/dsp/osc.md` used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// Samples analysed, a power of two (the last ones of the block).
    pub fft_len: usize,
    /// The band measured, in Hz (low and high, inclusive).
    pub band_hz: (f64, f64),
    /// Bins on each side of a harmonic `k·f0` that count as signal (the window's main lobe and a little pitch error).
    pub guard_bins: f64,
}

impl Measure {
    /// Research report 02's measure: 32,768 samples, 20 Hz to 20 kHz, ±8 bins around each harmonic.
    pub const REPORT_02: Measure = Measure {
        fft_len: FFT_LEN,
        band_hz: (20.0, 20_000.0),
        guard_bins: 8.0,
    };
}

impl Default for Measure {
    fn default() -> Self {
        Measure::REPORT_02
    }
}

/// Alias power over harmonic power in dB for a signal whose fundamental is `f0` Hz, from the last `m.fft_len`
/// samples of `x` (see the [module docs](self)). `+inf` when no harmonic power is in the band (a silent block), so
/// any bound fails.
///
/// # Panics
/// If `x` is shorter than `m.fft_len`, `m.fft_len` is not a power of two, or `f0` is not a positive frequency.
#[must_use]
pub fn alias_db(x: &[f32], f0: f64, sample_rate: f64, m: &Measure) -> f64 {
    let n = m.fft_len;
    assert!(
        n.is_power_of_two() && n >= 2,
        "fft_len must be a power of two"
    );
    assert!(
        x.len() >= n,
        "need at least {n} samples, got {}; render longer",
        x.len()
    );
    assert!(
        f0 > 0.0 && f0.is_finite(),
        "the fundamental must be a positive frequency"
    );
    let p = power_spectrum(&x[x.len() - n..]);
    let bin = sample_rate / n as f64;
    let mut harmonic = vec![false; p.len()];
    let mut k = 1.0;
    while k * f0 < sample_rate / 2.0 {
        let c = k * f0 / bin;
        let lo = (c - m.guard_bins).floor().max(0.0) as usize;
        let hi = ((c + m.guard_bins).ceil() as usize).min(p.len() - 1);
        for h in &mut harmonic[lo..=hi] {
            *h = true;
        }
        k += 1.0;
    }
    let (mut on, mut off) = (0.0, 0.0);
    for (i, &v) in p.iter().enumerate() {
        let f = i as f64 * bin;
        if f >= m.band_hz.0 && f <= m.band_hz.1 {
            if harmonic[i] {
                on += v;
            } else {
                off += v;
            }
        }
    }
    if on <= 0.0 {
        return f64::INFINITY;
    }
    10.0 * math::log10(off / on + 1e-30)
}

/// One-sided power spectrum `|X[k]|²`, `k = 0..=n/2`, of `x` under a 4-term Blackman-Harris window
/// (`x.len()` a power of two).
///
/// # Panics
/// If `x.len()` is not a power of two.
#[must_use]
pub fn power_spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    assert!(n.is_power_of_two(), "the length must be a power of two");
    let (a0, a1, a2, a3) = (0.35875, 0.48829, 0.14128, 0.01168);
    let denom = (n.max(2) - 1) as f64;
    let mut re: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let t = i as f64 / denom;
            let w = a0 - a1 * math::cos(2.0 * PI * t) + a2 * math::cos(4.0 * PI * t)
                - a3 * math::cos(6.0 * PI * t);
            f64::from(s) * w
        })
        .collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..=n / 2).map(|i| re[i] * re[i] + im[i] * im[i]).collect()
}

/// In-place radix-2 decimation-in-time FFT (`re.len() == im.len()`, a power of two), with a precomputed twiddle
/// table and a fixed loop order, so the result is the same on every platform (unlike `rustfft`, which picks its code
/// path by CPU).
///
/// # Panics
/// If the lengths differ or are not a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(
        n == im.len() && n.is_power_of_two(),
        "re and im: one power-of-two length"
    );
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let half = n / 2;
    let twiddles: Vec<(f64, f64)> = (0..half)
        .map(|k| {
            let a = -2.0 * PI * k as f64 / n as f64;
            (math::cos(a), math::sin(a))
        })
        .collect();
    let mut len = 2;
    while len <= n {
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = twiddles[k * stride];
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// `frames` samples of a sine at `freq_hz` and peak `amplitude`, starting at phase 0: an effect's test input. The
/// phase is computed in `f64` from the sample index (no accumulated drift), so the input itself is clean to about
/// −130 dB.
#[must_use]
pub fn sine(freq_hz: f64, amplitude: f32, sample_rate: f64, frames: usize) -> Vec<f32> {
    let w = 2.0 * PI * freq_hz / sample_rate;
    (0..frames)
        .map(|i| (math::sin(w * i as f64) * f64::from(amplitude)) as f32)
        .collect()
}

/// `points` frequencies spaced evenly in log frequency from `lo_hz` to `hi_hz`, both included: a stepped sweep.
///
/// # Panics
/// If `points < 2` or the band is not `0 < lo_hz < hi_hz`.
#[must_use]
pub fn log_sweep(lo_hz: f64, hi_hz: f64, points: usize) -> Vec<f64> {
    assert!(
        points >= 2 && lo_hz > 0.0 && lo_hz < hi_hz,
        "a sweep needs two points and 0 < lo < hi"
    );
    let ratio = hi_hz / lo_hz;
    (0..points)
        .map(|i| lo_hz * math::pow(ratio, i as f64 / (points - 1) as f64))
        .collect()
}

/// The frequency of MIDI key `key` (A4 = 69 = 440 Hz, equal temperament), in `f64`: the fundamental to measure an
/// engine render of that key against. (The engine's own pitch is `f32`, within about 1e-4 of this, well inside
/// [`Measure::guard_bins`].)
#[must_use]
pub fn midi_hz(key: f64) -> f64 {
    440.0 * math::exp2((key - 69.0) / 12.0)
}

/// The result of a stepped sweep: each fundamental with its alias measure, in sweep order.
#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    /// `(f0 in Hz, alias dB)` per point.
    pub points: Vec<(f64, f64)>,
}

impl Sweep {
    /// Measure `render(f0)` for each fundamental in `freqs` with [`alias_db`]. `render` returns a mono block of at
    /// least `m.fft_len` samples; its last `m.fft_len` are analysed.
    pub fn run(
        freqs: &[f64],
        sample_rate: f64,
        m: &Measure,
        mut render: impl FnMut(f64) -> Vec<f32>,
    ) -> Sweep {
        let points = freqs
            .iter()
            .map(|&f0| (f0, alias_db(&render(f0), f0, sample_rate, m)))
            .collect();
        Sweep { points }
    }

    /// [`Sweep::run`] over MIDI keys: `render(key)` is measured against [`midi_hz`]`(key)`. The usual sweep through
    /// the engine, where a note's pitch is its key.
    pub fn run_keys(
        keys: &[u8],
        sample_rate: f64,
        m: &Measure,
        mut render: impl FnMut(u8) -> Vec<f32>,
    ) -> Sweep {
        let points = keys
            .iter()
            .map(|&key| {
                let f0 = midi_hz(f64::from(key));
                (f0, alias_db(&render(key), f0, sample_rate, m))
            })
            .collect();
        Sweep { points }
    }

    /// The point with the highest (worst) alias measure; the first one on a tie.
    ///
    /// # Panics
    /// If the sweep is empty.
    #[must_use]
    pub fn worst(&self) -> (f64, f64) {
        let mut worst = *self.points.first().expect("an empty sweep");
        for &p in &self.points[1..] {
            if p.1 > worst.1 {
                worst = p;
            }
        }
        worst
    }

    /// One line per point (`  440.0 Hz  -40.6 dB`), for a test's failure message or a `docs/dsp` table.
    #[must_use]
    pub fn table(&self) -> String {
        use core::fmt::Write as _;
        let mut s = String::new();
        for &(f0, db) in &self.points {
            let _ = writeln!(s, "{f0:>9.1} Hz {db:>7.1} dB");
        }
        s
    }

    /// Assert that every point is at or below `bound_db` (a lane's recorded bound for `what`), listing the whole sweep
    /// when one is not.
    ///
    /// # Panics
    /// If a point is above the bound (or not finite).
    pub fn assert_below(&self, bound_db: f64, what: &str) {
        let (f0, db) = self.worst();
        assert!(
            db <= bound_db,
            "{what}: aliasing {db:.1} dB at {f0:.1} Hz is above the bound {bound_db:.1} dB\n{}",
            self.table()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::{MAX_DT_ANALOG, SAW, Shape, clamp_dt, clamp_pw};

    const SR: f64 = 48_000.0;

    /// One second of the polyBLEP saw at `hz`, rendered as `osc.rs`'s own tests do.
    fn saw(hz: f64) -> Vec<f32> {
        const N: usize = 32;
        let dt = clamp_dt((hz / SR) as f32, MAX_DT_ANALOG);
        let pw = clamp_pw(0.5, dt);
        let mut o = Shape::new(SAW, 0.0, pw);
        let mut x = vec![0.0f32; 48_000 / N * N];
        for c in x.as_chunks_mut::<N>().0 {
            o.render::<SAW, N>([dt, dt], [pw, pw], None, None, c);
        }
        x
    }

    /// The measure's floor is the window's: a 4-term Blackman-Harris leaks about -92 dB beyond its main lobe, so a
    /// pure sine measures about -93 dB at low pitches (osc.md's "-93 dB floor") and lower where the harmonics' guard
    /// bands cover more of the band.
    #[test]
    fn a_pure_sine_sits_at_the_window_floor() {
        for f0 in [55.0, 440.0, 3_520.0, 12_000.0] {
            let db = alias_db(&sine(f0, 0.5, SR, FFT_LEN), f0, SR, &Measure::REPORT_02);
            eprintln!("sine {f0} Hz: {db:.1} dB");
            assert!(db < -90.0, "{f0} Hz: {db:.1} dB");
        }
    }

    #[test]
    fn an_inharmonic_tone_counts_as_aliasing() {
        // A sine at 440 Hz plus one at 1,000 Hz (not a harmonic of 440) at a tenth of the amplitude: -20 dB.
        let a = sine(440.0, 0.5, SR, FFT_LEN);
        let b = sine(1_000.0, 0.05, SR, FFT_LEN);
        let x: Vec<f32> = a.iter().zip(&b).map(|(p, q)| p + q).collect();
        let db = alias_db(&x, 440.0, SR, &Measure::REPORT_02);
        assert!((db + 20.0).abs() < 0.1, "{db:.2} dB");
    }

    #[test]
    fn a_silent_block_fails_every_bound() {
        assert_eq!(
            alias_db(&vec![0.0; FFT_LEN], 440.0, SR, &Measure::REPORT_02),
            f64::INFINITY
        );
    }

    #[test]
    fn the_polyblep_saw_measures_what_osc_md_records() {
        // docs/dsp/osc.md, "Aliasing, measured": the saw at 440 Hz is -40.6 dB and at 3,520 Hz -28.8 dB with this
        // measure (research report 02). The harness must reproduce it to stay comparable.
        let sweep = Sweep::run(&[440.0, 3_520.0], SR, &Measure::REPORT_02, saw);
        let (saw440, saw3520) = (sweep.points[0].1, sweep.points[1].1);
        assert!((saw440 + 40.6).abs() < 1.0, "{}", sweep.table());
        assert!((saw3520 + 28.8).abs() < 1.0, "{}", sweep.table());
        assert_eq!(sweep.worst().0, 3_520.0);
        sweep.assert_below(-28.0, "polyBLEP saw");
    }

    #[test]
    #[should_panic(expected = "above the bound")]
    fn a_bound_that_is_too_tight_fails() {
        let sweep = Sweep {
            points: vec![(440.0, -40.0), (880.0, -30.0)],
        };
        sweep.assert_below(-35.0, "test");
    }

    #[test]
    fn sweeps_and_keys() {
        let s = log_sweep(55.0, 7_040.0, 8);
        assert_eq!(s.len(), 8);
        assert!((s[0] - 55.0).abs() < 1e-9 && (s[7] - 7_040.0).abs() < 1e-6);
        assert!((s[1] / s[0] - 2.0).abs() < 1e-9, "one octave per step");
        assert_eq!(midi_hz(69.0), 440.0);
        assert!((midi_hz(81.0) - 880.0).abs() < 1e-9);
    }
}
