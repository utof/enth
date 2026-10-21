//! **Owner: lane TP** (the schedule's first version, freeze point F1s); **MS** adds the model's proposals (R3b).
//! Pre-declared by lane S0s (R3); empty until TP writes it.
//!
//! K branches refined by successive halving (docs/r3.md, "K branches and successive halving"): each candidate is its
//! own [`Search`](crate::search); each round merges every live search's `ask()` batch into one pool batch and splits
//! the distances back to each `tell()`, so the outcome does not depend on the worker count (8 searches x lambda 16 =
//! 128 jobs a round keep a pool busy). Halving is deterministic and **a function of evaluation counts, never of time**:
//! all K get about 10 % of the evaluations, the best half the next 30 %, the best two the rest. The solver reads no
//! clock: the wall-clock cap only truncates, in the shell's `search_with` callback, and a result records the renders
//! it used so a render-budget replay reproduces it on any machine.
