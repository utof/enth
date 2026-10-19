# 02 — Native UI in plugins

- Date: 2026-10-19
- Sub-researcher: 02 (native UI in plugins)
- Questions answered: 1–7 from the brief. Q1 and Q4 are only partly answered: no GPU or in-host measurement was possible (the NVIDIA driver needs a reboot, and there's no native Linux host). The session was cut short for that reboot, so see Gaps.
- Markers: **[measured]** = run on the owner's laptop today; **[documented]** = code, changelog, licence file or official docs; **[anecdotal]** = issue or forum report. All versions and dates were looked up today on crates.io, Codeberg, GitHub and docs pages.

## Answer in brief

- **The egui stack changed completely in the last two months and is now reactive by default.** nice-plug-egui stopped repainting every frame in 0.4.0 (2026-08-17). baseview 0.4.0 (2026-10-05) made redraws lazy and added X11 frame pacing via the Present extension, EGL-first contexts, damage tracking and timers. egui-baseview 0.9.0 and nice-plug-egui 0.6.0 (2026-10-17/18) are built on it. ADR-0002's "redraws continuously" premise is out of date. [documented]
- **The new stack is days old and breaks often.** egui-baseview has had 14 releases since 2026-06-04, and nice-plug-egui 12 since 2026-05-17. A repaint bug in the new lazy model (RepaintNotifier did nothing until the mouse moved) was fixed the day before this report (egui-baseview #37, 2026-10-18). Pin exact versions. [documented]
- **vizia is a weak challenger today.** Upstream vizia-plug still targets nice-plug-core 0.1.4. Two unmerged forks port it to 0.4.2. vizia itself sits on baseview 0.2.2, two breaking versions behind 0.4.1, and its last commit was 2026-08-21. It renders through Skia (C++). Its AccessKit support works only with winit, not in plugins. It has no headless test mode (PR #687 is still open). [documented]
- **iced is now a better challenger than vizia.** nice-plug-iced 0.5.0 lives in the nice-plug repo, on baseview 0.4.1 and iced 0.14 (updated 2026-10-18). iced 0.14 has reactive rendering, headless and end-to-end testing, a markdown widget (useful for the lesson pane) and a tiny-skia CPU renderer. ADR-0002's "iced adapter is stale" is no longer true. [documented]
- **Slint is ruled out by its licence, not only by maturity.** It's GPL-3.0-only, or a royalty-free licence that requires attribution and forbids embedded use, or commercial. None of these is on the project's permissive list. [documented]
- **Use glow, not wgpu, for egui in the plugin.** egui-baseview's wgpu path creates a new wgpu Instance, Adapter and Device for every editor it opens [documented]. One user's eframe measurements on Windows: glow ≈31 MB RAM / 31 MB VRAM, wgpu-Vulkan ≈90 MB / 265 MB, and startup of 100–300 ms (glow) versus 1–7 s (wgpu) [anecdotal].
- **The owner's laptop runs PRIME in "nvidia" mode, and OpenGL is broken right now.** Every X client, plugin editors included, renders on the NVIDIA GPU, not the Intel UHD 630. GLX context creation currently fails with X_GLXCreateNewContext BadValue. Any GL plugin UI would fail to open until the reboot. [measured]
- **The agent feedback loop works today with egui_kittest, with no GPU.** Snapshot tests render through lavapipe. After an edit to the UI crate, the rebuild takes 3.7 s and the snapshot test 6–8 s, and the PNG it writes is readable by an agent. vizia has no equivalent. [measured]
- **egui's CPU cost per frame on its own is about 1.0 ms for a stand-in panel (release build).** That is about 6% of one core at 60 fps, before any GL work, so it already misses the 5% "open, animating" threshold. Expect to run scopes at 30 Hz or to revisit that threshold. [measured, under heavy load from other jobs]
- **Several parts of the benchmark protocol won't work as written on this machine.** `perf` is blocked (perf_event_paranoid=4). `nvidia-smi dmon` reports the whole device, not the plugin. Thread-based "GUI CPU" breaks when baseview moves X11 events onto the host's main thread (open PR #357). Neither arm can pass the Orca accessibility row, and the X11 compositor never reports occlusion. [measured / documented]

## Findings

### Q1 — egui inside plugins

**Versions and maintainers (looked up 2026-10-19)** [documented]

| Crate | Version (date) | Repo / maintainer | Notes |
| --- | --- | --- | --- |
| nice-plug | 0.4.2 (2026-09-14) | [codeberg RustAudio/nice-plug](https://codeberg.org/RustAudio/nice-plug), Billy Messenger (BillyDM) | 79 stars, 12 open issues, last commit 2026-10-18 |
| nice-plug-egui | 0.6.0 (2026-10-18) | same repo | egui 0.37.0, egui-baseview 0.9.0; features `opengl` (default), `wgpu`, `keyboard_focus_workaround` |
| egui-baseview | 0.9.0 (2026-10-17) | [codeberg RustAudio/egui-baseview](https://codeberg.org/RustAudio/egui-baseview), BillyDM. This is the successor of the archived GitHub repo | MIT OR Apache-2.0; 4 open issues |
| baseview | 0.4.1 (2026-10-09; first on crates.io 2026-05-03) | [github RustAudio/baseview](https://github.com/RustAudio/baseview), mostly Adrien Prokopowicz (prokopyl, also the clack author) | 390 stars, 23 open issues and PRs |
| egui / egui_glow / egui-wgpu / egui_kittest | 0.37.0 (2026-10-15) | [emilk/egui](https://github.com/emilk/egui) | eframe has defaulted to wgpu since 0.34 ([#7761](https://github.com/emilk/egui/issues/7761)) |

**How repainting works now** [documented, from code read on 2026-10-19]

- In nice-plug 0.3.0 (2026-08-17), nice-plug-egui stopped repainting every frame and added `RepaintNotifier` ([CHANGELOG](https://codeberg.org/RustAudio/nice-plug/src/branch/main/CHANGELOG.md)). research.md §4 cites the old nih-plug source ("for now, just always redraw"), which no longer describes nice-plug.
- baseview 0.4 made redrawing lazy ([#340](https://github.com/RustAudio/baseview/pull/340), merged 2026-09-30). It added partial rendering with damage notification ([#344](https://github.com/RustAudio/baseview/pull/344)), timers ([#350](https://github.com/RustAudio/baseview/pull/350)), X11 frame pacing through the Present extension with a 15 ms fallback timer ([#323](https://github.com/RustAudio/baseview/pull/323)), and EGL-first GL contexts on X11 with a GLX fallback ([#325](https://github.com/RustAudio/baseview/pull/325)).
- egui-baseview 0.9 ([window.rs](https://codeberg.org/RustAudio/egui-baseview/src/branch/main/src/window.rs)) triggers redraws from four sources:
  - egui's own `request_repaint` and `request_repaint_after`, through `waker.request_redraw_after(delay)`;
  - any mouse or keyboard event (every cursor move redraws);
  - a `RepaintNotifier` (`Arc<AtomicBool>`, real-time safe) that a baseview timer polls every **1/64 s by default** (`with_repaint_poll_interval`);
  - host parameter or modulation changes and `state_changed`, which nice-plug-egui forwards to the notifier ([editor.rs](https://codeberg.org/RustAudio/nice-plug/src/branch/main/crates/nice-plug-egui/src/editor.rs)).
- The poll timer means an idle open editor still wakes 64 times a second. Each wake is cheap (one atomic swap), but it hurts battery life and won't show up as "<1% CPU".
- The fix for the lazy-redraw bug ("request_repaint() has no effect until something else triggers a frame", [egui-baseview #37](https://codeberg.org/RustAudio/egui-baseview/issues/37)) landed on 2026-10-18 via PR #38. The stack is fresh.
- **Pattern for enth:** parameter changes arrive through the notifier automatically. A scope calls `ctx.request_repaint_after(1/30 or 1/60 s)` only while it's visible and has new ring data. Tune the poll interval, and count wake-ups as well as CPU.

**Measured idle CPU and GPU with the editor open:** none found in public sources for nice-plug-egui 0.6 / baseview 0.4. Targeted searches turned up only egui-app reports, such as [egui #5092](https://github.com/emilk/egui/issues/5092) and [#7096](https://github.com/emilk/egui/issues/7096). This is a gap, closed only by the Task 4 measurement.

**egui CPU cost per frame, no GPU** [measured 2026-10-19]

- Setup: a stand-in panel with 64 `DragValue` "knobs", 16 matrix rows, a 2048-point painted scope and 20 lesson paragraphs, at 1100×768. egui 0.37, release build. Each frame runs `run_ui` plus `tessellate`, over 600 frames, with the 1-minute load average between 10 and 22 from other jobs.
- Result: p50 ≈ **1.0 ms**, p99 ≈ 1.3–1.4 ms, max 1.7 ms. Idle and animating frames cost about the same, because immediate mode re-lays out everything on each repaint. Scale 2.0 cost the same as 1.0.
- At 60 fps that is about **6% of one core**, before GL upload, draw calls and driver time.
- The stand-in is simpler than the reference panel (no image, no custom knobs), so treat this as a lower bound on CPU per frame.
- Code: `/tmp/enth-research/02/egui-frame-bench` (uses the panel in `/tmp/enth-research/02/kittest-spike/src/lib.rs`).

**Open issues that matter** [documented]

- [egui-baseview #20](https://codeberg.org/RustAudio/egui-baseview/issues/20) (open since 2024): keyboard focus. The `keyboard_focus_workaround` feature calls `focus()` whenever egui emits platform output.
- [egui-baseview #6](https://codeberg.org/RustAudio/egui-baseview/issues/6) (open): closing the window doesn't drop the egui loop. Check it with the leak test.
- [baseview #359](https://github.com/RustAudio/baseview/issues/359) (2026-10-15): the host's scale isn't honoured on X11, because Xft.dpi wins over CLAP `set_scale`.
- [baseview #342](https://github.com/RustAudio/baseview/issues/342) and [PR #357](https://github.com/RustAudio/baseview/pull/357) (open, updated 2026-10-18): drop the per-window X11 thread and run on the host's main thread. This changes threading, and with it how GUI CPU must be measured.
- [baseview #348](https://github.com/RustAudio/baseview/issues/348): proper drag-and-drop and clipboard. egui-baseview uses `copypasta` for the clipboard for now.
- Fonts: egui 0.37 made `default_fonts` opt-in. nice-plug-egui depends on egui with `default-features = false`, so enth must enable `egui/default_fonts` or ship its own fonts. The OFL/UFL entries in deny.toml exist only because of these fonts ([egui CHANGELOG](https://github.com/emilk/egui/blob/main/CHANGELOG.md), [#8640](https://github.com/emilk/egui/issues/8640)).

### Q2 — vizia and vizia-plug

**State** [documented, 2026-10-19]

- **vizia 0.4.0** was released 2026-04-23 ([crates.io](https://crates.io/crates/vizia)). The repo has 2,339 stars and 65 open issues, and its last push was 2026-08-21. Nearly all recent commits are by one maintainer (geom3trik).
  - Rendering goes through **skia-safe 0.99** (Skia GL). The Skia GPU cache is capped at 128 MB by default ([#769](https://github.com/vizia/vizia/pull/769)).
  - The workspace pins `baseview = 0.2.2`, two breaking versions behind 0.4.1 ([Cargo.toml](https://github.com/vizia/vizia/blob/main/Cargo.toml)).
- **vizia-plug** ([github](https://github.com/vizia/vizia-plug), MIT, 31 stars, last push 2026-06-13) isn't on crates.io.
  - It depends on `nice-plug-core = "0.1.4"` and pins vizia to git rev `bbf98e6`, which used vizia's own baseview fork.
  - [Issue #41](https://github.com/vizia/vizia-plug/issues/41) (2026-10-01, "Update to work with nice-plug 0.4.2") has no maintainer reply.
  - Two forks have ported it and neither is merged: [Atreus-Technologies `nice-plug-0.4` branch](https://github.com/Atreus-Technologies/vizia-plug/tree/nice-plug-0.4) (2026-10-07) and [ebai101/vizia-plug](https://github.com/ebai101/vizia-plug) (2026-09-22).
  - Upstream, a Task 4 vizia arm would mean porting the adapter before measuring anything.
- Open vizia-plug issues relevant to us: #9 "Automation breaks interaction", #4 "Opening text input on sliders doesn't work properly", #6 "Resizing window", #24 (macOS scale factor). Fixed in May 2026: editor reopen crash (#31), two instances interfering (#30).

**Repainting** [documented]

- `vizia_baseview` runs `on_frame` at the host compositor's vsync rate, as a code comment in its window.rs says. That is the old baseview frame callback.
- Each frame it processes styles, animations and timers. It redraws (Skia flush plus swap, `vsync: true`) only when the redraw list isn't empty.
- So an idle editor costs a 60 Hz wake-up with some per-frame bookkeeping and no GPU draw. Until vizia moves to baseview 0.4 it misses lazy redraw and Present pacing.

**AccessKit** [documented]

- In vizia's Cargo.toml the feature is `accesskit = ["vizia_winit?/accesskit"]`. `vizia_baseview` contains no AccessKit code.
- baseview has no AccessKit support either ([#200](https://github.com/RustAudio/baseview/issues/200), open since 2024).
- **vizia's accessibility advantage therefore doesn't apply inside a plugin.** The same is true for egui: its AccessKit tree exists, and kittest uses it, but egui-baseview doesn't expose it to Orca.

### Q3 — HiDPI, keyboard focus and text input in REAPER and Bitwig on X11; NVIDIA

**The owner's display and GPU** [measured 2026-10-19]

- Panel eDP-1-1 at 1920×1080, 60 Hz. `Xft.dpi: 96`, and the Cinnamon scaling factor is 0 (automatic), so the real scale is 1.0. **Scale 2.0 has to be simulated**, for example with `xrdb -merge` setting `Xft.dpi: 192`, or in Xephyr.
- `prime-select query` returns `nvidia`. `xrandr --listproviders` shows NVIDIA-0 as the source and modesetting (Intel) as the output sink. **All GL and Vulkan rendering by X clients, including every plugin editor, goes to the Quadro RTX 3000, and the dGPU stays powered.**
- Right now `glxinfo -B` fails with `BadValue … X_GLXCreateNewContext`, and EGL on the X11 platform fails to initialise. That's the driver/library mismatch (NVML 595.99).

**HiDPI** [documented]

- baseview on X11 takes its scale from Xft.dpi first. A host's `set_scale` only counts as a fallback, and REAPER never calls `set_scale` ([baseview #359](https://github.com/RustAudio/baseview/issues/359), 2026-10-15).
- egui-baseview passes baseview's scale factor to egui as `native_pixels_per_point` and has a separate zoom factor.
- Expect correct 2.0 rendering when Xft.dpi=192, and a mismatch when a host scales its own UI differently.
- Bitwig's `set_scale` behaviour on Linux wasn't verified.

**Keyboard focus and spacebar** [documented from code; consistent with anecdotal reports]

- When the editor isn't focused, egui-baseview calls `window.focus()` on every mouse press. On X11 that's `XSetInputFocus(POINTER_ROOT, plugin_window)`.
- Key events that egui doesn't want come back as `EventStatus::Ignored`, but baseview's X11 backend never forwards ignored keys to the parent window. Its event_loop.rs and xcb_window.rs have no forwarding code, and forwarding is still an open PR for Windows ([#260](https://github.com/RustAudio/baseview/pull/260)).
- Expected result: **after any click in the editor, the spacebar stops reaching REAPER or Bitwig** until the user clicks the host.
- The same symptom is reported for JUCE plugins on Linux in REAPER, Bitwig and Ardour ([JUCE forum, 2025-09-22 to 2025-10-11](https://forum.juce.com/t/bug-linux-plugin-not-forwarding-space-to-daw-when-plugin-has-focus/67197)) [anecdotal].
- Fix to try in the spike: take X focus only while a text field is active, and hand focus back to the parent when editing ends.

**Text input** [anecdotal]

- [nih-plug #219](https://github.com/robbert-vdh/nih-plug/issues/219) (open, 2025-04-26): REAPER and Ableton grab keys meant for custom-drawn text fields in egui, vizia and iced alike. REAPER's per-FX "Send all keyboard input to plug-in" option works around it but then blocks every shortcut. The platform isn't stated.
- nice-plug-egui's `on_virtual_key_from_host` is still a `// TODO` that returns false (that path is VST3-only anyway).
- vizia-plug implemented it in its PR #17.
- Design consequence: keep in-plugin text entry minimal (value fields, preset names, search) and test it early.

**Bitwig** [documented and anecdotal]

- Bitwig hosts plugins "Within Bitwig", "Together" (the default) or "Individually" (one sandbox process per instance) ([user guide](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)).
- A baseview plugin opened a black window in Bitwig 6 under XWayland because of a visibility-tree bug, since fixed ([baseview #331](https://github.com/RustAudio/baseview/issues/331), 2026-08-26).

**NVIDIA with OpenGL or wgpu** [anecdotal]

- [egui #7561](https://github.com/emilk/egui/issues/7561) (open, 2025-09-24): random freezes with hybrid graphics on Ubuntu X11, NVIDIA drivers up to 580. The stack trace is in `libGLX_nvidia` waiting in xcb. It's more likely with several instances, and goes away when hybrid graphics is turned off or `WGPU_ADAPTER_NAME=Intel` is set. This setup (Ubuntu base, X11, hybrid, many instances) is close to the owner's.
- [egui #8416](https://github.com/emilk/egui/issues/8416) (open, 2026-08-13): egui_wgpu freezes with "timeout … acquire the next frame" on NVIDIA 610 under Wayland.
- No baseview-, vizia- or iced-specific NVIDIA reports turned up. Older JUCE threads describe swap blocking and lag with OpenGL on NVIDIA under Linux and with multiple contexts ([JUCE forum](https://forum.juce.com/t/the-state-of-opengl-on-linux/50936)).
- baseview 0.4 now prefers EGL over GLX on X11. Whether NVIDIA 595's EGL-on-X11 works inside REAPER is unverified.

### Q4 — glow or wgpu for egui in a plugin

- **Per-instance cost** [documented, code]: egui-baseview's wgpu `Renderer::new` creates a new `wgpu::Instance`, a surface, and a `RenderState` (adapter plus device), blocking on `pollster`, for **every editor it opens**. Nothing is shared between instances ([renderer.rs](https://codeberg.org/RustAudio/egui-baseview/src/branch/main/src/renderer/wgpu/renderer.rs)). The glow path takes baseview's GL context and builds one `egui_glow::Painter`.
- **Memory and startup** [anecdotal: one user's measurements, Windows, eframe 0.34, [egui #7761](https://github.com/emilk/egui/issues/7761) updated 2026-04-10]:
  - RAM (Task Manager): glow ≈31 MB, wgpu-Vulkan ≈90 MB, DX12 ≈142 MB.
  - VRAM: glow ≈31 MB, wgpu-Vulkan ≈265 MB.
  - Release binary: 17.7 MB (glow) versus 27.7 MB (wgpu).
  - Clean release compile: 31 s versus 42 s.
  - Startup in a real app: glow "consistently pretty quick (100~300ms)", wgpu "1 to about 7 seconds".
  - Input latency with vsync: about 2 frames (glow) versus about 3 (wgpu).
- **Direction of egui** [documented]: emilk made wgpu the eframe default in 0.34 and wants to settle on one backend ([#5889](https://github.com/emilk/egui/issues/5889), closed). egui_glow is still released in lockstep (0.37.0), but glow might eventually lose support in eframe. The plugin depends only on `egui_glow`, not eframe.
- **Multiple instances:** with wgpu, ten open editors means ten Vulkan devices. With glow it means ten GL contexts and painters. Neither has been measured on Linux.
- **Conclusion:** glow (the nice-plug-egui default) for Phase 0. Measure wgpu once, for open time and VRAM only, as a note rather than a full arm. If the editor ever needs wgpu (for example a heavy spectrogram), give it one shared device per process.

### Q5 — alternatives as of October 2026

| Option | Status (date) | Licence | Plugin path | Verdict |
| --- | --- | --- | --- | --- |
| **iced** via nice-plug-iced | iced 0.14.0 (2025-12-07), repo active (last push 2026-10-17); nice-plug-iced 0.5.0 and iced_baseview 0.6.0 (2026-10-18) on baseview 0.4.1 | MIT | Maintained in the nice-plug repo. Features include `wgpu`, `tiny-skia` (CPU), `markdown`, `canvas`, `hot`, `time-travel`, `unconditional-rendering` (off by default) | **Best challenger.** 0.14 brought reactive rendering, headless and end-to-end testing (`iced_test`), a markdown widget with images, and hot reloading ([release](https://github.com/iced-rs/iced/releases/tag/0.14.0)). Downsides: the Elm-style API is more verbose for custom widgets, the AI training corpus is smaller than egui's, and no release in 10 months |
| **vizia** via vizia-plug | See Q2 | MIT | Needs an adapter port, and the port still runs on old baseview | Drop from Phase 0 unless a nice-plug-0.4 port merges upstream |
| **Slint** | 1.18.1 (2026-09-21) | GPL-3.0-only OR Royalty-free-2.0 (attribution required, no embedded use, no "exposing the APIs") OR commercial ([LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md)) | slint-baseview is a WIP fork: 4 commits, last 2026-06-15, baseview 0.1.x, Slint pinned to 1.16.1, "works on Windows", Linux untested ([zff.dev](https://zff.dev/aidan729/slint-baseview)) | **Excluded by licence** under the brief's permissive-only rule |
| **Xilem / Masonry** | 0.4.0 (2025-10-29); repo active (2026-09-14); "experimental" in its own README | Apache-2.0 | Only a personal [masonry_baseview](https://github.com/baltobor/masonry_baseview) (last push 2026-03-03) | Too early |
| **Raw vello / femtovg / softbuffer** | vello 0.11.0 (2026-10-02), vello_cpu 0.3.0, femtovg 0.27.0 (2026-08-31) | MIT/Apache | nice-plug ships `byo_gui_gl`, `byo_gui_softbuffer` and `byo_gui_wgpu` examples on baseview 0.4.1 | Fine for one heavy view later. Too much work for a whole editor |
| **truce** (another plugin framework, has `truce-egui` and `truce-slint`) | 6.3.0 (2026-07-18), 227 stars | "Truce License 1.0": MIT/Apache plus a commercial-framework rider ([LICENSE](https://github.com/truce-audio/truce/blob/main/LICENSE)) | Has its own GUI layers | Not on the allowed list as is. Mentioned for sub-researcher 01 |

**Is "egui default, vizia challenger" still the right pair?**

- egui as default: **yes.** It has the most active adapter (same maintainer as nice-plug), built-in repaint gating, the largest corpus for agents, and the only mature no-GPU snapshot loop.
- vizia as challenger: **no.** If there's a challenger, it should be **iced via nice-plug-iced**: same shell, same baseview, maintained the same week, CPU renderer option, markdown lesson pane, headless tests.

### Q6 — UI testing and AI feedback

**egui_kittest 0.37.0 (2026-10-15)** [documented, [README](https://github.com/emilk/egui/tree/main/crates/egui_kittest), [CHANGELOG](https://github.com/emilk/egui/blob/main/crates/egui_kittest/CHANGELOG.md)]

- Tests query and drive the UI through the AccessKit tree (`get_by_label`, `click`, drag-and-drop helpers).
- The `snapshot` and `wgpu` features render a PNG and compare it with dify. Thresholds live in `kittest.toml`, and `UPDATE_SNAPSHOTS=true` or `force` regenerates images.
- From 0.37, a test fails when an input widget has no accessible name, and missing glyphs panic.
- Also in 0.37: `KITTEST_RECORD=1` records an mp4 of a test (needs ffmpeg, which is installed).
- Built from egui's own CI and Rerun's, so it's mature for egui apps. Snapshot rendering needs a wgpu adapter. Pixel differences across GPUs are documented.

**Measured on the owner's laptop, 2026-10-19** (dev profile, `CARGO_BUILD_JOBS=2`, load average about 20, lavapipe forced with `VK_ICD_FILENAMES=…/lvp_icd.json`, so no GPU used):

| Step | Result |
| --- | --- |
| Clean build of the test (egui_kittest + wgpu 30 + naga) | 3 min 41 s, peak RSS 1.03 GB |
| Rebuild after editing the UI crate | **3.7 s** |
| Whole snapshot test run (two harnesses, 13 renders) | **5.9–7.9 s** wall, 263 MB RSS |
| Harness build + first `run()` | 320–440 ms |
| AccessKit query plus `run()` | 23–43 ms |
| First render at 1100×768 | 0.46–1.15 s |
| Each later render at 1100×768 | **≈230–280 ms** |
| New harness at scale 2.0 + render (2200×1536) | 1.36–1.60 s |
| Dependencies at opt-level 2 | Clean rebuild didn't finish within 10 min; killed |

- The written PNG ([tests/snapshots/panel_1100x768.png](/tmp/enth-research/02/kittest-spike/tests/snapshots/panel_1100x768.png)) showed the whole panel correctly, and the agent read it with an image viewer.
- An agent loop of "edit, rebuild, snapshot, look" takes about 10–12 s, with no GPU, window or host.
- CI note: GitHub's ubuntu runners have no GPU either. They need Mesa lavapipe (the `mesa-vulkan-drivers` package) to run the same tests.

**Without a GPU (CPU renderers)** [documented]

- [egui_software_backend](https://github.com/DGriffin91/egui_software_backend) 0.0.3 (2026-03-29) is a CPU renderer with optional kittest integration, MIT/Apache. It targets egui 0.34, so it's stale for 0.37. Its policy says "No LLM-generated content", which rules out contributing to it.
- egui's own software renderer is still an open wish ([#1129](https://github.com/emilk/egui/issues/1129)).
- lavapipe works, so neither is needed.

**vizia** [documented]: no test harness and no headless mode. [PR #687](https://github.com/vizia/vizia/pull/687) "headless app support" has been open since 2026-05-03. The only option is screenshots of a real window under Xvfb. `Xvfb`, `xvfb-run`, `Xephyr`, `xwd` and `ffmpeg` are installed; `xdotool` isn't.

**iced** [documented]: `iced_test` 0.14.0 offers a headless simulator and snapshots, and 0.14 added first-class end-to-end tests. Not measured here.

### Q7 — Does the benchmark protocol measure the right things? Tools on Linux

Local facts [measured 2026-10-19]:

- `perf_event_paranoid` is 4, and even `perf stat -- true` fails as a normal user.
- Intel RAPL `energy_uj` is root-only. Battery power is readable without root as `current_now × voltage_now` in `/sys/class/power_supply/BAT1/`.
- `pidstat`, `smem` and `heaptrack` aren't installed (Task 1 installs them).
- NVML currently fails because of the driver mismatch.

**What the protocol gets right:** closed, idle, animating, occluded and minimised states; PSS rather than RSS; 1/10/50 instances; leak cycles; xruns; open time; the host checklist.

**What to change or add:**

1. **Per-editor CPU.**
   - `pidstat -t` assumes the GUI has its own thread. That's true today, because baseview's X11 backend spawns one unnamed thread per window (`thread::spawn` in window_thread.rs), but [PR #357](https://github.com/RustAudio/baseview/pull/357) moves events onto the host's main thread.
   - Measure inside the plugin instead: add `CLOCK_THREAD_CPUTIME_ID` deltas around each editor's event, timer and draw callbacks and log them per instance. Also log process-level deltas from `/proc/<pid>/stat` (utime, stime), which work without perf.
2. **Wake-ups per second while idle**, per thread, from `voluntary_ctxt_switches` deltas in `/proc/<pid>/task/*/status`. egui-baseview polls every 1/64 s by default, and "<1% CPU" hides that.
3. **GPU.**
   - `nvidia-smi dmon` reports the whole device. Use `nvidia-smi pmon -s um` (per process) and NVML per-process memory. Get per-editor GPU time with in-plugin GL timer queries (`GL_TIME_ELAPSED`).
   - Run the GPU rows in both PRIME modes. `nvidia` (current) renders everything on the dGPU. `on-demand` puts plugins on the Intel iGPU, where `intel_gpu_top` and DRM fdinfo apply. The choice changes power use and which driver bugs apply.
4. **PSS.** Report first-editor-in-process separately from marginal. The first GL context loads the NVIDIA userspace driver, a large fixed cost. The 30 MB threshold should apply to the marginal editor.
5. **Occluded state.** Under Cinnamon's compositing X11, a covered window gets no occlusion events, so an *animating* editor keeps drawing. Gate only "hidden by host or minimised" and "closed". For "covered", record the number without a pass/fail.
6. **"Open, animating < 5%" at 60 fps.** egui's CPU-side cost alone measured about 6% here (Q1). State the scope refresh rate in the protocol (30 Hz is a reasonable tuning step), then set the threshold from real numbers.
7. **Accessibility (Orca).** Not reachable in v1 by any baseview-based arm. Replace the gate with kittest's accessible-name check plus a note.
8. **Scale 2.0.** Simulate it with `xrdb -merge` (`Xft.dpi: 192`). REAPER doesn't call `set_scale`.
9. **Tool permissions.** `perf` needs the owner to run `sudo sysctl kernel.perf_event_paranoid=1`. `powertop` needs sudo. Watts can be read without root from BAT1 while on battery.
10. **Per-instance isolation.** Bitwig's "Individually" mode gives one process per plugin instance, so plain per-process tools measure one instance. REAPER's dedicated-process bridge on Linux for CLAP is unverified.
11. **Agent-loop UI metrics** (the owner ranks iteration speed first): clean and incremental build time per arm; snapshot-test wall time; whether a headless screenshot exists (yes or no); release binary size. Tracy or puffin (`profiling` crate) for frame-time p99 inside the editor.

## Recommendation for Phase 0

1. **Pin** nice-plug 0.4.2, nice-plug-egui 0.6.0, egui-baseview 0.9.0, baseview 0.4.1 and egui 0.37.0 with `=` versions plus a committed Cargo.lock, and upgrade on purpose. Enable `egui/default_fonts` (or ship fonts).
2. **egui arm on glow** (the default). Build gating in from the start:
   - scopes call `request_repaint_after` only while visible and fed;
   - start with a 30 Hz scope rate and also measure 60 Hz;
   - tune `repaint_poll_interval`;
   - log CPU time and wake-ups per instance inside the plugin.
3. **Replace the vizia arm with an iced arm** (nice-plug-iced 0.5.0, time-boxed to about 1.5 days, wgpu and tiny-skia both tried once), or drop the second arm and spend the time on host integration. Either is better evidence per day than porting vizia-plug and moving vizia to baseview 0.4 first. Keep WebView out.
4. **Measure wgpu once** for egui (open time, VRAM, 10 editors), as a note rather than an arm.
5. **Set up the agent loop on day 1:**
   - `enth-ui` as a plain library;
   - egui_kittest snapshot tests on lavapipe, locally and in CI;
   - a standalone dev window (nice-plug standalone or eframe) for a human to look at;
   - the kittest accessible-name check left on.
6. **Focus policy:** take X keyboard focus only while a text field is being edited, and return it to the parent afterwards. Test spacebar and typing in REAPER and Bitwig first.
7. **Before Task 4:** reboot and confirm GL works on NVIDIA (`glxinfo -B`, `eglinfo -B`). Decide which PRIME mode is the measurement baseline (recommend measuring both). Get a native Linux REAPER: the installed REAPER is the Windows build under Wine and can't load a Linux `.clap`; that's sub-researcher 01's area. Get the Bitwig demo as the second host.

## Plan check

**ADR-0002 should change.**

- Context bullet "egui's plugin adapter historically redraws continuously": out of date. nice-plug-egui since 0.4.0 (2026-08-17) and baseview since 0.4.0 (2026-10-05) are lazy and reactive. The remaining risk is a stack that's brand new and churning, plus a 64 Hz poll timer.
- Alternatives:
  - "iced — plugin adapter is stale": wrong now (nice-plug-iced 0.5.0, 2026-10-18).
  - "Slint — plugin integration immature": the stronger reason is the licence, which isn't permissive.
  - "vizia — accessibility": doesn't hold inside a plugin (no AccessKit through baseview).
- Decision: keep egui. Name **glow** as the renderer. Make the challenger iced, or none, instead of vizia.

**Task 4 (phase-0.md) should change.**

- `spike-ui-vizia` (vizia-plug) becomes `spike-ui-iced` (nice-plug-iced), time-boxed, or is removed.
- Add the kittest snapshot loop as a deliverable.
- Add pre-steps: reboot, GL check, PRIME mode, native Linux REAPER, Bitwig demo.
- The test host must give the editor an X11 parent window. baseview's `open_parented` example shows how.

**benchmarks.md (UI protocol) should change** as listed in Q7:

- in-plugin per-instance CPU and wake-ups;
- per-process and in-plugin GPU timing, in both PRIME modes;
- first versus marginal PSS;
- "covered" reported but not gated;
- scope fps stated, and the 5% threshold revisited;
- the Orca row replaced;
- scale 2.0 via Xft.dpi;
- perf and powertop permissions noted;
- agent-loop UI metrics added;
- decision rule 2 reworded from "vizia" to "the challenger".
- The "Repaint gating (egui)" paragraph should describe what *our* code must do. The adapter now provides the mechanism.

**AGENTS.md / phase-0.md Task 2 deny.toml:** the reason for the OFL/UFL entries ("fonts egui bundles by default") is now "fonts egui ships behind `default_fonts`". The same licences are still needed if that feature is enabled.

## Gaps

These were blocked by the reboot, a missing host or time. Each item gives the check that would settle it, to run after the reboot (`CARGO_BUILD_JOBS=2`, scratch dirs under `/tmp/enth-research/02/`).

1. **GL on NVIDIA after the reboot.** Run `nvidia-smi`, `glxinfo -B`, and `eglinfo -B | sed -n '/X11 platform/,/^$/p'`. Expect the NVIDIA renderer on both GLX and EGL-X11.
2. **Idle and animating CPU and wake-ups of a real nice-plug-egui 0.6 editor.**
   - Clone nice-plug into an empty `/tmp/enth-research/02/nice-plug/`.
   - Build and run `examples/gain_egui` standalone (see its GETTING_STARTED.md).
   - Then run `pidstat -t -p <pid> 1 60` and compare `voluntary_ctxt_switches` in `/proc/<pid>/task/*/status` over 10 s, with the window idle, covered and minimised.
   - Repeat with `--features wgpu` to get open time and `nvidia-smi pmon -s um`.
3. **Does baseview 0.4 stop drawing when minimised or covered under Cinnamon?** Run the same check while watching per-thread CPU and wake-ups.
4. **Spacebar and typing with egui-baseview in REAPER and Bitwig on X11.** Needs native Linux REAPER (Linux tarball, installs into $HOME) and the Bitwig demo. Load the nice-plug `gain_egui` CLAP, click the editor, press space; then try a TextEdit.
5. **HiDPI.** Run `xrdb -merge <<< 'Xft.dpi: 192'`, reopen the editor in each host, and check size and mouse mapping. Undo with `xrdb -merge <<< 'Xft.dpi: 96'`.
6. **Hybrid-graphics freeze ([egui #7561](https://github.com/emilk/egui/issues/7561)) on driver 595.** Open 10 editors in one host for 10 minutes with focus switching. Repeat after `prime-select on-demand` (owner, sudo, re-login) to compare.
7. **glow versus wgpu on Linux.** Measure editor open time (log `gui.create` to first `draw`), first versus marginal PSS (`/proc/<pid>/smaps_rollup`), and VRAM (`nvidia-smi pmon -s m`), with 1 and 10 editors.
8. **iced arm cost.** Build nice-plug `examples/gain_iced` with the `tiny-skia` and then the `wgpu` feature, and repeat the measurements in item 2. Not measured at all today.
9. **vizia build cost** (skia-safe prebuilt download or a source build) and its idle wake-ups. Not measured, and only relevant if vizia is kept.
10. **Release-mode kittest timing.** Rerun `/tmp/enth-research/02/kittest-spike` with `[profile.dev.package."*"] opt-level = 2` on an idle machine. The clean rebuild didn't finish in 10 minutes under load today.
11. **egui CPU per frame on the real reference panel**, with custom knobs, an image and a 512-bin spectrum, on an idle machine. Rerun `/tmp/enth-research/02/target/release/egui-frame-bench` with the real panel. Today's ~1.0 ms came from a simplified stand-in under load averages of 10–22.
12. **Public idle-CPU numbers** for nice-plug-egui 0.6 or baseview 0.4: none found. Only items 2 and 3 can fill this.
13. **Not verified:** whether REAPER on Linux offers a dedicated-process bridge for CLAP; whether Bitwig calls `set_scale` on Linux; the vsync and swap-interval behaviour of baseview's EGL path with several editors on one thread after PR #357.
