//! What the cards' displays draw: simple models of each module's sound or shape over 0..1, computed from its real
//! parameter values. Content, not look: how they are drawn (colours, strokes, grid) comes from the theme in
//! `widgets::Display`, and how an envelope places its stages comes from the `size.cards` tokens. These are pictures
//! of the sound, not renders of it (the engine's telemetry feeds live displays in lane ED2).

use std::f32::consts::{PI, TAU};

use enth_patch::catalogue::{FilterMode, LfoShape, NoiseColor, Warp, Wave, Wavetable, WtTable};

/// Samples in a drawn cycle.
pub const WAVE_SAMPLES: usize = 128;

/// Peak level of the drawn waves (−1..1 display scale), so they don't touch the display's edges.
const PEAK: f32 = 0.8;
/// Harmonics in the band-limited waves (saw, pulse): enough to read as the shape, with the ringing of a real one.
const HARMONICS: usize = 8;

/// One cycle of an oscillator, `WAVE_SAMPLES` long, in −1..1. `pw` and `position` are 0..1.
pub fn osc(wave: Option<Wave>, pw: f32, table: Option<Wavetable>, position: f32) -> Vec<f32> {
    cycle(|x| match wave {
        Some(Wave::Sine) => sine(x),
        Some(Wave::Triangle) => triangle(x),
        Some(Wave::Saw) | None => saw(x),
        Some(Wave::Pulse) => pulse(x, pw),
        Some(Wave::Table) => wavetable(x, table, position),
    })
}

/// One cycle of a `wt` (lane SKS), `WAVE_SAMPLES` long, in −1..1: the frame of its table at `position` (0..1), read
/// through its phase warp (`amount` −1..1), after the formulas of `docs/dsp/wt.md`. A picture of the cycle the
/// oscillator reads, from the table's recipe with a few harmonics; phase modulation and feedback are not in it.
pub fn wt(table: Option<WtTable>, position: f32, warp: Option<Warp>, amount: f32) -> Vec<f32> {
    let p = position.clamp(0.0, 1.0);
    let raw = cycle(|x| table_frame(table, p, warped(warp, amount, x)));
    // The recipes differ in level (the generator normalises each frame's peak): so does the picture.
    let peak = raw.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    if peak > 1.0 {
        raw.into_iter().map(|s| s * PEAK / peak).collect()
    } else {
        raw
    }
}

/// The harmonics a drawn `wt` frame sums (the generator's tables hold 256).
const WT_HARMONICS: usize = 24;

/// A `wt` table's frame at `p` (0..1), read at phase `x` (0..1), after `docs/dsp/wt.md`'s table recipes.
fn table_frame(table: Option<WtTable>, p: f32, x: f32) -> f32 {
    match table {
        Some(WtTable::Basic) | None => wavetable(x, Some(Wavetable::Basic), p),
        // Chowning's two-operator FM at 1:1 and with the modulator an octave up; the index rises to 10.
        Some(WtTable::Fm) => PEAK * (TAU * x + 10.0 * p * (TAU * x).sin()).sin(),
        Some(WtTable::FmOctave) => PEAK * (TAU * x + 10.0 * p * (2.0 * TAU * x).sin()).sin(),
        // A saw through one resonant peak that sweeps up the harmonics (32^p, so 1 to 32).
        Some(WtTable::Formant) => {
            let centre = 32.0_f32.powf(p);
            let width = (0.15 * centre).max(0.75);
            (1..=WT_HARMONICS)
                .map(|k| {
                    let k = k as f32;
                    let peak = 1.0 + 5.0 / (1.0 + ((k - centre) / width).powi(2));
                    -peak / k * (TAU * k * x).sin()
                })
                .sum::<f32>()
                * PEAK
                * 0.3
        }
        // A few strong, widely spaced partials; position brings more of the 24 in (the last one fading).
        Some(WtTable::Metal) => {
            const PARTIALS: [f32; 24] = [
                1.0, 3.0, 5.0, 8.0, 11.0, 14.0, 18.0, 23.0, 29.0, 35.0, 42.0, 50.0, 59.0, 69.0,
                80.0, 92.0, 105.0, 119.0, 134.0, 150.0, 167.0, 185.0, 204.0, 224.0,
            ];
            let sounding = 3.0 + 21.0 * p;
            PARTIALS
                .iter()
                .enumerate()
                .map(|(j, &h)| {
                    let fade = (sounding - j as f32).clamp(0.0, 1.0);
                    let phase = (0.618_034 * (j * j) as f32).fract();
                    fade / (1.0 + j as f32).sqrt() * (TAU * (h * x + phase)).sin()
                })
                .sum::<f32>()
                * PEAK
                * 0.35
        }
    }
}

