# The plugin (`enth-plugin`, L8)

**`enth-plugin` is the CLAP and VST3 shell around [`enth-session`](session.md), `enth-engine` and `enth-ui`, on nice-plug `=0.4.2` and nice-plug-egui `=0.6.0`.** It translates between the host and the product: 72 host parameters, notes, audio blocks, the saved state and the editor window. It holds no product rules: what an edit does, when a host value enters the patch and how a plan reaches the engine are the session's ([session.md](session.md)). Lane SK built the walking skeleton (freeze point F2b, [phase-1b.md](phase-1b.md)); lane PL completed the plugin (this page's state is PL's, 2026-10-20); ED1 and ED2 build the editor, RH the REAPER scenarios. Decisions behind it: [ADR-0001](adr/0001-plugin-shell.md), research report 01 in `_local/sessions/2026-10-19-p1b-plugin-editor/`.

## Files

| File | Holds | Lane |
| --- | --- | --- |
| `src/lib.rs` | the `Plugin`, `ClapPlugin` and `Vst3Plugin` impls, the export macros (in a module of their own), [`Shared`](#threads) (and the display names it sets), the background `Task` | SK, PL |
| `src/params.rs` | the 72 parameters and the two persistent fields, host value reads | SK |
| `src/process.rs` | [the audio block](#the-audio-block): host values, notes in chunks, `NOTE_END`, the pump, the rescan request, voice info | SK, PL |
| `src/notes.rs` | host notes, MIDI CC 1 and poly-mod as engine events; `NOTE_END` as nice-plug events | SK, PL |
| `src/ledger.rs` | the host notes not reported ended yet: [exactly one `NOTE_END` per host note](#note_end) | PL |
| `src/rt_stats.rs` | with the `rt-stats` cargo feature: [a timing histogram of `process()`](#rt-stats) | PL |
| `src/state.rs` | the persistent fields `patch` and `enth` | SK |
| `src/editor.rs` | the `NiceEguiApp` and `Glue`, which binds UI-C's cards to `SessionView`/`SessionEdit` | SK, ED1 |
| `src/tests_host.rs` | [in-process host tests](#tests-and-gates): the real plugin through fake nice-plug contexts, compared bit for bit with `enth_render` | PL |
| `clippy.toml` | the workspace's bans, minus I/O, plus `GuiContext::set_state`/`get_state`, `execute_gui` and `VoiceID::id_or_fallback` | SK |
| `tests/cli-equal.profile.toml` | the engine goldens `cli-equal` renders through the plugin (all of them since PL, `mod-controls` included) | SK, PL |

The plugin-level golden is `tools/test-host/golden/enth-plugin.toml` with its inputs `enth-plugin.patch.json`, `enth-plugin.bypass.state.json` and `enth-plugin.reverb-off.patch.json`; the REAPER scenarios' patches are the `xtask/reaper/enth-*.patch.json` files ([REAPER scenarios](#reaper-scenarios)).

## Identity

- CLAP id `dev.enth.synth`, VST3 class id `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD`, name "enth", vendor "enth" ([naming.md](naming.md); a test pins both ids). REAPER lists it as `CLAPi: enth (enth)` and `VST3i: enth (enth)`.
- 72 parameters in host order: `macro_1`…`macro_8`, then `slot_1`…`slot_64` (`#[nested(array)]`, the order of `enth_session::bridge::HOST_PARAM_IDS`). nice-plug hashes them to the ids hosts store: `macro_1` 825615486, `slot_1` 1248030256, `slot_64` 34232479, the same as the shell spike's (tests pin them, and `reaper-test` checks REAPER's idents).
- Each is a linear 0–1 `FloatParam` with no smoothing (the engine smooths). Macros default to 0 and take CLAP poly-mod (id = macro index); slots default to 0.5. The names fixed at construction are "Macro N" and "Slot N"; at run time they follow the patch ([Parameter names](#parameter-names)).
- Stereo out, no audio in. `MIDI_INPUT = MidiCCs` ([MIDI CCs](#midi-ccs-the-decision)): the mod wheel plays; VST3 lists 2,080 more (hidden) parameters. `SAMPLE_ACCURATE_AUTOMATION = true` (required, [Determinism](#determinism)). `CLAP_POLY_MODULATION_CONFIG` is 32 voices, overlapping; CLAP voice info reports the running plan's voice count ([below](#voice-info)).

## Threads

| Thread | Runs | Touches |
| --- | --- | --- |
| Audio (`process`, `reset`) | [the block](#the-audio-block) | the `Engine`, the `Audio` state (event list, [ledger](#note_end)), the atomics in `Shared`, the parameters' atomics. **Never the session lock** |
| Host main thread | `activate`, `deactivate`, state save and load (the persistent fields) | the session, through `Shared::with_session` |
| baseview's GUI thread | the editor's `build` and `ui` | the session, through `Shared::with_session`; `ParamSetter` after unlocking |
| nice-plug's background worker | `Task::Pump` | the session (pump, audition release) |

`Shared` holds the one lock, `parking_lot::Mutex<Session>`, and atomics: `pump_wanted`, `monitor_bypass`, `rebaseline`, `names_dirty`, `editor_closed`, plus a weak reference to the parameters for reading the host's values off the audio thread (and, with `rt-stats`, the histogram). **`Shared::with_session` is the only way in:** it locks, runs the closure, drains the session's requests, sets the 72 display names if one of them was `RenameParams` ([below](#parameter-names)), notes whether a compile is still pending, **unlocks**, then stores the monitor bypass, sets `names_dirty` on a rename and arms `pump_wanted`, and returns the host writes. Only editor edits make host writes, and the editor forwards them to `ParamSetter` after this returns, never under the lock, because a host may call back synchronously (to read the state, which locks).

## Lifecycle

- **Construction.** A `Session` on `Patch::init()`, the parameters, the editor state (sized by the theme's `size.window.default`), and an `Audio` whose event list and ledger are allocated here, never in `process()`.
- **`activate`** (main thread). Fold the host's values into the patch (`Session::fold_host`), then `Session::activate` (compile, `Engine::with_link` with `RENDER_SEED`, a disabled telemetry tap, the persisted monitor bypass on the fresh engine). The engine replaces the previous one, which drops here on the main thread. CLAP voice info gets the plan's voice count (`set_current_voice_capacity`). `Audio::activate` takes the host's current values as "last sent", so nothing is echoed and the engine starts from the patch, forgets the old engine's live notes (the host forgets its voices at deactivation, and a `NOTE_END` can only go out from `process()`), and clears `rebaseline` (this is the re-baseline a load before activation asked for). A sample rate outside the engine's 8–384 kHz activates **silent**, with the session detached, rather than failing: clap-validator sweeps 1 kHz to 768 kHz and a host that refuses a plugin is worse than silence.
- **`reset`** (audio thread; nice-plug also calls it at CLAP `start_processing` and VST3 `setProcessing(true)`). `Engine::reset()`, which frees the voices and empties the engine's queue without reporting them; the next block reports every live host note's `NOTE_END` at frame 0 ([the ledger](#note_end)).
- **`deactivate`** (main thread). `Session::deactivate`, then the engine drops. With `rt-stats`, the histogram's report is written ([below](#rt-stats)).

## The audio block

`Enth::process_block`, in order; nothing allocates, locks, logs or does I/O (nice-plug's `assert_process_allocs` aborts a debug build that does, and clap-validator and the test host's checks run the debug bundle):

1. **Host values.** Read the 72 parameters' modulated normalised values and send each one that changed since the last block as `EventKind::Macro { index, value }` or `EventKind::Slot { slot, value }` at offset 0. Never a bare `EventKind::Param`: a plan swap inside the block would misroute an id looked up before it. The modulated value lets the engine follow CLAP mono modulation while the patch keeps the unmodulated one. With sample-accurate automation nice-plug splits the host block at each parameter change, so offset 0 is the change's own frame. After a state load (`rebaseline`) the values are adopted without being sent.
2. **Reset ends.** After a `reset`, every live host note's `NOTE_END` at frame 0.
3. **Notes and render.** The monitor bypass goes to the engine, then the block's note events ([Notes](#notes)) into the event list (nice-plug's input capacity plus 72, 1,096 events) and `Engine::process`. **No host event is dropped:** when the list is full, the frames up to the event that did not fit are rendered with the list, and the list starts again from there. The engine renders the same bits for any split of a block (its grid is free-running), so this is invisible. After each chunk its `NOTE_END`s go out through the [ledger](#note_end), at their frames in the host block.
4. **Telemetry**: the tap's wake flag becomes a repaint request (lock-free).
5. **Editor closed**: `EguiEditorState::is_open()` falling disables the tap and sets `editor_closed` and `pump_wanted`; the next background pump releases audition notes. (The vendored nice-plug-egui also calls the editor's `editor_closed`, which does the session side on the GUI thread; both are idempotent.)
6. **The pump flag** ([below](#the-pump)).
7. **Rescan**: while `names_dirty` is set, at most once per 50 ms of audio, clear it and ask nice-plug for a parameter-info rescan ([Parameter names](#parameter-names)).
8. **Voice info**: when a plan swap changed the plan's voice count, report it (`set_current_voice_capacity`).

Steps 7 and 8 call nice-plug's own process-context functions, which queue one of nice-plug's main-thread tasks and call the host's `request_callback`; only a host that calls `process()` on its main thread gets the task inline (a host call: `rescan`, `voice_info.changed`), so both are wrapped in `permit_alloc` for the host's own work. Neither runs any of our code, takes our lock or allocates on our side. `execute_gui`, which would run **our** task inline, stays banned.

### Notes

A host note is `NoteKey { port: 0, channel, key, note_id: VoiceID::id() or −1 }` (`VoiceID::id()`, never `id_or_fallback`, which `clippy.toml` bans). In a note-off or choke a wildcard field is −1, the engine's wildcard; a note-on needs a key (a wildcard key is dropped) and takes channel 0 for a wildcard channel. Host notes are on port 0, so a host note-off never releases an editor audition note (`AUDITION_PORT`).

- **Zero-length notes.** A note-off the host lists ahead of the note-on with the same note id on one frame is moved behind it, so the note starts and ends (the order `enth render` gives a zero-length note) instead of starting after its own note-off and hanging. A note-off for another note id on that frame (a retrigger) is not moved. The test host's `note-end` script has such a note (`D4@0.25+0.00001` rounds to zero frames).
- **Poly-mod.** `PolyModulation` becomes `EventKind::MacroVoice { key: NoteKey::note_id(voice_id), .. }` for macros 0–7: only the voice whose note carries that host id moves (tested: id 9 moves, id 7 and an unknown id 1234 do not). nice-plug sends poly-mod only for a real note id; an id of −1, which would be the engine's wildcard, is dropped. `MonoAutomation` is ignored (step 1 already reads the modulated value). A note-on starts its voice's offsets at 0, so a modulation sent before the note starts waits for the host's next one.
- **MIDI CC 1** (the mod wheel's MSB), on any channel, becomes `EventKind::ModWheel` with nice-plug's `cc / 127`. Other CCs, CC 33 (the wheel's LSB), pitch bend and pressure are ignored: the engine has no input for them yet.

### `NOTE_END`

**Every host note-on gets exactly one `NOTE_END`**, with the note's own tuple (`VoiceTerminated`; nice-plug writes port 0; VST3 drops it). Hosts that track voices, and CLAP poly-mod, rely on it. The engine reports a voice's end, but some notes end without a report: more than 256 voices ending in one `Engine::process` call (an event flood: the engine counts the rest in `dropped_note_ends`); in mono and legato the one voice takes the new note's key, so the note it played before is never reported (and a return to a held key brings an already-replaced key back); and a note-on still waiting for its segment boundary when a reset empties the queue. So `ledger.rs` keeps every live host note (its tuple and the frame of its note-on). After each chunk: an engine report for a live note is sent and the note forgotten; a report for a note that is not live (already reported, or an audition note) is not sent again; then a **sweep** reports every live note whose note-on the engine has applied (the segment boundary after its frame has been rendered) and that no active voice plays, at the chunk's last frame. At a reset all live notes are reported at frame 0 of the next block.

In mono and legato the replaced note's end is therefore reported when the voice takes the new note, and a return to the held key does not end it twice. Cost: one scan of the active voices per live note per chunk (a few dozen comparisons while playing); the list holds 4,128 notes (the engine's queue plus every voice).

## Parameter names

On every `ShellRequest::RenameParams` (the session pushes it only when the host would see other names or kinds, and on every load), `Shared::with_session` sets the 72 display names from the patch with `enth_session::bridge::host_param_name` ("Filter 1 · Cutoff", a macro's own name; "Slot 7" for an empty slot), through lane FK's fork (`Param::set_display_name`; a name equal to the construction name clears the override). It does so **under the session lock**, so two threads that rename one after the other (an editor edit on the GUI thread, a host load on the main thread) always leave the later patch's names; `set_display_name` only stores a string behind the parameter's own lock and calls nothing, so the lock still never calls the host.

The host is then asked to re-read them: `names_dirty` is set after unlocking, and `process()` clears it and calls the fork's `ProcessContext::request_param_info_rescan` (step 7). nice-plug runs the rescan on the host's main thread: CLAP `clap_host_params.rescan(INFO | TEXT)`, VST3 `restartComponent(kParamTitlesChanged)`.

- **Why from `process()`.** A host state load runs on the main thread inside nice-plug's state code with the editor closed, and the fork offers the rescan only on the process and GUI contexts; nice-plug's activate context has none. design.md §4 asks for the rescan "on the main thread, never from `process()`": the rescan does run on the main thread, through `request_callback`, and what `process()` does is queue nice-plug's own task (the same path nice-plug's `set_current_voice_capacity` uses), never one of ours. Only a host that calls `process()` on its main thread gets the host call inline, and that is a main-thread call there too.
- **Measured in REAPER 7.82** (`reaper-test`, `state-create`, which loads `enth-state.patch.json` with a named macro and two slots on a labelled filter): **CLAP** shows "Bright", "Tone · Cutoff", "Tone · Resonance" once the rescan has run (checked after the scenario's 1.5 s wait; `param-renamed`). **VST3 shows them too, and at once:** REAPER ignores `kParamTitlesChanged` (as the Phase 0 spike found) but reads VST3 parameter titles live, so the fork's override reaches it anyway. The earlier note that "VST3 in REAPER stays Slot N" was about the title-change message, not what REAPER displays. An automation lane that already exists keeps its label but stays bound by id.
- **Not done: value text.** A slot's value is shown as the plain 0–1 number. Showing the target's units ("1.2 kHz") needs a `value_to_string`/`string_to_value` pair per slot that reads the slot's current spec; clap-validator's `param-conversions` test checks that text → value → text round trips, which rounding display text can break. Left open.

## MIDI CCs (the decision)

**`MIDI_INPUT = MidiConfig::MidiCCs`** (lane PL, 2026-10-20; it was the open check "decide `MidiCCs` before the first build leaves the machine"). With it the mod wheel (MIDI CC 1) reaches the engine in both formats; with `Basic` nice-plug drops every CC, so a patch's `modwheel` routes would never move. CLAP pays nothing. For VST3 nice-plug registers 130 × 16 = 2,080 hidden, read-only parameters (`IMidiMapping` maps a CC to one, nice-plug turns its changes back into `MidiCC` events), and REAPER lists them.

| Measured (REAPER 7.82 headless, Xvfb, 10 instantiations each) | `Basic` | `MidiCCs` |
| --- | --- | --- |
| Parameters REAPER lists, CLAP | 75 | 75 |
| Parameters REAPER lists, VST3 | 75 | **2,155** ("MIDI Ch. 1 CC 0" … after the 72) |
| `TrackFX_AddByName`, median, VST3, debug bundle | 2.77 ms | 13.50 ms |
| Reading every parameter name, VST3, debug bundle | 0.34 ms | 11.3 ms |
| `TrackFX_AddByName`, median, VST3, release bundle | — | 2.91 ms (name walk 3.47 ms) |
| `TrackFX_AddByName`, CLAP (debug / release) | 5.1 ms / — | 2.0 ms / 0.44 ms (noise: CLAP is unchanged) |
| clap-validator 0.4.1 (debug and release) | 36 passed, 0 failed, 8 skipped | the same |
| Steinberg's VST3 validator (SDK 3.8.1, Phase 0 build) | 47 passed, 0 failed (debug) | 47 passed, 0 failed (debug and release) |
| The mod wheel in a REAPER MIDI item (CC 1 0 → 127 halfway, routed to osc1's level at −100 %, offline render) | no effect in either format (RMS 0.384 → 0.384) | works in CLAP and VST3 (RMS 0.384 → 0.000) |
| `cli-equal` `mod-controls` | differs (exit 1) | equal at host blocks 1, 64, 100, 4096 |

Why on: the mod wheel is a modulation source of the patch model (routes from `modwheel`, a golden case, the CLI's `controls`), CLAP is the first format and pays nothing, and the VST3 cost is a longer parameter list in REAPER and a couple of milliseconds per instantiation. JUCE's VST3 wrapper exposes MIDI CCs the same way by default, so REAPER users know such lists. To undo it, set `MIDI_INPUT` back to `Basic` (and `reaper-test`'s VST3 `param-count` back to 75). A fork patch that hides the CC parameters from VST3 alone (a `Vst3Plugin` constant) would remove the cost, at the price of the VST3 mod wheel; that is lane FK's and the owner's call.

## Voice info

CLAP voice info reports the running plan's voice count: its polyphony in poly mode, 1 in mono and legato (`process::voice_count`), at `activate` and whenever a plan swap changes it (step 8). The capacity stays the engine's 32 voices, overlapping.

## The pump

A compile that the session could not send (two plans in flight: the host had stopped calling `process()`) waits for the session's next entry. With the editor closed there may be none, so ([session.md](session.md), "Plan delivery"; F1b decision 1):

- whoever unlocks the session with `compile_pending` set arms `pump_wanted` (`Shared::with_session` does it for every caller);
- `process()`, while the flag is set and at most once per `PUMP_INTERVAL_S` (50 ms) of audio, clears it and calls `execute_background(Task::Pump)`: one bounded `try_send` to nice-plug's worker, only while a compile is pending, never in steady state;
- the task locks the session, releases audition notes if the editor closed, pumps, and re-arms the flag if a compile is still pending.

Never `execute_gui` from `process()`: it can run the task inline (CLAP `request_callback` while the host calls `process()` on the main thread). `clippy.toml` bans it in the crate.

## The monitor bypass

The global FX bypass ([ADR-0006](adr/0006-effects-strategy.md)) is per-instance state outside the patch: `InstanceState::monitor.fx_bypass` in the `enth` field. The editor switches it with `SessionEdit::set_monitor_bypass`; the session pushes `ShellRequest::MonitorBypass`, `Shared` stores it in `monitor_bypass`, and every block hands it to the engine (`Engine::set_monitor_bypass`), which fades the `time_based` effects (the reverb) out or in over 20 ms. A loaded `enth` field reaches the engine the same way; on a fresh engine (a project opening with the bypass on) the switch is immediate. Tested: with the bypass on from the saved state the render equals `enth render` of the patch with its reverb disabled, bit for bit at host blocks 64 and 100, and through the test host the `monitor-bypass` and `reverb-off` host-render cases give one hash; switched on mid-render through the session, the engine fades for 20 ms and the setting is saved in `enth`, not in the patch. The editor's control is ED1's.

## State

nice-plug saves `{"version", "params": {id: {"f32": v}}, "fields": {...}}`; the CLAP stream is a `u64` length and that JSON. Two fields, both permanent names:

| Field | Text | On load (host main thread) | On save |
| --- | --- | --- | --- |
| `enth` | `InstanceState::to_json()`, `enth.plugin-state/1` | `Session::load_instance`; the monitor bypass reaches `process()` through `Shared` | `Session::save_instance` |
| `patch` | exactly `Patch::to_json()`, carried as a `serde_json::value::RawValue` (the `raw_value` feature; no extra crate), so it is neither escaped twice nor re-parsed | set `rebaseline`, `Session::load_patch` (the patch wins; its plan goes out at once and forgets the engine's remembered host slot values; the names are set), then `fold_host` with the restored values, which only records them | `Session::save_patch` with the host's values: folded first (rule 2 and 3 of the bridge), then the patch text, or the kept text of a patch that did not load |

nice-plug sets the parameters first, then `enth`, then `patch`, and neither re-activates nor resets after a load, so the engine is kept and the session sends the loaded patch's plan itself. **Host values are re-baselined on load:** the restored values never reach the engine as host events (`process()` adopts them, tested: after a load with slot 1 restored to 0.9 the engine remembers no host value for slot 1, and a later host change to 0.3 reaches it). A `.patch.json` file and the `patch` field are interchangeable: the test host's `--state some.patch.json` puts the file's text into the field, and the plugin saves it back byte for byte (`render --save-state F --check-roundtrip` reports `identical_text: true` for `enth-plugin.patch.json`).

**Never `GuiContext::set_state`** (nice-plug applies it on the audio thread while processing, where the field would lock and compile) **or `GuiContext::get_state`** (it serialises the fields, which lock: a deadlock inside an editor frame). An editor preset load calls `Session::load_patch` under the lock on the GUI thread; the host's values do not move, and the next frame's fold records them as the baseline (stale ones never enter the new patch).

## The editor

F2b's editor is one real card: UI-C's `Card` for the first sound source (`osc1` in the init patch), as a strip, with "More" for the advanced tier. `Glue` turns the frame's intents into session calls through `SessionEdit` only, so the same code runs in a test with a plain `Session`: `Begin` opens a gesture, `Set`/`SetMacro`/`SetDepth` edit inside it, `End` closes it, a release of the pointer anywhere closes it too, and clicks, toggles and enable switches are single edits. `ShowMore` is editor view state, kept in `Glue`.

Each frame (GUI thread): read the host's values, then under the lock `pump`, `fold_host`, draw from `SessionView::patch` inside a `CentralPanel`, apply the intents; unlock; forward the host writes (`begin_set_parameter`, `set_parameter_normalized`, `end_set_parameter`). Every `build()` applies the theme and ends any gesture the last window left open. An editor edit that renames (the slot "Automate" menu, a label, a macro name) sets the names inside the same `with_session` call; `process()` requests the rescan.

ED1 replaces the body with the full editor and its layout seam; the frame protocol stays. ED2 adds a `TelemetryReader` (create it in `build()` from `telemetry_source()`, `set_enabled(true)`; `process()` already disables the tap when the window closes and turns the tap's wake flag into repaints).

## rt-stats

`cargo xtask bundle enth-plugin --features rt-stats` (the bundler passes cargo options through) compiles `rt_stats.rs`: `process_block` reads `Instant::now()` at its top and records the elapsed time at its end (the plugin's own work, nice-plug's wrapper excluded) into a log-scale histogram of atomic counters, 16 buckets per octave of nanoseconds (each at most 6.25 % wide), plus the count, frames, sum and maximum: relaxed atomic adds, no lock, no allocation. `Shared::rt_stats.snapshot()` gives p50, p99, p99.9 (the upper edge of the bucket that holds them, so at most one bucket high) and the maximum; at `deactivate`, if `ENTH_RT_STATS` names a file, one JSON line (`enth.plugin-rt-stats/1`, microseconds, with the non-empty buckets) is appended and the counters restart. Without the feature the module is not compiled and `process()` reads no clock. It complements `enth-test-host process-stats`, which times the whole `process()` call from the host's side. Checked end to end: a debug feature bundle rendering the golden chord through `enth-test-host render` (1,500 blocks of 64 frames) wrote one report line at deactivation (debug build, a loaded laptop: p50 39 µs, p99 59 µs, max 76 µs; not an RT measurement).

## Determinism

A plugin render equals `enth render` when the sample rate matches, the engine was not re-activated or reset during the render, no plan swap, monitor bypass or audition note happened, and every event is one a `Performance` can express (notes, slot automation, macros and the mod wheel). The shell keeps its side: `SAMPLE_ACCURATE_AUTOMATION = true`, `RENDER_SEED` (the CLI's default), no echo of host values at activate, slot events resolved by the engine with the patch-wins equality rule, `reset` equal to a fresh engine, the `patch` field carried as text, events never dropped (chunks), and a zero-length note ordered like the CLI's.

Measured at PL (2026-10-20):

- `enth-test-host cli-equal` (debug and release bundles): **63 of 63** engine golden cases equal through the plugin at host blocks 1, 64, 100 and 4096, `mod-controls` included (its macros as host parameters, its mod wheel as MIDI CC 1); 10 are skipped by the test host (render seeds other than the default, the stand-in sine, engine-address automation that needs a profile).
- In-process (`tests_host.rs`), bit for bit against `enth_render`: slot automation (the CLI automates the target with `from_norm` of the host value), macro moves and the mod wheel (`controls`) at host blocks 1, 64, 100 and 4096; the monitor bypass against the patch without its reverb; a zero-length note; the rest of a render after a reset against a fresh render; a reactivation.

## Tests and gates

| Check | Command | PL result (2026-10-20) |
| --- | --- | --- |
| Unit and in-process host tests | `cargo test -p enth-plugin [--features rt-stats]` | 39 (41 with `rt-stats`): SK's 23, plus the ledger (a queued note is not swept early, mono and legato replaced notes end once, 600 ends in one segment all reported), the mod wheel and poly-mod mapping, and the in-process host: renders equal to `enth_render` (automation, macros, mod wheel, bypass, zero-length note, reset, reactivation), a 3,000-event block rendered in chunks with every note ended once, load re-baselining, poly-mod by host voice id, renames and one rescan per change, voice info |
| clap-validator 0.4.1 | `cargo xtask validate --package enth-plugin [--release]` | 36 passed, 0 failed, 8 skipped (debug and release) |
| Steinberg's VST3 validator | the Phase 0 build at `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/build-vst3sdk/bin/Release/validator <bundle>.vst3`, under `xvfb-run` | 47 passed, 0 failed (debug and release) |
| CLI = plugin | `enth-test-host cli-equal target/bundled/enth-plugin.clap --profile crates/enth-plugin/tests/cli-equal.profile.toml --min-compared 60` | 63 equal, exit 0 (debug and release); the `Basic` build is the negative control for `mod-controls` (exit 1) |
| Event bursts | `enth-test-host bursts <clap> --order both --capacity 1024` | exit 0: every count up to 4,096 events in a 64-frame block applied in both orders, 1,370 of 1,370 notes ended once (before PL: exit 1, 833 notes without `NOTE_END` at 4,096) |
| `NOTE_END` | `enth-test-host note-end <clap> --state tools/test-host/golden/enth-plugin.patch.json` | exit 0 at host blocks 1, 32, 64, 100, 512 (before PL: the zero-length note never ended) |
| Plugin golden | `cargo xtask host-render --package enth-plugin [--release]` | 10 cases, the same hashes for debug and release: SK's 7 unchanged, plus `monitor-bypass` = `reverb-off` and `steal-automation` |
| REAPER | `xvfb-run -a cargo xtask reaper-test --package enth-plugin [--bundle-dir D]` | RH's 8 scenarios and the cross-format checks, CLAP and VST3 ([REAPER scenarios](#reaper-scenarios)), plus PL's `param-renamed`; VST3 `param-count` is 2,155 (the MIDI CC parameters). PL's results: REAPER_RESULT |

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

- On CLAP a slot lane that never moves was usually ignored in the offline render (2 of 3 of RH's acceptance runs), and the first note of a lane that starts at the default 0.0 differed on CLAP only. **Fixed by PL** ([State](#state), "re-baselined on load"): a trace showed REAPER loading the project's state while the plugin is active and then sending the lane's value once, in the first block, which the load's re-baseline adopted silently along with the restored values. The plugin now adopts only the values the load restored. `CLAP_FLAT_LANE_IS_KNOWN_GAP` is `false`, so the CLAP flat lane is a check (PL: FLAT_RESULT).
- After a state load the host's macro display may differ from the patch's macro value (design.md section 4 allows it); `undo-state-load` and `patch-roundtrip` compare the patch and the slots, not the macro display.

**Maintaining the fixtures.** `enth-roundtrip.patch.json` must be the plugin's own saved text (key order matters); regenerate it from a first save when the patch format or the engine version changes. `dirty.lua`'s first knob position belongs to the F2b editor; the grid search takes over by itself when the editor's layout changes, only slower. The dirty-flag control and the two automation patches (`enth-slot-automation.patch.json`, `enth-macro-automation.patch.json`) route a slot or a macro to osc1's level or the filter cutoff; keep them when the routing changes.

The test host resolves a script's parameter names against the names a fresh instance shows as well as the current ones, so `Slot 1=…` keeps working after a state renamed slot 1 (lane PL's addition to `tools/test-host`).

## Known pitfalls

- **Voice ends.** `Engine::reset` reports no `NOTE_END`, the engine reports at most 256 ends per call, and in mono a voice's key changes under it: the [ledger](#note_end) covers all three.
- **Re-baseline on load.** Values the host restores with a state must not override the loaded patch. Three pieces: the session's first fold after a load only records (so the field folds the restored values right after `load_patch`, or a host change before the next save would be taken as the baseline and lost; the REAPER run found exactly that), the loaded plan forgets the engine's remembered slot values, and `process()` adopts the restored values without sending them (`rebaseline`, set before the plan goes out; `activate` clears it, since it re-baselines anyway, so a change in the first block still counts).
- **The patch wins at activation too.** Host values set before `activate` without a state are recorded, not folded: the engine plays the patch until the host moves a value.
- **Script names after a rename.** A host or script that finds parameters by name sees "Filter 1 · Cutoff" after a load, not "Slot 1". Ids never change; find parameters by id.
- **VST3.** The same code and the same state JSON. nice-plug drops `NOTE_END` and poly-mod for VST3, and voice info is CLAP's. REAPER reads VST3 titles live, so the renamed names show at once (above). The mod wheel arrives through the 2,080 MIDI CC parameters. A parameter set from the GUI while the host processes reaches the plugin through `process()`, and REAPER applies a script-set VST3 value only at the next process cycle. `ParamSetter` calls `IComponentHandler` on baseview's thread (F1b decision 3; RH's `slot-automation` scenario on lane RH's branch passes against PL's bundle, but write and touch modes from the editor are not driven there).
- **No dirty marking.** nice-plug never calls CLAP `mark_dirty` or VST3 `setDirty`; an editor-only edit leaves REAPER's project clean (measured by RH's `dirty-flag`, [REAPER scenarios](#reaper-scenarios)). `ShellRequest::PatchEdited` is drained and ignored until a sixth fork patch adds the marking.
- **Editor close.** Stock nice-plug-egui 0.6.0 never calls `editor_closed`; the vendored fork does (lane FK). The shell uses it (end a drag, release auditions, forward the `End`s), still watches `is_open()` in `process()`, and treats every `build()` as a fresh open.
- **The vendored fork** (lane FK, `vendor/`): the plugin builds against it through `[patch.crates-io]`. It also fixes nice-plug's CLAP wrapper losing events that precede a splitting parameter event on the same frame under `SAMPLE_ACCURATE_AUTOMATION`.
- **Damaged fields.** Text that is not JSON never reaches a field's `set` (nice-plug refuses it, with a debug assertion that fires under `cfg(test)`); JSON that is not a patch loads the init patch and is kept as text until the first edit.
- **Unsupported sample rates** activate silent (above).
- **nice-plug's own input queue** grows (one allocation, permitted inside its wrapper and logged) when a host sends more than 1,024 events in a block; the plugin's list then renders in chunks. The engine drops events only past 4,096 waiting for one segment boundary.

## For the next lanes

- **RT:** build with `--features rt-stats` and read `Shared::rt_stats` or the `ENTH_RT_STATS` report for the plugin's own share of `process()`; `process-stats` times the whole call. A block with an event flood now renders in chunks (several `Engine::process` calls, each of which renders at least the segments it needs), and the ledger sweeps once per chunk: measure a flood block, not only steady state. The rescan request and voice-info report are nice-plug task queue pushes plus `request_callback`, made only on a rename or a voice-count change. The pump's `try_send` is unchanged.
- **ED1:** an editor rename sets the names inside `with_session` (nothing to do in the editor); the monitor bypass control is `SessionEdit::set_monitor_bypass`, already wired to the engine with its 20 ms fade.
- **ED2:** a `TelemetryReader` per `build()`; repaints already come from the tap's wake flag.
- **RH:** done for the scenarios above; a new one loads any patch with `inject_state` and a chunk from `write_state_chunk`. After the dirty-marking fork patch, flip `DIRTY_MARKING_EXPECTED`. PL changed `state.lua` (the patch's names accepted in `param-names` and checked in `param-renamed`; 2,155 VST3 parameters) and flipped `CLAP_FLAT_LANE_IS_KNOWN_GAP`. A mod-wheel scenario (a MIDI item with CC 1) would pin the `MidiCCs` decision; PL's experiment is in `~/code/enth-scratch/p1b/pl/modwheel/`.
- **INT and owner build 1:** check that renamed slot names show in REAPER on a real display (CLAP after a moment, VST3 at once), the length of REAPER's VST3 parameter list with the MIDI CC entries, and the mod wheel from a hardware controller in both formats.
- **Open:** slot value text in the target's units; a fork patch hiding the VST3 MIDI CC parameters (FK, owner); poly-mod in a real CLAP host (Bitwig, owned by S1).
