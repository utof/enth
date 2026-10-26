//! `delay` through the engine (S2a lane DLY; `docs/dsp/delay.md`): the exact echo times across host block sizes and
//! tempo changes, the 2 s clamp, how a time change moves the read head, the decay of the repeats, ping-pong, `mix` 0
//! as a bit-exact bypass, two instances, the bound at 100 % feedback, the plan swaps (an instant one that moves the
//! rings and a crossfading one that copies them), the 40 s tail with zero subnormals, the render under every FTZ
//! mode, `process` under the no-allocation guard and the spectral aliasing bounds. The response of the core itself
//! (the soft clip, the filters, the crossfade, the idle path) is in `enth_dsp::delay`'s and the module's unit tests.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{Note, SEED, SR, bits, drive, events, id, note, reset_saw_patch};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep};
use enth_engine::module::{SEG, count_subnormals};
use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, PlanMap, PrepareCfg, compile};
use enth_patch::catalogue::{AdsrParam, Beats, DelayParam, OscParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

const SR_I: u64 = 48_000;

/// The longest delay at 48 kHz, in samples.
const MAX_DELAY: u64 = 2 * SR_I;

/// Set a `delay1` value by its parameter.
fn set(p: &mut Patch, param: DelayParam, value: f32) {
    let m =
        p.fx.iter_mut()
            .find(|m| m.id.as_str() == "delay1")
            .expect("the patch has a delay1");
    m.values[param.index()] = value;
}

/// A reset-phase saw through an envelope that makes a burst (no attack, a 3 ms decay to nothing, no release) into one
/// `delay1`, `sync` on with `beats` and the given feedback (percent); `mix` 100, the filters at their defaults.
/// The dry signal is silent after the burst, so every echo stands alone in silence.
fn burst_patch(beats: Beats, feedback: f32) -> Patch {
    let mut p = reset_saw_patch();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 3.0;
    env[AdsrParam::Sustain.index()] = 0.0;
    env[AdsrParam::Release.index()] = 0.0;
    p.fx.push(ModuleInstance::new(id("delay1"), ModuleKind::Delay));
    set(&mut p, DelayParam::Beats, beats.index() as f32);
    set(&mut p, DelayParam::Feedback, feedback);
    p
}

/// The same patch without its delay: the dry signal.
fn dry_of(p: &Patch) -> Patch {
    let mut d = p.clone();
    d.fx.clear();
    d
}

/// What a render is made of: notes, tempo changes and `delay1` parameter changes, all at absolute frames.
#[derive(Clone, Default)]
struct Script {
    notes: Vec<Note>,
    /// `(frame, bpm)`.
    tempo: Vec<(u64, f64)>,
    /// `(frame, delay1 parameter, plain value)`.
    params: Vec<(u64, DelayParam, f32)>,
}

impl Script {
    fn events(&self, map: &PlanMap) -> Vec<(u64, EventKind)> {
        let mut ev = events(&self.notes);
        for &(f, bpm) in &self.tempo {
            ev.push((f, EventKind::Tempo { bpm }));
        }
        for &(f, param, value) in &self.params {
            // A patch without the delay (the dry reference) has nothing to address: the event is dropped.
            let addr = format!("delay1.{}", param.key()).parse().unwrap();
            if let Some(id) = map.get(&addr) {
                ev.push((f, EventKind::Param { id, value }));
            }
        }
        ev.sort_by_key(|(f, _)| *f);
        ev
    }
}

/// Render `script` through `patch` in host blocks of `block`: (left, right, the engine afterwards).
fn run(
    patch: &Patch,
    script: &Script,
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>, Engine) {
    let (plan, map) = compile(patch, &PrepareCfg::new(SR)).expect("the patch compiles");
    let mut e = Engine::new(plan, SEED);
    let ev = script.events(&map);
    let (l, r) = drive(&mut e, &ev, frames, block, |_, _, _| {});
    (l, r, e)
}

/// The index of the first sample at or after `from` that is not exactly zero.
fn first_nonzero(x: &[f32], from: usize) -> Option<usize> {
    x.iter()
        .enumerate()
        .skip(from)
        .find(|(_, v)| **v != 0.0)
        .map(|(i, _)| i)
}

/// `a − b`, sample by sample.
fn minus(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x - y).collect()
}

/// The exact delay in samples of `num/den` beats at `bpm_num/bpm_den` beats per minute at 48 kHz, in integers:
/// `round(num·60·sr·bpm_den / (den·bpm_num))`, half up, clamped to 2 s. An independent statement of the rounding
/// rule that does not go through floating point.
fn exact_beats(num: u64, den: u64, bpm_num: u64, bpm_den: u64) -> u64 {
    let n = num * 60 * SR_I * bpm_den;
    let d = den * bpm_num;
    ((2 * n + d) / (2 * d)).min(MAX_DELAY)
}

#[test]
fn the_reference_rounding_matches_the_documented_examples() {
    // A quarter at 120 BPM is half a second; at 140 BPM 20,571.43 samples round to 20,571; a dotted eighth (3/4 of
    // a beat) at 140 is 15,428.57, which rounds up to 15,429; at 20 BPM a bar is clamped to 2 s.
    assert_eq!(exact_beats(1, 1, 120, 1), 24_000);
    assert_eq!(exact_beats(1, 1, 140, 1), 20_571);
    assert_eq!(exact_beats(3, 4, 140, 1), 15_429);
    assert_eq!(exact_beats(3, 2, 140, 1), 30_857);
    assert_eq!(exact_beats(4, 1, 20, 1), MAX_DELAY);
    assert_eq!(exact_beats(1, 1, 247, 2), 23_320);
}

/// The host block sizes the acceptance names: one frame, a power of two, one that divides nothing, a long one.
const BLOCKS: [usize; 4] = [1, 64, 100, 4096];

