//! Oscillator cores (P1a lane D1): the variable-shape polyBLEP/BLAMP core for saw, triangle and pulse with hard
//! sync, a polynomial sine, and the phase accumulator that drives the sine and the wavetable. Docs, sources and
//! measured aliasing: `docs/dsp/osc.md`.
//!
//! # The analog core
//!
//! [`Shape`] is a port of Mutable Instruments Plaits' `VariableShapeOscillator` (MIT, Copyright 2016 Emilie
//! Gillet) with stmlib's polyBLEP residuals (`polyblep.h`, MIT, Copyright 2017 Emilie Gillet; see
//! `THIRD_PARTY_NOTICES.md`). The oscillator computes the *naive* waveform of the next sample, adds a polynomial
//! band-limited step (a BLEP) or ramp (a BLAMP, for the triangle's corners) wherever the waveform has a
//! discontinuity, and outputs the previous step's sum, so a residual that straddles two samples can be split
//! between them: **the output is delayed by one sample**. Everything is `f32` `+ − × ÷`; there are no
//! transcendentals in the per-sample path, so the stream is bit-identical on every platform.
//!
//! Deviations from upstream, all deliberate:
//!
//! - **One wave per instantiation.** Upstream morphs triangle, saw and square with a `waveshape` parameter; enth
//!   has three separate waves, so the core is monomorphised on [`SAW`], [`TRIANGLE`] or [`PULSE`] and the terms
//!   that are multiplied by a constant zero are not there. A pulse is exactly `{0, 1}` before the output map.
//! - **Parameters ramp linearly over the block** from their previous to their current value
//!   (`prev + (cur − prev)·(i+1)/N`, like every enth ramp), where upstream's `ParameterInterpolator` accumulates
//!   an increment. Frequency and pulse width ramp, so glide, vibrato and PWM have no zipper noise.
//! - **Hard sync reads reset times from the master instead of running a master phase inside.** Upstream's
//!   `Render<true, …>` keeps `master_phase_` and computes `reset_time = master_phase_ / master_frequency` at each
//!   wrap. Here the master oscillator writes that same `reset_time` per frame to a bus (`sync_out`) and the slave
//!   reads it (`sync_in`): the fraction of the frame that remained after the master's wrap, `NO_SYNC` (−1) where it
//!   did not wrap. The slave does not need the master's frequency or phase, and any wave can be the master. The
//!   unit tests prove the two forms bit-identical.
//! - **Frequencies are clamped** to `[`[`MIN_DT`]`, `[`MAX_DT_ANALOG`]`]` (a quarter of the sample rate, upstream's
//!   `kMaxFrequency`); pulse width to `[2f, 1 − 2f]`, as upstream. A transition time that falls outside its sample
//!   (inconsistent state after a wave switch) is clamped to the sample and the transition loop is bounded, so
//!   nothing can produce NaN, infinities or a hang whatever the caller passes.
//!
//! # The phased core
//!
//! [`Phased`] is a plain phase accumulator for waves read from a function of the phase ([`sine`] and the wavetable).
//! It uses the same one-sample delay, the same sync bus and the same BLEP at a sync reset, so a synced sine or
//! table has the step at its reset band-limited like the analog waves do.

/// Saw: rises from −1 to +1, one BLEP per cycle.
pub const SAW: u8 = 0;
/// Symmetric triangle: starts at −1, +1 half way; BLAMPs at both corners.
pub const TRIANGLE: u8 = 1;
/// Pulse: −1 for the first `pw` of the cycle, +1 after. Not DC-free (the levels are ±1).
pub const PULSE: u8 = 2;

/// The smallest phase increment (cycles per sample) the cores accept; far below the lowest audible pitch.
pub const MIN_DT: f32 = 1.0e-7;
/// The largest phase increment of the analog core: a quarter of the sample rate (12 kHz at 48 kHz).
pub const MAX_DT_ANALOG: f32 = 0.25;
/// The largest phase increment of the phased core: 0.45 of the sample rate (21.6 kHz at 48 kHz).
pub const MAX_DT_PHASED: f32 = 0.45;

/// The largest `f32` below 1: reset times are in `[0, 1)`.
const BELOW_ONE: f32 = 0.999_999_94;

/// `NO_SYNC` for the cores' sync buses: "this frame did not wrap". Equals `enth_engine::module::NO_SYNC`.
pub const NO_RESET: f32 = -1.0;

/// Clamp a phase increment into `[MIN_DT, max]` (NaN becomes `MIN_DT`).
#[inline(always)]
#[must_use]
pub fn clamp_dt(dt: f32, max: f32) -> f32 {
    dt.max(MIN_DT).min(max)
}

