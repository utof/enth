//! The keytracked feedback comb filter behind the `comb` module (S2a lane FLT). Notes: `docs/dsp/comb.md`.
//!
//! One delay line, one first-order low-pass in the loop, a gain, and a fractional read: the feedback comb of Julius
//! O. Smith's *Physical Audio Signal Processing* ("Feedback Comb Filters") with the loop filter of Jaffe and Smith's
//! extended Karplus-Strong string (Computer Music Journal 7(2), 1983), written from those equations. No code is
//! ported.
//!
//! ```text
//! y[n] = (1 − |fb|)·x[n] + fb · lp( y[n − D] )          D = fs / f − (the low-pass's delay at f), in samples
//! out  = (1 − mix)·x + mix·y
//! ```
//!
//! - **The teeth are at the harmonics of `f`** (`fb > 0`) or half way between them (`fb < 0`), because the loop's
//!   total delay at `f` is one period: `D` is the period minus the phase delay the low-pass adds at `f` (closed form,
//!   [`Tuning::new`]), so the fundamental tooth is on pitch at every `damping`; the higher teeth drift a little flat
//!   as the low-pass bends the loop's phase.
//! - **The level is bounded.** The loop's input is scaled by `1 − |fb|`, so `|H| = (1 − |fb|) / |1 − fb·L·I·e^(−jωD)|
//!   ≤ (1 − |fb|) / (1 − |fb|·|L|·|I|) ≤ 1` (`L` the low-pass, `I` the interpolator, both of magnitude at most 1):
//!   the teeth peak at 0 dB for every `fb` and never above, where the bare `x + fb·lp(y[n − D])` peaks at
//!   `1 / (1 − |fb|)` (+6 dB at 50 %, +40 dB at 99 %). Between the teeth the gain is `(1 − |fb|)/(1 + |fb|)`: −9.5 dB
//!   at 50 %, −46 dB at 99 %.
//! - **The fractional read** is a four-point (third-order) Lagrange interpolator on the taps at `m − 1 … m + 2`
//!   (`m = floor(D)`), so `D ≥ 2` and the loop never reads the sample it is writing. Its magnitude response is at most
//!   1 for every fraction (checked in the tests), so it cannot make the loop unstable, and it is flat to better than
//!   0.1 dB below `fs/10`.
//! - **The loop low-pass** is the TPT one-pole (the bilinear transform of `1/(1 + s/ωd)` with the corner pre-warped):
//!   unity at DC, 3 dB down at `damping`, and a zero at Nyquist, so nothing near Nyquist circulates.
//! - **Modulation.** The caller computes a [`Tuning`] for the end of each 32-frame segment; [`process_block`] ramps
//!   the delay, the low-pass coefficient, the feedback and the mix linearly per sample from the previous segment's
//!   values, so a pitch sweep or an LFO on `semi` moves the read position smoothly (the flanger's Doppler) and never
//!   steps. The delay is clamped to `[2, fs/20]` samples, the feedback to ±0.99.
//!
//! Determinism: the transcendentals (`exp2`, `tan`, `atan`) run in [`frequency_hz`] and [`Tuning::new`] through the
//! pinned `libm`, once per voice and changed segment; the per-sample code is `+ − ×` and the integer index maths. No
//! `mul_add`. The loop's write is flushed ([`flush`]) so a decaying tail never reaches subnormals.

use crate::util::flush::flush;
use crate::util::math;

/// The lowest comb frequency: the clamp that bounds the delay line to `fs / 20` samples (50 ms), per the catalogue.
pub const MIN_FREQ_HZ: f32 = 20.0;
/// The highest comb frequency as a fraction of the sample rate (21.6 kHz at 48 kHz, where the period is 2.2 samples).
pub const MAX_FREQ_FRACTION: f32 = 0.45;
/// The shortest delay in samples: the interpolator reads `m − 1 … m + 2`, and `m − 1` must be a sample already written.
pub const MIN_DELAY: f32 = 2.0;
/// The largest feedback magnitude (the catalogue's 99 %).
pub const MAX_FEEDBACK: f32 = 0.99;
/// Middle C in Hz as the catalogue writes it: the pitch of `semi` 0 at `keytrack` 0 %, and of key 60 at 100 %.
pub const MIDDLE_C_HZ: f32 = 261.63;
/// The lowest damping corner the low-pass computes, in Hz (the catalogue's minimum is 500 Hz).
const MIN_DAMPING_HZ: f32 = 20.0;

