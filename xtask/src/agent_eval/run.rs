//! Running trials: one empty temp dir per trial holding only the budget-counting `enth` wrapper, the agent (headless
//! Claude Code, or a scripted fake with `--dry-run`), then the judge, which forces the goal's performance and its
//! expectation list on the final patch. Results land under `target/agent-eval/<stamp>/<goal>/<track>-t<N>/`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::goal::{EvalSet, Goal, fill};
use super::stream::{self, StreamSummary};
use super::util;
use super::wrap::{self, WrapCfg};
use super::{blind, fake, report};

/// The trial result schema.
pub const TRIAL_SCHEMA: &str = "enth.agent-eval.trial/1";

/// Which expectation list the agent works from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Track {
    /// The designer's list is given.
    Given,
    /// The agent writes its own list from the brief; the designer's list still judges.
    SelfWritten,
}

impl Track {
    pub fn name(self) -> &'static str {
        match self {
            Track::Given => "given",
            Track::SelfWritten => "self",
        }
    }
}

/// How Claude Code authenticates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    Auto,
    ApiKey,
    Subscription,
}

/// Options of `agent-eval run`.
#[derive(Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per command-line flag"
)]
pub struct Opts {
    pub dry_run: bool,
    pub fake: fake::Kind,
    pub tracks: Vec<Track>,
    pub goals: Option<Vec<String>>,
    pub trials: u32,
    pub model: String,
    pub max_usd: f64,
    pub jobs: usize,
    pub enth: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub auth: Auth,
    pub timeout_min: f64,
    pub confirm_spend: bool,
    pub keep_tmp: bool,
    pub seed: u64,
    pub blind: bool,
    pub save_summary: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            dry_run: false,
            fake: fake::Kind::Witness,
            tracks: vec![Track::Given, Track::SelfWritten],
            goals: None,
            trials: 3,
            model: "claude-sonnet-5-5".to_owned(),
            max_usd: 20.0,
            jobs: 1,
            enth: None,
            out: None,
            auth: Auth::Auto,
            timeout_min: 15.0,
            confirm_spend: false,
            keep_tmp: false,
            seed: 0x0E17_5EED,
            blind: true,
            save_summary: true,
        }
    }
}

/// Everything a trial needs from the machine.
pub struct Env {
    pub enth: PathBuf,
    pub xtask: PathBuf,
    pub claude: String,
    pub claude_version: Option<String>,
    pub auth: Auth,
    pub eval_version: u32,
}

/// One scored expectation item of the judge.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(
    clippy::struct_field_names,
    reason = "the field names are the keys of `enth check`'s JSON"
)]
pub struct Margin {
    pub expect: String,
    pub pass: bool,
    pub got: Value,
    pub margin: Option<f64>,
    pub margin_pct: Option<f64>,
    pub unit: Option<String>,
    pub thin: bool,
}

/// Token counts as written to `result.json`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TokensOut {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

/// What one trial produced: the record of a single run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialResult {
    pub schema: String,
    pub eval_version: u32,
    pub goal: String,
    pub track: Track,
    pub trial: u32,
    pub runner: String,
    pub model: Option<String>,
    pub date: String,
    pub claude_code_version: Option<String>,
    /// `ok`, `agent_error`, `max_turns`, `timeout` or `no_patch`.
    pub status: String,
    pub tokens: TokensOut,
    pub cost_usd: Option<f64>,
    pub cost_is_estimate: bool,
    pub turns: Option<u64>,
    pub wall_s: f64,
    pub calls: u32,
    pub invalid_calls: u32,
    pub refused_calls: u32,
    /// Renders spent (the wrapper's count, at most the budget).
    pub renders: u32,
    /// The judge passed the final patch against the designer's list with the goal's performance.
    pub pass: bool,
    /// Items that pass by less than twice their measurement noise.
    pub thin: u32,
    pub margins: Vec<Margin>,
    pub failed: Vec<String>,
    /// The list the agent wrote (self track).
    pub own_expect: Option<String>,
    pub own_pass: Option<bool>,
    pub own_error: Option<String>,
    pub final_patch: Option<String>,
    pub final_wav: Option<String>,
    /// How the final patch was found: `answer`, `calls` or `none`.
    pub patch_source: String,
    /// An `enth` call wrote the final patch (it was not hand-written JSON).
    pub produced_by_enth: bool,
    pub dir: String,
    pub warnings: Vec<String>,
}

