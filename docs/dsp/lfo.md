# LFO (`enth-dsp::lfo`, `enth-engine::modules::lfo`)

- Lane: P1a D2. Code: `crates/enth-dsp/src/lfo.rs` (the oscillator, white-box tests), `crates/enth-engine/src/modules/lfo.rs` (the module, both scopes). Catalogue entry: `crates/enth-patch/src/catalogue/lfo.rs`. Bench: `crates/enth-engine/benches/lfo.rs`.
- Decisions behind it: research report 02 (the "u32 phase" accumulator and the seeding rule) and engine.md (the seed derivation).
- S2a wave 2 (lane TMP, freeze F1m) added tempo sync, the shared `beats` divisions and a one-shot mode: [Tempo sync and one-shot](#tempo-sync-and-one-shot-s2a-lane-tmp). Code: `synced_increment`, `Lfo::set_synced` and `Lfo::set_one_shot` in `enth-dsp::lfo`; the module reads them in `enth-engine::modules::lfo`; the tempo itself is the engine's ([engine.md](engine.md#tempo-a-deterministic-render-input-s2a-lane-tmp)).

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

## Tempo sync and one-shot (S2a, lane TMP)

Three parameters, appended after the seven above with neutral defaults, so an LFO with `sync` and `one_shot` off is the module it was, bit for bit (S0d's test renders an unsynced patch at 90, 140 and 174 BPM to the same bits; the 73 goldens are unchanged).

| Key | Kind | Default | Tier | Meaning |
| --- | --- | --- | --- | --- |
| `sync` | toggle | off | advanced | Off: `rate` in Hz, as always. On: one cycle lasts `beats` at the engine's tempo and `rate` is ignored |
| `beats` | enum, **modulatable** | 1/4 | advanced | The cycle length as a note division. Modulating it steps through the list, shortest first; there is no interpolation between divisions |
| `one_shot` | toggle | off | advanced | On: one cycle from the start phase at each note-on that restarts the envelopes (not a legato note; the render start for a global LFO), then the last value is held |

All three sit in the `advanced` tier, behind a card's "More": the editor's P1 layout (1200 by 800, nothing scrolls) has no room for three more rows on the LFO card, and the dense cards of F2m (lane EDS) are where `sync` and `beats` move up a tier. The answer code appends them to the `LFO_V1` row (a toggle, an item of 25 in 5 bits, a toggle). Answer parameters are indices into that append-only table, so a code written before F1m names only indices 0 to 6 and reads with the three at their defaults; the table's pinned hash changed with the row, by design.

### Tempo

The tempo is the engine's, not the module's: it reaches the engine as a tempo event (applied at the first 32-frame segment boundary at or after its frame, like a note), is 20 to 999 BPM (140 until told), and the engine hands it to the module as `cx.tempo_bpm` at every segment ([engine.md](engine.md#tempo-a-deterministic-render-input-s2a-lane-tmp)). A beat is a quarter note and a bar is four beats, whatever the host's time signature. `enth render --bpm` and the plugin's host transport both end in that event.

- **Exact in beats.** `synced_increment(bpm, (num, den), fs, 32)` is `round(bpm / 60 · den / num · 32 / fs · 2⁶⁴)`: the cycle length stays a ratio of integers until one `f64` division, so the increment is the exact value to the `f64` rounding (about 1e-16 relative) and a synced phase after any number of segments at one tempo is exact in beats. A test checks the increment against 128-bit integer arithmetic (six tempos from 20 to 999 BPM, eight divisions including triplets and dotted notes, three sample rates), and another runs ten minutes of segments at five tempo and division pairs against the exact number of cycles (relative error below 1e-12).
- **Continuous across a tempo change.** The accumulator is the same one `rate` uses: a tempo event changes the increment from its segment on and the phase does not jump. Under a piecewise-constant tempo the phase is the integral of the tempo since the LFO started: from the note-on for a voice LFO, from the engine's start or reset for a global one. It is **not** locked to the host's bar line or beat position: a cycle of `1 bar` begins at the note, wherever the host's timeline is. Changing `sync` or `beats` keeps the phase too (only the increment changes), so a switch never jumps.
- **Host block size is not an input.** A tempo change in the middle of a host block applies at the next segment boundary, the same boundary at block sizes 1, 64, 100 and 4096; the test renders a synced patch with a mid-render tempo change at block sizes 1, 2, 7, 31, 33, 64, 100, 441, 1000, 4096 and one block of the whole render, and compares the bits.
- **Plan swaps and resets.** The tempo is engine state, carried over a plan swap (a synced LFO keeps its phase and its tempo through a swap). `Engine::reset` returns it to 140, like every event-driven state, and the plugin sends the host's tempo again.
- The tempo is read at the start of each segment, so `cx.tempo_bpm` is the value in force for the segment being rendered, and the output is, as for `rate`, the value at the segment's end.

### The divisions (`beats`)

`beats` is the shared `Beats` enum (`crates/enth-patch/src/catalogue/mod.rs`, introduced by lane CAT for every tempo-synced parameter: `lfo.beats` here, `mseg` and `delay` from CAT), not a list of the LFO's own: friends see one list. It is not a pinned enum, but it is persisted and `lfo.beats` is modulatable and automatable, so the list is **frozen at F1m**: `from_norm` spreads the parameter's range over `COUNT − 1` steps, so one more item anywhere would move the step that every stored route and automation lane lands on, and the sorted-by-length test also forbids appending anything shorter than 16 bars. A modulatable enum is frozen once released, and its items are ordered so that neighbouring items are neighbours in effect. `Beats::length()` gives each item as an exact fraction of a beat in lowest terms, which is what `synced_increment` takes. 25 items, sorted by length (strictly increasing, which the catalogue test checks):

| Beats | Items |
| --- | --- |
| under a beat | 1/64 (1/16 beat), 1/32T, 1/32, 1/16T, 1/32D, 1/16, 1/8T, 1/16D, 1/8, 1/4T, 1/8D |
| a beat to a bar | 1/4 (1 beat, the default), 1/2T, 1/4D, 1/2, 1 bar T, 1/2D, 1 bar |
| over a bar | 2 bars T, 1 bar D, 2 bars, 2 bars D, 4 bars, 8 bars, 16 bars |

Why this range and these items, for an LFO:

- **The wobble is 1/4, 1/8T and 1/16** (and 1/8, 1/8D, 1/4T next to them): the divisions bass music uses for its filter and amplitude movement, at the default 140 BPM about 2.3, 7.0 and 9.3 Hz for 1/4, 1/8T and 1/16. Every straight division from 1/32 to 2 bars has both a triplet and a dotted neighbour in the list; 1/64 and 4, 8 and 16 bars have neither (the fast and the slow ends stay straight). The dotted eighth is the classic syncopated bounce and the triplets are the rolling feel; a producer reaches for them by ear, and a list that lacks one sends them back to a free `rate` that cannot hold the beat.
- **Fast end, 1/64 and 1/32:** tremolo-like and ring-like movement. At 140 BPM 1/64 is 37 Hz. The ceiling is the engine's control rate, not the list: at 999 BPM a 1/64 cycle is 3.75 ms (267 Hz), about five and a half points per cycle at 48 kHz, joined by the engine's linear ramp, as for any fast `rate`.
- **Slow end, up to 16 bars:** risers, builds and slow filter sweeps that are phrase-long and must land on the bar. At 140 BPM 16 bars is 27 s, and at 20 BPM 192 s (0.005 Hz, below the free `rate`'s 0.01 Hz floor, which is a reason to sync, not a problem: the 64-bit increment is 6.4e13 counts per segment there). The lengths above four bars are what no free `rate` knob can do beat-exactly.
- **Modulating `beats` walks the list in order.** Because the list is sorted by length, a rising modulation always lengthens the cycle (an envelope or a macro on `beats` goes from 1/16 up to 1/4 through 1/8T, 1/16D, 1/8, 1/4T and 1/8D in that order). `from_norm` rounds a discrete parameter to the nearest item, so a modulated enum steps and never passes through a length between two items; the test sweeps a macro across `beats`, checks that only whole items are ever seen, and that each item renders at its own closed-form phase. A modulated `beats` is read per voice (and once for a global LFO) at the start of each segment.
- Enum modulation needed one change outside the LFO: `enth-params::check_spec` allowed `modulatable` only on `Float` parameters and now allows it on `Enum` too. Everything that reads "modulatable" (the UI drop target, the solver, answer routes) is generic over `ParamSpec`.

### One shot

With `one_shot` on, the LFO runs exactly one cycle from the phase it starts at and then holds the value it had at the end of that cycle.

- **Start.** A voice's LFO is armed at its first segment, so the cycle starts at the note's start phase (`phase`, in `reset` mode); a global LFO is armed at the render start. A voice LFO restarts at every note-on that restarts the envelopes: a poly note is a new voice, and a `mono` retrigger (a note-on on a voice that is still held or releasing) calls `Lfo::restart_one_shot`, which sets the phase back to the cycle's start phase (kept in the held state) and runs the cycle again, as `adsr` and `mseg` restart on `NoteState::trigger`. A `legato` note is not a note-on for the envelopes, so it does not restart the cycle: a finished cycle stays held. The random stream is not touched by a restart. A global LFO is armed at the render start and no note-on restarts it (see the open decision below).
- **The hold is the left limit.** At the end of the cycle the phase parks one count before where it started, so a `saw_up` holds +1 (not the −1 it wraps to), a `saw_down` −1, a `square` the second half's −1, and a sine or a triangle from phase 0 holds 0 (to within 1e-5). The value is held for as long as the note sounds; `voice_finished` stays false.
- **The end is exact in beats.** The cycle ends in the segment in which the travelled phase reaches 2⁶⁴; the integer arithmetic makes that the same segment at every block size (the engine test: 750 segments for a quarter note at 120 BPM). With `sync` on a tempo change shortens or lengthens the rest of the cycle, and the phase travelled is exact. A cycle starting at a phase other than 0 ends where it started, after one whole cycle's travel.
- **Random shapes** hold their own cycle: `sample_hold` holds this cycle's drawn value, `smooth_random` ends on the value it was heading to, which is already drawn (the draw order is as free-running, one draw per wrap, and a one-shot starting at phase 0 draws nothing past its own cycle).
- **Toggling.** Turning `one_shot` on mid-note starts a one-shot cycle from the phase it has then. Turning it off runs freely again from where it is. Neither restarts a cycle in flight, and the toggle is not smoothed.
- **Open decision for lane MSEG (F1m judge, S7): the restart of a global one-shot.** A global one-shot LFO (and a global MSEG) runs its cycle once, from the engine's start or a reset, and nothing restarts it: in a DAW that means plugin activation or a transport reset, not the first note of each phrase. A voice one-shot restarts at every note-on that restarts the envelopes; a global one has no such event today. The choices are: (a) leave it, as documented, and say so on the card; (b) restart a global one-shot when the engine goes from no sounding voice to one (the first note of a phrase), which is what a producer expects of a "one-shot" and needs a `NoteState`-level "voice count went 0 → 1" event the engine does not have yet; (c) never offer `one_shot` on a global scope. Lane MSEG builds on whichever is chosen, so decide it before MSEG starts; nothing in F1m depends on it.
- **The held flag can be one segment late (F1m judge, S5).** `Lfo::is_held` turns true one segment after the cycle's last value when the rounded increment falls short of a whole cycle (120 BPM, a quarter note: segment 751 and not 750). The output bits are unaffected (the value is already the held one); only a reader of `is_held` sees the difference, so a module that ends a voice or a stage on `is_held` (lane MSEG) should not assume the exact segment.

### Tests

| Where | What |
| --- | --- |
| `enth-dsp` `lfo.rs` | the synced increment against 128-bit exact arithmetic (six tempos, eight divisions, three sample rates); ten minutes of segments at five tempo and division pairs exact in beats; a tempo change keeps the phase and changes the speed; one-shot runs one cycle then holds (the five deterministic shapes, a start phase, a cycle that is not a whole number of segments, the random shapes); arming, disarming and re-arming; one-shot off changes nothing; the pinned output hashes are unchanged |
| `enth-patch` `catalogue/lfo.rs` | the three parameters are appended after the seven (indices 7, 8, 9) with neutral defaults, `beats` is the one modulatable enum, no item was added to `lfo.shape`, the shared divisions cover the LFO's needs, triplets are 2/3 and dotted 3/2 of their straight length |
| `enth-engine/tests/tempo.rs` | a synced sine LFO's phase at 90 and 174 BPM and four divisions (quarter, an eighth triplet, a dotted quarter, a bar), segment by segment for 30 s; a tempo change keeps the phase; a tempo event applies at the first segment boundary at or after its frame; the engine clamps a tempo, ignores NaN and `reset` restores 140; bit-exact at host blocks 1, 64, 100, 4096 and seven others with a mid-render tempo change; an unsynced patch ignores the tempo bit for bit; `beats` steps through the list and never between; one-shot runs one cycle, holds, and a voice LFO restarts at each note; the tempo survives a plan swap; no allocation with tempo events (`noalloc.rs`) |
| `enth-render` | a synced patch follows `tempo_bpm` and a mid-render `tempo_changes` entry is block-size independent; a tempo outside 20 to 999 is a `bad_performance` error, not a clamp |
| `enth-plugin` | the host's `Transport::tempo` drives a synced LFO like the render's tempo at every block size when the host reports a tempo change at a block start (the plugin sends one tempo event per block, at its frame 0; a change in the middle of a block reaches the engine at the next block, while the CLI's `tempo_changes` can sit at any frame, so plugin = CLI only for changes at block starts); none given keeps 140 (and the last tempo seen); a reset sends the host's tempo again |
| `enth-cli` `render_bpm.rs` | `--bpm` defaults to 140 and a synced patch follows it; an unsynced patch renders the same bits at any tempo; a tempo outside 20 to 999 is a usage error; the flag is in `render --help` (snapshot) |
| `enth-challenge` | the three parameters round-trip in an answer code (`LFO_V1`); the pinned answer-table hash changed with the row, by design |

## Tests (the oscillator and the module)

- Rate error over 60 s for seven rates (above), the increment is exact for binary fractions of the sample rate.
- Landmarks of every shape; `tick` returns the end-of-step value; sample & hold draws exactly once per cycle from the seed (using rates that are exact binary fractions, 187.5 Hz = 1/8 cycle per step); smooth random is continuous and passes through the drawn values.
- A random start phase depends on the seed alone; changing the rate keeps the phase.
- **Pinned output hashes**: FNV-1a over the bit patterns of 30,000 control-rate values per shape (rate 3.7 Hz, random start, seed `0x0E175EED`), seven hashes in `lfo.rs`. They were checked equal under wasmtime (wasm32-wasip1) when written down, and the `enth-dsp` unit tests pass there.
- The module: a 50 Hz sine, polarity, all three phase modes, S&H seed dependence, independence of the voices, a global LFO's reset and seeded modes, a continuous phase when the rate changes, `copy_state_from`, no subnormals.

**Patch-level goldens** (`tests/golden/cases/lfo-*.toml`, P1a CH, once routes landed): a saw at 110 Hz through a gate envelope (so the envelope does not move them) into an `svf` low-pass at 600 Hz, resonance 30 %, with `lfo1` routed to `filter1.cutoff` at 25 % (about 2.5 octaves either way). They match under wasmtime (`cargo xtask golden --wasm`).

| Case | Guards |
| --- | --- |
| `lfo-sine`, `lfo-triangle`, `lfo-saw-up`, `lfo-saw-down`, `lfo-square` | each deterministic shape at 2 Hz through the route, the end-of-segment sampling and the filter under modulation |
| `lfo-sample-hold`, `lfo-smooth-random` | the random shapes in `reset` mode with two notes: each note index draws its own stream from the render seed |
| `lfo-seeded-sh` | `phase_mode = seeded` (seed 4321) with a non-default render seed: the stream comes from the module's seed, not the render seed |
| `lfo-unipolar` | 0 to 1 polarity: the route only opens the filter |
| `lfo-global` | global scope: one LFO free-running from the render start; a second note joins mid-cycle and hears the same phase |
| `lfo-global-comp` | a global LFO driving a global processor (`comp1.mix`), the only scope that may modulate effects |
| `lfo-vibrato` | a voice LFO at 5.5 Hz on `osc1.fine` (about 40 ct either way) |
| `lfo-rate-mod` | an LFO on an LFO's `rate`: the phase stays continuous while the rate moves, and a source that ticks later is read one segment late |

The pinned output hashes above remain the oscillator's own cross-platform guard.

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
- Routes are S2b's: the LFO needs nothing from them except `cx.out[v]` / the global return value. A route from an LFO that ticks later in patch order reads its previous segment.
- **Tempo (F1m).** A module that syncs reads `cx.tempo_bpm` (a field of `VoiceModCx`, `GlobalModCx` and `GlobalCx`; not of `VoiceCx`). Other lanes that build those contexts by hand (tests, the `mseg` and `delay` lanes) set `tempo_bpm: enth_engine::tempo::DEFAULT_TEMPO_BPM`. `beats` is the shared `Beats` enum; `mseg` and `delay` use the same one and `Beats::length()` for exact lengths.
- **Stored patches.** `Patch::to_json()` is dense, so every stored `lfo` gained `"sync":false,"beats":"quarter","one_shot":false`: the content, fixtures, golden digests and REAPER-recorded files that hold an `lfo` were regenerated with the lane's commits. A branch that also touched such a file should regenerate it (the lane's regeneration tool is the catalogue's own `to_json`), not text-merge it.

## Proposal for S2b: where tempo enters a challenge's render spec

No challenge code changed in F1m. Today every challenge render takes its `Performance` from `Phrase::performance()` / `PhraseView::performance()` (`crates/enth-challenge/src/model.rs`), which uses `..Performance::default()`, so every challenge, round, lesson check and solver run already renders at the default 140 BPM, and a generator-v1 patch never has `sync` on (the generator does not draw it), so none of them depends on the tempo. The answer code needs nothing either: the player's `sync`, `beats` and `one_shot` travel in the patch, and the tempo is not part of the patch.

For generator v2 (S2b), which may draw synced LFOs, the tempo should be **a property of the phrase, derived from the spec, not a new field of the code**:

1. **Where it lives.** Add `tempo_bpm` to `PhraseView` (a `u16` of whole BPM in the phrase table, default 140; `PhraseView::performance()` copies it into `Performance::tempo_bpm`). Every consumer of the phrase (the plugin, the CLI's `compare` and `match`, the solver, the lessons' `Check::Sound`, `Phrase::performance` callers listed by `grep '\.performance()'`) then renders at that tempo with no change. This is the one seam; a second path would let the target and the player's render disagree.
2. **What fixes it.** Not the code: `RandomSpec` (`generator`, `seed`, `difficulty`, `amounts`, `musical`, `template`) has no room, and code v1 cannot grow. Generator v2 owns the phrase draw, so the tempo is a function of `(generator, seed, phrase)`, drawn like every other generator draw (`derive(derive(seed, fnv1a64(address)), attempt)`, with an address of its own) over a short table of musical tempos (for example 70, 87, 100, 128, 140, 150, 174), or fixed at 140 for templates that never sync. Generator 1 and every old code keep 140 and their hash goldens.
3. **The player must see it.** The challenge view shows the tempo ("140 BPM") next to the phrase, because the player's patch is judged at it and a synced `beats` only means something against it. The plugin follows the host's tempo in free play; while it plays or scores a challenge phrase it must send the phrase's tempo to the engine instead, as the phrase's notes replace the host's MIDI, or the player hears a synced LFO at one tempo and is judged at another.
4. **Lessons and expectations (challenge v2/v3 targets).** A lesson that teaches sync needs a tempo the author chooses: add an optional `tempo_bpm` to the draw and patch targets (and to `Check::Sound`'s phrase), serde-defaulted to 140 and written only when it differs, so the "written at the lowest version" rule keeps every existing challenge's bytes. This is an additive change after F1l, in the same style as the `locked`/`editable` fields.
5. **Scoring and fingerprints.** A fingerprint hashes what the generator decided (generator v1: every draw it made, by key), or for a patch target the patch's non-default values, plus the phrase id (`fingerprint.rs`, magic `enth-target-v1` and `enth-target-patch-v1`). It is **not** computed from the rendered target, and it holds no tempo, so a phrase tempo does not reach a fingerprint with no change; a target drawn at 174 BPM would share its fingerprint with the same draw at 140, and `target_mismatch` and `target_drift` could not tell them apart. So the tempo has to be written in explicitly, and only when it is not the default, so that every existing fingerprint keeps its bytes:
   - **Generator v2** adds its tempo draw as a fingerprint entry (key `tempo`, a `p` entry carrying the BPM as an `f32`), like every other draw it made.
   - **`of_patch`** (the patch-target fingerprint) gains a `tempo` entry, written only when the target's tempo is not 140, so every existing patch target's fingerprint is byte for byte what it was.
   - **The nearest-neighbour bank** (`enth-solver/src/nearest.rs`, whose header already records the render's `velocity`, `hold_s`, `duration_s`, `sample_rate` and `render_seed`) records its render tempo in the header (taking one of the reserved words, or a new `BANK_FORMAT`), because a bank's embeddings are the rendered sounds' descriptors and they depend on the tempo when the bank's patches sync. The catalogue pin (`Beats` is persisted and frozen at F1m) already covers the note-division list.
6. **Not in the render spec.** `RenderSpec` (`sample_rate`, `seed`) stays as it is: the tempo is a performance input like the notes, validated 20 to 999 by `enth-render`, and the CLI's `--bpm` is an override of the same field for a free render. A "create from sound" job whose intake detects a tempo sets `Performance::tempo_bpm` of the performance it infers directly.

