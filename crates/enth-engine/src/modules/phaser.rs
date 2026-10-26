//! The `phaser` module: the engine wrapper around `enth_dsp::phaser::Phaser` (S2a lane MOD; the core's algorithm,
//! measurements and sources are in `docs/dsp/phaser.md`).
//!
//! A global processor on the stereo bus with zero latency and **no allocation at all**: the core is 12 stages of two
//! floats per channel on the stack. One call per 32-frame segment, which is the block the core sees: the sweep
//! advances by one integer increment per segment, so a render does not depend on the host block size.
//!
//! # Parameters
//!
//! Each segment, [`core_segment`] turns the catalogue's values (`GlobalParams::prev` and `cur`) into the core's
//! [`Segment`]:
//!
//! | Catalogue | Core | Conversion |
//! | --- | --- | --- |
//! | `stages` | `stages` | the last segment's count and this one's (2 to 12); the core crosses the change |
//! | `rate` (Hz) or `beats` with `sync` | `phase_inc` | `lfo::phase_increment` / `lfo::synced_increment`: an integer per segment |
//! | `center` (Hz), `depth` (%) | `center_hz`, `depth` | as they are; `/ 100` for `depth`, at `cur` (the core ramps the coefficient) |
//! | `feedback`, `mix` (%) | the same, as fractions | `/ 100`, at `prev` and `cur` |
//! | `width` (%) | `lag` | [`lag_for_width`]: `width/100 · 180°` of the sweep, as a phase |
//!
//! `rate`, `depth`, `center`, `feedback` and `mix` are smoothed by the engine (20 ms, the catalogue's `Smoothing`);
//! `stages`, `sync`, `beats` and `width` are not, and take effect at the segment boundary (a change of `stages`
//! crossfades over that segment inside the core, so it does not click).
//!
//! # Real-time safety
//!
//! - **NaN and infinity never reach the filters.** One such sample would sit in the all-pass states and the
//!   feedback until `reset()`. Each segment is checked first: NaN becomes 0 and anything beyond ±1e6 (+120 dBFS) is
//!   clamped.
//! - **Idle detection** (see [`GlobalProcessor::is_silent`] below): once the input and the output of a segment are
//!   exactly zero and the state (26 floats) is exactly zero, the wrapper stops running the core. `process` of an
//!   idle wrapper with silent input returns at once, so skipping the call (as the engine does) gives the same bits
//!   as making it. The first non-silent input wakes the core with
//!   [`restart_from_silence`](enth_dsp::phaser::Phaser::restart_from_silence): the sweep restarts at phase 0.
//! - **Swaps.** `copy_state_from` and `take_state_from` copy the 200 bytes of state.
//!
//! # Flags for the global FX bypass (lane S2b)
//!
//! The phaser is `time_based` in the catalogue. A bypassed module is faded out by the engine, then not called;
//! `reset()` it before fading it back in (the `GlobalProcessor::reset` below clears the state and restarts the sweep).

use enth_dsp::chorus::lag_for_width;
use enth_dsp::lfo::{phase_increment, synced_increment};
use enth_dsp::phaser::{MAX_STAGES, Phaser as Core, Segment};
use enth_patch::catalogue::{Beats, PhaserParam};

use crate::module::{Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG};

/// Input beyond this magnitude (+120 dBFS) is clamped before it reaches the filters; NaN becomes 0.
const INPUT_LIMIT: f32 = 1.0e6;

/// The catalogue's `stages` value as a count: rounded, then 2 to 12 (the catalogue's range). NaN is 2.
#[must_use]
pub fn stage_count(value: f32) -> usize {
    let v = value + 0.5;
    // Written so that NaN takes the minimum (`clamp` would pass it on).
    if v >= 2.0 {
        v.min(MAX_STAGES as f32) as usize
    } else {
        2
    }
}

/// The sweep's increment for one segment: `rate` Hz, or with `sync` one cycle per `beats` at `tempo_bpm`.
#[must_use]
pub fn sweep_increment(params: &GlobalParams<'_>, tempo_bpm: f64, sample_rate: f32) -> u64 {
    if params.base(PhaserParam::Sync) == 0.0 {
        phase_increment(params.cur(PhaserParam::Rate), sample_rate, SEG)
    } else {
        let beats = Beats::from_value(params.base(PhaserParam::Beats)).unwrap_or(Beats::Bar);
        synced_increment(tempo_bpm, beats.length(), sample_rate, SEG)
    }
}

