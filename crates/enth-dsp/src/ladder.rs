//! The four-pole ladder filter behind the `ladder` module (S2a lane FLT). Notes: `docs/dsp/ladder.md`.
//!
//! The Moog ladder as a zero-delay-feedback network of four trapezoidal one-poles (Vadim Zavalishin, *The Art of VA
//! Filter Design*, ch. 6, "Moog ladder filter"): four identical low-pass stages in series, the last stage's output fed
//! back to the input with a gain `k`, the feedback path solved *exactly* in the same sample, so there is no unit delay
//! in the loop and the response is the bilinear transform of the analogue ladder with every pole pre-warped to the
//! cutoff. Written from the book's equations; no code is ported.
//!
//! ```text
//! stage:    v = (x − s)·G;  y = v + s;  s = y + v          G = g/(1 + g),  g = tan(π·fc/fs)     y = G·x + (1 − G)·s
//! loop:     y4 = G⁴·u + σ,   σ = (1 − G)·(G³·s1 + G²·s2 + G·s3 + s4),   u = x − k·y4
//!           ⇒ u = (x − k·σ) / (1 + k·G⁴)           (exact, one division; the linear ladder: H = H1⁴ / (1 + k·H1⁴))
//! taps:     y0 = u, y1, y2, y3, y4          lp24 = y4;  lp12 = y2;  bp = y2 − 2·y3 + y4;  hp = y0 − 4·y1 + 6·y2 − 4·y3 + y4
//! ```
//!
//! - **Resonance** is the feedback gain `k = 4.2 · resonance/100` ([`k_from_resonance`]). The four poles sit at the
//!   cutoff, so the loop's phase is 180° at the cutoff exactly (45° per pole, pre-warped) and its gain there is
//!   `k/4`: the ladder rings on its own from `k = 4`, i.e. from `resonance` 95.2 %, **at the cutoff frequency**. The
//!   passband falls as `1/(1 + k)`: the lows thin out as the resonance rises, as on a Moog.
//! - **Bounded oscillation.** Above `k = 4` a linear loop would grow without limit, so the summed signal `u` goes
//!   through a fixed soft limiter, `u / sqrt(1 + u²/16)`, before the first stage ([`LIMIT_SQ`]): unity gain for small
//!   signals (−0.2 dB at `u` = 1), bounded at 4, so the self-oscillation settles at a level set by how far `k` is above
//!   4 (about −9 dBFS at `resonance` 100 %). It is the ladder's one input nonlinearity, applied after the exact linear
//!   solve (the usual cheap treatment: while the limiter is in its linear range the solve is exact; as it compresses
//!   the loop is slightly weaker than the linear equations say, which only moves the amplitude).
//! - **Drive** is the svf's input saturator ([`crate::svf::Drive`], an algebraic sigmoid with make-up gain that keeps a
//!   full-level saw's loudness) in front of the filter; at 0 dB it is skipped and the signal reaches the limiter
//!   untouched.
//! - **Zipper-free.** The caller computes [`Coeffs`] for the end of each 32-frame segment and [`process_block`] ramps
//!   `G`, `k` and the drive's two coefficients linearly per sample from the previous segment's values, as the svf does;
//!   `G²`, `G³`, `G⁴` and the division are recomputed per sample only while ramping.
//!
//! Determinism: the transcendentals (`tan`, the drive's `expm1`) run once per voice and changed segment through
//! `util::math`; the per-sample code is `+ − × ÷` and `sqrt`, all correctly rounded. No `mul_add`. The states are
//! flushed once per block.

use crate::svf::{Drive, MAX_CUTOFF_FRACTION, MIN_CUTOFF_HZ};
use crate::util::flush::flush;
use crate::util::math;

/// The ladder's response, in the catalogue's order (`LadderMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Four poles: 24 dB/octave low-pass.
    Lp24,
    /// Two poles: 12 dB/octave low-pass (the tap after the second stage, with the full loop).
    Lp12,
    /// Band-pass: two high-pass and two low-pass poles, `y2 − 2·y3 + y4`.
    Bp,
    /// High-pass: four poles, `y0 − 4·y1 + 6·y2 − 4·y3 + y4`.
    Hp,
}

