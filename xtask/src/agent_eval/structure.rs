//! The structure check of a goal (`enth.agent-eval/2`, eval version 4, lane EV5): what the final patch is *made of*,
//! beside how it sounds.
//!
//! The descriptors cannot tell an FM growl from a filtered saw, or a unison reese from three detuned oscillators, so
//! a goal that names a technique lists it in `requires`, and the judge passes a patch only if `enth check` passes
//! **and** every requirement holds. Each requirement is one string, read from `enth get` (the product's own view of a
//! patch, so the check sees defaults and enabled flags the way the CLI does):
//!
//! | form | holds when |
//! | --- | --- |
//! | `wt` or `wt x2` | at least one (or N) enabled module of that kind is in the patch |
//! | `wt -> wt.pm` | an enabled source of that kind (or `any`: a module of any kind, or `velocity`, `key`, ...) has a route into that parameter of an enabled module of the destination kind (`*` for any parameter) |
//! | `lfo.sync = on` | some enabled module of that kind has that parameter equal to the value (`true` and `on` are the same) |
//! | `wt.unison >= 3` | some enabled module of that kind has a numeric value at least as large |
//!
//! Nothing here renders: `enth get` is free of the render budget, and the judge runs it after `enth check`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// One parsed requirement, with the text it was written as.
#[derive(Clone, Debug, PartialEq)]
pub struct Requirement {
    pub text: String,
    pub what: What,
}

/// What a requirement asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// At least `min` enabled modules of `kind`.
    Kind { kind: String, min: usize },
    /// A route from a `src` kind (or `any`) into `dst`'s `param` (`*`: any).
    Route {
        src: String,
        dst: String,
        param: String,
    },
    /// Some enabled module of `kind` has `param` `op` `value`.
    Param {
        kind: String,
        param: String,
        op: Op,
        value: String,
    },
}

/// A comparison of a parameter requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Equals,
    AtLeast,
}

fn word(text: &str, what: &str, whole: &str) -> Result<String, String> {
    let t = text.trim();
    let ok = !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok || (what == "parameter" && t == "*") || (what == "source" && t == "any") {
        Ok(t.to_owned())
    } else {
        Err(format!(
            "requirement `{whole}`: `{t}` is not a {what} (lowercase letters, digits and `_`)"
        ))
    }
}

/// Read one requirement string.
pub fn parse(text: &str) -> Result<Requirement, String> {
    let whole = text.trim();
    let what = if let Some((src, dst)) = whole.split_once("->") {
        let (kind, param) = dst.split_once('.').ok_or_else(|| {
            format!("requirement `{whole}`: a route needs `kind.param` after `->`")
        })?;
        What::Route {
            src: word(src, "source", whole)?,
            dst: word(kind, "kind", whole)?,
            param: word(param, "parameter", whole)?,
        }
    } else if let Some((left, value, op)) = whole
        .split_once(">=")
        .map(|(l, v)| (l, v, Op::AtLeast))
        .or_else(|| whole.split_once('=').map(|(l, v)| (l, v, Op::Equals)))
    {
        let (kind, param) = left.split_once('.').ok_or_else(|| {
            format!(
                "requirement `{whole}`: a parameter check needs `kind.param` before the operator"
            )
        })?;
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            return Err(format!(
                "requirement `{whole}`: no value after the operator"
            ));
        }
        if op == Op::AtLeast && value.parse::<f64>().is_err() {
            return Err(format!(
                "requirement `{whole}`: `>=` needs a plain number, not `{value}`"
            ));
        }
        What::Param {
            kind: word(kind, "kind", whole)?,
            param: word(param, "parameter", whole)?,
            op,
            value,
        }
    } else {
        let mut parts = whole.split_whitespace();
        let kind = word(parts.next().unwrap_or(""), "kind", whole)?;
        let min = match parts.next() {
            None => 1,
            Some(n) => n
                .strip_prefix('x')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| *n >= 1)
                .ok_or_else(|| format!("requirement `{whole}`: a count is written `x2`"))?,
        };
        if parts.next().is_some() {
            return Err(format!(
                "requirement `{whole}`: not a kind, a route or a parameter check"
            ));
        }
        What::Kind { kind, min }
    };
    Ok(Requirement {
        text: whole.to_owned(),
        what,
    })
}

/// A module of `enth get`'s list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Module {
    id: String,
    kind: String,
    enabled: bool,
}

/// A route of `enth get`'s list: source name, destination module id and parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Route {
    src: String,
    dst_id: String,
    dst_param: String,
}

/// The parts of an `enth.get/1` reply the check reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    modules: Vec<Module>,
    routes: Vec<Route>,
}

