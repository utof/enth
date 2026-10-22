//! **Owner: lane ECK.** Pre-declared by lane S0e (R4/R5); empty until ECK writes it.
//!
//! The players `ear-check` plays every game with (docs/ear.md, "GR4"): the **oracle**, an ideal observer that picks the
//! option nearest by `sound/1` (at least 95 % at the top level); the **random** player (chance); the **bias bots** (always
//! option 1, always the loudest, always the longest), which must stay at chance +-3 % over 10,000 items; and the **deaf**
//! player, which hears nothing and so shows which levels are unsolvable. Draws are seeded from `enth-dsp`'s pinned `rng`.
