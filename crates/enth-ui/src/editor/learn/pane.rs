//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); empty until LUI writes it.
//!
//! The lesson pane, the `LessonPane` of `widgets/lesson_pane.rs` fed by `enth-learn`'s block AST: headings, bold,
//! bullets and `enth:` links (a link such as `[Cutoff](enth:filter1.cutoff)` highlights its control on hover),
//! drawn with design tokens only; the Hear / Try / Do / Check controls; the hint tiers. A 320 px pane fits 1200 x 800
//! only when the step hides unused modules (the G1 patch overflows by 276 to 732 px), so LUI needs a collapsible pane
//! or a focus-card region, with an overflow report. Sizes go in `theme/sizes/learn.rs`.
