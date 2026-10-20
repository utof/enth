//! The editor (docs/design-system.md, research session 2026-10-19-p1b-plugin-editor, report 02): regions, then
//! presenters, then cards, then controls. Widgets emit intents; the editor turns them into ops through the
//! session at frame end. It depends on a narrow session trait, not on the plugin, so the same code runs in
//! kittest, the dev window and the plugin.
//!
//! The files are declared up front (P1b lane S0b) so the lanes that fill them never edit this file:
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | `regions.rs` | the named layout regions (header, macros, voice, effects, modulators, status, ...) | UI-C, ED1 |
//! | `layout.rs` | `LayoutSpec`: rows and columns sized by tokens, the section-to-region table, layout 1 ("Lanes") | ED1 |
//! | `cards.rs` | module card presenters built from the catalogue's `ParamSpec`s, with stable widget ids | UI-C, ED1 |
//! | `frame.rs` | [`Editor`]: one frame from the session through a layout, intents applied at frame end | ED1 |
//! | `overlay.rs` | the no-op [`Overlay`](overlay::Overlay) Learn and Recreate will fill (emphasis, scores) | ED1 |
//! | `describe.rs` | undo steps in the editor's words, for the Undo and Redo tooltips | ED1 |
//! | `live.rs` | telemetry from the engine's tap, and repaint gating | ED2 |
//! | `modulate.rs` | drag-to-modulate: the ⊕ payload, drop targets, the depth handle | ED2 |
//! | `reserved.rs` | the lesson, challenge-bar and results regions behind the `learn` and `recreate` features | ED1 |
//!
//! docs/editor.md describes the whole editor.
//!
//! Colours, sizes, spacing, radii, strokes, fonts and rates come from `theme` tokens only; a new value is a new
//! token in the file of its area (`theme/`), never a literal here.

pub mod cards;
mod describe;
mod frame;
pub mod layout;
pub mod live;
pub mod modulate;
pub mod overlay;
pub mod regions;
#[cfg(any(feature = "learn", feature = "recreate"))]
mod reserved;

pub use frame::{Editor, FrameReport};
pub use layout::{Layout, LayoutError, LayoutSpec, Region};
