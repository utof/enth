//! `PatchFile`, the serde layer of Patch v1 (design.md §2): the JSON shape of a `*.patch.json` file.
//!
//! **Lane S1a defines the types; lane S1b writes the conversions** (`PatchFile` ⇄ [`Patch`](crate::model::Patch):
//! resolve, defaults, validation, migrations, dense writing). The shape:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "engine": "0.1.0",
//!   "meta": { "name": "Soft pluck", "tags": ["pluck"] },
//!   "voice": { "mode": "poly", "polyphony": 8, "glide": 0, "amp_env": "env1" },
//!   "master": { "level": 0 },
//!   "generators": [ { "id": "osc1", "kind": "osc", "params": { "wave": "saw", "level": 100 } } ],
//!   "voice_fx": [ { "id": "filter1", "kind": "svf", "params": { "mode": "lp", "cutoff": 900, "resonance": 20 } } ],
//!   "fx": [],
//!   "modulators": [ { "id": "env1", "kind": "adsr", "params": { "attack": 2, "decay": 300, "sustain": 0 } } ],
//!   "routes": [ { "src": "env1", "dst": "filter1.cutoff", "depth": 40 } ],
//!   "macros": [ { "value": 0.5, "name": "Brightness" } ],
//!   "slots": { "7": "filter1.cutoff" }
//! }
//! ```
//!
//! Rules the conversions implement (hooks for S1b):
//!
//! - **Sparse in, dense out.** Reading accepts any subset of a module's parameters in any order (defaults fill the
//!   rest; a migration step supplies an *old* default where one changed). Writing is always dense, in spec order
//!   ([`Params`] keeps order both ways), so a later default change cannot alter an old file.
//! - **Values are parsed against their `ParamSpec`** ([`enth_params::ParamSpec::parse`] for text,
//!   [`enth_params::ParamSpec::check`] for numbers), so errors are typed (`out_of_range`, `wrong_unit`,
//!   `not_integral`, `bad_value`) rather than serde's "did not match any variant". Ids, kinds, addresses and
//!   positions are plain strings here for the same reason, resolved with the [`crate::addr`] parsers and
//!   [`ModuleKind::from_str`](crate::catalogue::ModuleKind).
//! - Enums are written as item keys and toggles as `true`/`false`; numbers as `serde_json` prints `f32` with
//!   `float_roundtrip` (shortest round-trip text, so `900.0`, not `900`; decided 2026-10-19). Non-finite numbers
//!   are rejected (`1e39` reads as infinity; NaN and infinity would write as `null`), and `-0.0` reads as `+0.0`.
//! - `voice.amp_env` is the module id of the amplitude envelope, stored with the voice parameters.
//! - `macros` holds up to eight entries, in order (written: all eight). `slots` maps 1-based slot numbers to
//!   addresses (written sparse, only assigned slots).
//! - Unknown fields are rejected (`deny_unknown_fields`); `schema` is checked and migrated before these types are
//!   used (`migrate` works on `serde_json::Value`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::PatchMeta;

/// The current patch file schema version. Bump it only with a migration step and fixtures (S1b).
pub const SCHEMA: u32 = 1;

/// A whole patch file. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchFile {
    /// Schema version ([`SCHEMA`]).
    pub schema: u32,
    /// Engine version the patch was authored on.
    pub engine: String,
    /// Name, author, description, tags.
    #[serde(default)]
    pub meta: MetaFile,
    /// The built-in `voice`: its parameters plus `amp_env`.
    #[serde(default)]
    pub voice: Params,
    /// The built-in `master`.
    #[serde(default)]
    pub master: Params,
    /// Generator modules, in order.
    #[serde(default)]
    pub generators: Vec<ModuleFile>,
    /// Per-voice effect modules, in order.
    #[serde(default)]
    pub voice_fx: Vec<ModuleFile>,
    /// Global effect modules, in order.
    #[serde(default)]
    pub fx: Vec<ModuleFile>,
    /// Modulator modules, in order.
    #[serde(default)]
    pub modulators: Vec<ModuleFile>,
    /// Routes, in summation order.
    #[serde(default)]
    pub routes: Vec<RouteFile>,
    /// Macros 1–8, in order.
    #[serde(default)]
    pub macros: Vec<MacroFile>,
    /// Assigned automation slots: 1-based slot number → address text.
    #[serde(default)]
    pub slots: BTreeMap<u8, String>,
}

/// File form of [`PatchMeta`]; every field optional on input.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MetaFile {
    /// Patch name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Author.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Free text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Tags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl From<PatchMeta> for MetaFile {
    fn from(m: PatchMeta) -> Self {
        MetaFile {
            name: m.name,
            author: m.author,
            description: m.description,
            tags: m.tags,
        }
    }
}

