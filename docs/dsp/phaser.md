# Phaser (`phaser`; `enth-dsp::phaser`, `enth-engine::modules::phaser`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane MOD owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `phaser`, discriminant 14, section `fx`, id prefix `phaser` (phaser1, phaser2…), a time-based effect, rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a stereo phaser (a chain of modulated all-pass filters) on MOD's modulated-delay core; `mix`.
- Sources to start from: research report 05 §3 (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane MOD implements, generated from `crates/enth-patch/src/catalogue/phaser.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** `stages` first-order TPT all-passes (an odd count is allowed), their corner swept around `center` by `depth` (100 % = ±3 octaves) at `rate` or one cycle per `beats` with `sync`; the right sweep lags by `width` × 180°; `feedback` around the chain.
- **The effect's own output is the dry signal plus the chain, level-matched** (the sum makes the notches): `wet = chain(v)` with `v[n] = (1 − |feedback|) · x[n] + feedback · wet[n − 1]`, and the output at `mix` 100 % is `(x + wet) / 2`; `mix` 0 % is the input bit for bit. The loop's input is scaled by `1 − |feedback|`, so the chain's gain peaks at 0 dB at every `feedback` (the bare loop peaks at `1 / (1 − |feedback|)`, +40 dB at 99 %), and the level-matched sum is 0 dB where the dry and the wet are in phase and not +6 dB: a freshly added phaser changes a full-scale saw's level by at most about 3 dB (catalogue rule 3). Lane MOD measures the level and may choose another normalisation with the same bound. Global: only global sources modulate it.
- **Defaults:** a gentle useful setting, six stages around 1 kHz, light feedback.

### `phaser` — Phaser (fx, ids `phaser1`, `phaser2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `stages` | none | 2 … 12 | int | 6 | auto | intermediate | How many all-pass filters are chained: every two make one notch, so more stages give more notches and a stronger sweep. |
| `rate` | hz | 0.01Hz … 20Hz | log | 0.3Hz | mod, auto | basic | How fast the notches sweep, in cycles per second, when tempo sync is off. |
| `sync` | none | off, on | toggle | off | auto | intermediate | On: one sweep lasts beats of the song's tempo, and rate is ignored. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar`, `two_bars_triplet`, `bar_dotted`, `two_bars`, `two_bars_dotted`, `four_bars`, `eight_bars`, `sixteen_bars` | enum | bar | auto | intermediate | How long one sweep lasts when tempo sync is on, as a note division. |
| `depth` | percent | 0% … 100% | linear | 50% | mod, auto | basic | How far the notches sweep around the center: 100 % is three octaves either way. |
| `center` | hz | 50Hz … 10kHz | log | 1kHz | mod, auto | intermediate | The frequency the notches sweep around; with depth at 0 % a static, comb-like color here. |
| `feedback` | percent | -99% … 99% | linear | 30% | mod, auto | intermediate | Sends the output back into the chain: sharper, ringing notches; negative moves the peaks between them. |
| `width` | percent | 0% … 100% | linear | 50% | auto | intermediate | How far apart the left and right sweeps run: 0 % is mono, 100 % opposite. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full phaser (the dry sound plus the swept all-passes). |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/phaser.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/phaser.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/phaser.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/phaser.rs` | placeholder |
| Golden cases | `tests/golden/cases/phaser-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane MOD).
