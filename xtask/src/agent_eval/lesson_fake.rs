//! The scripted fake agents behind `agent-eval lesson --dry-run` (lane EV3): they make the same `enth` calls an agent
//! would, through the same wrapper (so the unit counter, the refusals and the call log are exercised), and leave a
//! transcript in the `stream-json` shape the real runner parses. No model is called.
//!
//! - `witness`: the pack's own solutions. A do step gets the step's witness patch, a round the nearest passing patch
//!   `enth lesson closest` finds, an ear question the pack's recorded answer. It is the proof that every graded step of
//!   every lesson is reachable with this engine, and the ceiling of the track.
//! - `follower`: the floor, "a scripted agent that follows the hints and nothing else". It reads no text and has no ear.
//!   A do step: apply the measured hint's move until the step's check passes or the hint has none. A round: apply the
//!   coach's last hint (the control and the range it names, to the middle of the range) until the round passes or the
//!   hint has nothing to apply. An ear question: the first option. It costs no money, so the floor is measured for
//!   free, and it is deterministic, so one trial says it all.

use std::path::Path;

use serde_json::{Value, json};

use super::fake::{Kind, Transcript, exec, strings};
use super::lesson_track::{LessonFacts, LessonGoal, StepFacts};
use super::wrap::PACK_DIR;

/// Rounds of "hint, apply" a follower tries on a do step.
const DO_TRIES: usize = 6;
/// Rounds of "compare, hint, apply" a follower tries on a recreate step.
const ROUND_TRIES: usize = 8;
/// The coach's last level: it names the control and a range for it.
const COACH_LAST_LEVEL: usize = 4;

/// What a fake agent works with.
pub struct FakeCx<'a> {
    /// The trial's `enth` wrapper.
    pub wrapper: &'a Path,
    /// The trial's working directory.
    pub tmp: &'a Path,
    pub goal: &'a LessonGoal,
    pub facts: &'a LessonFacts,
    pub trial: u32,
    pub prompt_bytes: usize,
}

/// One scripted call: its exit code and stdout (also logged in the transcript).
struct Caller<'a, 'b> {
    cx: &'a FakeCx<'b>,
    tr: Transcript,
}

impl Caller<'_, '_> {
    fn call(&mut self, args: &[String]) -> Result<(i32, String), String> {
        let (code, stdout) = exec(self.cx.wrapper, self.cx.tmp, args)?;
        self.tr.call(args, &stdout, code != 0 && code != 1);
        Ok((code, stdout))
    }

