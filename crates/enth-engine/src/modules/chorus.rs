//! The `chorus` module: the engine wrapper around `enth_dsp::chorus::Chorus` (S2a lane MOD; the core's algorithm,
//! measurements and sources are in `docs/dsp/chorus.md`).
//!
//! A global processor on the stereo bus with zero latency. `Core::new` in [`Chorus::new`] is the only allocation: two
//! delay lines of 4,096 samples at 44.1 and 48 kHz (32 KiB), sized from the sample rate alone. One call per 32-frame
//! segment, which is the block the core sees: the sweep advances by one integer increment per segment, so a render
//! does not depend on the host block size.
//!
//! # Parameters
//!
//! Each segment, [`core_segment`] turns the catalogue's values (`GlobalParams::prev` and `cur`) into the core's
//! [`Segment`], and the core ramps linearly from the first to the second over the segment's 32 frames:
//!
//! | Catalogue | Core | Conversion |
//! | --- | --- | --- |
//! | `mode` | `mode` | chorus, ensemble, flanger |
//! | `rate` (Hz) or `beats` with `sync` | `phase_inc` | `lfo::phase_increment` / `lfo::synced_increment`: an integer per segment |
//! | `delay` (ms) | `delay` (samples) | `ms · sample_rate / 1000`, at `prev` and `cur` |
//! | `depth`, `feedback`, `mix` (%) | the same, as fractions | `/ 100`, at `prev` and `cur` |
//! | `width` (%) | `lag` | [`lag_for_width`]: `width/100 · 180°` of the sweep, as a phase |
//!
//! `rate`, `depth`, `delay`, `feedback` and `mix` are smoothed by the engine (20 ms, the catalogue's `Smoothing`), so
//! `prev` to `cur` is already a gentle ramp and the core's per-sample ramp only joins the segments. `mode`, `sync`,
//! `beats` and `width` are not smoothed: they take effect at the segment boundary (`mode` changes the sweep and the
//! number of taps at once; the delay lines are shared, so nothing clicks beyond the tap changing).
//!
//! # Real-time safety
//!
//! - **NaN and infinity never reach the lines.** One such sample would sit in the delay (and, with feedback, circulate
//!   until `reset()`). Each segment is checked first (about 8 instructions per frame): NaN becomes 0 and anything
//!   beyond ±1e6 (+120 dBFS) is clamped.
//! - **Idle detection** (see [`GlobalProcessor::is_silent`] below): once the input and the output have been exactly
//!   zero for a while, the wrapper scans the lines once, and if they hold only zeros it stops running the core.
//!   `process` of an idle wrapper with silent input returns at once, so skipping the call (as the engine does) gives
//!   the same bits as making it. The first non-silent input wakes the core with
//!   [`restart_from_silence`](enth_dsp::chorus::Chorus::restart_from_silence): the sweep restarts at phase 0
//!   (otherwise its phase would depend on how many segments were skipped).
//! - **Swaps.** `copy_state_from` (a crossfading plan swap, both plans run) copies the 32 KiB of line; `take_state_from`
//!   (an instant swap) exchanges the two cores in O(1).
//!
//! # Flags for the global FX bypass (lane S2b)
//!
//! The chorus is `time_based` in the catalogue. A bypassed module is faded out by the engine, then not called;
//! `reset()` it before fading it back in (the `GlobalProcessor::reset` below clears the lines and restarts the sweep).

use enth_dsp::chorus::{Chorus as Core, Mode, Segment, lag_for_width};
use enth_dsp::lfo::{phase_increment, synced_increment};
use enth_patch::catalogue::{Beats, ChorusMode, ChorusParam};

use crate::module::{Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG};

/// Input beyond this magnitude (+120 dBFS) is clamped before it reaches the lines; NaN becomes 0.
const INPUT_LIMIT: f32 = 1.0e6;

/// Zero frames (input and output) before the lines are first scanned for idleness: 0.68 s at 48 kHz. A scan costs
/// under 10 µs (8,192 floats), so this only keeps a live tail from being scanned in vain.
const SCAN_FIRST: u32 = 32_768;
/// The longest wait between two scans of lines that still hold a little state (22 s at 48 kHz).
const SCAN_MAX: u32 = 1 << 20;

/// The core's mode for a catalogue `mode` value (an unknown value is the default, a chorus).
#[must_use]
pub fn core_mode(value: f32) -> Mode {
    match ChorusMode::from_value(value) {
        Some(ChorusMode::Ensemble) => Mode::Ensemble,
        Some(ChorusMode::Flanger) => Mode::Flanger,
        Some(ChorusMode::Chorus) | None => Mode::Chorus,
    }
}

