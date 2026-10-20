# Phase 2 — R1 Recreate and S1 Instrument 0.1

**Goal: a Recreate round you and your friends can play, on an instrument you'd keep using.** R1: the synth generates a random patch at the difficulty you choose, you rebuild it by ear, and you get a sound %, a structure %, a combined %, a time to pass and coach hints. Its gate GR1: you and your friends play 20 rounds each on a build you can run, mostly agree with the score and want more, and the agent's solve rate is recorded as a baseline. S1: the editor adds and removes modules, browses and saves presets with undo, and ships for Windows and macOS. Its gate GS1: ten presets you'd actually use, no crashes or audio-thread allocations under clap-validator and a one-hour REAPER soak, and the editor still meets the UI thresholds ([roadmap.md](roadmap.md#r1--recreate-mode-34-weeks), [S1](roadmap.md#s1--instrument-01-46-weeks)). The two milestones are planned together because they share the editor, the build and distribution work, and your time.

Size: about **22–42 agent-hours** planned; expect **26–30**. That is calibrated on P1b, whose 15 lanes took about 20 agent-hours against an estimate of 27–51 (median lane 1.3 h; the walking skeleton 3.5 h), and on both earlier phases growing one or two lanes nobody planned. The critical path, S0r → CT + SG → SKR → RUI → INT, is about **8–11 h of wall time** with merges. Your own time is about 3 h new, plus the P1 backlog (owner builds 1 and 2, real audio, metric session 2, the reverb A/B, the G1 listen); each friend's is about 1.5–2 h. **What really sets GR1's end date is your friends' calendars**, which is why the friend build starts in wave 1.

