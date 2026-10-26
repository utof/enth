//! `dist` through the engine (S2a lane DST; `docs/dsp/dist.md`): the spectral aliasing bounds of each shape at
//! every oversampling setting, the chain of two modules, the plan swap with an oversampling change, the 40 s tail
//! with zero subnormals, the render under every FTZ mode, and `process` under the no-allocation guard. The response
//! checks (tone, `keep_lows`, `mix`, bias, bitcrush, downsample, the static curves) are in `enth_dsp::dist`'s and
//! the module's unit tests, which can feed a quiet, exactly known signal; here the signal is a held `osc` sine.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{SEED, SR, bits, drive, engine, events, id, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep};
use enth_engine::module::{SEG, count_subnormals};
use enth_engine::{Engine, Event, EventKind, Link, LinkCapacity, NoteKey, PrepareCfg, compile};
use enth_patch::catalogue::{AdsrParam, DistMode, DistParam, OscParam, Oversample, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch};

/// `key` held through `patch`, long enough that the harness's window starts after the attack: the left channel.
fn held(patch: &Patch, key: u8) -> Vec<f32> {
    let frames = FFT_LEN + 8_192;
    let (left, _, _) = render(patch, &[note(key, 0, frames as u64)], frames, 512, SEED);
    left
}

/// A sine oscillator into one `dist` with the given mode, oversampling and drive (the rest at its defaults).
fn dist_patch(mode: DistMode, os: Oversample, drive_db: f32) -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    p.voice_fx
        .push(ModuleInstance::new(id("dist1"), ModuleKind::Dist));
    let v = &mut p.voice_fx[0].values;
    v[DistParam::Mode.index()] = mode.index() as f32;
    v[DistParam::Oversample.index()] = os.index() as f32;
    v[DistParam::Drive.index()] = drive_db;
    p
}

/// The keys of the sweeps: 110 Hz to 3.5 kHz (the fundamentals whose harmonics fold: a sine at key 105 has its
/// fifth harmonic at 17.5 kHz).
const KEYS: [u8; 6] = [45, 57, 69, 81, 93, 105];

const SHAPES: [DistMode; 5] = [
    DistMode::Soft,
    DistMode::Hard,
    DistMode::Asymmetric,
    DistMode::SineFold,
    DistMode::TriangleFold,
];
const FACTORS: [Oversample; 3] = [Oversample::Off, Oversample::X2, Oversample::X4];

/// The worst alias measure (dB) of each shape at `drive` 24 dB, per oversampling setting (off, x2, x4), measured on
/// 2026-10-26 with `Measure::REPORT_02` over [`KEYS`] on the engine's sine; each bound is the worst plus 1.5 dB
/// (`docs/dsp/aliasing.md`). The tables are in `docs/dsp/dist.md`.
const BOUNDS_24_DB: [(DistMode, [f64; 3]); 5] = [
    (DistMode::Soft, [-25.8, -46.9, -68.9]),
    (DistMode::Hard, [-25.0, -43.4, -67.9]),
    (DistMode::Asymmetric, [-25.1, -43.5, -63.1]),
    (DistMode::SineFold, [-4.1, -45.3, -43.3]),
    (DistMode::TriangleFold, [-5.6, -11.8, -38.5]),
];

fn sweep(mode: DistMode, os: Oversample, drive_db: f32) -> Sweep {
    let p = dist_patch(mode, os, drive_db);
    Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |k| held(&p, k))
}

#[test]
fn each_shape_aliases_below_its_recorded_bound_at_every_oversampling() {
    for (mode, bounds) in BOUNDS_24_DB {
        for (os, bound) in FACTORS.into_iter().zip(bounds) {
            let s = sweep(mode, os, 24.0);
            eprintln!("{} {} (24 dB drive):\n{}", mode.key(), os.key(), s.table());
            s.assert_below(bound, &format!("{} {} at 24 dB", mode.key(), os.key()));
        }
    }
}