/// The length of one voice's delay line at `sample_rate`: a power of two holding the longest delay (`fs / 20`) and the
/// interpolator's four taps, so reads wrap with a mask.
#[must_use]
pub fn line_len(sample_rate: f32) -> usize {
    (max_delay(sample_rate).ceil() as usize + 4).next_power_of_two()
}

/// The longest delay in samples, `fs / `[`MIN_FREQ_HZ`].
#[must_use]
pub fn max_delay(sample_rate: f32) -> f32 {
    sample_rate / MIN_FREQ_HZ
}

/// The comb's frequency in Hz: `261.63 · 2^((semi + tracked) / 12)` with `tracked = keytrack · (pitch − 60)`
/// semitones, clamped to [`MIN_FREQ_HZ`]` … `[`MAX_FREQ_FRACTION`]` · sample_rate` (a NaN becomes the minimum).
#[must_use]
pub fn frequency_hz(semi: f32, tracked_semitones: f32, sample_rate: f32) -> f32 {
    let f = MIDDLE_C_HZ * math::exp2f((semi + tracked_semitones) / 12.0);
    if f >= MIN_FREQ_HZ {
        f.min(MAX_FREQ_FRACTION * sample_rate)
    } else {
        MIN_FREQ_HZ
    }
}

/// The loop's delay and low-pass for one comb frequency and damping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// The delay in samples, in `[`[`MIN_DELAY`]`, fs/20]`: one period minus the low-pass's phase delay at the comb
    /// frequency.
    pub delay: f32,
    /// The TPT one-pole's coefficient `G = g / (1 + g)`, `g = tan(π·damping/fs)`, in `(0, 1)`.
    pub lp: f32,
}

impl Tuning {
    /// The tuning for a comb at `freq_hz` (clamped as [`frequency_hz`] does) with the loop low-pass at `damping_hz`
    /// (clamped to 20 Hz … 0.45·fs).
    ///
    /// The loop must delay the signal by exactly one period at `freq_hz`. The low-pass already delays it by
    /// `atan(tan(ω/2)/g) / ω` samples there (`ω = 2π·freq/fs`: the phase of the bilinear one-pole
    /// `1/(1 + j·tan(ω/2)/g)` over the frequency), so the delay line supplies the rest.
    #[must_use]
    pub fn new(freq_hz: f32, damping_hz: f32, sample_rate: f32) -> Self {
        let hi = MAX_FREQ_FRACTION * sample_rate;
        let f = if freq_hz >= MIN_FREQ_HZ {
            freq_hz.min(hi)
        } else {
            MIN_FREQ_HZ
        };
        let fd = if damping_hz >= MIN_DAMPING_HZ {
            damping_hz.min(hi)
        } else {
            MIN_DAMPING_HZ
        };
        let sr = f64::from(sample_rate);
        let g = math::tan(core::f64::consts::PI * f64::from(fd) / sr);
        let w = 2.0 * core::f64::consts::PI * f64::from(f) / sr;
        let lp_delay = libm::atan(math::tan(0.5 * w) / g) / w;
        let delay = (sr / f64::from(f) - lp_delay)
            .clamp(f64::from(MIN_DELAY), f64::from(max_delay(sample_rate)));
        Self {
            delay: delay as f32,
            lp: (g / (1.0 + g)) as f32,
        }
    }
}

/// The comb's exact magnitude response at `freq_hz` for a static `tuning` and `feedback` (a fraction, clamped to
/// ±[`MAX_FEEDBACK`]): `(1 − |fb|) / |1 − fb · L(ω) · I(ω) · e^(−jω(m − 1))|`, with `L` the TPT one-pole
/// `1 / (1 + j·tan(ω/2)/g)` and `I` the Lagrange interpolator of the delay's fraction, in `f64` complex arithmetic.
/// It is the closed form the tests hold the filter to, and the numbers `docs/dsp/comb.md` quotes. At most 1.
#[must_use]
pub fn response(tuning: &Tuning, feedback: f64, freq_hz: f64, sample_rate: f64) -> f64 {
    let fb = feedback.clamp(-f64::from(MAX_FEEDBACK), f64::from(MAX_FEEDBACK));
    let w = 2.0 * core::f64::consts::PI * freq_hz / sample_rate;
    let g_lp = f64::from(tuning.lp);
    let g = g_lp / (1.0 - g_lp);
    let u = math::tan(0.5 * w) / g;
    let (l_re, l_im) = (1.0 / (1.0 + u * u), -u / (1.0 + u * u));
    let d = f64::from(tuning.delay);
    let m = d.floor();
    let c = weights((d - m) as f32);
    let (mut i_re, mut i_im) = (0.0, 0.0);
    for (k, ck) in c.iter().enumerate() {
        let ph = -w * (m - 1.0 + k as f64);
        i_re += f64::from(*ck) * math::cos(ph);
        i_im += f64::from(*ck) * math::sin(ph);
    }
    let (p_re, p_im) = (l_re * i_re - l_im * i_im, l_re * i_im + l_im * i_re);
    let (den_re, den_im) = (1.0 - fb * p_re, -fb * p_im);
    (1.0 - fb.abs()) / (den_re * den_re + den_im * den_im).sqrt()
}

