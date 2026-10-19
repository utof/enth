# Roadmap

**A minimal, Syntorial-sized synth first; then two tracks in parallel — Recreate (the game, learning, sound → patch) and Synth (depth and polish).** Every milestone starts with a deep-research session ([research-process.md](research-process.md)) before any code. Durations are rough guesses for one developer working with coding agents; the gates matter more than the dates. Thresholds marked *(set in P0)* are placeholders until the spikes produce real numbers.

![Roadmap: a minimal synth, then the Recreate and Synth tracks in parallel](media/roadmap.svg)

Changes from earlier plans:

- **2026-10-19:** effects are ported from permissive sources into our own deterministic modules (a reverb from Signalsmith Basics, an in-house compressor), with a global FX bypass; no third-party plugin hosting ([ADR-0006](adr/0006-effects-strategy.md)).
- **2026-10-18 (b):** a minimal synth comes first, then the Recreate and Synth tracks run in parallel; a research session is mandatory before each milestone; duels are serverless; sound → patch may take up to 5 minutes; ear-training games and modules-as-own-plugins are listed for later.
- **2026-10-18 (a):** native UI instead of WebView, so Phase 0 includes a UI-toolkit spike; the agent CLI moved earlier; the standalone app was deferred.

## P0 — Spikes and skeleton (3–4 weeks)

Research session: `_local/sessions/<date>-phase-0-spikes/`. Task list: [phase-0.md](phase-0.md).

- Workspace, CI (fmt, clippy, tests, `cargo deny`, wasm32 build of core crates, layer check), AGENTS.md at the repo root.
- **Shell spike:** a one-oscillator synth on `nice-plug`, loaded in REAPER and a second Linux host; `clap-validator` passes; VST3 export builds.
- **UI spike:** the same reference panel in **egui** (with repaint gating) and **vizia**, measured with the [benchmark protocol](benchmarks.md).
- **Engine spike:** offline render speed (× realtime per core) and determinism across Linux, Windows and wasm32.
- **Metric spike:** a 10-parameter template; three candidate distances; 30 pairs you rank by ear; CMA-ES recovers random targets.

