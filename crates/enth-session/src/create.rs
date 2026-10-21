//! **Owner: lane SK3** (the Create skeleton, freeze point F2s). Pre-declared by lane S0s (R3, freeze point F0s); empty
//! until SK3 writes it.
//!
//! The pure create state (docs/r3.md, "The Create view and the session"; docs/session.md gets its Create section from
//! SK3). Like [`game`](crate::game) and [`learn`](crate::learn) it does no I/O, spawns no threads and reads no clocks:
//! **time comes from timestamps the shell passes in**, and background work runs as jobs the shell executes on its own
//! thread and hands back. The audio thread never touches any of it.
//!
//! - **`Run` and `Cancel` go out to the shell**, results come back: a create job (the decoded upload, the intake's
//!   report, `--perf`, K and the budget) is handed to the shell's `enth-create` thread and its per-job pool
//!   (`enth-plugin`'s `create.rs`), never to the game worker. Progress and the K candidates (`enth_solver::create`'s
//!   `Candidate`: patch, distance, provenance, trace; each tile moves proposed, searching, done or pruned) arrive as
//!   posts, the way the game worker posts its results. This module holds the read model the Create view draws.
//! - **"Use this" is one undo step** (`ops_between`), delivered like a preset load, with **no host writes** (patches
//!   load through the session on the host's main thread, never through host automation; AGENTS.md, "Plugin shell").
//! - **A create job is not saved in the project** (provisional; the F1s judge may ask otherwise, docs/phase-3.md, "Open
//!   checks"): the patch you take is. So, unlike `game` and `learn`, this module adds **no field to the `enth` state
//!   blob** and `enth.plugin-state/1` is untouched.
//!
//! After F2s the session's create types change only additively (docs/phase-3.md, "Freezes"). Lane S0s declared the file
//! and nothing else; the crate's manifest already lists `enth-solver` and `enth-analysis`, whose types the read model
//! holds, so the lock is resolved.
