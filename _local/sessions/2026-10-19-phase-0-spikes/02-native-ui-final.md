# 02 — Native UI in plugins (final)

- **Date:** 2026-10-19
- **Sub-researcher:** 02 (native UI in plugins)
- **Questions answered:** 1–7 from the brief, plus every item that was under Gaps in the first report.
- **Relation to the first report:** `02-native-ui.md` stays verbatim. This report is self-contained and supersedes it. Change markers:
  - **[NEW]** = not in the first report;
  - **[CHANGED]** = the first report said something different;
  - **[CONFIRMED]** = re-checked and still true.
- **Evidence markers:**
  - *documented* = code, changelog, licence file or official docs;
  - *anecdotal* = issue or forum report;
  - *measured-real-GPU* = run on the owner's NVIDIA display in run 2, before the crash;
  - *measured-software-GL* = Xvfb with Mesa llvmpipe or lavapipe;
  - *measured* = CPU-only work that doesn't depend on the GPU.

**Runs**

| Run | Time (2026-10-19) | Conditions |
| --- | --- | --- |
| 1 | 01:30–02:23 | First report: web research, kittest on lavapipe, frame bench. Load average about 20 |
| 2 | 02:33–03:12 | After the first reboot, on the real display (PRIME = nvidia, driver 595.99.02). Ended when **Xorg crashed** inside `nvidia_drv.so`. Build outputs lost; numbers recovered from the transcript |
| 3 | 03:44–06:25 | After the second reboot, under new rules: no window or GL context on the real display; everything under `xvfb-run` with Mesa forced (`__GLX_VENDOR_LIBRARY_NAME=mesa`, Mesa EGL vendor file, `VK_ICD_FILENAMES=lvp`); persistent scratch; `flock -x` for timings; load average 6–23 (other researchers) |

All scratch is in `~/code/enth-scratch/2026-10-19-phase-0-spikes/02/`: running notes in `notes.md`, scripts in `scripts/`, raw results in `runs/`, the owner checklist in `OWNER-CHECKLIST.md`.

## Answer in brief

1. **egui stays the right default, but the risk has moved from the toolkit to baseview and the NVIDIA X driver.** [CHANGED]
   - The egui editor embeds in native REAPER 7.82 as CLAP and draws on first open: 280 of 280 opens under Xvfb.
   - Show to first frame took about 100 ms. Idle cost: +1.1 points of REAPER CPU, 0.1 % on the GUI thread, 64 wake-ups/s. *(measured-software-GL)*
2. **baseview 0.4.1 has measurable bugs that every toolkit on it inherits** (egui, iced, raw softbuffer). [NEW] *(measured-software-GL unless marked)*
   - **Leak:** PSS grows on every open/close, even for the GL-free softbuffer example: ≈70 KB per cycle; ≈115–160 KB per cycle with egui on Mesa. REAPER's own ReaEQ is flat over the same cycles.
   - **Frame-rate cap:** pacing tops out near 30 fps, and a 30 fps request lands on 20 fps. That is code plus measurement, and real-GPU too.
   - **HiDPI clipping:** at `Xft.dpi: 192`, REAPER sizes the FX window for scale 1 while baseview draws at scale 2, so the editor is clipped.
   - **Blank editor on NVIDIA:** on the real display (run 2), the standalone egui window often never drew its first frame. Same symptom as nice-plug #95, which is "fixed" upstream since 2026-10-18. *(measured-real-GPU)*
