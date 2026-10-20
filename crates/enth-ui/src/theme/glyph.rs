//! Symbols with a fixed meaning. All are covered by Inter (checked by a test). A lane that needs a glyph adds it here
//! and nowhere else.

/// Score: correct.
pub const GOOD: &str = "✓";
/// Score: close.
pub const NEAR: &str = "~";
/// Score: wrong.
pub const BAD: &str = "✗";
/// Drag handle of a modulation source.
pub const HANDLE: &str = "⊕";
/// Play / audition.
pub const PLAY: &str = "▶";
/// Separator in a parameter path ("Filter › Cutoff").
pub const PATH: &str = "›";
/// Flow to the next stage ("Send to › Master").
pub const SEND: &str = "→";
/// Bullet in lists.
pub const BULLET: &str = "•";
/// Filled and empty dots (difficulty).
pub const DOT_ON: &str = "●";
pub const DOT_OFF: &str = "○";
/// A card's module menu button (lane AM). The horizontal ellipsis U+2026: Inter has no midline ellipsis (U+22EF).
pub const MENU: &str = "…";
/// The dashed slot and the header button that add a module (lane AM).
pub const ADD: &str = "+";
