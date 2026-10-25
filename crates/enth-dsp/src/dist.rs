//! The DSP core of S2a lane DST: the distortion shapers (soft and hard clip, sine and triangle folds, asymmetric,
//! bitcrush, downsample) with first-order ADAA, used by the `dist` module. Pre-declared empty by lane S0d (freeze F0m,
//! docs/phase-s2.md) so that no module lane edits `lib.rs`; lane DST fills it, with the sources and licences in its
//! `docs/dsp` page and `THIRD_PARTY_NOTICES.md` (AGENTS.md: write DSP from cited papers and permissive sources, never
//! from memory of a GPL synth's code).
