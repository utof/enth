//! **Owner: lane DUI** (the duel card, feature `duel`; lane SKD writes its minimal card first, F2d). Pre-declared by lane
//! S0e (R4/R5, freeze point F0e).
//!
//! The duel card in the editor (docs/duels.md, "The flow" and "The duel history"; docs/editor.md gets its duel section
//! from DUI): the **share line** (a human line plus the scorecard, copied on a click), the **paste box** (a whole chat
//! message in, every code that decodes pulled out), **verification** ("Checked on your computer: their patch scores
//! 93.4 %, as reported"), the two results side by side, **the sealed answer** (a friend's answer stays sealed until you
//! have submitted or given up, then you can audition their patch or load it as one undo step), the local **duel
//! history**, and the About line ("Duel codes are for friends. A code proves what a patch scores, not who made it or how
//! long it took. enth's own tools can solve any challenge.").
//!
//! **The wording is a rule** (docs/phase-4.md, "Rules for every lane"): "checked on your computer" for scores and
//! "reported" for times and assists; never "proof", "verified time", "verified player" or "anti-cheat"; no streaks,
//! leaderboards, currency or loss messages. **No networking, and the clipboard is never read:** the card writes it with
//! egui's `copy_text` inside a click on **Copy**, and a paste is the platform's own Paste into the paste box (tests pin
//! both).
//!
//! **The seam** (lane SKD, F2d). The card has no header entry of its own: **Duel** sits in the overview region's
//! bottom-left corner ([`card::opener`]), and the card takes a column at the window's right edge while it is open
//! ([`card::show`], `size.duel.pane`), as the Create panel does (the duel card wins the column while both are open).
//! `Editor::set_duel(view, shell)` hands over the session's duel read model (`enth-session`'s `duel`, lane SE) and the
//! shell's codes ([`state::ShellDuel`]) before the frame, `Intent::Duel(DuelIntent)` carries the card's clicks, and
//! `Editor::apply_duel(session)` applies them after; the shell takes **Share** with `Editor::take_duel_share()`. A build,
//! a kittest frame or a dev window that never calls `set_duel` draws exactly as before. Colours, spacing, fonts and
//! sizes come from the theme's tokens only; the duel's size tokens are in `theme/sizes/duel.rs` and nowhere else.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`card`] | the card's entry points ([`card::show`], [`card::opener`]), its rows, the About line |
//! | [`state`] | the editor's duel state, `DuelIntent`, `set_duel` and `apply_duel` (the seam) |
//! | [`share`] | the share line and the copy button (the clipboard is written on a click) |
//! | [`paste`] | the paste box (the clipboard is read only on an explicit Paste) |
//! | [`verify`] | verification and the two results side by side |
//! | [`answer`] | the sealed answer: audition, and "load it" as one undo step |
//! | [`history`] | the local duel history list and the optional names the receiver types |
//!
//! Lane S0e left the card inert; lane SKD drew the minimal card (F2d), and the history is lane DUI's.

pub mod answer;
pub mod card;
pub mod history;
pub mod paste;
pub mod share;
pub mod state;
pub mod verify;

pub use state::{Duel, DuelIntent, ShareCodes, ShellDuel};
