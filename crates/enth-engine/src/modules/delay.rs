//! The `delay` module: the engine wrapper around `enth_dsp::delay::Delay` (S2a lane DLY; `docs/dsp/delay.md`).
//!
//! A stereo echo on the bus after the voices: zero latency (the dry signal is never delayed), 2 s of ring per channel
//! allocated in [`Delay::new`] from the sample rate alone (768,000 bytes at 48 kHz), one call per 32-frame segment.
//! `time_based` in the catalogue, so the global FX bypass fades it out and `reset`s it before it comes back.
//!
//! # Parameters, each segment ([`block_params`])
//!
//! | Catalogue | Core | Conversion |
//! | --- | --- | --- |
//! | `sync`, `beats`, `time` and the tempo | `delay` (samples) | synced: `round(num·60·sr / (den·bpm))` of the segment's tempo; else `round(ms·sr/1000)`; both clamped to `1..=2 s` |
//! | `feedback`, `mix` (%) | the same, 0–1 | `/ 100`, the engine's 10 ms smoothing ramped per sample from the previous segment's value |
//! | `low_cut`, `high_cut` (Hz) | the same | the filters' coefficients ramp per sample to this segment's value |
//! | `ping_pong` | the same | blended over 10 ms |
//!
//! The discrete values (`sync`, `beats`, `time`, `ping_pong`) are read as `base`: none of them is modulatable or
//! smoothed. The time is a whole number of samples, so a change to it (a new `beats`, a tempo change, a new `time`)
//! starts a 20 ms crossfade to the new read position in the core, never a jump; a tempo that changes in the middle of
//! a host block applies at the next segment boundary, like every event, so the host's block size cannot move a bit.
//!
//! # Idle
//!
//! [`GlobalProcessor::is_silent`] is the core's idleness: every ring slot has been overwritten by exact zeros and the
//! filters are at rest. The engine then skips the module while its input is exactly zero, which gives the bits it
//! would have given by running, because the core's idle path does nothing and a wake-up starts from the block's own
//! values.
//!
//! # Plan swaps
//!
//! An instant swap (nothing added or removed) *takes* the state: the rings are exchanged in O(1) and nothing is
//! copied. A swap that crossfades runs both plans for 256 frames, so the new plan's delay gets a copy of the old
//! one's two rings (768,000 bytes at 48 kHz; free when the old delay is idle). `docs/dsp/delay.md` has the measured
//! swap block.

use enth_dsp::delay::{self, BLOCK, BlockParams, Delay as Core};
use enth_patch::catalogue::{Beats, DelayParam};

use crate::module::{GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG};

// The core's block is the engine's segment.
const _: () = assert!(SEG == BLOCK);

/// The core's block parameters for one segment: `p` holds the catalogue values (percent, Hz, ms), `tempo_bpm` the
/// segment's tempo, `sample_rate` the plan's and `max` the ring length ([`Core::max_delay`]).
#[must_use]
pub fn block_params(
    p: GlobalParams<'_>,
    tempo_bpm: f64,
    sample_rate: f32,
    max: usize,
) -> BlockParams {
    let at = |param: DelayParam| param.index();
    let delay = if p.base(at(DelayParam::Sync)) == 0.0 {
        delay::delay_for_ms(p.base(at(DelayParam::Time)), sample_rate, max)
    } else {
        let item = Beats::from_value(p.base(at(DelayParam::Beats))).unwrap_or(Beats::EighthDotted);
        let (num, den) = item.length();
        delay::delay_for_beats(num, den, tempo_bpm, sample_rate, max)
    };
    BlockParams {
        delay,
        feedback_start: p.prev(at(DelayParam::Feedback)) / 100.0,
        feedback_end: p.cur(at(DelayParam::Feedback)) / 100.0,
        mix_start: p.prev(at(DelayParam::Mix)) / 100.0,
        mix_end: p.cur(at(DelayParam::Mix)) / 100.0,
        low_cut_hz: p.cur(at(DelayParam::LowCut)),
        high_cut_hz: p.cur(at(DelayParam::HighCut)),
        ping_pong: p.base(at(DelayParam::PingPong)) != 0.0,
    }
}

/// The delay.
#[derive(Clone, Debug)]
pub struct Delay {
    core: Core,
}

impl Delay {
    /// Prepare a delay for `cfg.sample_rate`: allocates the rings. Starts idle.
    #[must_use]
    pub fn new(_values: &[f32], cfg: &PrepareCfg) -> Self {
        Self {
            core: Core::new(cfg.sample_rate),
        }
    }

    /// The core, for tests and tools (`max_delay`, `allocated_bytes`, `current_delay`).
    #[must_use]
    pub fn core(&self) -> &Core {
        &self.core
    }
}

