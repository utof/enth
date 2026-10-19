# Reusable effects and general DSP libraries for an MIT-licensed Rust synth plugin (reverb excluded)

Method note (read first). All figures below were collected on 2026-10-18.
- **crates.io data** (version, release date, total downloads, "recent" = last-90-day downloads, licence field, reverse-dependency count, owners) comes from the crates.io JSON API.
- **Commit activity** ("last commit", "commits since 2025-10-18", "distinct authors") was measured from treeless `git clone`s of each repo.
- **Licences** were read from the LICENSE/COPYING files or source headers in those clones.

The GitHub REST API was blocked from this environment, so **star counts and open-issue counts are missing** for every repo (see Gaps). Codeberg was also unreachable except through crates.io/docs.rs mirrors.

**Build checks.** Local `cargo check` runs used rustc 1.97.0 (stable, 2026-07-07). The wasm32 std component could not be downloaded (static.rust-lang.org blocked), so **no crate was actually compiled for wasm32**. The wasm assessments are inferences from `cargo tree --target wasm32-unknown-unknown` dependency trees plus `no_std` status.

**Licence policy applied.**
- **Reusable:** MIT, Apache-2.0, ISC, BSD, 0BSD, Zlib, Unlicense/CC0, and STK-4.3 (MIT-style; see below).
- **Reference-only:** GPL, LGPL and AGPL. This document marks them **REFERENCE-ONLY**.

---

## Q1. Which general Rust DSP toolkits are healthy in 2026?

### Takeaway
**Healthy and reusable (permissive) as of Oct 2026:**
- **Toolkits:** fundsp (MIT/Apache, broadest effect set, no_std+alloc; last commit 2026-03-03), biquad and idsp (no_std filters).
- **FFT and resampling:** realfft/rustfft (mature FFT), rubato and audioadapter (very active resampling/buffer adapters).
- **Graph engine:** Firewheel (very active; few effects).
- **Audio I/O:** cpal.
- **Plugin framework:** nice-plug (ISC, the new community successor to nih-plug, but self-described "experimental").

**Stale, or not reusable under the licence policy:**
- dasp: no release since 2020-05-29.
- valib: MIT library code, but last commit 2024-12-24 and not on crates.io. The crates.io name `valib` belongs to an unrelated crate.
- synfx-dsp: GPL-3.0-or-later, needs nightly, and its GitHub repo is no longer publicly reachable.

### Cited Findings

**Toolkits / effect libraries**

