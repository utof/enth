//! GR2's inputs, read from text: the test forms (`enth.gr2-forms/1`), a person's answers (`enth.gr2-answers/1`),
//! their progress file (`enth.progress/<N>`, read leniently) and the lessons' titles, objectives and steps (the
//! authoring `pack.toml` and `lesson.toml`). Nothing here reads a file or a clock; the caller hands the text in.
//! docs/r2-owner.md ("The GR2 files") is the reference for the two GR2 formats.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gr1_report::code;

/// The forms file's schema string.
pub const FORMS_SCHEMA: &str = "enth.gr2-forms/1";
/// The answers file's schema string.
pub const ANSWERS_SCHEMA: &str = "enth.gr2-answers/1";

/// A test form: the pre- and post-test are A and B (counterbalanced: half the people take A first), M is the mid-test,
/// D the optional delayed test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub enum Form {
    /// One of the two parallel pre/post forms.
    A,
    /// The other parallel pre/post form.
    B,
    /// The mid-test, after the tour's first third.
    M,
    /// The delayed test, a week after the post-test (optional).
    D,
}

impl Form {
    /// The post-test's form for a pre-test form (A ↔ B); M and D have none.
    pub fn other(self) -> Option<Form> {
        match self {
            Form::A => Some(Form::B),
            Form::B => Some(Form::A),
            Form::M | Form::D => None,
        }
    }
}

impl fmt::Display for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Form::A => "A",
            Form::B => "B",
            Form::M => "M",
            Form::D => "D",
        })
    }
}

/// One family of skills: a generator category, the tour lessons that teach it, and one code per form.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    /// Its id (`oscillators`, `filter`, …).
    pub id: String,
    /// Its name in tables.
    pub title: String,
    /// The lessons that teach it (`pack/lesson`), in the tour's order.
    pub lessons: Vec<String>,
    /// Its codes by form, as written in the file.
    pub codes: BTreeMap<Form, String>,
}

/// The test forms: what counts as a pass, and which code tests which family on which form.
#[derive(Clone, Debug, PartialEq)]
pub struct Forms {
    /// Active time an item may take, in ms: a pass after it is not a clean pass.
    pub time_limit_ms: u64,
    /// Structure % points a family's item must move, with its pass unchanged, to count as better or worse.
    pub structure_step: f64,
    /// The mid-test comes after this many lessons of the tour track (lesson 0.1 included).
    pub mid_after: usize,
    /// The families, in the file's order.
    pub families: Vec<Family>,
    /// Each code's comparison form (`code::normalize`) → (family index, form).
    pub by_code: BTreeMap<String, (usize, Form)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormsFile {
    schema: String,
    time_limit_s: u64,
    structure_step: f64,
    mid_after: usize,
    families: Vec<FamilyFile>,
    items: Vec<ItemFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyFile {
    id: String,
    title: String,
    lessons: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    family: String,
    form: Form,
    code: String,
}

impl Forms {
    /// Read and check a forms file: its schema, one A and one B code per family (M and D optional, at most one
    /// each), every item's family declared, no code twice.
    pub fn from_toml(text: &str) -> Result<Forms, String> {
        let file: FormsFile = toml::from_str(text).map_err(|e| format!("forms: {e}"))?;
        if file.schema != FORMS_SCHEMA {
            return Err(format!(
                "forms: schema `{}`, this report reads `{FORMS_SCHEMA}`",
                file.schema
            ));
        }
        if file.time_limit_s == 0 {
            return Err("forms: time_limit_s must be above 0".to_owned());
        }
        if !(file.structure_step.is_finite() && file.structure_step > 0.0) {
            return Err("forms: structure_step must be above 0".to_owned());
        }
        let mut families: Vec<Family> = Vec::new();
        for f in file.families {
            if families.iter().any(|g| g.id == f.id) {
                return Err(format!("forms: family `{}` is declared twice", f.id));
            }
            families.push(Family {
                id: f.id,
                title: f.title,
                lessons: f.lessons,
                codes: BTreeMap::new(),
            });
        }
        let mut by_code = BTreeMap::new();
        for item in file.items {
            let Some(fi) = families.iter().position(|f| f.id == item.family) else {
                return Err(format!(
                    "forms: item {} names family `{}`, which is not declared",
                    item.code, item.family
                ));
            };
            let norm = code::normalize(&item.code);
            if norm.is_empty() {
                return Err(format!("forms: family `{}` has an empty code", item.family));
            }
            if by_code.insert(norm, (fi, item.form)).is_some() {
                return Err(format!("forms: code {} is listed twice", item.code));
            }
            if families[fi]
                .codes
                .insert(item.form, item.code.clone())
                .is_some()
            {
                return Err(format!(
                    "forms: family `{}` has two items on form {}",
                    item.family, item.form
                ));
            }
        }
        for f in &families {
            for form in [Form::A, Form::B] {
                if !f.codes.contains_key(&form) {
                    return Err(format!("forms: family `{}` has no form {form} item", f.id));
                }
            }
        }
        Ok(Forms {
            time_limit_ms: file.time_limit_s * 1000,
            structure_step: file.structure_step,
            mid_after: file.mid_after,
            families,
            by_code,
        })
    }
}

/// Which protocol a person followed (docs/r2-owner.md, "GR2's protocol").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Intake, pre-test, the tour (or the full path), mid-test, post-test, debrief.
    Full,
    /// The tour with per-lesson votes and the debrief; no tests.
    Tour,
}

/// A per-lesson vote: was the idea new?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Vote {
    /// "New to me."
    New,
    /// "I knew it."
    Knew,
}

