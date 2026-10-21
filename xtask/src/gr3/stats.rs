//! **Owner: lane EVK.** Pre-declared by lane S0s (R3).
//!
//! The statistics of clause 1 (docs/r3.md, "GR3 and its hygiene"), pure Rust, **checked against hand values** (the
//! tests below work each one out on paper):
//!
//! - [`mcnemar`]: the exact `McNemar` test on the discordant targets of the pass rate (a binomial test of B's
//!   discordant wins against ½);
//! - [`signed_rank`]: the Wilcoxon signed-rank test on the per-target differences in JND units, exact (the
//!   permutation distribution of the signs, conditional on the ranks, ties averaged and zeros dropped);
//! - [`hodges_lehmann`]: the median of the Walsh averages and its 95 % interval from the signed-rank distribution;
//! - [`kaplan_meier`]: the survival curve of time to pass, censored at the cap, and its median;
//! - [`wilson`]: a pass rate's 95 % interval, for a single arm.
//!
//! A sign test needs 69 targets to detect a 65 % win rate (37 for 70 %, 158 for 60 %), which is why the set is about
//! 120. "Beats" means better at 60 s (one-sided p < 0.05) and not worse at 300 s ([`super::report`] fixes how).
//! Nothing here draws a random number.

/// The two-sided normal quantile for 95 %.
pub const Z95: f64 = 1.959_963_984_540_054;

/// `ln(k!)` for `k` up to `n`, by summing logs (exact enough for n in the thousands).
fn ln_factorials(n: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(n + 1);
    out.push(0.0);
    let mut acc = 0.0;
    for k in 1..=n {
        acc += (k as f64).ln();
        out.push(acc);
    }
    out
}

/// P(X ≥ k) for X ~ Binomial(n, ½).
pub fn binom_half_ge(n: usize, k: usize) -> f64 {
    if k == 0 {
        return 1.0;
    }
    if k > n {
        return 0.0;
    }
    let lf = ln_factorials(n);
    let ln_half_n = -(n as f64) * std::f64::consts::LN_2;
    let p: f64 = (k..=n)
        .map(|j| (lf[n] - lf[j] - lf[n - j] + ln_half_n).exp())
        .sum();
    p.min(1.0)
}

/// P(X ≤ k) for X ~ Binomial(n, ½).
#[cfg(test)]
fn binom_half_le(n: usize, k: usize) -> f64 {
    if k >= n {
        return 1.0;
    }
    binom_half_ge(n, n - k)
}

/// `McNemar`'s exact test on paired pass/fail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct McNemar {
    /// Targets only A passed.
    pub a_only: usize,
    /// Targets only B passed.
    pub b_only: usize,
    /// One-sided p that B passes more often: P(X ≥ `b_only`) with X ~ Bin(`a_only` + `b_only`, ½).
    pub p_b_better: f64,
    /// One-sided p that A passes more often.
    pub p_a_better: f64,
    /// Two-sided p: twice the smaller tail, at most 1.
    pub p_two_sided: f64,
}

/// `McNemar`'s exact test from the discordant counts.
pub fn mcnemar(a_only: usize, b_only: usize) -> McNemar {
    let n = a_only + b_only;
    let p_b = binom_half_ge(n, b_only);
    let p_a = binom_half_ge(n, a_only);
    McNemar {
        a_only,
        b_only,
        p_b_better: p_b,
        p_a_better: p_a,
        p_two_sided: (2.0 * p_b.min(p_a)).min(1.0),
    }
}

/// The Wilcoxon signed-rank test.
#[derive(Clone, Debug, PartialEq)]
pub struct SignedRank {
    /// Non-zero differences (zeros are dropped).
    pub n: usize,
    /// The sum of the ranks of the positive differences.
    pub w_plus: f64,
    /// The sum of the ranks of the negative differences.
    pub w_minus: f64,
    /// One-sided p that the differences tend to be positive: P(W+ ≥ observed) under the null.
    pub p_greater: f64,
    /// One-sided p that they tend to be negative: P(W+ ≤ observed).
    pub p_less: f64,
    /// Two-sided: twice the smaller, at most 1.
    pub p_two_sided: f64,
}

/// Mid-ranks of `v` (1-based, ties averaged), in input order.
fn mid_ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut ranks = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i + 1;
        while j < idx.len() && v[idx[j]] == v[idx[i]] {
            j += 1;
        }
        // Positions i..j (0-based) share the mean of ranks i+1..=j.
        let r = (i + 1 + j) as f64 / 2.0;
        for &k in &idx[i..j] {
            ranks[k] = r;
        }
        i = j;
    }
    ranks
}

