//! **Owner: lane LM** (the lesson model, freeze point F1l). The reference is docs/learn.md, "The lesson model (lane
//! LM)".
//!
//! [`LearnError`]: every refusal of this crate, each with a stable snake-case [`code`](LearnError::code) and a
//! one-line [`hint`](LearnError::hint) the shell shows, as `ChallengeError` does. Packs and lessons are **strict
//! inputs**: a newer schema is refused and named (`pack_too_new`, `lesson_too_new`), unknown fields are refused, and
//! the `.enthpack` reader refuses an entry whose normalised name differs from its raw name, duplicate, oversize and
//! unlisted entries, and inflation past the listed size. Progress is a **lenient output** and is refused only when
//! it is not JSON at all (`progress.rs`). The player's own errors (a stale result, a call in the wrong phase) are
//! [`PlayerError`](crate::player::PlayerError).

use core::fmt;

/// Which document a refusal is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Doc {
    /// A pack manifest, `pack.json` (`enth.pack/1`) or the authoring `pack.toml`.
    Pack,
    /// A lesson, `lesson.json` (`enth.lesson/1`) or the authoring `lesson.toml`.
    Lesson,
    /// The per-user progress file, `progress.json` (`enth.progress/1`).
    Progress,
}

impl Doc {
    /// `pack`, `lesson`, `progress`.
    pub const fn name(self) -> &'static str {
        match self {
            Doc::Pack => "pack",
            Doc::Lesson => "lesson",
            Doc::Progress => "progress",
        }
    }

    /// The schema family: `enth.pack`, `enth.lesson`, `enth.progress`.
    pub const fn family(self) -> &'static str {
        match self {
            Doc::Pack => "enth.pack",
            Doc::Lesson => "enth.lesson",
            Doc::Progress => "enth.progress",
        }
    }
}

/// What a refused newer pack says about itself, read leniently from its JSON before the strict decode, so the shell
/// can list it by name ("Synth basics 2.0.0 needs a newer enth") instead of hiding it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackName {
    /// Its `id`, when it is a string.
    pub id: Option<String>,
    /// Its `version`, when it is a string.
    pub version: Option<String>,
    /// Its `title`, when it is a string.
    pub title: Option<String>,
}

impl fmt::Display for PackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .title
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or("a pack");
        f.write_str(name)?;
        if let Some(v) = &self.version {
            write!(f, " {v}")?;
        }
        Ok(())
    }
}

/// What a pack needs that this build lacks ([`LearnError::Requires`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    /// A lesson schema newer than this build's.
    Lesson(u32),
    /// A challenge schema newer than this build's.
    Challenge(u32),
    /// A patch schema newer than this build's.
    Patch(u32),
    /// A generator version this build does not run.
    Generator(u16),
    /// A scorer this build does not have.
    Scorer(String),
}

impl fmt::Display for Need {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Need::Lesson(v) => write!(f, "lesson schema {v}"),
            Need::Challenge(v) => write!(f, "challenge schema {v}"),
            Need::Patch(v) => write!(f, "patch schema {v}"),
            Need::Generator(v) => write!(f, "generator {v}"),
            Need::Scorer(id) => write!(f, "scorer {id}"),
        }
    }
}

