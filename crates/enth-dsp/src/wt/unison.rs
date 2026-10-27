//! Unison for `wt` (S2a lane UNI, `docs/dsp/wt.md`): two to eight detuned copies of the oscillator per voice, on
//! `wide`'s `f32x8` lanes, summed in a fixed order.
//!
//! # What a copy is
//!
//! Lane `k` of a [`UnisonVoice`] is **exactly a [`Voice`](super::Voice)**: the same phase accumulator, the same
//! feedback history, the same mip-level tracker and cross-fade, the same read through the same warp, with its own
//! phase increment (the centre's times the copy's detune ratio) and its own start phase. Every operation the lanes do
//! is an IEEE `f32` `+ − × ÷`, a compare, a select, a `floor`, a `min` or a `max` on the same operands in the same
//! order as `Voice::render` does it, and `wide` is bit-exact for exactly that subset (`clippy.toml` bans the rest:
//! fused operations, `recip`, the polynomial transcendentals, `reduce_add`, `round_int`). The unit tests render every
//! warp, with and without phase modulation, feedback, a moving position and a mip-level change, against plain
//! `Voice`s, bit for bit. The table read is a gather (`wide` has none): the indices and the four taps of each lane are
//! worked out one lane at a time, the interpolation arithmetic runs on all lanes at once.
//!
//! What the copies share: the phase modulation (one routed audio input), the warp, the position (so the frame pair),
//! the feedback amount, and the table. What differs: phase, increment, mip level, feedback history.
//!
//! # The mix
//!
//! [`Mix`] holds the per-copy constants of one end of a segment, from `unison`, `detune`, `blend` and `spread`:
//!
//! - **Detune.** Copy `k` of `n` sits at `pos_k = (2k − (n − 1)) / (n − 1)` in `[−1, 1]` and is `pos_k · detune`
//!   cents from the centre, so the copies are spread evenly across `±detune` (two copies sit at `±detune`).
//!   The increment is the centre's times `2^(cents / 1200)`, through the pinned `libm`.
//! - **Blend.** The copies nearest the centre (the middle one for an odd `n`, the middle two for an even `n`) have
//!   weight 1 and the others `blend`; with two copies there is no side copy, and `blend` does nothing.
//! - **Level.** Every gain is divided by `sqrt(Σ weight²)`: the copies are not correlated, so their power adds, and
//!   this keeps the loudness the same for any count and blend (`n = 1` would be gain 1).
//! - **Spread.** Copy `k` is panned to `spread · pos_k` with a constant-power law scaled to **1 at the centre**,
//!   `(√2·cos θ, √2·sin θ)`, `θ = (pan + 1)·π/4`, so at `spread` 0 both channels are exactly the mono mix (bit for
//!   bit: the two gains round to exactly 1.0), and the total power of the two channels is the same at any spread. The
//!   outermost copies are hard left and right at 100 %: exactly `(√2, 0)` and `(0, √2)`. The right half is the exact
//!   mirror of the left half, so a symmetric voice is symmetric to the last bit.
//!
//! # The sum
//!
//! A channel is `Σ_k y_k · c_k(t)` with `c_k(t) = c0_k + (c1_k − c0_k)·RAMP[i]`, added in lane order from lane 0,
//! starting from `+0.0`: `((((0 + t0) + t1) + t2) + …`, over the `n` copies. The horizontal sum never uses
//! `reduce_add` (its order depends on the target): the products go through `to_array()` and are added one by one.
//! The lanes beyond the `n` copies hold a gain of exactly 0 and a value of exactly `+0.0`, so their products are
//! `+0.0` and adding them changes nothing: the sum runs over all eight lanes without a loop on the count, and equals
//! the sum over the `n` copies bit for bit (the leading `+0.0` is what makes that true even when every product is
//! `−0.0`: the sum is `+0.0` either way).
//!
//! # The `wide` subset
//!
//! `f32x8` (and `f32x4` for the tap transposes) with `+ − × ÷`, `min`, `floor`, the compares, `select` and the bit
//! operations, `splat` and the array conversions, `trunc_int` (an exact truncation; the positions are never negative
//! or NaN) with `i32x8`'s `+` and `&`, `i32x8::round_float` (an exact conversion of integers below 2^24) and
//! `f32x4::transpose` (a permutation). All of them are exact on every target, so the six-target golden
//! (`wt-unison-*`) sees the same bits. Nothing from `clippy.toml`'s ban list is used.

use wide::{f32x4, f32x8, i32x8};

use super::{BELOW_ONE, NO_LEVEL, Prep, Prologue, Ramp, Segment, TRACK_DECAY, Warp, prologue};
use crate::osc::{MAX_DT_PHASED, clamp_dt};
use crate::util::flush::flush;
use crate::util::math;
use crate::wavetable::{FRAME_STRIDE, FRAMES, LEN, LEVELS, OFFSET, level_for};

/// The most copies of a unison voice: one `f32x8`.
pub const LANES: usize = 8;

/// `√2` in `f64`, the scale of the spread law.
const SQRT_2: f64 = core::f64::consts::SQRT_2;

// ---------------------------------------------------------------------------------------------------------------
// The mix: detune ratios and gains
// ---------------------------------------------------------------------------------------------------------------

/// The constants of the copies at one end of a segment (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    /// The number of copies, `2 ..= `[`LANES`].
    pub count: usize,
    /// Each copy's increment relative to the centre's, `2^(cents / 1200)`; 1 beyond `count`.
    pub ratio: [f32; LANES],
    /// The gain of each copy in the mono mix (the level and blend law); 0 beyond `count`.
    pub mono: [f32; LANES],
    /// The gain of each copy in the left channel: `mono` times the spread law's left gain.
    pub left: [f32; LANES],
    /// The gain of each copy in the right channel.
    pub right: [f32; LANES],
}

/// The spread law at `pan ∈ [−1, 1]` (−1 hard left): `(√2·cos θ, √2·sin θ)` with `θ = (pan + 1)·π/4`, computed in
/// `f64` and rounded once; exactly `(√2, 0)` and `(0, √2)` at the ends, and `1.0` for both at the centre.
fn pan_gains(pan: f32) -> (f32, f32) {
    if pan <= -1.0 {
        return (SQRT_2 as f32, 0.0);
    }
    if pan >= 1.0 {
        return (0.0, SQRT_2 as f32);
    }
    let theta = (f64::from(pan) + 1.0) * core::f64::consts::FRAC_PI_4;
    (
        (math::cos(theta) * SQRT_2) as f32,
        (math::sin(theta) * SQRT_2) as f32,
    )
}

