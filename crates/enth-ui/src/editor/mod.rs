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
//! | `cards.rs` | module card presenters built from the catalogue's `ParamSpec`s, with stable widget ids | UI-C |
//!
//! Colours, sizes, spacing, radii, strokes, fonts and rates come from `theme` tokens only; a new value is a new
//! token in the file of its area (`theme/`), never a literal here.

pub mod cards;
pub mod layout;
pub mod regions;
