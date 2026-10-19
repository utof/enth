# LFO (`enth-dsp::lfo`, `enth-engine::modules::lfo`)

- Lane: P1a D2. Code: `crates/enth-dsp/src/lfo.rs` (the oscillator, white-box tests), `crates/enth-engine/src/modules/lfo.rs` (the module, both scopes). Catalogue entry: `crates/enth-patch/src/catalogue/lfo.rs`. Bench: `crates/enth-engine/benches/lfo.rs`.
- Decisions behind it: research report 02 (the "u32 phase" accumulator and the seeding rule) and engine.md (the seed derivation).

A low-frequency oscillator with seven shapes, used as a modulation source. Voice scope (one LFO per sounding note, a `VoiceModulator`) and global scope (one for the engine, a `GlobalModulator`) are the same type; `ModuleImpl::new` picks the role from the structural `scope` parameter.

## Phase

The phase is a **64-bit integer accumulator**. Its top 32 bits are the phase of the cycle (report 02's "u32 phase"); the low 32 bits carry the fraction of an increment that a 32-bit accumulator would lose. That matters at the bottom of the range: at 0.01 Hz with 32-frame steps the 32-bit increment is 28,633.1 counts, and rounding it to 28,633 alone is a 1.7e-5 rate error (an LFO that is 1 s off after a minute). With the 64-bit accumulator the only error is the `f64` rounding of the increment, about 1e-16: the test counts the cycles of seven rates from 0.01 to 50 Hz over 60 s and requires a relative error below 1e-9 (it measures far below that). The phase wraps by integer overflow, never goes inexact and is identical on every platform.

- Increment: `round(rate · 32 / fs · 2⁶⁴)`, recomputed only when the rate's bit pattern changes. A non-finite or non-positive rate stops the LFO.
- **Advance, then evaluate.** Once per segment `tick` adds the increment and returns the shape at the *new* phase, so the output is the value at the *end* of the segment, which is what the engine's modulator contract asks for (it ramps from the previous segment's value to this one's). The first value of a note is therefore one step after the start phase.
- The shape is evaluated from the top 24 bits of the phase, `x ∈ [0, 1)`, which `f32` represents exactly.
- A modulated `rate` is read at the start of each segment and changes the speed from there; the phase is continuous (no jump) whatever the rate does.
- The LFO is sampled at the segment rate (1.5 kHz at 48 kHz): at 50 Hz that is 30 points per cycle, joined by the engine's linear ramp. A square wave's edges land on segment boundaries (up to 0.67 ms late), and a triangle or sine at the top rate is visibly polygonal. This is a property of the engine's control rate, not of the oscillator.

## Shapes

All shapes are bipolar, −1 … 1; `polarity = unipolar` maps `y` to `0.5 + 0.5·y`.

| Shape | at `x = 0` | Formula |
| --- | --- | --- |
| `sine` | 0, rising | `sin(2π·x)` (pinned `libm` `sinf`) |
| `triangle` | 0, rising, in phase with the sine | `4x` on `[0, ¼)`, `2 − 4x` on `[¼, ¾)`, `4x − 4` on `[¾, 1)` |
| `saw_up` | −1 | `2x − 1` |
| `saw_down` | +1 | `1 − 2x` |
| `square` | +1 | `+1` for `x < ½`, else `−1` |
| `sample_hold` | the cycle's drawn value | one new random value per cycle, held |
| `smooth_random` | the cycle's drawn value | smoothstep `x²(3 − 2x)` from this cycle's value to the next one's: continuous, and passes through every drawn value |

Randomness comes from a `Rng` (the project's `SplitMix64`) and never from a patch value. The draw order is fixed: the start phase (if random), then the first cycle's value `a`, then the next cycle's value `b`, then one draw per wrap (`a ← b`, `b ← draw`).

## Start phase and seeds (`phase_mode`)

| Mode | Start phase | Random stream (sample & hold, smooth random) |
| --- | --- | --- |
| `reset` (default) | the `phase` knob, degrees / 360 | the engine's seed for this note (render seed, module id, note index) |
| `free` | a draw from that stream | that stream, after the start-phase draw |
| `seeded` | a draw from the module's own `seed` parameter (`seeded_voice_seed(seed, module, note)` / `seeded_global_seed(seed, module)`) | that stream |

So a `reset` LFO is the same on every render for a given render seed, a `free` one changes with the render seed (the descriptors' noise estimate varies it), and a `seeded` one is fixed by the patch. Moving any other knob never reseeds the random shapes: the seed is derived from the render seed, the module id and the note index, not from patch values (this is what the seed-stability test checks).

## Tests

- Rate error over 60 s for seven rates (above), the increment is exact for binary fractions of the sample rate.
- Landmarks of every shape; `tick` returns the end-of-step value; sample & hold draws exactly once per cycle from the seed (using rates that are exact binary fractions, 187.5 Hz = 1/8 cycle per step); smooth random is continuous and passes through the drawn values.
- A random start phase depends on the seed alone; changing the rate keeps the phase.
- **Pinned output hashes**: FNV-1a over the bit patterns of 30,000 control-rate values per shape (rate 3.7 Hz, random start, seed `0x0E175EED`), seven hashes in `lfo.rs`. They were checked equal under wasmtime (wasm32-wasip1) when written down, and the `enth-dsp` unit tests pass there.
- The module: a 50 Hz sine, polarity, all three phase modes, S&H seed dependence, independence of the voices, a global LFO's reset and seeded modes, a continuous phase when the rate changes, `copy_state_from`, no subnormals.

There is **no patch-level golden for the LFO yet**: the S2a compiler refuses modulation routes (`CompileError::NotYet`, lane S2b), so an LFO in a patch has no audible effect. When S2b lands routes, add `tests/golden/cases/lfo-*.toml` (one per shape, an LFO on `filter1.cutoff`, a global scope one, a seeded sample & hold) in the same commit; until then the pinned hashes above are the LFO's cross-platform guard.

## Cost

Callgrind, 1 s at 48 kHz, whole engine, minus the `engine` bench with the same voices (3,873,015 and 43,370,997 instructions):

| Case | Instructions | Budget (+10 %) | Marginal |
| --- | --- | --- | --- |
| `voice_sine_1v` | 4,603,182 | 5,064,000 | |
| `voice_sine_16v` | 48,928,795 | 53,822,000 | 7.2 per sample per voice |
| `voice_random_16v` (smooth random, 20 Hz) | 47,774,635 | 52,553,000 | 5.7 per sample per voice |
| `global_sine_16v` | 43,795,237 | 48,175,000 | 8.8 per frame in total (not per voice) |

One tick is a 64-bit add, a wrap test and the shape. Callgrind's self cost for a sine is `Lfo::tick` 61 instructions plus `sinf` 36 per voice per segment (about 3 per sample per voice); the rest of the marginal figure is the engine handling one more modulator per voice per segment.

## Behaviour other lanes must know

- The output is a per-segment value; the engine does the interpolation. `voice_finished` is always false: an LFO never keeps a voice alive.
- `copy_state_from` copies every voice's LFO and the global one, so a plan swap keeps the phases.
- Routes are S2b's: the LFO needs nothing from them except `cx.out[v]` / the global return value.
