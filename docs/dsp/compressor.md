# Compressor (`enth-dsp::compressor`)

- Lane: P1a FX2, phase 1 (the DSP core). The `Module` wrapper and the golden render case come in phase 2, after freeze F2. The catalogue entry is done (`crates/enth-patch/src/catalogue/comp.rs`, frozen at F1, listed in [catalogue.md](catalogue.md)).
- Code: `crates/enth-dsp/src/compressor.rs`. Tests: the same file (white-box), `crates/enth-dsp/tests/compressor.rs` and `compressor_props.rs`.
- Decisions behind it: [ADR-0006](../adr/0006-effects-strategy.md) (in-house, zero latency, and the 2026-10-19 note that corrects its wording), research report 02 Q6 (`_local/sessions/2026-10-19-p1a-headless-core/02-dsp-modules.md`).

A stereo feed-forward compressor with a soft knee, make-up gain and a dry/wet mix. It is our own code: the structure is Julius O. Smith III's `compression_gain_mono`, the knee comes from Giannoulis, Massberg and Reiss (2012), and the stereo link, the make-up gain and the mix are ours. No look-ahead, so it reports **zero latency**. Nothing in it allocates, reads a clock or draws a random number, and every transcendental goes through the pinned `libm`.

## Signal flow

Per frame, with `fs` the sample rate, `T` the threshold, `R` the ratio, `W` the knee width, `M` the make-up (dB), `μ` = mix / 100, and times `τ` in seconds:

```text
e[n]   = max(|L[n]|, |R[n]|)                                    stereo link: the louder channel drives both gains
k_a    = 1 − exp(−1/(τ_attack·fs))      k_r = 1 − exp(−1/(τ_release·fs))      k_s = 1 − exp(−1/(τ_attack/2·fs))
env[n] = env[n−1] + k·(e[n] − env[n−1])   with k = k_a if e[n] > env[n−1], else k_r        amplitude follower
lvl[n] = 20·log10(max(env[n], 1e-10))                           dBFS; the floor keeps log(0) out of the audio path
d[n]   = lvl[n] − T
G[n]   = 0                          if 2d ≤ −W                  static curve, dB, always ≤ 0
       = s·(d + W/2)² / (2W)        if |2d| < W                 with s = 1/R − 1
       = s·d                        if 2d ≥ W
Gs[n]  = Gs[n−1] + k_s·(G[n] − Gs[n−1])                         one-pole gain smoother in dB
g[n]   = 10^((Gs[n] + M)/20)                                    wet gain with make-up
m[n]   = (1 − μ) + μ·g[n]
out_L  = m·L,  out_R = m·R
```

The follower is JOS's `amp_follower_ar` written as an increment: `env = p·env + (1 − p)·e` with `p = 1 − k = exp(−1/(τ·fs))` is the same recursion. The increment form has an exact fixed point at `env = e`, and `k` is computed in `f64` with `expm1`, so a 10 s release at 192 kHz (`k` ≈ 5e-7) keeps its precision instead of being quantised by `1 − p` in `f32`. `τ = 0` gives `k = 1`: an instant response.

Implementation details that matter for bit-exactness:

- `20·log10(x)` is `6.0206·log2f(x)` and `10^(x/20)` is `exp2f(x·0.1660964)`, both from `libm`. The coefficients use `libm::expm1` in `f64`. No `mul_add`, no `std` transcendentals.
- The static curve is evaluated in `f64` and rounded once to `f32` (see the test section for why).
- `env` and `Gs` below 1e-15 (−300 dBFS) are set to zero in code. No FTZ/DAZ flags.
- A NaN input is zeroed in the detector and an infinite one is clamped to +120 dBFS, so the state can never become NaN. The frame's output is `input × m`, so a non-finite input still gives a non-finite output for that frame, and the next finite frame is fine.
- At mix 100 % the blend is `0 + 1·g`, at mix 0 % it is `1 + 0·g`, so a dry setting returns the input bit for bit. Below the threshold (and the knee) with no make-up the gain is exactly 1 once the smoother has settled (its state is flushed to exactly 0 at 1e-15), so a quiet signal also passes bit for bit.

## Parameters

Real units, `CompressorParams`. `set_params` clamps every field into the core range below and replaces a NaN or infinity by the default. The range the patch catalogue offers is `enth-patch`'s business (lane S1a); the fourth and fifth columns are what it declared at F1 (the ranges are report 02's proposal).

