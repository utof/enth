//! `--analyze`: per-listener results, the attention filter, per-stratum reporting and the decision rule.
//!
//! **The rule, fixed in advance** (docs/r3.md, "Clause 2"; the statistics are in [`super::boot`]):
//!
//! 1. A listener who got fewer than 80 % of the four attention trials right is **excluded** (a listener who can no
//!    longer reach 80 % because of misses and the trials still unanswered is excluded as well). Catch trials never
//!    enter the preference.
//! 2. The estimate is the pooled share of the remaining judgements in which arm B's candidate was called closer.
//! 3. **B is preferred when** the lower end of the 95 % cluster-bootstrap interval (references, then listeners) is
//!    above 50 % **and** the degraded subset's point estimate is at least 50 %. Strata are reported, not gated.
//! 4. The report states the observed design effect and the observed share (the plan assumed 1.2 and 65 %).
//!
//! The result is final only when every listener the key knows (the owner and each kit) has answered every trial;
//! otherwise it is marked provisional, and `enth-listen gr3 --analyze` shows nothing but the progress unless
//! `--partial` is given (the listening key stays closed until everyone is done).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::answers::Responses;
use super::boot::{BOOTSTRAP_SEED, Boot, Judgement, bootstrap};
use super::design::{ATTENTION_MIN, Kind, Manifest, Source, Trial};
use crate::stats::{Z95, two_sided, wilson};

/// The report's schema id.
pub const SCHEMA_REPORT: &str = "enth.listen.gr3-report/1";
/// The design effect the plan assumed (report 04, 3.2).
pub const PLANNED_DEFF: f64 = 1.2;
/// The preference the plan's sizing assumed.
pub const PLANNED_SHARE: f64 = 0.65;

/// One listener's results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListenerReport {
    /// Who.
    pub name: String,
    /// `owner` or `kit<N>`.
    pub audience: String,
    /// Trials in the sequence (main and catch).
    pub assigned: usize,
    /// Trials answered.
    pub answered: usize,
    /// Main trials answered.
    pub main_answered: usize,
    /// Main trials in which arm B's candidate was chosen.
    pub main_b: usize,
    /// Main trials in which the first sound (A) was chosen: with the arm order balanced, 50 % unless there is a bias.
    pub main_first_chosen: usize,
    /// Exact two-sided p-value of `main_first_chosen` out of `main_answered` against a fair coin.
    pub first_bias_p: Option<f64>,
    /// Attention trials in the sequence.
    pub attention_assigned: usize,
    /// Attention trials answered.
    pub attention_answered: usize,
    /// Attention trials answered right.
    pub attention_right: usize,
    /// Identical pairs answered.
    pub identical_answered: usize,
    /// Identical pairs in which the first sound (A) was chosen.
    pub identical_first_chosen: usize,
    /// Every trial answered.
    pub complete: bool,
    /// Excluded by the attention rule.
    pub excluded: bool,
}

/// One stratum's row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StratumRow {
    /// The stratum.
    pub name: String,
    /// References judged.
    pub references: usize,
    /// Its bootstrap (the same rule), or `None` without judgements.
    pub boot: Option<Boot>,
}

/// The decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// The lower end of the 95 % interval is above 50 %.
    pub lower_bound_above_half: bool,
    /// The degraded subset's point estimate is at least 50 % (`None`: no degraded judgement).
    pub degraded_at_least_half: Option<bool>,
    /// Arm B is preferred under the rule.
    pub b_preferred: bool,
    /// Arm A was preferred: the upper end of the interval is below 50 %.
    pub a_preferred: bool,
    /// The verdict in one line.
    pub verdict: String,
}

/// The whole report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Schema id.
    pub schema: String,
    /// The set's name.
    pub name: String,
    /// Label of arm A.
    pub arm_a: String,
    /// Label of arm B.
    pub arm_b: String,
    /// Answers produced by `--dry-run`.
    pub synthetic: bool,
    /// Every listener of the key has answered every trial.
    pub complete: bool,
    /// Audiences with no answers yet.
    pub missing: Vec<String>,
    /// Bootstrap replicates behind every interval.
    pub replicates: usize,
    /// The seed of every bootstrap.
    pub seed: u64,
    /// Per listener.
    pub listeners: Vec<ListenerReport>,
    /// The pooled bootstrap over the included listeners.
    pub pooled: Option<Boot>,
    /// Clean references only.
    pub clean: Option<Boot>,
    /// Degraded references only.
    pub degraded: Option<Boot>,
    /// Per stratum.
    pub strata: Vec<StratumRow>,
    /// Plain (Wilson) interval of the pooled share, ignoring clustering.
    pub wilson: Option<(f64, f64)>,
    /// The decision.
    pub decision: Decision,
}

