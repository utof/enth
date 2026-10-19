# enth guide: ops

Every edit is an op. `--ops` takes a JSON array of ops (or one op), inline, from a file (`@ops.json`) or from stdin
(`-`). `--set k=v,k=v` is sugar for `set` ops. Ops apply in order, each to the patch the previous ones left, all or
nothing. `apply` and `step` print the `inverse` ops that undo the edit.

```json
[
  {"set": "env1.attack", "to": "600ms"},
  {"set": "voice.mode", "to": "legato"},
  {"set": "master.level", "to": "-6dB"},
  {"add": "lfo", "id": "lfo1", "params": {"rate": "0.2Hz"}},
  {"add": "svf", "at": "voice_fx[0]", "params": {"mode": "hp", "cutoff": "200Hz"}},
  {"remove": "comp1"},
  {"move": "filter2", "to": "voice_fx[0]"},
  {"route": "lfo1 -> filter1.cutoff", "depth": "30%"},
  {"route": "env2 -> filter1.cutoff", "depth": "+2oct"},
  {"unroute": "lfo1 -> filter1.cutoff"},
  {"enable": "reverb1", "to": false},
  {"amp_env": "env2"},
  {"macro": 1, "to": 0.5, "name": "Brightness"},
  {"slot": 7, "to": "filter1.cutoff"},
  {"label": "osc1", "to": "Body"},
  {"meta": {"name": "Soft pluck", "tags": ["pluck"]}}
]
```

| verb | keys | does |
|---|---|---|
| `set` | `to`: number, unit string, enum key or bool | set a parameter, also `voice.*` and `master.*` |
| `add` | kind; `id`, `at` (`section[i]`), `params`, `enabled`, `label` | add a module; no `id` takes the next free one (`osc2`, `filter1`, `env2`, `lfo1`) |
| `remove` | module id | remove it with its routes and slots |
| `move` | `to`: `section[i]` | reorder within its section |
| `route` | `"src -> dst"`, `depth`, `index` | add a route or change its depth |
| `unroute` | `"src -> dst"` | remove a route |
| `enable` | `to` (default true) | bypass or restore a module |
| `amp_env` | an `adsr` id | choose the amplitude envelope |
| `macro`, `slot`, `label`, `meta` | see above | knobs, host slots, names |

- Kinds: `osc`, `noise` (in `generators`, at most 4), `svf` (in `voice_fx`, at most 4), `adsr`, `lfo` (in
  `modulators`, at most 8), `reverb`, `comp` (in `fx`, at most 4). `enth modules` lists them.
- Route sources: `adsr` and `lfo` modules, `velocity`, `key`, `modwheel`, `macro1`..`macro8`. Destinations must be
  modulatable (`modulatable: true` in `enth params`). A per-voice source (an envelope, a voice LFO, velocity, key)
  can only drive per-voice destinations (generators, filters, `voice.pan`).
- Depth is percent of the destination's range (`30`, `30%`), or a musical interval converted at the destination's
  current value (`+1oct`, `+12st`, `-500ct`). On a cutoff (20 Hz to 20 kHz, log) 10 % is about one octave.
- A number never stands for an enum item: write `"wave": "saw"`, not `2`.
- An unknown verb is `unknown_op`, with the verbs listed.
