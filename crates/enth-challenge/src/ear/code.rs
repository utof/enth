//! **Owner: lane CV3** (the ear-challenge code); lane RC owns the ear result (kind 4) in [`crate::result`].
//! Pre-declared by lane S0e (R4/R5, freeze point F0e). The reference is docs/challenge.md, "Version 15: the kinds
//! behind the escape" and "Challenge v3", and docs/duels.md, "Ear duels".
//!
//! The **ear-challenge code**: kind [`KIND_EAR_CHALLENGE`] (3) behind code v1's reserved version-15 escape, carrying
//! an [`EarSpec`], so two players hear the same items (an ear duel's fixed ladder: every item of a spec is at its
//! level, and item `i` depends only on `i`). Anyone holding the code can compute the answer keys, and the UI says so.
//!
//! | Field | Bits | Note |
//! | --- | --- | --- |
//! | form, version | 1 + 4 | form 0, version 15 ([`CODE_VERSION_ESCAPE`]): the first symbol is always `F` |
//! | kind | 5 | 3 |
//! | layout | 4 | 1 ([`EAR_CODE_LAYOUT`]); a newer one is `code_too_new` |
//! | game, generator, level | 8 + 8 + 8 | the spec's |
//! | seed | 40 | the spec's seed below 2⁴⁰, as a short code keeps it (a seed above has no code) |
//! | count | 8 | the spec's |
//! | padding | 4 | zero |
//! | code v1's check | 5 | code v1's GF(32) symbol, last |
//!
//! **19 symbols, at a fixed length**, written in groups of four as code v1 is (`F3G0-…-…-…-XYZ`). Code v1's rules
//! hold: Crockford base32, case ignored, `O` read as `0` and `I`/`L` as `1`, hyphens and spaces ignored, and the
//! weighted GF(32) check symbol catches every single substitution and every adjacent swap; the fixed length catches a
//! dropped or added symbol. An R1 to R3 build reads the code as `code_too_new` ("update enth"): it is at most 64
//! symbols, its check is code v1's, and its version is 15 (tested against [`ChallengeCode::decode`]).
//!
//! The GF(32) check and the alphabet are code v1's, restated here (about 15 lines) so `code.rs` stays untouched; a
//! test proves they agree with code v1's decoder. Lane RC makes code v1's helpers `pub(crate)` and puts a shared
//! version-15 frame in [`crate::result`]; F1E may move this kind onto it with no change of bytes.

use core::fmt;
use core::str::FromStr;

use crate::code::{CODE_VERSION_ESCAPE, CodeError};
use crate::model::{Challenge, ChallengeError};
pub use crate::result::KIND_EAR_CHALLENGE;
use crate::v3::{EarError, EarSpec};

/// The ear-challenge code's layout version this build writes and reads.
pub const EAR_CODE_LAYOUT: u8 = 1;

/// The symbols of an ear-challenge code, the check included.
pub const EAR_CODE_SYMBOLS: usize = 19;

/// The bits of a seed in the code (as a short challenge code's).
pub const EAR_CODE_SEED_BITS: u32 = 40;

/// Code v1's alphabet: Crockford base32.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// x⁵ + x² + 1, code v1's GF(32) modulus.
const GF32_MODULUS: u8 = 0b10_0101;

/// The payload's bits, before padding: form and version, kind, layout, game, generator, level, seed, count.
const PAYLOAD_BITS: usize = 5 + 5 + 4 + 8 + 8 + 8 + EAR_CODE_SEED_BITS as usize + 8;

/// Symbols per group in the canonical text.
const GROUP: usize = 4;

/// The most symbols read before giving up, as code v1.
const MAX_SYMBOLS: usize = 64;

/// An ear challenge as a code: an [`EarSpec`] whose seed fits 40 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EarChallengeCode {
    spec: EarSpec,
}

impl EarChallengeCode {
    /// The code of `spec`, when this build plays it ([`EarSpec::validate`]) and its seed is below 2⁴⁰; `None`
    /// otherwise (the shell keeps 40 bits of a seed, as for a short challenge code).
    pub fn new(spec: EarSpec) -> Option<EarChallengeCode> {
        (spec.seed >> EAR_CODE_SEED_BITS == 0 && spec.validate().is_ok())
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
        let s = &self.spec;
        let mut w = Bits::default();
        w.put(u64::from(CODE_VERSION_ESCAPE), 5); // form 0, version 15
        w.put(u64::from(KIND_EAR_CHALLENGE), 5);
        w.put(u64::from(EAR_CODE_LAYOUT), 4);
        w.put(u64::from(s.game), 8);
        w.put(u64::from(s.generator), 8);
        w.put(u64::from(s.level), 8);
        w.put(s.seed, EAR_CODE_SEED_BITS as usize);
        w.put(u64::from(s.count), 8);
        let mut data = w.symbols();
        data.push(check_value(&data));
        let mut out = String::with_capacity(data.len() + data.len() / GROUP);
        for (i, &v) in data.iter().enumerate() {
            if i > 0 && i % GROUP == 0 {
                out.push('-');
            }
            out.push(char::from(ALPHABET[usize::from(v)]));
        }
        out
    }