fn listener_report(m: &Manifest, r: &Responses) -> ListenerReport {
    let trials = m.trials_of(&r.audience);
    let mut l = ListenerReport {
        name: r.listener.clone(),
        audience: r.audience.clone(),
        assigned: trials.len(),
        answered: 0,
        main_answered: 0,
        main_b: 0,
        main_first_chosen: 0,
        first_bias_p: None,
        attention_assigned: trials.iter().filter(|t| t.kind == Kind::Attention).count(),
        attention_answered: 0,
        attention_right: 0,
        identical_answered: 0,
        identical_first_chosen: 0,
        complete: false,
        excluded: false,
    };
    for t in &trials {
        let Some(&c) = r.choices.get(&t.n) else {
            continue;
        };
        l.answered += 1;
        match t.kind {
            Kind::Main => {
                l.main_answered += 1;
                l.main_b += usize::from(t.source_at(c) == Source::ArmB);
                l.main_first_chosen += usize::from(c == 'a');
            }
            Kind::Attention => {
                l.attention_answered += 1;
                l.attention_right += usize::from(t.right_position() == Some(c));
            }
            Kind::Identical => {
                l.identical_answered += 1;
                l.identical_first_chosen += usize::from(c == 'a');
            }
        }
    }
    l.complete = l.answered == l.assigned;
    if l.main_answered > 0 {
        l.first_bias_p = Some(two_sided(l.main_answered, l.main_first_chosen));
    }
    if l.attention_assigned > 0 {
        let best = l.attention_right + (l.attention_assigned - l.attention_answered);
        l.excluded = (best as f64 / l.attention_assigned as f64) < ATTENTION_MIN;
    }
    l
}

/// The judgements of the included listeners: main trials only.
#[must_use]
pub fn judgements(
    m: &Manifest,
    responses: &[Responses],
    listeners: &[ListenerReport],
) -> Vec<Judgement> {
    let mut out = Vec::new();
    for (r, l) in responses.iter().zip(listeners) {
        if l.excluded {
            continue;
        }
        let trials: Vec<&Trial> = m.trials_of(&r.audience);
        for t in trials.iter().filter(|t| t.kind == Kind::Main) {
            let Some(&c) = r.choices.get(&t.n) else {
                continue;
            };
            let item = m.item(&t.item).expect("the key lists its trials' items");
            out.push(Judgement {
                listener: r.listener.clone(),
                item: item.id.clone(),
                cluster: item.cluster.clone(),
                stratum: item.stratum.clone(),
                degraded: item.degraded,
                b: t.source_at(c) == Source::ArmB,
            });
        }
    }
    out
}

fn decide(pooled: Option<&Boot>, degraded: Option<&Boot>, complete: bool) -> Decision {
    let Some(p) = pooled else {
        return Decision {
            lower_bound_above_half: false,
            degraded_at_least_half: None,
            b_preferred: false,
            a_preferred: false,
            verdict:
                "NO RESULT: no valid judgements (every listener was excluded, or none has answered)"
                    .into(),
        };
    };
    let lb_ok = p.lo > 0.5;
    let deg_ok = degraded.map(|d| d.p >= 0.5);
    let b_preferred = lb_ok && deg_ok == Some(true);
    let a_preferred = p.hi < 0.5;
    let mut why = Vec::new();
    if !lb_ok {
        why.push(format!(
            "the lower bound {:.1}% is not above 50%",
            100.0 * p.lo
        ));
    }
    match deg_ok {
        Some(false) => why.push(format!(
            "the degraded subset's point estimate {:.1}% is below 50%",
            100.0 * degraded.map_or(0.0, |d| d.p)
        )),
        None => why.push("there is no degraded judgement to check".into()),
        Some(true) => {}
    }
    let head = if b_preferred {
        "B PREFERRED".to_string()
    } else if a_preferred {
        "B NOT PREFERRED (arm A was preferred: the upper bound is below 50%)".to_string()
    } else {
        format!("B NOT PREFERRED ({})", why.join("; "))
    };
    let verdict = if complete {
        head
    } else {
        format!("{head}; PROVISIONAL, not every listener has finished")
    };
    Decision {
        lower_bound_above_half: lb_ok,
        degraded_at_least_half: deg_ok,
        b_preferred,
        a_preferred,
        verdict,
    }
}