/// The sweep's increment for one segment: `rate` Hz, or with `sync` one cycle per `beats` at `tempo_bpm`.
#[must_use]
pub fn sweep_increment(params: &GlobalParams<'_>, tempo_bpm: f64, sample_rate: f32) -> u64 {
    if params.base(ChorusParam::Sync) == 0.0 {
        phase_increment(params.cur(ChorusParam::Rate), sample_rate, SEG)
    } else {
        let beats = Beats::from_value(params.base(ChorusParam::Beats)).unwrap_or(Beats::Bar);
        synced_increment(tempo_bpm, beats.length(), sample_rate, SEG)
    }
}

/// The core's segment for the catalogue values in `params` (this segment's `cur`, and `prev` for the ramps).
#[must_use]
pub fn core_segment(params: &GlobalParams<'_>, tempo_bpm: f64, sample_rate: f32) -> Segment {
    let ramp = |p: ChorusParam, f: &dyn Fn(f32) -> f32| [f(params.prev(p)), f(params.cur(p))];
    let percent = |x: f32| x / 100.0;
    // `ms · fs / 1000`, not `ms · (fs / 1000)`: exact for the round values (7 ms at 48 kHz is 336 samples).
    let samples = |ms: f32| ms * sample_rate / 1000.0;
    Segment {
        mode: core_mode(params.base(ChorusParam::Mode)),
        phase_inc: sweep_increment(params, tempo_bpm, sample_rate),
        lag: lag_for_width(params.cur(ChorusParam::Width)),
        delay: ramp(ChorusParam::Delay, &samples),
        depth: ramp(ChorusParam::Depth, &percent),
        feedback: ramp(ChorusParam::Feedback, &percent),
        mix: ramp(ChorusParam::Mix, &percent),
    }
}

/// The chorus.
#[derive(Debug)]
pub struct Chorus {
    core: Core,
    /// The lines hold only zeros and have been put to rest: silent input gives silent output and changes nothing, so
    /// `process` returns at once. Set by [`GlobalProcessor::reset`] and by a successful scan.
    idle: bool,
    /// Frames in a row for which both the input and the output of the core were exactly zero.
    quiet: u32,
    /// The `quiet` count at which the lines are scanned next.
    scan_at: u32,
}

impl Chorus {
    /// Prepare a chorus: allocates the delay lines (sized from `cfg.sample_rate`). Starts idle: a fresh line holds
    /// only zeros. `values` is unused (the first segment reads its parameters from the views), kept for the uniform
    /// constructor of the module kinds.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            core: Core::new(cfg.sample_rate),
            idle: true,
            quiet: 0,
            scan_at: SCAN_FIRST,
        }
    }

    /// The core, for tests and tools (`allocated_bytes`).
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

impl GlobalProcessor for Chorus {
    /// Silence the lines and restart the sweep: a fresh start, or an un-bypass. Does not allocate (a 32 KiB clear).
    fn reset(&mut self, _start: &GlobalStart<'_>) {
        self.core.reset();
        self.idle = true;
        self.quiet = 0;
        self.scan_at = SCAN_FIRST;
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
            self.idle = false;
        }
        let seg = core_segment(&cx.params, cx.tempo_bpm, self.core.sample_rate());
        self.core.process_block::<SEG>(&seg, cx.left, cx.right);

