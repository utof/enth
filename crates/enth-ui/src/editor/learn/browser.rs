//! **Owner: lane LUI** (the Learn view, feature `learn`). Pre-declared by lane S0l (R2); empty until LUI writes it.
//!
//! The lesson browser: the installed packs and their lessons, progress, the Review entry (lane REV feeds it), "Open
//! packs folder" and "Get more lessons" (which opens the releases page in the browser through egui's `OpenUrl`; R2 has
//! no network code). The view rescans when it opens. Sizes go in `theme/sizes/learn.rs`.
