//! CMA-ES on simple functions: convergence, bounds, determinism, restarts.

use spike_metric::cmaes::{Cmaes, StopReason, default_lambda};

/// Run for at most `max_gen` generations, stopping early when the restart rule fires.
fn run(es: Cmaes, f: impl Fn(&[f64]) -> f64, max_gen: usize) -> (Cmaes, f64) {
    run_with(es, f, max_gen, true)
}

fn run_with(
    mut es: Cmaes,
    f: impl Fn(&[f64]) -> f64,
    max_gen: usize,
    honour_stop: bool,
) -> (Cmaes, f64) {
    let mut best = f64::INFINITY;
    for _ in 0..max_gen {
        let xs = es.ask();
        let fit: Vec<f64> = xs.iter().map(|x| f(x)).collect();
        best = fit.iter().copied().fold(best, f64::min);
        es.tell(&fit);
        if honour_stop && es.stop_reason().is_some() {
            break;
        }
    }
    (es, best)
}

#[test]
fn quadratic_converges_to_the_optimum() {
    let n = 9;
    let centre: Vec<f64> = (0..n).map(|i| 0.2 + 0.07 * i as f64).collect();
    let f = |x: &[f64]| {
        x.iter()
            .zip(&centre)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
    };
    let es = Cmaes::new(
        vec![0.5; n],
        0.3,
        default_lambda(n),
        vec![0.0; n],
        vec![1.0; n],
        7,
    );
    // the restart rule fires once sigma * max(D) < 1e-4, i.e. at an objective of about 1e-8
    let (_, best) = run(
        Cmaes::new(
            vec![0.5; n],
            0.3,
            default_lambda(n),
            vec![0.0; n],
            vec![1.0; n],
            7,
        ),
        f,
        400,
    );
    assert!(best < 1e-6, "best {best}");
    // without the restart rule the search keeps converging to machine precision
    let (es, best) = run_with(es, f, 300, false);
    assert!(best < 1e-14, "best {best}");
    for (m, c) in es.mean().iter().zip(&centre) {
        assert!((m - c).abs() < 1e-6, "mean {m} vs {c}");
    }
}

#[test]
fn ill_conditioned_ellipsoid_is_solved_by_covariance_adaptation() {
    let n = 6;
    // axes scaled 1 .. 1e3 in the square root: condition number 1e6
    let f = |x: &[f64]| {
        x.iter()
            .enumerate()
            .map(|(i, v)| (1000f64.powf(i as f64 / (n - 1) as f64) * (v - 0.4)).powi(2))
            .sum::<f64>()
    };
    let es = Cmaes::new(vec![0.8; n], 0.3, 12, vec![0.0; n], vec![1.0; n], 3);
    let (_, best) = run_with(es, f, 800, false);
    assert!(best < 1e-8, "best {best}");
}

#[test]
fn optimum_outside_the_box_lands_on_the_bound() {
    let n = 4;
    // unconstrained optimum at 1.4 in coordinate 0 and -0.3 in coordinate 1
    let target = [1.4, -0.3, 0.5, 0.5];
    let f = |x: &[f64]| {
        x.iter()
            .zip(&target)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
    };
    let es = Cmaes::new(vec![0.5; n], 0.3, 10, vec![0.0; n], vec![1.0; n], 11);
    let (mut es, _) = run(es, f, 300);
    let xs = es.ask();
    assert!(
        xs.iter().flatten().all(|v| (0.0..=1.0).contains(v)),
        "asked points leave the box"
    );
    let m = es.mean();
    assert!((m[2] - 0.5).abs() < 0.02 && (m[3] - 0.5).abs() < 0.02);
    // the distribution mean may sit slightly outside, but the evaluated (clamped) points pile onto the bounds
    let at_bound0 = xs.iter().filter(|x| x[0] > 0.98).count();
    let at_bound1 = xs.iter().filter(|x| x[1] < 0.02).count();
    assert!(
        at_bound0 * 2 >= xs.len() && at_bound1 * 2 >= xs.len(),
        "{at_bound0} {at_bound1} of {}",
        xs.len()
    );
}

#[test]
fn same_seed_same_trajectory_different_seed_differs() {
    let f = |x: &[f64]| x.iter().map(|v| (v - 0.3).powi(2)).sum::<f64>();
    let make = |seed| Cmaes::new(vec![0.5; 5], 0.3, 10, vec![0.0; 5], vec![1.0; 5], seed);
    let (a, _) = run(make(1), f, 20);
    let (b, _) = run(make(1), f, 20);
    let (c, _) = run(make(2), f, 20);
    assert_eq!(a.mean(), b.mean());
    assert_eq!(a.sigma().to_bits(), b.sigma().to_bits());
    assert_ne!(a.mean(), c.mean());
}

#[test]
fn collapsed_search_asks_for_a_restart() {
    let f = |x: &[f64]| x.iter().map(|v| (v - 0.3).powi(2)).sum::<f64>();
    let es = Cmaes::new(vec![0.5; 3], 0.3, 8, vec![0.0; 3], vec![1.0; 3], 5);
    let (es, _) = run(es, f, 2000);
    assert!(matches!(
        es.stop_reason(),
        Some(StopReason::TolX | StopReason::Stall | StopReason::Condition)
    ));
}

#[test]
fn default_lambda_is_hansens() {
    assert_eq!(default_lambda(9), 10);
    assert_eq!(default_lambda(10), 10);
    assert_eq!(default_lambda(2), 6);
}
