//! `enth-listen pairs report`: agreement per distance bin, the checks, the retest, and the data a `sound/2` fit needs.
//! Pure: the command reads and writes the files.
//!
//! | Column | Meaning |
//! | --- | --- |
//! | ear same | test pairs whose mean rating is at least 4.5: the ear's pass (the curve puts rating 4.5 on the pass mark) |
//! | sound/1 pass | test pairs `sound/1` passes (at least 90 %) |
//! | agree | pairs where the two verdicts agree |
//! | ABX correct | trials where X was named right (two answers per pair after two sessions); 50 % is chance, and 75 % is the usual threshold of "just noticeable", so it should cross 75 % near 1 JND if `sound/1`'s JND scale is right |

use serde::{Deserialize, Serialize};
use spike_metric::stats::{kendall_tau_b, percentile_ci, pick, resamples, spearman};

use enth_challenge::calibrate::Curve;

use super::fit::CurveFit;
use super::{
    BIN_LOWER, Manifest, MoveKind, PairRole, ResultsFile, SCHEMA_FIT, SCHEMA_REPORT, bin_label,
};
use crate::stats::{Z95, wilson};

/// The ear's pass: a mean rating at or above this.
pub const EAR_PASS_RATING: f64 = 4.5;

/// An estimate with a 95 % interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ci {
    /// The estimate.
    pub estimate: f64,
    /// Lower bound.
    pub lo: f64,
    /// Upper bound.
    pub hi: f64,
}

/// One distance bin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BinRow {
    /// Bin index.
    pub bin: usize,
    /// Its range in JND (`0.5-1`).
    pub label: String,
    /// Rated test pairs in it.
    pub pairs: usize,
    /// Their median `sound/1` JND distance.
    pub median_jnd: Option<f64>,
    /// Their mean rating.
    pub mean_rating: Option<f64>,
    /// Pairs with a mean rating of at least 4.5.
    pub ear_same: usize,
    /// Pairs `sound/1` passes.
    pub sound_pass: usize,
    /// Pairs where both verdicts agree.
    pub agree: usize,
    /// ABX trials answered right.
    pub abx_correct: usize,
    /// ABX trials answered.
    pub abx_trials: usize,
    /// Wilson 95 % interval of the ABX share.
    pub abx_lo: f64,
    /// Upper bound.
    pub abx_hi: f64,
    /// Pairs per move: all, one, pitch.
    pub moves: [usize; 3],
}

/// The pass verdicts of the ear and of `sound/1`, over the rated test pairs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agreement {
    /// Both pass.
    pub both: usize,
    /// Only the ear passes (`sound/1` is stricter).
    pub ear_only: usize,
    /// Only `sound/1` passes (it is looser).
    pub sound_only: usize,
    /// Neither.
    pub neither: usize,
}

impl Agreement {
    /// The share that agrees.
    #[must_use]
    pub fn share(&self) -> f64 {
        let n = self.both + self.ear_only + self.sound_only + self.neither;
        (self.both + self.neither) as f64 / n.max(1) as f64
    }
}

/// The checks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchors {
    /// Mean rating of the identical pairs.
    pub same_mean: Option<f64>,
    /// Mean rating of the obviously different pairs.
    pub different_mean: Option<f64>,
    /// Obviously different pairs: ABX trials right, of trials.
    pub different_abx: (usize, usize),
    /// Identical pairs rated 4.5 or more and obviously different ones 1.5 or less (spike-metric's rule).
    pub ok: bool,
}

/// Session 1 against session 2.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Retest {
    /// Test pairs rated in both.
    pub n: usize,
    /// Spearman's rho of the two sessions' ratings.
    pub spearman: Ci,
    /// Kendall's tau-b.
    pub tau_b: Ci,
    /// Share rated exactly alike.
    pub exact: f64,
    /// Mean absolute difference.
    pub mean_abs_diff: f64,
}

/// `sound/1`'s curve, for comparison.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveRef {
    /// The 50 % point.
    pub z50: f64,
    /// The steepness.
    pub k: f64,
    /// The pass point (90 %).
    pub z_pass: f64,
}

