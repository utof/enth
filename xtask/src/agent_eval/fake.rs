//! The scripted fake agents behind `--dry-run`: they make the same `enth` calls an agent would, through the same
//! wrapper (so the budget counter, the call log and the judge are exercised), and leave a transcript in the
//! `stream-json` shape the real runner parses. No model is called.
//!
//! - `witness`: replays the goal's hand-picked solution (`witness/<id>.toml`). It is also the proof that each goal's
//!   expectation list is reachable with the current engine.
//! - `follower`: the mechanical baseline. It starts from the template the witness starts from, then applies `check`'s `fix.ops` until the
//!   list passes or no fix is offered (it cannot add modules, so it stops where `blocked` says structure is needed).

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::goal::{EvalSet, Goal, fill};
use super::run::Track;

/// The patch file every fake run writes.
pub const PATCH: &str = "sound.patch.json";
/// The model id the fake transcripts carry.
pub const MODEL: &str = "dry-run-scripted-agent";

/// Which fake agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Witness,
    Follower,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Witness => "witness",
            Kind::Follower => "follower",
        }
    }

    pub fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "witness" => Ok(Kind::Witness),
            "follower" => Ok(Kind::Follower),
            other => Err(format!(
                "--agent must be witness or follower, not `{other}`"
            )),
        }
    }
}

/// A transcript under construction.
struct Transcript {
    lines: Vec<Value>,
    id: u32,
    bytes_in: u64,
    bytes_out: u64,
}

impl Transcript {
    fn new(session: &str, prompt_bytes: usize) -> Transcript {
        Transcript {
            lines: vec![json!({
                "type": "system", "subtype": "init", "session_id": session, "model": MODEL,
                "claude_code_version": "dry-run", "tools": ["Bash"],
            })],
            id: 0,
            bytes_in: prompt_bytes as u64,
            bytes_out: 0,
        }
    }

