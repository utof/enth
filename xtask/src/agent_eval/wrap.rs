//! The `enth` wrapper: the only tool an agent under test has. It forwards a call to the real `enth`, logs it to
//! `enth-calls.jsonl`, and refuses a render once the goal's render budget is spent (exit 9, `budget_exhausted`).
//!
//! What counts as a render, in the agent's terms ("a `step` is one render, and `fix` renders are free"):
//! `render`, `step` and `bench`; `describe` and `check` of a patch (not of a WAV, which only re-reads a file);
//! `diff --describe`. A call that fails before rendering (usage, validation and file-not-found errors, exits 2, 3
//! and 5) gives its slot back, so a typo does not cost a render. The count lives in a state file behind a lock
//! file, so parallel tool calls cannot both take the last slot.
//!
//! **Lesson mode** (`--lesson`, lane EV3, the agent lesson track): the same wrapper with cost units instead of
//! renders (a check, describe, compare or measured hint is 1, `lesson closest` and `match` 10; see
//! [`classify_lesson`]) and one refusal more: `--reveal` (a lesson's answers, a challenge's target) is refused with
//! exit 9, as the budget is. The text goals' accounting above is untouched.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Exit code of a refused render.
pub const EXIT_BUDGET: i32 = 9;

/// Where the wrapper keeps its files and which `enth` it forwards to.
#[derive(Clone, Debug)]
pub struct WrapCfg {
    pub calls: PathBuf,
    pub state: PathBuf,
    pub enth: PathBuf,
    pub budget: u32,
    /// Lesson mode (lane EV3): cost units, and `--reveal` refused.
    pub lesson: bool,
}

/// What a call is, for the budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub sub: Option<String>,
    pub renders: bool,
}

/// One line of `enth-calls.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub n: u32,
    pub ts: String,
    pub argv: Vec<String>,
    pub sub: Option<String>,
    pub render: bool,
    pub refused: bool,
    pub renders_after: u32,
    /// Lesson mode: the cost units this call took (0 for a free one); text mode leaves it 0.
    #[serde(default)]
    pub cost: u32,
    pub exit: i32,
    pub wall_ms: u64,
    pub stdout_bytes: usize,
    pub error: Option<String>,
}

impl Call {
    /// Exit 2 (usage) or 3 (validation): the "invalid command" of the metrics.
    pub fn invalid(&self) -> bool {
        !self.refused && (self.exit == 2 || self.exit == 3)
    }
}

/// Classify `argv` (the arguments after `enth`). `is_wav` says whether a path is a WAV file.
pub fn classify(argv: &[String], is_wav: &dyn Fn(&str) -> bool) -> Class {
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--format" {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let Some(sub) = argv.get(i).cloned() else {
        return Class {
            sub: None,
            renders: false,
        };
    };
    let rest = &argv[i + 1..];
    let help = rest.iter().any(|a| a == "--help" || a == "-h");
    let renders = !help
        && match sub.as_str() {
            "render" | "step" | "bench" => true,
            "describe" | "check" => !rest.iter().any(|a| !a.starts_with('-') && is_wav(a)),
            "diff" => rest.iter().any(|a| a == "--describe"),
            _ => false,
        };
    Class {
        sub: Some(sub),
        renders,
    }
}

struct Lock(PathBuf);

