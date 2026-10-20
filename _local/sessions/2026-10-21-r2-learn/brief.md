# Research brief: R2 Learn

- **Date:** 2026-10-21
- **Milestone:** R2 Learn ([docs/roadmap.md](../../../docs/roadmap.md#r2--learn-ongoing)). Lessons as **content packs** (markdown + patches + challenges), loaded on demand: bundled, or downloaded from the project's GitHub releases with a button, so the plugin stays light. Nearest-solution knob feedback; auto-generated "deconstruct this preset" lessons; lessons from basics to advanced sound design.
- **Gate GR2:** the lessons cover every P1 module, and the owner and friends learn something new from them.
- **Asked by:** the owner's standing request to keep advancing milestones in parallel. Docs are suggestions; only Rust and a native, non-WebView UI are firm. The owner reads the summary at their own pace and steers.

## Where things stand (2026-10-21)

On main, all of P1 and R1 + S1 are built (owner gates pending):
- **The engine** (osc, noise, svf with drive, adsr, lfo, reverb, comp, voices, modulation, macros), deterministic across Linux, Windows, macOS arm64 and wasm32.
- **The session** (`enth-session`) owns the patch, undo, the host bridge, the Recreate round (`game`) and presets (`presets`).
- **The plugin** (`enth-plugin`, CLAP and VST3, nice-plug fork) and **the editor** (`enth-ui`, egui; layouts as data; drag-to-modulate, rings, scopes, a preset browser, add/remove modules).
- **Recreate:** challenge codes (`enth-challenge`), frozen generator v1, scorers `sound/1` and `structure/1`, a coach with four hint levels from measured sensitivity (`enth-solver`), match by CMA-ES, a results drawer with per-control rings, and the CLI (`enth suggest | match | explain | challenge | compare`).
- **Reserved hooks:** design.md §1 gives every parameter `ParamSpec.learn` (tier Basic/Intermediate/Advanced, tags, a plain description); roadmap.md says Learn content ships as data packs and the Learn and Recreate views sit behind cargo features.

## Constraints

Rust and a native non-WebView UI; permissive licences (MIT/Apache/BSD/ISC/Zlib/BSL-1.0; MPL-2.0 needs the owner's OK); real-time safety (nothing in Learn touches the audio thread); determinism (a lesson's challenge is a code; every machine builds the same target); design tokens only in UI code; shells translate, never decide; layers enforced (`cargo xtask check-layers`); wasm32 for core crates; Linux and REAPER first; a light build without Learn is a cargo feature; **no telemetry**: a download happens only on the user's click.

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | **Pedagogy and curriculum.** What makes people actually learn synthesis, from prior art and evidence: Syntorial, Ableton's Learning Synths, Synth Secrets, SoundGym, Melodics/Yousician-style practice loops, Duolingo-style spaced repetition, worked examples and fading, ear training. A curriculum that covers every P1 module (oscillator, noise, filter and drive, envelopes, LFO, reverb, compressor, voices and glide, modulation routes, macros) from basics to advanced sound design, with each lesson's shape (explain, hear, do, check). How lessons reuse Recreate rounds and the coach (nearest-solution knob feedback), "deconstruct this preset" lessons generated from factory presets, and how GR2's "learn something new" is measured with a handful of friends. | `01-pedagogy-curriculum.md` |
| 2 | **Content packs and delivery.** The pack format (markdown + patches + challenge codes + optional pre-rendered audio and images; a manifest; versioning against the engine and generator versions), markdown rendering in egui (e.g. egui_commonmark: version, licence, maturity checked today), bundled versus downloaded from GitHub releases (an HTTP client and TLS in a plugin: crates, licences, binary size, platform trust stores; user-initiated only), integrity (hashes, signatures), where packs live on disk per OS, wasm32, authoring tools (an `xtask lesson-check` that plays every lesson's checks through the real session, CI), content licences (CC-BY or similar for lessons, patches). | `02-content-packs.md` |
| 3 | **Architecture, code structure, risks and the lane plan.** Where Learn lives in the layer stack (a crate such as `enth-learn`, or session modules), the lesson state machine (steps, checks on session ops and measured audio, hints, progress saved per user, not per project), the editor's Learn view (step panel, highlighting and locking controls, overlays reused from Recreate), tests (kittest lessons completed by a scripted player; agent evals that complete lessons through the CLI), an LLM-assisted authoring pipeline whose lessons are verified by the solver, risks and unknowns, and a lane plan in waves (at most two heavy plugin builds at once), with freezes and acceptance per lane, and the owner's time. | `03-architecture-plan.md` |

## Rules

[docs/research-process.md](../../../docs/research-process.md) applies: date every claim and link it; verify licences from the actual file; mark claims *measured*, *documented* or *anecdotal*; say plainly when evidence is missing; reports of at most about 25 KB.

AGENTS.md also applies: Xvfb only; persistent scratch with `notes.md` (`~/code/enth-scratch/2026-10-21-r2/NN/`); `flock -s /tmp/enth-quiet.lock` for builds; **no personal details in any web or API request** (User-Agent `enth-research/0.1`).

Reports are saved verbatim and never edited.
