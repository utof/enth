//! **Owner: lane CT** (code v1, freeze point F1r). The reference is docs/challenge.md, "Codes v1".
//!
//! A challenge code is a [`RandomSpec`] in a form a person can read out, type and paste (report 01 §2.5, design.md
//! §9): a versioned bit string in Crockford base32 with a weighted check symbol over GF(32), itself one of the 32
//! data symbols. Decoding regenerates nothing; the caller regenerates the target from [`ChallengeCode::spec`] and
//! calls [`ChallengeCode::verify`] with its [`TargetFingerprint`].
//!
//! # The two forms
//!
//! | Form | Symbols | Carries | Starts with |
//! | --- | --- | --- | --- |
//! | **short** | 13 data + 1 check = 14 | a [`SpecPreset`] id, the generator (< 4096), the seed (< 2⁴⁰) | `1` |
//! | **long** | 34–43 data + 1 check = 35–44 | the whole spec, the engine version, a 32-bit target-fingerprint prefix | `H` |
//!
//! Bits are written most significant first, then cut into 5-bit symbols; the last symbol is zero-padded.
//!
//! - Short (65 bits): `form 1 = 0 | version 4 | preset 8 | generator 12 | seed 40`.
//! - Long (169 + 7k bits, k = the number of set amounts): `form 1 = 1 | version 4 | generator 16 | seed 64 |
//!   difficulty 7 | musical 7 | template 8 | engine major 8 | minor 8 | patch 8 | target prefix 32 | amounts mask 6
//!   (Category::ALL order, first category in the high bit) | 7 bits per set amount, in that order`.
//!
//! The first symbol is `form << 4 | version`, so a short v1 code starts with `1` and a long one with `H`, and every
//! later version can be told apart before its layout is known. Version 15 is reserved as an escape
//! ([`CODE_VERSION_ESCAPE`]): a later kind of code (a result code, a duel) starts with it and puts a kind field next,
//! so the 1-bit form never runs out. Versions 2–14 are for new layouts of these two forms.
//!
//! # Reading a code
//!
//! Case is ignored, `O` reads as `0` and `I`/`L` as `1` (Crockford), and hyphens and spaces are ignored anywhere.
//! Every symbol, the check included, is one of Crockford's 32 (`0`–`9`, `A`–`Z` without `I L O U`). The check makes
//! the whole word, read as a polynomial over GF(32) with the check as its constant term, vanish at α (a root of
//! x⁵ + x² + 1): it catches every single substitution and every adjacent swap, at any length (docs/challenge.md has
//! the argument), and the fixed short length catches a dropped or added symbol. The order of checks: symbols, length
//! bounds, the check symbol, the version (`code_too_new` for a newer one), the form's layout (`code_too_new` for a
//! preset this build does not have), then the spec ([`RandomSpec::validate`]: a newer generator or template is
//! refused too, because this build cannot regenerate its target).
//!
//! # Versions
//!
//! A code names its target forever: code v1's layout, its check, [`PRESETS`](crate::spec::PRESETS) and every frozen
//! generator are never edited. A new layout is a new version, decoded beside v1.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fingerprint::TargetFingerprint;
use crate::spec::{Category, Limits, RandomSpec, SpecError, SpecPreset};

/// The code format version this build writes, and the newest it reads.
pub const CODE_VERSION: u8 = 1;

/// The version reserved as an escape (F1r review N2): a later kind of code starts with it and a kind field follows.
/// This build reads none of them (`code_too_new`), and no layout version ever reaches it.
pub const CODE_VERSION_ESCAPE: u8 = 15;

const _: () = assert!(
    CODE_VERSION < CODE_VERSION_ESCAPE,
    "version 15 is the escape to a kind field, never a layout"
);

/// Seed bits in a short code: a short code holds seeds below 2⁴⁰.
pub const SHORT_SEED_BITS: u32 = 40;

/// Generator bits in a short code: a short code holds generator versions below 4096.
pub const SHORT_GENERATOR_BITS: u32 = 12;

/// Symbols in a short code, the check symbol included and hyphens not counted.
pub const SHORT_SYMBOLS: usize = 14;

/// The fewest symbols in a long code (no amounts set), the check symbol included.
pub const LONG_SYMBOLS_MIN: usize = 35;

/// The most symbols in a long code (every amount set), the check symbol included.
pub const LONG_SYMBOLS_MAX: usize = 44;

/// The 32 symbols: Crockford's alphabet, value = position. The check symbol is one of them too.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// GF(32)'s modulus: x⁵ + x² + 1, a primitive polynomial, so α = x has order 31.
const GF32_MODULUS: u8 = 0b10_0101;

/// Bits of a long code before its amounts: form, version, generator, seed, difficulty, musical, template, engine,
/// target prefix, amounts mask.
const LONG_FIXED_BITS: usize = 1 + 4 + 16 + 64 + 7 + 7 + 8 + 24 + 32 + 6;

/// Bits per amount in a long code.
const AMOUNT_BITS: usize = 7;

/// Data bits in a short code.
const SHORT_BITS: usize = 1 + 4 + 8 + 12 + 40;

/// Symbols in a group of the canonical text (`1K3M-QX9P-2HDT-VC`).
const GROUP: usize = 4;

/// The version of the engine a code was made with: three numbers, as `enth_patch::file::ENGINE_VERSION` writes them
/// (`0.2.0`). A code made with another engine still names the same target, so a difference only warns
/// ([`CodeWarning::EngineDiffers`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EngineVersion {
    /// Major.
    pub major: u8,
    /// Minor.
    pub minor: u8,
    /// Patch.
    pub patch: u8,
}

impl EngineVersion {
    /// This build's engine version. A version that does not parse as three numbers of at most 255 is a compile
    /// error here.
    pub const CURRENT: EngineVersion = match EngineVersion::parse(enth_patch::file::ENGINE_VERSION)
    {
        Some(v) => v,
        None => panic!("enth_patch::file::ENGINE_VERSION must be three numbers of at most 255"),
    };

