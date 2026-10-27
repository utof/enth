# Phaser (`phaser`; `enth-dsp::phaser`, `enth-engine::modules::phaser`)

- **Status: built (S2a lane MOD, wave 4).** The module replaced the S0d stub; every acceptance row is in [Acceptance](#acceptance-how-each-row-is-met). Kind `phaser`, discriminant 14, section `fx`, id prefix `phaser` (phaser1, phaser2…), a time-based global effect. Still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag; the parameter table is lane CAT's F1m table and was not changed.
- Code: the DSP core is `crates/enth-dsp/src/phaser.rs` (the all-pass chain; the sweep helpers are [the chorus](chorus.md)'s, `crates/enth-dsp/src/chorus.rs`); the module is `crates/enth-engine/src/modules/phaser.rs`. Tests: white-box in the same two files, `crates/enth-engine/tests/phaser.rs` (the swept and the static notches against the closed form, beats at four host blocks, aliasing, level, a stage-count change, the plan swap, the 40 s tail, FTZ, no allocation), helpers in `tests/common/modfx.rs`. Goldens: `tests/golden/cases/phaser-*.toml` (8 cases). Bench: `crates/enth-engine/benches/phaser.rs`.
- **Provenance: nothing is ported.** The core is written from Vadim Zavalishin's *The Art of VA Filter Design* (the topology-preserving one-pole and the all-pass made from it) and Julius O. Smith III's *Introduction to Digital Filters* (the notch rule of an all-pass chain), and from the F1m table. No code from any library was copied, so there is **no `THIRD_PARTY_NOTICES.md` entry** for this lane (the acceptance row asks for one "per port"; there is none to make).
- Decisions behind it: research report 05 §3 (modulation effects) in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`, lane CAT's table and the F1m decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m)). Where this lane settles something the table left open, the section says so; the list is under [Limits and decisions](#limits-and-decisions).

A stereo phaser: 2 to 12 first-order all-pass filters in a chain, their corner swept around `center` by `depth` octaves (100 % is ±3 octaves), summed with the dry signal so that the chain's phase makes notches, with `feedback` around the whole chain. The level is bounded at every setting, the latency is 0, and nothing is allocated: the state is 128 bytes. The sweep is the chorus's 64-bit phase, so a synced sweep is exact in beats whatever the host's block size.

## Parameters (lane CAT, F1m)

*Built as specified. What the table leaves open (the sweep shape, the size of the level loss) is under [Limits and decisions](#limits-and-decisions); the whole path is drawn under [Signal path](#signal-path).*

The table lane MOD implements, generated from `crates/enth-patch/src/catalogue/phaser.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `stages` first-order TPT all-passes (an odd count is allowed), their corner swept around `center` by `depth` (100 % = ±3 octaves) at `rate` or one cycle per `beats` with `sync`; the right sweep lags by `width` × 180°; `feedback` around the chain.
- **The effect's own output is the dry signal plus the chain, level-matched** (the sum makes the notches): `wet = chain(v)` with `v[n] = (1 − |feedback|) · x[n] + feedback · wet[n − 1]`, and the output at `mix` 100 % is `(x + wet) / 2`; `mix` 0 % is the input bit for bit. The loop's input is scaled by `1 − |feedback|`, so the chain's gain peaks at 0 dB at every `feedback` (the bare loop peaks at `1 / (1 − |feedback|)`, +40 dB at 99 %), and the level-matched sum is 0 dB where the dry and the wet are in phase and not +6 dB: a freshly added phaser changes a full-scale saw's level by at most about 3 dB (catalogue rule 3). Lane MOD measures the level and may choose another normalisation with the same bound. Global: only global sources modulate it.
- **Defaults:** a gentle useful setting, six stages around 1 kHz, light feedback.

### `phaser` — Phaser (fx, ids `phaser1`, `phaser2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `stages` | none | 2 … 12 | int | 6 | auto | intermediate | How many all-pass filters are chained: every two make one notch, so more stages give more notches and a stronger sweep. |
| `rate` | hz | 0.01Hz … 20Hz | log | 0.3Hz | mod, auto | basic | How fast the notches sweep, in cycles per second, when tempo sync is off. |
| `sync` | none | off, on | toggle | off | auto | intermediate | On: one sweep lasts beats of the song's tempo, and rate is ignored. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar`, `two_bars_triplet`, `bar_dotted`, `two_bars`, `two_bars_dotted`, `four_bars`, `eight_bars`, `sixteen_bars` | enum | bar | auto | intermediate | How long one sweep lasts when tempo sync is on, as a note division. |
| `depth` | percent | 0% … 100% | linear | 50% | mod, auto | basic | How far the notches sweep around the center: 100 % is three octaves either way. |
| `center` | hz | 50Hz … 10kHz | log | 1kHz | mod, auto | intermediate | The frequency the notches sweep around; with depth at 0 % a static, comb-like color here. |
| `feedback` | percent | -99% … 99% | linear | 30% | mod, auto | intermediate | Sends the output back into the chain: sharper, ringing notches; negative moves the peaks between them. |
| `width` | percent | 0% … 100% | linear | 50% | auto | intermediate | How far apart the left and right sweeps run: 0 % is mono, 100 % opposite. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full phaser (the dry sound plus the swept all-passes). |


## Signal path

```text
v[n]   = (1 − |fb|) · x[n] + fb · wet[n − 1]       the chain's input
wet[n] = A^N (v[n])                                 N first-order all-passes, one corner fc
y[n]   = (x[n] + wet[n]) / 2                        mix 100 %
out    = (1 − mix) · x + mix · y                    mix 0: x, bit for bit
fc(t)  = center · 2^(3 · depth · s(t))              s the sine sweep, −1 … 1; depth 100 % is ±3 octaves
```

- **The sum makes the notches.** Where the chain's phase is an odd multiple of −π the all-pass output has the opposite sign to the dry and the sum cancels. One stage is `A = (1 − j·u)/(1 + j·u)` with `u = tan(ω/2)/g` and `g = tan(π·fc/fs)` (the bilinear transform pre-warped, so the corner is exactly `fc`), phase `−2·atan(u)`. So `N` stages put notches at

  ```text
  ω_k = 2 · atan( g · tan((2k + 1) · π / (2N)) )      for 2k + 1 < N      (an odd N adds a notch at Nyquist)
  ```

  Every two stages make one notch; an odd count adds a half. At `fc` = 1 kHz and 48 kHz (`enth_dsp::phaser::notch_hz`):

  | Stages | Notches (Hz) |
  | --- | --- |
  | 2 | 1,000 |
  | 4 | 415, 2,398 |
  | 6 | 268, 1,000, 3,665 |
  | 8 | 199, 669, 1,494, 4,863 |
  | 12 | 132, 415, 768, 1,302, 2,398, 7,058 |
  | 3, 5 | 578 and Nyquist; 325, 1,375 and Nyquist |

- **The feedback is taken from the previous output**, `wet[n − 1]`: the all-passes pass the input straight through (`A(∞) ≠ 0`), so a feedback without the unit delay would have no solution at the same sample. It is around the whole chain, as the table says. Negative feedback moves the resonances between the notches.
- **Stages.** `stages` is an integer 2 to 12 (NaN is 2). A change runs the old and the new count for one 32-frame segment (both are taps of the same chain) and blends linearly from one to the other, so a held tone does not click: the largest sample step across a 6 → 7 change is 0.0525 against the steady 0.0429 (1.2×, limit 1.7×), where a hard switch would jump by up to twice the amplitude. A removed stage's state is cleared afterwards; an added one starts from rest.
- **Width.** The right channel's sweep runs `width` percent of 180° behind the left's (`lag_for_width`, the chorus's). The two channels have separate chains and no cross-feed.

### The all-pass: topology-preserving, and why

Each stage is the TPT (zero-delay-feedback) one-pole low-pass `lp` with `G = g/(1 + g)`, turned into an all-pass as `y = 2·lp − x`:

```text
v = (x − s) · G        lp = v + s        s ← lp + v        y = 2·lp − x
```

The structure keeps the corner exact under modulation and, unlike a direct-form all-pass, is stable for any coefficient: a fast sweep or a jump of `center` cannot blow it up. `G` is computed once per segment for the sweep at the segment's end (`exp2` and `tan` through the pinned `libm`) and ramped linearly per sample from the previous segment's, so a fast sweep moves smoothly and the transcendentals are a small share of the fixed per-frame cost (see [Cost](#cost)).

## The level

The loop's input is scaled by `1 − |fb|` and a pure phase has magnitude 1, so `|wet/x| = (1 − |fb|)/|1 − fb·e^(−jω)·A^N| ≤ 1` at every frequency and every `fb`, `N` and `fc`: the chain peaks at 0 dB where the bare `x + fb · wet` loop would peak at `1/(1 − |fb|)` (+40 dB at 99 %); the sum with the dry signal is at most 1. What it does to a full-scale saw chord is a loss, from the notches and (with feedback) the loop's own scaling. Measured on a three-note saw chord (RMS over the last two seconds, the worse channel; `the_level_is_bounded_at_the_defaults_and_at_full_feedback`, stages 2/6/12 × centre 200/1,000/6,000 Hz):

| Setting | Level change | Peak against the dry chord's |
| --- | --- | --- |
| feedback 0 | −0.4 … −3.8 dB | ×0.89 … ×1.55 |
| feedback 30 % | −0.7 … −5.6 dB | ×0.73 … ×1.36 |
| **defaults** (6 stages, feedback 30 %, centre 1 kHz) | **−4.7 dB** | ×0.87 |
| feedback ±99 % | −6.0 dB | ×0.50 … ×0.59 |

The peak of a chord can rise where the chain re-phases the low partials: ×1.55 at two stages around 200 Hz, with the RMS 3.8 dB down. The test bounds it at ×1.6.

**Resolved 2026-10-27:** the coordinator relaxed rule 3 to "never louder, at most about 5 dB quieter", which these measurements meet. The original note: **this contradicted one sentence of the catalogue.** The doc comment of `catalogue/phaser.rs`, [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m) and the F1m rule 3 say a freshly added phaser changes a full-scale saw's level "by at most about 3 dB". The bound the rule exists for (never louder) holds; at the defaults the phaser loses **4.7 dB**, because the 30 % feedback scales the loop's input by 0.7 and the notches take their share. The table allows another normalisation that keeps the bound; this lane keeps the table's own `(x + wet)/2` and `1 − |fb|`, and the coordinator can change the sentence or choose a different scaling.

## Idle, NaN and `mix` 0

- **No allocation at all.** The state is 24 all-pass states, two feedback samples, two coefficients, a phase and the sample rate: **128 bytes** (a unit test holds the size). Nothing in `compile` or the audio thread allocates; `process_does_not_allocate` runs a chord with `assert_no_alloc`.
- **Idle.** The wrapper scans for silence after every silent segment (26 floats): once the input and output of a segment are exactly zero and the state is exactly zero, it stops running the core, and the first non-silent input wakes it with the sweep restarted at phase 0, so skipping the module and running it are the same bits (`skipping_an_idle_phaser_is_bit_identical_to_running_it`).
- **NaN and infinity** never reach the filters: NaN becomes 0 and anything beyond ±1e6 is clamped, per segment.
- **`mix` 0 passes the input bit for bit**: the block is not written. The core still runs, so the states stay warm and a rising `mix` meets a settled chain; `mix0_1v` in [Cost](#cost) is that.
- **Bypass.** `time_based` in the catalogue: the global FX bypass fades it out, stops calling it, and expects a `reset()` before it fades back in.

## The plan swap

Both swap kinds copy the core's 128 bytes (`take_state_from` is the default copy: a swap of anything this small costs less than the bookkeeping to avoid it). It adds nothing measurable to the swap block: see [chorus.md](chorus.md#memory-and-the-plan-swap) for the measurement with both effects in the chain.

## Measured: the defining response

All through the engine (`tests/phaser.rs`). Measured on x86_64 Linux, 2026-10-26.

| Check | Result | Test |
| --- | --- | --- |
| **Static notches match `2·atan(g·tan((2k + 1)π/(2N)))`** within 2 % for every stage count | stages 2 … 12 (white-noise transfer function, sweep frozen at `depth` 0, centre 1 kHz): worst deviation **0.30 %** (limit 2 %) | `the_static_notches_are_the_all_pass_chains_closed_form_for_every_stage_count` |
| The static response is the closed form | the whole complex response against `enth_dsp::phaser::response`: error 1.8e-3 at 2 stages rising to 1.1e-2 at 12 (the noise estimate's own bias, `memory/len`; the test's limit is `1e-3·N + 1e-3`) | the same |
| **Swept notches follow the closed form** | a sine probe through the phaser against `H(fc(t))` averaged over each window, `fc(t)` from the definition in `f64`: worst gain error 9.6e-4 (2 stages, 0.4 Hz), 2.7e-3 (3 stages, 0.6 Hz), 5.0e-3 (the defaults), 7.6e-3 (12 stages, 0.1 Hz), 1.05e-2 (5 stages, feedback −60 %), 3.15e-2 (12 stages, feedback 60 %, the sharpest notches). The error rises with the sweep rate and the feedback (the closed form is the frozen response and a moving corner is not frozen); at a tenth of the rate it is 10 … 29 times smaller, so the implementation is not what limits it | `the_swept_notches_follow_the_closed_form` |
| A stage-count change does not click | 1.2× the steady step (6 → 7), within 1.7× | `a_change_of_stage_count_does_not_click` |
| **Synced, exact in beats** | bars across 140 → 174 → 63 → 140 BPM, 11.32 cycles in the render: within 1.7e-3 of the closed form (gain error), and the render is the same bits at host blocks 1, 64, 100 and 4096 | `a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block` |

## Aliasing

*Measured with the shared harness ([aliasing.md](aliasing.md)): research report 02's measure, a sine at eight keys from 110 Hz to 10.5 kHz through the module. Test file `tests/phaser.rs`; the bound is the measured worst plus 1.5 dB.*

A phaser has no delay line and no table to read: an all-pass chain is a rational filter, and a swept corner makes sidebands at multiples of the sweep rate and nothing else. So the measure stays at the sine's own floor.

| Case | Worst measure (dB) per key, 110 Hz … 10.5 kHz | Bound (dB) |
| --- | --- | --- |
| **Frozen** (depth 0): 6 stages with 30 % feedback, 12 with 60 %, 2 at 8 kHz with −60 % | worst −92.6 over all keys (−103.4, −97.7, −93.3, −95.5, −92.6, −119.5, −113.7, −98.2 per key) | −89.3 |
| **Slowest full-depth sweep** (0.01 Hz, 100 %) and **the defaults** (0.3 Hz, 50 %), the same three chains | −101.1, −97.6, −93.2, −93.0, −92.2, −97.0, −93.7, −90.8 (the defaults' sidebands come within 8 bins of the guard at the top key) | −89.3 |
| Fastest sweep (20 Hz, 100 %) | −7.1, −4.2, −3.4, −1.9, −2.7, −4.7, −4.5, −6.8: the modulation's own sidebands (the sweep rate times a modulation index, many hundreds of Hz), recorded and not bounded | none |

## Cost

Instruction counts under callgrind (`cargo xtask bench --bench phaser`), x86_64 Linux, pinned toolchain, 2026-10-26: 1 s at 48 kHz of a held saw chord (`prepare` and the chord are outside the measured function), with the `engine` bench's 3,616,649 (1 voice) and 37,296,489 (16 voices) subtracted to read the module's own cost. The effect is global, so its cost does not depend on the voice count; the 16-voice case checks that.

| Case | Instructions | Budget (+10 %) | Own cost per frame |
| --- | --- | --- | --- |
| `default_1v` (6 stages, 0.3 Hz, feedback 30 %) | 20,794,530 | 22,874,000 | 357.9 |
| `default_16v` | 54,474,370 | 59,922,000 | 357.9 |
| `stages2_1v` | 12,643,530 | 13,908,000 | 188.1 |
| `stages12_1v` | 32,727,030 | 36,000,000 | 606.5 |
| `mix0_1v` | 20,026,529 | 22,030,000 | 341.9 |

From two to twelve stages the cost grows by 41.8 instructions per stage and frame (about 21 per stage and channel sample: the TPT's five operations and the tap bookkeeping), over a fixed 104 per frame for the feedback, the two sweeps, the blend and the coefficient ramps. The serial dependence of the feedback keeps the chain scalar. Instruction counts under-report a subnormal penalty, so the 40 s tail test and the FTZ test stay mandatory. A change to `enth-dsp::phaser` re-measures all five in the same commit.

## Limits and decisions

1. **The level loss is 4.7 dB at the defaults**, not 3 (the catalogue's sentence; see [The level](#the-level)).
2. **The sweep is a sine** (the chorus's `sine_of`); the table does not name a shape.
3. **The coefficient is exact at every segment boundary and ramped linearly in between**: the corner's curvature over a 0.67 ms segment is far below anything audible at the rates the table allows.
4. **An odd stage count's last notch is at Nyquist**, outside the audible band: five stages sound like four with the resonances shifted.
5. **The corner is clamped to 1 Hz … 0.45 · sample rate** (21.6 kHz at 48 kHz): a `center` of 10 kHz with `depth` 100 % reaches 80 kHz in the formula and stays at 21.6 kHz.
6. **`mix` 0 still costs the whole chain** (state kept warm), as in the chorus.
7. **A tempo change** applies at the next 32-frame segment boundary (the engine's rule).

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/phaser.rs` (lane CAT's F1m table, unchanged) |
| DSP core | `crates/enth-dsp/src/phaser.rs` (the sweep helpers are `crates/enth-dsp/src/chorus.rs`'s) |
| Engine module | `crates/enth-engine/src/modules/phaser.rs`; `params.rs` gets one additive `param_index!(ChorusParam, PhaserParam)` |
| Tests | the two files above; `crates/enth-engine/tests/phaser.rs`, `tests/common/modfx.rs` |
| Instruction budget | `crates/enth-engine/benches/phaser.rs` |
| Golden cases | `tests/golden/cases/phaser-*.toml`: `default`, `stages-4`, `stages-5`, `stages-12`, `negative-feedback`, `synced`, `mix-half`, `lfo-center` (a global LFO on `center` and an automation of `stages`) |

## Acceptance: how each row is met

| Row (phase-s2.md, every module lane; then lane MOD's own) | How | Where |
| --- | --- | --- |
| Same bits on six targets | `f32` with `util::math` and the project's polynomial sine, fixed order, no fused operations; 8 golden cases, equal on x86_64 Linux (`cargo xtask golden`) and on `wasm32-wasip1` under wasmtime 48.0.6 (`cargo xtask golden --wasm`, 2026-10-26); the Windows and macOS targets run in CI | the goldens |
| FTZ-independent | the states are flushed once per block and the feedback sample on every write; the render is the same bits under every FTZ/DAZ setting | `the_render_is_the_same_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | 0 subnormal output samples and states | `a_40_s_tail_through_the_phaser_has_no_subnormals` |
| Latency 0 | the dry signal is not delayed; the module reports none | `latency_is_zero` |
| Budget within +10 % | five cases ([Cost](#cost)) | `benches/phaser.rs` |
| Two instances keep separate state | two modules in one chain differ from each other and from one | `two_instances_keep_separate_state` |
| `mix` 0 passes the input bit for bit | not written; the render equals the one without the module, at 2, 5 and 12 stages | `mix_zero_passes_the_input_bit_for_bit` |
| Aliasing measured, bound recorded | [Aliasing](#aliasing) | `the_phaser_adds_no_aliasing_frozen_slow_or_at_the_defaults` |
| Notches within 2 % of the closed form | 0.30 % | `the_static_notches_are_the_all_pass_chains_closed_form_for_every_stage_count` |
| Level bound at ±99 % and at the defaults | [The level](#the-level) | `the_level_is_bounded_at_the_defaults_and_at_full_feedback` |
| Synced rates exact in beats at blocks 1, 64, 100, 4096 across a tempo change | [Measured](#measured-the-defining-response) | `a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block` |
| No allocation | `assert_no_alloc` around a chord through the module | `guarded::process_does_not_allocate` |
| The swap's copied buffers | 128 bytes; [The plan swap](#the-plan-swap) | `the_plan_swap_carries_the_filter_state` |
| Provenance and a `THIRD_PARTY_NOTICES.md` entry per port | nothing is ported: no entry | the top of this page |

## Sources

- Vadim Zavalishin, *The Art of VA Filter Design* (Native Instruments): the topology-preserving one-pole, the all-pass as `2·lp − x`, and the stability of the structure under modulation.
- Julius O. Smith III, *Introduction to Digital Filters with Audio Applications* (CCRMA, online book): the phase of first-order all-passes, the bilinear transform's frequency pre-warp, and phasing as the sum of a signal with its all-pass-filtered copy.
- Research report 05 §3 (modulation effects) in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`.
- Lane CAT's F1m table and the S2a decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m)); [aliasing.md](aliasing.md) for the measure.
