//! The readable report: plain fixed-width tables for a terminal or a pasted message. `--json` prints the same
//! [`Report`] as `enth.gr1-report/1`.

use std::fmt::Write as _;

use super::numbers::{GroupOut, Rate, Report, RhoOut};

/// A fixed-width table: the first `left` columns are left-aligned, the rest right-aligned.
struct Table {
    head: Vec<String>,
    rows: Vec<Vec<String>>,
    left: usize,
}

impl Table {
    fn new(head: &[&str], left: usize) -> Table {
        Table {
            head: head.iter().map(|h| (*h).to_owned()).collect(),
            rows: Vec::new(),
            left,
        }
    }

    fn row(&mut self, cells: Vec<String>) {
        debug_assert_eq!(cells.len(), self.head.len());
        self.rows.push(cells);
    }

    fn render(&self, indent: &str) -> String {
        let width: Vec<usize> = (0..self.head.len())
            .map(|c| {
                self.rows
                    .iter()
                    .map(|r| r[c].chars().count())
                    .chain([self.head[c].chars().count()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |cells: &[String]| {
            let mut s = String::from(indent);
            for (c, cell) in cells.iter().enumerate() {
                if c > 0 {
                    s.push_str("  ");
                }
                let pad = width[c].saturating_sub(cell.chars().count());
                if c < self.left {
                    s.push_str(cell);
                    s.push_str(&" ".repeat(pad));
                } else {
                    s.push_str(&" ".repeat(pad));
                    s.push_str(cell);
                }
            }
            s.trim_end().to_owned()
        };
        let mut out = line(&self.head);
        out.push('\n');
        out.push_str(indent);
        out.push_str(
            &width
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  "),
        );
        out.push('\n');
        for r in &self.rows {
            out.push_str(&line(r));
            out.push('\n');
        }
        out
    }
}

fn level(l: Option<u8>) -> String {
    l.map_or("?".to_owned(), |l| l.to_string())
}

fn pct(x: Option<f64>) -> String {
    x.map_or("-".to_owned(), |x| format!("{:.0}%", x * 100.0))
}

fn time(ms: Option<f64>) -> String {
    ms.map_or("-".to_owned(), |ms| {
        let s = (ms / 1000.0).round() as u64;
        format!("{}:{:02}", s / 60, s % 60)
    })
}

fn one(x: Option<f64>) -> String {
    x.map_or("-".to_owned(), |x| format!("{x:.1}"))
}

fn two(x: Option<f64>) -> String {
    x.map_or("-".to_owned(), |x| format!("{x:.2}"))
}

fn ci(r: &Rate) -> String {
    r.ci.map_or("-".to_owned(), |(a, b)| {
        format!("{:.0}-{:.0}%", a * 100.0, b * 100.0)
    })
}

fn rho(r: &RhoOut) -> String {
    match (r.rho, r.ci_lo, r.ci_hi) {
        (Some(x), Some(lo), Some(hi)) => format!("{x:+.2}  [{lo:+.2}, {hi:+.2}]  n={}", r.n),
        (Some(x), _, _) => format!("{x:+.2}  (no interval)  n={}", r.n),
        _ => format!("n/a  n={}", r.n),
    }
}

fn int(x: Option<f64>) -> String {
    x.map_or("-".to_owned(), |x| format!("{x:.0}"))
}

fn dollars(x: Option<f64>) -> String {
    x.map_or("-".to_owned(), |x| format!("${x:.2}"))
}

/// The report as text.
pub fn text(r: &Report) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(w, "GR1 report ({})", r.schema);
    let i = &r.input;
    let _ = writeln!(
        w,
        "Read {} round log file(s): {} lines, {} read into rounds ({} unusable, {} from another schema version); {} error(s), {} warning(s), {} note(s).",
        i.files,
        i.lines,
        i.entries,
        i.skipped_unusable,
        i.skipped_other_schema,
        i.errors,
        i.warnings,
        i.notes
    );
    let _ = writeln!(
        w,
        "Privacy lint: {}",
        if i.privacy_findings == 0 {
            "clean (no email address or home-folder path found)".to_owned()
        } else {
            format!(
                "{} line(s) look like personal data; see Problems",
                i.privacy_findings
            )
        }
    );

    section(
        w,
        "Gate indicators (the proposed reading, docs/gr1.md; you decide GR1)",
    );
    for g in &r.gate {
        let _ = writeln!(w, "  [{:^8}] {}", g.status.text(), g.check);
        let _ = writeln!(w, "             reading: {}", g.reading);
        let _ = writeln!(w, "             logs:    {}", g.value);
    }

    section(w, "Players");
    let mut t = Table::new(
        &[
            "player",
            "rounds",
            "played",
            "skipped",
            "unusable",
            "restored",
            "passed",
            "pass",
            "assisted",
            "median ttp",
            "votes",
            "about right",
            "rated",
            "disputes",
            "want more",
        ],
        1,
    );
    for p in &r.players {
        t.row(vec![
            p.id.clone(),
            p.rounds.to_string(),
            p.played.to_string(),
            p.skipped.to_string(),
            p.unusable.to_string(),
            p.restored.to_string(),
            p.passed.to_string(),
            pct(p.pass_rate),
            pct(p.assisted_share),
            time(p.median_time_to_pass_ms),
            p.votes.to_string(),
            pct(p.about_right_share),
            p.rated.to_string(),
            p.disputes.to_string(),
            pct(p.wanted_more_rate),
        ]);
    }
    w.push_str(&t.render("  "));
    let _ = writeln!(
        w,
        "  played = tried (a scored submit or a reveal); restored = rounds a host restore logged again (the last end line counts); ttp = time to pass; want more = ended by \"another round?\"."
    );

    section(
        w,
        "Pass rate by level (all players; rounds are the trials, so the interval is a rough guide)",
    );
    let mut t = Table::new(
        &[
            "level",
            "played",
            "passed",
            "pass",
            "95% CI",
            "unassisted",
            "assisted",
            "median ttp",
            "unassisted ttp",
            "sound 1st",
            "sound best",
        ],
        1,
    );
    for l in &r.levels {
        t.row(vec![
            level(l.level),
            l.played.to_string(),
            l.passed.to_string(),
            pct(l.pass.rate),
            ci(&l.pass),
            format!("{}/{}", l.unassisted_passed, l.unassisted_played),
            l.assisted.to_string(),
            time(l.median_time_to_pass_ms),
            time(l.median_time_to_pass_unassisted_ms),
            one(l.median_first_sound),
            one(l.median_best_sound),
        ]);
    }
    w.push_str(&t.render("  "));
    let _ = writeln!(
        w,
        "  sound 1st = median sound % of the recorded (first) submit; sound best = median of each round's best submit."
    );

    if r.players.len() > 1 {
        let levels: Vec<Option<u8>> = r.levels.iter().map(|l| l.level).collect();
        let mut head = vec!["passed/played".to_owned()];
        head.extend(levels.iter().map(|l| format!("level {}", level(*l))));
        let head_refs: Vec<&str> = head.iter().map(String::as_str).collect();
        let mut t = Table::new(&head_refs, 1);
        for p in &r.players {
            let mut cells = vec![p.id.clone()];
            for l in &levels {
                cells.push(
                    p.by_level
                        .iter()
                        .find(|c| c.level == *l)
                        .map_or("-".to_owned(), |c| format!("{}/{}", c.passed, c.played)),
                );
            }
            t.row(cells);
        }
        let _ = writeln!(w);
        w.push_str(&t.render("  "));
    }

    section(w, "Time to pass and assists");
    let tp = &r.time_to_pass;
    let _ = writeln!(
        w,
        "  median time to pass {} over {} passes; unassisted {} over {} passes (active time to the first passing submit, m:ss).",
        time(tp.median_ms),
        tp.n,
        time(tp.unassisted_median_ms),
        tp.unassisted_n
    );
    let a = &r.assists;
    let _ = writeln!(
        w,
        "  assisted share {} ({} of {} played rounds): {} used a hint ({} hints in all), {} revealed the target, {} had the live meter; {} of the passes were assisted.",
        pct(a.share),
        a.assisted,
        a.played,
        a.with_hint,
        a.hints,
        a.revealed,
        a.live_meter,
        a.assisted_passes
    );

    section(w, "Agreement with the score (the vote after each round)");
    let g = &r.agreement;
    let mut t = Table::new(&["vote", "rounds", "share", "median sound %"], 1);
    t.row(vec![
        "too low".into(),
        g.too_low.to_string(),
        pct(g.too_low_share),
        one(g.median_sound_too_low),
    ]);
    t.row(vec![
        "about right".into(),
        g.about_right.to_string(),
        pct(g.about_right_share),
        one(g.median_sound_about_right),
    ]);
    t.row(vec![
        "too high".into(),
        g.too_high.to_string(),
        pct(g.too_high_share),
        one(g.median_sound_too_high),
    ]);
    w.push_str(&t.render("  "));
    let _ = writeln!(
        w,
        "  {} votes on {} scored rounds (response {}); {} vote(s) from a newer build not counted; {} scored round(s) without a vote.",
        g.votes,
        g.scored_rounds,
        pct(g.response_rate),
        g.unknown,
        g.no_vote
    );
    let by_player: Vec<String> = g
        .players
        .iter()
        .map(|p| {
            format!(
                "{} {} ({} votes{})",
                p.id,
                pct(p.share),
                p.votes,
                if p.rater { "" } else { ", too few to rate" }
            )
        })
        .collect();
    let _ = writeln!(w, "  about right by player: {}.", by_player.join("; "));

    section(
        w,
        "Blind rating against the score (rated before the score showed; the recorded first submit)",
    );
    let b = &r.rating;
    let _ = writeln!(
        w,
        "  rated rounds {}, mean rating {}.",
        b.n,
        two(b.mean_rating)
    );
    let _ = writeln!(
        w,
        "  Spearman rho, rating against sound %      {}",
        rho(&b.sound)
    );
    let _ = writeln!(
        w,
        "  Spearman rho, rating against structure %  {}",
        rho(&b.structure)
    );
    let _ = writeln!(
        w,
        "  Spearman rho, rating against combined %   {}",
        rho(&b.combined)
    );
    let _ = writeln!(
        w,
        "  (95% percentile bootstrap over rounds, {} resamples, seed {:#x}.)",
        r.thresholds.bootstrap_reps, r.thresholds.bootstrap_seed
    );
    if !b.by_rating.is_empty() {
        let mut t = Table::new(&["rating", "rounds", "median sound %"], 1);
        for x in &b.by_rating {
            t.row(vec![
                x.rating.to_string(),
                x.n.to_string(),
                one(x.median_sound),
            ]);
        }
        let _ = writeln!(w);
        w.push_str(&t.render("  "));
    }
    if !b.players.is_empty() {
        let each: Vec<String> = b
            .players
            .iter()
            .map(|p| {
                format!(
                    "{} {} (n={})",
                    p.id,
                    p.rho.map_or("n/a".to_owned(), |x| format!("{x:+.2}")),
                    p.n
                )
            })
            .collect();
        let _ = writeln!(
            w,
            "  per player (at least {} rated rounds): {}.",
            r.thresholds.min_rated_for_rho,
            each.join("; ")
        );
    }

    section(w, "Want more");
    let wm = &r.want_more;
    let _ = writeln!(
        w,
        "  \"another round?\" answered yes on {} of {} ended rounds ({}). Every sitting's last round cannot say yes, so this reads low by about one round per sitting.",
        wm.rounds.k,
        wm.rounds.n,
        pct(wm.rounds.rate)
    );
    let each: Vec<String> = wm
        .players
        .iter()
        .map(|p| {
            format!(
                "{} {} ({} of {})",
                p.id,
                pct(p.rounds.rate),
                p.rounds.k,
                p.rounds.n
            )
        })
        .collect();
    let _ = writeln!(w, "  by player: {}.", each.join("; "));
    if !wm.survey.is_empty() {
        let mut t = Table::new(&["player", "want more?", "os", "daw", "note"], 5);
        for s in &wm.survey {
            t.row(vec![
                if s.known_player {
                    s.id.clone()
                } else {
                    format!("{} (no log)", s.id)
                },
                s.want_more.clone(),
                s.os.clone().unwrap_or_default(),
                s.daw.clone().unwrap_or_default(),
                s.note.clone().unwrap_or_default(),
            ]);
        }
        let _ = writeln!(w, "  End-of-session answers:");
        w.push_str(&t.render("  "));
    }
    if !wm.unanswered.is_empty() {
        let _ = writeln!(
            w,
            "  No end-of-session answer yet from: {}.",
            wm.unanswered.join(", ")
        );
    }

    section(w, "Disputes");
    if r.disputes.is_empty() {
        let _ = writeln!(w, "  none.");
    }
    for d in &r.disputes {
        let scores = d.scores.map_or("no score".to_owned(), |s| {
            format!(
                "sound {:.1}, structure {:.1}, combined {:.1}",
                s[0], s[1], s[2]
            )
        });
        let _ = writeln!(
            w,
            "  {} serial {} (level {}, {}), {}, outcome {}, vote {}{}",
            d.player,
            d.serial,
            level(d.level),
            d.code,
            scores,
            d.outcome.as_deref().unwrap_or("unknown"),
            d.vote.as_deref().unwrap_or("none"),
            if d.has_end {
                ""
            } else {
                "; no end line (the session ended before the round did)"
            }
        );
        let _ = writeln!(
            w,
            "      \"{}\"",
            d.note.chars().take(300).collect::<String>()
        );
    }

    if let Some(b) = &r.baseline {
        baseline(w, r, b);
    }

    section(w, "Checks");
    let c = &r.checks;
    let _ = writeln!(
        w,
        "  codes played: {} distinct, {} played by every player.",
        c.codes, c.codes_played_by_all
    );
    let scorers: Vec<String> = c
        .scorers
        .iter()
        .map(|c| format!("{} x{}", c.value, c.count))
        .collect();
    let engines: Vec<String> = c
        .engines
        .iter()
        .map(|c| format!("{} x{}", c.value, c.count))
        .collect();
    let _ = writeln!(
        w,
        "  scorers: {}{}",
        if scorers.is_empty() {
            "none seen".to_owned()
        } else {
            scorers.join(", ")
        },
        if c.mixed_scorers {
            "  <- MIXED: scores of different scorers do not compare"
        } else {
            ""
        }
    );
    let _ = writeln!(
        w,
        "  engines: {}{}",
        if engines.is_empty() {
            "none seen".to_owned()
        } else {
            engines.join(", ")
        },
        if c.mixed_engines {
            "  <- MIXED: an engine change can move a sound score"
        } else {
            ""
        }
    );
    if c.fingerprint_drift.is_empty() {
        let _ = writeln!(w, "  targets: every code gave one target fingerprint.");
    } else {
        let _ = writeln!(
            w,
            "  targets: <- DRIFT, the same code gave different targets for: {}",
            c.fingerprint_drift.join(", ")
        );
    }
    if c.custom_codes > 0 || c.unplaced_rounds > 0 {
        let _ = writeln!(
            w,
            "  {} played round(s) used a custom code (hand-set amounts) and {} a code with no readable level.",
            c.custom_codes, c.unplaced_rounds
        );
    }

    section(w, "Problems");
    if i.problems.is_empty() && i.privacy.is_empty() {
        let _ = writeln!(w, "  none.");
    }
    for p in &i.problems {
        let _ = writeln!(
            w,
            "  {:<7} {} (x{}, first at {})",
            p.severity, p.message, p.count, p.first_at
        );
    }
    for p in &i.privacy {
        let _ = writeln!(w, "  privacy {p}");
    }
    out
}

fn section(w: &mut String, title: &str) {
    let _ = writeln!(w, "\n{title}");
}

fn baseline(w: &mut String, r: &Report, b: &super::numbers::BaselineOut) {
    section(
        w,
        "Agent baseline beside the humans (pass@1 = passes over single attempts)",
    );
    let _ = writeln!(
        w,
        "  {} code(s) in the baseline; humans played {} of them. Humans are scored on each round's best sound %, arms on the final patch's.",
        b.codes, b.codes_played_by_humans
    );
    let mut groups: Vec<&GroupOut> = vec![&b.humans_all];
    groups.extend(b.humans_same_codes.iter());
    groups.extend(b.arms.iter());
    let levels: Vec<Option<u8>> = r.levels.iter().map(|l| l.level).collect();
    let mut head = vec!["pass@1".to_owned()];
    head.extend(levels.iter().map(|l| format!("level {}", level(*l))));
    head.push("all".to_owned());
    head.push("95% CI".to_owned());
    let refs: Vec<&str> = head.iter().map(String::as_str).collect();
    let mut t = Table::new(&refs, 1);
    for g in &groups {
        let mut cells = vec![g.label.clone()];
        for l in &levels {
            cells.push(
                g.by_level
                    .iter()
                    .find(|c| c.level == *l)
                    .map_or("-".to_owned(), |c| {
                        format!("{} ({}/{})", pct(c.cell.rate), c.cell.passes, c.cell.n)
                    }),
            );
        }
        cells.push(format!(
            "{} ({}/{})",
            pct(g.overall.rate),
            g.overall.passes,
            g.overall.n
        ));
        cells.push(g.overall.ci.map_or("-".to_owned(), |(a, b)| {
            format!("{:.0}-{:.0}%", a * 100.0, b * 100.0)
        }));
        t.row(cells);
    }
    w.push_str(&t.render("  "));
    let mut head = vec!["median sound %".to_owned()];
    head.extend(levels.iter().map(|l| format!("level {}", level(*l))));
    head.push("all".to_owned());
    let refs: Vec<&str> = head.iter().map(String::as_str).collect();
    let mut t = Table::new(&refs, 1);
    for g in &groups {
        let mut cells = vec![g.label.clone()];
        for l in &levels {
            cells.push(
                g.by_level
                    .iter()
                    .find(|c| c.level == *l)
                    .map_or("-".to_owned(), |c| one(c.cell.median_sound)),
            );
        }
        cells.push(one(g.overall.median_sound));
        t.row(cells);
    }
    let _ = writeln!(w);
    w.push_str(&t.render("  "));
    let mut t = Table::new(
        &[
            "effort per attempt",
            "attempts",
            "codes",
            "renders",
            "tokens",
            "cost",
            "total cost",
        ],
        1,
    );
    for g in &b.arms {
        t.row(vec![
            g.label.clone(),
            g.overall.n.to_string(),
            g.codes.to_string(),
            int(g.median_renders),
            int(g.median_tokens),
            dollars(g.median_cost_usd),
            dollars(g.total_cost_usd),
        ]);
    }
    let _ = writeln!(w);
    w.push_str(&t.render("  "));
}
