# 01 — Pedagogy and curriculum (R2 Learn)

- **Date:** 2026-10-21. Sub-researcher 01 of the R2 Learn session ([brief](brief.md)).
- **Read in the repo:** AGENTS.md, research-process.md, roadmap.md (R2), design.md §1 and §6–§9, dsp/hints.md (the coach), editor.md (Overlay, the Recreate UI), the catalogue (`crates/enth-patch/src/catalogue/`), `enth-challenge`'s model, the 22 preset candidates, and R1 report 01 of 2026-10-20 (Syntorial's challenge mechanics, not repeated here).
- **Marks:** *measured* (I counted or ran it), *documented* (paper, manual, vendor page), *anecdotal* (forums, reviews). Papers were mostly checked at abstract level (see Gaps). All links were checked on 2026-10-21. Scratch: `~/code/enth-scratch/2026-10-21-r2/01/notes.md`.

## TL;DR

1. **Lesson shape: Hear → Name → Try → Do → Check, then Review in later sessions.** This changes the brief's "explain, hear, do, check": hearing a contrast before the explanation has better support than explaining first.
2. **The curriculum has 36 core lessons in 9 units, plus 6 sound-design studies.** Its order is the Recreate generator's (oscillator, filter, envelopes, modulation, effects), so real Recreate rounds at the matching level close each unit. All 54 parameters of the 10 P1 module kinds are covered (*measured*: 24 Basic, 19 Intermediate, 11 Advanced).
3. **A lesson's challenges are constrained Recreate rounds** with `structure/1`, `sound/1` and the existing 4-level coach. Hints get fewer as the lessons advance. After a Reveal or a level-4 hint, a sibling challenge must be solved before the lesson counts.
4. **"Deconstruct this preset" lessons come from ablation.** Remove each element, measure what the sound loses, drop what is inaudible, then build the preset back up in signal-flow order, with "with / without" A/B at every step.
5. **GR2 uses a 12-lesson "tour" (one lesson per module).** Parallel-form pre- and post-tests are made from challenge codes. Module families are taught at staggered times, and each participant answers "name what you learned". Everything stays local, and results are reported per person.
6. **Not recommended:** streaks (evidence of engagement, not learning), a spectrum display during Do (players match by eye) and the `fsrs` crate (Leitner boxes are enough).

## 1. What the evidence says

