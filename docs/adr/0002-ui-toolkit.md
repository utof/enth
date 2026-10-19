# ADR-0002: UI — native Rust, egui by default, no WebView

- Status: **Proposed — to be Accepted at G0** (proposed wording below). The first half of the Task 4 UI spike passed ([results](../spikes/ui.md)); the owner confirms at Gate G0 after checking the editor on the real display, and lane U2 adds the REAPER protocol numbers.
- Date: 2026-10-18 (supersedes the 2026-10-17 WebView/Svelte proposal); updated 2026-10-19 with the Phase 0 research (sub-report 02) and the UI spike

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
| Editor open, nothing sounding | **0 frames** in 22 s, 5 runs; GUI thread 0.05–0.19 % CPU; 64 wake-ups/s (egui-baseview's notifier poll); REAPER's CPU unchanged from editor-closed |
| Editor open, a chord playing | 28.6–29.4 fps for a 60 Hz request (baseview's cap); `ui()` 0.55–0.67 ms p50 |
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

**Proposed wording for G0:** *"Accepted at G0 (2026-10-__): native Rust UI on egui 0.37 with the glow renderer, no WebView and no second toolkit; every visual value from `enth-ui` theme tokens; repaint gated on visible, changing data (0 frames while idle, measured). Open: baseview's open/close leak, ≈30 fps cap, HiDPI clipping in REAPER and the NVIDIA first-frame and X-driver issues, tracked upstream and in the owner checklist."*

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
