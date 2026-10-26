//! The `ott` module: the engine wrapper around `enth_dsp::ott::Ott` (S2a lane OTT; the core, its constants and its
//! measurements are in `docs/dsp/ott.md`).
//!
//! Three-band upward and downward compression on the bus after the voices (and earlier effects): zero latency, no
//! look-ahead, no heap memory (the core is a few hundred bytes of state, so `new` is the only place that sets anything
//! up and nothing ever allocates). Not `time_based`: the global FX bypass leaves it on, because it sets level, not
//! space.
//!
//! # Parameters
//!
//! Every catalogue value goes to the core's field of the same meaning in the same unit ([`core_params`]): a pure copy.
//! The wrapper calls [`Ott::set_ramp`] once per 32-frame segment with the previous segment's values (`prev`) and this
//! segment's (`cur`), so `depth`, `upward`, `downward`, `input`, `output`, the three band gains and `mix` ramp
//! linearly across the segment (the engine smooths them over 10 ms, so a settled value is exact). `time`, `low_split`
//! and `high_split` are not smoothed and not modulatable; the core recomputes their coefficients when they change.
//! Both ends come from the engine's parameter state, so skipping an idle module (below) cannot change a bit.
//!
//! # Idle
//!
//! [`GlobalProcessor::is_silent`] is the core's `is_idle`: the filters, followers and gain smoothers are exactly zero.
//! They flush below −300 dBFS, so a finished tail reaches that on its own; an idle module given silence outputs
//! silence and changes nothing, so the engine may skip it with the same bits as a call.

use enth_dsp::ott::{Ott as OttCore, OttParams};
use enth_patch::catalogue::OttParam;

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg};

/// The core's settings for the catalogue values `read` returns. A pure copy: every field means the same thing in the
/// same unit, and the catalogue's ranges equal the core's.
pub fn core_params(read: impl Fn(OttParam) -> f32) -> OttParams {
    OttParams {
        depth: read(OttParam::Depth),
        time: read(OttParam::Time),
        upward: read(OttParam::Upward),
        downward: read(OttParam::Downward),
        input_db: read(OttParam::Input),
        output_db: read(OttParam::Output),
        low_split_hz: read(OttParam::LowSplit),
        high_split_hz: read(OttParam::HighSplit),
        low_gain_db: read(OttParam::LowGain),
        mid_gain_db: read(OttParam::MidGain),
        high_gain_db: read(OttParam::HighGain),
        mix: read(OttParam::Mix),
    }
}

/// The `ott` effect.
#[derive(Clone, Debug)]
pub struct Ott {
    core: OttCore,
}

impl Ott {
    /// Prepare an `ott` effect for `cfg.sample_rate` with the patch `values`.
    #[must_use]
    pub fn new(values: &[f32], cfg: &PrepareCfg) -> Self {
        let mut core = OttCore::new();
        core.prepare(cfg.sample_rate);
        core.set_params(&core_params(|p| {
            values.get(p.index()).copied().unwrap_or(p.spec().default)
        }));
        Self { core }
    }

    /// The core, for meters and tools: `band_gain_db`, `band_level_db`, `params`.
    #[must_use]
    pub fn core(&self) -> &OttCore {
        &self.core
    }
}

impl GlobalProcessor for Ott {
    /// Clear the filters, the followers and the gain smoothers, and read the patch values (`base`).
    fn reset(&mut self, start: &GlobalStart<'_>) {
        self.core.reset();
        self.core.set_params(&core_params(|p| start.params.base(p)));
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        let from = core_params(|p| cx.params.prev(p));
        let to = core_params(|p| cx.params.cur(p));
        self.core.set_ramp(&from, &to);
        self.core.process_in_place(cx.left, cx.right);
    }

    /// `true` when the core is idle: silent input then gives silent output and changes nothing, whatever the
    /// settings.
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
        ModuleKind::Ott.defaults()
    }

    #[test]
    fn the_catalogue_defaults_are_the_cores_defaults() {
        let v = defaults();
        assert_eq!(
            core_params(|p| v[p.index()]),
            OttParams::DEFAULT,
            "the catalogue and the core agree on the default OTT"
        );
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        let mut v = defaults();
        for (p, x) in [
            (OttParam::Depth, 41.0),
            (OttParam::Time, 250.0),
            (OttParam::Upward, 62.0),
            (OttParam::Downward, 17.0),
            (OttParam::Input, 3.5),
            (OttParam::Output, -2.5),
            (OttParam::LowSplit, 120.0),
            (OttParam::HighSplit, 4_000.0),
            (OttParam::LowGain, 1.0),
            (OttParam::MidGain, -2.0),
            (OttParam::HighGain, 3.0),
            (OttParam::Mix, 42.0),
        ] {
            v[p.index()] = x;
        }
        assert_eq!(
            core_params(|p| v[p.index()]),
            OttParams {
                depth: 41.0,
                time: 250.0,
                upward: 62.0,
                downward: 17.0,
                input_db: 3.5,
                output_db: -2.5,
                low_split_hz: 120.0,
                high_split_hz: 4_000.0,
                low_gain_db: 1.0,
                mid_gain_db: -2.0,
                high_gain_db: 3.0,
                mix: 42.0,
            }
        );
    }

    #[test]
    fn the_whole_catalogue_range_is_inside_the_cores_ranges() {
        // The core sanitises; a catalogue value must never be changed by it.
        for &p in OttParam::ALL {
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
        let o = Ott::new(&[], &PrepareCfg::new(48_000.0));
        assert_eq!(o.core().params(), OttParams::DEFAULT);
        assert!(o.is_silent());
    }

    #[test]
    fn it_is_idle_until_a_signal_has_come_and_gone() {
        let cfg = PrepareCfg::new(48_000.0);
        let vals = defaults();
        let mut o = Ott::new(&vals, &cfg);
        assert!(o.is_silent());
        let params = GlobalParams::from_slices(&vals, &vals, &vals);
        let run = |o: &mut Ott, level: f32, segments: usize| {
            for _ in 0..segments {
                let (mut l, mut r) = ([level; SEG], [level; SEG]);
                o.process(&mut GlobalCx {
                    tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                    params,
                    left: &mut l,
                    right: &mut r,
                });
            }
        };
        run(&mut o, 0.5, 30);
        assert!(!o.is_silent());
        // 20 s of silence: every state is flushed to exactly zero (the longest release is 200 ms).
        run(&mut o, 0.0, 48_000 * 20 / SEG);
        assert!(o.is_silent());
        assert_eq!(o.subnormals(), 0);
    }
}
