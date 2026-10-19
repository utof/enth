//! Output conventions: JSON normalisation (sorted keys, 4 significant digits) and the choice between JSON and a
//! table.

use serde::Serialize;
use serde_json::{Map, Number, Value};

/// Keys whose values stay exact: op lists, so they can be replayed (`--ops '<inverse>'`).
const EXACT_KEYS: &[&str] = &["ops", "inverse"];

/// The largest integral magnitude printed as an integer (beyond it, a float with 4 significant digits).
const MAX_EXACT_INT: f64 = 1e15;

/// A typed output as a normalised JSON value.
pub fn to_value<T: Serialize>(t: &T) -> Value {
    normalise(serde_json::to_value(t).unwrap_or_else(|e| {
        unreachable!("CLI output types always serialise: {e}");
    }))
}

/// Sort object keys and round numbers: an integral number prints as an integer, any other number with 4
/// significant digits (`0.1235`, `1840`, `2.31e-5`); `-0` prints as `0`. Non-finite numbers cannot occur in
/// `serde_json` values. Values under the keys in [`EXACT_KEYS`] are left as they are.
pub fn normalise(v: Value) -> Value {
    match v {
        Value::Number(n) => Value::Number(round_number(&n)),
        Value::Array(a) => Value::Array(a.into_iter().map(normalise).collect()),
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = m.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in entries {
                let v = if EXACT_KEYS.contains(&k.as_str()) {
                    sort_keys(v)
                } else {
                    normalise(v)
                };
                out.insert(k, v);
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// Sort keys without touching numbers.
fn sort_keys(v: Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.into_iter().map(sort_keys).collect()),
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = m.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, sort_keys(v)))
                    .collect(),
            )
        }
        other => other,
    }
}

fn round_number(n: &Number) -> Number {
    if !n.is_f64() {
        return n.clone();
    }
    let x = n.as_f64().unwrap_or(0.0);
    let x = if x.fract() == 0.0 { x } else { round4(x) };
    if x == 0.0 {
        return Number::from(0);
    }
    if x.fract() == 0.0 && x.abs() < MAX_EXACT_INT {
        let i = x as i64;
        return Number::from(i);
    }
    Number::from_f64(x).unwrap_or_else(|| Number::from(0))
}

/// `x` rounded to 4 significant digits, by decimal formatting (exact and idempotent).
pub fn round4(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.3e}").parse().unwrap_or(x)
}

/// A JSON value as one line.
pub fn line(v: &Value) -> String {
    let mut s = serde_json::to_string(v).unwrap_or_else(|e| unreachable!("values print: {e}"));
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rounds_to_four_significant_digits_and_keeps_integers() {
        let v = normalise(json!({
            "b": 1234.5678, "a": 0.000123456, "c": 3.0, "d": 65535.0, "e": -0.0, "f": 48000,
            "g": [2.71838, {"z": 1.0, "y": 0.5}], "ops": [{"to": 1234.5678}]
        }));
        assert_eq!(
            line(&v),
            "{\"a\":0.0001235,\"b\":1235,\"c\":3,\"d\":65535,\"e\":0,\"f\":48000,\
             \"g\":[2.718,{\"y\":0.5,\"z\":1}],\"ops\":[{\"to\":1234.5678}]}\n"
        );
    }

    #[test]
    fn rounding_is_idempotent() {
        for x in [
            0.123_45,
            1.0 / 3.0,
            9_999.5,
            1e-9 * 7.777_7,
            -42.424_242,
            1e20 / 3.0,
        ] {
            let once = round4(x);
            assert_eq!(round4(once), once, "{x}");
        }
    }
}
