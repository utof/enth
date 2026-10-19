//! `--analyze`: per-target same / different, the G0 count, the controls and what the test could not see.

use std::fmt::Write as _;

use serde::Serialize;

use super::{G0_SAME_OF_20, Manifest, SCHEMA_REPORT, Side, TrialKind};
use crate::flags::summary;
use crate::session::AnswerFile;
use crate::stats;

/// The verdict on one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The owner could not be told from guessing: the solution sounds the same.
    Same,
    /// The owner beat guessing: the solution sounds different.
    Different,
    /// Not all trials of this target are answered yet.
    Pending,
}

/// One target's row.
#[derive(Clone, Debug, Serialize)]
pub struct ItemRow {
    /// 1-based, as listed in the manifest.
    pub target: usize,
    /// Trials planned for this target.
    pub planned: usize,
    /// Trials answered.
    pub answered: usize,
    /// Correct answers.
    pub correct: usize,
    /// One-sided exact binomial p-value of at least this many correct answers by guessing.
    pub p: f64,
    /// The verdict.
    pub verdict: Verdict,
    /// Distance A of the solution over the strict threshold T1 (at most 1 means T1 was reached).
    pub distance_over_t1: f64,
    /// The search reached T1.
    pub reached_t1: bool,
    /// The search reached T3.
    pub reached_t3: bool,
    /// Evaluations used.
    pub evals: usize,
    /// Wall-clock seconds of the search.
    pub wall_s: f64,
    /// Click or discontinuity flags of the target and solution clips.
    pub flags: String,
}

/// Correct answers out of trials.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Count {
    /// Trials answered.
    pub n: usize,
    /// Correct answers.
    pub correct: usize,
}

/// The two kinds of control trial.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Controls {
    /// Two different targets: nearly all should be right.
    pub different: Count,
    /// The same clip twice: any answer is a guess, about half should be "right".
    pub null: Count,
    /// Two-sided exact p-value of the null trials against 50 %.
    pub null_p: f64,
    /// At least 90 % of the obviously different trials were right.
    pub different_ok: bool,
    /// The null trials do not differ significantly from 50 %.
    pub null_ok: bool,
}

/// What the test could and could not see at this many trials per target.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Sensitivity {
    /// Trials per target.
    pub trials: usize,
    /// Correct answers needed to be called *different* (`None`: even all correct would not be significant).
    pub critical_correct: Option<usize>,
    /// Chance of being called *different* by a listener who is right 85 % of the time.
    pub power_at_85: f64,
    /// Same at 95 %.
    pub power_at_95: f64,
}

/// The `--analyze` result (also written to `abx-report.json`).
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    /// [`SCHEMA_REPORT`].
    pub schema: String,
    /// True when any answer file holds synthetic (dry-run) answers.
    pub synthetic: bool,
    /// Significance level.
    pub alpha: f64,
    /// Per target.
    pub items: Vec<ItemRow>,
    /// Targets judged same.
    pub same: usize,
    /// Targets judged different.
    pub different: usize,
    /// Targets not fully answered.
    pub pending: usize,
    /// Targets in the design.
    pub targets: usize,
    /// Bar for *same*: 14 of 20, scaled for other sizes.
    pub g0_needed: usize,
    /// `pass`, `fail`, `open`, or `invalid` (too few trials for any target to be called different).
    pub g0: String,
    /// The same count if every target with a p-value under 0.15 were called different.
    pub same_strict: usize,
    /// Controls.
    pub controls: Controls,
    /// Share of all answers that were "a".
    pub answered_a_share: f64,
    /// Test sensitivity.
    pub sensitivity: Sensitivity,
    /// Targets whose search reached T1: how many were judged same.
    pub t1_reached_same: (usize, usize),
    /// Targets whose search did not reach T1: how many were judged same.
    pub t1_missed_same: (usize, usize),
    /// Longest search wall time, seconds.
    pub max_wall_s: f64,
    /// Workers and cap the searches ran with.
    pub workers: usize,
    /// Wall-clock cap in seconds.
    pub time_cap_s: f64,
    /// `uptime` load when the design started and ended.
    pub load: [String; 2],
    /// Clips with click or discontinuity flags: `target n (target|solution): summary`.
    pub flagged: Vec<String>,
}