impl CurveRef {
    /// `sound/1`'s.
    #[must_use]
    pub fn sound_v1() -> Self {
        let c = Curve::SOUND_V1;
        Self {
            z50: c.z50,
            k: c.k,
            z_pass: c.z_at(90.0),
        }
    }
}

/// The report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// [`SCHEMA_REPORT`].
    pub schema: String,
    /// True when any results file is synthetic.
    pub synthetic: bool,
    /// The sessions read, with answers / trials.
    pub sessions: Vec<(u32, usize, usize)>,
    /// The engine that rendered the clips.
    pub engine: String,
    /// The plan.
    pub plan_hash: String,
    /// Test pairs with at least one rating.
    pub test_pairs: usize,
    /// One row per bin.
    pub bins: Vec<BinRow>,
    /// The pass verdicts.
    pub agreement: Agreement,
    /// Spearman's rho of the mean rating against `−jnd` (positive = the metric orders the pairs as the ear does).
    pub spearman: Option<Ci>,
    /// Kendall's tau-b of the same.
    pub tau_b: Option<Ci>,
    /// The checks.
    pub anchors: Anchors,
    /// Session 1 against session 2, when both are played.
    pub retest: Option<Retest>,
    /// The proposed curve.
    pub fit: Option<CurveFit>,
    /// `sound/1`'s curve.
    pub sound_v1: CurveRef,
    /// One line on what the fit says about `sound/1`'s pass mark.
    pub reading: String,
}

/// One row of the fit data: a pair, what `sound/1` measured, and the ratings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitRow {
    /// The pair's index.
    pub pair: usize,
    /// Test, same or different (a fit uses the test rows).
    pub role: PairRole,
    /// The target's level.
    pub level: u8,
    /// The move.
    pub kind: Option<MoveKind>,
    /// The parameter a `one` or `pitch` move changed.
    pub param: Option<String>,
    /// The target's short code.
    pub code: Option<String>,
    /// The step count of the move.
    pub steps: Option<f64>,
    /// Metric v1.1 distance.
    pub distance: f64,
    /// The target's nuisance floor.
    pub floor: f64,
    /// The target's JND scale.
    pub scale: f64,
    /// `max(0, distance − floor) / scale`.
    pub z_metric: f64,
    /// |Δ pitch|, cents.
    pub pitch_delta_ct: Option<f64>,
    /// The target's pitch floor, cents.
    pub pitch_floor_ct: f64,
    /// |Δ odd share|.
    pub odd_delta: Option<f64>,
    /// The target's odd-share floor.
    pub odd_floor: f64,
    /// The pitch guard's JND.
    pub pitch_term: f64,
    /// The waveform guard's JND.
    pub wave_term: f64,
    /// `sound/1`'s JND distance (`z_metric + pitch_term + wave_term`).
    pub jnd: f64,
    /// `sound/1`'s sound %.
    pub percent: f64,
    /// The rating in each session read (in the order of `sessions`), `None` where unanswered.
    pub ratings: Vec<Option<u8>>,
    /// The mean of the ratings given.
    pub mean_rating: Option<f64>,
    /// Whether X was named right in each session (`None` for identical pairs and unanswered trials).
    pub x_correct: Vec<Option<bool>>,
}

/// What a `sound/2` fit reads (`pairs-fit.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitData {
    /// [`SCHEMA_FIT`].
    pub schema: String,
    /// True when any results file is synthetic: never fit on it.
    pub synthetic: bool,
    /// The engine that rendered the clips.
    pub engine: String,
    /// The plan.
    pub plan_hash: String,
    /// The scorer measured.
    pub scorer: String,
    /// The sessions the ratings come from.
    pub sessions: Vec<u32>,
    /// The rating model the proposed curve is fitted with.
    pub model: String,
    /// `sound/1`'s pitch guard unit (cents per JND).
    pub guard_pitch_ct: f64,
    /// `sound/1`'s waveform guard unit (odd share per JND).
    pub guard_wave: f64,
    /// `sound/1`'s curve.
    pub sound_v1: CurveRef,
    /// The proposed curve (from the test rows).
    pub fit: Option<CurveFit>,
    /// One row per pair.
    pub rows: Vec<FitRow>,
}

