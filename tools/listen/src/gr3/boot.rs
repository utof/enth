//! The statistics of GR3's decision rule, fixed in advance (docs/listening.md, "5. GR3"; docs/r3.md, "Clause 2").
//!
//! **The data** are judgements: one listener's forced choice on one reference, `b = true` when the listener found arm B's
//! candidate (model + search) closer to the reference. Only real trials count (catch trials never enter), and only
//! listeners who passed the attention check.
//!
//! **The estimate** is the pooled share `wins / n` over all judgements.
//!
//! **The interval is a two-stage cluster bootstrap.** The clusters are the *references* (a degraded copy is judged
//! against its clean original, so the two share one cluster when they share that file). Each replicate:
//!
//! 1. *references:* draws `C` clusters with replacement (`C` = the number of clusters);
//! 2. *listeners:* for each drawn cluster, draws with replacement as many of its listeners as it has, and adds the
//!    wins and judgements of each drawn listener *in that cluster*.
//!
//! The replicate's value is its wins over its judgements. The 95 % interval is the 2.5th and 97.5th percentile of the
//! replicates (linear interpolation between order statistics, R's type 7). The lower end is "the 95 % lower bound" of
//! the rule. The generator is `SplitMix64` from the fixed seed [`BOOTSTRAP_SEED`], consumed in a fixed order, and
//! listeners and clusters are sorted by name, so the same answers give the same interval on every machine. The
//! script `tools/listen/tests/gr3/bootstrap_ref.py` does the same with the standard library alone, and a test compares
//! the two.
//!
//! **The design effect** compares the bootstrap variance of the pooled share with the variance of the same share if
//! every judgement were an independent trial, `p (1 - p) / n`. It is measured on the *references-only* replicates
//! (stage 1 alone, taken from the same draws): drawing listeners again inside a cluster counts each judgement's own
//! noise a second time, which would inflate the ratio above 1 even for independent trials. The two-stage interval is
//! the one that decides, and it is the wider (more cautious) of the two; the report prints both.

/// The fixed seed of every bootstrap in the report ("GR3BOOTS" in ASCII). Not a flag: the rule is decided in advance.
pub const BOOTSTRAP_SEED: u64 = 0x4752_3342_4f4f_5453;
/// Replicates per interval.
pub const DEFAULT_REPLICATES: usize = 10_000;
/// The interval's tail probability on each side (95 % interval).
pub const TAIL: f64 = 0.025;

/// One listener's forced choice on one reference.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Judgement {
    /// Who listened.
    pub listener: String,
    /// The item (reference id) judged.
    pub item: String,
    /// The cluster the item belongs to (items judged against the same reference audio share one).
    pub cluster: String,
    /// The reporting stratum.
    pub stratum: String,
    /// The reference was a degraded upload (judged against its clean original).
    pub degraded: bool,
    /// Arm B's candidate was chosen as closer to the reference.
    pub b: bool,
}

/// `SplitMix64` (the same generator and the same `below` as the Python reference).
#[derive(Clone, Debug)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// A generator from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// An integer in `0..n` (`n > 0`): the next 64 bits modulo `n` (the bias is below 2^-40 for any `n` here).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// The judgements grouped for resampling: clusters sorted by name, each holding its listeners sorted by name, each
/// listener's `(wins, judgements)` in that cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grouped {
    groups: Vec<Vec<(u64, u64)>>,
    /// Distinct listeners.
    pub listeners: usize,
    /// Wins of arm B over all judgements.
    pub wins: u64,
    /// Judgements.
    pub n: u64,
}

impl Grouped {
    /// Group `js`.
    #[must_use]
    pub fn new(js: &[&Judgement]) -> Self {
        use std::collections::{BTreeMap, BTreeSet};
        let mut by: BTreeMap<&str, BTreeMap<&str, (u64, u64)>> = BTreeMap::new();
        let mut listeners = BTreeSet::new();
        let (mut wins, mut n) = (0, 0);
        for j in js {
            let cell = by
                .entry(j.cluster.as_str())
                .or_default()
                .entry(j.listener.as_str())
                .or_default();
            cell.0 += u64::from(j.b);
            cell.1 += 1;
            listeners.insert(j.listener.as_str());
            wins += u64::from(j.b);
            n += 1;
        }
        Self {
            groups: by
                .into_values()
                .map(|l| l.into_values().collect())
                .collect(),
            listeners: listeners.len(),
            wins,
            n,
        }
    }

