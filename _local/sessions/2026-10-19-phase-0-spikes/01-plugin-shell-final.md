# 01 — Plugin shell today (final)

- Date: 2026-10-19
- Sub-researcher: 01 (plugin shell)
- Questions answered: 1–8 from the brief, plus everything listed under Gaps in the first report.
- Relation to the first report: [01-plugin-shell.md](01-plugin-shell.md) is left unchanged. This file is self-contained: it repeats everything from the first report that still stands and adds the new local results.
- Change markers: **[new]** marks results that weren't in the first report, and **[changed]** marks conclusions that differ from it. Anything without a marker is carried over unchanged.
- Evidence labels: **measured** (I ran it on the owner's laptop today), **documented** (source code, changelog, release notes, official pages), **anecdotal** (issue reports, forum posts, single users).

### How the measurements were made

- **Runs.** There were three runs, all on 2026-10-19:
  - **RUN1** (about 01:30–02:20) is the first report.
  - **RUN2** (02:32–03:10) came after the first reboot. Its files were lost when Xorg crashed inside the NVIDIA driver at 03:12. The numbers survive in my notes.
  - **RUN3** (03:43–04:40) ran in persistent scratch, with every plugin-loading program under `xvfb-run`. Every RUN2 result I quote was reproduced in RUN3, with identical audio hashes. The one exception is the RUN2 CPU figure taken under an exclusive lock, which I could not repeat.
- **Crash context.** In RUN2, the validators and Carla ran with the real `DISPLAY` set; REAPER always ran in its own Xvfb. The three validator aborts just before the crash were deliberate allocation-check aborts in my rename test plugin, described under Q2(b). RUN3 never touched the real display and nothing crashed.
- **Scratch files.** Everything is in `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/`:
  - spike sources: `spike-np`, `spike-np-gui`, `spike-np-rename`, `clack-wrap`;
  - the patched nice-plug (`np-patched`) and the patch itself (`scripts/nice-plug-0.4.2-rename.patch`);
  - REAPER Lua tests (`reaper-tests/`) and runners (`scripts/rr.sh`);
  - results (`reaper-out/`, `carla-out/`, `validator-run/`);
  - a running log, `notes.md`.
- **Machine.** Builds used `CARGO_BUILD_JOBS` of 2–4 under the shared quiet lock. Five or six other researchers were running, so the load average was between 4 and 25; timings are tagged with their load.

## What changed versus the first report

1. **[new] All the hard must-haves are now measured in a real host, not only with validators.**
   - In native Linux REAPER 7.82 I ran the CLAP and VST3 builds headless, through Lua scripts with Dummy Audio under Xvfb. 72 params showed up with stable IDs, macro and slot automation reached the plugin, values and the patch blob survived a save and reload in a new process, and offline renders were bit-identical across runs and across formats.
   - Carla 2.5.8 loads the VST3 build and does the same save/reload round trip.
   - Steinberg's VST3 validator (SDK 3.8.1) passes 537 of 537 tests on every nice-plug build.
2. **[changed] Slot renaming (b) works for CLAP in REAPER with a small patch.**
   - Stock nice-plug 0.4.2 still can't rename. A 13-file, +123/−4-line patch that only adds API (no existing call changes) makes REAPER show "Osc 1 · Cutoff" within 50 ms.
   - **VST3 renaming fails in REAPER**: REAPER 7.82 rejects `restartComponent(kParamTitlesChanged)`. It does work in Carla's VST3 host.
   - REAPER keeps the old label on an automation lane that already exists, but the lane stays bound to the parameter by ID.
3. **[new] The editor behaves as intended in REAPER under Xvfb.**
   - 22 of 22 opens rendered, the first frame 10–49 ms after the editor was built, so issue #95 (blank first open) did not reproduce.
   - It draws **0 frames while idle** and about 26 frames/s while host automation plays with the mouse untouched.
   - An open, idle editor costs about +0.2 to +0.5 percentage points of one core.
   - New bug: `NiceEguiApp::editor_closed()` is never called.
4. **[new] nice-plug runs "GUI" tasks inline when `process()` is on the main thread.** Some hosts do that, including Steinberg's validator. So tasks scheduled from the audio thread must be realtime-safe.
5. **[new] Clack + clap-wrapper, measured.**
   - clack's own polysynth example plus clap-wrapper builds in 30 s.
   - That example **fails 3 clap-validator state tests**.
   - Its clap-wrapper VST3, via crates.io `clap-wrapper` 0.3.1, which bundles clap-wrapper 0.14.0, **shows a stale parameter in REAPER after reload**. That is the bug clap-wrapper 0.16.0 fixed, and 0.16.0 isn't on crates.io yet.
6. **[changed] Hosts.**
   - Native REAPER is now installed, and I found a working headless recipe: no licence or nag dialog blocks automation.
   - **The installed Carla 2.5.8 has no CLAP host.** Carla's CLAP code exists only on its unreleased main branch, so Carla tests VST3 only.
7. **[new] More validator and timing data.**
   - clap-validator's official 0.4.1 release binary gives the same results as the source build. Its zip naming bug is confirmed.
   - A 5-minute fuzz run was clean.
   - Incremental rebuilds take 6–10 s.
8. **Unchanged:** the licence traps (BSD-1-Clause, `Ubuntu-font-1.0`), the pinning advice, the maintainer and churn picture, Ardour without CLAP, and that no new framework has appeared since 2026-10-18.

## Answer in brief

1. **Keep nice-plug. Pin nice-plug 0.4.2 and nice-plug-egui 0.6.0 from crates.io.**
   - The licence is ISC (I read the file). nice-plug uses the MIT/Apache `vst3` 0.3.0 crate and has no `vst3-sys`.
   - Nothing changed upstream between the first report and 04:03 today.
   - One person, BillyDM, wrote about 89% of the commits since the fork.
   - Breaking releases come roughly monthly. The core had 3 breaking releases in 7 weeks; the egui adapter had 5 in under 4 months. *(documented + measured)*
2. **Must-haves (a), (c), (e), (f) and (g) pass in real hosts, with plain nice-plug 0.4.2.** *(measured)*
   - REAPER: CLAP and VST3, 72 params with stable IDs, automation, save and reload, patch blob, bit-identical renders.
   - Carla: VST3.
   - clap-validator: 36 passed, 0 failed.
   - Steinberg's VST3 validator: 537 of 537.
   - Stable Rust 1.95.
3. **(b) renaming needs a small patch to nice-plug, and works only for CLAP in REAPER.** *(measured)* REAPER refuses VST3 title changes. Plan to show "Slot N" in VST3 builds, and decide whether to carry the patch in a fork. The RustAudio AI policy rules out upstreaming code written by agents.
4. **(d) poly-mod is declared correctly but untested in a host.** *(measured)* The macros carry the per-note-modulation flag and the plugin exposes voice-info. Testing needs Bitwig, which is on the owner checklist.
5. **The editor is cheap and repaints only when needed.** *(measured)*
   - Under Xvfb: 0 frames while idle, about 26 frames/s while automation plays.
   - Never-blank first opens.
   - Behaviour on the real NVIDIA display, HiDPI and the keyboard still need your eyes and hands.
6. **Two nice-plug pitfalls to design around.** *(measured + documented)*
   - `execute_gui` tasks may run inline inside `process()`.
   - `editor_closed()` is never called.

   Neither blocks Phase 0.
7. **Clack + clap-wrapper remains the escape hatch, not the plan.** *(measured)* It builds fast and its dependency tree is tiny (12 crates). But you take on the parameter, state and GUI glue yourself, and the evidence shows how that goes: clack's own example fails 3 validator state tests, and the crates.io wrapper lags a VST3 fix.
8. **Licence traps that still break the planned `cargo deny`:** BSD-1-Clause (`assert_no_alloc`, `nice-assert-no-alloc`) and egui's `Ubuntu-font-1.0`.
9. **REAPER can be scripted as a fast local host-test harness.** *(measured)* The REAPER-plus-Lua recipe takes about 3–5 s per scenario and needs no hands. It complements the clack-host test host planned for Task 4.
10. **No new Rust plugin framework has appeared since 2026-10-18.** truce (custom licence) and lx-aura (GPL) both fail the licence gate. *(documented)*

## Findings

### Q1 — nice-plug: version, cadence, maintainers, issues, licences, `vst3`

**Versions and activity (documented; checked 2026-10-19 01:30 and again at 04:03, with no change in between):**
- **crates.io:** `nice-plug` 0.4.2 (2026-09-14) is the latest. It had 19 versions from 0.1.0 (2026-06-04) to 0.4.2; MSRV 1.88, edition 2024. Downloads on 2026-10-19: 15,687 in total, 12,206 in the last 90 days ([crates.io](https://crates.io/crates/nice-plug)).
- **Adapters:** `nice-plug-egui` 0.6.0 and `nice-plug-iced` 0.5.0, both 2026-10-18.
- **Codeberg `main`:** `7197104c0d` (2026-10-18). Everything after the `v0.4.2` tag (`e60b5db09d`) is adapter-only: egui 0.37, baseview 0.4.1 and a configurable poll timer ([commits](https://codeberg.org/RustAudio/nice-plug/commits/branch/main); [CHANGELOG](https://codeberg.org/RustAudio/nice-plug/src/branch/main/CHANGELOG.md)).
- **Repo statistics:** 79 stars, 39 forks, 12 open issues, 3 open PRs. Its CI (Woodpecker) does not run clap-validator ([.woodpecker/test.yaml](https://codeberg.org/RustAudio/nice-plug/src/branch/main/.woodpecker/test.yaml)).
- **Commits on `main` per month since the fork** (by author date): Apr 7, May 65, Jun 68, Jul 28, Aug 55, Sep 41, Oct 6.

**Maintainers (documented):** BillyDM wrote 240 of those 270 commits and is the only crates.io owner of nice-plug, nice-plug-egui and egui-baseview. In practice the bus factor is 1. The organisation's membership needs a login token, so it is unknown.

**Breaking-change rate (documented, CHANGELOG):**
- Core: 0.2.0 (07-24), 0.3.0 (08-17), 0.4.0 (09-09).
- Egui adapter: 0.2 (06-28), 0.3 (07-24), 0.4 (08-17), 0.5 (09-09), 0.6 (10-18).
- Each core release listed 9–14 breaking items.
- The README still says the project "is currently experimental".

**Open issues that matter to enth (documented/anecdotal):**
- [#95](https://codeberg.org/RustAudio/nice-plug/issues/95): the editor is sometimes blank on first open in REAPER on Linux. The maintainer posted "should be fixed in the latest commit" on 2026-10-18; the reporter still hasn't confirmed as of 04:03. **[new]** It did not reproduce in my 44 headless opens (Q3).
- [#31](https://codeberg.org/RustAudio/nice-plug/issues/31): "switch to using clack?".
- [PR #96](https://codeberg.org/RustAudio/nice-plug/pulls/96): a VST3 editor-spawn fix.
- [PR #15](https://codeberg.org/RustAudio/nice-plug/pulls/15): lets the plugin set its own parameters. Stalled.

**Licence:** ISC, in the repo's `LICENSE` at `7197104` (I read the file). The crates.io packages contain no LICENSE file, so the notice belongs in `THIRD_PARTY_NOTICES.md`.

**Dependency licences (measured with cargo-deny 0.19.4 against the planned Task 2 allow-list):**

| Tree | Result |
|---|---|
| nice-plug 0.4.2 (+ `assert_process_allocs`), with or without nice-plug-egui 0.6.0 | **Fails:** `nice-assert-no-alloc` 1.0.0 is `BSD-1-Clause` (I read the LICENSE file). The `assert_no_alloc` 1.1.2 crate planned for Task 5 is also BSD-1-Clause |
| Editor build with egui `default_fonts` | **Fails:** `epaint_default_fonts` 0.37.0 declares `… AND OFL-1.1 AND Ubuntu-font-1.0`. The allow-list's `LicenseRef-UFL-1.0` doesn't match it; it has been `Ubuntu-font-1.0` since 0.36.0 (2026-08-05) |
| **[new]** clack 0.2.0 + clap-wrapper 0.3.1 | Passes. 12 crates. The C++ bundled inside clap-wrapper is MIT/Apache: I read the AudioUnitSDK (Apache-2.0), clap, clap-wrapper, filesystem and vst3sdk (MIT) LICENSE files. cargo-deny can't see these, so they belong in the notices file |

**`vst3` versus `vst3-sys` (measured):** `cargo tree` shows `vst3 v0.3.0 → nice-plug v0.4.2` and no `vst3-sys`. vst3 0.3.0 is `MIT OR Apache-2.0`, with both LICENSE files present.

### Q2 — nice-plug against ADR-0001's must-haves

**Test plugins (measured).** `spike-np` is about 205 lines: an 8-voice saw, 8 macros with poly-mod IDs, 64 slots, and a persisted JSON patch blob (`#[persist]`) into which the plugin writes a time nonce on first activation. `spike-np-gui` adds an instrumented egui editor. `spike-np-rename` is built against the patched nice-plug.

| # | Must-have | Verdict | Evidence |
|---|---|---|---|
| a | 72 fixed params, stable IDs | **Yes; host-verified [new]** | **REAPER CLAP and VST3:** 75 params, i.e. 72 plus REAPER's own Bypass/Wet/Delta. Parameter IDs read `index:CLAP id`, e.g. `0:825615486` for `macro_1` and `71:34232479` for `slot_64`, and are **identical for VST3**. All 72 are automatable. Automation lanes are saved as `<PARMENV 1:825615487 …>`, i.e. by ID. An envelope ramp on Macro 1 (gain) and Slot 1 (cutoff) shows in the render: RMS 0.090 → 0.225 and spectral centroid 1880 → 5261 Hz over 1.5 s, identical for CLAP and VST3 (data hash `20fba1145f2510c6`). **Carla VST3:** 72 params, correct names. IDs are a 31-bit hash of the string ID, and state stores the string IDs |
| b | Runtime slot renaming | **[changed] Stock 0.4.2: no. Patched: yes for CLAP in REAPER; no for VST3 in REAPER; yes in Carla VST3** | See "Renaming" below |
| c | Patch blob saved with the project | **Yes; host-verified [new]** | **REAPER CLAP and VST3:** set Macro 1 = 0.7 and Slot 1 = 0.3, save, reopen in a **new REAPER process**: 0.7/0.3 restored, the automation envelope restored, and the decoded chunk's patch nonce identical before and after (CLAP `1792371899510299182`; VST3 `1792371911142699995`). **Carla VST3:** the same round trip, nonce identical. Caveat (measured): a `#[persist] String` holding JSON is escaped twice in the state, so persist a typed struct instead |
| d | CLAP poly-mod on macros | **Declared correctly; host behaviour untested** | Macros report `AUTOMATABLE \| MODULATABLE \| MODULATABLE_PER_NOTE_ID` and the plugin exposes `voice-info`. REAPER has no poly-mod UI and there is no Bitwig here, so this is on the owner checklist |
| e | VST3 export | **Yes; loads and validates [new]** | Loads, saves, reloads and renders in REAPER 7.82 and Carla 2.5.8. **Steinberg VST3 validator** (SDK 3.8.1, `3cdf9ca5`, 2026-08-11, which I built locally): `spike_np` 537 passed / 0 failed, `spike_np_gui` 537/0, `spike_np_rename` 537/0 |
| f | Passes clap-validator | **Yes** | 0.4.1 (source build in RUN1, official binary in RUN2/RUN3): 36 passed, 0 failed, 8 skipped (all not applicable) for the GUI-less, editor and rename builds. **[new]** `fuzz -j2` for 90 s (RUN1) and 5 min with JSON output (RUN3): no errors. clap-validator has no GUI tests |
| g | Stable Rust | **Yes** | rustc 1.95.0 stable. The only nightly feature is the optional `simd` |

**Renaming (b) [changed, measured]:**
- **Why stock nice-plug can't do it (documented, code at 0.4.2):** names are fixed at construction (`ParamInfo { name: String }`, `Param::name() -> &str`). The CLAP wrapper only calls `rescan(CLAP_PARAM_RESCAN_VALUES)`, and VST3 never sends `kParamTitlesChanged`. CLAP itself allows `CLAP_PARAM_RESCAN_INFO` for name changes while the plugin is active ([params.h, CLAP 1.2.10](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h)).
- **The patch** (`scripts/nice-plug-0.4.2-rename.patch`): 13 files, +123/−4 lines, additive only, so existing callers such as the egui widgets compile unchanged. It adds:
  - `Param::display_name()` and `set_display_name()`, backed by a `Mutex<Option<String>>` override;
  - `request_param_info_rescan()` on `ProcessContext` and `GuiContext`;
  - in CLAP, a main-thread `host_params.rescan(INFO | TEXT)`; in VST3, `restartComponent(kParamTitlesChanged)`;
  - `get_info` and `getParameterInfo` now report `display_name()`.
- **REAPER 7.82, CLAP:** `TrackFX_GetParamName(8)` changed from "Slot 1" to "Osc 1 · Cutoff" within 50 ms of the trigger, and a second rename ("Filter · Res") also worked. After a reload the names come back once the plugin re-derives them. An automation lane that already existed keeps its old label ("Slot 1 / …", both from `GetEnvelopeName` and in the saved `PARMENV`) but stays bound by ID. REAPER's changelog for 7.67 (2026-03-29) adds `CSURF_EXT_TRACKFX_PARAMINFO_CHANGED` "so control surfaces can update when FX parameter names … change" *(documented, `whatsnew.txt`)*.
- **REAPER 7.82, VST3:** names don't refresh. nice-plug's log shows `Failed the restart request call with flags '16'` (also with `16|4`), four times out of four, so REAPER doesn't return `kResultOk` for `kParamTitlesChanged`. After a reload one of the two new names showed up and the other didn't: a side effect of the reload, not something to rely on.
- **Carla 2.5.8, VST3:** the renamed title appears ("8=Osc 1 · Cutoff") and persists across a reload.
- **Design consequence:** names must be derived from the patch, on the main thread (when a slot is assigned in the GUI, or when a patch or state loads), and refreshed through `rescan(INFO)`. VST3 in REAPER will show "Slot N".

**[new] Pitfall: `execute_gui` runs inline (measured + documented).** My rename trigger in `process()` called `execute_gui(task)`. Steinberg's validator calls `process()` on the thread nice-plug treats as main, so the task ran inline inside `process()`, and `assert_process_allocs` aborted on its allocations ("memory allocation of 15 bytes failed", then "54 bytes"). nice-plug's `schedule_gui` executes the task directly when it is already on the main thread (code read). With the task body wrapped in `permit_alloc`, the validator passes 537 of 537. So:
- don't trigger main-thread work from `process()` when you can avoid it;
- any task that `process()` does schedule must be realtime-safe.

**[new] Scripted tests must wait for parameter changes (measured).** In REAPER, reading a parameter back right after `TrackFX_SetParamNormalized` returned the old value for both formats. The new value was in place after 1.5 s. For VST3, nice-plug ignores `setParamNormalized` while processing and the value arrives through `process()`. A save straight after a set therefore stored the old VST3 value. Host-driven tests must wait for one or more process cycles before asserting.

**Other code-reading notes (documented, unchanged):**
- `ext_params_get_info` checks `index > count` where it should check `>=`; latent.
- The wrapper sets FTZ only with `unsafe_flush_denormals`, off by default, so engine determinism is unaffected.
- The CLAP/VST3 process path is wrapped whole in `assert_no_alloc`. The 5-minute fuzz with it on found nothing.

**Build numbers (measured, contended machine):**
- Cold builds:
  - GUI-less: 90.8 s at load 4.4 with 4 jobs, and 65.6 s at load 3.6 with 3 jobs; peak RSS about 340 MB.
  - Editor: 136 s with 4 jobs and 234 s with 3 jobs as the load rose to about 12; peak RSS about 960 MB.
- **[new] Incremental** (exclusive lock acquired, load 12–13, a one-constant edit, debug build with `opt-level=1`, full debuginfo, default linker): GUI-less 8.85 / 6.76 / 6.33 s; editor 9.65 / 8.90 / 9.41 s.

### Q3 — Editor API, egui adapter, repaint gating, sizing, HiDPI

**API (documented, unchanged):**
- `Plugin::Editor` is an associated type.
- `Editor::spawn(parent, wait_for_parent, fallback_scale_factor, GuiContext, HostMethods)`.
- `EditorHandle` has `set_size`, `adjust_size`, `param_value_changed`, `state_changed` and `host_main_thread_callback`.
- `GuiContext` gives `param_setter()`, `get_state`/`set_state` and `request_restart`.
- The egui side implements `NiceEguiApp` and is created with `create_egui_editor(...)` ([editor.rs](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug-core/src/editor.rs)).

**Repainting (documented):**
- It has not redrawn continuously since nice-plug-egui 0.4.0 (2026-08-17).
- In 0.6.0 (egui-baseview 0.9.0, baseview 0.4.1) a redraw happens only for:
  - an egui `request_repaint`;
  - input;
  - host parameter or state changes;
  - a set `RepaintNotifier` flag, which is checked every 1/64 s by default (configurable with `with_repaint_poll_interval`).
- The regression in egui-baseview 0.8.x, where the notifier did nothing until the mouse moved ([#37](https://codeberg.org/RustAudio/egui-baseview/issues/37)), was fixed in 0.9.0. So pin 0.6.0.

**[new] Measured in REAPER 7.82 under Xvfb (Mesa software GL).** The test opened the editor 20 times (0.8 s open, 0.3 s closed), then left it open and idle for 6 s, then played the transport for 4 s with Macro 1 automated, without touching the mouse. CLAP and VST3, RUN2 and RUN3:

| Measure | CLAP | VST3 |
|---|---|---|
| Opens; opens that drew a first frame | 22; 22 | 22; 22 |
| First frame after `build()` | 10–49 ms | 10–43 ms |
| Frames per 0.8 s open | 3–4 | 3 |
| Frames during 5 s idle, no input | **0** | **0** |
| Frames/s during automated playback | 25.5 (RUN3), 26.2 (RUN2) | 27.5 (RUN3), 27.0 (RUN2) |
| `editor_closed()` calls | **0 of 22** | **0 of 22** |

- **Blank first open (#95): not reproduced** in 44 opens. That was under Xvfb with no window manager, so a real-display check is still needed.
- **CPU of the REAPER process** (from `/proc/self/stat`, 10 s windows):

  | Run | REAPER, no plugin | Plugin, editor closed | Editor open, idle | Editor open, automation playing |
  |---|---|---|---|---|
  | RUN2, exclusive lock, load ~11, one run each | 5.5 % | CLAP 6.1 %, VST3 6.0 % | CLAP 6.3 %, VST3 6.3 % | — |
  | RUN3, shared lock (the exclusive lock was starved for 15 min), load 18–25, two runs | 7.1–7.2 % | CLAP 7.8–8.0 %, VST3 7.6–8.2 % | CLAP 8.3–8.5 %, VST3 8.4–8.7 % | CLAP 9.4–9.5 %, VST3 9.3–10.3 % |

  So an open, idle editor costs about **+0.2 to +0.5 percentage points of one core** over a closed one. This is software GL, so GPU cost on the NVIDIA card is unmeasured.
- **[new] Bug:** `NiceEguiApp::editor_closed()` is declared, with an empty default, in `nice-plug-egui-0.6.0/src/lib.rs:94`, and nothing in nice-plug-egui 0.6.0 or egui-baseview 0.9.0 calls it *(documented; measured 0 calls)*. The gain_egui example relies on it to drop its `egui::Context` copies. enth's editor must not depend on it; it can detect reopening in `build()` instead.

**Sizing (documented):**
- `ResizeHint` sets resizable or not, min/max size and aspect ratio, and answers CLAP `can_resize`/`get_resize_hints`/`adjust_size`.
- The host can resize the editor, and the editor can ask the host to resize it.
- The window size isn't stored in preset state.
- There are no floating windows and no child windows ([baseview #216](https://github.com/RustAudio/baseview/issues/216)).

**HiDPI (documented/anecdotal):** CLAP `set_scale` is used only as a fallback. On X11, baseview takes `Xft.dpi` first ([baseview #359](https://github.com/RustAudio/baseview/issues/359), which also says "Reaper never calls set_scale"; anecdotal). HiDPI can't be tested meaningfully in Xvfb, so it is on the owner checklist.

**Keyboard (documented):** egui-baseview passes keys it doesn't want back to the host. `on_virtual_key_from_host` is still a TODO, so VST3 keys forwarded by the host are dropped. There is a `keyboard_focus_workaround` feature. On the owner checklist.

**Build dependencies (measured):** baseview 0.4.1 loads X11, xcb and xkbcommon at runtime. The editor `.clap` links only libc, libm and libgcc_s, and built without any X11 `-dev` packages (in RUN1, before the owner installed them).

### Q4 — clack, clap-wrapper and the cost of switching

**Status (documented, 2026-10-19):**
- **clack 0.2.0** (2026-09-12): `MIT OR Apache-2.0`, MSRV 1.85, 241 stars. prokopyl has 339 commits; the next contributor has 11.
  - Its README: "feature-complete … APIs can still have breaking changes".
  - An open soundness issue: [#56](https://github.com/prokopyl/clack/issues/56).
  - `clack-extensions` 0.2.0 covers `params`, `state`, `gui`, `voice-info`, `note-ports`, `timer`, `posix-fd`, `track-info` and more.
  - prokopyl is now also baseview's lead.
- **clap-wrapper (C++) 0.16.0** (2026-08-08), MIT, active (last push 2026-10-15). 0.16.0 includes "VST3: parameter values are synced from the CLAP plugin after state load" ([release](https://github.com/free-audio/clap-wrapper/releases/tag/v0.16.0)).
- **clap-wrapper-rs:** crates.io 0.3.1 (2026-05-18) bundles clap-wrapper **0.14.0**. Its main branch moved to 0.16.0 on 2026-08-20 but **hasn't been released**. No standalone ([repo](https://github.com/blepfx/clap-wrapper-rs)).

**[new] Measured switching probe.** I built clack's own polysynth example (tag `v0.2`, 761 lines, 1 parameter, no GUI) from the crates.io clack 0.2.0 crates, plus `clap_wrapper::export_vst3!()` from clap-wrapper 0.3.1:

| Check | Result |
|---|---|
| Dependency tree; `cargo deny licenses` | 12 crates; ok |
| Cold build, including the C++ wrapper and VST3 SDK via `cc` | **30.0 s**, 330 MB, at load about 20 |
| clap-validator 0.4.1 | **31 passed, 3 failed, 1 warning, 9 skipped.** Failures: `state-reproducibility-{basic,binary,buffered}`: "Volume … 0.2000 vs 0.0556 changed without a rescan request". Warning: `state-invalid-random` loaded random bytes |
| Steinberg VST3 validator (through clap-wrapper) | 537 passed / 0 failed |
| REAPER, CLAP | Loads; Volume 0.33 saved and restored |
| REAPER, clap-wrapper VST3 | Loads, and renders identically to the CLAP build (data hash `e47419dd26b7f1ad`). **After reload REAPER shows Volume 0.2000 (the default)**, yet the render of that same saved project peaks at 0.33. So the plugin's state was restored but REAPER's view of the parameter is stale: the bug 0.16.0 fixed, which crates.io doesn't have yet |

**Cost, judged against the evidence (estimate, not validated):**
- With clack you write the parameter, state, gesture, voice-info and GUI glue yourself: nice-plug's wrapper is 11,165 lines. The probe shows that even the library author's own example gets parameter rescans after state load wrong.
- For VST3 you'd use clap-wrapper-rs from git, or wait for a release that includes 0.16.0.
- The enth spike needed about 205 lines on nice-plug for 72 params, poly-mod IDs, state and synth. clack's 1-parameter example is 761 lines.
- My estimate stays at **2–4 weeks to reach parity on enth's narrow needs**, against ADR-0001's two-day switch rule.
- Clack's real advantages: full CLAP coverage, renaming without a fork, and a tiny, fast-building dependency tree.

### Q5 — Linux hosts

**Native REAPER versus Wine REAPER:**
- REAPER under Wine is the Windows build. It can't load Linux ELF `.clap`/`.vst3` plugins. *(Inference from the binary formats; no source states it explicitly.)*
- Native REAPER 7.82 (2026-10-04; [download](https://www.reaper.fm/download.php)) is now installed in portable mode, and it's what matters for testing. The evaluation is "60 days … with full functionality" ([purchase](https://www.reaper.fm/purchase.php)).

**[new] Headless REAPER recipe (measured, RUN2 and RUN3).** I ran a copy of the portable install, never your own copy:
- `xvfb-run reaper -nosplash -newinst script.lua` runs a Lua script, and `-renderproject x.rpp` renders and exits.
- `reaper.ini` keys found:
  - `linux_audio_mode=2` selects **Dummy Audio**; 3 is PulseAudio, and 0, 1, 4 and 5 didn't start audio here (JACK wasn't running);
  - `linux_audio_srate=48000`;
  - `clap_path_linux-x86_64=` and `vstpath=` set the plugin paths.
- With those keys already in `reaper.ini`, **no "new portable install" scan prompt and no evaluation nag dialog appeared**. The title bar does say "EVALUATION LICENSE".
- The one blocker: quitting with action 40004 prompts "Save unsaved project before closing?" even after `Main_SaveProjectEx`. The workaround is `Main_openProject("noprompt:empty.rpp")`, then 40004.
- Each scenario took 3–5 s.

**[new] Carla.**
- The installed Ubuntu package is Carla **2.5.8**. Its Python backend has no `PLUGIN_CLAP`, and `CarlaPluginCLAP.cpp` is absent at tags v2.5.8 and v2.5.10 (the latest release, 2025-07-31). It exists only on main, last touched 2025-11-10 *(documented)*. **So Carla here is a VST3-only second host.** The coordinator's note calling it a CLAP host is wrong for this version.
- It loads the Linux VST3 build headless through `carla_backend.py` and `libcarla_standalone2.so` under `pw-jack`; it has no Dummy audio driver.
- VST3 state is stored as `VC2!` + XML + JUCE base64; I decoded it and checked the values and nonce.

**Ardour (documented):** the 9.8 release notes (2026-08-23) mention LV2 and VST3 only, and the CLAP request is open ([tracker 9417](https://tracker.ardour.org/view.php?id=9417)). Ardour is VST3-only, and it isn't installed here.

**Bitwig (documented):** 30-day trial; once it ends, saving and exporting are blocked ([support](https://bitwig.com/support/shop_license_activation/48)). It is the only CLAP poly-mod host available on Linux. Not installed.

**Known issues, 2025–2026:**

| Host | Issue | Status | Label |
|---|---|---|---|
| REAPER on Linux | baseview editor sometimes blank on first open ([nice-plug #95](https://codeberg.org/RustAudio/nice-plug/issues/95)) | "Fixed in latest commit" 2026-10-18; **not reproduced in 44 headless opens [new]** | anecdotal + measured |
| REAPER on Linux | VST3 `kParamTitlesChanged` rejected | **[new]** | measured |
| REAPER on Linux | Never calls CLAP `set_scale` ([baseview #359](https://github.com/RustAudio/baseview/issues/359)) | Open | anecdotal |
| Bitwig 6 (flatpak, XWayland) | Black editor ([baseview #331](https://github.com/RustAudio/baseview/issues/331)) | Closed 2026-10-09 | anecdotal + fix |
| Bitwig, CLAP | Panic when latency changes ([nice-plug #71](https://codeberg.org/RustAudio/nice-plug/issues/71)) | Fixed in 0.4.0 | anecdotal + fix |
| Bitwig, VST3 | nih-plug editor black ([nih-plug #271](https://github.com/robbert-vdh/nih-plug/issues/271)) | Open | anecdotal |
| Ardour 8.4, VST3 | nih-plug crash ([nih-plug #259](https://github.com/robbert-vdh/nih-plug/issues/259)) | Open, old nih-plug | anecdotal |
| Carla, VST3 | Resize loop ([nice-plug #76](https://codeberg.org/RustAudio/nice-plug/issues/76)) | Fixed through baseview | anecdotal + fix |
| All X11 hosts | baseview moving X11 work to host timer/fd callbacks ([#342](https://github.com/RustAudio/baseview/issues/342), [PR #357](https://github.com/RustAudio/baseview/pull/357), still open 2026-10-19) | Coming breaking change | documented |

### Q6 — clap-validator

- **Status (documented):** 0.4.1 (2026-07-22) ends a gap since 0.3.2 in 2023. It adds the multi-process fuzzer, parameter-rescan checks in the state tests, `--include`/`--exclude`, `clap-validator.toml` and **MSRV 1.95.0**. MIT; maintainers robbert-vdh and Quant1um ([release](https://github.com/free-audio/clap-validator/releases/tag/0.4.1)).
- **[new] The official binary (measured):** the zip (`sha256 49edadcf…ad90`) contains a tarball named `clap-validator-0.3.2-127-g152b982-ubuntu-22.04.tar.gz`, but the binary inside reports `clap-validator 0.4.1`. That confirms [#39](https://github.com/free-audio/clap-validator/issues/39). Its results match the source build.
- **[new] Practical notes (measured):**
  - `--json` puts results on stdout, but under `xvfb-run` the log lines (stderr) end up in the same file, so parse from the first `{`.
  - `fuzz --json` prints `[]` when it finds nothing.
  - A source build takes 76 s (debug).
- **CI practice (documented):**
  - GitHub code search found 80 workflow files that run `clap-validator validate`.
  - OctaSine installs it from git and uses the old 0.3 filter flags, so pin a version.
  - clap-wrapper-rs builds Steinberg's VST3 `validator` with cmake. **[new]** That works locally too: cmake 4.4.4 from PyPI through `uv tool run`, then about 260–317 s to build the `validator` target.

### Q7 — New contenders since 2026-10-18

None found (web and crates.io searches on 2026-10-19). For the watch list only:
- **truce:** "The Truce License" (`LicenseRef-TruceLicense-1.0`) fails the licence gate. One maintainer, 82 versions between May and July 2026, no push since 2026-07-18.
- **lx-aura:** GPL-3.0.
- **plughost** 0.1.3: a host, not a plugin framework; relevant only to the hosting question.

### Q8 — `request_callback`, timer-support, posix-fd-support, child windows (open check #10)

All documented, from code at `7197104`/`v0.4.2`:
- **`request_callback`:** used only internally, by the CLAP task queue. Plugins reach it through `AsyncExecutor::execute_gui` and editors through `HostMainThreadCaller`. **[new]** If the caller is already on the thread nice-plug treats as main, the task runs inline (Q2).
- **`timer-support` and `posix-fd-support`:** neither is implemented or queried by the CLAP wrapper. The VST3 wrapper on Linux uses `IRunLoop` with an fd handler. Today baseview's X11 thread does the GUI work; baseview #342/#357 would change that.
- **Child or floating windows:** no.
- **Bottom line:** not enough to relay a hosted plugin's GUI. That would need a fork or clack.

## Recommendation for Phase 0

1. **Keep nice-plug.** Pin `nice-plug = "=0.4.2"` and `nice-plug-egui = "=0.6.0"`, which bring egui 0.37.0, egui-baseview 0.9.0 and baseview 0.4.1, and commit `Cargo.lock`. Task 3's hard must-haves are already shown to pass. Spend the spike on the editor on the real display, on poly-mod in Bitwig, and on the renaming decision.
2. **Renaming: carry the 123-line patch in a small fork** (`git` dependency pinned to a commit), or accept "Slot N". The evidence favours the fork for CLAP: it works in REAPER in 50 ms, costs little, and adds API without changing any. **VST3 will show "Slot N" in REAPER regardless.** Derive names from the patch on the main thread only, and call `request_param_info_rescan()` after slot assignment and after state load.
3. **Design around the two nice-plug pitfalls:**
   - keep `process()` from scheduling allocating GUI tasks, because `execute_gui` may run inline;
   - don't rely on `editor_closed()`.
4. **Use REAPER as a local automated host test now.** The Lua recipe gives load, parameter, automation, save/reload and render checks in seconds, with bit-identical renders across CLAP and VST3. It belongs in `xtask` (for example `cargo xtask reaper-test`) as a local, not CI, gate. Keep the clack-host test host (Task 4) for CI and for measurement.
5. **Licence plumbing (Task 2):** `deny.toml` should allow `Ubuntu-font-1.0` and decide on `BSD-1-Clause` (recommendation: allow it). Add notices for nice-plug (ISC) and, if clap-wrapper is ever used, for its bundled C++.
6. **Validation in CI:** run `clap-validator validate` pinned to 0.4.1 with a committed `clap-validator.toml`, run Steinberg's VST3 validator in a nightly job (cmake via `uv tool run --from cmake`), and add a short `fuzz -j2 -d2m` nightly.
7. **Keep clack + clap-wrapper as the escape hatch.** If it's ever used, take clap-wrapper-rs from git with clap-wrapper ≥ 0.16.0, and budget time for the parameter and state details the validator checks.
8. **Watch baseview #357.** Expect nice-plug-egui 0.7 with a host timer/fd API. Budget about a day.

## Plan check — what in the current docs the evidence says should change

- **ADR-0001 (keep nice-plug; update the details).**
  - (1) Pin 0.4.2 + 0.6.0.
  - (2) **(b): "works for CLAP with a small carried patch; VST3 in REAPER can't rename (host rejects it); Slot N fallback stays for VST3."**
  - (3) Record that (a), (c), (e), (f) and (g) pass in REAPER and Carla, with the Steinberg and CLAP validators, so the two-day switch rule won't trigger. The real risk is the editor on the real display; add an editor row.
  - (4) Under Consequences: bus factor 1, the RustAudio AI policy (patches live in a fork), the inline `execute_gui` pitfall, and the missing `editor_closed()` call.
  - (5) Alternatives: the clack + clap-wrapper probe results (fast build, but its own example fails state tests; crates.io wrapper lags 0.16.0).
- **phase-0.md Task 3.**
  - The must-have checks can be **automated with the REAPER Lua harness**: (a), (b), (c) and (e) don't need hands.
  - Add a row "(i) editor on the real display in REAPER: first insert 10/10, repaint on automation with the mouse still, 20 close/reopen cycles, HiDPI, keyboard".
  - (h): Carla 2.5.8 is a VST3-only second host, Ardour is VST3-only, and Bitwig is the only CLAP and poly-mod host.
  - Debug builds with `assert_process_allocs` need the BSD-1-Clause allowance.
  - The template is a cookiecutter (`uvx cookiecutter …`).
- **phase-0.md Task 1.**
  - Native REAPER is done. Put the `reaper.ini` keys above into the docs (Dummy Audio, plugin paths).
  - Note that the Ubuntu Carla has no CLAP.
  - clap-validator: use the official 0.4.1 binary (ignore the inner tarball name) or a source build (MSRV 1.95.0).
  - cmake is available through `uv tool run --from cmake` for Steinberg's validator.
- **phase-0.md Task 2 and ADR-0004:** `Ubuntu-font-1.0` instead of `LicenseRef-UFL-1.0`; decide on `BSD-1-Clause`. ADR-0004 needs a dated amendment.
- **ADR-0002:** replace "historically redraws continuously" with the measured behaviour: 0 frames when idle, about 26 frames/s during automation, and an idle open editor at about +0.2 to +0.5 percentage points of one core in software GL.
- **design.md §4:**
  - Name refresh works in REAPER through CLAP `RESCAN_INFO` only, and needs the patch.
  - VST3 hosts may reject `kParamTitlesChanged` (REAPER does).
  - Existing automation-lane labels don't update, but stay bound by ID.
  - Names must be re-derived from the patch on load.
- **research.md §3:** add Carla's missing CLAP support in 2.5.x, the clap-wrapper-rs lag (0.3.1 bundles 0.14.0 and is missing the 0.16.0 VST3 parameter-sync fix), and the up-to-date version list (unchanged since the first report).
- **What stays unchanged:** Rust, the native non-WebView UI, CLAP first with VST3 second, egui as default, clack-host for the test host. Nothing in the evidence argues for changing the shell.

## Gaps — what I couldn't verify, and how to settle it

| Gap | Why | Settles it |
|---|---|---|
| Editor on the real NVIDIA display: #95, HiDPI, keyboard focus, resize, GPU cost | Rule: never open windows on the real display (after the Xorg crash) | Owner checklist 1–3 |
| CLAP poly-mod behaviour in a host (d) | No Bitwig; REAPER has no poly-mod UI | Owner checklist 5 |
| Ardour (VST3) and CLAP in Carla | Ardour not installed; Ubuntu's Carla 2.5.8 has no CLAP | Install Ardour 9.8, or build Carla from main (heavy) |
| Real audio device (xruns, latency) | Only Dummy Audio used | Owner checklist 4 |
| Idle CPU under an exclusive lock in RUN3 | Lock starved for 15 min; measured under a shared lock instead (RUN2 had one exclusive run) | Repeat `cpu.lua` on a quiet machine |
| Whether REAPER's UI lane label follows a rename | API and saved project show the old label; UI not inspected | Owner checklist 6 |
| Effort to switch to clack | Estimate only | Port `spike-np` to clack + clap-wrapper (git, 0.16.0) and count the days |
| nice-plug organisation membership | Codeberg API needs a token | Log in to Codeberg, or ask on Discord |
| Cockos wiki "REAPER for Linux" | Bot check; the recipe above was measured directly instead | Read in a browser (optional) |
| RUN2 files | Lost in the 03:12 crash; RUN3 reproduced the results except the exclusive-lock CPU run | — |

### Owner checklist (needs your hands and eyes)

Bundles and scripts are in `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/bundles/` and `reaper-tests/`. Copy the bundles to `~/.clap` and `~/.vst3`, or point REAPER at that folder.

1. **Real display, REAPER:** insert `enth spike-np-gui` (CLAP, then VST3) 10 times on a fresh track. Is the editor ever blank or grey? Resize it. Close and reopen it 20 times.
2. **HiDPI and keyboard:** with Cinnamon at 100% and then 200%, is the editor sharp and correctly sized? Does the spacebar still start REAPER's transport while the editor has focus?
3. **Idle cost on NVIDIA:** editor open and idle for 60 s; run `pidstat -p $(pgrep reaper) 1 60` and `nvidia-smi dmon -c 60`. Compare with the editor closed.
4. **Audio:** set REAPER to PipeWire/JACK, play notes into `enth spike-np`, listen for clicks, and check the xrun counter.
5. **Bitwig trial:** load the CLAP build, add a per-voice modulator on Macro 1, and confirm voices modulate independently. Save and reload.
6. **Rename in the UI:** load `enth spike-np-rename` (CLAP), create an automation lane for Slot 1, then set Slot 64 to its maximum. Do the FX parameter list and the lane label show "Osc 1 · Cutoff"?
7. **Decide:** carry the rename patch in a fork, or accept "Slot N" everywhere?
