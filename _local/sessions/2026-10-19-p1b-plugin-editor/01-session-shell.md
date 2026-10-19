# 01 — Session and shell (P1b)

- Date: 2026-10-19
- Sub-researcher: 01 (session and shell)
- Brief: [brief.md](brief.md). Questions: 1 (`enth-session`, L6), 2 (`enth-plugin`, L8, on nice-plug), 3 (the nice-plug fork plan), 4 (determinism, CLI = plugin).
- Evidence labels: **measured** (I ran it on this laptop today), **documented** (source code, changelogs, official pages, all read today), **anecdotal** (issue reports, single users), **design** (my proposal, not yet run).
- Scratch: `~/code/enth-scratch/2026-10-19-p1b/01/` (running log `notes.md`). Two prototypes:
  - `fork-proto/`: nice-plug 0.4.2, nice-plug-core 0.4.2, nice-plug-egui 0.6.0 and baseview 0.4.1 vendored from the crates.io packages behind `[patch.crates-io]`, with three patches as separate git commits (`vendor/.git`), and a copy of `spike-shell` that logs editor lifecycle;
  - `det-proto/`: `enth-engine` 0.2.0 from the repo inside a small nice-plug CLAP shell (`enth-det-probe`), rendered by the repo's `enth-test-host` and compared bit for bit with `enth-render`.
  - Plus `vendor-lints/`: a ten-line workspace that checks how Cargo treats warnings in a vendored path dependency.
