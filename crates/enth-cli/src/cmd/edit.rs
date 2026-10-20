//! The patch commands: `new`, `apply`, `get`, `diff`.

use std::collections::BTreeMap;
use std::path::Path;

use enth_params::{Kind, ParamSpec};
use enth_patch::addr::Target;
use enth_patch::catalogue::Builtin;
use enth_patch::{ModuleId, ParamAddr, Patch, PatchError, file};

use super::common::{self, readings, resolve_edits, show};
use super::{Reply, Res};
use crate::cli::Edits;
use crate::error::{self, CliError, Ctx, nearest};
use crate::perf::{Perf, PerfArgs};
use crate::schema::{
    ApplyOut, DiffOut, DiffReading, GetOut, MetaOut, ModuleOut, NewOut, Reading, RouteOut,
    TemplateOut, TemplatesOut, ids,
};
use crate::{patchio, templates};

pub fn new(template: Option<&str>, out: Option<&Path>, list: bool) -> Res {
    if list {
        let out = TemplatesOut {
            schema: ids::TEMPLATES.to_owned(),
            templates: templates::TEMPLATES
                .iter()
                .map(|t| TemplateOut {
                    name: t.name.to_owned(),
                    summary: t.summary.to_owned(),
                    modules: templates::module_ids(&t.patch()),
                })
                .collect(),
        };
        return Ok(Reply::new(&out).columns(&["name", "summary"]));
    }
    let Some(name) = template else {
        return Err(error::usage(
            "usage",
            "new needs --template <name>",
            "e.g. `enth new --template pluck_1osc --out p.patch.json`; `enth new --list` lists the templates",
        )
        .known(templates::names()));
    };
    let t = templates::find(name).ok_or_else(|| {
        error::invalid(
            "unknown_template",
            format!("no template '{name}'"),
            "`enth new --list` lists the templates",
        )
        .suggest(nearest(name, templates::TEMPLATES.iter().map(|t| t.name)))
        .known(templates::names())
    })?;
    let Some(out) = out else {
        return Err(error::usage(
            "usage",
            "new needs --out <path>",
            format!("e.g. `enth new --template {} --out p.patch.json`", t.name),
        ));
    };
    let patch = t.patch();
    patchio::write(out, &patch)?;
    let path = patchio::show(out);
    let res = NewOut {
        schema: ids::NEW.to_owned(),
        wrote: path.clone(),
        template: t.name.to_owned(),
        engine: file::ENGINE_VERSION.to_owned(),
        modules: templates::module_ids(&patch),
        perf: Perf::default_perf().to_out(),
        next: format!(
            "enth step {path} --set <address>=<value>,... --expect '<list>' --perf '<perf>'"
        ),
    };
    Ok(Reply::new(&res))
}

pub fn apply(path: &Path, edits: &Edits, out: Option<&Path>, dry_run: bool, stdin: &str) -> Res {
    if edits.ops.is_none() && edits.set.is_empty() {
        return Err(error::usage(
            "nothing_to_apply",
            "apply needs --set or --ops",
            "e.g. `enth apply p.patch.json --set filter1.cutoff=1.2kHz,env1.attack=40ms`",
        ));
    }
    let (patch, notes) = patchio::load(path)?;
    let ed = resolve_edits(&patch, edits, stdin)?;
    let target = out.unwrap_or(path);
    let wrote = if dry_run {
        None
    } else {
        patchio::write(target, &ed.after)?;
        Some(patchio::show(target))
    };
    let res = ApplyOut {
        schema: ids::APPLY.to_owned(),
        wrote,
        dry_run,
        applied: common::changes(&patch, &ed.ops),
        ops: ed.ops_json(&patch),
        inverse: ed.inverse_json(),
    };
    let mut reply = Reply::new(&res).columns(&["op", "target", "from", "to"]);
    for n in notes {
        reply = reply.note(&n);
    }
    Ok(reply)
}

