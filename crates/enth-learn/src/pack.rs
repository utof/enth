//! **Owner: lane LM** (the lesson model, freeze point F1l). The reference is docs/learn.md, "Packs" and "The lesson
//! model (lane LM)".
//!
//! The pack: the manifest [`Manifest`] (`enth.pack/1`), its [`Requires`], the ids ([`PackId`], [`LessonId`],
//! [`StepId`]), [`PackVersion`] (semver), [`LessonKey`] (`pack/lesson`, the key of progress and tracks),
//! [`LessonRef`] (the in-flight lesson the session saves), the tracks (Q1), and [`Pack`]: a pack whose every file is
//! verified against its manifest, with its lessons decoded and its patches loaded, ready to hand a
//! [`LessonBundle`] to the player.
//!
//! **Strict input.** The manifest refuses unknown fields and a newer schema (`pack_too_new`, naming the pack from
//! its own JSON so the shell lists it rather than hiding it); `requires` must be met by this build (the highest
//! lesson, challenge and patch schema, the generator and the scorers the pack uses), and the pack's lessons may not
//! use more than it declares. The engine a pack was checked on is recorded (`engine`), never a gate.
//!
//! Authors write `pack.toml` (the `toml` feature); the pack tool seals it into `pack.json` with
//! [`Manifest::seal`], which lists every file with its size and SHA-256, and [`crate::reader::write_archive`]
//! writes the reproducible `.enthpack`. The two forms are one type: `pack.toml` holds every field but `files`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use enth_challenge::ScorerId;
use enth_patch::{ModuleKind, Patch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Doc, LearnError, Need, PackName, bad_field};
use crate::lesson::{LESSON_VERSION, Lesson, StepKind};

/// The pack manifest's schema family.
pub const PACK_KIND: &str = "enth.pack";
/// The manifest version this build writes, and the newest it reads.
pub const PACK_VERSION: u32 = 1;
/// The `schema` string this build writes: `enth.pack/1`.
pub const PACK_SCHEMA: &str = "enth.pack/1";
/// The manifest's own name inside a pack.
pub const MANIFEST_PATH: &str = "pack.json";
/// The credits file every pack carries (authors and licences, shown in the app).
pub const CREDITS_PATH: &str = "CREDITS.md";
/// A lesson's file inside its directory.
pub const LESSON_FILE: &str = "lesson.json";

/// The largest file a pack may hold, in bytes. A patch is about 2 KB and a step's text a few KB.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// The most bytes all of a pack's files may hold together (about 60 lessons are 0.5 MB raw).
pub const MAX_PACK_BYTES: u64 = 16 << 20;
/// The most lessons a pack may hold.
pub const MAX_LESSONS: usize = 256;

// ---------------------------------------------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------------------------------------------

/// Check a slug: `[a-z0-9]` then `[a-z0-9-]`, at most `max` bytes, no trailing or doubled `-`, and a letter first
/// when `letter_first`.
fn check_slug(text: &str, max: usize, letter_first: bool) -> Result<(), &'static str> {
    let b = text.as_bytes();
    if b.is_empty() || b.len() > max {
        return Err("must be 1 to the maximum length of lower-case letters, digits and '-'");
    }
    let first_ok = if letter_first {
        b[0].is_ascii_lowercase()
    } else {
        b[0].is_ascii_lowercase() || b[0].is_ascii_digit()
    };
    if !first_ok {
        return Err(if letter_first {
            "must start with a lower-case letter"
        } else {
            "must start with a lower-case letter or a digit"
        });
    }
    if !b
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
    {
        return Err("may hold only lower-case letters, digits and '-'");
    }
    if b.ends_with(b"-") || text.contains("--") {
        return Err("may not end with '-' or hold '--'");
    }
    Ok(())
}

