//! The delay-length draws that upstream takes from `std::mt19937`, as a constant table.
//!
//! Signalsmith's `MultiDelay::updateLengths(seed, ...)` is only ever called with five constant seeds, so the 8 + 8
//! random numbers it draws per seed do not depend on the room size or on anything else: they are constants of the
//! algorithm. Per seed and per channel `i` (0..8) it takes one `std::uniform_real_distribution<float>(0, 1)` draw
//! (`unit`) and one `std::uniform_int_distribution<int>(0, i)` draw (the swap index), interleaved. The table holds
//! those 40 pairs, as recorded from libstdc++ (GCC 13.3; `tables.cpp` in the FX1 scratch harness prints them), with
//! the floats as raw bits so nothing depends on a decimal round trip.
//!
//! Why a table: the distributions are not portable. libstdc++, libc++ and MSVC turn the same `mt19937` stream into
//! different numbers (P1a report 02, citing WG21 P2059R0), so upstream itself sounds slightly different per
//! platform. A table makes the port identical everywhere and needs no RNG at all (AGENTS.md: seeds never depend on
//! parameter values). The unit test `table_matches_the_regenerated_draws` rebuilds it from a from-scratch
//! `mt19937`, libstdc++'s `generate_canonical<float, 24>` and its Lemire `uniform_int_distribution`.
//!
//! Derived from Signalsmith Basics `reverb.h` (MIT), see `THIRD_PARTY_NOTICES.md`.

/// The five constant seeds of `updateDelays` in upstream order: delays 1 to 4, then the early reflections.
/// (The feedback delay is exponential and uses no seed.)
#[cfg(test)]
pub(super) const SEEDS: [u32; 5] = [
    0x6DD0_9EE5,
    0x8767_53A5,
    0x5974_DF44,
    0x8CDB_F7E6,
    0x0BDD_E171,
];

/// Index of each seed's table in [`DRAWS`].
pub(super) const DELAY1: usize = 0;
pub(super) const DELAY2: usize = 1;
pub(super) const DELAY3: usize = 2;
pub(super) const DELAY4: usize = 3;
pub(super) const EARLY: usize = 4;

/// One channel's draw: the raw bits of the `f32` unit value in `[0, 1)` and the swap index in `0..=i`.
pub(super) type Draw = (u32, u8);

