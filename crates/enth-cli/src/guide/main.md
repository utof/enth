# enth guide (for agents)

enth renders a synth patch and **measures** it. You cannot hear the result, so work from the numbers; descriptors
approximate hearing. Each call prints one JSON object (`"schema": "enth.<cmd>/1"`); errors are JSON with a `hint`.

## Workflow
1. `enth new --template pluck_1osc --out p.patch.json` (`enth new --list`: init; pluck_1osc, a filter envelope `env2`
   on the cutoff; pad_2osc, a slow LFO `lfo1` on the cutoff; bass_sub, mono with a short glide).
2. Turn the goal into expectations (below), or use the list you were given.
3. `enth step p.patch.json --set filter1.cutoff=2kHz,env1.decay=200ms --expect '<list>' --perf '<perf>'`; repeat
   until it exits 0 (`check.pass: true`) with no `thin` item.
4. `enth get p.patch.json` shows where you are; `enth params <kind>` shows units and ranges.

## Commands
| command | does |
|---|---|
| `modules`, `params <kind or id>` | module kinds; params with unit, range, default, effect |
| `new --template T --out P` | start a patch |
| `apply P --set k=v,... or --ops JSON [--out P2] [--dry-run]` | edit (in place without `--out`); prints `inverse` ops |
| `get P [addr...]`, `diff A B` | read values; compare two patches |
| `render P [--out x.wav]` | float WAV and its levels |
| `describe P or x.wav` | every descriptor |
| `check P or x.wav --expect L` | pass/fail, margin, noise; exit 1 on a failure |
| `step P [--set ... or --ops ...] --expect L` | apply + render + describe + check in one call |
| `schema [name]`, `guide --topic expect, ops or descriptors` | JSON Schemas; more help |

## Edits
`--set` takes `address=value` pairs: `osc1.fine=7ct`, `osc2.semi=-12st`, `osc1.wave=pulse`, `filter1.cutoff=1.2kHz`,
`env1.attack=40ms`, `env1.release=1.5s`, `env1.sustain=0%`, `master.level=-6dB`. A bare number is in the
parameter's unit. Addresses are `<module id>.<key>` plus `voice.*` and `master.*`. Out of range is an error that
names the nearest valid value (`try`). Adding, removing and routing modules takes `--ops` (`--topic ops`).
Envelope `sustain` is a percent of full level; the descriptor `envelope.sustain_db` is in dB.

## Performance
Patches store no performance. Without flags a command plays C3, velocity 100, held 1.5 s, in a 3 s clip. Pass the
goal's performance on every `step` and `check`: `--perf note=A3,vel=100,hold=1.5s,dur=2.5s` (JSON form: `enth
schema perf`). The judge uses the goal's performance, so changing the note or the length never counts.

## Expectations
Comma-separated `<feature> <op> <value>`; ops `< <= > >= = ~ in [a, b] == null != null`; values take units:
`brightness > 2kHz, attack < 10ms, sustain > -6dB, pitch = A3 +- 30ct, beating in [3Hz, 9Hz], tremolo == null`.

| name | path | unit, meaning |
|---|---|---|
| pitch | pitch.f0_hz | Hz or a note; `=` compares in cents |
| attack | envelope.attack_ms | ms, onset to within 1 dB of the peak |
| decay | envelope.decay_ms | ms until the level settles near sustain (not the knob) |
| sustain | envelope.sustain_db | dB re the peak before note-off (-80 silent) |
| release | envelope.release_ms | ms from note-off to 40 dB down |
| duration | envelope.duration_ms | ms within 40 dB of the peak |
| brightness | spectrum.centroid_hz | Hz, spectral centroid |
| noisiness | noise_ratio | 0..1, 0 is a clean tone |
| beating | detune.beating_hz | Hz from detuned oscillators; null if none |
| tremolo | modulation.amp_lfo.hz | Hz of an amplitude LFO |
| loudness | loudness.lufs_i | LUFS |
| peak | loudness.peak_dbfs | dBFS of the float render |

## Reading check
- `margin`: how far inside (+) or outside (-) the threshold, in `unit` (`margin_unit` says `ct` for pitch).
- `noise`: how far the reading moves across two re-renders that differ only in oscillator start phase.
  `thin: true` means |margin| <= 2 x noise: it could flip. Aim further.
- `got: null` with `why`: not measurable (no beating; the note not held long enough).
- `fix`: one measured move for every failure; apply all its `ops` together with `--ops`. Its `after` values
  were measured. It may be null (also when nothing is `blocked`: raise `--fix-budget`).
- `blocked`: no parameter in reach can satisfy it (`reason` says why: `at_bound`, `conflict` with the items in
  `conflicts_with`, `no_effect`, `absent`, `performance`); change the structure, or it cannot be met with this
  performance. An item `hint` with `measured: false` says what to add.
- `lower_bound` or `estimated`: the clip ended first; lengthen `dur`.

## Gotchas
- Pitch is the played note; tune with `semi` and `fine`.
- `loudness.clipping: true` bends brightness and levels: lower `master.level` first.
- Detune between oscillators sets the beating: about f0 x cents / 1731 Hz.
- `envelope.beat_corrected: false` (a beat slower than the hold) leaves sustain a few dB uncertain.
- Ranges are hard limits: read `enth params <kind>` before a big move.

## Budget
One `step` is one render. Change 2 to 6 parameters per step. Stop when every item passes and none is `thin`.

## Exit codes
0 ok; 1 `check` had failures; 2 usage (flag, expectation, perf); 3 validation (unknown name, out of range, bad
patch); 4 render, analysis or write failure; 5 file not found. Errors carry `error`, `message`, `hint`,
`did_you_mean` and often `path`.

## Writing your own expectations (starting points)
| word | try |
|---|---|
| pluck | attack < 10ms, sustain < -30dB, duration < 800ms |
| pad | attack > 300ms, sustain > -6dB, release > 800ms |
| bass | pitch = C2 +- 30ct, sustain > -8dB, brightness < 1.5kHz |
| lead | attack < 20ms, sustain > -6dB, brightness > 2kHz |
| bell | decay > 600ms, sustain < -30dB |
| bright, dark | brightness > 2.5kHz; brightness < 800Hz |
| soft | attack > 20ms, brightness < 1.2kHz |
| detuned, chorused | beating in [1Hz, 8Hz] |
| noisy, breathy | noisiness > 0.3 |
| wobbly | tremolo != null |
