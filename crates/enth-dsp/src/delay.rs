//! The stereo delay core of S2a lane DLY: two rings, a filtered and soft-clipped feedback loop, ping-pong and a click-free
//! change of the delay time. The `delay` module (`enth-engine`) turns the catalogue's values into [`BlockParams`];
//! `docs/dsp/delay.md` has the equations, the measured numbers and the sources.
//!
//! # Signal flow, per frame (stereo, interleaved ring)
//!
//! ```text
//! t   = (1 − w)·ring[n − D_old] + w·ring[n − D_new]       the taps; w is 0 outside a time change
//! e   = LP_high_cut( t − LP_low_cut(t) )                  the echo: each repeat passes both one-pole filters
//! r   = soft_clip(feedback · e)                           what returns; |r| ≤ 1 at every feedback
//! x_s = sanitised input                                   NaN → 0, clamped to ±1e6
//! in  = x_s + r                                           normal: each channel feeds itself
//!     = (L: mono(x_s) + r_R,  R: r_L)                     ping-pong: left → right → left, the input enters at left
//! ring[n] = flush(in)                                     one write per frame and channel
//! out = x + mix · e                                       the dry signal is never touched (mix 0 leaves the block alone)
//! ```
//!
//! - **The delay is a whole number of samples.** A synced time is `round(num·60·sr / (den·bpm))`, a time in
//!   milliseconds `round(ms·sr/1000)` ([`delay_for_beats`], [`delay_for_ms`]), both clamped to `1..=`[`Delay::max_delay`]
//!   (2 s). The taps read the ring with no interpolation, so an echo of an impulse lands on exactly the sample
//!   `n0 + k·D` and the loop adds no interpolation loss to a repeat.
//! - **A time change crossfades.** The module hands the core the target delay of each 32-frame block. When it differs
//!   from the delay being heard, a second tap at the new delay fades in over [`FADE_S`] (20 ms) with the weight
//!   `w = s²(3 − 2s)`, `s = (k + 1)/L` (a C¹ ramp that is exactly 1 on the last frame and complements the old tap's
//!   `1 − w`, so a fade never adds gain). While a fade runs, a newer target waits for it to end and the next one
//!   starts at the next block boundary: the delay follows a tempo ramp in steps of at most one fade, and never plays
//!   a read position that is not a whole sample.
//! - **The loop is bounded.** The return is `soft_clip(feedback·e)`: the identity up to 0.5, a C¹ quadratic knee
//!   up to 1.5, and exactly 1 beyond, so the ring never holds more than `|x| + 1` per channel whatever the feedback.
//!   Both filters have an impulse response of ℓ¹ norm ≤ 2, so `|e| ≤ 2·(|x|max + 1)` and 100 % repeats for ever
//!   without growing. At feedback up to 50 % and any signal up to full scale the clip is never reached, so the echoes
//!   of normal material are exactly linear.
//! - **Ping-pong is a cross-feed, blended** over [`PING_PONG_S`] (10 ms) when it is switched, so toggling it on a
//!   running echo does not click.
//! - **Idle.** Once every ring slot has been overwritten by exact zeros and both filters are at rest, the core is
//!   idle: a block of exact zeros is skipped (output zeros, nothing changes), and the first block with a sample
//!   wakes it with the delay, filters and ping-pong set straight to that block's values, so skipping equals running.
//!
//! Real-time and determinism: the rings are allocated in [`Delay::new`] (sized from the sample rate alone, 2 s per
//! channel) and nothing else ever allocates; transcendentals only through the pinned `libm` (one `tan` per filter
//! per block); no fused operations; every write into the loop is flushed, so a tail never reaches subnormals.

use crate::util::flush::flush;
use crate::util::math;

/// Frames per call: the engine's segment (`enth_engine::module::SEG`, which asserts it equals this).
pub const BLOCK: usize = 32;

/// The longest delay, in seconds: the length of each ring.
pub const MAX_DELAY_S: f64 = 2.0;

/// The shortest delay, in samples.
pub const MIN_DELAY: usize = 1;

/// The crossfade of a time change, in seconds.
pub const FADE_S: f64 = 0.020;

/// The blend when `ping_pong` is switched, in seconds.
pub const PING_PONG_S: f64 = 0.010;

/// The soft clip is the identity up to this magnitude.
pub const CLIP_KNEE: f32 = 0.5;

/// The soft clip's limit: what the loop can return at most.
pub const CLIP_LIMIT: f32 = 1.0;

/// Input beyond this magnitude (+120 dBFS) is clamped before it enters the loop; NaN becomes 0.
const INPUT_LIMIT: f32 = 1.0e6;

/// `(i + 1) / BLOCK`, exact in `f32`: the weight of a per-sample linear ramp that reaches its end value on the last
/// frame (the engine's `RAMP`).
const RAMP: [f32; BLOCK] = {
    let mut r = [0.0; BLOCK];
    let mut i = 0;
    while i < BLOCK {
        r[i] = (i + 1) as f32 / BLOCK as f32;
        i += 1;
    }
    r
};

/// What the module hands the core for one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockParams {
    /// The delay to play, in samples ([`delay_for_beats`], [`delay_for_ms`]); clamped into `1..=max_delay`.
    pub delay: usize,
    /// The feedback (0–1) on the block's first frame; it ramps linearly to [`feedback_end`](Self::feedback_end).
    pub feedback_start: f32,
    /// The feedback (0–1) on the block's last frame.
    pub feedback_end: f32,
    /// The echo level (0–1) on the first frame.
    pub mix_start: f32,
    /// The echo level (0–1) on the last frame. A block with both ends exactly 0 leaves its input untouched.
    pub mix_end: f32,
    /// The corner of the one-pole high-pass in the loop, in Hz (the end of the block; the filter ramps to it).
    pub low_cut_hz: f32,
    /// The corner of the one-pole low-pass in the loop, in Hz.
    pub high_cut_hz: f32,
    /// Cross the repeats between the channels.
    pub ping_pong: bool,
}

impl BlockParams {
    /// A block with a fixed delay and steady settings: `feedback` and `mix` in 0–1.
    #[must_use]
    pub const fn steady(delay: usize, feedback: f32, mix: f32) -> Self {
        Self {
            delay,
            feedback_start: feedback,
            feedback_end: feedback,
            mix_start: mix,
            mix_end: mix,
            low_cut_hz: 150.0,
            high_cut_hz: 6_000.0,
            ping_pong: false,
        }
    }
}

/// The delay in samples for `num/den` beats at `bpm` and `sample_rate`: `round(num·60·sr / (den·bpm))` in `f64`
/// (half rounds away from zero), clamped into `MIN_DELAY..=max`. A NaN or non-positive tempo gives `max`.
#[must_use]
pub fn delay_for_beats(num: u32, den: u32, bpm: f64, sample_rate: f32, max: usize) -> usize {
    let frames = f64::from(num) * 60.0 * f64::from(sample_rate) / (f64::from(den.max(1)) * bpm);
    to_samples(frames, max)
}

/// The delay in samples for `ms` milliseconds: `round(ms·sr/1000)` in `f64`, clamped into `MIN_DELAY..=max`.
#[must_use]
pub fn delay_for_ms(ms: f32, sample_rate: f32, max: usize) -> usize {
    to_samples(f64::from(ms) * f64::from(sample_rate) / 1000.0, max)
}

