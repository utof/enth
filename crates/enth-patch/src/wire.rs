//! The JSON wire form of ops used by the CLI (`set`, `add`, `route`, …) (P1a lane S1b).
//!
//! One format with two strictness levels: **input** may use unit strings and defaults, the **resolved** form
//! (what [`to_json`] writes, and what undo logs and duel messages carry) uses plain numbers and states everything.
//! Every op, inverses included, has a wire form, and `resolve(patch, to_json(patch, ops)) == ops`.
//!
//! ```json
//! [
//!   {"set": "env1.attack", "to": "600ms"},
//!   {"set": "voice.mode", "to": "legato"},
//!   {"add": "svf", "id": "filter2", "at": "voice_fx[1]", "params": {"cutoff": "800Hz"}},
//!   {"remove": "filter2"},
//!   {"move": "filter2", "to": "voice_fx[0]"},
//!   {"route": "lfo1 -> filter1.cutoff", "depth": "+1oct"},
//!   {"route": "env2 -> filter1.cutoff", "depth": 30, "index": 2},
//!   {"unroute": "lfo1 -> filter1.cutoff"},
//!   {"enable": "fx1", "to": false},
//!   {"macro": 1, "to": 0.5, "name": "Brightness"},
//!   {"slot": 7, "to": "filter1.cutoff"},
//!   {"amp_env": "env1"},
//!   {"label": "osc1", "to": "Bass"},
//!   {"meta": {"name": "Soft pluck", "tags": ["pluck"]}}
//! ]
//! ```
//!
//! | Verb | Keys | Becomes |
//! | --- | --- | --- |
//! | `set` | `to` (number, unit string, enum key, bool) | `SetParam`; reaches `voice.*` and `master.*`; **a number for an enum is `bad_value`** |
//! | `add` | kind; `id` (default: lowest free `<prefix><n>`), `at` (`section[i]`, default: end of the kind's section), `params` (sparse, like a file), `enabled` (default true), `label` | `AddModule` |
//! | `remove` | module id | `RemoveModule` |
//! | `move` | `to` (`section[i]`) | `Move` |
//! | `route` | `"src -> dst"`, `depth` (percent number, or `30%`, `+1oct`, `+12st` converted at the destination's **current** value), `index` | `SetDepth` if the route exists (then `index` is an error), else `InsertRoute` at `index` (default: append) |
//! | `unroute` | `"src -> dst"` | `RemoveRoute` |
//! | `enable` | `to` (bool, default true) | `SetEnabled` |
//! | `macro` | 1-based number; `to` (0–1), `name` (text or null); at least one | `SetMacro` and/or `SetMacroName` |
//! | `slot` | 1-based number; `to` (address or null to clear) | `SetSlot` |
//! | `amp_env` | module id | `SetAmpEnv` |
//! | `label` | module id; `to` (text or null) | `SetLabel` |
//! | `meta` | any of `name`, `author`, `description`, `tags`; the others keep their value | `SetMeta` |
//!
//! Input is an array of ops (a single object is one op). An object is read by finding its verb key in the order
//! above (`add` comes before `label`, so `{"add": "osc", "label": "Bass"}` is an `add`); a key the verb does not use
//! is `bad_op`, and no verb at all is `unknown_op` listing the verbs. Ops are resolved **in order against the patch
//! as the earlier ops leave it**, so an op may use a module an earlier `add` made, and the first op that does not
//! apply stops the parse with its path (`ops[2]`, `ops[2].params.cutoff`).
//!
//! Macros and slots are 1-based in text and 0-based in [`Op`]s.

use serde_json::{Map, Number, Value};

use crate::addr::{AddrError, MacroIndex, ModuleId, ParamAddr, Position, RouteAddr, SlotIndex};
use crate::catalogue::{MACRO_VALUE, ModuleKind};
use crate::error::{Invalid, PatchError, Violation};
use crate::file::{Params, RawValue, module_values, resolve_value};
use crate::model::{ModuleInstance, Patch, PatchMeta, Route};
use crate::ops::{Op, apply, canonical_value, resolve as resolve_addr};

/// The verbs, in the order they are looked for.
pub const VERBS: &[&str] = &[
    "set", "add", "remove", "move", "route", "unroute", "enable", "macro", "slot", "amp_env",
    "label", "meta",
];

