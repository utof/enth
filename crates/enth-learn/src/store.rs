//! **Owner: lane LM** (the lesson model, freeze point F1l). The reference is docs/learn.md, "Packs" ("Where packs
//! live").
//!
//! The [`PackStore`] seam, shaped like the session's `PresetStore` (`scan`, `read`, `install`), and [`MemStore`],
//! the store in memory for tests and the dev window. This crate does no file I/O: the plugin's `learn.rs` (lane
//! SKL) implements the store over the bundled packs (`include_bytes!` of each built `.enthpack`) and the Packs folder
//! (`<Documents>/enth/Packs/`, `ENTH_PACKS_DIR`), and the CLI and the lesson tool supply their own.
//!
//! [`load_library`] is the one place that turns a store into what the Learn view lists: every pack read through the
//! same verified reader, refusals kept and **listed, never hidden** (a newer pack is named, with its hint), and for
//! two packs with one id the newer version kept (a newer install replaces an older one; on a tie the bundled one).

use std::collections::BTreeMap;
use std::fmt;

use crate::error::LearnError;
use crate::pack::{Pack, PackId, PackVersion};
use crate::reader::read_archive;

/// Where a pack comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// Built into this binary.
    Bundled,
    /// The Packs folder.
    User,
}

/// A pack in a store: its origin and its name there (a file name, `basics.enthpack`, or a folder name).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackKey {
    /// Where it lives.
    pub origin: Origin,
    /// Its name there.
    pub name: String,
}

impl fmt::Display for PackKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let o = match self.origin {
            Origin::Bundled => "bundled",
            Origin::User => "user",
        };
        write!(f, "{o}:{}", self.name)
    }
}

/// A pack's bytes as the store holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackSource {
    /// An `.enthpack` archive.
    Archive(Vec<u8>),
    /// A folder with the same layout: every file by its pack path, `pack.json` included. A folder store skips
    /// hidden files (`.DS_Store`), which the reader would refuse as unlisted.
    Folder(BTreeMap<String, Vec<u8>>),
}

impl PackSource {
    /// Read and verify it.
    pub fn read(&self) -> Result<Pack, LearnError> {
        match self {
            PackSource::Archive(bytes) => read_archive(bytes),
            PackSource::Folder(files) => Pack::from_files(files),
        }
    }
}

/// Where packs are found, read and installed. The shell implements it; every method may fail with
/// [`LearnError::Store`] (the shell's I/O message) or [`LearnError::NotFound`].
pub trait PackStore {
    /// Every pack the store holds, bundled and user, in a stable order.
    fn scan(&mut self) -> Vec<PackKey>;

    /// The bytes of one pack.
    fn read(&self, key: &PackKey) -> Result<PackSource, LearnError>;

    /// Write an `.enthpack` into the Packs folder under `file_name` (replacing a file of that name). Call
    /// [`install`] instead, which verifies it first.
    fn write_user(&mut self, file_name: &str, bytes: &[u8]) -> Result<PackKey, LearnError>;
}

/// Install `bytes` as a user pack: verify it with the reader first (a refused pack is never written), then name the
/// file `<id>.enthpack`, so a newer version of a pack replaces the older file.
pub fn install(store: &mut dyn PackStore, bytes: &[u8]) -> Result<(PackKey, Pack), LearnError> {
    let pack = read_archive(bytes)?;
    let key = store.write_user(&format!("{}.enthpack", pack.id()), bytes)?;
    Ok((key, pack))
}

/// A pack the library could not open, listed with its reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// Where it is.
    pub key: PackKey,
    /// Why it was refused (`code()` and `hint()` for the list).
    pub error: LearnError,
}

/// A pack the library holds, and where it came from.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Where it is.
    pub key: PackKey,
    /// The verified pack.
    pub pack: Pack,
}

/// What the Learn view lists: the packs it can play (one per id) and the ones it refused.
#[derive(Clone, Debug, Default)]
pub struct Library {
    /// One pack per id, in id order.
    pub packs: Vec<Entry>,
    /// Packs that were refused, each with its reason.
    pub problems: Vec<Problem>,
    /// Packs replaced by a newer version with the same id (listed for the browser's "older copy" note).
    pub replaced: Vec<(PackKey, PackId, PackVersion)>,
}

impl Library {
    /// The pack with this id.
    pub fn pack(&self, id: &PackId) -> Option<&Pack> {
        self.packs
            .iter()
            .find(|e| e.pack.id() == id)
            .map(|e| &e.pack)
    }
}

/// Scan `store` and read every pack (see the [module docs](self)).
pub fn load_library(store: &mut dyn PackStore) -> Library {
    let mut lib = Library::default();
    let mut by_id: BTreeMap<PackId, Entry> = BTreeMap::new();
    for key in store.scan() {
        let pack = match store.read(&key).and_then(|s| s.read()) {
            Ok(p) => p,
            Err(error) => {
                lib.problems.push(Problem { key, error });
                continue;
            }
        };
        let id = pack.id().clone();
        let incoming = Entry { key, pack };
        match by_id.remove(&id) {
            None => {
                by_id.insert(id, incoming);
            }
            Some(have) => {
                let (keep, drop) = if newer(&incoming, &have) {
                    (incoming, have)
                } else {
                    (have, incoming)
                };
                lib.replaced
                    .push((drop.key, id.clone(), drop.pack.manifest().version));
                by_id.insert(id, keep);
            }
        }
    }
    lib.packs = by_id.into_values().collect();
    lib
}

/// Whether `a` should replace `b`: a higher version, or the same version bundled.
fn newer(a: &Entry, b: &Entry) -> bool {
    let (va, vb) = (a.pack.manifest().version, b.pack.manifest().version);
    va > vb || (va == vb && a.key.origin == Origin::Bundled && b.key.origin != Origin::Bundled)
}

/// A store in memory: bundled and user packs by name. For tests, the dev window and kittest; it can be told to fail
/// writes.
#[derive(Clone, Debug, Default)]
pub struct MemStore {
    packs: BTreeMap<PackKey, PackSource>,
    /// Writes fail with [`LearnError::Store`] while this is set.
    pub fail_writes: bool,
}

impl MemStore {
    /// An empty store.
    pub fn new() -> MemStore {
        MemStore::default()
    }

    /// Add a pack.
    pub fn insert(&mut self, origin: Origin, name: &str, source: PackSource) {
        self.packs.insert(
            PackKey {
                origin,
                name: name.to_owned(),
            },
            source,
        );
    }
}

impl PackStore for MemStore {
    fn scan(&mut self) -> Vec<PackKey> {
        self.packs.keys().cloned().collect()
    }

    fn read(&self, key: &PackKey) -> Result<PackSource, LearnError> {
        self.packs
            .get(key)
            .cloned()
            .ok_or_else(|| LearnError::NotFound {
                key: key.to_string(),
            })
    }

    fn write_user(&mut self, file_name: &str, bytes: &[u8]) -> Result<PackKey, LearnError> {
        if self.fail_writes {
            return Err(LearnError::Store {
                message: format!("cannot write {file_name}"),
            });
        }
        let key = PackKey {
            origin: Origin::User,
            name: file_name.to_owned(),
        };
        self.packs
            .insert(key.clone(), PackSource::Archive(bytes.to_vec()));
        Ok(key)
    }
}
