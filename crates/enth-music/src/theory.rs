//! **Owner: lane MUS** (Music, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until MUS
//! writes it. The reference is docs/ear.md, "Where R4 lives" and "The ear generator".
//!
//! The tables the games draw from, all integers (a key is a MIDI note number, a size is in semitones):
//!
//! - **intervals**: unison to octave, with the names the games print ("P5", "m3", "tritone");
//! - **chords**: triads (major, minor, diminished, augmented, sus2, sus4), sevenths and inversions;
//! - **scales** and **progressions** with simple voice leading (R4b's progressions follow Functional Ear Trainer's
//!   order);
//! - **note names**, for R4b's opt-in note naming.
//!
//! A generated phrase stays within C1 to C7 (MIDI 24 to 96), with at most 8 notes at once and at most 10 s
//! (docs/ear.md). Tables are append-only: a game's answer keys are indices into them, and ear generator v1 is frozen by
//! a golden. MUS may turn this file into `theory/mod.rs` with submodules (`mod theory;` still resolves).
