# ADR-0001: Plugin shell — start on nice-plug, keep it thin

- Status: **Accepted (G0, 2026-10-19).** The Task 3 shell spike passed every hard must-have ([results](../spikes/shell.md)), and the test host and the UI spike found nothing against it. The editor on the real display, poly-mod in Bitwig and real audio are on the [owner checklist](../spikes/owner-checklist.md); a failure there that we can't fix in a day reopens this ADR (see the last section).
- Date: 2026-10-18 (supersedes the 2026-10-17 plan to start on clack + WebView); updated 2026-10-19 with the shell spike results; accepted at G0 on 2026-10-19

## Context

- We ship CLAP first, VST3 second, AU later. The engine and session are plain Rust libraries; the shell only translates between a host and them.
- The UI is native Rust (ADR-0002), most likely egui on baseview.
- nih-plug is in maintenance mode. Its community successor **nice-plug** (ISC) provides CLAP + VST3 export, a standalone wrapper, polyphonic modulation, state persistence, an allocation-assert debug feature, and a maintained egui adapter. It calls itself experimental and has had large recent changes. One person wrote about 89% of the commits since the fork and is the only crates.io owner, so the bus factor is 1; it has had breaking releases about monthly (the core 3 in 7 weeks, the egui adapter 5 in under 4 months). *(documented, researcher, 2026-10-19)*
- **clack** offers safe, low-level wrappers for the whole CLAP API. With clack we'd write our own parameter handling, state, GUI embedding (baseview + egui bridge), timers and VST3 path (via clap-wrapper).
- Our host-parameter design is a fixed list (8 macros + 64 automation slots), which suits a framework with declared parameters. The open question was runtime slot *renaming*.

## Decision

Start the shell on **nice-plug**, pinned to exact versions, with `Cargo.lock` committed:

| Crate | Pin | Brings |
| --- | --- | --- |
| `nice-plug` | `=0.4.2` | the core; CLAP and VST3 (`vst3` 0.3.0, MIT/Apache; no GPL `vst3-sys`) |
| `nice-plug-egui` | `=0.6.0` | egui 0.37.0, egui-baseview 0.9.0, baseview 0.4.1 |
| `nice-plug-xtask` | `=0.1.1` | the bundler used by `cargo xtask bundle` |

Keep `crates/plugin` thin: no product logic, just mapping host params, notes, state and the editor onto `enth-session` and `enth-ui`.

Switch to **clack + clap-wrapper** if, within two days of trying, the Phase 0 shell spike can't achieve any of these must-haves: (a) 72 fixed host params with stable IDs, (c) persisting an arbitrary patch blob with the project, (e) VST3 export, (f) passing `clap-validator`, (g) building on stable Rust. Slot renaming (b) and poly-mod (d) failing are recorded but don't force a switch.

**Outcome (2026-10-19):** (a), (c), (e), (f) and (g) passed on the first build, so the switch rule did not trigger. Details and numbers are in [docs/spikes/shell.md](../spikes/shell.md).

## Findings from the shell spike

