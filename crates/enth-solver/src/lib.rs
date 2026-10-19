//! enth-solver (L5, see docs/architecture.md): sensitivity and the multi-parameter hint solver (P1a lane H).
//!
//! Empty at freeze point F0. The solver spawns no threads: batch evaluation goes through a caller-supplied
//! evaluator that returns results by input index, so results are identical for any worker count.
