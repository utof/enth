# Research brief — P1a, headless synth core

- **Date:** 2026-10-19
- **Milestone:** P1a, the headless core ([docs/roadmap.md](../../../docs/roadmap.md#p1--minimal-synth--cli-v0-46-weeks)). It covers the parameter schema, Patch v1 with ops and migrations, the `Module` trait, the plan compiler and swap, the voice manager, modules and effects, analysis descriptors and CLI v0, ending in the headless half of Gate G1.
- **Asked by:** the owner ("continue by working on the milestones as agreed, parallelizing where applicable"). The owner reads summaries at their own pace and steers if they disagree; work proceeds meanwhile.

## Context

Phase 0 is done (see `docs/spikes/` and `_local/sessions/2026-10-19-phase-0-spikes/summary.md`):
- the workspace has a skeleton, CI, a layer check and a wasm check;
- nice-plug 0.4.2 works as the shell;
- `spike-engine` proved bit-exact renders across Linux, Windows and wasm, using the four DSP rules: `libm`, no `mul_add`, fixed summation order, denormals flushed in code;
- `spike-metric` has features, distances and an own CMA-ES;
- `spike-agent` showed agents reach expectation lists easily, but its descriptors break under detune beating and its hints are one parameter at a time;
- `enth-ui` has design tokens.

P1a turns spikes into product crates: `enth-params` (L0), `enth-dsp` (L0), `enth-patch` (L1), `enth-engine` (L2), `enth-render` and `enth-analysis` (L3), and `enth-cli` (L8). docs/design.md (§1–§9), docs/architecture.md and docs/agent-cli.md are the current design. The research should confirm or correct them with evidence, and make them concrete enough to code from.

**Fixed:** Rust and a native non-WebView UI. Everything else in the docs can change if the evidence says so.

## Constraints

- Permissive licences only (deny.toml). GPL, LGPL and AGPL projects (Vital, Surge XT, Dexed, …) are reference-only for *ideas*, never code.
- Core crates (L0–L5): no I/O, threads, clocks, locks or global RNG; they must build for wasm32. Audio path: no allocation.
- Determinism: renders are bit-exact across platforms (the four rules above). Block size is part of the render function, so the engine re-chunks onto a fixed internal block (64).
- Patches store plain values in real units; every mutation is an op with an inverse; persisted types are versioned with migrations.
- AI agents write most of the code, so prefer designs that are easy to test and hard to get subtly wrong.

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | Engine architecture: ParamSpec catalogue, Patch v1 model, ops/inverses/undo, migrations, canonicalise/hash, `Module` trait, plan compiler and lock-free swap, voice manager (poly/mono/legato/glide), modulation matrix and macros, smoothing | `01-engine-architecture.md` |
| 2 | DSP modules and ports: oscillators (polyBLEP, wavetables), noise, TPT SVF (LP/HP/BP), envelopes, LFOs, the Signalsmith reverb port, the in-house compressor, global FX bypass. All under the determinism rules, with CPU budgets and golden-test plans | `02-dsp-modules.md` |
| 3 | CLI v0 and descriptors: command set, JSON schemas and versioning, unit parsing, clap/schemars/insta choices, descriptor fixes from the probe (beat-aware envelope, multi-parameter hints, clipping, margins), the G1 agent-eval protocol | `03-cli-descriptors.md` |
| 4 | Risks, prior art and test strategy: what slowed comparable Rust synth engines, what people report (KVR, Rust Audio, GitHub issues), real-time Rust patterns that bite, and the test pyramid for P1a (goldens, property tests on ops, fuzzing patches, instruction budgets). Also a proposed P1a lane plan with dependencies | `04-risks-tests-plan.md` |

## Rules

As in docs/research-process.md:
- date every claim about versions or activity and link it;
- verify licences from the actual file;
- mark claims *measured*, *documented* or *anecdotal*;
- nothing time-sensitive from memory;
- say plainly when evidence is missing.

Machine rules from AGENTS.md also apply:
- never open windows on the real display (use Xvfb);
- persistent scratch with a running `notes.md`;
- `flock` for builds;
- at most 2 heavy builds at once.

Reports are saved verbatim and never edited; corrections go in `summary.md`.

## Reports in this session

- `01-engine-architecture.md` … `04-risks-tests-plan.md`
- `summary.md`: synthesis, decisions, the P1a lane plan
