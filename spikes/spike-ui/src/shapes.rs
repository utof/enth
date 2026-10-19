//! What the module displays and source tiles draw: simple models of each module's sound or
//! shape over 0..1, standing in for the engine's real responses. Content, not look: how they are
//! drawn (colours, strokes, grid) comes from the theme in `enth_ui::widgets::Display`.

use std::f32::consts::TAU;

/// Samples in a wave table.
const TABLE_SIZE: usize = 256;

/// Wave tables for the oscillator displays, computed once.
pub struct Tables {
    pub saw: Vec<f32>,
    pub sine: Vec<f32>,
    pub noise: Vec<f32>,
    /// Detuned saws, as the unison card shows them.
    pub spread: Vec<f32>,
}

impl Default for Tables {
    fn default() -> Self {
        Self::new()
    }
}

impl Tables {
    pub fn new() -> Self {
        /// Peak level of the sine and noise tables (−1..1 scale).
        const PEAK: f32 = 0.8;
        /// Unison: voices drawn and the detune step between them.
        const VOICES: usize = 4;
        const DETUNE: f32 = 0.03;
        let x = |i: usize| i as f32 / TABLE_SIZE as f32;
        let mut noise = Xorshift(0x2545_f491);
        Self {
            saw: (0..TABLE_SIZE)
                .map(|i| enth_ui::gallery::saw(x(i)))
                .collect(),
            sine: (0..TABLE_SIZE).map(|i| (TAU * x(i)).sin() * PEAK).collect(),
            noise: (0..TABLE_SIZE)
                .map(|_| noise.next_signed() * PEAK)
                .collect(),
            spread: (0..TABLE_SIZE)
                .map(|i| {
                    (0..VOICES)
                        .map(|v| enth_ui::gallery::saw((x(i) * (1.0 + DETUNE * v as f32)).fract()))
                        .sum::<f32>()
                        / VOICES as f32
                })
                .collect(),
        }
    }
}

/// A seeded xorshift32 for the noise table (deterministic snapshots).
struct Xorshift(u32);

impl Xorshift {
    /// Next value in −1..1.
    fn next_signed(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// A resonant low-pass magnitude on a log-frequency axis: flat below `cutoff`, a resonant bump
/// at it, a steep fall above. All values 0..1.
pub fn low_pass(x: f32, cutoff: f32, resonance: f32) -> f32 {
    /// Pass-band level.
    const LEVEL: f32 = 0.6;
    /// Height of the bump at full resonance, and its width on the axis.
    const BUMP: f32 = 0.5;
    const BUMP_WIDTH: f32 = 0.04;
    /// Steepness of the fall above the cutoff.
    const SLOPE: f32 = 4.0;
    let d = x - cutoff;
    let bump = resonance * BUMP * (-(d / BUMP_WIDTH).powi(2)).exp();
    let fall = if d > 0.0 { (d * SLOPE).powi(2) } else { 0.0 };
    (LEVEL + bump - fall).clamp(0.0, 1.0)
}

/// An ADSR envelope over 0..1: attack, decay and release lengths (normalised) scaled into the
/// width, sustain level `s`, the rest held.
pub fn adsr(x: f32, a: f32, d: f32, s: f32, r: f32) -> f32 {
    /// Shortest and longest share of the width one stage can take.
    const MIN_STAGE: f32 = 0.05;
    const STAGE_RANGE: f32 = 0.25;
    let stage = |v: f32| MIN_STAGE + v * STAGE_RANGE;
    let (a, d, r) = (stage(a), stage(d), stage(r));
    let hold = 1.0 - a - d - r;
    match x {
        x if x < a => x / a,
        x if x < a + d => 1.0 - (1.0 - s) * (x - a) / d,
        x if x < a + d + hold => s,
        x => s * (1.0 - (x - a - d - hold) / r).max(0.0),
    }
}

/// A pulse wave with duty cycle `width` (0..1), drawn between two levels.
pub fn pulse(x: f32, width: f32) -> f32 {
    const NARROWEST: f32 = 0.05;
    const HIGH: f32 = 0.85;
    const LOW: f32 = 0.15;
    if x.fract() < width.clamp(NARROWEST, 1.0 - NARROWEST) {
        HIGH
    } else {
        LOW
    }
}

/// A soft clipper's transfer curve (input 0..1 → output 0..1) for drive `amount` (0..1).
pub fn shaper(x: f32, amount: f32) -> f32 {
    const MAX_DRIVE: f32 = 8.0;
    let drive = 1.0 + amount * MAX_DRIVE;
    0.5 + 0.5 * (drive * (2.0 * x - 1.0)).tanh() / drive.tanh()
}

/// A delay's echo train: taps at even spacing, each `feedback` times the last.
pub fn echoes(x: f32, feedback: f32) -> f32 {
    const TAPS: f32 = 6.0;
    /// Share of a tap's slot its spike fills.
    const SPIKE: f32 = 0.08;
    const FIRST: f32 = 0.9;
    const QUIETEST_FEEDBACK: f32 = 0.05;
    let slot = x * TAPS;
    if slot.fract() < SPIKE {
        FIRST * feedback.max(QUIETEST_FEEDBACK).powf(slot.floor())
    } else {
        0.0
    }
}

/// A reverb's exponential tail for decay `amount` (0..1).
pub fn decay(x: f32, amount: f32) -> f32 {
    const SHORTEST: f32 = 0.5;
    const RANGE: f32 = 4.0;
    const SPAN: f32 = 8.0;
    const START: f32 = 0.9;
    START * (-(x * SPAN) / (SHORTEST + amount * RANGE)).exp()
}

/// Source-tile shapes (0..1).
pub fn triangle(x: f32) -> f32 {
    1.0 - (2.0 * x - 1.0).abs()
}

pub fn ramp_up(x: f32) -> f32 {
    x
}

pub fn ramp_down(x: f32) -> f32 {
    1.0 - x
}

/// A source that isn't moving (e.g. the mod wheel at rest).
pub fn flat(_: f32) -> f32 {
    0.5
}

/// Sample-and-hold steps.
pub fn steps(x: f32) -> f32 {
    const LEVELS: [f32; 6] = [0.3, 0.8, 0.5, 0.15, 0.65, 0.4];
    LEVELS[((x * LEVELS.len() as f32) as usize).min(LEVELS.len() - 1)]
}