/// Pulse width as the analog core wants it at phase increment `dt`: within `[2·dt, 1 − 2·dt]`, and exactly 0.5 from
/// a quarter of the sample rate up (upstream's rule; NaN becomes `2·dt`).
#[inline(always)]
#[must_use]
pub fn clamp_pw(pw: f32, dt: f32) -> f32 {
    if dt >= 0.25 {
        0.5
    } else {
        pw.max(dt * 2.0).min(1.0 - 2.0 * dt)
    }
}

// stmlib/dsp/polyblep.h (MIT, Copyright 2017 Emilie Gillet): the four residuals. `t` is the fraction of the sample
// *after* the discontinuity.

/// Residual added to the sample that contains a step.
#[inline(always)]
fn this_blep(t: f32) -> f32 {
    0.5 * t * t
}

/// Residual added to the next sample.
#[inline(always)]
fn next_blep(t: f32) -> f32 {
    let t = 1.0 - t;
    -0.5 * t * t
}

/// Integrated residual (a ramp's slope change) for the next sample.
#[inline(always)]
fn next_int_blep(t: f32) -> f32 {
    let t1 = 0.5 * t;
    let t2 = t1 * t1;
    let t4 = t2 * t2;
    0.1875 - t1 + 1.5 * t2 - t4
}

/// Integrated residual for the sample that contains the slope change.
#[inline(always)]
fn this_int_blep(t: f32) -> f32 {
    next_int_blep(1.0 - t)
}

/// `num / den` clamped to `[0, 1]`; 0.5 when the denominator is not positive (an inconsistent state, which cannot
/// arise from a consistent one).
#[inline(always)]
#[allow(
    clippy::manual_clamp,
    reason = "max/min maps a NaN quotient to 0, where clamp would return NaN"
)]
fn fraction(num: f32, den: f32) -> f32 {
    if den > 1.0e-9 {
        (num / den).max(0.0).min(1.0)
    } else {
        0.5
    }
}

/// Linear ramp weights for a block of `N` frames: `(i + 1) / N`, so a ramp reaches its end value on the last frame.
/// Exact for the engine's 32-frame segment (identical to `enth_engine::module::RAMP`).
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

/// The naive (aliasing) sample in `[0, 1]` of wave `WAVE` at `phase`, for pulse width `pw` (the triangle is
/// symmetric: its `pw` is 0.5, so both slopes are 2).
#[inline(always)]
fn naive<const WAVE: u8>(phase: f32, pw: f32) -> f32 {
    match WAVE {
        SAW => phase,
        PULSE => {
            if phase < pw {
                0.0
            } else {
                1.0
            }
        }
        _ => {
            if phase < pw {
                phase * 2.0
            } else {
                1.0 - (phase - pw) * 2.0
            }
        }
    }
}

/// State of one analog oscillator (one voice of saw, triangle or pulse): see the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    phase: f32,
    next_sample: f32,
    previous_pw: f32,
    high: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Self::new(SAW, 0.0, 0.5)
    }
}

impl Shape {
    /// Start wave `wave` at `phase` (`0 ≤ phase < 1`) with the (already clamped, see [`clamp_pw`]) pulse width `pw`.
    /// The first output sample is the naive waveform at `phase`.
    #[must_use]
    pub fn new(wave: u8, phase: f32, pw: f32) -> Self {
        let phase = if (0.0..1.0).contains(&phase) {
            phase
        } else {
            0.0
        };
        let (pw, next_sample, high) = match wave {
            SAW => (0.5, phase, true),
            PULSE => (pw, naive::<PULSE>(phase, pw), phase >= pw),
            _ => (0.5, naive::<TRIANGLE>(phase, 0.5), phase >= 0.5),
        };
        Self {
            phase,
            next_sample,
            previous_pw: pw,
            high,
        }
    }

