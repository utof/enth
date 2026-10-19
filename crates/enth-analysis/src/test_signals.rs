//! Synthetic signals with known answers, for tests and accuracy sweeps. Not a synth.
//!
//! `enth-analysis` cannot depend on `enth-render` (same layer), so tests build their own audio here: sines,
//! polyBLEP saws, detuned pairs at a known beat rate, ADSR shapes, tremolo, noise, and the 10-parameter toy voice
//! that the P1a research measured its descriptor suites on (report 03). Everything is a pure function of its
//! arguments; transcendentals come from `libm`, so the signals are the same on every platform.
//!
//! The toy voice is ported from `spikes/spike-metric/src/render.rs` (in-repo, MIT) with the two extra inputs of the
//! research prototype (oscillator start phases, a master gain): two polyBLEP oscillators into a TPT state-variable
//! lowpass with a filter envelope and an amp ADSR, one voice, mono.

use core::f32::consts::PI as PI32;
use std::f64::consts::PI;

/// Sample rate used by the tests (Hz).
pub const SR: f64 = 48_000.0;

/// `amp · sin(2π f t)` for `seconds`.
#[must_use]
pub fn sine(sr: f64, freq: f64, amp: f64, seconds: f64) -> Vec<f32> {
    sine_phase(sr, freq, amp, seconds, 0.0)
}

/// `amp · sin(2π f t + phase)` for `seconds`.
#[must_use]
pub fn sine_phase(sr: f64, freq: f64, amp: f64, seconds: f64, phase: f64) -> Vec<f32> {
    (0..(sr * seconds) as usize)
        .map(|i| (amp * libm::sin(2.0 * PI * freq * i as f64 / sr + phase)) as f32)
        .collect()
}

/// Sample-wise sum of two signals (the shorter one is zero-padded).
#[must_use]
pub fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0))
        .collect()
}

/// Multiply every sample by `10^(db/20)`.
pub fn gain_db(x: &mut [f32], db: f64) {
    let g = libm::pow(10.0, db / 20.0) as f32;
    for s in x {
        *s *= g;
    }
}

/// Deterministic white noise, uniform in `-amp..amp` (xorshift64).
#[must_use]
pub fn white_noise(sr: f64, amp: f64, seconds: f64, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed | 1);
    (0..(sr * seconds) as usize)
        .map(|_| (amp * (2.0 * rng.next_f64() - 1.0)) as f32)
        .collect()
}

/// Shape `x` with a linear fade-in of `attack_s`.
pub fn linear_attack(x: &mut [f32], sr: f64, attack_s: f64) {
    for (i, s) in x.iter_mut().enumerate() {
        *s *= (i as f64 / sr / attack_s).min(1.0) as f32;
    }
}

/// Amplitude LFO: `x · (1 − depth · (1 − cos 2π r t) / 2)`. Depth 0.5 dips the level to half once per cycle.
pub fn tremolo(x: &mut [f32], sr: f64, rate_hz: f64, depth: f64) {
    for (i, s) in x.iter_mut().enumerate() {
        let t = i as f64 / sr;
        *s *= (1.0 - depth * 0.5 * (1.0 - libm::cos(2.0 * PI * rate_hz * t))) as f32;
    }
}

/// An ADSR with a linear attack and RC-style exponential decay and release, in the toy voice's convention: the
/// decay and release time constants are a quarter of the stated times (so the stage is about 98 % done by then).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    pub attack_s: f64,
    pub decay_s: f64,
    /// Linear level, 0 to 1.
    pub sustain: f64,
    pub release_s: f64,
}

impl Adsr {
    /// The level at time `t` for a note held until `hold_s`.
    #[must_use]
    pub fn level(&self, t: f64, hold_s: f64) -> f64 {
        let held = |t: f64| {
            if t < self.attack_s {
                t / self.attack_s
            } else {
                self.sustain
                    + (1.0 - self.sustain) * libm::exp(-(t - self.attack_s) / (0.25 * self.decay_s))
            }
        };
        if t < hold_s {
            held(t)
        } else {
            held(hold_s) * libm::exp(-(t - hold_s) / (0.25 * self.release_s))
        }
    }

