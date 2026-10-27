//! The formant (vowel) filter behind the `formant` module (S2a lane FLT). Notes: `docs/dsp/formant.md`.
//!
//! Three resonant band-passes in parallel, at the first two formants of a vowel (`f1`, `f2`, in Hz, set by the user)
//! and a fixed third at 2.7 kHz, summed with alternating signs: the parallel formant synthesiser of Klatt (*Software
//! for a cascade/parallel formant synthesizer*, JASA 67(3), 1980), whose alternating signs keep the valleys between
//! formants shallow, as the all-pole vocal tract's are, where a plain sum would put a zero between two adjacent
//! formants. Written from the equations; no code is ported.
//!
//! ```text
//! y = (1 · bp(f1) − 1 · bp(f2) + ½ · bp(f3)) · M(Q)         bp(f) = the unity-peak 2-pole band-pass at f
//! f_i  = f_i · 2^(shift/12)        (f3 = 2.7 kHz)           Q = 2 · 25^(resonance/100)       M = 1.5 · √Q
//! ```
//!
//! - **Each band-pass is the TPT state-variable filter** of [`crate::svf`] (Simper/Zavalishin: two trapezoidal
//!   integrators in a zero-delay loop), its band output `k·v1` having unity gain at the centre, so a section's centre
//!   is exactly where it is set (pre-warped by `tan`) and its −3 dB bandwidth is `f/Q` (a constant-Q section: every
//!   formant has the same relative width, so `shift` moves the whole response along the log-frequency axis and keeps
//!   the vowel).
//! - **The level.** A unity-peak band-pass passes `1/Q` of a white noise's power, so a bank of three would lose 13–20
//!   dB on a saw and more the narrower it is. The make-up `M = 1.5·√Q` cancels the narrowing (noise power through a
//!   section is `∝ 1/Q`, amplitude `∝ 1/√Q`) and puts a harmonic source near its input's loudness at every resonance;
//!   the peaks are then `M` (+6.5 dB at Q = 2, +13.5 dB at the default 10, +20.5 dB at Q = 50) above the input
//!   (`docs/dsp/formant.md`, "Level").
//! - **Zipper-free.** The caller computes a [`Tuning`] for the end of each 32-frame segment and [`process_block`]
//!   ramps every section's `a1, a2, a3` and its output weight linearly per sample from the previous segment's values
//!   (the svf's scheme, which report 02 found stable), so a sweep of `f2` never steps. `mix` ramps the same way.
//!
//! Determinism: the transcendentals (`tan`, `exp2`) run in [`Tuning::new`], once per voice and changed segment,
//! through `util::math`; the per-sample code is `+ − ×`. No `mul_add`. The states are flushed once per block.

use wide::f32x4;

use crate::svf::Coeffs;
use crate::util::flush::flush;
use crate::util::math;

/// The lowest Q, at `resonance` 0 %: sections as wide as half their centre frequency (a soft, breathy vowel).
pub const Q_MIN: f32 = 2.0;
/// The highest Q, at `resonance` 100 %: sections a fiftieth of their centre wide (a whistling vowel).
pub const Q_MAX: f32 = 50.0;
/// `log2(Q_MAX / Q_MIN)`: the resonance map is `Q = Q_MIN · 2^(LOG2_Q_RANGE · resonance)`.
pub const LOG2_Q_RANGE: f64 = 4.643_856_189_774_724;
/// The third formant, before `shift`.
pub const F3_HZ: f32 = 2_700.0;
/// The signed peak gains of the three sections before the make-up: Klatt's alternating signs, the third 6 dB down.
pub const SECTION_GAINS: [f32; 3] = [1.0, -1.0, 0.5];
/// The make-up is `MAKEUP · √Q`.
pub const MAKEUP: f32 = 1.5;
/// The largest `shift` the core computes, in semitones (the catalogue's maximum is 24).
pub const MAX_SHIFT: f32 = 48.0;
/// The number of sections.
pub const SECTIONS: usize = 3;

/// The Q for a resonance of `percent` (0–100, clamped): `Q = 2 · 25^(percent/100)`, exponential in Q so equal knob
/// steps are equal steps in decibels of peak height. Landmarks: 0 % 2.0, 25 % 4.5, 50 % (the default) 10.0, 75 %
/// 22.4, 100 % 50.0.
#[must_use]
pub fn q_from_resonance(percent: f32) -> f32 {
    let r = f64::from(percent.clamp(0.0, 100.0)) * 0.01;
    (f64::from(Q_MIN) * math::exp2(LOG2_Q_RANGE * r)) as f32
}

/// The output make-up for a section Q: `1.5·√Q`.
#[must_use]
pub fn makeup(q: f32) -> f32 {
    MAKEUP * q.sqrt()
}

/// The three sections' coefficients for one setting of `f1`, `f2`, `shift` and `resonance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// The state-variable coefficients of the sections (`f1`, `f2`, the third), sharing one damping `k = 1/Q`.
    pub coeffs: [Coeffs; SECTIONS],
    /// Each section's weight on its `v1` output: its signed gain times the make-up times `k` (the band output is
    /// `k·v1`).
    pub out: [f32; SECTIONS],
}

