//! Mip-mapped wavetables generated in code (P1a lane D1).
//!
//! A table is a set of [`FRAMES`] single-cycle frames. Each frame exists at [`LEVELS`] mip levels: level `l` holds only
//! the harmonics `1..=`[`top_harmonic`]`(l)` (`256 >> l`: 256, 128, … 1), so a note can read the level whose top
//! harmonic stays below the Nyquist limit and the table itself never aliases. A level is stored at
//! [`level_len`]`(l)` samples per cycle, 32 times its top harmonic (at least 1024): that keeps the 4-point Hermite
//! interpolation error of the highest harmonic, and so the interpolation images that fold back into the audio band,
//! at the same low level for every pitch (the first design, 2048 samples for every level, measured −78 dB aliasing for
//! a saw at 55 Hz and −60 dB for a very bright table; this one is below −85 dB everywhere, at the same memory).
//! Frames are formulas over harmonic amplitudes ([`TableKind`]); there are no audio assets and no licence question.
//! Docs: `docs/dsp/osc.md`.
//!
//! # Determinism
//!
//! Generation is exact integer-indexed additive synthesis in `f64`: harmonic `k` at sample `n` of a level of `N`
//! samples is `SINE[(k·n·(8192/N)) mod 8192]`, a table of `sin(2π·i/8192)` from the pinned `libm`, so there is no phase
//! drift and no rounding that depends on the harmonic. The sums run in a fixed order (ascending `k`), every frame is
//! normalised by its own level-0 peak in `f64` (all levels of a frame share one scale), and only then rounded to
//! `f32`. Only `+ − × ÷` and `libm` are involved, so the data is bit-identical on every platform; the unit tests pin
//! a fingerprint per table.
//!
//! # Cost and sharing
//!
//! One table costs a few tens of milliseconds to generate and 1 MB to hold. [`Wavetable::shared`] generates each
//! table at most once per process and hands out `Arc`s, so preparing a module (or a whole patch in a test) is a
//! refcount bump after the first use. The cache is a pure memo of a pure function: it changes no result, and nothing
//! on the audio path touches it.
//!
//! # Reading
//!
//! [`Wavetable::sample`] reads one frame pair at a (fractional) frame position with 4-point Hermite interpolation in
//! phase (the form in stmlib's `InterpolateHermite`) and a linear cross-fade between the two frames.
//! [`level_for`] picks the mip level for a phase increment; [`alias_limit`] is the highest harmonic frequency a
//! level may keep, as a fraction of the sample rate.

use core::f64::consts::PI;
use std::sync::{Arc, OnceLock};

use crate::util::math;

/// Mip levels per frame; level `l` keeps harmonics up to [`top_harmonic`]`(l)`.
pub const LEVELS: usize = 9;
/// The highest harmonic of any level (level 0).
pub const MAX_HARMONIC: usize = 256;
/// Guard samples per level: `x(-1)` in front, `x(N)` and `x(N+1)` behind, so Hermite never wraps the index.
pub const GUARD: usize = 3;
/// Frames per table: 13, so the 3 and 4 and 6 equal segments of the tables below fall on whole frames.
pub const FRAMES: usize = 13;

/// The top harmonic of `level`: `256 >> level`.
#[must_use]
pub const fn top_harmonic(level: usize) -> usize {
    MAX_HARMONIC >> level
}

/// Samples per cycle of `level`: 32 times its top harmonic, at least 1024 (a power of two).
#[must_use]
pub const fn level_len(level: usize) -> usize {
    let n = 32 * top_harmonic(level);
    if n < 1024 { 1024 } else { n }
}

/// The master sine table's length (the longest level).
pub(crate) const MASTER_LEN: usize = 32 * MAX_HARMONIC;

/// Per-level sample counts and start offsets inside a frame (each level is followed by its guards).
pub(crate) const LEN: [usize; LEVELS] = {
    let mut a = [0; LEVELS];
    let mut l = 0;
    while l < LEVELS {
        a[l] = level_len(l);
        l += 1;
    }
    a
};
pub(crate) const OFFSET: [usize; LEVELS + 1] = {
    let mut a = [0; LEVELS + 1];
    let mut l = 0;
    while l < LEVELS {
        a[l + 1] = a[l] + LEN[l] + GUARD;
        l += 1;
    }
    a
};
/// Floats per frame (all levels with their guards).
pub(crate) const FRAME_STRIDE: usize = OFFSET[LEVELS];

