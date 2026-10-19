//! The spike's patch: plain values in real units (Hz, ms, cents, %, octaves, dB), a canonical byte
//! form with a version, and a seed -> patch generator that draws in *parameter space only*.
//!
//! The generator never renders, never accepts or rejects a draw based on audio, and never retries: the
//! same seed gives the same patch (bit for bit) on every platform, so "a challenge is just a seed"
//! survives a change of machine. Log-scaled draws go through `libm::exp2f` on purpose, so the patch
//! hash also exercises the pinned transcendental.

use sha2::{Digest, Sha256};

use crate::math;
use crate::prng::Rng;

/// Bumped whenever the canonical bytes or the generator's draw order change.
pub const PATCH_VERSION: u32 = 1;

/// One oscillator (polyBLEP saw).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Osc {
    pub semitones: f32,
    pub cents: f32,
    pub level_pct: f32,
}

/// Envelope: linear attack, exponential decay to sustain, exponential release (times are time constants).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Env {
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub sustain_pct: f32,
    pub release_ms: f32,
}

/// State-variable low-pass (TPT) followed by a rational tanh saturator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub cutoff_hz: f32,
    pub q: f32,
    /// Filter envelope to cutoff, octaves at full envelope.
    pub env_oct: f32,
    /// LFO to cutoff, octaves at full LFO.
    pub lfo_oct: f32,
    pub key_track_pct: f32,
    pub drive: f32,
}

/// One global sine LFO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    pub rate_hz: f32,
    pub vibrato_cents: f32,
}

/// 4-line feedback delay network with a Hadamard mix, standing in for the reverb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reverb {
    /// 0 bypasses the reverb entirely (no cost).
    pub mix_pct: f32,
    pub rt60_s: f32,
    pub damping_pct: f32,
}

/// A complete patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    /// Oscillators per voice actually run: 1..=3.
    pub osc_count: u8,
    pub osc: [Osc; 3],
    pub filter: Filter,
    pub amp_env: Env,
    pub filter_env: Env,
    pub lfo: Lfo,
    pub reverb: Reverb,
    pub master_db: f32,
}

const OSC_INTERVALS: [f32; 9] = [-24.0, -12.0, -7.0, 0.0, 0.0, 0.0, 5.0, 7.0, 12.0];

impl Patch {
    /// The simple template of Task 5: one saw oscillator, a resonant low-pass, an amplitude envelope.
    /// No LFO, no reverb. Run with 1 and 16 voices.
    #[must_use]
    pub fn simple() -> Self {
        let off = Osc {
            semitones: 0.0,
            cents: 0.0,
            level_pct: 0.0,
        };
        Self {
            osc_count: 1,
            osc: [
                Osc {
                    semitones: 0.0,
                    cents: 0.0,
                    level_pct: 100.0,
                },
                off,
                off,
            ],
            filter: Filter {
                cutoff_hz: 900.0,
                q: 1.2,
                env_oct: 3.0,
                lfo_oct: 0.0,
                key_track_pct: 50.0,
                drive: 1.5,
            },
            amp_env: Env {
                attack_ms: 5.0,
                decay_ms: 250.0,
                sustain_pct: 60.0,
                release_ms: 150.0,
            },
            filter_env: Env {
                attack_ms: 2.0,
                decay_ms: 330.0,
                sustain_pct: 0.0,
                release_ms: 200.0,
            },
            lfo: Lfo {
                rate_hz: 0.5,
                vibrato_cents: 0.0,
            },
            reverb: Reverb {
                mix_pct: 0.0,
                rt60_s: 1.0,
                damping_pct: 30.0,
            },
            master_db: -6.0,
        }
    }

