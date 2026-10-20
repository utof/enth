//! **Lane PBC** (the preset core; its format is reviewed at freeze point F1r). Pre-declared by lane S0r (R1/S1).
//!
//! Presets (report 03 §2.3, summary "Presets" and "Preset loads"). This module is pure data and logic: the session
//! does no file I/O, so the files are read and written by a [`PresetStore`] the shell supplies (`enth-plugin` has the
//! one for folders, [`MemStore`] the one for tests and the dev window).
//!
//! # Format
//!
//! A preset is one `<name>.patch.json` file holding exactly [`Patch::to_json`](enth_patch::Patch::to_json): the same
//! text the plugin saves in its `patch` field, so a host project and a preset file are interchangeable and a preset
//! saves back byte for byte. There is no index file: the name, author, description and tags are the patch's own
//! [`PatchMeta`]. Folders are directories. A preset's identity is its [`PresetKey`]: where it lives
//! ([`Source`]) and its path without the suffix, `/`-separated, whatever the platform.
//!
//! # Where presets come from
//!
//! | Source | Where | Writable |
//! | --- | --- | --- |
//! | [`Source::Factory`] | `content/presets/**` of the repository, embedded in the build by `build.rs` ([`factory_files`]) | no |
//! | [`Source::User`] | the user folder (`enth-plugin`: `ENTH_PRESETS_DIR`, else `Documents/enth/Presets`) | yes |
//!
//! Factory presets use a small tag vocabulary ([`tags`]): one *type* plus up to four *character* words; user tags
//! are free. **Embedded, not loaded at run time**, because a plugin is one file the host copies around: no install
//! step can lose the sounds, and the browser works the first time it opens. It costs the binary about 6 KB a preset
//! and a rebuild when a preset changes; the build script finds new files by itself.
//!
//! # Index and search
//!
//! [`Library::build`] parses the files once (name, author, description, tags; a file that does not load is listed in
//! [`Library::problems`], never fatal) and [`Library::search`] answers a [`Query`]: the words of the text must each
//! begin a word of the name, tags, folder, author or description, in any order, ranked name first; `#tag` filters by
//! tag and `by:name` by author. [`Favorites`] are a list of keys, saved by the store in the user folder.
//!
//! # Loading and saving in the session
//!
//! - [`Session::load_preset`] is the editor's load: **one undo step** built from
//!   [`ops_between`](enth_patch::ops_between), delivered like a host state load (one plan that forgets the host's
//!   remembered slot values, a re-baseline of the fold, a rename request) with **no host writes** (a per-parameter
//!   write storm on a load crashed VST3 hosts, nih-plug #92), and it marks the project dirty (`PatchEdited`). Undo and
//!   redo of that step are delivered the same way.
//! - [`Session::save_preset`] writes the current patch as a user preset through a store, with a name and tags, and
//!   never overwrites without being told to ([`SaveRequest::overwrite`]).

use std::collections::{BTreeMap, BTreeSet};

use enth_patch::{Op, Patch, PatchMeta};
use serde::{Deserialize, Serialize};

use crate::bridge::host_layout_changed;
use crate::model::{Applied, Session, SessionError, ShellRequest};
use crate::state::StateError;

/// The suffix of a preset file.
pub const FILE_SUFFIX: &str = ".patch.json";

/// The file in the user folder that holds the favorites. It does not end in [`FILE_SUFFIX`], so a scan skips it.
pub const FAVORITES_FILE: &str = "favorites.json";

/// The schema string of the favorites file.
pub const FAVORITES_SCHEMA: &str = "enth.favorites/1";

/// The longest preset name, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// The most tags a user preset keeps, and the longest tag, in characters.
pub const MAX_TAGS: usize = 12;
/// See [`MAX_TAGS`].
pub const MAX_TAG_CHARS: usize = 24;

/// The most characters of a file stem the saved name may give (the rest is cut).
const MAX_STEM_CHARS: usize = 80;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/factory.rs"));
}

// ---------------------------------------------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------------------------------------------

/// The factory tag vocabulary. A factory preset carries exactly one *type* and up to [`tags::MAX_CHARACTER`]
/// *character* words, all lower case; `tests/presets.rs` in `enth-plugin` checks every file of `content/presets`
/// against [`tags::check_factory`]. User presets may use any words.
pub mod tags {
    use super::PatchMeta;

    /// What kind of sound it is. Exactly one per factory preset. `perc` is a drum or percussion hit (R1/S1 lane INT;
    /// the kick and snare candidates were `fx` before it); `init` is the empty starting sound.
    pub const TYPES: [&str; 10] = [
        "bass", "lead", "pad", "pluck", "keys", "arp", "perc", "fx", "texture", "init",
    ];

    /// How it sounds or moves. Up to [`MAX_CHARACTER`] per factory preset.
    pub const CHARACTER: [&str; 16] = [
        "bright", "dark", "warm", "clean", "gritty", "hollow", "metallic", "noisy", "soft", "hard",
        "smooth", "wide", "mono", "evolving", "rhythmic", "vintage",
    ];

    /// The most character words on one factory preset.
    pub const MAX_CHARACTER: usize = 4;

    /// Whether `tag` is one of the [`TYPES`].
    pub fn is_type(tag: &str) -> bool {
        TYPES.contains(&tag)
    }

    /// Whether `tag` is one of the [`CHARACTER`] words.
    pub fn is_character(tag: &str) -> bool {
        CHARACTER.contains(&tag)
    }

