//! **Owner: lane EVK.** Pre-declared by lane S0s (R3).
//!
//! `cargo xtask gr3-report`: the endpoints fixed before the first test run, from `content/gr3/runs.jsonl`.
//!
//! ```text
//! cargo xtask gr3-report                          # list the runs in the file
//! cargo xtask gr3-report --run RUN                # one arm: pass rate, JND, anytime curve, time to pass, strata
//! cargo xtask gr3-report --run RUN --rate-from P  # a render run read at the renders a measured idle rate gives
//! cargo xtask gr3-report --a RUN_A --b RUN_B      # clause 1: B against A, paired by target, and the decision
//!            [--at 60] [--long-a RUN --long-b RUN] [--long-at 300] [--runs PATH] [--json]
//! ```
//!
//! **Fixed in advance** (docs/gr3.md, "The decision"; changed only before the first arm-B test run):
//!
//! - **Primary, at 60 s** (`--at`): the pass rate (`sound/1` ≥ 90 against the target patch) by the exact `McNemar` test
//!   on the discordant targets, and the per-target difference in JND units, `d = jnd_A − jnd_B` (positive: B is
//!   closer), by the exact Wilcoxon signed-rank test with the Hodges–Lehmann estimate and 95 % interval. A target with
//!   no score at the mark fails the pass rate and is left out of the JND test.
//! - **"Better at 60 s"** means both one-sided tests favour B at p < 0.05 (an intersection–union test: no
//!   multiplicity correction is needed because both must hold). **"Not worse at 300 s"** (`--long-at`, from the
//!   longer runs, or the same runs when they reach it) means neither one-sided test favours A at p < 0.05.
//!   **B beats A** when both hold.
//! - **Secondary, reported, never gated:** anytime curves (the median JND at 10, 30, 60 and 300 s), time to pass
//!   (Kaplan–Meier over the checkpoint grid, censored at the endpoint), `structure/1`, every stratum (`gen-1note`,
//!   `gen-2note`: the generator's two-note phrases, which `create` plays as one note in every arm; `preset`) and
//!   level, and the Goodhart guard: the independent metric no arm optimises (MFCC mean squared error), by the same
//!   paired Wilcoxon test.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::runs::{self, Runs, TargetLine};
use super::stats::{self, KmStep};

/// The primary mark (s).
pub const PRIMARY_AT: f64 = 60.0;
/// The long mark (s).
pub const LONG_AT: f64 = 300.0;
/// The one-sided level of every decision.
pub const ALPHA: f64 = 0.05;
/// The anytime marks the report reads when a run has them (s).
pub const ANYTIME: [f64; 4] = [10.0, 30.0, 60.0, 300.0];
/// The anytime marks of a render-capped run (renders).
pub const ANYTIME_RENDERS: [f64; 5] = [500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// The anytime marks for a run's mode, and the unit its marks are in.
pub fn anytime_marks(mode: &str) -> (&'static [f64], &'static str) {
    if mode == "wall" {
        (&ANYTIME, "s")
    } else {
        (&ANYTIME_RENDERS, "renders")
    }
}

const USAGE: &str =
    "usage: cargo xtask gr3-report [--runs content/gr3/runs.jsonl] [--run RUN | --a RUN --b RUN]
       [--at 60] [--long-a RUN --long-b RUN] [--long-at 300]
       [--rate-from PROBE_RUN [--probe-runs PATH]] [--json]
With no run named, it lists the runs in the file.";

/// What one target scored at one mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Passed (`sound/1` ≥ 90).
    pub passed: bool,
    /// JND units (`None`: not scored, or no candidate yet).
    pub jnd: Option<f64>,
    /// The independent metric.
    pub mfcc: Option<f64>,
    /// `structure/1` %.
    pub structure: Option<f64>,
    /// Renders then.
    pub renders: usize,
}

/// A target at `mark`: `None` when the run never reached the mark.
pub fn point(t: &TargetLine, mark: f64) -> Option<Point> {
    point_of(t.at(mark))
}

/// What a checkpoint scored.
pub fn point_of(c: Option<&runs::CheckpointRec>) -> Option<Point> {
    let c = c?;
    Some(Point {
        passed: c.score.is_some_and(|s| s.passed),
        jnd: c.score.map(|s| s.jnd).filter(|x| x.is_finite()),
        mfcc: c.score.and_then(|s| s.mfcc).filter(|x| x.is_finite()),
        structure: c.score.and_then(|s| s.structure),
        renders: c.renders,
    })
}

/// One arm's summary over some targets.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Targets.
    pub n: usize,
    /// Passed at the mark.
    pub passed: usize,
    /// The pass rate's 95 % Wilson interval.
    pub wilson: (f64, f64),
    /// Median JND at the mark (targets with no score count as +∞).
    pub median_jnd: f64,
    /// Median JND at each anytime mark the run has.
    pub anytime: Vec<(f64, f64)>,
    /// Kaplan–Meier median time to pass (s), if the curve reaches ½.
    pub km_median: Option<f64>,
    /// The curve.
    pub km: Vec<KmStep>,
    /// Median MFCC MSE at the mark.
    pub median_mfcc: f64,
    /// Median `structure/1` % at the mark.
    pub median_structure: f64,
    /// Median renders at the mark.
    pub median_renders: f64,
}

/// Summarise `lines` at `mark`, with the anytime curve at `anytime` (the marks every line has).
pub fn summarize(lines: &[&TargetLine], mark: f64, anytime: &[f64]) -> Summary {
    summarize_by(lines, &|t| t.at(mark), anytime)
}

