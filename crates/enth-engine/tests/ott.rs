//! `ott` end to end (S2a lane OTT): the module is the core with the catalogue's values in the engine's 32-frame
//! segments; at depth 0 it is the flat band split; `mix` 0 hands back the input bit for bit; two instances keep
//! separate state and equal the modules run by hand in series; an idle module may be skipped without changing a bit;
//! the host block size and the floating-point mode (FTZ/DAZ) are not inputs; a 40 s tail holds no subnormal; nothing
//! allocates; and the aliasing sweeps behind the bounds `docs/dsp/ott.md` records.
//!
//! What the module *is* (the equations, the closed forms of the crossover and of the static curves) is held by
//! `crates/enth-dsp/tests/ott.rs`; this file holds the wrapper and the engine to it.
//!
//! The allocation tests run under `assert_no_alloc` in debug builds (release builds lack `AllocDisabler`; the guard
//! is then a no-op).
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]
#![allow(
    clippy::print_stderr,
    reason = "the sweeps print the tables docs/dsp/ott.md records (cargo test -p enth-engine --test ott -- --nocapture)"
)]

mod common;

use std::fmt::Write as _;

use assert_no_alloc::assert_no_alloc;
use common::{SEED, SR, bits, drive, engine, events, id, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep, midi_hz, power_spectrum};
use enth_dsp::ott::Ott as OttCore;
use enth_dsp::util::rng::Rng;
use enth_engine::module::{
    Block, GlobalCx, GlobalParams, GlobalProcessor, GlobalStart, PrepareCfg, SEG, count_subnormals,
};
use enth_engine::modules::ott::{Ott, core_params};
use enth_patch::catalogue::{AdsrParam, OscParam, OttParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// ---------------------------------------------------------------------------------------------------------------
// The rigs
// ---------------------------------------------------------------------------------------------------------------

/// A module and the parameter values it sees; `prev` differs from `cur` only when a test makes it.
struct Rig {
    module: Ott,
    base: Vec<f32>,
    cur: Vec<f32>,
    prev: Vec<f32>,
}

impl Rig {
    fn new(vals: &[f32]) -> Self {
        let mut rig = Self {
            module: Ott::new(vals, &PrepareCfg::new(SR)),
            base: vals.to_vec(),
            cur: vals.to_vec(),
            prev: vals.to_vec(),
        };
        rig.reset();
        rig
    }

    fn reset(&mut self) {
        let params = GlobalParams::from_slices(&self.base, &self.cur, &self.prev);
        self.module.reset(&GlobalStart {
            seed: 1,
            module_key: 2,
            params,
        });
    }

    /// Set `p` for good: the next segment sees it as `cur` and, one segment later, as `prev` too.
    fn set(&mut self, p: OttParam, v: f32) {
        self.base[p.index()] = v;
        self.cur[p.index()] = v;
    }

    fn run(&mut self, l: &mut Block, r: &mut Block) {
        let params = GlobalParams::from_slices(&self.base, &self.cur, &self.prev);
        self.module.process(&mut GlobalCx {
            tempo_bpm: enth_engine::tempo::DEFAULT_TEMPO_BPM,
            params,
            left: l,
            right: r,
        });
        self.prev.clone_from(&self.cur);
    }

    /// The way the engine drives it: an idle module with silent input is skipped (the engine's smoothers move on all
    /// the same, so `prev` follows `cur`). Returns whether it ran.
    fn run_like_the_engine(&mut self, l: &mut Block, r: &mut Block) -> bool {
        let zero = |b: &Block| b.iter().all(|x| *x == 0.0);
        if self.module.is_silent() && zero(l) && zero(r) {
            self.prev.clone_from(&self.cur);
            return false;
        }
        self.run(l, r);
        true
    }
}

fn values(settings: &[(OttParam, f32)]) -> Vec<f32> {
    let mut v = ModuleKind::Ott.defaults();
    for &(p, x) in settings {
        v[p.index()] = x;
    }
    v
}

/// `noisy` segments of stereo noise in `[-amp, amp)`, then `silent` segments of zeros.
fn noise_then_silence(seed: u64, noisy: usize, silent: usize, amp: f32) -> Vec<(Block, Block)> {
    let mut rng = Rng::new(seed);
    (0..noisy + silent)
        .map(|i| {
            let mut seg = ([0.0; SEG], [0.0; SEG]);
            if i < noisy {
                for k in 0..SEG {
                    seg.0[k] = amp * rng.bipolar_f32();
                    seg.1[k] = amp * rng.bipolar_f32();
                }
            }
            seg
        })
        .collect()
}

/// `Patch::init()` under a gate envelope (no attack or decay, sustain 100 %, `release_ms`), its oscillator on `wave`
/// at `level` per cent.
fn gated(wave: Wave, level: f32, release_ms: f32) -> Patch {
    let mut p = Patch::init();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = release_ms;
    let osc = &mut p.generators[0].values;
    osc[OscParam::Wave.index()] = wave.index() as f32;
    osc[OscParam::Level.index()] = level;
    p
}

/// `p` with an `ott` named `name` at the end of its fx chain, with `set` applied.
fn with_ott(mut p: Patch, name: &str, set: &[(OttParam, f32)]) -> Patch {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Ott);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    p.fx.push(m);
    p
}