    /// The P1-like template: 3 oscillators, a filter, 2 envelopes, an LFO and a reverb stand-in. Run with 8 voices.
    #[must_use]
    pub fn p1_like() -> Self {
        Self {
            osc_count: 3,
            osc: [
                Osc {
                    semitones: 0.0,
                    cents: -7.0,
                    level_pct: 100.0,
                },
                Osc {
                    semitones: 0.0,
                    cents: 7.0,
                    level_pct: 80.0,
                },
                Osc {
                    semitones: -12.0,
                    cents: 0.0,
                    level_pct: 60.0,
                },
            ],
            filter: Filter {
                cutoff_hz: 700.0,
                q: 2.0,
                env_oct: 3.0,
                lfo_oct: 0.5,
                key_track_pct: 60.0,
                drive: 1.8,
            },
            amp_env: Env {
                attack_ms: 12.0,
                decay_ms: 400.0,
                sustain_pct: 70.0,
                release_ms: 350.0,
            },
            filter_env: Env {
                attack_ms: 4.0,
                decay_ms: 500.0,
                sustain_pct: 10.0,
                release_ms: 300.0,
            },
            lfo: Lfo {
                rate_hz: 5.0,
                vibrato_cents: 8.0,
            },
            reverb: Reverb {
                mix_pct: 25.0,
                rt60_s: 2.5,
                damping_pct: 40.0,
            },
            master_db: -6.0,
        }
    }

    /// A random patch from a seed. Parameter space only: fixed draw order, no audio-dependent decisions.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut r = Rng::new(seed);
        let osc_count = 1 + r.below(3) as u8;
        let mut osc = [Osc {
            semitones: 0.0,
            cents: 0.0,
            level_pct: 0.0,
        }; 3];
        for o in &mut osc {
            o.semitones = OSC_INTERVALS[r.below(OSC_INTERVALS.len() as u64) as usize];
            o.cents = r.range(-25.0, 25.0);
            o.level_pct = r.range(30.0, 100.0);
        }
        let filter = Filter {
            cutoff_hz: log_range(&mut r, 40.0, 8.5),
            q: log_range(&mut r, 0.5, 5.0),
            env_oct: r.range(-4.0, 6.0),
            lfo_oct: r.range(0.0, 2.0),
            key_track_pct: r.range(0.0, 100.0),
            drive: r.range(1.0, 3.0),
        };
        let amp_env = random_env(&mut r);
        let filter_env = random_env(&mut r);
        let lfo = Lfo {
            rate_hz: log_range(&mut r, 0.1, 8.0),
            vibrato_cents: r.range(0.0, 50.0),
        };
        let reverb = Reverb {
            mix_pct: r.range(0.0, 60.0),
            rt60_s: log_range(&mut r, 0.3, 4.0),
            damping_pct: r.range(0.0, 90.0),
        };
        let master_db = r.range(-12.0, 0.0);
        Self {
            osc_count,
            osc,
            filter,
            amp_env,
            filter_env,
            lfo,
            reverb,
            master_db,
        }
    }

    /// Canonical bytes: magic, version, `osc_count`, then every f32 as little-endian bits in declaration order.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(256);
        b.extend_from_slice(b"enth-spike-patch");
        b.extend_from_slice(&PATCH_VERSION.to_le_bytes());
        b.push(self.osc_count);
        let mut f = |x: f32| b.extend_from_slice(&x.to_bits().to_le_bytes());
        for o in &self.osc {
            f(o.semitones);
            f(o.cents);
            f(o.level_pct);
        }
        let fl = &self.filter;
        for x in [
            fl.cutoff_hz,
            fl.q,
            fl.env_oct,
            fl.lfo_oct,
            fl.key_track_pct,
            fl.drive,
        ] {
            f(x);
        }
        for e in [&self.amp_env, &self.filter_env] {
            for x in [e.attack_ms, e.decay_ms, e.sustain_pct, e.release_ms] {
                f(x);
            }
        }
        f(self.lfo.rate_hz);
        f(self.lfo.vibrato_cents);
        f(self.reverb.mix_pct);
        f(self.reverb.rt60_s);
        f(self.reverb.damping_pct);
        f(self.master_db);
        b
    }

    /// SHA-256 of the canonical bytes, lowercase hex.
    #[must_use]
    pub fn hash(&self) -> String {
        hex(&Sha256::digest(self.canonical_bytes()))
    }
}

/// `lo * 2^(u * octaves)`: log-uniform in [lo, lo * 2^octaves), through the pinned exp2.
fn log_range(r: &mut Rng, lo: f32, octaves: f32) -> f32 {
    lo * math::exp2(r.unit() * octaves)
}

fn random_env(r: &mut Rng) -> Env {
    Env {
        attack_ms: log_range(r, 1.0, 10.0),
        decay_ms: log_range(r, 5.0, 10.0),
        sustain_pct: r.range(0.0, 100.0),
        release_ms: log_range(r, 5.0, 10.0),
    }
}

/// Lowercase hex of a byte slice.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