/// Summarise `lines` at the checkpoint `pick` chooses for each (a per-target endpoint: `--rate-from`).
pub fn summarize_by(
    lines: &[&TargetLine],
    pick: &dyn Fn(&TargetLine) -> Option<&runs::CheckpointRec>,
    anytime: &[f64],
) -> Summary {
    let pts: Vec<Point> = lines.iter().filter_map(|t| point_of(pick(t))).collect();
    let n = lines.len();
    let passed = pts.iter().filter(|p| p.passed).count();
    let jnd_or_inf = |p: Option<Point>| p.and_then(|p| p.jnd).unwrap_or(f64::INFINITY);
    let anytime = anytime
        .iter()
        .filter(|m| lines.iter().all(|t| t.at(**m).is_some()) && !lines.is_empty())
        .map(|m| {
            let v: Vec<f64> = lines.iter().map(|t| jnd_or_inf(point(t, *m))).collect();
            (*m, stats::median(&v))
        })
        .collect();
    let km_data: Vec<(f64, bool)> = lines
        .iter()
        .map(|t| {
            // Censored at the endpoint the summary reads (a pass after it does not count).
            let until = pick(t).map_or(t.cap, |c| c.mark);
            match t.time_to_pass_by(until) {
                Some(tp) => (tp, true),
                None => (until, false),
            }
        })
        .collect();
    let km = stats::kaplan_meier(&km_data);
    let med = |f: &dyn Fn(&Point) -> Option<f64>| {
        let v: Vec<f64> = pts.iter().filter_map(f).collect();
        stats::median(&v)
    };
    Summary {
        n,
        passed,
        wilson: stats::wilson(passed, n),
        median_jnd: stats::median(
            &lines
                .iter()
                .map(|t| jnd_or_inf(point_of(pick(t))))
                .collect::<Vec<_>>(),
        ),
        anytime,
        km_median: stats::km_median(&km),
        km,
        median_mfcc: med(&|p| p.mfcc),
        median_structure: med(&|p| p.structure),
        median_renders: med(&|p| Some(p.renders as f64)),
    }
}

/// B against A at one mark, paired by target.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Targets in both runs.
    pub pairs: usize,
    /// Passed by A, by B.
    pub passed: (usize, usize),
    /// `McNemar`'s test.
    pub mcnemar: stats::McNemar,
    /// Wilcoxon on `jnd_A − jnd_B`.
    pub wilcoxon: stats::SignedRank,
    /// Hodges–Lehmann on the same.
    pub hl: Option<stats::HodgesLehmann>,
    /// Wilcoxon on `mfcc_A − mfcc_B` (the Goodhart guard).
    pub mfcc: stats::SignedRank,
}

/// Compare `b` with `a` at `mark`.
pub fn compare(a: &[&TargetLine], b: &[&TargetLine], mark: f64) -> Comparison {
    let bmap: BTreeMap<&str, &&TargetLine> = b.iter().map(|t| (t.target.id.as_str(), t)).collect();
    let (mut a_only, mut b_only, mut pa, mut pb, mut pairs) = (0, 0, 0, 0, 0);
    let (mut d, mut dm) = (Vec::new(), Vec::new());
    for ta in a {
        let Some(tb) = bmap.get(ta.target.id.as_str()) else {
            continue;
        };
        pairs += 1;
        let (x, y) = (point(ta, mark), point(tb, mark));
        let (xp, yp) = (x.is_some_and(|p| p.passed), y.is_some_and(|p| p.passed));
        pa += usize::from(xp);
        pb += usize::from(yp);
        match (xp, yp) {
            (true, false) => a_only += 1,
            (false, true) => b_only += 1,
            _ => {}
        }
        if let (Some(ja), Some(jb)) = (x.and_then(|p| p.jnd), y.and_then(|p| p.jnd)) {
            d.push(ja - jb);
        }
        if let (Some(ma), Some(mb)) = (x.and_then(|p| p.mfcc), y.and_then(|p| p.mfcc)) {
            dm.push(ma - mb);
        }
    }
    Comparison {
        pairs,
        passed: (pa, pb),
        mcnemar: stats::mcnemar(a_only, b_only),
        wilcoxon: stats::signed_rank(&d),
        hl: stats::hodges_lehmann(&d),
        mfcc: stats::signed_rank(&dm),
    }
}

/// The decision, fixed in advance (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Both one-sided tests favour B at the primary mark.
    pub better_at_primary: bool,
    /// Neither one-sided test favours A at the long mark (`None`: no long runs).
    pub not_worse_at_long: Option<bool>,
}

impl Decision {
    /// B beats A.
    pub fn beats(self) -> bool {
        self.better_at_primary && self.not_worse_at_long == Some(true)
    }
}

/// Decide from the primary comparison and, when there is one, the long one.
pub fn decide(primary: &Comparison, long: Option<&Comparison>) -> Decision {
    Decision {
        better_at_primary: primary.mcnemar.p_b_better < ALPHA && primary.wilcoxon.p_greater < ALPHA,
        not_worse_at_long: long
            .map(|c| c.mcnemar.p_a_better >= ALPHA && c.wilcoxon.p_less >= ALPHA),
    }
}

