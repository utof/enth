# 03 — Engine speed and determinism (final)

- **Date:** 2026-10-19
- **Sub-researcher:** 03 (engine speed and determinism, plus local wasm32 builds)
- **Questions answered:** all seven:
  1. float determinism;
  2. denormals;
  3. throughput and SIMD;
  4. benchmarking;
  5. real-time safety tooling;
  6. local wasm32 builds and native-vs-wasm hashes;
  7. golden testing.

  Everything listed under the first report's Gaps that can be done on this machine is now done. The rest (aarch64 and Windows hashes) has a ready CI job (Appendix A).
- **Relation to the first report:** `03-engine-determinism.md` stays as written; it was cut short by a wrap-up call before a reboot. This report replaces it as the self-contained version. Changes are marked **(new)**, **(changed)** or **(confirmed)**. A "What changed" table follows the brief.
- **Machine:**
  - i7-9750H (6 cores / 12 threads), Linux Mint 22.3, kernel 7.0.0, glibc 2.39.
  - rustc/cargo 1.95.0 (LLVM 22.1.2); nightly 1.101.0 (2026-10-18), used only for RTSan and installed in a scratch `RUSTUP_HOME`.
  - node v22.22.3; wasmtime 48.0.6 (installed in scratch); valgrind 3.22.0; hyperfine 1.18.0 (apt) and 2.0.0 (installed in scratch).
  - `perf_event_paranoid = 4`, which blocks hardware counters. Governor `powersave`, turbo on, SMT on.
- **About the timings:** every wall-clock run held the shared session lock exclusively (`flock -x`), with `uptime` recorded. Other researchers' processes running outside the lock, mainly a python job at up to 900% CPU, dart and cinnamon, still loaded the machine (load average 2.3–26). Read wall-clock numbers as ±20%. Instruction counts (valgrind) are exact and don't depend on load. The hash results don't depend on timing at all.
- **Markings:** *measured* = run here today, commands given. *documented* = official docs, specs or source code. *anecdotal* = vendor, third-party or aggregate claims.
- **Scratch code** (persistent, outside the repo): `~/code/enth-scratch/2026-10-19-phase-0-spikes/03/`, holding `detdsp`, `engine2`, `smbench`, `fftdet`, `featcmp`, `rtsan-test`, `fundsp-speed`, `bench-*`, `ci/golden-hashes.yml` and `notes.md`. Raw outputs are in `timing-batch{1,2,3}.txt`, `bench-wall.txt`, `smbench-timing.txt`, `wasmtime-*.txt` and `fundsp-timing.txt`.

## Answer in brief

1. **Cross-platform bit-exact renders are cheap. Require them for the engine render path, and keep a feature-summary tolerance only as the diagnostic when a hash breaks.** Measured, with identical hashes throughout:
   - Two test engines matched across native x86-64, wasm32 under V8 (node) and wasm32 under Cranelift (wasmtime).
   - They matched across debug, release, `opt-level` s and 1, fat LTO, `-C target-cpu=native` and `+fma`.
   - They matched for wasm baseline and `+simd128`.
   - They matched between scalar code and `wide` f32x4/f32x8 SIMD lanes.
   - The rules: transcendentals only through the pinned `libm` crate or our own `+ − × ÷` approximations; no `mul_add`; fixed summation order; explicit denormal flushing. aarch64 and Windows are not yet run (CI job in Appendix A).
2. **The traps are now measured, not just documented:**
   - `std` maths differs between glibc and wasm (std vs libm differs on up to 14% of inputs).
   - LLVM rewrote `powf(2,x)` into `exp2`.
   - `wide::mul_add` changes its output as soon as the build enables FMA.
   - rustfft returns three different bit results on one machine (AVX, SSE, scalar).
   - C++ built with `-Ofast` differs from the same code at `-O2` by up to 9.5e-3.
3. **Denormals: flush in code, and end voices below −100 dB.** Over a 40 s release tail, a naive engine's hash changes with the FTZ setting, its blocks cost 66–78 µs instead of 3.5 µs, and the whole render takes 36× longer.
   - The flushed engine gives the **same hash with FTZ off, FTZ only (what nice-plug sets), FTZ+DAZ and on wasm**, at about 1 µs per block.
   - Setting MXCSR/FPCR is documented UB in Rust.
   - `no_denormals` fails to compile for wasm32.
4. **Throughput on this laptop, per voice per core:**

   | Implementation | ×realtime |
   |---|---|
   | Google SynthMark's subtractive voice, C++ `-O2` | ≈ 500 |
   | The same voice ported to Rust, **bit-identical output** | ≈ 505–550 |
   | C++ `-Ofast` (fast-math) | ≈ 550 |
   | Lean hand-written scalar voice (`engine2`) | ≈ 2,000 |
   | fundsp 0.23 voice | 210 (tick) / 445 (block) |

   - SIMD lanes add 1.35× (SSE2) and 1.8–1.9× (AVX2).
   - Under wasm the same SIMD code ran 1.17× faster on V8 and 0.65× (slower) on Cranelift.
   - G0's 100× floor is easy to meet. 16 voices at 50–236× is the number that matters.
5. **Benchmarking:**
   - With no code change between runs, Criterion reported two "Performance has improved" results (−13%, −36%, p ≤ 0.01) under load.
   - Instruction counts (callgrind/gungraun) were identical on every run.
   - Instruction counts under-report the denormal penalty about 18×, so the CI gate needs instruction counts **plus** a deterministic "no subnormals" test.
   - hyperfine 2's instruction metrics are blocked here by `perf_event_paranoid`.
6. **Real-time tooling:**
   - `assert_no_alloc` works in debug, and the negative control is caught. In release it doesn't compile (`AllocDisabler` is missing), so gate tests on `debug_assertions`. Its licence is **BSD-1-Clause**, which Task 2's `deny.toml` lacks.
   - **RTSan works on nightly with no extra downloads**: the runtime ships in rust-std. It caught a `malloc` and passed the engine. It did **not** catch an uncontended `std::sync::Mutex`.
7. **Plan changes:**
   - design.md §5: bit-exact across platforms, plus FTZ independence.
   - Task 1: `cargo install --locked wasmtime-cli` **fails on Rust 1.95** because the latest needs 1.96.
   - Task 2: `deny.toml` needs BSD-1-Clause.
   - Task 5: compare hashes, test lanes = scalar, test FTZ on = off, count subnormals, add RTSan.
   - benchmarks.md: instruction counts plus the subnormal test.
   - ADR-0006: the fundsp fallback needs `features = ["std"]`. **fundsp master is still broken without std.**

## What changed versus the first report

