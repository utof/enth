//! **Owner: lane LCK.** Pre-declared by lane S0l (R2); the command is built in `tools/lesson/src/new.rs`.
//!
//! `cargo xtask lesson new <pack> <id>`: a driver for `enth-lesson-tool new <pack> <id>`, which scaffolds a lesson in
//! the authoring layout of `content/lessons/<pack>/`.

/// `cargo xtask lesson`: only `new` exists; it runs `enth-lesson-tool new`.
pub fn lesson(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("new") => super::drive("new", &args[1..]),
        _ => Err("usage: cargo xtask lesson new <pack> <id>".to_owned()),
    }
}
