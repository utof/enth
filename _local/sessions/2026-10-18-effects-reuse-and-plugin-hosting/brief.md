# Research brief — effects reuse and plugin hosting

- Date: 2026-10-18
- Milestone: P1 (minimal synth) — effects section
- Asked by: project owner ("Wouldn't it be cool if we could plug in some external reverb so that we don't have to build it?")

## Context

`enth` is an MIT-licensed modular synth in Rust, shipped first as a CLAP plugin (nice-plug shell, native egui UI), developed on Linux Mint + REAPER. The engine must be real-time safe and deterministic (same inputs → same samples) because scoring, lessons, the agent CLI and ML all re-render patches offline. Core crates must build for wasm32. The P1 minimal synth wants one or two effects: a reverb and a compressor.

## Questions

1. Which existing reverb implementations can we reuse instead of writing our own? Licence (must be MIT-compatible for code), sound quality reputation, real-time safety, determinism, wasm32, effort to port, repo health.
2. Which other effects/DSP libraries (compressor, delay, chorus, EQ, distortion; general DSP toolkits) can we reuse under the same constraints?
3. Can our plugin host third-party plugins (e.g. a user's favourite reverb) inside it? Precedents, Rust tooling, GUI embedding on Linux, threading/latency/state, crash isolation, and what it means for deterministic scoring and the CLI.

## Reports in this session (verbatim)

- `01-reverb-dsp-reuse.md`
- `02-effects-and-dsp-libraries.md`
- `03-hosting-third-party-plugins.md`
- `summary.md` — synthesis and recommendations
