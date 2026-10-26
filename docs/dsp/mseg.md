# MSEG (`mseg`; `enth-dsp::mseg`, `enth-engine::modules::mseg`)

- **Status: implemented (S2a lane MSEG, 2026-10-26)**, to lane CAT's F1m table below: 16 points with a curve each, `loop`, `one_shot` and `sustain_loop`, a length synced in beats or free from 10 ms to 60 s, per-voice and global scope, mono-retrigger like the LFO's and `adsr`'s, and an exact one-shot end.
- Kind `mseg`, discriminant 12, section `modulators`, id prefix `mseg` (mseg1, mseg2…), rate voice or global by `scope`. Fixed and persisted from F0m; **still hidden** from the Add menu, `enth modules` and the generator until lane INT-S2a clears its flag (`ModuleSpec::hidden` is untouched).
- What it is: a multi-segment envelope. Drawn as a loop it is a tempo-synced LFO with any shape (a wobble, a 16-step gate); as a `one_shot` it is the long envelope (up to 60 s free or 16 bars synced, the riser and the swell); as a `sustain_loop` it is an envelope with a held part. It is the "biggest single dubstep modulator" of report 04.
- Sources: research report 05 §6 (modulators) and report 04 (`_local/sessions/2026-10-25-s2-depth-and-fast-training/`); the curve `x / (x + k(1 − x))` is report 05's, the same family as Schlick's bias function (Graphics Gems IV, 1994) with `a = 1 / (1 + k)`. **No code is ported and there is no `THIRD_PARTY_NOTICES.md` entry**; the shape, the exact increments and the restart rules are this lane's.
- Latency 0: a modulator reads no audio.

## Parameters (lane CAT, F1m)

