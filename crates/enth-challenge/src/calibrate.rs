//! **Owner: lane SS** (the sound scorer).
//!
//! Sound-score calibration (report 02 §2.4, ADR-0007's 2026-10-20 note, fix 4): the distance in per-challenge
//! just-noticeable units, and the curve that maps it to sound %. One global pass threshold would be unfair: an octave
//! of cutoff is 0.05 under metric v1 on a pluck and 0.46–0.54 on a sustained sound.
//!
//! **The JND scale.** Per challenge, [`jitter_set`] makes [`JITTERS`] copies of the target with **every observable,
//! continuous parameter and route depth** (lane RG's mask, [`crate::random::observable`]) moved by one calibration
//! step ([`step`]) up or down. The signs are drawn per parameter and copy from [`JITTER_SEED`] (keyed by address, like
//! every draw in this crate), so the set is the same on every machine. The scorer renders each and takes the median
//! distance to the target as the scale `s` (at least [`SCALE_MIN`]); then `z = max(0, d − floor) / s`.
//!
//! **The steps** are ADR-0007's "check's defaults to start" where they are a just-noticeable step (10 % for Hz and
//! times, 2 dB for dB values and levels), and these where they are not: cents and semitones 10 ct (`check`'s 25 ct
//! is a pass band for an absolute pitch, not a step; the pitch difference limen near C3 is of the order of 10 ct, and
//! a detune moved by 25 ct changes the beat rate many times over), other percentages 5 points and route depths
//! 2.5 points (half of the generator's minimum step, as 10 % is about half of its two semitones for Hz), ratios and
//! unitless values 10 %, degrees 10°.
//!
//! **The curve** ([`Curve`]): `sound % = 100 / (1 + (z / z50)^k)`. `sound/1`'s parameters are fitted to the owner's
//! metric session 1 (report 02 §2.2) so that its **rating-4.5 point lands on the pass mark, 90** (docs/challenge.md,
//! "the pass mark stays and the curve moves"), and its rating-3 point on 50 %; see [`Curve::SOUND_V1`] for the fit
//! and docs/challenge.md, "Sound score (lane SS)", for how it was bridged from the toy synth's distances to JND
//! units. They are **provisional**: metric session 2 (test–retest) and a real-engine pairs session refit them, and
//! GR1's blind ratings refit them again as `sound/2`, a new id beside `sound/1`, so a stored `sound/1` score keeps its
//! meaning.

use enth_dsp::util::rng::{derive, fnv1a64};
use enth_params::{ParamSpec, Unit};
use enth_patch::addr::RouteAddr;
use enth_patch::{Op, ParamAddr, Patch};

use crate::model::{Phrase, SCORING_SEED};
use crate::random;

/// How many jittered copies of the target set the scale (report 02: "about 16").
pub const JITTERS: usize = 16;

/// The root of the jitter signs: `derive(SCORING_SEED, fnv1a64("sound/1 jnd"))`. Each sign is
/// `derive(derive(JITTER_SEED, fnv1a64(address)), copy) & 1`.
pub const JITTER_SEED: u64 = derive(SCORING_SEED, fnv1a64(b"sound/1 jnd"));

/// The smallest scale: below it a target barely moves for any one-step jitter (a click, a sound under its gates),
/// and z would blow up over nothing. Metric v1.1 units; under the median scale of every level measured.
pub const SCALE_MIN: f64 = 0.01;

/// A route depth's calibration step, in points of depth (−100..=100).
pub const ROUTE_STEP: f32 = 2.5;

/// One calibration step of a parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// A ratio, applied to `(v + floor)`.
    Ratio {
        /// The factor (1.1 for 10 %).
        factor: f64,
        /// Added before and taken off after (keeps 0 ms moving).
        floor: f64,
    },
    /// An absolute difference in plain units.
    Abs(f64),
    /// A level step in dB on a linear percentage, applied to `(v + floor)`.
    Db {
        /// The step.
        db: f64,
        /// Added before and taken off after (keeps 0 % moving).
        floor: f64,
    },
}