/// How a lesson's pace felt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    /// Too easy or too slow.
    TooEasy,
    /// About right.
    Ok,
    /// Too hard or too fast.
    TooHard,
}

/// The coder's verdict on a "what did you learn?" note, against the lesson's objective.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coded {
    /// Names a specific new thing the lesson teaches.
    Specific,
    /// Says something was learned, but not what.
    Vague,
    /// Names nothing new, or something the lesson does not teach.
    None,
}

/// One "what did you learn?" note, from a lesson's block or the debrief.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Note {
    /// The lesson it is about (`pack/lesson`), if it names one.
    pub lesson: Option<String>,
    /// The text, as written.
    pub text: String,
    /// The coder's verdict; `None` until coded.
    pub coded: Option<Coded>,
    /// `lesson` or `debrief`.
    pub from: &'static str,
}

/// One lesson's answers.
#[derive(Clone, Debug, PartialEq)]
pub struct LessonAnswer {
    /// `pack/lesson`.
    pub key: String,
    /// New to me, or knew it.
    pub vote: Option<Vote>,
    /// Too easy, about right, too hard.
    pub pace: Option<Pace>,
}

/// A person's answers: intake, per-lesson votes and notes, the debrief.
#[derive(Clone, Debug, PartialEq)]
pub struct Answers {
    /// The anonymous id (`p1` is the owner by default).
    pub id: String,
    /// Which protocol they followed.
    pub protocol: Protocol,
    /// The pre-test's form (A or B; the post-test is the other), for the full protocol.
    pub pre_form: Option<Form>,
    /// Years making sounds with synths.
    pub years: Option<f64>,
    /// Confidence 1–5 per family id.
    pub confidence: BTreeMap<String, u8>,
    /// Per-lesson answers, in file order.
    pub lessons: Vec<LessonAnswer>,
    /// Every note, lesson blocks first, then the debrief's.
    pub notes: Vec<Note>,
    /// The overall pace from the debrief.
    pub pace: Option<Pace>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswersFile {
    schema: String,
    id: String,
    protocol: Protocol,
    #[serde(default)]
    pre_form: Option<String>,
    #[serde(default)]
    intake: Option<IntakeFile>,
    #[serde(default)]
    lessons: Vec<LessonFile>,
    #[serde(default)]
    debrief: Option<DebriefFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntakeFile {
    #[serde(default)]
    years: Option<f64>,
    #[serde(default)]
    synths: Option<String>,
    #[serde(default)]
    confidence: BTreeMap<String, i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LessonFile {
    key: String,
    #[serde(default)]
    vote: Option<String>,
    #[serde(default)]
    pace: Option<String>,
    #[serde(default)]
    learned: Option<String>,
    #[serde(default)]
    coded: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DebriefFile {
    #[serde(default)]
    pace: Option<String>,
    #[serde(default)]
    things: Vec<ThingFile>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ThingFile {
    Text(String),
    Coded {
        text: String,
        #[serde(default)]
        lesson: Option<String>,
        #[serde(default)]
        coded: Option<String>,
    },
}

/// An empty or blank string is "not answered".
fn answered(s: Option<&String>) -> Option<&str> {
    s.map(|s| s.trim()).filter(|s| !s.is_empty())
}

fn parse_vote(s: Option<&String>, at: &str) -> Result<Option<Vote>, String> {
    match answered(s) {
        None => Ok(None),
        Some("new") => Ok(Some(Vote::New)),
        Some("knew") => Ok(Some(Vote::Knew)),
        Some(other) => Err(format!("{at}: vote `{other}` is not `new` or `knew`")),
    }
}

fn parse_pace(s: Option<&String>, at: &str) -> Result<Option<Pace>, String> {
    match answered(s) {
        None => Ok(None),
        Some("too_easy") => Ok(Some(Pace::TooEasy)),
        Some("ok") => Ok(Some(Pace::Ok)),
        Some("too_hard") => Ok(Some(Pace::TooHard)),
        Some(other) => Err(format!(
            "{at}: pace `{other}` is not `too_easy`, `ok` or `too_hard`"
        )),
    }
}

fn parse_coded(s: Option<&String>, at: &str) -> Result<Option<Coded>, String> {
    match answered(s) {
        None => Ok(None),
        Some("specific") => Ok(Some(Coded::Specific)),
        Some("vague") => Ok(Some(Coded::Vague)),
        Some("none") => Ok(Some(Coded::None)),
        Some(other) => Err(format!(
            "{at}: coded `{other}` is not `specific`, `vague` or `none`"
        )),
    }
}

/// Whether `id` is a short anonymous id: 1 to 16 ASCII letters, digits, `-` or `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 16
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Answers {
    /// Read and check an answers file. Blank answers count as not given; a value outside its list is refused.
    pub fn from_toml(text: &str) -> Result<Answers, String> {
        let file: AnswersFile = toml::from_str(text).map_err(|e| e.to_string())?;
        if file.schema != ANSWERS_SCHEMA {
            return Err(format!(
                "schema `{}`, this report reads `{ANSWERS_SCHEMA}`",
                file.schema
            ));
        }
        if !valid_id(&file.id) {
            return Err(format!(
                "id `{}` is not a short anonymous id (letters, digits, - or _; at most 16)",
                file.id
            ));
        }
        let pre_form = match answered(file.pre_form.as_ref()) {
            None => None,
            Some("A") => Some(Form::A),
            Some("B") => Some(Form::B),
            Some(other) => return Err(format!("pre_form `{other}` is not `A` or `B`")),
        };
        if file.protocol == Protocol::Full && pre_form.is_none() {
            return Err(
                "a full-protocol answers file names its pre_form (`A` or `B`, the pre-test's form)"
                    .to_owned(),
            );
        }
        let (years, confidence) = match file.intake {
            None => (None, BTreeMap::new()),
            Some(intake) => {
                let _ = intake.synths; // free text for the reader's eyes; not counted
                if let Some(y) = intake.years
                    && !(y.is_finite() && (0.0..=80.0).contains(&y))
                {
                    return Err(format!("intake: years {y} is not between 0 and 80"));
                }
                let mut conf = BTreeMap::new();
                for (family, c) in intake.confidence {
                    let c = u8::try_from(c)
                        .ok()
                        .filter(|c| (1..=5).contains(c))
                        .ok_or_else(|| {
                            format!("intake: confidence for `{family}` is {c}, not 1 to 5")
                        })?;
                    conf.insert(family, c);
                }
                (intake.years, conf)
            }
        };
        let mut lessons = Vec::new();
        let mut notes = Vec::new();
        let mut seen = BTreeSet::new();
        for l in file.lessons {
            let at = format!("lesson {}", l.key);
            if !seen.insert(l.key.clone()) {
                return Err(format!("{at}: listed twice"));
            }
            lessons.push(LessonAnswer {
                key: l.key.clone(),
                vote: parse_vote(l.vote.as_ref(), &at)?,
                pace: parse_pace(l.pace.as_ref(), &at)?,
            });
            let coded = parse_coded(l.coded.as_ref(), &at)?;
            match answered(l.learned.as_ref()) {
                Some(text) => notes.push(Note {
                    lesson: Some(l.key),
                    text: text.to_owned(),
                    coded,
                    from: "lesson",
                }),
                None if coded.is_some() => {
                    return Err(format!("{at}: coded, but nothing is written in `learned`"));
                }
                None => {}
            }
        }
        let mut pace = None;
        if let Some(d) = file.debrief {
            pace = parse_pace(d.pace.as_ref(), "debrief")?;
            if d.things.len() > 3 {
                return Err(format!(
                    "debrief: {} things; the question asks for up to three",
                    d.things.len()
                ));
            }
            for t in d.things {
                let (text, lesson, coded) = match t {
                    ThingFile::Text(text) => (text, None, None),
                    ThingFile::Coded {
                        text,
                        lesson,
                        coded,
                    } => (text, lesson, coded),
                };
                let coded = parse_coded(coded.as_ref(), "debrief")?;
                let lesson = answered(lesson.as_ref()).map(str::to_owned);
                if text.trim().is_empty() {
                    continue;
                }
                notes.push(Note {
                    lesson,
                    text: text.trim().to_owned(),
                    coded,
                    from: "debrief",
                });
            }
        }
        Ok(Answers {
            id: file.id,
            protocol: file.protocol,
            pre_form,
            years,
            confidence,
            lessons,
            notes,
            pace,
        })
    }
}

/// One step's state in a progress file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepState {
    /// Passed, or read.
    pub done: bool,
    /// Passed at least once with no help.
    pub clean: bool,
}

/// What GR2 reads of a progress file: each lesson's steps.
pub type ProgressView = BTreeMap<String, BTreeMap<String, StepState>>;

/// Read a progress file leniently, as the plugin does (docs/learn.md, "Progress v1"): any `enth.progress/<N>`,
/// unknown fields ignored, an entry this reader cannot use skipped. Refused only when it is not a JSON object with an
/// `enth.progress/` schema and a `lessons` object.
pub fn progress_from_json(text: &str) -> Result<ProgressView, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON ({e})"))?;
    let schema = value.get("schema").and_then(Value::as_str).unwrap_or("");
    if !schema.starts_with("enth.progress/") {
        return Err(format!(
            "schema `{schema}`: not a progress file (enth.progress/<N>)"
        ));
    }
    let lessons = value
        .get("lessons")
        .and_then(Value::as_object)
        .ok_or("no `lessons` object")?;
    let mut out = ProgressView::new();
    for (key, l) in lessons {
        let Some(steps) = l.get("steps").and_then(Value::as_object) else {
            continue;
        };
        let mut view = BTreeMap::new();
        for (id, s) in steps {
            let flag = |name: &str| s.get(name).and_then(Value::as_bool).unwrap_or(false);
            let clean = flag("clean");
            view.insert(
                id.clone(),
                StepState {
                    done: flag("done") || clean,
                    clean,
                },
            );
        }
        out.insert(key.clone(), view);
    }
    Ok(out)
}

/// One step of a lesson: its id and kind (`listen`, `choose`, `read`, `do`, `recreate`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepInfo {
    /// Its id.
    pub id: String,
    /// Its kind.
    pub kind: String,
}

/// What GR2 shows of a lesson.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LessonInfo {
    /// Its title.
    pub title: String,
    /// What the player can do after it (the "what did you learn?" notes are coded against it).
    pub objective: String,
    /// Its steps in order.
    pub steps: Vec<StepInfo>,
}

impl LessonInfo {
    /// Read the fields GR2 needs from an authoring `lesson.toml` (the rest is `lesson-check`'s business).
    pub fn from_toml(text: &str) -> Result<LessonInfo, String> {
        let value: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let s = |k: &str| {
            value
                .get(k)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("no `{k}` string"))
        };
        let mut steps = Vec::new();
        for step in value
            .get("steps")
            .and_then(toml::Value::as_array)
            .ok_or("no `steps` array")?
        {
            let field = |k: &str| {
                step.get(k)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("a step without a `{k}` string"))
            };
            steps.push(StepInfo {
                id: field("id")?,
                kind: field("kind")?,
            });
        }
        Ok(LessonInfo {
            title: s("title")?,
            objective: s("objective")?,
            steps,
        })
    }
}

