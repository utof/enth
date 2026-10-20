//! **Owner: lane PBU** (the preset browser UI; after lane PBC's preset core). Pre-declared by lane S0r (R1/S1).
//!
//! The preset browser (report 03 §2.3, summary "Presets"; docs/presets.md, "The browser"): the header's
//! "◀ Name ▶ Browse Save" through the Presets hook, and three sheets drawn over the body in any layout (they are
//! editor view state, not layout regions, and while one is open the body's regions are not drawn at all):
//!
//! | Sheet | What it is |
//! | --- | --- |
//! | the browse panel ([`panel`]) | All, Factory, User and Favorites; a word search with `#tag` and `by:author`; a click loads, a double-click loads and closes, Escape closes; ▶ held auditions; ☆ stars |
//! | the Save dialog ([`save`]) | a name, a folder, tags from the vocabulary; overwrite protection with a confirm; the store's and the session's refusals with their hints |
//! | the crash report ([`crash`]) | opened from the "enth stopped unexpectedly last time" notice, which never blocks |
//!
//! Everything that changes the session goes through [`Intent::Preset`](crate::editor::cards::Intent::Preset) at
//! the end of the frame, like every other control: a load is the session's undoable preset load (lane PBC's
//! `SessionEdit::load_preset`, one undo step, delivered like a host state load with **no host parameter writes**,
//! marking the project dirty), never host automation and never `GuiContext::set_state`. The sheets' own view state
//! (the tab, the search text, the dialog's fields) is edited in place while drawing.
//!
//! The UI crate does no I/O: the shell hands the editor a [`PresetStore`] (the plugin's `FsStore` over the user
//! folder; [`MemStore`] here by default, in the dev window and in tests) and, at each open, the crash files waiting
//! to be offered as a [`CrashReports`]. Sizes are in `theme/sizes/browser.rs`.

mod crash;
#[cfg(feature = "presets")]
mod header;
mod panel;
mod save;

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, FontId, Galley, Rect, Response, Role, Sense, Ui, WidgetInfo, vec2};
use enth_session::presets::{self as core, Favorites, Library, PresetEntry};
use enth_session::{
    MemStore, PresetError, PresetKey, PresetState, PresetStore, Query, SessionEdit, SessionError,
    SessionView, Source,
};

pub use crash::CrashReports;
#[cfg(feature = "presets")]
pub(crate) use header::header;

use super::layout::{Placed, Region};
use crate::Theme;

/// The key an audition plays: middle C, the dev window's F3 note.
const AUDITION_KEY: u8 = 60;
/// Its velocity, as a MIDI value out of [`MIDI_MAX`].
const AUDITION_VELOCITY: u8 = 100;
const MIDI_MAX: u8 = 127;

/// What the header's Presets hook shows (part of [`HeaderView`](super::regions::HeaderView)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a read model: independent facts the header shows, not a state machine"
)]
pub struct HeaderPresets {
    /// The name between ◀ and ▶: the last loaded or saved preset's ([`PresetState::last_key`]), else the patch's;
    /// empty for an unnamed patch ("Untitled").
    pub name: String,
    /// The sound came from a preset (the instance has a "last loaded" bookmark).
    pub from_preset: bool,
    /// It was edited since that preset was loaded or saved ([`HistoryView::is_dirty`](enth_session::HistoryView)):
    /// the header marks the name with a dot.
    pub edited: bool,
    /// The browse panel is open (Browse shows as pressed).
    pub browsing: bool,
    /// The Save dialog is open (Save shows as pressed).
    pub saving: bool,
}

