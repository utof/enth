//! The GR2 numbers (docs/learn.md, "GR2"; docs/r2-owner.md, "Reading gr2-report"), reported per person:
//!
//! - **An item** is one test code a person played (the first played round with that code in their round log). It
//!   is a **clean pass** when a submit passed within the forms' time limit (active time) and the round used no
//!   help (hint, reveal, live meter, dimming); its **structure %** is the best structure % of the submits made within
//!   the limit (0 when none was).
//! - **A family's change** compares the pre-test's item with the post-test's: +1 when the post-test passes cleanly
//!   and the pre-test did not, −1 the other way round; with the pass unchanged, ±1 when structure % moved by at least
//!   the forms' `structure_step` points, else 0. The mid-test is compared with the pre-test the same way, the delayed
//!   test with the post-test.
//! - **A named new thing** is a "what did you learn?" note about a lesson that the coder marked `specific`.
//!
//! The proposed criteria (report 01 §7; the owner sets them) are evaluated as pass, fail or pending: pending when a
//! missing item or an uncoded note could still change the verdict.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::input::{Answers, Coded, Form, Forms, LessonInfo, Pace, ProgressView, Protocol, Vote};
use crate::gr1_report::logs::{Loaded, Round};

/// The report's schema string.
pub const REPORT_SCHEMA: &str = "enth.gr2-report/1";

/// One person's files, read.
#[derive(Clone, Debug)]
pub struct Person {
    /// Their answers.
    pub answers: Answers,
    /// Their progress file, if they sent one.
    pub progress: Option<ProgressView>,
}

/// The proposed criteria's numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Lessons with a named new thing every participant needs.
    pub lessons_named: usize,
    /// Families the median full-protocol participant improves on.
    pub families_improved: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        // Report 01 §7 proposed 3 of 10 families; GR2's forms have the generator's 6 categories, so 2 of 6.
        Thresholds {
            lessons_named: 3,
            families_improved: 2,
        }
    }
}

/// Everything the numbers are computed from.
pub struct Inputs<'a> {
    /// The test forms.
    pub forms: &'a Forms,
    /// The lessons by `pack/lesson`.
    pub lessons: &'a BTreeMap<String, LessonInfo>,
    /// The tour track, in order (lesson 0.1 first).
    pub tour: &'a [String],
    /// The people, in id order.
    pub people: &'a [Person],
    /// Everyone's round logs, read by `gr1-report`'s reader (players named by the same ids).
    pub rounds: &'a Loaded,
    /// The owner's id.
    pub owner: &'a str,
    /// The criteria's numbers.
    pub thresholds: Thresholds,
}

/// One test item as played.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item {
    /// The code as logged.
    pub code: String,
    /// Passed within the time limit with no help.
    pub clean: bool,
    /// The best structure % of the submits within the limit (0 when none).
    pub structure: f64,
    /// A submit was made within the limit.
    pub in_time: bool,
    /// The round used help (a clean pass needs none).
    pub assisted: bool,
    /// Active seconds to the first pass, if it passed at all.
    pub pass_s: Option<f64>,
}

/// One family's items and changes for one person.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FamilyCell {
    /// The family id.
    pub family: String,
    /// The pre-test's item.
    pub pre: Option<Item>,
    /// The mid-test's item.
    pub mid: Option<Item>,
    /// The post-test's item.
    pub post: Option<Item>,
    /// The delayed test's item.
    pub delayed: Option<Item>,
    /// Post against pre: +1, 0, −1; none without both.
    pub change: Option<i8>,
    /// Mid against pre.
    pub mid_change: Option<i8>,
    /// Delayed against post.
    pub kept: Option<i8>,
}

