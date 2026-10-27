# Chorus (`chorus`; `enth-dsp::chorus`, `enth-engine::modules::chorus`)

- **Status: built (S2a lane MOD, wave 4).** The module replaced the S0d stub; every acceptance row is in [Acceptance](#acceptance-how-each-row-is-met). Kind `chorus`, discriminant 13, section `fx`, id prefix `chorus` (chorus1, chorus2…), a time-based global effect. Still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag; the parameter table is lane CAT's F1m table and was not changed.
- Code: the DSP core is `crates/enth-dsp/src/chorus.rs` (the modulated-delay core, and the sweep helpers [the phaser](phaser.md) shares); the module is `crates/enth-engine/src/modules/chorus.rs`. Tests: white-box in the same two files, `crates/enth-engine/tests/chorus.rs` (the delay law, the comb, beats at four host blocks, aliasing, level, the plan swap, the 40 s tail, FTZ, no allocation), helpers in `tests/common/modfx.rs`. Goldens: `tests/golden/cases/chorus-*.toml` (7 cases). Bench: `crates/enth-engine/benches/chorus.rs`.
- **Provenance: nothing is ported.** The core is written from the equations of Julius O. Smith III's *Physical Audio Signal Processing* and from the F1m table; the ensemble's structure (three taps 120° apart on a slow and a fast sweep) is the one the table names in Mutable Instruments' Plaits and Rings, and the numbers (the fast sweep's ratio and share) are this lane's own. No code from any library was copied, so there is **no `THIRD_PARTY_NOTICES.md` entry** for this lane (the acceptance row asks for one "per port"; there is none to make).
- Decisions behind it: research report 05 §3 (modulation effects) in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`, lane CAT's table and the F1m decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m)). Where this lane settles something the table left open, the section says so; the list is under [Limits and decisions](#limits-and-decisions).

One stereo modulated delay with three modes: **chorus** (one sine-swept tap per channel), **ensemble** (three taps per channel on a slow and a fast sweep) and **flanger** (one triangle-swept tap per channel with feedback, reaching a one-sample delay without delaying the dry signal). The level is bounded at every setting and the latency is 0. The sweep is a 64-bit phase, so a synced sweep is exact in beats whatever the host's block size.

## Parameters (lane CAT, F1m)

*Built as specified. What the table leaves open (the sweep shapes, the ensemble's constants, the size of the level loss) is under [Limits and decisions](#limits-and-decisions); the whole path is drawn under [Signal path](#signal-path).*

The table lane MOD implements, generated from `crates/enth-patch/src/catalogue/chorus.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** a tap's delay is `delay · (1 + depth/100 · s(t))`, `s` the sweep (−1…1) at `rate` (or one cycle per `beats` with `sync`); the right sweep lags the left by `width` × 180°; `ensemble` is three taps per channel on a slow and a fast sweep (Plaits' ensemble), each sweeping by `depth`. `feedback` returns the taps into the delay.
- **`depth` is a percentage of `delay`** (a ratio: a flanger at 1 ms and a chorus at 20 ms sweep in the same proportion). 0–100 % is the physical range: at 100 % the delay swings between 0 and twice `delay` (clamped to at least one sample), and more could only push it below zero. In milliseconds at the default: 50 % of 7 ms is ±3.5 ms. The delay line holds twice `delay`'s maximum (60 ms) and its interpolation margin. The pitch swing is about `2π · rate · depth · delay`: ±19 cents at the defaults (0.5 Hz, 50 % of 7 ms); at the extreme (20 Hz, 100 % of 30 ms) the read head runs backwards, a warble and no instability.
- **The effect's own output is the dry signal plus the taps, level-matched**: with `wet` the mean of a channel's taps (unity gain together) the output at `mix` 100 % is `(x + wet) / 2`, and `mix` blends the input (0 %, bit for bit) with that sum. `feedback` returns the taps into the delay with its input scaled by `1 − |feedback|`, `v[n] = (1 − |feedback|) · x[n] + feedback · tap[n]`, so the loop's gain peaks at 0 dB at every `feedback` (the bare loop peaks at `1 / (1 − |feedback|)`, +40 dB at ±99 %) and the sum is 0 dB where dry and wet are in phase, not +6 dB: a freshly added chorus changes a full-scale saw's level by at most about 3 dB (catalogue rule 3). Lane MOD measures the level and may choose another normalisation with the same bound. Global: only global sources modulate it. A through-zero flanger can be appended to `mode` later.
- **Defaults:** a gentle useful setting, a slow, wide chorus (0.5 Hz, 7 ms, depth 50 % = ±3.5 ms).

