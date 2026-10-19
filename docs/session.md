# The session (`enth-session`, L6)

**One single-threaded `Session` owns the live patch.** It holds the `Patch`, its `History`, the engine's `Link`, the `PlanMap` of the latest plan it sent, and the per-instance state outside the patch. The editor and the plugin shell change the patch only through it. It does no I/O, spawns no threads, reads no clocks and takes no locks; the plugin wraps it in one mutex. Lane S1s wrote this page and the crate's public API (freeze point F1b, [phase-1b.md](phase-1b.md)); lanes SK and S2s fill in the bodies that talk to the engine. Sources: research report 01 (R2, R3, R7, R8) and 02 (D5, D7) in `_local/sessions/2026-10-19-p1b-plugin-editor/`, [design.md §3–§4](design.md#4-host-parameters-macros--automation-slots), and the engine seams in [dsp/engine.md](dsp/engine.md#p1b-seams-lane-e-what-the-session-plugin-and-editor-lanes-rely-on).

## The API in brief

| Item | Module | What it is | State (F1b) |
| --- | --- | --- | --- |
| `Session` | `model` | The owner: `new`, `patch`, `history`, `instance`, `status` (with `patch_unloaded`), `drain_requests`, `set_ui`, `set_monitor_bypass`, telemetry accessors | Implemented |
| `SessionView` | `model` | The editor's read model: patch, history, status, instance state, telemetry map and source | Implemented for `Session`; fakes can implement it |
| `SessionEdit` | `model` | The editor's edit trait: gestures, `edit(ops, gesture)`, undo, redo, UI state, monitor bypass, audition | Gestures, `undo` and `redo` implemented (S2s); `edit` and audition are SK's stubs |
| `ShellRequest` | `model` | The outbox the shell drains: host gesture writes, renames, the monitor bypass, "patch edited" | Implemented |
| `SessionError` | `model` | `Edit`, `Compile`, `State`, `Inactive`, `Busy`, `Offline`, `Unsupported`; each with `code()` and `hint()` | Implemented |
| `Routing`, `Delivery`, `route`, `delivery` | `apply` | How an op reaches the engine (the table below) | Implemented and tested per op kind |
| `HostParam`, `HostValues`, `HOST_PARAM_IDS` | `bridge` | The 72 host parameters with permanent ids (`macro_1`…`macro_8`, `slot_1`…`slot_64`) | Implemented |
| `fold_value`, `fold_against`, `Session::fold_host`, `host_values`, `host_updates`, `slots_written`, `host_param_name(s)`, `host_layout_changed` | `bridge` | The patch-wins rule (folded against the plan's value), host writes, the slots a recompile must forget, names and when a rescan is due | Implemented and tested |
| `InstanceState`, `SavedState`, `LoadReport`, `HistoryFate`, `StateError` | `state` | The saved state v1: the `patch` field and the `enth` field (`enth.plugin-state/1`); what a load did to the history | Implemented, fixtures under `tests/fixtures/state/v1/` (and the synthetic predecessor under `v0/`) |
| `Session::{save, save_patch, save_instance, load, load_patch, load_instance}` | `state` | Save and load; a patch that does not load is kept as text and saved back unchanged until the first edit | Implemented; the host-load history policy is "reconcile" ([below](#host-state-loads-and-the-history)) |
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

A transaction with any `Recompile` op is **one** plan compiled from the patch after it, and its value events are skipped (the plan carries them); the plan forgets the remembered host values of the slots its `SetParam`s write (`bridge::slots_written` → `Plan::forget_host_slots`), so the engine plays the edited value instead of re-applying the old host value until the host's echo arrives. Undo and redo deliver their ops the same way, through the same function as an edit (`Session::deliver`, [below](#undo-and-redo-and-the-engine)). Editor edits also produce host writes (`bridge::host_updates`): a `SetParam` on a slotted address sets each of its slots to `to_norm` of the new value, a `SetMacro` sets its macro, a `SetSlot` sets the new target's value. In a gesture the session sends `Begin` at the first write to a parameter and `End` for each at `end_gesture`; outside a gesture it sends begin, set and end at once.

### Invariants

Each gets a test in the lane that implements it (S2s's acceptance lists them).

- **I1, one writer.** Every patch change is an `Op` applied by the session. Editor edits, undo and redo are recorded; host folds and loads are not.
- **I2, delivery follows the op**, after the transaction succeeded. A rejected transaction changes nothing and delivers nothing.
- **I3, lazy plans.** At most one compile is pending, compiled from the patch at the moment it can be sent, so a plan always reflects the patch at the time it was sent.
- **I4, epochs.** Value and depth events carry the epoch of the latest *sent* plan and use its `PlanMap`.
- **I5, never blocks, never drops.** `SendError::InFlight` keeps the compile pending. `SendError::Full` queues the value (the last per parameter, at most `SessionConfig::value_queue`); an overflowing queue is cleared and a compile marked pending instead. Audition note-offs are never dropped: the session queues them, and the engine leaves an event from the ring in the ring while its event queue is full (F1b review); a note-on that does not fit the session's queue is refused with `Busy`.
- **I6, the patch wins** (next section).
- **I7, load.** The patch wins, in the session and in the engine: host values restored with the state are not folded during the load, and the loaded patch's plan forgets the engine's remembered host slot values. The history follows the host-load policy ([below](#host-state-loads-and-the-history): kept and moved when the loaded patch is a state it holds, else cleared) and the load counts as saved; names are refreshed; while active the plan is sent at once (or as soon as the link has room). A field that does not load gives the init patch (or the default instance state), a typed error in the report and in `last_error`; it never panics. The text of a `patch` field that did not load, and of an `enth` field from a newer build, is kept and saved back unchanged until the first edit (the first change of the preferences, for `enth`), so a project from a newer enth survives being opened and saved in an older build.
- **I8, save.** Pump, fold the host's values, then write `Patch::to_json()` (or the kept text, above). A host project save does not touch the history or the dirty flag.
- **I9, undo and redo** are recorded steps of the history that deliver through I2 like an edit; a structural undo is exactly one plan, a drag is one step, and undo ends an open drag first. Nothing to undo is `Ok(false)` and delivers nothing; a step that no longer applies is an error that changes nothing.
- **I10, audition notes** use `AUDITION_PORT` (`0x7F00`) and a fresh note id each, go through `Link::send_event`, are refused while the host renders offline, and are released on editor close, deactivate and load.

## The host bridge: the patch wins

The host sees 72 linear 0–1 parameters with permanent ids. Plain → normalised → plain is not a round trip (33–82 % of values move on Log, Power and signed parameters), so three rules decide when a host value enters the patch (`Session::fold_host`, called once per editor frame and before every save):

1. **The patch wins at load.** The first fold after construction or a load only records the host's values.
2. **A host value is folded only when it changed since the session last saw it.** A stale host value, after a slot reassignment that never reached the host, an editor preset load, or a host that restored no parameters, never overwrites the patch.
3. **And only when it truly differs** (`bridge::fold_value`): if the host's `n` equals `to_norm(p)` bit for bit the patch keeps `p`; otherwise the candidate is the canonical `from_norm(n)`, written (without history) only if it differs from `p` bit for bit. Folding the same value again, or after a save and reload, never moves the patch. Non-finite host values are ignored.

The engine resolves slot events with the same equality rule, and an editor edit updates the slot table's patch value, so the host's echo of an edit (`to_norm` of the same value, the same pinned function) lands on the exact plain value. A fold sends nothing to the engine: the engine already follows the host through slot and macro events from `process()`. Both sides canonicalise the same way (`from_norm` then the periodic wrap). The engine compares with the value its plan was compiled with (or the last editor edit), which earlier folds did not move in the engine; so the fold compares with that value too (`bridge::fold_against`, F1b review issue 4, closed by S2s). The session remembers, per address a fold moved, the value the engine still holds, together with the epoch of the plan that holds it; the memo ends when a plan is sent (the new plan is compiled from the patch) or when the patch's value changes any other way (an edit, undo or redo sent the engine an event). Every fold sweeps the memos, so they cannot outlive their cause; detached there is no engine and the patch's own value is the reference. A host that returns exactly to `to_norm` of the compiled value therefore leaves the patch on that exact value, the one the engine plays, and the two never differ by a rounding step. It needs no hook in the edit path: the session reads `Link::epoch()` and compares values.

Two slots may drive one address. The patch allows it (a transaction that moves an assignment passes through a duplicate, and hand-written files may hold one) and every rule handles it deterministically: the higher slot wins a fold, the later event wins in the engine, an edit writes every slot of the address, and `slots_written` names them all. The editor's "Automate" menu reuses the address's existing slot (`bridge::slots_for`) before taking a free one (F1b decision 4).

## Undo, redo and the engine

`Session::undo` and `redo` are steps of the `enth-patch` `History`, which stores ops only (an entry is its forward ops and its inverse). They run the same pipeline as an edit after the patch has changed, `Session::deliver(before, ops)`, where `ops` is the inverse of the undone entry or the forward ops of the redone one:

1. **Engine** (active): the `delivery` of those ops. A `Recompile` marks a plan pending and adds `slots_written` to `Port::forget_slots`; events go out in op order through the latest sent plan's `PlanMap` and epoch, once per parameter (the final value of the transaction), behind any events already waiting for room. A full command ring queues the value (the last per parameter, at most `value_queue`); a queue that would overflow is dropped and a plan is marked pending instead (I5). A depth the plan has no term for becomes a plan too. Then the session pumps, so a pending plan goes out at once unless two are in flight.
2. **Host**: each slotted parameter or macro the ops moved gets one complete begin, set, end. If the editor had a drag open, its host gestures are closed first (`end_gesture` semantics), so the pairs never interleave. A `SetSlot`'s write is made only if its plan went out in the same call.
3. **Shell**: `RenameParams` when the host would see other parameter names or kinds ([below](#parameter-names-and-rescans)), then the host writes, then `PatchEdited`.

Behaviour the editor can rely on:

- `undo`/`redo` first end an open gesture. The drag in flight becomes its (closed) step and one `undo` takes the whole of it back; a drag that ended where it started leaves no step (`History::end_gesture` annuls it), and a drag merges per `SetParam` address, `SetDepth` route and `SetMacro` index.
- A new edit clears redo, also a drag's first event, even if the drag later annuls itself.
- `Ok(false)` when there is nothing to do: nothing is delivered and nothing is requested. `Err` means something changed the patch behind the history's back so that the step no longer applies (a bug elsewhere: the only writes without history are folds, which touch values, and a load, which resets the history); the patch, the history and the engine are unchanged, the step is still there, and `last_error` says why.
- The first undo or redo after an unreadable `patch` field was replaced by the init patch clears `unloaded_patch` like the first edit does: the user now owns that patch.
- A fold is not history. A value the host's automation wrote into the patch is not undoable; an undo that restores an older value of the same parameter wins until the host's next write, which is folded again (the patch wins at load and on every unchanged host value, and the host's write is a new change).

## Host state loads and the history

A host loads state whenever it restores the project: REAPER's own Undo, opening a project, a snapshot. nice-plug 0.4.2 keeps the engine across a load, so it is `Session::load_patch` that sees it, on the host main thread. The history cannot simply stay: it is a list of ops, and an inverse op written for another patch writes old values into the wrong sound. The `enth-patch` ops also have no "replace the whole patch" step, so a load cannot be recorded as an undoable step either. The policy is **reconcile** (`HistoryFate`, reported in `LoadReport::history`):

| The loaded patch is | Fate | The history |
| --- | --- | --- |
| the patch the session holds (`to_json` equal) | `Kept` | untouched, counts as saved |
| the state `k` undo steps back, `k <= RECONCILE_STEPS` (64) | `Undone(k)` | those steps are undone in the history, **not delivered**; redo brings them back; counts as saved |
| the state `k` redo steps ahead, `k <= RECONCILE_STEPS` | `Redone(k)` | those steps are redone, not delivered; counts as saved |
| anything else, or a patch that did not load | `Cleared` | both stacks cleared, counts as saved |

States are compared by the text they would be saved as (so the engine stamp does not matter), nearest first; on a tie, back. The engine always gets the loaded patch through the load's own plan (marked to forget every remembered host slot value), whatever the fate, so "not delivered" means only that the steps do not also send events.

Why this and not clearing or recording. REAPER's Undo restores the project state the host saved at each point of *its* undo history, so after a few edits and a REAPER Undo the loaded patch is, in the usual case, exactly one step back of the session's. Clearing the history on that load would make the editor's Ctrl+Z and REAPER's Undo destroy one another (the editor loses the steps REAPER just walked over); keeping the history unmoved would leave it one step off. Reconcile makes the two walk one line. A state the history does not know (another project, a preset, a state from a different session) is a new line and starts a clean history, which is what clearing always did. The walk is bounded (64 steps each way, cloning the patch once per side) so it costs the host main thread a few milliseconds at worst.

Also on a load: an open drag is closed in the history without host writes (the host gestures it opened stay open until the editor's next `end_gesture`, so begins and ends stay balanced and are sent from the same thread); the gesture ids are not restarted when the history is kept, and SK's `edit` treats a `GestureId` that is not the open one as no gesture, so an id from before the load can never join a later drag; the host baseline is reset (the next fold only records); audition notes are released; names are refreshed.

## Parameter names and rescans

`bridge::host_param_names(patch)` gives the 72 display names in dense order: a macro's name or "Macro N"; a slot's target as the module's title and the parameter ("Osc 1 · Cutoff"; the title is the module's label, else its catalogue name plus the number its id ends with, "Filter (bass)" for an id with no number), a built-in's parameter name ("Glide", "Master level") or "Slot N" when empty. The shell sets them on a `ShellRequest::RenameParams`.

A rescan is not free (CLAP `rescan`, VST3 `restartComponent` can make a host rebuild its automation lanes), so the session asks only when the host would see something different: `bridge::host_layout_changed(before, after)`, evaluated for the ops that can change names at all (`apply::renames_params`), is true when any of the 72 names differs or a slot drives another kind of parameter (its value text and range differ even when a name does not). Labelling a module no slot drives, naming a macro twice to the same text, moving a module and adding one do not ask. Undo and redo ask exactly when the names they restore differ, and a load always asks.

## The saved state, version 1

Two nice-plug persistent fields, both permanent names ([naming.md](naming.md)):

| Field | Text | Versioned by |
| --- | --- | --- |
| `patch` | Exactly `Patch::to_json()`, carried raw (never re-parsed through `serde_json::Value`) | The patch file's `schema` number and `enth_patch::migrate`, the same path as `enth render` |
| `enth` | `{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":false},"ui":{"layout":"lanes","scale_percent":100,"advanced":false}}` | Its `schema` string and `state::INSTANCE_STEPS` |

- The slot assignments and the macros (values and names) live in the patch; the 72 host values are nice-plug's own `params` and are never needed to rebuild the sound.
- Byte-deterministic (clap-validator's `state-reproducibility-binary`); each field at most 1 MiB; the two fields load independently (nice-plug sets `enth` before `patch`).
- Within version 1, a new optional field with a default is allowed (unknown fields are ignored, missing ones default). Anything else is version 2: a step in `INSTANCE_STEPS`, `v2/` fixtures, and the `v1/` fixtures kept as migration inputs. A newer version than the build knows is refused (`schema_too_new`) and its text kept (below).
- **Two version axes, both exercised.** The `patch` field is versioned by the patch file's own `schema` and `enth_patch::migrate`; patch schema 0 is the synthetic predecessor kept to keep that machinery honest, so `tests/fixtures/state/v0/{lanes,minimal}.patch.json` (with their `.migrated.patch.json`) load through `Session::load_patch` as a saved state does, migrate, and save back as the current schema, byte for byte. The `enth` field's walk, `migrate_instance_with(value, current, steps)`, runs the same code `migrate_instance` runs with this build's (empty) `INSTANCE_STEPS`, but with a table of the test's own, so the day `STATE_VERSION` becomes 2 the hook is already proven for a chain, for a missing step (`migration_failed`, never a panic), and for a step that rejects its input.
- A `patch` field that does not load (a newer schema, damage) leaves the init patch playing, `status().patch_unloaded`, and its text, which every save writes back unchanged until the first edit, undo or redo. A field over 1 MiB is not kept. An `enth` field from a **newer** version (`schema_too_new`) is kept the same way: the defaults play, `status().instance_unloaded` is set, and every save writes the stored text back unchanged until the first real change of the monitor bypass or the editor preferences. A damaged `enth` field is not kept (it gives the defaults and is re-saved with them; only preferences are lost); unknown fields of a newer version 1 are dropped on re-save. A project saved by a newer build and saved again by this one therefore keeps both fields byte for byte (tested).
- Fixtures: `crates/enth-session/tests/fixtures/state/v1/{default,custom}.{patch,enth}.json`. `custom` carries three slots (the first, the second and the last), a named macro, a module label, a route and non-default editor preferences. Regenerate on purpose with `cargo test -p enth-session --test state -- --ignored regenerate` and read the diff. The `v0/` fixtures are inputs and are not regenerated by the v1 tests; their `.migrated` files are what the migration must produce.

## Decisions

1. **An outbox, not a host trait.** The session pushes `ShellRequest`s (host gesture writes, renames, the monitor bypass, "patch edited") that the shell drains after unlocking. Report 01 passed a `HostSync` trait into every edit instead. The outbox keeps the editor's traits free of host concepts, lets kittest assert on requests, and makes "never call the host under the lock" the natural shape.
2. **Two traits for the editor:** `SessionView` (read) and `SessionEdit: SessionView` (edits). Both are object-safe and implementable outside the crate (a test does it). Pumping and folding stay out of them: the editor glue does those, so the editor never reasons about the engine or the host.
3. **Host values are folded only when they changed since last seen, and only on a true difference.** Rule 2 is new against report 01's I6. Without it, a slot reassignment that never reached the host, or a host that restored no parameters, would overwrite the patch with a stale value.
4. **The saved state is two fields**, as architecture.md and naming.md fix them. The `enth` blob holds only what is not in the patch, and its UI scale is an integer percent, so the bytes never depend on float formatting.
5. **A host state load reconciles the history instead of clearing it** (S2s). `load_patch` keeps the history, moved to the matching state, when the loaded patch is the current one or at most 64 undo or redo steps away, and clears it otherwise (["Host state loads and the history"](#host-state-loads-and-the-history)). S1s cleared it always; the change is invisible unless the loaded patch is a state the history already holds.
6. **Delivery classification is a pure function**, `apply::route`, separate from sending. It needs the patch before the op (depth from or to 0), and a transaction with any recompile is one plan.
7. **`SetSlot` recompiles** (the slot table is compiled into each plan) and sets at most one host value.
8. **Telemetry goes through a trait**, `TelemetrySource`, which `enth_engine::Tap` implements. kittest and the dev window can fake it, because a real `Tap`'s values can only be written by an engine. The session keeps one `TelemetryMap` per recently sent epoch, and the reader hides values whose epoch has no map.
9. **Engine-facing bodies are stubs until SK** (`activate`, the active `pump`, `edit`, audition). They answer with typed errors that name the lane that fills them in. `undo` and `redo` are S2s's and implemented.
10. **`Session::with_config` raises the return ring to `MAX_PLANS_IN_FLIGHT`**, so "nobody needs to drain it before the next entry" holds for every configuration.
11. **The patch wins in the engine too** (F1b review). A plan carries a mask of slots whose remembered host value it forgets (`Plan::forget_host_slots`): every slot after a load, the written slots after a recompiling edit. Pushing up to 72 host values instead was rejected (nih-plug #92), and so was loading presets only through host state (`set_state` is banned).
12. **Entries that leave a compile pending pump again; loads and saves are entries** (F1b review). A host load sends its plan at once instead of waiting for the editor.
13. **A patch field that does not load is kept and saved back** until the first edit (F1b review): opening a newer project in an older build must not destroy it on the next host save.
14. **The telemetry reader follows the session's source** (F1b review), because a re-activation with the editor open builds a new tap and restarts the epochs.
15. **Undo and redo share `Session::deliver` with `edit`** (S2s). One function turns "the patch just changed from `before` by `ops`" into engine events or a plan, host writes and requests, so an undo cannot differ from the edit it reverses. SK's `edit` calls the same function after `History::edit`.
16. **The fold resolves against the plan's value by a memo, not by a hook** (S2s). The memo is `(address, the engine's value, the value written, the epoch)`, swept at each fold; it needs only `Link::epoch()` and a value comparison, so no code in the edit path has to know about it (["The host bridge"](#the-host-bridge-the-patch-wins)).
17. **A rescan only when the host would see a change** (S2s): ["Parameter names and rescans"](#parameter-names-and-rescans). S1s asked for every name-capable op.
18. **A newer `enth` blob is kept like a newer `patch` blob** (S2s): preferences written by a newer build survive an older build's save.
19. **The telemetry tap is a seqlock** (S2s, F1b engine issue): [dsp/engine.md](dsp/engine.md#telemetry-tap-p1b-lane-e). A reader can no longer pair one plan's epoch with another plan's layout.

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

- **ED2, engine:** the tap's layout tearing across a plan swap is fixed (S2s): the tap is a seqlock now, and a reader never pairs an epoch with another plan's layout; when the audio thread is preempted mid-publish for longer than a bounded retry, `read` returns the previous generation with no routed values and no modulators, and the reader's next refresh sees a whole frame. Still open: the tap's quiet rule stops publishing global-modulator outputs, macros and a new epoch once a ring of silence has passed, and an editor opened during silence gets nothing until a note plays. ED2 decides what keeps the tap publishing.
- **PL:** `Engine::reset` reports no `NOTE_END`. Emit them for `active_voices()` before calling it, for hosts that track voices. In mono and legato, a return to a held key keeps the released note's poly-mod offsets, because the engine keeps no per-note offsets. Bitwig decides whether that matters.
- **SK:** prune `telemetry_maps` to the last `MAX_PLANS_IN_FLIGHT + 1` epochs. Treat a stale `GestureId` as no gesture (`History::clear` restarts the ids). Clear `unloaded_patch` in `edit`. Push a `SetSlot`'s host write only when its plan went out.
- **S2s (done):** fold against the plan's value ("The host bridge" above); the host-load history policy ("Host state loads and the history"); state versions and migrations; slot and macro names and rescans; the telemetry seqlock. Left for SK to wire: see "What lane SK must know" below.
- An `Activation`'s `offline` flag is fixed at activate. If a host switches to offline rendering without re-activating, audition notes are not refused. REAPER re-activates, so add a setter only if another host needs one.

## What lane SK must know

S2s was written against the S1s stubs, so the engine-facing glue was built and tested with a hand-made active session (`src/tests/kit.rs`: a real `Engine::with_link`, a `Port` built by hand, and small doubles for `edit` and the pump's plan send). Public-API tests of the same ground are in `crates/enth-session/tests/engine.rs`; they are `#[ignore = "needs lane SK"]`, and a tripwire test there fails the day `activate` stops answering `Unsupported`.

SK's branch (`p1b/sk`, checked at `146d4cf`) implements `activate`, `pump`, `edit` and the audition calls, and also wrote its own `Session::deliver` (a different signature and a narrower job than S2s's). The two branches merge textually with one conflict (`tests/api.rs`, the stub test) and then do not compile (two `deliver`s). A trial merge, resolved as below, passes every test of both lanes (S2s lib 38, api 12, bridge 19, engine 8 with the ignores removed, state 22; SK link 20):

1. **One `deliver`: S2s's.** `Session::deliver(&mut self, before: &Patch, ops: &[Op], in_gesture: bool) -> Applied` is the shared tail of an edit, an undo and a redo. Delete SK's `deliver(routing, ops, in_gesture)` and rewrite `edit` to: `pump()`; treat a `GestureId` that is not `self.gestures.open` as `None`; return `Applied::default()` for no ops; `let before = self.patch.clone()`; `self.history.edit(&mut self.patch, ops.clone(), gesture)` (mapping its error to `SessionError::Edit`); clear `unloaded_patch` and `last_error`; `self.deliver(&before, &ops, gesture.is_some())`. S2s's `deliver` has everything SK's has (the host gesture bookkeeping is SK's logic, moved: a `Begin` at the first write inside a drag, a `Set` inside a host gesture a load left open, `PatchEdited` once) and adds: the events once per parameter, the queue helpers (`send_or_queue`; `Queued::is_value`/`same_target` can then lose their duplicates, and SK's `Port::send_value` has no caller), `forget_slots = u64::MAX` when the queue overflows into a plan, and a rescan only when the host would see other names or kinds.
2. **`activate` must `self.host.memos.clear()`** (S2s's fold memos are keyed by `Link::epoch()`, and a new engine starts at epoch 0 again; `tests/engine.rs::a_reactivation_forgets_what_the_fold_remembered_about_the_old_engine` fails by exactly one rounding step without the line). Leave `host.seen` alone.
3. **`tests/api.rs`**: keep SK's activate half of the stub test and S2s's `undo`/`redo` assertions (`Ok(false)` with nothing recorded); the `Unsupported` value the loop at the end uses can be any.
4. **`src/tests/kit.rs`**: add SK's `auditioned: false` to the hand-built `Port`.
5. **`tests/engine.rs`**: remove the `#[ignore]` attributes and delete `lane_sk_has_not_landed_yet`.

Behaviour that changed against S1s's text, for the editor and shell lanes: `ShellRequest::RenameParams` is pushed only when the host would see other names or kinds (so the shell may rescan on every one it receives), and always on a load; `Applied::renamed` follows it. `SessionStatus::instance_unloaded`, `LoadReport::history: Option<HistoryFate>` and `RECONCILE_STEPS` are additive.

Other notes for SK, PL and ED:

- **`pump`** must, when it sends a plan: set `port.map`, clear `compile_pending` and `forget_slots`, drop the queued value entries (the plan carries every value; audition entries stay) and push the new `TelemetryMap`. SK's does.
- **Do not hold the telemetry reader across a seqlock failure** (PL, ED2): `Tap::read` can return a frame with zero routed values and modulators and the *previous* generation if the audio thread is preempted mid-publish for longer than its retry budget. `TelemetryReader::refresh` handles it (it reports no change); a caller reading `Tap` directly must too.

## Tests (lane S2s)

| What | Where |
| --- | --- |
| Undo and redo round trips for every op kind, detached and active; gestures (a drag is one step, an annulled drag leaves none, depth and macro merging); a proptest of the history against a snapshot model, with a drag open or not; a refused step; host writes and rescans from undo | `src/tests/history.rs` |
| The delivery table per op kind against a live engine; epochs end to end; one plan per structural step; the bounded queue behind a full ring and its overflow into a plan; order behind queued events | `src/tests/deliver.rs` |
| The host-load policy: kept, undone, redone, nearest wins, bounded, new line; an open drag; the engine gets the loaded patch whatever the fate | `src/tests/host_load.rs` |
| The fold against the plan's value, with a proptest that patch and engine end on the same bits; memos end with their plan | `src/tests/fold.rs` |
| Names and rescans (`tests/bridge.rs`); versions, the v0 patch fixtures, the instance migration walk, a newer blob kept and saved back, and fuzz-lite (mutated and truncated blobs never panic) | `tests/bridge.rs`, `tests/state.rs`, `tests/fixtures/state/v0/` |
| The public-API session against a real engine: undo and redo reach it, a full ring never blocks or drops, the audio thread keeps rendering while the main thread edits, epochs and fold against the compiled value end to end | `tests/engine.rs` (ignored until SK) |
| The telemetry seqlock: a reader never pairs an epoch with another plan's layout (a stress test, with a negative control showing the old protocol tears) | `crates/enth-engine/src/tap.rs`, `crates/enth-engine/tests/tap.rs` |