    /// `enth lesson <sub> pack --lesson L --step ID <extra...>`.
    fn lesson(
        &mut self,
        sub: &str,
        step: &StepFacts,
        extra: &[&str],
    ) -> Result<(i32, String), String> {
        let mut args = strings(&["lesson", sub, PACK_DIR, "--lesson", &self.cx.goal.lesson]);
        args.extend(strings(&["--step", &step.id]));
        args.extend(strings(extra));
        self.call(&args)
    }
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Run a fake agent for one trial. Returns the transcript text.
pub fn run(kind: Kind, cx: &FakeCx<'_>) -> Result<String, String> {
    let session = format!("dry-run-{}-t{}", cx.goal.id, cx.trial);
    let mut c = Caller {
        cx,
        tr: Transcript::new(&session, cx.prompt_bytes),
    };
    // Trial 2 makes one deliberately invalid call, so the invalid-command counter has something to count.
    if cx.trial == 2 {
        c.call(&strings(&["params", "no-such-kind"]))?;
    }
    c.call(&strings(&[
        "lesson",
        "show",
        PACK_DIR,
        "--lesson",
        &cx.goal.lesson,
    ]))?;
    let mut answers = Vec::new();
    // The patch file the player would still have open: where a step with no start of its own carries on from.
    let mut carried: Option<String> = None;
    for step in &cx.facts.steps {
        let entry = match (step.kind.as_str(), kind) {
            ("do", Kind::Witness) => Some(witness_do(&mut c, step, &mut carried)?),
            ("do", Kind::Follower) => Some(follower_do(&mut c, step, &mut carried)?),
            ("recreate", Kind::Witness) => Some(witness_round(&mut c, step, &mut carried)?),
            ("recreate", Kind::Follower) => Some(follower_round(&mut c, step, &mut carried)?),
            ("choose", _) => Some(choose(&mut c, step, kind)?),
            _ => None,
        };
        answers.extend(entry);
    }
    let answer = json!({
        "steps": answers,
        "explanation": format!("scripted {} for {}", kind.name(), cx.goal.id),
    });
    Ok(c.tr.finish(&session, &answer))
}

/// Write the patch a do step opens with to `file`; a step that carries on from the one before copies that.
fn open_do(
    c: &mut Caller<'_, '_>,
    step: &StepFacts,
    file: &str,
    carried: Option<&str>,
) -> Result<(), String> {
    let (code, out) = c.lesson("start", step, &["--out", file])?;
    if code == 0 {
        return Ok(());
    }
    match carried {
        Some(prev)
            if json_of(&out).get("error").and_then(Value::as_str) == Some("carried_start") =>
        {
            std::fs::copy(c.cx.tmp.join(prev), c.cx.tmp.join(file))
                .map_err(|e| format!("cannot carry {prev} on: {e}"))?;
            Ok(())
        }
        None if json_of(&out).get("error").and_then(Value::as_str) == Some("carried_start") => {
            let (code, out) = c.call(&strings(&["new", "--template", "init", "--out", file]))?;
            if code == 0 {
                Ok(())
            } else {
                Err(format!("enth new failed: {}", out.trim()))
            }
        }
        _ => Err(format!(
            "lesson start of step {} failed (exit {code}): {}",
            step.id,
            out.trim()
        )),
    }
}

fn witness_do(
    c: &mut Caller<'_, '_>,
    step: &StepFacts,
    carried: &mut Option<String>,
) -> Result<Value, String> {
    let Some(witness) = &step.witness else {
        return Err(format!("step {} has no witness patch in the pack", step.id));
    };
    let (file, start) = (
        format!("{}.patch.json", step.id),
        format!("{}.start.patch.json", step.id),
    );
    open_do(c, step, &start, carried.as_deref())?;
    std::fs::copy(c.cx.facts.pack_dir.join(witness), c.cx.tmp.join(&file))
        .map_err(|e| format!("cannot copy the witness {witness}: {e}"))?;
    let (code, out) = c.lesson("check", step, &[&file, "--from", &start])?;
    if code != 0 {
        return Err(format!(
            "the witness of {} step {} does not pass its check (exit {code}): {}",
            c.cx.goal.lesson,
            step.id,
            out.trim().chars().take(300).collect::<String>()
        ));
    }
    *carried = Some(file.clone());
    Ok(json!({"step": step.id, "patch": file}))
}

fn follower_do(
    c: &mut Caller<'_, '_>,
    step: &StepFacts,
    carried: &mut Option<String>,
) -> Result<Value, String> {
    let (file, start) = (
        format!("{}.patch.json", step.id),
        format!("{}.start.patch.json", step.id),
    );
    // An untouched copy of the start stays beside the patch being edited, for `check --from`.
    open_do(c, step, &start, carried.as_deref())?;
    std::fs::copy(c.cx.tmp.join(&start), c.cx.tmp.join(&file))
        .map_err(|e| format!("cannot copy the start: {e}"))?;
    *carried = Some(file.clone());
    let measured = (step.hints + 1).to_string();
    for _ in 0..DO_TRIES {
        let (code, _) = c.lesson("check", step, &[&file, "--from", &start])?;
        if code != 1 {
            break; // passed (0), or the tool refused (anything else)
        }
        let (code, out) = c.lesson("hint", step, &["--n", &measured, &file])?;
        let ops = json_of(&out)
            .pointer("/measured/ops")
            .filter(|o| o.as_array().is_some_and(|a| !a.is_empty()))
            .cloned();
        let Some(ops) = ops.filter(|_| code == 0) else {
            break; // no move on offer
        };
        let (code, _) = c.call(&strings(&["apply", &file, "--ops", &ops.to_string()]))?;
        if code != 0 {
            break;
        }
    }
    Ok(json!({"step": step.id, "patch": file}))
}

/// `lesson start` of a round: its start patch and the round file.
fn open_round(c: &mut Caller<'_, '_>, step: &StepFacts) -> Result<(String, String), String> {
    let (file, round) = (
        format!("{}.patch.json", step.id),
        format!("{}.challenge.json", step.id),
    );
    let (code, out) = c.lesson("start", step, &["--out", &file, "--challenge-out", &round])?;
    if code != 0 {
        return Err(format!(
            "lesson start of round {} failed (exit {code}): {}",
            step.id,
            out.trim()
        ));
    }
    Ok((file, round))
}

fn witness_round(
    c: &mut Caller<'_, '_>,
    step: &StepFacts,
    carried: &mut Option<String>,
) -> Result<Value, String> {
    let (start, round) = open_round(c, step)?;
    let answer = format!("{}.answer.patch.json", step.id);
    let (code, out) = c.lesson("closest", step, &[&start, "--out", &answer])?;
    if code != 0 {
        return Err(format!(
            "closest failed on round {} (exit {code}): {}",
            step.id,
            out.trim().chars().take(300).collect::<String>()
        ));
    }
    let (code, out) = c.call(&strings(&["compare", &answer, "--challenge", &round]))?;
    if code != 0 {
        return Err(format!(
            "the nearest patch does not pass round {} of {} (exit {code}): {}",
            step.id,
            c.cx.goal.lesson,
            out.trim().chars().take(300).collect::<String>()
        ));
    }
    *carried = Some(answer.clone());
    Ok(json!({"step": step.id, "patch": answer}))
}

fn follower_round(
    c: &mut Caller<'_, '_>,
    step: &StepFacts,
    carried: &mut Option<String>,
) -> Result<Value, String> {
    let (file, round) = open_round(c, step)?;
    *carried = Some(file.clone());
    let last = (step.hints + COACH_LAST_LEVEL).to_string();
    for _ in 0..ROUND_TRIES {
        let (code, _) = c.call(&strings(&["compare", &file, "--challenge", &round]))?;
        if code != 1 {
            break; // passed, or the tool refused
        }
        let (code, out) = c.lesson("hint", step, &["--n", &last, &file])?;
        let coach = json_of(&out).get("coach").cloned().unwrap_or(Value::Null);
        let target = (code == 0).then(|| coach_target(&coach)).flatten();
        let Some((param, value)) = target else {
            break; // the hint names nothing to set
        };
        let ops = json!([{"set": param, "to": value}]);
        let (code, _) = c.call(&strings(&["apply", &file, "--ops", &ops.to_string()]))?;
        if code != 0 {
            break;
        }
    }
    Ok(json!({"step": step.id, "patch": file}))
}

/// The control and the value a coach hint at its last level points to: the middle of the range it names.
fn coach_target(coach: &Value) -> Option<(String, String)> {
    let param = coach.get("param")?.as_str()?.to_owned();
    let range = coach.get("range")?.as_array()?;
    let (lo, hi) = (range.first()?.as_str()?, range.get(1)?.as_str()?);
    Some((param, midpoint(lo, hi)?))
}

/// A value with its unit as the CLI prints it (`1kHz`, `300ms`, `-6dB`, `40%`), in a base unit.
fn value_of(text: &str) -> Option<(f64, &'static str)> {
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let n: f64 = number.parse().ok()?;
    let (scale, base) = match unit.trim() {
        "kHz" => (1000.0, "Hz"),
        "Hz" => (1.0, "Hz"),
        "s" => (1000.0, "ms"),
        "ms" => (1.0, "ms"),
        "%" => (1.0, "%"),
        "dB" => (1.0, "dB"),
        "ct" | "cents" => (1.0, "ct"),
        "st" | "semi" => (1.0, "st"),
        "" => (1.0, ""),
        _ => return None,
    };
    Some((n * scale, base))
}

/// The middle of two values with the same unit: the geometric mean for a frequency or a time (a pitch-like scale),
/// the arithmetic mean otherwise. `None` when they do not read or do not share a unit.
fn midpoint(lo: &str, hi: &str) -> Option<String> {
    let (a, unit) = value_of(lo)?;
    let (b, unit_b) = value_of(hi)?;
    if unit != unit_b {
        return None;
    }
    let mid = if matches!(unit, "Hz" | "ms") && a > 0.0 && b > 0.0 {
        (a * b).sqrt()
    } else {
        f64::midpoint(a, b)
    };
    let text = format!("{mid:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    Some(format!("{text}{unit}"))
}

fn choose(c: &mut Caller<'_, '_>, step: &StepFacts, kind: Kind) -> Result<Value, String> {
    let dir = format!("{}.ears", step.id);
    let (code, out) = c.lesson("start", step, &["--examples-out", &dir])?;
    if code != 0 {
        return Err(format!(
            "lesson start of ear question {} failed (exit {code}): {}",
            step.id,
            out.trim()
        ));
    }
    let pick = match kind {
        // The witness knows the answer; the follower has no ear and takes the first option.
        Kind::Witness => step.answer,
        Kind::Follower => Some(0),
    };
    let Some(label) = pick.and_then(|i| step.labels.get(i)) else {
        return Err(format!("step {} has no recorded answer", step.id));
    };
    Ok(json!({"step": step.id, "choice": label}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoints_are_geometric_for_frequencies_and_times() {
        assert_eq!(midpoint("1kHz", "1.9kHz").as_deref(), Some("1378.405Hz"));
        assert_eq!(midpoint("100ms", "400ms").as_deref(), Some("200ms"));
        assert_eq!(midpoint("1s", "2000ms").as_deref(), Some("1414.214ms"));
        assert_eq!(midpoint("-12dB", "-6dB").as_deref(), Some("-9dB"));
        assert_eq!(midpoint("20%", "60%").as_deref(), Some("40%"));
        assert_eq!(midpoint("0", "10").as_deref(), Some("5"));
        assert_eq!(midpoint("1kHz", "20%"), None, "units differ");
        assert_eq!(midpoint("1furlong", "2furlong"), None, "unknown unit");
        assert_eq!(midpoint("high", "low"), None);
    }

    #[test]
    fn a_coach_hint_names_a_control_and_a_value_only_at_its_last_level() {
        let last = json!({"param": "filter1.cutoff", "range": ["1kHz", "1.9kHz"], "turn": "lower"});
        assert_eq!(
            coach_target(&last),
            Some(("filter1.cutoff".to_owned(), "1378.405Hz".to_owned()))
        );
        let third = json!({"param": "filter1.cutoff", "turn": "lower"});
        assert_eq!(coach_target(&third), None);
        assert_eq!(coach_target(&Value::Null), None);
    }
}
