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
//! | [`fingerprint`] | [`TargetFingerprint`]: the schema-independent identity of a generated target (F1r review M1) | CT, with RG |
//! | [`score`] | the [`Scorer`] and [`Combiner`] traits, [`SoundScore`], [`StructureScore`], [`Combined`], [`ScoreReport`]; the round's composition ([`score::round`]: [`prepare_round`], [`score_round`]); the stubs; sound, structure, combined and matching | CT; SS; ST |
//! | [`random`] | the generator: keyed draws, the observability mask, minimum step, frozen generator versions | RG |
//! | [`priors`] | hand-written per-parameter priors relative to the phrase's note; the `musical` blend | RG |
//! | [`phrases`] | the versioned phrase table (one held note, an octave pair, a legato pair) | RG |
//! | [`calibrate`] | per-challenge just-noticeable-difference units and the fitted sound-% curve | SS |
//! | [`loudness`] | an in-house BS.1770 loudness on `libm` for the scored path, bit-exact on every platform | SS |
//!
//! R2 (Challenge v2, freeze point F1l): files pre-declared by lane S0l and filled by lane CV2 (docs/challenge.md,
//! "Challenge v2"):
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`v2`] | Challenge v2's pieces (`Target::Draw`, `Target::Patch`, `Start::Patch`, `Constraints { editable, locked }` over one `Selector`), the file forms of both versions and the v1 migration; `AnswerKind::Choice` and `Target::Expectations` are left out (lessons use `enth-learn`'s own) | CV2 |
//! | [`patch_round`] | `prepare_patch_round`, moved down from `enth-cli`'s `compare.rs`, and `prepare_draw_round` | CV2 |
//! | [`lesson_phrases`] | the lesson-only phrase ids (`CHORD`, `LONG_HOLD`, `STACCATO`, `STACCATO_RUN`) and their invariants | CV2 |
//!
//! R4 and R5 (Challenge v3 and the result codes, freeze point F1e): files pre-declared by lane S0e and filled by lanes CV3
//! and RC (docs/challenge.md, "Challenge v3" and "Version 15: the kinds behind the escape"; docs/ear.md; docs/duels.md).
//! `code.rs` stays code v1, frozen: the new kinds sit behind its reserved version-15 escape.
//!
//! | File | Holds | Lane |
//! | --- | --- | --- |
//! | [`v3`] | Challenge v3's pieces (`Target::Ear(EarSpec)` from `enth-music`, `AnswerKind::{Choice, Value, Taps}`), additive: v1 and v2 read and write as before | CV3 |
//! | [`ear`] | an ear question to a sound (the frozen timbre pool `timbres-v1/`, a `Performance`, the post-EQ band), the parameter games' items on v2 draws, the scorers `ear/1`, `ear-value/1` and `taps/1`, the ear-challenge code; `ear/ladder.rs` is G4's | CV3; G4 |
//! | [`result`] | the version-15 kind numbers (reserved by S0e), the scorecard (kind 1) and the ear result (kind 4) | RC |
//! | [`answer`] | the answer code (kind 2): the patch as a diff through the frozen `result-params-v1` table | RC |
//! | [`verify`] | `verify`: decode, regenerate, check the hash, re-score with the named scorer set, compare with the claim | RC |
//! | [`paste`] | paste-anything: every code in a pasted chat message, dashes and zero-width characters normalised | RC |
//! | [`daily`] | the daily challenge formula `daily::v1` and its golden | RC |

pub mod answer;
pub mod calibrate;
pub mod code;
pub mod daily;
pub mod ear;
pub mod fingerprint;
pub mod lesson_phrases;
pub mod loudness;
pub mod model;
pub mod paste;
pub mod patch_round;
pub mod phrases;
pub mod priors;
pub mod random;
pub mod result;
pub mod score;
pub mod spec;
pub mod v2;
pub mod v3;
pub mod verify;

pub use code::{ChallengeCode, CodeError, CodeForm, CodeWarning, EngineVersion, TargetPrefix};
pub use fingerprint::TargetFingerprint;
pub use model::{
    Agreement, AnswerKind, Assists, AssistsUsed, Challenge, ChallengeError, ChallengeMeta,
    Constraints, Outcome, Phrase, PhraseId, PhraseNote, PhraseView, RoundResult, ScoringSpec,
    Start, Target,
};
pub use score::round::{
    PreparedRound, RoundError, prepare_challenge, prepare_round, score_round, score_structure,
};
pub use score::{
    Closeness, Combined, Combiner, Component, ComponentKind, Direction, ScoreError, ScoreReport,
    Scorer, ScorerId, SoundScore, StructureScore, TargetContext, Verdict,
};
pub use spec::{Category, RandomSpec, SpecError, SpecPreset};
