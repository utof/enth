//! Schema 0 → 1. **Synthetic**: schema 0 was never released. It is modelled on the first draft of the patch format
//! (docs/agent-cli.md before the P1a research, `lanes: [[…]]`, an `analog` oscillator with a dB `level` and
//! `detune`), and it exists so that the migration framework has a real step with real fixtures that exercise every
//! kind of change a catalogue edit causes:
//!
//! | v0 | v1 | Kind of change |
//! | --- | --- | --- |
//! | `"name": "…"` | `meta.name` | moved key |
//! | `"master_level": -3` | `master.level` | moved key |
//! | `"lanes": [[…]]` | `voice_fx` for `svf`, `fx` for the rest | split a structure |
//! | module kind `"analog"` | `"osc"` | renamed kind |
//! | param `detune` | `fine` | renamed key |
//! | `level` in dB (−60…6, default −6) | `level` in percent (0…100, default 100) | unit change, with the old default written where a file omits the key |
//! | routes to `<analog>.level` | dropped, with a note | a route depth cannot follow a unit change |
//!
//! Everything else is untouched. The file's `schema` is set by [`super::migrate`].

use serde_json::{Map, Value};

use super::MigrationNote;

const FROM: u32 = 0;
/// v0's default `level` of an `analog` module, in dB.
const OLD_DEFAULT_LEVEL_DB: f64 = -6.0;

fn note(notes: &mut Vec<MigrationNote>, path: String, message: String) {
    notes.push(MigrationNote {
        from: FROM,
        path,
        message,
    });
}

fn obj<'a>(v: &'a mut Value, what: &str) -> Result<&'a mut Map<String, Value>, String> {
    v.as_object_mut()
        .ok_or_else(|| format!("{what} must be an object"))
}

fn list<'a>(map: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>, String> {
    match map.entry(key).or_insert_with(|| Value::Array(Vec::new())) {
        Value::Array(a) => Ok(a),
        _ => Err(format!("\"{key}\" must be an array")),
    }
}

/// dB → percent of full scale, clamped to 0…100 and rounded to 3 decimals. `None` if it was clamped.
fn db_to_percent(db: f64) -> (f64, bool) {
    let linear = 100.0 * libm::pow(10.0, db / 20.0);
    let clamped = linear > 100.0;
    let pct = linear.clamp(0.0, 100.0);
    (libm::round(pct * 1000.0) / 1000.0, clamped)
}

pub(super) fn migrate(mut v: Value, notes: &mut Vec<MigrationNote>) -> Result<Value, String> {
    let root = obj(&mut v, "the file")?;

    if let Some(name) = root.remove("name") {
        let meta = root
            .entry("meta")
            .or_insert_with(|| Value::Object(Map::new()));
        obj(meta, "\"meta\"")?.insert("name".to_owned(), name);
    }
    if let Some(level) = root.remove("master_level") {
        let master = root
            .entry("master")
            .or_insert_with(|| Value::Object(Map::new()));
        obj(master, "\"master\"")?.insert("level".to_owned(), level);
    }

    if let Some(lanes) = root.remove("lanes") {
        let Value::Array(lanes) = lanes else {
            return Err("\"lanes\" must be an array of arrays".to_owned());
        };
        let mut voice_fx = Vec::new();
        let mut fx = Vec::new();
        for lane in lanes {
            let Value::Array(modules) = lane else {
                return Err("\"lanes\" must be an array of arrays".to_owned());
            };
            for m in modules {
                if m.get("kind").and_then(Value::as_str) == Some("svf") {
                    voice_fx.push(m);
                } else {
                    fx.push(m);
                }
            }
        }
        list(root, "voice_fx")?.extend(voice_fx);
        list(root, "fx")?.extend(fx);
    }

    let mut analog_ids: Vec<String> = Vec::new();
    for (i, m) in list(root, "generators")?.iter_mut().enumerate() {
        let m = obj(m, "a generator")?;
        if m.get("kind").and_then(Value::as_str) != Some("analog") {
            continue;
        }
        m.insert("kind".to_owned(), Value::from("osc"));
        let id = m
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let at = format!("generators[{i}].params");
        let params = m
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()));
        let params = obj(params, "\"params\"")?;
        if let Some(detune) = params.remove("detune") {
            params.insert("fine".to_owned(), detune);
        }
        let old_db = match params.get("level") {
            None => {
                note(
                    notes,
                    format!("{at}.level"),
                    format!(
                        "{id}.level: the v0 default of {OLD_DEFAULT_LEVEL_DB} dB is written out as a percentage"
                    ),
                );
                OLD_DEFAULT_LEVEL_DB
            }
            Some(x) => x
                .as_f64()
                .ok_or_else(|| format!("{id}.level must be a number in dB"))?,
        };
        let (pct, clamped) = db_to_percent(old_db);
        if clamped {
            note(
                notes,
                format!("{at}.level"),
                format!("{id}.level: {old_db} dB is above full scale and was clamped to 100 %"),
            );
        }
        let pct = serde_json::Number::from_f64(pct)
            .ok_or_else(|| format!("{id}.level is not a finite number"))?;
        params.insert("level".to_owned(), Value::Number(pct));
        analog_ids.push(id);
    }

    let routes = list(root, "routes")?;
    let before = routes.len();
    let mut kept = Vec::with_capacity(before);
    for (i, r) in core::mem::take(routes).into_iter().enumerate() {
        let dst = r.get("dst").and_then(Value::as_str).unwrap_or_default();
        let drops = dst
            .strip_suffix(".level")
            .is_some_and(|id| analog_ids.iter().any(|a| a == id));
        if drops {
            note(
                notes,
                format!("routes[{i}]"),
                format!(
                    "the route to {dst} was dropped: level changed from dB to percent, so its depth no longer means the same"
                ),
            );
        } else {
            kept.push(r);
        }
    }
    *routes = kept;
    Ok(v)
}
