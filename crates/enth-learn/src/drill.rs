//! **Owner: lane DRL** (Drill player, freeze point F1e; its API is judged there with the other R4 and R5 APIs).
//! Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until DRL writes it. The reference is docs/ear.md ("The
//! drill player", "Difficulty and the staircase") and docs/learn.md, "Ear skills, bursts and drills (R4, planned, lane
//! DRL)".
//!
//! The pure drill player: N questions at a time, and the **3-down/1-up staircase** (about 79 % success) for drills and
//! Ear check, while Recreate keeps its 2-down/1-up. A staircase moves one thing at a time: the step in JND units, the
//! number of categories, their similarity, the context, roving, or memory load. Items are placed by measured JND
//! distance, with a 2-JND floor. The run counter stays internal: **no visible streak**. Pure like the rest of the
//! crate: no clock (the shell passes the day), no RNG (draws are keyed), no I/O.
//!
//! Three additive edits belong to DRL in files that already exist, and are not declared here: `ear.*` skills and
//! 5-item bursts in [`review`](crate::review) (4 or more right moves a box up, 3 holds it, 2 or fewer moves it down),
//! `StepKind::Drill` in [`lesson`](crate::lesson) (declared in `requires`), and the `drills` field in
//! [`progress`](crate::progress) (thresholds per skill over time and the practised days). Simulated before it ships:
//! 75 to 85 % success, and simulated listeners' thresholds within one step in 90 % of 40-trial runs.
