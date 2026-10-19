# Spike: design system and reference panel (Phase 0, Task 4, first half)

- Date: 2026-10-19
- Code: `crates/enth-ui` (the design system), `spikes/spike-ui` (the reference panel, dev window, benchmark), `spikes/spike-ui-plugin` (the panel inside a copy of the shell plugin); branch `lane-u1/ui-panel`
- Decision it feeds: [ADR-0002](../adr/0002-ui-toolkit.md), [design-system.md](../design-system.md)
- Verdict: **egui on glow holds up.** The full reference panel costs 1.2–1.4 ms of CPU per animating frame before the GPU (3.5–4.2 % of a core at 30 fps) and draws **nothing while idle**: 0 frames in 22 s with the editor open in REAPER. The design system works as specified: tokens in one module, widgets that can't use raw values (clippy plus a source scan), and a token gallery snapshot. What can't be measured here is how it looks and costs on the real NVIDIA display; that is the owner checklist below.

Every host measurement here ran **under Xvfb with Mesa's software GL (llvmpipe)**, never on the real display. Software GL rasterises on the CPU, so GUI-thread CPU while drawing is far higher than on a GPU; frame counts, idle behaviour, `ui()` time and memory deltas transfer, absolute drawing cost doesn't. Headless numbers are CPU only (egui pass plus tessellation, no GL at all).

## What was built

### `crates/enth-ui` (layer 7)

- **`theme/`**, three tiers as in the design system:
  - `primitives.rs` (private): the palette (`GRAY_950` … `RED_400`), the 4 px space scale `SPACE_0..6`, two radii, two strokes, three text sizes, two repaint rates.
  - `tokens.rs` (public): semantic tokens (`color.surface.card`, `color.accent.modulation`, `space.gap`, `text.value`, …) and component tokens (`size.knob.diameter`, `size.card.generator_height`, `size.bar.macros`, …). Every field says what it is for.
  - `mod.rs`: `Theme::dark()` builds the tokens from the primitives; `Theme::apply(&ctx)` installs the fonts and sets egui's `Style` and `Visuals` from the same tokens, for both of egui's dark and light slots, so egui's own buttons, fields and scroll bars match. `Theme` derives serde; colours read as `#rrggbb[aa]`.
- **Fonts** (`assets/fonts/`, OFL-1.1, licence alongside, entry in `THIRD_PARTY_NOTICES.md`): Inter 4.1 Regular and SemiBold, unmodified, plus **Inter Tabular**, a 10 KB subset of Inter Regular with `tnum` frozen into the cmap. egui 0.37 shapes text with harfrust but passes no OpenType features, so `tnum` can't be switched on at run time; the subset holds only digits and `. , : + - −` and space, and egui falls back to Inter Regular for units. `freeze_tabular.py` rebuilds it byte for byte. Three files, 841 KB, no egui default fonts. `cargo deny` doesn't see font files (they are not crates), so it is unaffected; it passes.
- **`widgets/`**: `Knob` (track, value arc, modulation arc on an inner ring, score ring outside with ✓ ~ ✗, dimmed emphasis, inline layout for the macro bar; drag, Shift for fine, wheel, double-click to reset), `ModuleCard` (category stripe, enable toggle, title and module type, display slot, parameter grid; a fixed-height *strip* and a *stacked* lane card; the header is a drag handle), `NumberField`, `Segmented`, `Display` (wave, curve with a phase marker, scope with min/max decimation, log-frequency spectrum), `SourceTile` (live curve, route-count badge, ⊕ handle), `MatrixGrid` (target × source, bars from the centre, drag to edit), `MacroBar`, `StatusLine`, and placeholder `LessonPane` and `ChallengeBar`. Every interactive part has an accessible name, which kittest's accessibility check enforces on the gallery.
- **Enforcement**: `crates/enth-ui/clippy.toml` repeats the workspace rules and bans 26 egui constructors (`Color32::from_*`, `gamma_multiply*`, `Rgba::*`, `Hsva::new`, `FontId::new/proportional/monospace`, `RichText::size`, `Stroke::new`, `CornerRadius::same`, `Margin::same/symmetric`). Each path was checked to resolve: a probe file calling all 26 got 26 errors, plus one more for the same call through the `egui::epaint` re-export. Only `theme/` opts out, per file, with a reason. `tests/no_raw_values.rs` scans the source for what clippy can't see: `Color32::WHITE`-style constants, hand-built `Shadow`s, `add_space(<literal>)`, and any float literal in widget code other than 0, ½, 1, 2 and 100 (percent). All workspace I/O, thread, clock and lock rules still apply to the UI crate; it needed no opt-out (time comes from egui's input, fonts are compiled in).
- **Tests**: the token gallery snapshot; contrast (labels ≥ 4.5:1 and values ≥ 7:1 on every surface, graphics ≥ 3:1), scale rules, glyph coverage of every glyph we draw, equal digit widths in `text.value`, the serde round trip; knob, segmented, card and matrix input through kittest.

