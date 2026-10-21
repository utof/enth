//! **Owner: lane DEC** (deconstruct and studies, wave 4). Pre-declared by lane S0l (R2).
//!
//! The ablation planner `deconstruct::plan(preset, ranks)` (docs/learn.md, "Deconstruct lessons"): elements are the
//! enabled modules, the routes and the parameter groups that differ from Init, ordered by signal flow and then by
//! contribution; step k's target is Init plus elements 1 to k. Pure: the caller measures the sensitivity `reach` that
//! orders it, so the planner needs no render. `lesson-gen deconstruct` (`xtask/src/lesson/gen.rs`) writes the factory
//! studies at authoring time and the plugin's worker runs the same planner on a user's presets. Empty until DEC
//! writes it.
