# ADR-0001: Plugin shell — start on nice-plug, keep it thin

- Status: **Proposed** — confirm or change at Gate G0 ([phase-0.md, Task 3](../phase-0.md#task-3--shell-spike-on-nice-plug-34-days))
- Date: 2026-10-18 (supersedes the 2026-10-17 plan to start on clack + WebView)

## Context

- We ship CLAP first, VST3 second, AU later. The engine and session are plain Rust libraries; the shell only translates between a host and them.
- The UI is native Rust (ADR-0002), most likely egui on baseview.
- nih-plug is in maintenance mode. Its community successor **nice-plug** (ISC) provides CLAP + VST3 export, a standalone wrapper, polyphonic modulation, state persistence, an allocation-assert debug feature, and a maintained egui adapter. It calls itself experimental and has had large recent changes.
- **clack** offers safe, low-level wrappers for the whole CLAP API. With clack we'd write our own parameter handling, state, GUI embedding (baseview + egui bridge), timers and VST3 path (via clap-wrapper).
- Our host-parameter design is a fixed list (8 macros + 64 automation slots), which suits a framework with declared parameters. The open question is runtime slot *renaming*.

## Decision

Start the shell on **nice-plug**, pinned to an exact version or commit. Keep `crates/plugin` thin: no product logic, just mapping host params, notes, state and the editor onto `enth-session` and `enth-ui`.

Switch to **clack + clap-wrapper** if, within two days of trying, the Phase 0 shell spike can't achieve any of these must-haves: (a) 72 fixed host params with stable IDs, (c) persisting an arbitrary patch blob with the project, (e) VST3 export, (f) passing `clap-validator`, (g) building on stable Rust. Slot renaming (b) and poly-mod (d) failing are recorded but don't force a switch.

## Consequences

- Fastest path to a working CLAP/VST3 synth with an egui editor in REAPER.
- We inherit a young framework's churn; pinning and a thin shell limit the blast radius.
- If slot renaming isn't supported, slots show generic names ("Slot 7") — acceptable for v1.

## Alternatives considered

- **clack + own glue** — most control and full CLAP coverage; costs weeks of plumbing before the first sound. Kept as the escape hatch, and used anyway for `tools/test-host` (clack-host).
- **Raw clap-sys** (as in NovoNotes' WRAC template) — even lower level; no benefit over clack for us.
- **Original nih-plug** — maintenance mode; VST3 export via the GPLv3 `vst3-sys` crate.

## How we'd know we're wrong

Any must-have in the Phase 0 shell table fails, or framework churn costs more than a day per month after S1 (Instrument 0.1).
