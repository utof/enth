# Distortion (`dist`; `enth-dsp::dist`, `enth-engine::modules::dist`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane DST owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `dist`, discriminant 7, section `voice_fx`, id prefix `dist` (dist1, dist2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: distortion per voice: soft and hard clipping, sine and triangle folds, asymmetric shaping, bitcrush and downsample, with ADAA-1 and zero-latency 2x or 4x IIR oversampling; drive, bias, tone and `mix`.
- Sources to start from: research report 05 §2 (ADAA-1, the shaper family) and §7 (oversampling); the half-band IIR from Valenzuela and Constantinides (1983), coefficients designed from the paper (`enth-dsp/src/resample.rs`) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane DST implements, generated from `crates/enth-patch/src/catalogue/dist.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `keep_lows` splits off the band below it (no split at 20 Hz), which bypasses the curve; `drive` and `bias` go into the `mode` curve (oversampled by `oversample` for the clip and fold curves; `bitcrush` reads `bits`, `downsample` reads `rate`, and neither is oversampled); a DC blocker; `tone` (a low-pass, open at 20 kHz); then `mix` against the input (0 % bit for bit).
- **Defaults:** a gentle useful setting, 6 dB into the soft curve, 2× oversampling. `oversample` is structural (it rebuilds the plan, so a factor change never clicks).
- Report 04's S6 "downsample, MSEG on drive" is `mseg → dist.rate` here: the amount of downsampling is `rate`.

### `dist` — Distortion (voice_fx, ids `dist1`, `dist2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `soft`, `hard`, `asym`, `sine_fold`, `tri_fold`, `bitcrush`, `downsample` | enum | soft | auto | basic | The distortion curve: soft or hard clipping, asymmetric, sine or triangle folding, bitcrush or downsample. |
| `drive` | db | 0dB … 48dB | linear | 6dB | mod, auto | basic | Gain into the curve: more drive is more distortion; route an LFO or MSEG here for rhythmic grit. |
| `bias` | percent | -100% … 100% | linear | 0% | mod, auto | intermediate | An offset added before the curve, so the top and bottom distort differently: adds even harmonics and a thicker, rasping tone. |
| `bits` | none | 1 … 16 | linear | 8 | mod, auto | intermediate | Bit depth in bitcrush mode, fractions allowed: 16 is clean, 8 is gritty, 2 is a buzz. Other modes ignore it. |
| `rate` | hz | 100Hz … 48kHz | log | 6kHz | mod, auto | intermediate | The held sample rate in downsample mode: lower is more metallic and aliased; at the engine's rate or above it does nothing. Other modes ignore it. |
| `tone` | hz | 100Hz … 20kHz | log | 20kHz | mod, auto | intermediate | A low-pass after the curve that tames fizz: lower is darker; 20 kHz leaves it open. |
| `keep_lows` | hz | 20Hz … 1kHz | log | 20Hz | auto | intermediate | Frequencies below this bypass the distortion, so the sub stays clean and only the mids distort; at 20 Hz everything is distorted. |
| `oversample` | none | `off`, `x2`, `x4` | enum | x2 | structural | advanced | How much the clipping and folding curves are oversampled to cut aliasing: off, 2x or 4x (no latency either way); changing it rebuilds the plan. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values blend in the clean signal (parallel distortion). |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/dist.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/dist.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/dist.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/dist.rs` | placeholder |
| Golden cases | `tests/golden/cases/dist-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): an `osc` sine through the module at a sweep of keys, measured against the sine's fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane DST).
