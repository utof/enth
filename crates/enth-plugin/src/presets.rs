//! The plugin's preset I/O (lane PBC of R1/S1; report 03 §2.3): where the user's presets live, and a [`PresetStore`]
//! over folders. The format, index, search, naming and the session side of loading and saving are in
//! `enth_session::presets`; the session does no file I/O, so this is the one place that touches the disk.
//!
//! # The user folder
//!
//! In this order ([`presets_dir`]):
//!
//! 1. `ENTH_PRESETS_DIR`, when set and not empty: used as it is (tests, portable setups, CI).
//! 2. The platform's Documents folder ([`documents_dir`]), then `enth/Presets` under it:
//!    - **Linux**: `XDG_DOCUMENTS_DIR` from `$XDG_CONFIG_HOME/user-dirs.dirs` (default `~/.config/user-dirs.dirs`),
//!      with `$HOME` expanded; else the environment variable of that name; else `~/Documents`.
//!    - **Windows**: `%USERPROFILE%\Documents`.
//!    - **macOS**: `~/Documents`.
//!
//! The `dirs` crate would do this, but it pulls `option-ext` (MPL-2.0), which `deny.toml` forbids; the rules above are
//! its own for the Documents folder. Known gap: on Windows a Documents folder that `OneDrive` or a policy moved is
//! `%USERPROFILE%\OneDrive\Documents`, which needs the Known Folder API (a `windows` crate dependency); until then
//! the preset folder is made under `%USERPROFILE%\Documents` and `ENTH_PRESETS_DIR` points elsewhere. The folder is
//! not created until the first save.
//!
//! The environment and the files it reads are behind [`Environment`], so every branch is tested with a fake one.
//!
//! # The folder store
//!
//! [`FsStore`] scans the user folder for `*.patch.json` (recursively, a bounded number of files, folders and bytes;
//! dot files, dot folders and symbolic links to folders are skipped), serves the embedded factory presets read-only,
//! and writes a saved preset as exactly `Patch::to_json()`: into a temporary dot file beside the target, then a
//! rename, so a crash mid-write leaves the old file or none. Favorites are `favorites.json` in the user folder
//! (beside the presets, not a preset). A name that matches an existing file ignoring case counts as existing on every
//! platform, so a preset saved on Linux cannot collide with another on a case-insensitive file system.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use enth_session::presets::{
    FAVORITES_FILE, FILE_SUFFIX, Favorites, Library, PresetError, PresetKey, PresetStore,
    RawPreset, Source, factory_files, factory_text, valid_path,
};

/// How deep the scan goes below the user folder.
pub const MAX_DEPTH: usize = 8;
/// The most preset files one scan reads; the rest are left out.
pub const MAX_FILES: usize = 5000;
/// The largest preset file read, in bytes. A saved patch is a few kilobytes; a bigger file is not one.
pub const MAX_FILE_BYTES: u64 = 1 << 20;

// ---------------------------------------------------------------------------------------------------------------
// The environment
// ---------------------------------------------------------------------------------------------------------------

/// The platforms whose Documents folder rules differ. Everything unix but macOS follows Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux and the other unixes.
    Linux,
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
}

impl Os {
    /// The platform this build runs on.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// What the folder rules read from the machine: environment variables and one text file. A fake in tests.
pub trait Environment {
    /// The value of an environment variable, if set (and valid for the platform).
    fn var(&self, name: &str) -> Option<OsString>;
    /// The text of a file, if it exists and is UTF-8.
    fn read_text(&self, path: &Path) -> Option<String>;
}

/// The real process environment and file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemEnv;

impl Environment for SystemEnv {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn read_text(&self, path: &Path) -> Option<String> {
        // A user-dirs file is a few hundred bytes; a huge one is not that file.
        let meta = fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > 1 << 16 {
            return None;
        }
        fs::read_to_string(path).ok()
    }
}

/// A variable that is set and not empty.
fn var(env: &dyn Environment, name: &str) -> Option<PathBuf> {
    env.var(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The user's home folder (`HOME`; on Windows `USERPROFILE`).
pub fn home_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    match os {
        Os::Windows => var(env, "USERPROFILE"),
        Os::Linux | Os::MacOs => var(env, "HOME"),
    }
}

/// The platform's Documents folder (see the module docs), or `None` if the home folder is unknown.
pub fn documents_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    let home = home_dir(env, os)?;
    match os {
        Os::Windows | Os::MacOs => Some(home.join("Documents")),
        Os::Linux => Some(linux_documents(env, &home)),
    }
}

fn linux_documents(env: &dyn Environment, home: &Path) -> PathBuf {
    let config = var(env, "XDG_CONFIG_HOME")
        .filter(|p| p.has_root())
        .unwrap_or_else(|| home.join(".config"));
    let from_file = env
        .read_text(&config.join("user-dirs.dirs"))
        .and_then(|text| user_dir_entry(&text, "XDG_DOCUMENTS_DIR", home));
    from_file
        .or_else(|| var(env, "XDG_DOCUMENTS_DIR").filter(|p| p.has_root() && p != home))
        .unwrap_or_else(|| home.join("Documents"))
}

/// The value of `key` in the text of a `user-dirs.dirs` file (`KEY="$HOME/Documents"` lines; `#` comments), with a
/// leading `$HOME` expanded. `None` when the key is missing, the value is not an absolute path, or it is the home
/// folder itself (how `xdg-user-dirs` marks a folder as disabled).
fn user_dir_entry(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            // The last assignment wins, as when the file is sourced.
            found = Some(unquote(value.trim()));
        }
    }
    let value = found?;
    let value = if let Some(rest) = value
        .strip_prefix("$HOME")
        .or_else(|| value.strip_prefix("${HOME}"))
    {
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        let mut path = home.as_os_str().to_owned();
        path.push(rest);
        PathBuf::from(path)
    } else {
        PathBuf::from(value)
    };
    (value.has_root() && value != home).then_some(value)
}

