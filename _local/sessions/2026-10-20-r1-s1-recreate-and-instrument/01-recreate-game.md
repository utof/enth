# 01 — The Recreate game: randomizer, difficulty, challenge codes, round flow

Sub-researcher 01, R1/S1 session, 2026-10-20. Scratch, prototype and raw outputs: `~/code/enth-scratch/2026-10-20-r1-s1/01/` (`notes.md`, `proto/`, `measure-200.txt`, `hashes-*.txt`, `timing.txt`).

Evidence labels: **measured** (I ran it), **documented** (a dated source or the repo says so), **anecdotal** (forum or reviewer opinion, or my reasoning without data).

## 1. Summary

The repo already has the hooks R1 needs:
- a `ChallengeBar` placeholder (`enth-ui/src/widgets/bars.rs`);
- the `Overlay` trait with Good/Near/Bad scores;
- `challenge_bar` and `results` regions, reserved behind the `recreate` feature;
- a const `SplitMix64`/`derive`/`fnv1a64` (`enth_dsp::util::rng`);
- `Performance` in `enth-render` (L3);
- the starter superset patch (`xtask/src/preset/starter.patch.json`): every P1 module, the extras disabled.

`enth-challenge` (L4) is declared in `xtask/layers.toml` but doesn't exist yet. `enth random` is documented but not built.

I built a 250-line prototype generator and measured it. Its results drive most rows below.

| Area | Recommendation | Evidence | Confidence |
| --- | --- | --- | --- |
| Draw order | **Keyed streams**: every draw is `derive(derive(seed, fnv1a64(address)), attempt)`, not a sequential RNG. Adding a draw never shifts the others, and a higher difficulty keeps the lower categories' values | measured: osc values the same at d=0 and d=1 for 1000/1000 seeds; 3×1000 target hashes identical on native x86-64 and wasm32-wasip1 | high |
| Priors | Hand-written priors per parameter, relative to the performance's note (LP cutoff ≥ 1.5 × f0). Resonance and drive skewed low; semitones on intervals; envelopes as a pluck/pad mixture. `musical` blends towards uniform | measured (200 per cell): silent 0–1 vs 5–8 for uniform; quieter than −40 LUFS 0–12 vs 43–60. documented: Synth1B1 saw the same | high |
| Observability | Never randomize what the phrase can't reveal: gauges (a lone source's level, `master.level`), dead parameters (`pw` without a pulse, glide or keytrack with one note) and nuisances (phase, seed). The same mask feeds structure scoring and the overlay | measured: `osc1.level` 100→50, `master.level` −6 dB and glide 300 ms all give distance **0.0000**; keytrack 100 % at C3 = cutoff 2→1 kHz (**0.4487** both) | high |
| Minimum step | The target must differ observably from the start. Redraw deterministically with `attempt`, in parameter space only (design.md §9) | measured: without it, 55/200 lowest-level targets were effectively the start | high |
| Reachable values | Quantize targets to a grid a player can hit: 3 significant digits, route depths in 5 % steps | documented: Syntorial snaps controls; its help lists "unreachable value" (2024-12-02) | high |
| Structure in R1 | Randomize **enable flags and routes over the starter superset**. The editor already enables modules and drags routes, so no add/remove is needed | code read | high (owner OK) |
| Difficulty | Five levels, one more category each (design.md §9); `amounts` set how many parameters and how wide. Calibrate empirically: distance from the start stops growing after the filter step | measured: median distance 0.36 / 0.89 / 1.00 / 0.99 / 1.06 | medium |
| Auto difficulty | Optional staircase: two passes up, one fail down (about 71 % passes) | documented (Levitt 1971); anecdotal here | medium-low |
| `RandomSpec` | Integer fields only (`u8` percentages), plus `generator: u16` and a template id. No floats in codes | reasoning: a text→float round trip can flip a draw | high |
| Codes | Versioned binary in Crockford base32 with a check symbol: a short form (~13 characters) for presets, a long one (~32) for custom settings. Daily challenge = a date-derived seed | documented: the Crockford spec; Wordle | medium-high |
| Generator versions | Freeze each version. Its output patch goes through `enth_patch::migrate`. Goldens: 1,000 spec→hash per version on Linux, Windows and wasm | measured (prototype) | high |
| A/B audio | **Target ▶**: a level-matched buffer of the phrase, pre-rendered and looped. **Mine ▶**: the same phrase live through the player's engine, in sync. A/B flips at the same position; the keyboard plays Mine | documented: Syntorial's looped toggle. measured: 52 ms to render a 3 s clip, 20 ms for features (under load) | medium |
| Feedback | No live meter by default. **Submit** puts Good/Near/Bad rings on the knobs; after the first submit they update live. Hints and reveal mark the round as assisted | documented: Syntorial; mixed learning evidence | medium (GR1 decides) |
| Placement | `enth-challenge` (L4): generator, phrases, codes. `enth-session` (L6): a pure `round` module, with time and jobs from the shell. UI and plugin: views, buffer player, worker | code read; AGENTS.md | high |
| GR1 data | A local round log (JSONL: code, patch, scores, time, an agree vote) with export, and a Disagree button | documented: Syntorial's dispute flow | high |
| Session length | Rounds of 2–5 min, sets of 5 (about 10–15 min) | documented: ToneGym 10–15 min; anecdotal for us | medium |

