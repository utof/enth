# MSEG (`mseg`; `enth-dsp::mseg`, `enth-engine::modules::mseg`)

- **Status: reserved stub (S2a lane S0d, freeze F0m).** Lane MSEG owns this page and replaces this stub when the module lands ([phase-s2.md](../phase-s2.md), "Lanes" and rule 7).
- Kind `mseg`, discriminant 12, section `modulators`, id prefix `mseg` (mseg1, mseg2…), rate voice. Fixed and persisted from F0m; hidden from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag.
- What it will be: a multi-segment envelope with 16 fixed points (lane CAT, F1m), a curve per segment, a length synced in beats or in ms, loop, one-shot and sustain-loop modes.
- Sources to start from: research report 05 §6 (modulators); lane CAT decides the point count and the envelope-time form (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`).

## Parameters (lane CAT, F1m)

The table lane MSEG implements, generated from `crates/enth-patch/src/catalogue/mseg.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **16 points, the first `points` used.** Point n's time, level and curve are parameters `11 + 3·(n − 1)` … `+ 2` (`MSEG_POINT_BASE`, `MSEG_POINTS`). Effective time = max(own, previous effective); before point 1 hold its level; after the last point hold its level, except in `loop`, where the last segment heads back to point 1's level at the end of the length.
- **Curves** bend the segment leaving a point: 0 % straight, positive early, negative late; +100 % jumps at once, −100 % holds then jumps (so 16 points are a 16-step pattern). Report 05's `x / (x + k(1 − x))`, `k` once per segment.
- **Length:** `beats` of the tempo when `sync` is on (default 1 bar), else `length` (10 ms–60 s). **Modes:** `loop`; `one_shot` (then hold); `sustain_loop` (loop `loop_start` … `loop_end` while held, equal points hold a level; after note-off play to the end). Out-of-range loop points clamp to `points`.
- **Start:** restarts at `phase` on each note-on that restarts the envelopes (the voice mode decides, as for `adsr`). **Output:** unipolar (levels as drawn) or bipolar (`2·level − 1`), scaled by velocity per `velocity`.
- **`scope` global** needs lane MSEG to add an `mseg` arm to `ModuleKind::rate` (as `lfo` has) and a global modulator arm in the engine; a global MSEG starts at the render start and never sees a note-off. Until then an MSEG runs per voice.
- **Envelope times above 10 s** are this kind (one-shot or sustain-loop), not an `adsr` parameter (docs/dsp/catalogue.md, "S2a decisions").

### `mseg` — MSEG (modulators, ids `mseg1`, `mseg2`…; 59 parameters)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `loop`, `one_shot`, `sustain_loop` | enum | loop | auto | basic | How the shape plays: loop (a drawn LFO), one-shot (an envelope) or sustain loop (loops while the note is held). |
| `sync` | none | off, on | toggle | on | auto | basic | On: the shape's length is beats of the song's tempo. Off: it is length, in milliseconds. |
| `length` | ms | 10ms … 60s | log | 1s | mod, auto | intermediate | How long one pass through the shape takes when tempo sync is off, up to a minute (long risers and swells). |
| `beats` | none | `sixty_fourth`, `thirty_second_triplet`, `thirty_second`, `sixteenth_triplet`, `thirty_second_dotted`, `sixteenth`, `eighth_triplet`, `sixteenth_dotted`, `eighth`, `quarter_triplet`, `eighth_dotted`, `quarter`, `half_triplet`, `quarter_dotted`, `half`, `bar_triplet`, `half_dotted`, `bar`, `two_bars_triplet`, `bar_dotted`, `two_bars`, `two_bars_dotted`, `four_bars`, `eight_bars`, `sixteen_bars` | enum | bar | auto | basic | How long one pass through the shape takes when tempo sync is on, as a note division: 1/16 to 16 bars. |
| `polarity` | none | `bipolar`, `unipolar` | enum | unipolar | auto | intermediate | Unipolar outputs the levels as drawn (0 to 1); bipolar maps them to -1 to +1, swinging around the destination's value. |
| `scope` | none | `voice`, `global` | enum | voice | structural | intermediate | One MSEG per voice or one global MSEG; only a global MSEG can modulate effects. Changing it rebuilds the plan. |
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | Where in the shape each note starts, as a share of the length (180deg starts halfway). |
| `velocity` | percent | 0% … 100% | linear | 0% | auto | intermediate | How much note velocity scales the output: 0 % ignores velocity, 100 % follows it fully. |
| `points` | none | 2 … 16 | int | 3 | auto | intermediate | How many of the 16 points make the shape; the rest are ignored. |
| `loop_start` | none | 1 … 16 | int | 2 | auto | advanced | The point the sustain loop jumps back to while the note is held. |
| `loop_end` | none | 1 … 16 | int | 2 | auto | advanced | The point where the sustain loop turns back while the note is held; equal to loop start, the level is held (a sustain). |
| `time1` | percent | 0% … 100% | linear | 0% | auto | advanced | Where point 1 sits, as a share of the length; a point never sits before the one before it. |
| `level1` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 1's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve1` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 1 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time2` | percent | 0% … 100% | linear | 50% | auto | advanced | Where point 2 sits, as a share of the length; a point never sits before the one before it. |
| `level2` | percent | 0% … 100% | linear | 100% | auto | advanced | Point 2's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve2` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 2 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time3` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 3 sits, as a share of the length; a point never sits before the one before it. |
| `level3` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 3's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve3` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 3 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time4` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 4 sits, as a share of the length; a point never sits before the one before it. |
| `level4` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 4's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve4` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 4 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time5` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 5 sits, as a share of the length; a point never sits before the one before it. |
| `level5` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 5's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve5` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 5 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time6` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 6 sits, as a share of the length; a point never sits before the one before it. |
| `level6` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 6's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve6` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 6 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time7` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 7 sits, as a share of the length; a point never sits before the one before it. |
| `level7` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 7's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve7` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 7 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time8` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 8 sits, as a share of the length; a point never sits before the one before it. |
| `level8` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 8's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve8` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 8 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time9` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 9 sits, as a share of the length; a point never sits before the one before it. |
| `level9` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 9's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve9` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 9 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time10` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 10 sits, as a share of the length; a point never sits before the one before it. |
| `level10` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 10's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve10` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 10 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time11` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 11 sits, as a share of the length; a point never sits before the one before it. |
| `level11` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 11's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve11` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 11 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time12` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 12 sits, as a share of the length; a point never sits before the one before it. |
| `level12` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 12's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve12` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 12 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time13` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 13 sits, as a share of the length; a point never sits before the one before it. |
| `level13` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 13's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve13` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 13 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time14` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 14 sits, as a share of the length; a point never sits before the one before it. |
| `level14` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 14's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve14` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 14 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time15` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 15 sits, as a share of the length; a point never sits before the one before it. |
| `level15` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 15's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve15` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 15 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |
| `time16` | percent | 0% … 100% | linear | 100% | auto | advanced | Where point 16 sits, as a share of the length; a point never sits before the one before it. |
| `level16` | percent | 0% … 100% | linear | 0% | auto | advanced | Point 16's level: 0 % is the bottom and 100 % the top (-1 and +1 when bipolar). |
| `curve16` | percent | -100% … 100% | linear | 0% | auto | advanced | The bend of the segment from point 16 to the next: 0 % is straight, positive moves early, negative late; +100 % jumps at once and -100 % holds, then jumps. |

## Files (pre-declared by S0d)

| What | Where | Today |
| --- | --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/mseg.rs` | lane CAT's F1m table ([Parameters](#parameters-lane-cat-f1m)) |
| DSP core | `crates/enth-dsp/src/mseg.rs` | empty |
| Engine module | `crates/enth-engine/src/modules/mseg.rs` | stub (outputs 0); the dispatch arms are written |
| Instruction budget | `crates/enth-engine/benches/mseg.rs` | placeholder |
| Golden cases | `tests/golden/cases/mseg-*.toml` (the case-name prefix is the lane's namespace; the directory is flat) | none |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 %; two instances keep separate state; `mix` at 0 passes the input bit for bit (effects); **aliasing measured spectrally** with the shared harness ([aliasing.md](aliasing.md)): not an audio path: a modulator's own aliasing is not measured, but its effect on a parameter it sweeps (zipper noise at segment steps) is listened for, the bound recorded here and in the test. A `THIRD_PARTY_NOTICES.md` entry and a provenance section here for every port.

## Aliasing

Bound: not yet measured (lane MSEG).