impl From<MetaFile> for PatchMeta {
    fn from(m: MetaFile) -> Self {
        PatchMeta {
            name: m.name,
            author: m.author,
            description: m.description,
            tags: m.tags,
        }
    }
}

/// One module in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleFile {
    /// Module id text (checked with [`crate::addr::ModuleId::new`]).
    pub id: String,
    /// Module kind key (`"svf"`).
    pub kind: String,
    /// Bypassed when false; omitted when true.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// UI label; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Parameter values by key.
    #[serde(default)]
    pub params: Params,
}

/// One route in a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFile {
    /// Source text (`"lfo1"`, `"velocity"`, `"macro2"`).
    pub src: String,
    /// Destination address text (`"filter1.cutoff"`).
    pub dst: String,
    /// Percent of the destination's range, −100..=100.
    pub depth: f32,
}

/// One macro in a file.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroFile {
    /// Knob position 0–1.
    #[serde(default)]
    pub value: f32,
    /// Display name; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn yes() -> bool {
    true
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_true(b: &bool) -> bool {
    *b
}

/// A raw parameter value as written in a file, before it is checked against its `ParamSpec`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    /// A number in the parameter's unit (`900.0`).
    Num(f32),
    /// An enum key (`"saw"`), a unit string (`"1.2kHz"`), or a module id (`voice.amp_env`).
    Text(String),
    /// A toggle (`true`/`false`).
    Bool(bool),
}

/// Parameter values by key, **in order**: a JSON object that keeps its key order when read and written, so files
/// are written in spec order. Reading rejects duplicate keys.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Params(pub Vec<(String, RawValue)>);

impl Params {
    /// The value written for `key`, if any.
    pub fn get(&self, key: &str) -> Option<&RawValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl Serialize for Params {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Params;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object of parameter values")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Params, A::Error> {
                let mut out: Vec<(String, RawValue)> =
                    Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((k, v)) = map.next_entry::<String, RawValue>()? {
                    if out.iter().any(|(seen, _)| *seen == k) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate parameter '{k}'"
                        )));
                    }
                    out.push((k, v));
                }
                Ok(Params(out))
            }
        }
        d.deserialize_map(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
      "schema": 1,
      "engine": "0.1.0",
      "meta": { "name": "Soft pluck", "tags": ["pluck"] },
      "voice": { "mode": "poly", "polyphony": 8, "glide": 0, "amp_env": "env1" },
      "master": { "level": 0 },
      "generators": [ { "id": "osc1", "kind": "osc", "params": { "wave": "saw", "level": 100, "sync": false } } ],
      "voice_fx": [ { "id": "filter1", "kind": "svf", "params": { "mode": "lp", "cutoff": "900Hz", "resonance": 20 } } ],
      "fx": [],
      "modulators": [ { "id": "env1", "kind": "adsr", "params": { "attack": 2, "decay": 300, "sustain": 0 } } ],
      "routes": [ { "src": "env1", "dst": "filter1.cutoff", "depth": 40 } ],
      "macros": [ { "value": 0.5, "name": "Brightness" } ],
      "slots": { "7": "filter1.cutoff", "12": "master.level" }
    }"#;

    #[test]
    fn the_documented_shape_parses_and_keeps_order() {
        let f: PatchFile = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(f.schema, SCHEMA);
        let keys: Vec<&str> = f.voice.0.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["mode", "polyphony", "glide", "amp_env"]);
        assert_eq!(f.voice.get("amp_env"), Some(&RawValue::Text("env1".into())));
        assert_eq!(
            f.generators[0].params.get("sync"),
            Some(&RawValue::Bool(false))
        );
        assert_eq!(
            f.voice_fx[0].params.get("cutoff"),
            Some(&RawValue::Text("900Hz".into()))
        );
        assert_eq!(
            f.voice_fx[0].params.get("resonance"),
            Some(&RawValue::Num(20.0))
        );
        assert!(f.generators[0].enabled);
        assert_eq!(f.slots.keys().copied().collect::<Vec<_>>(), [7, 12]);
        // Written back, keys keep their order and numbers print as f32.
        let text = serde_json::to_string(&f).unwrap();
        assert!(
            text.contains(
                r#""voice":{"mode":"poly","polyphony":8.0,"glide":0.0,"amp_env":"env1"}"#
            ),
            "{text}"
        );
        let again: PatchFile = serde_json::from_str(&text).unwrap();
        assert_eq!(again, f);
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_params() {
        let unknown = EXAMPLE.replace("\"fx\": []", "\"fx\": [], \"lanes\": []");
        assert!(serde_json::from_str::<PatchFile>(&unknown).is_err());
        let dup = EXAMPLE.replace("\"glide\": 0", "\"glide\": 0, \"mode\": \"mono\"");
        let e = serde_json::from_str::<PatchFile>(&dup)
            .unwrap_err()
            .to_string();
        assert!(e.contains("duplicate parameter 'mode'"), "{e}");
    }
}
