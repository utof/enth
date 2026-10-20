# Research brief: R1 Recreate and S1 Instrument 0.1

- **Date:** 2026-10-20
- **Milestones:** R1 Recreate mode and S1 Instrument 0.1 ([docs/roadmap.md](../../../docs/roadmap.md)). The roadmap runs the Recreate and Synth tracks in parallel once P1 is done. This session plans both together, because they share the editor, the build and distribution work, and the owner's time.
- **Asked by:** the owner ("continue by working on the milestones as agreed, parallelizing where applicable"). Docs are suggestions; only Rust and a native, non-WebView UI are firm. The owner reads summaries at their own pace and steers; work proceeds meanwhile.

## Where things stand (2026-10-20)

P1 is done, except the owner's gates (`docs/p1b-g1.md`, `docs/p1b-owner.md`). On main:
- **The engine:** a deterministic engine with osc, noise, svf, adsr, lfo, reverb and comp, voices, modulation and lock-free plan swaps. Bit-exact across Linux, Windows, macOS arm64 and wasm32.
- **The session:** `enth-session` owns the patch, undo and the host bridge.
- **The plugin:** `enth-plugin`, CLAP and VST3 on a privately vendored nice-plug/baseview fork with 9 patches. It renders exactly like the CLI, passes clap-validator and Steinberg's validator, and has REAPER scenarios, leak gates and real-time numbers with about 2.5× headroom.
- **The editor:** in `enth-ui`. Layouts are JSON data; layout 1 is "lanes". It has drag-to-modulate, telemetry rings, scopes, the right-click menu and envelope sculpting.
- **Analysis and the CLI:** descriptors (incl. `tonal_pitch` and `filter_sweep`) and the `enth` agent CLI. Hints (`enth-solver`) and an agent-eval harness (eval v2): 30/30 given, 15/30 self-written at G1.
- **Metric v1** is multi-resolution log-mel L1 (ADR-0007, provisional). The owner's listening calibration is partly done: session 1 of 2.

Known gaps:
- the editor can't add or remove modules, so a fresh instance plays `Patch::init()` and owner build 2 works around it with a starter project;
- there's no preset browser;
- there are no Windows or macOS plugin builds for friends;
- the owner's Claude Design layout variants are pending, and layout variants become data files (lane LV).

## Goals

- **R1:** the synth makes a random patch, you recreate it by ear, and you get a smart score: sound %, structure % and combined %, time to pass, coach hints. Gate GR1: the owner and friends play 20 rounds each, mostly agree with the score, and want more; the agent's solve rate is the baseline. Prerequisite: a **friend build** they can run.
- **S1:** a polished editor (lanes, drag-to-modulate, rings, scopes), a preset browser, undo/redo, CLAP poly-mod on macros, VST3, Windows and macOS builds in CI. Gate GS1: ten presets the owner would use, no crashes or audio-thread allocations under clap-validator and a one-hour REAPER soak, and the real editor still meets the UI thresholds.

## Constraints

Fixed: Rust and a native non-WebView UI. Also:
- permissive licences;
- real-time safety;
- determinism: a challenge is a seed plus settings, and every machine regenerates the same target (design.md §9: target generation in parameter space only, pinned maths);
- design tokens only in UI code;
- shells translate, never decide;
- layers enforced (`cargo xtask check-layers`);
- wasm32 for core crates;
- Linux and REAPER first;
- a light build is possible (Learn and Recreate behind cargo features).

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | **The Recreate game.** Randomizer and difficulty (design.md §9 `RandomSpec`): per-parameter priors, "musical/safe", reachability, structure randomization. The challenge object and codes (seed + settings; reused by duels and ML sampling). The round flow in the plugin (Recreate button, Target ▶ / Mine ▶, A/B, timer, submit, results drawer). What makes such games fun and fair, from prior art: Syntorial, SoundGym, Synth Secrets-style exercises, "guess the patch" games, ear-training apps. Session and editor hooks already reserved (`Overlay`, `recreate` feature). | `01-recreate-game.md` |
| 2 | **Scoring and coaching.** Sound % (metric v1 and its calibration: what the owner's listening sessions must still provide, and what else, e.g. a per-descriptor breakdown, makes a score feel fair); structure % (patch-graph and parameter distance with equivalences: modules off, routes at 0, enharmonic parameter choices); combined %; level-matched instant A/B in a plugin; time to pass. Coach hints from measured sensitivity, reusing `enth-solver`. CLI `compare`, `suggest`, `match`, `explain`. Growing agent evals to about 30 goals plus reference sounds. The agent solve-rate baseline. | `02-scoring-coaching.md` |
| 3 | **Instrument 0.1: editor and content.** Adding and removing modules in the editor (structural ops exist in the session), a richer init patch, the preset browser (format, folders, tags, factory vs user, search), ten factory presets and who makes them, polishing the lanes layout. How the LV layout variants arrive (Claude Design handoff `docs/handoff/claude-design-layout.md`). How Recreate and Learn present in the editor (overlay, challenge bar, results). CLAP poly-mod on macros (Bitwig check). UI thresholds on the real display. | `03-instrument-editor.md` |
| 4 | **Platforms, builds and distribution.** Windows and macOS plugin builds in CI (GitHub Actions on a private repo: minutes cost, runners, arm64). Code signing and notarization, and whether friends can run unsigned builds. A standalone app through nice-plug's wrapper for friends without a CLAP/VST3 host. AU for Logic: clap-wrapper's AUv2, its licence and maturity. Which DAWs friends likely use and their CLAP/VST3 support today. The one-hour REAPER soak, RTSan or `assert_no_alloc` in CI, crash reporting without telemetry. | `04-platforms-distribution.md` |
| 5 | **Risks, tests and the combined lane plan.** Prior art and failure stories. The test strategy for game logic (determinism of targets across platforms, scoring stability). Owner and friend time. A lane plan for R1 + S1 in parallel waves (two heavy builds at once on this laptop, as in P1). Freezes, acceptance per lane, and what can start at once. | `05-risks-tests-plan.md` |

## Rules

docs/research-process.md applies:
- date every claim and link it;
- verify licences from the actual file;
- mark claims *measured*, *documented* or *anecdotal*;
- say plainly when evidence is missing.

AGENTS.md also applies:
- Xvfb only;
- persistent scratch with `notes.md` (`~/code/enth-scratch/2026-10-20-r1-s1/NN/`);
- `flock -s /tmp/enth-quiet.lock` for builds;
- **no personal details in any web or API request** (User-Agent `enth-research/0.1`).

Reports are saved verbatim and never edited.
