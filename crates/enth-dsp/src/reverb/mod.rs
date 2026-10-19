//! Reverb core: a port of the Signalsmith Basics FDN reverb (`reverb.h`) and its delay, mix and filter helpers
//! (P1a lane FX1). Long form, provenance and the golden plan: `docs/dsp/reverb.md`.
//!
//! # What it is
//!
//! A stereo reverb of 8 feedback channels. The input goes through a diffuser (two delay stages with Hadamard mixes)
//! into a feedback loop of eight detuned, damped delays with a Householder matrix; early reflections and two more
//! delay stages follow, then a low-cut and a high-cut. Parameters are in real units (see [`ReverbParams`]).
//!
//! # Contract
//!
//! - **All allocation is in [`Reverb::prepare`]** (about 1.64 MB at 48 kHz, 3.3 MB at 96 kHz). [`Reverb::process`],
//!   [`Reverb::set_params`] and [`Reverb::reset`] never allocate, lock or panic.
//! - **Zero latency.** The wet path has no look-ahead; [`Reverb::latency_samples`] is 0.
//! - **Deterministic, and bit-exact with upstream's C++** (built without FMA contraction and fast-math) on every
//!   platform for as long as no value in the network falls below `1e-15` (-300 dB): transcendentals go through
//!   `libm`, there is no RNG (upstream's `mt19937` draws are a constant table, `table.rs`) and nothing depends on
//!   FTZ/DAZ. Below `1e-15` the flush in code takes over from upstream's hardware flush at `1.2e-38`; see
//!   `docs/dsp/reverb.md` for the deviation and its measured size.
//! - **The caller's chunking is an input while parameters move.** Upstream advances its 20 ms parameter cross-fade
//!   and recomputes its filters once per `process` call. With steady parameters the output was bit-identical for
//!   blocks of 32 to 480 frames (measured; only the rounding of the detune LFO's phase wrap could differ), but
//!   when a parameter changes, the fade starts at the next block boundary, so 32-frame and 64-frame renders
//!   differ by up to -44 dB re peak (measured). The engine always calls with its fixed 32-frame segment, which
//!   makes the render independent of the host block size; do the same.
//! - **No smoothing needed for `room` and `decay`, and they are not modulatable.** A change of any parameter starts
//!   a 20 ms A/B cross-fade inside the reverb (the delay offsets are redrawn and the taps cross-fade; gains
//!   interpolate linearly), so a second smoother in front would only add a second ramp. Parameters moved
//!   continuously are applied in 20 ms steps: a new fade starts when the previous one has finished. **The low-cut,
//!   high-cut and damping filters are the exception**: they follow [`Reverb::set_params`] at the next block
//!   with no fade (as upstream), so a large jump of a cut frequency steps the output (see `docs/dsp/reverb.md`).
//!
//! # Provenance
//!
//! `reverb.h` from <https://github.com/Signalsmith-Audio/basics> at `369e906e03760ec219d07b06fa06dc4fd099f377`
//! (MIT, Copyright 2022+ Signalsmith Audio Ltd. / Geraint Luff); `delay.h`, `mix.h` and `filters.h` from
//! <https://github.com/Signalsmith-Audio/dsp> at `ca390d28c41ea5108a528e2e81faa4088f2f8ac8` (MIT, Copyright (c) 2021
//! Geraint Luff / Signalsmith Audio Ltd.); the parameter fade is re-implemented from `stfx/stfx-library.h` of the
//! basics repository (Boost Software License 1.0, Copyright 2021-2022 Geraint Luff / Signalsmith Audio Ltd). Full
//! licence texts and the changes from upstream are in `THIRD_PARTY_NOTICES.md`.
//!
//! Evaluation order follows the C++ exactly, including where it mixes `double` and `float`: parameters, smoothing,
//! the LFO phase and the filter design are `f64`; the audio path and the coefficients are `f32`.

use core::f64::consts::PI;

use crate::util::flush::flush;

mod biquad;
mod delay;
mod mix;
mod table;
#[cfg(test)]
mod tests;

use biquad::Biquad;
use delay::{Array, CHANNELS, Lagrange7, MultiDelay};
use mix::{StereoMixer, hadamard, householder};