### `chorus` — Chorus (fx, ids `chorus1`, `chorus2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `chorus`, `ensemble`, `flanger` | enum | chorus | auto | basic | The algorithm: chorus, ensemble or flanger. |
| `rate` | hz | 0.01Hz … 20Hz | log | 0.5Hz | mod, auto | basic | How fast the delay sweeps, in cycles per second, when tempo sync is off: slow is a gentle shimmer, fast a warble. |
| `sync` | none | off, on | toggle | off | auto | intermediate | On: one sweep lasts beats of the song's tempo, and rate is ignored. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar`, `two_bars_triplet`, `bar_dotted`, `two_bars`, `two_bars_dotted`, `four_bars`, `eight_bars`, `sixteen_bars` | enum | bar | auto | intermediate | How long one sweep lasts when tempo sync is on, as a note division. |
| `depth` | percent | 0% … 100% | linear | 50% | mod, auto | basic | How far the delay sweeps either side of the delay time, as a share of it: 50 % of 7 ms is plus or minus 3.5 ms, and 100 % sweeps between none and double. More is a deeper wobble in pitch (chorus) or a wider sweep of the notches (flanger). |
| `delay` | ms | 0.1ms … 30ms | log | 7ms | mod, auto | intermediate | The delay the sweep moves around: about 5-20 ms for a chorus, 0.5-5 ms for a flanger; an LFO here is a manual flange. |
| `feedback` | percent | -99% … 99% | linear | 0% | mod, auto | intermediate | Sends the delayed sound back into the delay: a ringing, metallic flange; negative gives hollower notches. |
| `width` | percent | 0% … 100% | linear | 100% | auto | intermediate | How far apart the left and right sweeps run: 0 % is mono, 100 % opposite, the widest. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full chorus (the dry sound plus the wobbling copies). |


## Signal path

```text
tap(t) = delay · (1 + depth · s(t))              s the sweep, −1 … 1; the tap is at least one sample
v[n]   = (1 − |fb|) · x[n] + fb · wet[n]         what the delay line records
wet[n] = the mean of a channel's taps, read from v at lag tap(n)
y[n]   = (x[n] + wet[n]) / 2                     mix 100 %
out    = (1 − mix) · x + mix · y                 mix 0: x, bit for bit
```

- **The dry signal is never delayed**, so the module adds no latency (it reports 0, and `latency_is_zero` finds the first sample of a note where the dry one is). The sum with the dry signal is level-matched, `(x + wet)/2`.
- **`depth` is a percentage of `delay`**, as F1m settled it: `tap = delay · (1 ± depth/100)`. 100 % swings a tap between the one-sample floor and twice `delay`; a 7 ms chorus at 50 % sweeps 3.5 … 10.5 ms.
- **`mode`**: `chorus` is one tap per channel on a sine sweep, `flanger` one tap per channel on a **triangle** sweep, `ensemble` three taps per channel (below). The three share the line, so a `mode` change loses no state, but it takes effect at the next segment boundary and changes the wet signal at once: switch it between notes if a click would matter.
- **Width.** The right channel's sweep runs `width` percent of 180° behind the left's (`lag_for_width`: an integer phase, so 0 % is exactly mono and 100 % exactly opposite). The two channels have separate lines and no cross-feed.
- **Feedback** returns the mean of a channel's taps (`wet`) into that channel's own line. The input to the line is scaled by `1 − |fb|`, so the loop's peak gain is 0 dB at every `fb` (see [The level](#the-level)).
- **Ensemble.** Three taps per channel, 0°, 120° and 240° apart, each swept by `depth` around `delay` with `s = 0.9 · sine(phase) + 0.1 · sine(8 · phase)`: the slow sweep runs at `rate`, the fast one eight times faster, and the fast one's share is 10 % so that `|s| ≤ 1` and the range stays `delay ± depth`. The weights 0.9, 0.1 and the ratio 8 are this lane's (the table says "a fixed ratio" and no more), chosen to give the slow drift its small fast shimmer; the output is the mean of the three taps.