fn strata<'a>(lines: &[&'a TargetLine]) -> Vec<(String, Vec<&'a TargetLine>)> {
    let mut out: Vec<(String, Vec<&TargetLine>)> = vec![("all".to_owned(), lines.to_vec())];
    for s in ["gen-1note", "gen-2note", "preset"] {
        let v: Vec<&TargetLine> = lines
            .iter()
            .copied()
            .filter(|t| t.target.stratum == s)
            .collect();
        if !v.is_empty() {
            out.push((s.to_owned(), v));
        }
    }
    for l in 1..=5u8 {
        let v: Vec<&TargetLine> = lines
            .iter()
            .copied()
            .filter(|t| t.target.kind == "generator" && t.target.level == Some(l))
            .collect();
        if !v.is_empty() {
            out.push((format!("level {l}"), v));
        }
    }
    out
}

fn f(x: f64, digits: usize) -> String {
    if x.is_nan() {
        "–".to_owned()
    } else if x.is_infinite() {
        if x > 0.0 { "∞" } else { "−∞" }.to_owned()
    } else {
        format!("{x:.digits$}")
    }
}

fn opt(x: Option<f64>) -> String {
    x.map_or_else(|| "–".to_owned(), |v| f(v, 0))
}

fn mark_of(run: &runs::RunLine, at: Option<f64>) -> f64 {
    let cap = run
        .seconds
        .or(run.budget.map(|b| b as f64))
        .unwrap_or(PRIMARY_AT);
    match at {
        Some(a) => a,
        None if run.mode == "wall" => PRIMARY_AT.min(cap),
        // A render run that stands for a timed one: its endpoint is the renders it stands for.
        None => equivalence(run)
            .and_then(|e| e.get("renders").and_then(Value::as_f64))
            .unwrap_or(cap),
    }
}

/// The run header's render-equivalence flag (`gr3-run --equivalent-seconds`), if any.
fn equivalence(run: &runs::RunLine) -> Option<&Value> {
    run.raw
        .pointer("/header/equivalence")
        .filter(|v| !v.is_null())
}

/// The idle rate a probe run measured (`gr3-run --rate-probe`): the median over its targets of the renders at its
/// 60 s checkpoint, over 60 s; with how many targets were under the exclusive lock.
pub fn probe_rate(runs: &Runs, probe: &str) -> Result<(f64, usize, usize), String> {
    let lines = runs
        .targets
        .get(probe)
        .ok_or_else(|| format!("the probe run {probe} has no target lines"))?;
    let r: Vec<f64> = lines
        .iter()
        .filter_map(|t| t.at(PRIMARY_AT).map(|c| c.renders as f64 / PRIMARY_AT))
        .collect();
    if r.is_empty() {
        return Err(format!(
            "the probe run {probe} has no checkpoint at {PRIMARY_AT} s"
        ));
    }
    let locked = lines.iter().filter(|t| t.machine.locked).count();
    Ok((stats::median(&r), locked, lines.len()))
}

/// Each target's renders in the converted time: `renders` (the median rate's) scaled by the run's median CPU cost per
/// render over the target's own, at most the target's cap; and how many hit the cap.
fn per_target_renders(lines: &[&TargetLine], renders: f64) -> (BTreeMap<String, f64>, usize) {
    let costs: Vec<f64> = lines.iter().filter_map(|t| t.cpu_per_render()).collect();
    if costs.len() != lines.len() {
        return (BTreeMap::new(), 0);
    }
    let med = stats::median(&costs);
    let mut capped = 0;
    let out = lines
        .iter()
        .map(|t| {
            let want = renders * med / t.cpu_per_render().unwrap_or(med);
            if want > t.cap {
                capped += 1;
            }
            (t.target.id.clone(), want.min(t.cap))
        })
        .collect();
    (out, capped)
}

/// The largest mark at or below `renders` that every line has.
fn common_mark_at_most(lines: &[&TargetLine], renders: f64) -> Option<f64> {
    let first = lines.first()?;
    first
        .checkpoints
        .iter()
        .map(|c| c.mark)
        .filter(|m| *m <= renders && lines.iter().all(|t| t.at(*m).is_some()))
        .fold(None, |best: Option<f64>, m| {
            Some(best.map_or(m, |b| b.max(m)))
        })
}

fn run_text(runs: &Runs, id: &str) -> Result<String, String> {
    let r = runs
        .headers
        .iter()
        .find(|r| r.run_id == id)
        .ok_or_else(|| format!("no run {id}"))?;
    let h = &r.raw;
    let build = h
        .pointer("/header/build/commit")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let dirty = h
        .pointer("/header/build/dirty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(format!(
        "run {id}: arm {}, {} {}, {} workers, {} split, build {}{}, {}, cfg {}, bank {}",
        r.arm,
        r.mode,
        r.seconds.map_or_else(
            || format!("{} renders", r.budget.unwrap_or(0)),
            |s| format!("{s} s")
        ),
        r.workers,
        r.split,
        &build[..build.len().min(12)],
        if dirty { "+dirty" } else { "" },
        r.date,
        h.get("cfg").map_or_else(String::new, Value::to_string),
        h.pointer("/bank/entries")
            .and_then(Value::as_u64)
            .map_or_else(|| "none".to_owned(), |n| format!("{n} entries")),
    ))
}

