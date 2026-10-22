//! The model's side of determinism does not depend on FTZ or DAZ (docs/r3.md, the determinism contract, point 1;
//! the F3s judge's S4): feature set 1 and decoding give the same bits with FTZ off, FTZ only and FTZ+DAZ, as renders
//! do (`enth-engine/tests/tail.rs`). Lane ONX runs the model's output hashes the same way.
//!
//! Setting MXCSR/FPCR is documented undefined behaviour in Rust; it is done only here, in a test, to emulate hosts
//! (nice-plug can set FTZ around `process`, a DAW may set both, the CLI and wasm set nothing).
#![allow(
    unsafe_code,
    reason = "the FTZ test emulates a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

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
        ("FTZ only", FTZ),
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
    use super::fp;
    use enth_challenge::phrases::{HELD, phrase};
    use enth_solver::propose::decode::{Hearing, OutputLayout};
    use enth_solver::propose::features::{Cue, ModelInput};
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
        // The result passes through `black_box` before the guard restores the old mode, so the optimiser cannot
        // sink the closure's arithmetic past the restore (it did on linux-aarch64 in release, 5082fec's golden run).
        black_box(f())
    }

    #[test]
    fn ftz_really_changes_float_behaviour_here() {
        let product = |a: f32, b: f32| black_box(black_box(a) * black_box(b));
        assert!(with_mode(0, || product(1e-30, 1e-10)).is_subnormal());
        assert_eq!(with_mode(fp::MODES[1].1, || product(1e-30, 1e-10)), 0.0);
    }

    /// A pluck at 44.1 kHz, and a tone whose tail decays through the subnormal range (its last samples are
    /// subnormal, which DAZ reads as zero).
    fn clips() -> Vec<(Vec<f32>, u32)> {
        let pluck: Vec<f32> = (0..57_330)
            .map(|i| {
                let t = f64::from(i) / 44_100.0;
                ((2.0 * (t * 196.0).fract() - 1.0) * libm::exp(-3.0 * t)) as f32 * 0.5
            })
            .collect();
        let tail: Vec<f32> = (0..96_000)
            .map(|i| {
                let t = f64::from(i) / 48_000.0;
                (libm::sin(core::f64::consts::TAU * 330.0 * t) * libm::exp(-55.0 * t)) as f32
            })
            .collect();
        assert!(
            tail.iter().any(|v| v.is_subnormal()),
            "the tail reaches subnormals"
        );
        vec![(pluck, 44_100), (tail, 48_000)]
    }

    #[test]
    fn feature_set_1_is_the_same_under_every_ftz_mode() {
        let cue = Cue {
            key: 55,
            f0_hz: Some(196.0),
            on_s: 0.0,
            hold_s: 1.0,
            bandwidth_hz: Some(9_000.0),
        };
        for (i, (x, sr)) in clips().iter().enumerate() {
            let run = || ModelInput::compute(x, *sr, &cue).unwrap().sha256();
            let reference = with_mode(0, run);
            for (mode, bits) in fp::MODES {
                assert_eq!(with_mode(bits, run), reference, "clip {i} under {mode}");
            }
        }
    }

    #[test]
    fn decoding_is_the_same_under_every_ftz_mode() {
        let l = OutputLayout::v1();
        let h = Hearing::of_phrase(phrase(HELD).unwrap());
        let mut row = l.encode(l.template(), &h).unwrap().values;
        // A subnormal structure logit and subnormal values: they read as 0 in every mode.
        row[l.enables()[0].offset] = 1e-40;
        row[0] = 1e-39;
        row[l.continuous()[10].offset] = -1e-41;
        let run = || l.decode(&row, &h).unwrap();
        let reference = with_mode(0, run);
        assert!(
            !reference
                .find(&l.enables()[0].id)
                .is_some_and(|(_, m)| m.enabled),
            "a subnormal logit is not above 0"
        );
        for (mode, bits) in fp::MODES {
            assert_eq!(with_mode(bits, run), reference, "under {mode}");
        }
    }
}
