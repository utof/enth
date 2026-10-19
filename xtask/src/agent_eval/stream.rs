//! Reading a Claude Code `--output-format stream-json` transcript (one JSON object per line): the model id and Claude
//! Code version from `system/init`, tokens and cost from the closing `result`, and the agent's final answer.
//!
//! Parsing is defensive on purpose: the field names below are the documented ones (Claude Code 2.1, headless docs and
//! the cost-tracking page), but a missing field only leaves its value empty and adds a warning, so a first real run
//! shows what to adapt without losing the trial. The raw `stream.jsonl` is always kept.

use serde::Serialize;
use serde_json::Value;

/// Token counts of a trial.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// What a transcript says about itself.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StreamSummary {
    pub events: usize,
    pub model: Option<String>,
    pub claude_code_version: Option<String>,
    pub session_id: Option<String>,
    /// The tools the init event listed.
    pub tools: Vec<String>,
    pub turns: Option<u64>,
    pub duration_ms: Option<u64>,
    /// `total_cost_usd`: a client-side estimate from a price table, not a bill.
    pub cost_usd: Option<f64>,
    pub tokens: Tokens,
    /// The `result` event's subtype (`success`, `error_max_turns`, ...).
    pub subtype: Option<String>,
    pub is_error: bool,
    /// Tool calls the agent made (`tool_use` blocks).
    pub tool_calls: u32,
    /// The agent's closing text.
    pub result_text: Option<String>,
    /// The structured answer (`structured_output`, or a JSON object found in the closing text).
    pub structured: Option<Value>,
    pub warnings: Vec<String>,
}

fn u64_of(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_u64))
}

/// A JSON object inside `text`, which may be wrapped in a code fence or prose.
pub fn json_object_in(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let (a, b) = (t.find('{')?, t.rfind('}')?);
    if a >= b {
        return None;
    }
    serde_json::from_str::<Value>(&t[a..=b])
        .ok()
        .filter(Value::is_object)
}

/// Parse a whole transcript.
pub fn parse(text: &str) -> StreamSummary {
    let mut s = StreamSummary::default();
    let mut saw_result = false;
    let mut seen_models: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            s.warnings.push("a transcript line is not JSON".to_owned());
            continue;
        };
        s.events += 1;
        match v.get("type").and_then(Value::as_str) {
            Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                s.model = v.get("model").and_then(Value::as_str).map(str::to_owned);
                s.claude_code_version = v
                    .get("claude_code_version")
                    .or_else(|| v.get("version"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                s.session_id = v
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(tools) = v.get("tools").and_then(Value::as_array) {
                    s.tools = tools
                        .iter()
                        .filter_map(|t| t.as_str().map(str::to_owned))
                        .collect();
                }
            }
            Some("assistant") => {
                let msg = v.get("message");
                if let Some(m) = msg.and_then(|m| m.get("model")).and_then(Value::as_str)
                    && !seen_models.iter().any(|x| x == m)
                {
                    seen_models.push(m.to_owned());
                }
                if let Some(blocks) = msg.and_then(|m| m.get("content")).and_then(Value::as_array) {
                    s.tool_calls += blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .count() as u32;
                }
            }
            Some("result") => {
                saw_result = true;
                s.subtype = v.get("subtype").and_then(Value::as_str).map(str::to_owned);
                s.is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                s.turns = u64_of(&v, &["num_turns"]);
                s.duration_ms = u64_of(&v, &["duration_ms"]);
                s.cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                s.result_text = v.get("result").and_then(Value::as_str).map(str::to_owned);
                s.structured = v
                    .get("structured_output")
                    .filter(|x| x.is_object())
                    .cloned();
                if s.session_id.is_none() {
                    s.session_id = v
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                s.tokens = result_tokens(&v);
                if s.cost_usd.is_none() {
                    s.cost_usd = v.get("modelUsage").and_then(Value::as_object).map(|m| {
                        m.values()
                            .filter_map(|u| u.get("costUSD").and_then(Value::as_f64))
                            .sum()
                    });
                }
                if s.model.is_none()
                    && let Some(m) = v.get("modelUsage").and_then(Value::as_object)
                {
                    s.model = m.keys().next().cloned();
                }
            }
            _ => {}
        }
    }
    if s.model.is_none() {
        s.model = seen_models.first().cloned();
    }
    if s.structured.is_none()
        && let Some(text) = &s.result_text
    {
        s.structured = json_object_in(text);
    }
    if s.events == 0 {
        s.warnings.push("the transcript is empty".to_owned());
    }
    if !saw_result {
        s.warnings
            .push("no `result` event: the run ended early or the format changed".to_owned());
    }
    if s.model.is_none() {
        s.warnings
            .push("no model id found (system/init `model`)".to_owned());
    }
    if saw_result && s.cost_usd.is_none() {
        s.warnings
            .push("no `total_cost_usd` in the result event".to_owned());
    }
    if saw_result && s.tokens.total() == 0 {
        s.warnings
            .push("no token counts in the result event".to_owned());
    }
    s
}