/// `x` clamped to `[0, 1]`, NaN to 0.
fn unit(x: f32) -> f32 {
    if x > 0.0 { x.min(1.0) } else { 0.0 }
}

impl Mix {
    /// The mix of `count` copies (clamped to `2 ..= 8`) at `detune_cents` (the outermost copies' distance from the
    /// centre), `blend` (the side copies' weight against the centre, `0 ..= 1`) and `spread` (`0 ..= 1`).
    #[must_use]
    #[allow(
        clippy::needless_range_loop,
        reason = "k is the copy number in every formula as well as the index into each per-copy array"
    )]
    pub fn new(count: usize, detune_cents: f32, blend: f32, spread: f32) -> Self {
        let n = count.clamp(2, LANES);
        let blend = unit(blend);
        let spread = unit(spread);
        let last = (n - 1) as f32;
        // The distance of the nearest copies from the centre, in half steps: 0 for an odd count, 1 for an even one.
        let near = (n - 1) % 2;

        let mut ratio = [1.0f32; LANES];
        let mut weight = [0.0f32; LANES];
        let mut pos = [0.0f32; LANES];
        for k in 0..n {
            let d = 2 * k as i32 - (n as i32 - 1);
            pos[k] = d as f32 / last;
            ratio[k] = math::exp2(f64::from(pos[k] * detune_cents) / 1200.0) as f32;
            weight[k] = if d.unsigned_abs() as usize == near {
                1.0
            } else {
                blend
            };
        }
        let mut power = 0.0f32;
        for w in &weight[..n] {
            power += w * w;
        }
        let norm = power.sqrt();

        let mut mono = [0.0f32; LANES];
        let mut left = [0.0f32; LANES];
        let mut right = [0.0f32; LANES];
        for k in 0..n {
            let g = weight[k] / norm;
            // The right half mirrors the left half exactly: the pan of copy `k` is minus that of copy `n − 1 − k`.
            let (a, b) = if pos[k] <= 0.0 {
                pan_gains(spread * pos[k])
            } else {
                let (a, b) = pan_gains(-(spread * pos[k]));
                (b, a)
            };
            mono[k] = g;
            left[k] = g * a;
            right[k] = g * b;
        }
        Self {
            count: n,
            ratio,
            mono,
            left,
            right,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What a segment renders with
// ---------------------------------------------------------------------------------------------------------------

/// A unison segment: the centre's [`Segment`] (its `dt` is the centre's increment) and the mix at the two ends.
#[derive(Clone, Copy, Debug)]
pub struct UnisonSegment<'a, const N: usize> {
    /// The segment of the centre oscillator: table, position, warp, phase modulation and feedback are shared by
    /// every copy.
    pub seg: Segment<'a, N>,
    /// The mix at the start and at the end of the segment (the same `count`).
    pub mix: [Mix; 2],
}

/// Where a unison voice writes its segment.
pub enum Out<'a, const N: usize> {
    /// The mono mix only (a plan without the stereo voice path).
    Mono(&'a mut [f32; N]),
    /// The two channels, and the mono mix too when `mid` is `Some` (what an audio route leaving the module carries).
    Stereo {
        left: &'a mut [f32; N],
        right: &'a mut [f32; N],
        mid: Option<&'a mut [f32; N]>,
    },
}

/// The state of one voice's eight copies.
#[derive(Clone, Copy, Debug)]
pub struct UnisonVoice {
    phase: [f32; LANES],
    y1: [f32; LANES],
    y2: [f32; LANES],
    /// The tracked effective phase increment of each copy.
    tracked: [f32; LANES],
    /// The mip level each copy read in the previous segment.
    level: [u8; LANES],
    /// The last phase-modulation offset in cycles (shared by the copies: one input).
    last_off: f32,
}

impl Default for UnisonVoice {
    fn default() -> Self {
        Self::new([0.0; LANES])
    }
}

impl UnisonVoice {
    /// A voice whose copies start at `phases` (each `0 ≤ phase < 1`, else 0).
    #[must_use]
    pub fn new(phases: [f32; LANES]) -> Self {
        let mut phase = phases;
        for p in &mut phase {
            if !(0.0..1.0).contains(p) {
                *p = 0.0;
            }
        }
        Self {
            phase,
            y1: [0.0; LANES],
            y2: [0.0; LANES],
            tracked: [0.0; LANES],
            level: [NO_LEVEL; LANES],
            last_off: 0.0,
        }
    }

    /// Copy `k`'s own phase in cycles, `[0, 1)`.
    #[must_use]
    pub fn phase(&self, k: usize) -> f32 {
        self.phase[k]
    }

    /// The mip level copy `k` read in the previous segment (`None` before the first).
    #[must_use]
    pub fn level(&self, k: usize) -> Option<usize> {
        (self.level[k] != NO_LEVEL).then_some(usize::from(self.level[k]))
    }

    /// Whether any state value is subnormal.
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.phase
            .iter()
            .chain(&self.y1)
            .chain(&self.y2)
            .chain(&self.tracked)
            .any(|x| x.is_subnormal())
            || self.last_off.is_subnormal()
    }

    /// Render `N` frames of the copies' mix into `out`.
    #[allow(
        clippy::needless_range_loop,
        reason = "k is the copy number: it indexes the per-copy state, increments and levels together"
    )]
    pub fn render<const N: usize>(&mut self, s: &UnisonSegment<'_, N>, out: Out<'_, N>) {
        let seg = &s.seg;
        let count = s.mix[1].count.clamp(2, LANES);
        debug_assert_eq!(s.mix[0].count, s.mix[1].count);
        let Prologue {
            off,
            pm_slope,
            warp,
            x,
            steepest,
        } = prologue(seg, &mut self.last_off);

        // The copies' increments: the centre's times the detune ratio, clamped like the centre's.
        let mut dt = [[0.0f32; LANES]; 2];
        for k in 0..LANES {
            dt[0][k] = clamp_dt(seg.dt[0] * s.mix[0].ratio[k], MAX_DT_PHASED);
            dt[1][k] = clamp_dt(seg.dt[1] * s.mix[1].ratio[k], MAX_DT_PHASED);
        }

        // The mip level of each copy from its own tracked increment, and which of them cross-fade from an old one.
        let mut level = [0usize; LANES];
        let mut old = [0usize; LANES];
        let mut fade = [false; LANES];
        for k in 0..count {
            let dt_max = dt[0][k].max(dt[1][k]);
            self.tracked[k] =
                flush((self.tracked[k] * TRACK_DECAY).max(steepest * (dt_max + pm_slope)));
            level[k] = level_for(self.tracked[k], seg.limit);
            let before = self.level[k];
            fade[k] = before != NO_LEVEL && usize::from(before) != level[k];
            old[k] = if fade[k] {
                usize::from(before)
            } else {
                level[k]
            };
        }
        let any_fade = fade[..count].iter().any(|&f| f);

        let ctx = Ctx {
            data: &seg.table.data,
            warp,
            x,
            off: &off,
            pm: seg.pm.is_some(),
            new: Rows::new(&level, count),
            old: Rows::new(&old, count),
            fade: mask_of(fade),
            frames: Frames::new(seg.frame_pos),
            dt,
            feedback: seg.feedback,
            mix: &s.mix,
        };

        let (mut d_mid, mut d_left, mut d_right) = ([0.0f32; N], [0.0f32; N], [0.0f32; N]);
        match out {
            Out::Mono(mid) => {
                if any_fade {
                    self.run::<N, true, true, false>(&ctx, mid, &mut d_left, &mut d_right);
                } else {
                    self.run::<N, false, true, false>(&ctx, mid, &mut d_left, &mut d_right);
                }
            }
            Out::Stereo {
                left,
                right,
                mid: Some(mid),
            } => {
                if any_fade {
                    self.run::<N, true, true, true>(&ctx, mid, left, right);
                } else {
                    self.run::<N, false, true, true>(&ctx, mid, left, right);
                }
            }
            Out::Stereo {
                left,
                right,
                mid: None,
            } => {
                if any_fade {
                    self.run::<N, true, false, true>(&ctx, &mut d_mid, left, right);
                } else {
                    self.run::<N, false, false, true>(&ctx, &mut d_mid, left, right);
                }
            }
        }

        for k in 0..count {
            self.level[k] = level[k] as u8;
        }
        for v in self.y1.iter_mut().chain(&mut self.y2) {
            *v = flush(*v);
        }
    }

    /// The sample loop. `FADE`: some copy cross-fades from an old mip level over the segment. `M`: write the mono
    /// mix. `S`: write the left and right channels.
    #[inline(always)]
    fn run<const N: usize, const FADE: bool, const M: bool, const S: bool>(
        &mut self,
        c: &Ctx<'_, N>,
        mid: &mut [f32; N],
        left: &mut [f32; N],
        right: &mut [f32; N],
    ) {
        let one = f32x8::ONE;
        let half = f32x8::splat(0.5);
        let f0 = f32x8::from(c.dt[0]);
        let df = f32x8::from(c.dt[1]) - f0;
        let (b0, db) = (c.feedback[0], c.feedback[1] - c.feedback[0]);
        let feedback_on = c.feedback[0] != 0.0 || c.feedback[1] != 0.0;
        let warp_on = c.warp != Warp::Off;
        // Without phase modulation or feedback the read phase is the accumulator itself, which is always in `[0, 1)`
        // (the fold below keeps it there; `wrap01` of such a value is the value), so the add and the wrap are skipped.
        let direct = !c.pm && !feedback_on;
        // Quantize steps change by whole steps, so the end value holds for the segment; the others ramp.
        let warp_moves = c.x[0].to_bits() != c.x[1].to_bits() && c.warp != Warp::Quantize;
        let (x0, dx) = if c.warp == Warp::Quantize {
            (c.x[1], 0.0)
        } else {
            (c.x[0], c.x[1] - c.x[0])
        };
        let held = c.warp.prep(c.x[1]);
        let gain = |g: [&[f32; LANES]; 2]| {
            let g0 = f32x8::from(*g[0]);
            (g0, f32x8::from(*g[1]) - g0)
        };
        let (gm0, dgm) = gain([&c.mix[0].mono, &c.mix[1].mono]);
        let (gl0, dgl) = gain([&c.mix[0].left, &c.mix[1].left]);
        let (gr0, dgr) = gain([&c.mix[0].right, &c.mix[1].right]);

        let (mut phase, mut y1, mut y2) = (
            f32x8::from(self.phase),
            f32x8::from(self.y1),
            f32x8::from(self.y2),
        );
        for i in 0..N {
            let w = Ramp::<N>::W[i];
            let wv = f32x8::splat(w);
            let mut p = phase;
            if !direct {
                p += f32x8::splat(c.off[i]);
                if feedback_on {
                    let mean = half * (y1 + y2);
                    p += f32x8::splat(b0 + db * w) * mean;
                }
                p = wrap01_lanes(p);
            }
            if warp_on {
                p = if warp_moves {
                    warp_lanes(c.warp, &c.warp.prep(x0 + dx * w), p)
                } else {
                    warp_lanes(c.warp, &held, p)
                };
            }
            let mut y = read(c.data, &c.new, c.frames, w, p);
            if FADE {
                let o = read(c.data, &c.old, c.frames, w, p);
                y = c.fade.select(o + (y - o) * wv, y);
            }
            if M {
                mid[i] = hsum(y * (gm0 + dgm * wv));
            }
            if S {
                left[i] = hsum(y * (gl0 + dgl * wv));
                right[i] = hsum(y * (gr0 + dgr * wv));
            }
            y2 = y1;
            y1 = y;
            phase += f0 + df * wv;
            phase = phase.simd_ge(one).select(phase - one, phase);
        }
        self.phase = phase.to_array();
        self.y1 = y1.to_array();
        self.y2 = y2.to_array();
    }
}

