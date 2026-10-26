# Chorus (`chorus`; `enth-dsp::chorus`, `enth-engine::modules::chorus`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane MOD owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `chorus`, discriminant 13, section `fx`, id prefix `chorus` (chorus1, chorus2…), a time-based effect, rate global. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a stereo chorus with ensemble and flanger modes on one modulated-delay core (a through-zero flanger, with its fixed dry delay, can be appended as a mode later); `mix`.
- Sources to start from: research report 05 §3 (modulation effects: one modulated-delay core, a through-zero flanger with a fixed dry delay) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane MOD implements, generated from `crates/enth-patch/src/catalogue/chorus.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **Signal path:** taps sweep around `delay` by `depth` at `rate` (or one cycle per `beats` with `sync`); the right sweep lags the left by `width` × 180°; `ensemble` is three taps per channel on a slow and a fast sweep (Plaits' ensemble). `feedback` returns the taps into the delay.
- **The effect's own output is the dry signal plus the taps**, so `mix` 100 % is the full chorus or flange and `mix` scales the taps against an unchanged dry signal; 0 % is the input bit for bit. Global: only global sources modulate it. A through-zero flanger can be appended to `mode` later.
- **Defaults:** a gentle useful setting, a slow, wide chorus.

### `chorus` — Chorus (fx, ids `chorus1`, `chorus2`…; 9 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `chorus`, `ensemble`, `flanger` | enum | chorus | auto | basic | The algorithm: chorus, ensemble or flanger. |
| `rate` | hz | 0.01Hz … 20Hz | log | 0.5Hz | mod, auto | basic | How fast the delay sweeps, in cycles per second, when tempo sync is off: slow is a gentle shimmer, fast a warble. |
| `sync` | none | off, on | toggle | off | auto | intermediate | On: one sweep lasts beats of the song's tempo, and rate is ignored. |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar`, `two_bars_triplet`, `bar_dotted`, `two_bars`, `two_bars_dotted`, `four_bars`, `eight_bars`, `sixteen_bars` | enum | bar | auto | intermediate | How long one sweep lasts when tempo sync is on, as a note division. |
| `depth` | percent | 0% … 100% | linear | 50% | mod, auto | basic | How far the delay sweeps: more is a deeper wobble in pitch (chorus) or a wider sweep of the notches (flanger). |
| `delay` | ms | 0.1ms … 30ms | log | 7ms | mod, auto | intermediate | The delay the sweep moves around: about 5-20 ms for a chorus, 0.5-5 ms for a flanger; an LFO here is a manual flange. |
| `feedback` | percent | -99% … 99% | linear | 0% | mod, auto | intermediate | Sends the delayed sound back into the delay: a ringing, metallic flange; negative gives hollower notches. |
| `width` | percent | 0% … 100% | linear | 100% | auto | intermediate | How far apart the left and right sweeps run: 0 % is mono, 100 % opposite, the widest. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the effect is heard: at 0 % the input passes unchanged, at 100 % the full chorus (the dry sound plus the wobbling copies). |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/chorus.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/chorus.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/chorus.rs` | stub (pass-through); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/chorus.rs` | placeholder |
| Golden cases | `tests/golden/cases/chorus-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): a sine (or an `osc` sine voice) through the module at a sweep of frequencies, with its modulation depth at 0 where it has one, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane MOD).
