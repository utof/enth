//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3); empty until FE writes it.
//!
//! The noise floor of an upload (docs/r3.md, "The intake" and the measured table of report 04 §2): noise at 20 dB SNR
//! took today's search from 99 % to 10 % against the clean target, and trimming the noisy tail alone raised it to 56 %.
//! The floor and the hum (50 or 60 Hz and harmonics) are reported as `Degradation`s, and lane OBJ's robust distance
//! compares only the levels above the floor. Pure and deterministic, on the scalar FFT path.