3. **The Xorg crash at 03:12 matches a known NVIDIA 595-series bug.** [NEW]
   - It was a SIGSEGV at `nvidia_drv.so +0x3fc5a` during X resource destruction.
   - The same offset is reported on Linux Mint 22.3 with `prime-select nvidia` and driver 595.84, triggered by short-lived X clients ([NVIDIA forum 370339](https://forums.developer.nvidia.com/t/xorg-sigsegv-in-nvidia-drv-so-during-freepixmap-driver-595-71-05-open-rtx-5080-repeatable/370339); NVIDIA bug 6205955, no fix).
   - Opening and closing plugin editors churns exactly those resources. On this machine that can kill the whole X session, and the DAW with it.
4. **Use glow, not wgpu.** [CONFIRMED]
   - wgpu creates a device per editor *(documented)*.
   - Under software rendering it adds 16–21 MB PSS; the stripped plugin is 13.2 MB vs 7.6 MB; the clean build is slower.
   - Real-GPU numbers are on the owner checklist.
5. **iced is no longer a good challenger.** [CHANGED]
   - nice-plug-iced 0.5.0's `gain_iced` editor spins one core at ≈100 % while open and idle, in standalone and inside REAPER (109 % vs 10.6 % closed).
   - It leaks one thread per open/close (28 → 107 threads over 80 cycles). *(measured-software-GL; the spin needs a real-display check)*
6. **vizia is still not a viable challenger.** [CONFIRMED] Upstream hasn't moved since the first report: vizia-plug last pushed 2026-06-13, vizia 2026-08-21, issue #41 unanswered.
7. **Keyboard in REAPER** (Xvfb, no window manager). [CHANGED: the first report predicted only the spacebar risk]
   - Clicking the editor moves keys to the plugin, so **Space no longer reaches REAPER's transport**.
   - Clicking the text field and typing **works** ("a b" landed in the field).
   - With a real window manager this still needs the owner's hands.
8. **The agent loop is faster than first measured, and a host-level loop now exists.** [NEW]
   - egui_kittest on lavapipe: edit → rebuild → snapshot in **3.5 s**, about 107 ms per 1100×768 render, with a quieter machine.
   - REAPER can be driven fully headless with `Scripts/__startup.lua` + Xvfb + XTEST, with screenshots of the embedded editor.
9. **egui CPU per frame:**
   - Layout plus tessellation is 0.73–0.88 ms p50 *(measured)*.
   - On the real GPU the whole GUI thread was ≈1.7–2.0 ms per frame *(measured-real-GPU, one run)*.
   - At baseview's real 20–30 fps that is ≈3.5–6 % of one core while animating. The 5 % threshold is borderline but reachable at 20–30 fps.
10. **Phase 0 change: drop the second-toolkit arm.** [CHANGED]
    - Spend the days on the baseview/egui integration bugs: first frame, leak, HiDPI, focus, `editor_closed`.
    - Add an NVIDIA safety track: a CPU-rendered fallback, and a PRIME on-demand comparison.
    - Run all automated GUI tests under Xvfb, and real-GPU checks only with the owner present.

## What changed since the first report

| Topic | First report | Now |
| --- | --- | --- |
| Challenger | iced via nice-plug-iced | No second-toolkit arm. iced busy-loops while idle and leaks threads *(measured-software-GL)*; vizia unchanged |
| Repaint gating | "Built in, reactive" *(documented)* | Confirmed in a host: 0 frames while idle, 64 wake-ups/s. **New:** a cap near 30 fps from baseview's MSC+2 pacing |
| First frame | Not known | Often no first frame on the real NVIDIA display (standalone); 280/280 under Xvfb in REAPER; nice-plug #95 |
| Leaks | Unmeasured | Growth on every open/close in every baseview editor; not REAPER's |
| Spacebar | Predicted from code: lost after a click | Measured in REAPER/Xvfb: lost after a click; **text entry works** |
| HiDPI | Unverified | Standalone 2× correct; **REAPER clips the editor** at Xft.dpi 192 |
| glow vs wgpu | Anecdotal (Windows) | Software-GL PSS, binary size and build time measured; real GPU still open |
| kittest loop | 10–12 s, 230–280 ms per render (load about 20) | **3.5 s**, 107 ms per render (load about 6) |
| Frame bench | ≈1.0 ms ≈ 6 % at 60 fps | 0.73–0.88 ms p50 ≈ 4.4–5.8 % at 60 fps; the real rate is 20–30 fps |
| NVIDIA | Driver mismatch | GL works after the reboot; Xorg crash matched to a known 595 bug; X-resource churn is a crash risk |
| Measurement method | Real display | Xvfb for CPU, memory, leaks, keyboard and HiDPI; real GPU only with the owner present |

## Findings

### Q1 — egui inside plugins

**Versions** [CONFIRMED, re-checked 2026-10-19 ≈06:25, no new commits since the first report] *(documented)*

| Crate | Version | Repo |
| --- | --- | --- |
| nice-plug | 0.4.2 | Codeberg `main` 7197104 (2026-10-18) |
| nice-plug-egui | 0.6.0 (2026-10-18) | same |
| egui-baseview | 0.9.0 (2026-10-17) | HEAD = PR #38 "use baseview timer to poll for repaint requests" |
| baseview | 0.4.1 (2026-10-09) | GitHub RustAudio/baseview; master last pushed 2026-10-18 (PR #357 open) |
| egui / egui_glow / egui-wgpu / egui_kittest | 0.37.0 (2026-10-15) | — |

- Maintainers: BillyDM for nice-plug and the egui adapter; Adrien Prokopowicz (prokopyl) for baseview.
- Sub-researcher 01 counts BillyDM as author of 240 of 270 commits and the sole crates.io owner, so the bus factor is 1 (`01-plugin-shell-final.md`).
- Release cadence is about monthly breaking releases. Pin exact versions.

**How repainting works** [CONFIRMED] *(documented, code read)*

- egui-baseview redraws on four triggers:
  - egui's `request_repaint(_after)`;
  - every mouse or keyboard event;
  - a `RepaintNotifier` (atomic flag) that a baseview timer polls every 1/64 s;
  - host parameter or modulation changes, through nice-plug-egui.
- **[NEW]** baseview 0.4.1's X11 frame pacing asks for the next frame at `last_msc + 2` (`event_loop.rs:190`). That is at most one frame every two vblanks, ≈30 fps at 60 Hz. A redraw requested after 33 ms lands on every third vblank (20 fps).
- Measured:
  - real display (run 2): fps30 requested → 20–21 fps;
  - Xvfb: fps30 → 20.6 fps; fps60 → 21–30 fps; audio-thread notifier → 28–30 fps;
  - REAPER/Xvfb: ≈20 fps.
- **[NEW] Possible first-frame stall mechanism** *(documented, unconfirmed)*: the first NotifyMSC request has no timeout. If its completion event never arrives, the "sent our first request but have not gotten a response yet … do not send" branch stops all further drawing (`event_loop.rs:172–207`).

**In a real host: REAPER 7.82, CLAP, Dummy Audio 48 kHz/128** [NEW] *(measured-software-GL, `runs/rp-*`)*

| State | REAPER CPU | GUI thread | Wake-ups | PSS |
| --- | --- | --- | --- | --- |
| Plugin loaded, editor never opened | 10.6 % | — | — | 132 MB |
| Editor open, idle | 11.7 % (+1.1 points) | 0.10 % | 64/s | 205 MB (+73 MB: Mesa llvmpipe loads with the first GL context) |
| Open, scope requested at 30 fps | 77 % (llvmpipe rasterises on the CPU) | 19 % | — | — |

- Open time: TrackFX_Show blocks ≈60 ms; build → first `ui()` 25–51 ms; show → first frame ≈100 ms. That's under the 150 ms warm threshold even in software.
- Sub-researcher 01 independently saw 22/22 opens render, first frame 10–49 ms after build, and +0.2–0.5 points while idle (`01-plugin-shell-final.md`).

**Standalone on the real NVIDIA display (run 2)** *(measured-real-GPU)*

- The egui window often **never drew a first frame**: build() ran, ui() never did. That was most launches, including runs where the window was raised, focused, mouse-wiggled and xwd-captured. Under Xvfb it always drew.
- When it did draw, at fps30: 20–21 fps; GUI thread 33–41 ms of CPU per second (3.3–4.1 % of one core), so ≈1.7–2.0 ms per frame; `ui()` 0.32–0.45 ms; build → first `ui()` 37 ms.
- Idle (no frames): process 0.2–0.45 %; PSS ≈95 MB (Pss_Anon 20 MB) including the NVIDIA libraries; nvidia-smi FB 2 MB.
- Background noise: cinnamon used ≈100 % of one core during all of run 2 (35 min CPU in 40 min).
- The same symptom is [nice-plug #95](https://codeberg.org/RustAudio/nice-plug/issues/95) (2026-09-17): NVIDIA 595.71 and an AMD iGPU, REAPER; also gain_iced and baseview's own `plugin_clack_femtovg`, so it's a baseview race. The maintainer posted "should be fixed in the latest commit" on 2026-10-18. Run 2 used that commit (nice-plug-egui 0.6.0 / egui-baseview 0.9.0), so on this NVIDIA machine it isn't fixed for the standalone path. The host path on the real display is on the owner checklist.

**egui CPU per frame** *(measured)*

- Stand-in panel, release build, layout + tessellation, no GPU.
- Quieter machine (load ≈8): p50 0.73–0.88 ms, p99 0.79–1.64 ms → 4.4–5.8 % of one core at 60 fps, ≈2.2–2.9 % at 30 fps.
- Run 1 under load 10–22: ≈1.0 ms.

**Leaks over open/close cycles** [NEW] *(measured-software-GL, REAPER, TrackFX_Show 3/2 every 0.4 s)*

| Editor | Cycles | PSS after close #1 → last | Per cycle | Threads |
| --- | --- | --- | --- | --- |
| ReaEQ (REAPER's own UI) — control | 80 | 133.0 → 134.3 MB | ≈15 KB | flat 26 |
| byo_gui_softbuffer (baseview, no GL) | 80 | 134.7 → 140.1 MB | ≈70 KB, linear | flat 27 |
| gain_iced (tiny-skia) | 80 | 138.8 → 149.9 MB | ≈140 KB | **28 → 107** |
| gain_egui (glow on llvmpipe) | 80 | 182.1 → 210.2 MB | +17 MB in the first 10, then ≈160 KB | back to 27 |
| enth probe (glow) | 200 | 206 → 229 MB | ≈115 KB | back to 27 |

- All baseview editors fail the protocol's "< 2 MB after 200 cycles". The REAPER control doesn't.
- How much of the egui/Mesa growth would also happen on NVIDIA is unknown.
- stdout repeatedly shows `WARN Error while closing window: Request to X11 thread failed: Could not receive response (X11 thread disconnected)` (baseview 0.4.1).
- heaptrack can't attribute the leak: REAPER segfaults at startup under heaptrack's LD_PRELOAD.

**Other adapter facts**

- `NiceEguiApp::editor_closed()` is declared but never called in nice-plug-egui 0.6.0 (no caller; 0 calls in 200 closes). Sub-researcher 01 found the same. [NEW] *(documented + measured)*
- egui 0.37 made `default_fonts` opt-in, and nice-plug-egui depends on egui without default features. [CONFIRMED]
- Sub-researcher 01 found that `epaint_default_fonts` 0.37 declares `Ubuntu-font-1.0`, which the planned deny.toml's `LicenseRef-UFL-1.0` doesn't match.
- Open adapter and baseview issues from the first report still stand: egui-baseview #20 (focus), #6 (loop not dropped on close); baseview #359 (HiDPI scale source), #357 (X11 events onto the host main thread), #348 (DnD and clipboard), #200 (AccessKit). [CONFIRMED]

### Q2 — vizia and vizia-plug [CONFIRMED]

- vizia 0.4.0 (2026-04-23); repo last pushed 2026-08-21 (re-checked 06:25 today).
  - Renders through skia-safe, pinned at 0.99. The latest skia-safe is 0.153.3 (2026-09-04).
  - The Skia GPU cache defaults to 128 MB.
  - It pins baseview 0.2.2, two breaking versions behind 0.4.1.
- vizia-plug (MIT, 31 stars) was last pushed 2026-06-13 and still targets nice-plug-core 0.1.4.
  - [Issue #41](https://github.com/vizia/vizia-plug/issues/41) (port to nice-plug 0.4.2) has 0 comments.
  - Unmerged ports exist: Atreus `nice-plug-0.4` (2026-10-07) and ebai101 (2026-09-22).
- Repainting:
  - `vizia_baseview` calls `on_frame` at the compositor's vsync rate and redraws only when the dirty list is non-empty. That means 60 wake-ups/s plus bookkeeping while idle *(documented)*.
  - No lazy redraw or Present pacing until vizia moves to baseview 0.4.
- **AccessKit:** the `accesskit` feature only reaches `vizia_winit`. `vizia_baseview` has no AccessKit code, and baseview #200 is open. vizia's accessibility advantage doesn't exist inside a plugin.
- **Testing:** no headless mode ([PR #687](https://github.com/vizia/vizia/pull/687) open since 2026-05-03) and no test harness.
- **Build cost** [NEW] *(documented)*: skia-bindings' build.rs downloads prebuilt Skia from `github.com/rust-skia/skia-binaries` with curl, or builds Skia from source (LLVM, Python 3, Ninja). That means a network fetch in every clean CI build unless cached.
- **Not built:** porting the adapter first would have cost hours for a toolkit already ruled out by the points above. Listed under Gaps.

### Q3 — HiDPI, keyboard focus and text input; NVIDIA

**The owner's display and GPU** *(measured)*

- 1920×1080 at 60 Hz, Xft.dpi 96, so the real scale is 1.0.
- `prime-select` reports `nvidia`: the X server and every GL client run on the Quadro RTX 3000.
- After the reboots GL works: GLX and EGL-X11 report "Quadro RTX 3000 … 4.6.0 NVIDIA 595.99.02".
- Kernel log: `nvidia-drm … Failed to get memory pages for NvKmsKapiMemory` at boot and right after the crash.

**HiDPI** [NEW] *(measured-software-GL)*

- Setting `Xft.dpi: 192` on the root window (Xvfb `-noreset`, else the server resets and drops it): the standalone egui window is 2200×1536 physical and renders crisply at 2×.
- **In REAPER 7.82 (CLAP), REAPER's FX window stays 1100×796 while the plugin's child window is 2200×1536, so the editor is clipped to its top-left quarter** (`runs/rp-hidpi/end.png`).
- baseview takes the scale from Xft.dpi; the size REAPER got via the CLAP GUI extension is the logical size; REAPER (SWELL, `ui_scale=1`) didn't scale itself. Compare [baseview #359](https://github.com/RustAudio/baseview/issues/359).
- The checklist item "correct at HiDPI scale 2.0" fails in REAPER in this configuration.

**Keyboard and text** [NEW] *(measured-software-GL; Xvfb has no window manager)*

| Step | Result |
| --- | --- |
| REAPER's main window focused, press Space | Transport toggles (baseline OK) |
| Raise FX window, click an empty editor area, press Space | egui logs `Space` and **REAPER's transport doesn't toggle**: the spacebar is lost to the host after a click in the editor |
| Click the egui TextEdit (X focus moves to baseview's child window), type "a␣b" | Text appears ("a b"), transport untouched: **text entry works in REAPER CLAP** here, unlike the nih-plug #219 report |
| Click REAPER's main window, press Space | Space still went to the plugin. Probably a no-window-manager artefact (nothing moves X focus); must be checked under Cinnamon |

- *Documented:* baseview's X11 backend never forwards ignored key events to the parent, and egui-baseview calls `focus()` on every mouse press.
- *Design consequence:* take X focus only while a text field is being edited, and give it back afterwards. Needs an upstream change, or a fork of egui-baseview's `on_event`.

**NVIDIA**

- Run 2 *(measured-real-GPU)*: first-frame stalls; a 20 fps result for a 30 fps request; cinnamon at 100 %.
- **The Xorg crash** [NEW] (journal of the previous boot, systemd-coredump of PID 1160):
  - SIGSEGV → `FatalError` → abort, with frames `nvidia_drv.so +0x3fc5a` ← `+0x22041c`.
  - It happened at 03:12:14 while my loop was launching the egui probe and killing it with SIGTERM every 4 s on the real display.
  - Sub-researcher 01 was also running GUI clients (Carla, validators) on the real display.
  - Correlation, not proof.
- The NVIDIA developer-forum thread [370339](https://forums.developer.nvidia.com/t/xorg-sigsegv-in-nvidia-drv-so-during-freepixmap-driver-595-71-05-open-rtx-5080-repeatable/370339) *(anecdotal)*:
  - 2026-05-16: RTX 5080, 595.71.05, crash in FreePixmap/damageDestroyPixmap; NVIDIA internal bug 6205955 (2026-05-22); no fix; 595.80 and 610.43.02 reported affected.
  - **2026-09-08: RTX 4090 Laptop, `prime-select nvidia`, Linux Mint 22.3, 595.84. Faulting frame `nvidia_drv.so +0x3fc5a`, our offset, caller `FreeResource()`, triggered by short-lived X clients churning resources** (about 7 of 26 Steam launches).
  - Also an RTX 5090 HDMI-unplug Xorg crash across 580.119–595.58 ([thread](https://forums.developer.nvidia.com/t/xorg-sigsegv-nvidia-drm-warning-on-hdmi-hotplug-disconnect-rtx-5090-max-q-gb203m-reproducible-across-580-119-02-580-126-18-580-142-595-58/366327)).
- From the first report, still standing *(anecdotal)*: [egui #7561](https://github.com/emilk/egui/issues/7561) (hybrid-graphics freezes on Ubuntu X11 with several instances, `libGLX_nvidia`); [egui #8416](https://github.com/emilk/egui/issues/8416) (wgpu freeze, NVIDIA 610, Wayland).

**What the crash implies for plugin GUIs on this machine** [NEW]

1. The faulting path is X resource destruction in the NVIDIA X driver. A plugin editor's open/close destroys windows, GL drawables and pixmaps every time. A user (or a 200-cycle leak test) who opens and closes editors a lot can crash Xorg, and with it REAPER and unsaved work. This isn't egui-specific: any toolkit, and many non-plugin apps, can trigger it.
2. Real-display GUI testing on this machine must be owner-attended with work saved. Automated GUI tests (CI, agents) must run under Xvfb/Mesa. Their CPU numbers overstate GPU-rendered cost; memory, leak, threading, keyboard and HiDPI results transfer.
3. Mitigations to evaluate:
   - PRIME **on-demand**: the desktop renders on Intel, and plugins render on Intel unless offloaded.
   - Another driver branch: the reports say no known-good 595/610 build.
   - A CPU-rendered fallback for the editor (softbuffer).
   - Keeping the editor's X window alive across hide/show where the host allows it, though CLAP's `gui.destroy` limits this.
   - Reporting our backtrace to NVIDIA bug 6205955.

**Bitwig:** not installed, so not tested. Its "Individually" hosting mode would allow per-instance measurement ([user guide](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)).

### Q4 — glow or wgpu for egui

- *Documented:* egui-baseview's wgpu renderer creates a new `wgpu::Instance`, adapter and device for every editor it opens. glow uses baseview's GL context plus one painter. [CONFIRMED]
- *Measured-software-GL* [NEW]:

| | glow | wgpu |
| --- | --- | --- |
| Clean release build | 5 min 05 s | 6 min 45 s (other load) |
| Stripped plugin .so, no LTO | 7.6 MB | 13.2 MB |
| PSS while idle | 94.5 MB | 110–116 MB (+16–21 MB) |
| Launch → build | 75–94 ms | 73–115 ms |
| Build → first `ui()` | 25–38 ms | 21–32 ms |
| fps30 request | 20.6 fps | 19–20 fps |

  Software numbers say nothing about NVIDIA Vulkan device-creation time or VRAM, the anecdotal 1–7 s / 265 MB in egui #7761.
- *Anecdotal* (first report, still standing): Windows eframe 0.34, glow ≈31 MB RAM and VRAM vs wgpu-Vulkan ≈90 MB / 265 MB; startup 100–300 ms vs 1–7 s ([egui #7761](https://github.com/emilk/egui/issues/7761)).
- **Conclusion:** glow. Measure wgpu once on the real GPU (owner checklist step 1) for open time and VRAM.

### Q5 — Alternatives as of October 2026

| Option | Status | Verdict |
| --- | --- | --- |
| **iced** via nice-plug-iced 0.5.0 / iced_baseview 0.6.0 (2026-10-18) | **[NEW] measured-software-GL:** idle editor at ≈100 % of one core (standalone and REAPER; one spinning thread, `top -H`); one thread leaked per open/close (28 → 107 over 80 cycles); ≈140 KB/cycle PSS. Also hit by #95 per the issue. Strengths stand: tiny-skia CPU renderer (standalone PSS **8.3 MB**), markdown widget, `iced_test` | **[CHANGED]** Not a Phase 0 challenger until the spin and thread leak are fixed upstream. Re-check after the next nice-plug-iced release |
| **vizia** | See Q2 | Not viable |
| **Slint** | Licence: GPL-3.0-only or royalty-free (attribution, no embedded) or commercial. [NEW] nice-plug [#39](https://codeberg.org/RustAudio/nice-plug/issues/39) is upstreaming a Slint adapter; `RustAudio/slint-baseview` created 2026-06-06 | Excluded by the permissive-only rule, regardless of maturity |
| **Xilem/Masonry** | 0.4.0 (2025-10-29), "experimental"; personal masonry_baseview only | Too early |
| **Raw softbuffer / vello_cpu / tiny-skia / femtovg** | [NEW] nice-plug's `byo_gui_softbuffer`: PSS 3.8 MB standalone, 0.17–0.25 % idle, 2.2 MB plugin; ≈70 KB/cycle leak (baseview's) | Viable as an **NVIDIA-safe fallback renderer**, not a whole-editor toolkit. egui's CPU backend (`egui_software_backend` 0.0.3) targets egui 0.34 and would need porting |
| **truce** | Own framework with egui/Slint backends; custom licence with a commercial-framework rider | Not on the allowed list as is (sub-researcher 01's area) |

**Is "egui default, vizia challenger" still right?**

- **egui default: yes.** It is the most active adapter, works in REAPER, has the best testing story, and costs least at idle.
- **Challenger: none in Phase 0.** All three toolkits sit on the same baseview, whose bugs dominate. The most useful "second arm" is a renderer fallback (GL vs CPU), not a second toolkit.

### Q6 — UI testing and AI feedback

**egui_kittest 0.37.0** [CHANGED: faster than the first report] *(measured)*

- Setup: lavapipe forced, dev profile, `flock -x`, load ≈6.
- Clean build of the test: 1 min 15 s of compile time (3 jobs), peak RSS 1.04 GB.
- Each run: 2.65–2.81 s, 270 MB RSS.
- One 1100×768 render ≈106–108 ms; first render 260–340 ms; `snapshot()` ≈185 ms; scale-2.0 harness + render 650–720 ms.
- **Edit → incremental rebuild (1.0 s) → snapshot test (2.5 s) = 3.5 s.** The PNG shows the whole panel, and the AccessKit queries make tests addressable by label.
- *Documented:* egui's own CI runs its snapshot tests on `macos-latest` "because it will run with an actual GPU". enth should generate and compare golden images on one renderer only: lavapipe locally and in Linux CI (`mesa-vulkan-drivers`).

**A host-level loop** [NEW] *(measured-software-GL)*

- Native REAPER 7.82 can be driven headless from a scratch copy of the portable install:
  - `Scripts/__startup.lua` adds the plugin (`TrackFX_AddByName`), opens and closes its window (`TrackFX_Show`), logs `GetPlayState()`;
  - `xvfb-run` with Mesa provides the display; `python3-xlib` XTEST clicks and types; `xwd | ffmpeg` takes screenshots;
  - `linux_audio_mode=2` selects Dummy Audio.
- A coding agent can see the real embedded editor and test keyboard, HiDPI and leaks without touching the real display.
- First-run dialogs:
  - "portable install — scan system VST/CLAP/LV2 paths? Yes/No" (answered No in the scratch copy);
  - the evaluation nag ("Buy Me [2]") on one launch, never clicked, and it didn't block the scripts.
- Sub-researcher 01 found a similar headless REAPER recipe.

**Others**

- iced has `iced_test` (headless simulator, snapshots).
- vizia has nothing (PR #687 open).
- `egui_software_backend` (CPU kittest renderer) is stuck on egui 0.34.

### Q7 — Does the benchmark protocol measure the right things? Tools

Local facts *(measured)*:

- `perf_event_paranoid` is 4, so `perf` is unusable without sudo.
- RAPL is root-only; battery power is readable as `current_now × voltage_now`.
- pidstat, smem and heaptrack are now installed, but heaptrack crashes REAPER.
- nvidia-smi `pmon -s um` works per process (columns sm %, fb MB).
- Xvfb resets on last-client exit unless `-noreset`.

**Changes the evidence supports (all from Xvfb runs unless noted):**

1. **Two tiers of GUI measurement.**
   - Automated tier, under Xvfb + Mesa: CPU, wake-ups, PSS, leaks, threads, open time, keyboard, HiDPI, screenshots.
   - Owner-attended tier on the real GPU, with work saved: frame pacing, GPU %, VRAM, first-frame behaviour, crash exposure.
   - Never automate rapid editor open/close on the real NVIDIA display.
2. **Per-editor CPU:**
   - name the GUI thread (`prctl(PR_SET_NAME)` inside `build()` works);
   - log `CLOCK_THREAD_CPUTIME_ID` per window of frames;
   - sample `/proc/<pid>/task/*`.
   Thread attribution must be revisited when baseview PR #357 moves X11 events onto the host's main thread.
3. **Wake-ups per second** as a metric: egui-baseview polls every 1/64 s (64/s on the GUI thread); the standalone main thread wakes ≈93/s.
4. **Frame rate:** state the target as "≤ 30 fps scopes" while baseview 0.4.1 caps near 30 fps. "Frame time p99 < 16.7 ms" should become "CPU time per frame" plus "achieved fps vs requested".
5. **Leak test:**
   - keep it, but run it in Xvfb against a host control (ReaEQ was flat);
   - report per-cycle growth;
   - expect baseview itself to fail < 2 MB/200 until fixed upstream;
   - also count threads (iced leaked one per cycle).
6. **HiDPI:** test in the host, not standalone, with `Xft.dpi: 192` set on the root window (Xvfb `-noreset`). REAPER clips.
7. **Keyboard:** add "Space after clicking an empty editor area" and "focus returns to the host after clicking the host" as separate checks. The first currently fails.
8. **First open:** add "first open draws within 1 s, 20 of 20 tries, real display" (nice-plug #95).
9. **Covered/minimised:** under Xvfb, an override-redirect window over the editor does *not* stop drawing (no compositor). Under Cinnamon this is unknown (owner checklist). Gate only on "closed".
10. **GPU per process:** `nvidia-smi pmon -s um -d 1` (owner tier). Account for compositor (cinnamon) GPU and CPU deltas: cinnamon used 11–35 % SM and ≈100 % CPU during run 2.
11. **Per-instance isolation:** REAPER ships `Plugins/reaper_host_x86_64` (its bridge host). Whether CLAP bridging works on Linux is untested. Bitwig "Individually" is unavailable (Bitwig not installed).
12. **Agent-loop UI metrics:** edit → snapshot time (3.5 s), clean build per arm, stripped binary size.

## Recommendation for Phase 0

1. **Pin** nice-plug 0.4.2, nice-plug-egui 0.6.0, egui-baseview 0.9.0, baseview 0.4.1, egui 0.37.0 (`=` + Cargo.lock). Enable `egui/default_fonts` (or ship fonts) and fix the deny.toml font licence (sub-researcher 01).
2. **egui on glow** for `spike-ui-egui`. Gate repaints: scopes call `request_repaint_after(1/30 s)` only while visible and fed. Name the GUI thread; log CPU time per frame window.
3. **Replace the vizia (and iced) arm with an integration-bug track**, about 2–3 days, each with an upstream issue or PR:
   - **First-frame stall:** reproduce on the real display with the owner; propose a timeout on the first NotifyMSC in baseview.
   - **Open/close leak:** baseview ≈70 KB/cycle; find it with a heaptrack-friendly host (a clack-host test host, Task 4 step 1) rather than REAPER.
   - **HiDPI scale mismatch** in REAPER (#359).
   - **Focus policy:** focus only for text fields; return focus to the host.
   - **`editor_closed()` never called.**
   - **Close warning** "X11 thread disconnected".
4. **An NVIDIA safety track** (owner-attended, about half a day):
   - glow vs wgpu on the real GPU (open time, VRAM, fps);
   - first-open reliability, 20 opens in REAPER;
   - the same with `prime-select on-demand`;
   - decide whether a CPU-rendered fallback (softbuffer, or tiny-skia painting the scope) is needed.
5. **Build the automated GUI loop on day 1:**
   - kittest snapshots on lavapipe (CI with `mesa-vulkan-drivers`);
   - the headless REAPER harness (startup Lua + Xvfb + XTEST + screenshots) for host-level checks: open, keyboard, HiDPI, leaks;
   - Phase 0's `tools/test-host` (clack-host) for open time, cycles and heaptrack.
6. **Thresholds:**
   - keep "< 5 % animating" at ≤ 30 fps;
   - add wake-ups/s, first-open reliability and the two keyboard checks;
   - treat the leak gate as "no growth beyond the baseview baseline" until baseview is fixed, and record the baseline.

## Plan check — what should change

- **ADR-0002** (status Proposed):
  - Keep "native Rust, egui by default", and **name glow**.
  - Remove vizia as challenger, and don't substitute iced. Record why: vizia stale, no AccessKit in plugins, Skia; iced idle busy-loop and thread leak.
  - Replace the "egui redraws continuously" context bullet with: reactive repaint is built in; the risks are baseview (first frame, leak, ≈30 fps cap, HiDPI) and the NVIDIA X driver (Xorg crash on resource churn).
  - "How we'd know we're wrong" should add: the real-GPU first-open test fails, or the NVIDIA crash recurs with plugin editors.
- **Task 4 (phase-0.md):**
  - Drop `spike-ui-vizia`.
  - Add the integration-bug track and the NVIDIA safety track (owner-attended).
  - Require all automated GUI runs under Xvfb.
  - Make the test host (step 1) the heaptrack-capable cycle runner.
  - The REAPER-under-Xvfb harness is a cheap addition; the native Linux REAPER 7.82 portable install now exists.
- **benchmarks.md (UI protocol):** apply Q7 items 1–12: the two tiers; wake-ups; ≤ 30 fps until baseview changes; leak gate vs baseline plus thread count; in-host HiDPI; the two keyboard checks; first-open reliability; covered state reported only; compositor accounting. The results template gains rows for wake-ups/s, first-open success, threads after N cycles, per-cycle PSS, achieved fps.
- **AGENTS.md:** add "never open GUI windows on the real display from agents; use `xvfb-run` with Mesa" (this machine's Xorg crash). The fonts note becomes "egui fonts are opt-in via `default_fonts`; licence `Ubuntu-font-1.0`" (sub-researcher 01).
- **research.md §3/§4:**
  - "egui-baseview archived … BillyDM's fork" is now `RustAudio/egui-baseview` on Codeberg.
  - The "always redraws" row should be replaced with the measured reactive behaviour and the baseview caveats above.

## Owner-attended checklist (real display; exact commands in `OWNER-CHECKLIST.md`)

Save your work first: the Xorg crash may recur. Run with ≥10 s between launches.

1. **glow vs wgpu on the real GPU.** `scripts/measure-real.py` (refuses unless `ENTH_OWNER_PRESENT=1`) on `target-probe/release/enth_ui_probe` and `target-probe-wgpu/release/enth_ui_probe`, `ENTH_UI_MODE=idle` and `fps30`. Record: blank or drawn, launch → first ui, fps, GUI-thread CPU, wake-ups, PSS, nvidia-smi SM %/FB MB.
2. **Covered and minimised** under Cinnamon: during the fps30 run, cover the window, then minimise it, with `pidstat -t -p $(pidof enth_ui_probe) 1 30`.
3. **Native REAPER with `clap/enth_ui_probe.clap`:**
   - first open draws? (20 tries);
   - Space after clicking an empty editor area;
   - typing in "Preset name";
   - Space after clicking REAPER's arrange view;
   - `pidstat` / `nvidia-smi pmon` with the editor open and idle.
4. **HiDPI:** `echo "Xft.dpi: 192" | xrdb -merge`, restart REAPER, reopen (expect clipping); revert with `Xft.dpi: 96`.
5. **iced busy-loop on the real display:** `ICED_BACKEND=tiny-skia target-np/release/gain_iced --backend dummy & top -H -p $!`.
6. **Highest crash risk, everything saved:** 50 open/close cycles of the scratch REAPER copy on the real display (command in the checklist file).
7. **Optional:** `sudo prime-select on-demand`, reboot, repeat 1 and 3.

## Gaps

- **Real-GPU numbers** (glow vs wgpu open time, VRAM, GPU %, fps on NVIDIA; first-open reliability in REAPER on the real display): blocked by the no-real-display rule after the crash. Settled by owner checklist steps 1, 3 and 6.
- **Leak attribution** (baseview vs Mesa vs egui-baseview): REAPER segfaults under heaptrack. Settle with a clack-host test host under heaptrack, 20 vs 100 cycles, comparing leaked bytes by caller.
- **Whether the iced busy-loop and the editor still drawing while covered are Xvfb artefacts:** owner checklist steps 2 and 5.
- **Keyboard focus under a real window manager** (does clicking REAPER return keys to the host?): owner checklist step 3.
- **Bitwig and Ardour:** not installed. With the Bitwig demo installed, run the same Lua-less checks by hand; its "Individually" mode gives per-instance processes.
- **vizia build time and idle cost:** not measured (adapter needs porting; Skia binaries download). Only worth doing if vizia-plug merges a nice-plug-0.4 port.
- **REAPER CLAP bridging on Linux** (`reaper_host_x86_64`) for per-instance measurement: untested. Toggle "Run as dedicated process" on the FX in REAPER and check `ps` for a host process.
- **The cinnamon 100 % CPU in run 2:** not explained. If it recurs, restart Cinnamon (Ctrl+Alt+Esc) before measuring.
- **AccessKit/Orca in plugins:** unchanged. No baseview adapter has it (baseview #200); egui's tree is testable through kittest only.
