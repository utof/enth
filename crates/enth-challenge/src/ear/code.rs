//! **Owner: lane CV3** (the ear-challenge code); lane RC owns the frame and the ear result (kind 4) in
//! [`crate::result`]. Pre-declared by lane S0e (R4/R5, freeze point F0e). The reference is docs/challenge.md,
//! "Version 15: the kinds behind the escape" and "Challenge v3", and docs/duels.md, "Ear duels".
//!
//! The **ear-challenge code**: kind [`KIND_EAR_CHALLENGE`] (3) behind code v1's reserved version-15 escape, carrying
//! an [`EarSpec`], so two players hear the same items (an ear duel's fixed ladder: every item of a spec is at its
//! level, and item `i` depends only on `i`). Anyone holding the code can compute the answer keys, and the UI says so.
//!
//! | Field | Bits | Note |
//! | --- | --- | --- |
//! | form, version | 1 + 4 | form 0, version 15: the first symbol is always `F` |
//! | kind | 5 | 3 |
//! | layout | 4 | 1 ([`EAR_CODE_LAYOUT`]); a newer one is `code_too_new` |
//! | game, generator, level | 8 + 8 + 8 | the spec's |
//! | seed | 40 | the spec's seed below 2⁴⁰, as a short code keeps it (a seed above has no code) |
//! | count | 8 | the spec's |
//! | padding | 4 | zero |
//! | code v1's check | 5 | code v1's GF(32) symbol, last |
//!
//! **19 symbols, at a fixed length**, written in groups of four as code v1 is (`F320-…-…-…-XYZ`). It is written and
//! read on [`crate::result`]'s version-15 frame (the F1e review's M3: `Bits`, `Reader`, `text`, `unframe`, with no
//! change of bytes), so it reads as every other kind does: Crockford base32, case ignored, `O` read as `0` and `I`/`L`
//! as `1`, hyphens and spaces ignored; code v1's weighted GF(32) symbol catches every single substitution and every
//! adjacent swap, and the fixed length a dropped or added symbol; another version-15 kind is `wrong_kind`, a code v1
//! challenge code `wrong_kind` too, an unknown kind, layout or the reserved form `code_too_new`. An R1 to R3 build
//! reads the code as `code_too_new` ("update enth"): it is at most 64 symbols, its check is code v1's, and its version
//! is 15 (tested against [`ChallengeCode::decode`](crate::ChallengeCode::decode)).

use core::fmt;
use core::str::FromStr;

use crate::model::{Challenge, ChallengeError};
pub use crate::result::KIND_EAR_CHALLENGE;
use crate::result::{
    Bits, HEADER_BITS, Reader, ResultError, put_ear_spec, take_ear_spec, text, unframe,
};
use crate::v3::EarSpec;

/// The ear-challenge code's layout version this build writes and reads.
pub const EAR_CODE_LAYOUT: u8 = 1;

/// The symbols of an ear-challenge code, the check included.
pub const EAR_CODE_SYMBOLS: usize = 19;

/// The bits of a seed in the code (as a short challenge code's).
pub const EAR_CODE_SEED_BITS: u32 = crate::result::EAR_SEED_BITS;

/// An ear challenge as a code: an [`EarSpec`] whose seed fits 40 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EarChallengeCode {
    spec: EarSpec,
}

impl EarChallengeCode {
    /// The code of `spec`, when this build plays it ([`crate::ear::validate`]) and its seed is below 2⁴⁰; `None`
    /// otherwise (the shell keeps 40 bits of a seed, as for a short challenge code).
    pub fn new(spec: EarSpec) -> Option<EarChallengeCode> {
        (spec.seed >> EAR_CODE_SEED_BITS == 0 && crate::ear::validate(&spec).is_ok())
            .then_some(EarChallengeCode { spec })
    }

    /// The spec.
    pub fn spec(&self) -> &EarSpec {
        &self.spec
    }

