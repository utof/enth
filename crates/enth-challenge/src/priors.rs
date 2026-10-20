//! **Owner: lane RG** (the randomizer). Pre-declared empty by lane S0r.
//!
//! The hand-written priors of generator v1 (`priors::v1`, design.md §9, report 01 §2.3), and the just-noticeable
//! steps ([`jnd`]) the minimum step and the overlay's "audibly different" test use.
//!
//! The priors are relative to the phrase's note (C3, through osc 1's transposition): an LP cutoff at least 1.5 × f0,
//! a high-pass around the fundamental. Resonance and drive are skewed low, semitones sit on intervals, envelopes are
//! a pluck / pad / keys mixture, and the wave weights favour saw and pulse. `musical` blends towards uniform
//! (`random`). Envelope times never exceed the phrases' shortest hold and tail (1.5 s), so every drawn time is heard.
//! Maths through `libm` only (design.md §5).
//!
//! **A prior change is a new generator version**: [`v1::SLOTS`] is frozen with its 1,000-hash golden. Priors fitted
//! to enth's own presets come later, as v2.

use enth_params::{ParamSpec, Unit};

/// A just-noticeable step: how far a value must move from the start to count as audibly different (design.md §9:
/// cutoff 2 semitones, times × 1.5, levels 3 dB, a discrete change always). Hand-written, by unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Jnd {
    /// Any change (discrete values, enable flags).
    Any,
    /// An absolute difference in plain units (%, dB, semitones, cents).
    Abs(f32),
    /// A ratio, `(v + floor)` against `(start + floor)`; the floor keeps times near 0 from counting every
    /// microsecond.
    Ratio {
        /// The ratio that counts (1.5 for times).
        factor: f32,
        /// Added to both values first.
        floor: f32,
    },
    /// A frequency interval in semitones.
    Semitones(f32),
    /// A level difference in dB between two linear percentages, `(v + floor)` against `(start + floor)`.
    Db {
        /// The difference that counts.
        db: f32,
        /// Added to both values first (so 0 % is finite).
        floor: f32,
    },
}

impl Jnd {
    /// Whether `to` is at least one step from `from`.
    pub fn exceeded(self, from: f32, to: f32) -> bool {
        let (a, b) = (f64::from(from), f64::from(to));
        match self {
            Jnd::Any => from != to,
            Jnd::Abs(step) => (b - a).abs() >= f64::from(step),
            Jnd::Ratio { factor, floor } => {
                let (a, b) = (a + f64::from(floor), b + f64::from(floor));
                a > 0.0 && b > 0.0 && libm::log(b / a).abs() >= libm::log(f64::from(factor))
            }
            Jnd::Semitones(st) => {
                a > 0.0 && b > 0.0 && 12.0 * libm::log2(b / a).abs() >= f64::from(st)
            }
            Jnd::Db { db, floor } => {
                let (a, b) = (a + f64::from(floor), b + f64::from(floor));
                a > 0.0 && b > 0.0 && 20.0 * libm::log10(b / a).abs() >= f64::from(db)
            }
        }
    }

    /// The value 1.1 steps from `from`, up (`dir` = 1) or down (−1): the minimum step's forced value, before
    /// rounding.
    pub fn beyond(self, from: f32, dir: f32) -> f32 {
        let (a, k) = (f64::from(from), 1.1 * f64::from(dir));
        let v = match self {
            Jnd::Any => a,
            Jnd::Abs(step) => a + k * f64::from(step),
            Jnd::Ratio { factor, floor } => {
                let f = f64::from(floor);
                (a + f) * libm::pow(f64::from(factor), k) - f
            }
            Jnd::Semitones(st) => a * libm::pow(2.0, k * f64::from(st) / 12.0),
            Jnd::Db { db, floor } => {
                let f = f64::from(floor);
                (a + f) * libm::pow(10.0, k * f64::from(db) / 20.0) - f
            }
        };
        v as f32
    }
}

/// A route depth's step: one 5 % step (a route the start lacks always counts, since its depth is never 0).
pub const ROUTE_JND: Jnd = Jnd::Abs(5.0);

