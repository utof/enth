//! The set manifest: what lanes EVK (the GR3 kit) and DS (the held-out sets) fill and `enth-listen gr3` reads.
//!
//! A TOML file, `schema = "enth.listen.gr3-set/1"`, one `[[item]]` per reference. Relative paths are relative to the
//! file's own folder. Audio is WAV (PCM 16/24/32 or float 32/64, mono or stereo; see [`super::audio`]).
//!
//! ```toml
//! schema = "enth.listen.gr3-set/1"
//! name = "gr3-2026-10"                 # free text, printed in reports
//! arm_a = "search alone (R3a, tuned on dev)"   # optional labels; arm B is the arm the rule tests
//! arm_b = "model + search"
//!
//! [[item]]
//! id = "r017"                          # the reference id: unique, stable, only the key and the reports show it
//! stratum = "fm-wavetable"             # reporting group (a free string; the report has one row per stratum)
//! degraded = true                      # the arms were given a degraded upload
//! reference = "audio/r017-upload.wav"  # the audio the arms were given (a degraded upload, or the clean sound)
//! clean_reference = "audio/r017.wav"   # REQUIRED when degraded, forbidden otherwise: what the listener hears as R
//! a = "cands/r017-a.wav"               # arm A's top-ranked candidate (by its own objective, never a human pick)
//! b = "cands/r017-b.wav"               # arm B's top-ranked candidate
//! wrong = "cands/r017-init.wav"        # optional: the Init patch rendered with the same performance (catch trials)
//! ```
//!
//! The judged reference is `clean_reference` when there is one, else `reference`. Items that are judged against the
//! same file (a degraded copy and its clean original) form one cluster of the bootstrap.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::Res;

/// The set manifest's schema id.
pub const SCHEMA_SET: &str = "enth.listen.gr3-set/1";

fn default_a() -> String {
    "search alone".into()
}

fn default_b() -> String {
    "model + search".into()
}

/// The file as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetFile {
    schema: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_a")]
    arm_a: String,
    #[serde(default = "default_b")]
    arm_b: String,
    #[serde(rename = "item")]
    items: Vec<ItemFile>,
}

/// One `[[item]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    id: String,
    stratum: String,
    #[serde(default)]
    degraded: bool,
    reference: String,
    clean_reference: Option<String>,
    a: String,
    b: String,
    wrong: Option<String>,
}

/// One reference with its candidates, paths resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The reference id.
    pub id: String,
    /// Reporting stratum.
    pub stratum: String,
    /// The arms were given a degraded upload.
    pub degraded: bool,
    /// The audio the arms were given.
    pub reference: PathBuf,
    /// The clean original, for a degraded item.
    pub clean_reference: Option<PathBuf>,
    /// Arm A's top candidate.
    pub a: PathBuf,
    /// Arm B's top candidate.
    pub b: PathBuf,
    /// The obviously wrong candidate (the Init patch), if the set has one for this item.
    pub wrong: Option<PathBuf>,
    /// The bootstrap cluster: the first 12 hex digits of the SHA-256 of the judged reference file.
    pub cluster: String,
}

impl Item {
    /// What the listener hears as R.
    #[must_use]
    pub fn judged(&self) -> &Path {
        self.clean_reference.as_deref().unwrap_or(&self.reference)
    }

    /// The paths this item names, with the field they came from.
    #[must_use]
    pub fn files(&self) -> Vec<(&'static str, &Path)> {
        let mut v = vec![("reference", self.reference.as_path())];
        if let Some(c) = &self.clean_reference {
            v.push(("clean_reference", c));
        }
        v.push(("a", &self.a));
        v.push(("b", &self.b));
        if let Some(w) = &self.wrong {
            v.push(("wrong", w));
        }
        v
    }
}

/// A loaded, validated set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    /// The set file.
    pub path: PathBuf,
    /// SHA-256 of the set file.
    pub sha256: String,
    /// Free text name.
    pub name: String,
    /// Label of arm A.
    pub arm_a: String,
    /// Label of arm B.
    pub arm_b: String,
    /// The items, in file order.
    pub items: Vec<Item>,
    /// SHA-256 of every named audio file, keyed `<item id>/<field>`.
    pub inputs: BTreeMap<String, String>,
}

/// SHA-256 of `bytes` as lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

