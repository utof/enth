//! The hint seam: where `check` and `step` ask for one measured multi-parameter `fix` for all failing items, and
//! for the items that are `blocked` (docs/agent-cli.md, "One hint solves all failing items at once").
//!
//! The search itself belongs in `enth-solver::hint` (L5, lane H), not here: shells translate, they never decide.
//! Until lane H merges, [`propose`] returns [`FixOutcome::default`], so `fix` prints as `null` and `blocked` as
//! `[]`. Lane H plugs in by replacing the body of [`propose`]: call the solver with `req.patch`, `req.items`,
//! `req.report`, `req.budget` and an evaluator that wraps [`Evaluator::describe_patches`] (render + describe at the
//! judged seed, rayon-parallel, results in input order), then fill a [`FixOutcome`]. Everything else (when to
//! call, `--no-fix`, `--fix-budget`, turning ops into wire JSON, printing) is already wired.
//!
//! `check` calls [`propose`] only for a patch (a WAV has nothing to move), only when at least one item failed, and
//! not with `--no-fix`. The evaluator renders with the goal's performance (`--perf`), because envelope and beating
//! items cannot be judged on a short probe clip.

use enth_analysis::{CheckReport, Descriptors, Expect, Feature};
use enth_patch::{Op, ParamAddr, Patch};

use crate::eval::Evaluator;

/// The default `--fix-budget`: renders the search may use.
pub const DEFAULT_BUDGET: u32 = 16;

/// Everything the hint solver gets.
pub struct FixRequest<'a> {
    /// The patch being judged.
    pub patch: &'a Patch,
    /// The parsed expectation list, in order.
    pub items: &'a [Expect],
    /// The verdict on the judged render (with noise from the nuisance re-renders).
    pub report: &'a CheckReport,
    /// The descriptors of the judged render.
    pub base: &'a Descriptors,
    /// Renders and describes candidate patches with the goal's performance at the judged seed.
    pub evaluator: &'a Evaluator,
    /// The most renders the search may use (`--fix-budget`, default [`DEFAULT_BUDGET`]).
    pub budget: u32,
}

/// One measured move for the whole list.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The ops to apply together, in order (resolved, plain values).
    pub ops: Vec<Op>,
    /// The readings measured after applying them.
    pub after: Vec<(Feature, f64)>,
    /// Every item passes after the move.
    pub pass_all: bool,
    /// One parameter would have done.
    pub single_param_enough: bool,
    /// `after` holds real values from a render.
    pub measured: bool,
    /// Renders the search used.
    pub renders: u32,
    /// What to know about it ("apply these ops together; ...").
    pub note: String,
}

/// An item that no move of the patch's parameters can satisfy.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked {
    /// Index of the item in [`FixRequest::items`].
    pub item: usize,
    /// Why: the structure or the performance is the limit.
    pub why: String,
    /// Parameters sitting at a bound.
    pub at_bound: Vec<ParamAddr>,
}

/// A suggestion for one item (structural ones are not measured).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemHint {
    /// The suggestion.
    pub text: String,
    /// It was rendered and measured.
    pub measured: bool,
}

/// What the solver found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FixOutcome {
    /// The move, if one was found.
    pub fix: Option<Fix>,
    /// Items no move can satisfy.
    pub blocked: Vec<Blocked>,
    /// Per item (same length as the items, or empty): a suggestion.
    pub item_hints: Vec<Option<ItemHint>>,
}

/// Ask for a fix. **The seam lane H fills** (see the module docs); until then no fix and nothing blocked.
pub fn propose(req: &FixRequest<'_>) -> FixOutcome {
    let _ = req;
    FixOutcome::default()
}