impl Lock {
    fn take(state: &Path) -> Result<Lock, String> {
        let path = state.with_extension("lock");
        let start = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Lock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A crashed wrapper must not wedge the run.
                    if start.elapsed() > Duration::from_secs(10) {
                        let _ = std::fs::remove_file(&path);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn read_count(state: &Path) -> u32 {
    std::fs::read_to_string(state)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// Take a render slot: the new count, or `None` when the budget is spent.
pub fn reserve(state: &Path, budget: u32) -> Result<Option<u32>, String> {
    let _lock = Lock::take(state)?;
    let n = read_count(state);
    if n >= budget {
        return Ok(None);
    }
    std::fs::write(state, (n + 1).to_string()).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Some(n + 1))
}

/// Give a slot back; returns the new count.
pub fn refund(state: &Path) -> Result<u32, String> {
    let _lock = Lock::take(state)?;
    let n = read_count(state).saturating_sub(1);
    std::fs::write(state, n.to_string()).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(n)
}

/// How many renders have been spent.
pub fn spent(state: &Path) -> u32 {
    read_count(state)
}

fn log_call(cfg: &WrapCfg, call: &mut Call) -> Result<(), String> {
    let _lock = Lock::take(&cfg.state)?;
    call.n = std::fs::read_to_string(&cfg.calls).map_or(0, |t| t.lines().count() as u32) + 1;
    let mut line = serde_json::to_string(call).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cfg.calls)
        .map_err(|e| format!("{}: {e}", cfg.calls.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|e| format!("{}: {e}", cfg.calls.display()))
}

/// The refusal an agent sees (a JSON error object, like the CLI's own).
pub fn refusal_json(budget: u32) -> String {
    serde_json::json!({
        "schema": "enth.error/1",
        "error": "budget_exhausted",
        "exit": EXIT_BUDGET,
        "message": format!("the render budget of {budget} is spent: this render was refused"),
        "hint": "stop designing and give your best patch as the final answer; get, diff, params, apply, guide and a check of a WAV still work and cost no render",
    })
    .to_string()
}

/// Forward one call. Writes the child's output to `out` and `err`; returns the exit code to use.
pub fn call(
    cfg: &WrapCfg,
    argv: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, String> {
    if cfg.lesson {
        return call_lesson(cfg, argv, out, err);
    }
    let class = classify(argv, &|p| super::util::is_riff(Path::new(p)));
    let ts = super::util::iso8601(super::util::now_secs());
    let mut call = Call {
        n: 0,
        ts,
        argv: argv.to_vec(),
        sub: class.sub.clone(),
        render: class.renders,
        refused: false,
        renders_after: spent(&cfg.state),
        cost: 0,
        exit: 0,
        wall_ms: 0,
        stdout_bytes: 0,
        error: None,
    };
    if class.renders && reserve(&cfg.state, cfg.budget)?.is_none() {
        let text = refusal_json(cfg.budget);
        writeln!(out, "{text}").map_err(|e| e.to_string())?;
        call.refused = true;
        call.exit = EXIT_BUDGET;
        call.stdout_bytes = text.len() + 1;
        call.error = Some("budget_exhausted".to_owned());
        log_call(cfg, &mut call)?;
        return Ok(EXIT_BUDGET);
    }
    let start = Instant::now();
    let output = Command::new(&cfg.enth)
        .args(argv)
        .stdin(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", cfg.enth.display()))?;
    call.wall_ms = start.elapsed().as_millis() as u64;
    let code = output.status.code().unwrap_or(1);
    call.exit = code;
    call.stdout_bytes = output.stdout.len();
    call.error = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned));
    out.write_all(&output.stdout).map_err(|e| e.to_string())?;
    err.write_all(&output.stderr).map_err(|e| e.to_string())?;
    call.renders_after = if class.renders && matches!(code, 2 | 3 | 5) {
        refund(&cfg.state)?
    } else {
        spent(&cfg.state)
    };
    log_call(cfg, &mut call)?;
    Ok(code)
}

// ---------------------------------------------------------------------------------------------------------------
// Lesson mode (lane EV3)
// ---------------------------------------------------------------------------------------------------------------

/// The flags of `enth lesson` that take a value, so a positional count skips their values.
const LESSON_VALUE_FLAGS: &[&str] = &[
    "--lesson",
    "--step",
    "--n",
    "--workers",
    "--budget",
    "--free",
    "--out",
    "--challenge-out",
    "--from",
    "--format",
];

/// What a call costs in lesson mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LessonClass {
    /// The command (`lesson`, `compare`, ...).
    pub sub: Option<String>,
    /// Cost units: 0 for a read, 1 for one render or a few, 10 for a search.
    pub cost: u32,
    /// Why the call is refused outright, if it is.
    pub refuse: Option<Refusal>,
}

/// The calls lesson mode refuses without running them, at no cost (exit 9, like a spent budget).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `--reveal`: a lesson's answers or a challenge's target.
    Reveal,
    /// A path outside the working directory (absolute, or with `..`).
    Path,
    /// A file of the pack named to a command other than `enth lesson`.
    Pack,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Reveal => "reveal_refused",
            Refusal::Path => "path_refused",
            Refusal::Pack => "pack_refused",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Refusal::Reveal => {
                "--reveal gives a lesson's or a challenge's answers away, and this exercise is to find them"
            }
            Refusal::Path => {
                "this exercise's files are all in the working directory: a path is relative and has no `..`"
            }
            Refusal::Pack => {
                "the pack belongs to `enth lesson`: its files are not for the other commands"
            }
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Refusal::Reveal => {
                "leave --reveal out; `enth lesson hint` and `enth lesson closest` are the lesson's own help (they count as assistance)"
            }
            Refusal::Path => "use the files `enth lesson start` writes, by their relative paths",
            Refusal::Pack => {
                "`enth lesson start` writes a step's start patch and round into the working directory; work on those"
            }
        }
    }
}