The table lane MSEG implemented, generated from `crates/enth-patch/src/catalogue/mseg.rs` by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_reserved_markdown`. After F1m a key, unit, range, scale, default or flag changes only through the coordinator. The file's doc comment states the signal path the meanings assume; the decisions behind the table are in [catalogue.md](catalogue.md#s2a-decisions-lane-cat-f1m).

- **16 points, the first `points` used.** Point n's time, level and curve are parameters `11 + 3·(n − 1)` … `+ 2` (`MSEG_POINT_BASE`, `MSEG_POINTS`). Effective time = max(own, previous effective); before point 1 hold its level; after the last point hold its level, except in `loop`, where the last segment heads back to point 1's level at the end of the length.
- **Curves** bend the segment leaving a point: 0 % straight, positive early, negative late; +100 % jumps at once, −100 % holds then jumps (so 16 points are a 16-step pattern). Report 05's `x / (x + k(1 − x))`, `k` once per segment.
- **Length:** `beats` of the tempo when `sync` is on (default 1 bar), else `length` (10 ms–60 s). **Modes:** `loop`; `one_shot` (then hold); `sustain_loop` (loop `loop_start` … `loop_end` while held, equal points hold a level; after note-off play to the end). Out-of-range loop points clamp to `points`.
- **Start:** restarts at `phase` on each note-on that restarts the envelopes (the voice mode decides, as for `adsr`). **Output:** unipolar (levels as drawn) or bipolar (`2·level − 1`), scaled by velocity per `velocity`.
- **`scope` global** is one MSEG for the whole engine, the only one that can modulate effects; lane MSEG added the `mseg` arm of `ModuleKind::rate` and the global modulator role. Its restart rule is [below](#global-scope-the-restart-rule-f1m-s7-decided-here): a loop free-runs from the engine's start, a one-shot or sustain loop restarts at the first key of a phrase, and the gate is any key held.
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
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | Where in the shape each note starts, as a share of the length (180° starts halfway). |
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

## How it plays

### Position and length: exact in beats (F1m S5, fixed)

The shape is a function of a **position** `x` in `[0, 1)`, a share of the length, kept as a `u64` (`2⁶⁴` is the whole length, as the LFO's phase). The length (a note division at the tempo, or milliseconds) only sets how far the position moves per 32-frame segment, so a tempo or `length` change moves the speed and never a point.

- **The increment is the exact quotient, rounded up.** `synced_increment(beats, bpm)` and `length_increment(ms)` do the division in `u128` (every `f64` and `f32` is an exact ratio of integers), so a length that is a whole number of segments ends on exactly that segment: a quarter note at 120 BPM and 48 kHz is **750** segments, 60 s is **90,000**. Rounding to nearest would end it a segment late whenever the quotient fell short by half a count (the LFO's `is_held` is late by one segment at 120 BPM, a quarter: F1m judge S5; the MSEG does not have that).
- **Tempo is read every segment** from `cx.tempo_bpm`; the increment is cached against (tempo, beats) so a steady tempo costs a comparison and not a division. The engine test runs five tempos (120, 60, 240, 90 and 240 BPM for 150, 300, 100, 200 and 50 segments, which add up to exactly one quarter note) with each tempo event landing 5 frames early inside a block: the hold-then-jump one-shot jumps in segment index 799 and not 798 or 800, per voice and global, and the first sound is in segment 799's frames at host blocks 1, 7, 32, 64, 100, 4096 and the whole render.
- **`length` is per voice** (it is modulatable), the tempo is global; both are read at the segment.

### Points, curves and the "reaching a time passes it" convention

A point's **effective time** is its own or the previous effective one, whichever is later, so points never cross and equal times make a vertical edge. With `x` the position and `i` the last point whose time is at or before `x`:

- before the first point's time the output holds that point's level;
- between point `i` and `i + 1`: `level[i] + (level[i+1] − level[i]) · f(u)`, `u = (x − t[i]) / (t[i+1] − t[i])`;
- after the last point it holds the last level, except in `loop`, where the last segment heads back to point 1's level and ends there at `x = 1` (`u = (x − t[n]) / (1 − t[n])`, with the curve of point `n`), so the loop joins. A last point at 100 % leaves no room: the loop jumps at the wrap.

**The curve** is `f(u) = u / (u + k (1 − u))`, `k = (1 − c) / (1 + c)`, `c` the curve as a share (−1…+1): 0 is `k = 1`, a straight line; positive is early (fast then slow); negative is late; `f` for `−c` is the one for `c` turned half a revolution. `c = +1` is an exact jump (`f = 1` as soon as the position is at or past the point's time) and `c = −1` an exact hold (`f = 0` until the next point's time). One division per segment and voice, no `pow`, no libm.

**Point times are exact positions.** A point's time is turned into a position once, `ceil(percent · 2⁶⁴ / 100)` in `u128` (100 % is `u64::MAX`), and the shape is looked up by comparing integers. With the increments rounded up too, a position reaches a point's position at exactly the step where the ideal position reaches the point's time. A 16-step gate whose steps fall on segment boundaries steps on them; a sustain loop of 20 % of a 750-segment length is exactly 150 segments a lap, 200 laps in a row. (The first version compared `f64` shares: 30 % is not exact in binary, a lap came out at 151 segments at aligned points, and `point_times_are_exact_positions_rounded_up` and the engine's lap test now guard it. A mutation check, rounding down instead of up, fails six tests.)

**The convention:** reaching a point's time *passes* it. A segment that ends exactly on a point shows the new level (a jump at 50 % of a 100-segment length is high in the segment whose end is at 50 %). This follows from the rounded-up increments, which reach the position at or just past the ideal one, and it makes `+100 %` unconditionally `f = 1` (a "made jump") and `−100 %` `f = 0` for `u < 1`.

### Modes

| Mode | Behaviour |
| --- | --- |
| `loop` | The position wraps by integer overflow; the last point heads back to point 1's level. Free-runs per voice from the start `phase` (voice) or from the engine's start (global). |
| `one_shot` | Plays the shape once from its start `phase` and holds the last level for as long as the voice lives; the end is the wrap, exact as above. `voice_finished` is false (an MSEG never keeps a voice alive). |
| `sustain_loop` | Runs to point `loop_end`; while the key is held it loops between `loop_start` and `loop_end` (`pos = ls + (next − le) mod (le − ls)` in integers; equal points hold that level, a sustain); released, it goes on from where it is to the end and holds. A note released before the loop is reached skips it. `loop_start` and `loop_end` above `points` count as the last point, and `loop_start` above `loop_end` as `loop_end`. |

### Start, retrigger and legato

A fresh voice starts at `start_position(phase)` (degrees of the length, 0…360). A **mono retrigger** (a note-on on a voice that is still held or releasing: `trigger` and not `fresh`) restarts the position at the same place, as the LFO's one-shot and `adsr` do; a **legato** note is not a trigger and does not restart it, so a finished one-shot stays at its last level. Poly notes are new voices, each with its own position. A restart is a jump the engine ramps over one segment (as the LFO's), and it does not glide.

### Global scope: the restart rule (F1m S7, decided here)

The LFO page left it open: a global one-shot has no restart event. The decision, built on a small additive hook (`NoteState::{keys_held, phrase_start}` and `GlobalModulator::keys`, both neutral for every other module):

- **A global `loop` free-runs** from the engine's start or reset (and when it first appears in a plan), whatever the notes do. A DAW transport that loops over a bar sees a steady pattern.
- **A global `one_shot` or `sustain_loop` is an envelope for the phrase.** It restarts at the **first key of a phrase**: a note-on while no key was held (`NoteState::phrase_start`, set in `NoteCx::note_on` from `!voices.any_held()`). Further keys in the chord or a legato line do not restart it. A note-off and a note-on in the same segment count as a new phrase (no key was held at the on).
- **The global gate is "some key is held"** (`NoteState::keys_held`, from `voices.any_held()` after the events of the segment). A global `sustain_loop` loops while any key is down and plays out to the end when the last one is let go.
- **Before any key** it plays the whole shape once from the engine's start (a one-shot) or loops in its sustain section as if released (a sustain loop with no key held plays out), so a patch that starts sounding with a swell on `reverb.wet` shows it at once. `Engine::reset` restarts it and returns the tempo to 140.
- **Velocity is ignored** (there is no single velocity), as for the other global modulators.

The LFO keeps its documented behaviour (`lfo.md`, "One shot"); nothing in the LFO changed. The hook (`GlobalModulator::keys`) is a default no-op other global modulators can use.

### Output: polarity and velocity

Unipolar outputs the level as drawn (0…1); bipolar `2 · level − 1` (−1…1). The voice value is scaled as `y · (1 − velocity_amount · (1 − note_velocity))` (`velocity` 0 % ignores velocity, 100 % scales fully; the same law as `adsr`'s), so full velocity is never changed. A global MSEG skips this. `ModulationRange` (`source_span`) knows the span for each polarity, so the editor's modulation arcs are right.

### Edits do not click (the glide)

The shape is compiled from the parameters once per segment, and only when one that matters changed (a comparison of 6 fixed parameters and the used points' time, level and curve against a snapshot; `sync`, `length`, `beats`, `scope` and `phase` do not need a rebuild). At a change, each voice goes on from **where the old shape would have been** (a shadow cursor advanced by the old shape, so a mode or loop-point switch is covered too) and glides to the new value linearly over `EDIT_GLIDE_MS` = 5 ms (8 segments at 48 kHz; the glide re-targets on consecutive edits). The engine test drives random edits and bounds the step of the output at an eighth of the edit plus the slope, so dragging a point is smooth and automation of a point (a `Value` effect, one transaction per drag) never clicks. A restart (a note-on) is not an edit and does not glide.

### Determinism

The shape runs in `f64` with `+ − × ÷` only (and `u128` integer division for the increments and thresholds), no libm, one cast to `f32`; the same bits on every target, independent of FTZ and of the host block size (events apply at segment boundaries). The module has no recursive filter state; the edit glide is its only decaying value and it ends at exactly 0.0 (`subnormals` counts the glides' values, and the 40 s tail test sees none). A plan swap carries the positions, the glides and the global state (`copy_state` pairs by id and kind); the engine does not allocate with MSEGs, edits and tempo changes (a debug-build test with a disabled allocator).

## Measured

### Tests

| Where | What |
| --- | --- |
| `enth-dsp` (`mseg.rs`, 17 tests) | the straight triangle; the curve against the formula and its half-turn symmetry; ±1 are the two steps; points never cross; loop and one-shot ends; polarity and velocity; the exact increments (a whole number of steps ends on that step; the exact quotient rounded up); nonsense stops the shape and never panics; the start phase; sustain-loop laps, equal loop points, a note released before the loop, clamped loop points; exact point positions and 200 laps of exactly 150 steps |
| `enth-engine` (module unit tests, 14) | against an independent `f64` model: the default triangle; every curve within 1e-6; step curves; the exact one-shot end for 13 synced and ms cases (including 90,000 segments); tempo-change exactness; a tempo change moves the speed and never a point; velocity and polarity; start phase; retrigger and legato; the 8-step glide; the global loop, one-shot and sustain-loop rules; nonsense values |
| `crates/enth-engine/tests/mseg.rs` (25 tests) | the whole engine: loop, one-shot and sustain-loop, held and released, voice and global; equal loop points; the exact synced end across tempo changes and at host blocks 1, 7, 32, 64, 100 and 4096; ms lengths; the curves through the engine; a 60 s envelope read every 15 segments; velocity; mono and legato; poly voices; the S7 rules (free-running loop, phrase restart, any-key gate, reset); the 8-step glide and the random-edit click bound; block-size independence on a busy patch; two instances; plan swap; a 40 s tail with no subnormals; FTZ independence; no allocation; the zipper bound below |
| `crates/enth-patch/tests/mseg.rs` (5 tests, 2 proptests) | all 59 parameters apply, validate and invert bit for bit; a run of point edits is one transaction that inverts and redoes; only `scope` is `Effect::Structural` (the rest are `Value`); the source span follows polarity |
| `catalogue/mod.rs` | `mseg_rate_follows_scope` |

### Cost

Callgrind, 1 s at 48 kHz (1,500 segments), a held chord, the shape and the engine both counted; `cargo xtask bench --bench mseg` (needs `GUNGRAUN_RUNNER` and valgrind). Budget = measured + 10 %, rounded up to 1,000. The `engine` bench with the same voices and no MSEG is 3,621,357 (1 voice) and 37,256,482 (16 voices).

| Case | Measured | Budget | Over the engine bench |
| --- | --- | --- | --- |
| `voice_loop_1v` (default triangle) | 4,415,521 | 4,858,000 | 16.5 per sample |
| `voice_loop_16v` | 41,951,591 | 46,147,000 | 6.1 per sample per voice |
| `voice_curves_16v` (16 bent points) | 43,207,231 | 47,528,000 | 7.8 per sample per voice |
| `voice_sustain_16v` (a held sustain loop) | 42,443,093 | 46,688,000 | 6.8 per sample per voice |
| `global_loop_16v` | 37,932,307 | 41,726,000 | 14.1 per frame (once, not per voice) |

A fixed part of about 350 instructions per segment (the parameter snapshot comparison, the cached increment and the module's dispatch) is why one voice costs more per sample than each of sixteen; the marginal voice is about 175 instructions per segment (5.4 per sample) for the default shape.

### Aliasing: the zipper skirt of a stepped pattern

An MSEG is not an audio path, so its own aliasing is not measured. What it can do to a destination is **zipper noise**: the engine ramps a modulator's value over each 32-frame segment, so a hard step in a shape becomes a 0.67 ms ramp, and a stepped pattern on a gain is an amplitude modulation with a ramped edge. The test records the skirt of such a pattern with the shared spectrum helper (`enth_dsp::alias`): a sine at key 69 (440 Hz) and key 93 (1.76 kHz), its `level` driven by the MSEG at 100 %, the last `FFT_LEN` frames of 4×`FFT_LEN` rendered, the worst bin in each band above the carrier relative to the carrier peak.

| Pattern | 2–4 kHz | 4–8 kHz | 8–16 kHz | Bound asserted |
| --- | --- | --- | --- | --- |
| 16-step gate (−100 % steps, 682.7 ms), key 69 | −65.9 dB | −77.3 dB | −88.1 dB | −63.5 / −75.0 / −86.0 |
| 16-step gate, key 93 | −65.6 dB | −77.6 dB | −88.1 dB | (same) |
| drawn wobble (5 points with curves, 85 ms), key 69 | −90.4 dB | −100.0 dB | −111.6 dB | −87.5 / −96.5 / −107.5 |
| drawn wobble, key 93 | −89.9 dB | −98.8 dB | −109.8 dB | (same) |

The skirt falls about 12 dB per octave: a step falls 6 dB per octave and the 0.67 ms ramp (a sinc whose first null is at 1.5 kHz) adds another 6. A hard-stepped gate sits near −66 dB at 2–4 kHz off its carrier; a smooth drawn shape is below −87 dB. Bounds are the worst of each band plus 2 dB (`the_zipper_skirt_of_a_stepped_pattern_stays_inside_its_recorded_bound`; `cargo test -p enth-engine --test mseg -- --nocapture` prints the table).

### Goldens

13 cases, `tests/golden/cases/mseg-*.toml`, tempo fixed at the render's default 140 BPM, generated with `cargo xtask golden --update` (only the 13 new expected files moved; the 87 existing cases are unchanged):

| Case | What |
| --- | --- |
| `mseg-wobble-cutoff` | a drawn five-point wobble, one loop per eighth (0.214 s at 140 BPM), on a filter cutoff |
| `mseg-gate-16` | sixteen points with −100 % curves, one bar, on level: a 16-step gate |
| `mseg-riser-pitch` | a 2 s one-shot with a late curve on pitch (24 semitones at the top) |
| `mseg-one-shot-level` | a one-shot attack-decay envelope of four curved points on level; a second note has its own |
| `mseg-sustain-loop` | a sustain loop between points 3 and 4 while held (note-off at 0.9 s), then played out |
| `mseg-edit-automation` | a sustain-loop hold with automation of a point's level and time mid-note (the glide) |
| `mseg-curves` | eight points with different bends (±100 %, ±60, ±25, 0), looping |
| `mseg-bipolar-velocity` | a bipolar loop with a velocity amount of 100 %, at velocity 1.0 and 0.3 |
| `mseg-phase-start` | a looping ramp started at 270 degrees by each of two notes |
| `mseg-mono-retrigger` | voice mode mono: each of three overlapping notes restarts a one-shot rise |
| `mseg-global-comp` | a global loop (a triangle every 667 ms) on `comp1.mix` |
| `mseg-global-phrase` | a global one-shot restarting at the first key of a phrase and not at a key played over another |
| `mseg-global-sustain` | a global sustain loop gated by any key: it keeps looping while the second key is down |

### Taste-check WAVs (offline renders, never played)

`~/code/enth-scratch/s2/mseg/wav/` (the patches are in `patches/`; rendered with `enth render --bpm 140`, master −12 dB, peaks −3.2, −6.7 and −4.0 dBFS, not clipped): `mseg-wobble-eighth.wav` (a 1/8-synced drawn wobble on cutoff, C2, 4 s), `mseg-gate-16.wav` (a 16-step gate on level, C3, 4 s: 14 of 16 slots as drawn) and `mseg-riser-4bar.wav` (a 4-bar one-shot riser on pitch and cutoff, C3, 8 s: the spectral centroid rises from about 300 Hz to 7 kHz over the 6.9 s the riser lasts). The numbers are checked offline; whether they sound right is the owner's ear.

## Notes for lane INT-S2a, S2b and the coordinator

- **Still hidden, `ENGINE_VERSION` not bumped.** The kind's persisted parameters and every existing hash, golden, fixture and snapshot are untouched; the lane only added files, dispatch arms and the 13 new golden cases.
- **`ModuleKind::rate`** has an `mseg` arm that follows `scope` (voice → voice modulator, global → global modulator), as `lfo` does; `source_span` in `modulation.rs` has an arm for it. Flipping `scope` is structural (recompiles the plan) and the state is carried by `copy_state` when the kind is unchanged.
- **Engine hooks other lanes may use:** `NoteState::keys_held` and `NoteState::phrase_start`, and `GlobalModulator::keys(held, phrase_start)` (a default no-op, called before the global `tick`). They are the "first key of a phrase" event the LFO page asked for in S7 and can serve a global one-shot LFO if the coordinator decides to give it the same rule (the LFO is unchanged).
- **`every_kind_patch`** in `crates/enth-engine/tests/common` and `crates/enth-patch/tests/common` still omits the S2a kinds; lane INT-S2a should add `mseg` (and its non-default settings) there when the kind is unhidden.
- **Editors:** a point edit is a `Value` effect, one transaction per drag; the engine glides over 5 ms. A mode, `points` or loop-point edit also glides. `scope` recompiles.
- **`cargo xtask bench`** needs `GUNGRAUN_RUNNER` pointing at a `gungraun-runner` of the crate's version.
- **Golden tempo:** the goldens use the render's 140 BPM; a case that wants another tempo needs the case format to express it (the format carries none today).

## Limitations

- 16 points, fixed (decision 1); a longer pattern would need a second MSEG.
- The curve is one rational family per segment; there is no S-curve (a bend that swings both ways) and no per-point smoothing.
- A global MSEG ignores velocity and has one position for all voices; a per-voice shape needs `scope` `voice`.
- Point parameters are `auto` only, not modulatable (CAT's table); only `length` can be modulated.
- The restart rule for a global one-shot is this lane's decision for F1m S7 (option (b) plus the any-key gate, which the brief left to the lane); the LFO still follows its page's option (a), and the coordinator may want the two to agree.

## Files

| What | Where |
| --- | --- |
| Parameter table | `crates/enth-patch/src/catalogue/mseg.rs` (lane CAT's F1m table, untouched) |
| DSP core | `crates/enth-dsp/src/mseg.rs` (shape, cursor, exact increments, 17 tests) |
| Engine module | `crates/enth-engine/src/modules/mseg.rs` (voice and global roles, the glide, 14 tests) |
| Role and span | `ModuleKind::rate` in `crates/enth-patch/src/catalogue/mod.rs`; `source_span` in `crates/enth-patch/src/modulation.rs` |
| Hooks | `crates/enth-engine/src/module.rs` (`NoteState`, `GlobalModulator::keys`), `crates/enth-engine/src/engine.rs` (three small additions) |
| Engine tests | `crates/enth-engine/tests/mseg.rs` |
| Patch tests | `crates/enth-patch/tests/mseg.rs` |
| Instruction budget | `crates/enth-engine/benches/mseg.rs` |
| Golden cases | `tests/golden/cases/mseg-*.toml` (13) |

## Acceptance (from phase-s2.md, every module lane)

The same bits on six targets (the goldens, checked native and under `wasm32-wasip1` with `cargo xtask golden --wasm`: 100 of 100 cases, the 13 new included); FTZ-independent (`the_render_is_the_same_under_every_ftz_mode`); zero subnormals after a 40 s tail; latency 0; a budget of the measured count plus 10 % (above); two instances keep separate state; aliasing: not an audio path, so the zipper skirt of a stepped pattern is recorded above and asserted in the test; no `THIRD_PARTY_NOTICES.md` entry (no port).
