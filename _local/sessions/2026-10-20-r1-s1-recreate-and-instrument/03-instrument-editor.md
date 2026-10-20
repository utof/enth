# 03: Instrument 0.1, the editor and its content

- **Sub-researcher:** 03 of the R1/S1 session ([brief](brief.md)). **Date:** 2026-10-20.
- **Code read:** main at `4cd740a` plus the working tree (INT merged), through the codebase-memory graph (`home-vboxuser-code-enth`, no parse gaps) and Read/Grep.
- **Scratch:** `~/code/enth-scratch/2026-10-20-r1-s1/03/` (`notes.md`, `layouts/`, and `minsize/`: an egui_kittest harness outside the repo that prints `Editor::report()` for any layout, patch and window size, with no renderer).
- **Evidence tags:** *measured* (I ran it), *documented* (a vendor doc, changelog, header, repo doc or code says so), *anecdotal* (forum or secondary write-up).

## 1. Summary

The patch model already has every structural op: `AddModule`, `RemoveModule` and `Move`, each with an exact inverse. Only the editor's intents are missing, so add/remove is a small editor lane.

The S1 decisions that matter are three:

- **What a fresh instance plays.** Recommendation: the starter patch, as "Init".
- **Whether an editor preset load is undoable.** Recommendation: yes, built on the CLI's verified `diff_ops`.
- **Where Recreate and Learn go in 1200 × 800.** Measured: the challenge bar shares the header row, and the results drawer takes the overview slot.

GS1 asks for presets "you'd actually use". So agents draft them and the owner picks.

| Area | Recommendation | Evidence | Confidence |
| --- | --- | --- | --- |
| Add / remove modules | Intents `AddModule`, `RemoveModule`, `MoveModule`, `Duplicate`, `Rename`, one op and one undo step each. Entry points: a card "⋯" menu, right-click on empty region space, and a dashed "+ Add" slot where there is room | `enth-patch/src/ops.rs` (code); Phase Plant's dotted area, Bitwig Grid's palette and right-click (documented) | High |
| Init patch | A fresh plugin plays the starter patch as "Init": every P1 module, with osc 2, noise, comp and reverb off. `Patch::init()` stays minimal for the CLI and tests | INT's open check; disabled modules are absent from the plan and from `canonical_hash` (documented) | Medium-high |
| Preset format | Exactly `Patch::to_json()` in `<name>.patch.json`, the plugin's own `patch` field. Directories are folders; `meta.tags` uses a vocabulary for factory presets | `tests/presets.rs` (code) | High |
| Browser UI | Header gets ◀ name ▶, Browse and Save. The Browse panel covers the body and is not part of the layout. Factory, User and Favorites; word search; click loads; ▶ audition | Kilohearts' browser, Serum 2's preview (documented) | Medium |
| Load semantics | An editor load is one undo step: `ops_between(current, preset)`, delivered like a load (no host writes), and it marks the project dirty | `enth-cli/src/cmd/edit.rs:291` `diff_ops`, verified by applying (code) | Medium |
| User folder | `Documents/enth/Presets` on every OS, through `dirs` 7.0.0, with an `ENTH_PRESETS_DIR` override | dirs 7.0.0, 2026-09-05, MIT/Apache-2.0 (LICENSE files read) | Medium |
| DAW interplay | REAPER's FX presets already work. CLAP `preset-discovery` and `preset-load` are a later lane-FK patch | REAPER 7.68 (2026-04-09) loads CLAP factory presets (documented); nice-plug lacks them, issue #73 open | Medium |
| Ten presets | 4 from G1; about 20 agent drafts from the CLI, normalised by a new `xtask preset-add`; the owner keeps 6 | Agent eval v2: 30/30 with the expectations given, 15/30 when self-written (brief) | Medium |
| LV intake | Send Claude Design the `LayoutSpec` vocabulary. Every variant gets snapshots and an overflow report at 1200 × 800 and 1000 × 680 | Measured below | Medium |
| Recreate / Learn | A mode is a layout plus an `Overlay`. Bar in the header row, results in the overview slot. Lesson pane 320 px, with the overlay hiding unused modules | `layouts/minsize.txt` (measured) | Medium-high |
| Poly-mod | Implemented and test-host-checked. Bitwig 6.1.4 owner check of about 20 min; skip save and reload | Code; Bitwig trial needs an account (documented) | Medium |
| UI thresholds | baseview paces at X Present `msc + 2`: ask for 1/60 s while live data changes, and count frames. Also fix the 1000 × 680 minimum, which drops the overview | Vendored baseview code; Phase 0 fps (documented); overflow (measured) | Medium |