/// One voice's recursive state besides its delay line: the write position and the low-pass memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    write: u32,
    lp: f32,
}

impl State {
    /// A silent comb (the delay line must be zeroed too).
    pub const ZERO: Self = Self { write: 0, lp: 0.0 };

    /// Whether the low-pass memory is subnormal (test support; the line is flushed on every write).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.lp.is_subnormal()
    }

    /// The low-pass memory (test support).
    #[must_use]
    pub fn lp(&self) -> f32 {
        self.lp
    }
}

/// What one segment needs: the tunings, feedback and mix at its start (the previous segment's end) and its end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The tuning at the start of the block.
    pub from: Tuning,
    /// The tuning at the end of the block.
    pub to: Tuning,
    /// The feedback as a fraction (`−0.99 … 0.99`) at the start and the end (clamped to ±[`MAX_FEEDBACK`]).
    pub feedback: [f32; 2],
    /// The mix as a fraction (`0 … 1`) at the start and the end (clamped).
    pub mix: [f32; 2],
}

/// The feedback clamped to ±[`MAX_FEEDBACK`] (a NaN is 0).
#[inline(always)]
fn sane_feedback(fb: f32) -> f32 {
    if fb.is_nan() {
        0.0
    } else {
        fb.clamp(-MAX_FEEDBACK, MAX_FEEDBACK)
    }
}

/// The mix clamped to `0 … 1` (a NaN is 0).
#[inline(always)]
fn sane_mix(m: f32) -> f32 {
    if m.is_nan() { 0.0 } else { m.clamp(0.0, 1.0) }
}

/// The four-point Lagrange weights for the fraction `x` in `[0, 1)` between the taps at delays `m` and `m + 1`, for
/// taps ordered by delay `m − 1, m, m + 1, m + 2`. They sum to 1 and `weights(0)` is `[0, 1, 0, 0]`.
#[inline(always)]
#[must_use]
pub fn weights(x: f32) -> [f32; 4] {
    let xm1 = x - 1.0;
    let xm2 = x - 2.0;
    let xp1 = x + 1.0;
    [
        -x * xm1 * xm2 * (1.0 / 6.0),
        xp1 * xm1 * xm2 * 0.5,
        -xp1 * x * xm2 * 0.5,
        xp1 * x * xm1 * (1.0 / 6.0),
    ]
}

/// How the output blends the comb with the input.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wet {
    /// Mix 0 at both ends: the input passes untouched (bit for bit); the comb still runs to keep its state current.
    Off,
    /// Mix 1 at both ends: the output is the comb's.
    Full,
    /// Anything between.
    Blend,
}

const OFF: u8 = 0;
const FULL: u8 = 1;
const BLEND: u8 = 2;