/// Where the burst and its first two echoes land, as offsets from the dry burst's first sample, in a render of
/// `script`. The burst is measured in the dry render; the first echo is the first sample that differs between the
/// dry render and the render through a delay with feedback 0; the second is the first that differs between feedback
/// 0 and 35 %. Every comparison is of exact bits, so a sample of error is a failure. Also returns the two renders'
/// bits so the host block sizes can be compared.
struct Landing {
    echo1: usize,
    echo2: usize,
    left: Vec<u32>,
    right: Vec<u32>,
}

fn landing(beats: Beats, script: &Script, frames: usize, block: usize) -> Landing {
    let (p0, p1) = (burst_patch(beats, 0.0), burst_patch(beats, 35.0));
    let (dry, _, _) = run(&dry_of(&p0), script, frames, block);
    let (l0, r0, _) = run(&p0, script, frames, block);
    let (l1, r1, _) = run(&p1, script, frames, block);
    let s0 = first_nonzero(&dry, 0).expect("the burst sounds");
    let echo1 = first_nonzero(&minus(&l0, &dry), s0).expect("a first echo");
    let echo2 = first_nonzero(&minus(&l1, &l0), s0).expect("a second echo");
    // A centred input makes the two channels one signal: the right channel lands on the same samples.
    assert_eq!(bits(&l0), bits(&r0), "block {block}");
    assert_eq!(bits(&l1), bits(&r1), "block {block}");
    Landing {
        echo1: echo1 - s0,
        echo2: echo2 - s0,
        left: bits(&l1),
        right: bits(&r1),
    }
}

/// One scenario: the echoes land on `d1` and `d1 + d2` samples after the burst (`d2` is the delay the second echo's
/// repeat is read at), at every host block size, and the four renders are the same bits.
fn assert_lands(name: &str, beats: Beats, script: &Script, frames: usize, d1: u64, d2: u64) {
    let mut reference: Option<Landing> = None;
    for block in BLOCKS {
        let got = landing(beats, script, frames, block);
        assert_eq!(got.echo1 as u64, d1, "{name}: first echo at block {block}");
        assert_eq!(
            got.echo2 as u64,
            d1 + d2,
            "{name}: second echo at block {block}"
        );
        if let Some(r) = &reference {
            assert!(
                r.left == got.left && r.right == got.right,
                "{name}: the render changes with the host block {block}"
            );
        } else {
            reference = Some(got);
        }
    }
}

fn one_note(frame: u64) -> Vec<Note> {
    vec![note(60, frame, frame + 5_000)]
}

#[test]
fn echoes_land_on_the_exact_sample_at_every_host_block_size() {
    let d = |bpm: u32| exact_beats(1, 1, u64::from(bpm), 1);
    // A steady tempo: the burst is quantised to the segment grid (frame 1000 is 1024), the echoes are exact.
    let steady = Script {
        notes: one_note(1_000),
        tempo: vec![(0, 240.0)],
        ..Script::default()
    };
    assert_lands("steady", Beats::Quarter, &steady, 40_000, d(240), d(240));

    // The tempo changes at an odd frame while the delay is idle (it applies at the next segment boundary and the
    // note comes later), once to a faster and once to a slower tempo.
    for (name, bpm) in [("faster", 360), ("slower", 150)] {
        let s = Script {
            notes: one_note(30_000),
            tempo: vec![(0, 240.0), (20_017, f64::from(bpm))],
            ..Script::default()
        };
        assert_lands(name, Beats::Quarter, &s, 80_000, d(bpm), d(bpm));
    }

    // The tempo changes while the first echo is in flight (the ring is busy, so the read position fades): the first
    // echo is at the old time, and the repeat the first echo left in the ring is read at the new time after it
    // (docs/dsp/delay.md, "A time change moves the read head").
    for (name, bpm) in [("faster in flight", 360), ("slower in flight", 120)] {
        let s = Script {
            notes: one_note(1_000),
            tempo: vec![(0, 240.0), (16_017, f64::from(bpm))],
            ..Script::default()
        };
        assert_lands(name, Beats::Quarter, &s, 80_000, d(240), d(bpm));
    }
}

#[test]
fn every_synced_division_lands_on_its_exact_sample() {
    // The tempo is not a whole number and not a round one, so most divisions land on a rounded fraction.
    let table: [(Beats, u64, u64); 18] = [
        (Beats::SixtyFourth, 1, 16),
        (Beats::ThirtySecondTriplet, 1, 12),
        (Beats::ThirtySecond, 1, 8),
        (Beats::SixteenthTriplet, 1, 6),
        (Beats::ThirtySecondDotted, 3, 16),
        (Beats::Sixteenth, 1, 4),
        (Beats::EighthTriplet, 1, 3),
        (Beats::SixteenthDotted, 3, 8),
        (Beats::Eighth, 1, 2),
        (Beats::QuarterTriplet, 2, 3),
        (Beats::EighthDotted, 3, 4),
        (Beats::Quarter, 1, 1),
        (Beats::HalfTriplet, 4, 3),
        (Beats::QuarterDotted, 3, 2),
        (Beats::Half, 2, 1),
        (Beats::BarTriplet, 8, 3),
        (Beats::HalfDotted, 3, 1),
        (Beats::Bar, 4, 1),
    ];
    // 133.7 BPM = 1337/10. A bar is 1.7951 s, inside the 2 s ring.
    for (beats, num, den) in table {
        let want = exact_beats(num, den, 1_337, 10);
        let script = Script {
            notes: one_note(100),
            tempo: vec![(0, 133.7)],
            ..Script::default()
        };
        let frames = want as usize + 3_000;
        let p0 = burst_patch(beats, 0.0);
        let (dry, _, _) = run(&dry_of(&p0), &script, frames, 100);
        let (wet, _, _) = run(&p0, &script, frames, 100);
        let s0 = first_nonzero(&dry, 0).unwrap();
        let echo = first_nonzero(&minus(&wet, &dry), s0).unwrap();
        assert_eq!((echo - s0) as u64, want, "{}", beats.key());
    }
}

