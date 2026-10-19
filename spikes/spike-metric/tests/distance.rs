//! Distance sanity: identical audio is 0, loudness does not matter, bigger changes score larger.

use spike_metric::distance::{Components, Dist, SILENT, Scale, components, distance};
use spike_metric::features::{AnalysisCfg, Analyzer};
use spike_metric::render::{N_PARAMS, RenderCfg, Wave, render_unit};
use spike_metric::rng::Rng;
use spike_metric::yin::Yin;

fn patch(seed: u64) -> Vec<f64> {
    let mut r = Rng::new(seed);
    (0..N_PARAMS).map(|_| r.uniform()).collect()
}

fn setup() -> (RenderCfg, Analyzer) {
    let cfg = RenderCfg::short();
    let an = Analyzer::new(AnalysisCfg {
        sample_rate: cfg.sample_rate,
        with_f0: true,
        ..AnalysisCfg::default()
    });
    (cfg, an)
}

#[test]
fn identical_audio_is_zero_for_every_distance() {
    let (cfg, mut an) = setup();
    let sc = Scale::DEFAULT;
    let mut buf = Vec::new();
    for seed in 1..=4 {
        render_unit(&cfg, &patch(seed), &mut buf);
        let (fa, fb) = (an.features(&buf), an.features(&buf));
        assert!(!fa.silent);
        for kind in Dist::ALL {
            assert_eq!(distance(kind, &fa, &fb, &sc), 0.0, "{kind:?} seed {seed}");
        }
        let c = components(&fa, &fb);
        assert_eq!((c.logmel, c.env, c.centroid, c.mfcc), (0.0, 0.0, 0.0, 0.0));
    }
}

#[test]
fn distances_are_symmetric_and_non_negative() {
    let (cfg, mut an) = setup();
    let sc = Scale::DEFAULT;
    let (mut x, mut y) = (Vec::new(), Vec::new());
    render_unit(&cfg, &patch(1), &mut x);
    render_unit(&cfg, &patch(2), &mut y);
    let (fx, fy) = (an.features(&x), an.features(&y));
    for kind in Dist::ALL {
        let (d1, d2) = (distance(kind, &fx, &fy, &sc), distance(kind, &fy, &fx, &sc));
        assert!(
            d1 > 0.0 && (d1 - d2).abs() <= 1e-12 * d1.max(1.0),
            "{kind:?}: {d1} vs {d2}"
        );
    }
}

#[test]
fn loudness_is_normalised_away() {
    let (cfg, mut an) = setup();
    let mut x = Vec::new();
    render_unit(&cfg, &patch(5), &mut x);
    let quiet: Vec<f32> = x.iter().map(|v| v * 0.05).collect();
    let loud: Vec<f32> = x.iter().map(|v| v * 3.0).collect();
    let (f0, fq, fl) = (an.features(&x), an.features(&quiet), an.features(&loud));
    for kind in Dist::ALL {
        for other in [&fq, &fl] {
            let d = distance(kind, &f0, other, &Scale::DEFAULT);
            assert!(d < 1e-3, "{kind:?}: {d}");
        }
    }
    assert!(
        (f0.lufs - fq.lufs - 26.0).abs() < 0.2,
        "quiet copy should read 26 LU lower: {} vs {}",
        f0.lufs,
        fq.lufs
    );
}

#[test]
fn small_changes_score_below_large_ones() {
    let (cfg, mut an) = setup();
    let sc = Scale::DEFAULT;
    let mut mean = |amount: f64| -> [f64; 3] {
        let mut acc = [0.0; 3];
        let mut buf = Vec::new();
        for seed in 1..=6u64 {
            let base = patch(seed);
            render_unit(&cfg, &base, &mut buf);
            let fb = an.features(&buf);
            let mut rng = Rng::new(seed + 1000);
            let jittered: Vec<f64> = base
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if i == 0 {
                        *v
                    } else {
                        (v + amount * (2.0 * rng.uniform() - 1.0)).clamp(0.0, 1.0)
                    }
                })
                .collect();
            render_unit(&cfg, &jittered, &mut buf);
            let fj = an.features(&buf);
            for (k, kind) in Dist::ALL.iter().enumerate() {
                acc[k] += distance(*kind, &fb, &fj, &sc) / 6.0;
            }
        }
        acc
    };
    let (small, mid, large) = (mean(0.01), mean(0.1), mean(0.5));
    for k in 0..3 {
        assert!(
            small[k] < mid[k] && mid[k] < large[k],
            "distance {k}: {small:?} {mid:?} {large:?}"
        );
    }
}

