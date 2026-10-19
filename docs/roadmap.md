# Roadmap

**A minimal, Syntorial-sized synth first; then two tracks in parallel — Recreate (the game, learning, sound → patch) and Synth (depth and polish).** Every milestone starts with a deep-research session ([research-process.md](research-process.md)) before any code. Durations are rough guesses for one developer working with coding agents; the gates matter more than the dates. On 2026-10-19 Phase 0 replaced the *(set in P0)* placeholders with measured numbers (the G0 result below; derivations in [spikes/summary.md](spikes/summary.md)).

![Roadmap: a minimal synth, then the Recreate and Synth tracks in parallel](media/roadmap.svg)

Changes from earlier plans:

- **2026-10-19 (d):** P1a research (`_local/sessions/2026-10-19-p1a-headless-core/summary.md`). P1a is buildable as planned, with five design corrections: seeds never come from the patch hash; the processing quantum is a 32-frame segment behind a FIFO, not "block 64"; the reverb needs no RNG; the probe's 15 of 15 was partly a descriptor bug (13 of 15 when re-judged); and P1a gains a minimal `enth-solver` (hints), `enth-golden` and the text-goal agent-eval harness. G1's CPU budget is re-set, because (c)'s 340 M instructions per 2 s render was too tight for the real module set; macOS goldens run on demand or weekly; "a bass, pad, pluck and lead you like" moves to P1b. The P1a lane plan is [phase-1a.md](phase-1a.md).
- **2026-10-19 (c):** Phase 0 closed ([spikes/summary.md](spikes/summary.md)). G0 passes on the automated tier; the owner's real-display and listening checks are pending ([spikes/owner-checklist.md](spikes/owner-checklist.md)). ADR-0001 and ADR-0002 are accepted. The editor's PSS growth per open fails its gate until baseview's keymap leak is fixed upstream. G1 gets a proposed 16-voice CPU budget and an agent-eval protocol from the agent-loop probe.
- **2026-10-19 (b):** Phase 0 research (`_local/sessions/2026-10-19-phase-0-spikes/summary.md`). Phase 0 runs as parallel lanes and takes about 2 weeks. The UI spike is egui with glow only (vizia and iced are out); renders are bit-exact across platforms; Gate G0 has numeric metric and search thresholds; P1 splits into P1a (headless core) and P1b (plugin and editor); GR1 needs a survey of friends' OS and DAW and a friend build; second hosts are Carla (VST3 only) and a Bitwig trial for poly-mod.
- **2026-10-19 (a):** effects are ported from permissive sources into our own deterministic modules (a reverb from Signalsmith Basics, an in-house compressor), with a global FX bypass; no third-party plugin hosting ([ADR-0006](adr/0006-effects-strategy.md)).
- **2026-10-18 (b):** a minimal synth comes first, then the Recreate and Synth tracks run in parallel; a research session is mandatory before each milestone; duels are serverless; sound → patch may take up to 5 minutes; ear-training games and modules-as-own-plugins are listed for later.
- **2026-10-18 (a):** native UI instead of WebView, so Phase 0 includes a UI-toolkit spike; the agent CLI moved earlier; the standalone app was deferred.

## P0 — Spikes and skeleton (done 2026-10-19, except owner checks)

Research session: `_local/sessions/2026-10-19-phase-0-spikes/`. Lanes and tasks: [phase-0.md](phase-0.md). Planned at about 2 weeks; the lanes ran in parallel and were all merged on 2026-10-19. Results: [spikes/summary.md](spikes/summary.md); what waits for the owner: [spikes/owner-checklist.md](spikes/owner-checklist.md).

