# Lesson plan (R2 Learn)

> **What this is.** The plan that content lanes C1 to C3 (and DEC, for the studies) build the lessons from, and the table that `cargo xtask lesson-coverage` will check them against: every lesson of the R2 curriculum with its new controls, its Hear pairs, its Do challenges, the kinds of check it uses and the pack it ships in, plus the tour, the coverage of the catalogue, and what each lesson waits for.
>
> **Source.** The R2 Learn research session of 2026-10-21 ([summary](../../_local/sessions/2026-10-21-r2-learn/summary.md), [report 01](../../_local/sessions/2026-10-21-r2-learn/01-pedagogy-curriculum.md) §3, §4 and §6, [report 03](../../_local/sessions/2026-10-21-r2-learn/03-architecture-plan.md) §3, §4 and §10), the catalogue at [`crates/enth-patch/src/catalogue/`](../../crates/enth-patch/src/catalogue/) and the 22 drafts in [`content/presets-candidates/`](../presets-candidates/README.md), read at main `dafae9d` by lane CUR.
>
> **Not the schema.** The lesson and pack formats come later (lanes LM and CV2, frozen together at F1l). `Draw`, `edit` and `Kinds` below are shorthand for what the content needs, not a format; the content lanes translate the cards into the schema once it exists. Like the other docs, this plan is a suggestion: change a lesson here when playing it shows it is wrong.

## At a glance

| What | Count |
| --- | --- |
| Lessons | 42: the 36 core lessons of units U0 to U8, and 6 studies (U9) |
| Packs | 4: `basics` 16, `shaping` 13, `space-and-dynamics` 7, `studies` 6 (§2) |
| Tour | 12 lessons (6 in `basics`, 3 in `shaping`, 3 in `space-and-dynamics`), about 1.5 h, with 0.1 as the opening orientation (§3) |
| Parameters in the catalogue | 54 in 10 groups, counted from the source: 24 Basic, 19 Intermediate, 11 Advanced (report 01 is right; §7.1) |
| Parameters with a lesson | 54 of 54. Modules with no non-patch check: none |
| Weak spots | 29 parameters have the ear question as their only reliable non-patch check, 7 of them Basic-tier (§7.5); 10 lessons are carried by ear questions alone |
| Waiting on another lane | Drive (2.5 and the 6 studies) on DRV; new phrases on CV2; route draws, macro pin and Listen options on the schema lanes (§8) |
| S2a (lane LS2, 2026-10-27) | A fifth pack, `bass`: 17 lessons (16 core lessons in units U10 to U14 and one study) for S2a's kinds, aimed at dubstep; hidden until INT-S2a (§10) |

## 1. How to read this plan

### 1.1 Addresses

Lessons use the instance ids of the starter patch (`xtask/src/preset/starter.patch.json`, which Init matches): `osc1` (on), `osc2` and `noise1` (off), `filter1`, `env1` (the amplitude envelope) and `env2`, `lfo1` and `lfo2`, `reverb1` and `comp1` (off), the built-ins `voice.*` and `master.level`, and `macro1` to `macro8`. A route is `source -> destination`; its depth is a share of the destination's range (a cutoff route is 10 % per octave, `osc1.semi` is 1 % per 0.96 st). Every address in this file is checked against the catalogue and the starter patch, and every route destination against `modulatable()`. The coverage tables use the kind form (`osc.wave`).

### 1.2 The four kinds of check

A lesson step has a goal that a check decides (report 03 §3). Four kinds exist, because sound alone cannot check every module (03 §4).

| Kind | Type (report 03) | Passes when | Works for |
| --- | --- | --- | --- |
| **P** patch state | `Check::Patch` of predicates: `Value`, `Route`, `Has`, `Enabled`, `Touched` | the live patch satisfies every predicate (checked after each delivered edit) | every control; the only kind that does |
| **S** sound expectation | `Check::Sound { phrase, expect }`, in the expectation grammar of `enth-analysis` (`brightness<500Hz`) | the rendered phrase meets every item with a margin | pitch, filter, noise, envelopes, LFO rate and depth, master level; little else |
| **R** Recreate score | a real round (`RoundSource::Challenge`) against a hidden `Target::Draw`, `Target::Patch` or `Target::Seed`, with `Constraints.editable` | `structure/1` and `sound/1` reach the pass mark (90 %) | oscillator, filter, noise, voices with the right phrase; weak for short envelopes, effects; never for gauges and nuisances |
| **E** ear question | `StepKind::Choose` over rendered options (a lesson step; Challenge v2 has no `AnswerKind::Choice`, F1l) | the player picks the right option; options are at least 2 just-noticeable steps apart | everything; the only reliable check for resonance, reverb, the compressor and macros |

A `~` after S or R means the check exists but is **weak or conditional**: a probe to measure first, a score that barely moves, or a structure ring with no sound behind it. It never gates a lesson alone. The **Kinds** line of each card lists what it uses; every lesson has P and E. The Check step's mastery rule is report 01's: 2 of 3 Do challenges passed with no level-4 hint or Reveal, plus 1 or 2 mixed "what changed?" items, which come from a pool made of the E items of this lesson and the earlier ones (interleaving). The **Try** step (a goal checked live, with only the new control showing) uses P or S.

Numbers in S items (`brightness>400Hz`) are **indicative**: the authors measure the start and the witness with `enth describe` and keep a margin of at least 1e-3 (03 §8).

### 1.3 Draw, edit and the just-noticeable step

A Do challenge is a hidden target drawn from a short list: `Draw(<phrase>) address ∈ {values}`, with `edit` naming the controls the player may change. Rules for every Draw:

