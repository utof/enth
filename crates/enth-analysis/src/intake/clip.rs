//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3); empty until FE writes it.
//!
//! Clipped or limited spans of an upload (docs/r3.md, "The intake"): clipping after +12 dB took today's search to 64 %
//! against the clean target, with a cutoff of 51 Hz and a drive of 6.2 dB. The spans are reported as a `Degradation`,
//! and lane OBJ's robust distance leaves the clipped frames out. A clipped synth render is still a valid sound (the
//! training filters keep them); this file only says where.
