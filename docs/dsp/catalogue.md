# The P1a module catalogue

Every parameter of every P1a module kind, as declared in `crates/enth-patch/src/catalogue/` (lane S1a, freeze point F1, 2026-10-19). Values are plain numbers in the unit shown; patches store them densely in this order. **The source of truth is the code and its snapshot** (`crates/enth-patch/tests/snapshots/catalogue__catalogue.snap`); this page is generated from it by `cargo test -p enth-patch --test catalogue -- --ignored --nocapture print_markdown` and may lag behind.

How to read it:

- **Range** is in plain units; `enth` parses `800Hz`, `1.2kHz`, `A4`, `40ms`, `0.5s`, `-6dB`, `+7st`, `12ct`, `+1oct`, `30%`, `4:1`, `90deg`, enum keys and `on`/`off`, and a bare number is in the parameter's unit (`30` on a percent parameter is 30 %).
- **Scale** maps the range onto the normalised 0–1 domain used by route depths, the randomiser, ML and host automation. `log` gives every octave the same share (10.034 % of 20 Hz–20 kHz); `power k` is `min + (max − min)·n^k` (power 4 puts 625 ms at the middle of 0–10 s). The scale is part of the persisted contract: changing it needs a migration.
- **Flags:** `mod` = modulation routes may target it; `auto` = an automation slot may expose it to the host; `structural` = changing it rebuilds the render plan (never modulated or automated); `periodic` = wraps instead of clamping.
- Route **depth** is a percentage of the destination's range in its normalised domain, −100..100; polarity belongs to the source (an LFO's `polarity`; envelopes, velocity, mod wheel and macros run 0–1; `key` is `(note − 60)/120`).
- Sources: any `adsr` or `lfo` module, `velocity`, `key`, `modwheel`, `macro1`…`macro8`. A per-voice source (an envelope, a voice LFO, velocity, key) may not target a global destination (`fx` modules, `master`, a global LFO).
- `phase_mode` (osc, lfo): `free` draws each note's start phase from the render seed (the descriptors' noise estimate varies it), `reset` starts at `phase`, `seeded` draws it from the module's own `seed`, so the patch fixes it.
- Limits: 4 generators, 4 voice effects, 4 effects, 8 modulators, 64 routes, 8 macros, 64 automation slots, polyphony ≤ 16.

### `osc` — Oscillator (generators, ids `osc1`, `osc2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `wave` | none | `sine`, `triangle`, `saw`, `pulse`, `table` | enum | saw | auto | basic | The waveform: sine, triangle, saw, pulse, or the wavetable chosen by table. |
| `semi` | semitones | -48st … +48st | linear | 0st | mod, auto | basic | Transposition in semitones from the played note (+12st is an octave up); route an envelope here for pitch sweeps. |
| `fine` | cents | -100ct … +100ct | linear | 0ct | mod, auto | basic | Fine tuning in cents; a few cents between two oscillators makes them beat and sound thicker. |
| `level` | percent | 0% … 100% | linear | 100% | mod, auto | basic | Output level of this oscillator into the voice, as a linear percentage. |
| `pw` | percent | 5% … 95% | linear | 50% | mod, auto | intermediate | Width of the pulse wave: 50 % is a square; other waves ignore it. Modulate it for PWM. |
| `sync` | none | off, on | toggle | off | auto | intermediate | Restart this oscillator with each cycle of the first enabled oscillator in generators: tune it higher for tearing, vocal tones. Ignored on that first oscillator. |
| `table` | none | `basic`, `pulse`, `tilt`, `formant`, `organ` | enum | basic | structural | intermediate | Which wavetable the table wave plays; changing it rebuilds the voice plan. |
| `position` | percent | 0% … 100% | linear | 0% | mod, auto | intermediate | Where in the wavetable to play, from its first frame to its last; only for the table wave. |
| `phase_mode` | none | `free`, `reset`, `seeded` | enum | free | auto | advanced | How each note's start phase is chosen: free (random per render seed), reset (the phase knob) or seeded (from seed). |
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | The start phase in reset mode; between two oscillators at the same pitch it changes the timbre. |
| `seed` | none | 0 … 65535 | int | 0 |  | advanced | The random seed used in seeded phase mode; part of the patch, unlike the render seed. |

