//! **Owner: lane EVK.** Pre-declared by lane S0s (R3); empty until EVK writes it.
//!
//! The statistics of clause 1 (docs/r3.md, "GR3 and its hygiene"), pure Rust, **checked against hand values**:
//! `McNemar`'s test on the discordant targets of the pass rate at 60 s; the Wilcoxon signed-rank test and the
//! Hodges-Lehmann interval on the per-target difference in JND units; Kaplan-Meier for time to pass, censored at 300 s.
//! A sign test needs 69 targets to detect a 65 % win rate (37 for 70 %, 158 for 60 %), which is why the set is about 120.
//! "Beats" means better at 60 s (one-sided p < 0.05) and not worse at 300 s. Any randomness (bootstrap) is seeded.
