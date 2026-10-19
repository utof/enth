//! Listening-test design, level matching and analysis.

use spike_metric::distance::{Dist, Scale};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::listen::{
    Answer, AnswerFile, DesignCfg, Manifest, PairKind, analyze, design, stimulus, synthetic_answers,
};
use spike_metric::par::ParEval;
use spike_metric::render::RenderCfg;
use spike_metric::rng::Rng;
use std::collections::BTreeSet;

fn small_design(seed: u64) -> (Manifest, RenderCfg, AnalysisCfg) {
    let render = RenderCfg::short();
    let analysis = AnalysisCfg {
        sample_rate: render.sample_rate,
        ..AnalysisCfg::default()
    };
    let pe = ParEval::new(4, render, analysis, Dist::A, Scale::DEFAULT);
    let cfg = DesignCfg {
        seed,
        n_test: 12,
        n_bins: 4,
        pool: 48,
        n_same: 2,
        n_diff: 2,
    };
    let m = design(&cfg, &render, &Scale::DEFAULT, analysis.target_lufs, &|p| {
        pe.pair_components(p)
    });
    (m, render, analysis)
}

#[test]
fn design_is_balanced_deterministic_and_blind() {
    let (m, _, _) = small_design(1);
    let (m2, _, _) = small_design(1);
    let (m3, _, _) = small_design(2);
    assert_eq!(
        serde_json::to_string(&m).unwrap(),
        serde_json::to_string(&m2).unwrap(),
        "same seed, same design"
    );
    assert_ne!(
        serde_json::to_string(&m).unwrap(),
        serde_json::to_string(&m3).unwrap()
    );
    let test: Vec<_> = m
        .pairs
        .iter()
        .filter(|p| p.kind == PairKind::Test)
        .collect();
    assert_eq!(test.len(), 12);
    for bin in 0..4 {
        assert_eq!(
            test.iter().filter(|p| p.bin == Some(bin)).count(),
            3,
            "bin {bin}"
        );
    }
    // bins are distance-A quantiles: every bin's distances sit below the next bin's
    let max_of = |b: usize| {
        test.iter()
            .filter(|p| p.bin == Some(b))
            .map(|p| p.components.logmel)
            .fold(0.0, f64::max)
    };
    let min_of = |b: usize| {
        test.iter()
            .filter(|p| p.bin == Some(b))
            .map(|p| p.components.logmel)
            .fold(f64::MAX, f64::min)
    };
    for b in 0..3 {
        assert!(max_of(b) <= min_of(b + 1), "bin {b} overlaps bin {}", b + 1);
    }
    assert_eq!(
        m.pairs
            .iter()
            .filter(|p| p.kind == PairKind::AnchorSame)
            .count(),
        2
    );
    assert_eq!(
        m.pairs
            .iter()
            .filter(|p| p.kind == PairKind::AnchorDifferent)
            .count(),
        2
    );
    for p in m.pairs.iter().filter(|p| p.kind == PairKind::AnchorSame) {
        assert_eq!(p.components.logmel, 0.0);
        assert_eq!(p.a, p.b);
    }
    // two sessions with the same pairs, reshuffled, unique ids
    assert_eq!(m.sessions.len(), 2);
    let ids: BTreeSet<&str> = m.sessions.iter().flatten().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), 2 * m.pairs.len());
    for s in &m.sessions {
        let set: BTreeSet<usize> = s.iter().map(|t| t.pair).collect();
        assert_eq!(set.len(), m.pairs.len());
    }
    let order = |s: usize| m.sessions[s].iter().map(|t| t.pair).collect::<Vec<_>>();
    assert_ne!(order(0), order(1), "sessions should be reshuffled");
    assert!(
        m.sessions.iter().flatten().any(|t| t.swap) && m.sessions.iter().flatten().any(|t| !t.swap)
    );
}

#[test]
fn stimuli_are_level_matched_and_gapped() {
    let (m, render, analysis) = small_design(3);
    let mut an = Analyzer::new(analysis);
    let n = render.n_samples();
    let gap = (render.sample_rate * 0.5) as usize;
    for t in m.sessions[0].iter().take(8) {
        let audio = stimulus(&mut an, &render, &m.pairs[t.pair], t.swap);
        assert_eq!(audio.len(), 2 * n + gap);
        assert!(
            audio[n..n + gap].iter().all(|v| *v == 0.0),
            "gap must be silent"
        );
        assert!(audio.iter().all(|v| v.abs() <= 0.98 + 1e-6));
        assert_eq!(audio[0], 0.0, "faded in");
        let (la, lb) = (
            an.loudness(&audio[..n]).unwrap(),
            an.loudness(&audio[n + gap..]).unwrap(),
        );
        let peak_limited = audio.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.979;
        if !peak_limited {
            assert!(
                (la - m.target_lufs).abs() < 0.5 && (lb - m.target_lufs).abs() < 0.5,
                "{la} {lb}"
            );
        }
        assert!(
            (la - lb).abs() < 0.5,
            "halves differ by {} LU",
            (la - lb).abs()
        );
    }
}