/// The null distribution of W+ over integer rank weights `w2` (doubled ranks, so mid-ranks stay integers): entry
/// `s` is P(2·W+ = s) when every sign is + or − with probability ½.
fn signed_rank_null(w2: &[usize]) -> Vec<f64> {
    let total: usize = w2.iter().sum();
    let mut dist = vec![0.0f64; total + 1];
    dist[0] = 1.0;
    let mut reach = 0usize;
    for &w in w2 {
        for s in (0..=reach).rev() {
            let p = dist[s];
            if p != 0.0 {
                dist[s] = 0.5 * p;
                dist[s + w] += 0.5 * p;
            }
        }
        reach += w;
    }
    dist
}

/// The exact Wilcoxon signed-rank test of `d` (zeros dropped, ties given mid-ranks; the null distribution is the
/// exact one for those ranks).
pub fn signed_rank(d: &[f64]) -> SignedRank {
    let nz: Vec<f64> = d
        .iter()
        .copied()
        .filter(|x| *x != 0.0 && x.is_finite())
        .collect();
    let n = nz.len();
    if n == 0 {
        return SignedRank {
            n,
            w_plus: 0.0,
            w_minus: 0.0,
            p_greater: 1.0,
            p_less: 1.0,
            p_two_sided: 1.0,
        };
    }
    let abs: Vec<f64> = nz.iter().map(|x| x.abs()).collect();
    let ranks = mid_ranks(&abs);
    let w_plus: f64 = nz
        .iter()
        .zip(&ranks)
        .filter(|(x, _)| **x > 0.0)
        .map(|(_, r)| r)
        .sum();
    let total: f64 = ranks.iter().sum();
    let w2: Vec<usize> = ranks.iter().map(|r| (2.0 * r).round() as usize).collect();
    let dist = signed_rank_null(&w2);
    let obs = (2.0 * w_plus).round() as usize;
    let ge: f64 = dist[obs.min(dist.len() - 1)..].iter().sum();
    let le: f64 = dist[..=obs.min(dist.len() - 1)].iter().sum();
    SignedRank {
        n,
        w_plus,
        w_minus: total - w_plus,
        p_greater: ge.min(1.0),
        p_less: le.min(1.0),
        p_two_sided: (2.0 * ge.min(le)).min(1.0),
    }
}

/// The Hodges–Lehmann estimate of a paired shift and its interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HodgesLehmann {
    /// The median of the Walsh averages `(dᵢ + dⱼ) / 2`, i ≤ j.
    pub estimate: f64,
    /// The interval's ends: Walsh averages `k` and `M + 1 − k` (1-based, M = n(n + 1)/2), where `k − 1` is the
    /// largest count with P(T+ ≤ k − 1) ≤ 2.5 % under the untied signed-rank null; ±∞ when n is too small.
    pub lo: f64,
    /// The upper end.
    pub hi: f64,
    /// The interval's exact coverage under the untied null (`1 − 2·P(T+ ≤ k − 1)`).
    pub coverage: f64,
}

/// The Hodges–Lehmann estimate and 95 % interval of `d` (every finite value, zeros included).
pub fn hodges_lehmann(d: &[f64]) -> Option<HodgesLehmann> {
    let v: Vec<f64> = d.iter().copied().filter(|x| x.is_finite()).collect();
    let n = v.len();
    if n == 0 {
        return None;
    }
    let mut walsh = Vec::with_capacity(n * (n + 1) / 2);
    for i in 0..n {
        for j in i..n {
            walsh.push(f64::midpoint(v[i], v[j]));
        }
    }
    walsh.sort_by(f64::total_cmp);
    let m = walsh.len();
    let estimate = median_sorted(&walsh);
    // The untied null of T+ over ranks 1..=n (doubled weights, so read every other entry).
    let w2: Vec<usize> = (1..=n).map(|r| 2 * r).collect();
    let dist = signed_rank_null(&w2);
    let mut cum = 0.0;
    let mut k = 0usize; // the number of lower-tail counts with cumulative probability ≤ 2.5 %
    let mut tail = 0.0;
    for c in 0..=(m) {
        cum += dist.get(2 * c).copied().unwrap_or(0.0);
        if cum <= 0.025 + 1e-12 {
            k = c + 1;
            tail = cum;
        } else {
            break;
        }
    }
    if k == 0 {
        return Some(HodgesLehmann {
            estimate,
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
            coverage: 1.0,
        });
    }
    Some(HodgesLehmann {
        estimate,
        lo: walsh[k - 1],
        hi: walsh[m - k],
        coverage: 1.0 - 2.0 * tail,
    })
}