### `noise` — Noise (generators, ids `noise1`, `noise2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `color` | none | `white`, `pink` | enum | white | auto | basic | White is bright hiss; pink is darker and more natural. |
| `level` | percent | 0% … 100% | linear | 100% | mod, auto | basic | Output level of the noise into the voice, as a linear percentage. |

### `svf` — Filter (voice_fx, ids `filter1`, `filter2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `lp`, `bp`, `hp`, `notch` | enum | lp | auto | basic | The filter response: lp, bp, hp or notch. |
| `cutoff` | hz | 20Hz … 20kHz | log | 2kHz | mod, auto | basic | The corner frequency: lower is darker for a low-pass; route an envelope here for plucks and sweeps. |
| `resonance` | percent | 0% … 100% | linear | 10% | mod, auto | basic | Emphasis at the cutoff, from none (Q 0.5) to a sharp whistling peak (Q 25). |
| `keytrack` | percent | 0% … 100% | linear | 0% | auto | intermediate | How much the cutoff follows the played note: 100 % moves it an octave per octave played. |
| `drive` | db | 0dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain into a soft saturator before the filter, normalised back after: adds grit and harmonics. |

### `adsr` — Envelope (modulators, ids `env1`, `env2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `attack` | ms | 0ms … 10s | power 4 | 5ms | mod, auto | basic | Time to rise from the current level to full at note-on; under 10 ms sounds plucky, over 200 ms swells. |
| `decay` | ms | 0ms … 10s | power 4 | 300ms | mod, auto | basic | Time to fall from full to the sustain level. |
| `sustain` | percent | 0% … 100% | linear | 70% | mod, auto | basic | The level held while the note is down, as a percentage of full; 0 % makes a pluck. |
| `release` | ms | 0ms … 20s | power 4 | 400ms | mod, auto | basic | Time to fall to silence after note-off. |
| `velocity` | percent | 0% … 100% | linear | 0% | auto | intermediate | How much note velocity scales this envelope's output: 0 % ignores velocity, 100 % follows it fully. |

### `lfo` — LFO (modulators, ids `lfo1`, `lfo2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `shape` | none | `sine`, `triangle`, `saw_up`, `saw_down`, `square`, `sample_hold`, `smooth_random` | enum | sine | auto | basic | The waveform of the modulation. |
| `rate` | hz | 0.01Hz … 50Hz | log | 2Hz | mod, auto | basic | Cycles per second: about 5 Hz for vibrato, under 0.5 Hz for slow sweeps. |
| `polarity` | none | `bipolar`, `unipolar` | enum | bipolar | auto | intermediate | Bipolar swings around the destination's value; unipolar only adds to it. |
| `scope` | none | `voice`, `global` | enum | voice | structural | intermediate | One LFO per voice or one global LFO; only a global LFO can modulate effects. Changing it rebuilds the plan. |
| `phase_mode` | none | `free`, `reset`, `seeded` | enum | reset | auto | advanced | How each cycle start is chosen: reset (the phase knob at each note-on), free (random per render seed) or seeded (from seed). |
| `phase` | degrees | 0deg … 360deg | linear | 0deg | auto, periodic | advanced | Where in its cycle the LFO starts in reset mode. |
| `seed` | none | 0 … 65535 | int | 0 |  | advanced | The random seed used in seeded phase mode, including the random shapes. |

