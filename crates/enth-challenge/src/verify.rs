//! **Owner: lane RC** (Result codes, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until
//! RC writes it. The reference is docs/duels.md, "Verification".
//!
//! `verify(result, evaluator)`: decode, regenerate the challenge, apply the answer's ops to the start and check the hash
//! (and the scorecard's commitment, when both are at hand), score with the named scorer set, compare with the claim.
//! Renders go through a caller-supplied evaluator, as in the solver. **The receiver's own re-score of both answers
//! decides;** a claimed number never does. Another engine version re-scores and shows both numbers; an unknown scorer
//! set is `scorer_too_new`; only a generator or hash mismatch refuses. An ear result needs no render: the answers are
//! replayed against the key.