    /// Multiply `x` by this envelope.
    pub fn apply(&self, x: &mut [f32], sr: f64, hold_s: f64) {
        for (i, s) in x.iter_mut().enumerate() {
            *s *= self.level(i as f64 / sr, hold_s) as f32;
        }
    }
}

/// Oscillator waveform of the toy voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Square,
    Triangle,
}

impl Wave {
    pub const ALL: [Wave; 3] = [Wave::Saw, Wave::Square, Wave::Triangle];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Wave::Saw => "saw",
            Wave::Square => "square",
            Wave::Triangle => "triangle",
        }
    }
}

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

fn osc(wave: Wave, ph: f32, dt: f32) -> f32 {
    match wave {
        Wave::Saw => 2.0 * ph - 1.0 - poly_blep(ph, dt),
        Wave::Square => {
            let mut v = if ph < 0.5 { 1.0 } else { -1.0 };
            v += poly_blep(ph, dt);
            v -= poly_blep((ph + 0.5).fract(), dt);
            v
        }
        Wave::Triangle => 4.0 * (ph - 0.5).abs() - 1.0,
    }
}

/// A polyBLEP oscillator at `freq` starting at `phase` (0..1), amplitude 1.
#[must_use]
pub fn oscillator(sr: f64, wave: Wave, freq: f64, seconds: f64, phase: f64) -> Vec<f32> {
    let dt = (freq / sr) as f32;
    let mut ph = phase as f32;
    (0..(sr * seconds) as usize)
        .map(|_| {
            let v = osc(wave, ph, dt);
            ph += dt;
            if ph >= 1.0 {
                ph -= 1.0;
            }
            v
        })
        .collect()
}

/// Two equal oscillators detuned by `detune_ct` around `f0` (each `±detune/2`), summed at half level each.
/// They beat at [`theory_beat`]`(f0, detune_ct)`.
#[must_use]
pub fn detuned_pair(
    sr: f64,
    wave: Wave,
    f0: f64,
    detune_ct: f64,
    seconds: f64,
    phases: (f64, f64),
) -> Vec<f32> {
    let f1 = f0 * libm::exp2(detune_ct / 2400.0);
    let f2 = f0 * libm::exp2(-detune_ct / 2400.0);
    let a = oscillator(sr, wave, f1, seconds, phases.0);
    let b = oscillator(sr, wave, f2, seconds, phases.1);
    a.iter().zip(&b).map(|(x, y)| 0.5 * x + 0.5 * y).collect()
}

/// Beat rate of two oscillators detuned by `detune_ct` around `f0`: `f0 · (2^(d/2400) − 2^(−d/2400))`.
#[must_use]
pub fn theory_beat(f0: f64, detune_ct: f64) -> f64 {
    f0 * (libm::exp2(detune_ct / 2400.0) - libm::exp2(-detune_ct / 2400.0))
}

/// The toy voice's parameters, in the units of `spike-agent`'s `toy10` patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToyPatch {
    pub wave: Wave,
    pub detune_ct: f64,
    pub cutoff_hz: f64,
    pub resonance_q: f64,
    pub env_amount_oct: f64,
    pub env_decay_ms: f64,
    pub attack_ms: f64,
    pub decay_ms: f64,
    /// Sustain level in dB; −60 or below means off.
    pub sustain_db: f64,
    pub release_ms: f64,
}

impl Default for ToyPatch {
    /// `spike-agent`'s neutral patch.
    fn default() -> Self {
        Self {
            wave: Wave::Saw,
            detune_ct: 0.0,
            cutoff_hz: 2000.0,
            resonance_q: 0.7,
            env_amount_oct: 0.0,
            env_decay_ms: 200.0,
            attack_ms: 5.0,
            decay_ms: 300.0,
            sustain_db: -6.0,
            release_ms: 200.0,
        }
    }
}

/// How the toy voice is played: one note from time 0, held for `hold_s`, in a clip of `dur_s`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToyPerf {
    pub note_hz: f64,
    pub dur_s: f64,
    pub hold_s: f64,
}

