//! Measured hints for failed expectations.
//!
//! "Measured" means rendered, not guessed. For a failed expectation:
//!
//! 1. [`probe`] renders the patch with each continuous parameter nudged by 10 % of its range (and with each other
//!    waveform), 11 renders in all, and keeps the descriptors of each. That is the finite-difference sensitivity.
//! 2. The parameters are ranked by how little they need to move to reach the aim (a log scale for Hz and ms
//!    descriptors, linear otherwise), using those slopes.
//! 3. The best one or two are refined with up to three real renders each (secant steps), rounded to the 4 digits
//!    the hint will show, so `expected` is what that single change measures, not a prediction.
//!
//! One parameter at a time: a hint can say `reaches: false` and name others that push the same way. The agent
//! decides how to combine them.

use crate::descriptors::{Describer, Descriptors, Want};
use crate::error::Res;
use crate::expect::{Expect, Feature, Op};
use crate::patch::{CONT, Patch, Wave, note_text};
use crate::synth::measure;
use crate::units::{fmt_value, r4};
use serde_json::{Value, json};

/// Step used for the finite differences, as a fraction of each parameter's range.
const STEP_U: f64 = 0.1;
/// Renders allowed per refined candidate.
const REFINE_RENDERS: usize = 4;
/// Candidates refined per hint.
const REFINE_CANDIDATES: usize = 2;

/// The patch and its neighbours: one nudged copy per continuous parameter, one copy per other waveform.
pub struct Probes {
    pub base: Patch,
    pub base_d: Descriptors,
    /// `(parameter index, u0, u1, descriptors at u1)`
    pub cont: Vec<(usize, f64, f64, Descriptors)>,
    pub waves: Vec<(Wave, Descriptors)>,
}

/// Render the 11 neighbours and measure the descriptor groups in `want`.
pub fn probe(
    base: &Patch,
    base_d: &Descriptors,
    describer: &mut Describer,
    want: Want,
) -> Res<Probes> {
    let mut p = probe_step(base, base_d, describer, want, STEP_U)?;
    p.waves = probe_waves(base, describer, want)?;
    Ok(p)
}

fn probe_waves(
    base: &Patch,
    describer: &mut Describer,
    want: Want,
) -> Res<Vec<(Wave, Descriptors)>> {
    let mut waves = Vec::new();
    for w in Wave::ALL {
        if w != base.wave {
            let mut p = base.clone();
            p.wave = w;
            waves.push((w, measure(&p, describer, want)?));
        }
    }
    Ok(waves)
}

/// The continuous nudges only, with a chosen step (a bigger step finds descriptors that are absent at the base).
fn probe_step(
    base: &Patch,
    base_d: &Descriptors,
    describer: &mut Describer,
    want: Want,
    step_u: f64,
) -> Res<Probes> {
    let mut cont = Vec::with_capacity(CONT.len());
    for (i, spec) in CONT.iter().enumerate() {
        let u0 = spec.to_u(base.v[i]);
        let step = if u0 <= 1.0 - step_u { step_u } else { -step_u };
        let nudged = base.with_u(i, u0 + step);
        let u1 = spec.to_u(nudged.v[i]);
        if (u1 - u0).abs() < 1e-6 {
            continue;
        }
        let d = measure(&nudged, describer, want)?;
        cont.push((i, u0, u1, d));
    }
    Ok(Probes {
        base: base.clone(),
        base_d: base_d.clone(),
        cont,
        waves: Vec::new(),
    })
}

/// The expectation's value as the search sees it: logarithm for scale features, `None` where it cannot be taken.
fn search_value(e: &Expect, d: &Descriptors) -> Option<f64> {
    let c = e.compared(d)?;
    if e.feature.log() {
        (c > 0.0).then(|| c.ln())
    } else {
        Some(c)
    }
}

fn search_aim(e: &Expect) -> Option<f64> {
    let a = e.aim();
    if e.feature.log() {
        (a > 0.0).then(|| a.ln())
    } else {
        Some(a)
    }
}

