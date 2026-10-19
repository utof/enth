# ADR-0002: UI — native Rust, egui by default, no WebView

- Status: **Accepted (G0, 2026-10-19)** on the automated tier ([UI spike](../spikes/ui.md), [test host](../spikes/test-host.md), [G0 table](../benchmarks.md#ui-thresholds)). **The real-display checks are still pending with the owner** ([owner checklist](../spikes/owner-checklist.md), sections B and C); a failure there reopens this ADR as listed in the last section.
- Date: 2026-10-18 (supersedes the 2026-10-17 WebView/Svelte proposal); updated 2026-10-19 with the Phase 0 research (sub-report 02) and the UI spike; accepted at G0 on 2026-10-19

## Context

- Priorities: low CPU, GPU and RAM; many instances per project; near-zero cost when the editor is closed.
- Development and main use: Linux Mint (Cinnamon, X11), NVIDIA proprietary driver, REAPER. Must also work in other DAWs and on Windows and macOS.
- The main reason for a WebView was reusing the UI on a website. The website may never ship.
- Evidence ([research.md §4](../research.md#4-ui-webview-vs-native)): no public benchmark compares WebView and native plugin UIs. Anecdotal Windows reports put a WebView2 UI at ~100 MB shared plus ~35–80 MB per open instance. On Linux, wry embedding is X11-only and requires pumping GTK inside the host process; WebKitGTK has 2026 crash reports with NVIDIA's proprietary driver. The template we'd have used (WRAC) states its dropout measurements are unverified.
- egui's plugin adapter historically redraws continuously while the editor is open, so its idle-open cost is only low with repaint gating. *(Superseded 2026-10-19: repainting is reactive in nice-plug-egui 0.6.0; the risks moved to baseview and the NVIDIA X driver. See the update below.)*

## Decision

Build the plugin UI natively in Rust. **egui** is the default; **vizia** is the challenger in the Phase 0 spike. WebView is not used in v1; it may be measured once in the spike purely for comparison. *(Refined 2026-10-19, see the update below: egui on the glow renderer; no second toolkit in Phase 0.)*

Lesson content is Markdown in the repo, rendered natively now and reusable on a website later without a WebView in the plugin. A future website can compile the egui UI and the engine to wasm32.

## Update 2026-10-19: research and the UI spike

Sources: the Phase 0 research session (sub-report 02, `_local/sessions/2026-10-19-phase-0-spikes/02-native-ui-final.md`) and the first half of the UI spike ([docs/spikes/ui.md](../spikes/ui.md): `crates/enth-ui`, `spikes/spike-ui`, `spikes/spike-ui-plugin`). Host numbers are from REAPER 7.82 under Xvfb with Mesa's software GL; nothing ran on the real display.

**Refined decision**

- **egui on glow (OpenGL), not wgpu.** egui-baseview's wgpu renderer creates an instance, adapter and device per editor; under software rendering wgpu added 16–21 MB PSS and 5.6 MB to the stripped plugin and built slower (sub-report 02). The spike's plugin (`nice-plug-egui` default feature `opengl`) and dev window (eframe with `glow`) both use glow. Real-GPU open time and VRAM for wgpu are on the owner checklist, only to confirm.
- **No second toolkit in Phase 0.** vizia-plug is stale (last push 2026-06-13, still on nice-plug-core 0.1.4, port request unanswered), has no AccessKit inside plugins and needs Skia binaries; nice-plug-iced 0.5.0 spins one core while idle and leaks a thread per open/close (sub-report 02). All three sit on baseview, whose bugs dominate, so the useful "second arm" is a renderer fallback (GL vs CPU) for the NVIDIA risk, not another toolkit. Phase 0 Task 4's vizia arm is dropped.
- **Pins:** nice-plug 0.4.2, nice-plug-egui 0.6.0, egui-baseview 0.9.0, baseview 0.4.1, egui 0.37.0 (exact, `Cargo.lock` committed).
- **One design system:** every visual value comes from `enth-ui`'s theme tokens ([design-system.md](../design-system.md)), enforced by clippy (26 banned egui constructors outside `theme/`) and a source scan; a token gallery snapshot makes token changes reviewable.

**Measured repaint behaviour** (reference panel, release build)

| State | Result |
| --- | --- |
| Editor open, nothing sounding | **0 frames** in 22 s, 6 runs; GUI thread 0.05–0.19 % CPU; 64 wake-ups/s (egui-baseview's notifier poll); REAPER's CPU unchanged from editor-closed |
| Editor open, a chord playing | 28.6–29.6 fps for a 60 Hz request (baseview's cap; a 30 Hz request gives 19.5); `ui()` 0.55–0.67 ms p50 |
| CPU per frame, headless (layout + tessellation, no GL) | idle frame 0.9–1.1 ms, animating 1.2–1.4 ms mean, p99 ≤ 2.3 ms: 3.5–4.2 % of a core at 30 fps |
| Editor open time | `build()` → first frame in 44–67 ms (software GL) |
| Memory | +71 MB PSS on open, almost all Mesa's llvmpipe; not a GPU number |

Gating as built: the panel schedules the next frame only while something animated is visible and its data changed (`motion.scope_hz` for a fed scope, `motion.animated_hz` for live rings and tiles), and otherwise asks the audio thread, through a lock-free feed, to wake it on the next change (`RepaintNotifier`, an atomic store). A silent synth publishes nothing.

**baseview 0.4.1 issues every egui editor inherits** (sub-report 02, Xvfb unless noted)

- **Frame-rate cap:** next frame at MSC + 2, so ≈30 fps at 60 Hz, and a 30 fps request lands on ≈20 fps (also on the real GPU). Confirmed above.
- **Leak on open/close:** ≈70 KB PSS per cycle even for a GL-free editor, ≈115–160 KB with egui on Mesa; REAPER's own ReaEQ is flat. Fails "< 2 MB after 200 cycles" until fixed upstream.
- **HiDPI clipping in REAPER:** at `Xft.dpi: 192` REAPER sizes the window for scale 1 while baseview draws at 2, so the editor is clipped (baseview #359).
- **Blank first frame on NVIDIA:** the standalone window often never drew its first frame on the real display (nice-plug #95, reported fixed upstream, not on this machine).
- **Keyboard:** after a click in the editor, Space no longer reaches REAPER's transport (baseview never forwards unhandled keys); text entry works.
- **`NiceEguiApp::editor_closed()` is never called;** the spike rebuilds its state in `build()`.
- **The NVIDIA 595 X driver crashed Xorg** on this machine while GL windows were opened and closed (`nvidia_drv.so +0x3fc5a`, matching NVIDIA bug 6205955). Editor open/close churns exactly those resources, so real-display GUI testing is owner-attended and automated GUI tests run under Xvfb.

**Accepted at G0 (2026-10-19):** native Rust UI on egui 0.37 with the glow renderer, no WebView and no second toolkit; every visual value from `enth-ui` theme tokens; repaint gated on visible, changing data (0 frames while idle, measured). Open: baseview's open/close leak, ≈30 fps cap, HiDPI clipping in REAPER and the NVIDIA first-frame and X-driver issues, tracked upstream and in the owner checklist.

## Note — 2026-10-19: G0 numbers from the test host and the final UI runs

- **Leak, attributed** ([spikes/test-host.md](../spikes/test-host.md); Xvfb, Mesa llvmpipe, spike-shell's editor, 200 cycles × 2): PSS grows 76.9 KB per open/close cycle above a no-editor baseline of 0.2 KB. About 61 KB of it is one xkb keymap that baseview 0.4.1 never frees (`XkbcommonState::new` in `src/wrappers/xkbcommon.rs` lacks an `xkb_keymap_unref`; still so on baseview's master on 2026-10-19); with that line added in a scratch copy the growth is 16.1 KB per cycle. Threads stay constant (3 closed, 30 open). The "PSS growth" threshold fails until this is fixed upstream; a draft report waits for the owner (RustAudio's AI policy). We don't carry a patched baseview until we decide how we depend on it.
- **Open and first frame:** create → show median 53.1 ms (p95 69 ms) in the test host, 200 of 200 first frames drawn within 1 s, no "X11 thread disconnected" warnings in 400 cycles.
- **Scope rate:** a 60 Hz request (`motion.scope_hz`) gives 28.6–29.6 fps under baseview 0.4.1; a 30 Hz request gives 19.5 fps. The token stays at 60; whether 20 fps looks smooth enough is the owner's call on the real display.
- **Instructions per frame** (callgrind, the before/after number for UI changes): 6.69 M for an idle frame, 7.64 M for an animating one, at scale 1.
- **Snapshots** render only on Mesa's CPU Vulkan driver (lavapipe), so tests never load the NVIDIA driver.

## Note — 2026-10-19 (P1b research): layout as data, stable ids, keyboard focus

Sources: the P1b research session (`_local/sessions/2026-10-19-p1b-plugin-editor/`, reports 02 and 04, and the summary). Nothing ran on the real display; the interaction results are kittest's.

- **The editor stays in `enth-ui` (L7)**, as `theme`, `widgets`, `layout`, `view` and `editor`. A separate editor crate would be same-layer, which `check-layers` refuses. Widgets emit intents; the editor turns them into ops through the session at the end of the frame. A knob drag is one history gesture (one drag gave one undo entry, measured).
- **Layout is data.** A `LayoutSpec` is a tree of rows and columns sized by tokens or shares, plus a table that says which patch section fills which region. It loads from JSON like `ENTH_THEME` (`ENTH_LAYOUT`), so the owner can compare a designer's variant in REAPER without a rebuild. "Lanes" is layout 1, a variant is one file plus two snapshots, and a mode (Recreate, Learn) is a layout too. egui_taffy, egui_flex, egui_tiles and egui_dnd were not adopted: all four still required egui ^0.36 on 2026-10-19, and egui's built-in drag-and-drop is enough.
- **Stable ids keyed by (module id, parameter address).** Each card runs in a scope id made from its module id, and each control takes an explicit id (`Knob` gets one). Measured: 59 of 59 control ids survived a switch between two layouts, against 0 of 59 with today's automatic ids, and a knob drag continued across a mid-drag relayout (with automatic ids it was lost and its gesture stayed open). Gestures are also closed whenever the pointer is released, wherever the widget went.
- **Frame cost:** a generated editor (cards, knobs and segmented controls from `ParamSpec`s, 12 card displays, a scope) measured 4.55 M instructions for an idle frame and 4.90 M for an animating one, against the spike panel's 6.69 M and 7.64 M. The macro bar, matrix and spectrum are not in it yet, so expect 7–8 M for the real editor, about 4 % of a core at baseview's 30 fps. A gungraun case for one idle and one animating frame is proposed, with a hard limit of 10 M instructions per animating frame, re-based to the first P1b measurement plus 10 %.
- **Keyboard focus only while typing.** On X11, baseview takes focus on every click and never forwards keys, so Space never reached REAPER's transport. The policy is to hold keyboard focus only while a text field is edited and give it back afterwards (a patch in the vendored egui-baseview and baseview, [ADR-0001](0001-plugin-shell.md)). Shift-fine drags keep working, because modifiers arrive with mouse events. The cost is no keyboard shortcuts outside text fields on Linux, so P1b has Undo and Redo buttons in the header. The policy has to work both ways: Space typed into a text field must not start the transport. Open: whether it works inside REAPER's SWELL windows; owner build 1 decides.
- **The baseview decision is made.** The vendored baseview carries the xkb fix. The leak gate becomes at most 26 KB per open/close cycle (16.1 KB measured with the fix, plus 10 of margin), with threads constant; stock baseview would need 87 KB.
- **Interactions.** Drag-to-modulate uses egui's drag-and-drop (valid targets are highlighted, and a voice-rate source dropped on a global target is refused). Right-click "Type value…" takes units ("40ms", "A4"). Two fixes ride along: the parser rejects "−" (U+2212), which the design system displays, and sub-step drags on discrete knobs were discarded, so drags accumulate.
- **Repaint:** keep the spike's gating. "Visible" means open, shown and scrolled into view, because egui-baseview never reports occluded or minimised.
- **Room for Recreate and Learn:** a no-op `Overlay` (emphasis normal, dimmed or hidden, plus scores) and reserved lesson, challenge-bar and results regions behind cargo features. P1b builds no presenters for them.
- The reopen conditions below are unchanged.

## Consequences

- Smaller memory floor, no system-WebKit dependency on Linux, no second toolkit's event loop inside the host.
- Custom synth widgets (knobs, modulation rings, scopes, draggable modules) are straightforward in egui's painter.
- Rich text for lessons is more limited than HTML: headings, emphasis, images and lists are enough for v1.
- We must implement and test repaint gating, HiDPI and keyboard-focus behaviour ourselves.

## Alternatives considered

- **WebView (WRAC / wxp / wry)** — best for rich text and web reuse; worst RAM floor; Linux and NVIDIA risk; fallback only if both native arms fail the thresholds.
- **vizia** — retained mode, CSS-like styling, accessibility; smaller community. In the spike.
- **Slint** — software renderer is attractive; plugin integration immature in 2026.
- **iced** — plugin adapter is stale.
- **Raw wgpu** — best ceiling, most work; possible later for one heavy view (e.g. a spectrogram).

## How we'd know we're wrong

The chosen arm fails a [UI threshold](../benchmarks.md#ui-thresholds) with the real editor in S1 (Instrument 0.1), or the lesson views prove unusable for real lessons.

Added 2026-10-19: the real-GPU first-open test fails (the editor doesn't draw on first open in REAPER on the NVIDIA display), the NVIDIA X-driver crash recurs with plugin editors, or the animating editor costs more than 5 % of a core on the GPU at baseview's ≈30 fps (the CPU side alone is 3.5–4.2 %).

Added at G0 (2026-10-19), from the owner checklist: the open editor costs 30 MB of PSS or more on the real GPU; wgpu beats glow clearly on open time, fps and VRAM (switch the renderer, not the toolkit); HiDPI in REAPER or keyboard focus under Cinnamon is broken in a way we can't fix or work around in a day; or the 50-cycle open/close run on the real display crashes Xorg, which brings the CPU-rendered fallback decision ([benchmarks.md](../benchmarks.md#decision-rules-for-the-ui-toolkit-gate-g0), rule 3). The baseview leak alone does not reopen it: every Rust plugin toolkit sits on baseview.
