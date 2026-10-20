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
//! | `add_module.rs` | adding and removing modules: menus, the "+ Add" slot, the header's Add hook (`add-module`) | AM (R1/S1) |
//! | `browser.rs` | the preset browser: the header's Presets hook (`presets`) and the browse panel | PBU (R1/S1) |
//! | `recreate/` | the Recreate UI behind `recreate`: challenge bar and the header's Recreate hook, results, settings | RUI (R1/S1) |
//!
//! Lane S0r (R1/S1) declared the last three, and three header hook points: `regions/header.rs` calls one no-op
//! function in each of them, behind its feature, so AM, PBU and RUI fill their header controls in their own files.
//!
//! docs/editor.md describes the whole editor.
//!
//! Colours, sizes, spacing, radii, strokes, fonts and rates come from `theme` tokens only; a new value is a new
//! token in the file of its area (`theme/`), never a literal here.

pub mod add_module;
pub mod browser;
pub mod cards;
mod describe;
mod frame;
pub mod layout;
pub mod live;
pub mod modulate;
pub mod overlay;
#[cfg(feature = "recreate")]
pub mod recreate;
pub mod regions;
#[cfg(any(feature = "learn", feature = "recreate"))]
mod reserved;

pub use frame::{Editor, FrameReport};
pub use layout::{Layout, LayoutError, LayoutSpec, Region};