/// The lowest sample rate [`Reverb::prepare`] accepts (lower values are raised to it).
pub const MIN_SAMPLE_RATE: f32 = 8_000.0;
/// The highest sample rate [`Reverb::prepare`] accepts (higher values are lowered to it).
pub const MAX_SAMPLE_RATE: f32 = 768_000.0;

/// Longest room in milliseconds. Sizes the buffers; upstream's `maxRoomMs`.
const MAX_ROOM_MS: f64 = 200.0;
/// Depth of the detune modulation in milliseconds; upstream's `detuneDepthMs`.
const DETUNE_DEPTH_MS: f64 = 2.0;
/// Length of the parameter cross-fade in milliseconds; upstream's `paramFadeMs()` default.
const FADE_MS: f64 = 20.0;

/// The reverb's parameters in real units. Out-of-range values are clamped by [`Reverb::set_params`] to
/// [`ReverbParams::MIN`]..[`ReverbParams::MAX`] (NaN counts as the minimum).
///
/// Ranges are upstream's UI ranges; the defaults are upstream's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// Dry (direct) signal level in percent of unity gain: 100 is 0 dB, 0 is silent. Range 0 to 400 (+12 dB).
    pub dry_pct: f64,
    /// Reverb tail level in percent of unity gain: 100 is 0 dB, 0 is silent. Range 0 to 400 (+12 dB).
    pub wet_pct: f64,
    /// Room size in milliseconds (one millisecond is about one foot): the longest delay of the network. 10 to
    /// 200. Changing it redraws the delay lengths and cross-fades over 20 ms; it allocates nothing.
    pub room_ms: f64,
    /// RT20: the time the tail takes to fall by 20 dB, in seconds (three times that is the usual RT60). 0.01 to 30.
    pub rt20_s: f64,
    /// Early reflections level in percent. 0 to 250.
    pub early_pct: f64,
    /// Detune depth inside the feedback loop: a unitless rate, about cents per loop at a one-second room. 0 to 50.
    pub detune: f64,
    /// High-pass corner in Hz. 10 to 500.
    pub low_cut_hz: f64,
    /// Low-pass corner in Hz. 1000 to 20000.
    pub high_cut_hz: f64,
    /// How fast low frequencies die in the tail relative to the mid band: a power applied to the loop gain
    /// below the low-cut. 1 (no extra damping) to 10.
    pub low_damp_rate: f64,
    /// Same for high frequencies above half the high-cut. 1 to 10.
    pub high_damp_rate: f64,
}

impl ReverbParams {
    /// Lower bound of every field.
    pub const MIN: Self = Self {
        dry_pct: 0.0,
        wet_pct: 0.0,
        room_ms: 10.0,
        rt20_s: 0.01,
        early_pct: 0.0,
        detune: 0.0,
        low_cut_hz: 10.0,
        high_cut_hz: 1_000.0,
        low_damp_rate: 1.0,
        high_damp_rate: 1.0,
    };

    /// Upper bound of every field.
    pub const MAX: Self = Self {
        dry_pct: 400.0,
        wet_pct: 400.0,
        room_ms: MAX_ROOM_MS,
        rt20_s: 30.0,
        early_pct: 250.0,
        detune: 50.0,
        low_cut_hz: 500.0,
        high_cut_hz: 20_000.0,
        low_damp_rate: 10.0,
        high_damp_rate: 10.0,
    };

    /// Every field limited to [`Self::MIN`]..[`Self::MAX`]; NaN becomes the minimum.
    #[must_use]
    pub fn clamped(&self) -> Self {
        let c = |v: f64, lo: f64, hi: f64| v.max(lo).min(hi);
        let (lo, hi) = (Self::MIN, Self::MAX);
        Self {
            dry_pct: c(self.dry_pct, lo.dry_pct, hi.dry_pct),
            wet_pct: c(self.wet_pct, lo.wet_pct, hi.wet_pct),
            room_ms: c(self.room_ms, lo.room_ms, hi.room_ms),
            rt20_s: c(self.rt20_s, lo.rt20_s, hi.rt20_s),
            early_pct: c(self.early_pct, lo.early_pct, hi.early_pct),
            detune: c(self.detune, lo.detune, hi.detune),
            low_cut_hz: c(self.low_cut_hz, lo.low_cut_hz, hi.low_cut_hz),
            high_cut_hz: c(self.high_cut_hz, lo.high_cut_hz, hi.high_cut_hz),
            low_damp_rate: c(self.low_damp_rate, lo.low_damp_rate, hi.low_damp_rate),
            high_damp_rate: c(self.high_damp_rate, lo.high_damp_rate, hi.high_damp_rate),
        }
    }

