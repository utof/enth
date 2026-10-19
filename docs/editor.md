# The editor (`enth-ui::editor`, L7)

**The editor is one value, [`Editor`], that draws a layout's regions from the session every frame and turns what the controls ask for into session calls at the end of the frame.** It lives in `crates/enth-ui/src/editor/`, depends on the session only through `SessionView` and `SessionEdit` ([session.md](session.md)), and runs unchanged in the plugin window, in the dev window and in kittest. Lane ED1 built it (P1b, [phase-1b.md](phase-1b.md)) on UI-C's cards and widgets; ED2 adds drag-to-modulate, telemetry rings, scopes and repaint gating; LV adds layout variants. Visual rules: [design-system.md](design-system.md). Decisions: [ADR-0002](adr/0002-ui-toolkit.md) and its 2026-10-19 note (layout as data, stable ids, keyboard focus).

## Files

| File | Holds |
| --- | --- |
| `editor/frame.rs` | `Editor`: the frame (regions → read models → components), `apply` (intents → session calls), the editor's view state, `FrameReport` |
| `editor/layout.rs`, `editor/layout/<name>.json` | `LayoutSpec`, `Layout`, `Region`, `Part`, `Size`; the built-in layouts (`BUILTIN`; layout 1 is `lanes.json`) |
| `editor/regions/` | the region components: `Header`, `Macros`, `CardRegion`, `Overview` (tiles beside the matrix), `SourceTiles`, `Matrix`, `Status` |
| `editor/cards/` | UI-C's catalogue cards; ED1 added `menu.rs` (right-click menu, "Type value…"), the shared `control` (also used by the header) and the overlay fields on `ParamView` |
| `editor/overlay.rs` | the no-op `Overlay` (emphasis normal, dimmed or hidden; scores) |
| `editor/describe.rs` | undo steps in the editor's words, for the Undo and Redo tooltips |
| `editor/reserved.rs` | the lesson, challenge-bar and results regions behind the `learn` and `recreate` features |
| `theme/sizes/editor.rs` | the editor's size tokens (below) |
| `examples/dev_window.rs` | the dev window (feature `dev-window`) |
| `examples/frame_bench.rs` | instructions per editor frame, headless |
| `crates/enth-plugin/src/editor.rs` | the plugin window: `EditorApp` and `Glue`, the plugin's handle on `Editor` |

## The frame

