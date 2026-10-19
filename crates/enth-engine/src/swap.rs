//! Plan swap: the rtrb command ring, the return ring and state carry-over by module id (P1a lane S2b). Empty in
//! S2a.
//!
//! Seams S2a left for this file (see `docs/dsp/engine.md`, "Seams"):
//!
//! - **Step 1** of `Engine::render_segment` polls the command ring: `Cmd::Plan(Box<Plan>)` and epoch-tagged value
//!   events, in one ring so an event cannot overtake the plan that defines it. Old plans go back on a return ring
//!   with a pending slot (a full ring keeps the plan and retries next segment; nothing is freed on the audio thread).
//! - **State carry**: every module of the new plan whose (`Slot::key`, kind) matches one in the old plan copies its
//!   state (`ModuleImpl::copy_state_from` / the role enums' `copy_state`, which never allocate); fresh voice modules
//!   get `start_voice` for each active voice, fresh global ones `reset` with `crate::seed::global_seed`. The voice
//!   manager and `NoteState` live in the `Engine`, not the plan, so held notes survive.
//! - Both plans run for 256 frames with an equal-power ramp for added or removed modules.