    /// What is wrong with a factory preset's metadata, in words; empty if it is fine: a name, exactly one type, only
    /// vocabulary words, no repeats, at most [`MAX_CHARACTER`] character words.
    pub fn check_factory(meta: &PatchMeta) -> Vec<String> {
        let mut problems = Vec::new();
        if meta.name.trim().is_empty() {
            problems.push("it has no name".to_owned());
        }
        if meta.name.chars().count() > super::MAX_NAME_CHARS {
            problems.push(format!(
                "its name is longer than {} characters",
                super::MAX_NAME_CHARS
            ));
        }
        let types = meta.tags.iter().filter(|t| is_type(t)).count();
        if types != 1 {
            problems.push(format!(
                "it needs exactly one type tag ({}), it has {types}",
                TYPES.join(", ")
            ));
        }
        let character = meta.tags.iter().filter(|t| is_character(t)).count();
        if character > MAX_CHARACTER {
            problems.push(format!(
                "it has {character} character tags, the most is {MAX_CHARACTER}"
            ));
        }
        for tag in &meta.tags {
            if !is_type(tag) && !is_character(tag) {
                problems.push(format!(
                    "the tag {tag:?} is not in the vocabulary (types: {}; character: {})",
                    TYPES.join(", "),
                    CHARACTER.join(", ")
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for tag in &meta.tags {
            if !seen.insert(tag) {
                problems.push(format!("the tag {tag:?} is there twice"));
            }
        }
        problems
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------------------------------------------

/// Where a preset comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// Shipped with the plugin; read-only.
    Factory,
    /// In the user's folder; writable.
    User,
}

impl Source {
    /// "Factory" or "User", for the browser's tabs.
    pub const fn label(self) -> &'static str {
        match self {
            Source::Factory => "Factory",
            Source::User => "User",
        }
    }

    /// Whether the browser may not save here.
    pub const fn read_only(self) -> bool {
        matches!(self, Source::Factory)
    }

    const fn tag(self) -> &'static str {
        match self {
            Source::Factory => "factory",
            Source::User => "user",
        }
    }
}

/// The identity of a preset: its [`Source`] and its path without the suffix, `/`-separated and relative to the
/// source's folder (`bass/Sub One`). Stable across platforms, so favorites and the "last loaded" bookmark survive a
/// project moving between machines.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresetKey {
    /// Factory or user.
    pub source: Source,
    /// `folder/folder/name`, no suffix.
    pub path: String,
}

impl PresetKey {
    /// A key; the path is not checked (see [`PresetKey::parse`] and [`valid_path`]).
    pub fn new(source: Source, path: impl Into<String>) -> Self {
        PresetKey {
            source,
            path: path.into(),
        }
    }

    /// A factory key.
    pub fn factory(path: impl Into<String>) -> Self {
        PresetKey::new(Source::Factory, path)
    }

    /// A user key.
    pub fn user(path: impl Into<String>) -> Self {
        PresetKey::new(Source::User, path)
    }

    /// The last segment: the file's name without the suffix.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The folders above it (`bass` or `Mine/Leads`), empty for a preset at the top.
    pub fn folder(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(folder, _)| folder)
    }

    /// The file's path relative to the source's folder, with the suffix and `/` separators.
    pub fn file_name(&self) -> String {
        format!("{}{FILE_SUFFIX}", self.path)
    }

    /// The text form, `factory:bass/Sub One` or `user:Mine/Wobble`, as the favorites file and the saved bookmark
    /// ([`PresetState::last`]) hold it.
    pub fn to_text(&self) -> String {
        format!("{}:{}", self.source.tag(), self.path)
    }

    /// The key a text form names; `None` for anything else, a path that climbs out (`..`) or has an empty segment.
    pub fn parse(text: &str) -> Option<PresetKey> {
        let (source, path) = text.split_once(':')?;
        let source = match source {
            "factory" => Source::Factory,
            "user" => Source::User,
            _ => return None,
        };
        valid_path(path).then(|| PresetKey::new(source, path))
    }
}

impl std::fmt::Display for PresetKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_text())
    }
}

/// Whether `path` is a key's path: non-empty segments separated by `/`, none of them `.` or `..`, no backslash, no
/// control character.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && !path.chars().any(|c| c == '\\' || c.is_control())
}

// ---------------------------------------------------------------------------------------------------------------
// Entries and the library
// ---------------------------------------------------------------------------------------------------------------

/// A preset file as a store read it: its key and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPreset {
    /// Where it is.
    pub key: PresetKey,
    /// The file's text.
    pub text: String,
}

/// A preset in the index: its key and the metadata the browser shows and searches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetEntry {
    /// Where it is.
    pub key: PresetKey,
    /// Name, author, description, tags.
    pub meta: PatchMeta,
}

impl PresetEntry {
    /// The name to show: the patch's own, else the file's.
    pub fn title(&self) -> &str {
        if self.meta.name.trim().is_empty() {
            self.key.name()
        } else {
            &self.meta.name
        }
    }
}

/// A file the scan could not use, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The file.
    pub key: PresetKey,
    /// What was wrong with it.
    pub message: String,
}

/// The embedded factory presets, as files, in path order.
pub fn factory_files() -> Vec<RawPreset> {
    embedded::FACTORY_FILES
        .iter()
        .filter_map(|(rel, text)| {
            let path = rel.strip_suffix(FILE_SUFFIX)?;
            Some(RawPreset {
                key: PresetKey::factory(path),
                text: (*text).to_owned(),
            })
        })
        .collect()
}

