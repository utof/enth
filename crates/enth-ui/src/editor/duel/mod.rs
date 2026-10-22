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
//! leaderboards, currency or loss messages. **No networking and no clipboard read of its own:** the card asks the shell to
//! write the clipboard on a click and to read it only on an explicit Paste (a test pins it).
//!
//! **The seam.** The card has no header entry of its own: it belongs with a Recreate round's results (a copy button after
//! a pass) and with a paste box that takes challenge codes too, so it attaches where the Recreate results drawer ends and
//! beside the code field. [`card::DuelCardView`] is the part of the shell's duel read model it reads (`available` is false
//! by default, so a build, a kittest frame or a dev window that never sets it draws exactly as before) and
//! [`card::card`] its inert entry point; SKD wires the one call and the editor's duel state, copying Recreate's and
//! Create's seam: `Editor::set_duel(…)` before the frame (the session's duel read model, `enth-session`'s `duel`, lane
//! SE), `Intent::Duel(DuelIntent)` while it runs, `Editor::apply_duel(…)` after, all DUI's. Colours, spacing, fonts and
//! sizes come from the theme's tokens only; DUI's size tokens go in `theme/sizes/duel.rs` and nowhere else.
//!
//! | File | Holds |
//! | --- | --- |
//! | [`card`] | the card's entry point and [`DuelCardView`], the part of the duel read model it reads; the About line |
//! | [`state`] | the editor's duel state, `DuelIntent`, `set_duel` and `apply_duel` (the seam) |
//! | [`share`] | the share line and the copy button (the clipboard is written on a click) |
//! | [`paste`] | the paste box (the clipboard is read only on an explicit Paste) |
//! | [`verify`] | verification and the two results side by side |
//! | [`answer`] | the sealed answer: audition, and "load it" as one undo step |
//! | [`history`] | the local duel history list and the optional names the receiver types |
//!
//! Lane S0e left the card inert: it draws nothing, and the other files are empty.

pub mod answer;
pub mod card;
pub mod history;
pub mod paste;
pub mod share;
pub mod state;
pub mod verify;

pub use card::DuelCardView;
