# Delay (`delay`; `enth-dsp::delay`, `enth-engine::modules::delay`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane DLY owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `delay`, discriminant 16, section `fx`, id prefix `delay` (delay1, delay2…), a time-based effect, rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a stereo delay after the voices, its time in ms or synced in beats to the render's tempo; `mix`.
- Sources to start from: research report 05 §4 (delay); lane TMP's tempo for the synced time (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane DLY implements, generated from `crates/enth-patch/src/catalogue/delay.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Time:** `beats` of the tempo with `sync` (exact in beats across tempo changes), else `time`; never above 2 s (the rings are sized from `time`'s maximum at `prepare`), so a synced time past 2 s (a bar below 120 BPM) plays at 2 s. `beats` offers the shared list up to a bar (`DELAY_BEATS`, the same indices as `Beats`).
- **Loop:** each repeat passes `low_cut` and `high_cut` and returns at `feedback` (a soft clip keeps 100 % bounded); `ping_pong` crosses the repeats.
- **The effect's own output is the dry signal plus the echoes**, so `mix` scales the echoes against an unchanged dry signal (100 %: the first echo as loud as the dry, filtered); 0 % is the input bit for bit. Time changes glide or crossfade, never click. This is the one documented exception to the level-bounded rule for feedback effects (catalogue rule 3): the repeats are separate events in time that fall away with `feedback` (a soft clip bounds 100 %), the dry signal passes unchanged, and nothing is normalised.
- **Defaults:** a gentle useful setting: a synced dotted eighth, feedback 35 %, low cut 150 Hz, high cut 6 kHz.

### `delay` — Delay (fx, ids `delay1`, `delay2`…; 8 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `sync` | none | off, on | toggle | on | auto | basic | On: the delay time is beats of the song's tempo. Off: it is time, in milliseconds. |
| `time` | ms | 1ms … 2s | log | 300ms | auto | intermediate | The delay time when tempo sync is off: under 30 ms thickens, 80-150 ms is a slapback, longer is an echo. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar` | enum | eighth_dotted | auto | basic | The delay time when tempo sync is on, as a note division up to a bar: 1/8D is the classic bouncing echo. |
| `feedback` | percent | 0% … 100% | linear | 35% | mod, auto | basic | How much of each echo returns for another: more is more repeats; 100 % repeats for ever. |
| `ping_pong` | none | off, on | toggle | off | auto | intermediate | Bounces the echoes between the left and right channels. |
| `low_cut` | hz | 20Hz … 2kHz | log | 150Hz | mod, auto | intermediate | Removes lows from every repeat, so echoes stay off the bass; each repeat gets thinner. |
| `high_cut` | hz | 1kHz … 20kHz | log | 6kHz | mod, auto | intermediate | Removes highs from every repeat: each echo gets darker, like tape. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How loud the echoes are against the dry sound, which always passes unchanged: 100 % makes the first echo as loud as the dry; 0 % is no echo at all. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/delay.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/delay.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/delay.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/delay.rs` | placeholder |
| Golden cases | `tests/golden/cases/delay-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane DLY).