    /// Read a code. The checks, in order: every character a symbol (`bad_symbol`), 2 to 64 symbols (`bad_length`;
    /// `empty_code` for none), code v1's check (`bad_check`), version 15 and kind 3 (`not_ear_challenge` for any
    /// other code, so a paste box tries the next kind), a layout this build has (`code_too_new`; 0 is `bad_field`),
    /// form 0, the fixed length (`bad_length`), zero padding (`bad_field`), then the spec ([`EarSpec::validate`]:
    /// `code_too_new` for a newer game, generator or level). Never panics.
    pub fn decode(text: &str) -> Result<EarChallengeCode, EarCodeError> {
        let mut values: Vec<u8> = Vec::with_capacity(EAR_CODE_SYMBOLS);
        let mut count = 0usize;
        for (position, c) in text.chars().enumerate() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            count += 1;
            if count > MAX_SYMBOLS {
                continue;
            }
            let v = symbol_value(c).ok_or(EarCodeError::Code(CodeError::BadSymbol {
                symbol: c,
                position,
            }))?;
            values.push(v);
        }
        if count == 0 {
            return Err(EarCodeError::Code(CodeError::Empty));
        }
        if !(2..=MAX_SYMBOLS).contains(&count) {
            return Err(EarCodeError::Code(CodeError::BadLength { symbols: count }));
        }
        if at_alpha(&values) != 0 {
            return Err(EarCodeError::Code(CodeError::BadCheck));
        }
        let data = &values[..values.len() - 1];
        let mut r = Reader::new(data);
        let first = r.take(5);
        if first & 0b0_1111 != u64::from(CODE_VERSION_ESCAPE) {
            return Err(EarCodeError::NotEarChallenge { kind: None });
        }
        let kind = r.take(5);
        if kind == 0 {
            return Err(field("kind", 0));
        }
        if kind != u64::from(KIND_EAR_CHALLENGE) {
            return Err(EarCodeError::NotEarChallenge {
                kind: Some(kind as u8),
            });
        }
        let layout = r.take(4);
        if layout == 0 {
            return Err(field("layout", 0));
        }
        if layout > u64::from(EAR_CODE_LAYOUT) {
            return Err(EarCodeError::LayoutTooNew {
                layout: layout as u8,
            });
        }
        if first >> 4 != 0 {
            return Err(field("form", first >> 4));
        }
        if values.len() != EAR_CODE_SYMBOLS {
            return Err(EarCodeError::Code(CodeError::BadLength {
                symbols: values.len(),
            }));
        }
        let spec = EarSpec {
            game: r.take(8) as u8,
            generator: r.take(8) as u8,
            level: r.take(8) as u8,
            seed: r.take(EAR_CODE_SEED_BITS as usize),
            count: r.take(8) as u8,
        };
        let padding = r.take(data.len() * 5 - PAYLOAD_BITS);
        if padding != 0 {
            return Err(field("padding", padding));
        }
        spec.validate().map_err(EarCodeError::Spec)?;
        Ok(EarChallengeCode { spec })
    }
}

impl fmt::Display for EarChallengeCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl FromStr for EarChallengeCode {
    type Err = EarCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EarChallengeCode::decode(s)
    }
}

/// Why a text is not an ear-challenge code this build plays.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EarCodeError {
    /// Code v1's refusals, with their codes and hints: `empty_code`, `bad_symbol`, `bad_length`, `bad_check`,
    /// `bad_field`.
    Code(CodeError),
    /// A valid code of another kind: a code v1 challenge code (`kind` is `None`), or another kind behind version 15
    /// (`not_ear_challenge`).
    NotEarChallenge {
        /// The version-15 kind, if it is one.
        kind: Option<u8>,
    },
    /// A layout of kind 3 newer than this build's (`code_too_new`).
    LayoutTooNew {
        /// The layout.
        layout: u8,
    },
    /// The spec it carries is refused (`code_too_new` for a newer game, generator or level).
    Spec(EarError),
}

