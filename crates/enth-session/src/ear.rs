//! **Owner: lane SE** (Session, freeze point F1e; the F1e judge reviews the API with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until SE writes it.
//!
//! The pure ear state (docs/ear.md, "The session and the Ear view"; docs/session.md gets its Ear section from SE). Like
//! [`game`](crate::game), [`learn`](crate::learn) and [`create`](crate::create) it does no I/O, spawns no threads and
//! reads no clocks: **time comes from timestamps the shell passes in**, and background work runs as jobs the shell
//! executes on its game worker and hands back. The audio thread never touches any of it.
//!
//! - **Questions go out as jobs** (a set of up to about 10 renders in about 0.3 s: `enth-challenge`'s `ear`), answers and
//!   taps come back with times the shell supplies, and this module holds the read model the Ear view draws: the item, the
//!   options, the replay with the answer, the staircase's level. The drill player and the workout composer are
//!   `enth-learn`'s (`drill`, `workout`); this module binds them to a session.
//! - **An ear session is not saved in the project**, like a create job: unlike `game` and `learn` it adds **no field to
//!   the `enth` state blob** and `enth.plugin-state/1` is untouched. Thresholds and practised days live in the per-user
//!   progress file the shell owns (`drills`).
//! - **Stale results are refused**, harmlessly, as the other modules refuse theirs.
//!
//! After F2e the session's ear types change only additively (docs/phase-4.md, "Freezes"). Lane S0e declared the file and
//! nothing else; the crate's manifest already lists `enth-music`, whose types the read model holds, so the lock is
//! resolved.
