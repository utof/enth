//! **Owner: lane TP** (freeze point F1s). Pre-declared by lane S0s (R3); empty until TP writes it.
//!
//! The start portfolio (docs/r3.md, "The start portfolio and k-NN retrieval"): **the heuristic start**
//! ([`heuristic`](crate::heuristic)), always kept; **the templates** (`content/templates/`: `init`, `pluck_1osc`,
//! `pad_2osc`, `bass_sub`), because structure is never searched otherwise (GR1's level 5 solves 0 of 4); **k-NN
//! retrieval** over a pre-rendered patch bank ([`nearest`](crate::nearest)); and, in R3b, the model's K hypotheses
//! (lane MS). R3a runs without the bank. Sizing K and the budget waits for TP's measured per-evaluation cost on R1's
//! engine with effects.
