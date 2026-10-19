//! The self-describing commands: `guide`, `schema`, `modules`, `params`.

use std::path::Path;

use enth_params::{Kind, ParamSpec, Scale};
use enth_patch::ModuleId;
use enth_patch::catalogue::{Builtin, CATALOGUE, ModuleKind};

use super::{Reply, Res};
use crate::error::{self, CliError, nearest};
use crate::schema::{
    self, BuiltinOut, GuideOut, ModuleKindOut, ModulesOut, ParamOut, ParamsOut, Reading,
    SchemaListOut, SchemaRef, ids,
};
use crate::{aliases, guide as guide_text, patchio};

pub fn guide(topic: Option<&str>) -> Res {
    let topic = topic.unwrap_or("main");
    let text = guide_text::page(topic).ok_or_else(|| {
        error::invalid(
            "unknown_topic",
            format!("no guide topic '{topic}'"),
            "topics are main, expect, ops, descriptors",
        )
        .suggest(nearest(topic, guide_text::TOPICS.iter().copied()))
        .known(guide_text::TOPICS.iter().map(|t| (*t).to_owned()).collect())
    })?;
    let out = GuideOut {
        schema: ids::GUIDE.to_owned(),
        topic: topic.to_owned(),
        bytes: text.len(),
        text: text.clone(),
        topics: guide_text::TOPICS.iter().map(|t| (*t).to_owned()).collect(),
    };
    Ok(Reply::new(&out).text(text))
}

pub fn schema(name: Option<&str>) -> Res {
    let Some(name) = name else {
        let out = SchemaListOut {
            schema: ids::SCHEMA.to_owned(),
            schemas: schema::ENTRIES
                .iter()
                .map(|e| SchemaRef {
                    name: e.name.to_owned(),
                    id: e.id.to_owned(),
                    describes: e.describes.to_owned(),
                })
                .collect(),
            hint: "`enth schema <name>` prints one JSON Schema (draft 2020-12)".to_owned(),
        };
        return Ok(Reply::new(&out).columns(&["name", "id"]));
    };
    let e = schema::entry(name).ok_or_else(|| {
        let names: Vec<String> = schema::ENTRIES.iter().map(|e| e.name.to_owned()).collect();
        error::invalid(
            "unknown_schema",
            format!("no schema named '{name}'"),
            "`enth schema` lists them",
        )
        .suggest(nearest(name, schema::ENTRIES.iter().map(|e| e.name)))
        .known(names)
    })?;
    let value = crate::out::normalise(schema::json_schema(e));
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_default() + "\n";
    Ok(Reply {
        value,
        code: 0,
        text: Some(pretty),
        stderr: String::new(),
        columns: &[],
    })
}

pub fn modules() -> Reply {
    let out = ModulesOut {
        schema: ids::MODULES.to_owned(),
        modules: CATALOGUE
            .iter()
            .map(|m| ModuleKindOut {
                kind: m.key.to_owned(),
                name: m.name.to_owned(),
                section: m.section.key().to_owned(),
                id_prefix: m.id_prefix.to_owned(),
                summary: m.learn.desc.to_owned(),
                params: m.params.len(),
                tier: m.learn.tier.name().to_owned(),
            })
            .collect(),
        builtins: Builtin::ALL
            .iter()
            .map(|b| BuiltinOut {
                key: b.key().to_owned(),
                summary: builtin_summary(*b).to_owned(),
                params: b.params().len(),
            })
            .collect(),
        hint: "`enth params <kind>` lists a kind's parameters with units and ranges".to_owned(),
    };
    Reply::new(&out).columns(&["kind", "key", "section", "id_prefix", "params", "summary"])
}

fn builtin_summary(b: Builtin) -> &'static str {
    match b {
        Builtin::Voice => {
            "The voice manager and amp stage: mode (poly, mono, legato), polyphony, glide, pan; voice.amp_env names the amplitude envelope."
        }
        Builtin::Master => "The output stage after the effects: level in dB.",
    }
}

/// What `params` describes: a module kind or a built-in.
enum Target {
    Kind(ModuleKind),
    Builtin(Builtin),
}

