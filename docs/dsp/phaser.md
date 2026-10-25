# Phaser (`phaser`; `enth-dsp::phaser`, `enth-engine::modules::phaser`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane MOD owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `phaser`, discriminant 14, section `fx`, id prefix `phaser` (phaser1, phaser2…), a time-based effect, rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a stereo phaser (a chain of modulated all-pass filters) on MOD's modulated-delay core; `mix`.
- Sources to start from: research report 05 §3 (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/phaser.rs` | S0d's placeholder; lane CAT writes the table before F1m |
| DSP core | `crates/enth-dsp/src/phaser.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/phaser.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/phaser.rs` | placeholder |
| Golden cases | `tests/golden/cases/phaser-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane MOD).
