//! **Owner: lane ECK** (Ear check). Pre-declared by lane S0e (R4/R5); `ear-check` refuses to run until ECK builds it. The
//! reference is docs/ear.md, "GR4" (the mechanical half) and "The ear generator".
//!
//! `ear-check`: GR4's mechanical half, as `check` is the lessons'. `cargo xtask ear-check` becomes a thin driver of this
//! command (`xtask/src/ear.rs`, a placeholder that passes until then), so `xtask` stays light, and
//! `.github/workflows/ear.yml` runs it on `crates/enth-music/**`, the ear files of `enth-challenge`, this directory and the
//! crates a question's sound depends on, under 10 minutes. It fails on a seeded bad game of each kind: a leak a bias bot
//! finds, an inaudible pair, an unsolvable top level, a drifting hash. One file per concern, so the ladder lanes (G1 to G4)
//! and ECK never edit each other's:
//!
//! | File | Holds |
//! | --- | --- |
//! | `players` | the oracle (an ideal observer: picks the option nearest by `sound/1`), the random player, the bias bots (always option 1, the loudest, the longest) and the deaf player |
//! | `audibility` | every option pair's distance in JND units under `sound/1` per level (at least 2, at least 6 at level 1), the items scored on parameter steps listed apart as *not measured* |
//! | `staircase` | simulated listeners with known thresholds: the 3-down/1-up estimate within one step in 90 % of 40-trial runs, 75 to 85 % success |
//! | `golden` | ear generator v1's golden (1,000 specs to a hash), and 1,000 items per game on the six golden targets |

mod audibility;
mod golden;
mod players;
mod staircase;

/// `ear-check`: not built yet.
pub fn run(_args: &[String]) -> Result<(), String> {
    Err("`ear-check` is not built yet (R4 lane ECK)".to_owned())
}
