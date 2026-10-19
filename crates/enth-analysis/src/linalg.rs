//! Small dense least squares for the envelope and beat fits. Normal equations with partial pivoting: the systems
//! are at most about 20 × 20 and well conditioned (abscissae scaled to −1..1), and the summation order is fixed, so
//! results are deterministic.

/// Least-squares solution of `rows · c ≈ y` by the normal equations, with a tiny ridge for safety. `None` when the
/// system is singular or empty.
pub(crate) fn lstsq(rows: &[Vec<f64>], y: &[f64]) -> Option<Vec<f64>> {
    let k = rows.first()?.len();
    let mut a = vec![vec![0.0f64; k + 1]; k];
    for (r, &v) in rows.iter().zip(y) {
        for i in 0..k {
            for j in 0..k {
                a[i][j] += r[i] * r[j];
            }
            a[i][k] += r[i] * v;
        }
    }
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += 1e-9 * (1.0 + row[i]);
    }
    for col in 0..k {
        let piv = (col..k).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        a.swap(col, piv);
        let d = a[col][col];
        if d.abs() < 1e-14 {
            return None;
        }
        for r in 0..k {
            if r != col {
                let f = a[r][col] / d;
                if f != 0.0 {
                    let pivot = a[col].clone();
                    for (dst, src) in a[r][col..].iter_mut().zip(&pivot[col..]) {
                        *dst -= f * src;
                    }
                }
            }
        }
    }
    Some((0..k).map(|i| a[i][k] / a[i][i]).collect())
}

/// `x^p` by repeated multiplication (std `powi` is banned for determinism).
pub(crate) fn ipow(x: f64, p: usize) -> f64 {
    let mut v = 1.0;
    for _ in 0..p {
        v *= x;
    }
    v
}

/// Least-squares polynomial of degree `deg` through `y` (equally spaced, abscissa scaled to −1..1), evaluated at the
/// same points. Falls back to the mean when the fit fails.
pub(crate) fn polyfit_eval(y: &[f64], deg: usize) -> Vec<f64> {
    let n = y.len();
    let xs: Vec<f64> = (0..n)
        .map(|i| 2.0 * i as f64 / (n - 1).max(1) as f64 - 1.0)
        .collect();
    let rows: Vec<Vec<f64>> = xs
        .iter()
        .map(|&x| (0..=deg).map(|p| ipow(x, p)).collect())
        .collect();
    if let Some(c) = lstsq(&rows, y) {
        rows.iter()
            .map(|r| r.iter().zip(&c).map(|(a, b)| a * b).sum())
            .collect()
    } else {
        let m = y.iter().sum::<f64>() / n.max(1) as f64;
        vec![m; n]
    }
}

/// Straight line through `(i, y[i])`: `(slope per index, value at the last index)`.
pub(crate) fn line_fit(y: &[f64]) -> (f64, f64) {
    let n = y.len() as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for (k, v) in y.iter().enumerate() {
        let x = k as f64;
        sx += x;
        sy += v;
        sxx += x * x;
        sxy += x * v;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx).max(1e-9);
    let icpt = (sy - slope * sx) / n.max(1.0);
    (slope, icpt + slope * (n - 1.0))
}

/// Median (upper median for even lengths); NaN for an empty slice.
pub(crate) fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut w = v.to_vec();
    w.sort_by(f64::total_cmp);
    w[w.len() / 2]
}

/// The `p` quantile (nearest rank) of `v`; NaN for an empty slice.
pub(crate) fn quantile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut w = v.to_vec();
    w.sort_by(f64::total_cmp);
    w[libm::round((w.len() - 1) as f64 * p) as usize]
}

/// Centred moving average over `w` points; the edges average what is available.
pub(crate) fn movavg(v: &[f64], w: usize) -> Vec<f64> {
    if w <= 1 {
        return v.to_vec();
    }
    let mut pre = Vec::with_capacity(v.len() + 1);
    let mut acc = 0.0;
    pre.push(0.0);
    for &x in v {
        acc += x;
        pre.push(acc);
    }
    (0..v.len())
        .map(|i| {
            let a = i.saturating_sub(w / 2);
            let b = (i + w - w / 2).min(v.len());
            (pre[b] - pre[a]) / (b - a) as f64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_recover_polynomials_and_lines() {
        let y: Vec<f64> = (0..50)
            .map(|i| {
                let x = 2.0 * f64::from(i) / 49.0 - 1.0;
                1.0 - 2.0 * x + 0.5 * x * x * x
            })
            .collect();
        let f = polyfit_eval(&y, 3);
        assert!(y.iter().zip(&f).all(|(a, b)| (a - b).abs() < 1e-6));
        let (s, end) = line_fit(&[1.0, 3.0, 5.0, 7.0]);
        assert!((s - 2.0).abs() < 1e-12 && (end - 7.0).abs() < 1e-12);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(quantile(&[0.0, 1.0, 2.0, 3.0, 4.0], 0.75), 3.0);
        assert_eq!(movavg(&[0.0, 3.0, 6.0], 3), vec![1.5, 3.0, 4.5]);
        assert_eq!(ipow(2.0, 10), 1024.0);
    }
}
