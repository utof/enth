# Ladder filter (`ladder`; `enth-dsp::ladder`, `enth-engine::modules::ladder`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane FLT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `ladder`, discriminant 17, section `voice_fx`, id prefix `ladder` (ladder1, ladder2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a four-pole ladder filter per voice (a TPT Moog ladder with a saturator), low-pass at 24 or 12 dB/octave, band-pass or high-pass from the stage taps; the first item on the cut list.
- Sources to start from: research report 05 §5 (the TPT four-pole ladder) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/ladder.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Mirrors `svf`'s table** where the meanings match (keys, units, ranges, defaults), so a patch can swap one for the other: the effective cutoff is `cutoff · 2^(keytrack · (pitch − 60) / 1200)`; `drive` is gain into the input saturator, level-compensated; `resonance` self-oscillates near the top (FLT records where). `mode` taps the stages for 24 or 12 dB/octave low-pass, band-pass or high-pass; a diode ladder can be appended later.
- **Defaults:** a gentle useful setting, as a new `svf` is: a 24 dB low-pass at 2 kHz.

### `ladder` — Ladder filter (voice_fx, ids `ladder1`, `ladder2`…; 6 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `lp24`, `lp12`, `bp`, `hp` | enum | lp24 | auto | basic | The response: 24 or 12 dB/octave low-pass, band-pass or high-pass. |
| `cutoff` | hz | 20Hz … 20kHz | log | 2kHz | mod, auto | basic | The corner frequency: lower is darker for a low-pass; route an envelope or LFO here for sweeps and wobbles. |
| `resonance` | percent | 0% … 100% | linear | 10% | mod, auto | basic | Emphasis at the cutoff; near the top the ladder rings on its own (self-oscillation) and the lows thin out, as on a Moog. |
| `keytrack` | percent | 0% … 100% | linear | 0% | auto | intermediate | How much the cutoff follows the played note: 100 % moves it an octave per octave played. |
| `drive` | db | 0dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain into the ladder's saturator, normalised back after: grit and a thicker, compressed tone. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values blend the dry signal back in. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/ladder.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/ladder.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/ladder.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/ladder.rs` | placeholder |
| Golden cases | `tests/golden/cases/ladder-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): an `osc` sine through the module at a sweep of keys, measured against the sine's fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane FLT).