impl Set {
    /// Read and validate the set at `path`: the schema, unique ids, the degraded rules, and that every named file
    /// exists (its hash is taken).
    ///
    /// # Errors
    /// A message naming the file, the item and what is wrong.
    pub fn load(path: &Path) -> Res<Self> {
        let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let sha256 = sha256_hex(&text);
        let file: SetFile = toml::from_str(
            std::str::from_utf8(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        if file.schema != SCHEMA_SET {
            return Err(format!(
                "{} has schema {:?}, expected {SCHEMA_SET:?}",
                path.display(),
                file.schema
            )
            .into());
        }
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let resolve = |p: &str| -> PathBuf {
            let q = Path::new(p);
            if q.is_absolute() {
                q.to_path_buf()
            } else {
                base.join(q)
            }
        };
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        let mut inputs = BTreeMap::new();
        let mut hash_of: BTreeMap<PathBuf, String> = BTreeMap::new();
        for f in file.items {
            if f.id.trim().is_empty() || f.stratum.trim().is_empty() {
                return Err(
                    format!("{}: an item has an empty id or stratum", path.display()).into(),
                );
            }
            if !seen.insert(f.id.clone()) {
                return Err(
                    format!("{}: the item id {:?} appears twice", path.display(), f.id).into(),
                );
            }
            match (f.degraded, &f.clean_reference) {
                (true, None) => {
                    return Err(format!(
                        "{}: item {:?} is degraded, so it needs a clean_reference (a degraded upload is judged against the clean original)",
                        path.display(),
                        f.id
                    )
                    .into());
                }
                (false, Some(_)) => {
                    return Err(format!(
                        "{}: item {:?} has a clean_reference but is not marked degraded",
                        path.display(),
                        f.id
                    )
                    .into());
                }
                _ => {}
            }
            let mut item = Item {
                id: f.id,
                stratum: f.stratum,
                degraded: f.degraded,
                reference: resolve(&f.reference),
                clean_reference: f.clean_reference.as_deref().map(resolve),
                a: resolve(&f.a),
                b: resolve(&f.b),
                wrong: f.wrong.as_deref().map(resolve),
                cluster: String::new(),
            };
            for (field, p) in item.files() {
                let h = if let Some(h) = hash_of.get(p) {
                    h.clone()
                } else {
                    let bytes = std::fs::read(p).map_err(|e| {
                        format!(
                            "{}: item {:?}, {field}: {}: {e}",
                            path.display(),
                            item.id,
                            p.display()
                        )
                    })?;
                    let h = sha256_hex(&bytes);
                    hash_of.insert(p.to_path_buf(), h.clone());
                    h
                };
                inputs.insert(format!("{}/{field}", item.id), h);
            }
            item.cluster = inputs
                .get(&format!(
                    "{}/{}",
                    item.id,
                    if item.clean_reference.is_some() {
                        "clean_reference"
                    } else {
                        "reference"
                    }
                ))
                .map(|h| h[..12].to_string())
                .unwrap_or_default();
            items.push(item);
        }
        if items.is_empty() {
            return Err(format!("{}: the set has no items", path.display()).into());
        }
        Ok(Self {
            path: path.to_path_buf(),
            sha256,
            name: file.name,
            arm_a: file.arm_a,
            arm_b: file.arm_b,
            items,
            inputs,
        })
    }

    /// The item with this id.
    #[must_use]
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("enth-listen-gr3set-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(d: &Path, names: &[&str]) {
        for (i, n) in names.iter().enumerate() {
            std::fs::write(d.join(n), format!("audio {i} {n}")).unwrap();
        }
    }

    #[test]
    fn a_set_loads_with_relative_paths_hashes_and_clusters() {
        let d = dir("ok");
        touch(
            &d,
            &[
                "r1.wav", "a1.wav", "b1.wav", "w1.wav", "up2.wav", "r2.wav", "a2.wav", "b2.wav",
            ],
        );
        std::fs::write(
            d.join("set.toml"),
            format!(
                r#"schema = "{SCHEMA_SET}"
name = "t"
arm_b = "the model"

[[item]]
id = "x1"
stratum = "s"
reference = "r1.wav"
a = "a1.wav"
b = "b1.wav"
wrong = "w1.wav"

[[item]]
id = "x2"
stratum = "s"
degraded = true
reference = "up2.wav"
clean_reference = "r1.wav"
a = "a2.wav"
b = "b2.wav"
"#
            ),
        )
        .unwrap();
        let s = Set::load(&d.join("set.toml")).unwrap();
        assert_eq!(
            (s.name.as_str(), s.arm_a.as_str(), s.arm_b.as_str()),
            ("t", "search alone", "the model")
        );
        assert_eq!(s.items.len(), 2);
        // the degraded copy is judged against the clean original, which the clean item also uses: one cluster
        assert_eq!(s.items[0].cluster, s.items[1].cluster);
        assert_eq!(s.items[1].judged(), d.join("r1.wav"));
        assert_eq!(s.items[1].cluster.len(), 12);
        assert!(s.inputs.contains_key("x1/wrong") && s.inputs.contains_key("x2/clean_reference"));
        assert_eq!(s.inputs.len(), 4 + 4);
        assert_eq!(s.sha256.len(), 64);
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn load_text(name: &str, body: &str, files: &[&str]) -> String {
        let d = dir(name);
        touch(&d, files);
        std::fs::write(d.join("set.toml"), body).unwrap();
        let e = Set::load(&d.join("set.toml")).unwrap_err().to_string();
        std::fs::remove_dir_all(&d).unwrap();
        e
    }

    #[test]
    fn mistakes_are_refused_with_the_item_named() {
        let head = format!("schema = \"{SCHEMA_SET}\"\n");
        let item = |extra: &str| {
            format!(
                "{head}[[item]]\nid = \"x1\"\nstratum = \"s\"\nreference = \"r.wav\"\na = \"a.wav\"\nb = \"b.wav\"\n{extra}"
            )
        };
        let files = ["r.wav", "a.wav", "b.wav"];
        let e = load_text("degraded", &item("degraded = true\n"), &files);
        assert!(e.contains("x1") && e.contains("clean_reference"), "{e}");
        let e = load_text("clean", &item("clean_reference = \"r.wav\"\n"), &files);
        assert!(e.contains("not marked degraded"), "{e}");
        let e = load_text("missing", &item("wrong = \"nope.wav\"\n"), &files);
        assert!(
            e.contains("x1") && e.contains("wrong") && e.contains("nope.wav"),
            "{e}"
        );
        let e = load_text("typo", &item("stratun = \"s\"\n"), &files);
        assert!(e.contains("stratun"), "{e}");
        let e = load_text("schema", "schema = \"other/1\"\nitem = []\n", &[]);
        assert!(e.contains("expected"), "{e}");
        let dup = format!("{}{}", item(""), item("").replace(&head, ""));
        let e = load_text("dup", &dup, &files);
        assert!(e.contains("twice"), "{e}");
        let e = load_text("empty", &format!("{head}item = []\n"), &[]);
        assert!(e.contains("no items"), "{e}");
    }
}
