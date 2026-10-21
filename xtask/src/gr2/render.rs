//! The readable report: fixed-width tables for a terminal or a pasted message. `--json` prints the same
//! [`Report`] as `enth.gr2-report/1`.

use std::fmt::Write as _;

use super::input::Protocol;
use super::numbers::{FamilyCell, Item, Report, Verdict};

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

    fn render(&self, out: &mut String) {
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
            let mut s = String::from("  ");
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
        out.push_str(&line(&self.head));
        out.push('\n');
        let rule: Vec<String> = width.iter().map(|w| "-".repeat(*w)).collect();
        out.push_str(&line(&rule));
        out.push('\n');
        for r in &self.rows {
            out.push_str(&line(r));
            out.push('\n');
        }
    }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| v.to_string())
}

fn signed(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:+.2}"))
}

fn item_cell(i: Option<&Item>) -> String {
    match i {
        None => "-".to_owned(),
        Some(i) if i.clean => format!("pass {:.0}", i.structure),
        Some(i) if !i.in_time => "no submit".to_owned(),
        Some(i) => format!("{:.0}{}", i.structure, if i.assisted { "*" } else { "" }),
    }
}

fn change_cell(c: Option<i8>) -> &'static str {
    match c {
        Some(1) => "better",
        Some(0) => "same",
        Some(-1) => "worse",
        _ => "-",
    }
}

fn verdict(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "  met   ",
        Verdict::Fail => "NOT MET ",
        Verdict::Pending => "pending ",
    }
}

