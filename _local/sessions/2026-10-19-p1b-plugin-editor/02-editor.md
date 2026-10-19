# 02 — Editor (P1b)

- **Date:** 2026-10-19
- **Sub-researcher:** 02 (editor): wiring `enth-ui` to the session, engine → UI feedback, repaint gating, interactions, layout flexibility, Recreate/Learn hooks, tests.
- **Scratch:** `~/code/enth-scratch/2026-10-19-p1b/02/` — running notes in `notes.md`, the prototype in `proto/` (path dependencies on the repo's `enth-ui`, `enth-patch`, `enth-params`; nothing in the repo was changed), raw callgrind and render output in `runs/`.
- **Evidence markers:** *measured* = run here on 2026-10-19 (prototype, kittest, callgrind, lavapipe); *documented* = source code of the pinned crates, a repo, an issue tracker or official docs, read on 2026-10-19; *anecdotal* = a forum post or a user report.
- **Conditions:** builds with `CARGO_BUILD_JOBS=3` under `flock -s /tmp/enth-quiet.lock`; load average 11–22 (other researchers), so wall-clock times are information only and instruction counts are the comparable numbers. No window was opened anywhere: interaction tests run without a renderer, the two PNGs were rendered on lavapipe.

## Answer in brief

1. **Keep the editor inside `enth-ui` (L7), in five modules: `theme` and `widgets` (as today, no patch types), plus new `layout` (data), `view` (view-model) and `editor` (regions, cards, interactions).** A separate `enth-editor` crate can't depend on `enth-ui`: `check-layers` refuses same-layer edges (`xtask/layers.toml`). The view-model is rebuilt from the patch and the catalogue every frame. For the G1 patch (59 controls) that costs **0.23 M instructions, about 5 % of a frame** *(measured)*, so no cache is needed.
2. **Widgets emit intents; the editor turns them into ops at the end of the frame through the session.**
   - A knob drag is one `History` gesture: in the prototype one drag gave **one undo entry with one forward op**, and Cmd+Z / Cmd+Shift+Z undid and redid it *(measured)*.
   - The session opens host gestures lazily for addresses bound to an automation slot or a macro.
   - The editor must close any open gesture when the pointer is released, wherever the widget went. *Measured:* a drag whose widget changed id never sent `drag_stopped`, and its gesture stayed open.
3. **Stable ids from module ids and addresses.** Each card gets `UiBuilder::scope_id(Id::unique(("enth.card", module_id)))` and each control `("enth.param", addr)` (egui 0.37).
   - *Measured:* **59 of 59 control ids survived a switch between two layout variants, against 0 of 59** with today's widgets.
   - A knob drag continued across a mid-drag relayout; with today's auto ids the drag was lost.
   - `Knob` allocates an auto id today. It needs an id salt, like `Segmented` and `ModuleCard` already take.
4. **Layout is data: a `LayoutSpec`.**
   - It is a tree of rows and columns whose sizes are tokens or shares and whose leaves are named regions.
   - A card registry maps each patch section and built-in panel to a region and a presenter.
   - A per-kind presentation table holds the header enum, parameter rows and short labels.
   - Variants are values, serde-loadable like the theme (`ENTH_LAYOUT`, `--layout`). The prototype renders the same components in two variants by swapping one value *(measured, PNGs in `runs/`)*.
   - Don't adopt egui_taffy, egui_flex, egui_tiles or egui_dnd: all four still require egui ^0.36 on 2026-10-19 *(documented, crates.io)*.
5. **The engine must expose more than `stats()`.**
   - **(a) Route-depth value events.** `ops::effect` already reports `Effect::Depth`, but `Link` has no way to send it. Today every frame of a depth drag (the matrix, the knob's amount handle) would recompile and swap the plan *(documented)*.
   - **(b) A read-only tap of atomics, written at the end of `process()`.** It carries:
     - the routed parameters' current values for the newest voice (globals as is);
     - modulator outputs, the active-voice count, `EngineStats`, block peaks, and a scope ring;
     - a generation counter with the spike's wake handshake.
   - "Latest value" data belongs in atomics, not rtrb. P1b needs no rtrb ring towards the UI.
6. **Repaint gating: keep the spike's scheme.** A frame is scheduled only while a visible scope has fresh samples (`motion.scope_hz` 60 → ≈30 fps under baseview) or a visible live ring is moving while sound plays (`motion.animated_hz` 30 → ≈20 fps); otherwise the editor sleeps on the handshake.
   - "Visible" can only mean open, shown and inside the scroll viewport. egui-baseview never reports occluded or minimised, and nice-plug-egui's `hide()` leaves `is_open()` true *(documented)*.
7. **Interactions.**
   - **Drag-to-modulate** works with egui's built-in `DragAndDrop` *(measured)*: a global LFO lit 33 valid targets, an envelope 24, and a drop created `InsertRoute` at a default depth of 25 %. A voice-rate source dropped on the reverb was refused.
   - **Text entry with units:** right-click → "Type value…" → `ParamSpec::parse`.
     - "A4" set the cutoff to 440 Hz in one undo step.
     - "40ms" kept the old value and showed the parser's hint *(measured)*.
     - `parse` rejects the typographic minus "−" that the design system displays *(measured)*; it should accept it.
   - The matrix grid becomes `SetDepth` gestures. Envelope sculpting and disabled-module dimming exist already.
   - Right-click menus use `Response::context_menu`.
8. **Keyboard.** On X11, baseview takes focus on every click and never forwards keys, so Space never reaches REAPER. On macOS, and on Windows once baseview PR #260 lands, egui-baseview reports every key as unhandled unless a text field has focus. So a Cmd+Z that the editor consumes also reaches the host *(documented)*.
   - Recommendation: take focus only while a text field is edited (a fork item, sub-report 01).
   - Undo and redo as header buttons in P1b. Keyboard undo waits for a "captured shortcut" hook in the fork.
9. **R1/R2 hooks: leave room for, but don't build, two things.**
   - An `Overlay` passed to the view-model: per-parameter emphasis Normal / Dimmed / Hidden, score and direction; per-module emphasis.
   - The regions `LessonPane`, `ChallengeBar` and `ResultsDrawer` in the `Region` enum, with mode-specific `LayoutSpec`s and presenters behind the `learn` / `recreate` cargo features. P1b ships a no-op overlay and no presenters for those regions.
10. **Tests.**
    - Most editor tests are kittest interaction tests without a renderer. Six prototype tests run in about 11 s together *(measured)*.
    - Plus a small golden set on lavapipe: variants × {P1 default, maximum patch} plus a few interaction states, with a `kittest.toml` and deliberate updates.
    - A gungraun instruction budget for one editor frame. The prototype's generated editor costs **4.55 M (idle) / 4.90 M (animating) instructions per frame**, against the spike panel's 6.69 / 7.64 M *(measured)*.
    - An owner checklist for the real display.
11. **Generated cards need presentation rules, which the first renders show** *(measured, `runs/variant-*.png`)*:
    - 5- and 7-item enums as segmented controls overflow lane cards; use a dropdown above 4 items;
    - the "type" enum (wave, mode, shape, colour) belongs in the card header, as the handoff's component list says;
    - "Pulse width", "Resonance" and "Keytrack" are truncated in a 56 px cell;
    - `display_rounded` prints `-12st` and `1.23kHz`, not the design system's `−12 st` / `1.23 kHz`.

## Findings

### F1. What P1b starts from (documented: repo `main` at 1404239, read 2026-10-19)

**Widgets (`crates/enth-ui/src/widgets/`).** All take normalised values and `&Theme`; none knows the patch (`enth-ui` depends only on egui and serde).

| Widget | State | Id today |
| --- | --- | --- |
| `Knob` | value, modulated arc, score ring, `Emphasis::{Normal, Dimmed}`, inline layout | **auto id** (`allocate_exact_size`, `knob.rs`): changes when anything before it in the same `Ui` changes |
| `ModuleCard` | strip / stacked, header drag handle, enable toggle, whole-card dimming via `multiply_opacity` | `make_persistent_id(salt)` (scope-relative) |
| `Segmented`, `CurveHandles`, `MatrixGrid`, `SourceTile` | as in `docs/spikes/ui.md` | salt, scope-relative |
| `NumberField` | egui `DragValue` with a suffix | auto id |
| `MacroBar`, `StatusLine`, `ChallengeBar` (placeholder), `LessonPane` | | — |

**Spike panel (`spikes/spike-ui/src/panel.rs`, 775 lines).**
- Regions are hard-coded in `Regions::new`, so the layout is code.
- Module content is a `const` table (`layout.rs`) with its own `Unit::format`.
- Gestures go through a `HostParams` begin/set/end trait.
- The audio feed (`feed.rs`) is atomics only: a sample ring, an LFO phase, a generation counter and the sleep/wake handshake. That handshake is the model to keep.

**`enth-patch::History` (`history.rs`).**
- Merges a gesture's records only for a `SetParam` on the same address: it keeps the first inverse and takes the last forward. Anything else is concatenated.
- `begin_gesture` clones the patch, so `end_gesture` can annul a gesture that ended where it started.
- Undo and redo end any open gesture.
- `CAP` = 1000 entries.

**The real `Op` list differs from design.md §2.** It has `SetDepth`, `InsertRoute`/`RemoveRoute`, `Move`, `SetSlot` and `SetMacroName`, not design.md's `SetRoute`/`Unroute`/`MoveModule`/`AssignSlot`. `ops::effect` classifies each op as `Value(addr)`, `Depth(route)`, `Macro(i)`, `Structural` or `NoAudio`.

**Engine (`crates/enth-engine/src/swap.rs`, `engine.rs`, `params.rs`, `plan.rs`).**
- `Link` sends plans, parameter values, macros and the mod wheel. **There is no depth command**, so `Effect::Depth` has no path to the audio thread.
- The compiler leaves out depth-0 routes and routes from or to disabled modules. A routed parameter is always `from_norm(n)`, and that rule keeps settled values bit-exact.
- The data a tap needs exists on the audio thread, all `pub(crate)`:
  - `VoiceTable::{cur, routed}` and the global table;
  - `Plan::{vmod_cur, gmod_cur}`;
  - `NoteState::{gate, note_index}`;
  - `VoiceManager::active`.
- The UI cannot call `Engine::stats()`, because the engine lives on the audio thread.

**`enth-params`.**
- `ParamSpec::parse` reads `800Hz`, `1.2 kHz`, `A4`, `40ms`, `+1oct`; errors have a `hint()`.
- `depth_from_text(base, "+1oct")` converts musical intervals into route depths.
- `display` / `display_rounded` print the CLI's compact form (`1.23kHz`, `-12st`).

### F2. egui 0.37.0 APIs the design relies on (documented: egui 0.37.0, 2026-10-15, MIT OR Apache-2.0, [crates.io](https://crates.io/crates/egui/0.37.0); source in the cargo registry)

- **Ids.**
  - `Id::new` is deprecated in favour of `Id::unique(source)`, a global root id (`id.rs` L91).
  - `Ui::id` was renamed `Ui::scope_id`.
  - `UiBuilder::scope_id(Id)` (`ui_builder.rs` L73) "lets child widgets move in the ui tree without losing state". The new `Ui`'s unique id is that id, and its auto-id counter starts from it (`ui.rs` L254–263). So even auto-id widgets inside such a `Ui` keep their ids, as long as their order inside it is fixed.
  - The documented caveat: the same scope id must not render in two places in one frame. A "focus + overview" variant that shows a module twice must salt by view.
- **Drag-and-drop.**
  - `Response::dnd_set_drag_payload`, `dnd_hover_payload`, `dnd_release_payload` (`response.rs` L555–600) and `DragAndDrop::{set_payload, payload, has_payload_of_type}`.
  - Hover and release use `contains_pointer`, so they work while another widget is dragged.
- **Menus and popups.** `Response::context_menu` (`response.rs` L1112) and `Popup::context_menu` / `Popup::menu` (`containers/popup.rs`).
- **Text.** `Response::request_focus` / `lost_focus` / `surrender_focus`, `Context::egui_wants_keyboard_input`, `TextEdit::singleline`, `DragValue::custom_parser` / `custom_formatter`.
- **Cost control.** `ScrollArea::show_viewport` (lay out only the visible part), `Ui::is_rect_visible`, `Context::layer_painter` (for the modulation wire), `request_repaint_after`.

### F3. The plugin window stack (documented: egui-baseview 0.9.0 at Codeberg 1f645a3, 2026-10-17; baseview 0.4.1, master 4c79df3, 2026-10-09; nice-plug-egui 0.6.0 / nice-plug 7197104, 2026-10-18. No new commits in any of the three since then, checked through the Codeberg and GitHub APIs at about 18:00 MSK)

- **Focus on click.** egui-baseview `window.rs` L750–760 calls `window.focus()` on every mouse press if the window lacks focus. baseview's X11 `focus()` is `set_input_focus(POINTER_ROOT, child)` (`platform/x11/window_shared.rs` L230–236).
- **Key status.** egui-baseview pushes every key into egui's input, then returns `Ignored` unless `egui_wants_keyboard_input()` (`window.rs` L942–960). That check uses the *previous* frame's focus state, so the app can consume a shortcut but cannot mark it handled.
- **X11 drops `Ignored`.** baseview's `handle_event` discards the handler's return value (`platform/x11/event_loop.rs` L568–575, L650). macOS forwards `Ignored` keys to the next responder (`platform/macos/view.rs` L735–763). On Windows, forwarding is [baseview PR #260](https://github.com/RustAudio/baseview/pull/260), open since 2026-05-25.
- **Releasing focus is not in baseview.** [Issue #152](https://github.com/RustAudio/baseview/issues/152) was closed on 2026-10-02 with "the grabbing focus part" implemented (#202); the maintainer is "not sure releasing focus … is a much needed feature" and offers a new issue.
- **Modifiers.** They arrive with mouse events too (`update_modifiers`, `window.rs` L525–550), so Shift-fine drags work without keyboard focus. The `keyboard_focus_workaround` feature, which re-focuses on every output event, is off by default in nice-plug-egui.
- **Visibility.** `ViewportInfo` sets only `focused` (`window.rs` L403). `occluded` and `minimized` are never reported, so the spike's `hidden` check is dead code inside a plugin.
- nice-plug-egui's `hide()` calls `window.hide()` but leaves `egui_state.open` true (`editor.rs` L324, L387). An editor hidden by the host keeps receiving feed wake-ups.
- **Repaint sources.**
  - `param_value_changed` and `param_modulation_changed` call `request_repaint()` (`editor.rs` L376–382), so host automation on any of the 72 parameters repaints the open editor, capped by baseview's pacing.
  - The `RepaintNotifier` is polled by a 64 Hz baseview timer.

### F4. Prior art

- **Kilohearts Phase Plant** ([docs: Modulation](https://kilohearts.com/docs/modulation), undated, fetched 2026-10-19; *documented*):
  - hover a source and press its plus icon to enter "modulation target selection mode"; each target shows a small orange knob, and dragging it sets the amount;
  - a modulated control turns orange, and double-clicking the amount knob removes the link;
  - right-clicking a control lists its modulations, one row each with an enable toggle, source name, amount, curve and delete;
  - modulators have their own animated displays.
- **Ultra** ([help: Modulation](https://ultra.audio/help/synth/modulation/modulation), authored 2025-03-18; *documented*):
  - drag a source's cross icon onto a knob, and every modulatable control lights up while dragging;
  - a ring shows the range, a white dot "the current value … the synth engine is using", and a small round range knob sets the depth.
- **Local analysis** (`_local/reference/ui/README.md`, not committed): the same convergence (handle → highlighted targets → depth arc → popup list), plus a matrix or list overview in every product.
- **REAPER keyboard.**
  - REAPER has a per-plugin "Send all keyboard input to plug-in" option *(anecdotal:* [HISE forum](https://forum.hise.audio/post/82700), [JUCE forum](https://forum.juce.com/t/wierd-keypress-behaviour-blocks-full-vst-functionality-in-reaper/52739)*)*.
  - The opposite failure, Space typed into a plugin's text field starting REAPER's playback, was reported for Vital on macOS and fixed by that option *(anecdotal:* [Vital forum, 2024-02-03 – 2024-03-15](https://forum.vital.audio/t/preset-naming-in-reaper-space-bar-starts-daw-play/14416)*)*. A focus policy must cover both directions.
- **Snapshot practice.** egui and rerun keep kittest snapshots in git LFS; egui's `kittest.toml` uses `threshold = 2.0` with macOS CI (0.6) as the source of truth ([egui kittest.toml](https://github.com/emilk/egui/blob/main/kittest.toml), [rerun .gitattributes](https://github.com/rerun-io/rerun/blob/main/.gitattributes), fetched 2026-10-19; *documented*). kittest 0.37 defaults: `threshold` 0.6, `max_failed_pixels` 0, `kittest.toml` at the workspace root, `Harness::mask(rect)` for animated areas.
- **Layout helpers** (crates.io API, 2026-10-19; *documented*):

| Crate | Version (date) | Licence | egui requirement | Verdict |
| --- | --- | --- | --- | --- |
| egui_taffy | 0.14.0 (2026-08-27) | MIT | ^0.36 | Flexbox/grid engine; not on 0.37; more than we need |
| egui_flex | 0.8.0 (2026-08-06) | MIT | ^0.36 | Not on 0.37 |
| egui_tiles | 0.17.1 (2026-08-18) | MIT OR Apache-2.0 | ^0.36 | Docking tiles and tabs, a different problem |
| egui_dnd | 0.17.0 (2026-08-06) | MIT | ^0.36 | Sortable lists; built-in `DragAndDrop` suffices |
| egui_extras `StripBuilder` | 0.37.0 | MIT OR Apache-2.0 | =0.37 | Usable inside a region; layout as code, not data |

### F5. Prototype results (measured, `proto/tests/ids.rs`, kittest 0.37 without a renderer, the G1 reference patch, 1200 × 800, basic + intermediate tiers = 59 controls)

| Test | Result |
| --- | --- |
| Control ids, layout `lanes` → `flow` | **Stable scheme: 59/59 unchanged. Today's widgets: 0/59** |
| Drag Cutoff 40 px, switch layout, drag 40 px more, release | **Stable:** 1500 → 5971.6 → 20000 Hz, gesture closed, 1 undo step. **Today's ids:** 1500 → 5971.6 → 5971.6 Hz (drag lost); the gesture stays open because no `drag_stopped` ever arrives |
| One 50 px drag | 1 `History` entry, 1 forward + 1 inverse op; Cmd+Z restores, Cmd+Shift+Z redoes (keys delivered to egui) |
| Polyphony (Int 1–16), 60 px at 1 px per frame | **Unchanged (16)**: the knob re-derives its normalised value from the rounded patch value every frame, so sub-step motion is lost. The same 60 px at 20 px per frame gives 10 |
| Right-click → "Type value…", type `40ms`, Enter | Value unchanged; hint "write a number in hz, e.g. 800, 800Hz, 1.2kHz, A4, C#3, Db2"; `egui_wants_keyboard_input()` true while editing |
| …then type `A4`, Enter | 440 Hz, field closed, keyboard focus released, 1 undo step |
| Drag `lfo2` (global) ⊕ onto Cutoff | 33 valid targets outlined while dragging; drop adds `lfo2 → filter1.cutoff` at 25 % |
| Drag `env1` (per voice) ⊕ onto reverb Wet | 24 targets outlined; drop refused (voice-rate source, global target) |
| `ParamSpec::parse` probes | `1.2 kHz` ✓, `-12st` ✓, `−12 st` (U+2212) ✗ `bad_value` |
| Lavapipe render of both variants | 210 / 159 ms per 1200 × 800 render; the PNGs show the overflow and truncation issues of Answer 11 |

### F6. Frame cost of a generated editor (measured)

`proto/examples/frame_bench.rs`, release, headless (egui pass + tessellation, no GL). This is the spike's method on the prototype editor: cards, knobs and segmented controls generated from `ParamSpec`s, 12 card displays, a 2048-sample scope, source tiles. It has no macro bar, matrix or lesson.

| Case | callgrind instructions per frame | Wall p50 / p99 (load ≈ 20, information only) | % of a core at 30 fps (mean) |
| --- | --- | --- | --- |
| Idle frame (input, nothing changes) | **4.55 M** | 1.15 / 1.85 ms | 3.9 % |
| Animating (8 modulated rings + new scope data) | **4.90 M** | 1.45 / 2.11 ms | 4.6 % |
| View-model rebuild + 59 value strings alone | 0.23 M | 53 µs | — |
| Spike reference panel, for comparison (docs/spikes/ui.md) | 6.69 M / 7.64 M | 0.9–1.3 ms | 3.5–4.2 % |

Instructions are from `(I(250 frames) − I(50 frames)) / 200`. Adding the macro bar, matrix and spectrum should land the real P1b editor near the spike's 7–8 M per animating frame. That is about 4 % of a core on the CPU side at baseview's ≈30 fps, inside the < 5 % threshold before GPU cost.

## Recommended design

### D1. Where the code lives

`crates/enth-ui` (L7) gains:

```text
theme/      tokens (unchanged; new tokens listed in D9)
widgets/    pure components: normalised values, ids from salts, no enth-patch types (unchanged rule)
layout/     LayoutSpec, Region, Presenter, the built-in variants, serde for override files
view/       view-model: CardView/ParamView from Patch + catalogue + Overlay; presentation table per ModuleKind
editor/     Editor: regions → presenters → cards → controls; Intent → session calls; interactions (dnd, menus, text entry)
```

- `enth-ui` then depends on `enth-session` (L6), `enth-patch` and `enth-params`, as architecture.md already allows.
- The plugin's `NiceEguiApp::ui` stays a translation: it builds a `HostBridge` over `ParamSetter` and calls `editor.ui(ui, &mut session)`.
- The dev window (eframe) runs the same `Editor` over a local session with a fake engine, for the "standalone window plus 1.5 s rebuild" loop.

### D2. The view-model

- `view::cards(&Patch, &Overlay, &UiPrefs) -> Vec<CardView>`, computed every frame (0.23 M instructions, F6). Each `CardView` holds:
  - the card key (`Module(ModuleId)`, `Voice`, `Master`), title (label, else catalogue name + id number), kind and section, enabled;
  - rows of `ParamView { addr, spec: &'static ParamSpec, value (plain), norm, text, rate, emphasis, score, mod_range, live }`.
- Controls follow from the spec:

  | Spec | Control |
  | --- | --- |
  | `Float` / `Int` | knob |
  | `Enum` with ≤ 4 items | segmented |
  | `Enum` with more | dropdown (`egui::ComboBox` styled by the theme, or a new token-built `Dropdown`) |
  | `Toggle` | toggle |
  | `structural` | the same control; the session coalesces the recompiles (D5) |

- **The presentation table (`view/presentation.rs`) is UI data, not catalogue data.** It is a `const` table per `ModuleKind`:
  - the header enum, shown as the title's type dropdown (`osc.wave`, `svf.mode`, `lfo.shape`, `noise.color`);
  - the parameter rows (which keys, in what order);
  - the display kind (wave, filter response, envelope with handles, LFO shape, reverb or compressor curve);
  - short labels for the 56 px cell ("PW", "Res", "Key");
  - which tier is visible by default.

  A test checks that it names only catalogue keys, and that every basic and intermediate key of every kind appears somewhere: the generated-from-`ParamSpec` guarantee.
- **Value text.** Add a UI formatter, here or in `enth-params`, that prints the design system's form: a space before the unit, the typographic minus "−", and 3 significant digits ("1.23 kHz", "−12 st"). The text field pre-fills the ASCII exact form (`ParamSpec::display`); `parse` should accept "−" (a two-line change in `units::split_number`).

### D3. Layout as data

```rust
pub enum Region {
    Header, Macros, Generators, VoiceFx, Fx, Modulators, Sources, Matrix, Scope, Spectrum, Status,
    // later; render nothing in P1b:
    LessonPane, ChallengeBar, ResultsDrawer, Focus, Overview,
}
pub enum Size { Tok(SizeTok), Fill(f32) }                     // SizeTok resolves through the theme: no pixels in a layout
pub enum Node { Rows(Vec<(Size, Node)>), Cols(Vec<(Size, Node)>), Leaf(Region) }
pub enum Presenter { StripList, LaneStack, Band, Grid { columns: u8 }, Compact }  // how a region lays out its cards
pub struct LayoutSpec {
    pub name: String,
    pub root: Node,
    pub registry: Vec<(Section, Region, Presenter)>,         // which section fills which region, and how
    pub panels: Vec<(Panel, Region)>,                         // Macros, Matrix, Scope, Spectrum, Sources, Status …
}
```

- **Components are written against regions, never against a variant.**
  - A card renders the same in every presenter except its `CardShape`. `Compact` (title, display, two key knobs) is new and only needed by a "focus + overview" variant.
  - Regions scroll on their own (`ScrollArea` salted by region name) when content exceeds them, so the layout tree needs no minimum sizes for P1.
- **Variants.**
  - Built-ins are Rust constructors (`lanes()`, the handoff variants once they arrive).
  - `LayoutSpec` derives serde, so a variant can also be a JSON file: `spike-ui-dev --layout file.json` / `ENTH_LAYOUT=file.json`, exactly like `--theme` / `ENTH_THEME`. The owner can then compare a designer's variant in REAPER without a rebuild.
  - The handoff asks for "the main regions with sizes in logical px". Those become `Size::Tok` entries, with new size tokens where a variant needs them.
- **Modes are layouts.** Play, Recreate and Learn each pick a `LayoutSpec` (Recreate: `ChallengeBar` replaces `Macros` and `ResultsDrawer` overlays the body; Learn: `LessonPane` on the left). Switching mode changes data, not components.
- **The prototype proves the mechanism.** `proto/src/layout.rs` is 160 lines, and its `flow()` variant reorders every region with no component change.

### D4. Ids: rules

1. Every card body runs in `ui.scope_builder(UiBuilder::new().scope_id(Id::unique(("enth.card", view_role, module_id))), …)`; built-ins use `"voice"` / `"master"`.
2. Every control has an explicit id from `Id::unique(("enth.param", view_role, addr))`. Add `Knob::id_salt` and give `NumberField` one. Until then, wrapping a control in a scope with that id gives the same stability (the prototype does this).
3. Tiles: `("enth.source", src)`. Matrix cells: `("enth.route", src, dst)`. Handles: `("enth.handle", module_id, index)`. Region scroll areas: `("enth.region", region)`.
4. `view_role` is `"main"` unless a variant draws a module twice (focus + overview). egui forbids one scope id in two places in a frame.
5. Ids depend on module ids only. `Op::Move`, undo of `RemoveModule` and layout switches keep them; `SetLabel` doesn't touch them.

### D5. Edits: intents, gestures and the session

- Widgets push `Intent`s into a per-frame list:

  ```rust
  enum Intent {
      Begin, Set(ParamAddr, f32), End,
      Toggle(ModuleId, bool), Route(ModSource, ParamAddr), Depth(RouteAddr, f32), Unroute(RouteAddr),
      Move(ModuleId, Position), Add(Section, ModuleKind), Remove(ModuleId), Macro(MacroIndex, f32),
      Undo, Redo, …
  }
  ```

  `Editor::ui` applies them after layout, so no widget borrows the session. Latency: one frame plus one 32-frame segment, inside benchmarks.md's "knob → sound ≤ 1 block + 1 frame".
- **What the editor needs from `enth-session`** (sub-report 01 owns the design):
  - `patch()`;
  - `begin_gesture() -> GestureId`, `edit(ops, Option<GestureId>) -> Result`, `end_gesture()`;
  - `undo()` / `redo()` / `can_undo()` / `can_redo()`;
  - `tap() -> &TapReader` (D7);
  - `epoch_map()`, mapping the tap's routed slots to addresses.
  - **The session opens host gestures lazily:** the first `SetParam` (or `SetMacro`) inside a gesture whose address is bound to a slot or is a macro sends `begin` + `set` to the host, later ones `set`, and `end_gesture` closes every host gesture it opened (the spike's `gesture_if`, moved below the UI). Automation then records the same drag that undo sees as one step.

| Widget action | Ops | Gesture |
| --- | --- | --- |
| Knob or handle drag | `SetParam` per frame (two addresses for the decay/sustain handle) | one: `Begin` at `drag_started`, `End` at `drag_stopped` |
| Wheel burst on a knob | `SetParam` per tick | one per burst: opens on the first tick, closes after 300 ms without a tick or when the pointer leaves (one scheduled wake-up) |
| Double-click reset, text entry, segmented, dropdown, toggle | one op | none (a single record; the host gets begin/set/end) |
| Matrix cell drag, amount-handle drag | `SetDepth` per frame (`InsertRoute` first if the cell was empty) | one |
| Drop a source on a control | `InsertRoute { index: len, depth: 25 }` | none |
| Card header drag | nothing while dragging (local preview); `Move` once on drop | none |
| Card toggle, add, remove | `SetEnabled`, `AddModule`, `RemoveModule` | none |

- **Rules the prototype exposed** *(measured, F5)*:
  1. **Close gestures globally.** If a gesture is open and `!ctx.input(|i| i.pointer.any_down())`, end it. Also end it when `build()` runs again: nice-plug-egui 0.6.0 never calls `editor_closed()`. A vanished widget never reports `drag_stopped`.
  2. **Discrete knobs accumulate.** Keep the unrounded normalised value of a running drag in egui temp memory under the knob's id, and emit `SetParam(from_norm(accumulated))` only when the rounded value changes. Otherwise slow drags on `Int` and `Enum` knobs do nothing.
  3. **Structural edits are coalesced** (polyphony, voice mode, `osc.table`, `lfo.scope`, toggles, routes added or removed, moves). The session compiles at most once per frame and respects `MAX_PLANS_IN_FLIGHT`. For a structural knob, commit at `drag_stopped` instead of every step. benchmarks.md's "no frame > 50 ms during a knob drag" puts plan compile time on the GUI thread (sub-report 03 measures it).
- **History (small change, recommended):** extend `merge`'s same-key rule to `SetDepth` on the same route and `SetMacro` on the same index. Today a 2 s matrix-cell drag concatenates about 60 op pairs into one entry: correct, but wasteful. The property suite already covers the `SetParam` case; add these two to it.

### D6. Undo and redo

- **P1b: Undo and Redo buttons in the header**, with the `History` stack's top entry named in a tooltip ("Undo Filter 1 › Cutoff"). They work on every OS and host.
- **Keys:** Cmd/Ctrl+Z, Cmd/Ctrl+Shift+Z and Ctrl+Y are consumed through `input_mut(consume_key)` only when no text field has focus (`TextEdit` has its own undo). The prototype shows they work once egui sees them, but egui-baseview can't mark them handled (F3), so:
  - on Linux they never reach the editor under the recommended focus policy;
  - on macOS, and on Windows after #260, they would also reach the host.

  Ship keyboard undo when the fork adds a "captured shortcuts" list to egui-baseview: a set of (modifiers, key) that `on_event` returns as `Captured`. That is a ten-line change for sub-report 01's fork plan.
- An external change (host automation, state load, migration) applies `SetParam` without a history entry (AGENTS.md). Loading a state clears the history (`History::clear`).

### D7. Engine → UI feedback: the tap

**What the engine adds (`enth-engine::tap`, allocation only at construction):**

```rust
pub struct TapCapacity { pub routed: usize /* 64 */, pub modulators: usize /* 8 */, pub scope: usize /* 4096, power of two */ }
pub struct Tap {                         // shared through Arc; atomics only; the audio thread writes, the UI reads
    enabled: AtomicBool,                 // set by the shell: editor open and shown
    generation: AtomicU64,               // bumped by every publish that changed something
    ui_sleeping: AtomicBool,             // the spike's handshake: publish returns "wake the UI"
    epoch: AtomicU64,                    // the plan whose routed order the values below follow
    voices: AtomicU32,                   // active voices (held + releasing)
    display_note: AtomicU64,             // note index of the voice the values describe
    routed: Box<[AtomicU32]>,            // f32 bits, plain units: per routed destination, its `cur`
    mods: Box<[AtomicU32]>,              // per modulator: its output for the display voice (voice modulators) or global
    peaks: [AtomicU32; 2],               // block peaks, L and R
    scope: Box<[AtomicU32]>, written: AtomicUsize,   // mono mix, newest samples
    stats: [AtomicU64; N],               // EngineStats fields (stolen, cut, stale, swaps, crossfades, dropped)
}
```

- **The writer runs at the end of `Engine::process`**, only when a tap is attached and enabled.
  - The display voice is the active, non-stolen voice with the highest `note_index` and its gate held, else the newest releasing voice.
  - Routed voice-table parameters publish `cur[p][display_voice]`; global ones publish `cur[p]`; modulators publish `vmod_cur[m][display_voice]` or `gmod_cur[m]`.
  - A silent block after a silent block publishes nothing, as in the spike, so a silent synth costs the UI nothing.
  - The cost is a few dozen relaxed stores per `process` call, plus one store per sample for the scope. That is negligible next to the engine's measured ≈6,353 instructions per frame (an estimate, not measured).
  - The writer reads only, so renders and goldens are unchanged. `enth-render` never attaches a tap.
- **Main-thread pieces:**
  - `Plan::routed_ids() -> Vec<ParamId>` (and modulator order), computed at compile time;
  - the session's map from the tap's slots to `ParamAddr`s for that epoch. The UI ignores routed values whose epoch is not the one it has a map for: one stale frame is hidden, not shown wrong.
  - The shell calls `RepaintNotifier::request_repaint()` when the publish says the UI sleeps. The engine can't name nice-plug types, and this is translation.
- **Depth value events:**
  - `Link::send_depth(epoch, DepthId, depth)` and `PlanMap::depth(&RouteAddr) -> Option<DepthId>`; the engine overwrites the term's depth at the next segment.
  - A depth going from 0 to non-zero, or back, stays structural: the compiler omits depth-0 terms, which keeps settled values exact and plugin playback equal to CLI renders. The session sends a value event for non-zero → non-zero changes and recompiles otherwise.
  - Each needs an overflow test under `assert_no_alloc` (AGENTS.md): a full command ring with depth events, a tap with 64 routes and 16 voices, a plan swap during a publish.
- **Why atomics and not rtrb:** the UI wants the latest value, not every value.
  - An rtrb consumer would have to drain at audio rate even when the editor sleeps, and a full ring makes the producer drop new data.
  - Tearing across values is harmless for a display: a frame may mix two blocks, as the spike accepted.
  - rtrb fits event streams to the UI (note on/off for a keyboard display, clip events); P1b needs none. `triple_buffer` stays rejected (MPL-2.0).
- **UI side:**
  - Spectrum: a Hann-windowed FFT of the newest 2048 scope samples on the GUI thread (reuse `enth-analysis`; the spike's own FFT is the fallback).
  - Meters: peak hold with decay from egui's time.
  - Knob rings: the static range from the patch (D9) always; the live value from the tap only while voices > 0 or the output isn't silent.

### D8. Repaint gating

- Per frame, compute the next frame from what is visible and changing (the spike's code, kept):
  - a visible scope or spectrum with a new tap generation → `motion.scope_interval()`;
  - a visible live ring or source tile while sound plays → `motion.animated_interval()`;
  - otherwise call `tap.sleep(generation)` and schedule nothing; the next changing publish wakes the editor once.
- **"Visible" = editor open ∧ shown ∧ `ui.is_rect_visible(rect)`.**
  - `shown` needs a flag set by `show()` / `hide()`: a small nice-plug-egui patch, or the shell tracking CLAP `gui.show` / `gui.hide`. With `hide()` the tap's `enabled` goes false.
  - Occluded and minimised are not reported by egui-baseview; keep the spike's check for the dev window only.
- **Global LFOs** run while no note sounds. Their rings animate only while sound plays, or the editor would draw at 20 fps forever whenever a global LFO is routed.
- The tokens stay `motion.animated_hz` 30 and `motion.scope_hz` 60 (≈20 and ≈30 fps under baseview's MSC+2 pacing, ADR-0002).
- Host automation repaints through nice-plug-egui's `param_value_changed` (F3). That is accepted for P1b; a filter on visible slots would need a fork change.

### D9. Interactions

**Drag-to-modulate** (works in the prototype, F5).
1. The ⊕ handle on a source tile has `Sense::drag()` and calls `DragAndDrop::set_payload(ModDrag { src, rate, from })` at `drag_started`.
2. While a `ModDrag` payload exists:
   - each control whose spec is `modulatable` and whose target rate accepts the source's rate (`Patch::target_rate`; a voice-rate source can't drive a global target) draws an outline in `accent.modulation`, thick under the pointer;
   - invalid controls draw at `Emphasis::Dimmed`;
   - a straight wire from the handle to the pointer goes on a `layer_painter(Order::Tooltip)` layer.
3. On release, `dnd_release_payload::<ModDrag>()` emits `Route(src, dst)` → `InsertRoute` at `DEFAULT_ROUTE_DEPTH` = +25 %. A depth of 0 would be invisible and compiled out.
   - The default is a product constant (session or patch), not a token.
   - Dropping onto an existing route opens its row in the modulation popup instead.
4. Tokens to add: `size.target.outline` (reuse `stroke.thin` / `thick`) and `size.wire` (stroke). No new colours.

**Depth on the knob.**
- **Static:** the inner ring shows the route range `[lo, hi]` around the base value, in `accent.modulation_wash`. It is computed from the routes and the sources' polarity: bipolar LFO ±d, unipolar LFO / envelope / velocity / macro / mod wheel 0…+d, key (−0.5…+0.56)·d. Put that as a function in the session or patch (`modulation_range(patch, addr)`), so the CLI's `describe` can use it too.
- **Live:** a solid arc or dot at the tap's value, in `accent.modulation`.
- **Knob API change:** `.mod_range(lo, hi)` plus the existing `.modulated(live)`.
- **Amount handle:** on hover, a small handle at the end of the range arc; dragging it is a `SetDepth` gesture on the knob's first route, or on the route selected in the popup.

**Modulation popup** (right-click a modulated knob → "Modulation…", or click its ring).
- One row per route, in summation order: source name, depth as a `NumberField` with `%` and `custom_parser(|t| spec.depth_from_text(base, t))` (so "+1oct" works on a cutoff), and a remove button (`RemoveRoute`).
- The patch model has no route enable, so there is no toggle; that is a later additive field if wanted.

**Matrix.**
- `MatrixGrid` as is, fed from the patch:
  - rows are routed destinations, plus the modulatable parameters of a selected module;
  - columns are the sources present: modulators, velocity, key, mod wheel and used macros.
- A cell drag is a `SetDepth` gesture (`InsertRoute` first if empty), and a double-click is `RemoveRoute`.
- Routes from disabled modules draw dimmed, since the compiler omits them.
- A list mode (Surge-style, one row per route) is the same data and the CLI's shape: S1, not P1b.

**Envelope sculpting** exists (`CurveHandles`). Wire the real `adsr` with the spike's placement (each stage `0.05 + 0.25 × norm` of the width), so handles and knobs share the normalised scale. The decay/sustain handle is one gesture over two addresses.

**Disabled-module dimming** exists (`state.disabled_opacity`). `SetEnabled` is structural: one recompile per click.

**Right-click menus** (`Response::context_menu`; every item is an `Intent`):

| On | Items (P1b) | Later |
| --- | --- | --- |
| Knob or segmented control | Type value… · Reset to default · Modulation… (if routed) · Modulate with ▸ (sources) · Assign to automation slot ▸ / Clear slot · Show in matrix | MIDI learn, copy/paste value, "Why does this matter?" (R2) |
| Card header | Enable/Disable · Remove · Move up/down · Rename (label) · Reset module | Duplicate, save as snippet |
| Source tile | Remove all routes · Show in matrix | — |
| Matrix cell | Remove route · Type depth… | Curve |

**Text entry with units.**
- Right-click → "Type value…" replaces the control by a single-line `TextEdit` in the same cell. It is pre-filled with `ParamSpec::display` (exact, ASCII), selected and focused.
- Enter: `ParamSpec::parse`. On error, keep the field and show `ValueError::hint()` under it.
- Esc cancels. Losing focus cancels too, so a stray click never commits a half-typed value.
- One record, no gesture. Measured end-to-end in F5.
- Depth fields use `depth_from_text`. Accessible name "<Param> value".

**Keyboard focus** (decision with sub-report 01; owner check on Cinnamon).
- **Policy:** the editor holds keyboard focus only while a text field is edited.
- **egui-baseview fork:**
  - don't call `focus()` on every mouse press;
  - focus when `egui_wants_keyboard_input()` turns true;
  - release focus to the parent when it turns false. On X11, `set_input_focus(PARENT, parent)` or the host toplevel. That needs a baseview `release_focus`, which the maintainer offered to track (F3).
- **Result:** Space and the host's shortcuts keep working after a click on a knob; typing a value doesn't start REAPER's transport; Shift-fine drags still work (modifiers come with mouse events).
- **Cost:** no keyboard shortcuts in the editor outside text fields on Linux, hence D6's header buttons.
- Until the fork lands, ship as is and log the known failure ("Space lost after a click", benchmarks.md).

### D10. Recreate (R1) and Learn (R2): what P1b leaves room for

- **`Overlay`**, passed to `view::cards`:

  ```rust
  pub struct ParamOverlay { emphasis: Emphasis /* Normal | Dimmed | Hidden */, score: Option<Score>, hint: Option<Direction /* Up | Down */> }
  pub trait Overlay {
      fn param(&self, addr: &ParamAddr) -> ParamOverlay;
      fn module(&self, id: &ModuleId) -> Emphasis;
  }
  ```

  - P1b ships `NoOverlay` (all Normal).
  - `Emphasis::Hidden` is new. It collapses the cell, which stable ids make safe, and is only used by lessons. R1's "difficulty never locks features" means Recreate dims and scores, never hides.
  - `Knob` already draws score rings with ✓ ~ ✗. A direction arrow needs a glyph token.
- **Regions** `LessonPane`, `ChallengeBar` and `ResultsDrawer` exist in `Region` from P1b, and a Learn or Recreate `LayoutSpec` places them. Their presenters sit behind `learn` / `recreate` cargo features (roadmap: lean builds are a flag). P1b's header mode switch shows Play only.
- The challenge and lesson crates (L4) produce overlay data; the UI only renders it.
  - A/B audition is a session call (play the target or mine), with buttons in the challenge bar.
  - The editor always edits the session's current patch and never sees the target.

### D11. Testing

**Interaction tests** (kittest 0.37, no renderer, in `enth-ui` and run by `cargo test`):
- ids across variants; a drag across a relayout; one drag = one undo step; gestures closed on release; a slow discrete drag moves;
- right-click → type value (units, note names, error hint, Esc);
- drag-to-modulate (valid / invalid targets, the route created); a matrix cell drag = one `SetDepth` gesture; the popup's remove;
- card toggle dims and stays editable (exists); card move = one `Move` on drop;
- host gestures balanced, against a recording `HostBridge`;
- undo and redo buttons.

Helpers learned in the prototype: call `scroll_to_me()` before using a node's rect, because a rect may lie outside the `ScrollArea` clip; set accessible names on text fields through `WidgetInfo.label`.

**Golden screenshots** (lavapipe only, `.cargo/config.toml` as today):
- **What:** each built-in layout variant × {P1 default patch, maximum patch: 4 generators, 4 voice effects, 4 effects, 8 modulators} at 1200 × 800, plus one at the 1000 × 680 minimum and one at UI scale 2.0. Three interaction states: a modulation drag with outlined targets, an open context menu, a text-entry error. Plus the existing token gallery. That is about 12 PNGs of ≈300 KB.
- **Determinism:** fixed `RawInput.time`, the cursor removed, the scope fed from a fixed synthetic buffer (or `Harness::mask` over live areas), embedded fonts, scale fixed.
- **`kittest.toml`** at the root: `threshold = 0.6`, `max_failed_pixels = 0`. A Mesa upgrade re-baselines in one commit that says so.
- **Updates** only with `UPDATE_SNAPSHOTS=1` on purpose. The commit lists the changed PNGs and why, as for `cargo xtask golden --update`; reviewers open the `.diff.png`. Never updated in CI.
- **No git LFS** at ≈4 MB; revisit past 20 MB (egui and rerun use LFS at their scale).
- Agents read the PNGs as part of their loop (the prototype's renders found the overflow issues in Answer 11).

**Performance:**
- Move the spike's `frame_bench` to the real editor.
- Add a gungraun benchmark of one idle and one animating editor frame at scale 1, with a hard limit: proposed 10 M instructions per animating frame, then the first P1b measurement plus 10 %.
- Before/after instruction counts for UI changes, as AGENTS.md asks.
- Lay out only visible region content (`show_viewport`) if the budget is reached; the spike's profile says it is the largest saving.

**Owner checklist for the real display** (save work first, ≥ 10 s between opens, at most 10 open/close cycles):
1. The editor draws on first open in REAPER (CLAP), 10 tries.
2. Knobs: drag, Shift-fine, wheel, double-click reset. A slow drag on Polyphony moves it.
3. Drag a source ⊕ onto a knob: targets outline, the wire follows, the depth arc appears. Open the modulation popup and type "+1oct".
4. Sculpt an envelope; disable a module (it dims, stays editable).
5. Right-click menus near all four window edges stay inside the window.
6. Type "1.2 kHz", "A4" and a bad value. Space inside the field does not start REAPER's transport.
7. Click a knob, then press Space: logged result (expected to fail until the focus fork).
8. Undo and Redo buttons; Ctrl+Z (note whether REAPER also undoes).
9. Resize the window; UI scale 75 / 100 / 200 %; `Xft.dpi: 192` (clipping expected, baseview #359).
10. Play a chord: rings and scopes move at about 20–30 fps and stop when the notes end; `pidstat` near zero after notes end; `nvidia-smi pmon` while animating.
11. Layout variants through `ENTH_LAYOUT`: which reads best (the handoff's question).
12. G1: make a bass, pad, pluck and lead you like, through the editor only.

## Plan check

| Where | Change |
| --- | --- |
| design.md §2 | The `Op` list is out of date: `SetDepth`, `InsertRoute`/`RemoveRoute`, `Move`, `SetSlot`, `SetMacroName`, and `ops::effect`'s `Depth` |
| Engine (P1b work, design.md §3, engine.md) | Add `Link::send_depth` + `PlanMap::depth` (`Effect::Depth` has no engine path today); add the read-only tap (D7) with `Plan::routed_ids`; overflow tests under `assert_no_alloc` |
| `enth-patch::History` | Merge `SetDepth` per route and `SetMacro` per index like `SetParam` per address; extend the property suite |
| `enth-params` | Accept U+2212 "−" in `parse`; a UI display form (space, "−", 3 digits), or leave formatting to `enth-ui` |
| `enth-ui` widgets | `Knob::id_salt`, drag accumulation for discrete specs, `.mod_range(lo, hi)`, amount handle; a `Dropdown`; `Emphasis::Hidden`; `NumberField` id salt; a compact card shape only if a variant needs it |
| design-system.md | New tokens: target outline, wire stroke, popup row height, text-field error colour (reuse `score.bad` + glyph), drawer size, short-label rule; a "Layout" section: regions, `LayoutSpec`, tokens-only sizes |
| architecture.md | `enth-ui` = theme + widgets + layout + view + editor (one crate, because of the same-layer rule); widgets stay free of patch types |
| Sub-report 01 (fork plan) | Add: egui-baseview focus policy (no focus on click; focus while typing; release to parent) + baseview `release_focus` on X11; a captured-shortcut list; nice-plug-egui `show`/`hide` visibility flag; close open gestures when the editor closes (`editor_closed` fix) |
| Sub-report 03 | Plan compile time on the GUI thread during structural knob drags (coalesced to ≤ 1 per frame); the cost of `send_depth` vs a recompile per frame |
| AGENTS.md / benchmarks.md | The golden-screenshot policy (D11); an editor-frame instruction budget beside the engine's; the keyboard rows keep "expected to fail until the focus fork" |
| Roadmap P1b vs S1 | S1 lists "drag-to-modulate with depth rings" as polish. P1b needs only the functional part for G1: drop-to-route, static range arcs, the popup and the matrix. Live rings at 30 Hz come with the tap; the amount handle and list mode can slip to S1 |
| Handoff variants | Ask the designer to give region sizes as tokens or shares (they map one-to-one onto `LayoutSpec`) and to say where the "type" dropdown and module add/remove buttons sit; G1 needs adding and removing modules (`AddModule`/`RemoveModule`, limits 4/4/4/8) |
| Persisted editor state | Layout variant, UI scale, advanced-parameter toggle: a versioned `EditorPrefs` in the plugin state, outside the patch and not hashed (sub-report 01's state blob) |

## Gaps

- **Nothing ran in a host or on a GPU.** The interaction results are kittest's; focus, keyboard and HiDPI behaviour in REAPER under Cinnamon is the owner checklist, and the prototype had no window.
- **The tap is designed, not built:** no measured cost of its publish, no epoch-switch test. Expected to be small; a lane builds it with its overflow tests.
- **`send_depth` is designed, not built.** Whether depth changes need smoothing against zipper noise during a drag (about 30 steps per second) is open. A per-segment linear ramp of the term's depth would be the simple answer.
- **The frame budget's real number** awaits the full P1b editor (macro bar, matrix, spectrum, dropdowns, popups). The prototype's 4.55 / 4.90 M lacks them.
- **The focus fork is unwritten.** Whether `XSetInputFocus` to the parent works inside REAPER's SWELL windows, or whether forwarding keys with `XSendEvent` is needed instead, is untested. Synthetic events are often ignored *(unverified)*.
- **Which voice to show** (the newest held voice) is a design choice. I found no product document that states another rule, beyond Ultra's "the value the engine is using".
- **The layout variants themselves** come from the Claude Design handoff. This report only makes them cheap. A "focus + overview" variant would add the `Compact` card shape and view-role salts (D3, D4).
- **Rapid-fire wheel bursts and kinetic scrolling** over knobs inside scroll areas (the knob consumes the scroll today) need an owner feel-check.
