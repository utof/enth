# OTT (`ott`; `enth-dsp::ott`, `enth-engine::modules::ott`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane OTT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `ott`, discriminant 9, section `fx`, id prefix `ott` (ott1, ott2…), rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: three-band upward and downward compression after the voices, on the compressor's detector; `mix`.
- Sources to start from: research report 05 §4 (multiband, OTT); the compressor's detector (`docs/dsp/compressor.md`) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane OTT implements, generated from `crates/enth-patch/src/catalogue/ott.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `input` gain; a Linkwitz–Riley split at `low_split` and `high_split`; per band downward compression above a threshold and upward below it (a gain cap and a floor, so silence is not lifted), scaled by `downward` and `upward` and both by `depth`; `time` scales every band's attack and release; `low_gain`, `mid_gain`, `high_gain`; the sum; `output`; `mix` against the input.
- **The bands' thresholds, ratios and times are lane OTT's constants** (report 04 §2's preset values, to verify by measuring Xfer OTT or reading Ableton's preset), not parameters.
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

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/ott.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/ott.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/ott.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/ott.rs` | placeholder |
| Golden cases | `tests/golden/cases/ott-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane OTT).
