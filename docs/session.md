# The session (`enth-session`, L6)

**One single-threaded `Session` owns the live patch.** It holds the `Patch`, its `History`, the engine's `Link`, the `PlanMap` of the latest plan it sent, and the per-instance state outside the patch. The editor and the plugin shell change the patch only through it. It does no I/O, spawns no threads, reads no clocks and takes no locks; the plugin wraps it in one mutex. Lane S1s wrote this page and the crate's public API (freeze point F1b, [phase-1b.md](phase-1b.md)); lanes SK and S2s fill in the bodies that talk to the engine. Sources: research report 01 (R2, R3, R7, R8) and 02 (D5, D7) in `_local/sessions/2026-10-19-p1b-plugin-editor/`, [design.md §3–§4](design.md#4-host-parameters-macros--automation-slots), and the engine seams in [dsp/engine.md](dsp/engine.md#p1b-seams-lane-e-what-the-session-plugin-and-editor-lanes-rely-on).

## The API in brief

| Item | Module | What it is | State (F1b) |
| --- | --- | --- | --- |
| `Session` | `model` | The owner: `new`, `patch`, `history`, `instance`, `status` (with `patch_unloaded`), `drain_requests`, `set_ui`, `set_monitor_bypass`, telemetry accessors | Implemented |
| `SessionView` | `model` | The editor's read model: patch, history, status, instance state, telemetry map and source | Implemented for `Session`; fakes can implement it |
| `SessionEdit` | `model` | The editor's edit trait: gestures, `edit(ops, gesture)`, undo, redo, UI state, monitor bypass, audition | Gestures implemented; `edit` is SK's stub, `undo`/`redo` S2s's, audition SK's |
| `ShellRequest` | `model` | The outbox the shell drains: host gesture writes, renames, the monitor bypass, "patch edited" | Implemented |
| `SessionError` | `model` | `Edit`, `Compile`, `State`, `Inactive`, `Busy`, `Offline`, `Unsupported`; each with `code()` and `hint()` | Implemented |
| `Routing`, `Delivery`, `route`, `delivery` | `apply` | How an op reaches the engine (the table below) | Implemented and tested per op kind |
| `HostParam`, `HostValues`, `HOST_PARAM_IDS` | `bridge` | The 72 host parameters with permanent ids (`macro_1`…`macro_8`, `slot_1`…`slot_64`) | Implemented |
| `fold_value`, `Session::fold_host`, `host_values`, `host_updates`, `slots_written`, `host_param_name` | `bridge` | The patch-wins rule, host writes, the slots a recompile must forget, names | Implemented and tested |
| `InstanceState`, `SavedState`, `LoadReport`, `StateError` | `state` | The saved state v1: the `patch` field and the `enth` field (`enth.plugin-state/1`) | Implemented, fixtures under `tests/fixtures/state/v1/` |
| `Session::{save, save_patch, save_instance, load, load_patch, load_instance}` | `state` | Save and load; a patch that does not load is kept as text and saved back unchanged until the first edit | Implemented (the host-load history policy is S2s's to revisit) |
| `Session::{activate, deactivate, pump}`, `Activation`, `PumpReport` | `link` | The engine side | `deactivate` implemented; `activate` and the active `pump` are SK's stubs |
| `TelemetrySource`, `TelemetryFrame`, `TelemetryMap`, `TelemetryReader` | `telemetry` | The tap's reader side; `TelemetryFrame` is the engine's `TapFrame` (`non_exhaustive`: fakes build it from `Default`), and `Tap` implements `TelemetrySource`; the reader follows a re-activation (`set_enabled`, `refresh`) | Implemented and tested against a real engine |

Stubs return `SessionError::Unsupported { what, lane }` (or `Inactive` while detached), never panic, and name the lane that fills them in.

## Threading contract

### Who calls what, on which thread

| Thread | Calls into the session | Never |
| --- | --- | --- |
| **Host main thread** | nice-plug `initialize`/`activate`: `fold_host`, `activate` (returns the `Engine`); `deactivate`; state save through the persistent fields' `map`: `save_patch(Some(&host))`, `save_instance()`; state load through the fields' `set`: `load_instance`, `load_patch`; then `drain_requests` | Call into the host while holding the lock |
| **baseview's GUI thread** (the editor; a separate thread in baseview 0.4.1) | Once per frame, inside egui's update closure only (never across painting or the buffer swap): `pump`, `fold_host(&host)`, then the editor's reads and edits through `SessionView`/`SessionEdit`, then `drain_requests` into a local buffer; **unlock**; then forward `ShellRequest::Host` writes to `ParamSetter` | `GuiContext::set_state` (see below); keep the lock while calling `ParamSetter` |
| **nice-plug's background worker** (`bg-worker`) | The shell's "pump wanted" task: lock, `pump`, unlock (see "Plan delivery") | Anything but `pump` (it makes no `ShellRequest`, so there is nothing to forward) |
| **Audio thread** (`process`, CLAP `reset`) | **Nothing.** It owns the `Engine`, a preallocated event buffer and the last host values it saw. The one thing it schedules is the background pump task, rate-limited | Lock the session; touch it; schedule GUI work (`execute_gui`) |

- **The mutex.** The plugin holds `parking_lot::Mutex<Session>` in its shared state, used by the host main thread, the GUI thread and nice-plug's background worker (the pump task), never the audio thread. It is the only lock, so there is no lock order; nobody holding it waits on another thread or calls the host, so no thread can deadlock on it. A hold is at most one editor frame's edits, with at most one plan compile (about 0.9 ms for the G1 patch, mostly the reverb's preparation; [design.md §3](design.md#3-engine-and-real-time-safety)). The session itself takes no lock and is `Send` (a test checks).
- **Never call into the host or nice-plug while holding the lock.** A host may call back synchronously on the same thread (REAPER is reported, anecdotally, to read the state for an undo point when a parameter changes), and the persistent field's `map` would then lock the session again and deadlock. So the shell drains `ShellRequest`s into a local buffer, unlocks, and only then acts on them. This matters more once baseview PR #357 lands and the GUI thread becomes the main thread; the mutex stays correct either way.
- **The audio thread's side.** `process()` sends host automation as `EventKind::Slot { slot, value }` (the modulated normalised value, so CLAP mono modulation follows), macros as `EventKind::Macro`, CLAP poly-mod as `EventKind::MacroVoice { key: NoteKey::note_id(id), .. }`, never a bare `EventKind::Param` (a plan swap inside the block would misroute it). It sends only values that changed since its last block; at activate the shell sets its "last seen" values to the current host values, so nothing is echoed. CLAP `reset` is `Engine::reset()` there and nothing else. After `process` it takes the tap's wake flag (`engine.tap()` → `Tap::take_wake`) and asks for a repaint, which is a lock-free call.
- **A host state load keeps the engine.** nice-plug 0.4.2's state load sets the 72 parameters, then the persistent fields (`enth`, then `patch`), and does **not** re-run `initialize` (unlike nih-plug), so the shell does not re-activate: `load_patch` pumps the loaded patch's plan out at once and marks it to forget the engine's remembered host slot values (`Plan::forget_host_slots(u64::MAX)`), so the loaded patch wins in the engine too. A fold the GUI makes between the parameters and the fields lands in the old patch and is replaced; the first fold after the field set only records.
- **The `set_state` pitfall.** Never call `GuiContext::set_state`, from the editor or anywhere: while the plugin is processing, nice-plug applies that state **on the audio thread**, where the field's `set` would lock the session and compile a plan. An editor preset load calls `Session::load_patch` directly, on the GUI thread, under the lock. A clippy `disallowed-methods` entry in `enth-plugin` bans it, along with `execute_gui` from `process()`.
- **Host writes only on the GUI thread.** `ShellRequest::Host` writes are produced only inside `SessionEdit` calls, which only the editor makes, and the editor glue drains them before it unlocks. A main-thread or background drain therefore never sees one. A load that interrupts a drag leaves the host gestures it opened open until the editor's next `end_gesture`, so begins and ends stay balanced and are sent from the same thread. For VST3, nice-plug's `ParamSetter` calls `IComponentHandler::beginEdit/performEdit/endEdit` on the calling thread (baseview's, not the host's UI thread, as in every nih-plug egui plugin on Linux) and, while the host processes, leaves the parameter for the host to pass back in `process()`; lane PL confirms it with RH's `slot-automation` scenario on VST3 in REAPER (F1b decision 3).

### Plan delivery, and who drains the return ring

**The session, at its next entry; nobody needs to before that.** `Session::pump` takes replaced plans back (`Link::collect`) and drops them on the calling thread; it runs at the top of every entry that can send (an edit, undo, redo, a load, a save, activate), once per editor frame, and **again before an entry returns if it left a compile pending** (a load marks one and then pumps), so a plan goes out at once whenever fewer than two are in flight. Plans are only ever sent from such entries, at most `MAX_PLANS_IN_FLIGHT` (2) are outstanding, and the return ring has room for at least that many plus a pending slot (`Session::with_config` raises a smaller `returns` to 2). So the audio thread can always hand every plan back, and an idle session costs at most two retired plans' memory, about 1.6 MB each with a reverb, until the next entry. No timer runs while the editor is closed.

**A compile left pending** (two plans in flight: the host had stopped calling `process()`, then the editor closed or went idle, or a host load arrived) would otherwise wait for the next entry while the engine plays an older plan. The shell closes that without a fork patch (F1b decision 1): after unlocking, when `status().compile_pending`, it sets an atomic "pump wanted" flag; `process()`, while the flag is set, at most once per 50 ms of audio, clears it and calls `execute_background(Task::Pump)`; the task runs on nice-plug's background worker (never inline, for CLAP and VST3 alike), locks the session, pumps, and sets the flag again if a compile is still pending. Cost on the audio thread: one bounded crossbeam `try_send` (it wakes the parked worker, which takes crossbeam's internal lock briefly; the nightly RTSan run will see it), only while a compile is pending, never in steady state. A fork patch exposing CLAP `request_callback` without nice-plug's inline execution would be the RT-clean alternative for CLAP only; it is not needed for P1b.

### Editor open and close

- Every `build()` is a fresh open (nice-plug-egui calls no close hook without the fork): the editor ends any open gesture (`end_gesture`), creates a `TelemetryReader` from `telemetry_source()` (or, while inactive, when it first returns one) and calls `reader.set_enabled(true)`.
- The reader follows the session: after a re-activation with the editor open (a sample-rate change, REAPER's restart at the start of an offline render) `refresh` switches to the new tap, moves the enabled flag to it and drops the old map (a new engine counts epochs from 0 again).
- Closing (the fork's `editor_closed`, or `EguiEditorState::is_open()` turning false): `end_gesture`, `audition_all_off`, `reader.set_enabled(false)`. The session releases audition notes itself on deactivate and on every load.

## Delivery

`apply::route(before, op)` classifies an op against the patch before it; `apply::delivery(before, ops)` turns a transaction into one plan or a list of events.

| `ops::effect` | Routing | The session sends | The engine |
| --- | --- | --- | --- |
| `Value(addr)` | `Value` | `PlanMap::get(addr)` → `Link::send_param(epoch, id, plain)`; nothing if the address is not in the plan (a disabled module: the next plan carries it) | Base set at the next segment boundary, smoothed; the slot table's patch value follows |
| `Depth(route)`, old and new depth non-zero | `Depth` | `PlanMap::depth(route)` → `Link::send_depth(epoch, id, pct)`; a recompile if the id is `None` | Ramps over 10 ms, then exactly the recompiled value |
| `Depth(route)` from or to 0 | `Recompile` | A new plan (depth-0 routes have no term) | Swap, no crossfade |
| `Macro(i)` | `Macro` | `Link::send_macro(i, v)` | Smoothed over 10 ms |
| `Structural` | `Recompile` | A new plan | Swap with state carry, moved at an instant swap; crossfade only if modules were added or removed |
| `NoAudio` from `SetSlot` | `Recompile` | A new plan (the slot table lives in the plan), and one host write of the new target's value, only if that plan went out in the same entry (a write that reached `process()` first would go through the old slot table to the previous target) | The remembered host value is forgotten (the slot's key changed) |
| `NoAudio` from `SetLabel`, `SetMeta`, `SetMacroName` | `Silent` | `ShellRequest::RenameParams` when names may change | — |

A transaction with any `Recompile` op is **one** plan compiled from the patch after it, and its value events are skipped (the plan carries them); the plan forgets the remembered host values of the slots its `SetParam`s write (`bridge::slots_written` → `Plan::forget_host_slots`), so the engine plays the edited value instead of re-applying the old host value until the host's echo arrives. Undo and redo deliver their ops the same way. Editor edits also produce host writes (`bridge::host_updates`): a `SetParam` on a slotted address sets each of its slots to `to_norm` of the new value, a `SetMacro` sets its macro, a `SetSlot` sets the new target's value. In a gesture the session sends `Begin` at the first write to a parameter and `End` for each at `end_gesture`; outside a gesture it sends begin, set and end at once.

### Invariants

Each gets a test in the lane that implements it (S2s's acceptance lists them).

- **I1, one writer.** Every patch change is an `Op` applied by the session. Editor edits, undo and redo are recorded; host folds and loads are not.
- **I2, delivery follows the op**, after the transaction succeeded. A rejected transaction changes nothing and delivers nothing.
- **I3, lazy plans.** At most one compile is pending, compiled from the patch at the moment it can be sent, so a plan always reflects the patch at the time it was sent.
- **I4, epochs.** Value and depth events carry the epoch of the latest *sent* plan and use its `PlanMap`.
- **I5, never blocks, never drops.** `SendError::InFlight` keeps the compile pending. `SendError::Full` queues the value (the last per parameter, at most `SessionConfig::value_queue`); an overflowing queue is cleared and a compile marked pending instead. Audition note-offs are never dropped: the session queues them, and the engine leaves an event from the ring in the ring while its event queue is full (F1b review); a note-on that does not fit the session's queue is refused with `Busy`.
- **I6, the patch wins** (next section).
- **I7, load.** The patch wins, in the session and in the engine: host values restored with the state are not folded during the load, and the loaded patch's plan forgets the engine's remembered host slot values. The history is cleared and the load counts as saved; names are refreshed; while active the plan is sent at once (or as soon as the link has room). A field that does not load gives the init patch (or the default instance state), a typed error in the report and in `last_error`; it never panics. The text of a `patch` field that did not load is kept, and saved back unchanged until the first edit, so a project from a newer enth survives being opened and saved in an older build.
- **I8, save.** Pump, fold the host's values, then write `Patch::to_json()` (or the kept text, above). A host project save does not touch the history or the dirty flag.
- **I9, undo and redo** deliver through I2; a structural undo is exactly one plan.
- **I10, audition notes** use `AUDITION_PORT` (`0x7F00`) and a fresh note id each, go through `Link::send_event`, are refused while the host renders offline, and are released on editor close, deactivate and load.

## The host bridge: the patch wins

The host sees 72 linear 0–1 parameters with permanent ids. Plain → normalised → plain is not a round trip (33–82 % of values move on Log, Power and signed parameters), so three rules decide when a host value enters the patch (`Session::fold_host`, called once per editor frame and before every save):

1. **The patch wins at load.** The first fold after construction or a load only records the host's values.
2. **A host value is folded only when it changed since the session last saw it.** A stale host value, after a slot reassignment that never reached the host, an editor preset load, or a host that restored no parameters, never overwrites the patch.
3. **And only when it truly differs** (`bridge::fold_value`): if the host's `n` equals `to_norm(p)` bit for bit the patch keeps `p`; otherwise the candidate is the canonical `from_norm(n)`, written (without history) only if it differs from `p` bit for bit. Folding the same value again, or after a save and reload, never moves the patch. Non-finite host values are ignored.

The engine resolves slot events with the same equality rule, and an editor edit updates the slot table's patch value, so the host's echo of an edit (`to_norm` of the same value, the same pinned function) lands on the exact plain value. A fold sends nothing to the engine: the engine already follows the host through slot and macro events from `process()`. Both sides canonicalise the same way (`from_norm` then the periodic wrap). One known gap: the engine compares with the value its plan was compiled with (or the last editor edit), the fold with the current patch, which earlier folds moved; a host that returns exactly to `to_norm` of the compiled value then plays it exactly in the engine while the patch takes `from_norm` of it, one rounding step, healed at the next recompile (S2s may fold against the plan's value).

Two slots may drive one address. The patch allows it (a transaction that moves an assignment passes through a duplicate, and hand-written files may hold one) and every rule handles it deterministically: the higher slot wins a fold, the later event wins in the engine, an edit writes every slot of the address, and `slots_written` names them all. The editor's "Automate" menu reuses the address's existing slot (`bridge::slots_for`) before taking a free one (F1b decision 4).

## The saved state, version 1

Two nice-plug persistent fields, both permanent names ([naming.md](naming.md)):

| Field | Text | Versioned by |
| --- | --- | --- |
| `patch` | Exactly `Patch::to_json()`, carried raw (never re-parsed through `serde_json::Value`) | The patch file's `schema` number and `enth_patch::migrate`, the same path as `enth render` |
| `enth` | `{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":false},"ui":{"layout":"lanes","scale_percent":100,"advanced":false}}` | Its `schema` string and `state::INSTANCE_STEPS` |

- The slot assignments and the macros (values and names) live in the patch; the 72 host values are nice-plug's own `params` and are never needed to rebuild the sound.
- Byte-deterministic (clap-validator's `state-reproducibility-binary`); each field at most 1 MiB; the two fields load independently (nice-plug sets `enth` before `patch`).
- Within version 1, a new optional field with a default is allowed (unknown fields are ignored, missing ones default). Anything else is version 2: a step in `INSTANCE_STEPS`, `v2/` fixtures, and the `v1/` fixtures kept as migration inputs. A newer version than the build knows is refused (`schema_too_new`).
- A `patch` field that does not load (a newer schema, damage) leaves the init patch playing, `status().patch_unloaded`, and its text, which every save writes back unchanged until the first edit, undo or redo. A field over 1 MiB is not kept. An `enth` field that does not load gives the defaults and is re-saved with them (only preferences are lost); unknown fields of a newer version 1 are dropped on re-save.
- Fixtures: `crates/enth-session/tests/fixtures/state/v1/{default,custom}.{patch,enth}.json`. `custom` carries three slots (the first, the second and the last), a named macro, a module label, a route and non-default editor preferences. Regenerate on purpose with `cargo test -p enth-session --test state -- --ignored regenerate` and read the diff.

## Decisions

1. **An outbox, not a host trait.** The session pushes `ShellRequest`s (host gesture writes, renames, the monitor bypass, "patch edited") that the shell drains after unlocking. Report 01 passed a `HostSync` trait into every edit instead. The outbox keeps the editor's traits free of host concepts, lets kittest assert on requests, and makes "never call the host under the lock" the natural shape.
2. **Two traits for the editor:** `SessionView` (read) and `SessionEdit: SessionView` (edits). Both are object-safe and implementable outside the crate (a test does it). Pumping and folding stay out of them: the editor glue does those, so the editor never reasons about the engine or the host.
3. **Host values are folded only when they changed since last seen, and only on a true difference.** Rule 2 is new against report 01's I6. Without it, a slot reassignment that never reached the host, or a host that restored no parameters, would overwrite the patch with a stale value.
4. **The saved state is two fields**, as architecture.md and naming.md fix them. The `enth` blob holds only what is not in the patch, and its UI scale is an integer percent, so the bytes never depend on float formatting.
5. **`load_patch` is implemented now** with the plain I7 policy (clear the history, count as saved). Lane S2s owns the policy for a host load mid-session (REAPER's undo loads state) and may change it.
6. **Delivery classification is a pure function**, `apply::route`, separate from sending. It needs the patch before the op (depth from or to 0), and a transaction with any recompile is one plan.
7. **`SetSlot` recompiles** (the slot table is compiled into each plan) and sets at most one host value.
8. **Telemetry goes through a trait**, `TelemetrySource`, which `enth_engine::Tap` implements. kittest and the dev window can fake it, because a real `Tap`'s values can only be written by an engine. The session keeps one `TelemetryMap` per recently sent epoch, and the reader hides values whose epoch has no map.
9. **Engine-facing bodies are stubs** (`activate`, the active `pump`, `edit`, `undo`, `redo`, audition). They answer with typed errors that name the lane that fills them in.
10. **`Session::with_config` raises the return ring to `MAX_PLANS_IN_FLIGHT`**, so "nobody needs to drain it before the next entry" holds for every configuration.
11. **The patch wins in the engine too** (F1b review). A plan carries a mask of slots whose remembered host value it forgets (`Plan::forget_host_slots`): every slot after a load, the written slots after a recompiling edit. Pushing up to 72 host values instead was rejected (nih-plug #92), and so was loading presets only through host state (`set_state` is banned).
12. **Entries that leave a compile pending pump again; loads and saves are entries** (F1b review). A host load sends its plan at once instead of waiting for the editor.
13. **A patch field that does not load is kept and saved back** until the first edit (F1b review): opening a newer project in an older build must not destroy it on the next host save.
14. **The telemetry reader follows the session's source** (F1b review), because a re-activation with the editor open builds a new tap and restarts the epochs.

## What the session relies on from the engine (lane E+, merged at `6e505b8`)

- `Link::{send_plan, send_param, send_depth, send_macro, send_mod_wheel, send_event, collect, epoch, in_flight, free_slots}`, `MAX_PLANS_IN_FLIGHT = 2`, `SendError::{Full, InFlight}` returning the value. An event sent with `send_event` waits in the ring while the engine's event queue is full, so it is never dropped (F1b review).
- `Plan::forget_host_slots(mask)` (F1b review): the slots whose remembered host value a plan forgets when it swaps in.
- `PlanMap::{get, depth, addr}` and `DepthId`; `PlanMap::depth` is `None` for a depth-0 route or a route from or to a disabled module, and the session recompiles then.
- `Plan::{tap_params, tap_modulators}` for `TelemetryMap::from_plan`; `Tap::{new, read, frame, scope, generation, sleep, set_enabled, take_wake}`; `Engine::{with_link, attach_tap, tap, epoch, plan, set_monitor_bypass}`. `TapFrame`, `EngineStats` and `NoteEnd` are `non_exhaustive` (F1b review), so they can grow after the freeze. Every engine starts its epochs at 0.
- The slot table's equality rule and its "a value event updates the slot's patch value" behaviour (the host's echo of an edit is exact). The engine forgets a remembered host slot value when the slot's key changes.
- `EventKind::{Slot, MacroVoice}` and `Engine::reset` for the shell's `process()` and `reset`; `NoteEnd::offset` for `NOTE_END`.
- The monitor bypass switches immediately on a fresh engine, so `activate` sets the persisted value before returning the engine.

## F1b review decisions (2026-10-19)

The judge's answers to S1s's six open questions. Done means implemented in this crate, `enth-engine` or `enth-patch` at the freeze.

1. **A compile still pending when nobody calls the session.** No fork patch: nice-plug's `execute_background` already runs a task on its background worker, never inline, for CLAP and VST3. The shell arms an atomic flag after unlocking with `compile_pending` set, and `process()` schedules one pump task at most every 50 ms while the flag is set ("Plan delivery" above). Loads and saves pump too (done). nice-plug's `execute_gui` is CLAP `request_callback` underneath, but it runs the task inline when the host calls `process()` on the main thread, so it stays banned in `process()`.
2. **Stale host slot values after a preset load.** Done: `Plan::forget_host_slots(mask)` in the engine, `Port::forget_slots` in the session (all bits on a load; `bridge::slots_written` for a recompiling edit, undo or redo). This covers host state loads too, since nice-plug 0.4.2 keeps the engine across them. Residual: a host value that was stale when saved (after an editor preset load) is re-sent by `process()` if the restored value differs from the shell's last-seen one, and the engine then follows it while the patch keeps its value. The shell may re-baseline its last-seen values when it drains the load's `RenameParams` (PL decides; harmless when saved values agree with the patch, which folding before each save ensures).
3. **`ParamSetter` from the GUI thread for VST3.** Keep it. nice-plug calls `IComponentHandler` on the calling thread, like every nih-plug egui plugin on Linux. PL verifies write and touch modes on VST3 in REAPER with RH's `slot-automation` scenario. On Linux, nice-plug's VST3 event loop is not the host's UI thread either, so marshalling would need a fork patch using the host's `IRunLoop`. Only do that if REAPER misbehaves.
4. **Two slots on one address.** Allowed in the patch, handled deterministically, avoided by the editor ("The host bridge" above). `SetSlot` does not refuse duplicates: a transaction that moves an assignment passes through one, and existing files would turn invalid.
5. **`modulation_range(patch, addr)`** goes in `enth-patch` as a pure function: the clamped normalised span of `to_norm(base) + Σ depth · source range` over the address's routes, using each source's polarity. The CLI's `describe` and the editor's static rings share it. ED2 adds it additively. It is not part of the frozen session API.
6. **History merging for drags.** Done in `enth-patch::History`: a `SetDepth` that follows a `SetDepth` on the same route, and a `SetMacro` that follows a `SetMacro` on the same macro, merge like `SetParam` on one address. Each keeps the first inverse and takes the last forward, so a depth or macro drag is one op pair. Tested by the snapshot-model property and a unit test.

Left open after F1b (for the lanes named):

- **ED2, engine (fix soon):** the tap's layout can tear across a plan swap. Values are written before the epoch, and every access is `Relaxed`, so for one frame a reader may pair an old epoch with the new layout. A seqlock on the epoch would fix it, with no API change. The tap's quiet rule also stops publishing global-modulator outputs, macros and a new epoch once a ring of silence has passed, and an editor opened during silence gets nothing until a note plays. ED2 decides what keeps the tap publishing.
- **PL:** `Engine::reset` reports no `NOTE_END`. Emit them for `active_voices()` before calling it, for hosts that track voices. In mono and legato, a return to a held key keeps the released note's poly-mod offsets, because the engine keeps no per-note offsets. Bitwig decides whether that matters.
- **SK:** prune `telemetry_maps` to the last `MAX_PLANS_IN_FLIGHT + 1` epochs. Treat a stale `GestureId` as no gesture (`History::clear` restarts the ids). Clear `unloaded_patch` in `edit`. Push a `SetSlot`'s host write only when its plan went out.
- **S2s:** fold against the plan's value, not the current patch, to close the one-rounding-step gap ("The host bridge" above). Also the host-load history policy.
- An `Activation`'s `offline` flag is fixed at activate. If a host switches to offline rendering without re-activating, audition notes are not refused. REAPER re-activates, so add a setter only if another host needs one.