/// The directory the lesson track puts a copy of the pack in.
pub const PACK_DIR: &str = "pack";

/// Whether `arg` names a file outside the working directory, or a file of the pack (`Some(Refusal::Pack)` only when
/// `outside_lesson`, for a command that is not `lesson`).
fn path_refusal(arg: &str, outside_lesson: bool) -> Option<Refusal> {
    if arg.starts_with('{') || arg.starts_with('[') || arg.contains(' ') {
        return None; // inline JSON, an expectation list, a sentence
    }
    if arg.starts_with('/') || arg.split('/').any(|c| c == "..") {
        return Some(Refusal::Path);
    }
    let first = arg.trim_start_matches("./").split('/').next();
    (outside_lesson && first == Some(PACK_DIR)).then_some(Refusal::Pack)
}

/// The positional arguments of `args` (flags and the values of the lesson flags skipped).
fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if LESSON_VALUE_FLAGS.contains(&a) {
            i += 2;
        } else {
            if !a.starts_with('-') {
                out.push(a);
            }
            i += 1;
        }
    }
    out
}

/// Classify `argv` for lesson mode. Costs: `render`, `step`, `bench`, `describe` and `check` of a patch (not of a WAV),
/// `diff --describe`, `compare`, `lesson check` and a measured `lesson hint` (one with a patch) are 1; `suggest` 2;
/// `lesson closest` and `match` 10 (a search of tens of renders); everything else is free.
pub fn classify_lesson(argv: &[String], is_wav: &dyn Fn(&str) -> bool) -> LessonClass {
    let reveal = argv.iter().any(|a| a == "--reveal");
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--format" {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let Some(sub) = argv.get(i).cloned() else {
        return LessonClass {
            sub: None,
            cost: 0,
            refuse: reveal.then_some(Refusal::Reveal),
        };
    };
    let rest = &argv[i + 1..];
    let help = rest.iter().any(|a| a == "--help" || a == "-h");
    let pos = positionals(rest);
    let refuse = if reveal {
        Some(Refusal::Reveal)
    } else {
        // A path refusal outranks a pack refusal only when it is a `..` or absolute path; check each argument.
        argv.iter()
            .filter_map(|a| path_refusal(a, sub != "lesson"))
            .min_by_key(|r| *r != Refusal::Path)
    };
    let cost = if help {
        0
    } else {
        match sub.as_str() {
            "render" | "step" | "bench" | "compare" => 1,
            "describe" | "check" => u32::from(!pos.iter().any(|a| is_wav(a))),
            "diff" => u32::from(rest.iter().any(|a| a == "--describe")),
            "suggest" => 2,
            "match" => 10,
            "lesson" => match pos.first().copied() {
                Some("check") => 1,
                // `hint <pack> [patch]`: a patch makes it a measured hint.
                Some("hint") => u32::from(pos.len() >= 3),
                Some("closest") => 10,
                _ => 0,
            },
            _ => 0,
        }
    };
    LessonClass {
        sub: Some(sub),
        cost: if refuse.is_some() { 0 } else { cost },
        refuse,
    }
}

/// Take `cost` units: the new count, or `None` when they do not fit in the budget.
pub fn reserve_cost(state: &Path, budget: u32, cost: u32) -> Result<Option<u32>, String> {
    let _lock = Lock::take(state)?;
    let n = read_count(state);
    if n.saturating_add(cost) > budget {
        return Ok(None);
    }
    std::fs::write(state, (n + cost).to_string())
        .map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(Some(n + cost))
}

