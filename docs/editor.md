# The editor (`enth-ui::editor`, L7)

**The editor is one value, [`Editor`], that draws a layout's regions from the session every frame and turns what the controls ask for into session calls at the end of the frame.** It lives in `crates/enth-ui/src/editor/`, depends on the session only through `SessionView` and `SessionEdit` ([session.md](session.md)), and runs unchanged in the plugin window, in the dev window and in kittest. Lane ED1 built it (P1b, [phase-1b.md](phase-1b.md)) on UI-C's cards and widgets; ED2 added drag-to-modulate, depth arcs and handles, the route list, telemetry (knob rings, tile levels, the scope, spectrum and meters) and repaint gating; LV adds layout variants. Visual rules: [design-system.md](design-system.md). Decisions: [ADR-0002](adr/0002-ui-toolkit.md) and its 2026-10-19 note (layout as data, stable ids, keyboard focus).

## Files

| File | Holds |
| --- | --- |
| `editor/frame.rs` | `Editor`: the frame (regions → read models → components), `apply` (intents → session calls), the editor's view state, `FrameReport` |
| `editor/layout.rs`, `editor/layout/<name>.json` | `LayoutSpec`, `Layout`, `Region`, `Part`, `Size`; the built-in layouts (`BUILTIN`; layout 1 is `lanes.json`) |
| `editor/regions/` | the region components: `Header`, `Macros`, `CardRegion`, `Overview` (tiles beside the matrix), `SourceTiles`, `Matrix`, `RouteList` (ED2: the overview when the grid does not fit), `Output` (ED2: scope or spectrum, and meters), `Status` |
| `editor/modulate.rs` | drag-to-modulate (ED2): the `SourceDrag` payload, the drop check, drop targets, the depth handle |
| `editor/live.rs` | live feedback (ED2): the telemetry reader per open, the frame's fingerprint of visible live values, repaint gating |
| `meter.rs` | display maths for the output region (ED2): dB, the spectrum's FFT |
| `editor/cards/` | UI-C's catalogue cards; ED1 added `menu.rs` (right-click menu, "Type value…"), the shared `control` (also used by the header) and the overlay fields on `ParamView` |
| `editor/overlay.rs` | the no-op `Overlay` (emphasis normal, dimmed, hidden or highlighted; scores; a lesson's locks, `Layered`) |
| `editor/describe.rs` | undo steps in the editor's words, for the Undo and Redo tooltips |
| `editor/reserved.rs` | the lesson, challenge-bar and results regions behind the `learn` and `recreate` features |
| `editor/add_module.rs` | (lane AM) adding and removing modules: the card's "…" menu, the region menu, the dashed "+ Add" slot, the header's Add hook, the "Rename…" field, the status line's notes ([below](#adding-and-removing-modules-lane-am)) |
| `theme/sizes/editor.rs`, `modulation.rs`, `output.rs`, `add_module.rs` | the editor's size tokens (below; the middle two are ED2's, the last AM's) |
| `examples/dev_window.rs` | the dev window (feature `dev-window`) |
| `examples/frame_bench.rs` | instructions per editor frame, headless (ED2 added the live cases) |
| `crates/enth-plugin/src/editor.rs` | the plugin window: `EditorApp` and `Glue`, the plugin's handle on `Editor` |
| `editor/recreate/` | the Recreate UI (lane RUI): the challenge bar in the header row (`bar.rs`), the results drawer in the overview slot (`results.rs`), the randomizer settings page (`settings.rs`), the score overlay (`overlay.rs`), the editor's Recreate state and round intents (`state.rs`); tokens in `theme/sizes/recreate.rs` ([below](#the-recreate-ui-lane-rui)) |
| `editor/browser.rs`, `editor/browser/` | the preset browser (lane PBU): `Presets` (the editor's preset state), the header's Presets hook (`header.rs`), the browse panel (`panel.rs`), the Save dialog (`save.rs`), the crash notice and report (`crash.rs`); tokens in `theme/sizes/browser.rs` ([below](#the-preset-browser-lane-pbu)) |
| `editor/learn/`, `editor/layout/learn.json` | the Learn view (lane LUI, feature `learn`): the header's entry (`header.rs`), the editor's Learn state, intents and the lesson overlay (`state.rs`), the lesson pane (`pane.rs`), the lesson browser (`browser.rs`), the built-in layout `learn`; tokens in `theme/sizes/learn.rs` ([below](#the-learn-view-lane-lui)) |
| `editor/create/` | the Create view (lane PUI, feature `create`): the header's entry (`header.rs`), the editor's Create state, intents and the browser's seam (`state.rs`), the panel's column (`panel.rs`), the drop zone and the intake's findings (`drop_zone.rs`), the folder browser (`browser.rs`), the progress and Cancel (`progress.rs`), the K tiles (`tiles.rs`), the compare area with the A/B against the upload (`audition.rs`); tokens in `theme/sizes/create.rs` ([below](#the-create-view-lane-pui)) |
| `editor/duel/` | the duel card (lanes SKD, then DUI; feature `duel`): its entry points and rows (`card.rs`), the editor's duel state, intents and the seam to the shell (`state.rs`), the share line and the copy buttons (`share.rs`), the paste box, **Paste** and Read (`paste.rs`), verification and the two results side by side (`verify.rs`), the sealed answer's buttons (`answer.rs`), the history list (`history.rs`); tokens in `theme/sizes/duel.rs` ([below](#the-duel-card-lane-dui)) |

## The frame

The shell owns the protocol; the editor is the middle of it ([plugin.md](plugin.md#the-editor), [session.md](session.md#who-calls-what-on-which-thread)):

1. read the host's values (plugin only);
2. lock the session: `pump`, `fold_host`;
3. **`Editor::frame`**: read the telemetry tap (`Live::refresh`, never waiting for the audio thread), cut the window into the layout's regions, build each region's read model from `SessionView::patch` and the telemetry (nothing is cached across frames), draw the components, ask egui for the next frame only if visible live data changes (`Live::schedule`), then apply the frame's intents through `SessionEdit` (`Editor::apply`);
4. `drain_requests` into a local buffer; **unlock**;
5. forward `ShellRequest::Host` writes to nice-plug's `ParamSetter` (begin, set, end).

`Editor::frame` never pumps, folds or drains. The shell calls `Editor::open(session, theme)` at every `build()` (live feedback starts: a telemetry reader over the session's tap, enabled) and `Editor::close()` when the window closes (the tap stops publishing). The plugin draws it on egui's root `Ui` (the whole window; the editor paints its own background). An editor preset load is `Session::load_patch` under the lock, never `GuiContext::set_state` (`enth-plugin/clippy.toml` bans it); the dev window's `--patch` takes that path. (Planned, 2026-10-20: the preset browser's load becomes `Session::load_preset`, one undo step delivered like a load, with no host writes; [below](#planned-for-r1-and-s1-2026-10-20).)

### Intents

Components never touch the session. They push `Intent`s; `apply` runs them in order:

| Intent | Session call |
| --- | --- |
| `Begin(grip)` … `Set`/`SetMacro`/`SetDepth` … `End(grip)` | `begin_gesture`, edits inside the gesture (one undo step, one host gesture per slotted parameter or macro), `end_gesture` |
| `Set` outside a gesture (double-click reset, a typed value, "Reset to default") | one `edit`, one undo step |
| `Select`, `Toggle`, `Enable`, `Unroute`, `Route`, `Automate`, `StopAutomating` | one `edit` each, no gesture |
| `Undo`, `Redo` | `undo`, `redo` (each ends an open gesture first) |
| `ShowMore` | editor view state only |
| (lane AM) `AddModule`, `RemoveModule`, `MoveModule`, `Duplicate`, `Rename`, `MakeAmpEnv` | one `edit` each, no gesture: one op (`AddModule`, `RemoveModule`, `Move`, `AddModule` of a copy, `SetLabel`, `SetAmpEnv`) and one undo step, through the session's structural path (a recompiled plan) |

`SetDepth` comes from a matrix cell, a route-list cell or a knob's depth handle, always inside `Begin(Grip::Route)` … `End` (one undo step; the session sends depth events, so a depth drag never recompiles unless the depth goes to or from 0). `Route` is a source dropped on a control: one `InsertRoute` at `DEFAULT_ROUTE_DEPTH` (25 %). `Unroute` comes from a matrix cell's double-click, a route-list cell's double-click or menu, and every control's "Remove … modulation" item. A gesture still open when no pointer button is held is closed (the control that opened it vanished); every `build()` and `editor_closed` close it too. An edit the session refuses changes nothing; its hint shows at the right end of the status line.

### What the controls do

- **Every parameter control** (knob, stepped knob, segmented control, switch; the header's inline knobs too) has the same right-click menu:
  - **Type value…** floats a text field over the control, filled with the value as shown and selected. Enter parses units in either form ("1.2kHz", "1.20 kHz", "A4", "−12 st" with the U+2212 minus the editor displays) and makes one edit. A value that doesn't parse keeps the field open with the parser's hint under it ("✗ write a number in hz, e.g. …"). Escape, or a click elsewhere, cancels. The field is named "<Parameter> value" for accessibility; one is open at a time, in egui memory under `ids::entry()`.
  - **Reset to default (value)**.
  - **Automate** exposes the parameter as the lowest free host slot (`bridge::free_slot`; a slot that already drives it is reused, F1b decision 4). Then the menu reads "Automated by slot N" and offers **Stop automating**. Structural parameters (they recompile the plan) are not automatable, and the disabled item says why.
- **Remove … modulation**: one item per route into the parameter ("Remove Env 2 modulation (+40 %)"), each one undo step (ED2).
- **The status line** reads the hovered or dragged control: "Filter 1 › Cutoff 1.50 kHz Env 2 +40 %, Vel +10 %", plus "automation slot N" when a slot drives it. Its whole text is its accessible name ("Status: …").
- **Header**: wordmark, patch name, Undo and Redo with the step's name in the tooltip ("Undo Filter 1 › Cutoff"), and the built-ins a layout places there as inline knobs (layout 1: master level). Keyboard undo waits for a captured-shortcut fork patch (ADR-0002), so the buttons are the way to undo on Linux.
- **Envelope sculpting** (UI-C): the attack peak, the decay and sustain corner (one gesture over two addresses) and the release end are handles on the knobs' own power scales, so curve and knobs stay in sync both ways.
- **Drag-to-modulate** (ED2, `editor/modulate.rs`; the layout handoff's §4). Drag a source tile's ⊕ onto a control: an orange wire follows the pointer; every parameter control checks itself against the dragged source once per frame (`modulate::check`, the patch's own route rules) and shows it: a **target** is outlined in orange (filled while the pointer is over it, and the status line says "Drop to modulate it from Env 2 by +25 %"); a parameter the source already drives is outlined faintly ("Env 2 already modulates it (+40 %)"); anything else is **refused, visibly**: drawn dimmed, and hovering it names the reason with a not-allowed cursor. A per-voice source (envelope, voice LFO, velocity, key) on a global destination (an effect, a global LFO, the master) says "LFO 1 runs per voice and Reverb 1 is global: only a global LFO, a macro or the mod wheel can modulate it"; a parameter that is not modulatable says so; the 64-route limit says so. A drop makes the route at 25 % (`enth_patch::modulation::DEFAULT_ROUTE_DEPTH`), one undo step. Escape cancels a drag (egui).
- **Depth arcs and the depth handle** (ED2). A routed knob shows a thin orange arc on its modulation ring, the span its routes can reach (`enth_patch::modulation_range`: base plus each route's depth over its source's polarity span, clamped), and, while the engine runs, the thick live arc from the value to the modulated value now (telemetry, the display voice's). The orange dot on the ring is the **depth handle** of the knob's newest route (at value + depth): drag it up or down (Shift for fine) to set that route's depth; the other routes are edited in the overview. Its accessible name is "<label> depth from <source>" ("Cutoff depth from Vel").
- **A disabled module dims as a whole** (UI-C, owner requirement): stripe, title, display, knobs and values at `state.disabled_opacity`; the controls stay editable.

## The layout seam

**A layout is a value, not code.** A `LayoutSpec` is JSON (`"schema": "enth.layout/1"`): a tree of rows and columns whose leaves are regions, sized by theme tokens or shares, plus a placement table that says which part of the patch fills which region. The built-in layouts are files under `src/editor/layout/`, embedded with `include_str!` and listed in `layout::BUILTIN`; a file named by `ENTH_LAYOUT` has the same form, so layouts switch without a rebuild.

```json
{
  "schema": "enth.layout/1",
  "name": "lanes",
  "margin": {"token": "space.gap"},
  "gap": {"token": "space.gap"},
  "root": {"rows": [
    {"size": {"token": "size.bar.header"}, "cols": [
      {"size": {"share": 1}, "region": "header"},
      {"size": {"token": "size.output.width"}, "region": "output"}
    ]},
    {"size": {"token": "size.bar.macros"}, "region": "macros"},
    {"size": {"share": 1}, "cols": [
      {"size": {"share": 1}, "rows": [
        {"size": {"token": "size.card.generator_height", "count": 3}, "cols": [
          {"size": {"share": 1}, "region": "sources", "arrange": "strips"},
          {"size": {"token": "size.card.lane_width"}, "region": "voice_fx", "arrange": "lane"}
        ]},
        {"size": {"token": "size.editor.band_height"}, "region": "modulators", "arrange": "band"},
        {"size": {"share": 1}, "region": "overview"}
      ]},
      {"size": {"token": "size.card.lane_width"}, "region": "effects", "arrange": "lane"}
    ]},
    {"size": {"token": "size.bar.status"}, "region": "status"}
  ]},
  "place": [
    {"part": "generators", "region": "sources"},
    {"part": "voice_fx", "region": "voice_fx"},
    {"part": "voice", "region": "effects"},
    {"part": "fx", "region": "effects"},
    {"part": "modulators", "region": "modulators"},
    {"part": "master", "region": "header"}
  ]
}
```

| Piece | Values |
| --- | --- |
| Regions | `header`, `macros`, `sources`, `voice_fx`, `modulators`, `effects` (the four card regions), `overview` (source tiles beside the matrix or the route list), `output` (ED2, optional: scope or spectrum, and the stereo meter), `status`; reserved: `lesson` (feature `learn`), `challenge_bar` and `results` (feature `recreate`), which draw an empty panel in P1b |
| Parts | `generators`, `voice_fx`, `voice`, `modulators`, `fx`, `master` |
| `arrange` (card regions) | `strips` (full-width strips, top to bottom), `lane` (lane cards top to bottom), `band` (lane cards left to right); default per region |
| Sizes | `{"token": "<path>"}` with an optional `"count"` (count copies with the layout's gap between them: three generator strips are 3 × 96 + 2 × 8 = 304 px), or `{"share": n}`. A token path is any number in the theme's serde form (`size.*`, `space.*`) |

**No pixels in a layout.** Region sizes are tokens or shares, as everywhere in the editor ([design-system.md](design-system.md)). A variant that needs a region size of its own adds a token to `theme/sizes/editor.rs` (with a serde default).

**The rules** (`Layout::new`, each refusal names where): the schema matches; every child of a row or column has a size; a gap is a token; a region appears at most once; `header`, `macros` and `status` are there (Undo and Redo live in the header, the macros are host parameters, the status line is the help); every part is placed exactly once, in a card region of the tree (`voice` and `master` may also go in the header, as inline knobs); `arrange` only on card regions; every token resolves to a number.

**Geometry** (`Layout::regions`, every frame): token sizes first, each clamped to what is left (a small window cuts the last fixed children first), then shares split the rest in whole pixels. A card region scrolls on its own when its cards don't fit, so a layout needs no minimum sizes; `FrameReport::overflow` says by how much.

**Stable ids.** Every control interacts under an id made from what it controls (`cards::ids`: a module id and a parameter address, a macro index, a source), never from where it is, so a re-layout keeps drags, hovers, the "Type value…" field and egui's memory. Tested: 59 of 59 controls of the P1 default keep their ids (and their AccessKit node ids) across two layouts, most of them in another place, and a knob drag continues through a layout switch in the middle of it, as one undo step.

**Choosing a layout.** The plugin, at every open of the window: `ENTH_LAYOUT` (a built-in's name or a file) if set and valid, else the built-in named in the `enth` field (`ui.layout`, `"lanes"` by default), else layout 1. A refused file is named on the status line. The dev window: `--layout NAME|FILE` or `ENTH_LAYOUT`, F2 for the next built-in, F5 to reload the file.

### Layout 1, "lanes"

The owner liked the visual design but not the placement and information architecture (layout handoff §3). Layout 1 keeps every component and tries the handoff's cheap fixes:

| Handoff §3 hypothesis | Layout 1 |
| --- | --- |
| 1. The signal flow doesn't read (a filter between oscillators, an envelope among the sources) | Signal order along the top of the body: sources (strips) → the filter lane → voice settings and effects (lane), master level at the header's right end. No filter or envelope among the sources |
| 2. Modulators are split | Envelopes and LFOs together in one band under the signal path; the source tiles (with velocity, key, the mod wheel) right under them |
| 3. The lesson pane and the matrix take prime space | No lesson pane in Play (a Learn layout places the reserved `lesson` region); the matrix sits in the overview, bottom left |
| 6. The scope and spectrum compete with the source tiles | (ED2) The scope or spectrum and the meter sit in their own `output` region right of the header, next to the master level; the tiles keep the bottom row |
| 7. The generator column scrolls | Three generator strips fit; no module region scrolls at the P1 default |

Hypotheses 4 (focal point), 5 (the effects lane) and 8 (the macro bar's weight) are left to the Claude Design variants (lane LV).

**Measured at 1200 × 800, the P1 default patch** (two oscillators, noise, a filter, two envelopes, two LFOs, a compressor and a reverb: the G1 reference patch), `tests/editor.rs::nothing_scrolls_at_1200_by_800_for_the_p1_default`:

- No module region overflows: sources 304 px for three 96 px strips, the filter lane 304 px for a 251 px card, the effects lane 642 px for voice (159), compressor (223) and reverb (223), the modulator band 224 px for envelope cards of 223 and LFO cards of 207.
- The overview gets 956 × 98 px: the header, macro bar and status bands (40 + 56 + 22 px and their gaps) plus the signal row (304) and the modulator band (224) leave 98 px of the 800. The source tiles fit in one row (544 × 85 px).
- **Nothing scrolls, the overview included** (ED2; `tests/modulation.rs::the_overview_fits_the_p1_default_at_1200_by_800_as_a_route_list`). The sources × targets grid would need 320 × 161 px for the patch's 7 routed targets and 6 sources (ED1 measured it scrolling by 63 px), so the overview shows the same routes as a **compact route list** in the 404 × 98 px panel: two columns of at least `size.modulation.route_min_width` (176) px, four rows of 16 px, 97.3 px with the caption. Eight routes fit; a ninth scrolls the list by one row. The grid comes back wherever it fits (a 1200 × 1000 window, `the_maximum_patch_shows_the_grid_where_it_fits`). Moving the macro bar into the header row (48 px) was not needed.
- At the maximum patch (4 generators, 4 voice effects, 4 effects, 8 modulators) every card region scrolls on its own (sources by 104 px, the filter lane by 723, the effects lane by 440, the band by 860 sideways), and so does the overview (the tiles by 51 px, eleven of them in two rows; the routes by 63 px).
- The output region takes `size.output.width` (208) px of the header row, so the header's master knob sits 216 px further left than in ED1's layout 1; osc 1's Level knob (REAPER's `dirty-flag` scenario point, 366, 178) does not move.

Snapshots: `crates/enth-ui/tests/snapshots/editor_lanes_p1.png` and `editor_lanes_max.png` (`tests/editor_snapshots.rs`, lavapipe, kittest's default threshold).

## Live feedback and repaint gating (lane ED2)

**Telemetry.** `Editor::open` (the shell's `build()`, every time) makes one `TelemetryReader` over the session's tap and enables it; while the session is detached the reader is made when the session first has a source, and after a re-activation it follows the new tap (session.md). `Editor::close` disables it. Every frame `Live::refresh` copies the tap's latest values (a seqlock with a bounded retry: it never waits for the audio thread), and `Editor::cards` and `Editor::sources` put them into the read models:

| Read model | From the tap | Drawn as |
| --- | --- | --- |
| `ParamView::live` (routed parameters) | the modulated value now, the display voice's (newest held voice), normalised | the knob's thick live arc from the value |
| `ParamView::range` (routed parameters) | (static) `enth_patch::modulation_range` | the knob's thin orange range arc |
| `SourceView::level` (envelopes, LFOs, the mod wheel) | the modulator's output, mapped by its polarity span to the tile graph's 0..1 | a level line and a dot at the graph's right end |
| the output region | the scope ring's newest 1,024 samples of `(L + R) / 2`; each channel's block peak | a scope, or (click) a Hann-windowed FFT spectrum on a log axis; two meters in dB from −60, falling 24 dB/s |

The tap publishes outputs, not phases, so `SourceView::phase` stays `None` (reserved). Velocity and key are per note and not in the tap: their tiles show no level. Nothing published yet (generation 0) reads as no value, never as zeros.

**The quiet rule** (engine, fixed by ED2; [dsp/engine.md](dsp/engine.md#telemetry-tap-p1b-lane-e)): in silence after a full silent scope ring, the tap publishes only when a value it shows changed (a new plan's epoch, a global modulator still running, a macro or a parameter moved in silence, a value that changed while the editor was closed). So an editor opened in silence sees the current plan at once, and silence with nothing moving costs the audio thread one comparison pass (+235 instructions per 64-frame block) and the editor nothing.

**Repaint gating** (`Live::schedule`, at the end of every frame; `FrameReport::repaint` says what it asked for):

- the frame hashes every live value it drew **where it can be seen** into a fingerprint, quantised to 1/512 of a range (a tenth of a pixel on a knob's ring): rings and tile levels count only if their widget's clipped rectangle is not empty (`live::is_visible`: scrolled out of its region does not count), the output region always (scope sample count, meter levels);
- the fingerprint changed, or the tap published since the last frame: `ctx.request_repaint_after(motion.scope_interval())`, 1/60 s, never sooner (`repaint.scope_hz`; lane AM, R1/S1: it was `motion.animated_interval()`, 1/30 s). baseview 0.4.1 paces redraws at the X Present `msc + 2`, every other vblank, so a 1/30 s request drew about 20 frames a second in Phase 0 and a 1/60 s request about 30, the design system's 30 Hz. The plugin's `editor-counts` feature counts the frames the window really gets ([plugin.md](plugin.md#leak-gates)); under Xvfb, in `leak-test`'s 200 editor cycles, the live frame rate was 26.0 and 27.8 fps with the 1/60 s request against 23.0 and 23.8 with 1/30 s (two runs each); owner check 1.3 reads it on the real display;
- otherwise the editor sleeps on the tap (`TelemetryReader::sleep`) and asks for nothing: **0 frames**. The next publish with new data sets the tap's wake flag; the plugin's `process()` (and the dev window's engine thread) turns it into a repaint. Input wakes it as always.

"Visible" means open (between `open` and `close`), shown and scrolled into view. baseview 0.4.1 reports no hidden or minimised window, so "shown" is "open" in the plugin. A value that changes while scrolled out of view still wakes the editor, through the tap, at up to 60 requests a second (the engine does not know what is visible); in layout 1 the tiles and the output region are always in view, so this does not arise at the P1 default.

Measured (`tests/repaint.rs`, exact delays from egui's output, no clock): an idle editor reports `Duration::MAX` frame after frame and an audition note's first block sets the wake flag; the P1 default's global LFO 2 keeps the reverb's Wet ring and its tile moving, and every frame asks for exactly 16.67 ms (33.33 ms before lane AM); a closed editor asks for nothing and its reopening picks the engine up again; a stalled engine (no block rendered at all) still gets 200 frames of knob dragging, the edits land in the patch (one undo step), six structural edits leave two plans in flight and one compile pending without blocking, and everything is delivered when the audio thread resumes.

**Zipper noise of a depth drag** (phase-1b.md open check): a depth event ramps over 10 ms in the engine (one step per 32-frame segment), exactly like the parameter smoothing a knob drag gets. Measured with a sine whose level an envelope route modulates, the depth swept 1 → 60 % in 1 s: at 30 events a second (an editor drag) the output differs from 750 events a second by −36.8 dB re the signal, and the same sweep as a knob drag (`SetParam`) by −37.0 dB. A depth drag is as smooth as a knob drag; no extra smoothing is needed.

## Room for Recreate and Learn

- **`Overlay`** (`editor/overlay.rs`): per parameter an emphasis (normal, dimmed, hidden) and a score; per module an emphasis. The editor applies it to the read model before drawing (`overlay::apply`): a hidden parameter is not drawn, a hidden module's card is dropped, a dimmed one draws in the dimmed ink and stays editable, a score draws as the knob's ring with its glyph. P1b ships `NoOverlay` (`Editor::set_overlay` takes another). Recreate dims and scores; only a lesson hides.
- **Reserved regions** behind the `learn` and `recreate` features of `enth-ui`: `lesson`, `challenge_bar`, `results`. A mode is a layout that places them; P1b draws an empty panel with the region's name.
- **Where they go** (2026-10-20, R1 research, measured at 1200 × 800 with the G1 patch): a mode is a layout file plus an overlay. Recreate's challenge bar shares the header row, and the results drawer takes the overview slot after Submit; a Learn pane of 320 px fits only when the lesson hides the modules it doesn't use. Details under [Planned for R1 and S1](#planned-for-r1-and-s1-2026-10-20).

## Planned for R1 and S1 (2026-10-20)

Decided by the R1 + S1 research (`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`, report 03 and the summary); built by the lanes named in [phase-2.md](phase-2.md). Lane S0r declares three no-op hooks in the header (Recreate, Presets, Add), so AM, PBU and RUI each fill their own. Each lane adds its own section here when it lands.

**Adding and removing modules (lane AM).** Built: see [Adding and removing modules](#adding-and-removing-modules-lane-am) below.

**The preset header and browse panel (lanes PBC and PBU).** The header's patch name becomes **◀ Name ▶ Browse Save**. The browse panel is editor view state drawn over the body, not a layout region, so it works in every layout, and while it is open it replaces the cards (the frame gets cheaper). Report 03's design: Factory, User and Favorites; a word search over name, description, author and tags; click loads, double-click loads and closes, Escape closes; ◀ ▶ step through the current list (they come first because arrow keys may reach the host); Save never offers a read-only folder. A load is `Session::load_preset`: one undo step from `ops_between(current, preset)`, delivered like a load (no per-slot or per-macro host writes, the nih-plug #92 crash class), and it marks the project dirty. A preset file is exactly `Patch::to_json()`; user presets live in `Documents/enth/Presets` (`ENTH_PRESETS_DIR` overrides it). The search field relies on the keyboard-focus fork patch inside REAPER (owner check 1.5). Landed: [The preset browser](#the-preset-browser-lane-pbu).

**Recreate and Learn in the layout (lanes RUI and LV).** Measured overflow with the G1 patch (report 03's scratch harness, `Editor::report()` at each size):

| Candidate | 1200 × 800 | 1000 × 680 |
| --- | --- | --- |
| Lanes (Play) | none | band 148 × 21 px, effects 98 px, no overview |
| Challenge bar as its own 40 px row under the header | tiles 39, routes 47, effects 26 | band 148 × 69, effects 146 |
| **Challenge bar sharing the header row** (480 / 480) | **none** | as lanes |
| **The same, with the results in the overview slot** | **none** | as lanes |
| Lesson pane (320) in the effects column, or as a left column | the band (732 or 276 px), tiles and routes | the band (932 or 476 px); effects 98 in the left-column one |
| Both lesson layouts with a small patch (`init` or `pluck_1osc`, standing in for a lesson that hides unused modules) | none (`pad_2osc`: the band by 48 px in one of them) | not run |

So Recreate's bar shares the header row (the patch name hides; the target is unknown anyway), and after Submit the results drawer (scores, time, Retry, New, Show target) replaces the overview, while the per-control rings draw on the knobs through `Overlay`. A Learn step names its layout and its overlay, and hides what it doesn't use. Lane SKR's skeleton bar (F2r) played the first rounds ([The Recreate hook](#the-recreate-hook-lane-skr)); lane RUI's UI replaced it: [The Recreate UI](#the-recreate-ui-lane-rui).

**The minimum window.** Decided by lane AM: `size.window.min` rose to 1200 × 800 ([below](#the-minimum-window)). Every layout variant still gets its snapshots and an overflow report, at 1200 × 800 and at whatever smaller size it promises to handle; Claude Design gets the `enth.layout/1` vocabulary (regions, parts, `arrange` values, token names with their values).

## Adding and removing modules (lane AM)

**Every structural edit is one intent, one op and one undo step**, through one `session.edit` (the session's structural path: a recompiled plan, swapped with state carry, crossfaded when modules come or go; the swap block is reported, not gated, [benchmarks.md](benchmarks.md#lane-rt-the-plugin-measured-2026-10-20)). `editor/add_module.rs` holds the entry points; the intents are in `cards/intent.rs`:

| Intent | Op | From |
| --- | --- | --- |
| `AddModule { kind, index }` | `AddModule` of a module at its defaults, id `Patch::free_id(kind)` | the header's Add menu, a region's right-click menu and its "+ Add" slot (appended), a card's Add after ▸ (right after the card) |
| `RemoveModule(id)` | `RemoveModule`: its routes and slot assignments go with it, and undo puts them back at their places | a card's Remove |
| `MoveModule { id, index }` | `Move` within the section | a card's Move up / Move down (Move left / Move right in a band) |
| `Duplicate(id)` | `AddModule` of a copy right after it: kind, values and switch; a new id, no label, no routes | a card's Duplicate |
| `Rename { id, label }` | `SetLabel` (an empty name clears the label: the card shows its kind and number again) | a card's Rename…, a field over the title row; Enter commits, Escape or a click elsewhere cancels; named "<title> name" for accessibility |
| `MakeAmpEnv(id)` | `SetAmpEnv` | an envelope card that is not the amp envelope, while it is switched on |

The entry points:

- **The card's "…" menu.** Every module card ends its title row with a "…" button (`glyph::MENU`, U+2026: Inter has no U+22EF), named "<title> menu"; a right-click on the title row opens the same menu. Built-in cards (voice, master) have none.
- **A right-click on empty region space** opens the Add menu of the sections the layout places in that region (the region registers a click area under its cards, so cards and controls keep their own clicks; a right-click on a card's body is not empty space).
- **The dashed "+ Add" slot** follows a region's cards when the region has room for it and one of its sections is under its limit, and never makes the region scroll: `size.add_module.slot_height` (32) tall in a strip or lane region, `slot_width` (40) wide and the band's height in a band. A click opens the region's Add menu. Its accessible name says what it adds ("Add voice effect", "Add modulator"). At 1200 × 800 with the P1 default the filter lane and the modulator band have room; the three strips and the effects lane do not, and nothing scrolls (tested).
- **The header's "+ Add" menu**, the Add hook (feature `add-module`, a default feature of `enth-ui`; without it only this button goes): every section, so any kind can be added however full the regions are drawn.

An Add menu lists each section's kinds by their catalogue names (Oscillator, Noise; Filter; Reverb, Compressor; Envelope, LFO), under a heading per section when it shows several. A full section's items (4 generators, 4 voice effects, 4 effects, 8 modulators) and its cards' Add after and Duplicate are disabled and say why ("A patch holds at most 4 sources"); the amp envelope's Remove is disabled ("make another envelope the amp envelope first"). Anything the session still refuses (`too_many`, `in_use_as_amp_env`) shows as the status line's hint. The status line's right end names what an add or a removal did until the next edit: "Added Osc 3", "Added Filter 2, a copy of Filter 1", "Removed Filter 1, 2 routes and automation slot 3".

Closed menus are not built (`Popup::is_id_open` first), and the slot's dashes are one mesh: `Shape::dashed_line` makes a shape per dash and cost the P1 default's two slots about 0.28 M instructions a frame ([Frame cost](#frame-cost)).

### The minimum window

**`size.window.min` is 1200 × 800, the default** (it was about 1000 × 680). Measured with the P1 default and layout 1 (kittest, `Editor::report()`):

| Window | What does not fit |
| --- | --- |
| 1200 × 800 | nothing |
| 1200 × 790 | the tiles by 1 px, the route list by 9 |
| 1200 × 760 | the tiles by 31 px, the route list by 39, the effects lane by 18 |
| 1150 × 800 | the route list by 63 px (two route columns need the overview 956 px wide) |
| 1100 × 800 | the band by 48 px sideways, the tiles by 51, the route list by 63 |
| 1000 × 800 | the band by 148 px sideways, the tiles by 51, the route list by 63 |
| 1000 × 680 | **no overview at all**: the source tiles and their ⊕ handles, the only drag sources, are gone; the band by 148 × 21 px, the effects lane by 98 |

So layout 1 needs all of 1200 × 800: the band of lane cards needs 1,148 px of width, two route-list columns need 1,200, and the column of signal row (304), band (224) and one row of tiles (85) with the three bars needs about 790 px of height whatever the arrangement. Two options were open: raise the minimum, or make layout 1 keep the sources at 1000 × 680 (a fixed overview row with the band as the share). The second keeps the tiles but leaves the band about 104 px tall, so every modulator card scrolls to its knobs; it also stops the overview growing in taller windows (the route grid comes back only there). **Raised**, because:

- the plugin window is already fixed at `size.window.default` (`EguiEditorState::from_size`; hosts cannot shrink it), so the minimum binds only the dev window and a future UI-scale menu;
- a small screen is the UI scale's job, not a smaller logical layout: at 75 % (`scale_percent` in `enth.plugin-state/1`) 1200 × 800 draws in 900 × 600 pixels with layout 1 intact; a scale over 100 % grows the window (default × scale), never the reverse;
- a compact layout is a variant's (lane LV), with its own overflow report.

`tests/add_module.rs::the_minimum_window_keeps_every_region_of_layout_1` checks that the minimum equals the default and that every region, the overview and every tile's handle are there at it, so `editor_lanes_p1.png` (1200 × 800) is also the snapshot at the minimum.

## The preset browser (lane PBU)

**The header's patch name is "◀ Name ▶ Browse Save", and the browser's three sheets (the browse panel, the Save dialog, a crash report) are drawn over the body in any layout.** The behaviour, the stores and the crash offer are described in [presets.md](presets.md#the-browser-lane-pbu); this section is the editor's side.

- **Where it hooks in.** `Editor` owns a `Presets` (`Editor::presets`, `presets_mut`): the store (a boxed `PresetStore`; `MemStore` unless the shell hands another, which the plugin does with `FsStore::from_env()`), the last scan, the open sheet and its view state, the held audition note and the crash offer. The header's Presets hook (feature `presets`, a default feature since PBU) draws the group right after the wordmark from `HeaderView::presets` (`HeaderPresets`: the name, from a preset, edited, browsing, saving), so it never meets the right-hand controls (Add, Undo, Redo, the master knob), down to the minimum window. The name paints only through the hook while the feature is on.
- **Intents.** Every browser control pushes `Intent::Preset(PresetIntent)`, applied in order with the frame's other intents (`Presets::apply`): `Step(±1)`, `Browse(bool)`, `OpenSave`, `Close`, `Load { key, close }`, `Audition(key)`, `Favorite { key, on }`, `Save { overwrite }`, `ShowCrash`, `DismissCrash`. A load is `SessionEdit::load_preset` through `presets::load_from` (one undo step, no host writes, `PatchEdited`); a save is `SessionEdit::save_preset`. The tab, the search text and the dialog's fields are view state edited in place while drawing. `describe::entry` names a load step "Load <name>", so Undo's tooltip reads "Undo Load Glass Pad".
- **Over the body.** While a sheet is open, `Editor::frame` computes the body (the bounding box of every region but the header, the output, the status line and the challenge bar) and skips every region that overlaps it: no card is built or drawn, so the frame gets cheaper, and the live rings under the sheet stop asking for frames. The header row and the status line stay. A sheet that opens or closes at a frame's end asks egui for one more pass.
- **Audition.** ▶ held on a row: load it unless it is loaded, then `SessionEdit::audition_on(60, 100/127)`; the note is released at the first frame with no pointer button down (like a drag's gesture) and when the window closes. The note's key stays inside a boxed release closure, so the UI crate needs no engine types.
- **The crash notice** is a foreground `Area` in the body's lower right corner; it is not modal and the editor keeps working beside and under it. The shell hands the reports in at every open as a `CrashReports` (the UI crate does no I/O).
- **Accessibility labels** (kittest finds the controls by them): "Previous preset", "Next preset", "Preset <name>" (and "Preset <name> (edited)"), "Browse", "Save"; the tabs by name; "Search presets"; a row by the preset's title, its "Favorite <title>" and "Audition <title>"; "Close browser"; "Preset name", "Preset folder", the tag chips by tag, "Save preset", "Replace", "Cancel"; "Show report", "Dismiss", "Copy report", "Close report".

| Token (`theme/sizes/browser.rs`) | Value | For |
| --- | --- | --- |
| `size.browser.header_name` | 180 | the name between ◀ and ▶ ("…" past it) |
| `size.browser.search_width` | 280 | the panel's search field |
| `size.browser.name_column` | 240 | a row's name column |
| `size.browser.folder_column` | 140 | where it lives ("Factory › bass") |
| `size.browser.author_column` | 140 | the author, at the right end; the tags take the rest |
| `size.browser.dialog_width` | 480 | the Save dialog's card |
| `size.browser.notice_width` | 380 | the crash notice |

Rows, buttons and fields are `size.control_height` high. Glyphs (`theme/glyph.rs`): `PREVIOUS` ◀, `NEXT` ▶, `FAVORITE` ★, `NOT_FAVORITE` ☆, `CLOSE` ×, all in Inter.

## The Recreate hook (lane SKR)

**Lane SKR's skeleton plays a round through the header's Recreate hook; lane RUI replaces the bar with the real challenge bar, results drawer and settings (wave 4).** The `recreate` feature is on by default in `enth-ui` and `enth-plugin` since SKR; the hook draws nothing until a shell hands the editor a round view, so kittest, the dev window and every snapshot that never does are unchanged.

- **The seam** (what a shell does each frame, and what RUI builds on): `Editor::set_round(GameEdit::round())` before the frame; the bar pushes `Intent::Game(GameIntent)`, which the editor queues (`Recreate::push`) instead of applying, since it holds only a `SessionEdit`; `Editor::apply_game(&mut dyn GameEdit, seed)` after the frame applies them (`recreate::bar::apply_game`; the shell supplies a 40-bit seed for a round from settings); `Editor::hold_scores(results)` before the shell hands worker results to `Session::game_job_done`. The plugin's glue is `Glue::round_frame` ([plugin.md](plugin.md#the-recreate-round-lane-skr)).
- **The skeleton bar** (replaced by lane RUI's challenge bar and results drawer, [below](#the-recreate-ui-lane-rui)): **Start** (level 1), then **Target ▶**, **Mine ▶** and a **Round** menu (Submit, Hint, Reveal, End), which became **Rate yours** while the blind rating was asked, and a one-line readout on the status line (`Recreate::readout`, kept by RUI). `hold_scores` keeps the `Scored` result back until the player answers, so the session records the rating before the score (it refuses a rating after the first score is recorded); RUI kept that too.
- **Tests**: `tests/recreate.rs` drives Start → Target ▶ → Mine ▶ → Submit → the rating (the score held back) → Hint → Reveal → End over a real `Session`, the worker's jobs run between frames with the same `enth-challenge` functions; `recreate_round_live.png` shows the bar beside the Presets group and the readout on the status line at 1200 × 800; `bar.rs` unit tests cover the view, the rating hold and the readout.
- **For RUI**: the bar uses no size token of its own (default buttons and menus in `text.body`), so `theme/sizes/recreate.rs` is still empty; the reserved `challenge_bar` and `results` regions are untouched; the no-peek check (no label or tooltip shows a target value) is RUI's. `RoundView::params` (the randomized addresses) and `knobs` (the rings) are not drawn yet.

## The Recreate UI (lane RUI)

**A round is played in the header row and the overview slot; the cards stay where they are.** Lane RUI replaced SKR's skeleton bar with the challenge bar, the results drawer, the randomizer settings page and the score overlay (`editor/recreate/`, feature `recreate`). Everything draws through SKR's seam and nothing at all until a shell hands the editor a round view, so every snapshot of a shell that never does is unchanged.

**The header row.** In Recreate mode the challenge bar takes the Presets group's place, from the wordmark to the right-hand row's leftmost control ("+ Add"); Add, Undo, Redo and the master knob stay.

| When | The header row shows |
| --- | --- |
| Play | **Recreate** in the right-hand row, left of "+ Add": enters Recreate mode |
| Recreate mode, no round | **Level** and five dots (a click picks level 1–5: code v1's presets; "custom" after the settings page changed them), a code field ("Paste a code"; Enter or **Start** decodes it, and a code that does not decode says why on the status line), **Start**, **Settings…**, and × at the right end (back to Play, the Presets group returns) |
| A round | the level's dots, **Target ▶** / **Mine ▶** (a second click turns the side off) / **A/B** (flips at the same position), the timer (active time, "Preparing…" while the target is prepared), **Submit**, **Hint** with the levels used ("1/4"), **Reveal**, and **End** (a menu: **End and restore my patch**, **End and keep this patch**) |

Measured at 1200 × 800 (layout 1): every control of both rows sits inside the header and left of "+ Add" (the round's row ends at 573 px of 660), and the frame reports no overflow, the drawer included (`tests/recreate.rs::nothing_scrolls_at_1200_by_800_in_recreate`).

**The results drawer** takes the overview slot (956 × 98 px at 1200 × 800) once the round has something to show: a submit (from the rating on), a hint or a reveal. **Modulation** (named "Show modulation") gives the slot back to the source tiles and routes, and **Results** ("Show results", over the overview's top right corner) brings the drawer back; a new submit or hint brings it back too.

1. **Submit asks first.** "How close is yours to the target?" with **1**…**5** ("Rate 1"…"Rate 5"; "Nothing like it" to "I can't tell them apart") and **Skip**. `hold_scores` keeps the score back until the answer, so the rating is blind (`game_rate`).
2. **Then the score**, in three columns: **sound %** against the pass mark ("✗ Not yet • pass mark 90 %" or "✓ Passed …", the glyph and the words in the score ink, never colour alone), structure % and combined %, the time (or "Passed at m:ss"), the submits and "assisted"; the **components** in lane SS's order, a badge each (✓ close, ~ a bit off, ✗ far off) and `component_text`'s words (a line cut short shows the whole text as its tooltip); then the latest **hint** ("Hint 2/4: …", two lines), the **vote** beside "Score felt" (**Too low**, **About right**, **Too high**: `game_vote`, the chosen one stays selected), **Disagree…** (a note card over the editor: "What did the score get wrong? (optional)", **Send** logs it at once with `game_dispute`, **Cancel** or Escape closes it), and the next actions: **Try again** (the same code from its start patch: a new round, so the old one ends `new_round`), **Reveal** ("Reveal the target", while the round is open), **New round** (the round's settings with a fresh seed; GR1's "want more").

The drawer never reads a component's `target` or `answer` readings, nor SS's `jnd`, `floor` and `scale`.

**The score overlay** (`recreate/overlay.rs`, `RoundOverlay`, built from each frame's round view): after the first score, a Good / Near / Bad ring with its glyph on every row of the structure score that has a control in the player's patch (`RoundView::rings`, `answer_addr`), live after that (the session's `Rings` jobs). A knob draws its own ring; a switch, a segmented control, a route's matrix cell or route-list row and a module's enable toggle get `overlay::paint_score` (a thin outline and the glyph at the top right). Route rows (`lfo1 -> filter1.cutoff`) and enable rows (`osc2.enabled`) are read with `RingAt::parse` (the session's `rings()`, added beside `knobs()`). A row whose control the player lacks (a missing route) has nothing to ring. The `dim_unused` assist (the settings page's "Dim the modules the target leaves alone") dims every module none of whose parameters the target randomized (`RoundView::params`), and the built-ins' unrandomized parameters; nothing is ever hidden. **It counts as an assist** (lane HINT, FU1): once the dimming shows in a round (the assist on, the target's parameters known, a phase the session accepts), `Recreate::set_round` queues `GameIntent::DimUnused` once, the shell applies it with `game_dim_unused`, and the round is assisted from then on (`RoundView::dim_unused`, `AssistsUsed::dim_unused` in the round log), even if the assist is turned off later.

**Hint highlighting** (lane HINT, FU1; `recreate/overlay.rs`, `RoundOverlay::point`). The latest hint (`RoundView::hint`, kept on the coach's ladder by the session) marks what its text names on the player's controls: an outline in `color.accent.value` (`stroke.thick`, a `space.tight` gap outside the control, `radius.small`), with ↑ or ↓ (`glyph::RAISE`, `glyph::LOWER`) inside its top left corner when the text says raise or lower; never a value, never colour alone for the direction. `Overlay` grew `ParamOverlay::hint` (a `Pointer`: `Here`, `Raise`, `Lower`) and `module_hint`, which `overlay::apply` copies to `ParamView::hint` and `CardView::hinted`; `card.rs` paints them with `overlay::paint_hint`.

| Level | The text says | Marked |
| --- | --- | --- |
| 1 | which aspect is off, and how | nothing (an aspect is not a control) |
| 2 | which module (or region) | the module's card; nothing for a region (the voice settings, the modulation routes, general advice) |
| 3 | which parameter, and which way | the parameter's control, with ↑ or ↓ (a categorical parameter or a route's destination: the outline alone); its card instead while "More" hides the control (`CardView::hint_behind_more`) |
| 4 | a value range (a route: its source too) | as level 3; a route hint also outlines its source module's card |

The mark clears when the next hint replaces it, at a reveal (the answer is loaded), and when the round ends. `Close` hints (nothing to fix) point at nothing. A control drawn outside a card (the header's master knob) is not marked.

**The settings page** (**Settings…**) is a sheet over the body, like the preset browser's: the level (1–5, a preset's settings) and the difficulty (0–100), an amount per category (Oscillators, Filter, Envelopes, Modulation, Effects, Structure: "Auto" follows the level, else 0–100 %), Musical (0–100 %), the template (this build has the starter superset only) and the assist; **Start round** starts from them. It makes a `RandomSpec` and nothing else (`Settings`, integers only, the generator and the seed left to the shell); a level's settings give a short code, any other a long one, and the page says which.

**No peeking** (`tests/recreate.rs::no_target_value_or_reading_is_visible_or_reachable_until_reveal`): during a level-3 round, scored and hinted, no text the editor shows contains a randomized target value as the editor writes it ("989 ms", "587 Hz", "+12 st") or a component reading (whole tokens, three significant digits or more). It reads every painted galley (tooltips, menus and popups included) and every accessible label, value and description, while hovering each of the 50-odd controls with the drawer up and again with the overview up, with the End menu open and with each randomized control's right-click menu open. Values the start patch shows anyway are the player's own and are left out. After Reveal the same check finds the target's values on the knobs, so it can see them.

**A silent target is dealt again** (R1/S1 lane INT; lane GC found that a silent target scores any silent answer 100 %). `prepare_round` refuses it and the round breaks with `GameError::SilentTarget` before anything loads. For a round the editor dealt (a level's **Start**, **Start round**, **New round**), `set_round` queues the same settings again (`New(spec)`, a fresh seed from the shell), once per broken round and up to `MAX_REDEALS` (8) in a row, with "That target was silent: dealing another…" on the status line; after eight it stops and asks for more Musical. A pasted code or **Try again** stays broken with the hint. The broken round was never played, so it leaves no round-log line (`state.rs` tests; `tests/recreate.rs::a_silent_target_is_dealt_again_with_a_fresh_seed`).

**The seam**, unchanged from SKR (`set_round`, `apply_game`, `hold_scores`, `Intent::Game`): `Recreate::push` keeps view changes (`GameIntent::View`: the mode, the level, the code field's text, Start, the settings page, the drawer, the note) and queues the round's intents for the shell: `Start { level }`, `New(spec)` (the seed replaced by the shell's), `Code(code)`, `Listen`, `Submit`, `Rate`, `Hint`, `Reveal`, `Vote`, `Dispute(note)`, `End`. A pushed intent asks egui for one more pass, so the change shows without waiting for input.

**Repaint.** The Recreate UI asks for no frames of its own: with a scored round live, the drawer up and the rings drawn, an idle editor's frames ask for `Duration::MAX` (`an_idle_editor_with_a_scored_round_asks_for_no_frames`); the timer's next second and settled jobs are the shell's `RoundView::wake_at_ms`, as before.

**Accessibility labels** (kittest finds the controls by them): "Recreate", "Level 1"…"Level 5", "Challenge code", "Start", "Settings…", "Leave Recreate"; "Level n of 5", "Target ▶", "Mine ▶", "A/B", "Time m:ss", "Submit", "Hint", "Hint level n of 4", "Reveal", "End", "End and restore my patch", "End and keep this patch"; "How close is yours to the target?", "Rate 1"…"Rate 5", "Skip"; "Sound n %", "✓ Passed • pass mark 90 %" or "✗ Not yet • …", "Structure n %", "Combined n %", each component as "<badge> <words>", "Hint n/4: <text>", "Too low", "About right", "Too high", "Disagree…", "Dispute note", "Send", "Cancel", "Try again", "Reveal the target", "New round", "Show modulation", "Show results"; the settings page's "Recreate settings", "Close settings", "Settings level 1"…"5", "<Category> follows the level", "<Category> amount", "Template", "Dim unused modules", "Start round".

| Token (`theme/sizes/recreate.rs`) | Value | For |
| --- | --- | --- |
| `size.recreate.level_dot` | 14 | one level dot (a click target before a round); also a component row's badge column |
| `size.recreate.code_field` | 132 | the code field |
| `size.recreate.timer` | 40 | the timer, fixed so the bar does not shift |
| `size.recreate.drawer_score` | 180 | the drawer's score column |
| `size.recreate.drawer_component` | 188 | one of the two component columns |
| `size.recreate.drawer_row` | 16 | a component row, a line of the drawer's text |
| `size.recreate.settings_width` | 560 | the settings page's card |
| `size.recreate.settings_label` | 120 | its label column |
| `size.recreate.settings_slider` | 220 | its sliders |
| `size.recreate.dispute_width` | 320 | the "Disagree…" note's card |
| `size.recreate.dispute_rows` | 3 | the note field's rows |

Glyphs: `DOT_ON` ●, `DOT_OFF` ○, `PLAY` ▶, `GOOD` ✓, `NEAR` ~, `BAD` ✗, `CLOSE` ×, `BULLET` •; colours `score.good`, `score.near`, `score.bad` with their glyphs.

**Frame cost** (`examples/frame_bench.rs`, a fourth argument `recreate`: every case inside a level-5 round whose patch is the case's, scored once and hinted; callgrind on release, (I(250) − I(50)) / 200, measured on the same machine and build):

| Case | No round | In a round |
| --- | --- | --- |
| P1 default, idle | 6.83 M | 7.03 M |
| Maximum patch, idle | 9.10 M | 8.95 M |
| Maximum patch, live with the spectrum, minus `engine` (10.43 M) (the worst case) | 9.70 M | 9.41 M |

The drawer costs less than the overview it replaces at the maximum patch (eleven tiles and a scrolling route list), so a round's worst case keeps 5.9 % of the proposed 10 M limit. Without a round the worst case is 9.70 M on this build (AM measured 9.58 M before PBU's header and the later merges).

**Tests** (`crates/enth-ui/tests/recreate.rs`, through a real `Session`, the real `sound/1` and `structure/1` scorers on the test's worker, by label): a round from Recreate to End through the bar and the drawer (the A/B switch, the rating held before the score, the drawer's numbers and components against the report, the rings against the rings, a hint, the vote, a dispute line with its note, Reveal, End and restore, the round log's lines as JSONL); Skip; the rings following the player's edits without a submit; a level-5 round's route and enable-toggle rings, and the overview switch; Try again and New round (`new_round` log lines); a pasted code and a bad one; the settings page's custom spec (a long code) and the dim assist; the dim assist marking the round assisted and the round log saying so, against the same start without it (lane HINT); a hint highlighting what its level names, level by level, cleared by a reveal and the end (lane HINT, `a_hint_highlights_its_control`); nothing scrolling at 1200 × 800; no bar without a shell that plays rounds; no peeking; no idle frames; snapshots; REAPER's click points. Snapshots (lavapipe, 1200 × 800): `recreate_before.png` (the bar before a round), `recreate_round_live.png` (a round, Target ▶ on), `recreate_rating.png` (the blind rating), `recreate_results.png` (the drawer and the rings), `recreate_settings.png`, `recreate_hint.png` (lane HINT: a level-3 hint's outline and ↑ on its knob). Unit tests: `state.rs` (the hold, Start from the code field or the settings, the drawer's rules), `settings.rs` (presets give short codes, custom settings are integers and valid), `overlay.rs` (rings on params, routes and toggles; the dim assist; what each hint level marks, lane HINT). The plugin's `tests_game.rs::the_vote_and_the_dispute_reach_the_round_log_file`: through the real worker, the dispute line is in the file at once and the `end` line carries the vote and the rating.

**Left for later:**

- ~~The `dim_unused` assist is an editor setting: `AssistsUsed` has no field for it, so a dimmed round is not marked assisted.~~ Closed by lane HINT (FU1): `AssistsUsed::dim_unused` (additive, written only when true), recorded by the session (`game_dim_unused`) once the editor sees the dimming show; the round log and the results carry it.
- ~~The coach's structured fields (`aspect`, `module`, `param`) could highlight a module or knob, but `CoachHint` carries only the level and the text.~~ Closed by lane HINT (FU1): `CoachHint` carries them (docs/session.md, "What a hint points at"), the plugin's hint job fills them, and the overlay marks the hinted control ([Hint highlighting](#hint-highlighting-lane-hint-fu1)).
- A route hint's matrix cell and route-list row are not marked: the results drawer covers the overview while a hint shows. Its destination control and source card are.
- No "Daily" seed yet (the code field takes a daily's code when one exists).
- Route rings show only while the overview is up (the drawer takes its slot); a route the player lacks has no ring anywhere, only its share of structure %.

## The Learn view (lane LUI)

**A lesson is a layout plus an overlay: the lesson pane beside the cards, and the step's view laid over them.** Lane LUI replaced lane SKL's minimal panel (F2l) with the view in `editor/learn/` (feature `learn`, which the plugin's `learn` feature turns on). The design of record is [learn.md](learn.md#the-learn-view); the lesson types, the player and the markdown AST are `enth-learn`'s, the live binding `enth-session`'s ([session.md](session.md#learn-lane-sl-r2)).

**The seam**, copied from Recreate: `Editor::set_lesson(LearnEdit::lesson())` before every frame (the session's read model), `Editor::set_library(&Library, Progress)` when the window opens (the shell's installed packs, the refused ones, and the progress file; SKL's `set_lessons` still works), `Intent::Learn(LearnIntent)` while drawing, and `Editor::apply_learn(&mut dyn LearnEdit)` after the frame. The intents: `Toggle` (show or hide the pane), `Browse(bool)`, `Shelf`, `Start(i)`, `Next`, `Back`, `Hint`, `Show`, `Choose(i)`, `Listen(Option<i>)`, `Flip` (A/B), `Review` (a placeholder for lane REV) and `End(Restore | Keep)`. A queued intent asks egui for one more pass, never a timer. The plugin's glue (`Glue::round_frame`, `Glue::set_lessons`) does all of it ([plugin.md](plugin.md#learn-lane-skl-r2)).

### Where the pane goes

**The built-in layout `learn`** (`layout/learn.json`, in `layout::BUILTIN` with the `learn` feature) is drawn while the pane shows, unless the editor's own layout places the `lesson` region itself (then that layout is kept, with the pane in its region):

| Row | Holds |
| --- | --- |
| `size.bar.header` | the header and the output region, as in layout 1: a lesson's round keeps the whole header for its challenge bar |
| share | the cards column (macros, then layout 1's signal row of sources and the filter lane, the modulator band, the effects lane at its right) and the **`lesson` region**, `size.learn.pane` (320 px) wide |
| `size.learn.overview_height` (98) | the overview **across the whole window** (1,184 px at 1200 × 800, wider than layout 1's 956), so a round's results drawer and the route list keep their width |
| `size.bar.status` | the status line |

**Hiding the pane is the collapse.** The pane's **Hide** (and the header's entry) toggles it: hidden, the editor draws its own layout over the whole window, the lesson goes on (its overlay stays), and the header's entry reads **Lesson n/m**; a click brings the pane back. In Recreate mode the entry steps aside for the challenge bar, as the Presets group does, so a lesson's Recreate step keeps the pane (its **Hide** is disabled there).

**Focus.** A step whose view says `layout = "focus"` shows, beside the pane, only the modules it highlights or leaves editable (the built-ins stay); with the pane hidden, focus gives way and the whole patch shows. A module the step hides (or focus leaves out) also loses its source tile and its routes in the overview.

**The overflow report at 1200 × 800** (`tests/learn_view.rs`, `Editor::report()`):

| Case | What does not fit |
| --- | --- |
| The steps of How a lesson works and Low-pass cutoff that hide what they do not use or are focus steps (6 of their 16) | nothing |
| Their Recreate steps after Submit (the results drawer in the overview row) | nothing |
| Their steps with no start of their own that carry the player's patch over (7 of 16; the test's player has Init) | nothing for Init; a patch the size of the P1 default overflows as the next rows say |
| Their Listen and Choose steps right after a round, which carry the round's full starter over (3 of 16) | the modulator band by 276 px sideways (report 03's figure) and the effects lane by 84 px |
| The P1 default with the browser open | the same, and the browser's list scrolls by 4 px in the pane |
| The pane hidden (layout 1), the P1 default | nothing |
| The maximum patch, the pane open | every card region scrolls on its own (sources 104, filter lane 723, effects 546, the band 1,188 sideways) and the route list by 63 |

So a lesson fits beside the pane when its steps hide what they do not use: the content lanes should give a step that carries a full starter over (the Check steps after a round, a Name step on the player's own patch) a `hide` list or `layout = "focus"`; a player can always hide the pane.

**Beside the Create panel** (lane SK3's minimal panel, feature `create`; merged with this layout by lane SK3M). The Create panel is not a layout region: while it is open it takes a column `size.create.pane` (320 px) wide at the window's right edge, `space.gap` in, **under the layout's top row**, and the layout places the rest left of it (`frame.rs`, `beside`: the regions of the header's row come from the layout over the whole window, the others from the layout over what the column leaves, so the header keeps its width as it does for a lesson's round). A layout whose header is not in its top row, or whose regions would then overlap (a custom one), goes left of a column the window's height. The four cases: neither (the editor's layout over the window), Learn only (the layout `learn`), Create only (the editor's layout beside the panel) and both (the layout `learn` beside the panel: the lesson pane sits left of the Create panel). A file dropped anywhere on the window goes to Create whenever the shell says create is available, whatever is open (it opens the panel). The overflow report lists the pane's scroll as `learn` and the panel's as `create`.

| At 1200 × 800 (`tests/learn_create.rs`) | What does not fit |
| --- | --- |
| Init, in each of the four cases | nothing; with both open the cards column is 1200 − 2 × 320 − the gaps, and the sources strip is 72 px wide, so its card is squeezed (the strips arrangement squeezes rather than scrolls, so the report does not list it) |
| The P1 default, both open | the band by 604 px sideways, the effects lane by 84, the overview row (as narrow as the cards column) by 51 (tiles) and 63 (routes) |
| The P1 default, Create only | the band by 276 px sideways, the overview by 111 (tiles) and 63 (routes) |

Snapshot: `learn_create_open.png` (both open over Init, lavapipe). A player who wants the cards whole hides one of the two, or widens the window.

### The pane

**With no lesson** (or after **Lessons** over a live one) the pane is **the lesson browser**: a tab per track, **the tour first** (its lessons in order, numbered; a lesson whose pack is missing is listed as not installed), and **All lessons** (each pack's title, id, version and lessons in order). A row is **Start** (**Restart** for the live lesson), the title, its progress ("3/10", "✓ 10/10" once every step is done, in the score ink with the glyph) and the objective. Refused packs are listed with their reasons. **Review** is there and disabled ("comes in a later update") until lane REV feeds it. The progress is the file the shell handed over, plus the steps the view saw pass since (the file stays the shell's: `LearnRequest::Record` writes it).

**In a lesson**, top to bottom: the title row (the lesson's title, **Lessons**, **Hide**); the progress row (a dot per step: the current one in the value accent, done ones in the label ink; "Step n of m • <kind> • <state>", with Listen, Read, Try, Recreate and Choose for the kinds and "✓ Passed", "Checking…", "Lesson complete" for the state); the step's text, which scrolls; and **the controls, anchored to the pane's bottom** so they stand at the same place whatever the text (the REAPER `lesson` scenario clicks them by position):

| From the bottom up | What |
| --- | --- |
| The way on | **◀ Back**, **End** (a menu: **End and restore my patch**, **End and keep this patch**), **Next ▶** (**Finish ▶** on the last step) |
| Do and Choose | **Hint** and **Show me** (both mark the step helped) |
| Choose | a button per answer, under "Your answer:" |
| Listen and Choose | **■ Stop** and **A/B** (the other of the last two examples played, at the same position: they share the Target ▶ slot), above them a **▶** per example in rows that fit |
| Above | the verdict ("✓ Right.", "✗ Not that one: listen again.", "✓ The answer is B."), the latest hint in the value accent, "Working out a hint…", the last refusal in the bad-score ink |

A **Recreate** step is played with the Recreate UI unchanged (the challenge bar in the header, the rings, the results drawer in the overview row); the pane keeps its text and a line that says where the round's controls are. Its overlay is the round's laid over the step's (`overlay::Layered`: hidden beats dimmed beats highlighted; rings, the round's hint and the dim assist from the round; locks from the step).

**The text** is `enth-learn`'s block AST (`markdown::parse`), drawn with tokens only; the pane never sees markdown:

| AST | Drawn as |
| --- | --- |
| Heading 1, 2–3 | `text.title`, `text.strong` |
| Paragraph; list (bulleted, numbered, nested) | runs that flow and wrap, `text.body`; `•` or `n.` markers in the label ink, items indented by `space.section` |
| Strong, emphasis, code | `text.strong`; italics; `text.value` on `surface.recessed` |
| `enth:<selector>` link | underlined in the value accent; **hovering it outlines its control on the cards** (a route: its destination and its source's card; a module: its card). The pane is drawn before the cards, so the outline shows in the same frame |
| `enth:play/n` link, `play n` widget | plays example (or option) n, or stops it while it plays ("Play <label>" for the widget) |
| `https:` link | opens the browser through egui's `OpenUrl` (no network code) |
| `control <param>` widget | the live knob, under its own id (`ids::param(addr).with("lesson")`), so it never clashes with the card's: the same gestures (one undo step a drag), dimmed when locked; hovering it outlines the card's control. A switch or a short enum shows its value as a chip; a module, route or macro selector shows as a chip that outlines it |
| `scope` widget | the live scope from the telemetry the output region draws, `size.learn.scope_height` tall |
| `shape <module>` widget | an envelope's stages or an LFO's wave from the live patch, `size.learn.shape_height` tall |

### The overlay's highlight and lock

Additive in `overlay.rs` (docs at the top of the file):

- **`Emphasis::Highlighted`**: what a step highlights (`view.highlight`) and the hovered link. A control gets a thin outline in the value accent a tight gap outside it (`paint_highlight`, `stroke.thin`); a module's highlight outlines its card. Static, so it asks for no frames. A revealed hint's thick outline still reads stronger.
- **`Overlay::locked` and `Overlay::module_locked`** (default false): what the step locks (`view.locked`). The control is drawn dimmed with a **lock glyph** on its top right corner (`paint_lock`: a shackle over a filled body, `size.learn.lock`, the label ink; Inter has no padlock), a locked module's card carries it on its title row, left of "More" or "…". Hovering a locked control sets the not-allowed cursor and the status line reads "<path> <value> Locked in this step: the lesson keeps it where it is, so you hear only what it teaches", as a refused drag-to-modulate target names its reason. **The session refuses the edit anyway** (`SessionError::Locked`), and its hint shows at the status line's right end.
- `ParamView::locked`, `CardView::highlighted` and `CardView::locked` carry them to the cards (`cards/view.rs`, `cards/card.rs`, additive).

**Repaint.** The Learn view asks for no frames of its own: a settled lesson's frames ask for `Duration::MAX` on every step kind (an example playing, a passed Try step, the browser); job results and the lesson's settle moment (`LearnView::wake_at_ms`) wake the editor through the shell. In a Recreate step the round's own clock asks for its next second, as in any round.

**Accessibility labels** (kittest drives the view by them): "Learn" or "Lesson n/m" (the header's entry); "Lessons", "Hide lesson", "Hide lessons", "Back to the lesson", "Review", the track titles and "All lessons" (tabs), "Start <title>", "<title> progress d/n"; "Step n of m <state>" (a status); "Back", "Next", "Finish", "End lesson", "End and restore my patch", "End and keep this patch"; "Listen <label>", "Stop listening", "A/B", "Choose <label>", "Hint", "Show me"; links by their text; "Play <label>" for a play widget.

**Tests.** `crates/enth-ui/tests/learn_view.rs`, over a real `Session` with the worker's real functions (`evaluate_sounds`, `prepare_challenge`, `score_round`, `score_structure`) and the `basics` pack built from `content/lessons/basics`: **two lessons completed by label** (How a lesson works and Low-pass cutoff, every step kind: examples played and A/B flipped, the Try steps walked to the witness, the rounds matched and submitted, the answers chosen; every step recorded done; the browser shows both complete); the browser (the tour first, a missing pack's lesson, All lessons, the disabled Review); hiding the pane and the header's "Lesson n/m"; a step's highlight, locks and focus; a hovered link's outline; links and every widget (a play link, a play widget, the text's live knob turning the cutoff as one undo step); a locked control's reason; **no peeking** in a lesson's Recreate step (no text the editor shows, hovering every control, the Hint and the End menu, contains the drawn value; after Reveal it does); **the overflow report**; **no idle frames**. Snapshots (lavapipe, 1200 × 800): `learn_browser.png`, `learn_name_link.png` (a link hovered), `learn_try_focus.png` (a focus step's outline), `learn_round.png` (a lesson round, the lock glyphs), `learn_locked_hidden_pane.png` (the pane hidden on a step that locks three modules). `tests/learn_panel.rs` (lane SKL's) plays the example pack through the pane and pins the REAPER points; `tests/learn.rs` the header's entry; unit tests in `overlay.rs` and `learn/state.rs`.

| Token (`theme/sizes/learn.rs`) | Value | For |
| --- | --- | --- |
| `size.learn.pane` | 320 | the lesson pane's column in the layout `learn` |
| `size.learn.overview_height` | 98 | the overview row across the window in the layout `learn` (what layout 1 leaves its overview) |
| `size.learn.lock` | 9 | the lock glyph's side |
| `size.learn.scope_height` | 64 | a `scope` widget in a step's text |
| `size.learn.shape_height` | 56 | a `shape` widget |
| `size.learn.progress_width` | 56 | the browser's progress column |

Glyphs: `STOP` ■ (new, Inter covers it), `PLAY` ▶, `PREVIOUS` ◀, `NEXT` ▶, `DOT_ON` ●, `DOT_OFF` ○, `BULLET` •, `GOOD` ✓, `BAD` ✗.

**Lane LFU's follow-ups (2026-10-21):**

- **"How close am I?"** joins a lesson's Recreate step after a Submit that did not pass (`LearnView::closest`, the session's `LearnJobKind::Closest`; [learn.md](learn.md#learn-follow-ups-lane-lfu)). From the bottom up, in rows that fit: **How close am I?** (enabled while `closest.available`), **Look harder** once it has answered (about 1,000 renders), **■ Stop** while it searches ("Stop searching"; cancels), and **Try a new one** when a sibling redo is due (after a reveal, the coach's last hint or a passing answer). Above them, in the value accent: "Looking for the nearest passing sound… 42 of 300 renders" while it runs, then "N moves away: Cutoff (filter1.cutoff) down a little; …" (the session's `move_size`: a touch, a little, a lot, or change for a switch), with each control's exact values ("1.20 kHz → 850 Hz") only once the round is revealed, or "No passing sound found near yours…" when it found none. **On the cards**, `LessonOverlay` rings each control the answer would move (Good when it stays, Near for a touch or a little, Bad for a lot or a switch) with the hint's pointer (↑, ↓, or the outline alone); in `overlay::Layered` the step's ring and pointer now read over the round's when it has one. Labels: "How close am I?", "Look harder", "Stop searching", "Try a new one". `tests/learn_view.rs`'s `how_close_am_i_after_a_failed_submit` plays it by label, no value before Reveal.
- **The browser's bottom row:** **Open packs folder** (`LearnIntent::OpenPacksFolder`; the shell, which knows the folder through `Editor::set_packs_folder`, makes it if missing and opens it through egui's `OpenUrl`, read with `Editor::take_open_packs_folder` after the frame, since the editor does no I/O; disabled without a folder) and **Get more lessons** (`OpenUrl` to the releases page, `browser::RELEASES_URL`). The list scrolls above them, so its first row stays where it was. Snapshot `learn_browser.png` updated, and `learn_round.png` for the pane's line about the button.
- The plugin bundles the basics pack and turns `learn` on by default ([plugin.md](plugin.md#learn-lane-skl-r2)).
- One Show me now satisfies a `touched` predicate (the player notes the knob's value before its op).
- The basics ear questions after a round are focus steps with nothing highlighted, so they fit beside the pane.

**Left for later:**

- Review waits for lane REV. *(Since built by lane REV: the browser's **Review • N due** plays the weakest due skill's round and is disabled while a lesson is live or before a lesson with a round is passed; [learn.md's Review](learn.md#review-lane-rev), `editor/learn/browser.rs`, snapshot `learn_review.png`.)*
- A route selector in a link outlines its destination and its source's card, not its matrix cell or route-list row; a macro selector outlines nothing (the macro bar takes no overlay).
- A Name step on the player's own patch (no start of its own) still carries a full starter over and overflows beside the pane (LUI's report); a content lane can give it a `hide` list or `layout = "focus"`.

## The Create view (lane PUI)

**"Create from sound" is a column at the window's right edge: drop a sound (or browse to one), watch K candidates grow, hear each against the upload, take one, undo it.** Lane PUI replaced lane SK3's minimal panel (F2s) with the view in `editor/create/` (feature `create`, which the plugin's `create` feature turns on; off by default). The design of record is [r3.md](r3.md#the-create-view-and-the-session); the job runs in the plugin ([plugin.md](plugin.md#create-lane-sk3-r3)), the state machine is the session's ([session.md](session.md#create-lane-sk3-r3)). Where the column goes, beside layout 1 or beside the lesson pane, is [above](#where-the-pane-goes) ("Beside the Create panel", lane SK3M): `size.create.pane` (320 px) wide, under the layout's top row.

**The seam**, copied from Recreate and Learn: `Editor::set_create(CreateEdit::create())` before every frame (a shell that calls it says create is available: the header shows **Create** and a file dropped anywhere on the window is taken), `Intent::Create(CreateIntent)` while drawing, `Editor::apply_create(&mut dyn CreateEdit)` after the frame; and for the folder browser (the editor does no I/O) `Editor::take_create_browse()` after the frame (a `BrowseAt`: the shell's starting folder or a path) and `Editor::set_create_folder(Folder)` with the shell's listing. The intents: `Toggle`, `Choose(path)`, `Start`, `Cancel`, `Audition(branch, which)`, `AuditionUpload`, `Flip(branch)` (the A/B), `StopAudition`, `Use(branch, which)`, `Clear`, `Select(branch)`, `Browse(at)`, `CloseBrowser`. A queued intent asks egui for one more pass, never a timer. The plugin's glue (`Glue::round_frame`) does all of it, the listing with `create::browse::list_at` (one `read_dir` on the GUI thread).

### The column

Top to bottom, on `surface.card`:

| Part | What |
| --- | --- |
| Title | **Create from sound** and **×** (Close; a running job goes on) |
| The drop zone | `size.create.drop_zone` (56) tall on `surface.recessed`: "Drop a sound file here" and "WAV, AIFF, FLAC, Ogg Vorbis or MP3, any length" (AIFF added by lane INTa); the chosen file's name and, after the intake, its length and note ("0.80 s • A3"; a clip under 0.1 s in milliseconds); a file dropped while another file's job runs is only chosen, and says "Start it once pluck.wav is done". While files are dragged over the window it lights (`accent.value_wash`, a `stroke.thin` outline in `accent.value`) and reads "Drop to make patches from this sound" |
| **Browse…**, **Start** | Browse… shows the folder browser (drawn selected while it shows). **A drop or a pick starts a job at once** when none runs; Start runs the chosen file again, each run with the next seed, so it gives new candidates. The last refusal follows in the bad-score ink |
| The intake's findings | How it decoded ("WAV, 16-bit, 48000 Hz, mono"); **which proposal model the job uses, or why none** (R3b, lane PUIb: the shell's `CreateIntake::model` line, "Model: propose-v0-pilot (enth-model-propose-v0-pilot.bin)", "No model: search alone (no enth-model-*.bin found)", "No model: enth-model-x.bin was refused (model_mismatch); search alone"; nothing from a shell that looks for no model); "Note A3 (220.0 Hz) • held 0.30 s of 0.60 s" ("No stable pitch" when unpitched); the trim and the damage in one line ("Trimmed 12 ms before the onset • short: kept whole • low-passed at 11.0 kHz • clipped • noise at −60 dBFS", what applies); then each warning of the intake, a bullet in the near-score ink (a chord, a phrase, several notes, a sound past the cap, channels that cancel). **A short sound is shown as it is and never refused** |
| The progress | The phase ("Reading the file…", "Searching…", "Cancelling…", "Done", "Stopped", "Could not create") with **Cancel** at its right; one bar (`size.create.progress_height`) for renders against the cap or time against the cap, whichever is further, reading "800 of 4,000 renders • 1 of 60 s"; **the rungs**: "Rungs 8 › 4 › 2", the branches that reached each rung of successive halving so far (the current one in the value accent while it runs), then "• 4 searching" or "• 2 finalists". A failure shows "✗ message (code)" and its hint instead |
| The K tiles | `size.create.tile_columns` (2) to a row, `size.create.tile_height` (40) tall, **in branch order** so a tile never moves while the ranks change: the rank (1 is the closest) in `text.title`, the start's provenance ("heuristic", "template pluck_1osc", "k-NN 3", and in R3b "model 2" for a branch that grew from the model's hypothesis 2, numbered from 1), the distance in `text.value` ("–" before the first evaluation) and the state (proposed, searching, done, pruned; a pruned tile is dimmed and stays playable and usable), "✓ used" on the one taken last. A click selects it (the value accent's wash and outline); with nothing selected the closest is |
| The compare area | The selected tile: "Candidate 2 • k-NN 2", "start 0.510 → 0.170 • 100 renders • rung 1"; **▶ Candidate**, **▶ Start** (the proposal it grew from), **▶ Upload**; **A/B**, **■ Stop** and what was asked for last ("▶ the upload"); **Use this** and **Use the start**; "✓ Took candidate 1: the header's Undo gives your patch back" |
| Notes | The shell's notes on the job (the objective, a missing k-NN bank and where it looked, the model file found or each one refused with its hint, a hypothesis dropped, "model propose-v0-pilot: 6 of its 8 hypotheses became starts, 3 of them branches (inference 0.61 s)", the cap), in the dimmed ink |

While the folder browser shows it takes the place of everything under Browse…: **◀ Up** (the parent folder), the folder's name (its path as the tooltip) and **×**; the list, `size.create.browser_row` (20) per row, scrolling inside itself after `size.create.browser_rows` (22) rows: sub-folders first ("Samples ›", a click opens it), then the sound files (`browser::is_sound`: wav, wave, aif, aiff, aifc, flac, ogg, oga, mp3; a click chooses the file, closes the browser and starts the job), each with its size; a folder that cannot be read shows why. Under it a path field ("or type a path") with **Choose**: a sound file's path is chosen, anything else opens as a folder. The browser opens at the listed folder, else the chosen file's, else the shell's start (the plugin: Documents, else home). Nothing is filtered by length or size.

### Audition, A/B and Use this

- **Each ▶ plays once** through the plugin's audition path (lane SK3's one-shot `AuditionSlot`, mixed into the plugin's output): the candidate or its start rendered with the job's performance at the host's rate, or the upload (the job's scored clip, mono, resampled to the host's rate). Nothing plays unless a button is pressed, and the editor never plays audio itself.
- **A/B** plays the selected candidate if the upload was asked for last (or nothing was), else the upload, so repeated clicks alternate; after **▶ Start** the A/B alternates the start and the upload. **Level-matched** by RMS in the plugin: a candidate louder than the upload plays at the upload's level, and the upload plays at the level of the candidate auditioned last when it is the louder; nothing is amplified. Each side restarts from its beginning (a clip is at most 3 s, `create::SCORE_S`). The button is drawn selected while the upload was asked for last.
- **Use this** (and **Use the start**) is `CreateEdit::create_use`: one undo step through the session's preset load (one plan, no host writes, `PatchEdited`), the patch named after the file's stem; refused while a lesson is live, with the hint on the panel. The header's **Undo** gives the patch before back.

**Repaint.** The view asks for no frames of its own: the progress bar never animates, and the shell's posts wake the editor (each post asks the open editor for a repaint). In every state an idle editor asks for `Duration::MAX` (`an_idle_create_view_asks_for_no_frames`).

**The model's line and tiles** (lane PUIb, `tests/create_model.rs`, by label over a real `Session`): the findings name the model, or say why there is none; a tile from hypothesis k reads "model k+1" and the compare area "Candidate n • model k+1"; the end's note says what the model gave; no line from a shell without a lookup; an idle view with the model's line and tiles asks for no frames. The line is drawn only when the shell sends one, so the existing snapshots are unchanged.

**Accessibility labels** (kittest drives the view by them): "Create" (the header), "Close create", "Drop zone: <file> <length> • <note>", "Browse files", "Start create", "Cancel create"; the phase ("Searching…"), the bar's text ("800 of 4,000 renders • 1 of 60 s"), "Rungs 8 › 4"; "Select candidate n"; "Candidate n • <provenance>", "Play candidate n", "Play the start of candidate n", "Play the upload", "A/B candidate n against the upload", "Stop audition", "Played last: <what>", "Use candidate n", "Use the start of candidate n", "Took candidate n"; the browser's "Parent folder", "Close browser", "Open folder <name>", "Choose <name>", "Sound file path", "Choose file"; each finding and warning by its text.

| Token (`theme/sizes/create.rs`) | Value | For |
| --- | --- | --- |
| `size.create.pane` | 320 | the column's width (lane SK3) |
| `size.create.drop_zone` | 56 | the drop zone's height |
| `size.create.choose` | 88 | the room the browser's path field leaves for **Choose** |
| `size.create.tile_height` | 40 | one candidate tile |
| `size.create.tile_columns` | 2 | tiles per row (K = 8: four rows) |
| `size.create.rank_width` | 18 | a tile's rank column |
| `size.create.browser_row` | 20 | one row of the folder list |
| `size.create.browser_rows` | 22 | the rows the list shows before it scrolls |
| `size.create.progress_height` | 16 | the progress bar |

Glyphs (existing): `CLOSE` ×, `PLAY` ▶, `STOP` ■, `PREVIOUS` ◀, `PATH` › (a folder, the rungs), `SEND` → (start to candidate), `BULLET` •, `GOOD` ✓, `BAD` ✗, `MENU` … (a cut name).

**The overflow report at 1200 × 800 with Create open** (`tests/create_view.rs`, `Editor::report()`; the four states: the empty panel, the browser over 31 entries, a job running with K = 8 after round 2 with the intake's chord warning and a long bank note, a finished job with the A/B and a candidate taken):

| Case | What does not fit |
| --- | --- |
| Init, every state | nothing |
| The P1 default, the empty panel, the browser, a job running | the layout beside the panel, as without the view (lane SK3M's figures): the band by 276 px sideways, the overview by 111 (tiles) and 63 (routes); **the panel itself never scrolls** (its list scrolls inside itself) |
| The finished job (its candidate taken, Init with another level) | nothing |

**Tests.** `crates/enth-ui/tests/create_view.rs`, over a real `Session`, the test playing the shell (the intake of a real clip through `enth_analysis`'s intake, re-exported by the session: a synthetic chord, so its warning is real; rounds of K = 8 through rungs 8, 4, 2; the end) and listing folders from a table, by label: **the flow from a drop** (the drop starts the job, the intake's findings and warning, eight tiles, the rungs, a tile selected, its candidate and start played, the A/B both ways, Stop, Done, **Use this** as one undo step named after the file, the header's **Undo** back to the patch before, **Use the start** and undo); **the flow from the folder browser** (the start folder, a folder opened, ◀ Up, a typed folder, a file chosen, a 20 ms clip shown in milliseconds and kept whole); **cancel** (a file dropped meanwhile waits for Start; one `Cancel` in the click's frame, never twice, "Cancelling…", the cancelled job's candidates still usable) and a failure's message and hint; **the overflow report**; **no idle frames**; snapshots (lavapipe, 1200 × 800): `create_empty.png`, `create_browser.png`, `create_running.png`, `create_done.png`. `tests/create_panel.rs` (lane SK3's, kept: the typed path now lives in the browser) and `tests/learn_create.rs` (lane SK3M's; its snapshot `learn_create_open.png` redrawn with the view's empty panel) still pass. Unit tests: `state.rs` (a choice starts a job, a second Start takes the next seed, the browser asks once), `browser.rs` (the listing's order and filter, sizes), `drop_zone.rs` (lengths, note names), `progress.rs` (counts). The plugin: `tests_create.rs`'s `the_upload_plays_level_matched_against_the_candidate_played_last` (a real job; the candidate, then the upload, within 5 % in RMS, louder or quieter) and the resampler and gain; `create/browse.rs` (a real folder: folders first, sound files only, hidden ones left out, a one-byte file listed); the session: `tests/create.rs`'s `the_upload_plays_once_the_intake_read_the_file`.

**Left for later:**

- The A/B restarts each side from its beginning; a flip at the same position (as Recreate's) needs a two-clip player on the audio thread.
- The settings (K, the time cap) are the session's defaults (8, 60 s); the view has no control for them yet.
- Whether REAPER drags media items into the plugin window as files is still the open check of [r3.md](r3.md#file-input); the browser is the fallback either way.
- The dev window does not run create jobs (it has no `enth-create` thread); the view is exercised in kittest and the plugin.

## The duel card (lane DUI)

**Duels with a friend over any chat: share a round's result, paste their whole message, check their answer on this computer, compare side by side, open their sealed answer once you have played, load it as one undo step, and find every duel again in the history.** Lane DUI grew lane SKD's minimal card (F2d) into the card in `editor/duel/` (feature `duel`, which the plugin's `duel` feature turns on; off by default until INT5's friend build). The design of record is [duels.md](duels.md); the jobs, the salt and the history file are the plugin's ([plugin.md](plugin.md#duels-lane-skd-r5a), `enth-plugin/src/duel/`), the state machine is the session's ([session.md](session.md#duel-lane-se-r5a)).

**Where.** No header entry: **Duel** sits in the overview region's bottom-left corner while the card is closed, and the card takes a column at the window's right edge (`size.duel.pane`, 340 px, under the layout's top row), as the Create panel does; the duel card wins the column while both are open. A duel that opens (a paste, a Share, **Open** in the history) opens the card.

**The seam.** `Editor::set_duel(DuelEdit::duel(), ShellDuel)` before every frame (a shell that calls it says duels are available); `Intent::Duel(DuelIntent)` while drawing; `Editor::apply_duel(&mut dyn DuelEdit)` after the frame; then `Editor::take_duel_asks()` (lane DUI; `take_duel_share()` is lane SKD's and still works) hands the shell a `ShellAsks`: **Share** clicked, a history entry to reopen, the codes a Read imported (so the shell keeps the friend's in the history) and the friend's name typed. `ShellDuel` carries the shell's codes, "making", what it said, the history rows (`HistoryRow`, newest first), the open duel's history entry and its name. A queued intent or ask asks egui for one more pass, never a timer.

**The card,** top to bottom on `surface.card`; the rows of controls never move (the REAPER `duel` scenario clicks them by position, and `reaper_points_hit_their_duel_controls` pins the points):

| Part | What |
| --- | --- |
| Title | **Duel** and **×** (closes the card; the duel stays open) |
| Share row | **Share result** (the shell makes the scorecard and the answer code on its worker, the salt from OS randomness, and keeps them in the history before they reach the card), **Copy message**, **Copy answer** (egui's `copy_text`, inside the click and nowhere else) |
| The paste box | `size.duel.paste_height` (72) tall, "Paste your friend's whole message here (Ctrl+V)"; **Paste** at its right (`size.duel.paste_button`, 52, kept for it) |
| Their row | **Read** (paste-anything on the box: every challenge, scorecard and answer code in a whole chat message, mangled or not), **Verify**, **Use theirs**, **▶ Theirs** / **■ Stop** |
| Play row | **Play** (a Recreate round on the duel's challenge, through the bar's code start) and **Close duel** |
| What was said | the shell's and the session's refusals in the bad-score ink with ✗; "Reading the clipboard… If nothing arrives, click the box and press Ctrl+V." while a Paste click waits |
| **You** | your result ("Passed, 3 submits: 93.4 % sound in 2:41 (reported)"), why it cannot be shared yet, the share message, "Scorecard 51 symbols; answer code 68 symbols: send it once your friend has played." and, **over 64 symbols, "Your answer code is longer than 64 symbols: an enth from before duels calls it "bad length". Your friend needs to update enth."** |
| **Your friend** | their name (optional, "kept on this computer"; at most 40 characters; sent to the shell when the field is left, never in a code); what the last Read found; "Duel on <code>"; **the two results side by side**; their scorecard **as reported**; the sealed answer; the check **checked on your computer**, with both engines or scorer sets when they differ, the commitment, your own patch re-scored on the same check, their time "reported by their app; enth can't check times"; "Their patch is loaded: Undo takes it back." |
| **History** | the newest `history::SHOWN` (8) duels kept on this computer, newest first: "2026-10-24 · 1080-1000-0001-AB · you 93.4 % · them 92.9 % checked here, as reported · from Sam" (their sound **reported** until a check ran), each with **Open** ("Open now" on the open duel); then "N older duels are kept in the history file." |
| About | "Duel codes are for friends. A code proves what a patch scores, not who made it or how long it took. enth's own tools can solve any challenge.", said once |

**Side by side** (`verify::sides`): a grid of three columns (the row's name, **You**, and **Them** or the name you typed), at least `size.duel.side_column` (72) each: sound, passed, submits (up to the compared one), time, assists. Your sound is your round's until a check re-scores it beside theirs; theirs is their scorecard's until the check. **A score says on the cell where it comes from** ("100.0 %, checked", "93.4 %, reported"); times and assists are reported on both sides, which the line under the grid says ("Checked: scored on this computer. Times and assists are reported by each app; enth can't check them."). Each cell's accessible name says whose and what ("Their sound: 93.4 %, checked on your computer", "Your time: 2:41, reported").

**The clipboard.** Written only by **Copy message** and **Copy answer**, inside the click. **Read only on an explicit Paste:** the platform's own (Ctrl+V, ⌘V) into the focused box, which the integration turns into text, or the card's **Paste**, whose click (and no other frame) sends egui's `ViewportCommand::RequestPaste`; the `Event::Paste` that answers it replaces the box's text and is read at once (a box with the keyboard focus takes a paste itself, so it is never inserted twice). egui-winit (the dev window) answers `RequestPaste`, and so does **egui-baseview in the plugin since fork patch 13 (lane FK4)**: it reads the clipboard once per request and hands the next frame one `Event::Paste`. An empty clipboard answers nothing, and the note under the box says to press Ctrl+V in the box instead. The REAPER `duel` scenario pastes the friend's message with Ctrl+V and their answer with **Paste**, through the X clipboard.

**The sealed answer.** Their answer code stays sealed until you have a scored submit or have given up (the session's guard): nothing of it shows, and Verify, Use theirs and ▶ Theirs stay off ("Their answer opens once you have submitted or given up"). A live round with nothing submitted keeps it sealed. **Use theirs** loads their checked patch as **one undo step** through the session's preset load (no host writes), refused while a round is live ("end your round first"); the header's **Undo** gives your patch back.

**The history** is the plugin's file (`<Documents>/enth/Duels/duels.json`, `ENTH_DUELS_DIR`; [duels.md](duels.md#the-duel-history)): the card only lists the rows the shell hands it and asks for **Open**. The shell restores the duel (your result, its salt and codes, so **Copy answer** gives the same code as before, their codes) and checks it again if it was checked; a duel that opens on a code you played before (their message pasted the next day) resumes by itself.

**Repaint.** The card asks for no frames of its own: nothing animates, the shell's posts wake the editor, and a Paste click waits without a timer. Idle, it asks for `Duration::MAX` with a duel checked, its history listed, and a Paste click waiting (`an_idle_duel_card_asks_for_no_frames`).

**Accessibility labels** (kittest drives the card by them): "Open duel card", "Close duel card", "Share result", "Copy message", "Copy answer", "Paste box", "Paste from clipboard", "Read pasted codes", "Verify their answer", "Use their patch", "Audition their patch", "Stop audition", "Play this challenge", "Close duel", "Friend's name", "Open duel <id>"; the grid's "Column: You", "Column: <name>", "<Your|Their> <sound|passed|submits|time|assists>: <value>[, <where it comes from>]"; every line by its text.

| Token (`theme/sizes/duel.rs`) | Value | For |
| --- | --- | --- |
| `size.duel.pane` | 340 | the card's column (lane SKD) |
| `size.duel.paste_height` | 72 | the paste box (lane SKD) |
| `size.duel.opener` | 64 | **Duel** in the overview's corner (lane SKD) |
| `size.duel.paste_button` | 52 | the room kept at the paste box's right for **Paste** |
| `size.duel.side_column` | 72 | the narrowest column of the side-by-side grid |

Glyphs (existing): `CLOSE` ×, `PLAY` ▶, `STOP` ■, `BAD` ✗.

**Tests.** `crates/enth-ui/tests/duel.rs`, over a real `Session`, the test playing the game worker and the duel worker with the real `enth-challenge` functions, by label: the card opens and closes; **copy** (the clipboard written only inside a Copy click); **no clipboard read without Paste** (hovering, typing, every other click and many frames send no `RequestPaste`; one Paste click sends one; its answer is read; a paste into the focused box goes in once); **a whole chat message** pasted, mangled as a phone does it, and read; **the sealed answer** opening after a give-up and after a submit (not while a round runs with nothing submitted); **verify** ("as reported", the commitment matched) and **side by side** by the cells' names; **a load as one undo step** that one Undo takes back; the history rows and **Open**; the name sent when the field is left; the over-64-symbols note; **no idle frames**; the REAPER points. Unit tests: `state.rs`, `history.rs` (dates from day numbers, a row's wording). The plugin's side is in `enth-plugin`'s `duel/history.rs` and `tests_duel.rs` (`a_duel_survives_closing_the_plugin`).

**Left for later:**

- The history has no "forget this duel" control; the file is the user's to edit or delete.
- An answer code over 64 symbols reads "bad length" on an enth from before duels: a code-format question for the R5a release step ([duels.md](duels.md#the-card-and-the-history-lane-dui)), not the card's.

## Tokens (`theme/sizes/editor.rs`, `modulation.rs`, `output.rs`, `add_module.rs`)

| Token | Value | For |
| --- | --- | --- |
| `size.editor.band_height` | 224 | a band of lane cards (layout 1's modulators): one envelope card, the tallest P1 modulator |
| `size.editor.header_knob` | 112 | an inline knob in the header (dial, label, "−12.0 dB") |
| `size.editor.entry_width` | 120 | the "Type value…" field; also the menu's minimum width |
| `size.editor.entry_hint_width` | 240 | where the field's error hint wraps |
| `size.modulation.depth_handle` | 3 | radius of a knob's depth handle (ED2) |
| `size.modulation.depth_reach` | 6 | its grab radius, and the wash behind it while hovered or dragged |
| `size.modulation.route_min_width` | 176 | the narrowest column of the route list ("Env 2 → Filter 1 Cutoff +40") |
| `size.modulation.route_source_width` | 40 | the source name at the start of a route-list row |
| `size.output.width` | 208 | the output region in a layout (layout 1: right of the header) |
| `size.output.meter_width` | 4 | one channel's meter bar |
| `size.output.meter_floor_db` | −60 | the bottom of the meters and the spectrum |
| `size.output.meter_fall_db_per_s` | 24 | how fast a meter falls back after a peak |
| `size.output.scope_samples` | 1,024 | the samples the scope draws and the spectrum analyses (21 ms at 48 kHz; a power of two) |
| `size.add_module.menu_width` | 20 | a card's "…" menu button at the right end of its title row (lane AM) |
| `size.add_module.slot_height` | 32 | the dashed "+ Add" slot under a strip or lane region's cards |
| `size.add_module.slot_width` | 40 | the dashed "+ Add" slot after a band's cards |
| `size.add_module.dash`, `dash_gap` | 4, 4 | the slot outline's dashes and the space between them |
| `size.window.min` | 1200 × 800 | the smallest logical window (lane AM raised it from 1000 × 680; [above](#the-minimum-window)) |

The depth arcs and the wire use the existing strokes and the modulation accent (`accent.modulation`, `accent.modulation_wash`); the editor asks for live frames at `motion.scope_hz` (lane AM). The "+ Add" slot is a `stroke.thin` outline in `line.grid` (`text.label` while hovered), no fill; its glyphs are `glyph::ADD` and `glyph::MENU`.

## The dev window

`examples/dev_window.rs`, feature `dev-window` (eframe on glow, X11): the real editor over a real `Session` activated at 48 kHz, the engine rendering 256-frame blocks at real-time pace on its own thread into buffers nobody hears, so plans swap and the return ring drains as in a host. **It opens a window: Xvfb, or the owner's display with work saved; never an agent on the real display** (AGENTS.md).

```sh
xvfb-run -a -s "-screen 0 1920x1080x24" \
  cargo run -p enth-ui --example dev_window --features dev-window -- --patch content/templates/pad_2osc.patch.json
```

Options: `--patch FILE`, `--layout NAME|FILE` (or `ENTH_LAYOUT`), `--theme FILE` (or `ENTH_THEME`), `--frames N` (quit after N frames and print the frame report, including the repaint the last frame asked for: a smoke test), `--dump-layout NAME` (a variant's starting point), `--dump-theme`. Keys: F2, F5, and F3 (ED2) to hold or release an audition note (middle C) so the rings, tiles, scope and meters move. The engine thread turns the tap's wake flag into a repaint, as the plugin's `process()` does.

## Frame cost

`examples/frame_bench.rs`: the editor's egui pass and tessellation, headless, release build, callgrind, (I(250 frames) − I(50 frames)) / 200 (research 02's method):

| Case | ED1 | ED2 |
| --- | --- | --- |
| P1 default, idle (pointer moving over the header, nothing changes) | 6.55 M | 6.59 M |
| P1 default, a knob drag (an edit through the session every frame) | 6.81 M | 6.88 M |
| Maximum patch, idle | 8.74 M | 8.84 M |
| P1 default, live: a held note, telemetry drawn (rings, tile levels, scope, meters) | — | 6.94 M |
| P1 default, live with the spectrum | — | 7.23 M |
| Maximum patch, live | — | 9.09 M |
| Maximum patch, live with the spectrum (the worst case) | — | 9.43 M |

The live cases (ED2) render 1,600 frames of audio before each frame through an active session; the editor's share is that case minus `engine` (the same audio, no editor: 5.91 M per frame at the P1 default, 10.43 M at the maximum patch). ED2's additions cost +0.04 M idle at the P1 default and +0.10 M at the maximum patch (depth handles, range arcs, the route list, the output region); a `ui.scope` per control for the refused-target dimming had cost 0.25–0.4 M, so only a refused control gets one. Everything stays under the proposed 10 M limit; the maximum patch with the spectrum has 6 % to spare.

**Lane AM** (R1/S1; the "…" button on every module card, the region menus, the "+ Add" slots), measured against F0r (`c5efbf2`) on the same machine and toolchain:

| Case | F0r | AM |
| --- | --- | --- |
| P1 default, idle | 6.61 M | 6.73 M |
| Maximum patch, idle | 8.83 M | 8.97 M |
| Maximum patch, live with the spectrum, minus `engine` (10.43 M) (the worst case) | 9.45 M | 9.58 M |

The worst case keeps 4.2 % of the proposed 10 M limit (5.5 % before). A first cut with `Shape::dashed_line` cost 6.98 M at the P1 default: one shape per dash.

Research 02 measured 4.55 M (idle) and 4.90 M (animating) for a generated editor without the macro bar, matrix or overview, and expected 7–8 M for the real one; the spike panel was 6.69 M and 7.64 M. The top costs are epaint's tessellator (stroke and fill, 12 %), `memcpy` (12 %) and glyph hinting (4 %). The proposed gungraun case keeps a hard limit of 10 M per animating frame.

## Tests

| What | Where |
| --- | --- |
| Interaction through a real `Session`, by accessibility label (no renderer): a knob drag is one undo step and the header's Undo and Redo take it back and forth; "Automate" assigns slot 1 and a drag is then one host gesture; "Stop automating"; "Type value…" with units, a note name and the U+2212 minus; a bad value keeps the field with its hint and Escape cancels; "Reset to default"; a macro drag is one gesture and one host gesture; the master knob in the header and voice settings in the effects lane; a disabled module stays editable; envelope handles and knobs stay in sync; the status line's text; ids across two layouts; a drag across a layout switch; no scrolling at 1200 × 800 for the P1 default; the maximum patch scrolls inside its regions; a refused layout keeps the current one | `crates/enth-ui/tests/editor.rs` |
| Layout geometry; the reserved regions with and without their features | `crates/enth-ui/tests/layout.rs` |
| Every built-in parses, checks and round-trips; refusals name the problem; the no-op overlay changes nothing and a lesson overlay dims, hides and scores; undo step names | unit tests in `layout.rs`, `overlay.rs`, `describe.rs` |
| Whole-editor snapshots, layout 1 at the P1 default and the maximum patch | `crates/enth-ui/tests/editor_snapshots.rs` |
| (ED2) Through a real `Session`: dropping a source on a knob makes one route at 25 % and one undo step, and the target's status reading says so; a voice source on a global target and a non-modulatable parameter are refused with the reason on the status line; the drop check against the patch's own rules; the depth handle sets the newest route's depth as one undo step with depth events only (the epoch does not move); "Remove … modulation"; the route list fits the P1 default at 1200 × 800 with all eight routes editable; the grid comes back where it fits | `crates/enth-ui/tests/modulation.rs` |
| (ED2) Repaint gating with exact delays through a real session and engine: 0 frames idle and a wake-up on new data; exactly 1/60 s while visible data changes (1/30 s until lane AM); a closed editor; a stalled engine; visibility of a scrolled-out widget | `crates/enth-ui/tests/repaint.rs` |
| (AM) Through a real `Session`, by accessibility label: every module kind added from the header and removed from its card's menu, each one op and one undo step, with Undo and Redo; a removal takes its routes and automation slot, says so on the status line, and undo puts them back in place; a right-click on empty region space opens that region's Add menu (and not over a card); the "+ Add" slot only where there is room, nothing scrolling, and adding there; Duplicate, Move up, Rename… and an empty rename, the amp envelope's refused Remove and Make amp envelope; Add after; a full section's items disabled; an add on an active session is one plan, its undo another; the minimum window keeps every region and every tile's handle | `crates/enth-ui/tests/add_module.rs` |
| (ED2) Live snapshots from synthetic telemetry: range and live arcs, the depth handle, tile levels, the output region as scope and spectrum | `crates/enth-ui/tests/live_snapshots.rs` (`region_live.png`, `region_live_spectrum.png`) |
| (ED2) The tap's quiet rule: an editor opened in silence sees the plan sent while it was closed; a global LFO keeps publishing; a macro or value moved in silence publishes until it settles | `crates/enth-engine/tests/tap.rs` |
| The plugin's glue: gestures, "More", the layout choice (`ENTH_LAYOUT`, the saved name, a refused file), a headless drag through the session | `crates/enth-plugin/src/editor.rs` |
| (PBU) The preset browser through a real `Session`: browse hides the body and Escape and × close; a click loads one undo step with no host writes and one `PatchEdited`, Undo and Redo take it back and again; a double-click loads and closes; the search with words, `#tag` and `by:`; tabs; favorites saved by the store; ◀ ▶ step and wrap and follow the search; the edited mark; the Save dialog's tags, its confirm before replacing and its refusals with hints; ▶ held loads and sounds through an active engine until release; audition without an engine says why; the crash notice never blocks and is offered once; the REAPER scenario's click points | `crates/enth-ui/tests/browser.rs` |
| (PBU) Snapshots: the header with an edited preset, the browse panel and the Save dialog at 1200 × 800 | `crates/enth-ui/tests/browser_snapshots.rs` (`browser_header.png`, `browser_panel.png`, `browser_save.png`) |
| (PBU) The plugin's browser over a folder store (a load, its undo step, no host writes, a save with a confirm, exactly `to_json()`), the crash folder offered once | `crates/enth-plugin/src/editor.rs` |
| (SK3M) The lesson pane and the Create panel together over a real `Session`, by label, at 1200 × 800: neither, Learn only, both and Create only, where each pane goes (the Create column under the header row, the lesson pane left of it, nothing else in the column) and the overflow report for Init and the P1 default; a file dropped beside the lesson pane goes to Create; the snapshot of both open | `crates/enth-ui/tests/learn_create.rs` (`learn_create_open.png`) |
| (PUI) The Create view over a real `Session`, by label, the test playing the shell: the flow from a drop (the intake's findings, K = 8 tiles through the rungs, select, ▶ candidate and start, the A/B against the upload, Use this as one undo step, the header's Undo), the flow from the folder browser (a short clip kept whole), cancel and a failure, the overflow report at 1200 × 800, no idle frames; snapshots | `crates/enth-ui/tests/create_view.rs` (`create_empty.png`, `create_browser.png`, `create_running.png`, `create_done.png`) |

## For the next lanes

**ED2 (modulation and feedback)** is done (above). Left for later lanes:

- The depth handle edits a knob's newest route only; older routes are edited in the overview (grid or list). A per-source focus (hover a tile, see that source's handle on every knob it drives) would make multi-route knobs direct; nothing in the data model stops it.
- Macros are sources the patch allows but the overview has no tiles for them, so they cannot be dragged yet (routes from macros show and edit like any other).
- Velocity and key are not in the tap, so their tiles show no level; a per-note value in the tap (an engine change) would add them.
- A value scrolled out of view still wakes the editor through the tap, at up to 30 Hz (the engine does not know what is visible).
- The route list fits eight routes at 1200 × 800; a ninth scrolls it. A layout that gives the overview more height gets the grid back automatically.

**After AM (adding and removing modules).** Left for later lanes:

- The header's right end is, right to left: the inline knobs, Redo, Undo, "+ Add". PBU's ◀ name ▶ / Browse / Save and RUI's challenge bar take the rest of the row (their hooks draw on the whole header `Ui`).
- A preset load is one `Entry::load` step (lane PBC); `describe::entry` names it "Load <name>" (lane PBU). Add and remove steps read "add osc3", "remove filter1" there (the ids), which a later polish can turn into titles.
- Reordering is the menu's Move up and down; a header drag (`CardOutput::header`, "Move <title>") still does nothing. Duplicate copies no routes; there is no kind replacement (report 03: deferred).
- The "+ Add" slot appears only where a region has room; in layout 1 the sources and the effects lane rarely do, so the header's "+ Add" and the cards' menus are the dependable ways in. A variant with spare room shows slots by itself.

**LV (a layout variant).** A variant is one JSON file and two snapshots:

1. `cargo run -p enth-ui --example dev_window --features dev-window -- --dump-layout lanes > my.json` (under Xvfb) gives a start; edit it and try it with `--layout my.json` and F5, or in REAPER with `ENTH_LAYOUT=/path/my.json`.
2. Save it as `crates/enth-ui/src/editor/layout/<name>.json` and add one line to `layout::BUILTIN` (`("<name>", include_str!("layout/<name>.json"))`); the unit test checks that every built-in parses, passes the rules and round-trips.
3. Add its two snapshots beside `tests/editor_snapshots.rs`'s (`editor_<name>_p1`, `editor_<name>_max`, through the same `snapshot` helper with `Editor::with_layout`), and run `tests/editor.rs`'s overflow check against it if it promises no scrolling.
4. Sizes are tokens or shares only. A region size of its own goes into `theme/sizes/editor.rs` as a token with a serde default (an additive crossing into ED1's file; say so in the report).
5. Regions and parts are fixed sets; a variant that needs a new kind of region (a compact "focus" card region) adds a `Region` variant and its drawing in `Editor::frame`. The `output` region (scope or spectrum, and meters) exists since ED2 and is optional: place it anywhere with a `size.output.width`-wide column or row, or leave it out.
