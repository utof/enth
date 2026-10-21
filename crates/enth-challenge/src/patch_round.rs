//! **Owner: lane CV2** (Challenge v2, freeze point F1l). Pre-declared by lane S0l (R2); empty until CV2 moves the
//! code here.
//!
//! `prepare_patch_round`: the "hand-made round on a patch target" that lives in `enth-cli`'s `cmd/compare.rs` today
//! (`moved_params`), moved down into `enth-challenge` so the session (a lesson's Recreate step), xtask (`lesson-check`)
//! and the CLI share it. `enth compare --target` and the golden challenges stay byte-identical (CV2's acceptance).
//! The CLI keeps a thin call to it.
