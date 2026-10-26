# Formant filter (`formant`; `enth-dsp::formant`, `enth-engine::modules::formant`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane FLT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `formant`, discriminant 11, section `voice_fx`, id prefix `formant` (formant1, formant2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a formant (vowel) filter per voice, the vowel set by its first two formant frequencies (a vowel plane; a vowel path is one modulator routed to both); `mix`.
- Sources to start from: research report 05 §5; report 04's vowel table (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane FLT implements, generated from `crates/enth-patch/src/catalogue/formant.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Two formant frequencies, not a vowel knob:** F1 (openness) and F2 (frontness) in Hz place any vowel; a straight move between vowels is one modulator routed to both at two depths. No single vowel path makes report 04's moves (a "yoi" swings F2 while F1 stays low; "a → o" lowers both). The vowel table is the knob settings: i 240/2400, e 390/2300, a 850/1610, ɑ 750/940, o 360/640, u 250/595 (Catford 2001, report 04 §1). FLT's acceptance check: peaks at these settings.
- `shift` moves F1, F2 and a fixed third formant (2.7 kHz) together; `resonance` sets the peaks' Q; coefficients once per segment.
- **Defaults:** a gentle useful setting, the vowel "a" at a moderate resonance (a vowel filter fully wet is never transparent).

### `formant` — Formant filter (voice_fx, ids `formant1`, `formant2`…; 5 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `f1` | hz | 100Hz … 1kHz | log | 850Hz | mod, auto | basic | The first formant: how open the vowel is, from 'ee' and 'oo' (about 250 Hz) to 'ah' (about 850 Hz). |
| `f2` | hz | 400Hz … 4kHz | log | 1.61kHz | mod, auto | basic | The second formant: how far forward the vowel is, from 'oo' (about 600 Hz) through 'ah' (1.6 kHz) to 'ee' (2.4 kHz); sweep it for 'yoi' and 'wow'. |
| `shift` | semitones | -24st … +24st | linear | 0st | mod, auto | intermediate | Moves every formant up or down together, keeping the vowel: a smaller, brighter voice or a bigger, darker one. |
| `resonance` | percent | 0% … 100% | linear | 50% | mod, auto | intermediate | How narrow and strong the formant peaks are: low is a soft, breathy vowel, high a sharp, whistling one. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged; lower values keep some of the unfiltered body. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/formant.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/formant.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/formant.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/formant.rs` | placeholder |
| Golden cases | `tests/golden/cases/formant-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): an `osc` sine through the module at a sweep of keys, measured against the sine's fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane FLT).
