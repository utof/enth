//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e). The
//! reference is docs/duels.md, "Sharing and paste".
//!
//! **Paste-anything.** The plugin and the CLI share it: [`paste`] takes a whole chat message and returns every code in
//! it that decodes (a challenge, a scorecard, an answer, an ear challenge, an ear result), several per paste, in order,
//! each once. No
//! clipboard access here: the shell writes the clipboard on a click and reads it only on an explicit Paste.
//!
//! What a chat app or a phone does to a code, and what this undoes ([`normalize`]):
//!
//! | Damage | Undone by |
//! | --- | --- |
//! | Smart punctuation turns `-` or `--` into a dash (‐ ‑ ‒ – — ― −, and their small and full-width forms) | every dash-like character reads as a hyphen; codes never contain `--` |
//! | Zero-width characters (U+200B–U+200D, U+2060, U+FEFF), soft hyphens, the Mongolian vowel separator | dropped |
//! | Non-breaking and other spaces (U+00A0, U+2007, U+202F, U+3000) | read as a space |
//! | Full-width letters and digits from an East Asian keyboard | read as ASCII |
//! | Markdown and quotes around a code (`**…**`, `_…_`, `` `…` ``, “…”) | they are not code characters, so they end a token |
//! | A line break or spaces inside a long code (wrapping, typing it in groups) | neighbouring tokens of capitals, digits and hyphens are joined when the joined text decodes |
//!
//! A token is a run of ASCII letters, digits and hyphens. Each one is tried alone; a run of tokens made only of
//! capitals, digits and hyphens and separated only by spaces or line breaks is also tried joined, longest first. A
//! code-looking token that decodes as nothing (ten symbols or more with a digit) is returned in
//! [`Pasted::refused`] with its error, so the UI can say "there is a typo" or "update enth".

use core::fmt;

use crate::answer::AnswerCode;
use crate::code::{CODE_VERSION_ESCAPE, ChallengeCode, CodeError};
use crate::ear::code::EarChallengeCode;
use crate::result::{
    EarResult, KIND_ANSWER, KIND_EAR_CHALLENGE, KIND_EAR_RESULT, KIND_SCORECARD, MAX_SYMBOLS,
    ResultError, Scorecard, unframe_any,
};

/// One code found in a paste.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Found {
    /// A challenge code (code v1).
    Challenge(ChallengeCode),
    /// A scorecard (kind 1).
    Scorecard(Scorecard),
    /// An answer code (kind 2), its answer rebuilt (boxed: it holds a patch).
    Answer(Box<AnswerCode>),
    /// An ear result (kind 4).
    EarResult(EarResult),
    /// An ear challenge (kind 3): route it to the Ear view (the F1e review's M5).
    EarChallenge(EarChallengeCode),
}

impl Found {
    /// The code's canonical text.
    pub fn encode(&self) -> String {
        match self {
            Found::Challenge(c) => c.encode(),
            Found::Scorecard(c) => c.encode(),
            Found::Answer(c) => c.encode(),
            Found::EarResult(c) => c.encode(),
            Found::EarChallenge(c) => c.encode(),
        }
    }
}

/// Why a token that looks like a code was refused: a challenge code's error or a version-15 code's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteError {
    /// As a challenge code.
    Challenge(CodeError),
    /// As a version-15 code (a scorecard, an answer, an ear challenge, an ear result, or a kind this build does not
    /// read).
    Result(ResultError),
}

impl PasteError {
    /// A stable snake-case code.
    pub fn code(&self) -> &'static str {
        match self {
            PasteError::Challenge(e) => e.code(),
            PasteError::Result(e) => e.code(),
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            PasteError::Challenge(e) => e.hint(),
            PasteError::Result(e) => e.hint(),
        }
    }
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasteError::Challenge(e) => write!(f, "{e}"),
            PasteError::Result(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PasteError {}

/// A code-looking token that decoded as nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The token, as normalized (tokens joined across spaces are joined without them).
    pub text: String,
    /// Why.
    pub error: PasteError,
}

/// What a paste held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pasted {
    /// Every code that decoded, in order, each once.
    pub found: Vec<Found>,
    /// Code-looking tokens that did not decode.
    pub refused: Vec<Refused>,
}

