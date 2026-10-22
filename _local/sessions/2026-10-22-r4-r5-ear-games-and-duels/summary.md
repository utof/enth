# Summary: R4 Ear-training games and R5 Duels research (2026-10-22)

Three reports, written in parallel: [01 ear-games pedagogy](01-ear-games-pedagogy.md), [02 architecture, risks and the lane plan](02-architecture-plan.md), [03 duels, sharing and later live play](03-duels-sharing.md). The coordinator wrote this summary and checked the claims that touch the repo (main `b54ee01`). The decisions below are provisional until the owner steers; work proceeds on the defaults meanwhile.

## The answer

- **Sound-design games first, music theory second.** Musical training sharpens pitch but not brightness (Allen & Oxenham 2014), so interval drills won't train a cutoff or decay ear. The sound-design games are also the cheapest: each is a one-item v2 draw plus a choice, or a one-knob Recreate round (01 §2, §4).
- **R4a, in order:** Spot the change (which knob moved?), Dial it in (match one knob by ear), Ear check (a staircase that reports your threshold in real units), Frequency finder, Interval stacks, Chord quality, Rhythm pattern (listening only). Then progressions, opt-in note naming and tap-back; tap-along is deferred.
- **Honest training design:** about 80 % success (3-down/1-up); workouts of 3 games × about 4 minutes with an end; progress as thresholds in semitones, ms or dB; no streaks, leaderboards or perfect-pitch promises (after about 21 h, 2 of 12 adults learned all 12 notes and 35 of 48 starters quit; 01 §2–§3).
- **No engine change.** Every stimulus is an offline render played through the existing Target ▶ examples slot. New: a tiny pure crate `enth-music` (L1: theory tables and a frozen ear generator), Challenge v3 (`Target::Ear`; choice, value and tap answers), a drill player in `enth-learn` whose ear skills feed the Review boxes, and an offline peaking/shelf biquad in `enth-dsp` for the EQ game (02 §0–§3).
- **Rhythm timing is the one real unknown.** The editor sees input once per frame (up to about 33 ms at 30 fps). Listening-only rhythm comes first; tap-back is scored on the intervals between taps (latency cancels) after a measured spike; host-MIDI taps need a new audio-thread path and the owner's OK (02 §4).
- **R5 v1 is two codes on code v1's reserved version-15 escape:** a spoiler-free **scorecard** (about 50 symbols: the challenge, engine, scores, time, assists and a salted commitment to the answer), sent on finishing, then an **answer code** (the patch as a diff from the start; about 50 / 135 / 255 symbols at levels 1 / 3 / 5, measured independently by 02 and 03), swapped after both have played. Each side re-scores both answers on its own build, so claimed numbers never decide (03 §2).
- **Honest limits, stated in the UI:** a code proves what a patch scores, never who made it or how long it took. `enth match` passes 11–13 of 20 golden and GR1 codes in 6–12 s, and times are self-reported. The UI says "checked on your computer" for scores and "reported" for times (02 §5, 03 §3).
- **Plan:** R5a (duels on Recreate) ships first, then R4, in the free heavy slot while R3b keeps the other. About 28–47 agent-hours; roughly 12–18 h of wall time to R5a and 25–40 h to R4, calibrated on R2's 25 h against 9–13 planned (02 §8). Live duels wait for GR1 and the standalone app.

## Answers to the brief

