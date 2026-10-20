//! **Owner: lane SN** (sensitivity and coach). Pre-declared empty by lane S0r (R1/S1).
//!
//! Per-parameter sensitivity of a challenge's sound (report 02): how much each descriptor of the coach's vocabulary
//! ([`FEATURES`]: pitch, the envelope stages, brightness, noisiness, beating, tremolo, sweep depth) moves when one
//! parameter moves by one just-noticeable step, measured by renders through the caller's [`Evaluator`]. Structure %
//! uses it for its tolerances (lane ST, [`ParamSensitivity::tolerance_norm`]), the coach for the tolerances of its
//! `Expect` list ([`Sensitivity::tolerance`]) and for attributing a move to the aspect it fixes
//! ([`ParamSensitivity::slope`]). Identical results for 1 and 8 workers (SN's acceptance).
//!
//! # What is measured
//!
//! A one-sided finite difference per continuous parameter: the patch is rendered once as it is, and once per
//! parameter with that parameter moved by `jnd_steps` just-noticeable steps ([`enth_challenge::priors::jnd`]:
//! cutoff 2 semitones, times ×1.5, levels 3 dB, cents 3) toward the side of its range with more room. All probes go
//! in **one batch** of `1 + probed` evaluations of the full performance (envelope stages and slow beats cannot be
//! measured on a short clip), so the render budget is `max_renders` and nothing else. The step is a just-noticeable
//! step, not an infinitesimal one, on purpose: it is the size that matters to a listener, and a descriptor that
//! cannot move by that much is not worth a tolerance.
//!
//! Each descriptor's change is taken in its **compare domain** ([`domain_of`]): nepers (ln of the ratio) for Hz and
//! ms features, cents for pitch, the feature's unit otherwise. A change under a tenth of the descriptor's aim margin
//! ([`aim`]) counts as zero: descriptor quantisation (1 ms envelope frames, the pitch tracker's resolution) would
//! otherwise give every parameter a small, meaningless slope. A descriptor present in only one of the two renders is
//! a presence change ([`Change::Appeared`], [`Change::Vanished`]), not a number.
//!
//! # Tolerances
//!
//! [`Sensitivity::tolerance`] of a descriptor is the movement it would show if every probed parameter were one
//! just-noticeable step off (the root-sum-square of the parameters' deltas), kept between [`tolerance_bounds`]: two
//! aim margins at least, so the hint solver's aim band (the interval shrunk by one margin at each end) is never
//! empty, and eight at most, so a descriptor that every parameter moves wildly is not waved through. Pitch is bounded
//! between 10 and 25 cents (the CLI's default is 25).
//!
//! # Determinism and cost
//!
//! Pure: the result depends on the patch, the parameters, the performance, the render spec and the configuration
//! only, never on the evaluator's worker count (one batch, results by job index; fixed loop orders; `libm`).
//! Parameters are probed in the caller's order, with the ones that carry a `perceptual` descriptor tag first when the
//! budget cannot cover them all; those left out are in [`Sensitivity::unprobed`]. A Recreate round computes it once
//! per target (`1 +` the randomized continuous parameters, about 5 to 30 renders) and keeps it with the round.

use core::fmt;

use enth_analysis::descriptors::{Descriptors, Feature, Scale, Want};
use enth_challenge::priors::{Jnd, jnd};
use enth_params::{Kind, ParamSpec};
use enth_patch::{Op, ParamAddr, Patch, apply};
use enth_render::{Performance, RenderSpec};

use crate::eval::{EvalError, Evaluator, Job};

/// The descriptors the sensitivity and the coach read: report 02's per-descriptor breakdown (pitch, brightness, the
/// envelope's attack, decay, sustain and release, noisiness, movement as beating, tremolo and sweep depth).
/// `TonalPitch` stands in for `Pitch` under noise.
pub const FEATURES: [Feature; 11] = [
    Feature::Pitch,
    Feature::TonalPitch,
    Feature::Attack,
    Feature::Decay,
    Feature::Sustain,
    Feature::Release,
    Feature::Brightness,
    Feature::Noisiness,
    Feature::Beating,
    Feature::Tremolo,
    Feature::SweepDepth,
];

/// The aim margin for pitch, in cents (the hint solver's own, `hint/model.rs`).
const PITCH_AIM_CT: f64 = 3.0;
/// A delta under this many aim margins counts as no effect.
const DEAD_ZONE: f64 = 0.1;
/// A parameter whose strongest effect is under this fraction of a tolerance unit is inert for this target.
pub const INERT_BELOW: f64 = 0.25;