impl Step {
    /// The value one step from `v`, up (`up`) or down, before clamping to the parameter's range.
    pub fn apply(self, v: f32, up: bool) -> f32 {
        let v = f64::from(v);
        let out = match self {
            Step::Ratio { factor, floor } => {
                if up {
                    (v + floor) * factor - floor
                } else {
                    (v + floor) / factor - floor
                }
            }
            Step::Abs(d) => {
                if up {
                    v + d
                } else {
                    v - d
                }
            }
            Step::Db { db, floor } => {
                let f = libm::pow(10.0, db / 20.0);
                if up {
                    (v + floor) * f - floor
                } else {
                    (v + floor) / f - floor
                }
            }
        };
        out as f32
    }
}

/// The calibration step of a parameter (see the [module docs](self)); `None` for a discrete one (an enum, an integer,
/// a toggle), which a step cannot move.
pub fn step(spec: &ParamSpec) -> Option<Step> {
    if spec.is_discrete() {
        return None;
    }
    Some(match spec.unit {
        Unit::Ms => Step::Ratio {
            factor: 1.1,
            floor: 1.0,
        },
        Unit::Db => Step::Abs(2.0),
        // 10 ct, and 10° for a phase.
        Unit::Cents | Unit::Degrees => Step::Abs(10.0),
        Unit::Semitones => Step::Abs(0.1),
        Unit::Octaves => Step::Abs(10.0 / 1200.0),
        Unit::Percent if spec.key == "level" || spec.key == "sustain" => Step::Db {
            db: 2.0,
            floor: 1.0,
        },
        Unit::Percent => Step::Abs(5.0),
        // 10 %: Hz, ratios (`comp.ratio`), unitless values (reverb detune and damping) and any unit added later.
        _ => Step::Ratio {
            factor: 1.1,
            floor: 0.0,
        },
    })
}

/// Whether copy `copy` moves the parameter or route keyed `key` up.
fn up(key: &str, copy: usize) -> bool {
    derive(derive(JITTER_SEED, fnv1a64(key.as_bytes())), copy as u64) & 1 == 1
}

/// The value one step from `v` in the drawn direction, within `[lo, hi]`; the other direction when the drawn one
/// hits the range's edge.
fn jittered(v: f32, s: Step, up: bool, lo: f32, hi: f32) -> f32 {
    let first = s.apply(v, up).clamp(lo, hi);
    if first != v {
        return first;
    }
    s.apply(v, !up).clamp(lo, hi)
}

/// The parameters and routes [`jitter_set`] moves for `target` under `phrase`: every observable continuous parameter
/// and every observable route, in address order.
pub fn jittered_params(target: &Patch, phrase: &Phrase) -> (Vec<ParamAddr>, Vec<RouteAddr>) {
    let mask = random::observable(target, phrase);
    let params = mask
        .observable_params()
        .filter(|a| target.spec(a).and_then(step).is_some())
        .cloned()
        .collect();
    let routes = mask
        .routes()
        .filter(|(_, v)| v.is_observable())
        .map(|(r, _)| r.clone())
        .collect();
    (params, routes)
}

/// The [`JITTERS`] jittered copies of `target` (see the [module docs](self)). Deterministic: the same patches on every
/// machine. A copy that cannot take a move (it would not apply) keeps that parameter's value.
pub fn jitter_set(target: &Patch, phrase: &Phrase) -> Vec<Patch> {
    let (params, routes) = jittered_params(target, phrase);
    (0..JITTERS)
        .map(|copy| {
            let mut p = target.clone();
            for addr in &params {
                let (Some(spec), Some(v)) = (target.spec(addr), target.value(addr)) else {
                    continue;
                };
                let Some(s) = step(spec) else { continue };
                let value = jittered(v, s, up(&addr.to_string(), copy), spec.min(), spec.max());
                let _ = enth_patch::apply(
                    &mut p,
                    Op::SetParam {
                        addr: addr.clone(),
                        value,
                    },
                );
            }
            for route in &routes {
                let Some(r) = target
                    .routes
                    .iter()
                    .find(|r| r.src == route.src && r.dst == route.dst)
                else {
                    continue;
                };
                let s = Step::Abs(f64::from(ROUTE_STEP));
                let depth = jittered(r.depth, s, up(&route.to_string(), copy), -100.0, 100.0);
                let _ = enth_patch::apply(
                    &mut p,
                    Op::SetDepth {
                        route: route.clone(),
                        depth,
                    },
                );
            }
            p
        })
        .collect()
}