    /// Parse `major.minor.patch`: three decimal numbers of at most 255, without leading zeros or signs.
    pub const fn parse(text: &str) -> Option<EngineVersion> {
        let b = text.as_bytes();
        let mut parts = [0u16; 3];
        let mut part = 0;
        let mut digits = 0;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c == b'.' {
                if digits == 0 || part == 2 {
                    return None;
                }
                part += 1;
                digits = 0;
            } else if c.is_ascii_digit() {
                if digits == 1 && parts[part] == 0 {
                    return None; // a leading zero
                }
                parts[part] = parts[part] * 10 + (c - b'0') as u16;
                if parts[part] > 255 {
                    return None;
                }
                digits += 1;
            } else {
                return None;
            }
            i += 1;
        }
        if part != 2 || digits == 0 {
            return None;
        }
        Some(EngineVersion {
            major: parts[0] as u8,
            minor: parts[1] as u8,
            patch: parts[2] as u8,
        })
    }
}

impl fmt::Display for EngineVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for EngineVersion {
    /// As its text, `"0.2.0"` (score reports and round-log lines carry it).
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EngineVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        EngineVersion::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format_args!(
                "{text:?} is not an engine version (major.minor.patch)"
            ))
        })
    }
}

/// The first 32 bits of a target's [`TargetFingerprint`], big-endian: what a long code carries so a build that
/// regenerates a different target refuses the code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetPrefix(pub u32);

impl TargetPrefix {
    /// The prefix of `fingerprint`.
    pub fn of(fingerprint: &TargetFingerprint) -> TargetPrefix {
        let h = fingerprint.0;
        TargetPrefix(u32::from_be_bytes([h[0], h[1], h[2], h[3]]))
    }
}

impl fmt::Display for TargetPrefix {
    /// Eight lowercase hex digits: the start of the fingerprint's `to_hex()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

/// Which form a code has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeForm {
    /// 14 symbols: a preset, the generator and the seed.
    Short,
    /// 35–44 symbols: the whole spec, the engine version and a target-fingerprint prefix.
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Form {
    Short,
    Long {
        engine: EngineVersion,
        target: TargetPrefix,
    },
}

/// A challenge code: a valid [`RandomSpec`] in its short or long form. Build one with [`ChallengeCode::short`] or
/// [`ChallengeCode::long`], or read one with [`ChallengeCode::decode`]; write it with [`ChallengeCode::encode`] (or
/// `Display`). Serde reads and writes the canonical text. See the [module docs](self) for the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChallengeCode {
    spec: RandomSpec,
    form: Form,
}

impl ChallengeCode {
    /// The short form of `spec`, when it has one: a valid spec whose settings are a [`SpecPreset`]'s, with a generator
    /// below 4096 and a seed below 2⁴⁰. `None` otherwise (use [`ChallengeCode::long`]).
    pub fn short(spec: RandomSpec) -> Option<ChallengeCode> {
        let fits = spec.validate().is_ok()
            && spec.preset().is_some()
            && u32::from(spec.generator) < 1 << SHORT_GENERATOR_BITS
            && spec.seed < 1 << SHORT_SEED_BITS;
        fits.then_some(ChallengeCode {
            spec,
            form: Form::Short,
        })
    }

    /// The long form of any valid `spec`, made with `engine` (usually [`EngineVersion::CURRENT`]), whose regenerated
    /// target's fingerprint starts with `target`.
    pub fn long(
        spec: RandomSpec,
        engine: EngineVersion,
        target: TargetPrefix,
    ) -> Result<ChallengeCode, SpecError> {
        spec.validate()?;
        Ok(ChallengeCode {
            spec,
            form: Form::Long { engine, target },
        })
    }

    /// The spec this code names.
    pub fn spec(&self) -> &RandomSpec {
        &self.spec
    }

    /// The form.
    pub fn form(&self) -> CodeForm {
        match self.form {
            Form::Short => CodeForm::Short,
            Form::Long { .. } => CodeForm::Long,
        }
    }

    /// The engine version a long code was made with; `None` for a short code.
    pub fn engine(&self) -> Option<EngineVersion> {
        match self.form {
            Form::Short => None,
            Form::Long { engine, .. } => Some(engine),
        }
    }

    /// A long code's target-fingerprint prefix; `None` for a short code.
    pub fn target(&self) -> Option<TargetPrefix> {
        match self.form {
            Form::Short => None,
            Form::Long { target, .. } => Some(target),
        }
    }

    /// The canonical text: upper case, in groups of four symbols joined by hyphens, the check symbol last
    /// (`1K3M-QX9P-2HDT-VC`).
    pub fn encode(&self) -> String {
        let mut data = self.data_symbols(CODE_VERSION);
        let check = check_value(&data);
        data.push(check);
        let mut out = String::with_capacity(data.len() + data.len() / GROUP);
        for (i, &v) in data.iter().enumerate() {
            if i > 0 && i % GROUP == 0 {
                out.push('-');
            }
            out.push(char::from(ALPHABET[usize::from(v)]));
        }
        out
    }

    /// Read a code (see the [module docs](self) for what is accepted and the order of checks). Never panics.
    pub fn decode(text: &str) -> Result<ChallengeCode, CodeError> {
        decode_within(text, &Limits::BUILD)
    }