    /// The values in the order of the internal slots (gains as ratios).
    fn slots(&self) -> [f64; N_PARAMS] {
        let mut v = [0.0; N_PARAMS];
        v[DRY] = self.dry_pct / 100.0;
        v[WET] = self.wet_pct / 100.0;
        v[ROOM] = self.room_ms;
        v[RT20] = self.rt20_s;
        v[EARLY] = self.early_pct / 100.0;
        v[DETUNE] = self.detune;
        v[LOW_CUT] = self.low_cut_hz;
        v[LOW_DAMP] = self.low_damp_rate;
        v[HIGH_CUT] = self.high_cut_hz;
        v[HIGH_DAMP] = self.high_damp_rate;
        v
    }
}

impl Default for ReverbParams {
    /// Upstream's defaults: dry 100 % (0 dB), wet 50 % (-6 dB), room 80 ms, RT20 1 s, early 150 %, detune 2,
    /// low cut 80 Hz, high cut 12 kHz, damp rates 1.5 and 2.5.
    fn default() -> Self {
        Self {
            dry_pct: 100.0,
            wet_pct: 50.0,
            room_ms: 80.0,
            rt20_s: 1.0,
            early_pct: 150.0,
            detune: 2.0,
            low_cut_hz: 80.0,
            high_cut_hz: 12_000.0,
            low_damp_rate: 1.5,
            high_damp_rate: 2.5,
        }
    }
}

// Internal parameter slots.
const N_PARAMS: usize = 10;
const DRY: usize = 0;
const WET: usize = 1;
const ROOM: usize = 2;
const RT20: usize = 3;
const EARLY: usize = 4;
const DETUNE: usize = 5;
const LOW_CUT: usize = 6;
const LOW_DAMP: usize = 7;
const HIGH_CUT: usize = 8;
const HIGH_DAMP: usize = 9;

/// One parameter with the A/B fade bookkeeping of upstream's `LibraryParam`: `current` is what the user set,
/// `to` the value the running fade ends at, `from` the one it started at.
#[derive(Clone, Copy)]
struct Param {
    current: f64,
    from: f64,
    to: f64,
}

impl Param {
    const fn new(v: f64) -> Self {
        Self {
            current: v,
            from: v,
            to: v,
        }
    }

    /// `_libStartFade`: shift the values along to start a new fade; true if it actually changes.
    fn start_fade(&mut self) -> bool {
        self.from = self.to;
        self.to = self.current;
        self.to != self.from
    }

    fn snap(&mut self) {
        self.from = self.current;
        self.to = self.current;
    }
}

/// A linear ramp over one block: `at(i) = start + i * step` (upstream's `LinearSegment`).
#[derive(Clone, Copy)]
struct Segment {
    start: f64,
    step: f64,
}

impl Segment {
    /// `Block::smooth(from, to)`: the part of the `from -> to` fade this block covers.
    fn new(from: f64, to: f64, fade_start: f64, fade_step: f64) -> Self {
        let diff = to - from;
        Self {
            start: from + diff * fade_start,
            step: diff * fade_step,
        }
    }

    #[inline(always)]
    fn at(self, i: usize) -> f64 {
        self.start + i as f64 * self.step
    }
}

/// Everything `process` derives once per call.
struct BlockPlan {
    fading: bool,
    fade_start: f64,
    fade_step: f64,
    fade_end: f64,
    dry: Segment,
    wet: Segment,
    decay: Segment,
    input: Segment,
    early: Segment,
    lfo_rate: f64,
}

/// The inputs of the filter design; equal inputs give equal coefficients, so an unchanged key skips the redesign.
type FilterKey = [u64; 5];

/// The stereo FDN reverb. Build it with [`Reverb::prepare`].
pub struct Reverb {
    sample_rate: f64,
    params: [Param; N_PARAMS],
    just_had_reset: bool,
    fade_ratio: f64,
    detune_depth_samples: f64,
    lfo_phase: f64,
    mixer: StereoMixer,
    lagrange: Lagrange7,
    delay1: MultiDelay,
    delay2: MultiDelay,
    delay3: MultiDelay,
    delay4: MultiDelay,
    delay_feedback: MultiDelay,
    delay_early: MultiDelay,
    low_cut: [Biquad; 2],
    high_cut: [Biquad; 2],
    low_damp: [Biquad; CHANNELS],
    high_damp: [Biquad; CHANNELS],
    filter_key: Option<FilterKey>,
}

