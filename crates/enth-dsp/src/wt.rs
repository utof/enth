//! The DSP core of S2a lane WT: the `wt` oscillator's generated tables (FM-index sweeps, formant-swept saws, sparse
//! metal spectra), phase warps and phase-modulated reads, and later its unison on `wide` lanes (lane UNI). Pre-declared
//! empty by lane S0d (freeze F0m, docs/phase-s2.md) so that no module lane edits `lib.rs`; lane WT fills it, with the
//! sources and licences in its `docs/dsp` page and `THIRD_PARTY_NOTICES.md` (AGENTS.md: write DSP from cited papers and
//! permissive sources, never from memory of a GPL synth's code).
