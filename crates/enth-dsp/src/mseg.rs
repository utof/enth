//! The DSP core of S2a lane MSEG: the multi-segment envelope (a fixed point count, a curve per segment, loop, one-shot
//! and sustain-loop) behind the `mseg` module. Pre-declared empty by lane S0d (freeze F0m, docs/phase-s2.md) so that no
//! module lane edits `lib.rs`; lane MSEG fills it, with the sources and licences in its `docs/dsp` page and
//! `THIRD_PARTY_NOTICES.md` (AGENTS.md: write DSP from cited papers and permissive sources, never from memory of a GPL
//! synth's code).
