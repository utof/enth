//! Component tokens: per-widget dimensions, in logical px at 100 % UI scale. One file per UI area, so a lane adds
//! the dimensions of its own area in its own file: `bar`, `card`, `cards` (UI-C), `display`, `editor` (ED1), `handle`,
//! `knob`, `matrix`, `modulation` (ED2), `output` (ED2), `pane`, `tile`, `window`. A brand-new area is a new file plus its field here.
//! R1/S1 (lane S0r) pre-declared three empty areas: `recreate` (RUI), `browser` (PBU) and `add_module` (AM).

use serde::{Deserialize, Serialize};

mod add_module;
mod bar;
mod browser;
mod card;
mod cards;
mod display;
mod editor;
mod handle;
mod knob;
mod matrix;
mod modulation;
mod output;
mod pane;
mod recreate;
mod tile;
mod window;

pub use add_module::AddModuleSizes;
pub use bar::BarSizes;
pub use browser::BrowserSizes;
pub use card::CardSizes;
pub use cards::CardsSizes;
pub use display::DisplaySizes;
pub use editor::EditorSizes;
pub use handle::HandleSizes;
pub use knob::KnobSizes;
pub use matrix::MatrixSizes;
pub use modulation::ModulationSizes;
pub use output::OutputSizes;
pub use pane::PaneSizes;
pub use recreate::RecreateSizes;
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
    /// The catalogue cards' generated controls and envelope displays (lane UI-C).
    #[serde(default)]
    pub cards: CardsSizes,
    /// The output region: scope, spectrum and meters (lane ED2).
    #[serde(default)]
    pub output: OutputSizes,
    /// The Recreate UI: challenge bar, results drawer, randomizer settings (lane RUI; empty until it adds tokens).
    #[serde(default)]
    pub recreate: RecreateSizes,
    /// The preset browser (lane PBU; empty until it adds tokens).
    #[serde(default)]
    pub browser: BrowserSizes,
    /// Adding and removing modules (lane AM; empty until it adds tokens).
    #[serde(default)]
    pub add_module: AddModuleSizes,
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
            cards: CardsSizes::dark(),
            output: OutputSizes::dark(),
            recreate: RecreateSizes::dark(),
            browser: BrowserSizes::dark(),
            add_module: AddModuleSizes::dark(),
            control_height: 24.0,
        }
    }
}