/// The text of one embedded factory preset.
pub fn factory_text(key: &PresetKey) -> Option<&'static str> {
    if key.source != Source::Factory {
        return None;
    }
    let file = key.file_name();
    embedded::FACTORY_FILES
        .iter()
        .find(|(rel, _)| *rel == file)
        .map(|(_, text)| *text)
}

/// Read a preset's text into a patch. A file from an older schema migrates; one from a newer build, or damage, is an
/// [`PresetError::Invalid`] naming `key`.
pub fn parse_preset(key: &PresetKey, text: &str) -> Result<Patch, PresetError> {
    enth_patch::file::load(text)
        .map(|loaded| loaded.patch)
        .map_err(|e| PresetError::Invalid {
            key: key.clone(),
            message: e.to_string(),
        })
}

/// The index of every preset: factory and user, sorted (factory first, then by folder, then by name, ignoring
/// case), with the favorites. Build it from the files a store found ([`Library::build`]); scanning is lazy, done when
/// the browser opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    entries: Vec<PresetEntry>,
    problems: Vec<Problem>,
    favorites: Favorites,
}

impl Library {
    /// Index `files`. A file that does not load is reported in [`Library::problems`] and left out.
    pub fn build(files: impl IntoIterator<Item = RawPreset>, favorites: Favorites) -> Library {
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for file in files {
            match parse_preset(&file.key, &file.text) {
                Ok(patch) => entries.push(PresetEntry {
                    key: file.key,
                    meta: patch.meta,
                }),
                Err(PresetError::Invalid { key, message }) => {
                    problems.push(Problem { key, message });
                }
                Err(_) => {}
            }
        }
        entries.sort_by_cached_key(|e| {
            (
                e.key.source,
                e.key.folder().to_lowercase(),
                e.title().to_lowercase(),
                e.key.path.clone(),
            )
        });
        problems.sort_by(|a, b| a.key.cmp(&b.key));
        Library {
            entries,
            problems,
            favorites,
        }
    }

    /// Every entry, in browsing order.
    pub fn entries(&self) -> &[PresetEntry] {
        &self.entries
    }

    /// The files that did not load.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// The favorites.
    pub fn favorites(&self) -> &Favorites {
        &self.favorites
    }

    /// The entry with this key.
    pub fn get(&self, key: &PresetKey) -> Option<&PresetEntry> {
        self.entries.iter().find(|e| e.key == *key)
    }

    /// The folders that hold a preset of `source`, each as a path (`bass`, `Mine/Leads`), sorted like the entries
    /// (ignoring case, a parent before its children); every parent of a folder is listed too. The top level is not listed.
    pub fn folders(&self, source: Source) -> Vec<String> {
        let mut folders = BTreeSet::new();
        for entry in self.entries.iter().filter(|e| e.key.source == source) {
            let mut folder = entry.key.folder();
            while !folder.is_empty() {
                folders.insert(folder.to_owned());
                folder = folder.rsplit_once('/').map_or("", |(parent, _)| parent);
            }
        }
        let mut folders: Vec<String> = folders.into_iter().collect();
        folders.sort_by_cached_key(|f| (f.to_lowercase(), f.clone()));
        folders
    }

    /// Every tag in use with how many presets carry it (lower-cased), most used first, then by name: the browser's
    /// tag chips.
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.entries {
            let mut own = BTreeSet::new();
            for tag in &entry.meta.tags {
                own.insert(tag.trim().to_lowercase());
            }
            for tag in own.into_iter().filter(|t| !t.is_empty()) {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut tags: Vec<(String, usize)> = counts.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        tags
    }

    /// The entries a query matches, best first (browsing order when the query has no words).
    pub fn search(&self, query: &Query) -> Vec<&PresetEntry> {
        let parsed = ParsedText::new(&query.text);
        let wanted_tags: Vec<String> = query
            .tags
            .iter()
            .map(|t| t.trim().to_lowercase())
            .chain(parsed.tags.iter().cloned())
            .collect();
        let mut hits: Vec<(u32, usize, &PresetEntry)> = Vec::new();
        for (order, entry) in self.entries.iter().enumerate() {
            if query.source.is_some_and(|s| s != entry.key.source) {
                continue;
            }
            if let Some(folder) = &query.folder
                && !in_folder(entry.key.folder(), folder)
            {
                continue;
            }
            if query.favorites_only && !self.favorites.contains(&entry.key) {
                continue;
            }
            let own: Vec<String> = entry
                .meta
                .tags
                .iter()
                .map(|t| t.trim().to_lowercase())
                .collect();
            if !wanted_tags.iter().all(|t| own.contains(t)) {
                continue;
            }
            if let Some(score) = parsed.score(entry, &own) {
                hits.push((score, order, entry));
            }
        }
        if !parsed.words.is_empty() {
            hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        hits.into_iter().map(|(_, _, e)| e).collect()
    }
}

/// Whether a preset in `folder` is inside `wanted` (that folder or below it).
fn in_folder(folder: &str, wanted: &str) -> bool {
    let wanted = wanted.trim_matches('/');
    wanted.is_empty()
        || folder == wanted
        || folder
            .strip_prefix(wanted)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Step through a list: the entry `delta` places from `current`, wrapping round the ends. When `current` is not in the
/// list (the preset was filtered out, or none was loaded) a step forward gives the first entry and one back the last.
/// The header's ◀ and ▶ buttons.
pub fn step<'a>(
    list: &[&'a PresetEntry],
    current: Option<&PresetKey>,
    delta: isize,
) -> Option<&'a PresetEntry> {
    if list.is_empty() {
        return None;
    }
    let n = list.len() as isize;
    let at = current.and_then(|key| list.iter().position(|e| e.key == *key));
    let next = match at {
        Some(i) => (i as isize + delta).rem_euclid(n),
        None if delta < 0 => n - 1,
        None => 0,
    };
    list.get(usize::try_from(next).ok()?).copied()
}

/// What the browser asks the library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Words typed in the search field: each must begin a word of the name, the tags, the folder, the author or the
    /// description, in any order. `#tag` also requires that tag; `by:jane` requires an author word beginning
    /// "jane".
    pub text: String,
    /// Tags the entry must all have (the tag chips), compared without regard to case.
    pub tags: Vec<String>,
    /// Only this source.
    pub source: Option<Source>,
    /// Only this folder and what is below it.
    pub folder: Option<String>,
    /// Only favorites.
    pub favorites_only: bool,
}

/// The search field, taken apart.
struct ParsedText {
    words: Vec<String>,
    tags: Vec<String>,
    authors: Vec<String>,
}

impl ParsedText {
    fn new(text: &str) -> Self {
        let mut parsed = ParsedText {
            words: Vec::new(),
            tags: Vec::new(),
            authors: Vec::new(),
        };
        for token in text.split_whitespace() {
            let lower = token.to_lowercase();
            if let Some(author) = lower.strip_prefix("by:") {
                parsed.authors.extend(words_of(author));
            } else if let Some(tag) = lower.strip_prefix('#') {
                if !tag.is_empty() {
                    parsed.tags.push(tag.to_owned());
                }
            } else {
                parsed.words.extend(words_of(&lower));
            }
        }
        parsed
    }