fn to_samples(frames: f64, max: usize) -> usize {
    if frames.is_nan() || frames > max as f64 {
        // A tempo that makes the time infinite (or NaN) is the longest delay; the clamp below keeps the rest in range.
        return max;
    }
    (frames.round().max(MIN_DELAY as f64) as usize).min(max)
}

/// The ring length (and longest delay) in frames for `sample_rate`: `round(2 s · sr)`.
#[must_use]
pub fn ring_frames(sample_rate: f32) -> usize {
    ((MAX_DELAY_S * f64::from(sample_rate)).round() as usize).max(MIN_DELAY + 1)
}

/// The one-pole coefficient `G = g/(1 + g)`, `g = tan(π·fc/sr)` (a trapezoidal one-pole low-pass is
/// `v = (x − s)·G, y = s + v, s = y + v`; its −3 dB corner is `fc`). `fc` is held below 0.49 of the sample rate.
#[must_use]
pub fn one_pole_g(fc_hz: f32, sample_rate: f32) -> f32 {
    let fc = f64::from(fc_hz).clamp(1.0, 0.49 * f64::from(sample_rate));
    let g = math::tan(core::f64::consts::PI * fc / f64::from(sample_rate));
    (g / (1.0 + g)) as f32
}

/// The loop's saturator: the identity up to [`CLIP_KNEE`], then `knee + t − t²/2` (`t = |u| − knee`), a C¹ curve that
/// reaches [`CLIP_LIMIT`] with slope 0 at `|u| = 1.5`, and exactly ±1 beyond. Odd, `+ − × ÷` only.
#[inline(always)]
#[must_use]
pub fn soft_clip(u: f32) -> f32 {
    let a = u.abs();
    if a <= CLIP_KNEE {
        u
    } else if a >= 1.5 {
        CLIP_LIMIT.copysign(u)
    } else {
        let t = a - CLIP_KNEE;
        (CLIP_KNEE + t - 0.5 * t * t).copysign(u)
    }
}

/// NaN becomes 0 and anything beyond ±[`INPUT_LIMIT`] is clamped: one such sample would otherwise live in the loop for
/// ever at 100 % feedback.
#[inline(always)]
fn sanitize(x: f32) -> f32 {
    if x.abs() <= INPUT_LIMIT {
        x
    } else if x.is_nan() {
        0.0
    } else {
        x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
    }
}

/// A fraction in 0–1; NaN is 0.
#[inline(always)]
fn unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// The frame `d` behind the write position `w` in a ring of `n` frames (`1 ≤ d ≤ n`; `d = n` is the slot about to be
/// overwritten).
#[inline(always)]
fn read_pos(w: usize, d: usize, n: usize) -> usize {
    let r = w + n - d;
    if r >= n { r - n } else { r }
}

/// Everything but the ring: a small `Copy` record, so a state copy cannot forget a field.
#[derive(Clone, Copy, Debug, PartialEq)]
struct State {
    /// The next frame to write.
    w: usize,
    /// The delay being heard (the outgoing tap during a fade).
    delay: usize,
    /// The incoming tap's delay (meaningful while `fading`).
    fade_to: usize,
    /// Frames of the fade played so far.
    fade_pos: u32,
    fading: bool,
    /// Low-cut (one-pole low-pass subtracted from the input) state, left and right.
    lc: [f32; 2],
    /// High-cut (one-pole low-pass) state, left and right.
    hc: [f32; 2],
    /// The two filters' coefficients at the end of the previous block (the next block ramps from them).
    g_lc: f32,
    g_hc: f32,
    /// The ping-pong blend, 0 (each channel feeds itself) to 1 (crossed).
    pp: f32,
    /// Consecutive frames whose ring writes were exact zeros.
    zero_run: u32,
    /// The ring is all zeros and the filters are at rest (see the module docs).
    idle: bool,
}

impl State {
    const FRESH: Self = Self {
        w: 0,
        delay: MIN_DELAY,
        fade_to: MIN_DELAY,
        fade_pos: 0,
        fading: false,
        lc: [0.0; 2],
        hc: [0.0; 2],
        g_lc: 0.0,
        g_hc: 0.0,
        pp: 0.0,
        zero_run: 0,
        idle: true,
    };
}

/// The stereo delay core. See the module docs.
#[derive(Clone, Debug)]
pub struct Delay {
    sample_rate: f32,
    /// Frames per ring: the longest delay.
    len: usize,
    fade_len: u32,
    /// How far the ping-pong blend moves per block.
    pp_step: f32,
    /// `2·len` samples, left and right interleaved.
    ring: Box<[f32]>,
    s: State,
}

