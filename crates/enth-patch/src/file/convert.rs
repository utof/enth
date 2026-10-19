//! `Patch` ⇄ `PatchFile`: dense writing, sparse reading with values parsed against their `ParamSpec`, collected typed
//! errors with paths.

use enth_params::{DEPTH, Expected, Kind, ParamSpec, ValueError};

use super::{
    ENGINE_VERSION, MacroFile, MetaFile, ModuleFile, Params, PatchFile, RawValue, RouteFile, SCHEMA,
};
use crate::addr::{AddrError, ModSource, ModuleId, ParamAddr, SlotIndex};
use crate::catalogue::{
    MACRO_VALUE, MASTER_PARAMS, MasterParam, ModuleKind, ModuleSpec, VOICE_PARAMS, VoiceParam,
};
use crate::error::{Invalid, PatchError, Violation};
use crate::model::{
    MACROS, Macro, ModuleInstance, Patch, PatchMeta, Route, Section, VoiceSettings,
};
use crate::ops::canonical_value;
use crate::validate::violations;

// ---------------------------------------------------------------------------------------------------------------
// Patch -> PatchFile (dense)
// ---------------------------------------------------------------------------------------------------------------

/// How a stored value is written: an enum as its item key, a toggle as a bool, anything else as a number.
fn raw_of(spec: &ParamSpec, v: f32) -> RawValue {
    match spec.kind {
        Kind::Enum(_) => spec
            .enum_item(v)
            .map_or(RawValue::Num(v), |item| RawValue::Text(item.key.to_owned())),
        Kind::Toggle if v == 0.0 || v == 1.0 => RawValue::Bool(v == 1.0),
        _ => RawValue::Num(v),
    }
}

fn dense(specs: &[ParamSpec], values: &[f32]) -> Vec<(String, RawValue)> {
    specs
        .iter()
        .zip(values)
        .map(|(spec, &v)| (spec.key.to_owned(), raw_of(spec, v)))
        .collect()
}

fn module_file(m: &ModuleInstance) -> ModuleFile {
    ModuleFile {
        id: m.id.to_string(),
        kind: m.kind.key().to_owned(),
        enabled: m.enabled,
        label: m.label.clone(),
        params: Params(dense(m.kind.params(), &m.values)),
    }
}

impl Patch {
    /// The file form: dense (every parameter, in spec order, so a later default change cannot alter the file),
    /// enums as item keys, all eight macros, assigned slots only. An empty `engine` is stamped
    /// [`ENGINE_VERSION`].
    pub fn to_file(&self) -> PatchFile {
        let mut voice = dense(&VOICE_PARAMS, &self.voice.values);
        voice.push((
            "amp_env".to_owned(),
            RawValue::Text(self.voice.amp_env.to_string()),
        ));
        PatchFile {
            schema: SCHEMA,
            engine: if self.engine.is_empty() {
                ENGINE_VERSION.to_owned()
            } else {
                self.engine.clone()
            },
            meta: MetaFile::from(self.meta.clone()),
            voice: Params(voice),
            master: Params(dense(&MASTER_PARAMS, &self.master)),
            generators: self.generators.iter().map(module_file).collect(),
            voice_fx: self.voice_fx.iter().map(module_file).collect(),
            fx: self.fx.iter().map(module_file).collect(),
            modulators: self.modulators.iter().map(module_file).collect(),
            routes: self
                .routes
                .iter()
                .map(|r| RouteFile {
                    src: r.src.to_string(),
                    dst: r.dst.to_string(),
                    depth: r.depth,
                })
                .collect(),
            macros: self
                .macros
                .iter()
                .map(|m| MacroFile {
                    value: m.value,
                    name: m.name.clone(),
                })
                .collect(),
            slots: self
                .slots
                .iter()
                .enumerate()
                .filter_map(|(n, a)| {
                    let slot = SlotIndex::new(n)?;
                    Some((
                        u8::try_from(slot.one_based()).ok()?,
                        a.as_ref()?.to_string(),
                    ))
                })
                .collect(),
        }
    }

