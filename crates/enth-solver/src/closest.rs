//! **Owner: lane CLS** (closest, wave 1). Pre-declared by lane S0l (R2); empty until CLS writes it.
//!
//! The nearest passing patch from the player's own (docs/design.md §7, layer 3; docs/learn.md, "Knob feedback"):
//! CMA-ES from your patch over the step's editable set, penalising distance, answering per-parameter deltas. It is a
//! **"How close am I?" button** with progress and cancel, never live (a scoring render costs 15 to 30 ms and level-2
//! `match` runs took a median of 423 renders), and using it makes the redo a sibling challenge, as a Reveal does.
//!
//! The job payload is solver-native, `{round, patch, free, budget}`, so `enth-learn` never calls this crate (same
//! layer) and the shell's worker composes nothing. Like the rest of the solver it spawns no threads, reads no clocks
//! and does no I/O: parallelism comes from the caller's evaluator, which returns results by job index, so the answer is
//! the same for any worker count (CLS's acceptance: 1 and 8 workers). The free set is usually 1 to 4 parameters; CLS
//! measures the renders and seconds on ten real lesson starts and writes a cost table.
//!
//! This is R2's feature. [`nearest`](crate::nearest) is R3's ("the nearest preset as a start", filled by R3a's k-NN
//! retrieval); the two names stay apart.
