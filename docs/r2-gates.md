# R2: the GR2 table

- Date: 2026-10-22 (R2 lane INT2). Main at `fb299ae` (every R2 lane merged: S0l, CLS, LM, CV2, SL, F1L, PLY, SKL, LCK, C1, C1FIX, LUI, DEC, C2, EV3, LFU, C3, REV) plus lane INT2's branch `r2/int2`.
- Where the gate comes from: [roadmap.md](roadmap.md#r2--learn-ongoing) (GR2: the lessons cover every P1 module, and you and your friends learn something new from them) and [phase-3.md](phase-3.md#the-gr2-and-gr3-checks) (the checks, who runs each, when). The format follows [r1s1-gates.md](r1s1-gates.md).
- Status words: **passed** (measured or green, with the source), **pending owner** (your eyes, ears, hands or a decision; [owner sheet 4](r2-owner.md)), **pending friends** (your friends' tours), **pending CI** (a workflow that has not run on the tree yet), **open** (not done; the reason is given). Nothing is failed.
- Short version: **every automated GR2 check passes.** The mechanical half (coverage and honesty) is green on all four packs. The human half (you and your friends learn something new) waits for you: the kit is built (`cargo xtask gr2-report`, the test forms, the sheets), and the friend build with packs is rehearsed locally and ready to cut.

## GR2: the lessons cover every P1 module, and you and your friends learn something new from them

| # | Check | Status | Source |
| --- | --- | --- | --- |
| G1 | **The lessons cover every P1 module**: every module and Basic-tier parameter has a step and a non-patch check (`lesson-coverage`) | **passed**: `cargo xtask lesson-coverage` exit 0, "no hole": 4 packs (`basics`, `shaping`, `space-and-dynamics`, `studies`), 42 lessons; all 10 tables (seven module kinds, `voice`, `master`, the macro knob) have lessons and a non-patch check; **54 of 54 parameters taught, 24 of 24 Basic-tier**; no table "E only". Listed apart: two parameters checked by ear only (`comp.ratio`, `macro.value`); five taught but not checked (`lfo.scope`, `lfo.phase_mode`, `lfo.seed`, `comp.knee`, `voice.pan`: phase-3's Q9, **yours to accept**, row G7); seven ear questions `sound/1` cannot compare (transposed options or different phrases: a listening review decides) | LCK (`7500bca`), C1–C3, DEC; INT2's run on `r2/int2` |
| G2 | **Every lesson is honest**: the start fails, the witness passes with a margin of at least 1e-3, the solver reaches the pass, choices are audible, `--cross` holds | **passed**: `cargo xtask lesson-check --cross --solve` exit 0: **5 packs (the example included), 43 lessons, 422 steps, 0 failures**, 73 notes (steps that teach structure the hints cannot move, ear questions for a listening review); `--cross`: 102 Do steps, 714 check–witness pairs, none confused. 317 s in the tool (535 s with its release build) at a load of 25–36 | LCK, C1FIX (`2a7d216`), C2, C3, DEC; INT2's run |
| G3 | **Packs build reproducibly and match their pins** (`pack --check`) | **passed**: exit 0. `basics` 0.1.0 160,022 bytes, SHA-256 `f2b86e97…` (= the plugin's bundled archive); `example` 4,026 bytes `9ba7e94e…` (= its pin); `shaping` 148,956 bytes `28c12ee4…`; `space-and-dynamics` 108,770 bytes `e8f58553…`; `studies` 104,439 bytes `445a7ec9…` (no catalogue pins them yet: `content/packs/catalogue.sha256` waits for a release) | LCK; INT2's run |
| G4 | **A build friends can run, with packs** | **pending owner**: ready to cut. `release.yml` now packs `Packs/` (the three packs the plugin does not bundle), `gr2-friends.md` and `gr2-answers.toml` into each zip (INT2, `58fa6a4`). **The Linux leg rehearsed locally** at `620a67f`: clap-validator 0.4.1 on the debug and release bundles (36 passed, 0 failed, 8 skipped each), the release bundle, a **14.3 MB zip** with the plugin (`.clap` 19,723,472 bytes), the three packs and both friend sheets; the bundled `basics` archive plus the zip's `Packs/` read through the plugin's verified reader as **four packs, 42 lessons, 419 steps, the tour 13 of 13, none refused** (`enth lesson list`). The REAPER `lesson` scenario plays the bundled pack's first lesson on the default bundle (row A3). Not run on Windows or macOS: the pack step's lesson tool passes `clippy -D warnings` for `x86_64-pc-windows-gnu` and `aarch64-apple-darwin` locally. Next: you run `release.yml` ([owner sheet 4, step 5](r2-owner.md#5-decide-gr2s-criteria-and-the-friend-round-cut-the-friend-build-20-min-plus-runner-time)) | FB (`release.yml`), LFU (basics bundled, `f24dbdf`), INT2 ([distribution.md](distribution.md#cutting-a-friend-build)) |
| G5 | **You and your friends learn something new**: parallel-form pre- and post-tests, a staggered mid-test, "name up to three things you learned", reported per person | **pending owner and friends**. The kit is built: `cargo xtask gr2-report <dir>` (INT2, `c560685`, `d192c61`) reads each person's answers file (`enth.gr2-answers/1`), progress file and round log and prints the items, each family better, unchanged or worse, the staggered mid-test, the lessons' votes and the criteria; hand-made fixtures pin every number (three people, a host-restored round, an assisted pass, a pass after the limit, a reveal; 15 tests). The test forms: six families × forms A, B, M, D, 24 long codes ([content/gr2/](../content/gr2/README.md)): every item's start fails (sound at most 80 %); the CMA-ES floor passes on every item of four families and fails on 7 of the 8 `modulation` and `effects` items. Three differences from report 01's protocol, each because the plugin cannot do it yet: six families (generator categories), no "what changed?" items, per-lesson votes typed into the answers file ([owner sheet 4, step 5](r2-owner.md#5-decide-gr2s-criteria-and-the-friend-round-cut-the-friend-build-20-min-plus-runner-time)) | INT2; **you and your friends** ([r2-owner.md](r2-owner.md), [gr2-friends.md](gr2-friends.md)) |
| G6 | **The pass criteria** (report 01 §7, proposed) | **pending owner**: every participant names a specific new thing in at least 3 lessons; the median participant improves on at least **2 of the 6** families (report 01's 3 of 10, scaled) and no family gets worse on average; you name something new and pass a post-test item you failed before | You (owner sheet 4, step 5) |
| G7 | **Controls no ear question resolves are "taught, not checked"** (Q9) | **pending owner**: the five in G1 (`lfo.scope`, `lfo.phase_mode`, `lfo.seed`, `comp.knee`, `voice.pan`), plus the seven ear questions for a listening review | LCK; you |
| G8 | **GR2's friend time** (phase-3's open check) | **settled on the default** (INT2): the full protocol where a friend agrees (about 3 h), otherwise the tour with per-lesson votes and the debrief (1½–2 h); the pre/post rows from you and the full-protocol friends. Yours to change | INT2 (owner sheet 4) |
| G9 | **Taste check: lessons 1.1 and 2.1** | **pending owner** (20–30 min; [content/lessons/TASTE-basics.md](../content/lessons/TASTE-basics.md)). C1's bulk went ahead on report 01's rubric, as phase-3's default said | C1; you (owner sheet 4, step 3) |
| G10 | **Owner build 4: the Learn view on the real display** (the browser, highlight and lock, "How close am I?", hide, reopen mid-lesson, Review, feel) | **pending owner** (45–60 min) | LUI, LFU, REV; you (owner sheet 4, step 4) |

What is left of GR2: G4 (cut the friend build), G5 (your tests and tour, the friends' tours, the report), G6 and G7 (your decisions), G9 and G10 (your sittings).

The automated checks GR2 rests on:

| # | Check | Status | Source |
| --- | --- | --- | --- |
| A1 | **The agent lesson track** (an agent completes lessons through `enth lesson` from the text alone) | **passed for the two fakes**, recorded by EV3 (track version 1, engine 0.3.0, 2026-10-21, results in `fc581c2`, merged `8411d10`): `witness` (the ceiling, a reachability proof) 16 of 16 lessons, 107 of 107 graded steps; `follower` (**the floor**: hints only, first option on ear questions) **0 of 16 lessons, 32 of 107 steps** (do 7 of 19, rounds 11 of 45, ear questions 14 of 43, the first-option rate). **Pending owner** for a model's run (paid, only with your OK) | EV3 ([agent-evals.md](agent-evals.md#the-lesson-track-r2-lane-ev3)) |
| A2 | **The Learn view in kittest** (two lessons by label, no peeking in lesson rounds, the overflow report at 1200 × 800, 0 idle frames, overlay snapshots; Review's snapshot) | **passed** in INT2's full gate (2,454 tests, 0 failed; below) | LUI (`89167f2`), LFU, REV (`bede6d9`) |
| A3 | **A lesson in REAPER** (the `lesson` scenario on the default bundle: a step, the progress file, a save and reload mid-lesson) | **passed**: 12 of 12 on CLAP and VST3 in INT2's `reaper-test` (321 checks, below) | SKL, LFU; INT2's run |
| A4 | **The leak gate with Learn on by default** | **passed**: 14.53 KB per cycle (limit 26), threads constant, churn flat (below) | LK; INT2's run |
| A5 | **The one-hour REAPER soak** | see "Gates on INT2's branch" | SOAK; INT2's run |
| A6 | **Binary size with `basics` bundled** (report 02 put a TLS stack at 2.2 MB) | **measured**: the release `.clap` (Linux x86-64, default features `recreate` and `learn`) is **19,723,472 bytes** (15,208,840 stripped) at `620a67f`; lane LFU measured 19,504,816 with `learn` against 16,626,216 without (+2.88 MB, the pack 0.16 MB of it); the 218 KB since are R3's create work. No budget is set | LFU ([learn.md](learn.md#learn-follow-ups-lane-lfu)); INT2's bundle |
| A7 | **Progress, the player and Review keep their laws** (merge never shrinks, no pass without a result, locks hold, Leitner moves, a deterministic queue) | **passed** in the full gate (proptests at the fixed seed) | LM, PLY, REV |

## Integration (lane INT2)

### Gates on INT2's branch

Each judged by its exit status, on `r2/int2` (main `fb299ae` plus INT2's commits), 2026-10-22.

| Gate | Result |
| --- | --- |
| `scripts/gate.sh full .` | **exit 0** at `620a67f` ("GATE PASSED (full)", 2,814 s at a load of about 25): fmt, workspace clippy (178 s), layers, wasm-check, `cargo deny` (bans, licenses, sources), `cargo xtask golden` (engine goldens unchanged), the whole workspace's tests (**2,454 passed, 0 failed**, 42 ignored; 2,384 s), `host-render` (10 cases on the debug bundle; cli-equal 63 of 63 equal, 10 skipped by design) and `validate` |
| `cargo xtask lesson-check --cross --solve` | exit 0 (G2) |
| `cargo xtask lesson-coverage` | exit 0 (G1) |
| `cargo xtask pack --check` | exit 0 (G3) |
| `xvfb-run -a -s "-screen 0 1920x1080x24" cargo xtask reaper-test --package enth-plugin` (under `flock -x`) | **exit 0: 321 checks**, CLAP and VST3, every scenario green (417 s; the lock taken at a load of 8). `lesson` 12 of 12 on each (16.8 / 22.4 s): the bundled basics pack's first lesson starts, ▶ Saw, A peaks 0.669 on the track's meter with the transport stopped, Show me passes the Try step and the progress file records it 0.4 / 0.6 s later, and a new REAPER process resumes the lesson mid-way; `round` 10 of 10, `editor-smoke` 5 of 5, `dirty-flag` 18 of 18 |
| `cargo xtask leak-test` (under `flock -x`) | **exit 0, 5 checks** (442 s, at a load of 23): the editor grows **14.53 KB per cycle** over its baseline (slopes 13.90 and 14.59; limit 26), 3 threads after every close, 200 of 200 first frames in time in both runs, (200, 200) builds and `editor_closed`; instance churn 0.44 KB per cycle (−0.05 in the second half; limit 1.0), +0 threads, +0 descriptors |
| `cargo xtask reaper-soak --minutes 60` | <!-- int2:soak --> |
| `cargo xtask validate` (debug) and `--release --no-build` | exit 0 each: 36 passed, 0 failed, 8 skipped (the rehearsal, G4) |
| `cargo clippy -p enth-lesson-tool --target x86_64-pc-windows-gnu` and `aarch64-apple-darwin`, `-D warnings` | exit 0 both (the release legs' pack step) |
| `cargo test -p xtask` (`gr2-report` and its fixtures) | in the full gate (15 `gr2` tests) |

### What INT2 built and changed

| What | Where |
| --- | --- |
| `cargo xtask gr2-report`: reads plain files, so it lives in xtask (S0l's planned move); the tool's `gr2` stub and its stubs-test row went; round logs go through `gr1-report`'s reader (its `code`, `logs`, `privacy` and `schema` modules became `pub(crate)`, additive) | `xtask/src/gr2.rs`, `xtask/src/gr2/`, `xtask/fixtures/gr2/` |
| The test forms and the answers template | `content/gr2/` |
| The friend zip with packs and GR2's sheets | `.github/workflows/release.yml` (Package steps; not run) |
| Owner sheet 4, the friend sheet, this table | `docs/r2-owner.md`, `docs/gr2-friends.md`, `docs/r2-gates.md` |
| learn.md's status and as-built GR2 paragraph; Review pointers in session.md, plugin.md, editor.md; phase-3's friend-time check settled; the friend zip in distribution.md and friends-install.md | docs |
| Out of lane, one-word status fixes: r3.md's file-drops row is Built (fork patch 11 landed); architecture.md's tool list loses `gr2` | docs |

### Open

| Item | Why | Next |
| --- | --- | --- |
| GR2's "what changed?" items | The plugin plays ear questions only inside lessons | A GR2 test pack, or Choose items in Review (REV's open item) |
| A "new to me / knew it" tap and "what did you learn?" in the Check step | Not built (learn.md's lesson shape has them) | A Learn-view lane; `gr2-report` would read them from the progress file |
| The Macros, Glide and Compressor level lessons have no test item | The generator has no macro category; no form's code drew glide or the compressor | Accept, or make codes that draw them (the legato phrase, the compressor) |
| `modulation` and `effects` items mostly fail the CMA-ES floor | `sound/1` moves little for an LFO or a reverb on a held note (starts 66–80 %) | Read them on structure %; a future scorer change re-makes the codes |
| `release.yml`'s pack step on Windows and macOS | Release runs only on a tag or by hand | Your first run checks it (cross-target clippy is green) |
| Pre-existing broken anchors (not INT2's): editor.md `#hint-highlighting-lane-hint-fu1`; phase-3.md and architecture.md `challenge.md#challenge-v2-planned-lane-cv2-r2` | Headings renamed by earlier lanes | DOCS3 or the coordinator |