- **Value sets first, ranges later.** Early lessons draw 3 to 5 values at least 2 just-noticeable steps apart from each other and from the start; siblings (redo after a Reveal, Review) draw from the continuous range with a new seed. Steps come from the generator's table ([`priors::jnd`](../../crates/enth-challenge/src/priors.rs)): cutoff 2 st; times, ratios and LFO rates a factor of 1.5; a level 3 dB; dB and cents 3; semitones 1; other percentages 10; a route depth 5 %; any change of an enum or toggle.
- **Never draw what the phrase cannot reveal** ([design.md §9](../../docs/design.md#9-randomizer-and-difficulty)): `osc1.pw` unless the wave is pulse, `osc1.position` unless it is a table, sync on the first oscillator, anything in a disabled module, an attack longer than the 1.5 s hold or a release longer than the 1.5 s tail, keytrack without two notes an octave apart, velocity without two velocities, glide without overlapping or detached notes. Phase mode, phase and seed are nuisances, and the random LFO shapes follow the render seed: neither is ever a hidden target.
- **Gauges are ratios.** A lone source's level and `master.level` do not change `sound/1` (loudness-normalised); a level is drawn only against another source.
- **Earlier controls** stay editable and dimmed from the second Do on; the unit's last lesson uses the full editor (report 01 §3). Hints by tier: Basic lessons open all four coach levels, Intermediate open levels 3 and 4 after a first Submit, Advanced give none on the first try.

### 1.4 Base sounds and phrases

Hear pairs and starts are built from five base sounds, shared by all packs (each pack carries its own copies, 2 KB each): ops on Init (`osc1` saw, `filter1` lp 2 kHz, resonance 10 %, `env1` A 5 / D 300 / S 70 / R 400 ms).

| Name | Recipe |
| --- | --- |
| `saw` | Init as it is |
| `sine` | `osc1.wave` sine |
| `pulse` | `osc1.wave` pulse, `osc1.pw` 50 % |
| `pad` | `env1.attack` 600, `env1.sustain` 90, `env1.release` 1200 ms, `filter1.cutoff` 1500 Hz |
| `pluck` | `env1.attack` 2, `env1.decay` 250, `env1.sustain` 0, `env1.release` 150 ms, `filter1.cutoff` 3000 Hz |

Notes are named C2 to C5 (C3 is 130.8 Hz). **Hear** examples may use any performance; only the S and R checks need a phrase from the table in [`phrases.rs`](../../crates/enth-challenge/src/phrases.rs). Today's table has `HELD` (id 1: C3 held 1.5 s in a 3 s render), `OCTAVE_PAIR` (2: C3 at velocity 100, then C4 at 50, detached) and `LEGATO_PAIR` (3: C3, then G3 overlapping it). The cards also name four **new** phrases, which §8.1 specifies: `CHORD`, `LONG_HOLD`, `STACCATO` and `STACCATO_RUN`.

### 1.5 Other references

`finding N` is a row of the [agent-evals findings](../../docs/agent-evals.md#findings-for-the-descriptor-and-cli-owners). `risk N` is report 03 §10, `01 §N` and `03 §N` are sections of reports 01 and 03. `D1`, `D2` are the Do challenges of a card. `Qn` is an open question in §9 and `Gn` a coverage gap in §7.5.

## 2. Packs

The split follows the curriculum's units, which follow the Recreate generator's levels, so a learner meets the controls in the order the game will randomize them.

| Pack | Lane | Lessons | Count | Tour | Covers | Waits for |
| --- | --- | --- | --- | --- | --- | --- |
| `basics` | C1 | 0.1; U1 sources 1.1 to 1.7; U2 filter 2.1 to 2.5; U3 amp envelope 3.1 to 3.3 | 16 | 1.1, 1.4, 1.5, 2.1, 2.5, 3.2 | `osc`, `noise`, `svf`, `adsr`, `master` | F1l and LCK; 2.5 after DRV; no new phrase |
| `shaping` | C2 | U4 envelopes as modulators 4.1 to 4.4; U5 LFOs 5.1 to 5.6; U6 voices 6.1 to 6.3 | 13 | 4.1, 5.1, 6.2 | `lfo`, `voice`, routes, `adsr` as a modulator | `CHORD`, `LONG_HOLD`; route draws (Q4) |
| `space-and-dynamics` | C3 | U7 effects 7.1 to 7.4; U8 macros and finishing 8.1 to 8.3 | 7 | 7.1, 7.3, 8.1 | `reverb`, `comp`, macros, gain staging | `STACCATO`, `STACCATO_RUN`, `CHORD`; the macro pin (Q5) |
| `studies` | DEC | U9 studies 9.1 to 9.6 | 6 | none | all modules, through 22 presets | all three packs; DRV; `Target::Patch`; the planner |

**Why this split**

- **Order and dependencies.** No lesson uses a control taught in a later pack. `shaping` reuses `basics`; `space-and-dynamics` reuses both. The one forward glance is 5.5, whose start patch has `reverb1` on, so it needs nothing from 7.1. A learner who stops after `basics` has met everything generator levels 1 and 2 randomize, and the amp envelope.
- **Phrases follow the lane plan.** `basics` needs no new phrase, so C1 can start the moment F1l lands (wave 2, as 03 §11 schedules it). `shaping` needs two new phrases and `space-and-dynamics` up to three, and those packs are scheduled later (wave 4), after CV2 has added them.
- **Effort is balanced by difficulty, not by count.** `basics` has 16 lessons because most of them are sound-checkable (S and R both work), which is fast to write and to verify. `space-and-dynamics` has 7 because 4 of them (7.1 to 7.4) have no usable sound check, so each needs hand-made ear-question options, a listening review and new phrases. C1 is about 10 lessons in 03 §11; here the Basic-tier ten (0.1, 1.1, 1.2, 1.4, 1.5, 2.1, 2.2, 2.3, 3.1, 3.2) are a first batch, which is also what the owner's taste check and the tour need, and the other six follow once LCK exists. For the taste check, 1.1 and 2.1 are the two to taste: both are in the tour, one draws an enum and one a continuous value with an S check.
- **The tier does not decide the pack.** Tier is per parameter, and Basic-tier parameters occur in every unit (4.2, 5.1, 5.2, 6.2, 7.1, 7.3, 8.1, 8.2), so a "Basic pack" would split the units.
- **Studies are a fourth pack.** The summary names three packs for C1 to C3; the studies come from a different lane and pipeline (`lesson-gen deconstruct`, with text edited by hand), need all the modules, and wait for the owner's preset pick and for DRV. A separate pack lets DEC own `content/lessons/studies/` without crossing C3's files, and lets a player skip it. If only three packs are wanted, put them after 8.3 in `space-and-dynamics` (Q2).
- **Tour lessons are written first inside each pack**, so each content lane delivers its tour lessons before the rest (Q1 asks how the tour is played across packs).

## 3. The 12-lesson tour

The friend track for GR2: one lesson per module, about 7 minutes each (report 01 §4).

| # | Lesson | Pack | Teaches | Non-patch check |
| --- | --- | --- | --- | --- |
| 1 | 1.1 Waveforms | `basics` | `osc1.wave` | S, R, E |
| 2 | 1.4 Two oscillators | `basics` | `osc2.fine`, `osc2.level` | S, R, E |
| 3 | 1.5 Noise | `basics` | `noise1.color`, `noise1.level` | S, R, E |
| 4 | 2.1 Low-pass cutoff | `basics` | `filter1.cutoff` | S, R, E |
| 5 | 2.5 Drive | `basics` | `filter1.drive` | E, S~ and R~ (after DRV) |
| 6 | 3.2 Decay and sustain | `basics` | `env1.decay`, `env1.sustain` | S, E, R~ |
| 7 | 4.1 Routes and depth | `shaping` | `env2 -> filter1.cutoff` | S, R, E |
| 8 | 5.1 Vibrato | `shaping` | `lfo1.rate`, `lfo1 -> osc1.fine` | R, E, S~ |
| 9 | 6.2 Glide | `shaping` | `voice.glide`, `voice.glide_mode` | R, E, S~ |
| 10 | 7.1 Reverb space | `space-and-dynamics` | `reverb1.wet`, `reverb1.room`, `reverb1.decay` | E, S~, R~ |
| 11 | 7.3 Compressor level | `space-and-dynamics` | `comp1.threshold`, `comp1.ratio` | E, S~, R~ |
| 12 | 8.1 Macros | `space-and-dynamics` | `macro1` and its routes | E, S~ |

- **Coverage of the tour.** It touches 9 of the 10 parameter groups: osc, noise, svf, adsr, lfo, voice, reverb, comp and macro, plus the modulation matrix. **`master` is missing.** Lesson 0.1 teaches `master.level` and is where a friend learns the round controls, so it should open the tour as lesson 0 (not counted in the 12; Q3).
- **The tour needs the whole of its three packs' tour lessons**, so it is not playable until C2 and C3 have delivered theirs. Lesson 5 (Drive) waits for DRV.
- **Four tour lessons are carried by ear questions** (2.5, 7.1, 7.3 and 8.1): they have no reliable sound check. That is a property of the modules (03 §4); the per-lesson votes of GR2 will show whether it works.

## 4. Lesson index

Generated from the cards in §5. **Kinds** are the check kinds the card uses (§1.2). **Waits for** lists what a lesson needs that does not exist yet (§8): a phrase id, `Target::Patch`, `Start::Patch`, route draws (Q4), the macro pin (Q5), Listen options (Q7: a level-match switch, a render seed, a control curve) or DRV. Every Do needs `Target::Draw` with `Constraints.editable`, every Try or S a lesson's `Check::Sound` and every E a lesson's `StepKind::Choose` (§8.2; Challenge v2 left `Target::Expectations` and `AnswerKind::Choice` out at F1l, since those two lesson pieces say them), so those are not repeated.

| Lesson | Title | Pack | Tier | Tour | New controls | Kinds | Waits for |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0.1 | How a lesson works | `basics` | B |  | `master.level` | P S R E | level-match switch |
| 1.1 | Waveforms | `basics` | B | tour | `osc1.wave` | P S R E | Target::Seed |
| 1.2 | Octaves and intervals | `basics` | B |  | `osc1.semi` | P S R E | — (D2 a draw with a lock since F1l) |
| 1.3 | Pulse width | `basics` | I |  | `osc1.pw` | P S~ R E | — |
| 1.4 | Two oscillators | `basics` | B | tour | `osc2`, `osc2.fine`, `osc2.level` | P S R E | — |
| 1.5 | Noise | `basics` | B | tour | `noise1`, `noise1.color`, `noise1.level` | P S R E | — |
| 1.6 | Hard sync | `basics` | I |  | `osc2.sync` | P S R E | — |
| 1.7 | Wavetables | `basics` | I |  | `osc1.table`, `osc1.position` | P S~ R E | — |
| 2.1 | Low-pass cutoff | `basics` | B | tour | `filter1.cutoff` | P S R E | — |
| 2.2 | Resonance | `basics` | B |  | `filter1.resonance` | P R~ E | — |
| 2.3 | Filter modes | `basics` | B |  | `filter1.mode` | P S R E | — |
| 2.4 | Key tracking | `basics` | I |  | `filter1.keytrack` | P R E | — |
| 2.5 | Drive | `basics` | I | tour | `filter1.drive` | P S~ R~ E | DRV |
| 3.1 | Attack and release | `basics` | B |  | `env1.attack`, `env1.release`, `voice.amp_env` | P S R~ E | — |
| 3.2 | Decay and sustain | `basics` | B | tour | `env1.decay`, `env1.sustain` | P S R~ E | — |
| 3.3 | Velocity | `basics` | I |  | `env1.velocity` | P R E | — |
| 4.1 | Routes and depth | `shaping` | I | tour | `env2 -> filter1.cutoff` | P S R E | route draws |
| 4.2 | Filter-envelope shapes | `shaping` | I |  | `env2.attack`, `env2.decay`, `env2.sustain`, `env2.release` | P S~ R E | Target::Seed |
| 4.3 | Pitch envelope | `shaping` | I |  | `env2 -> osc1.semi` | P S~ R E | route draws |
| 4.4 | Negative depth | `shaping` | I |  | negative route depth (on the 4.1 and 4.3 routes) | P R E | route draws |
| 5.1 | Vibrato | `shaping` | B | tour | `lfo1.rate`, `lfo1 -> osc1.fine` | P S~ R E | route draws |
| 5.2 | Shapes and tremolo | `shaping` | B |  | `lfo1.shape`, `lfo1 -> osc1.level` | P S R E | route draws |
| 5.3 | Wobble and sweeps | `shaping` | I |  | `lfo1 -> filter1.cutoff` | P S R E | LONG_HOLD, route draws |
| 5.4 | PWM and table scanning | `shaping` | I |  | `lfo1 -> osc1.pw`, `lfo1 -> osc1.position` | P S~ R E | LONG_HOLD, route draws |
| 5.5 | Polarity and scope | `shaping` | I |  | `lfo1.polarity`, `lfo1.scope`, `lfo1 -> reverb1.high_cut` | P R~ E | route draws |
| 5.6 | Retrigger or free | `shaping` | A |  | `lfo1.phase_mode`, `lfo1.phase`, `lfo1.seed` | P E | render seed |
| 6.1 | Poly, mono, legato | `shaping` | I |  | `voice.mode`, `voice.polyphony` | P S~ R E | CHORD |
| 6.2 | Glide | `shaping` | I | tour | `voice.glide`, `voice.glide_mode` | P S~ R E | — |
| 6.3 | Playing sources | `shaping` | I |  | `velocity -> filter1.cutoff`, `key -> voice.pan`, `modwheel -> filter1.cutoff`, `voice.pan` | P R E | route draws, control curve |
| 7.1 | Reverb space | `space-and-dynamics` | I | tour | `reverb1`, `reverb1.wet`, `reverb1.dry`, `reverb1.room`, `reverb1.decay` | P S~ R~ E | STACCATO |
| 7.2 | Reverb tone | `space-and-dynamics` | A |  | `reverb1.low_cut`, `reverb1.high_cut`, `reverb1.low_damp`, `reverb1.high_damp`, `reverb1.early`, `reverb1.detune` | P R~ E | STACCATO |
| 7.3 | Compressor level | `space-and-dynamics` | I | tour | `comp1`, `comp1.threshold`, `comp1.ratio`, `comp1.makeup` | P S~ R~ E | STACCATO_RUN, level-match switch |
| 7.4 | Compressor time | `space-and-dynamics` | A |  | `comp1.attack`, `comp1.release`, `comp1.knee`, `comp1.mix` | P R~ E | STACCATO_RUN |
| 8.1 | Macros | `space-and-dynamics` | B | tour | `macro1`, `macro1 -> filter1.cutoff`, `macro1 -> reverb1.wet` | P S~ E | Target::Patch, macro pin |
| 8.2 | Gain staging | `space-and-dynamics` | B |  | none (headroom with `master.level`) | P S E | CHORD |
| 8.3 | Phase and seeds | `space-and-dynamics` | A |  | `osc1.phase_mode`, `osc1.phase`, `osc1.seed` | P S~ E | render seed |
| 9.1 | Bass | `studies` | I |  | none (combines) | P S R E | DRV, Target::Patch |
| 9.2 | Plucks and keys | `studies` | I |  | none (combines) | P S R E | DRV, Target::Patch |
| 9.3 | Pads | `studies` | I |  | none (combines) | P S R E | LONG_HOLD, DRV, Target::Patch |
| 9.4 | Leads and imitation | `studies` | I |  | none (combines) | P S R E | DRV, Target::Patch |
| 9.5 | Percussion | `studies` | I |  | none (combines) | P S R E | DRV, Target::Patch |
| 9.6 | FX and texture | `studies` | I |  | none (combines) | P S R E | LONG_HOLD, DRV, Target::Patch |

## 5. Lessons

One card per lesson, in curriculum order. The bracket after the title is `[pack · tier · tour]`; the tier is the highest learn tier among the parameters the lesson introduces (a lesson that adds only routes is I). Fields:

- **New:** the controls the lesson introduces; §7 counts the addresses here as taught.
- **Hear:** the A/B pairs of the Hear step. Each changes only the new control, on at least two bases or notes (§1.4).
- **Do:** the Try goal, then the Do challenges (§1.3).
- **Checks:** what gates the lesson, by kind (§1.2), with the ear-question items.
- **Kinds:** the kinds in use; `~` marks a weak or conditional check.
- **Needs:** what the lesson waits for beyond the universal pieces (§8.2).

### U0 Orientation

#### 0.1 How a lesson works [basics · B]
- **New:** `master.level`; the round controls (Target, Mine, A/B, Submit, Hint, the rings), which are not parameters.
- **Hear:** `saw` at `master.level` 0 dB vs −12 dB (the louder one "sounds better"), then the same pair level-matched; on `pad` at C3 and `saw` at C4.
- **Do:** Try: "make it quieter than this". D1: one demo round, `Draw(HELD)` `osc1.wave ∈ {saw, sine}`, edit `osc1.wave`, every hint level open. The point is Submit, the rings and the results drawer, not mastery.
- **Checks:** P `Touched master.level` (span ≥ 6 dB). S `peak<-20dBFS` (Try). R the demo round (finishes on Submit; no pass mark). E "same patch or different?" on a level-matched pair.
- **Kinds:** P S R E
- **Needs:** `RoundSource::Challenge` (SL) for D1; Listen pairs whose level match can be switched off (Q7).

### U1 Sources (generator level 1)

#### 1.1 Waveforms [basics · B · tour]
- **New:** `osc1.wave` (sine, triangle, saw, pulse; `table` waits for 1.7).
- **Hear:** sine↔triangle, triangle↔saw, saw↔pulse (50 %), sine↔saw; on `saw` at C2, C3 and C5 and on `pad` at C4. One "which is brighter?" item.
- **Do:** Try: start on sine, "brighter than this". D1: `Draw(HELD)` `osc1.wave ∈ {sine, triangle, saw, pulse}` (the start's value is left out), edit `osc1.wave`, ×3 seeds. D2: a level-1 round, `Target::Seed` (difficulty 0), edit `osc1.wave` and `osc1.semi`.
- **Checks:** S `brightness>400Hz` (Try; sine vs the rest). R D1 and D2 gate; the structure score sees the wave exactly, so it separates pulse from saw where S cannot (finding 9). E "which wave is this?" (4 options, on `saw` and `pad`) and "pulse or saw?" (03 §4: needed). P `osc1.wave` ≠ start.
- **Kinds:** P S R E
- **Needs:** `Target::Seed` through `RoundSource::Challenge` (SL) for D2.

#### 1.2 Octaves and intervals [basics · B]
- **New:** `osc1.semi`.
- **Hear:** 0↔+12 (octave), 0↔+7 (fifth), 0↔−12 on `sine` and `saw` at C3. Same note name, higher or lower.
- **Do:** Try: from C3, "play C4" (+12). D1: `Draw(HELD)` `osc1.semi ∈ {−24, −12, −7, +7, +12, +19, +24}`, edit `osc1.semi`. D2: ~~a level-1 round (as 1.1 D2) with `osc1.wave` locked~~ **refused since F1l** (a seed target takes no lock, and level 1 draws the wave): write it as a `Draw(HELD)` on `osc1.semi` from another start (a saw or a pulse), with `osc1.wave` locked (lane C1 writes it). Range for siblings: whole semitones in [−24, +24].
- **Checks:** S `pitch=<target note>±25ct` (strong: gates D1). R D2. E "higher or lower, and by how much?" (fifth or octave, 3 options). P `Value osc1.semi` in [-24, 24].
- **Kinds:** P S R E
- **Needs:** `Target::Draw` with a lock for D2 (since F1l; it was `Target::Seed` through `RoundSource::Challenge`).

#### 1.3 Pulse width [basics · I]
- **New:** `osc1.pw` (only the pulse wave reads it, so the start is a pulse).
- **Hear:** 50↔25↔10 % on `pulse` at C3 and C2, and 10 % pulse↔`saw` ("thin, reedy").
- **Do:** Try: "thinner than this" (`osc1.pw` below 30). D1: `Draw(HELD)` `osc1.pw ∈ {10, 30, 50}` (start left out), edit `osc1.pw`. Authoring rule: draw only widths ≤ 50 %, because a width and its mirror (100 − width) sound alike.
- **Checks:** S `brightness` rising as the width falls (probe: the descriptor behind `pw` is not built, finding 9). R D1. E "which is the 10 % pulse?" and "pulse or saw?". P `osc1.pw<30`.
- **Kinds:** P S~ R E
- **Needs:** none.

#### 1.4 Two oscillators [basics · B · tour]
- **New:** `osc2` switched on; `osc2.fine`, `osc2.level` (as a ratio to the first oscillator, not a volume).
- **Hear:** `osc2` off↔on at the same pitch; `osc2.fine` 0↔+7 ct (about 0.5 Hz of beating at C3, 1 Hz at C4), 0↔+25 ct; `osc2.level` 25↔100 %. On `saw` at C3 and C4, and `pad` at C3.
- **Do:** Try: "make it beat" (`beating in [0.6Hz,1.4Hz]`). D1: `Draw(HELD)` `osc2.fine ∈ {0, 12, 25}` ct, `osc2.level` locked at 100 %, edit `osc2.fine`. D2: `Draw(HELD)` `osc2.level ∈ {25, 50, 100}` %, `osc2.fine` locked at 12 ct. Draw positive detune only (±x beat alike).
- **Checks:** S `beating` within ±30 % of the target (D1), `beat_depth` for D2 (indicative). R D1 and D2. E "one oscillator or two?", "which is detuned more?" and "which has the louder second oscillator?". P `Enabled osc2`.
- **Kinds:** P S R E
- **Needs:** none; HELD shows detune of 12 ct or more as at least one full beat (smaller values only in Hear).

#### 1.5 Noise [basics · B · tour]
- **New:** `noise1` switched on; `noise1.color`, `noise1.level`.
- **Hear:** `osc1` alone↔+ noise at 30 %↔noise alone (`osc1.level` 0); white↔pink. On `saw` at C3 and `pluck` at C4.
- **Do:** Try: "hissier than this" (`noisiness>0.3`). D1: `Draw(HELD)` `noise1.level ∈ {20, 50, 100}` %, `osc1.level` locked at 100 %, `noise1.color` white. D2: `Draw(HELD)` `noise1.color ∈ {white, pink}` with `noise1.level ∈ {50, 100}` %. Colour is dead at level 0.
- **Checks:** S `noisiness` within a band (D1), `brightness` for colour (white above pink, D2). R D1 and D2. E "white or pink?" on a noise-only clip and "which has more hiss?". P `Enabled noise1`.
- **Kinds:** P S R E
- **Needs:** none.

#### 1.6 Hard sync [basics · I]
- **New:** `osc2.sync` (the first oscillator is the master; semitones are reused from 1.2).
- **Hear:** `osc2.sync` off↔on with `osc2.semi` +7 and +19 (tearing, vocal); on `saw` at C3 and `pulse` at C2.
- **Do:** Try: "make it tear" (`osc2.sync` on, `brightness>1500Hz`). D1: `Draw(HELD)` `osc2.semi ∈ {+5, +10, +15}` with `osc2.sync` on, `osc2.level` locked at 100 %. D2: `osc2.sync ∈ {off, on}` × `osc2.semi ∈ {+7, +19}`. Avoid +12 and +24: integer ratios sync inaudibly. Sync does nothing on the first enabled oscillator.
- **Checks:** S `brightness` above the unsynced start. R D1 and D2. E "synced or just two saws?". P `osc2.sync` is true.
- **Kinds:** P S R E
- **Needs:** none.

#### 1.7 Wavetables [basics · I]
- **New:** `osc1.table` (basic, pulse, tilt, formant, organ), `osc1.position`; the oscillator's fifth wave, table.
- **Hear:** `osc1.position` 0↔50↔100 on `basic` (sine to square), `tilt` (a brightness axis) and `formant` (vowel A to U); `basic`↔`organ` at position 0. On C3 and C4.
- **Do:** Try: "scan to the brightest end of tilt". D1: `Draw(HELD)` `osc1.table ∈ {basic, tilt, formant, organ}` at position 0. D2: `Draw(HELD)` `osc1.position ∈ {0, 35, 70, 100}` % on `tilt` and on `formant`. `osc1.position` is dead unless `osc1.wave` is table.
- **Checks:** S `brightness` rising with position (strong on `tilt` only). R D1 and D2. E "which table is this?" (`formant` vs `organ`) and "which clip is further along the scan?". P `osc1.wave` is table.
- **Kinds:** P S~ R E
- **Needs:** none.

### U2 Filter (generator level 2)

#### 2.1 Low-pass cutoff [basics · B · tour]
- **New:** `filter1.cutoff`.
- **Hear:** 4 kHz↔1 kHz↔300 Hz on `saw` at C3 and C2 and on `pad` at C4. One "which is darker?" item.
- **Do:** Try: "darker than this" (`brightness<500Hz`) from the 2 kHz start. D1: `Draw(HELD)` `filter1.cutoff ∈ {300, 600, 1200, 4000, 8000}` Hz, edit `filter1.cutoff`, ×3. D2: the same parameter as a continuous log range [200 Hz, 12 kHz] (the form siblings and Review use).
- **Checks:** S `brightness` within ±25 % of the target's (gates D1). R D1 and D2. E "which is darker?" and "which has the lower cutoff?" (3 clips). P `filter1.cutoff<1000`.
- **Kinds:** P S R E
- **Needs:** none.

#### 2.2 Resonance [basics · B]
- **New:** `filter1.resonance`.
- **Hear:** 10↔50↔90 % at cutoff 800 Hz and 2 kHz; on `saw` at C3 and `pluck` at C3 ("a whistling peak").
- **Do:** Try: "make it whistle" (`filter1.resonance>70`). D1: `Draw(HELD)` `filter1.resonance ∈ {10, 50, 90}` %, `filter1.cutoff` locked at 800 Hz, edit `filter1.resonance`. D2: the same at 2 kHz on `pluck`.
- **Checks:** no S: there is no resonance descriptor (finding 11). R~ gates D1 and D2 through the structure ring on `filter1.resonance` plus the sound pass. E is the main non-patch check: "which has more resonance?" and "which is the 90 % one?". P `Touched filter1.resonance`.
- **Kinds:** P R~ E
- **Needs:** none. A resonance descriptor (finding 11) would give it an S check.

#### 2.3 Filter modes [basics · B]
- **New:** `filter1.mode` (lp, hp, bp, notch).
- **Hear:** lp↔hp↔bp↔notch at 1 kHz, resonance 40 %; on `saw` at C3 and on noise alone (`noise1`, `osc1.level` 0), which exposes the shape.
- **Do:** Try: "keep only the highs" (hp, `brightness>1500Hz`). D1: `Draw(HELD)` `filter1.mode ∈ {lp, hp, bp, notch}`, `filter1.cutoff` locked at 1 kHz. D2: `filter1.mode` × `filter1.cutoff ∈ {500, 2000}` Hz.
- **Checks:** S `brightness` separates lp from hp; bp and notch rest on R and E. R D1 and D2. E "which mode?" (4 options) on the saw and on the noise clip. P `filter1.mode` is hp.
- **Kinds:** P S R E
- **Needs:** none.

#### 2.4 Key tracking [basics · I]
- **New:** `filter1.keytrack`.
- **Hear:** `filter1.keytrack` 0↔100 % on an ascending run C2 to C5, and a single-note pair showing that on one note keytrack is just a cutoff move. On `saw` and `pad`.
- **Do:** Try: "make the high note as bright as the low one" (`filter1.keytrack>80`). D1: `Draw(OCTAVE_PAIR)` `filter1.keytrack ∈ {0, 50, 100}` %, `filter1.cutoff` locked at 800 Hz. D2: `filter1.keytrack ∈ {0, 100}` × `filter1.cutoff ∈ {500, 1500}` Hz.
- **Checks:** R D1 and D2 (the generator reveals keytrack only with an octave pair, so OCTAVE_PAIR is required). E "which one keeps the same brightness on both notes?" and the confusable pair "cutoff moved or keytrack?" on one note (not answerable; it teaches the limit). S would need per-note brightness (not in the descriptors). P `filter1.keytrack>80`.
- **Kinds:** P R E
- **Needs:** none.

#### 2.5 Drive [basics · I · tour]
- **New:** `filter1.drive`.
- **Hear:** 0↔12↔24 dB, level-matched, on `saw` at C3 and `pad` at C4: grit and harmonics, not loudness.
- **Do:** Try: "grittier than this" (`filter1.drive>10`). D1: `Draw(HELD)` `filter1.drive ∈ {0, 12, 24}` dB (start left out), `filter1.cutoff` locked at 1.5 kHz and `filter1.resonance` at 20 %, edit `filter1.drive`.
- **Checks:** S~ `brightness` above the undriven start (probe after DRV). R~ D1 (partly, 03 §4). E "which has drive?" (needed). P `Touched filter1.drive`.
- **Kinds:** P S~ R~ E
- **Needs:** lane DRV's merge (the drive curve, engine 0.3.0, challenge pins). Draft after it, not before.

### U3 Amp envelope (generator level 3)

#### 3.1 Attack and release [basics · B]
- **New:** `env1.attack`, `env1.release`; "env1 is the voice's amplitude envelope" (`voice.amp_env`) is named here.
- **Hear:** attack 2↔300 ms; release 20↔600 ms; on `saw` at C3 and `pad` at C4.
- **Do:** Try: "make it swell" (`attack>200ms`). D1: `Draw(HELD)` `env1.attack ∈ {2, 25, 200, 900}` ms, `env1.release` locked at 150 ms. D2: `env1.release ∈ {20, 150, 800}` ms, `env1.attack` locked at 5 ms. D3 (Review form): both. HELD hides an attack over 1.5 s or a release over the 1.5 s tail.
- **Checks:** S `attack` and `release` within ±30 % of the target (gates; envelope checks replace Recreate, risk 11). R~ D1 and D2 (weak for short attacks, not gating). E "plucky or swelling?" and "which has the longer release?". P `Touched env1.attack`.
- **Kinds:** P S R~ E
- **Needs:** none.

#### 3.2 Decay and sustain [basics · B · tour]
- **New:** `env1.decay`, `env1.sustain`.
- **Hear:** sustain 100 % (organ) ↔ 0 % with decay 250 ms (pluck) ↔ sustain 35 % with decay 600 ms (e-piano); on `saw` and `sine` at C3 and C4.
- **Do:** Try: "make a pluck" (`sustain<-30dB`). D1: `Draw(HELD)` `env1.sustain ∈ {0, 35, 100}` %, `env1.decay` locked at 300 ms. D2: `env1.decay ∈ {80, 1000}` ms × `env1.sustain ∈ {0, 35}` %. Decay is dead at sustain 100 %.
- **Checks:** S `sustain` (dB) and `decay` (ms) within ±30 % (gates). R~ D1 and D2 (weak). E "organ or pluck?" and "which decays longer?". P `Touched env1.sustain`.
- **Kinds:** P S R~ E
- **Needs:** none.

#### 3.3 Velocity [basics · I]
- **New:** `env1.velocity`.
- **Hear:** `env1.velocity` 0↔100 % on a soft-then-hard pair of the same key: at 0 both notes are equally loud, at 100 the soft one is quieter. On `saw` and `pluck` at C3 (free performance).
- **Do:** Try: "make the soft note quieter" (`env1.velocity>60`). D1: `Draw(OCTAVE_PAIR)` `env1.velocity ∈ {0, 50, 100}` %, edit `env1.velocity`; the pair carries two velocities, which the generator needs to reveal the amount.
- **Checks:** R D1 (strong: the pair reveals it). No S (the descriptors read the whole phrase, not one note). E "which responds to how hard you play?". P `env1.velocity>60`.
- **Kinds:** P R E
- **Needs:** none (OCTAVE_PAIR changes the key too, which does not matter for the amount).

### U4 Envelopes as modulators (generator level 3 draws the filter envelope)

#### 4.1 Routes and depth [shaping · I · tour]
- **New:** the route `env2 -> filter1.cutoff` and its depth. Depth is a share of the cutoff's log range: 10 % is one octave. `env2.*` is reused as a modulator.
- **Hear:** no route↔route at +60 %; depth 20↔40↔60 %; on `pluck` at C3 and a `saw` bass at C2 (cutoff 400 Hz, `env2` A 1 / D 300 / S 0).
- **Do:** Try: "make it pluck" (`Route env2→filter1.cutoff>20`, `sweep_depth>1`). D1: `Draw(HELD)` route depth ∈ {0, 20, 40, 60} %, `env2.decay` locked at 300 ms, edit the depth. D2: depth × `env2.decay ∈ {120, 400, 900}` ms.
- **Checks:** S `sweep_depth` within a band and `brightness` (an envelope sweep reads as a filter sweep; indicative). R D1 and D2. E "which one has the filter envelope?". P `Route env2→filter1.cutoff` depth.
- **Kinds:** P S R E
- **Needs:** route draws (Q4): `Target::Draw` must draw route depths.

#### 4.2 Filter-envelope shapes [shaping · I]
- **New:** the four `env2` times as a modulator: `env2.attack`, `env2.decay`, `env2.sustain`, `env2.release` (same controls as 3.1 and 3.2, new job).
- **Hear:** pluck (`env2` A 2 / D 200 / S 0) ↔ brass swell (A 400 / D 300 / S 60) with the route at +50 %; on `saw` at C3 and C4.
- **Do:** Try: "make it swell" (`sweep_time>250ms`). D1: `Draw(HELD)` `env2.attack ∈ {2, 60, 400}` ms × `env2.sustain ∈ {0, 60}` %, route at +50 % locked. D2: a level-3 round, `Target::Seed` (difficulty 50), edit `env1.*`, `env2.*`, the route depth, `filter1.cutoff`, `filter1.resonance`.
- **Checks:** S~ `sweep_time` (direction is not in the vocabulary, finding 2). R D1 and D2 (level 3 randomizes exactly these). E "pluck or swell?". P `Touched env2.attack`.
- **Kinds:** P S~ R E
- **Needs:** `Target::Seed` through `RoundSource::Challenge` (SL).

#### 4.3 Pitch envelope [shaping · I]
- **New:** the route `env2 -> osc1.semi`. A depth of 12.5 % of the ±48 st range is +12 st.
- **Hear:** depth 0↔+12↔+36 st with `env2` A 0 / D 60 / S 0, on `sine` at C1 and C2 (a kick) and `saw` at C4 (a zap).
- **Do:** Try: "make it a kick" (`Route env2→osc1.semi>10`, `stability<0.5`). D1: `Draw(HELD)` route depth ∈ {+12, +24, +36} st, `env2.decay` locked at 60 ms. D2: `env2.decay ∈ {40, 150, 500}` ms at +24 st.
- **Checks:** S~ `stability` (a falling pitch reads as unstable, finding 6). R D1 and D2. E "does the pitch fall or rise?" (no direction descriptor, finding 11). P the route and its sign.
- **Kinds:** P S~ R E
- **Needs:** route draws (Q4). Study 9.5 builds on it.

#### 4.4 Negative depth [shaping · I]
- **New:** a route depth below zero (a mirrored envelope).
- **Hear:** `env2 -> filter1.cutoff` at +50↔−50 % (opens vs closes) at base cutoff 3 kHz; `env2 -> osc1.semi` at −12 st (starts low, rises to pitch); on `saw` at C3 and `pad` at C4.
- **Do:** Try: "make it close instead of open" (`Route env2→filter1.cutoff<-20`). D1: `Draw(HELD)` the cutoff route depth ∈ {−60, −30, +30, +60} % × `env2.decay ∈ {200, 800}` ms. D2: `Draw(HELD)` the `osc1.semi` route depth ∈ {−24, −12, +12, +24} st.
- **Checks:** R D1 and D2 (the sign is what a Recreate round hears). No S: the sweep direction has no expectation (finding 2). E "does it open or close?" and "does the pitch start low or high?". P the sign of the depth (robust).
- **Kinds:** P R E
- **Needs:** route draws (Q4).

### U5 LFOs (generator level 4)

#### 5.1 Vibrato [shaping · B · tour]
- **New:** `lfo1.rate`; the route `lfo1 -> osc1.fine` (depth 12.5 % is ±25 ct).
- **Hear:** rate 0.5↔5↔12 Hz at ±25 ct; depth 0↔±25↔±100 ct at 5 Hz; on `saw` and `sine` at C4.
- **Do:** Try: "add vibrato" (`Route lfo1→osc1.fine>5`, `stability<0.95`). D1: `Draw(HELD)` `lfo1.rate ∈ {1.5, 4, 9, 18}` Hz, the route locked at 12.5 %, edit `lfo1.rate`. D2: `Draw(HELD)` the route depth ∈ {6, 12.5, 25} %, `lfo1.rate` locked at 5 Hz.
- **Checks:** S~ `stability` falling with depth (probe; there is no vibrato descriptor, findings 3 and 11). R D1 and D2 (level 4 draws LFO routes and rates). E "slow or fast?" and "vibrato or tremolo?" (pitch vs loudness). P the route and `lfo1.rate` range.
- **Kinds:** P S~ R E
- **Needs:** route draws (Q4).

#### 5.2 Shapes and tremolo [shaping · B]
- **New:** `lfo1.shape` (sine, triangle, saw_up, saw_down, square, sample_hold, smooth_random); the route `lfo1 -> osc1.level` (tremolo, with the base level at 60 %).
- **Hear:** tremolo at 4 Hz with sine↔square↔saw_down↔sample_hold; vibrato with a square (a trill); on `saw` and `pad` at C3.
- **Do:** Try: "make it tremble" (`tremolo in [3Hz,6Hz]`). D1: `Draw(HELD)` `lfo1.shape ∈ {sine, triangle, square, saw_down}`, route and rate locked, edit `lfo1.shape`. D2: shape × `lfo1.rate ∈ {3, 7}` Hz. The random shapes (sample_hold, smooth_random) follow the render seed: Hear and E only, never a hidden target.
- **Checks:** S `tremolo` rate and depth (the shape itself is not read, so S gates the rate). R D1 and D2. E "which shape?" (sine, square, saw, sample_hold). P `lfo1.shape`.
- **Kinds:** P S R E
- **Needs:** route draws (Q4).

#### 5.3 Wobble and sweeps [shaping · I]
- **New:** the route `lfo1 -> filter1.cutoff` and its depth; the rate over a wide range.
- **Hear:** wobble at 2 and 6 Hz, depth 40 %, on `saw` at C2 with `filter1.resonance` 60 %; a slow sweep at 0.3 Hz on `pad` at C3.
- **Do:** Try: "make it wobble" (`filter_lfo in [2Hz,4Hz]`). D1: `Draw(HELD)` `lfo1.rate ∈ {1.5, 4, 9}` Hz × depth ∈ {20, 45, 70} %, base cutoff 1.2 kHz. D2: `Draw(LONG_HOLD)` `lfo1.rate ∈ {0.3, 0.7}` Hz on `pad`.
- **Checks:** S `filter_lfo` (rate) and `sweep_depth` (depth), found only with two or more cycles (finding 8), hence LONG_HOLD for the slow rates. R D1 and D2. E "wobble or vibrato?" (cutoff vs pitch). P the route.
- **Kinds:** P S R E
- **Needs:** LONG_HOLD for D2; route draws (Q4).

#### 5.4 PWM and table scanning [shaping · I]
- **New:** the routes `lfo1 -> osc1.pw` and `lfo1 -> osc1.position`.
- **Hear:** PWM on `pulse` at 0.8 Hz, depth 0↔25↔45 %; `lfo1 -> osc1.position` over the `basic` table at 0.5 Hz; on C3 and C2.
- **Do:** Try: "make the pulse move". D1: `Draw(LONG_HOLD)` the `osc1.pw` route depth ∈ {15, 30, 45} %, `lfo1.rate` locked at 0.8 Hz. D2: `Draw(LONG_HOLD)` the `osc1.position` route depth ∈ {20, 50, 80} % on `tilt`, `lfo1.rate` locked at 0.5 Hz.
- **Checks:** S~ `filter_lfo` (PWM and scanning move the spectrum slowly; probe). R D1 and D2. E "static or moving?" and "PWM or vibrato?". P the route.
- **Kinds:** P S~ R E
- **Needs:** LONG_HOLD; route draws (Q4).

#### 5.5 Polarity and scope [shaping · I]
- **New:** `lfo1.polarity`, `lfo1.scope`; a global LFO on an effect (`lfo1 -> reverb1.high_cut`; a voice LFO may not).
- **Hear:** a 3 Hz square tremolo bipolar↔unipolar; voice↔global with two staggered notes (the voice LFO restarts on each note, the global one keeps running); a global LFO on `reverb1.high_cut`. Start patches have `reverb1` on, so this lesson needs nothing from 7.1.
- **Do:** Try: "route an LFO to the reverb" (`lfo1.scope` global; the refusal for a voice LFO is the lesson). D1: `Draw(OCTAVE_PAIR)` `lfo1.scope ∈ {voice, global}` × `lfo1.polarity ∈ {bipolar, unipolar}`, the tremolo route locked, edit both.
- **Checks:** R~ D1 (the structure ring is exact; the sound score sees little). No S: reverb movement and scope have no descriptor. E "which LFO restarts on every note?" and "bipolar or unipolar?". P `lfo1.scope` is global and the effect route exists.
- **Kinds:** P R~ E
- **Needs:** route draws (Q4).

#### 5.6 Retrigger or free [shaping · A]
- **New:** `lfo1.phase_mode`, `lfo1.phase`, `lfo1.seed`.
- **Hear:** a sine LFO on the cutoff in reset, free and seeded mode across three render seeds (reset and seeded repeat; free does not), then a sample_hold LFO (only seeded repeats, because random steps come from the render seed otherwise); `lfo1.phase` 0↔180° at note start in reset mode. On `saw` and `pad` at C3.
- **Do:** Try: "make every note start the same" (`lfo1.phase_mode` reset). Challenges are patch states: "make two renders identical with random steps" (seeded plus a seed) and "start at the bottom of the wave" (reset plus a phase). There is no Recreate round: phase mode, phase and seed are nuisances the generator never draws.
- **Checks:** E "which two renders are identical?", "does it start the same each note?" and "which clip starts with the LFO at its peak?" (the only non-patch check). P as above, with `Touched lfo1.seed`.
- **Kinds:** P E
- **Needs:** a render seed per Listen example (Q7).

### U6 Voices and playing

#### 6.1 Poly, mono, legato [shaping · I]
- **New:** `voice.mode` (poly, mono, legato), `voice.polyphony`.
- **Hear:** a C-major chord in poly↔mono (mono keeps one note); two overlapping notes in mono↔legato (mono restarts the envelope, legato does not); `voice.polyphony` 1↔3↔8 on a four-note chord. On `saw` and `pluck`.
- **Do:** Try: "play the chord" (poly). D1: `Draw(CHORD)` `voice.mode ∈ {poly, mono, legato}`. D2: `Draw(LEGATO_PAIR)` `voice.mode ∈ {mono, legato}`. D3: `Draw(CHORD)` `voice.polyphony ∈ {1, 2, 8}` in poly.
- **Checks:** R D1 to D3 (new phrases, 03 §4). S~ `loudness` and `peak` higher in poly on the chord (probe). E "mono or poly?", "retrigger or legato?" and "how many notes are sounding?" (1, 2 or 4). P `voice.mode`.
- **Kinds:** P S~ R E
- **Needs:** CHORD.

#### 6.2 Glide [shaping · I · tour]
- **New:** `voice.glide`, `voice.glide_mode` (always, legato).
- **Hear:** `voice.glide` 0↔150↔600 ms on two overlapping notes in legato mode (bass at C2, lead at C4); `voice.glide_mode` always↔legato on detached notes and on overlapping ones.
- **Do:** Try: "slide between the notes" (`stability<0.5`). D1: `Draw(LEGATO_PAIR)` `voice.glide ∈ {0, 120, 400, 1500}` ms, `voice.mode` locked at legato. D2: `Draw(OCTAVE_PAIR)` `voice.glide_mode ∈ {always, legato}` at `voice.glide` 400 ms (the detached pair is where the two modes differ).
- **Checks:** R D1 and D2 (existing phrases suffice; the generator reveals glide with legato or overlapping notes). S~ `stability` falling with glide (finding 6, probe). E "which one glides?" and "does it glide on detached notes?". P `Value voice.glide>0`.
- **Kinds:** P S~ R E
- **Needs:** none.

#### 6.3 Playing sources [shaping · I]
- **New:** the sources `velocity`, `key` and `modwheel` as route sources (`velocity -> filter1.cutoff`, `key -> voice.pan`, `modwheel -> filter1.cutoff`); `voice.pan`.
- **Hear:** velocity to cutoff (soft dark, hard bright); key to pan (low notes left, high right; headphones); the mod wheel 0→1 opening the cutoff. On `saw` and `pluck`. The wheel needs a performance with a control curve.
- **Do:** Try: "make hard notes brighter" (`Route velocity→filter1.cutoff>20`). D1: `Draw(OCTAVE_PAIR)` the velocity route depth ∈ {0, 30, 60} %. D2: `Draw(OCTAVE_PAIR)` the `key -> filter1.cutoff` depth ∈ {0, 40, 80} %. The pan and wheel routes are patch checks plus E.
- **Checks:** R D1 and D2 (OCTAVE_PAIR carries two velocities and two keys). E "does the harder note get brighter?", "which side is the high note on?" (stereo; no width descriptor, 03 §4), "does the wheel brighten it?". P the routes and signs. R~ for pan if `sound/1` hears stereo (to check).
- **Kinds:** P R E
- **Needs:** route draws (Q4); a control curve in free performances (wheel, Q7); stereo listening.

### U7 Effects (generator level 5)

#### 7.1 Reverb space [space-and-dynamics · I · tour]
- **New:** `reverb1` switched on; `reverb1.wet`, `reverb1.dry`, `reverb1.room`, `reverb1.decay`.
- **Hear:** `reverb1.wet` off (−60 dB)↔−18↔−6 dB; `reverb1.decay` 0.4↔3↔8 s; `reverb1.room` 20↔150 ms; `reverb1.dry` off (fully wet). On `pluck` (a decaying sound, where a tail shows) and `pad` at C3 (a held sound, where the tail shows only after release).
- **Do:** Try: "put it in a hall" (`duration>2500ms`). Start: `reverb1` on, `reverb1.wet` −60 dB. D1: `Draw(STACCATO)` `reverb1.wet ∈ {−18, −9, −3}` dB. D2: `reverb1.decay ∈ {400, 1500, 6000}` ms at `reverb1.wet` −9 dB. D3: `reverb1.room ∈ {20, 70, 160}` ms.
- **Checks:** S~ `duration` and `release` rising with decay, `noisiness>0.02` for the wet level (finding 7: only on a decaying sound; a held pad reads dry). R~ D1 to D3 (03 §4: partly). E "which room is bigger?", "which tail is longer?", "which is wetter?" (needed). P `Enabled reverb1` and `reverb1.wet>-30`.
- **Kinds:** P S~ R~ E
- **Needs:** STACCATO for decays over about 2.5 s (HELD ends at 3 s).

#### 7.2 Reverb tone [space-and-dynamics · A]
- **New:** `reverb1.low_cut`, `reverb1.high_cut`, `reverb1.low_damp`, `reverb1.high_damp`, `reverb1.early`, `reverb1.detune`.
- **Hear:** (exaggerated pairs) `reverb1.high_cut` 12 kHz↔2 kHz; `reverb1.low_cut` 80↔400 Hz; `reverb1.high_damp` 2.5↔8; `reverb1.low_damp` 1.5↔6; `reverb1.early` 0↔250 %; `reverb1.detune` 2↔30. On `pluck` and a plain `sine` at C4.
- **Do:** Try: "darken the tail" (`reverb1.high_cut<4000`). D1: `Draw(STACCATO)` `reverb1.high_cut ∈ {2000, 5000}` Hz × `reverb1.low_cut ∈ {80, 250, 500}` Hz. The four advanced controls get no hidden target: they are heard as "what changed?" items (the curriculum's rule for hard-to-hear controls).
- **Checks:** R~ D1 (intermediate pair only). No S: no tail descriptor (finding 7's follow-up). E "what changed?" (cut high, cut low, damping, early, detune; a different pair each item, ≥ 2 JND apart). P `Touched` per control.
- **Kinds:** P R~ E
- **Needs:** STACCATO; the ear-item audibility gate in `lesson-check` (03 §9).

#### 7.3 Compressor level [space-and-dynamics · I · tour]
- **New:** `comp1` switched on; `comp1.threshold`, `comp1.ratio`, `comp1.makeup`.
- **Hear:** `comp1.threshold` −6↔−30 dB at ratio 4; `comp1.ratio` 1↔4↔20 at −30 dB; `comp1.makeup` 0↔+12 dB after heavy compression (this pair is not level-matched, or the lesson is silent). On a plucked run and a held chord.
- **Do:** Try: "tame the peaks" (`comp1.threshold<-20` and `comp1.ratio>3`). D1: `Draw(STACCATO_RUN)` `comp1.threshold ∈ {−36, −28, −8}` dB × `comp1.ratio ∈ {2, 6, 16}`. D2: `comp1.makeup ∈ {6, 12, 18}` dB, which `sound/1` cannot see because it is loudness-normalised, so it is a patch and ear check only.
- **Checks:** P the gate for D1 and D2 (value bands, hit by the witness only: `--cross`). S~ `peak` and `loudness` together on STACCATO_RUN (peak down, loudness kept; probe, and only if a plain `master.level` move does not satisfy it). R~ weak, not gating (finding 10). E "which one is compressed?", "which has the higher ratio?" and "same dynamics, which has makeup?" (the guaranteed non-patch checks).
- **Kinds:** P S~ R~ E
- **Needs:** STACCATO_RUN (soft: for the S and R probes only); un-matched Listen pairs (Q7).

#### 7.4 Compressor time [space-and-dynamics · A]
- **New:** `comp1.attack`, `comp1.release`, `comp1.knee`, `comp1.mix`.
- **Hear:** `comp1.attack` 0.5↔40 ms on a plucked run (the snap survives a slow attack); `comp1.release` 20↔800 ms (pumping); `comp1.knee` 0↔24 dB; `comp1.mix` 100↔30 % (parallel). On a drum-like `pluck` run and a `pad` under it.
- **Do:** Try: "keep the snap" (`comp1.attack>20`). D1: `Draw(STACCATO_RUN)` `comp1.attack ∈ {1, 10, 60}` ms × `comp1.release ∈ {40, 200, 1000}` ms. D2: `comp1.mix ∈ {30, 65, 100}` %. `comp1.knee` has no hidden target (E only).
- **Checks:** P bands as in 7.3. R~ weak, not gating. No S. E "slow or fast attack?", "which one is parallel?", "hard or soft knee?". `Touched` for each control.
- **Kinds:** P R~ E
- **Needs:** STACCATO_RUN (soft).

### U8 Macros and finishing

#### 8.1 Macros [space-and-dynamics · B · tour]
- **New:** the macro knob `macro1` (a 0–1 position) and routes from it with a depth of either sign (`macro1 -> filter1.cutoff`, `macro1 -> reverb1.wet`).
- **Hear:** one knob doing several things: `macro1` 0→1 opening `filter1.cutoff`, raising `reverb1.wet` and `filter1.resonance` together; then the candidates' Bright and Space macros (Velvet Pad) as the real-world case. On `saw` and `pad` at C3.
- **Do:** Try: "wire a Bright macro" (`Route macro1→filter1.cutoff>20`). D1: build it: routes `macro1 -> filter1.cutoff` (+50 %) and `macro1 -> osc2.level` (+40 %), then set `macro1` to 1. D2 (sibling): a `Target::Patch` where `macro1` also lowers `env1.release` (negative depth).
- **Checks:** P the routes, their signs and `macro1` at 1 (the gate). S~ with `macro1` pinned: at 0 `brightness<A`, at 1 `brightness>B` (needs a macro pin in the sound check, Q5; 03 §4 says no S here). E "which one has the macro at 1?" and "which macro is this (bright or space)?". No R (macros are not randomized).
- **Kinds:** P S~ E
- **Needs:** `Target::Patch`; the macro pin and a macro predicate form (Q5).

#### 8.2 Gain staging [space-and-dynamics · B]
- **New:** no new control. `master.level` (from 0.1) is used for headroom; `osc1.level`, `osc2.level`, `noise1.level` and `comp1.makeup` are seen as gauges, not as tone.
- **Hear:** a four-note chord at `master.level` 0 dB (clips, harsh) vs −12 dB; three sources at 100 % into one voice (clips) vs balanced; makeup vs master. On `saw` and `pad`, with the chord.
- **Do:** Try: "stop it clipping" (`clipping==false`). D1: a clipping start (three sources at 100 %, `master.level` +6 dB) that the player fixes without changing the sound. Overshoot ∈ {+4, +8, +12} dB. Editable: `master.level`, `osc1.level`, `osc2.level`, `noise1.level`, `comp1.makeup`; locked: filter, envelopes, effects.
- **Checks:** S `clipping==false`, `peak<-1dBFS`, `loudness>-24LUFS` (not silent) and `brightness` within ±15 % of the start (the tone is kept): strong, all built features. P `master.level<0`. E "which clips?" and "which is louder?". No R: levels are gauges the generator never draws alone.
- **Kinds:** P S E
- **Needs:** CHORD (a chord is what clips).

#### 8.3 Phase and seeds [space-and-dynamics · A]
- **New:** `osc1.phase_mode`, `osc1.phase`, `osc1.seed` (the LFO versions were taught in 5.6).
- **Hear:** three renders in free, reset and seeded mode (free differs, the others repeat); `osc1` and `osc2` at the same pitch with `osc2.phase` 0↔90↔180° in reset mode (the timbre changes). On `saw` at C3 and C4.
- **Do:** Try: "make every note start the same" (`osc1.phase_mode` reset). Challenges are patch states: "make two renders identical" (reset, or seeded plus `osc1.seed`), "make the pair thin" (`osc2.phase` near 180°). No Recreate round (nuisances).
- **Checks:** E "which two renders are identical?" and "0° or 180°?" (the main non-patch check). S~ `loudness` or `brightness` shifting between 0° and 180° at the same pitch (probe). P as above, `Touched osc1.seed`.
- **Kinds:** P S~ E
- **Needs:** a render seed per Listen example (Q7).

### U9 Studies (built from the preset candidates; lane DEC)

Each study is a deconstruct lesson: at most about 8 steps in signal-flow order, each step a `Target::Patch` of the Init sound plus the elements so far, with one element editable. Steps are scored by `structure/1` on that element plus a `sound/1` pass; the coach is on (report 01 §6). Every study ends with "What disappeared?" (the full preset minus one element, 1 of 4) and a variation goal ("keep it plucky, make it brighter") checked by expectations derived from the preset. The patches are the pack's own copies of the candidates, so the owner's pick does not matter; they wait for DRV, because all 22 carry a Drive macro (`macro4`, which raises `filter1.drive` and `comp1.makeup`) that moves with it. Subjects and elements below come from the candidate patches.

#### 9.1 Bass [studies · I]
- **New:** nothing new; it combines `osc1.wave`, `filter1.resonance`, `env2 -> filter1.cutoff`, `voice.mode` and `voice.glide`.
- **Hear:** Acid Snap built element by element, each step with and without. Elements: saw → `filter1.cutoff` 320 Hz and `filter1.resonance` 55 % → `env2 -> filter1.cutoff` +50 % (the snap) → legato mode with `voice.glide` 70 ms (the slide).
- **Do:** D1 to D4: the four steps above as patch targets. Variation siblings: Sub Floor (sine sub with an `osc2` octave, glide 40 ms), Warm Reese (two detuned saws, `osc2.fine`), Wobble Machine (`lfo1 -> filter1.cutoff`).
- **Checks:** P and R per step (structure on the editable element, sound pass). E "What disappeared?" (Acid Snap minus the envelope, the resonance, the glide or the drive). S the variation goal (`brightness`, `attack<30ms`).
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; DRV; prototype on Acid Snap first (01 §6).

#### 9.2 Plucks and keys [studies · I]
- **New:** nothing new; it combines two oscillators, a filter envelope, `env1.sustain` 0 and a reverb.
- **Hear:** Glass Pluck element by element: saw → `osc2` pulse an octave up at 30 % → `env2 -> filter1.cutoff` +45 % → `env1.decay` 650 ms with `env1.sustain` 0 → `reverb1.wet` −16 dB. Siblings: Poly Stab (`key -> voice.pan`, detune), Drawbar Organ (`osc1.table` organ, `lfo2 -> voice.pan`), Tremolo Keys (`lfo1 -> osc1.level`, a fading partial from `env2 -> osc2.level`), Soft Bell (a third oscillator, long ring).
- **Do:** D1 to D5 as patch targets. The sibling variation is the exam: "turn Glass Pluck into a bell" (longer decay, an inharmonic partial).
- **Checks:** P and R per step; S the variation goal (`decay`, `brightness`, `duration`); E "What disappeared?" and "pluck, organ or bell?".
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; DRV. Soft Bell uses `osc3`, a module no lesson adds (G5, §7).

#### 9.3 Pads [studies · I]
- **New:** nothing new; it combines detune, a slow envelope, a hall and `key -> voice.pan`.
- **Hear:** Velvet Pad element by element: two saws at `osc1.fine` −8 ct and `osc2.fine` +8 ct → `filter1.cutoff` 1.1 kHz → `env1.attack` 700 ms and `env1.release` 1.8 s → `reverb1.wet` −12 dB with `reverb1.decay` 3.5 s → `key -> voice.pan` +30 %. Siblings: Aurora (`lfo1 -> filter1.cutoff` at 0.8 Hz), Night Drift (triangle, sub saw, pink breath), Gate Keeper (a square `lfo1 -> osc1.level` chop at 8 Hz).
- **Do:** D1 to D5 as patch targets; the variation is "make it breathe" (Aurora's LFO) or "make it gate" (Gate Keeper).
- **Checks:** P and R per step; S the variation goal (`filter_lfo`, `tremolo`, `attack>400ms`); E "What disappeared?" and "static or breathing?".
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; LONG_HOLD for the slow LFO steps; DRV.

#### 9.4 Leads and imitation [studies · I]
- **New:** nothing new; the imitation study: a Breath Flute from a reference sound.
- **Hear:** Breath Flute element by element: triangle → `noise1.level` 70 % through a low-pass at 3.2 kHz (the breath) → `env1.attack` 160 ms (the swell) → `lfo1 -> osc1.fine` vibrato at 5 Hz → legato mode with `voice.glide` 50 ms → a light hall. Siblings: Solar Lead (two bright detuned saws), Pixel Square (a plain square), Sync Tear (`osc2.sync` and `env2 -> osc2.semi`).
- **Do:** D1 to D5 as patch targets; the exam is a Recreate round against Breath Flute heard only as a target (R at level 5, no editable limit).
- **Checks:** R (the exam, gating) plus per-step P and R; S the variation goal (`noisiness`, `attack`, `stability`); E "What disappeared?" and "flute or reed?".
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; DRV.

#### 9.5 Percussion [studies · I]
- **New:** nothing new; it combines the pitch envelope, noise and very short envelopes.
- **Hear:** Thump Kick element by element: `osc1` sine → `env2 -> osc1.semi` +37.5 % (+36 st) with `env2.decay` 70 ms (the drop) → `env1.decay` 450 ms, `env1.sustain` 0 → `env2 -> noise1.level` (the click) → mono mode. Sibling: Snare Crack (a noise burst over a tonal body).
- **Do:** D1 to D4 as patch targets; the variation is "turn the kick into a tom" (a slower, smaller drop).
- **Checks:** P and R per step; S the variation goal (`pitch`, `decay`, `stability`); E "What disappeared?" and "kick or snare?". Short sounds are valid targets (no duration filter).
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; DRV.

#### 9.6 FX and texture [studies · I]
- **New:** nothing new; it combines noise, a band-pass sweep and slow modulation.
- **Hear:** Wind Tunnel element by element: `noise1` alone → `filter1.mode` bp at 900 Hz, `filter1.resonance` 45 % → `lfo1 -> filter1.cutoff` at 0.7 Hz → `env1.attack` 900 ms → `reverb1.wet` −10 dB with `reverb1.decay` 5 s. Its `lfo2` is a smooth-random wander that ablation will drop as fine detail. Siblings: Noise Riser (`env2 -> filter1.cutoff` +55 %, long attack), Octave Arp (`lfo1 -> osc1.semi` stepping octaves).
- **Do:** D1 to D4 as patch targets; the variation is "make it rise" (Noise Riser's envelope).
- **Checks:** P and R per step; S the variation goal (`noisiness`, `filter_lfo`, `sweep_depth`); E "What disappeared?" and "wind or riser?".
- **Kinds:** P S R E
- **Needs:** `Target::Patch`; LONG_HOLD; DRV.

## 6. Unit tests and placement

A unit test is three Recreate rounds at the unit's generator level (`Target::Seed`, `musical` 100, staircase off), with no hint on the first attempt; the placement test is two of them per unit and lets an experienced player skip (report 01 §4). They are not lessons and are not in the counts above; they need only `RoundSource::Challenge` from lane SL, no Challenge v2 piece.

| Test | After | Level (difficulty) | What the level randomizes (design §9) | Pack |
| --- | --- | --- | --- | --- |
| UT1 | 1.7 | 1 (0) | oscillator 1 wave and octave | `basics` |
| UT2 | 2.5 | 2 (25) | + filter mode, cutoff, resonance | `basics` |
| UT3 | 4.4 | 3 (50) | + amp and filter envelopes, the filter envelope's depth | `shaping` |
| UT4 | 5.6 | 4 (75) | + 1 to 2 LFO routes and their rates | `shaping` |
| UT5 | 7.4 | 5 (100) | + oscillator 2, noise, effects, more routes, structure | `space-and-dynamics` |

- **UT3 comes after U4, not U3.** Level 3 randomizes the filter envelope's depth, which 4.1 teaches; a test after 3.3 would draw a control the player has not met. Lessons 3.1 to 3.3 close on their own Do challenges.
- **U6 and U8 have no level of their own.** UT5's level-5 rounds can draw the octave and legato pairs (design §9, phrases), so voices are exercised there; 8.2 closes U8 with its own S challenge.
- **UT5 waits for DRV.** Level 5 can draw `filter1.drive` (the golden challenge `max-l5-s147` does), so its seeds are chosen after the Drive fix.

## 7. Coverage

### 7.1 The parameter census

Counted from the source on main `dafae9d`, by reading every `ParamSpec` table in [`catalogue/`](../../crates/enth-patch/src/catalogue/) (a small script, run for this plan): **54 parameters**, **24 Basic, 19 Intermediate, 11 Advanced**. Report 01's figures are right. The only difference is wording: `ModuleKind` has **seven** kinds (`osc`, `noise`, `svf`, `adsr`, `lfo`, `reverb`, `comp`), and report 01's "10 module kinds" are those seven plus the built-ins `voice` and `master` and the macro knob (`MACRO_VALUE`), the ten tables that hold the 54. A module's own tier can differ from its parameters': `noise`, `lfo` and `reverb` are Intermediate modules and `comp` is Advanced, yet `noise` has two Basic parameters and `reverb` three.

### 7.2 Module groups

| Group | Parameters (B/I/A) | Taught in | Strong non-patch checks | Only the ear question is guaranteed for |
| --- | --- | --- | --- | --- |
| Oscillator (`ModuleKind::Osc`) | 11 (4/4/3) | 1.1, 1.2, 1.3, 1.4, 1.6, 1.7, 8.3 | S R E | `phase_mode`, `phase`, `seed` |
| Noise (`Noise`) | 2 (2/0/0) | 1.5 | S R E | — |
| Filter (`Svf`) | 5 (3/2/0) | 2.1, 2.2, 2.3, 2.4, 2.5 | S R E | `resonance`, `drive` |
| Envelope (`Adsr`) | 5 (4/1/0) | 3.1, 3.2, 3.3 | S R E | — |
| LFO (`Lfo`) | 7 (2/2/3) | 5.1, 5.2, 5.5, 5.6 | S R E | `polarity`, `scope`, `phase_mode`, `phase`, `seed` |
| Reverb (`Reverb`) | 10 (3/3/4) | 7.1, 7.2 | E | `dry`, `wet`, `room`, `decay`, `early`, `detune`, `low_cut`, `high_cut`, `low_damp`, `high_damp` |
| Compressor (`Comp`) | 7 (2/4/1) | 7.3, 7.4 | E | `threshold`, `ratio`, `attack`, `release`, `knee`, `makeup`, `mix` |
| `voice` (built-in) | 5 (2/3/0) | 6.1, 6.2, 6.3 | R E | `pan` |
| `master` (built-in) | 1 (1/0/0) | 0.1 | S E | — |
| Macro knob (`MACRO_VALUE`) | 1 (1/0/0) | 8.1 | E | `value` |

"Strong non-patch checks" counts a kind when at least one parameter of the group has a reliable one (S and R as in §1.2, E always). The last column lists the parameters of the group for which the ear question is the only reliable non-patch check.

### 7.3 Every parameter, and the lessons that teach it

**Taught in** is the first lesson that introduces the parameter; **Also in** lists later lessons that edit or mention it. **S** names the expectation feature when one is reliable (or `probe:` when it must be measured first); **R** is *yes*, *weak* (a score that barely moves, or a structure ring with no sound behind it) or *no* (never drawn: a gauge or a nuisance); **E** is the ear question.

| Parameter | Tier | Taught in | Also in | S (sound expectation) | R | E | Note |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `osc.wave` | B | 1.1 | 0.1, 1.2, 1.7, 9.1 | brightness | yes | yes | pulse reads like saw (finding 9); R and E separate them |
| `osc.semi` | B | 1.2 | 1.1, 1.6, 4.3, 4.4, 9.4, 9.5, 9.6 | pitch | yes | yes |  |
| `osc.fine` | B | 1.4 | 5.1, 9.1, 9.3, 9.4 | beating | yes | yes | 12 ct or more on HELD |
| `osc.level` | B | 1.4 | 1.5, 1.6, 2.3, 5.2, 8.1, 8.2, 9.2, 9.3 | probe: beat_depth | yes | yes | a ratio between sources; a lone level is a gauge |
| `osc.pw` | I | 1.3 | 5.4 | probe: brightness | yes | yes | draw widths of 50 % or less |
| `osc.sync` | I | 1.6 | 9.4 | brightness | yes | yes | avoid integer ratios |
| `osc.table` | I | 1.7 | 9.2 | no | yes | yes |  |
| `osc.position` | I | 1.7 | 5.4 | probe: brightness | yes | yes | S strong on tilt only |
| `osc.phase_mode` | A | 8.3 | — | no | no | yes | nuisance: never drawn |
| `osc.phase` | A | 8.3 | — | probe: loudness | no | yes | nuisance; two same-pitch oscillators |
| `osc.seed` | A | 8.3 | — | no | no | yes | nuisance: never drawn |
| `noise.color` | B | 1.5 | — | brightness | yes | yes |  |
| `noise.level` | B | 1.5 | 8.2, 9.4, 9.5 | noisiness | yes | yes |  |
| `svf.mode` | B | 2.3 | 9.6 | brightness | yes | yes | S separates lp from hp only |
| `svf.cutoff` | B | 2.1 | 2.2, 2.3, 2.4, 2.5, 4.1, 4.2, 4.4, 5.3, 6.3, 8.1, 9.1, 9.2, 9.3, 9.6 | brightness | yes | yes |  |
| `svf.resonance` | B | 2.2 | 2.5, 4.2, 5.3, 8.1, 9.1, 9.6 | no | weak | yes | no resonance descriptor (finding 11) |
| `svf.keytrack` | I | 2.4 | — | no | yes | yes | OCTAVE_PAIR; S needs per-note features |
| `svf.drive` | I | 2.5 | — | probe: brightness | weak | yes | after DRV |
| `adsr.attack` | B | 3.1 | 4.2, 9.3, 9.4, 9.6 | attack | weak | yes | R weak for short attacks |
| `adsr.decay` | B | 3.2 | 4.1, 4.2, 4.3, 4.4, 9.2, 9.5 | decay | weak | yes |  |
| `adsr.sustain` | B | 3.2 | 4.2, 9.2, 9.5 | sustain | weak | yes |  |
| `adsr.release` | B | 3.1 | 4.2, 8.1, 9.3 | release | weak | yes |  |
| `adsr.velocity` | I | 3.3 | — | no | yes | yes | OCTAVE_PAIR; S needs per-note loudness |
| `lfo.shape` | B | 5.2 | — | no | yes | yes | S reads rate and depth, not shape; random shapes never drawn |
| `lfo.rate` | B | 5.1 | 5.2, 5.3, 5.4 | filter_lfo, tremolo | yes | yes | two or more cycles (finding 8) |
| `lfo.polarity` | I | 5.5 | — | no | weak | yes | structure ring only |
| `lfo.scope` | I | 5.5 | — | no | weak | yes | structure ring only |
| `lfo.phase_mode` | A | 5.6 | — | no | no | yes | nuisance |
| `lfo.phase` | A | 5.6 | — | no | no | yes | nuisance |
| `lfo.seed` | A | 5.6 | — | no | no | yes | nuisance |
| `reverb.dry` | I | 7.1 | — | no | weak | yes |  |
| `reverb.wet` | B | 7.1 | 8.1, 9.2, 9.3, 9.6 | probe: noisiness | weak | yes | decaying sounds only (finding 7) |
| `reverb.room` | B | 7.1 | — | no | weak | yes | no descriptor |
| `reverb.decay` | B | 7.1 | 9.3, 9.6 | probe: release, duration | weak | yes | STACCATO for decays over 3 s |
| `reverb.early` | A | 7.2 | — | no | no | yes | hard to hear |
| `reverb.detune` | A | 7.2 | — | no | no | yes | hard to hear |
| `reverb.low_cut` | I | 7.2 | — | no | weak | yes |  |
| `reverb.high_cut` | I | 7.2 | 5.5 | no | weak | yes |  |
| `reverb.low_damp` | A | 7.2 | — | no | no | yes | hard to hear |
| `reverb.high_damp` | A | 7.2 | — | no | no | yes | hard to hear |
| `comp.threshold` | B | 7.3 | — | probe: peak, loudness | weak | yes | finding 10 |
| `comp.ratio` | B | 7.3 | — | no | weak | yes | finding 10 |
| `comp.attack` | I | 7.4 | — | no | weak | yes |  |
| `comp.release` | I | 7.4 | — | no | weak | yes |  |
| `comp.knee` | A | 7.4 | — | no | no | yes | hard to hear |
| `comp.makeup` | I | 7.3 | 8.2 | no | no | yes | gauge: sound/1 is loudness-normalised |
| `comp.mix` | I | 7.4 | — | no | weak | yes |  |
| `voice.mode` | B | 6.1 | 6.2, 9.1 | probe: loudness, peak | yes | yes | CHORD for poly; LEGATO_PAIR for mono/legato |
| `voice.polyphony` | I | 6.1 | — | probe: loudness | yes | yes | CHORD |
| `voice.glide` | B | 6.2 | 9.1, 9.4 | probe: stability | yes | yes | LEGATO_PAIR |
| `voice.glide_mode` | I | 6.2 | — | probe: stability | yes | yes | OCTAVE_PAIR vs LEGATO_PAIR |
| `voice.pan` | I | 6.3 | 9.2, 9.3 | no | weak | yes | no width descriptor; headphones |
| `master.level` | B | 0.1 | 7.3, 8.2 | peak, loudness, clipping | no | yes | gauge: never drawn alone |
| `macro.value` | B | 8.1 | — | probe: brightness (macro pinned) | no | yes | needs a macro pin (Q5) |

### 7.4 Sources, routes and other controls

| Control | Taught in | Non-patch check | Note |
| --- | --- | --- | --- |
| Route: add, depth, remove | 4.1; reused in 4.3, 4.4, 5.1 to 5.5, 6.3, 8.1 | S, R, E | Depth has no catalogue tier (it is an `Op`, not a `ParamSpec`) |
| Envelope as a source (`env2`) | 4.1 to 4.4 | S, R, E | The same four controls as 3.1 and 3.2 |
| LFO as a source (`lfo1`) | 5.1 to 5.5 | S, R, E | A voice LFO cannot modulate an effect (5.5) |
| `velocity` | 3.3 (the amount in `env1.velocity`), 6.3 (a route) | R, E | Needs `OCTAVE_PAIR` for any check |
| `key` | 2.4 (`filter1.keytrack`), 6.3 (a route) | R, E | Needs `OCTAVE_PAIR` |
| `modwheel` | 6.3 | E only | No table phrase carries a control curve (Q7) |
| `macro1` to `macro8` | 8.1 | E; S~ with the macro pin | Not randomized, so no R |
| Module on or off (`Enabled`) | 1.4 (`osc2`), 1.5 (`noise1`), 7.1 (`reverb1`), 7.3 (`comp1`) | E, P | Start patches switch the module on, so Draw never draws a flag |
| `voice.amp_env` (which envelope is the amplitude envelope) | named in 3.1 | none | Not a parameter; a P check only |
| Add or remove a module (card menu, "+ Add") | none | none | Gap G5 |
| Global FX bypass, macro names, host slots | none | none | Monitor or host state, not sound; out of scope |

### 7.5 Gaps and weak spots

- **No parameter is without a lesson (54 of 54), and no module is without a non-patch check.** Every lesson has an ear question, so the second claim holds for every group; for three of them (reverb, compressor, macro knob) the ear question is the only reliable check.
- **29 parameters have the ear question as their only reliable non-patch check** (11 Advanced, 11 Intermediate, 7 Basic). The Basic seven are `svf.resonance`, `reverb.wet`, `reverb.room`, `reverb.decay`, `comp.threshold`, `comp.ratio` and `macro.value`. They are grouped below; the sound or Recreate probes in the tables may upgrade them once measured.
  - **G1, reverb (all 10).** Sound checks work only on decaying sounds (finding 7), there is no tail or width descriptor, and Recreate barely moves. `reverb.wet`, `reverb.decay` and `reverb.room` rest on E plus the `STACCATO` probes.
  - **G2, compressor (all 7).** No descriptor (finding 10); the pairs of `peak` and `loudness` are a probe, and `comp.makeup` is a gauge `sound/1` cannot see.
  - **G3, macro knob (1).** Not randomized; an S check needs the macro pin (Q5).
  - **G4, nuisances (6).** `osc.phase_mode`, `osc.phase`, `osc.seed` and the three LFO twins are never drawn, by design; they are checked by P and E only.
  - **G5, no lesson for adding or removing a module** (a card's menu, the header's "+ Add"), although the editor has it and Soft Bell uses a third oscillator. Not a parameter, so `lesson-coverage` would not flag it.
  - **G6, the rest (5).** `svf.resonance` (no descriptor, finding 11), `svf.drive` (until DRV), `lfo.polarity` and `lfo.scope` (a structure ring only), and `voice.pan` (no width descriptor).
- **10 lessons are carried by ear questions alone** (no reliable S or R): 2.2, 2.5, 5.5, 5.6, 7.1, 7.2, 7.3, 7.4, 8.1 and 8.3. Four are in the tour (2.5, 7.1, 7.3, 8.1).
- **Hard to hear.** Reverb early reflections and detune, both damping controls, the compressor knee and oscillator phase are the controls an ear question may not resolve even with exaggerated options. If the audibility gate (03 §9) rejects every option set for one, it becomes "taught, not checkable" (Q9).
- **What the acceptance in 03 §11 asks.** "Every module and Basic-tier parameter has a step and a non-patch check" is met; for seven Basic-tier parameters the check is the ear question.

## 8. Dependencies

### 8.1 New phrase ids

The table is append-only; a released phrase never changes (`phrases.rs`). Ids are assigned by CV2; the names are proposals. Existing phrases cover U0 to U5 except where noted.

| Phrase | Notes | Lessons | Why |
| --- | --- | --- | --- |
| `CHORD` | C3, E3, G3 and B3 at velocity 100, held 1.5 s in a 3 s render | 6.1, 8.2; soft: 7.3 | Poly against mono, voice stealing, and a chord that clips are only heard with several notes at once. Also the 4-note chord the candidates are gain-staged for |
| `LONG_HOLD` | C3 at velocity 100, held 8 s in a 10 s render | 5.3, 5.4 (slow rates); soft: 9.3, 9.6 | An LFO is found only after two or more cycles (finding 8); 0.3 to 0.7 Hz needs 8 s. Costs about 3.3 times a `HELD` render |
| `STACCATO` | C3 at velocity 100, held 150 ms, 6 s render | 7.1, 7.2 | A reverb tail shows on a decaying sound (finding 7); `HELD` ends at 3 s, so a decay beyond about 2.5 s is cut off and the knob's top range (up to 30 s) is out of reach |
| `STACCATO_RUN` | C3 four times at velocity 100, each held 120 ms, one every 400 ms, tail 2 s | soft: 7.3, 7.4 | A compressor needs transients to act on; only for the S and R probes (E needs no table phrase) |

- **Not needed:** a detached pair (`OCTAVE_PAIR` is one: the second note starts after the first ends, so `glide_mode` always and legato differ there), a velocity pair (`OCTAVE_PAIR` has two velocities), and a mod-wheel phrase (6.3 uses a free performance for Hear and E).
- **Invariants break (Q8).** `STACCATO` and `STACCATO_RUN` hold notes for 120 to 150 ms, and `every_phrase_meets_the_shared_facts` asserts every table entry holds at least 1500 ms (`MIN_HOLD_MS`) and tails at least 1500 ms; `ids_are_append_only_and_valid` pins the ids to `[1, 2, 3]`. CV2 must split the table (lesson-only entries with their own invariants, never picked by generator v1, whose `PHRASE_WEIGHTS` only name 1 to 3) or relax the tests. The other three satisfy the invariants.

### 8.2 Challenge v2 and the other Learn pieces

| Piece | Lessons that need it | Notes |
| --- | --- | --- |
| `Target::Draw` | 32: every core lesson with a hidden-target Do (all but 5.6, 8.1, 8.2, 8.3) | Draw list of addresses with value sets, `edit` set, seeds and siblings. Must draw **route depths** in 9 lessons (4.1, 4.3, 4.4, 5.1 to 5.5, 6.3; Q4) |
| `Start::Patch` | every Do, core and studies | Each lesson starts from its own patch (a sine, a reverb switched on at −60 dB, a clipping patch), not from `Start::Template` |
| `Target::Patch` | 8.1 (the sibling) and the 6 studies (every step) | A hand-made or generated target; `closest` (lane CLS) gives the nearest-solution feedback on these |
| `Target::Seed` through `RoundSource::Challenge` | 1.1, 4.2 and the five unit tests | Not Challenge v2; lane SL. It takes `editable` but **no lock** (F1l review M2: its draws change with the seed), so 1.2's D2 moved to a draw |
| ~~`Target::Expectations`~~: the lesson's `Check::Sound { phrase, expect }` | 33 lessons with an S or S~ check, plus every Try that uses S | Left out of Challenge v2 at F1l (one representation); the Gauss-Newton `hint` gives the feedback |
| ~~`AnswerKind::Choice`~~: the lesson's `StepKind::Choose` | all 42 | Left out of Challenge v2 at F1l; options rendered live, pairwise at least 2 just-noticeable steps apart |
| `Constraints { editable, locked }` | every Do; `locked` in 19 cards (1.2, 1.4 to 1.6, 2.2 to 2.5, 3.1, 3.2, 4.1 to 4.3, 5.1, 5.2, 5.4, 5.5, 6.2, 8.2) | The session refuses an op on a locked address (`SessionError::Locked`) |
| Also, from the schema lanes | 7.3 and 0.1 (a switch to turn level matching off), 5.6 and 8.3 (a render seed per example), 6.3 (a control curve), 8.1 (the macro pin and a macro predicate form, Q5), the `Touched` predicate (0.1, 2.2, 2.5, 3.1, 3.2, 4.2, 5.6, 7.2, 7.4, 8.3) | Q5 and Q7 |

Content that every card also owes (checked by `lesson-check`, 03 §9): a **start patch** that fails the check, a **witness** (a reference solution touching only the editable controls) that passes it with a margin of at least 1e-3, one authored **listening cue** per Draw (report 01 §5), and, per E item, an option set with the right answer and options at least 2 just-noticeable steps apart.

### 8.3 Other lanes

| Lane | What it blocks | Lessons |
| --- | --- | --- |
| DRV (fu1) | The Drive curve and its level handling change, the engine goes to 0.3.0, and the preset candidates move (all 22 carry a `macro4` Drive that raises `filter1.drive` and `comp1.makeup`) | 2.5 (write it after DRV merges); UT5's seeds; every study (9.1 to 9.6) |
| F1l (LM, CV2, SL) | The schema, Challenge v2, `RoundSource::Challenge`, the Listen options | all |
| PLY | The predicate grammar (`Touched`, `Route`), sound and choice checks | all P, S and E checks |
| LCK | `lesson-check --solve`, `--cross`, `lesson-coverage`, `lessons.yml` | verifying every pack; C1's verification (wave 3) |
| CLS | `closest` (nearest passing patch) for patch targets | 8.1 sibling and the studies' feedback |
| DEC | The planner and `lesson-gen` | 9.1 to 9.6 |
| Owner's preset pick (GS1) | Which candidates ship as factory presets | none: the studies carry their own copies |

### 8.4 Order of work

C1 drafts `basics` in wave 2 (once F1l has frozen the schema) and verifies it in wave 3 (once LCK exists), tour lessons first, with 2.5 after DRV. C2 and C3 follow in wave 4, tour lessons first, with `shaping` before `space-and-dynamics`. DEC builds `studies` after F2l and CLS, from the prototype on Acid Snap and Glass Pluck (report 01 §6). The owner tastes 1.1 and 2.1 before bulk content.

## 9. Open questions

The plan proceeds on the defaults. None blocks the packs' first drafts.

| # | Question | Default |
| --- | --- | --- |
| Q1 | **The tour spans three packs** (6, 3, 3 lessons), but 03 §11 plans GR2 on "the basics pack". How is the tour played? | A track: an ordered list of lesson references the app follows across installed packs. LM decides the field. The alternative is a `tour` pack of copies, 12 duplicated lessons |
| Q2 | **A fourth pack, `studies`**, owned by DEC, instead of three packs | Four packs; if three, the studies go after 8.3 in `space-and-dynamics` |
| Q3 | **`master` is missing from the 12.** Should 0.1 open the tour as lesson 0? | Yes, outside the count of 12 |
| Q4 | **Does `Target::Draw` draw route depths and their sign**, not only parameters? 9 lessons need it (4.1, 4.3, 4.4, 5.1 to 5.5, 6.3) | Yes: a route is an address `source -> destination` in the draw list with values in %. Otherwise those lessons use hand-made `Target::Patch` siblings |
| Q5 | **Macros and sound.** 03 §4 says no S for macros. A macro's position is patch state and the routes act on it, so a sound check could render with macro positions pinned (one more field). Also, a predicate needs a text form for the knob: the catalogue's `MACRO_VALUE` has the key `value`, but only `macro1` as a source and `SetMacro` as an op exist | Add the pin and `macro1` as the predicate form; 8.1 is E-gated either way |
| Q6 | **Reverb, compressor and macros rest on the ear question.** Is that enough for GR2's "a non-patch check for every module"? | Yes, with `lesson-coverage` reporting "E only" separately; the descriptors of findings 7 and 10 would upgrade it |
| Q7 | **Listen examples need three options**: a level-match switch (0.1, and 7.3, where the A/B match would hide `comp1.makeup`), a render seed (5.6, 8.3) and a control curve (6.3, the mod wheel) | All three as optional fields of the example in lesson v1 |
| Q8 | **The phrase table's invariants** forbid short holds and pin the ids (§8.1) | CV2 splits the table into generator phrases and lesson-only phrases |
| Q9 | **Hard-to-hear parameters.** If no ear question is answerable for a control (early reflections, detune, damping, knee, phase), what then? | The audibility gate decides per control; failures are "taught, not checked" and listed in `lesson-coverage` for the owner to accept at GR2 |
| Q10 | **No lesson on adding or removing a module** (G5). Add a lesson 1.8 or leave it? | Leave it; mention it in 1.4 and 1.5 as the way the `osc2` and `noise1` switches work |

## 10. S2a: the `bass` pack (lane LS2)

**Built (2026-10-27, lane LS2, S2a wave 5).** A lesson for every user-facing S2a kind ([phase-s2.md](../../docs/phase-s2.md), rule 6 and LS2's row), aimed at a learner making dubstep: wobble, growl, reese, FM bass. The pack is `content/lessons/bass/`; its `lesson.toml` headers hold what was measured for each step (`enth describe` on the phrase each check plays, `enth compare --phrase` for sound/1's JND), as the other packs' do. `cargo xtask lesson-check --cross --solve bass` passes: 17 lessons, 139 steps, 0 failures (the notes are structure steps: a switch, a list, a route or a transposition, which the measured hints cannot move).

### 10.1 Order

The S2a plan's order, which is also the genre's: tempo first, then grit, then the new oscillator, then the filters and the effects. Three rules from the S2a brief hold: **distortion before OTT** (11.1, 11.2), **FM after the basic `wt`** (12.1, then 12.2), and **the synced LFO and the MSEG before the wobble study** (10.1, 10.2, then 15.1). Every start patch uses only controls taught in an earlier pack or an earlier lesson of this one, plus the lesson's own kind.

| Unit | Lesson | Id | Tier | Teaches | Non-patch checks |
| --- | --- | --- | --- | --- | --- |
| U10 Tempo | 10.1 Tempo sync | `tempo-sync` | A | `lfo.sync`, `lfo.beats`, `lfo.one_shot` | S (the sweep's rate; one-shot: no repeating sweep), E |
| | 10.2 Drawn shapes | `mseg` | A | `mseg.mode`, `mseg.sync`, `mseg.beats`, `mseg.curve1` | S (sweep rate, brightness, one-shot), E |
| U11 Grit | 11.1 Distortion | `distortion` | I | `dist.drive`, `dist.mode`, `dist.mix` | S (brightness), R (two rounds on the drive), E |
| | 11.2 OTT | `ott` | I | `ott.depth`, `ott.upward`, `ott.downward` | S (brightness), E |
| U12 The wavetable oscillator | 12.1 The wavetable oscillator | `wavetable-oscillator` | B | `wt.table`, `wt.position`, `wt.semi`, `wt.level` | S (brightness, pitch), R (two rounds on the position), E |
| | 12.2 FM bass | `fm` | I | `wt.pm`, `wt.pm_index`, `wt.fine`, `wt.level`; **audio routes** | S (brightness; a synced LFO on the index read as beating), E |
| | 12.3 Bend and feedback | `warp-and-feedback` | I | `wt.feedback`, `wt.warp`, `wt.warp_amount` | S (brightness), R (two rounds on the feedback), E |
| | 12.4 AM and ring mod | `am-and-ring` | I | `wt.am` (an audio route into it) | S (beating), E (ring mod by its tremolo) |
| | 12.5 Unison | `unison` | I | `wt.unison`, `wt.detune`, `wt.blend`, `wt.spread` | S (pitch steadiness), E |
| U13 Filters with character | 13.1 The ladder filter | `ladder` | B | `ladder.cutoff`, `ladder.mode`, `ladder.resonance` | S (brightness), E |
| | 13.2 The comb filter | `comb` | B | `comb.feedback`, `comb.semi` | S (brightness, pitch), E |
| | 13.3 The formant filter | `formant` | B | `formant.f1`, `formant.f2` | S (brightness; a yoi read as a sweep), E |
| U14 Movement and space | 14.1 Chorus and flanger | `chorus` | I | `chorus.mix`, `chorus.rate`, `chorus.mode`, `chorus.depth` | S (brightness, the copies' beating), E |
| | 14.2 Phaser | `phaser` | I | `phaser.rate`, `phaser.depth`, `phaser.sync`, `phaser.beats` | S (sweep, beating), E |
| | 14.3 EQ | `eq` | B | `eq.high_cut`, `eq.low_cut`, `eq.bell1_gain`, `eq.high_gain`, `eq.bell2_gain`, `eq.low_gain` | S (brightness), R (rounds on the low cut and the first bell), E |
| | 14.4 Delay | `delay` | I | `delay.mix`, `delay.feedback`, `delay.sync`, `delay.beats` | S (the sound's length, staccato), R (a round on the feedback), E |
| U15 Studies | 15.1 Study: a riddim wobble | `wobble-study` | I | combines `dist`, `svf`, the synced `lfo`, `ott` | R, S, E (generated by `lesson-gen deconstruct`, texts edited by hand) |

The tier is the highest learn tier among the parameters a lesson teaches (§5's rule), so 10.1 is Advanced (TMP put `sync`, `beats` and `one_shot` there) although it is this pack's first lesson; the tier only sets the coach's hint levels in rounds, and 10.1 has none.

### 10.2 What the measurements decided

- **sound/1 hears some S2a controls well and is deaf to others**, as C2 found for the P1 modulators. It hears the drive (3 to 4 JND per 12 dB), the wavetable position, the feedback, the formant vowels, the EQ's cuts and bells, the delay's feedback and division, and now the synced divisions (1/8 against 1/16 4.9 JND). It is nearly deaf to the FM index (about 0.9 JND per step), unison (0.5), the ladder's cutoff and slope (0.9), the comb's feedback on noise (0.2), the chorus (0.7), AM against ring mod at a slow rate (0.1) and OTT's default against none (1.0). **Rounds appear only where it hears the control**: the drive, the position, the feedback, the EQ's low cut and first bell, the delay's feedback. Elsewhere the Do steps check a descriptor that reads the control, and the ear questions pass the audibility gate on the descriptors.
- **Descriptors that read S2a controls:** the filter sweep reads a synced LFO's or MSEG's division to the hundredth of a hertz; a one-shot reads as no repeating sweep; brightness reads drive, OTT's lift, the position, the index, feedback, warps, the ladder, the comb on a saw, the vowels' F1, the chorus's mix and the EQ; pitch reads the comb's tuning on noise; pitch steadiness reads unison; beating reads slow AM, a synced LFO on the FM index and a fast chorus; tremolo reads ring mod; the sound's length on the staccato phrase reads the delay.
- **The measured hint probes a tenth of a control's range toward the middle**, so a continuous goal starts where that probe already moves the sound the right way (OTT's upward at 40 %, not 0 %; the FM index at 3, where 2 and 3 read alike; the chorus's rate above its goal).
- **Checks play the table phrases untransposed (C3)**, so every ear question stays measurable by sound/1 and counts in `lesson-coverage`; a bass an octave down is heard in Hear only.

### 10.3 A limit of the lesson model: audio routes have no selector

A step's `highlight`, `editable` and `locked`, and an `enth:` link, cannot name an audio route such as `wt2 -> wt1.pm`: `enth_challenge::v2`'s `check_route` asks that a route's destination be modulatable, and `wt.pm` and `wt.am` are audio parameters (lane RTE's contract), which are not. Route **predicates** (`wt2 -> wt1.pm > 50%`) work, and so does the playthrough. The FM and AM lessons work around it: the step makes `wt2.*` editable (a module selector covers the routes from that module), locks wt2's parameters one by one, and highlights `wt1.pm` or `wt1.am`. A one-line change in `check_route` (accept an `audio` destination when the source is a generator, as `validate.rs` does) would let a lesson name the route itself; it is not this lane's file, so it is left to the owner of `v2.rs`.

### 10.4 Coverage

`lesson-coverage` counts listed kinds only (`Group::all` is `ModuleKind::listed()`), so the hidden kinds are not in its tables yet and the pack changes nothing there but `lfo` (10.1 teaches its three S2a parameters) and `svf` (the study). With the hide flags cleared in a scratch build (every `hidden: true` in `catalogue/mod.rs` set to false, not committed), `lesson-coverage` over every pack reports **no hole**: every S2a kind has a lesson and a non-patch check, and every Basic-tier parameter of each is taught.

| Kind | Lessons | Checks | Taught (Basic) |
| --- | --- | --- | --- |
| `wt` | 5 | P S R E | 15/18 (5/5) |
| `dist` | 2 | P S R E | 3/9 (2/2) |
| `ott` | 2 | P S E | 3/12 (1/1) |
| `comb` | 1 | P S E | 2/5 (2/2) |
| `formant` | 1 | P S E | 2/5 (2/2) |
| `ladder` | 1 | P S E | 3/6 (3/3) |
| `mseg` | 1 | P S E | 4/59 (3/3) |
| `chorus` | 1 | P S E | 4/9 (3/3) |
| `phaser` | 1 | P S E | 4/9 (2/2) |
| `eq` | 1 | P S R E | 6/13 (6/6) |
| `delay` | 1 | P S R E | 4/8 (3/3) |

Listed apart for the owner at GR2 (Q6, Q9): **E only**, `wt.table`, `wt.warp`, `wt.fine`, `chorus.mode`, `phaser.rate`, `phaser.depth`, `eq.low_gain` and `delay.sync` among the taught ones; **taught, not checked**, `wt.level` (a gauge `sound/1` cannot hear, as `osc.level` alone), `wt.spread` (the descriptors read the mid signal, so stereo width is heard on headphones only), `mseg.sync` and `chorus.depth`. Not taught: 103 Intermediate and Advanced parameters of the S2a kinds, none Basic; 55 of them are the MSEG's (its point values beyond the first curve, its free length, scope, polarity and loop points: a drawn shape is one idea, taught through its first segment), and the rest are secondary controls such as `dist.bias`, `ott.time`, `formant.shift`, `eq`'s frequencies and Qs and `delay.ping_pong`, candidates for a second pass.

### 10.5 What INT-S2a flips

- **`ModuleSpec::hidden`** for each kind, in `crates/enth-patch/src/catalogue/mod.rs`: the pack's lessons then join `lesson-coverage`'s tables with no hole (§10.4).
- **Shipping the pack:** add `bass` to the three `cargo xtask pack --out … shaping space-and-dynamics studies` lines of `.github/workflows/release.yml` (the Linux, Windows and macOS legs) and to `docs/distribution.md`'s list of the friend zip's packs. The plugin bundles only `basics`; bundling `bass` too would be `cargo xtask pack --out crates/enth-plugin/packs bass` and a `BUNDLED` entry in `enth-plugin/src/learn.rs` (not proposed: the friend zip carries the other packs).
- **A track** (optional): a bass track across packs (for example 10.1, 11.1, 12.1, 12.2, 15.1) is a `tracks` entry in `pack.toml`.
- **Nothing else**: the lessons read hidden kinds through the same patch, `teaches`, `covers` and check code as listed ones, so `lesson-check` needs no option and none of its code changed.