/// Judge every target from the answers (any number of sessions, any subset answered).
#[must_use]
pub fn analyze(m: &Manifest, files: &[AnswerFile], alpha: f64) -> Report {
    let mut correct_of: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    let mut a_answers = 0usize;
    let mut total = 0usize;
    for f in files {
        for a in &f.answers {
            if let Some(t) = m.trials.iter().find(|t| t.id == a.id) {
                let right = a.choice == t.x_is.key();
                correct_of.insert(t.id.as_str(), right);
                total += 1;
                a_answers += usize::from(a.choice == Side::A.key());
            }
        }
    }
    let n_items = m.items.len();
    let mut items = Vec::new();
    for it in &m.items {
        let ts: Vec<_> = m
            .trials
            .iter()
            .filter(|t| t.kind == TrialKind::Real && t.item == it.index)
            .collect();
        let answered: Vec<bool> = ts
            .iter()
            .filter_map(|t| correct_of.get(t.id.as_str()).copied())
            .collect();
        let correct = answered.iter().filter(|b| **b).count();
        let p = stats::tail_ge(answered.len(), correct, 0.5);
        let verdict = if answered.len() < ts.len() {
            Verdict::Pending
        } else if p <= alpha {
            Verdict::Different
        } else {
            Verdict::Same
        };
        let fl: Vec<String> = m
            .clips
            .iter()
            .filter(|c| c.item == it.index && !c.flags.is_empty())
            .map(|c| format!("{:?} {}", c.role, summary(&c.flags)).to_lowercase())
            .collect();
        items.push(ItemRow {
            target: it.index + 1,
            planned: ts.len(),
            answered: answered.len(),
            correct,
            p,
            verdict,
            distance_over_t1: it.distance / it.t1.max(1e-12),
            reached_t1: it.reached_t1,
            reached_t3: it.reached_t3,
            evals: it.evals,
            wall_s: it.wall_s,
            flags: fl.join("; "),
        });
    }
    let count = |v: Verdict| items.iter().filter(|r| r.verdict == v).count();
    let (same, different, pending) = (
        count(Verdict::Same),
        count(Verdict::Different),
        count(Verdict::Pending),
    );
    let g0_needed = if n_items == 20 {
        G0_SAME_OF_20
    } else {
        (n_items * 7).div_ceil(10)
    };
    let can_flag = stats::critical_correct(m.config.trials_per_target, alpha).is_some();
    let g0 = if !can_flag {
        "invalid"
    } else if same >= g0_needed {
        "pass"
    } else if same + pending < g0_needed {
        "fail"
    } else {
        "open"
    };
    let same_strict = items
        .iter()
        .filter(|r| r.verdict == Verdict::Same && r.p > 0.15)
        .count();
    let tally = |kind: TrialKind| {
        let mut c = Count { n: 0, correct: 0 };
        for t in m.trials.iter().filter(|t| t.kind == kind) {
            if let Some(r) = correct_of.get(t.id.as_str()) {
                c.n += 1;
                c.correct += usize::from(*r);
            }
        }
        c
    };
    let (different_c, null_c) = (tally(TrialKind::Different), tally(TrialKind::Null));
    let null_p = stats::two_sided(null_c.n, null_c.correct);
    let controls = Controls {
        different: different_c,
        null: null_c,
        null_p,
        different_ok: different_c.n == 0
            || different_c.correct as f64 >= 0.9 * different_c.n as f64,
        null_ok: null_c.n == 0 || null_p >= 0.05,
    };
    let trials = m.config.trials_per_target;
    let split = |reached: bool| {
        let rows: Vec<&ItemRow> = items
            .iter()
            .filter(|r| r.reached_t1 == reached && r.verdict != Verdict::Pending)
            .collect();
        (
            rows.iter().filter(|r| r.verdict == Verdict::Same).count(),
            rows.len(),
        )
    };
    let (t1_reached_same, t1_missed_same) = (split(true), split(false));
    let mut flagged = Vec::new();
    for c in m.clips.iter().filter(|c| !c.flags.is_empty()) {
        flagged.push(
            format!("target {} {:?}: {}", c.item + 1, c.role, summary(&c.flags)).to_lowercase(),
        );
    }
    Report {
        schema: SCHEMA_REPORT.to_string(),
        synthetic: files.iter().any(|f| f.synthetic),
        alpha,
        items,
        same,
        different,
        pending,
        targets: n_items,
        g0_needed,
        g0: g0.to_string(),
        same_strict,
        controls,
        answered_a_share: if total == 0 {
            0.5
        } else {
            a_answers as f64 / total as f64
        },
        sensitivity: Sensitivity {
            trials,
            critical_correct: stats::critical_correct(trials, alpha),
            power_at_85: stats::power(trials, alpha, 0.85),
            power_at_95: stats::power(trials, alpha, 0.95),
        },
        t1_reached_same,
        t1_missed_same,
        max_wall_s: m.items.iter().map(|i| i.wall_s).fold(0.0, f64::max),
        workers: m.config.workers,
        time_cap_s: m.config.time_cap_s,
        load: m.load.clone(),
        flagged,
    }
}

