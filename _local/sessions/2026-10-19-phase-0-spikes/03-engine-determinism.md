# 03 — Engine speed and determinism

- **Date:** 2026-10-19
- **Sub-researcher:** 03 (engine speed and determinism, plus local wasm32 builds)
- **Questions answered:** 1 (float determinism), 2 (denormals), 3 (throughput and SIMD), 4 (benchmarking), 5 (real-time safety tooling), 6 (local wasm32 builds and native-vs-wasm hash check), 7 (golden testing). Cut short by the coordinator's wrap-up call before a reboot; unfinished items are under Gaps.
- **Machine for local checks:** i7-9750H, rustc/cargo 1.95.0, LLVM 22.1.2, glibc 2.39, node v22.22.3. Governor `powersave`, turbo on. **Load average 13–31 during every timing run**, because the other researchers were building at the same time. Treat all absolute timings as rough. The hash results don't depend on load.
- **Markings:** *measured* = run here today, with commands. *documented* = official docs, source code or spec. *anecdotal* = third-party, vendor or forum.

## Answer in brief

- **Cross-platform bit-exactness is cheap, and enth should aim for it in the render path.** A deterministic voice I wrote (2 polyBLEP saws → TPT low-pass ×2 → tanh → envelope, 1 and 16 voices, 2 s) gave **identical FNV-1a hashes on native x86-64 and wasm32 (V8)**. The same held for debug and release builds, for `-C target-cpu=native` (AVX2+FMA) and for wasm `+simd128`. The condition is that every transcendental goes through the `libm` crate. With `std` math, 64% of samples differed between native and wasm (max 3.0e-8, −150 dBFS) *(measured)*. aarch64 is still unmeasured.
- **What breaks bit-exactness is known and avoidable:**
  - `std` transcendentals. Their precision is "non-deterministic", even within one execution *(documented)*, and LLVM rewrote `powf(2.0, x)` into `exp2` *(measured)*.
  - `mul_add`-style SIMD helpers that fuse on some targets. `wide` 1.7.1 and `fearless_simd` 1.1.0 both do *(documented in their source)*.
  - Runtime CPU dispatch. rustfft 6.4.1 gives **three different bit results on one machine** from its AVX, SSE and scalar paths *(measured)*.
  - `algebraic_*` float methods, stable since Rust 1.98 and non-deterministic *(documented)*.
  - FTZ differences and NaN payloads.
- **Denormals:** a decaying one-pole that gets stuck in the subnormal range costs **14.9 ns vs 0.09 ns per filter-sample** with FTZ/DAZ on native, and **52 ns vs 1.4–2.1 ns** in wasm with explicit flushing *(measured)*. Rust documents changing MXCSR as undefined behaviour, and wasm has no FTZ mode, so **flush in code**. nice-plug 0.4.2 nonetheless turns FTZ on around every `process()` call *(documented in its source)*. Plugin renders and CLI renders therefore agree bit-for-bit only if the engine never produces subnormals.
- **fundsp 0.23.0 with `default-features = false` does not compile on any target** (`Vec`/`vec!` missing in `resample.rs`). This is a known bug, #86, fixed on master on 2026-01-11 but never released. With `features = ["std"]` it builds for wasm32: 34 crates, 47 s cold *(measured)*. `biquad`, `libm` and `rustfft` all build for wasm32 *(measured)*.
- **Throughput:** Google SynthMark's comparable C++ voice (LFO, 2 oscillators, biquad, 2 ADSRs) has a median **VoiceMark_100 of 701 voices on one core**, which is roughly 700× realtime per voice per core. That is across 1,785 public results; the benchmark caps at 1,024 *(documented/anecdotal aggregate)*. My naive scalar libm toy reached about 280–520× for 1 voice and 16–36× for 16 voices on the loaded laptop; under V8 the wasm build was in the same range *(measured)*. G0's 100× floor is easy to meet.
- **Benchmarking:**
  - Criterion's own FAQ says not to rely on its results on cloud CI *(documented)*.
  - The rule "regressions > 10% fail CI" needs instruction counts: gungraun 0.20.0 (Valgrind or perf) or hyperfine 2.0.0's new `--metrics`.
  - On this laptop perf is blocked (`perf_event_paranoid = 4`), and Valgrind and hyperfine are missing *(measured)*.
- **Real-time tooling:**
  - `assert_no_alloc` 1.1.2 (2021) and nice-plug's fork `nice-assert-no-alloc` 1.0.0 are both **BSD-1-Clause**. That licence is not in Task 2's `deny.toml` allow-list.
  - `rtsan-standalone` 0.3.0: the LICENSE file is Apache-2.0. It runs on Linux, macOS and iOS, and by default downloads prebuilt runtime binaries.
  - rustc nightly has `-Zsanitizer=realtime` (merged 2025-11-08).
- **Golden tests:** use an exact hash wherever bit-exactness is promised, and a quantized feature-summary snapshot (`cargo-insta` is already installed) as the diagnostic for when a hash breaks. Faust's cross-backend impulse tests (including wasm), fundsp's tests and web-audio-api-rs's tests all use tolerances *(documented)*.

## Findings

### Q1 — Float determinism across x86-64, aarch64 and wasm32

**What Rust guarantees** *(documented)*:

- The std `f32` docs, read 2026-10-19 on the current stable docs (Rust 1.99.1, released 2026-10-13, per `channel-rust-stable.toml`), say:
  - `+ − × ÷` round "according to the roundTiesToEven direction defined in IEEE 754-2008".
  - `sqrt` and `mul_add` are "guaranteed to be the rounded infinite-precision result … guaranteed not to change".
  - For `sin`, `exp`, `powf`, `tan` and the other transcendentals: "The precision of this function is non-deterministic. This means it varies by platform, Rust version, and can even differ within the same execution from one invocation to the next." `tan` "currently corresponds to the `tanf` from libc".
  - NaN results have a non-deterministic sign and payload.
  - The `algebraic_add/sub/mul/div/rem` methods are **stable since 1.98.0** and "non-deterministic" (they allow reassociation and contraction) <https://doc.rust-lang.org/std/primitive.f32.html>.
  - On my 1.95.0 they are still unstable *(measured: E0658)*.