/// What the browser's controls ask for, applied at the end of the frame by [`Presets::apply`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetIntent {
    /// ◀ (−1) or ▶ (+1): load the previous or next preset of the browser's current list (its tab and search),
    /// wrapping round the ends ([`core::step`]); from no bookmark, ▶ gives the first and ◀ the last.
    Step(isize),
    /// Browse: open the panel, with a fresh scan of the store, or close it.
    Browse(bool),
    /// Save: open the Save dialog, filled from the patch's name and tags and the bookmark's folder.
    OpenSave,
    /// Close whatever covers the body (the panel, the dialog, the report): its ×, Cancel, or Escape.
    Close,
    /// Load a preset: a click on its row; `close`: and close the panel (a double-click).
    Load { key: PresetKey, close: bool },
    /// ▶ pressed on a row: load the preset unless it is the one loaded, then hold an audition note until the pointer
    /// is released anywhere.
    Audition(PresetKey),
    /// Star or unstar a preset (saved by the store at once).
    Favorite { key: PresetKey, on: bool },
    /// The Save dialog's "Save preset", or "Replace" after it said the name exists (`overwrite`).
    Save { overwrite: bool },
    /// The crash notice's "Show report": the newest report over the body, every pending one marked as offered.
    ShowCrash,
    /// The crash notice's "Dismiss": every pending report marked as offered, nothing shown.
    DismissCrash,
}

/// What covers the body, if anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sheet {
    /// Nothing: the layout's regions draw.
    #[default]
    Closed,
    /// The browse panel.
    Browse,
    /// The Save dialog.
    Save,
    /// The crash report.
    Report,
}

/// The browse panel's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Factory first, then the user's presets.
    #[default]
    All,
    /// The presets shipped with the plugin (read-only).
    Factory,
    /// The user folder's presets.
    User,
    /// The starred presets of both.
    Favorites,
}

impl Tab {
    /// Every tab, in the panel's order.
    pub const ALL: [Tab; 4] = [Tab::All, Tab::Factory, Tab::User, Tab::Favorites];

    /// The tab's name.
    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Factory => Source::Factory.label(),
            Tab::User => Source::User.label(),
            Tab::Favorites => "Favorites",
        }
    }

    fn source(self) -> Option<Source> {
        match self {
            Tab::Factory => Some(Source::Factory),
            Tab::User => Some(Source::User),
            Tab::All | Tab::Favorites => None,
        }
    }
}

/// A line the browser shows: what went wrong and what to do about it, or what just happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// One sentence.
    pub text: String,
    /// What to do about it, for errors (`PresetError::hint`, `SessionError::hint`).
    pub hint: Option<String>,
    /// Shown as an error.
    pub error: bool,
}

impl Message {
    fn info(text: impl Into<String>) -> Self {
        Message {
            text: text.into(),
            hint: None,
            error: false,
        }
    }

    fn preset(e: &PresetError) -> Self {
        Message {
            text: e.to_string(),
            hint: Some(e.hint()),
            error: true,
        }
    }

    fn session(e: &SessionError) -> Self {
        Message {
            text: e.to_string(),
            hint: Some(e.hint()),
            error: true,
        }
    }
}

/// Releases the audition note an [`PresetIntent::Audition`] holds (the note's key stays inside, so this crate needs
/// no engine types).
type Release = Box<dyn FnOnce(&mut dyn SessionEdit) + Send + Sync>;

/// The editor's preset state (lane PBU): the store, the scanned library, the open sheet and its view state, the held
/// audition note and the crash offer. The [`Editor`](super::Editor) owns one; see the [module docs](self).
pub struct Presets {
    store: Box<dyn PresetStore + Send + Sync>,
    /// The last scan; `None` before the first and after a save (the next use scans again).
    library: Option<Library>,
    /// The favorites as the browser knows them: the scan's, then every successful toggle (the library keeps the ones
    /// it was built with).
    favorites: Favorites,
    /// Bumped by every scan and favorite toggle, so the cached list knows when to search again.
    generation: u64,
    sheet: Sheet,
    tab: Tab,
    search: String,
    /// The current list (the tab and the search), and what it was computed for.
    shown: Vec<PresetEntry>,
    shown_for: Option<(Tab, String, u64)>,
    save: save::SaveForm,
    message: Option<Message>,
    /// An audition was asked for and the pointer is still down (a failed one too, so it is not retried every frame).
    holding: bool,
    release: Option<Release>,
    crash: crash::CrashOffer,
    /// A sheet opened or closed in `apply`: the frame asks egui for one more pass to show it.
    changed: bool,
}

impl std::fmt::Debug for Presets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Presets")
            .field("sheet", &self.sheet)
            .field("tab", &self.tab)
            .field("search", &self.search)
            .field("scanned", &self.library.is_some())
            .field("message", &self.message)
            .field("holding", &self.holding)
            .finish_non_exhaustive()
    }
}

