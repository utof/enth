//! **Owner: lane PUI** (additive in lane SK3's `create` module; feature `create`).
//!
//! The Create view's folder browser needs a folder listed, and the editor does no I/O: after a frame the glue takes
//! the folder it asks for (`Editor::take_create_browse`), lists it here and hands it back
//! (`Editor::set_create_folder`). One `read_dir` on the GUI thread, under no lock the audio thread knows: sub-folders
//! (hidden ones left out) and the sound files the decoders read (`enth_ui::editor::create::browser::is_sound`), folders
//! first, each by name, at most [`MAX_ENTRIES`]. Nothing is filtered by length or size: a short sound is a sound.
//! The browser starts in the platform's Documents folder, else the home folder ([`start_dir`]).

use std::path::{Path, PathBuf};

use enth_ui::editor::create::{BrowseAt, EntryKind, Folder, FolderEntry, browser::is_sound};

use crate::presets::{Environment, Os, documents_dir, home_dir};

/// The most entries a listing hands over (the rest are counted in [`Folder::more`]).
pub const MAX_ENTRIES: usize = 2_000;

/// Where the browser starts: the platform's Documents folder, else the home folder, else the filesystem's root.
pub fn start_dir(env: &dyn Environment, os: Os) -> PathBuf {
    documents_dir(env, os)
        .filter(|d| d.is_dir())
        .or_else(|| home_dir(env, os).filter(|d| d.is_dir()))
        .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// The folder `at` names, listed.
pub fn list_at(at: &BrowseAt, env: &dyn Environment, os: Os) -> Folder {
    match at {
        BrowseAt::Start => list(&start_dir(env, os)),
        BrowseAt::Path(p) => list(p),
    }
}

/// `dir`'s sub-folders and sound files, folders first, each by name; an error when it cannot be read.
pub fn list(dir: &Path) -> Folder {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            return Folder::unreadable(dir.to_path_buf(), format!("cannot read this folder: {e}"));
        }
    };
    let mut entries = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `metadata` follows a symbolic link, so a linked folder opens and a linked file is chosen.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let kind = if meta.is_dir() {
            EntryKind::Folder
        } else if meta.is_file() && is_sound(&path) {
            EntryKind::Sound
        } else {
            continue;
        };
        entries.push(FolderEntry {
            name,
            path,
            kind,
            bytes: (kind == EntryKind::Sound).then_some(meta.len()),
        });
    }
    let mut folder = Folder::new(dir.to_path_buf(), entries);
    if folder.entries.len() > MAX_ENTRIES {
        folder.more = folder.entries.len() - MAX_ENTRIES;
        folder.entries.truncate(MAX_ENTRIES);
    }
    folder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_lists_its_folders_then_its_sounds_and_leaves_the_rest_out() {
        let dir = std::env::temp_dir().join(format!("enth-pui-browse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Kicks")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for (name, bytes) in [
            ("b.wav", 44usize),
            ("A.FLAC", 10),
            ("notes.txt", 3),
            ("tiny.mp3", 1),
            (".secret.wav", 5),
        ] {
            std::fs::write(dir.join(name), vec![0u8; bytes]).unwrap();
        }
        let f = list(&dir);
        let shown: Vec<(&str, EntryKind, Option<u64>)> = f
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.bytes))
            .collect();
        assert_eq!(
            shown,
            [
                ("Kicks", EntryKind::Folder, None),
                ("A.FLAC", EntryKind::Sound, Some(10)),
                ("b.wav", EntryKind::Sound, Some(44)),
                ("tiny.mp3", EntryKind::Sound, Some(1)),
            ],
            "a one-byte file is listed: nothing is filtered by size"
        );
        assert_eq!(f.parent.as_deref(), dir.parent());
        assert!(f.error.is_none());
        let gone = list(&dir.join("missing"));
        assert!(gone.error.is_some() && gone.entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