    /// The oscillator's phase in cycles, `[0, 1)`.
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Whether any state value is subnormal (for the release-tail invariant).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.phase.is_subnormal()
            || self.next_sample.is_subnormal()
            || self.previous_pw.is_subnormal()
    }

    /// Render `N` frames of wave `WAVE`.
    ///
    /// - `dt`: phase increment (cycles per sample) at the start and the end of the block, already clamped with
    ///   [`clamp_dt`]`(…, `[`MAX_DT_ANALOG`]`)`; the increment ramps linearly between them.
    /// - `pw`: pulse width at the start and end, clamped with [`clamp_pw`]; ignored unless `WAVE == PULSE`.
    /// - `sync_in`: `Some` makes this a hard-sync slave: for each frame, `≥ 0` is the fraction of the frame that
    ///   remained after the master's wrap, and restarts the oscillator there with a band-limited reset.
    /// - `sync_out`: `Some` makes this the master: writes the same fraction (in `[0, 1)`) into the frames where
    ///   its own phase wraps, and leaves the other frames untouched (the caller fills them with [`NO_RESET`]).
    /// - `out`: the (delayed) output, −1 to +1.
    #[inline]
    pub fn render<const WAVE: u8, const N: usize>(
        &mut self,
        dt: [f32; 2],
        pw: [f32; 2],
        sync_in: Option<&[f32; N]>,
        mut sync_out: Option<&mut [f32; N]>,
        out: &mut [f32; N],
    ) {
        let (mut phase, mut next, mut high, mut prev_pw) =
            (self.phase, self.next_sample, self.high, self.previous_pw);
        let (d_dt, d_pw) = (dt[1] - dt[0], pw[1] - pw[0]);
        for i in 0..N {
            let w = Ramp::<N>::W[i];
            let f = dt[0] + d_dt * w;
            let pw = if WAVE == PULSE { pw[0] + d_pw * w } else { 0.5 };
            // The triangle's slope change at a corner, per unit of phase: both slopes are 2, so 4·f per sample.
            let tri_step = 4.0 * f;
            let mut this = next;
            next = 0.0;
            let mut reset = false;
            let mut transition_during_reset = false;
            let mut reset_time = 0.0f32;

            if let Some(sync) = sync_in {
                let rt = sync[i];
                if rt >= 0.0 {
                    let rt = rt.min(BELOW_ONE);
                    // Where the slave's own waveform would be at the master's wrap, and the step to cancel.
                    let mut at_reset = phase + (1.0 - rt) * f;
                    reset = true;
                    if at_reset >= 1.0 {
                        at_reset -= 1.0;
                        transition_during_reset = true;
                    }
                    if WAVE != SAW && !high && at_reset >= pw {
                        transition_during_reset = true;
                    }
                    let value = naive::<WAVE>(at_reset, pw);
                    this -= value * this_blep(rt);
                    next -= value * next_blep(rt);
                    reset_time = rt;
                }
            }

            phase += f;
            // Upstream: `while (transition_during_reset || !reset)`; bounded here (two passes are the most a
            // consistent state needs).
            for _ in 0..3 {
                if reset && !transition_during_reset {
                    break;
                }
                if WAVE != SAW && !high {
                    if phase < pw {
                        break;
                    }
                    let t = fraction(phase - pw, prev_pw - pw + f);
                    if WAVE == PULSE {
                        this += this_blep(t);
                        next += next_blep(t);
                    } else {
                        this -= tri_step * this_int_blep(t);
                        next -= tri_step * next_int_blep(t);
                    }
                    high = true;
                }
                if high {
                    if phase < 1.0 {
                        break;
                    }
                    phase -= 1.0;
                    let t = fraction(phase, f);
                    if WAVE == TRIANGLE {
                        this += tri_step * this_int_blep(t);
                        next += tri_step * next_int_blep(t);
                    } else {
                        this -= this_blep(t);
                        next -= next_blep(t);
                    }
                    if let Some(so) = sync_out.as_deref_mut() {
                        so[i] = t.min(BELOW_ONE);
                    }
                    high = WAVE == SAW;
                }
            }

            if reset {
                phase = reset_time * f;
                high = WAVE == SAW;
            }
            next += naive::<WAVE>(phase, pw);
            prev_pw = pw;
            out[i] = 2.0 * this - 1.0;
        }
        self.phase = phase;
        self.next_sample = next;
        self.high = high;
        self.previous_pw = prev_pw;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The sine
// ---------------------------------------------------------------------------------------------------------------

// Odd minimax polynomial for sin(2πq) on q ∈ [−1/4, 1/4], five terms (Remez exchange in 50-digit arithmetic;
// maximum error 3.3e-9, below the f32 grid). Our own derivation, no upstream.
#[allow(
    clippy::approx_constant,
    reason = "the Remez coefficient, deliberately a hair below tau (6.2831852) to balance the error across the quarter turn"
)]
const SIN_C1: f32 = 6.283_185;
const SIN_C3: f32 = -41.341_656;
const SIN_C5: f32 = 81.601_006;
const SIN_C7: f32 = -76.549_78;
const SIN_C9: f32 = 39.536_705;

/// `sin(2π·phase)` for `0 ≤ phase < 1`: fold the phase into a quarter turn and evaluate an odd polynomial. `+ − ×`
/// only, so it is bit-identical everywhere and cheaper than `libm::sinf`; error below 1e-6 (the f32 grid).
#[inline(always)]
#[must_use]
pub fn sine(phase: f32) -> f32 {
    let q = if phase > 0.75 {
        phase - 1.0
    } else if phase > 0.25 {
        0.5 - phase
    } else {
        phase
    };
    let q2 = q * q;
    q * (SIN_C1 + q2 * (SIN_C3 + q2 * (SIN_C5 + q2 * (SIN_C7 + q2 * SIN_C9))))
}

// ---------------------------------------------------------------------------------------------------------------
// The phased core (sine and wavetable)
// ---------------------------------------------------------------------------------------------------------------

