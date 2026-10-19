# Phase 1b — the plugin and the editor

**Goal: a plugin you can play, and an editor you can make sounds with.** `enth-plugin` loads in REAPER (CLAP and VST3), exposes the 72 host parameters, saves and loads its state, and renders exactly like the CLI. A functional native editor, wired to the same session, lets you make "a bass, pad, pluck and lead you like". This is the second half of Gate G1 ([roadmap.md](roadmap.md#p1--minimal-synth--cli-v0-46-weeks)). P1a built the headless half ([phase-1a.md](phase-1a.md), [p1a-g1.md](p1a-g1.md)).

Size: about **27–51 agent-hours** in total, with a critical path of about **13.5–24**, plus 2–4 per layout variant later. Your own time is about **2.5–3.5 h**, plus 45 min if you try Bitwig. These are sized from what P1a actually took, about 18 agent-hours against an estimate of 52–93, and 7.3 h of wall time against a critical path estimated at 18–35 ([phase-1a.md](phase-1a.md#actual-against-estimated-hours)). They come out at 1.5–2.8× P1a's actual hours, because host and editor loops are slower and two heavy builds at a time serialise the work.

**Status (2026-10-20): the research is done, and freezes F0b, F1b and F2b are reached.** Lanes S0b (`29ed7b0`, F0b), E+ (`6e505b8`), UI-C (`d6fdabe`), S1s (`d9c4ed1`, F1b), TH (`9c185b3`), FK (on main through `7447f2f`, with no merge commit) S2s (`4bfffdb`) and SK (`5a0186a`, F2b) are merged; wave 4 (ED1, PL, RH) is in progress. Research session: `_local/sessions/2026-10-19-p1b-plugin-editor/` (`summary.md` for the decisions, reports 01–04 for the evidence and the numbers). The docs were updated to match on the same day: [design.md](design.md), [architecture.md](architecture.md), [benchmarks.md](benchmarks.md), [roadmap.md](roadmap.md), [ADR-0001](adr/0001-plugin-shell.md), [ADR-0002](adr/0002-ui-toolkit.md) and [AGENTS.md](../AGENTS.md). `docs/dsp/engine.md` followed with lane E+, and `docs/session.md` with S1s and S2s.

New in P1b: `enth-session` (L6) and `enth-plugin` (L8) are created, `enth-ui` (L7) grows its editor, and `vendor/` holds the fork of nice-plug and baseview. The Phase 0 spike crates (`spike-shell`, `spike-ui`, `spike-ui-plugin`) stay frozen as references until lane INT removes them from the workspace.

## What the research changed

Lanes should read these before they read the table, because each one changes what a lane builds.

- **The plugin renders exactly like the CLI.** In a nice-plug shell around `enth-engine`, 20 of 20 runs were bit-identical to `enth-render`: 44.1 and 48 kHz, host blocks of 1 to 4096 frames, with a slot automation point. It needs nice-plug's `SAMPLE_ACCURATE_AUTOMATION = true` (with `false`, block 4096 differed from frame 20,480) and the fixed default seed ([design.md §5](design.md#5-determinism)).
- **The engine needs seams first, so lane E+ sits in the spine.**
  - Host automation must not be looked up in the shell. A plan swap happens inside a block, so a parameter id taken from the old plan can hit another parameter of the new one (a bug found in the code). The fix is slot events resolved through a slot table compiled into each plan, and parameter events that carry an epoch.
  - A depth event, so a modulation-depth drag does not recompile the plan on every frame.
  - Per-voice macro offsets for poly-mod.
  - A read-only telemetry tap: modulated values for the knob rings, modulator outputs, voice count and scope.
  - `reset()` and `position()`, an audition-note command, the 20 ms monitor-bypass fade that [ADR-0006](adr/0006-effects-strategy.md) already specifies, and macro and mod-wheel events in `Performance`.
  - **Reverb state is moved, not copied, at a plan swap.** Copying 1.6 MB made a swap block take 0.8–1.06 ms, 75–79 % of a 64-frame deadline. A prototype that moves the delay lines brings an instant swap to 149 µs. A swap that adds or removes a module still copies, because both plans run side by side for 256 frames ([benchmarks.md](benchmarks.md#real-time-measurement-p1b)).
- **The fork is on by default, and it is vendored privately.** The vendored copy lives in the repo under `vendor/` behind `[patch.crates-io]`. A public git fork would expose the project name. All the patches apply and build ([ADR-0001](adr/0001-plugin-shell.md#note--2026-10-19-p1b-research-the-vendored-fork-and-the-nice-plug-pitfalls)); the release build of pristine nice-plug warns, which lane FK's lint cap keeps out of CI (its note in the same ADR).
- **One single-threaded `Session` owns the patch.** It holds the `Patch`, the `History`, the engine `Link` and the `PlanMap`. The plugin wraps it in one mutex shared by the host's main thread and baseview's GUI thread. The audio thread never touches it. The patch wins at load, and a host value is written back only when it truly differs ([design.md §4](design.md#4-host-parameters-macros--automation-slots)).
- **The editor makes layout data.** A `LayoutSpec` (regions sized by tokens) plus stable widget ids keyed by (module id, parameter address) kept 59 of 59 controls stable across two layouts, against 0 of 59 today. Each Claude Design variant becomes one value and two snapshots ([ADR-0002](adr/0002-ui-toolkit.md#note--2026-10-19-p1b-research-layout-as-data-stable-ids-keyboard-focus)).
- **Real-time measurement needs real-time priority.** rtkit grants `SCHED_RR` once `RLIMIT_RTTIME` is lowered to 200 ms. Deadline misses dropped from 980–2,299 to 3 of 45,000 blocks, and the misses were the scheduler's, not the plugin's. G1 gates on p99 and p99.9 under RR, and on swap blocks. Max is laptop noise.
- **The cheapest strong test is the engine's own goldens.** Render the existing golden cases through the plugin in the test host and assert the engine's hashes. That proves CLI = plugin with no new hash files.

## Freezes

The spine is serial and has three freeze points. Lanes fan out after each. **After F2b, session, engine and plugin types change only additively.**

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0b** | S0b: P1b dependencies pinned, empty crates and files, xtask and CI stubs, the two gate scripts | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)), judged by exit status |
| **F1b** | S1s: the `enth-session` API. E+ merged | A judge agent (Opus) reviews the session API and E+'s engine additions together |
| **F2b** | SK: plugin → session → engine end to end, 72 parameters, state round trip, one real card in the editor | Three engine goldens equal through the plugin; clap-validator with 0 failures (debug and release); `reaper-test --package enth-plugin` green |

The coordinator alone edits `[workspace.dependencies]`, `Cargo.lock` and `[patch.crates-io]` after S0b. A lane that needs a dependency asks for it, and FK's patch block goes in as one coordinator commit.

## Lanes

Sizes are agent-hours (estimates). H = heavy build (nice-plug, egui or clack-host), L = light. "Owner" is your time.

| Lane | Owns | Depends on | Size | Owner | State (2026-10-19) |
| --- | --- | --- | --- | --- | --- |
| **S0b** Workspace prep (→ F0b; H once) | Root `Cargo.toml` and `Cargo.lock` with every P1b dependency pinned (nice-plug `=0.4.2`, nice-plug-egui `=0.6.0`, egui `=0.37.0`, the clack crates the test host already uses); the fork's `[patch.crates-io]` block, commented out; empty `enth-session` (L6) and `enth-plugin` (L8, `cdylib` + `lib`); pre-declared module files (`enth-session`: model, apply, bridge, state, link, telemetry; `enth-plugin`: params, process, notes, state, editor; `enth-ui/src/editor/`: regions, layout, cards); theme tokens split into per-area files; `--package` for `bundle`, `host-render` and `reaper-test`; `validate` (pinned clap-validator 0.4.1, sha256-checked); CI jobs; `gate.sh lane` and `gate.sh full` | P1a main | 1–2 | No | Merged (`29ed7b0`, F0b) |
| **E+** Engine seams (L) | `enth-engine` link, command and telemetry additions; `enth-render` `Performance` macro and mod-wheel events. The list is under "What the research changed". No other P1b lane edits `enth-engine` or `enth-render` | P1a main (beside S0b) | 2–4 | No | Merged (`6e505b8`) |
| **S1s** Session types (→ F1b; L) | `enth-session` public types and traits: session, effects, history glue, slot and macro bridge, state blob v1 with fixtures, the editor's read model and edit trait, telemetry reader types, the threading contract; `docs/session.md` | F0b | 1.5–3 | No | Merged (`d9c4ed1`, F1b) |
| **UI-C** Catalogue cards (H) | `enth-ui` cards for every catalogue kind, built from `ParamSpec`s; stable ids keyed by (module id, parameter address), so `Knob` takes an id; accumulation for sub-step drags on discrete knobs; ADSR handles on real scales; region components with no layout; the token gallery; the `enth-params` parser fix for "−" (U+2212), which the design system displays (additive, flagged in the report) | F0b (the P1a catalogue is frozen) | 2–4 | No | Merged (`d6fdabe`) |
| **TH** Test-host extensions (H) | `tools/test-host`: `--state` from a patch file, golden-case input (`cli-equal`), instance churn, event bursts, `NOTE_END` checks; `process-stats` (rtkit `SCHED_RR`, a timing histogram, `--editor` under Xvfb only, `--instances N --threads T`, an environment snapshot in the JSON); knob → sound latency; if time allows, a pinned-nightly RTSan-built host | F0b | 2–4 | No | Merged (`9c185b3`) |
| **FK** Vendored fork (H) | `vendor/`: nice-plug, nice-plug-core, nice-plug-egui and baseview from the crates.io `.crate` files (egui-baseview joins them for the focus patch), a pristine-import commit, then one commit per patch: slot rename, `editor_closed`, baseview's xkb unref, keyboard focus only while typing, and a fifth that lane TH's note-loss finding added (a note-on listed ahead of a parameter change on the same frame was lost with `SAMPLE_ACCURATE_AUTOMATION = true`); the `[patch.crates-io]` entries; the lint cap (`[lints.rust] warnings = "allow"` in each vendored manifest); `vendor/README.md` (versions, sha256, patch list, upstream status); `THIRD_PARTY_NOTICES.md` entries; measured results into the ADR-0001/0002 notes | F0b | 1–2 (the focus patch is new since the sizing) | No | On main (through `7447f2f`) |
| **SK** Walking skeleton (→ F2b; H) | Minimal session implementation (value routing, initial compile, state save and load); `enth-plugin` (72 parameters, notes, process through the link, persist, rescan after load, one card in the editor) | F1b, E+ | 3–5 | No | Merged (`5a0186a`, F2b) |
| **S2s** Session complete (L) | Structural ops → recompile and swap; undo and redo with gestures; the slot and macro bridge with names and rescan requests; state versions and migrations; the host-state-load history policy | F1b (finish after F2b) | 2–3 | No | Merged (`4bfffdb`) |
| **PL** Plugin complete (H) | Note tuple, `NOTE_END`, voice-info, macro poly-mod with host voice ids (not `id_or_fallback`), the monitor FX bypass control, slot names through the fork (VST3 in REAPER stays "Slot N"), VST3 parity, `INPUT_EVENT_CAPACITY`, reset and reactivate, the plugin-side `rt-stats` histogram (a cargo feature) | F2b, FK | 2–4 | No | In progress |
| **ED1** Editor wiring (H) | The editor from UI-C components bound to the session: macro bar, voice and master, header Undo and Redo buttons, the slot "Automate" menu, right-click "Type value…" with units, the status line, envelope sculpting, disabled dimming; **the layout seam** with "Lanes" as layout 1; a no-op `Overlay` and the reserved lesson, challenge-bar and results regions behind cargo features; the dev window | F2b, UI-C | 3–6 | No | In progress |
| **ED2** Modulation and feedback (H) | Drag-to-modulate on egui drag-and-drop, depth arcs, the matrix overview, scopes, meters and LFO tiles from telemetry, repaint gating | ED1, S2s, E+ | 3–5 | No | Not started |
| **RH** REAPER harness (L, uses the bundle cache) | `xtask/reaper/*.lua`, `xtask/src/reaper.rs` scenarios | F2b | 1.5–3 | No | Merged (`7f77ed6`) |
| **RT** Real time in host (H) | The p99 and p99.9 measurement, the swap-block condition and multi-instance CPU on the G1 patch; if time allows, the private PipeWire daemon harness for xruns | PL, E+ | 1–3 | No | Not started |
| **INT** Integration and G1 (H) | Stale docs, this page's status, owner builds and sheet, `editor-cycles` and churn gates on `enth-plugin`, retiring the spike crates from the workspace, `content/presets/`, the P1b G1 rows | Everything | 2–3 | Owner builds | Not started |
| **LV** Layout variant (H; later, one per variant) | `enth-ui/src/editor/layout/<variant>.rs` and its snapshots | ED1's seam, the owner's pick | 2–4 each | The pick | Not started |

**Total: about 27–51 agent-hours**, plus 2–4 per layout variant.

## Waves

Two heavy slots, and every heavy lane is paired with a light one.

| Wave | Heavy slots | Light lanes | Starts when |
| --- | --- | --- | --- |
| 1 | S0b | E+ | — |
| 2 | UI-C, TH | S1s, E+ (finishing) | F0b |
| 3 | SK, FK | S2s | F1b and E+ merged |
| 4 | ED1, PL | RH, S2s (finishing); **owner build 1** (real display) | F2b; FK merged for PL |
| 5 | ED2, RT (after PL) | — | ED1, S2s merged |
| 6 | INT | — | All merged; **owner build 2**; owner gates |
| Later | LV | — | The owner's layout pick |

**Critical path:** S0b (1–2) → S1s (1.5–3) → SK (3–5) → ED1 (3–6) → ED2 (3–5) → INT (2–3), about **13.5–24 agent-hours**. The owner's gates come on top, and they usually set the wall-clock end.

## Acceptance per lane

Each lane stops at its own criteria and reports against them.

| Lane | Acceptance |
| --- | --- |
| **S0b** | The Done-means gate is green. `xtask validate` and `host-render --package spike-shell` work unchanged. A probe that adds a dependency in a lane fails `--locked` |
| **E+** | All 72 goldens unchanged. Parameter events carry an epoch and stale ones are ignored. Slot events resolve through the plan's slot table, also across a plan swap. A depth event equals a recompiled render once smoothing settles. Per-voice macro offsets (poly-mod) are tested. The telemetry tap is read-only (renders unchanged) and its ring overflow is tested under `assert_no_alloc`. `reset()` equals a fresh engine; `position()` exists. The audition-note command, the 20 ms monitor-bypass fade, and macro and mod-wheel events in `Performance` are tested. `NOTE_END` offsets are handled (the plugin clamps them into the block as well). Reverb state is moved at an instant swap: a test shows it moves rather than copies, and the paced swap block under RR drops from p99 797 µs (max 997) to p99 139 µs (max 149). The copy stays for a crossfading swap |
| **S1s** | Compiles with stubs. A state blob v1 fixture. The threading contract is written: who calls what on which thread, and who drains the return ring when the editor is closed. A judge agent reviews it |
| **UI-C** | Component snapshots, one PNG each. Interaction tests: a slow drag moves a discrete knob, coincident envelope handles pick the expected one, ids stay stable across a re-layout. The contrast and no-raw-values tests pass |
| **TH** | Each mode proven on spike-shell with a negative control (a leaky or unequal plugin is caught). `process-stats` reports p99 and p99.9 under rtkit RR and records the environment |
| **FK** | `editor-cycles`: `editor_closed` calls equal the number of cycles, growth at most 26 KB per cycle (16.1 KB measured with the xkb fix; lane FK's four runs gave 14.8 to 18.3 KB), threads constant. A CLAP rename reaches the host: **checked with a scratch CLAP host, not REAPER** (`vendor/tests/rename-check/`: a clack host that processes on its own thread and sees the callback request, one `rescan(INFO \| TEXT)` and the new name, with a control that sees none of them; `enth-test-host` cannot do it because its `rescan` handler does nothing). The Phase 0 spike showed the same patch renaming in REAPER, and [owner checklist](spikes/owner-checklist.md) C6 repeats it by hand. A text field takes keyboard focus and gives it back (11 of 11 XTest checks under Xvfb; REAPER's SWELL windows are owner build 1's). The vendored crates build under `CARGO_BUILD_WARNINGS=deny` because each vendored manifest carries `[lints.rust] warnings = "allow"` (pristine nice-plug warns in release builds); a hard error from a later toolchain still needs a patch. **Patch 5, the lost note-on**: a note-on listed ahead of a parameter change on the same frame is no longer lost with `SAMPLE_ACCURATE_AUTOMATION = true` (found by TH: 337 of 346 notes never started on spike-shell); proven with `enth-test-host bursts --order both --capacity 1024` and two controls. `vendor/README.md` lists every patch |
| **SK** | The F2b row above |
| **S2s** | The session tests: effect routing (a table-driven test per `Op`), undo and redo against the patch model, never blocks and never drops, epochs end to end, the host-parameter bridge, host state load mid-session, fuzz-lite on state blobs |
| **PL** | clap-validator with 0 failures on the debug bundle. Test-host event bursts, reset and `NOTE_END` checks. Poly-mod id unit tests. Plugin-only `host-render` cases |
| **ED1** | Interaction tests through a real session. Whole-editor snapshots at the P1 default and the maximum patch. No scrolling at 1200×800 for the P1 default, if feasible ([handoff §6](handoff/claude-design-layout.md)) |
| **ED2** | Route and depth ops come from drags. 0 frames while idle and at most 30 fps while visible data changes (exact-delay tests). The UI never waits on the audio thread: a stalled engine still gets editor frames |
| **RH** | The REAPER scenario list in [Test strategy](#test-strategy) is green on both formats in consecutive runs |
| **RT** | The G1 row below, with the load recorded, under `flock -x`. An instruction-count case for the swap |
| **INT** | The full gate is green. The leak gates pass. The owner sheet is written. The G1 rows are filled in except the owner's |
| **LV** | Snapshots at the P1 default and the maximum patch. Switchable in the dev window without a rebuild |

## Owner gates

| Gate | What you do | Time | When | Default if you don't answer |
| --- | --- | --- | --- | --- |
| Upstream issues (optional) | Post, rewrite or drop the drafted baseview xkb report, and file other issues in your own words (RustAudio's AI policy rules out agent-written contributions upstream) | 10 min | Any time | Not posted |
| Owner build 1: real display | Insert `enth-plugin` (CLAP, then VST3) 10× each; idle CPU; resize; Space after clicking the editor; HiDPI at `Xft.dpi: 192`. This replaces checklist C1, C4, C5 and C6 on the spike plugins | 40 min | After F2b | — (the G0 real-display rows stay pending) |
| Layout pick | Read the Claude Design variants; pick one, or "keep Lanes, fixed" | 20–30 min | When the variants arrive | Lanes, with the handoff's signal-flow fixes |
| Owner build 2: G1 | Make a bass, pad, pluck and lead you like in REAPER, and save them (an agent commits them to `content/presets/`) | 60–90 min, one or two sittings | After ED2, during INT | — (G1 waits) |
| Real audio (C3) | 8 instances at a 64-sample quantum, editors closed, then open. Your PipeWire config forces min-quantum 1024, so set `clock.force-quantum 64` and put it back to 0 afterwards | 30 min | With owner build 2 | — (the G0 xrun row stays pending) |
| Bitwig poly-mod (optional) | Start the 30-day trial (it needs an account); per-voice modulation of Macro 1; save and reload | 45 min | After PL | Deferred to S1 (roadmap) |

Total: about 2.5–3.5 h, plus 45 min if you do Bitwig. P1a's pending owner items (reverb A/B, the agent-eval blind listen, the metric sessions) are separate.

Two defaults apply until you say otherwise. **The fork is on, vendored privately** (this takes the default the coordinator gave you earlier; no objection was raised). **The layout stays "Lanes"** until you pick another.

## The plugin-side G1 checks

| Check | How | Who | When |
| --- | --- | --- | --- |
| CLI = plugin: the engine's golden cases, rendered through the plugin, give the engine's hashes | `cargo xtask host-render --package enth-plugin` | SK, PL | F2b, then every plugin edit |
| `process()` p99 and p99.9 under 25 % of the 64-sample deadline (333 µs), under `SCHED_RR`; with a structural edit every 250 ms, the p99 of instant-swap blocks and of the blocks during a crossfade under 25 % | `enth-test-host process-stats` on the release bundle, [conditions](benchmarks.md#real-time-measurement-p1b). A crossfading swap is reported; see the open check on the pre-effects crossfade | RT | After PL |
| clap-validator: 0 failures | `cargo xtask validate`, debug and release bundles | SK, PL | F2b |
| State round trip: an identical patch and a byte-identical second save | Properties, test host, REAPER `patch-roundtrip` | S2s, SK, RH | F2b |
| Editor leak at most 26 KB per open/close cycle, threads constant, instance churn flat | `editor-cycles` (200 cycles), instance churn | FK, INT | Before merging editor lanes; nightly |
| 0 editor frames while idle, at most 30 fps while visible data changes | kittest exact-delay tests | ED2 | ED2 |
| 8 instances on one thread: 59 % (p50) and 91 % (max) of the deadline; 16 spread over 4 threads, each thread's p99 under 50 % (information) | `process-stats --instances N --threads T` | RT | After PL |
| First open, focus, HiDPI, GPU cost | The real display | **You** | Owner build 1 |
| "A bass, pad, pluck and lead you like" | Through the editor | **You** | Owner build 2 |
| Poly-mod in Bitwig (optional) | The trial | **You** | Owned by S1 |

## Test strategy

Layers, by cost of signal, fastest first. Costs were measured in Phase 0 and P1a unless marked.

1. `cargo nextest run -p enth-session` (1–2 s, light): put the rules here, not in the shell or the editor.
2. `cargo test -p enth-ui` kittest interaction tests by accessibility label, without a renderer (about 2 s).
3. Component snapshots on lavapipe (about 2 s per PNG).
4. `cargo test -p enth-plugin` (incremental 1.3–2.9 s).
5. `cargo xtask host-render --package enth-plugin` and `cargo xtask validate`, on the debug bundle.
6. `cargo xtask reaper-test --package enth-plugin` (about 45 s) before merging plugin lanes.
7. `editor-cycles` and instance churn (minutes), before merging editor lanes and nightly.
8. Owner builds.

- **Plugin goldens are engine goldens.** A plugin-level case is an engine golden case played through the plugin, asserting the engine's hash, so a DSP change never churns plugin goldens. Plugin-only behaviour (slot automation, `reset()`, voice steal under host events, the monitor bypass) is compared with `enth render` of an equivalent performance, or gets its own one-file case. Traps: use velocities that are exactly `k/127` (or convert in one place), compute absolute event frames, use `DEFAULT_SEED`, test host blocks of 1, 64, 100 and 4096.
- **REAPER scenarios** (RH): `state-create` and `state-reload` (existing, retargeted), `patch-roundtrip` (a non-default patch survives a save and a reload in a new process), `slot-automation`, `macro-automation`, `undo-state-load`, `two-instances`, `editor-smoke` and `cross-format` (existing). Focus, Space, HiDPI and xruns are not in the harness: they are yours.
- **Interaction tests drive the real editor with a real session** over a narrow session trait, so the same code runs in kittest, the dev window and the plugin. A pixel snapshot is never the only assertion for an interaction.
- **Snapshots** are one PNG per component or region, plus per layout the whole editor at the P1 default and at the maximum patch (4 generators, 4 voice effects, 4 effects, 8 modulators). `kittest.toml` at the root with threshold 0.6 and 0 failed pixels; updated only on purpose, with the changed PNGs listed in the commit message.
- **Allocation and leak checks:** `assert_process_allocs` on debug bundles under clap-validator; 10,000-event bursts; `render --process-on caller` with the editor open; a plan swap while the editor animates; an RTSan-built test host (nightly pinned) can catch `malloc` in a stable-built plugin.

Reliability rules carry over from P1a: no wall-clock assertions in the gate, no random seeds in the gate, no test that depends on another lane's snapshots or goldens, and a flaky test is a bug. Two are new: a pixel snapshot is never the only assertion for an interaction, and every gate step is judged by its exit status.

## Rules for every lane

- Branch from the latest freeze, and write only the files your lane owns. One owner per doc: `docs/session.md` (S1s, S2s), `docs/plugin.md` (SK, PL), `docs/editor.md` (ED1, ED2), the G1 rows (INT). Report against your acceptance criteria when you stop.
- **Concurrency.** At most **2 heavy builds at once** ([AGENTS.md](../AGENTS.md#working-in-parallel-phase-0-lanes-and-any-concurrent-agents)): this laptop had 5.6 GiB free and a load of 17.5 when checked. Pair each heavy lane with a light one. Harness lanes reuse a release bundle that the coordinator builds after each merge (`~/code/enth-scratch/bundles/<sha>/`), so they never rebuild the plugin. Use `CARGO_BUILD_JOBS=3`, a separate `CARGO_TARGET_DIR` per worktree on ext4, `flock -s` around builds and `flock -x` around timing runs.
- **A two-tier gate, judged by exit status only** (no `cmd | tail && echo ok`, which reports `tail`'s status). Each prints the commit it checked.
  - `scripts/gate.sh lane <worktree>`: fmt, workspace clippy, check-layers, wasm-check, and `cargo test` of the crates the change touches plus their dependants (`cargo xtask affected`, against the merge base with main). Target under 5 minutes.
  - `gate.sh full` (the coordinator's): workspace tests, wasm-check, deny, golden, `host-render`, `validate`, and `reaper-test` separately under Xvfb. It runs in the foreground with a timeout above its measured length, or in the background with an explicit wake-up.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshot per layout, and the G1 table. A lane reports deltas and never edits another lane's budget.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `[workspace.dependencies]`, `Cargo.lock`, `[patch.crates-io]` | S0b, then the coordinator. FK's patch block is one coordinator commit |
  | `vendor/` | FK only |
  | `enth-engine`, `enth-render` | E+ only; RT after E+ is merged. Additive only, and every golden unchanged is part of acceptance |
  | `enth-session` files | S1s (types), SK (minimal implementation files), S2s (the rest) |
  | Theme tokens, the token gallery snapshot | Per-area token files from S0b; UI-C owns the gallery. A lane adds tokens only in its own area file |
  | `enth-ui` widgets | UI-C. ED lanes compose; widget signature changes are additive until F2b |
  | Snapshot PNGs (binary, about 300 KB) | One per component or region, owned by its lane; whole-editor snapshots per layout belong to that layout's lane. A lane that moves another lane's snapshot updates it in the same commit after rebasing |
  | Spike crates | Frozen. INT removes them from the workspace once xtask targets `enth-plugin` |
  | `xtask/src/main.rs`, CI workflows | S0b stubs every P1b command and job; lanes fill their own modules |
  | `tools/test-host` | TH only; a case file per plugin |

- **Ownership crossings** are additive only and flagged in the report (P1a's FXW and D2 crossed lanes). Plan the integration lane up front: INT replaces P1a's improvised CH.
- **Vendored crates** are edited only by FK. Any other lane that needs a change to nice-plug or baseview asks for it.
- Risks (report 04 lists the top eight with their cheapest retirement):
  1. two sources of truth for parameter values (the session's fold-in rule, clap-validator state tests, a test-host "automate, save, load, render = same hash");
  2. nice-plug threading pitfalls (shell rules in AGENTS.md, clap-validator on the debug bundle);
  3. real-display failures that agents can't see (owner build 1 right after F2b, exact pins);
  4. editor churn (the layout seam and stable ids, one PNG per component);
  5. merge hotspots, build contention and a slow gate (S0b pre-declares everything, the two-tier gate);
  6. CLI ≠ plugin renders (plugin goldens are engine goldens, E+ adds macro events);
  7. leaks and real-time regressions in the shell (the leak gates, the move at a swap, p99.9);
  8. owner bandwidth on G1 (two scheduled owner builds, defaults for every decision, Bitwig optional).

## Where the layout variants plug in

- ED1 builds a **layout seam**. Regions (header, macros, sources, voice effects, modulators, effects, overview, status) are components that draw into a rectangle.
- A layout is a value: a tree of rows and columns sized by tokens, plus a table that says which patch section fills which region. It is loadable from JSON like `ENTH_THEME`, so the dev window and the plugin can switch layouts without a rebuild. "Lanes" is layout 1.
- Stable widget ids keep drags, gestures and egui state alive across layouts.
- A variant then costs one file plus its two snapshots (lane LV) and touches no other lane's files. Layout work stays off G1's critical path unless you want the chosen layout before making your four sounds.
- Room is left for Recreate and Learn: a no-op `Overlay` (emphasis normal, dimmed or hidden, plus scores) and reserved lesson, challenge-bar and results regions behind the `learn` and `recreate` cargo features. P1b builds no presenters for them.

## Open checks

Not settled by the research; each is owned by a lane or by a later milestone.

- **Answered (RH, 2026-10-20): REAPER does not mark the project dirty after an editor-only patch edit** (`IsProjectDirty` reads 0 on CLAP and VST3; a slot-bound knob edit, the control, reads 1). A sixth fork patch adds dirty-marking (CLAP `mark_dirty`, VST3 `setDirty`); `DIRTY_MARKING_EXPECTED` in `xtask/src/reaper.rs` turns the `dirty-flag` scenario into a hard check when it lands.
- `MidiConfig::MidiCCs` (the mod wheel, later pitch bend) adds 2,080 hidden VST3 parameters. Decide it before the first build leaves the machine.
- Poly-mod and `NOTE_END` in a real CLAP host need Bitwig (optional here, owned by S1).
- Pre-effects crossfade: not adopted now because it changes the crossfade definition. A swap that adds or removes a module while a reverb survives still copies, and measured up to about 1 ms paced (p99 1,052 µs on main). That swap block is reported, not gated; if RT finds it unacceptable, that measurement is what reopens the pre-effects crossfade (blend the voice sums, then run one effect chain, which moves the state: swap block about 195 µs paced).
- Whether a depth drag needs smoothing against zipper noise (about 30 steps a second). A per-segment ramp of the term's depth would be the simple answer.
- Whether the keyboard-focus patch works inside REAPER's SWELL windows (`XSetInputFocus` to the parent). Owner build 1 decides.
- baseview PR #357 moves the X11 work off its own thread. Re-check at each nice-plug-egui release; the session mutex stays correct either way.
- All engine timings were taken at load 7–26 on a shared laptop. Re-run the G1 numbers on a quiet machine before the gate.
- Audition while the host has put a stopped plugin to sleep (nice-plug never calls `request_process`); editor size persistence (`EguiEditorState` has no setter).