/// The whole report as text.
pub fn render(r: &Report, header: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "GR2 report ({})", r.schema);
    out.push_str(header);
    let _ = writeln!(
        out,
        "Items: a clean pass is a pass within {} s of active time with no hint, reveal, live meter or dimming; a family \
         is better or worse on a changed pass, or on structure % moving {} points with the pass unchanged.",
        r.time_limit_s, r.structure_step
    );
    out.push('\n');

    out.push_str("Proposed criteria (report 01 §7; you set them)\n");
    for c in &r.criteria {
        let _ = writeln!(out, "  [{}] {}", verdict(c.verdict), c.text);
        let _ = writeln!(out, "             {}", c.detail);
    }
    out.push('\n');

    out.push_str("People\n");
    let mut t = Table::new(
        &[
            "id", "protocol", "pre", "years", "conf", "tour", "lessons", "new", "knew", "named",
            "uncoded", "better", "same", "worse", "missing",
        ],
        3,
    );
    for p in &r.people {
        let full = p.protocol == Protocol::Full;
        let fam = |n: usize| if full { n.to_string() } else { "-".to_owned() };
        t.row(vec![
            if p.owner {
                format!("{} (you)", p.id)
            } else {
                p.id.clone()
            },
            match p.protocol {
                Protocol::Full => "full".to_owned(),
                Protocol::Tour => "tour".to_owned(),
            },
            opt(p.pre_form),
            p.years.map_or_else(|| "-".to_owned(), |y| format!("{y}")),
            p.confidence
                .map_or_else(|| "-".to_owned(), |c| format!("{c:.1}")),
            if p.has_progress {
                format!("{}/{}", p.tour_done, p.tour_total)
            } else {
                "no file".to_owned()
            },
            if p.has_progress {
                p.lessons_done.to_string()
            } else {
                "-".to_owned()
            },
            p.voted_new.to_string(),
            p.voted_knew.to_string(),
            p.named.len().to_string(),
            p.uncoded.to_string(),
            fam(p.improved),
            fam(p.same),
            fam(p.worse),
            fam(p.missing),
        ]);
    }
    t.render(&mut out);
    out.push_str(
        "  tour = tour lessons with every step done (progress file); new/knew = per-lesson votes; named = lessons with a \
         note coded specific; better/same/worse/missing = families, post-test against pre-test.\n\n",
    );

    let full: Vec<_> = r.people.iter().filter(|p| !p.families.is_empty()).collect();
    if !full.is_empty() {
        out.push_str("Items per person (pre → mid → post; pass N = clean pass at structure N %; N = structure %, * = helped)\n");
        let mut head = vec!["id".to_owned(), "family".to_owned()];
        head.extend(
            [
                "pre",
                "mid",
                "post",
                "delayed",
                "post vs pre",
                "mid vs pre",
                "kept",
            ]
            .map(str::to_owned),
        );
        let head_ref: Vec<&str> = head.iter().map(String::as_str).collect();
        let mut t = Table::new(&head_ref, 2);
        for p in &full {
            for c in &p.families {
                t.row(cells(&p.id, c));
            }
        }
        t.render(&mut out);
        out.push('\n');
    }

    out.push_str("Families (full-protocol people)\n");
    let mut t = Table::new(
        &[
            "family",
            "lessons",
            "before mid",
            "people",
            "better",
            "same",
            "worse",
            "mean",
            "structure",
            "pre pass",
            "post pass",
            "mid better",
        ],
        1,
    );
    for f in &r.families {
        t.row(vec![
            f.id.clone(),
            f.lessons.to_string(),
            f.taught_before_mid.to_string(),
            f.people.to_string(),
            f.improved.to_string(),
            f.same.to_string(),
            f.worse.to_string(),
            signed(f.mean_change),
            signed(f.mean_structure_change),
            format!("{}/{}", f.pre_clean, f.people),
            format!("{}/{}", f.post_clean, f.people),
            format!("{}/{}", f.mid_improved, f.mid_people),
        ]);
    }
    t.render(&mut out);
    let _ = writeln!(
        out,
        "  mean = mean change (-1 to +1); structure = mean structure % change; before mid = the family's tour lessons in \
         the first {} of the tour (the mid-test comes after them).",
        r.mid_after
    );
    let _ = writeln!(
        out,
        "  median families better: {}; the staggered mid-test: better on {} of {} taught (family, person) pairs, {} of {} \
         not yet taught.",
        r.median_improved
            .map_or_else(|| "-".to_owned(), |m| format!("{m}")),
        r.mid.taught_improved,
        r.mid.taught_pairs,
        r.mid.untaught_improved,
        r.mid.untaught_pairs
    );
    if let Some(s) = &r.sign_test {
        let _ = writeln!(
            out,
            "  sign test: {} of {} people with more families better than worse ({} ties left out), one-sided p = {:.3}. \
             Describe it; with a handful of people, do not lean on it.",
            s.better,
            s.better + s.worse,
            s.ties,
            s.p_one_sided
        );
    }
    out.push('\n');

    out.push_str("Lessons (everyone; the tour first)\n");
    let mut t = Table::new(
        &[
            "#",
            "lesson",
            "done",
            "new",
            "knew",
            "easy",
            "ok",
            "hard",
            "named",
            "notes",
            "ear 1st try",
        ],
        2,
    );
    for l in &r.lessons {
        t.row(vec![
            l.tour.map_or_else(String::new, |i| i.to_string()),
            l.key.clone(),
            l.done.to_string(),
            l.new.to_string(),
            l.knew.to_string(),
            l.too_easy.to_string(),
            l.ok.to_string(),
            l.too_hard.to_string(),
            l.named.to_string(),
            l.notes.to_string(),
            format!("{}/{}", l.ear_clean, l.ear_done),
        ]);
    }
    t.render(&mut out);
    out.push_str(
        "  done = people who completed it; easy/ok/hard = pace votes; named = people with a specific note; ear 1st try = \
         its ear questions answered with no wrong pick, of those answered.\n\n",
    );

    let uncoded = r.notes.iter().filter(|n| n.coded.is_none()).count();
    let _ = writeln!(
        out,
        "Notes: {} in all, {uncoded} not coded yet (`--notes` lists each with its lesson's objective).",
        r.notes.len()
    );
    if !r.findings.is_empty() {
        out.push_str("\nFindings\n");
        for f in &r.findings {
            let _ = writeln!(out, "  {f}");
        }
    }
    out
}

fn cells(id: &str, c: &FamilyCell) -> Vec<String> {
    vec![
        id.to_owned(),
        c.family.clone(),
        item_cell(c.pre.as_ref()),
        item_cell(c.mid.as_ref()),
        item_cell(c.post.as_ref()),
        item_cell(c.delayed.as_ref()),
        change_cell(c.change).to_owned(),
        change_cell(c.mid_change).to_owned(),
        change_cell(c.kept).to_owned(),
    ]
}

/// `--notes`: every note, with the lesson's objective, for the coder.
pub fn render_notes(r: &Report) -> String {
    let mut out = String::from(
        "Notes to code (write `coded = \"specific\"`, `\"vague\"` or `\"none\"` beside each in the person's answers file)\n",
    );
    for n in &r.notes {
        let coded = match n.coded {
            None => "uncoded".to_owned(),
            Some(c) => serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        };
        let _ = writeln!(
            out,
            "\n  {} · {} · {} · {coded}",
            n.person,
            n.lesson.as_deref().unwrap_or("(no lesson)"),
            n.from
        );
        let _ = writeln!(out, "    note:      {}", n.text);
        if let Some(o) = &n.objective {
            let _ = writeln!(out, "    objective: {o}");
        }
    }
    out
}