## 2. Findings

### 2.1 Adding and removing modules

**What exists** (code):

- `enth_patch::Op` has `AddModule { at, module }`, `RemoveModule { id }`, `Move { id, to }`, `SetLabel` and `SetAmpEnv`.
- `remove_module` refuses the amp envelope (`InUseAsAmpEnv`). Its inverse re-adds the module and restores its routes and slot assignments at their original indices.
- `Patch::free_id(kind)` gives the next id. The limits are 4 generators, 4 voice effects, 4 effects, 8 modulators and 64 routes (`model.rs:39-47`).
- `editor/describe.rs` already names "add osc2" undo steps.
- `Intent` (`editor/cards/intent.rs:34-70`) has no add, remove or move.
- `CardRegion::show` returns card headers "for a later drag-to-reorder", which nothing uses.
- There is no kind-change op.

**Prior art:**

- **Phase Plant** (Kilohearts docs, undated; documented):
  - add a generator: "click in the dotted area in the unused generator space", then pick a type;
  - add a snapin: click a lane's empty space or between existing snapins;
  - add a modulator: click the dotted area of the bottom lane;
  - drag to move, Ctrl-drag to copy;
  - modulate: "+" on the source, then drag; double-click unroutes.
- **Bitwig Grid** (Bitwig tutorial; documented): drag from a palette or right-click empty space; drop on a module to replace it, keeping similar parameters; Delete removes.
- **Vital** (secondary guide; anecdotal) has fixed slots: 3 oscillators, a sampler and 2 filters, plus a fixed effects rack you toggle and reorder.

**Design** (independent of the layout, since intents carry ids):

- **Intents to ops.**
  - `AddModule { section, kind, index }` → `Op::AddModule` at `free_id`.
  - `RemoveModule { id }`.
  - `MoveModule { id, to }`.
  - `Duplicate { id }`: a clone with a new id, right after the original.
  - `Rename { id, label }` → `SetLabel`.
  - `MakeAmpEnv { id }` → `SetAmpEnv`.

  Each is one `session.edit(ops, None)`. Refusals (`TooMany`, `InUseAsAmpEnv`) already show as status-line hints.
- **The card "⋯" menu:** Add after ▸ (kinds valid for the section), Duplicate, Move up/down, Rename…, Make amp envelope, Remove. Right-click on empty region space opens Add.
- **A dashed ghost slot** appears while a section is under its limit. *Measured:* at 1200 × 800 the P1 default's sources region holds exactly three 96 px strips (304 px), so a ghost slot there would scroll. Show it only where there is room.
- **The status line reports what a removal took with it** ("Removed Filter 1, 2 routes and automation slot 3"). The bound host lane then shows "Slot 3", through the existing rescan.
- **Reorder through the menu first.** Order matters in the serial sections (`voice_fx`, `fx`); in `generators` it only changes the summation order.
- **Cost.** Every add, remove or enable recompiles. RT reported the crossfading swap block at p99 337–339 µs, 25.2–25.4 % of the deadline (documented, not gated). Watch it in the 1-hour soak.
- **Defer kind replacement.**
- **Polish bug** (measured from `editor_lanes_max.png`): at the maximum patch, Osc 3/4, Filter 2 and Env 3+ are scrolled out of view with no visible cue. Add an edge fade or a "+2 more" hint.