    /// The challenge it names ([`Challenge::ear`]).
    pub fn challenge(&self) -> Result<Challenge, ChallengeError> {
        Challenge::ear(self.spec)
    }

    /// The canonical text: upper case, groups of four joined by hyphens, code v1's check symbol last.
    pub fn encode(&self) -> String {
        let mut w = Bits::header(KIND_EAR_CHALLENGE, EAR_CODE_LAYOUT);
        put_ear_spec(&mut w, &self.spec);
        text(w.into_symbols())
    }

    /// Read a code. The checks, in order: the frame ([`crate::result`]'s: symbols, length bounds, code v1's symbol,
    /// version 15, the form, kind 3 (`wrong_kind` for another kind or a challenge code, `code_too_new` for an unknown
    /// one), a layout this build has (`code_too_new`; 0 is `bad_field`)), the fixed length (`bad_length`), zero padding
    /// (`bad_field`), then the spec ([`crate::ear::validate`]: `code_too_new` for a newer game, generator or level).
    /// Never panics.
    pub fn decode(text: &str) -> Result<EarChallengeCode, ResultError> {
        let f = unframe(text, KIND_EAR_CHALLENGE, EAR_CODE_LAYOUT)?;
        if f.data.len() + 1 != EAR_CODE_SYMBOLS {
            return Err(ResultError::BadLength {
                symbols: f.data.len() + 1,
            });
        }
        let mut r = Reader::at(&f.data, HEADER_BITS);
        let spec = take_ear_spec(&mut r)?;
        r.finish()?;
        crate::ear::validate(&spec).map_err(ResultError::Ear)?;
        Ok(EarChallengeCode { spec })
    }
}

