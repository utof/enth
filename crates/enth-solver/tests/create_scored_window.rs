//! Lane CLC: the robust objective for another scored window (`RobustMetricTarget::from_intake_scored`, behind
//! `enth create --score-seconds`). The default window is `from_intake`'s, bit for bit; a window above `SCORE_S` lets a
//! clip that ends in its floor compare the silence after the sound, which the default window would stop short of.

mod create_support;

use create_support::target_patch;
use enth_analysis::intake::Intake;
use enth_render::DEFAULT_SEED;
use enth_solver::create::{SCORE_S, Target};
use enth_solver::robust::RobustMetricTarget;

const SR: u32 = 22_050;

/// A 220 Hz tone that dies away over about 3.4 s (60 dB at 6.9 time constants of 0.5 s), then half a second of
/// silence: the intake keeps it whole (under its 5 s cap) and the clip ends in the floor.
fn long_decay() -> Vec<f32> {
    let n = (3.9 * f64::from(SR)) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(SR);
            if t < 3.4 {
                (0.5 * libm::sin(2.0 * core::f64::consts::PI * 220.0 * t) * libm::exp(-t / 0.5))
                    as f32
            } else {
                0.0
            }
        })
        .collect()
}

#[test]
fn the_default_window_is_from_intake() {
    let it = Intake::new(&long_decay(), 1, SR).unwrap();
    let target = Target::from_intake(&it, DEFAULT_SEED);
    let a = RobustMetricTarget::from_intake(&target, &it.report);
    let b = RobustMetricTarget::from_intake_scored(&target, &it.report, SCORE_S);
    assert_eq!(a.performance(), b.performance());
    assert_eq!(a.robust().extra_samples(), b.robust().extra_samples());
    let p = target_patch();
    let (da, db) = (a.worker().distance(&p), b.worker().distance(&p));
    assert!(da.is_finite());
    assert_eq!(da.to_bits(), db.to_bits(), "{da} {db}");
}

#[test]
fn a_wider_window_compares_the_silence_after_a_long_sound() {
    let it = Intake::new(&long_decay(), 1, SR).unwrap();
    assert!(
        it.report.span.end_s > SCORE_S + 0.2,
        "the premise: the intake keeps more than the default window: {:?}",
        it.report.span
    );
    // The default window cuts the clip: nothing ended inside it, and nothing is compared past it.
    let cut = Target::from_intake(&it, DEFAULT_SEED);
    let rc = RobustMetricTarget::from_intake(&cut, &it.report);
    assert_eq!(rc.robust().extra_samples(), 0);
    // A 5 s window keeps the whole clip, which ends in its floor...
    let whole = Target::from_intake_scored(&it, DEFAULT_SEED, 5.0);
    assert!(whole.perf.duration_s > SCORE_S);
    // ...which `from_intake` would not compare past (it bounds the silence by the default window),
    let stopped = RobustMetricTarget::from_intake(&whole, &it.report);
    assert_eq!(stopped.robust().extra_samples(), 0);
    // and `from_intake_scored` does, up to the window.
    let scored = RobustMetricTarget::from_intake_scored(&whole, &it.report, 5.0);
    assert!(scored.robust().extra_samples() > 0);
    let extra = scored.performance().duration_s - whole.perf.duration_s;
    assert!(
        extra > 0.0 && whole.perf.duration_s + extra <= 5.0 + 1e-9,
        "{extra}"
    );
    // A window that is not a time scores the whole clip, and compares nothing past it.
    let all = RobustMetricTarget::from_intake_scored(&whole, &it.report, f64::NAN);
    assert_eq!(all.robust().extra_samples(), 0);
}