### 2.2 A richer init patch

**Today:** a fresh instance plays `Session::default()` = `Patch::init()`, one saw and one envelope (`enth-session/src/model.rs:140`, plugin `lib.rs:248`). `xtask/src/preset/starter.patch.json` is already the right shape.

**Recommendation:**

- Ship the starter as the factory preset "Init".
- Make it the plugin's starting patch at the plugin level.
- Keep `Patch::init()` for the CLI `init` template, the fixtures and the tests.

**Why:**

- Newcomers get Vital's toggle-a-slot pattern; power users still have add/remove.
- It costs no CPU: a disabled module is not in the plan (session.md).
- It costs no structure score in Recreate: `canonical_hash` drops disabled modules (design.md §2).

**Before switching:**

- Fix the `ENGINE_VERSION` stamp INT found ("0.1.0" vs "0.2.0"), or every saved fresh instance carries the wrong one.
- Update the REAPER scenarios and docs that assume a fresh instance is `Patch::init()`. `dirty-flag`'s click on osc 1 Level at (366, 178) does not move.

### 2.3 The preset browser

**Format and scale.** One patch document per file, exactly `Patch::to_json()`. `PatchMeta` already has name, author, description and tags, so no index file is needed at S1 sizes. *Measured:* `enth get <starter> meta` takes 1.36 ms per process, start-up included, at load ~8. So 1,000 user presets scan in under about 1.4 s; scan lazily when the browser opens.

**Folders and tags:**

- Factory presets are embedded, read-only, with subfolders by type. User presets are writable.
- Favorites is a list of keys, not a folder.
- Factory tags: one type (`bass`, `lead`, `pad`, `pluck`, `keys`, `arp`, `fx`, `texture`) plus character words, checked by `tests/presets.rs`. User tags are free.
- Kilohearts turns `#words` in descriptions into tags. Pigments filters by types and styles (manual table of contents; documented).

