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

The plugin-level golden is `tools/test-host/golden/enth-plugin.toml` with its patch `enth-plugin.patch.json`; the REAPER scenarios' patches are `xtask/reaper/enth-state.patch.json` and `enth-automation.patch.json`.

## Identity

- CLAP id `dev.enth.synth`, VST3 class id `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD`, name "enth", vendor "enth" ([naming.md](naming.md); a test pins both ids). REAPER lists it as `CLAPi: enth (enth)` and `VST3i: enth (enth)`.
- 72 parameters in host order: `macro_1`…`macro_8`, then `slot_1`…`slot_64` (`#[nested(array)]`, the order of `enth_session::bridge::HOST_PARAM_IDS`). nice-plug hashes them to the ids hosts store: `macro_1` 825615486, `slot_1` 1248030256, `slot_64` 34232479, the same as the shell spike's (tests pin them, and `reaper-test` checks REAPER's idents).
- Each is a linear 0–1 `FloatParam` with no smoothing (the engine smooths). Macros default to 0 and take CLAP poly-mod (id = macro index); slots default to 0.5. Names are "Macro N" and "Slot N": stock nice-plug fixes names at construction. Lane FK's fork adds renaming; PL then sets `host_param_name` and asks for an info rescan when [`Shared::names_dirty`](#threads) is set.
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
6. **Editor closed**: `EguiEditorState::is_open()` falling (stock nice-plug-egui never calls `editor_closed`) disables the tap and sets `editor_closed` and `pump_wanted`; the next background pump releases audition notes.
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
| REAPER | `cargo xtask reaper-test --package enth-plugin` (Xvfb) | 53 checks, CLAP and VST3, three consecutive runs |

`reaper-test` loads a patch into the product as state through REAPER's `clap_chunk`/`vst_chunk` named config parameter (`common.lua`'s `inject_state`; layouts measured on REAPER 7.82: CLAP is the plugin's stream, VST3 is a `u32` length, a `u32` 1, the JSON and 8 zero bytes). `state-create` loads `enth-state.patch.json`, sets macro 1 = 0.7 and slot 1 = 0.3, saves; the checks then require the saved patch to come back byte for byte after a reload in a new process and to hold the folded host values (macro 1 0.7, slot 1's cutoff at `from_norm(0.3)`, about 159 Hz). `automation` loads `enth-automation.patch.json` (macro 1 raises osc1's level, slot 1 is the cutoff), so automating both from 0 to 1 is heard as louder and brighter.

## Known pitfalls

- **Voice ends on reset.** `Engine::reset` reports no `NOTE_END`. The shell copies the active voices' keys in `reset` and reports them at offset 0 of the next block; a host that tracks voices would otherwise wait forever.
- **Re-baseline on load.** Values the host restores with a state must not override the loaded patch. Three pieces: the session's first fold after a load only records (so the field folds the restored values right after `load_patch`, or a host change before the next save would be taken as the baseline and lost; the REAPER run found exactly that), the loaded plan forgets the engine's remembered slot values, and `process()` adopts the restored values without sending them (`rebaseline`, set before the plan goes out; `activate` clears it, since it re-baselines anyway, so a change in the first block still counts).
- **The patch wins at activation too.** Host values set before `activate` without a state are recorded, not folded: the engine plays the patch until the host moves a value.
- **VST3.** The same code and the same state JSON. nice-plug drops `NOTE_END` for VST3; REAPER rejects VST3 parameter renames (`restartComponent(kParamTitlesChanged)`), so VST3 keeps "Slot N" even with the fork. A parameter set from the GUI while the host processes reaches the plugin through `process()`, and REAPER applies a script-set VST3 value only at the next process cycle. `ParamSetter` calls `IComponentHandler` on baseview's thread (F1b decision 3; PL checks write and touch modes with RH's `slot-automation`). clap-validator covers CLAP only: Steinberg's VST3 validator (537 of 537 on the shell spike) has not been run on `enth-plugin` yet.
- **No dirty marking.** nice-plug never calls CLAP `mark_dirty` or VST3 `setDirty`; an editor-only edit may leave REAPER's project clean (an open check in phase-1b.md). `ShellRequest::PatchEdited` is drained and ignored until a fork patch adds it.
- **No editor close hook.** nice-plug-egui 0.6.0 never calls `editor_closed`; `process()` watches `is_open()`, and every `build()` is a fresh open.
- **Damaged fields.** Text that is not JSON never reaches a field's `set` (nice-plug refuses it, with a debug assertion that fires under `cfg(test)`); JSON that is not a patch loads the init patch and is kept as text until the first edit.
- **Unsupported sample rates** activate silent (above).

## For the next lanes

- **PL:** slot names through FK's fork (`names_dirty` → `set_display_name` on the main thread → `request_param_info_rescan`), value text, the `MidiCCs` decision (the mod wheel; then `mod-controls` can join the cli-equal profile once TH sends `controls`), voice info, VST3 parity and Steinberg's validator, `INPUT_EVENT_CAPACITY`, the `rt-stats` feature. The note tuple, `NOTE_END`, poly-mod as `MacroVoice` and the reset reports exist; review their policy.
- **ED1:** replace `Glue::frame`'s body with the editor and its layout seam; keep the frame protocol (pump, fold, draw, apply, drain, unlock, forward) and `Glue::apply`'s intent mapping. An editor preset load is `Session::load_patch` under the lock (the next frame's fold records the host's values); never `set_state`.
- **ED2:** a `TelemetryReader` per `build()`; repaints already come from the tap's wake flag.
- **RH:** the scenarios (`patch-roundtrip`, `slot-automation`, `macro-automation`, `undo-state-load`, `two-instances`) can load any patch with `inject_state` and a chunk from `write_state_chunk`.