/// The bootstraps of one set of judgements: pooled, clean, degraded and each stratum. This is the function the Python
/// reference is compared with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Everything.
    pub pooled: Option<Boot>,
    /// Clean references only.
    pub clean: Option<Boot>,
    /// Degraded references only.
    pub degraded: Option<Boot>,
    /// Per stratum, by name.
    pub strata: Vec<StratumRow>,
}

/// Bootstrap `js` and its subsets with the fixed seed.
#[must_use]
pub fn summarize(js: &[Judgement], replicates: usize) -> Summary {
    let pick = |f: &dyn Fn(&Judgement) -> bool| -> Option<Boot> {
        let v: Vec<&Judgement> = js.iter().filter(|j| f(j)).collect();
        bootstrap(&v, replicates, BOOTSTRAP_SEED)
    };
    let names: BTreeSet<&str> = js.iter().map(|j| j.stratum.as_str()).collect();
    Summary {
        pooled: pick(&|_| true),
        clean: pick(&|j| !j.degraded),
        degraded: pick(&|j| j.degraded),
        strata: names
            .into_iter()
            .map(|s| StratumRow {
                name: s.to_string(),
                references: js
                    .iter()
                    .filter(|j| j.stratum == s)
                    .map(|j| j.item.as_str())
                    .collect::<BTreeSet<_>>()
                    .len(),
                boot: pick(&|j| j.stratum == s),
            })
            .collect(),
    }
}

/// Analyze the answers against the key.
#[must_use]
pub fn analyze(m: &Manifest, responses: &[Responses], replicates: usize) -> Report {
    let listeners: Vec<ListenerReport> = responses.iter().map(|r| listener_report(m, r)).collect();
    let js = judgements(m, responses, &listeners);
    let Summary {
        pooled,
        clean,
        degraded,
        strata,
    } = summarize(&js, replicates);
    let seen: BTreeSet<&str> = responses.iter().map(|r| r.audience.as_str()).collect();
    let missing: Vec<String> = m
        .audiences()
        .into_iter()
        .filter(|a| !seen.contains(a.as_str()))
        .collect();
    let complete = missing.is_empty() && listeners.iter().all(|l| l.complete);
    let decision = decide(pooled.as_ref(), degraded.as_ref(), complete);
    Report {
        schema: SCHEMA_REPORT.into(),
        name: m.name.clone(),
        arm_a: m.arm_a.clone(),
        arm_b: m.arm_b.clone(),
        synthetic: responses.iter().any(|r| r.synthetic),
        complete,
        missing,
        replicates: pooled.as_ref().map_or(replicates, |p| p.replicates),
        seed: BOOTSTRAP_SEED,
        listeners,
        wilson: pooled
            .as_ref()
            .map(|p| wilson(p.wins as usize, p.n as usize, Z95)),
        pooled,
        clean,
        degraded,
        strata,
        decision,
    }
}

fn pct(x: f64) -> String {
    format!("{:.1}%", 100.0 * x)
}

fn interval(lo: f64, hi: f64) -> String {
    format!("[{}, {}]", pct(lo), pct(hi))
}

fn boot_cells(b: Option<&Boot>) -> String {
    b.map_or_else(
        || "none".to_string(),
        |b| {
            format!(
                "{} of {} = {}, 95% {}",
                b.wins,
                b.n,
                pct(b.p),
                interval(b.lo, b.hi)
            )
        },
    )
}