- Workspace, CI (fmt, clippy, tests, `cargo deny`, wasm32 build of core crates, layer check), AGENTS.md at the repo root.
- **Shell spike:** a one-oscillator synth on `nice-plug`, loaded in native REAPER (CLAP and VST3) and in Carla (VST3 only; Ardour is VST3-only too); `clap-validator` passes; VST3 export builds. A Bitwig trial is only for the CLAP poly-mod check.
- **UI spike:** the reference panel in **egui** on glow (with repaint gating), measured with the [benchmark protocol](benchmarks.md) in two tiers: automated under Xvfb, and owner-attended on the real GPU. No second toolkit.
- **Engine spike:** offline render speed (× realtime per core) and bit-exact renders across Linux, Windows and wasm32, with golden hashes in CI.
- **Metric spike:** a 10-parameter template; three candidate distances; two sessions of 30 pairs you rate by ear, whenever you have time; CMA-ES recovers random targets.

**Gate G0:**

- The plugin validates and runs in two hosts (REAPER and Carla) with no GUI-caused xruns.
- egui meets the [UI thresholds](benchmarks.md#ui-thresholds), including the owner's real-display checks.
- Renders are ≥ 100× realtime per core, and the golden hashes are identical on the platforms that have run (Linux, wasm32, Windows; aarch64 if runners exist).
- **Metric:** the best distance reaches Spearman ρ ≥ 0.7 against your mean rating of 30 pairs, with the lower 95% confidence interval ≥ 0.4.
- **Search:** with 8 workers and at most 60 s of wall time per target, at least 14 of 20 targets are judged *same* in an ABX listen.
- ADR-0001 and ADR-0002 become *Accepted*.

Metric v1 is provisional: it is recalibrated by your listening sessions and again in R1. Nothing in P1a waits for those sessions. *(Thresholds: 2026-10-19, from the Phase 0 research; the toy-synth numbers so far suggest they are reachable.)*

**G0 result (2026-10-19).** Status is *passed*, *pending owner* (real display, listening, Bitwig) or *failed*. Automated GUI numbers ran under Xvfb with Mesa software rendering; the per-threshold UI table is in [benchmarks.md](benchmarks.md#ui-thresholds).

| Gate | Measured | Status | Source |
| --- | --- | --- | --- |
| The plugin validates | clap-validator 0.4.1: 36 passed, 0 failed, 8 skipped, for spike-shell and spike-ui-plugin, debug and release, locally and in CI. Steinberg's VST3 validator: 537 of 537 | Passed | [shell.md](spikes/shell.md), [ui.md](spikes/ui.md) |
| Runs in REAPER | REAPER 7.82, CLAP and VST3: 45 `reaper-test` checks (params, automation, save and reload, offline render, 5 editor opens), 7 of 7 runs | Passed | [shell.md](spikes/shell.md) |
| Runs in Carla | Carla 2.5.8 (VST3 only): loads, 72 params, values and a byte-identical state chunk after save and reload. Playing, automating and the editor not tried | Passed for load and state; play and automate pending owner | [shell.md](spikes/shell.md) |
| No GUI-caused xruns | Not measured: Dummy Audio only | Pending owner (real audio, [owner checklist](spikes/owner-checklist.md) C3) | [ui.md](spikes/ui.md) |
| egui meets the UI thresholds | Idle editor: 0 frames in 22 s, GUI thread 0.05–0.19 % of a core. Animating: 1.16–1.41 ms CPU per frame before the GPU (3.5–4.2 % at 30 fps), 28.6–29.6 fps for a 60 Hz request. Open: 53 ms median (test host), 44–67 ms (REAPER). Threads constant over 200 cycles. PSS growth per open/close: 76.9 KB above baseline (16.1 KB with a one-line baseview fix). HiDPI clipped and Space lost after a click in REAPER | Automated tier passed except the PSS growth and the host checklist (both baseview, **failed**); real-display rows pending owner | [benchmarks.md](benchmarks.md#ui-thresholds), [ui.md](spikes/ui.md), [test-host.md](spikes/test-host.md) |
| Renders ≥ 100× realtime per core | 1-voice template 1,993× best, 1,783× median; 8-voice P1-like patch with reverb 152× / 149× scalar, 231× / 217× with f32x8 lanes; 16 voices 129× / 122× scalar (load 6–10) | Passed | [engine.md](spikes/engine.md) |
| Golden hashes identical across platforms | 1,010 hashes (10 renders, 1,000 seed → patch) identical on Linux x86-64 (local and CI), Windows x86-64 (CI `engine-golden`, first green run 2026-10-19), and `wasm32-wasip1` under wasmtime 48.0.6 with and without simd128 (local and CI); also across debug, release, `target-cpu=native`, FMA, LTO, lanes and FTZ/DAZ | Passed; aarch64 pending (manual CI job: no free arm runners for a private repo) | [engine.md](spikes/engine.md) |
| Metric: ρ ≥ 0.7, lower 95 % CI ≥ 0.4 | No ratings yet; the `listen` tool is built and dry-run tested | Pending owner (2 × 15 min) | [metric.md](spikes/metric.md) |
| Search: ≥ 14 of 20 judged *same* by ABX, ≤ 60 s on 8 workers | Proxy only: distance A with a heuristic start reaches the strict proxy threshold on 17 of 20 (19 loose), median 3.5 s and worst 10.9 s per solve; 17 / 14 / 15 on three search seeds. No ABX tool yet | Pending owner (and an ABX tool) | [metric.md](spikes/metric.md) |
| ADR-0001 and ADR-0002 accepted | Accepted 2026-10-19; their real-display checks are on the owner checklist | Passed | [ADR-0001](adr/0001-plugin-shell.md), [ADR-0002](adr/0002-ui-toolkit.md) |

## P1 — Minimal synth + CLI v0 (4–6 weeks)

A Syntorial-sized instrument, complete enough to make simple basses, pads, plucks and leads — and to be a fair target for the Recreate game.

2026-10-19: P1 splits in two, so the riskiest part (editor and host integration) no longer sits at the end of one long build, and the fastest feedback loops (offline, through the CLI) come first.

- **P1a — headless core:** everything below except the editor bullet: schema, patch, engine, render, analysis descriptors, modules, effects and CLI v0. 2026-10-19: P1a also includes a minimal `enth-solver` (the multi-parameter hints behind `check` and `step`, which need render and analysis and so cannot live in the CLI), the `enth-golden` harness, and the text-goal agent-eval harness (`content/agent-evals` with 10 goals, `cargo xtask agent-eval`), because G1 needs them. Lanes, freezes and owner gates are in [phase-1a.md](phase-1a.md).
- **P1b — plugin and editor:** the shell, the 72 host params, state save/load and a functional editor (marked *(P1b)* below).

The effort figures are still unvalidated guesses; Phase 0 records actual against estimated days per lane. The P1a plan counts agent-hours instead (about 52–93 in total, with a critical path of about 18–35, plus about 1.5–2 h of your time; see [phase-1a.md](phase-1a.md)).

- Parameter schema with units, Patch v1 with ops and migrations, `Module` trait, plan compiler and swap, voice manager (poly, mono, legato, glide).
- Modules: 2–3 oscillators (classic waveforms plus a few wavetables), noise, a multimode filter (LP/HP/BP), 2 envelopes, 1–2 LFOs, a small modulation matrix, macros.
- Effects: a reverb ported from Signalsmith Basics (MIT; fundsp is the fallback) and an in-house feed-forward compressor (Julius O. Smith's Faust `compression_gain_mono` structure, STK-4.3, plus a Giannoulis 2012 soft knee and make-up gain, which JOS's function lacks), both zero-latency, deterministic modules in `enth-dsp`; a persistent global FX bypass; no third-party plugin hosting ([research.md §8](research.md#8-reusing-effects-and-hosting-plugins), [ADR-0006](adr/0006-effects-strategy.md); full reports in `_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/`).
- *(P1b)* A functional native editor (clear, not polished), 72 host params (8 macros + 64 slots), state save/load. Slot names refresh through a CLAP rescan (this needs a small carried nice-plug patch); VST3 in REAPER shows "Slot N" ([design.md §4](design.md#4-host-parameters-macros--automation-slots)).
- **CLI v0:** `schema`, `modules`, `params`, `new`, `apply`, `get`, `diff`, `render`, `describe`, `check`, `step`, `guide`, `bench`.

**Gate G1** (the headless items are checked at the end of P1a, the rest at the end of P1b): a 16-voice patch stays within the CPU budget; patches round-trip through JSON; golden tests pass on Linux, Windows and wasm32 on every push, and on macOS (on demand or weekly, and once before G1; GitHub bills macOS minutes at about 10× Linux on a private repo), reverb and compressor included; the reverb was chosen by a blind, level-matched A/B; an agent passes the text-goal evaluation below; and *(P1b, it needs the editor)* you can make a bass, pad, pluck and lead you like.

- **16-voice CPU budget** *(re-set 2026-10-19 by the P1a research, report 02; the Phase 0 proposal of 340 M instructions per 2 s render, about 3,540 per stereo frame, was derived from the spike's 146-instruction voice plus a 1.5× allowance and is [too tight for the real module set](spikes/summary.md#the-16-voice-cpu-budget-for-g1): the prototype needs about 610 M)*: a 16-voice P1 patch with every P1 module in use, reverb and compressor on, renders at **at least 10× realtime on one core** of this laptop, offline, and costs **at most 12,000 instructions per stereo frame** on the scalar path (a gungraun hard limit, re-based to the first product measurement plus 10 %); and its p99 `process()` time stays under 25 % of the 64-sample block deadline in the test host (P1b). The prototype measured 6,353 instructions per frame and 22.7× realtime (4.4 % of a core): a full voice is 229 instructions per sample, the reverb 2,574 per frame, the compressor about 160. Sub-budgets for diagnosis: voice ≤ 400 per sample, reverb ≤ 3,000 per frame, compressor ≤ 250 per frame.
- **Agent evaluation** (the Phase 0 probe showed the old wording, "half of 10 goals within 20 renders", passes trivially when the expectation list is given: 15 of 15 in 1–7 renders, [agent-probe.md](spikes/agent-probe.md#trial-2026-10-19)). Protocol, from the probe and risk #8 of the plan-risks report:
  - 10 starter text goals with their expectation lists given (the harness and the goals are built in P1a); an agent passes a goal when `enth check` passes within 20 renders;
  - 3 runs per goal; report the median renders, tokens and cost; name the model and the date;
  - **the gate:** at least half of the goals pass, and in a blind listen you judge at least 7 of the 10 final sounds to match their text;
  - a second, harder track where the agent writes its own expectations from the brief; reported, not gated, at G1;
  - `cargo xtask agent-eval` runs nightly or on demand, never as a merge gate (it needs API access, costs money and is not deterministic).

### P1a lane plan

2026-10-19, from the P1a research. The full plan, with per-lane ownership and acceptance, is [phase-1a.md](phase-1a.md).

- **A serial spine with three freezes.** S0 (workspace prep: every P1a dependency pinned, empty crates and files, xtask and CI stubs) gives **F0**. S1a (parameter and patch types with the whole P1a catalogue) gives **F1**, reviewed before anyone fans out. S2a (a walking skeleton: the `Module` roles, the segment grid, seeds, a stub per module kind, a reference saw, goldens on Linux, Windows and wasm) gives **F2**. After F2, spine types change only additively.
- **Six waves:** S0; then S1a, analysis and the reverb and compressor cores; then S1b and S2a; then voices, plan swap, oscillators, filter/envelope/LFO, the effect wrappers and CLI phase 1; then hints, CLI phase 2 and the listening tools; then the evals and the G1 table.
- At most 4 light lanes build at once. About 52–93 agent-hours in total, a critical path of about 18–35. Your time: about 1.5–2 h (a reverb A/B of 20–30 min, the G1 blind listen of 15–20 min, an optional skim of the parameter list).

## Track R — Recreate

### R1 — Recreate mode (3–4 weeks)

The game you most want: the synth makes a random patch, you recreate it, and you get a smart score.

- A **Recreate** button that turns the plugin into game mode (no separate app or onboarding).
- **Randomizer settings page** ([design.md §9](design.md#9-randomizer-and-difficulty)): how much each category is randomized, a musical/safe slider, and one difficulty knob that widens what gets randomized (oscillators → filter → envelopes → modulation → effects). Everything stays available — difficulty changes the challenge, it doesn't lock features.
- Instant level-matched A/B, a 0–100 score with sound %, structure % and combined %, time to pass, and coach hints from measured sensitivity.
- CLI: `compare`, `suggest`, `match`, `explain`; the `content/agent-evals` set grows to about 30 goals plus reference sounds (the harness and the first 10 goals land in P1a).
- **Prerequisites for the friends' round (GR1):** ask your friends now which OS and DAW they use (Ableton, Logic and Cubase had no CLAP support as of July 2026), then provide a **friend build**: a standalone app through nice-plug's wrapper and/or a Windows VST3 build. Windows and macOS builds are otherwise not planned before S1, and the standalone app not before S4. If that is too much, GR1 becomes owner-only and the friends join in S1.

**Gate GR1:** you and friends (on a build they can run) play 20 rounds each, mostly agree with the score, and want more; the agent's solve rate is recorded as a baseline.

### R2 — Learn (ongoing)

Lessons as **content packs** (markdown + patches + challenges) loaded on demand — bundled, or downloaded from the project's GitHub releases with a button — so the plugin stays light. Nearest-solution knob feedback; auto-generated "deconstruct this preset" lessons; lessons go from basics to advanced sound design.

**Gate GR2:** the lessons cover every P1 module, and you and friends learn something new from them.

### R3 — Sound → patch (6–10 weeks)

"Create from sound": heuristic start + CMA-ES, then learned proposals (ONNX + `tract`) returning several candidates. Fully local. **Budget: under 5 minutes per sound on your laptop; aim for about 60 seconds.**

**Robust to real-world audio** (owner, 2026-10-19). Users upload degraded sounds, so training renders go through realistic augmentation:
- codecs at low bitrates (MP3 128 kbps and below, AAC, Opus), resampling, low bit depth;
- clipping, limiting and heavy compression;
- room reverb, background noise and hum;
- EQ tilts (phone or laptop speakers), stereo-to-mono;
- trimmed, layered or mixed-in sounds;
- varied note, velocity and length.

Unlabelled real recordings train the audio side. The R3 research session covers augmentation design and its licences: codec encoders such as LAME are LGPL, which is acceptable in the never-shipped `ml/` pipeline but never in the product.

**Gate GR3:** on held-out sounds from our own synth, model + search beats search alone at equal time; on real samples, **including a held-out set of degraded uploads (low-bitrate MP3, noisy, reverberant, clipped)**, a blind listening test prefers it; every result arrives in under 5 minutes.

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
| G0 | Any UI beyond the reference panel | egui is confirmed only at G0, after the real-display and in-host checks |
| G1 | Editor polish, AU/AAX, MPE, preset cloud, standalone app | The minimal synth must work first |
| GR1 | ML of any kind, networking, MCP server | The game and agent loop must work with plain search first |
| GR3 | Topology search, text-to-patch models | Templates must work before structure search pays off |
| GS1 | Hosting third-party plugins inside the synth | Costs weeks and breaks determinism, the CLI and preset sharing; the DAW's FX chain already provides it. Even then only a CLAP-only post-FX slot after the scoring tap; anything more needs a new ADR ([ADR-0006](adr/0006-effects-strategy.md)) |
| Never | Server-side compute or cloud AI; a differentiable PyTorch copy of the engine; plugdata/Pure Data in the core | Local-first; one engine of truth; GPL and a second runtime |
