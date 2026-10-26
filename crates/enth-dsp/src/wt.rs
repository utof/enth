//! The DSP core of `wt` (S2a lane WT, `docs/dsp/wt.md`): generated wavetables, phase warps, and a phase-read
//! oscillator voice with phase modulation and self-feedback. Everything is `f32` `+ − × ÷` and comparisons per sample
//! (no transcendentals, no `mul_add`, no allocation), so the stream is bit-identical on every platform.
//!
//! # The tables
//!
//! Five tables, generated in code (no audio assets), in the layout of [`crate::wavetable`]: [`FRAMES`] = 13 frames
//! of [`LEVELS`] = 9 mip levels, level `l` holding the harmonics `1 ..= 256 >> l` at `32 ×` its top harmonic samples
//! per cycle (at least 1,024), three guard samples per level, 4-point Hermite interpolation in phase and a linear
//! cross-fade between the two frames around the position (about 1.07 MB per table). The generator is the same code
//! (`wavetable::build`): exact integer-indexed additive synthesis in `f64`, harmonics summed in ascending order, every
//! frame peak-normalised at level 0 and the other levels sharing its scale, rounded once to `f32`. A frame is a
//! formula over harmonic amplitudes:
//!
//! | Table | Frame `pos ∈ [0, 1]` |
//! | --- | --- |
//! | `basic` | The `osc` basic table: sine → triangle → saw → square in three equal blends |
//! | `fm` | `sin(x + I·sin(x))`, `I = 10·pos`: Chowning's two-operator FM at 1:1, the harmonic `k` having amplitude `J(k−1)(I) + (−1)^k J(k+1)(I)` |
//! | `fm_octave` | `sin(x + I·sin(2x))`, `I = 10·pos`: the modulator an octave up, odd harmonics only |
//! | `formant` | A saw (`−1/k`) times `1 + 5/(1 + ((k − c)/w)²)`: one resonant peak at harmonic `c = 32^pos` (1 … 32), half-width `w = max(0.75, 0.15·c)` |
//! | `metal` | 24 partials at harmonics 1, 3, 5, 8, 11 … 224 (spacing growing by one every few partials) at `1/√(1+j)`, quadratic-residue phases; `3 + 21·pos` of them sound, the last one fading in |
//!
//! The Bessel functions of the first kind come from their power series in `f64` (fixed loops, `+ − × ÷` only), so
//! the data is the same on every platform; the unit tests pin one FNV-1a fingerprint per table.
//!
//! # The voice
//!
//! [`Voice::render`] reads the table at `warp(frac(phase + pm + feedback))` per sample, where
//!
//! - `phase` is the oscillator's own accumulator (cycles, `[0, 1)`, the increment ramping linearly over the
//!   segment), and the output at the first sample is the table at the start phase, as `osc`'s sine and table;
//! - `pm` is the phase modulation in cycles: the routed audio input times a gain (`pm_index · pm/100 / 2π`) that
//!   also ramps over the segment;
//! - `feedback` is the oscillator's own output delayed by 1.5 samples (the mean of the last two samples, report 05
//!   §1: the average stops the loop hunting) times a gain of up to 2 radians (`feedback/100 · 2`);
//! - `warp` bends the read phase ([`Warp`]); every warp is the identity at amount 0 (the module reads the table
//!   straight, so the bits equal an unwarped read).
//!
//! Because the table is read by phase (a smooth function of any phase, with the table's own band limit), PM and
//! feedback add no discontinuity; what they add is bandwidth (Carson's `2(I + 1)·f_m`), which is why the **mip level
//! follows the effective phase increment**: `level_for(slope · (dt + max |Δpm|))`, where `Δpm` is the largest step of
//! the phase modulation within the segment and `slope` the warp's steepest read-rate gain. The estimate is a tracked
//! maximum (instant attack, a time constant of about 330 ms release) so a periodic modulator does not make the level
//! flicker as its slope passes through zero, and a change of level cross-fades over the segment, so no step is
//! heard. Feedback does not take part (its slope is the loop's own), so its aliasing is the loop's: measured in
//! `docs/dsp/wt.md`.
//!
//! # Sources
//!
//! Chowning, "The synthesis of complex audio spectra by means of frequency modulation", JAES 21(7), 1973 (phase
//! modulation, the Bessel sidebands); Tomisawa, "Tone production method for an electronic musical instrument", US
//! patent 4,249,447 (1981, expired), and Schottstaedt's FM notes for self-feedback; research report 05 §1 of the S2
//! session for the two-sample average; Kleimola et al., "Phaseshaping oscillator algorithms for musical sound
//! synthesis", SMC 2010, for the phase warps; stmlib's Hermite interpolation is the one `wavetable` already carries
//! (MIT, `THIRD_PARTY_NOTICES.md`). The rest is our own code.

use core::f64::consts::PI;
use std::sync::{Arc, OnceLock};

use crate::util::flush::flush;
use crate::util::math;
use crate::wavetable::{
    self, Cursor, FRAMES, LEVELS, MAX_HARMONIC, TableKind, cursor_in, fingerprint_of, harmonics,
    level_for, row_in,
};

mod unison;
pub use unison::{LANES, Mix, Out, UnisonSegment, UnisonVoice};

