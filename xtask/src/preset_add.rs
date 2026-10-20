//! **Owner: lane PR** (factory presets; it follows lane PBC's preset format, docs/presets.md).
//!
//! `cargo xtask preset-add` turns a patch an agent drafted through the CLI (pretty-printed, unnamed, untagged) into
//! a preset file in the exact form `crates/enth-plugin/tests/presets.rs` requires, and promotes a picked candidate
//! from `content/presets-candidates/` into the factory folder `content/presets/`. It sits beside `extract-presets`
//! (`preset.rs`, the owner's sounds from REAPER) without editing it.
//!
//! ```text
//! cargo xtask preset-add draft.patch.json --name "Acid Snap" --tags bass,gritty,mono   # into content/presets-candidates/bass/
//! cargo xtask preset-add --promote "Acid Snap"                                         # candidate -> content/presets/bass/
//! cargo xtask preset-add --picks content/presets-candidates/PICKS.md                   # every row marked yes
//! ```
//!
//! **Adding** loads the file (migrating an older schema), sets `meta.name`, `meta.tags` (and `meta.description`
//! when given), stamps the current [`ENGINE_VERSION`](enth_patch::file::ENGINE_VERSION), checks the factory rules
//! (a name of at most 64 characters, exactly one type tag, at most four character tags, vocabulary words only, no
//! repeats) and writes exactly `Patch::to_json()` (compact, no trailing newline) to `<out>/<folder>/<slug>.patch.json`.
//! `--out` defaults to the candidates folder, so a draft never ships by accident; `--folder` defaults to the type tag
//! (docs/presets.md: "Keep the factory folders few and by type"); `.` means the top of `--out`. No two presets under
//! `--out` may share a name, ignoring case (the factory test's rule, applied to whichever folder is written). The
//! written file is read back and checked the way the factory test checks it.
//!
//! **Promoting** finds a candidate by its name (ignoring case) or file stem, checks it is already in preset form, and
//! writes it to `--to` (default `content/presets`) under its type folder with the same rules. `--name` renames it on
//! the way. `--move` removes the source file after a successful write (also for adding, e.g. to rename an owner's
//! sound in place). `--picks FILE` promotes every row of a `PICKS.md` table whose "Keep" cell is `yes`.
//!
//! **The vocabulary is a mirror.** xtask stays light (no engine, session or GUI crate, S0r's rule in
//! `xtask/Cargo.toml`), and adding a dependency would edit `Cargo.lock`, so [`vocab`] copies the lists in
//! `enth_session::presets::tags`; a test reads `crates/enth-session/src/presets.rs` and fails when they differ.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use enth_patch::Patch;
use enth_patch::file::{ENGINE_VERSION, SCHEMA};

use crate::preset::{PRESET_DIR, slug};

/// Where drafts wait for the owner's pick. Not shipped: only `content/presets/` is embedded.
pub const CANDIDATE_DIR: &str = "content/presets-candidates";

/// The suffix of a preset file (`enth_session::presets::FILE_SUFFIX`).
pub const FILE_SUFFIX: &str = ".patch.json";

const USAGE: &str = "usage:
  cargo xtask preset-add <patch.json> [--name NAME] [--tags TYPE,WORD,...] [--description TEXT]
                         [--out DIR] [--folder SUB|.] [--force] [--move] [--dry-run]
  cargo xtask preset-add --promote NAME[,NAME...] [--name NAME] [--from DIR] [--to DIR] [--folder SUB|.]
                         [--force] [--move] [--dry-run]
  cargo xtask preset-add --picks PICKS.md [--from DIR] [--to DIR] [--force] [--move] [--dry-run]
defaults: --out content/presets-candidates, --from content/presets-candidates, --to content/presets,
          --folder the type tag; tags: exactly one type and up to four character words (docs/presets.md)";

/// The factory tag vocabulary: a mirror of `enth_session::presets::tags` (see the module docs for why).
pub mod vocab {
    use enth_patch::PatchMeta;

    /// What kind of sound it is. Exactly one per factory preset.
    pub const TYPES: [&str; 9] = [
        "bass", "lead", "pad", "pluck", "keys", "arp", "fx", "texture", "init",
    ];

    /// How it sounds or moves. Up to [`MAX_CHARACTER`] per factory preset.
    pub const CHARACTER: [&str; 16] = [
        "bright", "dark", "warm", "clean", "gritty", "hollow", "metallic", "noisy", "soft", "hard",
        "smooth", "wide", "mono", "evolving", "rhythmic", "vintage",
    ];

