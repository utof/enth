//! **Owner: lane PLY** (player and checks, wave 3). Pre-declared by lane S0l (R2).
//!
//! The patch-predicate grammar and its evaluation: `Value(Addr, Cmp, Plain)`, `Route(Src, Dst, Cmp, Pct)`,
//! `Has(ModuleKind)`, `Enabled(Id, bool)` and `Touched { addr, min_span_pct }` ("turn it and listen", from the edit
//! log), with `macro1` as the form for the knob (Q5). Every CLI unit form (Hz, ms, dB, cents, %, octaves) parses in a
//! predicate. Predicates re-run after each delivered edit, in microseconds. Empty until PLY writes it.
