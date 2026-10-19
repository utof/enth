# Research and evidence

**As of 2026-10-18; §3, §4 and §10 updated 2026-10-19 from the Phase 0 research.** This is the evidence behind the decisions. Claims are marked *measured* (benchmarks, papers) or *anecdotal* (forum reports) where it matters. Re-check versions before relying on them — the Rust audio ecosystem moves fast.

## 1. Products and projects to learn from

Nobody ships all three pillars in one open product. Each piece exists separately.

| Project | What it is | What to take |
| --- | --- | --- |
| [Phase Plant](https://kilohearts.com/docs/phase_plant) (Kilohearts) | Semi-modular synth: add generators, modulators and effects; three effect lanes; 8 macros | The structured-modularity model; macros + [64 automation slots](https://kilohearts.com/docs/host_plugins) for host automation |
| [Syntorial](https://www.thomann.co.uk/audible_genius_syntorial_2.htm) (Audible Genius) | 190+ lessons, ~145 challenges on its 64-parameter synth "Primer"; a randomiser for endless patches | Progressive disclosure, hidden-patch challenges, the randomiser. Its ceiling is Primer itself — the gap we close |
| [Synplant 2](https://cdm.link/the-road-to-synplant-2/) (Sonic Charge) | "Genopatch" ML finds its own engine's parameters for a sample, offering several candidates; local | Several candidates per sound; train on your own engine. Its author describes ~10 years of ML iteration — plan it late |
| [Synth Basics sound-matching game](https://synthbasics.com/sound-match) | Browser game: hidden patch, warmer/colder meter, a coach naming the knob furthest off | The core game loop; difficulty = number of free knobs |
| [nice-plug](https://codeberg.org/RustAudio/nice-plug) (ISC) | Community successor to nih-plug: CLAP + VST3 + standalone, poly-mod, egui/iced/vizia adapters | Our starting plugin shell ([ADR-0001](adr/0001-plugin-shell.md)) |
| [clack](https://github.com/prokopyl/clack) | Safe, low-level Rust wrappers for the whole CLAP API, plugins and hosts | Escape hatch for the shell; `clack-host` for our test host |
| [HexoDSP / HexoSynth](https://github.com/WeirdConstructor/HexoSynth) | Rust modular synth with a runtime-changeable DSP graph | Ideas for graph rebuilds and per-module tests. Its DSP crate `synfx-dsp` is GPL-3.0-or-later: reference only |
| [Firewheel](https://github.com/BillyDM/firewheel) (MIT/Apache) | General Rust audio graph engine | Scheduling and silence-skipping ideas; not a dependency |
| [web-synth](https://github.com/Ameobea/web-synth) | Browser DAW running Rust DSP as WebAssembly in an AudioWorklet | Evidence the "same engine in the browser" path works |
| [torchsynth](https://github.com/torchsynth/torchsynth) | GPU modular synth for ML, 16,200× realtime; dataset of parameter-paired sounds | Dataset design; render throughput is the real ML constraint |
| [vital-mcp](https://github.com/mard4/vital-mcp) | Agent tools for Vital presets; renders and returns centroid, band energy, attack, sustain, tail | Prior art for descriptor feedback to LLMs |
| Surge XT, Vital, Dexed, Odin 2 | Excellent open synths, all GPL | Research benchmarks only. **Never copy their code** into this MIT project |

## 2. Sound → patch research

Three findings shape the design: the inverse problem has many valid answers; no single audio metric is best; gradient-free search is a strong ML-free baseline.

| Work | Core idea | What it means for us |
| --- | --- | --- |
| [Hayes, Saitis, Fazekas — ISMIR 2025](https://arxiv.org/abs/2506.07199) (best student paper) | Sound → params is ill-posed mostly because of synth symmetries (e.g. swapping identical oscillators). A permutation-equivariant flow-matching model beats regression on Surge XT | Expect several right answers; canonicalise over symmetries; models should propose several candidates |
| [Hayes — Sep 2026](https://arxiv.org/abs/2609.29320) | Model audio and parameters jointly; unlabelled real recordings train the audio side; better inversion of real-world sounds (Surge XT, Dexed) | Training data should include real recordings |
| [Braun & Finkelstein — DAFx 2026](https://www.dafx.de/paper-archive/2026/papers/DAFx26_paper_33.pdf) | A graph network mirroring FM routing generalises to unseen routings; flat parameter vectors don't. Joint audio–preset embeddings enable retrieval | Encode modular patches as graphs; preset retrieval is a cheap first "sound → patch" |
| [EASY-AS-PIE — Braun, 2025](https://lists.cs.princeton.edu/hyperkitty/list/talks@lists.cs.princeton.edu/thread/WUWLI7Z4YGUH6CXODQEGJ33W2CQFBAHT/) | Evolutionary search over synth architecture + parameters, scored by audio embeddings; mutations sample from real-preset distributions | Template for topology search later |
| [INSTRUMENTAL — Bogdan, Mar 2026](https://arxiv.org/abs/2603.15905) | 28-param subtractive synth + CMA-ES + composite loss (mel-STFT, centroid, MFCC). CMA-ES beat gradient descent; spectral init sped convergence | Our Phase 0 metric spike and permanent baseline |
| [Salimi et al. — 2025](https://arxiv.org/html/2506.22628v2), [2026](https://arxiv.org/abs/2608.27698) | Best loss depends on synthesis method. "Partial parameter distance" compares only critical params; agreed with blind listening in 5 of 7 scenarios | Composite, versioned, per-challenge metric weights; partial distance for knob feedback |
| [Perceptual-neural-physical matching — 2023](https://arxiv.org/pdf/2301.02886) | Weight parameter errors by how much each changes the sound | Sensitivity-weighted knob distance |
| [Neural proxies — Combes et al., JAES 2025](https://arxiv.org/abs/2509.07635) | A network maps presets to an audio-embedding space, so non-differentiable synths join training | If gradients are ever needed, do this instead of a PyTorch copy of the engine |
| [SynthRL — IJCAI 2025](https://www.ijcai.org/proceedings/2025/1129) | RL with a similarity reward fine-tunes matching on out-of-domain sounds | Later fine-tuning for real-world samples |
| [DiffMoog — 2024](https://cris.openu.ac.il/en/publications/diffmoog-a-differentiable-modular-synthesizer-for-sound-matching/) | Differentiable modular synth; later work reports its FM chains failed to converge | Don't build a differentiable twin of the engine |
| [CTAG — ICML 2024](https://github.com/PapayaResearch/ctag) | Text → sound by evolving a 78-param modular synth toward a text–audio embedding | "Describe a sound, get a patch" is the same solver with a different objective |

Name collision to remember: "CLAP" in ML papers is the audio–text embedding model, unrelated to the CLAP plugin format.

## 3. Rust plugin ecosystem (status)

- **nih-plug is in maintenance mode**; its README points framework users to a community fork, now **nice-plug** on Codeberg (ISC). nice-plug describes itself as experimental with recent large changes ([README](https://codeberg.org/RustAudio/nice-plug)).
- **VST3 SDK became MIT** with version 3.8 in Oct 2025 ([Steinberg](https://forums.steinberg.net/t/vst-3-sdk-license/201637)). The MIT/Apache `vst3` crate 0.3.0 now ships pre-generated bindings ([Micah Johnston, Dec 2025](https://micahrj.github.io/posts/vst3)). Original nih-plug still uses the GPLv3 `vst3-sys` (an [open PR](https://github.com/robbert-vdh/nih-plug/pull/263) switches it); a `cargo tree` of nice-plug 0.4.2 on 2026-10-18 shows it depends on the MIT/Apache `vst3` 0.3.0 crate, not `vst3-sys` (effects research session) — re-check at the version Phase 0 pins.
- **clack-host** has been on crates.io since 2026-05-03 (0.2.0 on 2026-09-12): usable for our test host.
- **clap-wrapper** wraps a CLAP as VST3/AUv2 (and AAX/AUv3 in recent versions); a Rust crate builds self-contained bundles ([lib.rs](https://lib.rs/crates/clap-wrapper)). *Measured 2026-10-19:* the crates.io `clap-wrapper-rs` 0.3.1 bundles clap-wrapper 0.14.0 and lacks the 0.16.0 fix for VST3 parameter sync (a reloaded plugin showed a stale parameter in REAPER), and 0.16.0 is not on crates.io yet. clack's own polysynth example fails 3 clap-validator state tests. That is why clack + clap-wrapper stays the escape hatch, not the plan.
- **Hosts on Linux.** *Measured 2026-10-19:* the Carla 2.5.8 that Ubuntu ships hosts **VST3 only**. CLAP support exists only on Carla's unreleased main branch (it is absent at v2.5.8 and v2.5.10). Ardour hosts no CLAP either. Bitwig is the only host with CLAP poly-mod and it is paid (trial). Native REAPER 7.82 hosts CLAP and VST3 and can run headless under Xvfb.
- **egui-baseview** was archived in Apr 2026 and now lives at `RustAudio/egui-baseview` on Codeberg, the basis of nice-plug's egui adapter (0.9.0, 2026-10-17). **vizia-plug** was last pushed 2026-06-13 and vizia 2026-08-21; the open vizia-plug issue #41 is unanswered, so it is not a challenger.

## 4. UI: WebView vs native

**No public benchmark compares WebView plugin UIs with native ones** (as of Oct 2026). The evidence below is anecdotal or indirect; Phase 0 replaces it with our own numbers.

| Evidence | Value | Source | Type |
| --- | --- | --- | --- |
| JUCE 8 WebView2 (Windows): shared browser process | ~100 MB for the manager hosting all instances | [JUCE forum, May 2024](https://forum.juce.com/t/webview-vs-juce-graphics/61156) | Anecdotal |
| Same: per instance | Peaks up to +80 MB, settles ~35 MB | same | Anecdotal |
| JUCE WebView leak | ~37 MB per test run (fixed upstream) | [JUCE forum](https://forum.juce.com/t/webview-nativeaction-memory-leak/64687) | Anecdotal |
| JUCE WebView keyboard | WebView eats the spacebar, so it never reaches the host | [JUCE forum](https://forum.juce.com/t/webviews-eating-spacebar/61941) | Anecdotal |
| WRAC template (WebView + Rust + CLAP) | "DAW playback dropout measurements remain unverified" | [PR #169](https://github.com/novonotes/wrac-plugin-template/pull/169) | Statement |
| wry on Linux | Window embedding is X11-only; you must init and pump GTK yourself | [wry docs](https://docs.rs/wry/latest/wry/struct.WebView.html) | Docs |
| GTK in a host process | CLAP co-author argues Linux plugins shouldn't use GTK or Qt because toolkits mix badly | [gist](https://gist.github.com/abique/4c1b9b40f3413f0df1591d2a7c760db4) | Expert opinion |
| WebKitGTK + NVIDIA proprietary driver | 2026 crash reports; env-var workarounds didn't help in one case | [magpie #1740](https://github.com/yetone/magpie/issues/1740), [tdesktop #31441](https://github.com/telegramdesktop/tdesktop/issues/31441) | Bug reports |
| nice-plug-egui 0.6.0 + egui-baseview 0.9.0 (the old nih-plug adapter "for now, just always redraw"s; this replaces that) | Repaints reactively: **0 frames while idle**, about 26 frames/s while host automation plays; 280 of 280 opens drew in REAPER under Xvfb; an open idle editor costs +0.2 to +1.1 points of one core, 64 wake-ups/s on the GUI thread | Phase 0 shell and UI research, 2026-10-19 (old behaviour: [source](https://nih-plug.robbertvanderhelm.nl/src/nih_plug_egui/editor.rs.html)) | Measured (Xvfb, software GL) |
| baseview 0.4.1 (what every Rust toolkit adapter sits on) | Leaks ≈70 KB per open/close cycle (≈115–160 KB with egui on Mesa; REAPER's own ReaEQ is flat); frame pacing tops out near 30 fps; REAPER sizes the FX window for scale 1 while baseview draws at scale 2, so a HiDPI editor is clipped | Phase 0 UI research, 2026-10-19 | Measured (Xvfb, software GL) |
| nice-plug-iced 0.5.0 | The `gain_iced` editor spins one core at ≈100% while open and idle (10.6% when closed) and leaks one thread per open/close | Phase 0 UI research, 2026-10-19 | Measured (Xvfb, software GL) |
| egui | "1–2 ms per frame" for typical UIs | [egui README](https://github.com/emilk/egui) | Author estimate |
| Third-party WRAC user | Kept the WRAC CLAP shell but replaced the UI with a wgpu panel ("no webview, no toolkit") | [open-pitch-quant](https://github.com/samuelmarquis/open-pitch-quant) | Project choice |

Native options considered: **egui** (immediate mode, custom painting easy, largest agent training corpus, also builds for the web), **vizia** (retained/reactive, CSS-like styling, accessibility via AccessKit), **Slint** (declarative, software renderer; plugin path immature), **iced** (adapter stale; the measured idle spin and thread leak above rule it out), **raw wgpu** (best ceiling, most work). Decision: [ADR-0002](adr/0002-ui-toolkit.md).

Linux desktop note: Linux Mint's Cinnamon Wayland session is announced as supported but not default, arriving with Mint 23 around Christmas 2026 ([It's FOSS](https://itsfoss.com/news/linux-mint-wayland-stable/)). Re-run the host checklist under Wayland/XWayland then.

## 5. Can LLMs hear?

- **SonicBench** (Jan 2026) tests 12 physical attributes (pitch, brightness, loudness, timbre…) with differences well above human thresholds: "most models perform near random guessing", and explicit reasoning barely helps — yet the frozen audio encoders do capture the cues ([arXiv 2601.11039](https://arxiv.org/html/2601.11039)). *Measured.*
- **MECAT** reports audio models degrade sharply on tasks about intrinsic acoustic properties ([arXiv 2507.23511](https://arxiv.org/pdf/2507.23511)). *Measured.*
- **LLM2Fx** (WASPAA 2025): text LLMs predict EQ/reverb parameters zero-shot, and do better when DSP features are injected into the prompt ([code](https://github.com/SonyResearch/LLM2Fx)). *Measured.*

Conclusion: give text LLMs compact numeric descriptors and measured hints, not raw audio. Keep the descriptor interface stable; audio-native models may catch up later and can be added as an extra signal.

## 6. Agents: CLI vs MCP

- Scalekit reports 75 runs on GitHub tasks with Claude Sonnet 4: on the simplest task the CLI used 1,365 tokens vs 44,026 for the MCP server (32×), and succeeded 25/25 vs 18/25 ([Scalekit](https://scalekit.com/blog/mcp-vs-cli-use); summary by [Firecrawl](https://www.firecrawl.dev/blog/mcp-vs-cli)). *Vendor benchmark.*
- Mario Zechner's benchmark found "MCP vs CLI truly is a wash" for a well-designed tool ([Aug 2025](https://mariozechner.at/posts/2025-08-15-mcp-vs-cli/)). *Blogger benchmark.*
- Anthropic's guidance: few, high-impact tools; meaningful, token-efficient outputs; clear descriptions; evaluate with real agent runs ([Writing effective tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)).

Conclusion: CLI first with compact JSON and a one-page `enth guide`; a thin MCP shim later only if a chat client needs it ([ADR-0003](adr/0003-agent-cli.md)).

## 7. Naming checks done so far

See [naming.md](naming.md) for candidates and what was checked.

## 8. Reusing effects and hosting plugins

Research session `_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/` (full reports there).

**Finding: port permissively licensed effects into our own deterministic modules; don't host third-party plugins inside the synth; your own reverb goes after enth on the REAPER track.** Status: adopted by the owner on 2026-10-19 and recorded in [ADR-0006](adr/0006-effects-strategy.md) (*Proposed* until Gate G1). The reverb pick is provisional until a wasm32 build check and a blind, level-matched listening test.

| Need | Recommended | Licence | Fallback |
| --- | --- | --- | --- |
| Reverb | Port Signalsmith Basics `reverb.h` (~409 lines, FDN + diffuser, real-unit parameters, seeded randomness, active 2026-09-30) | MIT | fundsp 0.23 `reverb2_stereo`/`reverb4_stereo` (MIT/Apache), no port needed |
| Compressor | Write in-house (a few hundred lines) after Julius O. Smith's Faust `compressor_mono`, cross-checked against web-audio-api-rs; no look-ahead, so zero latency | Algorithm STK-4.3 (MIT-style) / MIT | Signalsmith `dynamics.h` once it matures (started 2026-09-25) |
| "Use my own reverb" | A global FX-bypass switch (outside the patch, ignored by scoring) + put your reverb after enth in REAPER | — | — |
| Filter, oscillators | In-house TPT state-variable filter; polyBLEP after Mutable Instruments stmlib / DaisySP | Own / MIT | — |
| Later (S2) | CloudSeedCore port for a "lush" reverb (credit Ghost Note Audio, no "Cloud Seed" name); chorus/phaser from DaisySP core or Signalsmith; saturation from Faust `aanl.lib` | MIT | — |

- **No maintained, permissive Rust compressor exists**; healthy Rust crates cover FFT, resampling, filters and I/O, while effects are usually written per project.
- **Hidden randomness matters more than licences:** CloudSeedCore, Airwindows and the `basic-reverb` crate use unseeded global RNGs; any port must use the engine's seeded PRNG, flush denormals in code, and allocate only in `prepare`.
- **Licence traps in repos that look permissive:** JUCE modules are AGPLv3/commercial since JUCE 9; DaisySP's compressor and ReverbSc are in its LGPL submodule; chowdsp's DSP modules are GPLv3; Faust declares licences per function (several reverbs and compressors are GPL-3.0-only); Signalsmith's 2021 reverb example has no licence (use `basics` instead).
- **Hosting:** `clack-host` (on crates.io since May 2026) makes a CLAP-only slot technically possible, estimated at 4–8 weeks plus likely shell work. It would break deterministic renders, the CLI, preset sharing and seed-based duels, and adds nothing over the DAW's own FX chain. Users of other synths ask for a global FX bypass, not hosting.

Full reasoning, sources, corrections and 17 open checks: `summary.md` in the session folder.

## 9. Considered and rejected

| Option | Why not |
| --- | --- |
| plugdata / Pure Data as the engine or a prototyping layer | GPL; a second runtime beside our Rust engine; breaks "one engine, one truth" |
| WebView UI | See §4 and [ADR-0002](adr/0002-ui-toolkit.md) |
| Server-side scoring or AI | Local-first principle; determinism makes serverless duels possible |
| Hosting third-party plugins inside the synth | 4–8 weeks for a CLAP-only slot; breaks deterministic renders, the CLI, preset sharing and seed-based duels; the DAW's FX chain already does it (§8, [ADR-0006](adr/0006-effects-strategy.md)) |
| `cloudseedcore-rs` (C++ via `cxx`) | Needs a C++ toolchain and can't build for wasm32; port CloudSeedCore to Rust instead (S2) |

## 10. Watch list

- **baseview [#357](https://github.com/RustAudio/baseview/pull/357)** (open): moves X11 events onto the host's main thread. Expect nice-plug-egui 0.7 with a host timer/fd API (about a day to adopt), and re-check the per-thread CPU attribution, the 30 fps cap and the leak when it lands.
- **NVIDIA 595-series X driver, bug 6205955** (no fix as of 2026-10-19): a SIGSEGV at `nvidia_drv.so +0x3fc5a` during X resource destruction, reported with short-lived X clients on Linux Mint 22.3 with `prime-select nvidia` ([forum thread 370339](https://forums.developer.nvidia.com/t/xorg-sigsegv-in-nvidia-drv-so-during-freepixmap-driver-595-71-05-open-rtx-5080-repeatable/370339)). Opening and closing plugin editors churns exactly those resources and crashed Xorg on the development machine on 2026-10-19. Hence the Xvfb rule for agents. Re-check after each driver update.
- **Carla CLAP support** reaching a release; until then Carla is a VST3-only second host.
- **Real-time generative audio models** (e.g. Google's Magenta line) — possible future modules for sound generation or sound → patch seeding; not dependencies.
- **Audio-native LLMs** — they can't reliably judge timbre today (§5); re-check yearly.
- **Signalsmith `dynamics.h`** (MIT, history starts 2026-09-25) — compressor candidate once it has a few months of history (§8).
- **clack-host and vst3-host on Linux** — re-check before any hosting spike; vst3-host had been exercised on macOS only as of Oct 2026 (§8).