/// The phase a warp reads instead of `x`, after `docs/dsp/wt.md`'s maps; the identity at amount 0.
fn warped(warp: Option<Warp>, amount: f32, x: f32) -> f32 {
    let a = amount.clamp(-1.0, 1.0);
    if a == 0.0 {
        return x;
    }
    match warp {
        Some(Warp::Bend) => {
            let k = 8.0_f32.powf(-a);
            x / (x + k * (1.0 - x))
        }
        Some(Warp::Asymmetric) => {
            let knee = 0.5 - 0.45 * a;
            if x < 0.5 {
                x * 2.0 * knee
            } else {
                knee + (x - 0.5) * 2.0 * (1.0 - knee)
            }
        }
        Some(Warp::Squeeze) => {
            let w = 1.0 - 0.9 * a.abs();
            if a > 0.0 {
                (x / w).min(1.0)
            } else {
                ((x - (1.0 - w)) / w).max(0.0)
            }
        }
        Some(Warp::Sync) => (x * (1.0 + 7.0 * a.abs())).fract(),
        Some(Warp::Quantize) => {
            let steps = (256.0 * (-7.0 * a.abs()).exp2()).round().max(2.0);
            (x * steps).floor() / steps
        }
        Some(Warp::Off) | None => x,
    }
}

/// A short stretch of noise, in −1..1: the same samples every frame, pink smoothed from white.
pub fn noise(color: Option<NoiseColor>) -> Vec<f32> {
    let mut state = 0x2545_f491_u32;
    let mut white = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut low = 0.0;
    (0..WAVE_SAMPLES)
        .map(|_| {
            let w = white();
            match color {
                Some(NoiseColor::Pink) => {
                    /// One-pole smoothing for pink, and the gain that brings it back to the white level.
                    const POLE: f32 = 0.6;
                    const GAIN: f32 = 1.6;
                    low = POLE * low + (1.0 - POLE) * w;
                    (low * GAIN * PEAK).clamp(-1.0, 1.0)
                }
                _ => w * PEAK,
            }
        })
        .collect()
}

/// A filter's magnitude on a log-frequency axis (x 0..1, y 0..1): `cutoff` is the cutoff's normalised (log)
/// position, `resonance` 0..1.
pub fn filter(mode: Option<FilterMode>, cutoff: f32, resonance: f32, x: f32) -> f32 {
    /// Pass-band level.
    const LEVEL: f32 = 0.6;
    /// Height of the resonant bump at full resonance, and its width on the axis.
    const BUMP: f32 = 0.35;
    const BUMP_WIDTH: f32 = 0.04;
    /// Steepness of the slope outside the pass band.
    const SLOPE: f32 = 4.0;
    /// Width of a band-pass or notch at no resonance; resonance narrows it.
    const BAND: f32 = 0.12;
    let d = x - cutoff;
    let bump = resonance * BUMP * (-(d / BUMP_WIDTH).powi(2)).exp();
    let fall = |d: f32| if d > 0.0 { (d * SLOPE).powi(2) } else { 0.0 };
    let band = BAND * (1.0 - resonance * 0.5);
    let y = match mode {
        Some(FilterMode::Lp) | None => LEVEL + bump - fall(d),
        Some(FilterMode::Hp) => LEVEL + bump - fall(-d),
        Some(FilterMode::Bp) => (LEVEL + bump) * (-(d / band).powi(2)).exp(),
        Some(FilterMode::Notch) => LEVEL * (1.0 - (-(d / band).powi(2)).exp()),
    };
    y.clamp(0.0, 1.0)
}

/// An LFO's shape over one cycle (x 0..1, y 0..1).
pub fn lfo(shape: Option<LfoShape>, x: f32) -> f32 {
    /// Levels of the random shapes' steps: fixed, so the picture holds still.
    const STEPS: [f32; 6] = [0.7, 0.2, 0.9, 0.4, 0.6, 0.15];
    let x = x.clamp(0.0, 1.0);
    let n = STEPS.len() as f32;
    let at = (x * n).min(n - 1.0);
    match shape {
        Some(LfoShape::Sine) | None => 0.5 + 0.5 * (TAU * x).sin(),
        Some(LfoShape::Triangle) => 0.5 + 0.5 * triangle(x) / PEAK,
        Some(LfoShape::SawUp) => x,
        Some(LfoShape::SawDown) => 1.0 - x,
        Some(LfoShape::Square) => {
            if x < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        Some(LfoShape::SampleHold) => STEPS[at as usize],
        Some(LfoShape::SmoothRandom) => {
            let i = at.floor() as usize;
            let next = STEPS[(i + 1) % STEPS.len()];
            let t = at - at.floor();
            let ease = 0.5 - 0.5 * (PI * t).cos();
            STEPS[i] + (next - STEPS[i]) * ease
        }
    }
}

/// Where an envelope display puts its stages: each takes `min + range · n` of the width, `n` being the knob's own
/// normalised (power-scale) position, so the curve and its handles move on the knob's scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeStages {
    pub min: f32,
    pub range: f32,
}

