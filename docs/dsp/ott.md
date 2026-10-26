# OTT (`ott`; `enth-dsp::ott`, `enth-engine::modules::ott`)

- **Status: implemented (S2a lane OTT, 2026-10-26)**, to lane CAT's F1m table below. The table is unchanged; the verification of its low-confidence defaults is [Defaults evidence](#defaults-evidence), with no change proposed.
- Kind `ott`, discriminant 9, section `fx`, id prefix `ott` (ott1, ott2…), rate global. Fixed and persisted from F0m; **still hidden** from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag (`ModuleSpec::hidden` is untouched).
- Three-band upward and downward compression after the voices (the "OTT" sound of bass music), on the compressor's detector: a Linkwitz–Riley 4th-order split, per band a downward curve above one threshold and an upward curve below another, `depth` as intensity, `time`, per-band gains, `input`, `output`, `mix`. Stereo, **latency 0**, no look-ahead, no heap memory.
- Sources: the Giannoulis–Massberg–Reiss static curve and the JOS amplitude follower that `docs/dsp/compressor.md` already implements (reused, not copied: `StaticCurve`, `one_pole_gain` are the compressor's); Linkwitz, "Active crossover networks for noncoincident drivers", JAES 24(1), 1976 (the 4th-order alignment); Zavalishin, *The Art of VA Filter Design* (the topology-preserving-transform state-variable filter); Ableton Live 12 manual, 29.26 "Multiband Dynamics" (the structure and the meaning of Amount and Time); research report 05 §4 (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`). **No code is ported** (nothing to add to `THIRD_PARTY_NOTICES.md`): the band constants are this lane's own, see [Constants](#the-bands-constants-and-where-they-came-from).

## Parameters (lane CAT, F1m)

The table lane OTT implements, generated from `crates/enth-patch/src/catalogue/ott.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `input` gain; a Linkwitz–Riley split at `low_split` and `high_split`; per band downward compression above a threshold and upward below it (a gain cap and a floor, so silence is not lifted), scaled by `downward` and `upward` and both by `depth`; `time` scales every band's attack and release; `low_gain`, `mid_gain`, `high_gain`; the sum; `output`; `mix` against the input.
- **The bands' thresholds, ratios and times are lane OTT's constants**, not parameters: [what they are and where they came from](#the-bands-constants-and-where-they-came-from); no public primary source publishes the preset's values.
- **`depth` is intensity, not dry/wet** (Ableton's Amount), because a dry blend of a band-split signal notches the crossovers; `mix` is still there for the S2 rule. Defaults: a gentle useful setting at `depth` 30 %; the full effect is 100 %.

### `ott` — OTT (fx, ids `ott1`, `ott2`…; 12 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `depth` | percent | 0% … 100% | linear | 30% | mod, auto | basic | How hard all three bands are squashed up and down: 0 % leaves the levels alone, 100 % is the full, dense OTT sound. |
| `time` | percent | 10% … 1000% | log | 100% | auto | intermediate | Scales every band's attack and release: 100 % is the reference OTT timing, lower is faster and grittier, higher is smoother. |
| `upward` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much quiet parts of each band are lifted (upward compression): brings out tails, noise and detail. |
| `downward` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much loud parts of each band are pushed down (downward compression): evens out peaks. |
| `input` | db | -24dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain before the bands: more input drives more of the signal above the thresholds. |
| `output` | db | -24dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain after the bands, to match the level before and after. |
| `low_split` | hz | 40Hz … 500Hz | log | 88.3Hz | auto | advanced | The crossover between the low and mid bands. |
| `high_split` | hz | 500Hz … 10kHz | log | 2.5kHz | auto | advanced | The crossover between the mid and high bands. |
| `low_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | intermediate | Output level of the low band after compression. |
| `mid_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | intermediate | Output level of the mid band after compression. |
| `high_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | intermediate | Output level of the high band after compression. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged. Prefer depth for a lighter OTT: a dry blend notches the crossovers. |

## The signal path

Per stereo frame, with the parameters as in the table (`a_down`, `a_up`, the gains and `mix` ramp linearly over each 32-frame segment from the previous segment's value to this one's, so a settled value is exact):

```text
x      = sane(in) · input                           NaN/inf → 0, |x| ≤ 1e6
low, mid, high = LR4 split of x at low_split, high_split         zero latency; the sum is an allpass
per band b (stereo-linked: the louder channel drives both gains):
  env   += k·(max(|L_b|, |R_b|) − env)               the compressor's amplitude follower (k from attack / release)
  lvl    = 20·log10(max(env, 1e-10))                 dBFS
  G      = a_down · down_b(lvl) + a_up · up_b(lvl)   dB; a_down = depth·downward, a_up = depth·upward (0–1)
  gs    += k_smooth·(G − gs)                         one-pole gain smoother, time constant attack/2 (the compressor's)
  out_b  = band_b · 10^(gs/20) · band_gain_b         the onset guard may cut the lift for this sample (below)
wet    = (low' + mid + high) · output                low' = the low band through an allpass at high_split
out    = in·(1 − mix) + wet·mix                      mix 0 returns the input, bit for bit
```

`gs` is kept as two smoothers (downward and upward) so the guard can reset the lift alone; their sum is the one-pole of `G`.

### The split

Linkwitz–Riley 4th order: two cascaded Butterworth (`k = √2`) state-variable filters per branch, topology-preserving transform, in `f64`. With `A(s) = (s² − √2·s + 1)/(s² + √2·s + 1)`, `LP4 + HP4 = A` exactly, so the high branch is built as `HP4 = A(x) − LP4`, where `A(x) = x − 2√2·BP(x)` comes from the same state-variable filter. Three bands from two crossovers: `rest = A₁(x) − low`, `mid = LP4₂(rest)`, `high = A₂(rest) − mid`, and the low band goes through `A₂`, so with unity band gains the sum is `A₁(A₂(x))`: **flat in magnitude, with the phase of two second-order allpasses**. The response of each band is the textbook one, `u = tan(πf/fs)/tan(πf_c/fs)`, `s = ju`, exact for the transform (not a bilinear approximation of an analogue prototype).

### Intensity, time and the plain gains

- `depth` and the two knobs scale the **dB gain of each curve**: `a_down = depth·downward`, `a_up = depth·upward`. Depth 0 is every band at 0 dB (the band split alone), depth 100 % with both knobs at 100 % is the full curves, and anything between has the same thresholds with a proportionally smaller gain (an effective ratio of 1 at 0: Ableton's Amount, which the manual describes as the intensity of the compression or expansion in all bands, with no effect at 0 %). The two knobs pick which curve is used.
- `time` multiplies every attack and release (and so the gain smoother, which is attack/2): 10 % is ten times faster, 1,000 % ten times slower. It is not smoothed and not modulatable (the coefficients change at the next segment).
- `input` before the split, `output` after the sum, `low_gain`, `mid_gain`, `high_gain` after each band's compression: plain dB gains, ramped over the segment.
- `mix` blends the **input** (not a band-split dry) with the wet sum. A dry blend of a band-split signal notches the crossovers, which is why `depth` is the intensity knob ([measured below](#the-notches-a-dry-blend-would-cut)).

### The curves

For one band at `level` dBFS (the follower's reading), with the compressor's quadratic soft knee (Giannoulis–Massberg–Reiss 2012, eq. 4, evaluated in `f64` and rounded once to `f32`):

- **Downward:** `down(L)` = the compressor's `StaticCurve(T_d, R_d, W)`, ≤ 0 above `T_d − W/2`, 0 below.
- **Upward:** the same curve mirrored about the level axis, `up(L) = −StaticCurve(−T_u, R_u, W).gain_db(−L)`, ≥ 0 below `T_u + W/2`, 0 above; **capped at +30 dB** and **faded linearly to 0 between −84 and −96 dBFS**, so silence, dither and the ends of a tail are not lifted into noise.
- The knee width is `W = 12` dB for every threshold.

The ratio `R:1` means the output moves 1 dB per `R` dB of input beyond the threshold (the usual definition); an upward ratio of 66.7:1 therefore pins a quiet signal almost to its threshold (a −80 dBFS sine comes out at −50 dBFS: the cap), and a downward ratio of 1000:1 is the limiter.

### The bands' constants, and where they came from

| Band | Down threshold | Down ratio | Up threshold | Up ratio | Attack | Release |
| --- | --- | --- | --- | --- | --- | --- |
| low | −16 dBFS | 4:1 | −38 dBFS | 66.7:1 | 20 ms | 200 ms |
| mid | −16 dBFS | 4:1 | −38 dBFS | 66.7:1 | 8 ms | 100 ms |
| high | −26 dBFS | 1000:1 | −48 dBFS | 66.7:1 | 2 ms | 50 ms |

At `time` 100 %. **These are lane OTT's constants**, set by reasoning from what is public, not read from Ableton's preset or Xfer's plug-in (neither is available to us, and Xfer's must not be run): no primary source publishes the OTT preset's numbers ([Defaults evidence](#defaults-evidence)). What the constants follow:

- the **structure** is Ableton's Multiband Dynamics: per band an Above threshold (downward compression) and a Below threshold (upward compression), each with a ratio and attack and release, a global Time and an Amount (manual 29.26.2);
- the **66.7:1 upward ratio** and the high band's **∞:1 downward ratio** (1000:1 here) are the two numbers that blog write-ups of the preset repeat (single secondary source; kept because they define the sound: strong upward compression everywhere, a limiter on the top);
- the **thresholds** are placed for a mastered, tilted spectrum: low and mid share −16 / −38 dBFS (22 dB between the curves, the dense OTT window), the high band sits 10 dB lower;
- the **times** follow the band's cycle length: slow lows (20 / 200 ms), fast highs (2 / 50 ms). The compressor's detector reads a steady sine 1.0–1.4 dB under its peak (the same offset `compressor.md` records), which the tests account for.

The CAT table keeps these out of the parameters (it has `time`, `depth`, `upward`, `downward` and the three gains only).

### The detector, the stereo link and the onset guard

The detector is the compressor's: the amplitude follower in increment form with the attack or release coefficient (`k = 1 − exp(−1/(τ·fs))`, `expm1` in `f64`), `20·log10` through `libm::log2f`, the gain smoother at attack/2, and a stereo link by the louder channel of the band. `docs/dsp/compressor.md` ("Attack and release") has the time-constant derivation; the white-box tests reproduce its 63.2 % points for every band, `time` and sample rate.

**The onset guard.** With no look-ahead (latency 0, D10), a loud sample can meet a lift that was computed for the quiet signal before it: a note that starts after a gap, with up to 30 dB of lift held by the release, would leave the module 20 dB or more over its input for the first milliseconds. A lift therefore never carries a band sample above the band's **upward threshold + 20 dB**: where it would, the lift is cut for that sample (never below the downward gain) and forgotten, so it does not return until the detector asks for it. A steady signal is brought to the upward threshold (a sine at the threshold, noise about 12 dB over it), so the guard binds only on the first few milliseconds of an onset. Measured: a tone steps from −60 to −6 dBFS, and the peak of the next 100 ms is read, with the guard disabled and enabled, against the same step at depth 0 (the split alone, whose own onset ripple is the floor):

| Band (tone) | Depth | Without the guard | With it | Depth 0 |
| --- | --- | --- | --- | --- |
| low (60 Hz) | 100 % | +11.35 dBFS | −5.99 dBFS | −6.00 dBFS |
| mid (1 kHz) | 100 % | +15.03 dBFS | −5.29 dBFS | −5.29 dBFS |
| high (8 kHz) | 100 % | +6.10 dBFS | −5.58 dBFS | −5.58 dBFS |
| mid (1 kHz) | 30 % (the default) | +0.54 dBFS | −5.29 dBFS | −5.29 dBFS |

So a loud onset leaves exactly as loud as the split alone makes it, and the default depth needed the guard as well (6 dB over the input without it). A quieter onset keeps what lift the ceiling allows: `−60 → −20 dBFS` at depth 100 % peaks at −17.8 (low), −17.9 (mid) and −19.6 dBFS (high), against ceilings of −18, −18 and −28 dBFS and inputs 20 dB down. After about 17 ms the output settles on the steady curve. The guard is the only branch in the gain path that depends on the sample, and it is deterministic. The test `a_note_after_a_gap_is_not_thrown_far_over_its_input` holds it for all three bands, two depths and three steps.

### Real-time and determinism

No allocation (the state is a fixed-size struct), `libm` transcendentals only, no fused operations, a fixed band order (low, mid, high) for the sum. **Denormals are flushed in code, never by FTZ/DAZ:** the filter states at the end of every `process` call (the engine makes it a 32-frame segment), the follower and smoothers every frame, the wet output before it is stored. NaN or infinite input is replaced by 0 before it reaches a filter, so it cannot be held in the state. `is_silent` is true when the filters, followers and smoothers are exactly zero, and then an all-zero input gives an all-zero output and changes nothing for any settings, so the engine's skip rule is bit-safe (the parameter ramps use the engine's `prev` and `cur`, which move whether or not the module runs).

## Measured

All checks run natively (debug and release) and, for the DSP core, on `wasm32-wasip1` under wasmtime and with `-C target-cpu=native`; the render hash is one number on all of them (`0x2e24_790f_2cbd_7392`). The tests are `crates/enth-dsp/tests/ott.rs` (the core against closed forms) and `crates/enth-engine/tests/ott.rs` (the module and the engine); `cargo test -p enth-dsp --test ott -- --nocapture` and the same for `-p enth-engine` print the tables below. The platform checks:

```sh
RUSTFLAGS="-C target-cpu=native" cargo test -p enth-dsp --release --test ott           # another instruction set
CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --" \
  cargo test -p enth-dsp --release --target wasm32-wasip1 --test ott                   # wasm32 under wasmtime
cargo xtask golden --wasm                                                              # the goldens, same
```

### Closed-form checks (acceptance)

| Check | Result |
| --- | --- |
| Each of the three bands against its Linkwitz–Riley closed form (four split pairs, 20 Hz – 20 kHz) | worst complex error 4e-14 to 2e-13 (`f64` rounding) |
| **Flat sum**, unity bands, splits 88.3 / 2,500, 40 / 500, 500 / 10,000, 250 / 1,500 Hz | **0.00000 dB** at every tested frequency (bound 0.1 dB); phase equals the two allpasses to 8e-9 rad |
| Flat sum through the engine, depth 0, a sine at 9 keys (32 Hz … 8.4 kHz), three split settings | **0.0000 dB** worst level error |
| Static curve against the documented equation (14,001 levels × 5 intensity pairs × 3 bands) | worst 1.05e-6 dB |
| Per-band output level against the curve, one tone in each band (isolating splits), −100 … 0 dBFS in 4 dB steps, rising and falling | worst **0.036 dB** (low), **0.029 dB** (mid), **0.169 dB** (high, in the fade region); bound 0.5 dB |
| `depth`, `upward`, `downward` scale the dB gain linearly; the two knobs pick one curve | worst deviation 0.006 dB |
| Depth 0 is the band split (flat) | every band's gain is exactly 1 (`exp2f(0)` is exactly 1, a white-box test), so the output is the sum above; flat to 0.0000 dB through the engine |
| `mix` 0 is the input, bit for bit (module and engine, every other setting at its extreme) | exact |
| Latency | 0: an impulse leaves on the frame it enters |

Phase of the flat sum at the default splits (`A₁(A₂(x))`):

| Hz | 20 | 50 | 100 | 250 | 1,000 | 2,500 | 5,000 | 10,000 | 20,000 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| degrees | −38.6 | −102.6 | 153.6 | 43.3 | −53.0 | −174.3 | 86.8 | 36.6 | 7.4 |

The magnitude is flat everywhere, the phase is what two allpasses do; a patch that doubles the dry signal against `ott` at `mix` below 100 % gets the comb it implies.

Per-band curves, measured rows of the sweep (the gain in dB of a settled sine for its input level; depth 100 %, isolating splits; the full tables, 26 levels and the curve column, are in the test output):

| Band | in −88 dBFS | −80 | −60 | −44 | −32 | −20 | −8 | 0 | Worst error |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| low, 30 Hz | +17.07 | +30.00 | +22.82 | +7.06 | +0.06 | −0.02 | −5.12 | −11.12 | 0.036 dB |
| mid, 1 kHz | +17.47 | +30.00 | +22.67 | +6.91 | +0.04 | −0.03 | −5.24 | −11.21 | 0.029 dB |
| high, 15 kHz | +18.29 | +30.00 | +12.49 | +0.30 | 0.00 | −5.33 | −17.30 | −25.29 | 0.169 dB |

The cap (+30 dB) holds from −80 up to −68 dBFS in the low and mid bands (the high band's upward threshold is 10 dB lower, so its cap ends at −80); below −84 dBFS the fade takes the lift to 0 at −96.

### What the default crossovers do to a tone

The band curves above hold with isolating splits. **At the default crossovers a tone near a crossover also lands in the neighbouring band, and that band lifts it** (lifting what is quiet is its job) and adds it back, so the output of a tone deviates from its own band's curve, always upward and only in the upward region:

| Tone (band) | Worst deviation from its band curve | At input level |
| --- | --- | --- |
| 40 Hz (low) | +5.35 dB | −40 dBFS |
| 60 Hz (low) | +5.84 dB | −48 dBFS |
| 200 Hz (mid) | +5.01 dB | −40 dBFS |
| 500 Hz (mid) | +0.63 dB | −8 dBFS |
| 1 kHz (mid) | +2.23 dB | −44 dBFS |
| 6 kHz (high) | +4.88 dB | −36 dBFS |

This is how a multiband upward compressor sounds (it is a large part of OTT's density), not an error; the test `at_the_default_splits_…` records it so that the numbers on this page are checked.

### The notches a dry blend would cut

`mix` 50 % at depth 0 (the band split blended with its own input) against the closed form `|(1 + A₁A₂)/2|`:

| Hz | 50 | 80 | 88 | 100 | 200 | 1,000 | 2,000 | 2,500 | 3,000 | 8,000 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| dB | −4.08 | −20.54 | **−27.03** | −12.82 | −1.37 | −0.97 | −8.74 | **−26.11** | −13.30 | −0.83 |

Measured equals closed form to 0.01 dB. This is the reason `depth` scales the compression instead of blending a dry signal: a blend after the split would notch the crossovers by up to 27 dB.

### Silence, tails, non-finite input

- Silence in gives exact zeros for every setting; non-finite samples never enter the state (NaN and ±inf in, finite out, equal to the run with 0 in their place).
- **40 s tail:** after a loud burst the module reaches exact idle (all state 0) after **0.7 s at `time` 10 %, 6.6 s at 100 %, 25.8 s at 400 %** (a `time` of 1,000 % would need about 70 s, extrapolated: the follower falls 8.7 dB per release time and is flushed at −300 dBFS); **zero subnormals** in the output and the state throughout, in the module and through the engine (a saw chord with a 3 s release, `time` 400 %, depth 100 %, 40 s).
- **FTZ-independent:** the same bits under IEEE, FTZ and FTZ+DAZ (x86-64; FZ on aarch64), module and engine.
- **Host block size** (1, 7, 32, 64, 100, 4,096 frames) is not an input; **two instances keep separate state** (side by side, and two `ott`s in series equal the two modules run by hand, bit for bit); an idle module skipped by the engine gives the same bits as one always called (settings changing while it sleeps); `copy_state_from` makes two modules identical from then on (and `swap.rs` covers the plan-swap carry for the kind).
- No allocation in `process`, in the module and the engine (`assert_no_alloc`, debug builds).

### Aliasing, measured and bound

Report 02's measure ([aliasing.md](aliasing.md)): alias power over harmonic power, 20 Hz – 20 kHz, ±8 bins around each harmonic counted as signal; the engine at 48 kHz, an `osc` sine voice held through `ott1`, eight keys (33 … 117, i.e. 55 Hz … 7 kHz), three levels (**loud** −3 dBFS, where the downward curves work; **middle** −31 dBFS, near the knees; **quiet** −63 dBFS, where the upward curves lift), `enth_dsp::alias::alias_db`. A steady tone holds the gain still, so the module adds nothing to alias except the follower's ripple at twice the tone's frequency modulating the gain: it grows as the times shrink and the tone rises. The worst point of each sweep (the test prints the tables):

| Sweep | Worst | Bound (worst + 1.5 dB) |
| --- | --- | --- |
| depth 100 %, `time` 100 % | −92.6 dB | −91.1 dB |
| the catalogue defaults (depth 30 %) | −92.7 dB | −91.2 dB |
| depth 100 %, `time` 10 % (the fastest) | −65.9 dB | −64.4 dB |
| depth 100 %, `time` 10 %, `input` +24 dB, splits 40 / 500 Hz (the high band holds almost the whole spectrum) | −65.2 dB | −63.7 dB |

The −93 dB of the first two rows is the engine's own sine (`alias.rs` asserts it below −90 dB, and the measure's window leaks about −92 dB), not the module. At `time` 10 % the worst points are the high keys in the high band: depth 100 %, `time` 10 %, loud:

| Hz | 55 | 110 | 220 | 440 | 880 | 1,760 | 3,520 | 7,040 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| loud | −93.2 | −103.0 | −94.6 | −92.2 | −90.6 | −88.8 | −74.8 | −67.7 |
| middle | −93.2 | −103.3 | −96.9 | −93.4 | −94.3 | −92.0 | −91.2 | −101.9 |
| quiet | −93.2 | −102.7 | −92.9 | −91.2 | −88.6 | −87.7 | −70.8 | −65.9 |

At the default `time` there is nothing to bound above the floor, and no oversampling is needed: the module's gain is slow by design. The fastest setting is "gritty" on purpose; a user who wants it clean raises `time`.

### Cost, measured

gungraun (callgrind) on x86-64 Linux, 1 s of a held chord at 48 kHz through `ott1` (`benches/ott.rs`; the matching `engine` bench in the same tree is 3,610,682 and 37,290,522 instructions for 1 and 16 voices):

| Case | Instructions | Minus the engine bench | Per frame | Budget (+10 %) |
| --- | --- | --- | --- | --- |
| 1 voice, defaults | 50,492,504 | 46,881,822 | 977 | 55,542,000 |
| 16 voices, defaults | 83,802,537 | 46,512,015 | 969 | 92,183,000 |
| 16 voices, depth 100 % | 83,809,734 | 46,519,212 | 969 | 92,191,000 |
| 16 voices, `mix` 0 | 82,746,537 | 45,456,015 | 947 | 91,022,000 |

The module's cost is about 970 instructions per stereo frame, 47 M instructions per second at 48 kHz (roughly 1.5 % of one core at 3 GHz and one instruction per cycle), the same at any depth (every band, both curves and the mix branch run every frame, so the state is the same whether or not the engine skips an idle module). About five times the compressor module's 190: the split is ten `f64` state-variable steps a frame (five per channel), and the three band gains cost three `log2f` and three `exp2f`. Not optimised further.

### Goldens

`tests/golden/cases/ott-*.toml` (14, native and `wasm32-wasip1`; none of the 87 existing cases moved): `ott-default` (the catalogue defaults on a bass line), `ott-full` (depth 100 %), `ott-identity` (depth 0: the flat split), `ott-downward` (upward 0, input +12 dB), `ott-upward` (downward 0, a decaying pluck: the tail is lifted to the cap), `ott-fast` and `ott-slow` (`time` 10 % and 400 %), `ott-bands` (moved splits, band gains, input and output), `ott-mix` (50 %), `ott-automation` (depth, input, low gain, upward and mix ramping), `ott-wake` (two notes 1.5 s apart: the idle and skip path), `ott-noise` (white and pink noise), `ott-onset` (a quiet note then a loud one: the guard) and `ott-two` (two instances in series). They guard "same as before", not "correct"; the closed-form checks above are the second half.

### WAVs

A dry bass loop (the `bass_sub` template: a saw over a sine an octave down through a dark resonant low-pass, legato glide, master −6 dB) played as 16 eighth-note-grid notes at 140 bpm (7.9 s), through no effect, `ott1` at the defaults and `ott1` at depth 100 %. Rendered with `enth render`, never played. In `~/code/enth-scratch/s2/ott/wav/` (outside the repository; the patches and the performance are beside them):

| File | Integrated loudness | Spectral centroid | Peak |
| --- | --- | --- | --- |
| `ott-bass-dry.wav` | −19.1 LUFS | 233 Hz | −4.6 dBFS |
| `ott-bass-default.wav` (depth 30 %) | −20.8 LUFS | 260 Hz | −3.6 dBFS |
| `ott-bass-full.wav` (depth 100 %) | −24.3 LUFS | 365 Hz | −4.0 dBFS |

At the default the bass is denser and brighter (centroid +12 %, the envelope's ripple up by half a dB) and 1.7 LU quieter, the net of the downward compression of the loud low and mid bands and the lift of the tails; at full depth it is 5.2 LU quieter and 57 % brighter. With `output` at 0 dB the full effect needs make-up gain: the table's `output` is the knob (see the notes below).

## Defaults evidence

**Open check** (phase-s2.md, "OTT's defaults come from one or two blogs"): CAT's low-confidence defaults are `low_split` 88.3 Hz, `high_split` 2.5 kHz and `depth` 30 %. Method: one page of the vendor's manual, fetched with the User-Agent `enth-research/0.1` after reading `robots.txt` (it allows `/en/live-manual/`; the only legal pages we found, under `/en/legal/`, are trademark, patent and branding notices, and the site-terms URL we tried does not exist), no scraping, nothing else from the vendor's site. A second blog (bedroomproducersblog.com) answered 403 to the same User-Agent and was **not retried**; mixelite.com served a maintenance page. Xfer's plug-in was never downloaded or run (its site only lists release notes). The other sources below are web-search result snippets (the pages were not fetched).

**Primary source read:** Ableton, *Live 12 Reference Manual*, 29.26 "Multiband Dynamics" (https://www.ableton.com/en/live-manual/12/live-audio-effect-reference/). It confirms the **structure** CAT's table assumes and the meaning of two controls; it contains **no default value and no preset**:

| CAT's table says | Manual 29.26 |
| --- | --- |
| three bands, two adjustable crossovers | up to three independent bands with adjustable crossover points (an example of 500 Hz and 2,000 Hz is given, not a default) |
| downward compression above a threshold, upward below another, per band | each band has an Above and a Below threshold, so two kinds of processing run at once in every band |
| `time` scales every attack and release | the Time control scales all attack and release durations together |
| `depth` is intensity, not dry/wet | the Amount knob sets the intensity of the compression or expansion in all bands, and at 0 % every compressor/expander has an effective ratio of 1 |
| per-band gains, `input`, `output` | an Input and an Output knob per band, and a global Output |

**Secondary sources** (search-result snippets, pages not fetched): MusicRadar quotes the OTT preset's crossovers as 88.3 Hz and 2.5 kHz; EDMProd also gives 88.3 Hz, says the stock device's low–mid split is 250 Hz, and quotes the 1:66.7 upward ratios of the low and mid bands, an infinite ratio in the high band and about +5.2 dB of input gain per band (a single source for each of these); a plug-in review (KVR) puts Xfer's OTT about 1.3 dB higher in peak and 0.8 LUFS louder than Live's preset; a forum reply (FabFilter) claims the preset gets its upward action from an expansion setting and does not use both curves in one band (unverified: the manual shows the device can do both at once, and says nothing about the preset).

**Verdict per default:**

| Default | Evidence | Proposed value |
| --- | --- | --- |
| `low_split` 88.3 Hz | secondary only (two independent snippets agree); not contradicted by the manual; inside the range 40 – 500 Hz | **keep** |
| `high_split` 2.5 kHz | secondary only (one snippet); not contradicted by the manual; inside the range 500 Hz – 10 kHz | **keep** |
| `depth` 30 % | no source speaks to it: it is CAT's gentle-default decision (S2 rule), not a fact about the preset. Measured here instead: on the bass loop, 30 % is +12 % centroid and −1.7 LU, 100 % is +57 % and −5.2 LU ([WAVs](#wavs)) | **keep**: audible and gentle; the full effect is one turn of the knob |

**The table is not changed.** The sources that would settle the band constants (the preset's thresholds, ratios and times, and the per-band input gains EDMProd mentions) are inside Live's installed device presets and Xfer's plug-in; neither is available to us, so [the constants](#the-bands-constants-and-where-they-came-from) are lane OTT's, documented here and in `ott.rs`, and a later lane that can read the preset can replace the `BAND_SPECS` table in one place. If the owner wants OTT's loudness out of the box, the single-source +5 dB of band input is the lead to follow (a default `input` or band gains of about +5 dB); it is not proposed here, because a default that adds level invites clipping.

## Notes for lane INT-S2a, S2b and the coordinator

- **Hidden:** `ModuleSpec::hidden` is untouched. INT-S2a clears it when the module is to be shown.
- **Level:** at `depth` 100 % and `output` 0 dB the module is quieter than its input on loud material (−5 LU on the bass loop). The table has `output` (and `input`) for that; no default changed.
- **Cost:** about 970 instructions per frame (no per-voice cost: it is a global effect). G2B's patch budget gets it once.
- **`every_kind_patch`** in `crates/enth-engine/tests/common/mod.rs` does not list `ott`; INT-S2a may add `ott1` there (the tail and swap tests would then cover it too). OTT's own tests cover the tail, swap carry (`swap.rs` runs every kind) and allocation already.
- **Shared-file edits** (all additive, nothing else outside the lane's files): `crates/enth-engine/src/params.rs` (`OttParam` in the import and the `param_index!` list) and `crates/enth-dsp/src/compressor.rs` (`pub(crate)` on six private items the core reuses: `MIN_LEVEL`, `MAX_LEVEL`, `DB_PER_LOG2`, `LOG2_PER_DB`, `one_pole_gain`, `sane`; no numeric path changed).

## Limitations

- No look-ahead: a loud onset meets the gain of the quiet passage before it for the first milliseconds. The [onset guard](#the-detector-the-stereo-link-and-the-onset-guard) bounds the damage: a loud onset leaves as loud as the split alone makes it, a quieter one at most 2.2 dB over its input in the measured cases; a limiter before `ott` or a faster `time` is the user's tool for the rest.
- Upward compression lifts leakage across the crossovers ([measured](#what-the-default-crossovers-do-to-a-tone)); isolate a band by moving the splits apart.
- One stereo link per band (the louder channel drives both gains); no mid/side mode, no sidechain, no per-band bypass, no external detector.
- The follower reads sample peaks of the band signal, so a tone near a quarter of the sample rate reads a few dB differently depending on its phase against the sampling grid (the compressor's detector does the same).
- Constants are lane OTT's; see [Defaults evidence](#defaults-evidence).

## Files

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/ott.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)), unchanged |
| DSP core | `crates/enth-dsp/src/ott.rs` | the split, the curves, the guard, the ramps; white-box tests inside |
| Core tests | `crates/enth-dsp/tests/ott.rs` | closed forms, curves, intensity, mix, notches, silence, tail, latency, hash, allocation |
| Engine module | `crates/enth-engine/src/modules/ott.rs` | `core_params`, the per-segment ramp, idle and state carry |
| Engine tests | `crates/enth-engine/tests/ott.rs` | the module and the engine, FTZ, block size, aliasing sweeps |
| Instruction budget | `crates/enth-engine/benches/ott.rs` | [Cost](#cost-measured) |
| Golden cases | `tests/golden/cases/ott-*.toml` | 14, [Goldens](#goldens) |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)); a `THIRD_PARTY_NOTICES.md` entry and a provenance section for every port (none: nothing is ported).

**OTT's own row** ([phase-s2.md](../phase-s2.md#acceptance-per-lane)): the crossovers sum flat (0.00000 dB against the 0.1 dB bound, phase recorded), the per-band static curves within 0.5 dB (worst 0.169 dB), depth 0 the band-split identity, intensity scaling as documented, latency 0, aliasing measured with its bound ([Aliasing](#aliasing-measured-and-bound)), the goldens, the budget, the WAVs and the defaults evidence ([above](#defaults-evidence)).