/// Tokens from `modelUsage` (summed over models) when present, else from `usage`.
fn result_tokens(v: &Value) -> Tokens {
    if let Some(models) = v.get("modelUsage").and_then(Value::as_object) {
        let mut t = Tokens::default();
        for u in models.values() {
            t.input += u64_of(u, &["inputTokens", "input_tokens"]).unwrap_or(0);
            t.output += u64_of(u, &["outputTokens", "output_tokens"]).unwrap_or(0);
            t.cache_read +=
                u64_of(u, &["cacheReadInputTokens", "cache_read_input_tokens"]).unwrap_or(0);
            t.cache_write += u64_of(
                u,
                &["cacheCreationInputTokens", "cache_creation_input_tokens"],
            )
            .unwrap_or(0);
        }
        if t.total() > 0 {
            return t;
        }
    }
    v.get("usage").map_or_else(Tokens::default, |u| Tokens {
        input: u64_of(u, &["input_tokens", "inputTokens"]).unwrap_or(0),
        output: u64_of(u, &["output_tokens", "outputTokens"]).unwrap_or(0),
        cache_read: u64_of(u, &["cache_read_input_tokens", "cacheReadInputTokens"]).unwrap_or(0),
        cache_write: u64_of(
            u,
            &["cache_creation_input_tokens", "cacheCreationInputTokens"],
        )
        .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SHAPE: &str = r#"
{"type":"system","subtype":"init","session_id":"s-1","model":"claude-sonnet-5-5","claude_code_version":"2.1.296","tools":["Bash"]}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"enth guide"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"..."}]}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"enth new"}}]}}
{"type":"result","subtype":"success","is_error":false,"duration_ms":41000,"num_turns":9,"total_cost_usd":0.1423,"session_id":"s-1","result":"Done: {\"patch\":\"sound.patch.json\",\"expect\":\"attack < 10ms\"}","usage":{"input_tokens":120,"output_tokens":3400,"cache_creation_input_tokens":20000,"cache_read_input_tokens":210000},"modelUsage":{"claude-sonnet-5-5":{"inputTokens":120,"outputTokens":3400,"cacheReadInputTokens":210000,"cacheCreationInputTokens":20000,"costUSD":0.1423}}}
"#;

    #[test]
    fn reads_model_version_tokens_cost_and_the_answer() {
        let s = parse(REAL_SHAPE);
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(s.claude_code_version.as_deref(), Some("2.1.296"));
        assert_eq!(s.turns, Some(9));
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.cost_usd, Some(0.1423));
        assert_eq!(
            s.tokens,
            Tokens {
                input: 120,
                output: 3400,
                cache_read: 210_000,
                cache_write: 20_000
            }
        );
        assert!(!s.is_error);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        // no structured_output field: the answer is found in the closing text
        assert_eq!(s.structured.as_ref().unwrap()["patch"], "sound.patch.json");
    }

    #[test]
    fn structured_output_wins_over_the_text() {
        let line = r#"{"type":"result","subtype":"success","total_cost_usd":0.1,"usage":{"input_tokens":1,"output_tokens":2},"result":"{\"patch\":\"a\"}","structured_output":{"patch":"b","expect":"x"}}"#;
        let s = parse(line);
        assert_eq!(s.structured.unwrap()["patch"], "b");
    }

    #[test]
    fn missing_fields_warn_instead_of_failing() {
        let s = parse("{\"type\":\"system\",\"subtype\":\"init\"}\nnot json\n");
        assert!(s.model.is_none());
        assert!(s.warnings.iter().any(|w| w.contains("no `result`")));
        assert!(s.warnings.iter().any(|w| w.contains("not JSON")));
        assert!(parse("").warnings.iter().any(|w| w.contains("empty")));
    }

    #[test]
    fn json_is_found_inside_fences_and_prose() {
        let v = json_object_in("Here you go:\n```json\n{\"patch\": \"p.json\"}\n```").unwrap();
        assert_eq!(v["patch"], "p.json");
        assert!(json_object_in("no object here").is_none());
        assert!(json_object_in("} {").is_none());
    }

    #[test]
    fn usage_is_the_fallback_when_model_usage_is_absent() {
        let s = parse(
            r#"{"type":"result","subtype":"success","usage":{"input_tokens":5,"output_tokens":7,"cache_read_input_tokens":9}}"#,
        );
        assert_eq!(s.tokens.total(), 21);
    }
}
