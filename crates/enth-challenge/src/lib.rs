//! enth-challenge (L4, see docs/architecture.md and design.md §6 and §9): the [`Challenge`](model) behind a Recreate
//! round (and later a lesson step, a daily, a duel and "create from sound"), the Recreate randomizer, challenge codes,
//! the phrases a challenge plays, and the scorers. The R1/S1 research session
//! (`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`, `summary.md`) placed all of it here.
//!
//! A core crate: no I/O, no threads, no clocks, no global RNG, and it builds for `wasm32-unknown-unknown` (`cargo
//! xtask wasm-check`). Every random draw is keyed by its parameter address (`derive(derive(seed, fnv1a64(address)),
//! attempt)`), never taken in sequence, and target generation works in parameter space only (design.md §9), so a
//! seed gives the same target on every machine. Time comes from the shell: the session's `game` module passes it in.
//!
//! **Owner: lane CT** (re-exports at F1r). Lane S0r (R1/S1, freeze point F0r) created the crate and declared every
//! file up front, so the lanes that fill them never edit this one: RG, SS and ST name their items by module path
//! (`enth_challenge::random::…`) rather than adding re-exports here. Each file names its owner at the top. The
//! reference for the persisted types and the interfaces is docs/challenge.md.
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`spec`] | [`RandomSpec`], [`Category`], [`SpecPreset`]: the randomizer's integer-only settings and the frozen presets | CT, with RG |
//! | [`model`] | [`Challenge`] v1 (target, phrase, start, constraints, assists, [`ScoringSpec`]), [`Phrase`], and the round's results ([`RoundResult`]) | CT |
//! | [`code`] | [`ChallengeCode`]: code v1, Crockford base32 with a check symbol, short and long forms | CT |
//! | [`score`] | the [`Scorer`] and [`Combiner`] traits, [`SoundScore`], [`StructureScore`], [`Combined`], [`ScoreReport`]; sound, structure and combined scorers | CT; SS; ST |
//! | [`random`] | the generator: keyed draws, the observability mask, minimum step, frozen generator versions | RG |
//! | [`priors`] | hand-written per-parameter priors relative to the phrase's note; the `musical` blend | RG |
//! | [`phrases`] | the versioned phrase table (one held note, an octave pair, a legato pair) | RG |
//! | [`calibrate`] | per-challenge just-noticeable-difference units and the fitted sound-% curve | SS |
//! | [`loudness`] | an in-house BS.1770 loudness on `libm` for the scored path, bit-exact on every platform | SS |

pub mod calibrate;
pub mod code;
pub mod loudness;
pub mod model;
pub mod phrases;
pub mod priors;
pub mod random;
pub mod score;
pub mod spec;

pub use code::{ChallengeCode, CodeError, CodeForm, CodeWarning, EngineVersion, TargetPrefix};
pub use model::{
    Agreement, AnswerKind, Assists, AssistsUsed, Challenge, ChallengeError, ChallengeMeta,
    Constraints, Outcome, Phrase, PhraseId, PhraseNote, RoundResult, ScoringSpec, Start, Target,
};
pub use score::{
    Closeness, Combined, Combiner, Component, ComponentKind, Direction, ScoreError, ScoreReport,
    Scorer, ScorerId, SoundScore, StructureScore, TargetContext, Verdict,
};
pub use spec::{Category, RandomSpec, SpecError, SpecPreset};