| Field | Unit | Core accepts | Catalogue range / default | Catalogue smoothing | Notes |
| --- | --- | --- | --- | --- | --- |
| `threshold_db` | dBFS | −120 … 24 | −60 … 0, −18 | 10 ms | The knee is centred on it |
| `ratio` | :1 | 1 … 1000 | 1 … 20, 4 | 10 ms | 1 never reduces; 1000 is a near-limiter |
| `knee_db` | dB | 0 … 48 | 0 … 24, 6 | 10 ms | 0 is JOS's hard knee |
| `attack_ms` | ms | 0 … 2000 | 0.1 … 100 (log), 10 | none | Time constant of the follower; also sets the gain smoother (attack/2) |
| `release_ms` | ms | 0 … 10000 | 10 … 2000 (log), 200 | none | Time constant of the follower |
| `makeup_db` | dB | −48 … 48 | 0 … 24, 0 | 10 ms | Wet path only |
| `mix_pct` | % | 0 … 100 | 0 … 100 (`mix`), 100 | 10 ms | 100 = fully compressed; between = parallel compression. The catalogue's `mix` is this field as it is, in percent |

## Attack and release: what the numbers mean

`attack_ms` and `release_ms` are the **time constants of the amplitude follower**: after a step of the input amplitude, the follower covers 63.2 % of the way after exactly that long (the in-file tests check it to ±1 sample, or 0.01 % for the longest, at 44.1, 48 and 96 kHz). The *gain reduction* that the listener hears is slower, for three reasons that follow from JOS's structure (two cascaded one-poles around a nonlinearity), so the module documents the measured behaviour rather than pretending the times are the 63 % times of the gain:

1. **The gain smoother runs at attack/2** (JOS's `kneesmooth`) for rises and falls alike. It adds about 0.25 attack times to an attack and `attack/2` to a release.
2. **The follower works on amplitude, the curve on dB.** After the signal stops, the follower falls 8.686 dB per release time, a straight line in dB. The reduction is `(1 − 1/R)` times the level above the threshold, so it recovers linearly in dB too, at `(1 − 1/R)·8.686` dB per release time, and the time to recover depends on how far over the threshold the signal was.
3. **A rise starts from the silence floor.** The follower climbs `A·(1 − e^(−t/τ))`, which is logarithmic in dB; most of the reduction arrives early and the tail is slow.

Measured with a DC step (both channels), `T = −24 dB`, 4:1, hard knee, at 48 kHz. The step is 0.5 (−6.02 dBFS, 17.98 dB over the threshold), the final reduction is 13.48 dB, and "t63" is when the reduction has gone 63.2 % of the way (attack) or has lost 63.2 % of its value (release):

| attack ms | release ms | attack t63 | t63 / attack | release t63 | rule of thumb |
| --- | --- | --- | --- | --- | --- |
| 1 | 100 | 1.25 ms | 1.250 | 131.4 ms | 131.3 ms |
| 5 | 100 | 6.27 ms | 1.254 | 133.4 ms | 133.3 ms |
| 10 | 200 | 12.52 ms | 1.252 | 266.7 ms | 266.7 ms |
| 30 | 200 | 37.58 ms | 1.253 | 276.7 ms | 276.7 ms |
| 100 | 1000 | 125.27 ms | 1.253 | 1358.4 ms | 1358.4 ms |
| 100 | 10 | | | 60.7 ms | 63.1 ms |

The rules of thumb, for a hard knee and a step from or to silence:

```text
attack t63  ≈ 1.25 · attack_ms        for a step 18 dB over the threshold
release t63 ≈ 0.632 · d / 8.686 · release_ms + attack_ms / 2        d = dB over the threshold when the signal stops
```

The attack factor depends on the step height (attack 10 ms at 48 kHz, same curve: step 0.1 = −20 dBFS gives 24.5 ms, 0.25 gives 15.4 ms, 0.5 gives 12.5 ms, 1.0 gives 10.8 ms). The release rule is within 0.8 % whenever the release is at least as long as the attack, and within about 5 % in the fast-release corners tried (attack 50 ms with release 10 ms measures 36.1 ms against 38.1 ms; attack 100 ms with release 10 ms measures 60.7 ms against 63.1 ms): there the recovery cannot be faster than the `attack/2` smoother, so **a release shorter than half the attack is not reachable**. The times do not depend on the sample rate (tested at 44.1, 96 and 192 kHz).

A steady 1 kHz sine at −6.02 dBFS (static reduction at its peak 13.48 dB) settles at a mean reduction of 12.91 dB with under 0.05 dB of ripple, 10 ms / 200 ms: the follower sits just below the peaks. Report 02 measured 12.93 dB for a sine at exactly −6 dBFS; the 0.02 dB is the 0.02 dB of level.

**If listening finds this too sluggish**, the alternative is the branching detector that Giannoulis et al. recommend (the gain computer first, then a log-domain attack/release smoother on the reduction), which is what web-audio-api-rs does. It is a different module version, so a change needs a new golden, not a tweak. Nobody has listened to this compressor yet (the research and this lane may not play audio): every quality statement above is a metric.

## Provenance and licences

| Part | Source | Licence | In enth |
| --- | --- | --- | --- |
| Follower, static-curve slope, one-pole gain smoother at attack/2, dB conversion | Julius O. Smith III, Faust `compressors.lib`: `compression_gain_mono` (also `compressor_mono`, `compressor_lad_mono`), faustlibraries commit `2edec0537f536eaa130d42c354884d7db971663c` (2026-10-15). Book page: <https://ccrma.stanford.edu/~jos/filters/Nonlinear_Filter_Example_Dynamic.html> (qualitative only) | The file's header is LGPL-2.1+ with the Faust exception for compiled code, but these four functions each declare `LicenseRef-STK-4.3` ("Copyright (C) 2014-2020 by Julius O. Smith III"); the same file marks most other compressors GPL-3.0-only (see [ADR-0004](../adr/0004-licensing.md)), so only these functions are a source. STK-4.3 is MIT-style; full text in `THIRD_PARTY_NOTICES.md` | Re-implemented by hand in Rust from the equations; not Faust compiler output. Entry in `THIRD_PARTY_NOTICES.md` |
| `an.amp_follower_ar` and `si.onePoleSwitching` (the follower) | Faust `analyzers.lib` / `signals.lib`; `amp_follower_ar` is attributed to Jonatan Liljedahl, revised by Romain Michon | No licence declared | Not copied. The attack/release one-pole peak follower is a textbook recursion; ours is written from the equation |
| Quadratic soft knee | D. Giannoulis, M. Massberg, J. D. Reiss, "Digital Dynamic Range Compressor Design: A Tutorial and Analysis", JAES 60(6), 399–408, 2012, eq. 4 (<https://aes2.org/publications/elibrary-page/?id=16354>; author copy <https://www.eecs.qmul.ac.uk/~josh/documents/2012/GiannoulisMassbergReiss-dynamicrangecompression-JAES2012.pdf>) | A paper: cited, nothing copied | Ours. Reduces to JOS's `max(d, 0)·s` at `W = 0` |
| Stereo link by `max(|L|, |R|)`, make-up gain, dry/wet mix, f64 curve, NaN and range guards | Ours | MIT, like the project | JOS's own stereo version (`compressor_stereo`) links by `|L| + |R|`, which reads a centred mono signal 6 dB hot; the Chromium and web-audio-api-rs link is `max`, which we share |
| Test oracle for the gain computer | web-audio-api-rs `src/node/dynamics_compressor.rs` at commit `03d0cb28983896be5cde38d3961a47bdefb8ea8f` (2026-10-07), lines 356–426 | MIT, "Copyright (c) 2021 Otto Rottier" (repository `LICENSE`) | Only its gain-computer arithmetic is reproduced, in `f64`, inside `tests/compressor.rs`, with the notice in the comment. No detector, no look-ahead, no make-up, nothing shipped. Written by contributor b-ma on 2022-07-27/28 from the Web Audio spec's compression curve and Giannoulis eq. 4; 29 commits since. Per the lane brief no `THIRD_PARTY_NOTICES.md` entry, because no code ships; if that is judged wrong the entry is the MIT text with that copyright line |

What differs from JOS:

- **No `max(·, 0)` hard knee unless `W = 0`.** JOS's "knee" is only the smoothing of the gain; the quadratic knee is Giannoulis's.
- **Make-up gain** and **mix** do not exist in JOS's function.
- **Stereo link** by `max`, not by the sum.
- **Ratio** is clamped to ≥ 1: JOS's `1/max(ε, ratio) − 1` would also expand below 1.
- **Implementation:** an increment-form follower, `f64` coefficients and curve, level floor, denormal flush, NaN guard.

What differs from web-audio-api-rs (not copied on purpose):

- A 6 ms look-ahead ring (ours: none, zero latency; the ring also resizes on the first `process`, an allocation).
- Its threshold shifted by `+knee/2` (the Web Audio spec's convention, which its own comment calls "probably wrong"); the oracle test maps it: its threshold parameter is ours minus `W/2`.
- Auto make-up `(1/fullRangeGain)^0.6`, and `std` `powf`/`log10`/`exp`, which differ between platforms.
- The log-domain branching smoother after the gain computer.

## Cost

Measured with callgrind on the release profile (`-O3`, no LTO, x86-64, inclusive cost of `Compressor::process` for 480,000 frames of stereo, one set_params at the start):

| Case | Instructions per frame |
| --- | --- |
| Compressing a saw or noise | 187 |
| Silence | 181 |

About half of it is `libm::log2f` (55) and `libm::exp2f` (39). The lane budget is 250 per frame, report 02's prototype measured 160 with an `f32` curve; the `f64` curve and the NaN guard add the difference. `set_params` costs 92 instructions when nothing changed (the sanitising and a comparison), about 130 when the threshold, ratio or knee change, about 120 for make-up and mix, and about 260 when the attack changes (two `expm1` in `f64`), so a wrapper may call it every 32-frame segment: at most 8 instructions per frame.

## Tests

| Test | What it guarantees |
| --- | --- |
| `static_curve_table` | Hand-worked values for several (T, R, W): hard knee 0 / −4.5 / −13.5 / −18 dB at −24 / −18 / −6 / 0 for T −24, R 4; knee centre −0.5625 dB (W 6) and −1.125 dB (W 12); ratio 1 never reduces |
| `static_curve_matches_the_closed_form_giannoulis_equations` | 6.1 million (T, R, W, level) points against eq. 4 in `f64` |
| `static_curve_matches_web_audio_api_rs_within_1e6_db` | 4.1 million points against web-audio-api-rs's gain computer: worst 9.5e-7 dB wherever the reduction is under 32 dB, 1.9e-6 dB up to 64 dB (one `f32` ulp of the result alone is 3.8e-6 there, so half an ulp is the most that can be promised) |
| `hard_knee_is_jos_max_curve`, `static_curve_is_non_positive_monotone_continuous_and_bounded_in_slope` | `W = 0` is JOS's curve; the curve is never a boost, monotone, continuous, no steeper than `1 − 1/R` |
| `step_response_attack_63_percent_times`, `step_response_release_63_percent_times`, `step_responses_do_not_depend_on_the_sample_rate` | The table above as golden sample counts (±1) and the two rules of thumb; follower attack and release hit their time constants (in-file tests) |
| `the_step_response_matches_the_f64_reference_trajectory`, `the_f32_core_follows_an_f64_reference_of_the_same_equations`, `a_steady_sine_settles_just_under_the_static_reduction_at_its_peak` | The `f32` core stays within 1e-3 dB of an `f64` evaluation of the equations above, for steps, bursts of noise and a steady sine |
| `silence_gives_exact_zeros_and_no_nan`, `silence_after_a_loud_passage_decays_without_nan_or_subnormals`, `non_finite_input_does_not_poison_the_state` | Silence in is silence out for any setting; a 20 s tail has no NaN and no subnormal and the meter returns to exactly 0; NaN and infinity do not poison the state |
| `below_threshold_it_is_transparent_bit_for_bit`, `mix_zero_is_the_dry_input_and_mix_blends_linearly`, `makeup_gain_is_added_after_the_curve_to_the_wet_path_only`, `ratio_one_never_reduces` | Transparency and the mix and make-up algebra |
| `the_louder_channel_drives_both_gains` | The reduction trace is bit-identical for `(x, x)`, `(x, 0)`, `(0, x)`, `(x, x/2)`; one gain on both channels |
| `zero_latency_and_block_size_does_not_matter`, `compressor_props::the_block_size_is_not_an_input` | An impulse leaves on the frame it enters; 1, 7, 32, 63, 64, 65, 512 and 4096-frame chunks, in place or not, give the same bits |
| `compressor_props::output_is_finite_and_bounded_by_the_makeup_gain` | For arbitrary settings (including NaN, infinity and nonsense) and signals: finite output, `|out| ≤ |in|·max(1, make-up)`, meter ≥ 0 |
| `render_hash_is_the_same_on_every_platform` | One FNV-1a hash of 8 segments (sines, silence, noise, bursts, a 60 dB ramp, steps, denormal-range input, a full-scale square) with the settings changed every block and the meters hashed too: `c1e394837f03ee91` |
| `process_allocates_nothing` | `process`, `process_in_place`, `tick`, `set_params`, `reset` and `prepare` under `assert_no_alloc` (debug builds). Negative control run by hand: an allocation inside the guard aborts the test binary |

Mutation check (cargo-mutants 27.0.0, `-p enth-dsp -f crates/enth-dsp/src/compressor.rs`, 2026-10-19): 135 mutants, 133 caught. The two survivors are equivalent: `<` to `<=` at the exact edge of the knee in `StaticCurve::gain_db` (both branches give the same value there, by continuity) and `>` to `>=` where `e == env` in `tick` (the follower's increment is zero either way).

The hash holds on every build tried: native debug, native release, native release with `-C target-cpu=native` (this machine has FMA), `wasm32-wasip1` release and debug under wasmtime 48.0.6, and `wasm32-wasip1` with `-C target-feature=+simd128`. To reproduce:

```sh
cargo test -p enth-dsp --locked --test compressor -- --nocapture render_hash        # prints compressor-hash <hex>
RUSTFLAGS="-C target-cpu=native" cargo test -p enth-dsp --locked --release --test compressor -- --nocapture render_hash
CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --" \
  cargo test -p enth-dsp --locked --target wasm32-wasip1 --test compressor -- --nocapture   # the whole file
```

The static-curve comparison is the one place where the choice of `f64` matters. The same formula in `f32` is within 9.5e-7 dB of the oracle only by luck of the grid (a reduction of 16–32 dB has an `f32` spacing of 1.9e-6, so any two roundings can add up to more than 1e-6); evaluating once in `f64` and rounding the result makes the bound structural: half an `f32` ulp of the answer.

## Notes for the module wrapper (phase 2)

- **API:** `Compressor::new()`, `prepare(sample_rate)` (also resets), `set_params(&CompressorParams)`, `process(l, r, out_l, out_r)` or `process_in_place(l, r)` (`min` of the four lengths, never panics), `tick(l, r)`, `reset()`, `gain_reduction_db()` (current, ≥ 0, make-up excluded), `take_peak_gain_reduction_db()` (largest since the last call, for a meter polled by the UI), `latency_samples()` (0), `params()`. `StaticCurve::new(T, R, W).gain_db(level)` and `output_db(level)` draw the transfer curve in the editor from the same code.
- **One compressor per stereo bus**, after the voice sum. `Compressor` is `Clone` and owns no heap memory, so carrying state across a plan swap is a `clone_from` and a `set_params`.
- **Smoothing in the catalogue.** Changes to threshold, ratio and knee already glide: they move the target of the gain smoother (time constant attack/2); the catalogue smooths them too (linear, 10 ms). `makeup` and `mix` are applied directly, so a step in either would click: the catalogue gives both the engine's smoother (linear, 10 ms), and the wrapper calls `set_params` once per 32-frame segment with the ramped value (only the changed fields are recomputed). `attack` and `release` have no smoothing: they jump their coefficients, which is inaudible except at extremes.
- **Metering.** Read `gain_reduction_db` or `take_peak_gain_reduction_db` after `process` and publish it to the UI through an atomic `f32`. The module itself holds no channel.
- **Bypass.** The compressor is not `time_based`, so the global FX bypass leaves it on (ADR-0006 note). When the patch disables the module, make it the identity and `reset()` it, so it does not resume with a stale reduction.
- **Denormals.** The module has a private `flush` with the same rule as `enth_dsp::util::flush::flush` (lane FX1: magnitudes below 1e-15 become `+0.0`). The two give identical results, so replacing the private one with the shared one after FX1 merges leaves the hash unchanged.
- **Budget.** 187 instructions per frame in the core (the lane limit is 250); the wrapper's gungraun `comp` case adds its own copying.
- **Hash.** Changing any coefficient formula, constant or the order of operations changes `HASH_GOLDEN`. That is a deliberate golden update (`docs`, commit message), like any other.