    /// `None` if the entry does not match; else its score (higher is better).
    fn score(&self, entry: &PresetEntry, tags: &[String]) -> Option<u32> {
        let author = words_of(&entry.meta.author.to_lowercase());
        if !self
            .authors
            .iter()
            .all(|a| author.iter().any(|w| w.starts_with(a.as_str())))
        {
            return None;
        }
        if self.words.is_empty() {
            return Some(0);
        }
        let name = words_of(&entry.title().to_lowercase());
        let folder = words_of(&entry.key.folder().to_lowercase());
        let description = words_of(&entry.meta.description.to_lowercase());
        let tag_words: Vec<String> = tags.iter().flat_map(|t| words_of(t)).collect();
        let mut total = 0;
        for word in &self.words {
            let best = [
                (&name, 100, 80),
                (&tag_words, 60, 40),
                (&folder, 50, 30),
                (&author, 20, 15),
                (&description, 10, 5),
            ]
            .iter()
            .filter_map(|(field, exact, prefix)| {
                field
                    .iter()
                    .filter_map(|w| {
                        if w == word {
                            Some(*exact)
                        } else if w.starts_with(word.as_str()) {
                            Some(*prefix)
                        } else {
                            None
                        }
                    })
                    .max()
            })
            .max()?;
            total += best;
        }
        Some(total)
    }
}

/// The words of a lower-cased text: runs of letters and digits.
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Favorites
// ---------------------------------------------------------------------------------------------------------------

/// The presets the user starred. A list of keys, not a folder; the store keeps it in the user folder
/// ([`FAVORITES_FILE`]). Keys of presets that no longer exist are kept (a missing file is not a reason to forget a
/// star) and ignored when searching.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorites {
    keys: BTreeSet<PresetKey>,
}

#[derive(Serialize)]
struct FavoritesOut<'a> {
    schema: &'static str,
    favorites: &'a [String],
}

#[derive(Deserialize)]
struct FavoritesIn {
    #[serde(default)]
    favorites: Vec<String>,
}

impl Favorites {
    /// Whether `key` is starred.
    pub fn contains(&self, key: &PresetKey) -> bool {
        self.keys.contains(key)
    }

    /// Star or unstar `key`; whether that changed anything.
    pub fn set(&mut self, key: &PresetKey, on: bool) -> bool {
        if on {
            self.keys.insert(key.clone())
        } else {
            self.keys.remove(key)
        }
    }

    /// The starred keys, in key order.
    pub fn iter(&self) -> impl Iterator<Item = &PresetKey> {
        self.keys.iter()
    }

    /// How many are starred.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The file's text: `{"schema":"enth.favorites/1","favorites":["factory:bass/Sub","user:Mine/Wobble"]}`, keys
    /// sorted, so the same favorites always give the same bytes.
    pub fn to_json(&self) -> String {
        let keys: Vec<String> = self.keys.iter().map(PresetKey::to_text).collect();
        serde_json::to_string(&FavoritesOut {
            schema: FAVORITES_SCHEMA,
            favorites: &keys,
        })
        .unwrap_or_default()
    }