### The sweep

A 64-bit phase accumulator advanced by an integer per 32-frame segment. The increment is `lfo::phase_increment(rate)` or, with `sync`, `lfo::synced_increment(tempo, beats)`: an integer for the whole segment, so a synced sweep completes exactly one cycle per `beats` at the tempo of that segment, and a tempo change takes effect at the next segment boundary whatever the host's block size. Inside a segment the phase of frame `i` is `start + ⌊inc · (i + 1)/32⌋`, exact in integers and equal to `start + inc` on the last frame, so segment boundaries carry no rounding. The sine is the project's polynomial (`osc::sine`) of the top 24 bits of the phase.

**Chorus against flanger.** The F1m table does not say what separates the two modes beyond their `delay` ranges and feedback. This lane makes the flanger's sweep a **triangle** (the shape of a tape flanger's, and a constant speed through the notches, which is what makes the "jet" sweep audible as even), and the chorus's a **sine** (a smooth lingering vibrato at the ends of the sweep). Nothing else differs: `feedback` works in every mode, and the ranges are the table's. The mode is therefore a choice of sweep shape and tap count, and a flanger with a sine sweep is a chorus with a short `delay`.

## The tap: Lagrange interpolation, and the flanger's zero delay

A tap at `τ = ⌊τ⌋ + x` is read with a **four-point (cubic) Lagrange interpolator** over the lags `⌊τ⌋ − 1 … ⌊τ⌋ + 2`. Its weights sum to 1, it reproduces cubics, and its magnitude response is at most 1 at every fraction (a unit test checks 201 fractions at 401 frequencies), which is what keeps the loop's gain bound. Compared with the all-pass or linear alternatives it needs no state (a fractional delay that moves does not ring), is flat at low frequencies and keeps the bound; its price is a high-frequency droop and a fraction-dependent error that appears as aliasing-like sidebands at the highest frequencies (see [Aliasing](#aliasing)).

**Zero delay without latency.** A tap's delay is clamped to at least **one sample**: a tap at `τ = 1` reads the sample that is being written. With feedback that is a loop with no delay, which the module solves instead of avoiding. With `c0` the Lagrange weight of lag 0 (nonzero only for `1 ≤ τ < 2`) and `r` the sum of the taps at lags ≥ 1 (all already recorded),

```text
v   = ((1 − |fb|) · x + fb · r) / (1 − fb · c0)        wet = c0 · v + r
```

is exactly the equation `v = (1 − |fb|) · x + fb · wet` of the table, solved for the one unknown. `|fb · c0| ≤ 0.064` (the weight of lag 0 is at most 0.0642 in magnitude), so the divisor stays within 0.93 … 1.07. The result is a flanger that sweeps down to the **zero-delay null** (the first comb notch at `fs/(2τ)` reaches Nyquist when `τ` is one sample, where the comb leaves the audible band) **without delaying the dry signal**. A *through-zero* flanger, whose delay passes through zero to the other side of a fixed dry delay, is not a mode: it needs the dry signal delayed, which is latency (D10, ADR-0006). A mode can be appended to `mode` later, as the table says.

**The flanger's notches** (no feedback) are at the odd multiples of `fs/(2τ)`: `f_k = (2k + 1)·fs/(2τ)`, spaced `fs/τ` apart and all moving as `1/τ` while the delay sweeps. At 2 ms ± 80 % (`delay` 2, `depth` 80) `τ` runs from 0.4 to 3.6 ms, the first notch from 1,250 Hz down to 139 Hz, the next at 3× and 5× that. With the feedback, the response is `H = (1 + (1 − |fb|)·K/(1 − fb·K))/2` (`K` the tap's frequency response); positive feedback sharpens the peaks at the multiples of `fs/τ` (a ringing flange over a floor of 0.5), negative feedback sharpens the notches (the hollow flange).

## The level

The output is `(x + wet)/2` and the loop's input is scaled by `1 − |fb|`. With `|K| ≤ 1`, `|wet/x| = (1 − |fb|)·|K|/|1 − fb·K| ≤ 1` at every frequency, whatever `fb` and `τ`, so the feedback path peaks at 0 dB where the bare loop would peak at `1/(1 − |fb|)` (+40 dB at 99 %); and the sum with the dry signal is `|1 + wet/x|/2 ≤ 1`. White noise through the module never gains level at the extreme feedbacks (a unit test), and a 99 % tail decays to exact zero without a subnormal.

