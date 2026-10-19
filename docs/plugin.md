# The plugin (`enth-plugin`, L8)

**`enth-plugin` is the CLAP and VST3 shell around [`enth-session`](session.md), `enth-engine` and `enth-ui`, on nice-plug `=0.4.2` and nice-plug-egui `=0.6.0`.** It translates between the host and the product: 72 host parameters, notes, audio blocks, the saved state and the editor window. It holds no product rules: what an edit does, when a host value enters the patch and how a plan reaches the engine are the session's ([session.md](session.md)). Lane SK built the walking skeleton (freeze point F2b, [phase-1b.md](phase-1b.md)); lane PL completes the plugin, ED1 and ED2 the editor, RH the REAPER scenarios. Decisions behind it: [ADR-0001](adr/0001-plugin-shell.md), research report 01 in `_local/sessions/2026-10-19-p1b-plugin-editor/`.

## Files

| File | Holds | Lane |
| --- | --- | --- |
| `src/lib.rs` | the `Plugin`, `ClapPlugin` and `Vst3Plugin` impls, the export macros (in a module of their own), [`Shared`](#threads) and the background `Task` | SK |
| `src/params.rs` | the 72 parameters and the two persistent fields, host value reads | SK |
| `src/process.rs` | the audio block, the event list, the pump flag | SK, PL |
| `src/notes.rs` | host notes as engine events, `NOTE_END`, poly-mod | SK, PL |
| `src/state.rs` | the persistent fields `patch` and `enth` | SK |
| `src/editor.rs` | the `NiceEguiApp` and `Glue`, which binds UI-C's cards to `SessionView`/`SessionEdit` | SK, ED1 |
| `clippy.toml` | the workspace's bans, minus I/O, plus `GuiContext::set_state`/`get_state`, `execute_gui` and `VoiceID::id_or_fallback` | SK |
| `tests/cli-equal.profile.toml` | the engine goldens `cli-equal` renders through the plugin | SK |

The plugin-level golden is `tools/test-host/golden/enth-plugin.toml` with its patch `enth-plugin.patch.json`; the REAPER scenarios' patches are the `xtask/reaper/enth-*.patch.json` files ([REAPER scenarios](#reaper-scenarios)).

## Identity

- CLAP id `dev.enth.synth`, VST3 class id `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD`, name "enth", vendor "enth" ([naming.md](naming.md); a test pins both ids). REAPER lists it as `CLAPi: enth (enth)` and `VST3i: enth (enth)`.
- 72 parameters in host order: `macro_1`…`macro_8`, then `slot_1`…`slot_64` (`#[nested(array)]`, the order of `enth_session::bridge::HOST_PARAM_IDS`). nice-plug hashes them to the ids hosts store: `macro_1` 825615486, `slot_1` 1248030256, `slot_64` 34232479, the same as the shell spike's (tests pin them, and `reaper-test` checks REAPER's idents).
- Each is a linear 0–1 `FloatParam` with no smoothing (the engine smooths). Macros default to 0 and take CLAP poly-mod (id = macro index); slots default to 0.5. Names are "Macro N" and "Slot N", fixed at construction. Lane FK's vendored fork (on main since `7447f2f`) can rename them at run time; PL sets `host_param_name` and asks for an info rescan when [`Shared::names_dirty`](#threads) is set.
- Stereo out, no audio in. `MIDI_INPUT = Basic`: `MidiCCs` (the mod wheel) adds 2,080 hidden VST3 parameters, an open decision for PL. `SAMPLE_ACCURATE_AUTOMATION = true` (required, [Determinism](#determinism)). `CLAP_POLY_MODULATION_CONFIG` is 32 voices, overlapping.

## Threads

| Thread | Runs | Touches |
| --- | --- | --- |
| Audio (`process`, `reset`) | [the block](#the-audio-block) | the `Engine`, the `Audio` state, the atomics in `Shared`, the parameters' atomics. **Never the session lock** |
| Host main thread | `activate`, `deactivate`, state save and load (the persistent fields) | the session, through `Shared::with_session` |
| baseview's GUI thread | the editor's `build` and `ui` | the session, through `Shared::with_session`; `ParamSetter` after unlocking |
| nice-plug's background worker | `Task::Pump` | the session (pump, audition release) |

`Shared` holds the one lock, `parking_lot::Mutex<Session>`, and atomics: `pump_wanted`, `monitor_bypass`, `rebaseline`, `names_dirty`, `editor_closed`, plus a weak reference to the parameters for reading the host's values off the audio thread. **`Shared::with_session` is the only way in:** it locks, runs the closure, drains the session's requests, notes whether a compile is still pending, **unlocks**, then stores the monitor bypass, sets `names_dirty` on a rename and arms `pump_wanted`, and returns the host writes. Only editor edits make host writes, and the editor forwards them to `ParamSetter` after this returns, never under the lock, because a host may call back synchronously (to read the state, which locks).

## Lifecycle

- **Construction.** A `Session` on `Patch::init()`, the parameters, the editor state (sized by the theme's `size.window.default`), and an `Audio` whose event list is allocated here, never in `process()`.
- **`activate`** (main thread). Fold the host's values into the patch (`Session::fold_host`), then `Session::activate` (compile, `Engine::with_link` with `RENDER_SEED`, a disabled telemetry tap, the persisted monitor bypass on the fresh engine). The engine replaces the previous one, which drops here on the main thread. `Audio::activate` takes the host's current values as "last sent", so nothing is echoed and the engine starts from the patch, and clears `rebaseline` (this is the re-baseline a load before activation asked for). A sample rate outside the engine's 8–384 kHz activates **silent**, with the session detached, rather than failing: clap-validator sweeps 1 kHz to 768 kHz and a host that refuses a plugin is worse than silence.
- **`reset`** (audio thread). `Engine::reset()`. The engine frees its voices without reporting them, so the shell first copies their note keys and the next block sends their `NOTE_END`s.
- **`deactivate`** (main thread). `Session::deactivate`, then the engine drops.

## The audio block

`Enth::process_block`, in order; nothing allocates, locks, logs or does I/O (nice-plug's `assert_process_allocs` aborts a debug build that does, and clap-validator runs the debug bundle):

1. **Host values.** Read the 72 parameters' modulated normalised values and send each one that changed since the last block as `EventKind::Macro { index, value }` or `EventKind::Slot { slot, value }` at offset 0. Never a bare `EventKind::Param`: a plan swap inside the block would misroute an id looked up before it. The modulated value lets the engine follow CLAP mono modulation while the patch keeps the unmodulated one. With sample-accurate automation nice-plug splits the host block at each parameter change, so offset 0 is the change's own frame. After a state load (`rebaseline`) the values are adopted without being sent.
2. **Notes** ([`notes`](#notes)) into the event list; events past its capacity (nice-plug's input capacity plus 72) are dropped.
3. **Render**, after handing the engine the monitor bypass.
4. **`NOTE_END`**: the voices a reset freed (at offset 0), then the block's own ends at `NoteEnd::offset` (clamped into the block by the engine). Audition notes are not the host's and are skipped.
5. **Telemetry**: the tap's wake flag becomes a repaint request (lock-free).
6. **Editor closed**: `EguiEditorState::is_open()` falling disables the tap and sets `editor_closed` and `pump_wanted`; the next background pump releases audition notes. (The vendored nice-plug-egui also calls the editor's `editor_closed`, which does the session side on the GUI thread; both are idempotent.)
7. **The pump flag** (below).

### Notes

A host note is `NoteKey { port: 0, channel, key, note_id: VoiceID::id() or −1 }`. In a note-off or choke a wildcard field is −1, the engine's wildcard; a note-on needs a key (a wildcard key is dropped) and takes channel 0 for a wildcard channel. Host notes are on port 0, so a host note-off never releases an editor audition note (`AUDITION_PORT`). `PolyModulation` becomes `EventKind::MacroVoice { key: NoteKey::note_id(voice_id), .. }` for macros 0–7; `MonoAutomation` is ignored (step 1 already reads the modulated value). `NOTE_END` reports the note's own tuple (`VoiceTerminated`; nice-plug writes port 0).

## The pump

A compile that the session could not send (two plans in flight: the host had stopped calling `process()`) waits for the session's next entry. With the editor closed there may be none, so ([session.md](session.md), "Plan delivery"; F1b decision 1):

- whoever unlocks the session with `compile_pending` set arms `pump_wanted` (`Shared::with_session` does it for every caller);
- `process()`, while the flag is set and at most once per `PUMP_INTERVAL_S` (50 ms) of audio, clears it and calls `execute_background(Task::Pump)`: one bounded `try_send` to nice-plug's worker, only while a compile is pending, never in steady state;
- the task locks the session, releases audition notes if the editor closed, pumps, and re-arms the flag if a compile is still pending.

Never `execute_gui` from `process()`: it can run the task inline (CLAP `request_callback` while the host calls `process()` on the main thread). `clippy.toml` bans it in the crate.

## State

nice-plug saves `{"version", "params": {id: {"f32": v}}, "fields": {...}}`; the CLAP stream is a `u64` length and that JSON. Two fields, both permanent names:

| Field | Text | On load (host main thread) | On save |
| --- | --- | --- | --- |
| `enth` | `InstanceState::to_json()`, `enth.plugin-state/1` | `Session::load_instance`; the monitor bypass reaches `process()` through `Shared` | `Session::save_instance` |
| `patch` | exactly `Patch::to_json()`, carried as a `serde_json::value::RawValue` (the `raw_value` feature; no extra crate), so it is neither escaped twice nor re-parsed | set `rebaseline`, `Session::load_patch` (the patch wins; its plan goes out at once and forgets the engine's remembered host slot values), then `fold_host` with the restored values, which only records them | `Session::save_patch` with the host's values: folded first (rule 2 and 3 of the bridge), then the patch text, or the kept text of a patch that did not load |

nice-plug sets the parameters first, then `enth`, then `patch`, and neither re-activates nor resets after a load, so the engine is kept and the session sends the loaded patch's plan itself. A `.patch.json` file and the `patch` field are interchangeable: the test host's `--state some.patch.json` puts the file's text into the field, and the plugin saves it back byte for byte (`render --save-state F --check-roundtrip` reports `identical_text: true` for `enth-plugin.patch.json`).

**Never `GuiContext::set_state`** (nice-plug applies it on the audio thread while processing, where the field would lock and compile) **or `GuiContext::get_state`** (it serialises the fields, which lock: a deadlock inside an editor frame). An editor preset load calls `Session::load_patch` under the lock on the GUI thread; the host's values do not move, and the next frame's fold records them as the baseline (stale ones never enter the new patch).

## The editor

F2b's editor is one real card: UI-C's `Card` for the first sound source (`osc1` in the init patch), as a strip, with "More" for the advanced tier. `Glue` turns the frame's intents into session calls through `SessionEdit` only, so the same code runs in a test with a plain `Session`: `Begin` opens a gesture, `Set`/`SetMacro`/`SetDepth` edit inside it, `End` closes it, a release of the pointer anywhere closes it too, and clicks, toggles and enable switches are single edits. `ShowMore` is editor view state, kept in `Glue`.

Each frame (GUI thread): read the host's values, then under the lock `pump`, `fold_host`, draw from `SessionView::patch` inside a `CentralPanel`, apply the intents; unlock; forward the host writes (`begin_set_parameter`, `set_parameter_normalized`, `end_set_parameter`). Every `build()` applies the theme and ends any gesture the last window left open.

ED1 replaces the body with the full editor and its layout seam; the frame protocol stays. ED2 adds a `TelemetryReader` (create it in `build()` from `telemetry_source()`, `set_enabled(true)`; `process()` already disables the tap when the window closes and turns the tap's wake flag into repaints).

## Determinism

A plugin render equals `enth render` when the sample rate matches, the engine was not re-activated or reset during the render, no plan swap, monitor bypass or audition note happened, and every event is one a `Performance` can express (notes, and slot automation). The shell keeps its side: `SAMPLE_ACCURATE_AUTOMATION = true`, `RENDER_SEED` (the CLI's default), no echo of host values at activate, slot events resolved by the engine with the patch-wins equality rule, `reset` equal to a fresh engine, and the `patch` field carried as text.

Measured at F2b (`enth-test-host cli-equal`, debug and release bundles): **62 of 62** engine golden cases equal through the plugin at host blocks 1, 64, 100 and 4096; 10 are skipped by the test host (render seeds other than the default, the stand-in sine, engine-address automation that needs a profile). `mod-controls` is left out by the profile: its mod-wheel moves need `MidiCCs`, and the test host does not send a case's `controls` yet.

## Tests and gates

| Check | Command | F2b result |
| --- | --- | --- |
| Unit tests | `cargo test -p enth-plugin` | 23: ids and hashed ids, names, poly-mod ids, the two fields, the state round trip (identical patch and slots, byte-identical second save), a patch that does not load kept and saved back, host values folded after a load but never the restored ones, note tuple and `NOTE_END` mapping, host value events, the editor's gestures, and a headless egui drag of the card's level knob through the session |
| Session | `cargo test -p enth-session` | 20 engine-side tests in `tests/link.rs` (activate, pump, delivery, coalescing, queues, audition, the SK duties, a session engine rendering like `enth_render`) |
| clap-validator 0.4.1 | `cargo xtask validate --package enth-plugin [--release]` | 36 passed, 0 failed, 8 skipped (debug and release) |
| CLI = plugin | `enth-test-host cli-equal target/bundled/enth-plugin.clap --profile crates/enth-plugin/tests/cli-equal.profile.toml --min-compared 60` | 62 equal, exit 0; `--inject flip-bit` fails |
| Plugin golden | `cargo xtask host-render --package enth-plugin [--release]` | 7 cases (fresh chord, state patch, slot and macro automation, voice steal, command-line parameters, reset), the same hashes for debug and release |
| REAPER | `cargo xtask reaper-test --package enth-plugin [--bundle-dir D]` (Xvfb) | 8 scenarios and the cross-format checks, 217 checks, CLAP and VST3, about 105 s, three consecutive runs green ([REAPER scenarios](#reaper-scenarios)) |

## REAPER scenarios

`cargo xtask reaper-test --package enth-plugin` (lane RH) runs native REAPER 7.82 headless, always inside `xvfb-run`, with Dummy Audio and a throwaway copy of the install, through the Lua scripts in `xtask/reaper/`. Each script appends `check` and `data` lines to a log; `xtask/src/reaper.rs` and its submodules (`checks.rs`, `audio.rs`, `chunk.rs`) read them and make the checks that need the saved state or the rendered audio. It never plays sound. Needs `xvfb-run`, `timeout` and `python3`; ffmpeg is not needed.

```sh
cargo xtask reaper-test --package enth-plugin                                # builds, then runs everything
cargo xtask reaper-test --package enth-plugin --bundle-dir DIR               # a prebuilt bundle (ENTH_BUNDLE_DIR works too)
cargo xtask reaper-test --package enth-plugin --format clap --scenario state --scenario patch-roundtrip
```

`--bundle-dir` runs `DIR/enth-plugin.clap` and `DIR/enth-plugin.vst3` without building; it is how a lane or INT reuses the coordinator's bundle cache. `--scenario` is repeatable and `state-create` and `state-reload` are one scenario (`state`). The spike shell keeps its older three scenarios (`--package spike-shell`: `state`, `automation`, `editor-smoke`, 47 checks).

| Scenario | What it proves | Checks (each format) | Median time of three runs (CLAP / VST3) |
| --- | --- | --- | --- |
| `state` | 72 parameters with stable ids and names; `enth-state.patch.json` loaded as state, macro 1 = 0.7 and slot 1 = 0.3 set and saved; after a reload in a new process the saved patch is the same bytes and holds the folded host values (macro 1 0.7, slot 1's cutoff at `from_norm(0.3)`, about 159 Hz) | 17 | 5.7 / 5.5 s |
| `patch-roundtrip` | a rich non-default patch (`enth-roundtrip.patch.json`, the plugin's own first save) is byte for byte the same after save, reload, save and a second reload; the instance field and the parameters survive too | 12 | 6.9 / 6.4 s |
| `slot-automation` | slot 1 ramped 0 to 1 renders brighter and rises steadily; slot 64 held at 1 then lowered to 0.25 renders quieter and leaves the brightness alone | 15 CLAP, 16 VST3 | 4.8 / 6.3 s |
| `macro-automation` | macro 1 ramped 0 to 1 renders more than 3 times louder and leaves the brightness; macro 8 ramped late renders more than twice as bright | 9 | 2.3 / 2.2 s |
| `undo-state-load` | REAPER's undo and redo across plugin state loads restore the patch and the parameters (`Undo_BeginBlock2`, `DoUndo2`, `DoRedo2`) | 11 | 2.7 / 2.6 s |
| `two-instances` | two instances on one track keep their own patch and values through a save, a reload and a deleted neighbour | 21 | 4.9 / 5.0 s |
| `editor-smoke` | the editor opens, draws (at least 20 distinct colours read back with `XGetImage`), stays open and closes, five cycles | 5 | 10.5 / 10.9 s |
| `dirty-flag` | reads `IsProjectDirty` after an editor-only patch edit, with a slot-bound control (below) | 16 | 14.1 / 15.6 s |
| `cross-format` | CLAP and VST3 agree: parameter ids, the saved patch bytes, the rendered slot and macro audio | 4, and 2 info rows (one run) | within the above |

Full suite: exit 0 in three consecutive runs, 102, 105 and 123 s at a load of 8 to 15 (the old suite: about 45 s for 53 checks), 217, 217 and 218 `ok` rows, no `FAIL`; the rows have the same names in every run. The 218th is the CLAP flat lane below: it applied the lane in one run of three and the row became an `ok` instead of an `info`.

**How a scenario loads a patch.** `inject_state(track, fx, var)` in `common.lua` writes a state chunk into REAPER's `clap_chunk` / `vst_chunk` named config parameter. The chunk is built by `write_state_chunk` (`xtask/src/reaper/chunk.rs`) from a fixture and handed to the script through an environment variable. Layouts measured on REAPER 7.82: CLAP is a `u64` length and then the JSON; VST3 is a `u32` length, a `u32` 1, the JSON and 8 zero bytes. The JSON is nice-plug's `{"version","params":{id:{"f32":v}},"fields":{"patch":"<text>","enth":"<text>"}}` and the `patch` field is exactly `Patch::to_json()`, so a saved state is compared with `Saved::patch_text` byte for byte. `dump_chunk` in the Lua helpers prints a chunk for debugging.

**Robustness rules** (each came from a flake):

- Poll, do not sleep: `wait_until(pred, timeout, step, fn)` waits for a condition. `show_editor` waits up to 15 s for the window to be viewable and to draw.
- Raise the X window before reading pixels. Under Xvfb there is no window manager, so REAPER's main window can cover the plugin window and `XGetImage` returns the covering window's pixels. `xinput.py stats` raises it first. `xinput.py` (find, client, raise, click, drag, key, stats) refuses a real display: `DISPLAY` must be `:50` or higher, as `xvfb-run -a` gives.
- Offline render is not sample-exact. REAPER's `-renderproject` deactivates, reactivates and resets a CLAP plugin about 30 ms in, and once dropped a note under load, so renders are 32 short notes repeated every 0.125 s and the analysis uses per-note medians over the even-pitch notes (roughness is bimodal by pitch) and allows at most two outlier notes. Audio checks are directional (brighter, louder, steadier), with margins measured on both formats.
- Logs are named `{format}-{scenario}-{phase}` and the report scope is `{format}/{scenario}`; two renders sharing one append-mode log once mixed their lines.

**The open check: does an editor-only edit mark the project dirty? No.** `dirty-flag` opens the editor, finds a knob with a click-and-drag search (first try at editor point (366, 64), then a grid; the `ENTH_EDIT_POINT` environment variable overrides it), edits the patch, and reads `IsProjectDirty` at once, about 3 s later and after the window closes: 0 each time on CLAP and VST3. The control edits a slot-bound knob, which moves a host parameter and reads 1. Closing the floating FX window marks the project dirty by itself, so the "after close" reading says nothing about the edit. nice-plug never calls CLAP `mark_dirty` or VST3 `setDirty`, so a sixth fork patch is needed; until it lands the scenario reports the 0 as an `info` row (`DIRTY_MARKING_EXPECTED = false` in `reaper.rs`). Flip it to `true` with the patch and a clean project after an edit fails the scenario.

**What the renders showed** (for PL):

- On CLAP a slot lane that never moves is usually ignored in the offline render (2 of 3 acceptance runs; the third applied it, so it is a race, not a rule): the plugin takes the host's values at activation as its baseline ("the patch wins at activation"), REAPER reactivates the plugin about 30 ms in with the lane already at its value, so the patch plays until the lane moves. VST3 applies the lane. The same cause makes the first note of a lane that starts at the default 0.0 differ on CLAP only. `slot-automation` measures the flat lane (slot 64 held at 0.25) and reports `info` while `CLAP_FLAT_LANE_IS_KNOWN_GAP` is `true`; set it to `false` once the plugin applies such a lane and the CLAP result becomes a check (run it three times first: the race may stay). The cross-format render check reports the first-note level as `info` for the same reason.
- After a state load the host's macro display may differ from the patch's macro value (design.md section 4 allows it); `undo-state-load` and `patch-roundtrip` compare the patch and the slots, not the macro display.

**Maintaining the fixtures.** `enth-roundtrip.patch.json` must be the plugin's own saved text (key order matters); regenerate it from a first save when the patch format or the engine version changes. `dirty.lua`'s first knob position belongs to the F2b editor; the grid search takes over by itself when the editor's layout changes, only slower. The dirty-flag control and the two automation patches (`enth-slot-automation.patch.json`, `enth-macro-automation.patch.json`) route a slot or a macro to osc1's level or the filter cutoff; keep them when the routing changes.

## Known pitfalls

- **Voice ends on reset.** `Engine::reset` reports no `NOTE_END`. The shell copies the active voices' keys in `reset` and reports them at offset 0 of the next block; a host that tracks voices would otherwise wait forever.
- **Re-baseline on load.** Values the host restores with a state must not override the loaded patch. Three pieces: the session's first fold after a load only records (so the field folds the restored values right after `load_patch`, or a host change before the next save would be taken as the baseline and lost; the REAPER run found exactly that), the loaded plan forgets the engine's remembered slot values, and `process()` adopts the restored values without sending them (`rebaseline`, set before the plan goes out; `activate` clears it, since it re-baselines anyway, so a change in the first block still counts).
- **The patch wins at activation too.** Host values set before `activate` without a state are recorded, not folded: the engine plays the patch until the host moves a value.
- **VST3.** The same code and the same state JSON. nice-plug drops `NOTE_END` for VST3; REAPER rejects VST3 parameter renames (`restartComponent(kParamTitlesChanged)`), so VST3 keeps "Slot N" even with the fork. A parameter set from the GUI while the host processes reaches the plugin through `process()`, and REAPER applies a script-set VST3 value only at the next process cycle. `ParamSetter` calls `IComponentHandler` on baseview's thread (F1b decision 3; PL checks write and touch modes with RH's `slot-automation`). clap-validator covers CLAP only: Steinberg's VST3 validator (537 of 537 on the shell spike) has not been run on `enth-plugin` yet.
- **No dirty marking.** nice-plug never calls CLAP `mark_dirty` or VST3 `setDirty`; an editor-only edit leaves REAPER's project clean (measured by RH's `dirty-flag`, [REAPER scenarios](#reaper-scenarios)). `ShellRequest::PatchEdited` is drained and ignored until a sixth fork patch adds the marking.
- **Editor close.** Stock nice-plug-egui 0.6.0 never calls `editor_closed`; the vendored fork does (lane FK). The shell uses it (end a drag, release auditions, forward the `End`s), still watches `is_open()` in `process()`, and treats every `build()` as a fresh open.
- **The vendored fork** (lane FK, `vendor/`): the plugin builds against it through `[patch.crates-io]`. It also fixes nice-plug's CLAP wrapper losing events that precede a splitting parameter event on the same frame under `SAMPLE_ACCURATE_AUTOMATION`; the F2b results below were re-measured on it.
- **Damaged fields.** Text that is not JSON never reaches a field's `set` (nice-plug refuses it, with a debug assertion that fires under `cfg(test)`); JSON that is not a patch loads the init patch and is kept as text until the first edit.
- **Unsupported sample rates** activate silent (above).

## For the next lanes

- **PL:** slot names through FK's fork (`names_dirty` → `set_display_name` on the main thread → `request_param_info_rescan`), value text, the `MidiCCs` decision (the mod wheel; then `mod-controls` can join the cli-equal profile once TH sends `controls`), voice info, VST3 parity and Steinberg's validator, `INPUT_EVENT_CAPACITY`, the `rt-stats` feature. The note tuple, `NOTE_END`, poly-mod as `MacroVoice` and the reset reports exist; review their policy.
- **ED1:** replace `Glue::frame`'s body with the editor and its layout seam; keep the frame protocol (pump, fold, draw, apply, drain, unlock, forward) and `Glue::apply`'s intent mapping. An editor preset load is `Session::load_patch` under the lock (the next frame's fold records the host's values); never `set_state`.
- **ED2:** a `TelemetryReader` per `build()`; repaints already come from the tap's wake flag.
- **RH:** done for the scenarios above; a new one loads any patch with `inject_state` and a chunk from `write_state_chunk`. After the dirty-marking fork patch, flip `DIRTY_MARKING_EXPECTED`; after a fix for flat lanes, flip `CLAP_FLAT_LANE_IS_KNOWN_GAP`.