/// What GR2 reads of a `pack.toml`: its id, its lessons, its tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    /// The pack id.
    pub id: String,
    /// Its lesson ids, in order.
    pub lessons: Vec<String>,
    /// Its tracks: id → `pack/lesson` keys in order.
    pub tracks: BTreeMap<String, Vec<String>>,
}

impl PackInfo {
    /// Read the fields GR2 needs from an authoring `pack.toml`.
    pub fn from_toml(text: &str) -> Result<PackInfo, String> {
        let value: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let id = value
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or("no `id` string")?
            .to_owned();
        let strings = |v: Option<&toml::Value>| -> Vec<String> {
            v.and_then(toml::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let lessons = strings(value.get("lessons"));
        let mut tracks = BTreeMap::new();
        for t in value
            .get("tracks")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(tid) = t.get("id").and_then(toml::Value::as_str) {
                tracks.insert(tid.to_owned(), strings(t.get("lessons")));
            }
        }
        Ok(PackInfo {
            id,
            lessons,
            tracks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMS: &str = r#"
schema = "enth.gr2-forms/1"
time_limit_s = 180
structure_step = 10
mid_after = 2

[[families]]
id = "osc"
title = "Oscillators"
lessons = ["basics/waveforms"]

[[items]]
family = "osc"
form = "A"
code = "1040-1000-000Z-9V"

[[items]]
family = "osc"
form = "B"
code = "1040-1000-000z-ax"
"#;

    #[test]
    fn a_forms_file_reads_and_its_codes_compare_without_case_or_hyphens() {
        let f = Forms::from_toml(FORMS).unwrap();
        assert_eq!(f.time_limit_ms, 180_000);
        assert_eq!(f.families.len(), 1);
        assert_eq!(f.by_code[&code::normalize("10401000000ZAX")], (0, Form::B));
    }

    #[test]
    fn a_forms_file_without_a_b_item_or_with_a_code_twice_is_refused() {
        let no_b = FORMS.replace("form = \"B\"", "form = \"M\"");
        assert!(Forms::from_toml(&no_b).unwrap_err().contains("no form B"));
        let twice = FORMS.replace("1040-1000-000z-ax", "1040 1000 000z 9v");
        assert!(Forms::from_toml(&twice).unwrap_err().contains("twice"));
        let unknown = FORMS.replace(
            "family = \"osc\"\nform = \"B\"",
            "family = \"lfo\"\nform = \"B\"",
        );
        assert!(
            Forms::from_toml(&unknown)
                .unwrap_err()
                .contains("not declared")
        );
        let schema = FORMS.replace("gr2-forms/1", "gr2-forms/2");
        assert!(Forms::from_toml(&schema).unwrap_err().contains("schema"));
    }

    #[test]
    fn blank_answers_count_as_not_given_and_bad_values_are_named() {
        let text = r#"
schema = "enth.gr2-answers/1"
id = "p2"
protocol = "tour"
pre_form = ""

[[lessons]]
key = "basics/waveforms"
vote = "new"
pace = ""
learned = "  "
coded = ""

[debrief]
things = ["The saw is buzzy", { text = "Cutoff darkens", lesson = "basics/cutoff", coded = "specific" }, ""]
"#;
        let a = Answers::from_toml(text).unwrap();
        assert_eq!(a.pre_form, None);
        assert_eq!(a.lessons[0].vote, Some(Vote::New));
        assert_eq!(a.lessons[0].pace, None);
        assert_eq!(a.notes.len(), 2);
        assert_eq!(a.notes[1].coded, Some(Coded::Specific));
        assert_eq!(a.notes[1].lesson.as_deref(), Some("basics/cutoff"));
        let bad = text.replace("vote = \"new\"", "vote = \"maybe\"");
        assert!(
            Answers::from_toml(&bad)
                .unwrap_err()
                .contains("vote `maybe`")
        );
        let full = text.replace("protocol = \"tour\"", "protocol = \"full\"");
        assert!(Answers::from_toml(&full).unwrap_err().contains("pre_form"));
        let name = text.replace("id = \"p2\"", "id = \"Jane Doe\"");
        assert!(Answers::from_toml(&name).unwrap_err().contains("anonymous"));
    }

    #[test]
    fn a_progress_file_is_read_leniently() {
        let text = r#"{"schema":"enth.progress/3","lessons":{"basics/cutoff":{"steps":{"hear":{"done":true},"try":{"clean":true,"best":97,"time_ms":1},"odd":{"done":"yes"}},"votes":"new"},"x":7},"review":{}}"#;
        let p = progress_from_json(text).unwrap();
        let steps = &p["basics/cutoff"];
        assert!(steps["hear"].done && !steps["hear"].clean);
        assert!(steps["try"].done && steps["try"].clean);
        assert!(!steps["odd"].done);
        assert!(!p.contains_key("x"));
        assert!(progress_from_json(r#"{"schema":"enth.round-log/1"}"#).is_err());
    }
}