impl Default for ToyPerf {
    fn default() -> Self {
        Self {
            note_hz: 220.0,
            dur_s: 2.0,
            hold_s: 1.5,
        }
    }
}

/// Render the toy voice. `phases` are the oscillators' start phases (the research prototype's nuisance variable;
/// `(0.0, 0.37)` is `spike-metric`'s default) and `master_db` a gain after everything. The output is the float
/// signal, never clamped, so it may exceed full scale.
#[must_use]
pub fn render_toy(
    sr: f64,
    p: &ToyPatch,
    perf: &ToyPerf,
    phases: (f32, f32),
    master_db: f32,
) -> Vec<f32> {
    render_toy_lfo(sr, p, perf, phases, master_db, None)
}

/// A sine LFO on the toy voice's cutoff: `rate_hz`, and `depth_oct` octaves either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutoffLfo {
    pub rate_hz: f64,
    pub depth_oct: f64,
}

/// [`render_toy`] with an optional LFO on the cutoff (a filter wobble). Without one, the same bits as
/// [`render_toy`].
#[must_use]
pub fn render_toy_lfo(
    sr: f64,
    p: &ToyPatch,
    perf: &ToyPerf,
    phases: (f32, f32),
    master_db: f32,
    lfo: Option<CutoffLfo>,
) -> Vec<f32> {
    let sr32 = sr as f32;
    let n = (sr32 * perf.dur_s as f32) as usize;
    let note_off = perf.hold_s as f32;
    let det = p.detune_ct as f32;
    let f1 = perf.note_hz as f32 * libm::exp2f(det / 2.0 / 1200.0);
    let f2 = perf.note_hz as f32 * libm::exp2f(-det / 2.0 / 1200.0);
    let (dt1, dt2) = (f1 / sr32, f2 / sr32);
    let (mut ph1, mut ph2) = phases;
    let k = 1.0 / p.resonance_q as f32;
    let (cutoff, famt, fdec) = (
        p.cutoff_hz as f32,
        p.env_amount_oct as f32,
        (p.env_decay_ms / 1000.0) as f32,
    );
    let (a, d, r) = (
        (p.attack_ms / 1000.0) as f32,
        (p.decay_ms / 1000.0) as f32,
        (p.release_ms / 1000.0) as f32,
    );
    let s = if p.sustain_db <= -60.0 + 1e-9 {
        0.0
    } else {
        libm::pow(10.0, p.sustain_db / 20.0) as f32
    };
    let (mut ic1, mut ic2) = (0.0f32, 0.0f32);
    let (mut a1, mut a2, mut a3) = (0.0f32, 0.0f32, 0.0f32);
    let env_at = |t: f32| -> f32 {
        if t < a {
            t / a
        } else {
            s + (1.0 - s) * libm::expf(-(t - a) / (d * 0.25))
        }
    };
    let off_level = env_at(note_off);
    let (tau_d, tau_r) = (f64::from(d) * 0.25, f64::from(r) * 0.25);
    let (k_d, k_r) = (
        libm::exp(-1.0 / (sr * tau_d)),
        libm::exp(-1.0 / (sr * tau_r)),
    );
    let (mut e_d, mut e_r) = (f64::NAN, f64::NAN);
    let g = if master_db == 0.0 {
        1.0
    } else {
        libm::powf(10.0, master_db / 20.0)
    };
    let mut out = vec![0.0f32; n];
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / sr32;
        if i % 8 == 0 {
            let fe = libm::expf(-t / (fdec * 0.25));
            let oct = match lfo {
                None => famt * fe,
                Some(l) => {
                    famt * fe
                        + (l.depth_oct * libm::sin(2.0 * PI * l.rate_hz * f64::from(t))) as f32
                }
            };
            let fc = (cutoff * libm::exp2f(oct)).min(0.45 * sr32);
            let gg = libm::tanf(PI32 * fc / sr32);
            a1 = 1.0 / (1.0 + gg * (gg + k));
            a2 = gg * a1;
            a3 = gg * a2;
        }
        let x = 0.5 * osc(p.wave, ph1, dt1) + 0.5 * osc(p.wave, ph2, dt2);
        ph1 += dt1;
        if ph1 >= 1.0 {
            ph1 -= 1.0;
        }
        ph2 += dt2;
        if ph2 >= 1.0 {
            ph2 -= 1.0;
        }
        let v3 = x - ic2;
        let v1 = a1 * ic1 + a2 * v3;
        let v2 = ic2 + a2 * ic1 + a3 * v3;
        ic1 = 2.0 * v1 - ic1;
        ic2 = 2.0 * v2 - ic2;
        let env = if t < note_off {
            if t < a {
                t / a
            } else {
                e_d = if e_d.is_nan() {
                    libm::exp(-f64::from(t - a) / tau_d)
                } else {
                    e_d * k_d
                };
                s + (1.0 - s) * e_d as f32
            }
        } else {
            e_r = if e_r.is_nan() {
                f64::from(off_level) * libm::exp(-f64::from(t - note_off) / tau_r)
            } else {
                e_r * k_r
            };
            e_r as f32
        };
        *o = v2 * env * g;
    }
    out
}

