//! The DSP core of S2a lane DST: the zero-latency 2x and 4x IIR oversampler (a two-path polyphase all-pass half-band,
//! Valenzuela and Constantinides 1983, coefficients designed from the paper and committed with a test) that `dist` uses
//! first and other aliasing modules may share. Pre-declared empty by lane S0d (freeze F0m, docs/phase-s2.md) so that no
//! module lane edits `lib.rs`; lane DST fills it, with the sources and licences in its `docs/dsp` page and
//! `THIRD_PARTY_NOTICES.md` (AGENTS.md: write DSP from cited papers and permissive sources, never from memory of a GPL
//! synth's code).
