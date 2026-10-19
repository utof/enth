//! Measured, multi-parameter hints for `enth check` and `enth step` (design.md §7, report 03 §5).
//!
//! Given a patch, a performance and an expectation list with failures, [`hint`] returns **one** move of several
//! parameters that makes failing items pass while keeping every passing item passing, verified by real renders,
//! or an honest [`Blocked`] verdict naming the item and the parameter at its limit.
//!
//! # Algorithm
//!
//! 1. **Base.** Render and describe the patch with the goal's performance; judge every item.
//! 2. **Free parameters** (see `space`): continuous, modulatable or automatable, not structural, not periodic, of
//!    an enabled module, and not inert for this patch and performance. Enum, integer and toggle parameters are
//!    never moved in P1. They live on their normalised axis `u` in 0–1 (the search domain the scale defines);
//!    moves are reported in plain units.
//! 3. **Probe.** One render per free parameter at `u ± 0.1` (toward the middle), in one batch: the
//!    finite-difference Jacobian of every item's value in its search domain (ln for Hz and ms features, the value
//!    for dB, cents and ratios). When the budget cannot cover every parameter, those whose descriptor is in a
//!    failing item's family go first. Envelope and beating items need the goal's full performance; only when every
//!    item is spectral (pitch, brightness, noisiness) does the probe use a short clip.
//! 4. **Absent descriptors** (no beating yet, no pitch) have no slope. Probe further along the parameters that
//!    move that descriptor (`ParamSpec::perceptual`), `u ± 0.3` then `± 0.6`, jump to the point where it appears
//!    closest to its band, and measure its row there.
//! 5. **Step.** Minimise `Σ wᵢ·dist(gᵢ + Jᵢ·Δu, bandᵢ)² + λ‖Δu‖²` with `0 ≤ u + Δu ≤ 1`: each band is the
//!    accepted interval shrunk by the item's aim margin, `wᵢ` is one over that margin squared, and passing items are
//!    in the sum, so a move that pushes one out costs. Bounded Gauss-Newton with an active set.
//! 6. **Verify.** Render the step. Accept it only if no item that passes stops passing and it fails fewer items
//!    (or as many, with a smaller shortfall); then halve λ. Otherwise multiply λ by 4. Either way a Broyden rank-1
//!    update corrects the Jacobian with what the render measured.
//! 7. **Finalise.** Drop moves under 3 % of their range, round the rest to the 4 significant digits the CLI
//!    prints, and render once more: the moves an agent reads are exactly the moves that were measured.
//! 8. **Blocked.** For each item still failing, the local model at the final point says why: a parameter that
//!    would help sits at its bound, the item conflicts with others, nothing free moves it, its descriptor never
//!    appeared, or the performance is the limit.
//!
//! Determinism: the solver is serial and pure; parallelism lives in the caller's [`Evaluator`], which returns
//! results by job index. Every loop runs in a fixed order and the maths is `f64` with `libm`, so the result is
//! identical for any worker count and on every platform.

mod model;
mod run;
mod space;

use core::fmt;

use enth_analysis::descriptors::Descriptors;
use enth_analysis::expect::{Expect, ItemResult};
use enth_params::ParamSpec;
use enth_patch::{Op, ParamAddr, Patch};
use enth_render::{Performance, RenderSpec};

use crate::eval::{EvalError, Evaluator};

/// Settings for one hint. Build with `..Default::default()`; fields may be added, with defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct HintCfg {
    /// Most evaluations (render + describe) the hint may use, the base render included: the CLI's
    /// `--fix-budget`. At least 1.
    pub max_renders: usize,
    /// Most model steps verified by a render.
    pub max_steps: usize,
    /// The probe step on the normalised axis, 0–0.5.
    pub probe_step: f64,
    /// Moves smaller than this on the normalised axis are dropped if the result still holds.
    pub sparsify_below: f64,
    /// Clip length in seconds for the probe when every item is spectral; `None` always probes with the full
    /// performance. Verification always uses the full performance.
    pub short_clip_s: Option<f64>,
    /// Most renders spent looking for a parameter that makes an absent descriptor appear (and measuring it there).
    pub absent_probe: usize,
    /// Round moved values to this many significant digits before the final render (the CLI prints 4). `None`
    /// keeps exact values.
    pub round_digits: Option<u8>,
    /// When set, only these parameters may move (they must still be free, see the module docs).
    pub only: Option<Vec<ParamAddr>>,
    /// Parameters that never move.
    pub frozen: Vec<ParamAddr>,
    /// Allow moving transposition parameters (in semitones: `osc.semi`). Off by default: `semi` duplicates `fine`
    /// as a pitch offset, intervals between oscillators are design, and the step would prefer fractions of a
    /// semitone to a detune in cents.
    pub transpose: bool,
    /// Record a step-by-step trace in [`HintResult::trace`] (diagnostics; the text is not a stable format).
    pub trace: bool,
}