**Axis 1, pedagogy and game design (01).**
- *Prior art:* what works is short and varied (SoundGym's 5-game workout, TrainYourEars' "correct it by ear"). Complaints: "pot luck" difficulty from random settings, repetition, abstract sine quizzes. People quit when gains are slow and invisible; no product shows progress in physical units, and R4 should (§1).
- *Intervals* are slow to learn. Practice plus passive listening between blocks worked and transferred to new timbres (Little 2019). Harmonic intervals *are* `osc.semi` stacks.
- *Absolute pitch in adults* is real but rare. Copy: "Most adults who train for about 20 hours learn to name a handful of notes; a few learn all twelve; most stop sooner."
- *Chords:* results are bimodal (about 70 % near chance on major/minor tone scrambles, Chubb 2013), so no content is ever gated on them. *Rhythm:* the syncopation index gives a ready ladder, and people tap tens of ms early. *EQ:* training each person's weak bands beat uniform training (Kim 2015), which is the case for weakness-targeted Review.
- *Synth-parameter ear training* has no controlled study; GR4's held-out design will be enth's own evidence.
- *Difficulty and spacing:* 3-down/1-up; items placed by measured JND distance, not random settings; the skill is the unit of spacing and the game its vehicle (5-item bursts move a skill's Leitner box).

**Axis 2, architecture (02).**
- *Homes:* `enth-music` at L1 (not L0 or L4, because same-layer edges are refused and Challenge must embed `EarSpec`); v3, ear rendering, the scorers and result codes in `enth-challenge`; the drill player in `enth-learn`; `session::{ear, duel}`; `editor/ear/` and a duel card behind the features `ear` and `duel` (§2).
- *Round types:* label choice, clip choice (today's `Choose`), A/B and 2AFC on a staircase, slider estimate, "match it" (an existing Recreate round) and tap-back. Answer keys are seeded integers, so ear results verify without rendering (§3).
- *Rendering:* `Performance` already takes any notes and polyphony 1–16 exists; a 10-question set renders in about 0.3 s. No host transport: stimuli carry their own tempo and count-in.
- *Versions:* an unknown kind or layout gives `code_too_new`, an unknown scorer set `scorer_too_new`, an op on a module this build lacks `answer_too_new`; another engine version re-scores and shows both numbers (§5).

**Axis 3, duels and sharing (03).**
- *What lasts without a server:* one identical puzzle, a spoiler-free share line, plain text over existing chats, low stakes. What killed formats: a server switched off (Heardle), an organiser who stopped (VCV's patch challenge), judging nobody could make fair (KVR's preset contest). Determinism gives enth the identical puzzle and the fair judge for free (§1).
- *Code UX:* Crockford base32 survives chat markdown; a 300-symbol code fits Discord, Telegram, WhatsApp and Signal, but not one SMS. Paste-anything decoding matters most (codes found inside a whole message, dash-like characters mapped, zero-width characters dropped); the clipboard is read only on an explicit Paste; emoji always come with words (§2).
- *Privacy:* no names, machine ids, absolute times or patch `meta` in a code; names stay optional and local. Today's 40-bit seed mask keeps a seed's creation time private (checked in `crates/enth-plugin/src/game.rs`) (§4).
- *Live play later:* mdns-sd (+0.8 MB) with plain TCP or quinn (+2.8 MB); libp2p (+9 MB) and iroh (+15 MB, company relays by default) are out. The OS is the obstacle: macOS's local-network permission and Windows' firewall rule attach to the DAW, not the plugin, and hole punching fails about 30 % of the time without a relay (§5).

## Recommendations

**R4 games, in order** (ladders and scoring in 01 §4):
1. **Spot the change:** A, then B with one control moved; pick it from 2–6 knob names. 16 items.
2. **Dial it in:** a one-knob Recreate round; the tolerance narrows from 4 to 1.5 JND.
3. **Ear check:** a 3-interval odd one out on 3-down/1-up, about 40 trials, giving a threshold in st, ms, ct or dB. For placement and every two weeks, not daily; GR4's instrument.
4. **Frequency finder:** svf bands and resonance now, EQ boost and cut on the biquad.
5. **Interval stacks:** harmonic first (`osc.semi`), then melodic two-note phrases with a roving root.
6. **Chord quality:** major/minor with a "some people plateau" note, then dim/aug, sus, sevenths.
7. **Rhythm pattern:** listen, pick 1 of 4, then mark a 16-step grid; judged exactly, with no input timing.

R4b: progressions (seeded phrases), note naming (opt-in, never in the default workout) and tap-back (after the TAP spike). A **workout** is 3 games × 12–20 items chosen by due skills, ending in "done for today". After each item you replay A, B and the answer; feedback talks about the sound, never the person. A calendar of practised days can't be "lost", and after a break you warm up one level down. Parameter games stay on one-note phrases, where `sound/1` can measure them; items in its deaf spots use parameter steps, are marked *not measured* and get a listening review.

**R5 v1 (async, no networking):**
- **Codes:** the scorecard and the answer code (fields under Corrections). The answer is `ops_between(start, answer)` with exact `f32` bits through a frozen, append-only `result-params-v1` table; `meta`, labels, macro names and slots are stripped.
- **Verification:** decode, regenerate the challenge, apply the ops and check the hash, score with the named scorer set, compare with the claim; under about 1 s. The receiver's own re-score of both answers decides.
- **UX:**
  - a share line ("enth duel · Level 3 · 93.4 % in 2:41 · 4 submits · 0 hints") plus the code, and paste-anything;
  - the friend's answer stays sealed until you've submitted or given up (a UI guard, not crypto); afterwards you can audition it or load it as one undo step, so the duel becomes a lesson;
  - a local duel history with optional names the receiver types;
  - a daily challenge per level from a frozen seed formula of the UTC day.
- **Cheating limits, said once in an About line:** "Duel codes are for friends. A code proves what a patch scores, not who made it or how long it took. enth's own tools can solve any challenge." Never "verified time", "proof" or "anti-cheat". A revealed target gives no scorecard; assists print on the card, like Wordle's hard-mode asterisk.

**Defer:**
- live duels: after GR1, in the standalone app first, LAN through mdns-sd plus a typed `IP:port` (which also covers friends' VPN overlays), no relay of ours;
- perfect-pitch promises;
- tap-along, which needs a calibration screen;
- MIDI answers, until the owner OKs the audio-thread path;
- QR result cards (encoders and `rqrr` checked; past v1), leaderboards, internet NAT traversal, op-log replays.

**Gates (proposed; the owner sets the criteria):**
- **GR4, mechanical half (CI, `ear-check`):**
  - 1,000 items per game regenerate identical hashes on the six golden targets;
  - every option pair is at least 2 JND apart (6 at L1), with parameter-step items listed apart;
  - an ideal-observer bot scores at least 95 % at the top level, and bias bots stay at chance ± 3 %;
  - simulated listeners' 40-trial thresholds land within one step in 90 % of runs;
  - every observable Basic-tier parameter is in game 1 or 2 (01 §5, 02 §6).
- **GR4, human half:**
  - the owner and at least two friends take Ear check on 4 skills, then two weeks of at least six short workouts on 2 of them, then Ear check again; the other 2 are held out, counterbalanced;
  - the owner's retest pilot sets the "real change" bar;
  - pass: the median person improves both trained skills beyond that bar and by more than the held-out ones; no trained skill gets worse on average; people play beyond what's required; everyone names something they now hear;
  - results are described, not tested (N = 3–6).
- **GR5:**
  - three async duels finish with both codes verified both ways, any score difference explained by a flagged engine difference;
  - codes survive a paste through Discord, Telegram, WhatsApp, Signal, SMS and iMessage, smart punctuation included;
  - a test shows no name, `meta` or absolute time in any code.

## Corrections to sub-reports

- **One result code (02) or two (03).** The coordinator takes 03's pair:
  - why: the scorecard spoils nothing, so you send it the moment you finish, and its salted commitment shows your answer was fixed before you saw your friend's;
  - from 02: the answer encoding (`ops_between`, the frozen parameter table, `answer_too_new`) and two fields 03's scorecard lacks, the scorer-set index (8 bits, so GR1's `sound/2` can't silently move a score) and the `closest` assist;
  - checks: 03's, which are right for long codes (a fixed length and about 30 check bits on the scorecard, a 40-bit hash tail on the answer); the F1e judge fixes the exact layout.
- **The docs disagree, and both change.** roadmap.md R5 says the result code "contains their patch". challenge.md (Codes) says result codes carry the fingerprint and the scores, never the patch, because "a patch is kilobytes". That reason no longer holds: a diff from the start is 50–310 symbols, and 02 and 03 agree within a few percent. The pair satisfies both readings, and both pages get the new wording.
- **Old builds, refined.** Checked in `code.rs`: `decode` refuses more than 64 symbols as `bad_length` before it reads anything else, then checks the GF(32) symbol (`bad_check`), then the version (`code_too_new`).
  - So an answer code always reads "bad length" on R1–R3 builds.
  - A short-spec scorecard (about 51 symbols) reads "code too new" if it **keeps code v1's GF(32) symbol last**, on top of its stronger check. Recommended; a long-spec scorecard is over 64 anyway.
- **Which submit a duel compares** is left open by both (03 §7). Default: the **first passing submit** (its patch, time to pass and submit count), otherwise the best submit marked "not passed". The session keeps only the latest `submitted_patch` today, so SE adds the patch at the first pass (additive). Question 4.
- **Staircase rule.** 02's drill player uses 2-down/1-up (70.7 %); 01's evidence puts training and measurement at about 80 %. Drills and Ear check use 3-down/1-up, and DRL's simulation test retargets to 79 %. Recreate keeps 2-down/1-up.
- **Streaks.** 02's drill player lists "streaks"; 01 and learn.md rule out visible streaks. The staircase's run counter stays internal.
- **The walking-skeleton game.** 02's F2e walks intervals end to end; 01 ranks sound-design games first. SKE walks **Spot the change and Interval stacks** (a v2 draw and a `Target::Ear`, both label choices), so game 1 and both target kinds reach the plugin together (about +0.5 h).
- **Rhythm doesn't all wait for TAP.** 02 puts the rhythm ladder after the TAP spike, but Rhythm pattern is listening-only and needs no input timing, so it joins G2 in wave 5. Only tap-back waits.
- **Ear duels.** 02 replays staircases from the answers, but adaptive sessions diverge, so two players would hear different items (01 §4). Ear duels use a **fixed ladder** (item *i*'s level depends only on *i*) and come after R4's games exist. Anyone can compute their answer keys from the code; the UI says so.
- **GR4.** 02's gate (the staircase level rises on two games over three sessions) would pass from test practice alone (Amitay 2006). 01's trained-versus-held-out design is adopted, with 02's `ear-check` as the mechanical half. It costs the owner about 2.5 h over two weeks instead of about 40 minutes (question 10).
- **Unowned pieces.** 02's plan has no lane for 01's workout composer, a `gr4-report` or 03's duel history. DRL takes the composer (pure, in `enth-learn`), INT4 the report (like `gr2-report`), DUI the history.
- **EQ timing.** 01 waits for S2's EQ; 02 builds the biquad now. The biquad comes now (lane BQ: light, about 1 h, a new file in `enth-dsp` with its own golden, no existing output changes). Frequency finder keeps its svf form too, which teaches the synth's own filter.
- **Owner time.** 02's "about 2–3 h" assumes its light GR4; with 01's design it is about 4–5 h over two weeks.

## Decisions (provisional)

| Area | Decision (the default unless the owner says otherwise) | Evidence | Status |
| --- | --- | --- | --- |
| R4 order | Spot the change, Dial it in, Ear check, Frequency finder, Interval stacks, Chord quality, Rhythm pattern; then progressions, note naming, tap-back | 01 §4 | Needs owner (Q1) |
| Perfect pitch | Opt-in note naming with honest copy, outside the default workout | 01 §2 | Needs owner (Q2) |
| Difficulty | 3-down/1-up (about 80 %) for drills and Ear check; items placed by measured JND distance; Recreate keeps 2-down/1-up | 01 §3 | Proposed |
| Sessions | 3 games, 10–15 min, a clear end; replay with the answer; thresholds in physical units; no streaks, leaderboards or currency | 01 §3 | Proposed |
| Spacing | Skill keys `module.param` and `ear.*`; 5-item bursts move Leitner boxes; due skills choose the games | 01 §3 | Proposed |
| Crate | `enth-music` at L1: theory, rhythm, ear generator v1 frozen by a golden | 02 §2 | Needs owner (Q9) |
| Challenge v3 | `Target::Ear(EarSpec)`, `AnswerKind::{Choice, Value, Taps}`, scorers `ear/1`, `ear-value/1`, `taps/1`; judged at F1e | 02 §2, §8 | Proposed |
| Drill player | In `enth-learn`: the staircase, ear skills into Review, `StepKind::Drill`, the workout composer | 02 §2, 01 §3 | Proposed |
| EQ games | Offline RBJ biquad in `enth-dsp` now; S2's EQ module wraps it | 02 §2–§3 | Needs owner (Q7) |
| Rhythm v1 | Listening-only in R4a; tap-back scored on inter-onset intervals after TAP; no host transport | 01 §2, 02 §4 | Proposed |
| Tap source | Editor taps stamped on arrival (fork patch 12) first; a host-MIDI ring only with the owner's OK | 02 §4 | Needs owner (Q8) |
| Result codes | Scorecard (about 50 symbols) + answer code (the patch as a diff) on the version-15 escape | 03 §2.4, 02 §5 | Needs owner (Q3) |
| Answer form | `ops_between` with exact `f32` bits via a frozen `result-params-v1` table; `meta`, labels, names stripped | 02 §5, 03 §2.4 | Proposed |
| Code checks | Scorecard: fixed length, about 30 check bits, code v1's GF(32) symbol last; answer: a 40-bit hash tail | 03 §2.3, this summary | Proposed (F1e judge) |
| Compared submit | The first passing submit; otherwise the best, marked "not passed" | 03 §7 | Needs owner (Q4) |
| Verification | The receiver re-scores both answers; an engine mismatch shows both numbers; refuse only on a generator or hash mismatch | 02 §5, 03 §3 | Proposed |
| Wording | "Checked on your computer" (scores), "reported" (time, assists); never "verified player", "proof", "anti-cheat" | 03 §3 | Proposed |
| Spoilers | A friend's answer sealed until you've played or given up | 02 §5, 03 §2.4 | Needs owner (Q6) |
| Paste and privacy | Paste-anything; clipboard read only on Paste; no names, ids, absolute times or `meta` in codes; seeds keep the 40-bit mask | 03 §2.2, §4 | Proposed |
| Daily challenge | One per level, from a frozen formula of the UTC day (top 40 bits) | 02 §5, 03 §1 | Needs owner (Q6) |
| Ear duels | A fixed ladder, after R4's games; computable keys said plainly | 01 §4 | Proposed |
| Order | R5a (Recreate duels) before R4 | 02 §8 | Needs owner (Q5) |
| Live duels | After GR1; standalone first; LAN, mdns-sd + TCP or quinn; no libp2p, iroh or relay | 03 §5 | Needs owner (Q12) |
| GR4 | CI half + trained-versus-held-out human half | 01 §5, 02 §6 | Needs owner (Q10) |
| GR5 | Three duels verified both ways; chat-paste survival; no personal data in codes | 02 §8, 03 §2 | Needs owner (Q11) |

## Open checks

| Check | Owner | When |
| --- | --- | --- |
| **Rhythm-input timing spike:** jitter of editor taps (frame-quantised; fork patch 12's arrival stamps) against host-MIDI offsets, on the real display, in REAPER, on Windows and macOS; whether the tap pad's focus grab works with fork patch 4 | FK4, then TAP; the owner's REAPER check | Waves 2 and 7 |
| **The audio-thread MIDI path** (note-on sample positions into a preallocated `rtrb` ring, under `assert_no_alloc`) breaks the brief's rule, so it waits for the owner's OK | Owner | Before wave 7 |
| **`bad_length` on older builds:** what R1–R3 builds print for a scorecard and an answer code (expected "code too new" and "bad length"); the friend sheet's "update enth" line | RC (test), INT5 (sheet) | F1e, wave 4 |
| **macOS local-network permission** for later live play: behaviour when the DAW lacks `NSLocalNetworkUsageDescription`; Windows' first-listen alert naming the DAW; Tokio unload in a plugin | A live-duel research session | Before any live-duel lane |
| Real answer sizes from passing answers (targets stood in; maybe 1.2–2× longer) | SKD | F2d |
| Paste survival across chat apps, iOS smart punctuation, phone data detectors underlining codes | INT5; the owner's duel check | Wave 4 |
| `sound/1`'s deaf spots in parameter games: the *not measured* list and its listening review | G4, ECK | Waves 3–4 |
| 3-down/1-up in simulation: 75–85 % success, thresholds within one step in 90 % of runs | DRL | Wave 2 |
| Every REAPER setup runs FX with the transport stopped (Target ▶ already relies on it) | SKE | F2e |
| The ear column's overflow at 1200 × 800 | EUI | Wave 6 |
| The daily formula and the UTC day boundary (as Review's, it turns at local midnight only in UTC) | RC | F1e |

## Questions for the owner

Answer by number; silence means the default. All defaults: `1: yes 2: yes 3: yes 4: yes 5: yes 6: yes 7: yes 8: editor first 9: crate 10: two weeks 11: yes 12: yes`.

1. R4's games in the order above, sound-design games first? *Default: yes.*
2. Perfect pitch only as opt-in note naming with honest copy? *Default: yes.*
3. Duels use a scorecard plus an answer code that carries the patch as a diff? *Default: yes.*
4. A duel compares each player's first passing submit, re-scored on the receiver's machine, with times shown as reported? *Default: yes.*
5. R5a (Recreate duels) before R4? *Default: yes.*
6. Hide a friend's answer until you've played, and a daily challenge per level? *Default: yes to both.*
7. EQ games now, on an offline biquad that S2's EQ reuses? *Default: yes.*
8. Tap input: editor stamps first; may agents add a host-MIDI path on the audio thread? *Default: editor first, no audio-thread path until you say yes.*
9. `enth-music` as a new L1 crate rather than a module in `enth-challenge`? *Default: the crate.*
10. GR4's human half: trained versus held-out skills over two weeks (about 2.5 h for you), or three sessions with a rising level (about 40 minutes)? *Default: two weeks.*
11. GR5 as proposed? *Default: yes.*
12. Live duels after GR1, standalone app first, LAN only, no relay? *Default: yes.*

## Docs to change (lane DOCS4)

- `docs/roadmap.md`:
  - a dated line at the top;
  - R4: the game order, "perfect-pitch drills" becomes opt-in note naming, GR4;
  - R5: "a result code that contains their patch and time" becomes a scorecard, then an answer code carrying the patch as a diff, re-scored by the receiver, times reported; GR5; live duels after GR1, standalone first.
- `docs/challenge.md`: Codes replaces "never the patch itself" with the kinds behind version 15 (scorecard, answer, ear challenge, ear result; lesson sibling reserved), what old builds print, and the frozen daily formula; Challenge v3 is planned (lane CV3).
- `docs/learn.md`: Review gains `ear.*` skills and bursts, closing the open item "ear questions as Review items"; `StepKind::Drill` and a `drills` progress field are planned.
- `docs/design.md` §9: the drills' 3-down/1-up beside Recreate's 2-down/1-up.
- `docs/architecture.md`: an `enth-music` L1 row; `enth-dsp`'s biquad; `enth-challenge`'s v3, `result`, `answer` and `verify`; `enth-learn`'s drill player; `enth-session`'s `ear` and `duel`; `enth-ui`'s `editor/ear/` and duel card; the shells' `ear` and `duel`; `ear-check` in `tools/lesson`.
- New pages:
  - `docs/phase-4.md`: lanes, freezes, acceptance, waves;
  - `docs/ear.md`: games, round types, the ear generator, the drill player, GR4;
  - `docs/duels.md`: codes, verification, wording, privacy, GR5.
- `docs/README.md`: the new rows. `xtask/layers.toml` and AGENTS.md's layer line (`enth-music = 1`) belong to S0e, not DOCS4.

## The lane plan (from 02 §8, adjusted)

Report 02 §8 is the plan, with this summary's adjustments.

- **Freezes:**
  - F0e: S0e creates `enth-music`, the stubs, the features `ear` and `duel`, and the reserved code kinds;
  - F1e: the MUS, CV3, RC, DRL and SE APIs, judged together by an Opus judge because every persisted form goes to friends, then lane F1E's fixes;
  - F2d: SKD copies a result after a pass and verifies it in a second instance;
  - F2e: SKE plays Spot the change and intervals end to end in the plugin and through `enth ear`.
- **Slots:** everything runs in heavy slot L, because slot S stays R3b's (ONX, PUIb, INTb, and TR needs the machine alone); at most four light lanes at once.
- **Waves:**
  - 0: DOCS4, now;
  - 1: S0e;
  - 2: FK4 (heavy) beside MUS, CV3, RC and SE, then DRL and BQ;
  - 3: SKD, with F1E, ECK and ladder drafts;
  - 4: DUI, then INT5: **R5a ships**;
  - 5: SKE, with G2 (now including Rhythm pattern) and G3;
  - 6: EUI;
  - 7: TAP and the tap-back ladder;
  - 8: INT4, owner build 6 and a friend round.
- **Size:**
  - 02 plans about 26–44 agent-hours; about 28–47 with this summary's additions (the second skeleton game, the workout composer, `gr4-report`);
  - R2 took about 25 h of wall time against 9–13 planned, which puts R5a about 12–18 h out and R4 25–40 h.
- **The owner's sittings:**
  - read this summary: 30–40 min;
  - optional taste check, one exported WAV set per game: 15 min;
  - one async duel with a friend over your own chat: 20–30 min (default: a two-instance REAPER test);
  - owner build 6, the Ear view with four games and taps in REAPER: 45–60 min;
  - GR4: about 2.5 h over two weeks on the default design.