#[test]
fn identical_pair_stimulus_has_two_identical_halves() {
    let (m, render, analysis) = small_design(3);
    let mut an = Analyzer::new(analysis);
    let n = render.n_samples();
    let gap = (render.sample_rate * 0.5) as usize;
    let p = m
        .pairs
        .iter()
        .find(|p| p.kind == PairKind::AnchorSame)
        .unwrap();
    let audio = stimulus(&mut an, &render, p, false);
    assert_eq!(&audio[..n], &audio[n + gap..]);
}

fn answers_from(
    m: &Manifest,
    session: u8,
    f: &dyn Fn(&spike_metric::listen::PairSpec, &mut Rng) -> u8,
    seed: u64,
) -> AnswerFile {
    let mut rng = Rng::new(seed);
    let mut file = AnswerFile::new(session, false);
    for t in &m.sessions[usize::from(session) - 1] {
        file.answers.push(Answer {
            n: t.n,
            id: t.id.clone(),
            rating: f(&m.pairs[t.pair], &mut rng),
            replays: 0,
            ms: 0,
        });
    }
    file
}

#[test]
fn analysis_rewards_a_metric_that_matches_the_ratings() {
    let (m, _, _) = small_design(4);
    // listener rates by distance-A rank: A should correlate strongly
    let mut a_sorted: Vec<f64> = m
        .pairs
        .iter()
        .filter(|p| p.kind == PairKind::Test)
        .map(|p| p.components.logmel)
        .collect();
    a_sorted.sort_by(f64::total_cmp);
    let rate = |p: &spike_metric::listen::PairSpec, rng: &mut Rng| -> u8 {
        match p.kind {
            PairKind::AnchorSame => 5,
            PairKind::AnchorDifferent => 1,
            PairKind::Test => {
                let q = a_sorted
                    .iter()
                    .filter(|v| **v < p.components.logmel)
                    .count() as f64
                    / a_sorted.len() as f64;
                (5.0 - 4.0 * q + 0.3 * rng.normal()).round().clamp(1.0, 5.0) as u8
            }
        }
    };
    let files = [
        answers_from(&m, 1, &rate, 10),
        answers_from(&m, 2, &rate, 11),
    ];
    let r = analyze(&m, &files, 300, 1);
    assert_eq!(r.n_test_pairs, 12);
    assert!(r.anchors.ok, "{:?}", r.anchors);
    let a = r.metrics.iter().find(|x| x.distance == Dist::A).unwrap();
    assert!(
        a.tau_b.estimate > 0.7 && a.spearman.estimate > 0.85,
        "{a:?}"
    );
    assert!(a.tau_b.lo <= a.tau_b.estimate && a.tau_b.estimate <= a.tau_b.hi);
    let retest = r.retest.as_ref().expect("two sessions");
    assert!(retest.spearman.estimate > 0.8, "{retest:?}");
    assert_eq!(r.tau_differences.len(), 3);
    assert!(r.to_text().contains("recommendation"));
}

#[test]
fn analysis_does_not_invent_correlation_from_noise() {
    let (m, _, _) = small_design(5);
    let noise = |p: &spike_metric::listen::PairSpec, rng: &mut Rng| -> u8 {
        match p.kind {
            PairKind::AnchorSame => 5,
            PairKind::AnchorDifferent => 1,
            PairKind::Test => 1 + rng.below(5) as u8,
        }
    };
    let files = [
        answers_from(&m, 1, &noise, 20),
        answers_from(&m, 2, &noise, 21),
    ];
    let r = analyze(&m, &files, 300, 1);
    for metric in &r.metrics {
        assert!(metric.spearman.estimate.abs() < 0.75, "{metric:?}");
        assert!(!metric.gate_pass, "{metric:?}");
    }
    let retest = r.retest.unwrap();
    assert!(
        retest.spearman.estimate.abs() < 0.75,
        "independent noise sessions should not retest well: {retest:?}"
    );
}

#[test]
fn single_session_and_partial_answers_still_analyse() {
    let (m, _, _) = small_design(6);
    let mut f = synthetic_answers(&m, 1, 3);
    assert!(f.synthetic);
    f.answers.truncate(10);
    let r = analyze(&m, &[f], 100, 1);
    assert!(r.retest.is_none());
    assert!(r.synthetic);
    assert!(r.n_test_pairs <= 10);
}

#[test]
fn synthetic_answers_are_plausible() {
    let (m, _, _) = small_design(7);
    let files = [synthetic_answers(&m, 1, 1), synthetic_answers(&m, 2, 1)];
    assert_eq!(files[0].answers.len(), m.pairs.len());
    let r = analyze(&m, &files, 200, 1);
    let a = r.metrics.iter().find(|x| x.distance == Dist::A).unwrap();
    assert!(a.spearman.estimate > 0.5, "{a:?}");
}