impl GlobalProcessor for Delay {
    /// Clear the rings (skipped when they already are zero) and go idle: a fresh start, or an un-bypass.
    fn reset(&mut self, _start: &GlobalStart<'_>) {
        self.core.reset();
    }

    fn process(&mut self, cx: &mut GlobalCx<'_>) {
        let p = block_params(
            cx.params,
            cx.tempo_bpm,
            self.core.sample_rate(),
            self.core.max_delay(),
        );
        self.core.process(&p, cx.left, cx.right);
    }

    fn is_silent(&self) -> bool {
        self.core.is_idle()
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.core.copy_state_from(&other.core);
    }

    /// An instant swap: exchange the rings, an O(1) move of 768,000 bytes (at 48 kHz) instead of a copy. `other` is
    /// left with this module's old, fresh rings, which the main thread drops with the old plan.
    fn take_state_from(&mut self, other: &mut Self) {
        self.core.take_state_from(&mut other.core);
    }

    fn subnormals(&self) -> usize {
        self.core.count_subnormals()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::DEFAULT_TEMPO_BPM;
    use enth_patch::ModuleKind;
    use enth_patch::catalogue::DELAY_BEATS;

    const SR: f32 = 48_000.0;

    fn defaults() -> Vec<f32> {
        ModuleKind::Delay.defaults()
    }

    fn params_of(v: &[f32]) -> GlobalParams<'_> {
        GlobalParams::from_slices(v, v, v)
    }

    fn max() -> usize {
        Core::new(SR).max_delay()
    }

    #[test]
    fn the_defaults_are_a_synced_dotted_eighth_with_three_or_four_repeats() {
        let v = defaults();
        let p = block_params(params_of(&v), DEFAULT_TEMPO_BPM, SR, max());
        // 3/4 beat at 140 BPM: 0.75 · 60 / 140 · 48,000 = 15,428.57 samples, rounded.
        assert_eq!(p.delay, 15_429);
        assert_eq!(p.feedback_start, 0.35);
        assert_eq!(p.feedback_end, 0.35);
        assert_eq!((p.mix_start, p.mix_end), (1.0, 1.0));
        assert_eq!((p.low_cut_hz, p.high_cut_hz), (150.0, 6_000.0));
        assert!(!p.ping_pong);
    }

    #[test]
    fn sync_off_plays_the_time_in_milliseconds_whatever_the_tempo() {
        let mut v = defaults();
        v[DelayParam::Sync.index()] = 0.0;
        v[DelayParam::Time.index()] = 300.0;
        for bpm in [20.0, 140.0, 999.0] {
            assert_eq!(block_params(params_of(&v), bpm, SR, max()).delay, 14_400);
        }
        v[DelayParam::Time.index()] = 1.0;
        assert_eq!(block_params(params_of(&v), 140.0, SR, max()).delay, 48);
        v[DelayParam::Time.index()] = 2_000.0;
        assert_eq!(block_params(params_of(&v), 140.0, SR, max()).delay, 96_000);
    }

    #[test]
    fn every_synced_item_follows_the_tempo_and_a_bar_clamps_at_two_seconds_below_120_bpm() {
        let mut v = defaults();
        v[DelayParam::Sync.index()] = 1.0;
        for (i, item) in DELAY_BEATS.iter().enumerate() {
            let beats = Beats::from_value(i as f32).unwrap();
            assert_eq!(beats.key(), item.key);
            let (num, den) = beats.length();
            v[DelayParam::Beats.index()] = i as f32;
            for bpm in [20.0, 60.0, 100.0, 120.0, 128.0, 140.0, 174.0, 999.0] {
                let want = (f64::from(num) * 60.0 * 48_000.0 / (f64::from(den) * bpm))
                    .round()
                    .clamp(1.0, 96_000.0) as usize;
                let got = block_params(params_of(&v), bpm, SR, max()).delay;
                assert_eq!(got, want, "{} at {bpm} BPM", item.key);
                assert!((1..=96_000).contains(&got));
            }
        }
        // A bar: 2 s at 120 BPM exactly, clamped to 2 s below it, 1 s at 240.
        v[DelayParam::Beats.index()] = Beats::Bar.index() as f32;
        assert_eq!(block_params(params_of(&v), 120.0, SR, max()).delay, 96_000);
        assert_eq!(block_params(params_of(&v), 119.0, SR, max()).delay, 96_000);
        assert_eq!(block_params(params_of(&v), 20.0, SR, max()).delay, 96_000);
        assert_eq!(block_params(params_of(&v), 240.0, SR, max()).delay, 48_000);
    }

