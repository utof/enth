//! Same seed gives the same render hash, and results are bit-identical for any number of workers.

use spike_metric::distance::{Dist, Scale};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::heuristic::heuristic;
use spike_metric::par::ParEval;
use spike_metric::render::{N_PARAMS, RenderCfg, render_unit};
use spike_metric::rng::{Rng, hash_f32, hash_f64};
use spike_metric::search::{BatchEval, Search, SearchCfg, drive};

fn patch(seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    (0..N_PARAMS).map(|_| r.uniform()).collect()
}

#[test]
fn same_seed_same_render_hash() {
    let cfg = RenderCfg::default();
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    render_unit(&cfg, &patch(1), &mut a);
    render_unit(&cfg, &patch(1), &mut b);
    render_unit(&cfg, &patch(2), &mut c);
    assert_eq!(hash_f32(&a), hash_f32(&b));
    assert_ne!(hash_f32(&a), hash_f32(&c));
    assert_eq!(a.len(), 96_000);
    assert!(a.iter().all(|v| v.is_finite()));
}

/// The renderer uses only IEEE-exact operations and `libm` (pure Rust), so its output is bit-identical on every
/// platform and these hashes are portable. If you change the voice on purpose, update them and say why in the commit.
#[test]
fn render_hash_is_pinned() {
    let cfg = RenderCfg::default();
    let mut buf = Vec::new();
    let hashes: Vec<u64> = (1..=3)
        .map(|s| {
            render_unit(&cfg, &patch(s), &mut buf);
            hash_f32(&buf)
        })
        .collect();
    assert_eq!(
        hashes, PINNED_RENDER_HASHES,
        "render output changed: {hashes:016x?}"
    );
}

const PINNED_RENDER_HASHES: [u64; 3] = [
    0x13af_0d86_e4ff_ccfc,
    0xd811_cd4a_b4f7_883c,
    0x8abd_8235_4599_b289,
];

#[test]
fn one_worker_and_many_workers_agree_bit_for_bit() {
    let render = RenderCfg::short();
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let sc = Scale::DEFAULT;
    let cands: Vec<Vec<f64>> = (0..40).map(|i| patch(100 + i)).collect();
    for kind in Dist::ALL {
        let mut reference: Option<Vec<u64>> = None;
        for workers in [1, 2, 3, 8] {
            let mut pe = ParEval::new(workers, render, analysis, kind, sc);
            pe.set_target(&patch(7));
            let got: Vec<u64> = pe.eval(&cands).iter().map(|v| v.to_bits()).collect();
            match &reference {
                None => reference = Some(got),
                Some(r) => assert_eq!(
                    &got, r,
                    "{kind:?} with {workers} workers differs from 1 worker"
                ),
            }
        }
    }
}

#[test]
fn whole_search_is_identical_for_one_and_many_workers() {
    let render = RenderCfg::short();
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let target = patch(21);
    let mut audio = Vec::new();
    render_unit(&render, &target, &mut audio);
    let mut results = Vec::new();
    for workers in [1, 4] {
        let mut pe = ParEval::new(workers, render, analysis, Dist::A, Scale::DEFAULT);
        pe.set_target(&target);
        let cfg = |heur: bool| SearchCfg {
            seed: 5,
            start: heur.then(|| heuristic(&RenderCfg::short(), &audio)),
            lambda0: 8,
            budget: 160,
            ..SearchCfg::default()
        };
        let mut ss = vec![Search::new(cfg(true)), Search::new(cfg(false))];
        drive(&mut ss, &mut pe, |_| true);
        results.push((
            hash_f64(ss[0].best_x()),
            hash_f64(ss[1].best_x()),
            ss[0].best_fitness().to_bits(),
            ss[0].evals(),
        ));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn heuristic_is_deterministic_and_in_range() {
    let cfg = RenderCfg::default();
    let mut audio = Vec::new();
    render_unit(&cfg, &patch(33), &mut audio);
    let a = heuristic(&cfg, &audio);
    let b = heuristic(&cfg, &audio);
    assert_eq!(a.cont, b.cont);
    assert_eq!(a.cont.len(), N_PARAMS - 1);
    assert!(
        a.cont.iter().all(|v| (0.0..=1.0).contains(v)),
        "{:?}",
        a.cont
    );
    let mut an = Analyzer::new(AnalysisCfg::default());
    assert!(an.loudness(&audio).is_some());
}
