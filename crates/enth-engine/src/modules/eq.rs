//! The `eq` module: the engine wrapper around `enth_dsp::eq::Eq` (S2a lane EQ; the core and its reasoning are in
//! `docs/dsp/eq.md`).
//!
//! A six-band stereo parametric equaliser on the bus after the voices (a low cut, a low shelf, two bells, a high
//! shelf and a high cut, in series, then `mix`): zero latency, no look-ahead, no heap memory at all (the core is
//! about 0.5 KB of state, so `new` is the only place that sets anything up and nothing ever allocates). Not
//! `time_based`: the global FX bypass leaves it on, because it shapes tone, not space.
//!
//! # Parameters
//!
//! Every catalogue value goes to the core's field of the same meaning, in the same unit ([`core_controls`]): Hz, dB,
//! Q and percent as they are. The wrapper hands the core the segment's `prev` and `cur` values, and the core ramps
//! each band's coefficients linearly across the 32 frames from the design of the first to the design of the second,
//! and the `mix` likewise. The catalogue gives every parameter the engine's 10 ms smoother, so what arrives is
//! already a smooth line; the ramp is what makes the line smooth *inside* a segment too (a stepped coefficient
//! would put a zipper skirt around every tone: `docs/dsp/eq.md` measures it).
//!
//! # Idle
//!
//! [`GlobalProcessor::is_silent`] is the core's `is_idle`: every section's two integrator voltages are exactly
//! zero. The core flushes them below 1e-15, so a finished tail reaches that on its own (the longest ring the table
//! allows, a +24 dB Q 20 bell at 20 Hz, takes under 40 s); an idle equaliser with silent input gives silent output
//! and changes nothing, so the engine may skip it. The ramp starts from the design of `prev`, not from anything the
//! module remembers, so a skipped segment and a run one leave the same state.

use enth_dsp::eq::{Controls, Eq as Core, SEGMENT};
use enth_patch::catalogue::EqParam;

use crate::module::{GlobalCx, GlobalProcessor, GlobalStart, PrepareCfg, SEG};

// The core ramps over its own segment length; the engine's is the same number.
const _: () = assert!(SEG == SEGMENT);

/// The core's settings for the catalogue values `read` returns. A pure copy: the catalogue's ranges lie inside the
/// core's, and every field means the same thing in the same unit.
pub fn core_controls(read: impl Fn(EqParam) -> f32) -> Controls {
    Controls {
        low_cut_hz: read(EqParam::LowCut),
        low_freq_hz: read(EqParam::LowFreq),
        low_gain_db: read(EqParam::LowGain),
        bell1_freq_hz: read(EqParam::Bell1Freq),
        bell1_gain_db: read(EqParam::Bell1Gain),
        bell1_q: read(EqParam::Bell1Q),
        bell2_freq_hz: read(EqParam::Bell2Freq),
        bell2_gain_db: read(EqParam::Bell2Gain),
        bell2_q: read(EqParam::Bell2Q),
        high_freq_hz: read(EqParam::HighFreq),
        high_gain_db: read(EqParam::HighGain),
        high_cut_hz: read(EqParam::HighCut),
        mix_pct: read(EqParam::Mix),
    }
}

/// The equaliser.
#[derive(Clone, Debug)]
pub struct Eq {
    core: Core,
}

impl Eq {
    /// Prepare an equaliser for `cfg.sample_rate`. The patch `values` are not needed: the core reads the
    /// parameters, previous and current, at every segment.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            core: Core::new(cfg.sample_rate),
        }
    }

    /// The core, for tools and tests.
    #[must_use]
    pub fn core(&self) -> &Core {
        &self.core
    }
}

impl GlobalProcessor for Eq {
    /// Clear every section's state. (The parameters need no reading here: each segment brings its own.)
    fn reset(&mut self, _start: &GlobalStart<'_>) {
        self.core.reset();
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        let prev = core_controls(|p| cx.params.prev(p.index()));
        let cur = core_controls(|p| cx.params.cur(p.index()));
        self.core.process(&prev, &cur, cx.left, cx.right);
    }

    /// `true` when every integrator is exactly zero: silent input then gives silent output and changes nothing,
    /// whatever the settings.
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
    use crate::module::GlobalParams;
    use enth_patch::ModuleKind;

    fn defaults() -> Vec<f32> {
        ModuleKind::Eq.defaults()
    }