### `spikes/spike-ui`

- **The reference panel** (`panel.rs`) from [benchmarks.md](../benchmarks.md), as a literal slice of direction A "Lanes" in Learn mode at 1100 × 768: header (mode switch, preset), macro bar, lesson pane (about 1,500 words, headings, bold words, one figure texture, two lists) with the 16-row matrix under it, the generator column (8 strips with displays, scrolls), one effect lane with 3 stacked cards that reorder by dragging their headers, the modulator strip (8 live source tiles, a 2048-sample scope, a 512-bin spectrum from a Hann-windowed FFT) and the status line (hovered parameter, value, modulation, repaint state). **64 slot knobs plus the 8 macro knobs cover all 72 host parameters**; 8 knobs carry an LFO ring that animates. Everything visual comes from the theme; `spikes/spike-ui/clippy.toml` bans the same egui constructors.
- **Audio feed** (`feed.rs`): a single-producer ring of the newest output samples, LFO 1's phase and an "is anything sounding" flag, atomics only (no lock, no allocation on the producer side). A silent block after a silent block publishes nothing.
- **Repaint gating**: each frame decides what it needs. A visible scope with fresh samples asks for the next frame after `1 / motion.scope_hz`; a running LFO with a visible ring or tile asks after `1 / motion.animated_hz`; otherwise the panel tells the feed it is asleep and schedules nothing. The producer's next change wakes it once (`RepaintNotifier::request_repaint`, an atomic store, in the plugin; `ctx.request_repaint` in the dev window). The panel re-checks the feed after going to sleep, so a block that slips in between is never lost. A minimised or occluded viewport schedules nothing and asks for no wake-ups. Tests check the requested delays (exactly 1/60 s, then 1/30 s, then none) and that an idle panel schedules nothing.
- **Host parameters** go through a small `HostParams` trait with begin/set/end gestures (wheel and double-click changes become a one-shot gesture); `LocalParams` stands in for tests and the dev window.
- **`spike-ui-dev`** (`cargo run -p spike-ui --release`): the standalone dev window on eframe 0.37 with glow, X11, a fake audio thread (a chord at 48 kHz in 256-sample blocks) switched by "Audio on/off" in the header, `--gallery`, `--theme <file.json>` and `--dump-theme`. It opens a real GL window, so it was compiled and linted but **never launched** here.
- **`examples/frame_bench.rs`**: CPU per frame of the whole panel, headless.

### `spikes/spike-ui-plugin`

A copy of `spike-shell` (same 8-voice synth, the same 72 parameter ids, the JSON blob) with the panel as its editor; slot 1 still drives the cutoff and macro 1 the gain. New permanent ids: CLAP `dev.enth.spike-ui`, VST3 `EnthSpikeUiPnl01` ([naming.md](../naming.md)). While the editor is open, `process()` pushes every output sample into the feed and publishes LFO 1 (0.4 Hz) at the end of the block; `assert_process_allocs` stays on and clap-validator's process tests pass with it. `build()` rebuilds the panel on every open, because nice-plug-egui 0.6.0 never calls `editor_closed()`, and renames the GUI thread `enth-gui`. `ENTH_UI_LOG=<file>` logs every frame and a per-second summary with the thread's CPU time; `ENTH_THEME=<file.json>` replaces the theme's tokens when the editor opens (the serde form, as `spike-ui-dev --dump-theme` prints it), so token values such as repaint rates can be compared in a host without a rebuild. spike-ui's dev window sits behind a default feature that the plugin turns off, so eframe stays out of the plugin build.

## Screenshots