impl Default for Presets {
    /// The embedded factory presets in memory ([`MemStore`]), no user folder on disk: the dev window and tests. The
    /// plugin hands the editor its folder store ([`Presets::set_store`]).
    fn default() -> Self {
        Presets {
            store: Box::new(MemStore::default()),
            library: None,
            favorites: Favorites::default(),
            generation: 0,
            sheet: Sheet::Closed,
            tab: Tab::All,
            search: String::new(),
            shown: Vec::new(),
            shown_for: None,
            save: save::SaveForm::default(),
            message: None,
            holding: false,
            release: None,
            crash: crash::CrashOffer::default(),
            changed: false,
        }
    }
}

/// The preset the session last loaded or saved, from the instance's bookmark.
fn last_key(session: &dyn SessionView) -> Option<PresetKey> {
    session
        .instance()
        .preset
        .as_ref()
        .and_then(PresetState::last_key)
}

impl Presets {
    /// Use `store` for every scan, load, save and favorite from now on (the plugin's `FsStore::from_env()`); the next
    /// use scans it.
    pub fn set_store(&mut self, store: Box<dyn PresetStore + Send + Sync>) {
        self.store = store;
        self.library = None;
        self.generation += 1;
    }

    /// Offer the crash reports the shell found (the plugin: `crash::pending` at every open of the window), or none.
    /// While some wait, a small notice in the body's corner offers to show the newest; it never blocks the editor.
    pub fn offer_crash(&mut self, reports: Option<Box<dyn CrashReports>>) {
        self.crash.offer(reports);
    }

    /// What covers the body.
    pub fn sheet(&self) -> Sheet {
        self.sheet
    }

    /// The panel's tab.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The search field's text.
    pub fn search(&self) -> &str {
        &self.search
    }

    /// The last error or confirmation, until the next action.
    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    /// The Save dialog's error (with its hint), if the last save was refused.
    pub fn save_error(&self) -> Option<&Message> {
        self.save.error.as_ref()
    }

    /// The preset the Save dialog asks to replace, after a save hit an existing name.
    pub fn save_confirm(&self) -> Option<&PresetKey> {
        self.save.confirm.as_ref()
    }

    /// The last scan, if there was one since the store was set or saved to.
    pub fn library(&self) -> Option<&Library> {
        self.library.as_ref()
    }

    /// Whether the crash notice is showing.
    pub fn crash_notice(&self) -> bool {
        self.crash.notice()
    }

    /// The crash report shown over the body, if it is open.
    pub fn crash_report(&self) -> Option<&str> {
        self.crash.text()
    }

    /// Whether a sheet covers the body: the editor then draws it instead of the body's regions.
    pub fn covers_body(&self) -> bool {
        self.sheet != Sheet::Closed
    }

    /// The current list: the tab's presets that match the search, best first (browsing order with no words).
    pub fn list(&mut self) -> &[PresetEntry] {
        let wanted = (self.tab, self.search.clone(), self.generation);
        if self.shown_for.as_ref() != Some(&wanted) {
            self.shown = match &self.library {
                Some(library) => {
                    let query = Query {
                        text: self.search.clone(),
                        source: self.tab.source(),
                        ..Query::default()
                    };
                    library
                        .search(&query)
                        .into_iter()
                        .filter(|e| self.tab != Tab::Favorites || self.favorites.contains(&e.key))
                        .cloned()
                        .collect()
                }
                None => Vec::new(),
            };
            self.shown_for = Some(wanted);
        }
        &self.shown
    }

    /// Whether `key` is starred.
    pub fn is_favorite(&self, key: &PresetKey) -> bool {
        self.favorites.contains(key)
    }

    /// What the header's Presets hook shows.
    pub fn header(&self, session: &dyn SessionView) -> HeaderPresets {
        let last = last_key(session);
        let name = match &last {
            Some(key) => self
                .library
                .as_ref()
                .and_then(|l| l.get(key))
                .map_or_else(|| key.name().to_owned(), |e| e.title().to_owned()),
            None => session.patch().meta.name.clone(),
        };
        HeaderPresets {
            name,
            from_preset: last.is_some(),
            edited: last.is_some() && session.history().is_dirty(),
            browsing: self.sheet == Sheet::Browse,
            saving: self.sheet == Sheet::Save,
        }
    }

