//! Unit tests of the reverb core that need its internals. The bit-exact C++ comparison, the allocation test and the
//! click test are in `tests/reverb.rs` (a separate binary, because they install a global allocator).

use proptest::prelude::*;

use super::*;

const SR: f32 = 48_000.0;

/// Deterministic noise in `[-amp, amp]` (`SplitMix64` would do; this keeps the test self-contained).
fn noise(seed: u64, frames: usize, amp: f32) -> Vec<f32> {
    let mut s = seed;
    (0..frames)
        .map(|_| {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 40) as f32 / 16_777_216.0 - 0.5) * 2.0 * amp
        })
        .collect()
}

fn run(reverb: &mut Reverb, in_l: &[f32], in_r: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut out_l = vec![0.0f32; in_l.len()];
    let mut out_r = vec![0.0f32; in_l.len()];
    let mut pos = 0;
    while pos < in_l.len() {
        let end = (pos + block).min(in_l.len());
        reverb.process(
            &in_l[pos..end],
            &in_r[pos..end],
            &mut out_l[pos..end],
            &mut out_r[pos..end],
        );
        pos = end;
    }
    (out_l, out_r)
}

// --- memory and construction -----------------------------------------------------------------------------------

#[test]
fn allocation_is_sized_from_the_sample_rate_alone() {
    // 409,600 floats at 48 kHz: six multi-delays rounded up to a power of two (P1a report 02).
    let mut reverb = Reverb::prepare(48_000.0);
    assert_eq!(reverb.allocated_bytes(), 1_638_400);
    let before = reverb.allocated_bytes();
    reverb.set_params(&ReverbParams::MAX);
    let (l, r) = (noise(1, 4800, 0.5), noise(2, 4800, 0.5));
    run(&mut reverb, &l, &r, 32);
    reverb.set_params(&ReverbParams::MIN);
    run(&mut reverb, &l, &r, 32);
    assert_eq!(reverb.allocated_bytes(), before);
    assert!(Reverb::prepare(96_000.0).allocated_bytes() > before);
    assert_eq!(Reverb::prepare(44_100.0).allocated_bytes(), before);
}

#[test]
fn dry_only_is_the_identity() {
    let mut reverb = Reverb::prepare(SR);
    reverb.set_params(&ReverbParams {
        dry_pct: 100.0,
        wet_pct: 0.0,
        ..ReverbParams::default()
    });
    let (mut l, r) = (noise(51, 4800, 1.0), noise(52, 4800, 1.0));
    l[10] = -0.0;
    let (out_l, out_r) = run(&mut reverb, &l, &r, 32);
    for (x, y) in l.iter().zip(&out_l).chain(r.iter().zip(&out_r)) {
        assert!(
            x.to_bits() == y.to_bits() || (*x == 0.0 && *y == 0.0),
            "{x} became {y}"
        );
    }
    assert_eq!(out_l[10].to_bits(), 0.0f32.to_bits(), "-0.0 + 0.0 is +0.0");
}

#[test]
fn sample_rate_is_clamped_and_nan_falls_back() {
    let rate = |sr: f32| Reverb::prepare(sr).sample_rate().to_bits();
    assert_eq!(rate(1.0), MIN_SAMPLE_RATE.to_bits());
    assert_eq!(rate(1e9), MAX_SAMPLE_RATE.to_bits());
    assert_eq!(rate(f32::NAN), 48_000.0f32.to_bits());
    assert_eq!(rate(f32::NEG_INFINITY), MIN_SAMPLE_RATE.to_bits());
    assert_eq!(rate(44_100.0), 44_100.0f32.to_bits());
}

#[test]
fn zero_latency_and_tail_length() {
    let mut reverb = Reverb::prepare(SR);
    assert_eq!(reverb.latency_samples(), 0);
    assert_eq!(reverb.tail_samples(), 144_000); // rt20 1 s * 3 * 48 kHz
    reverb.set_params(&ReverbParams {
        rt20_s: 2.5,
        ..ReverbParams::default()
    });
    assert_eq!(reverb.tail_samples(), 360_000);
    // Zero latency: an impulse is heard in the very first output samples (the dry path), and the wet path starts
    // within a few milliseconds, not after a look-ahead.
    let mut reverb = Reverb::prepare(SR);
    let mut impulse = vec![0.0f32; 480];
    impulse[0] = 1.0;
    let zeros = vec![0.0f32; 480];
    let (l, _) = run(&mut reverb, &impulse, &zeros, 32);
    assert_eq!(l[0], 1.0, "dry at unity gain, in the same sample");
}