/// The five generated tables, in the catalogue's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TableKind {
    /// Sine, triangle, saw, square: each a third of the way.
    Basic,
    /// A pulse whose duty falls from 50 % to 3 %.
    Pulse,
    /// Harmonic amplitudes `k^-p`, `p` from 0.5 to 3.
    Tilt,
    /// Vowel formants A, E, I, O, U over a saw spectrum.
    Formant,
    /// Drawbar organ registrations.
    Organ,
}

impl TableKind {
    /// Every table, in catalogue order.
    pub const ALL: [TableKind; 5] = [
        TableKind::Basic,
        TableKind::Pulse,
        TableKind::Tilt,
        TableKind::Formant,
        TableKind::Organ,
    ];

    /// Position in [`TableKind::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The catalogue key (`basic`, `pulse`, `tilt`, `formant`, `organ`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            TableKind::Basic => "basic",
            TableKind::Pulse => "pulse",
            TableKind::Tilt => "tilt",
            TableKind::Formant => "formant",
            TableKind::Organ => "organ",
        }
    }
}

/// One generated table: [`FRAMES`] frames of [`LEVELS`] levels, each level `level_len(l) + GUARD` floats
/// (`x(-1), x(0), …, x(N-1), x(N), x(N+1)`), frame after frame.
pub struct Wavetable {
    kind: TableKind,
    data: Box<[f32]>,
}

impl core::fmt::Debug for Wavetable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Wavetable")
            .field("kind", &self.kind)
            .field("frames", &FRAMES)
            .field("levels", &LEVELS)
            .finish_non_exhaustive()
    }
}

impl Wavetable {
    /// Generate `kind` from scratch (tens of milliseconds; allocates). Prefer [`Wavetable::shared`].
    #[must_use]
    pub fn generate(kind: TableKind) -> Self {
        let data = build(&mut |pos, s, c| harmonics(kind, pos, s, c));
        Self { kind, data }
    }

    /// The process-wide copy of `kind`: generated on first use, then shared. See the module docs.
    #[must_use]
    pub fn shared(kind: TableKind) -> Arc<Wavetable> {
        static CACHE: [OnceLock<Arc<Wavetable>>; 5] = [const { OnceLock::new() }; 5];
        Arc::clone(CACHE[kind.index()].get_or_init(|| Arc::new(Wavetable::generate(kind))))
    }

    /// Which table this is.
    #[must_use]
    pub fn kind(&self) -> TableKind {
        self.kind
    }

    /// One level of one frame: `x(-1), x(0), …, x(N-1), x(N), x(N+1)` with `N = level_len(level)`.
    #[must_use]
    pub fn row(&self, frame: usize, level: usize) -> &[f32] {
        row_in(&self.data, frame, level)
    }

    /// Read `level` at the frame position `frame_pos` (`0 ..= FRAMES − 1`, clamped) and `phase` (`0 ≤ phase < 1`):
    /// 4-point Hermite in phase, linear between the two neighbouring frames.
    #[inline(always)]
    #[must_use]
    pub fn sample(&self, level: usize, frame_pos: f32, phase: f32) -> f32 {
        self.cursor(level, frame_pos).read(phase)
    }

    /// The part of a read that does not depend on the phase: the two frame rows of `level` around `frame_pos`
    /// (clamped to `0 ..= FRAMES − 1`) and the cross-fade weight. A module builds one per block while the position
    /// holds still and calls [`Cursor::read`] per sample; [`Wavetable::sample`] is exactly `cursor(…).read(…)`.
    #[inline(always)]
    #[must_use]
    pub fn cursor(&self, level: usize, frame_pos: f32) -> Cursor<'_> {
        cursor_in(&self.data, level, frame_pos)
    }

    /// FNV-1a (64 bit) over the little-endian bits of every float, in storage order: a fingerprint for tests and
    /// docs ("the table data is the same on every platform").
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        fingerprint_of(&self.data)
    }
}