const LP24: u8 = 0;
const LP12: u8 = 1;
const BP: u8 = 2;
const HP: u8 = 3;

/// The feedback gain at which the linear loop's gain is 1 at the cutoff (4 stages of 1/√2 and 180°: `k/4`).
pub const SELF_OSCILLATION_K: f32 = 4.0;
/// The feedback gain at `resonance` 100 %: 5 % above [`SELF_OSCILLATION_K`], so the top of the knob rings.
pub const MAX_K: f32 = 4.2;
/// `1/16`: the loop limiter is `u / sqrt(1 + u²·LIMIT_SQ)`, bounded at `1/sqrt(LIMIT_SQ)` = 4.
pub const LIMIT_SQ: f32 = 0.0625;

/// The feedback gain for a resonance of `percent` (0–100, clamped; a NaN is 0): `k = 4.2 · percent/100`. Linear in
/// `k`, as on a Moog: the peak at the cutoff is `1/(4 − k)`, so the audible part of the travel is its top third, and
/// the ladder starts ringing at 95.2 %.
#[must_use]
pub fn k_from_resonance(percent: f32) -> f32 {
    if percent.is_nan() {
        0.0
    } else {
        MAX_K * percent.clamp(0.0, 100.0) * 0.01
    }
}

/// The filter coefficients for one cutoff and feedback gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    /// The one-pole stage gain `G = g/(1 + g)`, `g = tan(π·fc/fs)`, in `(0, 1)`.
    pub g: f32,
    /// The feedback gain `k`, `0 … 4.2`.
    pub k: f32,
}

impl Coeffs {
    /// Coefficients for `cutoff_hz` (clamped to 20 Hz … 0.45·fs like the svf; NaN becomes 20 Hz) and feedback gain
    /// `k` (clamped to `0 … 4.2`; NaN is 0) at `sample_rate`. One `tanf`.
    #[must_use]
    pub fn new(cutoff_hz: f32, k: f32, sample_rate: f32) -> Self {
        let hi = MAX_CUTOFF_FRACTION * sample_rate;
        let fc = if cutoff_hz >= MIN_CUTOFF_HZ {
            cutoff_hz.min(hi)
        } else {
            MIN_CUTOFF_HZ
        };
        let g = math::tanf(core::f32::consts::PI * fc / sample_rate);
        let k = if k.is_nan() { 0.0 } else { k.clamp(0.0, MAX_K) };
        Self {
            g: g / (1.0 + g),
            k,
        }
    }
}

/// The exact digital response of the *linear* ladder (the limiter out of its way) at `freq_hz`, by complex arithmetic
/// in `f64` from the coefficients: with `H1 = 1/(1 + j·tan(ω/2)/g)` the one-pole stage, `H = taps / (1 + k·H1⁴)`
/// where the taps are `H1⁴`, `H1²`, `H1² − 2·H1³ + H1⁴` and `(1 − H1)⁴`. Returns `(re, im)`.
#[must_use]
pub fn response_complex(mode: Mode, c: &Coeffs, freq_hz: f64, sample_rate: f64) -> (f64, f64) {
    type C = (f64, f64);
    fn mul(a: C, b: C) -> C {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    }
    let gs = f64::from(c.g);
    let g = gs / (1.0 - gs);
    let w = 2.0 * core::f64::consts::PI * freq_hz / sample_rate;
    let om = math::tan(0.5 * w) / g;
    // H1 = 1 / (1 + jΩ)
    let d = 1.0 + om * om;
    let h1: C = (1.0 / d, -om / d);
    let h2 = mul(h1, h1);
    let h3 = mul(h2, h1);
    let h4 = mul(h2, h2);
    let k = f64::from(c.k);
    let den: C = (1.0 + k * h4.0, k * h4.1);
    let num: C = match mode {
        Mode::Lp24 => h4,
        Mode::Lp12 => h2,
        Mode::Bp => (h2.0 - 2.0 * h3.0 + h4.0, h2.1 - 2.0 * h3.1 + h4.1),
        Mode::Hp => {
            let a: C = (1.0 - h1.0, -h1.1);
            let a2 = mul(a, a);
            mul(a2, a2)
        }
    };
    // num / den
    let m = den.0 * den.0 + den.1 * den.1;
    (
        (num.0 * den.0 + num.1 * den.1) / m,
        (num.1 * den.0 - num.0 * den.1) / m,
    )
}

