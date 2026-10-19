# enth — project docs

> **Working name:** `enth` (the repo folder). The final product name is still open — see [naming.md](naming.md). Crates use the `enth-` prefix; renaming later is a find-and-replace.
>
> **As of:** 2026-10-19 · **Status:** Phase 0 complete except the owner's checks ([spikes/summary.md](spikes/summary.md), [owner checklist](spikes/owner-checklist.md)). P1a (the headless core, [phase-1a.md](phase-1a.md)) is done except the owner's gates (the reverb A/B and the blind listen); its G1 table is [p1a-g1.md](p1a-g1.md). P1b (the plugin and the editor, [phase-1b.md](phase-1b.md)) is in progress: the research session is done, freezes F0b and F1b are reached, and lanes S0b, E+, UI-C, S1s, TH, FK and S2s are merged.

`enth` is an open-source (MIT) modular synthesizer written in Rust, shipped first as a **CLAP plugin**, built for its author and friends first. It combines ideas that usually live in separate products:

1. **Recreate** — the synth generates a random patch at the difficulty you choose; you rebuild it by ear and get a smart score (how close it *sounds*, how close your *patch* is, and how fast you got there).
2. **A deep modular synth** in the spirit of Phase Plant — generators, modulators, effect lanes, a modulation matrix, macros — that stays light on CPU, GPU and RAM. It starts Syntorial-sized and grows in parallel with the game.
3. **Learning by ear** in the spirit of Syntorial — lessons from "what is a saw wave" to advanced sound design, on the same instrument you keep using.
4. **Sound → patch** in the spirit of Synplant's Genopatch — give it a sound, get editable patches back, fully local.

It is also built to be **driven by AI agents through a CLI**: an LLM can create a patch, render it, read a structured description of how it sounds, and adjust parameters until it meets a goal.

![Five features share one Challenge and four engine functions](media/core-idea.svg)

## Read in this order

| Doc | What it answers |
| --- | --- |
| [vision.md](vision.md) | What we're building, for whom, and the principles that decide trade-offs |
| [research-process.md](research-process.md) | **The rule:** a deep-research session before every milestone, saved in `_local/sessions/` |
| [architecture.md](architecture.md) | Crates, layers, folder layout, and the rules that keep it refactor-free |
| [design.md](design.md) | The nine load-bearing designs: params, patch, engine, host params, determinism, scoring, knob distance, solver, randomizer |
| [session.md](session.md) | `enth-session` (L6): the one `Session` that owns the live patch, its API, the threading contract, plan delivery, the host bridge, undo and the saved state |
| [agent-cli.md](agent-cli.md) | The `enth` CLI that lets LLM agents (and humans and scripts) build patches with feedback |
| [roadmap.md](roadmap.md) | A minimal synth, then the Recreate and Synth tracks in parallel; gates; what *not* to build yet |
| [phase-1a.md](phase-1a.md) | The P1a (headless core) lanes: freezes, waves, ownership, acceptance, owner gates (done; the actual hours are recorded there) |
| [phase-1b.md](phase-1b.md) | **Start here when coding.** The P1b (plugin and editor) lanes: freezes, waves, ownership, acceptance, owner gates |
| [phase-0.md](phase-0.md) | The Phase 0 lanes and tasks, with acceptance criteria (done) |
| [spikes/summary.md](spikes/summary.md) | What Phase 0 measured and changed, the G0 results, and the owner's [checklist](spikes/owner-checklist.md) |
| [benchmarks.md](benchmarks.md) | How we measure UI stacks, the engine, and the agent loop — and the pass/fail thresholds |
| [research.md](research.md) | Prior art, papers, and evidence behind the decisions, with sources |
| [adr/](adr/README.md) | Architecture decision records (plugin shell, UI toolkit, agent CLI, licensing, patch format, effects) |
| [AGENTS.md](../AGENTS.md) | Rules for AI coding agents (repo root; the single copy) |
| [listening.md](listening.md) | The owner's listening tests: metric pairs, ABX, reverb A/B — commands, timings, what each decides |
| [design-system.md](design-system.md) | Design tokens and the minimal visual language of the editor |
| [naming.md](naming.md) | Product name candidates and mode names |

