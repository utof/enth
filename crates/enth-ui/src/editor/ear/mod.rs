//! **Owner: lane EUI** (the Ear view, feature `ear`; lane SKE writes its minimal panel first, F2e). Pre-declared by lane
//! S0e (R4/R5, freeze point F0e).
//!
//! The Ear view in the editor (docs/ear.md, "The session and the Ear view"; docs/editor.md gets its Ear section from
//! EUI): a **right column like Create's**, so a parameter game keeps its card in view. **The Ear and Create columns are
//! exclusive**, and the column's overflow at 1200 x 800 is reported (an overflow report, like the layouts'). It draws
//! every round type (label choice, clip choice, A/B and odd one out, slider estimate, match it, and tap back after lane
//! TAP), the workout to its end ("done for today", with "one more" optional), the replay of A, B and the answer after
//! each item, thresholds per skill in physical units, the month calendar of practised days and the warm-up after a break.
//! Feedback talks about the sound, never the person: no streaks, no loss messages, no leaderboards, no currency.
//!
//! The seam is copied from Create and Learn (docs/editor.md, "The Recreate hook"): `Editor::set_ear(…)` before the
//! frame (the session's ear read model, `enth-session`'s `ear`, lane SE), `Intent::Ear(EarIntent)` while it runs,
//! `Editor::apply_ear(…)` after, all EUI's. The view repaints only on a worker post while visible: it asks for no frames
//! of its own. Colours, spacing, fonts and sizes come from the theme's tokens only; EUI's size tokens go in
//! `theme/sizes/ear.rs` and nowhere else. **The editor plays no audio**: the shell renders the examples and plays them
//! through the Target slot (lane SKE), and in kittest nothing does.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`header`] | the header's Ear hook (the entry point) and [`HeaderEar`], the part of `HeaderView` it reads |
//! | [`state`] | the editor's Ear state, `EarIntent`, `set_ear` and `apply_ear` (the seam) |
//! | [`panel`] | the Ear column: the game list, the workout's entry and its end |
//! | [`rounds`] | every round type: label and clip buttons, A/B, odd one out, the slider, match it |
//! | [`replay`] | the replay of A, B and the answer after each item, with words about the sound |
//! | [`progress`] | thresholds per skill over time, the practised-days calendar, the warm-up after a break |
//!
//! Lane S0e left the hook inert: it draws one **Ear** button when the shell says the Ear games are available, and the
//! click does nothing yet. The other files are empty.

pub mod header;
pub mod panel;
pub mod progress;
pub mod replay;
pub mod rounds;
pub mod state;

pub use header::HeaderEar;