/// Give `cost` units back; returns the new count.
pub fn refund_cost(state: &Path, cost: u32) -> Result<u32, String> {
    let _lock = Lock::take(state)?;
    let n = read_count(state).saturating_sub(cost);
    std::fs::write(state, n.to_string()).map_err(|e| format!("{}: {e}", state.display()))?;
    Ok(n)
}

/// The refusal an agent sees for a call lesson mode will not run.
pub fn lesson_refusal_json(why: Refusal) -> String {
    serde_json::json!({
        "schema": "enth.error/1",
        "error": why.code(),
        "exit": EXIT_BUDGET,
        "message": why.message(),
        "hint": why.hint(),
    })
    .to_string()
}

/// The refusal for a call that does not fit in the lesson budget.
pub fn lesson_budget_json(budget: u32) -> String {
    serde_json::json!({
        "schema": "enth.error/1",
        "error": "budget_exhausted",
        "exit": EXIT_BUDGET,
        "message": format!("the budget of {budget} units is spent: this call was refused"),
        "hint": "stop and give your best answer; show, list, start, an authored hint, get, diff, params, apply and guide still work and cost nothing",
    })
    .to_string()
}

/// [`call`] in lesson mode.
fn call_lesson(
    cfg: &WrapCfg,
    argv: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, String> {
    let class = classify_lesson(argv, &|p| super::util::is_riff(Path::new(p)));
    let mut call = Call {
        n: 0,
        ts: super::util::iso8601(super::util::now_secs()),
        argv: argv.to_vec(),
        sub: class.sub.clone(),
        render: class.cost > 0,
        refused: false,
        renders_after: spent(&cfg.state),
        cost: class.cost,
        exit: 0,
        wall_ms: 0,
        stdout_bytes: 0,
        error: None,
    };
    let refusal = if let Some(why) = class.refuse {
        Some((lesson_refusal_json(why), why.code()))
    } else if class.cost > 0 && reserve_cost(&cfg.state, cfg.budget, class.cost)?.is_none() {
        Some((lesson_budget_json(cfg.budget), "budget_exhausted"))
    } else {
        None
    };
    if let Some((text, why)) = refusal {
        writeln!(out, "{text}").map_err(|e| e.to_string())?;
        call.refused = true;
        call.cost = 0;
        call.exit = EXIT_BUDGET;
        call.stdout_bytes = text.len() + 1;
        call.error = Some(why.to_owned());
        log_call(cfg, &mut call)?;
        return Ok(EXIT_BUDGET);
    }
    let start = Instant::now();
    let output = Command::new(&cfg.enth)
        .args(argv)
        .stdin(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", cfg.enth.display()))?;
    call.wall_ms = start.elapsed().as_millis() as u64;
    let code = output.status.code().unwrap_or(1);
    call.exit = code;
    call.stdout_bytes = output.stdout.len();
    call.error = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned));
    out.write_all(&output.stdout).map_err(|e| e.to_string())?;
    err.write_all(&output.stderr).map_err(|e| e.to_string())?;
    // A call the tool rejects before it spends anything gives its units back (usage, validation, file not found).
    if class.cost > 0 && matches!(code, 2 | 3 | 5) {
        call.cost = 0;
        call.renders_after = refund_cost(&cfg.state, class.cost)?;
    } else {
        call.renders_after = spent(&cfg.state);
    }
    log_call(cfg, &mut call)?;
    Ok(code)
}

