//! The `comp` module: the engine wrapper around `enth_dsp::compressor::Compressor` (P1a lane FXW; the core is lane
//! FX2's, `docs/dsp/compressor.md`).
//!
//! A stereo feed-forward compressor on the bus after the voices: zero latency, no look-ahead, no heap memory at
//! all (the core is a few dozen bytes of state, so `new` is the only place that sets anything up and nothing ever
//! allocates). Not `time_based`: the global FX bypass leaves it on, because it sets level, not space.
//!
//! # Parameters
//!
//! Every catalogue value goes to the core's field of the same meaning, in the same unit ([`core_params`]): `mix` is
//! the core's `mix_pct` as it is (100 % fully compressed, less is parallel compression), `threshold`/`ratio`/
//! `knee`/`makeup` are dB, dB, :1 and dB, `attack`/`release` are ms. The wrapper calls `set_params` once per
//! 32-frame segment with the segment's `cur` values; the core recomputes only what changed (about 130 instructions
//! for a changed threshold, ratio or knee, 120 for make-up and mix, 260 for an attack) and does the rest of the
//! smoothing itself where it can:
//!
//! - `threshold`, `ratio` and `knee` move the target of the core's gain smoother (time constant attack/2), so a step
//!   glides on its own; the catalogue also gives them the engine's 10 ms smoother, which makes the target move
//!   gently as well.
//! - `makeup` and `mix` scale the output directly, so a step would click: the catalogue gives both the engine's
//!   10 ms linear smoother, and the value arriving here is the ramped one.
//! - `attack` and `release` are the follower's time constants and jump their coefficients (no smoothing); they are
//!   the 63 % times of the amplitude follower, not of the gain reduction you hear (`docs/dsp/compressor.md`
//!   measures that: about 1.25 × attack for a step 18 dB over the threshold).
//!
//! # Idle
//!
//! [`GlobalProcessor::is_silent`] is the core's `is_idle`: the follower and the gain smoother are exactly zero. The
//! core flushes both below −300 dBFS, so a finished tail reaches that on its own (the release time constant
//! decides how soon); an idle compressor with silent input gives silent output and changes nothing, so the engine
//! may skip it.

use enth_dsp::compressor::{Compressor, CompressorParams};
use enth_patch::catalogue::CompParam;

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The core's settings for the catalogue values `read` returns. A pure copy: the catalogue's ranges lie inside the
/// core's, and every field means the same thing in the same unit.
pub fn core_params(read: impl Fn(CompParam) -> f32) -> CompressorParams {
    CompressorParams {
        threshold_db: read(CompParam::Threshold),
        ratio: read(CompParam::Ratio),
        knee_db: read(CompParam::Knee),
        attack_ms: read(CompParam::Attack),
        release_ms: read(CompParam::Release),
        makeup_db: read(CompParam::Makeup),
        mix_pct: read(CompParam::Mix),
    }
}

/// The compressor.
#[derive(Clone, Debug)]
pub struct Comp {
    core: Compressor,
}

impl Comp {
    /// Prepare a compressor for `cfg.sample_rate` with the patch `values`.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let mut core = Compressor::new();
        core.prepare(cfg.sample_rate);
        core.set_params(&core_params(|p| {
            values.get(p.index()).copied().unwrap_or(p.spec().default)
        }));
        Self { core }
    }

    /// The core, for meters and tools: `gain_reduction_db`, `take_peak_gain_reduction_db`, `params`.
    #[must_use]
    pub fn core(&self) -> &Compressor {
        &self.core
    }

    /// The core, mutable: `take_peak_gain_reduction_db` restarts the meter's peak.
    pub fn core_mut(&mut self) -> &mut Compressor {
        &mut self.core
    }
}

impl GlobalProcessor for Comp {
    /// Clear the follower, the gain smoother and the meters, and read the patch values (`base`).
    fn reset(&mut self, start: &GlobalStart<'_>) {
        self.core.reset();
        self.core.set_params(&core_params(|p| start.params.base(p)));
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        self.core.set_params(&core_params(|p| cx.params.cur(p)));
        self.core.process_in_place(cx.left, cx.right);
    }

    /// `true` when the follower and the gain smoother are exactly zero: silent input then gives silent output and
    /// changes nothing (meters included), whatever the settings.
    fn is_silent(&self) -> bool {
        self.core.is_idle()
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.core.clone_from(&other.core);
    }

    fn subnormals(&self) -> usize {
        self.core.count_subnormals()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{GlobalParams, SEG};
    use enth_patch::ModuleKind;

    fn defaults() -> Vec<f32> {
        ModuleKind::Comp.defaults()
    }

    #[test]
    fn the_catalogue_defaults_are_the_cores_defaults() {
        let v = defaults();
        assert_eq!(
            core_params(|p| v[p.index()]),
            CompressorParams::DEFAULT,
            "the catalogue and the core agree on the default compressor"
        );
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        let mut v = defaults();
        for (p, x) in [
            (CompParam::Threshold, -33.0),
            (CompParam::Ratio, 7.5),
            (CompParam::Attack, 1.25),
            (CompParam::Release, 321.0),
            (CompParam::Knee, 9.0),
            (CompParam::Makeup, 5.5),
            (CompParam::Mix, 42.0),
        ] {
            v[p.index()] = x;
        }
        let c = core_params(|p| v[p.index()]);
        assert_eq!(
            c,
            CompressorParams {
                threshold_db: -33.0,
                ratio: 7.5,
                knee_db: 9.0,
                attack_ms: 1.25,
                release_ms: 321.0,
                makeup_db: 5.5,
                mix_pct: 42.0,
            }
        );
    }

    #[test]
    fn mix_passes_through_in_percent() {
        for m in [0.0, 0.5, 33.3, 100.0] {
            let mut v = defaults();
            v[CompParam::Mix.index()] = m;
            assert_eq!(core_params(|p| v[p.index()]).mix_pct, m);
        }
    }

    #[test]
    fn the_whole_catalogue_range_is_inside_the_cores_ranges() {
        // The core sanitises; a catalogue value must never be changed by it.
        for &p in CompParam::ALL {
            let spec = p.spec();
            for x in [spec.min(), spec.default, spec.max()] {
                let mut v = defaults();
                v[p.index()] = x;
                let c = core_params(|q| v[q.index()]);
                assert_eq!(c, c.sanitized(), "{} = {x}", spec.key);
            }
        }
    }

    #[test]
    fn a_short_values_slice_falls_back_to_the_defaults() {
        let c = Comp::new(&[], &PrepareCfg::new(48_000.0));
        assert_eq!(c.core().params(), CompressorParams::DEFAULT);
        assert!(c.is_silent());
    }

    #[test]
    fn it_is_idle_until_a_signal_has_come_and_gone() {
        let cfg = PrepareCfg::new(48_000.0);
        let vals = defaults();
        let mut c = Comp::new(&vals, &cfg);
        assert!(c.is_silent());
        let params = GlobalParams::from_slices(&vals, &vals, &vals);
        for _ in 0..30 {
            let (mut l, mut r) = ([0.5; SEG], [0.5; SEG]);
            c.process(&mut GlobalCx {
                params,
                left: &mut l,
                right: &mut r,
            });
        }
        assert!(!c.is_silent());
        assert!(c.core().gain_reduction_db() > 0.0);
        // 10 s of silence: both states are flushed to exactly zero (release 200 ms).
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        for _ in 0..(48_000 * 10 / SEG) {
            c.process(&mut GlobalCx {
                params,
                left: &mut l,
                right: &mut r,
            });
        }
        assert!(c.is_silent());
        assert_eq!(c.subnormals(), 0);
    }
}
