//! **Owner: lane RC** (Result codes, freeze point F1e; the F1e judge freezes the formula). Pre-declared by lane S0e
//! (R4/R5, freeze point F0e). The reference is docs/challenge.md, "The daily challenge formula", and docs/duels.md,
//! "The daily challenge".
//!
//! **`daily::v1`**: one challenge per level per day with no server and no organiser. Integer arithmetic on `enth-dsp`'s
//! pinned `rng`, the same on every platform:
//!
//! ```text
//! day   = whole days since 1970-01-01 in UTC        (the shell supplies it: a core crate reads no clock)
//! seed  = derive(derive(fnv1a64(b"enth-daily-v1"), day), level) >> 24      the top 40 bits
//! code  = the short code of preset `level` (1 to 5) with generator 1 and that seed
//! ```
//!
//! - **The generator is part of the formula:** daily v1 always plays generator 1, so a later generator never changes
//!   an old day's challenge; a daily on a newer generator is a new formula (`enth-daily-v2`).
//! - The top 40 bits keep the code short, and the level enters the seed, so the five dailies of a day are unrelated.
//! - **The day turns at 00:00 UTC** ([`utc_day`]), which is local midnight only in UTC, as Review's day does. The share
//!   line carries the code itself, so a friend in another time zone pastes it rather than relying on "today".
//! - A golden of (day, level) → code pins it (the tests below); after F1e it never changes.
//! - A daily's target can be silent through its phrase (generator v1 is frozen, so the formula cannot redraw): about
//!   one seed in a few hundred. The shell then says so and offers the next level's daily; the formula never skips.

use enth_dsp::util::rng::{derive, fnv1a64};

use crate::code::ChallengeCode;
use crate::random::GENERATOR_V1;
use crate::spec::SpecPreset;

/// The formula's name, hashed into its key: a new formula gets a new name.
pub const V1_KEY: &[u8] = b"enth-daily-v1";

/// The generator daily v1 always plays.
pub const V1_GENERATOR: u16 = GENERATOR_V1;

/// The levels a daily exists for: the preset ids 1 to 5.
pub const LEVELS: core::ops::RangeInclusive<u8> = 1..=5;

/// Seconds in a UTC day (no leap seconds: Unix time has none).
const DAY_SECONDS: i64 = 86_400;

/// Daily v1's seed for `day` (days since 1970-01-01, UTC) and `level`: the top 40 bits of
/// `derive(derive(fnv1a64(b"enth-daily-v1"), day), level)`.
pub const fn v1_seed(day: u32, level: u8) -> u64 {
    derive(derive(fnv1a64(V1_KEY), day as u64), level as u64) >> 24
}

/// Daily v1's challenge for `day` and `level` (1 to 5): the short code of preset `level` with generator 1 and
/// [`v1_seed`]. `None` for another level.
pub fn v1(day: u32, level: u8) -> Option<ChallengeCode> {
    if !LEVELS.contains(&level) {
        return None;
    }
    let preset = SpecPreset::by_id(level)?;
    ChallengeCode::short(preset.spec(V1_GENERATOR, v1_seed(day, level)))
}

/// The UTC day of a Unix time in seconds: whole days since 1970-01-01T00:00:00Z, turning at 00:00 UTC. `None` before
/// 1970 or past day `u32::MAX`. The shell reads its clock and passes the seconds in.
pub fn utc_day(unix_seconds: i64) -> Option<u32> {
    if unix_seconds < 0 {
        return None;
    }
    u32::try_from(unix_seconds / DAY_SECONDS).ok()
}

#[cfg(test)]
mod tests {
    use core::fmt::Write as _;

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::code::SHORT_SEED_BITS;

    /// The golden: (day, level) → code, computed by an independent Python implementation of the formula in
    /// docs/challenge.md (lane RC's scratch `daily.py`). **Frozen at F1e**: a changed line means the formula changed,
    /// which it never may (a new formula is `enth-daily-v2`, beside this one).
    const GOLDEN: [(u32, u8, &str); 12] = [
        (0, 1, "1040-134J-Q0SA-VV"),
        (0, 5, "10M0-16PB-MATQ-0X"),
        (1, 1, "1040-1T47-WJ14-QR"),
        (20_748, 1, "1040-119J-CFVS-2C"),
        (20_748, 2, "1080-13VJ-1R39-03"),
        (20_748, 3, "10C0-1FYS-GTCM-85"),
        (20_748, 4, "10G0-10RV-HTPF-TG"),
        (20_748, 5, "10M0-1GY2-2FXN-4T"),
        (20_749, 3, "10C0-1Z4S-KZA1-C6"),
        (21_915, 3, "10C0-1D30-9CF1-P9"),
        (100_000, 2, "1080-1A61-9Y4S-ZG"),
        (u32::MAX, 5, "10M0-1SZJ-EW9A-QN"),
    ];

    /// The SHA-256 of every daily from day 20,000 to 21,999 (levels 1 to 5), one canonical code per line.
    const SWEEP_SHA256: &str = "fd5accb0e6a629ea330803139eaf4e0e2f41ddc821c0ea22e68254c1734588dc";

    #[test]
    fn the_daily_formula_matches_its_golden() {
        for (day, level, want) in GOLDEN {
            assert_eq!(
                v1(day, level).unwrap().encode(),
                want,
                "day {day}, level {level}"
            );
        }
    }

    #[test]
    fn the_sweep_matches_its_pinned_hash() {
        let mut text = String::new();
        for day in 20_000..22_000 {
            for level in LEVELS {
                text.push_str(&v1(day, level).unwrap().encode());
                text.push('\n');
            }
        }
        let hex = Sha256::digest(text.as_bytes())
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
        assert_eq!(hex, SWEEP_SHA256);
    }

    #[test]
    fn a_daily_is_a_short_code_of_its_level_on_generator_1() {
        for day in [0, 20_748, u32::MAX] {
            for level in LEVELS {
                let code = v1(day, level).unwrap();
                let spec = code.spec();
                assert_eq!(spec.generator, 1);
                assert_eq!(spec.preset().unwrap().id, level);
                assert!(spec.seed < 1 << SHORT_SEED_BITS);
                assert_eq!(ChallengeCode::decode(&code.encode()), Ok(code));
            }
        }
        assert_eq!(v1(20_748, 0), None);
        assert_eq!(v1(20_748, 6), None);
    }

    #[test]
    fn the_five_dailies_of_a_day_and_neighbouring_days_differ() {
        let mut seeds: Vec<u64> = (20_740..20_760)
            .flat_map(|d| LEVELS.map(move |l| v1_seed(d, l)))
            .collect();
        let n = seeds.len();
        seeds.sort_unstable();
        seeds.dedup();
        assert_eq!(seeds.len(), n);
    }

    /// The open check "the UTC day boundary": the day turns at 00:00 UTC exactly.
    #[test]
    fn the_day_turns_at_midnight_utc() {
        assert_eq!(utc_day(0), Some(0));
        assert_eq!(utc_day(86_399), Some(0));
        assert_eq!(utc_day(86_400), Some(1));
        // 2026-10-22T00:00:00Z and the second before it.
        assert_eq!(utc_day(1_792_627_200), Some(20_748));
        assert_eq!(utc_day(1_792_627_199), Some(20_747));
        assert_eq!(utc_day(-1), None);
        assert_eq!(utc_day(i64::from(u32::MAX) * 86_400), Some(u32::MAX));
        assert_eq!(utc_day((i64::from(u32::MAX) + 1) * 86_400), None);
    }
}
