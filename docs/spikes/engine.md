# Spike: engine speed and determinism (Phase 0, Task 5)

- Date: 2026-10-19
- Code: `spikes/spike-engine` (package `spike-engine`), branch `lane-b/engine`; CI in `.github/workflows/engine-golden.yml`; `cargo xtask wasm-hash`
- Decision it feeds: [design.md §5](../design.md#5-determinism) (the determinism contract), the engine table in [benchmarks.md](../benchmarks.md), and the G0 speed threshold in [roadmap.md](../roadmap.md)
- Research behind it: `_local/sessions/2026-10-19-phase-0-spikes/03-engine-determinism-final.md` (rules, denormals, benchmarking), risk #7 in `07-plan-risks.md` (seed to target across platforms)
- Verdict: **make cross-platform bit-exact renders the contract.** On this machine 1,010 SHA-256 hashes (10 renders and 1,000 seed to patch) are identical across debug, release, `-C target-cpu=native`, explicit FMA/AVX2, fat LTO, scalar and SIMD-lane code, and `wasm32-wasip1` under wasmtime (with and without `simd128`). Host FTZ/DAZ settings do not change a render. Speed is far above the 100× floor: **1,993× realtime (best of 40) on the 1-voice template**, 152× scalar and 231× with SIMD lanes for the 8-voice P1-like patch with a reverb. **Windows and aarch64 are not run from here**: the CI job is written and linted but has not run on GitHub.

## Acceptance (docs/phase-0.md Task 5)

| Criterion | Result |
| --- | --- |
| ≥ 100× realtime per core, 1-voice template | **Pass: 1,993× best, 1,783× median** (hyperfine mean 1,724×, wasmtime 1,357×) |
| Same-platform hashes identical | **Pass**: twice in a row, debug = release, lanes = scalar, FTZ off = on |
| Cross-platform summary within a tolerance | **Pass for native vs wasm: difference 0** (hashes equal, so every feature difference is 0). Windows and aarch64 are pending CI. Tolerances for the case where maths is *not* pinned are recorded below |
| `assert_no_alloc` around `process()` | **Pass** (debug; scalar, x4, x8; simple, P1-like, 16 random patches), negative control aborts |
| hyperfine and a benchmark | hyperfine 2.0.0 for wall clock; the benchmark is **gungraun (callgrind) instruction counts**, not Criterion, per research 03 (Criterion gave two false "improved" verdicts with no code change and pulls 46 crates) |

## What was built

| Piece | Where |
| --- | --- |
| Engine: up to 3 polyBLEP saws, TPT state-variable low-pass, rational tanh, amp envelope, filter envelope, one global LFO (cutoff and vibrato), per-voice seeded phase and detune, one voice per note, 16 voices max. Control rate 32 samples; the amp ramps per sample | `src/engine.rs` |
| Scalar path and `wide` f32x4 / f32x8 lane paths (one voice per lane) with the same expressions in the same order | `src/engine.rs` (`process_scalar`, `lanes_impl!`) |
| Reverb stand-in: 4-line feedback delay network, Hadamard mix, one-pole damping, RT60 from the patch, seeded delay jitter | `src/fdn.rs` |
| Patch in real units, canonical bytes, SHA-256, `Patch::from_seed` (parameter space only) | `src/patch.rs` |
| Only door to `sin`, `exp`, `exp2`, `tan` (pinned `libm`), plus `flush` and `tanh_approx` | `src/math.rs` |
| Seeded SplitMix64 with `derive(seed, stream)` | `src/prng.rs` |
| Offline renderer (fixed 64-sample blocks, seed passed in), reference performances, golden cases | `src/render.rs` |
| `engine-hash` (golden lines, `lanes`, `dump <case>`) and `engine-speed` (hyperfine and valgrind driver) | `src/bin/` |
| Tests (17), golden files, instruction-count benchmark | `tests/`, `golden/`, `benches/instr.rs` |
| Native vs wasm check | `cargo xtask wasm-hash [--simd128]` (`xtask/src/engine.rs`) |

Templates (`Patch::simple()`, `Patch::p1_like()`):

- **Simple:** 1 saw → resonant low-pass → amp envelope; no LFO, no reverb. Run with 1 and 16 voices.
- **P1-like:** 3 saws (detuned pair plus a sub), filter, 2 envelopes, a 5 Hz LFO to vibrato and cutoff, FDN reverb at 25% mix, RT60 2.5 s. Run with 8 voices.
- Notes start and end on the control grid (32 samples, 0.67 ms).
- The engine is mono; the voice sum is `out[k] += voice 0, 1, 2, …` in that order on every path.

### The bit-exact rules, and how they are enforced

1. Transcendentals only through `libm = "=0.2.16"` (pinned with `=`; a bump is an engine-version change). `src/math.rs` is the only door.
2. No `mul_add` and no `algebraic_*`; no `wide` `mul_add`/`mul_sub`. Rust never contracts `a * b + c`, so plain operators are safe.
3. Fixed summation order: voices in index order; the lane paths add lane 0, 1, 2, … per sample.
4. Denormals flushed in code: envelope and filter state every control block, delay-line and damping writes in the reverb, the output; voices end when the amp envelope falls below −100 dB. Nothing sets or reads MXCSR/FPCR.
5. All randomness is `SplitMix64`, seeded: `derive(seed, voice)` per voice, `derive(seed, MODULE_REVERB)` for the reverb.
6. NaN is a bug: `hash_samples` panics on a non-finite sample.

Rules 1 and 2 are **enforced by clippy**, not by review. `spikes/spike-engine/clippy.toml` lists `disallowed-methods` for `f32::{sin, cos, tan, exp, exp2, ln, log*, powf, powi, tanh, …}`, the `f64` equivalents, `mul_add`, `algebraic_*` and `wide::f32x4/f32x8::{mul_add, mul_sub, mul_neg_add, mul_neg_sub}`. A probe with five violations produced five errors, including the `wide` ones. Only `math.rs` under `--features std-math` is allowed to call them. `disallowed-types` bans `Mutex`, `RwLock`, `HashMap`, `HashSet`.

## Results

Machine: i7-9750H (AVX2, FMA), Linux Mint 22.3, glibc 2.39, rustc 1.99.1, `libm =0.2.16`, `wide =1.7.1`, `sha2 0.11.1`, wasmtime 48.0.6, hyperfine 2.0.0, valgrind 3.22.0, gungraun 0.20.0. perf counters are blocked (`perf_event_paranoid = 4`), governor `powersave`.

### Determinism: 1,010 hashes, one value

Hashes: the 10 golden renders in `golden/render.txt` plus `Patch::from_seed(seed).hash()` for seeds 0–999 in `golden/patches.txt`.

| Build or runtime | Result |
| --- | --- |
| Native debug | identical to golden |
| Native release | identical |
| `-C target-cpu=native` (AVX2 + FMA) | identical, lanes = scalar |
| `-C target-feature=+fma,+avx2` | identical, lanes = scalar |
| Fat LTO, `opt-level = "s"`, 1 codegen unit | identical |
| `wasm32-wasip1` under wasmtime 48.0.6 (Cranelift) | **identical** (`cargo xtask wasm-hash`) |
| The same with `-C target-feature=+simd128` | identical (`wide` uses real `v128` lanes) |
| x4 and x8 lanes vs scalar, native and wasm | identical on all 10 render cases |
| Whole test suite run inside wasmtime (`CARGO_TARGET_WASM32_WASIP1_RUNNER='wasmtime run' cargo test --release --target wasm32-wasip1 -p spike-engine`) | 11 pass in release (the FTZ file is x86-64/aarch64 only; the 2 `assert_no_alloc` tests also pass under wasm in a debug build) |
| Windows x86-64, Linux aarch64 | **not run** (CI: `engine-golden.yml`) |

Render goldens (first 16 hex digits; the file has all 64): simple-1v `356201a105993e95`, simple-16v `7f31fa709fbba15a`, p1-8v-held `cbe04ce996e0274e`, p1-phrase-4s `b4203e8139453f35`, p1-tail-40s `f9d87e2d79c1ec2a`, fast-decay-tail-12s `5b81491e00f4c037`, four random-patch phrases.

Each patch hash is over the patch's canonical bytes (magic, version, then every f32 as its little-endian bits). All 1,000 are distinct. The generator draws only in parameter space (log-scaled values through `libm::exp2f`), never renders, never retries; this is the design.md §9 rule that risk #7 asks for.

#### What changes if maths is not pinned (negative control, `--features std-math`)

The same code with `f32::{sin, exp, exp2, tan}` instead of `libm`:

| Comparison | Render hashes differing | Patch hashes differing |
| --- | --- | --- |
| std maths, native (glibc) vs libm golden | 10 of 10 | **13 of 1,000** |
| std maths, `wasm32-wasip1` (wasi-libc) vs libm golden | 10 of 10 | 13 of 1,000 |
| std maths native vs std maths wasm | 10 of 10 | 0 of 1,000 |

So with `std` the same seed gives a different *patch* on 1.3% of seeds, and a different render always. The `exp2f` of glibc and wasi-libc happen to agree on all 1,000 draws; their `sinf`/`expf`/`tanf` do not.

Feature summary (RMS per 10 ms and spectral centroid per 480-sample hop, Hann 2048; `~/code/enth-scratch/lane-b/features.py`) on four cases:

| Pair | max \|Δ sample\| | max \|Δ RMS\| | max \|Δ centroid\| |
| --- | --- | --- | --- |
| libm native vs libm wasm | 0 | 0 | 0 |
| libm vs std maths, native | 1.7e-8 … 8.2e-8 | ≤ 2.5e-6 dB | ≤ 7.8e-4 Hz |
| std maths native vs wasm | 1.7e-8 … 8.2e-8 | ≤ 1.8e-6 dB | ≤ 7.7e-4 Hz |

The differences are inaudible, which is the point of research 03: the danger is *decisions* (accept/reject, thresholds), not the waveform.

### FTZ on vs off

`tests/ftz.rs` sets MXCSR (x86-64; FPCR.FZ on aarch64, untested) on the test thread, renders, restores. A first test checks that `1e-30 * 1e-10` is subnormal in IEEE mode and 0 under FTZ, so the rest cannot pass vacuously.

| Case (flushing on) | IEEE | FTZ only | FTZ+DAZ |
| --- | --- | --- | --- |
| simple 16 voices, x8 lanes, 2 s | `7f31fa70…` | same | same |
| P1-like 8 voices held, 2 s | `cbe04ce9…` | same | same |
| P1-like phrase, 4 s | `b4203e81…` | same | same |
| P1-like 40 s tail | `f9d87e2d…` | same | same |
| fast-decay 12 s tail, x4 lanes | `5b81491e…` | same | same |

Negative control (flushing off, 12 s fast-decay tail): IEEE `67356a83…`, FTZ `20f357f9…`. The unflushed engine does change, so the test can fail.

### Subnormals

`tests/subnormals.rs` renders a 40 s release tail (4 notes held 1 s), counts subnormal output samples every block and subnormal values in the whole engine state (voices, envelopes, LFO, reverb) every 16 blocks.

| Engine | Render | Subnormal output samples | Subnormal state values (max) | Voices alive at end |
| --- | --- | --- | --- | --- |
| Flushing on | P1 tail 40 s, scalar | 0 | 0 | 0 |
| Flushing on | P1 tail 40 s, x8 | 0 | 0 | 0 |
| Flushing on | fast-decay tail 40 s | 0 | 0 | 0 |
| Flushing on | simple template tail 40 s, x4 | 0 | 0 | 0 |
| Flushing on | 12 random patches, 30 s tail | 0 | 0 | – |
| **Flushing off** | fast-decay tail 12 s | **203,962** | **5,761** | 4 (never end) |

Wall time of the same 40 s tail (`engine-speed tail|tail-naive`, `flock -x`, load 10.8 → 10.2):

| | Wall (best of 5) | ×realtime | Instructions per render (callgrind) |
| --- | --- | --- | --- |
| Flushing on | 38.4 ms | 1,041 | 306.3 M |
| Flushing off | 1,287.8 ms | 31 | 1,251.4 M |
| Ratio | **33.5×** | | **4.1×** |

An instruction-count gate sees a 4× change where the clock sees 33×, and part of the 4× is voices that never end. Hence both gates: the instruction budget **and** the subnormal test.

### Allocation and real-time safety

- `tests/noalloc.rs` wraps every `process()` block in `assert_no_alloc` (scalar, x4 and x8 over simple 16 voices, P1-like held and the phrase; 16 random patches on x8). Debug builds only: the crate has no `AllocDisabler` in release, so the file is `#![cfg(debug_assertions)]`. A child-process negative control (a `Vec::with_capacity` inside the guard) is detected.
- Extra, not in CI: RTSan on the local nightly (rustc 1.101.0-nightly 2026-10-17, which ships the runtime), `RUSTFLAGS=-Zsanitizer=realtime`, `process()` inside a `#[sanitize(realtime = "nonblocking")]` function, scratch crate `~/code/enth-scratch/lane-b/rtsan-test`: scalar/x4/x8 over simple 16 voices, P1-like 8 voices, a phrase and 8 random patches: **no report**; an allocation inside is reported (`malloc` in real-time context, exit 43); an uncontended `std::sync::Mutex` is **not** reported (same as research 03).

### Speed

Setup: 2 s renders at 48 kHz, block 64, `--release`, one core. Timings were taken under `flock -x /tmp/enth-quiet.lock`, but other agents' processes were running outside the lock: `uptime` load average 10.0 → 5.9 (engine-speed batch, 07:24), 10.9 → 7.0 (hyperfine batch, 07:30). **Read wall-clock numbers as ±20%**; instruction counts are exact.

| Case | ×RT best | ×RT median | hyperfine mean | Instructions per render | Instr. per sample·voice |
| --- | --- | --- | --- | --- | --- |
| simple, 1 voice, scalar | **1,993** | 1,783 | 1,724 (100 renders per run) | 8,151,396 | 84.9 |
| simple, 1 voice, x4 / x8 | 1,159 / 713 | 1,117 / 676 | – | – | – |
| simple, 16 voices, scalar | 129 | 122 | 115 | 122,410,142 | 79.7 |
| simple, 16 voices, x4 | 233 | 216 | – | 65,116,191 | 42.4 |
| simple, 16 voices, x8 | 245 | 213 | 203 | 62,921,803 | 41.0 |
| P1-like, 8 voices, scalar | 152 | 149 | 137 | 112,373,737 | 146.3 |
| P1-like, 8 voices, x4 | 234 | 230 | – | – | – |
| P1-like, 8 voices, x8 | 231 | 217 | 210 | 62,326,059 | 81.2 |
| wasmtime 48.0.6, scalar, simple 1 voice | – | – | 1,357 | – | – |
| wasmtime, simple 16 voices | – | – | 98 | – | – |
| wasmtime, P1-like 8 voices | – | – | 122 | – | – |

- "Best" and "median" are over 40 renders inside one process (`engine-speed <template> <path> 40 stats`; includes `Engine::new`). hyperfine runs (`-N`, 3 warmups, 20 runs) include process start: 3.8 ms native, 10.6 ms under wasmtime.
- Cost is linear in voices: 16 voices at 129× total is 2,064 voice-equivalents of realtime, close to the single voice's 1,993×.
- The P1-like patch costs 1.8× the instructions per voice-sample of the simple one (3 oscillators, two envelopes, LFO and the FDN).
- Lanes pay off from about 4–8 voices (1.8–1.95× fewer instructions at 16 voices, 1.5× wall for the P1-like patch) and lose at 1 voice (empty lanes, 1,993 → 713). x4 and x8 are within 5% of each other at 16 voices on the baseline x86-64 build (x8 is two SSE2 f32x4 there; research 03 measured 1.8–1.9× over scalar for x8 in an AVX2 build, which I did not repeat).
- One 2 s P1-like candidate costs about 9–13 ms of one core. Research 03 puts the analysis FFTs at 0.7–3.6 ms per resolution per clip, so render and analysis are comparable and design.md §7 must budget both.
- wasm under wasmtime runs at 80–90% of native speed on scalar code for these templates (hyperfine means).

### Regression gate: instruction budgets

`benches/instr.rs` (gungraun 0.20.0 over callgrind) runs six cases with an **absolute** `hard_limits` Ir budget each (measured + 10%, matching benchmarks.md's 10% rule). It needs no stored baseline, so it works in CI. Verified: with a 1,000,000 budget the 1-voice case fails the run (exit 3, "exceeds limit of 1000000"); two consecutive runs print "No change" on every metric, so the counts repeat exactly. Budgets are tied to x86-64 Linux and the pinned toolchain, `libm` and `wide`; re-measure them together with the golden hashes.

Run: `GUNGRAUN_RUNNER=/path/to/gungraun-runner cargo bench -p spike-engine --bench instr` (the runner must be the same version, 0.20.0; `cargo install --locked --version 0.20.0 gungraun-runner`).

## How to reproduce

```
cargo test --workspace --locked                      # 17 spike-engine tests, debug (includes assert_no_alloc)
cargo xtask wasm-hash [--simd128]                    # needs wasmtime on PATH or $WASMTIME
CARGO_TARGET_WASM32_WASIP1_RUNNER='wasmtime run' cargo test --release --target wasm32-wasip1 -p spike-engine
cargo run --release -p spike-engine --bin engine-hash -- all        # the 1,010 lines; also: render | patches | lanes | dump <case>
cargo run --release -p spike-engine --bin engine-speed -- p1 x8 40 stats
cargo build --release -p spike-engine --features std-math --bin engine-hash   # negative control
RUSTFLAGS="-C target-cpu=native" cargo test --release -p spike-engine --test determinism
```

Regenerate goldens only on a deliberate engine change (bump `ENGINE_VERSION` or `PATCH_VERSION`, and say why in the commit):
`engine-hash -- render > spikes/spike-engine/golden/render.txt` and `-- patches > …/patches.txt`.

## Other findings

- **Block size is part of the render function.** Blocks of 32 or 64 samples give the same hash; blocks of 48 or 50 do not, because the control grid and the LFO restart at each block. A host-driven engine must keep an internal 32-sample counter and re-chunk the host's buffers (`block_size_is_part_of_the_render_function` documents it).
- `wasmtime run x.wasm all` passes `all` to the program; `wasmtime run x.wasm -- all` passes `--` too.
- `gungraun` does not compile for wasm and `cargo test --target wasm32-wasip1` builds dev-dependencies, so it is a `cfg(not(target_family = "wasm"))` dev-dependency.
- `spikes/spike-engine/clippy.toml` *replaces* the root `clippy.toml` for this crate (clippy uses the nearest file). When `enth-dsp`/`enth-engine` exist, the numeric bans belong in the root file next to the I/O bans.
- Windows checkouts may convert line endings; the golden files carry `.gitattributes` (`eol=lf`) and every comparison goes through `str::lines()`.
- The hash includes the sample rate, channel count and frame count, so the same samples at another rate do not collide.

## What is not done

- **Windows x86-64, aarch64, macOS: no hashes.** Nothing here can run them. `engine-golden.yml` runs Linux x86-64 and `windows-latest` on every relevant push and PR. An `aarch64` job (`ubuntu-24.04-arm`) is behind `workflow_dispatch` only: free arm64 hosted runners are for public repositories, and this one is private. The aarch64 FPCR.FZ branch of `tests/ftz.rs` has never been compiled or run.
- The CI workflow passed actionlint 1.7.12 with shellcheck 0.11.0 but has not run on GitHub. Least certain: `taiki-e/install-action` offering `wasmtime@48.0.6`, and `cargo install gungraun-runner` (205 s cold build in research 03; cached afterwards).
- Only wasmtime was run for wasm (research 03 ran V8 with an earlier engine); no browser engines.
- One CPU (Coffee Lake). FMA and AVX2 builds were measured; AVX-512 and arm64 NEON were not.
- Engine limits: mono, saw oscillators only, no voice stealing, notes quantised to 32 samples. The reverb is a stand-in, not the Signalsmith port of ADR-0006.
- RTSan is a manual run in scratch, not a CI job (it needs a nightly toolchain).
- No std-vs-libm *speed* comparison; the cost of libm per call is in research 03 (1–2.4× glibc, negligible at control rate).

## Recommended changes to design.md §5

Current text → proposal. (The coordinator updates shared docs; these are my recommendations.)

1. **Contract.** Replace "Bit-exact on the same build and platform; across platforms and wasm32, equal within a tolerance on a feature summary" with: *render output is bit-exact across supported platforms and wasm engines, across build flags (debug, release, `target-cpu=native`, FMA, LTO) and across host FTZ/DAZ settings, enforced by golden SHA-256 hashes in CI; a feature summary with tolerances is only the diagnostic when a hash breaks.* Keep the old wording as the fallback until Windows and aarch64 CI are green.
2. **Numeric rules** (add as a list): transcendentals only through `libm` pinned with `=` or our own `+ − × ÷ sqrt` approximations; no `mul_add`, `algebraic_*` or SIMD-crate fused ops; no runtime-dispatched numeric kernels; fixed voice and summation order; NaN is a bug. State that they are enforced by `clippy.toml` `disallowed-methods` (verified here, including `wide`).
3. **Denormals.** Replace "Flush denormals" with: *flush in code (state below 1e-15 → 0 at control rate and on every feedback write), end voices below −100 dB, never set or depend on FTZ/DAZ; a test renders a 40 s tail and asserts zero subnormals in output and state, and a second test asserts the render hash is the same with FTZ and DAZ on.*
4. **Seeds.** Spell out the derivation: per voice `derive(seed, voice)`, per effect `derive(seed, module_id)`, an integer PRNG whose only float step is an exact int-to-f32 conversion.
5. **Block size.** Say that block size and the control-rate grid are inputs of `render()`; hosts with other buffer sizes re-chunk onto the grid, and a note's start and end quantise to it.
6. **Golden tests.** One exact hash per reference render, the same file on every platform, plus the 1,000-seed patch-hash file; hashes over raw f32 bits, never quantised samples; the engine version is in the golden header.
7. **§9 randomizer.** Add the measurement: with `std` maths 13 of 1,000 seeds produce a different patch on glibc than with `libm` (renders always differ), so target generation must be parameter-space only and use pinned maths.

Smaller edits for other docs: benchmarks.md engine table: determinism row "hashes identical across platforms, wasm engines and build flags"; replace "regressions > 10% fail CI" with Ir `hard_limits` budgets plus the subnormal test; add an RTSan row. For G0, 100× is met by a wide margin (1-voice template 1,993×; 8-voice P1-like patch with reverb 152× scalar), so a tighter number such as 1,000× for the 1-voice template and 100× for the 8-voice P1-like patch on one core without lanes is defensible.
