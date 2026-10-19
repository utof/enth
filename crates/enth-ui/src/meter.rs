//! Display maths for the output region (lane ED2): peaks in dB and a magnitude spectrum. Content, not look: the
//! constants here are maths (20 dB per decade, a Hann window's gain), so this module sits outside `editor/`'s
//! no-raw-values scan. The UI crate may use `std`'s transcendentals: nothing here decides a render or a score.

/// A linear peak in dB, no lower than `floor`.
pub fn db(peak: f32, floor: f32) -> f32 {
    if peak > 0.0 {
        (20.0 * peak.log10()).max(floor)
    } else {
        floor
    }
}

/// The magnitude spectrum of `samples` (a power-of-two count; others are cut to one) through a Hann window, as bins
/// linear in frequency from DC to Nyquist, each in dB mapped from `floor` (0) to 0 dBFS (1).
pub fn spectrum(samples: &[f32], floor: f32, bins: &mut Vec<f32>) {
    let n = if samples.len().is_power_of_two() {
        samples.len()
    } else {
        samples.len().next_power_of_two() / 2
    };
    bins.clear();
    if n < 4 {
        return;
    }
    let tau = std::f32::consts::TAU;
    let mut re: Vec<f32> = samples[samples.len() - n..]
        .iter()
        .enumerate()
        .map(|(i, &s)| s * (0.5 - 0.5 * (tau * i as f32 / n as f32).cos()))
        .collect();
    let mut im = vec![0.0f32; n];
    fft(&mut re, &mut im);
    // A full-scale sine through a Hann window peaks at n / 4.
    let scale = 4.0 / n as f32;
    bins.extend((0..=n / 2).map(|k| {
        let magnitude = (re[k] * re[k] + im[k] * im[k]).sqrt() * scale;
        ((db(magnitude, floor) - floor) / -floor).clamp(0.0, 1.0)
    }));
}

/// An in-place iterative radix-2 FFT (`re.len()` a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
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
        let angle = -std::f32::consts::TAU / len as f32;
        let (w_im, w_re) = angle.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut c_re, mut c_im) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let t_re = re[b] * c_re - im[b] * c_im;
                let t_im = re[b] * c_im + im[b] * c_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next = c_re * w_re - c_im * w_im;
                c_im = c_re * w_im + c_im * w_re;
                c_re = next;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_sine_peaks_near_0_db_in_its_bin() {
        let n = 1024;
        let k = 64; // an exact bin
        let samples: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * k as f32 * i as f32 / n as f32).sin())
            .collect();
        let mut bins = Vec::new();
        spectrum(&samples, -60.0, &mut bins);
        assert_eq!(bins.len(), n / 2 + 1);
        let (peak, &level) = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_eq!(peak, k);
        assert!(level > 0.98, "{level}");
        assert!(bins[k + 8] < 0.2, "far bins are quiet: {}", bins[k + 8]);
        let mut silent = Vec::new();
        spectrum(&vec![0.0; n], -60.0, &mut silent);
        assert!(silent.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn peaks_read_in_db_with_a_floor() {
        assert_eq!(db(1.0, -60.0), 0.0);
        assert!((db(0.5, -60.0) + 6.0206).abs() < 1e-3);
        assert_eq!(db(0.0, -60.0), -60.0);
        assert_eq!(db(1e-9, -60.0), -60.0);
    }
}
