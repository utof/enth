//! Answers: the target moved by `m` of `sound/1`'s calibration steps (`enth_challenge::calibrate::step`), the steps
//! whose one-step jitter set defines the per-challenge JND scale. One step is 10 % for Hz and times, 2 dB for levels,
//! 10 ct for pitch, 5 points for other percentages and 2.5 points of route depth; `m` steps compound a ratio
//! (`1.1^m`) and add up a difference (`m × 10 ct`). A move that would leave the parameter's range is clamped, and one
//! that cannot move at all from an edge goes the other way, as the jitter set does.

use enth_challenge::Phrase;
use enth_challenge::calibrate::{self, Step};
use enth_patch::addr::RouteAddr;
use enth_patch::{Op, ParamAddr, Patch};

use super::MoveKind;
use super::plan::{fnv1a64, mix};

/// Whether the move keyed `sign_seed` goes up for the parameter or route `key`.
fn up(sign_seed: u64, key: &str) -> bool {
    mix(sign_seed ^ fnv1a64(key.as_bytes())) & 1 == 1
}

/// `v` moved `m` steps up or down, before clamping.
fn steps(s: Step, v: f32, up: bool, m: f64) -> f64 {
    let v = f64::from(v);
    let ratio = |f: f64, floor: f64| {
        if up {
            (v + floor) * f - floor
        } else {
            (v + floor) / f - floor
        }
    };
    match s {
        Step::Ratio { factor, floor } => ratio(libm::pow(factor, m), floor),
        Step::Abs(d) => {
            if up {
                v + m * d
            } else {
                v - m * d
            }
        }
        Step::Db { db, floor } => ratio(libm::pow(10.0, m * db / 20.0), floor),
    }
}

/// `v` moved `m` steps in the drawn direction within `[lo, hi]`; the other way when the drawn one cannot move.
fn moved(s: Step, v: f32, up: bool, m: f64, lo: f32, hi: f32) -> f32 {
    let first = (steps(s, v, up, m) as f32).clamp(lo, hi);
    if first != v {
        return first;
    }
    (steps(s, v, !up, m) as f32).clamp(lo, hi)
}

/// The parameters and routes a move changes on `target` under `phrase`: for `all`, every observable continuous
/// parameter and route (the jitter set's); for `one` and `pitch`, the named parameter.
#[must_use]
pub fn moved_params(
    target: &Patch,
    phrase: &Phrase,
    kind: MoveKind,
    param: Option<&str>,
) -> (Vec<ParamAddr>, Vec<RouteAddr>) {
    match (kind, param) {
        (MoveKind::All, _) | (_, None) => calibrate::jittered_params(target, phrase),
        (_, Some(p)) => (p.parse::<ParamAddr>().into_iter().collect(), Vec::new()),
    }
}

/// The answer: `target` with the move's parameters (and routes) moved `m` calibration steps, each in the direction
/// keyed by `sign_seed` and its address. Deterministic.
#[must_use]
pub fn answer_patch(
    target: &Patch,
    phrase: &Phrase,
    kind: MoveKind,
    param: Option<&str>,
    sign_seed: u64,
    m: f64,
) -> Patch {
    let (params, routes) = moved_params(target, phrase, kind, param);
    let mut p = target.clone();
    for addr in &params {
        let (Some(spec), Some(v)) = (target.spec(addr), target.value(addr)) else {
            continue;
        };
        let Some(s) = calibrate::step(spec) else {
            continue;
        };
        let value = moved(
            s,
            v,
            up(sign_seed, &addr.to_string()),
            m,
            spec.min(),
            spec.max(),
        );
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
        let s = Step::Abs(f64::from(calibrate::ROUTE_STEP));
        let depth = moved(
            s,
            r.depth,
            up(sign_seed, &route.to_string()),
            m,
            -100.0,
            100.0,
        );
        let _ = enth_patch::apply(
            &mut p,
            Op::SetDepth {
                route: route.clone(),
                depth,
            },
        );
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_challenge::{SpecPreset, random};

    #[test]
    fn steps_compound_and_add() {
        let hz = Step::Ratio {
            factor: 1.1,
            floor: 0.0,
        };
        assert!((steps(hz, 1000.0, true, 2.0) - 1210.0).abs() < 1e-6);
        assert!((steps(hz, 1210.0, false, 2.0) - 1000.0).abs() < 1e-6);
        assert!((steps(Step::Abs(10.0), 0.0, false, 2.5) + 25.0).abs() < 1e-12);
        let db = Step::Db {
            db: 2.0,
            floor: 1.0,
        };
        let v = steps(db, 49.0, true, 3.0);
        assert!((20.0 * libm::log10((v + 1.0) / 50.0) - 6.0).abs() < 1e-9);
        // at an edge the move goes the other way; past an edge it is clamped
        assert_eq!(moved(Step::Abs(5.0), 100.0, true, 1.0, 0.0, 100.0), 95.0);
        assert_eq!(moved(Step::Abs(5.0), 98.0, true, 1.0, 0.0, 100.0), 100.0);
    }

    #[test]
    fn a_move_is_deterministic_and_grows_with_m() {
        let spec = SpecPreset::by_id(3).unwrap().spec(1, 77);
        let g = random::target(&spec).unwrap();
        let (params, _) = moved_params(&g.patch, g.phrase(), MoveKind::All, None);
        assert!(params.len() >= 5);
        let a = answer_patch(&g.patch, g.phrase(), MoveKind::All, None, 9, 2.0);
        assert_eq!(
            a,
            answer_patch(&g.patch, g.phrase(), MoveKind::All, None, 9, 2.0)
        );
        assert_ne!(a, g.patch);
        assert_eq!(
            answer_patch(&g.patch, g.phrase(), MoveKind::All, None, 9, 0.0),
            g.patch
        );
        let cutoff: ParamAddr = "filter1.cutoff".parse().unwrap();
        let t = g.patch.value(&cutoff).unwrap();
        let one = |m| {
            answer_patch(
                &g.patch,
                g.phrase(),
                MoveKind::One,
                Some("filter1.cutoff"),
                9,
                m,
            )
        };
        let (d1, d4) = (
            (one(1.0).value(&cutoff).unwrap() / t).ln().abs(),
            (one(4.0).value(&cutoff).unwrap() / t).ln().abs(),
        );
        assert!((d1 - 1.1f32.ln()).abs() < 1e-4 && (d4 - 4.0 * 1.1f32.ln()).abs() < 1e-3);
        // a single-parameter move changes only that parameter
        let p = one(4.0);
        for addr in &params {
            if *addr != cutoff {
                assert_eq!(p.value(addr), g.patch.value(addr), "{addr}");
            }
        }
    }
}