/// [`Wavetable::fingerprint`] over any table data.
pub(crate) fn fingerprint_of(data: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in data {
        for b in x.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// A harmonic filler: see [`build`].
pub(crate) type Filler<'a> = dyn FnMut(f64, &mut [f64], &mut [f64]) + 'a;

/// Build the data of one table from a harmonic filler: `fill(pos, s, c)` writes the coefficients of `sin(k·x)` into
/// `s[k]` and of `cos(k·x)` into `c[k]` (`k = 1..=`[`MAX_HARMONIC`]) for the frame position `pos` in `[0, 1]`. The
/// layout is the one described at the top of this module; this is the whole generator, shared by [`Wavetable`] and
/// the `wt` module's tables (`crate::wt`), so both are built by the same code in the same order.
pub(crate) fn build(fill: &mut Filler<'_>) -> Box<[f32]> {
    let sine = sine_table();
    let mut data = vec![0.0f32; FRAMES * FRAME_STRIDE].into_boxed_slice();
    let mut sin_coef = [0.0f64; MAX_HARMONIC + 1];
    let mut cos_coef = [0.0f64; MAX_HARMONIC + 1];
    let mut acc = vec![0.0f64; MASTER_LEN];
    for frame in 0..FRAMES {
        let pos = frame as f64 / (FRAMES - 1) as f64;
        sin_coef.fill(0.0);
        cos_coef.fill(0.0);
        fill(pos, &mut sin_coef, &mut cos_coef);
        let mut scale = 1.0;
        // Level 0 first: its peak is the frame's scale for every level.
        for level in 0..LEVELS {
            let n = LEN[level];
            let step = MASTER_LEN / n;
            let acc = &mut acc[..n];
            acc.fill(0.0);
            for k in 1..=top_harmonic(level) {
                let (s, c) = (sin_coef[k], cos_coef[k]);
                if s != 0.0 {
                    for (i, a) in acc.iter_mut().enumerate() {
                        *a += s * sine[(k * i * step) & (MASTER_LEN - 1)];
                    }
                }
                if c != 0.0 {
                    // cos(x) = sin(x + π/2): a quarter turn is a quarter of the master table.
                    for (i, a) in acc.iter_mut().enumerate() {
                        *a += c * sine[(k * i * step + MASTER_LEN / 4) & (MASTER_LEN - 1)];
                    }
                }
            }
            if level == 0 {
                let peak = acc.iter().fold(0.0f64, |m, x| m.max(x.abs()));
                scale = if peak > 0.0 { 1.0 / peak } else { 1.0 };
            }
            let base = frame * FRAME_STRIDE + OFFSET[level];
            let row = &mut data[base..base + n + GUARD];
            for (dst, src) in row[1..=n].iter_mut().zip(acc.iter()) {
                *dst = (*src * scale) as f32;
            }
            row[0] = row[n];
            row[n + 1] = row[1];
            row[n + 2] = row[2];
        }
    }
    data
}

/// One level of one frame in table data laid out by [`build`].
pub(crate) fn row_in(data: &[f32], frame: usize, level: usize) -> &[f32] {
    let base = frame * FRAME_STRIDE + OFFSET[level];
    &data[base..base + LEN[level] + GUARD]
}

/// The cursor of [`Wavetable::cursor`] over any data laid out by [`build`].
#[inline(always)]
pub(crate) fn cursor_in(data: &[f32], level: usize, frame_pos: f32) -> Cursor<'_> {
    let fp = frame_pos.max(0.0).min((FRAMES - 1) as f32);
    // On the last frame the weight is 0 and `b` repeats `a`.
    let fi = (fp as usize).min(FRAMES - 1);
    let lv = level.min(LEVELS - 1);
    Cursor {
        a: row_in(data, fi, lv),
        b: row_in(data, (fi + 1).min(FRAMES - 1), lv),
        weight: fp - fi as f32,
        len: LEN[lv] as f32,
        mask: LEN[lv] - 1,
    }
}

/// One level at one frame position: see [`Wavetable::cursor`].
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    a: &'a [f32],
    b: &'a [f32],
    weight: f32,
    len: f32,
    mask: usize,
}

