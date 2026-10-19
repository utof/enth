//! The reference patch: which modules the panel shows, which host slot each knob is bound to,
//! and how values read in real units. Spike content, standing in for the patch catalogue.
//!
//! 64 slot knobs (8 generator strips, 3 lane cards) plus the 8 macros cover all 72 host
//! parameters of the shell spike. Slot 1 is the filter cutoff, as in spike-shell's DSP.

use enth_ui::theme::Category;

/// How a normalised value reads.
#[derive(Clone, Copy, Debug)]
pub enum Unit {
    Percent,
    /// Exponential frequency range.
    Hz(f32, f32),
    /// Exponential time range.
    Ms(f32, f32),
    Db(f32, f32),
    /// ± semitones, whole steps.
    Semitones(f32),
    /// ± cents.
    Cents(f32),
    /// Compression ratio 1:1 to max:1.
    Ratio(f32),
    /// Voice count 1..=max.
    Voices(f32),
}

impl Unit {
    pub fn format(self, normalized: f32) -> String {
        let v = normalized.clamp(0.0, 1.0);
        let exp = |lo: f32, hi: f32| lo * (hi / lo).powf(v);
        let lerp = |lo: f32, hi: f32| lo + (hi - lo) * v;
        match self {
            Unit::Percent => format!("{:.0} %", v * 100.0),
            Unit::Hz(lo, hi) => match exp(lo, hi) {
                hz if hz >= 1000.0 => format!("{:.2} kHz", hz / 1000.0),
                hz => format!("{hz:.0} Hz"),
            },
            Unit::Ms(lo, hi) => match exp(lo, hi) {
                ms if ms >= 1000.0 => format!("{:.2} s", ms / 1000.0),
                ms if ms >= 100.0 => format!("{ms:.0} ms"),
                ms => format!("{ms:.1} ms"),
            },
            Unit::Db(lo, hi) => format!("{:.1} dB", lerp(lo, hi)),
            Unit::Semitones(range) => format!("{:+.0} st", lerp(-range, range)),
            Unit::Cents(range) => format!("{:+.0} ct", lerp(-range, range)),
            Unit::Ratio(max) => format!("{:.1}:1", lerp(1.0, max)),
            Unit::Voices(max) => format!("{:.0}", lerp(1.0, max)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub unit: Unit,
    pub default: f32,
    pub bipolar: bool,
    /// Depth of LFO 1 on this parameter (normalised); 0 = unmodulated. Non-zero ones animate.
    pub lfo_depth: f32,
}

/// What a module's display shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Figure {
    Saw,
    Pulse,
    Sine,
    Noise,
    LowPass,
    Envelope,
    Spread,
    Shaper,
    Echoes,
    Decay,
}

#[derive(Clone, Copy, Debug)]
pub struct Module {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: &'static str,
    pub category: Category,
    pub figure: Figure,
    /// Host slot (0-based) of the first parameter; the rest follow.
    pub first_slot: usize,
    pub params: &'static [Param],
}

#[derive(Clone, Copy, Debug)]
pub struct MacroSpec {
    pub name: &'static str,
    pub default: f32,
}

const fn p(name: &'static str, unit: Unit, default: f32) -> Param {
    Param {
        name,
        unit,
        default,
        bipolar: false,
        lfo_depth: 0.0,
    }
}

const fn bi(name: &'static str, unit: Unit) -> Param {
    Param {
        name,
        unit,
        default: 0.5,
        bipolar: true,
        lfo_depth: 0.0,
    }
}

const fn lfo(param: Param, depth: f32) -> Param {
    Param {
        lfo_depth: depth,
        ..param
    }
}

const LEVEL: Unit = Unit::Db(-48.0, 6.0);
const TIME: Unit = Unit::Ms(1.0, 8000.0);

/// The generator column, top to bottom.
pub const GENERATORS: [Module; 8] = [
    Module {
        id: "osc-1",
        title: "Osc 1",
        kind: "Saw",
        category: Category::Generator,
        figure: Figure::Saw,
        first_slot: 6,
        params: &[
            p("Level", LEVEL, 0.82),
            bi("Semi", Unit::Semitones(24.0)),
            bi("Fine", Unit::Cents(100.0)),
            lfo(p("Shape", Unit::Percent, 0.35), 0.25),
            lfo(bi("Pan", Unit::Percent), 0.3),
            p("Phase", Unit::Percent, 0.0),
        ],
    },
    Module {
        id: "filter",
        title: "Filter",
        kind: "Low-pass 24",
        category: Category::Filter,
        figure: Figure::LowPass,
        first_slot: 0,
        params: &[
            lfo(p("Cutoff", Unit::Hz(20.0, 20000.0), 0.62), 0.3),
            lfo(p("Reso", Unit::Percent, 0.3), 0.15),
            p("Drive", Unit::Percent, 0.2),
            bi("Env amt", Unit::Percent),
            p("Key track", Unit::Percent, 0.5),
            p("Mix", Unit::Percent, 1.0),
        ],
    },
    Module {
        id: "amp-env",
        title: "Amp env",
        kind: "ADSR",
        category: Category::Generator,
        figure: Figure::Envelope,
        first_slot: 12,
        params: &[
            p("Attack", TIME, 0.1),
            p("Decay", TIME, 0.45),
            p("Sustain", Unit::Percent, 0.7),
            p("Release", TIME, 0.5),
            p("Velocity", Unit::Percent, 0.6),
            p("Curve", Unit::Percent, 0.5),
        ],
    },
    Module {
        id: "osc-2",
        title: "Osc 2",
        kind: "Pulse",
        category: Category::Generator,
        figure: Figure::Pulse,
        first_slot: 18,
        params: &[
            p("Level", LEVEL, 0.7),
            bi("Semi", Unit::Semitones(24.0)),
            lfo(bi("Fine", Unit::Cents(100.0)), 0.2),
            lfo(p("Width", Unit::Percent, 0.5), 0.2),
            bi("Pan", Unit::Percent),
            p("Phase", Unit::Percent, 0.25),
        ],
    },
    Module {
        id: "sub",
        title: "Sub",
        kind: "Sine",
        category: Category::Generator,
        figure: Figure::Sine,
        first_slot: 24,
        params: &[
            p("Level", LEVEL, 0.5),
            p("Octave", Unit::Semitones(24.0), 0.25),
            bi("Pan", Unit::Percent),
            p("Drive", Unit::Percent, 0.0),
            p("Phase", Unit::Percent, 0.0),
        ],
    },
    Module {
        id: "noise",
        title: "Noise",
        kind: "White",
        category: Category::Generator,
        figure: Figure::Noise,
        first_slot: 29,
        params: &[
            p("Level", LEVEL, 0.3),
            p("Colour", Unit::Percent, 0.5),
            bi("Pan", Unit::Percent),
            p("Width", Unit::Percent, 0.8),
            p("Decay", TIME, 0.4),
        ],
    },
    Module {
        id: "filter-env",
        title: "Filter env",
        kind: "ADSR",
        category: Category::Modulator,
        figure: Figure::Envelope,
        first_slot: 34,
        params: &[
            p("Attack", TIME, 0.05),
            p("Decay", TIME, 0.5),
            p("Sustain", Unit::Percent, 0.3),
            p("Release", TIME, 0.45),
            p("Delay", TIME, 0.0),
            bi("Amount", Unit::Percent),
        ],
    },
    Module {
        id: "unison",
        title: "Unison",
        kind: "Voices",
        category: Category::Generator,
        figure: Figure::Spread,
        first_slot: 40,
        params: &[
            p("Voices", Unit::Voices(16.0), 0.2),
            p("Detune", Unit::Cents(100.0), 0.6),
            p("Spread", Unit::Percent, 0.7),
            p("Blend", Unit::Percent, 0.5),
            p("Phase", Unit::Percent, 1.0),
            p("Glide", TIME, 0.0),
        ],
    },
];

/// The effect lane, in its initial order.
pub const LANE: [Module; 3] = [
    Module {
        id: "drive",
        title: "Drive",
        kind: "Soft clip",
        category: Category::Effect,
        figure: Figure::Shaper,
        first_slot: 46,
        params: &[
            lfo(p("Amount", Unit::Percent, 0.4), 0.2),
            p("Tone", Unit::Hz(200.0, 12000.0), 0.6),
            p("Bias", Unit::Percent, 0.5),
            p("Output", Unit::Db(-24.0, 6.0), 0.8),
            p("Asym", Unit::Percent, 0.1),
            p("Mix", Unit::Percent, 1.0),
        ],
    },
    Module {
        id: "delay",
        title: "Delay",
        kind: "Stereo",
        category: Category::Effect,
        figure: Figure::Echoes,
        first_slot: 52,
        params: &[
            lfo(p("Time", Unit::Ms(10.0, 2000.0), 0.55), 0.15),
            p("Feedback", Unit::Percent, 0.4),
            p("Tone", Unit::Hz(200.0, 12000.0), 0.5),
            p("Width", Unit::Percent, 0.8),
            p("Ducking", Unit::Percent, 0.2),
            p("Mix", Unit::Percent, 0.25),
        ],
    },
    Module {
        id: "reverb",
        title: "Reverb",
        kind: "Hall",
        category: Category::Effect,
        figure: Figure::Decay,
        first_slot: 58,
        params: &[
            p("Size", Unit::Percent, 0.7),
            p("Decay", Unit::Ms(100.0, 20000.0), 0.5),
            p("Damping", Unit::Percent, 0.45),
            p("Pre-delay", Unit::Ms(1.0, 200.0), 0.3),
            p("Width", Unit::Percent, 1.0),
            p("Mix", Unit::Percent, 0.3),
        ],
    },
];

pub const MACROS: [MacroSpec; 8] = [
    MacroSpec {
        name: "Level",
        default: 0.7,
    },
    MacroSpec {
        name: "Bright",
        default: 0.55,
    },
    MacroSpec {
        name: "Pluck",
        default: 0.3,
    },
    MacroSpec {
        name: "Space",
        default: 0.4,
    },
    MacroSpec {
        name: "Drive",
        default: 0.2,
    },
    MacroSpec {
        name: "Width",
        default: 0.6,
    },
    MacroSpec {
        name: "Motion",
        default: 0.35,
    },
    MacroSpec {
        name: "Air",
        default: 0.0,
    },
];

/// Modulation sources: matrix columns and source tiles.
pub const SOURCES: [&str; 6] = ["Env 2", "LFO 1", "LFO 2", "LFO 3", "Vel", "Key"];

/// Matrix rows.
pub const TARGETS: [&str; 16] = [
    "Cutoff",
    "Resonance",
    "Osc 1 shape",
    "Osc 1 pan",
    "Osc 2 fine",
    "Osc 2 width",
    "Drive",
    "Delay time",
    "Reverb mix",
    "Amp attack",
    "Filter env",
    "Sub level",
    "Noise level",
    "Unison detune",
    "Pitch",
    "Pan",
];

/// Initial matrix amounts, `TARGETS × SOURCES`, row-major. The LFO 1 column matches the
/// `lfo_depth` of the eight animated knobs.
pub fn matrix() -> [f32; TARGETS.len() * SOURCES.len()] {
    let mut m = [0.0; TARGETS.len() * SOURCES.len()];
    let lfo1 = 1;
    for (row, depth) in [0.3, 0.15, 0.25, 0.3, 0.2, 0.2, 0.2, 0.15]
        .into_iter()
        .enumerate()
    {
        m[row * SOURCES.len() + lfo1] = depth;
    }
    for (row, source, amount) in [
        (0, 0, 0.45),
        (0, 4, 0.2),
        (0, 5, 0.35),
        (8, 2, -0.1),
        (9, 4, -0.25),
        (10, 0, 0.6),
        (13, 3, 0.12),
        (14, 4, 0.05),
        (15, 2, 0.4),
    ] {
        m[row * SOURCES.len() + source] = amount;
    }
    m
}

/// Default of host slot `slot` (0.5 for slots the layout doesn't use).
pub fn slot_default(slot: usize) -> f32 {
    modules()
        .flat_map(|m| {
            m.params
                .iter()
                .enumerate()
                .map(move |(i, p)| (m.first_slot + i, p))
        })
        .find(|&(s, _)| s == slot)
        .map_or(0.5, |(_, p)| p.default)
}

/// Every module, generators first.
pub fn modules() -> impl Iterator<Item = &'static Module> {
    GENERATORS.iter().chain(LANE.iter())
}

/// The value of LFO 1 (−1..1) at `phase`.
pub fn lfo_wave(phase: f32) -> f32 {
    (std::f32::consts::TAU * phase).sin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::NUM_SLOTS;

    #[test]
    fn every_slot_is_bound_exactly_once() {
        let mut slots: Vec<usize> = modules()
            .flat_map(|m| (0..m.params.len()).map(move |i| m.first_slot + i))
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..NUM_SLOTS).collect::<Vec<_>>());
        assert_eq!(GENERATORS[1].params[0].name, "Cutoff");
        assert_eq!(GENERATORS[1].first_slot, 0, "slot 1 drives the DSP cutoff");
    }

    #[test]
    fn eight_knobs_animate() {
        let animated = modules()
            .flat_map(|m| m.params.iter())
            .filter(|p| p.lfo_depth != 0.0)
            .count();
        assert_eq!(animated, 8);
    }

    #[test]
    fn values_read_in_real_units() {
        assert_eq!(Unit::Hz(20.0, 20000.0).format(1.0), "20.00 kHz");
        assert_eq!(Unit::Hz(20.0, 20000.0).format(0.0), "20 Hz");
        assert_eq!(Unit::Ms(1.0, 8000.0).format(0.0), "1.0 ms");
        assert_eq!(Unit::Semitones(24.0).format(0.5), "+0 st");
        assert_eq!(Unit::Ratio(20.0).format(0.0), "1.0:1");
        assert_eq!(Unit::Percent.format(0.35), "35 %");
    }
}