macro_rules! slug_type {
    ($(#[$doc:meta])* $name:ident, $max:expr, $letter_first:expr, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The longest id, in bytes.
            pub const MAX_LEN: usize = $max;

            /// A checked id.
            pub fn new(text: &str) -> Result<Self, String> {
                check_slug(text, $max, $letter_first)
                    .map(|()| $name(text.to_owned()))
                    .map_err(|why| format!(concat!($what, " {:?} {} (at most {} bytes)"), text, why, $max))
            }

            /// The id as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                $name::new(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
                $name::new(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

slug_type!(
    /// A pack's id (`basics`, `space-and-dynamics`): a letter, then letters, digits and `-`, at most 32 bytes. A
    /// newer install replaces an older one with the same id; progress is keyed by it, so it never changes.
    PackId, 32, true, "pack id"
);
slug_type!(
    /// A lesson's id within its pack (`cutoff`, `low-pass-cutoff`), at most 48 bytes, and its directory's name
    /// (`lessons/<id>/`). Append-only: progress is keyed by it.
    LessonId, 48, false, "lesson id"
);
slug_type!(
    /// A step's id within its lesson (`hear`, `try`, `do-1`), at most 32 bytes. Append-only: progress is keyed by it.
    StepId, 32, false, "step id"
);
slug_type!(
    /// A track's id within its pack (`tour`), at most 32 bytes.
    TrackId, 32, true, "track id"
);

/// A pack's semver version, `MAJOR.MINOR.PATCH` (no pre-release or build part in v1). A newer install replaces an
/// older one with the same [`PackId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackVersion {
    /// Major.
    pub major: u32,
    /// Minor.
    pub minor: u32,
    /// Patch.
    pub patch: u32,
}

impl PackVersion {
    /// `major.minor.patch`.
    pub const fn new(major: u32, minor: u32, patch: u32) -> PackVersion {
        PackVersion {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for PackVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for PackVersion {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("version {s:?} is not MAJOR.MINOR.PATCH (e.g. 1.2.0)");
        let mut parts = s.split('.');
        let mut next = || -> Result<u32, String> {
            let p = parts.next().ok_or_else(bad)?;
            if p.is_empty()
                || (p.len() > 1 && p.starts_with('0'))
                || !p.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(bad());
            }
            p.parse().map_err(|_| bad())
        };
        let v = PackVersion::new(next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(v)
    }
}

impl Serialize for PackVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PackVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A lesson across packs: `pack/lesson` (`basics/cutoff`). The key of [`Progress`](crate::progress::Progress) and
/// the entries of a [`Track`]. Version-free, so updates never reset progress.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LessonKey {
    /// The pack.
    pub pack: PackId,
    /// The lesson.
    pub lesson: LessonId,
}

impl fmt::Display for LessonKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.pack, self.lesson)
    }
}

impl FromStr for LessonKey {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (pack, lesson) = s
            .split_once('/')
            .ok_or_else(|| format!("lesson key {s:?} is not pack/lesson (e.g. basics/cutoff)"))?;
        Ok(LessonKey {
            pack: pack.parse()?,
            lesson: lesson.parse()?,
        })
    }
}

impl Serialize for LessonKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LessonKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The first 8 bytes of a lesson file's SHA-256, written as 16 lower-case hex digits: tells a saved lesson whether
/// the pack under it changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId(pub [u8; 8]);

impl ContentId {
    /// The prefix of `bytes`' SHA-256.
    pub fn of(bytes: &[u8]) -> ContentId {
        let d = sha256(bytes);
        let mut p = [0; 8];
        p.copy_from_slice(&d[..8]);
        ContentId(p)
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl FromStr for ContentId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut p = [0; 8];
        unhex(s, &mut p)
            .ok_or_else(|| format!("content id {s:?} is not 16 lower-case hex digits"))?;
        Ok(ContentId(p))
    }
}

impl Serialize for ContentId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The lesson in flight, as the session saves it in the `enth` field's `learn` object (lane SL): the pack, its
/// version, the lesson, the step and the content prefix of the lesson file. On reload the shell loads the pack and
/// the session resumes the player at `step`; when the content prefix differs (the pack was updated) it restarts
/// the lesson at step 0, and the stash is kept either way.
///
/// JSON: `{"pack":"basics","version":"1.2.0","lesson":"cutoff","step":1,"content":"0123456789abcdef"}`. Saved state
/// is read tolerantly: an unknown field is ignored, so a newer build's reference still resumes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LessonRef {
    /// The pack.
    pub pack: PackId,
    /// The pack's version when the lesson started.
    pub version: PackVersion,
    /// The lesson.
    pub lesson: LessonId,
    /// The step, 0-based.
    pub step: u16,
    /// The first 8 bytes of the lesson file's SHA-256.
    pub content: ContentId,
}

impl LessonRef {
    /// The lesson, without version or step.
    pub fn key(&self) -> LessonKey {
        LessonKey {
            pack: self.pack.clone(),
            lesson: self.lesson.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------------------------

/// Check one path segment: `[A-Za-z0-9._-]+`, not `.` or `..`, not hidden.
fn check_segment(seg: &str) -> Result<(), &'static str> {
    if seg.is_empty() {
        return Err("has an empty segment (a leading, trailing or doubled '/')");
    }
    if seg.starts_with('.') {
        return Err("has a segment starting with '.' ('.', '..' and hidden files are refused)");
    }
    if !seg
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err("may hold only ASCII letters, digits, '.', '_', '-' and '/'");
    }
    Ok(())
}

/// The longest pack path, in bytes.
pub const MAX_PATH_LEN: usize = 160;

/// A file's place in a pack: `CREDITS.md`, `lessons/<lesson>/lesson.json`, `lessons/<lesson>/<text>.md` or
/// `patches/<name>.patch.json`. Relative, `/`-separated, ASCII; `media/` is reserved and empty in v1. Checked when
/// a manifest or an archive entry is read.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackPath(String);

impl PackPath {
    /// A checked path.
    pub fn new(text: &str) -> Result<PackPath, LearnError> {
        let bad = |why| LearnError::BadPath {
            path: text.to_owned(),
            why,
        };
        if text.len() > MAX_PATH_LEN {
            return Err(bad("is longer than 160 bytes"));
        }
        let segs: Vec<&str> = text.split('/').collect();
        for s in &segs {
            check_segment(s).map_err(bad)?;
        }
        match segs.as_slice() {
            [CREDITS_PATH] => Ok(()),
            ["lessons", lesson, file] => {
                check_slug(lesson, LessonId::MAX_LEN, false)
                    .map_err(|_| bad("names a lesson directory that is not a lesson id"))?;
                if *file == LESSON_FILE || is_text_name(file) {
                    Ok(())
                } else {
                    Err(bad(
                        "is neither lesson.json nor a .md text in a lesson directory",
                    ))
                }
            }
            ["patches", name] if is_patch_name(name) => Ok(()),
            ["patches", ..] => Err(bad("a patch is patches/<name>.patch.json")),
            ["media", ..] => Err(bad("media/ is reserved and empty in enth.pack/1")),
            [MANIFEST_PATH] => Err(bad("pack.json is the manifest, not a listed file")),
            _ => Err(bad("is not CREDITS.md or under lessons/ or patches/")),
        }?;
        Ok(PackPath(text.to_owned()))
    }