impl Cursor<'_> {
    /// Whether the position sits on a frame (cross-fade weight 0), so [`Cursor::read_first`] is the whole read.
    #[inline(always)]
    #[must_use]
    pub fn on_frame(&self) -> bool {
        self.weight == 0.0
    }

    /// Hermite index and fraction for `phase`.
    #[inline(always)]
    fn locate(&self, phase: f32) -> (usize, f32) {
        let p = phase * self.len;
        let i = (p as usize) & self.mask;
        (i, p - i as f32)
    }

    /// The value at `phase` (`0 ≤ phase < 1`): Hermite in each of the two frames, cross-faded.
    #[inline(always)]
    #[must_use]
    pub fn read(&self, phase: f32) -> f32 {
        let (i, fr) = self.locate(phase);
        let a = hermite(&self.a[i..i + 4], fr);
        let b = hermite(&self.b[i..i + 4], fr);
        a + (b - a) * self.weight
    }

    /// The value at `phase` in the lower frame alone; equal to [`Cursor::read`] when [`Cursor::on_frame`].
    #[inline(always)]
    #[must_use]
    pub fn read_first(&self, phase: f32) -> f32 {
        let (i, fr) = self.locate(phase);
        hermite(&self.a[i..i + 4], fr)
    }
}

/// 4-point Hermite (Catmull-Rom form) between `w[1]` and `w[2]` at `fr`, the form of stmlib's `InterpolateHermite`
/// (`xm1 = w[0]`, `x0 = w[1]`, `x1 = w[2]`, `x2 = w[3]`).
#[inline(always)]
pub(crate) fn hermite(w: &[f32], fr: f32) -> f32 {
    let (xm1, x0, x1, x2) = (w[0], w[1], w[2], w[3]);
    let c = (x1 - xm1) * 0.5;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * 0.5;
    let b_neg = w + a;
    (((a * fr) - b_neg) * fr + c) * fr + x0
}

/// `sin(2π·i / 8192)` for one cycle, in `f64` through the pinned `libm`.
fn sine_table() -> Vec<f64> {
    (0..MASTER_LEN)
        .map(|i| math::sin(core::f64::consts::TAU * i as f64 / MASTER_LEN as f64))
        .collect()
}

/// The highest harmonic frequency a mip level may keep, as a fraction of the sample rate (cycles per sample):
/// `max(1/2, 1 − 20 kHz / fs)`. Above Nyquist but never below `fs − 20 kHz`, so whatever aliases lands above
/// 20 kHz (at 48 kHz: harmonics up to 28 kHz, folding into 20–24 kHz, where nothing is audible).
#[must_use]
pub fn alias_limit(sample_rate: f32) -> f32 {
    (1.0 - 20_000.0 / sample_rate).max(0.5)
}

/// The mip level for a phase increment `dt` (cycles per sample): the lowest (brightest) level whose top harmonic,
/// times `dt`, stays at or below `limit` (see [`alias_limit`]); level 0 (256 harmonics) for anything lower. The last
/// level (a lone fundamental) is always valid.
#[inline]
#[must_use]
pub fn level_for(dt: f32, limit: f32) -> usize {
    let mut level = 0;
    while level < LEVELS - 1 && top_harmonic(level) as f32 * dt > limit {
        level += 1;
    }
    level
}

// ---------------------------------------------------------------------------------------------------------------
// The tables: harmonic amplitudes per frame
// ---------------------------------------------------------------------------------------------------------------