/// The largest `f32` below 1.
const BELOW_ONE: f32 = 0.999_999_94;
/// The index of the last frame of the `fm` tables, in radians.
const FM_INDEX_MAX: f64 = 10.0;
/// `1 / 2π`: radians to cycles.
pub const CYCLES_PER_RADIAN: f32 = 0.159_154_94;
/// The phase swing of 100 % feedback, in cycles per unit of output: 2 radians. Linearised where the table's slope is
/// `s` per cycle, the loop `y = table(φ + g·(y₁ + y₂)/2)` is `z² = (a/2)(z + 1)` with `a = g·s`; where the table falls
/// (`a < 0`) its roots form a complex pair of modulus `√(−a/2)`, a ring at a third of the sample rate that grows once
/// `a < −2`. A sine's steepest slope is `2π`, so 2 radians is the most swing at which a sine never rings, and there it
/// is still within 1.5–3 dB of a saw's `1/n` slope (`docs/dsp/wt.md`); a brighter table, being steeper, rings sooner.
pub const FEEDBACK_CYCLES: f32 = 2.0 * CYCLES_PER_RADIAN;
/// The tracked phase increment falls by this factor per segment (a 330 ms time constant at 48 kHz).
const TRACK_DECAY: f32 = 0.998;
/// `ln 8`: the bend warp's steepest slope at amount ±1 is 8.
const LN_8: f64 = 2.079_441_541_679_835_7;

// ---------------------------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------------------------

/// The five generated tables, in the catalogue's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WtKind {
    /// Sine, triangle, saw, square.
    Basic,
    /// Two-operator FM at 1:1, the index rising across the table.
    Fm,
    /// The same with the modulator an octave up.
    FmOctave,
    /// A saw through one resonant peak that sweeps up the harmonics.
    Formant,
    /// A few strong, widely spaced partials; position adds density.
    Metal,
}

impl WtKind {
    /// Every table, in catalogue order.
    pub const ALL: [WtKind; 5] = [
        WtKind::Basic,
        WtKind::Fm,
        WtKind::FmOctave,
        WtKind::Formant,
        WtKind::Metal,
    ];

    /// Position in [`WtKind::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The catalogue key.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            WtKind::Basic => "basic",
            WtKind::Fm => "fm",
            WtKind::FmOctave => "fm_octave",
            WtKind::Formant => "formant",
            WtKind::Metal => "metal",
        }
    }
}

/// One generated table: [`FRAMES`] frames of [`LEVELS`] levels (see the module docs).
pub struct Table {
    kind: WtKind,
    data: Box<[f32]>,
}

impl core::fmt::Debug for Table {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Table")
            .field("kind", &self.kind)
            .field("frames", &FRAMES)
            .field("levels", &LEVELS)
            .finish_non_exhaustive()
    }
}

impl Table {
    /// Generate `kind` from scratch (tens of milliseconds; allocates). Prefer [`Table::shared`].
    #[must_use]
    pub fn generate(kind: WtKind) -> Self {
        let data = wavetable::build(&mut |pos, s, c| fill(kind, pos, s, c));
        Self { kind, data }
    }

    /// The process-wide copy of `kind`: generated on first use, then shared (a memo of a pure function; nothing on
    /// the audio path touches it).
    #[must_use]
    pub fn shared(kind: WtKind) -> Arc<Table> {
        static CACHE: [OnceLock<Arc<Table>>; 5] = [const { OnceLock::new() }; 5];
        Arc::clone(CACHE[kind.index()].get_or_init(|| Arc::new(Table::generate(kind))))
    }

    /// Which table this is.
    #[must_use]
    pub fn kind(&self) -> WtKind {
        self.kind
    }

    /// One level of one frame: `x(-1), x(0), …, x(N-1), x(N), x(N+1)`.
    #[must_use]
    pub fn row(&self, frame: usize, level: usize) -> &[f32] {
        row_in(&self.data, frame, level)
    }

    /// The two frame rows of `level` around `frame_pos` (`0 ..= FRAMES − 1`, clamped) and their cross-fade weight.
    #[inline(always)]
    #[must_use]
    pub fn cursor(&self, level: usize, frame_pos: f32) -> Cursor<'_> {
        cursor_in(&self.data, level, frame_pos)
    }

    /// FNV-1a (64 bit) over the little-endian bits of every float, in storage order.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        fingerprint_of(&self.data)
    }
}

/// Fill the harmonic coefficients of one frame (`s[k]` of `sin(k·x)`, `c[k]` of `cos(k·x)`).
fn fill(kind: WtKind, pos: f64, s: &mut [f64], c: &mut [f64]) {
    match kind {
        WtKind::Basic => harmonics(TableKind::Basic, pos, s, c),
        WtKind::Fm => fm(1, pos, s),
        WtKind::FmOctave => fm(2, pos, s),
        WtKind::Formant => formant(pos, s),
        WtKind::Metal => metal(pos, s, c),
    }
}

/// `J_n(x)`, the Bessel function of the first kind, from its power series
/// `Σ (−1)^m (x/2)^(2m+n) / (m!(m+n)!)` in `f64` with a fixed number of terms (`x ≤ 10` converges to the `f64` grid
/// well inside 60 terms). Zero where the first term underflows (`n` far above `x`).
#[must_use]
pub fn bessel_j(n: usize, x: f64) -> f64 {
    let h = 0.5 * x;
    if n as f64 > 3.0 * x.abs() + 40.0 {
        return 0.0;
    }
    // (x/2)^n / n!
    let mut term = 1.0f64;
    for i in 1..=n {
        term *= h / i as f64;
    }
    let h2 = h * h;
    let mut sum = term;
    for m in 0..60 {
        term *= -h2 / (((m + 1) * (m + 1 + n)) as f64);
        sum += term;
    }
    sum
}

/// Two-operator FM `sin(x + I·sin(r·x))`, `I = 10·pos`: the carrier's sideband `1 + r·n` has amplitude `J_n(I)`, and
/// a negative frequency `−k` folds onto `+k` with a sign flip. Pure sines.
fn fm(ratio: usize, pos: f64, s: &mut [f64]) {
    let index = FM_INDEX_MAX * pos;
    for (k, sk) in s.iter_mut().enumerate().take(MAX_HARMONIC + 1).skip(1) {
        let mut v = 0.0;
        // n = (k − 1)/r: frequency +k.
        if (k - 1) % ratio == 0 {
            v += bessel_j((k - 1) / ratio, index);
        }
        // n = −(k + 1)/r: frequency −k, `J_{−m} = (−1)^m J_m` and `sin(−kx) = −sin(kx)`.
        if (k + 1) % ratio == 0 {
            let m = (k + 1) / ratio;
            let j = bessel_j(m, index);
            v -= if m.is_multiple_of(2) { j } else { -j };
        }
        *sk = v;
    }
}