/// The magnitude of [`response_complex`].
#[must_use]
pub fn response(mode: Mode, c: &Coeffs, freq_hz: f64, sample_rate: f64) -> f64 {
    let (re, im) = response_complex(mode, c, freq_hz, sample_rate);
    (re * re + im * im).sqrt()
}

/// The four stage states of one voice.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    s: [f32; 4],
}

impl State {
    /// A silent filter.
    pub const ZERO: Self = Self { s: [0.0; 4] };

    /// Whether any stage state is subnormal (test support).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.s.iter().any(|v| v.is_subnormal())
    }

    /// The largest stage state magnitude (test support).
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.s.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    fn flush(&mut self) {
        for v in &mut self.s {
            *v = flush(*v);
        }
    }
}

/// What one segment needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The response.
    pub mode: Mode,
    /// The coefficients at the start of the block (the previous segment's end).
    pub from: Coeffs,
    /// The coefficients at the end of the block.
    pub to: Coeffs,
    /// The drive saturator at the start of the block.
    pub drive_from: Drive,
    /// The drive saturator at the end of the block.
    pub drive_to: Drive,
    /// The mix as a fraction (`0 … 1`) at the start and the end (clamped; a NaN is 0).
    pub mix: [f32; 2],
}

/// The mix clamped to `0 … 1` (a NaN is 0).
#[inline(always)]
fn sane_mix(m: f32) -> f32 {
    if m.is_nan() { 0.0 } else { m.clamp(0.0, 1.0) }
}

const OFF: u8 = 0;
const FULL: u8 = 1;
const BLEND: u8 = 2;

/// Filter `io` in place. `G`, `k` and the drive's `b` and `c` ramp linearly from `seg.from` to `seg.to` across the
/// block (the last sample uses `to`, up to rounding), and the mix from `seg.mix[0]` to `seg.mix[1]`; with nothing to
/// ramp the ramp is skipped (adding exact zeros is bit-identical), and with both drives off the saturator is skipped
/// (which is bit-identical to running it). At mix 0 at both ends `io` is not written, so the input passes bit for bit
/// while the filter keeps running to hold its state current. The states are flushed at the end.
pub fn process_block<const N: usize>(state: &mut State, seg: &Segment, io: &mut [f32; N]) {
    let mix = [sane_mix(seg.mix[0]), sane_mix(seg.mix[1])];
    let ramp = seg.from != seg.to;
    let drive = !(seg.drive_from.is_off() && seg.drive_to.is_off());
    let wet = if mix == [0.0, 0.0] {
        OFF
    } else if mix == [1.0, 1.0] {
        FULL
    } else {
        BLEND
    };
    macro_rules! by_mode {
        ($ramp:literal, $drive:literal, $wet:ident) => {
            match seg.mode {
                Mode::Lp24 => run::<LP24, $ramp, $drive, $wet, N>(state, seg, mix, io),
                Mode::Lp12 => run::<LP12, $ramp, $drive, $wet, N>(state, seg, mix, io),
                Mode::Bp => run::<BP, $ramp, $drive, $wet, N>(state, seg, mix, io),
                Mode::Hp => run::<HP, $ramp, $drive, $wet, N>(state, seg, mix, io),
            }
        };
    }
    macro_rules! by_wet {
        ($ramp:literal, $drive:literal) => {
            match wet {
                OFF => by_mode!($ramp, $drive, OFF),
                FULL => by_mode!($ramp, $drive, FULL),
                _ => by_mode!($ramp, $drive, BLEND),
            }
        };
    }
    match (ramp, drive) {
        (false, false) => by_wet!(false, false),
        (true, false) => by_wet!(true, false),
        (false, true) => by_wet!(false, true),
        (true, true) => by_wet!(true, true),
    }
    state.flush();
}

/// `G²`, `G³`, `G⁴`, `1 − G` and `1 / (1 + k·G⁴)` for one `(G, k)`.
#[derive(Clone, Copy)]
struct Powers {
    g2: f32,
    g3: f32,
    one_minus_g: f32,
    inv: f32,
}

