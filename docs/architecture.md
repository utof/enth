# Architecture

**One deterministic engine library, many thin front doors.** The plugin, the CLI, the tests, ML training and a possible future website all call the same pure-Rust crates. Dependencies only point down, and the core crates (L0–L5) build for `wasm32-unknown-unknown`.

## The core idea

Every feature reduces to four functions over one data format:

```rust
render(patch, performance, spec)  -> Audio        // deterministic: same inputs, same samples
analyse(audio)                    -> Descriptors  // what it sounds like, in numbers and words
score(audio, target, metric)      -> ScoreReport  // versioned, loudness-normalised
solve(target, constraints, start) -> Vec<Patch>   // sensitivity + search now, ML later
```

A **Challenge** (target + starting patch + constraints + scoring spec + performance) is the single object behind a lesson step, a Recreate round, a daily challenge, a duel, "create from sound" and the agent loop.

![Five features share one Challenge and four engine functions](media/core-idea.svg)

## Crate layers

![Crate layers: dependencies point down; only the engine runs on the audio thread](media/crate-layers.svg)

| Layer | Crate (package name) | Responsibility | May depend on |
| --- | --- | --- | --- |
| L0 | `enth-dsp` | Oscillators (polyBLEP, wavetable), filters (SVF, ladder), envelopes, LFOs, smoothing, oversampling, effects (reverb, compressor; ported or in-house per [ADR-0006](adr/0006-effects-strategy.md)). No allocation in process paths | — |
| L0 | `enth-params` | `ParamSpec`, units, value mappings (plain ⇄ normalised), display/parse ("800Hz", "40ms") | — |
| L1 | `enth-patch` | Patch model, module catalogue (descriptors), **edit ops** (`SetParam`, `AddModule`, `Route`…) with inverses, serde, schema version + migrations, validate, canonicalise, hash | L0 |
| L2 | `enth-engine` | Module implementations, voice manager, Patch → `RenderPlan` compiler, real-time `process()`, plan swap | L0–L1 |
| L3 | `enth-render` | Offline deterministic render: (patch, performance, spec) → buffer | L0–L2 |
| L3 | `enth-analysis` | STFT/mel/MFCC, loudness, pitch, envelope and modulation estimates; **descriptors**; expectation checks; versioned distance metrics | L0 |
| L4 | `enth-challenge` | `Challenge`, `Constraints`, lessons, `ScoringSpec`; evaluates a score (render + analysis) | L0–L3 |
| L5 | `enth-solver` | Sensitivity (`suggest`, coach hints), CMA-ES (`match`), nearest-solution search; `onnx` feature later. Batch evaluation goes through a caller-supplied evaluator, so the solver itself spawns no threads | L0–L4 |
| L6 | `enth-session` | Owns the live patch on the main thread: applies ops, undo/redo, compiles plans, ships them to the audio thread, bridges host params | L0–L5 |
| L7 | `enth-ui` | Native editor (egui by default): `theme` (design tokens, [design-system.md](design-system.md)), widgets, module lanes, modulation rings, scopes, lesson and Recreate views | L0–L6 |
| L8 | `enth-plugin` | CLAP/VST3 shell: host params (macros + slots), state, notes, poly-mod, editor embedding | L0–L7 |
| L8 | `enth-cli` | The `enth` binary for humans, scripts and agents; supplies parallelism (e.g. rayon) to the solver | L0–L5 |
| L8 | `enth-standalone` | *(deferred)* desktop app | L0–L7 |
| L8 | `enth-wasm` | *(later, maybe)* browser build | L0–L7 |
| L8 | `enth-py` | *(later)* Python bindings for ML datasets | L0–L5 |

Tooling is outside the layer system and listed as exempt in `xtask/layers.toml`: `xtask`, `enth-test-host` (`tools/test-host`) and everything named `spike-*` (`spikes/`).

### Rules that keep it refactor-free

1. **Dependencies point down only.** `xtask/layers.toml` is the single source of truth for layers. `cargo xtask check-layers` reads `cargo metadata` and fails on an upward or same-layer edge, or on a workspace member that is neither layered nor exempt.
2. **Core crates (L0–L5) do no I/O**, spawn no threads, read no clocks, use no global RNG, and must build for `wasm32-unknown-unknown` (`cargo xtask wasm-check`). Parallelism and I/O are supplied by the caller.
3. **Only `enth-engine` runs on the audio thread.** Its `process()` never allocates, locks, logs, does I/O or panics.
4. **Descriptors in `enth-patch`, implementations in `enth-engine`.** The UI, challenges, solver and CLI reason about modules (names, ranges, units, tiers) without linking DSP. A test asserts every descriptor has an implementation and vice versa.
5. **Every change to a patch is an op.** The UI emits ops, the CLI applies ops, undo stores inverse ops, a future network duel would send ops. There is no second way to mutate a patch.
6. **Shells translate; they never decide.** No product rule lives in `enth-plugin` or `enth-cli` — it would be missing from the next shell.
7. **Everything persisted or sent is versioned** (patch, challenge, lesson, CLI JSON output) and has fixture tests.

## Threads and data flow inside the plugin