The shell owns the protocol; the editor is the middle of it ([plugin.md](plugin.md#the-editor), [session.md](session.md#who-calls-what-on-which-thread)):

1. read the host's values (plugin only);
2. lock the session: `pump`, `fold_host`;
3. **`Editor::frame`**: cut the window into the layout's regions, build each region's read model from `SessionView::patch` (nothing is cached across frames), draw the components, then apply the frame's intents through `SessionEdit` (`Editor::apply`);
4. `drain_requests` into a local buffer; **unlock**;
5. forward `ShellRequest::Host` writes to nice-plug's `ParamSetter` (begin, set, end).

`Editor::frame` never pumps, folds or drains. The plugin draws it on egui's root `Ui` (the whole window; the editor paints its own background). An editor preset load is `Session::load_patch` under the lock, never `GuiContext::set_state` (`enth-plugin/clippy.toml` bans it); the dev window's `--patch` takes that path.

### Intents

Components never touch the session. They push `Intent`s; `apply` runs them in order:

| Intent | Session call |
| --- | --- |
| `Begin(grip)` … `Set`/`SetMacro`/`SetDepth` … `End(grip)` | `begin_gesture`, edits inside the gesture (one undo step, one host gesture per slotted parameter or macro), `end_gesture` |
| `Set` outside a gesture (double-click reset, a typed value, "Reset to default") | one `edit`, one undo step |
| `Select`, `Toggle`, `Enable`, `Unroute`, `Automate`, `StopAutomating` | one `edit` each, no gesture |
| `Undo`, `Redo` | `undo`, `redo` (each ends an open gesture first) |
| `ShowMore` | editor view state only |

A gesture still open when no pointer button is held is closed (the control that opened it vanished); every `build()` and `editor_closed` close it too. An edit the session refuses changes nothing; its hint shows at the right end of the status line.

### What the controls do

- **Every parameter control** (knob, stepped knob, segmented control, switch; the header's inline knobs too) has the same right-click menu:
  - **Type value…** floats a text field over the control, filled with the value as shown and selected. Enter parses units in either form ("1.2kHz", "1.20 kHz", "A4", "−12 st" with the U+2212 minus the editor displays) and makes one edit. A value that doesn't parse keeps the field open with the parser's hint under it ("✗ write a number in hz, e.g. …"). Escape, or a click elsewhere, cancels. The field is named "<Parameter> value" for accessibility; one is open at a time, in egui memory under `ids::entry()`.
  - **Reset to default (value)**.
  - **Automate** exposes the parameter as the lowest free host slot (`bridge::free_slot`; a slot that already drives it is reused, F1b decision 4). Then the menu reads "Automated by slot N" and offers **Stop automating**. Structural parameters (they recompile the plan) are not automatable, and the disabled item says why.
- **The status line** reads the hovered or dragged control: "Filter 1 › Cutoff 1.50 kHz Env 2 +40 %, Vel +10 %", plus "automation slot N" when a slot drives it. Its whole text is its accessible name ("Status: …").
- **Header**: wordmark, patch name, Undo and Redo with the step's name in the tooltip ("Undo Filter 1 › Cutoff"), and the built-ins a layout places there as inline knobs (layout 1: master level). Keyboard undo waits for a captured-shortcut fork patch (ADR-0002), so the buttons are the way to undo on Linux.
- **Envelope sculpting** (UI-C): the attack peak, the decay and sustain corner (one gesture over two addresses) and the release end are handles on the knobs' own power scales, so curve and knobs stay in sync both ways.
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
    {"size": {"token": "size.bar.header"}, "region": "header"},
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
| Regions | `header`, `macros`, `sources`, `voice_fx`, `modulators`, `effects` (the four card regions), `overview` (source tiles beside the matrix), `status`; reserved: `lesson` (feature `learn`), `challenge_bar` and `results` (feature `recreate`), which draw an empty panel in P1b |
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
| 7. The generator column scrolls | Three generator strips fit; no module region scrolls at the P1 default |

Hypotheses 4 (focal point), 5 (the effects lane), 6 (scope and spectrum) and 8 (the macro bar's weight) are left to the Claude Design variants (lane LV).

**Measured at 1200 × 800, the P1 default patch** (two oscillators, noise, a filter, two envelopes, two LFOs, a compressor and a reverb: the G1 reference patch), `tests/editor.rs::nothing_scrolls_at_1200_by_800_for_the_p1_default`:

- No module region overflows: sources 304 px for three 96 px strips, the filter lane 304 px for a 251 px card, the effects lane 642 px for voice (159), compressor (223) and reverb (223), the modulator band 224 px for envelope cards of 223 and LFO cards of 207.
- The overview gets 98 px. The source tiles fit in one row (85 px). **The matrix needs 161 px for the patch's 7 routed targets and scrolls by 63 px** (3 rows show). The reason: the header, macro bar and status bands (40 + 56 + 22 px and their gaps) plus the signal row (304) and the modulator band (224) leave 98 px of the 800. Taking the macro bar into the header row would give 48 px more; the matrix overview is lane ED2's to grow.
- At the maximum patch (4 generators, 4 voice effects, 4 effects, 8 modulators) every card region scrolls on its own (sources by 104 px, the filter lane by 723, the effects lane by 440, the band by 860 sideways).

Snapshots: `crates/enth-ui/tests/snapshots/editor_lanes_p1.png` and `editor_lanes_max.png` (`tests/editor_snapshots.rs`, lavapipe, kittest's default threshold).

## Room for Recreate and Learn

- **`Overlay`** (`editor/overlay.rs`): per parameter an emphasis (normal, dimmed, hidden) and a score; per module an emphasis. The editor applies it to the read model before drawing (`overlay::apply`): a hidden parameter is not drawn, a hidden module's card is dropped, a dimmed one draws in the dimmed ink and stays editable, a score draws as the knob's ring with its glyph. P1b ships `NoOverlay` (`Editor::set_overlay` takes another). Recreate dims and scores; only a lesson hides.
- **Reserved regions** behind the `learn` and `recreate` features of `enth-ui`: `lesson`, `challenge_bar`, `results`. A mode is a layout that places them; P1b draws an empty panel with the region's name.

## Tokens (`theme/sizes/editor.rs`)

| Token | Value | For |
| --- | --- | --- |
| `size.editor.band_height` | 224 | a band of lane cards (layout 1's modulators): one envelope card, the tallest P1 modulator |
| `size.editor.header_knob` | 112 | an inline knob in the header (dial, label, "−12.0 dB") |
| `size.editor.entry_width` | 120 | the "Type value…" field; also the menu's minimum width |
| `size.editor.entry_hint_width` | 240 | where the field's error hint wraps |

## The dev window

`examples/dev_window.rs`, feature `dev-window` (eframe on glow, X11): the real editor over a real `Session` activated at 48 kHz, the engine rendering 256-frame blocks at real-time pace on its own thread into buffers nobody hears, so plans swap and the return ring drains as in a host. **It opens a window: Xvfb, or the owner's display with work saved; never an agent on the real display** (AGENTS.md).

```sh
xvfb-run -a -s "-screen 0 1920x1080x24" \
  cargo run -p enth-ui --example dev_window --features dev-window -- --patch content/templates/pad_2osc.patch.json
```

Options: `--patch FILE`, `--layout NAME|FILE` (or `ENTH_LAYOUT`), `--theme FILE` (or `ENTH_THEME`), `--frames N` (quit after N frames and print the frame report: a smoke test), `--dump-layout NAME` (a variant's starting point), `--dump-theme`. Keys: F2, F5.

## Frame cost

`examples/frame_bench.rs`: the editor's egui pass and tessellation, headless, release build, callgrind, (I(250 frames) − I(50 frames)) / 200 (research 02's method):

| Case | Instructions per frame |
| --- | --- |
| P1 default, idle (pointer moving over the header, nothing changes) | 6.55 M |
| P1 default, a knob drag (an edit through the session every frame) | 6.81 M |
| Maximum patch, idle | 8.74 M |

Research 02 measured 4.55 M (idle) and 4.90 M (animating) for a generated editor without the macro bar, matrix or overview, and expected 7–8 M for the real one; the spike panel was 6.69 M and 7.64 M. The top costs are epaint's tessellator (stroke and fill, 12 %), `memcpy` (12 %) and glyph hinting (4 %). The proposed gungraun case keeps a hard limit of 10 M per animating frame.

## Tests

| What | Where |
| --- | --- |
| Interaction through a real `Session`, by accessibility label (no renderer): a knob drag is one undo step and the header's Undo and Redo take it back and forth; "Automate" assigns slot 1 and a drag is then one host gesture; "Stop automating"; "Type value…" with units, a note name and the U+2212 minus; a bad value keeps the field with its hint and Escape cancels; "Reset to default"; a macro drag is one gesture and one host gesture; the master knob in the header and voice settings in the effects lane; a disabled module stays editable; envelope handles and knobs stay in sync; the status line's text; ids across two layouts; a drag across a layout switch; no scrolling at 1200 × 800 for the P1 default; the maximum patch scrolls inside its regions; a refused layout keeps the current one | `crates/enth-ui/tests/editor.rs` |
| Layout geometry; the reserved regions with and without their features | `crates/enth-ui/tests/layout.rs` |
| Every built-in parses, checks and round-trips; refusals name the problem; the no-op overlay changes nothing and a lesson overlay dims, hides and scores; undo step names | unit tests in `layout.rs`, `overlay.rs`, `describe.rs` |
| Whole-editor snapshots, layout 1 at the P1 default and the maximum patch | `crates/enth-ui/tests/editor_snapshots.rs` |
| The plugin's glue: gestures, "More", the layout choice (`ENTH_LAYOUT`, the saved name, a refused file), a headless drag through the session | `crates/enth-plugin/src/editor.rs` |

## For the next lanes

**ED2 (modulation and feedback).**

- Drag-to-modulate starts from the overview's tiles: `Overview::show` returns each source's ⊕ handle response (`OverviewOutput::handles`); set the drag payload from `drag_started` and push a new intent on drop (a route is `Intent::SetDepth` on a missing route today, which inserts it; add a dedicated intent if the default depth should differ). Drop targets are the parameter controls, all drawn by `cards::control` under `ids::param(addr)`, so the outline and the "valid target" test go there once for cards and the header alike.
- Telemetry: `ParamView::live` already feeds the knob's modulation arc and `SourceView::phase` the tile's marker; build a `TelemetryReader` per `build()` (plugin.md) and fill them in `Editor::cards` (one place for every part). The plugin's `process()` already turns the tap's wake flag into repaints.
- The overview is yours to grow: 98 px at 1200 × 800 in layout 1 (matrix 63 px short at 7 routed targets). A scope or a spectrum is a new region (`Region` gains a variant; layouts place it) or a part of the overview.
- Repaint gating: `Editor::frame` requests nothing today; egui repaints on input only. `FrameReport` is the place to say "an animated thing was visible".
- `Editor::apply` is the one place intents become session calls; keep new intents' ops in `Intent::ops` so tests and the editor share them.

**LV (a layout variant).** A variant is one JSON file and two snapshots:

1. `cargo run -p enth-ui --example dev_window --features dev-window -- --dump-layout lanes > my.json` (under Xvfb) gives a start; edit it and try it with `--layout my.json` and F5, or in REAPER with `ENTH_LAYOUT=/path/my.json`.
2. Save it as `crates/enth-ui/src/editor/layout/<name>.json` and add one line to `layout::BUILTIN` (`("<name>", include_str!("layout/<name>.json"))`); the unit test checks that every built-in parses, passes the rules and round-trips.
3. Add its two snapshots beside `tests/editor_snapshots.rs`'s (`editor_<name>_p1`, `editor_<name>_max`, through the same `snapshot` helper with `Editor::with_layout`), and run `tests/editor.rs`'s overflow check against it if it promises no scrolling.
4. Sizes are tokens or shares only. A region size of its own goes into `theme/sizes/editor.rs` as a token with a serde default (an additive crossing into ED1's file; say so in the report).
5. Regions and parts are fixed sets; a variant that needs a new kind of region (a compact "focus" card region, a scope) adds a `Region` variant and its drawing in `Editor::frame`.