    /// The path as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PackPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PackPath {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        PackPath::new(&s).map_err(serde::de::Error::custom)
    }
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "pack paths are case-sensitive: `.MD` is refused, so every reader sees the same names"
)]
fn is_text_name(name: &str) -> bool {
    name.len() > 3 && name.ends_with(".md") && check_segment(name).is_ok()
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "pack paths are case-sensitive, as above"
)]
fn is_patch_name(name: &str) -> bool {
    name.len() > ".patch.json".len() && name.ends_with(".patch.json") && check_segment(name).is_ok()
}

/// A patch a lesson names, pack-relative: `patches/saw.patch.json`. Exactly `Patch::to_json()` in the pack.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatchRef(String);

impl PatchRef {
    /// A checked reference.
    pub fn new(text: &str) -> Result<PatchRef, String> {
        match text.strip_prefix("patches/") {
            Some(name) if is_patch_name(name) => Ok(PatchRef(text.to_owned())),
            _ => Err(format!("patch {text:?} is not patches/<name>.patch.json")),
        }
    }

    /// The reference as text, which is its pack path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PatchRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PatchRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PatchRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        PatchRef::new(&s).map_err(serde::de::Error::custom)
    }
}

/// A step's text, a markdown file beside its `lesson.json`: `step-1.md` (in `lessons/<lesson>/`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRef(String);

impl TextRef {
    /// A checked reference: a plain `.md` file name.
    pub fn new(text: &str) -> Result<TextRef, String> {
        if is_text_name(text) {
            Ok(TextRef(text.to_owned()))
        } else {
            Err(format!(
                "text {text:?} is not a .md file name beside lesson.json (e.g. step-1.md)"
            ))
        }
    }

    /// The file name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its pack path in `lesson`'s directory.
    pub fn path_in(&self, lesson: &LessonId) -> String {
        format!("lessons/{lesson}/{}", self.0)
    }
}

impl fmt::Display for TextRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for TextRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TextRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        TextRef::new(&s).map_err(serde::de::Error::custom)
    }
}

/// The path of `lesson`'s file.
pub fn lesson_path(lesson: &LessonId) -> String {
    format!("lessons/{lesson}/{LESSON_FILE}")
}

// ---------------------------------------------------------------------------------------------------------------
// The manifest
// ---------------------------------------------------------------------------------------------------------------

/// The highest lesson, challenge and patch schemas, the generator and the scorers a pack uses. The pack loads only
/// if this build has them all ([`Requires::check`]), and [`Pack`] refuses a pack whose lessons use more than it
/// declares, so `requires` never lies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    /// The lesson schema (`enth.lesson/<N>`).
    pub lesson: u32,
    /// The challenge schema of its Recreate steps; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<u32>,
    /// The patch schema of its patches (`Patch` file `schema`).
    pub patch: u32,
    /// The generator version of its generated targets; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<u16>,
    /// The scorer ids its challenges name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scorers: Vec<ScorerId>,
}

impl Requires {
    /// The first thing this build lacks, if any.
    pub fn unmet(&self) -> Option<Need> {
        if self.lesson == 0 || self.lesson > LESSON_VERSION {
            return Some(Need::Lesson(self.lesson));
        }
        if let Some(c) = self.challenge
            && (c == 0 || c > enth_challenge::model::CHALLENGE_VERSION)
        {
            return Some(Need::Challenge(c));
        }
        if self.patch == 0 || self.patch > enth_patch::file::SCHEMA {
            return Some(Need::Patch(self.patch));
        }
        if let Some(g) = self.generator
            && (g == 0 || g > enth_challenge::spec::LATEST_GENERATOR)
        {
            return Some(Need::Generator(g));
        }
        self.scorers
            .iter()
            .find(|s| !crate::interim::is_known_scorer(s))
            .map(|s| Need::Scorer(s.to_string()))
    }
}