type Failure = (String, PatchError);

fn fail<T>(path: &str, e: impl Into<PatchError>) -> Result<T, Failure> {
    Err((path.to_owned(), e.into()))
}

fn bad_op<T>(verb: &'static str, path: &str, message: impl Into<String>) -> Result<T, Failure> {
    fail(
        path,
        PatchError::BadOp {
            verb,
            message: message.into(),
        },
    )
}

// ---------------------------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------------------------

/// Resolve ops JSON (an array, or one object) against a patch into [`Op`]s. See the [module docs](self).
pub fn resolve(patch: &Patch, ops: &Value) -> Result<Vec<Op>, Invalid> {
    let single;
    let list: &[Value] = match ops {
        Value::Array(a) => a,
        Value::Object(_) => {
            single = [ops.clone()];
            &single
        }
        other => {
            return Err(Invalid::one(
                "ops",
                PatchError::BadFile {
                    message: format!(
                        "ops must be an array of objects, found {}",
                        type_name(other)
                    ),
                },
            ));
        }
    };
    let mut scratch = patch.clone();
    let mut out = Vec::new();
    for (i, elem) in list.iter().enumerate() {
        let at = format!("ops[{i}]");
        let parsed = parse_op(&scratch, elem).map_err(|(sub, e)| {
            let path = if sub.is_empty() {
                at.clone()
            } else {
                format!("{at}.{sub}")
            };
            Invalid::one(path, e)
        })?;
        for op in &parsed {
            apply(&mut scratch, op).map_err(|e| Invalid::one(at.clone(), e))?;
        }
        out.extend(parsed);
    }
    Ok(out)
}

/// [`resolve`] for JSON text.
pub fn resolve_str(patch: &Patch, json: &str) -> Result<Vec<Op>, Invalid> {
    let v: Value = serde_json::from_str(json).map_err(|e| {
        Invalid::one(
            "ops",
            PatchError::BadJson {
                message: e.to_string(),
            },
        )
    })?;
    resolve(patch, &v)
}

/// The `--set k=v,k=v` sugar: `set` ops from `address=value` pairs separated by commas (values are text, parsed
/// against their parameter: `filter1.cutoff=1.2kHz,voice.mode=legato`).
pub fn set_ops(patch: &Patch, pairs: &str) -> Result<Vec<Op>, Invalid> {
    let mut list = Vec::new();
    for (i, pair) in pairs.split(',').enumerate() {
        let pair = pair.trim();
        let Some((k, v)) = pair.split_once('=') else {
            return Err(Invalid::one(
                format!("set[{i}]"),
                PatchError::BadOp {
                    verb: "set",
                    message: format!("'{pair}' is not address=value"),
                },
            ));
        };
        let mut m = Map::new();
        m.insert("set".to_owned(), Value::from(k.trim()));
        m.insert("to".to_owned(), Value::from(v.trim()));
        list.push(Value::Object(m));
    }
    resolve(patch, &Value::Array(list)).map_err(|mut inv| {
        // The paths say ops[i]; for the sugar they are the i-th pair.
        for v in &mut inv.0 {
            v.path = v.path.replace("ops[", "set[");
        }
        inv
    })
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a bool",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// An optional text field: absent, `null`, or text.
#[derive(Clone, Copy)]
enum Field<'a> {
    Missing,
    Null,
    Text(&'a str),
}

impl<'a> Field<'a> {
    /// The text, if any (`null` and absent both mean none).
    fn text(self) -> Option<&'a str> {
        match self {
            Field::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// Read the keys of an op object: the verb's own key first, then only the allowed extras.
struct Keys<'a> {
    verb: &'static str,
    map: &'a Map<String, Value>,
}

impl<'a> Keys<'a> {
    fn new(
        verb: &'static str,
        map: &'a Map<String, Value>,
        allowed: &[&str],
    ) -> Result<Self, Failure> {
        for k in map.keys() {
            if k != verb && !allowed.contains(&k.as_str()) {
                return bad_op(verb, k, format!("unexpected key '{k}'"));
            }
        }
        Ok(Keys { verb, map })
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }

    fn text(&self, key: &str) -> Result<&'a str, Failure> {
        match self.get(key) {
            Some(Value::String(s)) => Ok(s),
            Some(other) => bad_op(
                self.verb,
                key,
                format!("'{key}' must be a string, found {}", type_name(other)),
            ),
            None => bad_op(self.verb, key, format!("missing '{key}'")),
        }
    }

    fn opt_text(&self, key: &str) -> Result<Field<'a>, Failure> {
        match self.get(key) {
            None => Ok(Field::Missing),
            Some(Value::Null) => Ok(Field::Null),
            Some(Value::String(s)) => Ok(Field::Text(s)),
            Some(other) => bad_op(
                self.verb,
                key,
                format!(
                    "'{key}' must be a string or null, found {}",
                    type_name(other)
                ),
            ),
        }
    }

    fn flag(&self, key: &str) -> Result<Option<bool>, Failure> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                "on" | "true" => Ok(Some(true)),
                "off" | "false" => Ok(Some(false)),
                _ => bad_op(self.verb, key, format!("'{key}' must be true or false")),
            },
            Some(other) => bad_op(
                self.verb,
                key,
                format!("'{key}' must be true or false, found {}", type_name(other)),
            ),
        }
    }

    fn count(&self, key: &str) -> Result<usize, Failure> {
        match self.get(key).and_then(Value::as_u64) {
            Some(n) => Ok(n as usize),
            None => bad_op(
                self.verb,
                key,
                format!("'{key}' must be a non-negative whole number"),
            ),
        }
    }
}