#[test]
fn out_of_range_parameters_are_clamped_and_nan_is_the_minimum() {
    let mut reverb = Reverb::prepare(SR);
    reverb.set_params(&ReverbParams {
        dry_pct: 1e9,
        wet_pct: -5.0,
        room_ms: 5_000.0,
        rt20_s: f64::NAN,
        early_pct: f64::INFINITY,
        detune: f64::NEG_INFINITY,
        low_cut_hz: 0.0,
        high_cut_hz: 1e9,
        low_damp_rate: f64::NAN,
        high_damp_rate: 99.0,
    });
    let p = &reverb.params;
    assert_eq!(p[DRY].current, 4.0);
    assert_eq!(p[WET].current, 0.0);
    assert_eq!(p[ROOM].current, 200.0); // upstream does not clamp: a bigger room would read other channels' data
    assert_eq!(p[RT20].current, 0.01);
    assert_eq!(p[EARLY].current, 2.5);
    assert_eq!(p[DETUNE].current, 0.0);
    assert_eq!(p[LOW_CUT].current, 10.0);
    assert_eq!(p[HIGH_CUT].current, 20_000.0);
    assert_eq!(p[LOW_DAMP].current, 1.0);
    assert_eq!(p[HIGH_DAMP].current, 10.0);
    assert_eq!(ReverbParams::default().clamped(), ReverbParams::default());
}

// --- the fade bookkeeping ---------------------------------------------------------------------------------------

#[test]
fn parameters_set_before_the_first_block_apply_at_once() {
    let mut reverb = Reverb::prepare(SR);
    let p = ReverbParams {
        wet_pct: 100.0,
        room_ms: 150.0,
        ..ReverbParams::default()
    };
    reverb.set_params(&p);
    assert!(
        reverb
            .params
            .iter()
            .all(|q| q.from == q.to && q.to == q.current)
    );
    let (l, r) = (noise(5, 32, 0.5), noise(6, 32, 0.5));
    run(&mut reverb, &l, &r, 32);
    assert_eq!(reverb.fade_ratio, 0.0, "no fade ran");
    // After the first block a change does start one.
    reverb.set_params(&ReverbParams::default());
    run(&mut reverb, &l, &r, 32);
    assert!(reverb.fade_ratio > 0.0);
}

#[test]
fn a_fade_lasts_twenty_milliseconds_whatever_the_block_length() {
    for block in [32usize, 64, 100, 480] {
        let mut reverb = Reverb::prepare(SR);
        let (l, r) = (noise(7, 4800, 0.5), noise(8, 4800, 0.5));
        run(&mut reverb, &l[..block], &r[..block], block);
        reverb.set_params(&ReverbParams {
            room_ms: 120.0,
            ..ReverbParams::default()
        });
        let mut blocks = 0;
        loop {
            run(&mut reverb, &l[..block], &r[..block], block);
            blocks += 1;
            if reverb.fade_ratio == 0.0 {
                break;
            }
            assert!(blocks < 1000);
        }
        // 960 samples of fade: ceil(960 / block) blocks, or one more when the running sum of the per-block
        // ratios (n / 960 added over and over) lands a hair below 1.0, as it does for 30 blocks of 32.
        let ideal = 960_usize.div_ceil(block);
        assert!(
            (ideal..=ideal + 1).contains(&blocks),
            "block {block}: {blocks} blocks, ideal {ideal}"
        );
    }
}

#[test]
fn a_change_during_a_fade_waits_for_the_next_one() {
    let mut reverb = Reverb::prepare(SR);
    let (l, r) = (noise(9, 32, 0.5), noise(10, 32, 0.5));
    run(&mut reverb, &l, &r, 32);
    reverb.set_params(&ReverbParams {
        room_ms: 150.0,
        ..ReverbParams::default()
    });
    run(&mut reverb, &l, &r, 32);
    let to = reverb.params[WET].to;
    reverb.set_params(&ReverbParams {
        room_ms: 150.0,
        wet_pct: 100.0,
        ..ReverbParams::default()
    });
    run(&mut reverb, &l, &r, 32);
    assert_eq!(
        reverb.params[WET].to, to,
        "the running fade keeps its end point"
    );
    for _ in 0..40 {
        run(&mut reverb, &l, &r, 32);
    }
    assert_eq!(
        reverb.params[WET].to, 1.0,
        "the next fade picked the new value up"
    );
}

