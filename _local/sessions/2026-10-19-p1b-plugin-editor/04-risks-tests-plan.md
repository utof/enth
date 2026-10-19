# 04 — Risks, prior art, tests and the P1b lane plan

- **Date:** 2026-10-19
- **Sub-researcher:** 04 (risks, prior art, test strategy, P1b lane plan), P1b research session.
- **Questions answered:** all five in the brief:
  1. prior art for Rust plugins with real editors, and host quirks;
  2. lessons from P1a's run in this repo;
  3. the P1b test strategy;
  4. a P1b lane plan;
  5. the top 8 risks, each with its cheapest retirement.
- **Method:**
  - Read: brief.md, AGENTS.md, docs/README.md, roadmap.md, phase-1a.md, architecture.md, design.md §3–§5, p1a-g1.md, benchmarks.md (UI thresholds), the three spike docs (shell, ui, test-host), owner-checklist.md, the layout handoff, ADR-0001/0002 (grep), and the P1a report 04.
  - Local, read-only: `git log` and merge messages; the P1a lane scratch notes in `~/code/enth-scratch/p1a/*/notes.md`; the coordinator's `~/code/enth-scratch/gate.sh`; `xtask/src`; `Cargo.toml`; `free`, `uptime`.
  - Web, all on 2026-10-19 with User-Agent `enth-research/0.1`: the Codeberg API (nice-plug), `gh api` (baseview, nih-plug, Actuate, OctaSine, clap-validator, egui, Carla, culsynth), the crates.io API, WebSearch and WebFetch.
  - No builds.
  - Late in the session I read sub-researchers 01, 02 and 03's scratch `notes.md` to cross-check. Where I use them they are cited as "01's notes (unpublished)" and so on, never as my own measurement.
- **Labels:** *measured* (local output, or numbers read from an API or git), *documented* (docs, source, issue text by maintainers or reporters), *anecdotal* (forums, single user reports), *estimated* (my arithmetic), *inference*.
- **Scratch:** `~/code/enth-scratch/2026-10-19-p1b/04/notes.md`.

## Answer in brief