    /// Clusters.
    #[must_use]
    pub fn clusters(&self) -> usize {
        self.groups.len()
    }

    /// One replicate's `(references only, references then listeners)` shares, or `None` without data.
    fn replicate(&self, rng: &mut SplitMix64) -> Option<(f64, f64)> {
        let c = self.groups.len();
        if c == 0 {
            return None;
        }
        let drawn: Vec<usize> = (0..c).map(|_| rng.below(c)).collect();
        let (mut w1, mut n1) = (0u64, 0u64);
        for &g in &drawn {
            for &(w, n) in &self.groups[g] {
                w1 += w;
                n1 += n;
            }
        }
        let (mut w2, mut n2) = (0u64, 0u64);
        for &g in &drawn {
            let listeners = &self.groups[g];
            for _ in 0..listeners.len() {
                let (w, n) = listeners[rng.below(listeners.len())];
                w2 += w;
                n2 += n;
            }
        }
        Some((w1 as f64 / n1 as f64, w2 as f64 / n2 as f64))
    }
}

/// Order statistic `q` (0..1) of a sorted slice, linear interpolation (R's type 7, numpy's default).
#[must_use]
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let frac = pos - lo as f64;
    if lo + 1 < sorted.len() {
        sorted[lo] + frac * (sorted[lo + 1] - sorted[lo])
    } else {
        sorted[lo]
    }
}

/// Sample variance (n - 1), summed left to right.
#[must_use]
pub fn variance(x: &[f64]) -> f64 {
    let n = x.len() as f64;
    let mean = x.iter().fold(0.0, |s, v| s + v) / n;
    x.iter().fold(0.0, |s, v| s + (v - mean) * (v - mean)) / (n - 1.0)
}

/// The estimate, its intervals and the design effect for one set of judgements.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Boot {
    /// Judgements.
    pub n: u64,
    /// Wins of arm B.
    pub wins: u64,
    /// Clusters (references).
    pub clusters: usize,
    /// Listeners.
    pub listeners: usize,
    /// Replicates behind the intervals.
    pub replicates: usize,
    /// The pooled share of arm B.
    pub p: f64,
    /// The 95 % interval of the rule (references, then listeners): lower end, "the lower bound".
    pub lo: f64,
    /// The 95 % interval of the rule: upper end.
    pub hi: f64,
    /// The rule's replicate standard deviation.
    pub se: f64,
    /// The references-only 95 % interval: lower end.
    pub lo_ref: f64,
    /// The references-only 95 % interval: upper end.
    pub hi_ref: f64,
    /// The references-only replicate standard deviation.
    pub se_ref: f64,
    /// Observed design effect: `se_ref^2 / (p (1 - p) / n)`; `None` when `p` is 0 or 1.
    pub deff: Option<f64>,
    /// The same ratio for the rule's replicates (it carries the second stage's extra noise).
    pub deff_two_stage: Option<f64>,
}