/// Run the comb over `io` in place. `line` is this voice's delay line, [`line_len`] samples (a power of two).
///
/// The delay, the low-pass coefficient, the feedback and the mix ramp linearly from `seg.from` to `seg.to` across the
/// block; with nothing to ramp the interpolation weights are computed once. At mix 0 at both ends `io` is not written,
/// so the input passes bit for bit. The low-pass memory is flushed at the end.
///
/// # Panics
/// In debug builds, if `line.len()` is not a power of two.
pub fn process_block<const N: usize>(
    line: &mut [f32],
    state: &mut State,
    seg: &Segment,
    io: &mut [f32; N],
) {
    debug_assert!(line.len().is_power_of_two());
    let fb = [
        sane_feedback(seg.feedback[0]),
        sane_feedback(seg.feedback[1]),
    ];
    let mix = [sane_mix(seg.mix[0]), sane_mix(seg.mix[1])];
    let ramp = seg.from != seg.to || fb[0] != fb[1];
    let wet = if mix == [0.0, 0.0] {
        Wet::Off
    } else if mix == [1.0, 1.0] {
        Wet::Full
    } else {
        Wet::Blend
    };
    match (ramp, wet) {
        (false, Wet::Off) => run::<false, OFF, N>(line, state, seg, fb, mix, io),
        (false, Wet::Full) => run::<false, FULL, N>(line, state, seg, fb, mix, io),
        (false, Wet::Blend) => run::<false, BLEND, N>(line, state, seg, fb, mix, io),
        (true, Wet::Off) => run::<true, OFF, N>(line, state, seg, fb, mix, io),
        (true, Wet::Full) => run::<true, FULL, N>(line, state, seg, fb, mix, io),
        (true, Wet::Blend) => run::<true, BLEND, N>(line, state, seg, fb, mix, io),
    }
    state.lp = flush(state.lp);
}

