# 01 — Ear-training pedagogy and game design (R4)

- **Date:** 2026-10-22. Sub-researcher 01 of the R4/R5 session ([brief](brief.md)).
- **Read in the repo:** the brief, research-process.md, roadmap.md (R4, R5), learn.md (shape, Challenge v2, checks, Review), design.md §9 (generator, JND steps, staircase), challenge.md (phrases), `enth-learn`'s `StepKind::Choose` and `Audition`, and R2 report 01. What R2 report 01 already covered (Schwartz & Bransford, Dunlosky, Kellman, Kim 2013, Kluger & DeNisi, Duolingo streaks, Kreković, Syntorial) is not repeated.
- **Marks:** *measured* (I counted or ran it), *documented* (paper, vendor page), *anecdotal* (forums, reviews, company interviews). Links were checked on 2026-10-22, and several papers only at abstract level (see §6). Scratch: `~/code/enth-scratch/2026-10-22-r4r5/01/notes.md`.

## TL;DR

1. **Sound-design games first.** Musical training improves pitch discrimination but not brightness discrimination ([Allen & Oxenham 2014](https://pmc.ncbi.nlm.nih.gov/articles/PMC3985978/)). So interval and chord drills won't train cutoff, envelope or reverb ears. These games are also the cheapest to build, since every one is a one-item `Target::Draw` plus a Choose item or a one-knob Recreate round.
2. **Top three:**
   - **Spot the change:** which control moved?
   - **Dial it in:** match one knob by ear.
   - **Ear check:** an odd-one-out staircase that reports your threshold in real units. It is also GR4's instrument.
3. **Then the music side, on existing machinery:**
   - Frequency finder: svf bands now, EQ A/B after S2.
   - Interval stacks: `osc.semi`, then phrases with `transpose`.
   - Chord quality.
   - Rhythm pattern: perception only.

   Progressions need seeded phrases. Note naming is opt-in. Tap-along comes last.
4. **Absolute pitch is real but rare in adults.**
   - After about 21 h, 2 of 12 learned all 12 pitches, and 35 of 48 starters quit ([Wong et al. 2025](https://pmc.ncbi.nlm.nih.gov/articles/PMC12325523/)).
   - 2 of 6 adults selected for auditory memory reached AP ([Van Hedger et al. 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC6759182/)).
   - Never make it a headline promise.
5. **Difficulty:**
   - Train at about 80 % with 3-down/1-up ([Wilson et al. 2019](https://doi.org/10.1038/s41467-019-12552-4); [Jansen et al. 2013](https://dare.uva.nl/id/789fc422-4935-49ca-bcba-803239522951)). Recreate keeps 71 %.
   - Calibrate items by measured JND distance and simulated listeners, since there is no telemetry.
6. **The skill is the unit of spacing; the game is the vehicle.** One Leitner box per skill, fed by Recreate rings and by 5-item bursts in any game. Interleave confusable categories once each has been met alone.
7. **Sessions:**
   - Workouts of 3 games × about 4 min, with a clear end.
   - Replay with the answer after each item ([Little et al. 2019](https://featuredcontent.psychonomic.org/perfect-unison-or-perfect-fourth-practice-makes-perfect-only-with-exposure-during-musical-interval-training/)).
   - Show progress as thresholds in physical units.
   - No streaks or leaderboards ([Silverman & Barasch 2023](https://udspace.udel.edu/handle/19716/34160); [Hanus & Fox 2015](https://doi.org/10.1016/j.compedu.2014.08.019)).
8. **GR4 (§5):**
   - **Bots and goldens in CI.**
   - **Human:** trained vs held-out skills within each person, because test practice alone produces learning.

## 1. Prior art

| Product | Format | What works | Complaints |
| --- | --- | --- | --- |
| **SoundGym** ([SOS, July 2021](https://www.soundonsound.com/node/4923945); [site](https://www.soundgym.co/), fetched 2026-10-22; [interview, 2026-09-28](https://magneticmag.com/2026/09/noam-gingold-soundgym-interview/)) | Daily workout of 5 games from a pool of 16: Kit Cut (redraw an EQ cut), Balance Memory, Distorted Reality (2AFC), Reverb Wizard (odd one of 3), ReBeater (step grid), SwingBeat. Tolerance narrows with level. Site today: "Ten minutes a day", streaks, rank, coins, contests | Short, varied "sound atoms"; recreate by dragging (*documented*) | • Repetitive.<br>• **"Pot luck" difficulty**: random settings on uneven material.<br>• Depends on your monitors.<br>• The same few games recur (SOS, *anecdotal*).<br>• Price: $24.95/mo or $119/yr (2021).<br>• Users "get worse during breaks" (company, *anecdotal*) |
| **Quiztones** ([applevis](https://www.applevis.com/apps/ios/music/quiztones-eq-ear-training), [Designing Sound 2012](https://designingsound.org/2012/08/15/quiztones-frequency-ear-training-review/)) | Frequency ID on tones, noise, loops; ±10/+5 dB octaves; 1/3-octave "expert"; gain 1–16 dB | Tiers by gain and bandwidth (*documented*) | Abstract sine guessing; no score tracking in old versions (*anecdotal*) |
| **TrainYourEars EQ 2.0** ([EarMaster shop](https://earmaster.com/shop/product/trainyourears-eq-edition-2-0.html)) | Guess and correct modes; exercise designer (freq, type, gain, Q); your own music; A/B against your answer | **Correct mode**: fix it by ear (*documented*) | A Gearspace user: most tools are "simplistic" sine quizzes, train on your own monitors ([thread](https://gearspace.com/threads/frequency-ear-training.857488/), search snippet, *anecdotal*) |
| **EarMaster 7** ([Thomann](https://thomann.co.uk/earmaster_earmaster_7_download_reviews.htm?rating=5)) | 2,500+ exercises; clapback and singback by mic | Answer by performing (*anecdotal*) | Mic set-up; 4.3/5 from 50 ratings |
| **Functional Ear Trainer** ([App Store](https://apps.apple.com/app/id1088761926)) | A cadence, then name the scale degree | 4.9/5 (583 ratings) | Leans on the tonal centre, so bare intervals feel harder ([ViControl](https://vi-control.net/community/threads/ear-train.144179/), *anecdotal*). No controlled study found |
| **Complete Ear Trainer** ([App Store](https://apps.apple.com/app/id1012455471)) | 150+ drills in 28 chapters; master before moving on; $4.99 | Mastery gating; starts with far-apart pairs | Less depth (*anecdotal*) |
| **Tonedear**, **musictheory.net**, **Teoria** ([tonedear](https://tonedear.com/), [musictheory.net](https://www.musictheory.net/exercises)) | Intervals, chords, progressions, scale degrees, dictation; free or cheap | Simple, configurable | HN thread on Tonedear ([2026-06-08](https://hn.algolia.com/api/v1/items/48447598), 332 points): "doing these exercises regularly for years and haven't made much progress"; progressions "with no thought for voice leading" (*anecdotal*) |
| **Perfect Pitch Puzzle** ([HN 2023-11-24](https://hn.algolia.com/api/v1/items/38408205)) | Daily melody Wordle with a shared result line | Sharing spreads it | "One a day makes it hard to get better"; replay made it too easy (*anecdotal*) |

**Who quits, and why:**

- **Measured attrition:** 35 of 48 dropped out in Wong 2025, and 14 of 34 in Kim 2015's 15-week EQ course (*documented*).
- **Anecdotal reasons:** "little progress after hours of frustration" ([HN 2020](https://hn.algolia.com/api/v1/items/24872754)), repetitiveness, unmusical material, and one-a-day limits.
- **The pattern:** slow gains with no visible measure of progress. No product above shows progress in physical units; R4 should.

## 2. Evidence by skill

**Intervals and relative pitch.**

- **Little, Cheng & Wright 2019** ([AP&P 81](https://doi.org/10.3758/s13414-018-1584-x)): practice plus passive-exposure blocks was the only regime that learned (69 → 88 % in 3 days), and it transferred to untrained timbres (*documented*).
- **Wong, Chen & Lim 2021** ([Psych. of Music 49](https://doi.org/10.1177/0305735620922595)): for novices without reference songs or singing, interleaving beat blocking; with those aids, the two tied. Learners believed blocking worked better (*documented*, abstract).
- **Chin et al. 2024** ([n = 18](https://arxiv.org/html/2407.09721v1)): 20 minutes of training moved accuracy by about 4 points (*documented*).

**For enth:** intervals are slow. Tie them to `osc.semi` stacks (R2 lesson 1.2), which need no new phrase. Melodic intervals need static two-note phrases (append-only ids), with `Audition.transpose` roving the root.

**Absolute pitch (adults).**

| Study | Design | Result | Caveats |
| --- | --- | --- | --- |
| [Van Hedger et al. 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC6759182/) | n = 6, selected for auditory memory; about 32 h | 2 of 6 reached AP and kept it at about 128 days; the rest gained modestly | No group statistics; the best two already scored 60–70 % before training |
| [Wong et al. 2025](https://pmc.ncbi.nlm.nih.gov/articles/PMC12325523/) | 12 musicians; 21.4 h, 15,327 trials | Mean 7.08 pitches at 90 % or better (range 3–12); 2 of 12 got all 12; proctored test 0.139 → 0.317 | 35 of 48 never reached the post-test; no control group or retention test; no accuracy transfer to guitar. About 14.7 % across the group's studies learned all 12 |
| Implicit memory | Singing known songs | [Levitin 1994](https://web-archive.southampton.ac.uk/cogprints.org/643/1/pitch.HTM): 12 % exact on both trials. [Six-lab replication](https://research.gold.ac.uk/id/eprint/10162): 4 % | Common but weak |

**Copy for enth:** "Most adults who train for about 20 hours learn to name a handful of notes; a few learn all twelve; most stop sooner." Use Wong's ladder, but make the game opt-in and leave it out of the default workout. For sound design, pitch *height* (which band) matters more than pitch class, and Frequency finder trains that.

**Chords and progressions.**

- **Chubb et al. 2013** ([JASA 134](https://cwlab.ss.uci.edu/files/2014/01/ChubbEtAl2013.pdf)): across 275 listeners, results were bimodal, and about 70 % sat near chance on major/minor tone-scrambles. Skill correlated about 0.5 with years of training.
- Slower stimuli did not help (Mednicoff et al. 2018).
- Two hours of training helped only a minority of low performers ([UC dissertation](https://escholarship.org/uc/item/61c5158b); author and date not retrieved).

**For enth:** chord quality suits keys and pads (lesson phrase 4 is `chord`; polyphony is 8). Some players will plateau, so never gate content on it. Progressions need seeded note sequences, and no controlled study compares the functional and interval methods.

**Rhythm and syncopation.**

- **Fitch & Rosenfeld 2007** ([Music Perception 25](https://web.uvic.ca/~aschloss/course_mat/MUS%20511/ARTICLES%20AND%20REFS%20FOR%20320/FitchRosenfeld20071.pdf)): the Longuet-Higgins and Lee syncopation index predicts difficulty. Highly syncopated patterns make listeners reset their pulse and are remembered worse after 24 h (*documented*, abstract). This gives a ready-made difficulty ladder.
- **Rhythm Workers** ([Bégel et al. 2018](https://sonicfield.org/library/rhythm-workers)): tapping and perception-only versions, 10 players each over 2 weeks, both "encouraging". Later trials were small (n = 12–27; [Jamey et al. 2025](https://cris.maastrichtuniversity.nl/ws/files/282145334/Kotz-2025-Can-you-beat-the-music.pdf)).
- **Tapping:** people tap tens of milliseconds early (Repp 2005, via citing papers), so tap scoring needs an offset.

**For enth:** perception-only first ("which pattern?", step-grid dictation on static phrases, judged exactly, with no input latency). enth's LFO has no tempo sync (*measured*, in the catalogue).

**EQ, timbre and frequency identification.**

- **Kim 2015** ([JASA-EL 138](https://pubs.aip.org/asa/jasa/article/138/1/EL110/916336/An-assessment-of-individualized-technical-ear)), 15 weeks, n = 10 per group (34 enrolled):
  - Training: 7 bands, boosts going from +12 to +3 dB.
  - Picking each trainee's weak bands beat uniform selection on an EQ-matching test (group × time p = .024).
  - With Kim 2013, this is the case for weakness-targeted Review.
- **Olive 2003** ([JAES 51](https://aes2.org/publications/elibrary-page/?id=12206)): a trained panel was 3–27 times more discriminating than untrained groups, though selection and training are confounded.
- **Brightness needs its own training** ([Allen & Oxenham 2014](https://pmc.ncbi.nlm.nih.gov/articles/PMC3985978/), n = 21 + 24, roved):
  - Musicians beat non-musicians on F0 difference limens (0.8 % vs 1.9 %), but not on spectral-centroid limens (4.0 % vs 5.0 %, p = .18).
  - Pitch and brightness interfere symmetrically (*documented*).
- **Reverb-time JNDs vary with the stimulus.** ISO 3382-1's 5 % comes from impulsive noise; listener expertise matters ([Treybig et al., AES Europe 2023](https://www.tu-ilmenau.de/universitaet/fakultaeten/fakultaet-elektrotechnik-und-informationstechnik/profil/institute-und-fachgebiete/fachgebiet-elektronische-medientechnik/team/lukas-treybig)).

**For enth:**

- Hold pitch fixed while training cutoff, as the generator's keytrack/cutoff pair rule already does.
- Use design §9's steps (cutoff 2 st, times × 1.5, levels 3 dB), and generous steps for reverb.

**Synth-parameter ear training: no controlled study exists (a gap).**

- **What there is:** a one-week NIME study (n = 20) whose users wanted to understand how parameters map to sound ([NIME](https://nime.pubpub.org/pub/adtb2zl5/release/1)); interviews showing that novices fall back on presets ([SynthScribe](https://arxiv.org/html/2312.04690v2)); and Syntorial V2's remapping of values "too subtle to hear" ([V2](https://www.syntorial.com/whats-new-in-v2/)).
- **The case rests on three things:**
  - timbre needs its own training (Allen & Oxenham);
  - targeted spectral training works (Kim 2013 and 2015);
  - learning generalizes only when training varies ([Wright, Wilson & Sabin 2010](https://pmc.ncbi.nlm.nih.gov/articles/PMC3089592)).
- **So:** rove the patch, note and wave once the basics are met.

## 3. Adaptive difficulty and session design

**Staircases.**

| Method | Target | Use |
| --- | --- | --- |
| 2-down/1-up ([Levitt 1971](https://doi.org/10.1121/1.1912375)) | 70.7 % | Recreate's auto difficulty, unchanged |
| 3-down/1-up | 79.4 % | **The R4 default** for training and measurement |
| Weighted up-down ([Kaernbach 1991](https://doi.org/10.3758/Bf03214307)) | p = up/(up+down); 4:1 → 80 %, 5.67:1 → 85 % | Training at an in-between target |
| Elo for players and items ([Math Garden, 2011](https://doi.org/10.1016/j.compedu.2011.02.003)) | Online calibration | **No crowd data** (no telemetry); a local per-skill rating is fine |

- **The success target:**
  - Wilson 2019's 85 % is theory for machine learners.
  - Jansen 2013 (n = 207, 6 weeks): a higher preset success rate meant more practice and larger gains.
  - Letting children choose their own rate gave no benefit ([Jansen et al. 2016](https://dare.uva.nl/id/04551dd7-7f4e-481f-a544-dac96a5a4bd3)).
  - **So:** about 80 %, and no difficulty slider beyond Recreate's.
- **Even impossible items produce some learning** (Amitay et al. 2006, via [Micheyl 2009](https://mcdermottlab.mit.edu/papers/Micheyl_etal_2009_identical_stimuli.pdf)). The 2-JND floor (choice_audibility) is there for morale and fairness.
- **Difficulty is measured.** SoundGym's "pot luck" came from random settings. enth can state each item's distance in JND units under `sound/1`, or in parameter steps where `sound/1` is deaf, and choose values per level from that distance.
- **What a staircase may move**, one at a time: the step in JND units, the number of categories, their similarity, the context (sine → generator levels 3–5), roving (note, wave, velocity) and memory load.

**Spacing across games.**

- **Skill keys:** `module.param` (the existing Review keys) and `ear.*` (`ear.interval.m3`, `ear.chord.min`, `ear.band.1k`). The progress file keeps unknown keys, so this is additive.
- **Bursts:**
  - A burst is 5 items on one skill in any game.
  - 4 or more right moves the box up, 3 holds it, and 2 or fewer moves it down.
  - Single answers in 2–4 option items are 25–50 % guessable.
- **Due skills choose the workout's games.** A due `svf.cutoff` comes back as Spot the change one day, Dial it in another, and Frequency finder later: varied retrieval, which is Wright 2010's generalization point.
- **Interleaving** ([Carvalho & Goldstone 2014](https://doi.org/10.3389/fpsyg.2014.00936)): introduce each category alone, then mix similar ones (m3/M3, attack/decay, room/decay). One line of copy explains why mixing feels harder.
- **This closes learn.md's open item** "ear questions as Review items": a due skill without a Recreate step plays as a burst.

**Sessions, streaks and dark patterns.**

- **Dose:** the evidence conflicts.
  - [Molloy et al. 2012](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0036929): 100 trials a day (about 8 min) learned fastest per trial, gains were retained at 4–6 weeks, and the authors recommend short sessions.
  - Wright & Sabin 2007 (Exp. Brain Res. 180; [abstract](https://independent.academia.edu/WrightBeverly)): frequency discrimination needed more than 360 trials a day; temporal-interval discrimination did not.
- **Default workout:** 3 games × 12–20 items, about 10–15 min, ending in "done for today", with "one more" optional.
- **After each answer:** replay A, B and the right answer. Feedback talks about the sound ("the cutoff moved 3 semitones down"), never the person.
- **Progress:** thresholds per skill over time, plus a month calendar of practised days that is cumulative and can't be "lost".
- **Not in R4:**
  - streak counters: broken streaks cause drop-off, worse when self-blamed (Silverman & Barasch);
  - loss messages, daily quests and "playing by appointment", grinding, currency ([Zagal et al. 2013](https://research.chalmers.se/en/publication/177148));
  - leaderboards: Hanus & Fox's leaderboard course lowered motivation and exam scores.
- **After a break:** a warm-up one level below the last level reached, and no "you missed N days" message ([Barasch, CU 2023](https://www.colorado.edu/business/news/2023/04/20/research-streaks-marketing-tech-barasch)).

## 4. Ranked R4 games

Ranked by fit to sound design, evidence, reuse of built parts, and reliable deterministic scoring.

| # | Game | Round | Ladder (the staircase moves within a level) | Scoring | Session | Reuses |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **Spot the change** | A, then B with one control moved; pick it from 2–6 labelled knobs; replay with the answer | L1: 2 far options (filter vs envelope), ≥6 JND, Init saw · L2–3: 3–4 options, same module · L4: 4 JND · L5: generator level 2–3 bases · L6: 6 options, 3 JND · L7: roving note, preset bases · L8: 2 JND | Right or wrong; level; bursts | 16 items, about 4 min | One-item `Target::Draw`; Choose; `choice_audibility`; the observability mask; Review keys. Supplies GR2's dropped "what changed?" items |
| 2 | **Dial it in** | B differs by one knob; turn it to match; Submit | The pass tolerance narrows from 4 to 1.5 JND; the parameter pool widens by tier; two knobs from L6 | Error in JND units; rings | 6–8 rounds, about 5 min | A constrained Recreate round (`Constraints.editable`, `structure/1`, the coach); `closest` and a sibling redo after a fail |
| 3 | **Ear check** | 3-interval odd one out on one parameter | 3-down/1-up, 40 trials or 8 reversals, starting at 8 JND | **Threshold in st, ms, ct or dB** | 3–4 min per skill | Draw value sets; design §9's staircase code; placement; the GR4 instrument |
| 4 | **Frequency finder** | Noise or saw through svf bp or a resonant lp peak; pick the band, later place it on a log slider. After S2: EQ boost or cut (Quiztones tiers) | Bands 2 → 10 (octaves, then thirds); Q and gain fall; noise → saw → patch → chord | Right or wrong, or error in octaves | 15 items, about 4 min | Draw on `svf.cutoff` band centres; Choose; later `eq.*` |
| 5 | **Interval stacks** | A two-oscillator stack or a two-note phrase; name the interval, or play it back on MIDI | P1 vs P8, then add one interval at a time (P5, P4, M3, m3…); harmonic, then melodic; `transpose` and wave roving | Per-interval accuracy | 20 items, about 4 min | `osc.semi` draws; static melodic phrases; `osc.semi` and `ear.interval.*` |
| 6 | **Chord quality** | A chord on a pad; name its quality | maj/min (bimodal warning) → dim/aug → sus → 7ths → inversions | Right or wrong | 15 items | Phrase 4 plus new static chord phrases, `transpose` |
| 7 | **Rhythm pattern** | A one-bar pattern; pick 1 of 4, then mark it on a 16-step grid | Syncopation index low → high; density; swing | Steps right | 10 items | Static phrases (`start_ms`); no latency |
| 8 | Progressions and functional | Cadence, then a degree or progression | Functional Ear Trainer's order | Right or wrong | 15 items | **Needs seeded phrase draws** (axis 2) |
| 9 | Note naming (opt-in) | One note within a deadline | Wong 2025's ladder | Pitches at 90 % or better | ≤10 min | Single-note phrase and `transpose`; honest copy |
| 10 | Tap-along | Tap the pattern | Window; syncopation | Asynchrony | 10 items | Latency and timing work (axis 2) |

**Notes on the top five:**

- **Spot the change:**
  - Options are observable controls only, labelled with the editor's own knob names.
  - Pairs `sound/1` can't hear (attacks under about 60 ms, sustain, slow beating, pluck resonance) use parameter steps and are marked *not measured*.

- **Dial it in** is the hear-to-knob mapping that sound design is. At L1–3 the rings show only after Submit (R2: no live meter during Do).
- **Ear check** gives a number that means something ("decay changes of × 1.2"). Run it for placement and every 2 weeks, not daily.
- **Frequency finder** starts now on the svf ("where's the resonance?") and becomes the roadmap's EQ A/B once S2's EQ lands.
- **Interval stacks:** the harmonic form *is* the synth parameter `osc.semi`. Playing the answer back on MIDI is EarMaster's singback idea for a plugin; axis 2 checks MIDI input in the game view.

**Determinism and R5.**

- A practice session is (game, seed, settings) plus the answer log, and its items come from keyed streams `derive(seed, fnv("game/item#/level"))`.
- Adaptive sessions diverge between players, so **duels use a fixed ladder**: item *i*'s level depends only on *i*, and both players hear the same items.
- Anyone holding the code can compute the answers. That honest limit is axis 3's.

## 5. Measuring that R4 works: GR4 proposal

**Gate GR4** (proposed; the owner sets the criteria): the games are deterministic, fair and solvable, and you and your friends hear finer differences on trained skills than on held-out ones.

**Mechanical half (CI):**

- **Determinism:** 1,000 items per game regenerate identical target hashes on the six golden targets.
- **Audibility:** every option pair is at least 2 JND apart under `sound/1` (at least 6 at L1); items scored on parameter steps are listed separately.
- **Solvable:** an ideal-observer bot (picks the option nearest by `sound/1`) scores ≥95 % at the top level.
- **No leaks:** bias bots (always option 1, always the loudest, always the longest) stay at chance ±3 % over 10k items. Options are level-matched within 0.5 LU unless level is the cue.
- **Staircases:** simulated listeners with known Weibull thresholds get a 40-trial estimate within one step of the truth in at least 90 % of runs, and the training rule holds 75–85 % success.
- **Coverage:** every observable Basic-tier P1 parameter is in game 1 or 2, and bursts move Review boxes (proptests, as REV has).

**Human half (local, per person, as GR2):**

| Part | What | Time |
| --- | --- | --- |
| Retest pilot (owner) | Ear check on 4 skills twice, a day apart; "real change" = more than twice the retest spread | 2 × 12 min |
| Pre-test | Ear check on 4 skills: 2 to train, 2 held out, counterbalanced (A trains cutoff and decay and holds out room and detune; B the reverse), plus an interval block | 15 min |
| Training | 2 weeks, at least 6 workouts of 15 min or less, trained skills only | about 1.5 h |
| Post-test, delayed (optional, 2–4 weeks) | The same checks with new seeds | 15 min each |
| Transfer (exploratory) | GR2 family codes, trained vs held-out families | 10 min |
| Debrief | Fun 1–5; too easy or too hard; "name something you now hear" | 3 min |

**Why held-out skills:** test practice alone improves auditory thresholds (Amitay 2006), so only improvement *relative to held-out skills* counts.

**Proposed criteria:**

- The median participant improves both trained thresholds beyond the retest bar, and by more than the held-out skills improve.
- No trained skill gets worse on average.
- The owner and at least half the friends play at least one workout beyond the required ones (from the local log, sent by choice).
- Everyone names something specific they now hear.

With N = 3–6, describe the results rather than lean on p-values. Far transfer to Recreate is reported, not gated, because it is expected to be weak (Wright 2010).

## 6. Risks and gaps

- **Transfer:** no controlled evidence shows that synth-parameter games transfer to sound design. GR4's held-out design and the Recreate row produce enth's own evidence.
- **`sound/1`'s deaf spots:** use parameter steps, mark those items *not measured*, and give them a listening review.
- **Monitors:** nothing under 80 Hz below L5, and level matching (SoundGym's monitoring complaint).
- **Pitch–timbre confusion:** hold the note fixed while training brightness.
- **Hard skills:** chord perception is bimodal, intervals are slow and AP training sheds most players. Use honest copy and never gate content on these games.
- **Repetitiveness:** many skills across 4 vehicles, roving, and workouts with an end.
- **No crowd calibration:** use measured distances and simulated observers.
- **Gaps in this research:**
  - Reddit was unreachable from search (HTTP 400, "domain not accessible"). HN was read through its Algolia API (one call per thread), and Gearspace returned 403.
  - Read only at abstract or secondary level: Little 2019 (PMC served a browser check), Wong 2021, Chubb, Fitch & Rosenfeld, Rhythm Workers, Olive, Hanus & Fox, Silverman & Barasch, Amitay, Wright & Sabin, Repp.
  - Not found: Syntorial's quiz format, SoundGym's current games and prices, and any controlled study of functional or synth ear training.