    #[test]
    fn the_list_is_the_shared_beats_list_up_to_a_bar() {
        assert_eq!(DELAY_BEATS.len(), Beats::Bar.index() + 1);
        for (i, item) in DELAY_BEATS.iter().enumerate() {
            assert_eq!(Beats::from_value(i as f32).unwrap().key(), item.key);
        }
    }

    #[test]
    fn percent_and_hz_go_to_the_core_in_its_units_and_the_ramp_runs_from_prev_to_cur() {
        let v = defaults();
        let mut cur = defaults();
        cur[DelayParam::Feedback.index()] = 80.0;
        cur[DelayParam::Mix.index()] = 40.0;
        cur[DelayParam::LowCut.index()] = 300.0;
        cur[DelayParam::HighCut.index()] = 3_000.0;
        cur[DelayParam::PingPong.index()] = 1.0;
        let base = cur.clone();
        let p = block_params(GlobalParams::from_slices(&base, &cur, &v), 140.0, SR, max());
        assert_eq!((p.feedback_start, p.feedback_end), (0.35, 0.8));
        assert_eq!((p.mix_start, p.mix_end), (1.0, 0.4));
        assert_eq!((p.low_cut_hz, p.high_cut_hz), (300.0, 3_000.0));
        assert!(p.ping_pong);
    }

    #[test]
    fn the_whole_catalogue_range_is_inside_the_cores() {
        for &param in DelayParam::ALL {
            let spec = param.spec();
            for x in [spec.min(), spec.default, spec.max()] {
                let mut v = defaults();
                v[param.index()] = x;
                let p = block_params(params_of(&v), 140.0, SR, max());
                assert!((1..=96_000).contains(&p.delay), "{}", spec.key);
                assert!((0.0..=1.0).contains(&p.feedback_end), "{}", spec.key);
                assert!((0.0..=1.0).contains(&p.mix_end), "{}", spec.key);
            }
        }
    }

    /// Feed `blocks` of segments through the module with `input` at `at` frames.
    fn run(
        d: &mut Delay,
        v: &[f32],
        bpm_of: impl Fn(usize) -> f64,
        input: &[(usize, f32)],
        segments: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let (mut ol, mut or) = (vec![0.0; segments * SEG], vec![0.0; segments * SEG]);
        for &(at, x) in input {
            ol[at] = x;
            or[at] = x;
        }
        for k in 0..segments {
            let l: &mut [f32; SEG] = (&mut ol[k * SEG..(k + 1) * SEG]).try_into().unwrap();
            let r: &mut [f32; SEG] = (&mut or[k * SEG..(k + 1) * SEG]).try_into().unwrap();
            d.process(&mut GlobalCx {
                tempo_bpm: bpm_of(k),
                params: params_of(v),
                left: l,
                right: r,
            });
        }
        (ol, or)
    }

    /// `round(num·60·sr / (den·bpm))` for whole `sr` and `bpm`, by exact integer arithmetic (half up).
    fn exact(num: u64, den: u64, bpm: u64, sr: u64) -> u64 {
        (2 * num * 60 * sr + den * bpm) / (2 * den * bpm)
    }

    #[test]
    fn an_impulse_echoes_on_the_exact_sample_for_the_tempo_in_force() {
        let mut v = defaults();
        v[DelayParam::Beats.index()] = Beats::Eighth.index() as f32;
        v[DelayParam::Feedback.index()] = 50.0;
        v[DelayParam::LowCut.index()] = 20.0;
        v[DelayParam::HighCut.index()] = 20_000.0;
        for bpm in [90u64, 128, 140, 174] {
            let mut d = Delay::new(&v, &PrepareCfg::new(SR));
            let want = exact(1, 2, bpm, 48_000) as usize;
            let segs = (want * 4 / SEG) + 4;
            let (wl, _) = run(&mut d, &v, |_| bpm as f64, &[(7, 1.0)], segs);
            // `out = dry + echo`: take the dry impulse out.
            let mut echo = wl.clone();
            echo[7] -= 1.0;
            for k in 1..=3usize {
                let at = 7 + k * want;
                let top = echo[at - 20..at + 20]
                    .iter()
                    .fold(0.0f32, |m, x| m.max(x.abs()));
                let first = (at - 20..at + 20)
                    .find(|&i| echo[i].abs() > 0.1 * top)
                    .unwrap();
                assert_eq!(first, at, "echo {k} at {bpm} BPM: want {want}");
            }
            assert!(echo[..7 + want].iter().all(|x| *x == 0.0));
        }
    }

