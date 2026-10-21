//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3, freeze point F0s).
//!
//! "Create from sound" in the editor (docs/r3.md, "The Create view and the session"; docs/editor.md gets its Create
//! section from PUI): a drop zone showing the file's name, length, detected pitch and the intake's warnings; **K
//! candidate tiles**, each with its distance and state (proposed, searching, done or pruned); one progress bar (renders
//! against the budget, time left) and Cancel; per tile a ▶ audition and A/B against the upload (Recreate's `Mailbox` and
//! `Player`, level-matched) and "Use this". Branches keep both the proposal and the refined patch, so you hear what the
//! start gave and what search added. The seam is copied from Recreate and Learn (docs/editor.md, "The Recreate hook"):
//! `Editor::set_create(…)` before the frame, `Intent::Create(CreateIntent)` while it runs, `Editor::apply_create(…)`
//! after, all PUI's (the read model and the edit trait come from `enth-session`'s `create`, lane SK3).
//!
//! The view repaints only on a worker post while visible: it asks for no frames of its own, as the Recreate worker's
//! results and Learn's wake the editor. Colours, spacing, fonts and sizes come from the theme's tokens only; PUI's
//! size tokens go in `theme/sizes/create.rs` and nowhere else. A dropped file reaches the view as egui's
//! `dropped_files` (fork patch 11, lane FK3); the folder browser is the fallback (no `rfd`).
//!
//! | File | Holds |
//! | --- | --- |
//! | [`header`] | the header's Create hook (the entry point) and [`HeaderCreate`], the part of `HeaderView` it reads |
//! | [`state`] | the editor's Create state, `CreateIntent` and `apply_create`, `set_create` (the seam) |
//! | [`drop_zone`] | the drop zone and the file summary: name, length, detected pitch, the intake's warnings |
//! | [`browser`] | the in-editor folder browser (the fallback where drops do not reach the editor) |
//! | [`tiles`] | the K candidate tiles: distance, state, provenance, "Use this" |
//! | [`audition`] | ▶ audition and A/B against the upload, level-matched |
//! | [`progress`] | the progress bar (renders against the budget, time left) and Cancel |
//!
//! Lane S0s left the hook inert: it draws one **Create** button when the shell says create is available, and the click
//! does nothing yet. The other files are empty.

pub mod audition;
pub mod browser;
pub mod drop_zone;
pub mod header;
pub mod progress;
pub mod state;
pub mod tiles;

pub use header::HeaderCreate;