Both are rendered on the CPU (Mesa lavapipe through wgpu's Vulkan backend) at scale 1.0 and committed:

- Token gallery: [`crates/enth-ui/tests/snapshots/token_gallery.png`](../../crates/enth-ui/tests/snapshots/token_gallery.png) (1100 × 1320)
- Reference panel: [`spikes/spike-ui/tests/snapshots/reference_panel.png`](../../spikes/spike-ui/tests/snapshots/reference_panel.png) (1100 × 768, LFO running, scope fed)
- Disabled card and hovered envelope handle: [`spikes/spike-ui/tests/snapshots/disabled_card_and_hovered_handle.png`](../../spikes/spike-ui/tests/snapshots/disabled_card_and_hovered_handle.png) (1100 × 768, added later; see the section after the owner's first look)

A 2× render (2200 × 1536, not committed) was checked by eye: text and arcs are crisp.

## Measurements

The machine was shared with other agents; timings ran under an exclusive lock with the load average recorded (2–7). Raw logs: `~/code/enth-scratch/lane-u1/runs/`.

### CPU per frame, headless (egui pass + tessellation, release, no GL)

`cargo run -p spike-ui --release --example frame_bench 1000`, 8 runs of 600–1000 frames per case, load 4–6. The 5 runs on the final code first read 0.1–0.2 ms higher for animating frames than the 3 earlier ones; three interleaved old/new pairs under one lock (load 3) then gave 1.14–1.28 ms (code before the last layout changes) against 1.19–1.31 ms (final), so that cluster was the machine, and the ranges below cover all runs.

| Case | Scale | p50 | p99 | max | mean | At 30 fps | At 60 fps |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Idle frame (nothing changes, e.g. a mouse move) | 1.0 | 0.88–1.04 ms | 1.12–1.58 ms | 1.28–1.89 ms | 0.89–1.06 ms | 2.7–3.2 % | 5.3–6.4 % |
| Animating (new audio block, new LFO phase) | 1.0 | 1.15–1.33 ms | 1.36–2.30 ms | 1.77–2.53 ms | 1.16–1.41 ms | 3.5–4.2 % | 7.0–8.5 % |
| Idle frame | 2.0 | 0.89–1.01 ms | 0.96–1.80 ms | 1.09–3.44 ms | 0.92–1.07 ms | 2.7–3.2 % | 5.5–6.4 % |
| Animating | 2.0 | 1.03–1.17 ms | 1.42–1.99 ms | 1.69–4.90 ms | 1.06–1.25 ms | 3.2–3.8 % | 6.3–7.5 % |

**Instructions per frame** (callgrind, steady state: the difference between 250 and 50 frames of one case, divided by 200), the number to compare before and after a change (AGENTS.md):

| Case | Scale 1.0 | Scale 2.0 |
| --- | --- | --- |
| Idle frame | 6.69 M | 6.64 M |
| Animating | 7.64 M | 7.58 M |

`valgrind --tool=callgrind target/release/examples/frame_bench <frames> <idle1|animating1|idle2|animating2>` reproduces them.

A truly idle panel runs no frames, so its cost is zero; "idle frame" is what an input event costs. The researcher's stand-in panel measured 0.73–0.88 ms p50; this one is richer (lesson text, 72 knobs with labels and values, 11 displays, 96 matrix cells) at 0.9–1.3 ms.

A callgrind profile of the steady state (330 frames per case) puts the time in: the lesson pane's text, about a quarter (all 1,500 words are laid out and hashed every frame, visible or not); the 72 knobs, about a quarter; the displays (module figures resampled every frame, scope, spectrum), about a sixth; one-line text layout and galley-cache lookups for labels and values, about a tenth.

### In REAPER 7.82 (Xvfb, Mesa llvmpipe, Dummy Audio 48 kHz/128, release bundle)

A Lua script loads the plugin, waits 5 s, opens the editor, waits 25 s, plays a held four-note chord for 20 s, stops for 15 s and closes the editor. The plugin's frame log and a 1 s `/proc` sampler record each phase, skipping the first 3 s after each switch. Six runs (r2–r6, and the 60 Hz half of the paired run below, on the final code); ranges over all six:

| State | Frames | Frame rate | `ui()` p50 | GUI threads CPU | GUI wake-ups | REAPER CPU | PSS |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Loaded, editor never opened | — | — | — | — | — | 7.9–9.9 % | 138–141 MB |
| Open, idle (22 s) | **0** | 0 | — | 0.05–0.19 % | 64/s | 7.7–9.9 % | 210–212 MB |
| Open, playing (17 s) | 487–503 | 28.6–29.6 fps | 0.55–0.67 ms | 24–31 % (software GL) | 367–385/s | 196–200 % (llvmpipe threads) | 210–212 MB |
| Open, stopped (12 s) | **0** | 0 | — | 0.09–0.18 % | 64/s | 7.6–9.7 % | 211–213 MB |
| Closed again | — | — | — | — | — | 6.9–9.9 % | 188–191 MB |

- **Idle is free:** no frame at all while the editor is open and nothing sounds; the GUI thread's only activity is egui-baseview's 64 Hz notifier poll. Opening the editor doesn't raise REAPER's CPU beyond noise.
- **Animating hits baseview's cap:** the panel asks for 60 Hz (a fed scope) and gets 28.6–29.6 fps, the ≈30 fps ceiling of baseview 0.4.1's X11 pacing (next frame at MSC + 2) that the researcher found. Every playing frame is a "scope" frame, because fresh audio arrives every block; the 30 Hz "animated" state only occurs when the LFO runs without new audio.
- **Asking for 30 Hz gives 20 fps.** benchmarks.md and phase-0.md now say scopes request `1/30 s`; the design system's `motion.scope_hz` is 60. One paired run with `ENTH_THEME` set to a theme file with `scope_hz: 30` (no rebuild): **19.5 fps**, GUI thread 23.3 %, REAPER 166 %; the next run with the default 60 Hz: **29.6 fps**, 27.9 %, 200 %. A 33 ms request lands on every third vblank. So the token stays at 60 for now: it is what gives ≤ 30 fps under baseview 0.4.1, and changing it is one token edit once baseview's pacing changes.
- **GUI-thread CPU while playing is software rendering:** `ui()` takes 0.55–0.67 ms of the 8–10 ms the thread spends per frame; the rest is llvmpipe drawing 1100 × 768 on the CPU, plus its 12 worker threads (the 196–200 % process figure). On a GPU the frame should cost about the headless 1.2–1.4 ms plus the driver's submission.
- **Memory:** +71 MB PSS when the editor opens, almost all Mesa's llvmpipe (the researcher measured the same jump with a much smaller panel); closing leaves about 50 MB mapped, because Mesa stays loaded. Threads go 27 → 54 → 27. The open/close leak test is lane U2's.
- **Open time:** `build()` → first frame drawn in 44–67 ms. The first `ui()` alone takes 32–53 ms (rasterising the glyphs, laying out the lesson, uploading the figure); later ones about 0.5 ms.

### The snapshot loop (edit → test → PNG)

`touch` a widget file, then run one snapshot test (dev profile, under an exclusive lock, load 3.6–4.3, 3 runs):

| Step | Time |
| --- | --- |
| Edit → reference panel PNG (`cargo test -p spike-ui --test panel reference_panel_snapshot`) | 2.2–2.3 s |
| Edit → token gallery PNG (`cargo test -p enth-ui --test token_gallery`) | 1.9 s |
| Re-run without an edit | 1.2 s |
| The test itself (lavapipe device, layout, 1100 × 768 render, compare) | about 1.0 s |

The rebuild peaks at 1.1 GB RSS (rustc and the linker); the test process at 150 MB.

### Builds and bundles

| Item | Value |
| --- | --- |
| `spike-ui-plugin.clap`, release | 9,682,928 B on the final code (spike-shell: 9,114,152 B; this one has Inter instead of egui's default fonts, the panel and the theme parser for `ENTH_THEME`) |
| `spike-ui-plugin.clap`, debug | 55,723,088 B with main's line-tables-only dev profile (198,445,472 B before the merge) |
| clap-validator 0.4.1, debug and release | 36 passed, 0 failed, 0 warnings, 8 skipped each (the same 8 skips as spike-shell) |
| Release bundle, incremental | 1 min 13 s |
| Release build of the frame benchmark (with eframe and wgpu dev-deps) | 3 min 48 s |
| `cargo clippy --workspace --all-targets --locked -- -D warnings`, warm | 18 s |
| `cargo test --workspace --locked`, warm | 1 min 3 s |

## Results table (benchmarks.md template, egui column)

Only the cells this lane measured; lane U2 measures the REAPER protocol and the test host, the owner the real display.

| Metric | egui (this spike) | Threshold | Notes |
| --- | --- | --- | --- |
| GUI CPU, closed | 0 (no GUI thread) | ≈ 0 % | REAPER 7.9–8.9 % with the editor never opened; that is REAPER itself |
| GUI CPU, open idle | 0.05–0.19 % (64 wake-ups/s, 0 frames) | < 1 % | Xvfb |
| GUI CPU, open animating | 24–31 % software GL; 3.5–4.2 % expected CPU side at 30 fps (headless 1.2–1.4 ms/frame) | < 5 % | Xvfb numbers don't transfer; owner checklist 2 |
| GUI CPU, occluded | not measured | ≤ idle | Xvfb has no compositor (researcher: still draws); the panel stops when egui reports minimised or occluded |
| xruns (10 min) | not measured | 0 | Dummy Audio; U2 and owner |
| PSS per open editor | +71 MB (Mesa llvmpipe) | < 30 MB | Software GL inflates this; owner checklist 2 |
| PSS growth, 200 cycles | not measured | < 2 MB | U2 (researcher: baseview leaks ≈ 70 KB/cycle) |
| Frame time p99 | headless 1.4–2.3 ms; `ui()` p50 0.55–0.67 ms in REAPER | < 16.7 ms | |
| Editor open (warm) | 44–67 ms build → first frame | 150 ms | Xvfb |
| GPU SM % / VRAM | not measured | 5 % / 100 MB | Owner |
| Achieved fps vs requested | 28.6–29.6 for 60 requested; 19.5 for 30 requested | report | baseview cap |
| Checklist (REAPER / 2nd host) | not run | all pass | U2 and owner |

## Design-system findings and deviations

- **Tabular digits need a derived font.** egui 0.37 applies no OpenType features, and Inter's default figures are proportional. The 10 KB `tnum`-frozen subset solves it without a second family; a test checks that `text.value` digits have equal advances (and that the regular face's don't, so the test means something).
- **The knob cell is 56 px, not 48.** Values in real units at 12 px with tabular digits ("−12.5 dB", "1.20 kHz") are about 52 px wide. 56 fits six knobs beside a 128 px display in a generator strip and three per row in a 220 px lane.
- **Token additions** (all in design-system.md now): `text.body` and `text.strong` (running text and bold words; still three sizes), `accent.value_wash` and `accent.modulation_wash` (the area under curves, selection; flat translucent fills, not gradients), and component tokens the widgets needed (`control_height`, knob `cell_width`, `drag_travel`, `fine_ratio`, `sweep_degrees`, card `display_width`, `lane_display_height`, `toggle`, tile, matrix and display sizes), plus `motion.transition` for egui's own animations. No proposed value changed.
- **Translucent colours drift by a step in hex.** egui stores colours premultiplied, so a 40-alpha wash written as `#6cb4fc28` reads back as `#6cb3ff28`; the colour value round-trips exactly, only its digits differ.
- **egui 0.37 API notes:** `Id::new` is deprecated (use an `IdSalt` and `Ui::make_persistent_id`); `WidgetType` became `Role` (accesskit's); `Context::run` became `run_ui`; `set_fonts` applies at once if no text was laid out yet this pass; `FontData` supports variable-font coordinates, so one variable Inter file could replace the two static weights later.
- **kittest's accessibility check is on by default** and caught an unlabelled `DragValue`; all widgets now carry names, which also makes tests readable (`get_by_label("Cutoff")`, `get_by_label("Move Delay")`).
- **Snapshots render on lavapipe only.** The test harness accepts only a CPU Vulkan adapter, and `.cargo/config.toml` points the Vulkan loader at the lavapipe ICD, so tests never load the NVIDIA driver; CI installs `mesa-vulkan-drivers`. The goldens were made with Mesa 25.2.8; a different Mesa may need a small `max_failed_pixels` in a `kittest.toml`.

### Deviations from the brief

- **64 knobs bound to host parameters** are read as 64 slot knobs plus the 8 macro knobs, so all 72 parameters are on screen. Not all of them are visible at once: with the lesson pane open, the generator column and the lane scroll (direction A expects that). egui runs the code of scrolled-out widgets anyway, so their CPU is in the numbers; only their painting is clipped.
- **The scope and the LFO are fed by the synth, not a fake ring, in the plugin**; the fake producer is the dev window's.
- **Frame CPU under Xvfb** is reported as the GUI thread's total; the headless benchmark gives the renderer-independent part. Neither is a GPU number.
- **The dev window was never launched,** not even under Xvfb (the brief reserves it for the owner); the REAPER runs exercise the same panel code with glow.

## What I'd still improve

1. **Lay out only what is visible:** the lesson pane through `ScrollArea::show_viewport` with cached block heights, and skip knob and card code outside the scroll viewport. The profile says this is the biggest single saving, maybe a third of a frame.
2. **Cache static displays:** module figures (envelope, filter response, wave tables) are resampled every frame; cache their shapes keyed on the parameter values.
3. **Format values only when they change:** 72 value strings per frame are formatted and hashed for the galley cache; keep them per knob.
4. **Tune the scope rate on the real GPU:** under baseview 0.4.1 a 60 Hz request gives ≈30 fps and a 30 Hz request ≈20 fps (measured above). Whether 20 fps scopes look smooth enough to save a third of the drawing is an owner call on the real display; `ENTH_THEME` makes it a file edit, no rebuild.
5. **Lane drag feedback:** cards swap places as the pointer crosses a neighbour's middle; a lifted card that follows the pointer, with a drop gap, would read better.
6. **Matrix rows at 16 px** are tight for 12 px values; 20 px rows would need the lesson pane to shrink or the matrix to scroll.
7. **Knob value text lags one frame** behind a drag (the text is formatted before the widget handles input); pass a formatter instead.
8. **A theme file watcher** in the dev window (the `--theme` file is read once at start), for live token tweaking.

## After the owner's first look (2026-10-19, branch `ui/u3-disable-sculpt`)

The owner ran the dev window, kept the look, and asked for two interaction changes (`docs/handoff/claude-design-layout.md`, with annotated screenshots):

- **A disabled module dims as a whole.** Switching a card off used to grey its title only. Now `ModuleCard` lays out everything but its fill and toggle in one child `Ui` and calls `multiply_opacity(state.disabled_opacity)` (0.4, a new token) on it when the module is off: stripe, title, module type, display, every knob with its arcs and value text, segmented controls. One mechanism; no widget knows about it. **The controls stay editable** (a module can be set up before it is switched on), and the toggle stays at full strength since it is how the card comes back. egui fades shapes one by one, so where two overlap (a knob's pointer over its modulation arc) the overlap reads slightly brighter than either. Measured on lavapipe, a disabled card keeps 0.38–0.40 of its contrast against the card colour (title 0.40, display 0.38, knobs 0.39); `tests/disabled_card.rs` checks 0.4 ± 0.05.
- **Envelopes are sculpted on the curve.** `enth-ui` gained `CurveHandles` (`widgets/handles.rs`): draggable points on a curve display, each bound to one or two normalised values with a drag scale, a default and a formatter. `interact` runs first and changes the values, then `Display::handles` draws the curve from the new values with the handles over it, so curve, handles and knobs never lag one another. Handles show while the pointer is over the display (hollow circles, `size.handle.radius`), the hovered or dragged one fills with the accent and an accent wash (`size.handle.reach`, also the grab distance); Shift drags at `knob.fine_ratio`, double-click resets, and a tooltip above the handle reads the values in real units. The status line shows the same ("Amp env › Decay, Sustain 57.1 ms, 70 %"). The panel wires both envelope cards (Amp env, Filter env): the attack peak drags the attack sideways, the decay corner drags the decay sideways and the sustain up and down, the release corner drags the release sideways. Host gestures balance as for knobs (a handle that drags two values opens and closes both).
  - **Pixels to values:** the spike's curve (`shapes::adsr`) gives each stage `0.05 + 0.25 × value` of the width, linear in the knob's normalised value, so a drag across a quarter of the display sweeps the knob once and the point stays under the pointer. The knob's normalised value is the parameter's own (power) scale; real units come from the same `Unit::format` the knobs use. `enth-ui` doesn't use `enth-params`, so a real envelope gives its handles the `to_norm` slope of its curve the same way.
  - **The release handle sits where the release starts**, the corner the owner's screenshot points at, rather than at the end of the release: the spike's curve always ends at the right edge, so an end point could not follow the pointer. Dragging that corner left lengthens the release.
  - egui only starts a drag after the pointer has moved a few pixels; the first frame of a handle drag counts that stretch (`total_drag_delta`), so the point doesn't trail the pointer. (Knobs still drop it; nobody can see that on a knob.)
- **Snapshots:** the token gallery's disabled Filter card now has two knobs and shows the dimming; [`spikes/spike-ui/tests/snapshots/disabled_card_and_hovered_handle.png`](../../spikes/spike-ui/tests/snapshots/disabled_card_and_hovered_handle.png) shows the panel with Osc 2 switched off and the Amp env release handle hovered. The reference panel snapshot is unchanged (every card enabled, no pointer).
- **Tests:** kittest drags the sustain handle (sustain moves by the drag over the trace height, decay stays), the attack handle (attack moves, the peak stays under the pointer, the Attack knob's accessible value matches, gestures balance), a generic handle (both axes, Shift, double-click reset), a knob in a disabled card (still editable), and a pixel test that switching a card off fades its title, display and knobs to the token's opacity while the fill stays.
- **Follow-ups:** LFO curves are not sculptable yet: the panel's LFOs are source tiles with fixed shapes and no shape parameters, so there is nothing for a handle to drag (`CurveHandles` would take them as is once an LFO card has, say, a skew or a phase). Lane cards have no envelope, so `lane()` doesn't wire handles. Other candidates: the filter response (cutoff sideways, resonance up and down), a lifted handle that shows the stage's duration on a time axis once the engine's real envelope drives the display, and group opacity (an offscreen layer) if the overlap brightening ever shows.

## Not done, and why

- **Anything on the real display:** the editor in REAPER on the NVIDIA GPU, the dev window, GPU and VRAM numbers, covered and minimised windows, HiDPI in a host, keyboard focus under a window manager. Rule: no window or GL context on the real display. Owner checklist below.
- **The REAPER benchmark protocol** (8 instances, 64-sample buffer, xruns, 200 open/close cycles, a second host) is lane U2's; the plugin and its frame log are ready for it.
- **Real audio:** Dummy Audio only.

## How to reproduce

```
cargo test -p enth-ui -p spike-ui                                   # snapshots, gating, widgets
UPDATE_SNAPSHOTS=1 cargo test -p enth-ui --test token_gallery       # after a deliberate token change
cargo run -p spike-ui --release --example frame_bench 1000          # headless CPU per frame
cargo xtask bundle spike-ui-plugin [--release]                      # target/bundled/spike-ui-plugin.{clap,vst3}
clap-validator validate target/bundled/spike-ui-plugin.clap
cargo run -p spike-ui --release [-- --gallery]                      # dev window: real display, owner only
```

The REAPER runs used a scratch copy of the portable REAPER 7.82, a Lua script in the style of `xtask/reaper/` (using its `common.lua`), `xvfb-run -a -s "-screen 0 1920x1080x24"` with `__GLX_VENDOR_LIBRARY_NAME=mesa` and `LIBGL_ALWAYS_SOFTWARE=1`, `ENTH_UI_LOG` and a `/proc` sampler; the scripts are in `~/code/enth-scratch/lane-u1/{reaper,scripts}/`.

## Owner checklist (real display)

Save your work first: the NVIDIA X driver crashed on 2026-10-19 while GL windows were opened and closed (ADR-0002). Leave at least 10 s between opening windows.

1. **Dev window look and feel:** `cargo run -p spike-ui --release`. Is the first frame drawn (the researcher saw blank standalone windows on this GPU)? Do the type, contrast and spacing read well at 100 %? Drag knobs (and with Shift), use the wheel, double-click to reset, drag a lane card by its header, scroll the generator column and the lesson. Then `-- --gallery`.
2. **Animating cost on the GPU:** in the dev window click "Audio on" and run `pidstat -t -p $(pgrep spike-ui-dev) 1 20` and `nvidia-smi pmon -s um -c 20`; then "Audio off" and confirm the CPU drops to about zero (the panel stops drawing).
3. **In REAPER:** `cargo xtask install spike-ui-plugin --release`, rescan, insert "enth spike-ui" (CLAP). Does it draw on first open? Play notes: do the rings, tiles and scopes move smoothly at about 30 fps, and stop when the notes end? Hover knobs: does the status line follow?
4. **HiDPI:** at 200 % (`Xft.dpi: 192`), is the editor sharp, and does REAPER clip it (the researcher saw clipping)?
5. **Decide:** is the visual direction right (palette, density, knob size 36 in a 56 px cell, Inter)? Anything to change becomes a token edit plus a new gallery snapshot.
