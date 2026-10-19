# Reusable reverb DSP for an MIT-licensed Rust synth (state as of 2026-10-18)

Method note (read first). Licences were checked in the actual LICENSE files and source headers. I shallow-cloned each repo at HEAD on 2026-10-18, or downloaded the published `.crate` from static.crates.io. The "last commit" dates come from `git log -1` on those clones. Crate versions, dates and download counts come from the crates.io API, also pulled on 2026-10-18. The GitHub REST API was blocked in this session, so star and issue counts exist only for repos whose GitHub page I fetched directly. Those are fundsp, Firewheel, CloudSeedCore and airwindows. For everything else, star counts are a gap. No `wasm32-unknown-unknown` build was run, because the session could not download the rust-std component for that target (static.rust-lang.org was blocked). Every wasm32 statement below is therefore an inference from dependencies and source, not a compile result.

Project context, from the session brief: `enth` is an MIT-licensed Rust CLAP synth (nih-plug). It needs real-time safety and **bit-for-bit determinism**, because offline re-renders are used for scoring and ML, and the core crates must build for wasm32.

## Q1. Which Rust crates provide a production-quality reverb, and how do they rate on licence, health, algorithm, real-time safety, determinism and wasm32?

### Takeaway
**fundsp** is the only permissively licensed (MIT OR Apache-2.0), actively maintained Rust crate that ships several deterministic, pure-Rust reverbs, including 32-line FDNs and an allpass loop, with `no_std` support. Nearly every other permissive Rust option is a Freeverb port: `freeverb` (irh), Firewheel's node, kira, fyrox-sound and rustysynth. Two are Dattorro plates: `lanceverb` and `glicol_synth`'s plate. `cloudseedcore-rs` wraps the MIT CloudSeed algorithm, but through C++ (cxx), which hurts wasm32 and portability. The best-known Rust Dattorro (`synfx-dsp`) and the Vital reverb port (`vitalium_verb_dsp`) are GPL-3.0-or-later and are **reference-only**.

### Cited Findings