    /// Check the regenerated target against this code. `target` is the fingerprint of the target this build
    /// generated from [`spec`](Self::spec) (`random::Generated::fingerprint`); `build` is this build's engine
    /// version.
    ///
    /// - A long code whose prefix differs is refused (`target_mismatch`): the round must never be scored against a
    ///   different target.
    /// - A long code made with another engine gives [`CodeWarning::EngineDiffers`]: the target is the same, only the
    ///   audio may differ.
    /// - A short code carries neither, so it always passes; its guarantee is the frozen generator and its goldens.
    pub fn verify(
        &self,
        target: &TargetFingerprint,
        build: EngineVersion,
    ) -> Result<Vec<CodeWarning>, CodeError> {
        let Form::Long {
            engine,
            target: prefix,
        } = self.form
        else {
            return Ok(Vec::new());
        };
        let found = TargetPrefix::of(target);
        if found != prefix {
            return Err(CodeError::TargetMismatch {
                code: prefix,
                found,
            });
        }
        let mut warnings = Vec::new();
        if engine != build {
            warnings.push(CodeWarning::EngineDiffers {
                code: engine,
                build,
            });
        }
        Ok(warnings)
    }

    /// The data symbols (no check symbol) with `version` in the header; `version` is a parameter only so the tests
    /// can write codes from a future version.
    fn data_symbols(&self, version: u8) -> Vec<u8> {
        let s = &self.spec;
        let mut w = Writer::default();
        match self.form {
            Form::Short => {
                // `short` admits only specs with a preset, so the fallback is never taken.
                let preset = s.preset().map_or(0, |p| p.id);
                w.put(0, 1);
                w.put(u64::from(version), 4);
                w.put(u64::from(preset), 8);
                w.put(u64::from(s.generator), SHORT_GENERATOR_BITS);
                w.put(s.seed, SHORT_SEED_BITS);
            }
            Form::Long { engine, target } => {
                w.put(1, 1);
                w.put(u64::from(version), 4);
                w.put(u64::from(s.generator), 16);
                w.put(s.seed, 64);
                w.put(u64::from(s.difficulty), 7);
                w.put(u64::from(s.musical), 7);
                w.put(u64::from(s.template), 8);
                w.put(u64::from(engine.major), 8);
                w.put(u64::from(engine.minor), 8);
                w.put(u64::from(engine.patch), 8);
                w.put(u64::from(target.0), 32);
                let mask = s
                    .amounts
                    .iter()
                    .fold(0u64, |m, a| m << 1 | u64::from(a.is_some()));
                w.put(mask, Category::COUNT as u32);
                for a in s.amounts.iter().flatten() {
                    w.put(u64::from(*a), AMOUNT_BITS as u32);
                }
            }
        }
        w.syms
    }
}

impl fmt::Display for ChallengeCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl FromStr for ChallengeCode {
    type Err = CodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChallengeCode::decode(s)
    }
}

impl Serialize for ChallengeCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.encode())
    }
}

impl<'de> Deserialize<'de> for ChallengeCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        ChallengeCode::decode(&text)
            .map_err(|e| serde::de::Error::custom(format_args!("{}: {e} ({})", e.code(), e.hint())))
    }
}

/// A difference that does not stop a round, reported by [`ChallengeCode::verify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeWarning {
    /// The code was made with another engine version: the target is the same, but its audio, and so the scores, may
    /// differ slightly from the code maker's.
    EngineDiffers {
        /// The engine the code was made with.
        code: EngineVersion,
        /// This build's.
        build: EngineVersion,
    },
}

impl CodeWarning {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            CodeWarning::EngineDiffers { .. } => "engine_differs",
        }
    }

    /// What it means for the player, in one line.
    pub fn hint(&self) -> String {
        match self {
            CodeWarning::EngineDiffers { code, build } => format!(
                "made with engine {code} and played on {build}: the target is the same, but it may sound slightly different"
            ),
        }
    }
}

impl fmt::Display for CodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeWarning::EngineDiffers { code, build } => {
                write!(
                    f,
                    "the code was made with engine {code}, this build has {build}"
                )
            }
        }
    }
}

/// Why a code is refused. Every variant has a stable [`code`](CodeError::code) and a [`hint`](CodeError::hint).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeError {
    /// Nothing but hyphens and spaces.
    Empty,
    /// A character that is not a code symbol (or a check-only symbol before the end).
    BadSymbol {
        /// The character.
        symbol: char,
        /// Its position in the text, counting characters from 0.
        position: usize,
    },
    /// Neither a short nor a long code's length, for the form its first symbol names.
    BadLength {
        /// The symbols found, hyphens and spaces not counted.
        symbols: usize,
    },
    /// The check symbol does not match the rest: a typo.
    BadCheck,
    /// Written by a newer code version than this build reads.
    TooNew {
        /// The code's version.
        version: u8,
        /// The newest this build reads ([`CODE_VERSION`]).
        supported: u8,
    },
    /// A short code names a preset this build does not have: preset ids are append-only, so a newer build made it.
    PresetTooNew {
        /// The preset id.
        preset: u8,
    },
    /// A field holds a value no encoder writes: version 0, preset 0, padding bits that are not zero.
    BadField {
        /// The field.
        field: &'static str,
        /// Its value.
        value: u64,
    },
    /// The spec it carries is refused, a newer generator included (`code_too_new`).
    Spec(SpecError),
    /// The target this build regenerates has another hash prefix than the code's ([`ChallengeCode::verify`]).
    TargetMismatch {
        /// The prefix in the code.
        code: TargetPrefix,
        /// The regenerated target's.
        found: TargetPrefix,
    },
}

