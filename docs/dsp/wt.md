# Wavetable (`wt`; `enth-dsp::wt`, `enth-engine::modules::wt`)

- **Status: implemented (S2a lane WT, 2026-10-26)**, to lane CAT's F1m table below; **unison (`unison`, `detune`, `blend`, `spread`) implemented by lane UNI (2026-10-26)** on `wide` lanes through lane SVP's stereo voice path ([Unison](#unison-lane-uni)).
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

Fourteen cases in `tests/golden/cases/wt-*.toml`: `wt-pm-fm`, `wt-am`, `wt-ring-mod` and `wt-fm-chain` (the audio routes: osc → `wt.pm`, osc → `wt.am`, an operator that is itself a `wt` with `level` 0), `wt-position` (**its hash equals `osc-table-basic`'s**: a plain `wt` reads the osc table wave bit for bit), `wt-tables-fm`, `wt-tables-fm-octave`, `wt-tables-formant`, `wt-tables-metal`, `wt-warps-smooth`, `wt-warps-jump`, `wt-feedback`, `wt-phase-modes` and `wt-automation` (smoothed parameters ramping). The 73 existing cases do not move. The seven `wt-unison-*` cases are lane UNI's ([Unison](#goldens-and-the-taste-check)).

## Unison (lane UNI)

`unison` (1–8) copies of the oscillator per voice, detuned evenly across ±`detune` cents, the side copies at `blend` against the centre, panned across `spread`. The copies run on `wide`'s `f32x8` lanes, summed in a fixed order; with `unison` ≥ 2 the patch runs lane SVP's stereo voice path and the `wt` writes both channels. `unison` 1 is the plain oscillator (the plain `Voice` path, exactly the render above). Code: `crates/enth-dsp/src/wt/unison.rs` (the kernel, with its own module docs), `crates/enth-engine/src/modules/wt.rs` (the module), the engine seam in `docs/dsp/engine.md` ("Stereo voice path", "Audio routes between generators"). Sources: research report 05 §7 (the supersaw, the reese) and report 06 §3.2 (the stereo voice path, `_local/sessions/2026-10-25-s2-depth-and-fast-training/`); no code is ported (the detune law, the power normalisation and the constant-power spread are textbook).

### What a copy is

Lane `k` of a `UnisonVoice` is **exactly a plain `Voice`**: the same phase accumulator, feedback history, mip-level tracker and cross-fade, and the same table read through the same warp, with its own phase increment (the centre's times the detune ratio of the copy, clamped to 0.45 of the sample rate as for one oscillator) and its own start phase. What the copies share: the phase-modulation block (one routed audio input drives all of them), the warp, the position (so the frame pair), the feedback amount and the table. The lanes' operations are the IEEE `f32` `+ − × ÷`, `min`/`max`, `floor`, compares and selects, in the order `Voice::render` does them, so **a lane equals a plain `Voice` bit for bit** (unit tests in `unison.rs`: six warps × 2…8 copies × {PM, feedback, a moving position} × random pitches and mixes, including the mip-level cross-fades; the table read is a gather done lane by lane, `wide` has none). The start phase of copy `k` does not depend on how many copies there are (the detune position and the weight do, by the law below).

### The mix

| Law | Definition |
| --- | --- |
| Detune | copy `k` of `n` sits at `pos_k = (2k − (n − 1)) / (n − 1)` in [−1, 1], `pos_k · detune` cents from the centre (two copies at ±`detune`, seven at 0, ±⅓, ±⅔, ±1 of it); ratio `2^(cents/1200)` through the pinned `libm` |
| Blend | the copies nearest the centre (the middle one for odd `n`, the middle two for even `n`) have weight 1, the others `blend`/100; with two copies there is no side copy and `blend` does nothing |
| Level | every gain divided by `sqrt(Σ weight²)`: the copies are uncorrelated, so their powers add and the loudness is the same for any count and blend |
| Spread | copy `k` is panned to `spread · pos_k` with a constant-power law scaled to **1 at the centre**: `(√2·cos θ, √2·sin θ)`, `θ = (pan + 1)·π/4`. At `spread` 0 both gains are exactly 1.0; at 100 % the outermost copies are exactly `(√2, 0)` and `(0, √2)`; the right half is the exact mirror of the left half, so a symmetric voice is symmetric to the last bit |

`detune`, `blend` and `spread` are smoothed over 10 ms like the other modulatable parameters: the module takes each copy's phase increment and gain pair at the two ends of the 32-sample segment and ramps them linearly across it (the increment like the pitch). A mix that has not moved comes from a one-entry cache per voice (a key of the count and the three values' bits), so a steady state does no `exp2` per segment.

**The sum.** A channel is `Σ_k y_k · c_k(t)` added in lane order from lane 0, starting from `+0.0`: `((((0 + t0) + t1) + t2) + …`. The horizontal sum never uses `reduce_add` (its order depends on the target): the products go through `to_array()` and are added one by one. The unused lanes hold a gain of exactly 0 and a value of exactly `+0.0`, so adding them changes nothing: one loop-free sum over all eight lanes equals the sum over the `n` copies bit for bit (`the_horizontal_sum_adds_in_lane_order_from_lane_zero_over_the_copies`). The `wide` subset is listed in the module docs of `unison.rs`; nothing from `clippy.toml`'s ban list (fused operations, `recip`, the polynomial transcendentals, `reduce_add`, `round_int`) is used, so the six-target golden sees the same bits.

### Stereo: the flag, spread 0, the voice pan

- **The flag** (lane SVP's): an enabled `wt` with `unison` ≥ 2, read from structural parameters only (`stereo::patch_needs_stereo`, `modules::wt::unison_count`). `spread` is modulatable, so it is not in the flag; **`spread` 0 on a stereo plan equals mono on both channels, bit for bit** (`spread_zero_equals_mono_on_both_channels_for_every_count_and_phase_mode`, and with neighbours, routes, effects, pans and steals): the two gains are exactly 1.0 at 0, so each channel is the mono sum. `StereoMode::Off` runs any patch on the mono path (a unison `wt` writes its mono mix to its one output), the reference those tests compare with.
- **The voice `pan`** is lane SVP's constant-power balance of the two channels (engine.md): a wide voice at pan −100 keeps only its left channel, at +100 only its right (`a_wide_voice_is_balanced_by_the_pan_and_a_hard_pan_keeps_only_one_side`).
- **Widening is symmetric.** A centred voice (key 48, seven copies across ±30 cents, one second) widens with `spread` without leaning: the correlation of the two channels falls strictly with the spread and the two sides carry the same power within beating noise (the sign of the small imbalance changes with key and count: noise, not bias).

| `spread` | 0 % | 10 % | 25 % | 50 % | 75 % | 100 % |
| --- | --- | --- | --- | --- | --- | --- |
| Correlation of L and R | 1.0000 | 0.9947 | 0.9672 | 0.8724 | 0.7261 | 0.5445 |
| Power L/R | 0.000 dB | +0.014 | +0.035 | +0.073 | +0.114 | +0.160 |

Two copies at 100 % are hard left and right, two unrelated signals: correlation −0.008, +0.011 dB (`spread_widens_a_centred_voice_symmetrically` holds the bounds: strictly falling, imbalance under 0.35 dB, correlation under 0.6 at 100 % with seven copies, under 0.15 in magnitude with two).

### Start phases and seeds

Copy `k` draws its start phase from a stream that is a function of the seed rules only, **never of a parameter value** (a drawn phase for copy `k` does not depend on `unison`, `detune`, `blend` or `spread`; the lanes beyond the count are drawn too):

| `phase_mode` | Start phase of copy `k` | Same every note? |
| --- | --- | --- |
| `free` | `derive(voice_seed, k)`: the render seed, the module id and the note index, as one oscillator's `free` | no |
| `seeded` | `derive(seeded_voice_seed(seed, module_key, note_index), k)`: the patch's own `seed` and the note | no (per note), the same every render |
| `reset` | `derive(module_key, k)`: the module id and the index, then the `phase` knob turns **all copies together** (adds `phase`/360 and wraps) | yes |

In `reset` mode the copies do not start at one phase (that would be a thump of seven aligned waves at every note on): they start at a fixed, repeatable spread of phases. **Decision, to record against the catalogue text** ("the start phase in reset mode"): `phase` keeps its meaning as the rotation of the whole cluster, which is what an FM carrier and its modulator want from it; the copies' relative phases are the module id's. No mode starts the copies aligned (see Limitations).

### What an audio route carries

**The mono mix of the copies** — `Σ_k y_k · g_k`, the same sum that is the left channel at `spread` 0 — taken after the AM gain and before the `level` ramp (the pre-level point of one oscillator), for every active voice, whatever the spread, the voice pan and the stereo flag are. Consequences, each a test (`a_route_leaving_a_stereo_wt_carries_the_mono_mix_whatever_the_spread`, `the_route_signal_is_the_mono_mix_after_am_and_before_level`): a modulator's route is the same whether the plan runs the stereo path or not and whether the spread is 0 or 100; `level` 0 makes a silent unison operator that still modulates; the carrier of such a route (a plain `wt` with `pm`) is the same on both channels. **Why not the left channel or the mean of the two:** the pan is a placement, not part of the wave, and PM/AM want the wave; the left channel would make a modulator's sound change when the spread moves. A *carrier* `wt` with unison gets one `pm` block (and one `am` block) for all its copies.

An `osc` (or any mono generator) beside a stereo `wt` lands on both channels as it does in mono (`an_osc_beside_a_stereo_wt_lands_on_both_channels_as_it_does_in_mono`: the plain loop and the audio-route path, in either patch order), and a filter after a stereo `wt` runs once per channel (lane SVP's twin instances).

### Plan swaps, real time, tails

- A change of the copy count is a structural edit (the plan crossfades over 256 frames; the state is carried: one oscillator → copies starts every copy at the oscillator's phase, copies → one continues from the centre copy); an edit of `detune`, `blend` or `spread` is not (instant, the copies keep going; `tests/wt_unison_swap.rs`). A swap to an equal plan changes no bit.
- **No allocation:** `process` and the swaps under `assert_no_alloc`, with every voice stolen and a stereo `wt` in audio routes; 30 count flips with a return ring of one. The per-voice state is a fixed array of `UnisonVoice` (the lanes) next to the plain `Voice`s.
- **Subnormals:** none after a 40 s release tail (`wt_unison_tail.rs`: the lanes' phase accumulators and feedback histories are inspected as well as the output), the render is identical under every FTZ mode, **latency 0** (the first sample is every copy's table at its start phase).

### Aliasing, measured

The standard measure (report 02: the power outside the harmonics over the power on them; `enth_dsp::alias`) needs one fundamental, and a detuned unison has none, so two measurements (`tests/wt_unison_alias.rs`; `cargo test --release -p enth-engine --test wt_unison_alias -- --nocapture` prints the tables; dB, 32,768 samples after the attack):

- **Detune 0:** every copy plays the key's pitch (only the start phases differ), the sum is periodic and the measure applies to the whole render. Unison 2 and 7 against a single `wt`, every table at the saw-like and square frames, keys 24, 48, 72, 84, 96 and 105: worst single −93.4 dB, worst unison **−91.4 dB** (`metal`, key 72, seven copies; the measure's own floor is about −94 dB), inside the bound −90 dB recorded for one `wt` above; cell by cell the unison is within ±2 dB of the single, the sign of the difference is mixed (the aliasing powers of the copies add and so do their signal powers).
- **Detune 100 cents:** the outermost copies are the worst case for the mip-level choice (the highest copy has the fewest partials under the limit). Each copy is a plain voice at its own pitch to the last bit (the lanes' tests), so its aliasing is a single `wt`'s at that pitch; a single `wt` at the key −100/0/+100 cents is measured across the same grid: worst **−93.0 dB** (`metal`, key 72, −100 cents). The sum of copies at different frequencies has its aliasing and its signal powers add, so its ratio lies between the copies' ratios and cannot exceed the worst of them.

So copies alias no worse than a single `wt`: the worst cell of any measurement is −91.4 dB against the −90 dB bound, and the phase-modulated, warped and fed-back paths are the single `wt`'s (the same code per lane).

### Unison cost, measured

Callgrind instructions (`cargo xtask bench --bench wt`, `benches/wt.rs`; 1 s at 48 kHz, 16 voices, held chord of `common::prepare`; detune 20 cents, blend 100 %, spread 50 %; the `wt` is `wt2`, so `osc1` and the envelopes are in every number), over the `engine` bench's 37,326,575 for the same 16 voices (this branch; main's is 37,290,522):

| Case | Instructions | Budget (+10 %) | Per frame | Per sample and voice over `engine` |
| --- | --- | --- | --- | --- |
| 2 copies (the reese) | 260,666,680 | 286,734,000 | 5,431 | 290.8 |
| **7 copies (the supersaw)** | **314,516,960** | 345,969,000 | **6,552** | 360.9 (51.6 per copy) |
| 7 copies, PM from `osc1` | 427,763,006 | 470,540,000 | 8,912 | 508.4 |
| 7 copies, every optional path (formant table moving, bend warp, PM, AM, 50 % feedback) | 615,324,682 | 676,858,000 | 12,819 | 752.6 |
| 8 copies, every optional path | 635,368,406 | 698,906,000 | 13,237 | 778.7 |

**16 voices × 7 copies, the brief's number: 6,552 instructions per frame with the engine skeleton and an `osc` in it (5,775 for the `wt` alone) against G1's 12,000 per frame** — 55 % of it. Seven plain `wt`s cost 7 × 97.9 = 685 per sample and voice, so the lanes are 1.9× cheaper (the plain `wt` is 112,494,140 for 16 voices on this branch); the stereo path adds the right channel's mix and balance. The cost is nearly flat in the count: the lanes compute eight copies whatever `unison` is, so 2 copies cost 290.8, more than two plain `wt`s (196) — **the reese is the one case where a scalar path would win**, and it is not built (the lanes equal plain `Voice`s bit for bit, so adding it later changes no sound). The worst case, eight copies with every optional path on 16 voices, is 13,237 per frame: above G1's bound by itself, and G2's reference patch (lane G2B) would not use it.

The engine seam costs the existing paths almost nothing against main (`wt` plain 16 voices 112,494,140 vs 112,263,005: +0.2 %; with PM 121,792,982 vs 121,614,216: +0.15 %; everything on 170,783,237 vs 170,628,380: +0.1 %; one voice 8,626,495 vs 8,552,995: +0.9 %, a fixed ~24 instructions per segment; the `engine` bench 16 voices +0.1 %). The first version of the lanes was no faster than scalar copies (94.5 instructions per copy-sample); the index math on `i32x8`, 16-byte tap loads with two `f32x4` transposes, a loop-free sum and skipping the wrap without modulation brought it to 51.6.

### Goldens and the taste check

Seven cases in `tests/golden/cases/wt-unison-*.toml`: `wt-unison-supersaw` (7 copies, the saw frame, a two-voice chord), `wt-unison-reese` (2 copies, detune automated, spread 0: **equals the mono reference on both channels**, checked in `tests/golden/tests/stereo_path.rs`), `wt-unison-reese-wide` (the same at spread 100 % and automated to 0), `wt-unison-phase-modes` (4 copies in `reset`, `free` and `seeded`), `wt-unison-routes` (a stereo `wt` beside an `osc`, a PM carrier of the `osc` and the PM source of a plain `wt`), `wt-unison-full` (8 copies with the formant table, bend warp, feedback, PM, AM, moving position and spread) and `wt-unison-chord` (5 copies, `free`, voice pan, polyphony 3, a release tail). The 87 earlier cases do not move. WAVs (7-copy supersaw on a three-note chord, a 2-copy reese with its detune slowly swept by an LFO, the same reese with spread 100 %), rendered with `enth render`, never played: `~/code/enth-scratch/s2/uni/wav/` (patches in `../patches/`); the reese pair is mono (correlation 1.0000) and wide (0.08).

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
- **Unison costs the same for 2 copies as for 8** (the lanes compute eight): the reese is 2.7× the price of a plain `wt` and more than two plain `wt`s would cost ([Unison cost](#unison-cost-measured)). A scalar path for 2–3 copies would be bit-identical (the lanes equal plain `Voice`s) and cheaper; it is not built.
- **No `phase_mode` starts the copies aligned**: `reset` starts them at a repeatable spread of phases (rotated together by `phase`), `free` and `seeded` at random ones. A thump of aligned copies at the attack, as some synths have, is not available.
- **Unison shares one phase-modulation block, one warp, one position and one feedback amount** among the copies; per-copy feedback *state* is separate (each copy is its own feedback loop), but a modulator cannot treat the copies differently (no per-copy PM, no "random position per copy").
- **`blend` does nothing with 2 copies** (no side copy beside two centre ones) and the stereo spread of 2 copies is hard left/right at 100 %, whatever the detune.
- **A route leaving a unison `wt` is mono** (the mix before the spread); a stereo route would need a stereo `AudioRoutes` (not asked for).
- **Spread is a pan of the copies; the voice effects after it run twice** (lane SVP's twin instances), so a heavy `voice_fx` chain doubles in cost on a unison patch.

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/wt.rs` (lane CAT's F1m table, [above](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/wt.rs` (the tables, the warps, the voice); the shared generator is `crates/enth-dsp/src/wavetable.rs` |
| Unison kernel | `crates/enth-dsp/src/wt/unison.rs` (the lanes, the mix; its module docs and unit tests) |
| Engine module | `crates/enth-engine/src/modules/wt.rs` |
| Tests | `crates/enth-engine/tests/wt.rs` (closed forms, aliasing, the contract), the unit tests in `crates/enth-dsp/src/wt.rs`; lane UNI's: `tests/wt_unison.rs` (the flag, spread 0 = mono, widening, routes, neighbours), `wt_unison_swap.rs` (swaps, allocation), `wt_unison_tail.rs` (40 s tail, FTZ), `wt_unison_alias.rs` (aliasing tables), the unit tests in `unison.rs` and in `modules/wt.rs` (seeds), `tests/golden/tests/stereo_path.rs` |
| Instruction budget | `crates/enth-engine/benches/wt.rs` |
| Golden cases | `tests/golden/cases/wt-*.toml` (the seven `wt-unison-*` are lane UNI's) |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets (CI's golden workflow checks them after merge; locally `cargo xtask golden`: 87 cases ok on x86-64 Linux, the 73 existing unchanged); FTZ-independent (`the_render_is_the_same_under_every_ftz_mode`); zero subnormals after a 40 s tail (`a_40_s_release_tail_has_no_subnormals_and_every_voice_ends`); latency 0; a budget of the measured count plus 10 % (above); two instances keep separate state (`a_wt_as_a_source_feeds_another_wt_and_the_two_keep_separate_state`), a self-route is refused (`a_wt_routed_to_itself_is_refused`, RTE's `audio_cycle`); `mix` does not apply (a generator); aliasing measured spectrally (above), the bound recorded in the test and here. No port, so no `THIRD_PARTY_NOTICES.md` entry.

**WT's own row** ([phase-s2.md](../phase-s2.md#acceptance-per-lane)): the audio-route goldens (`wt-pm-fm`, `wt-am`, `wt-fm-chain`, `wt-ring-mod`), the index and feedback sweeps inside the recorded bound ([Aliasing](#aliasing-measured-and-bound)), and the closed-form spectral checks ([Measured](#closed-form-spectral-checks-cratesenth-enginetestswtrs)).

**UNI's own row** ([phase-s2.md](../phase-s2.md#acceptance-per-lane)): *lanes equal a scalar reference bit for bit* (`the_lanes_equal_plain_voices_bit_for_bit_for_every_warp_count_and_modulation` in `unison.rs`: plain `Voice`s, a scalar mix and the same fixed-order sum, six warps and every count; the six-target hash is the `wt-unison-*` goldens, which use only `wide`'s exact subset, and `cargo xtask wasm-check` renders them on `wasm32-wasip1`); *a sub-voice's seed never depends on a parameter value* (`a_copys_start_phase_never_depends_on_a_parameter_value` and `the_start_phases_follow_the_seed_rules_and_nothing_else` in `modules/wt.rs`, over every other knob and all three phase modes); *16 voices × 7 inside G2's budget* (6,552 instructions per frame including the skeleton against G1's 12,000, [Unison cost](#unison-cost-measured)); *spread 0 equals mono* (both channels, bit for bit, every count and phase mode, with neighbours, routes, effects and steals). Also: `unison` 1 renders exactly as `wt` without unison (`with_unison_off_detune_blend_and_spread_do_nothing`), copies alias no worse than a single `wt` ([Aliasing](#aliasing-measured)), and `ENGINE_VERSION` is not bumped (the coordinator's, as for the rest of S2a).