// --- reset ------------------------------------------------------------------------------------------------------

#[test]
fn reset_clears_all_six_delay_lines_including_delay4() {
    let mut reverb = Reverb::prepare(SR);
    reverb.set_params(&ReverbParams {
        dry_pct: 0.0,
        ..ReverbParams::default()
    });
    let (l, r) = (noise(11, 9600, 0.5), noise(12, 9600, 0.5));
    run(&mut reverb, &l, &r, 32);
    for (i, d) in reverb.delays().iter().enumerate() {
        assert!(
            d.samples().iter().any(|x| *x != 0.0),
            "delay {i} should hold audio before the reset"
        );
    }
    reverb.reset();
    for (i, d) in reverb.delays().iter().enumerate() {
        assert!(
            d.samples().iter().all(|x| *x == 0.0),
            "delay {i} not cleared"
        );
    }
    // The next block starts from silence: silence in gives silence out, for as long as you like.
    let zeros = vec![0.0f32; 9600];
    let (out_l, out_r) = run(&mut reverb, &zeros, &zeros, 32);
    assert!(out_l.iter().chain(&out_r).all(|x| *x == 0.0));
}

#[test]
fn reset_makes_the_render_repeatable_bit_for_bit() {
    let params = ReverbParams {
        room_ms: 130.0,
        detune: 15.0,
        ..ReverbParams::default()
    };
    let (l, r) = (noise(13, 24_000, 0.5), noise(14, 24_000, 0.5));
    let mut reverb = Reverb::prepare(SR);
    reverb.set_params(&params);
    let first = run(&mut reverb, &l, &r, 32);
    // Wander through other settings, then reset and come back.
    reverb.set_params(&ReverbParams::MAX);
    run(&mut reverb, &r, &l, 32);
    reverb.set_params(&params);
    reverb.reset();
    let second = run(&mut reverb, &l, &r, 32);
    assert!(
        first
            .0
            .iter()
            .zip(&second.0)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert!(
        first
            .1
            .iter()
            .zip(&second.1)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    // And equal to a fresh instance.
    let mut fresh = Reverb::prepare(SR);
    fresh.set_params(&params);
    let third = run(&mut fresh, &l, &r, 32);
    assert!(
        first
            .0
            .iter()
            .zip(&third.0)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn reset_cancels_a_running_fade() {
    let mut reverb = Reverb::prepare(SR);
    let (l, r) = (noise(15, 32, 0.5), noise(16, 32, 0.5));
    run(&mut reverb, &l, &r, 32);
    reverb.set_params(&ReverbParams::MAX);
    run(&mut reverb, &l, &r, 32);
    assert!(reverb.fade_ratio > 0.0);
    reverb.reset();
    assert_eq!(reverb.fade_ratio, 0.0);
    assert!(
        reverb
            .params
            .iter()
            .all(|q| q.from == q.to && q.to == q.current)
    );
    run(&mut reverb, &l, &r, 32);
    assert_eq!(
        reverb.fade_ratio, 0.0,
        "the first block after a reset does not fade"
    );
}

// --- denormals --------------------------------------------------------------------------------------------------

/// Feed a 1 s noise burst, then 60 s of silence in engine-sized blocks; the output and all internal state must
/// never hold a subnormal, and the tail must reach exact zero.
fn assert_tail_without_subnormals(
    sample_rate: f32,
    params: &ReverbParams,
    silence_seconds: usize,
) -> Option<usize> {
    let sr = sample_rate as usize;
    let mut reverb = Reverb::prepare(sample_rate);
    reverb.set_params(params);
    let (burst_l, burst_r) = (noise(21, sr, 0.5), noise(22, sr, 0.5));
    let (silence, mut out_l, mut out_r) = (vec![0.0f32; 32], vec![0.0f32; 32], vec![0.0f32; 32]);
    run(&mut reverb, &burst_l, &burst_r, 32);
    let mut last_nonzero = None;
    let mut subnormal_outputs = 0usize;
    for second in 0..silence_seconds {
        for block in 0..(sr / 32) {
            reverb.process(&silence, &silence, &mut out_l, &mut out_r);
            for (i, v) in out_l.iter().chain(&out_r).enumerate() {
                subnormal_outputs += usize::from(v.is_subnormal());
                if *v != 0.0 {
                    last_nonzero = Some(second * sr + block * 32 + i % 32);
                }
            }
        }
        assert_eq!(
            reverb.count_subnormals(),
            0,
            "subnormal state after {second} s of tail"
        );
    }
    assert_eq!(subnormal_outputs, 0);
    last_nonzero
}

#[test]
fn a_61_second_tail_has_no_subnormals_and_ends_in_exact_zero() {
    let last =
        assert_tail_without_subnormals(48_000.0, &ReverbParams::default(), 61).expect("a tail");
    // Measured in P1a report 02: exactly zero from 15.1 s with these parameters.
    assert!(last < 48_000 * 20, "tail still going at sample {last}");
}

#[test]
fn tails_without_subnormals_at_other_rates_and_settings() {
    let ambient = ReverbParams {
        wet_pct: 85.0,
        rt20_s: 8.5,
        early_pct: 55.0,
        detune: 8.5,
        low_cut_hz: 50.0,
        high_cut_hz: 7_200.0,
        high_damp_rate: 2.0,
        ..ReverbParams::default()
    };
    assert_tail_without_subnormals(44_100.0, &ambient, 30);
    let short = ReverbParams {
        room_ms: 10.0,
        rt20_s: 0.05,
        ..ReverbParams::default()
    };
    let last = assert_tail_without_subnormals(96_000.0, &short, 3).expect("a tail");
    assert!(
        last < 96_000,
        "a 50 ms RT20 is silent well within a second, but ran to {last}"
    );
}

#[test]
fn subnormal_input_does_not_reach_the_delay_lines() {
    let mut reverb = Reverb::prepare(SR);
    reverb.set_params(&ReverbParams {
        dry_pct: 0.0,
        ..ReverbParams::default()
    });
    let tiny = vec![f32::MIN_POSITIVE / 64.0; 48_000];
    let (out_l, out_r) = run(&mut reverb, &tiny, &tiny, 32);
    assert_eq!(reverb.count_subnormals(), 0);
    assert!(
        out_l.iter().chain(&out_r).all(|x| *x == 0.0),
        "a signal below -300 dB is dropped"
    );
}

// --- delay ranges -----------------------------------------------------------------------------------------------

/// Reads must stay inside each channel's own region of the ring, or they would hear another channel's data (that is
/// why upstream's unclamped `roomMs` above `maxRoomMs` is dangerous). Check every stage at the extreme rooms and
/// rates, including the detuned feedback read: lfo depth + offset + the 7 taps of the interpolator.
#[test]
fn every_read_stays_inside_its_channel_for_all_rooms_and_rates() {
    for sample_rate in [
        8_000.0f32, 22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0, 768_000.0,
    ] {
        for room_ms in [10.0, 10.5, 37.3, 80.0, 123.4, 199.9, 200.0] {
            let mut reverb = Reverb::prepare(sample_rate);
            reverb.update_delays(room_ms * 0.001 * f64::from(sample_rate));
            let depth = reverb.detune_depth_samples.ceil() as i32;
            let stages = [
                (&reverb.delay1, 0),
                (&reverb.delay2, 0),
                (&reverb.delay3, 0),
                (&reverb.delay4, 0),
                (&reverb.delay_early, 0),
                (&reverb.delay_feedback, depth + 8), // detune LFO and the 8-tap interpolator
            ];
            for (n, (stage, extra)) in stages.iter().enumerate() {
                let furthest = stage.offsets().iter().map(|o| -o).max().unwrap() + extra;
                assert!(
                    furthest < stage.capacity(),
                    "stage {n} at {sample_rate} Hz, room {room_ms} ms reads {furthest} samples back of {}",
                    stage.capacity()
                );
                assert!(stage.offsets().iter().all(|o| *o <= 0));
            }
        }
    }
}

// --- robustness -------------------------------------------------------------------------------------------------

/// The module does not sanitise: a NaN in the input recirculates in the feedback until `reset()`. The doc says so;
/// the engine's output stage is where non-finite samples get caught.
#[test]
fn a_nan_input_poisons_the_tail_until_reset() {
    let mut reverb = Reverb::prepare(SR);
    let (mut l, r) = (noise(61, 4800, 0.5), noise(62, 4800, 0.5));
    l[100] = f32::NAN;
    let (out_l, _) = run(&mut reverb, &l, &r, 32);
    assert!(out_l.last().unwrap().is_nan());
    reverb.reset();
    let (clean_l, clean_r) = (noise(63, 4800, 0.5), noise(64, 4800, 0.5));
    let (out_l, out_r) = run(&mut reverb, &clean_l, &clean_r, 32);
    assert!(out_l.iter().chain(&out_r).all(|x| x.is_finite()));
}

#[test]
fn mismatched_slice_lengths_process_the_shortest() {
    let mut reverb = Reverb::prepare(SR);
    let (l, r) = (noise(31, 100, 0.5), noise(32, 60, 0.5));
    let mut out_l = vec![9.0f32; 100];
    let mut out_r = vec![9.0f32; 80];
    reverb.process(&l, &r, &mut out_l, &mut out_r);
    assert!(out_l[..60].iter().all(|x| *x != 9.0));
    assert!(out_l[60..].iter().all(|x| *x == 9.0));
    assert!(out_r[60..].iter().all(|x| *x == 9.0));
    // Empty blocks do nothing, in particular they do not start or finish a fade.
    reverb.process(&[], &[], &mut [], &mut []);
    reverb.process_in_place(&mut [], &mut []);
}

#[test]
fn a_block_longer_than_the_fade_finishes_it_exactly() {
    let mut reverb = Reverb::prepare(SR);
    let (l, r) = (noise(33, 8192, 0.5), noise(34, 8192, 0.5));
    run(&mut reverb, &l[..32], &r[..32], 32);
    reverb.set_params(&ReverbParams::MAX);
    run(&mut reverb, &l, &r, 8192);
    assert_eq!(reverb.fade_ratio, 0.0);
    assert!(
        reverb
            .params
            .iter()
            .all(|q| q.from == q.to || q.to == q.current)
    );
}

/// Chunking only matters while parameters move: with steady parameters the output was bit-identical for every block
/// length measured, and could differ at most by the rounding of the LFO phase's wrap. A tolerance keeps the test
/// robust to that; the module docs quote the measurement.
#[test]
fn steady_state_output_hardly_depends_on_the_block_length() {
    let params = ReverbParams {
        detune: 20.0,
        ..ReverbParams::default()
    };
    let (l, r) = (noise(41, 96_000, 0.5), noise(42, 96_000, 0.5));
    let render = |block: usize| {
        let mut reverb = Reverb::prepare(SR);
        reverb.set_params(&params);
        run(&mut reverb, &l, &r, block)
    };
    let (a, b) = (render(32), render(100));
    let peak = a.0.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let worst =
        a.0.iter()
            .zip(&b.0)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
    assert!(peak > 0.1);
    assert!(
        worst < 1e-6 * peak,
        "worst difference {worst} against peak {peak}"
    );
}

proptest! {
    /// Any parameters (even out of range or not finite), any block lengths, parameters changed between blocks: the
    /// output stays finite and bounded.
    #[test]
    fn output_is_finite_and_bounded_for_any_parameters(
        sets in prop::collection::vec(
            (-50.0f64..500.0, -50.0f64..500.0, -10.0f64..300.0, -1.0f64..40.0, -10.0f64..300.0,
             -5.0f64..60.0, -10.0f64..800.0, 0.0f64..30_000.0, -2.0f64..12.0, -2.0f64..12.0),
            1..4,
        ),
        blocks in prop::collection::vec(1usize..400, 1..12),
        seed in any::<u64>(),
    ) {
        let mut reverb = Reverb::prepare(SR);
        let mut at = 0;
        for (k, s) in sets.iter().enumerate() {
            reverb.set_params(&ReverbParams {
                dry_pct: s.0, wet_pct: s.1, room_ms: s.2, rt20_s: s.3, early_pct: s.4, detune: s.5,
                low_cut_hz: s.6, high_cut_hz: s.7, low_damp_rate: s.8, high_damp_rate: s.9,
            });
            for &n in &blocks {
                let l = noise(seed.wrapping_add(at), n, 1.0);
                let r = noise(seed.wrapping_add(at + 1), n, 1.0);
                let mut out_l = vec![0.0f32; n];
                let mut out_r = vec![0.0f32; n];
                reverb.process(&l, &r, &mut out_l, &mut out_r);
                at += 2;
                prop_assert!(out_l.iter().chain(&out_r).all(|x| x.is_finite() && x.abs() < 1.0e4),
                    "set {k}, block {n}: {out_l:?}");
            }
        }
    }
}