/// The just-noticeable step of a parameter, from its unit (generator v1's table; ST and the overlay may reuse it).
pub fn jnd(spec: &ParamSpec) -> Jnd {
    if spec.is_discrete() {
        return Jnd::Any;
    }
    match spec.unit {
        Unit::Percent if spec.key == "level" => Jnd::Db {
            db: 3.0,
            floor: 1.0,
        },
        Unit::Hz if spec.key == "rate" => Jnd::Ratio {
            factor: 1.5,
            floor: 0.0,
        },
        Unit::Hz => Jnd::Semitones(2.0),
        Unit::Db | Unit::Cents => Jnd::Abs(3.0),
        Unit::Semitones => Jnd::Abs(1.0),
        Unit::Octaves => Jnd::Abs(1.0 / 6.0),
        Unit::Percent => Jnd::Abs(10.0),
        Unit::Degrees => Jnd::Abs(30.0),
        // Times (ms: × 1.5, with 1 ms added so times near 0 do not count every microsecond), ratios (`comp.ratio`),
        // unitless values (reverb detune and damping) and any unit added later.
        _ => Jnd::Ratio {
            factor: 1.5,
            floor: 1.0,
        },
    }
}

/// Whether a parameter moved audibly: [`jnd`]`(spec).exceeded(from, to)`.
pub fn exceeds_jnd(spec: &ParamSpec, from: f32, to: f32) -> bool {
    jnd(spec).exceeded(from, to)
}

/// Log-uniform in `[lo, hi]` from a uniform `u`: `lo · (hi / lo)^u`, in `f64` through `libm`.
pub(crate) fn log_uniform(u: f32, lo: f64, hi: f64) -> f32 {
    (lo * libm::pow(hi / lo, f64::from(u))) as f32
}

/// Generator v1's table: every value it may draw, by category, in rank order (Basic tier first), with its prior,
/// its uniform range, whether the category's default set draws it, and whether it counts towards the minimum step.
pub mod v1 {
    use super::log_uniform;
    use crate::phrases::{LEGATO_PAIR, MIN_HOLD_MS, PhraseFacts};
    use crate::random::{Ctx, Domain, Draws, Slot, What, always};
    use crate::spec::Category::{
        self, Effects, Envelopes, Filter, Modulation, Oscillators, Structure,
    };

    /// The phrase's prior: the held note, the octave pair or the legato pair (when structure is randomized).
    pub(crate) const PHRASE_WEIGHTS: [(f32, u32); 3] = [(1.0, 50), (2.0, 30), (3.0, 20)];

    /// The longest envelope time drawn: every v1 phrase holds and tails at least this long.
    const TIME_MAX: f32 = MIN_HOLD_MS as f32;

    // Item indices (the catalogue's enums, as stored values).
    const WAVES_ALL: &[u8] = &[0, 1, 2, 3, 4]; // sine, triangle, saw, pulse, table
    const WAVE_PRIOR: &[(f32, u32)] = &[(2.0, 45), (3.0, 30), (1.0, 15), (0.0, 10)];
    const MODES_ALL: &[u8] = &[0, 1, 2, 3]; // lp, bp, hp, notch
    /// LFO shapes without the random ones (sample & hold, smooth random): those follow the render seed, a nuisance.
    const SHAPES: &[u8] = &[0, 1, 2, 3, 4]; // sine, triangle, saw up, saw down, square
    const SHAPE_PRIOR: &[(f32, u32)] = &[(0.0, 55), (1.0, 20), (4.0, 10), (2.0, 8), (3.0, 7)];

    const fn slot(category: Category, what: What) -> Slot {
        Slot {
            what,
            category,
            default: true,
            step: true,
            when: always,
        }
    }