**fundsp** (Sami Perttu)
- Licence `MIT OR Apache-2.0`. Latest release 0.23.0 on 2026-01-07, 37 versions since 2022-01-30. 232,620 total downloads, 52,990 in the last 90 days. Published by SamiPerttu. — [crates.io/fundsp](https://crates.io/crates/fundsp)
- Last commit at HEAD: 2026-03-03. GitHub shows about 1.2k stars, 68 forks, 9 open issues, 3 open PRs and 704 commits (page fetched 2026-10-18). — [GitHub SamiPerttu/fundsp](https://github.com/SamiPerttu/fundsp)
- The README says FunDSP "supports `no_std` environments" if the default `std` feature is disabled. `alloc` is still needed. The `allocate()` method "preallocates all needed memory and should be called last before entering a real-time context", and the `Net` backend is described as real-time safe. — [GitHub SamiPerttu/fundsp](https://github.com/SamiPerttu/fundsp)
- It ships four stereo reverbs in `prelude.rs` (verified in the 0.23.0 source):
  - `reverb_stereo(room_size, time, damping)`: a 32-channel FDN with a fixed table of 32 "optimized delay times… from a legacy project" and FIR damping.
  - `reverb2_stereo(room, time, diffusion, modulation_speed, filter)`: a "32-channel hybrid FDN" with nested Schroeder allpasses, delay-line modulation and pre-diffusers, documented as "More sophisticated (and expensive)".
  - `reverb3_stereo(time, diffusion, filter)`: an "Allpass loop based stereo reverb".
  - `reverb4_stereo(room_size, time)`: two cascaded 16-line FDNs, with delays "Optimized… from `optimize.rs` example".
  - Every one is documented with "Allocates: delay lines". — [fundsp 0.23.0 src/prelude.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/prelude.rs)
- The modulation in `reverb2_stereo` uses `spline_noise(i, t…)`, which is `spline_noise(seed: u64, x)` built on `rnd2(seed)`. That is a hash of a fixed seed and time, with no OS RNG and no clock. The delay tables are compile-time constants. — [fundsp 0.23.0 src/math.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/math.rs); [prelude.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/prelude.rs)
- The delay nodes allocate only in `set_sample_rate` (`self.buffer.resize(buffer_length, 0.0)`) and at construction, never in `tick`. — [fundsp 0.23.0 src/delay.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/delay.rs)
- CHANGES.md records that "New reverb opcodes `reverb2_stereo` and `reverb3_stereo`" were added, along with "Attempted optimization of reverb delay times in the example `optimize`". — [fundsp CHANGES.md](https://docs.rs/crate/fundsp/0.23.0/source/CHANGES.md)

**Firewheel / firewheel-nodes** (Billy Messenger, BillyDM)
- Licence `MIT OR Apache-2.0`. Version 0.14.0 on 2026-09-05. firewheel-nodes has 54,264 downloads (19,845 in 90 days). — [crates.io/firewheel-nodes](https://crates.io/crates/firewheel-nodes)
- GitHub shows 317 stars, 29 forks and 21 open issues. The README says: "Included backends supporting … WebAssembly", "`no_std` compatibility (some features require the standard library)", and "Properly respects realtime constraints (no mutexes!)". It also says Firewheel is planned to be upstreamed into Bevy. — [GitHub BillyDM/firewheel](https://github.com/BillyDM/firewheel)
- The reverb node is `freeverb/`: "A Rust implementation of Freeverb by Ian Hobson… [freeverb-rs]". It is 725 lines including node plumbing. Delay lines are resized at stream start (`self.freeverb.resize(stream_info.sample_rate…)`), and lengths are scaled by `sr / 44100`. — [firewheel-nodes 0.14.0 src/freeverb/mod.rs](https://docs.rs/crate/firewheel-nodes/0.14.0/source/src/freeverb/mod.rs); [freeverb.rs](https://docs.rs/crate/firewheel-nodes/0.14.0/source/src/freeverb/freeverb.rs)

**freeverb** (Ian Hobson, irh)
- Licence MIT. Version 0.2.0 on 2025-10-29 (2 versions in total). 5,550 downloads. — [crates.io/freeverb](https://crates.io/crates/freeverb)
- It is 509 lines of Rust with no dependencies. `Freeverb::new(sr)` allocates the delay lines with `vec!` and scales lengths by `sr / 44100`. A grep found no RNG. It describes Freeverb as "released into the public domain in June 2000". — [freeverb 0.2.0 src/lib.rs](https://docs.rs/crate/freeverb/0.2.0/source/src/lib.rs); [src/freeverb.rs](https://docs.rs/crate/freeverb/0.2.0/source/src/freeverb.rs)

**kira, fyrox-sound and rustysynth** (game and soundfont engines that contain a Freeverb)
- kira: `MIT OR Apache-2.0`, 0.12.5 on 2026-09-26, 1,027,287 downloads. Its reverb says "This code is based on Freeverb by Jezar at Dreampoint". Buffers are allocated in `init` and `on_change_sample_rate`, scaled from a 44,100 Hz reference. — [crates.io/kira](https://crates.io/crates/kira); [kira 0.12.5 src/effect/reverb.rs](https://docs.rs/crate/kira/0.12.5/source/src/effect/reverb.rs)
- fyrox-sound: MIT, 1.0.1 on 2026-03-28. Its reverb is "implementation of Freeverb reverb effect". — [crates.io/fyrox-sound](https://crates.io/crates/fyrox-sound); [fyrox-sound src/effects/reverb.rs](https://docs.rs/crate/fyrox-sound/1.0.1/source/src/effects/reverb.rs)
- rustysynth: MIT, 1.3.7 on 2026-10-01. Its reverb is a Freeverb-style comb and allpass bank, with buffers allocated in `new`. — [crates.io/rustysynth](https://crates.io/crates/rustysynth); [rustysynth src/reverb.rs](https://docs.rs/crate/rustysynth/1.3.7/source/src/reverb.rs)

**Dattorro plates in Rust**
- `lanceverb` (sourcebox): MIT, a single release 0.4.0 on 2025-06-15, 516 downloads. The README calls it "A super-fast mono-to-stereo plate reverberator based on the design from Jon Dattorro (1997)", originally written in C++ by Lance Putnam and ported by MindBuffer. It is "`no_std` with an overall RAM usage of ~90kB", depends only on `libm`, and is 422 lines. — [crates.io/lanceverb](https://crates.io/crates/lanceverb); [lanceverb README](https://docs.rs/crate/lanceverb/0.4.0/source/README.md)
- The crate ships an `ORIGINAL_LICENSE.md` for Lance Putnam's upstream code. It is the Allocore BSD-style licence: "Copyright (C) 2009. AlloSphere Research Group… Redistribution and use… permitted provided…". That licence is permissive, but its notice must be kept. — [lanceverb ORIGINAL_LICENSE.md](https://docs.rs/crate/lanceverb/0.4.0/source/ORIGINAL_LICENSE.md)
- `reverb` (crate name): MIT, 1.0.47 on 2023-06-09, 26,483 downloads. It is a "Fork of… MindBuffer/lanceverb… A fast mono plate reverberator". **Stale (last release 2023).** — [crates.io/reverb](https://crates.io/crates/reverb)
- `glicol_synth`: crates.io shows the licence as "non-standard", but the packaged LICENSE file is "The MIT License (MIT) Copyright (c) 2020-present Qichao Lan". Version 0.13.5 on 2024-04-23. It contains `node/effect/plate.rs` (a plate built from glicol's own delay, allpass and one-pole nodes) and `reverb.rs`. **Stale (2024)**, and `plate.rs` holds `Vec` routing state. — [crates.io/glicol_synth](https://crates.io/crates/glicol_synth); [glicol_synth plate.rs](https://docs.rs/crate/glicol_synth/0.13.5/source/src/node/effect/plate.rs)

**FDN/diffuser design in Rust**
- `basic-reverb` (yolisses): MIT, 0.1.3 on 2023-11-28. "A Rust implementation of the reverb presented at 'Let's Write A Reverb'". It uses `rand::thread_rng()` to choose diffuser delay sizes and polarity flips (`diffusion_step.rs`), so it is **not deterministic**, and it pulls in `rand`. **Stale (2023).** — [crates.io/basic-reverb](https://crates.io/crates/basic-reverb); [basic-reverb src/diffuser/diffusion_step.rs](https://docs.rs/crate/basic-reverb/0.1.3/source/src/diffuser/diffusion_step.rs)

**CloudSeed in Rust**
- `cloudseedcore-rs` (Marius Metzger, CrushedPixel): MIT, 0.3.0 on 2025-11-28 (3 versions), 111 downloads in total. Last git commit 2026-01-07. It describes itself as "safe Rust bindings to the CloudSeedCore Reverb algorithm" and vendors talaviram's fork, which "fixes some cross-platform compatibility issues". "Building this crate requires a C++14 toolchain. The C++ bindings are created using cxx." `build.rs` compiles `Parameters.cpp`, `Biquad.cpp` and `RandomBuffer.cpp` with `cxx_build`. — [crates.io/cloudseedcore-rs](https://crates.io/crates/cloudseedcore-rs); [GitHub CrushedPixel/cloudseedcore-rs](https://github.com/CrushedPixel/cloudseedcore-rs)

**GPL and LGPL Rust crates (reference-only, cannot be reused in an MIT project)**
- `synfx-dsp` (WeirdConstructor): **GPL-3.0-or-later**, 0.5.6 on 2024-01-04. Its `dattorro.rs` header reads "Released under GPL-3.0-or-later… largely based on the C++ implementation from ValleyAudio / ValleyRackFree… Adapted under the GPL-3.0-or-later License". — [crates.io/synfx-dsp](https://crates.io/crates/synfx-dsp); [synfx-dsp src/dattorro.rs](https://docs.rs/crate/synfx-dsp/0.5.6/source/src/dattorro.rs)
- `vitalium_verb_dsp` (BillyDM): **GPL-3.0-or-later**, 0.2.2 on 2026-06-29. It is "A Rust port of the reverb module from the Vital/Vitalium synthesizer" and "requires the nightly Rust toolchain since it uses the `portable_simd` feature". — [crates.io/vitalium_verb_dsp](https://crates.io/crates/vitalium_verb_dsp); [GitHub BillyDM/vitalium-verb](https://github.com/BillyDM/vitalium-verb)
- `oxisynth-reverb`: **LGPL-2.1**, 0.1.0 on 2025-05-25 (FluidSynth lineage). — [crates.io/oxisynth-reverb](https://crates.io/crates/oxisynth-reverb)
- `surgefx-reverb`: **GPL-3.0**, 0.2.12-alpha.0 on 2023-04-07. — [crates.io/surgefx-reverb](https://crates.io/crates/surgefx-reverb)
- `mverb-rs` (Wint3rmute): **GPL-3.0**, a rewrite of martineastwood/mverb, with 3 stars and 6 commits. — [GitHub Wint3rmute/mverb-rs](https://github.com/Wint3rmute/mverb-rs)

**Crates with no reverb**
- SolarLiner's `valib` git repo (MIT, last commit 2024-12-24) contains filters, oscillators, saturators, WDF, oversampling and voice crates, but no reverb or FDN module at HEAD. The `valib` name on crates.io belongs to a different author (valflrt, MIT, 0.2.6 on 2026-02-08). — [GitHub SolarLiner/valib](https://github.com/SolarLiner/valib); [crates.io/valib](https://crates.io/crates/valib)
- `knyst` 0.5.1 (2024-11-25, MIT/Apache) had no reverb module found by a source grep. — [crates.io/knyst](https://crates.io/crates/knyst)

**Newer crates (2026) that mention reverb but were not inspected**
- These came up in a crates.io search but are low-download and immature: `oximedia-effects` (Apache-2.0, 2026-08-12, 882 downloads), `aetherdsp-nodes` (MIT, 2026-05-26, 401 downloads), `tinyviolin` (MIT, 2026-08-11), `audio-processor-time` (MIT, 2024-08-26, "delay/reverb"). — [crates.io search "reverb"](https://crates.io/search?q=reverb)

### Inferences
- **wasm32-unknown-unknown (inferred, not compiled):**
  - fundsp (with `no_std`/`alloc`), `freeverb`, `lanceverb` (`no_std` + libm) and firewheel-nodes' Freeverb are pure Rust with no C dependencies or OS calls in the reverb path, so they should build. Firewheel's README explicitly claims WebAssembly support.
  - `cloudseedcore-rs` will very likely **not** build for `wasm32-unknown-unknown` without extra work. It needs a C++14 compiler and the C++ standard library (`std::vector`, `std::rand`) through `cxx`, and that target has no C++ stdlib.
  - `basic-reverb` depends on `rand::thread_rng`. On wasm32-unknown-unknown that requires getrandom's JS backend, which conflicts with determinism anyway.
- **Determinism ranking for the Rust options:**
  - fundsp reverbs: deterministic by construction.
  - Freeverb ports: no modulation and no RNG, so deterministic.
  - lanceverb: a grep found no RNG.
  - cloudseedcore-rs: inherits CloudSeedCore's `std::rand()` (see Q2/Q4).
  - basic-reverb: non-deterministic.
- **Fit for enth:** fundsp is the strongest drop-in. It is permissively licensed, active (release 2026-01, commit 2026-03), deterministic, `no_std`-capable, allocation-free after `allocate()`/`set_sample_rate`, and offers four reverb flavours. Its main risk is subjective sound quality, which no third party has evaluated (see Q3). A Freeverb port is the zero-risk fallback but sounds dated (Q3).

### Gaps
- Star and issue counts for freeverb-rs, lanceverb, cloudseedcore-rs, kira and synfx-dsp were not retrieved (GitHub API blocked).
- No wasm32 compile test could be run (rust-std for the target could not be downloaded).
- No published listening tests or user reviews of fundsp's `reverb*_stereo` sound quality turned up (a search returned nothing relevant).
- Firewheel's maintainer and contributor counts were not shown on the fetched page.

## Q2. Which permissively licensed non-Rust implementations are worth porting, and which are GPL or otherwise reference-only?

### Takeaway
The verified permissive candidates are:
- **CloudSeedCore** (MIT, Ghost Note Audio, 2024). It is the "lush" algorithmic reverb with a commercial pedigree, but the repo is stale and small.
- **Signalsmith Basics `reverb.h`** (MIT, 2022–2025, active). A modern multichannel-diffuser plus FDN design with deterministic seeded randomness.
- **Mutable Instruments Clouds/Rings/Elements reverb** (MIT). A compact Griesinger/Dattorro plate.
- **Airwindows** (MIT, very active), with 40+ reverbs.
- **Faust `reverbs.lib` functions** under MIT or STK-4.3: `jpverb`, `greyhole`, `dattorro_rev`, `zita_rev1_stereo`, `valhallaSupermassive`.
- **fverb** (BSD-2-Clause Dattorro).
- **el-visio dattorro-verb** (MIT).
- **STK FreeVerb** (MIT-style).
- Freeverb itself (public domain).

Reference-only:
- **JUCE 9's `juce::Reverb`**, now AGPLv3 or commercial.
- Surge XT, Vital, Valley Plateau, MVerb and CloudReverb (all GPL-3.0).
- DaisySP's ReverbSc (LGPL-2.1).
- Faust `vital_rev` and `kb_rom_rev1` (GPL-3.0-only).
- Signalsmith's ADC'21 example repo has **no licence file**, so use the MIT `basics` version instead.

### Cited Findings

**CloudSeed and CloudSeedCore** (Valdemar Erlingsson / Ghost Note Audio)
- The original `ValdemarOrn/CloudSeed` `license.txt` is "The MIT License (MIT) Copyright (c) 2018 Valdemar Erlingsson". Its readme now says "Cloud Seed has moved to a new home" and points to `GhostNoteAudio/CloudSeedCore` for "the core algorithm". The legacy C# and C++ code is on the `legacy-v1` branch. Last commit 2024-05-26. — [GitHub ValdemarOrn/CloudSeed](https://github.com/ValdemarOrn/CloudSeed)
- The `GhostNoteAudio/CloudSeedCore` LICENSE is "MIT License Copyright (c) 2024 Ghost Note Audio". Source headers read "Copyright (c) 2024 Ghost Note Engineering Ltd" with the MIT text. — [GitHub GhostNoteAudio/CloudSeedCore LICENSE](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/LICENSE); [DSP/RandomBuffer.cpp](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/RandomBuffer.cpp)
- The README says: "This is the core DSP code of Cloud Seed 2.0… The plugin itself is not open source, but the core reverb algorithm is… This code it MIT Licensed… Please credit Ghost Note Audio… You *can* use the name 'Cloud Seed'… I do reserve the right to ask you to please stop using the Cloud Seed name". It also says "It requires C++14 to compile". — [CloudSeedCore Readme.md](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/Readme.md)
- Repo health: last commit 2024-09-22 ("undo use of clamp as it requires c++17"). GitHub shows 79 stars, 26 forks, 2 open PRs and **7 commits in total** (fetched 2026-10-18). **Stale: no commits for about 2 years.** — [GitHub GhostNoteAudio/CloudSeedCore](https://github.com/GhostNoteAudio/CloudSeedCore)
- Cloud Seed 2.0 (the commercial plugin) was released on 31 May 2024 at £39 Personal / £69 Professional. KVR says the 2014 original was "an open-source, Windows-only plugin". — [KVR news](https://www.kvraudio.com/news-print.php?id=60893)

**Ports of CloudSeed**
- **Aether** (Dougal Stewart): MIT ("Copyright (c) 2021 Dougal Stewart"). "an algorithmic reverb LV2 plugin based on Cloudseed". Last commit 2023-02-18. **Stale.** — [GitHub Dougal-s/Aether](https://github.com/Dougal-s/Aether)
- **CloudReverb** (xunil-cloud): **GPL-3.0** (the LICENSE file is the GPLv3 text). A JUCE port of CloudSeed legacy-v1, last commit 2026-09-05. **Reference-only.** — [GitHub xunil-cloud/CloudReverb](https://github.com/xunil-cloud/CloudReverb)

**Airwindows** (Chris Johnson)
- LICENSE: "MIT License Copyright (c) 2018 Chris Johnson". Per-file headers read "Copyright (c) airwindows, Airwindows uses the MIT license". Last commit 2026-10-17. GitHub shows about 1.2k stars, 122 forks and 34 open issues. The README says "Airwindows is read-only" and does not accept PRs. — [airwindows LICENSE](https://github.com/airwindows/airwindows/blob/HEAD/LICENSE); [Galactic3.h](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3.h); [GitHub airwindows](https://github.com/airwindows/airwindows)
- Airwindopedia lists 40+ reverbs, including VerbSevens, kCathedral5, Galactic3, kPlateA–D, Verbity2, Chamber/Chamber2, Infinity2, MatrixVerb and NonlinearSpace.
  - "Galactic3 is exactly Galactic again, but with better undersampling".
  - "Verbity2… expands Verbity's feedforward reverb topology".
  - "Chamber is a feedforward reverb, using three banks of four delays each in a Householder feedback matrix".
  - Recent designs use "5x5… 6x6" Householder matrices. — [Airwindopedia.txt](https://github.com/airwindows/airwindows/blob/HEAD/Airwindopedia.txt)

**Signalsmith Audio** (Geraint Luff)
- `reverb-example-code` (the ADC 2021 "Let's Write A Reverb" companion) contains only README.md, main.cpp, mix-matrix.h, reverb-example-code.h and wav.h. **There is no LICENSE file and no licence header** in its sources. Last commit 2021-11-15. The article itself states no licence for the code either. — [GitHub Signalsmith-Audio/reverb-example-code](https://github.com/Signalsmith-Audio/reverb-example-code); [Let's Write a Reverb](https://signalsmith-audio.co.uk/writing/2021/lets-write-a-reverb/)
- `Signalsmith-Audio/basics`: LICENSE.txt is "Copyright 2022-2025 Signalsmith Audio / Geraint Luff", with MIT text. The README says "available as plugins and re-usable open-source (MIT) C++ classes", including **Reverb**. Last commit 2026-09-30. — [Signalsmith basics LICENSE.txt](https://github.com/Signalsmith-Audio/basics/blob/HEAD/LICENSE.txt); [README](https://github.com/Signalsmith-Audio/basics)
- `basics/reverb.h` is 409 lines and depends on `signalsmith-dsp/delay.h`, `mix.h`, `filters.h` and `stfx-library.h`. Its parameters are dry, wet, roomMs, rt20, early, detune, lowCutHz, highCutHz, lowDampRate and highDampRate. Its randomness is `std::mt19937 engine(seed)` inside `updateLengths(int seed, …)`, which is deterministic given the seed. — [basics reverb.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/reverb.h)
- The article's design is a multichannel diffuser (delays, shuffle/polarity, Hadamard) feeding a Householder-matrix FDN. The example uses 8 channels, 4 diffusion steps and 100–200 ms feedback delays. The author claims "no magic numbers, no tricky tuning" and says it is "robust enough that I could throw a random-number generator at all the delay times". — [Let's Write a Reverb](https://signalsmith-audio.co.uk/writing/2021/lets-write-a-reverb/)

**Mutable Instruments** (Emilie Gillet)
- The eurorack README says "Code (AVR projects): GPL3.0… Code (STM32F projects): MIT license… Hardware: cc-by-sa-3.0". `clouds/dsp/fx/reverb.h` carries an MIT header ("Copyright 2014 Emilie Gillet"). Last commit 2026-10-12. — [eurorack README](https://github.com/pichenettes/eurorack); [clouds/dsp/fx/reverb.h](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/reverb.h)
- The Clouds reverb comments say "This is the Griesinger topology described in the Dattorro paper". It is 180 lines, plus `fx_engine.h` at 302 lines, and similar files exist in rings/, elements/ and beads/. The LFO rates are hard-coded relative to 32 kHz (`0.5f / 32000.0f`). The engine supports `FORMAT_12_BIT`, `FORMAT_16_BIT` and `FORMAT_32_BIT` delay storage. — [clouds/dsp/fx/reverb.h](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/reverb.h); [clouds/dsp/fx/fx_engine.h](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/fx_engine.h)

**Dattorro plate implementations**
- **fverb** (jpcima): "A stereo variant of the reverberator by Jon Dattorro". LICENSE.md is BSD 2-Clause, and `fverb.dsp` declares `license "BSD-2-Clause"`. It is a 180-line Faust file. Last commit 2022-01-10. **Stale.** — [GitHub jpcima/fverb](https://github.com/jpcima/fverb)
- **el-visio/dattorro-verb** (Pauli Pölkki): MIT, 2022. "Jon Dattorro reverb implementation in C… faithful to the original paper". It is 341 lines of C, with `malloc` only at creation. Last commit 2022-02-27. **Stale.** — [GitHub el-visio/dattorro-verb](https://github.com/el-visio/dattorro-verb)
- **Valley Plateau** (ValleyRackFree, Dale Johnson): **GPL-3.0**. LICENSE.md reads "This program is free software…" with LICENSE-GPLv3.txt. Last commit 2023-11-08. **Reference-only.** — [GitHub ValleyAudio/ValleyRackFree](https://github.com/ValleyAudio/ValleyRackFree)
- **MVerb** (Martin Eastwood): **GPL-3.0**, last commit 2022-07-03. **Reference-only.** — [GitHub martineastwood/mverb](https://github.com/martineastwood/mverb)

**Faust standard library** (`reverbs.lib`, last commit 2026-10-15). Licensing is per section and per function:
- "jos section": jcrev, satrev, fdnrev0, zita_rev_fdn, zita_rev1_stereo and zita_rev1_ambi are "Copyright (C) 2003-2017 by Julius O. Smith III… released under the (MIT-style) STK-4.3 license". zita_rev1 is described as the FDN "used in the FOSS Linux reverb `zita-rev1` by Fons Adriaensen". — [faustlibraries reverbs.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/reverbs.lib)
- "GRAME section" (which includes `mono_freeverb`/`stereo_freeverb`): LGPL-2.1+ with "EXCEPTION TO THE LGPL LICENSE… you may… distribute the compiled code generated by the FAUST compiler… under your own copyright and license… the resulting compiled code has no obligation to be LGPL or GPL". — [reverbs.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/reverbs.lib)
- Per-function declarations:
  - `dattorro_rev` (Jakob Zerbian): `"LicenseRef-STK-4.3"`.
  - `jpverb` and `greyhole` (Julian Parker): `"MIT"`.
  - `valhallaSupermassive` (David Braun): `"MIT"`.
  - **`vital_rev` (David Braun): `"GPL-3.0-only"`**.
  - **`kb_rom_rev1` (Luca Spanedda): `"GPL-3.0-only"`**.
  - The "further contributions" section says "all contributions… should indicate both the contributor and terms of license… If no such indication is found, 'git blame' will say who…". `springreverb` (Daniel Leonov) has an author but no licence declaration. — [reverbs.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/reverbs.lib)
- The STK-4.3 licence text (`licenses/stk-4.3.0.md`) is MIT-style: "Permission is hereby granted… The above copyright notice and this permission notice shall be included in all copies or substantial portions". — [faustlibraries licenses/stk-4.3.0.md](https://github.com/grame-cncm/faustlibraries/blob/HEAD/licenses/stk-4.3.0.md)

**JUCE**
- At HEAD (2026-10-12), LICENSE.md says "The JUCE Framework modules are dual-licensed under the AGPLv3 and the commercial JUCE licence" (the JUCE 9 EULA). Only "The JUCE examples are licensed under the terms of the ISC license". — [JUCE LICENSE.md](https://github.com/juce-framework/JUCE/blob/HEAD/LICENSE.md)
- The `juce_Reverb.h` header gives the JUCE EULA "Or: You may also use this code under the terms of the AGPLv3". It is "a simple stereo reverb, based on the technique and tunings used in FreeVerb". **Not reusable in MIT code.** Reimplement Freeverb from the public-domain original or an MIT port instead. — [juce_Reverb.h](https://github.com/juce-framework/JUCE/blob/HEAD/modules/juce_audio_basics/utilities/juce_Reverb.h)

**Surge XT and Vital**
- Surge XT: LICENSE is GPL-3.0, last commit 2026-10-18. **Reference-only.** — [Surge LICENSE](https://github.com/surge-synthesizer/surge/blob/HEAD/LICENSE)
- Vital: LICENSE is GPL-3.0, last commit 2022-04-20. Its reverb is `src/synthesis/effects/reverb.cpp`. **Reference-only.** The Rust ports (`vitalium_verb_dsp`, Faust `vital_rev`) inherit the GPL. — [Vital LICENSE](https://github.com/mtytel/vital/blob/HEAD/LICENSE)

**DaisySP**
- The main DaisySP repo is MIT ("DaisySP, copyright (c) 2020 Electrosmith, Corp. Published under the MIT license"; last commit 2026-09-28) and has no reverb. `ReverbSc` (a port of Sean Costello's Csound `reverbsc`) lives in **DaisySP-LGPL** ("Published under the LGPL-V2.1 license", last commit 2023-12-21). It uses "random variation in delay time". **Reference-only for an MIT project.** — [DaisySP LICENSE](https://github.com/electro-smith/DaisySP/blob/HEAD/LICENSE); [DaisySP-LGPL reverbsc.cpp](https://github.com/electro-smith/DaisySP-LGPL/blob/HEAD/Source/Effects/reverbsc.cpp)

**STK and Freeverb**
- STK LICENSE: "Copyright (c) 1995-2023 Perry R. Cook and Gary P. Scavone. Permission is hereby granted…" (MIT-like, with a non-binding request to send modifications). Last commit 2025-03-29. `FreeVerb.cpp` is "Jezar at Dreampoint's FreeVerb, implemented in STK… Ported to STK by Gregory Burlet, 2012". It notes that "The delay lengths are optimized for a sample rate of 44100 Hz". — [STK LICENSE](https://github.com/thestk/stk/blob/HEAD/LICENSE); [STK src/FreeVerb.cpp](https://github.com/thestk/stk/blob/HEAD/src/FreeVerb.cpp)
- The freeverb-rs crate docs say Freeverb by "Jezar at Dreampoint… was released into the public domain in June 2000". kira cites the same public-domain origin. — [freeverb src/lib.rs](https://docs.rs/crate/freeverb/0.2.0/source/src/lib.rs); [kira reverb.rs](https://docs.rs/crate/kira/0.12.5/source/src/effect/reverb.rs)

### Inferences
- For an MIT project that wants a "big, lush" reverb, **CloudSeedCore** is the strongest permissive algorithm source. It is explicitly MIT, it is the core of a commercial product, and several independent ports exist (Aether, Daisy, cloudseedcore-rs). The README's attribution and trademark requests should be honoured: credit Ghost Note Audio, and avoid marketing as "Cloud Seed" in a confusing way.
- **Signalsmith basics `reverb.h`** is the cleanest modern FDN to port: MIT, actively maintained, deterministic seeded RNG, under 500 lines plus small helpers. The 2021 example repo should not be copied verbatim because it has no licence. `basic-reverb` (MIT) is a port of that unlicensed code, which is a provenance concern worth noting.
- The **Faust route** is attractive for `jpverb`/`greyhole` (MIT) and `dattorro_rev`/`zita_rev1_stereo` (STK-4.3, MIT-style). Generated code should keep the STK notice. The GPL-declared functions (`vital_rev`, `kb_rom_rev1`) must be excluded, and `springreverb` (no licence declared) avoided until clarified.
- **Airwindows** is MIT and prolific, but it is not a library. Each reverb is a standalone plugin with a `Proc.cpp` (Galactic3 is about 800 lines across 3 files, with large `double` arrays). It is portable, but ownership of later fixes is on the porter, since the repo is read-only.

### Gaps
- Star and issue counts for Signalsmith basics, eurorack, faustlibraries, fverb, Aether and STK were not retrieved.
- I did not verify the licence of the Faust architecture files used when generating Rust (`faust -lang rust`). That needs checking if the Faust route is chosen.
- The Dattorro paper (AES JAES 1997) is an algorithm description. Whether any patent applies was not researched. The only related claim found is a Freeverb3 page saying a lawsuit "ultimately settled in Dattorro's favor" ([freeverb3 tips](https://freeverb3-vst.sourceforge.io/tips/reverb.shtml)). Treat it as unverified.
- talaviram's CloudSeedCore fork (vendored by cloudseedcore-rs) was not inspected separately.

## Q3. What do sound designers and plugin developers say about quality, and which free, permissively licensed algorithm is considered best in 2025–2026?

### Takeaway
I found no consensus "best permissive reverb" statement for 2025–2026. The opinion record is thin and scattered across forums.
- **CloudSeed** has the clearest positive reputation among the permissive options: commercial re-release in 2024, and hardware porters comparing it to Strymon.
- **Freeverb** (and therefore JUCE's Reverb and most Rust game-engine reverbs) is widely described as simple, "a bit metallic and raw".
- **Dattorro plates** are liked for density and low cost, but are fragile to retune.
- Developer discussion favours **FDN designs** (Signalsmith-style) as easier to get right.
- None of these are measured comparisons. They are opinions.

### Cited Findings
- **Opinion (Freeverb/JUCE):** IvanC on the JUCE forum (2022-09-30) said the JUCE/Freeverb "AllPassFilter… is not an all-pass filter at all!!" but a comb with extra gain (coefficient 0.5 versus about 0.618), and that the simple Schroeder structure "sounds a bit metallic and raw". Another user (Luzifer, 2022-10-04) could not see a difference in spectrograms. — [JUCE forum](https://forum.juce.com/t/the-all-pass-filter-in-juce-reverb-is-a-comb-filter/53202)
- **Opinion (Schroeder-style designs):** The Freeverb3 author's tips page says the parallel-comb plus series-allpass design "has a VERY metallic sound for sharp transients". — [freeverb3 tips](https://freeverb3-vst.sourceforge.io/tips/reverb.shtml)
- **Expert opinion (Sean Costello, Valhalla DSP, 2011):** "In practice, allpass delays don't sound flat… Run an impulsive signal through several short allpass delays in series, and the result is a metallic decaying sound". He lists the remedies as modulation, more or fewer allpasses, and smaller coefficients, each with trade-offs. — [Valhalla DSP blog](https://valhalladsp.com/2011/01/21/reverbs-diffusion-allpass-delays-and-metallic-artifacts/)
- **Developer opinion (Dattorro vs FDN, KVR DSP forum, 2021–2024):**
  - Richard_Synapse says the figure-8 (Dattorro) loop is cheap, but changing "a delay or two often spoils the reverb completely", and that FDNs are "far easier to get right".
  - mystran says "an FDN doesn't automatically produce a colorless tail" (modal clustering can ring).
  - Louis Couka prefers plate-style algorithms as "fast and dense". — [KVR "Dattorro reverb improvements"](https://www.kvraudio.com/forum/viewtopic.php?p=8090199)
- **User opinion (CloudSeed on hardware):**
  - On PedalPCB (Daisy Seed/Terrarium port), the porter keyth72 (2023-04-28) said CloudSeed "seems comparable in sound quality to some of the Strymon reverb algorithms". Other users called it "sounds amazing" and said it "really opens up the sonic landscape".
  - The porter also reported CPU limits ("5 was the max I was able to run" delay lines on Daisy) and a flash-size build error. — [PedalPCB forum](https://forum.pedalpcb.com/goto/post?id=222976)
- **Publisher claim (not independent):** The original Cloud Seed "garnered a good reputation" with sound designers. — [KVR news](https://www.kvraudio.com/news-print.php?id=60893)
- **Author claims (Airwindows, self-assessment):** "Verbity2 might beat Galactic, for you, for deep reverbs." Recent work is about "avoiding allpasses completely by using reverb matrixes". — [Airwindopedia.txt](https://github.com/airwindows/airwindows/blob/HEAD/Airwindopedia.txt)
- **Author claims (Signalsmith):** The FDN plus diffuser design is "smooth" and "high-quality", with "no magic numbers". — [Let's Write a Reverb](https://signalsmith-audio.co.uk/writing/2021/lets-write-a-reverb/)
- **Community (HISE forum, 2025-09):** A developer building a reverb cites the Signalsmith ADC video and the Valhalla DSP articles as the starting path. Another poster mentions "using neural networks to tune FDNs to be spectrally flat". — [HISE forum](https://forum.hise.audio/post/109525)

### Inferences
- Among code that can be reused under MIT, the community signals rank roughly as follows for lush, ambient, synth-pad use:
  1. CloudSeed (Core).
  2. A well-tuned Signalsmith-style FDN or the Airwindows Galactic/Verbity family.
  3. Dattorro plate (classic, characterful, a bit metallic without modulation).
  4. Freeverb (dated, metallic).
- For a synth (pads, plucks), CloudSeed and Dattorro/Galactic flavours fit better than a "realistic room".
- These are **opinions, not measurements**. A short A/B listening test against fundsp `reverb2_stereo`/`reverb4_stereo`, which are already deterministic and in Rust, would settle it for this project more cheaply than further web research.

### Gaps
- The KVR "what's the best free reverb" threads could not be fetched (fetch errors). Reddit r/DSP and r/audioengineering threads were behind a bot check and could not be read.
- The Rust Audio Discord is not publicly indexable. No Rust-community quality discussion of fundsp, lanceverb or freeverb-rs was found.
- No objective comparisons (echo-density, modal-density or listening tests) across these open implementations were found.

## Q4. Porting effort and pitfalls for the top candidates

### Takeaway
1. **Zero port:** use fundsp's reverbs directly. Deterministic, pure Rust, `no_std`.
2. **Port CloudSeedCore:** about 2.3k lines of dependency-free C++14. The pitfalls are `std::rand()` in modulation-phase initialisation, `std::vector` allocations when seed parameters change, roughly 64 MB of fixed-size buffers per stereo instance, and no denormal handling.
3. **Port Signalsmith basics `reverb.h`:** about 400 lines plus small MIT helpers, with a seeded `mt19937` that must be replaced by a Rust PRNG producing identical or project-defined sequences.

A Dattorro plate (lanceverb MIT/BSD, or Clouds MIT at about 480 lines) is the smallest characterful alternative. It needs care with 32 kHz-hardcoded constants.

### Cited Findings

**CloudSeedCore**
- Size: DSP headers and .cpp files total **2,336 lines**, plus `Parameters.cpp` (80), `Programs.h` (79) and `PluginProcessor.cpp` (87). There are no third-party dependencies, and it needs C++14. — [GhostNoteAudio/CloudSeedCore](https://github.com/GhostNoteAudio/CloudSeedCore); [Readme](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/Readme.md)
- RNG and determinism:
  - Delay and tap randomisation is a seeded LCG (`LcgRandom`, `RandomBuffer::Generate(seed, count, crossSeed)`), which is deterministic.
  - **However**, `ModulatedDelay.h` and `ModulatedAllpass.h` set the initial `modPhase = 0.01 + 0.98 * std::rand() / (float)RAND_MAX`. That depends on the process-global C RNG state, so output is not reproducible across hosts or instance-creation orders. — [ModulatedDelay.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ModulatedDelay.h); [ModulatedAllpass.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ModulatedAllpass.h); [RandomBuffer.cpp](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/RandomBuffer.cpp)
- Allocation:
  - The delay buffers are fixed-size member arrays. `ModulatedDelay` and `MultitapDelay` each hold `float delayBuffer[192000 * 2]`, and `ModulatedAllpass` holds `float delayBuffer[19200]` ("100ms at 192Khz").
  - `ReverbChannel` has `TotalLineCount = 12` delay lines, and each diffuser has `MaxStageCount = 12` stages.
  - Changing `SeedTap`/`SeedDiffusion`/`SeedDelay`/`EqCrossSeed` calls `UpdateSeeds()` → `RandomBuffer::Generate`, which builds `std::vector`s with `push_back`. **That is a heap allocation if parameters are set on the audio thread.** — [ReverbChannel.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ReverbChannel.h); [AllpassDiffuser.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/AllpassDiffuser.h); [MultitapDelay.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/MultitapDelay.h)
- Denormals: a grep of `DSP/` for "denormal", "flush" and "FLT_MIN" found nothing, so there is no explicit denormal protection. — [CloudSeedCore DSP/](https://github.com/GhostNoteAudio/CloudSeedCore/tree/HEAD/DSP)
- Sample rate: `ReverbController(int samplerate)` and `SetSamplerate` propagate to the channels. Times are converted with `Ms2Samples`, and buffers are sized for up to 192 kHz. — [ReverbController.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ReverbController.h)
- An existing Rust wrapper (`cloudseedcore-rs`, MIT) proves the API surface: `ReverbController::new(sample_rate, block_size)`, `set_program(&DARK_PLATE)`, `set_parameter(ParamId…)` and a block-based `process`. It builds the C++ with `cxx_build` and defines `BUFFER_SIZE=1024`. — [cloudseedcore-rs README/build.rs](https://github.com/CrushedPixel/cloudseedcore-rs)
- Hardware porters hit CPU limits on a Daisy (about 5 delay lines max) and flash-size limits. — [PedalPCB forum](https://forum.pedalpcb.com/goto/post?id=222976)

**Signalsmith basics `reverb.h`**
- It is 409 lines. It includes `signalsmith-dsp/delay.h`, `mix.h` and `filters.h` (Signalsmith DSP library) plus `stfx-library.h`, and uses `std::mt19937 engine(seed)` for delay lengths and shuffles. `configure(sampleRate, maxBlockSize, channels)` must be called outside processing ("not while actively processing audio"). — [basics reverb.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/reverb.h); [basics README](https://github.com/Signalsmith-Audio/basics)

**Freeverb ports (fallback)**
- `freeverb` (509 lines), Firewheel's node (725 lines with plumbing), kira and STK scale the 44.1 kHz tunings by `sr/44100` (`adjust_length`). STK notes the tunings are "optimized for a sample rate of 44100 Hz". — [freeverb src/freeverb.rs](https://docs.rs/crate/freeverb/0.2.0/source/src/freeverb.rs); [firewheel freeverb.rs](https://docs.rs/crate/firewheel-nodes/0.14.0/source/src/freeverb/freeverb.rs); [STK FreeVerb.cpp](https://github.com/thestk/stk/blob/HEAD/src/FreeVerb.cpp)

**Dattorro and Griesinger plates**
- Clouds' `reverb.h` is 180 lines plus `fx_engine.h` at 302 lines (MIT), with LFOs set as `0.5f / 32000.0f`, so it is tuned for 32 kHz. lanceverb is 422 lines of Rust (`no_std`, about 90 kB RAM, libm). el-visio is 341 lines of C (`malloc` at init only). — [clouds reverb.h](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/reverb.h); [lanceverb README](https://docs.rs/crate/lanceverb/0.4.0/source/README.md); [el-visio verb.c](https://github.com/el-visio/dattorro-verb/blob/HEAD/verb.c)
- Developers report that Dattorro figure-8 loops are fragile to retune ("a delay or two often spoils the reverb"). — [KVR](https://www.kvraudio.com/forum/viewtopic.php?p=8090199)

**Airwindows Galactic3**
- About 803 lines across `Galactic3.cpp/.h/Proc.cpp`, with fixed `double` arrays such as `aIL[6480]`. The constructor seeds the floating-point-dither PRNG with `fpdL = rand()*UINT32_MAX` and then runs an xorshift per sample. Output therefore includes a host-dependent dither unless that is removed or seeded. — [Galactic3.cpp](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3.cpp); [Galactic3Proc.cpp](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3Proc.cpp); [Galactic3.h](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3.h)

### Inferences
- **Memory (computed from the constants above):** CloudSeedCore holds about 1.536 MB per `ModulatedDelay`, plus about 0.92 MB per 12-stage diffuser, for each of 12 lines. Add the multitap and input diffuser and that is roughly 32 MB per channel, or **about 64 MB per stereo instance**, allocated whether or not it is used. A Rust port should size buffers from the actual sample rate and the maximum parameter ranges at `initialize` time, then allocate once. That matters for wasm32 linear memory and for running several instances.
- **Effort estimates (rough, my estimate, not sourced):**
  - CloudSeedCore → idiomatic Rust: about 1.5–2.5k lines. Replace `std::rand()` with a seeded per-instance PRNG, move `UpdateSeeds` allocation to preallocated arrays, add FTZ or denormal guards, and add parameter smoothing. Roughly 3–6 engineer-days including golden-file tests against the C++ via cloudseedcore-rs.
  - Signalsmith basics reverb: about 0.5–1k lines of Rust including helpers (delay, Hadamard/Householder mixers, filters), roughly 2–4 days. To match C++ output bit for bit, either reimplement `mt19937` exactly or accept project-defined seeds.
  - fundsp: zero port. The integration cost is wrapping an `An<…>` graph in the plugin's processor and calling `allocate()` and `set_sample_rate()` off the audio thread.
- **Determinism pitfalls common to all candidates:**
  1. Hidden C `rand()` in initialisation (CloudSeedCore, Airwindows).
  2. Thread-local or OS RNGs (`basic-reverb`).
  3. Floating-point differences across SIMD widths and targets. fundsp uses SIMD for some delay paths (`SIMD_N` in `delay.rs`), so native-vs-wasm bit-exactness should be tested rather than assumed.
  4. Denormal behaviour differs between x86 FTZ/DAZ settings in a DAW and wasm, which has no FTZ. Explicit denormal flushing or a tiny DC/noise offset is safer than relying on CPU flags.
  5. Sample-rate-dependent tunings (Freeverb's 44.1 kHz and Clouds' 32 kHz constants) change the character at 48 or 96 kHz unless they are rescaled.

### Gaps
- No measured CPU benchmarks (e.g. ns/sample) for fundsp's reverbs versus CloudSeedCore versus Freeverb were found.
- Whether fundsp output is bit-identical between x86_64 and wasm32 was not tested.
- No line-by-line review of CloudSeedCore's parameter-smoothing or zipper-noise behaviour was done.
- talaviram's fork changes (used by cloudseedcore-rs) were not diffed against upstream.
