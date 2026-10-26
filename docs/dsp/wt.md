# Wavetable (`wt`; `enth-dsp::wt`, `enth-engine::modules::wt`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane WT owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `wt`, discriminant 8, section `generators`, id prefix `wt` (wt1, wt2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a wavetable oscillator on phase-read waves: generated tables, phase warps, phase and amplitude modulation from another generator through audio routes (`pm` with the modulatable `pm_index`; `am`, ring modulation when its base is 0 %), `feedback`, and unison on `wide` lanes (lane UNI; its parameters are in the F1m table).
- Sources to start from: research report 05 §1 (PM, AM and ring modulation between generators, feedback) and §7 (unison on `wide` lanes); report 06 §3.1 (the audio-route contract) and §3.2 (unison) (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane WT (unison: UNI) implements, generated from `crates/enth-patch/src/catalogue/wt.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **PM:** the phase offset in radians is `pm_index · (pm / 100) · pm_in(t)`, where `pm_in` is lane RTE's summed audio input (each route's pre-level source × its depth). `pm` (audio, not modulatable) trims everything routed in, default 100 %; `pm_index` (0–10 rad, modulatable) is the amount an envelope, LFO or MSEG moves for growls.
- **AM:** the output is multiplied by `am / 100 + am_in(t)`: 100 % and no route is unity; a route with `am` above 0 is AM (sidebands at `depth / (2·am)` of the carrier); **`am` 0 % with a route is ring modulation** (no carrier). Without a route `am` scales the output.
- **Warps** are the identity at `warp_amount` 0; the sign flips the direction for `bend`, `asymmetric` and `squeeze`; `sync` and `quantize` use the magnitude. `feedback` is self-PM with a one-sample delay and two-sample averaging (report 05), 0 % none.
- **Unison (lane UNI):** `unison` copies (1–8, one `f32x8` lane) detuned evenly across ±`detune`, side copies at `blend`, panned across `spread`. `unison` is structural so the plan knows from the patch whether it needs SVP's stereo voice path (recommended: stereo when an enabled `wt` has `unison` ≥ 2; `spread` 0 must equal mono). A copy's start phase: `derive(voice_seed, k)` in `free`, the module id and `k` in `reset` (the same every note), `seed` and `k` in `seeded`; never a parameter value.
- **Defaults:** a plain sine at full level (`basic` at position 0), `phase_mode` reset, since PM, warps and feedback make the start phase part of the timbre.

### `wt` — Wavetable (generators, ids `wt1`, `wt2`…; 18 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `table` | none | `basic`, `fm`, `fm_octave`, `formant`, `metal` | enum | basic | structural | basic | Which generated wavetable to play: basic shapes, FM sweeps, a formant sweep or sparse metal; changing it rebuilds the voice plan. |
| `position` | percent | 0% … 100% | linear | 0% | mod, auto | basic | Where in the wavetable to play, from its first frame to its last; route an LFO or MSEG here for moving timbres. |
| `warp` | none | `off`, `bend`, `asymmetric`, `squeeze`, `sync`, `quantize` | enum | off | auto | intermediate | How the wave's cycle is bent before it is read: bend, asymmetric bend, squeeze, sync or quantize. |
| `warp_amount` | percent | -100% … 100% | linear | 0% | mod, auto | intermediate | How strongly the warp bends the cycle: 0 % leaves it straight; the sign flips the direction where the warp has one. |
| `semi` | semitones | -48st … +48st | linear | 0st | mod, auto | basic | Transposition in semitones from the played note; as an FM modulator, +12st is a 1:2 ratio and +19.02st a 1:3. |
| `fine` | cents | -100ct … +100ct | linear | 0ct | mod, auto | basic | Fine tuning in cents; a few cents off a 1:1 FM ratio makes the timbre slowly churn. |
| `level` | percent | 0% … 100% | linear | 100% | mod, auto | basic | Output level into the voice, as a linear percentage; at 0 % an FM modulator is silent but still modulates. |
| `pm` | percent | 0% … 100% | linear | 100% | auto, audio | advanced | Where audio routes from other generators land for phase modulation (FM); its value trims everything routed in: 100 % passes the route depths as set, 0 % turns PM off. |
| `pm_index` | none | 0 … 10 | linear | 1 | mod, auto | intermediate | How far a full-scale input into pm bends the phase, in radians: 1 is gentle, 5 a growl, 10 a harsh clang; route an envelope or MSEG here to make it talk. |
| `am` | percent | 0% … 100% | linear | 100% | auto, audio | intermediate | Where audio routes from other generators land for amplitude modulation; its value is the gain they swing around: 100 % is AM, 0 % is ring modulation (without a route it scales the output). |
| `feedback` | percent | 0% … 100% | linear | 0% | mod, auto | intermediate | Phase modulation of the oscillator by its own output: a sine turns towards a saw, then a rough, noisy edge. |
| `unison` | none | 1 … 8 | int | 1 | structural | intermediate | Copies of the oscillator per voice (1 is off), detuned against each other for width and thickness; changing it rebuilds the plan. |
| `detune` | cents | 0ct … +100ct | linear | +20ct | mod, auto | intermediate | How far the outermost unison copies sit from the center, in cents (±): small values thicken, large ones sound like a detuned chord. |
| `blend` | percent | 0% … 100% | linear | 100% | mod, auto | advanced | Level of the side unison copies against the center one: 0 % is the center alone, 100 % every copy equal. |
| `spread` | percent | 0% … 100% | linear | 50% | mod, auto | intermediate | How widely the unison copies are spread across the stereo field: 0 % is mono, 100 % fully left and right. |
| `phase_mode` | none | `free`, `reset`, `seeded` | enum | reset | auto | advanced | How each note's start phase is chosen: reset (the phase knob, the same every note, as FM and warps want), free (random per render seed) or seeded (from seed). |
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | The start phase in reset mode; between a carrier and its modulator it changes the FM timbre. |
| `seed` | none | 0 … 65535 | int | 0 |  | advanced | The random seed used in seeded phase mode, for the start phase and the unison copies; part of the patch, unlike the render seed. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/wt.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/wt.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/wt.rs` | stub (silent); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/wt.rs` | placeholder |
| Golden cases | `tests/golden/cases/wt-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): render the generator at a pitch sweep (and with PM from another generator, at a high index), measured against its fundamental, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

**WT's own row** ([phase-s2.md](../phase-s2.md#acceptance-per-lane)): the audio-route golden, an index and feedback sweep inside the aliasing bound, and closed-form spectral checks of what PM, AM and ring mod are: PM sidebands at carrier ± n × modulator with amplitudes |J_n(β)| within 0.5 dB (n ≤ 4, two indices); unipolar AM's sidebands at m/2 within 0.1 dB; ring mod's carrier below −60 dB; feedback moving toward a saw's 1/n slope.

## Aliasing

Bound: not yet measured (lane WT).