/// The JND scale from the jittered copies' distances to the target: their median (the mean of the middle two for an
/// even count, in sorted order), at least [`SCALE_MIN`]. Non-finite distances count as the largest.
pub fn scale(distances: &[f64]) -> f64 {
    let mut d: Vec<f64> = distances
        .iter()
        .map(|v| if v.is_finite() { *v } else { f64::MAX })
        .collect();
    if d.is_empty() {
        return SCALE_MIN;
    }
    d.sort_by(f64::total_cmp);
    let n = d.len();
    let median = if n % 2 == 1 {
        d[n / 2]
    } else {
        d[n / 2 - 1] / 2.0 + d[n / 2] / 2.0
    };
    median.max(SCALE_MIN)
}

/// The distance in JND units: `max(0, distance − floor) / scale`.
pub fn jnd(distance: f64, floor: f64, scale: f64) -> f64 {
    let d = (distance - floor).max(0.0);
    if scale > 0.0 {
        d / scale
    } else {
        f64::INFINITY
    }
}

/// The curve from JND units to sound %: `100 / (1 + (z / z50)^k)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    /// The JND distance that scores 50 %.
    pub z50: f64,
    /// The steepness.
    pub k: f64,
}

impl Curve {
    /// `sound/1`'s curve (**provisional**). From report 02 §2.2: on the owner's session 1, the logistic fit of rating
    /// on metric-v1 distance A puts rating 4.5 at A = 0.26 ([`RATING_4_5_A`]) and rating 3 at A = 0.89
    /// ([`RATING_3_A`]). Those are toy-synth distances under metric v1; lane SS's bridge (docs/challenge.md, "Sound
    /// score (lane SS)") re-measured the toy synth with these steps and metric v1.1: the median v1.1 distance of
    /// toy pairs at A = 0.26 is [`TOY_V1_1_AT_RATING_4_5`], at A = 0.89 [`TOY_V1_1_AT_RATING_3`], and the toy
    /// targets' median v1.1 JND scale is [`TOY_SCALE_V1_1`]. So rating 4.5 is z = 0.1611 / 0.1629 = 0.99 and rating
    /// 3 is z = 0.8325 / 0.1629 = 5.11. The curve puts the first on the pass mark, 90 %, and the second on 50 %:
    /// `z50 = 5.11` and `k = ln 9 / ln(0.8325 / 0.1611) = 1.338`.
    pub const SOUND_V1: Curve = Curve {
        z50: TOY_V1_1_AT_RATING_3 / TOY_SCALE_V1_1,
        k: 1.337_806_847_474_152,
    };

    /// Sound % at JND distance `z`: 100 at 0, falling to 0; 0 for a NaN.
    pub fn percent(&self, z: f64) -> f64 {
        if z.is_nan() {
            return 0.0;
        }
        if z <= 0.0 {
            return 100.0;
        }
        let p = 100.0 / (1.0 + libm::pow(z / self.z50, self.k));
        if p.is_finite() {
            p.clamp(0.0, 100.0)
        } else {
            0.0
        }
    }

    /// The JND distance that scores `percent` (0 < percent < 100): `z50 · (100 / percent − 1)^(1/k)`.
    pub fn z_at(&self, percent: f64) -> f64 {
        self.z50 * libm::pow(100.0 / percent - 1.0, 1.0 / self.k)
    }
}

