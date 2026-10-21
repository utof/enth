//! **Owner: lane OBJ** (the robust objective, wave 2, after FE). Pre-declared by lane S0s (R3); empty until OBJ writes it.
//!
//! The degradation-aware distance (docs/r3.md, "The robust objective"): compare a candidate with the upload only where
//! the intake ([`intake`](crate::intake)) says the upload still carries information (the trimmed span, the band below
//! the detected cut-off, the levels above the noise floor, the unclipped frames), so the search stops paying knobs to
//! reproduce noise, a codec's low-pass or clipping. **It gets a new metric id, which OBJ names** (metric ids here are
//! such as `logmel-pool85-l1/1`; the sound score's is `sound/1`): [`distance`](crate::distance) (v1),
//! [`pooled`](crate::pooled) (v1.1) and `sound/1` stay untouched, and every score already pinned stays valid.
//!
//! OBJ's acceptance: on 40 dev targets x 4 degradations, the median `sound/1` against the **clean** target improves on
//! v1.1, and clean inputs stay within 1 point. Pure, `libm` only, a fixed summation order, like every metric here.
