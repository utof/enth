//! The expected file: plain text, one fact per line, compared with a tolerance, merged additively.
//!
//! ```text
//! # comments are ignored
//! code 1040-1000-0000-1R                      <- the header: facts about the challenge
//! fingerprint 006f84…
//!
//! scoring sound/1 structure/1 combined/1 time/1 weight=50 pass_mark=90     <- a block: one scorer set
//!   answer exact                                                           <- its answers, in the case's order
//!     report sound=100.000000000 structure=100.000000000 …
//!     component pitch same close target=… answer=…
//!     ring osc1.wave good same 1.000000000 answer=osc1.wave
//! ```
//!
//! **Comparison.** Words must be equal. A decimal number (`-?digits.digits`, with or without a `key=` before it) must
//! be within [`TOLERANCE`] of the pinned one (component readings, which carry `target=` and `answer=`, within
//! [`TOLERANCE`] of the pinned value *relative* to it, since they are in Hz and ms). Scores are bit-exact across
//! platforms, so the tolerance only stands for the printed digits; it is the contract the plan asks for (1e-6).
//!
//! **Merging.** A block is a scorer set, keyed by its four ids. Scorers are additive (design.md §6): a new set adds a
//! block and an existing block never changes. [`merge`] keeps every pinned line as it is, appends what is new, and
//! refuses a change.

use std::fmt::Write as _;

use super::TOLERANCE;

/// A parsed expected file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    /// Facts about the challenge (code, spec, fingerprint, render), before the first block.
    pub header: Vec<String>,
    /// One block per scorer set.
    pub blocks: Vec<Block>,
}

/// The answers' results under one scorer set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// `scoring sound/1 structure/1 combined/1 time/1 weight=50 pass_mark=90`.
    pub heading: String,
    /// One section per answer.
    pub answers: Vec<Answer>,
}

/// One answer's result lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// `exact`, `start` or a case-defined name.
    pub name: String,
    /// `report …`, `sound …`, `component …`, `ring …`, `module …`.
    pub lines: Vec<String>,
}