- RFC 3514 "float semantics" (merged 2024-07-27; tracking issue #128288 still open) <https://rust-lang.github.io/rfcs/3514-float-semantics.html> says:
  - Rust follows IEEE 754-2008 "with some caveats around operations producing NaNs", with "default exception handling … without abruptUnderflow/flush-to-zero".
  - "it is UB for inline assembly to alter the behavior of floating-point operations in any way".
  - Strict semantics rule out FMA contraction. Rust never fuses `a*b+c` on its own.
- The `_mm_setcsr` docs (deprecated since 1.75.0) say that changing the masking, rounding or DAZ flags "leads to immediate Undefined Behavior: Rust assumes that these are always in their default state" <https://doc.rust-lang.org/core/arch/x86_64/fn._mm_setcsr.html>.

**wasm32** *(documented)*:

- WebAssembly 3.0 (spec dated 2026-10-13): "Floating-point arithmetic follows the IEEE 754 standard", round-to-nearest ties-to-even, non-stop mode. NaN payloads are non-deterministic. The **deterministic profile** makes NaNs canonical and gives relaxed vector instructions fixed behaviour <https://webassembly.github.io/spec/core/exec/numerics.html>, <https://webassembly.github.io/spec/core/appendix/profiles.html>.
- Rust's `f32x4_relaxed_madd` (stable since 1.82) "computes a * b + c with either one rounding or two roundings", so it must not be used <https://doc.rust-lang.org/core/arch/wasm32/fn.f32x4_relaxed_madd.html>.
- `simd128` intrinsics have been stable since 1.54.

**The `libm` crate** *(documented)*:

- 0.2.16, published 2026-01-24, MIT; it now lives in rust-lang/compiler-builtins.
- Reading the 0.2.16 source, its `arch` feature only swaps in hardware for correctly rounded operations (`sqrt`, `fma`, `rint`, `floor`, `ceil`, `trunc`, `fabs`; `src/math/arch/mod.rs`). Every transcendental is the same pure-Rust code on every target.
- Precedents:
  - Rapier's `enhanced-determinism` feature is just `simba/libm_force`. Its docs promise "exact same results" across platforms "including … WASM targets", and warn that `0.4.sin()` breaks this <https://rapier.rs/docs/user_guides/rust/determinism/>.
  - Bevy's `bevy_math` `libm` feature exists "to ensure consistent outputs across platforms at the cost of losing hardware-level optimization" (bevy main `crates/bevy_math/Cargo.toml`, read 2026-10-19).
- Open bug: libm's **software `fmaf` returns wrong results for subnormal outputs** (compiler-builtins #1262, opened 2026-08-09, still open; the same bug is in musl) <https://github.com/rust-lang/compiler-builtins/issues/1262>. On hardware without FMA (including wasm), `mul_add` can therefore disagree with hardware FMA. That is one more reason to avoid `mul_add` and to keep values out of the subnormal range.

**SIMD crates and fused multiply-add** *(documented in source)*:

- `wide` 1.7.1, `f32x4::mul_add`: uses `vfmadd` "On x86/x86_64 with FMA" and `vfmaq_f32` on ARM64 NEON; "Without FMA support: Uses `(self * m) + a` (two roundings)". `wide`'s `exp`, `ln` and `sin_cos` are built on `mul_add`, so their results depend on the target and on `-C target-cpu`.
- `fearless_simd` 1.1.0, `mul_add_f32x4`: "Depending on platform support, the result may be computed with only one rounding error, or … two". It also offers `mul_add_precise` (always a single rounding, slow on wasm).

**Local check A: native vs wasm32 hashes** *(measured, 2026-10-19)*. The crate is `/tmp/enth-research/03/detdsp`; its source is in the Appendix. The voice is 16 voices × (2 polyBLEP saws → 2× TPT one-pole with `tan` coefficient at control rate 32 → `tanh` → envelope with `exp`), 48 kHz, block 64, seeded SplitMix64, 2 s, FNV-1a over the f32 bits.

```
cd /tmp/enth-research/03/detdsp; export CARGO_BUILD_JOBS=2
cargo build --release && ./target/release/detdsp-native hash
cargo build --release --target wasm32-unknown-unknown --lib
node ../scripts/run-wasm.mjs target/wasm32-unknown-unknown/release/detdsp.wasm /tmp/enth-research/03/out-wasm
```

| Variant | Voices | Native x86-64 | wasm32 (node 22 / V8) | Same? |
|---|---|---|---|---|
| libm | 1 | ff54e53079dd0dd3 | ff54e53079dd0dd3 | **yes** |
| libm | 16 | c4511df4887a4736 | c4511df4887a4736 | **yes** |
| std math | 1 | 70c9ff326e36f7a6 | ff54e53079dd0dd3 | no |
| std math | 16 | 6997ae7f09634aea | 5312a45538af405c | no |
| libm + `libm::fmaf` in the filter | 1 / 16 | 0441afe8… / 83a78218… | 0441afe8… / 83a78218… | **yes** |

- The libm hashes stay the same under:
  - native debug (`--target-dir tgt-debug`, opt-level 0);
  - `RUSTFLAGS="-C target-cpu=native"` (AVX2+FMA allowed);
  - wasm `RUSTFLAGS="-C target-feature=+simd128"`;
  - FTZ/DAZ set during the render.
- Sample differences (`python3 -I scripts/cmp.py`), 16 voices:

  | Comparison | Samples differing | Max abs diff |
  |---|---|---|
  | native libm vs wasm libm | 0 | 0 |
  | native std vs wasm std | 64.2% | 2.98e-8 (−150.5 dBFS; peak 0.19) |
  | wasm std vs wasm libm | 0.96% | 1.49e-8 |

**Local check B: std vs libm, function by function** (1,000,000 inputs each; count of bitwise mismatches against libm 0.2.16) *(measured)*:

| Function | glibc 2.39 (native) | wasm32 std |
|---|---|---|
| sinf | 13,562 | 0 |
| expf | 97,069 | 0 |
| tanf | 144,464 | 0 |
| tanhf | 77,539 | 0 |
| powf(2, y) | 98,474 | 98,603 † |
| logf | 19,896 | 0 |
| cosf | 11,904 | 0 |
| `mul_add` vs `libm::fmaf` | 0 | 0 |
| sqrt | 0 | 0 |

† On wasm, std's `powf(2.0, y)` became a call to `llvm.exp2.f32` (seen in the emitted LLVM IR: `cargo rustc --release --target wasm32-unknown-unknown --lib --crate-type cdylib -- --emit=llvm-ir`). The compiler swapped the function. So even "std = libm on wasm" is not reliable.

**Local check C: rustfft SIMD paths** (`/tmp/enth-research/03/fftdet`, forward FFT of a seeded signal, hash of the output) *(measured)*:

| Path | n=256 | n=1000 | n=2048 | n=4096 |
|---|---|---|---|---|
| native default (AVX, runtime-detected) | d9df16… | b9a641… | 74df36… | 1e68cc… |
| native `sse` only | d57612… | a5d363… | 14d29f… | 6fcd79… |
| native scalar (`default-features = false`) | 9e292b… | 746844… | e90924… | 9a1269… |
| wasm32 scalar | 9e292b… | 746844… | e90924… | 9a1269… |
| wasm32 `wasm_simd` + `+simd128` | 9e292b… | a5d363… | 14d29f… | 9a1269… |

- Scalar rustfft is bit-exact between x86-64 and wasm32.
- The AVX path differs from everything else. So "same build, same platform" is **not** enough when a library picks its code path from the CPU at runtime.
- Time per 2048-point FFT: AVX 5.9 µs, SSE 14.8 µs, scalar 42.9 µs (best of 7, loaded machine).

**Cost of the libm path** (`/tmp/enth-research/03/detdsp/src/bin/mathspeed.rs`, latency-bound loop, best of 7, two runs, loaded machine) *(measured)*:

| Function | libm time ÷ glibc time |
|---|---|
| sinf | 1.26–1.58 |
| expf | 1.76–1.86 |
| exp2f | 1.31–1.41 |
| logf | 1.03–1.41 |
| tanf | 0.61–0.62 (libm faster) |
| tanhf | 0.49–0.54 (libm faster) |
| `libm::powf` | ~126 ns per call (about 9× std, whose call was turned into `exp2`) |

- So `pow` should not appear in hot loops. Use `exp2f`, or enth's own approximations.
- Across the whole 16-voice render, the libm and std variants ran within noise of each other.

**Verdict on design.md §5's contract.** "Bit-exact on the same build and platform; tolerance across platforms" is safe, but weaker than the evidence supports. It is also not quite right in the other direction: runtime dispatch, like rustfft's, breaks bit-exactness *within* a platform. The extra cost of cross-platform bit-exactness in the render path:

- libm or enth-owned approximations instead of std, about 1–2× per call;
- no FMA;
- no runtime-dispatched numeric variants;
- a fixed summation order.

SIMD across voices stays allowed, because each lane does the same IEEE operations as scalar code. The win is large. Seed-generated duel targets (§6, §9) involve a "musical" filter that renders audio and applies thresholds. Under a tolerance-only contract, a borderline candidate could be accepted on one machine and rejected on another, and the two duellists would get different targets. Leaderboards and pass thresholds can flip in the same way.

### Q2 — Denormals

- **x86:** MXCSR FTZ is bit 15 and DAZ is bit 6, set through `stmxcsr`/`ldmxcsr` inline asm. `_mm_setcsr` has been deprecated since 1.75. Rust documents the change as UB (Q1) *(documented)*. In practice nih-plug and nice-plug do it anyway.
- **nice-plug 0.4.2:** `crates/nice-plug/src/wrapper/util.rs` (Codeberg `main`, read 2026-10-19) wraps every process call in `process_wrapper`, which says "Make sure FTZ is always enabled, even if the host doesn't do it for us" (`ScopedFtz`: MXCSR bit 15, or AArch64 FPCR bit 24). The same function enters `nice_assert_no_alloc` *(documented)*.
- **aarch64:** FPCR.FZ is bit 24. With FPCR.AH = 0 it flushes both inputs and outputs, single and double precision. FEAT_AFP adds FIZ and AH. *Documented, but via secondary sources*: a mirror of Arm's register XML <https://df.lth.se/~getz/ARM/SysReg/AArch64-fpcr.html> and QEMU commits. developer.arm.com did not resolve from here.
- **wasm32:** there is no FTZ control. The spec qualifies IEEE 754 only for rounding, exceptions and NaNs. Subnormal flushing was moved to "future features" in 2015 (WebAssembly/design #148, #271) and never added *(documented)*. Subnormals run at full IEEE cost in V8 *(measured below)*.
- **Crates:** `no_denormals` 0.3.0 (2025-09-11, MIT); I did not read its source (see Gaps). fundsp has `denormal::prevent_denormals()`, which calls `_mm_setcsr(0x9fc0)` on x86 (FTZ+DAZ), does nothing on other targets, and is deprecated-API UB (read from fundsp 0.23.0 `src/denormal.rs`) *(documented)*.
- **Measured costs** (64 one-pole decays with `a = 0.999` fed silence; they get stuck around 7e-43 because rounding never takes them to 0; 2M samples, median of 5):

  | Mode | Native x86-64 (ns per filter-sample) | wasm32 / V8 |
  |---|---|---|
  | plain (stuck subnormal) | **14.92** | **52.06** |
  | explicit flush `if x.abs() < 1e-15 { x = 0 }` | 0.30 | 2.13 |
  | anti-denormal offset `x = a*x + 1e-18; x -= 1e-18` | 0.20 | 1.37 |
  | FTZ+DAZ on (inline asm; UB per Rust docs) | 0.09 | not available |

  Commands: `./target/release/detdsp-native denorm` and the `denorm` lines of `node ../scripts/run-wasm.mjs …`.

- **Pattern for enth:** keep every piece of recursive state either at zero or above about 1e-15, using an explicit flush at block or control rate (the cost is small) or a DC-offset trick. Avoid products of two tiny numbers that can land in the subnormal range, such as `x*x` in an RMS detector on near-silence.
  - Once no subnormal ever arises, the host's or nice-plug's FTZ setting can't change the output. Plugin renders then equal CLI and wasm renders.
  - I verified this only for the 2 s toy, which has no long tail: its hash was the same with FTZ/DAZ on and off.

### Q3 — Render throughput and SIMD

- **SynthMark** (Google, Apache-2.0; repo last pushed 2025-05-22):
  - The `SimpleVoice` is a sine LFO, a DPW saw and a DPW square, a biquad with cutoff modulated per sample, and filter and amp ADSRs. It runs at 48 kHz in 8-frame renders, with `kSynthmarkMaxVoices = 1024` (`source/SynthMark.h`) *(documented)*.
  - VoiceMark_100, the number of voices that fill one CPU (≈ ×realtime per core for one voice), across 1,785 public results:

    | | Voices |
    |---|---|
    | median | **701** |
    | 25th percentile | 588 |
    | 75th percentile | 814 |
    | i9-12900K | 978 |
    | i7-1165G7 | 889 |
    | Apple M1 | 681 |
    | Cortex-A53 | 84 |

    These come from the OpenBenchmarking mirror (data as of 2025-12-25) <https://mail.openbenchmarking.org/test/pts/synthmark> *(aggregate of third-party runs; the main openbenchmarking.org page is behind Cloudflare and was not fetched)*. Top CPUs saturate near the 1,024 cap.
- **torchsynth** reports 16,200× realtime on a GPU (already in docs/research.md).
- **My toy voice**, scalar code, libm, `tanhf` per sample, not optimized. Under load 13–31; min and median of 15 runs *(measured)*:

  | | 1 voice | 16 voices |
  |---|---|---|
  | native | 281–376× (median), up to 521× (min) | 16–26× (median), up to 36× |
  | wasm32 / V8 | 281–376× (median), 296–422× (min) | 20–22× (median) |

  wasm under V8 runs at roughly native scalar speed for this kind of code. Commands: `./target/release/detdsp-native time` and the node script.
- **Takeaways:**
  - A competent scalar voice should reach several hundred × realtime per core on this laptop.
  - 100× for one voice is a floor, not a target.
  - The solver budget really depends on render **+ feature extraction** per candidate. The analysis FFTs alone cost about 8 ms per 2 s clip per resolution with scalar rustfft at 2048, against about 20 ms for a render at 100×.
- **SIMD on stable Rust** (versions from the crates.io API, 2026-10-19):

  | Crate | Version (date) | Licence |
  |---|---|---|
  | `wide` | 1.7.1 (2026-09-14) | Zlib/Apache-2.0/MIT |
  | `pulp` | 0.22.3 (2026-06-20) | MIT |
  | `fearless_simd` | 1.1.0 (2026-10-06; 1.0.0 was 2026-09-21) | Apache-2.0/MIT |
  | `macerator` | 0.5.1 (2026-09-28) | — |

  - `std::simd` is still nightly-only: tracking issue rust-lang/rust#86656 is open, last updated 2026-08-12.
  - wasm `simd128` has been stable since 1.54, but has to be enabled at compile time (`-C target-feature=+simd128`).
  - For determinism:
    - Lane-parallel voice processing (design.md §3's structure-of-arrays) is safe.
    - The danger lies in `mul_add` helpers, approximations built on them, and `pulp`/`fearless_simd`-style runtime dispatch whose kernels differ numerically.
    - I did not measure SIMD speed-ups.

### Q4 — Benchmarking practice

- **Tools** (versions and dates from crates.io and GitHub, 2026-10-19):

  | Tool | Version (date) | Licence | Notes |
  |---|---|---|---|
  | Criterion | 0.8.2 (2026-02-04) | Apache-2.0/MIT | Repo moved to `criterion-rs`, pushed 2026-10-13 |
  | divan | 0.1.21 (2025-04-10) | MIT/Apache-2.0 | No release for 18 months; repo pushed 2026-07-19 |
  | gungraun | 0.20.0 (2026-09-26) | Apache-2.0/MIT | Formerly iai-callgrind, whose last release was 0.16.1 on 2025-07-30 |
  | hyperfine | 2.0.0 (2026-10-07) | Apache-2.0 | |
  | tango-bench | 0.9.0 (2026-10-16) | MIT | Paired A/B benchmarking against noise |
  | CodSpeed `codspeed-divan-compat` | 5.0.3 (2026-10-14) | MIT/Apache-2.0 | |

  - Criterion FAQ, on CI: "You probably shouldn't (or, if you do, don't rely on the results)… A better alternative is to use Iai" <https://bheisler.github.io/criterion.rs/book/faq.html> *(documented)*.
  - gungraun uses Callgrind, Cachegrind, DHAT or Linux perf. It "cannot be run on Windows", and its cycle estimates are only relative (README) *(documented)*.
  - hyperfine 2.0.0 adds `--metrics` (instructions, cycles, cache misses), runs commands without a shell by default and uses JSON schema v2. Scripts written for 1.x need updating (release notes) *(documented)*.
  - CodSpeed: vendor and third-party claims of under 1% variance with "CPU simulation" *(anecdotal)*.
- **Noise control:** the LLVM benchmarking guide <https://llvm.org/docs/Benchmarking.html> recommends:
  - the `performance` governor;
  - `intel_pstate/no_turbo = 1`;
  - `cset shield` core isolation;
  - taking SMT siblings offline;
  - ASLR off;
  - and claims "<0.1%" variation with all of that in place *(documented)*.

  Every one of these needs sudo. On this laptop today the governor is `powersave`, turbo is on, and `perf_event_paranoid = 4`, which blocks `perf stat` (so hyperfine's instruction metrics won't work either). Valgrind, hyperfine and wasmtime are not installed *(measured)*.
- **Practice for enth:**
  - Criterion (or divan) for local wall-time.
  - Report the min and median of N runs on a quiet machine, with governor and turbo noted.
  - Use instruction counts (gungraun on Linux CI, or hyperfine 2 `--metrics instructions` where perf is allowed) for the "regression > 10% fails CI" rule. Wall-time on hosted runners is too noisy for a 10% gate.

### Q5 — Real-time safety tooling

- **`assert_no_alloc`** 1.1.2 (2021-08-03). The repo's last commit was 2022-09-11 (a nih-plug merge). It has 10 open issues and hard-codes `std::alloc::System`.
  - **LICENSE file read: a one-clause BSD text, `license = "BSD-1-Clause"`.**
  - nice-plug uses a fork, `nice-assert-no-alloc` 1.0.0 (2026-05-17, BillyDM), also BSD-1-Clause.
  - Task 2's `deny.toml` allow-list lacks BSD-1-Clause, so `cargo deny check` will fail as soon as either crate is added *(documented)*.
  - web-audio-api-rs uses `alloc_counter` 0.0.4 (2019) for the same purpose.
- **`rtsan-standalone`** 0.3.0 (2025-09-27). The repo was pushed 2026-09-15; the last commit was 2025-09-27; 104 stars.
  - **The LICENSE file is the full Apache License 2.0** (the appendix boilerplate is unfilled).
  - Linux, macOS and iOS only.
  - The `#[nonblocking]` attribute is turned on with `RTSAN_ENABLE=1`.
  - By default its build script **downloads prebuilt runtime libraries with curl** from `realtime-sanitizer/rtsan-libs`. Building locally needs git, make and cmake ≥ 3.20 (README) *(documented)*.
- **Native rustc support:** PR rust-lang/rust#147935, "Add LLVM realtime sanitizer", merged 2025-11-08. The unstable book documents `-Zsanitizer=realtime` with `#[sanitize(realtime = "nonblocking" | "blocking" | "caller")]` under `#![feature(sanitize)]`, and recommends `-Zbuild-std` <https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html> *(documented)*. Nightly only.
- **Fit:** `assert_no_alloc` catches allocations only. RTSan also catches locks, syscalls and I/O. A nightly or rtsan-standalone job in Linux CI over the engine's `process()` tests is a cheap addition. I did not try either here (see Gaps).

### Q6 — Local wasm32 builds

`rustup target add wasm32-unknown-unknown` took 12.8 s *(measured)*. Each crate was built cold in its own scratch package and target directory with `CARGO_BUILD_JOBS=2`, using `cargo build --release --target wasm32-unknown-unknown`, as a `cdylib` that calls into the dependency (so linking was exercised). Crate count = unique lines of `cargo tree --target wasm32-unknown-unknown -e normal --prefix none`, including the root package and proc-macros *(measured)*:

| Dependency spec | Result | Crates | Cold build | .wasm |
|---|---|---|---|---|
| `libm = "=0.2.16"` | OK | 2 | 2.4 s | 7.7 KB |
| `biquad = "=0.6.0"` | OK (biquad, libm, num-traits) | 4 | 5.9 s | 7.7 KB |
| `rustfft = "=6.4.1"` (default) | OK | 8 | 9.2 s | 268 KB |
| rustfft, `default-features = false, features = ["wasm_simd"]` | OK | 8 | 59.4 s | 1.13 MB |
| **`fundsp = { version = "=0.23.0", default-features = false }`** | **FAIL**: `cannot find macro vec` / `cannot find type Vec` in `src/resample.rs`. Also fails with `cargo check` on native x86-64 | 34 | (fails at 32 s) | — |
| `fundsp`, `default-features = false, features = ["std"]` | OK | 34 | 46.9 s (peak RSS 518 MB) | 106 KB |
| `fundsp` (default: std, files/symphonia, fft) | OK | 71 | 98.3 s | 113 KB |

- The fundsp failure is upstream issue SamiPerttu/fundsp#86. It was opened 2026-01-11 and fixed on master the next day; a user asked for a release on 2026-04-18. The last commit was 2026-03-03, and no release has followed 0.23.0 (2026-01-07) (`gh api`, 2026-10-19).
- ADR-0006's fallback should read `default-features = false, features = ["std"]`, or pin a git revision.
- For comparison, the previous session's figure for fundsp's default wasm32 tree was 81 crates; my count method gives 71.
- Paths: `/tmp/enth-research/03/w-*`.

### Q7 — Golden testing of audio

- **Precedents** *(documented)*:
  - Faust's impulse-response tests compare each backend's output (C++, LLVM, Java, wasm and others) with stored references "with a low tolerance" (`tests/impulse-tests/README.md`).
  - fundsp's tests compare `tick` against `process` with a 1e-4 tolerance (`tests/test_basic.rs`).
  - web-audio-api-rs (MIT, pushed 2026-10-12) uses `float_eq`/`almost` tolerance asserts, `alloc_counter` and `iai` instruction-count benches next to Criterion (`Cargo.toml`).
  - I found no Rust audio project that relies on cross-platform hashes. enth would be unusual here. The measured results say it is feasible.
- **Hashing vs tolerance:**
  - A hash is exact and cheap, and catches any change, including intended DSP changes. That is the point when lessons pin an engine version.
  - Hashing quantized samples is brittle: one rounding-boundary flip changes the hash. Don't do it.
  - A feature summary (RMS per 10 ms, spectral centroid per frame, for example) compared with tolerances explains *what* changed and survives harmless last-bit differences.
  - Recommended pairing: exact SHA-256 or FNV hash for the render; an `insta` snapshot of the feature summary, rounded (e.g. 0.01 dB and 1 Hz), stored next to it as the diagnostic shown when the hash breaks.
  - Hash the **samples after canonicalising NaN** (or assert there are none). Keep long release tails in the rendered performance so subnormal behaviour is exercised.

## Recommendation for Phase 0

1. **Target cross-platform bit-exact renders** for the engine path (L0–L3 render; analysis covered under item 4). Rules for `enth-dsp`/`enth-engine`/`enth-render`:
   - transcendentals only through `libm` pinned with `=` (bumping it is an engine-version change), or enth-owned approximations built from `+ − × ÷ sqrt`;
   - ban `f32::{sin, cos, tan, exp, exp2, ln, log*, powf, powi, tanh, …}`, `mul_add` and `algebraic_*` through `clippy.toml` `disallowed-methods`;
   - no `wide::mul_add` or `fearless_simd` `mul_add`, and none of the transcendentals built on them;
   - no runtime CPU dispatch with numerically different kernels;
   - a fixed summation and voice order;
   - explicit denormal flushing;
   - NaN treated as a bug.
2. **Denormals:** flush in code (AGENTS.md already says so). Keep state either at zero or at magnitudes of 1e-15 and above. Don't rely on, or set, MXCSR/FPCR in engine code. Add a test that renders a patch with a long release tail with FTZ on and off (on native, test-only) and asserts identical hashes, because nice-plug always enables FTZ.
3. **Task 5 as amended:**
   - Hash 1 and 16 voices on native, on wasm32 (node works today with nothing to install; wasmtime + wasip1 after Task 1) and in CI on Windows x86-64, **macOS aarch64 and Linux aarch64**. Record "hashes equal: yes/no". Feature-summary tolerance is only the fallback measurement.
   - Measure ×realtime with Criterion plus min and median of N on a quiet machine (performance governor, turbo noted).
   - Also measure render + features per candidate, for the §7 solver budget.
4. **Analysis/scoring:** choose explicitly between scalar rustfft (bit-exact across platforms; about 7× slower than AVX at n = 2048 here) and accepting tolerance for scores. With tolerance, make every threshold decision (pass/fail, randomizer acceptance) robust, for example by computing it from a canonical reference render or by adding hysteresis. Coordinate with researcher 04.
5. **CI benchmarks:** instruction counts (gungraun on Linux CI runners; hyperfine 2 `--metrics instructions` locally once perf is allowed) for the 10% regression gate. Criterion for local wall-time.
6. **Real-time checks:** keep `assert_no_alloc` (add BSD-1-Clause to `deny.toml`). Add one Linux CI job with nightly `-Zsanitizer=realtime`, or `rtsan-standalone` built from source rather than prebuilt downloads.
7. **ADR-0006 fallback:** `fundsp = { version = "=0.23.0", default-features = false, features = ["std"] }`. Plain `default-features = false` does not compile.

## Plan check — what in the current docs the evidence says should change

| Doc | Change | Why |
|---|---|---|
| design.md §5 | Replace "Bit-exact on the same build and platform; across platforms and wasm32, equal within a tolerance on a feature summary" with "Bit-exact across supported platforms (x86-64, aarch64, wasm32) for the render path, enforced by golden hashes in CI; a feature summary with tolerances is the diagnostic when a hash breaks". Add the numeric rules (libm only, no FMA, no `algebraic_*`, no runtime-dispatched numeric kernels, fixed order, flush in code). Change "Flush denormals" to "flush denormals in code; never depend on FTZ/DAZ (nice-plug enables FTZ during `process`; wasm has none)". Keep the tolerance contract as the documented fallback **until aarch64 CI confirms**. | Measured bit-exact x86-64 ↔ wasm32 at little cost. Within-platform runtime dispatch (rustfft) breaks the current wording. Duels, seed targets and leaderboards need cross-machine equality (§6, §9). |
| design.md §6/§9 (note) | State that randomizer acceptance tests must give identical decisions on every platform, which bit-exact renders and analysis provide. | Otherwise "identical on every machine" (§6) isn't guaranteed. |
| design.md §7 | The budget should count render + feature extraction per candidate, not render alone. | Analysis FFTs are a comparable cost (measured FFT timings). |
| phase-0.md Task 5 | Compare **hashes** across native, wasm32 and CI (Windows, macOS aarch64, Linux aarch64), and record the feature-summary maximum difference only where hashes differ. Allow `wasm32-unknown-unknown` under node as the wasm runner (works now) next to wasip1 + wasmtime. Add the FTZ on/off hash test, a long-tail denormal timing test, and the clippy `disallowed-methods` ban. Note that `assert_no_alloc` needs BSD-1-Clause in `deny.toml`. | Measured feasibility; licence check. |
| phase-0.md Task 2 | Add `BSD-1-Clause` to the `deny.toml` allow-list (or a per-crate exception). | `assert_no_alloc` and `nice-assert-no-alloc` are BSD-1-Clause (LICENSE files read). |
| benchmarks.md, engine table | Determinism row: "hashes identical across platforms" (not just the same platform). "Regressions > 10% fail CI": measure in **instruction counts** (gungraun/Valgrind or perf), not Criterion wall-time on hosted runners. The 100× floor can stay as the pass line; record the real number, and add a 16-voice figure (SynthMark's median comparable voice is about 700× per core). Add an RTSan row (nightly or rtsan-standalone) next to `assert_no_alloc`. | Criterion FAQ; SynthMark data; tool status. |
| ADR-0006 | Fallback spec → `default-features = false, features = ["std"]` (or a git revision). | 0.23.0 no_std build broken (measured; fundsp#86). |
| AGENTS.md | Add: "in L0–L3, no std float transcendentals, `mul_add` or `algebraic_*`; use `enth_dsp::math` (libm-backed); no runtime-dispatched numeric kernels in the render path". | Enforces the contract for AI-written code. |

Only Rust and a native UI are fixed. Nothing here touches either.

## Gaps — not verified, and the check that would settle each

Scratch code lives in `/tmp/enth-research/03/` and **may be deleted at reboot** (Ubuntu-family systems clean `/tmp` on boot). The core source is in the Appendix so it can be recreated.

1. **aarch64 hashes (not run; no hardware or emulator).** Settles whether the cross-platform contract can be adopted. Recreate `detdsp` from the Appendix, then run `cargo run --release --bin detdsp-native hash` on a GitHub `macos-14` and `ubuntu-24.04-arm` runner. Compare with: libm 1 voice `ff54e53079dd0dd3`, 16 voices `c4511df4887a4736`; libm+fmaf `0441afe8b14e6f5a` / `83a78218f7a48b3f`.
2. **Windows x86-64 (MSVC) hashes (not run).** Same command on `windows-latest`.
3. **wasmtime / wasm32-wasip1, Firefox (SpiderMonkey) and Safari (JSC) not run.** Only V8 through node was tested. After Task 1: `rustup target add wasm32-wasip1`, build, `wasmtime run`. Compare hashes.
4. **Timings taken under load 13–31, `powersave`, turbo on.** After the reboot, with no other builds running: `./target/release/detdsp-native time`, `./target/release/mathspeed`, `fftspeed`. Ideally also `sudo cpupower frequency-set -g performance; echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo` (owner action).
5. **perf blocked (`perf_event_paranoid = 4`); Valgrind and hyperfine missing.** Owner: `sudo sysctl kernel.perf_event_paranoid=1`, `sudo apt install valgrind hyperfine` (Task 1). Then `perf stat -e cycles,instructions ./target/release/detdsp-native time` and a gungraun trial.
6. **FTZ on/off equality only checked on a 2 s render without a long tail.** Render 10 s with release to silence, with and without FTZ, and compare hashes and per-block timings.
7. **`no_denormals` 0.3.0 source not read** (already in `~/.cargo/registry/src/*/no_denormals-0.3.0/`). Check what it does on wasm and whether it uses `_mm_setcsr`.
8. **SynthMark numbers come from the OpenBenchmarking mirror only** (main site Cloudflare-blocked; not bypassed). Building and running SynthMark locally (C++, `linux/` makefile) would need the owner's OK, since it is outside the allowed cargo-only commands.
9. **SIMD speed-ups not measured** (`wide`, `pulp`, `fearless_simd`, wasm `simd128`). A 4-voice lane prototype in `spike-engine` would settle it.
10. **RTSan not tried locally.** Its default build downloads prebuilt binaries, so it needs explicit approval or a from-source build (cmake). Also nightly `-Zsanitizer=realtime`.
11. **fundsp master's no_std fix not built.** `fundsp = { git = "https://github.com/SamiPerttu/fundsp", default-features = false }` for wasm32.
12. **Arm FPCR semantics** come from secondary sources (developer.arm.com unresolvable from here). Read the Arm ARM FPCR page.
13. **CodSpeed claims** (variance, OSS pricing) unverified; vendor docs returned 404 or no details.
14. **The toolchain is behind:** the local rustc is 1.95.0 while stable is 1.99.1 (2026-10-13). The hashes should be re-run on whatever Task 2 pins. With libm pinned, they should not change.

## Appendix — `detdsp` source (for recreating the hash check)

`Cargo.toml`: package `detdsp`, edition 2024, `[lib] crate-type = ["cdylib", "rlib"]`, bin `detdsp-native` at `src/main.rs`, `libm = "=0.2.16"`, release profile `opt-level = 3, codegen-units = 1, panic = "abort"`.

<details><summary>src/lib.rs (render, hash, denormal stress, std-vs-libm census, wasm exports)</summary>

```rust
pub trait Math {
    fn sin(x: f32) -> f32; fn exp(x: f32) -> f32; fn tan(x: f32) -> f32;
    fn tanh(x: f32) -> f32; fn pow(x: f32, y: f32) -> f32;
    #[inline(always)] fn madd(a: f32, b: f32, c: f32) -> f32 { a * b + c }
}
pub struct Libm; pub struct Std; pub struct LibmFma;
impl Math for Libm {
    #[inline(always)] fn sin(x: f32) -> f32 { libm::sinf(x) }
    #[inline(always)] fn exp(x: f32) -> f32 { libm::expf(x) }
    #[inline(always)] fn tan(x: f32) -> f32 { libm::tanf(x) }
    #[inline(always)] fn tanh(x: f32) -> f32 { libm::tanhf(x) }
    #[inline(always)] fn pow(x: f32, y: f32) -> f32 { libm::powf(x, y) }
}
impl Math for Std {
    #[inline(always)] fn sin(x: f32) -> f32 { x.sin() }
    #[inline(always)] fn exp(x: f32) -> f32 { x.exp() }
    #[inline(always)] fn tan(x: f32) -> f32 { x.tan() }
    #[inline(always)] fn tanh(x: f32) -> f32 { x.tanh() }
    #[inline(always)] fn pow(x: f32, y: f32) -> f32 { x.powf(y) }
}
impl Math for LibmFma { /* as Libm, plus: */
    #[inline(always)] fn sin(x: f32) -> f32 { libm::sinf(x) }
    #[inline(always)] fn exp(x: f32) -> f32 { libm::expf(x) }
    #[inline(always)] fn tan(x: f32) -> f32 { libm::tanf(x) }
    #[inline(always)] fn tanh(x: f32) -> f32 { libm::tanhf(x) }
    #[inline(always)] fn pow(x: f32, y: f32) -> f32 { libm::powf(x, y) }
    #[inline(always)] fn madd(a: f32, b: f32, c: f32) -> f32 { libm::fmaf(a, b, c) }
}
pub const SR: f32 = 48_000.0; pub const BLOCK: usize = 64; pub const CTRL: usize = 32;
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { // SplitMix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 { (self.next() >> 40) as f32 / (1u64 << 24) as f32 }
}
#[inline(always)]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt { let x = t / dt; x + x - x * x - 1.0 }
    else if t > 1.0 - dt { let x = (t - 1.0) / dt; x * x + x + x + 1.0 } else { 0.0 }
}
struct Voice { p1: f32, p2: f32, dt1: f32, dt2: f32, s1: f32, s2: f32, lfo: f32, gain: f32 }
pub fn render<M: Math>(voices: usize, seconds: f32, seed: u64, out: &mut Vec<f32>) {
    let n = (seconds * SR) as usize; out.clear(); out.resize(n, 0.0);
    let mut rng = Rng(seed);
    let mut vs: Vec<Voice> = (0..voices).map(|v| {
        let note = 36.0 + ((v * 7) % 36) as f32;
        let det = (rng.unit() - 0.5) * 0.2;
        let f = 440.0 * M::pow(2.0, (note - 69.0 + det) / 12.0);
        Voice { p1: rng.unit(), p2: rng.unit(), dt1: f / SR, dt2: f * 1.005 / SR, s1: 0.0, s2: 0.0,
                lfo: rng.unit() * 6.283_185, gain: 0.5 / voices as f32 }
    }).collect();
    let release_at = (1.5 * SR) as usize;
    let mut i = 0usize;
    while i < n {
        let end = (i + BLOCK).min(n);
        for v in vs.iter_mut() {
            let mut j = i;
            while j < end {
                let cend = (j + CTRL).min(end);
                let t = j as f32 / SR;
                let fenv = M::exp(-t * 3.0);
                v.lfo += 6.283_185 * 0.5 * CTRL as f32 / SR;
                if v.lfo > 6.283_185 { v.lfo -= 6.283_185; }
                let fc = 200.0 + 6000.0 * fenv * (1.0 + 0.2 * M::sin(v.lfo));
                let g = M::tan(3.141_592_7 * fc / SR);
                let a = g / (1.0 + g);
                let amp = if j < (0.005 * SR) as usize { j as f32 / (0.005 * SR) }
                          else if j < release_at { 0.6 + 0.4 * M::exp(-(t - 0.005) * 4.0) }
                          else { (0.6 + 0.4 * M::exp(-(1.495) * 4.0)) * M::exp(-(t - 1.5) * 10.0) };
                for k in j..cend {
                    let s = (2.0 * v.p1 - 1.0 - poly_blep(v.p1, v.dt1))
                          + (2.0 * v.p2 - 1.0 - poly_blep(v.p2, v.dt2));
                    v.p1 += v.dt1; if v.p1 >= 1.0 { v.p1 -= 1.0; }
                    v.p2 += v.dt2; if v.p2 >= 1.0 { v.p2 -= 1.0; }
                    let d1 = s * 0.5 - v.s1; let y1 = M::madd(a, d1, v.s1); v.s1 = M::madd(a, d1, y1);
                    let d2 = y1 - v.s2; let y2 = M::madd(a, d2, v.s2); v.s2 = M::madd(a, d2, y2);
                    out[k] += M::tanh(y2 * 1.5) * amp * v.gain;
                }
                j = cend;
            }
        }
        i = end;
    }
}
pub fn fnv1a(xs: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in xs { for b in x.to_bits().to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x0100_0000_01b3); } }
    h
}
pub fn run(variant: u32, voices: usize, out: &mut Vec<f32>) {
    match variant {
        0 => render::<Libm>(voices, 2.0, 0xE17_5EED, out),
        1 => render::<Std>(voices, 2.0, 0xE17_5EED, out),
        _ => render::<LibmFma>(voices, 2.0, 0xE17_5EED, out),
    }
}
/// mode 0 plain, 1 explicit flush (abs < 1e-15 -> 0), 2 anti-denormal offset
pub fn denorm(mode: u32, samples: usize) -> f32 {
    let mut s = [1.0f32; 64]; let a = 0.999f32; let mut acc = 0.0f32;
    for _ in 0..samples {
        for x in s.iter_mut() {
            match mode {
                0 => { *x = a * *x; }
                1 => { *x = a * *x; if x.abs() < 1e-15 { *x = 0.0; } }
                _ => { *x = a * *x + 1e-18; *x -= 1e-18; }
            }
        }
        acc += s[0];
    }
    acc + s.iter().sum::<f32>()
}
// census(func, n): counts std-vs-libm bitwise mismatches over n inputs (see Q1 table); omitted for length.
static mut BUF: Vec<f32> = Vec::new();
#[unsafe(no_mangle)] pub extern "C" fn wasm_hash(variant: u32, voices: u32) -> u64 {
    let mut out = Vec::new(); run(variant, voices as usize, &mut out); fnv1a(&out) }
#[unsafe(no_mangle)] #[allow(static_mut_refs)]
pub extern "C" fn wasm_render(variant: u32, voices: u32) -> *const f32 {
    unsafe { run(variant, voices as usize, &mut BUF); BUF.as_ptr() } }
#[unsafe(no_mangle)] #[allow(static_mut_refs)] pub extern "C" fn wasm_len() -> u32 { unsafe { BUF.len() as u32 } }
#[unsafe(no_mangle)] pub extern "C" fn wasm_denorm(mode: u32, samples: u32) -> f32 { denorm(mode, samples as usize) }
```

</details>

Node runner (`run-wasm.mjs`): `WebAssembly.instantiate(fs.readFileSync(path), {})`, then call `wasm_hash(variant, voices)` and print `BigInt.asUintN(64, h).toString(16)`. The wasm module needs no imports.
