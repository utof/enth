//! **Owner: lane RC** (Result codes, freeze point F1e; the F1e judge fixes each layout). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e), which reserved the version-15 kind numbers below. The reference is docs/challenge.md, "Version
//! 15: the kinds behind the escape", and docs/duels.md, "The two codes".
//!
//! The kinds behind code v1's reserved version-15 escape ([`CODE_VERSION_ESCAPE`](crate::code::CODE_VERSION_ESCAPE)), and
//! the frame they share. This file holds the frame, the **scorecard** (kind 1) and the **ear result** (kind 4);
//! [`answer`](crate::answer) holds the answer code (kind 2), [`verify`](crate::verify) the receiver's re-score,
//! [`paste`](crate::paste) the paste-anything decoding and [`daily`](crate::daily) the daily formula.
//!
//! | Kind | Constant | What it carries | Lane |
//! | --- | --- | --- | --- |
//! | 1 | [`KIND_SCORECARD`] | the spoiler-free scorecard: the challenge, engine, scorer set, scores, time, assists and a salted commitment | RC |
//! | 2 | [`KIND_ANSWER`] | the answer code: the patch as a diff from the round's start through the frozen `result-params-v1` table | RC |
//! | 3 | [`KIND_EAR_CHALLENGE`] | an `EarSpec` on a fixed ladder, for ear duels | CV3 (`ear::code`) |
//! | 4 | [`KIND_EAR_RESULT`] | the ear result: the answers, the time and the assists, verified by replay with no render | RC |
//! | 5 | [`KIND_LESSON_SIBLING`] | reserved: (pack, lesson, step, seed); nothing plays it yet | none |
//!
//! # The frame
//!
//! Bits are written most significant first and cut into 5-bit symbols of Crockford's alphabet, as in code v1, and the
//! text is grouped in fours with hyphens (so a code never holds `--`, which phones turn into a dash). Every kind is:
//!
//! ```text
//! header   form 1 = 0 | version 4 = 15 | kind 5 | layout 4      14 bits; the first symbol is always `F`
//! fields   the kind's own, then zero bits to the end of a symbol
//! check    the kind's own: 30 bits (6 symbols) of SHA-256 for the scorecard and the ear result, a 40-bit tail
//!          (8 symbols) for the answer code
//! GF(32)   code v1's check symbol over everything before it
//! ```
//!
//! **Why code v1's symbol comes last on every kind:** an R1 to R3 build's `decode` checks the length (2 to 64
//! symbols, `bad_length`), then this symbol (`bad_check`), then the version (`code_too_new` for 15). With the symbol
//! last, a code of 64 symbols or fewer reads "made by a newer enth: update enth" on an old build, and only a longer one
//! reads `bad_length`, which no later change can fix. The symbol also catches every single substitution and every
//! adjacent swap at any length (docs/challenge.md has the argument); the kind's own check catches the rest of random
//! damage and truncation (the scorecard's length is fixed per challenge form, an answer's is not).
//!
//! The form bit of the first symbol is 0 on every kind this build knows; a code starting `Z` (form 1, version 15) is
//! reserved and refused as `code_too_new`, like an unknown kind or layout, before any field is read. Kind 0 and layout
//! 0 are never written (`bad_field`).
//!
//! # The scorecard (kind 1, layout 1)
//!
//! ```text
//! header                               14
//! challenge   flag 1 = 0: short        60   preset 8 | generator 12 | seed 40      (code v1's short payload)
//!             flag 1 = 1: long    164 + 7k   generator 16 | seed 64 | difficulty 7 | musical 7 | template 8 |
//!                                            engine 24 | target prefix 32 | amounts mask 6 | 7 per set amount
//! engine                               24   major | minor | patch of the build that scored the claim
//! scorer set                            8   an index into SCORER_SETS (a newer build's is read and shown as reported)
//! sound, combined                 10 + 10   claimed percent in tenths, 0..=1000
//! passed                                1
//! submits                               7   1..=127 (127 means "127 or more")
//! time                                 16   deciseconds, saturating: the time to pass, or the round's active time
//!                                           when it did not pass
//! assists                       4 + 1 × 4   hints (15 means "15 or more"), live meter, dim unused, closest, revealed
//! commitment                           60   the salted commitment to the answer (see `answer`)
//! ```
//!
//! 219 bits with a short challenge: 44 data symbols, 6 check symbols and code v1's symbol, **51 symbols, always**. A
//! long challenge gives 72 to 81 (over 64: `bad_length` on an old build, as challenge.md says). Names, ids and
//! absolute times have no field; the time is a duration from the shell's clock, and the scores are claims the
//! receiver re-scores ([`crate::verify`]). A revealed target gives no scorecard unless the round passed first.
//!
//! **A scorer set this build does not have still reads** (the F1e review's M4): the card shows the friend's result as
//! reported ([`ScorerSet::is_known`] is false), and [`crate::verify`] re-scores with this build's current set and
//! flags the difference ([`crate::verify::ClaimCheck::scorers_differ`]), so a duel across GR1's `sound/2` works both
//! ways: the newer build keeps set 1 and checks the older one exactly, the older one checks with what it has.
//!
//! # The ear result (kind 4, layout 1)
//!
//! ```text
//! header                               14
//! ear challenge                        72   game 8 | generator 8 | level 8 | seed 40 | count 8 (kind 3's payload)
//! ear rules                             8   an index into EAR_RULES: the ear scorers and the pass mark (1: ear/1,
//!                                           ear-value/1, taps/1, pass 80); a newer build's is read
//! answers           count × (2 + n)         tag 2: 0 none | 1 choice (4) | 2 value (an f32, the float coding) |
//!                                           3 taps (count 6, then each tap after the first as ms from the first,
//!                                           14 bits, non-decreasing; the first is 0 and not written)
//! time                                 16   deciseconds, saturating
//! assists                               8   hints 4 | revealed 1 | 3 zero bits (reserved)
//! ```
//!
//! Then the 30-bit check and code v1's symbol: 50 symbols for 16 choice answers (about 29 choice answers fit 64). The types are the ear vocabulary's
//! (the F1e review's M3): the spec is `enth-music`'s [`EarSpec`], an answer is [`EarAnswer`] from
//! [`crate::ear::scorers`]. **Tap answers are stored relative to their first tap**, on any clock the shell used: at
//! most [`EAR_TAPS_MAX`] taps over a span of at most [`EAR_TAP_MS_MAX`] ms, non-decreasing, the limits `taps/1`
//! itself judges by, and `taps/1` fits the offset, so the shift changes no score. Verified by replaying the answers
//! against the keys the ear generator gives, with no render ([`crate::verify::verify_ear`]). Anyone holding the code
//! can compute the keys, and the UI says so.

use core::fmt;
use core::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::code::{
    self, CODE_VERSION_ESCAPE, ChallengeCode, CodeError, CodeForm, EngineVersion, TargetPrefix,
};
use crate::ear::scorers;
pub use crate::ear::scorers::EarAnswer;
use crate::model::{AnswerKind, AssistsUsed, RECREATE_SCORERS, ScoringSpec};
use crate::score::{ScoreReport, ScorerId};
use crate::spec::{Category, RandomSpec, SpecPreset};
use crate::v3::{EAR_PASS_MARK, EarError, EarSpec};

