# Reverb (`enth-dsp::reverb`)

A stereo feedback-delay-network (FDN) reverb, ported from Signalsmith Basics' `reverb.h`. Zero latency, deterministic, bit-exact with the upstream C++ on every platform, no allocation after `prepare`.

- **Status:** the DSP core (P1a lane FX1, phase 1) is done. The `Module` wrapper, the engine-level golden case and the owner's A/B come in phase 2 (after freeze F2). The `ParamSpec`s are already in the catalogue (`crates/enth-patch/src/catalogue/reverb.rs`, frozen at F1, listed in [catalogue.md](catalogue.md)); the section Parameters below says how they map onto this core.
- **Code:** `crates/enth-dsp/src/reverb/` (`mod.rs` the network and the 20 ms parameter fade, `delay.rs`, `mix.rs`, `biquad.rs`, `table.rs`) and `crates/enth-dsp/src/util/flush.rs`.
- **Tests:** `crates/enth-dsp/src/reverb/tests.rs` (unit), `crates/enth-dsp/tests/reverb.rs` (the C++ hashes, allocation, clicks; its own binary because it installs `assert_no_alloc`'s allocator), fixtures in `crates/enth-dsp/tests/fixtures/`.
- **Decisions behind it:** [ADR-0006](../adr/0006-effects-strategy.md) (the 2026-10-19 note), [ADR-0004](../adr/0004-licensing.md) (BSL-1.0 for the fade), P1a research report 02 (`_local/sessions/2026-10-19-p1a-headless-core/02-dsp-modules.md`, Q5).

## Public API

```rust
use enth_dsp::reverb::{Reverb, ReverbParams};

let mut reverb = Reverb::prepare(48_000.0);          // the only allocation (1,638,400 bytes at 44.1 and 48 kHz)
reverb.set_params(&ReverbParams { room_ms: 120.0, ..ReverbParams::default() });
reverb.process(in_l, in_r, out_l, out_r);            // dry + wet; also process_in_place(left, right)
reverb.reset();                                      // silence the tail; parameters are kept
```

`Reverb::prepare(sample_rate: f32)` is the constructor (the engine's `prepare` is a constructor from `PrepareCfg`, whose `sample_rate` is `f32`). `latency_samples()` is 0; `tail_samples()` is `round(sr * rt20 * 3)`, the time to fall by 60 dB; `allocated_bytes()` reports the delay memory. Sample rates are clamped to 8 kHz to 768 kHz (NaN becomes 48 kHz). The module does not sanitise NaN or infinite input: one such sample poisons the feedback until `reset()`.

## Algorithm

Per frame, in the order of the code:

```text
in L,R --x input_gain--> flush --> stereo -> 8 channels (4 stereo pairs rotated by k*pi/8)
   |
   +--> delay1 (1/8 room) -> Hadamard -> delay2 (1 x room) -> Hadamard --> s
                                                                           |
        feedback loop                                                      |
        delayFB (room x 2^(+-1/16, +-3/16, ...); channels 0-2 are read through a
                 Lagrange-7 reader with an LFO-modulated extra delay = "detune")
          -> Householder -> low shelf -> high shelf (per channel) --> fb
          (s + fb x decay_gain) -> flush -> written to delayFB, channel c going to channel c + 3

   s -> delayEarly (1/4 room) -> Hadamard -> x early_gain  --+
   fb ------------------------------------------------------+--> sum
   sum -> delay3 (1/2 room) -> Hadamard -> delay4 (1/4 room) -> 8 channels -> stereo
       -> low-cut (high-pass, Q 0.5) -> high-cut (low-pass, Q 0.5) --> wet
out = in x dry + wet x wet_gain
```

How the parameters become gains (all in `Reverb::begin_block`):

| Quantity | Formula |
| --- | --- |
| Loop gain per pass | `decay = 10^((-20 / rt20) * (room_ms / 1000) / 20)`, the dB part in `f32` |
| Input gain | `2 * (0.5 / 64) * sqrt((1 - decay) / (1 - decay^(100 / room_ms)))`: scales the input by the number of echoes expected in the first 100 ms |
| Early gain | `early * 0.35` ("tuned by ear" upstream) |
| Detune LFO | depth 2 ms; rate `detune * sqrt(room_s) * 0.0004 / depth_samples` cycles per sample; three phases 120 degrees apart on channels 0 to 2 |
| Low damp | low shelf at `low_cut + 100` Hz, gain `max(decay^low_damp_rate, 1e-3)`, Q 0.5, one-sided bandwidth design |
| High damp | high shelf at `high_cut / 2`, gain `max(decay^high_damp_rate, 1e-3)` |
| Cuts | high-pass at `low_cut`, low-pass at `high_cut`, both Q 0.5 |

The delay offsets of each stage are spread over `room * scale` samples from fixed random-looking draws (the constant table, below), and re-drawn when a fade starts. The feedback delay is the exception: its offsets are exponentially spaced, no draws.

**Block structure.** Upstream's `process` is block-based: it advances the parameter fade, recomputes the filters and re-derives the gain ramps once per call. The port does the same, so `process` takes any length, but the result while parameters move depends on the length (see Behaviour).

**The 20 ms fade (A/B).** A parameter change does not move anything at once. At the start of a fade (the next call after a change, when no fade is running) the delay offsets are redrawn for the new room, and for 20 ms (`0.02 * sr` samples) every delay read cross-fades from the old offset set to the new one (`from + (to - from) * fade`) while the dry, wet, decay, input and early gains ramp linearly. A change made during a fade waits for the fade to end and starts the next one. This is the whole reason a room-size change neither allocates nor clicks.

## Provenance

| Upstream | Commit | Licence | Ported into |
| --- | --- | --- | --- |
| [Signalsmith-Audio/basics](https://github.com/Signalsmith-Audio/basics) `include/signalsmith-basics/reverb.h` (tag 1.2.1, 2026-09-30) | `369e906e03760ec219d07b06fa06dc4fd099f377` | MIT, Copyright 2022-2025 Signalsmith Audio / Geraint Luff | `reverb/mod.rs`, `reverb/delay.rs` (`MultiDelay`), `reverb/table.rs` |
| [Signalsmith-Audio/dsp](https://github.com/Signalsmith-Audio/dsp) `delay.h`, `mix.h`, `filters.h` (v1.7.3, 2026-10-13; identical to v1.7.1, which Basics pins) | `ca390d28c41ea5108a528e2e81faa4088f2f8ac8` | MIT, Copyright (c) 2021 Geraint Luff / Signalsmith Audio Ltd. | `reverb/delay.rs` (`Buffer`, `MultiBuffer`, `Reader::read`, `InterpolatorLagrangeN<7>`), `reverb/mix.rs` (`Hadamard<8>`, `Householder<8>`, `StereoMultiMixer<8>`), `reverb/biquad.rs` (`BiquadStatic`: high-pass, low-pass, low-shelf and high-shelf only) |
| Basics `include/stfx/stfx-library.h` (same commit as above) | `369e906e03760ec219d07b06fa06dc4fd099f377` | BSL-1.0 ("Released under the Boost Software License" in the header; the repository's `LICENSE.txt` is MIT), Copyright 2021-2022 Geraint Luff / Signalsmith Audio Ltd | `reverb/mod.rs` (`Param`, `Segment`, `begin_block`, `end_block`: the fade bookkeeping, re-implemented) |

Full licence texts and the list of changes: `THIRD_PARTY_NOTICES.md`. Ported source keeps upstream's evaluation order, including every place where C++ mixes `double` and `float` (parameters, ramps, the LFO phase and the filter design are `f64`; the audio and the coefficients are `f32`; `unit + i` is a float addition; `b1 * 0.5` in the low-pass uses the already rounded `b1`; the normalisation of the biquad coefficients rounds twice). Change any of these and the last bits move, which the C++ test notices.

## Deviations from upstream

| Change | Why | Effect on bits |
| --- | --- | --- |
| **A constant table instead of `std::mt19937`** (`reverb/table.rs`). `MultiDelay::updateLengths` is only called with five constant seeds, so its 40 draws (a unit `float` and a swap index per channel and seed) are constants of the algorithm. | `std::uniform_real_distribution` and `uniform_int_distribution` are not portable (libstdc++, libc++ and MSVC differ), so upstream sounds slightly different per platform. A table needs no RNG, and AGENTS.md forbids seeds that depend on parameter values (seeding from the patch hash would retune the reverb with every knob). The table holds the libstdc++ (GCC 13.3) draws that the C++ reference used. A test rebuilds it from a from-scratch `mt19937`, libstdc++'s `generate_canonical<float, 24>` and its Lemire `uniform_int_distribution`, and checks `mt19937` against the standard's published values. If variety is wanted later, give the reverb an explicit `seed` parameter. | None (same draws) |
| **`reset()` clears `delay4`**. Upstream clears `delay1`, `delay2`, `delay3`, the feedback and the early delays, but not `delay4`, so a stale tail survives a reset. The port also cancels a running fade (upstream would carry on fading from the old values) and makes the next block apply the parameters at once. | A reset must give silence; tested for all six lines, and that a reset render is bit-identical to a fresh one. | None in the golden scenarios (they never reset mid-render) |
| **Denormals are flushed in code, not by FTZ/DAZ.** Upstream's `process` wraps its work in `StopDenormals`, which sets FTZ and DAZ in MXCSR. The port drops it (the flags differ between platforms and do not exist on wasm) and flushes values with magnitude below `1e-15` (`util::flush`) at the feedback write, at the output of every biquad and at the wet input (after its gain). | A reverb tail decays for tens of seconds. Measured (report 02, 1 s of noise then 60 s of silence): without flushing 2,337,601 subnormal output samples and 229,118 subnormal state values, never reaching zero, 4.2x slower; with it none, and the output is exactly zero from 15.1 s. | Bits differ from upstream only once a value in the network falls below `1e-15` (-300 dB). Upstream flushes at the hardware threshold, `1.2e-38` (-760 dB), so the two agree until then. In one scenario that does reach it (RT20 0.05 s after a change at 1 s) the output differs from the C++ by at most 2.3e-12 absolute (-233 dB re full scale, against a peak of 1.5), starting at 1.399 s when the signal is already at 2.6e-10. The scenarios in the golden test deliberately stay above `1e-15`. |
| **Parameters are clamped.** `room_ms` to 10 to 200 and every other parameter to the ranges in the table below; NaN becomes the minimum. | Upstream does not clamp `roomMs`: above `maxRoomMs` a read would leave the channel's region of the ring and play another channel's data. A test checks that every read of every stage stays inside its channel for rooms of 10 to 200 ms at 8 to 768 kHz. | None inside the ranges |
| **`set_params` before the first block applies at once.** Upstream's `configure()` calls `reset()`, and the first block starts with all parameters settled; the port makes `set_params` after `prepare`/`reset` snap too. | So an engine that builds the module and sets its parameters before the first block does not hear a 20 ms fade from the defaults. | None (the C++ harness sets its parameters before `configure()`) |
| **The 20 filters are designed in 4 groups, and only when an input changed.** The two low-cuts, the two high-cuts, the eight low-damp and the eight high-damp sections each share one design; a design is a pure function of `low_cut`, `high_cut`, the two damp rates and the feedback gain, so it is recomputed only when one of those changed. | Upstream redesigns all 20 in `f64` (with `sin`, `cos`, `tan`, `sinh`, `log`, `pow`) every block. Measured: 2,601 instructions per frame with the 20 designs per 32-frame block against 2,422 without (7 % fewer; 4 % at blocks of 64). | None (the same function of the same inputs) |
| **Rust, `libm`.** Every transcendental goes through the pinned `libm`; no `mul_add`, no fast-math. | AGENTS.md determinism rules. glibc's `cos`/`sin`/`pow`/`tan`/`sinh` may differ from `libm` by an ulp, but the results are rounded to `f32` before they reach the audio path (measured: no difference in 768,000 samples). | None measured |
| **Not ported:** `state()` (UI metadata), `presets()` (the "ambient" preset becomes a patch preset), the Vicanek/cookbook/band-pass/notch/peak/all-pass biquads, the STFX library apart from the fade. | Become `ParamSpec`s in the catalogue, or are unused. | n/a |

## Parameters

Real units; `ReverbParams` (`f64`) is what `set_params` takes. Dry, wet and early are percent of gain (100 is unity, i.e. 0 dB): that maps exactly onto upstream's gains (`50 / 100.0 == 0.5`), needs no exponential and has a true zero. The ranges are upstream's UI ranges and are enforced by clamping. The `ParamSpec`s are lane S1a's catalogue; they stay inside these ranges, but not always in these units (next paragraphs).

| Field | Unit | Range | Default | Upstream | Notes |
| --- | --- | --- | --- | --- | --- |
| `dry_pct` | % of gain | 0 to 400 | 100 | `dry` | 100 is 0 dB, 400 is +12 dB |
| `wet_pct` | % of gain | 0 to 400 | 50 | `wet` | 50 is -6 dB |
| `room_ms` | ms | 10 to 200 | 80 | `roomMs` | "1 ms is about 1 foot"; the longest delay of the network; **not modulatable**; no allocation on change |
| `rt20_s` | s | 0.01 to 30 | 1 | `rt20` | RT20, the fall to -20 dB; RT60 is 3 times that; **not modulatable** |
| `early_pct` | % | 0 to 250 | 150 | `early` | early reflections level |
| `detune` | unitless rate | 0 to 50 | 2 | `detune` | detune depth in the feedback loop, about cents per loop at a 1 s room |
| `low_cut_hz` | Hz | 10 to 500 | 80 | `lowCutHz` | high-pass corner; also sets the low shelf (`+ 100` Hz) |
| `high_cut_hz` | Hz | 1,000 to 20,000 | 12,000 | `highCutHz` | low-pass corner; also sets the high shelf (`/ 2`) |
| `low_damp_rate` | power | 1 to 10 | 1.5 | `lowDampRate` | how much faster lows die than mids |
| `high_damp_rate` | power | 1 to 10 | 2.5 | `highDampRate` | how much faster highs die than mids |

**The catalogue's units differ from the core's in two places.** Dry and wet are gains in **dB** (−60 … +6 dB), and `decay` is the RT60 in **ms** (the core takes RT20 in seconds). The wrapper converts (their arithmetic is an engine-version matter, their meaning is the catalogue's contract):

- `rt20_s = decay / 3000`, with `decay` in ms. The core's RT20 is the fall to −20 dB, and a decay that is linear in dB falls 60 dB in three times that, so the catalogue's 100 ms … 30 s becomes 0.033 … 10 s, inside the core's 0.01 … 30 s, and the default 3 s becomes the core's default of 1 s.
- `pct = 100 · 10^(dB / 20)` for `dry` and `wet`, except that the minimum, −60 dB, is **off**: gain 0, like a fader at the bottom, so a fully wet or fully dry setting exists (the core's 0 % is a true zero). The maximum, +6 dB, is 199.5 %, inside the core's 0 … 400. The catalogue's default `wet` of −6 dB is 50.1 %, against the core's 50.
- `room`, `early`, `detune`, the two cuts and the two damping rates are the core's values as they are.

Who smooths what (`Smoothing` in the catalogue):

| Catalogue key | Core field | Smoothed by | Modulatable |
| --- | --- | --- | --- |
| `dry`, `wet`, `early`, `detune` | `dry_pct`, `wet_pct`, `early_pct`, `detune` | the reverb: its 20 ms fade (catalogue smoothing `None`) | yes |
| `room`, `decay` | `room_ms`, `rt20_s` | the reverb: its 20 ms fade (smoothing `None`) | no: a value that changes every call restarts a fade each 20 ms |
| `low_cut`, `high_cut`, `low_damp`, `high_damp` | `low_cut_hz`, `high_cut_hz`, `low_damp_rate`, `high_damp_rate` | **the engine**: a linear ramp over 20 ms, the length of the reverb's own fade (next section for why) | yes |

The module is flagged `time_based` for the global FX bypass (ADR-0006).

## Behaviour the wrapper must know

- **Call `process` with the engine's fixed 32-frame segment** (the last one of a render included). With steady parameters, the output was bit-identical for blocks of 32 to 480 frames (measured; at most the rounding of the detune LFO's phase wrap could differ). While a parameter changes, the fade starts at the next block boundary and the filters follow the call, so 32 against 64 differs by up to -44 dB re peak (measured). Blocks of any length work; none ever allocates.
- **The cut and damping filters are not faded.** `low_cut_hz`, `high_cut_hz` and the two damp rates change the filters at the start of the next block, as upstream does. A large jump steps the output: a 12 to 3 kHz high-cut change on a 440 Hz wet tone of amplitude 0.5 gives a second difference of 0.021 against 0.0028 steady (7x); room, decay, detune, early and wet changes, and a low-cut change to 300 Hz, stay within 1.4x for this tone (a step is larger the closer the signal is to the corner). P1a report 02 said the internal fade smooths every parameter; for these four it does not. So the catalogue gives `low_cut`, `high_cut`, `low_damp` and `high_damp` the engine's smoother, a linear ramp over 20 ms in front of the reverb (this does not change the C++ comparison, which sets parameters directly); the other six are faded inside the reverb and the engine leaves them alone, because a second ramp would only add lag.
- **`set_params` is cheap and idempotent.** It clamps and stores; nothing happens unless a value changed. Call it per segment from the plan's current values. Parameters set between two calls take effect together at the next call.
- **`reset()`** for a fresh start (voice-less global effect after a transport reset, or un-bypassing). It clears the six delay lines and 20 filter states and restarts the LFO; the next block uses the parameters with no fade. A bypass that fades the module out over 20 ms and then stops calling `process` should `reset()` before fading back in.
- **State carry across plan swaps:** the struct owns everything; move the instance, do not rebuild it. It is `Send`, with no interior mutability, no clock, no I/O.
- **Tail:** `tail_samples()` says how long non-zero output can follow silent input (flush makes it end in exact zeros), for offline renders that want to render a tail.
- **Dry path:** `out = in * dry + wet * wet_gain`: with `dry_pct = 100` and `wet_pct = 0` the module is the identity bit for bit (a `-0.0` input comes out as `+0.0`; tested).

## Cost

Measured 2026-10-19 on the dev laptop, release build, `libm 0.2.16`, stereo, default parameters, 48 kHz; instructions are callgrind counts (the AGENTS.md metric), wall time is information only (load average 5 to 12).

| Case | Instructions per stereo frame | Wall per frame |
| --- | --- | --- |
| Steady, blocks of 64 | 2,400 | about 315 ns (66x realtime, 1.5 % of a core) |
| Steady, blocks of 32 (the engine's segment) | 2,422 | about 325 ns |
| The same with upstream's 20 filter designs every block, blocks of 32 / 64 | 2,601 / 2,490 | |
| A room and a high-cut change every 100 ms (back-to-back fades), blocks of 32 | 2,577 | about 356 ns |
| Report 02's prototype, blocks of 64 | 2,574 | 342 ns |

Budget from the research summary: reverb at most 3,000 instructions per frame (G1 sub-budget). The share of the cost: delay reads and writes about 45 %, the 20 biquads 20 %, the Hadamard/Householder/mixer 5 %, the per-sample `f64` `cos`/`sin` for the detune LFO about 6 % (replacing them with a rotating phasor would break the match with upstream). Memory is fixed after `prepare`: 1,638,400 bytes at 44.1 and 48 kHz, 3,276,800 at 96 kHz.

## Golden plan

1. **One-time validation against the upstream C++** (done; repeat after any change to the port). The C++ harness renders the scenarios in `crates/enth-dsp/tests/fixtures/reverb-scenarios.txt` with Signalsmith's headers unmodified and writes interleaved little-endian `float32`. `crates/enth-dsp/tests/fixtures/reverb-cxx-sha256.txt` holds the SHA-256 of each output (`sha256sum` of the file). The Rust test renders the same scenarios and compares.
   - Scenarios: `default` (upstream defaults; room, RT20 and high cut change at 2 s; block 64), `ambient` (upstream's preset; same change), `segment32` (block 32, the engine's segment; a second change lands mid-fade), `cd44k` (44.1 kHz, block 48 that does not divide the render, other cut/damp/detune settings, three changes), `hi96k` (96 kHz, largest room, detune and damp rates, then the smallest room). Between them: 3 sample rates, 4 block lengths and 7 change events (each sets one to three parameters), one of them landing mid-fade.
   - Input: SplitMix64 noise (seed 12345) for 250 ms and an impulse `(1.0, -0.5)` at 1.5 s.
   - The harness, its build script and the C++ output are kept outside the repository, in the FX1 scratch directory (`~/code/enth-scratch/p1a/fx1/cxx/`): `harness.cpp` reads the scenario file, `tables.cpp` prints the `mt19937` draws, and `build-and-hash.sh` clones nothing but checks the pinned commits of both upstream repositories, builds with `g++` and `clang++` at `-std=c++17 -O2 -ffp-contract=off`, checks that the two compilers produce identical bytes, prints the SHA-256s, and diffs them against the fixture (`--write` rewrites it). Measured 2026-10-19: g++ 13.3.0 and clang 18.1.3 agree on all five, and equal the Rust output; the first two hashes equal report 02's.
   - **Build condition: no FMA contraction and no fast-math** (baseline x86-64 or `-ffp-contract=off`). Report 02 measured the C++ with `-march=native` (FMA) differing from the baseline build by up to 1.8e-5, and with `-Ofast` by up to 5.2e-5; clang's FMA output also differs from g++'s. The Rust output has no such choice (Rust does not contract), so it is the portable side.
   - The scenario file is the single source of the parameters for both sides (the C++ divides the percent gains by `100.0` exactly as `ReverbParams` does).
2. **The Rust hash is the cross-platform golden.** `cargo test -p enth-dsp --test reverb` compares against the SHA-256 fixtures, so CI needs no C++ toolchain. The same test built for `wasm32-wasip1` and run under wasmtime passes with the same fixtures, which is native = wasm = C++:
   ```text
   CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run" cargo test -p enth-dsp --locked --release --target wasm32-wasip1 --test reverb
   ```
   Measured 2026-10-19: native debug and release, `-C target-cpu=native` (with FMA) and wasm32-wasip1 under wasmtime 48.0.6 all pass. `cargo xtask wasm-check` also builds `enth-dsp` for `wasm32-unknown-unknown`. The scratch script `~/code/enth-scratch/p1a/fx1/wasm-hash.sh` runs the three.
3. **Behavioural tests** (all in the normal gate): a 61 s tail (1 s noise, 60 s silence, blocks of 32) with no subnormal output sample and no subnormal value in any delay line or filter, ending in exact zeros (plus 44.1 kHz and 96 kHz runs and a run fed with subnormal input); `assert_no_alloc` around `process`, `process_in_place`, `set_params` and `reset` with changing parameters, blocks of 1 to 4096 frames and NaN/infinite parameters (a deliberate allocation inside the wrapper aborts the test, checked once by hand); the click test below; reset repeatability; fade length; clamping; a proptest (fixed seed in the gate) that any parameters, block lengths and changes keep the output finite and bounded.
4. **Click test.** A 440 Hz sine (raised-cosine onset) through the wet path only; the room changes while it plays (80 to 150, 80 to 20, 10 to 200, 200 to 10, and room with RT20 together) in 32-frame blocks under `assert_no_alloc`. The metric is the largest absolute second difference in the window around the change, divided by the larger of the steady values before and 1.5 s after (a new room legitimately changes the level). Bound 2.0; measured 0.3 to 1.4. The negative control splices the renders of two rooms without a fade and reads 196, so the metric can see a click. This is a proxy; nobody has listened.
5. **Phase 2 (after F2, with the wrapper):** an engine-level golden case `reverb-impulse` in `tests/golden/cases/` (hash on Linux, Windows and wasm in CI; macOS on demand), the gungraun budget in `crates/enth-engine/benches/reverb.rs` (the core measures 2,422 instructions per frame at 32 frames; the limit is the wrapper's measurement plus 10 %), and the blind, level-matched A/B against fundsp's `reverb2`/`reverb4` for the owner (20 to 30 minutes), renders kept in scratch.

## Known limits and open items

- Cut and damping filters step on a change (above). The catalogue puts the engine's 20 ms smoother in front of the four; whether that is enough for a fast sweep or a modulated cut is for ears to say.
- The C++ comparison has not been run with libc++ or MSVC, and the table holds libstdc++'s draws: upstream's own output differs on those standard libraries, ours does not.
- The reverb is stereo-in, stereo-out only (as upstream). A mono source feeds both inputs.
- No input sanitising (NaN poisons the feedback until `reset()`); the engine's output stage is the place to catch it.
- Quality judgements (decay character, the detune depth, the early-reflection level) are upstream's "tuned by ear" settings; nothing here has been listened to. The owner's A/B decides whether this reverb stays.

## References

- Signalsmith Audio, [Let's Write A Reverb](https://signalsmith-audio.co.uk/writing/2021/lets-write-a-reverb/) (the design of this reverb: diffusion, Householder feedback, damping).
- Franck, A. (2009), *Efficient Algorithms and Structures for Fractional Delay Filtering Based on Lagrange Interpolation*, JAES 57(3) (the product-tree evaluation of the Lagrange reader).
- Bristow-Johnson, *Audio EQ Cookbook* (the biquad forms).
- Matsumoto and Nishimura (1998), *Mersenne Twister* (the table regeneration test); WG21 [P2059R0](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2020/p2059r0.pdf) (why standard distributions are not portable).
