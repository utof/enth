# P1b research — summary

- **Date:** 2026-10-19
- **Reports:** `01-session-shell.md`, `02-editor.md`, `03-realtime-host.md`, `04-risks-tests-plan.md`
- **Status:** the owner reads this at their own pace. Work proceeds on the assumption they agree.

## Verdict

**P1b is buildable on what exists, with an engine-additions lane in the spine and a vendored fork of nice-plug and baseview.**

1. **The plugin renders exactly like the CLI:** 20 of 20 bit-identical, at 44.1 and 48 kHz, for host blocks of 1–4096 frames, with automation. It needs nice-plug's `SAMPLE_ACCURATE_AUTOMATION = true` and the fixed default seed (01, measured).
2. **The engine needs seams P1b depends on (lane E+):**
   - epoch-tagged parameter events, or slot events resolved through a per-plan slot table. Host automation can otherwise land on the wrong parameter across a plan swap (01, a bug found in the code);
   - a depth event, so a modulation-depth drag doesn't recompile every frame (02);
   - per-voice macro offsets for poly-mod;
   - a read-only telemetry tap: modulated values for the rings, modulator outputs, voices, scope (02);
   - `reset()`/`position()`, an audition-note command, the 20 ms monitor-bypass fade from ADR-0006, and macro and mod-wheel events in `Performance`;
   - **reverb state moved, not copied, at a plan swap.** A swap block reached 0.8–1.06 ms, 75–79 % of a 64-frame deadline, from copying 1.6 MB. The move brings it to 149 µs (03, prototype).
3. **Vendor the fork in the repo behind `[patch.crates-io]`.** A public git fork would expose the project name. All three patches apply and build warning-free:
   - slot rename (+123/−4);
   - `editor_closed` (+9);
   - baseview's xkb leak (+4/−1; 77 → ~16 KB per editor cycle).

   Add **keyboard focus only while typing**, so Space reaches REAPER (02). Caveat: vendored crates aren't lint-capped, so future toolchain warnings can fail CI; pin and allow per crate.
4. **The editor makes layout data.** A `LayoutSpec` (regions sized by tokens) plus stable widget ids keyed by (module id, parameter address) kept 59/59 controls stable across two layouts, against 0/59 today. Each Claude Design variant becomes one value and two snapshots. The generated editor costs about 4.6–4.9 M instructions per frame, against the spike's 6.7–7.6 M (02, measured).
5. **Real-time measurement needs real-time priority.** rtkit grants SCHED_RR (priority ≤ 20) once `RLIMIT_RTTIME` is lowered to 200 ms, no `ulimit -r` needed. Misses dropped from 980–2,299 to 3 of 45,000 blocks; p99.9 is 166–185 µs (12–14 % of the deadline) for one G1 instance. The misses were scheduler noise, not plugin CPU (03).

## Decisions

| Area | Decision | Evidence |
| --- | --- | --- |
| Session | `enth-session` (L6): one single-threaded `Session` owning the `Patch`, `History`, `Link` and `PlanMap`. The op's `effect()` decides between a value event and a recompile. The plugin wraps it in a mutex shared by the host main thread and baseview's GUI thread; the audio thread never touches it. The patch wins at load; host values are written back only when they truly differ (plain → normalised → plain moves 33–82 % of values) | 01 |
| Automation | Slot events (`EventKind::Slot`) resolved through a slot table compiled into each plan; parameter events carry epochs; stale ones are ignored | 01 |
| nice-plug pitfalls | Never call `GuiContext::set_state` while processing (nice-plug would load state on the audio thread). nice-plug never marks the project dirty, so check REAPER's save prompt. `SAMPLE_ACCURATE_AUTOMATION = true` | 01 |
| Fork | **Yes, vendored privately** with the three patches plus focus-while-typing. This takes the "fork" default the coordinator gave the owner earlier; no objection was raised. Report 04 had "stock" as its default; the evidence (01, all patches clean) favours the fork | 01, 02 |
| FTZ | nice-plug sets FTZ only behind a feature we leave off; design.md §5's statement is corrected | 01 |
| Editor structure | The editor lives in `enth-ui` (a separate editor crate would be same-layer). Widgets emit intents that become ops through the session at frame end. Stable ids keyed by (module id, address); `Knob` takes an id | 02 |
| Layout seam | `LayoutSpec` data (rows/columns sized by tokens, plus a section-to-region table), loadable from JSON like `ENTH_THEME`; "Lanes" is layout 1; variants are one file plus snapshots | 02, 04 |
| Interactions | egui drag-and-drop for drag-to-modulate (valid targets highlighted; voice → global refused); right-click "Type value…" with units. Fix: the parser rejects "−" (U+2212), which the design system displays. Fix: sub-step drags on discrete knobs are discarded, so accumulate them. Undo/Redo header buttons | 02 |
| Recreate/Learn hooks | A no-op `Overlay` (emphasis normal/dimmed/hidden, scores) and reserved lesson / challenge-bar / results regions behind cargo features | 02 |
| Repaint | Keep the spike's gating; "visible" = open, shown and scrolled into view (occlusion isn't reported) | 02 |
| Plan swap | **Move** effect state at a swap. The pre-effects crossfade alternative changes the crossfade definition: not adopted now, revisit only if measurements need it | 03 |
| RT measurement | A test-host `process-stats` with rtkit SCHED_RR and a timing histogram; a plugin-side histogram. G1 gates on p99/p99.9 under RR and on swap blocks; max is laptop noise | 03 |
| Multi-instance | 8 G1 instances on one thread: 59 % (p50) / 91 % (max) of the deadline. 16 need several threads, which hosts provide | 03 |
| xrun testing | A private PipeWire daemon (no hardware, quantum 64, `rtportal.enabled = false`) gives agents a silent JACK graph. Headless REAPER over JACK didn't connect, so a REAPER xrun test stays owner-attended | 03 |
| Allocation checks | nice-plug's `assert_process_allocs` in debug builds; an RTSan-built test host (nightly pinned) can catch `malloc` in a stable-built plugin | 03 |
| Cheapest strong test | Render the existing engine golden cases through the plugin in the test host and assert the engine's hashes: proves CLI = plugin with no new hash files | 04 |
| Gates | A fast `gate.sh lane` and a full coordinator `gate.sh full`, judged by exit status only | 04 |

