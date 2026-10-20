# Presets

> Lane PBC of R1/S1 (the preset core, without UI). The browser and the save dialog are lane PBU's ([The browser](#the-browser-lane-pbu)); the factory sounds are lane PR's and AM's. Code: `crates/enth-session/src/presets.rs` (format, index, search, naming, the session's load and save), `crates/enth-plugin/src/presets.rs` (the user folder and the folder store), `crates/enth-plugin/src/crash.rs` (the crash file), `crates/enth-patch/src/diff.rs` (`ops_between`).

A preset is a patch in a file. There is no second format: a preset file is exactly `Patch::to_json()`, the same text the plugin saves in the host's `patch` field and `enth` reads and writes, so any preset is a valid saved state and any saved state is a preset.

## Files and folders

| | |
| --- | --- |
| File | `<name>.patch.json` (`FILE_SUFFIX`). Folders are directories. The name shown is the patch's own `meta.name`; the file name is only the fallback and the identity. |
| Factory presets | `content/presets/**/*.patch.json`. **Embedded**, read-only: `crates/enth-session/build.rs` turns every file under that directory into an `include_str!` at build time, so the plugin needs no folder next to it and a new preset needs no Rust edit, only the file (rebuild picks up adds, edits and removals). Chosen over loading from disk because a plugin has no reliable install location, a friend's bundle is one file, and the cost is about 3 KB a preset. |
| User presets | The user folder (below), any depth up to 8, at most 5000 files, each at most 1 MiB. Dot files, dot folders and links to folders are skipped. A file that is damaged, from a newer schema, too big or not UTF-8 is listed as a **problem** with its key, not hidden. |
| Favorites | `favorites.json` in the user folder (`{"schema":"enth.favorites/1","favorites":["factory:bass/Sub One","user:Mine/Wobble"]}`, sorted). Keys of factory presets can be starred too. A damaged file reads as none. |
| Key | `PresetKey { source, path }`, text form `factory:bass/Sub One` or `user:Mine/Wobble` (`/` separators on every platform, no `..`, no empty parts, no backslash). The key is what the bookmark and the favorites store. |

A factory preset's path is its folder and file name under `content/presets/` without the suffix. Keep the factory folders few and by type (`bass`, `lead`, `pad`, `pluck`, ...).

### The tag vocabulary

`meta.tags` of a factory preset come from a small vocabulary (`enth_session::presets::tags`), checked by `tags::check_factory` and by `cargo test -p enth-plugin --test presets` for every file in `content/presets/`:

- **Exactly one type**: `bass`, `lead`, `pad`, `pluck`, `keys`, `arp`, `fx`, `texture`, `init`.
- **Up to four character words**: `bright`, `dark`, `warm`, `clean`, `gritty`, `hollow`, `metallic`, `noisy`, `soft`, `hard`, `smooth`, `wide`, `mono`, `evolving`, `rhythmic`, `vintage`.
- Lower case, no spaces, no repeats; the patch needs a `meta.name`, at most 64 characters, and no two factory presets share a name (ignoring case).

User presets may carry any tags (tidied on save: trimmed, no repeats ignoring case, at most 12 of at most 24 characters), so the browser's tag chips show whatever is in use (`Library::tags()`, most used first). To extend the vocabulary, edit the two lists in `presets.rs` and this section together.

## The user folder

In this order (`enth_plugin::presets::presets_dir`):

1. `ENTH_PRESETS_DIR`, if set and not empty, as it is (tests, CI, portable setups).
2. The platform's Documents folder, then `enth/Presets`:

   | Platform | Documents folder |
   | --- | --- |
   | Linux | `XDG_DOCUMENTS_DIR` from `$XDG_CONFIG_HOME/user-dirs.dirs` (default `~/.config/user-dirs.dirs`; `$HOME` expanded; a value equal to `$HOME` means "disabled" and is skipped); else the environment variable of that name if absolute; else `~/Documents` |
   | Windows | `%USERPROFILE%\Documents` |
   | macOS | `~/Documents` |

3. Nothing found (no home folder): the store has the factory presets only; saving fails with `no_user_folder`.

The folder is created at the first save, not before. The `dirs` crate was not used: it pulls `option-ext` (MPL-2.0), which `deny.toml` forbids. **Known gap:** a Windows Documents folder redirected by OneDrive or policy (`%USERPROFILE%\OneDrive\Documents`) needs the Known Folder API, which means a `windows`-family dependency; until the owner allows one, `ENTH_PRESETS_DIR` points elsewhere. The same rules give the crash folder (`<Documents>/enth/Crashes`, `ENTH_CRASH_DIR`).