/// A parameter value for `get`: a unit string, or with `plain` a number (enum keys stay text, toggles booleans).
fn value_of(spec: &ParamSpec, v: f32, plain: bool) -> Reading {
    match spec.kind {
        Kind::Enum(_) => Reading::Text(show(Some(spec), v)),
        Kind::Toggle if plain => Reading::Bool(v >= 0.5),
        _ if plain => Reading::Num(f64::from(v)),
        _ => Reading::Text(show(Some(spec), v)),
    }
}

fn module_list(p: &Patch) -> Vec<ModuleOut> {
    p.modules()
        .map(|(pos, m)| ModuleOut {
            id: m.id.to_string(),
            kind: m.kind.key().to_owned(),
            section: pos.section.key().to_owned(),
            enabled: m.enabled,
        })
        .collect()
}

fn route_list(p: &Patch) -> Vec<RouteOut> {
    p.routes
        .iter()
        .map(|r| RouteOut {
            route: format!("{} -> {}", r.src, r.dst),
            depth: f64::from(r.depth),
        })
        .collect()
}

fn meta_of(p: &Patch) -> MetaOut {
    MetaOut {
        name: p.meta.name.clone(),
        author: p.meta.author.clone(),
        description: p.meta.description.clone(),
        tags: p.meta.tags.clone(),
    }
}

fn insert_builtin(
    values: &mut BTreeMap<String, Reading>,
    p: &Patch,
    b: Builtin,
    plain: bool,
    only_changed: bool,
) {
    let vals: &[f32] = match b {
        Builtin::Voice => &p.voice.values,
        Builtin::Master => &p.master,
    };
    for (spec, v) in b.params().iter().zip(vals) {
        if !only_changed || *v != spec.default {
            values.insert(
                format!("{}.{}", b.key(), spec.key),
                value_of(spec, *v, plain),
            );
        }
    }
    if b == Builtin::Voice {
        values.insert(
            "voice.amp_env".to_owned(),
            Reading::Text(p.voice.amp_env.to_string()),
        );
    }
}

fn unknown_module(p: &Patch, id: &str, written: &str) -> CliError {
    let existing: Vec<String> = p.modules().map(|(_, m)| m.id.to_string()).collect();
    error::from_patch_error(
        written,
        &PatchError::UnknownModule {
            id: id.to_owned(),
            existing,
        },
        Ctx {
            patch: Some(p),
            addr: Some(written),
            ..Ctx::default()
        },
    )
}

fn get_addr(
    p: &Patch,
    text: &str,
    plain: bool,
    values: &mut BTreeMap<String, Reading>,
) -> Result<(), CliError> {
    let ctx = Ctx {
        patch: Some(p),
        addr: Some(text),
        ..Ctx::default()
    };
    if text == "voice.amp_env" {
        values.insert(text.to_owned(), Reading::Text(p.voice.amp_env.to_string()));
        return Ok(());
    }
    let addr: ParamAddr = text
        .parse()
        .map_err(|e| error::from_patch_error(text, &PatchError::Addr(e), ctx))?;
    if let Some(spec) = p.spec(&addr) {
        let v = p.value(&addr).unwrap_or(spec.default);
        values.insert(addr.to_string(), value_of(spec, v, plain));
        return Ok(());
    }
    match addr.target() {
        Target::Module(id) => match p.find(id) {
            None => Err(unknown_module(p, id.as_str(), text)),
            Some((_, m)) => Err(error::from_patch_error(
                text,
                &PatchError::UnknownParam {
                    addr: text.to_owned(),
                    owner: m.kind.key(),
                    keys: m.kind.params().iter().map(|s| s.key).collect(),
                },
                ctx,
            )),
        },
        _ => Err(error::invalid(
            "unknown_param",
            format!("'{text}' names no parameter"),
            "`enth params voice` and `enth params master` list the built-in keys",
        )
        .at(text)),
    }
}