/// The core's segment for the catalogue values in `params` (this segment's `cur`, and `prev` for the ramps);
/// `last_stages` is the stage count the previous segment ran.
#[must_use]
pub fn core_segment(
    params: &GlobalParams<'_>,
    tempo_bpm: f64,
    sample_rate: f32,
    last_stages: usize,
) -> Segment {
    let ramp = |p: PhaserParam| [params.prev(p) / 100.0, params.cur(p) / 100.0];
    Segment {
        stages: [last_stages, stage_count(params.base(PhaserParam::Stages))],
        phase_inc: sweep_increment(params, tempo_bpm, sample_rate),
        lag: lag_for_width(params.cur(PhaserParam::Width)),
        center_hz: params.cur(PhaserParam::Center),
        depth: params.cur(PhaserParam::Depth) / 100.0,
        feedback: ramp(PhaserParam::Feedback),
        mix: ramp(PhaserParam::Mix),
    }
}

/// The phaser.
#[derive(Clone, Debug)]
pub struct Phaser {
    core: Core,
    /// The stage count the previous segment ran (the start of this segment's crossfade, if it changed).
    stages: usize,
    /// The state holds only zeros and has been put to rest: silent input gives silent output and changes nothing, so
    /// `process` returns at once. Set by [`GlobalProcessor::reset`] and by a successful check.
    idle: bool,
}

impl Phaser {
    /// Prepare a phaser. Starts idle: a fresh state holds only zeros. `values` is unused (the first segment reads
    /// its parameters from the views), kept for the uniform constructor of the module kinds.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            core: Core::new(cfg.sample_rate),
            stages: 6,
            idle: true,
        }
    }

    /// The core, for tests and tools.
    #[must_use]
    pub fn core(&self) -> &Core {
        &self.core
    }
}

/// Replace NaN by 0 and clamp the rest to ±[`INPUT_LIMIT`]; a block that is already fine is left untouched (the
/// comparison is false for NaN, so NaN always takes the slow path).
#[inline]
fn sanitize(block: &mut Block) {
    if block.iter().all(|x| x.abs() <= INPUT_LIMIT) {
        return;
    }
    for x in block.iter_mut() {
        *x = if x.is_nan() {
            0.0
        } else {
            x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
        };
    }
}

/// Whether every sample is exactly zero (`-0.0` counts).
#[inline]
fn is_zero(block: &Block) -> bool {
    block.iter().all(|x| *x == 0.0)
}

impl GlobalProcessor for Phaser {
    /// Silence the state and restart the sweep: a fresh start, or an un-bypass.
    fn reset(&mut self, start: &GlobalStart<'_>) {
        self.core.reset();
        self.stages = stage_count(start.params.base(PhaserParam::Stages));
        self.idle = true;
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        sanitize(cx.left);
        sanitize(cx.right);
        let input_silent = is_zero(cx.left) && is_zero(cx.right);
        if self.idle {
            if input_silent {
                return;
            }
            self.core.restart_from_silence();
            // The state is zero: a stage change needs no crossfade, and the count must not depend on how many silent
            // segments were skipped.
            self.stages = stage_count(cx.params.base(PhaserParam::Stages));
            self.idle = false;
        }
        let seg = core_segment(
            &cx.params,
            cx.tempo_bpm,
            self.core.sample_rate(),
            self.stages,
        );
        self.core.process_block::<SEG>(&seg, cx.left, cx.right);
        self.stages = seg.stages[1];

        // 26 floats: cheap enough to look at after every silent segment.
        if input_silent && is_zero(cx.left) && is_zero(cx.right) && self.core.is_state_zero() {
            self.idle = true;
        }
    }

    /// `true` once the state provably holds only zeros (after `reset`, or when a tail has died out exactly: the core
    /// flushes below 1e-15). Then `process` with silent input outputs silence and changes nothing that survives a
    /// wake, whether the engine calls it or skips it, so the skip is bit-identical to running.
    fn is_silent(&self) -> bool {
        self.idle
    }

    fn copy_state_from(&mut self, other: &Self) {
        if self.core.sample_rate().to_bits() == other.core.sample_rate().to_bits() {
            self.core.clone_from(&other.core);
            self.stages = other.stages;
            self.idle = other.idle;
        }
    }

    fn subnormals(&self) -> usize {
        self.core.count_subnormals()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_dsp::util::math;
    use enth_patch::ModuleKind;

    const SR: f32 = 48_000.0;

    fn defaults() -> Vec<f32> {
        ModuleKind::Phaser.defaults()
    }

    fn views(vals: &[f32]) -> GlobalParams<'_> {
        GlobalParams::from_slices(vals, vals, vals)
    }

    fn step(p: &mut Phaser, vals: &[f32], l: &mut Block, r: &mut Block) {
        p.process(&mut GlobalCx {
            tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
            params: views(vals),
            left: l,
            right: r,
        });
    }