- **(a), (c), (e), (f), (g) pass.** In native REAPER 7.82, headless: 72 params with stable ids (REAPER adds Bypass/Wet/Delta, so it reports 75), automation, a project save and reload in a new process restoring values, envelopes and the patch blob, CLAP and VST3. Also in Carla 2.5.8 (VST3). `clap-validator` 0.4.1: 36 passed, 0 failed, 8 skipped. Steinberg's VST3 validator (SDK 3.8.1): 537 of 537. Stable Rust 1.99.1, no nightly features. `cargo xtask reaper-test` repeats the REAPER checks as a local gate.
- **(b) Renaming works for CLAP with a small nice-plug patch; VST3 in REAPER can't.** Stock nice-plug 0.4.2 fixes parameter names at construction. A 13-file, +123/−4-line additive patch (a display-name override plus a main-thread CLAP `rescan(INFO)` and a VST3 `restartComponent(kParamTitlesChanged)`) makes REAPER show the new CLAP name in 50 ms. REAPER 7.82 rejects the VST3 title-change request, so VST3 in REAPER shows "Slot N" whatever we do. Carla's VST3 host does follow it. A lane that already exists keeps its old label but stays bound by id. The patch is not in the repo (it lives in the researcher's scratch); the open decision is to carry it in a small fork (RustAudio's AI-contribution policy rules out upstreaming code written by agents) or to accept "Slot N". Names must be derived from the patch on the main thread, after slot assignment and after state load.
- **(d) Poly-mod is declared correctly but untested in a host.** Macros are modulatable per note id and the plugin exposes `voice-info`. REAPER has no poly-mod UI; Bitwig is the reference host and is on the owner checklist.
- **(h) Second host:** Carla 2.5.8 is VST3-only (CLAP support exists only on its unreleased main branch); Ardour is VST3-only. Bitwig is the only CLAP and poly-mod host on Linux. Partly done; see the spike doc.
- **Two nice-plug pitfalls to design around:**
  1. `execute_gui` tasks may run **inline inside `process()`** when `process()` is on the thread nice-plug treats as main (Steinberg's validator does this), and then `assert_process_allocs` aborts on any allocation. Don't schedule main-thread work from `process()`, and make any task it does schedule real-time safe.
  2. **`NiceEguiApp::editor_closed()` is never called.** Don't rely on it; detect a reopen in `build()`.
- **Further findings:**
  - `VoiceID::id_or_fallback` in nice-plug-core 0.4.2 ignores the host's voice id; read `VoiceID::id()` and make our own fallback.
  - `assert_process_allocs` is gated on `debug_assertions`, so it checks only debug builds.
  - A `#[persist]` field of type `String` holding JSON is escaped twice in the state; persist a typed value.
  - REAPER's offline render restarts (deactivates, reactivates, resets) CLAP plugins about 30 ms after it starts, dropping a held note. Real-time playback isn't affected. Host-driven render tests must not depend on one long note.
  - A VST3 parameter set from a script takes effect only after a process cycle, so a script must wait before reading or saving.
- **Licences:** the dependency tree needed `BSD-1-Clause` (nice-assert-no-alloc), `Ubuntu-font-1.0` (egui's default fonts, scoped to one crate) and two `BSL-1.0` exceptions for Windows-only crates; see the 2026-10-19 note in [ADR-0004](0004-licensing.md).

## Note — 2026-10-19: accepted at G0

- **Test host** ([spikes/test-host.md](../spikes/test-host.md)): a minimal clack-host host renders spike-shell bit-identically from debug and release bundles (six plugin-level goldens, CI job `host-render`), and two or three instances render identically in four driving modes. The research report's second-instance difference in REAPER does not come from plugin state; REAPER's restart during an offline render is the likely cause (not proven). So plugin-level golden renders go through the test host, never through a REAPER offline render.
- **The reference panel inside the shell** (`spike-ui-plugin`, [spikes/ui.md](../spikes/ui.md)) passes clap-validator (36 / 0) with `assert_process_allocs` on while `process()` feeds the scopes, so the shell carries a real editor without breaking real-time rules.
- **Open, with the owner:** the editor on the real display (shell item i), poly-mod in Bitwig (d), play and automate in a second host (h), real audio and xruns, and the slot-rename decision (carry the patch in a fork or accept "Slot N"): [owner checklist](../spikes/owner-checklist.md).

## Consequences

- Fastest path to a working CLAP/VST3 synth with an egui editor in REAPER. The spike's shell is about 800 lines for 72 params, poly-mod ids, state and an 8-voice synth, and incremental rebuilds take 1–7 s.
- We inherit a young framework's churn and a single maintainer; pinning, a committed lockfile and a thin shell limit the blast radius. nice-plug-egui 0.7 is expected to move to a host timer API (baseview #357); budget about a day.
- Slots show generic names ("Slot 7") in VST3, and in CLAP unless we carry the rename patch; that is acceptable for v1.
- Tests that drive a real host stay local (`cargo xtask reaper-test`); CI runs `clap-validator` on the bundle.

## Alternatives considered

- **clack + own glue** — most control and full CLAP coverage; costs weeks of plumbing before the first sound. Kept as the escape hatch, and used anyway for `tools/test-host` (clack-host). The researcher's probe (clack 0.2.0's own polysynth example plus clap-wrapper 0.3.1 for VST3): builds in 30 s with 12 crates, but fails 3 clap-validator state tests, and the crates.io wrapper bundles clap-wrapper 0.14.0, which lacks the 0.16.0 fix for stale VST3 parameters after a state load (REAPER showed the default after a reload). The estimate for reaching parity on our narrow needs is 2–4 weeks.
- **Raw clap-sys** (as in NovoNotes' WRAC template) — even lower level; no benefit over clack for us.
- **Original nih-plug** — maintenance mode; VST3 export via the GPLv3 `vst3-sys` crate.

## How we'd know we're wrong

Any must-have in the Phase 0 shell table fails, or framework churn costs more than a day per month after S1 (Instrument 0.1). Also: the editor misbehaving on the real display (blank first open, wrong scale, lost keyboard focus) in a way we can't fix in a day, or the single maintainer leaving.
