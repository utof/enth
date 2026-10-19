# ADR-0002: UI — native Rust, egui by default, no WebView

- Status: **Proposed** — confirm at Gate G0 with the UI spike ([benchmarks.md](../benchmarks.md))
- Date: 2026-10-18 (supersedes the 2026-10-17 WebView/Svelte proposal)

## Context

- Priorities: low CPU, GPU and RAM; many instances per project; near-zero cost when the editor is closed.
- Development and main use: Linux Mint (Cinnamon, X11), NVIDIA proprietary driver, REAPER. Must also work in other DAWs and on Windows and macOS.
- The main reason for a WebView was reusing the UI on a website. The website may never ship.
- Evidence ([research.md §4](../research.md#4-ui-webview-vs-native)): no public benchmark compares WebView and native plugin UIs. Anecdotal Windows reports put a WebView2 UI at ~100 MB shared plus ~35–80 MB per open instance. On Linux, wry embedding is X11-only and requires pumping GTK inside the host process; WebKitGTK has 2026 crash reports with NVIDIA's proprietary driver. The template we'd have used (WRAC) states its dropout measurements are unverified.
- egui's plugin adapter historically redraws continuously while the editor is open, so its idle-open cost is only low with repaint gating.

## Decision

Build the plugin UI natively in Rust. **egui** is the default; **vizia** is the challenger in the Phase 0 spike. WebView is not used in v1; it may be measured once in the spike purely for comparison.

Lesson content is Markdown in the repo, rendered natively now and reusable on a website later without a WebView in the plugin. A future website can compile the egui UI and the engine to wasm32.

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