/// Oversampling measurably lowers aliasing: the mean over the sweep is at least 10 dB lower at 2x and at 4x than
/// with ADAA alone, for every shape; and 4x is at least 5 dB below 2x for all but the sine fold, whose
/// Bessel-flat harmonics out to 60 kHz make the half-bands' stopband (70 and 76.5 dB) the floor at either factor.
#[test]
fn oversampling_lowers_aliasing() {
    let mean = |s: &Sweep| s.points.iter().map(|p| p.1).sum::<f64>() / s.points.len() as f64;
    for mode in SHAPES {
        let off = mean(&sweep(mode, Oversample::Off, 24.0));
        let x2 = mean(&sweep(mode, Oversample::X2, 24.0));
        let x4 = mean(&sweep(mode, Oversample::X4, 24.0));
        eprintln!(
            "{}: mean {off:.1} (off), {x2:.1} (x2), {x4:.1} (x4) dB",
            mode.key()
        );
        assert!(
            x2 < off - 10.0,
            "{}: x2 {x2:.1} vs off {off:.1}",
            mode.key()
        );
        assert!(
            x4 < off - 10.0,
            "{}: x4 {x4:.1} vs off {off:.1}",
            mode.key()
        );
        if mode != DistMode::SineFold {
            assert!(x4 < x2 - 5.0, "{}: x4 {x4:.1} vs x2 {x2:.1}", mode.key());
        }
    }
}

/// A moderate drive is clean at the default setting: a soft clip at 12 dB with x2 is below -88 dB over the sweep, the
/// asymmetric curve below -68 dB and the sine fold below -91 dB.
#[test]
fn the_defaults_are_clean_at_moderate_drive() {
    for mode in [DistMode::Soft, DistMode::Asymmetric, DistMode::SineFold] {
        // Measured -89.9, -69.9 and -92.7 dB at 12 dB drive and x2; the bounds are the worst plus 1.5 dB.
        let bound = match mode {
            DistMode::Soft => -88.4,
            DistMode::Asymmetric => -68.4,
            _ => -91.2,
        };
        sweep(mode, Oversample::X2, 12.0)
            .assert_below(bound, &format!("{} x2 at 12 dB", mode.key()));
    }
}

/// The converters alias on purpose; their numbers are recorded for `docs/dsp/dist.md` and only bounded away from
/// nonsense (a measure that is finite and a block that is not silent).
#[test]
fn the_converters_record_their_aliasing() {
    for (mode, set) in [
        (DistMode::Bitcrush, vec![(DistParam::Bits, 4.0)]),
        (DistMode::Downsample, vec![(DistParam::Rate, 8_000.0)]),
    ] {
        let mut p = dist_patch(mode, Oversample::X2, 0.0);
        for (param, v) in set {
            p.voice_fx[0].values[param.index()] = v;
        }
        let s = Sweep::run_keys(&KEYS, f64::from(SR), &Measure::REPORT_02, |k| held(&p, k));
        eprintln!("{} (sine in):\n{}", mode.key(), s.table());
        assert!(s.points.iter().all(|p| p.1.is_finite() && p.1 > -90.0));
    }
}

/// Two `dist` modules in one voice-FX chain run, and each is its own state: the pair differs from either alone, the
/// result is finite, and swapping the two modules' settings between the slots changes the sound (the order matters).
#[test]
fn two_modules_in_one_chain() {
    let mut a = dist_patch(DistMode::Soft, Oversample::X2, 24.0);
    a.voice_fx
        .push(ModuleInstance::new(id("dist2"), ModuleKind::Dist));
    let v = &mut a.voice_fx[1].values;
    v[DistParam::Mode.index()] = DistMode::SineFold.index() as f32;
    v[DistParam::Drive.index()] = 12.0;
    v[DistParam::Tone.index()] = 5_000.0;
    let mut swapped = a.clone();
    swapped.voice_fx.swap(0, 1);
    swapped.voice_fx[0].id = id("dist1");
    swapped.voice_fx[1].id = id("dist2");
    let alone = dist_patch(DistMode::Soft, Oversample::X2, 24.0);
    let frames = 24_000;
    let n = [note(57, 0, 20_000)];
    let (l_pair, _, _) = render(&a, &n, frames, 512, SEED);
    let (l_swap, _, _) = render(&swapped, &n, frames, 512, SEED);
    let (l_alone, _, _) = render(&alone, &n, frames, 512, SEED);
    assert!(l_pair.iter().all(|s| s.is_finite()));
    assert!(l_pair.iter().any(|s| s.abs() > 0.1));
    assert_ne!(bits(&l_pair), bits(&l_alone));
    assert_ne!(bits(&l_pair), bits(&l_swap));
    // Deterministic: a second render is bit-identical.
    let (again, _, _) = render(&a, &n, frames, 512, SEED);
    assert_eq!(bits(&l_pair), bits(&again));
}

