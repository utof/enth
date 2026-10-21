//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2).
//!
//! Learn in the editor (docs/learn.md, "The Learn view"; docs/editor.md gets its Learn section from LUI): a lesson is a
//! layout plus an [`Overlay`](super::overlay::Overlay), and a lesson's Recreate step uses the Recreate UI unchanged.
//! The seam is copied from Recreate (docs/editor.md, "The Recreate hook"): `Editor::set_lesson(LearnEdit::lesson())`
//! before the frame, `Intent::Learn(LearnIntent)` while it runs, `Editor::apply_learn(&mut dyn LearnEdit)` after. LUI
//! adds all three, the overlay's `Emphasis::Highlighted` and `locked` flag (additive in `overlay.rs`), the `Lesson`
//! region's presenter (replacing the empty panel `reserved.rs` draws), the built-in layout `layout/learn.json` (in
//! `BUILTIN`, `layout.rs`) and the tokens in `theme/sizes/learn.rs`. Learn asks for no frames of its own: job results
//! wake the editor, as the Recreate worker's do.
//!
//! The markdown is our own small renderer: `enth-learn` parses a step into a block AST with `pulldown-cmark`, and the
//! lesson pane draws it with design tokens only (`egui_commonmark` was rejected for now; docs/learn.md).
//!
//! | File | Holds |
//! | --- | --- |
//! | [`header`] | the header's Learn hook (the entry point) and [`HeaderLearn`], the part of `HeaderView` it reads |
//! | [`state`] | the editor's Learn state, `LearnIntent` and `apply_learn`, `set_lesson` (the seam) |
//! | [`pane`] | the lesson pane: the step's text from the block AST, hear / try / check controls, hints |
//! | [`browser`] | the lesson browser, its Review entry and the Packs folder buttons |
//!
//! Lane S0l left the hook inert. **Lane SKL** (F2l) wired the minimal seam LUI grows: the click toggles a plain
//! Learn panel at the window's right edge ([`pane`]), the editor's [`Learn`] state with [`LearnIntent`], `set_lesson`,
//! `set_lessons` and `apply_learn`, and [`LessonOverlay`] (highlights through the hint outline, locks dimmed,
//! hidden modules dropped). `browser.rs` is still empty.

pub mod browser;
pub mod header;
pub mod pane;
pub mod state;

pub use header::HeaderLearn;
pub use state::{Learn, LearnIntent, LessonOverlay};