/// A saw under one Lorentzian resonance, centred at harmonic `32^pos` (1 … 32), gain 6 at the peak.
fn formant(pos: f64, s: &mut [f64]) {
    let centre = math::pow(32.0, pos);
    let width = (0.15 * centre).max(0.75);
    for (k, sk) in s.iter_mut().enumerate().take(MAX_HARMONIC + 1).skip(1) {
        let kf = k as f64;
        let d = (kf - centre) / width;
        let gain = 1.0 + 5.0 / (1.0 + d * d);
        *sk = -gain / kf;
    }
}

/// The harmonic numbers of `metal`'s partials: 1, 3, 5, then gaps growing from 3 to 20.
const METAL_PARTIALS: [usize; 24] = [
    1, 3, 5, 8, 11, 14, 18, 23, 29, 35, 42, 50, 59, 69, 80, 92, 105, 119, 134, 150, 167, 185, 204,
    224,
];

/// Sparse partials with `3 + 21·pos` sounding: partial `j` has weight `clamp(3 + 21·pos − j, 0, 1)`, amplitude
/// `1/√(1 + j)` and phase `2π·frac(φ·j²)` (`φ` the golden ratio's fraction), so the waveform is not a spike.
fn metal(pos: f64, s: &mut [f64], c: &mut [f64]) {
    let count = 3.0 + 21.0 * pos;
    for (j, &k) in METAL_PARTIALS.iter().enumerate() {
        let weight = (count - j as f64).clamp(0.0, 1.0);
        if weight == 0.0 {
            continue;
        }
        let amp = weight / (1.0 + j as f64).sqrt();
        let x = 0.618_033_988_749_894_9 * (j * j) as f64;
        let phase = 2.0 * PI * (x - x.floor());
        s[k] = amp * math::cos(phase);
        c[k] = amp * math::sin(phase);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Warps
// ---------------------------------------------------------------------------------------------------------------

/// How the read phase is bent before the table is read. Every warp is the identity at amount 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Warp {
    /// Straight.
    Off,
    /// `p / (p + k(1 − p))`, `k = 8^(−amount)`: the detail bunches at the start (positive) or the end (negative).
    Bend,
    /// A knee at `m = ½ − 0.45·amount`: the first half of the cycle plays in `[0, m]`, the second in `[m, 1]`
    /// (pulse-width-like for any wave).
    Asymmetric,
    /// The whole cycle plays in the first (positive) or last (negative) `w = 1 − 0.9|amount|` of the period, and the
    /// rest holds the cycle's end (start) value.
    Squeeze,
    /// `frac(r·p)` with `r = 1 + 7|amount|`: the cycle repeats and restarts, like hard sync.
    Sync,
    /// `⌊N·p⌋/N` with `N = round(256 · 2^(−7|amount|))` steps (256 … 2).
    Quantize,
}

/// A warp's constants at one amount (see [`Warp::prep`]).
#[derive(Clone, Copy, Debug)]
struct Prep {
    a: f32,
    b: f32,
    c: f32,
}

impl Warp {
    /// The primary parameter at `amount ∈ [−1, 1]` (bend: `k`; asymmetric: the knee `m`; squeeze: the signed
    /// amount; sync: the repeat count `r`; quantize: the step count `N`).
    fn primary(self, amount: f32) -> f32 {
        let a = f64::from(amount.clamp(-1.0, 1.0));
        match self {
            Warp::Off => 0.0,
            Warp::Bend => math::exp(-LN_8 * a) as f32,
            Warp::Asymmetric => (0.5 - 0.45 * a) as f32,
            Warp::Squeeze => a as f32,
            Warp::Sync => (1.0 + 7.0 * a.abs()) as f32,
            Warp::Quantize => math::exp2(8.0 - 7.0 * a.abs()).round().max(2.0) as f32,
        }
    }

    /// The constants the per-sample map needs, from the primary parameter `x`.
    #[inline(always)]
    fn prep(self, x: f32) -> Prep {
        match self {
            Warp::Off | Warp::Bend | Warp::Sync => Prep {
                a: x,
                b: 0.0,
                c: 0.0,
            },
            Warp::Asymmetric => Prep {
                a: x,
                b: 0.5 / x,
                c: 0.5 / (1.0 - x),
            },
            Warp::Squeeze => {
                let w = 1.0 - 0.9 * x.abs();
                Prep {
                    a: 1.0 / w,
                    b: x,
                    c: 1.0 - w,
                }
            }
            Warp::Quantize => Prep {
                a: x,
                b: 1.0 / x,
                c: 0.0,
            },
        }
    }

    /// The steepest read-rate gain of the map at primary parameter `x` (the mip level's slope factor).
    fn slope(self, x: f32) -> f32 {
        match self {
            Warp::Off | Warp::Quantize => 1.0,
            Warp::Bend => x.max(1.0 / x),
            Warp::Asymmetric => (0.5 / x).max(0.5 / (1.0 - x)),
            Warp::Squeeze => 1.0 / (1.0 - 0.9 * x.abs()),
            Warp::Sync => x,
        }
    }

    /// Bend phase `p ∈ [0, 1)` (never returns 1).
    #[inline(always)]
    fn apply(self, c: &Prep, p: f32) -> f32 {
        match self {
            Warp::Off => p,
            Warp::Bend => p / (p + c.a * (1.0 - p)),
            Warp::Asymmetric => {
                if p < c.a {
                    p * c.b
                } else {
                    0.5 + (p - c.a) * c.c
                }
            }
            Warp::Squeeze => {
                if c.b >= 0.0 {
                    (p * c.a).min(BELOW_ONE)
                } else {
                    ((p - c.c) * c.a).max(0.0)
                }
            }
            Warp::Sync => wrap01(p * c.a),
            Warp::Quantize => ((p * c.a).floor() * c.b).min(BELOW_ONE),
        }
    }
}

/// `p` folded into `[0, 1)`; NaN and the rounding case `p − ⌊p⌋ = 1` map to 0 (the same point of the cycle). The
/// three neighbouring cycles are one add (what `p − ⌊p⌋` is there, bit for bit), because `⌊·⌋` is a library call on
/// the baseline x86-64 target and a read without modulation never leaves `[0, 1)`.
#[inline(always)]
fn wrap01(p: f32) -> f32 {
    let r = if (0.0..1.0).contains(&p) {
        return p;
    } else if (1.0..2.0).contains(&p) {
        p - 1.0
    } else if (-1.0..0.0).contains(&p) {
        p + 1.0
    } else {
        p - p.floor()
    };
    if (0.0..1.0).contains(&r) { r } else { 0.0 }
}

// ---------------------------------------------------------------------------------------------------------------
// The voice
// ---------------------------------------------------------------------------------------------------------------

/// Linear ramp weights for a block of `N` frames: `(i + 1) / N` (the engine's `RAMP` for `N` = 32).
struct Ramp<const N: usize>;

impl<const N: usize> Ramp<N> {
    const W: [f32; N] = {
        let mut r = [0.0; N];
        let mut i = 0;
        while i < N {
            r[i] = (i + 1) as f32 / N as f32;
            i += 1;
        }
        r
    };
}

/// "No level yet" (a voice that has not rendered).
const NO_LEVEL: u8 = u8::MAX;

/// What a segment renders with. All ramped values are `[start, end]`; the per-sample value is
/// `start + (end − start)·(i + 1)/N`.
#[derive(Clone, Copy, Debug)]
pub struct Segment<'a, const N: usize> {
    /// The table.
    pub table: &'a Table,
    /// [`wavetable::alias_limit`] of the sample rate.
    pub limit: f32,
    /// Phase increment in cycles per sample (already clamped by the caller).
    pub dt: [f32; 2],
    /// Frame position, `0 ..= FRAMES − 1`.
    pub frame_pos: [f32; 2],
    /// The warp.
    pub warp: Warp,
    /// The warp amount, `−1 ..= 1`.
    pub warp_amount: [f32; 2],
    /// The phase-modulation input (the routed audio), `None` when no route reaches the module.
    pub pm: Option<&'a [f32; N]>,
    /// Cycles of phase offset per unit of `pm` input: `pm_index · (pm/100) · `[`CYCLES_PER_RADIAN`].
    pub pm_gain: [f32; 2],
    /// Cycles of phase offset per unit of output: `feedback/100 · `[`FEEDBACK_CYCLES`]; 0 is off.
    pub feedback: [f32; 2],
}