/// One person's row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PersonRow {
    /// The id.
    pub id: String,
    /// `full` or `tour`.
    pub protocol: Protocol,
    /// This is the owner.
    pub owner: bool,
    /// The pre-test's form.
    pub pre_form: Option<Form>,
    /// Years with synths (intake).
    pub years: Option<f64>,
    /// Mean confidence 1–5 over the families answered (intake).
    pub confidence: Option<f64>,
    /// A progress file was read.
    pub has_progress: bool,
    /// Tour lessons completed (every step done), of `tour_total`.
    pub tour_done: usize,
    /// The tour's length.
    pub tour_total: usize,
    /// Lessons completed in all.
    pub lessons_done: usize,
    /// Lessons voted "new to me".
    pub voted_new: usize,
    /// Lessons voted "knew it".
    pub voted_knew: usize,
    /// Lessons with a note coded `specific`, sorted.
    pub named: Vec<String>,
    /// Lessons with an uncoded note and none coded `specific`.
    pub uncoded: usize,
    /// Per family, in the forms' order (empty for the tour protocol).
    pub families: Vec<FamilyCell>,
    /// Families better on the post-test.
    pub improved: usize,
    /// Families unchanged.
    pub same: usize,
    /// Families worse.
    pub worse: usize,
    /// Families without both a pre- and a post-test item.
    pub missing: usize,
    /// A family failed (not clean) on the pre-test and passed cleanly on the post-test.
    pub passed_after_failing: bool,
}

/// One family's row over the full-protocol people.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FamilyRow {
    /// The id.
    pub id: String,
    /// The name.
    pub title: String,
    /// The tour lessons that teach it.
    pub lessons: usize,
    /// How many of them come before the mid-test.
    pub taught_before_mid: usize,
    /// People with a pre/post pair.
    pub people: usize,
    /// Better.
    pub improved: usize,
    /// Unchanged.
    pub same: usize,
    /// Worse.
    pub worse: usize,
    /// The mean change (−1 to +1); none without people.
    pub mean_change: Option<f64>,
    /// The mean structure % change (post − pre).
    pub mean_structure_change: Option<f64>,
    /// Clean passes on the pre-test, of `people`.
    pub pre_clean: usize,
    /// Clean passes on the post-test, of `people`.
    pub post_clean: usize,
    /// People with a pre/mid pair.
    pub mid_people: usize,
    /// Better at the mid-test than the pre-test.
    pub mid_improved: usize,
}

/// One lesson's row over everyone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LessonRow {
    /// `pack/lesson`.
    pub key: String,
    /// Its title, if the lessons were found.
    pub title: Option<String>,
    /// Its place in the tour (1-based; lesson 0.1 is 1).
    pub tour: Option<usize>,
    /// People who completed it.
    pub done: usize,
    /// "New to me" votes.
    pub new: usize,
    /// "Knew it" votes.
    pub knew: usize,
    /// "Too easy".
    pub too_easy: usize,
    /// "About right".
    pub ok: usize,
    /// "Too hard".
    pub too_hard: usize,
    /// People who named a specific new thing in it.
    pub named: usize,
    /// People with any note on it.
    pub notes: usize,
    /// Ear questions (Choose steps) passed first time, over everyone.
    pub ear_clean: usize,
    /// Ear questions done, over everyone.
    pub ear_done: usize,
}

/// The staggered mid-test: improvement at the mid-test on families taught before it, against families not yet taught.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MidSummary {
    /// (person, family) pairs on taught families with a pre and a mid item.
    pub taught_pairs: usize,
    /// Of them, better at the mid-test.
    pub taught_improved: usize,
    /// Pairs on families not yet taught.
    pub untaught_pairs: usize,
    /// Of them, better at the mid-test.
    pub untaught_improved: usize,
}

/// A one-sided sign test over people: more families better than worse, against fewer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SignTest {
    /// People with more families better than worse.
    pub better: usize,
    /// People with more worse than better.
    pub worse: usize,
    /// People with as many of each (left out).
    pub ties: usize,
    /// P(at least `better` of `better + worse` by chance), each person a fair coin.
    pub p_one_sided: f64,
}

/// A criterion's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Met.
    Pass,
    /// Not met, and nothing missing could change it.
    Fail,
    /// Not decided yet: missing items or uncoded notes could change it.
    Pending,
}