/// What the sample loop reads, worked out once per segment.
struct Ctx<'a, const N: usize> {
    data: &'a [f32],
    warp: Warp,
    x: [f32; 2],
    off: &'a [f32; N],
    /// Phase modulation is routed in this segment (else `off` is all zero).
    pm: bool,
    new: Rows,
    old: Rows,
    /// All ones in the lanes that cross-fade from an old mip level.
    fade: f32x8,
    frames: Frames,
    dt: [[f32; LANES]; 2],
    feedback: [f32; 2],
    mix: &'a [Mix; 2],
}

/// A lane mask (all ones where `on`).
fn mask_of(on: [bool; LANES]) -> f32x8 {
    let mut bits = [0.0f32; LANES];
    for (b, o) in bits.iter_mut().zip(on) {
        if o {
            *b = f32::from_bits(u32::MAX);
        }
    }
    f32x8::from(bits)
}

// ---------------------------------------------------------------------------------------------------------------
// The sum
// ---------------------------------------------------------------------------------------------------------------

/// The lanes of `products` added in lane order from lane 0, starting from `+0.0`: the fixed horizontal sum. The lanes
/// beyond the copies are `+0.0` and add nothing (see the module docs).
#[inline(always)]
fn hsum(products: f32x8) -> f32 {
    let a = products.to_array();
    (((((((0.0 + a[0]) + a[1]) + a[2]) + a[3]) + a[4]) + a[5]) + a[6]) + a[7]
}