/// Read `enth-calls.jsonl` (missing file: no calls).
pub fn read_calls(path: &Path) -> Vec<Call> {
    std::fs::read_to_string(path)
        .map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str::<Call>(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The command-line entry: `wrap --calls P --state P --enth P --budget N -- <enth args>`.
pub fn main(args: &[String]) -> Result<i32, String> {
    let (mut calls, mut state, mut enth, mut budget) = (None, None, None, None);
    let mut lesson = false;
    let mut it = args.iter();
    let mut argv: Vec<String> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--calls" => calls = it.next().map(PathBuf::from),
            "--state" => state = it.next().map(PathBuf::from),
            "--enth" => enth = it.next().map(PathBuf::from),
            "--budget" => budget = it.next().and_then(|b| b.parse::<u32>().ok()),
            "--lesson" => lesson = true,
            "--" => {
                argv = it.cloned().collect();
                break;
            }
            other => return Err(format!("wrap: unknown option {other}")),
        }
    }
    let cfg = WrapCfg {
        calls: calls.ok_or("wrap: --calls is required")?,
        state: state.ok_or("wrap: --state is required")?,
        enth: enth.ok_or("wrap: --enth is required")?,
        budget: budget.ok_or("wrap: --budget is required")?,
        lesson,
    };
    call(&cfg, &argv, &mut std::io::stdout(), &mut std::io::stderr())
}