impl core::fmt::Debug for Reverb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reverb")
            .field("sample_rate", &self.sample_rate)
            .field("allocated_bytes", &self.allocated_bytes())
            .finish_non_exhaustive()
    }
}

impl Reverb {
    /// Build a reverb for `sample_rate` (clamped to [`MIN_SAMPLE_RATE`]..[`MAX_SAMPLE_RATE`]; NaN becomes 48 kHz),
    /// with [`ReverbParams::default`] set. **The only place that allocates**; the buffers depend on the sample
    /// rate alone, never on parameter values.
    #[must_use]
    pub fn prepare(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_nan() {
            48_000.0
        } else {
            sample_rate.clamp(MIN_SAMPLE_RATE, MAX_SAMPLE_RATE)
        };
        let sr = f64::from(sample_rate);
        let detune_depth_samples = DETUNE_DEPTH_MS * 0.001 * sr;
        let max_room_samples = MAX_ROOM_MS * 0.001 * sr;

        let defaults = ReverbParams::default().slots();
        let mut params = [Param::new(0.0); N_PARAMS];
        for (p, v) in params.iter_mut().zip(defaults) {
            *p = Param::new(v);
        }
        let mut reverb = Self {
            sample_rate: sr,
            params,
            just_had_reset: true,
            fade_ratio: 0.0,
            detune_depth_samples,
            lfo_phase: 0.0,
            mixer: StereoMixer::new(),
            lagrange: Lagrange7::new(),
            delay1: MultiDelay::new(max_room_samples, 0.125),
            delay2: MultiDelay::new(max_room_samples, 1.0),
            delay3: MultiDelay::new(max_room_samples, 0.5),
            delay4: MultiDelay::new(max_room_samples, 0.25),
            delay_feedback: MultiDelay::new(max_room_samples * 1.6 + detune_depth_samples, 1.0),
            delay_early: MultiDelay::new(max_room_samples, 0.25),
            low_cut: [Biquad::NEW; 2],
            high_cut: [Biquad::NEW; 2],
            low_damp: [Biquad::NEW; CHANNELS],
            high_damp: [Biquad::NEW; CHANNELS],
            filter_key: None,
        };
        reverb.reset();
        reverb
    }