    /// Whether a sheet opened or closed since the last call (the frame then asks egui for one more pass).
    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn set_sheet(&mut self, sheet: Sheet) {
        if self.sheet != sheet {
            self.sheet = sheet;
            self.changed = true;
        }
    }

    fn rescan(&mut self) {
        let library = self.store.scan();
        self.favorites = library.favorites().clone();
        self.library = Some(library);
        self.generation += 1;
    }

    fn ensure_scanned(&mut self) {
        if self.library.is_none() {
            self.rescan();
        }
    }

    fn open_browse(&mut self) {
        self.rescan();
        self.set_sheet(Sheet::Browse);
    }

    /// Load `key` through the session (one undo step); a refusal becomes the message.
    fn load(&mut self, session: &mut dyn SessionEdit, key: &PresetKey) -> bool {
        match core::load_from(session, &*self.store, key) {
            Ok(_) => {
                self.message = None;
                true
            }
            Err(e) => {
                self.message = Some(Message::preset(&e));
                false
            }
        }
    }

    /// Apply one of the browser's intents (the editor's frame end, in order with the other controls' intents).
    pub fn apply(&mut self, session: &mut dyn SessionEdit, intent: &PresetIntent) {
        match intent {
            PresetIntent::Step(delta) => {
                self.ensure_scanned();
                let last = last_key(session);
                let next = {
                    let list: Vec<&PresetEntry> = self.list().iter().collect();
                    core::step(&list, last.as_ref(), *delta).map(|e| e.key.clone())
                };
                match next {
                    // A file that went away since the scan: the panel opens on a fresh scan, with the reason.
                    Some(key) => {
                        if !self.load(session, &key) {
                            self.open_browse();
                        }
                    }
                    None => self.message = Some(Message::info("No presets in this list")),
                }
            }
            PresetIntent::Browse(true) => {
                self.message = None;
                self.open_browse();
            }
            PresetIntent::Browse(false) | PresetIntent::Close => self.set_sheet(Sheet::Closed),
            PresetIntent::OpenSave => {
                let from_browse = self.sheet == Sheet::Browse;
                let location = self.store.user_location();
                self.save = save::SaveForm::open(session, last_key(session), location, from_browse);
                self.set_sheet(Sheet::Save);
            }
            PresetIntent::Load { key, close } => {
                if self.load(session, key) && *close {
                    self.set_sheet(Sheet::Closed);
                }
            }
            PresetIntent::Audition(key) => {
                self.release_note(session);
                self.holding = true;
                if last_key(session).as_ref() != Some(key) && !self.load(session, key) {
                    return;
                }
                let velocity = f32::from(AUDITION_VELOCITY) / f32::from(MIDI_MAX);
                match session.audition_on(AUDITION_KEY, velocity) {
                    Ok(note) => {
                        self.release = Some(Box::new(move |s: &mut dyn SessionEdit| {
                            let _ = s.audition_off(note);
                        }));
                    }
                    Err(e) => self.message = Some(Message::session(&e)),
                }
            }
            PresetIntent::Favorite { key, on } => match self.store.set_favorite(key, *on) {
                Ok(()) => {
                    self.favorites.set(key, *on);
                    self.generation += 1;
                }
                Err(e) => self.message = Some(Message::preset(&e)),
            },
            PresetIntent::Save { overwrite } => self.save_preset(session, *overwrite),
            PresetIntent::ShowCrash => {
                self.crash.show();
                self.set_sheet(Sheet::Report);
            }
            PresetIntent::DismissCrash => self.crash.dismiss(),
        }
    }