**Gate G0:** plugin validates and runs in two hosts with no GUI-caused xruns; a UI toolkit meets the [UI thresholds](benchmarks.md#ui-thresholds) and is chosen; renders ≥ 100× realtime per core; the metric agrees with your ears on most pairs; CMA-ES reaches "sounds the same to me" on most targets in under a minute. ADR-0001 and ADR-0002 become *Accepted*.

## P1 — Minimal synth + CLI v0 (4–6 weeks)

A Syntorial-sized instrument, complete enough to make simple basses, pads, plucks and leads — and to be a fair target for the Recreate game.

- Parameter schema with units, Patch v1 with ops and migrations, `Module` trait, plan compiler and swap, voice manager (poly, mono, legato, glide).
- Modules: 2–3 oscillators (classic waveforms plus a few wavetables), noise, a multimode filter (LP/HP/BP), 2 envelopes, 1–2 LFOs, a small modulation matrix, macros.
- Effects: a reverb ported from Signalsmith Basics (MIT; fundsp is the fallback) and an in-house feed-forward compressor (after Julius O. Smith's Faust `compressor_mono`, STK-4.3), both zero-latency, deterministic modules in `enth-dsp`; a persistent global FX bypass; no third-party plugin hosting ([research.md §8](research.md#8-reusing-effects-and-hosting-plugins), [ADR-0006](adr/0006-effects-strategy.md); full reports in `_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/`).
- A functional native editor (clear, not polished), 72 host params (8 macros + 64 slots), state save/load.
- **CLI v0:** `schema`, `modules`, `params`, `new`, `apply`, `get`, `diff`, `render`, `describe`, `check`, `step`, `guide`, `bench`.

**Gate G1:** a 16-voice patch stays within the CPU budget *(set in P0)*; patches round-trip through JSON; golden tests pass on three OSes and wasm32, reverb and compressor included; the reverb was chosen by a blind, level-matched A/B; you can make a bass, pad, pluck and lead you like; an agent solves at least half of 10 starter text goals with `enth step` within 20 renders each.

## Track R — Recreate

### R1 — Recreate mode (3–4 weeks)

The game you most want: the synth makes a random patch, you recreate it, and you get a smart score.

- A **Recreate** button that turns the plugin into game mode (no separate app or onboarding).
- **Randomizer settings page** ([design.md §9](design.md#9-randomizer-and-difficulty)): how much each category is randomized, a musical/safe slider, and one difficulty knob that widens what gets randomized (oscillators → filter → envelopes → modulation → effects). Everything stays available — difficulty changes the challenge, it doesn't lock features.
- Instant level-matched A/B, a 0–100 score with sound %, structure % and combined %, time to pass, and coach hints from measured sensitivity.
- CLI: `compare`, `suggest`, `match`, `explain`; the `content/agent-evals` set and `cargo xtask agent-eval`.

**Gate GR1:** you and friends play 20 rounds each, mostly agree with the score, and want more; the agent's solve rate is recorded as a baseline.

### R2 — Learn (ongoing)

Lessons as **content packs** (markdown + patches + challenges) loaded on demand — bundled, or downloaded from the project's GitHub releases with a button — so the plugin stays light. Nearest-solution knob feedback; auto-generated "deconstruct this preset" lessons; lessons go from basics to advanced sound design.

**Gate GR2:** the lessons cover every P1 module, and you and friends learn something new from them.

### R3 — Sound → patch (6–10 weeks)

"Create from sound": heuristic start + CMA-ES, then learned proposals (ONNX + `tract`) returning several candidates. Fully local. **Budget: under 5 minutes per sound on your laptop; aim for about 60 seconds.**

**Gate GR3:** on held-out sounds from our own synth, model + search beats search alone at equal time; on real samples, a blind listening test prefers it; every result arrives in under 5 minutes.

### R4 — Ear-training games (later)

SoundGym-style games built on the same Challenge object: intervals, perfect-pitch drills, chord progressions, rhythm and syncopation, EQ A/B.

### R5 — Duels with friends (later)

No server-side compute. Thanks to determinism, a challenge is just a **seed + settings**, so both players generate the identical target and score locally.

1. **Async duels over any chat:** share a challenge code; each player sends back a result code that contains their patch and time, which the friend's copy re-renders to verify. No networking code at all.
2. **Live duels:** peer-to-peer, LAN first; over the internet only with a minimal rendezvous if ever needed.

## Track S — Synth

### S1 — Instrument 0.1 (4–6 weeks)

Polished editor (generator / modulator / effect lanes, drag-to-modulate with depth rings, scopes), preset browser, undo/redo, CLAP poly-mod on macros, VST3, Windows and macOS builds in CI.

**Gate GS1:** ten presets you'd actually use; no crashes or audio-thread allocations under `clap-validator` and a one-hour REAPER soak; the real editor still meets the UI thresholds.

### S2 — Depth (rolling)

Wavetable import/editing, FM/PM/AM between generators, unison, more effects (a "lush" reverb ported from CloudSeedCore, chorus/flanger/phaser, anti-aliased saturation, EQ, a safety limiter), multi-segment envelopes, random modulators, MPE, oversampling, SIMD voices. Each feature lands with its lessons (R2) and agent evals.

### S3 — Modules on their own (ideal)

Any effect (and maybe generator) exportable as its own small plugin, Kilohearts Snapin-style, without bloating the main synth. Kept possible by the module boundaries in [architecture.md](architecture.md#modules-as-their-own-plugins-ideal).

### S4 — Standalone app (deferred); web (maybe never)

Standalone: window + the same egui editor + `cpal` audio + `midir` MIDI. Web, only if wanted: engine in an AudioWorklet (wasm32) + egui compiled to wasm.

## Light and heavy builds (later, prepared now)

Learn content ships as data packs, not code; the Learn and Recreate views sit behind cargo features, so a lean build that leaves them out is a build flag, not a refactor.

## Don't build yet

| Until | Leave out | Why |
| --- | --- | --- |
| G0 | Any UI beyond the reference panel | The toolkit isn't chosen yet |
| G1 | Editor polish, AU/AAX, MPE, preset cloud, standalone app | The minimal synth must work first |
| GR1 | ML of any kind, networking, MCP server | The game and agent loop must work with plain search first |
| GR3 | Topology search, text-to-patch models | Templates must work before structure search pays off |
| GS1 | Hosting third-party plugins inside the synth | Costs weeks and breaks determinism, the CLI and preset sharing; the DAW's FX chain already provides it. Even then only a CLAP-only post-FX slot after the scoring tap; anything more needs a new ADR ([ADR-0006](adr/0006-effects-strategy.md)) |
| Never | Server-side compute or cloud AI; a differentiable PyTorch copy of the engine; plugdata/Pure Data in the core | Local-first; one engine of truth; GPL and a second runtime |