#[test]
fn a_time_in_milliseconds_lands_on_its_rounded_sample() {
    for ms in [
        1.0f32, 2.5, 33.3, 100.0, 123.4, 300.0, 751.7, 1_999.9, 2_000.0,
    ] {
        let want = (f64::from(ms) * 48.0).round() as u64;
        let mut p = burst_patch(Beats::Quarter, 0.0);
        set(&mut p, DelayParam::Sync, 0.0);
        set(&mut p, DelayParam::Time, ms);
        let script = Script {
            notes: one_note(100),
            ..Script::default()
        };
        let frames = want as usize + 3_000;
        let (dry, _, _) = run(&dry_of(&p), &script, frames, 64);
        let (wet, _, _) = run(&p, &script, frames, 64);
        let s0 = first_nonzero(&dry, 0).unwrap();
        let echo = first_nonzero(&minus(&wet, &dry), s0).unwrap();
        assert_eq!((echo - s0) as u64, want, "{ms} ms");
    }
}

#[test]
fn a_synced_time_past_two_seconds_plays_at_two_seconds() {
    // A bar is 2 s at 120 BPM exactly, and longer below it. 119.9 BPM is 2.0017 s; 60 BPM is 4 s; 20 BPM is 12 s.
    for (bpm, clamped) in [(120.0, false), (119.9, true), (60.0, true), (20.0, true)] {
        let script = Script {
            notes: one_note(100),
            tempo: vec![(0, bpm)],
            ..Script::default()
        };
        let p0 = burst_patch(Beats::Bar, 0.0);
        let frames = 100_000 + 3_000;
        let (dry, _, _) = run(&dry_of(&p0), &script, frames, 4096);
        let (wet, _, _) = run(&p0, &script, frames, 4096);
        let s0 = first_nonzero(&dry, 0).unwrap();
        let echo = first_nonzero(&minus(&wet, &dry), s0).unwrap();
        assert_eq!((echo - s0) as u64, MAX_DELAY, "{bpm} BPM");
        let _ = clamped;
    }
}

#[test]
fn a_time_set_by_automation_moves_the_read_head_like_a_tempo_change() {
    // Sync off, 100 ms (4,800 samples); while the first echo is in flight the time becomes 250 ms (12,000): the
    // repeat the first echo left in the ring is read 12,000 samples after it was written.
    let mut base = burst_patch(Beats::Quarter, 0.0);
    set(&mut base, DelayParam::Sync, 0.0);
    set(&mut base, DelayParam::Time, 100.0);
    let script = Script {
        notes: one_note(1_000),
        params: vec![(6_300, DelayParam::Time, 250.0)],
        ..Script::default()
    };
    let mut p1 = base.clone();
    set(&mut p1, DelayParam::Feedback, 35.0);
    for block in BLOCKS {
        let (dry, _, _) = run(&dry_of(&base), &script, 40_000, block);
        let (l0, _, _) = run(&base, &script, 40_000, block);
        let (l1, _, _) = run(&p1, &script, 40_000, block);
        let s0 = first_nonzero(&dry, 0).unwrap();
        let e1 = first_nonzero(&minus(&l0, &dry), s0).unwrap() - s0;
        let e2 = first_nonzero(&minus(&l1, &l0), s0).unwrap() - s0;
        assert_eq!((e1, e2), (4_800, 4_800 + 12_000), "block {block}");
    }
}

/// A sine burst (sustain at full level, no release) held for `hold` frames from frame 0, into one `delay1` in ms
/// mode with the loop filters wide open.
fn sine_burst_patch(time_ms: f32, feedback: f32, ping_pong: bool) -> Patch {
    let mut p = burst_patch(Beats::Quarter, feedback);
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    p.modulators[0].values[AdsrParam::Sustain.index()] = 100.0;
    p.modulators[0].values[AdsrParam::Decay.index()] = 0.0;
    set(&mut p, DelayParam::Sync, 0.0);
    set(&mut p, DelayParam::Time, time_ms);
    set(&mut p, DelayParam::LowCut, 20.0);
    set(&mut p, DelayParam::HighCut, 20_000.0);
    set(
        &mut p,
        DelayParam::PingPong,
        if ping_pong { 1.0 } else { 0.0 },
    );
    p
}