impl EnvelopeStages {
    /// The share of the width of a stage at normalised length `n`.
    pub fn width(self, n: f32) -> f32 {
        self.min + self.range * n.clamp(0.0, 1.0)
    }

    /// The envelope over 0..1 for normalised attack, decay and release lengths and a sustain level (all 0..1). The
    /// release always ends at the right edge.
    pub fn curve(self, x: f32, a: f32, d: f32, s: f32, r: f32) -> f32 {
        let (a, d, r) = (self.width(a), self.width(d), self.width(r));
        let hold = (1.0 - a - d - r).max(0.0);
        match x {
            x if x < a => x / a,
            x if x < a + d => 1.0 - (1.0 - s) * (x - a) / d,
            x if x < a + d + hold => s,
            x => s * (1.0 - (x - a - d - hold) / r).max(0.0),
        }
    }

    /// The corners the display lets you drag, in its 0..1 space (y up): the attack peak, the end of the decay at
    /// the sustain level, and the start of the release.
    pub fn corners(self, a: f32, d: f32, s: f32, r: f32) -> [egui::Vec2; 3] {
        let peak = self.width(a);
        [
            egui::vec2(peak, 1.0),
            egui::vec2(peak + self.width(d), s),
            egui::vec2(1.0 - self.width(r), s),
        ]
    }
}

