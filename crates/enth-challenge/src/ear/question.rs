//! **Owner: lane CV3** (Challenge v3, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until
//! CV3 writes it. The reference is docs/ear.md, "Rendering and playback".
//!
//! A question from `enth-music`'s ear generator becomes a sound: a timbre patch from the frozen, append-only pool
//! `timbres-v1/` (sine, saw low-pass, pluck, organ table, pad, a noise-burst click; pitch drills rotate them, so the
//! timbre is never the cue), a `Performance` (polyphony of at least 8, 12-TET pitch through the pinned `libm`) and,
//! for the EQ games, an optional post-EQ band through `enth-dsp`'s biquad. **No engine change:** every stimulus is an
//! offline render played through the Target slot, about 15 to 30 ms each, and stimuli are level-matched within 0.5 LU
//! unless level is the cue.
