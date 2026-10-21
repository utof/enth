//! **Owner: lane LM** (progress v1 and its merge, freeze point F1l); **REV** adds the review field, additively.
//! Pre-declared by lane S0l (R2).
//!
//! Per-user progress, the GR2 votes and the review schedule, the content of `<Documents>/enth/Learn/progress.json` (a
//! file the shell owns). `Progress::merge` is the union of steps and the maximum of bests, and never shrinks: the
//! plugin re-reads, merges, writes a temporary file and renames, so two instances never lose each other's progress.
//! A newer progress file is read leniently. Progress is keyed by pack, lesson and step ids, which are append-only.
//! The in-flight lesson of a project is not here: it is the `enth` field's `learn` object (`enth-session`). Empty
//! until LM writes it.
