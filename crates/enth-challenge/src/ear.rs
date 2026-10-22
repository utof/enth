//! **Owner: lane CV3** (Challenge v3, freeze point F1e); **G4** owns [`ladder`]. Pre-declared by lane S0e (R4/R5, freeze
//! point F0e); empty until they write it. The reference is docs/ear.md ("Where R4 lives", "Rendering and playback") and
//! docs/challenge.md, "Challenge v3 (planned, lane CV3, R4)".
//!
//! What an ear question needs in `enth-challenge` (L4), which `enth-music` (L1) cannot hold: a question to sound, the
//! parameter games' items on v2 draws, the scorers and the ear-challenge code. One file each, so CV3 and G4 never edit
//! each other's:
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`question`] | a question to a timbre patch from the frozen pool `timbres-v1/`, a `Performance` (notes within C1 to C7, at most 8 at once, at most 10 s) and the optional post-EQ band rendered through `enth-dsp`'s biquad (lane BQ) | CV3 |
//! | [`params`] | the parameter games' items (Spot the change, Dial it in, Ear check) as v2 draws: the options, the control that moves and its distance in parameter steps | CV3 |
//! | [`ladder`] | those games' difficulty ladder in parameter steps, and the *not measured* list for the pairs `sound/1` cannot hear | G4 |
//! | [`scorers`] | the scorers `ear/1`, `ear-value/1` and `taps/1`: answer keys are seeded integers, so an ear result is judged with no render, identically on every platform and engine version | CV3 |
//! | [`code`] | the ear-challenge code (kind 3 behind version 15, an `EarSpec` on a fixed ladder for ear duels) | CV3 |
//!
//! The timbre pool `timbres-v1/` is frozen and append-only data that lane CV3 adds beside this file.

pub mod code;
pub mod ladder;
pub mod params;
pub mod question;
pub mod scorers;