| Principle | Evidence (*documented* unless marked) | For enth |
| --- | --- | --- |
| **Contrast before telling** | Students who first worked with contrasting cases learned more from a later lecture ([Schwartz & Bransford 1998](https://www.wright.edu/sites/www.wright.edu/files/uploads/2017/Feb/event/Schwartz_Bransford_1998_TimeForTelling.pdf)). Variation theory: an aspect is noticed only when it varies while the rest stays fixed; contrast comes first, then generalization ([Marton 2015](https://www.taylorfrancis.com/books/9780415739139); summary in [ZDM 2017](https://link.springer.com/10.1007/s11858-017-0858-4)) | Hear first: A/B pairs that change only the new control, on 2 or more base sounds. Explain after |
| **Worked examples, then fading** | Examples help early; fading them into problems helps, backward and performance-adaptive fading best, mainly near transfer ([Renkl & Atkinson 2003](https://asu.elsevierpure.com/en/publications/structuring-the-transition-from-example-study-to-problem-solving--2)). The same support can hinder experts: the expertise reversal effect ([Kalyuga et al. 2003](https://ro.uow.edu.au/edupapers/136)) | Generous hints early, fewer later; a placement test lets experienced players skip |
| **Retrieval and spacing** | Practice testing and distributed practice are the two "high utility" techniques; interleaving and self-explanation are moderate ([Dunlosky et al. 2013](https://www.psychologicalscience.org/news/releases/which-study-strategies-make-the-grade.html)). Duolingo's half-life regression cut recall-prediction error by 45 % or more; its A/B test showed +9.5 % practice retention, a company figure ([Settles & Meeder 2016](https://aclanthology.org/P16-1174/), [blog](https://blog.duolingo.com/how-we-learn-how-you-learn)) | A Review queue of short rounds and quiz items, spaced and mixed |
| **Interleaving** | Mixing 12 painters' works beat blocking them for learning their styles, yet learners believed blocking worked better (Kornell & Bjork 2008, *Psych. Sci.* 19(6); [2014 replication](https://doaj.org/article/7705cbc33f964520a04180b3fac2f278)) | Quiz items mix categories. Learners will *feel* they learn less |
| **Perceptual learning** | Many short classification trials with feedback build fluency ([Kellman et al. 2010](https://pmc.ncbi.nlm.nih.gov/articles/PMC6124488)). In auditory learning, generalization to untrained conditions lags behind learning ([Wright, Wilson & Sabin 2010](https://pmc.ncbi.nlm.nih.gov/articles/PMC3089592), *J. Neurosci.*) | "What changed?" items of 5–10 s; vary the note, the wave and the base patch |
| **Target weaknesses** | Personalised EQ training aimed at each trainee's weak spots beat conventional training over one month, with only 8 subjects ([Kim et al. 2013](https://www.jstage.jst.go.jp/article/ast/34/6/34_E1312/_article)) | Review picks the weakest skills, read from Recreate's rings |
| **Feedback can hurt** | Feedback helps on average (d = 0.41), but more than a third of feedback interventions lowered performance ([Kluger & DeNisi 1996](https://mrbartonmaths.com/resourcesnew/8.%20Research/Marking%20and%20Feedback/The%20effects%20of%20feedback%20interventions.pdf)). Good feedback is non-judgemental, timely and specific ([Shute 2008](https://myweb.fsu.edu/vshute/pdf/shute%202008_b.pdf)) | Talk about the sound and the knob ("darker than the target"), never the person |
| **Hint ladders** | Tutoring systems end with a bottom-out hint. Clicking straight through to it was 36 % of actions in one tutor unit ([LearnLab](https://learnlab.org/research/wiki/index.php/Help_abuse)), yet bottom-out hints teach when studied ([Shih et al. 2008](https://www.cmu.edu/dietrich/philosophy/docs/scheines/Shih_Koedinger_Scheines_2008_EDM.pdf)) | The coach's 4 levels are this ladder. Levels 3–4 open after a first Submit; a level-4 hint or a Reveal requires a sibling redo |
| **Streaks** | +0.38 % daily active learners from streak freezes, a company figure ([Duolingo](https://blog.duolingo.com/how-duolingo-streak-builds-habit)); limited causal evidence ([Northwestern WP 2026](https://www.ipr.northwestern.edu/documents/working-papers/2026/wp-26-05.pdf)) | Skip them |
| **Synth users find learning hard** | Kreković's survey ([SMC 2019](https://www.smc2019.uma.es/articles/S4/S4_03_SMC2019_paper.pdf)): n = 122, 71 % with more than 10 years' experience. Median answer "agree" that synths are "difficult and not intuitive to learn". Most-wanted help: a clearer UI (58.1 %), then tutorials (25.8 %). Sound matching was rated "Helpful" and random patches by category "Slightly helpful" | Lessons belong inside the instrument; Recreate is a welcome core activity |

Timbre words: three dimensions recur across languages, luminance, texture and mass ([Zacharakis et al. 2014](https://timbreandorchestration.org/bibliographies/mpcl/verbalization)). The coach's words (brighter, plucky, thicker) fit them; keep lesson text in those terms.

## 2. Prior art

| Product | Shape | Take | Avoid |
| --- | --- | --- | --- |
| **Syntorial** (2013-08-27; v2.0 in July 2023) | 199 lessons, 129 challenges, 39 quizzes. Each lesson is a 2–6 min video, then a hidden-patch challenge. 64 parameters arrive one by one, with snapped values ([Wikipedia](https://en.wikipedia.org/wiki/Syntorial), [Keyboard](https://www.pluginboutique.com/articles/531-Syntorial-Review-at-Keyboard-Mag-com)). V2 added an orange "near" tier, dropped values "too subtle to hear", and added a Randomizer to "review an old challenge" ([V2](https://syntorial.com/whats-new-in-v2/)) | One control at a time; by ear; well-separated early targets; the Randomizer as review | A video per lesson. "Later tests very difficult… rarely use it now" ([Elektronauts 2015](https://elektronauts.com/t/how-to-learn-synthesis/8758), *anecdotal*). No spaced review |
| **Ableton Learning Synths** ([2019-06-25](https://www.ableton.com/en/blog/learn-synthesis-in-your-browser/), [update 2022-04-28](https://mixdownmag.com.au/news/ableton-updates-their-interactive-learning-synths-website/)) | Chapters on a 2-oscillator monosynth, breakdowns of classic sounds, a Playground reachable from every page | Each page shows only its controls; lessons can be skipped; "Open in Playground" | No checks, so no evidence anyone learned |
| **Synth Basics sound match** ([page](https://synthbasics.com/sound-match), undated) | 2, 4 or any knobs free; 20 rounds from "Brightness" to "The boss"; a live meter; a coach naming the furthest knob; per-round hints saying *what moved and what to listen for* | Listening cues written by a person | A live meter by default |
| **Synth Secrets** (SOS, 63 parts, 1999–2004, [series](https://www.soundonsound.com/series/synth-secrets-sound-sound)) | Theory first, then imitating instruments by analysing, then patching | The analyse-then-build arc for the studies; link it for depth | Long essays |
| **SoundGym** ([SOS](https://www.soundonsound.com/node/4923945), [Gearnews 2017](https://www.gearnews.com/soundgym-2-0-revamps-audio-ear-training/)) | Short games from a pool, a daily workout, adaptive levels | Short identification drills | The reviewer doubted it stays engaging past a month (*anecdotal*) |
| **Melodics / Yousician** ([scoring, 2026-05-14](https://support.melodics.com/en/articles/15092017-lesson-scoring)) | Lessons in steps, scored attempts, stars, loop and slow-down practice, a guided path | A one-click "try again" | Uncapped scores, streaks |
| **Arturia Pigments** in-app tutorials ([review 2025-02](https://magneticmag.com/2025/02/arturia-pigments-6-review/)) | A Quick Tour plus tutorials per engine and modulation, inside the plugin | Lessons inside a plugin are accepted | Tutorials that never check skill |
| **Forum practice** ([KVR 2023-08](https://www.kvraudio.com/forum/viewtopic.php?p=8732934), [Elektronauts 2014](https://elektronauts.com/t/how-to-learn-synthesis/8758), *anecdotal*) | "Deconstructing and recreating your favourite synth presets is a great way to learn"; pick a synth that can "bypass and enable parts"; turn off FX and modulation first; "tedious" on complex synths | Supports deconstruct lessons; enth's enable switches and FX bypass make it cheap | Unguided deconstruction; automate the order |

## 3. The lesson shape

A lesson takes 5–10 min and teaches **one new control or idea**.

| Step | What happens | enth parts | Time |
| --- | --- | --- | --- |
| **1. Hear** | 2–4 A/B pairs that change only the new control, on at least 2 base sounds or notes; one "which is brighter?" item; a written listening cue | Live deterministic renders from the pack's patches (no audio files needed); level-matched A/B | 30–60 s |
| **2. Name** | At most about 80 words in perceptual terms, with `learn.desc` as the canonical sentence and an optional link for depth. Visuals only for modulator shapes (as Syntorial V2 does) | `ParamSpec.learn`, the scope | 30 s |
| **3. Try** | Only the new control is shown (the rest hidden); a goal such as "make it darker than this", checked live | `Target::Expectations`, `enth-solver::hint` | 1 min |
| **4. Do** | 2–3 hidden-target challenges. The new and earlier controls are editable; earlier ones dimmed. Early lessons use 3–5 target values at least 2 just-noticeable steps apart; later lessons use continuous values | A constrained Recreate round, the scorers, the rings, the coach | 2–5 min |
| **5. Check** | Mastery = 2 of 3 passed with no level-4 hint or Reveal, plus 1–2 mixed "what changed?" items, a "new to me / knew it" tap and an optional "what did you learn?" | The round log, choice items | 30 s |
| **6. Review** (later) | 3–5 items from passed lessons, by Leitner box and weakest skill; no hints on the first try | Seeded rounds, the ring history | 3–5 min |

**Rules:**

- **Scaffolds fade within a unit.** Controls go hidden, then dimmed, then normal, and each unit's last lesson uses the full editor ("never graduate out").
- **Hints by tier** (via `hint_budget`). Basic: all 4 levels. Intermediate: levels 3–4 open after a first Submit. Advanced, Review and unit tests: no hint on the first attempt. Any hint marks the attempt assisted.
- **Redo after a Reveal or level 4:** a sibling challenge with the same parameters and a new seed, so an example is always followed by a problem.
- **No spectrum in Do or Review.** If used, it is an assist that marks the round assisted, because Syntorial's premise is "all by ear" ([syntorial.com](https://www.syntorial.com/)). It stays available in Hear and Name.

## 4. Curriculum

The units follow the generator's levels (design §9). A **unit test = 3 Recreate rounds at that level**. Tiers come from the catalogue.

| Unit | Lessons (new controls: contrast heard) |
| --- | --- |
| **U0** | 0.1 How a lesson works (Target, Mine, A/B, Submit, Hint, `master.level`: the same patch at 0 and −12 dB, then level-matched) |
| **U1 Sources** (level 1) | 1.1 Waveforms (`osc.wave`: sine, triangle, saw, pulse; level-1 rounds) · 1.2 Octaves and intervals (`semi`: ±12, +7) · 1.3 Pulse width (`pw`: 50, 25, 10 %) · 1.4 Two oscillators (`level` as a ratio, `fine`: 0 or 7 ct beating) · 1.5 Noise (`noise.color`, `level`) · 1.6 Hard sync (`sync`: off or on at +7 and +19 st) · 1.7 Wavetables (`table`, `position`) |
| **U2 Filter** (level 2) | 2.1 Low-pass cutoff (`svf.cutoff`; "darker than X") · 2.2 Resonance (10, 50, 90 %) · 2.3 Modes (lp, hp, bp, notch; "which mode?" items) · 2.4 Key tracking (an octave pair; why it is confusable with cutoff on one note) · 2.5 Drive (0, 12, 24 dB) |
| **U3 Amp envelope** (level 3) | 3.1 Attack and release (5 or 300 ms) · 3.2 Decay and sustain (organ or pluck) · 3.3 Velocity (a two-velocity phrase) |
| **U4 Envelopes as modulators** | 4.1 Routes and depth (env 2 → cutoff, with or without) · 4.2 Filter-envelope shapes (brass swell or pluck; level-3 rounds) · 4.3 Pitch envelope (env → `semi`: a kick) · 4.4 Negative depth |
| **U5 LFOs** (level 4) | 5.1 Vibrato (`rate`, route to pitch) · 5.2 Shapes and tremolo (sine, square, S&H) · 5.3 Wobble and sweeps (→ cutoff) · 5.4 PWM and table scanning · 5.5 Polarity and scope (a global LFO on the reverb) · 5.6 Retrigger or free (`phase_mode`, `phase`, `seed`; items) |
| **U6 Voices and playing** | 6.1 Poly, mono, legato (`mode`, `polyphony`) · 6.2 Glide (`glide`, `glide_mode`; a legato pair) · 6.3 Playing sources (velocity, key, mod wheel; `pan`) |
| **U7 Effects** (level 5) | 7.1 Reverb space (`wet`, `dry`, `room`, `decay`) · 7.2 Reverb tone (the cuts, the damping, `early`, `detune`) · 7.3 Compressor level (`threshold`, `ratio`, `makeup`) · 7.4 Compressor time (`attack`, `release`, `knee`, `mix`: parallel) |
| **U8 Macros and finishing** | 8.1 Macros (± depth to several targets; "at macro 0 X, at 1 Y") · 8.2 Gain staging (`master.level`, clipping) · 8.3 Phase and seeds (oscillator `phase_mode`, `phase`, `seed`; items) |
| **U9 Studies** | 9.1 Bass · 9.2 Plucks and keys · 9.3 Pads · 9.4 Leads and imitation (Breath Flute) · 9.5 Percussion · 9.6 FX and texture: built from preset candidates as in §6, with text written by hand, ending in "make a variation" |

**Coverage** (*measured* against the catalogue): every parameter of osc (11), noise (2), svf (5), adsr (5), lfo (7), reverb (10), comp (7), voice (5), master and macro, and every modulation source (envelope, LFO, velocity, key, mod wheel, macro), appears in at least one lesson. Studies need only a patch, so all 22 candidates are usable whatever the owner picks.

**Two tracks.** The **tour** is 12 lessons, about 1.5 h, one per module: 1.1, 1.4, 1.5, 2.1, 2.5, 3.2, 4.1, 5.1, 6.2, 7.1, 7.3, 8.1. It is the GR2 track for friends. The **full path** is 36 lessons plus studies, about 5–7 h. Its placement test is 2 rounds per unit test, and passing skips the unit. That answers the expertise reversal effect and gets the owner to material that is new to them (U5.5–5.6, U7.2/7.4, U8, the studies).

## 5. Reusing Recreate, the coach and nearest-solution feedback

- **Challenge types.** Today there is only `Target::Seed`, `AnswerKind::Patch` and an empty `Constraints` (*measured*, `enth-challenge/src/model.rs`), and `RandomSpec` draws per category, not per parameter. Lessons need four additive pieces (the enums are `non_exhaustive`):
  - a **lesson target**: base patch, parameter list, value sets and seed, drawn in parameter space with §9's keyed streams and just-noticeable steps;
  - `Target::Expectations`;
  - `AnswerKind::Choice`;
  - `Constraints.editable`, which the Overlay reads.

  Axis 3 owns the types.
- **The coach as it is** forms the hint ladder from §1, and its "no digit before level 4" rule already matches it. Two additions: (a) an **authored listening cue** shown with level 1 (Synth Basics style); (b) carry `aspect`, `module` and `param` into `CoachHint` so the Overlay can highlight the knob (editor.md, "left for later").
- **Nearest-solution feedback.** For open goals, `enth-solver::hint` already finds the smallest verified move that passes: show it after a Submit as words and rings ("2 moves away: shorter decay, lower sustain"), with the exact values only at level 4. For hidden targets, `structure/1` is already symmetry-aware. A **"Show me the closest passing patch"** button would run CMA-ES from the player's own patch (`enth match`, design §7 layer 3). It is a worked example that respects the player's route, so it triggers a redo like a Reveal.
- **The game is the review engine.** The Good, Near and Bad rings of every Recreate round give a free signal for each skill (a module kind and parameter key, e.g. `svf.cutoff`). Keep a Leitner box per skill in a local progress file, and let Review serve the weakest due skills. That is the weakness-targeted drilling of Kim et al., with no model to train. `fsrs` (BSD-3-Clause per its [LICENSE](https://github.com/open-spaced-repetition/fsrs-rs/blob/main/LICENSE), v6.6.2 updated 2026-08-29, *measured*) pulls in ndarray, rayon and rand: too heavy for about 50 skills.
- **Unit tests and placement** use existing settings: level n, Musical 100, the staircase off.

## 6. "Deconstruct this preset" (generated)

1. **Elements:** each enabled module, each route, and each group of parameters that differ from Init.
2. **Ablation:** render the preset without each element and measure the `sound/1` distance plus the coach's descriptor changes (brightness, attack, beating, tremolo, sweep). 10–30 renders take about 1–4 s serially at design §7's 7–8 ms per evaluation (an estimate, not measured on presets).
3. **Prune:** an element below one just-noticeable step becomes "fine detail" and gets no step.
4. **Order:** by signal flow (source, filter, amp env, modulators, effects, macros), then by contribution. Step *k*'s target is Init plus elements 1 to *k*, measured in that context, because elements interact (resonance is silent until the cutoff moves).
5. **Each step:** Hear the target with and without the element; Do rebuilds it with only that element editable, scored by `structure/1` on those parameters plus a `sound/1` pass; the coach is on.
6. **Text:** a template from `learn.desc` and the measured changes ("this route makes the start brighter, then darker: a filter envelope"). Factory studies get human-edited text (or LLM text checked by the solver, axis 3); user presets get "auto" lessons.
7. **Close:** "What disappeared?" (the full preset minus one element, pick 1 of 4), then a variation goal ("keep it plucky, make it brighter") checked by expectations derived from the preset.

At most about 8 steps per lesson. Prototype on Acid Snap and Glass Pluck before writing studies (*no evidence yet* on step quality).

## 7. Measuring GR2 with a handful of friends

Coverage is automated (axis 3's lesson-check). For "learn something new":

| Part | What | Time |
| --- | --- | --- |
| Intake | Years with synths, which synths, confidence 1–5 per module family | 3 min |
| Pre-test (form A or B, counterbalanced) | 10 challenge codes, one per family, at fixed settings: pass within 3 min without hints, plus `structure %`; 12 mixed "what changed?" items | 25 min |
| Learning | The tour (friends) or the full path from placement (owner), over 1–2 weeks | 1.5 h |
| Mid-test | 5 items over all families, after the tour's first third | 8 min |
| Post-test | The other form: same specs, other seeds | 25 min |
| Delayed (optional) | 5 items a week later | 8 min |
| Debrief | "Name up to three things you learned"; too easy or too hard | 5 min |

- **Design.** Parallel forms rule out memorised targets. Staggering is a **multiple baseline across skills**: a family should improve only after its lesson, which separates the lessons' effect from practice on the tests. WWC's single-case standard asks for at least 3 demonstrations at different times ([WWC](https://ies.ed.gov/ncee/WWC/Docs/ReferenceResources/wwc_scd.pdf)); 10 families give several per person.
- **Proposed pass criteria (the owner decides).**
  - Every participant names a specific new thing in at least 3 lessons, coded by an agent against each lesson's objective.
  - The median participant improves on at least 3 of 10 families, and no family gets worse on average.
  - The owner names something new and passes a post-test item they failed before.

  Report per person. If all 5 of N = 5 improve, a one-sided sign test gives p ≈ 0.03; describe it, don't lean on it.
- **Privacy.** The progress file and round log stay local; participants send them by choice (as GR1 already works).
- **Threats.** Ceiling effects for experienced friends (handled by placement and studies), and politeness in self-reports (the pre/post numbers carry the gate). Friends' time is about 2.5 h on top of GR1. Pilot with the owner first. Agent evals through the CLI show that each lesson can be solved and how hard it is, not that anyone learns.

## 8. Risks

| Risk | Mitigation |
| --- | --- |
| Content cost: 36 × (Hear pairs, targets, cue, 80 words) | Agents draft patches and pairs and the solver checks them; a person reads the words |
| Some controls are hard to hear (damping, early reflections, knee, phase) | Exaggerated Hear pairs; "what changed?" items rather than recreation; §9's minimum step |
| Lesson pane width (320 px fits only when unused modules are hidden) | Try hides and Do dims, as above |
| Interleaving and redos feel slower | One line of text says why |
| New challenge types are persisted | Additive and versioned (axis 3) |

## 9. Gaps

- **No peer-reviewed evaluation of any synthesis course** (Syntorial, Learning Synths, Synth Basics). The design rests on general learning science plus very small ear-training studies (Kim et al.: 8 subjects).
- **Reddit's search JSON was blocked** (an HTML challenge from www and old.reddit, 2026-10-21), so the first-hand reports come from KVR and Elektronauts only.
- **Learning Synths' pages need JavaScript**; its structure comes from press coverage.
- **Read at abstract level only:** Marton, Kalyuga, Kornell & Bjork, Kim et al., Dunlosky and Zacharakis. **Not retrieved:** Olive 2001 (Harman's "How to Listen") and Wright & Zhang 2009.
- **Not measured yet:** ablation cost and text quality on enth presets, and how long friends stay engaged.

## 10. Recommendations

1. **Lesson shape: Hear → Name → Try → Do → Check → Review**, with §3's rules: one idea per lesson; scaffolds fade (hidden, dimmed, normal); hints by tier; a redo after a Reveal; no spectrum during Do.
2. **Build the 12-lesson tour first**, since it is GR2's coverage and the friends' track. Then grow it to the 36-lesson path and the studies.
3. **Make unit tests and placement out of Recreate rounds** at generator levels 1–5; the curriculum follows the generator's order.
4. **Ask axis 3 for four additive challenge pieces:** the lesson target, `Target::Expectations`, `AnswerKind::Choice` and `Constraints.editable`.
5. **Extend the coach:** an authored listening cue with level 1; levels 3–4 open after a first Submit from Intermediate on; the structured fields passed into `CoachHint` for highlighting.
6. **Add "Show me the closest passing patch"** (CMA-ES from the player's patch) after a failed Submit, followed by a redo.
7. **Track skills locally with Leitner boxes per module and parameter, fed by Recreate rings.** No streaks, no `fsrs`.
8. **Generate deconstruct lessons by ablation, ordered by signal flow** (§6). Prototype on two candidates; give factory studies human-edited text.
9. **Run GR2 as in §7:** parallel forms, a staggered mid-test, "name what you learned", all local. The owner sets the criteria.
10. **Keep text short and perceptual**, with `learn.desc` as the canonical sentence and Synth Secrets links for depth.
