//! enth-engine (L2): the real-time synth engine. See `docs/dsp/engine.md` for the architecture as built, and
//! docs/design.md §3 and §5 for the decisions behind it.
//!
//! ```text
//! Patch ──compile──▶ Plan (+ PlanMap) ──Engine::new──▶ Engine ──process(left, right, events)──▶ stereo f32
//! ```
//!
//! - [`plan::compile`] turns a [`Patch`](enth_patch::Patch) into a [`Plan`]: every enabled module prepared
//!   ([`modules::ModuleImpl::new`], an exhaustive `match` on `ModuleKind`) and sorted by role, plus the flat
//!   parameter tables. It allocates; nothing after it does.
//! - [`Engine::process`] renders any number of frames on the free-running 32-frame segment grid behind a
//!   one-segment FIFO, so the host's block size never changes the output; events take effect at the first segment
//!   boundary at or after their frame.
//! - Every random stream comes from (render seed, module id, note index) ([`seed`]); never from parameter values.
//!
//! The module contract (roles, contexts, parameter views) is [`module`]; each module kind has its own file in
//! [`modules`]. S2a built the walking skeleton with stub modules (a reference saw, a reference envelope,
//! pass-throughs and silence); lanes D1, D2, FX1 and FX2 replace the stubs, and lane S2b adds the voice manager's
//! remaining modes, the modulation matrix and smoothing ([`modulation`]) and the plan swap ([`swap`]). Seams are
//! marked `TODO(S2b)`.
//!
//! Real-time rules (AGENTS.md): `process` and everything it calls never allocate, free, lock, log, do I/O or panic;
//! no `std::sync::mpsc`, no unbounded channels, no `Mutex`/`RwLock` (clippy bans); every ring and queue has an
//! overflow test under `assert_no_alloc`.

// RealtimeSanitizer (nightly workflow, `--cfg enth_rtsan`): `process` and everything it calls carry
// `#[cfg_attr(enth_rtsan, sanitize(realtime = "nonblocking"))]`. The cfg is declared in the workspace lints.
#![cfg_attr(enth_rtsan, feature(sanitize))]

pub mod engine;
pub mod event;
pub mod modulation;
pub mod module;
pub mod modules;
pub mod params;
pub mod plan;
pub mod seed;
pub mod segment;
pub mod swap;
pub mod voice;

pub use engine::{Engine, EngineStats};
pub use event::{Event, EventKind, NoteKey};
pub use module::{MAX_VOICES, PrepareCfg, SEG};
pub use params::ParamId;
pub use plan::{CompileError, Plan, PlanMap, compile};

/// The engine version stamped into patches authored on this build (`Patch::engine`) and into golden files. Bump it
/// when a DSP change alters output on purpose (the goldens change in the same commit).
pub const ENGINE_VERSION: &str = "0.1.0";
