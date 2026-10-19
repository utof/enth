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
| L0 | `enth-dsp` | Oscillators (polyBLEP, wavetable), filters (SVF, ladder), envelopes, LFOs, smoothing, oversampling, effects (reverb, compressor; ported or in-house per [ADR-0006](adr/0006-effects-strategy.md)); `math` (the only door to `libm`) and `rng` (SplitMix64, `derive`, `fnv1a64`). No allocation in process paths | — |
| L0 | `enth-params` | `ParamSpec` (kind, scale, flags), **units** (the unit tables and parser), value scales (plain ⇄ normalised, through pinned `libm`), display/parse ("800Hz", "40ms"); declared as `const` tables, checked at compile time | — |
| L1 | `enth-patch` | Patch model, module catalogue (descriptors), **edit ops** (`SetParam`, `AddModule`, `SetRoute`…) with inverses, the file layer (serde, schema version + migrations), validate, canonicalise, two hashes (exact and canonical), undo history | L0 |
| L2 | `enth-engine` | Module implementations (enum-dispatched by role), voice manager, Patch → `RenderPlan` compiler, the 32-frame segment loop and host FIFO, real-time `process()`, plan swap with state carry | L0–L1 |
| L3 | `enth-render` | Offline deterministic render: (patch, performance, spec) → buffer | L0–L2 |
| L3 | `enth-analysis` | STFT/mel/MFCC, loudness, pitch, envelope and modulation estimates; **descriptors**; expectation checks; versioned distance metrics | L0 |
| L4 | `enth-challenge` | `Challenge`, `Constraints`, lessons, `ScoringSpec`; evaluates a score (render + analysis) | L0–L3 |
| L5 | `enth-solver` | **P1a: `hint` only**, the multi-parameter fix behind `check` and `step` (it needs render and analysis, which are both L3, so it can live in neither of those crates nor in the CLI). Then sensitivity (`suggest`, coach hints), CMA-ES (`match`), nearest-solution search; `onnx` feature later. Batch evaluation goes through a caller-supplied evaluator, so the solver itself spawns no threads. The evaluator must be **order-stable**: a work queue that writes each result by its input index, so results are identical for any worker count (measured: 1 and 8 workers give the same checksum) | L0–L4 |
| L6 | `enth-session` | Owns the live patch on the main thread: applies ops, undo/redo, compiles plans, ships them to the audio thread, bridges host params. P1b (2026-10-19): one `Session`, a single-threaded state machine (`&mut self`; no locks, threads, clocks or I/O) that owns the `Patch`, the `History`, the engine `Link` and the `PlanMap` of the latest plan it sent. The op's `effect()` decides between a value event, a depth event, a macro event and a recompile. It also holds the slot and macro bridge (the "patch wins, write back only on a true difference" rule of [design.md §4](design.md#4-host-parameters-macros--automation-slots), slot names, rescan requests), state save and load, audition notes, and the read model, edit trait and telemetry reader that the editor uses. A standalone or wasm shell could reuse it; the plugin adds the mutex | L0–L5 |
| L7 | `enth-ui` | Native editor (egui by default): `theme` (design tokens, [design-system.md](design-system.md)), widgets, module lanes, modulation rings, scopes, lesson and Recreate views. P1b (2026-10-19): the whole editor lives here, as `layout` (a `LayoutSpec` of regions sized by tokens, loadable from JSON), `view` (the view-model built from the patch and the catalogue) and `editor` (regions, cards, interactions), because a separate editor crate would be same-layer and `check-layers` refuses that. Widgets emit intents; the editor turns them into ops through the session at frame end | L0–L6 |
| L8 | `enth-plugin` | CLAP/VST3 shell: host params (macros + slots), state, notes, poly-mod, editor embedding. P1b (2026-10-19): the 72 host params with permanent ids; host notes become engine events (voice ids from `VoiceID::id()`) and voice ends become `NOTE_END`; state is two persisted fields, the exact patch JSON and a versioned `enth` blob; the shell holds the one session mutex and embeds the editor. It translates and nothing more | L0–L7 |
| L8 | `enth-cli` | The `enth` binary for humans, scripts and agents; supplies parallelism (e.g. rayon) to the solver | L0–L5 |
| L8 | `enth-standalone` | *(deferred)* desktop app | L0–L7 |
| L8 | `enth-wasm` | *(later, maybe)* browser build | L0–L7 |
| L8 | `enth-py` | *(later)* Python bindings for ML datasets | L0–L5 |