/// A chord with a release: three notes, off at 1 s.
fn chord() -> Vec<common::Note> {
    vec![
        note(36, 0, SR as u64),
        note(48, 960, SR as u64),
        note(60, 1_920, SR as u64 + 4_800),
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// The module is the core
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_module_is_the_core_with_the_catalogue_values_in_32_frame_segments() {
    let mut rig = Rig::new(&values(&[
        (OttParam::Depth, 80.0),
        (OttParam::Time, 150.0),
        (OttParam::Input, 6.0),
        (OttParam::Output, -3.0),
        (OttParam::LowSplit, 120.0),
        (OttParam::HighSplit, 3_000.0),
        (OttParam::LowGain, 2.0),
        (OttParam::HighGain, -2.0),
        (OttParam::Mix, 90.0),
    ]));
    let mut core = OttCore::new();
    core.prepare(SR);
    let mut to = core_params(|p| rig.cur[p.index()]);
    core.set_params(&to);
    let input = noise_then_silence(7, 600, 400, 0.6);
    for (i, (l, r)) in input.iter().enumerate() {
        // Moves in every field, one or two at a time, as the engine's smoothers and automation would make them.
        let from = to;
        match i {
            100 => rig.set(OttParam::Depth, 100.0),
            150 => rig.set(OttParam::Mix, 100.0),
            200 => {
                rig.set(OttParam::Upward, 40.0);
                rig.set(OttParam::Downward, 70.0);
            }
            300 => rig.set(OttParam::Time, 400.0),
            400 => {
                rig.set(OttParam::LowSplit, 60.0);
                rig.set(OttParam::HighSplit, 6_000.0);
            }
            450 => rig.set(OttParam::MidGain, 4.0),
            _ => {}
        }
        to = core_params(|p| rig.cur[p.index()]);
        core.set_ramp(&from, &to);
        let (mut ml, mut mr) = (*l, *r);
        let (mut cl, mut cr) = (*l, *r);
        rig.run(&mut ml, &mut mr);
        core.process_in_place(&mut cl, &mut cr);
        assert_eq!(bits(&ml), bits(&cl), "left, segment {i}");
        assert_eq!(bits(&mr), bits(&cr), "right, segment {i}");
    }
}

#[test]
fn mix_zero_is_the_input_bit_for_bit_whatever_the_rest_is_set_to() {
    let mut rig = Rig::new(&values(&[
        (OttParam::Mix, 0.0),
        (OttParam::Depth, 100.0),
        (OttParam::Input, 24.0),
        (OttParam::Output, 24.0),
        (OttParam::LowGain, 24.0),
        (OttParam::MidGain, -24.0),
        (OttParam::HighGain, 24.0),
    ]));
    for (l, r) in noise_then_silence(8, 200, 50, 0.9) {
        let (mut ml, mut mr) = (l, r);
        rig.run(&mut ml, &mut mr);
        assert_eq!(bits(&ml), bits(&l));
        assert_eq!(bits(&mr), bits(&r));
    }
}

#[test]
fn mix_zero_through_the_engine_is_the_patch_without_the_effect() {
    let dry = gated(Wave::Saw, 100.0, 400.0);
    let wet = with_ott(
        dry.clone(),
        "ott1",
        &[
            (OttParam::Mix, 0.0),
            (OttParam::Depth, 100.0),
            (OttParam::Input, 18.0),
            (OttParam::Output, 12.0),
            (OttParam::LowGain, 12.0),
        ],
    );
    let (dl, dr, _) = render(&dry, &chord(), 80_000, 512, SEED);
    let (wl, wr, _) = render(&wet, &chord(), 80_000, 512, SEED);
    assert!(dl.iter().any(|x| x.abs() > 0.1), "it sounds");
    assert_eq!(bits(&dl), bits(&wl));
    assert_eq!(bits(&dr), bits(&wr));
}

// ---------------------------------------------------------------------------------------------------------------
// Depth 0: the band split, flat
// ---------------------------------------------------------------------------------------------------------------

/// Power in ±6 bins around `f` Hz in the last `FFT_LEN` samples of `x`.
fn lobe_power(x: &[f32], f: f64) -> f64 {
    let p = power_spectrum(&x[x.len() - FFT_LEN..]);
    let c = (f / (f64::from(SR) / FFT_LEN as f64)).round() as isize;
    (c - 6..=c + 6)
        .filter(|&i| i >= 0 && (i as usize) < p.len())
        .map(|i| p[i as usize])
        .sum()
}

fn db(x: f64) -> f64 {
    20.0 * libm::log10(x.max(1e-30))
}

#[test]
fn depth_zero_through_the_engine_is_flat_within_a_tenth_of_a_db() {
    // A held sine at every octave of keys: the level out equals the level in (the three bands sum to an allpass),
    // at the default crossovers (88.3 Hz, 2.5 kHz) and at the extremes of the two split knobs.
    let frames = FFT_LEN + 8_192;
    for (low, high) in [(88.3, 2_500.0), (40.0, 500.0), (500.0, 10_000.0)] {
        let mut worst = 0.0_f64;
        for key in [24_u8, 36, 48, 60, 72, 84, 96, 108, 120] {
            let dry = gated(Wave::Sine, 50.0, 0.0);
            let wet = with_ott(
                dry.clone(),
                "ott1",
                &[
                    (OttParam::Depth, 0.0),
                    (OttParam::LowSplit, low),
                    (OttParam::HighSplit, high),
                ],
            );
            let notes = [note(key, 0, frames as u64)];
            let (d, _, _) = render(&dry, &notes, frames, 512, SEED);
            let (w, _, _) = render(&wet, &notes, frames, 512, SEED);
            let f = midi_hz(f64::from(key));
            let err = 10.0 * libm::log10(lobe_power(&w, f) / lobe_power(&d, f));
            worst = worst.max(err.abs());
        }
        eprintln!("depth 0, splits {low} / {high} Hz: worst level error {worst:.4} dB over 9 keys");
        assert!(worst < 0.1, "splits {low}/{high}: {worst} dB");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Two instances
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn two_modules_run_side_by_side_keep_separate_state() {
    let a_vals = values(&[(OttParam::Depth, 100.0), (OttParam::Time, 50.0)]);
    let b_vals = values(&[
        (OttParam::Depth, 60.0),
        (OttParam::Upward, 30.0),
        (OttParam::LowSplit, 150.0),
        (OttParam::Mix, 80.0),
    ]);
    let (mut a, mut b) = (Rig::new(&a_vals), Rig::new(&b_vals));
    let (mut alone_a, mut alone_b) = (Rig::new(&a_vals), Rig::new(&b_vals));
    let in_a = noise_then_silence(31, 300, 100, 0.5);
    let in_b = noise_then_silence(32, 200, 200, 0.1);
    for (i, ((la, ra), (lb, rb))) in in_a.iter().zip(&in_b).enumerate() {
        let (mut x, mut y) = ((*la, *ra), (*lb, *rb));
        let (mut xa, mut yb) = ((*la, *ra), (*lb, *rb));
        a.run(&mut x.0, &mut x.1);
        b.run(&mut y.0, &mut y.1);
        alone_a.run(&mut xa.0, &mut xa.1);
        alone_b.run(&mut yb.0, &mut yb.1);
        assert_eq!(bits(&x.0), bits(&xa.0), "a, segment {i}");
        assert_eq!(bits(&y.1), bits(&yb.1), "b, segment {i}");
    }
}

#[test]
fn two_ott_modules_in_a_patch_equal_the_modules_run_by_hand_in_series() {
    let a_set = [(OttParam::Depth, 100.0), (OttParam::Time, 50.0)];
    let b_set = [
        (OttParam::Depth, 60.0),
        (OttParam::Upward, 30.0),
        (OttParam::LowSplit, 150.0),
        (OttParam::Mix, 80.0),
    ];
    let dry = gated(Wave::Saw, 100.0, 400.0);
    let both = with_ott(with_ott(dry.clone(), "ott1", &a_set), "ott2", &b_set);
    let frames = 80_000;
    let (bl, br, _) = render(&both, &chord(), frames, 512, SEED);
    let (dl, dr, _) = render(&dry, &chord(), frames, 512, SEED);
    assert!(bl.iter().any(|x| x.abs() > 0.05), "it sounds");
    assert_ne!(bits(&bl), bits(&dl), "the effects do something");

    let a_vals = values(&a_set);
    let b_vals = values(&b_set);
    let (mut a, mut b) = (Rig::new(&a_vals), Rig::new(&b_vals));
    let mut by_hand = (vec![0.0_f32; frames], vec![0.0_f32; frames]);
    for s in 0..frames / SEG {
        let range = s * SEG..(s + 1) * SEG;
        let mut l: Block = dl[range.clone()].try_into().unwrap();
        let mut r: Block = dr[range.clone()].try_into().unwrap();
        a.run_like_the_engine(&mut l, &mut r);
        b.run_like_the_engine(&mut l, &mut r);
        by_hand.0[range.clone()].copy_from_slice(&l);
        by_hand.1[range].copy_from_slice(&r);
    }
    // The master level (0 dB) is a multiply by exactly 1.
    assert_eq!(
        bits(&bl[..frames / SEG * SEG]),
        bits(&by_hand.0[..frames / SEG * SEG])
    );
    assert_eq!(
        bits(&br[..frames / SEG * SEG]),
        bits(&by_hand.1[..frames / SEG * SEG])
    );

    // And a second module that does nothing (mix 0) neither changes the first nor is changed by it.
    let one = with_ott(dry.clone(), "ott1", &a_set);
    let one_plus = with_ott(
        with_ott(dry.clone(), "ott1", &a_set),
        "ott2",
        &[(OttParam::Mix, 0.0)],
    );
    let (ol, _, _) = render(&one, &chord(), frames, 512, SEED);
    let (pl, _, _) = render(&one_plus, &chord(), frames, 512, SEED);
    assert_eq!(bits(&ol), bits(&pl));
}

// ---------------------------------------------------------------------------------------------------------------
// Idle, skipping, state carry
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn an_idle_ott_skipped_by_the_engine_gives_the_same_bits_as_one_that_is_always_called() {
    // Time 10 % (the fastest) so the state is exactly zero within about a second of silence. Settings change while
    // idle (the skipped module never sees them until it wakes).
    let vals = values(&[(OttParam::Time, 10.0), (OttParam::Depth, 100.0)]);
    let mut always = Rig::new(&vals);
    let mut skipping = Rig::new(&vals);
    let burst = 300;
    let gap = 3 * 48_000 / SEG;
    let mut input = noise_then_silence(1, burst, gap, 0.8);
    input.extend(noise_then_silence(2, burst, gap, 0.8));
    let (mut skipped, mut woke, mut was_idle) = (0usize, 0usize, false);
    for (i, (l, r)) in input.iter().enumerate() {
        if i == burst + gap / 2 {
            for rig in [&mut always, &mut skipping] {
                rig.set(OttParam::Depth, 70.0);
                rig.set(OttParam::Upward, 50.0);
                rig.set(OttParam::Output, 3.0);
                rig.set(OttParam::Mix, 60.0);
                rig.set(OttParam::LowSplit, 200.0);
            }
        }
        let (mut al, mut ar) = (*l, *r);
        let (mut sl, mut sr) = (*l, *r);
        always.run(&mut al, &mut ar);
        let ran = skipping.run_like_the_engine(&mut sl, &mut sr);
        skipped += usize::from(!ran);
        assert_eq!(bits(&al), bits(&sl), "left, segment {i}");
        assert_eq!(bits(&ar), bits(&sr), "right, segment {i}");
        let idle = skipping.module.is_silent();
        assert_eq!(always.module.is_silent(), idle, "segment {i}");
        woke += usize::from(was_idle && !idle);
        was_idle = idle;
    }
    assert!(skipped > 1_000, "only {skipped} segments were skipped");
    assert_eq!(woke, 1);
}

#[test]
fn a_40_s_tail_holds_no_subnormal_and_the_module_goes_idle() {
    // A loud burst, then 40 s of silence with the slowest times that fit (time 400 %: the longest release is 4 x
    // 200 ms): nothing subnormal, ever, in the output or the state, and the state ends in exact zeros.
    for (time, depth) in [(400.0_f32, 100.0_f32), (100.0, 100.0), (10.0, 30.0)] {
        let mut rig = Rig::new(&values(&[
            (OttParam::Time, time),
            (OttParam::Depth, depth),
            (OttParam::Input, 12.0),
        ]));
        let silence = 40 * 48_000 / SEG;
        let mut idle_at = None;
        for (i, (l, r)) in noise_then_silence(3, 200, silence, 1.0)
            .into_iter()
            .enumerate()
        {
            let (mut l, mut r) = (l, r);
            rig.run_like_the_engine(&mut l, &mut r);
            assert_eq!(count_subnormals(&l) + count_subnormals(&r), 0);
            assert_eq!(rig.module.subnormals(), 0, "segment {i}");
            if idle_at.is_none() && rig.module.is_silent() {
                idle_at = Some(i);
            }
        }
        let idle_at = idle_at.unwrap_or_else(|| panic!("time {time} %: never idle in 40 s"));
        eprintln!(
            "time {time} %, depth {depth} %: idle {:.1} s after the burst ended",
            (idle_at - 200) as f64 * SEG as f64 / f64::from(SR)
        );
    }
}

#[test]
fn a_40_s_release_tail_through_the_engine_has_no_subnormals() {
    let p = with_ott(
        gated(Wave::Saw, 100.0, 3_000.0),
        "ott1",
        &[
            (OttParam::Depth, 100.0),
            (OttParam::Time, 400.0),
            (OttParam::Input, 12.0),
        ],
    );
    let mut e = engine(&p, SEED);
    let (mut output, mut state, mut non_finite) = (0usize, 0usize, 0usize);
    let mut calls = 0usize;
    drive(
        &mut e,
        &events(&chord()),
        40 * SR as usize,
        512,
        |e, l, r| {
            output += count_subnormals(l) + count_subnormals(r);
            non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                state = state.max(e.subnormals());
            }
            calls += 1;
        },
    );
    state = state.max(e.subnormals());
    assert_eq!(output, 0, "subnormal output samples");
    assert_eq!(state, 0, "subnormal values in the engine state");
    assert_eq!(non_finite, 0, "non-finite samples");
    assert!(e.active_voices().is_empty(), "every voice ends");
}

#[test]
fn nan_input_does_not_poison_the_state() {
    let vals = ModuleKind::Ott.defaults();
    let mut dirty = Rig::new(&vals);
    let mut clean = Rig::new(&vals);
    for (i, (l, r)) in noise_then_silence(11, 400, 100, 0.8)
        .into_iter()
        .enumerate()
    {
        let (mut dl, mut dr) = (l, r);
        let (mut cl, mut cr) = (l, r);
        if i % 9 == 4 {
            // NaN on one channel, then on both; the clean twin sees 0, which is what the module makes of it.
            dl[3] = f32::NAN;
            cl[3] = 0.0;
            dl[20] = f32::NAN;
            dr[20] = f32::NAN;
            cl[20] = 0.0;
            cr[20] = 0.0;
        }
        dirty.run(&mut dl, &mut dr);
        clean.run(&mut cl, &mut cr);
        // The input is cleaned before it enters any filter, so every output frame is finite and equals the twin's.
        assert_eq!(bits(&dl), bits(&cl), "left, segment {i}");
        assert_eq!(bits(&dr), bits(&cr), "right, segment {i}");
    }
    let (mut l, mut r) = ([0.3; SEG], [0.3; SEG]);
    l[5] = f32::INFINITY;
    dirty.run(&mut l, &mut r);
    let (mut l, mut r) = ([0.3; SEG], [0.3; SEG]);
    dirty.run(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}

#[test]
fn copy_state_makes_two_modules_identical_from_then_on() {
    let vals = ModuleKind::Ott.defaults();
    let mut a = Rig::new(&vals);
    let mut b = Rig::new(&vals);
    for (l, r) in noise_then_silence(21, 200, 0, 0.9) {
        let (mut l, mut r) = (l, r);
        a.run(&mut l, &mut r);
    }
    for (l, r) in noise_then_silence(22, 50, 0, 0.1) {
        let (mut l, mut r) = (l, r);
        b.run(&mut l, &mut r);
    }
    b.module.copy_state_from(&a.module);
    for rig in [&mut a, &mut b] {
        rig.set(OttParam::Downward, 50.0);
    }
    for (l, r) in noise_then_silence(23, 100, 100, 0.7) {
        let (mut al, mut ar) = (l, r);
        let (mut bl, mut br) = (l, r);
        a.run(&mut al, &mut ar);
        b.run(&mut bl, &mut br);
        assert_eq!(bits(&al), bits(&bl));
        assert_eq!(bits(&ar), bits(&br));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The host: block size, FTZ, allocation
// ---------------------------------------------------------------------------------------------------------------

fn busy() -> Patch {
    with_ott(
        gated(Wave::Saw, 100.0, 800.0),
        "ott1",
        &[
            (OttParam::Depth, 100.0),
            (OttParam::Time, 30.0),
            (OttParam::LowGain, 3.0),
            (OttParam::HighGain, -4.0),
        ],
    )
}

#[test]
fn the_host_block_size_is_not_an_input() {
    let p = busy();
    let notes = [
        note(36, 0, 9_000),
        note(55, 1_500, 9_000),
        note(64, 2_001, 7_000),
    ];
    let (reference, rr, _) = render(&p, &notes, 14_000, 512, SEED);
    assert!(reference.iter().any(|x| x.abs() > 0.1));
    for block in [1usize, 7, 32, 64, 100, 4_096] {
        let (l, r, _) = render(&p, &notes, 14_000, block, SEED);
        assert_eq!(bits(&l), bits(&reference), "left at block {block}");
        assert_eq!(bits(&r), bits(&rr), "right at block {block}");
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
    pub const MODES: [(&str, u64); 3] = [("IEEE", 0), ("FTZ", FTZ), ("FTZ+DAZ", FTZ | DAZ)];
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
    pub const MODES: [(&str, u64); 2] = [("IEEE", 0), ("FZ", FZ)];
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn the_render_is_the_same_under_every_ftz_mode() {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }
    // The module's own tail (time 400 %, so the decay is slow and long) and the engine's.
    let run = || {
        let mut e = engine(&busy(), SEED);
        let (l, r) = drive(
            &mut e,
            &events(&chord()),
            6 * SR as usize,
            512,
            |_, _, _| {},
        );
        let mut rig = Rig::new(&values(&[(OttParam::Time, 400.0)]));
        let mut tail = Vec::new();
        for (l, r) in noise_then_silence(5, 100, 1_500, 0.9) {
            let (mut l, mut r) = (l, r);
            rig.run(&mut l, &mut r);
            tail.extend(bits(&l));
            tail.extend(bits(&r));
        }
        (bits(&l), bits(&r), tail)
    };
    let with_mode = |mode: u64| {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | mode);
        std::hint::black_box(run())
    };
    let reference = with_mode(0);
    for (name, mode) in fp::MODES {
        assert!(with_mode(mode) == reference, "the render under {name}");
    }
}

#[test]
fn process_allocates_nothing() {
    let mut rig = Rig::new(&ModuleKind::Ott.defaults());
    let mut e = engine(&busy(), SEED);
    let mut l = vec![0.0_f32; 512];
    let mut r = vec![0.0_f32; 512];
    let input = noise_then_silence(41, 60, 20, 0.7);
    assert_no_alloc(|| {
        for &(a, b) in &input {
            let (mut a, mut b) = (a, b);
            rig.set(OttParam::Depth, 55.0);
            rig.run(&mut a, &mut b);
        }
        for _ in 0..40 {
            e.process(&mut l, &mut r, &[]);
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Aliasing
// ---------------------------------------------------------------------------------------------------------------

/// Keys 33 to 117 in octaves: 55 Hz to 8.4 kHz.
const KEYS: [u8; 8] = [33, 45, 57, 69, 81, 93, 105, 117];

/// `key` held through `patch`, long enough that the harness's window (the last 32,768 samples) starts after the
/// attack and the detector has settled: the left channel.
fn held(patch: &Patch, key: u8) -> Vec<f32> {
    let frames = FFT_LEN + 24_576;
    let (left, _, _) = render(patch, &[note(key, 0, frames as u64)], frames, 512, SEED);
    left
}

/// The peak of `key` held through `patch`, dBFS (the last second).
fn held_peak_db(patch: &Patch, key: u8) -> f64 {
    let x = held(patch, key);
    db(x[x.len() - 48_000..]
        .iter()
        .fold(0.0_f64, |m, v| m.max(f64::from(*v).abs())))
}

/// A sine at `level` per cent of full scale into an `ott` with `set`.
fn sine_into_ott(level: f32, set: &[(OttParam, f32)]) -> Patch {
    with_ott(gated(Wave::Sine, level, 0.0), "ott1", set)
}

/// One sweep, printed and checked: the keys, the three levels (loud: the downward curves work; middle: both curves are
/// near their knees; quiet: the upward curves lift), at depth 100 % and the stated `time`.
fn alias_sweep(name: &str, set: &[(OttParam, f32)], bound_db: f64) -> f64 {
    let mut out = String::new();
    let mut worst = f64::NEG_INFINITY;
    for (case, level) in [("loud", 100.0_f32), ("middle", 4.0), ("quiet", 0.1)] {
        let p = sine_into_ott(level, set);
        let sweep = Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |key| {
            held(&p, key)
        });
        let (_, w) = sweep.worst();
        worst = worst.max(w);
        let dry_peak = held_peak_db(&gated(Wave::Sine, level, 0.0), 69);
        let _ = writeln!(
            out,
            "{name}, {case} (input peak {dry_peak:+.1} dBFS):\n{}",
            sweep.table()
        );
    }
    eprintln!("{out}worst {worst:.1} dB (bound {bound_db:.1})");
    assert!(
        worst <= bound_db,
        "{name}: worst {worst:.1} dB over the bound {bound_db:.1}\n{out}"
    );
    worst
}

/// The defaults except `depth` 100 %: the full effect at the catalogue's times and crossovers.
#[test]
fn aliasing_at_full_depth_and_the_default_times() {
    alias_sweep(
        "ott, depth 100 %, time 100 %",
        &[(OttParam::Depth, 100.0)],
        BOUND_DEFAULT_DB,
    );
}

/// The fastest detector (time 10 %: 0.2 / 0.8 / 2 ms attacks, 5 / 10 / 20 ms releases), where the follower's ripple at
/// twice the sine's frequency, and so the gain's, is largest.
#[test]
fn aliasing_at_full_depth_and_the_fastest_times() {
    alias_sweep(
        "ott, depth 100 %, time 10 %",
        &[(OttParam::Depth, 100.0), (OttParam::Time, 10.0)],
        BOUND_FAST_DB,
    );
}

/// The catalogue defaults (depth 30 %).
#[test]
fn aliasing_at_the_catalogue_defaults() {
    alias_sweep("ott, defaults", &[], BOUND_FACTORY_DB);
}

/// The sines are not the worst case for a gain that follows the input: the same sweep with the gain staging pushed
/// into the knees (+24 dB input, bands pulled apart).
#[test]
fn aliasing_with_the_input_driven_into_the_curves() {
    alias_sweep(
        "ott, depth 100 %, input +24 dB, time 10 %, splits 40 / 500 Hz",
        &[
            (OttParam::Depth, 100.0),
            (OttParam::Time, 10.0),
            (OttParam::Input, 24.0),
            (OttParam::LowSplit, 40.0),
            (OttParam::HighSplit, 500.0),
        ],
        BOUND_DRIVEN_DB,
    );
}

/// Measured 2026-10-26 (lane OTT; the tables are in `docs/dsp/ott.md`): the worst point of each sweep was -92.6 dB
/// (depth 100 %), -65.9 dB (time 10 %), -92.7 dB (the defaults) and -65.2 dB (input +24 dB, time 10 %, wide splits);
/// each bound is the worst plus 1.5 dB. The -93 dB of the slow sweeps is the engine's own sine (`alias.rs` asserts it
/// is below -90 dB), not the module; the fast detector's gain ripple at twice the sine's frequency, in the high band
/// (keys 105 and 117), is the whole of the others.
const BOUND_DEFAULT_DB: f64 = -91.1;
const BOUND_FAST_DB: f64 = -64.4;
const BOUND_FACTORY_DB: f64 = -91.2;
const BOUND_DRIVEN_DB: f64 = -63.7;