        if input_silent && is_zero(cx.left) && is_zero(cx.right) {
            self.quiet = self.quiet.saturating_add(SEG as u32);
            if self.quiet >= self.scan_at {
                if self.core.is_state_zero() {
                    self.idle = true;
                } else {
                    self.scan_at = self.scan_at.saturating_mul(2).min(SCAN_MAX);
                }
            }
        } else {
            self.quiet = 0;
            self.scan_at = SCAN_FIRST;
        }
    }

    /// `true` once the lines provably hold only zeros (after `reset`, or when a tail has died out exactly: the core
    /// flushes below 1e-15). Then `process` with silent input outputs silence and changes nothing, whether the
    /// engine calls it or skips it, so the skip is bit-identical to running.
    fn is_silent(&self) -> bool {
        self.idle
    }

    fn copy_state_from(&mut self, other: &Self) {
        if self.core.copy_state_from(&other.core) {
            self.idle = other.idle;
            self.quiet = other.quiet;
            self.scan_at = other.scan_at;
        }
    }

    /// An instant plan swap: exchange the two cores, an O(1) move of the lines instead of `copy_state_from`'s 32 KiB
    /// copy. `other` is left with this module's old core, which the main thread drops with the old plan.
    fn take_state_from(&mut self, other: &mut Self) {
        if self.core.swap_state(&mut other.core) {
            self.idle = other.idle;
            self.quiet = other.quiet;
            self.scan_at = other.scan_at;
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
        ModuleKind::Chorus.defaults()
    }

    fn views(vals: &[f32]) -> GlobalParams<'_> {
        GlobalParams::from_slices(vals, vals, vals)
    }

    fn step(c: &mut Chorus, vals: &[f32], l: &mut Block, r: &mut Block) {
        c.process(&mut GlobalCx {
            tempo_bpm: crate::tempo::DEFAULT_TEMPO_BPM,
            params: views(vals),
            left: l,
            right: r,
        });
    }

    fn set(v: &mut [f32], p: ChorusParam, x: f32) {
        v[p.index()] = x;
    }

    #[test]
    fn every_catalogue_value_lands_in_its_core_field() {
        // A distinct value per parameter, so a crossed wire shows.
        let mut v = defaults();
        set(
            &mut v,
            ChorusParam::Mode,
            ChorusMode::Ensemble.index() as f32,
        );
        set(&mut v, ChorusParam::Rate, 2.0);
        set(&mut v, ChorusParam::Delay, 10.0);
        set(&mut v, ChorusParam::Depth, 25.0);
        set(&mut v, ChorusParam::Feedback, -40.0);
        set(&mut v, ChorusParam::Width, 50.0);
        set(&mut v, ChorusParam::Mix, 80.0);
        let s = core_segment(&views(&v), 140.0, SR);
        assert_eq!(s.mode, Mode::Ensemble);
        assert_eq!(s.phase_inc, phase_increment(2.0, SR, SEG));
        assert_eq!(s.lag, lag_for_width(50.0));
        assert_eq!(s.delay, [480.0; 2]);
        assert_eq!(s.depth, [0.25; 2]);
        assert_eq!(s.feedback, [-0.4; 2]);
        assert_eq!(s.mix, [0.8; 2]);
    }

    #[test]
    fn prev_and_cur_become_the_two_ends_of_the_ramp() {
        let (prev, cur) = (defaults(), {
            let mut v = defaults();
            set(&mut v, ChorusParam::Mix, 20.0);
            set(&mut v, ChorusParam::Delay, 14.0);
            v
        });
        let s = core_segment(&GlobalParams::from_slices(&cur, &cur, &prev), 140.0, SR);
        assert_eq!(s.mix, [1.0, 0.2]);
        assert_eq!(s.delay, [336.0, 672.0]);
    }

    #[test]
    fn mix_100_is_exactly_one_and_0_exactly_zero() {
        let mut v = defaults();
        assert_eq!(core_segment(&views(&v), 140.0, SR).mix, [1.0; 2]);
        set(&mut v, ChorusParam::Mix, 0.0);
        let s = core_segment(&views(&v), 140.0, SR);
        assert_eq!(s.mix[0].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn sync_takes_the_increment_from_the_beats_and_the_tempo() {
        let mut v = defaults();
        set(&mut v, ChorusParam::Sync, 1.0);
        set(&mut v, ChorusParam::Beats, Beats::Quarter.index() as f32);
        let at = |bpm| core_segment(&views(&v), bpm, SR).phase_inc;
        assert_eq!(at(120.0), synced_increment(120.0, (1, 1), SR, SEG));
        // One cycle per beat at 120 BPM is 2 Hz, whatever `rate` says.
        assert_eq!(at(120.0), phase_increment(2.0, SR, SEG));
        assert!((at(240.0) as f64 / at(120.0) as f64 - 2.0).abs() < 1e-12);
    }

    #[test]
    fn the_catalogue_defaults_are_the_documented_gentle_chorus() {
        let s = core_segment(&views(&defaults()), 140.0, SR);
        assert_eq!(s.mode, Mode::Chorus);
        assert_eq!(s.delay, [336.0; 2]); // 7 ms
        assert_eq!(s.depth, [0.5; 2]);
        assert_eq!(s.feedback, [0.0; 2]);
        assert_eq!(s.mix, [1.0; 2]);
        assert_eq!(s.lag, lag_for_width(100.0));
    }

    #[test]
    fn nan_and_out_of_range_catalogue_values_do_not_panic() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30] {
            let v = vec![x; 9];
            let mut c = Chorus::new(&v, &PrepareCfg::new(SR));
            let (mut l, mut r) = ([0.25f32; SEG], [0.25f32; SEG]);
            for _ in 0..4 {
                step(&mut c, &v, &mut l, &mut r);
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
    fn the_lines_are_32_kib_and_allocated_in_new() {
        let c = Chorus::new(&[], &PrepareCfg::new(SR));
        assert_eq!(c.core().allocated_bytes(), 2 * 4096 * 4);
        assert!(c.is_silent());
    }

    #[test]
    fn a_fresh_chorus_is_idle_and_skipping_silence_changes_nothing() {
        let vals = defaults();
        let mut c = Chorus::new(&vals, &PrepareCfg::new(SR));
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        for _ in 0..10 {
            step(&mut c, &vals, &mut l, &mut r);
        }
        assert!(c.is_silent());
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
        l[0] = 1.0;
        step(&mut c, &vals, &mut l, &mut r);
        assert!(!c.is_silent());
    }

    #[test]
    fn the_tail_dies_out_and_the_wrapper_goes_idle_again() {
        let vals = defaults();
        let mut c = Chorus::new(&vals, &PrepareCfg::new(SR));
        let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
        l[0] = 1.0;
        step(&mut c, &vals, &mut l, &mut r);
        assert!(!c.is_silent());
        // Without feedback the tail is the delay (at most 2 × 7 ms): the lines clear within a few ms, then the
        // first scan (32,768 frames) finds zeros.
        for _ in 0..(40_000 / SEG) {
            let (mut l, mut r) = ([0.0; SEG], [0.0; SEG]);
            step(&mut c, &vals, &mut l, &mut r);
        }
        assert!(c.is_silent());
    }

    #[test]
    fn skipping_an_idle_chorus_is_bit_identical_to_running_it() {
        let vals = defaults();
        let impulse = |l: &mut Block| {
            l.fill(0.0);
            l[0] = 0.8;
        };
        let (mut a, mut b) = (
            Chorus::new(&vals, &PrepareCfg::new(SR)),
            Chorus::new(&vals, &PrepareCfg::new(SR)),
        );
        let (mut la, mut ra, mut rb) = ([0.0; SEG], [0.0; SEG], [0.0; SEG]);
        // `a` is called for every silent segment, `b` is skipped while it says it is silent.
        for round in 0..6 {
            impulse(&mut la);
            let mut lb = la;
            ra.fill(0.0);
            rb.fill(0.0);
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
    fn copy_and_take_give_the_same_future() {
        let mut vals = defaults();
        set(&mut vals, ChorusParam::Feedback, 60.0);
        set(
            &mut vals,
            ChorusParam::Mode,
            ChorusMode::Flanger.index() as f32,
        );
        let cfg = PrepareCfg::new(SR);
        let mut src = Chorus::new(&vals, &cfg);
        for k in 0..50 {
            let mut l: Block = core::array::from_fn(|i| math::sinf((k * 32 + i) as f32 * 0.01));
            let mut r = l;
            step(&mut src, &vals, &mut l, &mut r);
        }
        let mut copied = Chorus::new(&vals, &cfg);
        copied.copy_state_from(&src);
        let mut taken = Chorus::new(&vals, &cfg);
        let mut donor = Chorus::new(&vals, &cfg);
        core::mem::swap(&mut donor, &mut src);
        taken.take_state_from(&mut donor);
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
    fn take_moves_the_state_and_copy_copies_it() {
        let vals = defaults();
        let cfg = PrepareCfg::new(SR);
        let excite = |c: &mut Chorus| {
            let (mut l, mut r) = ([0.5; SEG], [0.5; SEG]);
            step(c, &vals, &mut l, &mut r);
        };
        // Instant swap: the new plan's chorus takes the lines and is left holding the old plan's fresh ones.
        let (mut old, mut new) = (Chorus::new(&vals, &cfg), Chorus::new(&vals, &cfg));
        excite(&mut old);
        let lines = old.core().line_ptrs();
        new.take_state_from(&mut old);
        assert!(!new.is_silent() && !new.core().is_state_zero());
        assert_eq!(
            new.core().line_ptrs(),
            lines,
            "the allocation moved, nothing was copied"
        );
        assert!(old.core().is_state_zero(), "moved, not copied");
        // Crossfading swap: both plans run, so the state is copied and the old one keeps its own.
        let (mut old, mut new) = (Chorus::new(&vals, &cfg), Chorus::new(&vals, &cfg));
        excite(&mut old);
        let lines = old.core().line_ptrs();
        new.copy_state_from(&old);
        assert!(!new.core().is_state_zero() && !old.core().is_state_zero());
        assert_eq!(old.core().line_ptrs(), lines);
        assert_ne!(new.core().line_ptrs(), lines);
    }

    #[test]
    fn reset_clears_and_idles() {
        let vals = defaults();
        let mut c = Chorus::new(&vals, &PrepareCfg::new(SR));
        let (mut l, mut r) = ([0.5; SEG], [0.5; SEG]);
        step(&mut c, &vals, &mut l, &mut r);
        assert!(!c.is_silent());
        c.reset(&GlobalStart {
            seed: 0,
            module_key: 0,
            params: views(&vals),
        });
        assert!(c.is_silent() && c.core().is_state_zero());
    }
}
