//! `--analyze` for the reverb test: preference with confidence, per pair and per source.

use std::fmt::Write as _;

use serde::Serialize;

use super::{Calibration, Candidate, Manifest, SCHEMA_REPORT};
use crate::flags::summary;
use crate::session::AnswerFile;
use crate::stats::{Z95, two_sided, wilson};

/// Preference between two reverbs over a set of trials.
#[derive(Clone, Debug, Serialize)]
pub struct PairRow {
    /// The first-named reverb.
    pub first: Candidate,
    /// The second-named reverb.
    pub second: Candidate,
    /// The source, or `None` for all sources pooled.
    pub source: Option<String>,
    /// Trials the first was preferred.
    pub first_wins: usize,
    /// Trials the second was preferred.
    pub second_wins: usize,
    /// Trials answered "no preference".
    pub ties: usize,
    /// `first_wins / (first_wins + second_wins)`.
    pub first_rate: Option<f64>,
    /// 95 % Wilson interval of that rate.
    pub ci: Option<(f64, f64)>,
    /// Exact two-sided sign-test p-value against 50 %.
    pub p: Option<f64>,
}

/// One reverb's record over every trial it was in.
#[derive(Clone, Debug, Serialize)]
pub struct Score {
    /// The reverb.
    pub candidate: Candidate,
    /// Trials preferred.
    pub wins: usize,
    /// Trials not preferred.
    pub losses: usize,
    /// Trials with no preference.
    pub ties: usize,
    /// Wins over decided trials.
    pub rate: Option<f64>,
}

/// How often repeats of the same (source, pair) agree with their own majority.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Consistency {
    /// Mean share of repeats agreeing with the majority choice.
    pub observed: f64,
    /// What coin flipping gives for the same number of repeats.
    pub chance: f64,
}

/// Loudness bookkeeping of one clip.
#[derive(Clone, Debug, Serialize)]
pub struct LevelRow {
    /// Source.
    pub source: String,
    /// Reverb.
    pub candidate: Candidate,
    /// Wet-level correction, dB.
    pub wet_match_db: f64,
    /// Overall gain, dB.
    pub final_gain_db: f64,
    /// Final integrated loudness, LUFS.
    pub lufs: f64,
    /// Final RMS, dBFS.
    pub rms_dbfs: f64,
}

/// The `--analyze` result (also written to `reverb-report.json`).
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    /// [`SCHEMA_REPORT`].
    pub schema: String,
    /// True when the answers are synthetic (dry run).
    pub synthetic: bool,
    /// Trials answered.
    pub answered: usize,
    /// Trials in the session.
    pub planned: usize,
    /// Per pair, all sources pooled.
    pub pooled: Vec<PairRow>,
    /// Per pair and source.
    pub by_source: Vec<PairRow>,
    /// Per reverb.
    pub scores: Vec<Score>,
    /// Share of answers that were "a" (50 % is balanced; ties excluded).
    pub a_share: f64,
    /// Agreement between repeats.
    pub consistency: Consistency,
    /// Level bookkeeping.
    pub levels: Vec<LevelRow>,
    /// How the decays were calibrated.
    pub calibration: Vec<Calibration>,
    /// What the result means for ADR-0006, in words.
    pub verdicts: Vec<String>,
    /// Clips with click or discontinuity flags.
    pub flagged: Vec<String>,
}

/// Expected share of repeats agreeing with their majority when each is a fair coin flip.
fn chance_consistency(r: usize) -> f64 {
    if r == 0 {
        return 1.0;
    }
    (0..=r)
        .map(|k| crate::stats::pmf(r, k, 0.5) * k.max(r - k) as f64 / r as f64)
        .sum()
}

fn row(
    first: Candidate,
    second: Candidate,
    source: Option<&str>,
    w1: usize,
    w2: usize,
    ties: usize,
) -> PairRow {
    let n = w1 + w2;
    PairRow {
        first,
        second,
        source: source.map(str::to_string),
        first_wins: w1,
        second_wins: w2,
        ties,
        first_rate: (n > 0).then(|| w1 as f64 / n as f64),
        ci: (n > 0).then(|| wilson(w1, n, Z95)),
        p: (n > 0).then(|| two_sided(n, w1)),
    }
}

