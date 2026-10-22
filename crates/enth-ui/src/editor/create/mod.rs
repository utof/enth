//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! "Create from sound" in the editor (docs/r3.md, "The Create view and the session"; docs/editor.md, "The Create
//! view"): a drop zone showing the file's name, length and detected note, and the intake's findings and warnings; a
//! folder browser where drops do not reach the editor; one progress bar (renders against the cap, time against the
//! cap), the progress per rung of successive halving, and Cancel; **K candidate tiles**, each with its rank, where its
//! start came from, its distance and its state (proposed, searching, done or pruned); and a compare area for the
//! selected tile: ▶ the candidate, ▶ its start, ▶ the upload, A/B against the upload (level-matched), "Use this" and
//! "Use the start" (one undo step each). Branches keep both the proposal and the refined patch, so you hear what the
//! start gave and what search added.
//!
//! The seam is copied from Recreate and Learn (docs/editor.md, "The Recreate hook"): `Editor::set_create(…)` before the
//! frame, `Intent::Create(CreateIntent)` while it runs, `Editor::apply_create(…)` after (the read model and the edit
//! trait come from `enth-session`'s `create`, lane SK3), and for the folder browser `Editor::take_create_browse()`
//! after the frame and `Editor::set_create_folder(…)` with the shell's listing (the editor does no I/O).
//!
//! The view repaints only on a worker post while visible: it asks for no frames of its own, as the Recreate worker's
//! results and Learn's wake the editor. Colours, spacing, fonts and sizes come from the theme's tokens only; PUI's
//! size tokens are in `theme/sizes/create.rs` and nowhere else. A dropped file reaches the view as egui's
//! `dropped_files` (fork patch 11, lane FK3), wherever it lands on the window.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`header`] | the header's Create hook (the entry point) and [`HeaderCreate`], the part of `HeaderView` it reads |
//! | [`state`] | the editor's Create state, [`CreateIntent`], `apply_create`, `set_create` and the browser's seam |
//! | [`panel`] | the panel's column: the title, the drop zone, Browse… and Start, then the browser or the job |
//! | [`drop_zone`] | the drop zone and the intake's findings: length, note, trim, damage, warnings |
//! | [`browser`] | the in-editor folder browser and the listing the shell hands over ([`browser::Folder`]) |
//! | [`progress`] | the phase, Cancel, the progress bar and the rungs |
//! | [`tiles`] | the K candidate tiles: rank, provenance, distance, state; a click selects |
//! | [`audition`] | the compare area: ▶ candidate, start and upload, A/B against the upload, Use this, Use the start |

pub mod audition;
pub mod browser;
pub mod drop_zone;
pub mod header;
pub mod panel;
pub mod progress;
pub mod state;
pub mod tiles;

pub use browser::{BrowseAt, EntryKind, Folder, FolderEntry};
pub use header::HeaderCreate;
pub use state::{Create, CreateIntent};