**Status (2026-10-20): wave 1 is running: S0r, FB, SOAK, EV2 and DOCS2. Every other lane is not started.** Research session: [`_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/`](../_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/summary.md) ([summary.md](../_local/sessions/2026-10-20-r1-s1-recreate-and-instrument/summary.md) for the decisions, which win where they correct a report; reports 01–05 for the evidence and the numbers; report 05 §1 for the lane table this page adapts). The docs were updated to match on the same day (lane DOCS2): [design.md §9](design.md#9-randomizer-and-difficulty), [ADR-0007](adr/0007-metric-v1.md), [architecture.md](architecture.md), [editor.md](editor.md), [design-system.md](design-system.md), [roadmap.md](roadmap.md) and [AGENTS.md](../AGENTS.md).

New in phase 2: `enth-challenge` (L4) is created; `enth-session` (L6) grows a `game` module and presets; `enth-solver` (L5) grows sensitivity, coach hints and CMA-ES; `enth-ui` (L7) grows add/remove, the preset browser and the Recreate views; CI grows a `release.yml` for friend builds. `enth-standalone` (L8) joins only after your OK.

## What the research changed

Lanes should read these before they read the table, because each one changes what a lane builds.

- **The hooks are already there, so R1 and S1 run in parallel on what exists.** The patch model has every structural op (`AddModule`, `RemoveModule`, `Move`) with an exact inverse; the editor has an `Overlay` with Good/Near/Bad score rings and reserved `challenge_bar` and `results` regions behind the `recreate` feature; the engine plays the starter superset patch (every P1 module, the extras off) at no cost for the disabled modules; the real-time headroom is about 2.5×. Most of the risk is in the game logic's determinism and in score fairness, not in the plugin.
- **Targets can be identical on every machine if every random draw is keyed by its parameter address.** Each value is drawn from `derive(derive(seed, fnv1a64(address)), attempt)`, never in sequence, so adding a parameter or raising the difficulty leaves the other draws unchanged. A 250-line prototype gave identical target hashes on x86-64 and wasm for 3,000 seeds (report 01). The generator also masks what the phrase cannot reveal (gauges, dead parameters, nuisances), redraws until the target differs from the start by an audible step, and quantizes to values a player can reach. `RandomSpec` becomes integers only, and a challenge code carries the generator and engine versions and a target-hash prefix. All of it is in [design.md §9](design.md#9-randomizer-and-difficulty).
- **The sound score needs fixes before it feels fair** (report 02, measured on the real engine; [ADR-0007's 2026-10-20 note](adr/0007-metric-v1.md#note--2026-10-20-r1s1-research-five-fixes-for-the-sound-score)):
  - equivalent patches score as different, because start phase and noise follow module ids: re-adding an oscillator costs more than a third-of-an-octave cutoff error. The fix maps the player's modules onto the target's ids before the scoring render and subtracts the target's own nuisance floor;
  - short attacks barely register (5 → 20 ms moves metric v1 by at most 0.003), so descriptor components (attack, pitch, brightness, envelope, noisiness, movement) show beside the headline;
  - a single global pass threshold is unfair (an octave of cutoff is 0.05 on a pluck and 0.54 on a pad), so distance is measured in per-challenge JND units and mapped by a curve fitted to ratings;
  - metric v1.1 pools mel power over about 85 ms before the log (nuisance 2–20× lower, agreement with your ratings unchanged). A vs B and v1.1 wait for your metric session 2.
- **Sound % is not yet bit-exact across platforms.** Its loudness gain comes from `ebur128`, which uses `std` maths. That is harmless in a solo round, where one machine scores both clips, but fragile for golden challenges in CI and for R5's duel codes. Lane SS writes an in-house BS.1770 loudness on `libm` for the scored path (report 05).
- **The A/B needs no engine work** (a correction of report 05's [A1]). **Target ▶** plays a pre-rendered, loudness-matched loop of the scored phrase; **Mine ▶** plays the same phrase live through the player's engine, in sync; A/B flips at the same position, and the louder side is attenuated. There is no second engine, so the heard target is exactly the scored one.
- **Score disputes are the top product risk.** Syntorial keeps a support path for "I disagree with my score" (report 05). So a three-way "felt too low / right / too high" vote, a Disagree button and a local round log go into the first playable round. They also measure GR1's "mostly agree". There is no live score by default: Submit shows the rings, which then update live.
- **S1 is mostly editor and content.** Add/remove is an editor lane, because the ops exist. A fresh instance will play the starter patch as "Init", gain-staged: report 04 measured the current default peaking at +7.9 dBFS on a four-note chord. An editor preset load becomes one undo step, delivered like a load with **no host writes** (per-parameter writes are the nih-plug #92 crash class). A preset file is exactly `Patch::to_json()`.
- **Poly-mod is done.** It is implemented and test-host-checked (P1b lane PL), so report 05's PM lane is dropped; only your Bitwig check (about 20 min) is left.
- **The fork already compiles for Windows and macOS** (check and clippy pass; nothing was linked or run). Friends can run unsigned builds with one manual step on macOS. **CI minutes were the real limit**: 1,458 Linux and 418 Windows minutes in the repo's first ~1.1 days, and 69 of the first 107 commits changed only Markdown. Docs-only pushes now run no workflows (coordinator, 2026-10-20), and friend builds run on a tag or by hand, never on push.
- **A headless REAPER soak works, but only through the virtual keyboard**: REAPER's transport does not move under Dummy Audio. Report 04's 3-minute probe saw editor open/close grow memory by about 0.5–0.7 MB per cycle, which conflicts with the test host's 12.4 KB per cycle (`leak-test`): lane SOAK must attribute it, to REAPER's own windows or to ours.

## Freezes

The spine is serial and has three freeze points. Lanes fan out after each. **After F2r, challenge, session and plugin types change only additively.**

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0r** | S0r: `enth-challenge` created, every phase-2 file pre-declared, the three header hooks, pins | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)), judged by exit status |
| **F1r** | CT and SG: Challenge v1, `RandomSpec`, code v1, `Scorer` and `ScoreReport`, the session game API, the state blob's new fields; PBC's preset format | A judge agent (Opus) reviews them together: they are persisted, go to friends and, later, carry duels |
| **F2r** | SKR: a round end to end in the plugin | 5 golden challenges green on Linux, Windows and wasm; the kittest round flow; the A/B toggle measured in `process-stats`; `validate` and `reaper-test` green |

The coordinator alone edits `[workspace.dependencies]`, `Cargo.lock` and `[patch.crates-io]` after S0r. A lane that needs a dependency (PBC's `dirs` 7.0.0, for one) asks for it.

## Lanes

Sizes are agent-hours (estimates, from report 05 calibrated on P1b's actual hours). H = heavy local build (nice-plug, egui or clack), L = light; FB builds on GitHub's runners, so it is light here.

| Lane | Owns | Depends on | Size | State (2026-10-20) |
| --- | --- | --- | --- | --- |
| **S0r** Workspace prep (→ F0r; both tracks; H once) | Creates `enth-challenge` (L4: already in `xtask/layers.toml`, no crate yet) and pre-declares the solver, session, UI, plugin, CLI, xtask and CI files of the lanes below; reserves the state blob's optional `game` and `preset` fields with serde defaults; three no-op header hooks (Recreate, Presets, Add); per-area token files; pins | main (`6cc6418`) | 0.5–1 | In progress |
| **FB** Friend build and OS CI (S1, needed by GR1; L) | `release.yml`, on a tag or by hand and never on push: Linux, Windows MSVC with `+crt-static`, a universal macOS bundle, clap-validator on each leg's debug bundle; the per-push Windows and macOS cross-target clippy guard on the Linux runner; bundle config; `docs/distribution.md` and the friends' install sheet (unsigned, ad-hoc signed macOS bundles with the `xattr` line); why `nightly.yml` has never run, and one run started by hand. The standalone waits for your OK | Nothing (the friend survey sets the targets) | 1–2.5 | In progress |
| **SOAK** One-hour REAPER soak (S1; L, uses the bundle cache) | `cargo xtask reaper-soak --minutes 60` (`xtask/reaper/soak.lua`, a `soak` scenario): the debug bundle, notes from REAPER's virtual keyboard, macro and slot sweeps, state loads, editor opens and closes; memory, threads and descriptors sampled throughout. **Attributes the REAPER editor-cycle memory growth** | Nothing | 0.75–1.5, plus the 1 h runs | In progress |
| **EV2** Agent evals (R1; L) | `content/agent-evals/`: about 30 text goals (from 10) plus reference sounds (report 02: 25 randomizer targets and 5 recordings that are CC0 or yours), with fixtures; `eval_version` bumped; a baseline with model, date and cost. In wave 4, the recreate track: the agent on the humans' 20 codes | Nothing; the recreate track needs CL | 1–2 | In progress |
| **DOCS2** Docs (both; L) | This page; the summary's "Docs to change" rows in [roadmap.md](roadmap.md), [design.md §9](design.md#9-randomizer-and-difficulty), [ADR-0007](adr/0007-metric-v1.md), [architecture.md](architecture.md), [editor.md](editor.md), [design-system.md](design-system.md), [AGENTS.md](../AGENTS.md) and [README.md](README.md) (not in report 05's table) | The summary | About 1 | In progress |
| **CT** Challenge types (→ F1r; R1; L) | `enth-challenge`: `model` (Challenge v1, `RandomSpec`), `code` (code v1: Crockford base32 with a check symbol), `score/mod` (`Scorer`, `ScoreReport`); fixtures; `docs/challenge.md` | F0r | 1–1.5 | Not started |
| **SG** Session game API (→ F1r; R1; L) | `enth-session/src/game.rs`: the pure round state machine (start, A/B, submit, give up, reveal, hints, an edit log with the shell's timestamps, the stash of your own patch), its threading contract, the state blob's `game` field | F0r | 1–1.5 | Not started |
| **RG** Randomizer (R1; L) | `enth-challenge` `random` and `priors` (`priors::v1`): keyed streams, the observability mask, the minimum step, reachable quantization, enable flags and routes over the starter superset (if you agree); property tests; a 1,000-hash golden | F0r (`RandomSpec` pre-declared from design §9) | 1–2 | Not started |
| **PBC** Preset core (S1; L) | `enth-session/src/presets.rs` (format, index, tags, search) and the plugin's preset I/O (`Documents/enth/Presets` through `dirs` 7.0.0, `ENTH_PRESETS_DIR`); **undoable loads**: the CLI's `diff_ops` moved down into `enth-patch` as `ops_between`, and `Session::load_preset` as one undo step delivered like a load (no host writes) that marks the project dirty | F0r; format reviewed at F1r | 0.75–1.5 (report 05's size, before undoable loads joined it) | Not started |
| **AM** Add/remove modules, Init (S1, R1's highest difficulty needs it; H) | Add, remove, move, duplicate and rename intents and their entry points (the card "⋯" menu, right-click on empty region space, a dashed "+ Add" slot; the Add hook), one op and one undo step each; **the Init patch**: a fresh instance plays the starter patch as "Init", peaking at or below −1 dBFS on a four-note chord, with `Patch::init()` unchanged for the CLI and tests; **the minimum-window measurement** at 1000 × 680 | F0r | 1.25–2.5 | Not started |
| **SKR** Round skeleton (→ F2r; R1; H) | Plugin game wiring (the clock, the worker, Target ▶ as a pre-rendered loop and Mine ▶ as the live phrase, A/B at the same position), `enth challenge new\|show`, `enth compare` (stub), the Recreate hook, an A/B toggle in `process-stats`, 5 golden challenges | F1r, RG's lowest difficulty | 2.5–4 | Not started |
| **SS** Sound scorer (R1; L) | `score/sound` (module mapping, the target's nuisance floor, components; v1.1 under its own id), `calibrate` (per-challenge JND scale, the fitted curve), the in-house `libm` loudness for the scored path, the level match that A/B shares | F1r; the curve after metric session 2 | 1–2 | Not started |
| **ST** Structure, combined and time (R1; L) | The rest of `score/`: structure (canonicalise, match modules per kind, active parameters with sensitivity-based tolerances, equivalences), combined (w·sound + (1 − w)·structure, w from 0.5 to 0.8 by difficulty; only sound decides pass), time (the first passing submit) | F1r | 1–2 | Not started |
| **SN** Sensitivity and coach (R1; L) | `enth-solver` `sensitivity` and `coach`: the target's descriptors become an `Expect` list for `hint`, unchanged; hints revealed in levels | F1r | 0.75–1.5 | Not started |
| **MT** Match (R1; L) | `enth-solver` `cmaes`, `search` and `heuristic`, ported from `spike-metric`, for `match`; `nearest` (cut to R2 if late) | F1r | 0.75–1.5 | Not started |
| **PBU** Preset browser UI (S1; H) | `editor/browser.rs`: the header's ◀ name ▶ / Browse / Save (the Presets hook), the browse panel over the body (Factory, User, Favorites; word search; click loads); REAPER `preset-load` on both formats | PBC | 1.25–2.5 | Not started |
| **GC** Golden challenges (R1; L) | `tests/golden/challenges/` (about 24 cases), the `golden.yml` step, analysis determinism on Windows and macOS | SS, ST (minimal), RG | 0.75–1.25 | Not started |
| **PR** Factory presets (S1; L) | `content/presets/`: about 20 drafts agents make through the CLI, normalised by a new `cargo xtask preset-add`, beside your four G1 sounds; factory tags from a vocabulary | PBC's format | 0.75–1.5 | Not started |
| **RUI** Recreate UI (R1; H) | `editor/recreate/*`: the challenge bar in the header row, the results drawer in the overview slot, randomizer settings, the score overlay; the agreement buttons and the dispute file; `sizes/recreate.rs` | F2r, SS, ST | 1.5–3 | Not started |
| **CL** CLI for R1 (L) | `enth compare`, `suggest`, `match`, `explain`, `challenge`; insta snapshots; `docs/agent-cli.md` | SS, ST, SN, MT | 0.75–1.5 | Not started |
| **GK** GR1 kit (L) | Round-log schema v1 (JSONL: code, patch, scores, time, the agreement vote), `cargo xtask gr1-report`, the friend and owner sheets | SG, CT | 0.5–1 | Not started |
| **INT** Integration (both; H) | The GR1 and GS1 tables, owner sheet 3, the friend release, stale docs | Everything | 1.5–2.5 | Not started |
| **LV** Layout variant (S1; H; one per variant) | `enth-ui/src/editor/layout/<variant>.json` and its snapshots, with the overflow report at 1200 × 800 and 1000 × 680 | Your layout pick | 1–2 each | Not started |
| **PM** Poly-mod proof | — | — | — | **Dropped**: poly-mod is implemented and test-host-checked (P1b lane PL); your Bitwig check stays |

**Total: about 22–42 agent-hours**, plus 1–2 per layout variant. Dropping PM and adding DOCS2 leave report 05's range about where it was.

## Waves

Two heavy slots; at most four light lanes build at once. Every heavy lane is paired with light ones.

| Wave | Heavy | Light | Starts when |
| --- | --- | --- | --- |
| 1 | S0r (→ F0r) | FB, SOAK, EV2, DOCS2 | Now |
| 2 | AM | CT, SG (→ F1r, judge-reviewed), RG, PBC | F0r |
| 3 | SKR (→ F2r), PBU | SS, ST, SN, MT; then GC, PR | F1r (PBU after PBC) |
| 4 | RUI | CL, GK, EV2's recreate track | F2r, SS, ST |
| 5 | INT | — | Everything merged; then owner build 3 → a friend smoke round → fixes → GR1, and the friend release |
| Later | LV | — | Your layout pick |

**Critical path:** S0r (0.5–1) → CT + SG (1–1.5) → SKR (2.5–4) → RUI (1.5–3) → INT (1.5–2.5), about 7.5–12.5 agent-hours by report 05, **8–11 h of wall time** with merges. Your gates and your friends' rounds come on top, and they usually set the end date.

## Acceptance per lane

Each lane stops at its own criteria and reports against them. P1–P11 are the properties under [Test strategy](#test-strategy-for-the-game-logic).

| Lane | Acceptance |
| --- | --- |
| **S0r** | The Done-means gate is green. Every stub compiles. A probe that adds a dependency in a lane fails `--locked`. The v1 state fixtures still load |
| **FB** | Windows x86-64 and macOS arm64 artefacts (and the universal bundle) pass `validate` on their runners from a manual `release.yml` run. The cross-target guard runs on every push. The install sheet is written. `nightly.yml` has run once, and why it never ran on its schedule is answered. One friend per OS loads the plugin (at the smoke round) |
| **SOAK** | 60 minutes green twice in a row. Slopes reported against the leak gates. A leaky control is caught. The REAPER editor-cycle growth is attributed, to REAPER's windows or to ours, with the evidence |
| **EV2** | About 30 goals plus reference sounds with fixtures; `eval_version` bumped; a baseline recorded with model, date and cost |
| **DOCS2** | Every "Docs to change" row applied; every link and anchor it added resolves; `scripts/gate.sh lane` green |
| **CT** | Challenge v1 and code v1 fixtures. P1. The judge review done. `docs/challenge.md` written |
| **SG** | Session tests for the round lifecycle, A/B, submit, give up and injected time. The stash survives a project save mid-round. The threading contract is written |
| **RG** | P2–P6 and the 1,000-hash golden pass. A check that `random` never reaches `enth-render` |
| **PBC** | Format fixtures. Search and tag tests. Loading through state is asserted. `ops_between` has its property test (applying it to a gives b). A preset load is one undo step whose load, undo and redo make no host writes and mark the project dirty; REAPER's `undo-state-load` and `patch-roundtrip` stay green |
| **AM** | Add and remove every kind, with undo. The leak gate unchanged. The coordinator updates the whole-editor snapshots. Init peaks at or below −1 dBFS on a four-note chord; the REAPER scenarios that assumed `Patch::init()` follow. The 1000 × 680 measurement is reported, with a recommendation (raise the minimum, or make the variants handle it) |
| **SKR** | The F2r row above |
| **SS** | P7, P10 and P11. Score components ("brighter", "attack too fast"). The in-house loudness in the scored path |
| **ST** | P8 and P9. The combined formula documented. Injected time |
| **SN, MT** | Identical results for 1 and 8 workers. MT reports the golden-challenge solve rate |
| **PBU** | kittest browse, search and load. REAPER `preset-load` green on both formats |
| **GC** | About 24 cases on Linux, Windows and wasm (macOS weekly). Analysis determinism in the matrix. The margin assertion |
| **PR** | About 20 tagged candidates that round-trip. Your pick recorded |
| **RUI** | The round-flow and no-peek tests. Snapshots. The agreement and dispute files. 0 idle frames kept |
| **CL** | Versioned schemas. insta snapshots. Errors carry a `hint` |
| **GK** | `gr1-report` on fixture logs gives the GR1 numbers |
| **INT** | The full gate, the leak test, the soak and REAPER green. The GR1 and GS1 tables filled except your and your friends' rows. Owner sheet 3 written |
| **LV** | Snapshots at the P1 default and the maximum patch; the overflow report at 1200 × 800 and 1000 × 680. Switchable in the dev window without a rebuild |

## Owner gates

| Gate | What you do | Time | When | Default if you don't answer |
| --- | --- | --- | --- | --- |
| Friend survey | Ask your friends which OS and DAW they use, and whether 20 rounds is OK | 5 min | **First**: it sets FB's targets | GR1 is yours alone; the friends join in S1 (the roadmap allows it) |
| GitHub plan and minutes | Say which plan the repo is on, or allow the billing API to be read (it needs your `user` scope) | 2 min | Soon | Path filters and tag-only friend builds; macOS minutes only in `release.yml` and weekly jobs |
| Metric session 2 | Test–retest ([listening.md](listening.md#3-metric-pairs-spike-metric): `--session 2`, then `--analyze`) | 15 min | Before lane SS fixes the curve | Metric A, with weights leaning on structure |
| A real-engine pairs session | About 40 pairs rendered by the real engine; every rating so far is from the toy synth | 15–20 min | After SS, before GR1 | — (the curve stays provisional) |
| Structure randomization | Yes or no: R1 randomizes enable flags and routes over the starter superset | 1 min | Before RG | **Yes** |
| The standalone app | Yes or no to `enth-standalone` on nice-plug's `standalone` feature, for Logic users and friends with no DAW. It pulls `cpal`, `midir` and, on Linux, `jack`, which [AGENTS.md](../AGENTS.md#choosing-libraries) says need your OK | 1 min | Before the friend build | **Plugin bundles first; the standalone after your OK** |
| Owner build 3 | Five low-difficulty rounds; the real-display check of Add, the browser and Recreate; the preset pick (below) in the same sitting | 60–75 min | After INT | — (GR1 and GS1 wait) |
| Preset pick | Audition about 20 drafts beside your four G1 sounds; keep 6, tweak and save | Within owner build 3 (60–90 min on its own) | After PR | — (GS1's ten presets wait) |
| GR1 | 20 rounds with a 5-minute soft cap, logged automatically | 60–100 min, 3–4 sittings | After build 3's fixes | — (GR1 waits) |
| Layout pick | Read the Claude Design variants; pick one, or "keep Lanes, fixed" | 20–30 min | When the variants arrive | Lanes |
| Bitwig poly-mod (optional) | A per-voice modulator on Macro 1 with Macro 1 routed to cutoff; play a chord (each voice should differ); release (no voice hangs) | About 20 min | Any time | Poly-mod rests on the test-host check |

**Your friends:** install takes 10–20 minutes. A **smoke round** first (one friend per OS, 5 rounds), then fixes, then **GR1** (3–5 friends, 20 rounds each, 60–100 minutes in 2–4 sittings, at 2–5 minutes per low-difficulty round, extrapolated from Syntorial: no enth round has been timed). Each friend sends one file, the round log. `cargo xtask gr1-report` reads them: no network code, no telemetry.

Three defaults apply until you say otherwise. **Structure randomization is on.** **The friend build is plugin bundles, unsigned** (ad-hoc signed on macOS, with one Terminal line per bundle; no Windows signing), and the standalone waits for your OK. **The layout stays "Lanes".**

## The GR1 and GS1 checks

Lane INT fills the tables with the evidence.

| Gate | Check | How | Who | When |
| --- | --- | --- | --- | --- |
| GR1 | A build your friends can run | `release.yml` bundles, the install sheet, the smoke round | FB, INT | Wave 5 |
| GR1 | 20 rounds each, logged | The round log, `cargo xtask gr1-report` | GK; **you and your friends** | After owner build 3 |
| GR1 | "Mostly agree" with the score | The three-way vote after each round. Proposed reading (report 05, not yet settled): at least 70 % "about right" overall and no rater under 50 % | GK, RUI | GR1 |
| GR1 | "Want more" | "Another round?" Proposed reading: yes on at least 60 % of rounds | GK, RUI | GR1 |
| GR1 | The agent's solve rate as a baseline | The agent plays the humans' 20 codes 3 times each: pass@1 per difficulty, median sound %, renders, tokens and cost, beside a no-op floor and a CMA-ES floor | EV2, MT, CL | Wave 4 and INT |
| GS1 | Ten presets you'd actually use | Your four G1 sounds plus the six you keep | PR; **you** | Owner build 3 |
| GS1 | No crashes or audio-thread allocations under clap-validator | `validate` on each `release.yml` leg's debug bundle (`assert_process_allocs` on) | FB | Wave 1, then each release |
| GS1 | … and a one-hour REAPER soak | `cargo xtask reaper-soak --minutes 60`, twice in a row | SOAK, INT | Wave 1, then after the editor lanes |
| GS1 | The editor still meets the UI thresholds | 0 idle frames (exact-delay tests); the frame cost under the proposed 10 M instructions (the worst case had 6 % to spare, and the browser replaces the body while open); the leak gate; your real-display check | AM, PBU, RUI; **you** | Owner build 3 |
| GS1 | Undo and redo, preset loads included | Session and REAPER tests | PBC | Wave 2 |
| GS1 | CLAP poly-mod on macros | Implemented and test-host-checked (P1b PL); Bitwig optional | **You**, optional | Any time |

## Test strategy for the game logic

Like G1's: cheap, deterministic signals first, and every claim about "the same on every machine" backed by a golden on that machine.

**Already proven:** renders are bit-identical on Linux, Windows, macOS arm64 and wasm32 (72 goldens; macOS once, 2026-10-19); the scalar FFT, features and descriptors are pinned and equal on x86-64 and wasm32. **Not proven:** those analysis pins on Windows and macOS (they are not in `golden.yml`); `FINDINGS_SCALAR` under wasmtime; `ebur128`'s loudness, which is equal only within a tolerance and sets metric v1's gain.

**1. Target determinism** (parameter space only, [design.md §9](design.md#9-randomizer-and-difficulty)).

- Generation is pure, in `enth-challenge`: `derive` and `fnv1a64` plus `enth-params` scales through the pinned `libm`, with no render anywhere.
- Keyed streams, versioned priors (`priors::v1`; a change is a new generator version), and old generator versions frozen.
- Decoding a code regenerates the target and compares the hash prefix. A mismatch is an error with a hint, never a different target.
- **The 1,000 (seed, settings) → hash golden** runs in `enth-golden` on Linux, Windows and wasm on every push, and on macOS weekly. In the Phase 0 spike, `std` maths changed 13 of 1,000 seeds on glibc.

**2. Property tests** (a fixed seed and 256 cases in the gate, 10,000 nightly).

| # | Property |
| --- | --- |
| P1 | `decode(encode(c)) == c`; a corrupted code gives an error, never a panic; the check symbol catches every single substitution and adjacent swap |
| P2 | Generated patches pass `validate`, respect the challenge's constraints, and stay in musical bounds (level, resonance, pitch) when `musical` is high |
| P3 | Difficulty is monotone (the category sets nest); per-category `amounts` override the knob |
| P4 | Reachable and observable: every randomized parameter is editable, in range and visible to the phrase; disabled modules and lone gauges are never randomized |
| P5 | The minimum step from the start holds |
| P6 | Keyed-stream stability: with an extra test parameter, the shared parameters draw the same values |
| P7 | Scores stay in 0–100, and the target itself scores 100 |
| P8 | Structure equivalences score 100: a module off ≡ absent, depth 0 ≡ no route, permuted identical oscillators, enharmonic tunings |
| P9 | Structure is monotone along any single parameter toward the target |
| P10 | Sound stability: a 1e-3 relative parameter move changes the score by under 0.5 point; a nuisance seed or phase gives at least 99. These render, so 32 cases in the gate and 1,000 nightly |
| P11 | The A/B level match and scoring share one gain function |

**3. Golden challenges** (lane GC). About 24 cases: 5 difficulties × 4 seeds, plus `musical` = 0, structure randomized, the maximum patch and a near-silent target. Each records the code, the versions, the target's canonical and render hashes, three reference answers (exact, start, near miss), the scores to 1e-6 and pass or fail, and asserts a margin of at least 1e-3 to the pass threshold, so no golden sits where a last-bit gain flips it. Scorers are additive ([design.md §6](design.md#scoring-is-additive)): a new version adds columns and never changes old values. On the plugin side, a `host-render` case checks that the target played through the plugin equals `enth render` of the target (the `cli-equal` pattern).

**4. Scoring across platforms.** A tolerance is enough for a solo round, but not for golden challenges in CI or for R5. So SS's in-house BS.1770 loudness on `libm` (K-weighting biquads, 400 ms blocks with 75 % overlap, the −70 LUFS and −10 LU gates, a fixed summation order; about 150–250 lines) replaces `ebur128` in the scored path, which removes the only platform-dependent input from the goldens. `ebur128` stays for display and true peak. GC adds `-p enth-analysis --test determinism` to `golden.yml` and runs it under wasmtime.

**5. Game and plugin tests.**

- The round state machine lives in `enth-session`, with time injected by the shell; no wall clock in any gate.
- kittest drives the round by label: Recreate → bar → Target ▶ / Mine ▶ → Submit → results → agreement → next.
- **No-peek:** in Recreate, no label or tooltip shows a target value.
- **Real time:** `process-stats` with A/B toggled every 250 ms, reported like RT's edit row. A plan swap between different structures copies effect state (RT measured its block at p99 337 µs, 25.2 %).
- **The UI never blocks:** scoring and hints run on the worker, and a stalled scorer still gets frames.
- **Agent baseline:** a deterministic CMA-ES solve rate on the golden challenges as a nightly floor; the LLM agent through the CLI on demand, with model, date and cost recorded.

**6. S1 tests.** The soak (above). Presets load through state on the main thread, never through automation, and each round-trips byte for byte. Poly-mod is already covered (P1b PL).

Reliability rules carry over from P1: no wall-clock assertions in the gate, no random seeds in the gate, no test that depends on another lane's snapshots or goldens, a flaky test is a bug, a pixel snapshot is never the only assertion for an interaction, and every gate step is judged by its exit status.

## Rules for every lane

- Branch from the latest freeze, and write only the files your lane owns. One owner per doc: `docs/challenge.md` (CT, with sections by RG, SS and ST), `docs/distribution.md` (FB), `docs/presets.md` (PBC, PBU), `docs/agent-cli.md` (CL), each lane its own section of [editor.md](editor.md) (AM, PBU, RUI), the GR1 and GS1 tables (INT). Report against your acceptance criteria when you stop.
- **Privacy.** Never put your email, name or other personal details in any request, header, URL or payload (crates.io and GitHub User-Agents included); use `enth-research/0.1`.
- **No windows on the real display** (Xvfb, as [AGENTS.md](../AGENTS.md#ui) says), and **never play audio**.
- **Foreground commands** with a timeout above their measured length; a backgrounded agent can go idle and never resume. Keep a running `notes.md` in persistent scratch (`~/code/enth-scratch/r1s1/<lane>/`).
- **Concurrency.** At most **2 heavy builds at once** ([AGENTS.md](../AGENTS.md#working-in-parallel-phase-0-lanes-and-any-concurrent-agents)): 15 GiB of RAM is shared by every lane. `CARGO_BUILD_JOBS=3`, a separate `CARGO_TARGET_DIR` per lane on ext4, `flock -s` around builds and `flock -x` around timing runs. Build only the crates you need until the final gate. Harness lanes reuse the coordinator's release bundles.
- **A two-tier gate, judged by exit status only** (no `cmd | tail && echo ok`): `scripts/gate.sh lane <worktree>` before a lane reports, `gate.sh full` (the coordinator's) before a merge.
- **No engine work.** The research found none for R1 or S1: no lane edits `enth-engine` or `enth-render` without the coordinator, and no engine golden may change.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshots per layout. The GR1 and GS1 tables are INT's.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `Cargo.lock`, `[workspace.dependencies]`, `[patch.crates-io]` | S0r, then the coordinator. The standalone's `cpal`, `jack` and `midir` (through nice-plug's `standalone` feature in the vendored manifest) wait for your OK |
  | `vendor/` | Lane FK only (none is planned; CLAP preset discovery would be a later fork patch) |
  | The editor header | S0r declares three no-op hooks; AM, PBU and RUI fill their own |
  | The state blob | S0r reserves optional `game` and `preset` fields with serde defaults; SG and PBC fill them; the v1 fixtures still load |
  | `enth-challenge` | CT (`model`, `code`, `score/mod`), RG (`random`, `priors`), SS (`score/sound`, `calibrate`), ST (the rest of `score/`) |
  | `enth-solver` | SN and MT add files; `hint/` is untouched |
  | `enth-patch` | PBC adds `ops_between` (moved from `enth-cli`), additively |
  | Theme tokens | Per-area token files from S0r; a lane adds tokens only in its own area file ([design-system.md](design-system.md#token-files)) |
  | `tools/test-host` | SKR's A/B flag, additively |
  | `tests/golden` | GC adds `challenges/`; the engine cases are unchanged |
  | CI workflows | S0r stubs; FB owns `release.yml` and the cross-target guard; GC the `golden.yml` step |

- **Ownership crossings** are additive only and listed in the report. Both earlier phases grew an unplanned lane; INT is planned up front.
- **The cut list**, agreed in advance: if the phase runs late, nearest-solution (MT's `nearest`), categorical hints, AU and signing move to R2 or S2. LV stays off the critical path.
- **Risks** (report 05 §3 lists the top eight, each with its cheapest retirement):
  1. score disputes (the vote and dispute file from SKR on, shown components, low difficulties first, metric session 2 before owner build 3, disputes as metric v2's fixtures);
  2. target drift across versions or platforms (keyed streams, versioned priors, versions and a hash prefix in the code, the 1,000-hash golden on the CI matrix);
  3. sound % not bit-exact across machines (the in-house loudness, GC's margin assertion);
  4. the friend build not running (the survey now, FB in wave 1, a smoke round, owner-only GR1 as the fallback);
  5. your bandwidth (one sheet per sitting with defaults, combined sittings, automatic logs, nothing on the agent path waiting before INT);
  6. scope creep across two milestones (the acceptance tables are the definition of done; the cut list);
  7. A/B that costs real time or clicks, or doesn't match what is scored (SKR measures the toggle before RUI builds on it; P11's shared gain);
  8. merge hotspots and contention (S0r's pre-declarations and hooks, coordinator-owned composites, additive-only after F2r).

## Open checks

Not settled by the research; each is owned by a lane, by you or by a later milestone.

- **The REAPER editor-cycle memory growth** (report 04: about 0.5–0.7 MB per cycle in a 3-minute probe, against `leak-test`'s 12.4 KB): lane SOAK attributes it in wave 1.
- **Why `nightly.yml` has never run** although its schedule exists: lane FB finds out and starts it once by hand.
- **Linking and running on real Windows and Mac machines.** Nothing has been linked or run there, and the baseview + egui/glow editor has never run on either. The first friend build settles it; on Windows baseview may swallow DAW shortcuts, and Smart App Control blocks unsigned plugin DLLs with no override.
- **The code's size.** The summary gives the short code about 13 characters and also puts the generator and engine versions and a target-hash prefix in it. Report 01's 13-character short form (64 bits) has no room for a 32-bit hash prefix or an engine version, so CT settles the bit budget at F1r, and the judge checks it.
- **Whether an engine-version mismatch refuses a code or only warns.** A generator or hash mismatch changes the target, so it refuses; a DSP change moves the audio, not the target (report 01 warns on it for R5's result codes). CT proposes, F1r's review decides.
- **The state blob's version.** Report 05 calls the new `game` and `preset` fields "v2", but by [session.md](session.md#the-saved-state-version-1) an optional field with a default stays within version 1. S0r and SG decide.
- **The minimum window.** At 1000 × 680, lanes drops its overview, so the drag sources vanish. Lane AM measures; then either the minimum rises or the variants handle it (LV). Claude Design gets the `LayoutSpec` vocabulary and both sizes.
- **The frame rate on the real display** (adopted, not yet owned by a lane): ask for 1/60 s while live data changes, since baseview paces at the X Present `msc + 2` (1/30 s gave about 20 fps in Phase 0), and count drawn frames under `editor-counts`. Your owner check 1.3 confirms. **Assigned to AM** (coordinator, 2026-10-20), an editor lane in wave 2.
- **The RTSan-built test host around `process()` and a local panic-hook crash file** were owned by no lane in report 05's table. **Assigned** (coordinator, 2026-10-20): the panic-hook crash file to PBC (plugin-side, light); the RTSan-built test host to INT, and to the cut list if the phase runs late.
- **Target ▶ and preset audition in a host that stops calling `process()` while the transport is stopped** (nice-plug never calls `request_process`): SKR and PBU check REAPER; other hosts are open.
- **Where the preset library lives.** Report 03 proposed an `enth-content` crate (L2) for factory presets, templates and search; report 05, whose lane table the summary adopts, puts the format and search in `enth-session/src/presets.rs`. PBC follows report 05 unless F1r's review asks otherwise.
- **Found by lane PR (2026-10-20):**
  - **`filter1.drive` loses about 1 dB of level per dB of drive** at full oscillator level (about −25 dBFS at 24 dB), so turning Drive up makes a sound quieter. The candidates work around it with compressor make-up on the Drive macro. The real fix is level compensation in the saturator, a DSP change that moves goldens; it needs a lane.
  - **The owner's G1 sounds would collide with "Init".** `extract-presets` keeps the saved patch's name, and the starter project now loads Init, so every extracted sound is named "Init" and tagged `init`. Fix `extract-presets` to name each sound after its track and drop the `init` tag, before owner build 2 is extracted (INT, or the coordinator).
  - **The tag vocabulary has no percussion type,** so the kick and snare candidates are tagged `fx`. Add `perc` in `enth-session/src/presets.rs` and its copy in `xtask/src/preset_add.rs` together (a test catches drift).
- **`enth-standalone` is not in `xtask/layers.toml`.** AGENTS.md names `standalone` as an L8 shell, but the layer file has no line for it; it gets one when you approve the standalone.
- **Sound % calibration.** [design.md §6](design.md#6-challenge-and-scoring) still says "the starting patch scores 0, the pass threshold scores 100"; [ADR-0007's 2026-10-20 note](adr/0007-metric-v1.md#note--2026-10-20-r1s1-research-five-fixes-for-the-sound-score) replaces that for sound % with JND units and a fitted curve (a progress bar may still use start → pass). The pass display (a badge with headroom, or pass = 100) is yours to call after GR1.
- **The GR1 reading** ("mostly agree" and "want more" thresholds above) is a proposal until you or GR1's data settle it.
- **Live meter against score-on-submit, and the difficulty steps:** GR1's data decides (half the rounds with a live meter as an assist).
- **Level 1 repeats** (about 12 distinct sounds): keep a seed history per session, and widen the level if testers get bored.
- **HiDPI** (baseview #359, open) stays your owner check 1.6; the header's UI-scale menu is the fallback. Editor size persistence is still open (`EguiEditorState` has no setter).
- **CLAP preset discovery** (REAPER 7.68 and Bitwig list a plugin's factory presets; nice-plug lacks it, issue #73): a later lane-FK fork patch, not GS1.
- **macOS OpenGL** is deprecated; if it is removed, egui on glow breaks there ([ADR-0002](adr/0002-ui-toolkit.md)).
