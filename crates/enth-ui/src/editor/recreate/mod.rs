//! **Owner: lane RUI** (the Recreate UI, feature `recreate`). Pre-declared by lane S0r (R1/S1); lane SKR's
//! skeleton (F2r) played the first rounds through it.
//!
//! Recreate in the editor (report 01 §2.6, report 03, summary "Recreate and Learn in the layout"): a mode is a layout
//! plus an [`Overlay`](super::overlay::Overlay). The challenge bar shares the header row (the Presets group steps
//! aside in Recreate mode), the results drawer takes the overview slot once there is something to show, the score
//! rings draw on the player's controls through the overlay, and the randomizer settings page opens over the body.
//! Sizes are in `theme/sizes/recreate.rs`.
//!
//! | File | Holds |
//! | --- | --- |
//! | `state` | [`Recreate`] (the editor's Recreate state), [`GameIntent`] and [`ViewIntent`], [`apply_game`], [`BarView`] |
//! | [`bar`] | the challenge bar in the header row: "Recreate" in Play; the level, the code field, Start and Settings… before a round; Target ▶ / Mine ▶ / A/B, the timer, Submit, Hint n/4, Reveal and End during one |
//! | [`results`] | the results drawer: the blind rating, then sound % against the pass mark, structure and combined %, the components with their badges and words, the time, the hint, the vote, "Disagree…" and the next actions |
//! | [`settings`] | the randomizer settings page: level and difficulty, an amount per category, musical, the template, the `dim_unused` assist; it makes a `RandomSpec` and nothing else |
//! | [`overlay`] | the score overlay: Good / Near / Bad rings on knobs, switches, routes and enable toggles after the first Submit, live after that; the `dim_unused` dimming |
//!
//! **No peeking.** Nothing in Recreate shows a target value or a descriptor reading during a round: the drawer
//! shows the components' badges and `component_text` words, never their `target` or `answer` readings (lane SS's
//! rule), and no tooltip, menu, status text or accessible name carries one (`tests/recreate.rs`, the no-peek test).

pub mod bar;
pub mod overlay;
pub mod results;
pub mod settings;
mod state;

pub use state::{BarView, GameIntent, Recreate, ViewIntent, apply_game, clock, percent};
