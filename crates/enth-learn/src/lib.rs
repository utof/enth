//! enth-learn (L5, see docs/architecture.md and docs/learn.md): lessons, content packs and the pure lesson player.
//! R2 (Learn) puts here what the session, the CLI and the lesson tool all share: the pack and lesson model (`enth.pack/1`,
//! `LessonRef`, compatibility), the `.enthpack` reader, the four kinds of check and their evaluation (patch
//! predicates; sound expectations through `enth-render` and `enth-analysis`), progress and its merge, the markdown
//! block AST (`pulldown-cmark`), the deconstruct planner, coverage and the review boxes. The R2 research session
//! (`_local/sessions/2026-10-21-r2-learn/`, `summary.md`) placed all of it here, above `enth-challenge` (it embeds
//! `Challenge`) and beside `enth-solver`, which it never calls: solver jobs carry solver-native payloads the shell runs.
//!
//! A core crate: no I/O, no threads, no clocks, no global RNG, and it builds for `wasm32-unknown-unknown` (`cargo
//! xtask wasm-check`, which covers every layer up to 5). Time comes from the shell; files come through a
//! [`PackStore`](store) the shell supplies. Nothing here depends on the editor's `learn` feature or the plugin's:
//! a saved lesson restores in any build.
//!
//! **Owner: lane LM** (re-exports at F1l; the main types are re-exported at the root). Lane S0l (R2, freeze point F0l) created the crate and declared every file up
//! front, so the lanes that fill them never edit this one: PLY, LCK, DEC and REV name their items by module path
//! (`enth_learn::predicate::…`) rather than adding re-exports here. Each file names its owner at the top. The
//! reference is docs/learn.md; every body is a stub until its lane lands (the freeze order is in docs/phase-3.md).
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`error`] | the refusals, each with a hint (a newer pack is refused and named) | LM |
//! | [`pack`] | the manifest `enth.pack/1`, `requires`, `LessonRef`, compatibility, the track field (Q1) | LM |
//! | [`store`] | the `PackStore` seam (`scan`, `read`, `install`) and its memory store | LM |
//! | [`reader`] | the `.enthpack` reader on `rawzip`, `miniz_oxide` and `sha2`; reads in memory, never extracts | LM |
//! | [`lesson`] | lesson, step, `StepKind`, `StepView`, the Listen examples | LM |
//! | [`check`] | `Check` and the check specs (patch, sound, choice, all) | LM; PLY |
//! | [`predicate`] | the patch-predicate grammar (`Value`, `Route`, `Has`, `Enabled`, `Touched`) and its evaluation | PLY |
//! | [`player`] | the pure lifecycle: start, edits, results, hint, show, next, back, end | LM; PLY |
//! | [`progress`] | per-user progress v1 and `Progress::merge` | LM; REV (additive) |
//! | [`markdown`] | the block AST of a lesson step on `pulldown-cmark`, and the text lint | PLY |
//! | [`coverage`] | which modules and Basic-tier parameters the lessons touch (`lesson-coverage`) | LCK |
//! | [`deconstruct`] | the ablation planner `plan(preset, ranks)` | DEC |
//! | [`review`] | Leitner boxes per skill, fed by Recreate's rings | REV |
//!
//! R4 (the drill player, freeze point F1e): files pre-declared by lane S0e and filled by lane DRL (docs/ear.md, "The
//! drill player" and "Workouts"; docs/learn.md, "Ear skills, bursts and drills"). DRL also edits `review`, `lesson` and
//! `progress` additively (`ear.*` skills and bursts, `StepKind::Drill`, the `drills` field).
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`drill`] | the drill player: N questions, the 3-down/1-up staircase, no visible streak | DRL |
//! | [`workout`] | the pure workout composer: 3 games, due skills choose them, the same workout for the same progress file and day | DRL |

pub mod check;
pub mod coverage;
pub mod deconstruct;
pub mod drill;
pub mod error;
pub mod lesson;
pub mod markdown;
pub mod pack;
pub mod player;
pub mod predicate;
pub mod progress;
pub mod reader;
pub mod review;
pub mod store;
pub mod workout;

pub use check::{Check, Cmp, Knob, MacroPin, Plain, Predicate, SoundCheck};
pub use error::{LearnError, PackName};
pub use lesson::{Example, Lesson, Selector, Step, StepKind, StepView, Touch};
pub use pack::{
    LessonBundle, LessonKey, LessonRef, Manifest, Pack, PackId, PackVersion, Requires, StepId,
};
pub use player::{EditSource, Effect, EndChoice, Player, PlayerError, RoundEnd, StepPhase};
pub use progress::{Progress, StepProgress};
pub use store::{MemStore, PackStore};