/// Kind 1 behind version 15: the **scorecard**, sent on finishing. It spoils nothing.
pub const KIND_SCORECARD: u8 = 1;
/// Kind 2 behind version 15: the **answer code**, swapped after both players have played. It carries the patch as a
/// diff, so it spoils the answer.
pub const KIND_ANSWER: u8 = 2;
/// Kind 3 behind version 15: the **ear challenge** (an `EarSpec` on a fixed ladder), for ear duels.
pub const KIND_EAR_CHALLENGE: u8 = 3;
/// Kind 4 behind version 15: the **ear result** (the answers, the time and the assists), verified by replay.
pub const KIND_EAR_RESULT: u8 = 4;
/// Kind 5 behind version 15: reserved for a lesson sibling (pack, lesson, step, seed). Nothing plays it yet.
pub const KIND_LESSON_SIBLING: u8 = 5;

/// Every reserved kind, in number order.
pub const KINDS: [u8; 5] = [
    KIND_SCORECARD,
    KIND_ANSWER,
    KIND_EAR_CHALLENGE,
    KIND_EAR_RESULT,
    KIND_LESSON_SIBLING,
];

/// The first layout of every kind. A kind's layout changes only by a new number, decoded beside the old one.
pub const LAYOUT_V1: u8 = 1;

/// The most symbols a version-15 code may have (hyphens not counted): far above the longest answer code (an answer that
/// moves every value of the starter and every macro, and switches every module on, is 583 symbols).
pub const MAX_SYMBOLS: usize = 4096;

/// Symbols in a scorecard with a short challenge, check symbols and code v1's symbol included.
pub const SCORECARD_SYMBOLS: usize = 51;

/// Header bits: form 1, version 4, kind 5, layout 4.
pub(crate) const HEADER_BITS: usize = 14;

/// Bits of the scorecard's and the ear result's own check.
const CHECK_BITS: u32 = 30;
/// Its symbols.
const CHECK_SYMBOLS: usize = 6;
/// The check's preimage starts with this magic (frozen).
const CHECK_MAGIC: &[u8] = b"enth-v15-check-v1";

/// Bits of a short challenge's payload (code v1's short code without its form and version).
const SHORT_CHALLENGE_BITS: usize = 8 + 12 + 40;
/// Bits of a long challenge's payload before its amounts.
const LONG_CHALLENGE_FIXED_BITS: usize = 16 + 64 + 7 + 7 + 8 + 24 + 32 + 6;
/// The scorecard's fields after the challenge: engine, scorer set, sound, combined, passed, submits, time, assists,
/// commitment.
const SCORECARD_TAIL_BITS: usize = 24 + 8 + 10 + 10 + 1 + 7 + 16 + 8 + 60;

/// The most tenths a claimed percentage holds (100.0 %).
pub const TENTHS_MAX: u16 = 1000;
/// The most submits a scorecard says ("127 or more").
pub const SUBMITS_MAX: u8 = 127;
/// The most hints a code says ("15 or more").
pub const HINTS_MAX: u8 = 15;
/// The longest time a code says, in deciseconds ("1:49:13.5 or more").
pub const TIME_DS_MAX: u16 = u16::MAX;

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

/// Why a version-15 code is refused, or a result cannot be encoded. Every variant has a stable
/// [`code`](ResultError::code) and a [`hint`](ResultError::hint).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResultError {
    /// Nothing but hyphens and spaces.
    Empty,
    /// A character that is not a code symbol.
    BadSymbol {
        /// The character.
        symbol: char,
        /// Its position in the text, counting characters from 0.
        position: usize,
    },
    /// Not a length this kind can have (too short, too long, or not the fixed length).
    BadLength {
        /// The symbols found, hyphens and spaces not counted.
        symbols: usize,
    },
    /// Code v1's check symbol or the kind's own check does not match: a typo, or part of the code is missing.
    BadCheck,
    /// Another kind of code than the one asked for: `found` is `None` for a challenge code.
    WrongKind {
        /// The kind found (`None`: a challenge code, not version 15).
        found: Option<u8>,
        /// The kind asked for.
        expected: u8,
    },
    /// A kind, layout or form this build does not know: a newer build made it (`code_too_new`).
    TooNew {
        /// What is unknown: `kind`, `layout` or `form`.
        what: &'static str,
        /// Its value.
        value: u64,
    },
    /// A field holds a value no encoder writes.
    BadField {
        /// The field.
        field: &'static str,
        /// Its value.
        value: u64,
    },
    /// The challenge inside is refused: a newer generator, preset or template (`code_too_new`), an out-of-range spec, or
    /// a long code whose target this build regenerates differently (`target_mismatch`).
    Challenge(CodeError),
    /// The ear challenge inside is refused (the ear-challenge code, kind 3): a newer game, generator or level
    /// (`code_too_new`), or a field out of range ([`EarError::code`]).
    Ear(EarError),
    /// The answer holds an op, a module kind, a parameter or a source this build does not have (`answer_too_new`).
    AnswerTooNew {
        /// What is unknown: `op`, `module kind`, `parameter` or `source`.
        what: &'static str,
        /// Its value.
        value: u64,
    },
    /// The answer does not rebuild to the patch its tail names (`answer_mismatch`): the code is damaged, or this build
    /// regenerates another start or target from its challenge. The round is never judged on it.
    AnswerMismatch,
    /// This result cannot be put in a code (`unencodable`); the reason says why.
    Unencodable {
        /// Why.
        reason: String,
    },
}

impl ResultError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            ResultError::Empty => "empty_code",
            ResultError::BadSymbol { .. } => "bad_symbol",
            ResultError::BadLength { .. } => "bad_length",
            ResultError::BadCheck => "bad_check",
            ResultError::WrongKind { .. } => "wrong_kind",
            ResultError::TooNew { .. } => "code_too_new",
            ResultError::BadField { .. } => "bad_field",
            ResultError::Challenge(e) => e.code(),
            ResultError::Ear(e) => e.code(),
            ResultError::AnswerTooNew { .. } => "answer_too_new",
            ResultError::AnswerMismatch => "answer_mismatch",
            ResultError::Unencodable { .. } => "unencodable",
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            ResultError::Empty => "paste a code: a challenge, a scorecard or an answer".to_owned(),
            ResultError::BadSymbol { .. } => {
                "codes use 0-9 and A-Z without I, L, O and U: check for a typo".to_owned()
            }
            ResultError::BadLength { .. } => {
                "part of the code is missing or extra: copy the whole code again".to_owned()
            }
            ResultError::BadCheck => {
                "the code's check does not match: there is a typo, or part of it is missing".to_owned()
            }
            ResultError::WrongKind { found, expected } => {
                format!("this is {}, not {}", kind_name(*found), kind_name(Some(*expected)))
            }
            ResultError::TooNew { .. } => "made by a newer enth: update enth to read it".to_owned(),
            ResultError::BadField { .. } => {
                "the code is damaged, or was not made by enth".to_owned()
            }
            ResultError::Challenge(e) => e.hint(),
            ResultError::Ear(e) => e.hint(),
            ResultError::AnswerTooNew { .. } => {
                "the answer uses something a newer enth has (a module or a control): update enth to check it".to_owned()
            }
            ResultError::AnswerMismatch => "the answer does not rebuild on this enth: the code is damaged (copy it \
                                            again), or this enth regenerates the challenge differently (please \
                                            report it with the code)"
                .to_owned(),
            ResultError::Unencodable { .. } => "this result cannot be shared as a code".to_owned(),
        }
    }
}

/// "a scorecard", "a challenge code", for messages.
fn kind_name(kind: Option<u8>) -> &'static str {
    match kind {
        None => "a challenge code",
        Some(KIND_SCORECARD) => "a scorecard",
        Some(KIND_ANSWER) => "an answer code",
        Some(KIND_EAR_CHALLENGE) => "an ear challenge",
        Some(KIND_EAR_RESULT) => "an ear result",
        Some(_) => "another kind of code",
    }
}

