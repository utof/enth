//! Throwaway 10-parameter subtractive voice: two detuned polyBLEP oscillators into a TPT state-variable lowpass,
//! with a filter envelope and an amp ADSR. One voice, one note, offline, mono.
//!
//! Pure function of (config, parameters): no clocks, no I/O, no global RNG. Transcendentals come from `libm`.
//! The parameter vector is normalised (`0..=1` per entry); [`Patch`] holds the same values in real units.

use core::f32::consts::PI;
use serde::{Deserialize, Serialize};

pub const N_PARAMS: usize = 10;
pub const PARAM_NAMES: [&str; N_PARAMS] = [
    "wave",
    "detune_cents",
    "cutoff_hz",
    "resonance_q",
    "filter_env_octaves",
    "filter_env_decay_s",
    "attack_s",
    "decay_s",
    "sustain",
    "release_s",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Wave {
    Saw,
    Square,
    Triangle,
}

impl Wave {
    pub const ALL: [Wave; 3] = [Wave::Saw, Wave::Square, Wave::Triangle];

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Wave::Saw => 0,
            Wave::Square => 1,
            Wave::Triangle => 2,
        }
    }

    /// Centre of the wave's cell on the normalised axis (`0..1/3`, `1/3..2/3`, `2/3..1`).
    #[must_use]
    pub fn unit(self) -> f64 {
        (self.index() as f64 + 0.5) / 3.0
    }

    #[must_use]
    pub fn from_unit(u: f64) -> Wave {
        Wave::ALL[((u.clamp(0.0, 1.0) * 3.0) as usize).min(2)]
    }
}

/// Render settings. The scorer's canonical setup is 48 kHz, 2 s, note A3 held for 1.5 s.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderCfg {
    pub sample_rate: f32,
    pub duration_s: f32,
    pub note_off_s: f32,
    pub note_hz: f32,
}

impl Default for RenderCfg {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            duration_s: 2.0,
            note_off_s: 1.5,
            note_hz: 220.0,
        }
    }
}

impl RenderCfg {
    /// A quarter-length clip for debug-build tests; same code path, 4x cheaper.
    #[must_use]
    pub fn short() -> Self {
        Self {
            sample_rate: 48_000.0,
            duration_s: 0.5,
            note_off_s: 0.375,
            note_hz: 220.0,
        }
    }

    #[must_use]
    pub fn n_samples(&self) -> usize {
        (self.sample_rate * self.duration_s) as usize
    }
}

/// The ten parameters in real units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Patch {
    pub wave: Wave,
    pub detune_cents: f32,
    pub cutoff_hz: f32,
    pub resonance_q: f32,
    pub filter_env_octaves: f32,
    pub filter_env_decay_s: f32,
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
}

pub fn exp_map(u: f32, lo: f32, hi: f32) -> f32 {
    lo * libm::powf(hi / lo, u)
}

impl Patch {
    /// Normalised vector (`0..=1`, out-of-range entries clamp) to real units.
    #[must_use]
    pub fn from_unit(p: &[f64]) -> Patch {
        let pu = |i: usize| p[i].clamp(0.0, 1.0) as f32;
        Patch {
            wave: Wave::from_unit(p[0]),
            detune_cents: pu(1) * 50.0,
            cutoff_hz: exp_map(pu(2), 50.0, 16_000.0),
            resonance_q: exp_map(pu(3), 0.5, 10.0),
            filter_env_octaves: pu(4) * 6.0,
            filter_env_decay_s: exp_map(pu(5), 0.005, 2.0),
            attack_s: exp_map(pu(6), 0.001, 1.0),
            decay_s: exp_map(pu(7), 0.005, 2.0),
            sustain: pu(8),
            release_s: exp_map(pu(9), 0.005, 2.0),
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

/// Render a normalised parameter vector into `out` (resized to the clip length).
pub fn render_unit(cfg: &RenderCfg, p: &[f64], out: &mut Vec<f32>) {
    render(cfg, &Patch::from_unit(p), out);
}

/// Render a patch into `out` (resized to the clip length).
pub fn render(cfg: &RenderCfg, patch: &Patch, out: &mut Vec<f32>) {
    let sr = cfg.sample_rate;
    let n = cfg.n_samples();
    out.clear();
    out.resize(n, 0.0);
    let Patch {
        wave,
        detune_cents: det,
        cutoff_hz: cutoff,
        resonance_q: q,
        filter_env_octaves: famt,
        filter_env_decay_s: fdec,
        attack_s: a,
        decay_s: d,
        sustain: s,
        release_s: r,
    } = *patch;
    let f1 = cfg.note_hz * libm::exp2f(det / 2.0 / 1200.0);
    let f2 = cfg.note_hz * libm::exp2f(-det / 2.0 / 1200.0);
    let (dt1, dt2) = (f1 / sr, f2 / sr);
    let (mut ph1, mut ph2) = (0.0f32, 0.37f32);
    let k = 1.0 / q;
    let (mut ic1, mut ic2) = (0.0f32, 0.0f32);
    let (mut a1, mut a2, mut a3) = (0.0f32, 0.0f32, 0.0f32);
    let env_at = |t: f32| -> f32 {
        if t < a {
            t / a
        } else {
            s + (1.0 - s) * libm::expf(-(t - a) / (d * 0.25))
        }
    };
    let off_level = env_at(cfg.note_off_s);
    // The exponential stages run as per-sample recurrences (one multiply) started from a closed-form value, in f64 so
    // the accumulated rounding stays far below f32 precision.
    let (tau_d, tau_r) = (f64::from(d) * 0.25, f64::from(r) * 0.25);
    let (k_d, k_r) = (
        libm::exp(-1.0 / (f64::from(sr) * tau_d)),
        libm::exp(-1.0 / (f64::from(sr) * tau_r)),
    );
    let (mut e_d, mut e_r) = (f64::NAN, f64::NAN);
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        if i % 8 == 0 {
            let fe = libm::expf(-t / (fdec * 0.25));
            let fc = (cutoff * libm::exp2f(famt * fe)).min(0.45 * sr);
            let g = libm::tanf(PI * fc / sr);
            a1 = 1.0 / (1.0 + g * (g + k));
            a2 = g * a1;
            a3 = g * a2;
        }
        let x = 0.5 * osc(wave, ph1, dt1) + 0.5 * osc(wave, ph2, dt2); // exact halves: same bits as 0.5 * (a + b)
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
        let env = if t < cfg.note_off_s {
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
                f64::from(off_level) * libm::exp(-f64::from(t - cfg.note_off_s) / tau_r)
            } else {
                e_r * k_r
            };
            e_r as f32
        };
        *o = v2 * env;
    }
}