/// `DRAWS[seed][i]` is the `i`-th (unit, swap index) pair for that seed.
pub(super) const DRAWS: [[Draw; 8]; 5] = [
    // 0x6DD09EE5
    [
        (0x3e28_8d78, 0),
        (0x3f4b_a860, 0),
        (0x3d6c_2312, 1),
        (0x3f7e_4d4e, 2),
        (0x3f56_aecc, 1),
        (0x3f78_f69e, 3),
        (0x3f59_631e, 5),
        (0x3f09_0b3c, 3),
    ],
    // 0x876753A5
    [
        (0x3f4d_ece1, 0),
        (0x3f53_1605, 1),
        (0x3f6f_47ee, 1),
        (0x3f5f_6bbd, 2),
        (0x3efe_59de, 4),
        (0x3f73_66e6, 0),
        (0x3f11_08b4, 0),
        (0x3f43_b33e, 4),
    ],
    // 0x5974DF44
    [
        (0x3f3f_e472, 0),
        (0x3d76_ba3a, 0),
        (0x3e7d_b128, 0),
        (0x3eb4_08e2, 3),
        (0x3f53_4f79, 3),
        (0x3eb5_13a1, 3),
        (0x3f37_1cdf, 1),
        (0x3e67_2f46, 6),
    ],
    // 0x8CDBF7E6
    [
        (0x3f67_859e, 0),
        (0x3ee7_2d96, 1),
        (0x3ef5_7185, 2),
        (0x3ec5_2526, 2),
        (0x3ebc_6bff, 2),
        (0x3eeb_1be1, 1),
        (0x3f78_1d9d, 2),
        (0x3f7e_fec5, 3),
    ],
    // 0x0BDDE171 (early reflections)
    [
        (0x3f48_cc19, 0),
        (0x3f3b_403d, 1),
        (0x3e62_2c34, 0),
        (0x3e0b_fd17, 1),
        (0x3ebf_e252, 0),
        (0x3f24_7d53, 5),
        (0x3f15_212a, 0),
        (0x3f59_ffaa, 1),
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    /// `std::mt19937`, written from the published algorithm (Matsumoto and Nishimura 1998), to regenerate the table.
    struct Mt19937 {
        mt: [u32; 624],
        idx: usize,
    }

    impl Mt19937 {
        fn new(seed: u32) -> Self {
            let mut mt = [0u32; 624];
            mt[0] = seed;
            for i in 1..624 {
                mt[i] = 1_812_433_253u32
                    .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                    .wrapping_add(i as u32);
            }
            Self { mt, idx: 624 }
        }

        fn next_u32(&mut self) -> u32 {
            if self.idx >= 624 {
                for i in 0..624 {
                    let y = (self.mt[i] & 0x8000_0000) | (self.mt[(i + 1) % 624] & 0x7fff_ffff);
                    let mut v = self.mt[(i + 397) % 624] ^ (y >> 1);
                    if y & 1 != 0 {
                        v ^= 0x9908_b0df;
                    }
                    self.mt[i] = v;
                }
                self.idx = 0;
            }
            let mut y = self.mt[self.idx];
            self.idx += 1;
            y ^= y >> 11;
            y ^= (y << 7) & 0x9d2c_5680;
            y ^= (y << 15) & 0xefc6_0000;
            y ^= y >> 18;
            y
        }

        /// libstdc++'s `generate_canonical<float, 24>` over a 32-bit engine: one draw, `float(u) / 2^32`, and
        /// the result clamped below 1.
        fn canonical_f32(&mut self) -> f32 {
            let r = self.next_u32() as f32 / (1u64 << 32) as f32;
            if r >= 1.0 {
                f32::from_bits(0x3f7f_ffff)
            } else {
                r
            }
        }

        /// libstdc++'s (GCC 11 and later) `uniform_int_distribution<int>(0, hi)`: Lemire's nearly-divisionless
        /// method over the 32-bit engine range.
        fn uniform_int(&mut self, hi: u32) -> u32 {
            let range = hi + 1;
            let mut product = u64::from(self.next_u32()) * u64::from(range);
            let mut low = product as u32;
            if low < range {
                let threshold = range.wrapping_neg() % range;
                while low < threshold {
                    product = u64::from(self.next_u32()) * u64::from(range);
                    low = product as u32;
                }
            }
            (product >> 32) as u32
        }
    }

    #[test]
    fn mt19937_known_answers() {
        // The reference values of the standard (default seed 5489): the first output, and the 10000th (the one
        // the C++ standard itself requires of `std::mt19937`).
        let mut e = Mt19937::new(5489);
        assert_eq!(e.next_u32(), 3_499_211_612);
        let mut e = Mt19937::new(5489);
        let tenthousandth = (0..10_000).map(|_| e.next_u32()).last().unwrap();
        assert_eq!(tenthousandth, 4_123_659_995);
    }

    #[test]
    fn table_matches_the_regenerated_draws() {
        for (s, seed) in SEEDS.iter().enumerate() {
            let mut engine = Mt19937::new(*seed);
            for i in 0..8u8 {
                let unit = engine.canonical_f32();
                let swap = engine.uniform_int(u32::from(i));
                assert_eq!(
                    DRAWS[s][usize::from(i)],
                    (unit.to_bits(), swap as u8),
                    "seed {seed:#010x}, draw {i}"
                );
            }
        }
    }

    #[test]
    fn draws_are_valid() {
        for table in &DRAWS {
            for (i, &(bits, swap)) in table.iter().enumerate() {
                let unit = f32::from_bits(bits);
                assert!((0.0..1.0).contains(&unit));
                assert!(usize::from(swap) <= i);
            }
        }
    }
}