/// Settings of one [`sensitivity`] run. Build with `..Default::default()`; fields may be added, with defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct SensitivityCfg {
    /// Most evaluations (render + describe), the base included: the render budget. At least 1.
    pub max_renders: usize,
    /// The descriptors to read (default [`FEATURES`]).
    pub features: Vec<Feature>,
    /// The probe size, in just-noticeable steps of the parameter (default 1).
    pub jnd_steps: f64,
}

impl Default for SensitivityCfg {
    fn default() -> Self {
        Self {
            max_renders: 48,
            features: FEATURES.to_vec(),
            jnd_steps: 1.0,
        }
    }
}

/// What one descriptor did when a parameter moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// Present in both renders: the change in its compare domain ([`domain_of`]).
    Moved(f64),
    /// Absent in the base render, present after the move.
    Appeared,
    /// Present in the base render, absent after the move.
    Vanished,
    /// Absent in both.
    Absent,
}

/// One descriptor's response to a parameter's probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    /// The descriptor.
    pub feature: Feature,
    /// What it did.
    pub change: Change,
}

/// The response of every descriptor to one parameter's probe.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSensitivity {
    /// The parameter.
    pub addr: ParamAddr,
    /// Its spec (unit, range, display).
    pub spec: &'static ParamSpec,
    /// The value in the patch.
    pub from: f32,
    /// The value probed.
    pub to: f32,
    /// The probe's size and direction on the normalised axis (`to_norm(to) − to_norm(from)`, nonzero).
    pub step: f64,
    /// One entry per descriptor of the configuration, in its order.
    pub effects: Vec<Effect>,
}

impl ParamSensitivity {
    /// The response of `feature`, if it was read.
    #[must_use]
    pub fn effect(&self, feature: Feature) -> Option<&Effect> {
        self.effects.iter().find(|e| e.feature == feature)
    }

    /// The change of `feature` for the probe, in its compare domain, with deltas under the dead zone read as 0;
    /// `None` when the descriptor is absent in either render (or was not read).
    #[must_use]
    pub fn delta(&self, feature: Feature) -> Option<f64> {
        match self.effect(feature)?.change {
            Change::Moved(d) if libm::fabs(d) < DEAD_ZONE * aim(feature) => Some(0.0),
            Change::Moved(d) => Some(d),
            _ => None,
        }
    }

    /// `delta(feature)` per unit of the normalised axis: the slope the coach attributes moves with.
    #[must_use]
    pub fn slope(&self, feature: Feature) -> Option<f64> {
        self.delta(feature).map(|d| d / self.step)
    }

    /// How far the most affected descriptor moved for the probe, in units of its smallest tolerance
    /// ([`tolerance_bounds`]`.0`); a descriptor that appeared or vanished counts as one unit.
    #[must_use]
    pub fn reach(&self) -> f64 {
        self.effects
            .iter()
            .map(|e| match e.change {
                Change::Moved(_) => {
                    libm::fabs(self.delta(e.feature).unwrap_or(0.0)) / tolerance_bounds(e.feature).0
                }
                Change::Appeared | Change::Vanished => 1.0,
                Change::Absent => 0.0,
            })
            .fold(0.0, f64::max)
    }

    /// No descriptor notices one just-noticeable step of this parameter ([`INERT_BELOW`]): for this target it is
    /// inert, and structure % need not compare it.
    #[must_use]
    pub fn inert(&self) -> bool {
        self.reach() < INERT_BELOW
    }

    /// How far this parameter may move, as a fraction of its normalised range (at most 1), before the most affected
    /// descriptor has moved by its smallest tolerance: `|step| / reach`, linear in the probe. `None` when no
    /// descriptor moved at all (the parameter never reaches a tolerance).
    #[must_use]
    pub fn tolerance_norm(&self) -> Option<f64> {
        let r = self.reach();
        (r > 0.0).then(|| (libm::fabs(self.step) / r).min(1.0))
    }
}

