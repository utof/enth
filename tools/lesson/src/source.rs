//! **Owner: lane LCK.** Reading packs for `check`, `coverage` and `pack`: the authoring layout
//! (`content/lessons/<pack>/`, TOML) built into the files of a pack, or a built `.enthpack`.
//!
//! **The build** is what `cargo xtask pack` ships (docs/learn.md, "Packs"): each `lesson.toml` becomes `lesson.json`
//! through [`Lesson::to_json`] (never a copy of the authoring text: lane PLY's grammar reads predicate lines that the
//! frozen shape writes as objects), the texts, patches and credits are copied, and the manifest is sealed
//! ([`Manifest::seal`]: every file with its size and SHA-256, the engine stamped) into `pack.json`.
//!
//! **A pack directory holds only what its manifest lists** (lane C1's rule): `pack.toml`, `CREDITS.md`,
//! `lessons/<id>/lesson.toml` and `lessons/<id>/<name>.md` for each lesson in `pack.toml`'s `lessons`, and
//! `patches/<name>.patch.json`. Anything else is refused (a stray file would either ship unlisted or be dropped
//! silently), and so is a text or patch no lesson names (it would ship and never show).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::pack::{CREDITS_PATH, LessonId, MANIFEST_PATH, lesson_path};
use enth_learn::{LearnError, Lesson, Manifest};

/// The authoring manifest's name.
pub(crate) const PACK_TOML: &str = "pack.toml";
/// A lesson's authoring file.
pub(crate) const LESSON_TOML: &str = "lesson.toml";
/// Where the packs live, under the repository root.
pub(crate) const LESSONS_DIR: &str = "content/lessons";

/// The repository root: two levels above this package (`tools/lesson`), where the binary was built.
pub(crate) fn repo_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = here.join("../..");
    root.canonicalize().unwrap_or(root)
}

/// A refusal of a learn type as one line: its code, the message and the hint.
pub(crate) fn learn_error(e: &LearnError) -> String {
    format!("{} ({}): {}", e, e.code(), e.hint())
}

/// Every pack directory under `content/lessons/` (a directory with a `pack.toml`), by name.
pub(crate) fn pack_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join(LESSONS_DIR);
    let mut out = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.is_dir() && path.join(PACK_TOML).is_file() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// The packs the arguments name: a pack id under `content/lessons/`, a pack directory, or an `.enthpack` file; none
/// means every pack under `content/lessons/`.
pub(crate) fn resolve(root: &Path, names: &[String]) -> Result<Vec<PathBuf>, String> {
    if names.is_empty() {
        return pack_dirs(root);
    }
    names
        .iter()
        .map(|name| {
            let as_path = PathBuf::from(name);
            if as_path.exists() {
                return Ok(as_path);
            }
            let under = root.join(LESSONS_DIR).join(name);
            if under.join(PACK_TOML).is_file() {
                Ok(under)
            } else {
                Err(format!(
                    "{name}: no such pack (neither a path nor a directory under {LESSONS_DIR}/ with a {PACK_TOML})"
                ))
            }
        })
        .collect()
}