/// Run the bootstrap on `js`, or `None` for no judgements. `replicates` is at least 2.
#[must_use]
pub fn bootstrap(js: &[&Judgement], replicates: usize, seed: u64) -> Option<Boot> {
    let g = Grouped::new(js);
    if g.n == 0 {
        return None;
    }
    let replicates = replicates.max(2);
    let mut rng = SplitMix64::new(seed);
    let (mut one, mut two) = (
        Vec::with_capacity(replicates),
        Vec::with_capacity(replicates),
    );
    for _ in 0..replicates {
        let (a, b) = g.replicate(&mut rng)?;
        one.push(a);
        two.push(b);
    }
    let (se_ref, se) = (variance(&one).sqrt(), variance(&two).sqrt());
    one.sort_by(f64::total_cmp);
    two.sort_by(f64::total_cmp);
    let p = g.wins as f64 / g.n as f64;
    let srs = p * (1.0 - p) / g.n as f64;
    let ratio = |s: f64| (srs > 0.0).then(|| s * s / srs);
    Some(Boot {
        n: g.n,
        wins: g.wins,
        clusters: g.clusters(),
        listeners: g.listeners,
        replicates,
        p,
        lo: quantile(&two, TAIL),
        hi: quantile(&two, 1.0 - TAIL),
        se,
        lo_ref: quantile(&one, TAIL),
        hi_ref: quantile(&one, 1.0 - TAIL),
        se_ref,
        deff: ratio(se_ref),
        deff_two_stage: ratio(se),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(listener: &str, cluster: &str, b: bool) -> Judgement {
        Judgement {
            listener: listener.into(),
            item: cluster.into(),
            cluster: cluster.into(),
            stratum: "s".into(),
            degraded: false,
            b,
        }
    }

    #[test]
    fn splitmix64_matches_the_published_sequence() {
        // The reference outputs of SplitMix64 seeded with 0 (the generator of Java's SplittableRandom).
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn quantile_is_type_seven() {
        let x = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&x, 0.0), 1.0);
        assert_eq!(quantile(&x, 1.0), 4.0);
        assert!((quantile(&x, 0.5) - 2.5).abs() < 1e-15);
        assert!((quantile(&x, 0.25) - 1.75).abs() < 1e-15);
        assert_eq!(quantile(&[7.0], 0.3), 7.0);
    }

    #[test]
    fn grouping_counts_listeners_clusters_and_wins() {
        let js = [
            j("o", "c1", true),
            j("f", "c1", false),
            j("o", "c2", true),
            j("o", "c2", true),
        ];
        let refs: Vec<&Judgement> = js.iter().collect();
        let g = Grouped::new(&refs);
        assert_eq!((g.clusters(), g.listeners, g.wins, g.n), (2, 2, 3, 4));
    }

    #[test]
    fn all_wins_has_a_degenerate_interval_and_no_design_effect() {
        let js: Vec<Judgement> = (0..10).map(|i| j("o", &format!("c{i}"), true)).collect();
        let refs: Vec<&Judgement> = js.iter().collect();
        let b = bootstrap(&refs, 200, BOOTSTRAP_SEED).unwrap();
        assert_eq!((b.p, b.lo, b.hi), (1.0, 1.0, 1.0));
        assert_eq!(b.deff, None);
        assert!(bootstrap(&[], 200, 1).is_none());
    }

    #[test]
    fn independent_single_listener_data_gives_a_design_effect_near_one() {
        // 400 references, one listener, an independent coin per reference: no clustering to find.
        let mut rng = SplitMix64::new(11);
        let js: Vec<Judgement> = (0..400)
            .map(|i| j("o", &format!("c{i:03}"), rng.below(100) < 60))
            .collect();
        let refs: Vec<&Judgement> = js.iter().collect();
        let b = bootstrap(&refs, 4000, BOOTSTRAP_SEED).unwrap();
        let d = b.deff.unwrap();
        assert!((d - 1.0).abs() < 0.12, "design effect {d}");
        // one listener per cluster: the second stage changes nothing
        assert_eq!((b.lo, b.hi, b.se), (b.lo_ref, b.hi_ref, b.se_ref));
    }

    #[test]
    fn a_strong_reference_effect_shows_as_a_design_effect_above_one() {
        // Two listeners agree on every reference, but references differ (half always B, half never): the 2 judgements
        // per reference are perfectly correlated, so the design effect is 2.
        let mut js = Vec::new();
        for i in 0..200 {
            let b = i % 2 == 0;
            js.push(j("o", &format!("c{i:03}"), b));
            js.push(j("f", &format!("c{i:03}"), b));
        }
        let refs: Vec<&Judgement> = js.iter().collect();
        let boot = bootstrap(&refs, 4000, BOOTSTRAP_SEED).unwrap();
        let d = boot.deff.unwrap();
        assert!((d - 2.0).abs() < 0.25, "design effect {d}");
    }

    #[test]
    fn the_same_seed_gives_the_same_interval() {
        let js: Vec<Judgement> = (0..30)
            .map(|i| j("o", &format!("c{i}"), i % 3 != 0))
            .collect();
        let refs: Vec<&Judgement> = js.iter().collect();
        let a = bootstrap(&refs, 500, BOOTSTRAP_SEED).unwrap();
        let b = bootstrap(&refs, 500, BOOTSTRAP_SEED).unwrap();
        let c = bootstrap(&refs, 500, BOOTSTRAP_SEED + 1).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