/// One proposed criterion.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Criterion {
    /// Its id.
    pub id: &'static str,
    /// What it asks.
    pub text: String,
    /// The verdict.
    pub verdict: Verdict,
    /// What the data say.
    pub detail: String,
}

/// A note to code, with its lesson's objective beside it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NoteRow {
    /// Whose.
    pub person: String,
    /// The lesson it names.
    pub lesson: Option<String>,
    /// `lesson` or `debrief`.
    pub from: &'static str,
    /// The text.
    pub text: String,
    /// The verdict, once coded.
    pub coded: Option<Coded>,
    /// The lesson's objective, if known.
    pub objective: Option<String>,
}

/// The whole report (`enth.gr2-report/1`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    /// `enth.gr2-report/1`.
    pub schema: &'static str,
    /// The time limit in seconds.
    pub time_limit_s: u64,
    /// The structure step in points.
    pub structure_step: f64,
    /// Tour lessons before the mid-test.
    pub mid_after: usize,
    /// The owner's id.
    pub owner: String,
    /// The people.
    pub people: Vec<PersonRow>,
    /// The families.
    pub families: Vec<FamilyRow>,
    /// The median number of families improved, over the full-protocol people with a pre/post pair.
    pub median_improved: Option<f64>,
    /// The sign test.
    pub sign_test: Option<SignTest>,
    /// The staggered mid-test.
    pub mid: MidSummary,
    /// The lessons: the tour first, then any other lesson someone completed or answered.
    pub lessons: Vec<LessonRow>,
    /// The proposed criteria.
    pub criteria: Vec<Criterion>,
    /// Every note.
    pub notes: Vec<NoteRow>,
    /// Test codes played by an id with no answers file, or other findings while computing.
    pub findings: Vec<String>,
}

/// Score one played round as a test item.
pub fn item(round: &Round, limit_ms: u64) -> Item {
    let (in_time, structure, passed_in_time) = if round.submits.is_empty() {
        // An older line without the submit list: the recorded first submit counts if the whole round fit the limit.
        match (round.first(), &round.result) {
            (Some(first), Some(result)) if result.active_ms <= limit_ms => {
                (true, first.structure, first.passed)
            }
            _ => (false, 0.0, false),
        }
    } else {
        let within: Vec<_> = round
            .submits
            .iter()
            .filter(|s| s.at_ms <= limit_ms)
            .collect();
        let best = within
            .iter()
            .map(|s| f64::from(s.structure) / 100.0)
            .fold(0.0, f64::max);
        (!within.is_empty(), best, within.iter().any(|s| s.passed))
    };
    let passed_by_time = round.time_to_pass_ms().is_some_and(|t| t <= limit_ms);
    let assisted = round.assisted();
    Item {
        code: round.code.clone(),
        clean: (passed_in_time || passed_by_time) && round.passed() && !assisted,
        structure,
        in_time,
        assisted,
        pass_s: round.time_to_pass_ms().map(|t| t as f64 / 1000.0),
    }
}

/// `later` against `earlier`: +1, 0 or −1 (see the [module docs](self)).
pub fn change(earlier: &Item, later: &Item, step: f64) -> i8 {
    match (earlier.clean, later.clean) {
        (false, true) => 1,
        (true, false) => -1,
        _ => {
            let d = later.structure - earlier.structure;
            if d >= step {
                1
            } else if d <= -step {
                -1
            } else {
                0
            }
        }
    }
}

/// P(X ≥ k) for X ~ Binomial(n, 1/2).
pub fn sign_p(k: usize, n: usize) -> f64 {
    let mut total = 0.0;
    let mut c = 1.0; // C(n, 0)
    for i in 0..=n {
        if i >= k {
            total += c;
        }
        c = c * (n - i) as f64 / (i + 1) as f64;
    }
    total / 2f64.powi(n as i32)
}

