# Hosting third-party plugins inside a Rust CLAP synth (plugin-in-plugin): precedents, tooling, pitfalls, cost

Context: MIT-licensed Rust synth, shell = nice-plug (NIH-plug fork), egui UI on baseview, developed on Linux Mint/X11 + REAPER, must ship on Windows/macOS. It has a deterministic engine used for scoring and an offline CLI renderer. Research date: 2026-10-18. All repo-health facts are dated. GitHub's API was blocked in this session, so repo health comes from crates.io API data (release dates, downloads) and GitHub HTML pages fetched on 2026-10-18.

## 1. Precedents: who hosts third-party plugins inside a plugin, and how do they handle GUIs, latency, state and crashes?

### Takeaway
Plugin-in-plugin hosting is an established commercial category: Blue Cat PatchWork, DDMF Metaplugin, PluginGuru Unify and Kushview Element (free, GPL) all do it, and Carla does it on Linux. The "synth with a third-party FX slot" design is rare, though. The synth-ecosystem precedent (Kilohearts Phase Plant, Snap Heap, Multipass) keeps hosting inside a closed format (Snapins), so presets stay loadable for everyone. Products that do host third-party plugins document latency compensation, but their pages are mostly silent on crash isolation. Only DAWs (Bitwig) and bridges (yabridge, Carla) document out-of-process sandboxing.

### Cited Findings
**Blue Cat PatchWork (commercial chainer)**
- Hosts VST, VST3 and AU, with up to 64 plug-ins per instance in serial and parallel (up to 8 parallel chains, up to 8 virtual instruments). It ships as VST/VST3/AU/AAX plus a standalone app. Price $99, v2.78, Windows and macOS only (no Linux). The KVR page does not mention CLAP hosting. — [KVR Marketplace: PatchWork](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio)
- Latency: "Built-in latency compensation ensures phase alignment across all sub-hosted software." MIDI can be passed between sub-plugins or back to the host, and parameters can be mapped to MIDI controllers. — [KVR Marketplace: PatchWork](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio)
- The product page says nothing about sub-plugin GUI embedding versus separate windows, or about crash sandboxing. A 2017 user review says presets save window states and positions. — [KVR Marketplace: PatchWork](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio)

**DDMF Metaplugin**
- Hosts VST2, VST3 and AU (CLAP not mentioned). Ships as VST/AU/AAX/RTAS on Mac and Windows. Supports DAW plug-in delay compensation, up to 4x oversampling of the whole hosted chain, and 32-bit plug-ins "if your OS allows them". Price £34. Review published Jan 2020, so this is stale. — [Sound On Sound review, Metaplugin 3](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)
- Automation goes through "an assignable list that maps DAW automation to chosen hosted parameters". The reviewer found this "more fiddly than direct DAW parameter access". This is the fixed-proxy-parameter pattern a CLAP synth would also need. — [Sound On Sound review, Metaplugin 3](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)
- Crash handling and sandboxing are not mentioned. — [Sound On Sound review](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)