// ---------------------------------------------------------------------------------------------------------------
// Lane versions of the scalar steps of `Voice::run`
// ---------------------------------------------------------------------------------------------------------------

/// `wrap01` on every lane: `p` folded into `[0, 1)`, NaN and the rounding case `p − ⌊p⌋ = 1` mapped to 0.
#[inline(always)]
fn wrap01_lanes(p: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let in01 = p.simd_ge(zero) & p.simd_lt(one);
    if in01.all() {
        return p;
    }
    let above = p.simd_ge(one) & p.simd_lt(f32x8::splat(2.0));
    let below = p.simd_ge(f32x8::splat(-1.0)) & p.simd_lt(zero);
    // `⌊p⌋` is only wanted by a lane outside `[-1, 2)`, whose `p − ⌊p⌋` the selects below keep; when every lane is
    // inside, the floor (a library call per lane on the baseline x86-64 target) is not computed at all, and the
    // lanes that need it get an exact floor made of a truncation (see `floor_lanes`).
    let r = if (in01 | above | below).all() {
        below.select(p + one, p)
    } else {
        below.select(p + one, p - floor_lanes(p))
    };
    let r = above.select(p - one, r);
    let r = in01.select(p, r);
    (r.simd_ge(zero) & r.simd_lt(one)).select(r, zero)
}

/// `⌊p⌋` on every lane, exactly: `p.floor()` is a library call per lane on the baseline x86-64 target, so lanes
/// within `i32` range take the truncation (`trunc_int`, exact there) converted back (exact, as an integer of that
/// size that came from a float is representable) and lowered by one where it rounded up (a negative non-integer).
/// Beyond that range, or for NaN, the plain `floor`. The one difference is the sign of a zero (`⌊−0.0⌋` is `−0.0`
/// here, `+0.0` by the truncation); the caller only keeps `p − ⌊p⌋` for lanes outside `[-1, 2)`, which are never zero.
#[inline(always)]
fn floor_lanes(p: f32x8) -> f32x8 {
    let limit = f32x8::splat((1u32 << 30) as f32);
    if p.abs().simd_lt(limit).all() {
        let t = p.trunc_int().round_float();
        t - (t.simd_gt(p) & f32x8::ONE)
    } else {
        p.floor()
    }
}