/// A patch with a `dist` per voice, a long release and every setting in play, for the tail and FTZ tests.
fn tail_patch() -> Patch {
    let mut p = dist_patch(DistMode::Asymmetric, Oversample::X4, 18.0);
    p.modulators[0].values[AdsrParam::Release.index()] = 3_000.0;
    let v = &mut p.voice_fx[0].values;
    v[DistParam::Bias.index()] = 30.0;
    v[DistParam::KeepLows.index()] = 150.0;
    v[DistParam::Tone.index()] = 6_000.0;
    v[DistParam::Mix.index()] = 80.0;
    p
}

const FRAMES_PER_SECOND: usize = 48_000;

fn tail_notes() -> Vec<common::Note> {
    vec![
        note(48, 0, FRAMES_PER_SECOND as u64),
        note(55, 0, FRAMES_PER_SECOND as u64),
        note(60, 960, FRAMES_PER_SECOND as u64),
    ]
}

#[test]
fn a_40_s_tail_through_dist_has_no_subnormals_and_every_voice_ends() {
    for mode in [
        DistMode::Asymmetric,
        DistMode::Bitcrush,
        DistMode::Downsample,
    ] {
        let mut patch = tail_patch();
        patch.voice_fx[0].values[DistParam::Mode.index()] = mode.index() as f32;
        let mut e = engine(&patch, SEED);
        let (mut sub_out, mut state_max, mut non_finite) = (0, 0, 0);
        let mut calls = 0usize;
        drive(
            &mut e,
            &events(&tail_notes()),
            40 * FRAMES_PER_SECOND,
            512,
            |e, l, r| {
                sub_out += count_subnormals(l) + count_subnormals(r);
                non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
                if calls.is_multiple_of(16) {
                    state_max = state_max.max(e.subnormals());
                }
                calls += 1;
            },
        );
        assert_eq!(sub_out, 0, "{}: subnormal output samples", mode.key());
        assert_eq!(
            state_max.max(e.subnormals()),
            0,
            "{}: engine state",
            mode.key()
        );
        assert_eq!(non_finite, 0, "{}: non-finite samples", mode.key());
        assert_eq!(
            e.active_voices().len(),
            0,
            "{}: voices must end",
            mode.key()
        );
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

    /// The render of the tail patch is the same under every FTZ/DAZ mode: every state is flushed in code, so no
    /// subnormal ever reaches the arithmetic.
    #[test]
    fn the_render_is_the_same_under_every_ftz_mode() {
        for mode in [
            DistMode::Asymmetric,
            DistMode::TriangleFold,
            DistMode::Downsample,
        ] {
            let mut patch = tail_patch();
            patch.voice_fx[0].values[DistParam::Mode.index()] = mode.index() as f32;
            let run = || {
                let mut e = engine(&patch, SEED);
                let (l, r) = drive(
                    &mut e,
                    &events(&tail_notes()),
                    12 * FRAMES_PER_SECOND,
                    512,
                    |_, _, _| {},
                );
                (bits(&l), bits(&r))
            };
            let reference = with_mode(0, run);
            for (name, mode_bits) in fp::MODES {
                assert!(
                    with_mode(mode_bits, run) == reference,
                    "{} under {name}",
                    mode.key()
                );
            }
        }
    }
}

#[cfg(debug_assertions)]
mod guarded {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};
    use enth_engine::{Event, EventKind};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// `process()` with a `dist` in every mode and oversampling never allocates (notes, mode changes by a new
    /// plan are the swap tests' business).
    #[test]
    fn process_does_not_allocate() {
        for mode in 0..7 {
            for os in FACTORS {
                let mut p = tail_patch();
                p.voice_fx[0].values[DistParam::Mode.index()] = mode as f32;
                p.voice_fx[0].values[DistParam::Oversample.index()] = os.index() as f32;
                let mut e = engine(&p, SEED);
                let ev = events(&[note(48, 0, 4_000), note(64, 500, 6_000)]);
                let mut left = vec![0.0f32; 8_192];
                let mut right = vec![0.0f32; 8_192];
                let mut next = 0;
                let mut start = 0;
                let mut chunk: Vec<Event> = Vec::with_capacity(8);
                while start < left.len() {
                    let end = (start + 256).min(left.len());
                    chunk.clear();
                    while let Some(&(f, kind)) = ev.get(next) {
                        if f >= end as u64 {
                            break;
                        }
                        chunk.push(Event {
                            offset: (f - start as u64) as u32,
                            kind: kind as EventKind,
                        });
                        next += 1;
                    }
                    let (l, r) = (&mut left[start..end], &mut right[start..end]);
                    assert_no_alloc(|| e.process(l, r, &chunk));
                    start = end;
                }
                assert!(left.iter().any(|x| *x != 0.0), "mode {mode} {}", os.key());
            }
        }
    }
}

