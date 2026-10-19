//! `PatchFile` → `Patch` for golden cases: a **temporary**, minimal resolver, until lane S1b's file layer lands.
//!
//! A case embeds its patch as a `[source.patch]` table with exactly the shape of a `*.patch.json` file
//! ([`enth_patch::file::PatchFile`], written in TOML), so the case files will not change when S1b's conversion
//! replaces this one: swap [`patch_from_file`] for S1b's `PatchFile → Patch` (which also validates and migrates) and
//! delete this file. It resolves values with the catalogue's own parsers (`ParamSpec::parse` for text such as
//! `"saw"` or `"1.2kHz"`, `ParamSpec::check` for numbers), fills unspecified parameters with their defaults, and
//! leaves validation to the engine's compiler.

use enth_patch::catalogue::{Builtin, MASTER_PARAMS, VOICE_PARAMS};
use enth_patch::file::{ModuleFile, PatchFile, RawValue, SCHEMA};
use enth_patch::{
    Macro, ModSource, ModuleId, ModuleInstance, ModuleKind, ParamAddr, Patch, Route, Section,
    SlotIndex,
};

/// A raw file value checked against its spec: numbers with `ParamSpec::check`, text with `ParamSpec::parse`, booleans
/// as `on`/`off`. (A macro, because enth-golden reaches `ParamSpec` only through the catalogue's tables.)
macro_rules! resolve {
    ($spec:expr, $raw:expr) => {
        match $raw {
            RawValue::Num(x) => $spec.check(*x).map_err(|e| e.to_string()),
            RawValue::Text(t) => $spec.parse(t).map_err(|e| e.to_string()),
            RawValue::Bool(b) => $spec
                .parse(if *b { "on" } else { "off" })
                .map_err(|e| e.to_string()),
        }
    };
}

fn module(m: &ModuleFile) -> Result<ModuleInstance, String> {
    let id = ModuleId::new(&m.id).map_err(|e| e.to_string())?;
    let kind: ModuleKind = m
        .kind
        .parse()
        .map_err(|e: enth_patch::catalogue::UnknownKind| e.to_string())?;
    let mut inst = ModuleInstance::new(id, kind);
    inst.enabled = m.enabled;
    inst.label.clone_from(&m.label);
    for (key, raw) in &m.params.0 {
        let i = kind
            .param_index(key)
            .ok_or_else(|| format!("{}: {kind} has no parameter '{key}'", m.id))?;
        inst.values[i] =
            resolve!(kind.params()[i], raw).map_err(|e| format!("{}.{key}: {e}", m.id))?;
    }
    Ok(inst)
}

/// Convert a patch file (already parsed) into a `Patch`.
///
/// # Errors
/// An unknown kind, id, key, source or address, a value its spec rejects, or a schema other than the current one.
pub fn patch_from_file(f: &PatchFile) -> Result<Patch, String> {
    if f.schema != SCHEMA {
        return Err(format!(
            "schema {} (only {SCHEMA} until S1b's migrations land)",
            f.schema
        ));
    }
    let mut p = Patch::init();
    p.engine.clone_from(&f.engine);
    p.meta = f.meta.clone().into();
    for section in Section::ALL {
        let list = match section {
            Section::Generators => &f.generators,
            Section::VoiceFx => &f.voice_fx,
            Section::Fx => &f.fx,
            Section::Modulators => &f.modulators,
        };
        *p.section_mut(section) = list.iter().map(module).collect::<Result<_, _>>()?;
    }
    let mut amp_env = None;
    for (key, raw) in &f.voice.0 {
        if key == "amp_env" {
            let RawValue::Text(t) = raw else {
                return Err("voice.amp_env is a module id".to_owned());
            };
            amp_env = Some(ModuleId::new(t).map_err(|e| e.to_string())?);
            continue;
        }
        let i = Builtin::Voice
            .param_index(key)
            .ok_or_else(|| format!("voice has no parameter '{key}'"))?;
        p.voice.values[i] =
            resolve!(VOICE_PARAMS[i], raw).map_err(|e| format!("voice.{key}: {e}"))?;
    }
    p.voice.amp_env = match amp_env {
        Some(id) => id,
        None => p
            .modulators
            .iter()
            .find(|m| m.kind == ModuleKind::Adsr)
            .map(|m| m.id.clone())
            .ok_or("no voice.amp_env and no adsr in modulators")?,
    };
    for (key, raw) in &f.master.0 {
        let i = Builtin::Master
            .param_index(key)
            .ok_or_else(|| format!("master has no parameter '{key}'"))?;
        p.master[i] = resolve!(MASTER_PARAMS[i], raw).map_err(|e| format!("master.{key}: {e}"))?;
    }
    for r in &f.routes {
        p.routes.push(Route {
            src: r.src.parse::<ModSource>().map_err(|e| e.to_string())?,
            dst: r.dst.parse::<ParamAddr>().map_err(|e| e.to_string())?,
            depth: r.depth,
        });
    }
    for (i, m) in f.macros.iter().enumerate() {
        let slot = p.macros.get_mut(i).ok_or("more than eight macros")?;
        *slot = Macro {
            value: m.value,
            name: m.name.clone(),
        };
    }
    for (&n, addr) in &f.slots {
        let s = SlotIndex::from_one_based(usize::from(n))
            .ok_or_else(|| format!("slot {n} out of range"))?;
        p.slots[s.get()] = Some(addr.parse::<ParamAddr>().map_err(|e| e.to_string())?);
    }
    Ok(p)
}
