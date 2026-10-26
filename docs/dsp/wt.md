# Wavetable (`wt`; `enth-dsp::wt`, `enth-engine::modules::wt`)

- **Status: implemented (S2a lane WT, 2026-10-26)**, to lane CAT's F1m table below; unison (`unison`, `detune`, `blend`, `spread`) is lane UNI's and is inert here ([Unison](#unison-lane-uni)).
- Kind `wt`, discriminant 8, section `generators`, id prefix `wt` (wt1, wt2…), rate voice. Fixed and persisted from F0m; **still hidden** from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag (`ModuleSpec::hidden` is untouched).
- A wavetable oscillator on phase-read waves: five generated band-limited tables, a frame `position`, five phase `warp`s, phase modulation (`pm`, `pm_index`) and amplitude or ring modulation (`am`) from other generators through lane RTE's audio routes, self-`feedback`, and `semi`, `fine`, `level` and three phase modes. It also feeds audio routes itself (a silent `level` 0 `wt` is an FM operator).
- Sources: Chowning, "The synthesis of complex audio spectra by means of frequency modulation", JAES 21(7), 1973 (the Bessel sidebands); Tomisawa's self-feedback FM (US patent 4,249,447, 1981, expired) and Schottstaedt's FM notes (the two-sample average); research report 05 §1 (PM, AM, ring modulation, feedback) and report 06 §3.1 (the audio-route contract) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`); Kleimola et al., "Phaseshaping oscillator algorithms for musical sound synthesis", SMC 2010 (phase warps). No code is ported; the table layout and the 4-point Hermite read are `osc`'s ([osc.md](osc.md#wavetables); stmlib's `InterpolateHermite` is already in `THIRD_PARTY_NOTICES.md`), shared through `enth-dsp::wavetable`.

## Parameters (lane CAT, F1m)

The table lane WT (unison: UNI) implements, generated from `crates/enth-patch/src/catalogue/wt.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **PM:** the phase offset in radians is `pm_index · (pm / 100) · pm_in(t)`, where `pm_in` is lane RTE's summed audio input (each route's pre-level source × its depth). `pm` (audio, not modulatable) trims everything routed in, default 100 %; `pm_index` (0–10 rad, modulatable) is the amount an envelope, LFO or MSEG moves for growls.
- **AM:** the output is multiplied by `am / 100 + am_in(t)`: 100 % and no route is unity; a route with `am` above 0 is AM (sidebands at `depth / (2·am)` of the carrier); **`am` 0 % with a route is ring modulation** (no carrier). Without a route `am` scales the output.
- **Warps** are the identity at `warp_amount` 0; the sign flips the direction for `bend`, `asymmetric` and `squeeze`; `sync` and `quantize` use the magnitude. `feedback` is self-PM with a one-sample delay and two-sample averaging (report 05), 0 % none.
- **Unison (lane UNI):** `unison` copies (1–8, one `f32x8` lane) detuned evenly across ±`detune`, side copies at `blend`, panned across `spread`. `unison` is structural so the plan knows from the patch whether it needs SVP's stereo voice path (recommended: stereo when an enabled `wt` has `unison` ≥ 2; `spread` 0 must equal mono). A copy's start phase: `derive(voice_seed, k)` in `free`, the module id and `k` in `reset` (the same every note), `seed` and `k` in `seeded`; never a parameter value.
- **Defaults:** a plain sine at full level (`basic` at position 0), `phase_mode` reset, since PM, warps and feedback make the start phase part of the timbre.

### `wt` — Wavetable (generators, ids `wt1`, `wt2`…; 18 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `table` | none | `basic`, `fm`, `fm_octave`, `formant`, `metal` | enum | basic | structural | basic | Which generated wavetable to play: basic shapes, FM sweeps, a formant sweep or sparse metal; changing it rebuilds the voice plan. |
| `position` | percent | 0% … 100% | linear | 0% | mod, auto | basic | Where in the wavetable to play, from its first frame to its last; route an LFO or MSEG here for moving timbres. |
| `warp` | none | `off`, `bend`, `asymmetric`, `squeeze`, `sync`, `quantize` | enum | off | auto | intermediate | How the wave's cycle is bent before it is read: bend, asymmetric bend, squeeze, sync or quantize. |
| `warp_amount` | percent | -100% … 100% | linear | 0% | mod, auto | intermediate | How strongly the warp bends the cycle: 0 % leaves it straight; the sign flips the direction where the warp has one. |
| `semi` | semitones | -48st … +48st | linear | 0st | mod, auto | basic | Transposition in semitones from the played note; as an FM modulator, +12st is a 1:2 ratio and +19.02st a 1:3. |
| `fine` | cents | -100ct … +100ct | linear | 0ct | mod, auto | basic | Fine tuning in cents; a few cents off a 1:1 FM ratio makes the timbre slowly churn. |
| `level` | percent | 0% … 100% | linear | 100% | mod, auto | basic | Output level into the voice, as a linear percentage; at 0 % an FM modulator is silent but still modulates. |
| `pm` | percent | 0% … 100% | linear | 100% | auto, audio | advanced | Where audio routes from other generators land for phase modulation (FM); its value trims everything routed in: 100 % passes the route depths as set, 0 % turns PM off. |
| `pm_index` | none | 0 … 10 | linear | 1 | mod, auto | intermediate | How far a full-scale input into pm bends the phase, in radians: 1 is gentle, 5 a growl, 10 a harsh clang; route an envelope or MSEG here to make it talk. |
| `am` | percent | 0% … 100% | linear | 100% | auto, audio | intermediate | Where audio routes from other generators land for amplitude modulation; its value is the gain they swing around: 100 % is AM, 0 % is ring modulation (without a route it scales the output). |
| `feedback` | percent | 0% … 100% | linear | 0% | mod, auto | intermediate | Phase modulation of the oscillator by its own output: a sine turns towards a saw, then a rough, noisy edge. |
| `unison` | none | 1 … 8 | int | 1 | structural | intermediate | Copies of the oscillator per voice (1 is off), detuned against each other for width and thickness; changing it rebuilds the plan. |
| `detune` | cents | 0ct … +100ct | linear | +20ct | mod, auto | intermediate | How far the outermost unison copies sit from the center, in cents (±): small values thicken, large ones sound like a detuned chord. |
| `blend` | percent | 0% … 100% | linear | 100% | mod, auto | advanced | Level of the side unison copies against the center one: 0 % is the center alone, 100 % every copy equal. |
| `spread` | percent | 0% … 100% | linear | 50% | mod, auto | intermediate | How widely the unison copies are spread across the stereo field: 0 % is mono, 100 % fully left and right. |
| `phase_mode` | none | `free`, `reset`, `seeded` | enum | reset | auto | advanced | How each note's start phase is chosen: reset (the phase knob, the same every note, as FM and warps want), free (random per render seed) or seeded (from seed). |
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | The start phase in reset mode; between a carrier and its modulator it changes the FM timbre. |
| `seed` | none | 0 … 65535 | int | 0 |  | advanced | The random seed used in seeded phase mode, for the start phase and the unison copies; part of the patch, unlike the render seed. |

## The signal path

`Voice::render` (`crates/enth-dsp/src/wt.rs`) reads the table once per sample at

```text
p = wrap(phase + pm_offset(t) + feedback_offset(t))      y = table(position, warp(p))
```

then the module multiplies by the AM gain, copies the result into the audio route's buffer if a route leaves the module (`VoiceCx::pre_level`, **before** the `level` ramp, for every active voice, so `level` 0 makes a silent operator that still modulates), and applies the `level` ramp. The output at the first sample is the table at the start phase, like `osc`'s sine and table, so there is **no latency** (`osc` has one sample).

- **Pitch.** `pitch + semi + fine/100` semitones, ramped linearly across each 32-sample segment (`pitch_prev` to `pitch_cur`, so glide and vibrato work), `midi_to_hz` through the pinned `libm`, and the phase increment clamped to 0.45 of the sample rate as `osc` does.
- **Position, warp amount, level, `pm_index`, `pm`, `am`, `feedback`** are smoothed parameters: the module reads `prev` and `cur` and ramps linearly between them per sample (the engine's `RAMP`). `table`, `warp`, `phase_mode`, `phase` and `seed` are read from `base`.
- **Phase modes.** As `osc`: `reset` (the default) starts every note at `phase`; `free` draws the start phase from the voice seed (render seed, module id, note index); `seeded` from the module's own `seed`.

### The tables

All five are generated in code (no audio assets) by `enth-dsp::wavetable::build`, the generator behind `osc`'s tables, now shared: **13 frames**, each with **9 mip levels** (level `l` holds harmonics `1 … 256 >> l`, stored at `max(32 × the top harmonic, 1024)` samples plus 3 guard samples), a 4-point Hermite read in phase and a linear cross-fade between the two frames around `position` (0–100 % maps to frame 0 … 12). About 1.07 MB per table, shared process-wide through one `Arc` per table (a second `wt` or a second plan costs a refcount). Exact integer-indexed additive synthesis in `f64`, harmonics in ascending order, every frame peak-normalised at level 0 with the other levels sharing its scale, rounded once to `f32`; the Bessel functions come from their power series in `f64` (60 fixed terms, `+ − × ÷` only), so the data is the same on every platform. The tests pin one FNV-1a fingerprint per table (`the_table_data_is_pinned`). Generation takes tens of milliseconds per table at plan-compile time, never on the audio thread.

| Table | Frame at `pos ∈ [0, 1]` (harmonic amplitudes) |
| --- | --- |
| `basic` | The `osc` basic table: sine → triangle → saw → square in three equal blends. **Bit for bit the osc's** (the same fingerprint; the `wt-position` golden has the hash of `osc-table-basic`) |
| `fm` | `sin(x + I·sin(x))`, `I = 10·pos`: Chowning's two-operator FM at 1:1, harmonic `k` at `|J_{k−1}(I) + (−1)^k·J_{k+1}(I)|` (the unit test `fm_frames_are_chowning_spectra` checks it against the series) |
| `fm_octave` | `sin(x + I·sin(2x))`, `I = 10·pos`: the modulator an octave up, so only the odd harmonics sound, harmonic `k` collecting `J_{(k−1)/2}(I)` and `J_{(k+1)/2}(I)` |
| `formant` | A saw (`−1/k`) times `1 + 5/(1 + ((k − c)/w)²)`: one resonant peak sweeping harmonic `c = 32^pos` (1 … 32), half-width `w = max(0.75, 0.15·c)` |
| `metal` | 24 partials at harmonics 1, 3, 5, 8, 11, 14, 18, 23, 29, 35, 42, 50, 59, 69, 80, 92, 105, 119, 134, 150, 167, 185, 204, 224 at amplitude `1/√(1+j)` and quadratic-residue phases `2π·frac(φ·j²)`; `3 + 21·pos` of them sound, the last fading in, so the waveform is a bell-like cluster and not a spike |

Truncating an FM frame to a low mip level rings (Gibbs): the lowest levels of the `fm` and `fm_octave` frames peak up to 1.34 of the level-0 scale, which the table test allows (`≤ 1.5`).

### Warps

Every warp bends the read phase `p ∈ [0, 1)` before the table is read, and is the **identity at `warp_amount` 0** (the module takes the unwarped path, so the bits equal a module with `warp` `off`). `a` below is `warp_amount/100`.

| Warp | Map | Steepest slope |
| --- | --- | --- |
| `bend` | `p / (p + k(1 − p))`, `k = 8^(−a)`: the detail bunches at the start (`a > 0`) or the end (`a < 0`) | `max(k, 1/k)`, up to 8 |
| `asymmetric` | Piecewise linear with a knee at `m = ½ − 0.45a`: the first half of the cycle plays in `[0, m]`, the second in `[m, 1]` (pulse-width-like for any wave) | `max(0.5/m, 0.5/(1 − m))`, up to 10 |
| `squeeze` | The cycle plays in the first (`a > 0`) or last (`a < 0`) `w = 1 − 0.9|a|` of the period, the rest holds the cycle's end (start) value | `1/w`, up to 10 |
| `sync` | `frac(r·p)`, `r = 1 + 7|a|`: the cycle repeats and restarts, like hard sync | `r`, up to 8 |
| `quantize` | `⌊N·p⌋/N`, `N = round(256·2^(−7|a|))` steps (256 … 2); `N` changes by whole steps, so the end value holds for a segment | 1 |

### Phase modulation, AM and ring modulation

- **PM.** The offset in cycles is `pm_in(t) · pm_index · (pm/100) / 2π`, with `pm_in` the audio routed into `pm`: each source's pre-level output times its route depth, summed (`cx.audio_in.read(WtParam::Pm.index(), v, …)`, lane RTE). The offset in radians is therefore `pm_index · (pm/100) · pm_in(t)`, CAT's formula. With no route, `pm` and `pm_index` do nothing.
- **AM.** The output is multiplied by `am/100 + am_in(t)` (`am_in` the audio routed into `am`, ramped like every gain). `am` 100 % with a route is unipolar AM: the carrier at 1 and two sidebands at `depth/2` each. **`am` 0 % with a route is ring modulation**: the carrier is gone. Without a route `am` scales the output like a second level (and at 100 % the multiply is skipped).
- Because the table is read by phase, PM adds no discontinuity: what it adds is bandwidth (Carson's `2(I + 1)·f_m`), which the mip level tracks (next section).

### Feedback

`feedback` is self-PM: the table is read at `p + g·(y₁ + y₂)/2`, `y₁`, `y₂` the last two outputs before AM (a delay of 1.5 samples; the average is what stops the loop hunting, report 05 §1), with `g = feedback/100 · 2 rad / 2π` cycles. **100 % is 2 radians** (a deliberate choice, below). A sine under feedback turns monotonically into a saw's `1/n` spectrum (the table below).

**Why 2 radians, not π.** Linearised where the table's slope is `s` per cycle, the loop is `z² = (a/2)(z + 1)` with `a = g·s`. Where the table falls (`a < 0`) the roots form a complex pair of modulus `√(−a/2)`: a ring at a third of the sample rate (14–16 kHz) that grows once `a < −2`, and for a sine (steepest slope `2π`) that is `g > 1/π`, that is **2 radians**. An early build calibrated 100 % to π radians (the closest to a saw: 0.9 dB rms from `1/n`), and above about 64 % it rang near 15.9 kHz: the alias measure read −6 to −16 dB at every key for 75 – 100 %. At 2 radians the loop never rings on the sine table, and 100 % is still 1.6–2.1 dB rms from a saw's slope at keys 45 and 57. A brighter table is steeper, so it rings sooner; the catalogue text's "then a rough, noisy edge" is that edge.

### Band limiting: the mip level follows the effective phase increment

The level is picked once per segment by `level_for(tracked, alias_limit)` (the highest-quality level whose top harmonic times the increment stays at or below the alias limit, harmonics reaching 28 kHz at 48 kHz so whatever aliases folds into 20–24 kHz). The increment is the **effective** one: `tracked = slope · (dt + max|Δpm|)`, `dt` the larger end of the segment's phase increment, `slope` the warp's steepest read-rate gain, and `max|Δpm|` the largest per-sample step of the PM offset in the segment. The tracker has instant attack and a release of `0.998` per segment (about 330 ms), so a periodic modulator does not make the level flicker as its slope passes through zero, and **a change of level cross-fades over the segment**, so no step is heard. Feedback does not take part (its slope is the loop's own).

## Measured

### Closed-form spectral checks (`crates/enth-engine/tests/wt.rs`)

A modulator `osc1` (a sine, level 0, so it is only heard through its route) routed into a carrier `wt1` under a gate envelope; carrier key 71 with the modulator at -25 semitones (ratio 0.236, irrational, so no sideband lands on another). A component's amplitude is the square root of the power in ±6 bins around its frequency (a 32,768-sample Blackman-Harris spectrum, `enth_dsp::alias`) over the same for a clean `f64` unit sine.

| Check | Result | Bound |
| --- | --- | --- |
| PM sidebands at carrier ± n × modulator against `\|J_n(β)\|`, n = −4 … 4, β = 1 and β = 2.5 | **0.000 dB** worst error | ≤ 0.5 dB |
| The same β = 2.5 from `pm_index` 2.5 alone, from `pm` at 50 % with `pm_index` 5, from route depth 50 % with `pm_index` 5, and from route depth −100 % | 0.000 dB each | ≤ 0.5 dB |
| `pm_index` 0, or `pm` 0 %, or no route: the plain carrier | bit for bit | exact |
| Unipolar AM (`am` 100/60/50 %, depth 30 – 80 %): the carrier at `am/100` and each sideband at `depth/200` (`depth/(2·am)` of the carrier) | 0.0000 dB error | ≤ 0.1 dB |
| Ring modulation (`am` 0 %, depth ±100/60 %): the carrier against the plain carrier | **−117.9 / −122.3 dB** (−111.9 dB below a sideband) | ≤ −60 dB |
| `am` 0 % without a route | silent | exact |
| Feedback: rms distance of harmonics 2–10 from a saw's `1/n`, key 45 | 85.5, 62.2, 38.7, 21.5, 11.4, 6.2, 3.9, 2.8, 2.2, 1.8, **1.6 dB** at 0, 10 … 100 % | monotone, < 3 dB at 100 % |
| The same at key 57 | 85.5, 62.2, 39.0, 22.0, 12.4, 7.3, 4.9, 3.6, 2.9, 2.4, **2.1 dB** | monotone, < 3 dB at 100 % |
| `basic` at position 0: every harmonic of the sine | below −90 dB | < −90 dB |
| Every table band-limited at a high key | below −80 dB | < −80 dB |

### Aliasing, measured and bound

Report 02's measure ([aliasing.md](aliasing.md)): alias power over harmonic power, 20 Hz – 20 kHz, ±8 bins around each harmonic counted as signal; the engine at 48 kHz, one note per measurement, `enth_dsp::alias::alias_db`. The test (`tests/wt.rs`) prints these tables (`cargo test -p enth-engine --test wt -- --nocapture`) and fails if a cell reaches its bound. The bound is the worst measured point of each region plus 1–2 dB, **by region**, because the aliasing of a modulated oscillator is a property of the setting: a wide spectrum on a high note folds over at any sample rate. The floor of the measure is about −93 dB.

**Tables** (5 tables × position 0/50/100 % × keys 24 … 105): worst **−92.6 dB**; bound **−90 dB** everywhere.

**PM** (a sine carrier, a sine modulator at 2:1, 1:1 and 1:2 of the carrier, so the fundamental to pass is `f_c`, `f_c` and `f_c/2`; the 1:2 rows start at key 36 because key 24 is below the measure's 20 Hz). Bound: **−91 dB for every index 0 – 10 up to key 72** (C5, 523 Hz); −60 dB for an index up to 2.5 above it; +2 dB (no bound, the fold-over) for an index of 5 – 10 on a note above 2 kHz.

| 2:1, index | key 24 | 36 | 48 | 60 | 72 | 84 | 96 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | −97.8 | −93.7 | −94.5 | −93.1 | −97.0 | −93.0 | −98.2 |
| 1 | −98.2 | −94.1 | −94.7 | −93.4 | −96.7 | −93.2 | −97.5 |
| 2.5 | −95.4 | −95.1 | −93.4 | −97.9 | −100.8 | −97.6 | −61.8 |
| 5 | −98.9 | −96.8 | −94.9 | −94.4 | −97.5 | −93.1 | **−27.4** |
| 10 | −95.8 | −97.6 | −96.3 | −95.0 | −94.5 | **−32.2** | **+0.4** |

At 1:1 every cell is below −91.9 dB except index 10 at key 96 (−28.7 dB); at 1:2 every cell is below −93.0 dB. The two ways out are the user's: lower `pm_index` on high notes, or route the key to `pm_index` in the matrix (`pm_index` is modulatable).

**Feedback** (the sine table):

| Feedback | key 24 | 36 | 48 | 60 | 72 | 84 | 96 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 % | −97.8 | −93.7 | −94.5 | −93.1 | −97.0 | −93.0 | −98.2 |
| 25 % | −97.5 | −93.7 | −94.5 | −93.3 | −96.6 | −93.2 | −97.8 |
| 50 % | −97.2 | −93.9 | −94.6 | −93.7 | −96.1 | −91.8 | −68.1 |
| 75 % | −96.6 | −94.1 | −94.7 | −82.8 | −69.2 | −54.4 | −41.1 |
| 100 % | −95.9 | −94.3 | −94.7 | **−43.2** | **−30.6** | **−27.5** | **−22.5** |

Bound: **−90 dB** for any feedback up to key 48, for feedback up to 25 % anywhere, and for up to 50 % up to key 84; −21 dB for the rest. Near a saw (the 100 % row) the waveform has a near-vertical edge, and an edge aliases like a saw: `osc`'s polyBLEP saw is −40.6 dB at 440 Hz and −28.8 dB at 3,520 Hz ([osc.md](osc.md#aliasing-measured)), against −30.6 and −22.5 dB here at keys 72 and 96. The feedback edge is unmitigated (no BLEP), so it is a few dB worse than polyBLEP's at the same pitch.

**Warps** (the formant table at position 50 %, the setting where the table is bright):

| Warp, amount | key 24 | 36 | 48 | 60 | 72 | 84 | 96 | Ceiling |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| bend +50 % | −67.7 | −63.0 | −58.5 | −39.3 | −34.2 | −27.2 | −25.5 | −23 |
| bend ±100 % | −68.6 | −63.0 | −57.1 | −37.0 | −33.7 | −30.5 | −26.3 | −23 |
| asymmetric +50 % | −74.2 | −69.4 | −64.9 | −46.6 | −41.4 | −34.8 | −33.9 | −14 |
| asymmetric ±100 % | −64.7 | −59.0 | −53.2 | −32.2 | −29.8 | −25.4 | −16.5 | −14 |
| squeeze +50 % | −65.3 | −60.1 | −55.7 | −37.5 | −32.3 | −26.1 | −24.7 | −5 |
| squeeze ±100 % | −53.9 | −47.5 | −42.0 | −21.3 | −18.4 | −16.4 | −7.6 | −5 |
| sync +50 % (r = 4.5) | −68.3 | −63.3 | −58.2 | −39.1 | −33.0 | −32.5 | −30.1 | −28 |
| sync ±100 % (r = 8) | −95.6 | −96.4 | −94.6 | −95.5 | −94.0 | −93.9 | −92.7 | −90 |
| quantize +50 % | −37.4 | −31.4 | −27.9 | −20.1 | −17.5 | −13.3 | −9.9 | −8 |
| quantize ±100 % | −42.1 | −35.8 | −32.3 | −24.7 | −21.6 | −18.4 | −15.7 | −8 |

These are **recorded, not clean**: a warp is a read through a map, and the map's own corners are not smoothed. `bend` is smooth, so its aliasing is the table's band limit at the map's steepest point (the mip level follows it; a safety factor of 2 on the level would gain only 5–7 dB); `asymmetric` and `squeeze` have slope jumps; `quantize` and a `sync` ratio that is not a whole number have jumps in the phase, so they alias like a hard edge with no BLEP. `sync` at ±100 % is a ratio of 8, so every jump lands on the table's own period and it is clean. Smoothing the corners (a BLEP on the map) is a possible later step; the acceptance asks for the PM and feedback sweeps, which are the ones that matter on a sine.

### Cost, measured

Callgrind instructions (`cargo xtask bench --bench wt`, `benches/wt.rs`; 1 s of audio at 48 kHz, 16 voices unless noted, 512-frame host blocks), over the `engine` bench's 3,610,682 (1 voice) and 37,290,522 (16 voices) that hold the reference saw and the envelope.

| Case | Instructions | Budget (+10 %) | Per sample and voice over `engine` |
| --- | --- | --- | --- |
| `wt` alone, 1 voice | 8,552,995 | 9,409,000 | 103 |
| `wt` alone, 16 voices | 112,263,005 | 123,490,000 | 97.6 |
| `wt` with PM from `osc1` | 121,614,216 | 133,776,000 | 109.8 (+12.2 for PM) |
| PM + AM from `osc1`, formant table with a moving position, bend warp, 50 % feedback | 170,628,380 | 187,692,000 | 173.6 (+76.0 over alone) |

For reference `osc`'s table wave costs 88.8 per sample on a frame and 107.9 between two. The read avoids `floor` (a library call on baseline x86-64) while the phase stays within a cycle of [0, 1): 135,134,973 → 112,263,005 for the 16-voice case, the same bits.

### Goldens

Fourteen cases in `tests/golden/cases/wt-*.toml`: `wt-pm-fm`, `wt-am`, `wt-ring-mod` and `wt-fm-chain` (the audio routes: osc → `wt.pm`, osc → `wt.am`, an operator that is itself a `wt` with `level` 0), `wt-position` (**its hash equals `osc-table-basic`'s**: a plain `wt` reads the osc table wave bit for bit), `wt-tables-fm`, `wt-tables-fm-octave`, `wt-tables-formant`, `wt-tables-metal`, `wt-warps-smooth`, `wt-warps-jump`, `wt-feedback`, `wt-phase-modes` and `wt-automation` (smoothed parameters ramping). The 73 existing cases do not move.

## Unison (lane UNI)

`unison`, `detune`, `blend` and `spread` are read by nothing: any `unison` renders as 1 (a test, `unison_is_inert_until_lane_uni`, pins that two patches differing only in these render the same bits). Lane UNI inherits the per-voice `[Voice; MAX_VOICES]` state in `modules/wt.rs`, the `pre_level` copy (mono, per voice) and the start-phase function, and replaces them with its `f32x8` copies.

## Notes for lane INT-S2a, S2b and the coordinator

- **`ENGINE_VERSION`:** `wt` replaces a silent stub, so every render with a `wt` changes. This lane does not bump `ENGINE_VERSION` (`crates/enth-engine/src/lib.rs` is S2a's file); the coordinator bumps it when the lanes merge.
- **RTE's tests that assumed a silent `wt`** now set its `level` (or `pm_index`) to 0: `routing::tests::a_silent_generator_still_feeds_its_routes` and `the_voice_sum_stays_in_patch_order` (`routing.rs`), and `gens()` in `tests/audio_routes.rs`. Behaviour unchanged. `WtParam` joined `params.rs`'s `ParamIndex` list.
- **A matcher must read `am` and `level` together:** `am` at 100 % with a route is the carrier plus sidebands, at 0 % ring modulation; without a route `am` is a second level. `tests/common`'s `every_kind_patch` does not include the S2a kinds.
- **`enth-dsp::wavetable`** now exposes `build`, `row_in`, `cursor_in` and `fingerprint_of` (`pub(crate)`) so that `wt`'s tables share the osc generator; the osc fingerprints are unchanged (the existing pinned test guards them).
- **Taste check:** `~/code/enth-scratch/s2/wt/wav/` holds an FM bass (two sines 1:1 with an index envelope), a ring-mod bell (a sine and a sine at +18 st, `am` 0 %) and a formant table sweep (an envelope on `position`); the patches that made them are in `patches/` there. Rendered with `enth render`, never played.
- **Through the plugin and a host (lane SKS, F2m):** a `wt` fed by an audio route renders bit for bit the same in the plugin as in the CLI at host blocks 1, 64, 100 and 4096 (`cli-equal` takes the 14 `wt-*` cases, the plugin golden has `wt-audio-route`), and REAPER's offline render of a formant `wt` phase-modulated by a sine matches `enth render` to four digits in level and brightness on CLAP and VST3 ([plugin.md](../plugin.md#a-wt-and-an-audio-route-lane-sks-f2m)). A lowered PM depth leaves the mip tracker above its settled value for a few thousand frames (above), so a plugin test of depth edits uses a raised depth for bit equality with a patch loaded at that depth.

## Limitations

- Feedback above about 50 % and PM at an index above 2.5 alias on notes above C5 or so (the tables above); warps with jumps (`quantize`, `sync` at a non-integer ratio) and corners (`asymmetric`, `squeeze`) alias like a naive wave. None of them is band-limited by a BLEP.
- Feedback 100 % is 2 radians (not π): the sine never rings, a brighter table does.
- The mip level steps cross-fade over one 32-sample segment; a modulator whose slope grows faster than that can be heard as a brief brightness change.
- Unison is inert (lane UNI).

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/wt.rs` (lane CAT's F1m table, [above](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/wt.rs` (the tables, the warps, the voice); the shared generator is `crates/enth-dsp/src/wavetable.rs` |
| Engine module | `crates/enth-engine/src/modules/wt.rs` |
| Tests | `crates/enth-engine/tests/wt.rs` (closed forms, aliasing, the contract), the unit tests in `crates/enth-dsp/src/wt.rs` |
| Instruction budget | `crates/enth-engine/benches/wt.rs` |
| Golden cases | `tests/golden/cases/wt-*.toml` |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets (CI's golden workflow checks them after merge; locally `cargo xtask golden`: 87 cases ok on x86-64 Linux, the 73 existing unchanged); FTZ-independent (`the_render_is_the_same_under_every_ftz_mode`); zero subnormals after a 40 s tail (`a_40_s_release_tail_has_no_subnormals_and_every_voice_ends`); latency 0; a budget of the measured count plus 10 % (above); two instances keep separate state (`a_wt_as_a_source_feeds_another_wt_and_the_two_keep_separate_state`), a self-route is refused (`a_wt_routed_to_itself_is_refused`, RTE's `audio_cycle`); `mix` does not apply (a generator); aliasing measured spectrally (above), the bound recorded in the test and here. No port, so no `THIRD_PARTY_NOTICES.md` entry.

**WT's own row** ([phase-s2.md](../phase-s2.md#acceptance-per-lane)): the audio-route goldens (`wt-pm-fm`, `wt-am`, `wt-fm-chain`, `wt-ring-mod`), the index and feedback sweeps inside the recorded bound ([Aliasing](#aliasing-measured-and-bound)), and the closed-form spectral checks ([Measured](#closed-form-spectral-checks-cratesenth-enginetestswtrs)).
