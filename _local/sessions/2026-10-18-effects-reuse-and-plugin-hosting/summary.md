# Port permissive effects; let REAPER host plugins

**For P1, enth should port its reverb and compressor from permissively licensed code into its own deterministic Rust modules. It should not host third-party plugins.** The reverb to port is Signalsmith Audio's MIT-licensed `basics/reverb.h`. It is a 409-line design (a multichannel diffuser feeding a feedback delay network, or FDN), last committed on 2026-09-30. It seeds all of its randomness, and its parameters are already in real units (room in ms, RT20, cut-offs in Hz). fundsp's four deterministic reverbs (MIT OR Apache-2.0) are the zero-port fallback and the A/B reference. CloudSeedCore (MIT) is the permissive algorithm with the best reputation, so it becomes the "lush" upgrade after P1. That needs a Rust port that removes its process-global `std::rand()` and its fixed buffers of about 64 MB per stereo instance. There is no maintained, permissively licensed Rust compressor, so enth should write a few hundred lines of its own. The model is Julius O. Smith's Faust `compressor_mono` (STK-4.3, an MIT-style licence), with the MIT Rust compressor in web-audio-api-rs as a cross-check. enth could host the owner's reverb using clack-host (MIT/Apache, 0.2.0 on 2026-09-12). By the sub-researchers' estimates, though, a minimal CLAP-only slot costs 4–8 weeks, against a few days to port a reverb. It would also break bit-exact renders, the CLI and preset sharing. It adds nothing that the same reverb on the REAPER track after enth doesn't already give. What users actually ask for is a persistent global FX-bypass switch. Everything copyleft stays reference-only: Vital, Surge XT, Valley Plateau, MVerb, JUCE 9's reverb (now AGPL), DaisySP's LGPL compressor, chowdsp's DSP modules and most Faust compressors. Nothing has yet been compiled for wasm32 or compared by ear. The reverb pick is provisional until a one- to two-day spike confirms it.

## Signalsmith's 409-line MIT reverb is the one to own

