# Delay (`delay`; `enth-dsp::delay`, `enth-engine::modules::delay`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane DLY owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `delay`, discriminant 16, section `fx`, id prefix `delay` (delay1, delay2…), a time-based effect, rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a stereo delay after the voices, its time in ms or synced in beats to the render's tempo; `mix`.
- Sources to start from: research report 05 §4 (delay); lane TMP's tempo for the synced time (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/delay.rs` | S0d's placeholder; lane CAT writes the table before F1m |
| DSP core | `crates/enth-dsp/src/delay.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/delay.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/delay.rs` | placeholder |
| Golden cases | `tests/golden/cases/delay-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane DLY).
