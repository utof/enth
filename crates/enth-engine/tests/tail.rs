//! Denormals and FTZ (design.md §5, rule 4): a 40 s release tail through one module of every kind holds zero
//! subnormals in the output and in the engine state, every voice ends, every sample is finite, and the render is
//! bit-identical with FTZ off, FTZ only and FTZ+DAZ.
//!
//! The tail patch has one module of every kind, so each lane that replaces a stub (the SVF's integrators, the
//! reverb's feedback network, the compressor's detector) is covered here without editing this file. Setting
//! MXCSR/FPCR is documented undefined behaviour in Rust; it is done only here, in a test, to emulate hosts (nice-plug
//! sets FTZ around `process`, a DAW may set both, the CLI and wasm set nothing).
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

mod common;

use common::{SEED, bits, drive, engine, events, every_kind_patch, note, saw_patch};
use enth_engine::module::count_subnormals;

const SR: usize = 48_000;

/// Four notes held 1 s, then a 39 s tail.
fn tail_notes() -> Vec<common::Note> {
    vec![
        note(48, 0, SR as u64),
        note(55, 0, SR as u64),
        note(60, 960, SR as u64),
        note(64, 1920, SR as u64 + 4800),
    ]
}

struct Scan {
    output: usize,
    state_max: usize,
    non_finite: usize,
    active_at_end: usize,
}

fn scan(patch: &enth_patch::Patch, seconds: usize, block: usize) -> Scan {
    let mut e = engine(patch, SEED);
    let mut s = Scan {
        output: 0,
        state_max: 0,
        non_finite: 0,
        active_at_end: 0,
    };
    let mut calls = 0usize;
    drive(
        &mut e,
        &events(&tail_notes()),
        seconds * SR,
        block,
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

#[test]
fn a_40_s_release_tail_has_no_subnormals_and_every_voice_ends() {
    for (name, patch) in [
        ("every kind, 3 s release", every_kind_patch(3000.0)),
        ("every kind, 20 s release", every_kind_patch(20_000.0)),
        ("saw", saw_patch()),
    ] {
        let s = scan(&patch, 40, 512);
        assert_eq!(s.output, 0, "{name}: subnormal output samples");
        assert_eq!(s.state_max, 0, "{name}: subnormal values in engine state");
        assert_eq!(s.non_finite, 0, "{name}: non-finite output samples");
        assert_eq!(s.active_at_end, 0, "{name}: voices must end below -100 dB");
    }
}

#[test]
fn the_scan_can_see_subnormals() {
    // The counter the engine and the modules use, on a value that is subnormal.
    assert_eq!(
        count_subnormals(&[0.0, f32::MIN_POSITIVE / 2.0, 1.0, -1e-40]),
        2
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
    use std::hint::black_box;

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
        // The result passes through `black_box` before the guard restores the old mode, so the optimiser cannot sink the
        // closure's arithmetic past the restore (it did on linux-aarch64 in release: golden.yml on 5082fec and 3a573e1).
        std::hint::black_box(f())
    }

    #[test]
    fn ftz_really_changes_float_behaviour_here() {
        // 1e-30 * 1e-10 = 1e-40: subnormal under IEEE, zero under FTZ. Without this the test below could pass
        // vacuously.
        let product = |a: f32, b: f32| black_box(black_box(a) * black_box(b));
        assert!(with_mode(0, || product(1e-30, 1e-10)).is_subnormal());
        assert_eq!(with_mode(fp::MODES[1].1, || product(1e-30, 1e-10)), 0.0);
    }

    #[test]
    fn the_render_is_the_same_under_every_ftz_mode() {
        for (name, patch, seconds) in [
            ("every kind, 40 s tail", every_kind_patch(3000.0), 40),
            ("saw phrase", saw_patch(), 3),
        ] {
            let run = || {
                let mut e = engine(&patch, SEED);
                let (l, r) = drive(
                    &mut e,
                    &events(&tail_notes()),
                    seconds * SR,
                    512,
                    |_, _, _| {},
                );
                (bits(&l), bits(&r))
            };
            let reference = with_mode(0, run);
            for (mode, mode_bits) in fp::MODES {
                assert!(
                    with_mode(mode_bits, run) == reference,
                    "{name} under {mode}"
                );
            }
        }
    }
}