/// The peak of `x[from..to]`.
fn peak(x: &[f32], from: usize, to: usize) -> f32 {
    x[from..to].iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// The echo signal (render minus dry) of a sine burst: 100 ms of key 69 held from frame 0, so the dry signal ends
/// at 4,800 + the segment grid.
fn echoes(p: &Patch, frames: usize) -> (Vec<f32>, Vec<f32>, usize) {
    let script = Script {
        notes: vec![note(69, 0, 4_800)],
        ..Script::default()
    };
    let (dl, dr, _) = run(&dry_of(p), &script, frames, 512);
    let (wl, wr, _) = run(p, &script, frames, 512);
    let s0 = first_nonzero(&dl, 0).unwrap();
    (minus(&wl, &dl), minus(&wr, &dr), s0)
}

#[test]
fn the_repeats_decay_at_the_feedback() {
    // 300 ms (14,400 samples) apart, so the repeats of a 100 ms burst never overlap. With the loop filters wide
    // open (low cut 20 Hz, high cut 20 kHz) a 440 Hz repeat keeps 99.9 % of its level through the two one-poles, so
    // each repeat is `feedback` times the one before to within 0.4 %. Below the soft clip's knee (feedback·e < 0.5)
    // the loop is linear.
    for feedback in [20.0f32, 50.0, 75.0] {
        let p = sine_burst_patch(300.0, feedback, false);
        let (el, _, s0) = echoes(&p, 14_400 * 6);
        let window = |k: usize| {
            // The steady part of the repeat: the 20 Hz high-pass settles in about 2,500 samples (e^-6.5).
            let from = s0 + k * 14_400;
            peak(&el, from + 2_500, from + 4_500)
        };
        let first = window(1);
        assert!(
            first > 0.1,
            "{feedback}: the first echo is audible: {first}"
        );
        for k in 1..5 {
            let ratio = f64::from(window(k + 1)) / f64::from(window(k));
            let want = f64::from(feedback) / 100.0;
            eprintln!(
                "feedback {feedback} %: echo {} / echo {k} = {ratio:.5}",
                k + 1
            );
            // Never above `feedback` (the one-poles only lose level) and never more than 0.4 % below it.
            assert!(
                ratio <= want && ratio / want > 0.996,
                "feedback {feedback} %, repeat {k}: {ratio} against {want}"
            );
        }
    }
}

#[test]
fn ping_pong_alternates_the_channels() {
    // The mono dry signal enters the left ring: echo 1 is on the left alone, echo 2 on the right alone, and so on;
    // the other channel is exactly zero in each window. Without ping-pong both channels carry every echo.
    let d = 14_400;
    let window_energy = |x: &[f32], s0: usize, k: usize| -> f64 {
        x[s0 + k * d..s0 + k * d + 5_200]
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum()
    };
    let p = sine_burst_patch(300.0, 50.0, true);
    let (el, er, s0) = echoes(&p, d * 6);
    for k in 1..=5 {
        let (on, off) = if k % 2 == 1 { (&el, &er) } else { (&er, &el) };
        let e = window_energy(on, s0, k);
        assert!(e > 1.0e-3, "echo {k} sounds on its channel: {e}");
        // Not exactly zero: the other channel's filters still ring at 1e-12 from the echo before (they are flushed
        // at 1e-15, not at once); a million times below the echo is silence.
        assert!(
            window_energy(off, s0, k) < e * 1.0e-12,
            "echo {k}: the other channel"
        );
    }
    // The energy falls by feedback² per repeat across the channels too.
    let ratio = window_energy(&er, s0, 2) / window_energy(&el, s0, 1);
    assert!((ratio / 0.25 - 1.0).abs() < 0.01, "{ratio}");
    let p = sine_burst_patch(300.0, 50.0, false);
    let (el, er, s0) = echoes(&p, d * 6);
    for k in 1..=5 {
        assert!(window_energy(&el, s0, k) > 1.0e-3 && window_energy(&er, s0, k) > 1.0e-3);
        assert_eq!(bits(&el), bits(&er));
    }
}

/// A chord held for a second through the saw, with its envelope left alone, into one `delay1`.
fn chord_patch() -> Patch {
    let mut p = Patch::init();
    p.fx.push(ModuleInstance::new(id("delay1"), ModuleKind::Delay));
    p
}

fn chord() -> Vec<Note> {
    vec![
        note(48, 0, 48_000),
        note(55, 0, 48_000),
        note(60, 960, 48_000),
    ]
}

#[test]
fn mix_zero_is_the_input_bit_for_bit() {
    let script = Script {
        notes: chord(),
        ..Script::default()
    };
    let (dl, dr, _) = run(&Patch::init(), &script, 100_000, 512);
    assert!(dl.iter().any(|v| v.abs() > 0.1));
    // Every setting that could leak: full feedback, ping-pong, short and long times, filters at the extremes.
    for (sync, time, feedback, ping_pong, low, high) in [
        (1.0, 300.0, 35.0, 0.0, 150.0, 6_000.0),
        (0.0, 7.0, 100.0, 1.0, 20.0, 20_000.0),
        (0.0, 2_000.0, 100.0, 0.0, 2_000.0, 1_000.0),
        (1.0, 300.0, 100.0, 1.0, 20.0, 20_000.0),
    ] {
        let mut p = chord_patch();
        set(&mut p, DelayParam::Mix, 0.0);
        set(&mut p, DelayParam::Sync, sync);
        set(&mut p, DelayParam::Time, time);
        set(&mut p, DelayParam::Feedback, feedback);
        set(&mut p, DelayParam::PingPong, ping_pong);
        set(&mut p, DelayParam::LowCut, low);
        set(&mut p, DelayParam::HighCut, high);
        for block in [64, 4096] {
            let (l, r, _) = run(&p, &script, 100_000, block);
            assert_eq!(
                bits(&l),
                bits(&dl),
                "time {time} fb {feedback} block {block}"
            );
            assert_eq!(
                bits(&r),
                bits(&dr),
                "time {time} fb {feedback} block {block}"
            );
        }
    }
}

#[test]
fn mix_brought_to_zero_returns_to_the_input_bit_for_bit_with_echoes_still_in_the_ring() {
    // The delay runs at mix 100 and feedback 100 % (echoes circulate for ever), then `mix` goes to 0: after its
    // 10 ms smoothing the output is the dry signal again, bit for bit, though the ring is full.
    let script = Script {
        notes: chord(),
        params: vec![(30_000, DelayParam::Mix, 0.0)],
        ..Script::default()
    };
    let mut p = chord_patch();
    set(&mut p, DelayParam::Sync, 0.0);
    set(&mut p, DelayParam::Time, 20.0);
    set(&mut p, DelayParam::Feedback, 100.0);
    let (dl, dr, _) = run(&Patch::init(), &script, 100_000, 512);
    let (wl, wr, _) = run(&p, &script, 100_000, 512);
    assert_ne!(
        bits(&wl[..30_000]),
        bits(&dl[..30_000]),
        "the echoes sound while mix is 100"
    );
    assert_eq!(bits(&wl[30_000 + 1_000..]), bits(&dl[30_000 + 1_000..]));
    assert_eq!(bits(&wr[30_000 + 1_000..]), bits(&dr[30_000 + 1_000..]));
}

#[test]
fn two_instances_keep_separate_state() {
    // delay1 at 100 ms and delay2 at 250 ms in a chain, no feedback. The first echo of the chain is delay1's, at
    // 4,800; delay2 hears the dry burst at 12,000 and delay1's echo at 4,800 + 12,000.
    let mut chain = burst_patch(Beats::Quarter, 0.0);
    set(&mut chain, DelayParam::Sync, 0.0);
    set(&mut chain, DelayParam::Time, 100.0);
    let mut second = ModuleInstance::new(id("delay2"), ModuleKind::Delay);
    second.values[DelayParam::Sync.index()] = 0.0;
    second.values[DelayParam::Time.index()] = 250.0;
    second.values[DelayParam::Feedback.index()] = 0.0;
    chain.fx.push(second);
    let mut first_only = chain.clone();
    first_only.fx.truncate(1);
    let mut second_only = chain.clone();
    second_only.fx.remove(0);
    let script = Script {
        notes: one_note(1_000),
        ..Script::default()
    };
    // Script::events addresses delay1; this test sends no parameter events.
    let (dry, _, _) = run(&dry_of(&chain), &script, 40_000, 100);
    let (both, _, _) = run(&chain, &script, 40_000, 100);
    let (one, _, _) = run(&first_only, &script, 40_000, 100);
    let (two, _, _) = run(&second_only, &script, 40_000, 100);
    let s0 = first_nonzero(&dry, 0).unwrap();
    assert_eq!(first_nonzero(&minus(&one, &dry), s0).unwrap() - s0, 4_800);
    assert_eq!(first_nonzero(&minus(&two, &dry), s0).unwrap() - s0, 12_000);
    assert_eq!(first_nonzero(&minus(&both, &dry), s0).unwrap() - s0, 4_800);
    // What delay2 adds to the chain is what it adds to the dry signal plus its echo of delay1's echo; it starts
    // with its own echo of the dry burst at 12,000, not earlier (it is not fed by delay1's state).
    let extra = minus(&both, &one);
    assert_eq!(first_nonzero(&extra, s0).unwrap() - s0, 12_000);
    // And delay1 is unchanged by delay2 after it: the chain's first 12,000 samples are delay1 alone.
    assert_eq!(bits(&both[..s0 + 12_000]), bits(&one[..s0 + 12_000]));
}

const FRAMES_PER_SECOND: usize = 48_000;

/// One `delay1` in ms mode behind the chord, the loop open: for the level and tail tests.
fn loop_patch(time_ms: f32, feedback: f32, ping_pong: bool) -> Patch {
    let mut p = chord_patch();
    p.modulators[0].values[AdsrParam::Release.index()] = 200.0;
    set(&mut p, DelayParam::Sync, 0.0);
    set(&mut p, DelayParam::Time, time_ms);
    set(&mut p, DelayParam::Feedback, feedback);
    set(
        &mut p,
        DelayParam::PingPong,
        if ping_pong { 1.0 } else { 0.0 },
    );
    set(&mut p, DelayParam::LowCut, 20.0);
    set(&mut p, DelayParam::HighCut, 20_000.0);
    p
}

#[test]
fn feedback_100_percent_stays_bounded_for_40_seconds() {
    // 100 % repeats for ever. The soft clip keeps what returns to at most 1, so the ring holds at most |dry| + 1
    // and the echo at most 2·(|dry| + 1) (both loop filters have an impulse response of l1 norm at most 2), the
    // output at most 3·|dry| + 2 at mix 100. The measured peak is far below it.
    let script = Script {
        notes: chord(),
        ..Script::default()
    };
    let frames = 40 * FRAMES_PER_SECOND;
    let (dl, dr, _) = run(&Patch::init(), &script, 1, 512);
    drop((dl, dr));
    let (dry_l, dry_r, _) = run(&Patch::init(), &script, frames, 512);
    let dry_peak = peak(&dry_l, 0, frames).max(peak(&dry_r, 0, frames));
    let bound = 3.0 * dry_peak + 2.0;
    eprintln!("dry peak {dry_peak:.4}, proven bound {bound:.4}");
    for (name, p) in [
        ("1 ms", loop_patch(1.0, 100.0, false)),
        ("10 ms ping-pong", loop_patch(10.0, 100.0, true)),
        ("300 ms", loop_patch(300.0, 100.0, false)),
        ("2 s ping-pong", loop_patch(2_000.0, 100.0, true)),
    ] {
        let (l, r, e) = run(&p, &script, frames, 512);
        let sec =
            |x: &[f32], a: usize, b: usize| peak(x, a * FRAMES_PER_SECOND, b * FRAMES_PER_SECOND);
        let overall = peak(&l, 0, frames).max(peak(&r, 0, frames));
        let late = sec(&l, 20, 40).max(sec(&r, 20, 40));
        eprintln!("{name}: peak {overall:.4} over 40 s, {late:.4} over the last 20 s");
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{name}");
        assert!(overall <= bound, "{name}: {overall} against {bound}");
        // It does not grow; and where the loop is slow enough (a pass every 300 ms or more) it sustains at 40 s:
        // the two loop filters cost a few thousandths of the level per pass, which a 1 ms loop (a thousand
        // passes a second) shows as a decay within seconds.
        assert!(
            late <= sec(&l, 2, 10).max(sec(&r, 2, 10)) * 1.5 + 0.1,
            "{name}: does not grow"
        );
        if name.contains("ms") && !name.starts_with("1 ms") && !name.starts_with("10 ms")
            || name.starts_with("2 s")
        {
            assert!(late > 0.05, "{name}: the loop sustains: {late}");
        }
        assert_eq!(e.subnormals(), 0, "{name}");
        assert_eq!(count_subnormals(&l) + count_subnormals(&r), 0, "{name}");
    }
}

/// The loops whose tails die away, for the tail and FTZ tests.
fn tail_patches() -> Vec<(&'static str, Patch)> {
    let mut default = chord_patch();
    default.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    let mut tight = loop_patch(20.0, 80.0, true);
    tight.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    set(&mut tight, DelayParam::LowCut, 150.0);
    set(&mut tight, DelayParam::HighCut, 6_000.0);
    let mut slow = loop_patch(700.0, 25.0, false);
    slow.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    vec![
        ("defaults", default),
        ("20 ms 80 % ping-pong", tight),
        ("700 ms 25 %", slow),
    ]
}

fn tail_script() -> Script {
    Script {
        notes: vec![
            note(48, 0, 48_000),
            note(55, 0, 48_000),
            note(60, 960, 48_000),
        ],
        ..Script::default()
    }
}

#[test]
fn a_40_s_tail_through_delay_has_no_subnormals_and_ends_in_exact_silence() {
    for (name, p) in tail_patches() {
        let script = tail_script();
        let (plan, map) = compile(&p, &PrepareCfg::new(SR)).expect("compiles");
        let mut e = Engine::new(plan, SEED);
        let (mut sub_out, mut state_max, mut non_finite, mut calls) = (0, 0, 0, 0usize);
        let frames = 40 * FRAMES_PER_SECOND;
        let (l, r) = drive(&mut e, &script.events(&map), frames, 512, |e, l, r| {
            sub_out += count_subnormals(l) + count_subnormals(r);
            non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                state_max = state_max.max(e.subnormals());
            }
            calls += 1;
        });
        assert_eq!(sub_out, 0, "{name}: subnormal output samples");
        assert_eq!(state_max.max(e.subnormals()), 0, "{name}: engine state");
        assert_eq!(non_finite, 0, "{name}");
        assert_eq!(e.active_voices().len(), 0, "{name}: voices must end");
        // The repeats fall away and the loop goes to exact zeros: nothing sounds in the last 5 s.
        let late = frames - 5 * FRAMES_PER_SECOND;
        assert_eq!(
            peak(&l, late, frames).max(peak(&r, late, frames)),
            0.0,
            "{name}: exact silence"
        );
        assert!(l[..frames / 4].iter().any(|v| v.abs() > 0.1), "{name}");
    }
}