impl Report {
    /// A table for the terminal.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "ABX analysis: {} targets, {} trials each, one-sided exact binomial test at {}{}",
            self.targets,
            self.sensitivity.trials,
            self.alpha,
            if self.synthetic {
                "  ** SYNTHETIC ANSWERS (dry run), not the owner's **"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "{:>3}  {:>7}  {:>7}  {:<9}  {:>9}  {:>6}  {:>6}  proxy",
            "#", "correct", "p", "verdict", "dist/T1", "evals", "wall"
        );
        for r in &self.items {
            let _ = writeln!(
                s,
                "{:>3}  {:>3}/{:<3}  {:>7.4}  {:<9}  {:>9.2}  {:>6}  {:>5.1}s  {}{}",
                r.target,
                r.correct,
                r.answered,
                r.p,
                match r.verdict {
                    Verdict::Same => "same",
                    Verdict::Different => "DIFFERENT",
                    Verdict::Pending => "pending",
                },
                r.distance_over_t1,
                r.evals,
                r.wall_s,
                if r.reached_t1 {
                    "T1 reached"
                } else if r.reached_t3 {
                    "T3 only"
                } else {
                    "neither"
                },
                if r.flags.is_empty() {
                    String::new()
                } else {
                    format!("  [flags: {}]", r.flags)
                }
            );
        }
        let _ = writeln!(
            s,
            "\njudged same: {} of {}   different: {}   pending: {}",
            self.same, self.targets, self.different, self.pending
        );
        let bar = if self.targets == 20 {
            "14 of 20".to_string()
        } else {
            format!("{} of {}", self.g0_needed, self.targets)
        };
        let verdict = match self.g0.as_str() {
            "pass" => "PASS".to_string(),
            "fail" => "FAIL".to_string(),
            "invalid" => "INVALID: with this few trials per target no target can ever be called different, so every target would count as same".to_string(),
            _ => format!(
                "OPEN: {} target(s) still pending, between {} and {} same at the end",
                self.pending,
                self.same,
                self.same + self.pending
            ),
        };
        let _ = writeln!(
            s,
            "G0 search gate (at least {bar} judged same, within {:.0} s on {} workers): {verdict}",
            self.time_cap_s, self.workers
        );
        let within = if self.max_wall_s <= self.time_cap_s + 1.0 {
            "within the cap"
        } else {
            "OVER the cap"
        };
        let _ = writeln!(
            s,
            "  searches: longest {:.1} s ({within}), load at design start {} -> end {}",
            self.max_wall_s, self.load[0], self.load[1]
        );
        let _ = writeln!(
            s,
            "  counted stricter (a target with p <= 0.15 is not 'same'): {} of {} same",
            self.same_strict, self.targets
        );
        let _ = writeln!(
            s,
            "  proxy check: of the targets whose search reached T1, {} of {} judged same; of those that did not, {} of {}",
            self.t1_reached_same.0,
            self.t1_reached_same.1,
            self.t1_missed_same.0,
            self.t1_missed_same.1
        );
        let c = &self.controls;
        let _ = writeln!(
            s,
            "controls: obviously different pairs {}/{} right{}; identical pairs {}/{} 'right' (p = {:.2} against 50 %){}",
            c.different.correct,
            c.different.n,
            if c.different_ok {
                ""
            } else {
                "  ** CHECK headphones, volume, attention **"
            },
            c.null.correct,
            c.null.n,
            c.null_p,
            if c.null_ok {
                ""
            } else {
                "  ** CHECK: you 'hear' differences in identical sounds (or lean to one answer) **"
            }
        );
        let _ = writeln!(
            s,
            "position: answered A in {:.0} % of all trials (50 % is balanced)",
            100.0 * self.answered_a_share
        );
        let se = &self.sensitivity;
        let caught = se.critical_correct.map_or_else(
            || {
                format!(
                    "with {} trials per target no result can be significant at {}: use more trials",
                    se.trials, self.alpha
                )
            },
            |k| {
                format!(
                    "with {} trials per target a target is called different at {k} or more correct",
                    se.trials
                )
            },
        );
        let _ = writeln!(
            s,
            "sensitivity: {caught}; a listener who is right 85 % of the time is caught {:.0} % of the time, at 95 % {:.0} %. 'Same' means 'not caught', not 'proved identical'.",
            100.0 * se.power_at_85,
            100.0 * se.power_at_95
        );
        if !self.flagged.is_empty() {
            let _ = writeln!(
                s,
                "clips with click or discontinuity flags (kept in the test): {}",
                self.flagged.join("; ")
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ClipInfo, Config, Item, Role, SCHEMA_ANSWERS, SCHEMA_MANIFEST, StopAt, Trial,
    };
    use super::*;
    use crate::session::Answer;
    use spike_metric::render::RenderCfg;

    /// Four targets, 8 trials each (no controls): target 1 and 2 are in a manifest the tests answer by hand.
    fn manifest(trials: usize) -> Manifest {
        let items: Vec<Item> = (0..4)
            .map(|i| Item {
                index: i,
                target: vec![0.5; 10],
                solution: vec![0.5; 10],
                t1: 1.0,
                t3: 2.0,
                start_distance: None,
                distance: if i < 3 { 0.8 } else { 2.5 },
                evals: 100,
                restarts: 0,
                wall_s: 3.0,
                reached_t1: i < 3,
                reached_t3: true,
                evals_to_t1: None,
                wall_to_t1_s: None,
                wave_ok: true,
                param_err: 0.0,
                target_clip: format!("ct{i}"),
                solution_clip: format!("cs{i}"),
            })
            .collect();
        let mut clips = Vec::new();
        for i in &items {
            for (id, role) in [
                (&i.target_clip, Role::Target),
                (&i.solution_clip, Role::Solution),
            ] {
                clips.push(ClipInfo {
                    id: id.clone(),
                    item: i.index,
                    role,
                    raw_lufs: Some(-20.0),
                    gain: 1.0,
                    peak_dbfs: -6.0,
                    flags: vec![],
                });
            }
        }
        let mut tr = Vec::new();
        for s in 1..=trials as u32 {
            for i in &items {
                tr.push(Trial {
                    session: s,
                    n: tr.len() + 1,
                    id: format!("t{}-{}", s, i.index),
                    kind: TrialKind::Real,
                    item: i.index,
                    item_b: None,
                    a: i.target_clip.clone(),
                    b: i.solution_clip.clone(),
                    x_is: if s % 2 == 0 { Side::A } else { Side::B },
                });
            }
            for k in 0..2 {
                tr.push(Trial {
                    session: s,
                    n: 0,
                    id: format!("n{s}-{k}"),
                    kind: TrialKind::Null,
                    item: 0,
                    item_b: None,
                    a: "ct0".into(),
                    b: "ct0".into(),
                    x_is: Side::A,
                });
                tr.push(Trial {
                    session: s,
                    n: 0,
                    id: format!("d{s}-{k}"),
                    kind: TrialKind::Different,
                    item: 0,
                    item_b: Some(1),
                    a: "ct0".into(),
                    b: "ct1".into(),
                    x_is: Side::B,
                });
            }
        }
        Manifest {
            schema: SCHEMA_MANIFEST.into(),
            config: Config {
                seed: 1,
                n_targets: 4,
                trials_per_target: trials,
                n_null: 2,
                n_diff: 2,
                workers: 8,
                time_cap_s: 60.0,
                budget: 5000,
                stop_at: StopAt::T1,
                lambda: 16,
                sigma_start: 0.15,
                search_seed: 1000,
                render: RenderCfg::default(),
            },
            host_threads: 12,
            load: ["1 1 1".into(), "2 2 2".into()],
            design_wall_s: 10.0,
            items,
            clips,
            trials: tr,
        }
    }

    /// Answers where target `i` is answered correctly `right[i]` times in 8 trials; controls: different always right,
    /// null alternating.
    fn answers(m: &Manifest, right: &[usize]) -> Vec<AnswerFile> {
        let mut files = Vec::new();
        for s in 1..=m.sessions() {
            let mut f = AnswerFile::new(SCHEMA_ANSWERS, s, false);
            for t in m.session_trials(s) {
                let correct = match t.kind {
                    TrialKind::Real => (s as usize) <= right[t.item],
                    TrialKind::Different => true,
                    TrialKind::Null => s % 2 == 0,
                };
                let choice = if correct {
                    t.x_is
                } else if t.x_is == Side::A {
                    Side::B
                } else {
                    Side::A
                };
                f.answers.push(Answer {
                    n: t.n,
                    id: t.id.clone(),
                    choice: choice.key().into(),
                    replays: 0,
                    ms: 0,
                });
            }
            files.push(f);
        }
        files
    }

    #[test]
    fn verdicts_follow_the_exact_binomial_test() {
        let m = manifest(8);
        // 8/8 and 7/8 are different (p = 0.0039, 0.035); 6/8 (p = 0.145) and 4/8 are same
        let r = analyze(&m, &answers(&m, &[8, 7, 6, 4]), 0.05);
        let v: Vec<Verdict> = r.items.iter().map(|i| i.verdict).collect();
        assert_eq!(
            v,
            vec![
                Verdict::Different,
                Verdict::Different,
                Verdict::Same,
                Verdict::Same
            ]
        );
        assert!((r.items[1].p - 9.0 / 256.0).abs() < 1e-12);
        assert_eq!((r.same, r.different, r.pending), (2, 2, 0));
        assert_eq!(r.sensitivity.critical_correct, Some(7));
        // 4 targets: bar is ceil(0.7 * 4) = 3, so 2 same fails
        assert_eq!((r.g0_needed, r.g0.as_str()), (3, "fail"));
        // stricter reading: 6/8 (p = 0.145) is not clearly same
        assert_eq!(r.same_strict, 1);
        // controls
        assert!(r.controls.different_ok && r.controls.null_ok);
        assert_eq!((r.controls.different.n, r.controls.null.n), (16, 16));
        assert_eq!(r.controls.null.correct, 8);
        // proxy: three reached T1 (two different, one same), one missed (same)
        assert_eq!(r.t1_reached_same, (1, 3));
        assert_eq!(r.t1_missed_same, (1, 1));
        let text = r.to_text();
        assert!(
            text.contains("judged same: 2 of 4") && text.contains("FAIL"),
            "{text}"
        );
    }

    #[test]
    fn partial_answers_leave_the_gate_open_until_it_is_decided() {
        let m = manifest(8);
        let mut files = answers(&m, &[4, 4, 4, 4]);
        // drop sessions 5-8: every target has 4 of 8 answered
        files.truncate(4);
        let r = analyze(&m, &files, 0.05);
        assert_eq!(r.pending, 4);
        assert_eq!(r.g0, "open");
        assert!(r.to_text().contains("OPEN"));
        // nothing answered at all: still pending, no panic
        assert_eq!(analyze(&m, &[], 0.05).pending, 4);
        // everything answered "same": 4 same >= 3: pass
        let r = analyze(&m, &answers(&m, &[4, 4, 4, 4]), 0.05);
        assert_eq!((r.same, r.g0.as_str()), (4, "pass"));
        // too few trials to ever call a target different: the gate cannot pass by default
        let few = manifest(4);
        let r = analyze(&few, &answers(&few, &[2, 2, 2, 2]), 0.05);
        assert_eq!(r.g0, "invalid");
        assert!(r.to_text().contains("INVALID"));
    }

    #[test]
    fn failing_controls_are_called_out() {
        let m = manifest(8);
        let mut files = answers(&m, &[4, 4, 4, 4]);
        for f in &mut files {
            for a in &mut f.answers {
                let t = m.trials.iter().find(|t| t.id == a.id).unwrap();
                if t.kind == TrialKind::Null {
                    a.choice = t.x_is.key().into(); // always "right" on identical sounds: 16/16
                }
                if t.kind == TrialKind::Different {
                    a.choice = if t.x_is == Side::A {
                        "b".into()
                    } else {
                        "a".into()
                    }; // always wrong
                }
            }
        }
        let r = analyze(&m, &files, 0.05);
        assert!(!r.controls.different_ok && !r.controls.null_ok);
        assert!(r.to_text().contains("CHECK"));
    }
}