    fn set(v: &mut [f32], p: PhaserParam, x: f32) {
        v[p.index()] = x;
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        let mut v = defaults();
        set(&mut v, PhaserParam::Stages, 9.0);
        set(&mut v, PhaserParam::Rate, 2.0);
        set(&mut v, PhaserParam::Depth, 25.0);
        set(&mut v, PhaserParam::Center, 777.0);
        set(&mut v, PhaserParam::Feedback, -40.0);
        set(&mut v, PhaserParam::Width, 80.0);
        set(&mut v, PhaserParam::Mix, 70.0);
        let s = core_segment(&views(&v), 140.0, SR, 4);
        assert_eq!(s.stages, [4, 9]);
        assert_eq!(s.phase_inc, phase_increment(2.0, SR, SEG));
        assert_eq!(s.lag, lag_for_width(80.0));
        assert_eq!(s.center_hz, 777.0);
        assert_eq!(s.depth, 0.25);
        assert_eq!(s.feedback, [-0.4; 2]);
        assert_eq!(s.mix, [0.7; 2]);
    }

    #[test]
    fn prev_and_cur_become_the_two_ends_of_the_ramp() {
        let prev = defaults();
        let mut cur = defaults();
        set(&mut cur, PhaserParam::Mix, 20.0);
        set(&mut cur, PhaserParam::Feedback, 90.0);
        let s = core_segment(&GlobalParams::from_slices(&cur, &cur, &prev), 140.0, SR, 6);
        assert_eq!(s.mix, [1.0, 0.2]);
        assert_eq!(s.feedback, [0.3, 0.9]);
    }

    #[test]
    fn the_defaults_are_the_documented_gentle_phaser() {
        let s = core_segment(&views(&defaults()), 140.0, SR, 6);
        assert_eq!(s.stages, [6, 6]);
        assert_eq!(s.center_hz, 1_000.0);
        assert_eq!(s.depth, 0.5);
        assert_eq!(s.feedback, [0.3; 2]);
        assert_eq!(s.mix, [1.0; 2]);
        assert_eq!(s.lag, lag_for_width(50.0));
    }

    #[test]
    fn stage_counts_are_rounded_and_inside_two_to_twelve() {
        assert_eq!(stage_count(2.0), 2);
        assert_eq!(stage_count(12.0), 12);
        assert_eq!(stage_count(5.4), 5);
        assert_eq!(stage_count(5.6), 6);
        assert_eq!(stage_count(-3.0), 2);
        assert_eq!(stage_count(99.0), 12);
        assert_eq!(stage_count(f32::NAN), 2);
        assert_eq!(stage_count(f32::INFINITY), 12);
        let spec = PhaserParam::Stages.spec();
        assert_eq!(stage_count(spec.min()), 2);
        assert_eq!(stage_count(spec.max()), MAX_STAGES);
        assert_eq!(stage_count(spec.default), 6);
    }

    #[test]
    fn sync_takes_the_increment_from_the_beats_and_the_tempo() {
        let mut v = defaults();
        set(&mut v, PhaserParam::Sync, 1.0);
        set(&mut v, PhaserParam::Beats, Beats::Quarter.index() as f32);
        let at = |bpm| core_segment(&views(&v), bpm, SR, 6).phase_inc;
        assert_eq!(at(120.0), synced_increment(120.0, (1, 1), SR, SEG));
        // One cycle per beat at 120 BPM is 2 Hz, whatever `rate` says.
        assert_eq!(at(120.0), phase_increment(2.0, SR, SEG));
        assert!((at(240.0) as f64 / at(120.0) as f64 - 2.0).abs() < 1e-12);
    }

