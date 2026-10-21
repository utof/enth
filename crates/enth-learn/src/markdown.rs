//! **Owner: lane PLY** (player and checks, wave 3). Pre-declared by lane S0l (R2).
//!
//! The block AST of a lesson step on `pulldown-cmark` (MIT; docs/learn.md, "The Learn view"): the editor's
//! `LessonPane` draws it with design tokens only. The lint bans HTML, images and code blocks and allows `https:` links
//! only; `enth:` links (`[Cutoff](enth:filter1.cutoff)`) must resolve, which `lesson-check` verifies. PLY writes the
//! AST and the lint.
//!
//! Lane S0l left one probe, [`events`]: the call that makes `pulldown-cmark` part of what `cargo xtask wasm-check`
//! builds for `wasm32-unknown-unknown` (the F0l acceptance; docs/phase-3.md). PLY replaces it with the real parser.

use pulldown_cmark::{Event, Parser};

/// How many start events `text` parses into (a heading, a paragraph, an emphasis each count one): the F0l probe for
/// `pulldown-cmark` on wasm32.
pub fn events(text: &str) -> usize {
    Parser::new(text)
        .filter(|event| matches!(event, Event::Start(_)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::events;

    #[test]
    fn events_counts_the_starts_of_a_step() {
        assert_eq!(events(""), 0);
        // A heading, a paragraph and an emphasis.
        assert_eq!(events("# Saw\n\nA *saw* wave."), 3);
    }
}