/// The single-arm report.
pub fn single(
    runs: &Runs,
    id: &str,
    at: Option<f64>,
    rate_from: Option<(&Runs, &str)>,
) -> Result<(String, Value), String> {
    let run = runs
        .headers
        .iter()
        .find(|r| r.run_id == id)
        .ok_or_else(|| format!("no run {id}"))?;
    let lines: Vec<&TargetLine> = runs
        .targets
        .get(id)
        .map(|v| v.iter().collect())
        .unwrap_or_default();
    let mut mark = mark_of(run, at);
    let mut converted = Value::Null;
    let mut s = String::new();
    let _ = writeln!(s, "{}", run_text(runs, id)?);
    if let Some(e) = equivalence(run) {
        let _ = writeln!(
            s,
            "**Render-equivalent estimate:** {} renders stand for {} s at an assumed {} renders/s ({}); not a timed run.",
            e.get("renders").map_or_else(String::new, Value::to_string),
            e.get("seconds").map_or_else(String::new, Value::to_string),
            e.get("rate_renders_per_s")
                .and_then(Value::as_f64)
                .map_or_else(String::new, |r| f(r, 1)),
            e.get("source")
                .and_then(Value::as_str)
                .unwrap_or("no source given"),
        );
    }
    if let Some((probe_runs, probe)) = rate_from {
        if run.mode == "wall" {
            return Err("--rate-from converts a render-capped run; this one is timed".to_owned());
        }
        let (rate, locked, n) = probe_rate(probe_runs, probe)?;
        let secs = equivalence(run)
            .and_then(|e| e.get("seconds").and_then(Value::as_f64))
            .unwrap_or(PRIMARY_AT);
        let want = rate * secs;
        mark = common_mark_at_most(&lines, want).ok_or_else(|| {
            format!("no checkpoint at or below {want:.0} renders: run a twin with a larger budget")
        })?;
        converted =
            json!({"probe": probe, "rate": rate, "seconds": secs, "renders": want, "mark": mark});
        let (per, capped) = per_target_renders(&lines, rate * secs);
        let _ = writeln!(
            s,
            "**Converted with a measured idle rate:** probe {probe} ({locked} of {n} targets under the exclusive lock) \
             measured a median {} renders/s, so {secs} s is {want:.0} renders; the table reads the checkpoint {mark}.",
            f(rate, 1)
        );
        if !per.is_empty() {
            let pick: &dyn Fn(&TargetLine) -> Option<&runs::CheckpointRec> =
                &|t| per.get(t.target.id.as_str()).and_then(|m| t.at_most(*m));
            let sm = summarize_by(&lines, pick, &[]);
            let _ = writeln!(
                s,
                "Per target (each target's renders scaled by its CPU cost per render against the run's median, so a \
                 dry patch gets more renders in {secs} s than one with reverb): {} of {} pass, median JND {}; {capped} \
                 target(s) would need more renders than the run's budget and read at its cap (a longer twin removes that).",
                sm.passed,
                sm.n,
                f(sm.median_jnd, 2)
            );
            converted["per_target"] = json!({"passed": sm.passed, "n": sm.n, "median_jnd": sm.median_jnd, "capped": capped});
        }
    }
    let failed = runs.failed.get(id).map_or(0, Vec::len);
    let _ = writeln!(
        s,
        "{} targets recorded, {failed} failed{}; the endpoint is the checkpoint at {mark}{}",
        lines.len(),
        if runs.ended.contains_key(id) {
            ""
        } else {
            ", NO run_end line (the run stopped early)"
        },
        if run.mode == "wall" { " s" } else { " renders" },
    );
    let _ = writeln!(s);
    let (grid, unit) = anytime_marks(&run.mode);
    let any_marks: Vec<f64> = summarize(&lines, mark, grid)
        .anytime
        .iter()
        .map(|(m, _)| *m)
        .collect();
    let _ = write!(
        s,
        "| Stratum | n | Pass | Pass rate (95 % CI) | Median JND |"
    );
    for m in &any_marks {
        let _ = write!(s, " JND at {m} {unit} |");
    }
    let _ = writeln!(
        s,
        " KM median time to pass | Median MFCC | Median structure % | Median renders |"
    );
    let _ = writeln!(s, "|{}", " --- |".repeat(9 + any_marks.len()));
    let mut json_strata = Vec::new();
    for (name, v) in strata(&lines) {
        let sm = summarize(&v, mark, grid);
        let _ = write!(
            s,
            "| {name} | {} | {} | {} % ({}–{}) | {} |",
            sm.n,
            sm.passed,
            f(100.0 * sm.passed as f64 / sm.n.max(1) as f64, 1),
            f(100.0 * sm.wilson.0, 1),
            f(100.0 * sm.wilson.1, 1),
            f(sm.median_jnd, 2)
        );
        for m in &any_marks {
            let x = sm
                .anytime
                .iter()
                .find(|(a, _)| a == m)
                .map_or(f64::NAN, |(_, y)| *y);
            let _ = write!(s, " {} |", f(x, 2));
        }
        let _ = writeln!(
            s,
            " {} | {} | {} | {} |",
            sm.km_median
                .map_or_else(|| "not reached".to_owned(), |t| format!("{t} {unit}")),
            f(sm.median_mfcc, 2),
            f(sm.median_structure, 1),
            f(sm.median_renders, 0)
        );
        json_strata.push(json!({
            "stratum": name, "n": sm.n, "passed": sm.passed, "wilson": [sm.wilson.0, sm.wilson.1],
            "median_jnd": sm.median_jnd, "anytime": sm.anytime, "km_median_s": sm.km_median,
            "median_mfcc": sm.median_mfcc, "median_structure": sm.median_structure, "median_renders": sm.median_renders,
        }));
    }
    // The machine and the intake.
    let others: Vec<f64> = lines
        .iter()
        .filter_map(|t| t.machine.others_cores)
        .collect();
    let rps: Vec<f64> = lines.iter().map(|t| t.fin.renders_per_s).collect();
    let locked = lines.iter().filter(|t| t.machine.locked).count();
    let wait: f64 = lines.iter().map(|t| t.machine.lock_wait_s).sum();
    let max_others = others.iter().copied().fold(f64::NAN, f64::max);
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Machine: other processes kept a median {} cores busy (max {}) over the timed windows; {locked} of {} \
         targets under the exclusive quiet lock (waited {} s in all); a median {} renders/s.",
        f(stats::median(&others), 1),
        f(max_others, 1),
        lines.len(),
        f(wait, 0),
        f(stats::median(&rps), 0),
    );
    let mut warn: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &lines {
        for w in &t.intake.warnings {
            *warn.entry(w.as_str()).or_default() += 1;
        }
    }
    let _ = writeln!(
        s,
        "Intake warnings: {}.",
        if warn.is_empty() {
            "none".to_owned()
        } else {
            warn.iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    let mut prov: BTreeMap<String, usize> = BTreeMap::new();
    for t in &lines {
        if let Some(c) = t.at(mark) {
            *prov.entry(c.provenance.clone()).or_default() += 1;
        }
    }
    let _ = writeln!(
        s,
        "Best candidate's start at the endpoint: {}.",
        prov.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "| Target | Stratum | Pass | % | JND | Renders | Time to pass | Others (cores) |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- | --- | --- |");
    for t in &lines {
        let c = t.at(mark);
        let sc = c.and_then(|c| c.score);
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            t.target.id,
            t.target.stratum,
            if sc.is_some_and(|x| x.passed) {
                "yes"
            } else {
                "no"
            },
            sc.map_or_else(|| "–".to_owned(), |x| f(x.percent, 1)),
            sc.map_or_else(|| "–".to_owned(), |x| f(x.jnd, 2)),
            c.map_or_else(|| "–".to_owned(), |c| c.renders.to_string()),
            t.time_to_pass_by(c.map_or(t.cap, |c| c.mark))
                .map_or_else(|| "–".to_owned(), |x| format!("{x}")),
            opt(t.machine.others_cores.map(|x| (x * 10.0).round() / 10.0)),
        );
    }
    let j = json!({"run": id, "mark": mark, "strata": json_strata, "others_cores_median": stats::median(&others), "converted": converted});
    Ok((s, j))
}

