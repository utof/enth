//! **Owner: lane FE** (the intake, freeze point F1s). Pre-declared by lane S0s (R3); empty until FE writes it.
//!
//! The inferred performance, `InferredPerf` (docs/r3.md, "The intake"): the note from YIN ([`pitch`](crate::pitch)),
//! the note-off from [`infer_note_off`](crate::envelope::infer_note_off), and the hold. Several onsets raise a "this
//! looks like a phrase" warning and a chord a chord warning, and the CLI's `--perf` overrides all of it (CLC). FE's
//! acceptance: the note right on at least 95 % of pitched clips, a chord warning. `InferredPerf` is one of the three
//! intake types that reach freeze point F1s.