impl CodeError {
    /// A stable snake-case code. A newer code version and a newer generator are both `code_too_new`.
    pub fn code(&self) -> &'static str {
        match self {
            CodeError::Empty => "empty_code",
            CodeError::BadSymbol { .. } => "bad_symbol",
            CodeError::BadLength { .. } => "bad_length",
            CodeError::BadCheck => "bad_check",
            CodeError::TooNew { .. } | CodeError::PresetTooNew { .. } => "code_too_new",
            CodeError::BadField { .. } => "bad_field",
            CodeError::Spec(e) => e.code(),
            CodeError::TargetMismatch { .. } => "target_mismatch",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            CodeError::Empty => "paste a challenge code: 14 symbols, or 35 to 44 for custom settings".to_owned(),
            CodeError::BadSymbol { .. } => {
                "codes use 0-9 and A-Z without I, L, O and U: check for a typo".to_owned()
            }
            CodeError::BadLength { .. } => format!(
                "a code has {SHORT_SYMBOLS} symbols, or {LONG_SYMBOLS_MIN} to {LONG_SYMBOLS_MAX} for custom settings, hyphens not counted: part of it is missing or extra"
            ),
            CodeError::BadCheck => "the check symbol does not match: there is a typo in the code".to_owned(),
            CodeError::TooNew { .. } => "made by a newer enth: update enth to play it".to_owned(),
            CodeError::PresetTooNew { preset } => {
                format!("made with preset {preset}, from a newer enth: update enth to play it")
            }
            CodeError::BadField { .. } => "the code is damaged, or was not made by enth".to_owned(),
            CodeError::Spec(e) => e.hint(),
            CodeError::TargetMismatch { .. } => {
                "this enth regenerates a different target from this code: please report it with the code".to_owned()
            }
        }
    }
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Empty => f.write_str("the code is empty"),
            CodeError::BadSymbol { symbol, position } => {
                write!(f, "{symbol:?} at position {position} is not a code symbol")
            }
            CodeError::BadLength { symbols } => write!(f, "{symbols} symbols is not a code length"),
            CodeError::BadCheck => f.write_str("the check symbol does not match"),
            CodeError::TooNew { version, supported } => write!(
                f,
                "code version {version} is newer than this build's {supported}"
            ),
            CodeError::PresetTooNew { preset } => {
                write!(f, "preset {preset} is newer than this build's presets")
            }
            CodeError::BadField { field, value } => write!(f, "{field} {value} is not valid"),
            CodeError::Spec(e) => write!(f, "{e}"),
            CodeError::TargetMismatch { code, found } => write!(
                f,
                "the code's target starts {code}, the regenerated one {found}"
            ),
        }
    }
}

impl std::error::Error for CodeError {}

impl From<SpecError> for CodeError {
    fn from(e: SpecError) -> Self {
        CodeError::Spec(e)
    }
}

/// `v · α` in GF(32): a shift, reduced by x⁵ + x² + 1.
const fn times_alpha(v: u8) -> u8 {
    let w = v << 1;
    if w & 0b10_0000 != 0 {
        w ^ GF32_MODULUS
    } else {
        w
    }
}

/// The word's polynomial at α (Horner, first symbol highest): Σ sᵢ·α^(n−1−i), additions being XOR.
fn at_alpha(symbols: &[u8]) -> u8 {
    symbols.iter().fold(0u8, |acc, &s| times_alpha(acc) ^ s)
}

/// The check symbol: the value c that makes the data followed by c vanish at α, which is α · (the data at α).
fn check_value(data: &[u8]) -> u8 {
    times_alpha(at_alpha(data))
}

/// The value of a symbol (0–31), after Crockford's case and alias rules.
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

/// The most symbols [`decode`](ChallengeCode::decode) reads before it gives up: above every known length.
const MAX_SYMBOLS: usize = 64;

/// [`ChallengeCode::decode`] with other spec limits (the tests cover every value a field can hold this way).
fn decode_within(text: &str, limits: &Limits) -> Result<ChallengeCode, CodeError> {
    let mut syms: Vec<(u8, usize, char)> = Vec::with_capacity(LONG_SYMBOLS_MAX);
    let mut count = 0usize;
    for (position, c) in text.chars().enumerate() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        count += 1;
        if count > MAX_SYMBOLS {
            continue; // keep counting for the error, read nothing more
        }
        let v = symbol_value(c).ok_or(CodeError::BadSymbol {
            symbol: c,
            position,
        })?;
        syms.push((v, position, c));
    }
    if count == 0 {
        return Err(CodeError::Empty);
    }
    if !(2..=MAX_SYMBOLS).contains(&count) {
        return Err(CodeError::BadLength { symbols: count });
    }
    let values: Vec<u8> = syms.iter().map(|&(v, _, _)| v).collect();
    if at_alpha(&values) != 0 {
        return Err(CodeError::BadCheck);
    }
    let values = &values[..values.len() - 1];
    let first = values[0];
    let long = first & 0b1_0000 != 0;
    let version = first & 0b0_1111;
    if version == 0 {
        return Err(CodeError::BadField {
            field: "version",
            value: 0,
        });
    }
    if version > CODE_VERSION {
        return Err(CodeError::TooNew {
            version,
            supported: CODE_VERSION,
        });
    }
    let code = if long {
        decode_long(values)?
    } else {
        decode_short(values)?
    };
    code.spec.validate_within(limits)?;
    Ok(code)
}

fn decode_short(values: &[u8]) -> Result<ChallengeCode, CodeError> {
    if values.len() + 1 != SHORT_SYMBOLS {
        return Err(CodeError::BadLength {
            symbols: values.len() + 1,
        });
    }
    let mut r = Reader::new(values);
    r.take(5)?; // form and version, read by the caller
    let preset_id = r.take(8)? as u8;
    let generator = r.take(SHORT_GENERATOR_BITS as usize)? as u16;
    let seed = r.take(SHORT_SEED_BITS as usize)?;
    r.zero_padding(values.len() * 5 - SHORT_BITS)?;
    if preset_id == 0 {
        return Err(CodeError::BadField {
            field: "preset",
            value: 0,
        });
    }
    // Preset ids are append-only: one this build does not have is a newer build's.
    let preset =
        SpecPreset::by_id(preset_id).ok_or(CodeError::PresetTooNew { preset: preset_id })?;
    Ok(ChallengeCode {
        spec: preset.spec(generator, seed),
        form: Form::Short,
    })
}

