# Delay (`delay`; `enth-dsp::delay`, `enth-engine::modules::delay`)

- **Status: built (S2a lane DLY, wave 4).** The module replaced the S0d stub; every acceptance row is in [Acceptance](#acceptance-how-each-row-is-met). Kind `delay`, discriminant 16, section `fx`, id prefix `delay` (delay1, delay2…), a time-based effect, rate global. Still hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag; the parameter table is lane CAT's F1m table and was not changed.
- Code: the DSP core is `crates/enth-dsp/src/delay.rs` (the rings, the taps, the loop filters, the soft clip, the time-change crossfade, ping-pong, the idle path, the state copy and move); the module is `crates/enth-engine/src/modules/delay.rs` (the catalogue values and the tempo turned into the core's block parameters, the plan-swap hooks). Tests: white-box in those two files (33 and 11), `crates/enth-engine/tests/delay.rs` (the engine-level checks, 23 tests), the budgets in `crates/enth-engine/benches/delay.rs`, the goldens `tests/golden/cases/delay-*.toml`.
- **Provenance: nothing is ported.** The structure is the textbook delay line with a filtered, saturated feedback path ([Sources](#sources)); every equation is on this page. Research report 05 §4 names Signalsmith's `delay.h` and DaisySP's `delayline.h` (both MIT) as starting points; neither was opened or copied, so there is **no `THIRD_PARTY_NOTICES.md` entry** for this lane (the acceptance row asks for one "for every port"; there is none to make).
- Decisions behind it: research report 05 §4 in `_local/sessions/2026-10-25-s2-depth-and-fast-training/`, lane CAT's table and the F1m decisions ([catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m), the M6 exception for feedback effects), lane TMP's tempo ([tempo](../../crates/enth-engine/src/tempo.rs)). Where this lane settled something the table left open, the section says so; the list is under [Limits and decisions](#limits-and-decisions).

A stereo delay after the voices. The time is a **whole number of samples**, in milliseconds or in beats of the render's tempo up to one bar and never above 2 s; a change of time (a new `time` or `beats`, a tempo change) **crossfades the read position in 20 ms**, so an echo never clicks and an echo of a note always lands on the exact sample the time says. The repeats pass a one-pole high-pass (`low_cut`) and a one-pole low-pass (`high_cut`) every lap and return through a soft clip, so 100 % feedback is bounded; `ping_pong` crosses the laps between the channels. The output is the dry signal, untouched, plus `mix` times the echoes.

## Parameters (lane CAT, F1m)

*Built as specified; the notes the table leaves to lane DLY (the first echo's side in ping-pong, what 100 % really does, the 20 kHz high cut) are under [Limits and decisions](#limits-and-decisions).*

The table lane DLY implements, generated from `crates/enth-patch/src/catalogue/delay.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Time:** `beats` of the tempo with `sync` (exact in beats across tempo changes), else `time`; never above 2 s (the rings are sized from `time`'s maximum at `prepare`), so a synced time past 2 s (a bar below 120 BPM) plays at 2 s. `beats` offers the shared list up to a bar (`DELAY_BEATS`, the same indices as `Beats`).
- **Loop:** each repeat passes `low_cut` and `high_cut` and returns at `feedback` (a soft clip keeps 100 % bounded); `ping_pong` crosses the repeats.
- **The effect's own output is the dry signal plus the echoes**, so `mix` scales the echoes against an unchanged dry signal (100 %: the first echo as loud as the dry, filtered); 0 % is the input bit for bit. Time changes glide or crossfade, never click. This is the one documented exception to the level-bounded rule for feedback effects (catalogue rule 3): the repeats are separate events in time that fall away with `feedback` (a soft clip bounds 100 %), the dry signal passes unchanged, and nothing is normalised.
- **Defaults:** a gentle useful setting: a synced dotted eighth, feedback 35 %, low cut 150 Hz, high cut 6 kHz.

### `delay` — Delay (fx, ids `delay1`, `delay2`…; 8 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `sync` | none | off, on | toggle | on | auto | basic | On: the delay time is beats of the song's tempo. Off: it is time, in milliseconds. |
| `time` | ms | 1ms … 2s | log | 300ms | auto | intermediate | The delay time when tempo sync is off: under 30 ms thickens, 80-150 ms is a slapback, longer is an echo. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar` | enum | eighth_dotted | auto | basic | The delay time when tempo sync is on, as a note division up to a bar: 1/8D is the classic bouncing echo. |
| `feedback` | percent | 0% … 100% | linear | 35% | mod, auto | basic | How much of each echo returns for another: more is more repeats; 100 % repeats for ever. |
| `ping_pong` | none | off, on | toggle | off | auto | intermediate | Bounces the echoes between the left and right channels. |
| `low_cut` | hz | 20Hz … 2kHz | log | 150Hz | mod, auto | intermediate | Removes lows from every repeat, so echoes stay off the bass; each repeat gets thinner. |
| `high_cut` | hz | 1kHz … 20kHz | log | 6kHz | mod, auto | intermediate | Removes highs from every repeat: each echo gets darker, like tape. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How loud the echoes are against the dry sound, which always passes unchanged: 100 % makes the first echo as loud as the dry; 0 % is no echo at all. |

## Signal path

Per frame, stereo (left `L`, right `R`; the ring is `2·N` interleaved samples, `N = round(2 s · sr)` = 96,000 frames at 48 kHz, 768,000 bytes). `x` is the input after sanitising (NaN becomes 0, magnitudes clamp at 10⁶), `D` the delay in samples, `w` the crossfade weight (0 outside a time change):

```text
t_c   = (1 − w)·ring_c[n − D_old] + w·ring_c[n − D_new]       the taps, per channel c
e_c   = LP_high_cut( t_c − LP_low_cut(t_c) )                  the echo: the repeat passes both one-poles
r_c   = soft_clip( feedback · e_c )                           what returns; |r| ≤ 1 at every feedback
in    = x_c + r_c                                             normal: each channel feeds itself
      = ( mono(x) + r_R ,  r_L )                              ping-pong: left → right → left, the input enters left
ring_c[n] = flush(in_c)                                       one write per frame and channel
out_c = x_c + mix · e_c                                       the dry signal is never touched
```

- `LP_f` is the trapezoidal (TPT) one-pole low-pass, `v = (x − s)·G, y = s + v, s = y + v`, `G = g/(1 + g)`, `g = tan(π·f/sr)` (Zavalishin); its −3 dB corner is exactly `f` ([Sources](#sources)). The low cut is the input minus `LP_low_cut` (a one-pole high-pass of the same corner), the high cut is `LP_high_cut`. The corners are held below 0.49 of the sample rate. Each block's coefficients ramp linearly per sample from the previous block's to this block's (the engine already smooths `low_cut` and `high_cut` over 10 ms, so the ramp is only the block-to-block join). One `tan` per filter per 32-frame block, through the pinned `libm`.
- **The echo is taken after the filters**, so the first echo is `e₁ = F(x)` (`F` the pair of filters), the second `e₂ = fb·F(F(x))`, the k-th `e_k = fb^(k−1)·F^k(x)` below the clip: every repeat has passed the pair once more than the one before it. At `mix` 100 % the first echo is as loud as the dry signal, filtered (the table's "first echo as loud as the dry"). The filters are in the loop, not in front of it: lows thin out and highs darken lap by lap.
- **`mix` scales the echoes, not a dry/wet blend**: the dry signal passes unchanged at every setting (the table's wording), so there is no dry gain to lose and `mix` 0 leaves the block alone.
- `feedback` and `mix` arrive as the engine's per-sample smoothed values (the previous segment's value to this one's, 10 ms) and are divided by 100; `ping_pong`, `sync`, `beats` and `time` are discrete and read as the base value.
- **Cost:** about 150 instructions per frame ([Measured](#measured-numbers)), independent of the delay time (the taps are two indexed reads).

## The delay time

`D` is a whole number of samples, so the taps read the ring with **no interpolation**: an echo of an impulse lands on exactly the sample `n₀ + k·D` and the loop adds no interpolation loss to a repeat (a fractional tap would be an all-pass or a low-pass in the feedback path, and its response would change with the time).

| Mode | `D` | Rounding |
| --- | --- | --- |
| `sync` on | `round(num·60·sr / (den·bpm))`, with `num/den` the length of `beats` in quarter notes (`Beats::length`: `(3, 4)` for a dotted eighth) | in `f64`, half away from zero |
| `sync` off | `round(time_ms · sr / 1000)` | in `f64` |
| both | clamped into `1 ..= N` (1 sample to 2 s) | a tempo that makes the time infinite or NaN gives `N` |

- **Exact in beats.** The tempo is the engine's, one value per 32-frame segment (lane TMP): a tempo event applies at the first segment boundary at or after its frame, the same boundary whatever the host's block size. So `D` is a function of the segment only, and **the host's block size cannot change a bit**: the tests render a burst at host blocks of 1, 64, 100 and 4096 across tempo changes and compare every bit of the four renders ([Tests](#tests)).
- **The first echo of a note is at `s₀ + D`** where `s₀` is the first sample of the dry note (note events apply at a segment boundary too, so `s₀` is the boundary at or after the note's frame); the second at `s₀ + 2D`. The tests check both against an integer-only reference (`(2·num·60·sr·bpm_den + den·bpm_num) / (2·den·bpm_num)`, half up) for all 18 divisions at 133.7 BPM and for a hand-picked set of times in ms (1 ms = 48 samples … 2,000 ms = 96,000).
- **A synced time past 2 s plays at 2 s.** A bar is exactly 2 s at 120 BPM (96,000 samples, no clamp) and longer below it: 119.9, 60 and 20 BPM all play at 96,000 (tested). `beats` offers nothing longer than a bar, so the clamp is reached by a bar below 120 BPM, a dotted half (three beats) below 90, a half-note triplet … and so on down the list at lower tempos (a half note below 60, a quarter note below 30).
- The `time` parameter is *not* smoothed (it has no `smoothing` in the catalogue): a new value is a new target at the next segment, and the crossfade below does the gliding.

## A time change moves the read head

A new target `D'` (a tempo change, a new `beats`, a new `time`) does not move the ring; it moves **where the tap reads**. The core fades from the tap at the delay being heard to a second tap at the target:

```text
w(k) = s²(3 − 2s),  s = (k + 1)/L,  L = round(20 ms · sr) = 960 frames,  k = 0 … L−1
tap  = (1 − w)·ring[n − D] + w·ring[n − D']          (w is exactly 1 on the last frame; then D := D')
```

- **No click, no gain bump.** `w` and `1 − w` sum to 1, so the fade adds no gain; the weight is a C¹ ramp; both taps read whole samples, so no sample is invented. The unit test fades a 440 Hz sine's echo from 5,000 to 7,777 samples: the largest sample-to-sample step across the fade is within 15 % of the sine's own (0.029), where a hard switch between the same two settled delays jumps by more than 0.2.
- **A newer target waits.** While a fade runs, a newer target is not started; at the end of the fade, the next block that still differs starts its own fade. A tempo ramp therefore becomes steps of at most one fade (20 ms), each to the latest value; the delay never plays a position that is not a whole sample.
- **What it means for the sound: the ring is not stretched.** After the fade, `echo(n) = F(ring[n − D'])`: a repeat that was written at time `w` is heard at `w + D'`, wherever the old delay would have played it. If the delay *shrinks* (a faster tempo), repeats already in flight arrive early or are skipped; if it *grows*, older material is read again, so a burst can be heard twice (once at the old spacing, once replayed at the new one). The engine test fixes the arithmetic: with a first echo in flight at 240 BPM (`D` = 12,000) and a change to 360 BPM (`D'` = 8,000) the repeat it left in the ring sounds 8,000 samples after it was written (20,000 after the note), and with a change to 120 BPM (`D'` = 24,000) 24,000 after (36,000 after the note); the same holds for a `time` automation (100 → 250 ms).
- **A time change while idle has no fade**: with an empty ring there is nothing to read, and the wake-up snaps to the block's own values (see [Idle](#idle-reset-and-bypass)).
- Why a crossfade and not a slewed time: a slewed time (a read head that glides, as on tape) bends the pitch of the echoes during the glide and needs a fractional tap (see above); the crossfade keeps every echo at its exact time and costs a second tap for 20 ms. Report 05 §4 offers both; the F1m table says "glide or crossfade, never click" and leaves the choice to this lane.

## The loop: filters, feedback, the soft clip and the level bound

- **The soft clip** (on what *returns* only, never on the output) is odd, `+ − ×` only:

  ```text
  u = feedback · e;   a = |u|
  soft_clip(u) = u                              a ≤ 0.5
               = sign(u)·(0.5 + t − t²/2)       0.5 < a < 1.5,   t = a − 0.5
               = sign(u)                        a ≥ 1.5
  ```

  The identity up to 0.5, a C¹ quadratic knee (slope 1 at 0.5, 0 at 1.5) and exactly 1 beyond, so **what returns is at most 1** at every feedback. At feedback up to 50 % and a signal inside full scale the clip is never reached (`feedback·e ≤ 0.5` for `|e| ≤ 1`), so the echoes of ordinary material are exactly linear and the decay rate below is exact.
- **The bound.** The ring never holds more than `|x| + 1` per channel. Both loop filters have an impulse response of ℓ¹ norm at most 2 (`LP`: 1; `x − LP(x)`: at most 2), so `|e| ≤ 2·(|x|max + 1)` and, at `mix` 100 %, **`|out| ≤ 3·|x|max + 2`**. That is proved from the ring bound and holds at every setting; the loop cannot run away at 100 %, however long it runs.
- **Measured at feedback 100 %, `mix` 100 %** (a held three-note saw chord of dry peak 1.49 through the engine, 40 s; the filters wide open, 20 Hz and 20 kHz, the worst case for gain): peak 2.93 over 40 s with a 1 ms loop, 2.89 with 10 ms and ping-pong, 2.10 with 300 ms and 1.59 with 2 s and ping-pong, against the bound of 6.47. Noise at full scale through the core at 100 % with every filter setting at its extremes, with and without ping-pong, 12 s each, peaks at about 2 (`enth_dsp::delay` tests).
- **"100 % repeats for ever" is true of the loop, and only approximately of the sound.** The soft clip does not eat a repeat (a burst still comes back after ten laps at 100 %, tested), but each lap passes the two one-poles, which cost a few thousandths of the level at mid frequencies and more near the corners. A loop of one pass every 300 ms or more sustains for the 40 s tested (peak 0.72 at 20–40 s with 300 ms, 0.94 with 2 s ping-pong); a 1 ms loop (a thousand laps a second) loses its sound within seconds even with the filters wide open (0.0003 at 20–40 s), a 10 ms loop is down to 0.2. At the defaults (150 Hz and 6 kHz) the lows and highs fall away lap by lap by design. The table's sentence "100 % repeats for ever" is the intent; this is the measured behaviour, reported to the coordinator.
- **The decay rate.** Below the clip each repeat is `feedback × G(f)` of the one before, `G` the product of the two filters' gains at the signal's frequency. Measured through the engine (a 100 ms burst of a 440 Hz sine, 300 ms apart, the filters wide open, steady part of each repeat; `G` = 0.9990): ratios of successive repeats 0.1998 at 20 % (0.1999 … 0.2000 per repeat), 0.4995 at 50 % and 0.7493 at 75 %, always at or below `feedback` and within 0.4 % of it. In the core, with the closed-form response of the two filters, the echoes of a 1 kHz burst decay at `feedback × G` to 0.05 dB for three feedbacks, normal and ping-pong; the filters' response agrees with the closed form to 0.01 dB at seven corner/frequency combinations.

## Ping-pong

With `ping_pong` on, the input is mixed to mono and enters the **left** ring; what returns from each channel goes into the *other* channel's ring:

```text
ring_L[n] = mono(x)[n] + r_R[n]          ring_R[n] = r_L[n]          mono(x) = (x_L + x_R)/2
```

So echo 1 sounds on the left alone, echo 2 on the right alone, echo 3 on the left, and so on: the other channel is silent in each window (exact zeros, apart from the other channel's filters still ringing at about 10⁻¹² of the echo before; tested), and the energy falls by `feedback²` per lap across the channels. The first echo is on the left (a decision: the table says "bounces the echoes between the channels" and does not say which side leads). Without ping-pong a mono input gives identical channels.

Switching `ping_pong` on a running echo blends the two topologies over 10 ms (a per-block step of the blend `c`, `in = (1 − c)·normal + c·crossed`), so it never clicks (tested in the core).

## `mix`, the dry path and latency

- **Latency 0.** The dry signal is the input, untouched; only the echoes are delayed. The tests find the first sample at which the render through the delay differs from the dry render: it is `s₀ + D`, never earlier.
- **`mix` 0 is the input bit for bit.** A block whose `mix` is exactly 0 at both ends is left alone (no `+ 0·e`, which could turn a −0 into +0): the engine test renders a held chord through a delay at mix 0 with feedback 100 % and ping-pong, at the corners of every other setting, at two host blocks, and compares every bit with the render without the delay. The loop keeps recording at mix 0, so raising `mix` brings in echoes of what was played meanwhile. After an automation takes `mix` from 100 to 0, the output is the dry signal bit for bit again once the 10 ms ramp is over, with a full ring (tested).
- Between 0 and 100 the echoes are scaled by the smoothed `mix`, per sample.

## Idle, reset and bypass

- **Idle** (so a finished tail costs nothing): once every ring slot has been overwritten by exact zeros (`N` frames of zero writes in a row), no time-change fade is running and both filter states are exactly 0, the core is idle. A block of exact zeros is then skipped (the output stays zeros, nothing changes) and the engine skips the module while its input is exact zeros ([`GlobalProcessor::is_silent`]). The first block with a sample **wakes** the core with the delay, the filter coefficients and the ping-pong blend set straight to that block's values, so **skipping is bit-identical to running** (tested at the module level against a core that never skipped).
- Every write into the ring and every filter state is passed through `flush` (threshold 10⁻¹⁵), so a decaying tail reaches exact zero in code, not in the FPU: **no subnormal ever reaches the arithmetic** whatever FTZ/DAZ is set to, and the tail goes idle.
- **Bypass.** `delay` is `time_based` in the catalogue: the plugin's FX bypass fades it out, and `reset` clears the rings (skipped when they already are zeros) before it comes back, so no old echoes return.

## Plan swaps

A swap that adds or removes no module is an **instant swap**: the new plan's delay *takes* the old one's state, and the two rings are exchanged, an O(1) move (the old plan, holding this delay's fresh empty rings, is dropped on the main thread). A swap that adds or removes a module or an audible route **crossfades** for 256 frames: both plans run and the new plan's delay gets a **copy** of the old one's rings and state (`copy_state_from`, 768,000 bytes at 48 kHz; free when the old delay is idle). The copy is the one place the module does more than a block's worth of work, and it is measured against the real-time deadline ([Measured](#measured-numbers)).

Tested through the engine: a same-patch instant swap, sent at five segments before, during and after the first echo, changes no bit of the render; a crossfading swap (an unrouted LFO added or removed, at three points around the first echo and its repeat) leaves the render identical to the unswapped one (difference exactly 0, the two plans' equal outputs crossfading to themselves), and the control, a swap to a delay under another id (a module with no state to carry), loses the echoes (difference above 0.05).

## Real time and determinism

- **Allocation.** `Delay::new` (called from `compile`) allocates the one `Box<[f32]>` of `2·round(2 s·sr)` samples; nothing else allocates. `process` runs under `assert_no_alloc` in the tests with tempo, `time`, `sync`, `ping_pong` and `feedback` events and a host block of 100 frames, and so do an instant and a crossfading plan swap (the copy and the exchange allocate nothing).
- **Bits.** Only `+ − × ÷`, comparisons, `abs`/`copysign`/`midpoint`, and one pinned-`libm` `tan` per filter per block; no `mul_add`; the channels and the laps are summed in a fixed order; the integer arithmetic is `u32`/`usize` with the segment grid. The tests render the tail patches under FTZ off, FTZ only and FTZ+DAZ (x86-64; FZ on aarch64) with tempo changes inside and compare every bit. The goldens are checked on the host and `wasm32-wasip1`; CI checks the other four targets.
- **Input hygiene.** A NaN input becomes 0 and anything beyond ±10⁶ is clamped before it enters the loop (one such sample would live in the ring for ever at 100 %).

## Measured numbers

All measured 2026-10-27 on the dev machine (x86_64 Linux, the pinned toolchain 1.99.1).

### Instruction budget

Each case renders 1 s at 48 kHz of a held saw chord through one `delay` and the engine skeleton. The module's own cost is the case minus the `engine` bench with the same voices (3,612,149 at 1 voice, 37,291,989 at 16). A global module costs the same whatever the voice count; the 1- and 16-voice rows differ by the engine's own work, not the delay's. The budget is the measured count plus 10 %, rounded up to 1,000.

| Case | Setting | Instructions | Budget | Module's own, per frame |
| --- | --- | --- | --- | --- |
| `default_1v` | the defaults (synced dotted eighth, 35 %, 150 Hz / 6 kHz, mix 100) | 10,834,090 | 11,918,000 | 150 |
| `default_16v` | the same, 16 voices | 44,827,038 | 49,310,000 | 157 |
| `ms_16v` | `sync` off, 300 ms | 44,725,376 | 49,198,000 | 155 |
| `pingpong_16v` | ping-pong | 45,326,225 | 49,859,000 | 167 |
| `fb100_16v` | feedback 100 %, filters wide open (the clip engaged) | 45,073,429 | 49,581,000 | 162 |
| `mix0_16v` | `mix` 0 (the loop runs, the output is left alone) | 44,299,038 | 48,729,000 | 146 |
| `retime_16v` | the tempo alternates 120 / 140 BPM every 960 frames: a time-change crossfade runs for the whole second | 46,453,827 | 51,100,000 | 191 |

About 150 instructions per stereo frame, 7.2 million per second of audio, inside report 05's 80–200 estimate. Instruction counts under-report subnormal penalties; the tail tests are the check for those.

### Echo times, exactness and bits

| Check | Result |
| --- | --- |
| First and second echo of a burst, at host blocks 1, 64, 100 and 4096, with the tempo changing before the note, and while the first echo is in flight (a faster and a slower tempo, the change at an odd frame) | exactly `D` and `D + D'` samples after the burst; the four renders are the same bits |
| All 18 divisions at 133.7 BPM (a fractional tempo), 100-sample host blocks | exactly the integer reference |
| 9 times from 1 to 2,000 ms | exactly `round(ms·48)` |
| A bar at 120 BPM / 119.9 / 60 / 20 BPM | 96,000 samples (2 s) for all four (120 is exact, the others clamp) |
| `time` automated 100 → 250 ms with the first echo in flight | repeat sounds 12,000 samples after it was written, at four host blocks |
| Latency | the render through the delay equals the dry render until `s₀ + D` |
| `mix` 0, four extreme settings, two host blocks | every bit equal to the render without the delay |

### Level at 100 % feedback

A 40 s render of the held chord (dry peak 1.4906; the proven bound `3·peak + 2` is 6.4719):

| Loop | Peak over 40 s | Peak 20–40 s |
| --- | --- | --- |
| 1 ms | 2.930 | 0.0003 |
| 10 ms, ping-pong | 2.893 | 0.200 |
| 300 ms | 2.095 | 0.719 |
| 2 s, ping-pong | 1.586 | 0.938 |

The filters were wide open (20 Hz, 20 kHz). No sample is non-finite or subnormal in any of them, and the loop never grows (the last 20 s are below 1.5× the 2–10 s peak plus 0.1 in the test).

### Decay of the repeats

Through the engine: a 440 Hz burst, repeats 300 ms apart, the filters wide open (`G` = 0.9990).

| Feedback | Ratio of successive repeats (4 pairs) | `feedback × 0.9990` |
| --- | --- | --- |
| 20 % | 0.19980, 0.19979, 0.19977, 0.19980 | 0.19980 |
| 50 % | 0.49951, 0.49947, 0.49943, 0.49950 | 0.49950 |
| 75 % | 0.74866, 0.74927, 0.74925, 0.74937 | 0.74925 |

### Aliasing

Measured spectrally with `enth_dsp::alias` and report 02's measure ([aliasing.md](aliasing.md)); the sweep is an `osc` sine at keys 45, 57, 69, 81, 93 and 105 (110 Hz … 3.52 kHz).

The delay is linear below the soft clip, so a sine through it stays a sine: **the linear loop measures the measure's own floor.** The loop's one nonlinearity is the soft clip, at the base rate (no oversampling, no ADAA), so it is measured where it matters: feedback 100 % with the loop time a whole number of periods of the sine, so the repeats add up in phase until the clip limits them and what returns is a clipped sine.

| Case | 110 Hz | 220 Hz | 440 Hz | 880 Hz | 1.76 kHz | 3.52 kHz | Worst | Recorded bound (worst + 1.5 dB) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Linear: 20 ms, 35 %, default filters | −103.5 | −97.7 | −93.3 | −95.5 | −92.6 | −118.0 | −92.6 | −91.0 |
| Linear: the same, filters wide open | −103.5 | −97.7 | −93.3 | −95.5 | −92.6 | −118.4 | −92.6 | −91.0 |
| Saturating: 100 %, resonant, filters wide open | −58.4 | −77.0 | −71.1 | −57.8 | −50.8 | −46.4 | −46.4 | −44.9 |
| Saturating: 100 %, resonant, default filters | −55.2 | −73.6 | −91.0 | −81.8 | −73.6 | −70.8 | −55.2 | −53.7 |
| Saturating: 100 %, resonant, ping-pong, wide open | −48.6 | −59.9 | −52.1 | −56.4 | −53.5 | −53.3 | −48.6 | −47.1 |

(dB; lower is cleaner.) The bounds are asserted in `tests/delay.rs` (`LINEAR_BOUND_DB`, `SATURATED_*_BOUND_DB`). **Reading:** a delay at any ordinary setting adds no aliasing. A delay driven into its clip by a loop that rings at 100 % aliases like a polyBLEP saw at the top of the keyboard (osc.md: −40.6 dB at key 69, −28.8 at 105): the default 6 kHz high cut takes about 9 dB off the worst point, because it removes the harmonics that would fold, and ping-pong is no worse than the plain loop. The saturated check is a worst case that no preset needs (a held tone in a loop tuned to it); the soft clip is C¹, so its harmonics fall steeply. Oversampling or ADAA of the clip is not built: the clip acts only on the *return* path, and only when `feedback × echo` passes 0.5 (loud material at a high feedback, or a resonant loop), and the loop's high cut sits in front of it. (A sweep at the default *synced* time, where the first echo's onset falls inside the 32,768-sample window, measures −17 … −36 dB: that is the echo's attack transient entering the window, not aliasing, and it is not used.)

### The plan swap's copy

The crossfading swap is the one block where the delay does more than a block's worth of work: the new plan's delay gets a copy of the old one's rings, 768,000 bytes at 48 kHz (the reverb's six lines, 1.6 MB, are copied in the same block when a G1 patch holds both). It is measured with P1b's harness (`rtbench`, the research-03 tool, copied to the lane's scratch with its dependencies pointed at this tree and two patches added: G1 plus a `delay1` at its defaults, and G1 without the reverb plus the delay): one engine on one thread, 16 held voices, **paced 64-frame blocks** (a 1,333 µs deadline) under rtkit `SCHED_RR` 10, a plan sent every 250 ms for 30 s (119 swaps), the swap block being the block that contains the swap; and the deterministic single-thread mode (300 swaps, 40 normal blocks between). The unpaced single-thread numbers are the least noisy; the paced ones include cold caches and clock changes after each sleep.

**Conditions.** The dev machine was shared: the load average was 3.2–3.8 in round 1 and 3.2–12 in round 2, because other lanes were building and testing (their timing runs hold the same `flock -x`, but not everything they run does). Wall-clock numbers are therefore upper bounds, as in P1b's report 03 (load 7–26).

| Swap block, µs | p50 | p99 | max | Notes |
| --- | --- | --- | --- | --- |
| **Instant swap**, G1 + delay (the move), paced | 96 | **316** | 327 | P1b's reference (the copy, before the move): p99 797 / max 997; round 1 of this lane: p99 143 / max 152 |
| **Crossfading swap**, G1 alone (reverb copy), paced | 271 | 971 | 1,025 | same session; round 1: p99 966 / max 983 (P1b: 1,052 / 1,057 at load 13–17) |
| **Crossfading swap**, G1 + delay, paced, three runs (a, b, c) | 379 / 492 / 373 | **1,200 / 1,304 / 1,235** | 1,447 / 1,307 / 1,298 | deadline 1,333; 1, 2 and 0 blocks of 22,500 whose wake-up lateness plus run time passed the period |
| Crossfading swap, G1 + delay, single thread (300 swaps) | 266 | 512 | 532 | G1 alone: 179 / 334 / 353. The delay adds 88 µs at p50 and 178 µs at p99 |
| Crossfading swap, the delay alone (G1 without the reverb), paced | 137–171 | 327–511 | 382–534 | the delay's share of the copy |
| Crossfading swap, the delay alone, single thread | 108 | 189 | 197 | |
| Crossfade blocks (the 4 blocks after the swap), G1 + delay, paced | 143–161 | 258–587 | 335–604 | |
| A 1.6 MB `memcpy`, for scale (hot / cold) | 144 / 212 | 286 / 403 | 346 / 2,102 | the delay's 768,000 bytes are about 0.47 of it |

**Reading.**

- **The p99 of the crossfading swap block is under the 1,333 µs deadline in every run** (90–98 % of it in the three paced runs on a loaded machine, 38 % single-threaded), and the instant swap, which is every edit that adds or removes no module, is far under: the delay moves its rings (O(1)), so it adds nothing to that block (p99 316 µs paced with 16 voices, 143 µs in the quieter round). The paced **max** passed the deadline in one of three runs (1,447 µs) where the same session's G1-alone runs peaked at 1,025 µs: a block that large is a copy running into a cold cache or a clock change, the failure P1b found at load 13–17, and about one block in 120 swaps.
- **The delay adds about 90 µs at p50 and 180–330 µs at p99 to the crossfading swap block of a patch that already copies a reverb.** The cost is memory bandwidth (768,000 bytes). Without the reverb the whole block is 137–171 µs at p50, a third of the 25 % guard's 333 µs.
- **A page-fault bug was found and fixed here.** The first measured round had a swap-block max of 246 page faults and a paced p99 of 2,465 µs: `vec![0.0; n]` hands out untouched pages and the audio thread's copy was the first write to each. `Delay::new` now writes one zero to every 4 KB page on the compiling thread (+25 µs of `compile` at p50, measured: 30.6 → 55.6 µs without the reverb), and the page faults per swap block are 0 (max) in round 2. The reverb does not show it (0 faults) because its `reset` clears all six lines with a full write; the delay's `reset` skips the clear when the ring is already zero, so the touch had to move to the constructor.
- **Options if the margin is wanted back** (not taken; none is needed for the deadline, and each changes a contract or a design decision outside this lane): (1) copy only the part of the ring that the next reads can reach, `max(delay, fade target)` frames, and leave the older frames zero. It cuts the default dotted eighth (15,429 frames) to a sixth of the copy, but it breaks `copy_state_from`'s "identical output for identical input" for a later time increase, which would replay silence instead of old echoes; (2) P1b's pre-effects crossfade prototype (blend the two voice sums before one effect chain), which removes both the reverb's and the delay's copy and the second chain's cost, at the price of a different crossfade definition (P1b: "do not reopen for P1b"); (3) a `copy_state_from` that can run in pieces across the crossfade's eight segments, which needs a change of the module trait. The coordinator decides.

## Tests

- `crates/enth-dsp/src/delay.rs` (33): the delay is the rounded time in samples (synced, ms, the clamp); the rings are all the memory (768,000 bytes at 48 kHz); the soft clip is the identity to the knee and bounded after; the filter coefficient gives the −3 dB corner; echoes of an impulse land on exact samples and nothing comes before the first; ping-pong alternates and starts left, and takes the mono sum of a stereo input; the echo has the closed-form response of the two filters (to 0.01 dB at seven settings); feedback decays at the set rate (to 0.05 dB, normal and ping-pong); 100 % stays bounded for noise at every filter extreme and keeps repeating a burst; `mix` 0 leaves the block alone and the line keeps recording; `mix` ramps; a time change crossfades without a click, lasts the fade and ends on the new delay exactly, a newer target waits and a fade to the same delay never starts; toggling ping-pong does not click; two instances keep separate state; a copied or taken state runs on as the original; a taken ring is moved not copied; it goes idle when the tail has died and skipping equals running; a reset clears the ring; the tail leaves no subnormals; a silent run never wakes the core.
- `crates/enth-engine/src/modules/delay.rs` (11): the defaults are a synced dotted eighth with three or four repeats; `sync` off plays milliseconds whatever the tempo; every synced item follows the tempo and a bar clamps below 120 BPM; the list is the shared list up to a bar; percent and Hz reach the core in its units and the ramp runs from `prev` to `cur`; the whole catalogue range is inside the core's; an impulse echoes on the exact sample for the tempo in force; a tempo change sets the delay of everything still in the ring; skipping an idle module equals running it; copy and take carry the whole state; it allocates 2 s of stereo ring from the sample rate alone.
- `crates/enth-engine/tests/delay.rs` (23): the table above — exact echoes at four host blocks across tempo changes, all 18 divisions, milliseconds, the 2 s clamp, the read head after a `time` change, the decay, ping-pong, `mix` 0 (and `mix` brought to 0 with a full ring), two instances in a chain, the 100 % bound for 40 s, the 40 s tails with zero subnormals and exact silence at the end, the render under every FTZ mode, `process` under the no-allocation guard, the instant and crossfading swaps and their control, the aliasing bounds and the check that the resonant sweep really drives the clip.
- Goldens: `tests/golden/cases/delay-*.toml`, see below.

### Golden cases

The thirteen cases are in `tests/golden/cases/delay-*.toml` with their hashes in `tests/golden/expected/` (made with `cargo xtask golden --update`; nothing that existed changed). Each is a saw (reset phase) through a gate or pluck envelope at 48 kHz and the harness tempo of 140 BPM (a quarter note is 20,571 samples; a dotted eighth 15,429).

| Case | What it guards |
| --- | --- |
| `delay-default` | The catalogue defaults on a 110 Hz saw pluck: the dotted eighth, feedback 35 %, 150 Hz / 6 kHz in the loop, mix 100 %; 2 s |
| `delay-slapback` | `sync` off, 110 ms (5,280 samples), feedback 20 %, mix 60 % |
| `delay-pingpong` | Ping-pong on, an eighth, feedback 55 %: the echoes alternate, the first on the left; 3 s |
| `delay-dub` | A two-note chord, a quarter, feedback 80 %, low cut 300 Hz, high cut 2.5 kHz, mix 70 %: the repeats thin and darken; 5 s |
| `delay-fb100` | Feedback 100 %, filters wide open, 120 ms, a held saw: the soft clip bounds the loop; 3 s |
| `delay-mix0` | `mix` 0 with feedback 100 % and ping-pong on a chord: the dry signal, bit for bit |
| `delay-time-auto` | `delay1.time` automated 100 → 250 ms while the first echo is in flight: the 20 ms crossfade and the read head |
| `delay-beats-auto` | `delay1.beats` changed from a dotted eighth to a quarter at 0.45 s with echoes in flight |
| `delay-chain` | Two delays in one chain (90 ms, then a synced sixteenth triplet with ping-pong): each instance keeps its own state, the order matters |
| `delay-short` | 1 ms (48 samples, a comb), feedback 60 %, filters wide open, mix 50 % |
| `delay-long` | 2,000 ms, the ring length: the first echo at 2 s exactly; 5 s |
| `delay-bar` | A bar (82,286 samples at 140 BPM), feedback 50 %; 5 s |
| `delay-feedback-auto` | `feedback` 0 → 90 % at 0.2 s and `mix` → 40 % at 1.5 s: the per-sample ramps |

The golden harness renders at the default 140 BPM and has no tempo field, so the tempo-change exactness is covered by the engine tests above, not by a golden.

## Limits and decisions

| Decision | Why |
| --- | --- |
| The time is a whole number of samples; no fractional tap | Echoes land on exact samples (the acceptance row), the loop has no interpolation loss, and a time change cannot colour the sound. A fractional delay (Lagrange, all-pass) is the right tool for a *modulated* delay: lane MOD's chorus builds its own. |
| A time change crossfades in 20 ms; it does not glide | See [A time change moves the read head](#a-time-change-moves-the-read-head): no pitch bend, every echo at its exact time, a second tap for 20 ms. The cost of the choice is that a continuously sweeping tempo makes the delay move in steps of one fade. |
| The first echo is on the left with ping-pong, from the mono sum | The table does not say which side leads; the input enters one ring, like a classic ping-pong. |
| The loop filters sit after the tap, in the loop, and the soft clip only on the return | The echo and the repeats are filtered alike (a repeat is a filtered copy of the echo before it); the output is never clipped, so `mix` 100 % and a quiet loop are exactly linear. |
| The soft clip is the identity to 0.5 and ±1 beyond 1.5, not `tanh` | `+ − ×` only (no transcendental on the sample path), exact below the knee, a hard ceiling of 1 for the proof of the bound. |
| `mix` 0 skips the output add | `x + 0·e` could turn a −0 into +0; the acceptance is the input *bit for bit*. |
| Idle and wake-snap | A finished tail costs nothing and skipping equals running; a time change on an empty ring needs no fade. |
| Instant swap moves the ring; only the crossfading swap copies | O(1) for the common edit; the copy is paid in the rare structural swap and is measured. |
| Input beyond ±10⁶ is clamped, NaN is 0 | One such sample would live in the ring for ever at 100 %. |

**Where this contradicts or sharpens the table** (reported to the coordinator, not changed):

- "100 % repeats for ever": the loop is bounded and the soft clip never eats a repeat, but the two one-poles cost a little every lap, so a *very short* loop at 100 % dies in seconds even with the filters wide open (see [The loop](#the-loop-filters-feedback-the-soft-clip-and-the-level-bound)). Not a defect of the module; the sentence is the intent.
- `high_cut` at its 20 kHz maximum is a bilinear one-pole low-pass, which has zero gain at Nyquist and −0.4 dB at 10 kHz, so it is not fully "open"; `low_cut` at 20 Hz still takes a few thousandths off everything below a few hundred Hz each lap. A true bypass of either filter would need a flag the catalogue does not have.
- The table's `time` has a log scale from 1 ms: 1 ms is 48 samples at 48 kHz, so the shortest delays are a comb, not an echo, as the table's own "under 30 ms thickens" says.

## Sources

- J. O. Smith III, *Physical Audio Signal Processing*, CCRMA online: the delay line, the feedback comb filter and the stability of a loop whose gain is below 1 ("Feedback Comb Filters").
- V. Zavalishin, *The Art of VA Filter Design* rev. 2.1.0 (Native Instruments): the trapezoidal (TPT) one-pole low-pass, `G = g/(1 + g)`, and its use as the building block of a high-pass (the input minus the low-pass).
- T. I. Laakso, V. Välimäki, M. Karjalainen, U. K. Laine, "Splitting the unit delay", *IEEE Signal Processing Magazine* 13(1), 1996: why a fractional tap changes the loop's response (the reason this module does not use one).
- Research report 05 §4 (delay: two rings sized at `prepare`, ping-pong by cross-feed, LP/HP and a saturator in the loop, time change by a two-head crossfade or a slewed time; tempo from lane TMP), and `docs/dsp/aliasing.md` (the spectral measure).
- Not used: the Signalsmith and DaisySP delay sources the report lists (MIT) were not opened.

## Acceptance (how each row is met)

| Row | How |
| --- | --- |
| Same bits on six targets | Every operation is `+ − × ÷`, a comparison or a pinned-`libm` `tan`; the goldens `delay-*` are checked on the host (`cargo xtask golden`) and `wasm32-wasip1` (`--wasm`), CI checks the rest |
| FTZ-independent; zero subnormals after a 40 s tail | `tests/delay.rs`: three loops rendered under FTZ off / FTZ only / FTZ+DAZ with tempo changes, every bit equal; 40 s tails with zero subnormal output samples and engine state, and the last 5 s exactly 0 |
| Latency 0 | The dry signal is never delayed; the render through the delay equals the dry render until `s₀ + D` |
| A budget of the measured count plus 10 % | [Instruction budget](#instruction-budget) |
| Two instances keep separate state | `delay1` and `delay2` in a chain (engine test) and two cores (unit test) |
| `mix` 0 passes the input bit for bit | [`mix`, the dry path and latency](#mix-the-dry-path-and-latency) |
| Aliasing measured spectrally, bound recorded | [Aliasing](#aliasing) and `tests/delay.rs` |
| No allocation on the audio thread | `process_does_not_allocate` under `assert_no_alloc` |
| A delay time exact in beats at any host block across a tempo change | `echoes_land_on_the_exact_sample_at_every_host_block_size` (blocks 1, 64, 100, 4096; tempo changes before the note and in flight) |
| The swap's copied buffers measured against the deadline | [The plan swap's copy](#the-plan-swaps-copy) |
| Echoes: feedback decays at the documented rate; ping-pong alternates | [Decay](#decay-of-the-repeats), `ping_pong_alternates_the_channels` |
| The 2 s clamp for a synced time that is too long; the level bound at 100 %; how a time change moves the read head | `a_synced_time_past_two_seconds_plays_at_two_seconds`; `feedback_100_percent_stays_bounded_for_40_seconds`; [A time change moves the read head](#a-time-change-moves-the-read-head) and `echoes_land_…` (in-flight cases) |
| Golden cases, this page, a `THIRD_PARTY_NOTICES.md` entry only for a port | The cases above; this page; none (no port) |