impl Tuning {
    /// The tuning for first and second formants of `f1` and `f2` Hz, shifted by `shift` semitones (clamped to
    /// ±[`MAX_SHIFT`]; a NaN is 0) and a resonance of `resonance` percent. Centres are clamped to 20 Hz … 0.45·fs
    /// by [`Coeffs::new`]. One `exp2` for the shift, one for the Q, three `tan`.
    #[must_use]
    pub fn new(f1: f32, f2: f32, shift: f32, resonance: f32, sample_rate: f32) -> Self {
        let ratio = if shift == 0.0 || shift.is_nan() {
            1.0
        } else {
            math::exp2f(shift.clamp(-MAX_SHIFT, MAX_SHIFT) / 12.0)
        };
        let q = q_from_resonance(resonance);
        let k = 1.0 / q;
        let gain = makeup(q) * k;
        let centres = [f1 * ratio, f2 * ratio, F3_HZ * ratio];
        Self {
            coeffs: core::array::from_fn(|i| Coeffs::new(centres[i], k, sample_rate)),
            out: core::array::from_fn(|i| SECTION_GAINS[i] * gain),
        }
    }
}

/// The magnitude of the filter's exact digital response at `freq_hz` for a static `tuning`, by complex arithmetic in
/// `f64` from the coefficients themselves: with `Ω = tan(ω/2)/g`, `g = a2/a1`, a section is `out · jΩ / (1 − Ω² +
/// j·k·Ω)` (the bilinear transform of the analogue prototype), and the filter is their sum. This is the closed form
/// the tests hold the filter to and the numbers `docs/dsp/formant.md` quotes.
#[must_use]
pub fn response(tuning: &Tuning, freq_hz: f64, sample_rate: f64) -> f64 {
    let (re, im) = response_complex(tuning, freq_hz, sample_rate);
    (re * re + im * im).sqrt()
}

/// [`response`] as `(re, im)`.
#[must_use]
pub fn response_complex(tuning: &Tuning, freq_hz: f64, sample_rate: f64) -> (f64, f64) {
    let w = 2.0 * core::f64::consts::PI * freq_hz / sample_rate;
    let t = math::tan(0.5 * w);
    let (mut re, mut im) = (0.0, 0.0);
    for (c, &o) in tuning.coeffs.iter().zip(&tuning.out) {
        let g = f64::from(c.a2) / f64::from(c.a1);
        let k = f64::from(c.k);
        let om = t / g;
        // jΩ / (1 − Ω² + jkΩ) = jΩ·(1 − Ω² − jkΩ) / ((1 − Ω²)² + k²Ω²).
        let d = (1.0 - om * om) * (1.0 - om * om) + k * k * om * om;
        let (h_re, h_im) = (k * om * om / d, om * (1.0 - om * om) / d);
        re += f64::from(o) * h_re;
        im += f64::from(o) * h_im;
    }
    (re, im)
}

/// The integrator states of the three sections of one voice.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    ic1: [f32; SECTIONS],
    ic2: [f32; SECTIONS],
}

impl State {
    /// A silent filter.
    pub const ZERO: Self = Self {
        ic1: [0.0; SECTIONS],
        ic2: [0.0; SECTIONS],
    };

    /// Whether any integrator holds a subnormal number (test support).
    #[must_use]
    pub fn has_subnormal(&self) -> bool {
        self.ic1.iter().chain(&self.ic2).any(|v| v.is_subnormal())
    }

    /// The largest integrator magnitude (test support).
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.ic1
            .iter()
            .chain(&self.ic2)
            .fold(0.0, |m, v| m.max(v.abs()))
    }

    fn flush(&mut self) {
        for v in self.ic1.iter_mut().chain(self.ic2.iter_mut()) {
            *v = flush(*v);
        }
    }
}

/// What one segment needs: the tuning at its start (the previous segment's end) and at its end, and the mix at both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The tuning at the start of the block.
    pub from: Tuning,
    /// The tuning at the end of the block.
    pub to: Tuning,
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

/// Run the filter over `io` in place. Every section's `a1, a2, a3` and output weight ramp linearly from `seg.from` to
/// `seg.to` across the block (the last sample uses `to`, up to rounding), and the mix from `seg.mix[0]` to
/// `seg.mix[1]`. With nothing to ramp the ramp is skipped (adding exact zeros is bit-identical). At mix 0 at both
/// ends `io` is not written, so the input passes bit for bit while the filter keeps running to hold its state
/// current. The states are flushed at the end.
pub fn process_block<const N: usize>(state: &mut State, seg: &Segment, io: &mut [f32; N]) {
    let mix = [sane_mix(seg.mix[0]), sane_mix(seg.mix[1])];
    let ramp = seg.from != seg.to;
    if mix == [0.0, 0.0] {
        run::<OFF, N>(state, seg, ramp, mix, io);
    } else if mix == [1.0, 1.0] {
        run::<FULL, N>(state, seg, ramp, mix, io);
    } else {
        run::<BLEND, N>(state, seg, ramp, mix, io);
    }
    state.flush();
}

#[inline(always)]
fn run<const WET: u8, const N: usize>(
    state: &mut State,
    seg: &Segment,
    ramp: bool,
    mix: [f32; 2],
    io: &mut [f32; N],
) {
    if ramp {
        run_with::<true, WET, N>(state, seg, mix, io);
    } else {
        run_with::<false, WET, N>(state, seg, mix, io);
    }
}