    /// Read the file's text. A text that is not JSON is an error; keys that make no sense are skipped, and so is
    /// anything unknown (a file from a newer build keeps working, but saving it back drops what this build does not
    /// know).
    pub fn from_json(text: &str) -> Result<Favorites, PresetError> {
        let parsed: FavoritesIn = serde_json::from_str(text).map_err(|e| PresetError::Io {
            what: "read the favorites",
            message: e.to_string(),
        })?;
        Ok(Favorites {
            keys: parsed
                .favorites
                .iter()
                .filter_map(|t| PresetKey::parse(t))
                .collect(),
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------------------------------------------

/// What the Save dialog asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRequest {
    /// The preset's name. It becomes the patch's own name and, cleaned of what a file name cannot hold, the file's.
    pub name: String,
    /// The user subfolder, `Leads` or `Mine/Leads`; empty for the top. Created if it does not exist.
    pub folder: String,
    /// The tags; cleaned by [`clean_tags`].
    pub tags: Vec<String>,
    /// Replace a preset of the same name. Without it, saving over one is [`PresetError::Exists`], and the dialog
    /// asks.
    pub overwrite: bool,
}

impl SaveRequest {
    /// A request for a preset at the top of the user folder, with no tags, that will not overwrite.
    pub fn new(name: impl Into<String>) -> Self {
        SaveRequest {
            name: name.into(),
            ..SaveRequest::default()
        }
    }
}

/// What saving would write: the key and the metadata the patch gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavePlan {
    /// The user key the file will have.
    pub key: PresetKey,
    /// The patch's metadata after the save: its author and description kept, the name and tags from the request.
    pub meta: PatchMeta,
}

/// Plan a save of a patch whose metadata is `current`.
///
/// The name is trimmed and may hold anything printable (up to [`MAX_NAME_CHARS`] characters); the file's name is the
/// name with the characters Windows, macOS and Linux forbid in file names replaced by `_`, reserved Windows names
/// (`CON`, `NUL`, …) made safe, and trailing dots and spaces removed. The folder is cleaned segment by segment the
/// same way; a segment that climbs out (`..`) is refused.
pub fn plan_save(current: &PatchMeta, request: &SaveRequest) -> Result<SavePlan, PresetError> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err(PresetError::BadName { why: "it is empty" });
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(PresetError::BadName {
            why: "it is too long",
        });
    }
    let stem = file_stem(name).ok_or(PresetError::BadName {
        why: "it has nothing a file name can hold",
    })?;
    let folder = clean_folder(&request.folder)?;
    let path = if folder.is_empty() {
        stem
    } else {
        format!("{folder}/{stem}")
    };
    let mut meta = current.clone();
    name.clone_into(&mut meta.name);
    meta.tags = clean_tags(&request.tags);
    Ok(SavePlan {
        key: PresetKey::user(path),
        meta,
    })
}

/// A file stem for `name`: forbidden characters replaced by `_`, spaces collapsed, no leading or trailing dots and
/// spaces, reserved Windows names (`CON`, `NUL`, `COM1`…) suffixed with `_`, at most 80 characters. `None` if nothing
/// but replaced characters is left.
pub fn file_stem(name: &str) -> Option<String> {
    let mut stem = String::new();
    let mut last_space = false;
    for c in name.trim().chars() {
        let c = if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || c.is_control()
        {
            '_'
        } else {
            c
        };
        if c == ' ' {
            if !last_space {
                stem.push(' ');
            }
            last_space = true;
        } else {
            stem.push(c);
            last_space = false;
        }
    }
    let stem: String = stem
        .trim_matches([' ', '.'])
        .chars()
        .take(MAX_STEM_CHARS)
        .collect();
    let mut stem = stem.trim_end_matches([' ', '.']).to_owned();
    if stem.chars().all(|c| c == '_') {
        return None;
    }
    let base = stem.split('.').next().unwrap_or("").to_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            base.strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && n.chars().all(|c| ('1'..='9').contains(&c)))
        });
    if reserved {
        stem.push('_');
    }
    Some(stem)
}

/// A user folder path from what the dialog holds: `/` or `\` separate segments, each cleaned like a file stem;
/// empty segments are dropped. A `..` segment is [`PresetError::BadFolder`].
pub fn clean_folder(folder: &str) -> Result<String, PresetError> {
    let mut segments = Vec::new();
    for raw in folder.split(['/', '\\']) {
        let raw = raw.trim();
        if raw.is_empty() || raw == "." {
            continue;
        }
        if raw == ".." {
            return Err(PresetError::BadFolder {
                why: "it climbs out of the user folder",
            });
        }
        let segment = file_stem(raw).ok_or(PresetError::BadFolder {
            why: "a folder name has nothing a file name can hold",
        })?;
        segments.push(segment);
    }
    Ok(segments.join("/"))
}

/// User tags: trimmed, inner spaces collapsed, empty ones dropped, repeats dropped without regard to case (the first
/// spelling stays), each cut to [`MAX_TAG_CHARS`], at most [`MAX_TAGS`] kept. Case is kept: searching ignores it.
pub fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_TAG_CHARS)
            .collect::<String>();
        let tag = tag.trim().to_owned();
        if tag.is_empty() || out.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            continue;
        }
        out.push(tag);
        if out.len() == MAX_TAGS {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

/// Why a preset operation failed. Every variant has a stable snake-case [`code`](PresetError::code) and a one-line
/// [`hint`](PresetError::hint), like the session's errors.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PresetError {
    /// There is no user folder (no home or Documents folder was found, and `ENTH_PRESETS_DIR` is not set).
    NoUserFolder,
    /// The name cannot be saved.
    BadName {
        /// Why, in a few words.
        why: &'static str,
    },
    /// The folder cannot be saved to.
    BadFolder {
        /// Why, in a few words.
        why: &'static str,
    },
    /// A preset of this name is already there and the request did not say to replace it.
    Exists {
        /// The preset in the way.
        key: PresetKey,
    },
    /// There is no such preset.
    NotFound {
        /// The key asked for.
        key: PresetKey,
    },
    /// The source cannot be written to (the factory presets).
    ReadOnly {
        /// The key that was refused.
        key: PresetKey,
    },
    /// The file is not a patch this build can load.
    Invalid {
        /// The file.
        key: PresetKey,
        /// The loader's message.
        message: String,
    },
    /// The file system said no.
    Io {
        /// What was being done.
        what: &'static str,
        /// The system's message.
        message: String,
    },
    /// The session refused the load.
    Session(SessionError),
    /// This implementation does not do that (a fake session or store).
    Unsupported {
        /// What was asked.
        what: &'static str,
    },
}

