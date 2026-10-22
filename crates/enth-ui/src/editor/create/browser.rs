//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3).
//!
//! The in-editor folder browser, the fallback where a file drop does not reach the editor (docs/r3.md, "File input"):
//! no `rfd`, which would need a portal or a GTK backend on Linux. Whether REAPER drags media items into a plugin
//! window as files is an open check, and this browser is why that check is not blocking.
//!
//! **Directory listing is the shell's I/O**, handed over as data: **Browse…** asks for a folder ([`BrowseAt`], read by
//! the shell after the frame with `Editor::take_create_browse`), and the shell answers with a [`Folder`]
//! (`Editor::set_create_folder`; the plugin's `create::browse::list`): its sub-folders and the sound files in it
//! ([`is_sound`]), folders first, each by name. A folder that cannot be read comes back with its error, shown in the
//! list's place. Nothing is filtered by length or size: a short sound is a sound.
//!
//! The browser takes the panel's place under the drop zone while it shows: a row with **◀ Up** (the parent folder),
//! the folder's name and **×**; the list (`size.create.browser_row` per row, scrolling inside itself after
//! `size.create.browser_rows` rows, so the panel itself never scrolls for it); a field for a typed path with
//! **Choose**, which chooses a sound file and opens anything else as a folder. A click on a folder opens it, on a sound
//! file chooses it (and starts a job when none runs). Sizes come from `theme/sizes/create.rs`.

use std::path::{Path, PathBuf};

use egui::{Align, Label, Layout, RichText, ScrollArea, Sense, TextEdit, Ui, vec2};

use super::panel::{button, elided, named};
use super::state::{Create, CreateIntent};
use crate::Theme;
use crate::editor::cards::{Intent, Intents};
use crate::theme::glyph;

/// The extensions the browser lists as sound files (lower case): what the plugin's decoders read (WAV, FLAC, Ogg
/// Vorbis, MP3; `docs/r3.md`, "File input"). The decoders sniff the bytes, so a file with another name still decodes
/// when chosen by its path.
pub const SOUND_EXTENSIONS: &[&str] = &["wav", "wave", "flac", "ogg", "oga", "mp3"];

/// Whether `path` names a sound file the browser lists, by its extension.
#[must_use]
pub fn is_sound(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOUND_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Where the browser asks the shell to look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseAt {
    /// The shell's starting folder (its Documents, else home).
    Start,
    /// This folder.
    Path(PathBuf),
}

/// What an entry of a [`Folder`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A sub-folder: a click opens it.
    Folder,
    /// A sound file: a click chooses it.
    Sound,
}

/// One entry of a [`Folder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
    /// The name shown.
    pub name: String,
    /// Its path.
    pub path: PathBuf,
    /// A folder or a sound file.
    pub kind: EntryKind,
    /// A file's size in bytes, when the shell read it.
    pub bytes: Option<u64>,
}

/// A folder's listing, as the shell hands it to the browser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    /// The folder.
    pub path: PathBuf,
    /// Its parent, for **◀ Up**.
    pub parent: Option<PathBuf>,
    /// Its sub-folders, then its sound files, each sorted by name.
    pub entries: Vec<FolderEntry>,
    /// Entries left out past the shell's cap.
    pub more: usize,
    /// Why it could not be read.
    pub error: Option<String>,
}

impl Folder {
    /// A listing of `path` from its entries, sorted as the browser shows them: folders first, each kind by name
    /// (case-insensitive). Anything that is neither a folder nor a sound file ([`is_sound`]) is left out.
    #[must_use]
    pub fn new(path: PathBuf, entries: Vec<FolderEntry>) -> Self {
        let mut entries: Vec<FolderEntry> = entries
            .into_iter()
            .filter(|e| e.kind == EntryKind::Folder || is_sound(&e.path))
            .collect();
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Folder)
                .cmp(&(b.kind != EntryKind::Folder))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Folder {
            parent: path.parent().map(Path::to_path_buf),
            path,
            entries,
            more: 0,
            error: None,
        }
    }

    /// A folder that could not be read.
    #[must_use]
    pub fn unreadable(path: PathBuf, error: impl Into<String>) -> Self {
        Folder {
            parent: path.parent().map(Path::to_path_buf),
            path,
            entries: Vec::new(),
            more: 0,
            error: Some(error.into()),
        }
    }
}

/// A size in bytes, short: "820 B", "14 KB", "1.2 MB".
fn size_text(bytes: u64) -> String {
    const K: u64 = 1024;
    if bytes < K {
        format!("{bytes} B")
    } else if bytes < K * K {
        format!("{} KB", bytes.div_ceil(K))
    } else {
        format!("{:.1} MB", bytes as f64 / (K * K) as f64)
    }
}