/// Replace the placeholders of a prompt template.
pub fn render_prompt(template: &str, goal: &Goal) -> String {
    let perf_json = goal.perf.json();
    let perf_short = goal.perf.short().unwrap_or_else(|| perf_json.clone());
    fill(
        template,
        &[
            ("brief", &goal.brief),
            ("perf_json", &perf_json),
            ("perf_short", &perf_short),
            ("expect", &goal.expect),
            ("template", &goal.template),
            ("renders", &goal.budget.renders.to_string()),
            ("patch", fake::PATCH),
        ],
    )
}

fn answer_schema() -> String {
    json!({
        "type": "object",
        "required": ["patch", "expect"],
        "properties": {
            "patch": {"type": "string", "description": "path of the final patch file you wrote with enth"},
            "expect": {"type": "string", "description": "the expectation list you checked the patch against"},
            "explanation": {"type": "string"},
        },
    })
    .to_string()
}

struct Judged {
    code: Option<i32>,
    json: Option<Value>,
    stderr: String,
}

/// Run `enth check` on the final patch with the goal's performance forced and `list` as the expectations.
fn judge(
    env: &Env,
    dir: &Path,
    patch: &Path,
    perf: &str,
    list: &str,
    out_name: &str,
) -> Result<Judged, String> {
    let out = Command::new(&env.enth)
        .current_dir(dir)
        .arg("check")
        .arg(patch)
        .args(["--perf", perf, "--expect", list, "--no-fix"])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", env.enth.display()))?;
    std::fs::write(dir.join(out_name), &out.stdout).map_err(|e| format!("{out_name}: {e}"))?;
    Ok(Judged {
        code: out.status.code(),
        json: serde_json::from_slice(&out.stdout).ok(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn margins_of(j: &Value) -> Vec<Margin> {
    j.get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|r| Margin {
                    expect: r
                        .get("expect")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    pass: r.get("pass").and_then(Value::as_bool).unwrap_or(false),
                    got: r.get("got").cloned().unwrap_or(Value::Null),
                    margin: r.get("margin").and_then(Value::as_f64),
                    margin_pct: r.get("margin_pct").and_then(Value::as_f64),
                    unit: r.get("unit").and_then(Value::as_str).map(str::to_owned),
                    thin: r.get("thin").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where the agent's final patch is: its answer first, else the last file an `enth` call wrote with `--out`.
fn find_patch(
    tmp: &Path,
    answer: Option<&Value>,
    calls: &[wrap::Call],
) -> (Option<PathBuf>, &'static str) {
    if let Some(p) = answer.and_then(|a| a.get("patch")).and_then(Value::as_str) {
        let path = if Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            tmp.join(p)
        };
        let inside = match (path.canonicalize(), tmp.canonicalize()) {
            (Ok(a), Ok(b)) => a.starts_with(b),
            _ => false,
        };
        if inside && path.is_file() {
            return (Some(path), "answer");
        }
    }
    for c in calls.iter().rev() {
        if let Some(i) = c.argv.iter().position(|a| a == "--out")
            && let Some(p) = c.argv.get(i + 1)
            && Path::new(p)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
            && c.exit == 0
            && tmp.join(p).is_file()
        {
            return (Some(tmp.join(p)), "calls");
        }
    }
    (None, "none")
}

fn spawn_claude(
    env: &Env,
    opts: &Opts,
    goal: &Goal,
    tmp: &Path,
    dir: &Path,
    prompt: &str,
) -> Result<(Option<i32>, bool, String), String> {
    let system = dir.join("system.md");
    let mut cmd = Command::new(&env.claude);
    cmd.current_dir(tmp)
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            &opts.model,
        ])
        .args(["--max-turns", &goal.budget.turns.to_string()])
        .args(["--max-budget-usd", &goal.budget.usd.to_string()])
        .args(["--tools", "Bash", "--allowedTools", "Bash(enth *)"])
        .args([
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
        ])
        .arg("--no-session-persistence")
        .arg("--append-system-prompt-file")
        .arg(&system)
        .args(["--json-schema", &answer_schema()]);
    match env.auth {
        Auth::ApiKey => {
            cmd.arg("--bare");
        }
        Auth::Subscription | Auth::Auto => {
            cmd.args([
                "--safe-mode",
                "--strict-mcp-config",
                "--disable-slash-commands",
            ]);
        }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    cmd.env("PATH", format!("{}:{path}", tmp.display()))
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(dir.join("claude.stderr")).map_err(|e| e.to_string())?);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot run `{}`: {e}", env.claude))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(prompt.as_bytes())
        .map_err(|e| format!("cannot write the prompt: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stream_path = dir.join("stream.jsonl");
    let (done_tx, done_rx) = std::sync::mpsc::channel::<String>();
    let reader_path = stream_path.clone();
    std::thread::spawn(move || {
        let mut text = String::new();
        let mut file = std::fs::File::create(&reader_path).ok();
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            text.push_str(&line);
            text.push('\n');
        }
        let _ = done_tx.send(text);
    });
    let deadline = Instant::now() + Duration::from_secs_f64(opts.timeout_min * 60.0);
    let (code, timed_out) = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break (status.code(), false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    // After a kill, a stray grandchild can keep the pipe open: do not wait for it past a few seconds, and read what
    // the reader already wrote to disk instead.
    let wait = if timed_out {
        Duration::from_secs(3)
    } else {
        Duration::from_secs(60)
    };
    let text = done_rx
        .recv_timeout(wait)
        .unwrap_or_else(|_| std::fs::read_to_string(&stream_path).unwrap_or_default());
    Ok((code, timed_out, text))
}

fn new_tmp(stamp: &str, goal: &str, track: Track, trial: u32) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join(format!(
        "enth-agent-eval-{}-{stamp}-{goal}-{}-t{trial}",
        std::process::id(),
        track.name()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    Ok(tmp)
}

#[cfg(unix)]
fn install_wrapper(tmp: &Path, cfg: &WrapCfg, xtask: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let path = tmp.join("enth");
    std::fs::write(&path, wrap::script(xtask, cfg))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn install_wrapper(_tmp: &Path, _cfg: &WrapCfg, _xtask: &Path) -> Result<(), String> {
    Err("agent-eval needs a unix shell for its enth wrapper".to_owned())
}

/// What every trial of a run shares.
pub struct Ctx<'a> {
    pub env: &'a Env,
    pub set: &'a EvalSet,
    pub opts: &'a Opts,
    pub run_dir: &'a Path,
    pub stamp: &'a str,
}

/// One trial, start to finish. The trial directory gets the transcript, the final patch and WAV, the judge's raw
/// output, the call log and `result.json`.
pub fn run_trial(
    cx: &Ctx<'_>,
    goal: &Goal,
    track: Track,
    trial: u32,
) -> Result<TrialResult, String> {
    let Ctx {
        env,
        set,
        opts,
        run_dir,
        stamp,
    } = *cx;
    let dir = run_dir
        .join(&goal.id)
        .join(format!("{}-t{trial}", track.name()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = new_tmp(stamp, &goal.id, track, trial)?;
    let cfg = WrapCfg {
        calls: dir.join("enth-calls.jsonl"),
        state: dir.join("budget.state"),
        enth: env.enth.clone(),
        budget: goal.budget.renders,
    };
    let _ = std::fs::remove_file(&cfg.calls);
    let _ = std::fs::remove_file(&cfg.state);
    install_wrapper(&tmp, &cfg, &env.xtask)?;

    let prompt = render_prompt(
        &set.prompt(match track {
            Track::Given => "given",
            Track::SelfWritten => "self",
        })?,
        goal,
    );
    let system = render_prompt(&set.prompt("system")?, goal);
    std::fs::write(dir.join("prompt.md"), &prompt).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("system.md"), &system).map_err(|e| e.to_string())?;

    let started = Instant::now();
    let (exit, timed_out, text) = if opts.dry_run {
        let text = fake::run(
            opts.fake,
            set,
            goal,
            track,
            trial,
            &tmp,
            prompt.len() + system.len(),
        )?;
        std::fs::write(dir.join("stream.jsonl"), &text).map_err(|e| e.to_string())?;
        (Some(0), false, text)
    } else {
        spawn_claude(env, opts, goal, &tmp, &dir, &prompt)?
    };
    let wall_s = started.elapsed().as_secs_f64();
    let summary: StreamSummary = stream::parse(&text);
    let calls = wrap::read_calls(&cfg.calls);
    let mut warnings = summary.warnings.clone();

    let (patch, patch_source) = find_patch(&tmp, summary.structured.as_ref(), &calls);
    let perf = goal.perf.json();
    let mut result = TrialResult {
        schema: TRIAL_SCHEMA.to_owned(),
        eval_version: env.eval_version,
        goal: goal.id.clone(),
        track,
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
        } else {
            "ok".to_owned()
        },
        tokens: TokensOut {
            input: summary.tokens.input,
            output: summary.tokens.output,
            cache_read: summary.tokens.cache_read,
            cache_write: summary.tokens.cache_write,
            total: summary.tokens.total(),
        },
        cost_usd: summary.cost_usd,
        cost_is_estimate: true,
        turns: summary.turns,
        wall_s: util::round4(wall_s),
        calls: calls.len() as u32,
        invalid_calls: calls.iter().filter(|c| c.invalid()).count() as u32,
        refused_calls: calls.iter().filter(|c| c.refused).count() as u32,
        renders: wrap::spent(&cfg.state),
        pass: false,
        thin: 0,
        margins: Vec::new(),
        failed: Vec::new(),
        own_expect: None,
        own_pass: None,
        own_error: None,
        final_patch: None,
        final_wav: None,
        patch_source: patch_source.to_owned(),
        produced_by_enth: false,
        dir: dir.to_string_lossy().into_owned(),
        warnings: Vec::new(),
    };

    if let Some(src) = patch {
        let final_patch = dir.join("final.patch.json");
        std::fs::copy(&src, &final_patch).map_err(|e| format!("{}: {e}", src.display()))?;
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        result.produced_by_enth = calls.iter().any(|c| {
            !c.refused
                && c.exit != 2
                && c.exit != 3
                && matches!(c.sub.as_deref(), Some("new" | "apply" | "step"))
                && c.argv
                    .iter()
                    .any(|a| a == &name || a.ends_with(&format!("/{name}")))
        });
        result.final_patch = Some(final_patch.to_string_lossy().into_owned());

        let judged = judge(
            env,
            &dir,
            Path::new("final.patch.json"),
            &perf,
            &goal.expect,
            "judge.json",
        )?;
        match (judged.code, &judged.json) {
            (Some(0 | 1), Some(j)) => {
                result.pass = judged.code == Some(0);
                result.margins = margins_of(j);
                result.thin = j.get("thin").and_then(Value::as_u64).unwrap_or(0) as u32;
                result.failed = result
                    .margins
                    .iter()
                    .filter(|m| !m.pass)
                    .map(|m| m.expect.clone())
                    .collect();
            }
            _ => warnings.push(format!(
                "the judge failed (exit {:?}): {}",
                judged.code,
                judged.stderr.trim()
            )),
        }
        let wav = dir.join("final.wav");
        let rendered = Command::new(&env.enth)
            .current_dir(&dir)
            .args([
                "render",
                "final.patch.json",
                "--perf",
                &perf,
                "--out",
                "final.wav",
            ])
            .output();
        match rendered {
            Ok(o) if o.status.success() && wav.is_file() => {
                result.final_wav = Some(wav.to_string_lossy().into_owned());
            }
            _ => warnings.push("could not render the final patch to WAV".to_owned()),
        }

        if track == Track::SelfWritten {
            let own = summary
                .structured
                .as_ref()
                .and_then(|a| a.get("expect"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            match own {
                Some(list) if !list.trim().is_empty() => {
                    let j = judge(
                        env,
                        &dir,
                        Path::new("final.patch.json"),
                        &perf,
                        &list,
                        "judge-own.json",
                    )?;
                    match j.code {
                        Some(0) => result.own_pass = Some(true),
                        Some(1) => result.own_pass = Some(false),
                        _ => {
                            result.own_pass = Some(false);
                            let msg = j
                                .json
                                .as_ref()
                                .and_then(|v| v.get("message"))
                                .and_then(Value::as_str)
                                .unwrap_or("the agent's list could not be judged");
                            result.own_error = Some(msg.to_owned());
                        }
                    }
                    result.own_expect = Some(list);
                }
                _ => result.own_error = Some("the answer has no expectation list".to_owned()),
            }
        }
    } else {
        result.status = if timed_out {
            "timeout".to_owned()
        } else {
            "no_patch".to_owned()
        };
        warnings.push("no final patch was found".to_owned());
    }
    result.warnings = warnings;
    util::write_json(
        &dir.join("result.json"),
        &serde_json::to_value(&result).map_err(|e| e.to_string())?,
    )?;
    if !opts.keep_tmp {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    Ok(result)
}

fn git_line(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn command_version(bin: &str) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
                .map(str::to_owned)
        })
        .flatten()
}

fn ensure_enth(root: &Path, target: &Path, opts: &Opts) -> Result<PathBuf, String> {
    if let Some(p) = &opts.enth {
        return if p.is_file() {
            p.canonicalize()
                .map_err(|e| format!("--enth {}: {e}", p.display()))
        } else {
            Err(format!("--enth {}: no such file", p.display()))
        };
    }
    println!("agent-eval: building enth (release)");
    let status = crate::cargo()
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "enth-cli",
            "--bin",
            "enth",
        ])
        .status()
        .map_err(|e| format!("cannot run cargo build: {e}"))?;
    if !status.success() {
        return Err("cargo build -p enth-cli failed".to_owned());
    }
    Ok(target.join("release/enth"))
}

/// `agent-eval run`.
pub fn run(opts: &Opts) -> Result<(), String> {
    if !opts.dry_run && !opts.confirm_spend {
        return Err("a real run calls a model and spends the owner's money (about $9 for 10 goals x 3 trials x 2 \
tracks on Sonnet 5.5, an estimate). Start with --dry-run, which calls none. When the owner has approved the \
spend, add --confirm-spend"
            .to_owned());
    }
    let meta = crate::metadata()?;
    let root = meta.workspace_root.clone();
    let set = super::load_set(&root)?;
    let mut goals: Vec<&Goal> = set.goals.iter().collect();
    if let Some(wanted) = &opts.goals {
        for id in wanted {
            if !set.goals.iter().any(|g| &g.id == id) {
                return Err(format!(
                    "no goal `{id}`; goals: {}",
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
    let enth = ensure_enth(&root, &meta.target_directory, opts)?;
    let xtask =
        std::env::current_exe().map_err(|e| format!("cannot find the xtask binary: {e}"))?;
    let claude = std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".to_owned());
    let auth = match opts.auth {
        Auth::Auto if std::env::var_os("ANTHROPIC_API_KEY").is_some() => Auth::ApiKey,
        Auth::Auto => Auth::Subscription,
        other => other,
    };
    let env = Env {
        enth,
        xtask,
        claude_version: if opts.dry_run {
            None
        } else {
            command_version(&claude)
        },
        claude,
        auth,
        eval_version: set.meta.eval_version,
    };
    if !opts.dry_run && env.claude_version.is_none() {
        return Err(
            "cannot run `claude --version`; install Claude Code or set CLAUDE_BIN".to_owned(),
        );
    }

    let now = util::now_secs();
    let stamp = util::stamp(now);
    let run_dir = opts
        .out
        .clone()
        .unwrap_or_else(|| meta.target_directory.join("agent-eval").join(&stamp));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;

    let header = json!({
        "schema": "enth.agent-eval.header/1",
        "eval_version": set.meta.eval_version,
        "prompt_fingerprint": set.prompt_fingerprint()?,
        "date": util::iso8601(now),
        "stamp": stamp,
        "dry_run": opts.dry_run,
        "runner": if opts.dry_run { format!("fake:{}", opts.fake.name()) } else { "claude-code".to_owned() },
        "model_requested": if opts.dry_run { Value::Null } else { json!(opts.model) },
        "claude_code_version": env.claude_version,
        "auth": if opts.dry_run { "none" } else if env.auth == Auth::ApiKey { "api-key (--bare)" } else { "subscription (--safe-mode)" },
        "enth_git": git_line(&root, &["rev-parse", "HEAD"]),
        "enth_git_dirty": git_line(&root, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty()),
        "enth_version": command_version(&env.enth.to_string_lossy()),
        "trials": opts.trials,
        "tracks": opts.tracks.iter().map(|t| t.name()).collect::<Vec<_>>(),
        "goals": goals.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        "max_usd": opts.max_usd,
        "seed": opts.seed,
    });
    util::write_json(&run_dir.join("header.json"), &header)?;

    let mut plan: Vec<(&Goal, Track, u32)> = Vec::new();
    for goal in &goals {
        for &track in &opts.tracks {
            for trial in 1..=opts.trials {
                plan.push((goal, track, trial));
            }
        }
    }
    println!(
        "agent-eval: {} trials ({} goals x {} tracks x {}), {}, results in {}",
        plan.len(),
        goals.len(),
        opts.tracks.len(),
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

    let cx = Ctx {
        env: &env,
        set: &set,
        opts,
        run_dir: &run_dir,
        stamp: &stamp,
    };
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<TrialResult>> = Mutex::new(Vec::new());
    let spent: Mutex<f64> = Mutex::new(0.0);
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let total = plan.len();
    std::thread::scope(|scope| {
        for _ in 0..opts.jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&(goal, track, trial)) = plan.get(i) else { break };
                    if !opts.dry_run && *spent.lock().expect("lock") >= opts.max_usd {
                        errors
                            .lock()
                            .expect("lock")
                            .push(format!("skipped {} {} t{trial}: --max-usd {} reached", goal.id, track.name(), opts.max_usd));
                        continue;
                    }
                    match run_trial(&cx, goal, track, trial) {
                        Ok(r) => {
                            *spent.lock().expect("lock") += r.cost_usd.unwrap_or(0.0);
                            println!(
                                "[{:>2}/{total}] {:<22} {:<5} t{trial}: {} renders={:<2} thin={} cost=${:.3} calls={} {:.1}s{}",
                                i + 1,
                                goal.id,
                                track.name(),
                                if r.pass { "PASS" } else { "fail" },
                                r.renders,
                                r.thin,
                                r.cost_usd.unwrap_or(0.0),
                                r.calls,
                                r.wall_s,
                                if r.status == "ok" { String::new() } else { format!("  [{}]", r.status) },
                            );
                            results.lock().expect("lock").push(r);
                        }
                        Err(e) => {
                            println!("[{:>2}/{total}] {} {} t{trial}: ERROR {e}", i + 1, goal.id, track.name());
                            errors.lock().expect("lock").push(format!("{} {} t{trial}: {e}", goal.id, track.name()));
                        }
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().expect("lock");
    results.sort_by(|a, b| {
        (&a.goal, a.track.name(), a.trial).cmp(&(&b.goal, b.track.name(), b.trial))
    });
    let errors = errors.into_inner().expect("lock");

    let summary = report::summarize(&header, &results, &set);
    util::write_json(&run_dir.join("summary.json"), &summary)?;
    report::print(&summary);
    for e in &errors {
        println!("agent-eval: {e}");
    }
    if opts.blind && !results.is_empty() {
        match blind::build(&env, &set, &run_dir, &results, opts.seed) {
            Ok(n) => println!(
                "agent-eval: blind-listen set of {n} WAVs in {}/blind (owner: `cargo xtask agent-eval listen {}`)",
                run_dir.display(),
                run_dir.display()
            ),
            Err(e) => println!("agent-eval: no blind-listen set: {e}"),
        }
    }
    if !opts.dry_run && opts.save_summary {
        let model = summary
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .replace(['/', ' '], "-");
        let path = set
            .dir
            .join("results")
            .join(format!("{}-{model}.json", util::date(now)));
        std::fs::create_dir_all(path.parent().expect("has a parent")).map_err(|e| e.to_string())?;
        util::write_json(&path, &summary)?;
        println!(
            "agent-eval: summary saved to {} (commit it)",
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