- Conditions: the machine was shared (load average 8–20); builds used `CARGO_BUILD_JOBS=3` under `flock -s /tmp/enth-quiet.lock`; every program that loads a plugin ran under `xvfb-run`. Nothing was written inside the repo except this report. The repo was indexed in codebase-memory as project `enth` (fast mode) for symbol lookups.
- Code read: crates.io packages in `~/.cargo/registry` (nice-plug 0.4.2 = Codeberg tag [v0.4.2](https://codeberg.org/RustAudio/nice-plug/src/tag/v0.4.2), nice-plug-egui 0.6.0, egui-baseview 0.9.0, baseview 0.4.1) and the repo at `main` (`1404239`).

## Answer in brief

1. **`enth-session` should be a plain, single-threaded state machine** (`Session`, `&mut self` API, no locks, threads, clocks or I/O) that owns the live `Patch`, the `History`, the engine's `Link` and the `PlanMap` of the latest plan it sent. The plugin wraps it in one `Mutex` shared by the host's main thread (state, activate) and baseview's X11 GUI thread (the editor), which in baseview 0.4.1 are different threads. The audio thread never touches it. *(design, from documented threading)*
2. **Delivery is a table over `ops::effect`:** `Value` → `Link::send_param(epoch, id, plain)` with the epoch of the latest *sent* plan; `Macro` → `send_macro`; `Structural`, `Depth` (the engine has no depth event today) and `SetSlot` → a recompile, compiled lazily at the moment it can be sent. `SendError::InFlight` means coalesce and retry on the next pump; a full ring means queue, then fall back to a recompile. Undo and redo use the same path. *(design; engine API documented)*
3. **Host automation must not use a `PlanMap` lookup in the shell.** `EventKind::Param` carries no epoch, and a plan swap happens at a segment boundary *inside* `Engine::process`, so an id taken from the old `PlanMap` can hit another parameter of the new plan. Add an engine-side `EventKind::Slot { index, norm }`, resolved through a slot table compiled into each plan. *(documented from code: `engine.rs:507` applies `Param` without any check, while `Cmd::Param` is epoch-checked at `engine.rs:424`)*
4. **Normalised ⇄ plain is not a round trip:** `from_norm(to_norm(v)) != v` for 33–82 % of values on Log, Power and signed parameters. So the patch wins at load, initial host values are never echoed back, and one rule decides everywhere: if the host's normalised value equals `to_norm(patch value)` bit for bit, the exact plain value is used; otherwise `from_norm(n)`. *(measured, 100,001 values per parameter)*
5. **CLI renders equal plugin renders, measured:** `enth-engine` in a nice-plug CLAP shell, driven by `enth-test-host`, matched `enth-render` bit for bit in **20 of 20** runs (44.1 and 48 kHz × host blocks 1, 37, 64, 256 and 4096 × processing on the caller or a separate thread), including a slot automation point and voice-end reports. Two requirements: `SAMPLE_ACCURATE_AUTOMATION = true` (with `false`, block 4096 differed from frame 20,480), and a fixed render seed. *(measured)*
6. **What still breaks it:**
   - a different sample rate;
   - host restarts (deactivate, activate, reset), which restart the grid and the note index;
   - live plan swaps;
   - the monitor bypass;
   - anything a `Performance` cannot express: macros, the mod wheel, poly-mod, pitch bend;
   - same-frame note-on/off order (only zero-length notes);
   - any shell code that writes `from_norm(to_norm(v))`.

   FTZ does not break it: nice-plug sets FTZ only with its `unsafe_flush_denormals` feature, which we leave off, and the engine is FTZ-independent. *(measured + documented)*
7. **State:** nice-plug's JSON `PluginState` holds the 72 params by permanent string id, plus two persisted fields:
   - `patch`: the exact `Patch::to_json()` text, carried as a `Box<RawValue>` so it is neither double-escaped nor re-parsed through `serde_json::Value`;
   - `enth`: schema `enth.plugin-state/1`, holding the monitor bypass and UI state.

   The host loads state on its main thread, possibly while audio runs, and calls our field's `PersistentField::set` there. nice-plug neither re-activates nor resets the plugin after a load. **Never call `GuiContext::set_state`:** while the plugin is processing, nice-plug applies that state on the audio thread. *(documented: `wrapper/clap/wrapper.rs:1495–1679, 2287, 3433`)*
8. **Notes and voices:**
   - Host notes become `NoteKey { port 0, channel, key, note_id = VoiceID::id() or −1 }`.
   - `NOTE_END` comes from `Engine::note_ends()`, with the offset clamped into the block: `NoteEnd.frame` is the segment *end*, up to 32 frames past the block, because the FIFO renders ahead.
   - Poly-mod needs a new per-voice macro-offset event in the engine. nice-plug forwards poly-mod only when the host gives a note id.
   - VST3 drops `NOTE_END`.
   - The mod wheel needs `MidiConfig::MidiCCs`, which adds 2,080 hidden VST3 params. Decide that before the first build leaves the machine. The engine has no pitch bend. *(documented)*
9. **Engine API gaps for P1b:** `Engine::reset()` (allocation-free; CLAP calls `reset` on the audio thread), `Engine::position()`, slot events and slot table, `Cmd::Event` for audition notes, `EventKind::MacroVoice` for poly-mod, optionally a depth command (so drag-to-modulate does not recompile every frame), and the 20 ms bypass fade that ADR-0006 specifies. The engine switches the bypass instantly. *(documented)*
10. **Fork: vendor it in the repo behind `[patch.crates-io]`, not as a git fork.**
    - Measured on the vendored tree:
      - the rename patch applies cleanly to the crates.io sources;
      - the vendored crates build with **zero warnings** on 1.99.1 (cold build 1 min 53 s, 544 MB peak);
      - `cargo deny check licenses sources` passes.
    - Caveat (measured): vendored path dependencies are *not* lint-capped, so CI's `CARGO_BUILD_WARNINGS=deny` fails on any warning a future toolchain finds in them.
    - Three patches: slot rename (13 files, +123/−4), `editor_closed` (+9), and the baseview xkb leak (+4/−1).
11. **`editor_closed` is not needed for correctness.**
    - `EguiEditorState::is_open()` is set on spawn and cleared when nice-plug drops the editor handle. `process()` saw 20 of 20 opens and 20 of 20 closes.
    - The 9-line fork fix made `editor_closed()` fire 20 of 20 times.

    *(measured)*
12. **Upstream today (2026-10-19):**
    - **nice-plug:** no release since 0.4.2 (2026-09-14); nice-plug-egui 0.6.0 (2026-10-18) is the latest; Codeberg `main` is unchanged since `7197104c0d` (2026-10-18).
    - **baseview:**
      - 0.4.1 is the latest release;
      - `master` (last commit 2026-10-09) **still lacks `xkb_keymap_unref`**, and no issue mentions the leak;
      - PR #357, which takes baseview's X11 work off its separate thread and so changes our GUI thread, is open and was updated today.
    - **Upstreaming:** RustAudio's AI policy rules out upstreaming agent-written code. The owner can file human-written issues.

    *(documented)*

## Findings

### F1. Threads in nice-plug 0.4.2 on Linux (documented)

| Thread | What nice-plug runs there | Source |
| --- | --- | --- |
| Host main thread | `Plugin::default`, `params()`, `editor()` (once), `activate`, `deactivate`, CLAP and VST3 state save and load (`ext_state_save`/`ext_state_load`, VST3 `setState`), param info and value-text queries, `on_main_thread` tasks, GUI create and destroy | `wrapper/clap/wrapper.rs` 2370, 2864, 3389, 3433; `wrapper/vst3/wrapper.rs` 549 |
| Audio thread | `process`, `reset` (CLAP: audio thread while active), `params.flush` while active; state from `GuiContext::set_state` while processing (`wrapper.rs:2287`, VST3 `1639`) | `plugin.rs` docs; CLAP `params.h` `flush` is "[active ? audio-thread : main-thread]" ([params.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h), `main` at `a47f6badb`, 2026-07-27, latest tag 1.2.10) |
| baseview GUI thread (X11) | `NiceEguiApp::build`/`ui`/`resized`, `ParamSetter` calls | baseview 0.4.1 runs X11 on its own thread; [PR #357](https://github.com/RustAudio/baseview/pull/357) "X11: Do not use a separate thread anymore" is open (updated 2026-10-19 03:53) |

- **`schedule_gui` runs the task inline when the caller is the "main" thread** (`wrapper.rs:358–380`). Steinberg's validator calls `process()` on that thread, so a task scheduled from `process()` runs inside it (Phase 0 measurement, unchanged). nice-plug itself schedules `ParameterValueChanged` from `process()` while an editor is open. In nice-plug-egui that only calls `repaint_notifier.request_repaint()` (`editor.rs`, `param_value_changed`), which is real-time safe.
- **State load is not followed by `activate` or `reset`.** `set_state_inner` deserialises the params, calls `deserialize_fields` (our `PersistentField::set`), and schedules `Task::StateChanged`, which does `editor.state_changed()` and a host `rescan(VALUES)` (`wrapper.rs:1645–1679`, `407–430`). Its comment says "Reinitialize the plugin…", but no call does it. The plugin learns about a load only through `set()`.
- **`GuiContext::set_state` while processing** sends the state to the audio thread, which applies it at the end of `process()` under `permit_alloc` (`wrapper.rs:1495–1530`, `2287–2300`). Our `set()` would then run on the audio thread, lock the session and compile.
- **Order inside one load:** params first, then fields in `BTreeMap` order (`wrapper/state.rs:176–245`). A field named `enth` is set before `patch`.
- **No host dirty-marking.** nice-plug 0.4.2 never calls CLAP `clap_host_state.mark_dirty` and has no VST3 `IComponentHandler2::setDirty` (grep: no hits). CLAP says a param change makes the state dirty implicitly ([state.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/state.h)). So **an editor edit that changes only the patch blob may not mark the host project dirty.** Whether REAPER notices anyway is unmeasured (see Gaps).
- **FTZ:** `ScopedFtz` is compiled in only with the `unsafe_flush_denormals` feature (`wrapper/util.rs:20–280`). nice-plug's default features are `editor`, `vst3` and `tracing-subscriber` (crate `Cargo.toml`), and we use `default-features = false`. design.md §5's "nice-plug sets FTZ around `process()` anyway" is therefore not true for our build. Hosts may still set FTZ, and the engine does not care (P1a tests).

### F2. The engine and patch APIs the session builds on (documented, repo `main`)

- `History` (`enth-patch/src/history.rs`):
  - pure data: `begin_gesture(&Patch)`, `end_gesture(&Patch)` (annuls a gesture that ends where it started), `record`/`edit(patch, ops, gesture)`, `undo`/`redo(&mut Patch)`, `mark_saved`/`is_dirty`, `clear`;
  - a cap of 1,000 steps;
  - host automation is meant to bypass it ("applies `SetParam` with `apply` directly and records nothing").
- `ops::effect(&Patch, &Op) -> Effect` (`ops.rs:211`):
  - `SetParam` → `Value(addr)`, unless the parameter is structural;
  - `SetDepth` → `Depth(route)`; `SetMacro` → `Macro(i)`;
  - `SetSlot`, `SetLabel`, `SetMeta`, `SetMacroName` → `NoAudio`;
  - module, route, enable and amp-envelope ops → `Structural`.
  - **The engine has no depth event:** `ParamId` is only `Voice(u16)` or `Global(u16)` (`params.rs:36`), and depth-0 routes are left out at compile time (`plan.rs:852`). A depth change is a recompile today.
- Slots can only target automatable, non-structural parameters (`ParamSpec` const check; `ops::check_automatable`), so host automation never needs a recompile.
- `Link` (`swap.rs`):
  - `send_plan(Plan) -> Result<epoch, SendError<Box<Plan>>>`, `send_param(epoch, id, v)`, `send_macro`, `send_mod_wheel`, `collect()`, `epoch()`, `in_flight()`, `free_slots()`;
  - `MAX_PLANS_IN_FLIGHT = 2`; the default capacity is 1,024 commands and 4 returns;
  - plans and values share one ring, so order is preserved, and a plan that waits behind a crossfade holds back the commands queued after it (`engine.rs:405–440`).
- `Engine`:
  - `new`/`with_link(plan, render_seed, cap)`, `process(left, right, &[Event])`, `set_monitor_bypass`, `note_ends()`, `epoch()`, `stats()`;
  - **no `reset()`** and **no position getter** (`position` is private);
  - `NoteEnd.frame` is `segment start + 32` (`voice.rs:449–461`), and `process` clears the list at its start (`engine.rs:181`).
- `EventKind` (non-exhaustive): `NoteOn`, `NoteOff`, `Param{id, value}`, `Macro{index, value}`, `ModWheel`, `Choke`. There is no pitch bend and no per-voice modulation.
- `Performance` (`enth-render/src/lib.rs:111`) has notes and `Automation { time_s, addr: ParamAddr, value }`. `ParamAddr` cannot name a macro (`addr.rs:232`, targets are module, voice or master), so **macro and mod-wheel performances cannot be replayed by the CLI.**
- `RenderSpec::default()` uses `DEFAULT_SEED = 0x0E17_5EED` (`enth-render/src/lib.rs:33`).
- Monitor bypass: a hard switch at a segment boundary (`engine.rs:302–308`, `785–787`). ADR-0006 says "fades over 20 ms" (`docs/adr/0006-effects-strategy.md:56`). They disagree.

### F3. The parameter round trip (measured)

`refrender roundtrip` in `det-proto` sweeps 100,001 evenly spaced plain values per parameter and checks `ParamSpec::from_norm(to_norm(v))` bit for bit:

| Parameter (scale) | Values that change | Worst relative error |
| --- | --- | --- |
| `filter1.cutoff` (Log 20 Hz–20 kHz) | 77,850 | 2.5e-7 |
| `lfo1.rate` (Log) | 82,412 | 3.0e-7 |
| `env1.attack`, `env1.release`, `voice.glide` (Power) | 69,539 each | 2.8e-7 |
| `osc2.fine` (linear −100…100 ct) | 46,552 | 1.9e-3 (near 0) |
| `reverb1.wet` (dB −60…6) | 35,648 | 2.0e-3 (near 0) |
| `master.level` (dB −36…12) | 33,336 | 6.6e-4 (near 0) |
| `filter1.resonance`, `osc2.level` (linear 0…100) | 0 | 0 |

Consequence: if any path writes a host value back into the patch or the engine without checking whether it actually changed, a saved and reloaded project or an untouched slot drifts by an ulp or more. Renders stop matching the CLI, and the exact and canonical hashes change. The engine's docs already avoid this round trip for settled values ("It never goes through `from_norm(to_norm(x))`", `docs/dsp/engine.md`). The shell and session need the same rule.

### F4. CLI = plugin, end to end (measured)

`enth-det-probe` (`det-proto/plugin/src/lib.rs`, about 150 lines) is the shell recommended below, reduced to its core:
- `activate` builds `Engine::new(compile(patch, sr), 0x0E17_5EED)`;
- `process` turns nice-plug note events into `EventKind` with their offsets;
- one host param, "Slot 1" (0–1), is mapped to patch slot 1 (`filter1.cutoff`) and sent as `EventKind::Param { PlanMap id, spec.from_norm(n) }`, only when it changes;
- voice ends are reported as `NoteEvent::VoiceTerminated`, with the offset clamped into the block.

The patch is the `engine-chord` golden patch (two saws, noise, SVF, comp, reverb, two LFOs) plus three routes (LFO → cutoff, velocity → level, macro 1 → resonance), polyphony 8. The performance: five notes at off-grid times, and slot 1 set to 0.2 at 0.5 s. `refrender` renders it with `enth-render`, with the automation point `Automation { 0.5 s, filter1.cutoff, from_norm(0.2) }`.

| Run | Result |
| --- | --- |
| 48 kHz and 44.1 kHz × host block 1, 37, 64, 256, 4096 × `--process-on thread` and `caller` (20 runs) | **0 differing samples in all 20** (compared over the CLI's frames; the host rounds up to whole blocks) |
| Positive control: host render without the slot point, against the CLI with it | differs from frame 24,000 (144,000 samples), so the automation path is live |
| Negative control: `SAMPLE_ACCURATE_AUTOMATION = false` | block 64 identical (24,000 is a multiple of 64); **block 4096 differs from frame 20,480**, the start of the block that contains the change |
| Negative control: 44.1 kHz host against the 48 kHz CLI | differs from frame 0 |
| Echoing the initial host value (`from_norm(to_norm(3 kHz))`) | identical for 3 kHz: luck, not a property (see F3) |

- `enth-test-host`'s order for events on one sample is reset, param, note-off, note-on (`tools/test-host/src/notes.rs`). `enth-render`'s is automation first, then notes in performance order, each note's on before its off (`docs/dsp/engine.md`). Only a zero-length note differs, and the test host refuses those ("duration must be > 0", measured), so the case could not be run.
- Velocity: the test host and REAPER send CLAP `f64`, and nice-plug does `as f32` (`wrapper.rs:1301`); the CLI does `f32::from(v) / 127.0` (`enth-cli/src/perf.rs:366`). These agree, because rounding a correctly rounded f64 to f32 is innocuous for one division (53 ≥ 2·24 + 2). *(documented reasoning; the test used explicit f32 velocities)*

### F5. nice-plug-egui editor lifecycle (documented + measured)

- `NiceEguiApp::editor_closed()` has an empty default, and nothing calls it (`nice-plug-egui-0.6.0/src/lib.rs:94`; Phase 0 measured 0 calls in 22 closes).
- `EguiEditorState::is_open()` is an `AtomicBool`. `spawn` stores `true` and `impl Drop for EguiEditorHandle` stores `false` (`editor.rs`, last lines). CLAP `gui_destroy` drops the handle (`wrapper.rs:2864–2875`). **Measured** with the probe copy of `spike-shell` under `enth-test-host editor-cycles -n 20` (Xvfb): `process()` saw 20 rises and 20 falls of `is_open()`.
- The fix: `impl Drop for UserAppWrapper { self.egui_ctx.lock().take(); self.user_app.lock().editor_closed(); }`, 9 lines in `nice-plug-egui/src/editor.rs`. **Measured:** 20 `build()` calls and 20 `editor_closed()` calls, the n-th build seeing n−1 closes. The wrapper is the baseview window's app, so `Drop` runs on the GUI thread when the window closes.
- `EguiEditorState` has no setter for size or zoom and implements no `PersistentField` (0.6.0 source). Restoring the editor size from saved state needs a patch or a later version.
- Open and relevant: [nice-plug #95](https://codeberg.org/RustAudio/nice-plug/issues/95) (open; updated 2026-10-19): the egui UI is blank on first add in REAPER 7.73 with **NVIDIA 595.71.05**, the owner's driver series. *(anecdotal; report 02's area)*

### F6. Poly-mod, notes and VST3 (documented)

- CLAP `PARAM_MOD` with `note_id != −1` on a parameter that has a poly-mod id becomes `NoteEvent::PolyModulation { voice_id: note_id, poly_modulation_id, normalized_offset }`. Key and channel are dropped on purpose (`wrapper.rs:1240–1270`). Without a note id it becomes a mono modulation of the param value.
- CLAP `PARAM_VALUE` on such a parameter also emits `NoteEvent::MonoAutomation` (`wrapper.rs:1213–1238`). If we read param values directly, this event must be ignored, or the value is applied twice.
- NOTE_END:
  - nice-plug writes `CLAP_EVENT_NOTE_END` with `port_index: 0` and our `voice_id`, channel and key (`clap/context.rs:222–243`);
  - CLAP says NOTE_END carries "the port, channel, key and note_id … given by the host in the NOTE_ON event", and its example marks `time: ignored` ([events.h](https://github.com/free-audio/clap/blob/main/include/clap/events.h));
  - VST3 returns `Ok` and drops it (`vst3/context.rs:205`).
- The note port is one input, dialects CLAP and MIDI, CLAP preferred (`wrapper.rs:3111–3126`). `voice-info` reports the configured capacity (`wrapper.rs:3516`). The engine has `MAX_VOICES = 32` physical slots (`module.rs:40`).
- `MidiConfig::MidiCCs` (needed for CC1, the mod wheel, and for pitch bend) makes the VST3 wrapper register 130 × 16 = 2,080 extra parameter ids at the top of the id range (`vst3/util.rs:7–19`). The spike used `Basic`, so nobody has measured how REAPER lists them.
- CLAP `RESCAN_VALUES`: "The host will not record those changes as automation points". `RESCAN_INFO` covers name changes and "takes effect immediately". `RESCAN_ALL` is allowed only while deactivated (`params.h`, lines 310–335). nice-plug exposes `RESCAN_VALUES` only internally, after a state load. `clap_host.request_process` (wake a sleeping plugin, [host.h](https://github.com/free-audio/clap/blob/main/include/clap/host.h)) is never called by nice-plug (grep).

### F7. The fork, prototyped (measured + documented)

- **The rename patch** is `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/scripts/nice-plug-0.4.2-rename.patch` (a 342-line diff file; `git show --stat`: 13 files, +123/−4). It applies with `patch -p1` to a tree made of the **crates.io** packages laid out as `crates/{nice-plug,nice-plug-core,nice-plug-egui}`: measured, clean.
  - It adds `Param::display_name`/`set_display_name`, backed by a `std::sync::Mutex<Option<String>>` in every `ParamInfo`, and `request_param_info_rescan()` on `ProcessContext` and `GuiContext`. CLAP then calls `rescan(INFO|TEXT)` on the main thread; VST3 calls `restartComponent(kParamTitlesChanged)`.
  - Phase 0 measured it working in REAPER 7.82 for CLAP within 50 ms, and VST3 in REAPER rejecting it (`docs/spikes/shell.md`).
- **The xkb fix** is `~/code/enth-scratch/lane-h/patched-baseview/src/wrappers/xkbcommon.rs`: a call to `xkb_keymap_unref(keymap)` right after `xkb_x11_state_new_from_device` (+4/−1). Lane H measured it taking the editor's leak from 77 to 16 KB per cycle (lane H notes, 08:49).
- **The `editor_closed` fix:** +9 lines (F5).
- **The build:** `fork-proto` builds `spike-shell` against the four vendored crates. Cold debug build: 1 min 53 s in Cargo (2:38 wall), peak RSS 544 MB, load 10–20. **0 warnings** from the vendored crates on 1.99.1. `cargo deny check licenses sources` with the repo's `deny.toml`: "licenses ok, sources ok".
- **Vendored path dependencies are not lint-capped** (`vendor-lints`, a vendored `cfg-if` with two warnings):
  - `CARGO_BUILD_WARNINGS=deny cargo build` fails ("warnings are denied by `build.warnings` configuration", exit 101);
  - `cargo clippy --all-targets -- -D warnings` passes (exit 0; it shows the rustc warnings, but `-D` applies only to members);
  - `cargo fmt --check` ignores the vendored directory.
- **Licences:**
  - nice-plug, nice-plug-core and nice-plug-egui declare `ISC`, but their crates.io packages contain **no LICENSE file**. The text is in the Codeberg repo, read in Phase 0.
  - egui-baseview and baseview are `MIT OR Apache-2.0`, with both files present (read today).
- **Upstream (documented, 2026-10-19):**
  - crates.io latest versions: [nice-plug](https://crates.io/crates/nice-plug) 0.4.2 (2026-09-14), nice-plug-core 0.4.2, [nice-plug-egui](https://crates.io/crates/nice-plug-egui) 0.6.0 (2026-10-18), [egui-baseview](https://crates.io/crates/egui-baseview) 0.9.0 (2026-10-17), [baseview](https://crates.io/crates/baseview) 0.4.1 (2026-10-09), nice-plug-xtask 0.1.1.
  - Codeberg `main` is `7197104c0d` "update to baseview 0.4.1" (2026-10-18); nothing newer ([commits](https://codeberg.org/RustAudio/nice-plug/commits/branch/main)). Tags end at v0.4.2.
  - baseview `master` HEAD is `4c79df39b` (2026-10-09, #352). Its [`src/wrappers/xkbcommon.rs`](https://github.com/RustAudio/baseview/blob/master/src/wrappers/xkbcommon.rs) still says "keymap should be unref when XkbcommonState drop", but `Drop` unrefs only the state. The keymap code came with [#210](https://github.com/RustAudio/baseview/pull/210) (merged 2026-06-13). GitHub searches for "leak" and "xkb" in the repo find no report of it.
- **Policy:** [RustAudio's AI policy](https://rust.audio/community/ai/) (fetched 2026-10-19) applies to both nice-plug (README "AI policy") and baseview (README). It lists as unacceptable "AI generation of entire function blocks without further refinement", "'Vibe coding' entire pull requests" and AI-written PR descriptions and comments. It also bars "AI/'Vibe coded' projects" from the Discord showcase channels.

## Recommended design

### R1. Crates and files

| Crate (layer) | Files | Depends on |
| --- | --- | --- |
| `enth-session` (L6, new) | `src/lib.rs` (`Session`), `src/deliver.rs` (effects → `Link`), `src/host.rs` (`HostParam`, `HostSync`, the slot rules, slot names), `src/state.rs` (`SessionState`, `enth.plugin-state/1`), `src/audition.rs`, `tests/{deliver,history,host,state,bounce}.rs`, fixtures under `tests/fixtures/state/` | `enth-patch`, `enth-params`, `enth-engine`, `enth-render` (only for `DEFAULT_SEED`) |
| `enth-engine` (L2, extended) | `event.rs` (`Slot`, `MacroVoice`), `plan.rs` (slot table), `swap.rs` (`Cmd::Event`), `engine.rs` (`reset`, `position`, slot memory, bypass fade), `tests/{slots,reset,polymod}.rs` | unchanged |
| `enth-plugin` (L8, new; `cdylib` + `lib`) | `src/lib.rs` (the `Plugin`, `ClapPlugin` and `Vst3Plugin` impls, exports), `src/params.rs` (the 72 params and value text), `src/fields.rs` (the `PersistentField` bridges), `src/audio.rs` (the `process` translation), `src/editor.rs` (the `NiceEguiApp` around `enth-ui`), `src/shared.rs` | `enth-session`, `enth-ui`, `enth-engine`, `enth-patch`, `enth-params`, nice-plug, nice-plug-egui (vendored) |
| `vendor/` (not members; `[patch.crates-io]`) | `nice-plug/crates/{nice-plug,nice-plug-core,nice-plug-egui}`, `baseview/`, `README.md` (versions, `.crate` sha256, patch list, upstream status), `LICENSE` files | — |

`enth-session` stays pure: no locks, threads, clocks or I/O. It can then be unit-tested with a real `Engine` in the same thread, and reused by a standalone or wasm shell later. The plugin adds the `Mutex`.

### R2. Threading and shared state

```rust
// enth-plugin/src/shared.rs
pub struct Shared {
    pub session: parking_lot::Mutex<enth_session::Session>, // host main thread + GUI thread; never the audio thread
    pub monitor_bypass: AtomicBool,                          // written by the session or editor, read in process()
    pub names_dirty: AtomicBool,                             // set by the session; process() asks the host to rescan names
    pub offline: AtomicBool,                                 // BufferConfig::process_mode == Offline, set in activate
    pub transport_playing: AtomicBool,                       // written in process(), read by the session (audition policy)
    pub params: OnceLock<Weak<HostParams>>,                  // lets the main-thread bridges read host values (no Arc cycle)
    pub editor_state: Arc<EguiEditorState>,                  // is_open(): measured reliable, replaces editor_closed
}
```

- The audio thread owns `Engine` (inside the `Plugin` struct), the 72 `last_*` values, a preallocated `Vec<Event>` (capacity `INPUT_EVENT_CAPACITY + 72`), and the block start frame (or `Engine::position()`).
- Forbidden in `enth-plugin`, enforced by clippy `disallowed-methods` in the crate's `clippy.toml`, which must repeat the workspace bans:
  - `nice_plug::context::gui::GuiContext::set_state` (F1);
  - `AsyncExecutor::execute_gui` from `process`.
- Lock order: only the session `Mutex`. Nothing is ever locked from `process()`.

### R3. `enth-session` API (design)

```rust
pub struct Session { /* patch, history, port: Option<AudioPort>, gesture, pending, queue, names */ }
struct AudioPort { link: Link, map: PlanMap, epoch: u64, cfg: PrepareCfg }

pub const RENDER_SEED: u64 = enth_render::DEFAULT_SEED;   // never per instance, never the patch hash
pub const AUDITION_PORT: i16 = 0x7F00;                     // NoteKey.port for editor notes (−1 is the wildcard)

#[derive(Clone, Copy)] pub enum HostParam { Macro(MacroIndex), Slot(SlotIndex) }
pub trait HostSync {                                     // implemented by the shell
    fn norm(&self, p: HostParam) -> f32;                 // unmodulated normalised host value
    fn push(&mut self, p: HostParam, norm: f32, phase: Gesture); // ParamSetter begin/set/end (only while the editor is open)
}

impl Session {
    pub fn new(patch: Patch) -> Self;
    pub fn patch(&self) -> &Patch;
    // lifecycle
    pub fn attach(&mut self, cfg: PrepareCfg) -> Result<Engine, CompileError>; // compile + Engine::with_link(plan, RENDER_SEED, cap)
    pub fn detach(&mut self);
    pub fn pump(&mut self);                              // Link::collect, send a pending compile, drain queued values
    // edits (editor)
    pub fn begin_gesture(&mut self, host: &mut dyn HostSync) -> GestureId;
    pub fn end_gesture(&mut self, host: &mut dyn HostSync);
    pub fn edit(&mut self, ops: Vec<Op>, g: Option<GestureId>, host: &mut dyn HostSync) -> Result<Delivered, TxError>;
    pub fn undo(&mut self, host: &mut dyn HostSync) -> Result<bool, TxError>;
    pub fn redo(&mut self, host: &mut dyn HostSync) -> Result<bool, TxError>;
    pub fn is_dirty(&self) -> bool; pub fn mark_saved(&mut self);
    // host
    pub fn sync_from_host(&mut self, host: &dyn HostSync); // pull, without history: UI frame, before compile, before save
    pub fn host_norm(&self, p: HostParam) -> f32;          // to_norm(patch value): defaults and pushes
    pub fn slot_name(&self, s: SlotIndex) -> String;       // "Osc 1 · Cutoff" or "Slot 7": a product rule, so it lives here
    pub fn take_names_dirty(&mut self) -> bool;
    // state
    pub fn save(&mut self, host: &dyn HostSync) -> (String /* Patch::to_json() */, SessionState);
    pub fn load(&mut self, patch_json: &str, extra: Option<SessionState>) -> LoadReport; // clears history; recompiles if attached
    // audition (editor keyboard, Recreate)
    pub fn audition_on(&mut self, key: u8, velocity: f32) -> Option<NoteKey>;
    pub fn audition_off(&mut self, key: NoteKey);
    pub fn audition_all_off(&mut self);
}
```

**Invariants** (each needs a test):
- **I1. One writer.** Every patch change is an `Op` applied through the session. UI edits, undo and redo are recorded; host pulls, loads and migrations are not.
- **I2. Delivery follows `ops::effect`** (table R4), applied after the transaction succeeds. A failed transaction delivers nothing.
- **I3. Plans are compiled lazily from the current patch** at the moment `send_plan` can succeed. At most one compile is pending; a pending compile makes value events for the same edit unnecessary, so they are skipped. A plan always reflects the patch at the time it was sent.
- **I4. Value events carry the epoch of the latest *sent* plan, and its `PlanMap`.** An address missing from the map (a disabled module) sends nothing: the next structural edit's plan carries it.
- **I5. Full command ring:** queue at most N value events (keep the last one per `ParamId`); when the queue overflows, mark a recompile instead. Nothing panics, nothing blocks.
- **I6. The slot and macro rule** (shared with the engine, R6): for host value `n` and patch value `p`, the effective value is `p` if `n.to_bits() == to_norm(p).to_bits()`, else `from_norm(n)`. `sync_from_host` applies `SetParam(addr, from_norm(n))` / `SetMacro` **only** in the second case.
- **I7. Load:** the patch wins. Host values restored by nice-plug are never pulled during the load itself. History is cleared and the load counts as saved. Names are dirty. If attached, a recompile follows. A blob that does not load leaves an init patch and a `LoadReport` with the error's code and hint; it never panics.
- **I8. Save:** `sync_from_host` first, then `Patch::to_json()` (typed, dense) and `SessionState`. `mark_saved` only when the editor's preset save asks for it; a host project save does not touch the undo history.
- **I9. Undo and redo** deliver the applied ops through I2. A structural undo produces exactly one plan.
- **I10. Audition notes** use `AUDITION_PORT`, a fresh `note_id` per note, and `Link::send_event` (R6). They are refused while `offline`, and released on `audition_all_off`: the editor calls it when `is_open()` goes false, on deactivate and on load.

### R4. Delivery table

| `ops::effect` | Session action | Engine side |
| --- | --- | --- |
| `Value(addr)` | `map.get(addr)` → `send_param(epoch, id, plain)`; push to the host if a slot targets `addr` | base set at the next boundary, smoothed |
| `Macro(i)` | `send_macro(i, v)`; push to host `macro_{i+1}` | smoothed 10 ms |
| `Depth(route)` | recompile (P1b), or `Cmd::Depth` if R6 adds it | swap without a crossfade (no module added or removed) |
| `Structural` | recompile | swap with state carry; crossfade only if modules changed |
| `NoAudio` from `SetSlot` | recompile (the slot table lives in the plan, R6); names dirty | slot table updated |
| `NoAudio` from `SetLabel`/`SetMeta`/`SetMacroName` | names dirty | — |

A depth drag is a recompile per pointer move until `Cmd::Depth` exists, and every compile allocates the reverb's delay lines (about 1.6 MB). Report 03 owns the compile and swap cost. If those numbers are high, add `Cmd::Depth { epoch, route: u16, depth }` before the editor's drag-to-modulate lands.

### R5. The 72 host params

- **Ids** (permanent, `docs/naming.md`): `macro_1`…`macro_8`, `slot_1`…`slot_64` (`#[nested(array)]`), and the state fields `patch` and `enth`. The `enth` field name becomes permanent too. Product ids: CLAP `dev.enth.synth`; VST3 class id `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD` (reserved in `naming.md`).
- **Kind:** `FloatParam`, `FloatRange::Linear { 0, 1 }`. The value is normalised; plain values exist only in the patch and the engine.
  - Macros: `with_poly_modulation_id(i)`, default 0. A macro's host value *is* its patch value (identity).
  - Slots: no poly-mod; a fixed default of 0.5, because nice-plug fixes defaults at construction, so the host's "reset to default" means the midpoint.
- **Value text:** `with_value_to_string`/`with_string_to_value` closures over an `Arc<SlotTable>`: 64 `AtomicU32`s holding an index into a static list of (kind, param) pairs, or `u32::MAX` when the slot is unassigned. They format `spec.display(spec.from_norm(n))` ("1.20 kHz") and parse with `spec.parse`. They run on the host's main thread, where allocating is fine.
- **Names:** "Slot N" with stock nice-plug. With the rename patch: `Param::set_display_name(Some(session.slot_name(i)))` on the main thread, then `request_param_info_rescan()` from the audio thread when `names_dirty` is set. It is called rarely and is real-time safe: `schedule_gui` pushes to a lock-free queue, or runs inline on the validator's thread, as in F1. VST3 in REAPER keeps "Slot N" (Phase 0).
- **Editor edits of a slotted address:** `HostSync::push` sends begin, set and end with `to_norm(plain)` through `ParamSetter`, so the host records automation in write or touch mode. Undo and in-plugin preset loads push the same way. That writes automation in write mode. The clean alternative, setting values silently plus `RESCAN_VALUES` (which CLAP promises not to record, F6), needs a fourth patch. Defer it, and note it in Gaps.

### R6. Engine additions (P1b engine lane)

1. **`EventKind::Slot { index: u8, norm: f32 }`** and `Plan::slots: [Option<SlotTarget>; 64]`, where `SlotTarget { id: ParamId, spec: &'static ParamSpec, patch_plain: f32, patch_norm: f32 }`. `compile` builds it from `patch.slots`; a target in a disabled module gives `None`.
   - The engine keeps `host_slot: [Option<f32>; 64]`.
   - On a `Slot` event: store the value; if the target exists, `base = if norm == patch_norm { patch_plain } else { spec.from_norm(norm) }` (I6).
   - At a plan swap, after the state carry, re-apply every stored `host_slot` against the new plan's table, so host automation survives an edit, and a host value equal to the new patch value keeps the exact plain.
   - A `Cmd::Param` whose id is a slot target also updates that target's `patch_plain`/`patch_norm`. Then the host's echo of an editor edit (`to_norm(plain)`, computed by the same pinned function) resolves to the exact plain.
   - `new` clears `host_slot`; `reset` keeps it (CLAP: parameter values are unchanged by a reset).
2. **`Engine::reset()`**, allocation-free: free all voices, reset every module's state (as `start`/`reset_globals` do), empty the FIFO and the event queue, set the position, the segment counter and the note index to 0, and keep base values. A crossfade in progress ends by moving the old plan to the pending slot, never by freeing it.
   - Test: after `reset`, the output equals `Engine::new(plan with the same bases)`.
3. **`Engine::position() -> u64`**: the absolute frame of the next host frame, for `NOTE_END` offsets.
4. **`Cmd::Event(EventKind)` and `Link::send_event`**, for audition notes, applied at step 1 of the next segment. Offline engines have no link, so renders cannot see audition notes, by construction (like the monitor bypass).
5. **`EventKind::MacroVoice { note_id: i32, index: u8, offset: f32 }`** for CLAP poly-mod:
   - per-voice macro offsets, zero at voice start;
   - added to the macro source for **voice-rate destinations only** (global destinations see the mono macro);
   - nice-plug's `MonoAutomation` is ignored.
   - Determinism: renders with poly-mod are outside the CLI contract until `Performance` can express it.
6. **The bypass fade**, as ADR-0006 specifies: 20 ms on engage and on release. Or ADR-0006 drops the fade. Today the switch is instant (F2).
7. *(If report 03 says compile cost is high)* `Cmd::Depth`. *(For leads, G1)* `EventKind::PitchBend`, and `Performance` events for macros, mod wheel and pitch bend (a versioned change to `enth-render`).

Every new ring or queue path gets an overflow test under `assert_no_alloc` (AGENTS.md).

### R7. `enth-plugin` behaviour

- `Plugin` consts:
  - `MIDI_INPUT = MidiConfig::MidiCCs`, decided now: the mod wheel and later pitch bend need it, and it changes the VST3 param list (F6);
  - `SAMPLE_ACCURATE_AUTOMATION = true` (**required**, F4);
  - `AUDIO_IO_LAYOUTS` stereo out only;
  - `CLAP_POLY_MODULATION_CONFIG = { max_voice_capacity: 32, supports_overlapping_voices: true }`;
  - `CLAP_FEATURES = [Instrument, Synthesizer, Stereo]`;
  - `VST3_SUBCATEGORIES = [Instrument, Synth]`.
- **`activate`** (main thread):
  1. `offline` = `process_mode == Offline`;
  2. lock the session, `sync_from_host`, `attach(PrepareCfg { sample_rate })`, store the `Engine` in `self` (the old one is dropped here, on the main thread);
  3. set every `last_*` to the current host value, so nothing is echoed (I7).
- **`deactivate`**: `session.detach()`, drop the engine.
- **`reset`** (audio thread): `engine.reset()`.
- **`process`** (audio thread; no locks, no allocation; `assert_process_allocs` on in debug):
  1. `engine.set_monitor_bypass(shared.monitor_bypass)`.
  2. For each macro: `v = modulated_normalized_value()`; if it changed, push `Event { 0, Macro { i, v } }`. For each slot: if it changed, push `Event { 0, Slot { i, v } }`. Use the modulated value, so the engine follows CLAP mono modulation while the patch keeps the unmodulated one.
  3. Notes:
     - `NoteOn`/`NoteOff`/`Choke` → `NoteKey { port: 0, channel, key, note_id: voice_id.id().unwrap_or(-1) }`;
     - `PolyModulation` → `MacroVoice`;
     - `MidiCC { cc: 1 }` → `ModWheel`;
     - `MonoAutomation` and everything else → ignored.

     Never call `VoiceID::id_or_fallback`: it ignores the host's id (Phase 0).
  4. `engine.process(left, right, &events)`.
  5. For each `note_ends()` entry not on `AUDITION_PORT`: `try_send_event(VoiceTerminated { timing: min(frame − block_start, n − 1), … })`.
  6. If `names_dirty.swap(false)`: `ctx.request_param_info_rescan()` (fork).
  7. Feed the editor's rings only while `editor_state.is_open()` (report 02's area).
  8. `transport_playing` = `ctx.transport().playing`; return `KeepAlive`.
- **Fields** (`fields.rs`):
  - `#[persist = "patch"] patch: PatchField(Arc<Shared>)` implements `PersistentField<'_, Box<RawValue>>` (serde_json's `raw_value` feature):
    - `set` locks the session and calls `load(raw.get(), None)`;
    - `map` locks, runs `sync_from_host` and `save`, and hands the `RawValue` of the JSON text to `f`.
  - `#[persist = "enth"] extra: ExtraField(Arc<Shared>)` implements `PersistentField<'_, SessionState>`; `set` also stores `monitor_bypass`.
- **Editor glue:**
  - `build()` stores the `GuiContext` (for `ParamSetter` and the rename rescan) and treats every `build` as a fresh open: it replaces any `egui::Context` it held, as the stock nice-plug-egui requires.
  - `ui()` calls `session.pump()` and `sync_from_host()` once per frame, then draws.
- **VST3:** the same code.
  - No `NOTE_END` (dropped by the wrapper), no poly-mod, "Slot N" in REAPER.
  - A param set from the GUI while processing reaches the plugin through `process()` (Phase 0).
  - The state is the same JSON.
  - Validation: Steinberg's validator in a nightly job (Phase 0: 537 of 537 on the spike).

### R8. State format

```json
{ "version": "0.1.0",
  "params": { "macro_1": {"f32": 0.5}, "…": {}, "slot_1": {"f32": 0.3416}, "…": {} },
  "fields": {
    "enth":  "{\"schema\":\"enth.plugin-state/1\",\"monitor\":{\"fx_bypass\":false},\"ui\":{\"view\":\"lanes\"}}",
    "patch": "{\"schema\":1,\"engine\":\"0.2.0\",\"voice\":{…},…}" } }
```

- `version` is nice-plug's `Plugin::VERSION`, available to `filter_state` for param-id migrations. The ids are permanent, so it should never be needed.
- `patch` is exactly `Patch::to_json()`. A loader uses `enth_patch::file::load` and its migrations, the same path as `enth render`, so a plugin state and a `.patch.json` are interchangeable.
- `enth` is versioned by its own `schema` string. `#[serde(default)]` on every field lets older states load; the loader rejects a newer schema with a hint. Fixture tests and an insta snapshot of `serialize_fields()` cover both fields (AGENTS.md: persisted types carry a version and fixtures).
- The monitor bypass is persisted (ADR-0006: "saved with the plugin's state but outside the `Patch`"), defaults to off, and the editor shows it while on.
- Not persisted in P1b: editor size and zoom (`EguiEditorState` has no setter, F5).

### R9. The fork

- **Layout:**
  - `vendor/nice-plug/crates/{nice-plug,nice-plug-core,nice-plug-egui}` and `vendor/baseview`, imported from the **crates.io `.crate` files** (the bytes `Cargo.lock` pinned, sha256 recorded in `vendor/README.md`), not from git;
  - workspace `exclude = ["vendor"]`;
  - a `[patch.crates-io]` entry for each of the four crates, keeping their versions, so the `=` pins still match.
- **History:** one "pristine import" commit, then one commit per patch:
  1. slot rename (from the Phase 0 scratch, +123/−4);
  2. `editor_closed` (+9);
  3. xkb keymap unref (+4/−1).

  `cargo xtask vendor-diff` prints pristine against the vendored tree; CI fails if the diff is not exactly the listed patches.
- **Licences:** copy nice-plug's ISC `LICENSE` from the Codeberg tag `v0.4.2` (the crates lack one), keep baseview's MIT and Apache files, and add entries to `THIRD_PARTY_NOTICES.md`. Our patch lines carry the upstream licence of each crate, so a human could still upstream them.
- **Warnings:** zero today (measured). A toolchain bump that surfaces a warning in vendored code fails CI (F7); fix it in a fourth commit as part of the bump.
- **Why not a git fork:**
  - It needs public Codeberg and GitHub forks under the owner's account, which would expose the project's name (the repo is private).
  - CI depends on Codeberg being up.
  - `deny.toml` needs `allow-git`.
  - Its one advantage is capped lints.
- **Updates:** nice-plug-egui 0.7 is expected with baseview #357 (ADR-0001 budgets about a day). Re-import pristine and rebase the patch commits; #357 rewrites the X11 event loop, so expect to re-derive the xkb fix if upstream hasn't fixed it.
- **Upstreaming:** under RustAudio's policy, our agent-written patches cannot be PR'd. Recommended:
  - the owner files **issues** in their own words (xkb leak with the valgrind trace from lane H; `editor_closed` never called; runtime param names; no `mark_dirty`);
  - the owner writes the one-line xkb PR personally, if they like.

  Upstream fixes then shrink the fork.
- **Candidate patches for later** (not in P1b unless a measurement says so):
  - host dirty-marking: CLAP `mark_dirty`, VST3 `setDirty`;
  - silent param set plus `RESCAN_VALUES`;
  - `EguiEditorState::set_size`.

### R10. The determinism contract for the plugin

A plugin render equals `enth render` when all of these hold:
- the same engine build (`ENGINE_VERSION`);
- `RENDER_SEED = enth_render::DEFAULT_SEED`, or the challenge's seed passed through the session, the same value the CLI gets;
- the same sample rate;
- events at the same frames counted from engine creation or `reset`;
- no plan swap, no monitor bypass, no audition notes during the render;
- only events a `Performance` can express (notes, and automation through slots).

The shell guarantees:
- `SAMPLE_ACCURATE_AUTOMATION = true`;
- no initial echo of host values;
- I6;
- `Engine::reset` equal to a fresh engine;
- the `patch` field round-tripping exactly (text, not `Value`).

These are outside the contract and are documented as such: host restarts mid-render (REAPER restarts CLAP plugins about 30 ms into an offline render; Phase 0), poly-mod, the mod wheel and macro automation until `Performance` grows them, and same-frame zero-length notes.

### R11. Tests

- **`enth-session`:**
  - **delivery property** (proptest, fixed seed): random ops, undo, redo and host pulls against a real `Engine::with_link`, pumping and rendering a segment between steps. After the pumps settle, every `PlanMap` address's engine base equals `patch.value(addr)` bit for bit (needs a test-only `Engine::base(id)`);
  - **coalescing:** two plans in flight; a third edit waits; `pump` sends exactly one plan compiled from the latest patch;
  - **ring overflow:** a full command ring gives a queue, then a recompile; no panic;
  - **slots:** a host value equal to `to_norm(p)` keeps `p` exactly (all catalogue parameters, many values); the echo of an editor edit leaves the base exact; `SetSlot` reassignment;
  - **state:** `save` then `load` gives `canon::identical` and the same `exact_hash`; fixtures for `enth.plugin-state/1`; a corrupt blob gives an init patch and a `LoadReport` with code and hint; old patch schemas migrate;
  - **bounce:** a session-attached engine played with random block sizes equals `enth_render::render` of the same patch and performance (the in-process form of F4).
- **`enth-engine`:** `Slot` resolution and the re-apply across swaps under `assert_no_alloc`; reset equals a fresh engine; `MacroVoice` touches only matching voices and voice-rate destinations; a full command ring with `Cmd::Event`; the bypass fade.
- **`enth-plugin`:**
  - unit tests: 72 ids, names, flags and poly-mod ids (as `spike-shell`); `serialize_fields` snapshot; `NOTE_END` offset clamp; audition `NOTE_END` filtered;
  - `cargo xtask validate` (clap-validator 0.4.1) and the nightly VST3 validator;
  - `cargo xtask host-render` with the **real** plugin, CLI against plugin, F4's matrix as goldens;
  - `reaper-test` extended: patch blob round trip, CLAP slot rename visible, and `IsProjectDirty` after an editor-only edit (decides the `mark_dirty` patch);
  - `enth-test-host editor-cycles` with the vendored baseview: about 16 KB per cycle against 77 (lane H).

## Plan check

What the evidence says should change in the current docs:

- **`docs/dsp/engine.md`:**
  - "Host automation (`EventKind::Param` through `process`) applies to the running plan: use the `PlanMap` of `Engine::epoch()`" is unsafe when a swap happens mid-block. Replace it with slot events resolved in the engine (R6.1).
  - Add: `NoteEnd.frame` can lie up to 32 frames past the current block; the engine lacks `reset`/`position`.
  - The monitor bypass has no fade, unlike ADR-0006's "fades over 20 ms". Fix the engine or the ADR.
- **`docs/design.md`:**
  - §3: a value change through a route depth (`Effect::Depth`) is a recompile today; either say so or add `Cmd::Depth`.
  - §4: the slot table belongs in the plan; `SetSlot` recompiles; slot defaults are fixed at construction; the I6 equality rule; RESCAN_VALUES vs gestures.
  - §5: "nice-plug sets FTZ around `process()` anyway" holds only with nice-plug's `unsafe_flush_denormals` feature, which we leave off. Add the plugin's determinism conditions (R10), including `SAMPLE_ACCURATE_AUTOMATION = true`.
- **`docs/architecture.md`:**
  - the threads table needs a fourth row: baseview's X11 GUI thread is separate from the host main thread in baseview 0.4.1 (until #357);
  - `enth-session` is a pure state machine; the shell adds the `Mutex`.
  - The L6 row's "bridges host params" should name the slot rule and the `HostSync` trait.
- **`AGENTS.md`:**
  - "Host automation applies `SetParam` without a history entry" needs the clause "and only when the host value differs from `to_norm` of the patch value";
  - add "never call `GuiContext::set_state`" to the nice-plug pitfalls;
  - the baseview line ("don't carry a patched baseview until we decide how we depend on it") is ready for the decision in R9, recorded in ADR-0002;
  - "`NiceEguiApp::editor_closed()` is never called; detect a reopen in `build()`" should add "or read `EguiEditorState::is_open()`".
- **ADR-0001:**
  - record the fork form (vendored `[patch.crates-io]`, three patches, the pristine-diff check);
  - the rename patch applies to the crates.io sources unchanged;
  - nice-plug has no host dirty-marking;
  - `MidiConfig::MidiCCs` and its 2,080 hidden VST3 params need a decision before the first external build.
- **ADR-0006:** keep "persisted outside the `Patch`" (R8); reconcile the 20 ms fade with the engine.
- **`docs/naming.md`:** add the state field `enth` as permanent; the product ids move from "reserved" to "used by `crates/enth-plugin`" in P1b; directory names are now `crates/enth-plugin` (the doc still says `crates/plugin`).
- **`enth-render`:** `Performance` cannot express macros, the mod wheel or pitch bend. Add them, versioned, before Recreate or lessons need plugin-recorded performances.
- **The brief's "123-line patch":** confirmed, as 13 files +123/−4 by `git --stat`. It lives in the Phase 0 scratch, not the lane-A scratch.
- **Unchanged:** nice-plug stays (nothing in the evidence argues for clack); the 72-param scheme and ids; CLAP first, VST3 second; plugin goldens through the test host only.

## Gaps

| Gap | Why it is open | What settles it |
| --- | --- | --- |
| `enth-session` exists only as a design; the slot rules and coalescing are untested in code | Out of scope for a research lane (no code in the repo) | P1b session lane: R11 tests first |
| Does REAPER (or Bitwig) mark the project dirty after an editor-only patch edit, given nice-plug never calls `mark_dirty`? | Needs a host run with an editor edit | `reaper-test` step: change the patch through the GUI context, read `IsProjectDirty`; if 0, add the dirty-marking patch |
| How REAPER lists the 2,080 hidden VST3 MIDI params with `MidiConfig::MidiCCs`, and whether CC1 reaches the plugin in both formats | Not built | Build `spike-shell` with `MidiCCs`, run `reaper-test`, count params |
| Poly-mod and `NOTE_END` in a real CLAP host | No Bitwig here; REAPER has no poly-mod UI | Owner checklist (Bitwig trial) once `MacroVoice` exists |
| Cost of a recompile per drag step (depth edits) and of plan swaps with the reverb | Report 03's measurements | Report 03; then decide on `Cmd::Depth` |
| Audition while the host has put the plugin to sleep | nice-plug never calls `clap_host.request_process` (F6); hosts differ in whether stopped instruments are processed | Measure in REAPER (transport stopped, editor note); if needed, a small patch exposing `request_process` |
| Engine `reset()` semantics against hosts that call `reset` on stop or seek | No implementation yet | Engine lane test plus a `reaper-test` play–stop–play check |
| Silent host-param updates (no automation recording on undo or preset load) | nice-plug exposes `RESCAN_VALUES` only after a state load | Decide whether recording automation on undo is acceptable; else a fourth patch |
| Editor size persistence | `EguiEditorState` has no setter (0.6.0) | Accept for P1b, or a small patch; revisit with nice-plug-egui 0.7 |
| baseview #357 changes the GUI thread to the host main thread | Open PR (updated 2026-10-19) | Re-check at each nice-plug-egui release; the session `Mutex` stays correct either way |
| Leak numbers from the 20-cycle debug run (`fork-proto/cycles.json`: about 194 KB per cycle after warm-up) | Not comparable: debug build, 20 cycles, load ≈ 20; lane H's 200-cycle release runs are the reference | Re-run `editor-cycles -n 200` in release with the vendored baseview in P1b |
| Velocity equivalence for MIDI 1 hosts beyond the test host | Reasoned, not swept | A host-render case with velocities 1…127 |
| Same-frame zero-length notes (CLI on-before-off vs host order) | The test host refuses zero-length notes | Document as outside the contract, or define one order in `Performance` |