/// Why a pack, a lesson, an archive entry or a progress file is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LearnError {
    // --- documents -------------------------------------------------------------------------------------------
    /// Not JSON (or not TOML, for an authoring file).
    BadSyntax {
        /// The document.
        doc: Doc,
        /// Where it came from (a pack path), when known.
        path: Option<String>,
        /// The parser's message.
        message: String,
    },
    /// No `schema` field.
    MissingSchema {
        /// The document.
        doc: Doc,
    },
    /// A `schema` that is not `<family>/<N>` with `N ≥ 1`.
    BadSchema {
        /// The document.
        doc: Doc,
        /// What it was, as JSON.
        found: String,
    },
    /// A pack or lesson schema newer than this build reads: `pack_too_new`, `lesson_too_new`. Listed, never hidden.
    TooNew {
        /// The document.
        doc: Doc,
        /// Its version.
        found: u32,
        /// The newest this build reads.
        supported: u32,
        /// The pack's own name, read leniently (for a lesson: the pack it is in, when the reader knows it).
        name: PackName,
    },
    /// The right schema, but a missing, unknown or mistyped field.
    BadFields {
        /// The document.
        doc: Doc,
        /// Where it came from (a pack path), when known.
        path: Option<String>,
        /// The decoder's message.
        message: String,
    },
    /// A field that decodes but breaks a rule the types cannot hold (an id's grammar, a duplicate step id, an answer
    /// past the options, a reference to a file the pack does not list).
    BadField {
        /// The document.
        doc: Doc,
        /// The field (`steps[2].answer`).
        field: String,
        /// What is wrong.
        message: String,
    },
    /// The pack needs something this build lacks (its `requires`, or what its lessons use beyond it).
    Requires {
        /// The pack.
        name: PackName,
        /// What it needs.
        need: Need,
    },
    /// A pack made with a newer engine than this build's failed on its content: most likely on something this build
    /// lacks (a module kind, a parameter key, a phrase, a newer patch or an inline patch schema in a challenge), so it
    /// is named as needing a newer enth rather than as damaged (F1l review should-fix 6, lane PLY). The refusal it
    /// met is kept as `cause`. Integrity failures (a hash, a size, the archive) are never turned into this.
    NeedsNewer {
        /// The pack.
        name: PackName,
        /// The engine it records (`engine`), newer than this build's.
        engine: String,
        /// What failed.
        cause: Box<LearnError>,
    },
    // --- the archive and its files ---------------------------------------------------------------------------
    /// Not a zip archive, or a damaged one.
    BadArchive {
        /// `rawzip`'s message.
        message: String,
    },
    /// More entries than a pack may have.
    TooManyEntries {
        /// How many the archive declares.
        found: u64,
        /// The most a pack may have ([`MAX_ENTRIES`](crate::reader::MAX_ENTRIES)).
        max: u64,
    },
    /// An entry whose normalised name differs from its raw name (`rawzip` turns `../../evil.txt` into `evil.txt`),
    /// or whose local header names another file than its central-directory record.
    RenamedEntry {
        /// The raw name, lossily decoded.
        raw: String,
    },
    /// Two entries with one name.
    DuplicateEntry {
        /// The name.
        path: String,
    },
    /// A file larger than a pack file may be, or a pack larger than a pack may be.
    TooLarge {
        /// The file (or `*` for the whole pack).
        path: String,
        /// Its size in bytes.
        size: u64,
        /// The limit.
        max: u64,
    },
    /// An entry that is neither stored nor deflated.
    Compression {
        /// The entry.
        path: String,
        /// The zip method number.
        method: u16,
    },
    /// An encrypted entry.
    Encrypted {
        /// The entry.
        path: String,
    },
    /// An entry that inflates past the size the manifest lists for it (a zip bomb, or a lie).
    InflateOverflow {
        /// The entry.
        path: String,
        /// The listed size.
        listed: u64,
    },
    /// An entry whose data does not inflate, or fails its CRC.
    Corrupt {
        /// The entry.
        path: String,
        /// What failed.
        message: String,
    },
    /// A file the manifest does not list.
    Unlisted {
        /// The file.
        path: String,
    },
    /// A file the manifest lists that the pack does not hold.
    Missing {
        /// The file.
        path: String,
    },
    /// A file whose size is not the listed one.
    SizeMismatch {
        /// The file.
        path: String,
        /// The listed size.
        listed: u64,
        /// Its size.
        found: u64,
    },
    /// A file whose SHA-256 is not the listed one.
    HashMismatch {
        /// The file.
        path: String,
    },
    /// A path that breaks the pack's path rules (relative, `/`-separated, no `.` or `..`, a known top-level place).
    BadPath {
        /// The path.
        path: String,
        /// Which rule.
        why: &'static str,
    },
    /// A pack file that is not what its place says: a patch that does not load, a text that is not UTF-8.
    BadContent {
        /// The file.
        path: String,
        /// What failed (a patch's codes and hints).
        message: String,
    },
    // --- stores -----------------------------------------------------------------------------------------------
    /// The store has no such pack.
    NotFound {
        /// The key, as text.
        key: String,
    },
    /// The store failed to read or write (the shell's message).
    Store {
        /// The shell's message.
        message: String,
    },
}