Tooling is outside the layer system and listed as exempt in `xtask/layers.toml`: `xtask`, `enth-test-host` (`tools/test-host`), `enth-golden` (`tests/golden`) and everything named `spike-*` (`spikes/`). `enth-golden` is the golden-render harness, built in P1a: cases are text files read at run time (`tests/golden/cases/*.toml`, one per case), one hash per case valid on every platform, every mismatching case reported in one run with a summary diff, and `--update` refused under CI.

### Rules that keep it refactor-free

1. **Dependencies point down only.** `xtask/layers.toml` is the single source of truth for layers. `cargo xtask check-layers` reads `cargo metadata` and fails on an upward or same-layer edge, or on a workspace member that is neither layered nor exempt.
2. **Core crates (L0–L5) do no I/O**, spawn no threads, read no clocks, use no locks, use no global RNG, and must build for `wasm32-unknown-unknown` (`cargo xtask wasm-check`). Parallelism and I/O are supplied by the caller. `wasm-check` proves only that they *build*: a wasm32 build accepts `Instant::now`, `fs::read` and `thread::spawn`. The root `clippy.toml` (`disallowed-methods`, `disallowed-types`) is what bans them; shell crates and tooling opt out.
3. **Only `enth-engine` runs on the audio thread.** Its `process()` never allocates, locks, logs, does I/O or panics.
4. **Descriptors in `enth-patch`, implementations in `enth-engine`.** The UI, challenges, solver and CLI reason about modules (names, ranges, units, tiers) without linking DSP. `ModuleImpl::new` is an exhaustive `match` on `ModuleKind`, so a descriptor without an implementation is a compile error, not a failing test.
5. **Every change to a patch is an op.** The UI emits ops, the CLI applies ops, undo stores inverse ops, a future network duel would send ops. There is no second way to mutate a patch.
6. **Shells translate; they never decide.** No product rule lives in `enth-plugin` or `enth-cli` — it would be missing from the next shell.
7. **Everything persisted or sent is versioned** (patch, challenge, lesson, CLI JSON output) and has fixture tests.

## Threads and data flow inside the plugin

