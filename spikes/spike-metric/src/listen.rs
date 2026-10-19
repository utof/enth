//! The owner's listening test: stimulus design, loudness-matched audio, answer files and the analysis that turns
//! ratings into Kendall tau-b and Spearman correlations (with bootstrap CIs) per distance.
//!
//! Design (04-metric-search-final.md, Q4): 30 target/candidate pairs spread over six distance-A quantile bins (5 per
//! bin), plus 2 identical and 2 obviously different anchor pairs; rated 1 to 5 in two sessions with reshuffled order,
//! random A/B order and random file names, so the owner is blind to the metric and session 1 vs session 2 gives a
//! test-retest ceiling. Pure: this module does no I/O (the CLI reads and writes the files).

use crate::distance::{Components, Dist, SILENT, Scale};
use crate::features::Analyzer;
use crate::render::{N_PARAMS, RenderCfg, Wave, render_unit};
use crate::rng::Rng;
use crate::stats::{kendall_tau_b, percentile_ci, pick, resamples, spearman, triplet_agreement};
use serde::{Deserialize, Serialize};

pub const SCHEMA_MANIFEST: &str = "enth.spike-metric.listen-manifest/1";
pub const SCHEMA_ANSWERS: &str = "enth.spike-metric.listen-answers/1";
pub const SCHEMA_REPORT: &str = "enth.spike-metric.listen-report/1";
/// Silence between the two clips of a pair.
pub const GAP_S: f32 = 0.5;
/// Fade-out at the end of each clip, so a long release isn't cut with a click. There is no fade-in: the voice starts
/// at exactly zero (its amp envelope begins at 0), and a fade-in would cut the transient of percussive sounds and
/// change what the scorer hears.
pub const FADE_S: f32 = 0.01;
/// Peak ceiling after loudness matching; both clips of a pair are scaled together if either would exceed it.
pub const PEAK_CEILING: f32 = 0.98;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    Test,
    AnchorSame,
    AnchorDifferent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairSpec {
    pub pair: usize,
    pub kind: PairKind,
    /// Distance-A quantile bin (test pairs only); 0 = closest.
    pub bin: Option<usize>,
    /// Normalised parameter vectors of the two sounds.
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    pub components: Components,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trial {
    /// 1-based position in the session.
    pub n: usize,
    /// Random file stem; carries no information about the pair.
    pub id: String,
    pub pair: usize,
    /// Play b first.
    pub swap: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub seed: u64,
    pub render: RenderCfg,
    pub target_lufs: f64,
    pub scale: Scale,
    pub pairs: Vec<PairSpec>,
    /// `sessions[0]` is session 1, `sessions[1]` is session 2.
    pub sessions: Vec<Vec<Trial>>,
}

#[derive(Clone, Debug)]
pub struct DesignCfg {
    pub seed: u64,
    pub n_test: usize,
    pub n_bins: usize,
    /// Candidate pairs generated before binning.
    pub pool: usize,
    pub n_same: usize,
    pub n_diff: usize,
}

impl Default for DesignCfg {
    fn default() -> Self {
        Self {
            seed: 20_261_019,
            n_test: 30,
            n_bins: 6,
            pool: 600,
            n_same: 2,
            n_diff: 2,
        }
    }
}

fn random_patch(rng: &mut Rng) -> Vec<f64> {
    (0..N_PARAMS).map(|_| rng.uniform()).collect()
}

/// Jitter the continuous parameters by `s * N(0, 1)`; with probability `s / 2` switch the waveform.
fn perturb(rng: &mut Rng, base: &[f64], s: f64) -> Vec<f64> {
    let mut v = base.to_vec();
    for x in v.iter_mut().skip(1) {
        *x = (*x + s * rng.normal()).clamp(0.0, 1.0);
    }
    if rng.uniform() < 0.5 * s {
        let cur = Wave::from_unit(base[0]);
        let others: Vec<Wave> = Wave::ALL.iter().copied().filter(|w| *w != cur).collect();
        v[0] = others[rng.below(others.len())].unit();
    }
    v
}

/// Evaluates the per-component distances inside each `(a, b)` pair (the CLI supplies a parallel version).
pub type PairEval<'a> = &'a dyn Fn(&[(Vec<f64>, Vec<f64>)]) -> Vec<Components>;

/// Choose the pairs and both session orders. Deterministic in `cfg.seed` and the evaluator.
#[must_use]
pub fn design(
    cfg: &DesignCfg,
    render: &RenderCfg,
    scale: &Scale,
    target_lufs: f64,
    eval: PairEval<'_>,
) -> Manifest {
    let mut rng = Rng::new(cfg.seed);
    // 1. pool: random targets, candidates at log-uniform perturbation sizes (plus a few unrelated patches)
    let pool_pairs: Vec<(Vec<f64>, Vec<f64>)> = (0..cfg.pool)
        .map(|_| {
            let t = random_patch(&mut rng);
            let c = if rng.uniform() < 0.1 {
                random_patch(&mut rng)
            } else {
                let s = libm::exp(
                    libm::log(0.004) + rng.uniform() * (libm::log(0.6) - libm::log(0.004)),
                );
                perturb(&mut rng, &t, s)
            };
            (t, c)
        })
        .collect();
    let comps = eval(&pool_pairs);
    let mut keep: Vec<usize> = (0..cfg.pool)
        .filter(|&i| comps[i].logmel < SILENT)
        .collect();
    keep.sort_by(|&i, &j| comps[i].logmel.total_cmp(&comps[j].logmel).then(i.cmp(&j)));
    // 2. test pairs: equal numbers from each distance-A quantile bin
    let mut pairs: Vec<PairSpec> = Vec::new();
    for bin in 0..cfg.n_bins {
        let lo = bin * keep.len() / cfg.n_bins;
        let hi = (bin + 1) * keep.len() / cfg.n_bins;
        let want = cfg.n_test / cfg.n_bins + usize::from(bin < cfg.n_test % cfg.n_bins);
        let mut cell: Vec<usize> = keep[lo..hi].to_vec();
        rng.shuffle(&mut cell);
        for &i in cell.iter().take(want) {
            pairs.push(PairSpec {
                pair: pairs.len(),
                kind: PairKind::Test,
                bin: Some(bin),
                a: pool_pairs[i].0.clone(),
                b: pool_pairs[i].1.clone(),
                components: comps[i],
            });
        }
    }
    // 3. anchors: identical sounds, and the most different pool pairs with unequal waveforms (distinct targets)
    let same_try: Vec<(Vec<f64>, Vec<f64>)> = (0..cfg.n_same * 3)
        .map(|_| {
            let p = random_patch(&mut rng);
            (p.clone(), p)
        })
        .collect();
    let same_comps = eval(&same_try);
    for (i, p) in same_try.iter().enumerate() {
        if pairs
            .iter()
            .filter(|q| q.kind == PairKind::AnchorSame)
            .count()
            == cfg.n_same
        {
            break;
        }
        if same_comps[i].logmel < SILENT {
            pairs.push(PairSpec {
                pair: pairs.len(),
                kind: PairKind::AnchorSame,
                bin: None,
                a: p.0.clone(),
                b: p.1.clone(),
                components: same_comps[i],
            });
        }
    }
    let mut n_diff = 0;
    for &i in keep.iter().rev() {
        if n_diff == cfg.n_diff {
            break;
        }
        if Wave::from_unit(pool_pairs[i].0[0]) != Wave::from_unit(pool_pairs[i].1[0]) {
            pairs.push(PairSpec {
                pair: pairs.len(),
                kind: PairKind::AnchorDifferent,
                bin: None,
                a: pool_pairs[i].0.clone(),
                b: pool_pairs[i].1.clone(),
                components: comps[i],
            });
            n_diff += 1;
        }
    }
    // 4. two sessions: reshuffled order, random A/B order, random unique ids
    let mut used = Vec::new();
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let mut order: Vec<usize> = (0..pairs.len()).collect();
        rng.shuffle(&mut order);
        let trials = order
            .iter()
            .enumerate()
            .map(|(k, &pair)| {
                let id = loop {
                    let cand = format!("k{:06x}", rng.next_u64() & 0xff_ffff);
                    if !used.contains(&cand) {
                        used.push(cand.clone());
                        break cand;
                    }
                };
                Trial {
                    n: k + 1,
                    id,
                    pair,
                    swap: rng.uniform() < 0.5,
                }
            })
            .collect();
        sessions.push(trials);
    }
    Manifest {
        schema: SCHEMA_MANIFEST.to_string(),
        seed: cfg.seed,
        render: *render,
        target_lufs,
        scale: *scale,
        pairs,
        sessions,
    }
}

