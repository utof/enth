//! Lane OBJ: the robust distance (`logmel-pool85-robust/1`) as a create job's objective, through FE's intake and
//! TP's `create` on engine renders: the target patch scores (near) zero, a noisy upload no longer pays a candidate
//! for ringing into its noise, a slow attack that surfaced late starts the candidate early, and a create job under it
//! is the same for any batch order (the determinism a shell's pool relies on). The dev measurement (40 targets × 4
//! degradations) is in docs/r3.md.

mod create_support;

use create_support::{init, inputs, set, target_patch, templates};
use enth_analysis::intake::Intake;
use enth_analysis::robust::{METRIC_ROBUST, Settings};
use enth_analysis::test_signals::white_noise;
use enth_patch::Patch;
use enth_render::{DEFAULT_SEED, Note, Performance, RenderSpec, render};
use enth_solver::create::{CreateCfg, SCORE_S, Target, create};
use enth_solver::robust::RobustMetricTarget;
use enth_solver::search::{MetricTarget, Objective};

const SR: u32 = 22_050;

fn mid(p: &Patch, perf: &Performance) -> Vec<f32> {
    render(p, perf, &RenderSpec::new(SR, DEFAULT_SEED))
        .unwrap()
        .mid()
}

/// `p` held for `hold_s` from `on_s` in a clip of `len_s`.
fn note(on_s: f64, hold_s: f64, len_s: f64) -> Performance {
    Performance {
        notes: vec![Note::new(57, 0.8, on_s, on_s + hold_s)],
        duration_s: len_s,
        ..Performance::default()
    }
}

/// `x` plus white noise of amplitude `amp` (uniform), seeded.
fn noisy(x: &[f32], amp: f64, seed: u64) -> Vec<f32> {
    let n = white_noise(
        f64::from(SR),
        amp,
        x.len() as f64 / f64::from(SR) + 0.01,
        seed,
    );
    x.iter().zip(&n).map(|(a, b)| a + b).collect()
}

/// A pluck that dies away within the clip, after a lead-in of noise (which confirms the floor).
fn pluck_upload(amp: Option<f64>) -> (Patch, Vec<f32>) {
    let p = target_patch();
    let clean = mid(&p, &note(0.4, 0.25, 1.6));
    let x = match amp {
        Some(a) => noisy(&clean, a, 5),
        None => clean,
    };
    (p, x)
}

fn intake(x: &[f32]) -> Intake {
    Intake::new(x, 1, SR).unwrap()
}

#[test]
fn the_target_patch_scores_near_zero_through_the_intake() {
    let (p, x) = pluck_upload(None);
    let it = intake(&x);
    let target = Target::from_intake(&it, DEFAULT_SEED);
    let robust = RobustMetricTarget::from_intake(&target, &it.report);
    // the clip ends in silence well inside the scored window: the silence after it is compared too
    assert!(robust.robust().extra_samples() > 0);
    assert!(robust.performance().duration_s > target.perf.duration_s);
    assert!(target.perf.duration_s < SCORE_S);
    let mut w = robust.worker();
    let d = w.distance(&p);
    assert!(d < 0.02, "{d}");
    // a candidate that rings on after the upload went quiet is charged for it
    let mut ring = p.clone();
    set(&mut ring, "env1.release", 1_500.0);
    assert!(w.distance(&ring) > d + 0.05, "{} {d}", w.distance(&ring));
    assert_eq!(METRIC_ROBUST, "logmel-pool85-robust/1");
}

#[test]
fn a_noisy_upload_does_not_pay_for_ringing_into_its_noise() {
    let (p, x) = pluck_upload(Some(0.01));
    let it = intake(&x);
    let d = &it.report.degradation;
    assert!(d.floor_confirmed, "{d:?}");
    let target = Target::from_intake(&it, DEFAULT_SEED);
    let robust = RobustMetricTarget::from_intake(&target, &it.report);
    let mut ring = p.clone();
    set(&mut ring, "env1.sustain", 10.0);
    set(&mut ring, "env1.release", 2_000.0);
    let mut w = robust.worker();
    let (r_true, r_ring) = (w.distance(&p), w.distance(&ring));
    assert!(r_true < r_ring, "robust: {r_true} {r_ring}");
    // metric v1, today's objective, on the same clip: the truth is much farther than under the robust distance
    let v1 = MetricTarget::from_audio(&target.audio, &target.performance(), &target.render_spec());
    let mut vw = v1.worker();
    let v_true = vw.distance(&p);
    assert!(r_true < 0.5 * v_true, "{r_true} {v_true}");
}

#[test]
fn a_slow_attack_that_surfaced_late_starts_the_candidate_early() {
    let mut p = target_patch();
    set(&mut p, "env1.attack", 300.0);
    set(&mut p, "env1.sustain", 80.0);
    let clean = mid(&p, &note(0.4, 0.9, 1.6));
    let x = noisy(&clean, 0.02, 9);
    let it = intake(&x);
    assert!(it.report.degradation.floor_confirmed);
    let late = it.report.span.onset_s - 0.4;
    let target = Target::from_intake(&it, DEFAULT_SEED);
    let robust = RobustMetricTarget::from_intake(&target, &it.report);
    let lead = robust.robust().lead_s();
    if late > 0.008 {
        assert!(
            lead > 0.5 * late && lead < 1.5 * late + 0.005,
            "late {late}, lead {lead}"
        );
        let without = RobustMetricTarget::from_intake_with(
            &target,
            &it.report,
            &Settings {
                attack_lead: false,
                ..Settings::default()
            },
        );
        let (d_with, d_without) = (robust.worker().distance(&p), without.worker().distance(&p));
        assert!(d_with < d_without, "{d_with} {d_without}");
    } else {
        assert!(lead < 0.01, "late {late}, lead {lead}");
    }
}

/// Distances in input order from fresh workers, in the given order of work.
struct Shuffled<'a> {
    target: &'a RobustMetricTarget,
    reverse: bool,
}

impl Objective for Shuffled<'_> {
    fn distances(&mut self, candidates: &[Patch]) -> Vec<f64> {
        let mut out = vec![0.0; candidates.len()];
        let mut order: Vec<usize> = (0..candidates.len()).collect();
        if self.reverse {
            order.reverse();
        }
        // a fresh worker per half, as a pool hands work to whichever thread is free
        let (a, b) = order.split_at(order.len() / 2);
        for part in [a, b] {
            let mut w = self.target.worker();
            for &i in part {
                out[i] = w.distance(&candidates[i]);
            }
        }
        out
    }
}

#[test]
fn a_create_job_under_it_does_not_depend_on_the_order_of_work() {
    let (_, x) = pluck_upload(Some(0.005));
    let it = intake(&x);
    let target = Target::from_intake(&it, DEFAULT_SEED);
    let robust = RobustMetricTarget::from_intake(&target, &it.report);
    let (init, templates) = (init(), templates());
    let cfg = CreateCfg {
        k: 3,
        budget: 90,
        first_rung: 12,
        ..CreateCfg::default()
    };
    let run = |reverse: bool| {
        let mut obj = Shuffled {
            target: &robust,
            reverse,
        };
        create(
            &target,
            &inputs(&init, &templates),
            &mut obj,
            &cfg,
            &mut |_| true,
        )
        .unwrap()
    };
    let (a, b) = (run(false), run(true));
    assert_eq!(a.renders, b.renders);
    assert!(a.renders <= 90);
    for (x, y) in a.candidates.iter().zip(&b.candidates) {
        assert_eq!(x.distance, y.distance);
        assert_eq!(x.patch, y.patch);
    }
}