| Thread | Owns | Talks to others via |
| --- | --- | --- |
| Audio (host-owned) | `RenderPlan`, voices, smoothed param values | Receives plans and epoch-tagged value events in order from one `rtrb` command ring; returns old plans on a return ring that has a pending slot, so nothing is freed on this thread; writes the read-only telemetry tap (atomics: modulated values, modulator outputs, voices, peaks, a scope ring) |
| Host main | The plugin's `activate`, `deactivate`, state save and load, parameter-info queries; calls into `enth-session` (live `Patch`, undo stack, **plan compilation in P1**) | Locks the session mutex; sends compiled plans and value events down; drains the return ring |
| GUI (baseview's X11 thread in 0.4.1) | `enth-ui`, the editor's `ParamSetter` gestures | Locks the same session mutex; reads the telemetry tap; host param changes arrive as events |
| Background worker | Analysis for scopes, solver runs (plan compilation joins them only if P1b's measurement says P1 patches need it) | Results posted back to the main thread |

- **Value change** (knob, automation, macro): a parameter event, smoothed on the audio thread. No recompilation.
- **Structural change** (add module, add route): `enth-session` compiles a new `RenderPlan` on the main thread (P1 patches are small; P1b measures compile time), sends it through the command ring, the engine swaps it at a segment boundary, and sends the old plan back to be dropped. State is carried over for every module whose ID and kind survive (moved, not copied, at an instant swap); only added or removed modules crossfade ([design.md §3](design.md#3-engine-and-real-time-safety)). The `PlanMap` (address → parameter index) stays on the main thread, where an op becomes a value event or a recompile.
- **Editor closed:** no timers, no repaint, no analysis. The UI costs nothing.
- **Threading contract (P1b, 2026-10-19).** Lane S1s writes the full text in `docs/session.md`; the rules are:
  - The session is one mutex-wrapped value shared by the host main thread and the GUI thread. In baseview 0.4.1 these are different threads (until baseview PR #357 lands, which moves the X11 work to the main thread; the mutex stays correct either way). The audio thread never touches the session, and nothing is locked from `process()`.
  - The audio thread owns the `Engine`, a preallocated event buffer and the last host values it saw. Host automation reaches the engine there as slot events; the session folds it into the patch later, and only when it truly differs ([design.md §4](design.md#4-host-parameters-macros--automation-slots)).
  - Never call `GuiContext::set_state` from the editor while processing (nice-plug would load that state on the audio thread), and never schedule GUI work from `process()`.
  - Someone must drain the return ring when the editor is closed and no timer runs. S1s settles who; the proposal is the session itself, on every main-thread entry (an edit, a state save or load, `activate`).
  - The editor depends on a narrow session trait, not on the plugin, so kittest, the dev window and the plugin run the same editor code.

## Folder topology

Directory names match package names: `crates/enth-dsp` is the package `enth-dsp` (2026-10-19: the earlier short directory names, such as `crates/dsp`, were dropped).

```
enth/
├── Cargo.toml              # virtual workspace: members (crates/*, xtask, spikes/*, later tools/*), default-members, [workspace.package], [workspace.lints]; P1b adds [patch.crates-io] for vendor/ and excludes vendor/
├── rust-toolchain.toml     # pinned stable toolchain (1.99.1) + wasm32 targets
├── clippy.toml             # lint options: bans I/O, clocks, threads and locks in core crates
├── .cargo/config.toml      # [alias] xtask = "run --package xtask --"
├── .github/                # workflows/ci.yml, dependabot.yml
├── deny.toml               # cargo-deny: allowed licences, banned crates
├── LICENSE                 # MIT
├── THIRD_PARTY_NOTICES.md  # one entry per ported source: upstream, commit, licence, copyright
├── AGENTS.md               # agent rules (the single copy)
├── CLAUDE.md               # "Read AGENTS.md and docs/README.md before any task."
├── _local/sessions/        # research sessions: <YYYY-MM-DD>-<slug>/{brief,NN-topic,summary}.md (committed; see research-process.md)
├── docs/                   # these docs; docs/adr/ decisions; docs/dsp/ one note per module; docs/spikes/ results
├── crates/                 # directory name = package name
│   ├── enth-dsp/           # L0  src/{math,rng,…}.rs: the only libm door; one file per DSP core (osc, svf, reverb, …)
│   ├── enth-params/        # L0  src/{spec,units,scale,parse,display,macros}.rs (units live here)
│   ├── enth-patch/         # L1  src/{model,catalogue/,addr,validate,ops,wire,history,file,migrate,canon}.rs (the files today; S1b may add more)
│   ├── enth-engine/        # L2  src/{module,params,plan,voice,engine,link}.rs, modules/{generators,modulators,effects}/
│   ├── enth-render/        # L3
│   ├── enth-analysis/      # L3  src/{features,descriptors,expect,distance}.rs
│   ├── enth-challenge/     # L4
│   ├── enth-solver/        # L5  src/{hint,sensitivity,cmaes,nearest,coach}.rs (P1a: hint.rs only)
│   ├── enth-session/       # L6  the Session: patch, history, link, plan map, host-param bridge, state
│   ├── enth-ui/            # L7  src/{theme,widgets,layout,view,editor,scopes,learn,recreate}/
│   ├── enth-plugin/        # L8  CLAP (+VST3) shell
│   ├── enth-cli/           # L8  the `enth` binary
│   ├── enth-standalone/    # L8  deferred — created when needed
│   ├── enth-wasm/          # L8  later — created when needed
│   └── enth-py/            # L8  later — created when needed
├── content/
│   ├── presets/            # *.patch.json
│   ├── templates/          # constrained patch templates for games, ML and agent starts
│   ├── lessons/            # content packs: lesson.toml + markdown + patches; targets rendered on the fly
│   └── agent-evals/        # goals/*.toml + prompts (agent benchmark; P1a: 10 starter text goals, R1: more goals and reference sounds)
├── tests/golden/           # the `enth-golden` crate (it needs both L3 crates and `check-layers` rejects same-layer edges, so it is exempt): reference patches, expected hashes (one per case, valid on every platform; per-platform overrides only where a platform has not run yet) and feature summaries as text — no WAVs in git
├── spikes/                 # Phase 0 throwaway code (spike-* packages, exempt from layers)
├── tools/
│   └── test-host/          # enth-test-host: tiny CLAP host (clack-host), scripted editor open/close, timings
├── vendor/                 # P1b: nice-plug, nice-plug-core, nice-plug-egui, baseview (+ egui-baseview) from crates.io with our patches, behind [patch.crates-io]; not workspace members; edited only by the fork lane
├── ml/                     # later: Python (uv) training + ONNX export; never shipped
└── xtask/                  # layers.toml; commands: check-layers, wasm-check, bundle, install, reaper-test;
                            # P1a adds golden, bench and agent-eval; P1b adds validate and a --package option for bundle, host-render and reaper-test
```

## Vendored crates (P1b)

2026-10-19, from the P1b research. nice-plug, nice-plug-core, nice-plug-egui and baseview (and egui-baseview, for the focus patch) are carried in the repo under `vendor/`, not as a git fork. The patches and the reasons are in [ADR-0001](adr/0001-plugin-shell.md#note--2026-10-19-p1b-research-the-vendored-fork-and-the-nice-plug-pitfalls).

- **Behind `[patch.crates-io]`.** The workspace excludes `vendor/`, and each vendored crate keeps its version, so the exact `=` pins still match. The files come from the crates.io `.crate` files that `Cargo.lock` pinned; `vendor/README.md` records the versions, the sha256 sums, the patch list and the upstream status.
- **Why not a git fork.** A public fork would expose the project name (the repo is private), CI would depend on Codeberg being up, and `deny.toml` would need `allow-git`.
- **History.** A pristine-import commit, then one commit per patch, so a re-import after an upstream release is a rebase.
- **Ownership.** Only the fork lane (FK) edits `vendor/`. The coordinator edits `[patch.crates-io]` in one commit.
- **Lints.** Vendored path dependencies are not lint-capped. A warning that a future toolchain finds in them fails CI under `CARGO_BUILD_WARNINGS=deny` (clippy's `-D warnings` and `cargo fmt --check` skip them). Today they build without warnings on the pinned 1.99.1. Fix a new warning per crate, with an `allow` that carries a reason, as part of the toolchain bump.
- **Licences.** The nice-plug crates declare ISC but their packages carry no LICENSE file, so the ISC text is copied from the Codeberg tag. baseview and egui-baseview are `MIT OR Apache-2.0` with both files present. Each gets a `THIRD_PARTY_NOTICES.md` entry.

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

1. DSP primitive in `crates/enth-dsp` with unit tests.
2. Descriptor (params, units, ranges, lesson metadata) in `crates/enth-patch/src/catalogue`.
3. Implementation in `crates/enth-engine/src/modules/generators`.
4. Golden render test in `tests/golden`.
5. Module card in `crates/enth-ui`.
6. It's automatically available to the CLI (`enth modules`, `enth apply`) because the CLI reads the catalogue.
7. At least one lesson in `content/lessons` and one agent eval in `content/agent-evals`.

Nothing else changes.