fn median(values: &mut [usize]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let n = values.len();
    Some(if n % 2 == 1 {
        values[n / 2] as f64
    } else {
        (values[n / 2 - 1] + values[n / 2]) as f64 / 2.0
    })
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn completed(
    lesson: &LessonInfo,
    steps: Option<&BTreeMap<String, super::input::StepState>>,
) -> bool {
    let Some(steps) = steps else { return false };
    !lesson.steps.is_empty()
        && lesson
            .steps
            .iter()
            .all(|s| steps.get(&s.id).is_some_and(|st| st.done))
}

/// Compute the report.
pub fn compute(inp: &Inputs<'_>) -> Report {
    let forms = inp.forms;
    let mut findings = Vec::new();
    // (player, normalized code) -> the first played round.
    let mut first: BTreeMap<(&str, &str), &Round> = BTreeMap::new();
    for r in inp.rounds.rounds.iter().filter(|r| r.played()) {
        first.entry((&r.player, &r.code_norm)).or_insert(r);
    }
    let ids: BTreeSet<&str> = inp.people.iter().map(|p| p.answers.id.as_str()).collect();
    let mut orphans: BTreeSet<&str> = BTreeSet::new();
    for (player, code) in first.keys() {
        if forms.by_code.contains_key(*code) && !ids.contains(player) {
            orphans.insert(player);
        }
    }
    for o in orphans {
        findings.push(format!(
            "{o} played test codes but has no answers file: their items are not counted"
        ));
    }
    let tour_pos: BTreeMap<&str, usize> = inp
        .tour
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    for f in &forms.families {
        for l in &f.lessons {
            if !tour_pos.contains_key(l.as_str()) {
                findings.push(format!(
                    "family `{}` names lesson {l}, which is not in the tour",
                    f.id
                ));
            }
        }
    }

    let mut people = Vec::new();
    let mut notes = Vec::new();
    for p in inp.people {
        let a = &p.answers;
        let steps_of = |key: &str| p.progress.as_ref().and_then(|pr| pr.get(key));
        let tour_done = inp
            .tour
            .iter()
            .filter(|k| {
                inp.lessons
                    .get(k.as_str())
                    .is_some_and(|l| completed(l, steps_of(k)))
            })
            .count();
        let lessons_done = inp
            .lessons
            .iter()
            .filter(|(k, l)| completed(l, steps_of(k)))
            .count();
        for l in &a.lessons {
            if !inp.lessons.contains_key(&l.key) {
                findings.push(format!(
                    "{}: answers name lesson {}, which is not in the packs read",
                    a.id, l.key
                ));
            }
        }
        let named: BTreeSet<String> = a
            .notes
            .iter()
            .filter(|n| n.coded == Some(Coded::Specific))
            .filter_map(|n| n.lesson.clone())
            .collect();
        let uncoded: BTreeSet<&str> = a
            .notes
            .iter()
            .filter(|n| n.coded.is_none())
            .filter_map(|n| n.lesson.as_deref())
            .filter(|l| !named.contains(*l))
            .collect();
        for n in &a.notes {
            notes.push(NoteRow {
                person: a.id.clone(),
                lesson: n.lesson.clone(),
                from: n.from,
                text: n.text.clone(),
                coded: n.coded,
                objective: n
                    .lesson
                    .as_deref()
                    .and_then(|l| inp.lessons.get(l))
                    .map(|l| l.objective.clone()),
            });
        }
        let mut families = Vec::new();
        if a.protocol == Protocol::Full
            && let Some(pre_form) = a.pre_form
        {
            for f in &forms.families {
                let get = |form: Option<Form>| {
                    let code = f.codes.get(&form?)?;
                    let norm = crate::gr1_report::code::normalize(code);
                    first
                        .get(&(a.id.as_str(), norm.as_str()))
                        .map(|r| item(r, forms.time_limit_ms))
                };
                let pre = get(Some(pre_form));
                let post = get(pre_form.other());
                let mid = get(Some(Form::M));
                let delayed = get(Some(Form::D));
                let pair = |x: &Option<Item>, y: &Option<Item>| match (x, y) {
                    (Some(x), Some(y)) => Some(change(x, y, forms.structure_step)),
                    _ => None,
                };
                families.push(FamilyCell {
                    family: f.id.clone(),
                    change: pair(&pre, &post),
                    mid_change: pair(&pre, &mid),
                    kept: pair(&post, &delayed),
                    pre,
                    mid,
                    post,
                    delayed,
                });
            }
        }
        let count = |v: i8| families.iter().filter(|c| c.change == Some(v)).count();
        let improved = count(1);
        let same = count(0);
        let worse = count(-1);
        let missing = families.iter().filter(|c| c.change.is_none()).count();
        let passed_after_failing = families.iter().any(
            |c| matches!((&c.pre, &c.post), (Some(pre), Some(post)) if !pre.clean && post.clean),
        );
        let conf: Vec<f64> = a.confidence.values().map(|c| f64::from(*c)).collect();
        people.push(PersonRow {
            id: a.id.clone(),
            protocol: a.protocol,
            owner: a.id == inp.owner,
            pre_form: a.pre_form,
            years: a.years,
            confidence: mean(&conf),
            has_progress: p.progress.is_some(),
            tour_done,
            tour_total: inp.tour.len(),
            lessons_done,
            voted_new: a
                .lessons
                .iter()
                .filter(|l| l.vote == Some(Vote::New))
                .count(),
            voted_knew: a
                .lessons
                .iter()
                .filter(|l| l.vote == Some(Vote::Knew))
                .count(),
            named: named.into_iter().collect(),
            uncoded: uncoded.len(),
            families,
            improved,
            same,
            worse,
            missing,
            passed_after_failing,
        });
    }

    // Families over the full-protocol people.
    let mut family_rows = Vec::new();
    let mut mid = MidSummary::default();
    for (fi, f) in forms.families.iter().enumerate() {
        let cells: Vec<&FamilyCell> = people.iter().filter_map(|p| p.families.get(fi)).collect();
        let pairs: Vec<&FamilyCell> = cells
            .iter()
            .copied()
            .filter(|c| c.change.is_some())
            .collect();
        let changes: Vec<f64> = pairs
            .iter()
            .filter_map(|c| c.change.map(f64::from))
            .collect();
        let structure: Vec<f64> = pairs
            .iter()
            .filter_map(|c| Some(c.post.as_ref()?.structure - c.pre.as_ref()?.structure))
            .collect();
        let taught = f
            .lessons
            .iter()
            .filter(|l| {
                tour_pos
                    .get(l.as_str())
                    .is_some_and(|i| *i < forms.mid_after)
            })
            .count();
        let mids: Vec<i8> = cells.iter().filter_map(|c| c.mid_change).collect();
        let mid_improved = mids.iter().filter(|m| **m == 1).count();
        if taught > 0 {
            mid.taught_pairs += mids.len();
            mid.taught_improved += mid_improved;
        } else {
            mid.untaught_pairs += mids.len();
            mid.untaught_improved += mid_improved;
        }
        let n_of = |v: i8| pairs.iter().filter(|c| c.change == Some(v)).count();
        family_rows.push(FamilyRow {
            id: f.id.clone(),
            title: f.title.clone(),
            lessons: f.lessons.len(),
            taught_before_mid: taught,
            people: pairs.len(),
            improved: n_of(1),
            same: n_of(0),
            worse: n_of(-1),
            mean_change: mean(&changes),
            mean_structure_change: mean(&structure),
            pre_clean: pairs
                .iter()
                .filter(|c| c.pre.as_ref().is_some_and(|i| i.clean))
                .count(),
            post_clean: pairs
                .iter()
                .filter(|c| c.post.as_ref().is_some_and(|i| i.clean))
                .count(),
            mid_people: mids.len(),
            mid_improved,
        });
    }

    let full: Vec<&PersonRow> = people
        .iter()
        .filter(|p| p.protocol == Protocol::Full && p.missing < p.families.len())
        .collect();
    let mut improved_counts: Vec<usize> = full.iter().map(|p| p.improved).collect();
    let median_improved = median(&mut improved_counts);
    let better = full.iter().filter(|p| p.improved > p.worse).count();
    let worse = full.iter().filter(|p| p.improved < p.worse).count();
    let ties = full.len() - better - worse;
    let sign_test = (better + worse > 0).then(|| SignTest {
        better,
        worse,
        ties,
        p_one_sided: sign_p(better, better + worse),
    });

    let lessons = lesson_rows(inp, &tour_pos);
    let criteria = criteria(inp, &people, &family_rows, median_improved);
    Report {
        schema: REPORT_SCHEMA,
        time_limit_s: forms.time_limit_ms / 1000,
        structure_step: forms.structure_step,
        mid_after: forms.mid_after,
        owner: inp.owner.to_owned(),
        people,
        families: family_rows,
        median_improved,
        sign_test,
        mid,
        lessons,
        criteria,
        notes,
        findings,
    }
}

fn lesson_rows(inp: &Inputs<'_>, tour_pos: &BTreeMap<&str, usize>) -> Vec<LessonRow> {
    let mut keys: Vec<String> = inp.tour.to_vec();
    let mut others: BTreeSet<String> = BTreeSet::new();
    for p in inp.people {
        others.extend(p.answers.lessons.iter().map(|l| l.key.clone()));
        others.extend(p.answers.notes.iter().filter_map(|n| n.lesson.clone()));
        if let Some(progress) = &p.progress {
            for (k, steps) in progress {
                if inp
                    .lessons
                    .get(k)
                    .is_some_and(|l| completed(l, Some(steps)))
                {
                    others.insert(k.clone());
                }
            }
        }
    }
    keys.extend(
        others
            .into_iter()
            .filter(|k| !tour_pos.contains_key(k.as_str())),
    );
    keys.into_iter()
        .map(|key| {
            let info = inp.lessons.get(&key);
            let mut row = LessonRow {
                title: info.map(|l| l.title.clone()),
                tour: tour_pos.get(key.as_str()).map(|i| i + 1),
                done: 0,
                new: 0,
                knew: 0,
                too_easy: 0,
                ok: 0,
                too_hard: 0,
                named: 0,
                notes: 0,
                ear_clean: 0,
                ear_done: 0,
                key,
            };
            for p in inp.people {
                let steps = p.progress.as_ref().and_then(|pr| pr.get(&row.key));
                if let Some(info) = info {
                    if completed(info, steps) {
                        row.done += 1;
                    }
                    for s in info.steps.iter().filter(|s| s.kind == "choose") {
                        if let Some(st) = steps.and_then(|st| st.get(&s.id))
                            && st.done
                        {
                            row.ear_done += 1;
                            row.ear_clean += usize::from(st.clean);
                        }
                    }
                }
                if let Some(l) = p.answers.lessons.iter().find(|l| l.key == row.key) {
                    match l.vote {
                        Some(Vote::New) => row.new += 1,
                        Some(Vote::Knew) => row.knew += 1,
                        None => {}
                    }
                    match l.pace {
                        Some(Pace::TooEasy) => row.too_easy += 1,
                        Some(Pace::Ok) => row.ok += 1,
                        Some(Pace::TooHard) => row.too_hard += 1,
                        None => {}
                    }
                }
                let mine = p
                    .answers
                    .notes
                    .iter()
                    .filter(|n| n.lesson.as_deref() == Some(row.key.as_str()));
                let mut any = false;
                let mut specific = false;
                for n in mine {
                    any = true;
                    specific |= n.coded == Some(Coded::Specific);
                }
                row.notes += usize::from(any);
                row.named += usize::from(specific);
            }
            row
        })
        .collect()
}

fn criteria(
    inp: &Inputs<'_>,
    people: &[PersonRow],
    families: &[FamilyRow],
    median_improved: Option<f64>,
) -> Vec<Criterion> {
    let t = inp.thresholds;
    let mut out = Vec::new();

    // 1. Every participant names a specific new thing in at least N lessons.
    let (verdict, detail) = if people.is_empty() {
        (Verdict::Pending, "no answers files yet".to_owned())
    } else {
        let all_met = people.iter().all(|p| p.named.len() >= t.lessons_named);
        let some_short = people
            .iter()
            .any(|p| p.named.len() + p.uncoded < t.lessons_named);
        let detail = people
            .iter()
            .map(|p| {
                if p.uncoded > 0 {
                    format!("{} {} (+{} uncoded)", p.id, p.named.len(), p.uncoded)
                } else {
                    format!("{} {}", p.id, p.named.len())
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let v = if all_met {
            Verdict::Pass
        } else if some_short {
            Verdict::Fail
        } else {
            Verdict::Pending
        };
        (v, format!("lessons with a named new thing: {detail}"))
    };
    out.push(Criterion {
        id: "named",
        text: format!(
            "every participant names a specific new thing in at least {} lessons (notes coded against each lesson's objective)",
            t.lessons_named
        ),
        verdict,
        detail,
    });

    // 2. The median full-protocol participant improves on at least K families; no family worse on average.
    let full: Vec<&PersonRow> = people
        .iter()
        .filter(|p| p.protocol == Protocol::Full)
        .collect();
    let missing: usize = full.iter().map(|p| p.missing).sum();
    let worse_families: Vec<&str> = families
        .iter()
        .filter(|f| f.mean_change.is_some_and(|m| m < 0.0))
        .map(|f| f.id.as_str())
        .collect();
    let (verdict, detail) = match median_improved {
        None => (
            Verdict::Pending,
            "no full-protocol participant with a pre- and post-test pair yet".to_owned(),
        ),
        Some(m) => {
            let met = m >= t.families_improved as f64 && worse_families.is_empty();
            let worse_text = if worse_families.is_empty() {
                "no family worse on average".to_owned()
            } else {
                format!("worse on average: {}", worse_families.join(", "))
            };
            let mut detail = format!(
                "median families improved {m} of {} over {} participant(s); {worse_text}",
                families.len(),
                full.iter().filter(|p| p.missing < p.families.len()).count()
            );
            let verdict = if missing > 0 {
                let _ = std::fmt::Write::write_fmt(
                    &mut detail,
                    format_args!(
                        "; {missing} pre/post pair(s) missing, so this is provisional ({})",
                        if met { "met so far" } else { "not met so far" }
                    ),
                );
                Verdict::Pending
            } else if met {
                Verdict::Pass
            } else {
                Verdict::Fail
            };
            (verdict, detail)
        }
    };
    out.push(Criterion {
        id: "families",
        text: format!(
            "the median participant improves on at least {} families, and no family gets worse on average",
            t.families_improved
        ),
        verdict,
        detail,
    });

    // 3. The owner names something new and passes a post-test item they failed before.
    let (verdict, detail) = match people.iter().find(|p| p.owner) {
        None => (
            Verdict::Pending,
            format!("no answers file for the owner ({})", inp.owner),
        ),
        Some(o) => {
            let named = !o.named.is_empty();
            let passed = o.passed_after_failing;
            let detail = format!(
                "{}: {} lesson(s) with a named new thing{}; {}",
                o.id,
                o.named.len(),
                if o.uncoded > 0 {
                    format!(" (+{} uncoded)", o.uncoded)
                } else {
                    String::new()
                },
                if o.protocol == Protocol::Tour {
                    "no tests (tour protocol)".to_owned()
                } else if passed {
                    "passed a post-test item failed on the pre-test".to_owned()
                } else {
                    format!(
                        "no post-test item passed that the pre-test failed ({} pair(s) missing)",
                        o.missing
                    )
                }
            );
            let can_still_name = named || o.uncoded > 0;
            let can_still_pass = passed || (o.protocol == Protocol::Full && o.missing > 0);
            let verdict = if named && passed {
                Verdict::Pass
            } else if !can_still_name || !can_still_pass {
                Verdict::Fail
            } else {
                Verdict::Pending
            };
            (verdict, detail)
        }
    };
    out.push(Criterion {
        id: "owner",
        text: "you name something new and pass a post-test item you failed before".to_owned(),
        verdict,
        detail,
    });
    out
}