pub fn get(path: &Path, addrs: &[String], plain: bool) -> Res {
    let (p, _) = patchio::load(path)?;
    let mut values: BTreeMap<String, Reading> = BTreeMap::new();
    let (mut modules, mut routes, mut meta) = (None, None, None);
    if addrs.is_empty() {
        modules = Some(module_list(&p));
        routes = Some(route_list(&p));
        for (_, m) in p.modules() {
            for (spec, v) in m.kind.params().iter().zip(&m.values) {
                if *v != spec.default {
                    values.insert(format!("{}.{}", m.id, spec.key), value_of(spec, *v, plain));
                }
            }
        }
        insert_builtin(&mut values, &p, Builtin::Voice, plain, true);
        insert_builtin(&mut values, &p, Builtin::Master, plain, true);
    }
    for a in addrs {
        let a = a.trim();
        match a {
            "routes" => routes = Some(route_list(&p)),
            "modules" => modules = Some(module_list(&p)),
            "meta" => meta = Some(meta_of(&p)),
            "voice" => insert_builtin(&mut values, &p, Builtin::Voice, plain, false),
            "master" => insert_builtin(&mut values, &p, Builtin::Master, plain, false),
            _ if !a.contains('.') => {
                let found = ModuleId::new(a).ok().and_then(|id| p.find(&id));
                let Some((_, m)) = found else {
                    return Err(unknown_module(&p, a, a));
                };
                for (spec, v) in m.kind.params().iter().zip(&m.values) {
                    values.insert(format!("{}.{}", m.id, spec.key), value_of(spec, *v, plain));
                }
            }
            _ => get_addr(&p, a, plain, &mut values)?,
        }
    }
    let res = GetOut {
        schema: ids::GET.to_owned(),
        source: patchio::show(path),
        values,
        modules,
        routes,
        meta,
    };
    Ok(Reply::new(&res).columns(&["id", "kind", "route", "depth"]))
}

// ---------------------------------------------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------------------------------------------

pub fn diff(a_path: &Path, b_path: &Path, describe: bool, perf: &PerfArgs, stdin: &str) -> Res {
    let (a, _) = patchio::load(a_path)?;
    let (b, _) = patchio::load(b_path)?;
    let mut b_norm = b.clone();
    b_norm.engine.clone_from(&a.engine);
    let same = a == b_norm;
    let ops = enth_patch::ops_between(&a, &b_norm);
    let changes = ops
        .as_ref()
        .map(|o| common::changes(&a, o))
        .unwrap_or_default();
    let ops_json = ops.map(|o| common::as_array(enth_patch::wire::to_json(&a, &o)));
    let describe_map = if describe {
        let perf = Perf::from_args(perf, stdin)?.unwrap_or_else(Perf::default_perf);
        let ev = crate::eval::Evaluator::new(
            &perf,
            enth_render::DEFAULT_SEED,
            enth_analysis::descriptors::Want::ALL,
        );
        let mut res = ev
            .describe_jobs(&[(&a, ev.seed()), (&b, ev.seed())])
            .into_iter();
        let da = res.next().unwrap_or_else(|| unreachable!("two jobs"))?;
        let db = res.next().unwrap_or_else(|| unreachable!("two jobs"))?;
        let (ra, rb) = (readings(&da), readings(&db));
        Some(
            ra.into_iter()
                .filter_map(|(k, va)| {
                    let vb = rb.get(&k).cloned().flatten();
                    (va != vb).then_some((k, DiffReading { a: va, b: vb }))
                })
                .collect(),
        )
    } else {
        None
    };
    let res = DiffOut {
        schema: ids::DIFF.to_owned(),
        a: patchio::show(a_path),
        b: patchio::show(b_path),
        same,
        changes,
        ops: ops_json,
        describe: describe_map,
    };
    Ok(Reply::new(&res).columns(&["op", "target", "from", "to"]))
}