/// The licences of a pack's content, as SPDX ids (default: lesson text `CC-BY-4.0`, patches `CC0-1.0`; the owner
/// confirms the terms, `content/lessons/LICENSE`). `lesson-check` enforces the allowed ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct License {
    /// The lesson text: titles, steps, hints.
    pub text: String,
    /// The patches and other machine-readable data.
    pub patches: String,
}

/// One file of a pack: its path, size and SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Where it is.
    pub path: PackPath,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, 64 lower-case hex digits.
    pub sha256: Sha256Hex,
}

/// A SHA-256 written as 64 lower-case hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hex(pub [u8; 32]);

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl Serialize for Sha256Hex {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Hex {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        let mut out = [0; 32];
        unhex(&s, &mut out).ok_or_else(|| {
            serde::de::Error::custom(format!("sha256 {s:?} is not 64 lower-case hex digits"))
        })?;
        Ok(Sha256Hex(out))
    }
}

/// A module kind by its catalogue key (`osc`, `svf`, `adsr`): what a pack [covers](Manifest::covers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KindKey(pub ModuleKind);

impl Serialize for KindKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.key())
    }
}

impl<'de> Deserialize<'de> for KindKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        ModuleKind::from_key(&s).map(KindKey).ok_or_else(|| {
            serde::de::Error::custom(format!("{s:?} is not a module kind (osc, noise, svf, …)"))
        })
    }
}

/// An ordered list of lessons the app plays across installed packs (Q1): the 12-lesson tour is one, carried by the
/// pack that starts it. A lesson whose pack is not installed is shown as missing, never skipped silently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// Its id within the pack (`tour`).
    pub id: TrackId,
    /// Its title.
    pub title: String,
    /// The lessons, in order, from this or other packs.
    pub lessons: Vec<LessonKey>,
}

/// A pack's manifest, `pack.json` (`enth.pack/1`), or `pack.toml` when authoring (every field but `files`, which
/// [`Manifest::seal`] computes). See docs/learn.md, "The lesson model (lane LM)", for an example.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The pack's id.
    pub id: PackId,
    /// Its semver version.
    pub version: PackVersion,
    /// Its title, shown in the app.
    pub title: String,
    /// Its authors.
    pub authors: Vec<String>,
    /// The licences of its content.
    pub license: License,
    /// What this build must have to play it.
    pub requires: Requires,
    /// The engine version it was checked on (`lesson-check`): recorded, never a gate. Empty when authoring;
    /// [`Manifest::seal`] stamps this build's.
    pub engine: String,
    /// The module kinds its lessons teach, for the browser.
    pub covers: Vec<KindKey>,
    /// Its lessons, in the order the app offers them. Each lives in `lessons/<id>/lesson.json`.
    pub lessons: Vec<LessonId>,
    /// Ordered lists across packs (Q1).
    pub tracks: Vec<Track>,
    /// Every file of the pack but `pack.json`, each with its size and SHA-256. Empty when authoring.
    pub files: Vec<FileEntry>,
}

/// The on-disk shape: the schema first.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    schema: String,
    id: PackId,
    version: PackVersion,
    title: String,
    authors: Vec<String>,
    license: License,
    requires: Requires,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    engine: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    covers: Vec<KindKey>,
    lessons: Vec<LessonId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileEntry>,
}