| Topic | First report | Now |
|---|---|---|
| Native vs wasm hashes | Equal under node | **Confirmed**, plus wasmtime 48.0.6 (Cranelift), fat LTO, opt-level s and 1, `+fma`, and a second, more realistic engine (new) |
| SIMD lanes | "Lane-parallel is safe" (documented argument) | **Measured** (new): scalar = f32x4 = f32x8 bit for bit, native and wasm. `wide::mul_add` changes its hash under `+fma` and `target-cpu=native` |
| Long-tail FTZ | Checked only on a 2 s toy with no tail (gap) | **Done** (new): a 40 s tail separates naive from flushed engines (table in Q2) |
| `no_denormals` | Source not read (gap) | **Read and built** (new): `compile_error!` on anything other than x86, x86_64 and aarch64; uses deprecated `_mm_setcsr`, which is UB |
| fundsp no_std | "Fixed on master 2026-01-11, unreleased" | **Changed**: master (commit 5595840, 2026-03-03) still fails without std, in `oversample.rs` and `realseq.rs`. Only `features = ["std"]` builds |
| Timings | Taken at load 13–31 | **Re-measured** at lower load: detdsp 1 voice 681–778× (was 281–376×). Denormal stress 10.9 vs 0.05 ns (was 14.9 vs 0.09). libm/glibc ratios and FFT timings updated (changed numbers, same conclusions) |
| Throughput anchor | Published SynthMark aggregate only | **Added** a same-machine SynthMark voice (C++ `-O2`, C++ `-Ofast`, bit-identical Rust port), fundsp and engine2 (new) |
| Benchmark tools | Documented only; Valgrind and hyperfine missing | **Measured** (new): Criterion false positives, gungraun exact, hyperfine σ 16% under load, compile times, Ir under-reports denormals 18×, hyperfine 2 metrics blocked |
| `assert_no_alloc` | Licence checked | **Tested** (new): debug works and catches the negative control; release has a compile error |
| RTSan | Not tried | **Tested on nightly** (new): runtime ships in rust-std; catches malloc; misses an uncontended Mutex |
| wasmtime | Not installed | **Installed** (new): 48.0.6, since 49.x needs Rust 1.96. Task 1's install command fails on 1.95 |
| Cross-platform tolerance | Max sample difference only | **Feature-summary differences** for the std path (new): ≤ 3.3e-7 dB RMS, ≤ 1.4e-3 Hz centroid |
| Arm FPCR | Secondary sources | **Confirmed** from Arm's register XML (mirror): FZ flushes inputs and outputs when AH = 0 |
| CodSpeed | Anecdotal | **Documented** (vendor docs): one simulated run; free plan for open source |
| aarch64 | Gap | CI job written and YAML-validated (Appendix A); runners documented as free for public repos. Still not run |

## Findings

### Q1 — Float determinism across x86-64, aarch64 and wasm32

**What Rust guarantees** *(documented, read 2026-10-19; stable docs 1.99.1, released 2026-10-13)*:

