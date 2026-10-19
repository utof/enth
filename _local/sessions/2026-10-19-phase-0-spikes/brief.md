# Research brief — Phase 0 spikes

- Date: 2026-10-19
- Milestone: P0 (spikes and skeleton) — [docs/phase-0.md](../../../docs/phase-0.md) Task 0
- Asked by: project owner ("continue by working on the milestones as agreed, parallelizing where applicable")

## Context

`enth` is an MIT-licensed modular synth in Rust, shipped first as a CLAP plugin, with a Recreate game, lessons, sound → patch and an agent CLI built on one deterministic engine. Docs: `docs/README.md`, `vision.md`, `architecture.md`, `design.md`, `roadmap.md`, `phase-0.md`, `benchmarks.md`, `research.md`, `adr/`. The previous session (`_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/`) settled effects (ADR-0006) and left local checks open.

**What is fixed and what isn't.** The owner strongly prefers **Rust and a native, non-WebView UI**, for runtime speed and for fast AI + human coding, workflow and feedback. Everything else in the docs is a suggestion: milestones, their order, the shell, the UI toolkit, the gates and the tasks can all change if the evidence says so. Sub-researchers should say so plainly when it does.

## Constraints

- Permissive licences only (MIT, Apache-2.0, ISC, BSD, Zlib, Unicode-3.0, CC0; OFL/UFL for fonts). GPL/LGPL/AGPL are reference-only.
- Audio path: no allocation, locks, I/O or hidden RNG; deterministic renders (design.md §5).
- Core crates (L0–L5) build for `wasm32-unknown-unknown`.
- Linux + REAPER first; Windows and macOS later.
- Development is done largely by AI coding agents, so fast, reliable feedback loops matter as much as runtime speed.

## The owner's machine (checked 2026-10-19)

| Item | Value |
| --- | --- |
| Hardware | Acer ConceptD 5: 12 threads, 15 GiB RAM (about 2.5 GiB free while working), Intel UHD 630 + NVIDIA Quadro RTX 3000 Mobile |
| OS | Linux Mint 22.3, kernel 7.0.0, Cinnamon on X11, PipeWire 1.0.5 (PulseAudio shim) |
| GPU driver | `nvidia-smi` reports "Driver/library version mismatch" (NVML 595.99): kernel module and userspace differ, so a reboot is needed before any GPU measurement |
| Rust | rustup, rustc/cargo 1.95.0 (2026-04-14); only the `x86_64-unknown-linux-gnu` target |
| Cargo tools present | cargo-deny, cargo-nextest, cargo-insta, cargo-fuzz, cargo-mutants, cargo-sweep, just, typos, release-plz |
| Missing | mold, wild, lld, sccache, hyperfine, clap-validator, wasm32 target |
| Hosts | REAPER 7.80 only under Wine/Bottles (Windows build); **no native Linux REAPER**, Bitwig or Ardour |
| Other | clang, perf, uv, node 22, `gh` logged in |

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | Plugin shell today: nice-plug, clack, clap-wrapper, Linux host issues | `01-plugin-shell.md` |
| 2 | Native UI in plugins: egui, vizia, alternatives, UI testing | `02-native-ui.md` |
| 3 | Engine speed and determinism, plus local wasm32 builds | `03-engine-determinism.md` |
| 4 | Metric and search: analysis crates, CMA-ES, distances, listening test | `04-metric-search.md` |
| 5 | Workspace and CI | `05-workspace-ci.md` |
| 6 | Dev-loop speed for AI + human iteration, with local build timings | `06-dev-loop-speed.md` |
| 7 | Plan risks and unknown unknowns; parallel Phase 0 schedule | `07-plan-risks.md` |

Axes 6 and 7 are additions to the five listed in phase-0.md: 6 because the owner ranks iteration speed first, 7 because the docs are open to change.

## Rules

As in `docs/research-process.md`: date every claim about versions, releases, activity or popularity and link it; verify licences from the actual file; mark claims *measured*, *documented* or *anecdotal*; nothing time-sensitive from memory; say plainly when evidence is missing, and list blocked sources under Gaps. Reports are saved verbatim and never edited; corrections go in `summary.md`.

## Reports in this session (verbatim)

- `01-plugin-shell.md` … `07-plan-risks.md`
- `summary.md` — synthesis and recommendations