impl fmt::Display for ResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResultError::Empty => f.write_str("the code is empty"),
            ResultError::BadSymbol { symbol, position } => {
                write!(f, "{symbol:?} at position {position} is not a code symbol")
            }
            ResultError::BadLength { symbols } => {
                write!(f, "{symbols} symbols is not this code's length")
            }
            ResultError::BadCheck => f.write_str("the check does not match"),
            ResultError::WrongKind { found, expected } => write!(
                f,
                "{} where {} was expected",
                kind_name(*found),
                kind_name(Some(*expected))
            ),
            ResultError::TooNew { what, value } => {
                write!(f, "{what} {value} is newer than this build's")
            }
            ResultError::BadField { field, value } => write!(f, "{field} {value} is not valid"),
            ResultError::Challenge(e) => write!(f, "the challenge: {e}"),
            ResultError::Ear(e) => write!(f, "the ear challenge: {e}"),
            ResultError::AnswerTooNew { what, value } => {
                write!(f, "the answer's {what} {value} is newer than this build's")
            }
            ResultError::AnswerMismatch => {
                f.write_str("the answer does not rebuild to the patch its check names")
            }
            ResultError::Unencodable { reason } => write!(f, "cannot encode: {reason}"),
        }
    }
}

impl std::error::Error for ResultError {}

impl From<CodeError> for ResultError {
    fn from(e: CodeError) -> Self {
        ResultError::Challenge(e)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Bits and the frame
// ---------------------------------------------------------------------------------------------------------------

/// Writes bits most significant first into 5-bit symbols; the last symbol's unused low bits stay zero.
#[derive(Clone, Debug, Default)]
pub(crate) struct Bits {
    syms: Vec<u8>,
    len: usize,
}

impl Bits {
    /// A writer that starts with the header of `kind` at `layout`.
    pub(crate) fn header(kind: u8, layout: u8) -> Bits {
        let mut w = Bits::default();
        w.put(0, 1);
        w.put(u64::from(CODE_VERSION_ESCAPE), 4);
        w.put(u64::from(kind), 5);
        w.put(u64::from(layout), 4);
        w
    }

    /// The low `width` bits of `value`, high bit first.
    pub(crate) fn put(&mut self, value: u64, width: u32) {
        for i in (0..width).rev() {
            let bit = ((value >> i) & 1) as u8;
            let at = self.len % 5;
            if at == 0 {
                self.syms.push(0);
            }
            if let Some(last) = self.syms.last_mut() {
                *last |= bit << (4 - at);
            }
            self.len += 1;
        }
    }

    /// One bit.
    pub(crate) fn put_bool(&mut self, b: bool) {
        self.put(u64::from(b), 1);
    }

    /// The bits written so far.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The symbols, the last one zero-padded.
    pub(crate) fn into_symbols(self) -> Vec<u8> {
        self.syms
    }
}

/// Reads bits most significant first from 5-bit symbols.
pub(crate) struct Reader<'a> {
    syms: &'a [u8],
    bit: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `syms`, starting at bit `bit`.
    pub(crate) fn at(syms: &'a [u8], bit: usize) -> Reader<'a> {
        Reader { syms, bit }
    }

    /// The next `width` bits (at most 64) as a number; `bad_length` past the end.
    pub(crate) fn take(&mut self, width: usize) -> Result<u64, ResultError> {
        let mut v = 0u64;
        for _ in 0..width {
            let sym = self.syms.get(self.bit / 5).ok_or(ResultError::BadLength {
                symbols: self.syms.len(),
            })?;
            v = v << 1 | u64::from((sym >> (4 - self.bit % 5)) & 1);
            self.bit += 1;
        }
        Ok(v)
    }

    /// One bit.
    pub(crate) fn bool(&mut self) -> Result<bool, ResultError> {
        Ok(self.take(1)? == 1)
    }

    /// The bits left.
    pub(crate) fn remaining(&self) -> usize {
        (self.syms.len() * 5).saturating_sub(self.bit)
    }

    /// The end of the fields: what is left must be zero bits within the last symbol (`bad_length` for a whole symbol
    /// or more, `bad_field` for a set padding bit).
    pub(crate) fn finish(&mut self) -> Result<(), ResultError> {
        let left = self.remaining();
        if left >= 5 {
            return Err(ResultError::BadLength {
                symbols: self.syms.len(),
            });
        }
        let pad = self.take(left)?;
        if pad == 0 {
            Ok(())
        } else {
            Err(ResultError::BadField {
                field: "padding",
                value: pad,
            })
        }
    }
}

/// A version-15 code's symbols with code v1's check symbol checked and taken off, and its header read.
#[derive(Clone, Debug)]
pub(crate) struct Framed {
    /// The kind.
    pub(crate) kind: u8,
    /// The layout.
    pub(crate) layout: u8,
    /// The symbols before code v1's check symbol: the header, the fields and the kind's own check.
    pub(crate) data: Vec<u8>,
}

/// The canonical text of `data` (the header, the fields and the kind's own check): code v1's check symbol appended,
/// upper case, in groups of four joined by hyphens.
pub(crate) fn text(mut data: Vec<u8>) -> String {
    data.push(code::check_value(&data));
    let mut out = String::with_capacity(data.len() + data.len() / 4);
    for (i, &v) in data.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(char::from(code::ALPHABET[usize::from(v)]));
    }
    out
}

/// Read the symbols of a version-15 code of any kind (case, Crockford's aliases, hyphens and whitespace as code v1
/// reads them), check code v1's symbol and read the header. Checks, in order: symbols (`bad_symbol`), at least the
/// header and at most [`MAX_SYMBOLS`] (`bad_length`), code v1's symbol (`bad_check`), the version (`wrong_kind`
/// with `found: None` for a challenge code), the form (`code_too_new`), kind 0 and layout 0 (`bad_field`). `expected`
/// only labels a challenge code's error.
pub(crate) fn unframe_any(text: &str, expected: u8) -> Result<Framed, ResultError> {
    let mut syms: Vec<u8> = Vec::with_capacity(text.len().min(MAX_SYMBOLS));
    let mut count = 0usize;
    for (position, c) in text.chars().enumerate() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        count += 1;
        if count > MAX_SYMBOLS {
            continue;
        }
        syms.push(code::symbol_value(c).ok_or(ResultError::BadSymbol {
            symbol: c,
            position,
        })?);
    }
    if count == 0 {
        return Err(ResultError::Empty);
    }
    // The header (3 symbols) and code v1's symbol.
    if !(4..=MAX_SYMBOLS).contains(&count) {
        return Err(ResultError::BadLength { symbols: count });
    }
    if code::at_alpha(&syms) != 0 {
        return Err(ResultError::BadCheck);
    }
    syms.pop();
    let first = syms[0];
    if first & 0b0_1111 != CODE_VERSION_ESCAPE {
        return Err(ResultError::WrongKind {
            found: None,
            expected,
        });
    }
    if first & 0b1_0000 != 0 {
        return Err(ResultError::TooNew {
            what: "form",
            value: 1,
        });
    }
    let mut r = Reader::at(&syms, 5);
    let kind = r.take(5)? as u8;
    let layout = r.take(4)? as u8;
    if kind == 0 {
        return Err(ResultError::BadField {
            field: "kind",
            value: 0,
        });
    }
    if layout == 0 {
        return Err(ResultError::BadField {
            field: "layout",
            value: 0,
        });
    }
    Ok(Framed {
        kind,
        layout,
        data: syms,
    })
}

