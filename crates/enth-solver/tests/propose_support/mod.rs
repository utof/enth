//! Shared by lane ONX's kernel tests (`propose_kernels.rs`, `propose_parity.rs`): the FTZ/DAZ modes (as
//! `propose_ftz.rs` sets them), deterministic weights, hex, and the raw tensor file the parity fixtures use.
#![allow(
    dead_code,
    reason = "each test file uses a different part of the support module"
)]

/// Lowercase hex.
pub fn hex(b: &[u8]) -> String {
    use core::fmt::Write as _;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// `SplitMix64`, as `propose::net::synthetic_input` uses it.
pub fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Deterministic weights: `n` values in [−scale, scale), multiples of 2⁻²³ · scale (scale a power of two keeps
/// them exact).
pub fn values(seed: u64, n: usize, scale: f32) -> Vec<f32> {
    let mut s = seed ^ 0x5eed_0000;
    (0..n)
        .map(|_| ((splitmix64(&mut s) >> 40) as i32 - (1 << 23)) as f32 / (1 << 23) as f32 * scale)
        .collect()
}

/// One tensor of a raw tensor file.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub name: String,
    pub dims: Vec<u32>,
    pub values: Vec<f32>,
}

/// Reads a raw tensor file (`ENTHTNS1`, written by `ml/enth_ml/export/tensors.py` and read by
/// `tools/model-check`): the magic, a `u32` count, then per tensor a name (`u8` length and bytes), a rank (`u8`),
/// the dims (`u32` each) and the values (`f32`), all little-endian.
pub fn read_tensors(b: &[u8]) -> Vec<Tensor> {
    assert_eq!(&b[..8], b"ENTHTNS1", "not a tensor file");
    let mut at = 8;
    let mut take = |n: usize| {
        let s = &b[at..at + n];
        at += n;
        s
    };
    let u32_of = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let count = u32_of(take(4));
    let mut out = Vec::new();
    for _ in 0..count {
        let n = usize::from(take(1)[0]);
        let name = String::from_utf8(take(n).to_vec()).unwrap();
        let rank = usize::from(take(1)[0]);
        let dims: Vec<u32> = (0..rank).map(|_| u32_of(take(4))).collect();
        let len = dims.iter().product::<u32>() as usize;
        let values = take(4 * len)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        out.push(Tensor { name, dims, values });
    }
    out
}

#[cfg(target_arch = "x86_64")]
#[allow(
    unsafe_code,
    reason = "emulates a host that sets FTZ/DAZ by writing MXCSR, in a test only"
)]
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
        ("FTZ only", FTZ),
        ("FTZ+DAZ", FTZ | DAZ),
    ];
}

#[cfg(target_arch = "aarch64")]
#[allow(
    unsafe_code,
    reason = "emulates a host that sets FZ by writing FPCR, in a test only"
)]
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

/// The modes a host may run us under: every FTZ/DAZ setting this target has (wasm32 has none: IEEE only).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn modes() -> Vec<(&'static str, u64)> {
    fp::MODES.to_vec()
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn modes() -> Vec<(&'static str, u64)> {
    vec![("IEEE (no FTZ mode on this target)", 0)]
}

/// Runs `f` with the FTZ/DAZ bits set to `bits`, restoring the previous bits even if `f` panics.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn with_mode<R>(bits: u64, f: impl FnOnce() -> R) -> R {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            fp::set(self.0);
        }
    }
    let old = fp::get();
    let _restore = Restore(old);
    fp::set((old & !fp::MASK) | bits);
    // The result passes through `black_box` before the guard restores the old mode, so the optimiser cannot sink the
    // closure's arithmetic past the restore (it did on linux-aarch64 in release: golden.yml on 5082fec and 3a573e1).
    std::hint::black_box(f())
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn with_mode<R>(_bits: u64, f: impl FnOnce() -> R) -> R {
    f()
}