fn comparison_rows(s: &mut String, a: &[&TargetLine], b: &[&TargetLine], mark: f64) -> Vec<Value> {
    let _ = writeln!(
        s,
        "| Stratum | Pairs | Pass A | Pass B | A only | B only | McNemar p (B better) | Wilcoxon n | W+ | p (B closer) | HL Δ JND (95 % CI) | MFCC p (B closer) |"
    );
    let _ = writeln!(s, "|{}", " --- |".repeat(12));
    let mut out = Vec::new();
    let bset: Vec<&TargetLine> = b.to_vec();
    for (name, va) in strata(a) {
        let c = compare(&va, &bset, mark);
        let hl = c.hl.map_or_else(
            || "–".to_owned(),
            |h| format!("{} ({} to {})", f(h.estimate, 2), f(h.lo, 2), f(h.hi, 2)),
        );
        let _ = writeln!(
            s,
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {hl} | {} |",
            c.pairs,
            c.passed.0,
            c.passed.1,
            c.mcnemar.a_only,
            c.mcnemar.b_only,
            f(c.mcnemar.p_b_better, 4),
            c.wilcoxon.n,
            f(c.wilcoxon.w_plus, 1),
            f(c.wilcoxon.p_greater, 4),
            f(c.mfcc.p_greater, 4),
        );
        out.push(json!({
            "stratum": name, "pairs": c.pairs, "passed": [c.passed.0, c.passed.1],
            "mcnemar": {"a_only": c.mcnemar.a_only, "b_only": c.mcnemar.b_only, "p_b_better": c.mcnemar.p_b_better, "p_a_better": c.mcnemar.p_a_better, "p_two_sided": c.mcnemar.p_two_sided},
            "wilcoxon": {"n": c.wilcoxon.n, "w_plus": c.wilcoxon.w_plus, "p_greater": c.wilcoxon.p_greater, "p_less": c.wilcoxon.p_less},
            "hl": c.hl.map(|h| json!({"estimate": h.estimate, "lo": h.lo, "hi": h.hi, "coverage": h.coverage})),
            "mfcc_p_greater": c.mfcc.p_greater,
        }));
    }
    out
}

