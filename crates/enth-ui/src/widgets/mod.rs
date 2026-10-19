//! Components. Each takes `&Theme` and reads only semantic and component tokens; none defines a
//! colour, size, font or rate of its own.

mod bars;
mod display;
mod handles;
mod knob;
mod lesson_pane;
mod matrix_grid;
mod module_card;
mod number_field;
mod paint;
mod segmented;
mod source_tile;

pub use bars::{ChallengeBar, Macro, MacroBar, StatusLine};
pub use display::{Display, Ink, Trace};
pub use handles::{CurveHandles, Drag, Handle, HandleEdit};
pub use knob::{Emphasis, Knob, KnobLayout};
pub use lesson_pane::{Block, LessonPane, Span};
pub use matrix_grid::MatrixGrid;
pub use module_card::{CardResponse, CardShape, ModuleCard};
pub use number_field::NumberField;
pub use segmented::Segmented;
pub use source_tile::SourceTile;
