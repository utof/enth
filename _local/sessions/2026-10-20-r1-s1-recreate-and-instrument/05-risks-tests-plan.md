# 05: Risks, tests and the combined R1 + S1 lane plan

- **Date:** 2026-10-20. **Sub-researcher:** 05 (row 5 of the brief). **Main at:** `4cd740a` (P1b INT merged 05:34).
- **Read:** brief, research-process, AGENTS, roadmap, phase-1a/1b, p1b-g1, p1b-owner, design §5–§9, architecture, ADR-0007, the coordinator handoff, and P1b report 04 as a model.
- **Local, read-only:** git merge times, P1b lanes' `notes.md` birth times, merge-gate logs, the codebase-memory graph (project `enth`), greps of the code, workflows and `vendor/`, `free` and `uptime`.
- **Web:** WebSearch and WebFetch, 2026-10-20, no personal data in any request.
- **Not done:** no builds; reports 01–04 were not readable. Where they may change the plan I flag **[A1]–[A6]**.
- **Labels:** *measured* (local output, git, logs), *documented* (docs, source, vendor pages), *anecdotal* (forums, single reports), *estimated* (my arithmetic).
- **Scratch:** `~/code/enth-scratch/2026-10-20-r1-s1/05/notes.md`.

## 1. Summary

- **Calibrate on P1b's actual hours.**
  - P1b's 15 lanes took about **20 agent-hours** against an estimate of 27–51, counted from each lane's `notes.md` birth to its merge.
  - Wall time was **10.5 h** (19:07 → 05:34) against a critical path estimated at 13.5–24.
  - The median lane took 1.3 h; SK took 3.5 h.
  - P1a took 18 h against 52–93.
  - Both phases grew 1–2 lanes nobody planned (CH; FK2 and EV). The figures are *measured* from file times and git.
- **The plan: 22 lanes in 5 waves, about 22–42 agent-hours planned. Expect about 26–30** (22 lanes × the 1.3 h median, plus one unplanned lane).
  - Critical path: S0r → CT + SG (F1r) → SKR (F2r) → RUI → INT, about 7.5–12.5 agent-hours, or **8–11 h of wall time** with merges.
  - **GR1's real end date is the friends' calendars.** FB (friend-build CI), RG (randomizer), SOAK and EV2 don't need the R1 types, so they start in wave 1.
- **Renders are bit-exact; sound % is not yet.** Metric v1 scales each clip to −23 LUFS with a gain from `ebur128`, which uses `std` maths, so the gain can differ in its last bits between platforms (*documented*: `enth-analysis/src/features.rs` lines 203–225).
  - It is harmless in solo R1, where one machine scores both clips.
  - It is fragile for golden challenges in CI and for R5's result codes.
  - Fix: an in-house BS.1770 loudness on `libm` in the scored path, or a tolerance plus a margin rule.
