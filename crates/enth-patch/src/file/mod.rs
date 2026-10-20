//! `PatchFile`, the serde layer of Patch v1 (design.md §2): the JSON shape of a `*.patch.json` file.
//!
//! The types are lane S1a's; the conversions are lane S1b's: [`Patch::to_file`](crate::model::Patch::to_file),
//! [`Patch::to_json`](crate::model::Patch::to_json), [`PatchFile::to_patch`] and [`load`] (parse, migrate, resolve,
//! validate). The shape:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "engine": "0.2.0",
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
//! Rules the conversions implement:
//!
//! - **Sparse in, dense out.** Reading accepts any subset of a module's parameters in any order (defaults fill the
//!   rest; a migration step supplies an *old* default where one changed), and a file without `engine`, `meta`,
//!   `voice`, `master`, sections, `macros` or `slots` (only `schema` is required). Writing is always dense, in spec order
//!   ([`Params`] keeps order both ways), so a later default change cannot alter an old file.
//! - **Values are parsed against their `ParamSpec`** ([`enth_params::ParamSpec::parse`] for text,
//!   [`enth_params::ParamSpec::check`] for numbers), so errors are typed (`out_of_range`, `wrong_unit`,
//!   `not_integral`, `bad_value`) rather than serde's "did not match any variant". Ids, kinds, addresses and
//!   positions are plain strings here for the same reason, resolved with the [`crate::addr`] parsers and
//!   [`ModuleKind::from_str`](crate::catalogue::ModuleKind).
//! - Enums are written as item keys and toggles as `true`/`false`; numbers as `serde_json` prints `f32` with
//!   `float_roundtrip` (shortest round-trip text, so `900.0`, not `900`; decided 2026-10-19). Non-finite numbers
//!   are rejected (`1e39` reads as infinity; NaN and infinity would write as `null`), and `-0.0` reads as `+0.0`.
//! - `voice.amp_env` is the module id of the amplitude envelope, stored with the voice parameters. A file that
//!   omits it gets the first enabled `adsr` of `modulators`; with none, `missing_amp_env`.
//! - **Numbers never stand for enum items** (`"wave": 2` is `bad_value` with the item list: indices change when the
//!   catalogue grows). Toggles take `true`/`false`, `on`/`off` or `0`/`1`. A phase of 360 is read as 0 and `-0.0` as
//!   0 (see `validate`).
//! - **Errors are collected**: reading reports every problem it finds with a path (`generators[1].params.cutoff`),
//!   a code and a hint, and `did_you_mean` where a near miss exists.
//! - Duplicate keys: parameters, fields and sections of a current-schema file are rejected when repeated; a file that
//!   needs migration is read through `serde_json::Value`, where the last duplicate wins (`migrate` notes nothing).
//! - `macros` holds up to eight entries, in order (written: all eight). `slots` maps 1-based slot numbers to
//!   addresses (written sparse, only assigned slots).
//! - Unknown fields are rejected (`deny_unknown_fields`); `schema` is checked and migrated before these types are
//!   used (`migrate` works on `serde_json::Value`).
//! - **Write through the typed `PatchFile`** (`Patch::to_json`): `serde_json::to_value` widens an `f32` to `f64` text
//!   (`0.1` prints as `0.10000000149011612`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::PatchMeta;

mod convert;
mod load;

pub(crate) use convert::module_values;
pub use convert::resolve_value;
pub use load::{Loaded, load, load_value};

/// The current patch file schema version. Bump it only with a migration step and fixtures: `migrate::STEPS` must
/// have exactly `SCHEMA` entries, which the compiler checks.
pub const SCHEMA: u32 = 1;

/// The engine version stamped on a patch written without one (`Patch::init()` has none): the version of the engine
/// build that authors it. It must equal `enth_engine::ENGINE_VERSION`. This crate (L1) cannot depend on the engine
/// (L2), so the constant lives here and `enth-engine`'s `tests/engine_version.rs` fails when the two differ: a DSP
/// change that bumps one bumps the other in the same commit.
///
/// The stamp is a record, not an input: nothing branches on it today (loading keeps whatever stamp a file has, an
/// older one included, and the sound hash leaves it out). Compatibility modes keyed on it are later work
/// (design.md §5).
pub const ENGINE_VERSION: &str = "0.2.0";

/// A whole patch file. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchFile {
    /// Schema version ([`SCHEMA`]).
    pub schema: u32,
    /// Engine version the patch was authored on. A file may leave it out (a hand- or agent-written file does not
    /// know it); the patch then has an empty stamp and writing it stamps [`ENGINE_VERSION`].
    #[serde(default)]
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
    /// Anything else (`null`, an array, an object): never valid, kept so the conversion can report `bad_value` with
    /// the parameter's name instead of serde's "did not match any variant".
    Other(serde_json::Value),
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

    /// `f32` → text → `f32` through the three paths a patch value takes: a typed field, an untagged [`RawValue`]
    /// (serde buffers the number as an `f64` first) and `serde_json::Value` (migrations).
    fn json_round_trips(x: f32) -> Result<(), String> {
        let text = serde_json::to_string(&x).map_err(|e| e.to_string())?;
        let typed: f32 = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let raw: RawValue = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let via_value: f32 = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let raw_bits = match raw {
            RawValue::Num(v) => v.to_bits(),
            other => return Err(format!("{text} read as {other:?}")),
        };
        let bits = x.to_bits();
        if typed.to_bits() == bits && raw_bits == bits && via_value.to_bits() == bits {
            Ok(())
        } else {
            Err(format!(
                "{x:e} -> {text} -> {typed:e} / {:e} / {via_value:e}",
                f32::from_bits(raw_bits)
            ))
        }
    }

    #[test]
    fn f32_edge_cases_round_trip_through_json() {
        for x in [
            0.0,
            -0.0, // round-trips as -0.0; the conversions canonicalise it to +0.0
            0.1,
            1.0 / 3.0,
            900.0,
            16_777_216.0,
            f32::MIN_POSITIVE,
            f32::from_bits(1), // the smallest subnormal
            f32::MAX,
            f32::MIN,
            f32::EPSILON,
        ] {
            json_round_trips(x).unwrap();
        }
        // Reading text that is not an f32's shortest form rounds deterministically (here, to 1.0e-7).
        let x: f32 = serde_json::from_str("0.00000010000000000000001").unwrap();
        assert_eq!(x, 1.0e-7);
    }

    #[test]
    fn value_widens_f32_so_files_are_written_typed() {
        // `serde_json::Value` holds an f64, so an f32 written through it prints its widened digits. Files are
        // written from the typed `PatchFile` (shortest f32 text); `Value` is only for reading and migrating.
        assert_eq!(serde_json::to_string(&0.1f32).unwrap(), "0.1");
        assert_eq!(
            serde_json::to_value(0.1f32).unwrap().to_string(),
            "0.10000000149011612"
        );
    }

    proptest::proptest! {
        /// Every finite f32 survives JSON text exactly, on every path. Checked exhaustively once, outside the
        /// gate, over all 4,278,190,080 finite f32 (2026-10-19, F1 review, `serde_json` 1.0.154 with
        /// `float_roundtrip`: zero mismatches; decimal → f64 → f32 double rounding never bites on shortest
        /// f32 text). The gate keeps a fixed-seed sample.
        #[test]
        fn every_finite_f32_round_trips_through_json(bits in proptest::prelude::any::<u32>()) {
            let x = f32::from_bits(bits);
            proptest::prop_assume!(x.is_finite());
            proptest::prop_assert_eq!(json_round_trips(x), Ok(()));
        }
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