    #[test]
    fn a_tempo_change_sets_the_delay_of_everything_still_in_the_ring() {
        let mut v = defaults();
        v[DelayParam::Beats.index()] = Beats::Quarter.index() as f32;
        v[DelayParam::Feedback.index()] = 0.0;
        v[DelayParam::LowCut.index()] = 20.0;
        v[DelayParam::HighCut.index()] = 20_000.0;
        let mut d = Delay::new(&v, &PrepareCfg::new(SR));
        let d140 = exact(1, 1, 140, 48_000) as usize; // 20,571
        let d100 = exact(1, 1, 100, 48_000) as usize; // 28,800
        let change_seg = 3_000; // the tempo changes at frame 96,000
        let change_frame = change_seg * SEG;
        // One impulse 5,000 frames before the change, one 3,200 after it (the 960-frame fade is over by then).
        let (before, after) = (change_frame - 5_000, change_frame + 3_200);
        let (wl, _) = run(
            &mut d,
            &v,
            |k| if k < change_seg { 140.0 } else { 100.0 },
            &[(before, 1.0), (after, 1.0)],
            change_seg + 2_500,
        );
        let mut echo = wl.clone();
        echo[before] -= 1.0;
        echo[after] -= 1.0;
        let onset = |at: usize| {
            let top = echo[at - 50..at + 50]
                .iter()
                .fold(0.0f32, |m, x| m.max(x.abs()));
            (at - 50..at + 50).find(|&i| echo[i].abs() > 0.1 * top)
        };
        // Both are heard exactly one quarter note of the new tempo after they were written, wherever they were
        // when the tempo changed: the ring holds the signal, the tempo sets where it is read.
        assert_eq!(onset(before + d100), Some(before + d100));
        assert_eq!(onset(after + d100), Some(after + d100));
        // The old read position (before + d140 = 90,000 + 20,571 > the change) now plays nothing.
        assert!(
            echo[before + d140 - 50..before + d140 + 50]
                .iter()
                .all(|x| x.abs() < 1e-3)
        );
        assert!(echo.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn skipping_an_idle_module_equals_running_it() {
        let mut v = defaults();
        v[DelayParam::Feedback.index()] = 10.0;
        let burst: Vec<(usize, f32)> = (0..64).map(|i| (i, 0.5 - i as f32 * 0.01)).collect();
        // 12 s at 10 % feedback is long enough for the repeats to fall below the flush threshold and for the ring to be
        // rewritten with zeros: the module is idle.
        let mut a = Delay::new(&v, &PrepareCfg::new(SR));
        let (_, _) = run(&mut a, &v, |_| 140.0, &burst, 48_000 * 12 / SEG);
        assert!(a.is_silent());
        let mut b = a.clone();
        // `a` runs 100 silent segments, `b` skips them; then both get the same sound.
        let (_, _) = run(&mut a, &v, |_| 140.0, &[], 100);
        assert!(a.is_silent());
        let (la, ra) = run(&mut a, &v, |_| 99.0, &[(3, 0.7)], 40);
        let (lb, rb) = run(&mut b, &v, |_| 99.0, &[(3, 0.7)], 40);
        assert_eq!(la, lb);
        assert_eq!(ra, rb);
    }

    #[test]
    fn copy_and_take_carry_the_whole_state() {
        let v = defaults();
        let mut a = Delay::new(&v, &PrepareCfg::new(SR));
        let (_, _) = run(&mut a, &v, |_| 140.0, &[(0, 1.0), (40, -0.5)], 400);
        let mut copied = Delay::new(&v, &PrepareCfg::new(SR));
        copied.copy_state_from(&a);
        let mut taken = Delay::new(&v, &PrepareCfg::new(SR));
        let mut from = a.clone();
        taken.take_state_from(&mut from);
        let want = run(&mut a, &v, |_| 140.0, &[(5, 0.25)], 600);
        assert_eq!(run(&mut copied, &v, |_| 140.0, &[(5, 0.25)], 600), want);
        assert_eq!(run(&mut taken, &v, |_| 140.0, &[(5, 0.25)], 600), want);
    }

    #[test]
    fn it_allocates_two_seconds_of_stereo_ring_from_the_sample_rate_alone() {
        let v = defaults();
        let d = Delay::new(&v, &PrepareCfg::new(48_000.0));
        assert_eq!(d.core().allocated_bytes(), 768_000);
        assert_eq!(d.core().max_delay(), 96_000);
        // Parameter values do not size anything: a short slice is fine.
        let d = Delay::new(&[], &PrepareCfg::new(44_100.0));
        assert_eq!(d.core().max_delay(), 88_200);
        assert!(d.is_silent());
        assert_eq!(d.subnormals(), 0);
    }
}
