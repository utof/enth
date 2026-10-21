//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3); empty until FE writes it.
//!
//! Mono and trim (docs/r3.md, "The intake"): mono is the mean of the channels; leading silence (about 60 dB below the
//! peak) is stripped with a few ms of pre-roll; the length is capped at 4 to 6 s, because evaluation cost grows with
//! it. The reported start must land within 5 ms of the true onset (FE's acceptance). **A short sound is kept, never
//! rejected**: nothing here filters by duration.
