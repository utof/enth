//! **Owner: lane DEC** (deconstruct and studies, wave 4). Pre-declared by lane S0l (R2) as a stub that refuses to run.
//!
//! `gen deconstruct <preset>` (`cargo xtask lesson-gen`): writes a "deconstruct this preset" lesson by ablation
//! (docs/learn.md, "Deconstruct lessons"), using `enth_learn::deconstruct::plan` with the sensitivity `reach` it
//! measures. The factory studies in `content/lessons/studies/` come from here, with human-edited text; the plugin's worker
//! runs the same planner on a user's own presets.

/// `gen`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("lesson-gen is not built yet (R2 lane DEC; docs/phase-3.md)".to_owned())
}