impl LearnError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            LearnError::BadSyntax { .. } => "bad_json",
            LearnError::MissingSchema { .. } => "missing_schema",
            LearnError::BadSchema { .. } => "bad_schema",
            LearnError::TooNew { doc, .. } => match doc {
                Doc::Pack => "pack_too_new",
                Doc::Lesson => "lesson_too_new",
                Doc::Progress => "progress_too_new",
            },
            LearnError::BadFields { doc, .. } => match doc {
                Doc::Pack => "bad_pack",
                Doc::Lesson => "bad_lesson",
                Doc::Progress => "bad_progress",
            },
            LearnError::BadField { .. } => "bad_field",
            LearnError::Requires { .. } => "requires_unmet",
            LearnError::NeedsNewer { .. } => "needs_newer_enth",
            LearnError::BadArchive { .. } => "bad_archive",
            LearnError::TooManyEntries { .. } => "too_many_entries",
            LearnError::RenamedEntry { .. } => "renamed_entry",
            LearnError::DuplicateEntry { .. } => "duplicate_entry",
            LearnError::TooLarge { .. } => "too_large",
            LearnError::Compression { .. } => "unsupported_compression",
            LearnError::Encrypted { .. } => "encrypted_entry",
            LearnError::InflateOverflow { .. } => "inflate_overflow",
            LearnError::Corrupt { .. } => "corrupt_entry",
            LearnError::Unlisted { .. } => "unlisted_file",
            LearnError::Missing { .. } => "missing_file",
            LearnError::SizeMismatch { .. } => "size_mismatch",
            LearnError::HashMismatch { .. } => "hash_mismatch",
            LearnError::BadPath { .. } => "bad_path",
            LearnError::BadContent { .. } => "bad_content",
            LearnError::NotFound { .. } => "not_found",
            LearnError::Store { .. } => "store_failed",
        }
    }

    /// Whether the fix is a newer enth rather than a repaired pack: a newer schema (`pack_too_new`,
    /// `lesson_too_new`), a `requires` this build lacks, or a newer pack's content (`needs_newer_enth`). The browser
    /// lists these as "needs a newer enth", never as damaged.
    pub fn needs_newer_enth(&self) -> bool {
        matches!(
            self,
            LearnError::TooNew { .. } | LearnError::Requires { .. } | LearnError::NeedsNewer { .. }
        )
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            LearnError::BadSyntax { doc, .. } | LearnError::BadFields { doc, .. } => match doc {
                Doc::Pack | Doc::Lesson => format!(
                    "the {} file is damaged or was written by hand with a mistake: rebuild the pack with `cargo xtask pack`",
                    doc.name()
                ),
                Doc::Progress => "the progress file is damaged: keep a copy and start a new one".to_owned(),
            },
            LearnError::MissingSchema { doc } | LearnError::BadSchema { doc, .. } => {
                format!("a {} file starts with \"schema\": \"{}/1\"", doc.name(), doc.family())
            }
            LearnError::TooNew { doc, .. } => match doc {
                Doc::Pack => "this pack needs a newer enth: update enth to open it".to_owned(),
                _ => format!("this {} needs a newer enth: update enth to open it", doc.name()),
            },
            LearnError::BadField { field, .. } => format!("fix {field} in the source and rebuild the pack"),
            LearnError::Requires { .. } => "this pack needs a newer enth: update enth to open it".to_owned(),
            LearnError::NeedsNewer { engine, .. } => format!(
                "this pack needs a newer enth: it was made with engine {engine}, this build has {}; update enth to open it",
                enth_patch::file::ENGINE_VERSION
            ),
            LearnError::BadArchive { .. } | LearnError::Corrupt { .. } => {
                "the pack file is damaged: download or copy it again".to_owned()
            }
            LearnError::TooManyEntries { .. } | LearnError::TooLarge { .. } => {
                "this is not an enth pack, or a damaged one: packs are small (lessons, patches and text)".to_owned()
            }
            LearnError::RenamedEntry { .. }
            | LearnError::DuplicateEntry { .. }
            | LearnError::InflateOverflow { .. }
            | LearnError::Unlisted { .. }
            | LearnError::Encrypted { .. }
            | LearnError::Compression { .. } => {
                "the pack was not built by `cargo xtask pack` or was changed after: rebuild it, and only install packs from people you trust".to_owned()
            }
            LearnError::Missing { .. } | LearnError::SizeMismatch { .. } | LearnError::HashMismatch { .. } => {
                "the pack's files do not match its manifest: rebuild it with `cargo xtask pack`".to_owned()
            }
            LearnError::BadPath { .. } => {
                "pack paths are relative, use `/`, and live under lessons/, patches/ or CREDITS.md".to_owned()
            }
            LearnError::BadContent { .. } => {
                "check the file with `cargo xtask lesson-check` and rebuild the pack".to_owned()
            }
            LearnError::NotFound { .. } => "rescan the packs folder".to_owned(),
            LearnError::Store { .. } => "check that the packs folder exists and can be written".to_owned(),
        }
    }
}

