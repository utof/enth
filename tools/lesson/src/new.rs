//! **Owner: lane LCK.** Pre-declared by lane S0l (R2) as a stub that refuses to run.
//!
//! `new <pack> <id>` (`cargo xtask lesson new`): scaffolds a lesson in the authoring layout of `content/lessons/<pack>/`
//! (a `lesson.toml`, one markdown file per step and room for the start patches), the first step of the authoring
//! pipeline (docs/learn.md): an authoring agent then drafts from the plan and the catalogue, and `check` proves the lesson
//! honest.

/// `new`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("lesson new is not built yet (R2 lane LCK; docs/phase-3.md)".to_owned())
}
