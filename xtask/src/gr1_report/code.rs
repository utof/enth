//! Reading a challenge code only far enough to know its **level**, which the round log does not carry.
//!
//! This is a small, tolerant copy of code v1's layout (docs/challenge.md, "Codes v1"; `enth-challenge`'s `code.rs`),
//! kept here so that `xtask` stays light: it needs no render or analysis crate. Code v1 is frozen ("a promise to every
//! player who has a code"), and a test reads the frozen fixture `code/v1/codes.json` to prove this copy still agrees.
//!
//! - A **short** code (14 symbols) carries a preset id; the presets 1 to 5 are levels 1 to 5.
//! - A **long** code (35 to 44 symbols) carries the whole spec: its 7-bit difficulty picks the level
//!   (`enth_challenge::random::level`: 0 to 20 is level 1, 21 to 40 level 2, and so on); a nonzero amounts mask
//!   marks it as custom.
//! - A newer code version, a newer preset, a bad symbol or a short text give no level, and the round counts under
//!   "level ?" instead of failing the report. The check symbol is verified and reported, never required.

/// Crockford's alphabet: the value of a symbol is its position.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// GF(32)'s modulus, x^5 + x^2 + 1.
const GF32_MODULUS: u8 = 0b10_0101;

/// The newest code version this reader knows.
const CODE_VERSION: u8 = 1;

/// What a code says about its round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeInfo {
    /// The level, 1 to 5; `None` for a code this reader cannot place.
    pub level: Option<u8>,
    /// A long code (it carries the whole spec).
    pub long: bool,
    /// A long code with amounts set by hand (not one of the five level presets).
    pub custom: bool,
    /// The check symbol matched. A code that fails it was probably mistyped, but it is still read.
    pub check_ok: bool,
}

/// The canonical comparison form of a code's text: upper case, Crockford's aliases applied (`O` is `0`, `I` and `L`
/// are `1`), anything that is not a letter or digit (hyphens, spaces) dropped.
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            u => u,
        })
        .collect()
}

/// The level a difficulty picks (`enth_challenge::random::level`).
pub fn level_of_difficulty(difficulty: u8) -> u8 {
    let d = difficulty.min(100);
    1 + (d.saturating_sub(1) / 20).min(4)
}

/// The symbols (0 to 31) of a code's text, `None` if any character is not a code symbol.
fn symbols(normalized: &str) -> Option<Vec<u8>> {
    normalized
        .bytes()
        .map(|b| ALPHABET.iter().position(|&a| a == b).map(|p| p as u8))
        .collect()
}

/// `v` times alpha in GF(32).
const fn times_alpha(v: u8) -> u8 {
    let w = v << 1;
    if w & 0b10_0000 != 0 {
        w ^ GF32_MODULUS
    } else {
        w
    }
}

/// The word's polynomial at alpha (Horner, first symbol highest).
fn at_alpha(symbols: &[u8]) -> u8 {
    symbols.iter().fold(0u8, |acc, &s| times_alpha(acc) ^ s)
}

/// `width` bits from bit offset `at`, most significant first (symbols hold 5 bits each).
fn bits(symbols: &[u8], at: usize, width: usize) -> u64 {
    let mut out = 0u64;
    for i in at..at + width {
        let bit = (symbols[i / 5] >> (4 - i % 5)) & 1;
        out = out << 1 | u64::from(bit);
    }
    out
}

/// Read a code's text. `None` when it is not a code this reader can place (empty, a bad symbol, an unknown length,
/// a newer version).
pub fn decode(text: &str) -> Option<CodeInfo> {
    let syms = symbols(&normalize(text))?;
    let first = *syms.first()?;
    let (long_form, version) = (first >> 4 == 1, first & 0x0F);
    if version == 0 || version > CODE_VERSION {
        return None;
    }
    // The word followed by its check symbol vanishes at alpha.
    let check_ok = at_alpha(&syms) == 0;
    if long_form {
        // 35 to 44 symbols, the check symbol included; the fixed part is 163 bits, then the 6-bit amounts mask.
        if !(35..=44).contains(&syms.len()) {
            return None;
        }
        let difficulty = bits(&syms, 85, 7) as u8;
        let mask = bits(&syms, 163, 6);
        Some(CodeInfo {
            level: Some(level_of_difficulty(difficulty)),
            long: true,
            custom: mask != 0,
            check_ok,
        })
    } else {
        if syms.len() != 14 {
            return None;
        }
        let preset = bits(&syms, 5, 8);
        Some(CodeInfo {
            level: (1..=5).contains(&preset).then_some(preset as u8),
            long: false,
            custom: false,
            check_ok,
        })
    }
}