    /// The most character words on one factory preset.
    pub const MAX_CHARACTER: usize = 4;

    /// The longest preset name, in characters.
    pub const MAX_NAME_CHARS: usize = 64;

    /// The type tag of a tag list, when it has exactly one.
    pub fn type_of(tags: &[String]) -> Option<&str> {
        let mut types = tags.iter().filter(|t| TYPES.contains(&t.as_str()));
        match (types.next(), types.next()) {
            (Some(t), None) => Some(t.as_str()),
            _ => None,
        }
    }

    /// What is wrong with a factory preset's metadata, in words; empty if it is fine. The same rules and words as
    /// `enth_session::presets::tags::check_factory`.
    pub fn check(meta: &PatchMeta) -> Vec<String> {
        let mut problems = Vec::new();
        if meta.name.trim().is_empty() {
            problems.push("it has no name".to_owned());
        }
        if meta.name.chars().count() > MAX_NAME_CHARS {
            problems.push(format!(
                "its name is longer than {MAX_NAME_CHARS} characters"
            ));
        }
        let types = meta
            .tags
            .iter()
            .filter(|t| TYPES.contains(&t.as_str()))
            .count();
        if types != 1 {
            problems.push(format!(
                "it needs exactly one type tag ({}), it has {types}",
                TYPES.join(", ")
            ));
        }
        let character = meta
            .tags
            .iter()
            .filter(|t| CHARACTER.contains(&t.as_str()))
            .count();
        if character > MAX_CHARACTER {
            problems.push(format!(
                "it has {character} character tags, the most is {MAX_CHARACTER}"
            ));
        }
        for tag in &meta.tags {
            if !TYPES.contains(&tag.as_str()) && !CHARACTER.contains(&tag.as_str()) {
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

/// The metadata an add sets; `None` keeps the patch's own.
#[derive(Clone, Debug, Default)]
pub struct MetaEdit {
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    pub description: Option<String>,
}

/// Where and how a preset is written.
#[derive(Clone, Debug)]
pub struct Target {
    /// The folder whose names must stay unique (`content/presets`, the candidates folder, a temporary folder).
    pub root: PathBuf,
    /// The subfolder under `root`; `None` is the preset's type tag, `Some("")` the top of `root`.
    pub folder: Option<String>,
    /// Replace a file that exists with other content.
    pub force: bool,
    /// Plan only: check everything, write nothing.
    pub dry_run: bool,
}

/// What a write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    pub name: String,
    /// `new`, `replaced` or `unchanged`.
    pub state: &'static str,
    pub bytes: usize,
}

/// Tags as given on the command line: comma-separated, trimmed, lower case, empty entries dropped; the type tag
/// first, the character words in the order given.
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = text
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    tags.sort_by_key(|t| !vocab::TYPES.contains(&t.as_str()));
    tags
}

/// Normalise a patch document: load it (an older schema migrates), apply `edit`, stamp the current engine version and
/// check the factory rules. Returns the patch and its preset text, exactly `Patch::to_json()`.
pub fn normalise(text: &str, edit: &MetaEdit) -> Result<(Patch, String), String> {
    let loaded = enth_patch::file::load(text).map_err(|e| format!("not a valid patch: {e}"))?;
    let mut patch = loaded.patch;
    if let Some(name) = &edit.name {
        name.trim().clone_into(&mut patch.meta.name);
    }
    if let Some(tags) = &edit.tags {
        patch.meta.tags.clone_from(tags);
    }
    if let Some(description) = &edit.description {
        description.trim().clone_into(&mut patch.meta.description);
    }
    ENGINE_VERSION.clone_into(&mut patch.engine);
    enth_patch::validate::validate(&patch).map_err(|e| format!("not a valid patch: {e}"))?;
    let problems = vocab::check(&patch.meta);
    if !problems.is_empty() {
        return Err(format!(
            "{:?} breaks the factory rules: {}",
            patch.meta.name,
            problems.join("; ")
        ));
    }
    let json = patch.to_json();
    check_text(&json)?;
    Ok((patch, json))
}

/// The check `crates/enth-plugin/tests/presets.rs` makes of every factory file: it loads, it is the current schema
/// with no migration notes, and `Patch::to_json()` gives back the same bytes.
pub fn check_text(text: &str) -> Result<Patch, String> {
    let loaded = enth_patch::file::load(text).map_err(|e| format!("does not load: {e}"))?;
    if loaded.from_schema != SCHEMA || !loaded.notes.is_empty() {
        return Err(format!(
            "schema {} with migration notes {:?}: run it through `cargo xtask preset-add` again",
            loaded.from_schema, loaded.notes
        ));
    }
    let saved = loaded.patch.to_json();
    if saved != text {
        let at = saved
            .bytes()
            .zip(text.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(saved.len().min(text.len()));
        return Err(format!(
            "the plugin would save it differently from byte {at} (not exactly `Patch::to_json()`: edited or \
             pretty-printed? `cargo xtask preset-add` writes it)"
        ));
    }
    Ok(loaded.patch)
}

/// Every `*.patch.json` under `dir`, recursively, sorted; dot files and dot folders are skipped like the plugin's
/// scan. A missing folder has none.
pub fn preset_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    collect(dir, &mut out)?;
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("{}: {e}", dir.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(&path, out)?;
        } else if name.ends_with(FILE_SUFFIX) {
            out.push(path);
        }
    }
    Ok(())
}

/// The names in use under `dir` (trimmed, lower case) and the file that holds each. A file that does not load is an
/// error: the folder must be clean before anything joins it.
pub fn names_in(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut names = BTreeMap::new();
    for path in preset_files(dir)? {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let patch = enth_patch::file::load(&text)
            .map_err(|e| format!("{}: does not load: {e}", path.display()))?
            .patch;
        names.insert(patch.meta.name.trim().to_lowercase(), path);
    }
    Ok(names)
}

/// The file a preset goes to under `target`.
pub fn path_for(patch: &Patch, target: &Target) -> Result<PathBuf, String> {
    let stem = slug(&patch.meta.name);
    if stem.is_empty() {
        return Err(format!(
            "the name {:?} needs at least one ASCII letter or digit for its file name",
            patch.meta.name
        ));
    }
    let folder = match &target.folder {
        Some(f) => f.trim_matches('/').to_owned(),
        None => vocab::type_of(&patch.meta.tags)
            .ok_or("no type tag to name the folder by")?
            .to_owned(),
    };
    if folder
        .split('/')
        .any(|part| part == ".." || part.starts_with('.') && part != ".")
    {
        return Err(format!(
            "the folder {folder:?} may not climb out or be hidden"
        ));
    }
    let mut path = target.root.clone();
    for part in folder.split('/').filter(|p| !p.is_empty() && *p != ".") {
        path.push(part);
    }
    path.push(format!("{stem}{FILE_SUFFIX}"));
    Ok(path)
}

/// Write a normalised preset under `target`: its name must be unique under `target.root` (ignoring case; `replacing`
/// is a file that does not count, the source of a `--move`), and a file with other content is replaced only with
/// `force`. The written file is read back and checked.
pub fn write(
    patch: &Patch,
    json: &str,
    target: &Target,
    replacing: Option<&Path>,
) -> Result<Written, String> {
    let path = path_for(patch, target)?;
    let key = patch.meta.name.trim().to_lowercase();
    if let Some(other) = names_in(&target.root)?.get(&key)
        && other != &path
        && replacing.is_none_or(|r| !same_file(r, other))
    {
        return Err(format!(
            "the name {:?} is already {}'s (names are unique under {}, ignoring case)",
            patch.meta.name,
            other.display(),
            target.root.display()
        ));
    }
    let state = match std::fs::read_to_string(&path) {
        Ok(old) if old == json => "unchanged",
        Ok(old) => {
            let other = enth_patch::file::load(&old)
                .map(|l| l.patch.meta.name)
                .unwrap_or_default();
            if other.trim().to_lowercase() != key {
                return Err(format!(
                    "{} holds another preset ({other:?}) whose name gives the same file name: choose another name",
                    path.display()
                ));
            }
            if !target.force {
                return Err(format!(
                    "{} exists with other content (--force replaces it)",
                    path.display()
                ));
            }
            "replaced"
        }
        Err(_) => "new",
    };
    if !target.dry_run && state != "unchanged" {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        let back =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        check_text(&back).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(Written {
        path,
        name: patch.meta.name.clone(),
        state,
        bytes: json.len(),
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Add one drafted patch file.
pub fn add(
    file: &Path,
    edit: &MetaEdit,
    target: &Target,
    remove_source: bool,
) -> Result<Written, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let (patch, json) = normalise(&text, edit).map_err(|e| format!("{}: {e}", file.display()))?;
    let written = write(&patch, &json, target, remove_source.then_some(file))?;
    if remove_source && !target.dry_run && !same_file(file, &written.path) {
        std::fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(written)
}

/// The candidate under `from` called `key`: a preset name (ignoring case), a file stem, or a path.
pub fn find(from: &Path, key: &str) -> Result<PathBuf, String> {
    let as_path = Path::new(key);
    if key.ends_with(FILE_SUFFIX) && as_path.is_file() {
        return Ok(as_path.to_path_buf());
    }
    let wanted = key.trim().to_lowercase();
    let wanted_stem = slug(key);
    let mut found = Vec::new();
    for path in preset_files(from)? {
        let stem = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(FILE_SUFFIX))
            .unwrap_or_default();
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = enth_patch::file::load(&text)
            .map(|l| l.patch.meta.name.trim().to_lowercase())
            .unwrap_or_default();
        if name == wanted || stem == wanted_stem {
            found.push(path);
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!(
            "no candidate called {key:?} under {} (a preset name, ignoring case, or a file stem)",
            from.display()
        )),
        _ => Err(format!(
            "{key:?} matches {} files under {}: {found:?}",
            found.len(),
            from.display()
        )),
    }
}

/// Promote a candidate: it must already be in preset form (it came through [`add`]); `name` renames it on the way.
pub fn promote(
    from: &Path,
    key: &str,
    name: Option<&str>,
    target: &Target,
    remove_source: bool,
) -> Result<Written, String> {
    let source = find(from, key)?;
    let text =
        std::fs::read_to_string(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    check_text(&text).map_err(|e| format!("{}: {e}", source.display()))?;
    let edit = MetaEdit {
        name: name.map(str::to_owned),
        ..MetaEdit::default()
    };
    add(&source, &edit, target, remove_source)
}

/// One candidate row of a `PICKS.md` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    /// The candidate's name.
    pub candidate: String,
    /// Whether the owner keeps it: the "Keep" cell is `yes` or `y`, ignoring case.
    pub keep: bool,
    /// A new name for the factory copy, if the owner gave one.
    pub rename: Option<String>,
}

/// The candidate rows of a `PICKS.md` table: its header names the columns, `Candidate` and `Keep` required,
/// `Factory name` optional. Rows of other tables are ignored. [`picks`] keeps the rows marked to keep.
pub fn pick_rows(text: &str) -> Result<Vec<Pick>, String> {
    let mut out = Vec::new();
    let mut columns: Option<(usize, usize, Option<usize>)> = None;
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            columns = None;
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
        {
            continue;
        }
        let lower: Vec<String> = cells.iter().map(|c| c.to_lowercase()).collect();
        let find = |name: &str| lower.iter().position(|c| c == name);
        if let (Some(candidate), Some(keep)) = (find("candidate"), find("keep")) {
            columns = Some((candidate, keep, find("factory name")));
            continue;
        }
        let Some((candidate, keep, rename)) = columns else {
            continue;
        };
        let cell = |i: usize| cells.get(i).copied().unwrap_or_default();
        let name = cell(candidate).trim_matches('`').trim();
        if name.is_empty() {
            return Err(format!("a row has no candidate name: {line}"));
        }
        let rename = rename
            .map(cell)
            .map(|r| r.trim_matches('`').trim())
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        out.push(Pick {
            candidate: name.to_owned(),
            keep: matches!(cell(keep).to_lowercase().as_str(), "yes" | "y"),
            rename,
        });
    }
    Ok(out)
}

/// The rows of a `PICKS.md` table the owner marked to keep.
pub fn picks(text: &str) -> Result<Vec<Pick>, String> {
    Ok(pick_rows(text)?.into_iter().filter(|p| p.keep).collect())
}

#[derive(Default)]
struct Options {
    file: Option<PathBuf>,
    promote: Vec<String>,
    picks: Option<PathBuf>,
    edit: MetaEdit,
    out: Option<PathBuf>,
    from: Option<PathBuf>,
    to: Option<PathBuf>,
    folder: Option<String>,
    force: bool,
    remove_source: bool,
    dry_run: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--name" => o.edit.name = Some(value("--name")?),
            "--tags" => o.edit.tags = Some(parse_tags(&value("--tags")?)),
            "--description" => o.edit.description = Some(value("--description")?),
            "--out" => o.out = Some(PathBuf::from(value("--out")?)),
            "--from" => o.from = Some(PathBuf::from(value("--from")?)),
            "--to" => o.to = Some(PathBuf::from(value("--to")?)),
            "--folder" => {
                let f = value("--folder")?;
                o.folder = Some(if f == "." { String::new() } else { f });
            }
            "--promote" => o.promote.extend(
                value("--promote")?
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            ),
            "--picks" => o.picks = Some(PathBuf::from(value("--picks")?)),
            "--force" => o.force = true,
            "--move" => o.remove_source = true,
            "--dry-run" => o.dry_run = true,
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            file if o.file.is_none() => o.file = Some(PathBuf::from(file)),
            extra => return Err(format!("one patch file at a time ({extra:?})\n{USAGE}")),
        }
    }
    let modes = usize::from(o.file.is_some())
        + usize::from(!o.promote.is_empty())
        + usize::from(o.picks.is_some());
    if modes != 1 {
        return Err(format!(
            "give one of: a patch file, --promote, --picks\n{USAGE}"
        ));
    }
    if o.file.is_some() && (o.from.is_some() || o.to.is_some()) {
        return Err(format!(
            "--from and --to are for --promote and --picks; a file goes to --out\n{USAGE}"
        ));
    }
    if o.file.is_none() && o.out.is_some() {
        return Err(format!(
            "--out is for a patch file; --promote and --picks write to --to\n{USAGE}"
        ));
    }
    if o.file.is_none() && (o.edit.tags.is_some() || o.edit.description.is_some()) {
        return Err(format!(
            "--tags and --description are for a patch file: a candidate keeps its own\n{USAGE}"
        ));
    }
    if o.edit.name.is_some() && (o.promote.len() > 1 || o.picks.is_some()) {
        return Err(
            "--name renames one preset (in PICKS.md, use the Factory name column)".to_owned(),
        );
    }
    Ok(o)
}

fn report(w: &Written, dry_run: bool, from: Option<&Path>) {
    println!(
        "{}{:?}: {} ({}, {} bytes{})",
        if dry_run { "(dry run) " } else { "" },
        w.name,
        rel(&w.path),
        w.state,
        w.bytes,
        from.map(|f| format!(", from {}", rel(f)))
            .unwrap_or_default()
    );
}

fn rel(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// `cargo xtask preset-add`.
pub fn preset_add(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let root = crate::metadata()?.workspace_root;
    let target = |dir: Option<&PathBuf>, default: &str| Target {
        root: dir.cloned().unwrap_or_else(|| root.join(default)),
        folder: o.folder.clone(),
        force: o.force,
        dry_run: o.dry_run,
    };
    if let Some(file) = &o.file {
        let t = target(o.out.as_ref(), CANDIDATE_DIR);
        let w = add(file, &o.edit, &t, o.remove_source)?;
        report(&w, o.dry_run, Some(file));
        return Ok(());
    }
    let from = o.from.clone().unwrap_or_else(|| root.join(CANDIDATE_DIR));
    let t = target(o.to.as_ref(), PRESET_DIR);
    let wanted: Vec<Pick> = if let Some(file) = &o.picks {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let rows = picks(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        if rows.is_empty() {
            return Err(format!(
                "{}: no row is marked yes in a table with Candidate and Keep columns",
                file.display()
            ));
        }
        rows
    } else {
        o.promote
            .iter()
            .map(|c| Pick {
                candidate: c.clone(),
                keep: true,
                rename: o.edit.name.clone(),
            })
            .collect()
    };
    // Resolve every pick before writing any, so a typo in the last row writes nothing.
    for p in &wanted {
        find(&from, &p.candidate)?;
    }
    for p in &wanted {
        let source = find(&from, &p.candidate)?;
        let w = promote(
            &from,
            &p.candidate,
            p.rename.as_deref(),
            &t,
            o.remove_source,
        )?;
        report(&w, o.dry_run, Some(&source));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// A fresh, empty temporary folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(what: &str) -> Scratch {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "enth-xtask-preset-add-{what}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn target(root: &Path) -> Target {
        Target {
            root: root.to_path_buf(),
            folder: None,
            force: false,
            dry_run: false,
        }
    }

    fn edit(name: &str, tags: &str) -> MetaEdit {
        MetaEdit {
            name: Some(name.to_owned()),
            tags: Some(parse_tags(tags)),
            description: None,
        }
    }

    /// What the CLI writes: pretty JSON, no name, no tags (`enth new --template init`'s shape).
    fn draft() -> String {
        let mut p = Patch::init();
        p.engine = ENGINE_VERSION.to_owned();
        p.to_json_pretty()
    }

    /// The factory test's rules (`crates/enth-plugin/tests/presets.rs`) over a folder: every file is exactly
    /// `to_json()` of a current patch, uses the vocabulary, and has a name of its own ignoring case.
    fn factory_rules(dir: &Path) -> (usize, Vec<String>) {
        let files = preset_files(dir).unwrap();
        let mut problems = Vec::new();
        let mut names: BTreeMap<String, PathBuf> = BTreeMap::new();
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            match check_text(&text) {
                Ok(patch) => {
                    for why in vocab::check(&patch.meta) {
                        problems.push(format!("{}: {why}", path.display()));
                    }
                    let name = patch.meta.name.trim().to_lowercase();
                    if let Some(first) = names.insert(name.clone(), path.clone()) {
                        problems.push(format!(
                            "{}: the name {name:?} is also {}'s",
                            path.display(),
                            first.display()
                        ));
                    }
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        (files.len(), problems)
    }

    #[test]
    fn a_pretty_cli_draft_becomes_exactly_to_json_with_its_meta() {
        let (patch, json) = normalise(&draft(), &edit("Soft Pluck", "warm,pluck")).unwrap();
        assert_eq!(json, patch.to_json());
        assert!(!json.ends_with('\n'));
        assert_eq!(patch.meta.name, "Soft Pluck");
        assert_eq!(patch.meta.tags, ["pluck", "warm"], "the type tag first");
        assert_eq!(patch.engine, ENGINE_VERSION);
        check_text(&json).unwrap();
        // Normalising a preset again changes nothing.
        let (_, again) = normalise(&json, &MetaEdit::default()).unwrap();
        assert_eq!(again, json);
    }

    #[test]
    fn an_old_engine_stamp_is_replaced_by_the_current_one() {
        let mut p = Patch::init();
        p.engine = "0.0.1".to_owned();
        let (patch, _) = normalise(&p.to_json(), &edit("Old", "lead")).unwrap();
        assert_eq!(patch.engine, ENGINE_VERSION);
    }

    #[test]
    fn the_factory_rules_refuse_bad_names_and_tags() {
        let bad = [
            ("", "bass"),
            (&"x".repeat(65), "bass"),
            ("No Type", "warm"),
            ("Two Types", "bass,lead"),
            ("Unknown", "bass,fat"),
            ("Five Words", "pad,warm,wide,soft,dark,smooth"),
        ];
        for (name, tags) in bad {
            let e = normalise(&draft(), &edit(name, tags)).unwrap_err();
            assert!(e.contains("factory rules"), "{name:?} {tags:?}: {e}");
        }
        // Repeats survive parse_tags (it does not hide a mistake) and are refused.
        assert!(normalise(&draft(), &edit("Twice", "bass,warm,warm")).is_err());
        assert!(normalise(&draft(), &edit("Four", "pad,warm,wide,soft,dark")).is_ok());
        assert!(
            normalise(
                "{\"schema\":1,\"generators\":[{\"id\":\"x\"}]}",
                &edit("Bad", "pad")
            )
            .is_err()
        );
    }

    #[test]
    fn a_preset_goes_to_its_type_folder_under_a_slug() {
        let s = Scratch::new("folder");
        let (patch, json) = normalise(&draft(), &edit("Acid Snap #2", "bass,gritty")).unwrap();
        let w = write(&patch, &json, &target(&s.0), None).unwrap();
        assert_eq!(w.path, s.0.join("bass/acid-snap-2.patch.json"));
        assert_eq!(w.state, "new");
        assert_eq!(std::fs::read_to_string(&w.path).unwrap(), json);
        // The same again is unchanged; other content needs --force.
        assert_eq!(
            write(&patch, &json, &target(&s.0), None).unwrap().state,
            "unchanged"
        );
        let (p2, j2) = normalise(&draft(), &edit("Acid Snap #2", "bass,hard")).unwrap();
        assert!(
            write(&p2, &j2, &target(&s.0), None)
                .unwrap_err()
                .contains("--force")
        );
        let forced = Target {
            force: true,
            ..target(&s.0)
        };
        assert_eq!(write(&p2, &j2, &forced, None).unwrap().state, "replaced");
        // The top folder, and a named one.
        let top = Target {
            folder: Some(String::new()),
            ..target(&s.0)
        };
        let (p3, j3) = normalise(&draft(), &edit("Top", "lead")).unwrap();
        assert_eq!(
            write(&p3, &j3, &top, None).unwrap().path,
            s.0.join("top.patch.json")
        );
        let climb = Target {
            folder: Some("../out".to_owned()),
            ..target(&s.0)
        };
        assert!(write(&p3, &j3, &climb, None).is_err());
        // Another name with the same file name is refused, even with --force.
        let (p5, j5) = normalise(&draft(), &edit("Acid-Snap 2", "bass")).unwrap();
        let e = write(&p5, &j5, &forced, None).unwrap_err();
        assert!(e.contains("another preset"), "{e}");
        // A dry run writes nothing.
        let dry = Target {
            dry_run: true,
            ..target(&s.0)
        };
        let (p4, j4) = normalise(&draft(), &edit("Dry", "pad")).unwrap();
        let w = write(&p4, &j4, &dry, None).unwrap();
        assert_eq!(w.state, "new");
        assert!(!w.path.exists());
    }

    #[test]
    fn names_are_unique_under_the_root_ignoring_case() {
        let s = Scratch::new("unique");
        let (a, ja) = normalise(&draft(), &edit("Deep Sub", "bass")).unwrap();
        write(&a, &ja, &target(&s.0), None).unwrap();
        // Another folder and another file name, but the same name ignoring case.
        let (b, jb) = normalise(&draft(), &edit("deep sub", "pad")).unwrap();
        let e = write(&b, &jb, &target(&s.0), None).unwrap_err();
        assert!(e.contains("already"), "{e}");
    }

    #[test]
    fn move_renames_a_file_in_place() {
        let s = Scratch::new("move");
        // An owner's sound extracted from the starter project: the starter's name and tag.
        let mut p = Patch::init();
        p.meta.name = "Starter".to_owned();
        p.meta.tags = vec!["starter".to_owned()];
        let source = s.0.join("bass.patch.json");
        std::fs::write(&source, p.to_json()).unwrap();
        let w = add(
            &source,
            &edit("Owner Bass", "bass,warm"),
            &target(&s.0),
            true,
        )
        .unwrap();
        assert_eq!(w.path, s.0.join("bass/owner-bass.patch.json"));
        assert!(!source.exists());
        assert_eq!(factory_rules(&s.0), (1, vec![]));
    }

    #[test]
    fn promote_finds_by_name_or_stem_and_can_rename() {
        let from = Scratch::new("from");
        let to = Scratch::new("to");
        let (p, j) = normalise(&draft(), &edit("Glass Keys", "keys,bright")).unwrap();
        write(&p, &j, &target(&from.0), None).unwrap();
        assert!(find(&from.0, "glass keys").is_ok());
        assert!(find(&from.0, "glass-keys").is_ok());
        assert!(find(&from.0, "Nope").unwrap_err().contains("no candidate"));
        let w = promote(&from.0, "GLASS KEYS", None, &target(&to.0), false).unwrap();
        assert_eq!(w.path, to.0.join("keys/glass-keys.patch.json"));
        assert_eq!(
            std::fs::read_to_string(&w.path).unwrap(),
            j,
            "promotion copies the bytes"
        );
        let w = promote(
            &from.0,
            "Glass Keys",
            Some("Ice Keys"),
            &target(&to.0),
            false,
        )
        .unwrap();
        assert_eq!(w.path, to.0.join("keys/ice-keys.patch.json"));
        // A candidate that is not in preset form is refused, not silently fixed.
        std::fs::write(from.0.join("keys/pretty.patch.json"), p.to_json_pretty()).unwrap();
        assert!(promote(&from.0, "pretty", Some("Pretty"), &target(&to.0), false).is_err());
    }

    #[test]
    fn picks_reads_the_kept_rows() {
        let text = "\
# Picks

| Candidate | Keep | Factory name | Notes |
| --- | :-: | --- | --- |
| Deep Sub | yes | | warm |
| `Acid Snap` | no | | |
| Glass Keys | Y | Ice Keys | brighter |
| Wide Pad |  | | undecided |

| Other | Keep |
| --- | --- |
| Ignored | yes |
";
        assert_eq!(
            picks(text).unwrap(),
            [
                Pick {
                    candidate: "Deep Sub".to_owned(),
                    keep: true,
                    rename: None
                },
                Pick {
                    candidate: "Glass Keys".to_owned(),
                    keep: true,
                    rename: Some("Ice Keys".to_owned())
                },
            ]
        );
    }

    /// `PICKS.md` has one row per candidate, and every row names a candidate.
    #[test]
    fn the_picks_sheet_has_a_row_for_every_candidate() {
        let dir = repo().join(CANDIDATE_DIR);
        let text = std::fs::read_to_string(dir.join("PICKS.md")).unwrap();
        let rows = pick_rows(&text).unwrap();
        let mut listed = std::collections::BTreeSet::new();
        for p in &rows {
            listed.insert(find(&dir, &p.candidate).unwrap());
        }
        let all: std::collections::BTreeSet<PathBuf> =
            preset_files(&dir).unwrap().into_iter().collect();
        assert_eq!(listed, all, "PICKS.md lists each candidate once");
        assert_eq!(rows.len(), all.len());
    }

    #[test]
    fn options_parse_and_refuse_mixtures() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let o = parse(&args("d.patch.json --name X --tags pad,Warm --folder .")).unwrap();
        assert_eq!(o.edit.tags.unwrap(), ["pad", "warm"]);
        assert_eq!(o.folder.as_deref(), Some(""));
        assert_eq!(
            parse(&args("--promote A,B --to /tmp/x")).unwrap().promote,
            ["A", "B"]
        );
        for bad in [
            "",
            "a.patch.json --promote A",
            "a.patch.json --to x",
            "--promote A --out x",
            "--promote A --tags pad",
            "--promote A,B --name C",
            "a.patch.json b.patch.json",
            "a.patch.json --bogus",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    /// The vocabulary mirror equals `enth_session::presets::tags` and `MAX_NAME_CHARS`, read from the source.
    #[test]
    fn the_vocabulary_mirror_matches_enth_session() {
        let src =
            std::fs::read_to_string(repo().join("crates/enth-session/src/presets.rs")).unwrap();
        let list = |name: &str| -> Vec<String> {
            let start = src
                .find(&format!("pub const {name}: [&str;"))
                .unwrap_or_else(|| panic!("{name} not found"));
            let body = &src[start..];
            let open = body.find("= [").unwrap() + 3;
            let close = body[open..].find(']').unwrap() + open;
            body[open..close]
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let number = |name: &str| -> usize {
            let start = src
                .find(&format!("pub const {name}: usize = "))
                .unwrap_or_else(|| panic!("{name} not found"));
            let body = &src[start + format!("pub const {name}: usize = ").len()..];
            body[..body.find(';').unwrap()].trim().parse().unwrap()
        };
        assert_eq!(
            list("TYPES"),
            vocab::TYPES,
            "edit vocab::TYPES to match enth-session"
        );
        assert_eq!(
            list("CHARACTER"),
            vocab::CHARACTER,
            "edit vocab::CHARACTER to match enth-session"
        );
        assert_eq!(number("MAX_CHARACTER"), vocab::MAX_CHARACTER);
        assert_eq!(number("MAX_NAME_CHARS"), vocab::MAX_NAME_CHARS);
    }

    /// The candidates are already in preset form, and every one promotes into a factory folder (a temporary one,
    /// never `content/presets/`) that passes the factory test's rules.
    #[test]
    fn every_candidate_promotes_into_a_folder_that_passes_the_factory_rules() {
        let from = repo().join(CANDIDATE_DIR);
        let (count, problems) = factory_rules(&from);
        assert!(problems.is_empty(), "{problems:#?}");
        assert!(count >= 18, "about 20 candidates, found {count}");
        let to = Scratch::new("promote-all");
        for path in preset_files(&from).unwrap() {
            let w = promote(&from, path.to_str().unwrap(), None, &target(&to.0), false).unwrap();
            assert_eq!(w.state, "new");
            assert_eq!(
                std::fs::read(&w.path).unwrap(),
                std::fs::read(&path).unwrap(),
                "promotion copies the bytes"
            );
        }
        assert_eq!(factory_rules(&to.0), (count, vec![]));
    }

    /// The README's table lists every candidate, so the owner auditions from a complete list.
    #[test]
    fn the_candidates_readme_lists_every_candidate() {
        let dir = repo().join(CANDIDATE_DIR);
        let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
        for path in preset_files(&dir).unwrap() {
            let patch = check_text(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(
                readme.contains(&format!("| {} |", patch.meta.name)),
                "README.md has no row for {:?}",
                patch.meta.name
            );
        }
    }
}