/// State of one phase-driven oscillator (one voice of sine or wavetable): see the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phased {
    phase: f32,
    next_sample: f32,
}

impl Phased {
    /// Start at `phase` (`0 ≤ phase < 1`); `first` is the waveform's value there (the first output sample).
    #[must_use]
    pub fn new(phase: f32, first: f32) -> Self {
        let phase = if (0.0..1.0).contains(&phase) {
            phase
        } else {
            0.0
        };
        Self {
            phase,
            next_sample: first,
        }
    }

    /// The oscillator's phase in cycles, `[0, 1)`.
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Whether any state value is subnormal.
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.phase.is_subnormal() || self.next_sample.is_subnormal()
    }

    /// Render `N` frames of `shape(phase, w)`, where `w = (i + 1)/N` is the block's ramp weight (a table reads its
    /// frame position at `pos₀ + (pos₁ − pos₀)·w`). `dt`, `sync_in`, `sync_out` and `out` are as in
    /// [`Shape::render`] with the limit [`MAX_DT_PHASED`]. A sync reset restarts the phase at 0 and band-limits the
    /// jump between the old and the new waveform.
    #[inline]
    pub fn render<const N: usize>(
        &mut self,
        dt: [f32; 2],
        sync_in: Option<&[f32; N]>,
        mut sync_out: Option<&mut [f32; N]>,
        out: &mut [f32; N],
        mut shape: impl FnMut(f32, f32) -> f32,
    ) {
        let (mut phase, mut next) = (self.phase, self.next_sample);
        let d_dt = dt[1] - dt[0];
        for i in 0..N {
            let w = Ramp::<N>::W[i];
            let f = dt[0] + d_dt * w;
            let mut this = next;
            next = 0.0;
            let mut reset_time = -1.0f32;
            if let Some(sync) = sync_in {
                let rt = sync[i];
                if rt >= 0.0 {
                    let rt = rt.min(BELOW_ONE);
                    let mut at_reset = phase + (1.0 - rt) * f;
                    if at_reset >= 1.0 {
                        at_reset -= 1.0;
                    }
                    let jump = shape(0.0, w) - shape(at_reset, w);
                    this += jump * this_blep(rt);
                    next += jump * next_blep(rt);
                    reset_time = rt;
                }
            }
            phase += f;
            if phase >= 1.0 {
                phase -= 1.0;
                if let Some(so) = sync_out.as_deref_mut() {
                    so[i] = fraction(phase, f).min(BELOW_ONE);
                }
            }
            if reset_time >= 0.0 {
                phase = reset_time * f;
            }
            next += shape(phase, w);
            out[i] = this;
        }
        self.phase = phase;
        self.next_sample = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::math;
    use crate::wavetable::testkit::alias_ratio_db;

    const SR: f64 = 48_000.0;
    const N: usize = 32;
    /// A renderer that fills its first argument and writes sync reset times into its second.
    type Step = Box<dyn FnMut(&mut [f32; N], &mut [f32; N])>;

    /// Render `seconds` of wave `WAVE` at a constant `hz`, 32 frames at a time.
    fn tone<const WAVE: u8>(hz: f64, pw: f32, seconds: f64) -> Vec<f32> {
        let dt = clamp_dt((hz / SR) as f32, MAX_DT_ANALOG);
        let pw = clamp_pw(pw, dt);
        let mut o = Shape::new(WAVE, 0.0, pw);
        let mut x = vec![0.0f32; (seconds * SR) as usize / N * N];
        for c in x.as_chunks_mut::<N>().0 {
            o.render::<WAVE, N>([dt, dt], [pw, pw], None, None, c);
        }
        x
    }

    #[test]
    fn the_sine_polynomial_is_accurate_and_exact_at_the_corners() {
        let mut worst = 0.0f64;
        for i in 0..200_000 {
            let phase = i as f32 / 200_000.0;
            let want = math::sin(core::f64::consts::TAU * f64::from(phase));
            worst = worst.max((f64::from(sine(phase)) - want).abs());
        }
        assert!(worst < 1.0e-6, "{worst}");
        assert_eq!(sine(0.0), 0.0);
        assert!((sine(0.25) - 1.0).abs() < 2e-7);
        assert!((sine(0.75) + 1.0).abs() < 2e-7);
        assert!(sine(0.5).abs() < 1e-7);
        assert!(sine(0.25).abs() <= 1.000_000_5);
    }

    #[test]
    fn blep_residuals_match_stmlib() {
        // Spot values of the four polynomials (stmlib/dsp/polyblep.h).
        assert_eq!(this_blep(0.5), 0.125);
        assert_eq!(next_blep(0.5), -0.125);
        assert_eq!(next_int_blep(0.0), 0.1875);
        assert_eq!(next_int_blep(1.0), 0.1875 - 0.5 + 1.5 * 0.25 - 0.0625);
        assert_eq!(this_int_blep(0.25), next_int_blep(0.75));
        // A step is split between two samples: the residuals of one sample pair sum to the area of the BLEP.
        for k in 0..=20 {
            let t = k as f32 / 20.0;
            assert!((this_blep(t) + next_blep(t) - (t - 0.5)).abs() < 1e-6);
        }
    }

    #[test]
    fn saw_triangle_and_pulse_have_the_right_shape_and_peak() {
        for hz in [55.0, 440.0, 3_520.0] {
            let saw = tone::<SAW>(hz, 0.5, 1.0);
            let tri = tone::<TRIANGLE>(hz, 0.5, 1.0);
            let pul = tone::<PULSE>(hz, 0.2, 1.0);
            let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                peak(&saw) <= 1.1 && peak(&saw) > 0.9,
                "saw {hz}: {}",
                peak(&saw)
            );
            assert!(
                peak(&tri) <= 1.1 && peak(&tri) > 0.9,
                "tri {hz}: {}",
                peak(&tri)
            );
            assert!(
                peak(&pul) <= 1.1 && peak(&pul) > 0.9,
                "pulse {hz}: {}",
                peak(&pul)
            );
            // Saw and triangle are DC-free; the 20 % pulse has the DC of its duty cycle: 1 - 2·0.2 = 0.6 low... the
            // mean of (-1 for 20 %, +1 for 80 %) is +0.6.
            let mean = |x: &[f32]| x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len() as f64;
            assert!(mean(&saw).abs() < 2e-3, "saw {hz}: {}", mean(&saw));
            assert!(mean(&tri).abs() < 2e-3, "tri {hz}: {}", mean(&tri));
            assert!(
                (mean(&pul) - 0.6).abs() < 2e-3,
                "pulse {hz}: {}",
                mean(&pul)
            );
        }
        // The first sample is the naive waveform at the start phase: -1 for every wave started at phase 0.
        for first in [
            tone::<SAW>(440.0, 0.5, 0.01)[0],
            tone::<TRIANGLE>(440.0, 0.5, 0.01)[0],
            tone::<PULSE>(440.0, 0.5, 0.01)[0],
        ] {
            assert_eq!(first, -1.0);
        }
    }

    /// Frequency by falling zero crossings of the saw, which is accurate to well under the 1e-4 of the acceptance
    /// criteria even for f32 phase accumulation (10 s at 55, 440 and 3,520 Hz).
    #[test]
    fn the_pitch_is_exact_over_ten_seconds() {
        fn crossings(x: &[f32], rising: bool) -> (usize, f64, f64) {
            // Interpolated times of crossings of 0; returns (count, first, last) in samples.
            let (mut n, mut first, mut last) = (0, 0.0, 0.0);
            for i in 1..x.len() {
                let hit = if rising {
                    x[i - 1] < 0.0 && x[i] >= 0.0
                } else {
                    x[i - 1] >= 0.0 && x[i] < 0.0
                };
                if hit {
                    let t = i as f64 - 1.0 + f64::from(x[i - 1]) / f64::from(x[i - 1] - x[i]);
                    if n == 0 {
                        first = t;
                    }
                    last = t;
                    n += 1;
                }
            }
            (n, first, last)
        }
        for hz in [55.0, 440.0, 3_520.0] {
            // Rising crossings of the triangle and the pulse's edges and the saw's mid-cycle are all one per cycle.
            let tri = tone::<TRIANGLE>(hz, 0.5, 10.0);
            let (n, a, b) = crossings(&tri, true);
            let measured = (n - 1) as f64 * SR / (b - a);
            assert!(
                (measured / hz - 1.0).abs() < 1e-4,
                "triangle {hz}: {measured}"
            );
            let saw = tone::<SAW>(hz, 0.5, 10.0);
            let (n, a, b) = crossings(&saw, true);
            let measured = (n - 1) as f64 * SR / (b - a);
            assert!((measured / hz - 1.0).abs() < 1e-4, "saw {hz}: {measured}");
        }
    }

    #[test]
    fn aliasing_stays_below_the_researched_ceilings() {
        // Research report 02, 1 s at 48 kHz, 32,768-point Blackman-Harris, 20 Hz-20 kHz, alias over harmonic power.
        // Measured here (dB): saw 110 Hz -46.8, 440 Hz -40.6, 3,520 Hz -28.8; triangle 440 Hz -83.9; the numbers
        // below leave 1.5 dB.
        let r = |x: &[f32], f0: f64| alias_ratio_db(x, f0, SR, 4096);
        let saw440 = r(&tone::<SAW>(440.0, 0.5, 1.0), 440.0);
        let tri440 = r(&tone::<TRIANGLE>(440.0, 0.5, 1.0), 440.0);
        let sq440 = r(&tone::<PULSE>(440.0, 0.5, 1.0), 440.0);
        let p20 = r(&tone::<PULSE>(440.0, 0.2, 1.0), 440.0);
        let saw3520 = r(&tone::<SAW>(3_520.0, 0.5, 1.0), 3_520.0);
        eprintln!(
            "alias dB: saw440 {saw440:.1} tri440 {tri440:.1} sq440 {sq440:.1} p20 {p20:.1} saw3520 {saw3520:.1}"
        );
        assert!(saw440 <= -38.0, "saw 440 Hz: {saw440}");
        assert!(tri440 <= -80.0, "triangle 440 Hz: {tri440}");
        assert!(sq440 <= -40.0, "square 440 Hz: {sq440}");
        assert!(p20 <= -38.0, "pulse 20 % 440 Hz: {p20}");
        assert!(saw3520 <= -27.0, "saw 3,520 Hz: {saw3520}");
        // The naive saw is the contrast: about -21 dB at 440 Hz.
        let naive_saw: Vec<f32> = (0..48_000)
            .map(|i| 2.0 * ((f64::from(i) * 440.0 / SR).fract() as f32) - 1.0)
            .collect();
        let n = r(&naive_saw, 440.0);
        assert!(n > -25.0 && n - saw440 > 15.0, "naive {n} vs blep {saw440}");
    }

    use crate::wavetable::{TableKind, Wavetable, alias_limit, level_for};

    /// `seconds` of the table's `frame_pos` at `hz`, read at the mip level the engine would pick.
    fn table_tone(kind: TableKind, frame_pos: f32, hz: f64, seconds: f64) -> Vec<f32> {
        let t = Wavetable::shared(kind);
        let dt = clamp_dt((hz / SR) as f32, MAX_DT_PHASED);
        let level = level_for(dt, alias_limit(SR as f32));
        let mut o = Phased::new(0.0, t.sample(level, frame_pos, 0.0));
        let mut x = vec![0.0f32; (seconds * SR) as usize / N * N];
        for c in x.as_chunks_mut::<N>().0 {
            o.render::<N>([dt, dt], None, None, c, |ph, _| {
                t.sample(level, frame_pos, ph)
            });
        }
        x
    }

    #[test]
    fn wavetables_alias_below_the_ceiling_at_every_pitch() {
        // Acceptance: wavetable <= -85 dB at 440 and 3,520 Hz (research report 02 measured <= -93, the floor of
        // the 4-term Blackman-Harris measurement). Every table, three frames each.
        let mut all = Vec::new();
        for kind in TableKind::ALL {
            for frame_pos in [0.0f32, 3.0, 6.0, 8.0, 12.0] {
                for hz in [55.0, 110.0, 220.0, 440.0, 1_046.5, 3_520.0, 7_040.0] {
                    let r = alias_ratio_db(&table_tone(kind, frame_pos, hz, 1.0), hz, SR, 4096);
                    all.push((r, kind, frame_pos, hz));
                }
            }
        }
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (r, kind, frame_pos, hz) in all.iter().take(12) {
            eprintln!("wavetable alias {r:.1} dB: {kind:?} frame {frame_pos} at {hz} Hz");
        }
        assert!(all[0].0 <= -85.0, "worst: {:?}", all[0]);
    }

    #[test]
    fn the_wavetable_pitch_is_exact() {
        // A sine frame (table basic, frame 0): rising crossings of zero, 10 s.
        for hz in [55.0, 440.0, 3_520.0] {
            let x = table_tone(TableKind::Basic, 0.0, hz, 10.0);
            let (mut n, mut first, mut last) = (0usize, 0.0f64, 0.0f64);
            for i in 1..x.len() {
                if x[i - 1] < 0.0 && x[i] >= 0.0 {
                    let t = i as f64 - 1.0 + f64::from(x[i - 1]) / f64::from(x[i - 1] - x[i]);
                    if n == 0 {
                        first = t;
                    }
                    last = t;
                    n += 1;
                }
            }
            let measured = (n - 1) as f64 * SR / (last - first);
            assert!((measured / hz - 1.0).abs() < 1e-4, "{hz}: {measured}");
        }
    }

    /// Upstream's reset time for a master that accumulates `master_phase_ += f` from `phase0` (Plaits,
    /// `Render<true, …>`): `NO_RESET` where it does not wrap.
    fn upstream_reset_times(phase0: f32, f: f32, frames: usize) -> Vec<f32> {
        let mut phase = phase0;
        (0..frames)
            .map(|_| {
                phase += f;
                if phase >= 1.0 {
                    phase -= 1.0;
                    phase / f
                } else {
                    NO_RESET
                }
            })
            .collect()
    }

    #[test]
    fn the_master_writes_exactly_the_reset_times_upstream_computes() {
        // The per-frame reset-time format carries Plaits' own `reset_time`, bit for bit, for every wave as master.
        for (phase0, hz) in [
            (0.0f32, 440.0f64),
            (0.37, 55.0),
            (0.9, 3_520.0),
            (0.5, 11_900.0),
        ] {
            let f = (hz / SR) as f32;
            let frames = 48_000 / N * N;
            let want = upstream_reset_times(phase0, f, frames);
            let run = |mut one: Step| {
                let mut got = vec![NO_RESET; frames];
                let mut out = [0.0; N];
                for c in got.as_chunks_mut::<N>().0 {
                    one(&mut out, c);
                }
                got
            };
            let mut s = Shape::new(SAW, phase0, 0.5);
            let got = run(Box::new(move |out, sync| {
                s.render::<SAW, N>([f, f], [0.5, 0.5], None, Some(sync), out);
            }));
            assert_eq!(bits(&got), bits(&want), "saw {hz} Hz");
            let mut s = Shape::new(TRIANGLE, phase0, 0.5);
            let got = run(Box::new(move |out, sync| {
                s.render::<TRIANGLE, N>([f, f], [0.5, 0.5], None, Some(sync), out);
            }));
            assert_eq!(bits(&got), bits(&want), "triangle {hz} Hz");
            let pw = clamp_pw(0.3, f);
            let mut s = Shape::new(PULSE, phase0, pw);
            let got = run(Box::new(move |out, sync| {
                s.render::<PULSE, N>([f, f], [pw, pw], None, Some(sync), out);
            }));
            assert_eq!(bits(&got), bits(&want), "pulse {hz} Hz");
            let t = Wavetable::shared(TableKind::Basic);
            let mut s = Phased::new(phase0, 0.0);
            let got = run(Box::new(move |out, sync| {
                s.render::<N>([f, f], None, Some(sync), out, |ph, _| t.sample(5, 0.0, ph));
            }));
            assert_eq!(bits(&got), bits(&want), "phased {hz} Hz");
            assert!(
                want.iter()
                    .filter(|&&r| r >= 0.0)
                    .all(|r| (0.0..1.0).contains(r))
            );
        }
    }

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// Run a slave of wave `WAVE` at `slave_hz` against a saw master at `master_hz`, as the engine wires them.
    fn synced<const WAVE: u8>(master_hz: f64, slave_hz: f64, pw: f32, seconds: f64) -> Vec<f32> {
        let (fm, fs) = ((master_hz / SR) as f32, (slave_hz / SR) as f32);
        let pw = clamp_pw(pw, fs);
        let mut master = Shape::new(SAW, 0.0, 0.5);
        let mut slave = Shape::new(WAVE, 0.0, pw);
        let mut x = vec![0.0f32; (seconds * SR) as usize / N * N];
        let mut m_out = [0.0f32; N];
        for c in x.as_chunks_mut::<N>().0 {
            let mut bus = [NO_RESET; N];
            master.render::<SAW, N>([fm, fm], [0.5, 0.5], None, Some(&mut bus), &mut m_out);
            slave.render::<WAVE, N>([fs, fs], [pw, pw], Some(&bus), None, c);
        }
        x
    }

    #[test]
    fn a_slave_restarts_with_every_master_cycle() {
        // A master period of exactly 128 samples (dt = 1/128): every wrap lands on a frame boundary (reset time 0),
        // so once the first reset has happened every master period of a slave is identical, bit for bit.
        let master_hz = 375.0; // 1/128 cycles per sample at 48 kHz
        for (name, x) in [
            ("saw", synced::<SAW>(master_hz, master_hz * 2.37, 0.5, 0.5)),
            (
                "triangle",
                synced::<TRIANGLE>(master_hz, master_hz * 2.37, 0.5, 0.5),
            ),
            (
                "pulse",
                synced::<PULSE>(master_hz, master_hz * 2.37, 0.3, 0.5),
            ),
        ] {
            let period = 128;
            for start in [period * 3, period * 50, period * 150] {
                assert_eq!(
                    bits(&x[start..start + period]),
                    bits(&x[start + period..start + 2 * period]),
                    "{name} at {start}"
                );
            }
            // And it is not a plain free-running oscillator: the slave's own cycle (period 128/2.37) is cut off.
            let free = match name {
                "saw" => tone::<SAW>(master_hz * 2.37, 0.5, 0.5),
                "triangle" => tone::<TRIANGLE>(master_hz * 2.37, 0.5, 0.5),
                _ => tone::<PULSE>(master_hz * 2.37, 0.3, 0.5),
            };
            assert_ne!(
                bits(&x[period * 50..period * 51]),
                bits(&free[period * 50..period * 51]),
                "{name}"
            );
        }
    }

    #[test]
    fn hard_sync_aliases_like_the_researched_baseline() {
        // Research report 02: Plaits saw, hard sync x2.37: -43.6 (110 Hz), -36.8 (440 Hz), -28.6 (3,520 Hz) dB,
        // measured against the master frequency's harmonics. The measure here uses the master's f0 as well.
        let a440 = alias_ratio_db(
            &synced::<SAW>(440.0, 440.0 * 2.37, 0.5, 1.0),
            440.0,
            SR,
            4096,
        );
        let a110 = alias_ratio_db(
            &synced::<SAW>(110.0, 110.0 * 2.37, 0.5, 1.0),
            110.0,
            SR,
            4096,
        );
        eprintln!("sync alias dB: 110 Hz {a110:.1}, 440 Hz {a440:.1}");
        assert!(a440 <= -35.0, "{a440}");
        assert!(a110 <= -41.0, "{a110}");
    }

    #[test]
    fn a_synced_sine_and_table_band_limit_their_reset_jump() {
        // Without the BLEP at the reset a hard-synced sine would alias at about -25 dB; with it, far lower.
        let t = Wavetable::shared(TableKind::Basic);
        let mut master = Shape::new(SAW, 0.0, 0.5);
        let mut slave = Phased::new(0.0, 0.0);
        let (fm, fs) = ((440.0 / SR) as f32, (440.0 * 2.37 / SR) as f32);
        let mut x = vec![0.0f32; 48_000 / N * N];
        let mut m_out = [0.0f32; N];
        for c in x.as_chunks_mut::<N>().0 {
            let mut bus = [NO_RESET; N];
            master.render::<SAW, N>([fm, fm], [0.5, 0.5], None, Some(&mut bus), &mut m_out);
            let level = level_for(fs, alias_limit(SR as f32));
            slave.render::<N>([fs, fs], Some(&bus), None, c, |ph, _| {
                t.sample(level, 0.0, ph)
            });
        }
        let blep = alias_ratio_db(&x, 440.0, SR, 4096);
        // The same without the correction: reset the phase hard.
        let mut phase = 0.0f32;
        let mut hard = vec![0.0f32; 48_000];
        let mut mphase = 0.0f32;
        for h in &mut hard {
            *h = t.sample(8, 0.0, phase);
            mphase += fm;
            phase += fs;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            if mphase >= 1.0 {
                mphase -= 1.0;
                phase = 0.0;
            }
        }
        let naive = alias_ratio_db(&hard, 440.0, SR, 4096);
        eprintln!("synced sine: blep {blep:.1} dB, hard reset {naive:.1} dB");
        assert!(blep < naive - 10.0, "{blep} vs {naive}");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

        /// Whatever the increments, widths and sync times, every wave stays finite and bounded, the phase stays in
        /// [0, 1), and nothing hangs (the transition loop is bounded).
        #[test]
        fn the_cores_are_bounded_for_any_parameters(
            dt0 in 0.0f32..0.6, dt1 in 0.0f32..0.6, pw0 in 0.0f32..1.0, pw1 in 0.0f32..1.0,
            start in 0.0f32..1.0, syncs in proptest::collection::vec(-1.0f32..1.5, 32),
        ) {
            let (d0, d1) = (clamp_dt(dt0, MAX_DT_ANALOG), clamp_dt(dt1, MAX_DT_ANALOG));
            let (p0, p1) = (clamp_pw(pw0, d0), clamp_pw(pw1, d1));
            let sync: [f32; N] = syncs.try_into().unwrap();
            for use_sync in [false, true] {
                let sin = use_sync.then_some(&sync);
                macro_rules! check {
                    ($w:expr) => {{
                        let mut s = Shape::new($w, start, p0);
                        let mut out = [0.0f32; N];
                        for _ in 0..4 {
                            s.render::<{ $w }, N>([d0, d1], [p0, p1], sin, None, &mut out);
                            proptest::prop_assert!(out.iter().all(|v| v.is_finite() && v.abs() < 8.0), "{out:?}");
                            proptest::prop_assert!((0.0..1.0).contains(&s.phase()));
                        }
                    }};
                }
                check!(SAW);
                check!(TRIANGLE);
                check!(PULSE);
            }
            let (e0, e1) = (clamp_dt(dt0, MAX_DT_PHASED), clamp_dt(dt1, MAX_DT_PHASED));
            let mut ph = Phased::new(start, 0.0);
            let mut out = [0.0f32; N];
            for _ in 0..4 {
                ph.render::<N>([e0, e1], Some(&sync), None, &mut out, |p, _| sine(p));
                proptest::prop_assert!(out.iter().all(|v| v.is_finite() && v.abs() < 8.0));
                proptest::prop_assert!((0.0..1.0).contains(&ph.phase()));
            }
        }
    }
}
