//! The DSP core of S2a lane EQ: the parametric EQ's bands on `biquad.rs` (lane BQ's file) with per-segment coefficient
//! ramps, behind the `eq` module. Pre-declared empty by lane S0d (freeze F0m, docs/phase-s2.md) so that no module lane
//! edits `lib.rs`; lane EQ fills it, with the sources and licences in its `docs/dsp` page and `THIRD_PARTY_NOTICES.md`
//! (AGENTS.md: write DSP from cited papers and permissive sources, never from memory of a GPL synth's code).