/// Read one code of any kind: a challenge code, or a version-15 code by its kind. A kind this build does not read (a
/// newer one) is `code_too_new`.
pub fn decode(text: &str) -> Result<Found, PasteError> {
    let first = text
        .chars()
        .find(|c| *c != '-' && !c.is_whitespace())
        .and_then(crate::code::symbol_value);
    if first.is_none_or(|v| v & 0b0_1111 != CODE_VERSION_ESCAPE) {
        return ChallengeCode::decode(text)
            .map(Found::Challenge)
            .map_err(PasteError::Challenge);
    }
    let kind = unframe_any(text, KIND_SCORECARD)
        .map_err(PasteError::Result)?
        .kind;
    let r = match kind {
        KIND_SCORECARD => Scorecard::decode(text).map(Found::Scorecard),
        KIND_ANSWER => AnswerCode::decode(text).map(|a| Found::Answer(Box::new(a))),
        KIND_EAR_CHALLENGE => EarChallengeCode::decode(text).map(Found::EarChallenge),
        KIND_EAR_RESULT => EarResult::decode(text).map(Found::EarResult),
        other => Err(ResultError::TooNew {
            what: "kind",
            value: u64::from(other),
        }),
    };
    r.map_err(PasteError::Result)
}

/// The text with chat damage undone (see the [module docs](self)): dashes as hyphens, zero-width characters dropped,
/// odd spaces as spaces, full-width ASCII as ASCII.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{2010}'..='\u{2015}'
            | '\u{2212}'
            | '\u{2043}'
            | '\u{FE58}'
            | '\u{FE63}'
            | '\u{FF0D}' => {
                out.push('-');
            }
            '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{180E}' => {}
            '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{3000}' => out.push(' '),
            '\u{FF01}'..='\u{FF5E}' => {
                out.push(char::from_u32(u32::from(c) - 0xFEE0).unwrap_or(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// A token and whether only spaces or line breaks separate it from the next one.
struct Token {
    text: String,
    joins_next: bool,
}

/// Capitals, digits and hyphens only: a token that may be part of a code split by spaces or a line break.
fn code_shaped(t: &str) -> bool {
    t.bytes()
        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase() || b == b'-')
}

/// Code symbols in a token (hyphens not counted).
fn symbols(t: &str) -> usize {
    t.bytes().filter(|&b| b != b'-').count()
}

/// Whether a token that decoded as nothing looks like a code a person meant: ten symbols or more, one a digit.
fn plausible(t: &str) -> bool {
    symbols(t) >= 10 && t.bytes().any(|b| b.is_ascii_digit())
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut cur = String::new();
    // Whether only whitespace has come since the last token ended.
    let mut soft = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            if cur.is_empty()
                && let Some(last) = out.last_mut()
            {
                last.joins_next = soft;
            }
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(Token {
                    text: core::mem::take(&mut cur),
                    joins_next: false,
                });
                soft = true;
            }
            if !c.is_whitespace() {
                soft = false;
            }
        }
    }
    if !cur.is_empty() {
        out.push(Token {
            text: cur,
            joins_next: false,
        });
    }
    out
}

/// Every code in a pasted message (see the [module docs](self)).
pub fn paste(text: &str) -> Pasted {
    let text = normalize(text);
    let toks = tokens(&text);
    let mut pasted = Pasted::default();
    let mut i = 0;
    while i < toks.len() {
        // The stretch of code-shaped tokens from i that spaces alone join.
        let mut end = i;
        if code_shaped(&toks[i].text) {
            while end + 1 < toks.len() && toks[end].joins_next && code_shaped(&toks[end + 1].text) {
                end += 1;
            }
        }
        let mut hit = None;
        for j in (i..=end).rev() {
            let joined: String = toks[i..=j].iter().map(|t| t.text.as_str()).collect();
            if symbols(&joined) > MAX_SYMBOLS {
                continue;
            }
            if let Ok(found) = decode(&joined) {
                hit = Some((j, found));
                break;
            }
        }
        if let Some((j, found)) = hit {
            let text = found.encode();
            if !pasted.found.iter().any(|f| f.encode() == text) {
                pasted.found.push(found);
            }
            i = j + 1;
        } else {
            let joined: String = toks[i..=end].iter().map(|t| t.text.as_str()).collect();
            if plausible(&joined) {
                if let Err(error) = decode(&joined) {
                    pasted.refused.push(Refused {
                        text: joined,
                        error,
                    });
                }
                i = end + 1;
            } else {
                i += 1;
            }
        }
    }
    pasted
}

#[cfg(test)]
mod tests;