impl fmt::Display for EarChallengeCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl FromStr for EarChallengeCode {
    type Err = ResultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EarChallengeCode::decode(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChallengeCode;
    use crate::code::{ALPHABET, check_value};
    use crate::ear::params::{EAR_CHECK, SPOT_THE_CHANGE};
    use proptest::prelude::*;

    fn spec(seed: u64) -> EarSpec {
        EarSpec {
            game: SPOT_THE_CHANGE,
            generator: 1,
            level: 1,
            seed,
            count: 16,
        }
    }

    /// The layout's bytes, pinned: these texts are promises to every player who holds an ear code.
    #[test]
    fn the_layout_is_pinned() {
        let cases = [
            (spec(0), "F320-8104-0000-0000-804"),
            (spec(42), "F320-8104-0000-0058-80M"),
            (spec((1 << 40) - 1), "F320-8107-ZZZZ-ZZZW-80S"),
            (
                EarSpec {
                    game: EAR_CHECK,
                    count: 40,
                    ..spec(123_456_789)
                },
                "F320-R104-00EP-YD2M-M0Y",
            ),
        ];
        for (s, text) in cases {
            let code = EarChallengeCode::new(s).unwrap();
            assert_eq!(code.encode(), text, "{s:?}");
            assert_eq!(EarChallengeCode::decode(text), Ok(code));
        }
    }

    #[test]
    fn a_code_is_19_symbols_starting_with_f() {
        let text = EarChallengeCode::new(spec(42)).unwrap().encode();
        let symbols: String = text.chars().filter(|&c| c != '-').collect();
        assert_eq!(symbols.len(), EAR_CODE_SYMBOLS);
        assert!(symbols.starts_with('F'));
        assert!(!text.contains("--"));
    }

    /// An R1 to R3 build's decoder (code v1, unchanged) reads every ear code as `code_too_new`: at most 64 symbols,
    /// its own check symbol, version 15. So it says "update enth", never "a typo".
    #[test]
    fn code_v1_reads_an_ear_code_as_too_new() {
        for seed in [0, 1, 42, (1 << 40) - 1] {
            let text = EarChallengeCode::new(spec(seed)).unwrap().encode();
            let e = ChallengeCode::decode(&text).unwrap_err();
            assert_eq!(e.code(), "code_too_new", "{text}: {e}");
        }
    }

    #[test]
    fn codes_of_other_kinds_are_told_apart() {
        let v1 = ChallengeCode::short(crate::SpecPreset::by_id(1).unwrap().spec(1, 42))
            .unwrap()
            .encode();
        assert_eq!(
            EarChallengeCode::decode(&v1).unwrap_err(),
            ResultError::WrongKind {
                found: None,
                expected: KIND_EAR_CHALLENGE
            }
        );
        // The same layout with kind 1 (a scorecard's number) is another kind: `wrong_kind`, as every kind answers.
        let mut w = Bits::default();
        w.put(15, 5);
        w.put(1, 5);
        w.put(1, 4);
        w.put(0, 64);
        w.put(0, 8);
        let mut data = w.into_symbols();
        data.push(check_value(&data));
        let text: String = data
            .iter()
            .map(|&v| char::from(ALPHABET[usize::from(v)]))
            .collect();
        assert_eq!(
            EarChallengeCode::decode(&text).unwrap_err(),
            ResultError::WrongKind {
                found: Some(1),
                expected: KIND_EAR_CHALLENGE
            }
        );
        assert_eq!(
            EarChallengeCode::decode(&text).unwrap_err().code(),
            "wrong_kind"
        );
    }

    fn with_fields(
        form: u64,
        kind: u64,
        layout: u64,
        s: EarSpec,
        pad: u64,
        extra: usize,
    ) -> String {
        let mut w = Bits::default();
        w.put(form << 4 | 15, 5);
        w.put(kind, 5);
        w.put(layout, 4);
        w.put(u64::from(s.game), 8);
        w.put(u64::from(s.generator), 8);
        w.put(u64::from(s.level), 8);
        w.put(s.seed, 40);
        w.put(u64::from(s.count), 8);
        w.put(pad, 4);
        w.put(0, (extra * 5) as u32);
        let mut data = w.into_symbols();
        data.push(check_value(&data));
        data.iter()
            .map(|&v| char::from(ALPHABET[usize::from(v)]))
            .collect()
    }

    #[test]
    fn refusals_have_their_codes() {
        let good = with_fields(0, 3, 1, spec(42), 0, 0);
        assert_eq!(
            EarChallengeCode::decode(&good),
            Ok(EarChallengeCode::new(spec(42)).unwrap())
        );
        let code = |t: &str| EarChallengeCode::decode(t).unwrap_err().code();
        assert_eq!(code(&with_fields(0, 3, 2, spec(42), 0, 0)), "code_too_new");
        assert_eq!(code(&with_fields(0, 3, 0, spec(42), 0, 0)), "bad_field");
        assert_eq!(code(&with_fields(0, 0, 1, spec(42), 0, 0)), "bad_field");
        // The reserved form reads as every kind reads it (the frame's rule): a newer build's.
        assert_eq!(code(&with_fields(1, 3, 1, spec(42), 0, 0)), "code_too_new");
        assert_eq!(code(&with_fields(0, 3, 1, spec(42), 5, 0)), "bad_field");
        assert_eq!(code(&with_fields(0, 3, 1, spec(42), 0, 1)), "bad_length");
        // An unknown kind behind version 15 is a newer build's.
        assert_eq!(code(&with_fields(0, 9, 1, spec(42), 0, 0)), "code_too_new");
        assert_eq!(
            code(&with_fields(
                0,
                3,
                1,
                EarSpec {
                    game: 11,
                    ..spec(1)
                },
                0,
                0
            )),
            "code_too_new"
        );
        // A music game reads (the F1e review's M5).
        let interval = EarSpec { game: 5, ..spec(1) };
        assert_eq!(
            EarChallengeCode::decode(&with_fields(0, 3, 1, interval, 0, 0)),
            Ok(EarChallengeCode::new(interval).unwrap())
        );
        assert_eq!(
            code(&with_fields(
                0,
                3,
                1,
                EarSpec {
                    level: 2,
                    ..spec(1)
                },
                0,
                0
            )),
            "code_too_new"
        );
        assert_eq!(
            code(&with_fields(
                0,
                3,
                1,
                EarSpec {
                    count: 0,
                    ..spec(1)
                },
                0,
                0
            )),
            "bad_field"
        );
        assert_eq!(code(""), "empty_code");
        assert_eq!(code("--- "), "empty_code");
        assert_eq!(code("F"), "bad_length");
        assert_eq!(code("FU00"), "bad_symbol");
        // Zeros pass code v1's symbol and are no version-15 code; past the frame's bound is `bad_length`.
        assert_eq!(code(&"0".repeat(65)), "wrong_kind");
        assert_eq!(
            code(&"0".repeat(crate::result::MAX_SYMBOLS + 1)),
            "bad_length"
        );
        // A code's text is forgiving as code v1's: case, aliases, spaces.
        let text = EarChallengeCode::new(spec(7)).unwrap().encode();
        let loose = text
            .to_lowercase()
            .replace('-', " ")
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(EarChallengeCode::decode(&loose).unwrap().spec(), &spec(7));
        // A seed of 2^40 or more, and a spec this build cannot play, have no code.
        assert_eq!(EarChallengeCode::new(spec(1 << 40)), None);
        assert_eq!(
            EarChallengeCode::new(EarSpec {
                game: 11,
                ..spec(1)
            }),
            None
        );
    }

    /// The F1e review's M5: an ear-challenge code of every music game (4 to 10) round-trips and names its challenge.
    #[test]
    fn music_games_have_codes_that_round_trip() {
        for game in enth_music::ear::Game::ALL {
            let s = EarSpec::new(game, 1, (1 << 40) - 1, 20);
            let code = EarChallengeCode::new(s).unwrap_or_else(|| panic!("{game:?}"));
            let text = code.encode();
            assert_eq!(text.chars().filter(|&c| c != '-').count(), EAR_CODE_SYMBOLS);
            assert_eq!(EarChallengeCode::decode(&text), Ok(code));
            assert_eq!(code.challenge().unwrap().ear_spec(), Some(&s));
            assert_eq!(
                ChallengeCode::decode(&text).unwrap_err().code(),
                "code_too_new"
            );
        }
    }

    #[test]
    fn the_challenge_is_the_ear_challenge_of_its_spec() {
        let code = EarChallengeCode::new(spec(42)).unwrap();
        assert_eq!(code.challenge(), Challenge::ear(spec(42)));
    }

    proptest! {
        /// Round trip, and every single substitution and adjacent swap is caught.
        #[test]
        fn round_trips_and_catches_typos(seed in 0u64..(1 << 40), count in 1u8..=64) {
            let s = EarSpec { count, ..spec(seed) };
            let code = EarChallengeCode::new(s).unwrap();
            let text = code.encode();
            prop_assert_eq!(EarChallengeCode::decode(&text), Ok(code));
            let symbols: Vec<char> = text.chars().filter(|&c| c != '-').collect();
            for i in 0..symbols.len() {
                for &b in ALPHABET {
                    let c = char::from(b);
                    if c == symbols[i] {
                        continue;
                    }
                    let mut t = symbols.clone();
                    t[i] = c;
                    let t: String = t.into_iter().collect();
                    prop_assert!(EarChallengeCode::decode(&t).is_err(), "substitution at {} caught", i);
                }
                if i + 1 < symbols.len() && symbols[i] != symbols[i + 1] {
                    let mut t = symbols.clone();
                    t.swap(i, i + 1);
                    let t: String = t.into_iter().collect();
                    prop_assert!(EarChallengeCode::decode(&t).is_err(), "swap at {} caught", i);
                }
            }
        }

        /// Decoding never panics.
        #[test]
        fn decoding_is_total(text in ".{0,80}") {
            let _ = EarChallengeCode::decode(&text);
        }
    }
}
