//! Helpers of the `chorus` and `phaser` integration tests (S2a lane MOD), kept apart from `common/mod.rs` so that
//! file stays untouched: the sweep's phase from the tempo, a windowed-sinc fractional read, a lock-in amplifier, and
//! the FTZ/DAZ modes. Included with `#[path = "common/modfx.rs"] mod modfx;`.
#![allow(dead_code, reason = "each test binary uses a different subset")]
#![allow(
    unsafe_code,
    reason = "the FTZ helpers emulate a host that sets FTZ/DAZ by writing MXCSR/FPCR directly"
)]

use enth_dsp::util::math;
use enth_engine::{EventKind, SEG};

/// The sample rate as `f64`.
pub const FS: f64 = 48_000.0;

/// The tempo in force at segment `k`: `schedule` is `(frame, bpm)` in ascending frame order, every frame a multiple of
/// [`SEG`] (an event applies from the segment that starts at its frame); `initial` before the first.
pub fn bpm_at(schedule: &[(u64, f64)], initial: f64, k: usize) -> f64 {
    let frame = (k * SEG) as u64;
    schedule
        .iter()
        .rev()
        .find(|(f, _)| *f <= frame)
        .map_or(initial, |&(_, bpm)| bpm)
}

/// The tempo events of `schedule`, as `(frame, event)` pairs for the drivers.
pub fn tempo_events(schedule: &[(u64, f64)]) -> Vec<(u64, EventKind)> {
    schedule
        .iter()
        .map(|&(f, bpm)| (f, EventKind::Tempo { bpm }))
        .collect()
}

/// The first segment in which the module's input is not silent in either channel: the module wakes there and its
/// sweep starts at phase 0 (a module whose input and state are silent is skipped).
pub fn first_active_segment(l: &[f32], r: &[f32]) -> usize {
    let n = l
        .iter()
        .zip(r)
        .position(|(a, b)| *a != 0.0 || *b != 0.0)
        .expect("the dry signal is not silent");
    n / SEG
}

/// The sweep's position in cycles at every frame, as the modules compute it: the phase of frame `i` of segment `k` is
/// the cycles of the segments before it, plus `(i + 1)/32` of this segment's. `cycles_per_segment(k)` is the exact
/// value (in `f64`) for segment `k`; segments before `k0` do not count (the module sleeps).
pub fn sweep_cycles(
    frames: usize,
    k0: usize,
    cycles_per_segment: impl Fn(usize) -> f64,
) -> Vec<f64> {
    let mut out = vec![0.0; frames];
    let mut done = 0.0;
    for k in k0..frames / SEG {
        let c = cycles_per_segment(k);
        for i in 0..SEG {
            out[k * SEG + i] = done + c * (i + 1) as f64 / SEG as f64;
        }
        done += c;
    }
    out
}

/// Cycles per segment of a sweep that lasts `num/den` beats at `bpm` (`f64`, from the definition).
pub fn cycles_per_segment_synced(bpm: f64, beats: (u32, u32)) -> f64 {
    let (num, den) = beats;
    bpm * f64::from(den) * SEG as f64 / (60.0 * f64::from(num) * FS)
}

/// The signal `x` read at the fractional position `t` (in samples) by a 64-point Blackman-windowed sinc, in `f64`:
/// the reference a fractional delay is held to. Positions outside the signal read as zero.
pub fn sinc_at(x: &[f32], t: f64) -> f64 {
    let n0 = t.floor() as i64;
    // The fraction first, so `sin(π·d)` is taken of a small argument: `d = f − k`, `sin(π·d) = (−1)^k·sin(π·f)`.
    let f = t - n0 as f64;
    let sf = math::sin(core::f64::consts::PI * f);
    let mut acc = 0.0;
    for k in -31..=32_i64 {
        let i = n0 + k;
        if i < 0 || i as usize >= x.len() {
            continue;
        }
        let d = f - k as f64;
        let sinc = if d.abs() < 1e-9 {
            1.0
        } else {
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            sign * sf / (core::f64::consts::PI * d)
        };
        let a = core::f64::consts::PI * d / 32.0;
        let window = 0.42 + 0.5 * math::cos(a) + 0.08 * math::cos(2.0 * a);
        acc += f64::from(x[i as usize]) * sinc * window;
    }
    acc
}

/// A triangle sweep, `−1 … 1`, from a position in cycles: rises from 0 at 0, peaks at a quarter (the same shape as
/// `enth_dsp::chorus::triangle_of`, written out from the definition).
pub fn triangle(cycles: f64) -> f64 {
    let u = cycles.rem_euclid(1.0);
    if u < 0.25 {
        4.0 * u
    } else if u < 0.75 {
        2.0 - 4.0 * u
    } else {
        4.0 * u - 4.0
    }
}

/// A sine sweep, `−1 … 1`, from a position in cycles.
pub fn sine(cycles: f64) -> f64 {
    math::sin(core::f64::consts::TAU * cycles.rem_euclid(1.0))
}

/// The complex amplitude of `x[start..start + len]` at `freq_hz` under a Hann window: `Σ w[n]·x[n]·e^(−jωn)`.
pub fn lock_in(x: &[f64], start: usize, len: usize, freq_hz: f64) -> (f64, f64) {
    let w = core::f64::consts::TAU * freq_hz / FS;
    let (mut re, mut im) = (0.0, 0.0);
    for n in 0..len {
        let h = 0.5 - 0.5 * math::cos(core::f64::consts::TAU * (n as f64 + 0.5) / len as f64);
        let v = x[start + n] * h;
        let a = w * (start + n) as f64;
        re += v * math::cos(a);
        im -= v * math::sin(a);
    }
    (re, im)
}

/// `a / b` for complex numbers.
pub fn cdiv(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let d = b.0 * b.0 + b.1 * b.1;
    ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
}

/// `f32` samples as `f64`.
pub fn to_f64(x: &[f32]) -> Vec<f64> {
    x.iter().map(|v| f64::from(*v)).collect()
}

/// The RMS of `x[from..]`.
pub fn rms(x: &[f32], from: usize) -> f64 {
    let s: f64 = x[from..]
        .iter()
        .map(|v| f64::from(*v) * f64::from(*v))
        .sum();
    (s / (x.len() - from) as f64).sqrt()
}

/// The peak of `|x[from..]|`.
pub fn peak(x: &[f32], from: usize) -> f64 {
    x[from..].iter().fold(0.0, |m, v| m.max(f64::from(v.abs())))
}

/// 20·log10 of a ratio.
pub fn db(ratio: f64) -> f64 {
    20.0 * math::log10(ratio)
}

#[cfg(target_arch = "x86_64")]
pub mod fp {
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
pub mod fp {
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
pub mod ftz {
    use super::fp;

    /// Restores the previous FTZ/DAZ bits even if the closure panics.
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }

    /// Run `f` with the FTZ/DAZ bits set to `bits` (the others as they were).
    pub fn with_mode<R>(bits: u64, f: impl FnOnce() -> R) -> R {
        let old = fp::get();
        let _restore = Restore(old);
        fp::set((old & !fp::MASK) | bits);
        // Through `black_box` before the guard restores the old mode, so the optimiser cannot sink the arithmetic
        // past the restore (tests/tail.rs).
        std::hint::black_box(f())
    }
}