**Behaviour** (Kilohearts' browser, documented):

- Search matches words of the name, description, author and tags, in any order; "by:" filters by author.
- Click loads; double-click loads and closes; Escape closes.
- ◀ ▶ step through the current list.
- Save never offers read-only folders and can create a subfolder. "New" loads Init.

**Audition.** Serum 2 plays an embedded MIDI clip or a fallback clip (documented). enth's session audition notes can play a phrase by type while the transport is stopped. Open check: nice-plug never calls `request_process`, so a host that sleeps the plugin may stay silent.

**Placement:**

- The header shows the patch name centred, with spare width, so it becomes "◀ Name ▶ Browse Save".
- The panel is editor view state drawn over the body. It works in every layout, and while open it replaces the cards, so the frame cost goes down.
- The search field relies on the keyboard-focus fork patch inside REAPER (owner check 1.5). Arrow keys may reach the host, so the ◀ ▶ buttons come first.

**Load semantics** (the key decision):

- Today an editor load is `Session::load_patch`. A preset is a state the history does not know, so the load **clears undo** (session.md, "reconcile"), and one mis-click in the browser loses the user's work.
- Recommendation: `Session::load_preset(&Patch)` records one history entry from `ops_between(current, preset)`.
  - This is the CLI's `diff_ops`: routes out, modules out (amp env last), modules in, verified by applying.
  - Move it to `enth_patch::diff`.
- **Caveat:** the entry, and its undo and redo, must be delivered like a load: one plan with `forget_host_slots(all)`, a re-baseline and a rename, and **no** per-macro or per-slot host gestures. A naive `edit` could push up to 72 begin/set/end writes, the nih-plug #92 crash class (design.md §4).
- It must also mark the project dirty (open in plugin.md).

**Where user presets live:**

- Steinberg's VST3 user paths are `~/.vst3/presets/$COMPANY/$PLUGIN/` on Linux (hidden), `~/Library/Audio/Presets/…` on macOS and `Documents/VST3 Presets/…` on Windows (documented). They are meant for `.vstpreset` files.
- Vital (forum; anecdotal) and Kilohearts use visible user folders.
- Recommendation: `dirs::document_dir()/enth/Presets/` with `ENTH_PRESETS_DIR`; "Add location" later. `dirs` 7.0.0: LICENSE-MIT and LICENSE-APACHE read, one dependency (`dirs-sys`). File I/O stays out of core crates (2.9).

**DAW interplay:**

- REAPER's FX preset menu stores the whole state, which already round-trips byte for byte (G1 row 5). A REAPER preset load reconciles or clears the history like any host load.
- REAPER 7.68 (2026-04-09) added CLAP "support loading factory presets" and "saving/loading presets to file"; 7.80 (2026-09-13) links MIDI program change to factory presets (whatsnew.txt; documented).
- So a lane-FK fork patch could list enth's factory presets in REAPER's and Bitwig's own browsers through CLAP `preset-discovery`:
  - factory id `clap.preset-discovery-factory/2`;
  - factory, user and favourite flags;
  - `LOCATION_PLUGIN` for embedded presets.

  It also needs `preset-load` (main thread → `load_patch`). nice-plug has neither (no match in `vendor/`). Its issue #73, "Better preset management", has asked for this since 2026-08-18. A stretch goal, not GS1.
- `.vstpreset` export is deferred.

### 2.4 Ten factory presets

| Source | Count | How |
| --- | --- | --- |
| Owner, G1 build 2 | 4 (bass, pad, pluck, lead) | Starter project, then `cargo xtask extract-presets` (exists, tested) |
| Agents, through the CLI | ~20 drafts | A brief per preset (name, tags, one-line sound, macro roles), then the eval-v2 loop: `new --template`, `apply`, `render`, `describe`, `check` |
| Owner pass | keeps 6 (60–90 min) | Audition the drafts in the browser's "Drafts" folder, tweak, save, extract |

**Gaps in the pipeline** (code):

- The CLI writes pretty JSON stamped with `file::ENGINE_VERSION` (`patchio.rs:58`), and `tests/presets.rs` rejects anything that is not `Patch::to_json()`. Add `cargo xtask preset-add FILE --name --tags`: load, set meta, stamp, write `to_json()`.
- Adopt the G1 reference's macro names as the factory convention: 1–4 are Bright, Pluck, Space, Drive. That also gives Recreate stable handles.
- Cover the module set with the six: keys, a sync lead, an arp pluck (LFO), a sub bass, a noise texture, and a stab with the compressor.
- No third-party preset files.

### 2.5 Polishing lanes; how LV variants arrive

**Measured at 1200 × 800 with the G1 patch:** nothing overflows. The overview gets 956 × 98, the band 956 × 224 with about 50 px unused, the noise strip leaves about 430 px empty, and 8 macros spread over 1,184 px.

**Measured at the documented minimum, 1000 × 680:**

- Lanes has **no overview region**: the source tiles and the route list vanish, so tiles cannot be dragged.
- The band scrolls 148 × 21 px and the effects lane 98 px.
- Sources is 528 px wide, while the osc strip's content reaches about 625 px in the snapshot. Its scroll area is vertical-only, so the last knobs are probably clipped (Gaps).

**Recommendations:**

- Hold layout polish for the owner's pick (editor.md gives hypotheses 4, 5 and 8 to LV).
- Until then, do only what every variant needs:
  - add/remove;
  - the header: browser, mode tabs, UI scale;
  - card menus;
  - scroll cues;
  - a fix for the minimum size: a compact layout, or a raised `size.window.min`.

**Intake.** The handoff asks for mocks and px sizes, which are not JSON.

- Send Claude Design the `enth.layout/1` regions, parts, `arrange` values and token names with their values, and ask for each variant as a region table in those terms.
- LV then writes the JSON, renders kittest snapshots (P1 default and maximum), runs the overflow report at both sizes, and puts them side by side with the four G1 sounds loaded.
- Use the scratch harness's method for the overflow report: the dev window sizes its window from `Theme::dark()` and ignores `--theme`.
- Directions A and C fit today's schema. B needs a new `Region`, as editor.md foresees.

### 2.6 How Recreate and Learn present

**The mechanism already exists:**

- `Overlay` gives emphasis per parameter and module, plus score rings with ✓ ~ ✗.
- The reserved regions `challenge_bar`, `results` and `lesson` sit behind features.
- Stable ids keep a drag alive across a layout switch (tested).

So **a mode is a layout file plus an overlay**, picked by header tabs (Play, Recreate, Learn; only in builds with the feature).

Measured with the G1 patch (`layouts/minsize.txt`):

| Candidate | 1200 × 800 overflow | 1000 × 680 overflow |
| --- | --- | --- |
| lanes (Play) | none | band 148 × 21, effects 98, no overview |
| Challenge bar as a 40 px row under the header | tiles 39, routes 47, effects 26 | band 148 × 69, effects 146 |
| Challenge bar sharing the header row (480 / 480) | **none** | as lanes |
| The same, with results in the overview slot | **none** | as lanes |
| Lesson pane (320) in the effects column; voice and fx in the band | band 732 sideways, tiles 51, routes 63 | band 932 |
| Lesson pane as a left column | band 276, tiles 111, routes 63 | band 476, effects 98 |
| Both lesson layouts with `init` or `pluck_1osc` | **none** (`pad_2osc`: band 48 in the right-column one) | not run |

So:

- **Recreate's bar shares the header row.** The patch name hides; the target is unknown anyway.
- **The results drawer replaces the overview after Submit.** It shows the scores, the time, and Retry / New / Show target. The per-control rings with arrows draw on the knobs through `Overlay`.
- **Learn works because steps hide what they don't use.** A lesson step names its layout and its overlay.
- Round flow is report 01's; score content is report 02's.

### 2.7 CLAP poly-mod on macros

**Implemented** (code):

- Macros carry `poly_modulation_id` 0–7 (`params.rs:65`).
- `PolyModulation` becomes `EventKind::MacroVoice` keyed by the host's note id (`notes.rs`); `MonoAutomation` is ignored.
- `tests_host.rs:759` checks that only that voice moves and that its `NOTE_END` carries the id.

**Hosts:**

- Bitwig documents CLAP poly-mod.
- REAPER's whatsnew.txt (to v7.82, 2026-10-04) has no CLAP polyphonic modulation entry (measured, grep).
- cleveraudio.org's host list (Nov 2024) and clapdb do not record the feature per host.

**Bitwig 6.1.4** (try page, 2026-10-20): the trial needs "Sign up"; the Linux build needs Ubuntu 24.04+ or Flatpak, and AVX2. Bitwig's support page says demo mode, with save and export disabled, remains after the trial.

**Owner recipe, about 20 min:**

1. Put a per-voice modulator on enth and map it to Macro 1, with Macro 1 routed to cutoff in the patch.
2. Play a chord: each voice should differ.
3. Release the notes and check that no voice hangs.

Save and reload adds nothing, since the modulator lives in Bitwig's project. Whether demo mode runs without an account is unverified (Gaps).

### 2.8 UI thresholds on the real display

**How baseview paces frames** (code: vendored baseview 0.4.1, `platform/x11/event_loop.rs`, `window_shared.rs`):

- A redraw requests an X Present notify at `last_msc + 2`, so at most every other vblank: 30 fps at 60 Hz.
- A 15 ms fallback timer covers servers without Present.
- `request_redraw_after(d)` waits `d` and then the next tick. So 1/30 s gives about 20–24 fps (Phase 0 measured 19.5), and 1/60 s gives 28.6–29.6 (documented).

**Recommendations:**

1. While visible live data changes, ask for `motion.scope_interval()` (1/60 s). The exact-delay tests then expect 16.67 ms. Check CPU against owner check 1.3 (< 5 % of a core).
2. Count drawn frames in the `editor-counts` feature, so `leak-test` (Xvfb) and the owner's REAPER run (`ENTH_EDITOR_COUNTS`) report achieved fps.
3. **HiDPI:** baseview #359 has been open since 2026-10-15, and PR #357 (open) moves X11 off its thread. Keep owner check 1.6. A header UI-scale menu (`scale_percent` already exists in `enth.plugin-state/1`) is the fallback; editor size persistence is still an open check.
4. **Frame budget:** the worst case is 9.43 M of the 10 M limit, so S1 additions to the always-drawn body have about 6 % to spare. The browser replaces the body while open, so it does not add.

### 2.9 Code structure

| Where | What |
| --- | --- |
| `enth-patch/src/diff.rs` (L1) | `ops_between(a, b) -> Option<Vec<Op>>`, moved from `enth-cli`; property test: apply(a, ops) == b |
| New `enth-content` (L2, core rules) | `FACTORY` presets (`include_str!`), `TEMPLATES` (moved from `enth-cli`), `TagVocabulary`, `Library`, `PresetEntry { source, key, meta }`, `Query`, `search()`. At L2 so `enth-challenge` (L4, priors "fitted to enth's own presets", design.md §9), the CLI and the UI can all use it; a test checks the list against the directory; one `layers.toml` line |
| `enth-session` (L6) | `Session::load_preset(&Patch)`: one history entry with load-style delivery; `PatchEdited` |
| `enth-ui/src/editor/structure.rs`, `cards/intent.rs` | Add/remove/move menus, the ghost slot, and their intents; tokens in `theme/sizes/editor.rs` |
| `enth-ui/src/editor/browser/` | `Browser` view state, header stepping, the Save dialog, a `PresetStore` trait; `FsStore` (std::fs + dirs) behind a feature, shared by the plugin, the dev window and the standalone (or in `enth-plugin` if L7 should do no I/O) |
| `enth-ui/src/editor/layout/` | `lanes-recreate`, `lanes-results`, `lanes-learn` (candidates in scratch `layouts/`) |
| `enth-plugin` | Default patch = factory "Init"; frame counter in `editor_counts.rs`; later the FK CLAP preset patch |
| `xtask/src/preset.rs` | `preset-add`, next to `extract-presets` |

## 3. Risks and open questions

1. **An undoable load changes the session's host contract.** A naive `edit(diff)` makes up to 72 host writes. Give it its own lane, with REAPER `undo-state-load` and `patch-roundtrip` scenarios.
2. **Changing the default patch** moves fixtures, REAPER scenarios and owner-sheet text. Fix the engine stamp first.
3. **The 1000 × 680 minimum hides the overview** (measured): handle it in the variants, or raise the minimum.
4. **Search and arrow keys** depend on fork patch 4 in REAPER's windows (owner check 1.5).
5. **Audition from the browser** may be silent while a host sleeps a stopped plugin.
6. **Owner time:** preset pick 60–90 min, layout pick 20–30, Bitwig 20. Report 05 schedules them.
7. **For the owner to decide:**
   - Init = the starter, or small plus add buttons?
   - Is a preset load an undo step?
   - User folder in Documents (visible) or XDG data (hidden)?
   - Raise the minimum window?

## 4. Gaps

- **Not verified:** the add/remove flows of Serum 2 and Pigments, and Pigments' browser beyond its table of contents.
- **Not documented:** Phase Plant's preset file location and extension.
- **No primary source:** CLAP poly-mod in hosts other than Bitwig (FL Studio, MultitrackStudio, Studio One).
- **Unverified:** Bitwig 6.1.4's demo mode without an account. The only evidence is the post-trial note on its support page and a secondhand KVR quote.
- **Inferred, not rendered:** the osc strip clipping at 528 px width. Needs a kittest snapshot at 1000 × 680.
- **Not traced on a real display:** the baseview pacing analysis comes from reading code. The 1/60 s fix is unmeasured until the frame counter exists.
- **Not run:** REAPER's FX preset menu with enth, or a CLAP preset-discovery prototype. Run both before lane FK starts.

## 5. Sources

All read 2026-10-20 unless dated otherwise.

- **Repo** (`4cd740a` plus the working tree):
  - docs: `docs/{editor,session,plugin,design,p1b-owner,p1b-g1,phase-1b,benchmarks}.md`, `docs/handoff/claude-design-layout.md`;
  - code: `crates/enth-patch/src/{ops,model}.rs`, `crates/enth-ui/src/editor/{frame,overlay,reserved,describe}.rs`, `regions/{cards,header}.rs`, `cards/intent.rs`, `crates/enth-plugin/src/{lib,params,notes,tests_host}.rs`, `tests/presets.rs`, `crates/enth-cli/src/{cmd/edit,patchio,templates}.rs`, `xtask/src/preset/starter.patch.json`, `vendor/baseview/src/platform/x11/{event_loop,window_shared}.rs`;
  - snapshots: `editor_lanes_{p1,max}.png`.
- **Scratch measurements:** `layouts/minsize.txt` (the `minsize/` harness, release build); `enth get` timing.
- **Kilohearts:** [Phase Plant](https://kilohearts.com/docs/phase_plant), [Modulation](https://kilohearts.com/docs/modulation), [Preset Browser](https://kilohearts.com/docs/presets) (undated).
- **Bitwig:** [Grid tutorial](https://www.bitwig.com/en/community/learning/grid-tutorial.html), [Trial](https://bitwig.com/support/shop_license_activation/48), [Try page, 6.1.4](https://www.bitwig.com/try/); [Sound on Sound, Bitwig Studio 6 launch, 2026-03-11](https://www.soundonsound.com/news/bitwig-studio-6-launches) (seen as a search snippet).
- **Xfer:** [Serum 2, Loading a Serum Preset](https://xferrecords.com/web-manual/serum-2/loading-a-serum-preset).
- **Pigments:** [manual table of contents, mans.io](https://mans.io/files/viewer/2161698/193) (anecdotal mirror).
- **Vital:** [MODE Audio guide](https://modeaudio.com/magazine/a-guide-to-the-vital-soft-synth), [Vital forum](https://forum.vital.audio/t/ability-to-change-vital-presets-folder-location/14512) (anecdotal).
- **Steinberg:** [VST3 preset locations](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/Locations+Format/Preset+Locations.html).
- **CLAP:** [preset-discovery.h](https://github.com/free-audio/clap/blob/main/include/clap/factory/preset-discovery.h), [host list (Nov 2024)](https://cleveraudio.org/hosts-and-plug-ins/), [clapdb](https://clapdb.tech/).
- **REAPER:** [whatsnew.txt](https://www.reaper.fm/whatsnew.txt) (7.68: 2026-04-09; 7.80: 2026-09-13; 7.82: 2026-10-04).
- **nice-plug:** [issue #73](https://codeberg.org/RustAudio/nice-plug/issues/73) (open, 2026-08-18).
- **baseview:** [#359](https://github.com/RustAudio/baseview/issues/359) (open, 2026-10-15), [PR #357](https://github.com/RustAudio/baseview/pull/357) (open, updated 2026-10-19).
- **dirs 7.0.0** (2026-09-05): [crates.io](https://crates.io/crates/dirs), [repo with LICENSE files](https://codeberg.org/dirs/dirs-rs).
