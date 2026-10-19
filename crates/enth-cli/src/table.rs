//! The human view: a generic renderer from the normalised JSON. Scalars print as `key: value`, nested objects
//! flatten to dotted keys, and arrays of objects print as aligned tables. A command may name the columns it wants
//! first (`params` puts the range first, because agents in the Phase 0 probe were surprised by range limits).

use std::fmt::Write as _;

use serde_json::Value;

/// Render a normalised output object for a terminal. `first` names columns to put first in tables.
pub fn render(v: &Value, first: &[&str]) -> String {
    let mut out = String::new();
    let Value::Object(m) = v else {
        out.push_str(&scalar(v));
        out.push('\n');
        return out;
    };
    let mut tables: Vec<(String, &Vec<Value>)> = Vec::new();
    let mut lines: Vec<(String, String)> = Vec::new();
    for (k, val) in m {
        if k == "schema" {
            continue;
        }
        collect(k, val, &mut lines, &mut tables);
    }
    let width = lines.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, val) in &lines {
        let _ = writeln!(out, "{k:<width$}  {val}");
    }
    for (name, rows) in tables {
        out.push('\n');
        out.push_str(&name);
        out.push_str(":\n");
        out.push_str(&table(rows, first));
    }
    out
}

fn collect<'a>(
    key: &str,
    v: &'a Value,
    lines: &mut Vec<(String, String)>,
    tables: &mut Vec<(String, &'a Vec<Value>)>,
) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, val) in m {
                collect(&format!("{key}.{k}"), val, lines, tables);
            }
        }
        Value::Array(a) if a.iter().any(Value::is_object) => tables.push((key.to_owned(), a)),
        _ => lines.push((key.to_owned(), scalar(v))),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "-".to_owned(),
        Value::String(s) => s.clone(),
        Value::Array(a) if a.is_empty() => "-".to_owned(),
        Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(", "),
        Value::Object(m) if m.is_empty() => "-".to_owned(),
        other => other.to_string(),
    }
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, val) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, val, out);
            }
        }
        _ => out.push((prefix.to_owned(), scalar(v))),
    }
}

fn table(rows: &[Value], first: &[&str]) -> String {
    let flat: Vec<Vec<(String, String)>> = rows
        .iter()
        .map(|r| {
            let mut cells = Vec::new();
            flatten("", r, &mut cells);
            cells
        })
        .collect();
    let mut cols: Vec<String> = first
        .iter()
        .filter(|c| flat.iter().any(|r| r.iter().any(|(k, _)| k == *c)))
        .map(|c| (*c).to_owned())
        .collect();
    for r in &flat {
        for (k, _) in r {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
    }
    let cell = |r: &Vec<(String, String)>, c: &str| {
        r.iter()
            .find(|(k, _)| k == c)
            .map_or_else(String::new, |(_, v)| v.clone())
    };
    let widths: Vec<usize> = cols
        .iter()
        .map(|c| {
            flat.iter()
                .map(|r| cell(r, c).chars().count())
                .chain(std::iter::once(c.len()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    let row = |values: Vec<String>| {
        let mut s = String::from(" ");
        for (v, w) in values.iter().zip(&widths) {
            let pad = w.saturating_sub(v.chars().count());
            s.push(' ');
            s.push_str(v);
            s.push_str(&" ".repeat(pad));
            s.push(' ');
        }
        s.truncate(s.trim_end().len());
        s.push('\n');
        s
    };
    out.push_str(&row(cols.clone()));
    for r in &flat {
        out.push_str(&row(cols.iter().map(|c| cell(r, c)).collect()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_then_tables() {
        let v = json!({"schema": "enth.x/1", "pass": false, "perf": {"dur_s": 3},
                       "results": [{"expect": "a < 1", "pass": true}, {"expect": "b > 2", "got": null}]});
        let t = render(&v, &["expect"]);
        assert_eq!(
            t,
            "pass        false\nperf.dur_s  3\n\nresults:\n  expect  pass  got\n  a < 1   true\n  b > 2         -\n"
        );
    }
}