#[test]
fn wrong_waveform_is_farther_than_the_right_one() {
    let (cfg, mut an) = setup();
    let mut base = patch(9);
    base[0] = Wave::Saw.unit();
    base[2] = 0.9; // open filter, so the waveform is audible
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    render_unit(&cfg, &base, &mut a);
    let mut near = base.clone();
    near[1] = (near[1] + 0.02).min(1.0);
    render_unit(&cfg, &near, &mut b);
    let mut sq = base.clone();
    sq[0] = Wave::Square.unit();
    render_unit(&cfg, &sq, &mut c);
    let (fa, fb, fc) = (an.features(&a), an.features(&b), an.features(&c));
    for kind in Dist::ALL {
        let (near_d, far_d) = (
            distance(kind, &fa, &fb, &Scale::DEFAULT),
            distance(kind, &fa, &fc, &Scale::DEFAULT),
        );
        assert!(near_d < far_d, "{kind:?}: {near_d} !< {far_d}");
    }
}

#[test]
fn silence_is_flagged_and_maximally_far() {
    let (cfg, mut an) = setup();
    let mut x = Vec::new();
    render_unit(&cfg, &patch(3), &mut x);
    let silent = vec![0.0f32; x.len()];
    let (fx, fs) = (an.features(&x), an.features(&silent));
    assert!(fs.silent && !fx.silent);
    assert!(distance(Dist::A, &fx, &fs, &Scale::DEFAULT) >= SILENT);
    let c = components(&fx, &fs);
    assert_eq!(
        c,
        Components {
            logmel: SILENT,
            env: SILENT,
            centroid: SILENT,
            mfcc: SILENT,
            f0_cents: None
        }
    );
}

#[test]
fn yin_finds_the_note() {
    let sr = 48_000.0f32;
    let mut yin = Yin::new(sr, 2048, 50.0, 2000.0);
    for f in [110.0f32, 220.0, 330.0, 880.0] {
        let x: Vec<f32> = (0..2048)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / sr).sin())
            .collect();
        let est = yin.estimate(&x).expect("voiced");
        assert!(
            (1200.0 * (est / f).log2()).abs() < 5.0,
            "{f} Hz estimated as {est}"
        );
    }
    let noise: Vec<f32> = {
        let mut r = Rng::new(1);
        (0..2048).map(|_| r.normal() as f32).collect()
    };
    assert!(
        yin.estimate(&noise).is_none(),
        "white noise should be unvoiced"
    );
    assert!(yin.estimate(&[0.0; 2048]).is_none());
}

#[test]
fn f0_track_of_a_rendered_note_is_a3() {
    let (cfg, mut an) = setup();
    let mut p = patch(4);
    p[1] = 0.0; // no detune
    p[2] = 0.8;
    let mut buf = Vec::new();
    render_unit(&cfg, &p, &mut buf);
    let f = an.features(&buf);
    let voiced: Vec<f32> = f.f0_hz.iter().copied().filter(|v| *v > 0.0).collect();
    assert!(voiced.len() > 3, "too few voiced frames: {}", voiced.len());
    let mut v = voiced.clone();
    v.sort_by(f32::total_cmp);
    let med = v[v.len() / 2];
    assert!(
        (1200.0 * (med / 220.0).log2()).abs() < 15.0,
        "median f0 {med}"
    );
}
