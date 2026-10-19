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
2. **Real units everywhere.** `filter1.cutoff=800Hz`, `env1.attack=40ms`, `osc2.detune=12ct`, `fx1.mix=30%`. The CLI parses unit strings into the parameter's plain value using the parameter schema ([design.md §1](design.md#1-one-parameter-schema-eight-consumers)); normalised 0–1 values never appear in the CLI.
3. **Stateless files.** Every command reads a patch file and, if it changes it, writes a new one (`--out`). History is files: agents can branch, diff and roll back. An optional daemon may come later purely for speed.
4. **Deterministic.** Fixed default seed, sample rate (48 kHz) and render length; sorted keys; numbers rounded to 4 significant digits.
5. **Self-describing.** `enth schema`, `enth modules`, `enth params <module>` and `enth guide` (a one-page workflow guide for agents) mean no outside docs are needed.
6. **Errors teach.** Exit codes are stable; error JSON says what was wrong and how to fix it:
   `{"error":"unknown_param","path":"filter1.cutof","did_you_mean":["filter1.cutoff"],"hint":"enth params filter"}`
7. **Small outputs.** Summaries by default; `--full` or `--fields` for more. Never dump a whole patch unless asked.
8. **Every edit is an op** — the same ops the UI emits and undo stores ([design.md §2](design.md#2-patch-model-ops-and-versioning)).

| Exit code | Meaning |
| --- | --- |
| 0 | OK (for `check`: all expectations passed) |
| 1 | Ran fine, but `check` had failures |
| 2 | Usage error (bad flags) |
| 3 | Validation error (bad patch, unknown param, value out of range) |
| 4 | Render or analysis failure |
| 5 | File not found |

## Commands

| Command | Purpose |
| --- | --- |
| `enth guide` | Print the agent workflow guide (~1 page) |
| `enth schema [patch\|ops\|describe\|check\|…]` | JSON Schemas for files and outputs |
| `enth modules` / `enth params <module>` | Catalogue: module kinds; params with units, ranges, defaults, one-line descriptions, perceptual effect |
| `enth new --template <name> --out p.patch.json` | Start from a template (`init`, `pluck_1osc`, `pad_2osc`, `bass_sub`…) |
| `enth random --seed 7 --difficulty 0.4 [--amounts filter=0.5,effects=0] [--musical 0.8] --out t.patch.json` | Generate a Recreate target with the same randomizer as the game ([design.md §9](design.md#9-randomizer-and-difficulty)) |
| `enth apply p.patch.json --ops '<json>' --out p2.patch.json [--dry-run]` | Apply ops; prints a diff |
| `enth get p.patch.json [addr…]` / `enth diff a.patch.json b.patch.json` | Read values; compare patches |
| `enth render p.patch.json [--note C3 --vel 100 --dur 2 --release 1 \| --perf perf.json] --out p.wav` | Deterministic render |
| `enth describe <wav\|patch>` | Perceptual descriptors (renders first if given a patch) |
| `enth check <wav\|patch> --expect '<list>'` | Pass/fail per expectation, with measured hints for failures |
| `enth compare <wav\|patch> --target t.wav` | Score 0–100, component distances, perceptual differences |
| `enth suggest p.patch.json --target t.wav \| --expect '<list>' [--top 5]` | Ranked parameter moves from measured sensitivities |
| `enth match p.patch.json --target t.wav --free <addrs> [--budget 300] --out p2.patch.json` | Optimiser fine-tunes only the freed params |
| `enth step p.patch.json --ops '<json>' [--expect …\|--target …] --out p2.patch.json` | apply + render + describe + check/compare in one call (fewest round trips) |
| `enth explain p.patch.json` | Plain-language explanation of the patch (doubles as lesson text) |
| `enth validate` / `enth migrate` | Check and upgrade patch files |
| `enth bench` | Engine speed: ×realtime per core, per-voice CPU |

## Files

**Patch** (`*.patch.json`, abridged). Values are plain numbers in each parameter's unit (`enth params svf` shows `cutoff` is Hz, `resonance` is %); enums are names; route depth is a percentage of the destination's range:

```json
{
  "schema": 1,
  "engine": "0.1.0",
  "meta": { "name": "Soft pluck", "tags": ["pluck"] },
  "voice": { "mode": "poly", "voices": 8, "glide": 0 },
  "generators": [
    { "id": "osc1", "kind": "analog", "params": { "wave": "saw", "level": -6, "detune": 0 } }
  ],
  "modulators": [
    { "id": "env1", "kind": "adsr", "params": { "attack": 2, "decay": 300, "sustain": 0, "release": 200 } }
  ],
  "lanes": [[ { "id": "filter1", "kind": "svf", "params": { "mode": "lp", "cutoff": 900, "resonance": 20 } } ]],
  "routes": [ { "src": "env1", "dst": "filter1.cutoff", "depth": 40 } ]
}
```

**Ops**:

```json
[
  {"set": "env1.attack", "to": "600ms"},
  {"add": "lfo", "id": "lfo1", "params": {"rate": "0.2Hz"}},
  {"route": "lfo1 -> filter1.cutoff", "depth": "30%"},
  {"remove": "fx2"}
]
```

## Descriptors (`enth describe`)

Stable feature names with units. These names are also what expectations, hints and `compare` use. Descriptor names carry their unit (`envelope.attack_ms`) because they are measurements of audio; parameter keys don't (`env1.attack`), because a parameter's unit comes from its schema.

| Feature | Unit / type | Meaning |
| --- | --- | --- |
| `pitch.f0_hz`, `pitch.note`, `pitch.cents_dev` | Hz, note, cents | Fundamental and tuning |
| `pitch.stability` | 0–1 | 1 = steady pitch |
| `envelope.attack_ms`, `decay_ms`, `sustain_db`, `release_ms` | ms, dB | Estimated amplitude envelope |
| `envelope.shape` | word | `pluck`, `stab`, `pad`, `swell`, `drone`, `percussive` |
| `spectrum.centroid_hz` | Hz | Brightness |
| `spectrum.centroid_track_hz` | [start, mid, end] Hz | Brightness over time (reveals filter envelopes) |
| `spectrum.rolloff85_hz`, `flatness`, `tilt_db_oct` | Hz, 0–1, dB/oct | Spectral shape |
| `spectrum.harmonics_db`, `odd_even_ratio` | dB list, ratio | Harmonic recipe (square-ish vs saw-ish) |
| `noise_ratio` | 0–1 | Noise vs tone |
| `modulation.pitch_lfo`, `modulation.amp_lfo` | `{hz, depth}` or null | Detected vibrato / tremolo (depth in cents / dB) |
| `modulation.filter_sweep` | `{kind, dir, ms, hz}` or null | `kind`: `env` (one-shot, `dir` up/down, `ms` duration) or `lfo` (cyclic, `hz` rate); unused fields are null |
| `detune.beating_hz`, `detune.voices_est` | Hz, count | Chorus/unison thickness |
| `stereo.width`, `stereo.corr` | 0–1, −1–1 | Stereo image |
| `loudness.lufs_i`, `peak_dbfs`, `clipping` | LUFS, dBFS, bool | Level |
| `words` | list | Controlled vocabulary: `bright`, `dark`, `plucky`, `wide`, `detuned`, `noisy`, `hollow`… |

Example output (~300 tokens):

```json
{"schema":"enth.describe/1","file":"p1.wav","dur_s":3.0,
 "pitch":{"f0_hz":130.8,"note":"C3","cents_dev":-2,"stability":0.98},
 "envelope":{"attack_ms":4,"decay_ms":310,"sustain_db":-9.5,"release_ms":420,"shape":"pluck"},
 "spectrum":{"centroid_hz":1840,"centroid_track_hz":[3100,1500,900],"rolloff85_hz":5200,"flatness":0.07,
             "tilt_db_oct":-5.1,"harmonics_db":[0,-6.1,-9.4,-12.2,-14.0,-16.3],"odd_even_ratio":1.0},
 "noise_ratio":0.04,"modulation":{"pitch_lfo":null,"amp_lfo":null,"filter_sweep":{"kind":"env","dir":"down","ms":400,"hz":null}},
 "detune":{"beating_hz":1.8,"voices_est":3},"stereo":{"width":0.18,"corr":0.91},
 "loudness":{"lufs_i":-14.2,"peak_dbfs":-1.1,"clipping":false},
 "words":["bright","plucky","slightly detuned","narrow"]}
```

## Expectations (`enth check`)

A comma-separated list of `<feature> <op> <value>`. Operators: `<`, `<=`, `>`, `>=`, `~` (within tolerance, e.g. `pitch.note ~ C3`), `in [a,b]`, `!= null`, `== null`.

```bash
enth check p1.wav --expect 'envelope.attack_ms > 200, spectrum.centroid_hz < 1500, stereo.width > 0.5, modulation.filter_sweep != null'
```

```json
{"schema":"enth.check/1","pass":false,
 "results":[
  {"expect":"envelope.attack_ms > 200","got":540,"pass":true},
  {"expect":"spectrum.centroid_hz < 1500","got":2310,"pass":false,
   "hint":{"param":"filter1.cutoff","now":"3.2kHz","try":"1.2kHz","expected":"centroid ≈ -800Hz","measured":true}},
  {"expect":"stereo.width > 0.5","got":0.18,"pass":false,
   "hint":{"text":"No unison or stereo effect in the patch. Add osc unison with spread, or a chorus in lane 1.","measured":false}},
  {"expect":"modulation.filter_sweep != null","got":{"kind":"lfo","dir":null,"ms":null,"hz":0.2},"pass":true}]}
```

Hints marked `"measured": true` come from rendering small parameter moves and observing the descriptor change. Others are structural suggestions from module metadata.

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
  {"param":"osc1.detune","from":"18ct","to":"6ct","score_after":74,"moves_feature":"detune.beating_hz -1.2Hz"}]}
```

`suggest` renders ± small moves for each editable parameter (cheap: the engine renders hundreds of short clips per second), so its numbers are measured, not guessed. `match` hands the fine-tuning to CMA-ES, restricted to the parameters the agent frees, so structure stays the agent's choice.

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

`content/agent-evals/` holds about 30 text goals (each with an expectation list) and about 30 reference sounds (rendered from hidden patches and from real recordings). `cargo xtask agent-eval` runs an agent against them and records:

| Metric | Why |
| --- | --- |
| Solve rate (all expectations pass / score ≥ threshold) | Does the loop work? |
| Renders used per solve | Is the feedback informative? |
| Tokens per solve | Are outputs compact? |
| Invalid-command rate | Are the CLI and its errors learnable? |

These numbers are the regression test for CLI changes, following Anthropic's advice to evaluate tools with real agent transcripts ([Writing effective tools for agents, Sep 2025](https://www.anthropic.com/engineering/writing-tools-for-agents)).

## Why CLI first, MCP later

- A local CLI is versatile: agents, humans, shell scripts and other programs can all call it, and it embeds anywhere.
- In one 2026 benchmark the CLI used far fewer tokens than an equivalent MCP server on the same tasks; another found them roughly equal ([research.md](research.md#6-agents-cli-vs-mcp)). The difference depends on how many tool schemas an MCP server loads.
- If a chat client without a shell needs it, a ~30-line MCP server can shell out to the same commands.