/// The paired report and the decision.
pub fn paired(
    runs: &Runs,
    a: &str,
    b: &str,
    at: Option<f64>,
    long: Option<(&str, &str, f64)>,
) -> Result<(String, Value), String> {
    let get = |id: &str| -> Result<Vec<&TargetLine>, String> {
        runs.targets
            .get(id)
            .map(|v| v.iter().collect())
            .ok_or_else(|| format!("run {id} has no target lines"))
    };
    let (la, lb) = (get(a)?, get(b)?);
    let ra = runs
        .headers
        .iter()
        .find(|r| r.run_id == a)
        .ok_or_else(|| format!("no run {a}"))?;
    let mark = mark_of(ra, at);
    let mut s = String::new();
    let _ = writeln!(s, "A: {}", run_text(runs, a)?);
    let _ = writeln!(s, "B: {}", run_text(runs, b)?);
    let _ = writeln!(
        s,
        "\nAt {mark} (d = JND_A − JND_B; positive: B is closer):\n"
    );
    let rows = comparison_rows(&mut s, &la, &lb, mark);
    let primary = compare(&la, &lb, mark);
    let long_cmp = match long {
        Some((x, y, m)) => {
            let (lx, ly) = (get(x)?, get(y)?);
            let _ = writeln!(s, "\nAt {m} (runs {x} and {y}):\n");
            comparison_rows(&mut s, &lx, &ly, m);
            Some(compare(&lx, &ly, m))
        }
        None => None,
    };
    let d = decide(&primary, long_cmp.as_ref());
    let _ = writeln!(
        s,
        "\nDecision (fixed in advance, docs/gr3.md): better at {mark}: {} (McNemar p = {}, Wilcoxon p = {}, both < {ALPHA} needed); \
         not worse at the long mark: {}; B beats A: {}.",
        if d.better_at_primary { "yes" } else { "no" },
        f(primary.mcnemar.p_b_better, 4),
        f(primary.wilcoxon.p_greater, 4),
        match d.not_worse_at_long {
            Some(true) => "yes",
            Some(false) => "no",
            None => "not measured (no long runs)",
        },
        if d.beats() { "YES" } else { "no" },
    );
    let j = json!({
        "a": a, "b": b, "mark": mark, "strata": rows,
        "decision": {"better_at_primary": d.better_at_primary, "not_worse_at_long": d.not_worse_at_long, "beats": d.beats()},
    });
    Ok((s, j))
}

fn list(runs: &Runs) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "| Run | Arm | Mode | Cap | Split | Targets | Failed | Passed at cap | Ended |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &runs.headers {
        let lines = runs.targets.get(&r.run_id).map_or(&[][..], Vec::as_slice);
        let passed = lines
            .iter()
            .filter(|t| t.at_cap().and_then(|c| c.score).is_some_and(|x| x.passed))
            .count();
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {passed} | {} |",
            r.run_id,
            r.arm,
            r.mode,
            r.seconds
                .map_or_else(|| r.budget.unwrap_or(0).to_string(), |x| format!("{x} s")),
            r.split,
            lines.len(),
            runs.failed.get(&r.run_id).map_or(0, Vec::len),
            if runs.ended.contains_key(&r.run_id) {
                "yes"
            } else {
                "no"
            },
        );
    }
    s
}