#[cfg(target_arch = "x86_64")]
mod fp {
    use core::arch::asm;
    pub const FTZ: u64 = 1 << 15;
    pub const DAZ: u64 = 1 << 6;
    pub const MASK: u64 = FTZ | DAZ;
    pub fn get() -> u64 {
        let mut v: u32 = 0;
        // SAFETY: stmxcsr stores the 32-bit MXCSR to the address in the register; `v` is a valid u32.
        unsafe { asm!("stmxcsr [{}]", in(reg) &raw mut v, options(nostack, preserves_flags)) };
        u64::from(v)
    }
    pub fn set(v: u64) {
        let v = v as u32;
        // SAFETY: ldmxcsr loads MXCSR from a valid u32; only the FTZ/DAZ bits differ from the current value.
        unsafe { asm!("ldmxcsr [{}]", in(reg) &raw const v, options(nostack, preserves_flags)) };
    }
    pub const MODES: [(&str, u64); 3] = [
        ("IEEE (FTZ off)", 0),
        ("FTZ only (nice-plug)", FTZ),
        ("FTZ+DAZ", FTZ | DAZ),
    ];
}

#[cfg(target_arch = "aarch64")]
mod fp {
    use core::arch::asm;
    pub const FZ: u64 = 1 << 24;
    pub const MASK: u64 = FZ;
    pub fn get() -> u64 {
        let v: u64;
        // SAFETY: reads the FPCR system register.
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nostack, preserves_flags)) };
        v
    }
    pub fn set(v: u64) {
        // SAFETY: writes the FPCR system register; only FZ differs from the current value.
        unsafe { asm!("msr fpcr, {}", in(reg) v, options(nostack)) };
    }
    pub const MODES: [(&str, u64); 2] = [("IEEE (FZ off)", 0), ("FZ on", FZ)];
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod ftz {
    use super::*;

    /// Restores the previous FTZ/DAZ bits even if the closure panics.
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }

    fn with_mode<R>(bits: u64, f: impl FnOnce() -> R) -> R {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | bits);
        // Through `black_box` before the guard restores the old mode, so the optimiser cannot sink the arithmetic
        // past the restore (tests/tail.rs).
        std::hint::black_box(f())
    }

    /// The render of every tail loop is the same under every FTZ/DAZ mode: each write into the loop and each filter
    /// state is flushed in code, so no subnormal ever reaches the arithmetic. The tempo moves during the render, so
    /// the fade runs under each mode too.
    #[test]
    fn the_render_is_the_same_under_every_ftz_mode() {
        for (name, p) in tail_patches() {
            let mut script = tail_script();
            script.tempo = vec![(0, 120.0), (60_000, 77.0)];
            let run_it = || {
                let (plan, map) = compile(&p, &PrepareCfg::new(SR)).expect("compiles");
                let mut e = Engine::new(plan, SEED);
                let (l, r) = drive(
                    &mut e,
                    &script.events(&map),
                    12 * FRAMES_PER_SECOND,
                    512,
                    |_, _, _| {},
                );
                (bits(&l), bits(&r))
            };
            let reference = with_mode(0, run_it);
            for (mode, mode_bits) in fp::MODES {
                assert!(
                    with_mode(mode_bits, run_it) == reference,
                    "{name} under {mode}"
                );
            }
        }
    }
}

