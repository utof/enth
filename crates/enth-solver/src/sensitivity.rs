//! **Owner: lane SN** (sensitivity and coach). Pre-declared empty by lane S0r (R1/S1).
//!
//! Per-parameter sensitivity of a challenge's sound score (report 02): how far each parameter can move before the
//! score notices, measured by renders through the caller's [`Evaluator`](crate::Evaluator). Structure % uses it for
//! its tolerances (lane ST) and the results drawer for its Good/Near/Bad rings. Identical results for 1 and 8
//! workers (SN's acceptance).
