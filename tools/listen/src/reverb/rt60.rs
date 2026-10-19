//! Reverberation time from an impulse response: Schroeder backward integration, T20 extrapolated to 60 dB.

/// RT60 in seconds of a stereo wet signal, measured from frame `start`: the time the backward-integrated energy
/// takes to fall from -5 dB to -25 dB, times three. `None` for silence, and for a window shorter than the tail:
/// backward integration of a truncated tail always falls to -25 dB, but only near the window's end, so a crossing
/// in the last fifth of the window is not trusted.
#[must_use]
pub fn rt60_t20(left: &[f32], right: &[f32], sample_rate: u32, start: usize) -> Option<f64> {
    let n = left.len().min(right.len());
    if start + 16 >= n {
        return None;
    }
    let energy: Vec<f64> = (start..n)
        .map(|i| f64::from(left[i]).powi(2) + f64::from(right[i]).powi(2))
        .collect();
    let total: f64 = energy.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    // EDC[i] = sum of energy[i..]; find the first crossings of -5 dB and -25 dB
    let mut remaining = total;
    let (mut t5, mut t25) = (None, None);
    for (i, e) in energy.iter().enumerate() {
        let db = 10.0 * libm::log10((remaining / total).max(1e-300));
        if t5.is_none() && db <= -5.0 {
            t5 = Some(i);
        }
        if t25.is_none() && db <= -25.0 {
            t25 = Some(i);
            break;
        }
        remaining -= e;
    }
    let (t5, t25) = (t5?, t25?);
    if t25 as f64 > 0.8 * energy.len() as f64 {
        return None;
    }
    Some(3.0 * (t25 - t5) as f64 / f64::from(sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exponentially decaying noise with a known RT60.
    fn decaying_noise(sr: u32, rt60: f64, seconds: f64) -> (Vec<f32>, Vec<f32>) {
        let mut s = 99u32;
        let mut noise = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / 8_388_608.0 - 1.0
        };
        let n = (seconds * f64::from(sr)) as usize;
        let k = libm::log(1000.0) / rt60; // amplitude falls by 60 dB (x1000) in rt60 seconds
        let env = |i: usize| libm::exp(-k * i as f64 / f64::from(sr)) as f32;
        let l = (0..n).map(|i| noise() * env(i)).collect();
        let r = (0..n).map(|i| noise() * env(i)).collect();
        (l, r)
    }

    #[test]
    fn recovers_a_known_rt60() {
        for rt in [0.5, 1.0, 2.5] {
            let (l, r) = decaying_noise(16_000, rt, 2.0 * rt + 0.5);
            let m = rt60_t20(&l, &r, 16_000, 0).unwrap();
            assert!((m - rt).abs() / rt < 0.08, "rt60 {rt}: measured {m}");
        }
    }

    #[test]
    fn silence_and_short_windows_give_none() {
        assert!(rt60_t20(&[0.0; 4000], &[0.0; 4000], 16_000, 0).is_none());
        let (l, r) = decaying_noise(16_000, 3.0, 0.3);
        assert!(
            rt60_t20(&l, &r, 16_000, 0).is_none(),
            "the window ends before -25 dB"
        );
        assert!(rt60_t20(&l, &r, 16_000, l.len()).is_none());
    }
}