/// The result of [`sensitivity`].
#[derive(Clone, Debug, PartialEq)]
pub struct Sensitivity {
    /// The descriptors of the patch as it is.
    pub base: Descriptors,
    /// The descriptors that were read, in order.
    pub features: Vec<Feature>,
    /// The probed parameters, in the order they were given (the budget's priority order only chooses which are
    /// probed).
    pub params: Vec<ParamSensitivity>,
    /// Continuous parameters left out: over the render budget, no room to move, or a probe that failed to render.
    pub unprobed: Vec<ParamAddr>,
    /// Parameters that are not continuous (waveforms, modes, toggles) or that the patch does not have: a finite
    /// difference means nothing for them.
    pub discrete: Vec<ParamAddr>,
    /// Evaluations used, the base included.
    pub renders: usize,
}

impl Sensitivity {
    /// The sensitivity of one parameter.
    #[must_use]
    pub fn param(&self, addr: &ParamAddr) -> Option<&ParamSensitivity> {
        self.params.iter().find(|p| &p.addr == addr)
    }

    /// The movement `feature` would show if every probed parameter were one just-noticeable step off: the
    /// root-sum-square of their deltas, in its compare domain.
    #[must_use]
    pub fn combined(&self, feature: Feature) -> f64 {
        self.params
            .iter()
            .filter_map(|p| p.delta(feature))
            .fold(0.0, |acc, d| acc + d * d)
            .sqrt()
    }

    /// The coach's tolerance for `feature`: [`combined`](Self::combined) kept within [`tolerance_bounds`].
    #[must_use]
    pub fn tolerance(&self, feature: Feature) -> f64 {
        let (lo, hi) = tolerance_bounds(feature);
        self.combined(feature).clamp(lo, hi)
    }
}

/// Why [`sensitivity`] could not run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SensitivityError {
    /// `max_renders` is 0: not even the base can be rendered.
    NoBudget,
    /// The performance plays no note, so there is nothing to measure.
    NoNotes,
    /// The base patch could not be rendered or described.
    Base(EvalError),
    /// The evaluator returned the wrong number of results.
    Contract {
        /// Jobs sent.
        jobs: usize,
        /// Results returned.
        results: usize,
    },
}

impl SensitivityError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            SensitivityError::NoBudget => "no_budget",
            SensitivityError::NoNotes => "no_notes",
            SensitivityError::Base(e) => e.code(),
            SensitivityError::Contract { .. } => "evaluator_contract",
        }
    }
}

impl fmt::Display for SensitivityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SensitivityError::NoBudget => {
                f.write_str("the render budget is 0; it must cover the base render")
            }
            SensitivityError::NoNotes => {
                f.write_str("the performance plays no note; give it at least one")
            }
            SensitivityError::Base(e) => write!(f, "the patch could not be measured: {e}"),
            SensitivityError::Contract { jobs, results } => write!(
                f,
                "the evaluator returned {results} results for {jobs} jobs (one per job, in order)"
            ),
        }
    }
}

impl std::error::Error for SensitivityError {}

// ---------------------------------------------------------------------------------------------------------------
// The compare domain
// ---------------------------------------------------------------------------------------------------------------

/// A reading of `feature` in its compare domain: cents above 1 Hz for pitch (so a difference is a cents error), the
/// natural log for Hz and ms features (a difference is a log ratio, in nepers), the value itself otherwise. `None`
/// for a non-positive reading on a log axis or a reading that is not finite.
#[must_use]
pub fn domain_of(feature: Feature, value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    if feature.is_pitch() {
        (value > 0.0).then(|| 1200.0 * libm::log2(value))
    } else if feature.scale() == Scale::Log {
        (value > 0.0).then(|| libm::log(value))
    } else {
        Some(value)
    }
}

/// Whether `feature` is compared on a log axis (Hz and ms features, not pitch, which is in cents).
#[must_use]
pub fn is_log(feature: Feature) -> bool {
    !feature.is_pitch() && feature.scale() == Scale::Log
}

/// [`domain_of`] of the reading of `feature` in `d`; `None` when it is absent.
#[must_use]
pub fn domain_value(feature: Feature, d: &Descriptors) -> Option<f64> {
    d.get(feature).and_then(|v| domain_of(feature, v))
}

/// One aim margin of `feature` in its compare domain: how far past a threshold the hint solver aims
/// (`Feature::hint_margin`; 3 cents for pitch).
#[must_use]
pub fn aim(feature: Feature) -> f64 {
    if feature.is_pitch() {
        PITCH_AIM_CT
    } else if is_log(feature) {
        libm::log(feature.hint_margin())
    } else {
        feature.hint_margin()
    }
}