#[inline(always)]
fn run_with<const RAMP: bool, const WET: u8, const N: usize>(
    state: &mut State,
    seg: &Segment,
    mix: [f32; 2],
    io: &mut [f32; N],
) {
    let n = N as f32;
    let at = if RAMP { &seg.from } else { &seg.to };
    let mut a1: [f32; SECTIONS] = core::array::from_fn(|i| at.coeffs[i].a1);
    let mut a2: [f32; SECTIONS] = core::array::from_fn(|i| at.coeffs[i].a2);
    let mut a3: [f32; SECTIONS] = core::array::from_fn(|i| at.coeffs[i].a3);
    let mut o = at.out;
    let (d1, d2, d3, dout) = if RAMP {
        (
            core::array::from_fn(|i| (seg.to.coeffs[i].a1 - a1[i]) / n),
            core::array::from_fn(|i| (seg.to.coeffs[i].a2 - a2[i]) / n),
            core::array::from_fn(|i| (seg.to.coeffs[i].a3 - a3[i]) / n),
            core::array::from_fn(|i| (seg.to.out[i] - o[i]) / n),
        )
    } else {
        (
            [0.0; SECTIONS],
            [0.0; SECTIONS],
            [0.0; SECTIONS],
            [0.0; SECTIONS],
        )
    };
    let mut m = mix[0];
    let dm = (mix[1] - m) / n;
    let (mut ic1, mut ic2) = (state.ic1, state.ic2);
    for x in io.iter_mut() {
        let v0 = *x;
        if RAMP {
            for i in 0..SECTIONS {
                a1[i] += d1[i];
                a2[i] += d2[i];
                a3[i] += d3[i];
                o[i] += dout[i];
            }
        }
        let mut acc = 0.0f32;
        for i in 0..SECTIONS {
            let v3 = v0 - ic2[i];
            let v1 = a1[i] * ic1[i] + a2[i] * v3;
            let v2 = ic2[i] + a2[i] * ic1[i] + a3[i] * v3;
            ic1[i] = 2.0 * v1 - ic1[i];
            ic2[i] = 2.0 * v2 - ic2[i];
            acc += o[i] * v1;
        }
        match WET {
            OFF => {}
            FULL => *x = acc,
            _ => {
                m += dm;
                *x = (1.0 - m) * v0 + m * acc;
            }
        }
    }
    state.ic1 = ic1;
    state.ic2 = ic2;
}

// ---------------------------------------------------------------------------------------------------------------
// Several instances at once (lane PRF)
// ---------------------------------------------------------------------------------------------------------------

type V = f32x4;

/// What instances must share to run in lanes of one loop: whether the coefficients ramp and how the wet signal is
/// written (the choices `process_block` turns into const generics).
fn lane_class(seg: &Segment) -> (bool, u8) {
    let mix = [sane_mix(seg.mix[0]), sane_mix(seg.mix[1])];
    let wet = if mix == [0.0, 0.0] {
        OFF
    } else if mix == [1.0, 1.0] {
        FULL
    } else {
        BLEND
    };
    (seg.from != seg.to, wet)
}

/// [`process_block`] for `L` (at most 4) instances at once, one per lane of an `f32x4`: `states[l]` filters `ios[l]`
/// with `segs[l]`. **Each instance gets exactly the bits `process_block` gives it alone**: a lane performs the scalar
/// loop's `+ − ×` on its own values in the same order, and the instances share nothing but the loop (the stereo voice
/// path runs a voice's left and right instances, and several voices, side by side this way). Instances that differ in
/// whether they ramp or in how the wet signal is written run one after the other as plain `process_block` calls.
pub fn process_lanes<const N: usize, const L: usize>(
    states: [&mut State; L],
    segs: [&Segment; L],
    ios: [&mut [f32; N]; L],
) {
    const { assert!(L >= 1 && L <= 4) };
    let class = lane_class(segs[0]);
    if segs.iter().any(|s| lane_class(s) != class) {
        for ((state, seg), io) in states.into_iter().zip(segs).zip(ios) {
            process_block(state, seg, io);
        }
        return;
    }
    let (ramp, wet) = class;
    let mix: [[f32; 2]; L] =
        core::array::from_fn(|l| [sane_mix(segs[l].mix[0]), sane_mix(segs[l].mix[1])]);
    let (mut states, mut ios) = (states, ios);
    macro_rules! by_wet {
        ($ramp:literal) => {
            match wet {
                OFF => run_lanes::<$ramp, OFF, N, L>(&mut states, &segs, &mix, &mut ios),
                FULL => run_lanes::<$ramp, FULL, N, L>(&mut states, &segs, &mix, &mut ios),
                _ => run_lanes::<$ramp, BLEND, N, L>(&mut states, &segs, &mix, &mut ios),
            }
        };
    }
    if ramp {
        by_wet!(true);
    } else {
        by_wet!(false);
    }
    for state in states {
        state.flush();
    }
}

/// The lanes `0..L` from `f(lane)`, the rest zero.
#[inline(always)]
fn lanes<const L: usize>(f: impl Fn(usize) -> f32) -> V {
    V::new(core::array::from_fn(|l| if l < L { f(l) } else { 0.0 }))
}