/// A pack read from disk, built into its files.
#[derive(Debug)]
pub(crate) struct Source {
    /// Its files by path, `pack.json` included: what the `.enthpack` holds.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Read a pack: a directory in the authoring layout, or an `.enthpack` file (its files taken out by the reader, every
/// entry rule applied). `Err` holds every problem found, one line each.
pub(crate) fn read(path: &Path) -> Result<Source, Vec<String>> {
    if path.is_dir() {
        Ok(Source {
            files: build_dir(path)?,
        })
    } else {
        let bytes =
            fs::read(path).map_err(|e| vec![format!("cannot read {}: {e}", path.display())])?;
        let files = enth_learn::reader::extract(&bytes).map_err(|e| vec![learn_error(&e)])?;
        Ok(Source { files })
    }
}

/// Build a pack directory into its files (see the [module docs](self)).
pub(crate) fn build_dir(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, Vec<String>> {
    let mut problems = Vec::new();
    let toml = match fs::read_to_string(dir.join(PACK_TOML)) {
        Ok(t) => t,
        Err(e) => return Err(vec![format!("{PACK_TOML}: cannot read it: {e}")]),
    };
    let manifest = match Manifest::from_toml(&toml) {
        Ok(m) => m,
        Err(e) => return Err(vec![format!("{PACK_TOML}: {}", learn_error(&e))]),
    };
    let ids: BTreeSet<&str> = manifest.lessons.iter().map(LessonId::as_str).collect();

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut seen_lessons: BTreeSet<String> = BTreeSet::new();
    let mut lessons: Vec<Lesson> = Vec::new();
    for rel in walk(dir, &mut problems) {
        let path = dir.join(&rel);
        let parts: Vec<&str> = rel.split('/').collect();
        let read = |problems: &mut Vec<String>| match fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) => {
                problems.push(format!("{rel}: cannot read it: {e}"));
                None
            }
        };
        match parts.as_slice() {
            [PACK_TOML] => {}
            [CREDITS_PATH] => {
                if let Some(b) = read(&mut problems) {
                    files.insert(rel.clone(), b);
                }
            }
            ["patches", name] if name.ends_with(".patch.json") => {
                if let Some(b) = read(&mut problems) {
                    files.insert(rel.clone(), b);
                }
            }
            ["lessons", id, file] if ids.contains(id) => {
                if *file == LESSON_TOML {
                    seen_lessons.insert((*id).to_owned());
                    let Some(bytes) = read(&mut problems) else {
                        continue;
                    };
                    let Ok(text) = String::from_utf8(bytes) else {
                        problems.push(format!("{rel}: not UTF-8"));
                        continue;
                    };
                    match Lesson::from_toml(&text) {
                        Ok(lesson) => {
                            if lesson.id.as_str() == *id {
                                // The frozen shape, written by the type: never the authoring text (lane PLY's rule).
                                files.insert(format!("lessons/{id}/lesson.json"), lesson.to_json().into_bytes());
                                lessons.push(lesson);
                            } else {
                                problems.push(format!(
                                    "{rel}: its id is {}, its directory says {id}",
                                    lesson.id
                                ));
                            }
                        }
                        Err(e) => problems.push(format!("{rel}: {}", learn_error(&e))),
                    }
                } else if Path::new(file).extension().is_some_and(|e| e == "md") {
                    if let Some(b) = read(&mut problems) {
                        files.insert(rel.clone(), b);
                    }
                } else {
                    problems.push(stray(&rel));
                }
            }
            ["lessons", id, ..] if !ids.contains(id) => problems.push(format!(
                "{rel}: lessons/{id}/ is not in {PACK_TOML}'s `lessons` (add it there, or move the directory out)"
            )),
            _ => problems.push(stray(&rel)),
        }
    }
    for id in &manifest.lessons {
        if !seen_lessons.contains(id.as_str()) {
            problems.push(format!(
                "{}: missing ({PACK_TOML} lists the lesson {id})",
                format_args!("lessons/{id}/{LESSON_TOML}")
            ));
        }
    }
    if !files.contains_key(CREDITS_PATH) {
        problems.push(format!(
            "{CREDITS_PATH}: missing (every pack credits its authors and licences)"
        ));
    }

    // Every text and patch is named by a lesson: an unused file would ship and never show.
    let mut used: BTreeSet<String> = BTreeSet::from([CREDITS_PATH.to_owned()]);
    for lesson in &lessons {
        used.insert(lesson_path(&lesson.id));
        used.extend(lesson.texts().map(|t| t.path_in(&lesson.id)));
        used.extend(lesson.patches().map(|p| p.as_str().to_owned()));
    }
    for path in files.keys() {
        if !used.contains(path) {
            problems.push(format!(
                "{path}: no lesson names it (an unused file ships and never shows)"
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    match manifest.seal(&files) {
        Ok(sealed) => {
            files.insert(MANIFEST_PATH.to_owned(), sealed.to_json().into_bytes());
            Ok(files)
        }
        Err(e) => Err(vec![format!("sealing the manifest: {}", learn_error(&e))]),
    }
}

fn stray(rel: &str) -> String {
    format!(
        "{rel}: not a pack file (a pack directory holds only {PACK_TOML}, {CREDITS_PATH}, lessons/<id>/{LESSON_TOML}, \
         lessons/<id>/<name>.md and patches/<name>.patch.json)"
    )
}

/// Every file under `dir`, relative and `/`-separated, sorted. Unreadable directories are problems.
fn walk(dir: &Path, problems: &mut Vec<String>) -> Vec<String> {
    fn go(base: &Path, dir: &Path, out: &mut Vec<String>, problems: &mut Vec<String>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                problems.push(format!("cannot read {}: {e}", dir.display()));
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                go(base, &path, out, problems);
            } else if let Ok(rel) = path.strip_prefix(base) {
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out, problems);
    out.sort();
    out
}