#[cfg(debug_assertions)]
mod guarded {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// `process()` with a `delay` never allocates: notes, tempo changes (the read position fades), time and
    /// ping-pong changes by automation, every block shape.
    #[test]
    fn process_does_not_allocate() {
        for (name, p) in tail_patches() {
            let mut script = tail_script();
            script.tempo = vec![(0, 120.0), (6_000, 77.0), (9_000, 300.0)];
            script.params = vec![
                (3_000, DelayParam::Time, 90.0),
                (5_000, DelayParam::PingPong, 0.0),
                (7_000, DelayParam::Sync, 1.0),
                (8_000, DelayParam::Feedback, 100.0),
            ];
            let (plan, map) = compile(&p, &PrepareCfg::new(SR)).expect("compiles");
            let mut e = Engine::new(plan, SEED);
            let ev = script.events(&map);
            let mut left = vec![0.0f32; 24_000];
            let mut right = vec![0.0f32; 24_000];
            let mut next = 0;
            let mut start = 0;
            let mut chunk: Vec<Event> = Vec::with_capacity(16);
            while start < left.len() {
                let end = (start + 100).min(left.len());
                chunk.clear();
                while let Some(&(f, kind)) = ev.get(next) {
                    if f >= end as u64 {
                        break;
                    }
                    chunk.push(Event {
                        offset: (f - start as u64) as u32,
                        kind,
                    });
                    next += 1;
                }
                let (l, r) = (&mut left[start..end], &mut right[start..end]);
                assert_no_alloc(|| e.process(l, r, &chunk));
                start = end;
            }
            assert!(left.iter().any(|x| *x != 0.0), "{name}");
        }
    }
}