impl Manifest {
    /// Read `pack.json`: parse, check the schema (`missing_schema`, `bad_schema`, `pack_too_new` naming the pack),
    /// decode (unknown fields refused, a repeated key refused) and [`validate`](Self::validate).
    pub fn from_json(text: &str) -> Result<Manifest, LearnError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| syntax(Doc::Pack, e.to_string()))?;
        check_schema(Doc::Pack, &value, PACK_VERSION)?;
        let file: ManifestFile =
            serde_json::from_str(text).map_err(|e| fields(Doc::Pack, e.to_string()))?;
        let m = file.into_manifest();
        m.validate()?;
        Ok(m)
    }

    /// Read the authoring `pack.toml`: the same checks as [`from_json`](Self::from_json), and `files` must be absent
    /// (the pack tool computes it).
    #[cfg(feature = "toml")]
    pub fn from_toml(text: &str) -> Result<Manifest, LearnError> {
        let value: Value = toml::from_str(text).map_err(|e| syntax(Doc::Pack, e.to_string()))?;
        check_schema(Doc::Pack, &value, PACK_VERSION)?;
        let file: ManifestFile =
            serde_json::from_value(value).map_err(|e| fields(Doc::Pack, e.to_string()))?;
        if !file.files.is_empty() {
            return Err(bad_field(
                Doc::Pack,
                "files",
                "pack.toml lists no files: `cargo xtask pack` computes them",
            ));
        }
        let m = file.into_manifest();
        m.validate()?;
        Ok(m)
    }

    /// The JSON text: compact, schema first, byte-deterministic (what `pack.json` holds).
    pub fn to_json(&self) -> String {
        serde_json::to_string(&ManifestFile::from(self)).unwrap_or_default()
    }

    /// The JSON text, indented.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&ManifestFile::from(self)).unwrap_or_default()
    }

    /// The name a refusal shows.
    pub fn name(&self) -> PackName {
        PackName {
            id: Some(self.id.to_string()),
            version: Some(self.version.to_string()),
            title: Some(self.title.clone()),
        }
    }

    /// Check what the types cannot: a title, 1 to [`MAX_LESSONS`] lessons with no duplicate, unique track ids,
    /// every track entry of this pack naming one of its lessons, unique file paths, and the sizes within the limits.
    pub fn validate(&self) -> Result<(), LearnError> {
        if self.title.trim().is_empty() {
            return Err(bad_field(Doc::Pack, "title", "is empty"));
        }
        if self.lessons.is_empty() || self.lessons.len() > MAX_LESSONS {
            return Err(bad_field(
                Doc::Pack,
                "lessons",
                format!("holds 1 to {MAX_LESSONS} lessons"),
            ));
        }
        unique(self.lessons.iter(), |l| {
            bad_field(Doc::Pack, "lessons", format!("lists {l} twice"))
        })?;
        unique(self.tracks.iter().map(|t| &t.id), |t| {
            bad_field(Doc::Pack, "tracks", format!("has two tracks {t}"))
        })?;
        for (i, t) in self.tracks.iter().enumerate() {
            if t.lessons.is_empty() {
                return Err(bad_field(
                    Doc::Pack,
                    format!("tracks[{i}].lessons"),
                    "is empty",
                ));
            }
            for k in t.lessons.iter().filter(|k| k.pack == self.id) {
                if !self.lessons.contains(&k.lesson) {
                    return Err(bad_field(
                        Doc::Pack,
                        format!("tracks[{i}].lessons"),
                        format!("{k} is not a lesson of this pack"),
                    ));
                }
            }
        }
        unique(self.files.iter().map(|f| &f.path), |p| {
            bad_field(Doc::Pack, "files", format!("lists {p} twice"))
        })?;
        let mut total = 0_u64;
        for f in &self.files {
            if f.size > MAX_FILE_BYTES {
                return Err(LearnError::TooLarge {
                    path: f.path.to_string(),
                    size: f.size,
                    max: MAX_FILE_BYTES,
                });
            }
            total = total.saturating_add(f.size);
        }
        if total > MAX_PACK_BYTES {
            return Err(LearnError::TooLarge {
                path: "*".to_owned(),
                size: total,
                max: MAX_PACK_BYTES,
            });
        }
        Ok(())
    }

    /// The listed entry of `path`.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path.as_str() == path)
    }

    /// This manifest with `files` listing every one of `files` (sorted by path, with size and SHA-256) and the
    /// engine stamped when empty: what `cargo xtask pack` writes as `pack.json`. `files` holds every pack file but
    /// `pack.json`.
    pub fn seal(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<Manifest, LearnError> {
        let mut m = self.clone();
        m.files = files
            .iter()
            .map(|(path, bytes)| {
                Ok(FileEntry {
                    path: PackPath::new(path)?,
                    size: bytes.len() as u64,
                    sha256: Sha256Hex(sha256(bytes)),
                })
            })
            .collect::<Result<_, LearnError>>()?;
        if m.engine.is_empty() {
            enth_patch::file::ENGINE_VERSION.clone_into(&mut m.engine);
        }
        m.validate()?;
        Ok(m)
    }
}

impl ManifestFile {
    fn into_manifest(self) -> Manifest {
        Manifest {
            id: self.id,
            version: self.version,
            title: self.title,
            authors: self.authors,
            license: self.license,
            requires: self.requires,
            engine: self.engine,
            covers: self.covers,
            lessons: self.lessons,
            tracks: self.tracks,
            files: self.files,
        }
    }
}

