//! **Owner: lane DEC** (deconstruct and studies, wave 4). Pre-declared by lane S0l (R2); the command is built in
//! `tools/lesson/src/gen.rs`.
//!
//! `cargo xtask lesson-gen deconstruct <preset>`: a driver for `enth-lesson-tool gen deconstruct <preset>`.

/// `cargo xtask lesson-gen`: runs `enth-lesson-tool gen`.
pub fn lesson_gen(args: &[String]) -> Result<(), String> {
    super::drive("gen", args)
}