    /// The sample rate in Hz (after clamping).
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate as f32
    }

    /// Heap memory held by the delay lines, in bytes (1,638,400 at 48 kHz). Fixed after [`Reverb::prepare`].
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        let floats = self.delays().iter().map(|d| d.len()).sum::<usize>();
        floats * core::mem::size_of::<f32>()
    }

    /// Latency in samples: always 0.
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        0
    }

    /// How long the tail keeps going after the input stops, in samples: `round(sample_rate * rt20 * 3)`, the
    /// time to fall by 60 dB at the current RT20.
    #[must_use]
    pub fn tail_samples(&self) -> usize {
        libm::round(self.sample_rate * self.params[RT20].current * 3.0) as usize
    }

    /// Clear the delay lines and filters and restart the detune LFO, so the next block starts from silence.
    /// Parameters are kept, and the next block applies them at once with no cross-fade (an unfinished fade is
    /// cancelled). Upstream's `reset()` forgets `delay4`, which leaves a stale tail in it; this one clears all six.
    pub fn reset(&mut self) {
        for d in [
            &mut self.delay1,
            &mut self.delay2,
            &mut self.delay3,
            &mut self.delay4,
            &mut self.delay_feedback,
            &mut self.delay_early,
        ] {
            d.reset();
        }
        let filters = self
            .low_cut
            .iter_mut()
            .chain(&mut self.high_cut)
            .chain(&mut self.low_damp)
            .chain(&mut self.high_damp);
        for f in filters {
            f.reset();
        }
        for p in &mut self.params {
            p.snap();
        }
        self.fade_ratio = 0.0;
        self.lfo_phase = 0.0;
        self.just_had_reset = true;
    }

    /// Set the parameters (clamped, see [`ReverbParams`]). They take effect in the next [`Reverb::process`]
    /// call, through the 20 ms cross-fade, except before the first call after [`Reverb::prepare`] or
    /// [`Reverb::reset`], where they apply at once. Cheap, and safe to call every segment: nothing happens unless a
    /// value changed.
    pub fn set_params(&mut self, params: &ReverbParams) {
        let slots = params.clamped().slots();
        for (p, v) in self.params.iter_mut().zip(slots) {
            p.current = v;
            if self.just_had_reset {
                p.snap();
            }
        }
    }

    /// Process a block. `in_l`, `in_r` are read, `out_l`, `out_r` written: dry plus wet, `out = in * dry + tail *
    /// wet`. The four slices should have the same length; if not, the shortest sets the frame count. Never
    /// allocates. The output depends on the block length, see the module docs.
    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let n = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        if n == 0 {
            return;
        }
        let (in_l, in_r) = (&in_l[..n], &in_r[..n]);
        let (out_l, out_r) = (&mut out_l[..n], &mut out_r[..n]);
        let plan = self.begin_block(n);
        for i in 0..n {
            let out = self.tick(&plan, i, [in_l[i], in_r[i]]);
            out_l[i] = out[0];
            out_r[i] = out[1];
        }
        self.end_block(&plan);
    }

    /// Like [`Reverb::process`], with the output replacing the input in `left` and `right`. Gives the same bits.
    pub fn process_in_place(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        if n == 0 {
            return;
        }
        let (left, right) = (&mut left[..n], &mut right[..n]);
        let plan = self.begin_block(n);
        for i in 0..n {
            let out = self.tick(&plan, i, [left[i], right[i]]);
            left[i] = out[0];
            right[i] = out[1];
        }
        self.end_block(&plan);
    }

    fn delays(&self) -> [&MultiDelay; 6] {
        [
            &self.delay1,
            &self.delay2,
            &self.delay3,
            &self.delay4,
            &self.delay_feedback,
            &self.delay_early,
        ]
    }

    /// The block-level half of `processSTFX` and of `LibraryEffect::process`: advance the fade, redraw the delay
    /// offsets when a fade starts, derive this block's ramps, redesign the filters.
    fn begin_block(&mut self, n: usize) -> BlockPlan {
        let fade_samples = FADE_MS * 0.001 * self.sample_rate;
        // Fade position at the end of the block, and the step: finish exactly if the fade ends in this block.
        let mut fade_end = self.fade_ratio + n as f64 / fade_samples;
        let mut fade_step = if fade_end >= 1.0 {
            (1.0 - self.fade_ratio) / n as f64
        } else {
            1.0 / fade_samples
        };
        // At the start of a new fade, move every parameter along. None changing: no fade.
        if self.fade_ratio == 0.0 {
            let mut needs_fade = false;
            for p in &mut self.params {
                if p.start_fade() {
                    needs_fade = true;
                }
            }
            if !needs_fade {
                fade_step = 0.0;
                fade_end = 0.0;
            }
        }
        let fade_start = self.fade_ratio;
        let fading = fade_step != 0.0;

        // `setupFade`: on the first block after a reset, and when a fade starts.
        let room_to_samples = self.params[ROOM].to * 0.001 * self.sample_rate;
        if self.just_had_reset {
            self.update_delays(room_to_samples);
        }
        if fading && fade_start == 0.0 {
            self.update_delays(room_to_samples);
        }

        let p = &self.params;
        let dry = Segment::new(p[DRY].from, p[DRY].to, fade_start, fade_step);
        let wet = Segment::new(p[WET].from, p[WET].to, fade_start, fade_step);
        let decay_from = db_to_gain(f64::from(decay_db(
            p[RT20].from as f32,
            p[ROOM].from as f32,
        )));
        let decay_to = db_to_gain(f64::from(decay_db(p[RT20].to as f32, p[ROOM].to as f32)));
        let decay = Segment::new(decay_from, decay_to, fade_start, fade_step);
        // Scale the input by the number of echoes expected in the first 100 ms.
        let two_scaling = f64::from(2.0f32 * StereoMixer::SCALING);
        let echoes = |gain: f64, room_ms: f64| {
            two_scaling * libm::sqrt((1.0 - gain) / (1.0 - libm::pow(gain, 100.0 / room_ms)))
        };
        let input = Segment::new(
            echoes(decay_from, p[ROOM].from),
            echoes(decay_to, p[ROOM].to),
            fade_start,
            fade_step,
        );
        // 0.35: tuned by ear upstream.
        let early = Segment::new(
            p[EARLY].from * 0.35,
            p[EARLY].to * 0.35,
            fade_start,
            fade_step,
        );

        self.update_filters(decay_to);

        // Detune LFO rate; "tuned by ear, assuming 3/8 channels are detuned".
        let detune_cents_per_loop =
            self.params[DETUNE].current * libm::sqrt(self.params[ROOM].current * 0.001);
        let lfo_rate = (detune_cents_per_loop * 0.0004) / self.detune_depth_samples;

        BlockPlan {
            fading,
            fade_start,
            fade_step,
            fade_end,
            dry,
            wet,
            decay,
            input,
            early,
            lfo_rate,
        }
    }

    fn end_block(&mut self, plan: &BlockPlan) {
        self.lfo_phase -= libm::floor(self.lfo_phase);
        self.fade_ratio = if plan.fade_end >= 1.0 {
            0.0
        } else {
            plan.fade_end
        };
        self.just_had_reset = false;
    }

    /// `updateDelays`: redraw every stage's offsets for a room of `room_samples`.
    fn update_delays(&mut self, room_samples: f64) {
        use table::{DELAY1, DELAY2, DELAY3, DELAY4, DRAWS, EARLY as EARLY_SEED};
        self.delay1
            .update_lengths(&DRAWS[DELAY1], room_samples, false);
        self.delay2
            .update_lengths(&DRAWS[DELAY2], room_samples, true);
        self.delay3
            .update_lengths(&DRAWS[DELAY3], room_samples, true);
        self.delay4
            .update_lengths(&DRAWS[DELAY4], room_samples, true);
        self.delay_feedback.update_lengths_exponential(room_samples);
        self.delay_early
            .update_lengths(&DRAWS[EARLY_SEED], room_samples, true);
    }

    /// `updateFilters`. The 20 sections come in four groups of identical designs, and a design is a pure function
    /// of five numbers, so it is computed once per group and only when one of them changed. Same bits as
    /// upstream's 20 designs per block, and 7 % fewer instructions per frame at 32-frame blocks (measured).
    fn update_filters(&mut self, feedback_gain: f64) {
        let p = &self.params;
        let (low_cut, high_cut) = (p[LOW_CUT].current, p[HIGH_CUT].current);
        let (low_damp_rate, high_damp_rate) = (p[LOW_DAMP].current, p[HIGH_DAMP].current);
        let key = [
            low_cut.to_bits(),
            high_cut.to_bits(),
            low_damp_rate.to_bits(),
            high_damp_rate.to_bits(),
            feedback_gain.to_bits(),
        ];
        if self.filter_key == Some(key) {
            return;
        }
        self.filter_key = Some(key);

        let sr = self.sample_rate;
        let low_cut_k = biquad::highpass_q(low_cut / sr, 0.5);
        let high_cut_k = biquad::lowpass_q(high_cut / sr, 0.5);
        // `Sample lowDampHz = lowCutHz + 100;` is a float in C++, as are the gains.
        let low_damp_hz = (low_cut + 100.0) as f32;
        let high_damp_hz = (high_cut * 0.5) as f32;
        let low_damp_gain = libm::pow(feedback_gain, low_damp_rate).max(1e-3) as f32;
        let high_damp_gain = libm::pow(feedback_gain, high_damp_rate).max(1e-3) as f32;
        let low_damp_k =
            biquad::low_shelf_q(f64::from(low_damp_hz) / sr, f64::from(low_damp_gain), 0.5);
        let high_damp_k =
            biquad::high_shelf(f64::from(high_damp_hz) / sr, f64::from(high_damp_gain));

        for f in &mut self.low_cut {
            f.set_coeffs(low_cut_k);
        }
        for f in &mut self.high_cut {
            f.set_coeffs(high_cut_k);
        }
        for f in &mut self.low_damp {
            f.set_coeffs(low_damp_k);
        }
        for f in &mut self.high_damp {
            f.set_coeffs(high_damp_k);
        }
    }

    /// One frame of the network. `input` is the dry stereo pair.
    #[inline(always)]
    fn tick(&mut self, plan: &BlockPlan, i: usize, input: [f32; 2]) -> [f32; 2] {
        let input_gain = plan.input.at(i) as f32;
        let decay_gain = plan.decay.at(i) as f32;
        let early_gain = plan.early.at(i) as f32;

        // The wet path's input is flushed too, so a subnormal from an upstream module cannot reach the delays.
        let scaled = [flush(input[0] * input_gain), flush(input[1] * input_gain)];
        let mut samples = self.mixer.to_multi(scaled);

        let lfo_cos = libm::cos(self.lfo_phase * 2.0 * PI);
        let lfo_sin = libm::sin(self.lfo_phase * 2.0 * PI);
        let depth = self.detune_depth_samples;
        let lfo = [
            ((0.5 + lfo_cos * 0.5) * depth) as f32,
            ((0.5 + lfo_cos * -0.25 + lfo_sin * 0.43301270189) * depth) as f32,
            ((0.5 + lfo_cos * -0.25 + lfo_sin * -0.43301270189) * depth) as f32,
        ];
        self.lfo_phase += plan.lfo_rate;

        let fade = if plan.fading {
            Some((plan.fade_start + i as f64 * plan.fade_step) as f32)
        } else {
            None
        };
        let read = |d: &MultiDelay| match fade {
            None => d.read(),
            Some(f) => d.read_fade(f),
        };

        // Diffusion.
        self.delay1.write(&samples);
        samples = read(&self.delay1);
        hadamard(&mut samples);
        self.delay2.write(&samples);
        samples = read(&self.delay2);
        hadamard(&mut samples);

        // Feedback: detuned read, mix, damp, then write back rotated by three channels.
        let mut feedback = self.delay_feedback.read_detuned(&self.lagrange, &lfo, fade);
        householder(&mut feedback);
        for (c, v) in feedback.iter_mut().enumerate() {
            *v = self.high_damp[c].tick(self.low_damp[c].tick(*v));
        }
        let mut feedback_input: Array = [0.0; CHANNELS];
        for c in 0..CHANNELS {
            feedback_input[(c + 3) & 7] = flush(samples[c] + feedback[c] * decay_gain);
        }
        self.delay_feedback.write(&feedback_input);

        // Early reflections.
        self.delay_early.write(&samples);
        let mut early = read(&self.delay_early);
        hadamard(&mut early);
        for c in 0..CHANNELS {
            samples[c] = feedback[c] + early[c] * early_gain;
        }

        // Output diffusion.
        self.delay3.write(&samples);
        samples = read(&self.delay3);
        hadamard(&mut samples);
        self.delay4.write(&samples);
        samples = read(&self.delay4);

        let mut wet_out = self.mixer.to_stereo(&samples);
        for (c, v) in wet_out.iter_mut().enumerate() {
            *v = self.high_cut[c].tick(self.low_cut[c].tick(*v));
        }

        let dry_gain = plan.dry.at(i) as f32;
        let wet_gain = plan.wet.at(i) as f32;
        [
            input[0] * dry_gain + wet_out[0] * wet_gain,
            input[1] * dry_gain + wet_out[1] * wet_gain,
        ]
    }

    /// Test access: how many subnormal values sit in the delay lines and filter histories.
    #[cfg(test)]
    fn count_subnormals(&self) -> usize {
        let in_delays: usize = self
            .delays()
            .iter()
            .map(|d| d.samples().iter().filter(|x| x.is_subnormal()).count())
            .sum();
        let in_filters: usize = self
            .low_cut
            .iter()
            .chain(&self.high_cut)
            .chain(&self.low_damp)
            .chain(&self.high_damp)
            .map(|f| f.state().iter().filter(|x| x.is_subnormal()).count())
            .sum();
        in_delays + in_filters
    }
}

/// `dbToGain`: `10^(db / 20)`.
fn db_to_gain(db: f64) -> f64 {
    libm::pow(10.0, db * 0.05)
}

/// `getDecayDb`: the loop gain in dB that gives an RT20 of `rt20` seconds around a loop of `loop_ms`. `f32`
/// arithmetic, as upstream's `Sample`.
fn decay_db(rt20: f32, loop_ms: f32) -> f32 {
    let db_per_second = -20.0f32 / rt20;
    let seconds_per_loop = loop_ms * 0.001f32;
    db_per_second * seconds_per_loop
}