/// xorshift64 in `0..1`; the sweeps' patch generator.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    pub fn log_uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo * libm::pow(hi / lo, self.next_f64())
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next_f64() * items.len() as f64) as usize).min(items.len() - 1)]
    }
}

/// A random toy patch as in the research prototype's sweeps (report 03, §F2): wave, note A2 to A5, cutoff,
/// resonance, filter envelope, ADSR; 3 s clip, 2 s hold; detune 0 (the caller sets it).
pub fn random_toy(rng: &mut Rng) -> (ToyPatch, ToyPerf) {
    let wave = rng.pick(&Wave::ALL);
    let note_hz = rng.pick(&[110.0, 130.81, 220.0, 261.63, 440.0, 523.25, 880.0]);
    let p = ToyPatch {
        wave,
        detune_ct: 0.0,
        cutoff_hz: rng.log_uniform(400.0, 16_000.0),
        resonance_q: rng.log_uniform(0.5, 3.0),
        env_amount_oct: if rng.next_f64() < 0.5 {
            0.0
        } else {
            rng.uniform(0.0, 3.0)
        },
        env_decay_ms: rng.log_uniform(20.0, 1000.0),
        attack_ms: rng.log_uniform(1.0, 300.0),
        decay_ms: rng.log_uniform(50.0, 2000.0),
        sustain_db: if rng.next_f64() < 0.2 {
            -60.0
        } else {
            rng.uniform(-30.0, 0.0)
        },
        release_ms: rng.log_uniform(30.0, 1000.0),
    };
    (
        p,
        ToyPerf {
            note_hz,
            dur_s: 3.0,
            hold_s: 2.0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toy_render_is_deterministic_and_finite() {
        let p = ToyPatch {
            detune_ct: 23.0,
            ..ToyPatch::default()
        };
        let a = render_toy(SR, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
        let b = render_toy(SR, &p, &ToyPerf::default(), (0.0, 0.37), 0.0);
        assert_eq!(a, b);
        assert_eq!(a.len(), 96_000);
        assert!(a.iter().all(|s| s.is_finite()));
        let peak = a.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1 && peak < 1.0, "{peak}");
    }

    #[test]
    fn theory_beat_is_symmetric_detune() {
        // 20 ct at C3: 1.511 Hz (two oscillators at ±10 ct)
        assert!((theory_beat(130.81, 20.0) - 1.511).abs() < 0.001);
    }

    #[test]
    fn adsr_levels() {
        let e = Adsr {
            attack_s: 0.01,
            decay_s: 0.2,
            sustain: 0.5,
            release_s: 0.4,
        };
        assert!((e.level(0.005, 1.0) - 0.5).abs() < 1e-12);
        assert!((e.level(0.9, 1.0) - 0.5).abs() < 1e-6);
        // one release time constant after note-off: e^-1
        assert!((e.level(1.1, 1.0) / e.level(1.0, 1.0) - libm::exp(-1.0)).abs() < 1e-9);
    }
}