fn raw_from_json(v: &Value) -> RawValue {
    match v {
        Value::Number(n) => RawValue::Num(n.as_f64().map_or(f32::NAN, |x| x as f32)),
        Value::String(s) => RawValue::Text(s.clone()),
        Value::Bool(b) => RawValue::Bool(*b),
        other => RawValue::Other(other.clone()),
    }
}

fn module_id(keys: &Keys<'_>, key: &str) -> Result<ModuleId, Failure> {
    let text = keys.text(key)?;
    ModuleId::new(text).map_err(|e| (key.to_owned(), e.into()))
}

fn existing(p: &Patch, id: &ModuleId, path: &str) -> Result<(), Failure> {
    crate::ops::locate(p, id)
        .map(|_| ())
        .map_err(|e| (path.to_owned(), e))
}

fn parse_op(p: &Patch, elem: &Value) -> Result<Vec<Op>, Failure> {
    let Value::Object(map) = elem else {
        return bad_op(
            "op",
            "",
            format!("an op must be an object, found {}", type_name(elem)),
        );
    };
    let Some(&verb) = VERBS.iter().find(|v| map.contains_key(**v)) else {
        return fail(
            "",
            PatchError::UnknownOp {
                found: map.keys().cloned().collect(),
            },
        );
    };
    match verb {
        "set" => parse_set(p, map),
        "add" => parse_add(p, map),
        "remove" => {
            let k = Keys::new("remove", map, &[])?;
            let id = module_id(&k, "remove")?;
            existing(p, &id, "remove")?;
            Ok(vec![Op::RemoveModule { id }])
        }
        "move" => {
            let k = Keys::new("move", map, &["to"])?;
            let id = module_id(&k, "move")?;
            let to = k
                .text("to")?
                .parse::<Position>()
                .map_err(|e| ("to".to_owned(), e.into()))?;
            Ok(vec![Op::Move { id, to }])
        }
        "route" => parse_route(p, map),
        "unroute" => {
            let k = Keys::new("unroute", map, &[])?;
            let route = k
                .text("unroute")?
                .parse::<RouteAddr>()
                .map_err(|e| ("unroute".to_owned(), e.into()))?;
            Ok(vec![Op::RemoveRoute { route }])
        }
        "enable" => {
            let k = Keys::new("enable", map, &["to"])?;
            let id = module_id(&k, "enable")?;
            let enabled = k.flag("to")?.unwrap_or(true);
            Ok(vec![Op::SetEnabled { id, enabled }])
        }
        "macro" => parse_macro(map),
        "slot" => {
            let k = Keys::new("slot", map, &["to"])?;
            let n = k.count("slot")?;
            let slot = SlotIndex::from_one_based(n).ok_or_else(|| {
                (
                    "slot".to_owned(),
                    AddrError::BadIndex {
                        text: n.to_string(),
                        max: crate::model::SLOTS,
                    }
                    .into(),
                )
            })?;
            let addr = match k.opt_text("to")?.text() {
                Some(t) => Some(
                    t.parse::<ParamAddr>()
                        .map_err(|e| ("to".to_owned(), e.into()))?,
                ),
                None => None,
            };
            Ok(vec![Op::SetSlot { slot, addr }])
        }
        "amp_env" => {
            let k = Keys::new("amp_env", map, &[])?;
            let id = module_id(&k, "amp_env")?;
            Ok(vec![Op::SetAmpEnv { id }])
        }
        "label" => {
            let k = Keys::new("label", map, &["to"])?;
            let id = module_id(&k, "label")?;
            let label = k.opt_text("to")?.text().map(str::to_owned);
            Ok(vec![Op::SetLabel { id, label }])
        }
        "meta" => parse_meta(p, map),
        _ => fail(
            "",
            PatchError::UnknownOp {
                found: map.keys().cloned().collect(),
            },
        ),
    }
}

