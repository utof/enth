//! The agent lesson track (lane EV3): an agent completes whole lessons of a course pack from the lesson text, through
//! `enth lesson` and the patch CLI, and a judge says which steps it got. `cargo xtask agent-eval lesson`.
//!
//! It is the text goals' harness with a different task, not a second harness: the same wrapper (in its lesson mode:
//! cost units, `--reveal` and the pack's files refused), the same headless Claude Code run, the same fake agents behind
//! `--dry-run`, the same transcripts and summaries. What differs:
//!
//! - **Goals** (`content/agent-evals/lesson-goals/<id>.toml`): one lesson of a pack, with its steps pinned (a pack that
//!   drifted is refused, so a result names the lesson it was measured on) and a budget in cost units.
//! - **The trial directory** holds the wrapper and a copy of the pack (`pack/`). The judge uses the pristine pack.
//! - **The answer** is a list of `{step, patch}` and `{step, choice}`; read and listen steps need none.
//! - **The judge** grades each step on its own: a do step by `enth lesson check`, a round by `enth compare
//!   --challenge` (a round the judge writes itself from the pristine pack, never the agent's file), an ear question by
//!   the pack's recorded answer; and for all patches, the step's locked and editable controls (`lesson check` does not
//!   enforce them: the editor does). A lesson passes when every graded step does.
//! - **Assistance** is recorded, not forbidden: a step is *assisted* when the log shows `lesson hint` or `lesson
//!   closest` (or `suggest`/`match` on its round) for it, as the player marks a step at its first hint.
//! - **Fake agents** (`lesson_fake`): `witness` (the pack's own solutions: the proof that every lesson is reachable) and
//!   `follower` (the floor: applies the hints' moves and nothing else).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::goal::fill;
use super::lesson_fake;
use super::run::{
    ClaudeRun, Env, Opts, TokensOut, build_env, command_version, git_line, install_wrapper,
    spawn_claude_with,
};
use super::stream::{self, StreamSummary};
use super::util::{self, median, round4};
use super::wrap::{self, Call, WrapCfg};

/// The schema string every lesson goal file carries.
pub const GOAL_SCHEMA: &str = "enth.agent-eval-lesson/1";
/// The schema string of `lesson-track.toml`.
pub const SET_SCHEMA: &str = "enth.agent-eval-lesson-set/1";
/// The trial result schema.
pub const TRIAL_SCHEMA: &str = "enth.agent-eval-lesson.trial/1";
/// The summary schema.
pub const SUMMARY_SCHEMA: &str = "enth.agent-eval-lesson.summary/1";

// ---------------------------------------------------------------------------------------------------------------
// The set on disk
// ---------------------------------------------------------------------------------------------------------------

/// What one trial may spend.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonBudget {
    /// Cost units (see the wrapper's lesson mode); the wrapper refuses a call that does not fit.
    pub units: u32,
    /// Agent turns (`--max-turns`).
    pub turns: u32,
    /// Estimated dollars (`--max-budget-usd`).
    pub usd: f64,
}

/// A goal: one lesson of a pack.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    dead_code,
    reason = "format fields (`added`, `tags`, `rationale`) are for people and tools that read the goal files"
)]
pub struct LessonGoal {
    pub schema: String,
    pub id: String,
    pub added: String,
    /// 1 (basic) to 5.
    pub difficulty: u8,
    pub tags: Vec<String>,
    /// The pack, relative to the workspace root.
    pub pack: String,
    /// The lesson's id in the pack.
    pub lesson: String,
    /// The lesson's steps as `id:kind`, in order.
    pub steps: Vec<String>,
    /// For maintainers; never shown to the agent.
    pub rationale: String,
    pub budget: LessonBudget,
}

/// `lesson-track.toml`: the track's version. Changing a prompt, a goal's steps or budget, the wrapper's unit costs or
/// the judge's rules changes it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetMeta {
    pub schema: String,
    pub eval_version: u32,
    /// What changed in this version, newest first (read by people; the harness checks that it parses).
    #[allow(dead_code, reason = "read by people")]
    pub changelog: Vec<String>,
}

/// The whole track.
#[derive(Clone, Debug)]
pub struct LessonSet {
    /// `content/agent-evals`.
    pub dir: PathBuf,
    pub meta: SetMeta,
    pub goals: Vec<LessonGoal>,
}