    const fn param(
        category: Category,
        addr: &'static str,
        prior: fn(&Ctx<'_>, &Draws<'_>) -> f32,
        uniform: fn(&Ctx<'_>) -> Domain,
    ) -> Slot {
        slot(
            category,
            What::Param {
                addr,
                prior,
                uniform,
            },
        )
    }

    impl Slot {
        /// Not in the default set: drawn only when the category's amount is set.
        const fn extra(mut self) -> Slot {
            self.default = false;
            self
        }

        /// Does not count towards the minimum step (its visibility depends on a level-5 choice).
        const fn uncounted(mut self) -> Slot {
            self.step = false;
            self
        }

        const fn when(mut self, when: fn(&Ctx<'_>) -> bool) -> Slot {
            self.when = when;
            self
        }
    }

    fn legato_phrase(c: &Ctx<'_>) -> bool {
        c.phrase_id == LEGATO_PAIR
    }

    fn semi_range(c: &Ctx<'_>) -> Domain {
        if c.safe {
            Domain::Range(-24.0, 24.0)
        } else {
            Domain::Range(-48.0, 48.0)
        }
    }

    fn pct(_: &Ctx<'_>) -> Domain {
        Domain::Range(0.0, 100.0)
    }

    fn pw_range(_: &Ctx<'_>) -> Domain {
        Domain::Range(5.0, 95.0)
    }

    fn level_range(_: &Ctx<'_>) -> Domain {
        Domain::Range(5.0, 100.0)
    }

    fn times(_: &Ctx<'_>) -> Domain {
        Domain::Range(0.0, TIME_MAX)
    }

    fn waves(_: &Ctx<'_>) -> Domain {
        Domain::Items(WAVES_ALL)
    }

    fn wave_prior(_: &Ctx<'_>, d: &Draws<'_>) -> f32 {
        d.pick(WAVE_PRIOR)
    }

    fn pw_prior(_: &Ctx<'_>, d: &Draws<'_>) -> f32 {
        10.0 + 40.0 * d.u()
    }

    fn position_prior(_: &Ctx<'_>, d: &Draws<'_>) -> f32 {
        100.0 * d.u()
    }

    fn cutoff_prior(c: &Ctx<'_>, d: &Draws<'_>) -> f32 {
        let f0 = c.f0();
        let (lo, hi) = match c.value("filter1.mode").map_or(0, |m| m as u8) {
            0 => (1.5 * f0, 12_000.0), // low-pass: above the fundamental
            1 => (f0, 8.0 * f0),       // band-pass: within the strong harmonics
            2 => (0.5 * f0, 4.0 * f0), // high-pass: around the fundamental
            _ => (1.5 * f0, 8_000.0),  // notch
        };
        let (lo, hi) = (lo.clamp(20.0, 19_000.0), hi.clamp(20.0, 20_000.0));
        log_uniform(d.u(), lo, hi.max(lo * 1.5))
    }

    fn cutoff_range(c: &Ctx<'_>) -> Domain {
        if !c.safe {
            return Domain::Range(20.0, 20_000.0);
        }
        let f0 = c.f0() as f32;
        match c.value("filter1.mode").map_or(0, |m| m as u8) {
            0 => Domain::Range(f0.max(20.0), 20_000.0),
            1 => Domain::Range((0.5 * f0).max(20.0), 12_000.0),
            2 => Domain::Range(20.0, (8.0 * f0).clamp(40.0, 20_000.0)),
            _ => Domain::Range((0.5 * f0).max(20.0), 16_000.0),
        }
    }

    /// The amp envelope's archetype, shared by its four times (attempt 0): pluck, pad or keys.
    fn env1_shape(d: &Draws<'_>) -> u8 {
        d.pick_shared("env1/shape", &[(0.0, 45), (1.0, 25), (2.0, 30)]) as u8
    }

    fn lfo_rate_prior(c: &Ctx<'_>, d: &Draws<'_>, src: &str) -> f32 {
        let (lo, hi) = match c.route_dst(src).as_deref() {
            Some("osc1.fine") => (4.0, 7.0),      // vibrato
            Some("filter1.cutoff") => (0.3, 4.0), // wah and sweeps
            Some("osc1.level") => (2.0, 8.0),     // tremolo
            Some("osc1.pw") => (0.3, 3.0),        // PWM
            _ => (0.5, 5.0),
        };
        log_uniform(d.u(), lo, hi)
    }

    /// An LFO's destination: vibrato, wah, tremolo or (on a pulse) PWM, never `exclude`.
    fn lfo_dst(
        c: &Ctx<'_>,
        d: &Draws<'_>,
        prior: bool,
        name: &str,
        exclude: Option<&str>,
    ) -> Option<&'static str> {
        let pulse = c.wave("osc1") == Some(enth_patch::catalogue::Wave::Pulse);
        let candidates = [
            ("osc1.fine", 35),
            ("filter1.cutoff", 40),
            ("osc1.level", 25),
            ("osc1.pw", if pulse { 20 } else { 0 }),
        ];
        let items: Vec<(f32, u32)> = candidates
            .iter()
            .enumerate()
            .filter(|(_, (dst, w))| *w > 0 && Some(*dst) != exclude)
            .map(|(i, (_, w))| (i as f32, if prior { *w } else { 1 }))
            .collect();
        if items.is_empty() {
            return None;
        }
        Some(candidates[d.pick_shared(name, &items) as usize].0)
    }

    fn lfo_depth(_: &Ctx<'_>, d: &Draws<'_>, dst: &str) -> f32 {
        match dst {
            "filter1.cutoff" => 5.0 + 20.0 * d.u(), // ±0.5-2.5 octaves
            "osc1.level" => 10.0 + 30.0 * d.u(),
            _ => 10.0 + 20.0 * d.u(), // vibrato on osc1.fine (±20-60 cents), PWM
        }
    }

    fn lfo_rates(_: &Ctx<'_>) -> Domain {
        Domain::Range(0.25, 20.0)
    }

    fn shapes(_: &Ctx<'_>) -> Domain {
        Domain::Items(SHAPES)
    }

    fn shape_prior(_: &Ctx<'_>, d: &Draws<'_>) -> f32 {
        d.pick(SHAPE_PRIOR)
    }

    /// The v1 table. Its order is the rank within each category; the categories are drawn in `random`'s order.
    pub(crate) static SLOTS: &[Slot] = &[
        // --- Structure (level 5): extra sources, the phrase's voice behaviour, more routes ---
        slot(
            Structure,
            What::Enable {
                id: "osc2",
                percent: 50,
            },
        ),
        slot(
            Structure,
            What::Enable {
                id: "noise1",
                percent: 30,
            },
        ),
        param(
            Structure,
            "voice.mode",
            |_, _| 2.0,
            |_| Domain::Items(&[1, 2]),
        )
        .when(legato_phrase)
        .uncounted(),
        param(
            Structure,
            "voice.glide",
            |_, d| log_uniform(d.u(), 30.0, 400.0),
            |_| Domain::Range(5.0, 1_000.0),
        )
        .when(legato_phrase),
        slot(
            Structure,
            What::Route {
                src: "velocity",
                dst: |c, _, _| c.phrase.has_two_velocities().then_some("filter1.cutoff"),
                depth: |_, d, _| 10.0 + 20.0 * d.u(),
            },
        ),
        // --- Oscillators (level 1): osc 1's wave and octave, then what structure switched on ---
        param(Oscillators, "osc1.wave", wave_prior, waves),
        param(
            Oscillators,
            "osc1.semi",
            |_, d| d.pick(&[(0.0, 50), (-12.0, 25), (12.0, 25)]),
            semi_range,
        ),
        param(Oscillators, "osc1.pw", pw_prior, pw_range),
        param(Oscillators, "osc1.position", position_prior, pct),
        param(Oscillators, "osc2.wave", wave_prior, waves).uncounted(),
        param(
            Oscillators,
            "osc2.semi",
            |_, d| {
                d.pick(&[
                    (0.0, 40),
                    (12.0, 20),
                    (-12.0, 15),
                    (7.0, 15),
                    (19.0, 5),
                    (5.0, 5),
                ])
            },
            semi_range,
        )
        .uncounted(),
        param(
            Oscillators,
            "osc2.fine",
            |_, d| {
                let cents = 3.0 + 17.0 * d.u();
                if d.sub("sign") < 0.5 { -cents } else { cents }
            },
            |_| Domain::Range(-100.0, 100.0),
        )
        .uncounted(),
        param(
            Oscillators,
            "osc2.level",
            |_, d| 30.0 + 60.0 * d.u(),
            level_range,
        )
        .uncounted(),
        param(Oscillators, "osc2.pw", pw_prior, pw_range).uncounted(),
        param(Oscillators, "osc2.position", position_prior, pct).uncounted(),
        param(
            Oscillators,
            "noise1.color",
            |_, d| d.pick(&[(0.0, 50), (1.0, 50)]),
            |_| Domain::Items(&[0, 1]),
        )
        .uncounted(),
        param(
            Oscillators,
            "noise1.level",
            |_, d| 5.0 + 25.0 * d.u(),
            level_range,
        )
        .uncounted(),
        // --- Filter (level 2) ---
        param(
            Filter,
            "filter1.mode",
            |_, d| d.pick(&[(0.0, 70), (1.0, 10), (2.0, 15), (3.0, 5)]),
            |_| Domain::Items(MODES_ALL),
        ),
        param(Filter, "filter1.cutoff", cutoff_prior, cutoff_range),
        param(
            Filter,
            "filter1.resonance",
            |_, d| {
                let u = d.u();
                60.0 * u * u.sqrt()
            },
            |c| {
                if c.safe {
                    Domain::Range(0.0, 80.0)
                } else {
                    Domain::Range(0.0, 100.0)
                }
            },
        ),
        param(
            Filter,
            "filter1.drive",
            |_, d| {
                let u = d.u();
                9.0 * u * u
            },
            |c| {
                if c.safe {
                    Domain::Range(0.0, 12.0)
                } else {
                    Domain::Range(0.0, 24.0)
                }
            },
        )
        .extra(),
        param(
            Filter,
            "filter1.keytrack",
            |_, d| d.pick(&[(0.0, 20), (25.0, 20), (50.0, 30), (100.0, 30)]),
            pct,
        )
        .uncounted(),
        // --- Envelopes (level 3): the amp envelope as a pluck/pad/keys mixture, the filter envelope and its depth ---
        param(
            Envelopes,
            "env1.attack",
            |_, d| match env1_shape(d) {
                0 => log_uniform(d.u(), 0.5, 15.0),
                1 => log_uniform(d.u(), 80.0, 1_200.0),
                _ => log_uniform(d.u(), 1.0, 30.0),
            },
            times,
        ),
        param(
            Envelopes,
            "env1.decay",
            |_, d| match env1_shape(d) {
                0 => log_uniform(d.u(), 80.0, 800.0),
                1 => log_uniform(d.u(), 200.0, 1_500.0),
                _ => log_uniform(d.u(), 150.0, 1_500.0),
            },
            times,
        ),
        param(
            Envelopes,
            "env1.sustain",
            |_, d| match env1_shape(d) {
                0 => 15.0 * d.u(),
                1 => 50.0 + 50.0 * d.u(),
                _ => 40.0 + 50.0 * d.u(),
            },
            pct,
        ),
        param(
            Envelopes,
            "env1.release",
            |_, d| match env1_shape(d) {
                0 => log_uniform(d.u(), 30.0, 500.0),
                1 => log_uniform(d.u(), 300.0, 1_500.0),
                _ => log_uniform(d.u(), 60.0, 800.0),
            },
            times,
        ),
        slot(
            Envelopes,
            What::Route {
                src: "env2",
                dst: |_, _, _| Some("filter1.cutoff"),
                depth: |_, d, _| 10.0 + 40.0 * d.u(),
            },
        ),
        param(
            Envelopes,
            "env2.decay",
            |_, d| log_uniform(d.u(), 80.0, 1_200.0),
            times,
        ),
        param(Envelopes, "env2.sustain", |_, d| 40.0 * d.u(), pct),
        param(Envelopes, "env1.velocity", |_, d| 20.0 + 60.0 * d.u(), pct).uncounted(),
        param(
            Envelopes,
            "env2.attack",
            |_, d| log_uniform(d.u(), 0.5, 20.0),
            times,
        )
        .extra(),
        param(
            Envelopes,
            "env2.release",
            |_, d| log_uniform(d.u(), 30.0, 1_200.0),
            times,
        )
        .extra(),
        // --- Modulation (level 4): one or two LFO routes and their rates ---
        slot(
            Modulation,
            What::Route {
                src: "lfo1",
                dst: |c, d, prior| lfo_dst(c, d, prior, "lfo1/dst", None),
                depth: lfo_depth,
            },
        ),
        param(
            Modulation,
            "lfo1.rate",
            |c, d| lfo_rate_prior(c, d, "lfo1"),
            lfo_rates,
        ),
        param(Modulation, "lfo1.shape", shape_prior, shapes),
        slot(
            Modulation,
            What::Route {
                src: "lfo2",
                dst: |c, d, prior| {
                    let percent = if prior { 35.0 } else { 50.0 };
                    if d.shared("lfo2/on") * 100.0 >= percent {
                        return None;
                    }
                    let first = c.route_dst("lfo1");
                    lfo_dst(c, d, prior, "lfo2/dst", first.as_deref())
                },
                depth: lfo_depth,
            },
        ),
        param(
            Modulation,
            "lfo2.rate",
            |c, d| lfo_rate_prior(c, d, "lfo2"),
            lfo_rates,
        ),
        param(Modulation, "lfo2.shape", shape_prior, shapes),
        // --- Effects (level 5): the reverb and the compressor, and their Basic settings ---
        slot(
            Effects,
            What::Enable {
                id: "reverb1",
                percent: 60,
            },
        ),
        param(
            Effects,
            "reverb1.wet",
            |_, d| -24.0 + 16.0 * d.u(),
            |c| {
                if c.safe {
                    Domain::Range(-36.0, 0.0)
                } else {
                    Domain::Range(-48.0, 6.0)
                }
            },
        ),
        param(
            Effects,
            "reverb1.decay",
            |_, d| log_uniform(d.u(), 600.0, 5_000.0),
            |_| Domain::Range(200.0, 10_000.0),
        ),
        param(
            Effects,
            "reverb1.room",
            |_, d| 30.0 + 120.0 * d.u(),
            |_| Domain::Range(10.0, 200.0),
        ),
        slot(
            Effects,
            What::Enable {
                id: "comp1",
                percent: 30,
            },
        ),
        param(
            Effects,
            "comp1.threshold",
            |_, d| -30.0 + 20.0 * d.u(),
            |_| Domain::Range(-48.0, 0.0),
        ),
        param(
            Effects,
            "comp1.ratio",
            |_, d| log_uniform(d.u(), 2.0, 6.0),
            |c| {
                if c.safe {
                    Domain::Range(1.5, 10.0)
                } else {
                    Domain::Range(1.5, 20.0)
                }
            },
        ),
        param(
            Effects,
            "reverb1.high_cut",
            |_, d| log_uniform(d.u(), 3_000.0, 16_000.0),
            |_| Domain::Range(1_000.0, 20_000.0),
        )
        .extra(),
        param(
            Effects,
            "comp1.attack",
            |_, d| log_uniform(d.u(), 1.0, 30.0),
            |_| Domain::Range(0.1, 100.0),
        )
        .extra(),
        param(
            Effects,
            "comp1.release",
            |_, d| log_uniform(d.u(), 50.0, 500.0),
            |_| Domain::Range(10.0, 2_000.0),
        )
        .extra(),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_params::Scale;

    fn spec(unit: Unit, key: &'static str) -> ParamSpec {
        ParamSpec::float(key, "x", unit, 0.0, 20_000.0, Scale::Linear, 1.0).basic("x")
    }

    #[test]
    fn the_steps_design_9_names() {
        let cutoff = spec(Unit::Hz, "cutoff");
        assert_eq!(jnd(&cutoff), Jnd::Semitones(2.0));
        assert!(!exceeds_jnd(&cutoff, 2_000.0, 2_200.0)); // 1.65 st
        assert!(exceeds_jnd(&cutoff, 2_000.0, 2_250.0)); // 2.04 st
        assert!(exceeds_jnd(&cutoff, 2_000.0, 1_750.0));
        let attack = spec(Unit::Ms, "attack");
        assert!(!exceeds_jnd(&attack, 5.0, 7.0)); // (7 + 1) / (5 + 1) = 1.33
        assert!(exceeds_jnd(&attack, 5.0, 8.0)); // exactly 1.5 counts
        assert!(exceeds_jnd(&attack, 300.0, 190.0)); // 301 / 191 = 1.58
        let level = spec(Unit::Percent, "level");
        assert!(!exceeds_jnd(&level, 100.0, 80.0)); // −1.9 dB
        assert!(exceeds_jnd(&level, 100.0, 70.0)); // −3.1 dB
        assert_eq!(jnd(&spec(Unit::Percent, "resonance")), Jnd::Abs(10.0));
        assert!(ROUTE_JND.exceeded(0.0, 5.0) && !ROUTE_JND.exceeded(10.0, 5.1));
        assert!(Jnd::Any.exceeded(2.0, 3.0) && !Jnd::Any.exceeded(2.0, 2.0));
    }

    #[test]
    fn beyond_is_more_than_one_step_each_way() {
        for (j, from) in [
            (Jnd::Abs(3.0), -18.0),
            (Jnd::Semitones(2.0), 2_000.0),
            (
                Jnd::Ratio {
                    factor: 1.5,
                    floor: 1.0,
                },
                5.0,
            ),
            (
                Jnd::Db {
                    db: 3.0,
                    floor: 1.0,
                },
                60.0,
            ),
        ] {
            for dir in [1.0, -1.0] {
                assert!(j.exceeded(from, j.beyond(from, dir)), "{j:?} {from} {dir}");
            }
        }
    }
}