    /// The patch as compact JSON text, written from the typed [`PatchFile`] (shortest `f32` text, `float_roundtrip`):
    /// reading it back gives the same bits, and writing that gives the same bytes. Never goes through
    /// `serde_json::Value`, which would widen every `f32`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.to_file())
            .unwrap_or_else(|e| unreachable!("a PatchFile always serialises: {e}"))
    }

    /// [`to_json`](Self::to_json) indented with two spaces, for files a person reads.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&self.to_file())
            .unwrap_or_else(|e| unreachable!("a PatchFile always serialises: {e}"))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------------------------------------------

fn expected_of(spec: &ParamSpec) -> Expected {
    match spec.kind {
        Kind::Enum(items) => Expected::Item(items),
        Kind::Toggle => Expected::OnOff,
        Kind::Float { .. } | Kind::Int { .. } => Expected::Number(spec.unit),
    }
}

/// A file or wire value, checked against its parameter and stored canonically.
///
/// - A number is in the parameter's unit; **a number for an enum is `bad_value`** (with the item list): indices
///   change when the catalogue grows, keys do not.
/// - Text goes through [`ParamSpec::parse`] (unit suffixes, enum keys, `on`/`off`).
/// - A bool is for toggles only.
/// - Anything else (`null`, arrays, objects) is `bad_value`.
///
/// Non-finite numbers are `non_finite`, `-0.0` is stored as `0.0` and the top of a periodic range as its bottom
/// ([`canonical_value`]).
pub fn resolve_value(spec: &ParamSpec, raw: &RawValue) -> Result<f32, PatchError> {
    let bad = |text: String| {
        PatchError::Value(ValueError::BadValue {
            param: spec.key,
            text,
            expected: expected_of(spec),
        })
    };
    let v = match raw {
        RawValue::Num(x) => match spec.kind {
            Kind::Enum(_) => return Err(bad(format!("{x}"))),
            _ => *x,
        },
        RawValue::Text(t) => spec.parse(t)?,
        RawValue::Bool(b) => match spec.kind {
            Kind::Toggle => f32::from(u8::from(*b)),
            _ => return Err(bad(b.to_string())),
        },
        RawValue::Other(v) => return Err(bad(v.to_string())),
    };
    Ok(canonical_value(spec, v)?)
}

/// Resolve a sparse `params` object against a table: unknown keys are `unknown_param`, values go through
/// [`resolve_value`], missing keys keep their default. `owner_id` names the module in addresses (`filter1`).
fn resolve_params(
    specs: &[ParamSpec],
    owner: &'static str,
    owner_id: &str,
    params: &Params,
    path: &str,
    skip: Option<&str>,
    errs: &mut Vec<Violation>,
) -> Vec<f32> {
    let mut values: Vec<f32> = specs.iter().map(|s| s.default).collect();
    for (key, raw) in &params.0 {
        if skip == Some(key.as_str()) {
            continue;
        }
        let at = format!("{path}.{key}");
        match specs.iter().position(|s| s.key == key) {
            None => errs.push(Violation::new(
                at,
                PatchError::UnknownParam {
                    addr: format!("{owner_id}.{key}"),
                    owner,
                    keys: specs.iter().map(|s| s.key).collect(),
                },
            )),
            Some(i) => match resolve_value(&specs[i], raw) {
                Ok(v) => values[i] = v,
                Err(e) => errs.push(Violation::new(at, e)),
            },
        }
    }
    values
}

/// Sparse module `params` resolved for a kind: defaults fill the rest. Shared with the ops JSON (`wire`).
pub(crate) fn module_values(
    spec: &ModuleSpec,
    id: &str,
    params: &Params,
    path: &str,
    errs: &mut Vec<Violation>,
) -> Vec<f32> {
    resolve_params(spec.params, spec.key, id, params, path, None, errs)
}