- `+ − × ÷` round "according to the roundTiesToEven direction defined in IEEE 754-2008".
- `sqrt` and `mul_add` are "guaranteed to be the rounded infinite-precision result … guaranteed not to change".
- Transcendentals (`sin`, `exp`, `powf`, `tan`, `tanh`, `ln`, …): "The precision of this function is non-deterministic. This means it varies by platform, Rust version, and can even differ within the same execution from one invocation to the next."
- NaN results have a non-deterministic sign and payload.
- `algebraic_add/sub/mul/div/rem` are **stable since 1.98.0** and "non-deterministic" (they allow reassociation and contraction); on 1.95 they are unstable *(measured, E0658)*. Source: <https://doc.rust-lang.org/std/primitive.f32.html>.
- RFC 3514 (merged 2024-07-27; tracking issue #128288 open): Rust follows IEEE 754-2008 "with some caveats around operations producing NaNs", "without abruptUnderflow/flush-to-zero", and "it is UB for inline assembly to alter the behavior of floating-point operations in any way". Rust never contracts `a*b+c` into an FMA <https://rust-lang.github.io/rfcs/3514-float-semantics.html>.
- `_mm_setcsr` (deprecated since 1.75.0): changing the masking, rounding or DAZ flags "leads to immediate Undefined Behavior" <https://doc.rust-lang.org/core/arch/x86_64/fn._mm_setcsr.html>.

**wasm32** *(documented)*:

- WebAssembly 3.0 (spec dated 2026-10-13): IEEE 754 arithmetic, round-to-nearest ties-to-even, non-stop mode. NaN payloads are non-deterministic. The deterministic profile makes NaNs canonical and fixes the behaviour of relaxed-SIMD instructions <https://webassembly.github.io/spec/core/exec/numerics.html>, <https://webassembly.github.io/spec/core/appendix/profiles.html>.
- Rust's `f32x4_relaxed_madd` (stable since 1.82) "computes a * b + c with either one rounding or two roundings", so it must not be used. `simd128` has been stable since 1.54.

**`libm` and other crates** *(documented)*:

- `libm` 0.2.16: 2026-01-24, MIT, now lives in rust-lang/compiler-builtins. Its `arch` feature only uses hardware for correctly rounded operations (`sqrt`, `fma`, `rint`, `floor`, `ceil`, `trunc`, `fabs`). All transcendentals are the same Rust code on every target.
- Open bug compiler-builtins #1262 (opened 2026-08-09): libm's software `fmaf` is wrong for subnormal results. The same bug is in musl.
- Precedents: Rapier's `enhanced-determinism` feature is `simba/libm_force`; its docs promise identical results across IEEE 754-2008 platforms "including … WASM targets" <https://rapier.rs/docs/user_guides/rust/determinism/>. Bevy's `bevy_math` `libm` feature exists "to ensure consistent outputs across platforms".
- SIMD crates:
  - `wide` 1.7.1 (2026-09-14): `mul_add` is fused on x86 with FMA and on aarch64 NEON, and unfused otherwise. Its `exp`, `ln` and `sin_cos` are built on `mul_add`.
  - `fearless_simd` 1.1.0 (2026-10-06): `mul_add` "may be computed with only one rounding error, or … two"; `mul_add_precise` always rounds once.

**Measured A — the original deterministic voice (`detdsp`, source in the first report's appendix).** Hashes, 2 s at 48 kHz:

| Variant | Voices | Native x86-64 | wasm32 V8 (node 22) | wasm32 Cranelift (wasmtime 48.0.6) |
|---|---|---|---|---|
| libm | 1 / 16 | ff54e53079dd0dd3 / c4511df4887a4736 | same | same |
| std maths | 1 / 16 | 70c9ff326e36f7a6 / 6997ae7f09634aea | ff54e530… / 5312a45538af405c | same as V8 |
| libm + `libm::fmaf` | 1 / 16 | 0441afe8b14e6f5a / 83a78218f7a48b3f | same | same |

- The libm hashes are unchanged under debug, release, `-C target-cpu=native`, `CARGO_PROFILE_RELEASE_LTO=fat`, `opt-level = "s"`, `opt-level = 1`, wasm `+simd128` and FTZ/DAZ.
- The same hashes reproduced across two reboots.
- Commands: `cargo build --release`, then `detdsp-native hash`; for wasm, `node scripts/run-wasm.mjs <wasm>` and `wasmtime run --invoke wasm_hash detdsp.wasm <variant> <voices>`.

**Measured B — a more realistic engine (`engine2`, new).** Each voice is 2 polyBLEP saws → 2× TPT one-pole with `libm::tanf` at control rate → rational tanh → envelope. It comes as a scalar bank and as `wide` f32x4/f32x8 lanes using the same IEEE operations in the same order, plus a Freeverb-style tail:

| Build | Held notes 2 s, 1/4/7/16 voices: Scalar = X4 = X8? | X4 with `wide::mul_add` |
|---|---|---|
| native baseline (SSE2) | **yes** (960546cb…, b56e6f14…, a47eeebc…, 6601aea2…) | = scalar (unfused) |
| native `+fma` and `target-cpu=native` | **yes**, same hashes | **differs** (dbd9edfa…, ccd07468…, ab719e5d…, cbcb4f71…) |
| fat LTO | **yes**, same hashes | = scalar |
| wasm32 baseline and `+simd128`, node and wasmtime | **yes**, same hashes | = scalar |

**Measured C — std vs libm, function by function** (10⁶ inputs each; counts of results that differ bitwise from libm 0.2.16):

| Function | glibc 2.39 | wasm std |
|---|---|---|
| sinf | 13,562 | 0 |
| expf | 97,069 | 0 |
| tanf | 144,464 | 0 |
| tanhf | 77,539 | 0 |
| powf(2,y) | 98,474 | 98,603 † |
| logf | 19,896 | 0 |
| cosf | 11,904 | 0 |
| mul_add vs fmaf | 0 | 0 |
| sqrt | 0 | 0 |

† The emitted LLVM IR shows `llvm.exp2.f32`: LLVM replaced the function.

**Measured D — rustfft 6.4.1 code paths** (hash of an FFT output on one machine):

- AVX, SSE and scalar paths give three different results.
- Scalar on x86-64 equals wasm32 scalar.
- wasm `simd128` matches the SSE path at n = 1000 and 2048, and the scalar path at n = 256 and 4096.

So "bit-exact on the same build and platform" fails within one platform whenever a library picks its code path from the CPU at runtime.

**Measured E — C++ fast-math vs strict, on the same source (new).**

- Google SynthMark's synthesizer (Apache-2.0, commit 9f41b3f), compiled with g++ at `-O2`, and my Rust port that keeps C++'s float-to-double promotions are **bit-identical** at 1, 4, 16 and 64 voices (hashes 75242bcd…, 15fa9719…, 06ce46ed…, 9e536703…).
- The same C++ at upstream's `-Ofast` differs from both, by up to 9.46e-3 (4 voices) and 2.8e-3 (16 voices).
- So golden tests of a port against its C++ original, as ADR-0006 plans for the Signalsmith reverb, are feasible, provided the C++ is built without fast-math and the promotions are copied.

**Measured F — size of the cross-platform difference when std maths is used** (feature summary: RMS per 10 ms frame and spectral centroid per frame, Hann 2048, hop 480, 196 frames, 16 voices):

| Comparison | Max ΔRMS | Max Δcentroid | Max Δsample |
|---|---|---|---|
| native std vs wasm std | 3.28e-7 dB | 1.37e-3 Hz (relative 4.5e-6) | 2.98e-8 |
| libm path | 0 | 0 | 0 |

The differences are tiny. The danger lies elsewhere: anything that makes a threshold decision (randomizer acceptance, pass/fail) or behaves chaotically (feedback, sync) can amplify them.

**Cost of the libm path** (latency-bound ns per call, `flock -x`, load 15 → 10):

| Function | glibc | libm | libm ÷ glibc |
|---|---|---|---|
| sinf | 7.89 | 18.51 | 2.35 |
| expf | 6.45 | 11.91 | 1.85 |
| exp2f | 6.98 | 9.02 | 1.29 |
| logf | 8.01 | 11.86 | 1.48 |
| tanf | 18.20 | 16.21 | 0.89 |
| tanhf | 27.17 | 13.53 | 0.50 |
| powf | 6.84 (rewritten to exp2) | 77.36 | — |

- Avoid `powf` in hot code.
- In a full render the cost largely disappears: the engine2 voice calls libm only at control rate and runs at about 2,000× realtime per voice.

**Verdict.** design.md §5's contract ("bit-exact on the same build and platform; tolerance across platforms") is weaker than the evidence supports. It is also wrong in one direction: runtime CPU dispatch breaks it within a platform. The cost of cross-platform bit-exactness in the render path:

- libm or our own approximations (1–2.4× per call; negligible at control rate);
- no FMA;
- no runtime-dispatched numeric kernels;
- fixed summation order;
- explicit flushing.

SIMD across voices stays allowed (measured equal). aarch64 still has to be confirmed (Appendix A), but nothing documented predicts a difference there: libm's aarch64 path uses only correctly rounded instructions, and Rust doesn't contract.

### Q2 — Denormals

- **Mechanisms** *(documented)*:
  - x86 MXCSR: FTZ is bit 15, DAZ is bit 6; set through `stmxcsr`/`ldmxcsr` asm. Rust documents changing them as UB.
  - aarch64 FPCR.FZ (bit 24). Arm's register description (2026-03 SysReg XML, via a mirror; developer.arm.com doesn't resolve here): with FZ = 1, "If FPCR.AH is 0, denormalized single-precision and double-precision inputs to, and outputs from, floating-point instructions are flushed to zero. If FPCR.AH is 1, denormalized … outputs … are flushed to zero." FIZ (bit 0, FEAT_AFP) flushes inputs.
  - wasm has no FTZ control (spec; WebAssembly/design #148 and #271 from 2015).
- **nice-plug 0.4.2** wraps every process call in `process_wrapper`, which says "Make sure FTZ is always enabled, even if the host doesn't do it for us". It sets FTZ only (MXCSR bit 15, or FPCR bit 24), and the same function enters `nice_assert_no_alloc` (`crates/nice-plug/src/wrapper/util.rs`, Codeberg `main`, read 2026-10-19) *(documented)*.
- **Crates:**
  - `no_denormals` 0.3.0 (2025-09-11, MIT, © 2023 Sintel): an RAII guard setting FTZ|DAZ (0x8040) through the deprecated `_mm_getcsr`/`_mm_setcsr` on x86, and FZ (+ FZ16 with fp16) on aarch64. It is an `unsafe fn` whose docs admit the UB. It has `compile_error!("This crate only supports x86, x86_64 and aarch64.")`, and a **wasm32 build fails with exactly that error** *(measured, new)*. So it can't be in L0–L5.
  - fundsp's `prevent_denormals()` calls `_mm_setcsr(0x9fc0)` on x86 and does nothing elsewhere.
- **Stress test, re-measured** (64 one-pole decays that get stuck in the subnormal range; ns per filter-sample; `flock -x`, load 15 → 10):

  | Mode | Native | wasm (V8) |
  |---|---|---|
  | plain | **10.89** | **47.12** |
  | flush threshold `abs < 1e-15 → 0` | 0.14 | 1.24 |
  | anti-denormal offset ±1e-18 | 0.12 | 0.78 |
  | FTZ+DAZ (asm, UB) | 0.05 | n/a |

- **Long release tail (new; the first report's main open check).** `engine2` with 4 notes held 1 s and released, through the Freeverb-style tail, 40 s rendered:
  - *Naive* = no flushing, voices never end.
  - *Flushed* = state flushed at control rate (`abs < 1e-15 → 0`), comb and allpass writes flushed, voices end when the envelope falls below 1e-5 (−100 dB).

  Block cost is median/max µs per 64-sample block; the deadline is 1,333 µs. Runs 2026-10-19 03:07 (load 4.6) and 04:06 (load 18):

  | Engine | FP mode | Hash | Subnormal output samples | 0–1 s | 3–10 s | 20–40 s |
  |---|---|---|---|---|---|---|
  | Naive | IEEE (FTZ off) | 529eefe26dc533c6 | 1,099,588 | 2.9–3.4 | 29–35 / ≤ 50 | **66–78 / ≤ 100** |
  | Naive | FTZ only (nice-plug) | **724369776f94487a** | 0 | 3.2–3.3 | 3.2–3.5 | 3.2–3.5 |
  | Naive | FTZ+DAZ (no_denormals) | **724369776f94487a** | 0 | 2.9–3.4 | 3.0–3.5 | 3.1–3.5 |
  | Naive | wasm, node and wasmtime | 529eefe26dc533c6 | — | — | — | — |
  | Flushed | IEEE / FTZ / FTZ+DAZ | **cfb1fb31be29cfd1** (all three) | 0 | 3.0–3.9 (one noisy run: 6.9) | 0.9–1.0 (that run: 2.0) | 0.9–1.1 (that run: 2.1) |
  | Flushed | wasm, node and wasmtime, baseline and simd128 | **cfb1fb31be29cfd1** | — | — | — | — |

  - The naive engine's hash depends on FTZ: plugin, CLI and wasm renders would disagree. The flushed engine's doesn't.
  - Whole-render wall time for the 40 s tail: naive 1,505–1,607 ms vs flushed 41–43 ms (**36×**).
  - The explicit flush costs nothing measurable in the sounding part (0–1 s: within noise). Ending voices saves about 70% of CPU in the tail.
  - At 12 s the naive hash was still FTZ-independent: the reverb was still in normal range and absorbed the subnormal voice output. Only the CPU cost showed (33.8 vs 3.5 µs per block). **Golden tails must be long enough to reach the subnormal range**, or better, assert "no subnormals" directly (Q4).
  - Command: `cargo build --release && tgt/e2/release/tail` (sets MXCSR modes itself); wasm: `node scripts/e2.mjs engine2.wasm` and `wasmtime run --invoke wasm_tail_hash engine2.wasm <mode> <path>`.

### Q3 — Render throughput and SIMD

**Same-machine measurements** (2 s renders at 48 kHz, ×realtime per voice per core):

| Implementation | 1 voice | 16 voices (per voice) | 64 voices (per voice) | Instructions per sample-voice | Notes |
|---|---|---|---|---|---|
| SynthMark `SimpleVoice`, C++ g++ `-O2` (unmodified Google source via the `cc` crate) | 449 (min 546) | 498 | 501 | 275.1 | LFO, 2 osc, biquad with coefficients per 8 frames, 2 ADSRs |
| same, C++ `-Ofast` (upstream Makefile flags) | 515 | 544 | 554 | 210.2 | fast-math: about 10% faster; output differs |
| same, **Rust port, bit-identical to -O2** | 547 | 504 | 519 | 297.2 | Rust 8% more instructions, same speed |
| `engine2` scalar (polyBLEP ×2, 2× TPT LP, rational tanh) | 1,986 | 2,087 (130× total) | 2,126 (33× total) | 71–76 | libm only at control rate |
| `engine2` X4 (SSE2 baseline build) | 953 | 2,800 (175×) | 2,670 (42×) | — | 1.35× / 1.27× vs scalar |
| `engine2` X8 (AVX2, `target-cpu=native`) | 911 | 3,784 (236×) | 3,772 (59×) | 44 (16 voices) | **1.82× / 1.92×** |
| `detdsp` (libm `tanhf` per sample) | 778 (min 953) | 50× total | — | — | per-sample transcendental |
| fundsp 0.23 (2 `saw_hz` → `lowpass_hz` → `Tanh`) | 209 tick / 442 block | 214 / 447 | — | — | Rust ecosystem reference |

- Loads: smbench at load 2.3 (the quietest run of the day); engine2 and detdsp at load 10.6 → 7.8; fundsp at load 3.7. Instruction counts come from callgrind as the difference between 2 and 1 renders.
- Raw files: `smbench-timing.txt`, `timing-batch3.txt`, `fundsp-timing.txt`.
- **Published reference** *(aggregate, anecdotal)*: SynthMark VoiceMark_100 (voices that fill one CPU, capped at `kSynthmarkMaxVoices = 1024`) has a median of 701 over 1,785 public results (i9-12900K 978, i7-1165G7 889, Apple M1 681) <https://mail.openbenchmarking.org/test/pts/synthmark>. This laptop's ~500–550 with my harness fits that distribution.
- **wasm** (engine2, ×realtime total):
  - V8: scalar 1 voice 1,321×, 16 voices 76×; `simd128` X8 at 16 voices 97× (1.17× over the `simd128` scalar build at 83×).
  - wasmtime 48: scalar 1 voice 1,254×, 16 voices 79×; `simd128` X8 at 16 voices **54×, slower than scalar** (83×).
  - So wasm SIMD speed depends on the engine (measured; not investigated further).
  - wasm scalar code runs at roughly 60–70% of native.
- **SIMD on stable Rust** (versions from crates.io, 2026-10-19):

  | Crate | Version (date) | Licence |
  |---|---|---|
  | `wide` | 1.7.1 (2026-09-14) | Zlib/Apache/MIT |
  | `pulp` | 0.22.3 (2026-06-20) | MIT |
  | `fearless_simd` | 1.1.0 (2026-10-06) | Apache/MIT |
  | `macerator` | 0.5.1 (2026-09-28) | — |

  - `std::simd` is still nightly-only (rust-lang/rust#86656 open, updated 2026-08-12).
  - Lane-parallel voices were measured bit-identical to scalar. Speed-ups were modest (1.3–1.9×), because control rate stays scalar and fixed-order summation forces per-lane scalar adds.
  - Speed comes first from avoiding per-sample transcendentals: compare engine2 against detdsp, 2,000 vs 780 per voice.
- **Implication for the solver budget (design.md §7):** feature extraction can cost as much as the render, or more.
  - A 2048-point FFT takes 18.5 µs scalar, 7.3 µs SSE or 3.5 µs AVX; at hop 480 that is about 3.6 ms per resolution per 2 s clip with scalar FFT (bit-exact) or 0.7 ms with AVX.
  - A 1-voice render takes 1.0–3.7 ms.
  - The budget should count render + analysis per candidate.

### Q4 — Benchmarking practice

- **Tools** (crates.io and GitHub, 2026-10-19):

  | Tool | Version (date) | Notes |
  |---|---|---|
  | Criterion | 0.8.2 (2026-02-04) | Repo `criterion-rs` |
  | divan | 0.1.21 (2025-04-10) | No release for 18 months; repo pushed 2026-07-19 |
  | gungraun | 0.20.0 (2026-09-26) | Formerly iai-callgrind; Valgrind or perf; "cannot be run on Windows" |
  | hyperfine | 2.0.0 (2026-10-07) | Metrics `instructions`, `cpu_cycles`, …; JSON schema v2; no shell by default. apt ships 1.18.0, which has no `--metrics` |
  | tango-bench | 0.9.0 (2026-10-16) | Paired benchmarking |
  | CodSpeed `codspeed-divan-compat` | 5.0.3 (2026-10-14) | See below |

  - CodSpeed's CPU simulation: "A benchmark will be run only once and the CPU behavior will be simulated"; it weights instructions, models caches and excludes syscalls. The free plan is "Best for open-source projects": unlimited runs, CPU simulation and wall-time instruments, 600 macro-runner minutes a month (<https://codspeed.io/docs/instruments/cpu>, <https://codspeed.io/pricing>) *(documented, vendor)*.
  - Criterion FAQ: "You probably shouldn't (or, if you do, don't rely on the results)" on cloud CI *(documented)*.
- **Measured, same engine2 render** (new), with outside load from other researchers:

  | Tool | Run 1 | Run 2 (no code change) | Verdict |
  |---|---|---|---|
  | Criterion 0.8.2 (20 samples, `flock -x`) | 1 voice scalar 1.316 ms; 16 voices x8 26.86 ms | 1.259 ms **"Performance has improved" −13%, p = 0.01**; 14.62 ms **"improved" −36.5%, p = 0.00** | False positives |
  | divan 0.1.21 (20 samples) | 16 voices scalar median 20.11 ms (spread 18.9–32.8) | 20.66 ms | Medians ±5%, within-run spread up to 2× |
  | hyperfine 1.18 (`speed 16 0 20`, 10 runs) | 582.6 ± 94.5 ms (465–777) | 669.8 ± 100.1 ms | σ ≈ 16%; means 15% apart |
  | gungraun 0.20.0 (callgrind) | Instructions: 1v scalar 7,335,551; 16v scalar 109,432,247; 16v x8 68,049,636 | **"No change"** on every metric | Exact |
  | raw `valgrind --tool=callgrind`, 3 runs | 9,527,286 / 111,624,424 / 70,241,813 | identical ×3 | Exact |

- **Limitation of instruction counts (new):** the naive 40 s tail costs 36× more wall time than the flushed one but only 2.0× more instructions (861 M vs 434 M). Callgrind doesn't model microcode assists, so an instruction-count gate would miss a denormal regression. A deterministic test catches it: zero subnormal samples in output and state over a long tail.
- **Build cost** (cold `cargo bench --no-run`, 3 jobs, load ≈ 20; unique crates including engine2's 5):

  | Harness | Crates | Cold build | Incremental (touch engine lib) |
  |---|---|---|---|
  | Criterion | 46 | 113 s | 10.5 s |
  | divan | 24 | 50 s | 6.5 s |
  | gungraun | 32 | 37 s | 6.4 s |

  - gungraun needs a matching `gungraun-runner` binary: `cargo install --root …`, 205 s here.
- **Laptop noise control:**
  - The LLVM guide <https://llvm.org/docs/Benchmarking.html> recommends the `performance` governor, `no_turbo`, `cset shield`, taking SMT siblings offline and turning ASLR off, claiming "<0.1%" variation.
  - Here every one of those knobs is root-owned. Current state: intel_pstate active, governor powersave, EPP balance_performance, turbo on, SMT on (cpu0/cpu6), ASLR 2.
  - hyperfine 2.0's `--metrics instructions` fails with "Metric 'instructions' is unavailable … Check … hardware-counter permissions" at `perf_event_paranoid = 4` *(measured)*.
  - Today's biggest noise source was other processes, not frequency scaling.
- **Practice for enth:**
  - CI gate: gungraun (or CodSpeed simulation) instruction counts, plus a deterministic subnormal test.
  - Local: divan or Criterion for wall time, min and median of N on a quiet machine with load recorded.
  - Never trust a single wall-clock "regression" verdict.

### Q5 — Real-time safety tooling

- **`assert_no_alloc`** 1.1.2 (2021-08-03; repo's last commit 2022-09-11): the LICENSE file is a one-clause BSD text, and Cargo says `BSD-1-Clause`. nice-plug uses a fork, `nice-assert-no-alloc` 1.0.0 (2026-05-17), also BSD-1-Clause. Task 2's `deny.toml` allow-list lacks BSD-1-Clause *(documented)*. Measured (new, `engine2/tests/noalloc.rs`):
  - In debug `cargo test`, 1,500 blocks of 16 voices plus the tail inside `assert_no_alloc` pass.
  - The negative control (`Vec::with_capacity` inside) aborts the child process with "memory allocation of 64 bytes failed", so it is detected.
  - `cargo test --release` **does not compile**: `AllocDisabler` doesn't exist under the default `disable_release` feature. Gate such tests with `#[cfg(debug_assertions)]`, or use `default-features = false`.
- **RealtimeSanitizer:**
  - rustc PR #147935 (merged 2025-11-08) added `-Zsanitizer=realtime` with `#[sanitize(realtime = "nonblocking" | "blocking" | "caller")]` under `#![feature(sanitize)]`.
  - rustc's bootstrap lists `rtsan` among the sanitizer runtimes built for x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, macOS and iOS *(documented)*.
  - Measured (new): nightly 1.101.0 (2026-10-18), installed with `--profile minimal` into a scratch `RUSTUP_HOME` (605 MB, 52 s), ships `librustc-nightly_rt.rtsan.a`.

  With `RUSTFLAGS=-Zsanitizer=realtime cargo +nightly build --target x86_64-unknown-linux-gnu`, and no `-Zbuild-std`:

  | Case | Result |
  |---|---|
  | engine2 `process` (16 voices, X8, plus tail), 1,500 blocks | No report |
  | `Vec::with_capacity` inside a nonblocking fn | "ERROR: RealtimeSanitizer: unsafe-library-call / Intercepted call to real-time unsafe function `malloc` in real-time context!" |
  | `std::sync::Mutex::lock` (uncontended) | **Not detected**: Rust's futex-based Mutex makes no intercepted call when uncontended |

  - The stack traces were unsymbolized in this release build.
- **`rtsan-standalone`** 0.3.0 (2025-09-27; LICENSE file is Apache-2.0): Linux, macOS and iOS. By default its build script downloads prebuilt runtime binaries from `realtime-sanitizer/rtsan-libs`; building from source needs cmake ≥ 3.20, which isn't installed. It's unnecessary now that nightly ships the runtime. **Not built** (Gaps).
- **Fit:**
  - `assert_no_alloc` in debug engine tests: allocations only, stable toolchain.
  - A nightly RTSan CI job over the same tests: also catches syscalls, I/O and pthread locks.
  - A review/lint rule against `Mutex`/`RwLock` in `enth-engine`, since neither tool catches an uncontended std lock.

### Q6 — Local wasm32 builds

`rustup target add wasm32-unknown-unknown` took 12.8 s. Release builds, cold, separate target directories, `cdylib` calling into each dependency. Crate counts are unique entries of `cargo tree --target wasm32-unknown-unknown -e normal`, including the root.

| Dependency spec | Result | Crates | Cold build | .wasm |
|---|---|---|---|---|
| `libm = "=0.2.16"` | OK | 2 | 2.4 s | 7.7 KB |
| `biquad = "=0.6.0"` | OK | 4 | 5.9 s | 7.7 KB |
| `rustfft = "=6.4.1"` (default) | OK | 8 | 9.2 s | 268 KB |
| rustfft, `default-features = false, features = ["wasm_simd"]` | OK | 8 | 59.4 s | 1.13 MB |
| **`fundsp = "=0.23.0"`, `default-features = false`** | **FAIL** (`vec!`/`Vec` in `resample.rs`; also fails native) | 34 | — | — |
| **fundsp git master (5595840, 2026-03-03), `default-features = false`** | **FAIL (new)**: `Vec` in `oversample.rs:230`, `realseq.rs:28`; `Box` in `realseq.rs:33–34` | 29 | — | — |
| `fundsp`, `default-features = false, features = ["std"]` | OK | 34 | 46.9 s | 106 KB |
| `fundsp` (default) | OK | 71 | 98.3 s | 113 KB |
| `no_denormals = "=0.3.0"` | **FAIL** (`compile_error!`) | — | — | — |
| `engine2` (libm + wide) | OK | 5 | 13–19 s | — |

(Build times are from the first session, load 13–31, 2 jobs; engine2's is today's, 3 jobs.)

- fundsp issue #86 (no_std build) was closed on 2026-01-12, but master is broken again in other files. No release has followed 0.23.0 (2026-01-07).
- **wasm runners:**
  - node 22 runs `wasm32-unknown-unknown` modules with no imports. Nothing needs installing.
  - wasmtime runs them with `wasmtime run --invoke <fn> <module> <args>`. It warns that `--invoke` with arguments or return values "is experimental and may break in the future". No WASI target is needed.
  - Installing wasmtime-cli: 49.x requires Rust 1.96. On 1.95, `cargo install --locked wasmtime-cli` fails with "cannot install package `wasmtime-cli 49.0.3`, it requires rustc 1.96.0 or newer … `wasmtime-cli 48.0.6` supports rustc 1.95.0". 48.0.6 took 755 s and 2.17 GB peak RSS to build (4 jobs) *(measured)*.
  - Current stable is 1.99.1 (2026-10-13).

### Q7 — Golden testing of audio

- **Precedents** *(documented)*:
  - Faust's impulse-response tests compare each backend (C++, LLVM, Java, wasm and others) against stored references "with a low tolerance".
  - fundsp compares `tick` against `process` with a 1e-4 tolerance.
  - web-audio-api-rs (MIT, pushed 2026-10-12) uses `float_eq`/`almost`, `alloc_counter`, `iai` and Criterion.
  - No Rust audio project found relies on cross-platform hashes. The measurements above show enth can.
- **Recommended layout:**
  1. **Exact hash** (SHA-256 or FNV over the f32 bits) per reference patch × performance × sample rate. It must be equal on every platform and wasm engine (Appendix A).
  2. **Feature-summary snapshot** (`cargo-insta` is installed): RMS per 10 ms and centroid per frame, rounded (e.g. 0.01 dB, 1 Hz), shown as a diff when a hash breaks. Measured cross-platform differences in the std path were ≤ 3.3e-7 dB and ≤ 1.4e-3 Hz, so these roundings only absorb noise far below audibility.
  3. **Invariance tests:** scalar = SIMD lanes; FTZ off = FTZ on; debug = release; `target-cpu=native` = baseline.
  4. **Subnormal test:** render a long release tail (≥ 30 s here) and assert zero subnormal output samples, plus zero subnormal state where it is exposed.
  5. Never hash quantized samples: one rounding-boundary flip changes the hash. Canonicalise NaN, or assert there are none.
- For ported DSP (ADR-0006), a bit-exact golden test against the upstream C++ is feasible (Measured E), as long as the C++ is built without `-ffast-math`/`-Ofast` and the float/double promotions are copied.

## Recommendation for Phase 0

1. **Contract:** the engine render path is bit-exact across x86-64, aarch64 and wasm32 (all engines), and across build flags. Rules for L0–L3:
   - transcendentals only through `libm` pinned with `=` (bumping it is an engine-version change), or our own approximations from `+ − × ÷ sqrt`;
   - in `clippy.toml` `disallowed-methods`, ban `f32/f64::{sin, cos, tan, exp, exp2, ln, log*, powf, powi, tanh, …}`, `mul_add`, `algebraic_*`, `wide::*::mul_add/mul_sub` and fearless_simd `mul_add`;
   - no runtime CPU dispatch with numerically different kernels;
   - fixed voice and summation order;
   - NaN treated as a bug.

   Analysis and scoring: either scalar rustfft (bit-exact; about 5× slower than AVX) or a tolerance with decisions made robust. Coordinate with 04.
2. **Denormals:**
   - Flush recursive state in code (`abs < 1e-15 → 0`, at control rate or per write in feedback paths).
   - End voices below −100 dB.
   - Never set MXCSR/FPCR in engine code.
   - Rely on the measured fact that the flushed engine is FTZ-independent: nice-plug's FTZ then doesn't matter.
3. **Task 5 (spike-engine):**
   - hashes on native, node, wasmtime and CI (Appendix A);
   - lanes = scalar; FTZ off = on; a 40 s-tail subnormal-count test;
   - ×realtime as min and median with load recorded, plus callgrind instruction counts;
   - render + analysis per candidate;
   - `assert_no_alloc` tests gated on `debug_assertions`;
   - one nightly RTSan run.
4. **CI benchmarks:** gungraun instruction counts as the regression gate, plus the subnormal test. CodSpeed simulation is a hosted alternative (free for open source). Wall-time Criterion or divan only locally.
5. **Licences and tools:**
   - add BSD-1-Clause to `deny.toml`;
   - install wasmtime by pinning `--version 48.0.6` or after updating the toolchain; in CI use `bytecodealliance/actions/wasmtime/setup@v1`;
   - hyperfine 2.0 only helps once perf is allowed.
6. **ADR-0006 fallback:** `fundsp = { version = "=0.23.0", default-features = false, features = ["std"] }`.

## Plan check — what in the current docs the evidence says should change

| Doc | Change | Why |
|---|---|---|
| design.md §5 | Replace "Bit-exact on the same build and platform; across platforms and wasm32, equal within a tolerance on a feature summary" with "Bit-exact across supported platforms and wasm engines, and across build flags and host FTZ settings; enforced by golden hashes in CI. A feature summary with tolerances is the diagnostic when a hash breaks." Add the numeric rules (Recommendation 1). Change "Flush denormals" to "flush denormals in code and end voices below −100 dB; never depend on, or set, FTZ/DAZ". Keep the old tolerance contract as an explicit fallback **until aarch64 and Windows CI is green**. | Measured bit-exactness (native, V8, Cranelift, lanes, flags). Runtime dispatch breaks the current wording. The 40 s tail shows FTZ-dependent hashes unless flushing is in code. |
| design.md §6/§9 | Randomizer acceptance and pass/fail must give the same decision on every platform, which bit-exact renders provide. | Duel targets otherwise depend on the machine. |
| design.md §7 | Count render + feature extraction per candidate. | Analysis FFTs (0.7–3.6 ms per resolution per 2 s) rival renders (1–4 ms). |
| phase-0.md Task 1 | `cargo install --locked wasmtime-cli` fails on Rust 1.95. Either update the toolchain to current stable first (1.99.1) or pin `--version 48.0.6`; expect about 12 minutes and 2 GB of RAM. Note that node (already installed) runs wasm32-unknown-unknown modules today. apt's hyperfine is 1.18, without the 2.0 metrics, and perf is blocked anyway. | Measured. |
| phase-0.md Task 2 | Add `BSD-1-Clause` to `deny.toml`. | `assert_no_alloc` and `nice-assert-no-alloc` (LICENSE files read). |
| phase-0.md Task 5 | Compare **hashes** across native, wasm (node and wasmtime) and CI (Linux x64/arm64, macOS arm64/x64, Windows x64/arm64); record the feature-summary maximum only where hashes differ. Allow `wasm32-unknown-unknown` with `wasmtime run --invoke` or node alongside wasip1. Add: lanes = scalar, FTZ off = on, long-tail subnormal count, `assert_no_alloc` under `cfg(debug_assertions)`, one nightly RTSan run, instruction counts next to ×realtime, and render + analysis per candidate. | Measured feasibility; release builds lose `AllocDisabler`; RTSan available on nightly. |
| benchmarks.md, engine table | Determinism row: "hashes identical across platforms, wasm engines and build flags". The "regressions > 10% fail CI" rule becomes **instruction counts** (gungraun or CodSpeed simulation) **plus a deterministic subnormal test**, because Criterion produced false positives and Ir hides denormals. The 100× floor stays as the G0 pass line; record the real numbers (1 voice and 16 voices). Add an RTSan row. | Measured. |
| ADR-0006 | Fallback spec → `default-features = false, features = ["std"]`. A Signalsmith port can have a bit-exact golden test against the C++ if both avoid fast-math. | fundsp 0.23.0 and master fail without std; Rust ↔ C++ `-O2` bit-identical (SynthMark). |
| AGENTS.md | Add the `clippy.toml` `disallowed-methods` list (Recommendation 1). Add "end voices below −100 dB". Add "no Mutex/RwLock in `enth-engine`" (RTSan misses an uncontended std lock). Mention `no_denormals` as unusable in core crates (no wasm32). | Enforces the contract for AI-written code. |

Only Rust and a native UI are fixed. Nothing here touches either.

## Gaps — what is still unverified, and the check that would settle it

1. **aarch64 (Linux, macOS) and Windows (x64, ARM) hashes: not run.** There is no hardware or emulator here (no qemu, no sudo). Settle with the CI job in Appendix A once the repository exists, or by running `engine2`/`detdsp` on those runners against the hashes listed in Q1 and Q2. GitHub documents `ubuntu-24.04-arm`, `windows-11-arm`, `macos-latest` (arm64 M1) and `macos-15-intel` as standard runners, "free and unlimited on public repositories" (docs.github.com, read 2026-10-19).
2. **Browser engines** (SpiderMonkey, JavaScriptCore): not tested. Only V8 and Cranelift. A Playwright job, or a manual run of `scripts/e2.mjs` logic in Firefox and Safari, would settle it.
3. **Wall-clock numbers are contaminated** by unlocked processes (load 2.3–26); perf counters are blocked. Settle by having the owner run `sudo sysctl kernel.perf_event_paranoid=1` and repeat `timing-batch*` with nothing else running. Instruction counts in this report are the load-independent reference.
4. **wasm instruction counts:** valgrind on wasmtime (`--smc-check=all`) didn't finish one run in 10 minutes. Stopped.
5. **The wasmtime `simd128` slowdown** (X8 54× vs scalar 83×) wasn't investigated (Cranelift codegen for `wide`'s f32x8 select/compare). Check with `wasmtime explore`, or try `fearless_simd`/raw `core::arch::wasm32`.
6. **`rtsan-standalone` crate not built:** its default build downloads prebuilt binaries, and building from source needs cmake, which is absent. Not needed if nightly is acceptable for one CI job.
7. **SynthMark's own VoiceMark harness** (scheduling, note on/off) wasn't run. I rendered the same `Synthesizer` through my own C ABI harness. Building `linux/Makefile` would give the canonical number.
8. **The Arm FPCR text** came from a mirror of Arm's XML (developer.arm.com doesn't resolve from this machine).
9. **Toolchains:** hashes were measured on 1.95.0 (and nightly for RTSan only). Re-run on whatever Task 2 pins; no change is expected, since libm is pinned and Rust doesn't change IEEE semantics.
10. **aarch64 FZ semantics in the long tail:** FZ flushes inputs as well as outputs. The flushed engine should be unaffected, and the CI job's FTZ test is x86-only. Add an aarch64 variant that sets FPCR.FZ in a test.

## Appendix A — CI job for cross-platform hashes (draft, YAML-validated, not run)

Saved at `~/code/enth-scratch/2026-10-19-phase-0-spikes/03/ci/golden-hashes.yml`. It assumes a workspace binary `enth-golden`: `hash` prints one `<case> <hash>` line per reference render, and `wasm-hash <runner> <module>` does the same through wasmtime or node. Action versions as of 2026-10-19: `actions/checkout` v7.0.1, `upload-artifact` v7.0.2, `download-artifact` v8.0.2, `bytecodealliance/actions/wasmtime/setup` v1 (tag v1.1.3).

```yaml
name: golden-hashes
on: [push, pull_request]
jobs:
  native:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-latest, macos-15-intel, windows-latest, windows-11-arm]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
      - run: rustup show            # installs the toolchain pinned in rust-toolchain.toml
      - name: Render reference patches and hash them
        shell: bash
        run: cargo run --release --locked -p enth-golden -- hash > hashes-${{ matrix.os }}.txt
      - name: Same build with the CPU's own features (FMA etc.) must not change hashes
        if: runner.os == 'Linux'
        shell: bash
        run: RUSTFLAGS="-C target-cpu=native" cargo run --release --locked -p enth-golden --target-dir target-native -- hash > hashes-${{ matrix.os }}-native.txt
      - uses: actions/upload-artifact@v7
        with:
          name: hashes-${{ matrix.os }}
          path: "hashes-*.txt"
  wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - run: rustup show && rustup target add wasm32-unknown-unknown
      - uses: bytecodealliance/actions/wasmtime/setup@v1
      - name: Build the golden cdylib for wasm32 (baseline and simd128)
        run: |
          cargo build --release --locked -p enth-golden --lib --target wasm32-unknown-unknown
          RUSTFLAGS="-C target-feature=+simd128" cargo build --release --locked -p enth-golden --lib --target wasm32-unknown-unknown --target-dir target-simd
      - name: Hash under wasmtime (Cranelift) and node (V8)
        run: |
          cargo run --release --locked -p enth-golden -- wasm-hash wasmtime target/wasm32-unknown-unknown/release/enth_golden.wasm > hashes-wasm-wasmtime.txt
          cargo run --release --locked -p enth-golden -- wasm-hash node target-simd/wasm32-unknown-unknown/release/enth_golden.wasm > hashes-wasm-node-simd.txt
      - uses: actions/upload-artifact@v7
        with:
          name: hashes-wasm
          path: "hashes-*.txt"
  compare:
    needs: [native, wasm]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: actions/download-artifact@v8
        with:
          path: got
          merge-multiple: true
      - name: Every platform must equal the committed golden file
        shell: bash
        run: |
          status=0
          for f in got/hashes-*.txt; do
            if ! diff -u tests/golden/hashes.txt "$f"; then echo "::error::$f differs from tests/golden/hashes.txt"; status=1; fi
          done
          exit $status
```

A validation note: a first draft that used YAML flow mappings with `${{ }}` inside (`with: { name: hashes-${{ matrix.os }} … }`) is invalid YAML. PyYAML rejected it, and the block form above parses.

## Appendix B — Reproducing the local results

All paths are under `~/code/enth-scratch/2026-10-19-phase-0-spikes/03/`; targets are in `tgt/`. Builds used `CARGO_BUILD_JOBS=3 flock -s /tmp/enth-quiet.lock`; timings used `flock -x`.

| What | Command |
|---|---|
| detdsp hashes, native and wasm | `cd detdsp && cargo build --release --bins --target-dir ../tgt/detdsp && ../tgt/detdsp/release/detdsp-native hash`; `cargo build --release --lib --target wasm32-unknown-unknown --target-dir ../tgt/detdsp`; `node ../scripts/run-wasm.mjs <wasm> <prefix>`; `tools/bin/wasmtime run --invoke wasm_hash <wasm> <variant> <voices>` |
| std-vs-libm census | `tgt/detdsp/release/census`; `node scripts/census.mjs <wasm>` |
| engine2 lanes, tail, speed | `cd engine2 && cargo build --release --bins --target-dir ../tgt/e2`; `tgt/e2/release/lanes [time]`, `tgt/e2/release/tail`, `tgt/e2/release/tailrun <0\|1>`, `tgt/e2/release/speed <voices> <path> <reps>`; FMA variant via `RUSTFLAGS="-C target-cpu=native"` or `"-C target-feature=+fma"` |
| engine2 wasm | `cargo build --release --lib --target wasm32-unknown-unknown` (± `RUSTFLAGS="-C target-feature=+simd128"`); `node scripts/e2.mjs <wasm> [time]`; `wasmtime run --invoke wasm_held_hash\|wasm_tail_hash\|wasm_held_render <wasm> …` |
| Instruction counts | `valgrind --tool=callgrind --callgrind-out-file=/dev/null tgt/e2/release/speed 16 0 1` (and `… 2`); gungraun: `cd bench-gung && GUNGRAUN_RUNNER=../tools/bin/gungraun-runner cargo bench --bench gung` |
| Criterion / divan | `cd bench-crit && cargo bench --bench crit -- --noplot`; `cd bench-div && cargo bench --bench div` |
| SynthMark C++ vs Rust | `cd smbench && SM_SRC=../synthmark-src/synthmark/source SM_CXX_OPT=-O2 cargo build --release --target-dir ../tgt/sm-O2` (and `-Ofast`); `tgt/sm-O2/release/smbench hash <voices> 2.0`; `smbench cpp\|rust <voices> 2.0 15` |
| Feature summaries | `featcmp/target/release/featcmp out-native-v1-16.f32 out-wasm-v1-16.f32` |
| rustfft paths | `cd fftdet && cargo build --release --bins [--no-default-features [--features sse]] --target-dir ../tgt/fft-<v>`; `tgt/fft-<v>/release/fftdet` and `fftspeed` |
| RTSan | `RUSTUP_HOME=rustup CARGO_HOME=cargohome RUSTFLAGS=-Zsanitizer=realtime rustup run nightly cargo build --release --target x86_64-unknown-linux-gnu` in `rtsan-test`; then run with `good`, `alloc` or `lock` |
| assert_no_alloc | `cd engine2 && cargo test --test noalloc` (debug) |
| fundsp | `fundsp-speed`: `cargo build --release`, run with `1` and `16`; `w-fundsp-git`: `cargo build --release --target wasm32-unknown-unknown` |
