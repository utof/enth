//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until
//! RC writes it. The reference is docs/duels.md, "Sharing and paste".
//!
//! **Paste-anything.** The plugin and the CLI share it: it takes a whole chat message and pulls out every token that
//! decodes (a challenge, a scorecard, an answer), several per paste. It maps every dash-like character to a hyphen
//! (phones' smart punctuation turns `--` into a dash, so codes never contain `--`) and drops zero-width characters and
//! non-breaking spaces. No clipboard access here: the shell writes the clipboard on a click and reads it only on an
//! explicit Paste.
