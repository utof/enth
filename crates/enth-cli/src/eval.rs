//! The evaluator: render a patch with a performance and describe the result, many at once on rayon, results in
//! input order (so they are identical for any worker count). `check` uses it for the judged render and its nuisance
//! re-renders; the hint solver (lane H, through [`crate::fix`]) uses it for its probes. Core crates spawn no
//! threads: this is where the parallelism comes from (AGENTS.md).

use enth_analysis::descriptors::Want;
use enth_analysis::{Describer, Descriptors, NoteTiming};
use enth_patch::Patch;
use enth_render::{DEFAULT_SAMPLE_RATE, Performance, RenderOutput, RenderSpec, render};
use rayon::prelude::*;

use crate::error::{self, CliError};
use crate::perf::Perf;

/// Offsets that derive the two nuisance render seeds from the judged one. Oscillators in `free` phase mode (the
/// default) take their start phases from the render seed, so a re-render with another seed is the same sound with
/// other phases: how far a reading moves across them is its measurement noise.
const NUISANCE_OFFSETS: [u64; 2] = [0x9E37_79B9_7F4A_7C15, 0xC2B2_AE3D_27D4_EB4F];

/// Renders and describes patches with one performance.
#[derive(Clone, Debug)]
pub struct Evaluator {
    perf: Performance,
    timing: NoteTiming,
    seed: u64,
    want: Want,
}

impl Evaluator {
    /// An evaluator for `perf` at the judged render `seed`, computing the descriptor groups in `want` (pitch, the
    /// envelope and beating are always measured).
    pub fn new(perf: &Perf, seed: u64, want: Want) -> Self {
        Evaluator {
            perf: perf.to_render(),
            timing: perf.timing(),
            seed,
            want,
        }
    }

    /// The judged render seed.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The seeds of the two nuisance re-renders.
    pub fn nuisance_seeds(&self) -> [u64; 2] {
        NUISANCE_OFFSETS.map(|o| self.seed.wrapping_add(o))
    }

    /// The performance, as rendered.
    pub fn performance(&self) -> &Performance {
        &self.perf
    }

    /// The note timing the envelope is read against.
    pub fn timing(&self) -> NoteTiming {
        self.timing
    }

    /// The sample rate of every render (48 kHz).
    pub fn sample_rate(&self) -> u32 {
        DEFAULT_SAMPLE_RATE
    }

    /// Render one patch at a seed.
    pub fn render(&self, patch: &Patch, seed: u64) -> Result<RenderOutput, CliError> {
        render(
            patch,
            &self.perf,
            &RenderSpec::new(DEFAULT_SAMPLE_RATE, seed),
        )
        .map_err(|e| error::from_render(&e))
    }

    fn describe_with(
        &self,
        d: &mut Describer,
        patch: &Patch,
        seed: u64,
        want: Want,
    ) -> Result<Descriptors, CliError> {
        let out = self.render(patch, seed)?;
        d.describe_some(&out.mid(), self.timing, want)
            .map_err(|e| error::from_analysis(&e))
    }

    /// Render and describe each `(patch, seed)` job in parallel; results in input order.
    pub fn describe_jobs(&self, jobs: &[(&Patch, u64)]) -> Vec<Result<Descriptors, CliError>> {
        let want = self.want;
        jobs.par_iter()
            .map_init(
                || Describer::new(f64::from(DEFAULT_SAMPLE_RATE)),
                |d, (patch, seed)| self.describe_with(d, patch, *seed, want),
            )
            .collect()
    }

    /// Render and describe each patch at the judged seed, in parallel; results in input order. The probe batch of
    /// the hint solver.
    pub fn describe_patches(&self, patches: &[Patch]) -> Vec<Result<Descriptors, CliError>> {
        let jobs: Vec<(&Patch, u64)> = patches.iter().map(|p| (p, self.seed)).collect();
        self.describe_jobs(&jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_input_order_and_repeat() {
        let perf = Perf::default_perf();
        let ev = Evaluator::new(&perf, enth_render::DEFAULT_SEED, Want::CORE);
        let a = Patch::init();
        let mut b = Patch::init();
        b.voice.values[0] = 0.0; // poly, unchanged: the same patch twice under another name
        let jobs = [(&a, 1), (&b, 2), (&a, 1)];
        let r = ev.describe_jobs(&jobs);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], r[2]);
        let again = ev.describe_jobs(&jobs);
        assert_eq!(r, again);
    }
}