    fn call(&mut self, args: &[String], stdout: &str, is_error: bool) {
        self.id += 1;
        let id = format!("toolu_{}", self.id);
        let command = format!("enth {}", args.join(" "));
        self.bytes_out += command.len() as u64 / 3 + 20;
        self.bytes_in += stdout.len() as u64;
        self.lines.push(
            json!({"type": "assistant", "message": {"model": MODEL, "content": [
            {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}}),
        );
        self.lines.push(json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": id, "content": stdout, "is_error": is_error}]}}));
    }

    /// The closing `result` event. Token counts are synthetic (bytes over 4) and the cost uses Sonnet 5.5's list
    /// prices, so a dry run exercises the cost guard and the medians; none of it is a measurement.
    fn finish(mut self, session: &str, answer: &Value) -> String {
        let input = self.bytes_in / 4 * u64::from(self.id.max(1)) / 2 + 1;
        let output = self.bytes_out / 4 + 40;
        let cost = input as f64 * 2e-6 + output as f64 * 10e-6;
        self.lines.push(json!({
            "type": "result", "subtype": "success", "is_error": false, "duration_ms": 1000 * u64::from(self.id),
            "num_turns": self.id + 1, "total_cost_usd": cost, "session_id": session,
            "result": answer.to_string(), "structured_output": answer,
            "usage": {"input_tokens": input, "output_tokens": output,
                      "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
            "modelUsage": {MODEL: {"inputTokens": input, "outputTokens": output, "costUSD": cost}},
        }));
        let mut text = String::new();
        for l in &self.lines {
            text.push_str(&l.to_string());
            text.push('\n');
        }
        text
    }
}

fn exec(wrapper: &Path, cwd: &Path, args: &[String]) -> Result<(i32, String), String> {
    let out = Command::new(wrapper)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", wrapper.display()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        // a wrapper that died before running enth says why on stderr
        text = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    Ok((out.status.code().unwrap_or(1), text))
}

/// Run a fake agent for one trial. Returns the transcript text.
pub fn run(
    kind: Kind,
    set: &EvalSet,
    goal: &Goal,
    track: Track,
    trial: u32,
    tmp: &Path,
    prompt_bytes: usize,
) -> Result<String, String> {
    let wrapper = tmp.join("enth");
    let session = format!("dry-run-{}-{}-t{trial}", goal.id, track.name());
    let mut tr = Transcript::new(&session, prompt_bytes);
    let perf = goal.perf.json();
    let witness = set.witness(&goal.id)?;
    let expect = match track {
        Track::Given => goal.expect.clone(),
        Track::SelfWritten => witness.self_expect.clone(),
    };
    let mut run_one = |tr: &mut Transcript, args: Vec<String>| -> Result<(i32, String), String> {
        let (code, stdout) = exec(&wrapper, tmp, &args)?;
        tr.call(&args, &stdout, code != 0 && code != 1);
        Ok((code, stdout))
    };
    // Trial 2 makes one deliberately invalid call, so the invalid-command counter has something to count.
    if trial == 2 {
        run_one(&mut tr, strings(&["params", "no-such-kind"]))?;
    }
    match kind {
        Kind::Witness => {
            let mut last = (0, String::new());
            for step in &witness.step {
                let args = step
                    .args
                    .iter()
                    .map(|a| fill(a, &[("perf", &perf), ("expect", &expect)]))
                    .collect();
                last = run_one(&mut tr, args)?;
                if last.0 >= 2 {
                    return Err(format!(
                        "witness for {} failed on a call (exit {}): {}",
                        goal.id,
                        last.0,
                        last.1.trim()
                    ));
                }
            }
            // The witness is the proof that the list is reachable: its last call must pass.
            if last.0 != 0 {
                return Err(format!(
                    "witness for {} ({} list) does not pass: {}",
                    goal.id,
                    track.name(),
                    failing_items(&last.1)
                ));
            }
        }
        Kind::Follower => {
            // It starts from the template a person would pick: the one the witness starts from.
            let template = witness
                .step
                .first()
                .and_then(|st| {
                    st.args
                        .iter()
                        .position(|a| a == "--template")
                        .and_then(|i| st.args.get(i + 1))
                })
                .map_or(goal.template.as_str(), String::as_str);
            follow(&mut tr, &mut run_one, template, &perf, &expect)?;
        }
    }
    let answer = json!({
        "patch": PATCH,
        "expect": expect,
        "explanation": format!("scripted {} for {}", kind.name(), goal.id),
    });
    Ok(tr.finish(&session, &answer))
}

/// The failing items of a `step` or `check` output, for an error message.
fn failing_items(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout).unwrap_or(Value::Null);
    let check = v.get("check").unwrap_or(&v);
    let items: Vec<String> = check
        .get("results")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter(|r| r.get("pass") == Some(&Value::Bool(false)))
                .map(|r| {
                    format!(
                        "{} (got {}, margin {})",
                        r.get("expect").and_then(Value::as_str).unwrap_or("?"),
                        r.get("got").unwrap_or(&Value::Null),
                        r.get("margin").unwrap_or(&Value::Null)
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        stdout.trim().chars().take(300).collect()
    } else {
        items.join("; ")
    }
}

type RunOne<'a> = dyn FnMut(&mut Transcript, Vec<String>) -> Result<(i32, String), String> + 'a;

fn follow(
    tr: &mut Transcript,
    run_one: &mut RunOne<'_>,
    template: &str,
    perf: &str,
    expect: &str,
) -> Result<(), String> {
    let (code, out) = run_one(
        tr,
        strings(&["new", "--template", template, "--out", PATCH]),
    )?;
    if code != 0 {
        return Err(format!("enth new failed: {out}"));
    }
    for _ in 0..12 {
        let (code, out) = run_one(
            tr,
            strings(&["check", PATCH, "--perf", perf, "--expect", expect]),
        )?;
        if code == 0 {
            return Ok(());
        }
        if code != 1 {
            return Err(format!(
                "follower: check failed with exit {code}: {}",
                out.trim()
            ));
        }
        let v: Value =
            serde_json::from_str(&out).map_err(|e| format!("follower: check output: {e}"))?;
        let Some(ops) = v
            .get("fix")
            .and_then(|f| f.get("ops"))
            .filter(|o| o.is_array())
        else {
            return Ok(()); // no fix on offer (blocked or unfinished): the follower stops
        };
        let (code, out) = run_one(tr, strings(&["apply", PATCH, "--ops", &ops.to_string()]))?;
        if code != 0 {
            return Err(format!("follower: apply failed: {}", out.trim()));
        }
    }
    Ok(())
}

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|&s| s.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_parses_back_with_the_same_totals() {
        let mut tr = Transcript::new("s", 400);
        tr.call(&strings(&["guide"]), "{\"text\":\"...\"}", false);
        tr.call(&strings(&["params", "nope"]), "{\"error\":\"x\"}", true);
        let text = tr.finish(
            "s",
            &json!({"patch": "sound.patch.json", "expect": "attack < 10ms"}),
        );
        let s = super::super::stream::parse(&text);
        assert_eq!(s.model.as_deref(), Some(MODEL));
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.turns, Some(3));
        assert!(s.cost_usd.unwrap() > 0.0);
        assert!(s.tokens.total() > 0);
        assert_eq!(s.structured.unwrap()["expect"], "attack < 10ms");
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    }

    #[test]
    fn kind_names_round_trip() {
        assert_eq!(Kind::parse("witness"), Ok(Kind::Witness));
        assert_eq!(Kind::parse("follower").map(Kind::name), Ok("follower"));
        assert!(Kind::parse("x").is_err());
    }
}
