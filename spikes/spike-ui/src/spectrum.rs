//! A small spectrum analyser for the scope band: Hann window, radix-2 FFT, magnitudes in dB
//! mapped to 0..1 for the display. Runs on the UI thread, only when the spectrum is visible and
//! new samples arrived. Buffers are allocated once.

use std::f32::consts::PI;

/// Bins shown (the benchmark's 512-bin spectrum); the FFT is twice that.
pub const BINS: usize = 512;
const SIZE: usize = BINS * 2;
/// Display floor and ceiling in dBFS.
const FLOOR_DB: f32 = -90.0;
const CEILING_DB: f32 = 0.0;

pub struct Analyser {
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Bin levels in 0..1 (0 = floor).
    pub levels: Vec<f32>,
}

impl Default for Analyser {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyser {
    pub fn new() -> Self {
        Self {
            window: (0..SIZE)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / SIZE as f32).cos())
                .collect(),
            re: vec![0.0; SIZE],
            im: vec![0.0; SIZE],
            levels: vec![0.0; BINS],
        }
    }

    /// Analyses the last `2 * BINS` samples of `samples`.
    pub fn analyse(&mut self, samples: &[f32]) {
        let tail = &samples[samples.len().saturating_sub(SIZE)..];
        self.re.fill(0.0);
        self.im.fill(0.0);
        for (i, &s) in tail.iter().enumerate() {
            self.re[i] = s * self.window[i];
        }
        fft(&mut self.re, &mut self.im);
        // A full-scale sine through a Hann window peaks at SIZE / 4.
        let reference = SIZE as f32 / 4.0;
        for (bin, level) in self.levels.iter_mut().enumerate() {
            let magnitude = self.re[bin].hypot(self.im[bin]) / reference;
            let db = 20.0 * magnitude.max(1e-9).log10();
            *level = ((db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
        }
    }
}

/// In-place iterative radix-2 FFT. `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
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
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (sin, cos) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cos - im[b] * sin;
                let ti = re[b] * sin + im[b] * cos;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_peaks_in_its_bin_near_zero_db() {
        let bin = 64;
        let samples: Vec<f32> = (0..SIZE)
            .map(|i| (2.0 * PI * bin as f32 * i as f32 / SIZE as f32).sin())
            .collect();
        let mut analyser = Analyser::new();
        analyser.analyse(&samples);
        let (peak, level) =
            analyser
                .levels
                .iter()
                .enumerate()
                .fold(
                    (0, 0.0_f32),
                    |best, (i, &l)| if l > best.1 { (i, l) } else { best },
                );
        assert_eq!(peak, bin);
        assert!(level > 0.95, "peak level {level}");
        assert!(
            analyser.levels[bin * 3] < 0.2,
            "far bins stay near the floor"
        );
    }
}
