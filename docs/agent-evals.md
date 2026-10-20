# Agent evals: the goal set

- Date: 2026-10-20 (R1/S1 lane EV2). Eval version 3, 30 text goals. Engine 0.2.0.
- What it is: the set of text goals an agent designs sounds for with the `enth` CLI, and the checks that keep each goal reachable and honest. The protocol (tracks, trials, the wrapper, the judge, the blind listen) is in [agent-cli.md](agent-cli.md#evaluating-the-cli-like-a-product); the P1a history and the first paid run are in [p1a-g1.md](p1a-g1.md#6-the-agent-evaluation).
- Files: `content/agent-evals/goals/<id>.toml` (brief, performance, designer list, rationale), `witness/<id>.toml` (a hand-picked solution, never shown to the agent), `prompts/`, `eval.toml` (the version and why it changed), `targets/targets.toml` (a placeholder for the reference sounds, [below](#reference-targets-not-built)).

## The set (eval version 3)

Thirty goals in seven families, difficulty 1 to 5 (2, 8, 8, 7 and 5 goals). Every goal starts from the blank `init` template (one saw, the default envelope). "Reads" is what the witness's one render measures under the judge.

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
| 3 | `fx-reverse-swell` | FX | a reverse swell: a tone that fades in over the whole note and cuts off dead at the end | attack > 1 s, sustain > -1.5 dB, release < 60 ms | C4, 2/3 s | attack 1544 ms, release 12 ms |
| 3 | `hat-closed-noise` | percussion | a short closed hi-hat made of noise | noisiness > 0.8, brightness > 9 kHz, duration 25–120 ms | C3, 0.3/1 s | centroid 12 kHz, duration 44 ms |
| 3 | `keys-tremolo` | keys, modulation | an electric-piano-like key with a gentle tremolo | tremolo 3.5–7 Hz at 2.5–12 dB, sustain -20 to -4 dB | C4, 2.5/3.5 s | tremolo 5.0 Hz, 5.3 dB |
| 3 | `lead-vibrato` | lead, modulation | a singing saw lead with a clear vibrato | stability 0.4–0.85, `beating == null` | A4, 2/3 s | stability 0.586 |
| 3 | `pad-trance-gate` | pad, modulation | a trance-gate chop: a sustained saw chopped into fast rhythmic pulses | tremolo 5–10 Hz, depth > 12 dB | A3, 3/4 s | tremolo 8.0 Hz, 31.8 dB |
| 3 | `tom-low` | percussion | a low floor tom: a deep tonal thud with a little noise | tonal_pitch ~ G2, noisiness 0.08–0.6, duration 200–700 ms | G2, 0.6/1.5 s | noisiness 0.18, tonal 98.2 Hz |
| 4 | `bass-acid-squelch` | bass, modulation | an acid bass: a resonant filter that snaps shut on every note | filter_sweep > 1.2 oct, sweep_ms 80–500 ms, brightness 250 Hz–1.2 kHz | A2, 1.5/2.5 s | sweep 1.91 oct, 192 ms |
| 4 | `bell-soft` | keys | a soft bell-like tone | attack 8–40 ms, decay > 600 ms, sustain < -35 dB, beating 2–8 Hz | C5, 3/5 s | beating 4.24 Hz, sustain -39.8 dB |
| 4 | `brass-ish` | lead | a brassy sustained note with a slow swell | attack 40–120 ms, brightness 1.5–3 kHz, beating 0.7–2 Hz | E3, 2/3 s | attack 51 ms, beating 1.53 Hz |
| 4 | `filter-wobble` | bass, modulation | a low saw with a slow wobbling filter sweep | filter_lfo 1.5–3.5 Hz, filter_sweep > 1 oct | A2, 4/5 s | 2.00 Hz, 2.97 oct |
| 4 | `fx-noise-riser` | FX | a noise riser: a whoosh that builds up and brightens over the whole note | attack > 1.2 s, filter_sweep > 2 oct, sweep_ms > 1 s, brightness > 3 kHz | C3, 3/4 s | sweep 3.86 oct, 2060 ms, centroid 8.1 kHz |
| 4 | `pluck-hall` | pluck, effects | a plucked note ringing out in a big reverberant hall | release > 1 s, duration > 1.8 s, sustain -45 to -10 dB, noisiness > 0.02 | E4, 1/4 s | duration 3317 ms, noisiness 0.14 |
| 4 | `texture-wind` | texture, modulation | wind: white noise swept slowly up and down by a filter | filter_lfo 0.3–1.5 Hz, filter_sweep > 0.5 oct, noisiness > 0.8 | C3, 5/7 s | 0.80 Hz, 1.31 oct |
| 5 | `fx-laser-zap` | FX, modulation | a laser zap: a short tone whose pitch dives fast | filter_sweep > 0.8 oct, sweep_ms < 300 ms, stability < 0.5 | C4, 0.5/1.2 s | sweep 1.13 oct, 181 ms, stability 0 |
| 5 | `lead-flute-breathy` | lead | a breathy flute-like tone | tonal_pitch ~ A4, noisiness 0.04–0.3, attack 40–250 ms | A4, 2/3 s | noisiness 0.10, attack 137 ms |
| 5 | `lead-supersaw` | lead | a detuned supersaw-ish lead | brightness 3.5–6 kHz, beating 4–8 Hz, duration 2–2.6 s | A4, 2/3 s | beating 5.60 Hz, duration 2196 ms |
| 5 | `pad-moving-bright` | pad, modulation | a lush detuned pad with a slow, breathing filter movement | filter_lfo 0.4–1.5 Hz, beating 0.8–4 Hz, attack > 300 ms | C4, 5/7 s | 0.81 Hz, beating 1.81 Hz |
| 5 | `snare-crack` | percussion | a snare-like crack: a noise burst over a short tonal body | noisiness 0.4–0.85, tonal_pitch ~ A3, duration 100–300 ms | A3, 0.3/1 s | noisiness 0.62 |

Every list also carries the items not shown (usually the pitch, a headroom item `peak < -1dB` and the stages the family implies). The goal file's `rationale` says why each item and window is there, which design trap it avoids and what the witness reads.

- **The module set.** Across the witnesses: every oscillator wave (sine, triangle, saw, pulse, the `organ` wavetable), noise (white and pink), the filter in low-pass, high-pass and resonant use, a second envelope on the cutoff and on the pitch (`semi`), LFOs on the cutoff, on a level (sine and square, unipolar) and on `fine`, the reverb, and mono voice mode. The compressor has no goal ([findings](#findings-for-the-descriptor-and-cli-owners)).
- **Short sounds are goals.** The seven percussion goals (the 808 included) and the zap last from 44 ms to about a second. Nothing is filtered by duration: percussion holds the key at least 0.25 s, so noise, decay and sustain are measurable, in a clip of at most 1.5 s (a harness test).

## What makes a goal honest

Each rule has a check that anyone can run (no model, no spend):

| Rule | Check | Result, eval version 3 |
| --- | --- | --- |
| The witness passes the designer list and its own list, with no `thin` item | `cargo xtask agent-eval audit` | 30 of 30 |
| The untouched starting patch fails the list | the same | 30 of 30 fail (1 to 6 items each; the new goals 2 to 6) |
| Margins are not thin | the same (the lowest `margin_pct` of the Hz and ms items) | lowest 10 % (`filter-wobble` duration, `lead-supersaw` duration); the new goals 20 % or more |
| The list tells sounds apart | `cargo xtask agent-eval audit --cross`: each list against the other 29 witness patches, on its own note | 28 lists pass no other witness; `perc-woodblock` and `pluck-marimba` pass `kick-thump`'s (a low-passed, decaying saw, played at E5 and C4, is a fair woodblock and marimba) |
| The harness agrees end to end | `cargo xtask agent-eval --dry-run` (the witness replayed through the wrapper, the budget counter and the judge, 3 trials, both tracks) | 30 of 30 on the given track and 30 of 30 on the self track, every trial, 1 render each |
| Lists use descriptors only within their documented limits | design, per goal: the rationale names the limit it avoids ([docs/dsp/analysis.md](dsp/analysis.md), Limits) | — |

Negative controls (measured while designing; each fails the list it imitates): a falling noise sweep under a swell against `fx-noise-riser` (brightness 1.1 kHz, sweep 776 ms), a noise swell with no filter against the same (no sweep), a pad with a 1.2 s attack and a long release against `fx-reverse-swell` (attack, release), a filter envelope on a steady tone against `fx-laser-zap` (stability 0.89), a detuned pair against `lead-vibrato` (beating found, stability 0), a long envelope release instead of the reverb against `pluck-hall` (noisiness 0.0002).

**The mechanical baseline.** `cargo xtask agent-eval --dry-run --agent follower` starts from the witness's template and applies `check`'s `fix.ops` until nothing more is offered. It passes 3 of 30 on each track: `pluck-bright-short` and `pad-slow-dark` (from their templates) and `lead-chip-pulse`, the one new goal that parameters alone reach from `init` (a saw passes its brightness and stillness items; the brief names the pulse, which no descriptor tells from a saw). The other 27 need structure the hint solver cannot add (a wave change, a second oscillator, noise, a filter, an LFO or envelope and its route, the reverb). That is the floor a model has to beat.

## Adding or changing a goal

1. Prototype with the CLI: `enth new --template init`, `enth step ... --perf <perf> --expect <list>` until the witness passes with no `thin` item; read `enth describe` for the readings.
2. Write `goals/<id>.toml` and `witness/<id>.toml` (copy an existing pair). The rationale says what each item measures, why the windows, the trap it avoids and what the witness reads.
3. `cargo xtask agent-eval audit --goals <id> --cross` and `cargo xtask agent-eval --dry-run --goals <id>`; then the whole set.
4. Bump `eval_version` in `eval.toml` with a changelog line. A goal's brief, performance or list, a prompt, or the wrapper's render accounting changes it; a witness alone does not, but say so in the next changelog line.
5. Run the paid eval only with the owner's approval (`--confirm-spend`; [p1a-g1.md](p1a-g1.md#the-real-run)). At the plan's estimate of about $0.14 per trial, the full set is 30 goals × 3 trials × 2 tracks = 180 trials, about $25: above the default `--max-usd 20`, which stops starting trials at that estimate, so pass `--max-usd 30` or run one track at a time.

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

## Reference targets (not built)

R1 adds reference sounds to the text goals: **25 seeded targets** from the randomizer (lane RG, wave 2) and **5 recordings** that are CC0 or the owner's (report 02 §2.10). Nothing reads `content/agent-evals/targets/targets.toml` yet; it reserves the slots and names the plug points. How they plug in:

- **Goal schema `enth.agent-eval/2`**, additive over `/1`: an optional `[target]` table. Seeded: `kind = "seeded"`, `code = "<challenge code>"` (the `RandomSpec` code, which carries the generator and engine versions and a target-hash prefix; a mismatch is refused). Recorded: `kind = "recording"`, `wav = "targets/<id>.wav"`, `licence = "CC0-1.0"` or `"owner"`, `source` (URL, or "owner, date"). `expect` becomes optional when a target is set; `perf` stays (the performance the target plays, and the one the agent must pass).
- **The trial directory** holds only the wrapper and `target.wav`: the harness renders a seeded target from its code with the goal's performance at run time (never copying the code or the patch), or copies the recording. A third prompt, `prompts/target.md`, and a third track, `target`; adding it is an eval-version change.
- **The judge** is `enth compare <final patch> --target target.wav` with the challenge's calibration (`enth-challenge`'s `sound/1` scorer, lane SS): pass when the sound score reaches the challenge's threshold. A descriptor list may be added as a second judge. Records: sound %, renders, tokens, cost.
- **Witnesses and floors.** A seeded target's witness is its own patch (it scores 100 by construction). Beside the agent, two floors from report 02: **no-op** (the start patch's score) and **CMA-ES only** (`enth match` on every parameter, the same render budget). Recordings have no witness; the floors still apply.
- **Selection.** Seeded: 5 per difficulty 1–5, fixed seeds, through `enth random --seed N --difficulty d` with the randomizer's own rejection of silent and clipping targets; a target-hash golden guards generator drift (the generator version is recorded in every result). Recordings: about 1–3 s, mono, 48 kHz, small enough for git; a plucked string, a bass note, a sustained tone, a drum hit and a texture, chosen by the owner.
- **Needs before it can run:** `enth random` with codes (RG), `enth compare` on the game's scorer (SS, CL), and in the harness `target` handling in `goal.rs`, installing `target.wav` beside the wrapper, the compare judge and the report columns.

**Edit goals** (report 02: start from a flawed patch, clicky, harsh or clipping) need a start patch that is not a template: an optional `start = "starts/<id>.patch.json"` that the harness copies into the trial directory, and a prompt variant. Not built; no goal uses it yet.