### `reverb` — Reverb (fx, ids `reverb1`, `reverb2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `dry` | db | -60dB … 6dB | linear | 0dB | mod, auto | intermediate | Level of the untouched signal passing through; -60 dB is off, for a fully wet sound. |
| `wet` | db | -60dB … 6dB | linear | -6dB | mod, auto | basic | Level of the reverb tail: more wet sounds further away; -60 dB is off. |
| `room` | ms | 10ms … 200ms | linear | 80ms | auto | basic | Size of the space, as a delay time (1 ms is about a foot): small rooms to large halls. |
| `decay` | ms | 100ms … 30s | log | 3s | auto | basic | How long the tail takes to fall by 60 dB (RT60). |
| `early` | percent | 0% … 250% | linear | 150% | mod, auto | advanced | Level of the first reflections, which tell the ear the size of the room. |
| `detune` | none | 0 … 50 | power 3 | 2 | mod, auto | advanced | How much the pitch wobbles inside the tail (about cents per pass at a 1 s room): smears metallic ringing, chorus-like when high. |
| `low_cut` | hz | 10Hz … 500Hz | log | 80Hz | mod, auto | intermediate | Removes lows below this frequency from the reverb, keeping bass clean. |
| `high_cut` | hz | 1kHz … 20kHz | log | 12kHz | mod, auto | intermediate | Removes highs above this frequency from the reverb: lower sounds darker and softer. |
| `low_damp` | none | 1 … 10 | log | 1.5 | mod, auto | advanced | How much faster low frequencies die away than the decay time (1 = the same). |
| `high_damp` | none | 1 … 10 | log | 2.5 | mod, auto | advanced | How much faster high frequencies die away than the decay time (1 = the same): higher is darker. |

### `comp` — Compressor (fx, ids `comp1`, `comp2`…)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `threshold` | db | -60dB … 0dB | linear | -18dB | mod, auto | basic | Level above which the compressor turns the signal down; lower compresses more. |
| `ratio` | ratio | 1:1 … 20:1 | log | 4:1 | mod, auto | basic | How hard levels above the threshold are turned down: 4:1 lets 4 dB in become 1 dB out. |
| `attack` | ms | 0.1ms … 100ms | log | 10ms | auto | intermediate | How fast the compressor reacts to a rise in level; slower lets transients through. |
| `release` | ms | 10ms … 2s | log | 200ms | auto | intermediate | How fast the compressor lets go after the level falls. |
| `knee` | db | 0dB … 24dB | linear | 6dB | mod, auto | advanced | Width of the soft transition around the threshold; 0 dB is a hard knee. |
| `makeup` | db | 0dB … 24dB | linear | 0dB | mod, auto | intermediate | Gain added after compression to bring the level back up. |
| `mix` | percent | 0% … 100% | linear | 100% | mod, auto | intermediate | How much of the compressed signal is heard: 100 % is fully compressed, lower blends in the dry signal (parallel compression). |

### `voice` (built-in)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mode` | none | `poly`, `mono`, `legato` | enum | poly | structural | basic | poly plays chords; mono and legato play one note at a time (legato slides without retriggering). |
| `polyphony` | none | 1 … 16 | int | 8 | structural | intermediate | How many notes can sound at once in poly mode; extra notes steal the oldest voice. |
| `glide` | ms | 0ms … 10s | power 4 | 0ms | auto | basic | Time to slide from the previous note's pitch to the new one; 0 is off. Linear in semitones. |
| `glide_mode` | none | `always`, `legato` | enum | always | auto | intermediate | Glide on every note, or only on notes played legato. |
| `pan` | percent | -100% … 100% | linear | 0% | mod, auto | intermediate | Where each voice sits in the stereo field, -100 % left to +100 % right; route key or an LFO here to spread notes. |

### `master` (built-in)

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `level` | db | -36dB … 12dB | linear | 0dB | auto | basic | Output level after the effects; turn it down to fix clipping without retuning anything else. |

### Macro knob and route depth

| Key | Unit | Range | Scale | Default | Flags | Tier | What it does |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `value` | none | 0 … 1 | linear | 0 | auto | basic | The macro knob's position, 0 to 1; routes from the macro scale it by their depth. |
| `depth` | percent | -100% … 100% | linear | 0% |  | intermediate | How far a route moves its destination, as a percentage of the destination's range. |
