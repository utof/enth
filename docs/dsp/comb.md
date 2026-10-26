# Comb filter (`comb`; `enth-dsp::comb`, `enth-engine::modules::comb`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane FLT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `comb`, discriminant 10, section `voice_fx`, id prefix `comb` (comb1, comb2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a keytracked feedback comb filter per voice (teeth at the note's harmonics; `mix` with the dry signal adds the notches between them); `mix`.
- Sources to start from: research report 05 §5 (filters beyond the SVF) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/comb.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Pitch:** `261.63 Hz · 2^((semi + keytrack · (pitch − 60)) / 12)` with `keytrack` as a fraction (svf's key-60 reference), so at 100 % and `semi` 0 the teeth sit on the note's harmonics and follow glides.
- **Structure:** `y = x + feedback · lowpass(y[n − period])` (the low-pass is `damping`, open at 20 kHz; negative feedback puts the teeth between the harmonics), then `mix` (0 % bit for bit). A noise burst and feedback near 99 % make a Karplus–Strong pluck; an LFO on `semi` flanges.
- **Defaults:** a gentle useful setting: feedback 50 %, damping 10 kHz.

### `comb` — Comb filter (voice_fx, ids `comb1`, `comb2`…; 5 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `semi` | semitones | -48st … +48st | linear | 0st | mod, auto | basic | The comb's pitch in semitones from the played note (from middle C when keytrack is 0 %): +12st puts the teeth an octave up; an LFO here flanges. |
| `keytrack` | percent | 0% … 100% | linear | 100% | auto | intermediate | How much the comb's pitch follows the played note: 100 % keeps the teeth on the note's harmonics, 0 % holds a fixed pitch. |
| `feedback` | percent | -99% … 99% | linear | 50% | mod, auto | basic | How strongly the comb rings: higher is more metallic and longer; negative puts the resonances between the harmonics, hollow and an octave down. |
| `damping` | hz | 500Hz … 20kHz | log | 10kHz | mod, auto | intermediate | A low-pass inside the feedback loop: lower makes the high teeth die faster, for a softer, plucked ring; 20 kHz leaves it bright. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; around 50 % the dry signal adds notches between the teeth. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/comb.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/comb.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/comb.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/comb.rs` | placeholder |
| Golden cases | `tests/golden/cases/comb-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): an `osc` sine through the module at a sweep of keys, measured against the sine's fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane FLT).