/// `Warp::apply` on every lane (`p` is in `[0, 1)`; the result never reaches 1).
#[inline(always)]
fn warp_lanes(warp: Warp, c: &Prep, p: f32x8) -> f32x8 {
    let below_one = f32x8::splat(BELOW_ONE);
    match warp {
        Warp::Off => p,
        Warp::Bend => p / (p + f32x8::splat(c.a) * (f32x8::ONE - p)),
        Warp::Asymmetric => {
            let first = p.simd_lt(f32x8::splat(c.a));
            first.select(
                p * f32x8::splat(c.b),
                f32x8::splat(0.5) + (p - f32x8::splat(c.a)) * f32x8::splat(c.c),
            )
        }
        Warp::Squeeze => {
            if c.b >= 0.0 {
                (p * f32x8::splat(c.a)).min(below_one)
            } else {
                ((p - f32x8::splat(c.c)) * f32x8::splat(c.a)).max(f32x8::ZERO)
            }
        }
        Warp::Sync => wrap01_lanes(p * f32x8::splat(c.a)),
        Warp::Quantize => ((p * f32x8::splat(c.a)).floor() * f32x8::splat(c.b)).min(below_one),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The table read: a gather of four taps per lane, then the Hermite arithmetic on all lanes
// ---------------------------------------------------------------------------------------------------------------

/// Where the copies read, per lane: the row of the lane's mip level inside a frame, its length and index mask.
#[derive(Clone, Copy)]
struct Rows {
    /// The offset of each lane's row inside a frame.
    off: i32x8,
    mask: i32x8,
    len: f32x8,
}

/// The row offset of a lane beyond the copies: far outside any table, so a read there finds nothing and gives zeros.
const DEAD_OFFSET: i32 = 1 << 30;

impl Rows {
    fn new(levels: &[usize; LANES], count: usize) -> Self {
        let mut off = [0i32; LANES];
        let mut mask = [0i32; LANES];
        let mut len = [0.0f32; LANES];
        for k in 0..LANES {
            let l = levels[k].min(LEVELS - 1);
            off[k] = if k < count {
                OFFSET[l] as i32
            } else {
                DEAD_OFFSET
            };
            mask[k] = (LEN[l] - 1) as i32;
            len[k] = LEN[l] as f32;
        }
        Self {
            off: i32x8::from(off),
            mask: i32x8::from(mask),
            len: f32x8::from(len),
        }
    }

    /// Where every lane reads, as an index inside a frame, and the fraction between taps (`Cursor::locate`): the
    /// truncated position in the lane's row (`trunc_int` is exact and the position is never negative or NaN, so it is
    /// the scalar `p as usize`), masked to the row and moved to the row's place in the frame, and what is left of
    /// the position.
    #[inline(always)]
    fn locate(&self, phase: f32x8) -> (i32x8, f32x8) {
        let p = phase * self.len;
        let i = p.trunc_int() & self.mask;
        (i + self.off, p - i.round_float())
    }
}

/// Which frames a segment reads (`Reader` in the scalar code): on one frame, between two, or with the position
/// moving during the segment.
#[derive(Clone, Copy)]
enum Frames {
    Frame(usize),
    Between(usize, f32),
    Moving { p0: f32, dp: f32 },
}

impl Frames {
    fn new(frame_pos: [f32; 2]) -> Self {
        if frame_pos[0].to_bits() == frame_pos[1].to_bits() {
            let (fi, weight) = frame_of(frame_pos[1]);
            if weight == 0.0 {
                Frames::Frame(fi)
            } else {
                Frames::Between(fi, weight)
            }
        } else {
            Frames::Moving {
                p0: frame_pos[0],
                dp: frame_pos[1] - frame_pos[0],
            }
        }
    }
}

/// The lower frame and the cross-fade weight at a frame position (`cursor_in`).
#[inline(always)]
fn frame_of(frame_pos: f32) -> (usize, f32) {
    let fp = frame_pos.max(0.0).min((FRAMES - 1) as f32);
    let fi = (fp as usize).min(FRAMES - 1);
    (fi, fp - fi as f32)
}

/// Every lane's value at `phase` (`Reader::read`).
#[inline(always)]
fn read(data: &[f32], rows: &Rows, frames: Frames, w: f32, phase: f32x8) -> f32x8 {
    let (at, fr) = rows.locate(phase);
    match frames {
        Frames::Frame(fi) => gather(data, fi, at, fr),
        Frames::Between(fi, weight) => between(data, at, fr, fi, weight),
        Frames::Moving { p0, dp } => {
            let (fi, weight) = frame_of(p0 + dp * w);
            between(data, at, fr, fi, weight)
        }
    }
}

/// The cross-fade of frame `fi` and the next at `weight`: `a + (b − a)·weight`.
#[inline(always)]
fn between(data: &[f32], at: i32x8, fr: f32x8, fi: usize, weight: f32) -> f32x8 {
    let a = gather(data, fi, at, fr);
    let b = gather(data, (fi + 1).min(FRAMES - 1), at, fr);
    a + (b - a) * f32x8::splat(weight)
}

/// The Hermite read of `frame` at every lane's index (`at`, inside a frame) and fraction.
#[inline(always)]
fn gather(data: &[f32], frame: usize, at: i32x8, fr: f32x8) -> f32x8 {
    let at = (at + i32x8::splat((frame * FRAME_STRIDE) as i32)).to_array();
    // Each copy's four taps are contiguous: one 16-byte load per lane, then two 4x4 transposes (pure data movement)
    // turn the eight rows of taps into the four vectors of taps `xm1, x0, x1, x2` that the arithmetic wants. All
    // eight lanes are loaded, in straight code; a lane beyond the copies has its index (`Rows::new`) outside the
    // table, so it reads zeros, as a skipped lane's taps always were.
    let tap = |k: usize| {
        data.get(at[k] as u32 as usize..)
            .and_then(<[f32]>::first_chunk::<4>)
            .map_or(f32x4::ZERO, |&chunk| f32x4::from(chunk))
    };
    let lo = f32x4::transpose([tap(0), tap(1), tap(2), tap(3)]);
    let hi = f32x4::transpose([tap(4), tap(5), tap(6), tap(7)]);
    // The arithmetic runs on the two halves as the transposes leave them (an `f32x8` is two `f32x4` on the baseline
    // target, so this is the same instructions), and the halves are joined once, at the end.
    let f = fr.to_array();
    let y_lo = hermite_lanes(
        lo[0],
        lo[1],
        lo[2],
        lo[3],
        f32x4::from([f[0], f[1], f[2], f[3]]),
    );
    let y_hi = hermite_lanes(
        hi[0],
        hi[1],
        hi[2],
        hi[3],
        f32x4::from([f[4], f[5], f[6], f[7]]),
    );
    let (l, h) = (y_lo.to_array(), y_hi.to_array());
    f32x8::from([l[0], l[1], l[2], l[3], h[0], h[1], h[2], h[3]])
}

/// What [`hermite_lanes`] needs of a vector type.
trait Lanes32:
    Copy + core::ops::Add<Output = Self> + core::ops::Sub<Output = Self> + core::ops::Mul<Output = Self>
{
    fn splat(x: f32) -> Self;
}

impl Lanes32 for f32x4 {
    #[inline(always)]
    fn splat(x: f32) -> Self {
        f32x4::splat(x)
    }
}

impl Lanes32 for f32x8 {
    #[inline(always)]
    fn splat(x: f32) -> Self {
        f32x8::splat(x)
    }
}

/// `wavetable::hermite` on every lane, the same operations in the same order.
#[inline(always)]
fn hermite_lanes<T: Lanes32>(xm1: T, x0: T, x1: T, x2: T, fr: T) -> T {
    let half = T::splat(0.5);
    let c = (x1 - xm1) * half;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * half;
    let b_neg = w + a;
    (((a * fr) - b_neg) * fr + c) * fr + x0
}

#[cfg(test)]
mod tests {
    use super::super::{Table, Voice, WtKind};
    use super::*;
    use crate::util::rng::Rng;
    use crate::wavetable::alias_limit;

    const SR: f32 = 48_000.0;
    const N: usize = 32;

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// The scalar reference: copy `k` is a plain [`Voice`] with its own increments, and each channel is
    /// `Σ y_k · (c0 + (c1 − c0)·RAMP)` added in lane order from lane 0.
    fn reference(
        voices: &mut [Voice; LANES],
        s: &UnisonSegment<'_, N>,
    ) -> ([f32; N], [f32; N], [f32; N]) {
        let count = s.mix[1].count;
        let mut y = [[0.0f32; N]; LANES];
        for k in 0..count {
            let seg = Segment {
                dt: [
                    clamp_dt(s.seg.dt[0] * s.mix[0].ratio[k], MAX_DT_PHASED),
                    clamp_dt(s.seg.dt[1] * s.mix[1].ratio[k], MAX_DT_PHASED),
                ],
                ..s.seg
            };
            voices[k].render(&seg, &mut y[k]);
        }
        let sum = |g0: &[f32; LANES], g1: &[f32; LANES]| {
            let mut out = [0.0f32; N];
            for (i, o) in out.iter_mut().enumerate() {
                let w = Ramp::<N>::W[i];
                let mut acc = 0.0f32;
                for k in 0..count {
                    acc += y[k][i] * (g0[k] + (g1[k] - g0[k]) * w);
                }
                *o = acc;
            }
            out
        };
        (
            sum(&s.mix[0].mono, &s.mix[1].mono),
            sum(&s.mix[0].left, &s.mix[1].left),
            sum(&s.mix[0].right, &s.mix[1].right),
        )
    }

    /// A random segment: a pitch anywhere in the audible range (so the mip level changes often), a position that is
    /// on a frame, between two or moving, a warp, phase modulation, feedback and a mix.
    struct Plan {
        rng: Rng,
        warp: Warp,
        pm: bool,
        feedback: bool,
        moving: bool,
        pm_input: [f32; N],
    }

    impl Plan {
        fn segment<'a>(&'a mut self, table: &'a Table, count: usize) -> UnisonSegment<'a, N> {
            let r = &mut self.rng;
            let dt0 = r.range_f32(0.0004, 0.2);
            let dt1 = if r.unit_f32() < 0.5 {
                dt0
            } else {
                dt0 * r.range_f32(0.8, 1.25)
            };
            let pos = r.range_f32(0.0, 12.0);
            let frame_pos = match (self.moving, r.below(3)) {
                (true, _) => [pos, (pos + r.range_f32(-2.0, 2.0)).clamp(0.0, 12.0)],
                (false, 0) => [pos.floor(); 2],
                (false, _) => [pos; 2],
            };
            let amt0 = if r.unit_f32() < 0.2 {
                0.0
            } else {
                r.range_f32(-1.0, 1.0)
            };
            let amt1 = if r.unit_f32() < 0.5 {
                amt0
            } else {
                r.range_f32(-1.0, 1.0)
            };
            for x in &mut self.pm_input {
                *x = r.range_f32(-1.0, 1.0);
            }
            let pm_gain = r.range_f32(0.0, 0.6);
            let fb = r.range_f32(0.0, 0.3);
            let mix = |r: &mut Rng| {
                Mix::new(
                    count,
                    r.range_f32(0.0, 100.0),
                    r.range_f32(0.0, 1.0),
                    r.range_f32(0.0, 1.0),
                )
            };
            let m0 = mix(r);
            let m1 = if r.unit_f32() < 0.5 { m0 } else { mix(r) };
            UnisonSegment {
                seg: Segment {
                    table,
                    limit: alias_limit(SR),
                    dt: [dt0, dt1],
                    frame_pos,
                    warp: self.warp,
                    warp_amount: [amt0, amt1],
                    pm: self.pm.then_some(&self.pm_input),
                    pm_gain: [pm_gain, pm_gain * 0.7],
                    feedback: if self.feedback {
                        [fb, fb * 1.2]
                    } else {
                        [0.0, 0.0]
                    },
                },
                mix: [m0, m1],
            }
        }
    }

    fn start_phases(rng: &mut Rng) -> [f32; LANES] {
        let mut p = [0.0; LANES];
        for x in &mut p {
            *x = rng.unit_f32();
        }
        p
    }

    /// Render `segments` random segments through the lanes (all three outputs) and through the reference, and demand
    /// the same bits from every channel in every segment.
    fn lanes_equal_the_reference(
        kind: WtKind,
        plan: &mut Plan,
        count: usize,
        segments: usize,
        label: &str,
    ) -> (usize, usize) {
        let table = Table::shared(kind);
        let phases = start_phases(&mut plan.rng);
        let mut lanes = UnisonVoice::new(phases);
        let mut voices = phases.map(Voice::new);
        let (mut level_changes, mut sounding) = (0, 0);
        for n in 0..segments {
            let before: Vec<_> = (0..count).map(|k| lanes.level(k)).collect();
            let s = plan.segment(&table, count);
            let (mut mid, mut left, mut right) = ([0.0; N], [0.0; N], [0.0; N]);
            lanes.render(
                &s,
                Out::Stereo {
                    left: &mut left,
                    right: &mut right,
                    mid: Some(&mut mid),
                },
            );
            let (rm, rl, rr) = reference(&mut voices, &s);
            sounding += usize::from(mid.iter().any(|&x| x != 0.0));
            assert_eq!(bits(&mid), bits(&rm), "{label}: segment {n}: mono mix");
            assert_eq!(bits(&left), bits(&rl), "{label}: segment {n}: left");
            assert_eq!(bits(&right), bits(&rr), "{label}: segment {n}: right");
            for k in 0..count {
                assert_eq!(
                    lanes.level(k),
                    voices[k].level(),
                    "{label}: segment {n}: level {k}"
                );
                assert_eq!(
                    lanes.phase(k).to_bits(),
                    voices[k].phase().to_bits(),
                    "{label}: segment {n}: phase {k}"
                );
                if before[k].is_some() && lanes.level(k) != before[k] {
                    level_changes += 1;
                }
            }
        }
        (level_changes, sounding)
    }

    #[test]
    fn the_lanes_equal_plain_voices_bit_for_bit_for_every_warp_count_and_modulation() {
        let warps = [
            Warp::Off,
            Warp::Bend,
            Warp::Asymmetric,
            Warp::Squeeze,
            Warp::Sync,
            Warp::Quantize,
        ];
        let kinds = [WtKind::Basic, WtKind::Fm, WtKind::Formant, WtKind::Metal];
        let mut case = 0u64;
        let (mut changes, mut sounding) = (0, 0);
        for (wi, &warp) in warps.iter().enumerate() {
            for count in 2..=LANES {
                for (pm, feedback, moving) in [
                    (false, false, false),
                    (true, false, false),
                    (false, true, true),
                    (true, true, true),
                ] {
                    case += 1;
                    let mut plan = Plan {
                        rng: Rng::new(0x1234_0000 + case),
                        warp,
                        pm,
                        feedback,
                        moving,
                        pm_input: [0.0; N],
                    };
                    let (c, s) = lanes_equal_the_reference(
                        kinds[(wi + count) % kinds.len()],
                        &mut plan,
                        count,
                        24,
                        &format!("{warp:?} x{count} pm={pm} fb={feedback} moving={moving}"),
                    );
                    changes += c;
                    sounding += s;
                }
            }
        }
        assert!(case >= 6 * 7 * 4);
        // A vacuous pass would prove nothing: the cross-fade path ran, and the voices sounded.
        assert!(changes >= 300, "{changes} mip-level changes");
        assert!(
            sounding >= 3500,
            "{sounding} sounding segments of {}",
            case * 24
        );
    }

    #[test]
    fn a_pitch_that_jumps_across_mip_levels_cross_fades_in_the_lanes_as_in_a_voice() {
        // dt steps up and down by octaves every segment: every segment changes some copy's level.
        let table = Table::shared(WtKind::Basic);
        let mut rng = Rng::new(7);
        let phases = start_phases(&mut rng);
        let mut lanes = UnisonVoice::new(phases);
        let mut voices = phases.map(Voice::new);
        let mut faded = 0;
        for n in 0..40 {
            let dt = [0.002, 0.02, 0.2, 0.01][n % 4];
            let mix = Mix::new(5, 60.0, 0.7, 0.8);
            let s = UnisonSegment {
                seg: Segment {
                    table: &table,
                    limit: alias_limit(SR),
                    dt: [dt, dt],
                    frame_pos: [4.5, 4.5],
                    warp: Warp::Off,
                    warp_amount: [0.0, 0.0],
                    pm: None,
                    pm_gain: [0.0, 0.0],
                    feedback: [0.0, 0.0],
                },
                mix: [mix, mix],
            };
            let before: Vec<_> = (0..5).map(|k| lanes.level(k)).collect();
            let (mut l, mut r) = ([0.0; N], [0.0; N]);
            lanes.render(
                &s,
                Out::Stereo {
                    left: &mut l,
                    right: &mut r,
                    mid: None,
                },
            );
            let (_, rl, rr) = reference(&mut voices, &s);
            assert_eq!(bits(&l), bits(&rl), "segment {n}: left");
            assert_eq!(bits(&r), bits(&rr), "segment {n}: right");
            faded += (0..5)
                .filter(|&k| before[k].is_some() && lanes.level(k) != before[k])
                .count();
        }
        assert!(faded >= 8, "{faded} level changes");
    }

    #[test]
    fn the_mono_only_and_stereo_only_outputs_are_the_same_sums() {
        let table = Table::shared(WtKind::Formant);
        let mut rng = Rng::new(11);
        let phases = start_phases(&mut rng);
        let mut a = UnisonVoice::new(phases);
        let mut b = UnisonVoice::new(phases);
        let mut c = UnisonVoice::new(phases);
        let mut plan = Plan {
            rng: Rng::new(5),
            warp: Warp::Bend,
            pm: true,
            feedback: true,
            moving: false,
            pm_input: [0.0; N],
        };
        for n in 0..20 {
            let s = plan.segment(&table, 6);
            let (mut m1, mut l1, mut r1) = ([0.0; N], [0.0; N], [0.0; N]);
            a.render(
                &s,
                Out::Stereo {
                    left: &mut l1,
                    right: &mut r1,
                    mid: Some(&mut m1),
                },
            );
            let mut m2 = [0.0; N];
            b.render(&s, Out::Mono(&mut m2));
            let (mut l3, mut r3) = ([0.0; N], [0.0; N]);
            c.render(
                &s,
                Out::Stereo {
                    left: &mut l3,
                    right: &mut r3,
                    mid: None,
                },
            );
            assert_eq!(bits(&m1), bits(&m2), "segment {n}: mono");
            assert_eq!(bits(&l1), bits(&l3), "segment {n}: left");
            assert_eq!(bits(&r1), bits(&r3), "segment {n}: right");
        }
    }

    #[test]
    fn spread_zero_gives_the_mono_mix_on_both_channels() {
        for count in 2..=LANES {
            for detune in [0.0, 7.5, 33.0, 100.0] {
                for blend in [0.0, 0.3, 1.0] {
                    let m = Mix::new(count, detune, blend, 0.0);
                    assert_eq!(bits(&m.left), bits(&m.mono), "left x{count}");
                    assert_eq!(bits(&m.right), bits(&m.mono), "right x{count}");
                }
            }
        }
        // And through the lanes, at a spread that is exactly zero at one end and moving to zero at the other.
        let table = Table::shared(WtKind::Basic);
        let mut v = UnisonVoice::new([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]);
        let m = Mix::new(7, 25.0, 0.8, 0.0);
        let s = UnisonSegment {
            seg: Segment {
                table: &table,
                limit: alias_limit(SR),
                dt: [0.01, 0.011],
                frame_pos: [3.0, 3.0],
                warp: Warp::Off,
                warp_amount: [0.0, 0.0],
                pm: None,
                pm_gain: [0.0, 0.0],
                feedback: [0.0, 0.0],
            },
            mix: [m, m],
        };
        let (mut mid, mut l, mut r) = ([0.0; N], [0.0; N], [0.0; N]);
        v.render(
            &s,
            Out::Stereo {
                left: &mut l,
                right: &mut r,
                mid: Some(&mut mid),
            },
        );
        assert_eq!(bits(&l), bits(&mid));
        assert_eq!(bits(&r), bits(&mid));
        assert!(mid.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn the_mix_is_normalised_symmetric_and_pans_to_the_edges() {
        for count in 2..=LANES {
            for (blend, spread) in [(1.0, 1.0), (0.5, 0.4), (0.0, 1.0), (1.0, 0.0)] {
                let m = Mix::new(count, 40.0, blend, spread);
                let n = count;
                // Level: the squared mono gains add up to 1.
                let power: f32 = m.mono[..n].iter().map(|g| g * g).sum();
                assert!(
                    (power - 1.0).abs() < 1e-6,
                    "x{count} blend {blend}: power {power}"
                );
                // Unused lanes are silent and untuned.
                for k in n..LANES {
                    assert_eq!(
                        (m.mono[k], m.left[k], m.right[k], m.ratio[k]),
                        (0.0, 0.0, 0.0, 1.0)
                    );
                }
                for k in 0..n {
                    let j = n - 1 - k;
                    // The right half mirrors the left exactly.
                    assert_eq!(
                        m.left[k].to_bits(),
                        m.right[j].to_bits(),
                        "x{count} mirror {k}"
                    );
                    assert_eq!(
                        m.mono[k].to_bits(),
                        m.mono[j].to_bits(),
                        "x{count} weight {k}"
                    );
                    // Detune is symmetric in cents: the ratios multiply to one.
                    assert!(
                        (m.ratio[k] * m.ratio[j] - 1.0).abs() < 2e-7,
                        "x{count} ratio {k}"
                    );
                    // Constant power per copy, scaled to 1 at the centre.
                    let p = m.left[k] * m.left[k] + m.right[k] * m.right[k];
                    let want = 2.0 * m.mono[k] * m.mono[k];
                    assert!(
                        (p - want).abs() < 1e-6,
                        "x{count} copy {k}: power {p} vs {want}"
                    );
                }
                // The lowest copy is the leftmost.
                assert!(m.left[0] >= m.right[0]);
            }
            // The outermost copies sit at ±detune and, at 100 % spread, hard left and hard right.
            let m = Mix::new(count, 100.0, 1.0, 1.0);
            assert!((m.ratio[count - 1] - (math::exp2(100.0f64 / 1200.0) as f32)).abs() < 2e-7);
            assert!((m.ratio[0] - (math::exp2(-100.0f64 / 1200.0) as f32)).abs() < 2e-7);
            assert_eq!(m.right[0], 0.0);
            assert_eq!(m.left[count - 1], 0.0);
        }
    }

    #[test]
    fn blend_scales_the_side_copies_against_the_centre() {
        // Odd counts have a centre copy; blend 0 is the centre alone.
        for count in [3, 5, 7] {
            let m = Mix::new(count, 30.0, 0.0, 0.5);
            for k in 0..count {
                let want = if k == count / 2 { 1.0 } else { 0.0 };
                assert_eq!(m.mono[k], want, "x{count} copy {k}");
            }
        }
        // Even counts have two middle copies; blend 0 keeps them.
        let m = Mix::new(4, 30.0, 0.0, 0.5);
        assert_eq!(m.mono[0], 0.0);
        assert_eq!(m.mono[3], 0.0);
        assert_eq!(m.mono[1], m.mono[2]);
        assert!((m.mono[1] * m.mono[1] * 2.0 - 1.0).abs() < 1e-6);
        // Two copies have no side: blend does nothing.
        assert_eq!(Mix::new(2, 30.0, 0.0, 0.5), Mix::new(2, 30.0, 1.0, 0.5));
        // Blend 1 is every copy equal.
        let m = Mix::new(7, 30.0, 1.0, 0.5);
        assert!(
            m.mono[..7]
                .iter()
                .all(|&g| g.to_bits() == m.mono[0].to_bits())
        );
    }

    #[test]
    fn the_horizontal_sum_adds_in_lane_order_from_lane_zero_over_the_copies() {
        let all = [1.0e8, 1.0, -1.0e8, 1.0, 0.5, 0.25, 8.0, 3.0];
        for count in 2..=LANES {
            // The lanes beyond the copies are +0.0, as the mix leaves them.
            let mut lanes = [0.0f32; LANES];
            lanes[..count].copy_from_slice(&all[..count]);
            let mut want = 0.0f32;
            for &t in &lanes[..count] {
                want += t;
            }
            assert_eq!(
                hsum(f32x8::from(lanes)).to_bits(),
                want.to_bits(),
                "count {count}"
            );
        }
        // Order matters in f32: 1e8 + 1 - 1e8 is 0, not 1 -- the sum is the left fold, never a tree.
        let mut three = [0.0f32; LANES];
        three[..3].copy_from_slice(&all[..3]);
        assert_eq!(hsum(f32x8::from(three)), 0.0);
        // Every product -0.0: the sum is +0.0 whether or not the unused lanes are added.
        assert_eq!(hsum(f32x8::splat(-0.0)).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn the_wrap_and_the_warps_equal_the_scalar_ones_on_awkward_phases() {
        // wrap01: in range, one cycle out either side, far out, exact edges, -0.0, NaN and infinities.
        let probes = [
            0.0,
            -0.0,
            0.25,
            0.999_999_94,
            1.0,
            1.5,
            1.999_999_9,
            2.0,
            2.5,
            -0.25,
            -1.0,
            -1.5,
            37.3,
            -1.0e9,
            1.0e20,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.5,
            0.75,
            3.0,
            -0.000_000_1,
        ];
        for chunk in probes.chunks(LANES) {
            let mut a = [0.5f32; LANES];
            a[..chunk.len()].copy_from_slice(chunk);
            let got = wrap01_lanes(f32x8::from(a)).to_array();
            for (k, &p) in a.iter().enumerate() {
                assert_eq!(
                    got[k].to_bits(),
                    super::super::wrap01(p).to_bits(),
                    "wrap01({p})"
                );
            }
        }
        // Warps at a spread of phases and amounts.
        let mut rng = Rng::new(99);
        for warp in [
            Warp::Bend,
            Warp::Asymmetric,
            Warp::Squeeze,
            Warp::Sync,
            Warp::Quantize,
        ] {
            for amount in [-1.0, -0.6, -0.1, 0.07, 0.5, 1.0] {
                let c = warp.prep(warp.primary(amount));
                for _ in 0..50 {
                    let mut a = [0.0f32; LANES];
                    for x in &mut a {
                        *x = rng.unit_f32();
                    }
                    a[0] = 0.0;
                    a[1] = 0.999_999_94;
                    let got = warp_lanes(warp, &c, f32x8::from(a)).to_array();
                    for (k, &p) in a.iter().enumerate() {
                        assert_eq!(
                            got[k].to_bits(),
                            warp.apply(&c, p).to_bits(),
                            "{warp:?} {amount} at {p}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_state_stays_finite_and_unflushed_values_never_reach_subnormals() {
        let table = Table::shared(WtKind::Fm);
        let mut v = UnisonVoice::new([0.0; LANES]);
        let m = Mix::new(8, 100.0, 1.0, 1.0);
        let mut s = UnisonSegment {
            seg: Segment {
                table: &table,
                limit: alias_limit(SR),
                dt: [0.01, 0.01],
                frame_pos: [6.0, 6.0],
                warp: Warp::Off,
                warp_amount: [0.0, 0.0],
                pm: None,
                pm_gain: [0.0, 0.0],
                feedback: [0.3, 0.3],
            },
            mix: [m, m],
        };
        let (mut l, mut r) = ([0.0; N], [0.0; N]);
        for n in 0..4000 {
            // Falling pitch: the tracker releases through a long decay.
            let dt = if n < 100 { 0.2 } else { 0.0005 };
            s.seg.dt = [dt, dt];
            v.render(
                &s,
                Out::Stereo {
                    left: &mut l,
                    right: &mut r,
                    mid: None,
                },
            );
            assert!(l.iter().chain(&r).all(|x| x.is_finite()));
            assert!(!v.has_subnormal(), "segment {n}");
        }
    }
}