/// Session 1's rating-4.5 point on metric v1 (toy synth; report 02 §2.2).
pub const RATING_4_5_A: f64 = 0.26;
/// Session 1's rating-3 point on metric v1 (toy synth; report 02 §2.2).
pub const RATING_3_A: f64 = 0.89;
/// The median metric-v1.1 distance of toy-synth pairs whose metric-v1 distance is within 15 % of
/// [`RATING_4_5_A`] (148 pairs of the listening design's kind on 40 toy targets; lane SS's bridge).
pub const TOY_V1_1_AT_RATING_4_5: f64 = 0.1611;
/// The same within 15 % of [`RATING_3_A`] (89 pairs).
pub const TOY_V1_1_AT_RATING_3: f64 = 0.8325;
/// The median metric-v1.1 JND scale of the 40 toy targets (the spike's seeded list) under these steps. Under metric
/// v1 it is 0.2832, 2.4 times the spike's T1 (0.1189 re-measured, 0.123 in the spike).
pub const TOY_SCALE_V1_1: f64 = 0.1629;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DEFAULT_PASS_MARK;

    #[test]
    fn the_curve_puts_rating_4_5_on_the_pass_mark_and_rating_3_on_50() {
        let c = Curve::SOUND_V1;
        let k = libm::log(9.0) / libm::log(TOY_V1_1_AT_RATING_3 / TOY_V1_1_AT_RATING_4_5);
        assert!((c.k - k).abs() < 1e-12, "{} {k}", c.k);
        let z_pass = TOY_V1_1_AT_RATING_4_5 / TOY_SCALE_V1_1;
        let pass = c.percent(z_pass);
        assert!((pass - f64::from(DEFAULT_PASS_MARK)).abs() < 1e-9, "{pass}");
        assert!((c.percent(TOY_V1_1_AT_RATING_3 / TOY_SCALE_V1_1) - 50.0).abs() < 1e-9);
        assert_eq!(c.percent(0.0), 100.0);
        assert_eq!(c.percent(-1.0), 100.0);
        assert_eq!(c.percent(f64::INFINITY), 0.0);
        assert_eq!(c.percent(f64::NAN), 0.0);
        assert!((c.z_at(90.0) - z_pass).abs() < 1e-9);
        assert!((z_pass - 0.989).abs() < 1e-3 && (c.z50 - 5.110).abs() < 1e-3);
        let mut last = 100.0;
        for i in 1..200 {
            let p = c.percent(f64::from(i) * 0.1);
            assert!(p < last && p > 0.0, "strictly falling");
            last = p;
        }
    }

    #[test]
    fn scale_is_a_floored_median_and_jnd_subtracts_the_floor() {
        assert!((scale(&[]) - SCALE_MIN).abs() < 1e-15);
        assert_eq!(scale(&[0.3, 0.1, 0.2]), 0.2);
        assert_eq!(scale(&[0.4, 0.1, 0.2, 0.3]), 0.25);
        assert!((scale(&[0.0, 0.001]) - SCALE_MIN).abs() < 1e-15);
        assert_eq!(scale(&[0.2, f64::NAN, 0.1]), 0.2);
        assert_eq!(jnd(0.05, 0.1, 0.2), 0.0);
        assert!((jnd(0.5, 0.1, 0.2) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn steps_follow_the_units() {
        let ms = Step::Ratio {
            factor: 1.1,
            floor: 1.0,
        };
        assert!((ms.apply(9.0, true) - 10.0).abs() < 1e-5);
        assert!((ms.apply(10.0, false) - 9.0).abs() < 1e-5);
        assert!(ms.apply(0.0, true) > 0.0);
        let db = Step::Db {
            db: 2.0,
            floor: 1.0,
        };
        let up = db.apply(99.0, true);
        assert!((20.0 * libm::log10(f64::from(up + 1.0) / 100.0) - 2.0).abs() < 1e-4);
        assert_eq!(Step::Abs(5.0).apply(50.0, false), 45.0);
        // at the edge the move goes the other way
        assert_eq!(jittered(100.0, Step::Abs(5.0), true, 0.0, 100.0), 95.0);
        assert_eq!(jittered(0.0, Step::Abs(5.0), false, 0.0, 100.0), 5.0);
    }

    #[test]
    fn the_jitter_set_is_deterministic_and_moves_observable_parameters_by_one_step() {
        let spec = crate::spec::SpecPreset::by_id(3).unwrap().spec(1, 11);
        let g = random::target(&spec).unwrap();
        let (params, _) = jittered_params(&g.patch, g.phrase());
        assert!(params.len() >= 5, "{params:?}");
        let a = jitter_set(&g.patch, g.phrase());
        let b = jitter_set(&g.patch, g.phrase());
        assert_eq!(a, b);
        assert_eq!(a.len(), JITTERS);
        let mask = random::observable(&g.patch, g.phrase());
        for p in &a {
            for (addr, _) in mask.params() {
                let (t, v) = (g.patch.value(addr).unwrap(), p.value(addr).unwrap());
                if params.contains(addr) {
                    let s = step(g.patch.spec(addr).unwrap()).unwrap();
                    let (lo, hi) = (s.apply(t, false), s.apply(t, true));
                    assert!(v == lo || v == hi || v != t, "{addr}: {t} -> {v}");
                } else {
                    assert_eq!(t, v, "{addr} is not jittered");
                }
            }
        }
        // both directions occur
        let addr = &params[0];
        let ups = a
            .iter()
            .filter(|p| p.value(addr).unwrap() > g.patch.value(addr).unwrap())
            .count();
        assert!(ups > 0 && ups < JITTERS, "{ups}");
    }
}