/// A compressor's static curve (x: input level, y: output level, both over the bottom `RANGE_DB` dB), with a soft
/// knee and make-up gain.
pub fn compressor(threshold: f32, ratio: f32, knee: f32, makeup: f32, x: f32) -> f32 {
    /// The level range the curve shows, in dB below full scale.
    const RANGE_DB: f32 = 60.0;
    let input = -RANGE_DB + RANGE_DB * x;
    let over = input - threshold;
    let slope = 1.0 / ratio.max(1.0) - 1.0;
    let gain = if 2.0 * over < -knee {
        0.0
    } else if knee > 0.0 && 2.0 * over.abs() <= knee {
        slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
    } else {
        slope * over
    };
    ((input + gain + makeup + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
}

/// A reverb's tail on a log-time axis from `FROM_MS` to `TO_MS` (x 0..1), its level in dB over `RANGE_DB` (y 0..1):
/// silent through the pre-delay of `room` ms, a burst of early reflections, then a fall of 60 dB over `decay` ms from
/// the wet level (dB). The log axis spans the decay's whole range, so every decay time reads as a different knee.
pub fn reverb(room: f32, decay: f32, wet: f32, early: f32, x: f32) -> f32 {
    /// The time the picture spans, in ms (the decay parameter's range).
    const FROM_MS: f32 = 10.0;
    const TO_MS: f32 = 30_000.0;
    /// The level range shown, and its top (the wet level's maximum).
    const RANGE_DB: f32 = 66.0;
    const TOP_DB: f32 = 6.0;
    /// How far the early reflections rise above the tail at 250 %, in dB, and how long they last against the room.
    const EARLY_DB: f32 = 6.0;
    const EARLY_ROOMS: f32 = 3.0;
    let t = FROM_MS * (TO_MS / FROM_MS).powf(x.clamp(0.0, 1.0));
    if t < room {
        return 0.0;
    }
    let since = t - room;
    let mut level = wet - 60.0 * since / decay.max(1.0);
    if since < EARLY_ROOMS * room {
        level += EARLY_DB * early / 250.0 * (1.0 - since / (EARLY_ROOMS * room));
    }
    ((level - TOP_DB + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
}

fn cycle(f: impl Fn(f32) -> f32) -> Vec<f32> {
    (0..WAVE_SAMPLES)
        .map(|i| f(i as f32 / WAVE_SAMPLES as f32))
        .collect()
}

fn sine(x: f32) -> f32 {
    PEAK * (TAU * x).sin()
}

fn triangle(x: f32) -> f32 {
    let t = (x + 0.25).fract();
    PEAK * (1.0 - 4.0 * (t - 0.5).abs())
}

fn saw(x: f32) -> f32 {
    crate::gallery::saw(x)
}

/// A band-limited pulse of duty `pw` (0..1), its mean removed.
fn pulse(x: f32, pw: f32) -> f32 {
    /// Level of the summed harmonics.
    const LEVEL: f32 = 1.1;
    (1..=HARMONICS)
        .map(|k| {
            let k = k as f32;
            2.0 / (k * PI) * (PI * k * pw).sin() * (TAU * k * (x - pw / 2.0)).cos()
        })
        .sum::<f32>()
        * LEVEL
        * PEAK
}

fn square(x: f32) -> f32 {
    pulse(x, 0.5)
}

/// A wavetable at `position` (0..1): a picture of what each table morphs through.
fn wavetable(x: f32, table: Option<Wavetable>, position: f32) -> f32 {
    let p = position.clamp(0.0, 1.0);
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    match table {
        Some(Wavetable::Basic) | None => {
            // Sine, triangle, saw, square, a third of the way each.
            let seg = (p * 3.0).min(2.999);
            let t = seg.fract();
            match seg as usize {
                0 => mix(sine(x), triangle(x), t),
                1 => mix(triangle(x), saw(x), t),
                _ => mix(saw(x), square(x), t),
            }
        }
        // From a square to the narrowest pulse.
        Some(Wavetable::Pulse) => pulse(x, 0.5 - 0.45 * p),
        Some(Wavetable::Tilt) => mix(saw(x), sine(x), p),
        Some(Wavetable::Formant) => {
            // A resonance that walks up the harmonics.
            const LOW: f32 = 2.0;
            const SPAN: f32 = 6.0;
            let formant = LOW + SPAN * p;
            let sum: f32 = (1..=HARMONICS)
                .map(|k| {
                    let k = k as f32;
                    (-(k - formant).powi(2) / 2.0).exp() * (TAU * k * x).sin()
                })
                .sum();
            (sum * PEAK).clamp(-1.0, 1.0)
        }
        Some(Wavetable::Organ) => {
            // Drawbars 16', 8', 5⅓', 4', 2⅔', 2': the upper ones come in along the table.
            const HARMONIC: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0];
            /// Level of the summed drawbars against their total weight.
            const LEVEL: f32 = 1.6;
            let sum: f32 = HARMONIC
                .iter()
                .enumerate()
                .map(|(i, &h)| {
                    let weight = if i < 2 { 1.0 } else { p };
                    weight * (TAU * h * x).sin()
                })
                .sum();
            (sum / (2.0 + 4.0 * p) * LEVEL * PEAK).clamp(-1.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_corners_sit_on_the_curve() {
        let stages = EnvelopeStages {
            min: 0.05,
            range: 0.25,
        };
        let (a, d, s, r) = (0.3, 0.5, 0.6, 0.2);
        for corner in stages.corners(a, d, s, r) {
            let y = stages.curve(corner.x, a, d, s, r);
            assert!((y - corner.y).abs() < 1e-3, "{corner:?} vs {y}");
        }
    }

    #[test]
    fn shapes_stay_in_range() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            for shape in LfoShape::ALL {
                assert!((0.0..=1.0).contains(&lfo(Some(*shape), x)));
            }
            for mode in FilterMode::ALL {
                assert!((0.0..=1.0).contains(&filter(Some(*mode), 0.5, 1.0, x)));
            }
            assert!((0.0..=1.0).contains(&compressor(-18.0, 4.0, 6.0, 0.0, x)));
            assert!((0.0..=1.0).contains(&reverb(80.0, 3000.0, -6.0, 150.0, x)));
        }
        for wave in Wave::ALL {
            for table in Wavetable::ALL {
                let samples = osc(Some(*wave), 0.5, Some(*table), 0.7);
                assert!(
                    samples.iter().all(|s| (-1.0..=1.0).contains(s)),
                    "{wave:?} {table:?}"
                );
            }
        }
        assert!(noise(Some(NoiseColor::Pink)).iter().all(|s| s.abs() <= 1.0));
        for table in WtTable::ALL {
            for warp in Warp::ALL {
                for position in [0.0, 0.3, 1.0] {
                    for amount in [-1.0, -0.4, 0.0, 0.4, 1.0] {
                        let samples = wt(Some(*table), position, Some(*warp), amount);
                        assert_eq!(samples.len(), WAVE_SAMPLES);
                        assert!(
                            samples.iter().all(|s| s.abs() <= 1.0),
                            "{table:?} {warp:?} {position} {amount}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_wt_at_position_0_and_no_warp_is_a_sine_and_a_warp_at_amount_0_changes_nothing() {
        let sine = osc(Some(Wave::Sine), 0.5, None, 0.0);
        let plain = wt(Some(WtTable::Basic), 0.0, Some(Warp::Off), 0.0);
        assert_eq!(plain, sine);
        for warp in Warp::ALL {
            assert_eq!(wt(Some(WtTable::Basic), 0.0, Some(*warp), 0.0), plain);
        }
        assert_ne!(wt(Some(WtTable::Basic), 0.0, Some(Warp::Bend), 0.6), plain);
    }
}