/// The shell script written as `enth` into the trial's temp dir.
#[cfg_attr(
    not(unix),
    allow(
        dead_code,
        reason = "the budget wrapper is a unix shell script; agent-eval runs on Linux"
    )
)]
pub fn script(xtask_exe: &Path, cfg: &WrapCfg) -> String {
    let q = |p: &Path| super::util::sh_quote(&p.to_string_lossy());
    format!(
        "#!/bin/sh\nexec {} agent-eval wrap --calls {} --state {} --enth {} --budget {}{} -- \"$@\"\n",
        q(xtask_exe),
        q(&cfg.calls),
        q(&cfg.state),
        q(&cfg.enth),
        cfg.budget,
        if cfg.lesson { " --lesson" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|&x| x.to_owned()).collect()
    }

    fn no_wav(_: &str) -> bool {
        false
    }

    #[test]
    fn classification_counts_the_calls_that_render() {
        let c = |a: &[&str]| classify(&argv(a), &no_wav);
        assert!(c(&["step", "p.json", "--set", "a=1"]).renders);
        assert!(c(&["render", "p.json"]).renders);
        assert!(c(&["--format", "json", "check", "p.json", "--expect", "x"]).renders);
        assert!(c(&["describe", "p.json"]).renders);
        assert!(c(&["diff", "a.json", "b.json", "--describe"]).renders);
        assert!(!c(&["diff", "a.json", "b.json"]).renders);
        assert!(!c(&["guide"]).renders);
        assert!(!c(&["params", "osc"]).renders);
        assert!(!c(&["apply", "p.json", "--set", "a=1"]).renders);
        assert!(!c(&["step", "--help"]).renders);
        assert!(!c(&[]).renders);
        assert_eq!(
            c(&["--format", "table", "get", "p.json"]).sub.as_deref(),
            Some("get")
        );
    }

    #[test]
    fn checking_a_wav_is_not_a_render() {
        let is_wav = |p: &str| Path::new(p).extension().is_some_and(|x| x == "wav");
        assert!(!classify(&argv(&["check", "x.wav", "--expect", "a"]), &is_wav).renders);
        assert!(!classify(&argv(&["describe", "x.wav"]), &is_wav).renders);
        assert!(classify(&argv(&["describe", "x.json"]), &is_wav).renders);
    }

    #[test]
    fn reserve_stops_at_the_budget_and_refund_gives_a_slot_back() {
        let dir = tempdir("reserve");
        let state = dir.join("budget.state");
        assert_eq!(reserve(&state, 2).unwrap(), Some(1));
        assert_eq!(reserve(&state, 2).unwrap(), Some(2));
        assert_eq!(reserve(&state, 2).unwrap(), None);
        assert_eq!(refund(&state).unwrap(), 1);
        assert_eq!(reserve(&state, 2).unwrap(), Some(2));
        assert_eq!(spent(&state), 2);
    }

    #[test]
    fn script_quotes_its_paths() {
        let cfg = WrapCfg {
            calls: PathBuf::from("/a b/calls.jsonl"),
            state: PathBuf::from("/a b/state"),
            enth: PathBuf::from("/x/enth"),
            budget: 20,
            lesson: false,
        };
        let s = script(Path::new("/t/xtask"), &cfg);
        assert!(
            s.starts_with("#!/bin/sh\nexec '/t/xtask' agent-eval wrap --calls '/a b/calls.jsonl'")
        );
        assert!(s.contains("--budget 20 -- \"$@\""));
        // Lesson mode is one more wrapper flag, before the `--` after which the agent's own `--lesson ID` goes.
        let lesson = WrapCfg {
            lesson: true,
            ..cfg
        };
        assert!(script(Path::new("/t/xtask"), &lesson).contains("--budget 20 --lesson -- \"$@\""));
    }

    #[test]
    fn lesson_mode_costs_a_check_one_and_a_search_ten_and_refuses_reveal() {
        let c = |a: &[&str]| classify_lesson(&argv(a), &no_wav);
        let cost = |a: &[&str]| c(a).cost;
        assert_eq!(cost(&["lesson", "show", "p", "--lesson", "x"]), 0);
        assert_eq!(cost(&["lesson", "list", "p"]), 0);
        assert_eq!(
            cost(&[
                "lesson", "start", "p", "--lesson", "x", "--step", "s", "--out", "a"
            ]),
            0
        );
        assert_eq!(
            cost(&[
                "lesson", "check", "p", "--lesson", "x", "--step", "s", "a.json"
            ]),
            1
        );
        // An authored hint (no patch) is free; a measured one (a patch) costs a render.
        assert_eq!(
            cost(&[
                "lesson", "hint", "p", "--lesson", "x", "--step", "s", "--n", "1"
            ]),
            0
        );
        assert_eq!(
            cost(&[
                "lesson", "hint", "p", "--lesson", "x", "--step", "s", "--n", "3", "a.json"
            ]),
            1
        );
        assert_eq!(
            cost(&[
                "lesson", "closest", "p", "--lesson", "x", "--step", "s", "a.json"
            ]),
            10
        );
        assert_eq!(cost(&["compare", "a.json", "--challenge", "c.json"]), 1);
        assert_eq!(cost(&["suggest", "a.json", "--challenge", "c.json"]), 2);
        assert_eq!(cost(&["match", "--target", "t.json"]), 10);
        assert_eq!(cost(&["--format", "json", "describe", "a.json"]), 1);
        assert_eq!(cost(&["apply", "a.json", "--set", "x=1"]), 0);
        assert_eq!(cost(&["lesson", "closest", "--help"]), 0);
        let refuse = |a: &[&str]| c(a).refuse;
        assert_eq!(
            refuse(&["lesson", "show", "p", "--reveal"]),
            Some(Refusal::Reveal)
        );
        assert_eq!(refuse(&["compare", "a", "--reveal"]), Some(Refusal::Reveal));
        assert_eq!(refuse(&["lesson", "show", "pack"]), None);
        assert_eq!(
            c(&["compare", "a", "--reveal"]).cost,
            0,
            "a refused call costs nothing"
        );
        // The pack is for `enth lesson`; no path leaves the working directory.
        assert_eq!(
            refuse(&["get", "pack/patches/pad.patch.json"]),
            Some(Refusal::Pack)
        );
        assert_eq!(
            refuse(&["describe", "./pack/x.patch.json"]),
            Some(Refusal::Pack)
        );
        assert_eq!(refuse(&["get", "../x.patch.json"]), Some(Refusal::Path));
        assert_eq!(refuse(&["get", "/etc/hostname"]), Some(Refusal::Path));
        assert_eq!(
            refuse(&["lesson", "check", "pack", "--out", "../x"]),
            Some(Refusal::Path)
        );
        assert_eq!(refuse(&["lesson", "start", "pack/", "--lesson", "x"]), None);
        assert_eq!(
            refuse(&[
                "check",
                "a.patch.json",
                "--expect",
                "brightness < 1 kHz / 2"
            ]),
            None
        );
        assert_eq!(
            refuse(&[
                "apply",
                "a.patch.json",
                "--ops",
                "[{\"set\":\"x\",\"to\":\"1/2\"}]"
            ]),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_lesson_wrapper_charges_units_refuses_reveal_and_refunds_a_rejected_call() {
        let dir = tempdir("lesson-mode");
        let cfg = WrapCfg {
            calls: dir.join("enth-calls.jsonl"),
            state: dir.join("budget.state"),
            enth: stub(&dir),
            budget: 12,
            lesson: true,
        };
        let run = |a: &[&str]| {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = call(&cfg, &argv(a), &mut out, &mut err).unwrap();
            (code, String::from_utf8(out).unwrap())
        };
        assert_eq!(run(&["lesson", "closest", "p"]).0, 0);
        assert_eq!(spent(&cfg.state), 10);
        // A compare the tool rejects before rendering gives its unit back.
        assert_eq!(run(&["compare", "badunit"]).0, 3);
        assert_eq!(spent(&cfg.state), 10);
        // Reveal is refused and costs nothing; a second closest does not fit.
        let (code, text) = run(&["lesson", "show", "p", "--reveal"]);
        assert_eq!(code, EXIT_BUDGET);
        assert!(text.contains("reveal_refused"));
        let (code, text) = run(&["get", "pack/patches/x.patch.json"]);
        assert_eq!(code, EXIT_BUDGET);
        assert!(text.contains("pack_refused"));
        let (code, text) = run(&["lesson", "closest", "p"]);
        assert_eq!((code, spent(&cfg.state)), (EXIT_BUDGET, 10));
        assert!(text.contains("budget_exhausted"));
        assert_eq!(run(&["lesson", "show", "p"]).0, 0, "reads stay free");
        let calls = read_calls(&cfg.calls);
        assert_eq!(calls.len(), 6);
        assert_eq!(calls.iter().filter(|c| c.refused).count(), 3);
        assert_eq!(calls.iter().map(|c| c.cost).sum::<u32>(), 10);
        assert_eq!(calls[0].cost, 10);
    }

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "enth-xtask-wrap-{tag}-{}-{}",
            std::process::id(),
            super::super::util::now_secs()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stub `enth`: prints a JSON object; `badunit` exits 3, `missing` exits 5, `failcheck` exits 1.
    #[cfg(unix)]
    fn stub(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("stub-enth");
        let body = "#!/bin/sh\nfor a in \"$@\"; do\n  case \"$a\" in\n    badunit) echo '{\"error\":\"wrong_unit\",\"exit\":3}'; exit 3;;\n    missing) echo '{\"error\":\"file_not_found\",\"exit\":5}'; exit 5;;\n    failcheck) echo '{\"pass\":false}'; exit 1;;\n  esac\ndone\necho '{\"ok\":true}'\n";
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_refuses_the_render_after_the_budget_and_logs_every_call() {
        let dir = tempdir("budget");
        let cfg = WrapCfg {
            calls: dir.join("enth-calls.jsonl"),
            state: dir.join("budget.state"),
            enth: stub(&dir),
            budget: 3,
            lesson: false,
        };
        let run = |a: &[&str]| {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = call(&cfg, &argv(a), &mut out, &mut err).unwrap();
            (code, String::from_utf8(out).unwrap())
        };
        assert_eq!(run(&["step", "p.json"]).0, 0);
        assert_eq!(
            run(&["step", "p.json", "badunit"]).0,
            3,
            "a validation error is forwarded"
        );
        assert_eq!(spent(&cfg.state), 1, "and it gave its slot back");
        assert_eq!(
            run(&["check", "p.json", "failcheck"]).0,
            1,
            "a failed check is still a render"
        );
        assert_eq!(run(&["step", "p.json", "missing"]).0, 5);
        assert_eq!(run(&["describe", "p.json"]).0, 0);
        assert_eq!(spent(&cfg.state), 3);
        let (code, text) = run(&["step", "p.json"]);
        assert_eq!(code, EXIT_BUDGET);
        assert!(text.contains("budget_exhausted"));
        assert_eq!(
            run(&["get", "p.json"]).0,
            0,
            "reads stay free after the budget"
        );
        let calls = read_calls(&cfg.calls);
        assert_eq!(calls.len(), 7);
        assert_eq!(calls.iter().filter(|c| c.refused).count(), 1);
        assert_eq!(calls.iter().filter(|c| c.invalid()).count(), 1);
        assert_eq!(calls[1].error.as_deref(), Some("wrong_unit"));
        assert_eq!(calls.last().unwrap().renders_after, 3);
        assert_eq!(
            calls.iter().map(|c| c.n).collect::<Vec<_>>(),
            (1..=7).collect::<Vec<_>>()
        );
    }
}
