//! **Owner: lane RUI** (the Recreate UI, feature `recreate`). Pre-declared by lane S0r (R1/S1).
//!
//! Recreate in the editor (report 01 §2.6, report 03, summary "Recreate and Learn in the layout"): a mode is a layout
//! plus an [`Overlay`](super::overlay::Overlay). The challenge bar shares the header row, the results drawer takes the
//! overview slot, and the randomizer settings page opens over the body. RUI replaces the empty panels `reserved.rs`
//! draws for the challenge-bar and results regions. Sizes go in `theme/sizes/recreate.rs`.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`bar`] | the challenge bar (difficulty dots, Target ▶ / Mine ▶ / A/B, timer, Hint, Submit) and the header's Recreate hook |
//! | [`results`] | the results drawer: sound, structure and combined %, components, time, Agree / Disagree |
//! | [`settings`] | the randomizer settings page: level, amounts, musical, template, seed or code |

pub mod bar;
pub mod results;
pub mod settings;
