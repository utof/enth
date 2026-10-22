# Phase 4 — R4 Ear-training games and R5 Duels with friends

**Goal: games that train the ear sound design needs, and duels you can play with a friend over any chat, with no server.** R5a: async duels on Recreate, two codes on code v1's reserved version-15 escape (a spoiler-free scorecard, then an answer code carrying the patch as a diff), each re-scored by the receiver's own build. Its gate GR5: three duels verified both ways, codes that survive chat apps, no personal data in any code ([duels.md](duels.md#gr5)). R4: sound-design games first (Spot the change, Dial it in, Ear check, Frequency finder), then Interval stacks, Chord quality and Rhythm pattern, on a drill player whose ear skills feed Review. Its gate GR4: deterministic, fair and solvable games in CI, and you and your friends hear finer differences on trained skills than on held-out ones ([ear.md](ear.md#gr4)). The two are planned together because they share Challenge v3's judge, the session, the plugin's game worker, the editor, the root manifests and your time. **R5a ships first; both use heavy slot L, one heavy lane at a time, because slot S stays R3b's.**

Size: **about 28–47 agent-hours** (report 02's 26–44 plus the summary's additions: the second skeleton game, the workout composer, `gr4-report`). Critical paths: R5a's S0e → RC + SE → judge → SKD → DUI → INT5, about 6–11 agent-hours; R4's S0e → MUS + CV3 → judge → SKE → EUI → TAP → INT4, about 10.5–19, behind R5a's heavy lanes in the one slot. R2 was planned at 9–13 h of wall time and took about 25 h, so expect **about 12–18 h of wall time to R5a and 25–40 h to R4**. **Your sittings and the TAP spike set R4's end date.**

**Status (2026-10-22): planned; nothing is built.** The research session is done, and its decisions are provisional until you steer; work proceeds on the defaults meanwhile: [`_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/`](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md) (the summary wins where it corrects a report; report 02 §8 is the lane plan this page adapts). The designs of record are [ear.md](ear.md) and [duels.md](duels.md). [roadmap.md](roadmap.md), [challenge.md](challenge.md#version-15-the-kinds-behind-the-escape-r4-r5-planned), [learn.md](learn.md#ear-skills-bursts-and-drills-r4-planned-lane-drl), [design.md §9](design.md#9-randomizer-and-difficulty), [architecture.md](architecture.md) and [README.md](README.md) were updated the same day (lane DOCS4).

New in phase 4: `enth-music` (L1) is created; `enth-dsp` (L0) gains one new file, the biquad (lane BQ, the one exception to "no engine work": no existing output or golden changes); `enth-challenge` (L4) gains Challenge v3, the result codes, `verify`, paste-anything and the daily formula; `enth-learn` (L5) the drill player; `enth-session` (L6) `ear` and `duel`; `enth-ui` (L7) the Ear view and the duel card; `enth-plugin` and `enth-cli` (L8) `ear`, `duel`, `enth ear` and `enth duel`; `tools/lesson` gains `ear-check`; `vendor/egui-baseview` gains fork patch 12. No lane edits `enth-engine` or `enth-render`, and **no lane writes networking code**.

## What the research changed

Lanes should read these before the tables, because each changes what a lane builds.

**R5**

- **Two codes, not one.** A spoiler-free **scorecard** (about 51 symbols: the challenge, engine, scorer set, scores, time, assists and a salted commitment to the answer) is sent on finishing; an **answer code** (the patch as `ops_between(start, answer)` with exact `f32` bits through a frozen `result-params-v1` table; about 50, 135 and 255 symbols at levels 1, 3 and 5) is swapped after both have played ([duels.md](duels.md#the-two-codes)).
- **The receiver decides.** Each side re-scores both answers on its own build, so claimed numbers never decide; an engine mismatch shows both numbers, and only a generator or hash mismatch refuses.
- **Honest limits in the UI.** A code proves what a patch scores, never who made it or how long it took: `enth match` passes 11–13 of 20 golden and GR1 codes in 6–12 s, and times are self-reported. Scores read "checked on your computer", times "reported".
- **Old builds can't read the new codes,** and no later change fixes that: `decode` refuses more than 64 symbols as `bad_length` before it reads any field. So every version-15 kind keeps code v1's GF(32) symbol last, and short ones read "update enth" ([challenge.md](challenge.md#what-older-builds-print)).
- **Live duels wait** for GR1 and the standalone app: the macOS local-network permission and Windows' firewall rule attach to the DAW, not the plugin.

**R4**

- **Sound-design games first.** Musical training sharpens pitch but not brightness, so interval drills won't train a cutoff or decay ear; the sound-design games are also the cheapest (a one-item v2 draw plus a choice, or a one-knob Recreate round).
- **Train at about 80 %:** 3-down/1-up for drills and Ear check, items placed by measured JND distance; Recreate keeps 2-down/1-up.
- **Workouts end.** 3 games × about 4 minutes; progress as thresholds in physical units; no streaks, leaderboards, currency or perfect-pitch promises; note naming is opt-in and never in the default workout.
- **No engine change.** Every stimulus is an offline render played through the Target ▶ examples slot; a 10-question set renders in about 0.3 s.
- **The skill is the unit of spacing.** `ear.*` skills join Review's `module.param` boxes, and 5-item bursts in any game move a box, which closes learn.md's open item "ear questions as Review items".
- **Rhythm timing is the one real unknown.** The editor sees input once per frame (up to about 33 ms at 30 fps). Rhythm pattern is listening-only and ships in R4a; tap-back waits for lane TAP's spike; host-MIDI taps need a new audio-thread path and your OK.

## How the conflicts were resolved

The summary's corrections stand ([summary](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md#corrections-to-sub-reports)). This plan settles the rest, provisionally:

| Conflict | Resolution | By |
| --- | --- | --- |
| One result code (report 02) or two (report 03) | Two: the scorecard, then the answer code. From 02: the answer encoding, `answer_too_new`, the 8-bit scorer-set index and the `closest` assist; from 03: the checks (a fixed length and about 30 check bits on the scorecard, a 40-bit hash tail on the answer) | The summary |
| roadmap.md's result code "contains their patch"; challenge.md's "never the patch itself" | Both pages changed: the answer code carries the patch as a diff (50–310 symbols, not kilobytes) | The summary; DOCS4 |
| "An answer code always reads bad length on R1–R3 builds" (the summary) | Only above 64 symbols. A level-1 answer (about 50 symbols) passes the length check and would read `bad_check` in 31 of 32 cases. So every version-15 kind keeps code v1's GF(32) symbol last, not only the scorecard; then short codes read `code_too_new`, "update enth" (checked in `code.rs`) | This plan |
| Which submit a duel compares (report 03 §7) | The first passing submit, else the best marked "not passed"; SE adds the first pass's patch to the round. **The round record is saved within `ROUND_BYTES` (768 KiB) and already keeps one patch of up to 128 KiB,** so the second must fit that budget (for example as the answer's diff): a named F1e check | The summary (Q4); this plan |
| Report 02's 2-down/1-up drills; report 01's 80 % | 3-down/1-up for drills and Ear check; DRL's simulation retargets to 79 %; Recreate keeps 2-down/1-up | The summary |
| "Streaks" in report 02's drill player | No visible streaks; the run counter stays internal | The summary |
| The walking skeleton walks intervals (report 02) | SKE walks **Spot the change and Interval stacks** (a v2 draw and a `Target::Ear`, both label choices), about +0.5 h | The summary |
| The rhythm ladder after TAP (report 02) | Rhythm pattern is listening-only and joins G2 in wave 5; only tap-back waits | The summary |
| Ear duels replay staircases (report 02) | A fixed ladder, after R4's games | The summary |
| GR4: the level rises over three sessions (report 02) | Trained against held-out skills over two weeks (report 01), with `ear-check` as the mechanical half; about 2.5 h of yours instead of about 40 minutes (Q10) | The summary |
| No lane for the workout composer, `gr4-report` or the duel history | DRL, INT4 and DUI | The summary |
| EQ waits for S2 (report 01); a biquad now (report 02) | Lane BQ now: light, about 1 h, a new file in `enth-dsp` with its own golden; Frequency finder keeps its svf form | The summary (Q7) |
| Report 02's owner build 5 for R4 | **Owner build 6:** owner build 5 is R3a's create build. R5a has no owner build of its own, so INT5's sheet is owner sheet 6 and INT4's (with owner build 6) owner sheet 7 | This plan |
| Phase 3's rule "no lane edits `enth-dsp`" | Holds for every lane but BQ, whose one new file changes no existing output or golden | This plan |
| Where the parameter games' items live: `enth-music` (L1) cannot read the catalogue (`enth-patch` is L1 too) | In `enth-challenge` on v2 draws (CV3; G4 tunes the ladder); `enth-music` holds the music games | This plan |
| MUS freezes ear generator v1 with a golden, but G1–G3 tune its ladders later | It freezes at INT4's friend build, the first that shares ear codes; until then a ladder change updates the golden on purpose, and the commit names the games | This plan |
| DRL starts after the four API lanes (at most four light lanes at once), but its API is judged at F1e | F1e waits for DRL's API; DRL starts as soon as one of MUS, CV3, RC and SE finishes | This plan |

## Freezes

R5a and R4 share one spine to F1e, then fork: F2d is R5a's skeleton, F2e R4's. **After F1e (lane F1E's merge), the persisted forms change only additively** (Challenge v3, the scorecard, the answer code, the ear codes, `result-params-v1`, the daily formula, `StepKind::Drill`, the `drills` field); **after F2d and F2e, the challenge, learn and session Rust types too** (SKD and SKE may still adjust APIs the real plugin needs). Ear generator v1 freezes at INT4's friend build ([above](#how-the-conflicts-were-resolved)).

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0e** | S0e: `enth-music` created (`enth-music = 1` in `xtask/layers.toml` and AGENTS.md's layer line), every R4 and R5 file pre-declared, the features `ear` and `duel`, the Ear header hook and the duel card's seam, `sizes/ear.rs` and `sizes/duel.rs`, the version-15 kinds reserved, an `ear.yml` stub | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)), judged by exit status; `wasm-check` covers `enth-music`; the challenge, state and progress fixtures unchanged |
| **F1e** | MUS's API, CV3, RC, DRL's API and SE: Challenge v3, the scorecard and answer code, the ear codes, `result-params-v1`, `verify`, the daily formula, the drill player, `StepKind::Drill`, the `drills` field, `session::{ear, duel}` | **A judge agent (Opus)** reviews them together, because every persisted form goes to friends. Named checks: the size budget (a scorecard at a fixed length; answers at levels 1–5 against the measured sizes); privacy (no name, `meta`, label or absolute time survives encoding); refusals (`code_too_new`, `scorer_too_new`, `answer_too_new`); every kind through code v1's decoder; the first pass's patch within `ROUND_BYTES`; the daily formula's golden. Then lane F1E closes the must-fix items |
| **F2d** | SKD: copy a result after a pass, paste it into a second instance and verify it | The REAPER `duel` scenario (two instances); `validate`, `host-render`, `leak-test` |
| **F2e** | SKE: Spot the change and Interval stacks end to end in the plugin and through `enth ear` | `ear-check` green on both; the REAPER `ear` scenario; `validate`, `host-render`, `leak-test` |

**S0e touches the root manifests (`Cargo.toml`, `Cargo.lock`, `xtask/layers.toml`), so it waits until no R3b lane has root-manifest edits in flight.** R4 and R5a need no new external dependency (no networking, no QR); after S0e the coordinator alone edits `[workspace.dependencies]`, `Cargo.lock` and `[patch.crates-io]`, and a lane that needs a dependency asks.

## Lanes

Sizes are agent-hours (report 02 §8's estimates, calibrated on R2's; sizes this plan added are marked). H = heavy local build (nice-plug, egui or a vendored crate), L = light. Every file in the Owns column is planned: S0e pre-declares it as a compiling stub, or the lane creates it, unless it exists today (`game.rs`, `review.rs`, `lesson.rs`, `progress.rs`, `tools/lesson`).

**The spine (both milestones)**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **DOCS4** Docs | This page, [ear.md](ear.md), [duels.md](duels.md), and the summary's doc rows in roadmap, challenge, learn, design, architecture and README | The summary | 0.75–1 | L |
| **S0e** Workspace prep (→ F0e) | Creates `enth-music` (`enth-music = 1` in `xtask/layers.toml`, and the layer line in [AGENTS.md](../AGENTS.md)); pre-declares every file below: `enth-music`'s `theory`, `rhythm`, `ear` and `ladder/`; `enth-challenge`'s `v3`, `ear.rs` and `ear/`, `result.rs`, `answer.rs`, `verify.rs`, `paste.rs`, `daily.rs`; `enth-dsp/src/biquad.rs`; `enth-learn`'s `drill.rs` and `workout.rs`; `enth-session`'s `ear.rs` and `duel.rs`; `enth-ui`'s `editor/ear/`, `editor/duel/`, `sizes/ear.rs`, `sizes/duel.rs`, the Ear header hook and the duel card's seam; `enth-plugin`'s `ear.rs` and `duel.rs`; `enth-cli`'s `cmd/ear.rs` and `cmd/duel.rs`; `tools/lesson/src/ear/`; `xtask/src/ear.rs` and `xtask/src/gr4.rs`. Reserves the version-15 kind numbers (constants in `result.rs`); the features `ear` and `duel` in `enth-ui` and `enth-plugin`; an `ear.yml` stub | DOCS4; no R3b root-manifest edits in flight | 0.5–1 | **H** once |
| **MUS** Music (→ F1e) | `enth-music`: interval, chord, scale and progression tables, the 16-step rhythm grid and the syncopation measure, note names, **ear generator v1** and its golden; level 1 of each ladder file | F0e | 1.5–3 | L |
| **CV3** Challenge v3 (→ F1e) | `enth-challenge`: v3 (`Target::Ear`, `AnswerKind::{Choice, Value, Taps}`, v1 and v2 read as before), `ear.rs` and `ear/` (a question → a timbre patch, a `Performance`, the post-EQ band; the parameter games' items on v2 draws), `timbres-v1/`, the scorers `ear/1`, `ear-value/1`, `taps/1`, the ear-challenge kind; [challenge.md](challenge.md#challenge-v3-planned-lane-cv3-r4)'s v3 section | F0e; MUS's API | 1.5–2 | L |
| **RC** Result codes (→ F1e) | `enth-challenge/src/{result,answer,verify,paste,daily}.rs`: the scorecard, the answer code, the ear result, `result-params-v1`, `verify`, paste-anything, the daily formula; challenge.md's version-15 sections and [duels.md](duels.md)'s code sections as built | F0e | 1.5–2.5 | L |
| **SE** Session (→ F1e) | `enth-session/src/{ear,duel}.rs`; the first passing submit's patch in `game.rs` (additive, within `ROUND_BYTES`); [session.md](session.md)'s Ear and Duel sections | F0e | 1.5–2 | L |
| **DRL** Drill player (→ F1e) | `enth-learn/src/{drill,workout}.rs` (the 3-down/1-up staircase, the workout composer); additive in `review.rs` (`ear.*` skills, bursts), `lesson.rs` (`StepKind::Drill`) and `progress.rs` (the `drills` field); [learn.md](learn.md#ear-skills-bursts-and-drills-r4-planned-lane-drl)'s section | F0e; starts when one of MUS, CV3, RC and SE finishes; its API joins F1e | 1.5–3 (this plan adds 0.5–1 for the composer) | L |
| **BQ** Biquad | `enth-dsp/src/biquad.rs` (an offline RBJ peaking and shelf biquad, coefficients through `math`), its golden, `docs/dsp/biquad.md` | F0e | 0.5–1 | L |
| **F1E** The F1e judge's findings | Its must-fix items, in the files they name | The judge | 0.5–1.5 | L |

**R5a — Duels**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **SKD** Duel skeleton (→ F2d) | `enth-plugin/src/duel.rs` (duel jobs on the game worker, the clipboard written on a click and read only on Paste), a minimal card in `editor/duel/`, `enth duel result\|verify` (`enth-cli/src/cmd/duel.rs`), the REAPER `duel` scenario (additive in `xtask/src/reaper/`), [plugin.md](plugin.md)'s and [agent-cli.md](agent-cli.md)'s duel sections | F1e | 1.5–2.5 | **H** |
| **DUI** Duel card | `enth-ui/src/editor/duel/*` (the share line, the paste box, verification, side by side, the sealed answer and loading it as one undo step, the duel history, the About line), `sizes/duel.rs`; the history file in `enth-plugin/src/duel/` (additive after SKD); [editor.md](editor.md)'s duel section | F2d | 1–2 | **H** |
| **INT5** R5a ships | `docs/r5-gates.md` (GR5's table), `docs/r5-owner.md` (owner sheet 6: the duel check), the friend duel sheet `docs/duel-friends.md`, a friend build; duels.md's status | DUI | 1–1.5 | **H** |

**R4 — Ear games**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **FK4** Fork patch 12 | `vendor/egui-baseview`: an arrival instant per baseview event, and what a tap pad needs to take keyboard focus; `vendor/README.md`, the ADR-0001 note | F0e (slot L, while the API lanes run) | 0.5–1 | **H**, small |
| **SKE** Ear skeleton (→ F2e) | `enth-plugin/src/ear.rs` (ear jobs on the game worker, examples at the host's rate, the "play from the top" flag, additive in `game.rs`'s `TargetLoop`), a minimal panel in `editor/ear/`, `enth ear` (`enth-cli/src/cmd/ear.rs`), the REAPER `ear` scenario; plugin.md's and agent-cli.md's ear sections | F1e | 2.5–4 (this plan adds 0.5 for the second game) | **H** |
| **ECK** Ear check | `tools/lesson/src/ear/` (oracle, random, bias and deaf players; audibility per level; the staircase simulations; the golden check), `xtask/src/ear.rs` (`cargo xtask ear-check`), `.github/workflows/ear.yml`, the `gate.sh lane` hook for `enth-music` and the ear files; [ear.md](ear.md)'s `ear-check` section | F1e | 1–1.5 | L |
| **G1** Intervals and pitch | `enth-music/src/ladder/{intervals,pitch}.rs`: Interval stacks; R4b's note naming | ECK | 1–1.5 | L, Opus |
| **G2** Chords, progressions, rhythm | `enth-music/src/ladder/{chords,progressions,rhythm}.rs`: Chord quality, Rhythm pattern; R4b's progressions | ECK | 1–1.5 | L, Opus |
| **G3** Bands | `enth-music/src/ladder/bands.rs`: Frequency finder's svf and EQ forms | ECK, BQ | 1–1.5 | L, Opus |
| **G4** Parameters | The parameter games' ladder in `enth-challenge/src/ear/` (Spot the change, Dial it in, Ear check) and its *not measured* list | ECK | 1–1.5 | L, Opus |
| **EUI** Ear view | `enth-ui/src/editor/ear/*` (every round type, the workout, the replay with the answer, thresholds per skill, the practised-days calendar, the warm-up after a break), `sizes/ear.rs`; editor.md's Ear section | F2e | 2–3.5 | **H** |
| **TAP** Taps | The spike (a jitter table of editor stamps against host MIDI under Xvfb, then on your display in REAPER), then the chosen path: editor stamps and the tap pad; the host-MIDI ring in `process.rs` only with your OK; the tap-back round and its ladder (`enth-music/src/ladder/taps.rs`) | EUI, FK4 | 1.5–2.5 | **H** |
| **EV4** Agent tracks | The ear and duel tracks in `content/agent-evals/`; the rest of `enth ear` and `enth duel` (additive in SKE's and SKD's files) | SKD's CLI (duel), SKE's CLI (ear) | 0.75–1.5 | L |
| **INT4** R4 ships | `docs/r4-gates.md` (GR4's table), `docs/r4-owner.md` (owner sheet 7: owner build 6 and GR4), the friend sheet `docs/gr4-friends.md`, `cargo xtask gr4-report` (`xtask/src/gr4.rs`, like `gr2-report`), a friend build; ear.md's status | Everything | 2–3.5 (this plan adds 0.5–1 for `gr4-report`) | **H** |

**Totals:** about 28–47 agent-hours: the spine about 10–17, R5a 3.5–6, R4 about 14–23.5 (the ladders 4–6 of it).

## Waves

**One heavy slot, L.** Slot S stays R3b's (ONX, PUIb, INTb; TR uses the machine alone), so a heavy lane here waits for slot L to be free, and when TR runs overnight slot L pauses too. Otherwise at most four light lanes build at once; docs-only lanes (DOCS4) don't count. When a wave has more light lanes than room, the freeze-path lanes go first (in wave 2: MUS, CV3, RC and SE, then DRL and BQ).

| Wave | Slot L (heavy) | Light | Starts when |
| --- | --- | --- | --- |
| 0 | — | DOCS4 | The summary is sent (don't wait for your reading) |
| 1 | S0e (→ F0e) | — | DOCS4 merged; no R3b root-manifest edits in flight |
| 2 | FK4 | MUS, CV3, RC, SE (→ F1e, judged); then DRL, BQ | F0e |
| 3 | SKD (→ F2d) | F1E; ECK; ladder drafts (G1–G4) | F1e |
| 4 | DUI, then INT5: **R5a ships** | G1, G4, EV4 (duel) | F2d |
| 5 | SKE (→ F2e) | G2 (with Rhythm pattern), G3 | INT5 (or F1e, if you want R4 first: Q5) |
| 6 | EUI | EV4 (ear) | F2e |
| 7 | TAP | The tap-back ladder | EUI and FK4; your answer on host MIDI (Q8), or editor stamps alone |
| 8 | INT4, then owner build 6 and a friend round | — | Everything merged |

- **The machine alone:** INT5, INT4 and TAP's timing runs take `flock -x /tmp/enth-quiet.lock` for REAPER, the soak and timing steps, and never overlap R3b's INTb or a training run.
- **Critical paths:** R5a's S0e (0.5–1) → RC + SE (1.5–2.5) → judge and F1E (0.5–1.5) → SKD (1.5–2.5) → DUI (1–2) → INT5 (1–1.5), about 6–11 agent-hours. R4's S0e (0.5–1) → MUS + CV3 (1.5–3) → judge and F1E (0.5–1.5) → SKE (2.5–4) → EUI (2–3.5) → TAP (1.5–2.5) → INT4 (2–3.5), about 10.5–19, behind R5a's heavy lanes. Your sittings and your friends' come on top, and they usually set the end date.

## Acceptance per lane

Each lane stops at its own criteria and reports against them.

| Lane | Acceptance |
| --- | --- |
| **DOCS4** | Every "Docs to change" row of the summary applied; every relative link and anchor it wrote resolves; file paths it cites exist or are marked planned |
| **S0e** | The Done-means gate is green. `wasm-check` covers `enth-music`. Every stub compiles. The challenge, state and progress fixtures and every golden are unchanged |
| **MUS** | Proptests: every generated question is valid (C1–C7, at most 8 notes at once, at most 10 s, integers only), every answer key is one of its options. Ear generator v1's golden (1,000 specs → a hash) on Linux, Windows and wasm |
| **CV3** | v1 and v2 fixtures read and are written byte for byte as before; v3 fixtures; an ear target's strictness (phrase 0, the template start); the golden challenges and generator v1's golden unchanged; ear scorers judge with no render |
| **RC** | Round trips (proptest); every single substitution and adjacent swap caught on every kind, and a truncation caught; frozen fixtures with a pinned SHA; a size-budget test per level; no name, `meta`, label, macro name, slot or absolute time survives (a patch with them set); the refusals; every kind through code v1's decoder (`code_too_new` at 64 symbols or fewer, `bad_length` above); paste-anything on a corpus of mangled pastes; the daily formula's golden |
| **SE** | Ear and duel lifecycles; stale results refused; the first pass's patch kept and within `ROUND_BYTES` (the largest-round test still passes); no audio-thread contact under `assert_no_alloc` |
| **DRL** | The staircase against simulated listeners: 75–85 % success, thresholds within one step in 90 % of 40-trial runs; bursts move boxes (proptests, as REV's); `Progress::merge`'s laws hold with `drills`; the composer gives the same workout for the same progress file and day |
| **BQ** | The same bits on the six golden targets; within 0.1 dB of the cookbook's response; every existing golden unchanged |
| **F1E** | Every must-fix closed |
| **SKD** | The F2d row; real answer sizes measured on passing answers |
| **DUI** | kittest by label: copy, paste a whole chat message, verify, the sealed answer opening only after a submit or give-up, a load as one undo step; no clipboard read without Paste (a test); 0 idle frames; `leak-test` |
| **INT5** | The full gate, REAPER and leak green; GR5's automated rows filled; an R3 build's answer to each kind recorded; the friend duel sheet and owner sheet 6 written |
| **FK4** | Arrival stamps under Xvfb; the leak gate unchanged; [vendor/README.md](../vendor/README.md) lists patch 12 |
| **SKE** | The F2e row |
| **ECK** | Catches a seeded bad game of each kind (a leak a bias bot finds, an inaudible pair, an unsolvable top level, a drifting hash); GR4's mechanical bullets; under 10 minutes in CI |
| **G1–G4** | `ear-check` green on their games; every level's distances listed, the *not measured* items apart |
| **EUI** | Each round type by label in kittest; a workout to its end; the overflow report at 1200 × 800; 0 idle frames; `leak-test` |
| **TAP** | The jitter table; exact MIDI offsets in the test host if the MIDI path is OKed; your REAPER check |
| **EV4** | `enth ear` and `enth duel` with insta snapshots; the tracks recorded with model, date and cost (paid runs only with your OK) |
| **INT4** | The full gate, REAPER, leak and soak green; GR4's mechanical rows filled; `gr4-report` on a scripted fixture; owner sheet 7 and the friend sheet written |

## Your part

| What | Milestone | Time | When | Default if you don't answer |
| --- | --- | --- | --- | --- |
| Read the summary; answer its 12 questions by number | Both | 30–40 min | Now | `1: yes 2: yes 3: yes 4: yes 5: yes 6: yes 7: yes 8: editor first 9: crate 10: two weeks 11: yes 12: yes` |
| Host MIDI on the audio thread (Q8) | R4 | 2 min | Before wave 7 | No: editor stamps only |
| Optional taste check: one exported WAV set per game (agents export them and never play them) | R4 | 15 min | Waves 3–5, on the ladder drafts | Report 01's ladders and `ear-check` |
| Duel check: one async duel with a friend over your own chat (owner sheet 6) | R5 | 20–30 min, plus a friend | After INT5 | A two-instance REAPER test stands in; GR5's duel row waits |
| GR5's and GR4's criteria (Q10, Q11) | Both | 5 min | Before INT5 and INT4 | The proposed criteria ([duels.md](duels.md#gr5), [ear.md](ear.md#gr4)) |
| Owner build 6: the Ear view, four games, taps in REAPER (keys, mouse, MIDI if you have it) (owner sheet 7) | R4 | 45–60 min | After INT4 | GR4 waits; editor stamps |
| GR4: the retest pilot, the pre-test, two weeks of at least six short workouts, the post-test, the debrief | R4 | About 2.5 h over two weeks | After owner build 6 | GR4 waits; report 01's design and criteria |

About 4–5 h over two weeks, after your earlier sheets ([owner sheet 4](r2-owner.md) and R3a's). **Your friends:** one plays a duel with you (20–30 min); at least two take GR4's two weeks (about 2.5 h each). Each sends files back by choice; nothing leaves anyone's machine on its own.

Until you say otherwise: **R5a ships before R4; duels are a scorecard plus an answer code, compared on the first passing submit and re-scored by the receiver; a friend's answer stays sealed until you've played; there is a daily challenge per level; the games run in the order above, with note naming opt-in; EQ games use the offline biquad now; taps are editor stamps with no audio-thread path; `enth-music` is its own crate; GR4 is the two-week trained-against-held-out design; live duels wait for GR1 and the standalone app.**

## The GR4 and GR5 checks

The tables of record will be `docs/r5-gates.md` (INT5) and `docs/r4-gates.md` (INT4), both planned. The protocols are in [duels.md](duels.md#gr5) and [ear.md](ear.md#gr4).

| Gate | Check | How | Who | When |
| --- | --- | --- | --- | --- |
| GR5 | Three async duels finish with both codes verified both ways; any score difference explained by a flagged engine difference | Real duels through your own chat; a two-instance REAPER test stands in | **You** and a friend; INT5 | After INT5 |
| GR5 | Codes survive chat apps (Discord, Telegram, WhatsApp, Signal, SMS, iMessage), smart punctuation included | A corpus test of mangled pastes in CI; your duel check on real apps | RC, INT5; **you** | F1e; after INT5 |
| GR5 | No personal data in any code | A test on every kind with `meta.author`, labels and macro names set | RC | F1e |
| GR5 | The receiver's score decides | The 25 golden challenges: target → result → verify gives the same score bits on Linux, Windows and wasm | RC, INT5 | F1e; wave 4 |
| R5a | Ships | The full gate, REAPER, leak; GR5's automated rows; the friend build and sheet | INT5 | Wave 4 |
| GR4 | Deterministic | 1,000 items per game regenerate identical hashes on the six golden targets | ECK, MUS | CI from wave 3 |
| GR4 | Fair | Every option pair at least 2 JND apart under `sound/1` (6 at L1), parameter-step items listed apart; bias bots at chance ± 3 % over 10,000 items | ECK, G1–G4 | CI from wave 3 |
| GR4 | Solvable | An ideal-observer bot at 95 % or more at the top level; simulated listeners' 40-trial thresholds within one step in 90 % of runs | ECK, DRL | CI from wave 3 |
| GR4 | Covers the synth | Every observable Basic-tier parameter in game 1 or 2; bursts move Review boxes | ECK, G4, DRL | CI from wave 4 |
| GR4 | You and friends hear finer differences on trained skills | The retest pilot sets the bar; 4 skills, 2 trained and 2 held out, counterbalanced; two weeks; described, not tested (N = 3–6) | INT4's `gr4-report`; **you and at least two friends** | After owner build 6 |
| R4 | Ships | The full gate, REAPER, leak, soak; GR4's mechanical rows; the friend build | INT4 | Wave 8 |

## Rules for every lane

- Branch from the latest freeze, and write only the files your lane owns. One owner per doc: [ear.md](ear.md) (DOCS4, then a section each for MUS, CV3, BQ, DRL, ECK, EUI and TAP; INT4 its status), [duels.md](duels.md) (DOCS4, then RC, SE, SKD and DUI; INT5 its status), [challenge.md](challenge.md)'s v3 section (CV3) and version-15 sections (RC), [learn.md](learn.md)'s drill section (DRL), [session.md](session.md)'s Ear and Duel sections (SE), [editor.md](editor.md)'s Ear (EUI) and Duel (DUI) sections, [plugin.md](plugin.md)'s Ear (SKE) and Duel (SKD) sections, [agent-cli.md](agent-cli.md) (SKE's and EV4's `enth ear`, SKD's `enth duel`), `docs/dsp/biquad.md` (BQ), the gate tables and owner sheets (INT5, INT4). Report against your acceptance criteria when you stop.
- **Privacy.** Never put your email, name or other personal details in any request, header, URL or payload; use `enth-research/0.1`. **Codes carry no personal data** (no names, machine ids, absolute times or patch `meta`), and RC's test proves it.
- **No windows on the real display** (Xvfb, as [AGENTS.md](../AGENTS.md#ui) says), and **never play audio**: the taste check's WAVs are exported for you, never played by an agent.
- **Foreground commands** with a timeout above their measured length; a backgrounded agent can go idle and never resume. Keep a running `notes.md` in persistent scratch (`~/code/enth-scratch/r45/<lane>/`).
- **Concurrency.** One heavy slot, L ([Waves](#waves)); 15 GiB of RAM is shared with R3b's lanes. `CARGO_BUILD_JOBS` as the coordinator's lane rules say, a separate `CARGO_TARGET_DIR` per lane on ext4, `flock -s` around builds and `flock -x` around timing runs. Build only the crates you need until the final gate.
- **A two-tier gate, judged by exit status only** (no `cmd | tail && echo ok`): `scripts/gate.sh lane <worktree>` before a lane reports, `gate.sh full` (the coordinator's) before a merge. A docs-only lane checks its links and anchors too.
- **No engine work, except BQ's one new file.** No lane edits `enth-engine` or `enth-render`, and no existing engine golden may change. **Nothing new touches the audio thread** except TAP's host-MIDI ring, and only after your OK.
- **No networking code** in R4 or R5a: no sockets, no HTTP, no QR. The clipboard is written on a click and read only on an explicit Paste. Live duels get their own research session after GR1.
- **Honest product rules:** "checked on your computer" for scores and "reported" for times, never "proof", "verified time", "verified player" or "anti-cheat"; no streaks, leaderboards, currency or loss messages; no content gated on chords, intervals or note naming; note naming never in the default workout.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshots per layout.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `Cargo.lock`, `[workspace.dependencies]`, `[patch.crates-io]` | S0e (the new member); afterwards the coordinator only. No new external dependency is planned |
  | `xtask/layers.toml`, AGENTS.md's layer line | S0e (`enth-music = 1`) |
  | `enth-dsp` | BQ only: `biquad.rs` and its export in `lib.rs` (pre-declared by S0e). No existing file's output changes |
  | `enth-music` | MUS; then G1–G3 their ladder files and TAP `ladder/taps.rs`. A golden update names the games and why, until INT4 freezes the generator |
  | `enth-challenge` | CV3 (v3, `ear.rs` and `ear/`, `timbres-v1/`, `score/`'s ear scorers), RC (`result.rs`, `answer.rs`, `verify.rs`, `paste.rs`, `daily.rs`), G4 (the parameter ladder in `ear/`). `code.rs` stays code v1, frozen: a needed `pub` (the GF(32) check) is additive and reported. `v2.rs` additive only |
  | `enth-learn` | DRL: `drill.rs`, `workout.rs`, additive in `review.rs`, `lesson.rs` and `progress.rs` (R2's lanes are done) |
  | `enth-session` | SE: `ear.rs`, `duel.rs`, the first pass's patch in `game.rs` (additive); `lib.rs` exports pre-declared by S0e |
  | The state blob | No new field: an ear session is not saved per project (like a create job), and duels live in the per-user history. SE's first-pass patch fits the round's existing budget |
  | `enth-plugin` | SKD (`duel.rs`), DUI (the history under `duel/`), SKE (`ear.rs`, the restart flag in `game.rs`'s `TargetLoop`, additive), TAP (the tap path; `process.rs` only with your OK). Ear and duel jobs run on the game worker. S0e pre-declares `ear.rs`, `duel.rs` and their hooks in `lib.rs` and `editor.rs` |
  | `enth-ui`'s editor | S0e declares the Ear header hook, the duel card's seam, `editor/ear/` and `editor/duel/`; EUI and DUI fill their own (SKD and SKE their minimal panels first). Tokens only in `sizes/ear.rs` and `sizes/duel.rs`. The Ear and Create columns are exclusive |
  | `enth-cli` | SKD (`cmd/duel.rs`), SKE (`cmd/ear.rs`), EV4 additively in both |
  | `xtask/src/` | S0e pre-declares `ear.rs` (ECK) and `gr4.rs` (INT4); SKD and SKE add the `duel` and `ear` REAPER scenarios additively |
  | `tools/lesson` | ECK (`src/ear/`), additively |
  | `content/` | EV4 (`agent-evals/`'s ear and duel tracks) |
  | CI workflows | S0e stubs `ear.yml`, ECK fills it |
  | `scripts/gate.sh` | ECK adds the `ear-check` hook for `enth-music` and the ear files, reviewed by the coordinator |
  | `vendor/` | FK4 only (fork patch 12) |
  | REAPER and timing | INT5, INT4 and TAP take `flock -x`, never alongside R3b's INTb or a training run |

- **Ownership crossings** are additive only and listed in the report. Earlier phases each grew an unplanned lane; F1E is planned up front.
- **Risks** (report 02 §7 and report 01 §6 have the full lists, each with its cheapest retirement):
  1. tap jitter (listening-only rhythm first; interval scoring; the TAP spike; your REAPER check);
  2. scope: six round types × five families × ladders (R4a first; one ladder lane per family);
  3. `sound/1`'s deaf spots in parameter games (parameter steps, a *not measured* list, a listening review);
  4. easy cheating (honest wording, labelled assists, sealed answers, the social model);
  5. old builds reading "bad length" (code v1's GF(32) symbol on every kind; the friend sheet);
  6. scorer drift after GR1's `sound/2` (the scorer set in every code);
  7. format sprawl: v3, four code kinds, the parameter table, the daily formula, the `drills` field, `StepKind::Drill` (one judge at F1e, fixtures);
  8. one heavy slot (R5a first, FK4 small, heavy lanes in series);
  9. S2's EQ colliding with the biquad (BQ owns `biquad.rs`; S2 wraps it);
  10. your ears and your time (the mechanical half in CI, short sittings, defaults everywhere).

## Deferred

The summary's list, provisional until you confirm: live duels (after GR1, the standalone app first, LAN through mdns-sd plus a typed `IP:port`, no relay of ours); perfect-pitch promises; tap-along (it needs a calibration screen); MIDI answers until you OK the audio-thread path; QR result cards, leaderboards, internet NAT traversal and op-log replays.

## Open checks

Not settled by the research; each is owned by a lane, by you or by a later milestone.

| Check | Owner | When |
| --- | --- | --- |
| **The rhythm-input timing spike:** the jitter of editor taps (frame-quantised; fork patch 12's arrival stamps) against host-MIDI offsets, on the real display, in REAPER, on Windows and macOS; whether the tap pad's focus grab works with fork patch 4 | FK4, then TAP; your REAPER check | Waves 2 and 7 |
| **The audio-thread MIDI path** (note-on sample positions into a preallocated `rtrb` ring, under `assert_no_alloc`) breaks the brief's rule, so it waits for your OK (Q8) | You | Before wave 7 |
| **What R1–R3 builds print** for each kind: expected `code_too_new` ("update enth") for codes of 64 symbols or fewer that end with code v1's GF(32) symbol, and `bad_length` above 64 ([challenge.md](challenge.md#what-older-builds-print)); without that symbol a level-1 answer would read `bad_check`; the friend sheet's "update enth" line | RC (the test), INT5 (the sheet) | F1e, wave 4 |
| **Where the first pass's patch is kept:** the round record is saved within `ROUND_BYTES` (768 KiB) and already keeps the last submit's patch (up to 128 KiB), so a second patch must fit the budget (as the answer's diff, say) | SE; the F1e judge | F1e |
| **When ear generator v1 freezes:** default at INT4's friend build; until then the ladder lanes update its golden on purpose | MUS, G1–G3; INT4 | Wave 8 |
| Real answer sizes from passing answers (the measured sizes used targets as stand-ins; real ones may be 1.2–2× longer) | SKD | F2d |
| Paste survival across chat apps, iOS smart punctuation, phone data detectors underlining codes | INT5; your duel check | Wave 4 |
| `sound/1`'s deaf spots in the parameter games: the *not measured* list and its listening review | G4, ECK | Waves 3–4 |
| 3-down/1-up in simulation: 75–85 % success, thresholds within one step in 90 % of runs | DRL | Wave 2 |
| Every REAPER setup runs effects with the transport stopped (Target ▶ already relies on it) | SKE | F2e |
| The Ear column's overflow at 1200 × 800 | EUI | Wave 6 |
| The daily formula and the UTC day boundary (as Review's, it turns at local midnight only in UTC) | RC | F1e |
| **macOS's local-network permission** for later live play: behaviour when the DAW lacks the usage key; Windows' first-listen alert naming the DAW; unloading an async runtime in a plugin | A live-duel research session | Before any live-duel lane |
| No R3b root-manifest edits in flight before S0e | The coordinator | Wave 1 |
| The 12 questions; silence means the defaults | You | Any time |