/// The text of a shell word as a `user-dirs.dirs` file writes it: in double quotes with `\` escapes, or bare.
fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Where the user's presets live (see the module docs): `ENTH_PRESETS_DIR`, else `<Documents>/enth/Presets`. `None`
/// if neither can be found (no home folder); then the store has factory presets only and saving says so.
pub fn presets_dir(env: &dyn Environment, os: Os) -> Option<PathBuf> {
    var(env, "ENTH_PRESETS_DIR")
        .or_else(|| documents_dir(env, os).map(|d| d.join("enth").join("Presets")))
}

// ---------------------------------------------------------------------------------------------------------------
// The folder store
// ---------------------------------------------------------------------------------------------------------------

fn io_error(what: &'static str, e: &io::Error) -> PresetError {
    PresetError::Io {
        what,
        message: e.to_string(),
    }
}

/// A [`PresetStore`] over the user folder, with the embedded factory presets beside it.
#[derive(Clone, Debug)]
pub struct FsStore {
    dir: Option<PathBuf>,
    favorites: Favorites,
}

impl FsStore {
    /// A store over `dir` (`None`: factory presets only; saving fails with [`PresetError::NoUserFolder`]).
    pub fn new(dir: Option<PathBuf>) -> Self {
        FsStore {
            dir,
            favorites: Favorites::default(),
        }
    }

    /// The store the plugin uses: the folder the environment names ([`presets_dir`]).
    pub fn from_env() -> Self {
        FsStore::new(presets_dir(&SystemEnv, Os::current()))
    }

    /// The user folder.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The file a user key names under `dir`.
    fn path_of(dir: &Path, key: &PresetKey) -> PathBuf {
        let mut path = dir.to_path_buf();
        let mut parts = key.path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                path.push(part);
            } else {
                path.push(format!("{part}{FILE_SUFFIX}"));
            }
        }
        path
    }

    /// The favorites file as it is on disk now: absent, damaged or unreadable is "none".
    fn read_favorites(dir: &Path) -> Favorites {
        read_limited(&dir.join(FAVORITES_FILE))
            .ok()
            .and_then(|text| Favorites::from_json(&text).ok())
            .unwrap_or_default()
    }

    /// Collect the preset files under `dir` into `out`, `rel` being the key path of `dir` itself.
    fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<RawPreset>) {
        let Ok(read) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.flatten().collect();
        // A stable order makes a truncated scan (too many files) the same one every time.
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            if out.len() >= MAX_FILES {
                return;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // `file_type` does not follow links: a link to a folder is not entered (no loops), a link to a file is read.
            let is_dir = kind.is_dir();
            let is_file = kind.is_file() || (kind.is_symlink() && path.is_file());
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if is_dir {
                if depth < MAX_DEPTH {
                    FsStore::walk(&path, &child, depth + 1, out);
                }
            } else if is_file && let Some(stem) = child.strip_suffix(FILE_SUFFIX) {
                if !valid_path(stem) || stem.ends_with('/') {
                    continue;
                }
                // A file too big to be a preset, or not text, is listed with the text it has: the parse says why it
                // cannot be used and the browser reports it as a problem, not as nothing.
                let text = match read_limited(&path) {
                    Ok(text) => text,
                    Err(e) if e.kind() == io::ErrorKind::InvalidData => String::new(),
                    Err(_) => continue,
                };
                out.push(RawPreset {
                    key: PresetKey::user(stem),
                    text,
                });
            }
        }
    }

    /// Whether a file exists at `target`, or a sibling whose name equals its name ignoring case.
    fn exists_ignoring_case(target: &Path) -> bool {
        if fs::symlink_metadata(target).is_ok() {
            return true;
        }
        let (Some(parent), Some(name)) =
            (target.parent(), target.file_name().and_then(|n| n.to_str()))
        else {
            return false;
        };
        let wanted = name.to_lowercase();
        fs::read_dir(parent)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.to_lowercase() == wanted)
            })
    }
}