fn parse_set(p: &Patch, map: &Map<String, Value>) -> Result<Vec<Op>, Failure> {
    let k = Keys::new("set", map, &["to"])?;
    let addr = k
        .text("set")?
        .parse::<ParamAddr>()
        .map_err(|e| ("set".to_owned(), e.into()))?;
    let Some(to) = k.get("to") else {
        return bad_op("set", "to", "missing 'to'");
    };
    let (_, spec) = resolve_addr(p, &addr).map_err(|e| ("set".to_owned(), e))?;
    let value = resolve_value(spec, &raw_from_json(to)).map_err(|e| ("to".to_owned(), e))?;
    Ok(vec![Op::SetParam { addr, value }])
}

fn parse_add(p: &Patch, map: &Map<String, Value>) -> Result<Vec<Op>, Failure> {
    let k = Keys::new("add", map, &["id", "at", "params", "enabled", "label"])?;
    let kind = k
        .text("add")?
        .parse::<ModuleKind>()
        .map_err(|e| ("add".to_owned(), e.into()))?;
    let id = match k.get("id") {
        None => p.free_id(kind),
        Some(_) => module_id(&k, "id")?,
    };
    let at = match k.get("at") {
        None => Position {
            section: kind.section(),
            index: p.section(kind.section()).len(),
        },
        Some(_) => k
            .text("at")?
            .parse::<Position>()
            .map_err(|e| ("at".to_owned(), e.into()))?,
    };
    let params = match k.get("params") {
        None => Params::default(),
        Some(Value::Object(m)) => Params(
            m.iter()
                .map(|(key, v)| (key.clone(), raw_from_json(v)))
                .collect(),
        ),
        Some(other) => {
            return bad_op(
                "add",
                "params",
                format!("'params' must be an object, found {}", type_name(other)),
            );
        }
    };
    let mut errs: Vec<Violation> = Vec::new();
    let values = module_values(kind.spec(), id.as_str(), &params, "params", &mut errs);
    if let Some(first) = errs.into_iter().next() {
        return Err((first.path, first.error));
    }
    Ok(vec![Op::AddModule {
        at,
        module: ModuleInstance {
            id,
            kind,
            label: k.opt_text("label")?.text().map(str::to_owned),
            enabled: k.flag("enabled")?.unwrap_or(true),
            values,
        },
    }])
}