impl Report {
    /// The progress of every listener, without any result (what `--analyze` shows while someone is not done).
    #[must_use]
    pub fn progress_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "GR3 listening progress");
        for l in &self.listeners {
            let _ = writeln!(
                s,
                "  {:<12} {}/{} trials answered",
                l.name, l.answered, l.assigned
            );
        }
        for a in &self.missing {
            let _ = writeln!(
                s,
                "  {a:<12} no answers yet (put the returned file in friends/)"
            );
        }
        s
    }

    /// The report as text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let tag = if self.synthetic {
            "   [SYNTHETIC: generated answers, nobody listened; do not rely on the numbers]"
        } else {
            ""
        };
        let _ = writeln!(
            s,
            "GR3 blind listening: arm B ({}) against arm A ({}){tag}",
            self.arm_b, self.arm_a
        );
        if !self.name.is_empty() {
            let _ = writeln!(s, "set: {}", self.name);
        }
        let _ = writeln!(
            s,
            "\nListeners (first sound = A, second = B; the arm order is balanced, so about half should pick the first)"
        );
        let _ = writeln!(
            s,
            "  {:<10} {:>9} {:>6} {:>9} {:>10} {:>15}  status",
            "listener", "answered", "main", "attention", "identical", "first chosen"
        );
        for l in &self.listeners {
            let status = if l.excluded {
                format!(
                    "EXCLUDED: attention {}/{} under {:.0}%",
                    l.attention_right,
                    l.attention_assigned,
                    100.0 * ATTENTION_MIN
                )
            } else if l.complete {
                "included".to_string()
            } else {
                format!("included, {} trials not answered", l.assigned - l.answered)
            };
            let bias = l.first_bias_p.map_or_else(
                || "-".to_string(),
                |p| format!("{}/{} p={p:.2}", l.main_first_chosen, l.main_answered),
            );
            let _ = writeln!(
                s,
                "  {:<10} {:>9} {:>6} {:>6}/{:<2} {:>6}/{:<3} {:>15}  {status}",
                l.name,
                format!("{}/{}", l.answered, l.assigned),
                l.main_answered,
                l.attention_right,
                l.attention_assigned,
                l.identical_first_chosen,
                l.identical_answered,
                bias,
            );
        }
        let _ = writeln!(
            s,
            "  (identical: how many of the identical pairs went to the first sound)"
        );
        for a in &self.missing {
            let _ = writeln!(s, "  {a}: no answers yet");
        }
        let Some(p) = &self.pooled else {
            let _ = writeln!(s, "\nDecision: {}", self.decision.verdict);
            return s;
        };
        let _ = writeln!(
            s,
            "\nResult: arm B closer to the reference in {} of {} judgements = {} ({} clusters, {} listeners)",
            p.wins,
            p.n,
            pct(p.p),
            p.clusters,
            p.listeners
        );
        let _ = writeln!(
            s,
            "  95% cluster bootstrap (references, then listeners; {} replicates, seed {:#x}): {}",
            self.replicates,
            self.seed,
            interval(p.lo, p.hi)
        );
        let _ = writeln!(
            s,
            "    lower bound {} {} 50%: {}",
            pct(p.lo),
            if p.lo > 0.5 { ">" } else { "<=" },
            if p.lo > 0.5 { "yes" } else { "no" }
        );
        let _ = writeln!(
            s,
            "  references only (not the rule): {}",
            interval(p.lo_ref, p.hi_ref)
        );
        if let Some((lo, hi)) = self.wilson {
            let _ = writeln!(
                s,
                "  plain binomial, ignoring clustering (Wilson): {}",
                interval(lo, hi)
            );
        }
        match p.deff {
            Some(d) => {
                let _ = writeln!(
                    s,
                    "  design effect observed {d:.2} (the plan assumed {PLANNED_DEFF}); effective n {:.0} of {}; observed share {} (the plan assumed {})",
                    p.n as f64 / d,
                    p.n,
                    pct(p.p),
                    pct(PLANNED_SHARE)
                );
            }
            None => {
                let _ = writeln!(s, "  design effect: not defined (the share is 0% or 100%)");
            }
        }
        if let Some(d2) = p.deff_two_stage {
            let _ = writeln!(
                s,
                "    the rule's interval also redraws listeners inside each reference ({d2:.2} on this measure), which makes it the more cautious"
            );
        }
        let _ = writeln!(
            s,
            "  clean references:    {}",
            boot_cells(self.clean.as_ref())
        );
        let _ = writeln!(
            s,
            "  degraded references: {}",
            boot_cells(self.degraded.as_ref())
        );
        match self.decision.degraded_at_least_half {
            Some(ok) => {
                let _ = writeln!(
                    s,
                    "    degraded point estimate {} at least 50%: {}",
                    pct(self.degraded.as_ref().map_or(0.0, |d| d.p)),
                    if ok { "yes" } else { "no" }
                );
            }
            None => {
                let _ = writeln!(s, "    no degraded judgement");
            }
        }
        let _ = writeln!(s, "\nDecision: {}", self.decision.verdict);
        let _ = writeln!(s, "\nStrata (reported, not gated)");
        let _ = writeln!(
            s,
            "  {:<24} {:>5} {:>9} {:>8}  95% cluster bootstrap",
            "stratum", "refs", "judgem.", "arm B"
        );
        for r in &self.strata {
            if let Some(b) = &r.boot {
                let _ = writeln!(
                    s,
                    "  {:<24} {:>5} {:>9} {:>8}  {}",
                    r.name,
                    r.references,
                    b.n,
                    pct(b.p),
                    interval(b.lo, b.hi)
                );
            }
        }
        let mut per: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for l in self.listeners.iter().filter(|l| !l.excluded) {
            let e = per.entry(l.name.as_str()).or_default();
            e.0 += l.main_b;
            e.1 += l.main_answered;
        }
        let _ = writeln!(s, "\nArm B's share per included listener");
        for (name, (b, n)) in per {
            let _ = writeln!(
                s,
                "  {name:<12} {b} of {n} = {}",
                pct(if n > 0 { b as f64 / n as f64 } else { 0.0 })
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gr3::design::{DEFAULT_SEED, OWNER, design};
    use crate::gr3::dry::{demo_set, synthetic_responses};
    use crate::gr3::set::Set;

    fn key(name: &str, refs: usize) -> (std::path::PathBuf, Manifest) {
        let d = std::env::temp_dir().join(format!(
            "enth-listen-gr3analyze-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        let set = Set::load(&demo_set(&d, refs).unwrap()).unwrap();
        let m = design(&set, DEFAULT_SEED, 2, &d.join("out")).unwrap();
        (d, m)
    }

    /// Everyone answers every main trial with arm B (`b`) or arm A, and the catch trials right.
    fn uniform_responses(m: &Manifest, audience: &str, name: &str, prefer_b: bool) -> Responses {
        let choices = m
            .trials_of(audience)
            .iter()
            .map(|t| {
                let want = |s: Source| if t.pos_a == s { 'a' } else { 'b' };
                let c = match t.kind {
                    Kind::Main => want(if prefer_b { Source::ArmB } else { Source::ArmA }),
                    Kind::Attention => t.right_position().unwrap(),
                    Kind::Identical => 'a',
                };
                (t.n, c)
            })
            .collect();
        Responses {
            listener: name.into(),
            audience: audience.into(),
            synthetic: false,
            choices,
        }
    }

    #[test]
    fn an_owner_who_always_picks_b_prefers_b_and_the_gate_reads_both_conditions() {
        let (d, m) = key("allb", 12);
        let r = analyze(&m, &[uniform_responses(&m, OWNER, "owner", true)], 500);
        assert!(r.decision.b_preferred, "{}", r.to_text());
        assert_eq!(r.pooled.as_ref().unwrap().p, 1.0);
        assert!(
            r.decision.verdict.contains("PROVISIONAL"),
            "kits have not answered: {}",
            r.decision.verdict
        );
        assert_eq!(r.missing, vec!["kit1".to_string(), "kit2".to_string()]);
        // always arm A: not preferred, and A is
        let r = analyze(&m, &[uniform_responses(&m, OWNER, "owner", false)], 500);
        assert!(
            !r.decision.b_preferred && r.decision.a_preferred,
            "{}",
            r.decision.verdict
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_degraded_condition_can_veto_a_strong_overall_result() {
        let (d, m) = key("veto", 60);
        // B on every clean reference, A on every degraded one: overall 2/3 B with a high lower bound, degraded 0
        let mut resp = uniform_responses(&m, OWNER, "owner", true);
        for t in m.trials_of(OWNER).iter().filter(|t| t.kind == Kind::Main) {
            if m.item(&t.item).unwrap().degraded {
                resp.choices
                    .insert(t.n, if t.pos_a == Source::ArmA { 'a' } else { 'b' });
            }
        }
        let r = analyze(&m, &[resp], 500);
        assert!(r.decision.lower_bound_above_half, "{}", r.to_text());
        assert_eq!(r.decision.degraded_at_least_half, Some(false));
        assert!(
            !r.decision.b_preferred && r.decision.verdict.contains("degraded subset"),
            "{}",
            r.decision.verdict
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn attention_under_80_percent_excludes_a_listener_from_the_preference() {
        let (d, m) = key("exclude", 12);
        let good = uniform_responses(&m, OWNER, "owner", false);
        // kit1 always picks B but misses one of the four attention trials: 3/4 = 75% < 80%
        let mut bad = uniform_responses(&m, "kit1", "friend", true);
        let t = m
            .trials_of("kit1")
            .into_iter()
            .find(|t| t.kind == Kind::Attention)
            .unwrap();
        bad.choices.insert(
            t.n,
            if t.right_position() == Some('a') {
                'b'
            } else {
                'a'
            },
        );
        let r = analyze(&m, &[good.clone(), bad.clone()], 300);
        let f = r.listeners.iter().find(|l| l.name == "friend").unwrap();
        assert!(f.excluded && f.attention_right == 3);
        // the excluded friend's all-B judgements are out: the pooled share is the owner's alone (all A)
        assert_eq!(r.pooled.as_ref().unwrap().p, 0.0);
        assert_eq!(r.pooled.as_ref().unwrap().listeners, 1);
        assert!(r.to_text().contains("EXCLUDED"));
        // all four right: included
        let mut ok = bad.clone();
        ok.choices.insert(t.n, t.right_position().unwrap());
        let r = analyze(&m, &[good, ok], 300);
        assert!(!r.listeners[1].excluded);
        assert_eq!(r.pooled.as_ref().unwrap().listeners, 2);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unanswered_trials_keep_a_listener_in_until_the_attention_rule_cannot_be_met() {
        let (d, m) = key("partial", 12);
        let mut r = uniform_responses(&m, OWNER, "owner", true);
        let attn: Vec<usize> = m
            .trials_of(OWNER)
            .iter()
            .filter(|t| t.kind == Kind::Attention)
            .map(|t| t.n)
            .collect();
        // two attention trials unanswered, the two answered right: still reachable (4/4 possible)
        r.choices.remove(&attn[0]);
        r.choices.remove(&attn[1]);
        let rep = analyze(&m, &[r.clone()], 200);
        assert!(!rep.listeners[0].excluded && !rep.listeners[0].complete);
        // one answered wrong, one unanswered: at best 3/4 = 75%, already out
        let t = m
            .trials_of(OWNER)
            .into_iter()
            .find(|t| t.n == attn[2])
            .unwrap();
        r.choices.insert(
            t.n,
            if t.right_position() == Some('a') {
                'b'
            } else {
                'a'
            },
        );
        let rep = analyze(&m, &[r], 200);
        assert!(rep.listeners[0].excluded);
        assert!(rep.decision.verdict.starts_with("NO RESULT"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_synthetic_dry_run_listeners_give_a_full_report() {
        let (d, m) = key("dry", 60);
        let resp: Vec<Responses> = m
            .audiences()
            .iter()
            .map(|a| synthetic_responses(&m, a, a, 7))
            .collect();
        let r = analyze(&m, &resp, 1000);
        assert!(r.synthetic && r.complete);
        // kit2 misses an attention trial on purpose
        assert!(
            r.listeners
                .iter()
                .find(|l| l.name == "kit2")
                .unwrap()
                .excluded
        );
        assert!(
            !r.listeners
                .iter()
                .find(|l| l.name == "owner")
                .unwrap()
                .excluded
        );
        let p = r.pooled.as_ref().unwrap();
        assert_eq!(
            p.clusters, 40,
            "60 references: 20 degraded copies share their clean original's cluster"
        );
        assert_eq!(r.strata.len(), 4);
        let text = r.to_text();
        assert!(
            text.contains("SYNTHETIC") && text.contains("design effect") && text.contains("Strata"),
            "{text}"
        );
        assert!(r.clean.is_some() && r.degraded.is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