What it does to a full-scale saw chord is a loss, not a gain: where the dry and the delayed copy are not in phase the sum is below 0 dB. Measured on a three-note saw chord (RMS over the last two seconds, the worse channel; `the_level_is_bounded_at_the_defaults_and_at_full_feedback`):

| Setting | Level change | Peak against the dry chord's |
| --- | --- | --- |
| chorus, feedback 0, delay 1 / 7 / 30 ms | −2.3 … −3.2 dB (**−3.18 dB at the defaults**) | ×0.50 … ×0.91 |
| flanger, feedback 0 | −2.3 … −3.0 dB | |
| ensemble, feedback 0 | −2.8 … −4.8 dB (the wet part is the mean of three decorrelated taps) | |
| any mode, feedback ±99 % | −6.0 dB (the loop's input is scaled by 0.01, so the wet part shrinks where it resonates) | never above ×1.07 |

**Resolved 2026-10-27:** the coordinator relaxed rule 3 to "never louder, at most about 5 dB quieter", which these measurements meet. The original note: **this contradicted one sentence of the catalogue.** The doc comment of `catalogue/chorus.rs`, [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m) and the F1m rule 3 say a freshly added chorus changes a full-scale saw's level "by at most about 3 dB". The bound the rule exists for (never louder, no +6 dB, no +40 dB) holds; the size of the loss does not quite: the chorus at its defaults loses 3.18 dB and the ensemble up to 4.8 dB. The table allows another normalisation that keeps the bound ("Lane MOD measures the level and may choose another normalisation that keeps the same bound"); this lane keeps `(x + wet)/2` because it is the table's own sum, the loss is the comb of a real chorus (a make-up gain would push the in-phase frequencies, which are already at 0 dB, above the input and break the bound), and the coordinator can change the sentence ("at most about 3 dB" → "about 3 to 5 dB below") or the sum.

## Memory, and the plan swap

- **Allocation.** The only allocation is `Chorus::new`, which `compile` calls (never the audio thread): two lines of `line_len(fs)` samples, the smallest power of two that holds the longest tap (twice the catalogue's 30 ms) and the interpolator's reach. That is **4,096 samples at 44.1 and 48 kHz (32 KiB for the pair)**, 8,192 at 88.2 and 96 kHz (64 KiB) and 16,384 at 176.4 and 192 kHz (128 KiB), sized from the sample rate alone, never from the parameter values (a unit test), so a `delay` change never reallocates. `process_does_not_allocate` runs a chord through the module with `assert_no_alloc`.
- **The swap.** A *crossfading* plan swap runs the old and the new plan side by side for 256 frames, so the surviving modules' state is copied: the chorus copies its 32 KiB (`copy_state_from`). An *instant* swap exchanges the two cores in O(1) (`take_state_from`, `mem::swap` of the line pointers; the test `take_moves_the_state_and_copy_copies_it` checks the address). The copy is a memory-bandwidth cost, and [benchmarks.md](../benchmarks.md#the-plan-swap) names it as a risk of a second large-state effect: the reverb's 1.6 MB of lines took P1b's engine-only p99 swap block to 797 µs (the move reduced it to 139 µs; a crossfading swap still copied, p99 1,052 µs) against the 1,333 µs deadline. Measured here with the same harness (`enth-test-host process-stats --swap-state`, 64-frame blocks at 48 kHz, an edit every 250 ms, 120 s a run, G1 and its 16-note chord, release bundle of this tree, `flock -x`):

| Plan edit | Patch | Runs | Normal block p50 | Swap block p50 | Swap block p99 | Swap block max | Deadline misses |
| --- | --- | --- | --- | --- | --- | --- | --- |
| instant (osc1's level) | G1 | 2 | 69 … 76 µs | 86 … 87 µs | 212 … 349 µs | 381 … 430 µs | 0, 0 |
| instant | G1 + `chorus` + `phaser` | 2 | 67 … 92 µs | 75 … 104 µs | 213 … 342 µs | 330 … 406 µs | 0, 0 |
| crossfade (a voice LFO added) | G1 | 4 | 66 … 202 µs | 277 … 757 µs | 919 … 1,150 µs (median 1,049) | 1,033 … 1,726 µs | 0, 0, 4, 0 |
| crossfade | G1 + `chorus` + `phaser` | 4 | 88 … 92 µs | 306 … 381 µs | 678 … 1,223 µs (median 1,086) | 1,080 … 8,311 µs | 1, 9, 0, 0 |

(G1 plus the two effects at their defaults between `comp1` and `reverb1`. The runs alternate between the two patches. Raw reports and the patch files: `~/code/enth-scratch/s2/mod/swap/`.)

- **The p99 of every run is under the 1,333 µs deadline** (at most 1,223 µs), **and the effects change nothing beyond the run-to-run spread**: the medians of the crossfading swap block's p99 are 1,049 µs without and 1,086 µs with the two effects, against a spread of 231 µs between runs of the same patch without them and 545 µs with them. The p50 of the swap block is about 50 µs higher on the calm-core runs (the means of 277 and 332 µs without, 306 … 381 µs with: 305 against 354), of which the copy is at most 15 µs (below) and the rest is the two effects' own work running in both plans through the crossfade.
- **This machine was not quiet.** The load average was 3 … 14 during every run (the owner's other tools and the other lanes' builds, which the exclusive lock cannot keep out), and P1b's quiet-machine figure for the same swap block is 337 µs (p99, 25.2 … 25.4 % of the deadline; [benchmarks.md](../benchmarks.md#lane-rt-the-plugin-measured-2026-10-20)). Here the *same G1 patch without these effects* reads 919 … 1,150 µs, so the absolute figures are the machine's, the pairs are what this lane measured, and the benchmarks page's gate (p99 under 25 % of the deadline) is not decidable from them. The maxima beyond the deadline are scheduler events: 1,726 µs without the effects (4 misses) and 8,311 µs with them (9 misses, at a load average of 13.6).
- **The copy itself, measured alone** (`swap_copy_cost_is_microseconds`, an ignored test run under the exclusive lock, 2,000 copies each, twice; "cold" flushes the caches between copies, the state after a sleep):

  | Copy | Warm p50 / p99 | Cold p50 / p99 | Cold max |
  | --- | --- | --- | --- |
  | chorus at 48 kHz, 32 KiB | 0.9 / 1.3 … 1.6 µs | 5.1 … 5.7 / 12.0 … 15.0 µs | 29 … 40 µs |
  | chorus at 96 kHz, 64 KiB | 1.5 … 1.9 / 1.9 … 2.9 µs | 6.7 … 9.0 / 17.4 … 23.0 µs | 35 µs |
  | chorus at 192 kHz, 128 KiB | 2.8 … 3.6 / 4.0 … 5.3 µs | 11.0 … 19.4 / 25.4 … 43.4 µs | 40 … 65 µs |
  | the reverb's rings, 1.6 MB (for scale) | 48 … 86 / 65 … 136 µs | 132 … 213 / 221 … 452 µs | 250 … 550 µs |

  So the chorus adds at most 15 µs (1.1 % of the deadline) to a crossfading swap block at 48 kHz and 43 µs at 192 kHz, against the reverb's 221 … 452 µs; the phaser's 128 bytes are not measurable. An instant swap exchanges the two cores in O(1) and copies nothing. The reopening conditions of [benchmarks.md](../benchmarks.md#the-plan-swap) ("a second large-state effect lands") are not met by the chorus; the `delay`, whose rings are the large ones, is the lane to watch.


## Idle, NaN and `mix` 0

- **Idle.** A silent input into silent lines costs nothing: the wrapper scans the lines once after 0.68 s (32,768 frames) of exact zeros in and out and, if they hold only zeros, stops running the core; the first non-silent input wakes it with the sweep restarted at phase 0, so skipping the module and running it are the same bits (`skipping_an_idle_chorus_is_bit_identical_to_running_it` in the wrapper). A live tail is scanned again at growing intervals up to 22 s.
- **NaN and infinity** never reach the lines: each segment's input is checked first (NaN becomes 0, anything beyond ±1e6 is clamped), because one such sample would circulate in the loop until `reset()`.
- **`mix` 0 passes the input bit for bit**: the block is not written. The core still runs, so the lines keep recording and a `mix` that rises from 0 hears a filled line and not a 60 ms hole; the cost of `mix0_1v` in [Cost](#cost) is that. (The distortion costs nothing at `mix` 0 because it has no state to keep warm; the chorus and the phaser do.)
- **Bypass.** The chorus is `time_based` in the catalogue, so the global FX bypass fades it out, stops calling it and expects a `reset()` before it fades back in; `reset()` clears the lines and restarts the sweep.

## Measured: the defining response

All through the engine (`tests/chorus.rs`) unless marked *core* (`enth-dsp::chorus` unit tests). Measured on x86_64 Linux, 2026-10-26.

| Check | Result | Test |
| --- | --- | --- |
| The delay of an impulse over a cycle is `delay · (1 + depth · s)`, 64 sweep phases, chorus and flanger | within 2e-4 sample; the swing is exactly 168.0 … 504.0 samples for 336 ± 50 % (1e-3) | *core* `the_impulse_delay_follows_the_sweep_exactly` |
| 100 % depth swings between none and double | the clamp at 1.000 sample, double at 200.000 for `delay` 100 | *core* `at_full_depth_the_delay_swings_between_none_and_double` |
| A moving sweep (1 Hz, impulses every 2,048 samples for two cycles) stays inside `delay ± depth` | the delays seen reach below 175 and above 497 samples and never leave 167.5 … 504.5; each impulse within 0.12 sample of the sweep | *core* `a_moving_sweep_keeps_the_delay_inside_its_range` |
| The module's wet part is the input read at `n − τ(n)` | wet = 2y − x against a 64-point windowed-sinc read of the dry render, `τ` from the catalogue's definition in `f64`: within 1.0e-5 … 2.5e-5 of the peak (limit 1e-4), chorus and flanger, widths 0, 40 and 100 % | `the_swept_delay_follows_the_catalogue_law` |
| A frozen sweep is the closed-form comb | the transfer function from white noise (65,536 points) against `H = (1 + (1 − \|fb\|)·K/(1 − fb·K))/2`, notches included: within 2.8e-3 … 5.6e-3 (limit 8e-3; the noise estimate's own bias is `memory/len`) | `the_static_comb_matches_the_closed_form_through_the_engine` |
| The feedback loop is the geometric series of the delay | within 1e-7 … 8e-7 | `the_feedback_loop_is_the_geometric_series_of_the_delay` |
| The flanger's notches are at `(2k + 1)/(2τ)` | four delays (24 … 144.25 samples), within 1 % | *core* `the_flanger_notches_are_the_odd_multiples_of_half_over_tau` |
| The right channel lags by the width; the ensemble's taps are 120° apart | exact | *core* `the_right_channel_lags_the_left_by_the_width`, `the_ensemble_runs_three_taps_a_third_of_a_turn_apart` |
| The flanger reaches the zero-delay null without delaying the dry signal | the first notch at Nyquist at `τ` = 1, the dry sample on time | *core* `the_flanger_reaches_its_zero_delay_null_without_delaying_the_output`, `the_dry_signal_is_never_delayed` |
| **Synced, exact in beats** | quarter notes across 140 → 174 → 63 → 140 BPM, 45.27 cycles in the render: within 8.9e-6 of the closed form in beats, and the render is the same bits at host blocks 1, 64, 100 and 4096 | `a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block` |

## Aliasing

*Measured with the shared harness ([aliasing.md](aliasing.md)): research report 02's measure (alias power over harmonic power, 20 Hz – 20 kHz, 32,768 samples under a 4-term Blackman-Harris window, ±8 bins around each harmonic counted as signal), a sine at eight keys from 110 Hz to 10.5 kHz through the module. Test file `tests/chorus.rs`; the bounds are the measured worst plus 1.5 dB.*

A modulated delay does two things a measure of "everything that is not a harmonic" cannot tell apart, and the page records both.

1. **A moving delay is a frequency modulation.** A tone `f0` through a delay that moves by `Δτ` has a phase deviation of `2π·f0·Δτ` radians: at the defaults (3.5 ms) 77 rad at 3.5 kHz, a band of about ±40 Hz around the tone, far outside the ±8 guard bins. That is the chorus effect itself and not aliasing: it is the same at any sample rate and folds nothing. At the defaults and above the measure reads up to −13 dB (3.5 kHz, default chorus) and about 0 dB (20 Hz, 100 %, 30 ms) for this reason, and the table below holds them as information, not a bound.
2. **The interpolator's error.** The four-point Lagrange read is not exact: its error depends on the fraction of the tap, and the fraction cycles at the Doppler rate, so a tone gets sidebands at ± that rate whose size is the interpolation error at its frequency. This is a real, small, pure artefact and the thing the bound limits. At the slowest sweep (0.01 Hz) and the deepest (100 %) the Doppler shift of a 7 ms tap is 4.4e-4 of the frequency and its own FM sidebands stay inside the guard bins, so what the measure sees outside them is this error.

| Case | Worst measure (dB) per key, 110 Hz … 10.5 kHz | Bound (dB) |
| --- | --- | --- |
| **Frozen sweep** (depth 0), chorus, ensemble, flanger with 60 % feedback | worst −92.6 over all keys: the window's floor, equal to the oscillator's own | −91.1 |
| **Slowest full-depth sweep** (0.01 Hz, 100 %), chorus, ensemble, flanger (2 ms, 60 %) and a mono chorus | −103.4, −96.5, −93.4, −92.5, −90.5, −67.3, −49.9, −35.1 | −101.9, −95.0, −91.9, −91.0, −89.0, −65.8, −48.4, −33.6 |
| Default rate and above | the FM sidebands dominate: information only | none |

The cubic's error grows as the fourth power of the frequency, so the bound is −90 dB or better up to 1.76 kHz, −67 dB at 3.5 kHz and rises to −35 dB at 10.5 kHz. That is a property of the interpolation that was chosen for its bounded level and statelessness; a higher-order or windowed-sinc read would push it down at a cost in instructions, and nothing in the table asks for it. Frozen, the chorus adds nothing a sine did not have.

## Cost

Instruction counts under callgrind (`cargo xtask bench --bench chorus`), x86_64 Linux, pinned toolchain, 2026-10-26: 1 s at 48 kHz of a held saw chord (`prepare` and the chord are outside the measured function), with the `engine` bench's 3,616,649 (1 voice) and 37,296,489 (16 voices) subtracted to read the module's own cost. The effect is global, so its cost does not depend on the voice count; the 16-voice case checks that.

| Case | Instructions | Budget (+10 %) | Own cost per frame |
| --- | --- | --- | --- |
| `default_1v` (chorus, 0.5 Hz, 50 %, 7 ms) | 18,379,246 | 20,218,000 | 307.6 |
| `default_16v` | 52,059,086 | 57,265,000 | 307.6 |
| `ensemble_1v` | 49,678,246 | 54,647,000 | 959.6 |
| `flanger_1v` (2 ms, feedback 70 %) | 17,398,246 | 19,139,000 | 287.1 |
| `mix0_1v` | 17,386,246 | 19,125,000 | 286.9 |

A stereo frame is about 300 instructions: two taps, a sine of the sweep each, the cubic read and the blend. The ensemble's six taps and two sweeps take 3.1× that. Instruction counts under-report a memory copy and a subnormal penalty, so the swap measurement above, the 40 s tail test and the FTZ test stay mandatory. A change to `enth-dsp::chorus` re-measures all five in the same commit; the `osc` and `adsr` in every bench patch mean a lane that changes either re-measures them with the `engine` budgets.

## Limits and decisions

1. **The chorus/flanger distinction** is the sweep shape (sine against triangle), settled above; the table is silent.
2. **The minimum tap is one sample**, not zero: the zero-delay-feedback solve reaches it without latency; a through-zero flanger is not a mode.
3. **The level loss is up to 4.8 dB, not 3** (the catalogue's sentence; see [The level](#the-level)).
4. **The ensemble's constants** (ratio 8, share 10 %, taps 120° apart) are this lane's; the table says only "a fixed ratio".
5. **Cubic Lagrange, not a sharper read:** bounded level and no state, at a high-frequency error recorded above.
6. **A saw's peak overshoots by up to about +0.5 dB at its edges** (the cubic's Gibbs-like ringing on a discontinuity); the unit tests bound the peak at ×1.07 of the dry chord's.
7. **`mix` 0 still costs the whole core** (state kept warm); the bypass that removes the cost is the engine's global FX bypass.
8. **A tempo change** applies at the next 32-frame segment boundary (the engine's rule), so a synced sweep changes speed there, never mid-segment.

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/chorus.rs` (lane CAT's F1m table, unchanged) |
| DSP core | `crates/enth-dsp/src/chorus.rs` (also holds the sweep helpers `phaser` shares) |
| Engine module | `crates/enth-engine/src/modules/chorus.rs`; `params.rs` gets one additive `param_index!(ChorusParam, PhaserParam)` |
| Tests | the two files above; `crates/enth-engine/tests/chorus.rs`, `tests/common/modfx.rs` |
| Instruction budget | `crates/enth-engine/benches/chorus.rs` |
| Golden cases | `tests/golden/cases/chorus-*.toml`: `default`, `ensemble`, `flanger`, `flanger-negative`, `synced`, `mix-half`, `manual-flange` (a global LFO on `delay` and an automation of `depth`) |

## Acceptance: how each row is met

| Row (phase-s2.md, every module lane; then lane MOD's own) | How | Where |
| --- | --- | --- |
| Same bits on six targets | `f64`/`f32` with `util::math` and the project's polynomial sine, fixed order, no fused operations; 7 golden cases, equal on x86_64 Linux (`cargo xtask golden`) and on `wasm32-wasip1` under wasmtime 48.0.6 (`cargo xtask golden --wasm`, 2026-10-26); the Windows and macOS targets run in CI | the goldens |
| FTZ-independent | every write into the line is flushed; the render is the same bits under every FTZ/DAZ setting | `the_render_is_the_same_under_every_ftz_mode` |
| Zero subnormals after a 40 s tail | 0 subnormal output samples and 0 in the lines, three modes | `a_40_s_tail_through_the_chorus_has_no_subnormals` |
| Latency 0 | the dry signal is not delayed; the module reports none | `latency_is_zero`, [The tap](#the-tap-lagrange-interpolation-and-the-flangers-zero-delay) |
| Budget within +10 % | five cases ([Cost](#cost)) | `benches/chorus.rs` |
| Two instances keep separate state | two modules in one chain differ from each other and from one | `two_instances_keep_separate_state` |
| `mix` 0 passes the input bit for bit | not written; the render equals the one without the module | `mix_zero_passes_the_input_bit_for_bit` |
| Aliasing measured, bound recorded | [Aliasing](#aliasing) | `a_frozen_sweep_adds_no_aliasing_in_any_mode`, `the_slowest_full_depth_sweep_stays_below_the_recorded_bounds` |
| The swap's copied buffers measured | 32 KiB copied; [Memory, and the plan swap](#memory-and-the-plan-swap) | `process-stats --swap-state` |
| Delay spans exactly `delay ± depth` | [Measured](#measured-the-defining-response) | core `the_impulse_delay_follows_the_sweep_exactly` |
| Flanger notches move as documented | at `(2k + 1)·fs/(2τ)` | core `the_flanger_notches_are_the_odd_multiples_of_half_over_tau` |
| Level bound at ±99 % and at the defaults | [The level](#the-level) | `the_level_is_bounded_at_the_defaults_and_at_full_feedback` |
| Synced rates exact in beats at blocks 1, 64, 100, 4096 across a tempo change | [Measured](#measured-the-defining-response) | `a_synced_sweep_is_exact_in_beats_across_tempo_changes_at_every_host_block` |
| No allocation | `assert_no_alloc` around a chord through the module | `guarded::process_does_not_allocate` |
| Plan swap carries the lines | an instant swap moves them, a crossfade copies them | `the_plan_swap_carries_the_delay_lines`, `take_moves_the_state_and_copy_copies_it` |
| Provenance and a `THIRD_PARTY_NOTICES.md` entry per port | nothing is ported: no entry | the top of this page |

## Sources

- Julius O. Smith III, *Physical Audio Signal Processing* (CCRMA, online book), the *Delay Effects* chapter: "Flanging", "Chorus" and the fractional-delay sections (Lagrange interpolation, its magnitude response and the cubic case). The comb and feedback-comb equations above are from there.
- Mutable Instruments Plaits and Rings (MIT-licensed), for the ensemble's *structure* only: three taps a third of a turn apart on a slow and a fast sweep. The constants here are not theirs, and no code was read into this implementation.
- Research report 05 §3 (modulation effects: one modulated-delay core, the flanger's fixed dry delay) in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`.
- Lane CAT's F1m table and the S2a decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m)); [aliasing.md](aliasing.md) for the measure.
