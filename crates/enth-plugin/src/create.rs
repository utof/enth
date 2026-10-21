//! **Owner: lane SK3** (the Create skeleton, freeze point F2s; feature `create`). Pre-declared by lane S0s (R3, freeze
//! point F0s); empty until SK3 writes it.
//!
//! The plugin's side of "create from sound" (docs/r3.md, "The cancellable pool" and "File input"; docs/plugin.md gets
//! its Create section from SK3). Shells translate and hold no product rules: the intake is `enth-analysis`'s, the
//! portfolio and the K searches are `enth-solver`'s `create`, the state machine is `enth-session`'s `create`, and this
//! file is the one place that decodes files, owns threads and touches the disk for it:
//!
//! - **An `enth-create` orchestrator thread**, spawned at the first job and joined on drop, like `enth-game`: it
//!   decodes, computes features, runs inference on one thread (R3b, milliseconds) and drives the searches. It never
//!   runs on the audio thread and never holds the session lock while computing; results are posted the way the game
//!   worker posts them. **It never touches `game.rs`**: the game worker is joined on drop ("a few hundred ms at most"),
//!   so a 60 s job there would hang an unload.
//! - **A per-job `rayon::ThreadPool`** (never the global one), its threads named `enth-search-N` and the pool dropped
//!   when the job ends, so no threads exist between jobs; workers `min(available_parallelism / 2, 8)` at normal
//!   priority, never real-time. The solver reads no clock, so the wall-clock cap lives here, in the `search_with`
//!   callback: it stops at the cap minus the intake, and the cap guarantees a result within 5 minutes.
//! - **Cancel** is an `AtomicBool` checked per job in the pool and after each round, so a cancel lands within one batch
//!   (about one evaluation, ~10 ms).
//! - **Decoders** (L8 only; the cargo feature `create` turns them on): WAV (`hound`), FLAC (`claxon`), Ogg Vorbis
//!   (`lewton`), MP3 (`nanomp3`, a pure-Rust port of the CC0 minimp3) and a small in-house AIFF reader. `symphonia`
//!   (M4A/AAC) is MPL-2.0 and waits for the owner's OK. A result records the decoded audio's hash, because whether
//!   `nanomp3` and `lewton` decode bit-identically across platforms is an open check (WAV and FLAC decode exactly).
//! - **The folder browser's listing** for the editor (`enth-ui`'s `create/browser.rs`), where a file drop does not
//!   reach the editor. No network code.
//!
//! The gate under load is `enth-test-host process-stats --gate g1` with a create job running, as lane SKR gated the
//! A/B. Off by default: the plugin's `create` feature (it forwards `enth-ui/create`) gates this module; the session's
//! `create` always compiles.
