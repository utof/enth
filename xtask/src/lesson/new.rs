//! **Owner: lane LCK.** Pre-declared by lane S0l (R2) as a stub that refuses to run.
//!
//! `cargo xtask lesson new <pack> <id>`: scaffolds a lesson in the authoring layout of `content/lessons/<pack>/` (a
//! `lesson.toml`, one markdown file per step and room for the start patches), the first step of the authoring
//! pipeline (docs/learn.md): an authoring agent then drafts from the plan and the catalogue, and `lesson-check` proves
//! the lesson honest.

/// `cargo xtask lesson`: only `new` exists, and it is not built yet.
pub fn lesson(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("new") => Err("lesson new is not built yet (R2 lane LCK; docs/phase-3.md)".to_owned()),
        _ => Err(
            "usage: cargo xtask lesson new <pack> <id>  (not built yet: R2 lane LCK)".to_owned(),
        ),
    }
}
