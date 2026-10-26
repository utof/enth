//! The filter lane's (FLT, S2a wave 3) acceptance through the engine, one section per kind (`comb`, `formant`,
//! `ladder`): spectral aliasing with the shared harness and the bound recorded in `docs/dsp/<kind>.md`, `mix` 0 as an
//! exact pass-through, stability under fast modulation, a 40 s tail with zero subnormals, the same bits under every
//! FTZ mode, no allocation in `process`, and that two instances keep separate state.
//!
//! The module-level checks (the defining responses, caches, ramps) are in `src/modules/<kind>.rs`, and the closed
//! forms in `enth-dsp`.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{Note, SEED, SR, bits, drive, engine, events, id, note, render};
use enth_dsp::alias::{FFT_LEN, Measure, Sweep};
use enth_engine::module::count_subnormals;
use enth_patch::catalogue::{AdsrParam, CombParam, LfoParam, OscParam, Wave};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

#[cfg(debug_assertions)]
use assert_no_alloc::{AllocDisabler, assert_no_alloc};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

// ---------------------------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------------------------

/// `Patch::init()` with its oscillator on a sine: an effect's clean test input.
fn sine_patch() -> Patch {
    let mut p = Patch::init();
    p.generators[0].values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    p
}

/// A module of `kind` with the given values over its defaults.
fn module(name: &str, kind: ModuleKind, set: &[(usize, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), kind);
    for &(i, v) in set {
        m.values[i] = v;
    }
    m
}

