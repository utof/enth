//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); lane SKL wired the minimal
//! seam (F2l), which LUI grew into the view.
//!
//! Learn in the editor (docs/learn.md, "The Learn view"; docs/editor.md, "The Learn view (lane LUI)"): a lesson is a
//! layout plus an [`Overlay`](super::overlay::Overlay), and a lesson's Recreate step uses the Recreate UI unchanged.
//! The seam is copied from Recreate (docs/editor.md, "The Recreate hook"): `Editor::set_lesson(LearnEdit::lesson())`
//! before the frame, `Intent::Learn(LearnIntent)` while it runs, `Editor::apply_learn(&mut dyn LearnEdit)` after;
//! `Editor::set_library` when the window opens. While the pane shows, the editor draws the built-in layout `learn`
//! (`layout/learn.json`: the `lesson` region as a column right of the cards, under the header row), unless its own
//! layout places the `lesson` region; hidden ("collapsed"), the lesson goes on over the whole editor and the header's
//! entry reads "Lesson n/m". Learn asks for no frames of its own: job results wake the editor, as the Recreate
//! worker's do.
//!
//! The markdown is our own small renderer: `enth-learn` parses a step into a block AST with `pulldown-cmark`, and the
//! lesson pane draws it with design tokens only (`egui_commonmark` was rejected for now; docs/learn.md).
//!
//! | File | Holds |
//! | --- | --- |
//! | [`header`] | the header's Learn hook (the entry point, "Learn" or "Lesson n/m") and [`HeaderLearn`] |
//! | [`state`] | the editor's Learn state ([`Learn`]), [`LearnIntent`], the browser's data, [`LessonOverlay`] (highlights, locks, hidden modules, focus, the hovered link) |
//! | [`pane`] | the lesson pane: the title and progress rows, the step's text from the block AST with its links and widgets, the bottom-anchored controls |
//! | [`browser`] | the lesson browser: the tour first, every pack, progress per lesson, the Review entry |

pub mod browser;
pub mod header;
pub mod pane;
pub mod state;

pub use header::HeaderLearn;
pub use state::{
    Learn, LearnIntent, LessonOverlay, LessonStanding, PackInfo, Shelf, TrackEntry, TrackInfo,
};