/// Analyse the answers (a partial session is fine).
#[must_use]
pub fn analyze(m: &Manifest, files: &[AnswerFile]) -> Report {
    // (trial index, choice)
    let mut answered: Vec<(usize, &str)> = Vec::new();
    for f in files {
        for a in &f.answers {
            if let Some(i) = m.trials.iter().position(|t| t.id == a.id) {
                answered.push((i, a.choice.as_str()));
            }
        }
    }
    // outcome of one trial: Some(winner) / None for a tie
    let winner = |i: usize, choice: &str| -> Option<Candidate> {
        let t = &m.trials[i];
        match choice {
            "a" => Some(t.a_candidate),
            "b" => Some(t.b_candidate),
            _ => None,
        }
    };
    let pairs = [
        (Candidate::Ours, Candidate::Reverb2),
        (Candidate::Ours, Candidate::Reverb4),
        (Candidate::Reverb2, Candidate::Reverb4),
    ];
    let tally = |first: Candidate, second: Candidate, source: Option<&str>| {
        let (mut w1, mut w2, mut ties) = (0, 0, 0);
        for &(i, c) in &answered {
            let t = &m.trials[i];
            let same_pair = (t.a_candidate == first && t.b_candidate == second)
                || (t.a_candidate == second && t.b_candidate == first);
            if !same_pair || source.is_some_and(|s| s != t.source) {
                continue;
            }
            match winner(i, c) {
                Some(w) if w == first => w1 += 1,
                Some(_) => w2 += 1,
                None => ties += 1,
            }
        }
        row(first, second, source, w1, w2, ties)
    };
    let pooled: Vec<PairRow> = pairs.iter().map(|&(a, b)| tally(a, b, None)).collect();
    let by_source: Vec<PairRow> = m
        .sources
        .iter()
        .flat_map(|s| pairs.iter().map(move |&(a, b)| (s.name.clone(), a, b)))
        .map(|(s, a, b)| tally(a, b, Some(&s)))
        .collect();
    let scores: Vec<Score> = Candidate::ALL
        .iter()
        .map(|&c| {
            let (mut wins, mut losses, mut ties) = (0, 0, 0);
            for &(i, ch) in &answered {
                let t = &m.trials[i];
                if t.a_candidate != c && t.b_candidate != c {
                    continue;
                }
                match winner(i, ch) {
                    Some(w) if w == c => wins += 1,
                    Some(_) => losses += 1,
                    None => ties += 1,
                }
            }
            Score {
                candidate: c,
                wins,
                losses,
                ties,
                rate: (wins + losses > 0).then(|| wins as f64 / (wins + losses) as f64),
            }
        })
        .collect();
    let decided: Vec<&(usize, &str)> = answered.iter().filter(|(_, c)| *c != "=").collect();
    let a_share = if decided.is_empty() {
        0.5
    } else {
        decided.iter().filter(|(_, c)| *c == "a").count() as f64 / decided.len() as f64
    };
    // consistency: repeats of one (source, unordered pair) vs their own majority, by candidate preferred
    let mut groups: std::collections::BTreeMap<(String, Candidate, Candidate), Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for &(i, c) in &answered {
        if let Some(w) = winner(i, c) {
            let t = &m.trials[i];
            let (lo, hi) = if t.a_candidate < t.b_candidate {
                (t.a_candidate, t.b_candidate)
            } else {
                (t.b_candidate, t.a_candidate)
            };
            groups
                .entry((t.source.clone(), lo, hi))
                .or_default()
                .push(w);
        }
    }
    let shares: Vec<(f64, usize)> = groups
        .iter()
        .filter(|(_, v)| v.len() >= 2)
        .map(|((_, lo, _), v)| {
            let k = v.iter().filter(|w| **w == *lo).count();
            (k.max(v.len() - k) as f64 / v.len() as f64, v.len())
        })
        .collect();
    let consistency = Consistency {
        observed: if shares.is_empty() {
            1.0
        } else {
            shares.iter().map(|s| s.0).sum::<f64>() / shares.len() as f64
        },
        chance: if shares.is_empty() {
            1.0
        } else {
            shares.iter().map(|s| chance_consistency(s.1)).sum::<f64>() / shares.len() as f64
        },
    };
    let levels: Vec<LevelRow> = m
        .clips
        .iter()
        .map(|c| LevelRow {
            source: c.source.clone(),
            candidate: c.candidate,
            wet_match_db: c.wet_match_db,
            final_gain_db: c.final_gain_db,
            lufs: c.lufs,
            rms_dbfs: c.rms_dbfs,
        })
        .collect();
    let flagged: Vec<String> = m
        .clips
        .iter()
        .filter(|c| !c.flags.is_empty())
        .map(|c| {
            format!(
                "{} {}: {}",
                c.source,
                c.candidate.label(),
                summary(&c.flags)
            )
        })
        .collect();
    let mut verdicts = Vec::new();
    for r in pooled.iter().filter(|r| r.first == Candidate::Ours) {
        let other = r.second.long_label();
        verdicts.push(match (r.first_rate, r.ci, r.p) {
            (Some(rate), Some((lo, hi)), Some(p)) => {
                let head = format!("ours vs {other}: ours preferred in {} of {} decided trials ({:.0} %, 95 % CI {:.0}-{:.0} %, p = {p:.3})", r.first_wins, r.first_wins + r.second_wins, 100.0 * rate, 100.0 * lo, 100.0 * hi);
                if p <= 0.05 && rate > 0.5 {
                    format!("{head}: ours is preferred")
                } else if p <= 0.05 {
                    format!("{head}: {other} is preferred; this is ADR-0006's condition 'the port loses the blind A/B'")
                } else {
                    format!("{head}: no clear preference")
                }
            }
            _ => format!("ours vs {other}: no decided trials yet"),
        });
    }
    for c in &m.calibration {
        let off = (c.measured_rt60_s - m.config.rt60_s) / m.config.rt60_s;
        if off.abs() > 0.1 {
            verdicts.push(format!(
                "{} decays {:.2} s against the {:.2} s target ({:+.0} %): the decay match is loose for it, which favours or hurts it in the A/B",
                c.candidate.long_label(),
                c.measured_rt60_s,
                m.config.rt60_s,
                100.0 * off
            ));
        }
    }
    let total_decided: usize = pooled.iter().map(|r| r.first_wins + r.second_wins).sum();
    let total_ties: usize = pooled.iter().map(|r| r.ties).sum();
    if total_ties * 2 > total_decided + total_ties && total_ties > 0 {
        verdicts.push(format!("{total_ties} of {} answers were 'no preference': the reverbs may be too close to separate at this decay and wet level", total_decided + total_ties));
    }
    Report {
        schema: SCHEMA_REPORT.to_string(),
        synthetic: files.iter().any(|f| f.synthetic),
        answered: answered.len(),
        planned: m.trials.len(),
        pooled,
        by_source,
        scores,
        a_share,
        consistency,
        levels,
        calibration: m.calibration.clone(),
        verdicts,
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
            "reverb A/B analysis: {} of {} trials answered{}",
            self.answered,
            self.planned,
            if self.synthetic {
                "  ** SYNTHETIC ANSWERS (dry run), not the owner's **"
            } else {
                ""
            }
        );
        let _ = writeln!(s, "calibration (RT60 matched across the three):");
        for c in &self.calibration {
            let _ = writeln!(
                s,
                "  {:<8} {} = {:.3} -> RT60 {:.2} s",
                c.candidate.label(),
                c.parameter,
                c.value,
                c.measured_rt60_s
            );
        }
        let fmt = |r: &PairRow| match (r.first_rate, r.ci, r.p) {
            (Some(rate), Some((lo, hi)), Some(p)) => format!(
                "{:>3.0} % [{:>3.0}, {:>3.0}]  p = {p:.3}",
                100.0 * rate,
                100.0 * lo,
                100.0 * hi
            ),
            _ => "-".to_string(),
        };
        let _ = writeln!(
            s,
            "\n{:<20} {:>6} {:>6} {:>5}  first-named preferred [95 % CI]",
            "pair (all sources)", "first", "second", "ties"
        );
        for r in &self.pooled {
            let _ = writeln!(
                s,
                "{:<20} {:>6} {:>6} {:>5}  {}",
                format!("{} vs {}", r.first.label(), r.second.label()),
                r.first_wins,
                r.second_wins,
                r.ties,
                fmt(r)
            );
        }
        let _ = writeln!(
            s,
            "\n{:<20} {:<10} {:>6} {:>6} {:>5}  first-named preferred [95 % CI]",
            "pair", "source", "first", "second", "ties"
        );
        for r in &self.by_source {
            let _ = writeln!(
                s,
                "{:<20} {:<10} {:>6} {:>6} {:>5}  {}",
                format!("{} vs {}", r.first.label(), r.second.label()),
                r.source.as_deref().unwrap_or("-"),
                r.first_wins,
                r.second_wins,
                r.ties,
                fmt(r)
            );
        }
        let _ = writeln!(
            s,
            "\noverall record (wins / losses / ties, win rate of decided):"
        );
        for sc in &self.scores {
            let _ = writeln!(
                s,
                "  {:<8} {:>3} / {:>3} / {:>3}   {}",
                sc.candidate.label(),
                sc.wins,
                sc.losses,
                sc.ties,
                sc.rate
                    .map_or("-".to_string(), |r| format!("{:.0} %", 100.0 * r))
            );
        }
        let _ = writeln!(
            s,
            "\nposition: chose A in {:.0} % of decided trials (50 % is balanced)",
            100.0 * self.a_share
        );
        let _ = writeln!(
            s,
            "repeats: {:.0} % agree with their own majority (coin flipping gives {:.0} %)",
            100.0 * self.consistency.observed,
            100.0 * self.consistency.chance
        );
        let worst = self
            .levels
            .iter()
            .map(|l| l.source.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|src| {
                let v: Vec<f64> = self
                    .levels
                    .iter()
                    .filter(|l| l.source == src)
                    .map(|l| l.lufs)
                    .collect();
                v.iter().fold(f64::MIN, |a, b| a.max(*b))
                    - v.iter().fold(f64::MAX, |a, b| a.min(*b))
            })
            .fold(0.0, f64::max);
        let _ = writeln!(
            s,
            "levels: the three clips of a source differ by at most {worst:.2} LU in integrated loudness; wet-level corrections applied to the fundsp tails:"
        );
        for l in self
            .levels
            .iter()
            .filter(|l| l.candidate != Candidate::Ours)
        {
            let _ = writeln!(
                s,
                "  {:<10} {:<8} wet {:+.1} dB, overall {:+.1} dB",
                l.source,
                l.candidate.label(),
                l.wet_match_db,
                l.final_gain_db
            );
        }
        let _ = writeln!(s, "\nreading:");
        for v in &self.verdicts {
            let _ = writeln!(s, "  - {v}");
        }
        let _ = writeln!(
            s,
            "  - p-values treat every trial as independent. Repeats of one (source, pair) test your consistency, not new material: three sources are a thin base for 'always'."
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
    use super::super::{ClipInfo, Config, SCHEMA_ANSWERS, SCHEMA_MANIFEST, SourceInfo, Trial};
    use super::*;
    use crate::session::Answer;

    fn manifest(repeats: usize) -> Manifest {
        let mut clips = Vec::new();
        for s in ["pad", "pluck"] {
            for c in Candidate::ALL {
                clips.push(ClipInfo {
                    id: format!("{s}-{}", c.label()),
                    source: s.into(),
                    candidate: c,
                    wet_match_db: 0.0,
                    final_gain_db: 0.0,
                    lufs: -23.0,
                    peak_dbfs: -6.0,
                    rms_dbfs: -27.0,
                    flags: vec![],
                });
            }
        }
        let pairs = [
            (Candidate::Ours, Candidate::Reverb2),
            (Candidate::Ours, Candidate::Reverb4),
            (Candidate::Reverb2, Candidate::Reverb4),
        ];
        let mut trials = Vec::new();
        for s in ["pad", "pluck"] {
            for (f, g) in pairs {
                for r in 0..repeats {
                    let (a, b) = if r % 2 == 0 { (f, g) } else { (g, f) };
                    trials.push(Trial {
                        n: trials.len() + 1,
                        id: format!("{s}-{}-{}-{r}", f.label(), g.label()),
                        source: s.into(),
                        repeat: r + 1,
                        a: format!("{s}-{}", a.label()),
                        b: format!("{s}-{}", b.label()),
                        a_candidate: a,
                        b_candidate: b,
                    });
                }
            }
        }
        Manifest {
            schema: SCHEMA_MANIFEST.into(),
            config: Config {
                seed: 1,
                sample_rate: 48_000,
                rt60_s: 2.0,
                wet_db: -6.0,
                repeats,
                tail_rt60s: 1.5,
            },
            sources: ["pad", "pluck"]
                .iter()
                .map(|s| SourceInfo {
                    name: (*s).into(),
                    description: String::new(),
                    seconds: 5.0,
                })
                .collect(),
            calibration: vec![],
            clips,
            trials,
        }
    }

    /// Answer every trial by a preference order `rank` (higher wins), with no ties.
    fn answers(m: &Manifest, rank: impl Fn(Candidate) -> i32) -> AnswerFile {
        let mut f = AnswerFile::new(SCHEMA_ANSWERS, 1, false);
        for t in &m.trials {
            let choice = if rank(t.a_candidate) >= rank(t.b_candidate) {
                "a"
            } else {
                "b"
            };
            f.answers.push(Answer {
                n: t.n,
                id: t.id.clone(),
                choice: choice.into(),
                replays: 0,
                ms: 0,
            });
        }
        f
    }

    #[test]
    fn a_consistent_preference_is_significant_and_names_the_loser() {
        let m = manifest(6);
        // reverb4 > ours > reverb2, always
        let r = analyze(
            &m,
            &[answers(&m, |c| match c {
                Candidate::Reverb4 => 3,
                Candidate::Ours => 2,
                Candidate::Reverb2 => 1,
            })],
        );
        let ours_v_r2 = &r.pooled[0];
        assert_eq!(
            (ours_v_r2.first_wins, ours_v_r2.second_wins, ours_v_r2.ties),
            (12, 0, 0)
        );
        assert!(ours_v_r2.p.unwrap() < 0.001);
        let ours_v_r4 = &r.pooled[1];
        assert_eq!((ours_v_r4.first_wins, ours_v_r4.second_wins), (0, 12));
        assert!(
            r.verdicts
                .iter()
                .any(|v| v.contains("reverb4_stereo is preferred") && v.contains("ADR-0006")),
            "{:?}",
            r.verdicts
        );
        assert!(r.verdicts.iter().any(|v| v.contains("ours is preferred")));
        assert_eq!(r.scores[0].wins, 12);
        assert!((r.consistency.observed - 1.0).abs() < 1e-12 && r.consistency.chance < 0.7);
        assert_eq!(r.by_source.len(), 6);
        assert!(r.to_text().contains("reverb2 vs reverb4"));
    }

    #[test]
    fn no_preference_means_no_clear_preference() {
        let m = manifest(6);
        // always pick A: positions balanced across repeats, so every pair ends 6-6
        let mut f = answers(&m, |_| 0);
        for a in &mut f.answers {
            a.choice = "a".into();
        }
        let r = analyze(&m, &[f]);
        assert!(r.pooled.iter().all(|p| p.first_wins == p.second_wins));
        assert!((r.a_share - 1.0).abs() < 1e-12);
        assert!(
            r.verdicts.iter().all(|v| v.contains("no clear preference")),
            "{:?}",
            r.verdicts
        );
    }

    #[test]
    fn ties_partial_answers_and_empty_input_are_handled() {
        let m = manifest(2);
        let mut f = answers(&m, |c| i32::from(c == Candidate::Ours));
        f.answers.truncate(5);
        f.answers[0].choice = "=".into();
        let r = analyze(&m, &[f]);
        assert_eq!(r.answered, 5);
        assert_eq!(r.pooled.iter().map(|p| p.ties).sum::<usize>(), 1);
        let empty = analyze(&m, &[]);
        assert_eq!(empty.answered, 0);
        assert!(empty.pooled.iter().all(|p| p.first_rate.is_none()));
        assert!(empty.verdicts[0].contains("no decided trials"));
        let _ = empty.to_text();
    }

    #[test]
    fn chance_consistency_matches_hand_values() {
        // two coin flips agree half the time; 4 flips: (2*1 + 8*0.75 + 6*1... ) = E[max(k,4-k)/4] = 0.6875
        assert!((chance_consistency(2) - 0.75).abs() < 1e-12);
        assert!((chance_consistency(4) - 0.6875).abs() < 1e-12);
        assert_eq!(chance_consistency(0), 1.0);
    }
}