impl Default for HintCfg {
    fn default() -> Self {
        Self {
            max_renders: 16,
            max_steps: 6,
            probe_step: 0.1,
            sparsify_below: 0.03,
            short_clip_s: Some(0.5),
            absent_probe: 8,
            round_digits: Some(4),
            only: None,
            frozen: Vec::new(),
            transpose: false,
            trace: false,
        }
    }
}

/// One parameter move of a fix, in plain units.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    /// The parameter.
    pub addr: ParamAddr,
    /// Its spec (unit, range, display).
    pub spec: &'static ParamSpec,
    /// The value in the patch.
    pub from: f32,
    /// The value the fix sets.
    pub to: f32,
}

impl Move {
    /// The `SetParam` op for this move.
    #[must_use]
    pub fn op(&self) -> Op {
        Op::SetParam {
            addr: self.addr.clone(),
            value: self.to,
        }
    }

    /// The new value as text that parses back to exactly `to`: the CLI's four significant digits (`1.2kHz`,
    /// `40ms`) when the fix was rounded to them, else the exact display.
    #[must_use]
    pub fn to_text(&self) -> String {
        let rounded = self.spec.display_rounded(self.to, 4).to_string();
        if self
            .spec
            .parse(&rounded)
            .is_ok_and(|v| v.to_bits() == self.to.to_bits())
        {
            rounded
        } else {
            self.spec.display(self.to).to_string()
        }
    }

    /// The old value, printed the same way.
    #[must_use]
    pub fn from_text(&self) -> String {
        self.spec.display_rounded(self.from, 4).to_string()
    }

    /// How far it moves, as a fraction of the parameter's normalised range (−1–1).
    #[must_use]
    pub fn normalised(&self) -> f64 {
        f64::from(self.spec.to_norm(self.to)) - f64::from(self.spec.to_norm(self.from))
    }
}

/// A verified fix: apply all `moves` together.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The moves, in patch order.
    pub moves: Vec<Move>,
    /// The patch with every move applied.
    pub patch: Patch,
    /// Every item judged on the fixed patch's render with the goal's performance (no nuisance renders, so
    /// `noise` is `None`): real values, not predictions.
    pub after: Vec<ItemResult>,
    /// The descriptors of that render.
    pub descriptors: Descriptors,
    /// Every item passes after the fix.
    pub pass_all: bool,
    /// The fix moves a single parameter and every item passes.
    pub single_param_enough: bool,
    /// The values are rounded to `HintCfg::round_digits` (false when only the exact values held).
    pub rounded: bool,
}

impl Fix {
    /// The `SetParam` ops, in move order.
    #[must_use]
    pub fn ops(&self) -> Vec<Op> {
        self.moves.iter().map(Move::op).collect()
    }

    /// Indices of the items that still fail.
    #[must_use]
    pub fn still_failing(&self) -> Vec<usize> {
        (0..self.after.len())
            .filter(|&i| !self.after[i].pass)
            .collect()
    }
}

/// Why an item cannot be fixed by moving the free parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum BlockReason {
    /// The parameters that would move it the right way sit at their bounds (listed in `at_bound`).
    AtBound,
    /// The performance is the limit: a held note sounds for at least its hold; no note-off inside the clip
    /// leaves no release to measure. `at_bound` lists parameters already at their limit.
    Performance,
    /// It could pass on its own, but only by pushing the items in `conflicts_with` out.
    Conflict,
    /// No free parameter moves it: the structure is the limit (a module it needs is missing or a stub).
    NoEffect,
    /// Its descriptor is absent, and no free parameter made it appear.
    Absent,
    /// `== null` items are judged, not aimed at: the solver has no continuous measure for making a descriptor
    /// disappear (`!= null` is handled as an absent descriptor).
    Unmodelled,
}

impl BlockReason {
    /// A stable snake-case code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            BlockReason::AtBound => "at_bound",
            BlockReason::Performance => "performance",
            BlockReason::Conflict => "conflict",
            BlockReason::NoEffect => "no_effect",
            BlockReason::Absent => "absent",
            BlockReason::Unmodelled => "unmodelled",
        }
    }
}

