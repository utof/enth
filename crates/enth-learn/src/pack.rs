//! **Owner: lane LM** (the lesson model, freeze point F1l). Pre-declared by lane S0l (R2).
//!
//! The pack: the manifest `enth.pack/1` (`id`, semver `version`, `title`, `authors`, `license`, `requires`, `covers`,
//! the `files` with a size and a SHA-256 each), `requires` (the highest lesson, challenge and patch schema, generator
//! and scorer ids the pack uses; it loads only if this build has them all), `LessonRef` (pack, pack version, lesson,
//! step, an 8-byte SHA-256 content prefix) and the track field (an ordered list of lesson references across installed
//! packs; Q1, decided at F1l). The engine version is recorded, not a gate (docs/learn.md, "Packs"). The file format
//! is report 02's of the R2 research session; LM freezes the fields at F1l. Empty until then.