    #[test]
    fn the_catalogue_defaults_are_the_cores_defaults() {
        let v = defaults();
        assert_eq!(
            core_controls(|p| v[p.index()]),
            Controls::DEFAULT,
            "the catalogue and the core agree on the default equaliser"
        );
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        let mut v = defaults();
        for (p, x) in [
            (EqParam::LowCut, 33.0),
            (EqParam::LowFreq, 55.0),
            (EqParam::LowGain, -3.5),
            (EqParam::Bell1Freq, 321.0),
            (EqParam::Bell1Gain, 4.25),
            (EqParam::Bell1Q, 2.5),
            (EqParam::Bell2Freq, 3_210.0),
            (EqParam::Bell2Gain, -7.5),
            (EqParam::Bell2Q, 0.75),
            (EqParam::HighFreq, 9_876.0),
            (EqParam::HighGain, 11.0),
            (EqParam::HighCut, 15_000.0),
            (EqParam::Mix, 42.0),
        ] {
            v[p.index()] = x;
        }
        assert_eq!(
            core_controls(|p| v[p.index()]),
            Controls {
                low_cut_hz: 33.0,
                low_freq_hz: 55.0,
                low_gain_db: -3.5,
                bell1_freq_hz: 321.0,
                bell1_gain_db: 4.25,
                bell1_q: 2.5,
                bell2_freq_hz: 3_210.0,
                bell2_gain_db: -7.5,
                bell2_q: 0.75,
                high_freq_hz: 9_876.0,
                high_gain_db: 11.0,
                high_cut_hz: 15_000.0,
                mix_pct: 42.0,
            }
        );
    }

    #[test]
    fn the_whole_catalogue_range_is_inside_the_cores_ranges() {
        // The core sanitises; a catalogue value must never be changed by it.
        for &p in EqParam::ALL {
            let spec = p.spec();
            for x in [spec.min(), spec.default, spec.max()] {
                let mut v = defaults();
                v[p.index()] = x;
                let c = core_controls(|q| v[q.index()]);
                assert_eq!(c, c.sanitized(), "{} = {x}", spec.key);
            }
        }
    }

    #[test]
    fn it_is_idle_until_a_signal_has_come_and_gone() {
        let cfg = PrepareCfg::new(48_000.0);
        let mut vals = defaults();
        vals[EqParam::Bell1Gain.index()] = 9.0;
        let mut e = Eq::new(&vals, &cfg);
        assert!(e.is_silent());
        let params = GlobalParams::from_slices(&vals, &vals, &vals);
        let (mut l, mut r) = ([0.5; SEG], [-0.25; SEG]);
        e.process(&mut GlobalCx {
            tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: &mut l,
            right: &mut r,
        });
        assert!(!e.is_silent());
        for _ in 0..(40 * 48_000 / SEG) {
            let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
            e.process(&mut GlobalCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                params,
                left: &mut l,
                right: &mut r,
            });
            assert_eq!(e.subnormals(), 0);
        }
        assert!(e.is_silent(), "a finished tail is idle");
    }

    #[test]
    fn the_wrapper_is_the_core_with_prev_and_cur() {
        let cfg = PrepareCfg::new(44_100.0);
        let mut prev = defaults();
        let mut cur = defaults();
        prev[EqParam::Bell1Gain.index()] = 3.0;
        cur[EqParam::Bell1Gain.index()] = -6.0;
        cur[EqParam::Mix.index()] = 60.0;
        let params = GlobalParams::from_slices(&prev, &cur, &prev);
        let mut e = Eq::new(&prev, &cfg);
        let mut core = Core::new(44_100.0);
        let mut x = [0.0f32; SEG];
        for (i, v) in x.iter_mut().enumerate() {
            *v = ((i * 7 % 13) as f32 - 6.0) / 8.0;
        }
        let (mut l, mut r) = (x, x);
        e.process(&mut GlobalCx {
            tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: &mut l,
            right: &mut r,
        });
        let (mut wl, mut wr) = (x, x);
        core.process(
            &core_controls(|p| prev[p.index()]),
            &core_controls(|p| cur[p.index()]),
            &mut wl,
            &mut wr,
        );
        assert_eq!(l.map(f32::to_bits), wl.map(f32::to_bits));
        assert_eq!(r.map(f32::to_bits), wr.map(f32::to_bits));
    }

    #[test]
    fn a_copy_continues_where_the_original_would() {
        let cfg = PrepareCfg::new(48_000.0);
        let mut vals = defaults();
        vals[EqParam::LowGain.index()] = 6.0;
        vals[EqParam::Bell2Gain.index()] = -9.0;
        let params = GlobalParams::from_slices(&vals, &vals, &vals);
        let mut a = Eq::new(&vals, &cfg);
        let burst = |e: &mut Eq| {
            let mut l = [0.0f32; SEG];
            for (i, v) in l.iter_mut().enumerate() {
                *v = enth_dsp::util::math::sin(i as f64 * 0.7) as f32;
            }
            let mut r = l;
            e.process(&mut GlobalCx {
                tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
                params,
                left: &mut l,
                right: &mut r,
            });
            (l, r)
        };
        burst(&mut a);
        let mut b = Eq::new(&vals, &cfg);
        b.copy_state_from(&a);
        let (la, ra) = burst(&mut a);
        let (lb, rb) = burst(&mut b);
        assert_eq!(la.map(f32::to_bits), lb.map(f32::to_bits));
        assert_eq!(ra.map(f32::to_bits), rb.map(f32::to_bits));
    }
}
