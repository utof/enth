//! The DSP core of S2a lane OTT: the three-band upward and downward compressor (a Linkwitz-Riley split and the
//! compressor's detector) behind the `ott` module. Pre-declared empty by lane S0d (freeze F0m, docs/phase-s2.md) so
//! that no module lane edits `lib.rs`; lane OTT fills it, with the sources and licences in its `docs/dsp` page and
//! `THIRD_PARTY_NOTICES.md` (AGENTS.md: write DSP from cited papers and permissive sources, never from memory of a GPL
//! synth's code).