| Thread | Owns | Talks to others via |
| --- | --- | --- |
| Audio (host-owned) | `RenderPlan`, voices, smoothed param values | Receives plans and events from `rtrb` ring buffers; returns old plans for deallocation; writes scope/meter samples into a ring |
| Main / GUI | `enth-session` (live `Patch`, undo stack), `enth-ui` | Sends compiled plans + param events down; reads scope ring; host param changes arrive as events |
| Background worker | Plan compilation for big patches, analysis for scopes, solver runs | Results posted back to the main thread |

- **Value change** (knob, automation, macro): a parameter event, smoothed on the audio thread. No recompilation.
- **Structural change** (add module, add route): `enth-session` compiles a new `RenderPlan` off the audio thread, sends it through a ring buffer, the engine swaps it at a block boundary with a short crossfade, and sends the old plan back to be dropped.
- **Editor closed:** no timers, no repaint, no analysis. The UI costs nothing.

## Folder topology

Directory names are short; package names carry the `enth-` prefix (`crates/dsp` → `enth-dsp`).

```
enth/
├── Cargo.toml              # virtual workspace: members, [workspace.package], [workspace.lints]
├── rust-toolchain.toml     # pinned stable toolchain + wasm32 targets
├── .cargo/config.toml      # [alias] xtask = "run --package xtask --"
├── deny.toml               # cargo-deny: allowed licences, banned crates
├── LICENSE                 # MIT
├── THIRD_PARTY_NOTICES.md  # one entry per ported source: upstream, commit, licence, copyright
├── AGENTS.md               # agent rules (the single copy)
├── CLAUDE.md               # "Read AGENTS.md and docs/README.md before any task."
├── _local/sessions/        # research sessions: <YYYY-MM-DD>-<slug>/{brief,NN-topic,summary}.md (see research-process.md)
├── docs/                   # these docs; docs/adr/ decisions; docs/dsp/ one note per module; docs/spikes/ results
├── crates/
│   ├── dsp/                # L0
│   ├── params/             # L0
│   ├── patch/              # L1  src/{model,catalogue,ops,units,schema,migrate,canon}.rs
│   ├── engine/             # L2  src/modules/{generators,modulators,effects}/, voice.rs, plan.rs
│   ├── render/             # L3
│   ├── analysis/           # L3  src/{features,descriptors,expect,distance}.rs
│   ├── challenge/          # L4
│   ├── solver/             # L5  src/{sensitivity,cmaes,nearest,coach}.rs
│   ├── session/            # L6
│   ├── ui/                 # L7  src/{widgets,editor,scopes,learn,recreate}/
│   ├── plugin/             # L8  CLAP (+VST3) shell
│   ├── cli/                # L8  the `enth` binary
│   ├── standalone/         # L8  deferred — created when needed
│   ├── wasm/               # L8  later — created when needed
│   └── py/                 # L8  later — created when needed
├── content/
│   ├── presets/            # *.patch.json
│   ├── templates/          # constrained patch templates for games, ML and agent starts
│   ├── lessons/            # content packs: lesson.toml + markdown + patches; targets rendered on the fly
│   └── agent-evals/        # prompts + expectations + reference sounds (agent benchmark)
├── tests/golden/           # reference patches + expected render hashes and feature summaries
├── spikes/                 # Phase 0 throwaway code (spike-* packages, exempt from layers)
├── tools/
│   └── test-host/          # enth-test-host: tiny CLAP host (clack-host), scripted editor open/close, timings
├── ml/                     # later: Python (uv) training + ONNX export; never shipped
└── xtask/                  # layers.toml; commands: check-layers, wasm-check, bundle, install,
                            # validate, golden, bench, agent-eval (each added when first needed)
```

## Light and heavy builds: content packs and features

- **Lessons are data, not code.** A content pack is a folder (or archive) of `lesson.toml`, markdown and patches. Packs can be bundled, or downloaded on demand from the project's GitHub releases with a button — plain file downloads, no server of our own.
- **Views behind cargo features.** `enth-ui` puts the Learn and Recreate views behind features (`learn`, `recreate`), so a lean build is a flag, not a refactor.
- Nothing in the engine or patch format depends on whether those features are compiled in.

## Modules as their own plugins (ideal)

Kilohearts-style: an effect from the synth could also ship as its own small plugin. To keep this possible without committing to it:

- A module's DSP depends only on `enth-dsp` and its `ParamSpec`s — never on engine internals such as the voice manager or plan compiler.
- If it's ever wanted, module code moves into its own crates (`crates/modules/<name>`) below `enth-engine`, and a tiny generic shell wraps any one module as a CLAP effect.

## Adding a feature: the predictable path

A new oscillator, for example:

1. DSP primitive in `crates/dsp` with unit tests.
2. Descriptor (params, units, ranges, lesson metadata) in `crates/patch/src/catalogue`.
3. Implementation in `crates/engine/src/modules/generators`.
4. Golden render test in `tests/golden`.
5. Module card in `crates/ui`.
6. It's automatically available to the CLI (`enth modules`, `enth apply`) because the CLI reads the catalogue.
7. At least one lesson in `content/lessons` and one agent eval in `content/agent-evals`.

Nothing else changes.