fn parse_route(p: &Patch, map: &Map<String, Value>) -> Result<Vec<Op>, Failure> {
    let k = Keys::new("route", map, &["depth", "index"])?;
    let addr = k
        .text("route")?
        .parse::<RouteAddr>()
        .map_err(|e| ("route".to_owned(), e.into()))?;
    let (_, spec) = resolve_addr(p, &addr.dst).map_err(|e| ("route".to_owned(), e))?;
    let Some(depth) = k.get("depth") else {
        return bad_op("route", "depth", "missing 'depth'");
    };
    let depth = match depth {
        Value::Number(_) => canonical_value(
            &enth_params::DEPTH,
            match raw_from_json(depth) {
                RawValue::Num(x) => x,
                _ => f32::NAN,
            },
        )
        .map_err(|e| ("depth".to_owned(), e.into()))?,
        Value::String(t) => {
            let base = p.value(&addr.dst).unwrap_or(spec.default);
            let d = spec
                .depth_from_text(base, t)
                .map_err(|e| ("depth".to_owned(), e.into()))?;
            canonical_value(&enth_params::DEPTH, d).map_err(|e| ("depth".to_owned(), e.into()))?
        }
        other => {
            return bad_op(
                "route",
                "depth",
                format!(
                    "'depth' must be a number or text like \"30%\", found {}",
                    type_name(other)
                ),
            );
        }
    };
    let exists = p
        .routes
        .iter()
        .any(|r| r.src == addr.src && r.dst == addr.dst);
    if exists {
        if k.get("index").is_some() {
            return fail(
                "index",
                PatchError::DuplicateRoute {
                    route: addr.to_string(),
                },
            );
        }
        return Ok(vec![Op::SetDepth { route: addr, depth }]);
    }
    let index = match k.get("index") {
        None => p.routes.len(),
        Some(_) => k.count("index")?,
    };
    Ok(vec![Op::InsertRoute {
        index,
        route: Route {
            src: addr.src,
            dst: addr.dst,
            depth,
        },
    }])
}

fn parse_macro(map: &Map<String, Value>) -> Result<Vec<Op>, Failure> {
    let k = Keys::new("macro", map, &["to", "name"])?;
    let n = k.count("macro")?;
    let index = MacroIndex::from_one_based(n).ok_or_else(|| {
        (
            "macro".to_owned(),
            AddrError::BadIndex {
                text: n.to_string(),
                max: crate::model::MACROS,
            }
            .into(),
        )
    })?;
    if k.get("to").is_none() && k.get("name").is_none() {
        return bad_op(
            "macro",
            "",
            "give 'to' (the knob position), 'name', or both",
        );
    }
    let mut ops = Vec::new();
    if let Some(to) = k.get("to") {
        let value =
            resolve_value(&MACRO_VALUE, &raw_from_json(to)).map_err(|e| ("to".to_owned(), e))?;
        ops.push(Op::SetMacro { index, value });
    }
    let name = k.opt_text("name")?;
    if !matches!(name, Field::Missing) {
        ops.push(Op::SetMacroName {
            index,
            name: name.text().map(str::to_owned),
        });
    }
    Ok(ops)
}

fn parse_meta(p: &Patch, map: &Map<String, Value>) -> Result<Vec<Op>, Failure> {
    let k = Keys::new("meta", map, &[])?;
    let Some(Value::Object(fields)) = k.get("meta") else {
        return bad_op("meta", "meta", "'meta' must be an object");
    };
    let mut meta = p.meta.clone();
    for (key, v) in fields {
        let path = format!("meta.{key}");
        let text = |v: &Value| match v {
            Value::String(s) => Ok(s.clone()),
            other => bad_op::<String>(
                "meta",
                &path,
                format!("'{key}' must be a string, found {}", type_name(other)),
            ),
        };
        match key.as_str() {
            "name" => meta.name = text(v)?,
            "author" => meta.author = text(v)?,
            "description" => meta.description = text(v)?,
            "tags" => match v {
                Value::Array(a) => {
                    meta.tags = a.iter().map(text).collect::<Result<_, _>>()?;
                }
                other => {
                    return bad_op(
                        "meta",
                        &path,
                        format!(
                            "'tags' must be an array of strings, found {}",
                            type_name(other)
                        ),
                    );
                }
            },
            _ => {
                return bad_op(
                    "meta",
                    &path,
                    format!("unknown meta field '{key}' (name, author, description, tags)"),
                );
            }
        }
    }
    Ok(vec![Op::SetMeta { meta }])
}

// ---------------------------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------------------------