/// One voice: the phase accumulator, the feedback history and the mip-level tracker.
#[derive(Clone, Copy, Debug)]
pub struct Voice {
    phase: f32,
    y1: f32,
    y2: f32,
    /// The last phase-modulation offset, in cycles (for the slope across a segment boundary).
    last_off: f32,
    /// The tracked effective phase increment (cycles per sample), instant attack, slow release.
    tracked: f32,
    /// The mip level of the previous segment.
    level: u8,
}

impl Default for Voice {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// One frame-pair read at a fixed level (or a moving position): the reader of a segment.
#[derive(Clone, Copy)]
enum Reader<'a> {
    /// On a frame: the lower frame alone is the read.
    Frame(Cursor<'a>),
    /// Between two frames at a fixed position.
    Between(Cursor<'a>),
    /// The position moves during the segment.
    Moving {
        table: &'a Table,
        level: usize,
        p0: f32,
        dp: f32,
    },
}

impl<'a> Reader<'a> {
    #[inline(always)]
    fn new(table: &'a Table, level: usize, frame_pos: [f32; 2]) -> Self {
        if frame_pos[0].to_bits() == frame_pos[1].to_bits() {
            let at = table.cursor(level, frame_pos[1]);
            if at.on_frame() {
                Reader::Frame(at)
            } else {
                Reader::Between(at)
            }
        } else {
            Reader::Moving {
                table,
                level,
                p0: frame_pos[0],
                dp: frame_pos[1] - frame_pos[0],
            }
        }
    }

    #[inline(always)]
    fn read(&self, phase: f32, w: f32) -> f32 {
        match self {
            Reader::Frame(c) => c.read_first(phase),
            Reader::Between(c) => c.read(phase),
            Reader::Moving {
                table,
                level,
                p0,
                dp,
            } => table.cursor(*level, *p0 + *dp * w).read(phase),
        }
    }
}

/// What a segment works out before its sample loop, shared by one voice and by each copy of a unison voice
/// ([`unison`]): the phase modulation in cycles and its steepest step, the warp (the identity at amount 0) with its
/// primary parameter at the two ends, and the warp's steepest read-rate gain.
struct Prologue<const N: usize> {
    off: [f32; N],
    pm_slope: f32,
    warp: Warp,
    x: [f32; 2],
    steepest: f32,
}

/// The [`Prologue`] of `s`; `last_off` is the voice's last phase-modulation offset (read, then replaced).
#[inline(always)]
fn prologue<const N: usize>(s: &Segment<'_, N>, last_off: &mut f32) -> Prologue<N> {
    // 1. The phase modulation in cycles, and its steepest step across the segment.
    let mut off = [0.0f32; N];
    let mut pm_slope = 0.0f32;
    if let Some(pm) = s.pm {
        let (g0, dg) = (s.pm_gain[0], s.pm_gain[1] - s.pm_gain[0]);
        let mut last = *last_off;
        for i in 0..N {
            let o = (g0 + dg * Ramp::<N>::W[i]) * pm[i];
            off[i] = o;
            pm_slope = pm_slope.max((o - last).abs());
            last = o;
        }
        *last_off = flush(last);
    } else {
        *last_off = 0.0;
    }

    // 2. The warp's constants at the two ends.
    let warp = if s.warp_amount[0] == 0.0 && s.warp_amount[1] == 0.0 {
        Warp::Off
    } else {
        s.warp
    };
    let x = [
        warp.primary(s.warp_amount[0]),
        warp.primary(s.warp_amount[1]),
    ];
    let steepest = if warp == Warp::Off {
        1.0
    } else {
        warp.slope(x[0]).max(warp.slope(x[1]))
    };
    Prologue {
        off,
        pm_slope,
        warp,
        x,
        steepest,
    }
}

impl Voice {
    /// A voice starting at `phase` (`0 ≤ phase < 1`, else 0).
    #[must_use]
    pub fn new(phase: f32) -> Self {
        let phase = if (0.0..1.0).contains(&phase) {
            phase
        } else {
            0.0
        };
        Self {
            phase,
            y1: 0.0,
            y2: 0.0,
            last_off: 0.0,
            tracked: 0.0,
            level: NO_LEVEL,
        }
    }

    /// The oscillator's own phase in cycles, `[0, 1)` (without the modulation).
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// The mip level the previous segment read (`None` before the first segment).
    #[must_use]
    pub fn level(&self) -> Option<usize> {
        (self.level != NO_LEVEL).then_some(usize::from(self.level))
    }

    /// Whether any state value is subnormal.
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.phase.is_subnormal()
            || self.y1.is_subnormal()
            || self.y2.is_subnormal()
            || self.last_off.is_subnormal()
            || self.tracked.is_subnormal()
    }

    /// Render `N` frames into `out`.
    pub fn render<const N: usize>(&mut self, s: &Segment<'_, N>, out: &mut [f32; N]) {
        // 1 and 2. The phase modulation in cycles and its steepest step, and the warp's constants at the two ends.
        let Prologue {
            off,
            pm_slope,
            warp,
            x,
            steepest,
        } = prologue(s, &mut self.last_off);

        // 3. The mip level from the tracked effective increment.
        let dt_max = s.dt[0].max(s.dt[1]);
        self.tracked = flush((self.tracked * TRACK_DECAY).max(steepest * (dt_max + pm_slope)));
        let level = level_for(self.tracked, s.limit);
        let new = Reader::new(s.table, level, s.frame_pos);
        if self.level == NO_LEVEL || usize::from(self.level) == level {
            self.run::<N, false>(s, warp, x, &off, new, new, out);
        } else {
            let old = Reader::new(s.table, usize::from(self.level), s.frame_pos);
            self.run::<N, true>(s, warp, x, &off, new, old, out);
        }
        self.level = level as u8;
        self.y1 = flush(self.y1);
        self.y2 = flush(self.y2);
    }

    /// The sample loop. `FADE`: cross-fade `old` into `new` over the segment (a change of mip level).
    #[inline(always)]
    #[allow(
        clippy::too_many_arguments,
        reason = "the loop's inputs are all distinct and per segment; a struct would only rename them"
    )]
    fn run<const N: usize, const FADE: bool>(
        &mut self,
        s: &Segment<'_, N>,
        warp: Warp,
        x: [f32; 2],
        off: &[f32; N],
        new: Reader<'_>,
        old: Reader<'_>,
        out: &mut [f32; N],
    ) {
        let (f0, df) = (s.dt[0], s.dt[1] - s.dt[0]);
        let (b0, db) = (s.feedback[0], s.feedback[1] - s.feedback[0]);
        let feedback_on = s.feedback[0] != 0.0 || s.feedback[1] != 0.0;
        let warp_on = warp != Warp::Off;
        // Quantize steps change by whole steps, so the end value holds for the segment; the others ramp.
        let warp_moves = x[0].to_bits() != x[1].to_bits() && warp != Warp::Quantize;
        let (x0, dx) = if warp == Warp::Quantize {
            (x[1], 0.0)
        } else {
            (x[0], x[1] - x[0])
        };
        let held = warp.prep(x[1]);
        let (mut phase, mut y1, mut y2) = (self.phase, self.y1, self.y2);
        for i in 0..N {
            let w = Ramp::<N>::W[i];
            let mut p = phase + off[i];
            if feedback_on {
                #[allow(
                    clippy::manual_midpoint,
                    reason = "0.5·(a + b) is an add and an exact halving on every target; f32::midpoint's code differs by target"
                )]
                let mean = 0.5 * (y1 + y2);
                p += (b0 + db * w) * mean;
            }
            p = wrap01(p);
            if warp_on {
                p = if warp_moves {
                    warp.apply(&warp.prep(x0 + dx * w), p)
                } else {
                    warp.apply(&held, p)
                };
            }
            let mut y = new.read(p, w);
            if FADE {
                let o = old.read(p, w);
                y = o + (y - o) * w;
            }
            out[i] = y;
            y2 = y1;
            y1 = y;
            phase += f0 + df * w;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        self.phase = phase;
        self.y1 = y1;
        self.y2 = y2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::{FFT_LEN, Measure, alias_db, power_spectrum};
    use crate::osc::sine;
    use crate::wavetable::{alias_limit, level_len, top_harmonic};

    const SR: f32 = 48_000.0;
    const N: usize = 32;

    fn rows_spectrum(t: &Table, frame: usize, level: usize) -> Vec<f64> {
        let n = level_len(level);
        let mut re: Vec<f64> = t.row(frame, level)[1..=n]
            .iter()
            .map(|&x| f64::from(x))
            .collect();
        let mut im = vec![0.0; n];
        crate::alias::fft(&mut re, &mut im);
        (0..=n / 2)
            .map(|k| 2.0 * (re[k] * re[k] + im[k] * im[k]).sqrt() / n as f64)
            .collect()
    }

    /// A segment of a table with nothing else on.
    fn plain(t: &Table, dt: f32, pos: f32) -> Segment<'_, N> {
        Segment {
            table: t,
            limit: alias_limit(SR),
            dt: [dt, dt],
            frame_pos: [pos, pos],
            warp: Warp::Off,
            warp_amount: [0.0, 0.0],
            pm: None,
            pm_gain: [0.0, 0.0],
            feedback: [0.0, 0.0],
        }
    }

    fn render(v: &mut Voice, s: &Segment<'_, N>, blocks: usize) -> Vec<f32> {
        let mut x = Vec::with_capacity(blocks * N);
        let mut out = [0.0; N];
        for _ in 0..blocks {
            v.render(s, &mut out);
            x.extend_from_slice(&out);
        }
        x
    }

    #[test]
    fn bessel_matches_scipy() {
        let want: [(f64, [f64; 6]); 3] = [
            (
                1.0,
                [
                    0.765_197_686_557_966_6,
                    0.440_050_585_744_933_55,
                    0.114_903_484_931_900_5,
                    0.019_563_353_982_668_414,
                    0.002_476_638_964_109_955,
                    0.000_249_757_730_211_234_66,
                ],
            ),
            (
                2.5,
                [
                    -0.048_383_776_468_197_89,
                    0.497_094_102_464_274_1,
                    0.446_059_058_439_617_24,
                    0.216_600_391_039_113_58,
                    0.073_781_880_054_255_23,
                    0.019_501_625_134_503_22,
                ],
            ),
            (
                10.0,
                [
                    -0.245_935_764_451_348_24,
                    0.043_472_746_168_861_49,
                    0.254_630_313_685_120_5,
                    0.058_379_379_305_186_73,
                    -0.219_602_686_102_008_48,
                    -0.234_061_528_186_793_54,
                ],
            ),
        ];
        for (x, js) in want {
            for (n, &j) in js.iter().enumerate() {
                let got = bessel_j(n, x);
                assert!((got - j).abs() < 1e-11, "J{n}({x}) = {got}, want {j}");
            }
        }
        assert!((bessel_j(30, 10.0) - 1.551_096_078_257_474_5e-12).abs() < 1e-20);
        assert_eq!(bessel_j(256, 10.0), 0.0);
        assert_eq!(bessel_j(5, 0.0), 0.0);
        assert_eq!(bessel_j(0, 0.0), 1.0);
    }

    #[test]
    fn the_first_frame_of_basic_is_a_sine_with_every_harmonic_below_minus_90_db() {
        let t = Table::generate(WtKind::Basic);
        for level in 0..LEVELS {
            let a = rows_spectrum(&t, 0, level);
            assert!(
                (a[1] - 1.0).abs() < 1e-6,
                "level {level}: fundamental {}",
                a[1]
            );
            for (k, &v) in a.iter().enumerate().skip(2) {
                assert!(
                    v < 3.2e-5,
                    "level {level}: harmonic {k} at {:.1} dB",
                    20.0 * math::log10(v)
                );
            }
            // DC.
            assert!(a[0] < 1e-6);
        }
    }

    #[test]
    fn every_level_holds_only_its_harmonics_and_every_frame_peaks_at_one() {
        for kind in WtKind::ALL {
            let t = Table::generate(kind);
            for frame in [0, 3, 6, 9, 12] {
                for level in 0..LEVELS {
                    let n = level_len(level);
                    let top = top_harmonic(level);
                    let a = rows_spectrum(&t, frame, level);
                    let above = a[top + 1..n / 2].iter().fold(0.0f64, |m, &v| m.max(v));
                    assert!(
                        above < 3e-6,
                        "{} frame {frame} level {level}: {above:e} above harmonic {top}",
                        kind.name()
                    );
                    let row = t.row(frame, level);
                    assert_eq!(row[0], row[n]);
                    assert_eq!(row[n + 1], row[1]);
                    assert_eq!(row[n + 2], row[2]);
                    // A truncated level can overshoot the full table (Gibbs): the FM sweeps by up to 1.34.
                    assert!(row.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
                }
                let peak = t.row(frame, 0)[1..=level_len(0)]
                    .iter()
                    .fold(0.0f32, |m, x| m.max(x.abs()));
                assert!((peak - 1.0).abs() < 1e-6, "{} frame {frame}", kind.name());
            }
        }
    }

    #[test]
    fn basic_is_the_osc_basic_table() {
        let a = Table::generate(WtKind::Basic);
        let b = wavetable::Wavetable::generate(TableKind::Basic);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn fm_frames_are_chowning_spectra() {
        // Frame 6 is I = 5. Harmonic k of the 1:1 table is J(k−1)(5) + (−1)^k J(k+1)(5); the octave table has the
        // odd harmonics only.
        let fm1 = Table::generate(WtKind::Fm);
        let a = rows_spectrum(&fm1, 6, 0);
        let peak = fm1.row(6, 0)[1..=level_len(0)]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            (peak - 1.0).abs() < 1e-6,
            "an FM frame is a sine of something: peak 1"
        );
        for (k, &got) in a.iter().enumerate().take(13).skip(1) {
            let want = (bessel_j(k - 1, 5.0)
                + if k.is_multiple_of(2) { 1.0 } else { -1.0 } * bessel_j(k + 1, 5.0))
            .abs();
            assert!((got - want).abs() < 2e-6, "harmonic {k}: {got} vs {want}");
        }
        let fm2 = Table::generate(WtKind::FmOctave);
        let b = rows_spectrum(&fm2, 6, 0);
        for k in (2..40).step_by(2) {
            assert!(b[k] < 2e-6, "even harmonic {k}: {}", b[k]);
        }
        assert!(b[1] > 0.1 && b[3] > 0.05);
        // The first frame of every FM table is the sine.
        for t in [&fm1, &fm2] {
            let z = rows_spectrum(t, 0, 0);
            assert!((z[1] - 1.0).abs() < 1e-6 && z[2..300].iter().all(|&v| v < 3e-6));
        }
    }

    #[test]
    fn the_formant_peak_sweeps_up_and_the_metal_table_gets_denser() {
        let f = Table::generate(WtKind::Formant);
        let mut last = 0usize;
        for frame in 0..FRAMES {
            let a = rows_spectrum(&f, frame, 0);
            let k = (1..80)
                .max_by(|&i, &j| (a[i] * i as f64).total_cmp(&(a[j] * j as f64)))
                .unwrap();
            assert!(
                k >= last,
                "the strongest harmonic times k never falls: {k} after {last}"
            );
            last = k;
        }
        assert!(last > 20, "the sweep reaches the upper harmonics: {last}");
        let m = Table::generate(WtKind::Metal);
        let count = |frame| {
            rows_spectrum(&m, frame, 0)[1..256]
                .iter()
                .filter(|&&v| v > 1e-4)
                .count()
        };
        assert_eq!(count(0), 3);
        assert!(count(6) > count(0) && count(12) > count(6));
        assert_eq!(count(12), 24);
    }

    #[test]
    fn the_table_data_is_pinned() {
        let got: Vec<(WtKind, u64)> = WtKind::ALL
            .iter()
            .map(|&k| (k, Table::shared(k).fingerprint()))
            .collect();
        eprintln!("{got:?}");
        let want = [
            (WtKind::Basic, 1_928_649_884_303_330_563),
            (WtKind::Fm, 3_191_159_021_918_761_313),
            (WtKind::FmOctave, 8_857_352_278_237_311_971),
            (WtKind::Formant, 8_661_710_950_882_910_322),
            (WtKind::Metal, 2_911_790_962_221_655_094),
        ];
        assert_eq!(
            got, want,
            "update the pins on purpose, with the golden cases"
        );
    }

    #[test]
    fn shared_tables_are_one_allocation() {
        let a = Table::shared(WtKind::Metal);
        let b = Table::shared(WtKind::Metal);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.kind(), WtKind::Metal);
    }

    #[test]
    fn a_plain_voice_is_an_exact_read_of_the_table() {
        let t = Table::shared(WtKind::Basic);
        let dt = 440.0 / SR;
        let mut v = Voice::new(0.25);
        let s = plain(&t, dt, 0.0);
        let x = render(&mut v, &s, 8);
        let level = level_for(dt, alias_limit(SR));
        let mut phase = 0.25f32;
        for (i, &y) in x.iter().enumerate() {
            let want = t.cursor(level, 0.0).read(phase);
            assert_eq!(y.to_bits(), want.to_bits(), "sample {i}");
            phase += dt;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        // And it is the sine: against the osc's polynomial sine, within the interpolation error.
        let mut v = Voice::new(0.0);
        let x = render(&mut v, &s, 8);
        let mut phase = 0.0f32;
        for (i, &y) in x.iter().enumerate() {
            assert!(
                (y - sine(phase)).abs() < 1.5e-5,
                "sample {i}: {y} vs {}",
                sine(phase)
            );
            phase += dt;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
    }

    #[test]
    fn warps_are_the_identity_at_amount_zero_and_move_the_wave_otherwise() {
        let t = Table::shared(WtKind::Formant);
        let dt = 220.0 / SR;
        let base = render(&mut Voice::new(0.1), &plain(&t, dt, 5.5), 6);
        for warp in [
            Warp::Bend,
            Warp::Asymmetric,
            Warp::Squeeze,
            Warp::Sync,
            Warp::Quantize,
        ] {
            let mut s = plain(&t, dt, 5.5);
            s.warp = warp;
            let same = render(&mut Voice::new(0.1), &s, 6);
            assert_eq!(base, same, "{warp:?} at 0 is the straight read");
            for amount in [0.5f32, -0.5, 1.0, -1.0] {
                s.warp_amount = [amount, amount];
                let bent = render(&mut Voice::new(0.1), &s, 6);
                assert!(
                    bent.iter().all(|x| x.is_finite() && x.abs() <= 1.3),
                    "{warp:?} {amount}"
                );
                let diff: f32 = base.iter().zip(&bent).map(|(a, b)| (a - b).abs()).sum();
                assert!(diff > 1.0, "{warp:?} at {amount} changes the wave ({diff})");
            }
        }
    }

    #[test]
    fn every_warp_maps_the_unit_interval_into_itself_and_is_continuous_where_it_should_be() {
        for warp in [
            Warp::Bend,
            Warp::Asymmetric,
            Warp::Squeeze,
            Warp::Sync,
            Warp::Quantize,
        ] {
            for amount in [-1.0f32, -0.7, -0.2, 0.2, 0.7, 1.0] {
                let c = warp.prep(warp.primary(amount));
                let mut prev = warp.apply(&c, 0.0);
                for i in 1..4000 {
                    let p = i as f32 / 4000.0;
                    let q = warp.apply(&c, p);
                    assert!((0.0..1.0).contains(&q), "{warp:?} {amount}: {p} -> {q}");
                    if matches!(warp, Warp::Bend | Warp::Asymmetric | Warp::Squeeze) {
                        assert!(
                            (q - prev).abs() < 0.02 && q >= prev - 1e-6,
                            "{warp:?} {amount}: {p} -> {q} after {prev}"
                        );
                    }
                    prev = q;
                }
            }
        }
        // The bend's ends and its steepest slope.
        let c = Warp::Bend.prep(Warp::Bend.primary(1.0));
        assert!((c.a - 0.125).abs() < 1e-6);
        assert!((Warp::Bend.slope(c.a) - 8.0).abs() < 1e-4);
    }

    #[test]
    fn pm_adds_the_input_times_the_gain_to_the_phase() {
        // A sine table read at phase + offset: with a constant input the output is the table at the shifted phase.
        let t = Table::shared(WtKind::Basic);
        let dt = 100.0 / SR;
        let input = [0.5f32; N];
        let mut s = plain(&t, dt, 0.0);
        s.pm = Some(&input);
        s.pm_gain = [0.5, 0.5]; // 0.25 cycles
        let mut v = Voice::new(0.0);
        let mut out = [0.0; N];
        v.render(&s, &mut out);
        let mut phase = 0.0f32;
        for (i, &y) in out.iter().enumerate() {
            let want = sine(wrap01(phase + 0.25));
            assert!((y - want).abs() < 2e-5, "sample {i}: {y} vs {want}");
            phase += dt;
        }
    }

    #[test]
    fn the_level_follows_the_modulation_and_changes_are_cross_faded() {
        let t = Table::shared(WtKind::Formant);
        let dt = 110.0 / SR;
        let mut v = Voice::new(0.0);
        let mut out = [0.0; N];
        let s = plain(&t, dt, 3.0);
        v.render(&s, &mut out);
        let base = v.level().unwrap();
        assert_eq!(base, level_for(dt, alias_limit(SR)));
        // A fast modulator pushes the tracked increment up: the level drops (fewer harmonics) and stays.
        let input: Vec<f32> = (0..N)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let input: [f32; N] = input.try_into().unwrap();
        let mut m = plain(&t, dt, 3.0);
        m.pm = Some(&input);
        m.pm_gain = [0.1, 0.1];
        v.render(&m, &mut out);
        let hit = v.level().unwrap();
        assert!(
            hit > base,
            "level {hit} after the modulation, {base} before"
        );
        assert!(out.iter().all(|x| x.is_finite()));
        // The release is slow: a hundred quiet segments later the level is still the low one.
        for _ in 0..100 {
            v.render(&s, &mut out);
        }
        assert_eq!(v.level().unwrap(), hit);
        // And it comes back eventually.
        for _ in 0..4000 {
            v.render(&s, &mut out);
        }
        assert_eq!(v.level().unwrap(), base);
    }

    #[test]
    fn feedback_turns_a_sine_towards_a_saw_and_stays_bounded() {
        let t = Table::shared(WtKind::Basic);
        let dt = 110.0 / SR;
        let mut last = f64::INFINITY;
        for fb in [0.0f32, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let mut s = plain(&t, dt, 0.0);
            s.feedback = [fb * FEEDBACK_CYCLES, fb * FEEDBACK_CYCLES];
            let x = render(&mut Voice::new(0.0), &s, 1500);
            assert!(x.iter().all(|y| y.abs() <= 1.001), "bounded at {fb}");
            let p = power_spectrum(&x[x.len() - FFT_LEN..]);
            let bin = f64::from(SR) / FFT_LEN as f64;
            let amp = |k: usize| {
                let c = (k as f64 * f64::from(110.0f32) / bin).round() as usize;
                p[c - 4..=c + 4].iter().sum::<f64>().sqrt()
            };
            // Distance of the first ten harmonics from 1/n, in dB (floored at −100).
            let a1 = amp(1);
            let d: f64 = (2..=10)
                .map(|k| {
                    let db = (20.0 * math::log10(amp(k) / a1 + 1e-5)).max(-100.0);
                    let saw = -20.0 * math::log10(k as f64);
                    (db - saw) * (db - saw)
                })
                .sum::<f64>()
                .sqrt();
            assert!(d < last, "feedback {fb}: distance {d} not below {last}");
            last = d;
        }
    }

    #[test]
    fn nan_and_huge_inputs_cannot_poison_the_voice() {
        let t = Table::shared(WtKind::Formant);
        let mut input = [0.0f32; N];
        input[3] = f32::NAN;
        input[9] = f32::INFINITY;
        input[10] = -3.0e38;
        let mut s = plain(&t, 0.01, 2.5);
        s.pm = Some(&input);
        s.pm_gain = [1.0, 1.0];
        s.feedback = [0.5, 0.5];
        s.warp = Warp::Sync;
        s.warp_amount = [0.7, 0.7];
        let mut v = Voice::new(0.3);
        let mut out = [0.0; N];
        for _ in 0..10 {
            v.render(&s, &mut out);
            assert!(out.iter().all(|x| x.is_finite()), "{out:?}");
        }
        s.pm = None;
        for _ in 0..4 {
            v.render(&s, &mut out);
        }
        assert!(out.iter().all(|x| x.is_finite() && x.abs() < 1.5));
    }

    /// Alias power over harmonic power for one table at one key, the harness's measure.
    fn alias_of(kind: WtKind, key: f32, pos: f32) -> f64 {
        let t = Table::shared(kind);
        let hz = f64::from(440.0 * math::powf(2.0, (key - 69.0) / 12.0));
        let dt = (hz / f64::from(SR)) as f32;
        let s = plain(&t, dt, pos * 12.0);
        let x = render(&mut Voice::new(0.0), &s, (FFT_LEN + 4096) / N);
        alias_db(&x, hz, f64::from(SR), &Measure::REPORT_02)
    }

    #[test]
    fn each_table_is_band_limited_at_a_high_key() {
        for kind in WtKind::ALL {
            for pos in [0.0f32, 0.5, 1.0] {
                for key in [45.0f32, 81.0, 105.0] {
                    let a = alias_of(kind, key, pos);
                    assert!(a < -80.0, "{} pos {pos} key {key}: {a:.1} dB", kind.name());
                }
            }
        }
    }
}