/// [`run_with`] with one instance per lane.
#[inline(always)]
#[allow(
    clippy::needless_range_loop,
    reason = "the sample index is shared by the blocks of all the lanes"
)]
fn run_lanes<const RAMP: bool, const WET: u8, const N: usize, const L: usize>(
    states: &mut [&mut State; L],
    segs: &[&Segment; L],
    mix: &[[f32; 2]; L],
    ios: &mut [&mut [f32; N]; L],
) {
    let n = V::splat(N as f32);
    let at = |l: usize| if RAMP { &segs[l].from } else { &segs[l].to };
    let mut a1: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| at(l).coeffs[i].a1));
    let mut a2: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| at(l).coeffs[i].a2));
    let mut a3: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| at(l).coeffs[i].a3));
    let mut o: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| at(l).out[i]));
    let (d1, d2, d3, dout) = if RAMP {
        (
            core::array::from_fn(|i| (lanes::<L>(|l| segs[l].to.coeffs[i].a1) - a1[i]) / n),
            core::array::from_fn(|i| (lanes::<L>(|l| segs[l].to.coeffs[i].a2) - a2[i]) / n),
            core::array::from_fn(|i| (lanes::<L>(|l| segs[l].to.coeffs[i].a3) - a3[i]) / n),
            core::array::from_fn(|i| (lanes::<L>(|l| segs[l].to.out[i]) - o[i]) / n),
        )
    } else {
        (
            [V::ZERO; SECTIONS],
            [V::ZERO; SECTIONS],
            [V::ZERO; SECTIONS],
            [V::ZERO; SECTIONS],
        )
    };
    let mut m = lanes::<L>(|l| mix[l][0]);
    let dm = (lanes::<L>(|l| mix[l][1]) - m) / n;
    let mut ic1: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| states[l].ic1[i]));
    let mut ic2: [V; SECTIONS] = core::array::from_fn(|i| lanes::<L>(|l| states[l].ic2[i]));
    let two = V::splat(2.0);
    for i in 0..N {
        let v0 = lanes::<L>(|l| ios[l][i]);
        if RAMP {
            for j in 0..SECTIONS {
                a1[j] += d1[j];
                a2[j] += d2[j];
                a3[j] += d3[j];
                o[j] += dout[j];
            }
        }
        let mut acc = V::ZERO;
        for j in 0..SECTIONS {
            let v3 = v0 - ic2[j];
            let v1 = a1[j] * ic1[j] + a2[j] * v3;
            let v2 = ic2[j] + a2[j] * ic1[j] + a3[j] * v3;
            ic1[j] = two * v1 - ic1[j];
            ic2[j] = two * v2 - ic2[j];
            acc += o[j] * v1;
        }
        let out = match WET {
            OFF => None,
            FULL => Some(acc),
            _ => {
                m += dm;
                Some((V::ONE - m) * v0 + m * acc)
            }
        };
        if let Some(out) = out {
            let a = out.to_array();
            for l in 0..L {
                ios[l][i] = a[l];
            }
        }
    }
    for (l, state) in states.iter_mut().enumerate() {
        state.ic1 = core::array::from_fn(|j| ic1[j].to_array()[l]);
        state.ic2 = core::array::from_fn(|j| ic2[j].to_array()[l]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fft;
    use crate::util::rng::Rng;

    const SR: f32 = 48_000.0;

    /// Report 04's vowel table (Catford 2001, male speech): `(name, F1, F2)` in Hz.
    const VOWELS: [(&str, f32, f32); 6] = [
        ("i", 240.0, 2_400.0),
        ("e", 390.0, 2_300.0),
        ("a", 850.0, 1_610.0),
        ("ɑ", 750.0, 940.0),
        ("o", 360.0, 640.0),
        ("u", 250.0, 595.0),
    ];

    fn static_segment(f1: f32, f2: f32, shift: f32, resonance: f32, mix: f32) -> Segment {
        let t = Tuning::new(f1, f2, shift, resonance, SR);
        Segment {
            from: t,
            to: t,
            mix: [mix, mix],
        }
    }

    fn db(x: f64) -> f64 {
        20.0 * math::log10(x.max(1e-30))
    }

    /// The impulse response of a static filter, `len` samples.
    fn impulse_response(seg: &Segment, len: usize) -> Vec<f32> {
        let mut st = State::ZERO;
        let mut out = Vec::with_capacity(len);
        let mut first = true;
        while out.len() < len {
            let mut block = [0.0f32; 32];
            if first {
                block[0] = 1.0;
                first = false;
            }
            process_block(&mut st, seg, &mut block);
            out.extend_from_slice(&block);
        }
        out
    }

    /// The local maxima of the closed-form response on a fine log grid (20 Hz … 20 kHz): `(Hz, magnitude)`.
    fn peaks(t: &Tuning) -> Vec<(f64, f64)> {
        let n = 24_000;
        let lo = math::log(20.0);
        let step = (math::log(20_000.0) - lo) / f64::from(n);
        let at = |i: i32| {
            let f = math::exp(lo + step * f64::from(i));
            (f, response(t, f, f64::from(SR)))
        };
        let mut out = Vec::new();
        let (mut prev, mut cur) = (at(0), at(1));
        for i in 2..=n {
            let next = at(i);
            if cur.1 > prev.1 && cur.1 >= next.1 {
                out.push(cur);
            }
            prev = cur;
            cur = next;
        }
        out
    }

    /// The error in percent of the local maximum nearest `target` (within a factor 1.5 either way), or `None` when
    /// the response has no maximum there (two formants merged into one).
    fn peak_error(pks: &[(f64, f64)], target: f32) -> Option<f64> {
        let t = f64::from(target);
        pks.iter()
            .filter(|p| p.0 > t / 1.5 && p.0 < t * 1.5)
            .min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))
            .map(|p| (p.0 / t - 1.0) * 100.0)
    }

    #[test]
    fn the_resonance_map_landmarks() {
        assert_eq!(q_from_resonance(0.0), 2.0);
        assert!((q_from_resonance(100.0) - 50.0).abs() < 1e-4);
        for (pct, q) in [(25.0f32, 4.472), (50.0, 10.0), (75.0, 22.361)] {
            assert!(
                (q_from_resonance(pct) - q).abs() < 0.001,
                "{pct}: {}",
                q_from_resonance(pct)
            );
        }
        let mut last = 0.0;
        for i in 0..=100 {
            let q = q_from_resonance(i as f32);
            assert!(q > last);
            last = q;
        }
        assert_eq!(q_from_resonance(-5.0), q_from_resonance(0.0));
        assert_eq!(q_from_resonance(500.0), q_from_resonance(100.0));
        // The make-up is 1.5·√Q: +6.5 dB at Q 2, +13.5 dB at 10, +20.5 dB at 50.
        assert!((db(f64::from(makeup(2.0))) - 6.53).abs() < 0.01);
        assert!((db(f64::from(makeup(10.0))) - 13.52).abs() < 0.01);
        assert!((db(f64::from(makeup(50.0))) - 20.5).abs() < 0.05);
    }

    #[test]
    fn the_measured_response_matches_the_closed_form() {
        // The impulse response's spectrum against `response`, at every bin from 60 Hz to 20 kHz where the response
        // is above −50 dB of its own peak: within 0.05 dB, for the six vowels at three resonances.
        for &(name, f1, f2) in &VOWELS {
            for res in [0.0f32, 50.0, 100.0] {
                let seg = static_segment(f1, f2, 0.0, res, 1.0);
                let ir = impulse_response(&seg, 262_144);
                let mut re: Vec<f64> = ir.iter().map(|&v| f64::from(v)).collect();
                let mut im = vec![0.0; ir.len()];
                fft(&mut re, &mut im);
                let bin = f64::from(SR) / ir.len() as f64;
                let top = (0..=ir.len() / 2)
                    .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
                    .fold(0.0, f64::max);
                let (mut worst, mut checked) = (0.0f64, 0usize);
                for i in (60.0 / bin) as usize..=(20_000.0 / bin) as usize {
                    let m = (re[i] * re[i] + im[i] * im[i]).sqrt();
                    if m < top * 0.003 {
                        continue;
                    }
                    let want = response(&seg.to, i as f64 * bin, f64::from(SR));
                    worst = worst.max((db(m) - db(want)).abs());
                    checked += 1;
                }
                eprintln!("{name} res {res}: {checked} bins, worst {worst:.4} dB");
                assert!(checked > 1_000);
                assert!(worst < 0.05, "{name} res {res}: {worst} dB");
            }
        }
    }

    #[test]
    fn the_peaks_sit_where_f1_and_f2_say_for_report_04s_vowels() {
        // Acceptance: peaks within 3 % of the vowel table for at least five vowels, set through f1 and f2. At the
        // default resonance (50 %, Q 10) and above all six are within 1.5 %; at 25 % (Q 4.5) five are within 3 % and
        // ɑ (F1 and F2 a quarter apart) merges toward its middle.
        // (Resonance 0 % is printed for the docs and not asserted: at Q 2 the sections are half their centre wide
        // and the peaks are humps pulled toward each other.)
        for res in [0.0f32, 25.0, 50.0, 75.0, 100.0] {
            let mut within = 0;
            for &(name, f1, f2) in &VOWELS {
                let t = Tuning::new(f1, f2, 0.0, res, SR);
                let pks = peaks(&t);
                let e1 = peak_error(&pks, f1);
                let e2 = peak_error(&pks, f2);
                eprintln!("res {res:>5}: {name} {f1}/{f2}: F1 {e1:+.2?} %, F2 {e2:+.2?} %");
                let ok = e1.is_some_and(|e| e.abs() < 3.0) && e2.is_some_and(|e| e.abs() < 3.0);
                if res >= 50.0 {
                    let tight =
                        e1.is_some_and(|e| e.abs() < 1.5) && e2.is_some_and(|e| e.abs() < 1.5);
                    assert!(tight, "{name} at {res} %: {e1:?} {e2:?}");
                }
                within += usize::from(ok);
            }
            assert!(
                res < 25.0 || within >= 5,
                "resonance {res}: only {within} vowels within 3 %"
            );
        }
    }

    #[test]
    fn shift_moves_the_whole_response_along_the_log_frequency_axis() {
        // The vowel is kept: with sections of one Q, every peak is at the unshifted frequency times 2^(shift/12).
        for shift in [-24.0f32, -12.0, -5.0, 7.0, 12.0] {
            let ratio = f64::from(math::exp2f(shift / 12.0));
            for &(name, f1, f2) in &[VOWELS[0], VOWELS[2], VOWELS[4]] {
                let t = Tuning::new(f1, f2, shift, 75.0, SR);
                let pks = peaks(&t);
                for want in [f1, f2] {
                    let target = f64::from(want) * ratio;
                    let got = pks
                        .iter()
                        .min_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()))
                        .expect("a peak");
                    assert!(
                        (got.0 / target - 1.0).abs() < 0.015,
                        "{name} shift {shift}: wanted {target:.1} Hz, peak {:.1} Hz",
                        got.0
                    );
                }
            }
        }
    }

    #[test]
    fn a_sections_bandwidth_is_its_centre_over_q() {
        // One section alone (the others weighted 0): the −3 dB width is f/Q (the bilinear pre-warp moves it by
        // under 2 % at 1 kHz), the gain at the centre is the make-up exactly, and it is a band-pass (zero at DC).
        for res in [0.0f32, 25.0, 50.0, 75.0, 100.0] {
            let q = q_from_resonance(res);
            let mut t = Tuning::new(1_000.0, 2_000.0, 0.0, res, SR);
            t.out[1] = 0.0;
            t.out[2] = 0.0;
            let centre = response(&t, 1_000.0, f64::from(SR));
            let want = f64::from(makeup(q));
            assert!(
                (db(centre) - db(want)).abs() < 0.01,
                "Q {q}: {centre} vs {want}"
            );
            let half = centre / core::f64::consts::SQRT_2;
            let walk = |dir: f64| {
                let mut f = 1_000.0f64;
                while response(&t, f, f64::from(SR)) > half {
                    f *= 1.0 + dir * 0.000_5;
                }
                f
            };
            let width = walk(1.0) - walk(-1.0);
            let wanted = 1_000.0 / f64::from(q);
            eprintln!("Q {q:.2}: bandwidth {width:.1} Hz (f/Q = {wanted:.1})");
            assert!((width / wanted - 1.0).abs() < 0.02, "Q {q}: {width}");
            assert!(response(&t, 1.0, f64::from(SR)) < 0.01 * centre);
        }
    }

    #[test]
    fn the_peak_level_is_bounded_by_the_make_up() {
        // The response never exceeds the make-up: a unity band-pass's response lies on the circle with diameter
        // [0, 1], so two sections of opposite sign differ by at most 1, and the third at half weight adds nothing
        // measurable. Over a dense grid, for the six vowels at every resonance, max |H| / M is below 1.05 (1.000).
        let mut worst = 0.0f64;
        for &(name, f1, f2) in &VOWELS {
            for res in [0.0f32, 25.0, 50.0, 75.0, 100.0] {
                let t = Tuning::new(f1, f2, 0.0, res, SR);
                let m = f64::from(makeup(q_from_resonance(res)));
                let top = peaks(&t).iter().fold(0.0f64, |a, p| a.max(p.1));
                worst = worst.max(top / m);
                assert!(top / m < 1.05, "{name} at {res} %: {}", top / m);
            }
        }
        eprintln!("largest peak over the make-up: {worst:.3}");
    }

    /// The white-noise power gain of a static filter in dB (the mean of |H|² over 0 … fs/2) and the gain on a sawtooth
    /// of `f0` Hz (harmonic amplitudes 1/n up to 0.45·fs) in dB RMS.
    fn levels(t: &Tuning, f0: f64) -> (f64, f64) {
        let sr = f64::from(SR);
        let n = 20_000;
        let noise = (0..n)
            .map(|i| {
                let h = response(t, (f64::from(i) + 0.5) * sr / 2.0 / f64::from(n), sr);
                h * h
            })
            .sum::<f64>()
            / f64::from(n);
        let (mut num, mut den) = (0.0, 0.0);
        let mut k = 1.0;
        while k * f0 < 0.45 * sr {
            let h = response(t, k * f0, sr);
            num += h * h / (k * k);
            den += 1.0 / (k * k);
            k += 1.0;
        }
        (10.0 * math::log10(noise), 10.0 * math::log10(num / den))
    }

    #[test]
    fn the_make_up_keeps_a_harmonic_source_near_its_input_level() {
        // Documented in docs/dsp/formant.md ("Level"): noise power gain and saw gain, six vowels, five resonances.
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for res in [0.0f32, 25.0, 50.0, 75.0, 100.0] {
            for &(name, f1, f2) in &VOWELS {
                let t = Tuning::new(f1, f2, 0.0, res, SR);
                let (noise, saw110) = levels(&t, 110.0);
                let (_, saw220) = levels(&t, 220.0);
                eprintln!(
                    "res {res:>5} {name}: noise {noise:+.1} dB, saw 110 Hz {saw110:+.1} dB, saw 220 Hz {saw220:+.1} dB"
                );
                lo = lo.min(noise);
                hi = hi.max(noise);
                assert!(
                    (-12.0..-1.0).contains(&noise),
                    "{name} at {res} %: noise gain {noise}"
                );
            }
        }
        eprintln!("noise power gain over the grid: {lo:+.1} … {hi:+.1} dB");
    }

    #[test]
    fn the_filter_has_no_latency() {
        // The first output sample already depends on the first input sample: h[0] = Σ weight_i · a2_i (the band
        // output of a fresh section is a2·x), nonzero for every vowel. No delay line, no look-ahead.
        for &(name, f1, f2) in &VOWELS {
            let seg = static_segment(f1, f2, 0.0, 50.0, 1.0);
            let ir = impulse_response(&seg, 32);
            let want: f32 = (0..SECTIONS)
                .map(|i| seg.to.out[i] * seg.to.coeffs[i].a2)
                .sum();
            assert!(ir[0].abs() > 1e-4, "{name}: h[0] = {}", ir[0]);
            assert!((ir[0] - want).abs() <= 1e-6 * want.abs().max(1.0), "{name}");
        }
    }

    #[test]
    fn mix_zero_leaves_the_input_untouched_and_keeps_the_state_current() {
        let mut rng = Rng::new(3);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32());
        let off = static_segment(850.0, 1_610.0, 0.0, 50.0, 0.0);
        let on = static_segment(850.0, 1_610.0, 0.0, 50.0, 1.0);
        let (mut a, mut b) = (State::ZERO, State::ZERO);
        let mut x = input;
        process_block(&mut a, &off, &mut x);
        assert_eq!(x.map(f32::to_bits), input.map(f32::to_bits));
        // The state after a mix-0 block is the state after a mix-1 block.
        let mut y = input;
        process_block(&mut b, &on, &mut y);
        assert_eq!(a, b);
        assert_ne!(y, input);
    }

    #[test]
    fn a_mix_ramp_blends_from_dry_to_wet() {
        let t = Tuning::new(850.0, 1_610.0, 0.0, 50.0, SR);
        let seg = Segment {
            from: t,
            to: t,
            mix: [0.0, 1.0],
        };
        let mut rng = Rng::new(9);
        let input: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32());
        let mut wet = input;
        let mut st = State::ZERO;
        process_block(
            &mut st,
            &static_segment(850.0, 1_610.0, 0.0, 50.0, 1.0),
            &mut wet,
        );
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
    fn a_retune_across_one_block_has_no_click() {
        // A tuning change across one block lands on the new tuning's coefficients by the last frame: the next block
        // at the new tuning continues without a step larger than the signal's own.
        let a = Tuning::new(240.0, 2_400.0, 0.0, 75.0, SR);
        let b = Tuning::new(360.0, 640.0, 0.0, 75.0, SR);
        let w = 2.0 * core::f64::consts::PI * 300.0 / f64::from(SR);
        let mut st = State::ZERO;
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        let mut peak = 0.0f32;
        for blk in 0..400 {
            let seg = match blk {
                ..200 => Segment {
                    from: a,
                    to: a,
                    mix: [1.0, 1.0],
                },
                200 => Segment {
                    from: a,
                    to: b,
                    mix: [1.0, 1.0],
                },
                _ => Segment {
                    from: b,
                    to: b,
                    mix: [1.0, 1.0],
                },
            };
            let mut x: [f32; 32] =
                core::array::from_fn(|i| math::sin(w * (blk * 32 + i) as f64) as f32);
            process_block(&mut st, &seg, &mut x);
            for y in x {
                if blk > 100 {
                    worst = worst.max((y - last).abs());
                }
                peak = peak.max(y.abs());
                last = y;
            }
        }
        // A 300 Hz sine's own step is 0.039; through the filter (gain up to ~M = 4.7) and with the retune the steps
        // stay of that order, nowhere near the whole swing.
        eprintln!("retune across one block: worst step {worst:.3}, peak {peak:.3}");
        assert!(worst < 0.5 * peak, "step {worst}, peak {peak}");
    }

    #[test]
    fn random_jumps_of_f1_f2_shift_and_resonance_stay_bounded_and_the_tail_dies() {
        // The stress: a new random f1, f2, shift and resonance every segment for 20 s (an audio-rate 1.5 kHz
        // modulation of every parameter at its extremes), white noise in, at Q up to 50.
        for fixed_res in [None, Some(100.0f32)] {
            let mut st = State::ZERO;
            let mut rng = Rng::new(31);
            let mut prev = Tuning::new(850.0, 1_610.0, 0.0, 50.0, SR);
            let mut peak = 0.0f32;
            let mut state_peak = 0.0f32;
            for _ in 0..30_000 {
                let f1 = 100.0 * math::exp2f(rng.unit_f32() * math::log2f(10.0));
                let f2 = 400.0 * math::exp2f(rng.unit_f32() * math::log2f(10.0));
                let shift = (rng.unit_f32() - 0.5) * 48.0;
                let res = fixed_res.unwrap_or_else(|| rng.unit_f32() * 100.0);
                let t = Tuning::new(f1, f2, shift, res, SR);
                let seg = Segment {
                    from: prev,
                    to: t,
                    mix: [rng.unit_f32(), rng.unit_f32()],
                };
                let mut block: [f32; 32] = core::array::from_fn(|_| rng.bipolar_f32() * 0.5);
                process_block(&mut st, &seg, &mut block);
                for y in block {
                    assert!(y.is_finite());
                    peak = peak.max(y.abs());
                }
                state_peak = state_peak.max(st.peak());
                prev = t;
            }
            eprintln!(
                "random-jump stress ({fixed_res:?}): output peak {peak}, state peak {state_peak}"
            );
            assert!(peak < 4.0, "peak {peak}");
            // Park at a vowel and let it ring out: the state decays to exact zero with no subnormal on the way.
            let park = static_segment(850.0, 1_610.0, 0.0, 100.0, 1.0);
            for _ in 0..(48_000 * 30 / 32) {
                let mut block = [0.0f32; 32];
                process_block(&mut st, &park, &mut block);
                assert!(!st.has_subnormal());
            }
            let mut block = [0.0f32; 32];
            process_block(&mut st, &park, &mut block);
            assert!(block.iter().all(|&v| v == 0.0));
            assert_eq!(st, State::ZERO);
        }
    }

    #[test]
    fn the_coefficients_stay_stable_along_every_interpolation_path() {
        // The state update of one section is [[2·a1 − 1, −2·a2], [2·a2, 1 − 2·a3]]; its eigenvalues must stay inside
        // the unit circle along the straight line between any two coefficient sets (what a ramp passes through), at
        // the sharpest and the widest Q and the extreme centres (report 02's check, here at Q 50).
        let freqs = [
            20.0f32, 25.0, 100.0, 400.0, 1_000.0, 4_000.0, 10_000.0, 21_600.0,
        ];
        let mut worst = 0.0f64;
        for res in [0.0f32, 100.0] {
            let k = 1.0 / q_from_resonance(res);
            for &fa in &freqs {
                for &fb in &freqs {
                    let (ca, cb) = (Coeffs::new(fa, k, SR), Coeffs::new(fb, k, SR));
                    for s in 0..=100 {
                        let u = f64::from(s) / 100.0;
                        let mix = |a: f32, b: f32| f64::from(a) * (1.0 - u) + f64::from(b) * u;
                        let (a1, a2, a3) =
                            (mix(ca.a1, cb.a1), mix(ca.a2, cb.a2), mix(ca.a3, cb.a3));
                        let (m11, m12, m21, m22) =
                            (2.0 * a1 - 1.0, -2.0 * a2, 2.0 * a2, 1.0 - 2.0 * a3);
                        let tr = m11 + m22;
                        let det = m11 * m22 - m12 * m21;
                        let disc = tr * tr - 4.0 * det;
                        let radius = if disc < 0.0 {
                            det.sqrt()
                        } else {
                            let r = disc.sqrt();
                            f64::midpoint(tr, r).abs().max(f64::midpoint(tr, -r).abs())
                        };
                        worst = worst.max(radius);
                        assert!(
                            radius < 1.0,
                            "res {res}, {fa} -> {fb} Hz at {u}: radius {radius}"
                        );
                    }
                }
            }
        }
        eprintln!("largest eigenvalue magnitude along every path: {worst:.6}");
    }

    /// A random segment of the given class (`ramp`: the end tuning differs from the start's; `wet`: 0 off, 1 full,
    /// 2 blend).
    fn random_segment(rng: &mut Rng, ramp: bool, wet: u8) -> Segment {
        let tuning = |rng: &mut Rng| {
            Tuning::new(
                rng.range_f32(200.0, 900.0),
                rng.range_f32(600.0, 2_800.0),
                rng.range_f32(-12.0, 12.0),
                rng.range_f32(0.0, 100.0),
                SR,
            )
        };
        let from = tuning(rng);
        let to = if ramp { tuning(rng) } else { from };
        let mix = match wet {
            0 => [0.0, 0.0],
            1 => [1.0, 1.0],
            _ => [rng.range_f32(0.05, 0.95), rng.range_f32(0.05, 0.95)],
        };
        Segment { from, to, mix }
    }

    /// `process_lanes` on `L` instances against `process_block` on each, over several blocks: outputs and states,
    /// bit for bit.
    fn lanes_match_scalar<const L: usize>(rng: &mut Rng, classes: [(bool, u8); L]) {
        let mut scalar = [State::ZERO; L];
        let mut laned = [State::ZERO; L];
        for block in 0..12 {
            let segs: [Segment; L] =
                core::array::from_fn(|l| random_segment(rng, classes[l].0, classes[l].1));
            let amp = [0.3f32, 2.0, 9.0][block % 3];
            let mut want: [[f32; 32]; L] = core::array::from_fn(|l| {
                core::array::from_fn(|i| {
                    amp * math::sinf(0.05 * (1 + l) as f32 * (block * 32 + i) as f32)
                        + 0.3 * rng.bipolar_f32()
                })
            });
            let mut got = want;
            for l in 0..L {
                process_block(&mut scalar[l], &segs[l], &mut want[l]);
            }
            process_lanes(laned.each_mut(), segs.each_ref(), got.each_mut());
            for l in 0..L {
                let (w, g) = (want[l].map(f32::to_bits), got[l].map(f32::to_bits));
                assert_eq!(
                    w, g,
                    "output, lane {l} of {L}, block {block}, {:?}",
                    classes[l]
                );
                assert_eq!(
                    format!("{:?}", scalar[l]),
                    format!("{:?}", laned[l]),
                    "state, lane {l} of {L}, block {block}"
                );
            }
        }
    }

    #[test]
    fn instances_in_lanes_are_process_block_bit_for_bit() {
        let mut rng = Rng::new(0x1ADD_0003);
        for ramp in [false, true] {
            for wet in 0..3u8 {
                let class = (ramp, wet);
                lanes_match_scalar::<1>(&mut rng, [class]);
                lanes_match_scalar::<2>(&mut rng, [class; 2]);
                lanes_match_scalar::<3>(&mut rng, [class; 3]);
                lanes_match_scalar::<4>(&mut rng, [class; 4]);
            }
        }
    }

    #[test]
    fn instances_of_different_classes_run_one_after_the_other() {
        let mut rng = Rng::new(0x1ADD_0004);
        lanes_match_scalar::<4>(&mut rng, [(true, 2), (false, 2), (true, 1), (true, 0)]);
        lanes_match_scalar::<2>(&mut rng, [(false, 1), (false, 2)]);
    }
}