impl PresetError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            PresetError::NoUserFolder => "no_user_folder",
            PresetError::BadName { .. } => "bad_preset_name",
            PresetError::BadFolder { .. } => "bad_preset_folder",
            PresetError::Exists { .. } => "preset_exists",
            PresetError::NotFound { .. } => "preset_not_found",
            PresetError::ReadOnly { .. } => "preset_read_only",
            PresetError::Invalid { .. } => "preset_invalid",
            PresetError::Io { .. } => "preset_io",
            PresetError::Session(e) => e.code(),
            PresetError::Unsupported { .. } => "unsupported",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            PresetError::NoUserFolder => {
                "set ENTH_PRESETS_DIR to a folder for your presets".to_owned()
            }
            PresetError::BadName { .. } => "type a name with letters or digits in it".to_owned(),
            PresetError::BadFolder { .. } => {
                "use a folder name inside the user folder, like Leads".to_owned()
            }
            PresetError::Exists { .. } => {
                "choose another name, or replace the preset on purpose".to_owned()
            }
            PresetError::NotFound { .. } => {
                "the file was moved or deleted: open the browser again".to_owned()
            }
            PresetError::ReadOnly { .. } => {
                "factory presets cannot be changed: save a copy under your own name".to_owned()
            }
            PresetError::Invalid { .. } => {
                "the file is damaged or from a newer enth: open it with `enth describe`".to_owned()
            }
            PresetError::Io { .. } => {
                "check that the user folder exists and can be written".to_owned()
            }
            PresetError::Session(e) => e.hint(),
            PresetError::Unsupported { .. } => "not available here".to_owned(),
        }
    }
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresetError::NoUserFolder => f.write_str("there is no folder for user presets"),
            PresetError::BadName { why } => write!(f, "that name cannot be saved: {why}"),
            PresetError::BadFolder { why } => write!(f, "that folder cannot be saved to: {why}"),
            PresetError::Exists { key } => write!(f, "{key} already exists"),
            PresetError::NotFound { key } => write!(f, "{key} was not found"),
            PresetError::ReadOnly { key } => write!(f, "{key} is read-only"),
            PresetError::Invalid { key, message } => write!(f, "{key} does not load: {message}"),
            PresetError::Io { what, message } => write!(f, "could not {what}: {message}"),
            PresetError::Session(e) => write!(f, "{e}"),
            PresetError::Unsupported { what } => write!(f, "{what} is not available here"),
        }
    }
}

impl std::error::Error for PresetError {}