## Decisions at a glance

| Area | Decision | Status |
| --- | --- | --- |
| Process | Deep-research session before every milestone; sub-reports saved verbatim in `_local/sessions/<date>-<slug>/` | Decided — [research-process.md](research-process.md) |
| Roadmap shape | Minimal Syntorial-sized synth first, then Recreate and Synth tracks in parallel | Decided — [roadmap.md](roadmap.md) |
| Language & licence | Rust, MIT; dependencies limited to permissive licences | Decided — [ADR-0004](adr/0004-licensing.md) |
| Plugin format | CLAP first; VST3 second; AU later; standalone app deferred | Decided |
| Plugin shell | Start on `nice-plug`; keep it thin; `clack` is the escape hatch. Carried as a private vendored fork under `vendor/` (slot rename, `editor_closed`, baseview xkb fix, keyboard focus only while typing) | Accepted at G0 (2026-10-19); fork decided 2026-10-19 (P1b research) — [ADR-0001](adr/0001-plugin-shell.md) |
| UI | Native Rust UI, **no WebView**: egui rendered with glow, no second toolkit | Accepted at G0 (2026-10-19); real-display checks pending with the owner — [ADR-0002](adr/0002-ui-toolkit.md) |
| Agent interface | CLI first (`enth`), JSON output, real units; thin MCP wrapper later if wanted | Decided — [ADR-0003](adr/0003-agent-cli.md) |
| Patch format | JSON, plain values in real units, versioned with migrations | Decided — [ADR-0005](adr/0005-patch-format.md) |
| Effects | Port permissive DSP into our own deterministic modules (Signalsmith reverb, in-house compressor); global FX bypass; no third-party plugin hosting | Proposed — confirm at G1 — [ADR-0006](adr/0006-effects-strategy.md) |
| AI and network | Everything runs locally; no server-side compute. Duels later via shared seeds and peer-to-peer | Decided |
| Website | Not planned. Kept possible: core crates and egui both build for wasm32 | Deferred |

## Developer environment

- Laptop: Acer ConceptD 5, NVIDIA Quadro RTX 3000, Linux Mint (Cinnamon, X11). Versions and tool locations: [spikes/environment.md](spikes/environment.md).
- The repo lives on ext4 (`/home`), not on the NTFS/FUSE disk: file-metadata calls dropped from about 5 ms to about 3 µs.
- Main test host: **native REAPER 7.82 on Linux** (portable install, 60-day evaluation), driven headless for automated host tests (`cargo xtask reaper-test`). The Wine/Windows REAPER cannot load Linux plugins.
- Second hosts: **Carla 2.5.8** for VST3 (Ubuntu's build has no CLAP host); Ardour hosts VST3 only and is not installed. A **Bitwig trial** is only for the CLAP poly-mod check, which Bitwig alone supports.
- Automated GUI work runs under Xvfb with Mesa; real-display checks are the owner's ([AGENTS.md](../AGENTS.md)). The NVIDIA 595-series X driver crashed Xorg once on 2026-10-19, after short-lived X clients ran on the real display.
- Development is done largely with AI coding agents (Claude Code / Codex).

## How to use these docs with a coding agent

Start a session in the repo root and say: *"Read `docs/README.md`, `AGENTS.md`, `docs/research-process.md` and `docs/phase-1b.md`, then do P1b lane <name>."* Phase 0 and P1a are done (their lanes are in `phase-0.md` and `phase-1a.md`); the P1b research summary is `_local/sessions/2026-10-19-p1b-plugin-editor/summary.md`, so read it before saying go. The work runs as parallel lanes behind three freezes (see the lane table in `phase-1b.md`). Each lane has acceptance criteria; ask the agent to stop and report at the end of its lane.