/// The median of sorted values (the mean of the middle two for an even count); NaN for none.
pub fn median_sorted(v: &[f64]) -> f64 {
    match v.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => v[n / 2],
        n => f64::midpoint(v[n / 2 - 1], v[n / 2]),
    }
}

/// The median of `v` (NaN values sort last; +∞ counts as a value).
pub fn median(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    s.sort_by(f64::total_cmp);
    median_sorted(&s)
}

/// One step of a Kaplan–Meier curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KmStep {
    /// The event time.
    pub t: f64,
    /// At risk just before it.
    pub at_risk: usize,
    /// Events at it.
    pub events: usize,
    /// S(t) just after it: the share not yet passed.
    pub survival: f64,
}

/// The Kaplan–Meier curve of `(time, event)` (`event` false: censored at that time). Censored times tied with an
/// event count as at risk at it.
pub fn kaplan_meier(data: &[(f64, bool)]) -> Vec<KmStep> {
    let mut times: Vec<f64> = data.iter().filter(|(_, e)| *e).map(|(t, _)| *t).collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    let mut s = 1.0;
    let mut out = Vec::new();
    for t in times {
        let at_risk = data.iter().filter(|(u, _)| *u >= t).count();
        let events = data.iter().filter(|(u, e)| *e && *u == t).count();
        if at_risk == 0 {
            continue;
        }
        s *= 1.0 - events as f64 / at_risk as f64;
        out.push(KmStep {
            t,
            at_risk,
            events,
            survival: s,
        });
    }
    out
}

/// The median time to pass: the first event time where S(t) ≤ ½; `None` when the curve never gets there.
pub fn km_median(curve: &[KmStep]) -> Option<f64> {
    // A hair of tolerance: a product such as 0.7 × 5/7 should read as ½, not as ½ + 1 ulp.
    curve
        .iter()
        .find(|s| s.survival <= 0.5 + 1e-12)
        .map(|s| s.t)
}

