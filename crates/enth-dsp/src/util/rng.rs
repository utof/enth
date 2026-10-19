//! Seeded randomness: `SplitMix64`, [`derive()`] and [`fnv1a64`], the pieces of the engine's seed rule.
//!
//! All randomness in enth is seeded and passed in (AGENTS.md, design.md §5). There is no global RNG, no system time
//! and no thread-local state here: a module gets a `u64` seed from the engine and draws from its own [`Rng`]. The
//! engine derives that seed from the render seed, the module id and the note index, **never from parameter values or
//! the patch hash**:
//!
//! ```text
//! module_key = fnv1a64(module_id bytes)                       stable when other modules come and go
//! per voice:          seed = derive(derive(render_seed, module_key), note_index)
//! per global module:  seed = derive(render_seed, module_key)
//! ```
//!
//! (`enth_engine::seed` applies the rule; this file only provides the arithmetic.) Everything is integer arithmetic
//! with wrapping operations, and the only float steps are exact integer-to-float conversions, so every stream is the
//! same on every platform, in debug and release, native and wasm. The generator and `derive` are the Phase 0 engine
//! spike's (`spikes/spike-engine/src/prng.rs`), unchanged, so its measurements carry over.
//!
//! Changing any constant or formula here changes every seeded render: it is an engine-version change and updates the
//! goldens.

/// Derive an independent stream seed from a parent seed and a stream number, e.g. `derive(render_seed, module_key)`.
///
/// One `SplitMix64` step over `seed ^ stream·K` (K odd, from the spike): nearby inputs give unrelated outputs, and
/// `derive(derive(a, b), c)` nests without correlation between the levels for the sizes used here.
#[inline]
#[must_use]
pub const fn derive(seed: u64, stream: u64) -> u64 {
    splitmix_step(seed ^ stream.wrapping_mul(0xD6E8_FEB8_6659_FD93)).1
}

/// FNV-1a, 64 bit, over `bytes`: the stable key of a module id (`fnv1a64(b"osc1")`). Not a cryptographic hash; it
/// only has to be stable and spread short ASCII ids.
#[must_use]
pub const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

/// One `SplitMix64` step: (new state, output).
#[inline(always)]
const fn splitmix_step(state: u64) -> (u64, u64) {
    let s = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (s, z ^ (z >> 31))
}

/// `SplitMix64` (Steele, Lea and Flood 2014; the public-domain reference by Sebastiano Vigna): a small, fast,
/// full-period 64-bit generator. Copy it to fork a stream; `derive` a new seed for an independent one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// A generator whose stream is fixed by `seed`.
    #[inline]
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let (s, out) = splitmix_step(self.0);
        self.0 = s;
        out
    }

    /// The next 32 random bits (the high half of [`next_u64`](Self::next_u64)).
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)` with 24 random bits, so every value is exact in `f32` (the spike's `unit`).
    #[inline]
    pub fn unit_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Uniform in `[0, 1)` with 53 random bits, exact in `f64`.
    #[inline]
    pub fn unit_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / 9_007_199_254_740_992.0
    }

    /// Uniform in `[-1, 1)`: `2·unit − 1`, exact in `f32`.
    #[inline]
    pub fn bipolar_f32(&mut self) -> f32 {
        2.0 * self.unit_f32() - 1.0
    }

    /// Uniform in `[lo, hi)`: `lo + (hi − lo)·unit`, plain IEEE operations in this order.
    #[inline]
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit_f32()
    }

    /// Uniform integer in `0..n` for `n > 0` (`n == 0` gives 0). Modulo bias is below 2⁻³² for the sizes used here.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_the_reference_vectors() {
        // Test vectors from the FNV reference (Fowler, Noll, Vo; draft-eastlake-fnv).
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        // Ids that differ by one character get unrelated keys.
        assert_ne!(fnv1a64(b"osc1"), fnv1a64(b"osc2"));
    }

    #[test]
    fn splitmix64_matches_the_reference_stream() {
        // Vigna's splitmix64.c with x = 0: the first outputs.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(r.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(r.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn derive_is_pinned() {
        // Pinned values: a change here changes every seeded render (an engine-version change).
        assert_eq!(derive(0, 0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(derive(0x0E17_5EED, fnv1a64(b"osc1")), 0x2931_f78f_4edb_0cc0);
        assert_eq!(
            derive(derive(0x0E17_5EED, fnv1a64(b"osc1")), 3),
            0x12dd_a249_22a4_ad74
        );
        assert_ne!(derive(1, 2), derive(2, 1));
        assert_ne!(derive(7, 0), derive(7, 1));
    }

    #[test]
    fn unit_values_are_in_range_and_exact() {
        let mut r = Rng::new(42);
        for _ in 0..10_000 {
            let u = r.unit_f32();
            assert!((0.0..1.0).contains(&u));
            assert_eq!((u * 16_777_216.0).fract(), 0.0, "24-bit grid");
            let d = r.unit_f64();
            assert!((0.0..1.0).contains(&d));
            let b = r.bipolar_f32();
            assert!((-1.0..1.0).contains(&b));
            let x = r.range_f32(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&x));
            assert!(r.below(7) < 7);
        }
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn copies_fork_the_same_stream() {
        let mut a = Rng::new(9);
        let _ = a.next_u64();
        let mut b = a;
        assert_eq!(a.next_u64(), b.next_u64());
        assert_eq!(a.next_u32(), b.next_u32());
    }
}
