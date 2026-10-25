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
//! pass-throughs and silence); lanes D1, D2, FX1 and FX2 replace the stubs. Lane S2b added the voice manager's
//! modes ([`voice`]: stealing, mono, legato, glide, choke, voice-end reports), smoothing ([`params`]), the
//! modulation matrix with macros and the mod wheel ([`modulation`]) and the plan swap with state carry ([`swap`],
//! [`Engine::with_link`]). S2a's lane S0d appended its reserved kinds as stubs and two seam files, [`routing`]
//! (lane RTE: audio routes between generators) and [`stereo`] (lane SVP: the stereo voice path), so those lanes edit
//! different files.
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
pub mod routing;
pub mod seed;
pub mod segment;
pub mod stereo;
pub mod swap;
pub mod tap;
pub mod voice;

pub use engine::{CROSSFADE_FRAMES, Engine, EngineStats, MONITOR_BYPASS_FADE_MS};
pub use event::{Event, EventKind, NoteKey};
pub use modulation::{DEPTH_SMOOTHING_MS, POLY_MACROS};
pub use module::{MAX_VOICES, PrepareCfg, SEG};
pub use params::ParamId;
pub use plan::{CompileError, DepthId, Plan, PlanMap, compile};
pub use swap::{Link, LinkCapacity, MAX_PLANS_IN_FLIGHT, SendError};
pub use tap::{Tap, TapCapacity, TapFrame};
pub use voice::{NoteEnd, VoiceState};

/// The engine version stamped into patches authored on this build (`Patch::engine`) and into golden files. Bump it
/// when a DSP change alters output on purpose (the goldens change in the same commit).
pub const ENGINE_VERSION: &str = "0.3.0";