## 2. Findings

### 2.1 What exists, and what's missing (code read 2026-10-20)

- **The ChallengeBar mock.** It already draws difficulty dots, Target ▶ / Mine ▶ / A/B, a timer, live sound and structure scores, Hint and Submit, with placeholder values.
- **Overlay and scores.** The `Overlay` docs say "Recreate never hides; it dims and scores". `theme::Score` (Good/Near/Bad) matches Syntorial 2's added "orange" tier.
- **The RNG.** `derive` and `fnv1a64` are pinned `const fn`s whose only float step is an exact int→float conversion. A generator using them, with maths through the pinned `libm`, inherits cross-platform equality.
- **Spec metadata.** `ParamSpec.learn.tier` (Basic, Intermediate, Advanced) orders parameters within a category. `perceptual` names the descriptor each one moves.
- **The starter superset.** It has `osc2`, `noise1`, `comp1` and `reverb1` disabled, and env2, lfo1 and lfo2 with no routes. With `Intent::Enable` and drag-to-modulate, a target that enables a subset and adds routes can be reached in today's editor. `canonical_hash` already drops disabled modules and depth-0 routes.
- **Missing.** The crate, the generator, phrases and codes, round state, a target playback path in the plugin, and presets: `content/presets/` is empty, so priors start hand-written.

### 2.2 Prior art: what makes recreate-by-ear fun and fair