/// [`unframe_any`], then the kind: an unknown kind is `code_too_new`, another known kind `wrong_kind`, and a layout
/// above `latest` `code_too_new`.
pub(crate) fn unframe(text: &str, expected: u8, latest: u8) -> Result<Framed, ResultError> {
    let f = unframe_any(text, expected)?;
    if f.kind != expected {
        return Err(if KINDS.contains(&f.kind) {
            ResultError::WrongKind {
                found: Some(f.kind),
                expected,
            }
        } else {
            ResultError::TooNew {
                what: "kind",
                value: u64::from(f.kind),
            }
        });
    }
    if f.layout > latest {
        return Err(ResultError::TooNew {
            what: "layout",
            value: u64::from(f.layout),
        });
    }
    Ok(f)
}

/// The 30-bit check of a scorecard's or an ear result's data symbols.
fn check30(data: &[u8]) -> u64 {
    let digest = Sha256::new()
        .chain_update(CHECK_MAGIC)
        .chain_update(data)
        .finalize();
    let top = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    top >> (64 - CHECK_BITS)
}

/// The data symbols with their 30-bit check appended.
fn with_check30(data: Vec<u8>) -> Vec<u8> {
    let check = check30(&data);
    let mut w = Bits {
        len: data.len() * 5,
        syms: data,
    };
    w.put(check, CHECK_BITS);
    w.into_symbols()
}

