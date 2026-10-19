//! FTZ on vs off give an identical hash. The engine flushes in code, so the host's FTZ/DAZ setting
//! (nice-plug sets FTZ, a DAW may set both, the CLI and wasm set nothing) cannot change a render.
//! The negative control shows the test has teeth: an engine that does not flush *does* change.
//!
//! Setting MXCSR/FPCR is documented UB in Rust and is done here, in a test, only to emulate hosts.
#![cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#![allow(
    unsafe_code,
    reason = "tests emulate a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

use std::hint::black_box;

use spike_engine::render::{fast_decay_patch, frames, held_chord, phrase, tail_notes};
use spike_engine::{Flush, Patch, Path, hash_samples, render};

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
        unsafe { asm!("mrs {}, fpcr", out(reg) v, options(nomem, nostack, preserves_flags)) };
        v
    }
    pub fn set(v: u64) {
        // SAFETY: writes the FPCR system register; only FZ differs from the current value.
        unsafe { asm!("msr fpcr, {}", in(reg) v, options(nomem, nostack)) };
    }
    pub const MODES: [(&str, u64); 2] = [("IEEE (FZ off)", 0), ("FZ on", FZ)];
}

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
    f()
}

#[test]
fn ftz_really_changes_float_behaviour_here() {
    // 1e-30 * 1e-10 = 1e-40: subnormal under IEEE, zero under FTZ. Without this the tests below could pass vacuously.
    let product = |a: f32, b: f32| black_box(a) * black_box(b);
    assert!(with_mode(0, || product(1e-30, 1e-10)).is_subnormal());
    assert_eq!(with_mode(fp::MODES[1].1, || product(1e-30, 1e-10)), 0.0);
}

#[test]
fn flushed_engine_hash_is_independent_of_ftz() {
    let cases = [
        (
            "simple 16v x8",
            Patch::simple(),
            held_chord(16, 2.0),
            2.0,
            Path::X8,
        ),
        (
            "p1 held scalar",
            Patch::p1_like(),
            held_chord(8, 2.0),
            2.0,
            Path::Scalar,
        ),
        (
            "p1 phrase scalar",
            Patch::p1_like(),
            phrase(),
            4.0,
            Path::Scalar,
        ),
        (
            "p1 40 s tail",
            Patch::p1_like(),
            tail_notes(),
            40.0,
            Path::Scalar,
        ),
        (
            "fast-decay 12 s tail x4",
            fast_decay_patch(),
            tail_notes(),
            12.0,
            Path::X4,
        ),
    ];
    for (name, patch, notes, seconds, path) in cases {
        let run = || {
            hash_samples(&render(
                &patch,
                &notes,
                frames(seconds),
                0x0E17_5EED,
                path,
                Flush::On,
            ))
        };
        let reference = with_mode(0, run);
        for (mode, bits) in fp::MODES {
            assert_eq!(with_mode(bits, run), reference, "{name} under {mode}");
        }
        println!(
            "flush on, {name}: identical hash {} under every FTZ/DAZ mode",
            &reference[..16]
        );
    }
}

#[test]
fn negative_control_unflushed_engine_depends_on_ftz() {
    let patch = fast_decay_patch();
    let notes = tail_notes();
    let run = || {
        hash_samples(&render(
            &patch,
            &notes,
            frames(12.0),
            0x0E17_5EED,
            Path::Scalar,
            Flush::Off,
        ))
    };
    let ieee = with_mode(0, run);
    let ftz = with_mode(fp::MODES[1].1, run);
    println!("flush OFF, 12 s fast-decay tail: hash under IEEE {ieee}, under FTZ {ftz}");
    assert_ne!(
        ieee, ftz,
        "an unflushed engine must change under FTZ, otherwise this test cannot detect a regression"
    );
}
