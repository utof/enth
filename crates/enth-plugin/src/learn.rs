//! **Owner: lane SKL** (the Learn skeleton, freeze point F2l; feature `learn`). Pre-declared by lane S0l (R2); empty
//! until SKL writes it.
//!
//! The plugin's Learn I/O (docs/learn.md, "Packs" and "Progress and the saved state"; docs/plugin.md gets its Learn
//! section from SKL). Shells translate and hold no product rules: the pack format, the player, the checks and the
//! progress merge are `enth-learn`'s, the live binding is `enth-session`'s `learn`, and this file is the one place that
//! touches the disk for lessons:
//!
//! - **Pack I/O** through a `PackStore` over the bundled packs (the built `.enthpack` files, embedded with
//!   `include_bytes!` and read through the same reader as any other) and the Packs folder
//!   (`<Documents>/enth/Packs`, `ENTH_PACKS_DIR` overrides), rescanned when the view opens. No network code in R2:
//!   "Get more lessons" only opens the releases page in the browser.
//! - **The progress file**, `<Documents>/enth/Learn/progress.json` (`ENTH_LEARN_DIR` overrides): a write re-reads,
//!   merges with `Progress::merge`, writes a temporary file and renames it, so two instances never lose each other's
//!   progress.
//! - **Learn jobs on the game worker** (`game.rs`, fu1-hint's parallel evaluator included): an expectation hint is
//!   `{patch, perf, items, free}` for `enth_solver::hint`, a Recreate step's hint is the game's coach job, "How close am
//!   I?" is `{round, patch, free, budget}` for `enth_solver::closest`, and `JobKind::PrepareChallenge` is answered
//!   with `prepare_challenge`. The worker composes nothing.
//!
//! Off by default: the plugin's `learn` feature (it forwards `enth-ui/learn`) gates this module; `enth-learn` and the
//! session's `learn` always compile, so a saved lesson restores in any build.