/// `cargo xtask gr3-report`.
pub fn gr3_report(args: &[String]) -> Result<(), String> {
    let mut kv: BTreeMap<String, String> = BTreeMap::new();
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json_out = true;
                i += 1;
            }
            "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            k @ ("--runs" | "--run" | "--a" | "--b" | "--at" | "--long-a" | "--long-b"
            | "--long-at" | "--rate-from" | "--probe-runs") => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{k} needs a value"))?;
                kv.insert(k.to_owned(), v.clone());
                i += 2;
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let path = kv
        .get("--runs")
        .map_or_else(|| runs::gr3_dir(&root).join("runs.jsonl"), PathBuf::from);
    let (runs, notes) = runs::read_runs(&path)?;
    for n in notes {
        eprintln!("gr3-report: {n}");
    }
    let at = kv
        .get("--at")
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| format!("--at: not a number: {v}"))
        })
        .transpose()?;
    let long_at: f64 = kv.get("--long-at").map_or(Ok(LONG_AT), |v| {
        v.parse()
            .map_err(|_| format!("--long-at: not a number: {v}"))
    })?;
    let (text, j) = match (kv.get("--run"), kv.get("--a"), kv.get("--b")) {
        (Some(id), None, None) => {
            let probe_runs = match kv.get("--probe-runs") {
                Some(p) => Some(runs::read_runs(Path::new(p))?.0),
                None => None,
            };
            let rate_from = kv
                .get("--rate-from")
                .map(|r| (probe_runs.as_ref().unwrap_or(&runs), r.as_str()));
            single(&runs, id, at, rate_from)?
        }
        (None, Some(a), Some(b)) => {
            let long = match (kv.get("--long-a"), kv.get("--long-b")) {
                (Some(x), Some(y)) => Some((x.as_str(), y.as_str(), long_at)),
                (None, None) => {
                    // The same runs, when they reach the long mark.
                    let reach = |id: &str| {
                        runs.targets.get(id).is_some_and(|v| {
                            !v.is_empty() && v.iter().all(|t| t.at(long_at).is_some())
                        })
                    };
                    (reach(a) && reach(b)).then_some((a.as_str(), b.as_str(), long_at))
                }
                _ => return Err("--long-a and --long-b go together".to_owned()),
            };
            paired(&runs, a, b, at, long)?
        }
        (None, None, None) => (list(&runs), Value::Null),
        _ => {
            return Err(format!(
                "name one run (--run) or two (--a and --b)\n{USAGE}"
            ));
        }
    };
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&j).map_err(|e| e.to_string())?
        );
    } else {
        print!("{text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gr3::runs::parse_runs;

    /// A run of `n` targets: target i passes at 60 s when `pass(i)`, with `jnd(i)` (and the same MFCC).
    fn run(
        id: &str,
        arm: &str,
        n: usize,
        pass: &dyn Fn(usize) -> bool,
        jnd: &dyn Fn(usize) -> f64,
    ) -> String {
        let mut s = format!(
            "{{\"schema\":\"enth.gr3-run/1\",\"record\":\"run\",\"run_id\":\"{id}\",\"arm\":\"{arm}\",\"mode\":\"wall\",\"seconds\":60.0,\"workers\":8,\"split\":\"test\",\"date\":\"d\"}}\n"
        );
        for i in 0..n {
            let (p, j) = (pass(i), jnd(i));
            let stratum = if i % 2 == 0 { "gen-1note" } else { "preset" };
            let kind = if i % 2 == 0 { "generator" } else { "preset" };
            // Passed targets pass from 10 s on when i < 3, else only at 60 s.
            let p10 = p && i < 3;
            let _ = writeln!(
                s,
                "{{\"schema\":\"enth.gr3-run/1\",\"record\":\"target\",\"run_id\":\"{id}\",\"arm\":\"{arm}\",\"mode\":\"wall\",\"cap\":60.0,\
                 \"target\":{{\"id\":\"t{i}\",\"kind\":\"{kind}\",\"level\":1,\"stratum\":\"{stratum}\",\"core\":true}},\
                 \"checkpoints\":[{{\"mark\":10.0,\"renders\":100,\"score\":{{\"percent\":50.0,\"jnd\":{},\"passed\":{p10},\"mfcc\":{}}}}},\
                 {{\"mark\":30.0,\"renders\":200,\"score\":{{\"percent\":50.0,\"jnd\":{},\"passed\":{p10},\"mfcc\":{}}}}},\
                 {{\"mark\":60.0,\"renders\":300,\"score\":{{\"percent\":50.0,\"jnd\":{j},\"passed\":{p},\"mfcc\":{j}}}}}]}}",
                j + 2.0,
                j + 2.0,
                j + 1.0,
                j + 1.0
            );
        }
        s
    }

    #[test]
    fn a_single_run_by_hand() {
        // Ten targets; 0, 1, 2, 3, 4 pass at 60 s (0, 1, 2 already at 10 s); JND at 60 s = i.
        let text = run("a", "A", 10, &|i| i < 5, &|i| i as f64);
        let (runs, _) = parse_runs(&text).unwrap();
        let lines: Vec<&TargetLine> = runs.targets["a"].iter().collect();
        let sm = summarize(&lines, 60.0, &ANYTIME);
        assert_eq!((sm.n, sm.passed), (10, 5));
        assert!((sm.wilson.0 - 0.236_592).abs() < 1e-5);
        // JND 0..9: median 4.5; at 10 s, 2..11: 6.5; at 30 s, 1..10: 5.5.
        assert!((sm.median_jnd - 4.5).abs() < 1e-12);
        assert_eq!(sm.anytime, vec![(10.0, 6.5), (30.0, 5.5), (60.0, 4.5)]);
        // Time to pass: 3 at 10 s, 2 at 60 s, 5 censored at 60: S = 7/10 after 10 s, then 7/10 × (1 − 2/7) = ½
        // after 60 s, so the median is 60 s.
        assert_eq!(sm.km.len(), 2);
        assert!((sm.km[0].survival - 0.7).abs() < 1e-12);
        assert!((sm.km[1].survival - 0.5).abs() < 1e-12);
        assert_eq!(sm.km_median, Some(60.0));
        let (out, _) = single(&runs, "a", None, None).unwrap();
        assert!(out.contains("| all | 10 | 5 | 50.0 %"), "{out}");
        assert!(out.contains("NO run_end line"));
    }

    #[test]
    fn a_paired_comparison_by_hand() {
        // Ten targets. A passes 0..=1 and 8..=9; B passes 0..=9 except 8 and 9: concordant 0, 1; A only 8, 9; B only
        // 2..=7. So McNemar has 2 against 6: P(X ≥ 6 | 8, ½) = (28 + 8 + 1)/256 = 37/256.
        // JND: A = i + 1, B = i + 0.5 on even i and i + 1.25 on odd i: d = 0.5 (even), −0.25 (odd). |d| ranks: the
        // five 0.25s take ranks 1–5 (3 each), the five 0.5s ranks 6–10 (8 each): W+ = 40, W− = 15.
        let a = run("a", "A", 10, &|i| i <= 1 || i >= 8, &|i| i as f64 + 1.0);
        let b = run("b", "B", 10, &|i| i < 8, &|i| {
            if i % 2 == 0 {
                i as f64 + 0.5
            } else {
                i as f64 + 1.25
            }
        });
        let (runs, _) = parse_runs(&format!("{a}{b}")).unwrap();
        let la: Vec<&TargetLine> = runs.targets["a"].iter().collect();
        let lb: Vec<&TargetLine> = runs.targets["b"].iter().collect();
        let c = compare(&la, &lb, 60.0);
        assert_eq!(c.pairs, 10);
        assert_eq!((c.mcnemar.a_only, c.mcnemar.b_only), (2, 6));
        assert!((c.mcnemar.p_b_better - 37.0 / 256.0).abs() < 1e-12);
        assert_eq!(c.wilcoxon.n, 10);
        assert!((c.wilcoxon.w_plus - 40.0).abs() < 1e-12);
        assert!((c.wilcoxon.w_minus - 15.0).abs() < 1e-12);
        // Not better at 60 s: McNemar's p is 0.145.
        let d = decide(&c, None);
        assert!(!d.better_at_primary && !d.beats());
        // B against itself at the long mark: no discordance, every d zero: not worse.
        let same = compare(&lb, &lb, 60.0);
        assert_eq!(decide(&same, Some(&same)).not_worse_at_long, Some(true));
        let (out, j) = paired(&runs, "a", "b", None, None).unwrap();
        assert!(out.contains("B beats A: no"), "{out}");
        assert_eq!(j["decision"]["beats"], json!(false));
    }

    #[test]
    fn a_render_run_converts_with_a_measured_idle_rate() {
        // The probe: three targets with 3,000, 3,600 and 4,200 renders at 60 s: rates 50, 60, 70, median 60 renders/s,
        // so 60 s is 3,600 renders; a render run with marks every 250 is read at 3,500.
        let mut text = String::from(
            "{\"schema\":\"enth.gr3-run/1\",\"record\":\"run\",\"run_id\":\"p\",\"arm\":\"A\",\"mode\":\"wall\",\"seconds\":60.0}\n",
        );
        for (i, r) in [3000, 3600, 4200].iter().enumerate() {
            let _ = writeln!(
                text,
                "{{\"schema\":\"enth.gr3-run/1\",\"record\":\"target\",\"run_id\":\"p\",\"arm\":\"A\",\"mode\":\"wall\",\"cap\":60.0,\
                 \"target\":{{\"id\":\"d{i}\"}},\"machine\":{{\"locked\":true}},\"checkpoints\":[{{\"mark\":60.0,\"renders\":{r}}}]}}"
            );
        }
        let _ = writeln!(
            text,
            "{{\"schema\":\"enth.gr3-run/1\",\"record\":\"run\",\"run_id\":\"c\",\"arm\":\"A\",\"mode\":\"renders\",\"budget\":8000,\
             \"header\":{{\"equivalence\":{{\"kind\":\"estimate\",\"seconds\":60.0,\"renders\":4000,\"rate_renders_per_s\":66.7,\"source\":\"TP\"}}}}}}"
        );
        // Two targets: t costs 12.5 ms of CPU per render, u 25 ms; the median is 18.75 ms. Per target, 3,600 renders
        // become 3,600 × 18.75 / 12.5 = 5,400 for t (read at 5,250) and 2,700 for u (read at 2,500). Both pass from
        // 5,000 renders on, so the common mark (3,500) passes neither, and the per-target reading passes t.
        for (id, cpu) in [("t", 100.0), ("u", 200.0)] {
            let marks: Vec<String> = (1..=32)
                .map(|k| {
                    let m = 250 * k;
                    let pass = m >= 5000;
                    let j = if pass { 0.5 } else { 2.0 };
                    format!("{{\"mark\":{m}.0,\"renders\":{m},\"score\":{{\"percent\":95.0,\"jnd\":{j},\"passed\":{pass}}}}}")
                })
                .collect();
            let _ = writeln!(
                text,
                "{{\"schema\":\"enth.gr3-run/1\",\"record\":\"target\",\"run_id\":\"c\",\"arm\":\"A\",\"mode\":\"renders\",\"cap\":8000.0,\
                 \"target\":{{\"id\":\"{id}\"}},\"machine\":{{\"cpu_s\":{cpu}}},\"final\":{{\"renders\":8000}},\"checkpoints\":[{}]}}",
                marks.join(",")
            );
        }
        let (runs, _) = parse_runs(&text).unwrap();
        let (rate, locked, n) = probe_rate(&runs, "p").unwrap();
        assert!((rate - 60.0).abs() < 1e-12);
        assert_eq!((locked, n), (3, 3));
        // Without a probe the endpoint is the equivalence's 4,000 renders, flagged as an estimate.
        let (out, j) = single(&runs, "c", None, None).unwrap();
        assert!(out.contains("Render-equivalent estimate"), "{out}");
        assert_eq!(j["mark"], json!(4000.0));
        let (out, j) = single(&runs, "c", None, Some((&runs, "p"))).unwrap();
        assert_eq!(j["mark"], json!(3500.0), "{out}");
        assert!(out.contains("3600 renders"), "{out}");
        assert_eq!(j["converted"]["per_target"]["passed"], json!(1), "{out}");
        assert_eq!(j["converted"]["per_target"]["capped"], json!(0));
        assert!(out.contains("| all | 2 | 0 |"), "{out}");
        // A timed run is not converted.
        assert!(single(&runs, "p", None, Some((&runs, "p"))).is_err());
    }

    #[test]
    fn the_decision_needs_both_tests_at_the_primary_mark() {
        // B passes every target A fails (10 discordant, all B's: p = 1/1024) and is closer on all ten (W+ = 55,
        // p = 1/1024): better. At the long mark, A is closer on all ten: worse, so B does not beat A.
        let a = run("a", "A", 10, &|_| false, &|i| i as f64 + 2.0);
        let b = run("b", "B", 10, &|_| true, &|i| {
            i as f64 + 1.0 - 0.01 * i as f64
        });
        let (runs, _) = parse_runs(&format!("{a}{b}")).unwrap();
        let la: Vec<&TargetLine> = runs.targets["a"].iter().collect();
        let lb: Vec<&TargetLine> = runs.targets["b"].iter().collect();
        let p = compare(&la, &lb, 60.0);
        assert!((p.mcnemar.p_b_better - 1.0 / 1024.0).abs() < 1e-12);
        assert!((p.wilcoxon.p_greater - 1.0 / 1024.0).abs() < 1e-12);
        let worse = compare(&lb, &la, 60.0);
        let d = decide(&p, Some(&worse));
        assert!(d.better_at_primary);
        assert_eq!(d.not_worse_at_long, Some(false));
        assert!(!d.beats());
        assert!(decide(&p, Some(&p)).beats());
    }
}
