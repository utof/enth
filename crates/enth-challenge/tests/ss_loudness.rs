//! Lane SS: the in-house BS.1770 loudness (`enth_challenge::loudness`) against `ebur128` (the reference
//! implementation `enth_analysis::loudness` wraps), on synthetic signals and on real renders.

use enth_analysis::loudness::LoudnessMeter;
use enth_analysis::test_signals::{SR, Wave, mix, oscillator, sine, white_noise};
use enth_challenge::loudness::integrated_lufs;
use enth_challenge::random;
use enth_challenge::spec::SpecPreset;
use enth_render::{RenderSpec, render};

/// The largest difference allowed from `ebur128` on clips of a block or longer, in LU. Both implement the same
/// filters and gates; what differs is the filter structure (two biquads in series here, one fourth-order section
/// there) and the order of sums, so they agree to far better than any listener could tell.
const TOLERANCE_LU: f64 = 1e-3;

fn both(x: &[f32]) -> (Option<f64>, Option<f64>) {
    let mut m = LoudnessMeter::new(48_000);
    (integrated_lufs(x, 48_000), m.integrated(x))
}

fn agree(name: &str, x: &[f32]) -> f64 {
    let (ours, theirs) = both(x);
    match (ours, theirs) {
        (Some(a), Some(b)) => {
            assert!(
                (a - b).abs() < TOLERANCE_LU,
                "{name}: ours {a}, ebur128 {b}"
            );
            (a - b).abs()
        }
        (None, None) => 0.0,
        other => panic!("{name}: one is silent: {other:?}"),
    }
}

#[test]
fn synthetic_signals_agree_with_ebur128() {
    let mut worst = 0.0f64;
    for (hz, amp) in [
        (997.0, 0.1),
        (997.0, 1.0),
        (40.0, 0.5),
        (100.0, 0.3),
        (3_000.0, 0.2),
        (12_000.0, 0.4),
    ] {
        worst = worst.max(agree(
            &format!("sine {hz} Hz {amp}"),
            &sine(SR, hz, amp, 3.0),
        ));
    }
    for wave in Wave::ALL {
        worst = worst.max(agree(wave.name(), &oscillator(SR, wave, 130.81, 2.0, 0.0)));
    }
    worst = worst.max(agree("white noise", &white_noise(SR, 0.3, 2.0, 11)));
    // Gating: a tone between silences, and a loud passage then one 20 dB down.
    let mut gated = vec![0.0f32; 96_000];
    gated.extend(sine(SR, 440.0, 0.2, 1.0));
    gated.extend(vec![0.0f32; 96_000]);
    worst = worst.max(agree("tone between silences", &gated));
    let mut steps = sine(SR, 440.0, 0.5, 2.0);
    steps.extend(sine(SR, 440.0, 0.05, 2.0));
    worst = worst.max(agree("loud then quiet", &steps));
    worst = worst.max(agree("silence", &vec![0.0f32; 48_000]));
    let chord = mix(&sine(SR, 220.0, 0.2, 2.0), &sine(SR, 331.0, 0.2, 2.0));
    worst = worst.max(agree("two sines", &chord));
    eprintln!("worst |ours - ebur128|: {worst:e} LU");
    assert!(worst < TOLERANCE_LU, "{worst}");
}

#[test]
fn rendered_targets_agree_with_ebur128() {
    let mut worst = 0.0f64;
    for level in 1..=5u8 {
        for seed in [1u64, 2] {
            let spec = SpecPreset::by_id(level).unwrap().spec(1, seed);
            let g = random::target(&spec).unwrap();
            let mid = render(&g.patch, &g.phrase().performance(), &RenderSpec::default())
                .unwrap()
                .mid();
            worst = worst.max(agree(&format!("level {level} seed {seed}"), &mid));
        }
    }
    eprintln!("worst |ours - ebur128|: {worst:e} LU");
    assert!(worst < TOLERANCE_LU, "{worst}");
}

#[test]
fn short_clips_fall_back_to_the_whole_clip() {
    // Under one 400 ms block neither gate applies: both read the ungated clip (ebur128 through its window).
    let blip = sine(SR, 997.0, 0.5, 0.2);
    let (ours, theirs) = both(&blip);
    assert!(
        (ours.unwrap() - theirs.unwrap()).abs() < 0.05,
        "{ours:?} {theirs:?}"
    );
}