    #[test]
    fn nan_and_out_of_range_catalogue_values_do_not_panic() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30] {
            let v = vec![x; 9];
            let mut p = Phaser::new(&v, &PrepareCfg::new(SR));
            let (mut l, mut r) = ([0.25f32; SEG], [0.25f32; SEG]);
            for _ in 0..4 {
                step(&mut p, &v, &mut l, &mut r);
                assert!(l.iter().chain(&r).all(|y| y.is_finite()), "{x}");
            }
        }
    }

    #[test]
    fn sanitize_zeroes_nan_and_clamps_infinity_and_leaves_the_rest_alone() {
        let mut b: Block = [0.25; SEG];
        let before = b;
        sanitize(&mut b);
        assert_eq!(b, before);
        b[3] = f32::NAN;
        b[4] = f32::INFINITY;
        b[5] = f32::NEG_INFINITY;
        b[6] = 2.0e6;
        b[7] = -0.0;
        sanitize(&mut b);
        assert_eq!(b[3].to_bits(), 0.0f32.to_bits());
        assert_eq!(b[4].to_bits(), INPUT_LIMIT.to_bits());
        assert_eq!(b[5].to_bits(), (-INPUT_LIMIT).to_bits());
        assert_eq!(b[6].to_bits(), INPUT_LIMIT.to_bits());
        assert_eq!(b[0].to_bits(), 0.25f32.to_bits());
        assert_eq!(b[7].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn a_fresh_phaser_is_idle_and_skipping_silence_changes_nothing() {
        let vals = defaults();
        let mut p = Phaser::new(&vals, &PrepareCfg::new(SR));
        assert!(p.is_silent());
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        for _ in 0..10 {
            step(&mut p, &vals, &mut l, &mut r);
        }
        assert!(p.is_silent());
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
        l[0] = 1.0;
        step(&mut p, &vals, &mut l, &mut r);
        assert!(!p.is_silent());
    }

    #[test]
    fn the_tail_dies_out_and_the_wrapper_goes_idle_again() {
        let vals = defaults();
        let mut p = Phaser::new(&vals, &PrepareCfg::new(SR));
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        l[0] = 1.0;
        step(&mut p, &vals, &mut l, &mut r);
        assert!(!p.is_silent());
        // The default's loop (1 kHz, 6 stages, feedback 30 %) rings for well under a second.
        for _ in 0..(10 * 48_000 / SEG) {
            let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
            step(&mut p, &vals, &mut l, &mut r);
            if p.is_silent() {
                return;
            }
        }
        panic!("still running after 10 s");
    }

    #[test]
    fn skipping_an_idle_phaser_is_bit_identical_to_running_it() {
        // A depth of 100 % and a rate that is not a divisor of the silence, so a sweep that kept running while idle
        // would show.
        let mut vals = defaults();
        set(&mut vals, PhaserParam::Depth, 100.0);
        set(&mut vals, PhaserParam::Rate, 3.3);
        let cfg = PrepareCfg::new(SR);
        let (mut a, mut b) = (Phaser::new(&vals, &cfg), Phaser::new(&vals, &cfg));
        for round in 0..6 {
            let mut la: Block = core::array::from_fn(|i| 0.7 * math::sinf((i + round) as f32));
            let mut ra = la;
            let (mut lb, mut rb) = (la, ra);
            step(&mut a, &vals, &mut la, &mut ra);
            step(&mut b, &vals, &mut lb, &mut rb);
            assert_eq!((la, ra), (lb, rb), "round {round}");
            for _ in 0..1300 {
                let (mut sl, mut sr) = ([0.0; SEG], [0.0; SEG]);
                step(&mut a, &vals, &mut sl, &mut sr);
                if !b.is_silent() {
                    let (mut tl, mut tr) = ([0.0; SEG], [0.0; SEG]);
                    step(&mut b, &vals, &mut tl, &mut tr);
                    assert_eq!((sl, sr), (tl, tr));
                }
            }
        }
    }

    #[test]
    fn copy_gives_the_same_future() {
        let mut vals = defaults();
        set(&mut vals, PhaserParam::Feedback, 70.0);
        let cfg = PrepareCfg::new(SR);
        let mut src = Phaser::new(&vals, &cfg);
        for k in 0..50 {
            let mut l: Block = core::array::from_fn(|i| math::sinf((k * 32 + i) as f32 * 0.01));
            let mut r = l;
            step(&mut src, &vals, &mut l, &mut r);
        }
        let mut copied = Phaser::new(&vals, &cfg);
        copied.copy_state_from(&src);
        let mut taken = Phaser::new(&vals, &cfg);
        taken.take_state_from(&mut src);
        assert!(!taken.is_silent() && !copied.is_silent());
        for k in 50..120 {
            let mut l: Block = core::array::from_fn(|i| math::sinf((k * 32 + i) as f32 * 0.01));
            let (mut r, mut l2, mut r2) = (l, l, l);
            step(&mut copied, &vals, &mut l, &mut r);
            step(&mut taken, &vals, &mut l2, &mut r2);
            assert_eq!((l, r), (l2, r2));
        }
    }

    #[test]
    fn reset_clears_and_idles() {
        let vals = defaults();
        let mut p = Phaser::new(&vals, &PrepareCfg::new(SR));
        let (mut l, mut r) = ([0.5; SEG], [0.5; SEG]);
        step(&mut p, &vals, &mut l, &mut r);
        assert!(!p.is_silent());
        p.reset(&GlobalStart {
            seed: 0,
            module_key: 0,
            params: views(&vals),
        });
        assert!(p.is_silent() && p.core().is_state_zero());
    }
}
