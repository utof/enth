# Wavetable (`wt`; `enth-dsp::wt`, `enth-engine::modules::wt`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane WT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `wt`, discriminant 8, section `generators`, id prefix `wt` (wt1, wt2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a wavetable oscillator on phase-read waves: generated tables, phase warps, phase and amplitude modulation from another generator through audio routes (`pm`, `am`; bipolar AM is ring modulation), `feedback`, and later unison on `wide` lanes (lane UNI).
- Sources to start from: research report 05 §1 (PM, AM and ring modulation between generators, feedback) and §7 (unison on `wide` lanes); report 06 §3.1 (the audio-route contract) and §3.2 (unison) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/wt.rs` | S0d's placeholder; lane CAT writes the table before F1m |
| DSP core | `crates/enth-dsp/src/wt.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/wt.rs` | stub (silent); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/wt.rs` | placeholder |
| Golden cases | `tests/golden/cases/wt-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): render the generator at a pitch sweep (and with PM from another generator, at a high index), measured against its fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane WT).