/// Fill `s[k]` (coefficient of `sin(k·x)`) and `c[k]` (of `cos(k·x)`) for `k = 1..=`[`MAX_HARMONIC`].
/// `pos` is the frame position in `[0, 1]`. Every shape starts like its analog counterpart in
/// [`crate::osc`]: sine at 0 rising, the others at their low value.
#[allow(
    clippy::needless_range_loop,
    reason = "k is the harmonic number in every formula as well as the index into s and c"
)]
pub(crate) fn harmonics(kind: TableKind, pos: f64, s: &mut [f64], c: &mut [f64]) {
    let top = MAX_HARMONIC;
    match kind {
        TableKind::Basic => {
            // Four shapes, three equal blends. Weights (sine, triangle, saw, square).
            let t = pos * 3.0;
            let (seg, u) = if t >= 3.0 {
                (2, 1.0)
            } else {
                (t as usize, t - (t as usize) as f64)
            };
            let mut w = [0.0f64; 4];
            w[seg] = 1.0 - u;
            w[seg + 1] = u;
            for k in 1..=top {
                let kf = k as f64;
                let odd = k % 2 == 1;
                // sine: sin(x)
                if k == 1 {
                    s[k] += w[0];
                }
                // triangle (starts at -1, peaks at x = π): -(8/π²) Σ_{k odd} cos(kx) / k²
                if odd {
                    c[k] += w[1] * (-8.0 / (PI * PI) / (kf * kf));
                }
                // saw (starts at -1, rising, jump at x = 0): -(2/π) Σ sin(kx) / k
                s[k] += w[2] * (-2.0 / (PI * kf));
                // square (low first half): -(4/π) Σ_{k odd} sin(kx) / k
                if odd {
                    s[k] += w[3] * (-4.0 / (PI * kf));
                }
            }
        }
        TableKind::Pulse => {
            // High for the last `d` of the cycle, low before; DC removed. 50 % -> 3 %.
            let d = 0.5 - 0.47 * pos;
            let a = 2.0 * PI * (1.0 - d);
            for k in 1..=top {
                let kf = k as f64;
                s[k] = 2.0 * (math::cos(kf * a) - 1.0) / (PI * kf);
                c[k] = -2.0 * math::sin(kf * a) / (PI * kf);
            }
        }
        TableKind::Tilt => {
            // -k^-p: p = 1 is the saw, below it buzzier, above it nearer a sine.
            let p = 0.5 + 2.5 * pos;
            for k in 1..=top {
                s[k] = -math::pow(k as f64, -p);
            }
        }
        TableKind::Formant => {
            // Peterson and Barney (1952) male-average formants F1..F3 (Hz) of A, E, I, O, U, as harmonic numbers of
            // a 110 Hz fundamental, morphing linearly between neighbouring vowels; a saw spectrum under three
            // Lorentzian resonances of half-width 1.0, 1.2 and 1.8 harmonics and gains 1, 0.7, 0.35.
            const VOWELS: [[f64; 3]; 5] = [
                [730.0, 1090.0, 2440.0], // A
                [530.0, 1840.0, 2480.0], // E
                [270.0, 2290.0, 3010.0], // I
                [570.0, 840.0, 2410.0],  // O
                [300.0, 870.0, 2240.0],  // U
            ];
            const WIDTH: [f64; 3] = [1.0, 1.2, 1.8];
            const GAIN: [f64; 3] = [1.0, 0.7, 0.35];
            const F0: f64 = 110.0;
            let t = pos * 4.0;
            let (seg, u) = if t >= 4.0 {
                (3, 1.0)
            } else {
                (t as usize, t - (t as usize) as f64)
            };
            let mut centre = [0.0f64; 3];
            for (j, ctr) in centre.iter_mut().enumerate() {
                *ctr = (VOWELS[seg][j] + (VOWELS[seg + 1][j] - VOWELS[seg][j]) * u) / F0;
            }
            for k in 1..=top {
                let kf = k as f64;
                let mut f = 0.02;
                for ((ctr, width), gain) in centre.iter().zip(WIDTH).zip(GAIN) {
                    let d = (kf - ctr) / width;
                    f += gain / (1.0 + d * d);
                }
                s[k] = -f / kf;
            }
        }
        TableKind::Organ => {
            // Seven drawbars on harmonics 1, 2, 3, 4, 5, 6, 8 (8', 4', 2 2/3', 2', 1 3/5', 1 1/3', 1'); the 16' and
            // 5 1/3' drawbars sit below the fundamental and cannot live in a one-cycle table. Settings 0-8, each
            // step 3 dB, 0 is off; six equal blends between seven registrations.
            const HARMONIC: [usize; 7] = [1, 2, 3, 4, 5, 6, 8];
            const REGISTRATIONS: [[u8; 7]; 7] = [
                [8, 0, 0, 0, 0, 0, 0],
                [8, 8, 0, 0, 0, 0, 0],
                [8, 6, 8, 0, 0, 0, 0],
                [8, 8, 8, 8, 0, 0, 0],
                [8, 8, 8, 8, 8, 0, 0],
                [6, 8, 6, 8, 6, 8, 0],
                [8, 8, 8, 8, 8, 8, 8],
            ];
            let t = pos * 6.0;
            let (seg, u) = if t >= 6.0 {
                (5, 1.0)
            } else {
                (t as usize, t - (t as usize) as f64)
            };
            for (j, &k) in HARMONIC.iter().enumerate() {
                let amp = |v: u8| {
                    if v == 0 {
                        0.0
                    } else {
                        math::pow(10.0, -(8.0 - f64::from(v)) * 3.0 / 20.0)
                    }
                };
                let a = amp(REGISTRATIONS[seg][j]);
                let b = amp(REGISTRATIONS[seg + 1][j]);
                s[k] = a + (b - a) * u;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Test support: a small radix-2 FFT and the alias measurement of research report 02
// ---------------------------------------------------------------------------------------------------------------

/// Spectrum helpers shared by the unit tests of `osc` and `noise`.
#[cfg(test)]
pub(crate) mod testkit {
    use core::f64::consts::PI;

    use crate::util::math;

    /// In-place radix-2 FFT (`re.len()` a power of two).
    pub fn fft(re: &mut [f64], im: &mut [f64]) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let ang = -2.0 * PI / len as f64;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = (math::cos(ang * k as f64), math::sin(ang * k as f64));
                    let (a, b) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }

    /// One-sided power spectrum `|X|²` of `x` under a 4-term Blackman-Harris window (`x.len()` a power of two).
    pub fn power_spectrum(x: &[f32]) -> Vec<f64> {
        let n = x.len();
        let (a0, a1, a2, a3) = (0.35875, 0.48829, 0.14128, 0.01168);
        let mut re: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                let w = a0 - a1 * math::cos(2.0 * PI * t) + a2 * math::cos(4.0 * PI * t)
                    - a3 * math::cos(6.0 * PI * t);
                f64::from(x[i]) * w
            })
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        (0..=n / 2).map(|i| re[i] * re[i] + im[i] * im[i]).collect()
    }

    /// Research report 02's alias measure: alias power over harmonic power in dB, 20 Hz to 20 kHz, from 32,768
    /// samples starting at `start`. Power within ±8 bins of `k·f0` counts as signal, every other bin as alias.
    pub fn alias_ratio_db(x: &[f32], f0: f64, sr: f64, start: usize) -> f64 {
        const N: usize = 32_768;
        let p = power_spectrum(&x[start..start + N]);
        let bin = sr / N as f64;
        let mut is_harm = vec![false; p.len()];
        let mut k = 1.0;
        while k * f0 < sr / 2.0 {
            let c = k * f0 / bin;
            let (lo, hi) = (
                (c - 8.0).floor().max(0.0) as usize,
                (c + 8.0).ceil() as usize,
            );
            for h in is_harm.iter_mut().take(hi.min(p.len() - 1) + 1).skip(lo) {
                *h = true;
            }
            k += 1.0;
        }
        let (mut ph, mut pa) = (0.0, 0.0);
        for (i, &v) in p.iter().enumerate() {
            let f = i as f64 * bin;
            if (20.0..=20_000.0).contains(&f) {
                if is_harm[i] {
                    ph += v;
                } else {
                    pa += v;
                }
            }
        }
        10.0 * math::log10(pa / ph + 1e-30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_is_consistent() {
        assert_eq!(LEN, [8192, 4096, 2048, 1024, 1024, 1024, 1024, 1024, 1024]);
        assert_eq!(top_harmonic(0), 256);
        assert_eq!(top_harmonic(LEVELS - 1), 1);
        for (l, &len) in LEN.iter().enumerate() {
            assert!(len.is_power_of_two() && len >= 32 * top_harmonic(l));
        }
        assert_eq!(FRAME_STRIDE, 8192 + 4096 + 2048 + 6 * 1024 + LEVELS * GUARD);
        assert_eq!(MASTER_LEN, LEN[0]);
    }

    #[test]
    fn tables_are_finite_normalised_and_have_guards() {
        for kind in TableKind::ALL {
            let t = Wavetable::generate(kind);
            for frame in 0..FRAMES {
                let top = t.row(frame, 0);
                let peak = top[1..=LEN[0]].iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!(
                    (peak - 1.0).abs() < 1e-6,
                    "{kind:?} frame {frame}: peak {peak}"
                );
                for (level, &n) in LEN.iter().enumerate() {
                    let row = t.row(frame, level);
                    assert_eq!(row.len(), n + GUARD);
                    assert!(
                        row.iter().all(|x| x.is_finite()),
                        "{kind:?} {frame} {level}"
                    );
                    assert_eq!(row[0], row[n], "guard x(-1) = x(N-1)");
                    assert_eq!(row[n + 1], row[1], "guard x(N) = x(0)");
                    assert_eq!(row[n + 2], row[2], "guard x(N+1) = x(1)");
                    let mean: f64 =
                        row[1..=n].iter().map(|&x| f64::from(x)).sum::<f64>() / n as f64;
                    assert!(mean.abs() < 1e-6, "{kind:?} {frame} {level}: DC {mean}");
                    // Shared scale: no level exceeds the Gibbs overshoot of the full table.
                    assert!(
                        row.iter().all(|x| x.abs() <= 1.2),
                        "{kind:?} {frame} {level}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_basic_corners_are_the_analog_shapes() {
        let t = Wavetable::generate(TableKind::Basic);
        let n = LEN[0];
        // Frame 0: a sine, rising from zero.
        let r = t.row(0, 0);
        for i in [0usize, n / 8, n / 4, n / 2, 3 * n / 4, n - 1] {
            let want = math::sin(core::f64::consts::TAU * i as f64 / n as f64) as f32;
            assert!((r[1 + i] - want).abs() < 1e-6, "sine at {i}: {}", r[1 + i]);
        }
        // Frame 4 (1/3): the triangle, starting low, +1 half way.
        let r = t.row(4, 0);
        assert!((r[1] + 1.0).abs() < 2e-2, "{}", r[1]);
        assert!((r[1 + n / 2] - 1.0).abs() < 2e-2, "{}", r[1 + n / 2]);
        // Frame 8 (2/3): the saw, rising from -1 after its jump to +1 at the end.
        let r = t.row(8, 0);
        // (With 256 harmonics the Gibbs peak sits 16 samples from the jump of the 8192-sample level.)
        let low = r[1..=40].iter().fold(0.0f32, |m, &x| m.min(x));
        let high = r[n - 39..=n].iter().fold(0.0f32, |m, &x| m.max(x));
        assert!(low < -0.99 && high > 0.99, "{low} {high}");
        assert!(
            r[1 + n / 2].abs() < 0.01,
            "mid-cycle crossing {}",
            r[1 + n / 2]
        );
        // Frame 12: the square, low then high. Peak normalisation puts the plateau at about 1/1.18 (Gibbs).
        let r = t.row(12, 0);
        assert!((r[1 + n / 4] + 0.85).abs() < 0.03 && (r[1 + 3 * n / 4] - 0.85).abs() < 0.03);
    }

    #[test]
    fn mip_levels_hold_only_their_harmonics() {
        // Every level is band-limited: the spectrum above its top harmonic is empty.
        for kind in [TableKind::Tilt, TableKind::Pulse] {
            let t = Wavetable::generate(kind);
            for (level, &n) in LEN.iter().enumerate() {
                let top = top_harmonic(level);
                let mut re: Vec<f64> = t.row(6, level)[1..=n]
                    .iter()
                    .map(|&x| f64::from(x))
                    .collect();
                let mut im = vec![0.0; n];
                testkit::fft(&mut re, &mut im);
                let in_band: f64 = (1..=top).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
                let above: f64 = (top + 1..n / 2)
                    .map(|k| re[k] * re[k] + im[k] * im[k])
                    .sum();
                assert!(
                    above / in_band < 1e-10,
                    "{kind:?} level {level}: leak {}",
                    above / in_band
                );
                assert!(in_band > 0.0);
            }
        }
    }

    #[test]
    fn level_choice_keeps_the_top_harmonic_below_the_limit() {
        let limit = alias_limit(48_000.0);
        assert!((limit - 0.583_333).abs() < 1e-5, "{limit}");
        assert_eq!(alias_limit(8_000.0), 0.5);
        assert_eq!(alias_limit(44_100.0), 1.0 - 20_000.0 / 44_100.0);
        assert_eq!(
            level_for(1e-5, limit),
            0,
            "a sub-audio rate keeps all 256 harmonics"
        );
        for hz in [
            27.5f32, 55.0, 110.0, 220.0, 440.0, 880.0, 3_520.0, 12_000.0, 21_000.0,
        ] {
            let dt = hz / 48_000.0;
            let m = level_for(dt, limit);
            assert!(
                top_harmonic(m) as f32 * dt <= limit || m == LEVELS - 1,
                "{hz} Hz: level {m}"
            );
            if m > 0 {
                assert!(
                    top_harmonic(m - 1) as f32 * dt > limit,
                    "{hz} Hz: level {m} is not the brightest allowed"
                );
            }
        }
        // The levels fall as pitch rises, and the last is a lone fundamental.
        assert!(level_for(0.01, limit) < level_for(0.1, limit));
        assert_eq!(level_for(0.5, limit), LEVELS - 1);
    }

    #[test]
    fn sample_reads_a_sine_frame_exactly() {
        let t = Wavetable::generate(TableKind::Basic);
        // Hermite on a sine table: error far below the f32 grid of audio. Frame position 0 and the last level.
        for i in 0..4096 {
            let phase = i as f32 / 4096.0;
            let want = math::sin(core::f64::consts::TAU * f64::from(phase)) as f32;
            let got = t.sample(LEVELS - 1, 0.0, phase);
            assert!((got - want).abs() < 3e-6, "{phase}: {got} vs {want}");
        }
        // The last frame position is clamped, a position beyond it too, and NaN reads frame 0.
        assert_eq!(t.sample(3, 12.0, 0.3), t.sample(3, 99.0, 0.3));
        assert!(t.sample(3, f32::NAN, 0.3).is_finite());
        // Halfway between two frames is the mean of the two reads.
        let (a, b) = (t.sample(3, 5.0, 0.37), t.sample(3, 6.0, 0.37));
        assert!((t.sample(3, 5.5, 0.37) - f32::midpoint(a, b)).abs() < 1e-6);
        // Every level at every phase just below 1 stays inside its row (no index past the guards).
        for level in 0..LEVELS {
            assert!(t.sample(level, 12.0, 0.999_999_94).is_finite());
            assert!(t.sample(level, 0.0, 0.0).is_finite());
        }
    }

    #[test]
    fn a_cursor_reads_what_sample_reads_and_knows_when_it_sits_on_a_frame() {
        let t = Wavetable::shared(TableKind::Formant);
        for level in [0, 3, LEVELS - 1] {
            for frame_pos in [0.0, 0.25, 5.0, 7.75, 11.999, 12.0, 40.0, -3.0] {
                let at = t.cursor(level, frame_pos);
                for i in 0..200 {
                    let phase = i as f32 / 200.0 + 0.001;
                    assert_eq!(
                        at.read(phase).to_bits(),
                        t.sample(level, frame_pos, phase).to_bits()
                    );
                }
            }
            // On a frame (including the last one, whose weight is 0 because the second row repeats the first) the
            // lower frame alone is the whole read.
            for frame in 0..FRAMES {
                let at = t.cursor(level, frame as f32);
                assert!(at.on_frame(), "frame {frame}");
                for i in 0..200 {
                    let phase = i as f32 / 200.0 + 0.001;
                    assert_eq!(at.read_first(phase).to_bits(), at.read(phase).to_bits());
                }
            }
            assert!(!t.cursor(level, 4.5).on_frame());
        }
    }

    #[test]
    fn shared_tables_are_one_allocation_and_equal_a_fresh_one() {
        let a = Wavetable::shared(TableKind::Organ);
        let b = Wavetable::shared(TableKind::Organ);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(
            a.fingerprint(),
            Wavetable::generate(TableKind::Organ).fingerprint()
        );
        assert_eq!(a.kind(), TableKind::Organ);
    }

    /// The table data is the same on every platform: these are the native x86-64 values, and the golden cases
    /// `osc-table-*` check the wasm build through renders that read every frame of every table.
    #[test]
    fn the_table_data_is_pinned() {
        let got: Vec<(TableKind, u64)> = TableKind::ALL
            .iter()
            .map(|&k| (k, Wavetable::shared(k).fingerprint()))
            .collect();
        eprintln!("{got:?}");
        let want = [
            (TableKind::Basic, 1_928_649_884_303_330_563),
            (TableKind::Pulse, 8_164_407_817_302_734_457),
            (TableKind::Tilt, 7_556_160_347_434_841_981),
            (TableKind::Formant, 14_543_095_876_329_344_712),
            (TableKind::Organ, 18_368_377_192_750_842_625),
        ];
        assert_eq!(
            got, want,
            "update the pins on purpose, with the golden cases"
        );
    }
}