/// The Wilson score interval (95 %) of `k` passes in `n`.
pub fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n) = (k as f64, n as f64);
    let z2 = Z95 * Z95;
    let centre = (k + z2 / 2.0) / (n + z2);
    let half = Z95 * (k * (n - k) / n + z2 / 4.0).sqrt() / (n + z2);
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn binomial_tails_by_hand() {
        // C(10,8) + C(10,9) + C(10,10) = 45 + 10 + 1 = 56, over 2^10.
        close(binom_half_ge(10, 8), 56.0 / 1024.0);
        close(binom_half_le(10, 2), 56.0 / 1024.0);
        close(binom_half_ge(5, 0), 1.0);
        close(binom_half_ge(5, 6), 0.0);
        close(binom_half_ge(1, 1), 0.5);
    }

    #[test]
    fn mcnemar_by_hand() {
        // 2 targets only A passed, 8 only B: P(X ≥ 8 | 10, ½) = 56/1024.
        let m = mcnemar(2, 8);
        close(m.p_b_better, 56.0 / 1024.0);
        close(m.p_two_sided, 112.0 / 1024.0);
        // P(X ≥ 2 | 10) = 1 − 11/1024.
        close(m.p_a_better, 1013.0 / 1024.0);
        // Five discordant targets, all B's: 1/32.
        close(mcnemar(0, 5).p_b_better, 1.0 / 32.0);
        // No discordant target: nothing to say.
        close(mcnemar(0, 0).p_b_better, 1.0);
        close(mcnemar(0, 0).p_two_sided, 1.0);
    }

    #[test]
    fn signed_rank_by_hand() {
        // All five positive: W+ = 15, and only the all-plus assignment reaches it: 1/32.
        let w = signed_rank(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        close(w.w_plus, 15.0);
        close(w.p_greater, 1.0 / 32.0);
        close(w.p_less, 1.0);
        // One negative (rank 1): W+ = 14; W+ ≥ 14 by {all} and {all but 1}: 2/32.
        let w = signed_rank(&[-1.0, 2.0, 3.0, 4.0, 5.0]);
        close(w.w_plus, 14.0);
        close(w.w_minus, 1.0);
        close(w.p_greater, 2.0 / 32.0);
        close(w.p_two_sided, 4.0 / 32.0);
        // A zero is dropped and ties take mid-ranks: |d| = 1, 1, 2, 2, 3 -> ranks 1.5, 1.5, 3.5, 3.5, 5; W+ = 13.5,
        // W- = 1.5. W- ≤ 1.5 by {}, {1.5a}, {1.5b}: 3/32.
        let w = signed_rank(&[0.0, 1.0, -1.0, 2.0, 2.0, 3.0]);
        assert_eq!(w.n, 5);
        close(w.w_plus, 13.5);
        close(w.w_minus, 1.5);
        close(w.p_greater, 3.0 / 32.0);
        // Nothing but zeros.
        close(signed_rank(&[0.0, 0.0]).p_greater, 1.0);
    }

    #[test]
    fn hodges_lehmann_by_hand() {
        // d = 1..6: 21 Walsh averages, median 3.5. P(T+ ≤ 0) = 1/64 ≤ 2.5 % < P(T+ ≤ 1) = 2/64, so k = 1: the
        // interval is the smallest and largest Walsh average, [1, 6], with coverage 1 − 2/64.
        let h = hodges_lehmann(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        close(h.estimate, 3.5);
        close(h.lo, 1.0);
        close(h.hi, 6.0);
        close(h.coverage, 1.0 - 2.0 / 64.0);
        // d = 1..8: subsets of {1..8} summing to at most 3 are {}, {1}, {2}, {3}, {1,2}: 5/256 ≤ 2.5 %; at most 4
        // adds {4}, {1,3}: 7/256 > 2.5 %. So k = 4: Walsh averages 4 and 33 of 36. Sorted from the bottom: 1, 1.5,
        // 2, 2 -> 2; from the top: 8, 7.5, 7, 7 -> 7. The tables' critical value for n = 8 is 3 (k = 4).
        let h = hodges_lehmann(&[3.0, 1.0, 8.0, 2.0, 7.0, 4.0, 6.0, 5.0]).unwrap();
        close(h.estimate, 4.5);
        close(h.lo, 2.0);
        close(h.hi, 7.0);
        close(h.coverage, 1.0 - 2.0 * 5.0 / 256.0);
        // n = 5 cannot reach 95 %: P(T+ ≤ 0) = 1/32 > 2.5 %.
        let h = hodges_lehmann(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert!(h.lo.is_infinite() && h.hi.is_infinite());
        assert!(hodges_lehmann(&[]).is_none());
    }

    #[test]
    fn kaplan_meier_by_hand() {
        // Times 1, 2, 2, 3 (censored), 4: S = 4/5 after 1, × 2/4 after 2 = 0.4, × 0/1 after 4 = 0. Median 2.
        let km = kaplan_meier(&[
            (1.0, true),
            (2.0, true),
            (2.0, true),
            (3.0, false),
            (4.0, true),
        ]);
        assert_eq!(km.len(), 3);
        close(km[0].survival, 0.8);
        assert_eq!((km[1].at_risk, km[1].events), (4, 2));
        close(km[1].survival, 0.4);
        assert_eq!(km[2].at_risk, 1);
        close(km[2].survival, 0.0);
        assert_eq!(km_median(&km), Some(2.0));
        // Everything censored at the cap: no median.
        let km = kaplan_meier(&[(60.0, false), (60.0, false)]);
        assert_eq!(km, Vec::new());
        assert_eq!(km_median(&km), None);
        // One pass of three: S = 2/3, never ≤ ½.
        let km = kaplan_meier(&[(10.0, true), (60.0, false), (60.0, false)]);
        assert_eq!(km_median(&km), None);
    }

    #[test]
    fn wilson_by_hand() {
        // 0 of 10: centre = half = (z²/2)/(10 + z²) = 1.920729 / 13.841459 = 0.138766…, so [0, 0.277533…].
        let (lo, hi) = wilson(0, 10);
        close(lo, 0.0);
        assert!((hi - 0.277_532_8).abs() < 1e-6, "{hi}");
        // 5 of 10: ½ ± z·√(2.5 + z²/4)/(10 + z²) = ½ ± 0.263408…
        let (lo, hi) = wilson(5, 10);
        assert!((lo - 0.236_592).abs() < 1e-5, "{lo}");
        assert!((hi - 0.763_408).abs() < 1e-5, "{hi}");
    }

    #[test]
    fn medians() {
        close(median(&[3.0, 1.0, 2.0]), 2.0);
        close(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        close(median(&[1.0, f64::INFINITY, 2.0]), 2.0);
        assert!(median(&[]).is_nan());
    }
}