impl From<SessionError> for PresetError {
    fn from(e: SessionError) -> Self {
        PresetError::Session(e)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------------------------------------------

/// Where preset files live: the one thing the session cannot do itself. `enth-plugin` implements it over folders
/// (`FsStore`); [`MemStore`] is the in-memory one for tests and the dev window. Both serve the embedded factory
/// presets read-only.
///
/// The methods are plain I/O. Parsing, indexing, searching, naming and the metadata of a save are done here in
/// `enth-session`, so every store behaves the same.
pub trait PresetStore {
    /// Find every preset file, factory and user, and the favorites, and index them. The browser calls it when it
    /// opens; it may take a moment for a big user folder (about 1.4 ms a file by report 03's measurement).
    fn scan(&mut self) -> Library;

    /// The text of one preset ([`PresetError::NotFound`] if it is gone).
    fn read_text(&self, key: &PresetKey) -> Result<String, PresetError>;

    /// Write one user preset: create its folders, and refuse with [`PresetError::Exists`] if it is already there and
    /// `overwrite` is false. [`PresetError::ReadOnly`] for a factory key. The write is all or nothing: a crash
    /// mid-write leaves the old file or none.
    fn write_text(
        &mut self,
        key: &PresetKey,
        text: &str,
        overwrite: bool,
    ) -> Result<(), PresetError>;

    /// Star or unstar a preset, saved at once.
    fn set_favorite(&mut self, key: &PresetKey, on: bool) -> Result<(), PresetError>;

    /// Where user presets go, for the dialog to show; `None` if there is no user folder.
    fn user_location(&self) -> Option<String>;
}

/// A store in memory: the embedded factory presets (or ones given), a map of user files and the favorites. For
/// tests, the dev window and kittest; it can also be told to fail writes.
#[derive(Clone, Debug)]
pub struct MemStore {
    factory: Vec<RawPreset>,
    user: BTreeMap<String, String>,
    favorites: Favorites,
    /// Writes fail with `PresetError::Io` while this is set.
    pub fail_writes: bool,
}

impl Default for MemStore {
    /// The embedded factory presets and no user files.
    fn default() -> Self {
        MemStore::with_factory(factory_files())
    }
}

impl MemStore {
    /// A store serving `factory` instead of the embedded presets.
    pub fn with_factory(factory: Vec<RawPreset>) -> Self {
        MemStore {
            factory,
            user: BTreeMap::new(),
            favorites: Favorites::default(),
            fail_writes: false,
        }
    }

    /// Put a user file there directly (a path like `Mine/Wobble`).
    pub fn put_user(&mut self, path: &str, text: &str) {
        self.user.insert(path.to_owned(), text.to_owned());
    }

    /// The paths of the user files.
    pub fn user_paths(&self) -> Vec<&str> {
        self.user.keys().map(String::as_str).collect()
    }
}

impl PresetStore for MemStore {
    fn scan(&mut self) -> Library {
        let files = self
            .factory
            .iter()
            .cloned()
            .chain(self.user.iter().map(|(path, text)| RawPreset {
                key: PresetKey::user(path.as_str()),
                text: text.clone(),
            }));
        Library::build(files, self.favorites.clone())
    }

    fn read_text(&self, key: &PresetKey) -> Result<String, PresetError> {
        let found = match key.source {
            Source::Factory => self
                .factory
                .iter()
                .find(|f| f.key == *key)
                .map(|f| f.text.clone()),
            Source::User => self.user.get(&key.path).cloned(),
        };
        found.ok_or_else(|| PresetError::NotFound { key: key.clone() })
    }

    fn write_text(
        &mut self,
        key: &PresetKey,
        text: &str,
        overwrite: bool,
    ) -> Result<(), PresetError> {
        if key.source.read_only() {
            return Err(PresetError::ReadOnly { key: key.clone() });
        }
        if !valid_path(&key.path) {
            return Err(PresetError::BadName {
                why: "the path is not valid",
            });
        }
        if self.fail_writes {
            return Err(PresetError::Io {
                what: "write the preset",
                message: "the store was told to fail".to_owned(),
            });
        }
        if self.user.contains_key(&key.path) && !overwrite {
            return Err(PresetError::Exists { key: key.clone() });
        }
        self.user.insert(key.path.clone(), text.to_owned());
        Ok(())
    }

    fn set_favorite(&mut self, key: &PresetKey, on: bool) -> Result<(), PresetError> {
        self.favorites.set(key, on);
        Ok(())
    }

    fn user_location(&self) -> Option<String> {
        Some("memory".to_owned())
    }
}

/// Load the preset `key` names from `store` into the session as one undo step ([`Session::load_preset`]).
pub fn load_from(
    session: &mut dyn crate::SessionEdit,
    store: &dyn PresetStore,
    key: &PresetKey,
) -> Result<PresetLoad, PresetError> {
    let text = store.read_text(key)?;
    let patch = parse_preset(key, &text)?;
    Ok(session.load_preset(&patch, Some(key))?)
}

// ---------------------------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------------------------

/// The instance's preset bookkeeping, as the `enth` field saves it
/// ([`InstanceState::preset`](crate::InstanceState::preset)): the key of the preset last loaded or saved, for the
/// header's ◀ name ▶ to step from.
///
/// The field is optional and absent until a preset is involved, so a state without one keeps its bytes, and the
/// contents are optional with defaults, so this stays within `enth.plugin-state/1` (docs/session.md: a new optional
/// field with a default is not a new version).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetState {
    /// [`PresetKey::to_text`] of the preset last loaded or saved by the editor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

impl PresetState {
    /// The bookmarked key, if it reads as one.
    pub fn last_key(&self) -> Option<PresetKey> {
        self.last.as_deref().and_then(PresetKey::parse)
    }
}

/// What [`Session::load_preset`] did. Build one with `Default` (it is `non_exhaustive`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PresetLoad {
    /// The patch changed. `false` when the preset was the patch already (the bookmark still moves); nothing is
    /// recorded or delivered then.
    pub changed: bool,
    /// The load is one undo step. `false` only if no list of ops could be found for the pair (never seen for valid
    /// patches; the property test of `ops_between` covers random pairs), in which case the patch was replaced like a
    /// host state load and the history cleared.
    pub undoable: bool,
    /// How many ops the step holds.
    pub ops: usize,
    /// What delivery did: `host_writes` is always 0.
    pub applied: Applied,
}

impl Session {
    /// Load `preset` as the editor's preset load: **one undo step**, delivered **like a host state load**, marking
    /// the project dirty. `key` is the preset's key when it came from a store; the "last loaded" bookmark
    /// ([`PresetState::last`]) follows it.
    ///
    /// - **History.** [`ops_between`](enth_patch::ops_between)`(current, preset)` is applied as one transaction and
    ///   recorded as a *load step* ([`Entry::load`](enth_patch::history::Entry::load)); the redo stack is cleared like
    ///   any edit's. Undo restores the patch from before the load, redo the preset, each in one step. The load counts as
    ///   saved ([`HistoryView::is_dirty`](crate::HistoryView::is_dirty) is false right after it, true after any
    ///   edit or an undo), so a header can show whether the sound still is the preset. The engine stamp is the session's
    ///   own (no op changes it). An open drag is closed first, as for undo.
    /// - **Delivery** (this load, and its undo and redo): one plan compiled from the patch as it is, marked to forget
    ///   every remembered host slot value, the fold's baseline reset (the next fold only records), audition notes
    ///   released, [`ShellRequest::RenameParams`] only if the host would see other names, and
    ///   [`ShellRequest::PatchEdited`]. **No [`ShellRequest::Host`] write is made**: the host's 72 parameters are not
    ///   touched, a stale one never overwrites the patch (docs/session.md, "The host bridge"), and a per-parameter
    ///   write storm on a load crashed VST3 hosts (nih-plug #92).
    /// - **Threads.** The GUI thread under the session lock, like any editor edit; never the audio thread, never
    ///   `GuiContext::set_state`.
    pub fn load_preset(
        &mut self,
        preset: &Patch,
        key: Option<&PresetKey>,
    ) -> Result<PresetLoad, SessionError> {
        self.end_gesture();
        self.pump();
        let loaded = self.load_preset_steps(preset, key)?;
        self.remember_preset(key);
        Ok(loaded)
    }

