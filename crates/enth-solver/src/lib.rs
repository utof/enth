//! enth-solver (L5, see docs/architecture.md): measured hints over render + analysis (P1a lane H).
//!
//! - [`hint`](mod@hint): one multi-parameter move that makes failing expectations pass while keeping passing ones
//!   passing, verified by real renders, or a [`Blocked`] verdict naming the item and the parameter at its limit.
//!   The `fix` and `blocked` of `enth check` and `enth step` (`docs/dsp/hints.md`).
//! - [`eval`]: the [`Evaluator`] trait the caller implements (batches of candidate patches → descriptors, results
//!   in job order), the one render-and-describe function [`measure`], and the single-threaded [`Serial`].
//!
//! R1/S1 files, pre-declared empty by lane S0r so the lanes that fill them never edit this one (`hint/` is untouched):
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`sensitivity`] | per-parameter sensitivity of a challenge's sound score | SN |
//! | [`coach`] | Recreate hints: the target's descriptors as an `Expect` list for [`hint()`] | SN |
//! | [`cmaes`] | CMA-ES ported from `spike-metric`: ask-tell, seeded, bounded, IPOP restarts | MT |
//! | [`search`] | `enth match`: free parameters, a distance, a render budget; probe, CMA-ES runs and restarts (`docs/dsp/match.md`) | MT |
//! | [`heuristic`] | the search's start: envelope, pitch, cutoff and detune from a target's descriptors | MT |
//! | [`nearest`] | k-NN retrieval over a pre-rendered bank of generator patches (`knn-logmel-rel/1`), one of a create job's starts; the stub MT left became R3a's (R2's "how close am I" is [`closest`]) | TP (R3a) |
//!
//! R2 file, pre-declared empty by lane S0l (`hint/` and `search.rs` stay untouched):
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`closest`] | "How close am I?": CMA-ES from the player's patch over a lesson step's editable set (design §7); leaves `nearest` to R3 | CLS |
//!
//! R3 directories, pre-declared empty by lane S0s (`hint/` and `search.rs` stay untouched; `nearest` is filled by TP):
//!
//! | Directory | Holds | Lane |
//! | --- | --- | --- |
//! | [`create`] | "create from sound": `Target::from_intake`, `candidate`, `portfolio`, `schedule` (the start set, K branches, successive halving on budget-independent milestones), `record` (`enth.create/1`); the k-NN retrieval fills [`nearest`] | TP, F1S, MS |
//! | [`propose`] | R3b's inference on our own scalar kernels: `pack`, `ops`, `net`, `features`, `decode`; wasm32, bytes passed in | ONX |
//!
//! The solver spawns no threads, reads no clocks and does no I/O: parallelism comes from the caller's evaluator (an
//! [`Objective`](search::Objective) for a match), which returns results by job index, so a hint or a match is
//! identical for any worker count.
//!
//! ```
//! use enth_analysis::parse_list;
//! use enth_patch::Patch;
//! use enth_render::{Performance, RenderSpec};
//! use enth_solver::{HintCfg, Serial, hint};
//!
//! let patch = Patch::init();
//! let perf = Performance::note(69, 0.8, 0.3, 0.6);
//! let items = parse_list("attack > 40ms").unwrap();
//! let cfg = HintCfg { max_renders: 8, ..HintCfg::default() };
//! let out = hint(&patch, &perf, &RenderSpec::default(), &items, &mut Serial::new(), &cfg).unwrap();
//! assert!(out.stats.renders <= 8);
//! if let Some(fix) = &out.fix {
//!     // Every move is measured: `after` holds the readings of a real render.
//!     assert!(fix.after[0].got.is_some());
//! }
//! ```

pub mod closest;
pub mod cmaes;
pub mod coach;
pub mod create;
pub mod eval;
pub mod heuristic;
pub mod hint;
pub mod nearest;
pub mod propose;
pub mod search;
pub mod sensitivity;

pub use eval::{EvalError, Evaluator, Job, Serial, measure, note_timing};
pub use hint::{BlockReason, Blocked, Fix, HintCfg, HintError, HintResult, HintStats, Move, hint};
