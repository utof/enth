//! **Owner: lane DG.** The loss scale of layout 1: one just-noticeable step per entry, in the normalised domain, computed
//! in Rust so that `ml/` never re-implements `ParamSpec` (docs/r3.md, "Who computes what").
//!
//! Which step: the loss of docs/r3.md's propose v0 table counts continuous error "in JND units (25 ct; 10 % for times
//! and Hz; 2 dB)". Those are used for pitch (25 ct, so 0.25 of a semitone), times, frequencies and ratios (10 %), and
//! levels (2 dB, whether the parameter is in dB or a level in percent). The units that table does not name keep generator
//! v1's gameplay step (`enth_challenge::priors::jnd`): a percentage 10 points, an octave count 1/6, an angle 30 degrees.
//! A step is a distance in plain units, so its
//! width in the normalised domain depends on where the value sits (a time on a power scale, a cutoff on a log scale),
//! and the manifest carries both:
//!
//! - `table`: for each of the 33 continuous entries, the step's width in normalised units at 17 evenly spaced
//!   normalised values (0, 1/16, ..., 1), the width taken upward from the value (downward at the top of the range);
//!   a reader interpolates linearly;
//! - `scale`: one number per entry of the 102-wide row, the table's value at 0.5 (continuous), 0.025 for a route
//!   depth (one 5 % step of `priors::ROUTE_JND`, in `(depth / 100 + 1) / 2`), and 1.0 for categorical, structure and
//!   route-presence entries (a miss is a whole step).
#![allow(
    clippy::disallowed_methods,
    reason = "tooling: the sampler's arithmetic goes through libm; the file I/O is the caller's"
)]

use enth_params::{ParamSpec, Unit};
use enth_solver::propose::decode::OutputLayout;

/// Evenly spaced normalised values of a table row: `0, 1/16, ..., 1`.
pub const POINTS: usize = 17;

/// A route depth's step in the normalised depth `(depth / 100 + 1) / 2`: 5 % of the range's 200 %.
pub const ROUTE_DEPTH_STEP: f32 = 0.025;

/// One step of a loss scale, in plain units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Any change of a discrete value counts as a whole step.
    Whole,
    /// A difference in plain units.
    Abs(f32),
    /// A ratio of `(v + floor)`.
    Ratio {
        /// The ratio.
        factor: f32,
        /// Added to both values first.
        floor: f32,
    },
    /// A level difference in dB between two linear percentages, `(v + floor)`.
    Db {
        /// The difference.
        db: f32,
        /// Added to both values first.
        floor: f32,
    },
}

/// The step of a continuous parameter (see the module docs).
pub fn step_of(spec: &ParamSpec) -> Step {
    match spec.unit {
        Unit::Semitones => Step::Abs(0.25),
        Unit::Cents => Step::Abs(25.0),
        Unit::Hz | Unit::Ratio => Step::Ratio {
            factor: 1.10,
            floor: 0.0,
        },
        Unit::Ms => Step::Ratio {
            factor: 1.10,
            floor: 1.0,
        },
        Unit::Db => Step::Abs(2.0),
        Unit::Percent if spec.key == "level" => Step::Db {
            db: 2.0,
            floor: 1.0,
        },
        Unit::Percent => Step::Abs(10.0),
        Unit::Octaves => Step::Abs(1.0 / 6.0),
        Unit::Degrees => Step::Abs(30.0),
        // `Unit::None` (a discrete value) and any unit a later catalogue adds: any change is a whole step.
        _ => Step::Whole,
    }
}

/// The value one `step` above (`dir` = 1) or below (`dir` = -1) `v`, in plain units; `None` when the step cannot be
/// taken from `v` (a ratio of a non-positive number).
fn moved(step: Step, v: f64, dir: f64) -> Option<f64> {
    match step {
        Step::Whole => None,
        Step::Abs(s) => Some(v + dir * f64::from(s)),
        Step::Ratio { factor, floor } => {
            let f = f64::from(floor);
            (v + f > 0.0).then(|| (v + f) * enth_dsp::util::math::pow(f64::from(factor), dir) - f)
        }
        Step::Db { db, floor } => {
            let f = f64::from(floor);
            (v + f > 0.0)
                .then(|| (v + f) * enth_dsp::util::math::pow(10.0, dir * f64::from(db) / 20.0) - f)
        }
    }
}

/// The width in normalised units of one `step` from the normalised value `n`: taken upward, or downward when the
/// range ends first. A whole step (a discrete value) is 1.
pub fn width_at(spec: &ParamSpec, step: Step, n: f32) -> f32 {
    if matches!(step, Step::Whole) {
        return 1.0;
    }
    let (min, max) = (f64::from(spec.min()), f64::from(spec.max()));
    let v = f64::from(spec.from_norm(n));
    let n = f64::from(n);
    let mut best = 0.0f64;
    for dir in [1.0, -1.0] {
        if let Some(w) = moved(step, v, dir) {
            let clipped = w.clamp(min, max);
            let reach = (f64::from(spec.to_norm(clipped as f32)) - n).abs();
            // The first direction that fits entirely inside the range wins; at an end, the longer reach.
            if (w >= min && w <= max) || reach > best {
                best = reach;
                if w >= min && w <= max {
                    break;
                }
            }
        }
    }
    best as f32
}

