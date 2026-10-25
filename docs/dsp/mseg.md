# MSEG (`mseg`; `enth-dsp::mseg`, `enth-engine::modules::mseg`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane MSEG owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `mseg`, discriminant 12, section `modulators`, id prefix `mseg` (mseg1, mseg2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a multi-segment envelope with a fixed point count (lane CAT decides it), a curve per segment, a length synced in beats or in ms, loop, one-shot and sustain-loop modes.
- Sources to start from: research report 05 §6 (modulators); lane CAT decides the point count and the envelope-time form (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/mseg.rs` | S0d's placeholder; lane CAT writes the table before F1m |
| DSP core | `crates/enth-dsp/src/mseg.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/mseg.rs` | stub (outputs 0); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/mseg.rs` | placeholder |
| Golden cases | `tests/golden/cases/mseg-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): not an audio path: a modulator's own aliasing is not measured, but its effect on a parameter it sweeps (zipper noise at segment steps) is listened for, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane MSEG).
