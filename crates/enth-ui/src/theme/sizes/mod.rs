//! Component tokens: per-widget dimensions, in logical px at 100 % UI scale. One file per UI area, so a lane adds
//! the dimensions of its own area in its own file: `bar`, `card`, `display`, `editor` (ED1), `handle`, `knob`,
//! `matrix`, `modulation` (ED2), `pane`, `tile`, `window`. A brand-new area is a new file plus its field here.

use serde::{Deserialize, Serialize};

mod bar;
mod card;
mod display;
mod editor;
mod handle;
mod knob;
mod matrix;
mod modulation;
mod pane;
mod tile;
mod window;

pub use bar::BarSizes;
pub use card::CardSizes;
pub use display::DisplaySizes;
pub use editor::EditorSizes;
pub use handle::HandleSizes;
pub use knob::KnobSizes;
pub use matrix::MatrixSizes;
pub use modulation::ModulationSizes;
pub use pane::PaneSizes;
pub use tile::TileSizes;
pub use window::WindowSizes;

/// Component tokens: per-widget dimensions, in logical px at 100 % UI scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sizes {
    pub window: WindowSizes,
    pub bar: BarSizes,
    pub card: CardSizes,
    pub knob: KnobSizes,
    pub tile: TileSizes,
    pub matrix: MatrixSizes,
    pub pane: PaneSizes,
    pub display: DisplaySizes,
    /// Draggable points on a curve display (an envelope's corners).
    #[serde(default)]
    pub handle: HandleSizes,
    /// Layout regions and popups of the editor (lane ED1).
    #[serde(default)]
    pub editor: EditorSizes,
    /// Drag-to-modulate and the matrix overview (lane ED2).
    #[serde(default)]
    pub modulation: ModulationSizes,
    /// Height of single-line controls: buttons, segmented controls, number fields.
    pub control_height: f32,
}

impl Sizes {
    /// The dark theme's component sizes, one area at a time.
    pub(super) fn dark() -> Self {
        Self {
            window: WindowSizes::dark(),
            bar: BarSizes::dark(),
            card: CardSizes::dark(),
            knob: KnobSizes::dark(),
            tile: TileSizes::dark(),
            matrix: MatrixSizes::dark(),
            pane: PaneSizes::dark(),
            display: DisplaySizes::dark(),
            handle: HandleSizes::dark(),
            editor: EditorSizes::dark(),
            modulation: ModulationSizes::dark(),
            control_height: 24.0,
        }
    }
}