/// Check and take off the 30-bit check (`bad_check`).
fn strip_check30(data: &[u8]) -> Result<&[u8], ResultError> {
    let n = data
        .len()
        .checked_sub(CHECK_SYMBOLS)
        .ok_or(ResultError::BadLength {
            symbols: data.len() + 1,
        })?;
    let (body, check) = data.split_at(n);
    let found = Reader::at(check, 0).take(CHECK_BITS as usize)?;
    if found == check30(body) {
        Ok(body)
    } else {
        Err(ResultError::BadCheck)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shared fields: the challenge, the engine, an f32
// ---------------------------------------------------------------------------------------------------------------

/// Write a challenge code's payload: a flag (0 short, 1 long), then code v1's fields without its form and version.
pub(crate) fn put_challenge(w: &mut Bits, c: &ChallengeCode) {
    let s = c.spec();
    if let (CodeForm::Long, Some(engine), Some(target)) = (c.form(), c.engine(), c.target()) {
        w.put(1, 1);
        w.put(u64::from(s.generator), 16);
        w.put(s.seed, 64);
        w.put(u64::from(s.difficulty), 7);
        w.put(u64::from(s.musical), 7);
        w.put(u64::from(s.template), 8);
        put_engine(w, engine);
        w.put(u64::from(target.0), 32);
        let mask = s
            .amounts
            .iter()
            .fold(0u64, |m, a| m << 1 | u64::from(a.is_some()));
        w.put(mask, Category::COUNT as u32);
        for a in s.amounts.iter().flatten() {
            w.put(u64::from(*a), 7);
        }
    } else {
        // A short code always has a preset (`ChallengeCode::short` admits no other spec).
        w.put(0, 1);
        w.put(u64::from(s.preset().map_or(0, |p| p.id)), 8);
        w.put(u64::from(s.generator), 12);
        w.put(s.seed, 40);
    }
}

/// Read what [`put_challenge`] wrote, checking the spec as code v1 does (this build's generators, presets and
/// templates).
pub(crate) fn take_challenge(r: &mut Reader<'_>) -> Result<ChallengeCode, ResultError> {
    if r.bool()? {
        let generator = r.take(16)? as u16;
        let seed = r.take(64)?;
        let difficulty = r.take(7)? as u8;
        let musical = r.take(7)? as u8;
        let template = r.take(8)? as u8;
        let engine = take_engine(r)?;
        let target = TargetPrefix(r.take(32)? as u32);
        let mask = r.take(Category::COUNT)?;
        let mut amounts = [None; Category::COUNT];
        for (i, a) in amounts.iter_mut().enumerate() {
            if mask >> (Category::COUNT - 1 - i) & 1 == 1 {
                *a = Some(r.take(7)? as u8);
            }
        }
        let spec = RandomSpec {
            generator,
            seed,
            difficulty,
            amounts,
            musical,
            template,
        };
        Ok(ChallengeCode::long(spec, engine, target).map_err(CodeError::Spec)?)
    } else {
        let preset = r.take(8)? as u8;
        let generator = r.take(12)? as u16;
        let seed = r.take(40)?;
        if preset == 0 {
            return Err(ResultError::BadField {
                field: "preset",
                value: 0,
            });
        }
        let preset = SpecPreset::by_id(preset).ok_or(CodeError::PresetTooNew { preset })?;
        let spec = preset.spec(generator, seed);
        spec.validate().map_err(CodeError::Spec)?;
        ChallengeCode::short(spec).ok_or(ResultError::BadField {
            field: "challenge",
            value: u64::from(preset.id),
        })
    }
}

/// The long challenge's amounts mask, read without moving `r`: what fixes a long-challenge code's length.
fn peek_long_mask(data: &[u8], challenge_at: usize) -> Result<u64, ResultError> {
    let mut r = Reader::at(data, challenge_at + 1 + LONG_CHALLENGE_FIXED_BITS - 6);
    r.take(6)
}

/// An engine version: three bytes.
pub(crate) fn put_engine(w: &mut Bits, e: EngineVersion) {
    w.put(u64::from(e.major), 8);
    w.put(u64::from(e.minor), 8);
    w.put(u64::from(e.patch), 8);
}

/// Read an engine version.
pub(crate) fn take_engine(r: &mut Reader<'_>) -> Result<EngineVersion, ResultError> {
    Ok(EngineVersion {
        major: r.take(8)? as u8,
        minor: r.take(8)? as u8,
        patch: r.take(8)? as u8,
    })
}

/// Exact powers of ten in `f64`, 10⁰ to 10⁸.
/// An ear spec as kind 3 writes it, and kind 4 inside it: game 8 | generator 8 | level 8 | seed 40 | count 8, the
/// integers of `enth-music`'s [`EarSpec`] in its order (the seed below 2⁴⁰: the caller checks it).
pub(crate) fn put_ear_spec(w: &mut Bits, spec: &EarSpec) {
    w.put(u64::from(spec.game), 8);
    w.put(u64::from(spec.generator), 8);
    w.put(u64::from(spec.level), 8);
    w.put(spec.seed, EAR_SEED_BITS);
    w.put(u64::from(spec.count), 8);
}

/// The ear spec [`put_ear_spec`] wrote (its ranges are the caller's to check).
pub(crate) fn take_ear_spec(r: &mut Reader<'_>) -> Result<EarSpec, ResultError> {
    Ok(EarSpec {
        game: r.take(8)? as u8,
        generator: r.take(8)? as u8,
        level: r.take(8)? as u8,
        seed: r.take(EAR_SEED_BITS as usize)?,
        count: r.take(8)? as u8,
    })
}

const POW10: [f64; 9] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8];

/// The decimal form's value: `±m · 10^e`, one correctly rounded `f64` operation then the cast to `f32`, so it is the
/// same on every IEEE 754 platform (wasm32 included).
fn decimal(negative: bool, m: u64, e: i32) -> f32 {
    let p = POW10[e.unsigned_abs() as usize];
    let x = if e >= 0 { m as f64 * p } else { m as f64 / p };
    let v = x as f32;
    if negative { -v } else { v }
}

/// Write an `f32` exactly, in the **float coding** (frozen with layout 1): `0`, then a sign bit, a 10-bit mantissa m
/// and a 4-bit exponent e + 8 (the value is ±m·10^e with e from −8 to 7), when that gives the same bits (16 bits: a
/// value typed or quantized to three significant digits); otherwise `1` and the 32 raw bits (33 bits). `-0.0` is
/// written as `0.0`, as patches store it.
pub(crate) fn put_f32(w: &mut Bits, v: f32) {
    let v = if v == 0.0 { 0.0 } else { v };
    if v.is_finite() {
        let negative = v.is_sign_negative();
        let a = f64::from(v.abs());
        for e in -8i32..=7 {
            let p = POW10[e.unsigned_abs() as usize];
            let m = if e >= 0 { a / p } else { a * p }.round();
            if m <= 1023.0 && decimal(negative, m as u64, e).to_bits() == v.to_bits() {
                w.put(0, 1);
                w.put_bool(negative);
                w.put(m as u64, 10);
                w.put((e + 8) as u64, 4);
                return;
            }
        }
    }
    w.put(1, 1);
    w.put(u64::from(v.to_bits()), 32);
}

/// Read an `f32` written by [`put_f32`]. A decimal `-0` reads as `0.0`; raw bits are returned as they are (the caller
/// checks them against the parameter).
pub(crate) fn take_f32(r: &mut Reader<'_>) -> Result<f32, ResultError> {
    if r.bool()? {
        Ok(f32::from_bits(r.take(32)? as u32))
    } else {
        let negative = r.bool()?;
        let m = r.take(10)?;
        let e = r.take(4)? as i32 - 8;
        let v = decimal(negative, m, e);
        Ok(if v == 0.0 { 0.0 } else { v })
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Scorer sets
// ---------------------------------------------------------------------------------------------------------------

/// The frozen, append-only table of scorer sets a scorecard names by index (index − 1 here; index 0 is never written).
/// A set fixes the four scorer ids **and** the Recreate weight and pass-mark rule ([`ScoringSpec::with_scorers`]):
/// GR1's `sound/2`, or a new pass mark, is a new set; an old index never changes its meaning.
const SCORER_SETS: [[ScorerId; 4]; 1] = [[
    ScorerId::SOUND_V1,
    ScorerId::STRUCTURE_V1,
    ScorerId::COMBINED_V1,
    ScorerId::TIME_V1,
]];

/// A scorer set: an index into a frozen table of (sound, structure, combined, time) scorer ids with the Recreate
/// weight and pass-mark rule. What a scorecard's claim was scored with, and what [`crate::verify`] re-scores with.
///
/// **It may name a set this build does not have** (the F1e review's M4): a scorecard from a newer build (GR1's
/// `sound/2`, say) still reads, [`is_known`](Self::is_known) is false, and [`ids`](Self::ids) and
/// [`scoring`](Self::scoring) are `None`, so the card shows the claim as reported and the re-score uses this build's
/// [`current`](Self::current) set, flagged ([`crate::verify::scorer_set`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScorerSet(u8);

impl ScorerSet {
    /// Set 1: `sound/1`, `structure/1`, `combined/1`, `time/1` (R1's scorers).
    pub const V1: ScorerSet = ScorerSet(1);

    /// The set this build scores Recreate rounds with ([`RECREATE_SCORERS`]).
    pub fn current() -> ScorerSet {
        SCORER_SETS
            .iter()
            .position(|ids| *ids == RECREATE_SCORERS)
            .map_or(ScorerSet::V1, |i| ScorerSet(i as u8 + 1))
    }

    /// The set with this index, known to this build or not; `None` for 0, which no code writes.
    pub fn from_index(index: u8) -> Option<ScorerSet> {
        (index != 0).then_some(ScorerSet(index))
    }

    /// Whether this build has the set (an older build reads a newer build's index, and cannot score with it).
    pub fn is_known(self) -> bool {
        (1..=SCORER_SETS.len()).contains(&usize::from(self.0))
    }

    /// The index a code carries.
    pub fn index(self) -> u8 {
        self.0
    }

    /// The four ids: sound, structure, combined, time; `None` for a set this build does not have.
    pub fn ids(self) -> Option<[ScorerId; 4]> {
        self.is_known()
            .then(|| SCORER_SETS[usize::from(self.0 - 1)].clone())
    }

    /// The scoring a round at `difficulty` uses under this set; `None` for a set this build does not have.
    pub fn scoring(self, difficulty: u8) -> Option<ScoringSpec> {
        let [sound, structure, combined, time] = self.ids()?;
        Some(ScoringSpec::with_scorers(
            difficulty, sound, structure, combined, time,
        ))
    }

    /// The set whose scoring at `difficulty` is exactly `scoring`, if the table has one.
    pub fn of(scoring: &ScoringSpec, difficulty: u8) -> Option<ScorerSet> {
        (1..=SCORER_SETS.len() as u8)
            .map(ScorerSet)
            .find(|s| s.scoring(difficulty).as_ref() == Some(scoring))
    }
}

impl fmt::Display for ScorerSet {
    /// `sound/1 + structure/1 + combined/1 + time/1`, or `scorer set 2 (a newer enth's)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ids() {
            Some(ids) => write!(f, "{} + {} + {} + {}", ids[0], ids[1], ids[2], ids[3]),
            None => write!(f, "scorer set {} (a newer enth's)", self.0),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Ear rules (kind 4)
// ---------------------------------------------------------------------------------------------------------------

/// One row of [`EAR_RULES`]: the scorer of each non-patch answer kind and the pass mark.
struct EarRulesRow {
    choice: ScorerId,
    value: ScorerId,
    taps: ScorerId,
    pass_mark: u8,
}

/// The frozen, append-only table of ear rules an ear result names by index (index − 1 here; index 0 is never written):
/// what a receiver judges the answers with (should-fix S5). A retuned scorer (`taps/2`) or another pass mark is a new
/// row; an old index never changes its meaning.
const EAR_RULES: [EarRulesRow; 1] = [EarRulesRow {
    choice: ScorerId::EAR_V1,
    value: ScorerId::EAR_VALUE_V1,
    taps: ScorerId::TAPS_V1,
    pass_mark: EAR_PASS_MARK,
}];

/// The ear rules an ear result was judged by: an index into a frozen table of the ear scorers (`ear/1`,
/// `ear-value/1`, `taps/1`) and the pass mark (should-fix S5). Kind 4 names it, so the receiver judges with the
/// sender's rules when it has them, and flags the difference when it does not ([`crate::verify::verify_ear`]). Like
/// [`ScorerSet`], it may name rules this build does not have ([`is_known`](Self::is_known)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EarRules(u8);

impl EarRules {
    /// Rules 1: `ear/1`, `ear-value/1`, `taps/1`, pass at 80 %.
    pub const V1: EarRules = EarRules(1);

    /// The rules this build gives a new ear challenge ([`ScoringSpec::ear`]).
    pub fn current() -> EarRules {
        [AnswerKind::Choice, AnswerKind::Value, AnswerKind::Taps]
            .iter()
            .filter_map(|&a| ScoringSpec::ear(a))
            .find_map(|scoring| EarRules::of(&scoring))
            .unwrap_or(EarRules::V1)
    }

    /// The rules with this index, known to this build or not; `None` for 0, which no code writes.
    pub fn from_index(index: u8) -> Option<EarRules> {
        (index != 0).then_some(EarRules(index))
    }

    /// Whether this build has the rules.
    pub fn is_known(self) -> bool {
        (1..=EAR_RULES.len()).contains(&usize::from(self.0))
    }

    /// The index a code carries.
    pub fn index(self) -> u8 {
        self.0
    }

    /// The scoring of an `answer` under these rules (its scorer in the sound, structure and combined slots, weight 100,
    /// `time/1`, the pass mark); `None` for a patch answer or rules this build does not have.
    pub fn scoring(self, answer: AnswerKind) -> Option<ScoringSpec> {
        if !self.is_known() {
            return None;
        }
        let row = &EAR_RULES[usize::from(self.0 - 1)];
        let id = match answer {
            AnswerKind::Choice => row.choice.clone(),
            AnswerKind::Value => row.value.clone(),
            AnswerKind::Taps => row.taps.clone(),
            AnswerKind::Patch => return None,
        };
        let mut scoring = ScoringSpec::ear(answer)?;
        scoring.sound = id.clone();
        scoring.structure = id.clone();
        scoring.combined = id;
        scoring.pass_mark = row.pass_mark;
        Some(scoring)
    }

    /// The rules whose scoring of `scoring`'s answer kind is exactly `scoring`, if the table has them.
    pub fn of(scoring: &ScoringSpec) -> Option<EarRules> {
        let answer = scorers::answer_of(&scoring.sound)?;
        (1..=EAR_RULES.len() as u8)
            .map(EarRules)
            .find(|r| r.scoring(answer).as_ref() == Some(scoring))
    }
}

impl fmt::Display for EarRules {
    /// `ear/1 + ear-value/1 + taps/1, pass 80 %`, or `ear rules 2 (a newer enth's)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_known() {
            let row = &EAR_RULES[usize::from(self.0 - 1)];
            write!(
                f,
                "{} + {} + {}, pass {} %",
                row.choice, row.value, row.taps, row.pass_mark
            )
        } else {
            write!(f, "ear rules {} (a newer enth's)", self.0)
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The scorecard (kind 1)
// ---------------------------------------------------------------------------------------------------------------

/// A salted commitment to an answer: 60 bits of SHA-256 over the salt, the target and the answer (computed by
/// [`crate::answer::AnswerCode::commitment`]). A scorecard carries it, so the answer code sent later can be shown to
/// be the one fixed when the scorecard was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Commitment(u64);

impl Commitment {
    /// Its bits.
    pub const BITS: u32 = 60;

    /// A commitment from its 60 bits; `None` when a higher bit is set.
    pub fn from_bits(bits: u64) -> Option<Commitment> {
        (bits >> Self::BITS == 0).then_some(Commitment(bits))
    }

    /// The 60 bits.
    pub fn bits(self) -> u64 {
        self.0
    }
}

/// The help a round used, as a scorecard says it ([`AssistsUsed`], with the hints saturating at [`HINTS_MAX`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent assists, each one flag in the code"
)]
pub struct CardAssists {
    /// Hints revealed, at most [`HINTS_MAX`] ("15 or more").
    pub hints: u8,
    /// The live meter was on at some time.
    pub live_meter: bool,
    /// "Dim unused" was on at some time.
    pub dim_unused: bool,
    /// "How close am I?" answered with a passing patch's moves.
    pub closest: bool,
    /// The target was revealed (only after a pass: a reveal before it gives no scorecard).
    pub revealed: bool,
}

impl CardAssists {
    /// What a round used, the hints saturating.
    pub fn of(used: &AssistsUsed) -> CardAssists {
        CardAssists {
            hints: used.hints.min(HINTS_MAX),
            live_meter: used.live_meter,
            dim_unused: used.dim_unused,
            closest: used.closest,
            revealed: used.revealed,
        }
    }

    /// Whether any assist was used.
    pub fn any(&self) -> bool {
        self.hints > 0 || self.live_meter || self.dim_unused || self.closest || self.revealed
    }
}

/// What a scorecard says. Build a [`Scorecard`] from it with [`Scorecard::from_fields`] (which checks it), or from a
/// round with [`Scorecard::new`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScorecardFields {
    /// The challenge played.
    pub challenge: ChallengeCode,
    /// The engine of the build that scored the claim.
    pub engine: EngineVersion,
    /// The scorers the claim was scored with.
    pub scorers: ScorerSet,
    /// The claimed sound score in tenths of a percent, 0..=[`TENTHS_MAX`].
    pub sound_tenths: u16,
    /// The claimed combined score in tenths of a percent.
    pub combined_tenths: u16,
    /// Whether the compared submit passed (the first passing submit; otherwise the best, not passed).
    pub passed: bool,
    /// Submits up to the compared one, 1..=[`SUBMITS_MAX`] ("127 or more").
    pub submits: u8,
    /// Deciseconds: the time to pass, or the round's active time when it did not pass; [`TIME_DS_MAX`] means "at
    /// least that". Reported by the sender's app, never checked.
    pub time_ds: u16,
    /// The help used.
    pub assists: CardAssists,
    /// The salted commitment to the answer.
    pub commitment: Commitment,
}

/// The **scorecard** (kind 1): a spoiler-free result sent on finishing. See the [module docs](self) for the layout.
/// Its numbers are claims: the receiver re-scores the answer ([`crate::verify`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scorecard {
    fields: ScorecardFields,
}

impl Scorecard {
    /// The scorecard of a round: `challenge`, the compared submit's `report` scored with `scoring`, the `submits` up
    /// to it, the time in editor-visible milliseconds (to the pass, or the round's active time without one), the
    /// `assists` used and the answer's `commitment`. Refused (`unencodable`) when no frozen scorer set matches
    /// `scoring`, the report is not one `scoring` could have made, there was no submit, or the target was revealed
    /// before a pass.
    pub fn new(
        challenge: ChallengeCode,
        report: &ScoreReport,
        scoring: &ScoringSpec,
        submits: u32,
        time_ms: u64,
        assists: &AssistsUsed,
        commitment: Commitment,
    ) -> Result<Scorecard, ResultError> {
        let scorers = ScorerSet::of(scoring, challenge.spec().difficulty).ok_or_else(|| {
            ResultError::Unencodable {
                reason: "the round's scorers are not a frozen scorer set".to_owned(),
            }
        })?;
        report
            .check_against(scoring)
            .map_err(|reason| ResultError::Unencodable { reason })?;
        Scorecard::from_fields(ScorecardFields {
            challenge,
            engine: report.engine.unwrap_or(EngineVersion::CURRENT),
            scorers,
            sound_tenths: tenths(report.sound.percent),
            combined_tenths: tenths(report.combined.percent),
            passed: report.passed,
            submits: submits.min(u32::from(SUBMITS_MAX)) as u8,
            time_ds: ((time_ms.saturating_add(50)) / 100).min(u64::from(TIME_DS_MAX)) as u16,
            assists: CardAssists::of(assists),
            commitment,
        })
    }

    /// A scorecard from its fields, checked: percentages at most [`TENTHS_MAX`], 1 to [`SUBMITS_MAX`] submits, at most
    /// [`HINTS_MAX`] hints, and no reveal without a pass (`unencodable`).
    pub fn from_fields(fields: ScorecardFields) -> Result<Scorecard, ResultError> {
        let bad = |reason: &str| {
            Err(ResultError::Unencodable {
                reason: reason.to_owned(),
            })
        };
        if fields.sound_tenths > TENTHS_MAX || fields.combined_tenths > TENTHS_MAX {
            return bad("a percentage above 100");
        }
        if fields.submits == 0 || fields.submits > SUBMITS_MAX {
            return bad("a scorecard has 1 to 127 submits");
        }
        if fields.assists.hints > HINTS_MAX {
            return bad("at most 15 hints");
        }
        if fields.assists.revealed && !fields.passed {
            return bad("a revealed target gives no scorecard");
        }
        Ok(Scorecard { fields })
    }

    /// What it says.
    pub fn fields(&self) -> &ScorecardFields {
        &self.fields
    }

    /// The challenge played.
    pub fn challenge(&self) -> &ChallengeCode {
        &self.fields.challenge
    }

    /// The claimed sound %, from its tenths.
    pub fn sound_percent(&self) -> f64 {
        f64::from(self.fields.sound_tenths) / 10.0
    }

    /// The claimed combined %, from its tenths.
    pub fn combined_percent(&self) -> f64 {
        f64::from(self.fields.combined_tenths) / 10.0
    }

    /// The canonical text (51 symbols with a short challenge, in groups of four).
    pub fn encode(&self) -> String {
        let f = &self.fields;
        let mut w = Bits::header(KIND_SCORECARD, LAYOUT_V1);
        put_challenge(&mut w, &f.challenge);
        put_engine(&mut w, f.engine);
        w.put(u64::from(f.scorers.index()), 8);
        w.put(u64::from(f.sound_tenths), 10);
        w.put(u64::from(f.combined_tenths), 10);
        w.put_bool(f.passed);
        w.put(u64::from(f.submits), 7);
        w.put(u64::from(f.time_ds), 16);
        w.put(u64::from(f.assists.hints), 4);
        w.put_bool(f.assists.live_meter);
        w.put_bool(f.assists.dim_unused);
        w.put_bool(f.assists.closest);
        w.put_bool(f.assists.revealed);
        w.put(f.commitment.bits(), Commitment::BITS);
        text(with_check30(w.into_symbols()))
    }

    /// Read a scorecard. Checks, in order: the frame ([`unframe`]: symbols, length bounds, code v1's symbol, version,
    /// form, kind, layout), the length the challenge's form fixes (`bad_length`), the 30-bit check (`bad_check`), the
    /// challenge (as code v1 checks it), then each field (`bad_field`; a scorer set of 0). A scorer set this build does
    /// not have reads ([`ScorerSet::is_known`] false): the card is shown as reported (the F1e review's M4). Never
    /// panics.
    pub fn decode(text: &str) -> Result<Scorecard, ResultError> {
        let f = unframe(text, KIND_SCORECARD, LAYOUT_V1)?;
        let data = &f.data;
        let symbols = data.len() + 1;
        // The length the challenge's form fixes.
        let long = Reader::at(data, HEADER_BITS).bool()?;
        let challenge_bits = if long {
            let mask = peek_long_mask(data, HEADER_BITS)?;
            1 + LONG_CHALLENGE_FIXED_BITS + 7 * mask.count_ones() as usize
        } else {
            1 + SHORT_CHALLENGE_BITS
        };
        let want = (HEADER_BITS + challenge_bits + SCORECARD_TAIL_BITS).div_ceil(5) + CHECK_SYMBOLS;
        if data.len() != want {
            return Err(ResultError::BadLength { symbols });
        }
        let body = strip_check30(data)?;
        let mut r = Reader::at(body, HEADER_BITS);
        let challenge = take_challenge(&mut r)?;
        let engine = take_engine(&mut r)?;
        let set = r.take(8)? as u8;
        // Any non-zero set reads, known or not (the F1e review's M4): a newer build's card is shown as reported.
        let scorers = ScorerSet::from_index(set).ok_or(ResultError::BadField {
            field: "scorer set",
            value: 0,
        })?;
        let sound_tenths = r.take(10)? as u16;
        let combined_tenths = r.take(10)? as u16;
        let passed = r.bool()?;
        let submits = r.take(7)? as u8;
        let time_ds = r.take(16)? as u16;
        let assists = CardAssists {
            hints: r.take(4)? as u8,
            live_meter: r.bool()?,
            dim_unused: r.bool()?,
            closest: r.bool()?,
            revealed: r.bool()?,
        };
        let commitment = Commitment(r.take(Commitment::BITS as usize)?);
        r.finish()?;
        for (field, value) in [("sound", sound_tenths), ("combined", combined_tenths)] {
            if value > TENTHS_MAX {
                return Err(ResultError::BadField {
                    field,
                    value: u64::from(value),
                });
            }
        }
        if submits == 0 {
            return Err(ResultError::BadField {
                field: "submits",
                value: 0,
            });
        }
        if assists.revealed && !passed {
            return Err(ResultError::BadField {
                field: "revealed",
                value: 1,
            });
        }
        Ok(Scorecard {
            fields: ScorecardFields {
                challenge,
                engine,
                scorers,
                sound_tenths,
                combined_tenths,
                passed,
                submits,
                time_ds,
                assists,
                commitment,
            },
        })
    }

    /// The share message's human line, as docs/duels.md words it ("enth duel · Level 2 · 1080-1000-0001-AB" and "93.4 %
    /// in 2:41 · 4 submits · 0 hints"); the shell adds the scorecard itself on the next line. Every claim on it is the
    /// sender's app's.
    pub fn share_lines(&self) -> [String; 2] {
        let f = &self.fields;
        let level = f
            .challenge
            .spec()
            .preset()
            .filter(|_| f.challenge.form() == CodeForm::Short)
            .map_or("Custom", |p| p.name);
        let first = format!("enth duel · {level} · {}", f.challenge);
        let mut second = format!("{:.1} %", self.sound_percent());
        if f.passed {
            let _ = write!(second, " in {}", clock(f.time_ds));
        } else {
            let _ = write!(second, ", not passed, {}", clock(f.time_ds));
        }
        let submits = if f.submits == SUBMITS_MAX {
            "127+ submits".to_owned()
        } else if f.submits == 1 {
            "1 submit".to_owned()
        } else {
            format!("{} submits", f.submits)
        };
        let hints = match f.assists.hints {
            1 => "1 hint".to_owned(),
            HINTS_MAX => "15+ hints".to_owned(),
            n => format!("{n} hints"),
        };
        let _ = write!(second, " · {submits} · {hints}");
        for (on, what) in [
            (f.assists.live_meter, "live meter"),
            (f.assists.dim_unused, "dim unused"),
            (f.assists.closest, "how close"),
            (f.assists.revealed, "target revealed after passing"),
        ] {
            if on {
                let _ = write!(second, " · {what}");
            }
        }
        [first, second]
    }
}

impl fmt::Display for Scorecard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// A percentage in tenths, rounded, within 0..=1000.
fn tenths(percent: f64) -> u16 {
    if percent.is_finite() {
        (percent * 10.0).round().clamp(0.0, f64::from(TENTHS_MAX)) as u16
    } else {
        0
    }
}

/// `2:41`, `1:02:03` from deciseconds (whole seconds, rounded down; "+" at the saturated value).
fn clock(ds: u16) -> String {
    let s = u32::from(ds) / 10;
    let plus = if ds == TIME_DS_MAX { "+" } else { "" };
    if s >= 3600 {
        format!("{}:{:02}:{:02}{plus}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}{plus}", s / 60, s % 60)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The ear result (kind 4)
// ---------------------------------------------------------------------------------------------------------------

/// Seed bits in an ear challenge (kind 3's payload, inside kind 4 too).
pub const EAR_SEED_BITS: u32 = 40;
/// The most options a choice answer indexes.
pub const EAR_CHOICES: u8 = 16;
/// The most taps one answer holds: `taps/1`'s own limit ([`scorers::MAX_TAPS`]).
pub const EAR_TAPS_MAX: usize = scorers::MAX_TAPS;
/// The longest span of one tap answer, first tap to last, in ms: `taps/1`'s own limit ([`scorers::MAX_TAP_SPAN_MS`]).
pub const EAR_TAP_MS_MAX: u32 = scorers::MAX_TAP_SPAN_MS;

/// Bits of a stored tap (its time from the first tap).
const EAR_TAP_BITS: u32 = 14;
const _: () = assert!(EAR_TAP_MS_MAX < 1 << EAR_TAP_BITS);
const _: () = assert!(EAR_TAPS_MAX < 1 << 6);

/// The **ear result** (kind 4): the answers to an ear challenge, the rules they are judged by, the time and the
/// assists. See the [module docs](self) for the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct EarResult {
    challenge: EarSpec,
    rules: EarRules,
    answers: Vec<Option<EarAnswer>>,
    time_ds: u16,
    hints: u8,
    revealed: bool,
}

impl EarResult {
    /// An ear result: the challenge's spec, the ear `rules` it was judged by ([`EarRules::current`] for this build's),
    /// one answer (or `None`) per question, the time in editor-visible milliseconds, the hints used (saturating at
    /// [`HINTS_MAX`]) and whether the answers were revealed. **Tap answers may be on any clock**: each is stored relative
    /// to its first tap. Refused (`unencodable`) when the seed is 2⁴⁰ or more, the answers do not match the
    /// challenge's count, or an answer does not fit (a choice of 16 or more, a value that is not finite, taps out of
    /// order, more than [`EAR_TAPS_MAX`] or spanning more than [`EAR_TAP_MS_MAX`] ms).
    pub fn new(
        challenge: EarSpec,
        rules: EarRules,
        answers: Vec<Option<EarAnswer>>,
        time_ms: u64,
        hints: u8,
        revealed: bool,
    ) -> Result<EarResult, ResultError> {
        let bad = |reason: String| Err(ResultError::Unencodable { reason });
        if challenge.seed >> EAR_SEED_BITS != 0 {
            return bad("an ear challenge's seed is below 2^40".to_owned());
        }
        if answers.len() != usize::from(challenge.count) {
            return bad(format!(
                "{} answers to {} questions",
                answers.len(),
                challenge.count
            ));
        }
        let mut kept = Vec::with_capacity(answers.len());
        for (i, a) in answers.into_iter().enumerate() {
            let a = a.map(relative);
            if let Some(reason) = a.as_ref().and_then(answer_problem) {
                return bad(format!("answer {}: {reason}", i + 1));
            }
            kept.push(a);
        }
        Ok(EarResult {
            challenge,
            rules,
            answers: kept,
            time_ds: ((time_ms.saturating_add(50)) / 100).min(u64::from(TIME_DS_MAX)) as u16,
            hints: hints.min(HINTS_MAX),
            revealed,
        })
    }

    /// The ear challenge's spec.
    pub fn challenge(&self) -> &EarSpec {
        &self.challenge
    }

    /// The ear rules the answers were judged by (perhaps a newer build's: [`EarRules::is_known`]).
    pub fn rules(&self) -> EarRules {
        self.rules
    }

    /// The answers, one per question; tap answers relative to their first tap.
    pub fn answers(&self) -> &[Option<EarAnswer>] {
        &self.answers
    }

    /// Deciseconds, saturating at [`TIME_DS_MAX`]; reported, never checked.
    pub fn time_ds(&self) -> u16 {
        self.time_ds
    }

    /// Hints used, at most [`HINTS_MAX`].
    pub fn hints(&self) -> u8 {
        self.hints
    }

    /// Whether the answers were revealed.
    pub fn revealed(&self) -> bool {
        self.revealed
    }

    /// The canonical text.
    pub fn encode(&self) -> String {
        let mut w = Bits::header(KIND_EAR_RESULT, LAYOUT_V1);
        put_ear_spec(&mut w, &self.challenge);
        w.put(u64::from(self.rules.index()), 8);
        for a in &self.answers {
            match a {
                None => w.put(0, 2),
                Some(EarAnswer::Choice(i)) => {
                    w.put(1, 2);
                    w.put(u64::from(*i), 4);
                }
                Some(EarAnswer::Value(v)) => {
                    w.put(2, 2);
                    put_f32(&mut w, *v);
                }
                Some(EarAnswer::Taps(taps)) => {
                    w.put(3, 2);
                    w.put(taps.len() as u64, 6);
                    // The first tap is 0 (stored relative to it), so it is not written.
                    for t in taps.iter().skip(1) {
                        w.put(u64::from(*t), EAR_TAP_BITS);
                    }
                }
            }
        }
        w.put(u64::from(self.time_ds), 16);
        w.put(u64::from(self.hints), 4);
        w.put_bool(self.revealed);
        w.put(0, 3);
        text(with_check30(w.into_symbols()))
    }

    /// Read an ear result. Checks: the frame, the 30-bit check, then each field (`bad_field`: ear rules of 0, an answer
    /// that does not fit, the reserved bits). The spec is not checked against this build: an ear result names what its
    /// sender played, and [`crate::verify::verify_ear`] refuses a spec this build cannot replay. Never panics.
    pub fn decode(text: &str) -> Result<EarResult, ResultError> {
        let f = unframe(text, KIND_EAR_RESULT, LAYOUT_V1)?;
        let body = strip_check30(&f.data)?;
        let mut r = Reader::at(body, HEADER_BITS);
        let challenge = take_ear_spec(&mut r)?;
        let rules = EarRules::from_index(r.take(8)? as u8).ok_or(ResultError::BadField {
            field: "ear rules",
            value: 0,
        })?;
        let mut answers = Vec::with_capacity(usize::from(challenge.count));
        for _ in 0..challenge.count {
            let a = match r.take(2)? {
                0 => None,
                1 => Some(EarAnswer::Choice(r.take(4)? as u8)),
                2 => Some(EarAnswer::Value(take_f32(&mut r)?)),
                _ => {
                    let n = r.take(6)? as usize;
                    let mut taps = Vec::with_capacity(n);
                    if n > 0 {
                        taps.push(0);
                    }
                    for _ in 1..n {
                        taps.push(r.take(EAR_TAP_BITS as usize)? as u32);
                    }
                    Some(EarAnswer::Taps(taps))
                }
            };
            if let Some(a) = &a
                && answer_problem(a).is_some()
            {
                return Err(ResultError::BadField {
                    field: "answer",
                    value: answers.len() as u64,
                });
            }
            answers.push(a);
        }
        let time_ds = r.take(16)? as u16;
        let hints = r.take(4)? as u8;
        let revealed = r.bool()?;
        let reserved = r.take(3)?;
        r.finish()?;
        if reserved != 0 {
            return Err(ResultError::BadField {
                field: "reserved",
                value: reserved,
            });
        }
        Ok(EarResult {
            challenge,
            rules,
            answers,
            time_ds,
            hints,
            revealed,
        })
    }
}

impl fmt::Display for EarResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// A tap answer relative to its first tap (taps on any clock become times from the first); other answers as they are.
/// Taps out of order are left as they are, for [`answer_problem`] to refuse.
fn relative(a: EarAnswer) -> EarAnswer {
    match a {
        EarAnswer::Taps(t) if t.windows(2).all(|p| p[0] <= p[1]) => {
            let first = t.first().copied().unwrap_or(0);
            EarAnswer::Taps(t.into_iter().map(|ms| ms - first).collect())
        }
        other => other,
    }
}

/// What makes an answer unencodable, if anything (taps already relative to their first).
fn answer_problem(a: &EarAnswer) -> Option<&'static str> {
    match a {
        EarAnswer::Choice(i) if *i >= EAR_CHOICES => Some("a choice index is below 16"),
        EarAnswer::Value(v) if !v.is_finite() => Some("a value is finite"),
        EarAnswer::Taps(t) if t.len() > EAR_TAPS_MAX => Some("at most 63 taps"),
        EarAnswer::Taps(t) if t.windows(2).any(|p| p[1] < p[0]) => Some("taps are in time order"),
        EarAnswer::Taps(t) if t.first().is_some_and(|&ms| ms != 0) => {
            Some("taps are stored relative to the first")
        }
        EarAnswer::Taps(t) if t.last().is_some_and(|&ms| ms > EAR_TAP_MS_MAX) => {
            Some("taps span at most 16383 ms")
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests;
