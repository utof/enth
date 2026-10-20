# The `enth` CLI — built for agents, usable by humans

**Goal: an LLM agent can build a patch on its own** — choose modules, set parameters, render, get feedback it can act on, and iterate until the sound meets a goal. The same CLI serves humans, shell scripts, ML pipelines and other programs. An MCP wrapper can be added later as a thin shim over it.

## The division of labour

LLMs are good at intent and structure ("a pluck needs a fast attack and a filter envelope") and bad at hearing fine detail or tuning numbers by ear. Current audio-input models perform near chance on basic attributes like pitch and brightness ([SonicBench, 2026](https://arxiv.org/html/2601.11039)). So the work is split three ways:

| Who | Does what |
| --- | --- |
| **The LLM** | Picks modules and routing, sets rough values in real units, writes expectations for text goals, explains choices |
| **The engine** | Renders deterministically and measures: descriptors, pass/fail checks, distances, and *measured* parameter sensitivities |
| **The optimiser** | Fine-tunes the parameters the LLM chose to free, when there is a reference sound |

![The agent loop: the LLM decides, the engine measures, the optimiser fine-tunes](media/agent-loop.svg)

## Design rules

1. **JSON by default when stdout is not a TTY**; tables for humans. One JSON object per call (JSONL for streams). Every object carries `"schema": "enth.<command>/<version>"`.
2. **Real units everywhere.** `filter1.cutoff=800Hz`, `env1.attack=40ms`, `osc2.fine=12ct`, `comp1.mix=30%`. The CLI parses unit strings into the parameter's plain value using the parameter schema ([design.md §1](design.md#1-one-parameter-schema-eight-consumers)); normalised 0–1 values never appear in the CLI.
3. **Stateless files.** Every command reads a patch file and, if it changes it, writes it back, or a new one with `--out`. History is files: agents can branch, diff and roll back. An optional daemon may come later purely for speed.
4. **Deterministic.** Fixed default seed, sample rate (48 kHz) and render length; sorted keys; numbers rounded to 4 significant digits (2026-10-19, as built: an integral number prints exactly, so `65535` and `48000` survive; the `ops` and `inverse` arrays stay exact so they can be replayed).
5. **Self-describing.** `enth schema`, `enth modules`, `enth params <module>` and `enth guide` (a one-page workflow guide for agents) mean no outside docs are needed.
6. **Errors teach.** Exit codes are stable; error JSON says what was wrong and how to fix it:
   `{"error":"unknown_param","path":"filter1.cutof","did_you_mean":["filter1.cutoff"],"hint":"enth params svf"}`
   `did_you_mean` also draws on the CLI's alias table: names an agent is likely to guess, mapped to the catalogue's keys (`detune` → `fine`, `res` → `resonance`). So `osc1.detune` gets `"did_you_mean":["osc1.fine"]` where edit distance alone would find nothing. The table lives in the CLI and only feeds suggestions; the catalogue keys, which patches store, have no aliases.
7. **Small outputs.** Summaries by default; `--full` or `--fields` for more. Never dump a whole patch unless asked.
8. **Every edit is an op** — the same ops the UI emits and undo stores ([design.md §2](design.md#2-patch-model-ops-and-versioning)).
9. **Types are the schema.** Each output is a Rust struct (`Serialize + Deserialize + JsonSchema`, `deny_unknown_fields`) and `enth schema <name>` prints its JSON Schema (draft 2020-12, from `schemars`). Tests snapshot the output (insta), deserialize it back into the struct, and snapshot the schema itself, so code and schema cannot drift. There is no `jsonschema` crate: its default features pull in a C build. Any change to an output's shape, or to a field's meaning (like `decay_ms` between the spike and P1), bumps its version and updates the snapshots (AGENTS.md).

| Exit code | Meaning | Error codes (`enth.error/1`, as built in P1a) |
| --- | --- | --- |
| 0 | OK (for `check`: all expectations passed) | |
| 1 | Ran fine, but `check` had failures | |
| 2 | Usage error (bad flags) | `usage` (clap, with the usage line in `hint`), `bad_expect`, `bad_perf`, `nothing_to_apply` |
| 3 | Validation error (bad patch, unknown param, value out of range) | every `enth-patch` code (`unknown_param`, `unknown_module`, `out_of_range` with `try`, `min`, `max`, `bad_value`, `wrong_unit`, `bad_json`, `unknown_op`, …), `unknown_feature`, `unknown_kind`, `unknown_template`, `unknown_topic`, `unknown_schema`, `not_a_patch` |
| 4 | Render or analysis failure | `render_failed`, `analysis_failed`, `unsupported_wav`, `write_failed` |
| 5 | File not found | `file_not_found` |

Every error object has `error`, `message`, `exit`, `hint` and `did_you_mean` (possibly empty), and where it applies `path` (an address such as `filter1.cutof`, an op such as `ops[2]`, or a path into a file such as `generators[1].params.cutoff`), `file`, `known` (the valid names, when the list is short) and `errors` (every problem, when a file has several). On a terminal the error is plain text on stderr instead.

## Commands

| Command | Purpose |
| --- | --- |
| `enth guide [--topic expect\|ops\|descriptors]` | Print the agent workflow guide (one markdown page, at most about 6 KB) |
| `enth schema [patch\|ops\|perf\|describe\|check\|…]` | JSON Schemas for files and outputs (the list without a name) |
| `enth modules` / `enth params <kind\|voice\|master\|id> [--patch p]` | Catalogue: module kinds; params with units, ranges (shown prominently: agents in the Phase 0 probe were surprised by range limits), defaults, accepted spellings, one-line descriptions, perceptual effect, and `reads_as` where a knob and its descriptor differ (2026-10-20: `envelope.release_ms` reads about half the `release` knob). A module id (`filter1`) resolves to its kind |
| `enth new --template <name> --out p.patch.json` / `enth new --list` | Start from a template (`init`, `pluck_1osc`, `pad_2osc`, `bass_sub`); the file is written dense and stamped with the engine version |
| `enth random --seed 7 --difficulty 0.4 [--amounts filter=0.5,effects=0] [--musical 0.8] --out t.patch.json` | Generate a Recreate target with the same randomizer as the game ([design.md §9](design.md#9-randomizer-and-difficulty)) |
| `enth apply p.patch.json --ops '<json>' \| --set k=v[,…] [--out p2.patch.json] [--dry-run]` | Apply ops (`--set` is sugar for `set` ops; both may be given, ops first); in place without `--out`. Prints the applied changes in real units (a route change adds `means`: what its depth does to the destination), the resolved ops and the inverse ops (exact). `--ops` takes inline JSON, `@file.json` or `-` (stdin) |
| `enth get p.patch.json [addr…] [--plain]` / `enth diff a.patch.json b.patch.json [--describe]` | Read values (unit strings; numbers with `--plain`; `filter1`, `voice`, `master`, `routes`, `modules`, `meta` as well as addresses; no address gives a summary of what differs from the defaults); compare patches (the changes, and ops that turn `a` into `b`, verified by applying them) |
| `enth render p.patch.json [--note C3 --vel 100 --hold 1.5 --dur 3 \| --perf perf.json] [--out p.wav] [--bits f32\|16\|24] [--seed n]` | Deterministic render. Writes a stereo 32-bit float WAV with an `enth` chunk holding the performance (`--bits 16\|24` for integer PCM, same chunk); reports `peak_dbfs`, `true_peak_dbtp`, `clipped_pct` and `clipping` over both channels |
| `enth describe <wav\|patch> [--perf perf.json]` | Perceptual descriptors (renders first if given a patch). A WAV needs its performance. Precedence: `--perf`, then the `enth` chunk that `render` wrote, then an inferred note-off (`perf.source` says which: `flag`, `default`, `wav`, `inferred`, `none`); otherwise decay, sustain and release come back null, and `missing` says why for every null descriptor |
| `enth check <wav\|patch> --expect '<list>' [--perf perf.json] [--no-fix] [--fix-budget 16]` | Pass/fail per expectation with its margin and noise, and one measured multi-parameter `fix` for all the failures |
| `enth compare <wav\|patch> --target t.wav` | Score 0–100, component distances, perceptual differences |
| `enth suggest p.patch.json --target t.wav \| --expect '<list>' [--top 5]` | Ranked parameter moves from measured sensitivities |
| `enth match p.patch.json --target t.wav --free <addrs> [--budget 300] --out p2.patch.json` | Optimiser fine-tunes only the freed params |
| `enth step p.patch.json --ops '<json>' \| --set k=v[,…] [--expect …\|--target …] --out p2.patch.json` | apply + render + describe + check/compare in one call (fewest round trips). It echoes only the features named in `--expect` plus a headline, unless `--full` |
| `enth explain p.patch.json` | Plain-language explanation of the patch (doubles as lesson text) |
| `enth validate` / `enth migrate` | Check and upgrade patch files |
| `enth bench [--patch p] [--voices 8] [--seconds 10]` | Engine speed: ×realtime per core, ns per voice per frame (wall-clock, information only; the regression gate is instruction counts in `xtask`) |

**CLI v0 (P1a) is 13 commands:** `schema`, `modules`, `params`, `new`, `apply`, `get`, `diff`, `render`, `describe`, `check`, `step`, `guide` and `bench`, each with a defined output shape. `random`, `compare`, `suggest`, `match`, `explain`, `validate` and `migrate` come later.

### CLI v0 as built (2026-10-19, P1a lane C)

`crates/enth-cli` is a thin binary over `enth_cli::run(args, stdin) -> Output { code, stdout, stderr }`; snapshot tests run in process.

| Schema id | Printed by |
| --- | --- |
| `enth.schema/1`, `enth.modules/1`, `enth.params/2`, `enth.guide/1` | `schema` (the list), `modules`, `params`, `guide` |
| `enth.new/1`, `enth.templates/1` | `new`, `new --list` |
| `enth.apply/2`, `enth.get/1`, `enth.diff/2` | `apply`, `get`, `diff` |
| `enth.render/1`, `enth.describe/2`, `enth.check/2`, `enth.step/2`, `enth.bench/1` | `render`, `describe`, `check`, `step`, `bench` |
| `enth.error/1` | every error |
| `enth.patch/1`, `enth.ops/1`, `enth.perf/1`, `enth.wavmeta/1` | input documents: a patch file, the ops JSON, a performance, the WAV chunk |

`enth schema <name>` prints the JSON Schema itself (draft 2020-12, `$id` set to the schema id, the `schema` field pinned with `const`).

Version 2 (2026-10-20, P1b lane EV, the G1 eval findings in [p1a-g1.md](p1a-g1.md)): `params` adds `reads_as`; `apply`, `diff` and `step` add `means` to route changes; `describe` adds `tonal` and `modulation.filter_sweep` and reads its envelope with a tremolo fitted out; `check` and `step` call a true/false item `thin` only when the re-renders disagree.

- **Performance.** Patch files (schema 1) carry no performance. Without flags every command plays the default: C3, velocity 100, held 1.5 s, in a 3 s clip (`perf.source: "default"`). `--perf` takes inline JSON (`enth.perf/1`; `schema` may be left out, so an eval goal's `perf` table works as is), a file, `-` (stdin) or the short form `note=A3,vel=100,hold=1.5s,dur=2.5s`; `--note`, `--vel`, `--hold`, `--dur` change one field of a single-note performance (`--hold` alone sets the clip to hold + 1.5 s). The judge passes the goal's performance, so agents must pass it on every `step` too. `perf.source` is `flag`, `default`, `wav`, `inferred` or `none` (there is no `patch` source while patches store no performance).
- **The WAV.** Stereo IEEE float (format 3, an 18-byte `fmt ` chunk and a `fact` chunk), then an `enth` chunk with `enth.wavmeta/1` JSON (`engine`, `sample_rate`, `seed`, `patch_hash`, `perf`), padded with spaces to an even length so no pad byte follows it (hound skips unknown chunks but not their pad byte), then `data`. `describe` and `check` on a WAV analyse the mean of its channels.
- **`check`.** The judged render uses the default seed (or `--seed`); the two nuisance re-renders use seeds derived from it, which move only the start phases of oscillators in `free` phase mode (the default). On a WAV the noise comes from re-analysing the signal delayed by 137 and 311 samples. Per item: `expect` (canonical), `got` (in `unit`), `pass`, `margin`, `margin_unit` (only when it differs from `unit`: `ct` for a pitch judged against a note), `margin_pct` (Hz and ms items), `noise`, `thin`, `note` (pitch and tonal-pitch items: `C3+9ct`), `why` (when `got` is null), and `lower_bound`, `estimated`, `unstable` when true. A true/false item has margin 0 and is `thin` only when a nuisance render disagrees with it (2026-10-20; before, every `clipping = false` was thin). `check` and `step` judge identically.
- **`fix` and `blocked`.** The search is lane H's `enth-solver::hint` ([dsp/hints.md](dsp/hints.md)); the CLI (`crates/enth-cli/src/fix.rs`) calls it only for a patch with failures, not with `--no-fix` and not with `--fix-budget 0`, with the goal's performance and a rayon evaluator that is the solver's own `measure` (so the hint's `before` is `check`'s reading, to the bit, for any number of workers). `fix.ops` are `{"set": "<addr>", "to": "<value with unit>"}` in the units the parameter shows (`1.199kHz`, `7.984%`): the values were rounded to four significant digits *before* the final render, so replaying them with `step --ops` reproduces `fix.after` exactly. `fix.renders` counts every evaluation of the search, the base included, against `--fix-budget` (default 16). `fix.note` says what still fails; `fix.after` holds booleans as booleans, like `got`. `blocked[]` carries `expect`, `why`, `reason` (`performance`, `at_bound`, `conflict`, `no_effect`, `absent` or `unmodelled`), `at_bound` (addresses) and `conflicts_with` (indices into `results`). For a blocked item whose structure is the limit (`no_effect`, `absent`, `at_bound`) the item also carries a `hint` with `"measured": false`: add a second oscillator for beating, an LFO routed to a level for tremolo, a filter for brightness, a noise source for noisiness, a filter with an LFO or envelope on its cutoff for a filter sweep, and for pitch the transposed oscillator to set back (`osc2.semi` to `0st`: the fix never moves `semi`), the noise to lower or `tonal_pitch` to judge instead (each with an `--ops` example that applies as written). Since 2026-10-20 `why` names the module behind the verdict (`enth-solver`'s `hint/culprit.rs`): on `bass_sub` at C2, "it reads C1, 12 st below the note: osc2 is transposed -12 st (osc2.semi) and sets the period". **A list can end with no `fix` and nothing `blocked`:** the search ran out of renders or steps before it could prove a verdict (a real conflict between two items at the default budget did); `fix.renders` of a later call, or a larger `--fix-budget`, continues.
- **Clipping first.** When an item about `clipping`, `clipped_pct`, `peak` or `true_peak` fails and lowering `master.level` makes every such item pass without failing a passing one (checked on the judged readings, since gain at the end of the chain moves only the level readings), the fix starts with exactly that: `master.level` lowered by the overshoot plus 1 dB (1.5 dB for `peak`/`true_peak` bounds), frozen while the search continues on the rest. When it is the whole fix it costs one render. Otherwise the solver searches the whole space as usual.
- **`step`.** Writes the patch (in place without `--out`) and a WAV beside it (`--wav` to choose), echoes a flat `describe` map of the features named in `--expect` plus a headline (`pitch.note`, `spectrum.centroid_hz`, `envelope.attack_ms`, `envelope.sustain_db`, `envelope.duration_ms`, `loudness.peak_dbfs`, `loudness.clipping`; everything with `--full`), the applied changes and the inverse ops.
- **Templates** live in `content/templates/*.patch.json` (sparse files, embedded in the binary), each with one piece of modulation so an agent meets routes and voice modes in a working patch (2026-10-19, P1a integration pass). `pluck_1osc`: a second envelope `env2` (1 ms attack, 220 ms decay, no sustain) routed to `filter1.cutoff` at 45 % (about +4 octaves) over a 1 kHz cutoff, so the note starts bright and closes. `pad_2osc`: a per-voice sine `lfo1` at 0.3 Hz routed to the cutoff at 10 % (about an octave either way) over 1 kHz, on top of the two detuned saws. `bass_sub`: `voice.mode = mono` with a 40 ms glide in `legato` glide mode (notes played while another is held slide; a single note is unaffected). `init` is unchanged. 2026-10-20: `pad_2osc` and `bass_sub` have `master.level` −6 dB (`pad_2osc` peaked at +1.9 dBFS at C3, `bass_sub` near 0 dBFS with 2.8 dB of phase noise); both now peak at −3.8 dBFS or lower.
- **The guide** is `enth guide` (6.0 KB of markdown in `enth.guide/1`'s `text`, raw markdown on a terminal) with topic pages `expect`, `ops` and `descriptors` (the last generated from `enth-analysis`'s vocabulary). 2026-10-20: it states how envelope knobs read (release about 0.5 × the knob), what a route's `means` is, `tonal_pitch` and the filter-sweep features.

## Files

**Patch** (`*.patch.json`, abridged). Values are plain numbers in each parameter's unit (`enth params svf` shows `cutoff` is Hz, `resonance` is %); enums are names; route depth is a percentage of the destination's range. Reading accepts sparse parameters (defaults fill in) and any key order; writing is always dense, in spec order. The sections are `generators` and `voice_fx` (per voice), `fx` (global) and `modulators`; `voice` and `master` are built-in, so `voice.glide` and `master.level` are ordinary addresses:

```json
{
  "schema": 1,
  "engine": "0.2.0",
  "meta": { "name": "Soft pluck", "tags": ["pluck"] },
  "voice": { "mode": "poly", "polyphony": 8, "glide": 0, "amp_env": "env1" },
  "master": { "level": 0 },
  "generators": [
    { "id": "osc1", "kind": "osc", "params": { "wave": "saw", "level": 100, "fine": 0 } }
  ],
  "modulators": [
    { "id": "env1", "kind": "adsr", "params": { "attack": 2, "decay": 300, "sustain": 0, "release": 200 } }
  ],
  "voice_fx": [ { "id": "filter1", "kind": "svf", "params": { "mode": "lp", "cutoff": 900, "resonance": 20 } } ],
  "fx": [],
  "routes": [ { "src": "env1", "dst": "filter1.cutoff", "depth": 40 } ]
}
```

**Ops**. One format, with values as plain numbers or unit strings. Every op has an inverse that is also expressible, so undo logs and future duel messages are serialisable:

```json
[
  {"set": "env1.attack", "to": "600ms"},
  {"set": "voice.mode", "to": "legato"},
  {"set": "master.level", "to": "-6dB"},
  {"add": "lfo", "id": "lfo1", "at": "modulators[1]", "params": {"rate": "0.2Hz"}},
  {"remove": "comp1"},
  {"move": "filter2", "to": "voice_fx[0]"},
  {"route": "lfo1 -> filter1.cutoff", "depth": "30%"},
  {"route": "env2 -> filter1.cutoff", "depth": "+1oct", "index": 2},
  {"unroute": "lfo1 -> filter1.cutoff"},
  {"enable": "reverb1", "to": false},
  {"macro": 1, "to": 0.5},
  {"slot": 7, "to": "filter1.cutoff"},
  {"amp_env": "env1"}
]
```

- `add` without `id` takes the lowest free `<prefix><n>` (`osc1`, `filter2`, `reverb1`, `comp1`; the prefix belongs to the kind: an `svf` is a `filter`, an `adsr` an `env`). `at` is `section[index]` and defaults to the end of the kind's section.
- `set` reaches `voice.*` and `master.*` too. `route` takes `index` to place the route in the summation order (undo uses it) and a depth in percent or musical units (`+1oct`, `+12st`), converted to a percentage at the parameter's current value. The change that `apply` and `step` print says what the stored percentage does (`means`, 2026-10-20, eval finding 2): `"osc1.semi swings -12st..+12st around 0st (-12 st / +12 st) as lfo1 swings -1 to 1 (bipolar); 12.5% of osc1.semi's -48st..+48st linear range"`. An LFO is bipolar unless its `polarity` is `unipolar`; envelopes, velocity, the mod wheel and macros run 0 to 1.
- Macros and slots are 1-based in text and 0-based in the engine; a macro's value is its knob position, 0–1.
- **`--set` sugar.** `enth apply p --set filter1.cutoff=1.2kHz,env1.attack=40ms` becomes `set` ops. The agents in the Phase 0 trial wrote 3 to 10 `--set` pairs per step and none used ops JSON, so `--set` stays.
- An unknown verb is a `unknown_op` error that lists the verbs.

## Descriptors (`enth describe`)

Stable feature names with units. These names are also what expectations, hints and `compare` use. Descriptor names carry their unit (`envelope.attack_ms`) because they are measurements of audio; parameter keys don't (`env1.attack`), because a parameter's unit comes from its schema.

Envelope descriptors are measured on a beat-corrected envelope: the beat rate is measured first, and the periodic part of the log envelope at that rate is fitted and divided out ([design.md §6](design.md#6-challenge-and-scoring)). Smoothing over a beat period (a crest or boxcar envelope) was tried and rejected: it reaches only 23 % invariance against 64 % for the fit, and it moves attack edges. In the Phase 0 probe ([spikes/agent-probe.md](spikes/agent-probe.md)), a raw envelope on a detuned patch put the "peak" on the first beat, so `attack_ms` read about 1/(4 × beat rate) whatever the attack was, beat nulls looked like decay, and `sustain_db` moved by several dB with where the hold time fell in the beat cycle. All three agents in the trial ran into it.

| Feature | Unit / type | Meaning |
| --- | --- | --- |
| `pitch.f0_hz`, `pitch.note`, `pitch.cents_dev` | Hz, note, cents | Fundamental and tuning: the waveform's period, 25 Hz–2 kHz (a sub oscillator an octave down reads the octave below) |
| `tonal.f0_hz`, `tonal.note`, `tonal.cents_dev` | Hz, note, cents | Pitch of the tonal part from the steady spectral peaks: found under noise where `pitch` is not (2026-10-20). Expectations: `tonal_pitch ~ A3` |
| `pitch.stability` | 0–1 | 1 = steady pitch |
| `envelope.attack_ms`, `decay_ms`, `sustain_db`, `release_ms`, `duration_ms` | ms, dB | Estimated amplitude envelope on the beat-corrected curve (a tremolo is fitted out too when there is no beat). `decay_ms` is the time until the level settles within a dB threshold of the sustain (not the ADSR parameter, and not the spike's "90 % of the way"). `release_ms` is extrapolated when the clip ends first |
| `envelope.beat_corrected`, `ripple_db`, `release_extrapolated` | bool, dB, bool | Honesty fields. `beat_corrected` is false when a beat was found but is too slow to remove (less than one period in the held part), so the sustain may be off by a few dB |
| `envelope.shape` | word | `pluck`, `stab`, `pad`, `swell`, `drone`, `percussive` |
| `spectrum.centroid_hz` | Hz | Brightness |
| `spectrum.centroid_track_hz` | [start, mid, end] Hz | Brightness over time (reveals filter envelopes) |
| `spectrum.rolloff85_hz`, `flatness`, `tilt_db_oct` | Hz, 0–1, dB/oct | Spectral shape |
| `spectrum.harmonics_db`, `odd_even_ratio` | dB list, ratio | Harmonic recipe (square-ish vs saw-ish) |
| `noise_ratio` | 0–1 | Noise vs tone |
| `modulation.pitch_lfo`, `modulation.amp_lfo` | `{hz, depth}` or null | Detected vibrato / tremolo (depth in cents / dB) |
| `modulation.filter_sweep` | `{kind, dir, ms, hz, depth_oct}` or null | Built 2026-10-20, from the spectral centroid over time. `kind`: `env` (one-shot, `dir` up/down, `ms` onset to 90 % of the change) or `lfo` (cyclic, `hz` rate); `depth_oct` the swing or change in octaves; unused fields are null. A tremolo does not move it. Expectations: `filter_lfo` (`.hz`), `filter_sweep` (`.depth_oct`), `sweep_ms` (`.ms`) |
| `detune.beating_hz`, `detune.depth_db`, `detune.voices_est` | Hz, dB, count | Chorus/unison thickness. `beating_hz` comes from a multi-harmonic estimator (0.15–25 Hz) that tells detune from a tremolo LFO; it is null below that band |
| `stereo.width`, `stereo.corr` | 0–1, −1–1 | Stereo image |
| `loudness.lufs_i`, `peak_dbfs`, `true_peak_dbtp`, `clipped_pct`, `clipping` | LUFS, dBFS, dBTP, %, bool | Level, measured on the engine's float render, so `peak_dbfs` can be above 0. `clipped_pct` is the share of samples at or beyond full scale; `clipping` is true when it is above 0. Clipping bends brightness (a centroid read 24 % high in one case, 116 % in another), so it is reported and fixed first: the patch's `master.level` (dB) undoes it without retuning every module (agents in the probe had no way back from clipping) |
| `perf.source` | word | Where the performance came from: `flag`, `default` (the CLI's default performance; patches store none), `wav` (the `enth` chunk), `inferred` or `none` |
| `words` | list | Controlled vocabulary: `bright`, `dark`, `plucky`, `wide`, `detuned`, `noisy`, `hollow`… |

Example output (~300 tokens):

```json
{"schema":"enth.describe/2","file":"p1.wav","dur_s":3.0,
 "pitch":{"f0_hz":130.8,"note":"C3","cents_dev":-2,"stability":0.98},
 "tonal":{"f0_hz":130.9,"note":"C3","cents_dev":-1},
 "perf":{"source":"default","dur_s":3,"notes":[{"note":"C3","vel":100,"on_s":0,"off_s":1.5}]},
 "envelope":{"attack_ms":4,"decay_ms":310,"sustain_db":-9.5,"release_ms":420,"shape":"pluck","beat_corrected":true},
 "spectrum":{"centroid_hz":1840,"centroid_track_hz":[3100,1500,900],"rolloff85_hz":5200,"flatness":0.07,
             "tilt_db_oct":-5.1,"harmonics_db":[0,-6.1,-9.4,-12.2,-14.0,-16.3],"odd_even_ratio":1.0},
 "noise_ratio":0.04,"modulation":{"amp_lfo":null,"filter_sweep":{"kind":"env","dir":"down","ms":400,"hz":null,"depth_oct":1.4}},
 "detune":{"beating_hz":1.8,"voices_est":3},"stereo":{"width":0.18,"corr":0.91},
 "loudness":{"lufs_i":-14.2,"peak_dbfs":-1.1,"true_peak_dbtp":-0.9,"clipped_pct":0,"clipping":false},
 "words":["bright","plucky","slightly detuned","narrow"]}
```

## Expectations (`enth check`)

A comma-separated list of `<feature> <op> <value>`. Operators: `<`, `<=`, `>`, `>=`, `~` (within tolerance, e.g. `pitch.note ~ C3`), `in [a,b]`, `!= null`, `== null`.

```bash
enth check p1.wav --expect 'envelope.attack_ms > 200, spectrum.centroid_hz < 1500, stereo.width > 0.5, modulation.filter_sweep != null'
```

```json
{"schema":"enth.check/2","pass":false,"passed":2,"failed":2,"thin":0,
 "results":[
  {"expect":"envelope.attack_ms > 200","got":540,"unit":"ms","pass":true,"margin":340,"noise":3},
  {"expect":"spectrum.centroid_hz < 1500","got":2310,"unit":"Hz","pass":false,"margin":-810,"noise":6},
  {"expect":"stereo.width > 0.5","got":0.18,"pass":false,"margin":-0.32,"noise":0.01,
   "hint":{"text":"No unison or stereo effect in the patch. Add osc unison with spread, or a chorus in fx.","measured":false}},
  {"expect":"modulation.filter_sweep != null","got":{"kind":"lfo","dir":null,"ms":null,"hz":0.2},"pass":true}],
 "fix":{"measured":true,"pass_all":false,"renders":14,
        "ops":[{"set":"filter1.cutoff","to":"1.2kHz"}],
        "after":{"spectrum.centroid_hz":1420,"envelope.attack_ms":540},
        "single_param_enough":true,
        "note":"apply these ops together; stereo.width still fails, see blocked"},
 "blocked":[{"expect":"stereo.width > 0.5","why":"no module in the patch can widen the stereo image","at_bound":[]}]}
```

The numbers are illustrative. A `fix` is one move for the whole list, rendered and measured: `after` holds real values from a render, not predictions. Structural suggestions (`"measured": false`, for example "add a second oscillator with detune") stay on the result, because no parameter of the current structure can create the missing descriptor. `check` still exits 1 when it offers a fix.

- **Margins and noise.** Every result carries `margin`: how far the measured value is inside (positive) or outside (negative) the threshold, in the feature's unit (`margin_pct` as well for Hz and ms features), and `noise`: the largest change of the reading over two nuisance re-renders (a common phase shift of the oscillators, plus a different inter-oscillator phase when they are detuned). A result is `thin: true` when |margin| ≤ 2 × noise. A pass at 3,211 Hz against `> 3 kHz` is thin, and agents need to see that: in the trial re-judged with the corrected descriptors, a 9 ms attack against `> 8 ms` had a noise of 1–2 ms. Two renders plus a describe cost about 50 ms on the toy synth.
- **One hint solves all failing items at once.** A one-parameter hint can break an item that already passes (in the probe, "lower detune" for decay pushed the beating out of its window). So `check` takes a least-squares step on the finite-difference Jacobian it measures, across every failing item, constrained to keep passing items passing, and returns it as one multi-parameter `fix` ([design.md §7](design.md#7-how-far-were-your-knobs-when-many-answers-are-right)). `pass_all` says whether it satisfies the whole list, and `single_param_enough` whether one parameter would have done. The probes use the goal's performance whenever the list has an envelope or beating item. `--fix-budget` caps the renders (default 16); `--no-fix` turns it off for scripts.
- **`blocked` says what cannot be done.** When no move of the free parameters satisfies an item, the result names the item and the parameters sitting at their bounds, and says when the performance or the structure is the limit (a held note lasts at least `perf.hold`; there is no noise source). It never offers a hint that trades one failure for another.
- **`check` on a WAV** gives margins and noise (the noise comes from re-analysing shifted windows, not re-rendering) but no `fix`: there is no patch to move.

## Comparing to a reference (`compare`, `suggest`, `match`)

```json
{"schema":"enth.compare/1","score":71,
 "distance":{"spectral":0.22,"envelope":0.35,"pitch":0.01,"modulation":0.10,"stereo":0.05},
 "differences":[
  {"feature":"spectrum.centroid_hz","ours":1840,"target":1120,"perceptual":"ours is noticeably brighter","severity":"high"},
  {"feature":"envelope.attack_ms","ours":4,"target":45,"perceptual":"target fades in; ours clicks","severity":"high"},
  {"feature":"detune.beating_hz","ours":1.8,"target":0.6,"perceptual":"ours sounds more chorused","severity":"medium"}]}
```

```json
{"schema":"enth.suggest/1","base_score":71,
 "moves":[
  {"param":"filter1.cutoff","from":"3.2kHz","to":"1.4kHz","score_after":80,"moves_feature":"spectrum.centroid_hz -690Hz"},
  {"param":"env1.attack","from":"2ms","to":"40ms","score_after":78,"moves_feature":"envelope.attack_ms +38ms"},
  {"param":"osc1.fine","from":"18ct","to":"6ct","score_after":74,"moves_feature":"detune.beating_hz -1.2Hz"}]}
```

`suggest` renders ± small moves for each editable parameter (cheap: the engine renders hundreds of short clips per second), so its numbers are measured, not guessed. `match` hands the fine-tuning to CMA-ES, restricted to the parameters the agent frees, so structure stays the agent's choice.

### `enth challenge` and `enth compare` as built (lane SKR, F2r; lane CL finishes them)

A first version for the round skeleton: the same functions the game's worker and the golden challenges call, so the CLI scores exactly what the plugin scores.

| Command | Output | Does |
| --- | --- | --- |
| `enth challenge new [--level N \| --code X] [--seed S]` | `enth.challenge-new/1` | A challenge from a level's preset (1–5, default 1) and a seed (default a fresh 40-bit one, so the code stays short; from 2^40 the long code), or the challenge a code names: the code and its form, the settings, the phrase, the target's fingerprint and its 8-digit prefix, the engine, and the `enth.challenge/1` JSON |
| `enth challenge show <code> [--reveal] [--out T.patch.json]` | `enth.challenge-show/1` | Decodes the code (case, hyphens, spaces and Crockford's aliases ignored), regenerates the target, checks a long code's fingerprint prefix (`target_mismatch`) and engine (`engine_differs`, a warning on stderr), and shows the phrase, the scoring, the randomized parameters' addresses and the draw count; **no target value** unless `--reveal`, which adds every randomized value beside the start's and the target patch (`--out` writes it) |
| `enth compare <patch> --code <code> \| --challenge FILE` | `enth.compare/1` | Scores the patch with `enth_challenge::score_round` and the scorers the challenge names (`sound/1` since lane SS; ST's structure and combined scores switch in by themselves): sound, structure and combined %, the pass mark, `passed`, the margin, the scorer ids and the full `enth.score-report/1`. **Exit 1 when it does not pass**, like `check` |

Errors carry the code's or the round's own reason and hint (`bad_check`, `bad_symbol`, `code_too_new`, `target_mismatch`, `out_of_range`, exit 3). Not yet: a WAV or patch reference (`--target`, the planned shape above), custom settings for `new` (amounts, `musical`, template), and `decode`; lane CL owns those and versions the schemas when the shapes change.

## Three typical workflows

**Text goal** ("make a warm evolving pad"):

1. `enth guide` once; `enth modules` if needed.
2. LLM writes expectations: `envelope.attack_ms > 300, spectrum.centroid_hz < 1500, stereo.width > 0.5, modulation.filter_sweep != null`.
3. `enth new --template pad_2osc`, then `enth step` with ops + `--expect` until `pass: true` or the render budget runs out.
4. `enth explain` to describe what it built.

**Reference sound** ("make this"):

1. `enth describe target.wav` → LLM picks a template and structure.
2. `enth step … --target target.wav` a few times for coarse moves; follow `suggest`.
3. `enth match --free <params>` to fine-tune; stop at the score threshold.

**Edit** ("less clicky, a bit brighter"):

1. `enth describe` before → LLM picks ops → `enth step` → `enth diff` on the descriptors.

## Evaluating the CLI like a product

`content/agent-evals/` holds text goals (each with an expectation list) and, from R1, reference sounds (rendered from hidden patches and from real recordings). 2026-10-19: P1a ships the harness and the 10 starter text goals, because G1 needs them; R1 grows the set to about 30 goals and about 30 reference sounds. `cargo xtask agent-eval` runs an agent against them and records:

| Metric | Why |
| --- | --- |
| Solve rate (all expectations pass / score ≥ threshold) | Does the loop work? |
| Renders used per solve | Is the feedback informative? |
| Tokens per solve | Are outputs compact? |
| Invalid-command rate | Are the CLI and its errors learnable? |

These numbers are the regression test for CLI changes, following Anthropic's advice to evaluate tools with real agent transcripts ([Writing effective tools for agents, Sep 2025](https://www.anthropic.com/engineering/writing-tools-for-agents)).

**Protocol** (2026-10-19, from the Phase 0 agent-loop probe and risk #8 of the plan-risks report; it is the G1 gate in [roadmap.md](roadmap.md#p1--minimal-synth--cli-v0-46-weeks)):

- When the expectation list is given, the loop is easy: in the probe three fresh agents (Claude Sonnet 5.5) passed 15 of 15 goals on a 10-parameter toy synth in 1–7 renders each, and a mechanical hint follower passed 4 of 5. So a solve rate on given lists measures whether descriptors and hints are consistent, not whether the sound matches the words. 2026-10-19, P1a research: part of that 15 of 15 was a descriptor bug. Re-judged with beat-corrected descriptors, 13 of the 15 final patches pass and 3 of those 13 pass within noise; and a mechanical follower using multi-parameter hints passes 5 of 5 goals in one hint step each.
- **Given lists:** each text goal comes with its expectation list. 3 runs per goal; report the median renders, tokens and cost, and record the model name and the date.
- **A blind listen:** you judge whether the final sounds match their text, without seeing the numbers; G1 needs at least 7 of 10.
- **Agent-written lists:** a second, harder track where the agent writes its own expectations from the brief. Reported separately, not gated at G1. Both are scored: does the agent's own list pass (self-consistency), and does the hidden designer list pass (the real score).
- `cargo xtask agent-eval` runs nightly or on demand, **never as a merge gate**: it needs API access, costs money and is not deterministic.

### The harness (P1a, lane E)

Goals live one per file in `content/agent-evals/goals/<id>.toml`, so reviews and diffs stay small:

```toml
schema = "enth.agent-eval/1"
id = "pluck-bright-short"
difficulty = 1                       # 1..5
brief = "a short bright pluck"       # the only text the agent gets on the self-written track
perf = { notes = [{ note = "A3", vel = 100, on_s = 0.0, off_s = 1.5 }], dur_s = 2.0 }
template = "init"                    # the starting patch (templates are part of the product)
expect = "envelope.attack_ms < 10, spectrum.centroid_hz > 2kHz, envelope.sustain_db < -30, envelope.duration_ms < 800, pitch.note ~ A3"
budget = { renders = 20, turns = 60, usd = 1.0 }
```

The prompt templates (`prompts/given.md`, `prompts/self.md`) are versioned; changing one is an eval-version change. Committed results are summaries only (`results/<date>-<model>.json`); transcripts, patches and WAVs go to `target/agent-eval/` and stay out of git.

`cargo xtask agent-eval [--track given|self|both] [--goals …] [--trials 3] [--model …] [--runner claude-code] [--max-usd 20] [--jobs 1]` runs each trial in an empty temp dir whose only tool is a wrapper named `enth`. The wrapper logs every call and refuses the 21st render. It drives headless Claude Code:

```bash
claude --bare -p "$(render prompt)" --output-format stream-json --verbose --model <id> \
  --max-turns 60 --max-budget-usd 1.00 --tools "Bash,Read" --allowedTools "Bash(enth *)" "Read" \
  --permission-mode dontAsk --no-session-persistence
```

The judge is `enth check <the agent's patch> --perf <goal perf> --expect <goal list>` (exit 0 is solved), and it also records whether the patch came from `enth` rather than hand-written JSON. `--bare` needs an API key; on a subscription, drop it and set the `CLAUDE_CODE_DISABLE_*` variables and a clean `CLAUDE_CONFIG_DIR`, so personal Claude Code instructions stay out of the agent's context. Claude Code has no seed or temperature flag, so runs are not reproducible: that is why there are 3 trials.

Per goal and track it records the solve rate over 3 trials (pass@3 and pass^3), median and maximum renders, tokens and cost (`total_cost_usd` is a client-side estimate), the invalid-command rate (exit 2 or 3 calls over all calls), wall time and the final margins, with the model id, Claude Code version, enth git hash and eval version in the header. For the blind listen it picks one passing trial per goal (a seeded choice, median renders), loudness-matches the 10 WAVs to −23 LUFS, shuffles the names and shows only the brief; `agent-eval listen` is run by the owner. Estimated cost: about $0.14 per trial on Sonnet 5.5, so about $9 for 10 goals × 3 trials × 2 tracks (an estimate; the first run measures it).

## Why CLI first, MCP later

- A local CLI is versatile: agents, humans, shell scripts and other programs can all call it, and it embeds anywhere.
- In one 2026 benchmark the CLI used far fewer tokens than an equivalent MCP server on the same tasks; another found them roughly equal ([research.md](research.md#6-agents-cli-vs-mcp)). The difference depends on how many tool schemas an MCP server loads.
- If a chat client without a shell needs it, a ~30-line MCP server can shell out to the same commands.