    fn save_preset(&mut self, session: &mut dyn SessionEdit, overwrite: bool) {
        let request = self.save.request(overwrite);
        match session.save_preset(&mut *self.store, &request) {
            Ok(_) => {
                self.message = Some(Message::info(format!("Saved {}", request.name.trim())));
                self.save.error = None;
                self.save.confirm = None;
                self.library = None;
                self.generation += 1;
                if self.save.from_browse {
                    self.open_browse();
                    // Show the saved preset where it went.
                    if self.tab == Tab::Factory {
                        self.tab = Tab::User;
                    }
                } else {
                    self.set_sheet(Sheet::Closed);
                }
            }
            Err(PresetError::Exists { key }) => {
                self.save.confirm = Some(key);
                self.save.error = None;
            }
            Err(e) => {
                self.save.confirm = None;
                self.save.error = Some(Message::preset(&e));
            }
        }
    }

    fn release_note(&mut self, session: &mut dyn SessionEdit) {
        if let Some(release) = self.release.take() {
            release(session);
        }
    }

    /// The end of a frame: an audition note is held only while a pointer button is (released anywhere, like a drag).
    pub(crate) fn settle(&mut self, session: &mut dyn SessionEdit, pointer_down: bool) {
        if !pointer_down {
            self.holding = false;
            self.release_note(session);
        }
    }

    /// The window closed: no sheet survives it (every open is a fresh one), and the shell releases audition notes.
    pub(crate) fn closed(&mut self) {
        self.sheet = Sheet::Closed;
        self.holding = false;
        self.release = None;
    }

    /// Draw the open sheet over `body` (the editor skips the regions under it). `session` is read for the bookmark.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        body: Rect,
        session: &dyn SessionView,
        out: &mut super::cards::Intents,
    ) {
        match self.sheet {
            Sheet::Closed => {}
            Sheet::Browse => panel::show(self, ui, t, body, last_key(session).as_ref(), out),
            Sheet::Save => save::show(&mut self.save, ui, t, body, out),
            Sheet::Report => crash::show_report(&self.crash, ui, t, body, out),
        }
    }

    /// Draw the crash notice in `body`'s lower right corner, while reports wait.
    pub(crate) fn show_notice(
        &self,
        ui: &Ui,
        t: &Theme,
        body: Rect,
        out: &mut super::cards::Intents,
    ) {
        if self.crash.notice() && self.sheet != Sheet::Report {
            crash::show_notice(ui, t, body, out);
        }
    }
}

/// Where the sheets go: the bounding box of every region but the header, the output, the status line and the
/// challenge bar (the header row and the bottom line stay). `None` for a layout with nothing else.
pub(crate) fn body(regions: &[Placed]) -> Option<Rect> {
    regions
        .iter()
        .filter(|p| !is_chrome(p.region))
        .map(|p| p.rect)
        .reduce(Rect::union)
}

/// Whether `placed` is under a sheet over `body` (and so is not drawn): every region that overlaps it, but the header
/// and the status line.
pub(crate) fn covered(placed: &Placed, body: Rect) -> bool {
    !matches!(placed.region, Region::Header | Region::Status) && placed.rect.intersects(body)
}

fn is_chrome(region: Region) -> bool {
    #[cfg(feature = "recreate")]
    if region == Region::ChallengeBar {
        return true;
    }
    matches!(region, Region::Header | Region::Status | Region::Output)
}

// ---------------------------------------------------------------------------------------------------------------
// Small drawing helpers shared by the sheets and the header
// ---------------------------------------------------------------------------------------------------------------

/// `text` laid out on one line, cut with "…" at `width`.
fn fitted(ui: &Ui, text: &str, font: &FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font.clone(), color);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// Paints `text` cut to `rect`'s width, left-aligned and vertically centred.
fn text_left(ui: &Ui, rect: Rect, text: &str, font: &FontId, color: Color32) {
    let galley = fitted(ui, text, font, color, rect.width());
    let pos = Align2::LEFT_CENTER
        .align_size_within_rect(galley.size(), rect)
        .min;
    ui.painter().galley(pos, galley, color);
}

/// A square button of one glyph over `rect`, labelled `label` for screen readers and tests.
fn glyph_button(
    ui: &Ui,
    t: &Theme,
    rect: Rect,
    id: egui::Id,
    glyph: &str,
    ink: Color32,
    label: &str,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        t.text.body.clone(),
        ink,
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    response
}

/// A square of the control height, for a glyph button in a row of controls.
fn square(t: &Theme) -> egui::Vec2 {
    vec2(t.size.control_height, t.size.control_height)
}