/// An `f32` as a JSON number with its shortest digits (`0.1`, not `0.10000000149011612`): the shortest `f32` text,
/// read as an `f64`. Reading it back as `f32` gives the same bits. (`serde_json::to_value(f32)` would widen.)
fn num(x: f32) -> Value {
    format!("{x}")
        .parse::<f64>()
        .ok()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn value_json(spec: Option<&enth_params::ParamSpec>, v: f32) -> Value {
    match spec {
        Some(s) => match s.kind {
            enth_params::Kind::Enum(_) => s
                .enum_item(v)
                .map_or_else(|| num(v), |item| Value::from(item.key)),
            enth_params::Kind::Toggle if v == 0.0 || v == 1.0 => Value::Bool(v == 1.0),
            _ => num(v),
        },
        None => num(v),
    }
}

fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn opt(s: Option<&String>) -> Value {
    s.map_or(Value::Null, |s| Value::from(s.as_str()))
}

fn meta_json(m: &PatchMeta) -> Value {
    obj([
        ("name", Value::from(m.name.as_str())),
        ("author", Value::from(m.author.as_str())),
        ("description", Value::from(m.description.as_str())),
        (
            "tags",
            Value::Array(m.tags.iter().map(|t| Value::from(t.as_str())).collect()),
        ),
    ])
}

/// The wire form of ops that are about to be applied to `patch`, in order: `resolve(patch, &to_json(patch, ops))`
/// gives `ops` back (for ops whose values are stored canonically, like every inverse). The patch is only read, to
/// find the parameter specs of enum and toggle values; ops that name a module an earlier op of the list adds are
/// handled.
pub fn to_json(patch: &Patch, ops: &[Op]) -> Value {
    let mut scratch = patch.clone();
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        out.push(op_json(&scratch, op));
        let _ = apply(&mut scratch, op);
    }
    Value::Array(out)
}

fn op_json(p: &Patch, op: &Op) -> Value {
    match op {
        Op::SetParam { addr, value } => {
            let spec = resolve_addr(p, addr).ok().map(|(_, s)| s);
            obj([
                ("set", Value::from(addr.to_string())),
                ("to", value_json(spec, *value)),
            ])
        }
        Op::AddModule { at, module } => {
            let params: Map<String, Value> = module
                .kind
                .params()
                .iter()
                .zip(&module.values)
                .map(|(s, &v)| (s.key.to_owned(), value_json(Some(s), v)))
                .collect();
            let mut m = Map::new();
            m.insert("add".to_owned(), Value::from(module.kind.key()));
            m.insert("id".to_owned(), Value::from(module.id.as_str()));
            m.insert("at".to_owned(), Value::from(at.to_string()));
            m.insert("params".to_owned(), Value::Object(params));
            if !module.enabled {
                m.insert("enabled".to_owned(), Value::Bool(false));
            }
            if let Some(label) = &module.label {
                m.insert("label".to_owned(), Value::from(label.as_str()));
            }
            Value::Object(m)
        }
        Op::RemoveModule { id } => obj([("remove", Value::from(id.as_str()))]),
        Op::Move { id, to } => obj([
            ("move", Value::from(id.as_str())),
            ("to", Value::from(to.to_string())),
        ]),
        Op::InsertRoute { index, route } => obj([
            (
                "route",
                Value::from(format!("{} -> {}", route.src, route.dst)),
            ),
            ("depth", num(route.depth)),
            ("index", Value::from(*index)),
        ]),
        Op::RemoveRoute { route } => obj([("unroute", Value::from(route.to_string()))]),
        Op::SetDepth { route, depth } => obj([
            ("route", Value::from(route.to_string())),
            ("depth", num(*depth)),
        ]),
        Op::SetEnabled { id, enabled } => obj([
            ("enable", Value::from(id.as_str())),
            ("to", Value::Bool(*enabled)),
        ]),
        Op::SetAmpEnv { id } => obj([("amp_env", Value::from(id.as_str()))]),
        Op::SetMacro { index, value } => obj([
            ("macro", Value::from(index.one_based())),
            ("to", num(*value)),
        ]),
        Op::SetSlot { slot, addr } => obj([
            ("slot", Value::from(slot.one_based())),
            (
                "to",
                addr.as_ref()
                    .map_or(Value::Null, |a| Value::from(a.to_string())),
            ),
        ]),
        Op::SetLabel { id, label } => obj([
            ("label", Value::from(id.as_str())),
            ("to", opt(label.as_ref())),
        ]),
        Op::SetMeta { meta } => obj([("meta", meta_json(meta))]),
        Op::SetMacroName { index, name } => obj([
            ("macro", Value::from(index.one_based())),
            ("name", opt(name.as_ref())),
        ]),
    }
}