fn decode_long(values: &[u8]) -> Result<ChallengeCode, CodeError> {
    let symbols = values.len() + 1;
    if values.len() * 5 < LONG_FIXED_BITS {
        return Err(CodeError::BadLength { symbols });
    }
    let mut r = Reader::new(values);
    r.take(5)?; // form and version, read by the caller
    let generator = r.take(16)? as u16;
    let seed = r.take(64)?;
    let difficulty = r.take(7)? as u8;
    let musical = r.take(7)? as u8;
    let template = r.take(8)? as u8;
    let engine = EngineVersion {
        major: r.take(8)? as u8,
        minor: r.take(8)? as u8,
        patch: r.take(8)? as u8,
    };
    let target = TargetPrefix(r.take(32)? as u32);
    let mask = r.take(Category::COUNT)?;
    let bits = LONG_FIXED_BITS + mask.count_ones() as usize * AMOUNT_BITS;
    if values.len() != bits.div_ceil(5) {
        return Err(CodeError::BadLength { symbols });
    }
    let mut amounts = [None; Category::COUNT];
    for (i, a) in amounts.iter_mut().enumerate() {
        if mask >> (Category::COUNT - 1 - i) & 1 == 1 {
            *a = Some(r.take(AMOUNT_BITS)? as u8);
        }
    }
    r.zero_padding(values.len() * 5 - bits)?;
    let spec = RandomSpec {
        generator,
        seed,
        difficulty,
        amounts,
        musical,
        template,
    };
    Ok(ChallengeCode {
        spec,
        form: Form::Long { engine, target },
    })
}

/// Writes bits most significant first into 5-bit symbols; the last symbol's unused low bits stay zero.
#[derive(Default)]
struct Writer {
    syms: Vec<u8>,
    bits: usize,
}

impl Writer {
    /// The low `width` bits of `value`, high bit first.
    fn put(&mut self, value: u64, width: u32) {
        for i in (0..width).rev() {
            let bit = ((value >> i) & 1) as u8;
            let at = self.bits % 5;
            if at == 0 {
                self.syms.push(0);
            }
            if let Some(last) = self.syms.last_mut() {
                *last |= bit << (4 - at);
            }
            self.bits += 1;
        }
    }
}

/// Reads bits most significant first from 5-bit symbols.
struct Reader<'a> {
    syms: &'a [u8],
    bit: usize,
}

impl<'a> Reader<'a> {
    fn new(syms: &'a [u8]) -> Self {
        Reader { syms, bit: 0 }
    }

    /// The next `width` bits (at most 64) as a number; `bad_length` past the end.
    fn take(&mut self, width: usize) -> Result<u64, CodeError> {
        let mut v = 0u64;
        for _ in 0..width {
            let sym = self.syms.get(self.bit / 5).ok_or(CodeError::BadLength {
                symbols: self.syms.len() + 1,
            })?;
            v = v << 1 | u64::from((sym >> (4 - self.bit % 5)) & 1);
            self.bit += 1;
        }
        Ok(v)
    }