## P1b lane plan

From report 04, with FK on by default.
- **Freezes:**
  - **F0b:** S0b workspace prep;
  - **F1b:** S1s session types + E+ engine seams, reviewed together by a judge agent;
  - **F2b:** SK walking skeleton through the real plugin.
- **Waves**, with two heavy slots each, every heavy lane paired with a light one:
  1. S0b with E+;
  2. UI-C (catalogue cards) and TH (test-host extensions), plus S1s;
  3. SK and FK, plus S2s (session complete);
  4. ED1 (editor wiring + layout seam) and PL (plugin complete), plus RH (REAPER harness) and **owner build 1 (real display)**;
  5. ED2 (modulation and feedback) and RT (real time in host);
  6. INT (integration, owner build 2, G1 rows);
  7. later, LV (layout variants) once the owner picks.
- **Estimate:** about 27–51 agent-hours in total, critical path about 13.5–24 h. P1a ran at about a quarter of its estimate.
- **Owner time:**
  - build 1 (real display, ~40 min);
  - the layout pick (20–30 min);
  - build 2: make a bass, pad, pluck and lead in REAPER (60–90 min);
  - real audio with 8 instances (30 min; note your PipeWire config forces min-quantum 1024);
  - optional Bitwig poly-mod (45 min).

## Corrections

| Where | Said | Correction |
| --- | --- | --- |
| design.md §5 | nice-plug sets FTZ around `process()` | Only behind an unused feature; irrelevant to our renders (01) |
| docs/dsp/engine.md | Plan swaps copy surviving state | Move effect state; copying 1.6 MB of reverb takes 75–79 % of a 64-frame block (03) |
| 04 | Fork default "stock" | Default is the vendored fork (coordinator's earlier stated default; 01's evidence) |
| 04 | 16 instances budget | 03's later measurement: 16 on one thread fails; spread across threads |

## Docs to change

| Doc | Change |
| --- | --- |
| `docs/phase-1b.md` (new) | The lane plan, freezes, waves, owner gates and acceptance from report 04, adjusted as above |
| `docs/design.md` | §3: slot events + epochs, depth event, telemetry tap, move-on-swap; §4: host values vs patch (the patch wins; write back only on true difference), slot names via the fork; §5: FTZ correction, plugin = CLI conditions |
| `docs/architecture.md` | `enth-session` (L6) and `enth-plugin` (L8) responsibilities, threading contract; vendored crates under `vendor/` with `[patch.crates-io]` |
| `docs/adr/0001-plugin-shell.md` | Dated note: vendored fork with four patches; pins; SAMPLE_ACCURATE_AUTOMATION; the `set_state` pitfall |
| `docs/adr/0002-ui-toolkit.md` | Dated note: layout as data, stable ids, keyboard focus while typing |
| `docs/benchmarks.md` | RT measurement with rtkit SCHED_RR; G1 rows for p99/p99.9 and swap blocks; multi-instance numbers |
| `docs/roadmap.md` | P1b points to phase-1b.md; G1's remaining rows |
| `AGENTS.md` | Never call `set_state` from the editor while processing; lanes run `gate.sh lane`, the coordinator `gate.sh full`; vendored crates are edited only by the fork lane |
