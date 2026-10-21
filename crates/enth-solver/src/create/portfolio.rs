//! **Owner: lane TP** (freeze point F1s). Pre-declared by lane S0s (R3).
//!
//! The start portfolio (docs/r3.md, "The start portfolio and k-NN retrieval"), in this order:
//!
//! 1. **the heuristic start**, always kept: the init template with the target's descriptors applied before
//!    screening ([`heuristic`](crate::heuristic): pitch → tuning, envelope → ADSR, centroid → cutoff, on the
//!    parameters the job searches), so its proposal and its screening distance are the heuristic's; the bare init
//!    template when the clip is too short or quiet to describe;
//! 2. **the templates** the caller passes (`content/templates/`: `pluck_1osc`, `pad_2osc`, `bass_sub`), because
//!    structure is never searched otherwise (GR1's level 5 solved 0 of 4); one equal to the init template is skipped;
//! 3. **k-NN retrieval** over the bank ([`nearest`](crate::nearest)), nearest first, spread apart
//!    ([`CreateCfg::min_spread`]); R3a runs without the bank;
//! 4. **the caller's proposals** ([`Inputs::proposals`]): R3b's model hypotheses (lane MS, [`Provenance::Model`]),
//!    in the model's order.
//!
//! The order is fixed (the screening round's ties go to the earlier start). An exact duplicate of an earlier start
//! is skipped. Every start keeps its provenance into the result and the record. Sizing K and the budget comes from
//! TP's measured per-evaluation cost (docs/r3.md, the cost table).

use enth_patch::{Op, Patch, apply_all};

use super::candidate::Provenance;
use super::{Analysis, CreateCfg, Inputs, Target, free_set};
use crate::heuristic::Heuristic;

/// A named template patch (the caller loads `content/templates/`; the solver does no I/O).
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// Its name (`pluck_1osc`).
    pub name: String,
    /// The patch.
    pub patch: Patch,
}

/// One start: a patch and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    /// The start patch.
    pub patch: Patch,
    /// Where it came from.
    pub provenance: Provenance,
}

/// The starts of a job, in proposal order, and what building them noticed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Portfolio {
    /// The starts; the heuristic start first when there is one.
    pub starts: Vec<Start>,
    /// What the user should know (a bank entry that no longer regenerates).
    pub notes: Vec<String>,
}

impl Portfolio {
    /// The portfolio for `target` (measured as `analysis`): the heuristic start from `inputs.init`, the templates,
    /// up to `cfg.neighbours` k-NN neighbours from `inputs.bank`, then `inputs.proposals` (see the
    /// [module docs](self)).
    #[must_use]
    pub fn build(
        target: &Target,
        analysis: &Analysis,
        inputs: &Inputs<'_>,
        cfg: &CreateCfg,
    ) -> Portfolio {
        let mut p = Portfolio::default();
        p.push(Start {
            patch: heuristic_start(target, analysis, inputs.init),
            provenance: Provenance::Heuristic,
        });
        for t in inputs.templates {
            if t.patch == *inputs.init {
                continue;
            }
            p.push(Start {
                patch: t.patch.clone(),
                provenance: Provenance::Template {
                    name: t.name.clone(),
                },
            });
        }
        if let Some(bank) = inputs.bank {
            let neighbours = bank.nearest(&analysis.embedding, cfg.neighbours, cfg.min_spread);
            for (rank, n) in neighbours.iter().enumerate() {
                match bank.patch(n.index) {
                    Ok(patch) => {
                        let e = &bank.entries()[n.index];
                        p.push(Start {
                            patch,
                            provenance: Provenance::Knn {
                                rank,
                                bank_index: n.index,
                                seed: e.seed,
                                difficulty: e.difficulty,
                                musical: e.musical,
                                embedding_distance: n.distance,
                            },
                        });
                    }
                    Err(e) => p
                        .notes
                        .push(format!("k-NN neighbour {} skipped: {e}", n.index)),
                }
            }
        }
        for proposal in inputs.proposals {
            p.push(proposal.clone());
        }
        if analysis.descriptors.is_none() {
            p.notes.push(
                "the clip is too short or too quiet to describe: the starts are searched without the heuristic"
                    .to_owned(),
            );
        }
        p
    }

    /// Add a start (R3b's model hypotheses go in here), unless an earlier start has the same patch. Returns whether
    /// it was added.
    pub fn push(&mut self, start: Start) -> bool {
        if self.starts.iter().any(|s| s.patch == start.patch) {
            return false;
        }
        self.starts.push(start);
        true
    }
}

/// The heuristic start: `init` with the heuristic's suggestions for the target applied to the parameters a branch
/// searches (exact plain values, through ops). `init` itself when the clip has no descriptors, nothing is free, or
/// an op is refused.
fn heuristic_start(target: &Target, analysis: &Analysis, init: &Patch) -> Patch {
    let Some(desc) = analysis.descriptors.as_ref() else {
        return init.clone();
    };
    let Ok(free) = free_set(init, &target.perf) else {
        return init.clone();
    };
    let heuristic = Heuristic::new(desc, &target.performance());
    let ops: Vec<Op> = heuristic
        .suggest(init, &free)
        .into_iter()
        .map(|s| Op::SetParam {
            addr: s.addr,
            value: s.value,
        })
        .collect();
    let mut patch = init.clone();
    match apply_all(&mut patch, &ops) {
        Ok(_) => patch,
        Err(_) => init.clone(),
    }
}