There are fewer permissive reverbs than it first appears. In Rust, **fundsp is the only actively developed, permissively licensed crate that ships more than a Freeverb**. Its 0.23.0 release (2026-01-07) includes four reverbs ([prelude.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/prelude.rs); [crates.io](https://crates.io/crates/fundsp)):

| fundsp reverb | Design |
|---|---|
| `reverb_stereo` | 32-line FDN |
| `reverb2_stereo` | "32-channel hybrid FDN" with nested allpasses and delay modulation |
| `reverb3_stereo` | Allpass loop |
| `reverb4_stereo` | Two cascaded 16-line FDNs |

Nearly every other permissive Rust reverb is either a Freeverb port (irh's `freeverb`, Firewheel's node, kira, fyrox-sound, rustysynth) or a Dattorro plate such as `lanceverb` ([freeverb](https://docs.rs/crate/freeverb/0.2.0/source/src/lib.rs); [firewheel-nodes](https://docs.rs/crate/firewheel-nodes/0.14.0/source/src/freeverb/mod.rs); [lanceverb](https://docs.rs/crate/lanceverb/0.4.0/source/README.md)). The best-known Rust Dattorro (`synfx-dsp`) and the Rust port of Vital's reverb (`vitalium_verb_dsp`) are both **GPL-3.0-or-later** ([synfx-dsp dattorro.rs](https://docs.rs/crate/synfx-dsp/0.5.6/source/src/dattorro.rs); [vitalium_verb_dsp](https://crates.io/crates/vitalium_verb_dsp)). Outside Rust, these are the verified permissive sources worth porting:
- CloudSeedCore
- Signalsmith Basics
- the plate in Mutable Instruments' Clouds
- Airwindows
- the MIT and STK-4.3 functions in Faust's `reverbs.lib`
- fverb (BSD-2)
- el-visio's Dattorro in C (MIT)

Sources: [reverbs.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/reverbs.lib); [fverb](https://github.com/jpcima/fverb); [el-visio](https://github.com/el-visio/dattorro-verb).

| Candidate | Licence | Health on 2026-10-18 | Determinism | wasm32 (inferred, not compiled) | Cost to use | Role |
|---|---|---|---|---|---|---|
| [Signalsmith Basics `reverb.h`](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/reverb.h) | MIT | Last commit 2026-09-30, tag 1.2.1; one author | Seeded `std::mt19937` | Yes once ported | Port 409 lines plus small MIT helpers; est. 2–4 days | **P1 pick** |
| [fundsp `reverb2_stereo` / `reverb4_stereo`](https://docs.rs/crate/fundsp/0.23.0/source/src/prelude.rs) | MIT OR Apache-2.0 | 0.23.0 on 2026-01-07; last commit 2026-03-03; ~1.2k stars | Seeded spline noise; constant delay tables | Likely, with `default-features = false` | No port needed; heavy dependency | Fallback and A/B reference |
| [CloudSeedCore](https://github.com/GhostNoteAudio/CloudSeedCore) | MIT; credit requested | 7 commits in total, last 2024-09-22; 79 stars | `std::rand()` in the modulation phase | No through `cloudseedcore-rs` (needs C++14); yes once ported | 2,336 lines of C++; est. 3–6 days | "Lush" mode after P1 |
| Freeverb ports ([`freeverb`](https://crates.io/crates/freeverb), Firewheel, kira) | MIT; the original Freeverb is public domain | `freeverb` 0.2.0 on 2025-10-29 | No RNG | Yes | None | Anchor in the listening test only |
| Plates: [lanceverb](https://crates.io/crates/lanceverb) and [Clouds](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/reverb.h) | lanceverb: MIT plus an Allocore BSD notice. Clouds: MIT | lanceverb: one release, 2025-06-15 | No RNG found | lanceverb is `no_std` plus libm | 422 and ~480 lines | Alternative plate character |
| [Airwindows Galactic3](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3.cpp) | MIT | Last commit 2026-10-17; repo is read-only | Dither seeded from `rand()` | Yes once ported | ~800 lines per plugin | Reference for later |
| synfx-dsp, vitalium_verb_dsp, Valley Plateau, MVerb, Vital, Surge XT, `juce::Reverb`, DaisySP `ReverbSc` | GPL, AGPL or LGPL | — | — | — | — | Reference-only |

**Three things tip the choice from fundsp to a Signalsmith port.**

1. **Control.** Basics is published both as plugins and as "re-usable open-source (MIT) C++ classes". Its reverb's parameters are dry, wet, roomMs, rt20, early, detune, lowCutHz, highCutHz, lowDampRate and highDampRate. These are the same real units that enth's patches store ([README](https://github.com/Signalsmith-Audio/basics); [reverb.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/reverb.h)). fundsp's reverbs, by contrast, take room size, time and damping as constructor arguments ([prelude.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/prelude.rs)). Nobody has checked whether decay or damping can then follow a macro or a knob sweep without rebuilding the graph.
2. **Health.** Basics was last committed on 2026-09-30, and the Signalsmith DSP library underneath it tagged v1.7.3 on 2026-10-13 ([commits](https://github.com/Signalsmith-Audio/basics/commits/HEAD); [dsp](https://github.com/Signalsmith-Audio/dsp)). fundsp's last commit (2026-03-03) falls outside the project's six-month activity criterion, and its unreleased 0.24 work has been quiet for seven months ([CHANGES.md](https://github.com/SamiPerttu/fundsp/blob/HEAD/CHANGES.md)).
3. **Scope.** fundsp has **no compressor, only a look-ahead limiter** ([README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)). In P1 it would therefore buy exactly one reverb, at the price of a dependency heavy with generics. With default features its wasm32 dependency tree has 81 crates. Its API breaks often enough that pinning an exact version is mandatory ([lib.rs](https://lib.rs/crates/fundsp)).

fundsp still has real strengths, which is why it is the fallback and the A/B reference rather than a rejected option. It needs no porting, its README documents a "deterministic pseudorandom phase" system, and it allocates only in `allocate()` and `set_sample_rate()` ([README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md); [delay.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/delay.rs)).

CloudSeedCore is the reverb people actually praise, which makes it the right *second* reverb rather than the first. It is the core of the commercial Cloud Seed 2.0, released on 31 May 2024 at £39/£69 ([KVR](https://www.kvraudio.com/news-print.php?id=60893)). A porter who put it on Daisy hardware said it "seems comparable in sound quality to some of the Strymon reverb algorithms" ([PedalPCB](https://forum.pedalpcb.com/goto/post?id=222976)). The code has several problems, though:
- The repo is **stale**: seven commits in total, the last on 2024-09-22 ([GitHub](https://github.com/GhostNoteAudio/CloudSeedCore)).
- It is C++14.
- Every `ModulatedDelay` holds a fixed `float delayBuffer[192000 * 2]`. From these constants the sub-researcher computed about 64 MB per stereo instance, allocated whether used or not ([ReverbChannel.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ReverbChannel.h); [MultitapDelay.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/MultitapDelay.h)).
- The existing Rust wrapper, `cloudseedcore-rs`, compiles the C++ through `cxx`. It cannot sit in a wasm32 core crate, and AGENTS.md already requires asking before adding a C++ toolchain ([cloudseedcore-rs](https://github.com/CrushedPixel/cloudseedcore-rs)).

The upstream README also asks users to "credit Ghost Note Audio" and reserves the right to ask them to stop using the "Cloud Seed" name ([Readme](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/Readme.md)). A Rust port of 1.5–2.5k lines is a sensible S2 task, with credit to Ghost Note Audio and no Cloud Seed branding.

**All the quality evidence is opinion, and none of it is measured.**
- Freeverb, and with it JUCE's reverb and most Rust game-engine reverbs, is described as sounding "a bit metallic and raw" ([JUCE forum](https://forum.juce.com/t/the-all-pass-filter-in-juce-reverb-is-a-comb-filter/53202)) and as having a "VERY metallic sound for sharp transients" ([Freeverb3 tips](https://freeverb3-vst.sourceforge.io/tips/reverb.shtml)).
- Sean Costello of Valhalla DSP explains why chains of series allpasses ring metallically ([Valhalla DSP](https://valhalladsp.com/2011/01/21/reverbs-diffusion-allpass-delays-and-metallic-artifacts/)).
- On KVR, developers call FDNs "far easier to get right" than Dattorro's figure-8 loop, where changing "a delay or two often spoils the reverb completely". They also warn that an FDN does not automatically give a colourless tail ([KVR](https://www.kvraudio.com/forum/viewtopic.php?p=8090199)).

That points to a modern FDN (Signalsmith or fundsp) or CloudSeed for pads and plucks, and away from Freeverb. Only a blind, level-matched A/B in REAPER can settle which one enth ships.

## No Rust compressor earns a dependency, so write one from textbook sources

**No permissively licensed Rust compressor is both maintained and used.**
- fundsp offers only `limiter`/`limiter_stereo` ([README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md)).
- augmented-audio's `audio-processor-dynamics` has had no release since 2024-08-26 and had 36 downloads in 90 days ([crates.io](https://crates.io/crates/audio-processor-dynamics)).
- Firewheel's node set has no compressor, chorus or distortion ([firewheel-nodes](https://github.com/BillyDM/firewheel/tree/HEAD/crates/firewheel-nodes/src)).
- web-audio-api-rs (MIT, 1.7.0 on 2026-08-08) does contain a Rust `DynamicsCompressorNode` ([dynamics_compressor.rs](https://github.com/orottier/web-audio-api-rs/blob/HEAD/src/node/dynamics_compressor.rs); [crates.io](https://crates.io/crates/web-audio-api)). But the crate drags 128 crates into a wasm32 build, including cpal, and needs ALSA on Linux. Port the node's code; do not depend on the crate.

The other references divide sharply by licence:
- In Faust's `compressors.lib`, **22 functions are GPL-3.0-only**, 1 is MIT, and only 4 carry STK-4.3. Those four are Julius O. Smith's `compressor_mono`, `compressor_stereo`, `compressor_lad_mono` and `compression_gain_mono` ([compressors.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/compressors.lib)).
- DaisySP's compressor sits in the LGPL submodule, not the MIT core ([DaisySP-LGPL](https://github.com/electro-smith/DaisySP-LGPL)).
- chowdsp's compressor is GPLv3 ([chowdsp_utils](https://github.com/Chowdhury-DSP/chowdsp_utils/blob/HEAD/README.md)).
- lamb-rs is AGPL-3.0 ([lamb-rs](https://github.com/magnetophon/lamb-rs)).
- Signalsmith's MIT `dynamics.h` looks modern, with BLEP-style soft knees and an adaptive-attack envelope follower. But its history starts on 2026-09-25, so it is three weeks old ([dynamics.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/dynamics.h)).
- Airwindows' MIT ButterComp2 and Pressure4–6 are "character" designs, useful later ([Airwindows](https://github.com/airwindows/airwindows/tree/HEAD/plugins/LinuxVST/src)).

The P1 compressor should therefore be **a few hundred lines of in-house code in `enth-dsp`**. It needs a feed-forward, stereo-linked detector with attack and release, a soft-knee gain computer and make-up gain, following JOS's textbook `compressor_mono`. web-audio-api-rs serves as a Rust cross-check. It should have **no look-ahead**. CLAP allows latency to change only while the plugin is being activated; otherwise the plugin must ask the host to restart it ([CLAP latency.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h)). A zero-latency compressor and reverb keep P1 out of latency reporting altogether. Hand-porting the Faust function beats shipping Faust compiler output, because Debian's copyright summary does not state the licence of generated code ([Debian](https://metadata.ftp-master.debian.org/changelogs/main/f/faust/stable_copyright)).

The rest of the ecosystem confirms a pattern: **shared Rust crates cover FFT, resampling, I/O and filters, and effect DSP gets written per project.** BillyDM's curated list shows community plugins hand-writing their effects ([awesome-audio-dsp](https://github.com/BillyDM/awesome-audio-dsp/blob/HEAD/sections/OPEN_SOURCE_PLUGINS_AND_SOFTWARE.md)). A March 2026 overview still finds "no JUCE equivalent yet" ([Joel Löf](https://joellof.com/blog/audio-plugin-development-cpp-rust-web/)). The commit and release record sorts the Rust crates into three groups:

| Status | Crates |
|---|---|
| Healthy | rustfft 6.4.1, realfft, rubato 5.0.1 (2026-10-01), cpal, and `biquad` 0.6.0 (2026-03-22, `no_std`, a 4-crate tree) |
| Usable but evolving | fundsp, Firewheel, and nice-plug (which calls itself "experimental") |
| Stale | dasp (no release since 2020), SolarLiner's valib (git-only since 2024-12; the crates.io name belongs to someone else), synfx-dsp (GPL, needs nightly), knyst, glicol_synth and augmented-audio |

Sources: [rubato](https://crates.io/crates/rubato); [biquad](https://crates.io/crates/biquad); [nice-plug](https://docs.rs/crate/nice-plug/0.4.2); [valib](https://github.com/SolarLiner/valib/blob/HEAD/README.md).

The P1 modules beyond effects follow the same logic:
- **Multimode filter:** write it in-house as a TPT state-variable filter after Cytomic's paper, which fundsp and DaisySP already follow ([fundsp README](https://github.com/SamiPerttu/fundsp/blob/HEAD/README.md); [DaisySP](https://github.com/electro-smith/DaisySP/tree/HEAD/Source)).
- **Oscillators:** the MIT polyBLEP code in Mutable Instruments' stmlib and in DaisySP is the reference for P1. Signalsmith's MIT `elliptic-blep` is for a later, synth-grade upgrade ([stmlib](https://github.com/pichenettes/stmlib/blob/HEAD/dsp/filter.h); [elliptic-blep](https://github.com/Signalsmith-Audio/elliptic-blep)).

After P1, the permissive menu is rich:
- **Chorus, flanger and phaser:** DaisySP's MIT core or Signalsmith's `chorus.h` ([DaisySP Effects](https://github.com/electro-smith/DaisySP/tree/HEAD/Source/Effects)).
- **Anti-aliased saturation:** Faust's `aanl.lib`, whose 37 functions are all MIT ([aanl.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/aanl.lib)), with oversampling modelled on Signalsmith's `rates.h`.
- **Safety limiter:** Signalsmith's `limiter.h`.

## Hidden randomness, not licences, decides what can be reused unchanged

A LICENSE file takes minutes to check. The reuse failures that would actually hurt enth are behavioural. enth defines `render()` as a pure function of patch, performance, sample rate, block size, seed and engine version. It must be bit-exact on the same build and platform, and equal within a tolerance across platforms and wasm32 (docs/design.md §5). Any code that reads a global RNG, allocates when a parameter changes, or relies on CPU denormal flags breaks golden tests, scoring and serverless duels.

**Three candidates hide exactly that kind of randomness:**
- CloudSeedCore's `ModulatedDelay` and `ModulatedAllpass` set `modPhase = 0.01 + 0.98 * std::rand() / RAND_MAX`, so the output depends on process-global C RNG state and on the order instances are created ([ModulatedDelay.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ModulatedDelay.h)).
- Airwindows' Galactic3 seeds its floating-point dither with `rand()*UINT32_MAX` ([Galactic3.cpp](https://github.com/airwindows/airwindows/blob/HEAD/plugins/LinuxVST/src/Galactic3/Galactic3.cpp)).
- The MIT `basic-reverb` crate chooses diffuser delays and polarities with `rand::thread_rng()` ([diffusion_step.rs](https://docs.rs/crate/basic-reverb/0.1.3/source/src/diffuser/diffusion_step.rs)).

By contrast, fundsp's modulation is `spline_noise(seed, t)` over constant tables ([math.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/math.rs)), and Signalsmith seeds `std::mt19937` explicitly in `updateLengths(seed, …)` ([reverb.h](https://github.com/Signalsmith-Audio/basics/blob/HEAD/include/signalsmith-basics/reverb.h)). The fix in a port is mechanical: replace the upstream generator with enth's seeded PRNG, derived from the patch hash and module id as AGENTS.md requires. Signalsmith's author says the design is "robust enough that I could throw a random-number generator at all the delay times" ([Let's Write a Reverb](https://signalsmith-audio.co.uk/writing/2021/lets-write-a-reverb/)), so seeds chosen by the project should not cost quality. A first golden test against the C++ is still easier if the port reproduces mt19937 exactly.

Allocation and numerics are the second filter:
- **Allocation.** enth's `Module::prepare` may allocate and `process` never does. fundsp's `allocate()`/`set_sample_rate()` and Signalsmith's `configure()` (which must not run "while actively processing audio") map straight onto `prepare` ([delay.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/delay.rs); [Basics README](https://github.com/Signalsmith-Audio/basics/blob/HEAD/README.md)). CloudSeedCore, however, rebuilds `std::vector`s with `push_back` whenever a seed parameter changes, so a port must move that work into preallocated arrays ([ReverbChannel.h](https://github.com/GhostNoteAudio/CloudSeedCore/blob/HEAD/DSP/ReverbChannel.h)).
- **Denormals.** CloudSeedCore has no denormal protection anywhere in `DSP/` ([DSP/](https://github.com/GhostNoteAudio/CloudSeedCore/tree/HEAD/DSP)). Wasm has no flush-to-zero mode (an inference), so `enth-dsp` should flush denormals explicitly in code rather than through CPU flags.
- **Fixed tunings need rescaling.** Freeverb's delays are "optimized for a sample rate of 44100 Hz" ([STK FreeVerb.cpp](https://github.com/thestk/stk/blob/HEAD/src/FreeVerb.cpp)), and the Clouds LFO rates are hard-coded for 32 kHz ([Clouds reverb.h](https://github.com/pichenettes/eurorack/blob/HEAD/clouds/dsp/fx/reverb.h)).
- **SIMD.** fundsp's delay paths use SIMD widths (`SIMD_N`). That fits the cross-platform tolerance clause, but it has to be tested rather than assumed ([delay.rs](https://docs.rs/crate/fundsp/0.23.0/source/src/delay.rs)).

**No candidate was compiled for `wasm32-unknown-unknown`**, because the sub-researchers could not download that target's standard library. The only proxy is dependency trees, from the sub-researcher's local `cargo tree` on 2026-10-18:

| Crate | Crates in its wasm32 dependency tree |
|---|---|
| biquad | 4 |
| rustfft | 9 |
| rubato | 24 |
| firewheel-nodes | 49 |
| fundsp (default features) | 81 |
| web-audio-api | 128 |

`cloudseedcore-rs` needs a C++14 toolchain through `cxx`, so it will very likely not build for this target at all ([cloudseedcore-rs](https://github.com/CrushedPixel/cloudseedcore-rs)). Porting into `enth-dsp` means wasm32 support no longer depends on someone else's crate. Instead, `cargo xtask wasm-check` checks it on every commit.

## Licence traps hide inside repos that look permissive

The riskiest reuse mistakes come from projects whose headline licence is permissive but whose useful file is not:
- **JUCE.** Since JUCE 9 its modules are "dual-licensed under the AGPLv3 and the commercial JUCE licence". `juce::Reverb`, a Freeverb derivative, is therefore off-limits. Reimplement Freeverb from the public-domain original instead ([JUCE LICENSE.md](https://github.com/juce-framework/JUCE/blob/HEAD/LICENSE.md); [juce_Reverb.h](https://github.com/juce-framework/JUCE/blob/HEAD/modules/juce_audio_basics/utilities/juce_Reverb.h)).
- **DaisySP.** The core is MIT, but its compressor, Moog ladder and ReverbSc live in the **LGPL-2.1 submodule** ([DaisySP .gitmodules](https://github.com/electro-smith/DaisySP/blob/HEAD/.gitmodules)).
- **chowdsp_utils.** It licenses each module separately: compressor, EQ, filters, reverb and waveshapers are GPLv3, while core, math and SIMD are BSD ([chowdsp README](https://github.com/Chowdhury-DSP/chowdsp_utils/blob/HEAD/README.md)).
- **Faust.** It declares a licence per function. `vital_rev` and `kb_rom_rev1` are GPL-3.0-only, and `springreverb` declares no licence at all ([reverbs.lib](https://github.com/grame-cncm/faustlibraries/blob/HEAD/reverbs.lib)).
- **Surge's `sst-basic-blocks`.** It is GPL3 apart from headers explicitly marked MIT ([README](https://github.com/surge-synthesizer/sst-basic-blocks/blob/HEAD/README.md)).
- **SolarLiner's valib.** Its library `src/` is MIT, but its plugins are GPLv3 ([valib README](https://github.com/SolarLiner/valib/blob/HEAD/README.md)).
- **Mutable Instruments.** The STM32 code is MIT, but the AVR code is GPL-3.0 ([eurorack](https://github.com/pichenettes/eurorack)).
- **nih-plug.** Its bundled `plugins/` "may be licensed under the GPLv3" ([nih-plug README](https://github.com/robbert-vdh/nih-plug/blob/HEAD/README.md)).
- **Signalsmith's ADC 2021 `reverb-example-code`.** It has no licence file at all ([repo](https://github.com/Signalsmith-Audio/reverb-example-code)). That makes the MIT `basic-reverb` crate, a port of it, a provenance risk on top of its non-determinism. Use the MIT `basics` version instead.

The plugin-format side has loosened in enth's favour. Steinberg relicensed the VST 3 SDK under MIT with version 3.8 on 2025-10-29, and the vst3sdk LICENSE.txt confirms it ([Sonic State](https://sonicstate.com/news/2025/10/30/vst-3-now-available-under-mit-license); [vst3sdk LICENSE.txt](https://github.com/steinbergmedia/vst3sdk/blob/master/LICENSE.txt)). A local `cargo tree` of **nice-plug 0.4.2 shows it depending on the MIT/Apache `vst3` 0.3.0 crate and `clap-sys` 0.5.0, not the GPLv3 `vst3-sys`** ([crates.io nice-plug](https://crates.io/crates/nice-plug)). That answers the "verify in Phase 0" note in research.md §3, though it should be re-checked at whatever version P0 pins.

Reuse also creates paperwork. MIT, BSD and STK-4.3 all require keeping the notice; the STK-4.3 text says "the above copyright notice and this permission notice shall be included in all copies or substantial portions" ([stk-4.3.0.md](https://github.com/grame-cncm/faustlibraries/blob/HEAD/licenses/stk-4.3.0.md)). `lanceverb` carries a separate Allocore BSD-style licence for Lance Putnam's upstream code ([ORIGINAL_LICENSE.md](https://docs.rs/crate/lanceverb/0.4.0/source/ORIGINAL_LICENSE.md)). ADR-0004's allowed list governs *dependencies* through `cargo deny`. It does not yet say which licences are acceptable for *ported source*, which `cargo deny` cannot see. It also does not name STK-4.3, 0BSD or public domain. Both gaps belong in an ADR amendment and a `THIRD_PARTY_NOTICES.md`.

## Hosting the owner's reverb costs weeks and breaks the deterministic core

The owner's question was: "Wouldn't it be cool if we could plug in some external reverb so that we don't have to build it?" Plugin-in-plugin hosting is a real product category, but it belongs to *chainers*, not synths:
- Blue Cat PatchWork ($99) hosts up to 64 plug-ins with built-in latency compensation ([KVR](https://www.kvraudio.com/marketplace/patchwork-by-blue-cat-audio)).
- DDMF Metaplugin maps DAW automation to hosted parameters through an assignable list that a reviewer found "more fiddly than direct DAW parameter access" ([Sound On Sound](https://www.soundonsound.com/reviews/ddmf-metaplugin-3)).
- Kushview Element (GPL) and Carla (GPL-2.0+) do the same in open source. Carla still labels its out-of-process bridges experimental ([Element](https://www.kvraudio.com/product/element-by-kushview); [Carla](https://github.com/falkTX/Carla)).

The two closest synth-world precedents both protect preset portability. **Kilohearts hosts only its own closed Snapin format**, so a Snap Heap preset "works even if you don't own every Snapin" ([Kilohearts](https://kilohearts.com/blog/wtf_is_a_snap_heap)). PluginGuru's Unify, an instrument built around hosting, ships 400 factory patches that "use ONLY the bundled and built-in plug-ins". A user in the same thread warned that third-party dependencies create "very segmented user niches" ([KVR Unify thread](https://www.kvraudio.com/forum/viewtopic.php?p=7632550)). None of the surveyed chainers documents protection against a hosted plugin crashing in-process. Bitwig, the reference for sandboxing, presents isolation as a resource trade-off across five hosting modes ([Bitwig user guide](https://bitwig.com/userguide/latest/vst_plug-in_handling_and_options)).

The Rust tooling has only just become viable, and only for CLAP:
- **clack-host** (MIT/Apache) reached crates.io on 2026-05-03 and released 0.2.0 on 2026-09-12. It calls itself "feature-complete" but warns of breaking changes. On crates.io only `clack-extensions` and `plughost-formats` depend on it ([GitHub](https://github.com/prokopyl/clack); [crates.io](https://crates.io/crates/clack-host)).
- **vst3-host 0.10.0** has been "exercised against real plugins on macOS" only. Its embedded egui editor is macOS-only, and its default playback path "locks on the audio callback" ([docs.rs](https://docs.rs/crate/vst3-host/latest)).
- **LV2:** the last release of `livi` was on 2024-11-12 ([crates.io](https://crates.io/crates/livi)).

No Rust crate is documented as hosting plugins *from inside another plugin*.

The audio path itself is easy, because CLAP's audio thread is "symbolic" and the outer plugin can call the inner plugin's `process()` ([thread-check.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h)). Everything around it is hard:
- **Main-thread callbacks.** The inner plugin's `request_callback`, timers and file-descriptor watches must be relayed to the DAW's main thread, and Linux GUIs depend on timer-support and posix-fd-support ([timer-support.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/timer-support.h); [posix-fd-support.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/posix-fd-support.h)). nice-plug's docs do not mention exposing these host extensions.
- **Editor windows.** X11 embedding uses XEmbed, and Wayland supports only floating windows ([gui.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/gui.h)). yabridge documents focus stealing, REAPER's FX window intercepting drag-and-drop, HiDPI mismatches and Xorg's 256-client limit ([yabridge](https://github.com/robbert-vdh/yabridge)).
- **Latency.** Loading a reverb with different latency forces a host restart ([latency.h](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h)).
- **Automation.** enth's 64 automation slots point at `ParamAddr`s, and a foreign plugin's parameters are not `ParamAddr`s.

**Determinism is the decisive objection.** Sound On Sound explains that renders fail to null because "some plug-ins behave differently on each playthrough" ([SOS, April 2025](https://www.soundonsound.com/sound-advice/q-why-arent-my-rendered-and-live-plugin-sounds-nulling)). VSL's MIRacle reverb exports are "never precisely the same twice" because of pseudo-random modulation ([VSL forum](https://forum.vsl.co.at/topic/59562/miracle-reverb-never-the-same-twice)). A hosted reverb would sit outside the scored path, would force the CLI to become a plugin host that requires the user to own the plugin, and would break the "a challenge is just a seed" premise of serverless duels.

What users do ask for is a switch. In a VI-CONTROL thread about effects in presets, players valued designer-voiced effects, but they also asked for "a global control for turning preset time based FX on and off", which the Access Virus, Hive 2 and Repro-1/5 already have ([VI-CONTROL](https://vi-control.net/community/threads/to-drench-or-not-to-drench-fx-in-synth-presets-discussion.120519/)). Nobody asked for hosting.

| Option | Effort (sub-researcher estimate, unvalidated) | Effect on determinism and scoring | Main risk |
|---|---|---|---|
| (b) User's reverb on the REAPER track, plus a global FX bypass in enth | 0.5–2 days | None | Presets don't carry the user's reverb; the CLI renders dry |
| (a) Built-in deterministic reverb | 1–3 weeks to write from scratch; a few days if ported as above | Can sit inside the scored path | Quality against commercial reverbs |
| (d) (a)+(b) now; later a CLAP-only, in-process, floating-editor post-FX slot via clack-host | (a) plus 4–8 weeks, likely plus nice-plug fork work | Slot sits after a dry scoring tap, is flagged non-reproducible, and the CLI bypasses it | Main-thread relay, Linux timers and fds, restart glitches, being blamed for third-party crashes |
| (c) Full hosting: CLAP + VST3 (+ LV2), embedded editors, out-of-process isolation, three OSes | 3–6+ months, then ongoing | Breaks reproducibility for any patch that uses it | Immature crates, embedding quirks; effectively a PatchWork-class product inside a synth |

## Conclusion

The owner's instinct (reuse a reverb instead of building one) is right, but plugin hosting is the most expensive way to act on it. The cheap kinds of reuse are porting MIT algorithms, which takes days, and the DAW's own FX chain, which already exists. The DAW chain already gives the user's reverb latency compensation, automation, crash handling and a working GUI.

enth's determinism contract also changes what "reuse" means. It is no longer "add a crate". It becomes "port and own, keep the notices, and replace the RNG". The result is that `enth-dsp` itself becomes the reusable asset. That is the same boundary architecture.md already reserves for shipping individual modules as their own plugins in S3.

Only effects the project owns can sit inside the scored path, and Recreate's top difficulty randomises effects. A built-in deterministic reverb is therefore a requirement of the game, not a convenience. Any future third-party slot can only ever live after a dry scoring tap.

## Corrections to the sub-reports

The sub-reports are unchanged. Where they conflict with each other or with the project docs, these corrections apply.

| Where | What it says | Correction |
|---|---|---|
| 01, project context | The shell is "nih-plug" | The shell is nice-plug (ADR-0001). This does not affect the reverb findings. |
| 01, project context | "Bit-for-bit determinism" | design.md §5 requires bit-exactness only on the same build and platform, and a tolerance on a feature summary across platforms and wasm32. fundsp's SIMD paths are therefore a tolerance question, not a blocker. |
| 02, Q5 | Use nice-plug's `SmoothingStyle` smoothers | The engine (L2) cannot depend on the shell (L8), and CLI renders must equal plugin renders. Smoothing belongs in `enth-dsp`/`enth-engine`. |
| 02, Q1 | fundsp has had "19 breaking releases" | The figure comes from lib.rs, which 02 itself found stale (it showed 0.20.0). crates.io lists 37 versions. The churn is real; the count is approximate. |
| 02, Q5 | "STK-4.3 wording was not quoted" | 01 quotes it: MIT-style, with notices kept. |
| 03, §2–3 | Cites nice-plug 0.3.0 docs | 02 inspected 0.4.2. The hosting-related API should be re-checked at whatever version P0 pins. |
| docs/research.md §3 | Whether nice-plug uses the MIT `vst3` crate needs verifying in Phase 0 | 02's `cargo tree` shows that 0.4.2 depends on `vst3` 0.3.0 (MIT/Apache). Re-check at the pinned version. |
| docs/research.md §1 | HexoSynth: "check licence" | Its DSP crate, `synfx-dsp`, is GPL-3.0-or-later, and its GitHub repo is no longer publicly reachable. Reference-only. |

## Open checks

| # | Check | Why it matters | How | Needed before |
|---|---|---|---|---|
| 1 | **wasm32 builds** | Nothing was compiled; the session could not fetch the target's rust-std | `cargo xtask wasm-check` on the Signalsmith port prototype, on `fundsp` with `default-features = false`, and on `biquad` | Choosing the reverb |
| 2 | **Blind A/B listening test** | All quality evidence is forum opinion | At 48 kHz, level-matched, render a pad, a pluck and a transient through: the Signalsmith Basics reverb (its plugin, in REAPER); fundsp `reverb2_stereo` and `reverb4_stereo`; Cloud Seed (through `cloudseedcore-rs` in a `spike-*` crate after asking, or the MIT Aether LV2); and a Freeverb anchor | Choosing the reverb |
| 3 | fundsp runtime parameters | Its reverbs take room size, time and damping as constructor arguments | Read the 0.23 source; test a live change under `assert_no_alloc` | Only if fundsp is chosen |
| 4 | Signalsmith parameter changes | Does a `roomMs` change re-run `updateLengths`, allocate or click? | Read `reverb.h` and `delay.h`; test the port under `assert_no_alloc` | During the port |
| 5 | Golden tests and denormals across Linux, Windows and wasm32 | design.md §5 contract | Golden hashes on the same platform; feature-summary tolerance across platforms; explicit denormal flushing | Gate G1 |
| 6 | CPU cost per reverb and compressor instance | No benchmarks were found | `enth bench` against the G1 CPU budget | During P1 |
| 7 | web-audio-api-rs compressor provenance and look-ahead | Its Chromium-style defaults hint at upstream lineage; any look-ahead would add latency | Read the file header and history | Before porting from it |
| 8 | STK-4.3 notice and the licence of Faust's generated code | Notice obligations; the licence of generated code is unstated | Copy the licence text into the notices file; hand-port rather than ship compiler output | Shipping the compressor |
| 9 | GitHub stars, issues and maintainers | GitHub API was blocked; Codeberg (nice-plug, synfx-dsp) was unreachable | `gh api` locally | Low priority |
| 10 | nice-plug at the pinned version | Its `vst3` dependency and licence; whether it exposes `request_callback`, timer-support, posix-fd-support and child windows to plugin code | `cargo tree`; read the source | P0 for the licence; the rest only before any hosting spike |
| 11 | Whether REAPER's CLAP host implements timer-support and posix-fd-support | Relaying an inner plugin's Linux GUI depends on it | One-day spike | Only before hosting |
| 12 | Unverified licences: Pirkle's book code, Teensy Audio, Maximilian, signalo, `rtsan-standalone-rs`, Faust `springreverb` | Excluded until verified | Read LICENSE files | Before using any of them |
| 13 | Dattorro patent claim (single, unverified source) | Legal hygiene | Low-priority check | Only if a Dattorro plate ships |
| 14 | Community opinion: KVR "best free reverb" threads, Reddit, Rust Audio Discord | These sources could not be fetched | Read manually | Optional |
| 15 | Effort estimates | Every day and week figure here is unvalidated | Record actual effort in the next session's summary | — |
| 16 | Hosting precedents: Vital, Serum 2, Phase Plant; how Unify and PatchWork handle missing plugins | Completes the precedent picture | Vendor docs and manuals | Only before hosting |
| 17 | talaviram's CloudSeedCore fork, vendored by `cloudseedcore-rs` | Undiffed changes | Diff against upstream | S2 CloudSeedCore port |

## Recommendations for P1

| Need | Pick | Licence | Why | Fallback |
|---|---|---|---|---|
| **Reverb** | Port Signalsmith Basics `reverb.h` and its MIT helpers (`delay.h`, `mix.h`, `filters.h`) into `enth-dsp` as the `reverb` module. Replace `mt19937` with enth's seeded PRNG (or reproduce mt19937 for the first golden test), flush denormals in code, and allocate in `prepare` | MIT (© 2022–2025 Signalsmith Audio / Geraint Luff; DSP library © 2021) | Actively maintained; parameters already in real units; deterministic once seeded; small; adds no core dependency; the same family offers a limiter and chorus later | fundsp 0.23 `reverb2_stereo` or `reverb4_stereo`, `default-features = false`, pinned to an exact version and wrapped behind `Module` |
| **Compressor** | Write it in `enth-dsp`: feed-forward, stereo-linked, soft knee, attack/release, make-up gain, **no look-ahead** | Own code. The algorithm follows JOS's Faust `compressor_mono` (STK-4.3); web-audio-api-rs `dynamics_compressor.rs` (MIT) is the Rust cross-check | No healthy Rust crate exists; a few hundred lines; zero latency avoids CLAP latency restarts; enth owns determinism | Signalsmith `dynamics.h` once it has a few months of history |
| **"Use my own reverb"** | No hosting in P1. Add a persistent global FX bypass ("dry out") stored outside the `Patch` and ignored by scoring and CLI renders, and document "put your reverb after enth in REAPER" | — | The DAW already gives latency compensation, automation, crash isolation and a GUI; determinism is preserved; matches what users request | — |
| **Multimode filter** | In-house TPT SVF after Cytomic's paper | Own code | About 100 lines; no licence question | DaisySP `svf` (MIT) as a cross-check |
| **Oscillator anti-aliasing** | polyBLEP after Mutable Instruments stmlib and DaisySP | MIT | Enough for P1's classic waveforms | Signalsmith `elliptic-blep` (MIT) later |
| **Parameter smoothing** | One-pole and linear smoothers in `enth-dsp` | Own code | CLI renders must equal plugin renders | — |
| **Real-time checks** | `assert_no_alloc` (already required) and nice-plug's `assert_process_allocs`; try `rtsan-standalone-rs` once its licence is checked | — | Catch allocations and locks in `process` | — |

After P1, the list continues:

| When | What | Licence |
|---|---|---|
| S2 | Rust port of CloudSeedCore as a "lush" reverb mode. Credit Ghost Note Audio, use no "Cloud Seed" branding, size buffers in `prepare` from the actual sample rate | MIT |
| S2 | Chorus, flanger and phaser ported from DaisySP core or Signalsmith `chorus.h` | MIT |
| S2 | Saturation: Faust `aanl.lib` ADAA plus Signalsmith `rates.h`-style oversampling | MIT |
| S2 | EQ: the `biquad` crate or an in-house SVF bell/shelf; safety limiter from Signalsmith `limiter.h` | MIT/Apache, MIT |
| S2+ | Character compressors: Airwindows ButterComp2 and Pressure4–6 | MIT |
| After GS1, only for a concrete need *inside* the synth's signal flow | A one-day spike (clack-host CLAP inside the nice-plug CLAP, floating editor, REAPER on X11). Then optionally a CLAP-only, in-process post-FX slot after the scoring tap, flagged non-reproducible, bypassed by the CLI by default, with non-destructive missing-plugin loading | clack: MIT/Apache |
| Not planned (reopen only by ADR) | VST3 or LV2 hosting, embedded third-party editors, out-of-process sandboxing | — |

## Docs to change

| Doc | Change |
|---|---|
| `docs/roadmap.md`, P1 "Effects" line | Replace it with: "Effects: a reverb ported from Signalsmith Basics (MIT) and an in-house feed-forward compressor (after JOS's Faust compressor, STK-4.3), both zero-latency, deterministic modules in `enth-dsp`; a persistent global FX bypass; no third-party plugin hosting ([research.md §8](research.md#8-reusing-effects-and-hosting-plugins), ADR-0006)." Add to Gate G1: "reverb and compressor golden tests pass on wasm32; the reverb was chosen by a blind A/B." Add a row to "Don't build yet": *GS1 — third-party plugin hosting slot — costs weeks, breaks determinism, the CLI and preset sharing; the DAW chain already provides it.* Name the CloudSeedCore port, chorus and saturation under S2 "more effects". |
| `docs/research.md` §8 | Replace `PENDING_EFFECTS_SUMMARY` with a short version of this summary: the P1 picks table, the reference-only list (GPL/AGPL/LGPL, including JUCE 9 and the DaisySP-LGPL, chowdsp and Faust per-function traps) and the hosting verdict. Also update §3 (nice-plug 0.4.2 depends on MIT/Apache `vst3` 0.3.0, per `cargo tree` on 2026-10-18; clack-host on crates.io since May 2026) and the §1 HexoSynth row (synfx-dsp is GPL-3.0-or-later). Add §9 rows: "Hosting third-party plugins inside the synth" and "`cloudseedcore-rs` (C++ via cxx, not wasm32)". Add to the §10 watch list: Signalsmith `dynamics.h`, clack-host, vst3-host on Linux. |
| `docs/AGENTS.md` | **Licensing:** extend the never-copy list with the traps in mixed-licence repos (JUCE modules, DaisySP-LGPL, chowdsp DSP modules, GPL-declared Faust functions, `sst-basic-blocks` outside its MIT-marked headers, valib's plugins, nih-plug `plugins/`, unlicensed code such as Signalsmith's ADC example). Require per-function and per-module licence checks. Require every port to record the upstream URL, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and in `docs/dsp/<module>.md`. **Architecture:** "replace any upstream RNG (`rand()`, `mt19937`, `thread_rng`) with the engine's seeded PRNG; flush denormals in code, not through CPU flags; effects report zero latency unless an ADR says otherwise; no C/C++-wrapping crates in L0–L5." |
| `docs/research-process.md` | Add a blind A/B listening check to the "What people say" axis for DSP picks, because forum opinion on DSP quality is thin. Add "check licences per function or module in mixed-licence repos" to the sub-researcher rules. Add "when the sandbox blocks the GitHub API or rustup targets, mark it in Gaps and run `wasm-check` and the health checks locally before deciding". Make "Corrections to sub-reports" a standard `summary.md` section. |
| `docs/adr/0006-effects-strategy.md` (new, Proposed, confirm at G1) | Decision: own deterministic effects ported from permissive sources, behind `Module`. Reverb is the Signalsmith port, with fundsp as fallback; the compressor is in-house; no look-ahead in P1; no third-party hosting in the engine. Any future slot is a CLAP-only post-stage in `enth-plugin` after the scoring tap, flagged non-reproducible. Alternatives: fundsp-first, CloudSeedCore-first, a hosting slot. Wrong if: the reverb loses the blind A/B to something unportable, the port takes more than twice its estimate, or users concretely need third-party FX inside the voice or modulation path. Add the ADR to the table in `docs/adr/README.md`. |
| `docs/adr/0004-licensing.md` | Add a dated note. Ported *source* may come from MIT, BSD, ISC, Apache-2.0, Zlib, 0BSD, STK-4.3 (MIT-style) or public domain, with notices in `THIRD_PARTY_NOTICES.md`. JUCE modules have been AGPLv3/commercial since JUCE 9 and are reference-only. |
| `THIRD_PARTY_NOTICES.md` (new, repo root) | One entry per ported source: name, upstream URL and commit hash, SPDX id (or `LicenseRef-STK-4.3`), copyright line, the enth files derived from it, and the full licence text. Initial entries: Signalsmith Basics and the Signalsmith DSP library (MIT), JOS's Faust compressor (STK-4.3), web-audio-api-rs (MIT) if any code is ported. Later: CloudSeedCore (MIT, with credit to Ghost Note Audio). Crate dependencies' licences are listed separately (generated from the lockfile, consistent with `deny.toml`). |
| `docs/dsp/reverb.md`, `docs/dsp/compressor.md` (new; AGENTS.md already requires them) | Algorithm, provenance and upstream commit, deviations from upstream (PRNG, denormals, sample-rate scaling), parameters with units, golden-test plan. |
| `docs/design.md` §5 (optional) | One line: the global FX bypass is per-instance monitoring state outside the `Patch`, and scoring and CLI renders ignore it. Only effects enth owns may sit in the scored path. |