/// An item no move of the free parameters satisfies (together with the others).
#[derive(Clone, Debug, PartialEq)]
pub struct Blocked {
    /// Index into the expectation list.
    pub item: usize,
    /// The item in canonical form.
    pub expect: String,
    /// Why.
    pub reason: BlockReason,
    /// Parameters that would move it the right way but sit at their bounds.
    pub at_bound: Vec<ParamAddr>,
    /// Items (indices) that the move it needs would push out of their bands.
    pub conflicts_with: Vec<usize>,
    /// One plain sentence for agents.
    pub why: String,
}

/// Counters of one hint. They depend only on the inputs, never on time or worker count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent counters and flags"
)]
pub struct HintStats {
    /// Every evaluation (render + describe), all phases.
    pub renders: usize,
    /// Calls to the evaluator.
    pub batches: usize,
    /// The base render(s): 1, or 2 when the probe uses a short clip.
    pub base_renders: usize,
    /// Finite-difference probes.
    pub probe_renders: usize,
    /// Renders looking for an absent descriptor and measuring it where it appeared.
    pub absent_renders: usize,
    /// Model steps verified.
    pub verify_renders: usize,
    /// Renders of the sparsified, rounded fix.
    pub final_renders: usize,
    /// Renders on the short clip (counted in the phases above too).
    pub short_renders: usize,
    /// Evaluations that returned an error (their candidate was skipped).
    pub failed: usize,
    /// Free parameters.
    pub free_params: usize,
    /// Free parameters the budget allowed to probe (only these can move).
    pub probed_params: usize,
    /// Verified steps that were accepted.
    pub accepted_steps: usize,
    /// The hint stopped because the render budget ran out.
    pub budget_exhausted: bool,
}

/// The outcome of [`hint`].
#[derive(Clone, Debug, PartialEq)]
pub struct HintResult {
    /// Every item judged on the base render (no nuisance renders).
    pub before: Vec<ItemResult>,
    /// The best verified move, if any improves the list without breaking a passing item.
    pub fix: Option<Fix>,
    /// Items that cannot pass by moving the free parameters, with the reason.
    pub blocked: Vec<Blocked>,
    /// The parameters that were probed (the only ones a fix can move), in patch order.
    pub probed: Vec<ParamAddr>,
    /// Counters.
    pub stats: HintStats,
    /// The step-by-step trace when `HintCfg::trace` is set (diagnostics; not a stable format).
    pub trace: Vec<String>,
}

/// Why [`hint`] could not run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HintError {
    /// The expectation list is empty.
    NoItems,
    /// `max_renders` is 0: not even the base can be rendered.
    NoBudget,
    /// The performance plays no note, so there is nothing to measure.
    NoNotes,
    /// The base patch could not be rendered or described.
    Base(EvalError),
    /// The evaluator returned the wrong number of results.
    Contract {
        /// Jobs sent.
        jobs: usize,
        /// Results returned.
        results: usize,
    },
}

impl HintError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            HintError::NoItems => "no_items",
            HintError::NoBudget => "no_budget",
            HintError::NoNotes => "no_notes",
            HintError::Base(e) => e.code(),
            HintError::Contract { .. } => "evaluator_contract",
        }
    }
}

impl fmt::Display for HintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HintError::NoItems => f.write_str("the expectation list is empty"),
            HintError::NoBudget => {
                f.write_str("the render budget is 0; it must cover the base render")
            }
            HintError::NoNotes => {
                f.write_str("the performance plays no note; give it at least one")
            }
            HintError::Base(e) => write!(f, "the patch could not be measured: {e}"),
            HintError::Contract { jobs, results } => write!(
                f,
                "the evaluator returned {results} results for {jobs} jobs (one per job, in order)"
            ),
        }
    }
}

impl std::error::Error for HintError {}

/// Find one measured move for the whole list (see the [module docs](self)).
///
/// `perf` is the goal's performance: the base, every verification and the final render use it. `expects` are
/// parsed items (`enth_analysis::parse_list`). Renders go through `eval` in batches; the result does not depend
/// on how the evaluator spreads them over workers.
///
/// # Errors
/// An empty list, a zero budget, a performance without notes, a base patch that cannot be measured, or an
/// evaluator that breaks its contract.
pub fn hint(
    patch: &Patch,
    perf: &Performance,
    spec: &RenderSpec,
    expects: &[Expect],
    eval: &mut dyn Evaluator,
    cfg: &HintCfg,
) -> Result<HintResult, HintError> {
    if expects.is_empty() {
        return Err(HintError::NoItems);
    }
    if cfg.max_renders == 0 {
        return Err(HintError::NoBudget);
    }
    if perf.notes.is_empty() {
        return Err(HintError::NoNotes);
    }
    run::Run::new(patch, perf, spec, expects, eval, cfg).go()
}