/// Render `script` (notes and tempo only) one segment at a time through `base`, swapping `edit` in at segment
/// `swap_at` when given: (left, the engine afterwards).
fn run_with_swap(
    base: &Patch,
    edit: Option<&Patch>,
    swap_at: usize,
    script: &Script,
    segments: usize,
) -> (Vec<f32>, Engine) {
    let plan = |p: &Patch| compile(p, &PrepareCfg::new(SR)).expect("compiles");
    let (base_plan, map) = plan(base);
    let (mut e, mut link): (Engine, Link) =
        Engine::with_link(base_plan, SEED, LinkCapacity::default());
    let mut edit = edit.map(|p| plan(p).0);
    let ev = script.events(&map);
    let mut left = vec![0.0; segments * SEG];
    let mut next = 0;
    for k in 0..segments {
        if k == swap_at
            && let Some(p) = edit.take()
        {
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        let end = ((k + 1) * SEG) as u64;
        let mut chunk = Vec::new();
        while let Some(&(f, kind)) = ev.get(next) {
            if f >= end {
                break;
            }
            chunk.push(Event {
                offset: (f - (k * SEG) as u64) as u32,
                kind,
            });
            next += 1;
        }
        let mut right = [0.0f32; SEG];
        e.process(&mut left[k * SEG..(k + 1) * SEG], &mut right, &chunk);
    }
    (left, e)
}

/// A patch with the delay at 240 BPM quarter notes (12,000 samples), feedback 35 %, and a modulator on the side that
/// a swap can add or remove: it changes the module set, so the swap crossfades.
fn swap_patch(extra: bool) -> Patch {
    let mut p = burst_patch(Beats::Quarter, 35.0);
    if extra {
        p.modulators
            .push(ModuleInstance::new(id("lfo1"), ModuleKind::Lfo));
    }
    p
}

fn swap_script() -> Script {
    Script {
        notes: one_note(1_000),
        tempo: vec![(0, 240.0)],
        ..Script::default()
    }
}

#[test]
fn an_instant_swap_moves_the_rings_and_changes_no_bit() {
    // The first echo is in flight when the same patch is swapped in: nothing is added or removed, so the swap does
    // not crossfade and the delay takes the old one's rings (a move, no copy); the render is the unswapped one.
    let (base, segments) = (swap_patch(false), 1_500);
    let (plain, _) = run_with_swap(&base, None, 0, &swap_script(), segments);
    assert!(plain.iter().any(|v| *v != 0.0));
    for swap_at in [100, 403, 450, 800, 1_000] {
        let (swapped, e) = run_with_swap(&base, Some(&base), swap_at, &swap_script(), segments);
        assert_eq!(e.stats().swaps, 1);
        assert_eq!(e.stats().crossfades, 0);
        assert_eq!(bits(&swapped), bits(&plain), "swapped at segment {swap_at}");
    }
}

#[test]
fn a_crossfading_swap_copies_the_rings_and_the_echoes_go_on() {
    // A module is added (and in the second half removed) while the first echo is starting: the two plans run for
    // 256 frames and the new delay starts from a copy of the old one's rings and filters. The crossfade of two
    // equal signals is that signal to within a rounding, and the echoes after it are the unswapped ones; with no
    // copy the new delay would be empty and the echoes would be missing.
    let segments = 1_500;
    let (plain, _) = run_with_swap(&swap_patch(false), None, 0, &swap_script(), segments);
    let echo_peak = peak(&plain, 1_024 + 12_000, 1_024 + 12_600);
    assert!(echo_peak > 0.05);
    for (from, to, name) in [(false, true, "added"), (true, false, "removed")] {
        // 403 is 128 frames before the first echo: the crossfade spans its start. 440 is during the first echo's
        // repeat; 800 is after the second echo.
        for swap_at in [403, 440, 800] {
            let (swapped, e) = run_with_swap(
                &swap_patch(from),
                Some(&swap_patch(to)),
                swap_at,
                &swap_script(),
                segments,
            );
            assert_eq!(e.stats().crossfades, 1, "{name} at {swap_at}");
            let worst = swapped
                .iter()
                .zip(&plain)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            eprintln!("{name} at segment {swap_at}: worst difference {worst:e}");
            assert!(worst < 1.0e-6, "{name} at {swap_at}: {worst}");
        }
    }
}

#[test]
fn a_swap_to_a_new_delay_has_no_state_to_carry() {
    // The control of the test above: the same patch with the delay under another id is a new module, which starts
    // empty, so the echoes that were in the ring are not in the render, and the difference is as large as an echo.
    let segments = 1_500;
    let (plain, _) = run_with_swap(&swap_patch(false), None, 0, &swap_script(), segments);
    let mut renamed = swap_patch(false);
    renamed.fx[0].id = id("delay2");
    let (swapped, e) = run_with_swap(
        &swap_patch(false),
        Some(&renamed),
        403,
        &swap_script(),
        segments,
    );
    assert_eq!(e.stats().crossfades, 1);
    let worst = swapped
        .iter()
        .zip(&plain)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst > 0.05, "{worst}");
}

#[test]
fn removing_the_delay_by_a_swap_fades_the_echoes_out() {
    // The other direction of the same machinery: the old plan's echoes are heard only through the crossfade.
    let segments = 1_500;
    let mut gone = swap_patch(false);
    gone.fx.clear();
    let (swapped, e) = run_with_swap(
        &swap_patch(false),
        Some(&gone),
        403,
        &swap_script(),
        segments,
    );
    let (dry, _) = run_with_swap(&gone, None, 0, &swap_script(), segments);
    assert_eq!(e.stats().crossfades, 1);
    let after = 403 * SEG + 256;
    assert_eq!(bits(&swapped[after..]), bits(&dry[after..]));
    assert!(swapped.iter().all(|v| v.is_finite()));
}