/// One trial's audio: clip A, a gap, clip B (or B first when `swap`). Each clip is scaled to the target loudness
/// (BS.1770 integrated, the same measure the scorer uses, taken after the fade-out), and if either would exceed the
/// peak ceiling both are scaled down together so the pair stays level-matched.
pub fn stimulus(an: &mut Analyzer, cfg: &RenderCfg, pair: &PairSpec, swap: bool) -> Vec<f32> {
    let fade = ((cfg.sample_rate * FADE_S) as usize).max(1);
    let mut clips: Vec<Vec<f32>> = [&pair.a, &pair.b]
        .iter()
        .map(|p| {
            let mut buf = Vec::new();
            render_unit(cfg, p, &mut buf);
            let n = buf.len();
            for (i, v) in buf.iter_mut().enumerate() {
                *v *= ((n - 1 - i) as f32 / fade as f32).min(1.0);
            }
            let g = an.gain_to_target(&buf).expect("designed pairs are audible");
            for v in &mut buf {
                *v *= g;
            }
            buf
        })
        .collect();
    let peak = clips.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > PEAK_CEILING {
        let k = PEAK_CEILING / peak;
        for v in clips.iter_mut().flatten() {
            *v *= k;
        }
    }
    if swap {
        clips.swap(0, 1);
    }
    let gap = vec![0.0f32; (cfg.sample_rate * GAP_S) as usize];
    let mut out = clips.remove(0);
    out.extend_from_slice(&gap);
    out.extend_from_slice(&clips[0]);
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub n: usize,
    pub id: String,
    /// 1 = very different .. 5 = indistinguishable.
    pub rating: u8,
    pub replays: u32,
    /// Time from first playback to the answer.
    pub ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnswerFile {
    pub schema: String,
    pub session: u8,
    /// True for the `--dry-run` answers (generated from the distances; never the owner's).
    pub synthetic: bool,
    pub answers: Vec<Answer>,
}

impl AnswerFile {
    #[must_use]
    pub fn new(session: u8, synthetic: bool) -> Self {
        Self {
            schema: SCHEMA_ANSWERS.to_string(),
            session,
            synthetic,
            answers: Vec::new(),
        }
    }
}

/// Plausible ratings for `--dry-run`: similarity falls with the distance-A rank of the pair, plus noise;
/// anchors get their expected 5 and 1.
#[must_use]
pub fn synthetic_answers(m: &Manifest, session: u8, seed: u64) -> AnswerFile {
    let mut rng = Rng::new(seed ^ (u64::from(session) << 32));
    let mut test_a: Vec<f64> = m
        .pairs
        .iter()
        .filter(|p| p.kind == PairKind::Test)
        .map(|p| p.components.logmel)
        .collect();
    test_a.sort_by(f64::total_cmp);
    let mut f = AnswerFile::new(session, true);
    for t in &m.sessions[usize::from(session) - 1] {
        let p = &m.pairs[t.pair];
        let ideal = match p.kind {
            PairKind::AnchorSame => 5.0,
            PairKind::AnchorDifferent => 1.0,
            PairKind::Test => {
                let q = test_a.iter().filter(|v| **v < p.components.logmel).count() as f64
                    / test_a.len().max(1) as f64;
                5.0 - 4.0 * q + 0.6 * rng.normal()
            }
        };
        f.answers.push(Answer {
            n: t.n,
            id: t.id.clone(),
            rating: ideal.round().clamp(1.0, 5.0) as u8,
            replays: 0,
            ms: 0,
        });
    }
    f
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Ci {
    pub estimate: f64,
    pub lo: f64,
    pub hi: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Retest {
    pub n: usize,
    pub spearman: Ci,
    pub tau_b: Ci,
    pub exact_agreement: f64,
    pub mean_abs_diff: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Anchors {
    pub same_mean_rating: Option<f64>,
    pub different_mean_rating: Option<f64>,
    /// Identical pairs rated 4.5 or more and obviously different pairs 1.5 or less.
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricReport {
    pub distance: Dist,
    pub spearman: Ci,
    pub tau_b: Ci,
    /// Pairs of pairs rated differently that the metric orders the same way.
    pub triplet_agreement: Ci,
    /// Same, restricted to pairs in the closer half of the distance-A bins (the decisions that matter near the pass threshold).
    pub triplet_agreement_near: Option<f64>,
    /// Spearman >= 0.7, its lower CI >= 0.4, and tau-b within 0.15 of the test-retest tau-b (when there is one).
    pub gate_pass: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct TauDiff {
    pub first: Dist,
    pub second: Dist,
    pub diff: Ci,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: String,
    pub synthetic: bool,
    pub sessions: usize,
    pub n_test_pairs: usize,
    pub bootstrap_reps: usize,
    pub retest: Option<Retest>,
    pub anchors: Anchors,
    pub metrics: Vec<MetricReport>,
    pub tau_differences: Vec<TauDiff>,
    pub recommendation: String,
}

fn ci_of(estimate: f64, samples: &[f64]) -> Ci {
    let (lo, hi) = percentile_ci(samples);
    Ci { estimate, lo, hi }
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Correlate each distance with the owner's mean rating. Accepts one or two answer files.
#[must_use]
pub fn analyze(m: &Manifest, files: &[AnswerFile], reps: usize, seed: u64) -> Report {
    let np = m.pairs.len();
    let mut rat: Vec<Vec<Option<f64>>> = vec![vec![None; np]; files.len()];
    for (si, f) in files.iter().enumerate() {
        let trials = &m.sessions[usize::from(f.session) - 1];
        for a in &f.answers {
            if let Some(t) = trials.iter().find(|t| t.id == a.id) {
                rat[si][t.pair] = Some(f64::from(a.rating));
            }
        }
    }
    let mean_rating = |pair: usize| mean(&rat.iter().filter_map(|s| s[pair]).collect::<Vec<_>>());
    let tp: Vec<usize> = (0..np)
        .filter(|&i| m.pairs[i].kind == PairKind::Test && mean_rating(i).is_some())
        .collect();
    let rating: Vec<f64> = tp
        .iter()
        .map(|&i| mean_rating(i).unwrap_or(f64::NAN))
        .collect();
    let boots = resamples(tp.len(), reps, seed);

    // test-retest over the pairs rated in both sessions
    let retest = (files.len() == 2).then(|| {
        let both: Vec<usize> = tp
            .iter()
            .copied()
            .filter(|&i| rat[0][i].is_some() && rat[1][i].is_some())
            .collect();
        let r1: Vec<f64> = both
            .iter()
            .map(|&i| rat[0][i].unwrap_or(f64::NAN))
            .collect();
        let r2: Vec<f64> = both
            .iter()
            .map(|&i| rat[1][i].unwrap_or(f64::NAN))
            .collect();
        let b = resamples(both.len(), reps, seed ^ 0x5eed);
        let rho: Vec<f64> = b
            .iter()
            .map(|ix| spearman(&pick(&r1, ix), &pick(&r2, ix)))
            .collect();
        let tau: Vec<f64> = b
            .iter()
            .map(|ix| kendall_tau_b(&pick(&r1, ix), &pick(&r2, ix)))
            .collect();
        Retest {
            n: both.len(),
            spearman: ci_of(spearman(&r1, &r2), &rho),
            tau_b: ci_of(kendall_tau_b(&r1, &r2), &tau),
            exact_agreement: r1.iter().zip(&r2).filter(|(a, b)| a == b).count() as f64
                / both.len().max(1) as f64,
            mean_abs_diff: mean(
                &r1.iter()
                    .zip(&r2)
                    .map(|(a, b)| (a - b).abs())
                    .collect::<Vec<_>>(),
            )
            .unwrap_or(f64::NAN),
        }
    });

    let anchor_mean = |kind: PairKind| {
        mean(
            &(0..np)
                .filter(|&i| m.pairs[i].kind == kind)
                .filter_map(mean_rating)
                .collect::<Vec<_>>(),
        )
    };
    let (same, diff) = (
        anchor_mean(PairKind::AnchorSame),
        anchor_mean(PairKind::AnchorDifferent),
    );
    let anchors = Anchors {
        same_mean_rating: same,
        different_mean_rating: diff,
        ok: same.is_some_and(|v| v >= 4.5) && diff.is_some_and(|v| v <= 1.5),
    };

    let n_bins = m
        .pairs
        .iter()
        .filter_map(|p| p.bin)
        .max()
        .map_or(0, |b| b + 1);
    let near: Vec<usize> = (0..tp.len())
        .filter(|&k| m.pairs[tp[k]].bin.is_some_and(|b| b < n_bins.div_ceil(2)))
        .collect();
    let mut tau_samples: Vec<(Dist, Vec<f64>)> = Vec::new();
    let mut metrics = Vec::new();
    for kind in Dist::ALL {
        let sim: Vec<f64> = tp
            .iter()
            .map(|&i| -m.pairs[i].components.distance(kind, &m.scale))
            .collect();
        let tau_b = kendall_tau_b(&sim, &rating);
        let taus: Vec<f64> = boots
            .iter()
            .map(|ix| kendall_tau_b(&pick(&sim, ix), &pick(&rating, ix)))
            .collect();
        let rhos: Vec<f64> = boots
            .iter()
            .map(|ix| spearman(&pick(&sim, ix), &pick(&rating, ix)))
            .collect();
        let trip: Vec<f64> = boots
            .iter()
            .map(|ix| triplet_agreement(&pick(&sim, ix), &pick(&rating, ix)).unwrap_or(f64::NAN))
            .collect();
        let rho_ci = ci_of(spearman(&sim, &rating), &rhos);
        let tau_ci = ci_of(tau_b, &taus);
        let near_trip = triplet_agreement(&pick(&sim, &near), &pick(&rating, &near));
        let gate_pass = rho_ci.estimate >= 0.7
            && rho_ci.lo >= 0.4
            && retest
                .as_ref()
                .is_none_or(|r| tau_ci.estimate >= r.tau_b.estimate - 0.15);
        metrics.push(MetricReport {
            distance: kind,
            spearman: rho_ci,
            tau_b: tau_ci,
            triplet_agreement: ci_of(triplet_agreement(&sim, &rating).unwrap_or(f64::NAN), &trip),
            triplet_agreement_near: near_trip,
            gate_pass,
        });
        tau_samples.push((kind, taus));
    }
    let mut tau_differences = Vec::new();
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let d: Vec<f64> = tau_samples[i]
            .1
            .iter()
            .zip(&tau_samples[j].1)
            .map(|(a, b)| a - b)
            .collect();
        tau_differences.push(TauDiff {
            first: tau_samples[i].0,
            second: tau_samples[j].0,
            diff: ci_of(metrics[i].tau_b.estimate - metrics[j].tau_b.estimate, &d),
        });
    }
    let best = metrics
        .iter()
        .max_by(|a, b| a.tau_b.estimate.total_cmp(&b.tau_b.estimate))
        .map(|b| b.distance);
    let recommendation = match best {
        None => "no ratings".to_string(),
        Some(Dist::A) => "A has the highest tau-b; keep A as provisional metric v1".to_string(),
        Some(b) => {
            let d = tau_differences
                .iter()
                .find(|t| t.first == Dist::A && t.second == b)
                .map(|t| t.diff);
            let separated = d.is_some_and(|c| c.hi < 0.0 || c.lo > 0.0);
            if separated {
                format!(
                    "{b:?} beats A with a paired tau-b difference whose 95 % CI excludes 0; consider {b:?} for metric v1"
                )
            } else {
                format!(
                    "{b:?} has the highest tau-b but the paired difference to A includes 0 (n = {}); prefer the simpler A",
                    tp.len()
                )
            }
        }
    };
    Report {
        schema: SCHEMA_REPORT.to_string(),
        synthetic: files.iter().any(|f| f.synthetic),
        sessions: files.len(),
        n_test_pairs: tp.len(),
        bootstrap_reps: reps,
        retest,
        anchors,
        metrics,
        tau_differences,
        recommendation,
    }
}

impl Report {
    /// A fixed-width table for the terminal.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let f = |c: &Ci| format!("{:+.2} [{:+.2}, {:+.2}]", c.estimate, c.lo, c.hi);
        let _ = writeln!(
            s,
            "listening analysis: {} test pairs, {} session(s), {} bootstrap resamples{}",
            self.n_test_pairs,
            self.sessions,
            self.bootstrap_reps,
            if self.synthetic {
                "  ** SYNTHETIC ANSWERS (dry run), not the owner's ratings **"
            } else {
                ""
            }
        );
        if let Some(r) = &self.retest {
            let _ = writeln!(
                s,
                "test-retest (n = {}): Spearman {}  tau-b {}  exact agreement {:.0} %  mean |diff| {:.2}",
                r.n,
                f(&r.spearman),
                f(&r.tau_b),
                100.0 * r.exact_agreement,
                r.mean_abs_diff
            );
        }
        let _ = writeln!(
            s,
            "anchors: identical pairs mean {:?}, obviously different mean {:?} -> {}",
            self.anchors
                .same_mean_rating
                .map(|v| (v * 100.0).round() / 100.0),
            self.anchors
                .different_mean_rating
                .map(|v| (v * 100.0).round() / 100.0),
            if self.anchors.ok {
                "ok"
            } else {
                "CHECK (identical should be >= 4.5, different <= 1.5)"
            }
        );
        let _ = writeln!(
            s,
            "{:<4} {:<26} {:<26} {:<26} {:<8} gate",
            "dist",
            "Spearman rho [95 % CI]",
            "Kendall tau-b [95 % CI]",
            "triplets agree [95 % CI]",
            "near"
        );
        for m in &self.metrics {
            let _ = writeln!(
                s,
                "{:<4} {:<26} {:<26} {:<26} {:<8} {}",
                format!("{:?}", m.distance),
                f(&m.spearman),
                f(&m.tau_b),
                format!(
                    "{:.2} [{:.2}, {:.2}]",
                    m.triplet_agreement.estimate, m.triplet_agreement.lo, m.triplet_agreement.hi
                ),
                m.triplet_agreement_near
                    .map_or("-".to_string(), |v| format!("{v:.2}")),
                if m.gate_pass { "PASS" } else { "fail" }
            );
        }
        for t in &self.tau_differences {
            let _ = writeln!(
                s,
                "tau-b difference {:?} - {:?}: {}",
                t.first,
                t.second,
                f(&t.diff)
            );
        }
        let _ = writeln!(
            s,
            "gate: rho >= 0.7, rho lower CI >= 0.4, tau-b within 0.15 of the test-retest tau-b"
        );
        let _ = writeln!(s, "recommendation: {}", self.recommendation);
        s
    }
}