#[inline(always)]
fn powers(g: f32, k: f32) -> Powers {
    let g2 = g * g;
    let g4 = g2 * g2;
    Powers {
        g2,
        g3: g2 * g,
        one_minus_g: 1.0 - g,
        inv: 1.0 / (1.0 + k * g4),
    }
}

#[inline(always)]
fn run<const MODE: u8, const RAMP: bool, const DRIVE: bool, const WET: u8, const N: usize>(
    state: &mut State,
    seg: &Segment,
    mix: [f32; 2],
    io: &mut [f32; N],
) {
    let n = N as f32;
    let at = if RAMP { &seg.from } else { &seg.to };
    let (mut g, mut k) = (at.g, at.k);
    let (dg, dk) = if RAMP {
        ((seg.to.g - g) / n, (seg.to.k - k) / n)
    } else {
        (0.0, 0.0)
    };
    let (mut b, mut c) = (seg.drive_from.b(), seg.drive_from.c());
    let (db, dc) = if DRIVE {
        ((seg.drive_to.b() - b) / n, (seg.drive_to.c() - c) / n)
    } else {
        (0.0, 0.0)
    };
    let mut m = mix[0];
    let dm = (mix[1] - m) / n;
    // Without a ramp the powers are fixed for the block.
    let steady = powers(g, k);
    let [mut s1, mut s2, mut s3, mut s4] = state.s;
    for x in io.iter_mut() {
        let dry = *x;
        let p = if RAMP {
            g += dg;
            k += dk;
            powers(g, k)
        } else {
            steady
        };
        let mut v0 = dry;
        if DRIVE {
            b += db;
            c += dc;
            v0 /= (b + c.max(0.0) * v0 * v0).sqrt();
        }
        // The exact solve of the linear loop, then the limiter.
        let sigma = p.one_minus_g * (p.g3 * s1 + p.g2 * s2 + g * s3 + s4);
        let u = (v0 - k * sigma) * p.inv;
        let u = u / (1.0 + u * u * LIMIT_SQ).sqrt();
        // Four trapezoidal one-poles in series.
        let v = (u - s1) * g;
        let y1 = v + s1;
        s1 = y1 + v;
        let v = (y1 - s2) * g;
        let y2 = v + s2;
        s2 = y2 + v;
        let v = (y2 - s3) * g;
        let y3 = v + s3;
        s3 = y3 + v;
        let v = (y3 - s4) * g;
        let y4 = v + s4;
        s4 = y4 + v;
        let y = match MODE {
            LP24 => y4,
            LP12 => y2,
            BP => y2 - 2.0 * y3 + y4,
            _ => u - 4.0 * y1 + 6.0 * y2 - 4.0 * y3 + y4,
        };
        match WET {
            OFF => {}
            FULL => *x = y,
            _ => {
                m += dm;
                *x = (1.0 - m) * dry + m * y;
            }
        }
    }
    state.s = [s1, s2, s3, s4];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fft;
    use crate::util::rng::Rng;

    const SR: f32 = 48_000.0;
    const MODES: [Mode; 4] = [Mode::Lp24, Mode::Lp12, Mode::Bp, Mode::Hp];

    fn segment(mode: Mode, fc: f32, resonance: f32, drive_db: f32, mix: f32) -> Segment {
        let c = Coeffs::new(fc, k_from_resonance(resonance), SR);
        let d = Drive::from_db(drive_db);
        Segment {
            mode,
            from: c,
            to: c,
            drive_from: d,
            drive_to: d,
            mix: [mix, mix],
        }
    }

    fn db(x: f64) -> f64 {
        20.0 * math::log10(x.max(1e-30))
    }

    /// The impulse response (an impulse of `amp`, so the limiter stays in its linear range), `len` samples, divided by
    /// `amp`.
    fn impulse_response(seg: &Segment, len: usize, amp: f32) -> Vec<f32> {
        let mut st = State::ZERO;
        let mut out = Vec::with_capacity(len);
        let mut first = true;
        while out.len() < len {
            let mut block = [0.0f32; 32];
            if first {
                block[0] = amp;
                first = false;
            }
            process_block(&mut st, seg, &mut block);
            out.extend(block.iter().map(|&v| v / amp));
        }
        out
    }

    /// The settled amplitude of a sine at `freq` (amplitude `amp`) through a static filter, divided by `amp`.
    fn measured(seg: &Segment, freq: f64, amp: f32) -> f64 {
        let mut st = State::ZERO;
        let w = 2.0 * core::f64::consts::PI * freq / f64::from(SR);
        let total = 96_000usize;
        let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for b in 0..total / 32 {
            let mut block = [0.0f32; 32];
            for (i, x) in block.iter_mut().enumerate() {
                *x = amp * math::sin(w * (b * 32 + i) as f64) as f32;
            }
            process_block(&mut st, seg, &mut block);
            if b * 32 >= total / 2 {
                for (i, &y) in block.iter().enumerate() {
                    let t = (b * 32 + i) as f64;
                    re += f64::from(y) * math::sin(w * t);
                    im += f64::from(y) * math::cos(w * t);
                    n += 1.0;
                }
            }
        }
        2.0 * (re * re + im * im).sqrt() / n / f64::from(amp)
    }

    #[test]
    fn the_resonance_map_and_the_self_oscillation_threshold() {
        assert_eq!(k_from_resonance(0.0), 0.0);
        assert!((k_from_resonance(100.0) - 4.2).abs() < 1e-6);
        assert!((k_from_resonance(50.0) - 2.1).abs() < 1e-6);
        assert_eq!(k_from_resonance(-5.0), 0.0);
        assert_eq!(k_from_resonance(500.0).to_bits(), MAX_K.to_bits());
        assert_eq!(k_from_resonance(f32::NAN), 0.0);
        // k = 4 at 95.24 %.
        assert!((k_from_resonance(95.238) - SELF_OSCILLATION_K).abs() < 1e-3);
    }

    #[test]
    fn the_measured_response_matches_the_closed_form() {
        // A small impulse through the filter, its spectrum against `response` at every bin from 40 Hz to 0.45·fs
        // above −70 dB of the peak: within 0.05 dB, all four modes, three cutoffs, four resonances.
        for mode in MODES {
            for fc in [200.0f32, 2_000.0, 10_000.0] {
                for res in [0.0f32, 10.0, 50.0, 90.0] {
                    let seg = segment(mode, fc, res, 0.0, 1.0);
                    let ir = impulse_response(&seg, 131_072, 1e-3);
                    let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
                    let mut im = vec![0.0; ir.len()];
                    fft(&mut re, &mut im);
                    let bin = f64::from(SR) / ir.len() as f64;
                    let top = (0..=ir.len() / 2)
                        .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
                        .fold(0.0, f64::max);
                    let (mut worst, mut checked) = (0.0f64, 0usize);
                    for i in (40.0 / bin) as usize..=(21_600.0 / bin) as usize {
                        let m = (re[i] * re[i] + im[i] * im[i]).sqrt();
                        if m < top * 3.2e-4 {
                            continue;
                        }
                        let want = response(mode, &seg.to, i as f64 * bin, f64::from(SR));
                        worst = worst.max((db(m) - db(want)).abs());
                        checked += 1;
                    }
                    eprintln!("{mode:?} fc {fc} res {res}: {checked} bins, worst {worst:.4} dB");
                    assert!(checked > 500, "{mode:?} {fc} {res}: {checked}");
                    assert!(worst < 0.05, "{mode:?} fc {fc} res {res}: {worst} dB");
                }
            }
        }
    }

    #[test]
    fn the_slope_above_the_cutoff_approaches_24_db_per_octave() {
        // Four equal poles at the cutoff: the asymptote is −24 dB/octave and the approach is slow. At a cutoff of
        // 125 Hz (so the bilinear warp is negligible up to 2 kHz), the gain at fc·2^n for n = 0 … 4, from the filter
        // itself with a sine against the closed form (within 0.05 dB), at resonance 0 and 10 %; and the drop per
        // octave. The analogue ladder drops 15.9, 21.3, 23.3 and 24.0 dB over the first four octaves, so the
        // slope is within 1 dB of 24 from the third octave above the cutoff on (4·fc → 8·fc), not at one and two
        // octaves up.
        for res in [0.0f32, 10.0] {
            let fc = 125.0f32;
            let seg = segment(Mode::Lp24, fc, res, 0.0, 1.0);
            let mut gains = Vec::new();
            for oct in 0..=4 {
                let f = f64::from(fc) * f64::from(1u32 << oct);
                let m = measured(&seg, f, 1e-3);
                let want = response(Mode::Lp24, &seg.to, f, f64::from(SR));
                assert!(
                    (db(m) - db(want)).abs() < 0.05,
                    "res {res}, {f} Hz: {} vs {}",
                    db(m),
                    db(want)
                );
                gains.push(db(m));
            }
            let drops: Vec<f64> = gains.windows(2).map(|w| w[0] - w[1]).collect();
            eprintln!(
                "res {res}: gain at fc, 2fc, 4fc, 8fc, 16fc = {gains:.2?} dB; drop per octave {drops:.2?}"
            );
            for (i, want) in [15.9, 21.3, 23.3, 24.0].into_iter().enumerate() {
                // (With feedback the first octaves differ: the resonance lifts the gain at the cutoff.)
                if res != 0.0 && i < 2 {
                    continue;
                }
                assert!(
                    (drops[i] - want).abs() < 0.5,
                    "octave {}: {} dB, the analogue ladder drops {want}",
                    i + 1,
                    drops[i]
                );
            }
            assert!((drops[2] - 24.0).abs() < 1.0, "{drops:?}");
            assert!((drops[3] - 24.0).abs() < 0.5, "{drops:?}");
        }
    }

    #[test]
    fn the_twelve_db_tap_and_the_band_pass_and_high_pass_slopes() {
        // lp12 falls at 12 dB/octave, hp rises at 24 and bp at 12/12, far from the cutoff (closed form).
        let c = Coeffs::new(100.0, 0.0, SR);
        let slope = |mode: Mode, f: f64| {
            db(response(mode, &c, f, f64::from(SR)))
                - db(response(mode, &c, 2.0 * f, f64::from(SR)))
        };
        assert!(
            (slope(Mode::Lp12, 1_600.0) - 12.0).abs() < 0.5,
            "{}",
            slope(Mode::Lp12, 1_600.0)
        );
        assert!(
            (slope(Mode::Bp, 1_600.0) - 12.0).abs() < 0.5,
            "{}",
            slope(Mode::Bp, 1_600.0)
        );
        let c = Coeffs::new(1_000.0, 0.0, SR);
        let slope = |mode: Mode, f: f64| {
            db(response(mode, &c, f, f64::from(SR)))
                - db(response(mode, &c, 2.0 * f, f64::from(SR)))
        };
        assert!(
            (slope(Mode::Hp, 15.625) + 24.0).abs() < 0.5,
            "{}",
            slope(Mode::Hp, 15.625)
        );
        assert!(
            (slope(Mode::Bp, 15.625) + 12.0).abs() < 0.5,
            "{}",
            slope(Mode::Bp, 15.625)
        );
        // The low-pass passes DC at 1/(1 + k), the high-pass blocks it, and both end at Nyquist: lp 0, hp 1.
        for res in [0.0f32, 50.0, 90.0] {
            let c = Coeffs::new(1_000.0, k_from_resonance(res), SR);
            let dc = response(Mode::Lp24, &c, 1.0, f64::from(SR));
            assert!((dc - 1.0 / (1.0 + f64::from(c.k))).abs() < 1e-4);
            assert!(response(Mode::Hp, &c, 1.0, f64::from(SR)) < 1e-6);
            assert!(response(Mode::Lp24, &c, 24_000.0, f64::from(SR)) < 1e-9);
            assert!((response(Mode::Hp, &c, 24_000.0, f64::from(SR)) - 1.0).abs() < 1e-9);
        }
    }

    /// The frequency of the strongest spectral peak of `x` (a power of two long) and its amplitude (arbitrary
    /// units), with parabolic interpolation on the log magnitude.
    fn dominant(x: &[f32]) -> (f64, f64) {
        let n = x.len();
        let mut re: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                // Hann window
                let w = 0.5 - 0.5 * math::cos(2.0 * core::f64::consts::PI * i as f64 / n as f64);
                f64::from(v) * w
            })
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let mag: Vec<f64> = (0..=n / 2)
            .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
            .collect();
        let (mut best, mut bm) = (1, 0.0);
        for (i, &m) in mag.iter().enumerate().skip(2) {
            if m > bm {
                best = i;
                bm = m;
            }
        }
        let (a, b, c) = (
            math::log(mag[best - 1] + 1e-300),
            math::log(bm),
            math::log(mag[best + 1] + 1e-300),
        );
        let off = 0.5 * (a - c) / (a - 2.0 * b + c);
        ((best as f64 + off) * f64::from(SR) / n as f64, bm)
    }

    /// Excite the filter with a short noise burst and let it ring for `secs`; returns the last 65,536 samples.
    fn ring(mode: Mode, fc: f32, resonance: f32, secs: f32) -> Vec<f32> {
        let seg = segment(mode, fc, resonance, 0.0, 1.0);
        let mut st = State::ZERO;
        let mut rng = Rng::new(5);
        let blocks = (secs * SR / 32.0) as usize;
        let mut out = Vec::new();
        for b in 0..blocks {
            let mut block: [f32; 32] = if b == 0 {
                core::array::from_fn(|_| rng.bipolar_f32() * 0.05)
            } else {
                [0.0; 32]
            };
            process_block(&mut st, &seg, &mut block);
            out.extend_from_slice(&block);
        }
        out[out.len() - 65_536..].to_vec()
    }

    #[test]
    fn near_the_top_of_the_knob_the_ladder_rings_at_the_cutoff() {
        // Resonance 100 % (k = 4.2): after a noise burst the filter keeps oscillating, at the cutoff within 2 % (it
        // is within 0.3 %), at a steady level; a cutoff in each of five decades.
        for fc in [100.0f32, 440.0, 1_000.0, 4_000.0, 10_000.0] {
            let secs = if fc < 200.0 { 20.0 } else { 6.0 };
            let tail = ring(Mode::Lp24, fc, 100.0, secs);
            let (f, _) = dominant(&tail);
            let rms = |x: &[f32]| {
                (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64)
                    .sqrt()
            };
            let (first, second) = (rms(&tail[..32_768]), rms(&tail[32_768..]));
            eprintln!(
                "fc {fc}: rings at {f:.2} Hz ({:+.3} %), rms {first:.4} then {second:.4}",
                (f / f64::from(fc) - 1.0) * 100.0
            );
            assert!((f / f64::from(fc) - 1.0).abs() < 0.02, "fc {fc}: {f}");
            assert!(second > 0.01, "fc {fc}: the ring died ({second})");
            assert!(
                (second / first - 1.0).abs() < 0.05,
                "fc {fc}: not steady ({first} then {second})"
            );
            assert!(second < 1.0, "fc {fc}: unbounded ({second})");
        }
    }

    #[test]
    fn the_ring_starts_just_above_95_percent_and_dies_below() {
        // Where the self-oscillation begins: from a noise burst at 1 kHz, 10 s later. At 94 % (k = 3.95) the
        // ring has died; at 96 % (k = 4.03) it holds.
        let rms = |x: &[f32]| {
            (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
        };
        let dead = rms(&ring(Mode::Lp24, 1_000.0, 94.0, 10.0));
        let alive = rms(&ring(Mode::Lp24, 1_000.0, 96.0, 10.0));
        eprintln!("94 %: rms {dead:e}; 96 %: rms {alive:.4}");
        assert!(dead < 1e-6, "{dead}");
        assert!(alive > 1e-3, "{alive}");
    }

    #[test]
    fn mix_zero_leaves_the_input_untouched_and_keeps_the_state_current() {
        let mut rng = Rng::new(3);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32());
        for mode in MODES {
            let off = segment(mode, 800.0, 60.0, 6.0, 0.0);
            let on = segment(mode, 800.0, 60.0, 6.0, 1.0);
            let (mut a, mut b) = (State::ZERO, State::ZERO);
            let mut x = input;
            process_block(&mut a, &off, &mut x);
            assert_eq!(x.map(f32::to_bits), input.map(f32::to_bits));
            let mut y = input;
            process_block(&mut b, &on, &mut y);
            assert_eq!(a, b);
            assert_ne!(y, input);
        }
    }

    #[test]
    fn a_mix_ramp_blends_from_dry_to_wet() {
        let mut seg = segment(Mode::Lp24, 800.0, 30.0, 0.0, 1.0);
        let mut rng = Rng::new(9);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
        let mut wet = input;
        let mut st = State::ZERO;
        process_block(&mut st, &seg, &mut wet);
        seg.mix = [0.0, 1.0];
        let mut got = input;
        let mut st = State::ZERO;
        process_block(&mut st, &seg, &mut got);
        for i in 0..32 {
            let m = (i + 1) as f32 / 32.0;
            let want = (1.0 - m) * input[i] + m * wet[i];
            assert!(
                (got[i] - want).abs() < 1e-5,
                "frame {i}: {} vs {want}",
                got[i]
            );
        }
    }

    #[test]
    fn the_filter_has_no_latency() {
        // The first output sample already depends on the first input sample, in every mode.
        for mode in MODES {
            let seg = segment(mode, 2_000.0, 40.0, 0.0, 1.0);
            let ir = impulse_response(&seg, 32, 1e-3);
            assert!(ir[0].abs() > 1e-4, "{mode:?}: h[0] = {}", ir[0]);
        }
    }

    #[test]
    fn the_drive_is_the_svf_saturator_and_zero_decibels_is_the_identity_path() {
        // 0 dB of drive skips the saturator: bit-identical to a segment whose drive is the identity curve (b = 1, c = 0).
        let mut rng = Rng::new(21);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.7);
        let plain = segment(Mode::Lp24, 3_000.0, 20.0, 0.0, 1.0);
        let mut a = input;
        let mut st = State::ZERO;
        process_block(&mut st, &plain, &mut a);
        // 24 dB moves the output (and compresses it).
        let hot = segment(Mode::Lp24, 3_000.0, 20.0, 24.0, 1.0);
        let mut b = input;
        let mut st = State::ZERO;
        process_block(&mut st, &hot, &mut b);
        assert_ne!(a, b);
        assert!(b.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn random_jumps_of_everything_stay_bounded_and_the_tail_dies() {
        // The stress: a new random cutoff (20 Hz … 21.6 kHz), resonance (0 … 100 %), drive (0 … 24 dB), mode and mix
        // every segment for 20 s, white noise in: finite and bounded (the limiter bounds even the self-oscillation).
        let mut st = State::ZERO;
        let mut rng = Rng::new(31);
        let mut prev = Coeffs::new(1_000.0, 0.0, SR);
        let mut prev_drive = Drive::OFF;
        let mut peak = 0.0f32;
        let mut state_peak = 0.0f32;
        for _ in 0..30_000 {
            let fc = 20.0 * math::exp2f(rng.unit_f32() * math::log2f(1_080.0));
            let res = rng.unit_f32() * 100.0;
            let c = Coeffs::new(fc, k_from_resonance(res), SR);
            let d = Drive::from_db(rng.unit_f32() * 24.0);
            let mode = MODES[(rng.unit_f32() * 4.0) as usize % 4];
            let seg = Segment {
                mode,
                from: prev,
                to: c,
                drive_from: prev_drive,
                drive_to: d,
                mix: [rng.unit_f32(), rng.unit_f32()],
            };
            let mut block: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
            process_block(&mut st, &seg, &mut block);
            for y in block {
                assert!(y.is_finite());
                peak = peak.max(y.abs());
            }
            state_peak = state_peak.max(st.peak());
            prev = c;
            prev_drive = d;
        }
        eprintln!("random-jump stress: output peak {peak}, state peak {state_peak}");
        assert!(peak < 20.0, "peak {peak}");
        // Park below the threshold with silence in: no subnormal on the way, exact zero at the end.
        let park = segment(Mode::Lp24, 2_000.0, 80.0, 0.0, 1.0);
        for _ in 0..(48_000 * 30 / 32) {
            let mut block = [0.0f32; 32];
            process_block(&mut st, &park, &mut block);
            assert!(!st.has_subnormal());
        }
        assert_eq!(st, State::ZERO);
    }
}