impl From<&Manifest> for ManifestFile {
    fn from(m: &Manifest) -> Self {
        ManifestFile {
            schema: PACK_SCHEMA.to_owned(),
            id: m.id.clone(),
            version: m.version,
            title: m.title.clone(),
            authors: m.authors.clone(),
            license: m.license.clone(),
            requires: m.requires.clone(),
            engine: m.engine.clone(),
            covers: m.covers.clone(),
            lessons: m.lessons.clone(),
            tracks: m.tracks.clone(),
            files: m.files.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A verified pack
// ---------------------------------------------------------------------------------------------------------------

/// A pack whose every file matched its manifest: its lessons decoded (in manifest order, each with its content id),
/// its patches loaded and its texts checked as UTF-8. Built by [`Pack::from_files`] (a folder, or the files a
/// [`reader`](crate::reader) took out of an `.enthpack`).
#[derive(Clone, Debug)]
pub struct Pack {
    manifest: Manifest,
    lessons: Vec<(Arc<Lesson>, ContentId)>,
    patches: BTreeMap<PatchRef, Arc<Patch>>,
    texts: BTreeMap<String, Arc<str>>,
}

impl Pack {
    /// Verify a pack given as its files by path (`pack.json` included):
    ///
    /// 1. the manifest reads (a newer schema is `pack_too_new`, named) and `requires` is met by this build;
    /// 2. every file is listed (`unlisted_file`), every listed file is there (`missing_file`) with its size and
    ///    SHA-256 (`size_mismatch`, `hash_mismatch`); `CREDITS.md` and each lesson's `lesson.json` are listed;
    /// 3. each lesson reads (`lesson_too_new`, unknown fields refused), has its directory's id, and names only listed
    ///    texts and patches; patches load (older schemas migrate) and texts are UTF-8;
    /// 4. the lessons use no more than `requires` declares (schemas, the generator, the scorers).
    pub fn from_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Pack, LearnError> {
        let manifest_bytes = files
            .get(MANIFEST_PATH)
            .ok_or_else(|| LearnError::Missing {
                path: MANIFEST_PATH.to_owned(),
            })?;
        let manifest_text = std::str::from_utf8(manifest_bytes)
            .map_err(|_| syntax(Doc::Pack, "pack.json is not UTF-8".to_owned()))?;
        let manifest = Manifest::from_json(manifest_text)?;
        if let Some(need) = manifest.requires.unmet() {
            return Err(LearnError::Requires {
                name: manifest.name(),
                need,
            });
        }
        verify_files(&manifest, files)?;
        for path in [CREDITS_PATH.to_owned()]
            .into_iter()
            .chain(manifest.lessons.iter().map(lesson_path))
        {
            if manifest.file(&path).is_none() {
                return Err(LearnError::Missing { path });
            }
        }

        let mut patches = BTreeMap::new();
        let mut texts = BTreeMap::new();
        let mut lessons = Vec::with_capacity(manifest.lessons.len());
        let mut used = Used::default();
        for id in &manifest.lessons {
            let path = lesson_path(id);
            let bytes = &files[&path];
            let text = std::str::from_utf8(bytes).map_err(|_| LearnError::BadContent {
                path: path.clone(),
                message: "not UTF-8".to_owned(),
            })?;
            let (lesson, version) =
                Lesson::from_json_versioned(text).map_err(|e| with_pack(e, &manifest))?;
            if &lesson.id != id {
                return Err(bad_field(
                    Doc::Lesson,
                    format!("{path}: id"),
                    format!("is {}, its directory says {id}", lesson.id),
                ));
            }
            used.lesson = used.lesson.max(version);
            for t in lesson.texts() {
                let p = t.path_in(id);
                let bytes = files
                    .get(&p)
                    .filter(|_| manifest.file(&p).is_some())
                    .ok_or_else(|| {
                        bad_field(
                            Doc::Lesson,
                            format!("{path}: text"),
                            format!("{p} is not a listed file"),
                        )
                    })?;
                let s = std::str::from_utf8(bytes).map_err(|_| LearnError::BadContent {
                    path: p.clone(),
                    message: "not UTF-8".to_owned(),
                })?;
                texts.insert(p, Arc::from(s));
            }
            for r in lesson.patches() {
                if patches.contains_key(r) {
                    continue;
                }
                let bytes = files
                    .get(r.as_str())
                    .filter(|_| manifest.file(r.as_str()).is_some())
                    .ok_or_else(|| {
                        bad_field(
                            Doc::Lesson,
                            format!("{path}: patch"),
                            format!("{r} is not a listed file"),
                        )
                    })?;
                let loaded = load_patch(r.as_str(), bytes)?;
                used.patch = used.patch.max(loaded.from_schema);
                patches.insert(r.clone(), Arc::new(loaded.patch));
            }
            used.note(&lesson);
            lessons.push((Arc::new(lesson), ContentId::of(bytes)));
        }
        used.check(&manifest)?;
        Ok(Pack {
            manifest,
            lessons,
            patches,
            texts,
        })
    }

    /// The manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The pack's id.
    pub fn id(&self) -> &PackId {
        &self.manifest.id
    }

    /// The lessons, in manifest order.
    pub fn lessons(&self) -> impl Iterator<Item = &Lesson> {
        self.lessons.iter().map(|(l, _)| l.as_ref())
    }

    /// A lesson by id.
    pub fn lesson(&self, id: &LessonId) -> Option<&Lesson> {
        self.lessons
            .iter()
            .find(|(l, _)| &l.id == id)
            .map(|(l, _)| l.as_ref())
    }

    /// A loaded patch.
    pub fn patch(&self, r: &PatchRef) -> Option<&Arc<Patch>> {
        self.patches.get(r)
    }

    /// A lesson with everything the player needs: its patches and texts, its pack's id and version, its content id.
    pub fn bundle(&self, id: &LessonId) -> Option<LessonBundle> {
        let (lesson, content) = self.lessons.iter().find(|(l, _)| &l.id == id)?;
        let patches = lesson
            .patches()
            .filter_map(|r| self.patches.get(r).map(|p| (r.clone(), Arc::clone(p))))
            .collect();
        let texts = lesson
            .texts()
            .filter_map(|t| {
                self.texts
                    .get(&t.path_in(id))
                    .map(|s| (t.clone(), Arc::clone(s)))
            })
            .collect();
        Some(LessonBundle {
            pack: self.manifest.id.clone(),
            version: self.manifest.version,
            content: *content,
            lesson: Arc::clone(lesson),
            patches,
            texts,
        })
    }
}

/// One lesson ready to play: the lesson, its pack's id and version, its content id, and the patches and texts it
/// names. The shell builds it from a [`Pack`] and hands it to the session, which builds the
/// [`Player`](crate::player::Player) with it.
#[derive(Clone, Debug, PartialEq)]
pub struct LessonBundle {
    /// The pack.
    pub pack: PackId,
    /// The pack's version.
    pub version: PackVersion,
    /// The lesson file's content id.
    pub content: ContentId,
    /// The lesson.
    pub lesson: Arc<Lesson>,
    /// Every patch the lesson names.
    pub patches: BTreeMap<PatchRef, Arc<Patch>>,
    /// Every text the lesson names.
    pub texts: BTreeMap<TextRef, Arc<str>>,
}

impl LessonBundle {
    /// `pack/lesson`.
    pub fn key(&self) -> LessonKey {
        LessonKey {
            pack: self.pack.clone(),
            lesson: self.lesson.id.clone(),
        }
    }

    /// The reference the session saves, at `step`.
    pub fn lesson_ref(&self, step: u16) -> LessonRef {
        LessonRef {
            pack: self.pack.clone(),
            version: self.version,
            lesson: self.lesson.id.clone(),
            step,
            content: self.content,
        }
    }

    /// A patch the lesson names.
    pub fn patch(&self, r: &PatchRef) -> Option<&Arc<Patch>> {
        self.patches.get(r)
    }

    /// A text the lesson names.
    pub fn text(&self, t: &TextRef) -> Option<&str> {
        self.texts.get(t).map(AsRef::as_ref)
    }
}

/// What a pack's lessons use, to compare with its `requires`.
#[derive(Default)]
struct Used {
    lesson: u32,
    patch: u32,
    challenge: bool,
    generator: Option<u16>,
    scorers: BTreeSet<ScorerId>,
}

impl Used {
    fn note(&mut self, lesson: &Lesson) {
        for step in &lesson.steps {
            if let StepKind::Recreate { challenge, .. } = &step.kind {
                self.challenge = true;
                self.generator = self.generator.max(challenge.generator());
                let s = &challenge.scoring;
                for id in [&s.sound, &s.structure, &s.combined, &s.time] {
                    self.scorers.insert(id.clone());
                }
            }
        }
    }

    fn check(&self, m: &Manifest) -> Result<(), LearnError> {
        let r = &m.requires;
        let over = |what: String| bad_field(Doc::Pack, "requires", what);
        if self.lesson > r.lesson {
            return Err(over(format!(
                "says lesson {}, a lesson uses {}",
                r.lesson, self.lesson
            )));
        }
        if self.patch > r.patch {
            return Err(over(format!(
                "says patch {}, a patch uses {}",
                r.patch, self.patch
            )));
        }
        if self.challenge && r.challenge.is_none() {
            return Err(over(
                "names no challenge schema, and a lesson has a Recreate step".to_owned(),
            ));
        }
        if let Some(g) = self.generator
            && r.generator.is_none_or(|have| have < g)
        {
            return Err(over(format!("a challenge uses generator {g}")));
        }
        if let Some(s) = self.scorers.iter().find(|s| !r.scorers.contains(s)) {
            return Err(over(format!(
                "a challenge names scorer {s}, which requires.scorers lacks"
            )));
        }
        Ok(())
    }
}

/// Every file listed and every listed file present, with its size and hash.
fn verify_files(manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> Result<(), LearnError> {
    for path in files.keys().filter(|p| p.as_str() != MANIFEST_PATH) {
        PackPath::new(path)?;
        if manifest.file(path).is_none() {
            return Err(LearnError::Unlisted { path: path.clone() });
        }
    }
    for entry in &manifest.files {
        let path = entry.path.as_str();
        let bytes = files.get(path).ok_or_else(|| LearnError::Missing {
            path: path.to_owned(),
        })?;
        if bytes.len() as u64 != entry.size {
            return Err(LearnError::SizeMismatch {
                path: path.to_owned(),
                listed: entry.size,
                found: bytes.len() as u64,
            });
        }
        if sha256(bytes) != entry.sha256.0 {
            return Err(LearnError::HashMismatch {
                path: path.to_owned(),
            });
        }
    }
    Ok(())
}

fn load_patch(path: &str, bytes: &[u8]) -> Result<enth_patch::file::Loaded, LearnError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LearnError::BadContent {
        path: path.to_owned(),
        message: "not UTF-8".to_owned(),
    })?;
    enth_patch::file::load(text).map_err(|e| LearnError::BadContent {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

/// Name the pack in a lesson's `lesson_too_new`.
fn with_pack(e: LearnError, m: &Manifest) -> LearnError {
    match e {
        LearnError::TooNew {
            doc: Doc::Lesson,
            found,
            supported,
            ..
        } => LearnError::TooNew {
            doc: Doc::Lesson,
            found,
            supported,
            name: m.name(),
        },
        other => other,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shared helpers (schema, hashing, hex)
// ---------------------------------------------------------------------------------------------------------------

/// A pack's name read leniently from its JSON (for `pack_too_new`).
pub(crate) fn lenient_name(value: &Value) -> PackName {
    let get = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    PackName {
        id: get("id"),
        version: get("version"),
        title: get("title"),
    }
}

/// The version of a document's `schema` (`<family>/<N>`, `N ≥ 1`).
pub(crate) fn schema_version(doc: Doc, value: &Value) -> Result<u32, LearnError> {
    let schema = value
        .as_object()
        .ok_or_else(|| fields(doc, format!("a {} file is a JSON object", doc.name())))?
        .get("schema")
        .ok_or(LearnError::MissingSchema { doc })?;
    let bad = || LearnError::BadSchema {
        doc,
        found: schema.to_string(),
    };
    let n = schema
        .as_str()
        .and_then(|t| t.strip_prefix(doc.family()))
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(bad)?;
    if n.is_empty() || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    n.parse().map_err(|_| bad())
}

/// [`schema_version`], refusing a version above `supported` as too new (naming a pack from its JSON).
pub(crate) fn check_schema(doc: Doc, value: &Value, supported: u32) -> Result<u32, LearnError> {
    let found = schema_version(doc, value)?;
    if found > supported {
        return Err(LearnError::TooNew {
            doc,
            found,
            supported,
            name: if doc == Doc::Pack {
                lenient_name(value)
            } else {
                PackName::default()
            },
        });
    }
    Ok(found)
}

pub(crate) fn syntax(doc: Doc, message: String) -> LearnError {
    LearnError::BadSyntax {
        doc,
        path: None,
        message,
    }
}

pub(crate) fn fields(doc: Doc, message: String) -> LearnError {
    LearnError::BadFields {
        doc,
        path: None,
        message,
    }
}

/// The first duplicate of `items`, as an error.
pub(crate) fn unique<'a, T: Ord + 'a>(
    items: impl Iterator<Item = &'a T>,
    err: impl Fn(&T) -> LearnError,
) -> Result<(), LearnError> {
    let mut seen = BTreeSet::new();
    for i in items {
        if !seen.insert(i) {
            return Err(err(i));
        }
    }
    Ok(())
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

/// Lower-case hex into `out`, exactly `2 · out.len()` digits.
fn unhex(text: &str, out: &mut [u8]) -> Option<()> {
    let b = text.as_bytes();
    if b.len() != out.len() * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let (pairs, _) = b.as_chunks::<2>();
    for (o, pair) in out.iter_mut().zip(pairs) {
        *o = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_their_grammar() {
        assert!(PackId::new("space-and-dynamics").is_ok());
        assert!(PackId::new("1basics").is_err());
        assert!(PackId::new("Basics").is_err());
        assert!(PackId::new("a--b").is_err());
        assert!(PackId::new("trailing-").is_err());
        assert!(LessonId::new("2-1-cutoff").is_ok());
        assert!(LessonId::new(&"a".repeat(49)).is_err());
        assert!(StepId::new("do-1").is_ok());
    }

    #[test]
    fn versions_are_strict_semver_cores() {
        assert_eq!(
            "1.2.0".parse::<PackVersion>(),
            Ok(PackVersion::new(1, 2, 0))
        );
        for bad in ["1.2", "1.2.0.0", "01.2.0", "1.2.0-beta", "", "a.b.c"] {
            assert!(bad.parse::<PackVersion>().is_err(), "{bad}");
        }
        assert!(PackVersion::new(1, 10, 0) > PackVersion::new(1, 9, 9));
    }

    #[test]
    fn keys_and_content_ids_round_trip() {
        let k: LessonKey = "basics/cutoff".parse().unwrap();
        assert_eq!(k.to_string(), "basics/cutoff");
        assert!("basics".parse::<LessonKey>().is_err());
        let c = ContentId::of(b"x");
        assert_eq!(c.to_string().parse::<ContentId>(), Ok(c));
        assert!("ABCDEF0123456789".parse::<ContentId>().is_err());
    }

    #[test]
    fn pack_paths_keep_to_their_places() {
        for ok in [
            "CREDITS.md",
            "lessons/cutoff/lesson.json",
            "lessons/cutoff/step-1.md",
            "patches/saw.patch.json",
        ] {
            assert!(PackPath::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "../evil.txt",
            "/abs/x.md",
            "lessons/cutoff/../x.md",
            "lessons//x.md",
            "lessons/cutoff/.hidden.md",
            "lessons/Cutoff/lesson.json",
            "lessons/cutoff/notes.txt",
            "patches/x.json",
            "patches/sub/x.patch.json",
            "media/a.wav",
            "pack.json",
            "README.md",
            "lessons\\cutoff\\lesson.json",
        ] {
            assert_eq!(
                PackPath::new(bad).map_err(|e| e.code()),
                Err("bad_path"),
                "{bad}"
            );
        }
    }
}
