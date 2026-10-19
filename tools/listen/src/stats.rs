//! Exact binomial statistics for the two tests (all against a fair coin unless a probability is given).

/// `ln n!` through `libm::lgamma`.
fn ln_fact(n: usize) -> f64 {
    libm::lgamma(n as f64 + 1.0)
}

/// `P(X = k)` for `X ~ Binomial(n, p)`.
#[must_use]
pub fn pmf(n: usize, k: usize, p: f64) -> f64 {
    if k > n {
        return 0.0;
    }
    if p <= 0.0 {
        return f64::from(u8::from(k == 0));
    }
    if p >= 1.0 {
        return f64::from(u8::from(k == n));
    }
    let ln = ln_fact(n) - ln_fact(k) - ln_fact(n - k)
        + k as f64 * libm::log(p)
        + (n - k) as f64 * libm::log1p(-p);
    libm::exp(ln)
}

/// `P(X >= k)` for `X ~ Binomial(n, p)`.
#[must_use]
pub fn tail_ge(n: usize, k: usize, p: f64) -> f64 {
    (k..=n).map(|i| pmf(n, i, p)).sum::<f64>().min(1.0)
}

/// Exact two-sided p-value of `k` successes in `n` fair trials (R's `binom.test`: the total probability of all
/// outcomes no more likely than the observed one).
#[must_use]
pub fn two_sided(n: usize, k: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let observed = pmf(n, k, 0.5);
    (0..=n)
        .map(|i| pmf(n, i, 0.5))
        .filter(|q| *q <= observed * (1.0 + 1e-7))
        .sum::<f64>()
        .min(1.0)
}

/// The smallest number of correct answers out of `n` whose one-sided p-value is at most `alpha`, if any.
#[must_use]
pub fn critical_correct(n: usize, alpha: f64) -> Option<usize> {
    (0..=n).find(|k| tail_ge(n, *k, 0.5) <= alpha)
}

/// Chance that a listener whose true accuracy is `accuracy` gets enough answers right in `n` trials to be flagged
/// as hearing a difference at one-sided level `alpha`.
#[must_use]
pub fn power(n: usize, alpha: f64, accuracy: f64) -> f64 {
    critical_correct(n, alpha).map_or(0.0, |k| tail_ge(n, k, accuracy))
}

/// Wilson score interval for `k` of `n` (95 % at `z = 1.96`). `(0, 1)` when `n = 0`.
#[must_use]
pub fn wilson(k: usize, n: usize, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n = n as f64;
    let p = k as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = z * libm::sqrt(p * (1.0 - p) / n + z2 / (4.0 * n * n)) / denom;
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

/// The 95 % two-sided normal quantile.
pub const Z95: f64 = 1.959_963_984_540_054;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_exact_values() {
        // 7 or 8 of 8: (8 + 1) / 256
        assert!((tail_ge(8, 7, 0.5) - 9.0 / 256.0).abs() < 1e-12);
        assert!((tail_ge(6, 6, 0.5) - 1.0 / 64.0).abs() < 1e-12);
        assert!((pmf(10, 3, 0.5) - 120.0 / 1024.0).abs() < 1e-12);
        assert_eq!(tail_ge(5, 0, 0.5), 1.0);
        assert_eq!(tail_ge(5, 6, 0.5), 0.0);
    }

    #[test]
    fn two_sided_matches_r() {
        // binom.test(15, 20): p-value 0.04139; binom.test(10, 20): 1; binom.test(0, 8): 0.0078125
        assert!(
            (two_sided(20, 15) - 0.041_389_465_3).abs() < 1e-8,
            "{}",
            two_sided(20, 15)
        );
        assert!((two_sided(20, 10) - 1.0).abs() < 1e-12);
        assert!((two_sided(8, 0) - 2.0 / 256.0).abs() < 1e-12);
        assert_eq!(two_sided(0, 0), 1.0);
    }

    #[test]
    fn critical_values_and_power() {
        assert_eq!(critical_correct(8, 0.05), Some(7));
        assert_eq!(critical_correct(6, 0.05), Some(6));
        assert_eq!(critical_correct(4, 0.05), None);
        assert_eq!(critical_correct(10, 0.05), Some(9));
        // P(>= 7 of 8) at accuracy 0.85
        let want = 8.0 * 0.85f64.powi(7) * 0.15 + 0.85f64.powi(8);
        assert!((power(8, 0.05, 0.85) - want).abs() < 1e-12);
        assert_eq!(power(4, 0.05, 0.95), 0.0);
        assert!(power(8, 0.05, 0.5) < 0.05);
    }

    #[test]
    fn wilson_interval_is_sane() {
        let (lo, hi) = wilson(15, 20, Z95);
        assert!(
            (lo - 0.5313).abs() < 1e-3 && (hi - 0.8882).abs() < 1e-3,
            "{lo} {hi}"
        );
        let (lo, hi) = wilson(0, 10, Z95);
        assert!(lo == 0.0 && hi > 0.2 && hi < 0.35);
        assert_eq!(wilson(0, 0, Z95), (0.0, 1.0));
    }
}
