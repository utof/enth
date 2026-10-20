//! The statistics the GR1 report needs: medians, Spearman's ρ with a seeded bootstrap interval (as the Phase 0 metric
//! spike did, `spikes/spike-metric/src/stats.rs`), and Wilson intervals for rates. Pure: the caller passes the seed.

/// Median of the finite values (the mean of the middle two for an even count); `None` for an empty input.
pub fn median(values: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        f64::midpoint(v[n / 2 - 1], v[n / 2])
    })
}

/// Mean; `None` for an empty input.
pub fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Average ranks (1-based; ties share the mean of their positions).
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut r = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &idx[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

/// Pearson correlation; `None` when either input is constant or shorter than 2.
fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    if n < 2 {
        return None;
    }
    let (mx, my) = (
        x[..n].iter().sum::<f64>() / n as f64,
        y[..n].iter().sum::<f64>() / n as f64,
    );
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (dx, dy) = (x[i] - mx, y[i] - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

/// Spearman's rank correlation (Pearson on average ranks, so ties are handled); `None` when either input is constant.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    pearson(&ranks(x), &ranks(y))
}

/// Seeded `SplitMix64`: every consumer owns its generator and the seed is always passed in.
struct SplitMix(u64);

impl SplitMix {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`); the modulo bias is irrelevant at the sizes used here.
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Spearman's ρ and its 95 % percentile bootstrap interval (`reps` resamples of the pairs, with replacement).
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "`rho` is the statistic; the struct holds its sample and interval"
)]
pub struct Rho {
    /// Pairs.
    pub n: usize,
    /// The estimate; `None` when either variable is constant or there are fewer than 3 pairs.
    pub rho: Option<f64>,
    /// The interval; `None` when fewer than 20 resamples gave a finite ρ.
    pub ci: Option<(f64, f64)>,
}

/// ρ of `x` against `y` and a bootstrap interval. The pairs are the unit of resampling.
pub fn spearman_ci(x: &[f64], y: &[f64], reps: usize, seed: u64) -> Rho {
    let n = x.len().min(y.len());
    if n < 3 {
        return Rho {
            n,
            rho: None,
            ci: None,
        };
    }
    let rho = spearman(&x[..n], &y[..n]);
    let mut rng = SplitMix(seed);
    let mut draws = Vec::with_capacity(reps);
    for _ in 0..reps {
        let idx: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
        let xs: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
        let ys: Vec<f64> = idx.iter().map(|&i| y[i]).collect();
        if let Some(r) = spearman(&xs, &ys) {
            draws.push(r);
        }
    }
    let ci = (draws.len() >= 20).then(|| {
        draws.sort_by(f64::total_cmp);
        let at = |q: f64| draws[((draws.len() - 1) as f64 * q).round() as usize];
        (at(0.025), at(0.975))
    });
    Rho { n, rho, ci }
}

/// The 95 % Wilson score interval of `k` successes in `n` trials; `None` for `n == 0`.
pub fn wilson(k: usize, n: usize) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let z = 1.959_963_984_540_054_f64;
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let denom = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    Some(((centre - half).max(0.0), (centre + half).min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_even_and_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        assert_eq!(
            median(&[f64::NAN, 5.0]),
            Some(5.0),
            "non-finite values are ignored"
        );
    }

    #[test]
    fn ranks_average_ties() {
        assert_eq!(ranks(&[10.0, 20.0, 20.0, 5.0]), vec![2.0, 3.5, 3.5, 1.0]);
    }

    #[test]
    fn spearman_matches_the_spikes_hand_values() {
        // The same pairs as spike-metric's stats tests.
        let (x, y) = ([1.0, 2.0, 3.0, 4.0, 5.0], [1.0, 3.0, 2.0, 5.0, 4.0]);
        assert!((spearman(&x, &y).unwrap() - 0.8).abs() < 1e-12);
        let (x, y) = ([1.0, 2.0, 2.0, 3.0], [1.0, 2.0, 3.0, 3.0]);
        assert!((spearman(&x, &y).unwrap() - 0.75 / 0.9).abs() < 1e-12);
        let (x, y) = ([1.0, 2.0, 3.0, 4.0], [4.0, 3.0, 2.0, 1.0]);
        assert!((spearman(&x, &y).unwrap() + 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_constant_variable_has_no_rho() {
        assert_eq!(spearman(&[2.0, 2.0, 2.0], &[1.0, 2.0, 3.0]), None);
        let r = spearman_ci(&[2.0; 6], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 100, 1);
        assert_eq!(r.rho, None);
        assert_eq!(r.ci, None);
        assert_eq!(
            spearman_ci(&[1.0, 2.0], &[1.0, 2.0], 100, 1).rho,
            None,
            "two pairs are too few"
        );
    }

    #[test]
    fn the_bootstrap_brackets_the_estimate_and_is_deterministic() {
        let x: Vec<f64> = (0..40).map(f64::from).collect();
        // A noisy monotone relation: y = x with a deterministic wobble.
        let y: Vec<f64> = x.iter().map(|v| v + 9.0 * ((v * 2.3).sin())).collect();
        let a = spearman_ci(&x, &y, 500, 7);
        let b = spearman_ci(&x, &y, 500, 7);
        assert_eq!(a, b, "the same seed gives the same interval");
        let (lo, hi) = a.ci.unwrap();
        let rho = a.rho.unwrap();
        assert!(lo < rho && rho < hi, "{lo} {rho} {hi}");
        assert!(hi - lo < 0.5);
        let c = spearman_ci(&x, &y, 500, 8);
        assert_ne!(a.ci, c.ci, "another seed moves the interval");
    }

    #[test]
    fn wilson_matches_the_reports_example() {
        // Report 02 §2.10: 10 of 20 has a Wilson 95 % CI of [0.30, 0.70].
        let (lo, hi) = wilson(10, 20).unwrap();
        assert!(
            (lo - 0.299).abs() < 0.005 && (hi - 0.701).abs() < 0.005,
            "{lo} {hi}"
        );
        assert_eq!(wilson(0, 0), None);
        let (lo, hi) = wilson(20, 20).unwrap();
        assert!(lo > 0.83 && (hi - 1.0).abs() < 1e-12);
    }
}
