//! **Owner: lane DEC.** The authoring form of a generated lesson: `lesson.toml` in C1's style (docs/learn.md, "The
//! lesson model (lane LM)": authors write TOML; `cargo xtask pack` writes the canonical JSON). Pure text building, no
//! TOML dependency: the lesson's own JSON (`Lesson::to_json`) is walked and written as TOML, with
//!
//! - a comment block above each step (what was measured for it);
//! - `[[steps]]`, `[[steps.examples]]`, `[[steps.options]]` and `[[steps.check.checks]]` as arrays of tables, other
//!   object arrays (predicates, a patch's modules) inline;
//! - each inline patch written sparse, as C1 writes them: a parameter at its catalogue default, an enabled flag that
//!   is on, a default voice, no routes, unnamed macros at 0 and no slots are left out, and the patch reader fills
//!   them back in, so the lesson reads back equal (a test checks it).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use enth_patch::{ModuleId, ModuleInstance, Op, Patch, Position, Section, apply};
use serde_json::{Map, Value};

use crate::lesson::Lesson;

/// Keys written first, in this order (the rest follow alphabetically): what a reader looks for first in a lesson, a
/// step, a check, a challenge and a patch.
const FIRST: [&str; 26] = [
    "schema",
    "id",
    "kind",
    "title",
    "tier",
    "objective",
    "teaches",
    "text",
    "label",
    "patch",
    "phrase",
    "answer",
    "witness",
    "hints",
    "engine",
    "meta",
    "voice",
    "master",
    "generators",
    "voice_fx",
    "fx",
    "modulators",
    "routes",
    "macros",
    "predicates",
    "expect",
];

/// `obj`'s entries in [`FIRST`]'s order, then alphabetically.
fn ordered(obj: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut v: Vec<(&String, &Value)> = obj.iter().collect();
    v.sort_by_key(|(k, _)| {
        (
            FIRST.iter().position(|f| f == k).unwrap_or(FIRST.len()),
            (*k).clone(),
        )
    });
    v
}

/// Object arrays written as arrays of tables (`[[…]]`); every other object array is written inline.
const TABLE_ARRAYS: [&str; 4] = ["steps", "examples", "options", "checks"];

/// `lesson` as `lesson.toml`: `header` as the top comment, `notes` (by step id) as comments above each step.
pub fn lesson_toml(
    lesson: &Lesson,
    header: &[String],
    notes: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut value: Value = serde_json::from_str(&lesson.to_json()).unwrap_or(Value::Null);
    sparsen_patches(&mut value);
    let mut out = String::new();
    for line in header {
        comment(&mut out, line);
    }
    if !header.is_empty() {
        out.push('\n');
    }
    let Value::Object(top) = value else {
        return out;
    };
    for (k, v) in ordered(&top) {
        if k != "steps" {
            let _ = writeln!(out, "{} = {}", key(k), inline(v));
        }
    }
    if let Some(Value::Array(steps)) = top.get("steps") {
        for step in steps {
            let Value::Object(s) = step else { continue };
            out.push('\n');
            if let Some(Value::String(id)) = s.get("id") {
                for line in notes.get(id).into_iter().flatten() {
                    comment(&mut out, line);
                }
            }
            out.push_str("[[steps]]\n");
            table_body(&mut out, "steps", s);
        }
    }
    out
}

/// `# line`, wrapped at about 118 columns.
fn comment(out: &mut String, line: &str) {
    let mut cur = String::from("#");
    for word in line.split_whitespace() {
        if cur.len() + 1 + word.len() > 118 && cur.len() > 1 {
            out.push_str(&cur);
            out.push('\n');
            cur = String::from("#  ");
        }
        cur.push(' ');
        cur.push_str(word);
    }
    out.push_str(&cur);
    out.push('\n');
}

/// The body of the table at `path`: its plain values, then its sub-tables, then its arrays of tables.
fn table_body(out: &mut String, path: &str, obj: &Map<String, Value>) {
    let is_patch = path.rsplit('.').next() == Some("patch");
    let mut tables = Vec::new();
    let mut arrays = Vec::new();
    for (k, v) in ordered(obj) {
        match v {
            Value::Null => {}
            Value::Object(o) if !is_patch => tables.push((k, o)),
            Value::Array(a)
                if !is_patch
                    && TABLE_ARRAYS.contains(&k.as_str())
                    && a.iter().all(Value::is_object) =>
            {
                arrays.push((k, a));
            }
            Value::Array(a) if is_patch && a.iter().all(Value::is_object) && !a.is_empty() => {
                let _ = writeln!(out, "{} = [", key(k));
                for item in a {
                    let _ = writeln!(out, "  {},", inline(item));
                }
                out.push_str("]\n");
            }
            _ => {
                let _ = writeln!(out, "{} = {}", key(k), inline(v));
            }
        }
    }
    for (k, o) in tables {
        let p = format!("{path}.{}", key(k));
        let _ = writeln!(out, "\n[{p}]");
        table_body(out, &p, o);
    }
    for (k, a) in arrays {
        let p = format!("{path}.{}", key(k));
        for item in a {
            if let Value::Object(o) = item {
                let _ = writeln!(out, "\n[[{p}]]");
                table_body(out, &p, o);
            }
        }
    }
}