fn with(mut patch: Patch, m: ModuleInstance) -> Patch {
    let section = m.kind.section();
    patch.section_mut(section).push(m);
    patch
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

/// `key` held through `patch` for `preroll` frames before the harness's window (the last 32,768 samples), so the
/// attack and any ring it excites are over: the left channel.
fn held_after(patch: &Patch, key: u8, preroll: usize) -> Vec<f32> {
    let frames = FFT_LEN + preroll;
    let (left, _, _) = render(patch, &[note(key, 0, frames as u64)], frames, 512, SEED);
    left
}

/// The aliasing sweep of `patch` over `keys`, printed for the docs, asserted below `bound_db`.
fn alias_sweep(patch: &Patch, keys: &[u8], preroll: usize, bound_db: f64, what: &str) -> Sweep {
    let sweep = Sweep::run_keys(keys, f64::from(SR), &Measure::REPORT_02, |key| {
        held_after(patch, key, preroll)
    });
    eprintln!("{what}:\n{}", sweep.table());
    sweep.assert_below(bound_db, what);
    sweep
}

/// Two overlapping notes with a glide-free release, the notes the pass-through and stability tests play.
fn phrase() -> Vec<Note> {
    vec![
        note(48, 0, 30_000),
        note(55, 4_000, 30_000),
        note(64, 12_000, 40_000),
    ]
}

/// A mid-bass and a high note, held 1 s, then a 39 s tail.
fn tail_notes() -> Vec<Note> {
    vec![
        note(36, 0, SR as u64),
        note(55, 0, SR as u64),
        note(72, 960, SR as u64),
        note(84, 1_920, SR as u64 + 4_800),
    ]
}

/// A patch with a long release, so the tail runs through the module for tens of seconds.
fn tail_patch(release_ms: f32, fx: Vec<ModuleInstance>) -> Patch {
    let mut p = Patch::init();
    p.modulators[0].values[AdsrParam::Release.index()] = release_ms;
    for m in fx {
        p = with(p, m);
    }
    p
}

struct Scan {
    output: usize,
    state_max: usize,
    non_finite: usize,
    active_at_end: usize,
}

fn scan(patch: &Patch, seconds: usize) -> Scan {
    let mut e = engine(patch, SEED);
    let mut s = Scan {
        output: 0,
        state_max: 0,
        non_finite: 0,
        active_at_end: 0,
    };
    let mut calls = 0usize;
    let sr = SR as usize;
    drive(
        &mut e,
        &events(&tail_notes()),
        seconds * sr,
        512,
        |e, l, r| {
            s.output += count_subnormals(l) + count_subnormals(r);
            s.non_finite += l.iter().chain(r).filter(|x| !x.is_finite()).count();
            if calls.is_multiple_of(16) {
                s.state_max = s.state_max.max(e.subnormals());
            }
            calls += 1;
        },
    );
    s.state_max = s.state_max.max(e.subnormals());
    s.active_at_end = e.active_voices().len();
    s
}

fn assert_clean_tail(name: &str, patch: &Patch) {
    let s = scan(patch, 40);
    assert_eq!(s.output, 0, "{name}: subnormal output samples");
    assert_eq!(s.state_max, 0, "{name}: subnormal values in engine state");
    assert_eq!(s.non_finite, 0, "{name}: non-finite output samples");
    assert_eq!(s.active_at_end, 0, "{name}: voices must end below -100 dB");
}

/// `patch` rendered over `phrase()`: both channels.
fn phrase_bits(patch: &Patch) -> (Vec<u32>, Vec<u32>) {
    let (l, r, _) = render(patch, &phrase(), 48_000, 100, SEED);
    (bits(&l), bits(&r))
}

/// A render of `patch` with the oscillator replaced by noise-like modulation: audio-rate LFOs on the given
/// destinations, 10 s of the phrase. Returns the peak and checks every sample is finite.
fn stress_peak(patch: &Patch, seconds: usize) -> f32 {
    let mut e = engine(patch, SEED);
    let mut peak = 0.0f32;
    let mut bad = 0usize;
    let sr = SR as usize;
    drive(&mut e, &events(&phrase()), seconds * sr, 512, |_, l, r| {
        for &x in l.iter().chain(r) {
            if x.is_finite() {
                peak = peak.max(x.abs());
            } else {
                bad += 1;
            }
        }
    });
    assert_eq!(bad, 0, "non-finite output samples");
    peak
}

/// Run `ev` through a fresh engine in 512-frame calls, each under the allocation guard (debug builds), and return
/// the left channel.
#[cfg(debug_assertions)]
fn guarded(patch: &Patch, ev: &[Note], frames: usize) -> Vec<f32> {
    use enth_engine::Event;
    let mut e = engine(patch, SEED);
    let events = events(ev);
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let mut chunk: Vec<Event> = Vec::with_capacity(events.len() + 1);
    let (mut start, mut next) = (0usize, 0usize);
    while start < frames {
        let end = (start + 512).min(frames);
        chunk.clear();
        while let Some(&(f, kind)) = events.get(next) {
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
    left
}

// ---------------------------------------------------------------------------------------------------------------
// FTZ: the same bits under every floating-point mode (copied from tail.rs's harness, which covers the P1 kinds)
// ---------------------------------------------------------------------------------------------------------------

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

    pub fn with_mode<R>(bits: u64, f: impl FnOnce() -> R) -> R {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | bits);
        // The result passes through `black_box` before the guard restores the old mode (see tail.rs).
        std::hint::black_box(f())
    }

    /// `patch` over the tail notes for `seconds`, in every mode: the same bits as with FTZ off.
    pub fn assert_same_in_every_mode(name: &str, patch: &Patch, seconds: usize) {
        let run = || {
            let mut e = engine(patch, SEED);
            let (l, r) = drive(
                &mut e,
                &events(&tail_notes()),
                seconds * SR as usize,
                512,
                |_, _, _| {},
            );
            (bits(&l), bits(&r))
        };
        let reference = with_mode(0, run);
        assert!(
            reference.0.iter().any(|&b| b != 0),
            "{name}: the render is silent"
        );
        for (mode, mode_bits) in fp::MODES {
            assert!(
                with_mode(mode_bits, run) == reference,
                "{name} under {mode}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// comb
// ---------------------------------------------------------------------------------------------------------------

fn comb(set: &[(CombParam, f32)]) -> ModuleInstance {
    let set: Vec<(usize, f32)> = set.iter().map(|&(p, v)| (p.index(), v)).collect();
    module("comb1", ModuleKind::Comb, &set)
}

/// The comb at its defaults and at its most resonant (feedback ±99 %, damping 20 kHz), a sine in, keys over six
/// octaves. The comb is linear and time-invariant at a static setting, so a sine comes out as a sine: whatever lands
/// between the harmonics is the engine's sine floor (about -93 dB), plus the ring the note's attack excites. At
/// positive feedback that ring is at the teeth, which are the sine's own harmonics, so it counts as signal; at
/// negative feedback the teeth are half way between them, and the ring decays with a time constant of 100 periods, so
/// that sweep waits seven seconds before its window and starts at key 57 (220 Hz: 0.45 s per time constant, so the ring is
/// down by 130 dB; at key 45 it would still show at -70 dB).
/// The bound is the worst of each sweep plus a margin.
#[test]
fn a_comb_adds_no_aliasing() {
    let keys = [33, 45, 57, 69, 81, 93, 105];
    let default = with(sine_patch(), comb(&[]));
    alias_sweep(
        &default,
        &keys,
        8_192,
        BOUND_COMB_DB,
        "comb, defaults, sine in",
    );
    let ring = with(
        sine_patch(),
        comb(&[(CombParam::Feedback, 99.0), (CombParam::Damping, 20_000.0)]),
    );
    alias_sweep(
        &ring,
        &keys,
        8_192,
        BOUND_COMB_DB,
        "comb, fb 99 %, damping 20 kHz, sine in",
    );
    let hollow = with(
        sine_patch(),
        comb(&[(CombParam::Feedback, -99.0), (CombParam::Damping, 20_000.0)]),
    );
    alias_sweep(
        &hollow,
        &keys[2..],
        7 * SR as usize,
        BOUND_COMB_DB,
        "comb, fb -99 %, damping 20 kHz, sine in",
    );
}

/// Measured 2026-10-26 (`docs/dsp/comb.md`): the worst point of the three sweeps plus 1.5 dB.
const BOUND_COMB_DB: f64 = -91.0;

/// The amplitude of the sine at `hz` in `x` (the last `n` samples), by correlation with a sine and a cosine at that
/// frequency: exact for a settled output, and blind to everything else in it.
fn amplitude_at(x: &[f32], hz: f64, n: usize) -> f64 {
    use enth_dsp::util::math::{cos, sin};
    let w = 2.0 * core::f64::consts::PI * hz / f64::from(SR);
    let start = x.len() - n;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x[start..].iter().enumerate() {
        let t = (start + i) as f64;
        re += f64::from(v) * sin(w * t);
        im += f64::from(v) * cos(w * t);
    }
    2.0 * (re * re + im * im).sqrt() / n as f64
}

/// A sine at `key` (`osc` on a sine, centre pan, steady level) through `m`, and the gain in dB: the sine's amplitude
/// out over the same sine without the module, measured over the last 32,768 of 240,000 frames (a ring at feedback
/// 90 % has fallen by 1e-5 by then). The sine is `440 · 2^((key − 69)/12)` Hz.
fn sine_gain_db(m: Option<ModuleInstance>, key: u8) -> f64 {
    let hz = 440.0 * 2.0f64.powf((f64::from(key) - 69.0) / 12.0);
    let mut patch = sine_patch();
    patch.modulators[0].values[AdsrParam::Sustain.index()] = 100.0;
    let dry = {
        let (l, _, _) = render(&patch, &[note(key, 0, 400_000)], 240_000, 512, SEED);
        amplitude_at(&l, hz, 32_768)
    };
    let patch = match m {
        Some(m) => with(patch, m),
        None => patch,
    };
    let (l, _, _) = render(&patch, &[note(key, 0, 400_000)], 240_000, 512, SEED);
    20.0 * (amplitude_at(&l, hz, 32_768) / dry).log10()
}

/// The defining response, end to end: a sine through the comb at three keys (C2, C4, C6), with the comb's `semi`
/// shifted so that the sine sits on its k-th tooth (`semi = −12·log2(k)`) or half way between two (k + ½), at
/// feedback +90 % and -90 %. The gain the engine gives is the closed form's (`enth_dsp::comb::response`) within
/// 0.05 dB, and the closed form says teeth of 0 dB (within 0.2) with valleys at `(1 − |fb|)/(1 + |fb|)` = -25.6 dB.
/// (A noise-in estimate of the same response was tried first: with teeth a few hertz wide the estimator's own noise,
/// about 1 dB, hid the shape, where a sine is exact.)
#[test]
fn the_comb_gain_at_its_teeth_and_valleys_is_the_closed_forms() {
    let valley_db = 20.0 * (0.1f64 / 1.9).log10();
    for fb in [90.0f32, -90.0] {
        for key in [36u8, 60, 84] {
            // On a tooth the sine is at k·f for fb > 0 and at (k − ½)·f for fb < 0 (the comb has gone down an octave).
            let teeth: &[f64] = if fb > 0.0 {
                &[1.0, 2.0, 3.0, 5.0]
            } else {
                &[0.5, 1.5, 2.5, 4.5]
            };
            let valleys: &[f64] = if fb > 0.0 {
                &[1.5, 2.5, 3.5]
            } else {
                &[1.0, 2.0, 3.0]
            };
            let sine_hz = 440.0 * 2.0f64.powf((f64::from(key) - 69.0) / 12.0);
            for (what, ratios, target_db, tol_db) in [
                ("tooth", teeth, 0.0, 0.2),
                ("valley", valleys, valley_db, 0.2),
            ] {
                for &r in ratios {
                    // The comb's frequency is the sine's over r: semi = −12·log2(r) at 100 % key tracking.
                    let semi = (-12.0 * r.log2()) as f32;
                    let semi = if semi == 0.0 { 0.0 } else { semi }; // a patch holds no -0.0
                    if sine_hz / r < 20.0 {
                        continue;
                    }
                    let m = comb(&[(CombParam::Feedback, fb), (CombParam::Semi, semi)]);
                    let measured = sine_gain_db(Some(m), key);
                    let t = enth_dsp::comb::Tuning::new(
                        enth_dsp::comb::frequency_hz(semi, f32::from(key) - 60.0, SR),
                        10_000.0,
                        SR,
                    );
                    let closed = 20.0
                        * enth_dsp::comb::response(
                            &t,
                            f64::from(fb) / 100.0,
                            sine_hz,
                            f64::from(SR),
                        )
                        .log10();
                    assert!(
                        (measured - closed).abs() < 0.05,
                        "fb {fb}, key {key}, {what} {r}: engine {measured:.3} dB, closed form {closed:.3} dB"
                    );
                    // The closed form's 0 dB teeth and the valleys' depth (the damping and the interpolator
                    // round the tooth a little).
                    assert!(
                        (closed - target_db).abs() < tol_db
                            || (what == "tooth" && closed < 0.0 && closed > -1.0),
                        "fb {fb}, key {key}, {what} {r}: closed form {closed:.3} dB, wanted {target_db:.2} dB"
                    );
                }
            }
        }
    }
    // No comb: the gain is 0 dB, so the measure itself is calibrated.
    assert!(sine_gain_db(None, 60).abs() < 1e-9);
}

/// A comb at `mix` 0 is the identity, bit for bit, however hard it is set: the patch with it renders the same bits
/// as the patch without, on both channels, over a phrase with overlapping notes, at a host block that is not a
/// multiple of the segment.
#[test]
fn mix_zero_renders_the_same_bits_as_no_comb() {
    let bare = Patch::init();
    let (bl, br) = phrase_bits(&bare);
    assert!(bl.iter().any(|&b| b != 0));
    for (name, m) in [
        ("defaults", comb(&[(CombParam::Mix, 0.0)])),
        (
            "fb 99, damping 500, semi -48",
            comb(&[
                (CombParam::Mix, 0.0),
                (CombParam::Feedback, 99.0),
                (CombParam::Damping, 500.0),
                (CombParam::Semi, -48.0),
            ]),
        ),
    ] {
        let (l, r) = phrase_bits(&with(Patch::init(), m));
        assert_eq!(l, bl, "{name}: left");
        assert_eq!(r, br, "{name}: right");
    }
}

/// Feedback 0 makes the comb the identity (`y = x`): nothing is delayed, so the filter has no latency.
#[test]
fn a_comb_has_zero_latency() {
    let bare = Patch::init();
    let (bl, br) = phrase_bits(&bare);
    let (l, r) = phrase_bits(&with(Patch::init(), comb(&[(CombParam::Feedback, 0.0)])));
    assert_eq!(l, bl);
    assert_eq!(r, br);
}

/// Two combs in a chain keep separate state: the second at `mix` 0 changes nothing about the first's render, and
/// the same two combs swapped give different (not mirrored) output.
#[test]
fn two_combs_keep_separate_state() {
    let a = [
        (CombParam::Feedback, 85.0),
        (CombParam::Damping, 4_000.0),
        (CombParam::Semi, 0.0),
    ];
    let b = [(CombParam::Feedback, -60.0), (CombParam::Semi, 7.0)];
    let one = phrase_bits(&with(Patch::init(), comb(&a)));
    let silent_second = with(
        with(Patch::init(), comb(&a)),
        module(
            "comb2",
            ModuleKind::Comb,
            &[
                (CombParam::Mix.index(), 0.0),
                (CombParam::Feedback.index(), 99.0),
            ],
        ),
    );
    assert_eq!(phrase_bits(&silent_second), one);
    let to_inst = |name: &str, set: &[(CombParam, f32)]| {
        let set: Vec<(usize, f32)> = set.iter().map(|&(p, v)| (p.index(), v)).collect();
        module(name, ModuleKind::Comb, &set)
    };
    let ab = with(
        with(Patch::init(), to_inst("comb1", &a)),
        to_inst("comb2", &b),
    );
    let ba = with(
        with(Patch::init(), to_inst("comb1", &b)),
        to_inst("comb2", &a),
    );
    let (xab, xba) = (phrase_bits(&ab), phrase_bits(&ba));
    assert_ne!(xab, one);
    assert_ne!(
        xab, xba,
        "the order matters (the interpolation and the loops are not commutative bit for bit)"
    );
    // And the chain is the same render twice.
    assert_eq!(phrase_bits(&ab), xab);
}

/// An LFO at 50 Hz (the top of its range) sweeping the comb's pitch over the full range, another on the feedback
/// and a third on the damping, at feedback 99 %: finite and bounded for 10 s of two overlapping notes.
#[test]
fn a_comb_survives_audio_rate_modulation_of_everything() {
    let mut p = Patch::init();
    p = with(
        p,
        comb(&[(CombParam::Feedback, 90.0), (CombParam::Damping, 12_000.0)]),
    );
    for (i, rate) in [50.0f32, 37.0, 43.0].into_iter().enumerate() {
        let mut lfo = ModuleInstance::new(id(&format!("lfo{}", i + 1)), ModuleKind::Lfo);
        lfo.values[LfoParam::Rate.index()] = rate;
        p.modulators.push(lfo);
    }
    p.routes = vec![
        route("lfo1", "comb1.semi", 100.0),
        route("lfo2", "comb1.feedback", 100.0),
        route("lfo3", "comb1.damping", 100.0),
    ];
    let peak = stress_peak(&p, 10);
    let dry = stress_peak(&Patch::init(), 10);
    eprintln!("comb under audio-rate modulation: peak {peak}, dry {dry}");
    assert!(peak < 1.5 * dry, "peak {peak}, dry {dry}");
}

#[test]
fn a_40_s_tail_through_combs_holds_no_subnormals() {
    let ring = comb(&[
        (CombParam::Feedback, 99.0),
        (CombParam::Damping, 20_000.0),
        (CombParam::Semi, -12.0),
    ]);
    let mut slow = comb(&[
        (CombParam::Feedback, -90.0),
        (CombParam::Keytrack, 0.0),
        (CombParam::Semi, -48.0),
    ]);
    slow.id = id("comb2");
    assert_clean_tail(
        "comb x2, 3 s release",
        &tail_patch(3_000.0, vec![ring.clone(), slow.clone()]),
    );
    assert_clean_tail(
        "comb x2, 20 s release",
        &tail_patch(20_000.0, vec![ring, slow]),
    );
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn a_comb_renders_the_same_bits_under_every_ftz_mode() {
    let p = tail_patch(
        3_000.0,
        vec![comb(&[
            (CombParam::Feedback, 97.0),
            (CombParam::Damping, 3_000.0),
        ])],
    );
    ftz::assert_same_in_every_mode("comb, 20 s", &p, 20);
}

#[cfg(debug_assertions)]
#[test]
fn a_comb_never_allocates_in_process() {
    let mut p = Patch::init();
    p = with(p, comb(&[(CombParam::Feedback, 95.0)]));
    let mut lfo = ModuleInstance::new(id("lfo1"), ModuleKind::Lfo);
    lfo.values[LfoParam::Rate.index()] = 30.0;
    p.modulators.push(lfo);
    p.routes = vec![route("lfo1", "comb1.semi", 80.0)];
    let out = guarded(&p, &phrase(), 48_000);
    assert!(out.iter().all(|x| x.is_finite()) && out.iter().any(|&x| x != 0.0));
}