    /// The last `width` bits must be zero.
    fn zero_padding(&mut self, width: usize) -> Result<(), CodeError> {
        let pad = self.take(width)?;
        if pad == 0 {
            Ok(())
        } else {
            Err(CodeError::BadField {
                field: "padding",
                value: pad,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    //! P1 (docs/phase-2.md): `decode(encode(c)) == c`; a corrupted code gives an error, never a panic; the check
    //! symbol catches every single substitution and adjacent swap. Plus the versioning rules.

    use proptest::prelude::*;

    use super::*;
    use crate::spec::{LATEST_GENERATOR, MAX_SEED, PERCENT_MAX, PRESETS};

    fn percent() -> impl Strategy<Value = u8> {
        0..=PERCENT_MAX
    }

    /// Any spec the long form must carry: every `u16` generator but 0, every seed and template (the tests decode
    /// with [`Limits::ANY`]).
    fn any_spec() -> impl Strategy<Value = RandomSpec> {
        (
            1..=u16::MAX,
            any::<u64>(),
            percent(),
            proptest::array::uniform6(proptest::option::of(percent())),
            percent(),
            any::<u8>(),
        )
            .prop_map(
                |(generator, seed, difficulty, amounts, musical, template)| RandomSpec {
                    generator,
                    seed,
                    difficulty,
                    amounts,
                    musical,
                    template,
                },
            )
    }

    fn any_engine() -> impl Strategy<Value = EngineVersion> {
        (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(major, minor, patch)| EngineVersion {
            major,
            minor,
            patch,
        })
    }

    /// A long code over any spec (built directly: `long` checks against this build's generators).
    fn any_long() -> impl Strategy<Value = ChallengeCode> {
        (any_spec(), any_engine(), any::<u32>()).prop_map(|(spec, engine, t)| ChallengeCode {
            spec,
            form: Form::Long {
                engine,
                target: TargetPrefix(t),
            },
        })
    }

    /// A short code over any preset, generator below 4096 and seed below 2⁴⁰.
    fn any_short() -> impl Strategy<Value = ChallengeCode> {
        (
            0..PRESETS.len(),
            1u16..(1 << SHORT_GENERATOR_BITS),
            0u64..(1 << SHORT_SEED_BITS),
        )
            .prop_map(|(p, generator, seed)| ChallengeCode {
                spec: PRESETS[p].spec(generator, seed),
                form: Form::Short,
            })
    }

    fn any_code() -> impl Strategy<Value = ChallengeCode> {
        prop_oneof![any_short(), any_long()]
    }

    fn decode_any(text: &str) -> Result<ChallengeCode, CodeError> {
        decode_within(text, &Limits::ANY)
    }

    /// The symbols of a canonical code, hyphens dropped.
    fn symbols(code: &ChallengeCode) -> Vec<u8> {
        code.encode().bytes().filter(|&b| b != b'-').collect()
    }

    fn text(symbols: &[u8]) -> String {
        symbols.iter().map(|&b| char::from(b)).collect()
    }

    proptest! {
        #[test]
        fn p1_codes_round_trip(code in any_code()) {
            let text = code.encode();
            prop_assert_eq!(decode_any(&text), Ok(code));
            // Lower case, aliases and other hyphenation read the same.
            let loose = text.replace('-', "").to_ascii_lowercase().replace('0', "o").replace('1', "l");
            prop_assert_eq!(decode_any(&loose), Ok(code));
            let len = symbols(&code).len();
            match code.form() {
                CodeForm::Short => prop_assert_eq!(len, SHORT_SYMBOLS),
                CodeForm::Long => prop_assert!((LONG_SYMBOLS_MIN..=LONG_SYMBOLS_MAX).contains(&len)),
            }
        }

        #[test]
        fn p1_every_single_substitution_is_caught(code in any_code()) {
            let syms = symbols(&code);
            for (i, _) in syms.iter().enumerate() {
                for &b in ALPHABET {
                    if b == syms[i] {
                        continue;
                    }
                    let mut bad = syms.clone();
                    bad[i] = b;
                    prop_assert!(decode_any(&text(&bad)).is_err(), "position {} -> {}", i, char::from(b));
                }
            }
        }

        #[test]
        fn p1_every_adjacent_swap_is_caught(code in any_code()) {
            let syms = symbols(&code);
            for i in 0..syms.len() - 1 {
                if syms[i] == syms[i + 1] {
                    continue;
                }
                let mut bad = syms.clone();
                bad.swap(i, i + 1);
                prop_assert!(decode_any(&text(&bad)).is_err(), "swap {}", i);
            }
        }

        #[test]
        fn p1_a_dropped_or_added_symbol_in_a_short_code_is_caught(code in any_short(), at in 0usize..SHORT_SYMBOLS, b in 0usize..32) {
            let syms = symbols(&code);
            let mut dropped = syms.clone();
            dropped.remove(at);
            prop_assert!(decode_any(&text(&dropped)).is_err());
            let mut added = syms;
            added.insert(at, ALPHABET[b]);
            prop_assert!(decode_any(&text(&added)).is_err());
        }

        #[test]
        fn p1_garbage_never_panics(s in "\\PC{0,80}") {
            let _ = ChallengeCode::decode(&s);
        }

        #[test]
        fn p1_symbol_soup_never_panics(s in "[0-9A-Za-z*~$=-]{0,70}") {
            let _ = ChallengeCode::decode(&s);
        }

        #[test]
        fn engine_versions_round_trip_through_text(v in any_engine()) {
            prop_assert_eq!(EngineVersion::parse(&v.to_string()), Some(v));
        }
    }

    fn level(id: u8, seed: u64) -> RandomSpec {
        SpecPreset::by_id(id).unwrap().spec(1, seed)
    }

    fn custom() -> RandomSpec {
        RandomSpec {
            generator: 1,
            seed: MAX_SEED,
            difficulty: 40,
            amounts: [None, Some(100), None, None, Some(0), None],
            musical: 70,
            template: 0,
        }
    }

    /// Re-encode a code with another version in its header and a valid check symbol.
    fn with_version(code: &ChallengeCode, version: u8) -> String {
        let mut data = code.data_symbols(version);
        data.push(check_value(&data));
        data.iter()
            .map(|&v| char::from(ALPHABET[usize::from(v)]))
            .collect()
    }

    /// Re-encode raw data symbols with a valid check symbol.
    fn with_check(mut data: Vec<u8>) -> String {
        data.push(check_value(&data));
        data.iter()
            .map(|&v| char::from(ALPHABET[usize::from(v)]))
            .collect()
    }

    #[test]
    fn short_codes_start_with_1_and_long_ones_with_h() {
        let short = ChallengeCode::short(level(1, 42)).unwrap();
        assert!(short.encode().starts_with('1'), "{short}");
        assert_eq!(short.encode().len(), SHORT_SYMBOLS + 3, "three hyphens");
        let long = ChallengeCode::long(custom(), EngineVersion::CURRENT, TargetPrefix(7)).unwrap();
        assert!(long.encode().starts_with('H'), "{long}");
        assert_eq!(long.form(), CodeForm::Long);
        assert_eq!(long.engine(), Some(EngineVersion::CURRENT));
        assert_eq!(long.target(), Some(TargetPrefix(7)));
        assert_eq!(short.engine(), None);
        assert_eq!(short.target(), None);
    }

    #[test]
    fn short_needs_a_preset_a_short_seed_and_a_short_generator() {
        assert!(ChallengeCode::short(level(3, (1 << SHORT_SEED_BITS) - 1)).is_some());
        assert!(ChallengeCode::short(level(3, 1 << SHORT_SEED_BITS)).is_none());
        assert!(ChallengeCode::short(custom()).is_none());
        let mut s = level(3, 1);
        s.generator = 0;
        assert!(ChallengeCode::short(s).is_none());
        s.generator = LATEST_GENERATOR + 1;
        assert!(
            ChallengeCode::short(s).is_none(),
            "not a generator this build runs"
        );
        assert!(ChallengeCode::long(s, EngineVersion::CURRENT, TargetPrefix(0)).is_err());
    }

    #[test]
    fn a_newer_code_version_is_refused() {
        let code = ChallengeCode::short(level(2, 9)).unwrap();
        assert_eq!(decode_any(&with_version(&code, 1)), Ok(code));
        let e = decode_any(&with_version(&code, 2)).unwrap_err();
        assert_eq!(
            e,
            CodeError::TooNew {
                version: 2,
                supported: 1
            }
        );
        assert_eq!(e.code(), "code_too_new");
        let long = ChallengeCode::long(custom(), EngineVersion::CURRENT, TargetPrefix(1)).unwrap();
        for version in 2..=14 {
            assert_eq!(
                decode_any(&with_version(&long, version))
                    .unwrap_err()
                    .code(),
                "code_too_new"
            );
        }
        assert_eq!(
            decode_any(&with_version(&code, 0)).unwrap_err(),
            CodeError::BadField {
                field: "version",
                value: 0
            }
        );
    }

    #[test]
    fn a_newer_generator_is_refused() {
        let mut spec = level(2, 9);
        spec.generator = LATEST_GENERATOR + 1;
        let code = ChallengeCode {
            spec,
            form: Form::Short,
        };
        let text = code.encode();
        assert_eq!(decode_any(&text), Ok(code), "the codec carries it");
        let e = ChallengeCode::decode(&text).unwrap_err();
        assert_eq!(
            e,
            CodeError::Spec(SpecError::GeneratorTooNew {
                generator: LATEST_GENERATOR + 1,
                latest: LATEST_GENERATOR
            })
        );
        assert_eq!(e.code(), "code_too_new");
        assert!(e.hint().contains("update enth"), "{}", e.hint());
    }

    /// N2 (F1r review): version 15 is the escape to a kind field, in both forms; this build reads no kind, so a code
    /// that uses it is a newer build's.
    #[test]
    fn version_15_is_the_reserved_escape() {
        for first in [CODE_VERSION_ESCAPE, 0b1_0000 | CODE_VERSION_ESCAPE] {
            // The header symbol, a kind field and some payload: whatever follows, it is too new.
            let text = with_check(vec![first, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            let e = decode_any(&text).unwrap_err();
            assert_eq!(
                e,
                CodeError::TooNew {
                    version: CODE_VERSION_ESCAPE,
                    supported: CODE_VERSION
                }
            );
            assert_eq!(e.code(), "code_too_new");
        }
        assert_eq!(char::from(ALPHABET[usize::from(CODE_VERSION_ESCAPE)]), 'F');
        assert_eq!(
            char::from(ALPHABET[16 | usize::from(CODE_VERSION_ESCAPE)]),
            'Z'
        );
    }

    fn short_with_preset(preset: u8) -> String {
        let mut w = Writer::default();
        w.put(0, 1);
        w.put(1, 4);
        w.put(u64::from(preset), 8);
        w.put(1, 12);
        w.put(5, 40);
        with_check(w.syms)
    }

    /// S1 (F1r review): a preset this build does not have is a newer build's (ids are append-only), so it is
    /// `code_too_new`, not damage; preset 0 is never written.
    #[test]
    fn an_unknown_preset_is_too_new_and_generator_0_is_refused() {
        for preset in [6, 7, 200, 255] {
            let e = decode_any(&short_with_preset(preset)).unwrap_err();
            assert_eq!(e, CodeError::PresetTooNew { preset });
            assert_eq!(e.code(), "code_too_new");
            assert!(e.hint().contains("update enth"), "{}", e.hint());
        }
        assert_eq!(
            decode_any(&short_with_preset(0)),
            Err(CodeError::BadField {
                field: "preset",
                value: 0
            })
        );
        let mut w = Writer::default();
        w.put(0, 1);
        w.put(1, 4);
        w.put(1, 8);
        w.put(0, 12);
        w.put(5, 40);
        assert_eq!(
            decode_any(&with_check(w.syms)),
            Err(CodeError::Spec(SpecError::NoGenerator))
        );
    }

    #[test]
    fn out_of_range_fields_and_padding_are_refused() {
        let long = ChallengeCode::long(custom(), EngineVersion::CURRENT, TargetPrefix(1)).unwrap();
        let mut data = long.data_symbols(1);
        // Difficulty sits at bits 85..92: set it to 127.
        let mut bits: Vec<u8> = data
            .iter()
            .flat_map(|s| (0..5).rev().map(move |i| (s >> i) & 1))
            .collect();
        for b in &mut bits[85..92] {
            *b = 1;
        }
        let repack = |bits: &[u8]| -> Vec<u8> {
            bits.chunks(5)
                .map(|c| c.iter().fold(0u8, |v, b| v << 1 | b))
                .collect()
        };
        assert_eq!(
            decode_any(&with_check(repack(&bits))).unwrap_err().code(),
            "out_of_range"
        );
        // A set padding bit (169 + 7·2 = 183 bits in 37 symbols: 2 padding bits).
        assert_eq!(data.len(), 37);
        *data.last_mut().unwrap() |= 1;
        assert_eq!(
            decode_any(&with_check(data)),
            Err(CodeError::BadField {
                field: "padding",
                value: 1
            })
        );
    }

    /// S1 and S4 (F1r review): a long code carries any template and seed, and this build refuses a newer template
    /// (`code_too_new`) and a seed a JSON reader could change (`out_of_range`).
    #[test]
    fn a_newer_template_or_a_huge_seed_in_a_long_code_is_refused() {
        let mut spec = custom();
        spec.template = 3;
        let code = ChallengeCode {
            spec,
            form: Form::Long {
                engine: EngineVersion::CURRENT,
                target: TargetPrefix(9),
            },
        };
        assert_eq!(decode_any(&code.encode()), Ok(code), "the codec carries it");
        let e = ChallengeCode::decode(&code.encode()).unwrap_err();
        assert_eq!(e.code(), "code_too_new");
        assert!(e.hint().contains("template 3"), "{}", e.hint());
        let mut spec = custom();
        spec.seed = MAX_SEED + 1;
        let code = ChallengeCode {
            spec,
            form: Form::Long {
                engine: EngineVersion::CURRENT,
                target: TargetPrefix(9),
            },
        };
        assert_eq!(decode_any(&code.encode()), Ok(code));
        assert_eq!(
            ChallengeCode::decode(&code.encode()).unwrap_err().code(),
            "out_of_range"
        );
        assert!(ChallengeCode::long(spec, EngineVersion::CURRENT, TargetPrefix(0)).is_err());
    }

    /// S2 (F1r review): every check symbol is a data symbol, so no code ends in punctuation or `U`; the old mod-37
    /// check put one in about every seventh code.
    #[test]
    fn check_symbols_are_data_symbols() {
        let mut seen = [false; 32];
        for seed in 0..2_000u64 {
            let text = ChallengeCode::short(level(1 + (seed % 5) as u8, seed * 7_919))
                .unwrap()
                .encode();
            let last = text.bytes().last().unwrap();
            let v = ALPHABET
                .iter()
                .position(|&b| b == last)
                .expect("a data symbol");
            seen[v] = true;
        }
        assert!(seen.iter().all(|s| *s), "every symbol occurs as a check");
        for bad in ['*', '~', '$', '=', 'U', 'u'] {
            let text = format!("1080-1000-0001-A{bad}");
            assert_eq!(
                ChallengeCode::decode(&text).unwrap_err().code(),
                "bad_symbol"
            );
        }
    }

    /// The check's arithmetic: α has order 31 in GF(32), so the weights α^j at adjacent places always differ and are
    /// never 0, which is what the substitution and swap arguments need.
    #[test]
    fn alpha_is_primitive() {
        let mut v = 1u8;
        for n in 1..=31 {
            v = times_alpha(v);
            assert!(v < 32 && v != 0);
            assert_eq!(v == 1, n == 31, "order 31: {n}");
        }
    }

    #[test]
    fn errors_name_what_went_wrong() {
        assert_eq!(ChallengeCode::decode(""), Err(CodeError::Empty));
        assert_eq!(ChallengeCode::decode(" - -"), Err(CodeError::Empty));
        assert_eq!(
            ChallengeCode::decode("1K3M-QX9P-2HDT-V!"),
            Err(CodeError::BadSymbol {
                symbol: '!',
                position: 16
            })
        );
        // `U` is no symbol at all.
        assert_eq!(
            ChallengeCode::decode("U0000000000000").unwrap_err().code(),
            "bad_symbol"
        );
        assert_eq!(
            ChallengeCode::decode("1").unwrap_err(),
            CodeError::BadLength { symbols: 1 }
        );
        let long_garbage = "0".repeat(200);
        assert_eq!(
            ChallengeCode::decode(&long_garbage).unwrap_err(),
            CodeError::BadLength { symbols: 200 }
        );
        // A valid check over a short-form header of the wrong length.
        assert_eq!(
            decode_any(&with_check(vec![1, 0, 0])).unwrap_err(),
            CodeError::BadLength { symbols: 4 }
        );
        assert_eq!(
            decode_any(&with_check(vec![17, 0, 0])).unwrap_err(),
            CodeError::BadLength { symbols: 4 }
        );
        let code = ChallengeCode::short(level(1, 1)).unwrap().encode();
        let mut typo: Vec<char> = code.chars().collect();
        typo[1] = if typo[1] == 'Z' { 'Y' } else { 'Z' };
        let typo: String = typo.into_iter().collect();
        assert_eq!(ChallengeCode::decode(&typo), Err(CodeError::BadCheck));
        for e in [
            CodeError::Empty,
            CodeError::BadCheck,
            CodeError::BadLength { symbols: 3 },
        ] {
            assert!(!e.hint().is_empty() && !e.to_string().is_empty());
        }
    }

    #[test]
    fn verify_refuses_another_target_and_warns_on_another_engine() {
        let hash = TargetFingerprint([0xAB; 32]);
        let other = TargetFingerprint([0xAC; 32]);
        let long =
            ChallengeCode::long(custom(), EngineVersion::CURRENT, TargetPrefix::of(&hash)).unwrap();
        assert_eq!(long.verify(&hash, EngineVersion::CURRENT), Ok(vec![]));
        let e = long.verify(&other, EngineVersion::CURRENT).unwrap_err();
        assert_eq!(e.code(), "target_mismatch");
        let newer = EngineVersion {
            major: 9,
            minor: 0,
            patch: 0,
        };
        let w = long.verify(&hash, newer).unwrap();
        assert_eq!(
            w,
            vec![CodeWarning::EngineDiffers {
                code: EngineVersion::CURRENT,
                build: newer
            }]
        );
        assert_eq!(w[0].code(), "engine_differs");
        // A short code carries neither, so it has nothing to check.
        let short = ChallengeCode::short(level(4, 3)).unwrap();
        assert_eq!(short.verify(&other, newer), Ok(vec![]));
    }

    #[test]
    fn target_prefixes_are_the_start_of_the_hex_fingerprint() {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&[0x01, 0x23, 0xab, 0xcd]);
        let f = TargetFingerprint(h);
        assert_eq!(TargetPrefix::of(&f).to_string(), f.to_hex()[..8]);
        assert_eq!(f.prefix(), TargetPrefix(0x0123_abcd));
    }

    #[test]
    fn engine_versions_parse_strictly() {
        assert_eq!(
            EngineVersion::parse("0.2.0"),
            Some(EngineVersion {
                major: 0,
                minor: 2,
                patch: 0
            })
        );
        assert_eq!(
            EngineVersion::CURRENT.to_string(),
            enth_patch::file::ENGINE_VERSION
        );
        for bad in [
            "", "1", "1.2", "1.2.3.4", "1..2", "01.2.3", "1.2.256", "1.2.-3", "1.2.3 ", "a.b.c",
        ] {
            assert_eq!(EngineVersion::parse(bad), None, "{bad:?}");
        }
        assert!(EngineVersion::parse("255.10.0").is_some());
    }

    #[test]
    fn serde_uses_the_canonical_text() {
        let code = ChallengeCode::short(level(5, 123_456)).unwrap();
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, format!("\"{code}\""));
        assert_eq!(serde_json::from_str::<ChallengeCode>(&json).unwrap(), code);
        let e = serde_json::from_str::<ChallengeCode>("\"1234\"").unwrap_err();
        assert!(e.to_string().contains("bad_"), "{e}");
    }
}
