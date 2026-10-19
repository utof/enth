//! Rank statistics and bootstrap helpers for the listening-test analysis. Pure; the caller supplies the seed.

use crate::rng::Rng;

/// Average ranks (1-based; ties share the mean of their positions).
#[must_use]
pub fn ranks(v: &[f64]) -> Vec<f64> {
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
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
}

/// Pearson correlation; NaN when either input is constant or the inputs are shorter than 2.
#[must_use]
pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return f64::NAN;
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
    if sxx <= 0.0 || syy <= 0.0 {
        f64::NAN
    } else {
        sxy / libm::sqrt(sxx * syy)
    }
}

/// Spearman rank correlation (Pearson on average ranks, so ties are handled).
#[must_use]
pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    pearson(&ranks(x), &ranks(y))
}

/// Kendall's tau-b (ties in either variable are corrected for); NaN when one variable is constant.
#[must_use]
pub fn kendall_tau_b(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    let (mut conc, mut disc, mut tie_x, mut tie_y) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        for j in i + 1..n {
            let dx = (x[i] - x[j]).partial_cmp(&0.0).map_or(0, |o| o as i32);
            let dy = (y[i] - y[j]).partial_cmp(&0.0).map_or(0, |o| o as i32);
            if dx == 0 {
                tie_x += 1.0;
            }
            if dy == 0 {
                tie_y += 1.0;
            }
            match dx * dy {
                p if p > 0 => conc += 1.0,
                p if p < 0 => disc += 1.0,
                _ => {}
            }
        }
    }
    let n0 = (n * n.saturating_sub(1)) as f64 / 2.0;
    let denom = libm::sqrt((n0 - tie_x) * (n0 - tie_y));
    if denom <= 0.0 {
        f64::NAN
    } else {
        (conc - disc) / denom
    }
}

/// Fraction of item pairs the metric orders like the listener, over pairs the listener rated differently.
/// `sim` is higher = more similar (use the negated distance). Metric ties count half. `None` = no such pairs.
#[must_use]
pub fn triplet_agreement(sim: &[f64], rating: &[f64]) -> Option<f64> {
    let n = sim.len().min(rating.len());
    let (mut agree, mut total) = (0.0, 0.0);
    for i in 0..n {
        for j in i + 1..n {
            if rating[i] == rating[j] {
                continue;
            }
            total += 1.0;
            let ds = sim[i] - sim[j];
            if ds == 0.0 {
                agree += 0.5;
            } else if (ds > 0.0) == (rating[i] > rating[j]) {
                agree += 1.0;
            }
        }
    }
    (total > 0.0).then(|| agree / total)
}

/// `reps` resamples (with replacement) of `0..n`, shared by every statistic so differences are paired.
#[must_use]
pub fn resamples(n: usize, reps: usize, seed: u64) -> Vec<Vec<usize>> {
    let mut rng = Rng::new(seed);
    (0..reps)
        .map(|_| (0..n).map(|_| rng.below(n)).collect())
        .collect()
}

/// 95 % percentile interval of the finite values (NaN if fewer than 20 are finite).
#[must_use]
pub fn percentile_ci(values: &[f64]) -> (f64, f64) {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 20 {
        return (f64::NAN, f64::NAN);
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.025), at(0.975))
}

/// Select `data[i]` for each index.
#[must_use]
pub fn pick(data: &[f64], idx: &[usize]) -> Vec<f64> {
    idx.iter().map(|&i| data[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kendall_and_spearman_match_hand_values() {
        let (x, y) = ([1.0, 2.0, 3.0, 4.0, 5.0], [1.0, 3.0, 2.0, 5.0, 4.0]);
        assert!((kendall_tau_b(&x, &y) - 0.6).abs() < 1e-12);
        assert!((spearman(&x, &y) - 0.8).abs() < 1e-12);
        // ties in both variables
        let (x, y) = ([1.0, 2.0, 2.0, 3.0], [1.0, 2.0, 3.0, 3.0]);
        assert!((kendall_tau_b(&x, &y) - 0.8).abs() < 1e-12);
        assert!((spearman(&x, &y) - 0.75 / 0.9).abs() < 1e-12);
    }

    #[test]
    fn constant_input_gives_nan_and_reversal_gives_minus_one() {
        assert!(kendall_tau_b(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_nan());
        assert!(spearman(&[2.0, 2.0], &[1.0, 2.0]).is_nan());
        let x = [1.0, 2.0, 3.0, 4.0];
        let y = [4.0, 3.0, 2.0, 1.0];
        assert!((kendall_tau_b(&x, &y) + 1.0).abs() < 1e-12);
        assert!((spearman(&x, &y) + 1.0).abs() < 1e-12);
    }

    #[test]
    fn ranks_average_ties() {
        assert_eq!(ranks(&[10.0, 20.0, 20.0, 5.0]), vec![2.0, 3.5, 3.5, 1.0]);
    }

    #[test]
    fn triplets_count_only_rated_differences() {
        // metric orders 0 > 1 > 2; listener says 0 > 1, 1 == 2, 0 > 2: three pairs, one tied rating, two agree
        let a = triplet_agreement(&[3.0, 2.0, 1.0], &[5.0, 4.0, 4.0]);
        assert_eq!(a, Some(1.0));
        let b = triplet_agreement(&[1.0, 2.0, 3.0], &[5.0, 4.0, 4.0]);
        assert_eq!(b, Some(0.0));
        assert_eq!(triplet_agreement(&[1.0, 2.0], &[3.0, 3.0]), None);
    }

    #[test]
    fn bootstrap_ci_brackets_the_estimate() {
        let mut rng = Rng::new(5);
        let x: Vec<f64> = (0..40).map(|_| rng.uniform()).collect();
        let y: Vec<f64> = x.iter().map(|v| v + 0.3 * rng.normal()).collect();
        let est = spearman(&x, &y);
        let vals: Vec<f64> = resamples(40, 500, 7)
            .iter()
            .map(|idx| spearman(&pick(&x, idx), &pick(&y, idx)))
            .collect();
        let (lo, hi) = percentile_ci(&vals);
        assert!(lo < est && est < hi, "{lo} {est} {hi}");
        assert!(hi - lo < 0.5);
    }
}
