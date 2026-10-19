//! The modulation matrix, macros and per-segment parameter evaluation (P1a lane S2b). Empty in S2a.
//!
//! Seams S2a left for this file (see `docs/dsp/engine.md`, "Seams"):
//!
//! - **Step 3, smoothers** (`crate::params::VoiceTable::advance`, `GlobalTable::advance`): S2a copies `base` into
//!   `cur`. Smoothers run on the base value in the normalised domain (`ParamSpec::to_norm`/`from_norm`, the spec's
//!   `Smoothing`), and a settled smoother must deliver `base` exactly (never `from_norm(to_norm(x))`).
//! - **Step 6, routes**: `n = clamp(to_norm(smoothed) + Σ depth/100 · src, 0, 1)`, `cur = from_norm(n)`, summed in
//!   patch route order, per active voice for voice-rate destinations. Sources: voice modulator outputs
//!   (`Plan::vmod_cur`), global modulator outputs (`Plan::gmod_cur`), velocity and key (`NoteState`), macros and the
//!   mod wheel (new global sources). A route from a modulator later in the order reads its previous-segment value
//!   (`vmod_prev`): a deterministic one-segment delay. Modulator parameters are evaluated just before that
//!   modulator ticks (steps 4–5), processor parameters in step 6.
//! - The plan compiler (`crate::plan::compile`) refuses routes with `CompileError::NotYet` until this lands.
