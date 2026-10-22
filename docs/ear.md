# Ear training (R4): games, drills and workouts

R4 trains the ear that sound design needs: which knob moved, how far, where a band sits, and then the music side (intervals, chords, rhythm). Every game is built from what exists: a stimulus is an offline render played through the Target ▶ slot, an answer is a label, a value, a patch or taps, and every skill feeds Review's boxes. This page is the design of record; the lane plan is [phase-4.md](phase-4.md), duels on these games are in [duels.md](duels.md), and the evidence is the [R4/R5 research session](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md) (reports 01 pedagogy and game design, 02 architecture and the lane plan, 03 duels; the summary wins where it corrects a report).

**Status (2026-10-22): planned; nothing is built.** Every decision here is provisional until you steer; work proceeds on the defaults meanwhile. MUS, CV3, BQ, DRL, ECK, EUI and TAP add their sections as they land, and INT4 keeps this status line ([phase-4.md](phase-4.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| Theory tables, rhythm, ear generator v1 | `enth-music` (L1, new) | MUS; G1–G3 the ladders | Planned (F0e, F1e) |
| Challenge v3: `Target::Ear`, choice, value and tap answers, ear rendering, timbres, scorers, the ear-challenge code | `enth-challenge` (L4) | CV3 | Planned (F1e; [challenge.md](challenge.md#challenge-v3-planned-lane-cv3-r4)) |
| The parameter games' items (Spot the change, Dial it in, Ear check) | `enth-challenge` (L4), on v2 draws | CV3; G4 the ladder | Planned |
| The EQ biquad | `enth-dsp/src/biquad.rs` (L0) | BQ | Planned |
| The drill player: staircase, ear skills in Review, `StepKind::Drill`, the `drills` field, the workout composer | `enth-learn` (L5) | DRL | Planned (F1e; [learn.md](learn.md#ear-skills-bursts-and-drills-r4-planned-lane-drl)) |
| Ear sessions | `enth-session/src/ear.rs` (L6) | SE | Planned (F1e) |
| Ear jobs, examples, the restart flag; `enth ear` | `enth-plugin/src/ear.rs`, `enth-cli/src/cmd/ear.rs` (L8) | SKE, EV4 | Planned (F2e) |
| The Ear view | `enth-ui/src/editor/ear/` (L7, feature `ear`) | EUI | Planned |
| Tap input | Fork patch 12 (`vendor/egui-baseview`), the plugin's tap path | FK4, TAP | Planned |
| `ear-check` | `tools/lesson`, `cargo xtask ear-check`, `ear.yml` | ECK | Planned |
| GR4 | `docs/r4-gates.md`, `cargo xtask gr4-report` (both planned) | INT4 | Planned |

## Why sound-design games come first

- **Pitch training does not train brightness.** Musicians beat non-musicians on pitch difference limens (0.8 % against 1.9 %) but not on spectral-centroid limens (4.0 % against 5.0 %, p = .18), and pitch and brightness interfere both ways (Allen & Oxenham 2014; report 01 §2). So interval and chord drills won't train a cutoff, envelope or reverb ear; those need games of their own.
- **They are the cheapest to build.** Each is a one-item v2 draw plus a choice, or a one-knob Recreate round: the machinery lessons already use.
- **What works elsewhere is short and varied** (SoundGym's five-game workout, TrainYourEars' "correct it by ear"). The complaints are "pot luck" difficulty from random settings, repetition and abstract sine quizzes. People quit when gains are slow and invisible: 35 of 48 starters in an absolute-pitch study (Wong 2025), 14 of 34 in a 15-week EQ course (Kim 2015). No product shows progress in physical units; R4 does.
- **Synth-parameter ear training has no controlled study.** The case rests on timbre needing its own training, targeted spectral training working (Kim 2013, 2015), and learning generalising only when training varies (Wright 2010). GR4's held-out design is enth's own evidence.

## The games

**R4a, in order** (report 01 §4, with the summary's changes: EQ now, rhythm listening-only in R4a). The staircase moves within a level.

| # | Game | Round | Ladder | Scoring | Session | Reuses |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **Spot the change** | A, then B with one control moved; pick it from 2–6 knob names; replay with the answer | L1: 2 far options (filter against envelope), at least 6 JND, Init saw · L2–3: 3–4 options in one module · L4: 4 JND · L5: generator level 2–3 bases · L6: 6 options, 3 JND · L7: a roving note, preset bases · L8: 2 JND | Right or wrong; level; bursts | 16 items, about 4 min | A one-item `Target::Draw`, a label choice, the observability mask, Review's keys. It supplies GR2's dropped "what changed?" items |
| 2 | **Dial it in** | B differs by one knob; turn it to match; Submit | The pass tolerance narrows from 4 to 1.5 JND; the parameter pool widens by tier; two knobs from L6 | Error in JND units; the rings | 6–8 rounds, about 5 min | A constrained Recreate round (`Constraints.editable`, `structure/1`, the coach); `closest` and a sibling redo after a fail |
| 3 | **Ear check** | A 3-interval odd one out on one parameter | 3-down/1-up, 40 trials or 8 reversals, starting at 8 JND | **A threshold in st, ms, ct or dB** | 3–4 min per skill | Draw value sets, the staircase, placement; GR4's instrument |
| 4 | **Frequency finder** | Noise or saw through an svf band-pass or a resonant low-pass peak: pick the band, later place it on a log slider. EQ boost or cut on the biquad | Bands 2 → 10 (octaves, then thirds); Q and gain fall; noise → saw → patch → chord | Right or wrong, or error in octaves | 15 items, about 4 min | A draw on `svf.cutoff` band centres; later S2's `eq.*` |
| 5 | **Interval stacks** | A two-oscillator stack, then a two-note phrase; name the interval | P1 against P8, then one interval at a time (P5, P4, M3, m3, …); harmonic, then melodic with a roving root; timbre roving | Accuracy per interval | 20 items, about 4 min | `osc.semi` draws; `ear.interval.*` skills |
| 6 | **Chord quality** | A chord on a pad; name its quality | maj/min (with a "some people plateau" note) → dim/aug → sus → sevenths → inversions | Right or wrong | 15 items | Phrase 4 and generated chords, `transpose` |
| 7 | **Rhythm pattern** | A one-bar pattern: pick 1 of 4, then mark it on a 16-step grid | Syncopation index low → high (Longuet-Higgins and Lee; Fitch & Rosenfeld 2007), density, swing | Steps right, judged exactly, with no input timing | 10 items | Generated phrases with their own tempo |

**R4b:** progressions (seeded phrases, Functional Ear Trainer's order); **note naming, opt-in and never in the default workout** (Wong 2025's ladder, with this copy: "Most adults who train for about 20 hours learn to name a handful of notes; a few learn all twelve; most stop sooner"); tap-back (after the TAP spike, [below](#rhythm-input)). Tap-along is deferred: it needs a calibration screen.

- **Spot the change** offers observable controls only, labelled with the editor's own knob names. Pairs `sound/1` can't hear (attacks under about 60 ms, sustain, slow beating, pluck resonance) use parameter steps, are marked *not measured* and get a listening review.
- **Dial it in** is the hear-to-knob mapping that sound design is. At L1–3 the rings show only after Submit, as in R2 (no live meter during Do).
- **Ear check** gives a number that means something ("decay changes of × 1.2"). It runs for placement and every two weeks, not daily.
- **Frequency finder** keeps its svf form after the EQ form lands, because it teaches the synth's own filter. Nothing under 80 Hz below L5, for small monitors.
- **Interval stacks:** the harmonic form *is* the synth parameter `osc.semi`. Playing the answer back on MIDI waits for the owner's OK on host MIDI ([below](#rhythm-input)).
- **Chord quality:** results are bimodal (about 70 % of listeners sit near chance on major/minor tone scrambles, Chubb 2013), so **no content is ever gated on it**, nor on intervals or note naming.
- **Parameter games stay on one-note phrases**, where `sound/1` can measure them, and hold the pitch fixed while training brightness.

## Round types

Report 02 §3. Every target is deterministic; answer keys are seeded integers, so an ear result verifies without rendering.

| Round | Answer | Target | Scored | Reuses | New |
| --- | --- | --- | --- | --- | --- |
| **Label choice** (interval, chord, note, band, which knob moved) | An index | The ear generator, or a draw | Exact | The examples slot | Label buttons, the restart flag |
| **Clip choice** | An index | The generator or a draw | Exact | `Choose` as it is | — |
| **A/B, same/different, 2AFC, odd one out** (cents, a cutoff ratio) | An index | The generator plus the staircase | Exact; replayed from the answers | Two or three locked loops | The staircase |
| **Slider estimate** (the attack? the cutoff?) | A value | A one-item draw | Error in perceptual units (octaves, log ms): `ear-value/1` | A knob, a v2 draw | `ear-value/1` |
| **Match it** | A patch | A v2 draw, the rest locked | `sound/1` | **The whole Recreate round** | Nothing |
| **Tap back** (rhythm, syncopation) | Onsets | The rhythm generator | Inter-onset intervals after a best-fit offset, ±10 % tempo, misses and extras: `taps/1` | A count-in inside the clip | [Rhythm input](#rhythm-input) |

## Where R4 lives

| Piece | Home | Why |
| --- | --- | --- |
| Interval, chord, scale and progression tables (simple voice leading), a 16-step rhythm grid, a syncopation measure, note names; **ear generator v1** (an `EarSpec` → questions), frozen by a golden | **`enth-music`, new, L1** (needs only `enth-dsp`'s `derive` and `fnv1a64`) | Pure and tiny; the UI and CLI get labels without the challenge crate; spares `enth-challenge` (15.8k lines) a third owner set. It cannot be L0 (it uses `enth-dsp`, and same-layer edges are refused), nor L4 (`enth-challenge` must embed `EarSpec`) |
| Challenge v3 (`Target::Ear(EarSpec)`, `AnswerKind::{Choice, Value, Taps}`); `ear.rs` (a question → a timbre patch, a `Performance`, an optional post-EQ band); the timbre pool `timbres-v1/` (frozen, append-only); the scorers `ear/1`, `ear-value/1`, `taps/1`; the parameter games' items on v2 draws; the ear-challenge code kind | `enth-challenge` (L4) | Persisted challenge types live here, and the parameter games need the catalogue |
| A peaking and shelf biquad (RBJ form, coefficients through `math`) | `enth-dsp` (L0) | One primitive for the game EQ now and S2's EQ later |
| The drill player: N questions, the 3-down/1-up staircase, ear skills into Review's boxes, `StepKind::Drill` and the `drills` progress field (both additive), the workout composer | `enth-learn` (L5) | Progress and Review live here |
| `ear.rs`: questions as jobs, answers and taps with times from the shell, nothing saved per project (like Create) | `enth-session` (L6) | The seams of `game`, `learn` and `create` |
| `editor/ear/`, feature `ear`, the header hook, `sizes/ear.rs` | `enth-ui` (L7) | A right column like Create's, so a parameter game keeps a card in view |
| Ear jobs on the game worker, examples at the host's rate, the tap path; `enth ear` | `enth-plugin`, `enth-cli` (L8) | Shells translate |
| `ear-check`: oracle, random, bias and deaf players, audibility per level, the golden | `tools/lesson`, driven by `cargo xtask ear-check` | As `lesson-check` |

**Rejected:** everything inside `enth-challenge` (one crate fewer, but the result codes, v3 and the ladder tuning would collide; question 9 keeps it open); an L4 `enth-ear` (`enth-challenge` must embed `EarSpec`, and an L4 → L4 edge is refused); drills as `Choose` steps (`Choose` options are clips, while drills are open-ended and use labels).

```rust
pub struct EarSpec { game: u8, generator: u8, level: u8, seed: u64 /* ≤ 2^53 − 1 */, count: u8 }
pub struct Question { timbre: u8, notes: Vec<NoteSpec /* key, vel, start_ms, hold_ms */>, length_ms: u32,
                      post: Option<EqBand /* integers */>, answer: Answer, options: Options }
pub enum Answer { Choice(u8), Value { key: &'static str, bits: u32 }, Taps(Vec<u32 /* ms */>) }
```

A sketch from report 02 §2; the F1e judge fixes the real types.

## The ear generator

- **Keyed streams, as generator v1's.** Each item draws from `derive(seed, fnv1a64("game/item#/level"))`, so adding a game or an item never moves another's draw. A practice session is (game, seed, settings) plus the answer log.
- **Questions are integers:** notes (key, velocity, start and hold in ms), a length, a timbre id, an optional EQ band, the answer and the options. Generated phrases stay within C1–C7, at most 8 notes at once and at most 10 s.
- **Timbres** come from a frozen pool of our own patches (sine, saw low-pass, pluck, organ table, pad, a noise-burst click), append-only. Pitch drills rotate them, so the timbre is never the cue.
- **Answer keys are integer arithmetic**, identical on every platform and engine version; only a generator change moves them, and that is a new frozen version.
- **Frozen by a golden:** 1,000 specs → a hash, on Linux, Windows and wasm (macOS weekly), like generator v1's. **When it freezes** (this page's default; an open check in [phase-4.md](phase-4.md#open-checks)): at INT4's friend build, the first build that shares ear codes. Until then G1–G3 tune the ladders and update the golden on purpose (the commit names the games and why); after it, a change is ear generator v2.
- **The parameter games** (Spot the change, Dial it in, Ear check) are not in `enth-music`: they need the catalogue, which is L1 too. Their items are v2 draws built in `enth-challenge`: the game picks the option controls and the one that moves, at the level's distance in parameter steps (design §9's steps: cutoff 2 semitones, times × 1.5, levels 3 dB; generous steps for reverb), in parameter space only, as every generator rule is ([design.md §9](design.md#9-randomizer-and-difficulty)). `ear-check` measures each level's distances with `sound/1` offline; items in its deaf spots are listed as *not measured*.

## Rendering and playback

- **No engine change.** `Performance` already takes any notes and times; polyphony 1–16 exists, and timbre patches set it to at least 8; 12-TET pitch goes through the pinned `libm`.
- **Cheap:** one render is about 15–30 ms, so a 10-question set prepares in about 0.3 s, as a job on the game worker.
- **Played through the Target ▶ examples slot** (at most 8 position-locked loops, an `AtomicPtr` mailbox, never freed on the audio thread), with one additive flag: "play from the top".
- **No host transport.** Stimuli carry their own tempo and count-in, so a game or a duel never depends on a project. Target ▶ already relies on every REAPER setup running effects with the transport stopped; SKE checks it.
- **Level-matched** within 0.5 LU unless level is the cue.
- **EQ:** one band from the level's set (7 octave bands at ±12 dB, down to ⅓-octave at ±3 dB, as Quiztones' tiers) on pink noise or a pad, played as two level-matched loops. The band is integers in the question; lane BQ's biquad renders it offline, within 0.1 dB of the cookbook and with the same bits on six targets. S2's EQ module wraps the same primitive later.

## Rhythm input

**The facts** (report 02 §4, *measured* in the vendored source): egui-baseview sets the frame's input time once per frame and events wait for that frame, so at about 30 fps an editor tap is quantised to up to about 33 ms. Fork patch 4 gives keyboard focus only while a text field is edited, so keys reach the host unless a tap pad takes focus; clicks always arrive. Host notes reach `process()` with a sample offset in the buffer. People also tap tens of milliseconds early (Repp 2005), so tap scoring needs an offset anyway.

- **Rhythm pattern comes first and needs no input timing** (listen, pick, mark a grid), so it ships in R4a.
- **Tap-back is call and response:** `taps/1` scores the intervals between taps after a best-fit offset, with a one-to-one match, so constant output, block and input latencies cancel.
- **Two tap sources, picked by lane TAP's measurement:**
  - *Editor taps stamped on arrival* (the default): fork patch 12 (lane FK4) records an instant per baseview event, plus a tap pad that takes focus. No audio-thread contact.
  - *Host MIDI in the response window:* the player writes note-on sample positions into a preallocated `rtrb` ring, with an overflow test under `assert_no_alloc`. Sample-exact on our side, but **a new audio-thread path, so it waits for the owner's OK** (question 8). REAPER's live-input offsets are unmeasured.
- **Tap-along waits** for a calibration screen (16 clicks, the median offset kept in progress).
- **Missing evidence:** real jitter on the owner's display, in REAPER, on Windows and macOS; whether the tap pad's focus grab works with fork patch 4 (the TAP spike, [phase-4.md](phase-4.md#open-checks)).

## Difficulty and the staircase

- **3-down/1-up (about 79 %) for drills and Ear check;** Recreate keeps its 2-down/1-up (about 71 %, [design.md §9](design.md#9-randomizer-and-difficulty)). A higher preset success rate meant more practice and larger gains (Jansen 2013, n = 207 over 6 weeks); letting learners pick their own rate gave no benefit (Jansen 2016); 85 % is theory for machine learners (Wilson 2019). So about 80 %, and no difficulty slider beyond Recreate's.
- **Items are placed by measured JND distance**, never by random settings: SoundGym's "pot luck" came from random settings. Each item's distance is stated in JND units under `sound/1`, or in parameter steps where `sound/1` is deaf.
- **What a staircase may move**, one thing at a time: the step in JND units, the number of categories, their similarity, the context (a sine, then generator levels 3–5), roving (note, wave, velocity) and memory load.
- **A 2-JND floor** (`choice_audibility`'s rule) for morale and fairness, although even impossible items teach a little (Amitay 2006).
- **No visible streak:** the staircase's run counter stays internal.
- **Simulated before it ships:** DRL's test retargets to 79 %, holds 75–85 % success, and recovers simulated listeners' thresholds within one step in 90 % of 40-trial runs.

## The drill player

Lane DRL, in `enth-learn` (L5). The skill is the unit of spacing; the game is its vehicle ([learn.md](learn.md#ear-skills-bursts-and-drills-r4-planned-lane-drl)).

- **Skill keys:** `module.param` (Review's keys today) and `ear.*` (`ear.interval.m3`, `ear.chord.min`, `ear.band.1k`). The progress file keeps unknown keys, so this is additive.
- **Bursts:** 5 items on one skill, in any game; 4 or more right moves its box up, 3 holds it, 2 or fewer moves it down. Single answers in 2–4 option items are 25–50 % guessable.
- **Due skills choose the games.** A due `svf.cutoff` comes back as Spot the change one day, Dial it in another and Frequency finder later: varied retrieval, which is what generalises (Wright 2010). Weakness-targeted training beat uniform training for EQ (Kim 2015).
- **Interleaving:** each category is met alone first, then confusable ones mix (m3/M3, attack/decay, room/decay); one line of copy says why mixing feels harder (Carvalho & Goldstone 2014).
- **This closes learn.md's open item** "ear questions as Review items": a due skill without a Recreate step plays as a burst.
- **`StepKind::Drill`** (additive, declared in `requires`) lets a lesson play a drill; **the `drills` progress field** (additive) keeps thresholds per skill over time and the practised days.

## Workouts

- **A workout is 3 games × 12–20 items, about 10–15 minutes,** chosen by due skills, ending in "done for today", with "one more" optional. DRL's composer is pure: the same progress file on the same day gives the same workout.
- **After each item you replay A, B and the answer.** Practice plus passive exposure was the only regime that learned intervals and transferred to new timbres (Little 2019).
- **Feedback talks about the sound** ("the cutoff moved 3 semitones down"), never the person.
- **Progress is thresholds per skill over time,** in semitones, ms, cents or dB, plus a month calendar of practised days that is cumulative and can't be "lost".
- **After a break,** a warm-up one level below the last level reached, and no "you missed N days".
- **Not in R4:** streak counters (broken streaks cause drop-off, worse when self-blamed), loss messages, daily quests, "playing by appointment", grinding, currency, leaderboards (a leaderboard course lowered motivation and exam scores, Hanus & Fox 2015), perfect-pitch promises.
- **Dose:** the evidence conflicts (about 8 minutes a day learned fastest per trial in Molloy 2012; frequency discrimination needed more than 360 trials a day in Wright & Sabin 2007), so the default is short and the end is clear.

## The session and the Ear view

- **`session::ear`** (lane SE): questions go out as jobs, answers and taps come back with times the shell supplies, and nothing is saved per project, as with a create job; thresholds and practised days live in the per-user progress file.
- **`editor/ear/`** (lane EUI, feature `ear`): a right column like Create's, so a parameter game keeps its card in view; the Ear and Create columns are exclusive, and its overflow at 1200 × 800 is reported. Every round type is driven by label in kittest; 0 idle frames.
- **`enth ear`** (SKE, then EV4): play and answer ear games from the CLI, so agents can take them too (paid agent runs only with the owner's OK).

## Ear duels

Adaptive sessions diverge between players, so two players would hear different items. **Ear duels use a fixed ladder:** item *i*'s level depends only on *i*, and both players hear the same items. They come after R4's games exist. Anyone holding the code can compute the answer keys; the UI says so. The ear-challenge and ear-result codes are in [duels.md](duels.md#ear-duels).

## GR4

**Gate GR4** (proposed; the owner sets the criteria): the games are deterministic, fair and solvable, and you and your friends hear finer differences on trained skills than on held-out ones. The table of record will be `docs/r4-gates.md` (INT4, planned).

**Mechanical half** (CI: `cargo xtask ear-check`, lane ECK):

- **Determinism:** 1,000 items per game regenerate identical hashes on the six golden targets.
- **Audibility:** every option pair is at least 2 JND apart under `sound/1` (at least 6 at L1); items scored on parameter steps are listed apart.
- **Solvable:** an ideal-observer bot (picks the option nearest by `sound/1`) scores at least 95 % at the top level.
- **No leaks:** bias bots (always option 1, always the loudest, always the longest) stay at chance ± 3 % over 10,000 items.
- **Staircases:** simulated listeners with known thresholds get a 40-trial estimate within one step in at least 90 % of runs, and training holds 75–85 % success.
- **Coverage:** every observable Basic-tier P1 parameter is in game 1 or 2, and bursts move Review boxes (proptests, as REV's).

**Human half** (local, per person, as GR2; about 2.5 h for you over two weeks):

| Part | What | Time |
| --- | --- | --- |
| Retest pilot (you) | Ear check on 4 skills twice, a day apart; "real change" is more than twice the retest spread | 2 × 12 min |
| Pre-test | Ear check on 4 skills: 2 to train and 2 held out, counterbalanced (A trains cutoff and decay and holds out room and detune; B the reverse), plus an interval block | 15 min |
| Training | Two weeks, at least six workouts of 15 minutes or less, on the trained skills only | About 1.5 h |
| Post-test | The same checks with new seeds; a delayed one (2–4 weeks) is optional | 15 min each |
| Transfer (exploratory) | GR2's family codes, trained against held-out families | 10 min |
| Debrief | Fun 1–5; too easy or too hard; "name something you now hear" | 3 min |

- **Why held-out skills:** test practice alone improves auditory thresholds (Amitay 2006), so only improvement *relative to the held-out skills* counts. Report 02's lighter gate (the level rises on two games over three sessions, about 40 minutes) would pass from test practice alone; it stays the alternative (question 10).
- **Proposed criteria:** the median person improves both trained skills beyond the retest bar and by more than the held-out ones; no trained skill gets worse on average; you and at least half the friends play at least one workout beyond the required ones (from the local log, sent by choice); everyone names something specific they now hear.
- **Results are described, not tested** (N = 3–6). Far transfer to Recreate is reported, not gated, because it is expected to be weak.
- **Privacy:** the progress file and logs stay local; people send them by choice, and INT4's `gr4-report` reads them, as `gr2-report` does.

## Risks

| Risk | Retirement |
| --- | --- |
| **Tap jitter:** a frame-quantised editor, the host's keyboard focus, unknown MIDI offsets per host | Listening-only rhythm first; interval scoring; the TAP spike; your REAPER check |
| **Scope:** six round types × five families × ladders | R4a first; tap-back and progressions after; one game per ladder lane |
| **`sound/1`'s deaf spots** make parameter games gate weakly | Parameter steps, a *not measured* list, a listening review |
| **No transfer evidence** for synth-parameter games | GR4's held-out design and the Recreate row: enth's own evidence |
| **Hard skills:** bimodal chords, slow intervals, absolute-pitch training sheds most players | Honest copy; no content gated on them; note naming opt-in |
| **Repetitiveness** | Many skills across four vehicles, roving, workouts with an end |
| **No crowd calibration** (no telemetry) | Measured distances and simulated listeners |
| **Monitors** | Nothing under 80 Hz below L5; level matching |
| **Columns squeeze cards** at 1200 × 800 | Ear and Create columns exclusive; an overflow report |
| **Your ears:** GR4 needs listening, and you can't always listen | The mechanical half in CI; short sittings; defaults |

## Decisions (provisional)

| Area | Decision (the default unless you say otherwise) | Evidence | Status |
| --- | --- | --- | --- |
| Order | Spot the change, Dial it in, Ear check, Frequency finder, Interval stacks, Chord quality, Rhythm pattern; then progressions, note naming, tap-back | Report 01 §4 | Needs you (Q1) |
| Perfect pitch | Opt-in note naming with honest copy, outside the default workout | Report 01 §2 | Needs you (Q2) |
| Difficulty | 3-down/1-up for drills and Ear check; items by measured JND distance; Recreate keeps 2-down/1-up | Report 01 §3 | Proposed |
| Sessions | 3 games, 10–15 min, a clear end; replay with the answer; thresholds in physical units; no streaks, leaderboards or currency | Report 01 §3 | Proposed |
| Spacing | Skill keys `module.param` and `ear.*`; 5-item bursts move boxes; due skills choose the games | Report 01 §3 | Proposed |
| Crate | `enth-music` at L1 | Report 02 §2 | Needs you (Q9) |
| Challenge v3 | `Target::Ear(EarSpec)`, `AnswerKind::{Choice, Value, Taps}`, scorers `ear/1`, `ear-value/1`, `taps/1` | Report 02 §2 | Proposed (F1e judge) |
| Drill player | In `enth-learn`: the staircase, ear skills into Review, `StepKind::Drill`, the workout composer | Reports 02 §2, 01 §3 | Proposed |
| EQ games | An offline RBJ biquad in `enth-dsp` now; S2's EQ wraps it; Frequency finder keeps its svf form | Report 02 §2–§3 | Needs you (Q7) |
| Rhythm v1 | Listening-only in R4a; tap-back on inter-onset intervals after TAP; no host transport | Reports 01 §2, 02 §4 | Proposed |
| Tap source | Editor stamps (fork patch 12) first; a host-MIDI ring only with your OK | Report 02 §4 | Needs you (Q8) |
| Generator freeze | Ear generator v1 freezes at INT4's friend build; ladders move its golden on purpose until then | This page | Proposed |
| Ear duels | A fixed ladder, after R4's games; computable keys said plainly | Report 01 §4 | Proposed |
| GR4 | The CI half plus trained against held-out skills over two weeks | Reports 01 §5, 02 §6 | Needs you (Q10) |