    fn load_preset_steps(
        &mut self,
        preset: &Patch,
        key: Option<&PresetKey>,
    ) -> Result<PresetLoad, SessionError> {
        let Some(ops) = enth_patch::ops_between(&self.patch, preset) else {
            // No list of ops reproduces it. For a valid preset that never happens (the property test of `ops_between`
            // draws random pairs); an invalid one is refused, with the validator's reasons.
            if let Err(invalid) = enth_patch::validate::validate(preset) {
                let error = SessionError::State(StateError::Patch(invalid));
                self.last_error = Some(error.clone());
                return Err(error);
            }
            return Ok(self.replace_with_preset(preset));
        };
        if ops.is_empty() {
            self.unloaded_patch = None;
            return Ok(PresetLoad {
                changed: false,
                undoable: true,
                ..PresetLoad::default()
            });
        }
        let name = if preset.meta.name.trim().is_empty() {
            key.map_or_else(String::new, |k| k.name().to_owned())
        } else {
            preset.meta.name.clone()
        };
        let before = self.patch.clone();
        let count = ops.len();
        self.history
            .edit_load(&mut self.patch, ops, name)
            .map_err(SessionError::Edit)?;
        self.history.mark_saved();
        self.unloaded_patch = None;
        self.last_error = None;
        let applied = self.deliver_load(&before);
        Ok(PresetLoad {
            changed: true,
            undoable: true,
            ops: count,
            applied,
        })
    }

    /// Save the current patch as a user preset through `store`, under `request`'s name, folder and tags. Nothing
    /// changes unless the write succeeds: [`PresetError::Exists`] for a name in use (unless
    /// [`SaveRequest::overwrite`]), [`PresetError::NoUserFolder`] or [`PresetError::Io`] from the store. On success
    /// the patch takes the saved name and tags (one `SetMeta` edit, undoable, skipped if nothing differs), counts as
    /// saved, and becomes the "last loaded" preset. Returns the key it was saved under.
    ///
    /// The file is exactly [`Patch::to_json`] of the patch with that metadata. The author and description are the
    /// patch's own; this does not invent them.
    pub fn save_preset(
        &mut self,
        store: &mut dyn PresetStore,
        request: &SaveRequest,
    ) -> Result<PresetKey, PresetError> {
        self.end_gesture();
        let plan = plan_save(&self.patch.meta, request)?;
        let mut saved = self.patch.clone();
        saved.meta.clone_from(&plan.meta);
        store.write_text(&plan.key, &saved.to_json(), request.overwrite)?;
        if self.patch.meta != plan.meta {
            self.edit(
                vec![Op::SetMeta {
                    meta: plan.meta.clone(),
                }],
                None,
            )?;
        }
        self.history.mark_saved();
        self.remember_preset(Some(&plan.key));
        Ok(plan.key)
    }

    /// The "last loaded" bookmark follows `key`; a load without one forgets it.
    fn remember_preset(&mut self, key: Option<&PresetKey>) {
        let last = key.map(PresetKey::to_text);
        match (&mut self.instance.preset, last) {
            (Some(state), last) => state.last = last,
            (slot @ None, Some(last)) => *slot = Some(PresetState { last: Some(last) }),
            (None, None) => {}
        }
    }

    /// The fallback when no list of ops reproduces the preset: replace the patch like a host state load (the history
    /// starts again), and still tell the host.
    fn replace_with_preset(&mut self, preset: &Patch) -> PresetLoad {
        let report = self.load_patch(&preset.to_json());
        debug_assert!(report.errors.is_empty(), "a preset re-encoded must load");
        if !self.requests.contains(&ShellRequest::PatchEdited) {
            self.requests.push(ShellRequest::PatchEdited);
        }
        PresetLoad {
            changed: true,
            undoable: false,
            ops: 0,
            applied: Applied {
                recompile: self.port.is_some(),
                renamed: true,
                ..Applied::default()
            },
        }
    }

    /// Deliver a whole-patch change from `before` as a load: used by [`Session::load_preset`] and by the undo and
    /// redo of its step. One plan that forgets every remembered host slot value, the fold's baseline reset, audition
    /// notes released, a rename request if the host's parameter names or kinds differ, and `PatchEdited`. No host
    /// write.
    pub(crate) fn deliver_load(&mut self, before: &Patch) -> Applied {
        self.reset_host_baseline();
        // Releasing audition notes needs an active engine; detached there is nothing to release.
        let _ = self.audition_all_off();
        let mut applied = Applied::default();
        if let Some(port) = &mut self.port {
            port.compile_pending = true;
            port.forget_slots = u64::MAX;
            applied.recompile = true;
        }
        if host_layout_changed(before, &self.patch) {
            applied.renamed = true;
            self.requests.push(ShellRequest::RenameParams);
        }
        self.pump();
        if !self.requests.contains(&ShellRequest::PatchEdited) {
            self.requests.push(ShellRequest::PatchEdited);
        }
        applied
    }
}
