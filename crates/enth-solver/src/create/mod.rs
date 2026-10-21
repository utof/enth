//! **Owner: lane TP** (the portfolio, freeze point F1s); **MS** edits only `schedule.rs`. Pre-declared by lane S0s (R3,
//! freeze point F0s).
//!
//! "Create from sound", R3a's search side (docs/r3.md, "The start portfolio and k-NN retrieval" and "K branches and
//! successive halving"): every create job starts from several places at once, refines each as its own [`Search`]
//! (the ask/tell CMA-ES that takes one start and has `cancel()`), and answers K `Candidate`s (the type is
//! [`candidate`]'s). Like the rest of the solver it spawns no threads, reads no clocks and does no I/O: parallelism
//! comes from the caller's evaluator, which returns results by job index, and **the wall-clock cap lives in the
//! shell's `search_with` callback**, so the outcome is the same for any worker count and the schedule never depends on
//! wall time (TP's acceptance).
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`candidate`] | `Candidate` (patch, distance, provenance, trace), the `create` entry point's types and what `enth.create/1` is built from; **reaches F1s with FE's intake types** | TP |
//! | [`portfolio`] | the start set: the heuristic start (always kept), the templates, k-NN retrieval over the bank (R3a runs without it), later the model's hypotheses | TP |
//! | [`schedule`] | K branches, each round's `ask()` batches merged into one pool batch and split back to each `tell()`; deterministic successive halving (about 10 %, 30 %, the rest); MS adds the model's proposals here | TP; MS |
//!
//! The k-NN retrieval itself fills the existing [`nearest`](crate::nearest) stub, which keeps that stub's meaning ("the
//! nearest preset as a start"); R2's [`closest`](crate::closest) is another feature. `hint/` and `search.rs` stay
//! untouched: a needed `pub` is additive and reported.
//!
//! [`Search`]: crate::search::Search

pub mod candidate;
pub mod portfolio;
pub mod schedule;