impl View {
    /// Read `enth get <patch>` (no addresses): the modules and the routes.
    pub fn from_get(get: &Value) -> Result<View, String> {
        let modules = get
            .get("modules")
            .and_then(Value::as_array)
            .ok_or("`enth get` has no `modules`")?
            .iter()
            .map(|m| {
                Some(Module {
                    id: m.get("id")?.as_str()?.to_owned(),
                    kind: m.get("kind")?.as_str()?.to_owned(),
                    enabled: m.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("a module of `enth get` has no id or kind")?;
        let mut routes = Vec::new();
        for r in get
            .get("routes")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            let text = r
                .get("route")
                .and_then(Value::as_str)
                .ok_or("a route of `enth get` has no `route`")?;
            let (src, dst) = text
                .split_once("->")
                .ok_or_else(|| format!("route `{text}` has no `->`"))?;
            let (dst_id, dst_param) = dst
                .trim()
                .split_once('.')
                .ok_or_else(|| format!("route `{text}` has no `.param` after `->`"))?;
            routes.push(Route {
                src: src.trim().to_owned(),
                dst_id: dst_id.to_owned(),
                dst_param: dst_param.to_owned(),
            });
        }
        Ok(View { modules, routes })
    }

    fn enabled(&self, kind: &str) -> impl Iterator<Item = &Module> {
        self.modules
            .iter()
            .filter(move |m| m.enabled && m.kind == kind)
    }

    fn module(&self, id: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.id == id)
    }

    /// The addresses a parameter requirement needs read: every enabled module of its kind, with the parameter.
    fn addresses(&self, reqs: &[Requirement]) -> Vec<String> {
        let mut out = Vec::new();
        for r in reqs {
            if let What::Param { kind, param, .. } = &r.what {
                for m in self.enabled(kind) {
                    let a = format!("{}.{param}", m.id);
                    if !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }

    fn route_holds(&self, src: &str, dst: &str, param: &str) -> bool {
        self.routes.iter().any(|r| {
            let to = self.module(&r.dst_id);
            let from = self.module(&r.src);
            let to_ok = to.is_some_and(|m| m.enabled && m.kind == dst)
                && (param == "*" || r.dst_param == param);
            let from_ok = match from {
                Some(m) => m.enabled && (src == "any" || m.kind == src),
                // a source that is not a module: `velocity`, `key`, `modwheel`, `macro1`...
                None => src == "any" || r.src == src,
            };
            to_ok && from_ok
        })
    }
}

fn normal(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" => "on".to_owned(),
        "false" | "no" => "off".to_owned(),
        other => other.to_owned(),
    }
}

/// The leading number of a printed value (`7`, `+30ct`, `66%`).
fn number(value: &str) -> Option<f64> {
    let t = value.trim().trim_start_matches('+');
    let end = t
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && c == '-')))
        .map_or(t.len(), |(i, _)| i);
    t[..end].parse().ok()
}

/// The requirements that do not hold on `view`; `values` maps an address (`wt1.unison`) to its printed value.
pub fn unmet(view: &View, values: &BTreeMap<String, String>, reqs: &[Requirement]) -> Vec<String> {
    reqs.iter()
        .filter(|r| !match &r.what {
            What::Kind { kind, min } => view.enabled(kind).count() >= *min,
            What::Route { src, dst, param } => view.route_holds(src, dst, param),
            What::Param {
                kind,
                param,
                op,
                value,
            } => view.enabled(kind).any(|m| {
                values
                    .get(&format!("{}.{param}", m.id))
                    .is_some_and(|v| match op {
                        Op::Equals => normal(v) == normal(value),
                        Op::AtLeast => match (number(v), value.parse::<f64>()) {
                            (Some(got), Ok(want)) => got >= want,
                            _ => false,
                        },
                    })
            }),
        })
        .map(|r| r.text.clone())
        .collect()
}

fn get(enth: &Path, dir: &Path, patch: &Path, addresses: &[String]) -> Result<Value, String> {
    let out = Command::new(enth)
        .current_dir(dir)
        .arg("get")
        .arg(patch)
        .args(addresses)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", enth.display()))?;
    if !out.status.success() {
        return Err(format!(
            "enth get exited {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("enth get output: {e}"))
}

/// Run the requirements on a patch file: `enth get` for the modules and routes, and once more for the parameters.
/// Returns the requirements that do not hold (empty: all hold).
pub fn check(
    enth: &Path,
    dir: &Path,
    patch: &Path,
    reqs: &[Requirement],
) -> Result<Vec<String>, String> {
    if reqs.is_empty() {
        return Ok(Vec::new());
    }
    let view = View::from_get(&get(enth, dir, patch, &[])?)?;
    let addresses = view.addresses(reqs);
    let mut values = BTreeMap::new();
    if !addresses.is_empty() {
        let reply = get(enth, dir, patch, &addresses)?;
        for (k, v) in reply
            .get("values")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(s) = v.as_str() {
                values.insert(k.clone(), s.to_owned());
            }
        }
    }
    Ok(unmet(&view, &values, reqs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reqs(texts: &[&str]) -> Vec<Requirement> {
        texts.iter().map(|t| parse(t).expect("parses")).collect()
    }

    fn fm() -> Value {
        json!({"schema": "enth.get/1", "modules": [
            {"id": "wt1", "kind": "wt", "enabled": true, "section": "generators"},
            {"id": "wt2", "kind": "wt", "enabled": true, "section": "generators"},
            {"id": "env1", "kind": "adsr", "enabled": true, "section": "modulators"},
            {"id": "lfo1", "kind": "lfo", "enabled": false, "section": "modulators"},
        ], "routes": [
            {"route": "wt2 -> wt1.pm", "depth": 100},
            {"route": "lfo1 -> wt1.pm_index", "depth": 30},
            {"route": "velocity -> wt1.level", "depth": 20},
        ]})
    }

    #[test]
    fn the_three_forms_parse_and_typos_do_not() {
        assert_eq!(
            parse("wt x2").unwrap().what,
            What::Kind {
                kind: "wt".into(),
                min: 2
            }
        );
        assert_eq!(
            parse("ott").unwrap().what,
            What::Kind {
                kind: "ott".into(),
                min: 1
            }
        );
        assert_eq!(
            parse("any -> formant.*").unwrap().what,
            What::Route {
                src: "any".into(),
                dst: "formant".into(),
                param: "*".into()
            }
        );
        assert_eq!(
            parse("wt.unison >= 3").unwrap().what,
            What::Param {
                kind: "wt".into(),
                param: "unison".into(),
                op: Op::AtLeast,
                value: "3".into()
            }
        );
        assert_eq!(
            parse("lfo.sync = On").unwrap().what,
            What::Param {
                kind: "lfo".into(),
                param: "sync".into(),
                op: Op::Equals,
                value: "on".into()
            }
        );
        for bad in [
            "",
            "wt x0",
            "wt twice",
            "wt -> pm",
            "WT -> wt.pm",
            "wt.unison >= many",
            "wt.unison >=",
            "unison = 3",
            "wt extra words",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` should not parse");
        }
    }

    #[test]
    fn kinds_count_enabled_modules_only() {
        let view = View::from_get(&fm()).unwrap();
        let none = BTreeMap::new();
        assert_eq!(
            unmet(&view, &none, &reqs(&["wt x2", "adsr"])),
            Vec::<String>::new()
        );
        assert_eq!(unmet(&view, &none, &reqs(&["wt x3"])), ["wt x3"]);
        // lfo1 is bypassed
        assert_eq!(unmet(&view, &none, &reqs(&["lfo"])), ["lfo"]);
    }

    #[test]
    fn routes_match_kinds_params_and_skip_bypassed_sources() {
        let view = View::from_get(&fm()).unwrap();
        let none = BTreeMap::new();
        assert_eq!(
            unmet(
                &view,
                &none,
                &reqs(&["wt -> wt.pm", "any -> wt.*", "velocity -> wt.level"])
            ),
            Vec::<String>::new()
        );
        // the lfo is bypassed, so its route does not count
        assert_eq!(
            unmet(
                &view,
                &none,
                &reqs(&["lfo -> wt.pm_index", "wt -> wt.am", "adsr -> wt.pm"])
            ),
            ["lfo -> wt.pm_index", "wt -> wt.am", "adsr -> wt.pm"]
        );
    }

    #[test]
    fn parameters_compare_as_printed_by_enth_get() {
        let view = View::from_get(&fm()).unwrap();
        let mut values = BTreeMap::new();
        values.insert("wt1.unison".to_owned(), "5".to_owned());
        values.insert("wt2.unison".to_owned(), "1".to_owned());
        values.insert("wt1.table".to_owned(), "fm".to_owned());
        let r = reqs(&[
            "wt.unison >= 5",
            "wt.unison >= 6",
            "wt.table = fm",
            "wt.table = basic",
        ]);
        assert_eq!(
            view.addresses(&r),
            ["wt1.unison", "wt2.unison", "wt1.table", "wt2.table"]
        );
        assert_eq!(
            unmet(&view, &values, &r),
            ["wt.unison >= 6", "wt.table = basic"]
        );
        assert_eq!(number("+30ct"), Some(30.0));
        assert_eq!(number("-6dB"), Some(-6.0));
        assert_eq!(number("66%"), Some(66.0));
        assert_eq!(normal("TRUE"), "on");
    }

    #[test]
    fn a_reply_without_modules_is_an_error() {
        assert!(View::from_get(&json!({"routes": []})).is_err());
    }
}