/// The scale of layout 1: see the module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct JndScale {
    /// One number per entry of the row (`layout.width()`).
    pub scale: Vec<f32>,
    /// The continuous entries' widths at [`POINTS`] normalised values, in layout order.
    pub table: Vec<[f32; POINTS]>,
}

impl JndScale {
    /// The scale of `layout`.
    pub fn build(layout: &OutputLayout) -> JndScale {
        let mut scale = vec![1.0f32; layout.width()];
        let mut table = Vec::with_capacity(layout.continuous().len());
        for e in layout.continuous() {
            let step = step_of(e.spec);
            let mut row = [0.0f32; POINTS];
            for (i, w) in row.iter_mut().enumerate() {
                *w = width_at(e.spec, step, i as f32 / (POINTS - 1) as f32);
            }
            scale[e.offset] = row[POINTS / 2];
            table.push(row);
        }
        for c in layout.routes() {
            scale[c.depth] = ROUTE_DEPTH_STEP;
        }
        JndScale { scale, table }
    }

    /// The step's width for continuous entry `i` (layout order) at the normalised value `n` (linear between points).
    pub fn at(&self, i: usize, n: f32) -> f32 {
        let x = n.clamp(0.0, 1.0) * (POINTS - 1) as f32;
        let lo = (x.floor() as usize).min(POINTS - 2);
        let t = x - lo as f32;
        let row = &self.table[i];
        row[lo] + (row[lo + 1] - row[lo]) * t
    }
}

/// Whether `spec` is a continuous entry (a `Float`): the ones that have a table.
#[cfg(test)]
fn is_continuous(spec: &ParamSpec) -> bool {
    matches!(spec.kind, enth_params::Kind::Float { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_has_one_number_per_entry_and_a_row_per_continuous_value() {
        let layout = OutputLayout::v1();
        let s = JndScale::build(&layout);
        assert_eq!(s.scale.len(), 102);
        assert_eq!(s.table.len(), 33);
        assert!(
            s.scale
                .iter()
                .all(|w| w.is_finite() && *w > 0.0 && *w <= 1.0)
        );
        for e in layout.continuous() {
            assert!(is_continuous(e.spec), "{}", e.addr);
        }
    }

    #[test]
    fn a_step_is_the_distance_the_units_say() {
        let layout = OutputLayout::v1();
        let s = JndScale::build(&layout);
        let by_name = |name: &str| {
            layout
                .continuous()
                .iter()
                .position(|e| e.addr.to_string() == name)
                .unwrap()
        };
        // osc1.semi: 0.25 semitone of a 96-semitone linear range.
        let i = by_name("osc1.semi");
        assert!(
            (s.at(i, 0.5) - 0.25 / 96.0).abs() < 1e-6,
            "{}",
            s.at(i, 0.5)
        );
        // osc2.fine: 25 cents of 200.
        let i = by_name("osc2.fine");
        assert!((s.at(i, 0.5) - 25.0 / 200.0).abs() < 1e-6);
        // filter1.cutoff: 10 % on a log scale is the same width everywhere.
        let i = by_name("filter1.cutoff");
        let w = f64::from(s.at(i, 0.5));
        let want = enth_dsp::util::math::log(1.1) / enth_dsp::util::math::log(1000.0);
        assert!((w - want).abs() < 1e-4, "{w} vs {want}");
        assert!((f64::from(s.at(i, 0.2)) - want).abs() < 1e-4);
        // comp1.threshold: 2 dB of 60.
        let i = by_name("comp1.threshold");
        assert!((s.at(i, 0.5) - 2.0 / 60.0).abs() < 1e-6);
        // A time on a power scale gets narrower towards 0.
        let i = by_name("env1.attack");
        assert!(s.at(i, 0.125) < s.at(i, 0.75));
    }

    #[test]
    fn a_step_at_the_top_of_the_range_is_taken_downward() {
        let layout = OutputLayout::v1();
        let s = JndScale::build(&layout);
        for (i, e) in layout.continuous().iter().enumerate() {
            let top = s.at(i, 1.0);
            assert!(top > 0.0 && top <= 1.0, "{}: {top}", e.addr);
        }
    }

    #[test]
    fn structure_presence_and_categoricals_are_whole_steps_and_depths_are_5_percent() {
        let layout = OutputLayout::v1();
        let s = JndScale::build(&layout);
        for e in layout.categorical() {
            assert!(
                s.scale[e.offset..e.offset + e.len()]
                    .iter()
                    .all(|w| *w == 1.0)
            );
        }
        for e in layout.enables() {
            assert_eq!(s.scale[e.offset], 1.0);
        }
        for c in layout.routes() {
            assert_eq!(s.scale[c.presence], 1.0);
            assert_eq!(s.scale[c.depth].to_bits(), ROUTE_DEPTH_STEP.to_bits());
        }
    }
}