impl EarCodeError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            EarCodeError::Code(e) => e.code(),
            EarCodeError::NotEarChallenge { .. } => "not_ear_challenge",
            EarCodeError::LayoutTooNew { .. } => "code_too_new",
            EarCodeError::Spec(e) => e.code(),
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            EarCodeError::Code(CodeError::BadLength { .. }) => format!(
                "an ear challenge code has {EAR_CODE_SYMBOLS} symbols, hyphens not counted: part of it is missing or extra"
            ),
            EarCodeError::Code(e) => e.hint(),
            EarCodeError::NotEarChallenge { .. } => {
                "this is another kind of enth code, not an ear challenge".to_owned()
            }
            EarCodeError::LayoutTooNew { .. } => {
                "made by a newer enth: update enth to play it".to_owned()
            }
            EarCodeError::Spec(e) => e.hint(),
        }
    }
}

impl fmt::Display for EarCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EarCodeError::Code(e) => write!(f, "{e}"),
            EarCodeError::NotEarChallenge { kind: None } => f.write_str("not a version-15 code"),
            EarCodeError::NotEarChallenge { kind: Some(k) } => {
                write!(f, "a version-15 code of kind {k}, not an ear challenge")
            }
            EarCodeError::LayoutTooNew { layout } => {
                write!(
                    f,
                    "ear challenge layout {layout} is newer than this build's {EAR_CODE_LAYOUT}"
                )
            }
            EarCodeError::Spec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EarCodeError {}

fn field(field: &'static str, value: u64) -> EarCodeError {
    EarCodeError::Code(CodeError::BadField { field, value })
}

/// `v · α` in GF(32): a shift, reduced by x⁵ + x² + 1 (code v1's).
const fn times_alpha(v: u8) -> u8 {
    let w = v << 1;
    if w & 0b10_0000 != 0 {
        w ^ GF32_MODULUS
    } else {
        w
    }
}

/// The word's polynomial at α (Horner, first symbol highest), as code v1's.
fn at_alpha(symbols: &[u8]) -> u8 {
    symbols.iter().fold(0u8, |acc, &s| times_alpha(acc) ^ s)
}

/// Code v1's check symbol: α · (the data at α).
fn check_value(data: &[u8]) -> u8 {
    times_alpha(at_alpha(data))
}

/// A symbol's value after Crockford's case and alias rules, as code v1's.
fn symbol_value(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        'O' => Some(0),
        'I' | 'L' => Some(1),
        u => ALPHABET
            .iter()
            .position(|&b| char::from(b) == u)
            .map(|i| i as u8),
    }
}

/// Bits written most significant first, cut into 5-bit symbols, the last zero-padded.
#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn put(&mut self, value: u64, width: usize) {
        for i in (0..width).rev() {
            self.bits.push(i < 64 && value >> i & 1 == 1);
        }
    }

    fn symbols(&self) -> Vec<u8> {
        self.bits
            .chunks(5)
            .map(|c| {
                (0..5).fold(0u8, |acc, i| {
                    acc << 1 | u8::from(c.get(i).copied().unwrap_or(false))
                })
            })
            .collect()
    }
}

/// Reads bits most significant first from 5-bit symbols; past the end it reads zeros (the caller checks the length).
struct Reader<'a> {
    symbols: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(symbols: &'a [u8]) -> Self {
        Reader { symbols, at: 0 }
    }

    fn take(&mut self, width: usize) -> u64 {
        let mut v = 0u64;
        for _ in 0..width {
            let (s, b) = (self.at / 5, 4 - self.at % 5);
            let bit = self.symbols.get(s).map_or(0, |&x| (x >> b) & 1);
            v = v << 1 | u64::from(bit);
            self.at += 1;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChallengeCode;
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
            EarCodeError::NotEarChallenge { kind: None }
        );
        // The same layout with kind 1 (a scorecard's number) is another kind.
        let mut w = Bits::default();
        w.put(15, 5);
        w.put(1, 5);
        w.put(1, 4);
        w.put(0, 72);
        let mut data = w.symbols();
        data.push(check_value(&data));
        let text: String = data
            .iter()
            .map(|&v| char::from(ALPHABET[usize::from(v)]))
            .collect();
        assert_eq!(
            EarChallengeCode::decode(&text).unwrap_err(),
            EarCodeError::NotEarChallenge { kind: Some(1) }
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
        w.put(0, extra * 5);
        let mut data = w.symbols();
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
        assert_eq!(code(&with_fields(1, 3, 1, spec(42), 0, 0)), "bad_field");
        assert_eq!(code(&with_fields(0, 3, 1, spec(42), 5, 0)), "bad_field");
        assert_eq!(code(&with_fields(0, 3, 1, spec(42), 0, 1)), "bad_length");
        assert_eq!(
            code(&with_fields(0, 3, 1, EarSpec { game: 5, ..spec(1) }, 0, 0)),
            "code_too_new"
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
        assert_eq!(code(&"0".repeat(65)), "bad_length");
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
        assert_eq!(EarChallengeCode::new(EarSpec { game: 5, ..spec(1) }), None);
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