struct Candidate {
    param: usize,
    u0: f64,
    u1: f64,
    /// Linear-model position for the aim.
    u_target: f64,
    /// Distance on the u axis to get there.
    cost: f64,
    /// The model says the aim is inside the parameter's range.
    in_range: bool,
    /// Which way the parameter must move: +1 raise, -1 lower.
    dir: i32,
}

fn candidates(e: &Expect, probes: &Probes) -> Vec<Candidate> {
    let (Some(aim), base_g) = (search_aim(e), search_value(e, &probes.base_d)) else {
        return Vec::new();
    };
    let floor = e.feature.noise_floor();
    let mut out = Vec::new();
    for (i, u0, u1, d1) in &probes.cont {
        let g1 = search_value(e, d1);
        match (base_g, g1) {
            (Some(g0), Some(g1)) => {
                if (g1 - g0).abs() < floor {
                    continue;
                }
                let slope = (g1 - g0) / (u1 - u0);
                let du = (aim - g0) / slope;
                let u_target = u0 + du;
                out.push(Candidate {
                    param: *i,
                    u0: *u0,
                    u1: *u1,
                    u_target: u_target.clamp(0.0, 1.0),
                    cost: du.abs(),
                    in_range: (-1e-9..=1.0 + 1e-9).contains(&u_target),
                    dir: if du >= 0.0 { 1 } else { -1 },
                });
            }
            (None, Some(_)) => {
                // The descriptor only exists after the nudge: keep going the same way.
                let dir = if u1 > u0 { 1 } else { -1 };
                out.push(Candidate {
                    param: *i,
                    u0: *u0,
                    u1: *u1,
                    u_target: (u1 + 2.0 * (u1 - u0)).clamp(0.0, 1.0),
                    cost: 0.5,
                    in_range: true,
                    dir,
                });
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| b.in_range.cmp(&a.in_range).then(a.cost.total_cmp(&b.cost)));
    out
}

/// A rendered point on one parameter's axis.
struct Point {
    u: f64,
    pass: bool,
    /// Distance outside the accepted interval (0 when it passes).
    miss: f64,
    /// Distance from the aim, in the search domain.
    off_aim: f64,
    value: f64,
    d: Descriptors,
}

struct Refined {
    best: Point,
}

/// Next position to try: regula falsi when points on both sides of the aim are known, otherwise a secant through
/// the two points nearest the aim, otherwise a step the way the model pointed.
fn next_u(known: &[(f64, f64)], aim: f64, dir: i32) -> Option<f64> {
    let below = known
        .iter()
        .filter(|p| p.1 <= aim)
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let above = known
        .iter()
        .filter(|p| p.1 >= aim)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let (Some(a), Some(b)) = (below, above)
        && (a.0 - b.0).abs() > 1e-4
        && (b.1 - a.1).abs() > 1e-12
    {
        return Some((a.0 + (aim - a.1) / (b.1 - a.1) * (b.0 - a.0)).clamp(0.0, 1.0));
    }
    let mut near = known.to_vec();
    near.sort_by(|a, b| (a.1 - aim).abs().total_cmp(&(b.1 - aim).abs()));
    let &(ua, ga) = near.first()?;
    if let Some(&(ub, gb)) = near.iter().skip(1).find(|p| (p.0 - ua).abs() > 1e-4) {
        let slope = (gb - ga) / (ub - ua);
        (slope.abs() > 1e-9).then(|| (ua + (aim - ga) / slope).clamp(0.0, 1.0))
    } else {
        Some((ua + f64::from(dir) * 2.0 * STEP_U).clamp(0.0, 1.0))
    }
}

/// Search along one parameter with up to [`REFINE_RENDERS`] real renders, starting at the model's guess.
fn refine(
    e: &Expect,
    probes: &Probes,
    cand: &Candidate,
    describer: &mut Describer,
    want: Want,
) -> Res<Refined> {
    let spec = &CONT[cand.param];
    let aim = search_aim(e).unwrap_or(0.0);
    let mut known: Vec<(f64, f64)> = Vec::new();
    if let Some(g) = search_value(e, &probes.base_d) {
        known.push((cand.u0, g));
    }
    if let Some((_, _, _, d1)) = probes.cont.iter().find(|c| c.0 == cand.param)
        && let Some(g) = search_value(e, d1)
    {
        known.push((cand.u1, g));
    }
    let mut points: Vec<Point> = Vec::new();
    let mut u_next = cand.u_target;
    for _ in 0..REFINE_RENDERS {
        let patch = probes.base.with_u(cand.param, u_next);
        let value = patch.v[cand.param];
        let u_actual = spec.to_u(value);
        if points.iter().any(|p| (p.u - u_actual).abs() < 1e-4) {
            break;
        }
        let d = measure(&patch, describer, want)?;
        let ev = e.eval(&d);
        let g = search_value(e, &d);
        if let Some(g) = g {
            known.push((u_actual, g));
        }
        let off_aim = g.map_or(f64::INFINITY, |g| (g - aim).abs());
        points.push(Point {
            u: u_actual,
            pass: ev.pass,
            miss: e.shortfall(&d),
            off_aim,
            value,
            d,
        });
        // Stop at a pass that is not a big overshoot past the aim (the mildest change that works is the better hint).
        let start_gap = known
            .first()
            .map_or(f64::INFINITY, |(_, g0)| (g0 - aim).abs());
        if ev.pass && (off_aim <= 0.3 * start_gap || !start_gap.is_finite()) {
            break;
        }
        match next_u(&known, aim, cand.dir) {
            Some(n) => u_next = n,
            None => break,
        }
    }
    let best = points
        .into_iter()
        .min_by(|a, b| {
            b.pass
                .cmp(&a.pass)
                .then(a.miss.total_cmp(&b.miss))
                .then(a.off_aim.total_cmp(&b.off_aim))
        })
        .expect("at least one render was attempted");
    Ok(Refined { best })
}

fn expected_text(e: &Expect, d: &Descriptors) -> String {
    match e.compared(d) {
        None => format!("{} could not be measured", e.feature.alias()),
        Some(c) if e.in_cents() => format!("pitch error {}ct", r4(c)),
        Some(c) => format!("{} {}", e.feature.alias(), fmt_value(c, e.feature.unit())),
    }
}

fn dir_word(dir: i32) -> &'static str {
    if dir > 0 { "raise" } else { "lower" }
}

fn text_hint(text: impl Into<String>, measured: bool) -> Value {
    json!({"text": text.into(), "measured": measured})
}

/// A hint for one failed expectation.
pub fn hint_for(
    e: &Expect,
    got: &Descriptors,
    probes: &Probes,
    describer: &mut Describer,
    want: Want,
) -> Res<Value> {
    let base = &probes.base;
    // The clip ended before the measurement could finish.
    if e.feature.truncated(got) && !matches!(e.op, Op::Gt | Op::Ge) {
        let longer = r4(base.perf.dur_s + base.perf.dur_s.max(2.0));
        return Ok(json!({
            "param": "perf.dur", "now": base.show(crate::patch::Key::Dur), "try": format!("{longer}s"),
            "text": format!("the clip ends before the {} could be measured; render longer", e.feature.alias()),
            "measured": false,
        }));
    }
    // Pitch comes from the performance, not from the sound design, unless it is only a few cents off.
    if e.feature == Feature::Pitch {
        let far = e
            .compared(got)
            .is_none_or(|c| !e.in_cents() || c.abs() > 100.0);
        if far {
            let target = if e.in_cents() {
                e.value
            } else {
                base.perf.note_hz
            };
            if e.in_cents() {
                return Ok(json!({
                    "param": "perf.note", "now": note_text(base.perf.note_hz), "try": note_text(target),
                    "text": "pitch is the played note, set with perf.note (a note name or Hz); the sound design only detunes it slightly",
                    "measured": false,
                }));
            }
            return Ok(text_hint(
                "pitch is the played note: change perf.note (a note name or Hz) to move it",
                false,
            ));
        }
    }
    let mut cands = candidates(e, probes);
    let mut wide;
    let mut probes = probes;
    if cands.is_empty() && search_value(e, &probes.base_d).is_none() {
        // The descriptor is absent at the base and after every small nudge: look further out.
        wide = probe_step(&probes.base, &probes.base_d, describer, want, 0.5)?;
        wide.waves.clone_from(&probes.waves);
        probes = &wide;
        cands = candidates(e, probes);
    }
    let mut chosen: Option<(usize, Refined)> = None;
    for cand in cands.iter().take(REFINE_CANDIDATES) {
        let r = refine(e, probes, cand, describer, want)?;
        let done = r.best.pass;
        let better = match &chosen {
            None => true,
            Some((_, c)) => !c.best.pass && (done || r.best.miss < c.best.miss),
        };
        if better {
            chosen = Some((cand.param, r));
        }
        if done {
            break;
        }
    }
    // A different waveform may do it alone.
    let wave_fix = probes
        .waves
        .iter()
        .find(|(_, d)| e.eval(d).pass)
        .map(|(w, d)| (*w, d));
    let cont_passes = chosen.as_ref().is_some_and(|(_, r)| r.best.pass);
    if !cont_passes && let Some((w, d)) = wave_fix {
        return Ok(json!({
            "param": "osc.wave", "now": base.wave.name(), "try": w.name(),
            "expected": expected_text(e, d), "measured": true, "reaches": true,
        }));
    }
    let Some((param, r)) = chosen else {
        if e.feature == Feature::Noisiness {
            return Ok(text_hint(
                "this toy synth has no noise source, so noisiness stays near 0 whatever the parameters; it cannot be raised here",
                false,
            ));
        }
        return Ok(text_hint(
            if e.feature.get(got).is_none() {
                format!(
                    "{} cannot be measured: {}",
                    e.feature.alias(),
                    e.feature.missing_reason()
                )
            } else {
                format!(
                    "none of the parameters moves {} measurably (10% steps of each range): it does not depend on this sound design; try osc.wave",
                    e.feature.alias()
                )
            },
            true,
        ));
    };
    let spec = &CONT[param];
    if !r.best.pass && e.feature.truncated(&r.best.d) {
        // Every candidate was cut off by the end of the clip: the clip must be longer first.
        let need = match e.feature {
            Feature::Release => base.perf.hold_s + e.value * 1.2 / 1000.0 + 0.1,
            _ => e.value * 1.2 / 1000.0 + 0.1,
        };
        let dur = ((need * 2.0).ceil() / 2.0).clamp(base.perf.dur_s, crate::patch::DUR_MAX_S);
        return Ok(json!({
            "param": "perf.dur", "now": base.show(crate::patch::Key::Dur), "try": format!("{}s", r4(dur)),
            "text": format!("the clip ends before the {} could finish; lengthen it, then set {} as needed", e.feature.alias(), spec.key),
            "also": {"param": spec.key, "try": fmt_value(r.best.value, spec.unit)},
            "measured": false,
        }));
    }
    let mut hint = json!({
        "param": spec.key,
        "now": fmt_value(base.v[param], spec.unit),
        "try": fmt_value(r.best.value, spec.unit),
        "dir": dir_word(if r.best.u >= spec.to_u(base.v[param]) { 1 } else { -1 }),
        "expected": expected_text(e, &r.best.d),
        "measured": true,
        "reaches": r.best.pass,
    });
    let others: Vec<Value> = cands
        .iter()
        .filter(|c| c.param != param && c.in_range)
        .take(2)
        .map(|c| json!({"param": CONT[c.param].key, "dir": dir_word(c.dir)}))
        .collect();
    if !others.is_empty() {
        hint["others"] = json!(others);
    }
    if !r.best.pass {
        hint["note"] = json!("this parameter alone does not get there; combine it with the others");
    }
    Ok(hint)
}