**PluginGuru Unify (an instrument plugin built around hosting other plugins; the closest analogue to a "synth with slots")**
- Described as "a powerful and flexible VST host plugin". It layers Instrument, Auxiliary and MIDI-effect layers and works in any DAW. The AU build can load VSTs inside Logic. — [Bedroom Producers Blog review (2020)](https://bedroomproducersblog.com/?p=34815)
- Reproducibility strategy: it ships a bundled sampler, built-in effects and freeware plugins with the installer. Co-developer getdunne (Shane Dunne, 2020-01-10): "The 400 included patches use ONLY the bundled and built-in plug-ins"; future libraries would add "one or two popular plug-ins (e.g. Serum and Spire)". — [KVR forum, Unify thread](https://www.kvraudio.com/forum/viewtopic.php?p=7632550); [BPB review](https://bedroomproducersblog.com/?p=34815)
- User concern in the same thread (krraqk, 2020-01-10): relying on external plugins could create "very segmented user niches, because there are countless plugins and it is impossible that we all have the same ones." — [KVR forum](https://www.kvraudio.com/forum/viewtopic.php?p=7632550)
- The 2020 review says macro knobs could not link to third-party plugins not bundled with Unify. This is stale and may since have been fixed. — [BPB review](https://bedroomproducersblog.com/?p=34815)

**Kushview Element (open source)**
- A "modular AU/VST/VST3/CLAP/LV2 audio plugin host" that also ships as a plugin (VST, VST3, AU, AAX, CLAP, LV2) and as a standalone app. Runs on Windows, macOS and Linux ("Mostly tested on Ubuntu"). GPL, free, v1.1.1 listed on KVR (no date; a user review dated Sept 2026 covers v1.0). — [KVR: Element](https://www.kvraudio.com/product/element-by-kushview)
- Sandboxing and latency compensation are not documented on KVR. A 2023 user review of 0.46.6 reported a hosted VST's data-entry popup freezing with the window hidden behind the host window. This is an old, user-reported GUI-nesting bug. — [KVR: Element](https://www.kvraudio.com/product/element-by-kushview)

**Carla (falkTX)**
- Hosts LADSPA, DSSI, LV2, VST2, VST3 and AU, and is itself available as an LV2 and VST2 plugin (rack/patchbay modes). Plugin bridges (separate process, for example 32-bit or Windows plugins on Linux) are listed as experimental/WIP. GPL-2.0-or-later. 2.1k stars and 705 open issues as fetched 2026-10-18; release and commit dates were not visible. — [GitHub falkTX/Carla](https://github.com/falkTX/Carla)

**Kilohearts (Phase Plant / Snap Heap / Multipass): closed-format hosting**
- Snap Heap is described as a "Snapin Host" where "all our effects plugins can be used as building blocks". The post (2020-10-28) mentions only Kilohearts effects, not third-party VST/AU/CLAP. — [Kilohearts blog, "wtf is a snap heap"](https://kilohearts.com/blog/wtf_is_a_snap_heap)
- Preset portability: Snap Heap presets "work even if you don't own every Snapin they use". Macros remain tweakable, but parameters of unlicensed effects are not. Chains can be saved as presets and loaded into other Kilohearts hosts such as Phase Plant. — [Kilohearts blog](https://kilohearts.com/blog/wtf_is_a_snap_heap)

**Bitwig Studio (DAW-level crash isolation; the reference for sandbox modes)**
- Five hosting modes: Within Bitwig (in the engine's process; "one plug-in crashing would also crash the audio engine"), Together (all plug-ins in one separate process), By manufacturer, By plug-in, and Individually ("This will require more computing resources, but that is the trade-off"). A crashed plugin's panel shows a notification with Reload Plug-in / Reload All Plug-ins buttons. — [Bitwig user guide: Plug-in Handling and Options](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)

**yabridge (out-of-process bridge; evidence on IPC and editor-embedding costs)**
- Bridged plugins "should have a minimal performance overhead, but you may still notice an increase in latency spikes and overall DSP load." Plugin groups put several plugins in one process, which "greatly reduces the loading time", but "plugins are no longer sandboxed from each other". — [GitHub robbert-vdh/yabridge](https://github.com/robbert-vdh/yabridge)
- Embedded-editor pitfalls documented there: keyboard focus (yabridge "will always focus the parent window passed by the host", so the host loses typed spaces), REAPER's FX window intercepting drag-and-drop ("a long standing issue in REAPER's FX window implementation"), HiDPI scaling, wrong GUI placement after expansion, redraw failures, and Xorg's 256-client limit when many plugin windows are open. — [GitHub robbert-vdh/yabridge](https://github.com/robbert-vdh/yabridge)

### Inferences
- The products that host third-party plugins (PatchWork, Metaplugin, Unify, Element, Carla) are chainers/hosts by identity. Their users expect plugin management (scanning, PDC, automation mapping). A synth adding a hosting slot takes on that whole support surface for one feature.
- Kilohearts' choice of a closed module format is the strongest synth-world signal. It keeps every preset loadable, which is exactly the property a deterministic, shareable-patch synth needs.
- No surveyed chainer documents in-process crash protection. Plausibly a hosted plugin crash takes down the DAW, and the user blames the outer plugin. Bitwig's modes show that isolation is a host-level resource trade-off.

### Gaps
- No primary source fetched for Plogue Bidule, Cableguys, Bitwig Grid (third-party hosting inside Grid), or Vital, Serum 2 and Phase Plant explicitly *not* hosting third-party plugins. My understanding is that none of them host third-party VST/CLAP inside the synth, but this was not verified in this session.
- Neither the PatchWork nor the Metaplugin page describes how sub-plugin editors are shown (embedded or floating) or crash behaviour. Their manuals were not fetched.
- How Unify currently handles a patch that references a plugin the user lacks (placeholder, browser filter, or error) was not found.
- Whether PatchWork or Metaplugin now host CLAP (2026) was not verified.

## 2. Rust tooling for hosting (status as of 2026-10-18)

### Takeaway
CLAP hosting in Rust is viable now. clack-host (MIT/Apache) reached crates.io in May 2026 (0.2.0 on 2026-09-12) and ships host examples. VST3 hosting has MIT/Apache bindings (`vst3` 0.3.0, Dec 2025) plus a young convenience crate (`vst3-host`, MIT, 0.10.0 on 2026-10-09) that has only been exercised with real plugins on macOS. LV2 hosting crates are stale (last releases Nov 2024). Several 2026 "all-format host" crates exist but have tiny adoption (hundreds of downloads). Nothing here is mature in Linux/Windows GUI embedding.

### Cited Findings
**clack (CLAP, prokopyl)**
- Crates: clack-host / clack-plugin / clack-extensions / clack-common, "Low-level, safe Rust wrappers to create audio plugin hosts using the CLAP audio plugin API". License MIT OR Apache-2.0. First crates.io release 2026-05-03 (0.1.0), then 0.1.1 on 2026-07-29 and 0.2.0 on 2026-09-12. clack-host has about 33.9k downloads (all-time, as of 2026-10-18). — [crates.io clack-host](https://crates.io/crates/clack-host)
- README: "feature-complete" but "still under active development and constant refinement", with possible breaking API changes and incomplete docs and examples. It includes a basic host example and "a more featured CPAL-based host in `host/examples/cpal`". 239 stars, 31 forks, 376 commits (GitHub page fetched 2026-10-18). Built on `clap-sys`. — [GitHub prokopyl/clack](https://github.com/prokopyl/clack)
- Reverse dependencies of clack-host on crates.io (2026-10-18): only `clack-extensions` and `plughost-formats`. Downstream hosting adoption is thin. — [crates.io clack-host](https://crates.io/crates/clack-host)

**`vst3` crate (coupler-rs, Micah Johnston)**
- "Rust bindings for the VST 3 API", MIT OR Apache-2.0. Releases: 0.1.0 (2023-08-07) … 0.1.2 (2024-04-30), 0.2.0 (2025-11-01), 0.3.0 (2025-12-07). About 96k downloads. — [crates.io vst3](https://crates.io/crates/vst3)
- It "ships pre-generated bindings", so no SDK is needed to build (as described by its dependent vst3-host). — [docs.rs vst3-host 0.5.0](https://docs.rs/crate/vst3-host/0.5.0)
- These are raw COM bindings. Host-side plumbing (module loading, IHostApplication, IComponentHandler, IPlugFrame, run loop) is the developer's job. *Inference from "bindings" scope; see Gaps.*

**`vst3-host` crate (HelgeSverre/rust-vst3-host)**
- "Safe, simple VST3 plugin hosting with audio playback, MIDI, and crash protection", MIT. Created 2026-06-20. Twelve-plus releases in four months (0.2.1 on 2026-06-22 → 0.10.0 on 2026-10-09), about 5.5k downloads. — [crates.io vst3-host](https://crates.io/crates/vst3-host)
- Status (0.10.0): "The core is working and exercised against real plugins on macOS." "Windows/Linux build and test in CI but aren't interactively exercised (no plugin run or editor opened)." The default `play` path is "correctness-first (it locks on the audio callback)". `play_realtime` is lock-free (SPSC ring) but not "a fully RT-audited (zero-allocation) engine yet". — [docs.rs vst3-host latest](https://docs.rs/crate/vst3-host/latest)
- Process isolation is opt-in (`with_process_isolation(true)`) via a `vst3-host-helper` binary. A crash returns `Error::PluginCrashed`, and `Plugin::recover()` respawns the helper and reloads. Editors across the isolation boundary work on macOS only. `EmbeddedEditor` (egui/eframe embedding) is macOS-only, and Windows/Linux window code "compiles but isn't yet runtime-verified". save_state/load_state work in-process and isolated. No latency-reporting API is described. — [docs.rs vst3-host latest](https://docs.rs/crate/vst3-host/latest)
- Depends on `vst3` ^0.3.0, cpal, midir, libloading, rtrb and optional egui. A Linux build needs libclang, ALSA and libxcb headers. — [docs.rs vst3-host 0.5.0](https://docs.rs/crate/vst3-host/0.5.0)

**Other 2026 multi-format host crates (low adoption; crates.io data as of 2026-10-18)**
- `truce-rack` 1.1.5 (2026-06-19; created 2026-05-15): "Cross-format audio plugin host (CLAP + VST3 by default; AU, AU3, LV2 behind features)", MIT OR Apache-2.0, 551 downloads. — [crates.io truce-rack](https://crates.io/crates/truce-rack)
- `plughost-formats` 0.1.2 (2026-10-11; created 2026-09-30): "In-process VST3, Audio Unit, and CLAP hosting", MIT OR Apache-2.0, 130 downloads. — [crates.io plughost-formats](https://crates.io/crates/plughost-formats)
- `rack` 0.4.8 (2025-11-15): "A modern Rust library for hosting audio plugins", MIT OR Apache-2.0, about 5.6k downloads. No release in about 11 months. — [crates.io rack](https://crates.io/crates/rack)
- `maolan-plugin-host` 0.1.2 (2026-10-14): "Out-of-process audio plugin hosting library", **BSD-2-Clause**, 453 downloads. — [crates.io maolan-plugin-host](https://crates.io/crates/maolan-plugin-host)
- `plugin_host` 0.1.0 (single release 2026-02-25, no repository listed): "VST3 and CLAP plugin host for DAW — scanner, sandboxing…", MIT, 206 downloads. — [crates.io plugin_host](https://crates.io/crates/plugin_host)

**LV2 hosting crates (stale)**
- `livi` 0.7.5, "a library for hosting LV2 plugins", MIT. Last release 2024-11-12 (about 23 months before this research; previous 0.7.4 was 2023-10-07). About 42k downloads. — [crates.io livi](https://crates.io/crates/livi)
- `lilv` (Rust bindings) 0.2.4, MIT, last release 2024-11-30, no repository listed. — [crates.io lilv](https://crates.io/crates/lilv)
- `lv2-host-minimal` 0.1.4, last updated 2021-11-26 (abandoned). — [crates.io lv2-host-minimal](https://crates.io/crates/lv2-host-minimal)

**The project's own shell**
- nice-plug "started out as a fork of the awesome NIH-plug framework" and is now community-led. ISC license, crates.io owner BillyDM. 0.3.0 released 2026-08-17, latest 0.4.2 on 2026-09-14. Exports VST3 and CLAP. GUI via egui/iced/slint on baseview. Its docs do not mention plugin hosting or child-window embedding. — [docs.rs nice-plug](https://docs.rs/crate/nice-plug/0.3.0)

### Inferences
- The realistic stack is clack-host for a CLAP-only slot, with the hosted plugin in-process and its editor in a floating window. Everything else (VST3 via `vst3`/`vst3-host`, LV2 via livi) adds a second or third hosting implementation with weaker maturity on the developer's own platform (Linux). vst3-host has never opened an editor on Linux.
- The burst of 2026 hosting crates (truce-rack, plughost-formats, maolan, plugin_host) shows interest, but download counts in the hundreds mean they are unproven. Adopting one means depending on a crate likely to churn or die.
- No surveyed Rust crate is documented as hosting plugins *from inside another plugin*. All examples are standalone hosts (cpal apps or DAWs). The nested case (our plugin is both a CLAP plugin to REAPER and a CLAP host to the reverb) is unexplored territory in Rust.

### Gaps
- GitHub API access was blocked, so last-commit dates for clack, vst3-rs, rust-vst3-host, livi-rs and Carla could not be retrieved. Health is inferred from crates.io release dates only.
- Not verified: whether clack-host's examples implement the GUI extension (embedding versus floating) and the timer-support/posix-fd-support host extensions on Linux.
- Not verified: whether the `vst3` crate includes any host-side helpers beyond bindings, or how it handles the Linux `IRunLoop` interface.

## 3. Technical pitfalls of nesting a host inside a CLAP plugin

### Takeaway
The audio path is the easy part: calling the hosted plugin's `process()` from our own `process()` is legal in CLAP's symbolic-thread model. The hard parts are (1) relaying main-thread callbacks, timers and fd watches from the hosted plugin through our plugin to the DAW, which nice-plug likely does not expose; (2) GUI nesting on X11, where the editor should float rather than embed; (3) latency changes, which can only be reported via a DAW restart; (4) proxying automation; and (5) crash isolation, where in-process is the only cheap option and it means a third-party crash kills the DAW. Licensing is no longer a blocker: the VST3 SDK has been MIT since 3.8 (Oct 2025).

### Cited Findings
**Threading and main-thread obligations (CLAP 1.2.10, main branch header as fetched 2026-10-18)**
- The audio-thread is "symbolic". A host may run `process()` on different OS threads over time and "may mark any OS thread, including the main-thread as the audio-thread", as long as only one thread at a time acts as it. This permits calling the inner plugin's process from our process, and running everything on one thread in a CLI. — [CLAP thread-check.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h)
- The main-thread "will be the same OS thread throughout the lifetime of the plug-in. On macOS and Windows, this must be the thread on which gui and timer events are received (i.e., the main thread of the program)." — [CLAP thread-check.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h)
- `on_main_thread` is "Called by the host on the main thread in response to a previous call to: host->request_callback(host)". So when the inner plugin calls our `request_callback`, we must request a callback from the DAW and forward it. — [CLAP plugin.h](https://github.com/free-audio/clap/blob/main/include/clap/plugin.h)
- `activate`/`deactivate` are `[main-thread & !active]` / `[main-thread & active]`. "Once activated the latency and port configuration must remain constant, until deactivation." — [CLAP plugin.h](https://github.com/free-audio/clap/blob/main/include/clap/plugin.h)
- Linux event-loop hooks the inner plugin may require from us: `clap.timer-support` (`register_timer`, "30 Hz should be allowed", [main-thread]) and `clap.posix-fd-support` ("hook itself into the host select/poll/epoll/kqueue reactor", [main-thread]). — [CLAP timer-support.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/timer-support.h); [CLAP posix-fd-support.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/posix-fd-support.h)
- The spec explicitly anticipates wrappers: init()/deinit() "may be called multiple times in a process … If hosts load a plugin which itself wraps another CLAP … while also loading that same clap in its memory space, both the host and the wrapper will call init() and deinit()". — [CLAP entry.h](https://github.com/free-audio/clap/blob/main/include/clap/entry.h)

**GUI nesting**
- CLAP GUI supports embedding ("the plugin creates a window and embeds it into the host's window") or floating windows ("Floating window are sometimes the only option due to technical limitations"). X11 embedding uses XEmbed. For Wayland, "embed is currently not supported, use floating windows". Floating windows use `set_transient()`. Embedded windows use `set_parent()` plus resize negotiation (`request_resize`, `adjust_size`). — [CLAP gui.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/gui.h)
- Real-world nested-editor issues under REAPER on Linux: focus stealing from the host, REAPER's FX window intercepting drag-and-drop, HiDPI mismatch, misplacement after resize, and Xorg's 256-client limit. — [yabridge README](https://github.com/robbert-vdh/yabridge)
- The only Rust crate offering egui embedding of a hosted editor (vst3-host `EmbeddedEditor`) is macOS-only. Windows/Linux window code is "not yet runtime-verified". — [docs.rs vst3-host](https://docs.rs/crate/vst3-host/latest)
- nice-plug's GUI layer is baseview-based. Its docs do not mention exposing child windows for foreign content. — [docs.rs nice-plug](https://docs.rs/crate/nice-plug/0.3.0)

**Latency / PDC**
- CLAP: "The latency is only allowed to change during plugin->activate. If the plugin is activated, call host->request_restart()". `get()` is `[main-thread & (being-activated | active)]`. — [CLAP latency.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h)
- Chainers advertise PDC as a feature: PatchWork has "Built-in latency compensation ensures phase alignment across all sub-hosted software", and Metaplugin supports DAW PDC. — [KVR PatchWork](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio); [SOS Metaplugin](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)

**State**
- vst3-host exposes save_state/load_state blobs (in-process and isolated), which is the minimum needed to nest a hosted plugin's state inside ours. — [docs.rs vst3-host](https://docs.rs/crate/vst3-host/latest)
- Metaplugin-style automation needs a mapping list from DAW automation to hosted parameters. — [SOS Metaplugin](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)

**Crash isolation cost**
- In-process means a crash takes down the engine. Separate processes cost RAM/CPU ("This will require more computing resources, but that is the trade-off"). — [Bitwig user guide](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)
- Out-of-process bridging: "minimal performance overhead, but you may still notice an increase in latency spikes and overall DSP load". Grouping into one process trades away sandboxing for load time and CPU. — [yabridge README](https://github.com/robbert-vdh/yabridge)
- vst3-host's isolation spawns a helper process and offers `recover()`. Editors across the boundary work on macOS only. No overhead figures are published. — [docs.rs vst3-host](https://docs.rs/crate/vst3-host/latest)
- Carla's bridges (out-of-process) are still labelled experimental/WIP. — [GitHub falkTX/Carla](https://github.com/falkTX/Carla)

**Scanning paths**
- CLAP: Linux `~/.clap`, `/usr/lib/clap`. Windows `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`. macOS `/Library/Audio/Plug-Ins/CLAP`, `~/Library/Audio/Plug-Ins/CLAP`. Hosts "must" also honour `CLAP_PATH` and search recursively for `.clap`. — [CLAP entry.h](https://github.com/free-audio/clap/blob/main/include/clap/entry.h)
- VST3 on Linux: the user path `~/.vst3/` is used by vendors (u-he installs to `/home/USERNAME/.vst3/u-he/`). — [u-he Linux install paths](https://u-he.com/support/faq/compatibility/install-paths-linux/)

**Licensing**
- On 2025-10-29 Steinberg released VST 3.8, adding the MIT license for the VST 3 SDK, for commercial and non-commercial use with notice retention. The ASIO SDK went GPLv3 or proprietary. — [Sonic State (2025-10-30)](https://sonicstate.com/news/2025/10/30/vst-3-now-available-under-mit-license); [Steinberg press release PDF](https://ocl-steinberg-live.steinberg.net/_storage/asset/819253/storage/master/Press%20Release%20-%202025-10-29%20-%20VST%203.8%20-%20EN.pdf)
- Verified: the vst3sdk and vst3_pluginterfaces repos' LICENSE.txt read "MIT License, Copyright (c) 2026, Steinberg Media Technologies GmbH" (fetched 2026-10-18). — [vst3sdk LICENSE.txt](https://github.com/steinbergmedia/vst3sdk/blob/master/LICENSE.txt)
- Sonic State also says the SDK is available "under the existing licensing terms", which conflicts with the MIT announcement. The LICENSE file resolves this in favour of MIT. — [Sonic State](https://sonicstate.com/news/2025/10/30/vst-3-now-available-under-mit-license)
- The Rust hosting options are permissive: clack (MIT/Apache), vst3 (MIT/Apache), vst3-host (MIT), livi (MIT), maolan-plugin-host (BSD-2). Carla is GPL-2.0+, and Element is GPL. — [crates.io clack-host](https://crates.io/crates/clack-host); [crates.io vst3](https://crates.io/crates/vst3); [crates.io vst3-host](https://crates.io/crates/vst3-host); [crates.io livi](https://crates.io/crates/livi); [GitHub Carla](https://github.com/falkTX/Carla); [KVR Element](https://www.kvraudio.com/product/element-by-kushview)

### Inferences
- **Main-thread relay is the hidden cost.** The inner plugin's `request_callback`, timers (Linux GUIs almost always need them) and fd watches must be serviced on the DAW's main thread. Our plugin can only do that if the shell lets plugin code (a) call the DAW's `request_callback`/timer/fd extensions and (b) receive `on_main_thread`/`on_timer`/`on_fd`. NIH-plug-style frameworks hide the raw CLAP host. nice-plug probably needs a fork or a new API (unverified; see Gaps). A private thread cannot stand in for the main thread on macOS/Windows per the spec.
- **GUI:** the cheapest robust option is a *floating* window for the hosted editor (`create(api, is_floating=true)` plus `set_transient` to our window), opened from a button in the egui UI. Embedding into a baseview/egui window on X11 means creating an X11 child window, handing its XID to the inner plugin, and managing focus, resize, DPI and REAPER DnD quirks. vst3-host only reached embedding on macOS. Plan for floating windows on all OSes in v1.
- **Latency:** our reported latency must equal the hosted plugin's latency, plus ours, plus any internal dry-path alignment (for parallel dry/wet we must delay the dry path to match). Loading or swapping a plugin with different latency forces `request_restart`, a deactivate/reactivate cycle that causes an audible glitch, and REAPER re-evaluates PDC. Many algorithmic reverbs report 0 latency, but convolution and linear-phase effects do not.
- **State:** our CLAP state blob should carry a versioned envelope: format (clap/vst3), plugin ID, plugin version, path hint, the inner state bytes, and slot bypass/mix. On load, a missing plugin must degrade gracefully: bypass the slot and keep the bytes so re-saving does not destroy them.
- **Automation:** CLAP parameter lists are expected to be stable, so we cannot expose the inner plugin's parameters directly. The workable pattern is Metaplugin's: N fixed "FX macro" parameters mapped to chosen inner parameters.
- **Sample-rate/block-size:** inner `activate(sr, min, max)` must mirror ours. If the engine oversamples internally or uses fixed sub-blocks, the slot must run at DAW rate after decimation and accept variable block sizes. Our `activate` (main thread) must activate the inner plugin, and our `start_processing` (audio thread) must start it.
- **Self-loading and recursion:** block loading our own synth (or another wrapper of it) into the slot. Expect the same .clap DSO to be loaded both by REAPER and by us, which the spec says must be tolerated.
- **Out-of-process:** this needs a helper binary per OS, shared-memory audio buffers, and a cross-process wake per audio block, a direct cost in latency spikes and CPU (per yabridge and Bitwig). It also requires cross-process GUI parenting, which even vst3-host only has on macOS. It is not realistic for a solo/small team in v1.

### Gaps
- Not verified: whether nice-plug exposes the CLAP host's `request_callback`, timer-support or posix-fd-support to plugin code, or lets a plugin add custom CLAP extensions. This determines whether hosting is feasible without forking the shell.
- Not verified from a primary source: VST3's Linux `Steinberg::Linux::IRunLoop` requirement (the host must provide a run loop for plugin timers/fds via IPlugFrame). This would add a second relay path if VST3 is supported. Steinberg's dev-portal locations page returned 403 to curl, so system-wide VST3 paths (Linux `/usr/lib/vst3`, Windows `C:\Program Files\Common Files\VST3`, macOS `/Library/Audio/Plug-Ins/VST3`) are from memory, unverified this session.
- No measured numbers found for per-block IPC overhead of out-of-process hosting.
- LV2 standard search paths (`~/.lv2`, `/usr/lib/lv2`, `LV2_PATH`) were not verified from lv2plug.in.
- Whether REAPER's CLAP host implements timer-support and posix-fd-support (needed to relay inner Linux GUIs) was not checked.

## 4. Determinism, scoring and shared presets

### Takeaway
A hosted third-party plugin breaks all three guarantees: bit-exact reproducibility (many effects are deliberately non-deterministic), CLI rendering (the CLI must become a plugin host and the user must own the plugin), and preset sharing (recipients may lack the plugin). Products in this space handle it either by closing the ecosystem (Kilohearts) or by restricting factory content to bundled plugins (Unify). Scoring should therefore treat any third-party slot as outside the deterministic signal path.

### Cited Findings
- Renders fail to null because "some plug-ins behave differently on each playthrough … the modulation may not trigger from the start of its cycle each time you hit Play." Random sample selection is another cause, so it is "simply a matter of the plug-in doing slightly different things each time you hit play." (Matt Houghton, Sound On Sound, April 2025.) — [SOS: Why aren't my rendered and live plug-in sounds nulling?](https://www.soundonsound.com/sound-advice/q-why-arent-my-rendered-and-live-plugin-sounds-nulling)
- Concrete case: VSL's MIRacle offline exports are "never precisely the same twice". The reply: "modulation introduces pseudo-random modulation that won't get replicated on repeat plays/exports." No seed or deterministic option is mentioned. — [VSL forum](https://forum.vsl.co.at/topic/59562/miracle-reverb-never-the-same-twice)
- Kilohearts: presets referencing unowned Snapins still load. Macros work, but unlicensed effect parameters are locked. This is possible only because Kilohearts controls every module. — [Kilohearts blog](https://kilohearts.com/blog/wtf_is_a_snap_heap)
- Unify: factory patches "use ONLY the bundled and built-in plug-ins". Users warned that third-party dependencies create "very segmented user niches". — [KVR Unify thread](https://www.kvraudio.com/forum/viewtopic.php?p=7632550)
- CLAP's thread model allows a host to run everything (main and audio) on one thread, so a headless CLI could in principle host CLAP plugins single-threaded. The spec also ties real-time constraints to the `render` extension (offline versus realtime mode). — [CLAP thread-check.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h)

### Inferences
- **Scoring:** compute scores on the synth's own output *before* any third-party slot (a "dry tap"). Never score through a hosted plugin. If the built-in reverb is deterministic (seeded RNG, no time or thread dependence), it can sit inside the scored path.
- **CLI renderer:** the options are (1) skip or bypass third-party slots with a warning, which is the recommended default; (2) fail hard when a slot is present and `--strict`; or (3) optionally host CLAP plugins headless. Option 3 still won't be bit-exact for modulated reverbs, and some plugins misbehave without a GUI or event loop or need license checks (unverified).
- **Patch format:** mark patches containing a third-party slot as `reproducible: false`. Keep the synth's own patch data loadable without the slot, so the slot is a removable post-stage. When the plugin is missing, show "missing: ‹vendor/name/version›" and keep the opaque state bytes so re-saving doesn't destroy them.
- **Determinism caveats even with the same plugin owned:** the plugin version, block size, sample rate, CPU features (SIMD paths) and internal threading can all change output. Bit-exactness across machines is not achievable for arbitrary third-party code.

### Gaps
- No source found on how Unify, PatchWork or Metaplugin present missing plugins when loading a shared patch.
- No data found on what share of popular reverbs are deterministic (seeded) versus time-randomized.
- Not verified: which commercial plugins refuse to process or behave differently when no GUI or event loop is running (relevant to a headless CLI host).

## 5. The simpler alternatives: DAW-side reverb or built-in reverb. What do users say?

### Takeaway
User opinion is split but leans "both". Many want designer-voiced built-in FX for presets to sound as intended. Others want easy bypass, ideally a global or persistent "FX off" switch, so they can use their own reverb in the DAW. Nobody in the sources demands third-party hosting inside a synth; the common request is a global FX bypass.

### Cited Findings
- VI-CONTROL "To drench or not to drench, fx in synth presets" (started 2022-01-24 by emptyvessel, a preset maker):
  - For built-in FX: el-bo said "I'd much rather trust the designer when it comes to how best to use them". AudioLoco argued that without FX "the patch sounds lifeless and not what the preset designer intended". Nico5 valued "more end-to-end sound designs". — [VI-CONTROL thread](https://vi-control.net/community/threads/to-drench-or-not-to-drench-fx-in-synth-presets-discussion.120519/)
  - For bypass and own FX: kenose said "It takes like half a second to turn it off", and jcrosby noted that external effects can replace weak built-ins. — [VI-CONTROL thread](https://vi-control.net/community/threads/to-drench-or-not-to-drench-fx-in-synth-presets-discussion.120519/)
  - Feature requests: Paul@Sydney wished for "a global control for turning preset time based FX on and off". babylonwaves noted the Access Virus and Hive 2 already have one. el-bo cited Repro-1/5's all-FX bypass that persists across presets. — [VI-CONTROL thread](https://vi-control.net/community/threads/to-drench-or-not-to-drench-fx-in-synth-presets-discussion.120519/)
  - Against heavy FX: ghobii said "heavy handed use of reverb and delay tends to mask the interesting character of the sound". md11 called Falcon presets "way over processed for my liking". timprebble (sound designer) noted that disabling FX sometimes leaves "nothing much at all". — [VI-CONTROL thread](https://vi-control.net/community/threads/to-drench-or-not-to-drench-fx-in-synth-presets-discussion.120519/)
- Unify's bundling strategy (built-in effects plus bundled freeware) is a vendor choosing built-ins for portability. — [BPB review](https://bedroomproducersblog.com/?p=34815)

### Inferences
- A built-in reverb with a persistent global "FX bypass / dry output" switch covers both camps. Users who prefer their own reverb put it on the REAPER track after the synth with zero integration cost and full DAW PDC, automation, crash isolation (in Bitwig-style hosts) and GUI handling.
- The DAW FX chain already provides everything a hosting slot would, except per-voice or pre-mix placement and storing the reverb inside the synth preset. Hosting is only uniquely valuable if the reverb must sit *inside* the synth's signal flow (per-layer or modulated by synth envelopes/LFOs) or travel with the preset.

### Gaps
- KVR's "reverbs inside synths" thread (forum post IDs around 8717332–8717620) could not be fetched (fetch errors). Gearspace "the slathering" thread not read. Opinion coverage is therefore one forum thread plus vendor signals.
- No Reddit sources were retrieved.

## 6. Options ranked by effort and risk

### Takeaway
Ranking from lowest to highest effort and risk: (b) DAW-side reverb < (a) built-in deterministic reverb < (d) hybrid (built-in now, optional CLAP-only floating-window slot later, excluded from scoring) < (c) a full in-plugin third-party hosting slot across CLAP/VST3/LV2 on three OSes. Recommendation: ship (a)+(b) now, meaning a built-in reverb with a global FX bypass, and document using a DAW reverb. Treat (d) as a gated experiment, not a roadmap commitment.

### Cited Findings
- Hosting maturity on the developer's platform is weak. vst3-host has not run a plugin or opened an editor on Linux or Windows. Its embedded egui editor is macOS-only. Its lock-free path is not RT-audited. — [docs.rs vst3-host](https://docs.rs/crate/vst3-host/latest)
- clack-host is usable (MIT/Apache, 0.2.0 on 2026-09-12, CPAL host example) but warns of breaking changes and incomplete docs. — [GitHub prokopyl/clack](https://github.com/prokopyl/clack); [crates.io clack-host](https://crates.io/crates/clack-host)
- LV2 crates last released Nov 2024. — [crates.io livi](https://crates.io/crates/livi); [crates.io lilv](https://crates.io/crates/lilv)
- Crash isolation is a resource trade-off even for a DAW vendor. — [Bitwig user guide](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)
- Commercial chainers treat PDC, automation mapping and multi-format support as core features: PatchWork at $99, Metaplugin with an automation mapping list. — [KVR PatchWork](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio); [SOS Metaplugin](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)

### Inferences
Effort figures are my estimates for one experienced Rust audio developer. They are not sourced.

| Rank | Option | Effort (estimate) | Determinism / scoring | Main risks |
|---|---|---|---|---|
| 1 | **(b) DAW-side reverb** (document it; add a global FX-bypass or "dry out" switch) | ~0.5–2 days | Unaffected: the synth output stays deterministic | Presets don't carry the reverb sound, and the CLI renders dry. Low risk. |
| 2 | **(a) Built-in reverb** (e.g. FDN/Dattorro/plate, seeded modulation, block-size-independent) | ~1–3 weeks for a good one, plus tuning | Fully deterministic if RNG is seeded and processing is sample-exact independent of block size; can be inside the scored path | Quality bar versus commercial reverbs (users may bypass it anyway). CPU cost per instance. Low technical risk. |
| 3 | **(d) Hybrid**: (a)+(b) now, later an optional **CLAP-only, in-process, floating-editor** post-FX slot via clack-host, flagged non-reproducible and excluded from scoring and CLI by default | (a) + ~4–8 weeks for a usable CLAP slot (scan `CLAP_PATH`/std dirs, load/activate mirroring, state envelope, latency plus restart, N macro params, floating editor, main-thread/timer/fd relay) + likely nice-plug fork work | The scored "dry tap" stays deterministic. Slot output is marked non-reproducible. The CLI bypasses the slot by default. | Main-thread relay may require forking nice-plug. Linux GUI timers/fds. REAPER restart glitches on latency change. Third-party crashes blamed on us. Ongoing support of "plugin X doesn't work in your synth". |
| 4 | **(c) Full third-party hosting** (CLAP + VST3 [+ LV2], embedded editors in egui, out-of-process isolation, three OSes) | ~3–6+ months initial, then continuous maintenance | Breaks reproducibility for any patch using it. The CLI must become a full host. Scoring must exclude it. | Immature Rust crates (vst3-host Linux/Windows unverified; LV2 crates stale). X11/Win32/Cocoa embedding quirks. Out-of-process IPC/GUI parenting. A per-OS scanning UI. Essentially building a PatchWork-class product inside a synth. High risk. |

- Decision heuristics:
  - Choose (c) or (d) only if users concretely need an effect *inside* the synth's voice or mod path, or bundled into presets. Otherwise the DAW chain already provides hosting, PDC, automation and isolation for free.
  - If (d) proceeds, build it as a separate, feature-flagged post-stage after the scoring tap.
  - Make missing-plugin load paths non-destructive.
  - Keep VST3 and LV2 out of scope until CLAP is stable on all three OSes.

### Gaps
- No public effort data from developers who added hosting slots to a synth was found. The estimates above are unvalidated.
- Not verified: whether nice-plug or REAPER provide everything needed for the main-thread/timer relay on Linux. A one-day spike (a clack-host CLAP inside a nice-plug CLAP, floating editor, in REAPER on X11) would settle the feasibility of (d).