**fundsp**
- **Release and licence:** 0.23.0, released 2026-01-07; licence `MIT OR Apache-2.0`. 232,620 total downloads, 52,990 in the last 90 days, 29 reverse dependencies. — [crates.io/fundsp](https://crates.io/crates/fundsp)
- **Activity:** last commit 2026-03-03. 52 commits from 4 authors since 2025-10-18. — [GitHub commits](https://github.com/SamiPerttu/fundsp/commits/HEAD)
- **Unreleased work:** CHANGES.md lists an unreleased "Version 0.24 (Next Version)" with Sequencer looping and a graph introspection module. Since 2026-03 the repo has been quiet (7 months, which is moderate staleness). — [CHANGES.md](https://github.com/SamiPerttu/fundsp/blob/HEAD/CHANGES.md)
- **Licence files:** LICENSE-MIT and LICENSE-APACHE are both present at the repo root. — [LICENSE-MIT](https://github.com/SamiPerttu/fundsp/blob/HEAD/LICENSE-MIT)
- **no_std:** the README says "FunDSP supports `no_std` environments… disable the feature `std`… The `alloc` crate is still needed for components that allocate memory." — [README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
- **Real-time safety:**
  - The README says "The `allocate` method preallocates all needed memory. It should be called last before sending something into a real-time context."
  - A `Net` "can be divided into a frontend… while the real-time safe backend renders audio."
  - The wavetable oscillator "allocates memory in the form of global wavetables."
  - Source: [README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
- **Determinism:** the README has a section titled "Deterministic Pseudorandom Phase": "FunDSP uses a deterministic pseudorandom phase system for audio generators." — [README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
- **Effect and filter opcodes listed in the README:**
  - Dynamics: `limiter(a, r)` and `limiter_stereo` ("Look-ahead limiter with attack time (and latency)…").
  - Modulation: `chorus(seed, sep, var, mod)`, `flanger(fb, min_d, max_d, f)`, `phaser(fb, f)`.
  - Delays: `delay`, `tap`/`tap_linear` (cubic/linear interpolation), `multitap`, `feedback`/`feedback2`.
  - Oversampling and shaping: `oversample(node)` ("2x oversample enclosed node"), `shape(mode)`/`shape_fn(f)` waveshapers, `clip`.
  - Filters: Simper-SVF-based `lowpass`/`highpass`/`bandpass`/`notch`/`peak`/`bell`/`lowshelf`/`highshelf`/`morph`; `moog` ladder; "dirty biquad" and feedback-biquad variants (`dlowpass`, `dbell`, `fbell`…, citing Chowdhury's nonlinear-biquad article).
  - Utilities and oscillators: `follow`/`afollow` smoothing (asymmetric attack/release), `dcblock`, `declick`, `resample_fir`, `convolve` (0.23), PolyBLEP `poly_saw`/`poly_square`/`poly_pulse` (0.21), DSF oscillators, and bandlimited wavetables.
  - Source: [README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
  - **There is no compressor opcode in the README**, only the limiter.
- **wasm32 dependency footprint:** with default features, the wasm32 dependency tree has 81 crates, including `symphonia` (audio decoding) and `thingbuf`. It compiles natively on rustc 1.97. — local `cargo tree`/`cargo check` of [fundsp 0.23.0](https://docs.rs/crate/fundsp/0.23.0)
- **Conflicting data source:** lib.rs still shows fundsp 0.20.0 (Oct 2024) and "358 downloads/month". This contradicts the crates.io API (0.23.0, 2026-01-07), so the lib.rs cache appears stale. — [lib.rs/fundsp](https://lib.rs/crates/fundsp) vs [crates.io/fundsp](https://crates.io/crates/fundsp)

**dasp**
- **Release:** 0.11.0, released 2020-05-29 and never updated since. 5.34M total downloads, 1.09M in the last 90 days (mostly transitive), 27 reverse dependencies, `MIT OR Apache-2.0`. — [crates.io/dasp](https://crates.io/crates/dasp)
- **Activity:** last commit 2025-09-09; 0 commits since 2025-10-18. — [GitHub commits](https://github.com/RustAudio/dasp/commits/HEAD)
- **Build:** `dasp` with `features=["all"]` still compiles natively on rustc 1.97. — local `cargo check`, [dasp 0.11.0](https://docs.rs/crate/dasp/0.11.0)

**valib (SolarLiner)**
- **Licence:** LICENSE is MIT ("Copyright © 2022, SolarLiner"). The README says: "The code for the library (that is, everything under the `src` directory) is available under the `MIT` license. Plugins and examples are available under the `GPLv3` license." — [LICENSE](https://github.com/SolarLiner/valib/blob/HEAD/LICENSE), [README](https://github.com/SolarLiner/valib/blob/HEAD/README.md)
- **Activity:** last commit 2024-12-24; 0 commits in the last 12 months. — [GitHub commits](https://github.com/SolarLiner/valib/commits/HEAD)
- **Contents:** sub-crates `valib-core`, `-filters`, `-saturators`, `-oversample`, `-oscillators`, `-wdf`, `-voice`, `-fundsp` and `-nih-plug`. Features are "Generic over scalars / Generic over saturators / Oversampling filter / Biquad / SVF / Ladder". Its showcase plugin "Abrasive" is marked "(currently broken)". — [README](https://github.com/SolarLiner/valib/blob/HEAD/README.md)
- **crates.io name collision:** the crates.io crate `valib` (0.2.6, MIT, owner `valflrt`, described as "valflrt's utility crate", homepage gitlab.com/valflrt/valib) is **a different project**. `valib-core`, `valib-filters` and the other sub-crates do not exist on crates.io. SolarLiner's valib is therefore git-only. — [crates.io/valib](https://crates.io/crates/valib)

**synfx-dsp**
- **Release and licence:** 0.5.6, released 2024-01-04; licence `GPL-3.0-or-later` → **REFERENCE-ONLY**. 707 downloads in the last 90 days, 2 reverse dependencies. — [crates.io/synfx-dsp](https://crates.io/crates/synfx-dsp)
- **Repo:** the listed GitHub repo (github.com/WeirdConstructor/synfx-dsp) could not be cloned. git asked for credentials, which means it is not public or no longer exists. — [crates.io/synfx-dsp](https://crates.io/crates/synfx-dsp)
- **Build:** it fails on stable rustc 1.97 with ``error[E0554]: `#![feature]` may not be used on the stable release channel`` at `#![feature(portable_simd)]`, so it needs nightly. — local `cargo check`, [synfx-dsp 0.5.6 source](https://docs.rs/crate/synfx-dsp/0.5.6/source/src/lib.rs)

**Filters**

**biquad (korken89)**
- **Release and licence:** 0.6.0, released 2026-03-22; `MIT OR Apache-2.0`. 431k total downloads, 141k in the last 90 days, 23 reverse dependencies. crates.io categories are `embedded` and `no-std`. — [crates.io/biquad](https://crates.io/crates/biquad)
- **Activity:** last commit and tag v0.6.0 on 2026-03-22. 2 commits from 1 author in the past year (low activity but alive). — [GitHub](https://github.com/korken89/biquad-rs/commits/HEAD)
- **Dependency footprint:** only 4 crates in the wasm32 tree. — local `cargo tree`

**idsp (quartiq)**
- **Release and licence:** 0.23.1, released 2026-10-17; `MIT OR Apache-2.0`. 7 reverse dependencies. — [crates.io/idsp](https://crates.io/crates/idsp)
- **Activity:** 337 commits from 2 authors since 2025-10-18. — [GitHub](https://github.com/quartiq/idsp/commits/HEAD)
- **Design focus:** the README describes biquads in DF1, DF2T, DF1 with noise shaping and DF1 with wide output, coefficient sharing across lanes, and fixed-point support, and it benchmarks against biquad-rs. It is embedded/control-oriented (PID, lock-in), not music-oriented. — [README](https://github.com/quartiq/idsp/blob/HEAD/README.md)

**simper-filter (neodsp)**
- 0.1.1, released 2025-05-31; `MIT OR Apache-2.0`. Last commit 2025-07-29, 0 reverse dependencies. Stale and tiny. — [crates.io](https://crates.io/crates/simper-filter), [GitHub](https://github.com/neodsp/simper-filter)

**iir_filters**
- 0.1.3 (2023-10-16), Apache-2.0. Stale. — [crates.io](https://crates.io/crates/iir_filters)

**dsp (klangner)**
- 0.11.0 (2024-01-30), Apache-2.0. 312 downloads in the last 90 days. — [crates.io](https://crates.io/crates/dsp)

**FFT and resampling**

**rustfft**
- **Release and licence:** 6.4.1, released 2025-09-18; `MIT OR Apache-2.0`. 32.5M total downloads, 555 reverse dependencies. — [crates.io/rustfft](https://crates.io/crates/rustfft)
- **Activity:** last commit 2026-10-17; 18 commits from 5 authors in the past year. — [GitHub](https://github.com/ejmahler/RustFFT/commits/HEAD)

**realfft**
- **Release and licence:** 3.5.0, released 2025-06-12; licence `MIT`. 18.6M total downloads, 175 reverse dependencies. — [crates.io/realfft](https://crates.io/crates/realfft)
- **Activity:** 0 commits in the past year (mature and stable rather than abandoned; it is a thin wrapper over rustfft). — [GitHub](https://github.com/HEnquist/realfft/commits/HEAD)

**phastft** (newcomer FFT)
- 0.4.1, released 2026-07-31; `MIT OR Apache-2.0`. 314 commits from 2 authors in the past year, 2 reverse dependencies. — [crates.io](https://crates.io/crates/phastft), [GitHub](https://github.com/smu160/PhastFT)

**microfft** (no_std FFT)
- 0.6.0 (2024-04-14), MIT, hosted on GitLab. — [crates.io](https://crates.io/crates/microfft)

**rubato**
- **Release and licence:** 5.0.1, released 2026-10-01; `MIT OR Apache-2.0`. 13.4M total downloads, 243 reverse dependencies. — [crates.io/rubato](https://crates.io/crates/rubato)
- **Activity:** 119 commits from 6 authors in the past year. — [GitHub](https://github.com/HEnquist/rubato/commits/HEAD)

**audioadapter**
- 5.0.0 (2026-07-31), `MIT OR Apache-2.0`. 2.3M downloads in the last 90 days. — [crates.io](https://crates.io/crates/audioadapter)

**Graph engines, I/O and frameworks**

**Firewheel / firewheel-nodes**
- **Release and licence:** 0.14.0, released 2026-09-05; `MIT OR Apache-2.0`. firewheel-nodes has 2 reverse dependencies. — [crates.io/firewheel-nodes](https://crates.io/crates/firewheel-nodes)
- **Activity:** 272 commits from 9 authors in the past year. — [GitHub](https://github.com/BillyDM/firewheel/commits/HEAD)
- **Node set:** `svf`, `fast_filters`, `volume`, `volume_pan`, `mix`, `peak_meter`, `fast_rms`, `freeverb`, `convolution`, `sampler`, `noise_generator`, `delay_compensation`, `spatial_basic`, `stereo_to_mono` and `triple_buffer`. Feature flags include `all_nodes_no_std` and `libm`. **There is no compressor, chorus, phaser or distortion node.** — [firewheel-nodes/src](https://github.com/BillyDM/firewheel/tree/HEAD/crates/firewheel-nodes/src)
- **DSP primitives in firewheel-core:** `algo`, `declick`, `fade`, `filter`, `mix` and `volume`. — [firewheel-core/src/dsp](https://github.com/BillyDM/firewheel/tree/HEAD/crates/firewheel-core/src/dsp)

**cpal**
- **Release and licence:** 0.18.2, released 2026-08-16; licence `Apache-2.0` only. 24.8M total downloads, 715 reverse dependencies. — [crates.io/cpal](https://crates.io/crates/cpal)
- **Activity:** 451 commits from 39 authors in the past year. — [GitHub](https://github.com/RustAudio/cpal/commits/HEAD)

**nih-plug**
- **Not on crates.io.** ISC licence ("Copyright (c) 2022-2024 Robbert van der Helm"). — [LICENSE](https://github.com/robbert-vdh/nih-plug/blob/HEAD/LICENSE)
- **Maintenance status:** the README says "**`NIH-plug` the plugin framework is currently in maintenance mode.** If you are interested in the framework… check out this community fork". The link was added in the 2026-05-10 commit "Link to a community fork of the framework". Only 5 commits in the past year; the last was 2026-09-03. — [README](https://github.com/robbert-vdh/nih-plug/blob/HEAD/README.md), [commits](https://github.com/robbert-vdh/nih-plug/commits/HEAD)
- **VST3 licensing:** the README says the VST3 bindings (`vst3-sys`) "are licensed under the GPLv3 license. This means that… any VST3 plugins built with NIH-plug need to be able to comply with the terms of the GPLv3", and that plugins in `plugins/` "may be licensed under the GPLv3". — [README](https://github.com/robbert-vdh/nih-plug/blob/HEAD/README.md)

**nice-plug**
- **Release and licence:** 0.4.2, released 2026-09-14; crate created 2026-06-04; ISC; crates.io owner `BillyDM`; repository codeberg.org/RustAudio/nice-plug. 12,075 downloads in the last 90 days. — [crates.io/nice-plug](https://crates.io/crates/nice-plug)
- **Self-description (README):**
  - It "started out as a fork of the awesome NIH-plug framework… It has since become its own separate community-led project, and is now the recommended toolkit for Rust audio plugin developers" (this is the project's own claim).
  - "This project is currently experimental… expect some bugs."
  - "The nice-plug framework and all of the example plugins are licensed under the ISC license."
  - Source: [docs.rs README](https://docs.rs/crate/nice-plug/0.4.2)
- **VST3 bindings:** nice-plug 0.4.2 depends on the `vst3` crate 0.3.0, licensed `MIT OR Apache-2.0`, plus `clap-sys` 0.5.0. That is, it **no longer uses GPL vst3-sys**. — local `cargo tree` of [nice-plug 0.4.2](https://crates.io/crates/nice-plug)
- **Utility modules in nice-plug-core:**
  - Parameter smoothers with `SmoothingStyle::{None, Linear, Logarithmic, Exponential, OversamplingAware}`.
  - `util/stft.rs` (bring-your-own-FFT STFT helper) and `util/window.rs`.
  - An `assert_process_allocs` cargo feature (allocation assertion in the process call).
  - Source: [nice-plug-core 0.4.2 source](https://docs.rs/crate/nice-plug-core/0.4.2/source/)
- **Community endorsement:** BillyDM's awesome-audio-dsp list says: "The NIH-plug framework is no longer maintained. Please use the new community fork called nice-plug." — [awesome-audio-dsp OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md)

**clack-plugin** (CLAP; hosting is covered by the other researcher)
- 0.2.0, released 2026-09-12; `MIT OR Apache-2.0`. — [crates.io/clack-plugin](https://crates.io/crates/clack-plugin)

**Other Rust crates (effect-bearing)**

**web-audio-api (orottier)**
- **Release and licence:** 1.7.0, released 2026-08-08; licence MIT (LICENSE "Copyright (c) 2021 Otto Rottier"). — [crates.io](https://crates.io/crates/web-audio-api), [LICENSE](https://github.com/orottier/web-audio-api-rs/blob/HEAD/LICENSE)
- **Activity:** 224 commits from 10 authors in the past year. — [GitHub](https://github.com/orottier/web-audio-api-rs/commits/HEAD)
- **Relevant nodes:** `dynamics_compressor.rs`, `waveshaper.rs`, `biquad_filter.rs`, `iir_filter.rs`, `delay.rs` and `convolver.rs`. — [src/node](https://github.com/orottier/web-audio-api-rs/tree/HEAD/src/node)
- **Build:** the wasm32 dependency tree has 128 crates (cpal, wasm-bindgen, js-sys, symphonia, hound). On Linux it needs the ALSA system library. The README says WASM works "via `cpal`'s `wasm-bindgen` backend". — [README](https://github.com/orottier/web-audio-api-rs/blob/HEAD/README.md)

**augmented-audio (yamadapc)**
- **Releases:** `augmented-dsp-filters` 2.5.0 and `audio-processor-dynamics` 2.5.0, both released 2024-08-26 under MIT. In the last 90 days they had 686 and 36 downloads respectively. — [crates.io/augmented-dsp-filters](https://crates.io/crates/augmented-dsp-filters), [crates.io/audio-processor-dynamics](https://crates.io/crates/audio-processor-dynamics)
- **Licence split:** the repo README says "Most of this repository is published under the MIT LICENSE. Some directories which contain full applications are licensed under the AGPLv3". — [README](https://github.com/yamadapc/augmented-audio/blob/HEAD/README.md)
- **Activity:** 13 commits in the past year; last commit 2026-06-25. — [GitHub](https://github.com/yamadapc/augmented-audio/commits/HEAD)
- **Dependency footprint:** even the filters crate pulls wasm-bindgen, js-sys and crossbeam into its wasm32 tree (61 crates). — local `cargo tree`

**mi-plaits-dsp-rs**
- A native Rust port of the Mutable Instruments Plaits DSP. MIT ("Copyright (c) 2022 Oliver Rockstedt"). Last commit 2026-09-21; 51 commits in the past year. — [GitHub](https://github.com/sourcebox/mi-plaits-dsp-rs), [awesome-audio-dsp CODE_LIBRARIES](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)

**signalo**
- A "Rust DSP toolbox with focus on embedded environments". Last commit 2026-08-02; 142 commits in the past year. There is no licence file at the repo root, so the licence is **not verified**. — [GitHub](https://github.com/signalo/signalo), [awesome-audio-dsp CODE_LIBRARIES](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)

**Stale crates**

| Crate | Version | Released | Licence | Note | Source |
|---|---|---|---|---|---|
| glicol_synth | 0.13.5 | 2024-04-23 | crates.io says "non-standard"; repo LICENSE is MIT | Last commit 2025-01-23 | [crates.io](https://crates.io/crates/glicol_synth), [LICENSE](https://github.com/chaosprint/glicol/blob/HEAD/LICENSE) |
| knyst | 0.5.1 | 2024-11-25 | MIT/Apache | Last commit the same day | [GitHub](https://github.com/ErikNatanael/knyst) |
| twang | 0.9.0 | 2022 | | | [crates.io](https://crates.io/crates/twang) |
| fon | 0.6.0 | 2022 | | | [crates.io](https://crates.io/crates/fon) |
| synthizer | 0.5.6 | 2023 | | Bindings | [crates.io](https://crates.io/crates/synthizer) |

**Plumbing crates.** BillyDM's list names real-time plumbing crates relevant to plugin DSP: `assert_no_alloc`, `no_denormals`, `rtsan-standalone-rs` (RealtimeSanitizer wrapper), `basedrop`, `rtrb`, `ringbuf`, `triple_buffer` and `atomic_float`. — [awesome-audio-dsp CODE_LIBRARIES](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)

**wasm32 dependency-tree sizes (proxy only, not a compile test):**

| Crate | Crates in wasm32 tree |
|---|---|
| biquad | 4 |
| idsp | 7 |
| rustfft | 9 |
| realfft | 10 |
| dasp (all) | 18 |
| rubato | 24 |
| firewheel-nodes | 49 (includes crossbeam-utils) |
| augmented-dsp-filters | 61 |
| fundsp | 81 |
| web-audio-api | 128 |

- None of the pure-DSP crates' own sources call `thread::spawn` or `Instant::now`, except firewheel-core (1 file each, presumably std-only paths) and synfx-dsp (1 file uses Instant/SystemTime).
- Source: local `cargo tree` and source grep, 2026-10-18.

### Inferences
- **Inherently portable:** no_std crates (biquad, idsp, fundsp without `std`, firewheel-nodes `all_nodes_no_std`) with tiny dependency trees are the most likely to build cleanly for `wasm32-unknown-unknown`. This is unverified by compilation.
- **fundsp on wasm:** fundsp should be used with `default-features = false` to drop symphonia for wasm builds.
- **fundsp is the only broad Rust toolkit that is both permissive and recently maintained.** Its compile-time graph types make it a heavyweight dependency. Its real-time safety depends on calling `allocate()` up-front and avoiding wavetable creation on the audio thread.
- **Pinning:** fundsp has shipped 19 breaking releases (per lib.rs; "34 releases (19 breaking)"). Expect API churn, so pin exact versions. — [lib.rs/fundsp](https://lib.rs/crates/fundsp)
- **dasp** is effectively frozen. Use it only for sample/frame conversion traits, if at all.
- **valib** is a good *reading* resource for generic-over-scalar VA filters and saturators (the MIT `src/`). But it is unmaintained for nearly 2 years and git-only. Copy specific functions with attribution rather than depending on it.
- **nice-plug's switch to MIT/Apache `vst3` bindings** removes nih-plug's GPLv3 constraint on VST3 builds. This matters for an MIT-licensed plugin; cross-check with the hosting researcher's notes.
- **Determinism:**
  - All examined crates do plain f32/f64 arithmetic.
  - Bit-exact determinism across platforms will hinge on transcendental functions (std vs `libm`) and SIMD paths, not on the crates themselves.
  - fundsp is the only one that documents determinism explicitly (seeded pseudorandom phase).

### Gaps
- **Stars and issue counts:** GitHub stars, open-issue counts and maintainer lists could not be fetched (GitHub API blocked). The commit counts and authors above are the substitute health signal.
- **wasm32:** no compile test was possible. The wasm32 std component download was blocked.
- **Docs quality:** not systematically assessed beyond README content.
- **synfx-dsp:** its current home (possibly Codeberg) could not be verified. Codeberg was blocked.
- **Firewheel's real-time guarantees** were not examined beyond source layout.

---

## Q2. Which compressor (and other effect) implementations are worth reusing or porting?

### Takeaway
No healthy permissive **Rust** compressor crate exists:
- fundsp has only a look-ahead limiter.
- augmented-audio's dynamics crate is little-used and dormant since 2024-08.
- web-audio-api-rs has an MIT DynamicsCompressorNode that is portable.

The best permissive references to port are:
- Faust's JOS `compressor_mono`/`compressor_stereo` (STK-4.3, MIT-style).
- Signalsmith Basics (MIT): mature limiter, a brand-new compressor in `dynamics.h` (started 2026-09-25), chorus and crunch.
- Airwindows (MIT): ButterComp2, Pressure4/5/6, Logical4 and others.
- DaisySP core (MIT): limiter, chorus, flanger, phaser, overdrive.
- cycfi Q (MIT).

**Reference-only (copyleft):**
- DaisySP's compressor (in DaisySP-LGPL).
- chowdsp's compressor/EQ/filters/waveshapers (GPLv3).
- Most of Faust's `compressors.lib` (GPL-3.0-only functions).
- x42 dpl (GPL-3), Valentine (GPL-3) and lamb-rs (AGPL-3).

### Cited Findings

**Compressors / limiters**

**Signalsmith Basics (C++, header-only)**
- **Licence:** MIT ("Copyright 2022-2025 Signalsmith Audio / Geraint Luff"). The README says "basic effects, available as plugins and re-usable open-source (MIT) C++ classes". — [LICENSE.txt](https://github.com/Signalsmith-Audio/basics/blob/HEAD/LICENSE.txt), [README](https://github.com/Signalsmith-Audio/basics/blob/HEAD/README.md)
- **Headers:** `analyser.h`, `chorus.h`, `crunch.h`, `dynamics.h`, `freq-shifter.h`, `limiter.h` and `reverb.h`. — [include/signalsmith-basics](https://github.com/Signalsmith-Audio/basics/tree/HEAD/include/signalsmith-basics)
- **Release and activity:** tag 1.2.1 on 2026-09-30; 17 commits from 1 author in the past year. — [GitHub](https://github.com/Signalsmith-Audio/basics/commits/HEAD)
- **dynamics.h is very new:** its history is "2026-09-25 Start Dynamics", "Refactor controls, add aux input", "Add makeup gain". Code comments describe "Knees are implemented as BLEP-like curve, cancelling out the hard corner" and "An envelope follower with adaptive attack". **It is 3 weeks old; treat it as immature.** — [dynamics.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/dynamics.h)
- **Limiter:** "Limiter with maximum attack/lookahead of 100ms… `signalsmith::basics::LimiterFloat effect(100)`". The README says configuration must not happen "while actively processing audio" (allocation happens in `configure()`). — [README](https://github.com/Signalsmith-Audio/basics/blob/HEAD/README.md)

**Signalsmith DSP library (C++11 header-only)**
- **Licence and release:** MIT ("Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd."); tag v1.7.3 on 2026-10-13. — [LICENSE.txt](https://github.com/Signalsmith-Audio/dsp/blob/HEAD/LICENSE.txt), [commits](https://github.com/Signalsmith-Audio/dsp/commits/HEAD)
- **Headers:** `delay.h`, `filters.h`, `envelopes.h`, `fft.h`, `rates.h` (oversampling/resampling), `spectral.h`, `windows.h`, `mix.h` and `curves.h`. — [repo](https://github.com/Signalsmith-Audio/dsp)

**Faust libraries — per-function licence declarations in `compressors.lib`**
- **Licence breakdown:**

  | Count | Licence | Functions |
  |---|---|---|
  | 22 | **GPL-3.0-only** | Bart Brouns' `FFcompressor_N_chan`, `FBcompressor_N_chan`, `FBFFcompressor_N_chan`, `RMS_*` and expanders; Dario Sanfilippo's `limiter_lad_*` |
  | 4 | `LicenseRef-STK-4.3` | Julius O. Smith's `compressor_mono`, `compressor_stereo`, `compressor_lad_mono`, `compression_gain_mono` |
  | 1 | MIT | David Braun's `xfer_ott` |

- **File header:** the file header is LGPL with an "EXCEPTION… grants you the right to freely choose the license for the resulting… closed source license or any other license".
- Source: [compressors.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/compressors.lib)
- **Activity:** faustlibraries is very active, with 481 commits from 9 authors in the past year (last 2026-10-15). — [GitHub](https://github.com/grame-cncm/faustlibraries/commits/HEAD)
- **STK-4.3 licence text:** a copy is kept at [licenses/stk-4.3.0.md](https://github.com/grame-cncm/faustlibraries/tree/HEAD/licenses).

**Faust compiler and generated code**
- Debian's copyright summary says:
  - The compiler is GPL-2+ by default.
  - Architecture files are mostly GPL/LGPL "with FAUST exception" ("you may create a larger work that contains this FAUST architecture section and distribute that work under terms of your choice").
  - **It does not explicitly state the licence of compiler-generated code.**
  - Source: [Debian faust copyright](https://metadata.ftp-master.debian.org/changelogs/main/f/faust/stable_copyright)

**web-audio-api-rs `dynamics_compressor.rs`**
- MIT Rust. The source comments carry Chromium-style defaults (`knee = 30`, `ratio = 12`, `release = 0.25`). — [dynamics_compressor.rs](https://github.com/orottier/web-audio-api-rs/blob/HEAD/src/node/dynamics_compressor.rs)

**Airwindows**
- **Licence:** a single root MIT LICENSE ("Copyright (c) 2018 Chris Johnson"). — [LICENSE](https://github.com/airwindows/airwindows/blob/HEAD/LICENSE)
- **Size and activity:** 527 plugin directories under `plugins/LinuxVST/src`. Last commit 2026-10-17; 58 commits in the past year, all from 1 author. — [repo](https://github.com/airwindows/airwindows/tree/HEAD/plugins/LinuxVST/src)
- **Dynamics plugins:** `ButterComp`, `ButterComp2`, `Pressure4`, `Pressure5`, `Pressure6`, `Logical4`, `Dynamics`, `Dynamics2`, `Dynamics3`, `BeziComp`, `StoneFireComp` and `Compresaturator`.
- **Clipping and saturation:** `ADClip7`/`8`/`9`, `ClipOnly2`/`3`, `ClipSoftly`, `FinalClip`, `OneCornerClip`, `Density`/`2`/`3`, `Drive`, `PurestDrive` and `PurestSaturation`.
- **Modulation:** `Chorus`, `ChorusEnsemble` and `StereoChorus`.
- **Delays:** `TapeDelay`/`2`, `PitchDelay` and `SampleDelay`.
- **EQs:** `EQ`, `SmoothEQ`/`2`/`3`, `BezEQ`…, `PearEQ` and `ResEQ`.
- Source: [plugins/LinuxVST/src](https://github.com/airwindows/airwindows/tree/HEAD/plugins/LinuxVST/src)

**DaisySP (C++)**
- **Licence:** MIT ("DaisySP, copyright (c) 2020 Electrosmith, Corp. Published under the MIT license"). — [LICENSE](https://github.com/electro-smith/DaisySP/blob/HEAD/LICENSE)
- **Activity:** last commit 2026-09-28, but only 1 commit in the past year; latest tag V1.0.0 on 2024-01-08. **Low activity.** — [commits](https://github.com/electro-smith/DaisySP/commits/HEAD)
- **MIT core modules:**
  - `Dynamics/limiter`: header credits "Electrosmith, Corp, Emilie Gillet".
  - `Dynamics/crossfade`.
  - `Effects/`: `chorus`, `flanger`, `phaser`, `overdrive` (credited "Emilie Gillet", "Ported by Ben Sergentanis"), `wavefolder`, `autowah`, `tremolo`, `decimator`, `sampleratereducer` and `pitchshifter`.
  - `Filters/`: `svf` ("Credit to Andrew Simper from musicdsp.org"), `ladder` ("Ported from Audio Library for Teensy… Richard van Hoesel… Infrasonic Audio LLC", MIT text), `onepole`, `soap` and `fir`.
  - `Synthesis/oscillator` ("polyBLEP bandlimited waveforms"), `variablesawosc`, `variableshapeosc`, `zoscillator` and `formantosc`.
  - `Utility/delayline` and `dcblock`.
  - Source: [Source tree](https://github.com/electro-smith/DaisySP/tree/HEAD/Source)
- **DaisySP-LGPL submodule** (github.com/Electro-smith/DaisySP-LGPL; last commit 2023-12-21). It contains `Dynamics/compressor`, `balance`, `Filters/moogladder`, `biquad`, `tone`, `atone`, `allpass`, `comb`, `mode`, `nlfilt`, `Synthesis/blosc`, `Effects/bitcrush`, `fold`, `reverbsc`, `Utility/port` and `jitter`. **These are LGPL → REFERENCE-ONLY.** In particular, DaisySP's compressor and moogladder are *not* MIT. — [DaisySP .gitmodules](https://github.com/electro-smith/DaisySP/blob/HEAD/.gitmodules), [DaisySP-LGPL](https://github.com/electro-smith/DaisySP-LGPL)

**Mutable Instruments (pichenettes/eurorack)**
- **Licence (README):** "Code (AVR projects): GPL3.0. Code (STM32F projects): MIT license. Hardware: cc-by-sa-3.0". — [README](https://github.com/pichenettes/eurorack/blob/HEAD/README.md)
- **Activity:** last commit 2026-10-12; 2 commits in the past year. — [commits](https://github.com/pichenettes/eurorack/commits/HEAD)
- **stmlib:**
  - MIT header ("Copyright 2014 Emilie Gillet. Permission is hereby granted…").
  - `dsp/` contains `filter.h`, `polyblep.h`, `limiter.h`, `delay_line.h`, `parameter_interpolator.h`, `sample_rate_converter.h`, `cosine_oscillator.h` and `atan.h`.
  - Last commit 2023-09-03 (stable and stale).
  - Source: [stmlib dsp/filter.h](https://github.com/pichenettes/stmlib/blob/HEAD/dsp/filter.h)
- **MIT STM32 module effect code:** `rings/dsp/fx/chorus.h`, `rings/dsp/fx/ensemble.h`, `plaits/dsp/fx/ensemble.h`, `plaits/dsp/fx/overdrive.h`, `rings/dsp/limiter.h`, `warps/dsp/limiter.h`, `warps/dsp/sample_rate_conversion_filters.h` and `beads/dsp/multitap_delay.h`. — [eurorack tree](https://github.com/pichenettes/eurorack)

**cycfi Q (C++)**
- **Licence:** MIT ("Copyright (c) 2014-2026 Joel de Guzman"). — [LICENSE](https://github.com/cycfi/q/blob/HEAD/LICENSE)
- **Release and activity:** v1.5.4 on 2026-10-09; 457 commits from 4 authors in the past year. — [commits](https://github.com/cycfi/q/commits/HEAD)
- **`q/fx` headers:** `dynamic.hpp`, `envelope.hpp`, `biquad.hpp`, `svf.hpp`, `ladder.hpp`, `allpass.hpp`, `delay.hpp`, `clip.hpp`, `noise_gate.hpp`, `moving_maximum.hpp`, `dc_block.hpp` and `fast_downsample.hpp`. — [q_lib/include/q/fx](https://github.com/cycfi/q/tree/HEAD/q_lib/include/q/fx)
- **`q/synth/va` headers:** `analog_osc`, `saw_osc`, `pulse_osc`, `square_osc`, `triangle_osc` and `sync`. — [q_lib/include/q/synth/va](https://github.com/cycfi/q/tree/HEAD/q_lib/include/q/synth/va)

**chowdsp_utils**
- **Licence model:** "Each module in this repository has its own unique license… All non-module code… is licensed under the GPLv3." — [LICENSE.md](https://github.com/Chowdhury-DSP/chowdsp_utils/blob/HEAD/LICENSE.md)
- **Module licences:**

  | Licence | Modules |
  |---|---|
  | **GPLv3 (REFERENCE-ONLY)** | `chowdsp_compressor`, `chowdsp_dsp_utils`, `chowdsp_dsp_data_structures`, `chowdsp_eq`, `chowdsp_filters`, `chowdsp_modal_dsp`, `chowdsp_reverb`, `chowdsp_sources`, `chowdsp_waveshapers`, `chowdsp_plugin_base`, `chowdsp_plugin_utils`, GUI modules |
  | BSD | `chowdsp_core`, `chowdsp_data_structures`, `chowdsp_math`, `chowdsp_simd`, `chowdsp_buffers`, `chowdsp_parameters`, `chowdsp_plugin_state`, `chowdsp_presets` and others |

  Source: [README](https://github.com/Chowdhury-DSP/chowdsp_utils/blob/HEAD/README.md)
- **Release and activity:** v2.4.0 on 2025-11-29; last commit 2025-12-09; 10 commits in the past year. **Slowing; no commits for about 10 months.** — [commits](https://github.com/Chowdhury-DSP/chowdsp_utils/commits/HEAD)

**Copyleft dynamics references (REFERENCE-ONLY)**
- **x42 dpl.lv2** (look-ahead digital peak limiter): GPL-3 (COPYING); v0.7.3 on 2026-09-08. — [repo](https://github.com/x42/dpl.lv2)
- **Valentine** (compressor/saturator): GPL-3; last commit 2025-08-24. — [repo](https://github.com/tote-bag-labs/valentine)
- **lamb-rs** ("A clean lookahead compressor/limiter" in Faust + Rust + NIH-plug): **AGPL-3.0**; last commit 2025-09-29. — [repo](https://github.com/magnetophon/lamb-rs), [awesome-audio-dsp](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md)
- **nih-plug's Safety Limiter / Spectral Compressor:** in `plugins/`, which "may be licensed under the GPLv3". — [nih-plug README](https://github.com/robbert-vdh/nih-plug/blob/HEAD/README.md)

**Delay / modulation / EQ (permissive)**
- **Delay:**
  - fundsp: `tap`/`multitap` with cubic interpolation, plus `feedback(...)` (README example: "1 second ping-pong delay"). — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
  - Signalsmith `delay.h`. — [Signalsmith dsp](https://github.com/Signalsmith-Audio/dsp)
  - DaisySP `delayline.h`. — [DaisySP](https://github.com/electro-smith/DaisySP/tree/HEAD/Source/Utility)
  - stmlib `delay_line.h`. — [stmlib](https://github.com/pichenettes/stmlib)
- **EQ:**
  - fundsp `bell`, `lowshelf` and `highshelf` are implemented as Simper SVFs (README cites Cytomic's "SvfLinearTrapOptimised2.pdf"). — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
  - The biquad crate (MIT/Apache, no_std). — [crates.io](https://crates.io/crates/biquad)
  - web-audio-api `BiquadFilterNode`/`IIRFilterNode` (MIT). — [src/node](https://github.com/orottier/web-audio-api-rs/tree/HEAD/src/node)

### Inferences
- **A compressor is small enough to write in-house:** gain computer with soft knee, attack/release detector, optional look-ahead and make-up gain. Porting is lower-risk than depending on a crate. The best permissive references, in order:
  1. Faust JOS `compressor_mono` (STK-4.3; textbook, well documented).
  2. web-audio-api-rs DynamicsCompressorNode (already Rust, MIT; Chromium-compatible behaviour).
  3. Signalsmith `dynamics.h` (MIT, modern design, but brand-new).
  4. Airwindows ButterComp2/Pressure (MIT; "character" designs; code is per-plugin and monolithic).
- **Faust usage:** pick Faust functions by their per-function `declare … license` line. Using compiler-generated code from only STK-4.3/MIT functions appears compatible with MIT. The compiler-output licence is not explicitly stated, so confirm with GRAME or the Faust docs before shipping generated code.
- **Modulation effects:** DaisySP core (MIT) chorus/flanger/phaser and Signalsmith Basics chorus are the most directly portable C++ sources. fundsp's chorus/flanger/phaser are usable directly if fundsp is adopted.

### Gaps
- **Airwindows:** no per-plugin quality comparison was found, and Airwindows' anti-aliasing approach (generally none, or its own tricks) was not verified from source.
- **Will Pirkle's code licence:** the licence of the book code (`fxobjects.h`, ASPiK) could **not be verified**. Searches returned nothing authoritative, and github.com/willpirkle/ASPiK was not found. Treat it as unknown/reference-only until the LICENSE in the book's download package is checked.
- **Teensy Audio and Maximilian:** licences not verified. Teensy Audio (PaulStoffregen/Audio; last commit 2026-05-05) has no root licence file. Maximilian (last commit 2025-01-04) is stale.
- **Faust Rust backend:** whether Faust's Rust backend output is real-time-safe or deterministic was not checked in this session.

---

## Q3. Permissively licensed virtual-analog filters and anti-aliased oscillators/waveshapers worth porting?

### Takeaway
**Yes, several, verified permissive:**
- **Faust `aanl.lib`:** anti-aliased nonlinearities (ADAA), all 37 functions MIT.
- **Faust `vaeffects.lib` / `filters.lib`:** mostly STK-4.3/MIT.
- **Signalsmith `elliptic-blep`** (MIT) and **`hilbert-iir`** (0BSD).
- **Signalsmith `rates.h`** oversampling (MIT).
- **DaisySP core** (MIT): SVF, Teensy-derived ladder, polyBLEP oscillators, wavefolder, overdrive.
- **Mutable Instruments stmlib/STM32 code** (MIT): SVF, polyBLEP, SRC filters, overdrive.
- **cycfi Q** (MIT): VA oscillators, ladder, SVF.
- **`sst-oscillators-mit`** (MIT, stale).
- **chowdsp_wdf** (BSD-3).
- **In Rust:** fundsp's PolyBLEP/DSF oscillators, `oversample`, Simper SVFs and `moog`; valib's MIT `src/` (stale); mi-plaits-dsp-rs (MIT).

**Reference-only (GPL):**
- Vital, Surge's `sst-filters`, `sst-effects` and `sst-waveshapers`.
- chowdsp `filters` and `waveshapers`.
- AVR-era Mutable code.

**Zavalishin's "The Art of VA Filter Design"** is a book (theory), not code. Implementing its TPT/ZDF equations is unencumbered (inference; not verified as a legal statement).

### Cited Findings

**Faust libraries**
- **Per-function licence counts:**

  | Library | Licence counts |
  |---|---|
  | `aanl.lib` | 37 × MIT |
  | `filters.lib` | 120 × STK-4.3, 25 × MIT, 1 × ISC, 1 × LGPL-2.1-or-later-with-Faust-exception |
  | `vaeffects.lib` | 20 × STK-4.3, 10 × MIT, 1 × BSD-3-Clause |
  | `misceffects.lib` | 13 × STK-4.3, 8 × MIT |
  | `oscillators.lib` | 6 × STK-4.3, 2 × LGPL-with-exception |

  Source: [faustlibraries](https://github.com/grame-cncm/faustlibraries)
- **Discrepancy:** Debian's summary lists `oscillators.lib` and `phaflangers.lib` as "LGPL-2.1 or later" at file level. The per-function `declare license` lines are more granular. Check each function before reuse. — [Debian faust copyright](https://metadata.ftp-master.debian.org/changelogs/main/f/faust/stable_copyright)

**Signalsmith**
- **elliptic-blep:** MIT ("Copyright 2024 Signalsmith Audio Ltd. / Geraint Luff"); last commit 2025-08-21. — [repo](https://github.com/Signalsmith-Audio/elliptic-blep)
- **hilbert-iir:** "BSD Zero Clause License"; last commit 2026-01-20. awesome-audio-dsp describes it as "A high quality, public domain filter in a single-header C++ file". — [repo](https://github.com/Signalsmith-Audio/hilbert-iir), [awesome-audio-dsp](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)

**Surge (sst-\*)**
- **sst-basic-blocks:**
  - The README says it "is largely derived from refactoring surge… which are GPL3 and as such this library is GPL3".
  - "A small number of individual header files are also available to use in an MIT license context… explicitly marked in the text of the header file". `include/sst/basic-blocks/dsp/LanczosResampler.h` is listed among them.
  - It bundles Signalsmith's MIT elliptic-blep in `libs/elliptic-blep`.
  - Source: [README](https://github.com/surge-synthesizer/sst-basic-blocks/blob/HEAD/README.md)
- **GPL-3 repos (all active as of Oct 2026) → REFERENCE-ONLY:**

  | Repo | Last commit | Source |
  |---|---|---|
  | `sst-filters` | 2026-10-13 | [sst-filters](https://github.com/surge-synthesizer/sst-filters) |
  | `sst-effects` | 2026-10-16 | [sst-effects](https://github.com/surge-synthesizer/sst-effects) |
  | `sst-waveshapers` | 2026-02-25 | [sst-waveshapers](https://github.com/surge-synthesizer/sst-waveshapers) |

- **sst-oscillators-mit:** MIT ("Copyright 2019-2020, Paul Walker"), but last commit 2022-03-28 (**stale**). — [repo](https://github.com/surge-synthesizer/sst-oscillators-mit)

**Vital**
- GPL-3.0; last commit 2022-04-20 → **REFERENCE-ONLY**. The README offers "non GPLv3 licensing options" via licensing@vital.audio. — [README](https://github.com/mtytel/vital/blob/HEAD/README.md)

**chowdsp_wdf**
- BSD-3-Clause ("Copyright (c) 2022, Chowdhury-DSP"); last commit 2026-07-02. A header-only C++ wave-digital-filter library. — [repo](https://github.com/Chowdhury-DSP/chowdsp_wdf)

**DaisySP core (MIT)**
- **Oscillators:** `Synthesis/oscillator.h` describes "Synthesis of several waveforms, including polyBLEP bandlimited waveforms".
- **Filters:** `Filters/svf.h` gives "Credit to Andrew Simper from musicdsp.org". `Filters/ladder.h` is "Ported from Audio Library for Teensy" with MIT permission text.
- **Effects:** `Effects/wavefolder` and `Effects/overdrive` are in core.
- Source: [DaisySP Source](https://github.com/electro-smith/DaisySP/tree/HEAD/Source)

**Mutable Instruments (MIT STM32 code)**
- stmlib `dsp/polyblep.h`, `dsp/filter.h` and `dsp/sample_rate_converter.h`; `warps/dsp/sample_rate_conversion_filters.h`; `plaits/dsp/fx/overdrive.h`. — [stmlib](https://github.com/pichenettes/stmlib), [eurorack](https://github.com/pichenettes/eurorack)

**Rust options**
- **fundsp:**
  - Oscillators: "The PolyBLEP oscillator is a fast approximation with fair quality". DSF is "similar quality but… somewhat expensive". "The wavetable oscillator is bandlimited".
  - `oversample()` is 2x only.
  - `moog` is a 4th-order ladder.
  - Source: [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
- **web-audio-api `WaveShaperNode`:** implements `OverSampleType::{None, X2, X4}` using `rubato::FftFixedInOut` for the up/down-sampling (MIT). — [waveshaper.rs](https://github.com/orottier/web-audio-api-rs/blob/HEAD/src/node/waveshaper.rs)
- **valib:** its MIT `src/` covers oversampling, biquad, SVF, ladder, saturators and WDF (stale since 2024-12-24). — [valib](https://github.com/SolarLiner/valib)

### Inferences
- **Recommended anti-aliased waveshaping stack (all permissive):**
  - First-order ADAA from Faust `aanl.lib` (MIT) for static curves (tanh/hardclip/etc.).
  - Plus 2×–4× polyphase/IIR oversampling modelled on Signalsmith `rates.h` (MIT) or MI's SRC filters (MIT).
  - FFT-based `rubato` (as in web-audio-api) adds block latency. It is better suited to offline or fixed-block use than to a low-latency insert. This is inference from the rubato type name `FftFixedInOut`, not benchmarked.
- **VA filters:** implementing Zavalishin/Simper TPT SVF and ladder directly in Rust is about 100 lines each. Cytomic's SVF paper is already what fundsp and DaisySP follow, so writing your own avoids any licence question.
- **Oscillators:** for anti-aliased oscillators, fundsp's PolyBLEP is "fair quality". Signalsmith elliptic-blep (MIT) or MI-style polyBLEP (MIT) are better references for a synth-grade oscillator.

### Gaps
- **Zavalishin licence:** the licence or terms of "The Art of VA Filter Design" PDF were not fetched. The claim that implementing its equations is unencumbered is inference, not a verified legal position.
- **Pirkle code licence:** unknown (see Q2).
- **Quality:** no measured aliasing comparisons between fundsp PolyBLEP, DaisySP polyBLEP and elliptic-BLEP were found.

---

## Q4. What do Rust audio developers say about which crates are production-ready vs abandoned?

### Takeaway
Direct community discussion (Rust Audio Discord, Reddit, KVR) could not be retrieved from this environment. The fetch attempts for the KVR thread failed, and Reddit and Discord were not reachable.

The strongest *verifiable* community signals are maintainer and curator statements:
- **nih-plug's own README** says the framework is "in maintenance mode" and points to the community fork.
- **BillyDM's curated list** (widely used in the Rust audio community) says to use nice-plug, and lists fundsp, valib, synfx-dsp, mi-plaits-dsp and signalo as Rust DSP libraries without health caveats.
- **A March 2026 blog post** calls the Rust audio ecosystem "still young compared to C++" with "no JUCE equivalent yet".

### Cited Findings
- "NIH-plug the plugin framework is currently in maintenance mode. If you are interested in the framework rather than the plugin, please check out this community fork instead." — [nih-plug README](https://github.com/robbert-vdh/nih-plug/blob/HEAD/README.md)
- "The NIH-plug framework is no longer maintained. Please use the new community fork called nice-plug." — [awesome-audio-dsp (BillyDM)](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md)
- **nice-plug's self-assessment:** it calls itself "now the recommended toolkit for Rust audio plugin developers" but also says it is "currently experimental, and has recently undergone some large changes, so expect some bugs". — [nice-plug README on docs.rs](https://docs.rs/crate/nice-plug/0.4.2)
- **awesome-audio-dsp "Code Libraries" entries:**
  - fundsp: "A Rust DSP crate with a unique and fun syntax".
  - synfx-dsp: "A comprehensive DSP library in Rust, used by the HexoSynth synthesizer plugin".
  - valib: "A Rust library of reusable blocks for musical DSP".
  - Firewheel DSP: "A small collection of DSP I use in my Firewheel library".
  - The list carries no maintenance warnings.
  - Source: [CODE_LIBRARIES.md](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)
- **BillyDM's advice on learning DSP:** "I often get asked about beginner-friendly DSP learning resources which focus on the Rust programming language. Unfortunately there isn't really anything out there. I would suggest learning DSP in another language first…" — [awesome-audio-dsp README](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/README.md)
- **Joel Löf blog (opinion, "Last updated: Mar 22, 2026"):**
  - Rust is "gaining traction in audio, though the ecosystem is still young compared to C++".
  - It has a "Smaller ecosystem", "no JUCE equivalent yet", and GUI options that are "less mature".
  - Rust DSP performance is "comparable to C++ for DSP workloads".
  - It names NIH-plug, CPAL and Symphonia as the "Growing audio ecosystem".
  - Source: [joellof.com](https://joellof.com/blog/audio-plugin-development-cpp-rust-web/)
- **Rust plugins in the wild:** community plugins built on Rust/NIH-plug that implement effects include ardura's plugins, Dust Saturator, Havregryn, lamb (AGPL) and VitaliumVerb. These show that effect DSP is typically hand-written per plugin rather than taken from a shared crate. — [awesome-audio-dsp OPEN_SOURCE_PLUGINS_AND_SOFTWARE](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md)
- **KVR thread:** "Future of Rust in audio development" exists in the KVR DSP and Plugin Development forum, but its content could not be fetched. — [KVR t=626803](https://www.kvraudio.com/forum/viewtopic.php?t=626803)

### Inferences
- **Health tiers from the objective signals (commit cadence, releases, reverse deps):**

  | Tier | Crates |
  |---|---|
  | Production-ready, actively maintained | rustfft, realfft (mature), rubato, cpal, biquad, idsp |
  | Usable but evolving | fundsp (breaking releases, 7 months since last commit), Firewheel (pre-1.0, fast-moving), nice-plug (experimental) |
  | Abandoned or stale | dasp (no release since 2020), valib (no commits since 2024-12), synfx-dsp (GPL, nightly-only, repo gone from GitHub), knyst (2024-11), glicol_synth (2025-01), augmented-audio DSP crates (no release since 2024-08) |

- **Ecosystem pattern:** in the Rust ecosystem, effect DSP is mostly written per-project. Shared crates concentrate on FFT, resampling, I/O, graph engines and filters.

### Gaps
- **No direct community discussion was retrieved:** Rust Audio Discord summaries, r/rust or r/DSP threads, and KVR posts on crate production-readiness were inaccessible. No first-hand developer quotes about fundsp, biquad or Firewheel in production were found.
- **Popularity signals:** GitHub stars and issue counts are unavailable, so popularity signals rely on crates.io downloads and reverse dependencies only.

---

## Q5. Recommended "reuse list" for a first release (compressor + 1–2 modulation effects + EQ)

### Takeaway
Depend on only a few small, permissive, healthy crates, and port small permissive algorithms in-house for the effects themselves:
- **Crates:** `biquad` or an in-house TPT SVF for EQ, `realfft`/`rustfft` only if needed, and `nice-plug`'s smoothers if using that framework.
- **Ported algorithms:**
  - Compressor from Faust JOS (STK-4.3) and web-audio-api-rs (MIT).
  - Chorus/flanger/phaser from DaisySP core (MIT) or Signalsmith Basics (MIT).
  - Anti-aliased saturation from Faust `aanl.lib` (MIT) plus Signalsmith `rates.h`-style oversampling (MIT).
- **Treat as REFERENCE-ONLY:** everything GPL/LGPL/AGPL (chowdsp DSP modules, DaisySP-LGPL, Surge sst-\*, Vital, most of Faust `compressors.lib`, lamb-rs, x42 dpl).

### Cited Findings (basis for each pick)

**EQ**
- The `biquad` crate: MIT/Apache, no_std, 0.6.0 released 2026-03-22, 141k downloads in the last 90 days, 4-crate dependency tree. — [crates.io/biquad](https://crates.io/crates/biquad)
- **Alternative:** a Simper-SVF bell/shelf, as fundsp does (README cites Cytomic SvfLinearTrapOptimised2.pdf). — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)

**Compressor (port, do not depend)**
- Faust `compressor_mono`/`compressor_stereo` by Julius O. Smith III, `LicenseRef-STK-4.3`. — [compressors.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/compressors.lib)
- web-audio-api-rs `dynamics_compressor.rs` (MIT, Rust). — [source](https://github.com/orottier/web-audio-api-rs/blob/HEAD/src/node/dynamics_compressor.rs)
- **Optional character references:** Airwindows ButterComp2/Pressure4 (MIT). — [Airwindows](https://github.com/airwindows/airwindows/tree/HEAD/plugins/LinuxVST/src)
- **Optional modern design reference:** Signalsmith `dynamics.h` (MIT, started 2026-09-25). — [dynamics.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/dynamics.h)
- **Safety limiter:** fundsp `limiter`/`limiter_stereo` (look-ahead). — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)
- **Alternative safety limiter:** Signalsmith `limiter.h` (MIT). — [Signalsmith Basics](https://github.com/Signalsmith-Audio/basics)

**Modulation (chorus plus phaser or flanger)**
- DaisySP core `Effects/chorus`, `flanger` and `phaser` (MIT). — [DaisySP Effects](https://github.com/electro-smith/DaisySP/tree/HEAD/Source/Effects)
- Signalsmith Basics `chorus.h` (MIT). — [Signalsmith Basics](https://github.com/Signalsmith-Audio/basics)
- MI `rings/dsp/fx/chorus.h` / `ensemble.h` (MIT STM32 code). — [eurorack](https://github.com/pichenettes/eurorack)
- **Or directly** fundsp `chorus`/`flanger`/`phaser`, if adopting fundsp. — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)

**Delay lines and interpolation**
- Signalsmith `delay.h` (MIT). — [Signalsmith dsp](https://github.com/Signalsmith-Audio/dsp)
- fundsp `tap` (cubic). — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)

**Distortion with anti-aliasing (if in scope)**
- Faust `aanl.lib` (all MIT). — [aanl.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/aanl.lib)
- Signalsmith `rates.h` (MIT). — [Signalsmith dsp](https://github.com/Signalsmith-Audio/dsp)
- DaisySP `overdrive` (MIT, Emilie Gillet-derived). — [DaisySP](https://github.com/electro-smith/DaisySP/tree/HEAD/Source/Effects)

**Parameter smoothing**
- nice-plug's `SmoothingStyle::{Linear, Logarithmic, Exponential, OversamplingAware}` (ISC). — [nice-plug-core](https://docs.rs/crate/nice-plug-core/0.4.2/source/)
- **Or** an in-house one-pole, like fundsp's `follow`/`afollow`. — [fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)

**FFT / resampling (only if needed for analysers or offline rendering)**
- `realfft` (MIT), `rustfft` (MIT/Apache), `rubato` (MIT/Apache). All are actively used, with 175, 555 and 243 reverse dependencies respectively. — [realfft](https://crates.io/crates/realfft), [rustfft](https://crates.io/crates/rustfft), [rubato](https://crates.io/crates/rubato)

**Real-time testing aids**
- `assert_no_alloc`, `rtsan-standalone-rs` and `no_denormals`. — [awesome-audio-dsp CODE_LIBRARIES](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/CODE_LIBRARIES.md)
- nice-plug's `assert_process_allocs` feature. — [nice-plug](https://crates.io/crates/nice-plug)

### Inferences
- **Why port rather than depend for the effects themselves:**
  - (a) No permissive Rust compressor crate is both maintained and used.
  - (b) Each algorithm is a few hundred lines.
  - (c) In-house code gives full control over determinism (fixed math functions, no hidden SIMD paths), allocation (preallocate in `initialize`), and wasm32 portability (no_std + `libm`).
  - (d) It avoids fundsp's breaking-release churn and heavyweight generic types in a plugin's public parameter model.
- **When fundsp is the better fit:** adopting fundsp wholesale is reasonable if the synth engine already uses it. It provides limiter, chorus, flanger, phaser, SVF EQ, oversampling and waveshapers in one MIT/Apache crate. Use `default-features = false` (no_std/alloc) and call `allocate()` before audio starts.
- **Attribution:** keep a `THIRD_PARTY_NOTICES` file carrying the copyright and permission notices for every MIT/BSD/STK-4.3 source ported. All of these licences require retaining notices. The STK-4.3 wording was not quoted here, so check the [licenses/stk-4.3.0.md](https://github.com/grame-cncm/faustlibraries/tree/HEAD/licenses) text.
- **Avoid dependencies on:**
  - dasp: stale since 2020.
  - valib: stale since 2024-12, git-only, crates.io name collision.
  - synfx-dsp: GPL, nightly-only.
  - augmented-audio DSP crates: dormant, and they pull wasm-bindgen/crossbeam.
  - web-audio-api as a dependency (128-crate tree including cpal); port its node code instead.

### Gaps
- **No benchmarks or listening tests:** none were found comparing these compressor or chorus implementations. Quality reputation is inferred from author reputation and adoption, not from measurements.
- **Pirkle book code licence:** unverified, so it was excluded from the reuse list.
- **Faust compiler output licence:** not explicitly stated in the sources checked. Prefer hand-porting the STK-4.3/MIT Faust functions over shipping compiler output until this is confirmed.