// ---------------------------------------------------------------------------------------------------------------
// PatchFile -> Patch (sparse in)
// ---------------------------------------------------------------------------------------------------------------

impl PatchFile {
    /// Resolve into a [`Patch`]: ids, kinds, addresses and sources parsed, parameters parsed against their specs
    /// with defaults filling the gaps, then [`validate`](crate::validate::validate)d. Reports **every** problem it
    /// finds, in document order, each with a path, a code and a hint. Expects the current [`SCHEMA`]; [`load`](super::load)
    /// migrates older files first.
    pub fn to_patch(&self) -> Result<Patch, Invalid> {
        let mut errs: Vec<Violation> = Vec::new();
        if self.schema != SCHEMA {
            let e = if self.schema > SCHEMA {
                PatchError::SchemaTooNew {
                    found: self.schema,
                    supported: SCHEMA,
                }
            } else {
                PatchError::BadSchema {
                    found: format!(
                        "{} (older files go through load, which migrates them)",
                        self.schema
                    ),
                }
            };
            return Err(Invalid::one("schema", e));
        }

        // Problems are collected per part and reported in the order of a file: voice, master, the sections, routes,
        // macros, slots.
        let mut voice_errs: Vec<Violation> = Vec::new();
        let mut master_errs: Vec<Violation> = Vec::new();
        let mut module_errs: Vec<Violation> = Vec::new();
        let mut sections: [Vec<ModuleInstance>; 4] = Default::default();
        for section in Section::ALL {
            let files = match section {
                Section::Generators => &self.generators,
                Section::VoiceFx => &self.voice_fx,
                Section::Fx => &self.fx,
                Section::Modulators => &self.modulators,
            };
            for (i, mf) in files.iter().enumerate() {
                let path = format!("{section}[{i}]");
                if let Some(m) = resolve_module(mf, section, &path, &mut module_errs) {
                    sections[section as usize].push(m);
                }
            }
        }
        let [generators, voice_fx, fx, modulators] = sections;

        let voice_values = resolve_params(
            &VOICE_PARAMS,
            "voice",
            "voice",
            &self.voice,
            "voice",
            Some("amp_env"),
            &mut voice_errs,
        );
        let amp_env = resolve_amp_env(&self.voice, &modulators, &mut voice_errs);
        let master_values = resolve_params(
            &MASTER_PARAMS,
            "master",
            "master",
            &self.master,
            "master",
            None,
            &mut master_errs,
        );
        errs.extend(voice_errs);
        errs.extend(master_errs);
        errs.extend(module_errs);

        let mut routes = Vec::with_capacity(self.routes.len());
        for (i, rf) in self.routes.iter().enumerate() {
            let path = format!("routes[{i}]");
            let src = rf.src.parse::<ModSource>();
            let dst = rf.dst.parse::<ParamAddr>();
            let depth = canonical_value(&DEPTH, rf.depth);
            match (src, dst, depth) {
                (Ok(src), Ok(dst), Ok(depth)) => routes.push(Route { src, dst, depth }),
                (src, dst, depth) => {
                    if let Err(e) = src {
                        errs.push(Violation::new(format!("{path}.src"), e));
                    }
                    if let Err(e) = dst {
                        errs.push(Violation::new(format!("{path}.dst"), e));
                    }
                    if let Err(e) = depth {
                        errs.push(Violation::new(format!("{path}.depth"), e));
                    }
                }
            }
        }

        if self.macros.len() > MACROS {
            errs.push(Violation::new(
                "macros",
                PatchError::TooMany {
                    what: "macros",
                    max: MACROS,
                },
            ));
        }
        let mut macros: [Macro; MACROS] = core::array::from_fn(|_| Macro::default());
        for (i, (slot, mf)) in macros.iter_mut().zip(&self.macros).enumerate() {
            match canonical_value(&MACRO_VALUE, mf.value) {
                Ok(value) => {
                    *slot = Macro {
                        value,
                        name: mf.name.clone(),
                    };
                }
                Err(e) => errs.push(Violation::new(format!("macros[{i}].value"), e)),
            }
        }

        let mut slots: [Option<ParamAddr>; crate::model::SLOTS] = core::array::from_fn(|_| None);
        for (&n, text) in &self.slots {
            let path = format!("slots.{n}");
            let Some(index) = SlotIndex::from_one_based(usize::from(n)) else {
                errs.push(Violation::new(
                    path,
                    AddrError::BadIndex {
                        text: n.to_string(),
                        max: crate::model::SLOTS,
                    },
                ));
                continue;
            };
            match text.parse::<ParamAddr>() {
                Ok(a) => slots[index.get()] = Some(a),
                Err(e) => errs.push(Violation::new(path, e)),
            }
        }

        if !errs.is_empty() {
            return Err(Invalid(errs));
        }
        let (Some(amp_env), Ok(voice_values), Ok(master)) = (
            amp_env,
            <[f32; VoiceParam::COUNT]>::try_from(voice_values),
            <[f32; MasterParam::COUNT]>::try_from(master_values),
        ) else {
            return Err(Invalid::one("voice.amp_env", PatchError::MissingAmpEnv));
        };
        let patch = Patch {
            engine: self.engine.clone(),
            meta: PatchMeta::from(self.meta.clone()),
            voice: VoiceSettings {
                values: voice_values,
                amp_env,
            },
            master,
            generators,
            voice_fx,
            fx,
            modulators,
            routes,
            macros,
            slots,
        };
        let problems = violations(&patch);
        if problems.is_empty() {
            Ok(patch)
        } else {
            Err(Invalid(problems))
        }
    }
}