- **P1a ran at about a quarter of its estimate.** From commit times and scratch-file times, the 16 P1a lanes (S0 to E, plus an unplanned integration lane "CH") took about **18 agent-hours**, against the plan's 52–93. Wall time from S0's start to E's merge was about **7.3 h**, against a critical-path estimate of 18–35 h (*measured* from git, durations *estimated* from file times). Size P1b from P1a's actual times, not from its estimates.
- **P1a's merge pain had three sources, and all three can be designed out:**
  - Shared composite goldens (`engine-chord`, `ref-saw-*`) were changed by S2b, FXW, D1 and D2. The coordinator regenerated them "on the merged tree" at three merges (merge messages 9ce47f6, 0fe4edb, 7393944).
  - Bench budgets were set in one worktree and broke in others. FXW and D2 found S2a's `saw_16v` budget exceeded by about 1 % on a clean HEAD in their environments; D1's merge "re-measured on the merged tree".
  - `Cargo.lock`: Phase 0's UI lane conflicted on it twice. P1a's S0 pre-declared every dependency, and the lock then stayed quiet except for one coordinator commit (T's fundsp).
- **The gate lesson.** `gate.sh` runs under `set -euo pipefail` and says it has "no `&& echo` traps": in `cmd | tail -1 && echo ok` the status is `tail`'s, so a failing `cmd` still prints "ok". Lanes were told to "check `$?` itself".
  - The full gate took 3 min 54 s warm, and more than 595 s cold (lane T).
  - D2's gate run was moved to the background once it passed 600 s.
  - P1b adds nice-plug, egui, wgpu and clack to the workspace build, so it needs a two-tier gate: a fast lane gate and a full coordinator gate.
- **P1b's spine is not just "session types first".** It needs three things before the fan-out:
  - **engine seams** in the P1a spine crate. 01 and 02's notes (unpublished) find no epoch on parameter events, no depth event, no per-voice macro offset for poly-mod, no modulated values exposed for the editor's rings, and a `NOTE_END` frame that can fall past the block end;
  - the **session API**, reviewed before the fan-out;
  - a **walking skeleton**: plugin → session → engine, a state round trip, clap-validator, a REAPER run and "plugin render = engine golden" in the test host.

  The freezes are F0b, F1b and F2b, and after F2b changes are additive only.
- **Heavy builds bind P1b, not lane count.** Almost every P1b lane pulls nice-plug, egui or clack-host, and AGENTS.md allows 2 heavy builds at once. When I checked at 18:15 the machine had 5.6 GiB available, 14 GiB of swap in use and a load of 17.5 (*measured*). The plan therefore pairs one heavy and one light lane per slot, and gives harness lanes a shared bundle cache so they never rebuild the plugin.
- **The cheapest P1b signal is a plugin-level golden that *is* the engine golden.** Render existing `tests/golden/cases` through the plugin in the test host and assert the engine's hash. That gives CLI = plugin equality with no new hash files, so a later DSP change can't cause plugin-golden churn.
  - Known traps: CLI velocity is `v/127` while CLAP sends an f64 (01's notes); `Performance` has no macro events yet; the seed must be `enth_render::DEFAULT_SEED`; event frames must be absolute.
- **clap-validator 0.4.1 already tests the riskiest P1b wiring.** `state-reproducibility-basic/buffered/binary` randomise parameters *through `process()`*, save, recreate, reload and compare; `binary` requires byte-identical state ([source](https://github.com/free-audio/clap-validator/blob/master/src/tests/plugin_instance.rs), 0.4.1 released 2026-07-22, *documented*). nih-plug's own gain example fails four of these tests ([#279](https://github.com/robbert-vdh/nih-plug/issues/279), 2026-09-13). This is the cheap automated check for the "two sources of truth" risk (#1).
- **Framework status (2026-10-19):**
  - nice-plug 0.4.0 fixed the state-length abort, non-finite parameter events, event-queue allocation and the block-split bug (#85, #86, #87, #91, [CHANGELOG](https://codeberg.org/RustAudio/nice-plug/src/branch/main/CHANGELOG.md), *documented*).
  - The blank first open in REAPER ([#95](https://codeberg.org/RustAudio/nice-plug/issues/95), reported on an NVIDIA 595 driver) is reported fixed by the baseview 0.4.1 update, i.e. nice-plug-egui 0.6.0, which enth already pins. That rests on one user's comment of 2026-10-19 (*anecdotal*), and the issue is still open.
  - Still open: `VoiceID::id_or_fallback` ignores the host's voice id on main (*documented*, source); plugin-initiated parameter changes ([PR #15](https://codeberg.org/RustAudio/nice-plug/pulls/15), open since 2026-05-03, maintainers reluctant); the baseview xkb keymap leak, still on baseview master (*documented*, source read 2026-10-19).
  - baseview is churning: [PR #357](https://github.com/RustAudio/baseview/pull/357) removes the X11 event thread (open, updated 2026-10-19). Keep the exact pins through P1b.
- **Host quirks that shape the tests:**
  - REAPER asks for plugin state on every parameter change, for its undo (JUCE forum, 2023-09-10, *anecdotal*), and its undo loads state back. So state save must be cheap, and a state load while the editor is open is a normal event.
  - Bitwig's trial is 30 days, needs an account and an internet connection, and disables saving once it expires ([Bitwig](https://bitwig.com/support/shop_license_activation/48), *documented*, undated page). Start it only once poly-mod is built.
- **Owner's P1b time:** about **2.5–3.5 h**, plus an optional 45 min for Bitwig.
  - Two owner builds: after F2b, the real-display checks, which retire Phase 0's C1, C4, C5 and C6 by running them on `enth-plugin` instead of the spikes; after the modulation lane, "a bass, pad, pluck and lead you like" and real-audio xruns.
  - Plus a layout pick when the Claude Design variants arrive, and two short decisions with defaults (the fork, the baseview report).
- **Size:** about **27–51 agent-hours** in total, with a critical path of about **13.5–24**, plus 2–4 h per layout variant later (*estimated*). That is 1.5–2.8× P1a's actual 18 h, because host and UI loops are slower and two heavy slots serialise the work.
- **The top 8 risks:**
  1. two sources of truth for parameter values;
  2. nice-plug threading pitfalls;
  3. real-display failures that agents can't see;
  4. editor churn: slow feedback, binary snapshots, layout variants arriving mid-phase;
  5. merge hotspots, build contention and a slow gate;
  6. CLI ≠ plugin renders;
  7. leaks and real-time regressions in the shell;
  8. owner bandwidth on G1.

  Each has a retirement that costs under a day, and most are tests in the spine lanes.

## Findings

### Q1 — Prior art: Rust plugins with real editors, pitfalls, host quirks

Licences and activity are from `gh api` and the Codeberg API on 2026-10-19 (*measured*). Issue contents are *documented* (reporter or maintainer text) unless marked.

| Project | Licence, activity | What it shows | What slowed or broke it | Copy / avoid in P1b |
| --- | --- | --- | --- | --- |
| [nice-plug](https://codeberg.org/RustAudio/nice-plug) (enth's shell) | ISC. 0.4.2 on 2026-09-14; nice-plug-egui 0.6.0 on 2026-10-18 | The `gain_egui` example is the reference editor wiring: `ParamSetter` gestures, `#[persist]` state, `EguiState` | #95 (2026-09-17, open): REAPER on NVIDIA 595 / GNOME showed a blank or transparent UI on first add. "`build()` is called … but `ui()` is not." A user reports it fixed at 7197104, the baseview 0.4.1 update (2026-10-19 comment, *anecdotal*). #91 (2026-09-08): a parameter event first in the block was applied too early. #87: event queues grew inside `process()`; fixed by reserving capacity, but "frame count doesn't bound event count". #85: a bad state length aborted the host. #86: NaN from the host poisoned parameters. #76: Carla calls VST3 `attached` from another thread (panic). #77: resize grew without bound on Linux Mint. All fixed in 0.4.0 or later per the CHANGELOG | **Copy:** reserve event capacity, reject non-finite values, rescan values after a state load (now built in). **Avoid:** relying on `editor_closed` (never called), `execute_gui` from `process()`, `GuiContext::set_state` (01's notes: it applies on the audio thread), and `id_or_fallback` |
| [nih-plug](https://github.com/robbert-vdh/nih-plug) (upstream, maintenance mode) | ISC framework. Pushed 2026-09-03; 123 open issues | Same wiring | [#92](https://github.com/robbert-vdh/nih-plug/issues/92) (2023-11): a preset browser setting ~150 parameters through `ParamSetter` crashed VST3 hosts (Waveform, Ardour, Bitwig). The maintainer: "Don't try to load presets using automation". [#254](https://github.com/robbert-vdh/nih-plug/issues/254) (2026-02-01): `gain_gui_egui` produced an undo-point loop and a slider snap; the issue text I read doesn't name the host. [#222](https://github.com/robbert-vdh/nih-plug/issues/222) (2025-07-01): a wrong `Arc` count meant the plugin's drop never ran, leaking up to hundreds of MB per instance. nice-plug merged "Fix background/gui task executor memory leaks" (PR #4, 2026-03-27); whether that is the same bug is *inference*. [#98](https://github.com/robbert-vdh/nih-plug/issues/98): crash or blank on the second UI open on Debian (baseview). [#271](https://github.com/robbert-vdh/nih-plug/issues/271) (2026-05-15): black VST3 UI in Bitwig on Linux; a reply says "software GL may fix" (*anecdotal*). [#279](https://github.com/robbert-vdh/nih-plug/issues/279): the gain example fails 4 clap-validator state tests | **Copy:** load patches through state, never through host automation. **Test:** instance create/destroy churn for leaks, and second-open checks |
| [Actuate](https://github.com/ardura/Actuate) | GPL-3.0 (ideas only). Pushed 2026-06-04; 152 stars | nih-plug and egui with a large editor (generators, filters, FX, presets) | [#48](https://github.com/ardura/Actuate/issues/48) (2024-09): "doesn't set generators again on project reopen … the dropdown just changes … to Off" while the data was present (a state-restore ordering bug; fixed in 1.3.4). [#8](https://github.com/ardura/Actuate/issues/8): in Bitwig, next/previous preset stepped by 2. [#5](https://github.com/ardura/Actuate/issues/5): a file dialog opened on the GUI path stalled audio in Bitwig. [#22](https://github.com/ardura/Actuate/issues/22): MIDI behaved differently in Bespoke, Ardour and Carla, while nih-plug's `poly_mod_synth` worked in all three. [#62](https://github.com/ardura/Actuate/issues/62) (2026-05): it still runs a custom nih-plug fork | **Avoid:** restore paths that differ from the edit path (one `apply` for both); blocking work on the GUI thread; a fork with no upstream plan. **Test:** state round trip of a *non-default* patch in a real host |
| [OctaSine](https://github.com/greatest-ape/OctaSine) | AGPL-3.0 (ideas only). Last push 2024-07 | An FM synth with its own GUI, envelope editing on curves | [#145](https://github.com/greatest-ape/OctaSine/issues/145) (2023): no CLAP GUI in Bitwig on Linux. [#91](https://github.com/greatest-ape/OctaSine/issues/91) (2022, open): picking envelope points fails "where multiple [are] close together … (like a sustain level of 0 and a short release)". [#76](https://github.com/greatest-ape/OctaSine/issues/76): UI state (the theme) lost on reopen in Bitwig. [#18](https://github.com/greatest-ape/OctaSine/issues/18): HiDPI, crash on window close in Bitwig | **Test:** handle picking at coincident points (enth now sculpts envelopes). **Design:** editor view state lives in the session, not the editor, because the editor is rebuilt on every open |
| [culsynth](https://github.com/rbmj/culsynth) | MIT. Pushed 2026-01-22 | One egui editor for a nih-plug plugin, a PipeWire standalone and a web AudioWorklet. The editor talks through narrow traits (`MidiHandler`, `ContextReader`) | Only VST3 tested (README) | **Copy:** the editor depends on a narrow session trait, so kittest and a dev window can drive the real editor without a plugin |
| [GlicolVerb](https://www.vmunix.com/posts/glicol-verb-first-release) | Blog (P1a report 04 dates it 2025-12-30) | nih-plug and egui | "The hard part was the UI": a text box that works in every DAW; a patched baseview carried for a macOS crash (*anecdotal*) | Budget editor and host time above DSP time |

**baseview, the layer under every egui editor:**

- [PR #357](https://github.com/RustAudio/baseview/pull/357) ("X11: Do not use a separate thread anymore", by clack's author) is open and was updated today. [#342](https://github.com/RustAudio/baseview/issues/342) plans to move all X11 UI and events to the main thread. Lazy redraw (#340) and a `baseview-host` crate (#354) were merged in Sept–Oct 2026.
- So the next baseview release may change threading, frame pacing (the ≈30 fps cap) and focus.
- [#359](https://github.com/RustAudio/baseview/issues/359) (host scaling not reflected on X11; enth's HiDPI clipping) is open.
- The xkb keymap leak: `XkbcommonState::drop` still unrefs only the state on master (`src/wrappers/xkbcommon.rs`, *documented*). No issue is filed. AGENTS.md says the owner posts the report.

**Host quirks, collected:**

| Host | Quirk | Evidence | Consequence for P1b |
| --- | --- | --- | --- |
| REAPER 7.82 | Deactivates, reactivates and resets CLAP plugins about 30 ms into an offline render | shell.md (*measured* locally) | Plugin goldens stay in the test host. REAPER render checks use short retriggered notes |
| REAPER | Rejects VST3 `kParamTitlesChanged`; CLAP rescan renames in 50 ms with the patch | shell.md, ADR-0001 (*measured*) | "Slot N" for VST3 whatever we do |
| REAPER | Calls get-state on every slider move (undo history); undo sets state back | [JUCE forum](https://forum.juce.com/t/how-to-prevent-getstateinformation-being-called-on-slider-movement/57791), 2023-09-10 (*anecdotal*) | State save must be cheap and deterministic. A state load mid-session, with the editor open, is routine. Decide what enth's own undo history does on a host state load |
| REAPER | A VST3 parameter set from a script takes effect only after a process cycle; it adds Bypass, Wet and Delta | shell.md (*measured*) | The harness waits; counts are 75, not 72 |
| REAPER on Xvfb | Space is lost after a click in the editor; HiDPI is clipped | ui.md, benchmarks.md (*measured*, Xvfb) | Owner checks on the real display |
| Bitwig | Trial: 30 days full, account and internet required, once per account and computer; after expiry "save and export will be disabled" | [Bitwig support](https://bitwig.com/support/shop_license_activation/48) (*documented*, undated) | Start the trial only when poly-mod is built (end of lane PL), or the save-and-reload half can't be done |
| Bitwig | Black or missing editors on Linux (OctaSine #145, nih-plug #271); poly-mod needs host note ids, while nice-plug's `id_or_fallback` ignores them | issues above; nice-plug `midi.rs` line 66 on main (*documented*) | A unit test feeds host voice ids; the owner's Bitwig check is optional in P1b (roadmap S1 owns poly-mod polish) |
| Carla 2.5.x | No CLAP host in the Ubuntu build; latest release 2.5.10 (2025-07-31); calls VST3 `attached` from another thread (nice-plug #76) | shell.md; [releases](https://github.com/falkTX/Carla/releases) | Carla stays a VST3 load-and-state check |
| Many VST3 hosts | Crash when a plugin pushes about 150 parameter changes at once | nih-plug #92 | Patch and preset loads go through state; a slot reassignment sets at most one host parameter |

### Q2 — Lessons from P1a's run in this repo

**Timeline** (*measured* from `git log` and merge commits; lane durations *estimated* from scratch-directory file times, because agents' own notes timestamps were estimates, as lane A's notes say):

| Event | Time (2026-10-19) |
| --- | --- |
| Research summary committed | 10:22 |
| S0 starts / F0 merged | ~10:36 / 11:04 |
| FX2, FX1 merged | 11:46, 11:54 |
| S1a merged (F1) / A merged | 12:23 / 12:34 |
| S2a merged (F2) | 13:44 |
| S1b, FXW, S2b, D2, D1, H, C, T merged | 14:19 to 16:11 |
| CH (unplanned integration pass) merged | 16:56 |
| E merged (headless G1 table) | 17:52 |

Lane durations in hours: S0 0.4, FX1 0.75, FX2 0.65, A 1.4, S1a 0.85 plus a 0.3 F1 review, S2a 1.2, S1b 1.9, S2b 1.4, D1 1.8, D2 1.4, FXW 0.9, C 1.4, H 0.8, T 0.9, CH 0.75, E 0.9. That is **about 18 agent-hours** against an estimate of 52–93, and about **7.3 h of wall time** against a critical path estimated at 18–35. Coordinator time (merges and golden regeneration) is not counted. phase-1a.md promised to record actual against estimated hours, and these figures are that record.

**What caused merge pain:**

1. **Golden regeneration across lanes.** P1a's per-module cases worked: no `osc-*`, `svf-*` or `reverb-*` case conflicted. The *composite* cases did not.
   - `engine-chord`, `ref-saw-1v` and `ref-saw-gate` sound through every module and every envelope, so S2b (voice stealing, smoothed automation), FXW (real effects), D2 (a 1 ms minimum ramp) and D1 (real oscillators) each changed them.
   - Each of those merges says "regenerated on the merged tree"; D1's notes "every case".
   - D2 also had to edit S2a's spine-owned `tests/contract.rs`, because a 1.5-segment ramp could not meet the contract's "exact at the end of segment 0".
   - **Lesson:** composite goldens are owned by the coordinator and regenerated once per wave, not per merge. A lane whose change necessarily moves a composite case says so in its report, and the coordinator does the update.
2. **Shared bench budgets.**
   - S2b's first smoother pass went over S2a's budgets (5.68 M against 3.66 M), which forced a fast path inside the lane.
   - Separately, FXW and D2 both measured S2a's `saw_16v` at 44,866,745 instructions on a *clean* HEAD, against its 44.4 M budget: about 1 % over in another worktree, from environment drift (cause not found; *measured* by those lanes).
   - D1's merge re-measured every budget on the merged tree.
   - **Lesson:** a lane never sets an absolute budget on a shared bench. Budgets are re-based by the coordinator on the merged tree under `flock -x`, and lanes report deltas.
3. **`Cargo.lock`.**
   - In Phase 0 the UI lane merged main twice, and both merges conflicted on `Cargo.lock` (plus `ci.yml` and README).
   - In P1a, S0 resolved every dependency up front. Lanes S1a, A, S2a and C each noted "can't add deps, `--locked`" and worked around it, for example lane A kept unused dependencies, and S2a's render tests use power ratios because `enth-render` has no `libm`.
   - Only T needed a new crate (fundsp, +122 lock lines), and the coordinator merged it.
   - **Lesson:** keep this rule, but pre-declare more generously. Lanes paid small workarounds for missing dependencies.
4. **Ownership crossings.**
   - FXW needed accessors in `enth-dsp`, which it didn't own.
   - D2 took over `smoothing.rs`, which the S0 stub said belonged to S2b.
   - S2a's compiler refused routes until S2b, so LFO patch goldens could not exist in D2. They came in CH, a lane nobody planned, which also wired hints into the CLI and cleaned up stale text.
   - **Lesson:** plan the integration lane up front, and list "who may add an accessor to whose crate" (additive only, flagged in the report).

**The gate script:** `~/code/enth-scratch/gate.sh` (last edited 13:45, around F2).

- What it does:
  - runs under `set -euo pipefail`, inside `flock -s /tmp/enth-quiet.lock bash -euo pipefail -c '…'`;
  - unsets `DISPLAY` and `WAYLAND_DISPLAY`;
  - checks clippy's `PIPESTATUS[0]`;
  - captures the test output and fails with its tail;
  - prints `GATE PASSED` last.
- Lane prompts say "exits 0 (check `$?` itself)".
- The trap it closes: `cmd 2>&1 | tail -1 && echo ok` reports `tail`'s status. Without `pipefail`, a failing clippy or test run still prints "ok", and an agent reads "ok".
- What it does **not** run: `cargo xtask golden --wasm`, `bench`, `nextest`, `host-render`, `validate` or `reaper-test`. Lanes ran those by hand.
- Time: 3 min 54 s warm (lane T); lane T's first attempt hit its own 595 s timeout while compiling, and D2's run was moved to the background after 600 s.
- With P1b's heavy crates in `--workspace`, a cold full gate will be longer still (*inference*).
- The user's global instructions warn that backgrounded agents can go idle and never resume.

**How to shape P1b's lanes to avoid all of it:**

- **Plugin goldens equal engine goldens** (test strategy below), so DSP changes never require plugin-golden updates. The few plugin-only cases (slot automation, reset, state restore) are still compared with `enth render` of an equivalent performance where one exists.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshot per layout, and the G1 table.
- **S0b pre-declares everything:** all P1b dependencies, including the fork's `[patch]` lines as a commented block; the empty crates; the module files; the theme token files per area; the xtask commands and CI jobs.
- **Engine changes go through one lane (E+).** No other P1b lane edits `enth-engine` or `enth-render`.
- **One heavy and one light lane per build slot.** Harness lanes reuse a release bundle that the coordinator builds after each merge (`~/code/enth-scratch/bundles/<sha>/`).
- **A two-tier gate:**
  - `gate.sh lane <worktree> <crates…>`: fmt, workspace clippy, `nextest -p` on the lane's crates, check-layers. Target under 5 minutes.
  - `gate.sh full`: the coordinator's, with workspace tests, wasm-check, deny, golden, `host-render`, `validate`, and `reaper-test` separately under Xvfb. It runs in the background with an explicit wake-up, or in the foreground with a timeout above its measured length.
  - Both decide only by exit status, and both print the commit they checked.

## Test strategy

### The layers, by cost of signal

Costs are *measured* in Phase 0 and P1a unless marked.

| Layer | What it proves | Cost per iteration | Where it runs | Notes |
| --- | --- | --- | --- | --- |
| `enth-session` unit and property tests | Op routing, undo, the slot bridge, state, ring contracts | About 1–2 s (light crate, no egui; *estimated* from P1a's 0.5–2 s per crate) | Every edit | The fastest P1b loop; most product logic should live here |
| `enth-ui` kittest interaction tests (accessibility queries, no pixels) | Gestures, ops emitted, handle picking | About 1.2–2 s rerun (ui.md: test 1.0 s, rerun 1.2 s) | Every editor edit | Drive the real editor with a real `Session`, no plugin |
| kittest snapshots (lavapipe) | Look and layout | Edit to PNG 1.9–2.3 s (ui.md) | Every editor edit; CI | Binary files of about 300 KB each (02's notes): one per component or region |
| `enth-plugin` lib tests (param mapping, state serde without a host) | The shell's translation | Debug incremental 1.3–2.9 s (shell.md) | Every shell edit | nice-plug is heavy only when built cold |
| `cargo xtask host-render` (bundle plus test host) | Plugin = engine goldens; state restore; reset | 25 ms per 2 s render; the bundle build dominates | Plugin edits; CI | Exact. Deterministic block size and events |
| clap-validator 0.4.1 | State reproducibility, parameter fuzzing, block sizes, reset | Seconds (36 tests in Phase 0) | Plugin edits; CI | Run on the **debug** bundle, so `assert_process_allocs` aborts on any allocation |
| `cargo xtask reaper-test --package enth-plugin` | Real host: state, automation, editor smoke, both formats | About 45 s warm (shell.md) | Locally, before merging plugin lanes | Xvfb; not CI |
| `enth-test-host editor-cycles` | Editor leak and thread slope against a no-editor baseline | 200 cycles ≈ 3 min 12 s (test-host.md) | Before merging editor lanes; nightly | Could run in CI under Xvfb (*inference*; CI has no real display risk) |
| Instance churn (new) | Leaks per instance create/destroy (nih-plug #222's class) | Minutes (*estimated*) | Nightly | Test host, no editor |
| Test-host timing (03) | p99/p99.9 `process()` at 64 frames, plan-swap blocks | Minutes, under `flock -x` | On demand; G1 | 03's notes: swap blocks 203–416 µs today against a 1,333 µs deadline |
| Owner, real display and real audio | First open, focus, HiDPI, GPU cost, xruns, "sounds you like" | Owner minutes | Two owner builds | Not automatable on this laptop by rule |

### Session unit tests (`enth-session`)

- **Effect routing, table-driven:** for every `Op` variant, the effect the session takes: a value event; a depth event (once E+ adds one); a macro event; a recompile and swap; or none (`SetSlot`, labels, meta).
  - 01 and 02's notes found that `Effect::Depth` has no engine path today, and that depth-0 routes are left out of the compiled plan, so going from 0 to a non-zero depth needs a recompile. The table makes that explicit.
- **Undo and redo through the session match the patch model.** A proptest state machine over op sequences, gestures and undo/redo: the session's patch always equals the reference model's, and each gesture is one history entry (02's notes measured one drag = one entry).
- **Never blocks, never drops:**
  - with an engine that never drains its rings, 1,000 edits leave the session responsive, with plans pending, not dropped;
  - plans created equals plans dropped (a counter) once the engine drains;
  - old plans are freed on the main thread only.
  - Also decide who drains the return ring while the editor is closed and no timer runs, for example on every main-thread entry: `apply`, `state save` and `state load`.
- **Epochs:** after a structural edit, a host-automation event addressed through the *old* `PlanMap` must not reach the new plan's table. 01's notes found `EventKind::Param` carries no epoch today; E+ adds one, and the session test proves it end to end.
- **The host-parameter bridge:**
  - normalised slot value → target plain value through `enth-params`, and back;
  - assign, unassign and reassign a slot;
  - a slot pointing at a removed module is cleared by the op's cascade;
  - display names ("Osc 1 · Cutoff") are derived only on the main thread;
  - a rescan is requested after a slot change and after a state load.
- **Host state load mid-session:**
  - loading state while the editor model exists rebuilds the model;
  - the history policy is explicit (cleared, or one undoable "load" entry) and tested;
  - the dirty flag is right.
- **Fuzz-lite:** arbitrary and mutated state blobs never panic and give a typed error; a size cap (nice-plug #85's class); fixtures for every state version.

### Plugin state round trip

Save, then load, must give an identical patch (exact structural equality and the exact hash), identical slots and macros, and the same editor view state if that is persisted.

1. **In process, no host:** the plugin's persist field serialises the session state, and a fresh instance deserialises it.
   - Generate patches, slot tables and host values with proptest.
   - Assert exact equality, and a byte-identical second save, which is what clap-validator's `binary` test needs.
   - JSON with deterministic field order; no `HashMap` iteration order in persisted types (*inference*).
2. **Host values folded in:** automate slot 3 and macro 1 through `process()`, then save. The saved state must hold those values, and after a reload the host must read them back.
   - This is exactly clap-validator's `state-reproducibility-basic`.
   - It fails if the session's `Patch` and nice-plug's parameter atomics disagree (risk #1).
3. **The test host:** `render --state patch-state.json` then `--save-state`, then render from the saved state. Both hashes must equal each other and the engine golden for the same patch.
4. **REAPER:** `state-create` and `state-reload` with a **non-default** patch (extra modules, routes, three assigned slots, a renamed macro), in a new REAPER process, for CLAP and VST3. Actuate #48 is the cautionary case: default patches hide restore-order bugs.
5. **Carla (VST3):** load, set, save, reload; a byte-identical chunk, as in Phase 0.

### REAPER harness scenarios (`cargo xtask reaper-test --package enth-plugin`)

Parametrise the xtask first: today `xtask/src/reaper.rs` and `host_render.rs` hard-code `const PACKAGE: &str = "spike-shell"` (*measured*). Then:

| Scenario | Checks |
| --- | --- |
| `state-create` / `state-reload` (existing, retargeted) | 75 parameters, names, idents stable across processes and formats; envelopes survive; the decoded state equals the patch written |
| `patch-roundtrip` | A non-default patch (above) survives a save and a reload in a new process, checked by decoding the chunk and comparing it with the source JSON |
| `slot-automation` | Assign slot 1 to `svf.cutoff`, automate slot 1 from 0 to 1 over 2 s with short retriggered notes (REAPER's restart rule); the render is at least 2× brighter at the end, as in Phase 0's check |
| `macro-automation` | Macro 1 drives a route; level or brightness changes as the route says |
| `undo-state-load` | Change a parameter through the plugin, call REAPER's undo action; the plugin's patch reverts, and the editor (if open) shows it |
| `two-instances` | Two instances with different patches; save, reload, each keeps its own |
| `editor-smoke` (existing) | 5 open/close cycles with a full patch; each draws |
| `cross-format` (existing) | CLAP and VST3 report the same ids; a CLAP-saved project reloads |

Not in the harness: keyboard focus and Space (Xvfb has no window manager, so it can't decide this), HiDPI (it can show the clipping but not judge a real display), and xruns (Dummy Audio). These are the owner's.

### Test-host plugin goldens, and CLI = plugin equality

- **The rule:** a plugin-level case is an engine golden case played through the plugin. `cargo xtask host-render --package enth-plugin` reads a list of `tests/golden/cases/*.toml` names, renders each through the bundle with its patch as state and its performance as the notes and automation script, and asserts the **engine's** expected hash.
- **Plugin-only cases** (host automation of a slot, `reset()`, voice steal under host events, the monitor FX bypass, which `enth-render` cannot set by design) are compared with `enth render` of an equivalent performance where one exists. Otherwise each gets its own one-file case with a hash.
- **Pitfalls to design out** (01's notes, unpublished, plus *inference*):
  - **Velocity:** the CLI uses `v/127` and nice-plug passes CLAP's f64 as f32. Use velocities exactly representable as `k/127` in shared cases, or convert in one place (the session) for both shells.
  - **Event frames:** the engine quantises absolute frames to `ceil(frame/32)·32`. The plugin must compute block start + offset, and the CLI's seconds-to-frames rounding must match. Test with host blocks of 1, 64, 100 and 4096, and event times off the grid.
  - **Macro and mod-wheel events:** `Performance` has none (01's notes), so E+ adds them before macro cases can be shared.
  - **Seed:** the plugin uses `enth_render::DEFAULT_SEED` (`0x0E17_5EED`, *measured*).
  - **Smoothing:** host automation must take the same engine path as `Performance::automation`.
  - **NOTE_END:** its frame can fall past the block end (01's notes). The plugin clamps it; the audio is unaffected.
  - **Sample rate:** 48 kHz for shared cases.
  - **FTZ:** nice-plug sets it only with `unsafe_flush_denormals` (01's notes), and the engine is FTZ-invariant anyway (design.md §5).
- 01's `det-proto` (enth-engine in a nice-plug shell against `enth-render`) was in progress when I read its notes. Its result is the first data point for this test.

### kittest editor snapshots and interactions

- **Drive the real editor with a real `enth-session`.** Use the engine in process with manual `process()` calls or a fake link, and no plugin. The editor depends on a narrow session trait (culsynth's pattern), so the same code runs in kittest, the dev window and the plugin.
- **Interaction tests, preferring accessibility queries over pixels:**
  - a knob drag gives one history entry and balanced host gestures;
  - a discrete parameter (polyphony) follows a slow drag (02's notes: sub-step deltas are lost today);
  - a layout switch mid-drag keeps the drag and closes its gesture. 02's notes measured that automatic ids lose the drag and leave the gesture open, while stable ids keyed by (module id, parameter address) keep it;
  - drag-to-modulate creates a route and a depth;
  - envelope handles at coincident points pick the expected handle (OctaSine #91);
  - a disabled card dims and stays editable;
  - a state load while open updates every control;
  - undo and redo;
  - the slot "Automate" menu emits `SetSlot`;
  - repaint requests are the exact delays (the spike's tests), and an idle editor schedules nothing.
- **Snapshots:**
  - one PNG per component (cards per module kind, the macro bar, tiles);
  - plus, per layout, the whole editor at the P1 default patch and at the maximum (4 generators, 4 voice effects, 4 effects, 8 modulators);
  - live displays fed deterministic telemetry or masked (`Harness::mask`, [egui #7467](https://github.com/emilk/egui/pull/7467), merged 2025-08-21);
  - threshold 0.6 and 0 failed pixels by default (02's notes); Mesa drift handled with `kittest.toml`;
  - gotcha: `get_by_label` can return a control outside a scroll area's clip, so call `scroll_to_me()` first (02's notes).

### Leak and xrun checks

- **Editor leak:** `editor-cycles` on `enth-plugin`, 200 cycles, `--mode both`.
  - Gate: editor minus baseline ≤ 87 KB per cycle on stock baseview (Phase 0's 76.9 + 10 margin), or ≤ 26 KB with the xkb fix (16.1 + 10). Threads constant.
  - The fork lane tightens the number. Attribute with `valgrind --leak-check=full`, since heaptrack segfaults once a window opens.
- **Instance churn:** create, activate, process, deactivate and destroy ×200 with no editor. The PSS slope stays within the baseline's (0.2 KB per cycle) plus noise. This catches the nih-plug #222 class.
- **Plans:** the session's counters (above).
- **Real-time safety in the shell:**
  - debug bundle under clap-validator, with `assert_process_allocs` aborting on an allocation;
  - a test-host event burst (10,000 events in one block; nice-plug #87 says frame count doesn't bound event count);
  - a plan swap while the editor is open and animating;
  - `render --process-on caller` with the editor open, the arrangement where nice-plug runs GUI tasks inline.
- **xruns:** automated runs use Dummy Audio, so they can't count xruns. The proxy is 03's p99.9 `process()` in the test host. The real check is the owner's C3.
  - 03's notes: the owner's PipeWire config sets min-quantum 1024, so a 64-sample graph needs the owner to change it, as the checklist already says.

### What gives fast, reliable signal

1. `cargo nextest run -p enth-session` (1–2 s, light). Put rules here, not in the shell or the editor.
2. `cargo test -p enth-ui` kittest *interaction* tests, by accessibility label (about 2 s).
3. Component snapshots for the edited widget (about 2 s per PNG).
4. `cargo test -p enth-plugin` (incremental 1.3–2.9 s).
5. `cargo xtask host-render --package enth-plugin` (engine-golden equality plus plugin-only cases) and `cargo xtask validate`, on the debug bundle.
6. `cargo xtask reaper-test --package enth-plugin` (about 45 s) before merging plugin lanes.
7. `editor-cycles` and instance churn (minutes), before merging editor lanes and nightly.
8. Owner builds.

Reliability rules carry over from P1a: no wall-clock assertions, no random seeds in the gate, no test that depends on another lane's snapshots or goldens; a flaky test is a bug. Two are new: **a pixel snapshot is never the only assertion for an interaction**, and **every gate step is judged by its exit status**.

## Proposed P1b lane plan

### What changes from P1a

- The spine includes **engine seams (E+)** beside the session types, because P1b needs depth events, epochs on parameter events, per-voice macro offsets, modulated values for the editor, macro events in `Performance`, a `NOTE_END` clamp, and probably 03's reverb-state move at a swap.
- **Heavy builds are the bottleneck:** at most 2 heavy lanes (nice-plug, egui, clack-host) at once, each paired with a light lane.
- **A planned integration lane (INT)** replaces P1a's improvised CH.
- **Owner builds are scheduled:** one after F2b and one after the modulation lane.

### Freezes

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0b** | S0b: P1b dependencies pinned (nice-plug `=0.4.2`, nice-plug-egui `=0.6.0`, egui `=0.37.0`, the clack crates the test host already uses); the fork's `[patch]` block commented out; empty `enth-session` (L6) and `enth-plugin` (L8, `cdylib` + `lib`); pre-declared module files; theme tokens split into per-area files; xtask `--package` for `bundle`, `host-render` and `reaper-test`, and `validate` (pinned clap-validator 0.4.1, sha256-checked); CI jobs; `gate.sh lane` and `gate.sh full` | The Done-means gate is green, judged by exit status |
| **F1b** | S1s: the `enth-session` API (session, effects, history glue, slot and macro bridge, state blob v1 with fixtures, editor read model and edit trait, telemetry reader types, the threading contract). E+ merged | A judge agent (Opus) reviews the session API and E+'s engine additions together |
| **F2b** | SK: plugin → session → engine end to end; 72 parameters; state v1 round trip; one real card in the editor; three engine goldens equal through the plugin; clap-validator with 0 failures (debug and release); `reaper-test --package enth-plugin` green | After F2b, session, engine and plugin types change only additively |

### Lanes

Sizes are agent-hours (*estimated*). I sized them at about 1.5–2× P1a's actual per-lane times (0.4–1.9 h), for heavier builds (01's notes: a cold fork build took 2 min 38 s wall at 544 MB) and slower host and UI loops. H = heavy build, L = light.

| Lane | Owns | Depends on | Acceptance | Size | Owner |
| --- | --- | --- | --- | --- | --- |
| **S0b** Workspace prep → F0b (H once) | Root `Cargo.toml` deps and `Cargo.lock`; empty crates and module files (`enth-session`: model, apply, bridge, state, link, telemetry; `enth-plugin`: params, process, notes, state, editor; `enth-ui/src/editor/`: regions, layout, cards); per-area token files; xtask commands and CI; the gate scripts | P1a main | Gate green; `xtask validate` and `host-render --package spike-shell` work unchanged; a probe adding a dependency in a lane fails `--locked` | 1–2 | **Decide:** fork yes/no (5 min; default: stock, "Slot N") |
| **E+** Engine seams (L) | `enth-engine` link, command and telemetry additions; `enth-render` `Performance` macro and mod-wheel events | P1a main (beside S0b) | All 72 goldens unchanged; parameter events carry an epoch and stale ones are ignored; a depth event equals a recompiled render once smoothing settles; per-voice macro offsets (poly-mod) tested; telemetry ring overflow under `assert_no_alloc`; `NOTE_END` frames inside the block; reverb state moved, not copied, at a swap (03's prototype: swap blocks 203–416 → 74–191 µs) | 2–4 | No |
| **S1s** Session types → F1b (L) | `enth-session` public types and traits; `docs/session.md` | F0b | Compiles with stubs; state blob v1 fixture; the threading contract written (who calls what on which thread; who drains the return ring when the editor is closed); judge review | 1.5–3 | No |
| **UI-C** Catalogue cards (H) | `enth-ui` cards for every catalogue kind, built from `ParamSpec`s; stable ids keyed by (module id, parameter address); accumulation for discrete knobs; ADSR handles on real scales; region components with no layout; the token gallery | F0b (the P1a catalogue is frozen) | Component snapshots, one PNG each; interaction tests (discrete drag, coincident handles, stable ids across a re-layout); contrast and no-raw-values tests pass | 2–4 | No |
| **TH** Test-host extensions (H) | `tools/test-host`: `--state` from a patch file, golden-case input (`cli-equal`), instance churn, event bursts, `NOTE_END` checks, a timing histogram (with 03), knob → sound latency | F0b | Each mode proven on spike-shell, with a negative control (a leaky or unequal plugin is caught) | 2–4 | No |
| **FK** nice-plug fork (H; only if the owner says yes) | Forked nice-plug (rename patch, an `editor_closed` `Drop` fix) and baseview (xkb unref) via `[patch]`; ADR-0001/0002 notes | F0b, owner decision | `editor-cycles`: `editor_closed` calls = N and ≤ 26 KB per cycle; a CLAP rename visible in REAPER through the harness; `CARGO_BUILD_WARNINGS=deny` holds (01's notes: vendored path dependencies are not lint-capped) | 1–2 | Decision above |
| **SK** Walking skeleton → F2b (H) | Minimal session impl (value routing, initial compile, state save and load); `enth-plugin` (72 parameters, notes, process through the link, persist, rescan after load, one card in the editor) | F1b, E+ | The F2b row above | 3–5 | No |
| **S2s** Session complete (L) | Structural ops → recompile and swap; undo and redo with gestures; the slot and macro bridge with names and rescan requests; state versions and migrations; the host-state-load history policy | F1b (finish after F2b) | The session tests above, including the never-blocks and epoch tests | 2–3 | No |
| **PL** Plugin complete (H) | Note tuple, `NOTE_END`, voice-info, macro poly-mod with host voice ids (not `id_or_fallback`), the monitor FX bypass control, slot names (rescan if FK, else "Slot N"), VST3 parity, `INPUT_EVENT_CAPACITY`, reset and reactivate | F2b | clap-validator with 0 failures on the debug bundle; test-host bursts, reset and `NOTE_END` checks; poly-mod id unit tests; plugin-only `host-render` cases | 2–4 | No |
| **ED1** Editor wiring (H) | Editor from UI-C components bound to the session; macro bar, voice and master, undo and redo, the slot "Automate" menu, the status line, envelope sculpting, disabled dimming; **the layout seam** with "Lanes" as layout 1; the dev window | F2b, UI-C | Interaction tests through a real session; whole-editor snapshots at the P1 default and maximum patches; no scrolling for the P1 default at 1200×800 if feasible (handoff §6) | 3–6 | No |
| **ED2** Modulation and feedback (H) | Drag-to-modulate, depth arcs, the matrix overview, scopes, meters and LFO tiles from telemetry, repaint gating | ED1, S2s, E+ | Route and depth ops from drags; 0 frames while idle and ≤ 30 fps while visible data changes (exact-delay tests); the UI never waits on the audio thread (a stalled engine still gets editor frames) | 3–5 | No |
| **RH** REAPER harness and validate (L, uses the bundle cache) | `xtask/reaper/*.lua`, `xtask/src/reaper.rs` scenarios | F2b | The scenario table above, green on both formats in consecutive runs | 1.5–3 | No |
| **RT** Real time in host (H, per 03) | The p99.9 measurement and multi-instance CPU on the G1 patch; the G1 "p99 < 25 % of the 64-sample deadline" row | PL, E+ | Numbers with load recorded, under `flock -x`; an instruction case for the swap | 1–3 | No |
| **INT** Integration and G1 (H) | Stale docs, `docs/phase-1b.md` status, owner builds and sheet, `editor-cycles` and churn gates on `enth-plugin`, retiring the spike crates from the workspace, `content/presets/`, the P1b G1 rows | Everything | Gate full green; leak gates as above; owner sheet written; G1 table rows filled except the owner's | 2–3 | Owner builds |
| **LV** Layout variant (H; later, one per variant) | `enth-ui/src/editor/layout/<variant>.rs` and its snapshots | ED1's seam, the owner's pick | Snapshots at the P1 default and maximum; switchable in the dev window without a rebuild | 2–4 each | The pick |

**Total: about 27–51 agent-hours**, plus 2–4 per layout variant.

**Critical path:** S0b (1–2) → S1s (1.5–3) → SK (3–5) → ED1 (3–6) → ED2 (3–5) → INT (2–3), about **13.5–24 agent-hours**. Owner gates come on top, and they usually set the wall-clock end.

### Waves, packed into two heavy slots

| Wave | Heavy slots | Light lanes | Starts when |
| --- | --- | --- | --- |
| 1 | S0b | E+ | — |
| 2 | UI-C, TH | S1s, E+ (finishing) | F0b |
| 3 | SK, FK (or UI-C finishing if FK is off) | S2s | F1b and E+ merged |
| 4 | ED1, PL | RH, S2s (finishing) | F2b; owner build 1 |
| 5 | ED2, RT (after PL) | — | ED1, S2s merged |
| 6 | INT | — | All merged; owner build 2; owner gates |
| Later | LV | — | The owner's layout pick |

### Merge hotspots and mitigations

| Hotspot | Who edits it | Mitigation |
| --- | --- | --- |
| `[workspace.dependencies]`, `Cargo.lock`, `[patch.crates-io]` | S0b, then the coordinator | FK's `[patch]` is one coordinator commit; pre-declare generously (P1a lanes paid workarounds) |
| `enth-engine`, `enth-render` (P1a spine) | E+ only; RT after E+ is merged | Additive only; every golden unchanged is part of acceptance |
| `enth-session` files | S1s (types), SK (minimal impl files), S2s (the rest) | Files pre-declared per concern; SK and S2s own different files |
| Theme tokens, the token gallery snapshot | Per-area token files from S0b; UI-C owns the gallery | A lane adds tokens only in its own area file |
| `enth-ui` widgets | UI-C | ED lanes compose; widget signature changes are additive until F2b |
| Snapshot PNGs (binary, ~300 KB) | One per component or region, owned by its lane; whole-editor snapshots per layout owned by that layout's lane | Binary files can't be merged: a lane that moves another lane's snapshot updates it in the same commit after rebasing; the coordinator never regenerates PNGs in bulk |
| Spike crates (`spike-ui` snapshots, `spike-ui-plugin`, `spike-shell`) | Frozen | INT removes them from the workspace once xtask targets `enth-plugin` (F2b), so widget changes stop rippling into spike code |
| `xtask/src/main.rs`, CI workflows | S0b stubs every P1b command and job | Lanes fill their own modules |
| `tools/test-host` | TH only | A case file per plugin |
| Plugin goldens | None as such | They reference engine goldens; plugin-only cases are one file each |
| Composite goldens, bench budgets | The coordinator, once per wave | Lanes report deltas, never edit another lane's budget |
| Docs | `docs/session.md` (S1s, S2s), `docs/plugin.md` (SK, PL), `docs/editor.md` (ED1, ED2), the G1 rows (INT) | One owner per file |

### Owner gates

| Gate | What you do | Time | When | Default if you don't answer |
| --- | --- | --- | --- | --- |
| Fork decision | Carry the nice-plug and baseview patches, or keep stock (Phase 0 decision 1) | 5 min | Wave 1 | Stock: "Slot N" everywhere, the leak documented |
| Baseview report | Post, rewrite or drop the drafted xkb report (Phase 0 decision 5) | 10 min | Any time | Not posted |
| Owner build 1: real display | Insert `enth-plugin` (CLAP, then VST3) 10× each; idle CPU; resize; Space after clicking the editor; HiDPI at `Xft.dpi: 192`. This replaces checklist C1, C4, C5 (and C6 if FK) on the spike plugins | 40 min | After F2b | — (the G0 real-display rows stay pending) |
| Layout pick | Read the Claude Design variants; pick one, or "keep Lanes, fixed" | 20–30 min | When the variants arrive | Lanes, with the handoff's signal-flow fixes |
| Owner build 2: G1 | Make a bass, pad, pluck and lead you like in REAPER, and save them (an agent commits them to `content/presets/`) | 60–90 min, one or two sittings | After ED2 | — (G1 waits) |
| Real audio (C3) | 8 instances at a 64-sample quantum (your PipeWire config forces min-quantum 1024; 03's notes), editors closed then open | 30 min | Owner build 2 | — (the G0 xrun row stays pending) |
| Bitwig poly-mod (optional) | Start the 30-day trial (needs an account); per-voice modulation of Macro 1; save and reload | 45 min | After PL | Deferred to S1 (roadmap) |

Total: about 2.5–3.5 h, plus 45 min if you do Bitwig. P1a's pending owner items (reverb A/B 20–30 min, the agent-eval blind listen 15–20 min, the metric sessions 2 × 15 min) are separate.

### Where the layout variants plug in

- ED1 builds a **layout seam**: regions (header, macros, sources, voice effects, modulators, effects, overview, status) are components that draw into a `Rect`.
- A layout is a function from (window rect, patch shape, mode) to region rects, chosen by a token or enum, so the dev window and the plugin can switch layouts without a rebuild.
- Stable widget ids keyed by (module id, parameter address) keep drags, gestures and egui state alive across layouts (02's notes measured 59 of 59 ids stable with this scheme, against 0 of 59 for automatic ids).
- A variant then costs one file plus its two snapshots (LV) and touches no other lane's files. Layout work stays off G1's critical path unless you want the chosen layout before making your four sounds.

## Top risks

| # | Risk | Evidence | Impact | Cheapest retirement | Lane |
| --- | --- | --- | --- | --- | --- |
| 1 | **Two sources of truth for parameter values:** host automation reaches the engine on the audio thread, the session's `Patch` lives on the main thread, and nice-plug keeps its own parameter atomics. Plus REAPER's undo loading state mid-session | clap-validator's state tests randomise *through `process()`* ([source](https://github.com/free-audio/clap-validator/blob/master/src/tests/plugin_instance.rs)); nih-plug #275 and #279; Actuate #48; nih-plug #254; REAPER get/set state on every change (JUCE forum, *anecdotal*) | Saved projects lose automated values, the editor shows stale values, automation and editor edits fight, enth's undo history contradicts the host's | One owner (the session) with an explicit fold-in of host values at save and on each editor frame, written into the F1b threading contract; proved in F2b by clap-validator `state-reproducibility-*` plus a test-host "automate, save, load, render = same hash" | S1s, SK |
| 2 | **nice-plug threading and API pitfalls** | `execute_gui` inline in `process()` and `editor_closed` never called (shell.md, *measured*); `GuiContext::set_state` applies on the audio thread (01's notes); `id_or_fallback` ignores host ids (source); PR #15 (no plugin-initiated parameter changes); nih-plug #92 (mass parameter sets crash VST3 hosts) | Allocations or GUI work on the audio thread, broken poly-mod, host crashes on patch load | Shell rules in AGENTS.md (never schedule GUI work from `process()`; patches load through state only; read `VoiceID::id()`); clap-validator on the debug bundle; test-host `--process-on caller` with the editor open; poly-mod id unit tests | SK, PL |
| 3 | **Real-display failures agents can't see** | nice-plug #95 (blank first open, NVIDIA 595; fix reported in the versions enth pins, *anecdotal*); the Xorg crash under GL window churn (AGENTS.md); baseview #359 (HiDPI); Space lost (ui.md); Bitwig black editors (OctaSine #145, nih-plug #271); baseview PR #357 (threading change pending) | The plugin looks done under Xvfb and fails on your screen; G0's reopen triggers (ADR-0002) | Owner build 1 right after F2b, with a 40-minute sheet that replaces Phase 0's C1, C4, C5 and C6; exact pins through P1b; the software-GL fallback documented for a black editor | SK, INT |
| 4 | **Editor churn:** slow feedback, binary snapshot conflicts, layout variants arriving mid-phase | Snapshots of ~300 KB that can't be merged (02's notes); automatic ids drop drags on a re-layout (02's notes); the owner asked for layout alternatives (the handoff) | Rework across ED lanes, unresolvable PNG conflicts, regressions hidden behind "updated snapshots" | The layout seam and stable ids in UI-C and ED1; one PNG per component; interaction tests by accessibility label; variants as separate LV lanes after the pick | UI-C, ED1 |
| 5 | **Merge hotspots, build contention, a slow gate** | P1a: composite goldens regenerated at three merges, budget drift between worktrees, a gate of 3 min 54 s warm and more than 595 s cold, a gate run moved to the background after 600 s; Phase 0: `Cargo.lock` conflicts; now 5.6 GiB available at load 17.5 (*measured*) | Lost hours, idle agents, false greens | S0b pre-declares everything; at most 2 heavy lanes; a bundle cache for the harness lanes; the two-tier, exit-status-only gate; composite artefacts belong to the coordinator | S0b, coordinator |
| 6 | **CLI ≠ plugin renders** | Velocity `v/127` vs f64; no macro events in `Performance`; a `NOTE_END` past the block end (01's notes); nice-plug #91 shows how fragile event timing is | "Determinism: CLI renders = plugin renders" fails silently; scores and lessons disagree with what you hear | Plugin goldens *are* engine goldens through the test host, in F2b, with off-grid events and host blocks of 1, 64, 100 and 4096; E+ adds macro events; velocity converted in one place | E+, SK, TH |
| 7 | **Leaks and real-time regressions in the shell** | baseview xkb leak, 76.9 KB per cycle (test-host.md, *measured*); nih-plug #222 drop leak; nice-plug #87 ("frame count doesn't bound event count"); a 1.6 MB reverb copy at a swap (brief; 03's notes measure 203–416 µs swap blocks); return-ring draining while the editor is closed is undefined | Memory growth in long sessions, xruns at structural edits, GS1's one-hour soak failing later | `editor-cycles` and instance-churn gates; session plan counters; 10,000-event bursts on the debug bundle; E+ moves reverb state at a swap; RT measures p99.9 | E+, TH, INT |
| 8 | **Owner bandwidth and G1's validity** | G1 needs "a bass, pad, pluck and lead you like" through the editor; about 2.5–3.5 h of P1b owner time on top of P1a's pending listening items; the Bitwig trial's 30 days and its account step | G1 waits on you; checks pile up | Two scheduled owner builds with one sheet each; defaults for every decision; Bitwig optional and timed after PL; announce owner build 2 early | INT |

Just outside the top 8:

- VST3 slot names stay "Slot N" in REAPER whatever we do: a known limit.
- nice-plug's bus factor of 1 and its monthly breaking releases (ADR-0001): a long-term cost; clack is the escape hatch.
- Keyboard focus: Space is lost after clicking the editor. That is baseview's policy (02's notes: X11 never forwards ignored keys), so it is a known limit to log, not a P1b fix.
- The CI cost of running `editor-cycles` under Xvfb: unmeasured.

## Plan check

| Doc | Current text | Proposed change | Why |
| --- | --- | --- | --- |
| docs/README.md | "the P1a lanes … are next. No product code yet." | P1a is done (headless G1 table in p1a-g1.md); P1b research under way | Stale since 17:52 |
| docs/phase-1a.md | "Status: … lane S0 is in progress, the rest has not started" | Done; record actual hours (about 18 agent-h, 7.3 h wall, against 52–93 estimated) and the unplanned CH lane | phase-1a.md promised actual-against-estimate |
| New docs/phase-1b.md | — | This lane plan: freezes F0b, F1b, F2b; heavy/light slots; owner builds | — |
| AGENTS.md, Done means | "plugin changes pass `cargo xtask validate` … once they exist" | `validate` exists from S0b; plugin changes pass `host-render --package enth-plugin`, which includes engine-golden equality, and `validate` on the debug bundle | `validate` does not exist yet (*measured*: no such xtask command) |
| AGENTS.md, working in parallel | "At most 2 heavy builds at once" | Add: pair each heavy lane with a light one; harness lanes use the coordinator's bundle cache; gates are judged by exit status only (no `cmd \| tail && echo ok`); long commands run in the foreground with an explicit timeout, or in the background with a wake-up | P1a's gate lessons |
| AGENTS.md, plugin rules | — | Never schedule GUI work from `process()`; never call `GuiContext::set_state`; patches load through state, never host automation; read `VoiceID::id()`, not `id_or_fallback` | shell.md; 01's notes; nih-plug #92 |
| architecture.md, threads table | The background worker and main thread | Name who drains the return ring when the editor is closed and no timer runs; the editor depends on a session trait, not on the plugin | Undefined today; culsynth's pattern |
| design.md §4 | Names are refreshed after slot assignment and state load | Add: the session folds host parameter values into the patch at save; a slot reassignment sets at most one host value; state is byte-deterministic (clap-validator `binary`) | Risk #1 |
| design.md §5 | "Plugin-level goldens go through the test host" | Plugin-level goldens reuse the engine goldens' hashes; only plugin-only behaviour gets its own cases | Kills plugin-golden churn |
| roadmap.md, G1 | P1b checks: p99 in the test host, "bass, pad, pluck and lead" | Add "CLI = plugin renders (engine goldens through the plugin)" as a G1 check; Bitwig poly-mod is optional in P1b and owned by S1 as the roadmap says | Makes determinism across shells a gate |
| benchmarks.md | "Knob → sound latency … moves to P1b (test host)" | Owned by lane TH | Unowned otherwise |
| owner-checklist.md | Sections B–C on the spike plugins | Superseded by the P1b owner sheet on `enth-plugin` | Do each check once, on the real product |
| xtask (`reaper.rs`, `host_render.rs`) | `const PACKAGE: &str = "spike-shell"` | `--package` (S0b) | Needed by every plugin lane |

## Gaps

- **No builds.** P1b sizes are estimates calibrated on P1a's actual times. P1a lane durations come from scratch-file times and commit times, not from a log of agent hours. Coordinator time is not counted.
- **nice-plug #95's fix** rests on one user's comment (2026-10-19); the issue is open. Whether enth's pins show a blank first open on the owner's NVIDIA display is decided only by owner build 1.
- **I did not verify Bitwig's behaviour** with nice-plug plugins myself; the evidence is issue text from other projects.
- **REAPER's get-state frequency** comes from a 2023 JUCE forum thread (*anecdotal*); the REAPER harness can count state calls to confirm it.
- **nih-plug #222's relation** to nice-plug PR #4 is *inference* from the PR title.
- **The `editor-cycles` CI idea** (Xvfb on a GitHub runner) is untested.
- **Sub-researchers 01–03's reports were not written yet.** Where I rely on their notes (engine gaps, `set_state` on the audio thread, the fork prototype, stable ids, swap timings, the PipeWire config), the summary should check this report against their final reports.
- **Not searched:** KVR threads specific to Rust plugins, and the Rust Audio Discord, which can't be searched.
