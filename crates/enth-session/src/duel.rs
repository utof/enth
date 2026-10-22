//! **Owner: lane SE** (Session, freeze point F1e; the F1e judge reviews the API with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until SE writes it.
//!
//! The pure duel state (docs/duels.md, "The flow"; docs/session.md gets its Duel section from SE). It does no I/O,
//! spawns no threads and reads no clocks and **has no networking: the clipboard is the shell's**, written on a click and
//! read only on an explicit Paste. Background work runs as jobs on the shell's game worker.
//!
//! - **A scorecard and an answer** are built from a Recreate round's **first passing submit** (otherwise the best submit,
//!   marked "not passed"): its patch, the time to pass and the submit count. The session keeps only the latest submitted
//!   patch today, so SE adds the first pass's patch to the round in [`game`](crate::game), additively and within
//!   `ROUND_BYTES` (768 KiB, with a patch of up to 128 KiB already inside): a named F1e check.
//! - **The Verify job** goes out to the shell, which runs `enth-challenge`'s `verify` on the game worker; the receiver's
//!   own re-score of both answers decides, and the read model holds both numbers.
//! - **A friend's answer stays sealed** until you have submitted or given up (a UI guard, not cryptography); afterwards
//!   loading it is one undo step with no host writes.
//! - **Nothing is saved in the project:** duels live in the per-user history the shell owns (lane DUI), so
//!   `enth.plugin-state/1` is untouched.
//!
//! After F2d the session's duel types change only additively (docs/phase-4.md, "Freezes"). Lane S0e declared the file and
//! nothing else.