fn resolve_module(
    mf: &ModuleFile,
    section: Section,
    path: &str,
    errs: &mut Vec<Violation>,
) -> Option<ModuleInstance> {
    let id = ModuleId::new(&mf.id);
    let kind = mf.kind.parse::<ModuleKind>();
    let (id, kind) = match (id, kind) {
        (Ok(id), Ok(kind)) => (id, kind),
        (id, kind) => {
            if let Err(e) = id {
                errs.push(Violation::new(format!("{path}.id"), e));
            }
            if let Err(e) = kind {
                errs.push(Violation::new(format!("{path}.kind"), e));
            }
            return None;
        }
    };
    if kind.section() != section {
        errs.push(Violation::new(
            path,
            PatchError::WrongSection {
                id: id.to_string(),
                kind,
                expected: kind.section(),
                got: section,
            },
        ));
        return None;
    }
    let values = module_values(
        kind.spec(),
        id.as_str(),
        &mf.params,
        &format!("{path}.params"),
        errs,
    );
    Some(ModuleInstance {
        id,
        kind,
        label: mf.label.clone(),
        enabled: mf.enabled,
        values,
    })
}

/// `voice.amp_env` from the file, or the first enabled `adsr` of `modulators` when it is left out.
fn resolve_amp_env(
    voice: &Params,
    modulators: &[ModuleInstance],
    errs: &mut Vec<Violation>,
) -> Option<ModuleId> {
    match voice.get("amp_env") {
        Some(RawValue::Text(t)) => match ModuleId::new(t) {
            Ok(id) => Some(id),
            Err(e) => {
                errs.push(Violation::new("voice.amp_env", e));
                None
            }
        },
        Some(other) => {
            errs.push(Violation::new(
                "voice.amp_env",
                PatchError::BadFile {
                    message: format!("amp_env must be a module id like \"env1\", found {other:?}"),
                },
            ));
            None
        }
        None => {
            let first = modulators
                .iter()
                .find(|m| m.enabled && m.kind == ModuleKind::Adsr);
            if first.is_none() {
                errs.push(Violation::new("voice.amp_env", PatchError::MissingAmpEnv));
            }
            first.map(|m| m.id.clone())
        }
    }
}
