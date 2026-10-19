//! enth-solver (L5, see docs/architecture.md): measured hints over render + analysis (P1a lane H).
//!
//! - [`hint`](mod@hint): one multi-parameter move that makes failing expectations pass while keeping passing ones
//!   passing, verified by real renders, or a [`Blocked`] verdict naming the item and the parameter at its limit.
//!   The `fix` and `blocked` of `enth check` and `enth step` (`docs/dsp/hints.md`).
//! - [`eval`]: the [`Evaluator`] trait the caller implements (batches of candidate patches → descriptors, results
//!   in job order), the one render-and-describe function [`measure`], and the single-threaded [`Serial`].
//!
//! The solver spawns no threads, reads no clocks and does no I/O: parallelism comes from the caller's evaluator,
//! which returns results by job index, so a hint is identical for any worker count.
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

pub mod eval;
pub mod hint;

pub use eval::{EvalError, Evaluator, Job, Serial, measure, note_timing};
pub use hint::{BlockReason, Blocked, Fix, HintCfg, HintError, HintResult, HintStats, Move, hint};