impl LessonSet {
    /// The prompt template named `name` (`lesson`, `lesson-system`).
    pub fn prompt(&self, name: &str) -> Result<String, String> {
        let path = self.dir.join("prompts").join(format!("{name}.md"));
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A fingerprint of the two prompts, for the result header.
    pub fn prompt_fingerprint(&self) -> Result<String, String> {
        let mut all = String::new();
        for name in ["lesson-system", "lesson"] {
            all.push_str(&self.prompt(name)?);
            all.push('\u{1}');
        }
        Ok(format!("{:016x}", util::fnv1a64(all.as_bytes())))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn validate(goal: &LessonGoal, stem: &str) -> Result<(), String> {
    if !valid_id(&goal.id) {
        return Err(format!(
            "{}: id must be lowercase letters, digits and `-`",
            goal.id
        ));
    }
    if goal.schema != GOAL_SCHEMA {
        return Err(format!("{stem}: schema must be {GOAL_SCHEMA}"));
    }
    if goal.id != stem {
        return Err(format!(
            "{stem}: id `{}` does not match the file name",
            goal.id
        ));
    }
    if !(1..=5).contains(&goal.difficulty) {
        return Err(format!("{stem}: difficulty must be 1..=5"));
    }
    if !valid_id(&goal.lesson) {
        return Err(format!(
            "{stem}: lesson `{}` is not a lesson id",
            goal.lesson
        ));
    }
    for (what, text) in [("pack", &goal.pack), ("rationale", &goal.rationale)] {
        if text.trim().is_empty() {
            return Err(format!("{stem}: {what} is empty"));
        }
    }
    if goal.pack.starts_with('/') || goal.pack.split('/').any(|c| c == "..") {
        return Err(format!(
            "{stem}: pack must be a path inside the workspace, not `{}`",
            goal.pack
        ));
    }
    if goal.steps.is_empty() {
        return Err(format!("{stem}: steps is empty"));
    }
    for s in &goal.steps {
        let ok = s.split_once(':').is_some_and(|(id, kind)| {
            valid_id(id) && matches!(kind, "read" | "listen" | "do" | "recreate" | "choose")
        });
        if !ok {
            return Err(format!("{stem}: step `{s}` is not `id:kind`"));
        }
    }
    if goal.budget.units == 0 || goal.budget.turns == 0 || goal.budget.usd <= 0.0 {
        return Err(format!(
            "{stem}: budget needs positive units, turns and usd"
        ));
    }
    Ok(())
}

/// Load `lesson-track.toml` and every `lesson-goals/*.toml` under `dir`, sorted by difficulty then id.
pub fn load(dir: &Path) -> Result<LessonSet, String> {
    let meta_path = dir.join("lesson-track.toml");
    let meta_text =
        std::fs::read_to_string(&meta_path).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let meta: SetMeta =
        toml::from_str(&meta_text).map_err(|e| format!("{}: {e}", meta_path.display()))?;
    if meta.schema != SET_SCHEMA {
        return Err(format!(
            "{}: schema must be {SET_SCHEMA}",
            meta_path.display()
        ));
    }
    let goals_dir = dir.join("lesson-goals");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&goals_dir)
        .map_err(|e| format!("{}: {e}", goals_dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    let mut goals = Vec::new();
    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let goal: LessonGoal =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        validate(&goal, &stem)?;
        goals.push(goal);
    }
    if goals.is_empty() {
        return Err(format!("{}: no goals", goals_dir.display()));
    }
    goals.sort_by(|a, b| (a.difficulty, &a.id).cmp(&(b.difficulty, &b.id)));
    Ok(LessonSet {
        dir: dir.to_owned(),
        meta,
        goals,
    })
}

/// Replace the placeholders of a prompt template.
pub fn render_prompt(template: &str, goal: &LessonGoal) -> String {
    fill(
        template,
        &[
            ("lesson", &goal.lesson),
            ("pack", wrap::PACK_DIR),
            ("units", &goal.budget.units.to_string()),
        ],
    )
}

// ---------------------------------------------------------------------------------------------------------------
// What the pack says about a lesson
// ---------------------------------------------------------------------------------------------------------------

/// One step as the judge and the fake agents need it (from `enth lesson show`, and `--reveal` for the answers).
#[derive(Clone, Debug)]
pub struct StepFacts {
    pub number: usize,
    pub id: String,
    pub kind: String,
    /// How many authored hints the step has.
    pub hints: usize,
    /// The controls the step lets the player edit (a round's: its challenge's), as selector text. Empty: any.
    pub editable: Vec<String>,
    /// The controls it locks.
    pub locked: Vec<String>,
    /// A listen step's examples or a choose step's options, by label.
    pub labels: Vec<String>,
    /// A choose step's right option (an index into `labels`); from `--reveal`.
    pub answer: Option<usize>,
    /// A do step's witness patch, relative to the pack; from `--reveal`.
    pub witness: Option<String>,
    /// The step names a start patch of its own.
    pub has_start: bool,
}

/// A lesson of a pack.
#[derive(Clone, Debug)]
pub struct LessonFacts {
    /// The pristine pack directory the judge uses.
    pub pack_dir: PathBuf,
    pub steps: Vec<StepFacts>,
}

/// Run the real `enth` in `cwd`: its exit code, its stdout parsed as JSON (`Null` when it is not), and its stderr.
fn enth(env: &Env, cwd: &Path, args: &[&str]) -> Result<(i32, Value, String), String> {
    let out = Command::new(&env.enth)
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", env.enth.display()))?;
    let value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    Ok((
        out.status.code().unwrap_or(1),
        value,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn strings_of(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Read a lesson's steps from the pristine pack and check them against the goal's pinned list.
pub fn facts(env: &Env, root: &Path, goal: &LessonGoal) -> Result<LessonFacts, String> {
    let pack_dir = root.join(&goal.pack);
    let pack = pack_dir.to_string_lossy().into_owned();
    let show = |reveal: bool| -> Result<Value, String> {
        let mut args = vec![
            "lesson",
            "show",
            pack.as_str(),
            "--lesson",
            goal.lesson.as_str(),
        ];
        if reveal {
            args.push("--reveal");
        }
        let (code, v, err) = enth(env, root, &args)?;
        if code != 0 {
            return Err(format!(
                "{}: `enth lesson show` exited {code}: {}",
                goal.id,
                if v.is_null() { err } else { v.to_string() }
            ));
        }
        Ok(v)
    };
    let plain = show(false)?;
    let revealed = show(true)?;
    let steps_of = |v: &Value| -> Vec<Value> {
        v.pointer("/lessons/0/steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (plain, revealed) = (steps_of(&plain), steps_of(&revealed));
    let mut steps = Vec::new();
    for (i, s) in plain.iter().enumerate() {
        let text = |k: &str| s.get(k).and_then(Value::as_str).map(str::to_owned);
        let kind = text("kind").unwrap_or_default();
        let constraints = s.pointer("/challenge/constraints").unwrap_or(&Value::Null);
        let (editable, locked) = if kind == "recreate" {
            (
                strings_of(constraints.get("editable").unwrap_or(&Value::Null)),
                strings_of(constraints.get("locked").unwrap_or(&Value::Null)),
            )
        } else {
            (
                strings_of(s.get("editable").unwrap_or(&Value::Null)),
                strings_of(s.get("locked").unwrap_or(&Value::Null)),
            )
        };
        let r = revealed.get(i).unwrap_or(&Value::Null);
        steps.push(StepFacts {
            number: i + 1,
            id: text("id").unwrap_or_default(),
            kind,
            hints: s.get("hints").and_then(Value::as_array).map_or(0, Vec::len),
            editable,
            locked,
            labels: strings_of(s.get("examples").unwrap_or(&Value::Null)),
            answer: r.get("answer").and_then(Value::as_u64).map(|n| n as usize),
            witness: r.get("witness").and_then(Value::as_str).map(str::to_owned),
            has_start: s.get("start").is_some_and(|x| !x.is_null()),
        });
    }
    let have: Vec<String> = steps
        .iter()
        .map(|s| format!("{}:{}", s.id, s.kind))
        .collect();
    if have != goal.steps {
        return Err(format!(
            "{}: the pack's lesson `{}` has drifted from the goal. The goal pins [{}]; the pack has [{}]. Update the goal's steps and bump the track's version",
            goal.id,
            goal.lesson,
            goal.steps.join(", "),
            have.join(", ")
        ));
    }
    Ok(LessonFacts { pack_dir, steps })
}

// ---------------------------------------------------------------------------------------------------------------
// The judge
// ---------------------------------------------------------------------------------------------------------------

/// One step as judged.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepResult {
    pub step: String,
    pub kind: String,
    /// The judge grades it (`do`, `recreate` and `choose` steps; `read` and `listen` steps have nothing to hand in).
    pub graded: bool,
    pub pass: bool,
    /// `n/a`, `pass`, `fail`, `missing` (not in the answer), `locked` (it changes a control the step does not allow)
    /// or `error` (the judge could not run).
    pub status: String,
    pub detail: String,
    /// `lesson hint` or `closest` (or `suggest` or `match` on its round) was used on the step.
    pub assisted: bool,
}

/// Whether the selector text `sel` covers the change to `target`. A parameter selector covers that parameter; `m.*`
/// covers the module, its parameters and the routes from it or into it; a route selector covers that route. The
/// challenge's own check (`enth_challenge::v2::touches`) is the exact rule; this reads `enth diff`'s targets.
pub fn covers(sel: &str, target: &str) -> bool {
    let sel = sel.trim();
    if sel == target {
        return true;
    }
    let Some(module) = sel.strip_suffix(".*") else {
        return false;
    };
    let in_module = |t: &str| {
        t == module
            || t.strip_prefix(module)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    in_module(target) || target.split(" -> ").any(in_module)
}

/// The changes (targets) a step does not allow: locked ones, and ones its `editable` list leaves out when it has one.
pub fn violations(changes: &[String], editable: &[String], locked: &[String]) -> Vec<String> {
    changes
        .iter()
        .filter(|t| {
            locked.iter().any(|s| covers(s, t))
                || (!editable.is_empty() && !editable.iter().any(|s| covers(s, t)))
        })
        .cloned()
        .collect()
}

/// The first two words of a call (`lesson hint`, `suggest`), after any leading `--format X`.
fn verbs(argv: &[String]) -> (Option<&str>, Option<&str>) {
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--format" => i += 2,
            a if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    (
        argv.get(i).map(String::as_str),
        argv.get(i + 1).map(String::as_str),
    )
}

/// Which steps the call log shows help on: `lesson hint|closest ... --step X`, and `suggest|match --challenge F`
/// where F is the round file a `lesson start --step X --challenge-out F` call wrote.
pub fn assisted_steps(calls: &[Call], steps: &[StepFacts]) -> Vec<String> {
    let resolve = |which: &str| {
        steps
            .iter()
            .find(|s| s.id == which || s.number.to_string() == which)
            .map(|s| s.id.clone())
    };
    let value_of = |argv: &[String], flag: &str| {
        argv.iter()
            .position(|a| a == flag)
            .and_then(|i| argv.get(i + 1))
            .cloned()
    };
    let mut rounds: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for c in calls
        .iter()
        .filter(|c| !c.refused && c.exit != 2 && c.exit != 3)
    {
        let step = value_of(&c.argv, "--step").and_then(|s| resolve(&s));
        match verbs(&c.argv) {
            (Some("lesson"), Some("start")) => {
                if let (Some(file), Some(step)) = (value_of(&c.argv, "--challenge-out"), step) {
                    rounds.insert(file, step);
                }
            }
            (Some("lesson"), Some("hint" | "closest")) => out.extend(step),
            (Some("suggest" | "match"), _) => {
                if let Some(step) = value_of(&c.argv, "--challenge").and_then(|f| rounds.get(&f)) {
                    out.push(step.clone());
                }
            }
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The final patch or choice the answer names for a step, by id or number.
fn answer_for<'a>(answer: Option<&'a Value>, step: &StepFacts) -> Option<&'a Value> {
    answer?.get("steps")?.as_array()?.iter().find(|e| {
        e.get("step").is_some_and(|s| {
            s.as_str() == Some(step.id.as_str())
                || s.as_u64() == Some(step.number as u64)
                || s.as_str() == Some(step.number.to_string().as_str())
        })
    })
}

/// A patch the agent named, inside its working directory and a file.
fn inside(tmp: &Path, p: &str) -> Option<PathBuf> {
    let path = if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        tmp.join(p)
    };
    let ok = match (path.canonicalize(), tmp.canonicalize()) {
        (Ok(a), Ok(b)) => a.starts_with(b),
        _ => false,
    };
    (ok && path.is_file()).then_some(path)
}

fn step_result(step: &StepFacts, status: &str, detail: impl Into<String>) -> StepResult {
    StepResult {
        step: step.id.clone(),
        kind: step.kind.clone(),
        graded: true,
        pass: status == "pass",
        status: status.to_owned(),
        detail: detail.into(),
        assisted: false,
    }
}

/// What the judge reads: the machine, the pristine pack, the trial's directories and the agent's answer.
pub struct JudgeCx<'a> {
    pub env: &'a Env,
    pub goal: &'a LessonGoal,
    pub facts: &'a LessonFacts,
    /// The trial directory (the judge's files go in `judge/`, the agent's patches in `answers/`).
    pub dir: &'a Path,
    /// The agent's working directory.
    pub tmp: &'a Path,
    pub answer: Option<&'a Value>,
}

/// Judge every step.
pub fn judge(cx: &JudgeCx<'_>, calls: &[Call]) -> Result<Vec<StepResult>, String> {
    let assisted = assisted_steps(calls, &cx.facts.steps);
    let judge_dir = cx.dir.join("judge");
    let answers_dir = cx.dir.join("answers");
    for d in [&judge_dir, &answers_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let pack = cx.facts.pack_dir.to_string_lossy().into_owned();
    let mut out = Vec::new();
    for step in &cx.facts.steps {
        let mut r = match step.kind.as_str() {
            "do" | "recreate" => judge_patch(cx, step, &pack, &judge_dir, &answers_dir)?,
            "choose" => judge_choice(cx, step),
            _ => StepResult {
                step: step.id.clone(),
                kind: step.kind.clone(),
                graded: false,
                pass: true,
                status: "n/a".to_owned(),
                detail: String::new(),
                assisted: false,
            },
        };
        r.assisted = assisted.contains(&step.id);
        out.push(r);
    }
    Ok(out)
}

fn judge_choice(cx: &JudgeCx<'_>, step: &StepFacts) -> StepResult {
    let Some(entry) = answer_for(cx.answer, step) else {
        return step_result(step, "missing", "no answer for this step");
    };
    let Some(choice) = entry.get("choice").and_then(Value::as_str) else {
        return step_result(step, "missing", "the entry has no `choice` label");
    };
    let Some(right) = step.answer.and_then(|i| step.labels.get(i)) else {
        return step_result(step, "error", "the pack records no answer for this step");
    };
    if choice.trim().eq_ignore_ascii_case(right) {
        step_result(step, "pass", format!("chose {choice}"))
    } else {
        step_result(step, "fail", format!("chose {choice}"))
    }
}

fn judge_patch(
    cx: &JudgeCx<'_>,
    step: &StepFacts,
    pack: &str,
    judge_dir: &Path,
    answers_dir: &Path,
) -> Result<StepResult, String> {
    let Some(entry) = answer_for(cx.answer, step) else {
        return Ok(step_result(step, "missing", "no answer for this step"));
    };
    let Some(name) = entry.get("patch").and_then(Value::as_str) else {
        return Ok(step_result(
            step,
            "missing",
            "the entry has no `patch` file",
        ));
    };
    let Some(src) = inside(cx.tmp, name) else {
        return Ok(step_result(
            step,
            "missing",
            format!("`{name}` is not a file in the working directory"),
        ));
    };
    let copy = answers_dir.join(format!("{}.patch.json", step.id));
    std::fs::copy(&src, &copy).map_err(|e| format!("{}: {e}", src.display()))?;
    let copy_s = copy.to_string_lossy().into_owned();
    let lesson = cx.goal.lesson.as_str();

    // The patch the step opens with, which the editor would have loaded: the pack's own start, or a round's. A step
    // that carries on from the one before has none to read here.
    let is_round = step.kind == "recreate";
    let start = judge_dir.join(format!("{}.start.patch.json", step.id));
    let round = judge_dir.join(format!("{}.challenge.json", step.id));
    let has_start = is_round || step.has_start;
    if has_start {
        let (start_s, round_s) = (start.to_string_lossy(), round.to_string_lossy());
        let mut args = vec![
            "lesson", "start", pack, "--lesson", lesson, "--step", &step.id, "--out", &start_s,
        ];
        if is_round {
            args.extend(["--challenge-out", &round_s]);
        }
        let (code, v, err) = enth(cx.env, cx.dir, &args)?;
        if code != 0 {
            return Ok(step_result(
                step,
                "error",
                format!("lesson start exited {code}: {}", brief(&v, &err)),
            ));
        }
    }
    let mut result = if is_round {
        score_round(cx, step, &copy_s, &round, judge_dir)?
    } else {
        check_do(
            cx,
            step,
            pack,
            &copy_s,
            has_start.then_some(&start),
            judge_dir,
        )?
    };
    // The editor would not have let the player change what the step locks; `lesson check` and `compare` do not
    // look. Only a step with a start (or a round) has one to compare with.
    if result.pass && has_start {
        result = lock_check(cx, step, &start, &copy_s, result)?;
    }
    Ok(result)
}

/// `enth lesson check`, with `--from` the step's start when it has one (a `touched` predicate needs it).
fn check_do(
    cx: &JudgeCx<'_>,
    step: &StepFacts,
    pack: &str,
    patch: &str,
    start: Option<&PathBuf>,
    judge_dir: &Path,
) -> Result<StepResult, String> {
    let start_s = start.map(|s| s.to_string_lossy().into_owned());
    let mut args = vec![
        "lesson",
        "check",
        pack,
        "--lesson",
        cx.goal.lesson.as_str(),
        "--step",
        step.id.as_str(),
        patch,
    ];
    if let Some(s) = &start_s {
        args.extend(["--from", s.as_str()]);
    }
    let (code, v, err) = enth(cx.env, cx.dir, &args)?;
    std::fs::write(
        judge_dir.join(format!("{}.check.json", step.id)),
        v.to_string(),
    )
    .map_err(|e| e.to_string())?;
    Ok(match code {
        0 => step_result(step, "pass", "the step's check passes"),
        1 => {
            let failing: Vec<String> = v
                .get("sounds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|s| s.get("pass") == Some(&Value::Bool(false)))
                        .filter_map(|s| s.get("item").and_then(Value::as_str).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let predicates = v
                .get("predicates")
                .and_then(Value::as_array)
                .map_or(0, |a| {
                    a.iter()
                        .filter(|p| p.get("pass") == Some(&Value::Bool(false)))
                        .count()
                });
            step_result(
                step,
                "fail",
                format!(
                    "failing: {predicates} patch check(s), sound items [{}]",
                    failing.join(", ")
                ),
            )
        }
        _ => step_result(
            step,
            "error",
            format!("lesson check exited {code}: {}", brief(&v, &err)),
        ),
    })
}

/// `enth compare --challenge` on the round the judge wrote itself.
fn score_round(
    cx: &JudgeCx<'_>,
    step: &StepFacts,
    patch: &str,
    round: &Path,
    judge_dir: &Path,
) -> Result<StepResult, String> {
    let (code, v, err) = enth(
        cx.env,
        cx.dir,
        &["compare", patch, "--challenge", &round.to_string_lossy()],
    )?;
    std::fs::write(
        judge_dir.join(format!("{}.compare.json", step.id)),
        v.to_string(),
    )
    .map_err(|e| e.to_string())?;
    // The round passes on its sound score (`combined` also weighs the structure, and does not decide).
    let score = v.get("sound").and_then(Value::as_f64);
    let mark = v.get("pass_mark").and_then(Value::as_f64);
    let detail = match (score, mark) {
        (Some(s), Some(m)) => format!("sound {s:.1} % of pass mark {m:.0}"),
        _ => String::new(),
    };
    Ok(match code {
        0 => step_result(step, "pass", detail),
        1 => step_result(step, "fail", detail),
        _ => step_result(
            step,
            "error",
            format!("compare exited {code}: {}", brief(&v, &err)),
        ),
    })
}

/// A passing patch that changed a control the step locks (or leaves out of its editable list) does not count.
fn lock_check(
    cx: &JudgeCx<'_>,
    step: &StepFacts,
    start: &Path,
    patch: &str,
    passed: StepResult,
) -> Result<StepResult, String> {
    let (code, diff, err) = enth(cx.env, cx.dir, &["diff", &start.to_string_lossy(), patch])?;
    if code != 0 {
        return Ok(step_result(
            step,
            "error",
            format!("diff exited {code}: {}", brief(&diff, &err)),
        ));
    }
    let changes: Vec<String> = diff
        .get("changes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                // A rename or a new description touches no control (`SetMeta` touches nothing).
                .filter(|c| c.get("op").and_then(Value::as_str) != Some("meta"))
                .filter_map(|c| c.get("target").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let bad = violations(&changes, &step.editable, &step.locked);
    Ok(if bad.is_empty() {
        passed
    } else {
        step_result(
            step,
            "locked",
            format!("changes what the step does not allow: {}", bad.join(", ")),
        )
    })
}

/// A short reason out of a CLI error object or stderr.
fn brief(v: &Value, stderr: &str) -> String {
    v.get("message")
        .and_then(Value::as_str)
        .map_or_else(|| stderr.trim().chars().take(200).collect(), str::to_owned)
}

// ---------------------------------------------------------------------------------------------------------------
// Trials
// ---------------------------------------------------------------------------------------------------------------

/// What one trial produced: the record of a single run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LessonTrial {
    pub schema: String,
    pub eval_version: u32,
    pub goal: String,
    pub lesson: String,
    pub trial: u32,
    pub runner: String,
    pub model: Option<String>,
    pub date: String,
    pub claude_code_version: Option<String>,
    /// `ok`, `agent_error`, `max_turns`, `timeout` or `no_answer`.
    pub status: String,
    pub tokens: TokensOut,
    pub cost_usd: Option<f64>,
    pub cost_is_estimate: bool,
    pub turns: Option<u64>,
    pub wall_s: f64,
    pub calls: u32,
    pub invalid_calls: u32,
    pub refused_calls: u32,
    /// Cost units spent (the wrapper's count, at most the budget).
    pub units: u32,
    /// Every graded step passed.
    pub pass: bool,
    pub graded: u32,
    pub passed: u32,
    /// Graded steps that passed without a hint, a closest search or a suggestion on them.
    pub passed_unassisted: u32,
    pub steps: Vec<StepResult>,
    pub dir: String,
    pub warnings: Vec<String>,
}

/// Everything a trial needs from the run.
pub struct LessonCx<'a> {
    pub env: &'a Env,
    pub set: &'a LessonSet,
    pub opts: &'a Opts,
    pub run_dir: &'a Path,
    pub stamp: &'a str,
}

fn answer_schema() -> String {
    json!({
        "type": "object",
        "required": ["steps"],
        "properties": {
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["step"],
                    "properties": {
                        "step": {"type": "string", "description": "the step's id"},
                        "patch": {"type": "string", "description": "path of the patch file for a do or recreate step"},
                        "choice": {"type": "string", "description": "the label of your answer to a choose step"},
                    },
                },
            },
            "explanation": {"type": "string"},
        },
    })
    .to_string()
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn new_tmp(stamp: &str, goal: &str, trial: u32) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join(format!(
        "enth-agent-eval-lesson-{}-{stamp}-{goal}-t{trial}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    Ok(tmp)
}

/// One trial, start to finish. The trial directory gets the transcript, the call log, the judge's files, the agent's
/// patches as judged and `result.json`.
pub fn run_trial(
    cx: &LessonCx<'_>,
    goal: &LessonGoal,
    facts: &LessonFacts,
    trial: u32,
) -> Result<LessonTrial, String> {
    let LessonCx {
        env,
        set,
        opts,
        run_dir,
        stamp,
        ..
    } = *cx;
    let dir = run_dir.join(&goal.id).join(format!("t{trial}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = new_tmp(stamp, &goal.id, trial)?;
    copy_dir(&facts.pack_dir, &tmp.join(wrap::PACK_DIR))?;
    let cfg = WrapCfg {
        calls: dir.join("enth-calls.jsonl"),
        state: dir.join("budget.state"),
        enth: env.enth.clone(),
        budget: goal.budget.units,
        lesson: true,
    };
    let _ = std::fs::remove_file(&cfg.calls);
    let _ = std::fs::remove_file(&cfg.state);
    install_wrapper(&tmp, &cfg, &env.xtask)?;

    let prompt = render_prompt(&set.prompt("lesson")?, goal);
    let system = render_prompt(&set.prompt("lesson-system")?, goal);
    std::fs::write(dir.join("prompt.md"), &prompt).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("system.md"), &system).map_err(|e| e.to_string())?;

    let started = Instant::now();
    let (exit, timed_out, text) = if opts.dry_run {
        let text = lesson_fake::run(
            opts.fake,
            &lesson_fake::FakeCx {
                wrapper: &tmp.join("enth"),
                tmp: &tmp,
                goal,
                facts,
                trial,
                prompt_bytes: prompt.len() + system.len(),
            },
        )?;
        std::fs::write(dir.join("stream.jsonl"), &text).map_err(|e| e.to_string())?;
        (Some(0), false, text)
    } else {
        spawn_claude_with(&ClaudeRun {
            env,
            model: &opts.model,
            timeout_min: opts.timeout_min,
            turns: goal.budget.turns,
            usd: goal.budget.usd,
            schema: &answer_schema(),
            tmp: &tmp,
            dir: &dir,
            prompt: &prompt,
        })?
    };
    let wall_s = started.elapsed().as_secs_f64();
    let summary: StreamSummary = stream::parse(&text);
    let calls = wrap::read_calls(&cfg.calls);
    let mut warnings = summary.warnings.clone();
    if summary.structured.is_none() {
        warnings.push("the agent gave no structured answer".to_owned());
    }

    let steps = judge(
        &JudgeCx {
            env,
            goal,
            facts,
            dir: &dir,
            tmp: &tmp,
            answer: summary.structured.as_ref(),
        },
        &calls,
    )?;
    let graded = steps.iter().filter(|s| s.graded).count() as u32;
    let passed = steps.iter().filter(|s| s.graded && s.pass).count() as u32;
    let passed_unassisted = steps
        .iter()
        .filter(|s| s.graded && s.pass && !s.assisted)
        .count() as u32;
    let result = LessonTrial {
        schema: TRIAL_SCHEMA.to_owned(),
        eval_version: set.meta.eval_version,
        goal: goal.id.clone(),
        lesson: goal.lesson.clone(),
        trial,
        runner: if opts.dry_run {
            format!("fake:{}", opts.fake.name())
        } else {
            "claude-code".to_owned()
        },
        model: summary.model.clone(),
        date: util::iso8601(util::now_secs()),
        claude_code_version: if opts.dry_run {
            Some("dry-run".to_owned())
        } else {
            summary
                .claude_code_version
                .clone()
                .or_else(|| env.claude_version.clone())
        },
        status: if timed_out {
            "timeout".to_owned()
        } else if summary.subtype.as_deref() == Some("error_max_turns") {
            "max_turns".to_owned()
        } else if exit != Some(0) || summary.is_error {
            "agent_error".to_owned()
        } else if summary.structured.is_none() {
            "no_answer".to_owned()
        } else {
            "ok".to_owned()
        },
        tokens: if opts.dry_run {
            TokensOut::default()
        } else {
            TokensOut {
                input: summary.tokens.input,
                output: summary.tokens.output,
                cache_read: summary.tokens.cache_read,
                cache_write: summary.tokens.cache_write,
                total: summary.tokens.total(),
            }
        },
        // A scripted agent spends no tokens and no money, whatever its synthetic transcript says.
        cost_usd: if opts.dry_run {
            Some(0.0)
        } else {
            summary.cost_usd
        },
        cost_is_estimate: !opts.dry_run,
        turns: summary.turns,
        wall_s: round4(wall_s),
        calls: calls.len() as u32,
        invalid_calls: calls.iter().filter(|c| c.invalid()).count() as u32,
        refused_calls: calls.iter().filter(|c| c.refused).count() as u32,
        units: wrap::spent(&cfg.state),
        pass: graded > 0 && passed == graded,
        graded,
        passed,
        passed_unassisted,
        steps,
        dir: dir.to_string_lossy().into_owned(),
        warnings,
    };
    util::write_json(
        &dir.join("result.json"),
        &serde_json::to_value(&result).map_err(|e| e.to_string())?,
    )?;
    if !opts.keep_tmp {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------------------------------------------

/// Strictly more than half of the trials: a goal passes when its median run passes.
fn majority(passes: usize, trials: usize) -> bool {
    passes * 2 > trials
}

fn stats(values: &[f64]) -> Value {
    json!({
        "median": median(values).map(round4),
        "max": values.iter().copied().max_by(f64::total_cmp).map(round4),
        "min": values.iter().copied().min_by(f64::total_cmp).map(round4),
    })
}

/// Graded and passed steps of each kind over `trials`.
fn by_kind(trials: &[&LessonTrial]) -> Value {
    let mut kinds: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
    for t in trials {
        for s in t.steps.iter().filter(|s| s.graded) {
            let e = kinds.entry(s.kind.as_str()).or_default();
            e.0 += 1;
            e.1 += u32::from(s.pass);
            e.2 += u32::from(s.pass && !s.assisted);
        }
    }
    let mut o = serde_json::Map::new();
    for (k, (graded, passed, unassisted)) in kinds {
        o.insert(
            k.to_owned(),
            json!({"graded": graded, "passed": passed, "passed_unassisted": unassisted}),
        );
    }
    Value::Object(o)
}

fn entry(goal: &LessonGoal, rs: &[&LessonTrial]) -> Value {
    let n = rs.len();
    let passes = rs.iter().filter(|r| r.pass).count();
    let f = |g: &dyn Fn(&LessonTrial) -> f64| -> Vec<f64> { rs.iter().map(|r| g(r)).collect() };
    let calls: u32 = rs.iter().map(|r| r.calls).sum();
    let invalid: u32 = rs.iter().map(|r| r.invalid_calls).sum();
    let tokens_median =
        |g: &dyn Fn(&LessonTrial) -> u64| median(&f(&|r| g(r) as f64)).map(|x| x.round() as u64);
    // The steps that failed in most trials, to say where a goal is lost.
    let mut failed: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rs {
        for s in r.steps.iter().filter(|s| s.graded && !s.pass) {
            *failed.entry(s.step.as_str()).or_default() += 1;
        }
    }
    json!({
        "goal": goal.id,
        "lesson": goal.lesson,
        "difficulty": goal.difficulty,
        "trials": n,
        "passes": passes,
        "solve_rate": round4(passes as f64 / n as f64),
        "pass_at_k": passes > 0,
        "pass_pow_k": passes == n,
        "majority_pass": majority(passes, n),
        "steps": {
            "graded": rs.first().map_or(0, |r| r.graded),
            "passed": stats(&f(&|r| f64::from(r.passed))),
            "passed_unassisted": stats(&f(&|r| f64::from(r.passed_unassisted))),
            "by_kind": by_kind(rs),
            "failed_in": failed,
        },
        "units": stats(&f(&|r| f64::from(r.units))),
        "tokens": {
            "median_total": tokens_median(&|r| r.tokens.total),
            "median_input": tokens_median(&|r| r.tokens.input),
            "median_output": tokens_median(&|r| r.tokens.output),
        },
        "cost_usd": {
            "median": median(&rs.iter().filter_map(|r| r.cost_usd).collect::<Vec<_>>()).map(round4),
            "total": round4(rs.iter().filter_map(|r| r.cost_usd).sum::<f64>()),
            "is_estimate": rs.iter().any(|r| r.cost_is_estimate),
        },
        "invalid_rate": if calls == 0 { Value::Null } else { json!(round4(f64::from(invalid) / f64::from(calls))) },
        "refused_calls": rs.iter().map(|r| r.refused_calls).sum::<u32>(),
        "wall_s": stats(&f(&|r| r.wall_s)),
        "statuses": rs.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(),
    })
}

/// Build the summary of a run from its trial results.
pub fn summarize(header: &Value, results: &[LessonTrial], set: &LessonSet) -> Value {
    let mut entries = Vec::new();
    for goal in &set.goals {
        let rs: Vec<&LessonTrial> = results.iter().filter(|r| r.goal == goal.id).collect();
        if !rs.is_empty() {
            entries.push(entry(goal, &rs));
        }
    }
    let count = |key: &str| entries.iter().filter(|e| e[key] == true).count();
    let all: Vec<&LessonTrial> = results.iter().collect();
    let cost: f64 = results.iter().filter_map(|r| r.cost_usd).sum();
    let graded: u32 = all.iter().map(|r| r.graded).sum();
    let passed: u32 = all.iter().map(|r| r.passed).sum();
    let unassisted: u32 = all.iter().map(|r| r.passed_unassisted).sum();
    let model = header
        .get("model_requested")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            results
                .first()
                .and_then(|r| r.runner.strip_prefix("fake:").map(str::to_owned))
        })
        .or_else(|| results.iter().find_map(|r| r.model.clone()))
        .unwrap_or_else(|| "unknown".to_owned());
    json!({
        "schema": SUMMARY_SCHEMA,
        "track": "lesson",
        "model": model,
        "header": header,
        "goals": entries,
        "totals": {
            "goals": entries.len(),
            "lessons_majority_pass": count("majority_pass"),
            "lessons_any_pass": count("pass_at_k"),
            "lessons_all_pass": count("pass_pow_k"),
            "trials": results.len(),
            "trials_passed": results.iter().filter(|r| r.pass).count(),
            "steps_graded": graded,
            "steps_passed": passed,
            "steps_passed_unassisted": unassisted,
            "by_kind": by_kind(&all),
            "units": stats(&all.iter().map(|r| f64::from(r.units)).collect::<Vec<_>>()),
            "cost_usd": {"total": round4(cost), "is_estimate": results.iter().any(|r| r.cost_is_estimate)},
        },
    })
}

/// Print the summary as a table.
pub fn print(summary: &Value) {
    println!(
        "\n{:<30} {:>1} {:>6} {:>8} {:>9} {:>6} {:>7}  failed in",
        "goal", "d", "pass", "steps", "unassist", "units", "cost"
    );
    for e in summary["goals"].as_array().into_iter().flatten() {
        let failed: Vec<String> = e["steps"]["failed_in"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| format!("{k}x{v}")).collect())
            .unwrap_or_default();
        println!(
            "{:<30} {:>1} {:>3}/{:<2} {:>3}/{:<4} {:>4}/{:<4} {:>6} {:>7.3}  {}",
            e["goal"].as_str().unwrap_or("?"),
            e["difficulty"],
            e["passes"],
            e["trials"],
            e["steps"]["passed"]["median"],
            e["steps"]["graded"],
            e["steps"]["passed_unassisted"]["median"],
            e["steps"]["graded"],
            e["units"]["median"],
            e["cost_usd"]["total"].as_f64().unwrap_or(0.0),
            failed.join(" "),
        );
    }
    let t = &summary["totals"];
    println!(
        "\nlessons passed (majority of trials): {} of {}; trials passed: {} of {}; steps passed: {} of {} ({} unassisted); cost ${:.3}{}",
        t["lessons_majority_pass"],
        t["goals"],
        t["trials_passed"],
        t["trials"],
        t["steps_passed"],
        t["steps_graded"],
        t["steps_passed_unassisted"],
        t["cost_usd"]["total"].as_f64().unwrap_or(0.0),
        if t["cost_usd"]["is_estimate"] == true {
            " (client-side estimate)"
        } else {
            ""
        },
    );
    if let Some(kinds) = t["by_kind"].as_object() {
        for (k, v) in kinds {
            println!(
                "  {k:<9} passed {} of {} graded ({} unassisted)",
                v["passed"], v["graded"], v["passed_unassisted"]
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The commands
// ---------------------------------------------------------------------------------------------------------------

const USAGE: &str = "usage: cargo xtask agent-eval lesson [run] [options]
  The agent lesson track: an agent completes whole lessons of the basics pack with `enth lesson` and the patch CLI.
  Options are the text goals' (`agent-eval --help`): --dry-run, --agent witness|follower, --goals id,..., --trials N,
  --model ID, --max-usd N, --jobs N, --timeout-min N, --auth ..., --enth PATH, --out DIR, --confirm-spend, --keep-tmp,
  --no-save-summary, and one more:
  --save-floor              with --dry-run --agent follower: save the summary as the track's floor
                            (content/agent-evals/results/<date>-lesson-follower.json; cost 0)
  agent-eval lesson goals [--enth PATH]
                            list the goals, and check each against its pack (the pinned steps)";

/// `agent-eval lesson ...`.
pub fn lesson(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("goals") => goals(&args[1..]),
        Some("run") => run(&super::parse(&args[1..])?),
        _ => run(&super::parse(args)?),
    }
}

fn load_track(root: &Path) -> Result<LessonSet, String> {
    load(&root.join("content/agent-evals"))
}

fn goals(args: &[String]) -> Result<(), String> {
    let mut opts = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--enth" => {
                opts.enth = Some(PathBuf::from(it.next().ok_or(USAGE)?));
            }
            _ => return Err(format!("unknown argument `{a}`\n{USAGE}")),
        }
    }
    opts.dry_run = true;
    let meta = crate::metadata()?;
    let root = meta.workspace_root.clone();
    let set = load_track(&root)?;
    let env = build_env(&root, &meta.target_directory, &opts, set.meta.eval_version)?;
    println!(
        "lesson track version {} ({} goals)",
        set.meta.eval_version,
        set.goals.len()
    );
    println!(
        "{:<30} {:>1}  {:<20} {:>5} {:>3} {:>3} {:>3}  units",
        "id", "d", "lesson", "steps", "do", "rnd", "chs"
    );
    let mut drifted = 0;
    for g in &set.goals {
        let count = |k: &str| {
            g.steps
                .iter()
                .filter(|s| s.ends_with(&format!(":{k}")))
                .count()
        };
        let state = match facts(&env, &root, g) {
            Ok(_) => String::new(),
            Err(e) => {
                drifted += 1;
                format!("  DRIFTED: {e}")
            }
        };
        println!(
            "{:<30} {:>1}  {:<20} {:>5} {:>3} {:>3} {:>3}  {}{state}",
            g.id,
            g.difficulty,
            g.lesson,
            g.steps.len(),
            count("do"),
            count("recreate"),
            count("choose"),
            g.budget.units,
        );
    }
    if drifted > 0 {
        return Err(format!("{drifted} goal(s) no longer match their pack"));
    }
    Ok(())
}

/// `agent-eval lesson run`.
pub fn run(opts: &Opts) -> Result<(), String> {
    if !opts.dry_run && !opts.confirm_spend {
        return Err("a real run calls a model and spends the owner's money (unmeasured: a lesson is about 50 tool \
calls; plan on well under $1 a trial on Sonnet 5.5, so 16 lessons x 3 trials is tens of dollars, and the first real run \
measures it). Start with --dry-run, which calls none. When the owner has approved the spend, add --confirm-spend"
            .to_owned());
    }
    let meta = crate::metadata()?;
    let root = meta.workspace_root.clone();
    let set = load_track(&root)?;
    let mut goals: Vec<&LessonGoal> = set.goals.iter().collect();
    if let Some(wanted) = &opts.goals {
        for id in wanted {
            if !set.goals.iter().any(|g| &g.id == id) {
                return Err(format!(
                    "no lesson goal `{id}`; goals: {}",
                    set.goals
                        .iter()
                        .map(|g| g.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        goals.retain(|g| wanted.contains(&g.id));
    }
    let env = build_env(&root, &meta.target_directory, opts, set.meta.eval_version)?;
    let mut all_facts: BTreeMap<String, LessonFacts> = BTreeMap::new();
    for g in &goals {
        all_facts.insert(g.id.clone(), facts(&env, &root, g)?);
    }

    let now = util::now_secs();
    let stamp = util::stamp(now);
    let run_dir = opts
        .out
        .clone()
        .unwrap_or_else(|| meta.target_directory.join("agent-eval-lesson").join(&stamp));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
    let header = json!({
        "schema": "enth.agent-eval-lesson.header/1",
        "track": "lesson",
        "eval_version": set.meta.eval_version,
        "prompt_fingerprint": set.prompt_fingerprint()?,
        "date": util::iso8601(now),
        "stamp": stamp,
        "dry_run": opts.dry_run,
        "runner": if opts.dry_run { format!("fake:{}", opts.fake.name()) } else { "claude-code".to_owned() },
        "model_requested": if opts.dry_run { Value::Null } else { json!(opts.model) },
        "claude_code_version": env.claude_version,
        "auth": if opts.dry_run { "none" } else if env.auth == super::run::Auth::ApiKey { "api-key (--bare)" } else { "subscription (--safe-mode)" },
        "enth_git": git_line(&root, &["rev-parse", "HEAD"]),
        "enth_git_dirty": git_line(&root, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty()),
        "enth_version": command_version(&env.enth.to_string_lossy()),
        "trials": opts.trials,
        "goals": goals.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        "max_usd": opts.max_usd,
        "seed": opts.seed,
    });
    util::write_json(&run_dir.join("header.json"), &header)?;

    let plan: Vec<(&LessonGoal, u32)> = goals
        .iter()
        .flat_map(|g| (1..=opts.trials).map(move |t| (*g, t)))
        .collect();
    println!(
        "agent-eval lesson: {} trials ({} goals x {}), {}, results in {}",
        plan.len(),
        goals.len(),
        opts.trials,
        if opts.dry_run {
            format!(
                "DRY RUN with the scripted `{}` agent, no model is called",
                opts.fake.name()
            )
        } else {
            format!("model {}", opts.model)
        },
        run_dir.display()
    );

    let cx = LessonCx {
        env: &env,
        set: &set,
        opts,
        run_dir: &run_dir,
        stamp: &stamp,
    };
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<LessonTrial>> = Mutex::new(Vec::new());
    let spent: Mutex<f64> = Mutex::new(0.0);
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let total = plan.len();
    std::thread::scope(|scope| {
        for _ in 0..opts.jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&(goal, trial)) = plan.get(i) else { break };
                    if !opts.dry_run && *spent.lock().expect("lock") >= opts.max_usd {
                        errors.lock().expect("lock").push(format!(
                            "skipped {} t{trial}: --max-usd {} reached",
                            goal.id, opts.max_usd
                        ));
                        continue;
                    }
                    match run_trial(&cx, goal, &all_facts[&goal.id], trial) {
                        Ok(r) => {
                            *spent.lock().expect("lock") += r.cost_usd.unwrap_or(0.0);
                            println!(
                                "[{:>2}/{total}] {:<30} t{trial}: {} steps={}/{} unassisted={} units={:<3} cost=${:.3} calls={} {:.1}s{}",
                                i + 1,
                                goal.id,
                                if r.pass { "PASS" } else { "fail" },
                                r.passed,
                                r.graded,
                                r.passed_unassisted,
                                r.units,
                                r.cost_usd.unwrap_or(0.0),
                                r.calls,
                                r.wall_s,
                                if r.status == "ok" { String::new() } else { format!("  [{}]", r.status) },
                            );
                            results.lock().expect("lock").push(r);
                        }
                        Err(e) => {
                            println!("[{:>2}/{total}] {} t{trial}: ERROR {e}", i + 1, goal.id);
                            errors.lock().expect("lock").push(format!("{} t{trial}: {e}", goal.id));
                        }
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().expect("lock");
    results.sort_by(|a, b| (&a.goal, a.trial).cmp(&(&b.goal, b.trial)));
    let errors = errors.into_inner().expect("lock");

    let summary = summarize(&header, &results, &set);
    util::write_json(&run_dir.join("summary.json"), &summary)?;
    print(&summary);
    for e in &errors {
        println!("agent-eval lesson: {e}");
    }
    let is_floor = opts.dry_run && opts.fake == super::fake::Kind::Follower && opts.save_floor;
    if (!opts.dry_run && opts.save_summary) || is_floor {
        let model = summary
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .replace(['/', ' '], "-");
        let path = set
            .dir
            .join("results")
            .join(format!("{}-lesson-{model}.json", util::date(now)));
        std::fs::create_dir_all(path.parent().expect("has a parent")).map_err(|e| e.to_string())?;
        util::write_json(&path, &summary)?;
        println!(
            "agent-eval lesson: summary saved to {} (commit it)",
            path.display()
        );
    }
    if results.is_empty() {
        return Err("no trial finished".to_owned());
    }
    if !errors.is_empty() {
        return Err(format!("{} trial(s) errored or were skipped", errors.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> LessonSet {
        load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../content/agent-evals"))
            .expect("the shipped lesson track loads")
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|&s| s.to_owned()).collect()
    }

    fn step(id: &str, number: usize, kind: &str) -> StepFacts {
        StepFacts {
            number,
            id: id.to_owned(),
            kind: kind.to_owned(),
            hints: 0,
            editable: Vec::new(),
            locked: Vec::new(),
            labels: Vec::new(),
            answer: None,
            witness: None,
            has_start: false,
        }
    }

    fn call(argv: &[&str]) -> Call {
        Call {
            n: 0,
            ts: String::new(),
            argv: strings(argv),
            sub: None,
            render: false,
            refused: false,
            renders_after: 0,
            cost: 0,
            exit: 0,
            wall_ms: 0,
            stdout_bytes: 0,
            error: None,
        }
    }

    #[test]
    fn the_shipped_track_has_a_goal_for_each_basics_lesson() {
        let set = set();
        assert_eq!(set.goals.len(), 16);
        let mut ids: Vec<&str> = set.goals.iter().map(|g| g.lesson.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 16, "one goal per lesson");
        for g in &set.goals {
            assert_eq!(g.id, format!("basics-{}", g.lesson));
            assert_eq!(g.pack, "content/lessons/basics");
            assert!(
                g.steps
                    .iter()
                    .any(|s| s.ends_with(":do") || s.ends_with(":recreate")),
                "{}: a lesson with something to make",
                g.id
            );
            // A witness's cost (a check per do step, a search and a compare per round) fits the budget.
            let count = |k: &str| {
                g.steps
                    .iter()
                    .filter(|s| s.ends_with(&format!(":{k}")))
                    .count()
            };
            let witness_units = count("do") + 11 * count("recreate");
            assert!(
                (witness_units as u32) * 2 <= g.budget.units,
                "{}: the budget is at least twice the witness's {witness_units} units",
                g.id
            );
        }
    }

    #[test]
    fn prompts_fill_completely_and_leak_no_answer_text() {
        let set = set();
        for g in &set.goals {
            for name in ["lesson-system", "lesson"] {
                let text = render_prompt(&set.prompt(name).unwrap(), g);
                for key in ["lesson", "pack", "units"] {
                    assert!(
                        !text.contains(&format!("{{{key}}}")),
                        "{} {name}: {{{key}}} left unfilled",
                        g.id
                    );
                }
            }
            let text = render_prompt(&set.prompt("lesson").unwrap(), g);
            assert!(text.contains(&g.lesson) && text.contains(&g.budget.units.to_string()));
            assert!(!text.contains(&g.rationale));
        }
        // The system prompt tells the agent the rules the wrapper enforces.
        let system = set.prompt("lesson-system").unwrap();
        for rule in [
            "--reveal",
            "units",
            "budget_exhausted",
            "locks",
            "assistance",
        ] {
            assert!(system.contains(rule), "the system prompt lacks `{rule}`");
        }
    }

    #[test]
    fn a_goal_file_with_a_stray_key_or_a_bad_step_is_refused() {
        let good = r#"
schema = "enth.agent-eval-lesson/1"
id = "basics-cutoff"
added = "2026-10-21"
difficulty = 1
tags = ["basics"]
pack = "content/lessons/basics"
lesson = "cutoff"
steps = ["try:do"]
rationale = "why"
budget = { units = 120, turns = 100, usd = 2.0 }
"#;
        let goal: LessonGoal = toml::from_str(good).expect("parses");
        assert!(validate(&goal, "basics-cutoff").is_ok());
        assert!(validate(&goal, "other").is_err(), "the id is the file name");
        assert!(toml::from_str::<LessonGoal>(&format!("{good}extra = 1")).is_err());
        let mut bad = goal.clone();
        bad.steps = strings(&["try"]);
        assert!(validate(&bad, "basics-cutoff").is_err());
        bad.steps = strings(&["try:play"]);
        assert!(validate(&bad, "basics-cutoff").is_err());
        let mut bad = goal.clone();
        bad.pack = "../elsewhere".to_owned();
        assert!(validate(&bad, "basics-cutoff").is_err());
        let mut bad = goal;
        bad.budget.units = 0;
        assert!(validate(&bad, "basics-cutoff").is_err());
    }

    #[test]
    fn selectors_cover_what_the_editor_would_lock() {
        assert!(covers("filter1.cutoff", "filter1.cutoff"));
        assert!(!covers("filter1.cutoff", "filter1.resonance"));
        assert!(covers("osc1.*", "osc1.wave"));
        assert!(covers("osc1.*", "osc1"));
        assert!(
            !covers("osc1.*", "osc10.wave"),
            "a module is not its neighbour"
        );
        assert!(covers("osc1.*", "osc1 -> filter1.cutoff"));
        assert!(covers("filter1.*", "lfo1 -> filter1.cutoff"));
        assert!(!covers("filter1.*", "lfo1 -> osc1.pw"));
        assert!(covers("lfo1 -> filter1.cutoff", "lfo1 -> filter1.cutoff"));
        let changes = strings(&["filter1.cutoff", "osc1.wave"]);
        // Locked: the oscillator.
        assert_eq!(
            violations(&changes, &[], &strings(&["osc1.*"])),
            strings(&["osc1.wave"])
        );
        // Editable: only the cutoff.
        assert_eq!(
            violations(&changes, &strings(&["filter1.cutoff"]), &[]),
            strings(&["osc1.wave"])
        );
        assert!(
            violations(&changes, &[], &[]).is_empty(),
            "nothing set: anything goes"
        );
    }

    #[test]
    fn assistance_is_what_the_log_shows_per_step() {
        let steps = vec![
            step("try", 1, "do"),
            step("round-1", 2, "recreate"),
            step("round-2", 3, "recreate"),
        ];
        let calls = vec![
            call(&["lesson", "show", "pack", "--lesson", "cutoff"]),
            call(&[
                "lesson", "check", "pack", "--lesson", "cutoff", "--step", "try", "t.json",
            ]),
            call(&[
                "lesson", "hint", "pack", "--lesson", "cutoff", "--step", "1", "--n", "3", "t.json",
            ]),
            call(&[
                "lesson",
                "start",
                "pack",
                "--lesson",
                "cutoff",
                "--step",
                "round-1",
                "--challenge-out",
                "r1.json",
                "--out",
                "a.json",
            ]),
            call(&[
                "--format",
                "json",
                "match",
                "a.json",
                "--challenge",
                "r1.json",
            ]),
            call(&[
                "lesson", "closest", "pack", "--lesson", "cutoff", "--step", "round-2", "b.json",
            ]),
            call(&["compare", "a.json", "--challenge", "r1.json"]),
        ];
        assert_eq!(
            assisted_steps(&calls, &steps),
            ["round-1", "round-2", "try"]
        );
        // A refused or rejected call helped with nothing.
        let mut refused = call(&[
            "lesson", "closest", "pack", "--lesson", "cutoff", "--step", "try", "t.json",
        ]);
        refused.refused = true;
        let mut bad = call(&[
            "lesson", "hint", "pack", "--lesson", "cutoff", "--step", "try", "--n", "9", "t.json",
        ]);
        bad.exit = 2;
        assert_eq!(
            assisted_steps(&[refused, bad], &steps),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_answer_names_a_step_by_id_or_number() {
        let answer = json!({"steps": [{"step": "try", "patch": "a.json"}, {"step": 2, "choice": "B"}, {"step": "3", "choice": "C"}]});
        let find = |id: &str, n: usize| answer_for(Some(&answer), &step(id, n, "do")).is_some();
        assert!(find("try", 1));
        assert!(find("x", 2), "a number works");
        assert!(find("y", 3), "so does a number as text");
        assert!(!find("z", 4));
        assert!(answer_for(None, &step("try", 1, "do")).is_none());
    }

    fn trial(goal: &str, pass: bool, steps: &[(&str, &str, bool, bool)]) -> LessonTrial {
        let steps: Vec<StepResult> = steps
            .iter()
            .map(|&(id, kind, ok, assisted)| StepResult {
                step: id.to_owned(),
                kind: kind.to_owned(),
                graded: true,
                pass: ok,
                status: if ok { "pass" } else { "fail" }.to_owned(),
                detail: String::new(),
                assisted,
            })
            .collect();
        LessonTrial {
            schema: TRIAL_SCHEMA.to_owned(),
            eval_version: 1,
            goal: goal.to_owned(),
            lesson: "cutoff".to_owned(),
            trial: 1,
            runner: "fake:follower".to_owned(),
            model: None,
            date: String::new(),
            claude_code_version: None,
            status: "ok".to_owned(),
            tokens: TokensOut::default(),
            cost_usd: Some(0.0),
            cost_is_estimate: false,
            turns: None,
            wall_s: 1.0,
            calls: 10,
            invalid_calls: 1,
            refused_calls: 0,
            units: 20,
            pass,
            graded: steps.len() as u32,
            passed: steps.iter().filter(|s| s.pass).count() as u32,
            passed_unassisted: steps.iter().filter(|s| s.pass && !s.assisted).count() as u32,
            steps,
            dir: String::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn the_summary_counts_steps_by_kind_and_names_a_scripted_model() {
        let set = set();
        let results = vec![
            trial(
                "basics-cutoff",
                false,
                &[
                    ("try", "do", true, true),
                    ("round-1", "recreate", false, true),
                    ("which", "choose", true, false),
                ],
            ),
            trial("basics-noise", true, &[("try", "do", true, false)]),
        ];
        let header = json!({"model_requested": null});
        let s = summarize(&header, &results, &set);
        assert_eq!(s["schema"], SUMMARY_SCHEMA);
        assert_eq!(s["model"], "follower");
        assert_eq!(s["totals"]["trials_passed"], 1);
        assert_eq!(s["totals"]["steps_graded"], 4);
        assert_eq!(s["totals"]["steps_passed"], 3);
        assert_eq!(s["totals"]["steps_passed_unassisted"], 2);
        assert_eq!(s["totals"]["by_kind"]["do"]["graded"], 2);
        assert_eq!(s["totals"]["by_kind"]["recreate"]["passed"], 0);
        assert_eq!(s["totals"]["cost_usd"]["total"], 0.0);
        let cutoff = &s["goals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["goal"] == "basics-cutoff")
            .unwrap();
        assert_eq!(cutoff["steps"]["failed_in"]["round-1"], 1);
        assert_eq!(cutoff["majority_pass"], false);
        // A real run names the model it asked for.
        let named = summarize(
            &json!({"model_requested": "claude-sonnet-5-5"}),
            &results,
            &set,
        );
        assert_eq!(named["model"], "claude-sonnet-5-5");
    }

    #[test]
    fn a_real_run_needs_the_spend_to_be_confirmed() {
        let err = run(&super::super::parse(&[]).unwrap())
            .expect_err("a real run without --confirm-spend is refused");
        assert!(err.contains("--confirm-spend"));
    }
}