impl Delay {
    /// A delay for `sample_rate`: allocates the two rings (`2·round(2 s·sr)` samples, 768,000 bytes at 48 kHz). Idle.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let len = ring_frames(sample_rate);
        Self {
            sample_rate,
            len,
            fade_len: ((FADE_S * f64::from(sample_rate)).round() as u32).max(2),
            pp_step: (BLOCK as f64 / (PING_PONG_S * f64::from(sample_rate))).min(1.0) as f32,
            ring: vec![0.0; 2 * len].into_boxed_slice(),
            s: State::FRESH,
        }
    }

    /// The sample rate the core was built for.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The longest delay in samples (the ring length).
    #[must_use]
    pub fn max_delay(&self) -> usize {
        self.len
    }

    /// Frames of a time-change crossfade.
    #[must_use]
    pub fn fade_frames(&self) -> u32 {
        self.fade_len
    }

    /// The bytes held by the rings.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.ring.len() * core::mem::size_of::<f32>()
    }

    /// The delay being heard, in samples (the outgoing one during a fade). Test and meter support.
    #[must_use]
    pub fn current_delay(&self) -> usize {
        self.s.delay
    }

    /// Whether a time-change crossfade is running.
    #[must_use]
    pub fn is_fading(&self) -> bool {
        self.s.fading
    }

    /// Forget everything: the rings are cleared (skipped if they already are) and the core is idle.
    pub fn reset(&mut self) {
        if !self.s.idle {
            self.ring.fill(0.0);
        }
        self.s = State::FRESH;
    }

    /// Whether processing a block of exact zeros would give exact zeros and change nothing.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.s.idle
    }

    /// Start running at `p`'s values with no glide: the delay, filters and ping-pong are the block's own.
    fn wake(&mut self, p: &BlockParams) {
        let s = &mut self.s;
        s.idle = false;
        s.zero_run = 0;
        s.delay = p.delay.clamp(MIN_DELAY, self.len);
        s.fading = false;
        s.fade_pos = 0;
        s.lc = [0.0; 2];
        s.hc = [0.0; 2];
        s.g_lc = one_pole_g(p.low_cut_hz, self.sample_rate);
        s.g_hc = one_pole_g(p.high_cut_hz, self.sample_rate);
        s.pp = if p.ping_pong { 1.0 } else { 0.0 };
    }

    /// Process one block in place: `left` and `right` hold the input and get `input + mix·echo`.
    pub fn process(&mut self, p: &BlockParams, left: &mut [f32; BLOCK], right: &mut [f32; BLOCK]) {
        if self.s.idle {
            if left.iter().all(|x| *x == 0.0) && right.iter().all(|x| *x == 0.0) {
                return;
            }
            self.wake(p);
        }
        let n = self.len;
        let mut s = self.s;

        let target = p.delay.clamp(MIN_DELAY, n);
        if !s.fading && target != s.delay {
            s.fading = true;
            s.fade_to = target;
            s.fade_pos = 0;
        }

        let g_lc1 = one_pole_g(p.low_cut_hz, self.sample_rate);
        let g_hc1 = one_pole_g(p.high_cut_hz, self.sample_rate);
        let (g_lc0, g_hc0) = (s.g_lc, s.g_hc);
        let (d_lc, d_hc) = (g_lc1 - g_lc0, g_hc1 - g_hc0);

        let goal = if p.ping_pong { 1.0 } else { 0.0 };
        let pp0 = s.pp;
        let pp1 = if pp0 < goal {
            (pp0 + self.pp_step).min(goal)
        } else {
            (pp0 - self.pp_step).max(goal)
        };
        let d_pp = pp1 - pp0;

        let (fb0, fb1) = (unit(p.feedback_start), unit(p.feedback_end));
        let (d_fb, m0) = (fb1 - fb0, unit(p.mix_start));
        let m1 = unit(p.mix_end);
        let d_m = m1 - m0;
        let write_out = m0 != 0.0 || m1 != 0.0;

        let fade_len = self.fade_len;
        let ring = &mut self.ring[..];
        let mut w = s.w;
        let mut nz = 0u32;

        for i in 0..BLOCK {
            let r = RAMP[i];

            // The taps.
            let a = read_pos(w, s.delay, n) * 2;
            let (mut tl, mut tr) = (ring[a], ring[a + 1]);
            if s.fading {
                let b = read_pos(w, s.fade_to, n) * 2;
                let x = (s.fade_pos + 1) as f32 / fade_len as f32;
                let wb = x * x * (3.0 - 2.0 * x);
                let wa = 1.0 - wb;
                tl = wa * tl + wb * ring[b];
                tr = wa * tr + wb * ring[b + 1];
                s.fade_pos += 1;
                if s.fade_pos >= fade_len {
                    s.delay = s.fade_to;
                    s.fading = false;
                }
            }

            // The echo: the low cut (the input minus a one-pole low-pass), then the high cut (a one-pole low-pass).
            let g = g_lc0 + d_lc * r;
            let v = (tl - s.lc[0]) * g;
            let lp = v + s.lc[0];
            s.lc[0] = lp + v;
            let yl = tl - lp;
            let v = (tr - s.lc[1]) * g;
            let lp = v + s.lc[1];
            s.lc[1] = lp + v;
            let yr = tr - lp;

            let g = g_hc0 + d_hc * r;
            let v = (yl - s.hc[0]) * g;
            let el = v + s.hc[0];
            s.hc[0] = el + v;
            let v = (yr - s.hc[1]) * g;
            let er = v + s.hc[1];
            s.hc[1] = er + v;

            // What returns, and what is written.
            let fb = fb0 + d_fb * r;
            let rl = soft_clip(fb * el);
            let rr = soft_clip(fb * er);
            let xl = sanitize(left[i]);
            let xr = sanitize(right[i]);
            let c = pp0 + d_pp * r;
            let (il, ir) = if c == 0.0 {
                (xl + rl, xr + rr)
            } else if c == 1.0 {
                (xl.midpoint(xr) + rr, rl)
            } else {
                let (nl, nr) = (xl + rl, xr + rr);
                let (pl, pr) = (xl.midpoint(xr) + rr, rl);
                ((1.0 - c) * nl + c * pl, (1.0 - c) * nr + c * pr)
            };
            let (wl, wr) = (flush(il), flush(ir));
            nz |= wl.to_bits() | wr.to_bits();
            ring[w * 2] = wl;
            ring[w * 2 + 1] = wr;
            w += 1;
            if w == n {
                w = 0;
            }

            // The output.
            if write_out {
                let m = m0 + d_m * r;
                left[i] += m * el;
                right[i] += m * er;
            }
        }

        s.w = w;
        s.g_lc = g_lc1;
        s.g_hc = g_hc1;
        s.pp = pp1;
        s.lc = [flush(s.lc[0]), flush(s.lc[1])];
        s.hc = [flush(s.hc[0]), flush(s.hc[1])];
        s.zero_run = if nz == 0 {
            s.zero_run.saturating_add(BLOCK as u32)
        } else {
            0
        };
        if s.zero_run as usize >= n && !s.fading && s.lc == [0.0; 2] && s.hc == [0.0; 2] {
            s.idle = true;
        }
        self.s = s;
    }

    /// Whether `other` has the same rings (same sample rate), so state can pass between them.
    fn same_shape(&self, other: &Self) -> bool {
        self.len == other.len && self.fade_len == other.fade_len
    }

    /// Copy all state from `other` (a plan swap that crossfades: both keep running). Returns `false`, changing
    /// nothing, if the two were built for different sample rates. An idle `other` costs no ring copy.
    pub fn copy_state_from(&mut self, other: &Self) -> bool {
        if !self.same_shape(other) {
            return false;
        }
        if other.s.idle {
            if !self.s.idle {
                self.ring.fill(0.0);
            }
        } else {
            self.ring.copy_from_slice(&other.ring);
        }
        self.s = other.s;
        true
    }

    /// Take all state from `other`, which is never run again (an instant swap): the rings are exchanged, an O(1)
    /// move, and `other` is left holding this core's old rings. Returns `false`, changing nothing, if the two were
    /// built for different sample rates.
    pub fn take_state_from(&mut self, other: &mut Self) -> bool {
        if !self.same_shape(other) {
            return false;
        }
        core::mem::swap(&mut self.ring, &mut other.ring);
        self.s = other.s;
        true
    }

    /// Subnormal values in the rings and filter states (test support).
    #[must_use]
    pub fn count_subnormals(&self) -> usize {
        let state = self
            .s
            .lc
            .iter()
            .chain(&self.s.hc)
            .filter(|x| x.is_subnormal())
            .count();
        state + self.ring.iter().filter(|x| x.is_subnormal()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Run `core` over `left`/`right` in place, one `BLOCK` at a time, with the parameters `p(block_index)` returns.
    fn run(
        core: &mut Delay,
        left: &mut [f32],
        right: &mut [f32],
        p: impl Fn(usize) -> BlockParams,
    ) {
        let (lc, lrest) = left.as_chunks_mut::<BLOCK>();
        let (rc, rrest) = right.as_chunks_mut::<BLOCK>();
        assert!(lrest.is_empty() && rrest.is_empty(), "whole blocks only");
        for (k, (l, r)) in lc.iter_mut().zip(rc.iter_mut()).enumerate() {
            core.process(&p(k), l, r);
        }
    }

    /// The wet signal of a run: the output minus the dry input.
    fn wet(
        core: &mut Delay,
        input: &[f32],
        input_r: &[f32],
        p: impl Fn(usize) -> BlockParams,
    ) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (input.to_vec(), input_r.to_vec());
        run(core, &mut l, &mut r, p);
        let sub = |o: Vec<f32>, i: &[f32]| o.iter().zip(i).map(|(a, b)| a - b).collect();
        (sub(l, input), sub(r, input_r))
    }

    fn impulse(len: usize, at: usize) -> Vec<f32> {
        let mut v = vec![0.0; len];
        v[at] = 1.0;
        v
    }

    /// The pass band of the loop's filters, wide open: 20 Hz and 20 kHz.
    fn open(delay: usize, fb: f32) -> BlockParams {
        BlockParams {
            low_cut_hz: 20.0,
            high_cut_hz: 20_000.0,
            ..BlockParams::steady(delay, fb, 1.0)
        }
    }

    /// `round(num·60·sr / (den·bpm))` by exact integer arithmetic (half up), for whole `sr` and `bpm`.
    fn reference_delay(num: u64, den: u64, bpm: u64, sr: u64) -> u64 {
        (2 * num * 60 * sr + den * bpm) / (2 * den * bpm)
    }

    #[test]
    fn a_synced_delay_is_the_rounded_time_in_samples() {
        // (num, den) of every synced time the delay offers (the shared `Beats` list up to a bar).
        let beats: [(u32, u32); 18] = [
            (1, 16),
            (1, 12),
            (1, 8),
            (1, 6),
            (3, 16),
            (1, 4),
            (1, 3),
            (3, 8),
            (1, 2),
            (2, 3),
            (3, 4),
            (1, 1),
            (4, 3),
            (3, 2),
            (2, 1),
            (8, 3),
            (3, 1),
            (4, 1),
        ];
        for sr in [44_100u64, 48_000, 96_000] {
            let max = ring_frames(sr as f32);
            assert_eq!(max as u64, 2 * sr);
            for bpm in (20..=999u64).step_by(7).chain([140, 128, 174, 999, 20]) {
                for (num, den) in beats {
                    let want = reference_delay(u64::from(num), u64::from(den), bpm, sr)
                        .clamp(1, max as u64);
                    let got = delay_for_beats(num, den, bpm as f64, sr as f32, max);
                    assert_eq!(got as u64, want, "{num}/{den} at {bpm} BPM, {sr} Hz");
                }
            }
        }
        // The documented examples at 48 kHz.
        let m = ring_frames(SR);
        assert_eq!(delay_for_beats(3, 4, 140.0, SR, m), 15_429); // 15428.57
        assert_eq!(delay_for_beats(1, 1, 120.0, SR, m), 24_000);
        assert_eq!(delay_for_beats(4, 1, 120.0, SR, m), 96_000); // a bar at 120 BPM is exactly 2 s
        // Slower than 120 BPM a bar is longer than the ring and plays at 2 s.
        assert_eq!(delay_for_beats(4, 1, 100.0, SR, m), 96_000);
        assert_eq!(delay_for_beats(4, 1, 20.0, SR, m), 96_000);
        // A bad tempo is the longest delay, never a panic.
        assert_eq!(delay_for_beats(1, 1, f64::NAN, SR, m), m);
        assert_eq!(delay_for_beats(1, 1, 0.0, SR, m), m);
        assert_eq!(delay_for_beats(1, 1, f64::INFINITY, SR, m), MIN_DELAY);
    }

    #[test]
    fn a_time_in_milliseconds_is_rounded_to_a_sample() {
        let m = ring_frames(SR);
        assert_eq!(delay_for_ms(300.0, SR, m), 14_400);
        assert_eq!(delay_for_ms(1.0, SR, m), 48);
        assert_eq!(delay_for_ms(2_000.0, SR, m), 96_000);
        assert_eq!(delay_for_ms(0.001, SR, m), MIN_DELAY);
        assert_eq!(delay_for_ms(1e9, SR, m), m);
        // 80 ms at 44.1 kHz is 3,528 samples; 100.3 ms is 4,423.23.
        let m = ring_frames(44_100.0);
        assert_eq!(delay_for_ms(80.0, 44_100.0, m), 3_528);
        assert_eq!(delay_for_ms(100.3, 44_100.0, m), 4_423);
    }

    #[test]
    fn the_rings_hold_two_seconds_and_are_all_the_memory() {
        let d = Delay::new(SR);
        assert_eq!(d.max_delay(), 96_000);
        assert_eq!(d.allocated_bytes(), 2 * 96_000 * 4);
        assert_eq!(Delay::new(44_100.0).max_delay(), 88_200);
        assert_eq!(Delay::new(192_000.0).allocated_bytes(), 3_072_000);
        assert!(d.is_idle());
        assert_eq!(d.fade_frames(), 960);
    }

    #[test]
    fn the_soft_clip_is_the_identity_up_to_the_knee_and_bounded_after() {
        for i in -50..=50 {
            let u = i as f32 * 0.01;
            assert_eq!(soft_clip(u).to_bits(), u.to_bits(), "{u}");
        }
        assert_eq!(soft_clip(1.5), 1.0);
        assert_eq!(soft_clip(-1.5), -1.0);
        assert_eq!(soft_clip(1e30), 1.0);
        assert_eq!(soft_clip(-1e30), -1.0);
        assert!((soft_clip(1.0) - 0.875).abs() < 1e-7);
        // Odd, monotone, never above the limit, and C1: the slope is 1 at the knee and 0 at 1.5.
        let mut last = -2.0;
        for i in -2000..=2000 {
            let u = i as f32 * 0.001;
            let y = soft_clip(u);
            assert!(y >= last, "monotone at {u}");
            assert!(y.abs() <= CLIP_LIMIT);
            assert_eq!(soft_clip(-u).to_bits(), (-y).to_bits());
            last = y;
        }
        let h = 1e-3;
        let slope = |u: f32| (soft_clip(u + h) - soft_clip(u - h)) / (2.0 * h);
        assert!((slope(0.5) - 1.0).abs() < 2e-3, "{}", slope(0.5));
        assert!(slope(1.5).abs() < 2e-3, "{}", slope(1.5));
        assert!((slope(1.0) - 0.5).abs() < 2e-3, "{}", slope(1.0));
    }

    #[test]
    fn the_filter_coefficient_gives_the_3_db_corner() {
        // |H(e^jw)| of the trapezoidal one-pole low-pass at its own corner is 1/sqrt(2) for any corner below Nyquist.
        for fc in [20.0f32, 150.0, 1_000.0, 6_000.0, 15_000.0, 20_000.0] {
            let g = f64::from(one_pole_g(fc, SR));
            let w = 2.0 * core::f64::consts::PI * f64::from(fc) / f64::from(SR);
            let (c, s) = (libm::cos(w), libm::sin(w));
            // H = G (1 + z^-1) / (1 − (1 − 2G) z^-1)
            let (nr, ni) = (g * (1.0 + c), -g * s);
            let (dr, di) = (1.0 - (1.0 - 2.0 * g) * c, (1.0 - 2.0 * g) * s);
            let mag = libm::sqrt((nr * nr + ni * ni) / (dr * dr + di * di));
            assert!(
                (mag - core::f64::consts::FRAC_1_SQRT_2).abs() < 2e-6,
                "{fc} Hz: {mag}"
            );
        }
        // A corner above Nyquist is held below it; a tiny one stays above zero.
        assert!(one_pole_g(1e9, 8_000.0) < 1.0);
        assert!(one_pole_g(0.0, SR) > 0.0);
    }

    #[test]
    fn echoes_of_an_impulse_land_on_exact_samples() {
        for d in [1usize, 2, 31, 32, 33, 100, 777, 4_800, 15_429, 96_000] {
            let mut core = Delay::new(SR);
            let frames = ((d * 5 + 100).div_ceil(BLOCK) + 1) * BLOCK;
            let x = impulse(frames, 100);
            let (wl, wr) = wet(&mut core, &x, &x, |_| open(d, 0.9));
            // The first echo starts on exactly sample 100 + d: the wet signal is exactly zero before it (the filters
            // have no latency, so the first sample is already the echo).
            assert!(wl[..100 + d].iter().all(|v| *v == 0.0), "delay {d}");
            assert!(wl[100 + d] != 0.0, "delay {d}");
            if d < 100 {
                continue;
            }
            // And every later echo starts on its own sample: in a window around it, the first sample above a tenth
            // of the window's peak is exactly `100 + k·d` (the filters have no latency, so an echo has no lead-in).
            for k in 1..=4usize {
                let at = 100 + k * d;
                if at + 50 >= frames {
                    continue;
                }
                let win = at - 50..at + 50;
                let top = win.clone().fold(0.0f32, |m, i| m.max(wl[i].abs()));
                let first = win.clone().find(|&i| wl[i].abs() > 0.1 * top).unwrap();
                assert_eq!(first, at, "echo {k} of an impulse at delay {d}");
                assert!(wl[at].abs() > 0.02 * geo(0.9, k - 1), "{}", wl[at]);
                assert_eq!(wl[at].to_bits(), wr[at].to_bits(), "the channels agree");
            }
        }
    }

    #[test]
    fn nothing_comes_before_the_first_echo() {
        let mut core = Delay::new(SR);
        let x = impulse(4_096, 0);
        let (wl, _) = wet(&mut core, &x, &x, |_| open(1_000, 0.5));
        assert!(wl[..1_000].iter().all(|v| *v == 0.0));
        assert!(wl[1_000] != 0.0);
    }

    #[test]
    fn ping_pong_alternates_the_channels_and_starts_left() {
        let d = 640;
        let mut core = Delay::new(SR);
        let x = impulse(d * 7, 0);
        assert_eq!(x.len() % BLOCK, 0);
        let mut p = open(d, 0.8);
        p.ping_pong = true;
        let (wl, wr) = wet(&mut core, &x, &x, |_| p);
        // The first echo has the right channel exactly silent; later the other channel holds only filter tails.
        assert!(wr[..d + 100].iter().all(|v| *v == 0.0));
        for k in 1..=6usize {
            let (l, r) = (wl[k * d].abs(), wr[k * d].abs());
            let (loud, quiet) = if k % 2 == 1 { (l, r) } else { (r, l) };
            assert!(
                loud > 0.05 * geo(0.8, k - 1) && quiet < 0.01,
                "echo {k}: {l} {r}"
            );
        }
        // Without ping-pong a mono impulse echoes on both channels at once.
        let mut core = Delay::new(SR);
        let (wl, wr) = wet(&mut core, &x, &x, |_| open(d, 0.8));
        assert!(wl[d] != 0.0 && wl[d].to_bits() == wr[d].to_bits());
    }

    #[test]
    fn ping_pong_takes_the_mono_sum_of_a_stereo_input() {
        let d = 512;
        let mut core = Delay::new(SR);
        let (xl, xr) = (impulse(d * 3, 0), vec![0.0; d * 3]);
        assert_eq!(d * 3 % BLOCK, 0);
        let mut p = open(d, 0.5);
        p.ping_pong = true;
        let (wl, wr) = wet(&mut core, &xl, &xr, |_| p);
        // A left-only impulse enters at half its level, as the mono sum does, and is heard on the left first.
        let mut mono = Delay::new(SR);
        let one = impulse(d * 3, 0);
        let (ml, _) = wet(&mut mono, &one, &one, |_| p);
        assert_eq!(wl[d].to_bits(), (0.5 * ml[d]).to_bits());
        assert!(wr[d] == 0.0 && wr[2 * d] != 0.0);
    }

    /// `base^k` by repeated multiplication (`powi` and `powf` are banned for determinism).
    fn geo(base: f32, k: usize) -> f32 {
        (0..k).fold(1.0, |a, _| a * base)
    }

    /// The RMS of `v[from..to]`.
    fn rms(v: &[f32], from: usize, to: usize) -> f64 {
        let s: f64 = v[from..to]
            .iter()
            .map(|x| {
                let x = f64::from(*x);
                x * x
            })
            .sum();
        libm::sqrt(s / (to - from) as f64)
    }

    /// The magnitude of the loop's two filters at `f`: a one-pole high-pass at `lc` and a one-pole low-pass at `hc`.
    fn filter_gain(f: f64, lc: f32, hc: f32) -> f64 {
        let w = 2.0 * core::f64::consts::PI * f / f64::from(SR);
        let lp = |fc: f32| {
            let g = f64::from(one_pole_g(fc, SR));
            let (c, s) = (libm::cos(w), libm::sin(w));
            // (re, im) of H = G (1 + z^-1) / (1 − (1 − 2G) z^-1)
            let (nr, ni) = (g * (1.0 + c), -g * s);
            let (dr, di) = (1.0 - (1.0 - 2.0 * g) * c, (1.0 - 2.0 * g) * s);
            let den = dr * dr + di * di;
            ((nr * dr + ni * di) / den, (ni * dr - nr * di) / den)
        };
        let (hr, hi) = lp(lc);
        let hp = libm::hypot(1.0 - hr, -hi);
        hp * libm::hypot(lp(hc).0, lp(hc).1)
    }

    #[test]
    fn the_echo_has_the_closed_form_response_of_the_two_filters() {
        // A sine with a whole number of cycles in the window, delayed by 2,000 samples, measured after it settles.
        for (f, lc, hc) in [
            (1_000.0, 150.0, 6_000.0),
            (150.0, 150.0, 6_000.0),
            (6_000.0, 150.0, 6_000.0),
            (60.0, 150.0, 6_000.0),
            (9_000.0, 150.0, 6_000.0),
            (1_000.0, 800.0, 1_500.0),
            (12_000.0, 20.0, 20_000.0),
        ] {
            let mut core = Delay::new(SR);
            let n = 4 * 48_000usize;
            let w = 2.0 * core::f64::consts::PI * f / f64::from(SR);
            let x: Vec<f32> = (0..n)
                .map(|i| (0.25 * libm::sin(w * i as f64)) as f32)
                .collect();
            let mut p = BlockParams::steady(2_000, 0.0, 1.0);
            p.low_cut_hz = lc;
            p.high_cut_hz = hc;
            let (wl, _) = wet(&mut core, &x, &x, |_| p);
            let got = rms(&wl, 2 * 48_000, 4 * 48_000) / rms(&x, 2 * 48_000, 4 * 48_000);
            let want = filter_gain(f, lc, hc);
            let db = 20.0 * libm::log10(got / want);
            assert!(db.abs() < 0.01, "{f} Hz: {got} vs {want} ({db} dB)");
        }
    }

    #[test]
    fn feedback_decays_at_the_set_rate() {
        // A 1 kHz burst, filters passing it at the same gain every repeat: each repeat is `feedback × gain` of the
        // one before, in RMS, to a hundredth of a dB, normal or ping-pong (where the repeats alternate channels).
        let d = 6_048;
        for pp in [false, true] {
            for fb in [0.25f32, 0.5, 0.8] {
                let mut core = Delay::new(SR);
                let n = d * 7;
                let w = 2.0 * core::f64::consts::PI * 1_000.0 / f64::from(SR);
                let mut x = vec![0.0f32; n];
                for (i, v) in x.iter_mut().enumerate().take(2_400) {
                    *v = (0.25 * libm::sin(w * i as f64)) as f32;
                }
                let mut p = BlockParams::steady(d, fb, 1.0);
                p.ping_pong = pp;
                let (wl, wr) = wet(&mut core, &x, &x, |_| p);
                let gain = filter_gain(1_000.0, 150.0, 6_000.0);
                // Echo k occupies [k·d, k·d + 2400 + a short tail): its RMS over the burst's span, on the channel
                // that carries it.
                let e = |k: usize| {
                    let v = if pp && k.is_multiple_of(2) { &wr } else { &wl };
                    rms(v, k * d + 200, k * d + 2_400)
                };
                for k in 1..5 {
                    let ratio = e(k + 1) / e(k);
                    let want = f64::from(fb) * gain;
                    let db = 20.0 * libm::log10(ratio / want);
                    assert!(
                        db.abs() < 0.05,
                        "pp {pp} fb {fb} echo {k}: {ratio} vs {want} ({db} dB)"
                    );
                }
            }
        }
    }

    /// A deterministic white-ish noise (xorshift), −1..1.
    fn noise(n: usize, mut state: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn a_hundred_percent_feedback_stays_bounded() {
        // Full-scale noise for 12 s at 100 % feedback, every filter setting at its extremes, with and without
        // ping-pong: |wet| ≤ 2·(1 + 1) by the filters' ℓ1 norm, and in practice about 2.
        for (lc, hc, pp) in [
            (150.0, 6_000.0, false),
            (150.0, 6_000.0, true),
            (20.0, 20_000.0, false),
            (2_000.0, 1_000.0, true),
        ] {
            for d in [37usize, 1_000, 24_000] {
                let mut core = Delay::new(SR);
                let n = 12 * 48_000;
                let x = noise(n, 0x1234_5678 + d as u32);
                let y = noise(n, 0x8765_4321 + d as u32);
                let mut p = BlockParams::steady(d, 1.0, 1.0);
                p.low_cut_hz = lc;
                p.high_cut_hz = hc;
                p.ping_pong = pp;
                let (wl, wr) = wet(&mut core, &x, &y, |_| p);
                let peak = wl.iter().chain(&wr).fold(0.0f32, |m, v| m.max(v.abs()));
                eprintln!("fb 100 %, {lc}/{hc} Hz, ping-pong {pp}, delay {d}: wet peak {peak:.3}");
                assert!(peak <= 4.0, "{lc}/{hc} pp {pp} d {d}: {peak}");
                assert!(wl.iter().chain(&wr).all(|v| v.is_finite()));
                // And the ring never holds more than |x| + the clip limit.
                let ring_peak = core.ring.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(ring_peak <= 1.0 + CLIP_LIMIT, "{ring_peak}");
            }
        }
    }

    #[test]
    fn a_hundred_percent_feedback_keeps_repeating_a_burst() {
        // The soft clip must not eat a repeat for ever: a burst still comes back at 100 % after ten laps.
        let d = 2_400;
        let mut core = Delay::new(SR);
        let n = d * 12;
        let mut x = vec![0.0f32; n];
        for (i, v) in x.iter_mut().enumerate().take(480) {
            *v = 0.3 * math::sinf(i as f32 * 0.13);
        }
        let (wl, _) = wet(&mut core, &x, &x, |_| open(d, 1.0));
        let early = rms(&wl, d + 100, d + 400);
        let late = rms(&wl, 10 * d + 100, 10 * d + 400);
        // A 20 kHz one-pole ahead of the Nyquist-bound low-pass costs a little each lap; the burst is alive.
        assert!(late > 0.05 * early && late < 1.5 * early, "{early} {late}");
    }

    #[test]
    fn mix_zero_leaves_the_block_alone_and_the_line_keeps_recording() {
        let mut core = Delay::new(SR);
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        l[0] = -0.0;
        l[1] = f32::NAN;
        l[2] = 0.7;
        r[3] = -0.25;
        r[4] = f32::INFINITY;
        let (bl, br) = (l.map(f32::to_bits), r.map(f32::to_bits));
        core.process(&BlockParams::steady(40, 0.5, 0.0), &mut l, &mut r);
        assert_eq!(l.map(f32::to_bits), bl);
        assert_eq!(r.map(f32::to_bits), br);
        // Recording went on at mix 0, and the garbage entered the loop sanitised: 40 frames after the 0.7 sample the
        // echo is there, and everything is finite.
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        core.process(&BlockParams::steady(40, 0.5, 1.0), &mut l, &mut r);
        assert!(l[2 + 40 - BLOCK].abs() > 0.1, "{}", l[2 + 40 - BLOCK]);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        for _ in 0..20 {
            core.process(&BlockParams::steady(40, 0.5, 1.0), &mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        }
    }

    #[test]
    fn mix_ramps_inside_a_block_and_zero_to_zero_is_a_no_op() {
        let mut core = Delay::new(SR);
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        // Prime the ring with ones, then read them back through a ramp from 0 to 1.
        let mut ones = [1.0f32; BLOCK];
        let mut z = [0.0f32; BLOCK];
        core.process(&open(BLOCK, 0.0), &mut ones, &mut z);
        core.process(
            &BlockParams {
                mix_start: 0.0,
                mix_end: 1.0,
                ..open(BLOCK, 0.0)
            },
            &mut l,
            &mut r,
        );
        // The echo of a block of ones, filtered; the ramp scales it by (i + 1)/32.
        let mut full = Delay::new(SR);
        let mut ones = [1.0f32; BLOCK];
        let mut z = [0.0f32; BLOCK];
        full.process(&open(BLOCK, 0.0), &mut ones, &mut z);
        let mut f = [0.0f32; BLOCK];
        let mut g = [0.0f32; BLOCK];
        full.process(&open(BLOCK, 0.0), &mut f, &mut g);
        for i in 0..BLOCK {
            let want = f[i] * ((i + 1) as f32 / BLOCK as f32);
            assert!((l[i] - want).abs() < 1e-6, "{i}: {} vs {want}", l[i]);
        }
    }

    /// The largest jump between neighbouring samples.
    fn max_step(v: &[f32]) -> f32 {
        v.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
    }

    #[test]
    fn a_time_change_crossfades_without_a_click() {
        let n = 48_000usize;
        let w = 2.0 * core::f64::consts::PI * 440.0 / f64::from(SR);
        let x: Vec<f32> = (0..n)
            .map(|i| (0.5 * libm::sin(w * i as f64)) as f32)
            .collect();
        let change = 500; // block index: after the first echoes have started
        let run_with = |first: usize, second: usize| {
            let mut core = Delay::new(SR);
            wet(&mut core, &x, &x, |k| {
                BlockParams::steady(if k < change { first } else { second }, 0.0, 1.0)
            })
            .0
        };
        let steady = run_with(5_000, 5_000);
        let moved = run_with(5_000, 7_777);
        // The sine's own largest step is 0.5·w ≈ 0.029; both filters keep it there. A hard switch between the two
        // delays would jump by about the sine's amplitude.
        let natural = max_step(&steady[10_000..]);
        let across = max_step(&moved[change * BLOCK - 200..change * BLOCK + 2_000]);
        eprintln!("time change: natural step {natural:.4}, across the fade {across:.4}");
        assert!(natural < 0.035, "{natural}");
        assert!(across < 1.15 * natural + 0.005, "{across} vs {natural}");
        // Before the change the two runs are the same bits.
        assert_eq!(&steady[..change * BLOCK], &moved[..change * BLOCK]);
        // A hard switch between the same two settled delays does click, at some phase of the sine: the largest jump
        // between the two settled signals over the next 100 frames is far above the sine's own step.
        let old = run_with(5_000, 5_000);
        let new = run_with(7_777, 7_777);
        let jump = (change * BLOCK..change * BLOCK + 100)
            .fold(0.0f32, |m, i| m.max((old[i] - new[i]).abs()));
        assert!(jump > 0.2, "{jump}");
    }

    #[test]
    fn the_crossfade_lasts_the_fade_and_ends_on_the_new_delay_exactly() {
        let mut core = Delay::new(SR);
        let fade = core.fade_frames() as usize; // 960
        let mut x = vec![0.0f32; 20 * BLOCK];
        let mut y = vec![0.0f32; 20 * BLOCK];
        // Start at delay 100, run a block, then ask for 300.
        run(&mut core, &mut x[..BLOCK], &mut y[..BLOCK], |_| {
            open(100, 0.0)
        });
        assert!(!core.is_fading());
        // Nothing has been heard yet, so the core is still idle and would snap; feed it a signal first.
        let mut core = Delay::new(SR);
        let mut sig = noise(40 * BLOCK, 7);
        let mut sig2 = sig.clone();
        let target = |k: usize| open(if k < 3 { 100 } else { 300 }, 0.0);
        run(
            &mut core,
            &mut sig[..3 * BLOCK],
            &mut sig2[..3 * BLOCK],
            target,
        );
        assert_eq!(core.current_delay(), 100);
        let mut blocks = 0;
        let mut k = 3;
        while k < 40 {
            let mut l = [0.1f32; BLOCK];
            let mut r = [0.1f32; BLOCK];
            core.process(&target(k), &mut l, &mut r);
            blocks += 1;
            k += 1;
            if !core.is_fading() {
                break;
            }
        }
        // 960 frames = exactly 30 blocks.
        assert_eq!(blocks * BLOCK, fade);
        assert_eq!(core.current_delay(), 300);
    }

    #[test]
    fn a_target_that_changes_during_a_fade_waits_and_then_gets_its_own_fade() {
        let mut core = Delay::new(SR);
        let mut l = [0.2f32; BLOCK];
        let mut r = [0.2f32; BLOCK];
        core.process(&open(100, 0.0), &mut l, &mut r);
        core.process(&open(300, 0.0), &mut l, &mut r);
        assert!(core.is_fading());
        // Ask for a third delay while the first fade runs: the first still completes, to 300.
        for _ in 0..10 {
            let mut l = [0.2f32; BLOCK];
            let mut r = [0.2f32; BLOCK];
            core.process(&open(500, 0.0), &mut l, &mut r);
        }
        assert!(core.is_fading());
        let mut guard = 0;
        while core.is_fading() && core.current_delay() != 300 {
            let mut l = [0.2f32; BLOCK];
            let mut r = [0.2f32; BLOCK];
            core.process(&open(500, 0.0), &mut l, &mut r);
            guard += 1;
            assert!(guard < 100);
        }
        assert_eq!(core.current_delay(), 300);
        let mut guard = 0;
        while core.current_delay() != 500 {
            let mut l = [0.2f32; BLOCK];
            let mut r = [0.2f32; BLOCK];
            core.process(&open(500, 0.0), &mut l, &mut r);
            guard += 1;
            assert!(guard < 100);
        }
        assert!(!core.is_fading());
    }

    #[test]
    fn a_fade_to_the_same_delay_never_starts() {
        let mut core = Delay::new(SR);
        for _ in 0..10 {
            let mut l = [0.2f32; BLOCK];
            let mut r = [0.2f32; BLOCK];
            core.process(&open(777, 0.5), &mut l, &mut r);
            assert!(!core.is_fading());
        }
    }

    #[test]
    fn the_delay_is_clamped_into_the_ring() {
        let mut core = Delay::new(SR);
        let x = impulse(BLOCK * 3, 0);
        let (wl, _) = wet(&mut core, &x, &x, |_| open(0, 0.0));
        // Delay 0 plays as the shortest delay, one frame.
        assert_eq!(core.current_delay(), MIN_DELAY);
        assert!(wl[1] != 0.0 && wl[0] == 0.0);
        let mut core = Delay::new(SR);
        let mut l = [0.3f32; BLOCK];
        let mut r = [0.3f32; BLOCK];
        core.process(&open(usize::MAX, 0.0), &mut l, &mut r);
        assert_eq!(core.current_delay(), 96_000);
    }

    #[test]
    fn toggling_ping_pong_on_a_running_echo_does_not_click() {
        let n = 48_000usize;
        let w = 2.0 * core::f64::consts::PI * 330.0 / f64::from(SR);
        let x: Vec<f32> = (0..n)
            .map(|i| (0.4 * libm::sin(w * i as f64)) as f32)
            .collect();
        let toggle = 400usize;
        let mut core = Delay::new(SR);
        let (wl, wr) = wet(&mut core, &x, &x, |k| {
            let mut p = BlockParams::steady(3_000, 0.6, 1.0);
            p.ping_pong = k >= toggle / BLOCK;
            p
        });
        let natural = 0.4 * w as f32 * 1.6; // the sine's steepest slope, with the echoes summed at ≤ 1.6×
        assert!(max_step(&wl) < natural + 0.01, "{}", max_step(&wl));
        assert!(max_step(&wr) < natural + 0.01, "{}", max_step(&wr));
    }

    #[test]
    fn two_instances_keep_separate_state() {
        let x = noise(BLOCK * 200, 99);
        let mut a = Delay::new(SR);
        let mut b = Delay::new(SR);
        let pa = |_| open(900, 0.7);
        let pb = |_| open(1_300, 0.4);
        let (la, _) = wet(&mut a, &x, &x, pa);
        let (lb, _) = wet(&mut b, &x, &x, pb);
        // Alone, each gives the same as beside the other.
        let mut a2 = Delay::new(SR);
        let (la2, _) = wet(&mut a2, &x, &x, pa);
        assert_eq!(la, la2);
        assert_ne!(la, lb);
        // Interleaved calls change nothing either.
        let mut c = Delay::new(SR);
        let mut d = Delay::new(SR);
        let (mut cl, mut cr) = (x.clone(), x.clone());
        let (mut dl, mut dr) = (x.clone(), x.clone());
        let (cl_blocks, _) = cl.as_chunks_mut::<BLOCK>();
        let (cr_blocks, _) = cr.as_chunks_mut::<BLOCK>();
        let (dl_blocks, _) = dl.as_chunks_mut::<BLOCK>();
        let (dr_blocks, _) = dr.as_chunks_mut::<BLOCK>();
        for k in 0..cl_blocks.len() {
            c.process(&pa(k), &mut cl_blocks[k], &mut cr_blocks[k]);
            d.process(&pb(k), &mut dl_blocks[k], &mut dr_blocks[k]);
        }
        let sub = |o: &[f32]| o.iter().zip(&x).map(|(a, b)| a - b).collect::<Vec<_>>();
        assert_eq!(sub(&cl), la);
        assert_eq!(sub(&dl), lb);
    }

    #[test]
    fn a_copied_state_runs_on_as_the_original_and_a_taken_one_too() {
        let x = noise(BLOCK * 300, 5);
        let y = noise(BLOCK * 300, 6);
        let p = |k: usize| {
            let mut p = open(if k < 120 { 1_100 } else { 1_700 }, 0.7);
            p.ping_pong = k >= 60;
            p
        };
        let mut a = Delay::new(SR);
        let (mut l, mut r) = (x[..BLOCK * 100].to_vec(), y[..BLOCK * 100].to_vec());
        run(&mut a, &mut l, &mut r, p);
        assert!(!a.is_idle());
        // Copy into a core that has run something else.
        let mut b = Delay::new(SR);
        let mut junk = noise(BLOCK * 20, 1);
        let mut junk2 = junk.clone();
        run(&mut b, &mut junk, &mut junk2, |_| open(333, 0.9));
        assert!(b.copy_state_from(&a));
        // Take into a third from a clone.
        let mut c = Delay::new(SR);
        let mut a_clone = a.clone();
        assert!(c.take_state_from(&mut a_clone));
        let rest = |core: &mut Delay| {
            let (mut l, mut r) = (x[BLOCK * 100..].to_vec(), y[BLOCK * 100..].to_vec());
            let (lb, _) = l.as_chunks_mut::<BLOCK>();
            let (rb, _) = r.as_chunks_mut::<BLOCK>();
            for (k, (l, r)) in lb.iter_mut().zip(rb.iter_mut()).enumerate() {
                core.process(&p(k + 100), l, r);
            }
            (l, r)
        };
        let want = rest(&mut a);
        assert_eq!(rest(&mut b), want, "copy");
        assert_eq!(rest(&mut c), want, "take");
        // A core built for another sample rate refuses.
        let mut other = Delay::new(44_100.0);
        assert!(!other.copy_state_from(&a));
        assert!(!other.take_state_from(&mut a));
    }

    #[test]
    fn a_taken_ring_is_moved_not_copied() {
        let mut a = Delay::new(SR);
        let mut l = [0.5f32; BLOCK];
        let mut r = [0.5f32; BLOCK];
        a.process(&open(100, 0.5), &mut l, &mut r);
        let ptr = a.ring.as_ptr();
        let mut b = Delay::new(SR);
        assert!(b.take_state_from(&mut a));
        assert_eq!(
            b.ring.as_ptr(),
            ptr,
            "the new core holds the old allocation"
        );
        assert!(
            a.ring.iter().all(|v| *v == 0.0),
            "the old core holds the fresh, zeroed one"
        );
    }

    #[test]
    fn it_goes_idle_when_the_tail_has_died_and_skipping_equals_running() {
        let mut core = Delay::new(SR);
        let n = BLOCK * 6000;
        let mut l = vec![0.0f32; n];
        l[..BLOCK * 3].copy_from_slice(&noise(BLOCK * 3, 3));
        let mut r = l.clone();
        // 4 s at feedback 0.1: the ring is rewritten with exact zeros once the last repeat falls below 1e-15.
        run(&mut core, &mut l, &mut r, |_| open(1_000, 0.1));
        assert!(core.is_idle());
        // A silent block now changes nothing, wet or state (the engine may skip it).
        let before = core.clone();
        let mut zl = [0.0f32; BLOCK];
        let mut zr = [0.0f32; BLOCK];
        core.process(&open(4_321, 0.9), &mut zl, &mut zr);
        assert!(zl.iter().chain(&zr).all(|v| v.to_bits() == 0));
        assert_eq!(core.s, before.s);
        assert!(core.ring.iter().all(|v| *v == 0.0));
        // The first sound wakes it straight onto the block's settings: the same bits whether or not the silent
        // blocks in between were run.
        let mut run_all = before.clone();
        for _ in 0..50 {
            let mut a = [0.0f32; BLOCK];
            let mut b = [0.0f32; BLOCK];
            run_all.process(&open(4_321, 0.9), &mut a, &mut b);
        }
        let mut skipped = before;
        let mut loud = noise(BLOCK, 11);
        let mut loud2 = loud.clone();
        let mut loud3 = loud.clone();
        let mut loud4 = loud.clone();
        run_all.process(
            &open(4_321, 0.9),
            loud.as_mut_slice().try_into().unwrap(),
            loud2.as_mut_slice().try_into().unwrap(),
        );
        skipped.process(
            &open(4_321, 0.9),
            loud3.as_mut_slice().try_into().unwrap(),
            loud4.as_mut_slice().try_into().unwrap(),
        );
        assert_eq!(loud, loud3);
        assert_eq!(loud2, loud4);
        assert!(!skipped.is_idle());
    }

    #[test]
    fn a_reset_clears_the_ring_and_a_fresh_wake_has_no_glide() {
        let mut core = Delay::new(SR);
        let x = impulse(BLOCK * 30, 3);
        let (_, _) = wet(&mut core, &x, &x, |_| open(100, 0.5));
        assert!(!core.is_idle());
        core.reset();
        assert!(core.is_idle() && core.ring.iter().all(|v| *v == 0.0));
        // Reset, then straight to 700: the echo is at 700 with no crossfade.
        let (wl, _) = wet(&mut core, &x, &x, |_| open(700, 0.0));
        assert!(!core.is_fading());
        assert!(wl[3 + 700] != 0.0 && wl[3 + 100] == 0.0);
    }

    #[test]
    fn the_tail_leaves_no_subnormals_and_ends_flushed() {
        for (fb, pp) in [(0.5f32, false), (0.95, true), (1.0, false)] {
            let mut core = Delay::new(SR);
            let n = 40 * 48_000 / BLOCK * BLOCK;
            let mut l = vec![0.0f32; n];
            l[..BLOCK * 20].copy_from_slice(&noise(BLOCK * 20, 21));
            let mut r = l.clone();
            let mut p = open(7_919, fb);
            p.ping_pong = pp;
            run(&mut core, &mut l, &mut r, |_| p);
            assert_eq!(core.count_subnormals(), 0, "fb {fb}");
            assert!(l.iter().chain(&r).all(|v| !v.is_subnormal()));
        }
    }

    #[test]
    fn a_silent_run_never_wakes_the_core() {
        let mut core = Delay::new(SR);
        for _ in 0..100 {
            let mut l = [0.0f32; BLOCK];
            let mut r = [0.0f32; BLOCK];
            core.process(&open(1_000, 0.9), &mut l, &mut r);
        }
        assert!(core.is_idle());
        // -0.0 counts as silence too.
        let mut l = [-0.0f32; BLOCK];
        let mut r = [-0.0f32; BLOCK];
        core.process(&open(1_000, 0.9), &mut l, &mut r);
        assert!(core.is_idle());
        assert!(l.iter().all(|v| v.to_bits() == (-0.0f32).to_bits()));
    }
}