/// A file's text, refusing one over [`MAX_FILE_BYTES`] or not UTF-8 (`InvalidData`).
fn read_limited(path: &Path) -> io::Result<String> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file is too big to be a preset",
        ));
    }
    let bytes = fs::read(path)?;
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the file is not UTF-8 text"))
}

/// Write `text` to `target` through a temporary dot file in the same folder and a rename.
fn write_atomic(target: &Path, text: &str) -> io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("preset");
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, target)
    })();
    if result.is_err() {
        // The temporary file is ours; do not leave it behind.
        let _ = fs::remove_file(&temp);
    }
    result
}

impl PresetStore for FsStore {
    fn scan(&mut self) -> Library {
        let mut files = factory_files();
        if let Some(dir) = &self.dir {
            FsStore::walk(dir, "", 0, &mut files);
            self.favorites = FsStore::read_favorites(dir);
        }
        Library::build(files, self.favorites.clone())
    }

    fn read_text(&self, key: &PresetKey) -> Result<String, PresetError> {
        if key.source == Source::Factory {
            return factory_text(key)
                .map(str::to_owned)
                .ok_or_else(|| PresetError::NotFound { key: key.clone() });
        }
        let dir = self.dir.as_ref().ok_or(PresetError::NoUserFolder)?;
        if !valid_path(&key.path) {
            return Err(PresetError::NotFound { key: key.clone() });
        }
        read_limited(&FsStore::path_of(dir, key)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => PresetError::NotFound { key: key.clone() },
            io::ErrorKind::InvalidData => PresetError::Invalid {
                key: key.clone(),
                message: e.to_string(),
            },
            _ => io_error("read the preset", &e),
        })
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
        let dir = self.dir.as_ref().ok_or(PresetError::NoUserFolder)?;
        if !valid_path(&key.path) {
            return Err(PresetError::BadName {
                why: "the path is not valid",
            });
        }
        let target = FsStore::path_of(dir, key);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error("create the folder", &e))?;
        }
        if !overwrite && FsStore::exists_ignoring_case(&target) {
            return Err(PresetError::Exists { key: key.clone() });
        }
        write_atomic(&target, text).map_err(|e| io_error("write the preset", &e))
    }

    fn set_favorite(&mut self, key: &PresetKey, on: bool) -> Result<(), PresetError> {
        let dir = self.dir.as_ref().ok_or(PresetError::NoUserFolder)?;
        // Start from the file as it is now, so a store that never scanned (or another instance) loses nothing.
        let mut favorites = FsStore::read_favorites(dir);
        if !favorites.set(key, on) {
            // Already as asked.
            self.favorites = favorites;
            return Ok(());
        }
        fs::create_dir_all(dir).map_err(|e| io_error("create the folder", &e))?;
        write_atomic(&dir.join(FAVORITES_FILE), &favorites.to_json())
            .map_err(|e| io_error("write the favorites", &e))?;
        self.favorites = favorites;
        Ok(())
    }

    fn user_location(&self) -> Option<String> {
        self.dir.as_ref().map(|d| d.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use enth_session::presets::{SaveRequest, load_from};
    use enth_session::{Session, SessionEdit};

    use super::*;

    /// A fake machine: variables and files.
    #[derive(Default)]
    struct Fake {
        vars: BTreeMap<&'static str, &'static str>,
        files: BTreeMap<String, &'static str>,
    }

    /// A path as text with `/` separators, so the tests read the same on every host (`Path::join` uses the host's).
    fn norm(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    impl Fake {
        fn var(mut self, name: &'static str, value: &'static str) -> Self {
            self.vars.insert(name, value);
            self
        }

        fn file(mut self, path: &str, text: &'static str) -> Self {
            self.files.insert(path.to_owned(), text);
            self
        }
    }

    impl Environment for Fake {
        fn var(&self, name: &str) -> Option<OsString> {
            self.vars.get(name).map(OsString::from)
        }

        fn read_text(&self, path: &Path) -> Option<String> {
            self.files.get(&norm(path)).map(|t| (*t).to_owned())
        }
    }

    fn dir(env: &Fake, os: Os) -> Option<String> {
        presets_dir(env, os).as_deref().map(norm)
    }

    fn docs(env: &Fake, os: Os) -> Option<String> {
        documents_dir(env, os).as_deref().map(norm)
    }

    #[test]
    fn enth_presets_dir_wins_on_every_platform() {
        for os in [Os::Linux, Os::Windows, Os::MacOs] {
            let env = Fake::default()
                .var("ENTH_PRESETS_DIR", "/work/presets")
                .var("HOME", "/home/ana")
                .var("USERPROFILE", "C:/Users/ana");
            assert_eq!(dir(&env, os), Some("/work/presets".to_owned()), "{os:?}");
        }
        // Even without a home folder.
        let env = Fake::default().var("ENTH_PRESETS_DIR", "/work/presets");
        assert_eq!(dir(&env, Os::Linux), Some("/work/presets".to_owned()));
    }

    #[test]
    fn an_empty_enth_presets_dir_is_not_set() {
        let env = Fake::default()
            .var("ENTH_PRESETS_DIR", "")
            .var("HOME", "/home/ana");
        assert_eq!(
            dir(&env, Os::Linux),
            Some("/home/ana/Documents/enth/Presets".to_owned())
        );
    }

    #[test]
    fn linux_reads_the_documents_folder_from_user_dirs() {
        let file =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        let env = Fake::default()
            .var("HOME", "/home/ana")
            .file("/home/ana/.config/user-dirs.dirs", file);
        assert_eq!(
            docs(&env, Os::Linux),
            Some("/home/ana/Dokumente".to_owned())
        );
        assert_eq!(
            dir(&env, Os::Linux),
            Some("/home/ana/Dokumente/enth/Presets".to_owned())
        );
    }

    #[test]
    fn linux_honours_xdg_config_home_when_it_is_absolute() {
        let env = Fake::default()
            .var("HOME", "/home/ana")
            .var("XDG_CONFIG_HOME", "/cfg")
            .file("/cfg/user-dirs.dirs", "XDG_DOCUMENTS_DIR=\"/data/docs\"\n")
            .file(
                "/home/ana/.config/user-dirs.dirs",
                "XDG_DOCUMENTS_DIR=\"$HOME/wrong\"\n",
            );
        assert_eq!(docs(&env, Os::Linux), Some("/data/docs".to_owned()));
        // A relative XDG_CONFIG_HOME is ignored, as the spec says.
        let env = Fake::default()
            .var("HOME", "/home/ana")
            .var("XDG_CONFIG_HOME", "cfg")
            .file(
                "/home/ana/.config/user-dirs.dirs",
                "XDG_DOCUMENTS_DIR=\"$HOME/Papers\"\n",
            );
        assert_eq!(docs(&env, Os::Linux), Some("/home/ana/Papers".to_owned()));
    }

    #[test]
    fn linux_user_dirs_syntax_is_read_like_the_shell_does() {
        let home = Path::new("/home/ana");
        let read = |text: &str| user_dir_entry(text, "XDG_DOCUMENTS_DIR", home).map(|p| norm(&p));
        assert_eq!(
            read("XDG_DOCUMENTS_DIR=\"$HOME/D\""),
            Some("/home/ana/D".to_owned())
        );
        assert_eq!(
            read("XDG_DOCUMENTS_DIR=\"${HOME}/D\""),
            Some("/home/ana/D".to_owned())
        );
        assert_eq!(
            read("  XDG_DOCUMENTS_DIR = '/srv/docs'  "),
            Some("/srv/docs".to_owned())
        );
        assert_eq!(
            read("XDG_DOCUMENTS_DIR=/srv/docs"),
            Some("/srv/docs".to_owned())
        );
        assert_eq!(
            read("XDG_DOCUMENTS_DIR=\"$HOME/My \\\"Docs\\\" \\$x\""),
            Some("/home/ana/My \"Docs\" $x".to_owned())
        );
        // The last assignment wins; a comment is skipped.
        assert_eq!(
            read("XDG_DOCUMENTS_DIR=\"/a\"\n# XDG_DOCUMENTS_DIR=\"/b\"\nXDG_DOCUMENTS_DIR=\"/c\""),
            Some("/c".to_owned())
        );
        // Not usable: other key, home itself (disabled), relative, `$HOMEX`, nothing.
        assert_eq!(read("XDG_MUSIC_DIR=\"$HOME/Music\""), None);
        assert_eq!(read("XDG_DOCUMENTS_DIR=\"$HOME\""), None);
        assert_eq!(read("XDG_DOCUMENTS_DIR=\"$HOME/\""), None, "still home");
        assert_eq!(read("XDG_DOCUMENTS_DIR=\"docs\""), None);
        assert_eq!(read("XDG_DOCUMENTS_DIR=\"$HOMEX/d\""), None);
        assert_eq!(read(""), None);
        assert_eq!(read("garbage line without equals"), None);
    }

    #[test]
    fn linux_falls_back_to_the_variable_then_to_home_documents() {
        // A disabled folder in the file (equal to HOME) falls through.
        let env = Fake::default().var("HOME", "/home/ana").file(
            "/home/ana/.config/user-dirs.dirs",
            "XDG_DOCUMENTS_DIR=\"$HOME\"\n",
        );
        assert_eq!(
            docs(&env, Os::Linux),
            Some("/home/ana/Documents".to_owned())
        );
        // The variable, when the file does not say.
        let env = Fake::default()
            .var("HOME", "/home/ana")
            .var("XDG_DOCUMENTS_DIR", "/mnt/docs");
        assert_eq!(docs(&env, Os::Linux), Some("/mnt/docs".to_owned()));
        // A relative variable, or one equal to HOME, is not used.
        for bad in ["docs", "/home/ana"] {
            let env = Fake::default()
                .var("HOME", "/home/ana")
                .var("XDG_DOCUMENTS_DIR", bad);
            assert_eq!(
                docs(&env, Os::Linux),
                Some("/home/ana/Documents".to_owned()),
                "{bad}"
            );
        }
        // Nothing at all.
        let env = Fake::default().var("HOME", "/home/ana");
        assert_eq!(
            dir(&env, Os::Linux),
            Some("/home/ana/Documents/enth/Presets".to_owned())
        );
        assert_eq!(dir(&Fake::default(), Os::Linux), None);
        assert_eq!(dir(&Fake::default().var("HOME", ""), Os::Linux), None);
    }

    #[test]
    fn windows_uses_user_profile_documents() {
        let env = Fake::default()
            .var("USERPROFILE", "C:\\Users\\ana")
            .var("HOME", "/ignored");
        assert_eq!(
            dir(&env, Os::Windows),
            Some("C:/Users/ana/Documents/enth/Presets".to_owned())
        );
        // HOME alone is not a Windows home.
        assert_eq!(dir(&Fake::default().var("HOME", "/h"), Os::Windows), None);
        // And user-dirs.dirs is not read there.
        let env = Fake::default().var("USERPROFILE", "C:\\Users\\ana").file(
            "C:/Users/ana/.config/user-dirs.dirs",
            "XDG_DOCUMENTS_DIR=\"/x\"",
        );
        assert_eq!(
            docs(&env, Os::Windows),
            Some("C:/Users/ana/Documents".to_owned())
        );
    }

    #[test]
    fn macos_uses_home_documents() {
        let env = Fake::default()
            .var("HOME", "/Users/ana")
            .var("USERPROFILE", "/ignored")
            .file(
                "/Users/ana/.config/user-dirs.dirs",
                "XDG_DOCUMENTS_DIR=\"/x\"",
            );
        assert_eq!(
            dir(&env, Os::MacOs),
            Some("/Users/ana/Documents/enth/Presets".to_owned())
        );
        assert_eq!(dir(&Fake::default(), Os::MacOs), None);
    }

    #[test]
    fn the_current_platform_is_one_of_the_three() {
        let os = Os::current();
        assert_eq!(os == Os::Windows, cfg!(windows));
        assert_eq!(os == Os::MacOs, cfg!(target_os = "macos"));
    }

    // -----------------------------------------------------------------------------------------------------------
    // The folder store, on a real temporary folder
    // -----------------------------------------------------------------------------------------------------------

    /// A fresh folder under the system's temp dir, removed when dropped.
    struct Temp(PathBuf);

    impl Temp {
        fn new(label: &str) -> Temp {
            static N: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "enth-pbc-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Temp(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn put(&self, rel: &str, text: &str) {
            let file = self.0.join(rel);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn preset_text(name: &str, tags: &[&str]) -> String {
        let mut p = enth_patch::Patch::init();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetMeta {
                meta: enth_patch::PatchMeta {
                    name: name.to_owned(),
                    tags: tags.iter().map(|t| (*t).to_owned()).collect(),
                    ..enth_patch::PatchMeta::default()
                },
            },
        )
        .unwrap();
        p.to_json()
    }

    fn user_keys(lib: &Library) -> Vec<String> {
        lib.entries()
            .iter()
            .filter(|e| e.key.source == Source::User)
            .map(|e| e.key.to_text())
            .collect()
    }

    #[test]
    fn a_missing_user_folder_is_an_empty_one() {
        let temp = Temp::new("missing");
        let mut store = FsStore::new(Some(temp.path().join("nope")));
        let lib = store.scan();
        assert_eq!(user_keys(&lib), Vec::<String>::new());
        assert_eq!(lib.problems(), []);
        // No folder at all: factory only, and saving says why not.
        let mut store = FsStore::new(None);
        assert_eq!(store.dir(), None);
        assert_eq!(store.user_location(), None);
        assert_eq!(
            store.write_text(&PresetKey::user("x"), "{}", false),
            Err(PresetError::NoUserFolder)
        );
        assert_eq!(
            store.set_favorite(&PresetKey::user("x"), true),
            Err(PresetError::NoUserFolder)
        );
        assert_eq!(
            store.read_text(&PresetKey::user("x")),
            Err(PresetError::NoUserFolder)
        );
        assert_eq!(store.scan().entries().len(), factory_files().len());
    }

    #[test]
    fn the_scan_finds_nested_presets_and_ignores_what_is_not_one() {
        let temp = Temp::new("scan");
        temp.put("Lead.patch.json", &preset_text("Lead", &["lead"]));
        temp.put("Mine/Deep/Bass.patch.json", &preset_text("Bass", &["bass"]));
        temp.put("Mine/Pad.patch.json", &preset_text("Pad", &[]));
        temp.put("notes.txt", "not a preset");
        temp.put("Mine/readme.md", "# hi");
        temp.put(".hidden.patch.json", &preset_text("Hidden", &[]));
        temp.put(".git/Config.patch.json", &preset_text("In dot folder", &[]));
        temp.put("Mine/.draft.patch.json", &preset_text("Draft", &[]));
        temp.put("Broken.patch.json", "{ not json");
        temp.put("Newer.patch.json", "{\"schema\": 99}");
        temp.put("favorites.json", "{}");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        let lib = store.scan();
        assert_eq!(
            user_keys(&lib),
            ["user:Lead", "user:Mine/Pad", "user:Mine/Deep/Bass"]
        );
        let bad: Vec<String> = lib.problems().iter().map(|p| p.key.to_text()).collect();
        assert_eq!(bad, ["user:Broken", "user:Newer"]);
        assert_eq!(
            lib.entries()
                .iter()
                .find(|e| e.key.name() == "Bass")
                .unwrap()
                .meta
                .tags,
            ["bass"]
        );
    }

    #[test]
    fn the_scan_is_bounded() {
        let temp = Temp::new("bounds");
        // Too deep.
        let rel = "deep/".repeat(MAX_DEPTH + 2);
        temp.put(&format!("{rel}Deep.patch.json"), &preset_text("Deep", &[]));
        temp.put("Top.patch.json", &preset_text("Top", &[]));
        // Too big, and not text.
        temp.put("Huge.patch.json", &" ".repeat(MAX_FILE_BYTES as usize + 1));
        fs::write(temp.path().join("Binary.patch.json"), [0xff, 0xfe, 0x00]).unwrap();
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        let lib = store.scan();
        assert_eq!(user_keys(&lib), ["user:Top"]);
        let bad: Vec<String> = lib.problems().iter().map(|p| p.key.to_text()).collect();
        assert_eq!(
            bad,
            ["user:Binary", "user:Huge"],
            "reported, not silently missing"
        );
        assert_eq!(
            store
                .read_text(&PresetKey::user("Huge"))
                .unwrap_err()
                .code(),
            "preset_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_folder_is_not_entered_and_a_link_to_a_file_is_read() {
        let temp = Temp::new("links");
        temp.put("Real/One.patch.json", &preset_text("One", &[]));
        std::os::unix::fs::symlink(temp.path().join("Real"), temp.path().join("Loop")).unwrap();
        std::os::unix::fs::symlink(temp.path(), temp.path().join("Real/Up")).unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("Real/One.patch.json"),
            temp.path().join("Alias.patch.json"),
        )
        .unwrap();
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        assert_eq!(user_keys(&store.scan()), ["user:Alias", "user:Real/One"]);
    }

    #[test]
    fn saving_writes_the_patch_text_and_the_scan_finds_it() {
        let temp = Temp::new("save");
        let mut store = FsStore::new(Some(temp.path().join("enth/Presets")));
        let key = PresetKey::user("Mine/Leads/Sharp");
        let text = preset_text("Sharp", &["lead"]);
        store.write_text(&key, &text, false).unwrap();
        let file = temp.path().join("enth/Presets/Mine/Leads/Sharp.patch.json");
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            text,
            "exactly the text given"
        );
        assert_eq!(store.read_text(&key).unwrap(), text);
        assert_eq!(user_keys(&store.scan()), ["user:Mine/Leads/Sharp"]);
        // Nothing else is left in the folder: the temporary file is gone.
        let names: Vec<String> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Sharp.patch.json"]);
        assert!(store.user_location().unwrap().ends_with("Presets"));
    }

    #[test]
    fn an_existing_preset_is_never_overwritten_unless_told_to() {
        let temp = Temp::new("overwrite");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        let key = PresetKey::user("Keep");
        store.write_text(&key, "first", false).unwrap();
        assert_eq!(
            store.write_text(&key, "second", false),
            Err(PresetError::Exists { key: key.clone() })
        );
        assert_eq!(store.read_text(&key).unwrap(), "first", "untouched");
        // A name that differs only in case is the same name (a case-insensitive file system would say so).
        let loud = PresetKey::user("KEEP");
        assert_eq!(
            store.write_text(&loud, "third", false),
            Err(PresetError::Exists { key: loud.clone() })
        );
        store.write_text(&key, "second", true).unwrap();
        assert_eq!(store.read_text(&key).unwrap(), "second");
        let names: Vec<String> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Keep.patch.json"], "no temporary file left");
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file_and_the_old_one_alone() {
        let temp = Temp::new("failed");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        // A folder where the file should go: the rename cannot replace it.
        fs::create_dir_all(temp.path().join("Blocked.patch.json")).unwrap();
        let err = store
            .write_text(&PresetKey::user("Blocked"), "text", true)
            .unwrap_err();
        assert_eq!(err.code(), "preset_io", "{err}");
        let names: Vec<String> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Blocked.patch.json"]);
        // A file where a folder should go.
        temp.put("Plain", "a file");
        let err = store
            .write_text(&PresetKey::user("Plain/Inside"), "text", true)
            .unwrap_err();
        assert_eq!(err.code(), "preset_io", "{err}");
    }

    #[test]
    fn factory_presets_are_read_only_and_keys_cannot_climb_out() {
        let temp = Temp::new("readonly");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        let factory = PresetKey::factory("anything");
        assert_eq!(
            store.write_text(&factory, "x", true),
            Err(PresetError::ReadOnly {
                key: factory.clone()
            })
        );
        assert_eq!(
            store.read_text(&factory),
            Err(PresetError::NotFound { key: factory })
        );
        for bad in ["../escape", "a/../../b", "/abs", "a\\b", ""] {
            let key = PresetKey::user(bad);
            assert_eq!(
                store.write_text(&key, "x", true).unwrap_err().code(),
                "bad_preset_name",
                "{bad:?}"
            );
            assert_eq!(
                store.read_text(&key).unwrap_err().code(),
                "preset_not_found",
                "{bad:?}"
            );
        }
        assert_eq!(
            fs::read_dir(temp.path()).unwrap().count(),
            0,
            "nothing was made"
        );
    }

    #[test]
    fn missing_files_say_not_found() {
        let temp = Temp::new("notfound");
        let store = FsStore::new(Some(temp.path().to_path_buf()));
        let key = PresetKey::user("gone/Away");
        assert_eq!(store.read_text(&key), Err(PresetError::NotFound { key }));
    }

    #[test]
    fn favorites_live_in_the_user_folder_and_survive_a_new_store() {
        let temp = Temp::new("favs");
        let one = PresetKey::user("One");
        let two = PresetKey::factory("bass/Two");
        let mut store = FsStore::new(Some(temp.path().join("fresh")));
        // Before any scan, and in a folder that does not exist yet.
        store.set_favorite(&one, true).unwrap();
        store.set_favorite(&two, true).unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("fresh").join(FAVORITES_FILE)).unwrap(),
            r#"{"schema":"enth.favorites/1","favorites":["factory:bass/Two","user:One"]}"#
        );
        // A different store over the same folder sees them (after writing a preset it can list).
        let mut other = FsStore::new(Some(temp.path().join("fresh")));
        other
            .write_text(&one, &preset_text("One", &[]), false)
            .unwrap();
        let lib = other.scan();
        assert!(lib.favorites().contains(&one) && lib.favorites().contains(&two));
        // Un-starring one keeps the other; un-starring twice is fine.
        other.set_favorite(&one, false).unwrap();
        other.set_favorite(&one, false).unwrap();
        let lib = FsStore::new(Some(temp.path().join("fresh"))).scan();
        assert!(!lib.favorites().contains(&one) && lib.favorites().contains(&two));
        // The favorites file is not a preset and not a problem.
        assert_eq!(lib.problems(), []);
        assert_eq!(user_keys(&lib), ["user:One"]);
    }

    #[test]
    fn a_damaged_favorites_file_is_none_and_is_replaced_by_the_next_change() {
        let temp = Temp::new("badfavs");
        temp.put(FAVORITES_FILE, "this is not json");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        assert!(store.scan().favorites().is_empty());
        store.set_favorite(&PresetKey::user("A"), true).unwrap();
        assert!(
            FsStore::new(Some(temp.path().to_path_buf()))
                .scan()
                .favorites()
                .contains(&PresetKey::user("A"))
        );
    }

    #[test]
    fn a_session_saves_into_a_folder_and_loads_it_back_as_one_undo_step() {
        let temp = Temp::new("session");
        let mut store = FsStore::new(Some(temp.path().to_path_buf()));
        let mut session = Session::default();
        session
            .edit(
                vec![enth_patch::Op::SetParam {
                    addr: "osc1.semi".parse().unwrap(),
                    value: 5.0,
                }],
                None,
            )
            .unwrap();
        let mut request = SaveRequest::new("Fifth");
        request.folder = "Mine".to_owned();
        request.tags = vec!["lead".to_owned()];
        let key = session.save_preset(&mut store, &request).unwrap();
        assert_eq!(key, PresetKey::user("Mine/Fifth"));
        let on_disk = fs::read_to_string(temp.path().join("Mine/Fifth.patch.json")).unwrap();
        assert_eq!(
            on_disk,
            session.patch().to_json(),
            "the file is Patch::to_json()"
        );
        // Another session loads it by key.
        let mut other = Session::default();
        let loaded = load_from(&mut other, &store, &key).unwrap();
        assert!(loaded.changed && loaded.undoable);
        assert_eq!(other.patch().to_json(), on_disk);
        assert_eq!(other.undo(), Ok(true));
        assert_eq!(
            other.patch().value(&"osc1.semi".parse().unwrap()),
            Some(0.0)
        );
        assert_eq!(other.undo(), Ok(false));
    }

    // -----------------------------------------------------------------------------------------------------------
    // Through the plugin's session lock and the host requests
    // -----------------------------------------------------------------------------------------------------------

    /// A host that counts the fork's requests.
    #[derive(Default)]
    struct CountingHost {
        dirty: AtomicU32,
        rescans: AtomicU32,
    }

    impl nice_plug::context::host::HostRequestsInner for CountingHost {
        fn mark_dirty(&self) {
            self.dirty.fetch_add(1, Ordering::SeqCst);
        }

        fn request_param_info_rescan(&self) {
            self.rescans.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// An editor preset load, its undo and its redo, through `Shared::with_session` like the editor makes them:
    /// each marks the host project dirty once, none writes a host parameter, and only a load that changes the
    /// parameter names asks for a rescan.
    #[test]
    fn a_preset_load_undo_and_redo_mark_the_project_dirty_and_write_nothing_to_the_host() {
        use nice_plug::prelude::{HostRequests, Plugin};

        let mut plugin = crate::Enth::default();
        let host = Arc::new(CountingHost::default());
        plugin.host_requests(HostRequests::new(host.clone()));
        let (dirty, rescans) = (
            || host.dirty.load(Ordering::SeqCst),
            || host.rescans.load(Ordering::SeqCst),
        );
        let shared = Arc::clone(plugin.shared());

        // A preset with a slot bound (so the host's parameter 1 has a name and a value) and other values everywhere.
        let mut preset = enth_patch::Patch::init();
        enth_patch::apply_all(
            &mut preset,
            &[
                enth_patch::Op::SetSlot {
                    slot: enth_patch::SlotIndex::from_one_based(1).unwrap(),
                    addr: Some("osc1.semi".parse().unwrap()),
                },
                enth_patch::Op::SetParam {
                    addr: "osc1.semi".parse().unwrap(),
                    value: 7.0,
                },
                enth_patch::Op::SetParam {
                    addr: "master.level".parse().unwrap(),
                    value: -6.0,
                },
            ],
        )
        .unwrap();
        let key = PresetKey::user("Seven");

        let (loaded, writes) =
            shared.with_session(|s| SessionEdit::load_preset(s, &preset, Some(&key)));
        let loaded = loaded.unwrap();
        assert!(loaded.changed && loaded.undoable);
        assert_eq!(writes.len(), 0, "a load makes no host write (nih-plug #92)");
        assert_eq!(dirty(), 1, "the project is dirty");
        assert_eq!(rescans(), 1, "slot 1 has a new name");

        let (moved, writes) = shared.with_session(SessionEdit::undo);
        assert_eq!(moved, Ok(true));
        assert_eq!(writes.len(), 0, "undo of a load makes no host write");
        assert_eq!(dirty(), 2);
        assert_eq!(rescans(), 2, "the names are back");

        let (moved, writes) = shared.with_session(SessionEdit::redo);
        assert_eq!(moved, Ok(true));
        assert_eq!(writes.len(), 0, "redo of a load makes no host write");
        assert_eq!(dirty(), 3);
        assert_eq!(rescans(), 3);

        // The host's own parameters were never touched: slot 1 still reports its default.
        let values = shared.host_values().unwrap();
        let default = Session::default().host_values();
        assert_eq!(values.slots, default.slots);

        // Loading what is loaded already is nothing: no dirty mark, no rescan.
        let (again, writes) =
            shared.with_session(|s| SessionEdit::load_preset(s, &preset, Some(&key)));
        assert!(!again.unwrap().changed);
        assert_eq!((writes.len(), dirty(), rescans()), (0, 3, 3));

        // A load that keeps the host's names marks dirty without a rescan.
        let mut warmer = preset.clone();
        enth_patch::apply(
            &mut warmer,
            enth_patch::Op::SetParam {
                addr: "master.level".parse().unwrap(),
                value: -3.0,
            },
        )
        .unwrap();
        let (loaded, writes) = shared.with_session(|s| SessionEdit::load_preset(s, &warmer, None));
        assert!(loaded.unwrap().changed);
        assert_eq!((writes.len(), dirty(), rescans()), (0, 4, 3));
    }
}