impl Block {
    /// The scorer ids, which identify the block (the weight and the pass mark follow from them and the difficulty,
    /// and are pinned as part of the heading).
    pub fn key(&self) -> String {
        self.heading
            .split_whitespace()
            .skip(1)
            .take(4)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Doc {
    /// Parse the text.
    ///
    /// # Errors
    /// A line outside any section, or an `answer` outside a block.
    pub fn parse(text: &str) -> Result<Doc, String> {
        let mut doc = Doc::default();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with("scoring ") {
                doc.blocks.push(Block {
                    heading: line.to_owned(),
                    answers: Vec::new(),
                });
            } else if let Some(name) = line.strip_prefix("answer ") {
                let block = doc
                    .blocks
                    .last_mut()
                    .ok_or_else(|| format!("line {}: `answer` before any `scoring`", n + 1))?;
                block.answers.push(Answer {
                    name: name.trim().to_owned(),
                    lines: Vec::new(),
                });
            } else if let Some(answer) = doc.blocks.last_mut().and_then(|b| b.answers.last_mut()) {
                answer.lines.push(line.to_owned());
            } else if doc.blocks.is_empty() {
                doc.header.push(line.to_owned());
            } else {
                return Err(format!("line {}: `{line}` is outside any answer", n + 1));
            }
        }
        Ok(doc)
    }

    /// The canonical text, under `title` (a comment line).
    pub fn render(&self, title: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# {title}");
        for line in &self.header {
            let _ = writeln!(out, "{line}");
        }
        for block in &self.blocks {
            let _ = write!(out, "\n{}\n", block.heading);
            for answer in &block.answers {
                let _ = writeln!(out, "  answer {}", answer.name);
                for line in &answer.lines {
                    let _ = writeln!(out, "    {line}");
                }
            }
        }
        out
    }

    /// The block for these scorer ids, if pinned.
    pub fn block(&self, key: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.key() == key)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Comparison

/// Whether two lines say the same thing: equal words, numbers within the tolerance.
pub fn lines_match(want: &str, got: &str) -> bool {
    let (mut w, mut g) = (want.split_whitespace(), got.split_whitespace());
    loop {
        match (w.next(), g.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if token_matches(a, b) => {}
            _ => return false,
        }
    }
}

fn token_matches(want: &str, got: &str) -> bool {
    if want == got {
        return true;
    }
    let (wk, wv) = split_key(want);
    let (gk, gv) = split_key(got);
    if wk != gk {
        return false;
    }
    match (decimal(wv), decimal(gv)) {
        (Some(a), Some(b)) => {
            let tol = if wk == "target" || wk == "answer" {
                TOLERANCE * a.abs().max(1.0)
            } else {
                TOLERANCE
            };
            (a - b).abs() <= tol
        }
        _ => false,
    }
}

fn split_key(token: &str) -> (&str, &str) {
    token.split_once('=').unwrap_or(("", token))
}

/// A decimal number as this module prints it: an optional minus, digits, a point, digits.
fn decimal(token: &str) -> Option<f64> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    let (int, frac) = digits.split_once('.')?;
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (all_digits(int) && all_digits(frac))
        .then(|| token.parse().ok())
        .flatten()
}

/// The differences between pinned and computed lines, as `want`/`got` pairs, at most `limit` of them.
pub fn diff_lines(label: &str, want: &[String], got: &[String], limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut differing = 0;
    for i in 0..want.len().max(got.len()) {
        let (w, g) = (want.get(i), got.get(i));
        if w.zip(g).is_some_and(|(w, g)| lines_match(w, g)) {
            continue;
        }
        differing += 1;
        if out.len() < limit {
            out.push(format!(
                "{label} line {}:\n    want {}\n    got  {}",
                i + 1,
                w.map_or("(none)", String::as_str),
                g.map_or("(none)", String::as_str)
            ));
        }
    }
    if differing > out.len() {
        out.push(format!(
            "{label}: ... and {} more differing lines",
            differing - out.len()
        ));
    }
    out
}

/// Every difference between a pinned block and a computed one (same scorer ids): the heading, the answers' names and
/// every line.
pub fn diff_block(pinned: &Block, fresh: &Block, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    if !lines_match(&pinned.heading, &fresh.heading) {
        out.push(format!(
            "{} heading:\n    want {}\n    got  {}",
            pinned.key(),
            pinned.heading,
            fresh.heading
        ));
    }
    for want in &pinned.answers {
        match fresh.answers.iter().find(|a| a.name == want.name) {
            Some(got) => out.extend(diff_lines(
                &format!("{} / {}", pinned.key(), want.name),
                &want.lines,
                &got.lines,
                limit,
            )),
            None => out.push(format!(
                "{}: the pinned answer `{}` is no longer produced (the case lost its recipe)",
                pinned.key(),
                want.name
            )),
        }
    }
    for got in &fresh.answers {
        if !pinned.answers.iter().any(|a| a.name == got.name) {
            out.push(format!(
                "{}: the answer `{}` is not pinned yet",
                pinned.key(),
                got.name
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Merging

/// Why a pinned case may not change, with the way out.
pub const FROZEN: &str = "a pinned result never changes (design.md §6, scorers are additive): a scorer that moves a score is a new id with its own block, never an edit of the old one";

/// The pinned document extended with what is new in `fresh`: new blocks, and new answers in an existing block. Every
/// pinned line stays exactly as it is.
///
/// # Errors
/// Every pinned line that `fresh` no longer reproduces (the header, a block, an answer), as messages.
pub fn merge(pinned: &Doc, fresh: &Doc) -> Result<Doc, Vec<String>> {
    let mut problems = diff_lines("header", &pinned.header, &fresh.header, 6);
    let mut out = pinned.clone();
    for block in &fresh.blocks {
        match pinned.block(&block.key()) {
            None => out.blocks.push(block.clone()),
            Some(old) => {
                for answer in &block.answers {
                    let Some(want) = old.answers.iter().find(|a| a.name == answer.name) else {
                        if let Some(target) = out.blocks.iter_mut().find(|b| b.key() == block.key())
                        {
                            target.answers.push(answer.clone());
                        }
                        continue;
                    };
                    problems.extend(diff_lines(
                        &format!("{} / {}", old.key(), want.name),
                        &want.lines,
                        &answer.lines,
                        6,
                    ));
                }
                for want in &old.answers {
                    if !block.answers.iter().any(|a| a.name == want.name) {
                        problems.push(format!(
                            "{}: the pinned answer `{}` is no longer produced",
                            old.key(),
                            want.name
                        ));
                    }
                }
                if !lines_match(&old.heading, &block.heading) {
                    problems.push(format!(
                        "{} heading:\n    want {}\n    got  {}",
                        old.key(),
                        old.heading,
                        block.heading
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# t\ncode 1040-1000-0000-1R\nfingerprint 00ab\n\nscoring sound/1 structure/1 combined/1 time/1 weight=50 pass_mark=90\n  answer exact\n    report sound=100.000000000 passed=true\n    component pitch same close target=-1200.500000000 answer=-1200.500000000\n  answer start\n    report sound=4.390000000 passed=false\n";

    #[test]
    fn a_document_reads_back_to_the_same_text() {
        let doc = Doc::parse(SAMPLE).unwrap();
        assert_eq!(doc.header, ["code 1040-1000-0000-1R", "fingerprint 00ab"]);
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].key(), "sound/1 structure/1 combined/1 time/1");
        assert_eq!(doc.blocks[0].answers.len(), 2);
        assert_eq!(doc.render("t"), SAMPLE);
    }

    #[test]
    fn numbers_match_within_the_tolerance_and_words_exactly() {
        assert!(lines_match(
            "report sound=100.000000000 passed=true",
            "report sound=99.999999500 passed=true"
        ));
        assert!(!lines_match(
            "report sound=100.000000000 passed=true",
            "report sound=99.999990000 passed=true"
        ));
        assert!(!lines_match("report passed=true", "report passed=false"));
        assert!(!lines_match("ring a good", "ring a good extra"));
        // Readings are compared relative to their size.
        assert!(lines_match(
            "component brightness same close target=3000.000000000 answer=1.500000000",
            "component brightness same close target=3000.001000000 answer=1.500000000"
        ));
        assert!(!lines_match(
            "component brightness same close target=3000.000000000 answer=1.500000000",
            "component brightness same close target=3000.010000000 answer=1.500000000"
        ));
        // Integers and hex are words, never numbers.
        assert!(!lines_match("fingerprint 0012", "fingerprint 0013"));
        assert!(!lines_match("spec seed=12345", "spec seed=12346"));
        // Signed zero is zero.
        assert!(lines_match("x=-0.000000000", "x=0.000000000"));
    }

    #[test]
    fn merge_appends_new_blocks_and_answers_and_refuses_changes() {
        let pinned = Doc::parse(SAMPLE).unwrap();
        // Identical: nothing to add.
        assert_eq!(merge(&pinned, &pinned).unwrap(), pinned);
        // A new scorer set is a new block; the old block stays byte for byte.
        let mut fresh = pinned.clone();
        fresh.blocks.push(Block {
            heading: "scoring sound/2 structure/1 combined/1 time/1 weight=50 pass_mark=90"
                .to_owned(),
            answers: vec![Answer {
                name: "exact".to_owned(),
                lines: vec!["report sound=100.000000000 passed=true".to_owned()],
            }],
        });
        let merged = merge(&pinned, &fresh).unwrap();
        assert_eq!(merged.blocks.len(), 2);
        assert_eq!(merged.blocks[0], pinned.blocks[0]);
        // A new answer in an old block is added.
        let mut fresh = pinned.clone();
        fresh.blocks[0].answers.push(Answer {
            name: "near".to_owned(),
            lines: vec!["report sound=91.000000000 passed=true".to_owned()],
        });
        assert_eq!(merge(&pinned, &fresh).unwrap().blocks[0].answers.len(), 3);
        // A moved score is refused, and so is a moved header.
        let mut moved = pinned.clone();
        moved.blocks[0].answers[1].lines[0] = "report sound=4.500000000 passed=false".to_owned();
        let why = merge(&pinned, &moved).unwrap_err().join("\n");
        assert!(why.contains("want report sound=4.390000000"), "{why}");
        let mut moved = pinned.clone();
        moved.header[1] = "fingerprint 00ac".to_owned();
        assert!(merge(&pinned, &moved).is_err());
        // A lost answer is refused.
        let mut lost = pinned.clone();
        lost.blocks[0].answers.pop();
        assert!(merge(&pinned, &lost).is_err());
    }
}
