# The session (`enth-session`, L6)

**One single-threaded `Session` owns the live patch.** It holds the `Patch`, its `History`, the engine's `Link`, the `PlanMap` of the latest plan it sent, and the per-instance state outside the patch. The editor and the plugin shell change the patch only through it. It does no I/O, spawns no threads, reads no clocks and takes no locks; the plugin wraps it in one mutex. Lane S1s wrote this page and the crate's public API (freeze point F1b, [phase-1b.md](phase-1b.md)); lanes SK and S2s fill in the bodies that talk to the engine. Sources: research report 01 (R2, R3, R7, R8) and 02 (D5, D7) in `_local/sessions/2026-10-19-p1b-plugin-editor/`, [design.md §3–§4](design.md#4-host-parameters-macros--automation-slots), and the engine seams in [dsp/engine.md](dsp/engine.md#p1b-seams-lane-e-what-the-session-plugin-and-editor-lanes-rely-on).

## The API in brief

| Item | Module | What it is | State (F1b) |
| --- | --- | --- | --- |
| `Session` | `model` | The owner: `new`, `patch`, `history`, `instance`, `status`, `drain_requests`, `set_ui`, `set_monitor_bypass`, telemetry accessors | Implemented |
| `SessionView` | `model` | The editor's read model: patch, history, status, instance state, telemetry map and source | Implemented for `Session`; fakes can implement it |
| `SessionEdit` | `model` | The editor's edit trait: gestures, `edit(ops, gesture)`, undo, redo, UI state, monitor bypass, audition | Gestures implemented; `edit` is SK's stub, `undo`/`redo` S2s's, audition SK's |
| `ShellRequest` | `model` | The outbox the shell drains: host gesture writes, renames, the monitor bypass, "patch edited" | Implemented |
| `SessionError` | `model` | `Edit`, `Compile`, `State`, `Inactive`, `Busy`, `Offline`, `Unsupported`; each with `code()` and `hint()` | Implemented |
| `Routing`, `Delivery`, `route`, `delivery` | `apply` | How an op reaches the engine (the table below) | Implemented and tested per op kind |
| `HostParam`, `HostValues`, `HOST_PARAM_IDS` | `bridge` | The 72 host parameters with permanent ids (`macro_1`…`macro_8`, `slot_1`…`slot_64`) | Implemented |
| `fold_value`, `Session::fold_host`, `host_values`, `host_updates`, `host_param_name` | `bridge` | The patch-wins rule, host writes, names | Implemented and tested |
| `InstanceState`, `SavedState`, `LoadReport`, `StateError` | `state` | The saved state v1: the `patch` field and the `enth` field (`enth.plugin-state/1`) | Implemented, fixtures under `tests/fixtures/state/v1/` |
| `Session::{save, save_patch, save_instance, load, load_patch, load_instance}` | `state` | Save and load | Implemented (the host-load history policy is S2s's to revisit) |
| `Session::{activate, deactivate, pump}`, `Activation`, `PumpReport` | `link` | The engine side | `deactivate` implemented; `activate` and the active `pump` are SK's stubs |
| `TelemetrySource`, `TelemetryFrame`, `TelemetryMap`, `TelemetryReader` | `telemetry` | The tap's reader side; `TelemetryFrame` is the engine's `TapFrame`, and `Tap` implements `TelemetrySource` | Implemented and tested against a real engine |

Stubs return `SessionError::Unsupported { what, lane }` (or `Inactive` while detached), never panic, and name the lane that fills them in.

## Threading contract

### Who calls what, on which thread

| Thread | Calls into the session | Never |
| --- | --- | --- |
| **Host main thread** | nice-plug `initialize`/`activate`: `fold_host`, `activate` (returns the `Engine`); `deactivate`; state save through the persistent fields' `map`: `save_patch(Some(&host))`, `save_instance()`; state load through the fields' `set`: `load_instance`, `load_patch`; then `drain_requests` | Call into the host while holding the lock |
| **baseview's GUI thread** (the editor; a separate thread in baseview 0.4.1) | Once per frame: `pump`, `fold_host(&host)`, then the editor's reads and edits through `SessionView`/`SessionEdit`, then `drain_requests` into a local buffer; **unlock**; then forward `ShellRequest::Host` writes to `ParamSetter` | `GuiContext::set_state` (see below); keep the lock while calling `ParamSetter` |
| **Audio thread** (`process`, CLAP `reset`) | **Nothing.** It owns the `Engine`, a preallocated event buffer and the last host values it saw | Lock anything; touch the session; schedule GUI work (`execute_gui`) |

- **The mutex.** The plugin holds `parking_lot::Mutex<Session>` in its shared state, used by the host main thread and the GUI thread only. It is the only lock, so there is no lock order. A hold is at most one editor frame's edits, with at most one plan compile (about 0.9 ms for the G1 patch, mostly the reverb's preparation; [design.md §3](design.md#3-engine-and-real-time-safety)). The session itself takes no lock and is `Send` (a test checks).
- **Never call into the host or nice-plug while holding the lock.** A host may call back synchronously on the same thread, for example REAPER reading the state for an undo point when a parameter changes, and the persistent field's `map` would then lock the session again and deadlock. So the shell drains `ShellRequest`s into a local buffer, unlocks, and only then acts on them. This matters more once baseview PR #357 lands and the GUI thread becomes the main thread; the mutex stays correct either way.
- **The audio thread's side.** `process()` sends host automation as `EventKind::Slot { slot, value }` (the modulated normalised value, so CLAP mono modulation follows), macros as `EventKind::Macro`, CLAP poly-mod as `EventKind::MacroVoice { key: NoteKey::note_id(id), .. }`, never a bare `EventKind::Param` (a plan swap inside the block would misroute it). It sends only values that changed since its last block; at activate the shell sets its "last seen" values to the current host values, so nothing is echoed. CLAP `reset` is `Engine::reset()` there and nothing else. After `process` it takes the tap's wake flag (`engine.tap()` → `Tap::take_wake`) and asks for a repaint, which is a lock-free call.
- **The `set_state` pitfall.** Never call `GuiContext::set_state`, from the editor or anywhere: while the plugin is processing, nice-plug applies that state **on the audio thread**, where the field's `set` would lock the session and compile a plan. An editor preset load calls `Session::load_patch` directly, on the GUI thread, under the lock. A clippy `disallowed-methods` entry in `enth-plugin` bans it, along with `execute_gui` from `process()`.
- **Host writes only on the GUI thread.** `ShellRequest::Host` writes are produced only inside `SessionEdit` calls, which only the editor makes, and the editor glue drains them before it unlocks. A main-thread drain therefore never sees one. A load that interrupts a drag leaves the host gestures it opened open until the editor's next `end_gesture`, so begins and ends stay balanced and are sent from the same thread.

### Who drains the return ring when the editor is closed

**The session, at its next entry; nobody needs to before that.** `Session::pump` takes replaced plans back (`Link::collect`) and drops them on the calling thread; it runs at the top of every entry that can send (an edit, undo, redo, a load, activate) and once per editor frame. Plans are only ever sent from such entries, at most `MAX_PLANS_IN_FLIGHT` (2) are outstanding, and the return ring has room for at least that many plus a pending slot (`Session::with_config` raises a smaller `returns` to 2). So the audio thread can always hand every plan back, and an idle session (editor closed, no host calls) costs at most two retired plans' memory, about 1.6 MB each with a reverb, until the next entry: a state save, a load, activate, deactivate, or the next editor frame. No timer runs while the editor is closed. The residual case is under "Open questions".

### Editor open and close

- Every `build()` is a fresh open (nice-plug-egui calls no close hook without the fork): the editor ends any open gesture (`end_gesture`), creates a `TelemetryReader` from `telemetry_source()` and enables the tap.
- Closing (the fork's `editor_closed`, or `EguiEditorState::is_open()` turning false): `end_gesture`, `audition_all_off`, disable the tap. The session releases audition notes itself on deactivate and on every load.

## Delivery

`apply::route(before, op)` classifies an op against the patch before it; `apply::delivery(before, ops)` turns a transaction into one plan or a list of events.

| `ops::effect` | Routing | The session sends | The engine |
| --- | --- | --- | --- |
| `Value(addr)` | `Value` | `PlanMap::get(addr)` → `Link::send_param(epoch, id, plain)`; nothing if the address is not in the plan (a disabled module: the next plan carries it) | Base set at the next segment boundary, smoothed; the slot table's patch value follows |
| `Depth(route)`, old and new depth non-zero | `Depth` | `PlanMap::depth(route)` → `Link::send_depth(epoch, id, pct)`; a recompile if the id is `None` | Ramps over 10 ms, then exactly the recompiled value |
| `Depth(route)` from or to 0 | `Recompile` | A new plan (depth-0 routes have no term) | Swap, no crossfade |
| `Macro(i)` | `Macro` | `Link::send_macro(i, v)` | Smoothed over 10 ms |
| `Structural` | `Recompile` | A new plan | Swap with state carry, moved at an instant swap; crossfade only if modules were added or removed |
| `NoAudio` from `SetSlot` | `Recompile` | A new plan (the slot table lives in the plan), and one host write of the new target's value | The remembered host value is forgotten (the slot's key changed) |
| `NoAudio` from `SetLabel`, `SetMeta`, `SetMacroName` | `Silent` | `ShellRequest::RenameParams` when names may change | — |

A transaction with any `Recompile` op is **one** plan compiled from the patch after it, and its value events are skipped (the plan carries them). Undo and redo deliver their ops the same way. Editor edits also produce host writes (`bridge::host_updates`): a `SetParam` on a slotted address sets each of its slots to `to_norm` of the new value, a `SetMacro` sets its macro, a `SetSlot` sets the new target's value. In a gesture the session sends `Begin` at the first write to a parameter and `End` for each at `end_gesture`; outside a gesture it sends begin, set and end at once.

### Invariants

Each gets a test in the lane that implements it (S2s's acceptance lists them).

- **I1, one writer.** Every patch change is an `Op` applied by the session. Editor edits, undo and redo are recorded; host folds and loads are not.
- **I2, delivery follows the op**, after the transaction succeeded. A rejected transaction changes nothing and delivers nothing.
- **I3, lazy plans.** At most one compile is pending, compiled from the patch at the moment it can be sent, so a plan always reflects the patch at the time it was sent.
- **I4, epochs.** Value and depth events carry the epoch of the latest *sent* plan and use its `PlanMap`.
- **I5, never blocks, never drops.** `SendError::InFlight` keeps the compile pending. `SendError::Full` queues the value (the last per parameter, at most `SessionConfig::value_queue`); an overflowing queue is cleared and a compile marked pending instead. Audition note-offs are never dropped; a note-on that does not fit is refused with `Busy`.
- **I6, the patch wins** (next section).
- **I7, load.** The patch wins; host values restored with the state are not folded during the load. The history is cleared and the load counts as saved; names are refreshed; while active a recompile is pending. A field that does not load gives the init patch (or the default instance state) and a typed error; it never panics.
- **I8, save.** Fold the host's values first, then write `Patch::to_json()`. A host project save does not touch the history or the dirty flag.
- **I9, undo and redo** deliver through I2; a structural undo is exactly one plan.
- **I10, audition notes** use `AUDITION_PORT` (`0x7F00`) and a fresh note id each, go through `Link::send_event`, are refused while the host renders offline, and are released on editor close, deactivate and load.

## The host bridge: the patch wins

The host sees 72 linear 0–1 parameters with permanent ids. Plain → normalised → plain is not a round trip (33–82 % of values move on Log, Power and signed parameters), so three rules decide when a host value enters the patch (`Session::fold_host`, called once per editor frame and before every save):

1. **The patch wins at load.** The first fold after construction or a load only records the host's values.
2. **A host value is folded only when it changed since the session last saw it.** A stale host value, after a slot reassignment that never reached the host, an editor preset load, or a host that restored no parameters, never overwrites the patch.
3. **And only when it truly differs** (`bridge::fold_value`): if the host's `n` equals `to_norm(p)` bit for bit the patch keeps `p`; otherwise the candidate is the canonical `from_norm(n)`, written (without history) only if it differs from `p` bit for bit. Folding the same value again, or after a save and reload, never moves the patch. Non-finite host values are ignored.

The engine resolves slot events with the same equality rule, and an editor edit updates the slot table's patch value, so the host's echo of an edit (`to_norm` of the same value, the same pinned function) lands on the exact plain value. A fold sends nothing to the engine: the engine already follows the host through slot and macro events from `process()`. Two slots may drive one address; the higher slot wins a fold.

## The saved state, version 1

Two nice-plug persistent fields, both permanent names ([naming.md](naming.md)):

| Field | Text | Versioned by |
| --- | --- | --- |
| `patch` | Exactly `Patch::to_json()`, carried raw (never re-parsed through `serde_json::Value`) | The patch file's `schema` number and `enth_patch::migrate`, the same path as `enth render` |
| `enth` | `{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":false},"ui":{"layout":"lanes","scale_percent":100,"advanced":false}}` | Its `schema` string and `state::INSTANCE_STEPS` |

- The slot assignments and the macros (values and names) live in the patch; the 72 host values are nice-plug's own `params` and are never needed to rebuild the sound.
- Byte-deterministic (clap-validator's `state-reproducibility-binary`); each field at most 1 MiB; the two fields load independently (nice-plug sets `enth` before `patch`).
- Within version 1, a new optional field with a default is allowed (unknown fields are ignored, missing ones default). Anything else is version 2: a step in `INSTANCE_STEPS`, `v2/` fixtures, and the `v1/` fixtures kept as migration inputs. A newer version than the build knows is refused (`schema_too_new`).
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

## What the session relies on from the engine (lane E+, merged at `6e505b8`)

- `Link::{send_plan, send_param, send_depth, send_macro, send_mod_wheel, send_event, collect, epoch, in_flight, free_slots}`, `MAX_PLANS_IN_FLIGHT = 2`, `SendError::{Full, InFlight}` returning the value.
- `PlanMap::{get, depth, addr}` and `DepthId`; `PlanMap::depth` is `None` for a depth-0 route or a route from or to a disabled module, and the session recompiles then.
- `Plan::{tap_params, tap_modulators}` for `TelemetryMap::from_plan`; `Tap::{new, read, frame, scope, generation, sleep, set_enabled, take_wake}`; `Engine::{with_link, attach_tap, tap, epoch, plan, set_monitor_bypass}`.
- The slot table's equality rule and its "a value event updates the slot's patch value" behaviour (the host's echo of an edit is exact). The engine forgets a remembered host slot value when the slot's key changes.
- `EventKind::{Slot, MacroVoice}` and `Engine::reset` for the shell's `process()` and `reset`; `NoteEnd::offset` for `NOTE_END`.
- The monitor bypass switches immediately on a fresh engine, so `activate` sets the persisted value before returning the engine.

## Open questions for the F1b review

1. **A compile still pending when the editor closes.** If the audio thread had not returned a plan when the editor's last frame ran (the host stopped calling `process()`), the coalesced compile waits for the next session entry, and the engine plays the previous plan until then. nice-plug offers no main-thread timer or callback that `process()` could trigger without running inline. Options: a fork patch exposing CLAP `request_callback` with an `on_main_thread` hook that pumps, or accept it and document it.
2. **Stale host slot values after an editor preset load.** The engine re-applies a remembered host slot value after a swap when the slot's target is unchanged, so after an in-editor preset load the old host value could override the new patch's value for that parameter, while the session (rule 2) keeps the patch's. Options: a `Link` command that forgets the remembered slot values, pushing the new values to the host (up to 72 writes, which nih-plug #92 warns against), or loading presets only through host state. P1b has no in-editor preset browser yet.
3. **Host writes from the main thread.** By design none arise there. Lane PL should still confirm that nice-plug's `ParamSetter` from the GUI thread is fine for VST3 in REAPER (VST3 expects `beginEdit` on the UI thread).
4. **Two slots on one address.** The patch allows it and the higher slot wins a fold. Should `SetSlot` refuse a duplicate (an `enth-patch` rule), or should the editor's "Automate" menu only reuse the existing slot (which `bridge::slots_for` supports)?
5. **`modulation_range(patch, addr)`** for the static knob rings (report 02, D9) belongs in `enth-patch`, so the CLI's `describe` can share it. It is not in this API.
6. **The history merge rule.** Report 02 recommends merging `SetDepth` per route and `SetMacro` per index inside a gesture, as `SetParam` per address already is. That change is in `enth-patch::History`; the session needs nothing new for it.
