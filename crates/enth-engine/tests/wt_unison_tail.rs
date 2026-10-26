//! `wt` unison over a long time and under every floating-point mode (S2a lane UNI): a 40 s release tail through
//! seven copies with feedback at its maximum leaves no subnormal output sample or state value and ends every voice,
//! and the render, both channels, is the same bits whether the host runs with flush-to-zero set or not. The lanes'
//! history is flushed in code (`enth_dsp::util::flush`), never by the host's FTZ.
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{SEED, SR, bits, drive, engine, events, id, note};
use enth_engine::module::count_subnormals;
use enth_patch::catalogue::{AdsrParam, OscParam, PhaseMode, Warp, Wave, WtParam, WtTable};
use enth_patch::{ModuleInstance, ModuleKind, Patch, Route};

fn wt(name: &str, set: &[(WtParam, f32)]) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Wt);
    for &(param, value) in set {
        m.values[param.index()] = value;
    }
    m
}

fn route(src: &str, dst: &str, depth: f32) -> Route {
    Route {
        src: src.parse().unwrap(),
        dst: dst.parse().unwrap(),
        depth,
    }
}

/// A modulator `osc` (a sine at level 0, heard only through its route).
fn modulator(name: &str, semi: f32) -> ModuleInstance {
    let mut m = ModuleInstance::new(id(name), ModuleKind::Osc);
    m.values[OscParam::Wave.index()] = Wave::Sine.index() as f32;
    m.values[OscParam::PhaseMode.index()] = PhaseMode::Reset.index() as f32;
    m.values[OscParam::Semi.index()] = semi;
    m.values[OscParam::Level.index()] = 0.0;
    m
}

/// Seven copies of a feeding-back, phase-modulated, warped `wt` (the FM table, moving between frames by `am`'s route)
/// and a second three-copy `wt` it modulates, under a gate envelope with a 3 s release.
fn patch() -> Patch {
    let mut p = Patch::init();
    p.generators.clear();
    let env = &mut p.modulators[0].values;
    env[AdsrParam::Attack.index()] = 0.0;
    env[AdsrParam::Decay.index()] = 0.0;
    env[AdsrParam::Sustain.index()] = 100.0;
    env[AdsrParam::Release.index()] = 3_000.0;
    p.generators.push(modulator("osc1", 7.0));
    p.generators.push(wt(
        "wt1",
        &[
            (WtParam::Table, WtTable::Fm.index() as f32),
            (WtParam::Position, 35.0),
            (WtParam::Warp, Warp::Asymmetric.index() as f32),
            (WtParam::WarpAmount, 40.0),
            (WtParam::PmIndex, 4.0),
            (WtParam::Feedback, 100.0),
            (WtParam::Unison, 7.0),
            (WtParam::Detune, 100.0),
            (WtParam::Blend, 40.0),
            (WtParam::Spread, 100.0),
        ],
    ));
    p.generators.push(wt(
        "wt2",
        &[
            (WtParam::Semi, -12.0),
            (WtParam::PmIndex, 2.0),
            (WtParam::Feedback, 60.0),
            (WtParam::Unison, 3.0),
            (WtParam::Detune, 9.0),
        ],
    ));
    p.routes.push(route("osc1", "wt1.pm", 80.0));
    p.routes.push(route("wt1", "wt2.pm", 50.0));
    p
}

fn tail_notes() -> Vec<common::Note> {
    vec![
        note(36, 0, SR as u64),
        note(48, 0, SR as u64),
        note(60, 960, SR as u64),
        note(84, 1920, SR as u64 + 4800),
    ]
}

#[test]
fn a_40_s_release_tail_has_no_subnormals_and_every_voice_ends() {
    let mut e = engine(&patch(), SEED);
    assert!(e.plan().is_stereo());
    let (mut output, mut state, mut non_finite) = (0usize, 0usize, 0usize);
    let mut calls = 0usize;
    let mut heard = false;
    let (l, _) = drive(
        &mut e,
        &events(&tail_notes()),
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
    heard |= l.iter().any(|x| x.abs() > 0.1);
    state = state.max(e.subnormals());
    assert!(heard, "the patch makes sound");
    assert_eq!(output, 0, "subnormal output samples");
    assert_eq!(state, 0, "subnormal values in the engine state");
    assert_eq!(non_finite, 0, "non-finite samples");
    assert!(
        e.active_voices().is_empty(),
        "every voice ends below -100 dB"
    );
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
    let run = || {
        let mut e = engine(&patch(), SEED);
        let (l, r) = drive(
            &mut e,
            &events(&tail_notes()),
            6 * SR as usize,
            512,
            |_, _, _| {},
        );
        (bits(&l), bits(&r))
    };
    let with_mode = |mode: u64| {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | mode);
        std::hint::black_box(run())
    };
    let reference = with_mode(0);
    assert!(reference.0 != reference.1, "the render is wide");
    for (name, mode) in fp::MODES {
        assert!(with_mode(mode) == reference, "the render under {name}");
    }
}