/// A chord held, released late; `edit` is swapped in at segment 200 when given. Returns the left channel and the
/// engine afterwards.
fn chord_with_swap(base: &Patch, edit: Option<&Patch>) -> (Vec<f32>, Engine) {
    let plan = |p: &Patch| {
        compile(p, &PrepareCfg::new(SR))
            .expect("the patch compiles")
            .0
    };
    let (mut e, mut link): (Engine, Link) =
        Engine::with_link(plan(base), SEED, LinkCapacity::default());
    let mut edit = edit.map(plan);
    let key = |k: u8, i: i32| NoteKey::new(k, i);
    let mut left = vec![0.0; 600 * SEG];
    for k in 0..600 {
        if k == 200
            && let Some(p) = edit.take()
        {
            link.send_plan(p).expect("room for a plan");
        }
        link.collect();
        let ev: Vec<Event> = match k {
            0 => [48, 55, 60]
                .iter()
                .enumerate()
                .map(|(i, &n)| Event {
                    offset: 0,
                    kind: EventKind::NoteOn {
                        key: key(n, i as i32),
                        velocity: 0.8,
                    },
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut right = [0.0f32; SEG];
        e.process(&mut left[k * SEG..(k + 1) * SEG], &mut right, &ev);
    }
    (left, e)
}

/// A held chord survives recompiling the same patch bit for bit (the voices' state carries by module id and kind),
/// and a change of the structural `oversample` is a swap without a crossfade whose output stays finite and
/// continuous within the transient of re-priming the oversampler.
#[test]
fn the_plan_swap_carries_dist_state() {
    let base = tail_patch();
    let (l0, _) = chord_with_swap(&base, None);
    let (l1, e) = chord_with_swap(&base, Some(&base));
    assert!(l0.iter().any(|x| *x != 0.0));
    assert_eq!(e.stats().swaps, 1);
    assert_eq!(e.stats().crossfades, 0);
    assert_eq!(bits(&l1), bits(&l0));
    for (from, to) in [
        (Oversample::X4, Oversample::X2),
        (Oversample::X4, Oversample::Off),
        (Oversample::X4, Oversample::X4),
    ] {
        let mut a = base.clone();
        a.voice_fx[0].values[DistParam::Oversample.index()] = from.index() as f32;
        let mut b = base.clone();
        b.voice_fx[0].values[DistParam::Oversample.index()] = to.index() as f32;
        let (la, _) = chord_with_swap(&a, None);
        let (lc, _) = chord_with_swap(&b, None);
        let (lb, e) = chord_with_swap(&a, Some(&b));
        assert_eq!(e.stats().crossfades, 0, "nothing added or removed");
        assert!(lb.iter().all(|x| x.is_finite()));
        let swap = 200 * SEG;
        assert_eq!(
            bits(&la[..swap]),
            bits(&lb[..swap]),
            "nothing changes before the swap"
        );
        // The swap segment differs from the new setting's own render by the re-priming transient only, and the
        // carried voices converge on it (the DC blocker's 32 ms is the slowest state).
        let gap = |r: std::ops::Range<usize>| r.map(|i| (lb[i] - lc[i]).abs()).fold(0.0, f32::max);
        let jump = gap(swap..swap + SEG);
        let settled = gap(lb.len() - 100 * SEG..lb.len());
        eprintln!(
            "{} -> {}: against the new setting's own render: {jump:.4} in the swap segment, {settled:.5} at the end",
            from.key(),
            to.key()
        );
        assert!(jump < 2.0, "{} -> {}: {jump}", from.key(), to.key());
        assert!(
            settled < 5.0e-3,
            "{} -> {}: {settled}",
            from.key(),
            to.key()
        );
        let peak = lb[swap..].iter().fold(0.0, |m: f32, x| m.max(x.abs()));
        assert!(peak < 3.0, "{peak}");
    }
}
