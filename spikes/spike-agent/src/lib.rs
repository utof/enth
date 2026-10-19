//! Phase 0 lane P: the agent-loop probe (part 1). A CLI over the metric spike's toy synth, shaped like the real
//! `enth` CLI of `docs/agent-cli.md`: patch file in real units, `describe`, `check` with measured hints, `step`,
//! stable exit codes. See `docs/spikes/agent-probe.md`.
//!
//! | module | what |
//! | --- | --- |
//! | [`patch`] | the patch file, parameter table, `--set` parsing with units |
//! | [`synth`] | patch to 16-bit-quantised audio via `spike_metric::render` |
//! | [`descriptors`] | measurements from audio: envelope, spectrum, pitch, noise, beating, loudness |
//! | [`expect`] | the descriptor vocabulary and expectation lists |
//! | [`hints`] | finite-difference sensitivities and measured hints |
//! | [`commands`], [`guide`] | what each subcommand prints |
#![allow(
    clippy::needless_range_loop,
    reason = "numeric code indexes several arrays by one counter; iterator zips would obscure it"
)]

pub mod commands;
pub mod descriptors;
pub mod error;
pub mod expect;
pub mod guide;
pub mod hints;
pub mod patch;
pub mod synth;
pub mod units;