## Loading

`Session::load_preset(&Patch, Option<&PresetKey>)` (also on `SessionEdit`; `enth_session::presets::load_from(session, store, key)` reads the key first). An editor preset load is **one undo step**:

- The step's ops are `enth_patch::ops_between(current, preset)`: remove routes and modules the preset lacks, add the ones the current patch lacks, swap the amplitude envelope, reorder, set values, enabled flags and labels, the built-ins, routes in the preset's summation order, macros, slots, metadata. The list is built so every intermediate patch is valid and is **checked by applying it** (`None`, never a wrong list); two plans are tried (minimal first, then one that takes every route out and puts the preset's back). A property test draws random pairs of valid patches (45000 pairs in three seeds, 256 cases in the gate) and requires the result to equal the target bit for bit. `enth diff` uses the same function.
- The step is recorded as a **load step** (`Entry::load`, the preset's name; `History::edit_load`): undo restores the patch from before the load, redo the preset, each in one step, whatever the number of ops. The redo line is cleared like any edit. An open drag is closed first. The engine stamp (`Patch::engine`) is the session's own, as no op changes it.
- A load of the patch the session already holds changes nothing: no entry, no request, no plan (`PresetLoad::changed == false`). The bookmark still moves.
- The load counts as **saved** (`HistoryView::is_dirty` is false right after it, true after an edit or an undo of it), so a header can show whether the sound still is the preset. The host project is **marked dirty** all the same: the load pushes `ShellRequest::PatchEdited`, which `Shared::settle` turns into `HostRequests::mark_dirty()`.
- **Delivery, for the load and for its undo and redo** (`Session::deliver_load`): one plan compiled from the patch as it is, marked to forget every remembered host slot value; the fold's baseline reset (the next fold only records the host's values, so a stale host value never overwrites the loaded patch); audition notes released; `RenameParams` only if the host would see other parameter names; `PatchEdited`. **No `ShellRequest::Host` write**: the host's 72 parameters are not touched, and a per-parameter write storm on a load crashed VST3 hosts (nih-plug #92). The host's parameter *values* therefore stay where they were until the user or automation moves them; the patch's values are what sounds and what is saved (see "The host bridge" in [session.md](session.md)).
- A preset that is not a valid patch is refused with the validator's reason and nothing changes. The "last loaded" bookmark moves only on success.
- Threads: the GUI thread under the session lock, like any editor edit. Never the audio thread; never `GuiContext::set_state`.

The bookmark is `PresetState { last }` in the `enth` field's `preset` member (`enth.plugin-state/1`: an optional member with a default, absent until a preset is involved, so a state without one keeps its bytes). `InstanceState::preset.last_key()` is where the header's ◀ name ▶ steps from (`presets::step(list, current, ±1)` wraps and works on any filtered list).

### Notes for the UI (PBU)

- The undo step's label is `Entry::load` (`Some(name)`); `enth_ui::describe::entry` does not know it yet and should say "Load <name>".
- `PresetStore` (`scan`, `read_text`, `write_text`, `set_favorite`, `user_location`) is the only I/O seam. `MemStore` serves tests and the dev window; `FsStore::from_env()` is the plugin's. `scan()` is meant for when the browser opens (about 1.4 ms a file).
- `Library::search(&Query)` does the rest: `Query { text, tags, source, folder, favorites_only }`. Free text is words in any order, each matching the **beginning of a word** in the name, tags, folder, author or description (that order of weight; an exact word beats a longer one; ties keep browsing order). Inside the text, `#tag` requires a tag and `by:name` an author, so a one-line search box needs no extra widgets. `tags` are chips (whole tags, all required). With no words the result is the browsing order: factory first, then by folder, then by name, ignoring case.
- Errors are `PresetError` with a stable `code()` and a one-line `hint()`: `no_user_folder`, `bad_preset_name`, `bad_preset_folder`, `preset_exists`, `preset_not_found`, `preset_read_only`, `preset_invalid`, `preset_io`, plus the session's own.

## Saving

`Session::save_preset(&mut dyn PresetStore, &SaveRequest { name, folder, tags, overwrite })` saves the current patch as a user preset.

- The file name is the name made safe on every platform (`file_stem`: forbidden characters and control characters become `_`, runs of spaces collapse, leading and trailing dots and spaces go, Windows device names like `CON` get a `_`, at most 80 characters); the folder is cleaned part by part and cannot contain `..`. A name of more than 64 characters, or one with nothing usable in it, is refused (`bad_preset_name`).
- **Overwrite protection:** a name that exists, ignoring case (a case-insensitive file system would say so anyway), is `preset_exists` unless `overwrite` is set. The factory presets are read-only (`preset_read_only`).
- The write is atomic (`FsStore`): a dot temporary file beside the target, `sync_all`, then a rename; the temporary file is removed if anything fails. Two processes saving the same new name at the same moment can still race between the existence check and the rename (a hard link would close it, but not every file system has them).
- The file is exactly `Patch::to_json()` of the patch **with the saved name and tags** (the author and description are the patch's own; nothing is invented). Nothing changes unless the write succeeds. On success the patch takes the name and tags as one `SetMeta` edit (undoable, skipped if nothing differs), counts as saved, and the bookmark moves to the new key.

## The crash file

`enth_plugin::crash`, installed from `Plugin::setup_logger` when the library loads (that function returns `Some(false)`, which stops nice-plug installing a subscriber or its own `log_panics`, which would replace this hook). **Local only: no network, no signal handlers, no uploads.**

- The hook chains to the one it replaces (the host's or the default), so nothing printed before is lost. It writes only for enth's panics: when the backtrace has symbols and none mentions `enth`, the panic belongs to another plugin in the same host and is left alone; with no symbols it writes anyway.
- `enth-crash-<UTC date>-<UTC time>-<counter>-<process id>.txt` in `ENTH_CRASH_DIR` or `<Documents>/enth/Crashes`: version and profile, time (UTC and unix), platform, thread name and id, location, message, backtrace. **No personal data**: no patch, project, host or environment, and the home folder is replaced by `~` everywhere in the text, including in the message.
- Bounded: three files per process, nothing once the folder holds 50, a report cut at 64 KiB. Nothing is ever deleted.
- **Real time:** installing allocates once, on the host's main thread at library load; the non-panic path runs none of this code. In a panic (possibly on the audio thread) the hook allocates inside `nice_plug::util::permit_alloc`, as a debug build's `assert_process_allocs` would abort otherwise.
- For the editor, on the next load: `crash::pending(dir)` lists the files not yet offered, `read_report(path)` reads one, `mark_seen(path)` renames it to `….seen.txt` (kept, offered once). The editor's offer is lane PBU's ([below](#the-crash-offer)).

## The browser (lane PBU)

Code: `crates/enth-ui/src/editor/browser.rs` and `browser/{header,panel,save,crash}.rs` (the editor's side is in [editor.md](editor.md#the-preset-browser-lane-pbu)); the plugin's wiring in `crates/enth-plugin/src/editor.rs`. Every load below is `SessionEdit::load_preset` (one undo step, no host parameter writes, the project marked dirty); every save is `SessionEdit::save_preset`.

**The header: "◀ Name ▶ Browse Save"**, right after the wordmark.

- The name is the bookmarked preset's (`InstanceState::preset.last_key()`: its title from the last scan, else its file name), else the patch's own, else "Untitled". A dot follows it while the sound differs from that preset (`HistoryView::is_dirty`: after an edit, or after an undo of the load). Clicking the name opens or closes the browser, like Browse.
- ◀ and ▶ load the previous and the next preset of the browser's current list (its tab and its search; `presets::step`), wrapping round; with no bookmark, or one the list does not hold, ▶ gives the first and ◀ the last. Each step is one undo step. A file that vanished since the scan opens the panel on a fresh scan, with the reason.
- **Known limit:** the dot follows the session's history, which starts again when the host loads a project; a sound edited and saved with the project shows no dot after a reload, until it is edited again.

**The browse panel**, over the body in any layout (the regions under it are not drawn, so the frame gets cheaper):

- Tabs: All (factory first, then the user's), Factory, User, Favorites. A search field takes words in any order, `#tag` and `by:author` (`Library::search`); it is never focused by itself, because inside a host the keyboard reaches the plugin only while a field is typed in (fork patch 4), so the host's shortcuts keep working while browsing.
- A row is ☆ (★ when starred), ▶, the name, where it lives ("Factory › bass" in All and Favorites, the folder otherwise), the tags and the author; the description is its tooltip, and the loaded preset's row is highlighted. A click loads it; a double-click loads it and closes the panel; Escape and × close it. The footer shows the last message (an error with its hint, in red) and "N files could not be read", with the files and the reasons as a tooltip.
- **The scan** (`PresetStore::scan`) runs every time the panel opens, and on the first ◀ ▶ when nothing was scanned yet, on the GUI thread inside the session lock (the plugin draws under it). A big user folder makes that one frame slow; it is not measured, and moving the scan before the lock is a follow-up.
- **Favorites:** ☆ calls `PresetStore::set_favorite` (saved at once; a refusal is the footer's message); the browser keeps its own copy, since `Library` keeps the favorites it was built with.
- **Audition:** ▶ held loads the preset (unless it is the loaded one), then plays middle C at velocity 100 through the session's audition notes until the pointer is released anywhere, or the window closes (the shell's `audition_all_off`). A session with no engine says why in the footer. REAPER keeps calling `process()` with the transport stopped, so it sounds there (measured by `preset-load`: a peak of about 0.67 on CLAP and on VST3 with ▶ held, the transport stopped); other hosts are still the open check of [phase-2.md](phase-2.md#open-checks).

**The Save dialog**, a card over the body:

- Filled from the patch: its name, its tags, and the folder of the user preset it came from. A folder may be typed (made if new); the factory presets are never offered (they are read-only).
- Tags from the vocabulary: one type (a second click clears it), up to four character words; the patch's other tags are kept and offered beside them.
- "Save preset" or Enter saves. A name in use (`preset_exists`) is never replaced silently: the dialog asks "“Name” is already in your presets. Replace it?", and Replace saves again with `overwrite`; Cancel goes back to the fields. Any other refusal shows its message and its `hint()` (`bad_preset_name`, `bad_preset_folder`, `no_user_folder`, `preset_io`, …). Escape or Cancel closes it. Opened from the panel, the panel comes back after a save, rescanned (on the User tab if it was on Factory).
- With no user folder the dialog says so in red ("set ENTH_PRESETS_DIR").

**The undo step's name** is "Load <name>" (`describe::entry` on `Entry::load`; "Load preset" for an unnamed one), so the header's Undo tooltip reads "Undo Load Glass Pad".

**Stores.** The plugin hands the editor `FsStore::from_env()` when the host creates the editor; the editor's default is `MemStore::default()` (the embedded factory presets, no user files), which the dev window and the tests use.

### The crash offer

At every open of the window the plugin lists the crash folder's pending files (`crash::pending`) and hands them to the editor as a `CrashReports` (`CrashFolder` in `enth-plugin/src/editor.rs`; the UI crate does no I/O). While some wait, a small notice sits in the body's lower right corner: "enth stopped unexpectedly last time. It wrote a report on this computer. Nothing was sent anywhere." with Show report and Dismiss. It is a foreground area, not a modal: the editor works beside and under it. Either button marks every pending file as offered (`mark_seen`, a rename; nothing is deleted), so each is offered once. Show report puts the newest report over the body, with where the files are, Copy report (to the clipboard) and Close report.

## Tests and gates

| What | Where |
| --- | --- |
| `ops_between`: random pairs, hand-built hazards (scope change under a kept route, full `modulators`), a load step undoes and redoes in one step | `crates/enth-patch/tests/diff.rs` |
| Format fixtures (`tests/fixtures/presets/**`, exactly `to_json()`, regenerated by `cargo test -p enth-session --test presets -- --ignored regenerate`), loading through state byte for byte, index, search, tags, favorites, naming, `MemStore`, the bookmark | `crates/enth-session/tests/presets.rs` |
| One undo step; no host writes and `PatchEdited` on load, undo and redo; one plan each; the engine plays the values; stale host values; identical preset; drags; invalid presets; dirty flag; host state load after a preset load | `crates/enth-session/src/tests/presets.rs` |
| Every user-folder branch with fake environments; `FsStore` on temporary folders; a preset load, undo and redo through `Shared::with_session` mark the project dirty once each and write nothing to the host | `crates/enth-plugin/src/presets.rs` |
| The panic hook, in a test binary of its own | `crates/enth-plugin/tests/crash.rs`, `src/crash.rs` |
| Every file in `content/presets/` (recursively) is `to_json()`, uses the vocabulary, has its own name and is embedded | `crates/enth-plugin/tests/presets.rs` |
| (PBU) The browser through a real session by kittest: browse, search, tabs, favorites, a click and a double-click load, ◀ ▶, the edited dot, the Save dialog (tags, confirm, refusals with hints), audition, the crash notice; snapshots of the header, the panel and the dialog | `crates/enth-ui/tests/browser.rs`, `tests/browser_snapshots.rs` |
| (PBU) The browser over `FsStore` on a temporary folder; the crash folder offered once | `crates/enth-plugin/src/editor.rs` |
| (PBU) REAPER `preset-load`, CLAP and VST3: a user preset loaded with real clicks through the browser; the bookmark saved; no host parameter moves on the load, its undo or its redo; the project dirty; one editor Undo and Redo; ▶ held with the transport stopped measured | `cargo xtask reaper-test --scenario preset-load` (`xtask/reaper/preset.lua`) |