/// A key: bare when it can be, else quoted.
fn key(k: &str) -> String {
    if !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        k.to_owned()
    } else {
        string(k)
    }
}

/// A TOML basic string (JSON's escapes are TOML's).
fn string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A value written inline: strings, numbers (a float always with a point or an exponent), booleans, arrays and
/// inline tables. Nulls are left out of tables (an absent optional field reads as none).
fn inline(v: &Value) -> String {
    match v {
        Value::Null => "\"\"".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                let s = format!("{f:?}");
                if s.contains(['.', 'e', 'E']) {
                    s
                } else {
                    format!("{s}.0")
                }
            }
        }
        Value::String(s) => string(s),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(inline).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(o) => {
            let items: Vec<String> = ordered(o)
                .into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("{} = {}", key(k), inline(v)))
                .collect();
            if items.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{ {} }}", items.join(", "))
            }
        }
    }
}

/// Every inline patch in a lesson's JSON (a challenge's `target.patch` and `start.patch`), made sparse.
fn sparsen_patches(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, child) in o.iter_mut() {
                if k == "patch" && child.is_object() {
                    sparsen(child);
                } else {
                    sparsen_patches(child);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(sparsen_patches),
        _ => {}
    }
}

/// The default parameters of a module kind as the patch file writes them.
fn default_params(kind: &str) -> Option<Value> {
    let kind = enth_patch::ModuleKind::from_key(kind)?;
    let mut p = Patch::init();
    let id = ModuleId::new("probe1").ok()?;
    let section = kind.section();
    let index = p.section(section).len();
    apply(
        &mut p,
        Op::AddModule {
            at: Position { section, index },
            module: ModuleInstance::new(id, kind),
        },
    )
    .ok()?;
    let v: Value = serde_json::from_str(&p.to_json()).ok()?;
    v.get(section.key())?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str) == Some("probe1"))?
        .get("params")
        .cloned()
}

/// One patch file's JSON made sparse (see the [module docs](self)).
fn sparsen(patch: &mut Value) {
    let Value::Object(p) = patch else { return };
    let init: Value = serde_json::from_str(&Patch::init().to_json()).unwrap_or(Value::Null);
    let mut defaults: BTreeMap<String, Option<Value>> = BTreeMap::new();
    for section in Section::ALL {
        let Some(Value::Array(mods)) = p.get_mut(section.key()) else {
            continue;
        };
        for m in mods.iter_mut() {
            let Value::Object(m) = m else { continue };
            let kind = m
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let def = defaults
                .entry(kind.clone())
                .or_insert_with(|| default_params(&kind))
                .clone();
            if let (Some(Value::Object(params)), Some(Value::Object(def))) =
                (m.get_mut("params"), def)
            {
                params.retain(|k, v| def.get(k) != Some(v));
            }
            if m.get("params")
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                m.remove("params");
            }
            if m.get("enabled") == Some(&Value::Bool(true)) {
                m.remove("enabled");
            }
        }
    }
    if let (Some(Value::Object(voice)), Some(Value::Object(dv))) =
        (p.get_mut("voice"), init.get("voice"))
    {
        voice.retain(|k, v| dv.get(k) != Some(v));
    }
    if p.get("voice")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        p.remove("voice");
    }
    if p.get("routes")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        p.remove("routes");
    }
    if p.get("macros") == init.get("macros") {
        p.remove("macros");
    }
    if p.get("slots")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        p.remove("slots");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_keys_are_toml() {
        assert_eq!(inline(&serde_json::json!(320.0)), "320.0");
        assert_eq!(inline(&serde_json::json!(1)), "1");
        assert_eq!(inline(&serde_json::json!("a \"b\"")), "\"a \\\"b\\\"\"");
        assert_eq!(key("osc1"), "osc1");
        assert_eq!(key("a b"), "\"a b\"");
        assert_eq!(
            inline(&serde_json::json!({"kind": "value", "x": null, "v": [1, 2.5]})),
            "{ kind = \"value\", v = [1, 2.5] }"
        );
    }

    #[test]
    fn a_sparse_patch_reads_back_equal() {
        let starter = super::super::element::tests::starter();
        let mut v: Value = serde_json::from_str(&starter.to_json()).unwrap();
        sparsen(&mut v);
        let o = v.as_object().unwrap();
        assert!(!o.contains_key("routes") && !o.contains_key("macros") && !o.contains_key("voice"));
        let back = enth_patch::file::load_value(v).unwrap().patch;
        assert_eq!(back, starter);
    }
}