impl fmt::Display for LearnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LearnError::BadSyntax { doc, path, message }
            | LearnError::BadFields { doc, path, message } => match path {
                Some(p) => write!(f, "{} {p}: {message}", doc.name()),
                None => write!(f, "{}: {message}", doc.name()),
            },
            LearnError::MissingSchema { doc } => write!(f, "the {} has no schema", doc.name()),
            LearnError::BadSchema { doc, found } => {
                write!(f, "schema {found} is not a {}", doc.name())
            }
            LearnError::TooNew {
                doc,
                found,
                supported,
                name,
            } => write!(
                f,
                "{name}: {} version {found} is newer than this build's {supported}",
                doc.name()
            ),
            LearnError::BadField {
                doc,
                field,
                message,
            } => write!(f, "{} {field}: {message}", doc.name()),
            LearnError::NeedsNewer {
                name,
                engine,
                cause,
            } => write!(
                f,
                "{name} needs a newer enth (made with engine {engine}, this build has {}): {cause}",
                enth_patch::file::ENGINE_VERSION
            ),
            LearnError::Requires { name, need } => {
                write!(f, "{name} needs {need}, which this build lacks")
            }
            LearnError::BadArchive { message } => {
                write!(f, "not a readable pack archive: {message}")
            }
            LearnError::TooManyEntries { found, max } => {
                write!(
                    f,
                    "the archive declares {found} entries; a pack holds at most {max}"
                )
            }
            LearnError::RenamedEntry { raw } => {
                write!(
                    f,
                    "entry {raw:?} is not a plain relative name (it would be renamed on reading)"
                )
            }
            LearnError::DuplicateEntry { path } => write!(f, "{path} appears twice in the archive"),
            LearnError::TooLarge { path, size, max } => {
                write!(f, "{path} is {size} bytes; the limit is {max}")
            }
            LearnError::Compression { path, method } => {
                write!(
                    f,
                    "{path} uses zip method {method}; packs use stored or deflate only"
                )
            }
            LearnError::Encrypted { path } => write!(f, "{path} is encrypted"),
            LearnError::InflateOverflow { path, listed } => {
                write!(f, "{path} inflates past its listed {listed} bytes")
            }
            LearnError::Corrupt { path, message } | LearnError::BadContent { path, message } => {
                write!(f, "{path}: {message}")
            }
            LearnError::Unlisted { path } => write!(f, "{path} is not listed in the manifest"),
            LearnError::Missing { path } => {
                write!(f, "{path} is listed in the manifest but missing")
            }
            LearnError::SizeMismatch {
                path,
                listed,
                found,
            } => {
                write!(f, "{path} is {found} bytes; the manifest lists {listed}")
            }
            LearnError::HashMismatch { path } => {
                write!(f, "{path} does not match its listed SHA-256")
            }
            LearnError::BadPath { path, why } => write!(f, "path {path:?}: {why}"),
            LearnError::NotFound { key } => write!(f, "no pack {key}"),
            LearnError::Store { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for LearnError {}

/// A [`LearnError::BadField`] for `doc`.
pub(crate) fn bad_field(
    doc: Doc,
    field: impl Into<String>,
    message: impl Into<String>,
) -> LearnError {
    LearnError::BadField {
        doc,
        field: field.into(),
        message: message.into(),
    }
}
