# Ear training (R4): games, drills and workouts

R4 trains the ear that sound design needs: which knob moved, how far, where a band sits, and then the music side (intervals, chords, rhythm). Every game is built from what exists: a stimulus is an offline render played through the Target ▶ slot, an answer is a label, a value, a patch or taps, and every skill feeds Review's boxes. This page is the design of record; the lane plan is [phase-4.md](phase-4.md), duels on these games are in [duels.md](duels.md), and the evidence is the [R4/R5 research session](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md) (reports 01 pedagogy and game design, 02 architecture and the lane plan, 03 duels; the summary wins where it corrects a report).

**Status (2026-10-22): planned; nothing is built.** Every decision here is provisional until you steer; work proceeds on the defaults meanwhile. MUS, CV3, BQ, DRL, ECK, EUI and TAP add their sections as they land, and INT4 keeps this status line ([phase-4.md](phase-4.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| Theory tables, rhythm, ear generator v1 | `enth-music` (L1, new) | MUS; G1–G3 the ladders | MUS built, level 1 of every ladder ([as built](#ear-generator-v1-as-built-lane-mus)); freezes at INT4 (F1e judges the API) |
| Challenge v3: `Target::Ear`, choice, value and tap answers, ear rendering, timbres, scorers, the ear-challenge code | `enth-challenge` (L4) | CV3 | Planned (F1e; [challenge.md](challenge.md#challenge-v3-planned-lane-cv3-r4)) |
| The parameter games' items (Spot the change, Dial it in, Ear check) | `enth-challenge` (L4), on v2 draws | CV3; G4 the ladder | Planned |
| The EQ biquad | `enth-dsp/src/biquad.rs` (L0) | BQ | Built ([as built](#the-offline-biquad-as-built-lane-bq); [dsp/biquad.md](dsp/biquad.md)); `Clip::render` applies it |
| The drill player: staircase, ear skills in Review, `StepKind::Drill`, the `drills` field, the workout composer | `enth-learn` (L5) | DRL | Built (F1e; [learn.md](learn.md#ear-skills-bursts-and-drills-r4-lane-drl)) |
| Ear sessions | `enth-session/src/ear.rs` (L6) | SE | Planned (F1e) |
| Ear jobs, examples, the restart flag; `enth ear` | `enth-plugin/src/ear.rs`, `enth-cli/src/cmd/ear.rs` (L8) | SKE, EV4 | Planned (F2e) |
| The Ear view | `enth-ui/src/editor/ear/` (L7, feature `ear`) | EUI | Planned |
| Tap input | Fork patch 12 (`vendor/egui-baseview`), the plugin's tap path | FK4, TAP | Planned |
| `ear-check` | `tools/lesson/src/ear/`, `cargo xtask ear-check`, `ear.yml`, the `scripts/gate.sh lane` hook | ECK | Built ([as built](#ear-check-as-built-lane-eck)) |
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
| `ear-check`: oracle, random, bias and deaf players, audibility per level, the golden | `tools/lesson`, driven by `cargo xtask ear-check` | As `lesson-check` (built: [as built](#ear-check-as-built-lane-eck)) |

**Rejected:** everything inside `enth-challenge` (one crate fewer, but the result codes, v3 and the ladder tuning would collide; question 9 keeps it open); an L4 `enth-ear` (`enth-challenge` must embed `EarSpec`, and an L4 → L4 edge is refused); drills as `Choose` steps (`Choose` options are clips, while drills are open-ended and use labels).

```rust
pub struct EarSpec { game: u8, generator: u8, level: u8, seed: u64 /* ≤ 2^53 − 1 */, count: u8 }
pub struct Question { timbre: u8, notes: Vec<NoteSpec /* key, vel, start_ms, hold_ms */>, length_ms: u32,
                      post: Option<EqBand /* integers */>, answer: Answer, options: Options }
pub enum Answer { Choice(u8), Value { key: &'static str, bits: u32 }, Taps(Vec<u32 /* ms */>) }
```

A sketch from report 02 §2; the F1e judge fixes the real types. The types as built are in [Ear generator v1, as built](#ear-generator-v1-as-built-lane-mus).

## The ear generator

- **Keyed streams, as generator v1's.** Each item draws from `derive(seed, fnv1a64("game/item#/level"))`, so adding a game or an item never moves another's draw. A practice session is (game, seed, settings) plus the answer log.
- **Questions are integers:** notes (key, velocity, start and hold in ms), a length, a timbre id, an optional EQ band, the answer and the options. Generated phrases stay within C1–C7, at most 8 notes at once and at most 10 s.
- **Timbres** come from a frozen pool of our own patches (sine, saw low-pass, pluck, organ table, pad, a noise-burst click), append-only. Pitch drills rotate them, so the timbre is never the cue.
- **Answer keys are integer arithmetic**, identical on every platform and engine version; only a generator change moves them, and that is a new frozen version.
- **Frozen by a golden:** fixed specs → question-set digests, one block per (game, level), on Linux, Windows and wasm (macOS weekly), like generator v1's. **When it freezes** (this page's default; an open check in [phase-4.md](phase-4.md#open-checks)): at INT4's friend build, the first build that shares ear codes. Until then G1–G3 tune the ladders and update the golden on purpose (the commit names the games and why); after it, a change is ear generator v2.
- **The parameter games** (Spot the change, Dial it in, Ear check) are not in `enth-music`: they need the catalogue, which is L1 too. Their items are v2 draws built in `enth-challenge`: the game picks the option controls and the one that moves, at the level's distance in parameter steps (design §9's steps: cutoff 2 semitones, times × 1.5, levels 3 dB; generous steps for reverb), in parameter space only, as every generator rule is ([design.md §9](design.md#9-randomizer-and-difficulty)). `ear-check` measures each level's distances with `sound/1` offline; items in its deaf spots are listed as *not measured*.

## Ear generator v1, as built (lane MUS)

Built 2026-10-22 in `enth-music` (L1: `theory`, `rhythm`, `ear`, `ladder/`), with level 1 of every ladder file. Start at `enth_music::ear::EarSpec` and `enth_music::ear::question`. The types are proposals for the F1e judge; the rest is frozen from INT4's friend build.

**Games and ids.** One numbering for every ear game, so a spec names its game with one byte. The parameter games are generated in `enth-challenge` (they need the catalogue), and this crate refuses their ids.

| Id | Game | Generated in | Ladder file | Answer | Levels built |
| --- | --- | --- | --- | --- | --- |
| 1, 2, 3 | Spot the change, Dial it in, Ear check | `enth-challenge` (CV3; G4 the ladder) | — | — | refused here (`UnknownGame`) |
| 4 | Frequency finder | `enth-music` | `ladder::bands` (G3) | a choice of band centre | 1 |
| 5 | Interval stacks | `enth-music` | `ladder::intervals` (G1) | a choice of interval | 1 |
| 6 | Chord quality | `enth-music` | `ladder::chords` (G2) | a choice of quality | 1 |
| 7 | Rhythm pattern | `enth-music` | `ladder::rhythm` (G2) | a choice of one of four patterns | 1 |
| 8 | Progressions (R4b) | `enth-music` | `ladder::progressions` (G2) | a choice of progression | 1 |
| 9 | Note naming (R4b, opt-in) | `enth-music` | `ladder::pitch` (G1) | a choice of pitch class | 1 |
| 10 | Tap back (R4b, after the TAP spike) | `enth-music` | `ladder::taps` (TAP) | tap times | 1 |

**The spec.** `{"game":5,"generator":1,"level":1,"seed":42,"count":20}`: unsigned integers in this order, unknown fields refused. **One type for every game and every crate** (lane F1E, the F1e review's M3): `enth-challenge` re-exports it, and the session's ear plan holds it. `EarSpec::validate_music` accepts a generator from 1 to the build's latest (1), a music game, a level from 1 to the game's `max_level()` (at most 15), a seed below 2^53 and a count of 1 to 64; each refusal is a distinct `EarError`, and a build maps any of them to "update enth". It knows only the music games (the parameter games are `UnknownGame` here); `enth_challenge::ear::validate` checks both families and is what challenges, codes and sessions use. **`generator` is read per family** (should-fix S9): `enth_music::ear::LATEST_GENERATOR` counts games 4 and up, `enth_challenge::ear::params::PARAMS_GENERATOR` games 1 to 3. A released level never changes its questions; a change is that family's next version.

**The question.** `Question { timbre, notes, length_ms, post, answer, options }`, all integers:

| Field | Form |
| --- | --- |
| `timbre` | An id into `enth-challenge`'s frozen pool `timbres-v1` (0 sine, 1 saw low-pass, 2 pluck, 3 organ table, 4 pad, 5 noise-burst click), and **6, sustained pink noise, which level 1 of Frequency finder needs and the pool must add** (append-only) |
| `notes` | `NoteSpec { key, velocity, start_ms, hold_ms }`, the same four integers as the challenge crate's `PhraseNote`, so a question becomes a phrase one to one. Keys C1 to C7 (24 to 96, C4 = 60), velocities 1 to 127 |
| `length_ms` | The last note-off plus a 1 s release tail, at most 10,000 |
| `post` | An optional `EqBand { kind, freq_hz, gain_cdb, q_milli }`: `Peak`, `LowShelf`, `HighShelf` (the biquad) or `BandPass` (the synth's svf band-pass, Frequency finder's level 1) |
| `answer` | `Choice(index into options)` or `Taps { onsets_ms, window_ms }` (onsets from the start of the response bar, strictly increasing). No value answer yet: G3's log-slider level adds it, additively |
| `options` | `Options { kind, ids }`: what the ids mean (an interval size, a chord index, a pitch class, a band centre in Hz, a 16-bit rhythm mask, a progression index) so labels and skills come from the tables; none for taps |

`Question::validate` checks the invariants: C1 to C7, at most 8 notes sounding at once (half-open intervals, as the phrase facts count), at most 10 s, every note inside the clip, a known timbre, a sane band, and a choice that is one of 2 to 12 distinct options. The proptests run it on every game and level.

**Keyed streams.** Item `i` of a game at level `l` draws from `derive(seed, fnv1a64("<game key>/<i>/<l>"))` with the keys `band`, `interval`, `chord`, `rhythm`, `progression`, `pitch` and `tap` (`interval/7/1`). `ear::question(game, level, seed, item)` computes any item at any level, which is what a staircase, a replay and an ear duel's fixed ladder (item `i`'s level a function of `i`) need. Each ladder file documents its **draw order**; it is part of the frozen form. Label games list options in ascending order of their id, so buttons keep their places, and the answer's position is uniform (a test checks it, and that every timbre meets every answer). The rhythm patterns are listed in draw order. Review skill keys come from the right answer (`Question::skill()`): `ear.interval.m3`, `ear.chord.min`, `ear.band.1k`, `ear.pitch.cs`, `ear.progression.i-iv-v-i`, `ear.rhythm.pattern`, `ear.rhythm.taps`. A burst on one skill (the drill player's 5 items) has no field in the spec: `ear::find_skill_item(game, level, seed, skill, from)` scans the keyed items from `from` for the next whose question exercises the skill, so the burst is a function of the seed and two players get the same one.

**Level 1 of each ladder**, as G1 to G3 and TAP find it:

| Game | Level 1 |
| --- | --- |
| Interval stacks | P1 against P8, harmonic, root C3 to C5, a saw, an organ or a pad, held 2 s. A unison is one note (a stack of two oscillators on one key); an octave is the root and the key 12 above |
| Chord quality | Major or minor, root position, root C3 to C4, a pad, an organ or a saw, held 2.5 s |
| Note naming | Two pitch classes a fourth or more apart (circular distance 5 or 6), one note over C3 to B5, four timbres |
| Frequency finder | The svf band-pass (Q 8) on noise at one of 125, 250, 500, 2,000, 4,000 or 8,000 Hz; two options at least three octaves apart |
| Progressions | `I-IV-V-I` against `I-V-vi-IV` in any of 12 major keys, four chords of 1.2 s voiced with small moves (root in the bass, three upper voices), a pad or an organ |
| Rhythm pattern | Four eighth-note patterns (3 to 6 onsets, the downbeat always struck, syncopation 0 or 1, pairwise at least 3 steps apart) at 100 bpm; a bar of clicks, then two bars of the pattern over the clicks, 8.2 s. Mark the answer on the grid with `options.ids[answer]` |
| Tap back | A pattern of 3 to 5 eighth-note onsets, syncopation 0, after a bar of clicks; the answer is its onset times in one 2.4 s bar |

**The tables** (`theory`, append-only): 13 intervals, 11 chord qualities (triads, sus, sevenths) with inversions, 13 scales, 12 progressions with `voice_progression`, note names with sharps and flats. **The rhythm grid** (`rhythm`): a bar of 16 sixteenths as a `u16` mask, density, swing as an integer percentage, and the Longuet-Higgins and Lee syncopation measure on the metric weights 0, -1, -2, -3, -4 (the sum, over the onsets, of how far the silence after an onset climbs the hierarchy above it; son clave measures 4, rumba clave 5).

**The golden.** `crates/enth-music/tests/golden/ear-v1.txt`, written by `cargo test -p enth-music --test ear_golden -- --ignored regenerate`: **one block per (game, level)** (should-fix S4 of the F1e review, lane F1E): a `block` line, then 150 specs of that game and level, the `j`-th with a 53-bit seed from a `SplitMix64` stream keyed by (game, level, `j`) and a count of 1 to 20 (every tenth 64), each line the first 16 hex digits of the SHA-256 of the canonical bytes of its questions, then a `sum` line, the SHA-256 of the block's digests. A spec depends only on its game, level and `j`, so **a new level or game appends a block and no existing line changes**, and tuning a level changes its block alone: a ladder lane's diff names its games (MUS's first form cycled 1,000 specs through every pair, so any new level rewrote every line). The test prints a hash of the block sums (`blocks b6449a68…`); `golden.yml` runs it on Linux (x86-64 and aarch64), Windows, macOS and under wasmtime (wasm32-wasip1); lane F1E ran it natively in debug and release and on wasm32-wasip1 under node's WASI, with the same hash. The questions are unchanged by the restructure (the old file still passed before it was replaced). **Not yet frozen:** until INT4's friend build a ladder lane that adds or tunes a level regenerates the file on purpose, and the commit names the games and why.

## Rendering and playback

- **No engine change.** `Performance` already takes any notes and times; polyphony 1–16 exists, and timbre patches set it to at least 8; 12-TET pitch goes through the pinned `libm`.
- **Cheap:** one render is about 15–30 ms, so a 10-question set prepares in about 0.3 s, as a job on the game worker.
- **Played through the Target ▶ examples slot** (at most 8 position-locked loops, an `AtomicPtr` mailbox, never freed on the audio thread), with one additive flag: "play from the top".
- **No host transport.** Stimuli carry their own tempo and count-in, so a game or a duel never depends on a project. Target ▶ already relies on every REAPER setup running effects with the transport stopped; SKE checks it.
- **Level-matched** within 0.5 LU unless level is the cue.
- **EQ:** one band from the level's set (7 octave bands at ±12 dB, down to ⅓-octave at ±3 dB, as Quiztones' tiers) on pink noise or a pad, played as two level-matched loops. The band is integers in the question; lane BQ's biquad renders it offline, within 0.1 dB of the cookbook and with the same bits on six targets. S2's EQ module wraps the same primitive later.

## Challenge v3 as built (lane CV3)

**Built 2026-10-22; judged at F1e.** The reference is [challenge.md](challenge.md#challenge-v3-planned-lane-cv3-r4); what a game lane needs to know:

- **One target for every question game.** `Target::Ear(EarSpec)` names games 1 (Spot the change) and 3 (Ear check), generated in `enth_challenge::ear::params` on v2 draws, and games 4 and up, `enth-music`'s. Dial it in (game 2) stays a constrained Recreate round: `ear::params::dial_it_in` gives each round as a v2 draw challenge. An ear target takes phrase 0, the template 0 start, no constraint, and the answer its game asks for.
- **Level 1 of each parameter game** (two far options, filter against envelope, 6 steps; Ear check from 8 steps on one control drawn per set; Dial it in 6 steps), on the starter through the held note. G4's ladder replaces `params::level`.
- **The timbre pool `timbres-v1`:** 0 sine, 1 saw low-pass, 2 pluck, 3 organ table, 4 pad, 5 noise click, 6 sustained pink noise (MUS's Frequency finder), frozen by SHA-256 and append-only.
- **Bands:** a band-pass band sets the timbre's own filter (Frequency finder's svf form, rendered by the engine); a peak or shelf band is applied after the engine by BQ's biquad ([as built](#the-offline-biquad-as-built-lane-bq); `needs_biquad` was the answer before BQ landed).
- **Judged with no render:** `ear/1` (an index), `ear-value/1` (an error in octaves, doublings of ms or the unit, within a tolerance), `taps/1` (after the best-fit offset and a tempo within ±10 %, a one-to-one match; misses and extras count). Keys are integers (a value's `f32` bits), logarithms go through the pinned `libm`.
- **The ear-challenge code** (kind 3): 19 symbols starting with `F`, code v1's check last; an R1–R3 build reads it as "update enth". A seed above 40 bits has no code, as for a short challenge code.
- **Closed at F1e (lane F1E):** an `EarSpec` names no skill (the judge accepted it: fixed-ladder duels need none); a burst's focus goes through one resolver over both families, `enth_challenge::ear::find_item(&spec, skill, from)` (should-fix S3), which the session's ear plan takes as its start item. The music games are wired (`ear::music`, `question::music_question`), so every game from 4 to 10 prepares, judges and has a code.

## The offline biquad as built (lane BQ)

**Built 2026-10-22.** The reference is [dsp/biquad.md](dsp/biquad.md); what a game lane needs to know:

- **What it is:** `enth_dsp::biquad`, the Audio EQ Cookbook's `peakingEQ`, `lowShelf` and `highShelf` in their `Q` form, designed and run in `f64` through the pinned `libm` (`pow`, `sin`, `cos`), over an `f32` buffer in place. `Coeffs::design(kind, freq_hz, gain_db, q, sample_rate)` then `Biquad::new(coeffs).process(&mut buf)`; no allocation, any block size gives the same bits, a gain of 0 is the identity bit for bit.
- **Accuracy:** against the cookbook's analog prototypes under its pre-warped bilinear transform, the coefficients agree to 3.8e-8 dB over 84,672 points and the processed response (impulse response through `process` in `f32`) to 5.1e-5 dB over 285 sections, against the 0.1 dB acceptance. The cookbook's digital curve is squeezed near Nyquist relative to the analog one at the plain frequency (2.2 dB at most for an 8 kHz peak, 7.2 dB for a 16 kHz one, +12 dB, `Q` 1, 48 kHz): the corner is exact, the shape above about 4 kHz is the cookbook's.
- **The golden:** `crates/enth-dsp/tests/biquad_golden.rs` (117 sections, coefficient bits and output hashes, one sum), run by `golden.yml` on the six golden targets: Linux x86-64 and aarch64, Windows, macOS arm64, wasm32-wasip1 and the same with `simd128`.
- **Wired in:** `Clip::render` (in `ear/question.rs`) renders a stimulus with the engine and then filters both channels, release tail included, with the section its peak or shelf band designs; the band's integers become `f64` by one IEEE division each (`gain_cdb / 100`, `q_milli / 1000`). `needs_biquad` is no longer returned (the variant and its code stay, so nothing that matches on it moves). A band-pass band never reaches the biquad.
- **For G3:** the ranges in [challenge.md](challenge.md) (20 Hz to 20 kHz, ±0.01 to ±24 dB, `Q` 0.1 to 25) are well inside the biquad's; a +24 dB boost can pass full scale, so **level matching stays playback's**. The first banded questions are G3's to write: the generator needs no change, only a level that draws a peak or shelf band.
- **S2's EQ** wraps `Coeffs` and `Biquad`; per-segment coefficient ramps (automation) are its to add.

## `ear-check` as built (lane ECK)

**Built 2026-10-25.** `cargo xtask ear-check [options]` drives `enth-lesson-tool ear-check` (`tools/lesson/src/ear/`; `--help` lists the options: `--items`, `--bot-items`, `--runs`, `--jobs`, `--seed`, `--only`, `--strict`, `--ladders enforce|report`, `--update-golden`). It plays every level of every shipped game and prints one report; the exit status is the verdict.

**What it plays.** An *item* is a question plus the clip of **every option as it would be if that option were the answer** (the counterfactual). Music games get theirs from `enth_music::ladder::with_answer` (the question with another right answer; identity-tested: for the question's own answer it returns the question). Spot the change gets the base and one patch per option, each moved by that control (the other direction when the wanted one is out of range); Ear check gets the base and the odd interval. Anything that implements `EarGame` (id, levels, `item(level, seed, index)`) can be checked, which is how the tests plant bad games.

**Players, all judged through the real `ear/1`:**

| Player | Picks | What it finds |
| --- | --- | --- |
| Oracle | the key | 100 % right, or the key and the judge disagree |
| Random, deaf | uniformly; `item index mod options` | a key that follows the order of play |
| Bias bots: first, last, longest, loudest (note energy), highest, densest | the extreme option of one property, ties by a seeded draw | a leak: the key correlates with a property of the options |
| Ideal observer | the option whose clip is within 1 JND of the right clip (guesses among those); the change must be at least 1 JND to be detected | a pair `sound/1` cannot tell apart; exact expected accuracy, no sampling |
| Loudest as rendered | the option with the highest rendered loudness | a finding only: playback level-matches ([above](#rendering-and-playback)), so it never fails |

**Checks and the exit policy.** A **hard** check fails the run always; a **ladder** check is reported for a game that has one level (G1–G4's ladders are still to come) and fails the run for a game with two levels or more; `--strict` (`--ladders enforce`) makes every ladder check hard, `--ladders report` makes none.

| Check | Gate | Holds when |
| --- | --- | --- |
| `golden-music` | hard | `crates/enth-music/tests/golden/ear-v1.txt`, recomputed from the file's own seeds and counts with the build's generator, has 0 differences and no missing or extra block |
| `golden-items` | hard | every (game, level)'s 1,000 items (4 seeds of 250: question and every option's clip) digest to the line in `tools/lesson/src/ear/golden/items-v1.txt` |
| `structure` | hard | every item draws, draws the same twice, names a Review skill key, and the judge accepts every answer (music: the counterfactual of the key is the question) |
| `bursts` | hard | each skill a cell exercises finds five items and a burst of them moves its Review boxes |
| `oracle`, `chance` | hard | the key through the judge is 100 %; random and deaf are at chance ± 3 % (five standard errors on a run shorter than 10,000 items) |
| `bias` | hard | every bias bot at chance ± 3 % over 10,000 items per level (same allowance) |
| `bias-render` | note | the loudest option as rendered is at chance, with a binomial allowance |
| `audibility` | ladder | every option pair of a 24-item sample per level is at least 2 JND apart, 6 at level 1 |
| `observer` | ladder | at the top level the ideal observer scores at least 95 % |
| `staircase` | ladder | for a game of four levels or more, and for Ear check's step ladder: the F1e judge's criteria (below) |
| `coverage` | ladder | every observable Basic-tier P1 parameter is in the pool of game 1 or 2 |

**`sound/1` offline.** `sound/1` only compares patches under one table phrase, and a music clip's notes are not a table phrase, so `ear/metric.rs` replays its pipeline over any clip: the scoring-seed render (and the biquad band of an EQ question), the pooled log-mel at -23 LUFS, the nuisance floor from two other-seed renders, the scale from 16 jittered copies, and the pitch and odd-harmonic guard. It uses `enth-challenge`'s own functions (`analyse_render` and `clip_distance`, two additive public functions in `score/sound.rs`), and a test pins it to `score_round`'s `jnd` on a patch pair to 1e-9, so a change to `sound/1` moves both.

**Not measured, listed apart:** pairs whose clips hold the same pitch classes for the same total time and differ only in order or timing (rhythm patterns, progressions' voice order, which a pooled spectrum cannot hear); the ideal observer is credited with telling such a pair apart, so an unmeasured pair never makes a level look unsolvable. Ear check's parameter steps of controls `sound/1` cannot hear (attack under 60 ms, sustain, detune) are `DEAF`, and are listed, not counted.

**Staircases.** `enth_learn::drill::Staircase` (3-down/1-up, estimate the mean of the last 12 reversals at most) plays 40 trials, 300 runs per cell, against a Gaussian m-alternative listener with `d' = 0.954 x acuity x JND` (acuities 0.5, 0.7, 1, 1.4, 2; lapse 0 and 3 %), whose threshold is where it is 79.4 % right on the game's own **measured** ladder (one row per level: the median closest pair, in JND). A ladder is simulated from four levels; at least three of the five acuities must have a threshold on it; a ladder is steep from a median 0.75 JND per level and must be monotone (a harder level at most 0.25 JND easier). The ruling checked is the F1e one: steep, at least 90 % pooled within one level and every listener-and-start cell at least 75 %; flat, at least 90 % within two; success at threshold 75 to 85 %.

**How a game lane uses it.**

- A new level of a music game is a `LEVELS` entry plus an appended block in the music golden; of a parameter game, the new rows of `params::level`. Run `cargo xtask ear-check --update-golden` and commit the lines it lists as moved (the commit message says which games and why; it is refused under CI). Level 2 makes the ladder checks hard for that game: `cargo xtask ear-check --only <game>` is the loop.
- A level that fails `audibility` names its closest pair (item, seed, options) in the report: replay it with `--only <game> --items N`; fix the generator, or if `sound/1` is deaf there, say so in the game's doc and ask for the pair to be listed as not measured (the `order_only` and `DEAF` tables are in `audibility.rs`).

**Numbers on main today (2026-10-25).** The hard checks pass for every game. The run is 109 s on the dev machine (4 threads, 8 s golden, 91 s levels, 10 s staircases) and 54 s on a quiet 12-thread desktop, with identical output on both; the release build of the tool is the rest of CI's time (about 5 min cold at 2 jobs, one to two minutes with `rust-cache` warm), so the workflow takes well under its 10 minutes. `gate.sh lane` ran it as its last step (83 s there, build included). Today's single levels fail some ladder criteria, reported and not failing:

| Game | Closest pair, min / median (6 JND at L1) | Observer (95 %) | Not measured |
| --- | --- | --- | --- |
| Spot the change | 0.0 / 2.0 | 85.4 % | 0 of 72 |
| Ear check | 0.0 / 0.4 | 33.3 % | 0 of 96 |
| Frequency finder | 9.4 / 11.4 | 100 % | 0 of 24 |
| Interval stacks | 0.8 / 1.0 | 77.1 % | 0 of 24 |
| Chord quality | 0.1 / 48.4 | 91.7 % | 0 of 24 |
| Rhythm pattern | 2.3 / 4.4 | 100 % (credit) | 29 of 144 |
| Progressions | 0.1 / 0.2 | 50.0 % | 0 of 24 |
| Note naming | 20.4 / 25.9 | 100 % | 0 of 24 |
| Tap back | no options | n/a | n/a |

Ear check's reference step ladder (8 steps, one control per set), simulated as a ladder: `filter1.cutoff` is steep (0.77 JND per level) and places 92.6 % within one level, but its worst cell (acuity 0.5, from the hard end) is 72.7 %, under 75 %; `filter1.resonance` is flat and passes (94.2 % within two); `env1.decay` is inaudible through the held phrase (0.0 to 0.2 JND at every step); `env1.release` is flat and not monotone. Coverage: 4 of the 10 observable Basic-tier parameters are in game 1 or 2's pool (missing `osc1.wave`, `osc1.semi`, `osc1.fine`, `filter1.mode`, `env1.attack`, `env1.sustain`).

**Open.** The golden's six-target run is for the music generator (`golden.yml`, by `ear_golden.rs`); the parameter games' items digest runs on `ear.yml`'s Linux runner only. The rendered-loudness note says Ear check's odd interval is louder as rendered, which playback's level matching must cover. `sound/1` hears a unison and an octave 0.8 JND apart, and progressions' order 0.1 JND apart: G1 and G2's L1 pairs need another cue, or the 6-JND bar for those games needs the owner's ruling.

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
- **Simulated before it ships:** DRL's test retargets to 79 %, holds 75–85 % success, and recovers simulated listeners' 40-trial thresholds as the F1e judge ruled: **pooled over the simulated listeners, at least 90 % within one level on ladders of about one JND or more per level; every listener-and-start cell at least 75 %; flat ladders at least 90 % within two levels; success at threshold 75–85 %.** 40 answers cannot place a threshold within one level of half a JND (the maximum-likelihood bound is about 89 % there), so the criterion changed, not the rule: G1–G4 build ladders with steps of at least about one JND and list each level's measured distance, and the Ear view shows thresholds as trends.

## The drill player

Lane DRL, in `enth-learn` (L5). The skill is the unit of spacing; the game is its vehicle ([learn.md](learn.md#ear-skills-bursts-and-drills-r4-lane-drl)).

- **Skill keys:** `module.param` (Review's keys today) and `ear.*` (`ear.interval.m3`, `ear.chord.min`, `ear.band.1k`). The progress file keeps unknown keys, so this is additive.
- **Bursts:** 5 items on one skill, in any game; 4 or more right moves its box up, 3 holds it, 2 or fewer moves it down. Single answers in 2–4 option items are 25–50 % guessable.
- **Due skills choose the games.** A due `svf.cutoff` comes back as Spot the change one day, Dial it in another and Frequency finder later: varied retrieval, which is what generalises (Wright 2010). Weakness-targeted training beat uniform training for EQ (Kim 2015).
- **Interleaving:** each category is met alone first, then confusable ones mix (m3/M3, attack/decay, room/decay); one line of copy says why mixing feels harder (Carvalho & Goldstone 2014).
- **This closes learn.md's open item** "ear questions as Review items": a due skill without a Recreate step plays as a burst.
- **`StepKind::Drill`** (an `enth.lesson/2` piece, so a pack with one says `requires.lesson = 2` and an R2 or R3 build answers "needs a newer enth"; the F1e review's M1) lets a lesson play a drill; **the `drills` progress field** (additive) keeps thresholds per skill over time and the practised days.
- **As built (2026-10-22, lane DRL):** all of the above is in `enth-learn` and pure; [learn.md](learn.md#ear-skills-bursts-and-drills-r4-lane-drl) has the built form, the shell's loop and the numbers. Two things differ from the plan's wording. The estimate is the mean of the last up to 12 reversals (a 40-trial track turns about ten times), and "within one step in 90 % of 40-trial runs" holds pooled for steep ladders (a level of about a JND or more: 92 % over 60 simulated listeners), not in every cell and not for ladders of half a JND a level, where 40 answers cannot carry it (79 % within one level, 97 % within two): build ladders with steps of about a JND. The F1e judge ruled the criterion accordingly ([Difficulty and the staircase](#difficulty-and-the-staircase)), and these numbers meet it. And the composer drills every due skill its catalogue lists, so the shell lists only the parameter skills the learner has met.

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

**Mechanical half** (CI: `cargo xtask ear-check`, lane ECK; built, with its exit policy and today's numbers: [as built](#ear-check-as-built-lane-eck)):

- **Determinism:** 1,000 items per game regenerate identical hashes on the six golden targets.
- **Audibility:** every option pair is at least 2 JND apart under `sound/1` (at least 6 at L1); items scored on parameter steps are listed apart.
- **Solvable:** an ideal-observer bot (picks the option nearest by `sound/1`) scores at least 95 % at the top level.
- **No leaks:** bias bots (always option 1, always the loudest, always the longest) stay at chance ± 3 % over 10,000 items.
- **Staircases** (as the F1e judge ruled): simulated listeners' 40-trial estimates are, pooled, within one level in at least 90 % of runs on ladders of about one JND or more per level; every listener-and-start cell at least 75 %; flat ladders within two levels in at least 90 %; training holds 75–85 % success at threshold. ECK checks ladder steps against `sound/1` distances where it can measure them and lists the rest as *not measured*.
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