#[inline(always)]
fn run<const RAMP: bool, const WET: u8, const N: usize>(
    line: &mut [f32],
    state: &mut State,
    seg: &Segment,
    fb: [f32; 2],
    mix: [f32; 2],
    io: &mut [f32; N],
) {
    let n = N as f32;
    let mask = line.len() - 1;
    // The values at the start of the block, and (when ramping) their per-sample steps. The first sample uses start +
    // step and the last the end value, as `svf::process_block` does.
    let (mut d, mut g, mut f, mut a) = if RAMP {
        (seg.from.delay, seg.from.lp, fb[0], 1.0 - fb[0].abs())
    } else {
        (seg.to.delay, seg.to.lp, fb[1], 1.0 - fb[1].abs())
    };
    let (dd, dg, df, da) = if RAMP {
        (
            (seg.to.delay - d) / n,
            (seg.to.lp - g) / n,
            (fb[1] - f) / n,
            ((1.0 - fb[1].abs()) - a) / n,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let mut m = mix[0];
    let dm = (mix[1] - m) / n;
    // The read position stays inside the line (and a NaN falls to the minimum), whatever the caller passed.
    let dmax = (line.len() - 4) as f32;
    let place = |d: f32| {
        let d = d.max(MIN_DELAY).min(dmax);
        let whole = d as usize;
        (whole, weights(d - whole as f32))
    };
    // Without a ramp the read position and weights are fixed for the block.
    let steady = place(d);
    let mut w = state.write;
    let mut s = state.lp;
    for x in io.iter_mut() {
        let input = *x;
        let (whole, c) = if RAMP {
            d += dd;
            g += dg;
            f += df;
            a += da;
            place(d)
        } else {
            steady
        };
        // y[n − (m − 1)] … y[n − (m + 2)]: the line holds y[n − 1] at `w − 1`.
        let at = |back: usize| line[(w.wrapping_sub(back as u32) as usize) & mask];
        let delayed =
            c[0] * at(whole - 1) + c[1] * at(whole) + c[2] * at(whole + 1) + c[3] * at(whole + 2);
        // The TPT one-pole.
        let v = (delayed - s) * g;
        let low = v + s;
        s = low + v;
        let y = a * input + f * low;
        line[(w as usize) & mask] = flush(y);
        w = w.wrapping_add(1);
        match WET {
            OFF => {}
            FULL => *x = y,
            _ => {
                m += dm;
                *x = (1.0 - m) * input + m * y;
            }
        }
    }
    state.write = w;
    state.lp = s;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fft;
    use crate::util::rng::Rng;

    const SR: f32 = 48_000.0;

    fn zeroed(sr: f32) -> Vec<f32> {
        vec![0.0; line_len(sr)]
    }

    fn static_segment(freq: f32, damping: f32, fb: f32, mix: f32) -> Segment {
        let t = Tuning::new(freq, damping, SR);
        Segment {
            from: t,
            to: t,
            feedback: [fb, fb],
            mix: [mix, mix],
        }
    }

    /// The impulse response of a static comb, `len` samples.
    fn impulse_response(seg: &Segment, len: usize) -> Vec<f32> {
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let mut out = Vec::with_capacity(len);
        let mut first = true;
        while out.len() < len {
            let mut block = [0.0f32; 32];
            if first {
                block[0] = 1.0;
                first = false;
            }
            process_block(&mut line, &mut st, seg, &mut block);
            out.extend_from_slice(&block);
        }
        out
    }

    /// |H(f)| of the impulse response, one value per FFT bin up to Nyquist.
    fn magnitude(ir: &[f32]) -> Vec<f64> {
        let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
        let mut im = vec![0.0; ir.len()];
        fft(&mut re, &mut im);
        (0..=ir.len() / 2)
            .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
            .collect()
    }

    /// |H| at angular frequency `w` (rad/sample): the module's closed form.
    fn analytic(t: &Tuning, fb: f64, w: f64) -> f64 {
        response(
            t,
            fb,
            w * f64::from(SR) / (2.0 * core::f64::consts::PI),
            f64::from(SR),
        )
    }

    /// The settled amplitude of a sine at `freq` through a static comb (correlation over the last 2^15 samples).
    fn measured(seg: &Segment, freq: f64) -> f64 {
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let w = 2.0 * core::f64::consts::PI * freq / f64::from(SR);
        let total = 131_072usize;
        let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for b in 0..total / 32 {
            let mut block = [0.0f32; 32];
            for (i, x) in block.iter_mut().enumerate() {
                *x = math::sin(w * (b * 32 + i) as f64) as f32;
            }
            process_block(&mut line, &mut st, seg, &mut block);
            if b * 32 >= total - 65_536 {
                for (i, &y) in block.iter().enumerate() {
                    let t = (b * 32 + i) as f64;
                    re += f64::from(y) * math::sin(w * t);
                    im += f64::from(y) * math::cos(w * t);
                    n += 1.0;
                }
            }
        }
        2.0 * (re * re + im * im).sqrt() / n
    }

    fn db(x: f64) -> f64 {
        20.0 * math::log10(x.max(1e-30))
    }

    #[test]
    fn line_length_is_a_power_of_two_holding_fifty_milliseconds() {
        for sr in [
            8_000.0f32, 44_100.0, 48_000.0, 96_000.0, 192_000.0, 384_000.0,
        ] {
            let n = line_len(sr);
            assert!(n.is_power_of_two());
            assert!(n as f32 >= sr / MIN_FREQ_HZ + 4.0, "{sr}: {n}");
            assert!(n as f32 <= 2.0 * (sr / MIN_FREQ_HZ + 4.0), "{sr}: {n}");
        }
        assert_eq!(line_len(48_000.0), 4_096);
        // 20 Hz at 48 kHz is exactly 2,400 samples, and the clamp holds it.
        let t = Tuning::new(1.0, 20_000.0, 48_000.0);
        assert!(t.delay <= 2_400.0 && t.delay > 2_390.0, "{}", t.delay);
    }

    #[test]
    fn the_frequency_follows_the_note_and_is_clamped() {
        // Middle C at semi 0 and 100 % key tracking, an octave up at +12, and key 72 with 100 % tracking.
        assert!((frequency_hz(0.0, 0.0, SR) - 261.63).abs() < 1e-3);
        assert!((frequency_hz(12.0, 0.0, SR) - 523.26).abs() < 1e-2);
        assert!((frequency_hz(0.0, 12.0, SR) - 523.26).abs() < 1e-2);
        // The floor: semi -48 at key 0 (tracked -60) would be 0.5 Hz.
        assert_eq!(frequency_hz(-48.0, -60.0, SR), MIN_FREQ_HZ);
        assert_eq!(frequency_hz(f32::NAN, 0.0, SR), MIN_FREQ_HZ);
        // The ceiling: 0.45·fs.
        assert_eq!(frequency_hz(48.0, 67.0, SR), 0.45 * SR);
    }

    #[test]
    fn the_delay_is_one_period_less_the_low_passs_delay() {
        // Open damping at 1 kHz: period 48 samples, the low-pass (20 kHz) delays 1 kHz by a fraction of a sample.
        let t = Tuning::new(1_000.0, 20_000.0, SR);
        assert!(t.delay < 48.0 && t.delay > 47.0, "{}", t.delay);
        // A dark low-pass delays more: 500 Hz damping at 1 kHz takes about 0.2 ms = 7 samples off... atan(2)/2π.
        let dark = Tuning::new(1_000.0, 500.0, SR);
        let want = 48.0 - 0.176_3 * 48.0;
        assert!((dark.delay - want).abs() < 0.05, "{} vs {want}", dark.delay);
        // The clamp at 2 samples for a comb near Nyquist.
        let top = Tuning::new(40_000.0, 500.0, SR);
        assert_eq!(top.delay, MIN_DELAY);
    }

    #[test]
    fn the_interpolator_never_gains_and_sums_to_one() {
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0, 0.0]);
        let mut worst = 0.0f64;
        for i in 0..=200 {
            let x = i as f32 / 200.0;
            let c = weights(x);
            assert!((c.iter().sum::<f32>() - 1.0).abs() < 1e-6, "{x}");
            for j in 0..=512 {
                let w = core::f64::consts::PI * f64::from(j) / 512.0;
                let (mut re, mut im) = (0.0, 0.0);
                for (k, ck) in c.iter().enumerate() {
                    re += f64::from(*ck) * math::cos(w * k as f64);
                    im -= f64::from(*ck) * math::sin(w * k as f64);
                }
                worst = worst.max((re * re + im * im).sqrt());
            }
        }
        assert!(worst < 1.0 + 1e-6, "{worst}");
    }

    #[test]
    fn the_measured_response_matches_the_closed_form() {
        // A sine through the static comb against |H(e^jw)|, within 0.03 dB, at several settings: a positive and a
        // negative feedback, light and heavy damping, fractional and integer delays.
        for (freq, damping, fb) in [
            (440.0f32, 10_000.0f32, 0.5f32),
            (440.0, 2_000.0, 0.9),
            (261.63, 20_000.0, -0.7),
            (1_000.0, 800.0, 0.95),
            (97.3, 6_000.0, -0.9),
        ] {
            let seg = static_segment(freq, damping, fb, 1.0);
            for k in 0..9 {
                let f = f64::from(freq) * (0.5 + 0.25 * f64::from(k));
                let w = 2.0 * core::f64::consts::PI * f / f64::from(SR);
                let want = analytic(&seg.to, f64::from(fb), w);
                let got = measured(&seg, f);
                assert!(
                    (db(got) - db(want)).abs() < 0.03,
                    "{freq} Hz, damping {damping}, fb {fb}, at {f:.1} Hz: {:.3} dB vs {:.3} dB",
                    db(got),
                    db(want)
                );
            }
        }
    }

    #[test]
    fn the_gain_never_exceeds_0_db_and_the_valleys_are_where_the_formula_says() {
        for fb in [-0.99f32, -0.5, 0.0, 0.3, 0.5, 0.9, 0.99] {
            for (freq, damping) in [
                (110.0f32, 20_000.0f32),
                (261.63, 10_000.0),
                (523.3, 500.0),
                (2_000.0, 20_000.0),
            ] {
                let seg = static_segment(freq, damping, fb, 1.0);
                // A coarse grid over the whole band, and a fine one (±1 %) around each of the first dozen teeth and
                // valleys, because the teeth are only a few hertz wide at high feedback.
                let mut peak = 0.0f64;
                let mut probe = |hz: f64| {
                    let w = 2.0 * core::f64::consts::PI * hz / f64::from(SR);
                    peak = peak.max(analytic(&seg.to, f64::from(fb), w));
                };
                for k in 1..1_000 {
                    probe(24_000.0 * f64::from(k) / 1_000.0);
                }
                for k in 1..=24 {
                    let centre = f64::from(freq) * f64::from(k) * 0.5;
                    for j in -100..=100 {
                        probe(centre * (1.0 + 0.0001 * f64::from(j)));
                    }
                }
                assert!(peak <= 1.0 + 1e-6, "fb {fb}, {freq} Hz: peak {peak}");
                if fb > 0.1 && damping == 20_000.0 {
                    // The teeth reach within 1 dB of 0 dB at open damping (the interpolator and the low-pass droop a
                    // little, and at 99 % a hundredth of a decibel of droop per pass is worth 0.5 dB).
                    assert!(peak > 0.9, "fb {fb}, {freq} Hz: peak {peak}");
                }
            }
        }
        // Between the teeth: (1 − fb)/(1 + fb) for a positive feedback, at one and a half times the comb frequency.
        for fb in [0.5f32, 0.9, 0.99] {
            let seg = static_segment(110.0, 20_000.0, fb, 1.0);
            let w = 2.0 * core::f64::consts::PI * 110.0 * 1.5 / f64::from(SR);
            let valley = analytic(&seg.to, f64::from(fb), w);
            let want = (1.0 - f64::from(fb)) / (1.0 + f64::from(fb));
            assert!(
                (db(valley) - db(want)).abs() < 0.2,
                "{fb}: {} vs {}",
                db(valley),
                db(want)
            );
        }
    }

    #[test]
    fn the_teeth_sit_on_the_harmonics_within_one_percent() {
        // The impulse response's spectrum at three pitches: every tooth below 4 kHz is within 1 % of k·f0 (positive
        // feedback), or of (k − ½)·f0 (negative), at the default damping.
        for (freq, fb) in [
            (65.41f32, 0.9f32),
            (261.63, 0.9),
            (1_046.5, 0.9),
            (261.63, -0.9),
            (261.63, 0.5),
        ] {
            let seg = static_segment(freq, 10_000.0, fb, 1.0);
            let ir = impulse_response(&seg, 131_072);
            let mag = magnitude(&ir);
            let bin = f64::from(SR) / ir.len() as f64;
            let mut checked = 0;
            let mut worst = 0.0f64;
            for k in 1..200 {
                let ideal = if fb > 0.0 {
                    f64::from(freq) * f64::from(k)
                } else {
                    f64::from(freq) * (f64::from(k) - 0.5)
                };
                if ideal > 4_000.0 {
                    break;
                }
                // The largest bin within ±25 % of the tooth spacing.
                let lo = ((ideal - 0.25 * f64::from(freq)) / bin) as usize;
                let hi = ((ideal + 0.25 * f64::from(freq)) / bin) as usize;
                let (best, _) = (lo..=hi)
                    .map(|i| (i, mag[i]))
                    .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
                let found = best as f64 * bin;
                assert!(
                    (found - ideal).abs() / ideal < 0.01,
                    "{freq} Hz, fb {fb}: tooth {k} at {found:.1} Hz, wanted {ideal:.1} Hz"
                );
                worst = worst.max((found - ideal).abs() / ideal);
                checked += 1;
            }
            eprintln!(
                "{freq} Hz, fb {fb}: {checked} teeth, worst error {:.3} %",
                worst * 100.0
            );
            assert!(checked >= 3, "{freq}: only {checked} teeth checked");
        }
    }

    #[test]
    fn a_noise_burst_at_high_feedback_rings_as_a_plucked_string() {
        // Karplus-Strong: the pitch of the ring is the comb frequency (110 Hz → period 436.4 samples).
        let seg = static_segment(110.0, 8_000.0, 0.99, 1.0);
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let mut rng = Rng::new(5);
        let mut out = Vec::new();
        for b in 0..(48_000 / 32) {
            let mut block = [0.0f32; 32];
            if b < 14 {
                for x in &mut block {
                    *x = rng.bipolar_f32();
                }
            }
            process_block(&mut line, &mut st, &seg, &mut block);
            out.extend_from_slice(&block);
        }
        // The ring is still going half a second later and its autocorrelation peaks at one period.
        let tail = &out[24_000..];
        let energy: f32 = tail.iter().map(|v| v * v).sum();
        assert!(energy > 1e-3, "{energy}");
        let lag = |l: usize| tail.iter().zip(&tail[l..]).map(|(a, b)| a * b).sum::<f32>();
        let best = (300..600)
            .max_by(|&a, &b| lag(a).total_cmp(&lag(b)))
            .unwrap();
        assert!((best as f32 - 48_000.0 / 110.0).abs() < 1.5, "{best}");
    }

    #[test]
    fn mix_zero_leaves_the_block_untouched_and_mix_one_is_the_comb() {
        let input: [f32; 32] = core::array::from_fn(|i| ((i * 37) % 11) as f32 / 11.0 - 0.5);
        let mut dry = input;
        dry[3] = -0.0; // even a negative zero must survive
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let seg = static_segment(300.0, 10_000.0, 0.8, 0.0);
        for _ in 0..20 {
            let mut block = dry;
            process_block(&mut line, &mut st, &seg, &mut block);
            assert!(
                block
                    .iter()
                    .zip(&dry)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
        // The state ran: a switch to mix 1 gives the same bits as a comb that was wet all along.
        let wet = Segment {
            mix: [1.0, 1.0],
            ..seg
        };
        let mut line_b = zeroed(SR);
        let mut st_b = State::ZERO;
        for _ in 0..20 {
            let mut block = dry;
            process_block(&mut line_b, &mut st_b, &wet, &mut block);
        }
        let (mut a, mut b) = (dry, dry);
        process_block(&mut line, &mut st, &wet, &mut a);
        process_block(&mut line_b, &mut st_b, &wet, &mut b);
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        // Half way blends the dry and wet signals.
        let half = Segment {
            mix: [0.5, 0.5],
            ..seg
        };
        let mut line_c = zeroed(SR);
        let mut st_c = State::ZERO;
        let mut line_d = zeroed(SR);
        let mut st_d = State::ZERO;
        let mut c = input;
        let mut d = input;
        process_block(&mut line_c, &mut st_c, &half, &mut c);
        process_block(&mut line_d, &mut st_d, &wet, &mut d);
        for i in 0..32 {
            assert!((c[i] - (0.5 * input[i] + 0.5 * d[i])).abs() < 1e-6);
        }
    }

    #[test]
    fn random_jumps_of_every_parameter_stay_bounded_and_the_tail_dies() {
        // The stress: a new random pitch, damping, feedback (±0.99) and mix every segment for 20 s, white noise in.
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let mut rng = Rng::new(21);
        let mut prev = Tuning::new(200.0, 10_000.0, SR);
        let mut prev_fb = 0.0f32;
        let mut peak = 0.0f32;
        for _ in 0..30_000 {
            let f = 20.0 * math::exp2f(rng.unit_f32() * math::log2f(1_000.0));
            let damping = 500.0 * math::exp2f(rng.unit_f32() * math::log2f(40.0));
            let t = Tuning::new(f, damping, SR);
            let fb = (rng.bipolar_f32() * 1.1).clamp(-1.0, 1.0) * 0.99;
            let seg = Segment {
                from: prev,
                to: t,
                feedback: [prev_fb, fb],
                mix: [rng.unit_f32(), rng.unit_f32()],
            };
            let mut block: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
            process_block(&mut line, &mut st, &seg, &mut block);
            for y in block {
                assert!(y.is_finite());
                peak = peak.max(y.abs());
            }
            prev = t;
            prev_fb = fb;
        }
        eprintln!("random-jump stress peak {peak}");
        assert!(peak < 2.0, "peak {peak}");
        // Park at a short period and moderate feedback, silence in: the loop dies out to exact zero.
        let t = Tuning::new(2_000.0, 8_000.0, SR);
        let park = Segment {
            from: t,
            to: t,
            feedback: [0.9, 0.9],
            mix: [1.0, 1.0],
        };
        for _ in 0..(48_000 * 30 / 32) {
            let mut block = [0.0f32; 32];
            process_block(&mut line, &mut st, &park, &mut block);
        }
        assert!(!st.has_subnormal());
        assert!(line.iter().all(|v| !v.is_subnormal()));
        let mut block = [0.0f32; 32];
        process_block(&mut line, &mut st, &park, &mut block);
        assert!(block.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn a_moving_delay_is_smooth_and_a_pitch_sweep_follows_the_ramp() {
        // A sine through a comb whose delay ramps across one block: the output has no step larger than the signal's
        // own slope plus the Doppler (a ramp of 1 sample in 32 changes the read-out slope by at most 3 %).
        let t0 = Tuning::new(200.0, 20_000.0, SR);
        let t1 = Tuning::new(206.0, 20_000.0, SR);
        let seg = Segment {
            from: t0,
            to: t1,
            feedback: [0.5, 0.5],
            mix: [1.0, 1.0],
        };
        let mut line = zeroed(SR);
        let mut st = State::ZERO;
        let w = 2.0 * core::f64::consts::PI * 200.0 / f64::from(SR);
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        for b in 0..200 {
            let mut block: [f32; 32] =
                core::array::from_fn(|i| math::sin(w * (b * 32 + i) as f64) as f32);
            let s = match b {
                ..100 => static_segment(200.0, 20_000.0, 0.5, 1.0),
                100 => seg,
                _ => static_segment(206.0, 20_000.0, 0.5, 1.0),
            };
            process_block(&mut line, &mut st, &s, &mut block);
            for y in block {
                if b > 50 {
                    worst = worst.max((y - last).abs());
                }
                last = y;
            }
        }
        // A sine at 200 Hz has a maximum step of 2π·200/48000 = 0.026; the sum with the comb's ring is below 0.06.
        assert!(worst < 0.06, "{worst}");
    }
}
