//! The named regions of the editor and the components that fill them (lane UI-C; ED1 places them).
//!
//! Each component draws into the `Rect` it is given, from data, and pushes what its controls ask for into
//! [`Intents`](super::cards::Intents). None of them knows where the others are: the layout (`layout.rs`, lane ED1)
//! decides the rectangles, so a layout variant changes no component.
//!
//! | Component | Region | Data |
//! | --- | --- | --- |
//! | [`Header`] | the top band: wordmark, patch name, Undo and Redo, inline knobs of built-ins placed there | [`HeaderView`], [`CardView`](super::cards::CardView)s |
//! | [`Macros`] | the macro bar | [`MacroView`] per macro |
//! | [`CardRegion`] | the sound sources (generators), voice effects, effects, modulators | [`CardView`](super::cards::CardView)s |
//! | [`SourceTiles`] | the modulation sources' live tiles with their ⊕ handles | [`SourceView`] per source |
//! | [`Matrix`] | the modulation overview: targets × sources | [`MatrixView`] |
//! | [`Overview`] | the overview region: the source tiles beside the matrix (lane ED1) | [`SourceView`]s, [`MatrixView`] |
//! | [`RouteList`] | the modulation overview as a compact list, when the grid does not fit (lane ED2) | [`MatrixView`] |
//! | [`Output`] | the output region: scope or spectrum, and the stereo meter (lane ED2) | [`OutputView`] |
//! | [`Status`] | the status line: the hovered control, its value and modulation | [`Hovered`](super::cards::Hovered) |
//!
//! Every region runs in a child `Ui` over its rectangle, clipped to it, whose scope id comes from the region's name;
//! controls inside keep the global ids of what they control ([`ids`](super::cards::ids)), so a control that moves
//! to another region keeps its state.

mod cards;
mod header;
mod macros;
mod matrix;
mod output;
mod overview;
mod routes;
mod status;
mod tiles;

pub use cards::{Arrangement, CardRegion, CardRegionOutput};
pub use header::{Header, HeaderView};
pub use macros::{MacroView, Macros};
pub use matrix::{Matrix, MatrixView};
pub use output::{Output, OutputState, OutputView};
pub use overview::{Overview, OverviewOutput};
pub use routes::RouteList;
pub use status::Status;
pub use tiles::{SourceShape, SourceTiles, SourceView};

use egui::{Rect, Ui, UiBuilder};

use super::cards::ids;

/// A child `Ui` over `rect`, clipped to it, with the region's own scope id.
fn region_ui(ui: &mut Ui, rect: Rect, name: &str) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).scope_id(ids::region(name)));
    child.shrink_clip_rect(rect);
    child
}