/// A short code's canonical text (`1K3M-QX9P-2HDT-VC`). For tests and the fixture generator: the round log never
/// needs to write a code.
#[cfg(test)]
pub fn encode_short(preset: u8, generator: u16, seed: u64) -> String {
    let mut bits_out: Vec<u8> = Vec::new();
    let mut put = |value: u64, width: u32| {
        for i in (0..width).rev() {
            bits_out.push(((value >> i) & 1) as u8);
        }
    };
    put(0, 1);
    put(u64::from(CODE_VERSION), 4);
    put(u64::from(preset), 8);
    put(u64::from(generator), 12);
    put(seed, 40);
    let mut data: Vec<u8> = bits_out
        .chunks(5)
        .map(|c| {
            let mut v = 0u8;
            for (i, b) in c.iter().enumerate() {
                v |= b << (4 - i);
            }
            v
        })
        .collect();
    let check = times_alpha(at_alpha(&data));
    data.push(check);
    let mut out = String::new();
    for (i, &v) in data.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(char::from(ALPHABET[usize::from(v)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The frozen codes of `enth-challenge` (their SHA-256 is pinned there): every one must read as its spec says.
    #[test]
    fn this_copy_agrees_with_the_frozen_code_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/enth-challenge/tests/fixtures/code/v1/codes.json"
        );
        let text = std::fs::read_to_string(path).expect("the frozen code fixture");
        let cases: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert!(cases.len() >= 10, "{} cases", cases.len());
        let (mut shorts, mut longs) = (0, 0);
        for case in &cases {
            let difficulty = case["spec"]["difficulty"].as_u64().unwrap() as u8;
            let amounts_set = case["spec"]["amounts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| !a.is_null());
            let name = case["name"].as_str().unwrap();
            for form in ["short", "long"] {
                let Some(text) = case[form].as_str() else {
                    continue;
                };
                let info = decode(text).unwrap_or_else(|| panic!("{name} {form} {text} reads"));
                assert!(info.check_ok, "{name} {form} {text}: check symbol");
                assert_eq!(info.long, form == "long", "{name}");
                if form == "short" {
                    shorts += 1;
                    // A short code is exactly a preset: level n is difficulty 0, 25, 50, 75, 100.
                    assert_eq!(
                        info.level,
                        Some(level_of_difficulty(difficulty)),
                        "{name} {text}"
                    );
                } else {
                    longs += 1;
                    assert_eq!(
                        info.level,
                        Some(level_of_difficulty(difficulty)),
                        "{name} {text}"
                    );
                    assert_eq!(info.custom, amounts_set, "{name} {text}");
                }
            }
        }
        assert!(
            shorts >= 5 && longs >= 5,
            "{shorts} short and {longs} long codes read"
        );
    }

    #[test]
    fn the_encoder_reproduces_the_frozen_short_codes() {
        // level2-seed42 in codes.json: preset 2, generator 1, seed 42, short text 1080-1000-0001-AB.
        assert_eq!(encode_short(2, 1, 42), "1080-1000-0001-AB");
        assert_eq!(encode_short(1, 1, 0), "1040-1000-0000-0T");
        assert_eq!(encode_short(3, 1, 0xFF_FFFF_FFFF), "10C0-1ZZZ-ZZZZ-Z4");
    }

    #[test]
    fn short_codes_give_their_preset_as_the_level() {
        for level in 1..=5u8 {
            let text = encode_short(level, 1, 12_345 + u64::from(level));
            let info = decode(&text).unwrap();
            assert_eq!(info.level, Some(level));
            assert!(info.check_ok && !info.long && !info.custom);
        }
        // A preset this reader does not know is a code it cannot place, not an error.
        let info = decode(&encode_short(9, 1, 1)).unwrap();
        assert_eq!(info.level, None);
    }

    #[test]
    fn case_hyphens_spaces_and_crockford_aliases_are_ignored() {
        let text = encode_short(4, 1, 777);
        let messy = text
            .to_lowercase()
            .replace('-', "  ")
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(normalize(&messy), normalize(&text));
        assert_eq!(decode(&messy), decode(&text));
    }

    #[test]
    fn a_mistyped_code_still_gives_a_level_and_says_its_check_failed() {
        // Swap two adjacent symbols of a valid code: the check catches it, the preset is still readable.
        let text = normalize(&encode_short(3, 1, 5000));
        let mut chars: Vec<char> = text.chars().collect();
        chars.swap(10, 11);
        let swapped: String = chars.into_iter().collect();
        let info = decode(&swapped).unwrap();
        assert!(!info.check_ok);
        assert_eq!(info.level, Some(3));
    }

    #[test]
    fn junk_and_newer_codes_give_nothing() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("not a code!"), None);
        assert_eq!(decode("1080-1000"), None, "a short text is not a code");
        // Version 2 in the first symbol (form 0, version 2): a layout this reader does not know.
        assert_eq!(decode("2080-1000-0001-AB"), None);
        // The reserved escape, version 15.
        assert_eq!(decode("F080-1000-0001-AB"), None);
    }

    #[test]
    fn levels_follow_the_difficulty_bands() {
        let bands = [
            (0, 1),
            (20, 1),
            (21, 2),
            (40, 2),
            (41, 3),
            (60, 3),
            (61, 4),
            (80, 4),
            (81, 5),
            (100, 5),
        ];
        for (d, level) in bands {
            assert_eq!(level_of_difficulty(d), level, "difficulty {d}");
        }
        // The presets sit at 0, 25, 50, 75 and 100.
        let presets = [0, 25, 50, 75, 100].map(level_of_difficulty);
        assert_eq!(presets, [1, 2, 3, 4, 5]);
    }
}
