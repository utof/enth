# The session (`enth-session`, L6)

**One single-threaded `Session` owns the live patch.** It holds the `Patch`, its `History`, the engine's `Link`, the `PlanMap` of the latest plan it sent, and the per-instance state outside the patch. The editor and the plugin shell change the patch only through it. It does no I/O, spawns no threads, reads no clocks and takes no locks; the plugin wraps it in one mutex. Lane S1s wrote this page and the crate's public API (freeze point F1b, [phase-1b.md](phase-1b.md)); lanes SK and S2s fill in the bodies that talk to the engine. Sources: research report 01 (R2, R3, R7, R8) and 02 (D5, D7) in `_local/sessions/2026-10-19-p1b-plugin-editor/`, [design.md §3–§4](design.md#4-host-parameters-macros--automation-slots), and the engine seams in [dsp/engine.md](dsp/engine.md#p1b-seams-lane-e-what-the-session-plugin-and-editor-lanes-rely-on). Lane SG (R1, freeze point F1r, [phase-2.md](phase-2.md)) added the Recreate round: its API, threading contract and saved state are in ["The Recreate round"](#the-recreate-round-lane-sg-r1). Lane SL (R2, freeze point F1l, [phase-3.md](phase-3.md)) added the Learn binding (the stash, step loads, locks in the edit path, lesson rounds, the `learn` object): ["Learn"](#learn-lane-sl-r2).

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
| `Session::{load_preset, save_preset}`, `SessionEdit::{load_preset, save_preset}` | `presets` | The editor's preset load (one undo step from `enth_patch::ops_between`, delivered like a host load with no host writes, `PatchEdited` marks the project dirty) and save (a user preset, `Patch::to_json()`); the index, search, tags and favorites are in the same module ([presets.md](presets.md)) | Implemented (PBC) |
| `Session::{activate, deactivate, pump}`, `Activation`, `PumpReport` | `link` | The engine side | `deactivate` implemented; `activate` and the active `pump` are SK's stubs |
| `TelemetrySource`, `TelemetryFrame`, `TelemetryMap`, `TelemetryReader` | `telemetry` | The tap's reader side; `TelemetryFrame` is the engine's `TapFrame` (`non_exhaustive`: fakes build it from `Default`), and `Tap` implements `TelemetrySource`; the reader follows a re-activation (`set_enabled`, `refresh`) | Implemented and tested against a real engine |
| `GameEdit`, `RoundView`, `RoundSource`, `Listen`, `EndChoice` | `game` | The editor's round intents (start from settings or a code, A/B, submit, hint, reveal, vote with CT's `Agreement`, dispute, end) and its read model; `Session` implements the trait, a kittest fake can | Lane SG (R1, freeze point F1r); [below](#the-recreate-round-lane-sg-r1) |
| `Session::{game_tick, game_job_done, drain_game_requests, game_ab, game_record}`, `Job`, `JobKind`, `JobResult`, `GameRequest`, `EditorView` | `game` | The shell's side of a round: the injected clock, the jobs its worker runs, the round's outbox | Lane SG |
| `GameState`, `RoundRecord`, `RoundLogEntry` | `game` | The `enth` field's `game` object (absent while no round is live) and the round log's line (`enth.round-log/1`, carrying CT's `enth.round-result/1`) | Lane SG; fixture `tests/fixtures/state/v1/round.*.json` |
| `LearnEdit`, `LearnView`, `Session::{learn_job_done, drain_learn_requests, learn_record, learn_player, locks, checked_patch}`, `LearnJob`, `LearnResult`, `LearnRequest`, `LearnError` | `learn` | The editor's lesson intents and read model, and the shell's side of a lesson: jobs, progress records, the Listen state | Lane SL (R2, freeze point F1l); [below](#learn-lane-sl-r2) |
| `LearnState`, `LessonRef`, `Selector`, `SessionError::{Locked, LessonLive}`, `RoundSource::Challenge`, `JobKind::PrepareChallenge`, `RoundOrigin` | `learn`, `model`, `game` | The `enth` field's `learn` object, the lock selectors and refusals, and lesson rounds | Lane SL; the selector and `LessonRef` are stand-ins for lanes CV2's and LM's types until they merge |

Stubs return `SessionError::Unsupported { what, lane }` (or `Inactive` while detached), never panic, and name the lane that fills them in.

## Threading contract

### Who calls what, on which thread

| Thread | Calls into the session | Never |
| --- | --- | --- |
| **Host main thread** | nice-plug `initialize`/`activate`: `fold_host`, `activate` (returns the `Engine`); `deactivate`; state save through the persistent fields' `map`: `save_patch(Some(&host))`, `save_instance()` (it carries the round's record); state load through the fields' `set`: `load_instance` (it reconciles the round: [below](#host-loads-during-a-round)), `load_patch`; then `drain_requests` and `drain_game_requests` | Call into the host while holding the lock |
| **baseview's GUI thread** (the editor; a separate thread in baseview 0.4.1) | Once per frame, inside egui's update closure only (never across painting or the buffer swap): `pump`, `fold_host(&host)`, `game_tick(now, view)`, then `game_job_done` for each job result the game worker posted, then the editor's reads and edits through `SessionView`/`SessionEdit` and `GameEdit`, then `drain_requests` and `drain_game_requests` into local buffers; **unlock**; then forward `ShellRequest::Host` writes to `ParamSetter`, jobs to the game worker, A/B to its atomics and round-log lines to the log writer | `GuiContext::set_state` (see below); keep the lock while calling `ParamSetter` |
| **nice-plug's background worker** (`bg-worker`) | The shell's "pump wanted" task: lock, `pump`, unlock (see "Plan delivery") | Anything but `pump` (it makes no `ShellRequest`, so there is nothing to forward) |
| **The shell's game worker** (lane SKR's thread) | **Nothing.** It runs the round's `Job`s from their own payloads (owned patches, shared `Arc`s), keeps the target's loop buffer for the audio thread, and posts each `JobResult` to a queue the GUI thread drains | Lock the session; call `game_job_done` itself (a prepared target loads the start patch, whose requests must be forwarded from the GUI thread) |
| **Audio thread** (`process`, CLAP `reset`) | **Nothing.** It owns the `Engine`, a preallocated event buffer and the last host values it saw. The one thing it schedules is the background pump task, rate-limited. In a round it also plays the target's loop buffer (handed over like a plan, through a ring) and reads the A/B atomics | Lock the session; touch it; schedule GUI work (`execute_gui`) |

- **The mutex.** The plugin holds `parking_lot::Mutex<Session>` in its shared state, used by the host main thread, the GUI thread and nice-plug's background worker (the pump task), never the audio thread. It is the only lock, so there is no lock order; nobody holding it waits on another thread or calls the host, so no thread can deadlock on it. A hold is at most one editor frame's edits, with at most one plan compile (about 0.9 ms for the G1 patch, mostly the reverb's preparation; [design.md §3](design.md#3-engine-and-real-time-safety)). The session itself takes no lock and is `Send` (a test checks).
- **Never call into the host or nice-plug while holding the lock.** A host may call back synchronously on the same thread (REAPER is reported, anecdotally, to read the state for an undo point when a parameter changes), and the persistent field's `map` would then lock the session again and deadlock. So the shell drains `ShellRequest`s into a local buffer, unlocks, and only then acts on them. This matters more once baseview PR #357 lands and the GUI thread becomes the main thread; the mutex stays correct either way.
- **The audio thread's side.** `process()` sends host automation as `EventKind::Slot { slot, value }` (the modulated normalised value, so CLAP mono modulation follows), macros as `EventKind::Macro`, CLAP poly-mod as `EventKind::MacroVoice { key: NoteKey::note_id(id), .. }`, never a bare `EventKind::Param` (a plan swap inside the block would misroute it). It sends only values that changed since its last block; at activate the shell sets its "last seen" values to the current host values, so nothing is echoed. CLAP `reset` is `Engine::reset()` there and nothing else. After `process` it takes the tap's wake flag (`engine.tap()` → `Tap::take_wake`) and asks for a repaint, which is a lock-free call.
- **A host state load keeps the engine.** nice-plug 0.4.2's state load sets the 72 parameters, then the persistent fields (`enth`, then `patch`), and does **not** re-run `initialize` (unlike nih-plug), so the shell does not re-activate: `load_patch` pumps the loaded patch's plan out at once and marks it to forget the engine's remembered host slot values (`Plan::forget_host_slots(u64::MAX)`), so the loaded patch wins in the engine too. A fold the GUI makes between the parameters and the fields lands in the old patch and is replaced; the first fold after the field set only records.
- **The `set_state` pitfall.** Never call `GuiContext::set_state`, from the editor or anywhere: while the plugin is processing, nice-plug applies that state **on the audio thread**, where the field's `set` would lock the session and compile a plan. An editor preset load calls `Session::load_preset` ([presets.md](presets.md)) directly, on the GUI thread, under the lock: one undo step, delivered like a host load, no host writes. A clippy `disallowed-methods` entry in `enth-plugin` bans it, along with `execute_gui` from `process()`.
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

A transaction with any `Recompile` op is **one** plan compiled from the patch after it, and its value events are skipped (the plan carries them); the plan forgets the remembered host values of the slots its `SetParam`s write (`bridge::slots_written` → `Plan::forget_host_slots`), so the engine plays the edited value instead of re-applying the old host value until the host's echo arrives. Undo and redo deliver their ops the same way, through the same function as an edit (`Session::deliver`, [below](#undo-redo-and-the-engine)). Editor edits also produce host writes (`bridge::host_updates`): a `SetParam` on a slotted address sets each of its slots to `to_norm` of the new value, a `SetMacro` sets its macro, a `SetSlot` sets the new target's value. In a gesture the session sends `Begin` at the first write to a parameter and `End` for each at `end_gesture`; outside a gesture it sends begin, set and end at once.

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
| `enth` | `{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":false},"ui":{"layout":"lanes","scale_percent":100,"advanced":false}}`, plus `"game":{"round":{…}}` while a Recreate round is live ([below](#what-a-round-saves)) and PBC's `preset`, and SL's `learn` while a lesson is live ([below](#what-a-lesson-saves)) | Its `schema` string and `state::INSTANCE_STEPS` |

- The slot assignments and the macros (values and names) live in the patch; the 72 host values are nice-plug's own `params` and are never needed to rebuild the sound.
- Byte-deterministic (clap-validator's `state-reproducibility-binary`); each field at most 1 MiB; the two fields load independently (nice-plug sets `enth` before `patch`).
- Within version 1, a new optional field with a default is allowed (unknown fields are ignored, missing ones default). Anything else is version 2: a step in `INSTANCE_STEPS`, `v2/` fixtures, and the `v1/` fixtures kept as migration inputs. A newer version than the build knows is refused (`schema_too_new`) and its text kept (below).
- **Two version axes, both exercised.** The `patch` field is versioned by the patch file's own `schema` and `enth_patch::migrate`; patch schema 0 is the synthetic predecessor kept to keep that machinery honest, so `tests/fixtures/state/v0/{lanes,minimal}.patch.json` (with their `.migrated.patch.json`) load through `Session::load_patch` as a saved state does, migrate, and save back as the current schema, byte for byte. The `enth` field's walk, `migrate_instance_with(value, current, steps)`, runs the same code `migrate_instance` runs with this build's (empty) `INSTANCE_STEPS`, but with a table of the test's own, so the day `STATE_VERSION` becomes 2 the hook is already proven for a chain, for a missing step (`migration_failed`, never a panic), and for a step that rejects its input.
- A `patch` field that does not load (a newer schema, damage) leaves the init patch playing, `status().patch_unloaded`, and its text, which every save writes back unchanged until the first edit, undo or redo. A field over 1 MiB is not kept. An `enth` field from a **newer** version (`schema_too_new`) is kept the same way: the defaults play, `status().instance_unloaded` is set, and every save writes the stored text back unchanged until the first real change of the monitor bypass or the editor preferences. A damaged `enth` field is not kept (it gives the defaults and is re-saved with them; only preferences are lost); unknown fields of a newer version 1 are dropped on re-save. A project saved by a newer build and saved again by this one therefore keeps both fields byte for byte (tested).
- Fixtures: `crates/enth-session/tests/fixtures/state/v1/{default,custom}.{patch,enth}.json`. `custom` carries three slots (the first, the second and the last), a named macro, a module label, a route and non-default editor preferences. Regenerate on purpose with `cargo test -p enth-session --test state -- --ignored regenerate` and read the diff. The `v0/` fixtures are inputs and are not regenerated by the v1 tests; their `.migrated` files are what the migration must produce.

## The Recreate round (lane SG, R1)

`enth-session`'s `game` module is the pure Recreate round ([design.md §9](design.md#the-round-ab-and-feedback-2026-10-20), [phase-2.md](phase-2.md)). It keeps the session's rules: no clocks, no threads, no I/O, no locks. **Time comes from timestamps the shell passes in**, and **background work (generating and rendering the target, scoring, hints) runs as jobs the shell executes** on its own worker and hands back. The audio thread never touches any of it. It runs on lane CT's types ([challenge.md](challenge.md)): `RandomSpec`, `ChallengeCode`, `TargetFingerprint`, `Challenge`, `PreparedRound`, `ScoreReport`, `StructureScore`, `Verdict`, `Agreement`, `Outcome`, `AssistsUsed` and `RoundResult`. The F1r review's fixes (lane F1R) are folded in below.

### The lifecycle

| Phase | Entered by | The patch | Leaves by |
| --- | --- | --- | --- |
| (no round) | — | the player's own | `game_start(RoundSource::Settings(spec) \| RoundSource::Code(code))` |
| `Preparing` | `game_start`: a `Prepare` job is asked for (refused settings change nothing) | unchanged (the player can still edit it) | `Prepared`: the code is checked against the target, the player's patch and undo history are stashed, the start patch loads |
| `Playing` | the target is ready | the start, then the player's edits | `game_submit` and its `Scored` result |
| `Submitted` | a scored submit that did not pass | the player's | a passing submit |
| `Passed` | the first passing submit, which sets the time to pass | the player's | — (keep tweaking, vote, end) |
| `Revealed` | `game_reveal`: gives up, assisted, no more submits | the target | — (vote, end) |
| `Broken` | the target could not be prepared or regenerated (a failed job, a code that does not decode, `target_mismatch`, another saved hash, a challenge for other settings), or the prepare job answered `JobResult::SilentTarget` (`GameError::SilentTarget`, `silent_target`: nobody can hear the target; R1/S1 lane INT) | unchanged | `game_end` only; the stash is kept. A round that broke before its target existed was never played, so ending it or starting another logs nothing |

From any phase, `game_end(Restore)` brings back the stashed patch and its undo history, and `game_end(Keep)` keeps the round's patch and history. `game_start` from a live round is a new round: the old one is logged and the **original** stash carries over, so "your patch" is always the one from before the first round. Only sound decides a pass (lane ST's combined score is reported beside it).

### Codes and settings

A round keeps **the text of the code it started from**, so a long code keeps its maker's engine version (challenge.md, "How a round uses these types"). From the settings page (`RoundSource::Settings`), the spec is validated first (`out_of_range`, `no_generator`, `code_too_new`; a refused spec changes nothing), and a preset's spec has its short code at once. A custom spec's code is the long form, which carries the target's fingerprint prefix, so the session makes it with `Challenge::code(fingerprint.prefix())` when the target exists. A pasted code (`RoundSource::Code`) is decoded by the editor first (`ChallengeCode::decode`). Settings with a seed from 2⁵³ are refused (`out_of_range`), and so is a template this build does not have (`code_too_new`). When the target arrives, the session checks it itself, **by its fingerprint** (`TargetFingerprint`: SHA-256 of what the generator decided, by name; it never holds the patch schema, so a schema bump breaks no code and no saved round): the challenge must be for the round's spec, `ChallengeCode::verify` refuses a long code whose fingerprint prefix differs (`target_mismatch`, the round breaks before any patch loads) and reports another engine version as a warning (`engine_differs`, shown in `RoundView::warnings`; the round plays), and a resumed round's target must match the full fingerprint it saved (`target_drift`: a short code carries no prefix). The challenge comes back from the prepare job, because its phrase is the generator's pick (`Challenge::recreate(spec, phrase)`).

### Who calls what

| Entry | Thread | Does |
| --- | --- | --- |
| `game_tick(now_ms, EditorView)` | GUI, once per frame after `pump` and `fold_host` | The clock and the settled jobs. `Opened` at every `build()` (the time since the last tick never counts, even if a close went unreported), `Visible` each frame, `Hidden` when the editor closes |
| `GameEdit::{game_start, game_listen, game_submit, game_hint, game_reveal, game_dim_unused, game_rate, game_vote, game_dispute, game_end}`, `GameEdit::round` | GUI, at the end of the frame like `SessionEdit` | The editor's intents and its read model (`RoundView`, rebuilt every frame: phase, code, difficulty, timer, scores, rings, A/B, hints, the dim assist, the blind rating, the vote, the randomized `params`, `wake_at_ms`, code warnings, the last error; a kittest fake builds one from `Default`). Lane RUI added `rings` beside `knobs` (additive): every ring of the structure score, routes and enable toggles included (`game::rings`, `RingAt::{Param, Route, Enable}`, read from each row's `answer_addr` as lane ST writes it), kept from each report and each `Rings` job |
| `game_job_done(id, result)` | GUI (or host main), under the lock | Hands a job's result back. A result for a job the round no longer waits for is `StaleJob` and changes nothing |
| `drain_game_requests()` | Wherever `drain_requests` is drained, under the lock | `Run(job)`, `Cancel(id)`, `Ab(state)`, `Log(line)`, `Dirty`; acted on **after unlocking** |
| `load_instance` / `save_instance` | Host main | Restore, reconcile or end the round ([below](#host-loads-during-a-round)); save its record |

Every other entry uses the latest tick's time. Edits, undo and redo need no new entry: the session's edit path (`deliver`) and host folds log them into the round.

### Jobs

Each job is a pure function of its payload (patches are owned or `Arc`-shared), so the worker never locks the session. At most one of each kind is in flight. Every job carries its `RoundId`, under which the worker keeps the round's state (the target's buffer, the round's `PreparedRound`) until another round starts or `Ab.target_ready` turns false. **The worker composes nothing** (F1r review S5): it calls lane CT's `prepare_round`, `score_round` and `score_structure`, which map the challenge's scorer ids to their implementations.

| Job | When | The worker | Result |
| --- | --- | --- | --- |
| `Prepare { spec }` | `game_start`, and on restoring a saved round (the spec from its code) | `enth_challenge::prepare_round(&spec)` (the target, phrase, randomized addresses and fingerprint; `Challenge::recreate(spec, phrase)`; every scorer it names, prepared), kept under the round; renders the target's loop of the scored phrase **and keeps the buffer** for Target ▶ (the session never sees audio), measures its loudness | `Prepared(Prepared::from_round(&round, target_lufs))`: challenge, start, target, `params`, `fingerprint`, `target_lufs` (a `non_exhaustive` struct, so a field added later breaks no shell); `SilentTarget` when `prepare_round` refuses a silent target (R1/S1 lane INT) |
| `Score { challenge, target, patch, submit }` | `game_submit` (the patch at the moment of Submit) | `score_round(&round, &patch)` | `Scored(ScoreReport)` |
| `Loudness { challenge, patch }` | The patch has settled (`SETTLE_MS`, 300 ms, after the last change) | Renders the phrase through the player's patch and measures it, for Mine ▶'s gain | `Loudness { lufs }` |
| `Rings { challenge, target, patch }` | Settled, after the first submit | `score_structure(&round, &patch)`: the per-knob Good / Near / Bad (no render) | `Rings(StructureScore)` |
| `Hint { challenge, target, patch, level }` | `game_hint` | The coach at that level (lane SN) | `Hint(Box<CoachHint>)`: the text and, since FU1 (lane HINT), what it points at ([below](#what-a-hint-points-at-lane-hint-fu1)) |

Any job may answer `Failed { message }`: a failed `Prepare` breaks the round, a failed score records no submit. **A report is checked before it counts** (F1r review S10): its scorer ids and pass mark must be the challenge's, every percentage finite and within 0–100, and its `passed` exactly what the challenge's `passes(sound %)` gives (`ScoreReport::check_against`); otherwise it is refused (`wrong_result`), no submit is recorded and Submit is free again. Rings from another structure scorer are refused the same way. A settled job that measured an older revision of the patch is asked again at the next tick. The game worker is SKR's own thread, not nice-plug's background worker (a one-second calibration would hold up the plan pump).

### What a hint points at (lane HINT, FU1)

`CoachHint` (`non_exhaustive`) carries the text and structured fields from the coach, each filled only from the level whose text already says it, and **none ever holds a value**: no target value, no fix, no level-4 range (the plan's verified fix stays in the worker). All are additive and serde-default (`skip_serializing_if` none), so a hint without them reads and writes as before. Shells build one with `CoachHint::new(level, text)` and the `with_aspect`, `with_module`, `with_param` and `with_route` builders.

| Field | From level | What |
| --- | --- | --- |
| `aspect`, `difference` | 1 | the coach's aspect code (`brightness`, `attack`, …; text, so a newer aspect never fails the hint) and how the player's sound differs (CT's `Direction`: `higher`, `lower`, `missing`, `extra`; a newer value reads `unknown`) |
| `module` | 2 | the module to look at; none for a region (the voice settings, the routes, general advice) |
| `param`, `turn` | 3 | the parameter (a route hint's destination) and which way (`HintTurn::{Raise, Lower}`; none for a categorical parameter or a route) |
| `route` | 4 | a route hint's route, source and destination (never its depth) |

**The session keeps each hint on that ladder** (`CoachHint::laddered`, applied when the `Hint` result arrives): a shell that fills too much can never make the editor mark a control before the text names it (`tests/game.rs::hints_keep_only_the_fields_their_level_names`). The latest hint is `RoundView::hint`; the next hint replaces it, and it goes with the round.

### A/B

`game_listen(Off | Target | Mine)` sets the switch; `AbState` carries it with `target_ready` and the loudness-match gains, and the session pushes `GameRequest::Ab` whenever it changes. **Target ▶** is the shell's pre-rendered loop; **Mine ▶** is the same phrase played live through the engine, locked to the loop's position; the shell does the flip and its short crossfade. The louder side is attenuated by the loudness difference and the quieter one is never boosted (`game::ab_gains`, both gains ≤ 0 dB; an unmeasured or silent side leaves both at 0 dB). The gains come from the same loudness measurement as the scorer's (P11); when lane SS lands, `ab_gains` calls its shared gain function. A/B sends the engine nothing: no engine work, as design.md §9 says.

### Time, submits and feedback

- **Active time** counts only between visible ticks while the target is ready: it pauses while the editor is closed and while the target is prepared or regenerated. A timestamp older than the last is treated as the last.
- **The time to pass** is the active time when the player pressed Submit on the first passing submit (not when its score arrived). A submit made just before a reveal still counts.
- **Score on submit**: the first scored submit is the recorded score; each later one is shown and kept. After the first, the rings come from each report and then from `Rings` jobs as the patch settles.
- **Hints** go up one level per call, up to `HINT_LEVELS` = 4 or the challenge's `assists.hint_budget` if lower; **a reveal** loads the target; both mark the round assisted (CT's `AssistsUsed`).
- **The dim assist** (the editor's `dim_unused` setting: the modules the target leaves alone are dimmed) marks the round assisted too (lane HINT, FU1): the editor calls `game_dim_unused` once the dimming first shows (the target ready, its randomized parameters known), and the record's `dim_unused` stays set for the round even if the assist is turned off later. It is refused while preparing (`not_ready`: nothing is dimmed yet) and for a broken round (`broken`); the editor only sends it when it would be accepted. `RoundView::dim_unused` says it is recorded, so the editor reports it once.
- **The blind rating** (`game_rate`, 1–5: "nothing like it" to "can't tell them apart"; F1r review S9) is asked **before the first score shows**, so the score cannot anchor it (ADR-0007's note, review point (a)): it is accepted from the start of play until the first score is recorded (`already_scored` after), never after a reveal, and a later rating replaces an earlier one. The editor asks for it when the player presses Submit and holds the score back until it is given or skipped. GR1 refits the curve with it (`sound/2`); it goes to `RoundRecord::rating` and `RoundResult::rating`.
- **The vote** (CT's `Agreement`: too low, about right, too high) and **a dispute** need a scored submit. A dispute is logged at once (a crash cannot lose it); the round's `end` line carries it too.

### The edit log

Every change to the round's patch is logged with the round's active time: editor edits, undo and redo (from `deliver`), host folds (from `fold_host`), host loads and the reveal (whole-patch changes, logged with empty ops). Ops are kept in their resolved wire form (`enth_patch::wire::to_json`), so replaying the log from the start patch with `wire::resolve` gives the round's patches (tested). A drag's moves of one target merge into one entry, as the history merges them. The log is capped at 64 KiB (`EDIT_LOG_BYTES`; every save and every host undo point carries it); entries over the cap are counted in `edits_dropped`. The whole log goes to the round's `end` line (`RoundLogEntry::edit_log`), where lane GK reads it.

### Undo and the stash

- Starting play stashes the player's patch (as patch-file text, exactly what the `patch` field held, so a patch this build could not read survives too) and their undo history, and gives the round an empty history: **Undo never leaves the round**.
- `game_end(Restore)` puts both back; the player's own steps are undoable again. After a project reload the stash comes back without its history (no history is ever saved).
- The start, a reveal and the restore each replace the whole patch the way a load does: an open drag is closed (its host gestures stay open until the editor's next `end_gesture`), the plan forgets the engine's remembered host slot values, the host baseline is reset, audition notes are released, the plan is sent at once, `RenameParams` goes out when the host would see other names, and `PatchEdited` marks the project dirty. Each is exactly one plan (tested against a running engine).
- **A reveal starts a new history line** for now: the round's steps do not apply to the target. An undoable reveal is lane PBC's undoable load (`ops_between`), an F1r follow-up.

### Host automation

The patch-wins rules hold in a round unchanged. The start patch has no slot assignments (lane RG's generator must keep it so), so host slot automation does nothing while a round plays; the first fold after the start only records, and the player's slots and names come back with the restore, where the first fold again only records (tested). Host macro automation does fold into the round's patch (macros are not slotted); it is logged as `host`, and it is audible only if the start patch routes a macro, which it should not. A slot the player assigns during a round works as usual.

### What a round saves

The `enth` field's optional `game` object, `{"round":{…}}` (`GameState`, `RoundRecord`), is **absent while no round is live**, so every state without a round keeps its bytes (the v1 fixtures are re-checked through game calls that start nothing, and after a round ended with Restore). It stays within `enth.plugin-state/1`: optional fields with defaults, unknown fields ignored, empty ones omitted. The round's entries take and set only its `round` field, so a later field of the `game` object (a seed history, a staircase) is never dropped with a round (F1r review N3).

**A newer build's round fails only itself** (F1r review M3). Every enum in it reads a newer value as `unknown`: the phase (a round in an unknown phase restores `Broken`), an edit's source (`other`), the vote and, inside the first report's text, every output enum. And the `game` object is read leniently (`GameState::lenient`): one this build cannot decode at all (a field of another type) restores its round `Broken` with its id, code and stash, so ending it brings the player's patch back; the editor preferences in the rest of the field always load (tested for each enum and for broken types).

**The budget** (F1r review S8). Every text the record saves is counted as it is saved, escaped inside the `enth` field's JSON: the stash at most `STASH_BYTES` (384 KiB), the submitted patch `MAX_SUBMITTED_PATCH_BYTES` (128 KiB, omitted above), the first report `FIRST_REPORT_BYTES` (64 KiB, omitted above), the edit log 64 KiB, the dispute 2,000 characters; with 255 submits and the fixed fields they stay under `ROUND_BYTES` (768 KiB), which leaves the rest of the field room under its 1 MiB (a compile-time check, and a test of the largest round). **A round whose stash would not fit is refused at its start** (`too_large`, nothing changes): the player's patch, typically 20 KB, only gets that big as a newer build's text kept unread. It holds:

| Field | What |
| --- | --- |
| `id` | `{serial, nonce}`: rounds counted in this instance, and the shell's time at the start |
| `code` | The text of the code the round started from. **The target is never saved**: it is regenerated from the code. Empty only while a custom-settings round prepares |
| `phase` | Where the round is |
| `stash` | The player's own patch, as patch-file text |
| `target_hash` | The target's fingerprint (64 hex digits): a regenerated target must match it, or the round is `Broken` (`target_drift`). Schema-independent, so a patch schema bump never breaks a saved round |
| `active_ms`, `time_to_pass_ms` | Active time so far, and the first passing submit's |
| `submits` | Every scored submit: its active time, pass, and sound, structure and combined % as **integer hundredths** (no JSON float in the record but the edit log's patch values) |
| `first_report` | The first submit's full `ScoreReport` (the recorded score) as its own versioned JSON text (`enth.score-report/1`), for the round's `RoundResult`; text, so a report schema this build cannot read never makes the `enth` field unloadable |
| `hints`, `revealed`, `dim_unused`, `rating`, `agreement`, `dispute` | Assists, the blind rating and the player's verdict. `dim_unused` (lane HINT, FU1) is omitted when false, so a round without it keeps its bytes and a record saved before it existed reads with it off (`a_round_saved_before_the_dim_assist_loads_with_it_off`) |
| `submitted_patch` | The last scored submit's patch, for the round log (omitted above 128 KiB escaped) |
| `edits`, `edits_dropped` | The edit log (64 KiB) |

Not saved, and recomputed after a reload: the target and its audio (the `Prepare` job runs again; play, Submit and Target ▶ wait for it, and so does the clock), Mine's loudness, the rings, the latest hint's text and the last full score report. The `patch` field holds the round's patch, so `enth render` of a mid-round project plays what the player hears. A pre-R1 build ignores `game` and drops it on its next save, losing the stash; no pre-R1 build went to friends.

### Host loads during a round

A host loads state when it opens a project and on its own Undo. nice-plug sets `enth` before `patch`, and `load_instance` reconciles the round it loaded with the live one:

| The loaded `enth` field holds | The live round | Then |
| --- | --- | --- |
| the same round (`id` and `code`) | live | **The live record wins.** REAPER's Undo restores an earlier save of this project, and must not roll back submits, the time or the vote. The patch still follows the host (logged as a `load`) |
| another round, or none | live | The live round ends, logged with `ended_by: host`; then the loaded one is restored, if any. A host load never marks the project dirty (a dirty mark would add a host undo point and cut REAPER's redo) |
| a round | none | **Restored**: the code is decoded (an undecodable one restores `Broken`, `code_too_new` included) and a `Prepare` job asked for with its spec; the patch is not touched. A custom round saved before its code existed restores nothing if it had no stash yet, and `Broken` (so the stash can come back) if it had |

### The round log

Each played round ends with one `GameRequest::Log(RoundLogEntry)`, schema `enth.round-log/1`: `event` (`end` or `dispute`), `round`, `code` (its text), `ended_by` (`restore`, `keep`, `new_round` (GR1's "want more"), `host`), `broken`, **`result`: lane CT's `RoundResult` (`enth.round-result/1`: outcome `passed`, `not_passed` or `gave_up`, the first submit's report, the submit count, the time to pass, active time, `AssistsUsed` (hints, the reveal and, since FU1, the dim assist: `dim_unused`, written only when true), the `Agreement`, the blind `rating`)**, `submits` (every scored submit as integers, beside the result's first report and count), `dispute`, `edits`, `patch` (the last scored submit's), `report` (its full report, while the session still holds it), `fingerprint` (the target's, once it was ready) and `engine` (this build's), so a replay of a logged short code can tell drift from another target (F1r review S3), and on the `end` line `edit_log` (the round's edit log). The line is read tolerantly: the fields added at F1r default when missing. `result` is `None` only when the code does not decode; if the saved first report is unreadable (a newer build's), the result counts no submit, so it stays valid, and `submits` keeps the scores. The shell appends it to a local JSONL file (I/O, never on the audio thread); lane GK writes the exporter and `cargo xtask gr1-report`. A round a host load restored after its `end` line can log again: readers keep the last `end` per `round`.

### Still to swap after F1r

| Now | Becomes | Touches |
| --- | --- | --- |
| `game::ab_gains` (attenuate the louder side by the loudness difference) | Lane SS's shared gain function (P11): **done**, SS's in-house loudness and `gain_db` (`9a0dfa2`) | `ab_gains` |
| A reveal starts a new history line | PBC's undoable load (`ops_between`) | `game_reveal` |
| `AssistsUsed::live_meter` is always false | The live-meter assist, if GR1 keeps it | `RoundRecord::assists` |
| The default scorers are CT's stubs (`sound-stub/1`, `structure-stub/1`, `combined-stub/1`) | `sound/1` (SS), `structure/1` and `combined/1` (ST), each switched in `enth_challenge::model::RECREATE_SCORERS` by the lane that landed it: **done** (`9a0dfa2`, `762351f`) | nothing in the session: it checks reports against whatever the challenge names |

### Decisions (lane SG)

1. **The saved record is the single source of truth.** It lives in `InstanceState::game` and is changed in place, so `save_instance` needed no change; the transient part (target, stashed history, jobs, clock, outbox) is a `Session` field.
2. **Integer scores in the saved state.** Floats never enter the record (scores are hundredths of a percent), so it stays `Eq` and byte-deterministic; the full report goes to the round log while it is in memory.
3. **The game has its own outbox** (`drain_game_requests`) beside `ShellRequest`: jobs go to a worker, A/B to atomics, log lines to a file, and `ShellRequest`'s shape stays as F1b froze it.
4. **Jobs are the only way to compute**, including generation, which is cheap: the session never depends on RG's or SS's code, and every job can be replayed from its payload.
5. **Results come back on the GUI thread**, never from the worker, because a prepared target replaces the patch, whose requests the GUI thread forwards.
6. **The same round loaded by the host keeps its live progress**: a host's Undo cannot undo a submit.
7. **The stash is patch-file text**, so it migrates like the `patch` field and keeps a patch this build cannot read.
8. **The record keeps the code's text, not a parsed code**, and the first report as text: a code or a report from a newer build fails only its own round (restored `Broken`, or a result without its first report), never the `enth` field and the preferences in it.
9. **The session verifies the target itself** (`ChallengeCode::verify`, the saved full fingerprint, the challenge's spec): a shell bug or a drifted generator cannot make a round score against another target.
10. **The session verifies each report too** (F1r review S10): it recomputes the pass from the sound % and refuses a report whose scorer ids or pass mark are not the challenge's.
11. **Every saved text is budgeted escaped, and a stash that would not fit refuses the start** (F1r review S8), so no round makes the `enth` field unloadable.
12. **A saved round is read leniently** (F1r review M3): unknown enum values read as `unknown`, and a round this build cannot decode keeps only its stash.

### What lane SKR must wire

1. **The clock.** In the editor's update closure, after `pump` and `fold_host`: `game_tick(monotonic_ms, Visible)`; `Opened` in every `build()`; `Hidden` from the fork's `editor_closed` (or when `EguiEditorState::is_open()` turns false).
2. **Waking an idle editor** (F1r review S11). The editor draws nothing while idle, so nothing would show Mine's gain, the live rings, a score or a prepared target until the next mouse move. Two wake-ups: (a) after each frame, ask for a repaint at `RoundView::wake_at_ms` (the settled jobs' time, or the running timer's next second; `None` while closed); (b) the worker requests a repaint, lock-free (egui's `Context::request_repaint` from its thread), each time it posts a result.
3. **The worker.** Its own thread: run each `GameRequest::Run(job)` and post `(id, result)`; drop cancelled ids; the GUI thread hands results back with `game_job_done` at the top of the frame, before the editor's edits. **Neither side ever blocks on a full queue**: two bounded queues between the GUI and the worker can deadlock them. The GUI keeps jobs it could not queue and retries them next frame; the worker keeps results it could not post and retries (or posts to an unbounded channel: the shell may use one, core crates may not).
4. **The `Prepare` job.** `enth_challenge::prepare_round(&spec)`, kept under the job's `RoundId`; answer `JobResult::Prepared(Box::new(Prepared::from_round(&round, lufs)))`. Render the target (`round.target()`) through its phrase (`round.phrase().performance()`, `SCORING_SAMPLE_RATE`, `SCORING_SEED`) into a preallocated loop buffer and measure its loudness; the buffer goes to the audio thread like a plan (a ring with a return ring), and is dropped when `Ab.target_ready` turns false. Score jobs are `score_round(&round, &patch)` and rings `score_structure(&round, &patch)`. The editor decodes a pasted code (`ChallengeCode::decode`) and passes `RoundSource::Code`; the settings page passes `RoundSource::Settings` with a freshly drawn seed (40 bits, so a preset's code stays short; never above 2⁵³). A refused start (`too_large`, `out_of_range`, `code_too_new`) shows its hint.
5. **A/B in `process()`.** Read the A/B atomics; play the buffer or the phrase (the shell's sequencer feeding Mine's notes), locked to one position, with a short crossfade and the gains (dB to linear in the shell). The `process-stats` A/B toggle (F2r).
6. **Score, rings, loudness and hint jobs**, through CT's `score_round` and `score_structure` (`sound/1`, `structure/1` and `combined/1` since SS and ST) and SN's coach.
7. **The blind rating.** When the player presses Submit, ask for the 1–5 rating (`game_rate`) before showing the score; hold the score back until it is given or skipped.
8. **The round log.** Append each `GameRequest::Log` line to the local JSONL file after unlocking (GK names the path).
9. **Dirty.** Coalesce `GameRequest::Dirty` with `ShellRequest::PatchEdited` into one `mark_dirty()` per frame.
10. **Draining.** `drain_game_requests` wherever `drain_requests` is drained (the GUI thread after each frame, the host main thread after a state load, which may ask for a `Prepare`).

### Tests (lane SG)

| What | Where |
| --- | --- |
| The dim assist (lane HINT, FU1): refused without a round, while preparing and for a broken round; marks the round assisted once; saved and restored; the log line's result carries it; a log line and a saved round from before the field still read | `tests/game.rs` (`the_dim_assist_*`, `a_log_line_from_before_the_dim_assist_reads`, `a_round_saved_before_the_dim_assist_loads_with_it_off`) |
| The lifecycle from start to pass; reveal; a submit in flight at the reveal; hint levels; a new round keeping the original stash; an end before play; abandon; disputes; stale, wrong and failed results; a failed preparation; every log line's `RoundResult` reads back as a valid `enth.round-result/1` | `tests/game.rs` |
| Codes and settings: a custom spec's long code made with the target and resumed from; a pasted long code with another prefix refused (`target_mismatch`); another engine warns (`engine_differs`) and plays; refused settings change nothing; a challenge for other settings; the hint budget; a custom round saved before its code | `tests/game.rs` |
| A/B: sides, flips, the louder side attenuated, a stale loudness asked again, rings after the first submit | `tests/game.rs` |
| The injected clock: preparation, idle frames, a closed editor, an unreported close, a timestamp from the past, the time to pass | `tests/game.rs` |
| Undo inside the round, the player's history back, Keep keeping the round's history, a drag across the start; the edit log's merge, times, host entries and replay; host slots in a round | `tests/game.rs` |
| The saved state: v1 fixtures byte-identical without a round and after one; a project saved mid-round restored, stash included; target drift; a bad code and a newer phase; same-round, other-round and no-round host loads; within version 1; no float in the record; the round fixture | `tests/game.rs`, `tests/fixtures/state/v1/round.*.json` |
| No audio-thread contact: only the start, a reveal and the restore reach the engine, one plan each; twelve rounds while a test audio thread renders under `assert_no_alloc` | `tests/game.rs`, `tests/game_audio.rs` |
| Gains, score rounding, unknown phases, escaped lengths, `Send` | `src/game.rs` unit tests |
| The F1r fixes: newer enum values and an undecodable round keep the preferences and the stash (T3); the largest round fits the field (T3); a stash too large refuses the start; inconsistent reports refused, seeds from 2⁵³ refused (T6); `params`, the blind rating, `wake_at_ms`, the log line's fingerprint, engine and edit log, rings on the player's control, a fingerprint drift | `tests/game.rs` (`t3_*`, `t6_*`, `s*_*`, `m1_*`) |
| **T5:** a state machine of random intents, edits, ticks, saves, host loads and job results in any order (right, failed, wrong kind, stale, inconsistent): no panic, every log line's `RoundResult` reads back, `time_to_pass_ms ≤ active_ms`, no submit after a reveal, at most one job of each kind in flight, the outbox drains, nothing left in flight after the end | `tests/game_props.rs` |

## Learn (lane SL, R2)

`enth-session`'s `learn` module binds a lesson to the live patch ([learn.md](learn.md#the-lesson-model-sketch), [phase-3.md](phase-3.md), freeze point F1l). Like `game`, it does no I/O, spawns no threads and reads no clocks: **time comes from the timestamps the shell passes to `game_tick`** (the round and the lesson share that clock), and **background work runs as `LearnJob`s the shell's worker executes** and hands back. The audio thread never touches any of it: a lesson reaches the engine only by replacing the patch, **one plan per step load**.

**Who does what.** Lane LM's player (`enth_learn::player::Player`) owns the lesson: its steps, checks, hints, phases and progress records. It is pure, and every mutating call answers with effects, in order: `LoadPatch`, `StartRound`, `EndRound`, `View`, `Examples`, `Check`, `HintJob`, `Hint`, `Measured`, `Apply`, `Chose`, `Reveal`, `Passed`, `Record`, `Finished`, `End`. The session acts on them and does the rest: the stash, step loads, locks, the round, job bookkeeping, the saved state and the read model. **Until LM's player and CV2's selector merge, `learn.rs` carries stand-ins** mirroring the types on their branches (checked against `r2/lm` at `b9f511f` and `r2/cv2` at `3219eaf`): `LessonRef`, `Selector`, `Touch`, `StepView`, `StepPhase`, `PlayerEffect` and the others, and a `LessonPlayer` trait whose methods mirror LM's `Player` one to one. It adds one accessor, `round_challenge()` (below, under reloads). Lane F1L swaps the stand-ins for re-exports and implements the trait for `Player`, or holds `Player` directly.

### The lifecycle

| Entry | Does |
| --- | --- |
| `LearnEdit::learn_start(player)` | Stashes your patch (as patch-file text, so an unreadable one survives) and your undo history, saves `{lesson, stash}`, and loads the first step. Refused over a game round of its own (`round_live`: two stashes would not fit the field) and for a patch too large to stash or a reference too long to save (`too_large`). A new lesson while one is live keeps the original stash |
| A step load (start, `learn_next`, `learn_back`, or the player moving on by itself) | Ends the step that was: its round (logged, `ended_by: keep`), its jobs (cancelled), its examples. Then a fresh history (**undo never leaves the step**), the step's view and locks, and its start seated like a load: one plan. A Recreate step's start comes from its round instead, which is still one plan; a step without a start keeps the patch (no plan) |
| `learn_hint` | The player's authored text at once (`Hint`), or a measured-hint job (`HintJob`, its answer shown as `Measured`); `no_hint` when none is left |
| `learn_show` | The witness's next op (`Apply`), applied **through `Session::edit`**: one undo step, and the locks hold. For an ear question, its answer (`Reveal`) |
| `learn_choose(i)` | An ear question's answer |
| `learn_listen(Some(i) \| None)` | Plays an example once the examples are rendered (`no_example` before) |
| `learn_end(Restore \| Keep)` | The step's round ends first (logged with the same choice). Restore seats the stash with your own history (one plan); Keep keeps the lesson's patch and the step's history. The `learn` object goes, so the field keeps its earlier bytes |

The read model is `LearnEdit::lesson()` (`LearnView`, rebuilt every frame): the lesson reference and step, the step count, the phase, the step's view (highlight, hide, lock), whether the step's round is live (draw the Recreate view through `GameEdit::round`, whose new `lesson` flag hides the round's own end and new-round controls), pending jobs, the latest authored hint, the Listen state, `finished`, `broken` and the last error. `Session::learn_player()` gives the editor the player for the step's text.

### Who calls what

| Entry | Thread | Does |
| --- | --- | --- |
| `game_tick(now, view)` | GUI, once per frame (unchanged) | Also the lesson's clock: the player's `tick` may ask for a sound check once the patch has settled |
| `LearnEdit::*` | GUI, at the end of the frame like `SessionEdit` | The editor's intents and read model |
| `learn_job_done(id, result)` | GUI (or host main), under the lock | Hands a job's result back. A result for a job the lesson no longer waits for, or one that measured an older revision of the patch, is `stale_job` and changes nothing |
| `drain_learn_requests()` | Wherever `drain_requests` is drained | `Run(job)`, `Cancel(id)`, `Record { lesson, record }` (merge into the progress file), `Listen(state)`, `Dirty`; acted on **after unlocking** |
| `load_instance` / `save_instance` | Host main | Restore or reconcile the lesson (below); save its object |

Edits, undo, redo, host folds and host loads need no new entry: the edit path, `fold_host` and `load_patch` tell the player (as `Player`, `Host` and `Load` edits), each with a revision that counts every change of the patch over the whole lesson. The player's jobs carry that revision.

### Locks

**The session refuses, the overlay only draws.** `Session::edit` checks the step's lock selectors **before anything changes**, so the editor, Show me and every shell get the rule: a transaction in which any op touches a locked control is refused whole, `SessionError::Locked { op, touched, lock }` (code `locked`, with a hint), shown in the status line through `last_error`. Each op is judged against the patch as the ops before it leave it. A selector is lane CV2's `Selector`, in LM's text grammar: `filter1.cutoff`, `filter1.*` (also `voice.*` and `master.*`: the parameters, the module itself and the routes from or into it), `lfo1 -> filter1.cutoff`, `macro3` (its knob; a macro's routes are touches of their own, so a route or parameter lock holds against the knob). What an op touches is `learn::touches`:

| Op | Touches |
| --- | --- |
| `SetParam` | the parameter |
| `InsertRoute`, `RemoveRoute`, `SetDepth` | the route, and its destination parameter (modulation moves it) |
| `SetMacro` | the macro, and each route from it with its destination |
| `SetMacroName` | the macro |
| `AddModule`, `RemoveModule`, `SetEnabled` | the module, every parameter it has, every route from or into it |
| `Move`, `SetLabel` | the module |
| `SetAmpEnv` | the new amplitude envelope and the old one |
| `SetSlot` | the slot's new target and its old one (a slot lets the host move a parameter) |
| `SetMeta` | nothing |
| a kind this build does not know (`Op` is `non_exhaustive`) | refused while anything is locked (fail closed) |

**The policy, for the F1l judge:**

- **Only `locked` is enforced.** `Constraints.editable` is advisory: the editor dims, and lane CLS uses it as `closest`'s free set. A Recreate step's locks come from its challenge's `Constraints.locked`, which the player copies into the step's view, so the session has one channel.
- **Host automation of a locked parameter folds but never counts.** The patch-wins rules hold, and the host is never refused. Every check (the player's patch predicates, its sound checks, a lesson round's score and rings) sees `Session::checked_patch()`, the live patch with each locked control at its value from the step load. A host state load is treated the same way.
- **Undo never leaves the step:** every step load starts a fresh history; your own history waits with the stash.
- **A preset load asks Restore or Keep:** `Session::load_preset` is refused while a lesson is live (`SessionError::LessonLive`, code `lesson_live`). The editor asks, ends the lesson, then loads.
- **One stash at a time:** `learn_start` is refused over a game round of its own, and the editor's `game_start` is refused while a lesson is live (`GameError::LessonLive`), as is `game_end` on the lesson's own round (`GameError::LessonRound`).
- **While a lesson is reopening after a reload,** its locks are not known yet and edits are allowed (a frame or two; if the lesson cannot be found, it breaks and only ending it is left).

### Jobs

| Job | When | The worker | Result |
| --- | --- | --- | --- |
| `Open { lesson }` | After a reload | Finds the lesson in the installed packs and resumes its player at the saved step (`Player::resume(bundle, step)`) | `Opened { player, effects }` (`effects` is what the resume returned), or `Failed` (the pack is gone: the lesson breaks, and Restore still brings your patch back). A player for another lesson is refused (`other_lesson`) |
| `Check(request)` | The player's `Check` effect, from `tick`, once the patch settled | Runs the sound check (`enth_learn`'s evaluation) | `Checked(verdict)` |
| `Hint(request)` | The player's `HintJob` effect, past the authored hints | `enth_solver::hint` | `Hinted(hint)` |
| `Examples { examples }` | A Listen or Choose step loads | Renders each example into the Target ▶ slot, as one loop each, and measures their loudness | `Examples { lufs }` |

At most one job of each kind is in flight; a step load cancels the step's jobs. **Stale results are refused twice:** by the session, by job id and revision, before the player sees them, and by the player, by revision.

### Lesson rounds

A Recreate step starts a real game round through the additive `RoundSource::Challenge(Box<Challenge>)` (so `RoundSource` is no longer `Copy`). The worker answers the new `JobKind::PrepareChallenge { challenge }` with `Prepared::from_round(&enth_challenge::prepare_challenge(challenge)?, lufs)`, and the session checks that the prepared challenge is the one it asked for. The round's record and its log line carry `origin: "lesson"` (`RoundOrigin`, omitted for a game round, so game rounds keep their bytes and older lines read as game rounds; a newer origin reads as `other`). `gr1-report` leaves such lines out. **A lesson round never stashes**: the `learn` object holds the only stash. It has a code only when its challenge has a seed target, and its log line's `result` is `None` without one. When the round first passes, the player hears `round_ended { passed: true, assisted, best_sound_pct }` (the best sound %, rounded down), and on a reveal `round_ended { passed: false, assisted: true }`. The round stays live until the step moves, or until the player's `EndRound`. Outside a lesson, `RoundSource::Challenge` starts an ordinary round that stashes; one with a non-seed target has no code, so it does not survive a project reload (it restores `Broken`, its stash kept).

### What a lesson saves

The `enth` field's optional `learn` object holds the in-flight `LessonRef` (pack, version, lesson, step and the 16-hex-digit content prefix) and the stash: `{"lesson":{"pack":"basics","version":"1.0.0","lesson":"02-filter","step":1,"content":"0123456789abcdef"},"stash":"…"}`. It is **absent while no lesson is live**, so every state without a lesson keeps its bytes (the v1 fixtures are re-checked without a lesson and after one ended with Restore). It stays within `enth.plugin-state/1`. Per-user progress is not here: it is the shell's file, fed by `LearnRequest::Record`.

- **Read leniently, keeping the stash** (S0l's open item, decided): a `learn` object this build cannot decode keeps its stash, as `GameState::lenient` keeps a round's. The lesson restores broken, and Restore brings your patch back.
- **A reload** restores the object and asks for `Open`. When the player comes back at the saved step with the same content, the step **resumes** with the effects its resume gave (its view and examples, never a start patch or a round): the locks come back, the start patch is not seated (the `patch` field holds the step's patch), and a saved lesson round is prepared again from the player's `round_challenge()`, under its own id, and checked against its saved fingerprint. When the lesson changed (another content prefix), the session calls the player's `start()` and loads step 0 normally; the old step's round ends. A lesson round whose lesson does not come back breaks; it never stashed, so ending it changes nothing.
- **A host state load during a lesson.** The same lesson at the same step with the same stash (REAPER's Undo within a step) goes on with its player. Anything else is the host's: the live lesson ends without a restore (the patch is the host's), nothing is marked dirty, and the loaded lesson, if any, is reopened.

**The budget** (the F1l named check). The field is 1 MiB. A game round reserves `ROUND_BYTES` (768 KiB, a 384 KiB stash included). A lesson round never stashes, so it takes at most `LESSON_ROUND_BYTES` (384 KiB). The `learn` object takes at most `LEARN_BYTES`: the stash, a reference of at most `LESSON_REF_BYTES` (1 KiB; a longer one is refused at the start), and 1 KiB of keys. A compile-time check keeps the two plus 64 KiB for the rest of the field under 1 MiB. Measured by `tests/learn.rs::the_largest_lesson_and_its_round_fit_the_enth_field`, with every part at its cap and escaped at its worst:

| Part | Bytes | Budget |
| --- | --- | --- |
| The `learn` object | 394,252 | 395,264 |
| The lesson round | 292,965 | 393,216 |
| The whole field (with a 64-character layout and a 4 KiB preset key) | 691,543 | 1,048,576 |

### Listen examples and the Target ▶ slot

**They can share it, by time.** The open check from [phase-3.md](phase-3.md#open-checks) is answered as follows. A Listen or Choose step never has a round, and the session ends a step's round before the next step loads, so the slot is free whenever examples play. An A/B pair must flip position-locked like Target ▶ and Mine ▶, so the slot's payload becomes **a set of loops, one per example** (2 to 4), with the playing one picked in the Listen state (`ListenState { example, ready, gains_db }`, pushed like the round's A/B state). The louder level-matched examples are attenuated to the quietest and never boosted (`level_match`, default on). The shell drops the loops when `ready` turns false. The session side is built and tested; the shell's (lane SKL) is not.

### What lanes SKL and LUI must wire

1. **The player.** Build it from the pack the shell read (`Player::new(bundle)`) and pass it to `learn_start`. Answer `Open` with `Player::resume(bundle, step)`'s player and effects.
2. **The worker.** Run `LearnRequest::Run` jobs like the game's (never blocking on a full queue), and answer `JobKind::PrepareChallenge` with `prepare_challenge`. Results come back through `learn_job_done` on the GUI thread.
3. **The progress file.** Merge each `LearnRequest::Record` after unlocking (re-read, `Progress::merge`, temporary file, rename).
4. **Listen.** Store `LearnRequest::Listen` where `process()` reads it, and play the example loops from the Target ▶ slot.
5. **The editor.** Draw `LearnView`. Ask Restore or Keep before a preset load (on `lesson_live`). Show a `locked` refusal on the status line.
6. **Dirty.** Coalesce `LearnRequest::Dirty` with `ShellRequest::PatchEdited` and `GameRequest::Dirty`.

### Tests (lane SL)

| What | Where |
| --- | --- |
| The lifecycle: the stash, step loads, Next, Back, Finished, Restore with your history, Keep, a new lesson keeping the first stash, `too_large` refusals | `tests/learn.rs` |
| Every op kind on a locked control refused, changing nothing and delivering nothing (13 kinds, plus a macro route and a cleared slot); a transaction refused whole; free controls and `SetMeta` stay editable; Show me through the locks; a negative control (the check disabled) fails these tests | `tests/learn.rs` |
| One plan per step load against a running engine (a patch step, a Recreate step, a step without a start, Back, Restore), and no other learn entry reaching the engine | `tests/learn.rs` |
| No audio-thread contact: six lessons while a test audio thread renders under `assert_no_alloc` | `tests/learn.rs` |
| Jobs: a settled sound check, a stale result refused after an edit, wrong and failed results, cancellation on a step change, hints, Listen examples level-matched | `tests/learn.rs` |
| The lesson round: `prepare_challenge`, no stash, `origin: lesson` on the log line, `game_end` refused, a pass reaching the player, resumed after a reload, broken without its lesson; a game round's bytes unchanged; a challenge round outside a lesson | `tests/learn.rs` |
| The saved state: a mid-lesson save keeps the stash and resumes at its step; a lesson not found again (or another lesson, or an unreadable object) keeps its stash; an updated lesson restarts at step 0; the v1 fixtures byte-identical; the budget; host folds of locked controls never counting; host loads | `tests/learn.rs`, `tests/state.rs` |
| The selector grammar, covering rules, the lenient reader, `Send` | `src/learn.rs` unit tests |

### Decisions (lane SL, for the F1l judge)

1. **The lock check sits at the head of the edit path**, before the history applies anything, rather than in `deliver`, which runs after the patch has changed. Undo and redo need no check: a step's history only ever holds ops that passed it.
2. **Locks are judged on touches**, and a parameter lock also covers routes into that parameter and the module's enable flag and removal. Only `locked` is enforced.
3. **Checks see the checked patch**, with locked controls pinned at their step-load values, so the host can automate a locked control without it counting.
4. **One stash at a time**, kept by refusals both ways between lessons and game rounds.
5. **An unreadable `learn` object keeps its stash.**
6. **Lesson rounds resume from the lesson**, by its reference and the round's saved fingerprint, not from a code (non-seed targets have none).
7. **The session refuses stale results itself**, by job id and revision, and the player's own check stays as a second guard.
8. **A host load other than the same lesson's ends the live lesson without a restore**, like a host load of another round.

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

## What the session relies on from the engine (lane E+, merged at `92102b0`)

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

- **ED2, engine:** the tap's layout tearing across a plan swap is fixed (S2s): the tap is a seqlock now, and a reader never pairs an epoch with another plan's layout; when the audio thread is preempted mid-publish for longer than a bounded retry, `read` returns the previous generation with no routed values and no modulators, and the reader's next refresh sees a whole frame. The tap's quiet rule, which stopped publishing global-modulator outputs, macros and a new epoch once a ring of silence had passed (an editor opened during silence got nothing until a note played), is decided and fixed by ED2: in silence the tap publishes only when a value it shows changed ([dsp/engine.md](dsp/engine.md#telemetry-tap-p1b-lane-e), [editor.md](editor.md#live-feedback-and-repaint-gating-lane-ed2)).
- **PL:** `Engine::reset` reports no `NOTE_END`. Emit them for `active_voices()` before calling it, for hosts that track voices. In mono and legato, a return to a held key keeps the released note's poly-mod offsets, because the engine keeps no per-note offsets. Bitwig decides whether that matters.
- **SK:** prune `telemetry_maps` to the last `MAX_PLANS_IN_FLIGHT + 1` epochs. Treat a stale `GestureId` as no gesture (`History::clear` restarts the ids). Clear `unloaded_patch` in `edit`. Push a `SetSlot`'s host write only when its plan went out.
- **S2s (done):** fold against the plan's value ("The host bridge" above); the host-load history policy ("Host state loads and the history"); state versions and migrations; slot and macro names and rescans; the telemetry seqlock. Left for SK to wire: see "What lane SK must know" below.
- An `Activation`'s `offline` flag is fixed at activate. If a host switches to offline rendering without re-activating, audition notes are not refused. REAPER re-activates, so add a setter only if another host needs one.

## What lane SK must know

S2s was written against the S1s stubs, so the engine-facing glue was built and tested with a hand-made active session (`src/tests/kit.rs`: a real `Engine::with_link`, a `Port` built by hand, and small doubles for `edit` and the pump's plan send). Public-API tests of the same ground are in `crates/enth-session/tests/engine.rs`; they are `#[ignore = "needs lane SK"]`, and a tripwire test there fails the day `activate` stops answering `Unsupported`.

SK's branch (`p1b/sk`, checked at `d70e842`) implements `activate`, `pump`, `edit` and the audition calls, and also wrote its own `Session::deliver` (a different signature and a narrower job than S2s's). The two branches merge textually with one conflict (`tests/api.rs`, the stub test) and then do not compile (two `deliver`s). A trial merge, resolved as below, passes every test of both lanes (S2s lib 38, api 12, bridge 19, engine 8 with the ignores removed, state 22; SK link 20):

1. **One `deliver`: S2s's.** `Session::deliver(&mut self, before: &Patch, ops: &[Op], in_gesture: bool) -> Applied` is the shared tail of an edit, an undo and a redo. Delete SK's `deliver(routing, ops, in_gesture)` and rewrite `edit` to: `pump()`; treat a `GestureId` that is not `self.gestures.open` as `None`; return `Applied::default()` for no ops; `let before = self.patch.clone()`; `self.history.edit(&mut self.patch, ops.clone(), gesture)` (mapping its error to `SessionError::Edit`); clear `unloaded_patch` and `last_error`; `self.deliver(&before, &ops, gesture.is_some())`. S2s's `deliver` has everything SK's has (the host gesture bookkeeping is SK's logic, moved: a `Begin` at the first write inside a drag, a `Set` inside a host gesture a load left open, `PatchEdited` once) and adds: the events once per parameter, the queue helpers (`send_or_queue`; `Queued::is_value`/`same_target` can then lose their duplicates, and SK's `Port::send_value` has no caller), `forget_slots = u64::MAX` when the queue overflows into a plan, and a rescan only when the host would see other names or kinds.
2. **`activate` must `self.host.memos.clear()`** (S2s's fold memos are keyed by `Link::epoch()`, and a new engine starts at epoch 0 again; `tests/engine.rs::a_reactivation_forgets_what_the_fold_remembered_about_the_old_engine` fails by exactly one rounding step without the line). Leave `host.seen` alone.
3. **`tests/api.rs`**: keep SK's activate half of the stub test and S2s's `undo`/`redo` assertions (`Ok(false)` with nothing recorded); the `Unsupported` value the loop at the end uses can be any.
4. **`src/tests/kit.rs`**: add SK's `auditioned: false` to the hand-built `Port`.
5. **`tests/engine.rs`**: remove the `#[ignore]` attributes and delete `lane_sk_has_not_landed_yet`.
6. **Dead code after step 1**: delete SK's `Port::send_value` and `Queued::same_target` (S2s's `send_or_queue` and `same_target` in `apply.rs` replace them), and allow `dead_code` with a reason on `SessionError::unsupported` (no stub is left).
7. **`enth-plugin/src/state.rs`, `a_damaged_field_never_panics`**: a newer `enth` field (`enth.plugin-state/99`) is now kept and saved back unchanged (S2s's policy), so assert equality with the stored text and `status().instance_unloaded`, and test the re-saved-defaults case with a damaged field that is not newer (`{"schema":"bogus"}`).

The result is on the local branch `trial/s2s-plus-sk` (a merge of `p1b/sk` into `p1b/s2s`, not pushed): `scripts/gate.sh lane .` exits 0 there with 700 tests passing. Use it, or redo the seven steps.

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