/// The folder's name for the browser's row ("/" for the root).
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Draws the browser into `ui` (the panel's column, under the drop zone).
pub(crate) fn show(create: &mut Create, ui: &mut Ui, t: &Theme, out: &mut Intents) {
    let folder = create.folder().cloned();
    ui.horizontal(|ui| {
        let up = folder.as_ref().and_then(|f| f.parent.clone());
        button(
            ui,
            t,
            &format!("{} Up", glyph::PREVIOUS),
            "Parent folder",
            up.is_some(),
            up.map_or(CreateIntent::CloseBrowser, |p| {
                CreateIntent::Browse(super::browser::BrowseAt::Path(p))
            }),
            out,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(
                ui,
                t,
                glyph::CLOSE,
                "Close browser",
                true,
                CreateIntent::CloseBrowser,
                out,
            );
            let name = folder.as_ref().map_or_else(
                || "Reading the folder…".to_owned(),
                |f| folder_name(&f.path),
            );
            let shown = ui.add(
                Label::new(
                    RichText::new(name)
                        .font(t.text.strong.clone())
                        .color(t.color.text.primary),
                )
                .truncate(),
            );
            if let Some(f) = &folder {
                shown.on_hover_text(f.path.to_string_lossy());
            }
        });
    });
    let row = t.size.create.browser_row;
    let rows_tall = row * t.size.create.browser_rows as f32;
    ui.allocate_ui(vec2(ui.available_width(), rows_tall), |ui| {
        ui.painter()
            .rect_filled(ui.max_rect(), t.radius.small, t.color.surface.recessed);
        ScrollArea::vertical()
            .id_salt("enth.create.browser")
            .auto_shrink([false, false])
            .max_height(rows_tall)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                list(folder.as_ref(), ui, t, row, out);
            });
    });
    // The typed path: a sound file is chosen, anything else opens as a folder.
    let width = ui.available_width();
    ui.horizontal(|ui| {
        let field = ui.add(
            TextEdit::singleline(&mut create.path_text)
                .hint_text("or type a path")
                .font(t.text.label.clone())
                .desired_width(width - t.size.create.choose),
        );
        field.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Sound file path")
        });
        let typed = create.path_text.trim().to_owned();
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let response = ui.add_enabled(
            !typed.is_empty(),
            egui::Button::new(RichText::new("Choose").font(t.text.body.clone())),
        );
        named(&response, "Choose file");
        if (response.clicked() || enter) && !typed.is_empty() {
            let path = PathBuf::from(typed);
            out.push(Intent::Create(if is_sound(&path) {
                CreateIntent::Choose(path)
            } else {
                CreateIntent::Browse(BrowseAt::Path(path))
            }));
        }
    });
}

/// The listing's rows.
fn list(folder: Option<&Folder>, ui: &mut Ui, t: &Theme, row: f32, out: &mut Intents) {
    let Some(folder) = folder else {
        return;
    };
    let width = ui.available_width();
    if let Some(e) = &folder.error {
        ui.add(
            Label::new(
                RichText::new(format!("{} {e}", glyph::BAD))
                    .font(t.text.label.clone())
                    .color(t.color.score.bad),
            )
            .wrap(),
        );
        return;
    }
    if folder.entries.is_empty() {
        ui.add(
            Label::new(
                RichText::new("No folders or sound files here (WAV, FLAC, Ogg Vorbis, MP3)")
                    .font(t.text.label.clone())
                    .color(t.color.text.dimmed),
            )
            .wrap(),
        );
        return;
    }
    for entry in &folder.entries {
        let (rect, response) = ui.allocate_exact_size(vec2(width, row), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, t.radius.small, t.color.surface.raised);
        }
        let inner = rect.shrink2(vec2(t.space.tight, 0.0));
        let (text, label, intent, color) = match entry.kind {
            EntryKind::Folder => (
                format!("{} {}", entry.name, glyph::PATH),
                format!("Open folder {}", entry.name),
                CreateIntent::Browse(BrowseAt::Path(entry.path.clone())),
                t.color.text.label,
            ),
            EntryKind::Sound => (
                entry.name.clone(),
                format!("Choose {}", entry.name),
                CreateIntent::Choose(entry.path.clone()),
                t.color.text.primary,
            ),
        };
        let size = entry.bytes.map(size_text).unwrap_or_default();
        let size_width = if size.is_empty() {
            0.0
        } else {
            ui.painter()
                .layout_no_wrap(size.clone(), t.text.label.clone(), color)
                .size()
                .x
        };
        let name = elided(
            ui,
            &text,
            &t.text.body,
            inner.width() - size_width - t.space.gap,
        );
        ui.painter().text(
            inner.left_center(),
            egui::Align2::LEFT_CENTER,
            name,
            t.text.body.clone(),
            color,
        );
        if !size.is_empty() {
            ui.painter().text(
                inner.right_center(),
                egui::Align2::RIGHT_CENTER,
                size,
                t.text.label.clone(),
                t.color.text.dimmed,
            );
        }
        named(&response, &label);
        if response.clicked() {
            out.push(Intent::Create(intent));
        }
    }
    if folder.more > 0 {
        ui.label(
            RichText::new(format!("and {} more", folder.more))
                .font(t.text.label.clone())
                .color(t.color.text.dimmed),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> FolderEntry {
        FolderEntry {
            name: name.to_owned(),
            path: PathBuf::from("/s").join(name),
            kind,
            bytes: None,
        }
    }

    #[test]
    fn a_listing_has_folders_first_then_sound_files_by_name_and_nothing_else() {
        let f = Folder::new(
            "/s".into(),
            vec![
                entry("b.WAV", EntryKind::Sound),
                entry("notes.txt", EntryKind::Sound),
                entry("Zeta", EntryKind::Folder),
                entry("a.mp3", EntryKind::Sound),
                entry("alpha", EntryKind::Folder),
                entry("c.flac", EntryKind::Sound),
                entry("d.ogg", EntryKind::Sound),
            ],
        );
        let names: Vec<&str> = f.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["alpha", "Zeta", "a.mp3", "b.WAV", "c.flac", "d.ogg"]
        );
        assert_eq!(f.parent.as_deref(), Some(Path::new("/")));
    }

    #[test]
    fn sizes_read_short() {
        assert_eq!(size_text(820), "820 B");
        assert_eq!(size_text(14_000), "14 KB");
        assert_eq!(size_text(1_300_000), "1.2 MB");
    }
}