/// Per pair, per file: the rating and whether X was right.
fn collect(m: &Manifest, files: &[ResultsFile]) -> Vec<Vec<(Option<u8>, Option<bool>)>> {
    let mut out = vec![vec![(None, None); files.len()]; m.pairs.len()];
    for (fi, f) in files.iter().enumerate() {
        for a in &f.answers {
            let Some(t) = m
                .trials
                .iter()
                .find(|t| t.session == f.session && t.id == a.id)
            else {
                continue;
            };
            if let Some(slot) = out.get_mut(t.pair) {
                slot[fi] = (Some(a.rating), a.x_correct);
            }
        }
    }
    out
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn median(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    Some(if n % 2 == 1 {
        s[n / 2]
    } else {
        f64::midpoint(s[n / 2 - 1], s[n / 2])
    })
}

fn ci_of(estimate: f64, samples: &[f64]) -> Ci {
    let (lo, hi) = percentile_ci(samples);
    Ci { estimate, lo, hi }
}

/// The fit data: one row per pair (`reps` bootstrap resamples for the proposed curve, from `seed`).
#[must_use]
pub fn fit_rows(m: &Manifest, files: &[ResultsFile], reps: usize, seed: u64) -> FitData {
    let got = collect(m, files);
    let rows: Vec<FitRow> = m
        .pairs
        .iter()
        .map(|p| {
            let g = &got[p.plan.index];
            let ratings: Vec<Option<u8>> = g.iter().map(|(r, _)| *r).collect();
            let given: Vec<f64> = ratings.iter().flatten().map(|r| f64::from(*r)).collect();
            let s = &p.measured;
            FitRow {
                pair: p.plan.index,
                role: p.plan.role,
                level: p.plan.level,
                kind: p.kind,
                param: p.moved_param().map(str::to_string),
                code: p.code.clone(),
                steps: p.steps,
                distance: s.distance,
                floor: s.floor,
                scale: s.scale,
                z_metric: s.z_metric,
                pitch_delta_ct: s.pitch_delta_ct,
                pitch_floor_ct: s.pitch_floor_ct,
                odd_delta: s.odd_delta,
                odd_floor: s.odd_floor,
                pitch_term: s.pitch_term,
                wave_term: s.wave_term,
                jnd: s.jnd,
                percent: s.percent,
                ratings,
                mean_rating: mean(&given),
                x_correct: g.iter().map(|(_, x)| *x).collect(),
            }
        })
        .collect();
    let (z, r): (Vec<f64>, Vec<f64>) = rows
        .iter()
        .filter(|row| row.role == PairRole::Test)
        .filter_map(|row| Some((row.jnd, row.mean_rating?)))
        .unzip();
    FitData {
        schema: SCHEMA_FIT.to_string(),
        synthetic: files.iter().any(|f| f.synthetic),
        engine: m.engine.clone(),
        plan_hash: m.plan_hash.clone(),
        scorer: m.scorer.clone(),
        sessions: files.iter().map(|f| f.session).collect(),
        model: "rating = 1 + 4 / (1 + (jnd / c)^q); z50 = c, k = q ln 9 / ln 7, pass at c 7^(-1/q)"
            .to_string(),
        guard_pitch_ct: enth_challenge::score::sound::GUARD_PITCH_CT,
        guard_wave: enth_challenge::score::sound::GUARD_WAVE,
        sound_v1: CurveRef::sound_v1(),
        fit: CurveFit::new(&z, &r, reps, seed),
        rows,
    }
}

fn reading(fit: Option<&CurveFit>, v1: &CurveRef) -> String {
    let Some(f) = fit else {
        return "not enough rated test pairs for a fit (at least 3)".to_string();
    };
    if f.fit.at_bound {
        return format!(
            "the fit sits on the edge of its range (c = {:.2}, q = {:.2}): the ratings do not pin the curve; check the anchors and the spread of ratings",
            f.fit.c, f.fit.q
        );
    }
    let ci = f.z_pass_ci;
    let verdict = if ci.lo > v1.z_pass {
        "the ear tolerates more than sound/1's pass mark: sound/1 is stricter than the ear"
    } else if ci.hi < v1.z_pass {
        "the ear hears less than sound/1's pass mark allows: sound/1 is looser than the ear"
    } else {
        "consistent with sound/1's pass mark"
    };
    format!(
        "rating 4.5 at {:.2} JND [{:.2}, {:.2}] against sound/1's {:.2}: {verdict}",
        f.z_pass, ci.lo, ci.hi, v1.z_pass
    )
}

/// The report over the results files read (one or both sessions).
#[must_use]
pub fn analyze(m: &Manifest, files: &[ResultsFile], reps: usize, seed: u64) -> Report {
    let got = collect(m, files);
    let data = fit_rows(m, files, reps, seed);
    let test: Vec<&FitRow> = data
        .rows
        .iter()
        .filter(|r| r.role == PairRole::Test && r.mean_rating.is_some())
        .collect();
    let ear = |r: &FitRow| r.mean_rating.is_some_and(|v| v >= EAR_PASS_RATING);
    let pass = |r: &FitRow| r.percent >= m.pass_mark;
    let mut bins = Vec::new();
    for b in 0..BIN_LOWER.len() {
        let rows: Vec<&&FitRow> = test.iter().filter(|r| m.pairs[r.pair].bin == b).collect();
        let jnds: Vec<f64> = rows.iter().map(|r| r.jnd).collect();
        let ratings: Vec<f64> = rows.iter().filter_map(|r| r.mean_rating).collect();
        let xs: Vec<bool> = rows
            .iter()
            .flat_map(|r| r.x_correct.iter().flatten().copied())
            .collect();
        let k = xs.iter().filter(|x| **x).count();
        let (lo, hi) = wilson(k, xs.len(), Z95);
        let mut moves = [0usize; 3];
        for r in &rows {
            match r.kind {
                Some(MoveKind::All) => moves[0] += 1,
                Some(MoveKind::One) => moves[1] += 1,
                Some(MoveKind::Pitch) => moves[2] += 1,
                None => {}
            }
        }
        bins.push(BinRow {
            bin: b,
            label: bin_label(b),
            pairs: rows.len(),
            median_jnd: median(&jnds),
            mean_rating: mean(&ratings),
            ear_same: rows.iter().filter(|r| ear(r)).count(),
            sound_pass: rows.iter().filter(|r| pass(r)).count(),
            agree: rows.iter().filter(|r| ear(r) == pass(r)).count(),
            abx_correct: k,
            abx_trials: xs.len(),
            abx_lo: lo,
            abx_hi: hi,
            moves,
        });
    }
    let mut agreement = Agreement {
        both: 0,
        ear_only: 0,
        sound_only: 0,
        neither: 0,
    };
    for r in &test {
        match (ear(r), pass(r)) {
            (true, true) => agreement.both += 1,
            (true, false) => agreement.ear_only += 1,
            (false, true) => agreement.sound_only += 1,
            (false, false) => agreement.neither += 1,
        }
    }
    let sim: Vec<f64> = test.iter().map(|r| -r.jnd).collect();
    let rat: Vec<f64> = test.iter().filter_map(|r| r.mean_rating).collect();
    let (rho, tau) = if test.len() >= 3 {
        let boots = resamples(test.len(), reps, seed ^ 0x5eed);
        let rhos: Vec<f64> = boots
            .iter()
            .map(|ix| spearman(&pick(&sim, ix), &pick(&rat, ix)))
            .filter(|v| v.is_finite())
            .collect();
        let taus: Vec<f64> = boots
            .iter()
            .map(|ix| kendall_tau_b(&pick(&sim, ix), &pick(&rat, ix)))
            .filter(|v| v.is_finite())
            .collect();
        (
            Some(ci_of(spearman(&sim, &rat), &rhos)),
            Some(ci_of(kendall_tau_b(&sim, &rat), &taus)),
        )
    } else {
        (None, None)
    };
    let role_ratings = |role: PairRole| -> Vec<f64> {
        m.pairs
            .iter()
            .filter(|p| p.plan.role == role)
            .flat_map(|p| {
                got[p.plan.index]
                    .iter()
                    .filter_map(|(r, _)| r.map(f64::from))
            })
            .collect()
    };
    let (same, diff) = (
        mean(&role_ratings(PairRole::Same)),
        mean(&role_ratings(PairRole::Different)),
    );
    let diff_x: Vec<bool> = m
        .pairs
        .iter()
        .filter(|p| p.plan.role == PairRole::Different)
        .flat_map(|p| got[p.plan.index].iter().filter_map(|(_, x)| *x))
        .collect();
    let anchors = Anchors {
        same_mean: same,
        different_mean: diff,
        different_abx: (diff_x.iter().filter(|x| **x).count(), diff_x.len()),
        ok: same.is_some_and(|v| v >= 4.5) && diff.is_some_and(|v| v <= 1.5),
    };
    let retest = (files.len() == 2).then(|| {
        let both: Vec<(f64, f64)> = data
            .rows
            .iter()
            .filter(|r| r.role == PairRole::Test)
            .filter_map(|r| Some((f64::from(r.ratings[0]?), f64::from(r.ratings[1]?))))
            .collect();
        let (r1, r2): (Vec<f64>, Vec<f64>) = both.iter().copied().unzip();
        let boots = resamples(both.len(), reps, seed ^ 0x7e57);
        let rhos: Vec<f64> = boots
            .iter()
            .map(|ix| spearman(&pick(&r1, ix), &pick(&r2, ix)))
            .filter(|v| v.is_finite())
            .collect();
        let taus: Vec<f64> = boots
            .iter()
            .map(|ix| kendall_tau_b(&pick(&r1, ix), &pick(&r2, ix)))
            .filter(|v| v.is_finite())
            .collect();
        Retest {
            n: both.len(),
            spearman: ci_of(spearman(&r1, &r2), &rhos),
            tau_b: ci_of(kendall_tau_b(&r1, &r2), &taus),
            exact: both.iter().filter(|(a, b)| a == b).count() as f64 / both.len().max(1) as f64,
            mean_abs_diff: mean(&both.iter().map(|(a, b)| (a - b).abs()).collect::<Vec<_>>())
                .unwrap_or(f64::NAN),
        }
    });
    let v1 = CurveRef::sound_v1();
    Report {
        schema: SCHEMA_REPORT.to_string(),
        synthetic: data.synthetic,
        sessions: files
            .iter()
            .map(|f| {
                (
                    f.session,
                    f.answers.len(),
                    m.session_trials(f.session).len(),
                )
            })
            .collect(),
        engine: m.engine.clone(),
        plan_hash: m.plan_hash.clone(),
        test_pairs: test.len(),
        bins,
        agreement,
        spearman: rho,
        tau_b: tau,
        anchors,
        retest,
        reading: reading(data.fit.as_ref(), &v1),
        fit: data.fit,
        sound_v1: v1,
    }
}

impl Report {
    /// A fixed-width table for the terminal.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let f = |c: &Ci| format!("{:+.2} [{:+.2}, {:+.2}]", c.estimate, c.lo, c.hi);
        let opt = |v: Option<f64>, d: usize| v.map_or("-".to_string(), |v| format!("{v:.d$}"));
        let sessions: Vec<String> = self
            .sessions
            .iter()
            .map(|(n, a, t)| format!("session {n} {a}/{t}"))
            .collect();
        let _ = writeln!(
            s,
            "pairs report: {} rated test pairs; {}{}",
            self.test_pairs,
            sessions.join(", "),
            if self.synthetic {
                "  ** SYNTHETIC ANSWERS (dry run), not the owner's ratings **"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "engine {}, plan {}; sound/1 passes at {:.2} JND, 50 % at {:.2} JND",
            self.engine, self.plan_hash, self.sound_v1.z_pass, self.sound_v1.z50
        );
        let _ = writeln!(
            s,
            "checks: identical pairs mean {}, obviously different mean {} (ABX {}/{}) -> {}",
            opt(self.anchors.same_mean, 2),
            opt(self.anchors.different_mean, 2),
            self.anchors.different_abx.0,
            self.anchors.different_abx.1,
            if self.anchors.ok {
                "ok"
            } else {
                "CHECK (identical should be >= 4.5, different <= 1.5)"
            }
        );
        if let Some(r) = &self.retest {
            let _ = writeln!(
                s,
                "test-retest (n = {}): Spearman {}  tau-b {}  exact {:.0} %  mean |diff| {:.2}",
                r.n,
                f(&r.spearman),
                f(&r.tau_b),
                100.0 * r.exact,
                r.mean_abs_diff
            );
        }
        let _ = writeln!(
            s,
            "{:<9} {:>5} {:>7} {:>7} {:>9} {:>9} {:>7}  {:<22} moves a/o/p",
            "bin (JND)",
            "pairs",
            "median",
            "rating",
            "ear same",
            "s/1 pass",
            "agree",
            "ABX correct [95 % CI]"
        );
        for b in &self.bins {
            let abx = if b.abx_trials == 0 {
                "-".to_string()
            } else {
                format!(
                    "{}/{} {:.0} % [{:.0}, {:.0}]",
                    b.abx_correct,
                    b.abx_trials,
                    100.0 * b.abx_correct as f64 / b.abx_trials as f64,
                    100.0 * b.abx_lo,
                    100.0 * b.abx_hi
                )
            };
            let _ = writeln!(
                s,
                "{:<9} {:>5} {:>7} {:>7} {:>9} {:>9} {:>7}  {:<22} {}/{}/{}",
                b.label,
                b.pairs,
                opt(b.median_jnd, 2),
                opt(b.mean_rating, 2),
                format!("{}/{}", b.ear_same, b.pairs),
                format!("{}/{}", b.sound_pass, b.pairs),
                format!("{}/{}", b.agree, b.pairs),
                abx,
                b.moves[0],
                b.moves[1],
                b.moves[2]
            );
        }
        let a = &self.agreement;
        let _ = writeln!(
            s,
            "pass agreement (ear: mean rating >= 4.5; sound/1: >= 90 %): both {}, ear only {}, sound/1 only {}, neither {} -> {:.0} %",
            a.both,
            a.ear_only,
            a.sound_only,
            a.neither,
            100.0 * a.share()
        );
        if let (Some(rho), Some(tau)) = (&self.spearman, &self.tau_b) {
            let _ = writeln!(
                s,
                "rating against sound/1's distance: Spearman {}  tau-b {}",
                f(rho),
                f(tau)
            );
        }
        if let Some(c) = &self.fit {
            let _ = writeln!(
                s,
                "fit (rating = 1 + 4 / (1 + (jnd / c)^q), n = {}, rmse {:.2}): z50 {:.2} [{:.2}, {:.2}], k {:.2} [{:.2}, {:.2}], pass (rating 4.5) {:.2} [{:.2}, {:.2}]",
                c.fit.n,
                c.rmse,
                c.z50,
                c.z50_ci.lo,
                c.z50_ci.hi,
                c.k,
                c.k_ci.lo,
                c.k_ci.hi,
                c.z_pass,
                c.z_pass_ci.lo,
                c.z_pass_ci.hi
            );
            let _ = writeln!(
                s,
                "sound/1's curve: z50 {:.2}, k {:.2}, pass {:.2} (bridged from the toy synth)",
                self.sound_v1.z50, self.sound_v1.k, self.sound_v1.z_pass
            );
        }
        let _ = writeln!(s, "reading: {}", self.reading);
        s
    }
}
