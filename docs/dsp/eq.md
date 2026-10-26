# EQ (`eq`; `enth-dsp::eq`, `enth-engine::modules::eq`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane EQ owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `eq`, discriminant 15, section `fx`, id prefix `eq` (eq1, eq2…), rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a parametric equaliser after the voices, wrapping `enth-dsp`'s biquad (lane BQ's file), with per-segment coefficient ramps; `mix`.
- Sources to start from: research report 05 §4; `docs/dsp/biquad.md` (lane BQ's biquad, within 0.1 dB of the cookbook) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane EQ implements, generated from `crates/enth-patch/src/catalogue/eq.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Bands in series, in table order:** low cut, low shelf, bell 1, bell 2, high shelf, high cut. The cuts are 12 dB/octave Butterworth sections, **off at the ends of their ranges** (`low_cut` 10 Hz, `high_cut` 20 kHz: skip the stage); shelves and bells are the cookbook's (within 0.1 dB, EQ's acceptance); a band at 0 dB is skipped. A bell at high Q and −24 dB is a notch.
- **Defaults:** near-transparent: every gain 0 dB, both cuts off.

### `eq` — EQ (fx, ids `eq1`, `eq2`…; 13 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `low_cut` | hz | 10Hz … 1kHz | log | 10Hz | mod, auto | basic | Removes everything below this frequency (12 dB/octave), clearing mud or the sub from effects; at 10 Hz it is off. |
| `low_freq` | hz | 20Hz … 1kHz | log | 100Hz | mod, auto | intermediate | The corner of the low shelf. |
| `low_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts everything below the low shelf's corner: weight and warmth. |
| `bell1_freq` | hz | 20Hz … 20kHz | log | 400Hz | mod, auto | intermediate | The centre of the first bell. |
| `bell1_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts a band around the first bell's centre: a cut near 300-500 Hz cleans boxiness. |
| `bell1_q` | none | 0.1 … 20 | log | 1 | mod, auto | intermediate | How narrow the first bell is: 0.7 is broad, 10 a narrow notch or peak. |
| `bell2_freq` | hz | 20Hz … 20kHz | log | 2.5kHz | mod, auto | intermediate | The centre of the second bell. |
| `bell2_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts a band around the second bell's centre: presence and bite around 2-5 kHz. |
| `bell2_q` | none | 0.1 … 20 | log | 1 | mod, auto | intermediate | How narrow the second bell is: 0.7 is broad, 10 a narrow notch or peak. |
| `high_freq` | hz | 1kHz … 20kHz | log | 8kHz | mod, auto | intermediate | The corner of the high shelf. |
| `high_gain` | db | -24dB … 24dB | linear | 0dB | mod, auto | basic | Boosts or cuts everything above the high shelf's corner: air and sparkle, or a darker top. |
| `high_cut` | hz | 1kHz … 20kHz | log | 20kHz | mod, auto | basic | Removes everything above this frequency (12 dB/octave); at 20 kHz it is off. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/eq.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/eq.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/eq.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/eq.rs` | placeholder |
| Golden cases | `tests/golden/cases/eq-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane EQ).