/// The least and the most a tolerance of `feature` may be, in its compare domain: two and eight aim margins, except
/// pitch, which sits between 10 and 25 cents.
#[must_use]
pub fn tolerance_bounds(feature: Feature) -> (f64, f64) {
    if feature.is_pitch() {
        (10.0, 25.0)
    } else {
        (2.0 * aim(feature), 8.0 * aim(feature))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The probe
// ---------------------------------------------------------------------------------------------------------------

/// The value `k` just-noticeable steps from `from` (negative `k` goes down), before clamping.
fn steps_from(step: Jnd, from: f32, k: f64) -> f64 {
    let a = f64::from(from);
    match step {
        Jnd::Any => a,
        Jnd::Abs(s) => a + k * f64::from(s),
        Jnd::Ratio { factor, floor } => {
            let f = f64::from(floor);
            (a + f) * libm::pow(f64::from(factor), k) - f
        }
        Jnd::Semitones(st) => a * libm::pow(2.0, k * f64::from(st) / 12.0),
        Jnd::Db { db, floor } => {
            let f = f64::from(floor);
            (a + f) * libm::pow(10.0, k * f64::from(db) / 20.0) - f
        }
    }
}

/// The probe value for a continuous parameter: `steps` just-noticeable steps from `from`, on the side with more
/// room (clamped to the range), toward the middle of the range on a tie. `None` when the range leaves no room.
fn probe_value(spec: &'static ParamSpec, from: f32, steps: f64) -> Option<f32> {
    let step = jnd(spec);
    let u0 = f64::from(spec.to_norm(from));
    let first = if u0 <= 0.5 { 1.0 } else { -1.0 };
    let mut best: Option<(f32, f64)> = None;
    for dir in [first, -first] {
        let v = steps_from(step, from, dir * steps) as f32;
        if !v.is_finite() {
            continue;
        }
        let v = spec.clamp(v);
        let room = libm::fabs(f64::from(spec.to_norm(v)) - u0);
        if v != from && best.is_none_or(|(_, r)| room > r + 1e-9) {
            best = Some((v, room));
        }
    }
    best.map(|(v, _)| v)
}

fn is_continuous(spec: &ParamSpec) -> bool {
    matches!(spec.kind, Kind::Float { .. })
}

/// Measure how `params` of `patch` move the descriptors (see the [module docs](self)).
///
/// `params` are the parameters to probe (a Recreate round passes the randomized ones); the patch is the target (or
/// any patch whose neighbourhood matters). `perf` is the performance every render plays, `spec` the sample rate and
/// seed, `eval` the caller's parallelism.
///
/// # Errors
/// A zero budget, a performance without notes, a base patch that cannot be measured, or an evaluator that breaks its
/// contract. A probe that fails to render only leaves its parameter in [`Sensitivity::unprobed`].
pub fn sensitivity(
    patch: &Patch,
    params: &[ParamAddr],
    perf: &Performance,
    spec: &RenderSpec,
    eval: &mut dyn Evaluator,
    cfg: &SensitivityCfg,
) -> Result<Sensitivity, SensitivityError> {
    if cfg.max_renders == 0 {
        return Err(SensitivityError::NoBudget);
    }
    if perf.notes.is_empty() {
        return Err(SensitivityError::NoNotes);
    }
    // Which parameters can be probed: continuous ones the patch has, once each, in the caller's order.
    let mut discrete: Vec<ParamAddr> = Vec::new();
    let mut unprobed: Vec<ParamAddr> = Vec::new();
    let mut candidates: Vec<(ParamAddr, &'static ParamSpec, f32, f32)> = Vec::new();
    let mut seen: Vec<&ParamAddr> = Vec::new();
    for addr in params {
        if seen.contains(&addr) {
            continue;
        }
        seen.push(addr);
        let (Some(pspec), Some(from)) = (patch.spec(addr), patch.value(addr)) else {
            discrete.push(addr.clone());
            continue;
        };
        if !is_continuous(pspec) {
            discrete.push(addr.clone());
        } else if let Some(to) = probe_value(pspec, from, cfg.jnd_steps) {
            candidates.push((addr.clone(), pspec, from, to));
        } else {
            unprobed.push(addr.clone());
        }
    }
    // The budget: tagged parameters first (their descriptor is known to matter), then the rest; both in the
    // caller's order.
    let room = cfg.max_renders - 1;
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&i| (candidates[i].1.perceptual.is_none(), i));
    let mut chosen: Vec<usize> = order[..room.min(order.len())].to_vec();
    for &i in &order[chosen.len()..] {
        unprobed.push(candidates[i].0.clone());
    }
    chosen.sort_unstable();

    let want = Want::for_features(&cfg.features);
    let mut jobs: Vec<Job<'_>> = Vec::with_capacity(1 + chosen.len());
    jobs.push(Job {
        patch: patch.clone(),
        perf,
        spec: *spec,
        want,
    });
    let mut probes: Vec<usize> = Vec::with_capacity(chosen.len());
    for &i in &chosen {
        let (addr, _, _, to) = &candidates[i];
        let mut p = patch.clone();
        let op = Op::SetParam {
            addr: addr.clone(),
            value: *to,
        };
        if apply(&mut p, &op).is_err() {
            unprobed.push(addr.clone());
            continue;
        }
        jobs.push(Job {
            patch: p,
            perf,
            spec: *spec,
            want,
        });
        probes.push(i);
    }
    let results = eval.evaluate(&jobs);
    let contract = SensitivityError::Contract {
        jobs: jobs.len(),
        results: results.len(),
    };
    if results.len() != jobs.len() {
        return Err(contract);
    }
    let mut results = results.into_iter();
    let base = results
        .next()
        .ok_or(contract)?
        .map_err(SensitivityError::Base)?;
    let mut out: Vec<ParamSensitivity> = Vec::with_capacity(probes.len());
    for (&i, result) in probes.iter().zip(results) {
        let (addr, pspec, from, to) = &candidates[i];
        let Ok(d) = result else {
            unprobed.push(addr.clone());
            continue;
        };
        let step = f64::from(pspec.to_norm(*to)) - f64::from(pspec.to_norm(*from));
        let effects = cfg
            .features
            .iter()
            .map(|&feature| Effect {
                feature,
                change: match (domain_value(feature, &base), domain_value(feature, &d)) {
                    (Some(a), Some(b)) => Change::Moved(b - a),
                    (None, Some(_)) => Change::Appeared,
                    (Some(_), None) => Change::Vanished,
                    (None, None) => Change::Absent,
                },
            })
            .collect();
        out.push(ParamSensitivity {
            addr: addr.clone(),
            spec: pspec,
            from: *from,
            to: *to,
            step,
            effects,
        });
    }
    Ok(Sensitivity {
        base,
        features: cfg.features.clone(),
        params: out,
        unprobed,
        discrete,
        renders: jobs.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_cents_nepers_and_units() {
        let a = domain_of(Feature::Pitch, 440.0).unwrap();
        let b = domain_of(Feature::Pitch, 880.0).unwrap();
        assert!((b - a - 1200.0).abs() < 1e-9);
        let c = domain_of(Feature::Brightness, 2000.0).unwrap();
        let d = domain_of(Feature::Brightness, 4000.0).unwrap();
        assert!((d - c - libm::log(2.0)).abs() < 1e-12);
        assert_eq!(domain_of(Feature::Sustain, -6.0), Some(-6.0));
        assert_eq!(domain_of(Feature::Decay, 0.0), None);
        assert_eq!(domain_of(Feature::Sustain, f64::NAN), None);
    }

    #[test]
    fn tolerance_bounds_hold_two_to_eight_aim_margins() {
        for f in FEATURES {
            let (lo, hi) = tolerance_bounds(f);
            assert!(lo > 0.0 && hi > lo, "{f:?}");
            if !f.is_pitch() {
                assert!((lo - 2.0 * aim(f)).abs() < 1e-12 && (hi - 8.0 * aim(f)).abs() < 1e-12);
            }
        }
        assert_eq!(tolerance_bounds(Feature::Pitch), (10.0, 25.0));
    }

    #[test]
    fn a_probe_goes_toward_the_roomier_side_and_stays_in_range() {
        let cutoff = enth_patch::catalogue::ModuleKind::Svf.param("cutoff").unwrap();
        // 2 kHz is above the middle of the log axis (632 Hz): the probe goes down by 2 semitones.
        let down = probe_value(cutoff, 2_000.0, 1.0).unwrap();
        assert!((down / 2_000.0 - libm::pow(2.0, -2.0 / 12.0) as f32).abs() < 1e-4);
        // Below the middle it goes up.
        let up = probe_value(cutoff, 100.0, 1.0).unwrap();
        assert!((up / 100.0 - libm::pow(2.0, 2.0 / 12.0) as f32).abs() < 1e-4);
        // Never outside the range.
        let top = probe_value(cutoff, cutoff.max(), 1.0).unwrap();
        assert!(top < cutoff.max());
    }
}
