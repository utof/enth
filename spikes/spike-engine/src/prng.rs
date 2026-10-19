//! Seeded integer PRNG (`SplitMix64`). All randomness in the engine and the patch generator comes from here:
//! no system time, no global RNG. Only integer operations and one exact int-to-float conversion, so the
//! stream is identical on every platform.

/// Derive an independent stream seed, e.g. `derive(seed, voice_index)` or `derive(patch_hash, module_id)`.
#[must_use]
pub fn derive(seed: u64, stream: u64) -> u64 {
    let mut r = Rng::new(seed ^ stream.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    r.next_u64()
}

/// `SplitMix64`.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1) with 24 random bits (exactly representable in f32).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Uniform in [lo, hi): `lo + (hi - lo) * u`, plain IEEE operations.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Uniform integer in 0..n (n > 0; modulo bias is irrelevant here).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
