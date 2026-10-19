# Research brief — P1b, plugin and editor

- **Date:** 2026-10-19
- **Milestone:** P1b, plugin and editor ([docs/roadmap.md](../../../docs/roadmap.md), [docs/phase-1a.md](../../../docs/phase-1a.md)).
- **Scope:** the plugin shell, the 72 host params (8 macros + 64 slots), state save/load, a functional native editor, and the plugin-side half of Gate G1. G1 includes "a bass, pad, pluck and lead you like", made through the editor.
- **Asked by:** the owner ("continue by working on the milestones as agreed, parallelizing where applicable"). The owner reads summaries at their own pace and steers if they disagree; work proceeds meanwhile.

## Context

P1a is done. On main: params, patch with ops/undo/migrations, a deterministic engine with real modules (osc, noise, svf, adsr, lfo, reverb, comp), voices and modulation, lock-free plan swap (`Engine::with_link` / `Link`), a monitor FX bypass, analysis, hints, the `enth` CLI, and listening tools. Phase 0 spikes cover:
- the shell: `spikes/spike-shell` on nice-plug 0.4.2 + nice-plug-egui 0.6.0;
- the UI: `crates/enth-ui` design tokens and widgets; `spikes/spike-ui` reference panel, direction "Lanes";
- a CLAP test host: `tools/test-host`;
- a headless REAPER harness: `cargo xtask reaper-test`.

The owner likes the visual design, but asked for **layout alternatives**; a Claude Design handoff is out (`docs/handoff/claude-design-layout.md`). Recent owner requests already built: a disabled module dims as a whole, and envelopes can be sculpted directly.

## Known issues to address

- **nice-plug pitfalls:** `execute_gui` tasks may run inline in `process()`; `editor_closed()` is never called. Slot renaming needs a 123-line add-only patch (CLAP only; VST3 in REAPER shows "Slot N").
- **baseview:** an editor leak of ~77 KB per open/close, mostly a missing `xkb_keymap_unref` (a one-line fix measured to 16 KB); a ≈ 30 fps cap; HiDPI clipping in REAPER; Space lost after clicking the editor.
- **NVIDIA:** the 595-series X driver crashed Xorg once during GL window churn. Agents never open windows on the real display (Xvfb only); real-display checks need the owner.
- **Performance:** plan swaps copy ~1.6 MB of reverb state on the audio thread; p99.9 `process()` is unmeasured; the test host has no editor measurements on a real GPU.
- REAPER restarts CLAP plugins ~30 ms into offline renders, so plugin-level goldens use the test host.

## Constraints

Fixed: Rust and a native non-WebView UI. Plus:
- permissive licences;
- real-time safety (no allocation, locks or I/O on the audio thread);
- determinism (CLI renders = plugin renders);
- design tokens only in UI code;
- shells translate, never decide.

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | Session and shell: an `enth-session` (L6) design, the `enth-plugin` (L8) mapping onto nice-plug, the nice-plug fork plan (rename patch, `editor_closed`, baseview xkb fix via `[patch]`), CLAP features, VST3 | `01-session-shell.md` |
| 2 | Editor: wiring `enth-ui` to the session; engine → UI feedback rings; repaint gating; drag-to-modulate and other interactions; layout flexibility for the coming variants; real-display risks | `02-editor.md` |
| 3 | Real-time in host: measuring p99.9 `process()`, plan-swap cost, multi-instance CPU, editor CPU/GPU on the owner's NVIDIA; test-host extensions; what to fix vs accept | `03-realtime-host.md` |
| 4 | Risks, prior art, tests and the P1b lane plan (as P1a's report 04) | `04-risks-tests-plan.md` |

## Rules

docs/research-process.md applies:
- date every claim and link it;
- verify licences from the actual file;
- mark claims *measured*, *documented* or *anecdotal*.

AGENTS.md also applies:
- Xvfb only; persistent scratch with `notes.md`; `flock` for builds;
- **no personal details in any web or API request** (User-Agent `enth-research/0.1`).

Reports are saved verbatim and never edited.