- **Syntorial**, the closest product (*documented*):
  - **The challenge.** A short note sequence loops through a hidden patch. The player toggles "My Patch" against it while the loop keeps playing, and can also play the hidden patch by hand. Controls snap to a few values. After Submit, right controls go green and wrong ones red, and red turns green once fixed ([Keyboard review](https://www.pluginboutique.com/articles/531-Syntorial-Review-at-Keyboard-Mag-com), about 2014; [Wikipedia](https://en.wikipedia.org/wiki/Syntorial)).
  - **Syntorial 2.0** (announced [2023-06-22](https://www.attackmagazine.com/news/syntorial-announces-2-0-update/); [What's new](https://syntorial.com/whats-new-in-v2/), undated):
    - a **Randomizer mode** with "infinite" new patches;
    - an **orange** tier;
    - fixed values **remapped "to avoid differences that are too subtle to hear"**;
    - group challenges cut from 6 to 3 against "frustrating roadblocks".
  - **2.3 "Remakes"** ([2025-04-01](https://rekkerd.org/?p=280099)): famous sounds, filterable by difficulty, scored at submit.
  - **The [help page](https://audiblegenius.helpjuice.com/en_US/problem-or-issue-with-a-specific-patch-in-a-syntorial-challenge)** (2024-12-02) lists "disagree with the score", "can't be matched" and "unreachable value", and attaches both patches to the report. Those are the three frustrations to design out.
- **Ear-training gyms:**
  - **ToneGym:** "five short ear training games", "10 to 15 minutes a day", levels that "adapt to your skill" ([site](https://www.tonegym.co/?lg=de)). *documented*.
  - **SoundGym 2.0** ([Gearnews 2017-02-20](https://www.gearnews.com/soundgym-2-0-revamps-audio-ear-training/)): adaptive stages, lives, achievements; the reviewer doubts it stays engaging past a month.
  - **SOS on SoundGym** ([review](https://www.soundonsound.com/node/4923945), undated): the five daily games come from a pool of 16, and the reviewer "kept getting roughly the same games". *anecdotal*.
  - Short sessions and variety matter.
- **Shared seeds:**
  - **Wordle** has one daily puzzle "the same for all players worldwide", and its shareable emoji result drove its spread ([Wikipedia](https://en.wikipedia.org/wiki/Wordle)).
  - **Slay the Spire's Daily Climb** gives everyone the same seed ([wiki](https://slaythespire.wiki.gg/wiki/Custom_Mode)); Steam threads report players mixing up 0/O and 1/I when typing seeds.
  - **[Crockford base32](https://www.crockford.com/base32.html)** fixes that: no I, L, O or U; i, l and o read as 1 and 0; a mod-37 check symbol; hyphens are ignored.
- **Randomizers:**
  - **[Synth1B1](https://arxiv.org/pdf/2104.12922)** (2021): uniform sampling gives many near-silent or extreme-pitch renders. *documented*.
  - **[arXiv 2407.16643](https://arxiv.org/pdf/2407.16643)** (DAFx24) samples from the spread of Massive's presets instead. *documented*.
  - **A [KVR](https://www.kvraudio.com/forum/viewtopic.php?p=8898478) user** of u-he's randomizer reports silent patches and odd pitch jumps. *anecdotal*.
- **Learning science** (informs defaults; settles nothing):
  - **Reduced feedback improved retention** in Winstein & Schmidt 1990 (*J Exp Psychol LMC* 16:677), the guidance hypothesis.
  - **The opposite held** for sonified feedback in [Fujii et al. 2016](https://www.frontiersin.org/articles/10.3389/fnins.2016.00251/full).
  - **Nothing tests timbre matching.**
  - **Levitt 1971** (*JASA* 49:467): 1-up/2-down converges on 70.7 % correct.
  - **The "85 % rule"** ([Wilson et al. 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC6831579)) is derived for gradient-descent learners. Using it for humans is *anecdotal*.
  - **Lichess** rates each puzzle like a player with Glicko-2 ([secondary](https://mintlify.com/lichess-org/lila/features/puzzles)). Without a server, enth can only rate the player.
- **For enth** (*anecdotal synthesis*):
  - a looped phrase with instant toggling;
  - hittable values and audible differences;
  - per-control feedback at submit, with a "near" tier, fixable afterwards;
  - short varied sessions;
  - a dispute path;
  - sharing by seed.

### 2.3 The randomizer: measured prototype

**The prototype** (`proto/src/lib.rs`, not product code):

- It starts from the starter superset. Categories switch on at filter ≥ 0.2, envelopes ≥ 0.4, modulation ≥ 0.6, effects and structure ≥ 0.8.
- **Musical prior:**
  - wave weights: saw .45, pulse .30, triangle .15, sine .10;
  - semitones from {0, ±12};
  - LP cutoff log-uniform over [1.5 f0, 12 kHz];
  - resonance `60·u^1.5` %;
  - env1: pluck/pad mixtures;
  - env2→cutoff: +10…+50 %;
  - one LFO route: vibrato, cutoff or tremolo;
  - enabled with probability: osc2 .5, noise .3, reverb .6, compressor .3.
- **Uniform prior:** the same structure decisions, with values uniform over the full normalized range.
- **Rounding:** every value to 3 significant digits.
- **Run:** C3, velocity .8, held 1.5 s in a 3 s clip; release build, toolchain 1.99.1. The load average was about 12, so wall times are upper bounds.

| Prior | Level | Silent | < −40 LUFS | Peak > 0 dBFS (float) | Distance from start p10 / p50 / p90 | Effectively the start (< 0.02) |
| --- | --- | --- | --- | --- | --- | --- |
| musical | 0 (osc) | 0 | 0 | 0 | 0.00 / 0.36 / 2.19 | **55/200** |
| musical | .2 (+filter) | 0 | 1 | 29 | 0.38 / 0.89 / 1.88 | 0 |
| musical | .4 (+env) | 0 | 12 | 22 | 0.56 / 1.00 / 2.05 | 0 |
| musical | .6 (+mod) | 1 | 12 | 24 | 0.60 / 0.99 / 2.05 | 0 |
| musical | .8 (+fx/structure) | 1 | 7 | 25 | 0.63 / 1.06 / 1.80 | 0 |
| uniform | 0 … .8 | 5–8 | 43–60 | 3–7 | p50 1.72–2.23 | 0 |

**Single-parameter moves from the starter** (metric v1, *measured*):

- **Zero:** `osc1.level` 100→50 %, `master.level` −6 dB and glide 300 ms with one note all give **0.0000**.
- **Filter:** cutoff +1 st 0.035, +4 st 0.135, 2→1 kHz 0.449. `keytrack` 100 % gives 0.449, identical to that cutoff move. Resonance 10→40 % gives 0.134.
- **Oscillator:** saw→pulse 0.376; +12 st 0.363.
- **Envelope:** attack 5→20 ms 0.003, 5→100 ms 0.018; release 400→800 ms 0.237.

**What follows:**

1. **Priors remove silence.** Uniform targets are silent 2.5–4 % of the time and very quiet 21–30 %; musical ones almost never.
2. **Gauges and dead parameters are real.** Three of thirteen moves were inaudible after loudness normalisation, and one equals another parameter. The generator needs observability rules, and structure scoring needs the same mask. That is input for report 02.
3. **The minimum step is required.** At level 1, 27.5 % of targets equalled the start: P(saw) × P(0 st) = .45 × .6.
4. **Float peaks.** 11–15 % of musical targets peak above 0 dBFS in float. Level-matched A/B at −23 LUFS fixes playback. The generator may also set `master.level` for headroom: it is a gauge, and so is outside structure.
5. **Distance from the start is a weak difficulty proxy.** Define difficulty by what is randomized.
6. **Metric v1 barely sees attack.** Changing the attack from 5 to 100 ms gives 0.018, half a 1-semitone cutoff move. Flagged for report 02.
7. **Determinism and speed.** 3 × 1,000 targets hash identically on native x86-64 and wasm32-wasip1 (wasmtime 48.0.6), `libm` `pow`/`log10`/`round` included. Generation takes 328 µs per target, mostly parsing the starter JSON.

**Observability rules** (pure, parameter-space; shared with scoring as `observable(&Patch, &Performance) -> Mask`):

- **Gauges never vary alone:** the only source's level, `master.level`, `comp.makeup`. Levels vary only as ratios between sources, and wet only against dry.
- **Dead by structure:**
  - `pw` unless wave = pulse;
  - `position` unless wave = table;
  - `sync` on the first osc;
  - anything in a disabled module;
  - depths of absent routes.
- **Dead by the phrase:**
  - keytrack needs two notes an octave apart;
  - velocity amounts need two velocities;
  - glide needs mono or legato with overlapping notes;
  - attack must be ≤ hold;
  - release must fit the tail.
- **Nuisances:** phase mode, phase, seed.
- **Degenerate pairs** (keytrack and cutoff with one note): randomize at most one.

**Minimum step.** A hand-written just-noticeable step per parameter, in normalized units: cutoff ≥ 2 st, times ×1.5, levels 3 dB, a wave always counts. The target must exceed it on k parameters, where k comes from the level. If it doesn't, bump `attempt` for the failing keys. The attempt is recomputed identically everywhere, because the test is on values, never on a render.

**Musical/safe.** At `musical` = 100 the generator uses the priors. Lower values draw from the prior with probability `musical`, else uniformly. Safety bounds (resonance cap, audible pitch, cutoff relative to f0, total level) stay on above 20. The same knob is R3's dataset knob. Priors fitted to presets later become a new generator version.

**Archetypes.** `RandomSpec.template` picks a start and a prior set (pluck, pad, bass, lead, keys), so a target is recognisably *something*. That is more fun than a random point, and Syntorial's Remakes point the same way. *anecdotal*.

### 2.4 Difficulty

- **Levels** (Basic-tier parameters first in each category):

  | Level | Randomizes | Observable parameters |
  | --- | --- | --- |
  | 1 | osc1 wave and octave | 1–2 |
  | 2 | + filter mode, cutoff, resonance | 3–5 |
  | 3 | + amp and filter envelopes, filter-env depth | 6–10 |
  | 4 | + 1–2 LFO routes and rates | 9–14 |
  | 5 | + osc2/noise/effects and their settings, more routes | 12–20 |

  `amounts` override how many parameters a category draws and how wide.
- **Level 1 is a tutorial level:** about 12 distinct sounds (4 waves × 3 octaves), plus pulse widths. Keep a seed history per session, and widen the level if testers get bored.
- **Calibration** (*proposed*):
  1. Per level, measure the solver's or CMA-ES's evaluations to pass from the start, and the agent solve rate.
  2. Check them against GR1 pass rates.
  3. Then show a predicted difficulty per target.
- **Auto mode:** the 2-up/1-down staircase in steps of 0.05. A local Glicko-style player rating can come later, with duels.
- **"Never locks features."** An assist dims the parameters not in play (Overlay `Dimmed`). It's on at levels 1–2 and is part of the code, so duels match.

### 2.5 The challenge object and codes

- **`RandomSpec`:**
  - `generator: u16`; `seed: u64`; `difficulty: u8`;
  - `amounts: [Option<u8>; 6]`; `musical: u8`; `template: u8`.

  With integers, a code, a JSON file and the UI all reproduce exactly. design.md's `f32` fields don't guarantee that.
- **`Challenge`:**
  - `Target::Seed(RandomSpec)`, `AnswerKind::Patch`;
  - `performance` = a phrase id;
  - `start` = the template;
  - no constraints (R1 locks nothing);
  - `scoring` = scorer ids, versions, weights and pass mark (report 02);
  - `assists` = live meter, dim-unused, hint budget.
- **Phrases.** A versioned table in `enth-challenge`:
  - one held note for levels 1–3;
  - two notes an octave apart with two velocities when keytrack or velocity is in play;
  - a legato pair for mono or glide.

  The generator picks the phrase, so observability can be checked before the round starts.
- **Codes** (*proposed*):
  - **Short form:** `version:4b | preset:8b | generator:12b | seed:40b`, encoded in Crockford base32 plus a check symbol: `R1-7K3M-QX9P-2HDT-V`.
  - **Long form:** adds explicit difficulty, amounts (a bitmask), musical, template, phrase, assists and the scoring version; about 32 symbols.
  - A newer version or generator is refused as `code_too_new`, with a hint.
- **Daily:** `seed = derive(fnv1a64("daily"), day_number)`, with the preset rotating by weekday. The date comes from the shell. The share line is spoiler-free ("enth daily 2026-10-20 · 87/74 · 3:12").
- **R5 result codes** (reserved): challenge code + time + scores + the patch as a compact diff from the start (index + quantized value, about 100 bytes). The friend re-generates and re-scores. They carry the engine version and warn on a mismatch: DSP changes move the audio, not the target.
- **Versioning:**
  - frozen generator modules, each emitting a patch in its own schema, then `migrate`, so catalogue changes keep old codes valid;
  - 1,000-entry goldens per version on three platforms;
  - any prior change is a new version.
- **Reuse.** Agent evals, ML sampling (a dataset is a list of codes), duels and `enth random`, which prints the code too.

### 2.6 The round flow in the plugin

**States:**

1. **Play.**
2. **Setup:** level, amounts, musical, template, seed (random, daily or a code).
3. **Preparing:** generate the target (µs), stash the current patch and history, load the start through `Session::load_patch`, and queue the target render on the worker (about 50 ms under load).
4. **Playing:**
   - the timer runs while the editor is open;
   - Target ▶ and Mine ▶ loop the phrase, and A/B flips between them;
   - the keyboard plays Mine;
   - a hidden worker evaluation runs after edits settle (about 300 ms debounce). It records the first pass, for time-to-pass, and is shown only with the live-meter assist.
5. **Submitted:**
   - sound, structure and combined % with their components (report 02), time and time-to-first-pass;
   - Good/Near/Bad rings on the randomized knobs;
   - target vs mine values on hover;
   - buttons: Keep tweaking (rings update live; the first submit stays the recorded score), Reveal (loads the target; undoable), Retry, Next, Copy code, Save target as preset, Agree / Disagree.
6. **Exit:** restore the stash, or keep mine.

**Audio** (*proposed*):

- **Target:** the offline render of the phrase, which is exactly the scored audio, scaled to −23 LUFS. It is handed to the audio thread as a preallocated `Box<[f32]>` through an `rtrb` slot, with a return ring (the plan-swap pattern).
- **Mine:** the live engine, fed the same phrase by a sequencer in `process()`, locked to the buffer playhead and gain-matched from the worker's latest loudness.
- **A/B:** a 10 ms crossfade at the same position. One engine runs, plus a buffer read.
- **Why not two engines:** twice the RT cost against about 2.5× headroom, a second plan pipeline, and the heard target wouldn't be the scored one.
- **Later:** "keys play Target", if GR1 asks for it. Syntorial allows playing the hidden patch by hand.

**Feedback:**

- No live meter by default (the `ChallengeBar` mock shows live scores).
- Hints come from `enth-solver` and mark the round assisted.
- Reveal = give up.
- The evidence is mixed, so GR1 compares: half the rounds with the meter, half without, then asks which felt better and which taught more.

**Fairness:**

- the timer pauses when the editor closes (baseview can't report a minimised window);
- the start patch has no slots, so host automation does nothing during a round, and the stash restores the slots;
- Disagree logs the code, patch, scores and a comment locally.

**Session:** sets of 5 rounds, each aimed at 2–5 min. GR1's 20 rounds come to about 1–1.5 h per person, a planning figure.

### 2.7 Code structure (proposed)

| Crate (layer) | New items |
| --- | --- |
| `enth-challenge` (L4, core, wasm32) | `RandomSpec`, `Category`, `Level`, `generator::v1` (priors, keyed draws, minimum step), `observable()`, `phrase::PHRASES`, `Challenge`, `Constraints`, `Assists`, `ScoringSpec` (with report 02), `code::{encode, decode}` |
| `enth-solver` (L5) | Coach hints over a challenge's scorer |
| `enth-session` (L6, `recreate`) | `round::{Round, RoundState, RoundJob, JobResult, Stash}`. Entries: `start_round(code, now_ms)`, `round_tick(now_ms, visible)`, `submit`, `reveal`, `end_round(keep)`, `take_round_jobs`, `round_job_done`. The shell supplies time; an optional `recreate` section in the `enth` field (allowed within v1) resumes a round |
| `enth-ui` (L7, `recreate`) | Settings page, `ChallengeBar` presenter, `Results` drawer, `RecreateOverlay`, a `recreate.json` layout (the challenge bar replaces the macro bar) |
| `enth-plugin` (L8) | Clock, worker thread, `TargetPlayer` + `PhrasePlayer` in `process()`, A/B crossfade, round log (JSONL under the user config directory; export only) |
| `enth-cli` (L8) | `enth random` (with code), `enth challenge new\|show\|decode`, `compare --challenge` |

**Tests:**

- generator goldens on three platforms;
- proptests: monotone in difficulty, minimum step met, everything randomized is observable, no lone gauge, the output migrates;
- code round trip, plus the check symbol catching every single substitution and adjacent swap, plus fixtures;
- the round state machine with a fake clock;
- the A/B player under `assert_no_alloc`, including a full return ring.

## 3. Risks and open questions

- **Envelope rounds feel unfairly scored.** Metric v1 barely sees attack (*measured*). Report 02 adds envelope components, or structure carries those levels.
- **Inaudible differences get punished**, unless structure scoring uses the generator's observability mask and excludes gauges (*measured*).
- **Difficulty is miscalibrated until the solver and agent calibration runs.** Label the levels "beta" for GR1.
- **Level 1 repeats** (about 12 sounds).
- **Target ▶ falls silent** in hosts that stop calling `process()` while the transport is stopped, or that sleep an idle instrument. Unverified per host; the standalone build (report 04) avoids it.
- **The stash must survive a mid-round project save** (the `recreate` section), or a crash loses the user's sound.
- **Generator versions pile up.** Keep v1 small, and fit priors only once S1's presets exist.
- **Owner decisions:**
  1. live meter off by default?
  2. dim unused knobs at low levels?
  3. daily challenge in R1 or R5?
  4. "keys play Target" later?
  5. where the challenge bar sits (report 03)?

## 4. Gaps

- **Platforms.** Generator determinism was measured only on x86-64 Linux and wasm32-wasip1; Windows, macOS and aarch64 are not run.
- **Audio path.** The buffer-player A/B path (crossfade, RT cost) is a design, not a measurement.
- **First-hand player reports.** None found on Reddit or KVR for Syntorial or SoundGym; searches returned vendor and retailer pages.
- **Undocumented internals.** Syntorial's randomizer and scoring formula; SoundGym's adaptive algorithm.
- **Human data.** No study found on human patch-matching times, so round lengths are guesses.
- **Calibration.** No solver-based difficulty measured: there is no quick public CMA-ES-to-pass API.
- **Host behaviour.** Whether REAPER, Bitwig or Ableton call `process()` while stopped was not checked.
- **Timing conditions.** Wall times were measured under load average 12.

## 5. Sources

All fetched 2026-10-20.

- Syntorial: [Wikipedia](https://en.wikipedia.org/wiki/Syntorial); [Keyboard review](https://www.pluginboutique.com/articles/531-Syntorial-Review-at-Keyboard-Mag-com) (about 2014); [What's new in V2](https://syntorial.com/whats-new-in-v2/); [Attack, 2023-06-22](https://www.attackmagazine.com/news/syntorial-announces-2-0-update/); [rekkerd, 2025-04-01](https://rekkerd.org/?p=280099); [help, 2024-12-02](https://audiblegenius.helpjuice.com/en_US/problem-or-issue-with-a-specific-patch-in-a-syntorial-challenge)
- Gyms: [Gearnews 2017-02-20](https://www.gearnews.com/soundgym-2-0-revamps-audio-ear-training/); [SOS](https://www.soundonsound.com/node/4923945); [ToneGym](https://www.tonegym.co/?lg=de)
- Seeds and codes: [Wordle](https://en.wikipedia.org/wiki/Wordle); [Slay the Spire Custom Mode](https://slaythespire.wiki.gg/wiki/Custom_Mode); [Crockford base32](https://www.crockford.com/base32.html)
- Sampling: [Synth1B1](https://arxiv.org/pdf/2104.12922); [arXiv 2407.16643](https://arxiv.org/pdf/2407.16643); [KVR u-he randomizer](https://www.kvraudio.com/forum/viewtopic.php?p=8898478)
- Learning: Winstein & Schmidt 1990 (*J Exp Psychol LMC* 16:677; via secondary sources); [Fujii et al. 2016](https://www.frontiersin.org/articles/10.3389/fnins.2016.00251/full); Levitt 1971 (*JASA* 49:467, [PDF](https://moodle2.units.it/pluginfile.php/553227/mod_resource/content/0/Levitt_1971_SCALA.pdf)); [Wilson et al. 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC6831579); [Lichess puzzles](https://mintlify.com/lichess-org/lila/features/puzzles)
- Repo (2026-10-20): `docs/design.md` §5, §6, §9; `docs/roadmap.md`; `docs/editor.md`; `docs/session.md`; `docs/agent-cli.md`; `enth-ui/src/widgets/bars.rs`; `enth-ui/src/editor/{overlay,reserved}.rs`; `enth-dsp/src/util/rng.rs`; `enth-patch/src/catalogue/`; `enth-render/src/lib.rs`; `xtask/layers.toml`; `xtask/src/preset/starter.patch.json`