fn resolve_target(name: &str, patch: Option<&Path>) -> Result<Target, CliError> {
    let n = name.trim();
    match n {
        "voice" => return Ok(Target::Builtin(Builtin::Voice)),
        "master" => return Ok(Target::Builtin(Builtin::Master)),
        _ => {}
    }
    if let Some(path) = patch {
        let (p, _) = patchio::load(path)?;
        if let Ok(id) = ModuleId::new(n)
            && let Some((_, m)) = p.find(&id)
        {
            return Ok(Target::Kind(m.kind));
        }
    }
    if let Some(k) = ModuleKind::from_key(n).or_else(|| aliases::kind_for_name(n)) {
        return Ok(Target::Kind(k));
    }
    let mut pool: Vec<&str> = ModuleKind::ALL.iter().map(|k| k.key()).collect();
    pool.extend(CATALOGUE.iter().map(|m| m.id_prefix));
    pool.extend(["voice", "master"]);
    let mut known: Vec<String> = ModuleKind::ALL.iter().map(|k| k.key().to_owned()).collect();
    known.extend(["voice".to_owned(), "master".to_owned()]);
    Err(error::invalid(
        "unknown_kind",
        format!("'{n}' is not a module kind, a built-in or a module id"),
        "`enth modules` lists the kinds; a module id like filter1 also works",
    )
    .suggest(nearest(n, pool))
    .known(known))
}

pub fn params(name: &str, patch: Option<&Path>) -> Res {
    let out = match resolve_target(name, patch)? {
        Target::Kind(k) => {
            let spec = k.spec();
            ParamsOut {
                schema: ids::PARAMS.to_owned(),
                kind: spec.key.to_owned(),
                name: spec.name.to_owned(),
                section: Some(spec.section.key().to_owned()),
                summary: spec.learn.desc.to_owned(),
                params: spec.params.iter().map(param_out).collect(),
            }
        }
        Target::Builtin(b) => ParamsOut {
            schema: ids::PARAMS.to_owned(),
            kind: b.key().to_owned(),
            name: b.key().to_owned(),
            section: None,
            summary: builtin_summary(b).to_owned(),
            params: b.params().iter().map(param_out).collect(),
        },
    };
    Ok(Reply::new(&out).columns(&["key", "min", "max", "unit", "default", "values", "effect"]))
}

fn param_out(s: &ParamSpec) -> ParamOut {
    let (min, max, scale) = match s.kind {
        Kind::Float { min, max, scale } => (
            Some(f64::from(min)),
            Some(f64::from(max)),
            match scale {
                Scale::Linear => "linear",
                Scale::Log => "log",
                Scale::Power(_) => "power",
            },
        ),
        Kind::Int { min, max } => (Some(f64::from(min)), Some(f64::from(max)), "int"),
        Kind::Enum(_) => (None, None, "enum"),
        Kind::Toggle => (None, None, "toggle"),
    };
    let default = match s.kind {
        Kind::Enum(_) => Reading::Text(
            s.enum_item(s.default)
                .map_or_else(String::new, |i| i.key.to_owned()),
        ),
        Kind::Toggle => Reading::Bool(s.default >= 0.5),
        _ => Reading::Num(f64::from(s.default)),
    };
    let accepts = match s.kind {
        Kind::Enum(items) => items.iter().map(|i| i.key.to_owned()).collect(),
        Kind::Toggle => vec!["on".to_owned(), "off".to_owned()],
        _ => s.unit.accepted().iter().map(|a| (*a).to_owned()).collect(),
    };
    ParamOut {
        key: s.key.to_owned(),
        name: s.name.to_owned(),
        unit: s.unit.suffix().to_owned(),
        min,
        max,
        default,
        scale: scale.to_owned(),
        values: s
            .enum_items()
            .map(|items| items.iter().map(|i| i.key.to_owned()).collect())
            .unwrap_or_default(),
        accepts,
        modulatable: s.flags.modulatable,
        automatable: s.flags.automatable,
        structural: s.flags.structural,
        tier: s.learn.tier.name().to_owned(),
        effect: s.learn.desc.to_owned(),
        moves: s.perceptual.map(str::to_owned),
    }
}