/// `key` held through `patch`, long enough that the harness's window starts after the attack: the left channel.
fn held(patch: &Patch, key: u8) -> Vec<f32> {
    let frames = FFT_LEN + 8_192;
    let (left, _, _) = run(
        patch,
        &Script {
            notes: vec![note(key, 0, frames as u64)],
            ..Script::default()
        },
        frames,
        512,
    );
    left
}

/// The keys of the sweeps: 110 Hz to 3.5 kHz (a sine at key 105 has its fifth harmonic at 17.5 kHz, which folds).
const KEYS: [u8; 6] = [45, 57, 69, 81, 93, 105];

/// An `osc` sine into one `delay1`.
fn sine_delay_patch() -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    p.fx.push(ModuleInstance::new(id("delay1"), ModuleKind::Delay));
    p
}

/// The loop time, in ms, that holds a whole number of periods of `key` (about 15 ms): the delay is then in phase
/// with the sine and the repeats add up until the soft clip limits them. The nearest whole sample is within half a
/// sample of a whole number of periods.
fn resonant_ms(key: u8) -> f32 {
    let f = enth_dsp::alias::midi_hz(f64::from(key));
    let periods = (0.015 * f).round().max(1.0);
    let samples = (periods * f64::from(SR) / f).round();
    (samples * 1000.0 / f64::from(SR)) as f32
}

fn sweep_with(configure: impl Fn(&mut Patch, u8)) -> Sweep {
    Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |k| {
        let mut p = sine_delay_patch();
        configure(&mut p, k);
        held(&p, k)
    })
}

/// The linear loop adds no aliasing: a sine through the delay (20 ms, feedback 35 %, mix 100) measures what the
/// dry sine measures, which is the measure's own floor (−93 to −120 dB; docs/dsp/aliasing.md). Measured 2026-10-27
/// with `Measure::REPORT_02`: −103.5, −97.7, −93.3, −95.5, −92.6 and −118.0 dB at 110 Hz to 3.52 kHz, with the
/// default filters and with them wide open alike; the bound is the worst plus 1.5 dB.
const LINEAR_BOUND_DB: f64 = -91.0;

/// The saturating loop, which is the one place the delay is not linear: feedback 100 % with the loop time a whole
/// number of periods of the sine, so the repeats add up in phase until the soft clip limits them and the
/// returned signal is a clipped sine at the base rate (no oversampling). Measured 2026-10-27, worst of the keys
/// 45–105 (110 Hz – 3.52 kHz): −46.4 dB with the filters wide open, −55.2 dB at their defaults (the 6 kHz high cut
/// takes the folding harmonics down) and −48.6 dB with ping-pong, filters wide open. Each bound is the worst plus
/// 1.5 dB. The tables are in `docs/dsp/delay.md`.
const SATURATED_OPEN_BOUND_DB: f64 = -44.9;
const SATURATED_DEFAULT_BOUND_DB: f64 = -53.7;
const SATURATED_PING_PONG_BOUND_DB: f64 = -47.1;

fn open_filters(p: &mut Patch) {
    set(p, DelayParam::LowCut, 20.0);
    set(p, DelayParam::HighCut, 20_000.0);
}

fn linear_sweep(wide_open: bool) -> Sweep {
    sweep_with(|p, _| {
        set(p, DelayParam::Sync, 0.0);
        set(p, DelayParam::Time, 20.0);
        if wide_open {
            open_filters(p);
        }
    })
}

fn saturated_sweep(ping_pong: bool, wide_open: bool) -> Sweep {
    sweep_with(|p, k| {
        set(p, DelayParam::Sync, 0.0);
        set(p, DelayParam::Time, resonant_ms(k));
        set(p, DelayParam::Feedback, 100.0);
        set(p, DelayParam::PingPong, if ping_pong { 1.0 } else { 0.0 });
        if wide_open {
            open_filters(p);
        }
    })
}

#[test]
fn a_sine_through_the_linear_loop_measures_the_floor() {
    let dry = Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |k| {
        let mut p = sine_delay_patch();
        p.fx.clear();
        held(&p, k)
    });
    for wide_open in [false, true] {
        let s = linear_sweep(wide_open);
        eprintln!("linear loop, wide open {wide_open}:\n{}", s.table());
        s.assert_below(LINEAR_BOUND_DB, "the linear loop");
        // Nothing of the loop's own: the worst point is the dry sine's, to within a dB.
        let (got, floor) = (s.worst().1, dry.worst().1);
        assert!(got <= floor + 1.0, "{got} against the dry sine's {floor}");
    }
}

#[test]
fn the_saturating_loop_aliases_below_its_recorded_bounds() {
    for (name, ping_pong, wide_open, bound) in [
        ("wide open", false, true, SATURATED_OPEN_BOUND_DB),
        ("default filters", false, false, SATURATED_DEFAULT_BOUND_DB),
        (
            "ping-pong, wide open",
            true,
            true,
            SATURATED_PING_PONG_BOUND_DB,
        ),
    ] {
        let s = saturated_sweep(ping_pong, wide_open);
        eprintln!("saturating loop, {name}:\n{}", s.table());
        s.assert_below(bound, &format!("the saturating loop, {name}"));
    }
}

#[test]
fn the_loop_is_driven_into_the_clip_by_the_resonant_sweep() {
    // The check on the setup above: the resonant loop really does saturate (a loop that stayed linear would measure
    // the floor and prove nothing). Its output is far above the dry sine's level and its aliasing far above the
    // linear loop's.
    let key = 81;
    let mut dry = sine_delay_patch();
    dry.fx.clear();
    let mut p = sine_delay_patch();
    set(&mut p, DelayParam::Sync, 0.0);
    set(&mut p, DelayParam::Time, resonant_ms(key));
    set(&mut p, DelayParam::Feedback, 100.0);
    open_filters(&mut p);
    let level = |x: &[f32]| peak(x, x.len() - 8_192, x.len());
    let (dry_level, wet_level) = (level(&held(&dry, key)), level(&held(&p, key)));
    eprintln!("dry peak {dry_level:.3}, saturated loop peak {wet_level:.3}");
    assert!(wet_level > 2.0 * dry_level);
}
