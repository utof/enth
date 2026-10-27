# Agent evals: the goal set

- Date: 2026-10-27 (S2a lane EV5). Eval version 4, 41 text goals: the 30 of version 3 (2026-10-20, R1/S1 lane EV2) and 11 new S2a goals. Engine 0.3.0. **No paid run has been made on version 4** ([record](#results-and-the-paid-run-record-eval-version-4)).
- What it is: the set of text goals an agent designs sounds for with the `enth` CLI, and the checks that keep each goal reachable and honest. The protocol (tracks, trials, the wrapper, the judge, the blind listen) is in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product); the P1a history and the first paid run are in [p1a-g1.md](p1a-g1.md#6-the-agent-evaluation).
- The lesson track (R2, lane EV3: an agent completes whole lessons of the basics pack through `enth lesson`) is a second set beside this one, [below](#the-lesson-track-r2-lane-ev3). Everything on this page above it is the text goals.
- Files: `content/agent-evals/goals/<id>.toml` (brief, performance, designer list, rationale, and from version 4 an optional `requires`), `witness/<id>.toml` (a hand-picked solution, never shown to the agent), `prompts/`, `eval.toml` (the version and why it changed), `targets/targets.toml` (a placeholder for the reference sounds, [below](#reference-targets-not-built)).

## The set (eval version 4)

Forty-one goals, difficulty 1 to 5 (2, 8, 10, 12 and 9 goals): the thirty of version 3 in seven families, and eleven S2a goals in five more (family column `S2a ...`, tag `s2a`; [below](#the-s2a-families-eval-version-4)). Every goal starts from the blank `init` template (one saw, the default envelope). "Reads" is what the witness's one render measures under the judge.

| d | Goal | Family | Brief | Key expectations | Note, hold/clip | Witness reads |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `bass-sub-sine` | bass | a deep, clean sub bass | brightness < 80 Hz, sustain > -3 dB, `beating == null` | A1, 1.5/2.5 s | centroid 56.6 Hz, release 74 ms |
| 1 | `pluck-bright-short` | pluck | a short bright pluck | attack < 10 ms, brightness > 2 kHz, sustain < -30 dB | A3, 1.5/2 s | centroid 2.4 kHz, duration 181 ms |
| 2 | `bass-808-boom` | bass, percussion | a booming 808-style sub kick that rings for about a second | brightness < 120 Hz, duration 600–1400 ms, sustain < -30 dB | A1, 1.2/1.5 s | duration 979 ms, sustain -48.8 dB |
| 2 | `hat-open` | percussion | an open hi-hat: a bright noise hiss that rings for about half a second | noisiness > 0.8, brightness > 7 kHz, duration 250–800 ms, `pitch == null` | C3, 1/1.5 s | centroid 14.3 kHz, duration 555 ms |
| 2 | `keys-organ` | keys | a clean organ tone that starts and stops instantly | sustain > -1.5 dB, release < 60 ms, brightness 0.5–1.5 kHz | C4, 1.5/2.5 s | centroid 865 Hz, release 22 ms |
| 2 | `kick-thump` | percussion | a short kick-like thump | brightness 70–200 Hz, duration 150–450 ms, pitch ~ A1 | A1, 0.3/1 s | centroid 167 Hz, duration 204 ms |
| 2 | `lead-chip-pulse` | lead | a chiptune square lead: hard on, hard off, no movement | attack < 5 ms, release < 40 ms, stability > 0.95, `tremolo == null` | C5, 1/1.5 s | release 8 ms, stability 0.999 |
| 2 | `pad-slow-dark` | pad | a slow dark pad | attack > 400 ms, brightness < 700 Hz, beating 0.5–3 Hz | C3, 3.5/6 s | attack 605 ms, beating 1.21 Hz |
| 2 | `perc-woodblock` | percussion | a short, high wooden click like a woodblock | duration 40–200 ms, pitch ~ E5, brightness 0.5–2 kHz | E5, 0.3/1 s | duration 105 ms, centroid 1.23 kHz |
| 2 | `pluck-marimba` | pluck | a soft wooden marimba-like note | brightness 300–900 Hz, duration 150–600 ms | C4, 1/1.5 s | centroid 581 Hz, duration 358 ms |
| 3 | `bass-buzzy` | bass | a buzzy bass | brightness 1.2–2.5 kHz, sustain > -6 dB, pitch ~ C2 | C2, 1.5/2 s | centroid 1.64 kHz |
| 3 | `bass-detuned-fat` | bass | a fat detuned bass, warm rather than buzzy | beating 0.6–2 Hz, brightness 350 Hz–1 kHz | E2, 2/3 s | beating 1.05 Hz, centroid 593 Hz |
| 3 | `bass-fold-sine` | S2a distortion | a sine bass folded into a buzzy tone by a sine wavefolder | brightness 0.6–1.8 kHz, noisiness < 0.05, no clipping; requires `dist.mode = sine_fold` | A1, 1.5/2.5 s | centroid 1.12 kHz, noisiness 3e-5 |
| 3 | `bass-reese-unison` | S2a unison | a reese bass: detuned unison copies of one oscillator, low-passed | brightness 220–600 Hz, noisiness 0.04–0.4, `beating == null`; requires `wt.unison >= 3`, `svf` | E1, 3/4 s | centroid 328 Hz, noisiness 0.093 |
| 3 | `fx-reverse-swell` | FX | a reverse swell: a tone that fades in over the whole note and cuts off dead at the end | attack > 1 s, sustain > -1.5 dB, release < 60 ms | C4, 2/3 s | attack 1544 ms, release 12 ms |
| 3 | `hat-closed-noise` | percussion | a short closed hi-hat made of noise | noisiness > 0.8, brightness > 9 kHz, duration 25–120 ms | C3, 0.3/1 s | centroid 12 kHz, duration 44 ms |
| 3 | `keys-tremolo` | keys, modulation | an electric-piano-like key with a gentle tremolo | tremolo 3.5–7 Hz at 2.5–12 dB, sustain -20 to -4 dB | C4, 2.5/3.5 s | tremolo 5.0 Hz, 5.3 dB |
| 3 | `lead-vibrato` | lead, modulation | a singing saw lead with a clear vibrato | stability 0.4–0.85, `beating == null` | A4, 2/3 s | stability 0.586 |
| 3 | `pad-trance-gate` | pad, modulation | a trance-gate chop: a sustained saw chopped into fast rhythmic pulses | tremolo 5–10 Hz, depth > 12 dB | A3, 3/4 s | tremolo 8.0 Hz, 31.8 dB |
| 3 | `tom-low` | percussion | a low floor tom: a deep tonal thud with a little noise | tonal_pitch ~ G2, noisiness 0.08–0.6, duration 200–700 ms | G2, 0.6/1.5 s | noisiness 0.18, tonal 98.2 Hz |
| 4 | `bass-acid-squelch` | bass, modulation | an acid bass: a resonant filter that snaps shut on every note | filter_sweep > 1.2 oct, sweep_ms 80–500 ms, brightness 250 Hz–1.2 kHz | A2, 1.5/2.5 s | sweep 1.91 oct, 192 ms |
| 4 | `bass-dist-ott` | S2a distortion | a loud, dense, bright bass: a distortion on the voice, then OTT on the mix | brightness 1.2–2.4 kHz, loudness > -18 LUFS, decay < 30 ms; requires `dist`, `ott` | A1, 2/3 s | centroid 1.45 kHz, -15.9 LUFS |
| 4 | `bass-fm-growl` | S2a FM | a growling bass made with FM, hardest at the start of the note | brightness 380–900 Hz, filter_sweep > 1 oct over 100–600 ms, noisiness < 0.1; requires `wt x2`, `wt -> wt.pm` | A1, 2/3 s | sweep 1.70 oct, 245 ms, centroid 501 Hz |
| 4 | `bass-formant-talk` | S2a chain | a talking bass: a formant filter whose vowel opens and closes about twice a second | filter_lfo 1.5–3 Hz, filter_sweep > 0.5 oct, brightness 0.8–2 kHz; requires `formant`, `lfo -> formant.*` | A1, 4/5 s | 2.00 Hz, 0.69 oct |
| 4 | `bass-wobble-eighth` | S2a synced wobble | a wobble once per eighth note, locked to the tempo (140 BPM) | filter_lfo 4–5.5 Hz, filter_sweep > 1.5 oct; requires `lfo.sync = on`, `lfo -> svf.cutoff` | A1, 4/5 s | 4.667 Hz, 3.02 oct |
| 4 | `bell-soft` | keys | a soft bell-like tone | attack 8–40 ms, decay > 600 ms, sustain < -35 dB, beating 2–8 Hz | C5, 3/5 s | beating 4.24 Hz, sustain -39.8 dB |
| 4 | `brass-ish` | lead | a brassy sustained note with a slow swell | attack 40–120 ms, brightness 1.5–3 kHz, beating 0.7–2 Hz | E3, 2/3 s | attack 51 ms, beating 1.53 Hz |
| 4 | `filter-wobble` | bass, modulation | a low saw with a slow wobbling filter sweep | filter_lfo 1.5–3.5 Hz, filter_sweep > 1 oct | A2, 4/5 s | 2.00 Hz, 2.97 oct |
| 4 | `fx-noise-riser` | FX | a noise riser: a whoosh that builds up and brightens over the whole note | attack > 1.2 s, filter_sweep > 2 oct, sweep_ms > 1 s, brightness > 3 kHz | C3, 3/4 s | sweep 3.86 oct, 2060 ms, centroid 8.1 kHz |
| 4 | `pluck-comb-string` | S2a chain | a plucked string from a noise burst, rung by a comb filter | tonal_pitch ~ A3, attack < 25 ms, duration 150–700 ms; requires `noise`, `comb` | A3, 1/2.5 s | duration 341 ms, tonal 220.3 Hz |
| 4 | `pluck-hall` | pluck, effects | a plucked note ringing out in a big reverberant hall | release > 1 s, duration > 1.8 s, sustain -45 to -10 dB, noisiness > 0.02 | E4, 1/4 s | duration 3317 ms, noisiness 0.14 |
| 4 | `texture-wind` | texture, modulation | wind: white noise swept slowly up and down by a filter | filter_lfo 0.3–1.5 Hz, filter_sweep > 0.5 oct, noisiness > 0.8 | C3, 5/7 s | 0.80 Hz, 1.31 oct |
| 5 | `bass-chain-ladder-phaser-eq` | S2a chain | a dark swirling bass: ladder low-pass, slow phaser, equaliser | filter_lfo 0.7–1.2 Hz, filter_sweep > 0.3 oct, brightness 450–900 Hz; requires `ladder`, `phaser`, `eq` | A2, 4/5 s | 0.90 Hz, 0.67 oct |
| 5 | `bass-fm-growl-wobble` | S2a FM | a fast FM growl whose modulation depth swings about six times a second | filter_lfo 5–7 Hz, filter_sweep > 1.5 oct, noisiness > 0.1; requires `wt x2`, `wt -> wt.pm`, `lfo -> wt.pm_index` | A1, 3.5/4.5 s | 6.00 Hz, 3.17 oct |
| 5 | `bass-reese-moving` | S2a unison | a reese that breathes: unison copies, low-pass swept slowly | filter_lfo 0.9–1.6 Hz, filter_sweep > 1.5 oct, `beating == null`; requires `wt.unison >= 3`, `lfo -> svf.cutoff` | E1, 5/6 s | 1.20 Hz, 2.98 oct |
| 5 | `bass-wobble-dotted-mseg` | S2a synced wobble | a wobble drawn with an MSEG, one cycle per dotted eighth | filter_lfo 2.6–3.6 Hz, filter_sweep > 1 oct; requires `mseg.sync = on`, `mseg -> svf.cutoff` | A1, 4/5 s | 3.108 Hz, 1.86 oct |
| 5 | `fx-laser-zap` | FX, modulation | a laser zap: a short tone whose pitch dives fast | filter_sweep > 0.8 oct, sweep_ms < 300 ms, stability < 0.5 | C4, 0.5/1.2 s | sweep 1.13 oct, 181 ms, stability 0 |
| 5 | `lead-flute-breathy` | lead | a breathy flute-like tone | tonal_pitch ~ A4, noisiness 0.04–0.3, attack 40–250 ms | A4, 2/3 s | noisiness 0.10, attack 137 ms |
| 5 | `lead-supersaw` | lead | a detuned supersaw-ish lead | brightness 3.5–6 kHz, beating 4–8 Hz, duration 2–2.6 s | A4, 2/3 s | beating 5.60 Hz, duration 2196 ms |
| 5 | `pad-moving-bright` | pad, modulation | a lush detuned pad with a slow, breathing filter movement | filter_lfo 0.4–1.5 Hz, beating 0.8–4 Hz, attack > 300 ms | C4, 5/7 s | 0.81 Hz, beating 1.81 Hz |
| 5 | `snare-crack` | percussion | a snare-like crack: a noise burst over a short tonal body | noisiness 0.4–0.85, tonal_pitch ~ A3, duration 100–300 ms | A3, 0.3/1 s | noisiness 0.62 |

Every list also carries the items not shown (usually the pitch, a headroom item `peak < -1dB` and the stages the family implies). The goal file's `rationale` says why each item and window is there, which design trap it avoids and what the witness reads.

- **The module set.** Across the witnesses: every oscillator wave (sine, triangle, saw, pulse, the `organ` wavetable), noise (white and pink), the filter in low-pass, high-pass and resonant use, a second envelope on the cutoff and on the pitch (`semi`), LFOs on the cutoff, on a level (sine and square, unipolar) and on `fine`, the reverb, and mono voice mode. The compressor has no goal ([findings](#findings-for-the-descriptor-and-cli-owners)).
- **Short sounds are goals.** The seven percussion goals (the 808 included) and the zap last from 44 ms to about a second. Nothing is filtered by duration: percussion holds the key at least 0.25 s, so noise, decay and sustain are measurable, in a clip of at most 1.5 s (a harness test).

- **The S2a kinds.** The eleven new witnesses use `wt` (FM through an audio route into `wt.pm`, the envelope or LFO on `pm_index`, `unison` and `detune`, `feedback`), `lfo` and `mseg` synced to the tempo, `dist` (soft clipping and the sine folder), `ott`, `formant`, `comb`, `ladder`, `phaser` and `eq`, beside the old `svf`, `noise`, `adsr`. Nine of them (`wt dist ott formant comb ladder phaser eq mseg`) are reserved S2a kinds (`ModuleSpec::hidden`) until INT-S2a clears the flag; the CLI takes them by key.

## The S2a families (eval version 4)

Five families, written for the kinds INT-S2a has not yet cleared. Each goal is a sound with a designer list, as before, **and** (new) a `requires` list, because for these kinds the sound alone does not say how the patch was made.

| Family | Goals | Kinds | What the list reads | What `requires` binds |
| --- | --- | --- | --- | --- |
| FM growl | `bass-fm-growl`, `bass-fm-growl-wobble` | `wt` x2 | a clean, held bass whose centroid sweeps once at the attack (index envelope), or at 6 Hz (LFO on `pm_index`, with a noisier spectrum) | two `wt`, an audio route into `wt.pm`, and for the wobble `lfo -> wt.pm_index` |
| Unison reese | `bass-reese-unison`, `bass-reese-moving` | `wt` (`unison`, `detune`), `svf`, `lfo` | a dark, dense stack with no single beat rate; the moving one adds a 1.2 Hz sweep | `wt.unison >= 3`, the low-pass, and `lfo -> svf.cutoff` |
| Synced wobble | `bass-wobble-eighth`, `bass-wobble-dotted-mseg` | `lfo`, `mseg` (`sync`, `beats`) | the filter sweep at the tempo's note value: 4.667 Hz (eighth) or 3.111 Hz (dotted eighth) at the CLI's 140 BPM | `lfo.sync = on` or `mseg.sync = on`, routed to `svf.cutoff` |
| Distortion into OTT | `bass-fold-sine`, `bass-dist-ott` | `dist`, `ott` | a folded sine's centroid; a loud, bright, level bass with no bump at the attack | `dist.mode = sine_fold`; `dist` and `ott` |
| Filter and effect chains | `bass-formant-talk`, `pluck-comb-string`, `bass-chain-ladder-phaser-eq` | `formant`, `comb`, `noise`, `ladder`, `phaser`, `eq` | a vowel sweep at 2 Hz; a pitched, decaying pluck from noise; a slow centroid swing from the phaser | the kinds, and `lfo -> formant.*` |

### `requires`: the judge checks how the patch is built

A descriptor reads a sound, not a technique, and for S2a that matters. Measured while designing (the table of controls below): a low-pass filter swept five octaves by a fast envelope passes `bass-fm-growl`'s list; four detuned saws on the old oscillator pass `bass-reese-unison`'s; a free LFO typed at 4.667 Hz passes `bass-wobble-eighth`'s; distortion without OTT passes `bass-dist-ott`'s (OTT is nearly invisible to every descriptor on a held bass). An eval of S2a that those patches pass would measure nothing about S2a. So eval version 4 adds one optional key to a goal, and the judge reads it next to `enth check`:

```toml
schema = "enth.agent-eval/2"
requires = ["wt x2", "wt -> wt.pm", "lfo -> wt.pm_index"]
```

- **Schema.** `enth.agent-eval/2` is additive over `/1`: the same keys plus `requires`. A `/1` goal loads exactly as before and cannot carry `requires` (refused); an entry that does not parse is refused at load. ([Reference targets](#reference-targets-not-built) reserved `/2` for an optional `[target]` table; that table joins `requires` in `/2`, both additive, so there is no `/3`.)
- **Grammar** (one requirement per entry; kinds, parameters and sources are lowercase keys): `wt` (at least one enabled module of that kind), `wt x2` (at least two), `wt -> wt.pm` (an enabled route from a module of kind `wt` to the `pm` parameter of a module of kind `wt`; the source may be `any`, `velocity`, ..., and the parameter `*`), `lfo.sync = on` (some enabled module of that kind has that value; `true` and `on` are the same, as are `false` and `off`), `wt.unison >= 3` (some enabled module of that kind has a number at least that large). A bypassed module or a route from one counts for nothing.
- **How.** After `enth check` passes or fails on the sound, the harness runs `enth get <final patch>` (and `enth get <patch> <address>...` for the parameter requirements) and evaluates the list. A goal passes only if both pass; an unmet requirement shows in the trial's `failed` as `requires <text>` and sets `pass` to false. The audit runs it on the witness; `--cross` and the start-patch rule stay sound-only.
- **What the agent is told.** A goal with `requires` gets one sentence after its brief: *The check looks at how the patch is built as well as how it sounds: build it the way the goal says.* The requirement texts are never shown (a harness test: no entry with a space appears in either prompt). The three prompt files are unchanged, and the 30 older goals render exactly as before, so version 3's numbers for them stay comparable.
- **Both tracks.** On the self track the agent still writes its own list; the designer's `requires` applies to its patch as the designer's list does.
- **Old results.** Nothing in a trial result or summary is new (an unmet requirement is a `failed` entry), so every older run directory and summary reads and rebuilds as before (a unit test reads a version 3 `result.json`; the older `eval.toml` shape still loads).

**Negative controls** (the old kinds imitating each goal; measured with the debug `enth`, 2026-10-27). Every one passes the sound list, so the sound list alone cannot tell the technique, and every one fails `requires`:

| Goal | Imitation | Sound list | `requires` |
| --- | --- | --- | --- |
| `bass-fm-growl` | low-pass (resonance 45 %), +5 oct from a fast envelope | passes, 0 of 9 items fail | unmet: `wt x2`, `wt -> wt.pm` |
| `bass-fm-growl-wobble` | low-pass swept by a 6 Hz LFO | passes, 0 of 8 | unmet: all three |
| `bass-reese-unison` | four saws at -23, -8, +9, +26 ct, low-pass 700 Hz (three saws beat at 0.5 to 1 Hz and fail `beating == null`) | passes, 0 of 7 | unmet: `wt.unison >= 3` |
| `bass-reese-moving` | the four saws with a 1.2 Hz LFO on the cutoff | passes, 0 of 8 | unmet: `wt.unison >= 3` |
| `bass-wobble-eighth` | a free LFO typed at 4.667 Hz | passes, 0 of 7 | unmet: `lfo.sync = on` |
| `bass-wobble-dotted-mseg` | a free triangle LFO at 3.111 Hz | passes, 0 of 7 | unmet: `mseg.sync = on`, the MSEG route |
| `bass-fold-sine` | the template's saw into a low-pass at 1.6 kHz | passes, 0 of 7 | unmet: `dist.mode = sine_fold` |
| `bass-dist-ott` | the witness without OTT (distortion alone) | passes, 0 of 8 | unmet: `ott` |
| `bass-formant-talk` | a band-pass swept by a 2 Hz LFO | passes, 0 of 7 | unmet: `formant`, the route |
| `pluck-comb-string` | the template's saw, low-pass 1.5 kHz, a pluck envelope | passes, 0 of 7 | unmet: `noise`, `comb` |
| `bass-chain-ladder-phaser-eq` | low-pass swept by a 0.9 Hz LFO | passes, 0 of 7 | unmet: `ladder`, `phaser`, `eq` |

Without any imitation the bare template (one saw, the default envelope) fails 1 to 4 items of each sound list. End to end, through the wrapper: the `bass-wobble-eighth` witness with `sync`/`beats` replaced by `rate = 4.667Hz` fails its dry-run trial with every sound item passing and `failed = ["requires lfo.sync = on"]` (checked once by hand, then the witness was restored).

### Discovery: can an agent find these kinds with the CLI's own help?

`cargo xtask agent-eval audit` prints a table of the required kinds as the CLI describes them, and `--require-discoverable` turns the first finding into a failure. Run on 2026-10-27, against the debug `enth` of this branch:

| # | Finding | Effect | For INT-S2a |
| --- | --- | --- | --- |
| 1 | **`enth modules` lists seven kinds** (`osc noise svf adsr lfo reverb comp`). Nine of the twelve required kinds are hidden from it: `comb dist eq formant ladder mseg ott phaser wt` | An agent that does not already know a key cannot find it by listing. `enth params <kind>` and `enth explain <kind>` do take every hidden key and describe it well (summary, ranges, `effect`, `moves`), so a model that knows the name, or reads it from the brief, can use it | Clearing `hidden` fixes it; `audit --require-discoverable` then passes and should become the audit's default |
| 2 | **`unknown_kind` lists the same seven** (`known` in the error) | A wrong guess gets no pointer to the S2a kinds | Same flag |
| 3 | **`enth guide --topic ops` is out of date for S2a**: its kinds list stops at `reverb`/`comp`; its route sources are `adsr`, `lfo`, `velocity`, `key`, `modwheel`, `macro1..8` (no generator as a source, so no audio route); it says destinations must be `modulatable: true`, but an audio input such as `wt.pm` is `modulatable: false, audio: true`; `mseg` and `sync` are not mentioned | An agent that follows the guide to the letter never routes `wt2 -> wt1.pm`. `enth params wt` (the `pm` and `am` parameters) is what shows the route | Update the guide's ops page when the flag clears |
| 4 | **No alias for the goal words**: `wavetable`, `distortion`, `moog`, `compressor`, `mseg` and `envelope` resolve; `fm`, `unison`, `reese`, `growl` are `unknown_kind` | A model that looks for "fm" finds nothing | Add aliases (`fm`, `unison` to `wt`) with the catalogue |
| 5 | **`check`, `describe` and `step` have no tempo flag** (only `render --bpm`); a synced `lfo` or `mseg` is judged at the 140 BPM default | The two synced goals are written for 140 BPM and say so in the brief; a different tempo cannot be tried | A `--bpm` on the judge commands, or a `bpm` in the performance |
| 6 | **`enth params lfo` summarises the kind without the sync** (the `sync` and `beats` parameters say it) | A model learns the tempo option only by reading all parameters | One clause in the kind summary |

None of these blocks a goal: each witness reaches its sound through `enth apply` and `enth step` with the keys above. They decide how much of the prompt has to say (the briefs name the effect, never the key) and what a first paid run will reveal: a model that fails these goals may be failing at discovery, not at sound design.

## What makes a goal honest

Each rule has a check that anyone can run (no model, no spend):

| Rule | Check | Result, eval version 4 (measured 2026-10-27) |
| --- | --- | --- |
| The witness passes the designer list and its own list, with no `thin` item | `cargo xtask agent-eval audit` | 41 of 41; the eleven S2a witnesses also meet their `requires` |
| The untouched starting patch fails the list | the same | 41 of 41 fail (1 to 6 items each; the S2a goals 1 to 4) |
| Margins are not thin | the same (the lowest `margin_pct` of the Hz and ms items) | lowest 10 % (`filter-wobble` duration, `lead-supersaw` duration); the S2a goals 14 % or more (`bass-fm-growl-wobble` and `bass-wobble-dotted-mseg` sweep rates, which read the LFO or the tempo exactly) |
| The list tells sounds apart | `cargo xtask agent-eval audit --cross`: each list against the other 40 witness patches, on its own note | 30 lists pass no other witness. `perc-woodblock` and `pluck-marimba` pass `kick-thump`'s (as in version 3: a low-passed, decaying saw, played at E5 and C4, is a fair woodblock and marimba). Nine more lists pass a new witness, none of them a flaw: `bass-buzzy` and `bass-fold-sine` pass each other's (a folded sine is a buzzy bass), and `bass-fold-sine` passes `bass-dist-ott`'s; `bass-reese-unison` passes `bass-wobble-eighth`'s, `bass-reese-moving`'s and `bass-wobble-dotted-mseg`'s (a dark, rough, low-passed saw on E1); `filter-wobble` passes `bass-wobble-dotted-mseg`'s (a 3.1 Hz wobble is a filter wobble); `pad-trance-gate` passes `bass-fm-growl-wobble`'s and `bass-acid-squelch` passes `bass-fm-growl`'s (a 6 Hz or a decaying centroid sweep); `pluck-comb-string` passes `pluck-bright-short`'s; `bass-chain-ladder-phaser-eq` passes `bass-reese-unison`'s; `lead-flute-breathy` passes `bass-wobble-eighth`'s. Reported, not gated; for the S2a goals `requires` is what separates them |
| The harness agrees end to end | `cargo xtask agent-eval --dry-run` (the witness replayed through the wrapper, the budget counter and the judge, 3 trials, both tracks) | 41 of 41 on the given track and 41 of 41 on the self track, every one of the 246 trials, 1 render each (the structure check included) |
| The structure check binds the technique | `requires` on the old-kinds imitations of the eleven S2a goals ([controls](#requires-the-judge-checks-how-the-patch-is-built)) | 11 of 11 imitations pass the sound list and fail `requires` |
| Old versions still load | `cargo test -p xtask agent_eval` (61 tests) | the 30 version 3 goals load on `/1` with no `requires`; a version 3 `eval.toml` and a version 3 `result.json` still parse |
| Lists use descriptors only within their documented limits | design, per goal: the rationale names the limit it avoids ([docs/dsp/analysis.md](dsp/analysis.md), Limits) | — |

Negative controls (measured while designing; each fails the list it imitates): a falling noise sweep under a swell against `fx-noise-riser` (brightness 1.1 kHz, sweep 776 ms), a noise swell with no filter against the same (no sweep), a pad with a 1.2 s attack and a long release against `fx-reverse-swell` (attack, release), a filter envelope on a steady tone against `fx-laser-zap` (stability 0.89), a detuned pair against `lead-vibrato` (beating found, stability 0), a long envelope release instead of the reverb against `pluck-hall` (noisiness 0.0002).

**The mechanical baseline.** `cargo xtask agent-eval --dry-run --agent follower` starts from the witness's template and applies `check`'s `fix.ops` until nothing more is offered. It passes 3 of 41 on each track: `pluck-bright-short` and `pad-slow-dark` (from their templates) and `lead-chip-pulse`, the one new goal that parameters alone reach from `init` (a saw passes its brightness and stillness items; the brief names the pulse, which no descriptor tells from a saw). The other 38 need structure the hint solver cannot add (a wave change, a second oscillator, noise, a filter, an LFO or envelope and its route, the reverb, and for the eleven S2a goals a kind it does not know). That is the floor a model has to beat.

## Results and the paid-run record (eval version 4)

**No paid run has been made on eval version 4.** What exists is free: the two scripted agents, measured on 2026-10-27 with the debug `enth` of lane EV5's branch (the cost the dry runs print, $0.93 and $1.86, is the harness's token arithmetic over scripted calls; no money was spent and no model was called).

| Agent | Model | Date | Cost | Goals passed, given track | Goals passed, self track | Renders |
| --- | --- | --- | --- | --- | --- | --- |
| `witness` (ceiling; reachability proof), 3 trials | none | 2026-10-27 | $0 | **41 of 41** | **41 of 41** | 1 per trial |
| `follower` (**the floor**), 3 trials | `follower` | 2026-10-27 | $0 | **3 of 41** (the same three as in version 3) | **3 of 41** | 1 to 5 |

The follower passes none of the eleven S2a goals: each needs a kind that no parameter move adds. A model has to beat it on every family.

The rows to fill when the owner approves a paid run (`--confirm-spend`; a run saves `results/<date>-<model>.json` itself, with the model id, Claude Code version, `enth` git hash and eval version in the header):

| Run | Model | Date | Cost (measured) | Given track: goals passed, median renders | Self track: goals passed, own list passing | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| S2a goals, 3 trials, both tracks | _to fill_ | _to fill_ | _to fill_ | _to fill_ | _to fill_ | per family: FM growl, unison reese, synced wobble, distortion into OTT, chains |
| Whole set, 3 trials, both tracks | _to fill_ | _to fill_ | _to fill_ | _to fill_ | _to fill_ | version 4 results are not comparable with versions 1 to 3 |

```sh
# the eleven S2a goals, about 66 trials, about $9 at the plan's $0.14 a trial; ONLY with the owner's approval
cargo xtask agent-eval --confirm-spend --model claude-sonnet-5-5 --trials 3 --max-usd 12 \
  --goals bass-fm-growl,bass-fm-growl-wobble,bass-reese-unison,bass-reese-moving,bass-wobble-eighth,bass-wobble-dotted-mseg,bass-fold-sine,bass-dist-ott,bass-formant-talk,pluck-comb-string,bass-chain-ladder-phaser-eq
```

What the first paid run should read, beyond the pass rates: how many calls a trial spends on kinds that are not in `enth modules` (`enth params wt` and friends, or `unknown_kind` errors), and whether the one sentence about the build check is enough to make a model use the S2a kinds ([discovery](#discovery-can-an-agent-find-these-kinds-with-the-clis-own-help)). A goal that fails on `requires` only (the sound passes) is a model that imitated the sound with the old kinds.

## Adding or changing a goal

1. Prototype with the CLI: `enth new --template init`, `enth step ... --perf <perf> --expect <list>` until the witness passes with no `thin` item; read `enth describe` for the readings.
2. Write `goals/<id>.toml` and `witness/<id>.toml` (copy an existing pair). The rationale says what each item measures, why the windows, the trap it avoids and what the witness reads. If an imitation with the old kinds would pass the list (build it and run `enth check` on it), the goal names the technique in `requires` and uses `schema = "enth.agent-eval/2"` ([above](#requires-the-judge-checks-how-the-patch-is-built)); the rationale says what `requires` binds. A witness step's `--ops` JSON must not contain a single quote (the file quotes it).
3. `cargo xtask agent-eval audit --goals <id> --cross` and `cargo xtask agent-eval --dry-run --goals <id>`; then the whole set.
4. Bump `eval_version` in `eval.toml` with a changelog line. A goal's brief, performance, list or `requires`, a prompt, or the wrapper's render accounting changes it; a witness alone does not, but say so in the next changelog line.
5. Run the paid eval only with the owner's approval (`--confirm-spend`; [p1a-g1.md](p1a-g1.md#the-real-run)). At the plan's estimate of about $0.14 per trial, the full set is 41 goals × 3 trials × 2 tracks = 246 trials, about $34: above the default `--max-usd 20`, which stops starting trials at that estimate, so pass `--max-usd 40` or run one track at a time. The eleven S2a goals alone are 66 trials, about $9 (`--goals bass-fm-growl,...`).

## Findings for the descriptor and CLI owners

Properties of the current analysis found while making every goal reachable (eval version 3). None blocks a goal; they decide what a list can ask.

| # | Finding | Effect on the set | Suggested follow-up |
| --- | --- | --- | --- |
| 1 | A filter LFO on **pink noise through a low-pass** is not found (±1 and ±2 octaves, resonance 0–30 %); white noise through a low-pass and either colour through a band-pass are | `texture-wind`'s brief names white noise | Look at the sweep detector's band median on pink-noise centroid tracks; document in analysis.md Limits |
| 2 | The sweep **direction** (`modulation.filter_sweep.dir`) is in `describe` but not in the expectation vocabulary | `fx-noise-riser` carries the direction with a brightness floor; `bass-acid-squelch` and `fx-laser-zap` cannot say up or down | A `sweep_dir` feature (`== up`) |
| 3 | `pitch.stability` reads **0 on a detuned pair** (the YIN track wobbles with the beat) | `lead-vibrato` asks for `beating == null`; `lead-chip-pulse` uses `stability > 0.95` as "no detune, no vibrato" | Document; a vibrato descriptor (rate, depth in cents) would replace stability for vibrato |
| 4 | **Tremolo with detune**: a sine tremolo on a detuned pair reads as beating and no tremolo; a square gate reads the tremolo and hides the beat | `pad-trance-gate` uses one oscillator | Matches the "detune with tremolo untested" limit; add a test case |
| 5 | On a **resonant low-pass closing over a held note** the attack reads 64–161 ms with a 1 ms amp attack (the level peaks when the resonance crosses the low harmonics) | `bass-acid-squelch` has no attack item | Document |
| 6 | A **pitch dive** reads noisiness 0.33–1 (a gliding pitch has no steady spectral peaks) and stability 0 | `fx-laser-zap` has no noisiness item | Document; a pitch-sweep descriptor |
| 7 | A **reverb tail on a decaying sound** reads noisiness 0.05–0.16 over the ten settings tried (an envelope tail 0.0002); on a **held pad** the same reverb reads 0.00004, like the dry pad | `pluck-hall` uses `noisiness > 0.02`; a reverb-pad goal was dropped (only the release would show the reverb, which an envelope makes too) | A space or tail descriptor (decay of the diffuse part, stereo width) |
| 8 | A filter LFO at **0.5 Hz with a 5 s hold** after an 800 ms attack is not found (about 2 cycles); 0.8 Hz is | `pad-moving-bright` asks for 0.4–1.5 Hz | Matches the "two cycles" limit |
| 9 | A **pulse and a saw** read alike (4.2 and 4.9 kHz at C5); `spectrum.harmonics_db` and `odd_even_ratio` are listed in agent-cli.md but not built | `lead-chip-pulse` cannot ask for the pulse | Build the harmonic recipe descriptors |
| 10 | The **compressor** has no single-note goal: its audible result (a tamer peak, a louder tail) is an envelope shape an ADSR makes | No compressor goal | A dynamics or crest-factor descriptor and a phrase performance |
| 11 | No descriptor for **resonance**, **stereo width**, **vibrato rate** or a **pitch envelope's direction** | Briefs that depend on them measure what they can (acid: the sweep; zap: the sweep and stability) | As above |

## The lesson track (R2, lane EV3)

- Date: 2026-10-21. Track version 1, 16 goals, one per lesson of the basics pack (`content/lessons/basics`). Engine 0.3.0.
- What it asks: can an agent complete a lesson **from the lesson text alone**, through `enth lesson show|start|check|hint|closest` and the patch CLI, the way a player completes it in the Learn view? A lesson is a list of steps. The agent cannot hear, so what it does, per step kind: **read** and **listen**, nothing to hand in; **do**, make a patch that passes the step's check; **recreate** (a round), make a patch that passes the round's pass mark against a hidden target; **choose** (an ear question), measure each option (`lesson start --examples-out` writes them as patch and performance files, `enth describe` reads them) and answer with a label. The commands are in [agent-cli.md](agent-cli.md#lessons-on-the-command-line-enth-lesson).
- Run it: `cargo xtask agent-eval lesson [--dry-run] [--agent witness|follower] [--goals id,...] [--trials N]`; `cargo xtask agent-eval lesson goals` lists the goals and checks each against its pack. It is the text goals' harness with a different task: the same wrapper, the same headless Claude Code run, the same `--dry-run` fakes and the same transcripts and summaries. The text goals, their eval version (now 4) and their wrapper accounting are untouched.
- Files: `content/agent-evals/lesson-track.toml` (the version and why it changed), `lesson-goals/<id>.toml` (the lesson, its steps pinned as `id:kind`, a budget), `prompts/lesson-system.md` and `prompts/lesson.md`, `results/<date>-lesson-<model>.json` (a saved summary). Code: `xtask/src/agent_eval/lesson_track.rs` (goals, trials, judge, summary), `lesson_fake.rs` (the fake agents), `wrap.rs` (the wrapper's lesson mode).

### What differs from the text goals

| | Text goals | Lesson track |
| --- | --- | --- |
| Goal | A brief, a performance and a designer's expectation list; the agent builds a sound | One lesson of a pack; the agent completes its steps |
| The agent's directory | Empty, with the `enth` wrapper | The wrapper and `pack/`, a copy of the pack (the judge uses the pristine one) |
| Budget | 20 renders | 120 **cost units** (below) |
| Final answer | `{patch, expect, explanation}` | `{steps: [{step, patch} or {step, choice}], explanation}`; a `do` or `recreate` step with its patch file, a `choose` step with its label; a step left out counts as not done |
| Judge | `enth check` of the final patch with the goal's performance | Each graded step on its own (below); a lesson passes when every graded step does |
| Tracks | Given list, self-written list | One (the lesson's text is the whole brief) |
| Assistance | n/a | Recorded per step, not forbidden |

**Cost units** (the wrapper's `--lesson` mode; any change to them is a track version change). 1 unit: `lesson check`, `compare`, `describe` or `check` of a patch, `render`, `step`, `bench`, and a measured `lesson hint` (one given a patch file). 2: `suggest`. 10: `lesson closest` and `match` (a search of tens of renders). Free: `lesson show`, `list` and `start`, an authored hint (no patch), `apply`, `get`, `diff`, `params`, `guide`, `check` of a WAV. A call the tool rejects before rendering (exit 2, 3 or 5) gives its units back. The wrapper refuses, with exit 9 and an `enth.error/1` that says why and costs nothing: a call that does not fit in the budget (`budget_exhausted`), `--reveal` (`reveal_refused`: a lesson's answers, a round's target), a path outside the working directory (`path_refused`: absolute, or with `..`), and the pack's files named to any command but `enth lesson` (`pack_refused`: the witness patches sit there). The witness spends 12 to 35 units a lesson (a check per do step, a search and a compare per round); 120 is about three times the most.

**The judge** (`enth` itself, on the pristine pack, never the agent's files):
- a **do** step: `enth lesson check --from <the step's start>` (exit 0); `--from` because two basics steps have a `touched` predicate, which reads the editor's edit log (finding 1 below);
- a **recreate** step: `enth compare <patch> --challenge <round>` (exit 0, the round's sound score at the pass mark), where the round is written by the judge's own `lesson start`, never the agent's file;
- a **choose** step: the label against the pack's recorded answer (`lesson show --reveal`, which only the judge runs);
- every patch, on top: the step's `locked` and `editable` controls. `lesson check` and `compare` do not enforce them (the editor does), so the judge diffs the patch against the step's start (`enth diff`) and a change to a locked control, or to one an `editable` list leaves out, fails the step as `locked`. A step with no start of its own (it carries on from the step before) has nothing to diff with and is not lock-checked. The matching is textual (`osc1.*`, `filter1.cutoff`, a route as `lfo1 -> filter1.cutoff`), a close reading of `enth_challenge::v2::touches`, not the same code;
- **assisted** steps are listed, not failed: the call log shows `lesson hint` or `lesson closest` on the step, or `suggest`/`match` on its round's file. The summary reports steps passed and steps passed unassisted.

### The fake agents (`--dry-run --agent ...`)

No model is called; each makes the calls an agent would through the same wrapper, so the unit counter, the refusals, the judge and the report are exercised.

- **`witness`**, the ceiling and the proof that every graded step is reachable with this engine: a do step gets the step's witness patch; a round gets the nearest passing patch `enth lesson closest` finds; an ear question gets the pack's recorded answer.
- **`follower`**, the floor: a scripted agent that **follows the hints and nothing else**. It reads no text and has no ear. A do step: apply the measured hint's move (`lesson hint` with a patch, then `apply --ops`) until the check passes or the hint has no move (at most 6 times). A round: apply the coach's last hint, the control and the range it names, to the geometric middle of the range, until the round passes or the hint names nothing to set (at most 8 times). An ear question: the first option. It costs no money and is deterministic, so one trial says it all (`--trials 1`).

### Results

Track version 1, engine 0.3.0, measured 2026-10-21 on the debug `enth` of the commit named in each file's header, one trial each (both fakes are deterministic). **No model has run on this track**: the paid run is [pending the owner's OK](#running-it). 107 graded steps in 16 lessons: 19 do steps, 45 rounds and 43 ear questions (the other 43 steps of the 150 are read or listen steps, which have nothing to hand in).

| Agent | Model | Date | Cost | Lessons passed | Steps passed | Do | Rounds | Ear questions | Units a lesson |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `witness` (ceiling; reachability proof) | none | 2026-10-21 | $0 | **16 of 16** | 107 of 107 | 19 of 19 | 45 of 45 | 43 of 43 | median 34, 12 to 35 |
| `follower` (**the floor**) | `follower` | 2026-10-21 | $0 | **0 of 16** | 32 of 107 | 7 of 19 | 11 of 45 | 14 of 43 | median 12, 5 to 50 |

The follower's summary is saved as `results/2026-10-21-lesson-follower.json` (model `follower`, cost 0; `--save-floor`). The witness's is not saved (nothing to compare it to; a rerun takes about 12 minutes). A model has to beat the follower on every column, and on the ear questions it has to beat **chance**: the follower's 14 of 43 is the first-option rate, a third of 43.

Where the follower is lost, so a model's result reads against it:

- **Do steps, 7 of 19.** It passes `attack-and-release/try`, `cutoff/try`, `decay-and-sustain/try` and `e-piano`, `how-a-lesson-works/try`, `two-oscillators/balance` and `wavetables/try`: steps whose sound check one numeric move over the editable controls satisfies. It fails where a measured hint cannot help: the check asks for a wave, a filter mode or a noise colour (an enum the solver does not move: `waveforms/try`, `filter-modes/try`, `noise/try`); the step has no sound item, only a patch predicate, so there is no measured rung (`pulse-width/try`); or a patch predicate asks for more than the sound check does (`resonance/try`: the hint stops at 66.55 %, the patch predicate wants more). The measured hint reads the step's sound items only (finding 7).
- **Rounds, 11 of 45.** It passes all three rounds of `attack-and-release` and `two-oscillators`, and single rounds of five other lessons. The coach's last hint names a control and a range; the middle of the range is often just short of the pass mark (six of the 34 failed rounds end within 5 points of it, e.g. `cutoff/round-3` at 89.6 of 90), and a round with several free controls needs several moves it does not make. `lesson closest` (the witness) passes all 45, so the rounds are reachable; the follower shows what the hints alone give.
- **Ear questions, 14 of 43,** chance.
- It never finishes a lesson, so the track's per-lesson pass is 0 of 16; the step counts are the sensitive measure at this level.

### Findings for the CLI and lesson owners

Found while making every lesson reachable. None is a bug in the track; each decides what the track can ask.

| # | Finding | Effect | Done or suggested |
| --- | --- | --- | --- |
| 1 | **A `touched` predicate cannot hold on a patch file.** It reads the editor's edit log; `lesson check` gave it an empty log. Two basics steps have one: `how-a-lesson-works/try` and `decay-and-sustain/e-piano` | Both steps were unreachable from the CLI: the witness patches failed their own check | `enth lesson check --from START` (added by EV3, additive): the control counts as turned as far as it moved from the start. |
| 2 | **`lesson check` does not enforce `locked` and `editable`**; the editor does | A patch that changes a locked oscillator passes `lesson check`; the judge has to diff for it | The judge diffs (above). A `--from` check could report the violation itself |
| 3 | **`lesson show` prints every round's whole embedded challenge**, start patch included: about 13 KB (roughly 4 000 tokens) a lesson, mostly the start patch repeated in each round | An agent reads it once per lesson; it is the largest single output of a trial | `show` could print a round's `constraints`, `target.kind` and pass mark, and leave the start to `lesson start` |
| 4 | **`enth guide` does not mention lessons** | The track's system prompt teaches them; a guide topic (`enth guide --topic lessons`) would let the guide alone do it | Add a topic page (the main page is 6 106 of its 6 144 bytes) |
| 5 | **An ear question is answerable by measurement only if `describe` measures what the question asks**: "which is darker" is `spectrum.centroid_hz` (checked in a CLI test); whether the others have a descriptor (an attack time for "which swells", and so on) is not checked | A model can only answer the choose steps whose question a descriptor reads; the follower answers them at chance | The first paid run shows which choose steps models get wrong; a descriptor per question is the fix |
| 6 | **The pass mark is the sound score alone**: a round with `combined` 75 and `sound` 90.5 passes (`structure` is weighed in `combined`, not in the verdict) | The judge reports the sound score | Documented in `enth compare`'s hint |
| 7 | **The measured hint reads a do step's sound items, not its patch predicates.** `resonance/try` passes its sound check at 66.55 % resonance and still fails a patch predicate; the hint, asked again, says the same move | A hint-follower stops short; so does a player who trusts `pass_all: true` (it means every *sound* item) | The hint request could carry the predicates, or `pass_all` could say "sound items". Not changed (the player builds the request) |

### Running it

```sh
# no model, no money (these are what the gate and the doc numbers use)
cargo xtask agent-eval lesson goals --enth target/release/enth
cargo xtask agent-eval lesson --dry-run --agent witness --trials 1 --enth target/release/enth
cargo xtask agent-eval lesson --dry-run --agent follower --trials 1 --save-floor --enth target/release/enth

# a model: only with the owner's approval of the spend. PENDING THE OWNER'S OK; not run.
cargo xtask agent-eval lesson --confirm-spend --model claude-sonnet-5-5 --trials 3 --max-usd 50
```

The paid run is **pending the owner's OK**. The cost is not measured: a lesson trial is on the order of 50 tool calls with a context that grows to 60 000 tokens or so, which on Sonnet 5.5 is a guess of $0.3 to $1 a trial, so 16 lessons × 3 trials = 48 trials is $15 to $50 (above the default `--max-usd 20`, which stops starting trials at the running estimate; hence `--max-usd 50`). Measure first: `--goals basics-cutoff --trials 1` (one trial, one lesson). A real run saves `results/<date>-lesson-<model>.json` itself (`--no-save-summary` to skip).

### Adding or changing a goal

1. A goal is a lesson of a pack: copy a file in `lesson-goals/`, set `lesson`, and pin `steps` as `id:kind` (`cargo xtask agent-eval lesson goals` prints the pack's steps when they differ).
2. `cargo xtask agent-eval lesson --dry-run --agent witness --goals <id> --trials 1`: the witness must pass every graded step. A step it cannot pass is a lesson or CLI problem to fix first (finding 1 was found this way).
3. Bump `eval_version` in `lesson-track.toml` with a changelog line when a prompt, a goal's steps or budget, the wrapper's unit costs or the judge's rules change. A lesson changing under a goal is refused (the pinned steps), so a result always names the lesson it was measured on.
4. Run the paid eval only with the owner's approval.

## Reference targets (not built)

R1 adds reference sounds to the text goals: **25 seeded targets** from the randomizer (lane RG, wave 2) and **5 recordings** that are CC0 or the owner's (report 02 §2.10). Nothing reads `content/agent-evals/targets/targets.toml` yet; it reserves the slots and names the plug points. How they plug in:

- **Goal schema `enth.agent-eval/2`**, additive over `/1`: an optional `[target]` table (eval version 4 already uses `/2` for `requires`; the table joins it there). Seeded: `kind = "seeded"`, `code = "<challenge code>"` (the `RandomSpec` code, which carries the generator and engine versions and a target-hash prefix; a mismatch is refused). Recorded: `kind = "recording"`, `wav = "targets/<id>.wav"`, `licence = "CC0-1.0"` or `"owner"`, `source` (URL, or "owner, date"). `expect` becomes optional when a target is set; `perf` stays (the performance the target plays, and the one the agent must pass).
- **The trial directory** holds only the wrapper and `target.wav`: the harness renders a seeded target from its code with the goal's performance at run time (never copying the code or the patch), or copies the recording. A third prompt, `prompts/target.md`, and a third track, `target`; adding it is an eval-version change.
- **The judge** is `enth compare <final patch> --target target.wav` with the challenge's calibration (`enth-challenge`'s `sound/1` scorer, lane SS): pass when the sound score reaches the challenge's threshold. A descriptor list may be added as a second judge. Records: sound %, renders, tokens, cost.
- **Witnesses and floors.** A seeded target's witness is its own patch (it scores 100 by construction). Beside the agent, two floors from report 02: **no-op** (the start patch's score) and **CMA-ES only** (`enth match` on every parameter, the same render budget). Recordings have no witness; the floors still apply.
- **Selection.** Seeded: 5 per difficulty 1–5, fixed seeds, through `enth random --seed N --difficulty d` with the randomizer's own rejection of silent and clipping targets; a target-hash golden guards generator drift (the generator version is recorded in every result). Recordings: about 1–3 s, mono, 48 kHz, small enough for git; a plucked string, a bass note, a sustained tone, a drum hit and a texture, chosen by the owner.
- **Needs before it can run:** `enth random` with codes (RG), `enth compare` on the game's scorer (SS, CL), and in the harness `target` handling in `goal.rs`, installing `target.wav` beside the wrapper, the compare judge and the report columns.

**Edit goals** (report 02: start from a flawed patch, clicky, harsh or clipping) need a start patch that is not a template: an optional `start = "starts/<id>.patch.json"` that the harness copies into the trial directory, and a prompt variant. Not built; no goal uses it yet.