- **Target drift across versions is the second determinism trap.** If the generator draws values in sequence, adding one parameter silently changes every shared code.
  - Key each draw by parameter address.
  - Put the generator version, the engine version and a target-hash prefix in the code, and refuse a mismatch rather than score a different target (Factorio's CRC lesson).
- **Score disputes are the top product risk.** Syntorial, with notched knobs, still runs a support path for "I disagree with my score" and "the value can't be reached", and attaches both patches to the report (*documented*).
  - Build a three-button "score felt too low / right / too high" and a dispute file into the walking skeleton.
  - That also measures GR1's "mostly agree".
- **Owner time:** about **2.5–3 h** new, plus about **3.5–4.5 h** of P1 backlog.
- **Friend time:** about **1.5–2 h each** (install, plus 20 rounds with a 5-minute soft cap).
- **The first owner action is the friend survey** (5 min): its answers set FB's targets.

### Lane table

H = heavy local build (egui, nice-plug, clack); L = light. FB builds on GitHub runners, so it is light locally. Sizes are agent-hours, calibrated on P1b's per-lane actuals.

| Lane | Track | H/L | Owns | Depends on | Size | Wave |
| --- | --- | --- | --- | --- | --- | --- |
| **S0r** Workspace prep (→ F0r) | both | H once | Creates `enth-challenge` (L4: in `layers.toml`, no crate yet) and pre-declares solver, session, UI, plugin, CLI, xtask and CI files; state-blob v2 field reservations; three header hooks (Recreate, Presets, Add); per-area token files; pins | main | 0.5–1 | 1 |
| **FB** Friend build and OS CI | S1, needed by GR1 | L (CI) | `friend-build.yml`, bundle config, `docs/distribution.md`, the friends' install sheet | Nothing; 04 **[A3]** | 1–2.5 | 1 |
| **SOAK** One-hour REAPER soak | S1 | L (bundle cache) | `xtask/reaper/soak.lua`, a `soak` scenario | Nothing | 0.75–1.5, plus a 1 h run | 1 |
| **EV2** Agent evals to about 30 goals plus reference sounds | R1 | L | `content/agent-evals/` | Nothing; the recreate track needs CL | 1–2 | 1, then 4 |
| **CT** Challenge types (→ F1r) | R1 | L | `enth-challenge`: `model`, `code`, `score/mod`; fixtures; `docs/challenge.md` | F0r | 1–1.5 | 2 |
| **SG** Session game API (→ F1r) | R1 | L | `enth-session/src/game.rs`, the threading contract, the state-blob game field | F0r | 1–1.5 | 2 |
| **RG** Randomizer | R1 | L | `random`, `priors`; property tests; a 1,000-hash golden | F0r (`RandomSpec` pre-declared from design §9) | 1–2 | 2 |
| **PBC** Preset core | S1 | L | `enth-session/src/presets.rs` (format, index, tags, search), the plugin's preset I/O | F0r; format reviewed at F1r **[A4]** | 0.75–1.5 | 2 |
| **AM** Add/remove modules, richer init | S1 (R1's Highest needs it) | H | `editor/add_module.rs`, additive intents, the Add hook, the init patch | F0r **[A5]** | 1.25–2.5 | 2 |
| **PM** Poly-mod proof in a scratch host | S1 | H (clack) | `tools/test-host` poly-mod mode | F0r | 0.75–1.5 | 2 |
| **SKR** Round skeleton (→ F2r) | R1 | H | Plugin game wiring, `enth challenge new\|show`, `enth compare` (stub), the Recreate hook, an A/B toggle in `process-stats`, 5 golden challenges | F1r, RG's lowest difficulty **[A1]** | 2.5–4 | 3 |
| **SS** Sound scorer | R1 | L | `score/sound`, `calibrate`, the scored-path loudness, the shared A/B level match | F1r **[A2]** | 1–2 | 3 |
| **ST** Structure, combined and time | R1 | L | The rest of `score/`, canonicalisation, equivalences | F1r **[A2]** | 1–2 | 3 |
| **SN** Sensitivity and coach | R1 | L | `enth-solver`: `sensitivity`, `coach` | F1r | 0.75–1.5 | 3 |
| **MT** Match | R1 | L | `cmaes` (ported from `spike-metric`), `nearest` (cut to R2 if late) | F1r | 0.75–1.5 | 3 |
| **PBU** Preset browser UI | S1 | H | `editor/browser.rs`, the Presets hook, REAPER `preset-load` | PBC | 1.25–2.5 | 3 |
| **GC** Golden challenges | R1 | L | `tests/golden/challenges/`, the `golden.yml` step, analysis determinism on Windows and macOS | SS, ST (minimal), RG | 0.75–1.25 | 3–4 |
| **PR** Factory presets | S1 | L | `content/presets/`: about 20 CLI-made candidates plus the owner's G1 four | PBC's format | 0.75–1.5 | 3–4 |
| **RUI** Recreate UI | R1 | H | `editor/recreate/*` (bar, results drawer, randomizer settings, overlay), agreement buttons, `sizes/recreate.rs` | F2r, SS, ST | 1.5–3 | 4 |
| **CL** CLI for R1 | R1 | L | `compare`, `suggest`, `match`, `explain`, `challenge`; insta snapshots; `docs/agent-cli.md` | SS, ST, SN, MT | 0.75–1.5 | 4 |
| **GK** GR1 kit | R1 | L | Round-log schema v1, `cargo xtask gr1-report`, friend and owner sheets | SG, CT | 0.5–1 | 4 |
| **INT** Integration | both | H | GR1 and GS1 tables, owner sheet 3, the friend release, stale docs | Everything | 1.5–2.5 | 5 |
| **LV** Layout variant | S1 | H | `editor/layout/<variant>.rs` and its snapshots | The owner's pick | 1–2 each | When picked |

## 2. Findings

### F1. P1 actuals in this repo (*measured*)

- **P1b lane hours**, from `notes.md` birth to merge:

  | Lane | Hours | Lane | Hours | Lane | Hours |
  | --- | --- | --- | --- | --- | --- |
  | S0b | 0.75 | S2s | 1.4 | ED2 | 1.5 |
  | E+ | 1.07 | SK | 3.5 | FK2 (unplanned) | 1.3 |
  | UI-C | 0.9 | RH | 0.9 | RT | 1.3 |
  | S1s | 1.2 | ED1 | 1.1 | INT | about 1.5 |
  | TH | 1.35 | PL | 1.33 | | |
  | FK | about 1.3 | | | | |

  The sum is about 20 h. EV, a non-P1b lane beside them, took 1.2 h. Caveats: the lanes overlap, and coordinator time is excluded.
- **The full gate took 197–335 s** over nine merge logs.
- **REAPER flaked once:** at the ED1 merge, 2 of 219 checks failed, then passed on the rerun.
- **The machine at 05:30:** 15 GiB total, 9 GiB available, load about 7. The two-heavy-builds rule stands.
- **What carries over:**
  - pre-declared files kept merges quiet;
  - a judge at F1 caught naming problems cheaply;
  - the skeleton lane is the one that runs long;
  - composites (goldens, budgets, whole-editor snapshots, tables) stay the coordinator's.

### F2. Prior art and failure stories

**Recreate games and ear trainers**

- **Syntorial's parameters "are notched"**, for example five attack times (MusicTech, 2014-02-11, *documented*).
  - Version 2.0 added an orange middle score tier and remapped the fixed values for "fewer frustrating roadblocks" (Best Service listing, release date 12.01.2024; vendor text, *documented*).
  - Lesson: partial credit and reachable, readable steps. Generated values should snap to steps a player can reach **[A2]**.
- **Syntorial's support page lists three issues:** disagreeing with a score, a hidden patch that can't be matched, and an unreachable value. Its feedback "will automatically attach your patch and the hidden patch" (updated 2024-12-02, *documented*).
  - enth can do the same with no server: a dispute button that writes the code, the player's patch and the score report to a file.
- **Syntorial's round lengths:** simple challenges take a few minutes, harder ones "10–30 minutes on average" (help page, updated 2024-07-27, *documented*). This is the only basis for sizing GR1's rounds.
- **SoundGym's EQ Mirror** players call the scoring opaque. A low-pass at 4.3 instead of 4.1 scored 27 %, a clearly wrong bell 75 %, and near-perfect attempts capped at about 80 % (SoundGym Cafe forum, undated, *anecdotal*). This is GR1's failure mode: non-monotone or capped scores.
- **Metrics disagree with ears.**
  - Parameter distance, spectrogram distance and listening agree only moderately, and the best loss "is highly dependent on the synthesizer" (Salimi et al., arXiv 2506.22628, v2 2025-10-08).
  - Spectral distances are bad at telling pitch direction (Turian and Henry, arXiv 2012.04572, 2020).

  Both are *documented*, and both support ADR-0007's modest `sound` weight until owner session 2 and the friends' ratings.
- **In music apps, perceived misgrading drives bad reviews:** 28 % of the 1–3-star reviews of five apps were about correct playing not being registered (Unstar, 2026; *anecdotal*, secondary).
- **Not found:** the r/synthrecipes browser matching game with daily and duel modes.

**Small plugin products**

- **Scope creep:** one developer spent two years on a first plugin. "You can always add something", so he shipped without auto gain compensation and put it first on the update list (BPB, 2026-07-31, *anecdotal*).
- **The work never ends:** OS and DAW changes keep forcing it (Baby Audio interview, undated, *anecdotal*).
- **Distribution advice:** a small private beta of strong testers, code signing early, and testing across DAWs and OS versions (Moonbase, 2025-05-05, *anecdotal*). 04 owns the facts.
- **Playtests:** 5 users find about 85 % of usability problems; three rounds of 5 beat one round of 15 (Nielsen, 2000-03-18; *documented* model). For GR1: a friend smoke round, then a fix, then the real round.

**Determinism**

- **Lockstep games need bit-identical maths;** transcendentals were the AMD-versus-Intel break (Fiedler, 2010, *documented*).
- **Factorio checksums the game state every tick** and diffs the first divergent tick in replays (FFF #47, 2014, *documented*). The enth analogue is a target-hash prefix in each code.

### F3. Test strategy for game logic

**Already proven** (*documented*):
- Renders are bit-identical on Linux, Windows, macOS arm64 and wasm32 (72 goldens; macOS once, 2026-10-19).
- The scalar FFT, features and descriptors are pinned and equal on x86-64 and wasm32.

**Not proven:**
- those analysis pins on Windows and macOS (they are not in `golden.yml`);
- `FINDINGS_SCALAR` under wasmtime;
- `ebur128` LUFS, which is tolerance-equal only and sets metric v1's gain.

**1. Target determinism** (parameter space only, design §9)

- **Generation is pure** in `enth-challenge`: SplitMix64 `derive` plus `enth-params` scales through pinned `libm`, with no render anywhere.
- **Keyed streams:** each value comes from `derive(derive(seed, fnv1a64(category)), fnv1a64(param_addr))`, so a new parameter or module never moves old draws. This is the voice-seed rule applied to the generator.
- **Priors are versioned code** (`priors::v1`). A change means a new generator version.
- **Code v1** holds the seed, `RandomSpec`, the generator and engine versions, a 32-bit target canonical-hash prefix and a checksum.
  - Decoding regenerates the target and compares the prefix.
  - A mismatch is an error with a hint, never a different target.
- **The 1,000 (seed, settings) → hash golden** runs in `enth-golden`: Linux, Windows and wasm on every push, macOS weekly. In the Phase 0 spike, `std` maths changed 13 of 1,000 seeds on glibc.

**2. Property tests** (fixed seed and 256 cases in the gate, 10,000 nightly)

| # | Property |
| --- | --- |
| P1 | `decode(encode(c)) == c`; corrupted codes give an error, never a panic (fuzz-lite) |
| P2 | Generated patches pass `validate`, respect `Constraints`, and stay in musical bounds (level, resonance, pitch) when `musical` is high |
| P3 | Difficulty is monotone (category sets nest); per-category `amounts` override the knob |
| P4 | Reachable: randomized parameters are editable and in range; disabled modules aren't randomized |
| P5 | Minimum step from `start`, which also guards the 0–100 calibration against a near-zero start distance |
| P6 | Keyed-stream stability: with an extra test parameter, shared parameters draw the same values |
| P7 | Score in 0–100; start = 0, target = 100 |
| P8 | Structure equivalences score 100: module off ≡ absent, depth 0 ≡ no route, permuted identical oscillators, enharmonic tunings **[A2]** |
| P9 | Structure is monotone along any single parameter toward the target |
| P10 | Sound stability: a 1e-3 relative parameter move changes the score by < 0.5 point; a nuisance seed or phase gives ≥ 99. These render, so 32 cases in the gate, 1,000 nightly |
| P11 | A/B level matching and scoring share one gain function |

**3. Golden challenges** (lane GC)

- **About 24 cases:** 5 difficulties × 4 seeds, plus `musical = 0`, structure randomized, the maximum patch, and a near-silent target.
- **Each case records:** the code, the versions, the target's canonical and render hashes, three reference answers (exact, start, near miss), the scores to 1e-6, and pass/fail.
- **Each case asserts |margin to threshold| ≥ 1e-3**, so no golden sits where a last-bit gain flips it.
- **Scorers are additive** (design §6): new versions add expected columns, old values never change.
- **Plugin side:** a `host-render` case where the target played through the plugin equals `enth render` of the target (the `cli-equal` pattern).

**4. Scoring across platforms**

- **Solo R1:** a tolerance is enough.
- **R5 needs one of two:**
  - (a) an in-house BS.1770 loudness on `libm`: K-weighting biquads, 400 ms blocks with 75 % overlap, −70 LUFS and −10 LU gates, a fixed summation order. About 150–250 lines (*estimated*); `ebur128` stays for display and true peak;
  - (b) a deterministic tie rule within 1e-4 of the threshold.
- (a) is cheap enough for SS now, and it removes the only platform-dependent input from the goldens.

**5. Game and plugin tests**

- **The round state machine** lives in `enth-session`, with time injected by the shell.
- **kittest round flow** by label: Recreate → bar → Target ▶ / Mine ▶ → submit → results → agreement → next.
- **No-peek test:** no label or tooltip shows a target value in Recreate mode.
- **Real time:** `process-stats` with A/B toggled every 250 ms, reported like RT's edit row. A plan swap between different structures copies effect state (RT measured p99 337 µs, 25.2 %) **[A1]**.
- **The UI never blocks:** scoring and hints run on the background worker, and a stalled-scorer test still gets frames.
- **Agent baseline:**
  - a deterministic CMA-ES solve rate on the golden challenges, as a nightly floor;
  - the LLM agent through the CLI, on demand, with model, date and cost recorded.

**6. S1 tests**

- **SOAK:** 60 minutes in REAPER under Xvfb, debug bundle (`assert_process_allocs`). It plays notes and slot automation, loads a preset every minute and opens the editor every 2 minutes. RSS, threads and fds are recorded per minute against the leak-gate slopes.
- **Presets** load through state on the main thread, never through automation (nih-plug #92), and each round-trips byte for byte.
- **Poly-mod:** a scratch clack host modulates one voice and checks that only that voice moves (FK's rename-check pattern). Bitwig stays optional.

### F4. Owner and friend time (*estimated*)

| Owner item | Time | When | Default |
| --- | --- | --- | --- |
| Friend survey: OS, DAW, 20 rounds OK? | 5 min | **Now** | GR1 owner-only; friends join in S1 |
| Metric session 2 (already pending) | 15 min | Before owner build 3 | Structure-leaning weights |
| Owner build 3: 5 low-difficulty rounds, real-display check of Add, the browser and Recreate, pick 10 of about 24 presets (GS1) | 60–75 min | After INT | — |
| GR1: 20 rounds with a 5-minute soft cap, logged automatically | 60–100 min, 3–4 sittings | After build 3's fixes | — |
| Layout pick | 20–30 min | When variants arrive | Lanes |
| Bitwig poly-mod | 45 min | Optional | PM's proof |

- **The owner's total:** about 2.5–3 h new, plus 3.5–4.5 h of P1 backlog (owner builds 1–2, real audio, metric session 2, reverb A/B, G1 listen).
- **Keep it small:**
  - one sheet per sitting, with defaults;
  - logs instead of notes;
  - build 3 doubles as the preset pick and the UI check;
  - nothing on the agent path waits for the owner before INT.

**Friends:**
- **Time:** install takes 10–20 minutes. 20 rounds take 60–100 minutes in 2–4 sittings, at 2–5 minutes per low-difficulty round (extrapolated from Syntorial; no enth round has been timed).
- **Two friend rounds (Nielsen):**
  - a smoke round: one friend per OS, 5 rounds, then fixes;
  - GR1: 3–5 friends, 20 rounds each.
- **Each friend sends one file,** the round log: agreement per round, "another round?", time to pass, disputes. `cargo xtask gr1-report` aggregates the logs, with no network code and no telemetry.
- **Suggested GR1 reading,** for the summary to settle:
  - "mostly agree": at least 70 % "about right" overall and no rater under 50 %;
  - "want more": "another round?" answered yes on at least 60 % of rounds.

### F5. The plan in detail

**Freezes.** After F2r, challenge, session and plugin types change only additively.

| Freeze | Merged | Gate before the fan-out |
| --- | --- | --- |
| **F0r** | S0r | The Done-means gate, judged by exit status |
| **F1r** | CT and SG: Challenge v1, `RandomSpec`, code v1, `Scorer` and `ScoreReport`, the session game API, state-blob v2 fields, PBC's format | A judge agent (Opus) reviews them together: they are persisted and go to friends and, later, duels |
| **F2r** | SKR | 5 golden challenges green on Linux, Windows and wasm; the kittest round flow; the A/B toggle measured; `validate` and `reaper-test` green |

**Waves** (two heavy slots; at most 4 light lanes building at once)

| Wave | Heavy | Light | Starts when |
| --- | --- | --- | --- |
| 1 | S0r | FB, SOAK, EV2 (new files only) | The owner has read the summary and the docs are updated (AGENTS) |
| 2 | AM, PM | CT, SG, RG, PBC | F0r |
| 3 | SKR, PBU | SS, ST, SN, MT; then GC, PR | F1r (PBU: after PBC) |
| 4 | RUI; AM or PM if still running | CL, GK, EV2's recreate track, GC | F2r |
| 5 | INT; LV when picked | — | All merged, then owner build 3 → friend smoke round → fixes → GR1 |

**Hotspots, defused up front**

| Hotspot | Who edits it |
| --- | --- |
| `Cargo.lock`, `[workspace.dependencies]` | S0r, then the coordinator. If 04 picks the standalone wrapper, nice-plug's `standalone` feature pulls `cpal`, `jack` and `midir` (vendored manifest), so it needs the owner's OK under AGENTS' system-library rule |
| The editor header | S0r declares three no-op hooks; AM, PBU and RUI fill their own |
| State blob v2 | S0r reserves optional `game` and `preset` fields with serde defaults; SG and PBC fill them; v1 fixtures still load |
| `enth-challenge` | CT (`model`, `code`, `score/mod`), RG (`random`, `priors`), SS (`score/sound`, `calibrate`), ST (the rest of `score/`) |
| `enth-solver` | SN and MT add files; `hint/` is untouched |
| `tools/test-host` | PM in wave 2; SKR's A/B flag is additive, in wave 3 |
| `tests/golden` | GC adds `challenges/`; engine cases are unchanged |
| Docs, one owner each | `challenge.md` (CT, plus RG, SS and ST sections), `distribution.md` (FB), `presets.md` (PBC, PBU), `agent-cli.md` (CL), the GR1 and GS1 tables (INT) |

**Acceptance per lane**

| Lane | Acceptance |
| --- | --- |
| S0r | Gate green. Every stub compiles. A probe dependency fails `--locked` |
| FB | Windows x86-64 and macOS arm64 artefacts (plus whatever 04 adds) pass `validate` on their runners. Install sheet written. One friend per OS loads the plugin |
| SOAK | 60 minutes green twice in a row. Slopes reported. A leaky control is caught |
| EV2 | About 30 goals plus reference sounds with fixtures; `eval_version` bumped; baseline recorded with model, date and cost |
| CT | Challenge v1 and code v1 fixtures. P1. Judge review done |
| SG | Session tests for the round lifecycle, A/B, submit, give up and injected time. Threading contract written |
| RG | P2–P6 and the 1,000-hash golden pass. A check that `random` never reaches `enth-render` |
| SS | P7, P10 and P11. Score components ("brighter", "attack too fast"). In-house loudness, or the margin rule documented |
| ST | P8 and P9. Documented combined formula. Injected time |
| SN, MT | Identical results for 1 and 8 workers. MT reports the golden-challenge solve rate |
| SKR | The F2r row |
| PBC | Format fixtures. Search and tag tests. Loading through state is asserted |
| PBU | kittest browse, search and load. REAPER `preset-load` green on both formats |
| AM | Add and remove every kind, with undo. Leak gate unchanged. Coordinator updates the whole-editor snapshots |
| PM | Per-voice modulation proven, with a failing control |
| GC | About 24 cases on Linux, Windows and wasm (macOS weekly). Analysis determinism in the matrix. Margin assertion |
| PR | About 20 tagged candidates that round-trip. The owner's pick recorded |
| RUI | Round-flow and no-peek tests. Snapshots. Agreement and dispute files. 0 idle frames kept |
| CL | Versioned schemas. insta snapshots. Errors carry a `hint` |
| GK | `gr1-report` on fixture logs gives the GR1 numbers |
| INT | Full gate, leak test, soak and REAPER green. Tables filled except the owner's and friends' rows. Owner sheet 3 written |

## 3. Risks (top 8, each with its cheapest retirement)

1. **Score disputes: "it sounded the same and scored 60."** Metric v1 is provisional and its calibration half done; research shows only moderate metric-to-ear agreement; SoundGym and Syntorial show the complaint is common. *Retire:*
   - agreement buttons and a dispute file from SKR on;
   - shown components;
   - GR1 at low difficulty, where structure % weighs more;
   - metric session 2 before owner build 3;
   - disputes become the fixture set for metric v2.
2. **Target drift.** A code regenerates a different target on another version (sequential draws, priors edits) or platform (`std` maths). *Retire:* keyed streams, versioned priors, versions and hash prefix in the code, and the 1,000-hash golden on the CI matrix.
3. **Sound % is not bit-exact across machines** (the `ebur128` gain): harmless solo, fragile for CI goldens, breaks R5. *Retire:* in-house `libm` loudness in SS, or the tolerance, margin rule and GC margin assertion.
4. **The friend build doesn't run** (Gatekeeper, SmartScreen, a DAW without CLAP, Logic needing AU, a standalone's audio device). *Retire:*
   - the survey now;
   - FB in wave 1 on GitHub runners;
   - a one-friend-per-OS smoke round;
   - the fallback of owner-only GR1 with friends in S1, as the roadmap allows.

   **[A3]**: 04 decides the form.
5. **Owner bandwidth:** 3.5–4.5 h of backlog plus 2.5–3 h new, and owner gates have set the end date in every phase so far. *Retire:* three sheets with defaults, combined sittings, automatic logs, and nothing on the agent path waiting before INT.
6. **Scope creep across two milestones.** "Polished" has no edge, and coaching, `explain`, nearest-solution, LV and AU can each eat a week. *Retire:* acceptance tables as the only definition of done; a pre-agreed cut list (nearest-solution, categorical hints, AU and signing move to R2/S2 if late); LV off the critical path.
7. **Instant A/B costs real time or clicks, or doesn't match what is scored.** Two engines double CPU; swaps between structures copy state (p99 337 µs, 25.2 %). *Retire:* SKR measures the toggle before RUI builds on it, and P11's shared gain function **[A1]**.
8. **Merge hotspots and contention: two tracks in the same crates.** *Retire:* S0r's pre-declarations and hooks, coordinator-owned composites, additive-only after F2r, heavy lanes paired with light ones.

Watch list:
- unwinnable challenges where an inaudible randomized parameter is punished (P4, sensitivity weighting in ST, and a nightly informational audibility audit that never feeds generation);
- REAPER flakiness in the soak;
- agent-baseline cost (on demand only).

## 4. Gaps

- **Reports 01–04 were not readable.**
  - **[A1]** The A/B mechanism, two engines or plan swaps (01, 03). If it needs engine work, an engine lane joins the spine before SKR, as E+ did in P1b.
  - **[A2]** Structure % and value snapping (02).
  - **[A3]** Friend-build form and signing (04).
  - **[A4]** Preset format (03).
  - **[A5]** Whether Add module belongs in R1's spine (03).
  - **[A6]** Coach-hint scope (02).
- **No enth round has been timed.** The owner and friend times are extrapolated from Syntorial's help pages.
- **The friends' OS and DAW are unknown** until the survey.
- **Unmeasured determinism:** `ebur128` equality on Windows and macOS, the analysis pins there, and `FINDINGS_SCALAR` under wasmtime. Run `CARGO_TARGET_WASM32_WASIP1_RUNNER=wasmtime cargo test -p enth-analysis --target wasm32-wasip1 --test determinism`, and add `-p enth-analysis --test determinism` to `golden.yml`.
- **The standalone wrapper's dependencies** (`jack`, `cpal`, `midir`) were not licence-checked or built: `cargo deny` and the owner's OK are needed.
- **The P1b hours undercount:** they start at each `notes.md` birth and exclude coordinator time.
- **Thin prior art on disputes:** one undated SoundGym thread, and the r/synthrecipes game was not found.
- **No builds were run.**

## 5. Sources

**Local** (2026-10-20): git merges `29ed7b0`…`a662b0b`; `~/code/enth-scratch/p1b/*/notes.md`; `~/code/enth-scratch/merge-*-{gate,reaper}.log`; `crates/enth-analysis/src/{features,distance,fft,loudness}.rs`; `docs/dsp/analysis.md`; `.github/workflows/{golden,ci,nightly}.yml`; `xtask/layers.toml`; `crates/enth-ui/Cargo.toml`; `crates/enth-ui/src/editor/cards/intent.rs`; `vendor/nice-plug/crates/nice-plug/Cargo.toml`; `docs/phase-1a.md`, `phase-1b.md`, `p1b-g1.md`; `_local/handoff/coordinator.md`.

**Web** (all fetched 2026-10-20):
- MusicTech, Syntorial review, 2014-02-11: https://musictech.com/reviews/audible-genius-syntorial-review/
- Best Service, Syntorial 2.0 (release 12.01.2024): https://www.bestservice.com/en/syntorial_2_0.html
- Audible Genius help, patch issues, updated 2024-12-02: https://audiblegenius.helpjuice.com/en_US/problem-or-issue-with-a-specific-patch-in-a-syntorial-challenge
- Audible Genius help, challenge length, updated 2024-07-27: https://audiblegenius.helpjuice.com/en_US/how-long-does-it-take-to-complete-each-lesson-in-syntorial
- SoundGym Cafe forum, EQ Mirror thread (undated): https://www.soundgym.co/space/index
- Salimi, Hindle and Zaïane, arXiv 2506.22628 (2025-06-27, v2 2025-10-08): https://arxiv.org/abs/2506.22628
- Turian and Henry, arXiv 2012.04572 (2020-12): https://arxiv.org/abs/2012.04572
- Unstar, music-learning apps ranked (2026): https://unstar.app/blog/yousician-simply-piano-flowkey-fender-play-music-learning-apps-ranked-2026
- Fiedler, "Floating Point Determinism" (2010): https://gafferongames.com/post/floating_point_determinism
- Factorio Friday Facts #47 (2014): https://factorio.com/blog/post/fff-47
- Nielsen, 2000-03-18: https://www.nngroup.com/articles/why-you-only-need-to-test-with-5-users/
- Sudara, Moonbase, 2025-05-05: https://moonbase.sh/articles/the-last-mile-part-i-getting-ready-to-launch-your-first-plugin/
- Zlatic, BPB, 2026-07-31: https://bedroomproducersblog.com/2026/07/31/making-vatra/
- Attack Magazine, Baby Audio interview (undated): https://www.attackmagazine.com/features/interview/a-smooth-operator-caspar-bock-founder-of-baby-audio/
