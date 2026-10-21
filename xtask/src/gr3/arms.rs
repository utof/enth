//! **Owner: lane EVK.** Pre-declared by lane S0s (R3); empty until EVK writes it.
//!
//! The four arms (docs/r3.md, "GR3 and its hygiene"). They share the intake, the start set, the free set, the workers and
//! the seeds: **A**, R3a's best configuration tuned on dev (the baseline must be the best search, not GR1's 1,500-render
//! floor); **B**, model + search from the same budget; **C**, the model's proposals unrefined (a diagnostic); **D**, A
//! given the true template (it separates structure error from parameter error). B's clock includes intake, features and
//! inference. A run is `enth create` (or `enth match`) through the `enth` binary, with the target rendered from the
//! set's manifest; **tuning happens on dev only**, and a test run is never repeated to pick a winner.
