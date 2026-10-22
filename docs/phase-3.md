# Phase 3 — R2 Learn and R3 Sound → patch

**Goal: lessons that teach you something on the instrument you keep, and a "create from sound" you can trust with a real file.** R2: lessons of 5–10 minutes, one control or idea each, played inside the plugin from bundled packs, whose "recreate this" steps are real Recreate rounds. Its gate GR2: the lessons cover every P1 module, and you and your friends learn something new from them ([roadmap.md](roadmap.md#r2--learn-ongoing)). R3: "create from sound" in the CLI and the plugin, first with search alone behind a robust intake (R3a), then with a small model's proposals (R3b). Its gate GR3: on held-out synth sounds model + search beats the best search alone at equal time; on real and degraded sounds a blind listening test prefers it; every result arrives in under 5 minutes ([roadmap.md](roadmap.md#r3--sound--patch-610-weeks)). The two milestones are planned together because they share the solver, the plugin's threads, the editor, the root manifests and your time. **Each holds one of the two heavy build slots.**

Size: R2 about **22–40 agent-hours of code plus 6–9 of content**, roughly 9–13 h of wall time judging by R1/S1 (which ran about 12 h against a planned 8–11). R3a about **14–18** agent-hours expected (14.8–26.5 estimated), R3b about **11–16** (10.5–20.5) plus 1–3 days of training runs. Critical paths: R2's S0l → LM + CV2 + SL → judge → SKL → LUI → INT2, about 7.5–13.5 agent-hours; R3a's S0s → TP → SK3 → PUI → INTa, about 8–11 h of wall time, starting once S0l has merged. **Content quality and your sittings set R2's end date; your listening and the training runs set R3b's.**

**Status (2026-10-21): planned; nothing is built.** Both research sessions are done; their decisions are provisional until you steer, and work proceeds on the defaults meanwhile. R2: [`_local/sessions/2026-10-21-r2-learn/`](../_local/sessions/2026-10-21-r2-learn/summary.md) (the summary wins where it corrects a report; report 03 §11 is the lane plan this page adapts) and lane CUR's [lesson plan](../content/lessons/PLAN.md). R3: [`_local/sessions/2026-10-21-r3-sound-to-patch/`](../_local/sessions/2026-10-21-r3-sound-to-patch/summary.md) (report 04 §5 is its lane plan). The designs of record are [learn.md](learn.md) and [r3.md](r3.md). [design.md §5 and §8](design.md#8-solver-pipeline-sound--patch), [architecture.md](architecture.md), [roadmap.md](roadmap.md), [challenge.md](challenge.md#challenge-v2-planned-lane-cv2-r2) and [README.md](README.md) were updated the same day (lane DOCS3).

New in phase 3: `enth-learn` (L5) is created; `enth-challenge` (L4) gains Challenge v2; `enth-solver` (L5) gains `closest` (R2), `create` and `propose` (R3), and its `nearest` stub becomes R3's k-NN retrieval; `enth-analysis` (L3) gains the intake and a robust distance; `enth-session` (L6) gains `learn` and `create`; `enth-ui` (L7) gains the Learn and Create views; `enth-plugin` and `enth-cli` (L8) gain the I/O, the jobs, `enth lesson` and `enth create`; `ml/` (Python, never shipped) and `tools/datagen` join for R3b; `content/lessons/` and `content/gr3/` fill with data. No lane edits `enth-dsp`, `enth-engine` or `enth-render`.

## What the research changed

Lanes should read these before the tables, because each changes what a lane builds.

**R2**

- **Hear before you are told.** A lesson is Hear → Name → Try → Do → Check, then Review in later sessions: A/B pairs that change only the new control come *before* the explanation, which the evidence favours over "explain, hear, do, check" (report 01 §1). Scaffolds fade (hidden, dimmed, normal), hints thin out by tier, and there is no spectrum during Do. [learn.md](learn.md#the-lesson-shape).
- **Lessons reuse the game.** A lesson's "recreate this" step is a real Recreate round (`RoundSource::Challenge`): A/B, rings, the 4-level coach and the results drawer come free. After a failed Submit, "How close am I?" runs CMA-ES from your own patch (`closest`), and after it, as after a Reveal, you solve a sibling challenge.
- **Challenge v1 cannot express a lesson** (only a seed target, a template start, no constraints, patch answers). **Challenge v2**, additive with a v1 migration, is R2's one persisted change, and a judge reviews it at F1l.
- **Sound alone cannot check every module** (the compressor, reverb on held notes, resonance, macros), so checks come in four kinds: patch state, sound expectations, Recreate scores and ear questions.
- **Bundle; do not download.** About 60 lessons zip to about 0.2 MB, while a TLS stack would add 2.2 MB to an 11 MB plugin and needs a C toolchain. R2 has no network code: bundled `.enthpack` packs, a Packs folder, and a button that opens the releases page in the browser.
- **The authoring loop comes first.** `cargo xtask lesson-check` plays every lesson's reference solution through a real session (start fails, witness passes, the solver can reach it); lesson-only commits run no CI today, so a `lessons.yml` workflow joins.
- **Learn lives in a new core crate**, `enth-learn` at L5, shared by the session, the CLI and xtask; architecture.md's L4 row loses "lessons".

**R3**

- **Nobody turns arbitrary real sounds into patches well, and nobody has tested degraded uploads.** Published work trains on the synth's own renders, and out-of-domain error roughly doubles (Hayes 2025). Only one small study compares model + search with search alone (Shier 2021, 7 parameters). GR3 will be new evidence.
- **Today's search copies the damage** (report 04 §2, *measured*, one target, scored against the clean target): clean 99 %, MP3 92 %, noise 10 %, reverb 25 %, clipping 64 %. Trimming the noisy tail alone raised 10 % to 56 %. So **R3a needs a robust intake and a degradation-aware objective before it ships**, and GR3 judges degraded uploads against the clean original.
- **R3a ships first, with no ML:** an intake, a robust objective under a new metric id (`sound/1` unchanged), a portfolio of starts (the heuristic, templates, k-NN retrieval over a pre-rendered bank), K branches refined by successive halving, on a cancellable pool of its own.
- **The first model is narrow:** a small CNN with **K = 8 winner-takes-all hypotheses** and a confidence head, whose categorical heads matter most, because that is where CMA-ES fails (level 5 solves 0 of 4 GR1 codes). It must beat k-NN retrieval and search alone.
- **Our own scalar kernels, not `tract`.** Any runtime is fast enough (18–160 ms against a 60 s budget), but `tract` adds about 15.5 MB to a 16 MB plugin, needs `cc`, `tract-onnx` does not build for wasm32, and it is not bit-exact across platforms (37–48 of 512 outputs equal, native against wasm32). Our naive kernels gave 512 of 512 on x86-64, `target-cpu=native` and wasm32, for about 0.1 MB. ONNX stays the export and parity format.
- **Generator v1 is unfit as training data** (level 1 has about 12 sounds, every phrase starts on C3, values lean towards the start), so a new sampler, `ds1`, sits beside it.
- **Real audio without labels is a hypothesis, not a plan:** first use is search-distilled pseudo-labels, capped at about 30 %.
- **A 60 s search cannot run on the plugin's game worker,** which is joined on drop: create jobs get their own thread and a per-job pool that cancels within one batch.

## How the conflicts were resolved

The summaries' own corrections stand ([R2](../_local/sessions/2026-10-21-r2-learn/summary.md#corrections-to-sub-reports), [R3](../_local/sessions/2026-10-21-r3-sound-to-patch/summary.md#corrections-to-sub-reports)). This plan settles the rest, provisionally:

| Conflict | Resolution | By |
| --- | --- | --- |
| Engine version: report 03 says packs pin the engine; report 02 says the engine is not a gate | A pack records the engine it was checked on and loads on any; `lessons.yml` re-runs `lesson-check` on every DSP change, and sound checks keep a margin ([design.md §5](design.md#5-determinism)) | This plan |
| GR3's sets: report 02 sizes them at about 2,000 + 42 synth, 500 degraded, about 150 real, 60–80 uploads; report 04 at about 120 targets and 60 references | GR3 runs on report 04's sets, frozen and hashed in wave 1. Report 02's `r3/test` stream is defined by its key and `ds1`'s version, never trained on, and used for development curves. Halving the preset candidates (the summary) leaves about 30 hand-made renders, so DS draws more generator targets to keep about 120 | This plan |
| The drop patch: wave 3 in the summary, beside SK3 | Lane FK3 in wave 2, in R3's otherwise idle heavy slot, so two R3 heavies never overlap | This plan |
| The F1s judge's findings have no lane (report 04) | Lane F1S, like R2's F1L; both earlier phases grew one | This plan |
| TP depends on F1s and also reaches it (report 04) | TP's API (`Candidate`, the `create` entry point, `enth.create/1`) and FE's intake types reach F1s together; TP's body continues after | This plan |
| `nearest.rs`: R2 leaves it to R3 as "the nearest preset as a start" | TP fills it with R3a's k-NN retrieval; R2's feature is `closest.rs` | Both summaries |
| Report 04 gives `docs/r3.md` to INTa | DOCS3 writes [r3.md](r3.md) now as the design of record; INTa updates its status and writes the R3a table in `docs/r3-gates.md` (planned) | This plan |
| GR2's friend time: report 01's protocol is about 2.5 h per friend; the summary says 60–90 min | Settled on the default by INT2: the full protocol where a friend agrees, else the tour with votes and the debrief ([below](#open-checks), [owner sheet 4](r2-owner.md)) | INT2 |

**Lane CUR's ten questions** ([PLAN.md §9](../content/lessons/PLAN.md#9-open-questions)):

| # | Question | Resolution | Owner |
| --- | --- | --- | --- |
| Q1 | The tour spans three packs; report 03 planned GR2 on "the basics pack" | **Decided by the summary:** the 12-lesson tour is GR2's friend track and is built first, so each content lane writes its tour lessons first, and GR2 waits for C2's and C3's. **Open:** how the app plays it; default a track (an ordered list of lesson references across installed packs), a field LM decides at F1l | LM |
| Q2 | A fourth pack, `studies`, owned by DEC | Not decided by the summaries. The plan takes CUR's default, four packs, so DEC owns `content/lessons/studies/` and no directory has two owners. If you want three, the studies go after 8.3 in `space-and-dynamics` | DEC; you |
| Q3 | `master` is missing from the 12; should 0.1 open the tour? | Not decided. Default yes, outside the count of 12 | LM, C1 |
| Q4 | Does `Target::Draw` draw route depths and their sign? | Not decided. Default yes: a route `src -> dst` in the draw list, values in % | CV2, at F1l |
| Q5 | A macro pin for sound checks, and a predicate form for the knob | Not decided. Default: the pin as a field of the sound check, `macro1` as the predicate form; 8.1 is gated by its ear question either way | LM (at F1l), PLY |
| Q6 | Is the ear question enough for "a non-patch check for every module"? | **Decided by the summary** (report 03 §4): yes, the four kinds exist for this. `lesson-coverage` reports "E only" separately | LCK |
| Q7 | Listen examples need a level-match switch, a render seed and a control curve | Not decided. Default: three optional fields of an example in lesson v1. SL checks whether examples can share the Target ▶ loop's buffer slot | LM, SL, at F1l |
| Q8 | The phrase table's invariants forbid short holds and pin the ids | **Partly decided:** new phrase ids are added (report 03's default; CV2 owns them). Default for the rest: split the table into generator phrases (ids 1–3, generator v1 untouched) and lesson-only phrases with their own invariants | CV2, at F1l |
| Q9 | A control no ear question can resolve | **Partly decided:** the audibility gate is `lesson-check --solve`'s (report 03 §9). Default: a control that fails it is "taught, not checked", listed by `lesson-coverage` for you to accept at GR2 | LCK; you |
| Q10 | No lesson on adding or removing a module | Not decided. Default: leave it; 1.4 and 1.5 mention it where the `osc2` and `noise1` switches appear | C1 |

## Freezes

Each milestone has a serial spine with its own freeze points; lanes fan out after each. **After F1l (lane F1L's merge), the persisted forms change only additively** (pack, lesson, challenge v2, progress, the `learn` object, the round log); **after F2l, the learn, challenge and session Rust types too** (SKL may still adjust APIs the real plugin needs). **After F2s, the create API, `enth.create/1` and the session's create types change only additively. After F3s, the model file changes only with a version bump.**

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0l** | S0l: `enth-learn` created (`enth-learn = 5`), every R2 file pre-declared, the `learn` field reserved, the Learn header hook, `sizes/learn.rs`, `content/lessons/` with one example pack, a `lessons.yml` stub | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)), judged by exit status; `wasm-check` covers `enth-learn` with `pulldown-cmark` and `rawzip` in it |
| **F1l** | LM, CV2 and SL: pack and lesson v1, the player API, progress v1, Challenge v2 with the v1 migration, the session's Learn API, `RoundSource::Challenge`, the `learn` field | **A judge agent (Opus)** reviews the three together: pack, lesson, challenge v2, progress, the `learn` field and the round's `origin` are all persisted. A named check: the largest lesson plus its round fit the 1 MiB `enth` field. Then lane F1L closes the must-fix items |
| **F2l** | SKL: one bundled lesson end to end in the real plugin | `lesson-check` green on it; kittest; the REAPER `lesson` scenario; `validate`, `host-render`, `leak-test` |
| **F0s** | S0s (stubs, the `ml/` skeleton, `content/gr3/`) and DS (the held-out sets with their hashes and exclusion lists) | Done-means green; `ml/` in no cargo or CI build; hashes committed; your picks or the default |
| **F1s** | FE's intake types (`Intake`, `Degradation`, `InferredPerf`) and TP's API (`Candidate`: patch, distance, provenance, trace; the `create` entry point; `enth.create/1`) | **A judge agent (Opus)**: the CLI, the plugin, the GR3 kit and R3b build on it. Then lane F1S |
| **F2s** | SK3: `enth create` and the plugin's "Create from sound" end to end | The cap enforced; cancel within one batch; `process-stats --gate g1` green with a create job running; time to the first candidate ≤ 15 s; `validate` and `reaper-test` green |
| **F3s** (R3b) | The model contract on our kernel op set: input features, outputs (K × continuous, categorical, structure flags, confidence), parameter order tied to the catalogue and generator versions, the model file's header | **A judge agent (Opus)**, before any long training run; a mismatched model header is refused, like `code_too_new` |

The coordinator commits both milestones' dependency pins first; **S0l and S0s both touch the root manifests (`Cargo.toml`, `Cargo.lock`, `xtask/layers.toml`), so they run one after the other, S0l first** (R2's F1l review gates the content, its long pole; R3's wave-1 light lanes are off R3a's critical path). After S0s, the coordinator alone edits `[workspace.dependencies]`, `Cargo.lock` and `[patch.crates-io]`; a lane that needs a dependency asks.

## Lanes

Sizes are agent-hours (estimates from reports 03 and 04, calibrated on P1b's and R1/S1's actual hours; sizes this plan added are marked). H = heavy local build (nice-plug, egui or clack), L = light. TR is runtime-heavy: a training run uses the machine alone. Every file or directory in the Owns column is planned (created by S0l or S0s, or by the lane) unless it exists today, such as `nearest.rs`, `game.rs`, `overlay.rs` and `tools/listen`.

**R2 — Learn**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **DOCS3** Docs (both milestones) | This page, [learn.md](learn.md), [r3.md](r3.md), and the summaries' doc rows in design, roadmap, architecture, challenge and README | The two summaries | 0.75–1 | L |
| **CUR** Lesson plan | `content/lessons/PLAN.md` | Report 01 | 0.5–1 | L; **merged** (`21c0114`) |
| **S0l** Workspace prep (→ F0l) | Creates `enth-learn` (`enth-learn = 5` in `xtask/layers.toml`, and the layer line in [AGENTS.md](../AGENTS.md)); pre-declares every R2 file below, `tools/lesson/` (`enth-lesson-tool`, exempt, holding the heavy dependencies) with one file per command, and the thin drivers `xtask/src/lesson/` and `xtask/src/gr2.rs`; reserves the `enth` field's optional `learn` object; the Learn header hook; `sizes/learn.rs`; an `enth-plugin/learn` feature; `content/lessons/` with one example pack; a `lessons.yml` stub | fu1-hint and fu1-fk merged; the coordinator's pins | 0.5–1 | H once |
| **CLS** Closest | `enth-solver/src/closest.rs` (design §7's nearest passing patch: CMA-ES from your patch over the step's editable set), its tests, a cost table | F0l | 1–2 | L |
| **LM** Lesson model (→ F1l) | `enth-learn`: pack, lesson, step and check types, the player (API and a basic body), progress v1 and its merge, the track field (Q1) | F0l | 1–1.5 | L |
| **CV2** Challenge v2 (→ F1l) | `enth-challenge`: model v2 and the v1 migration, `prepare_patch_round` (moved down from `enth-cli`'s `compare.rs`), the new phrase ids and the table split (Q8), [challenge.md](challenge.md#challenge-v2-lane-cv2-r2)'s v2 section | F0l | 1–1.5 | L |
| **SL** Session Learn API (→ F1l) | `enth-session/src/learn.rs` (stash, step loads, **locks in the edit path**, jobs, read model), `RoundSource::Challenge` in `game.rs` (additive), the `learn` field, [session.md](session.md)'s Learn section | F0l | 1–1.5 | L |
| **F1L** The F1l judge's findings | Its must-fix and should-fix items, in the files they name | The judge | 0.5–1.5 | L |
| **PLY** Player and checks | The predicate grammar (`Value`, `Route`, `Has`, `Enabled`, `Touched`), sound and choice checks, the markdown block AST on `pulldown-cmark`, the player's proptests | F1l | 1–2 | L |
| **SKL** Learn skeleton (→ F2l) | `enth-plugin/src/learn.rs` (pack I/O, the progress file, Learn jobs on the worker), a minimal panel, `enth lesson show\|check` (`enth-cli/src/cmd/lesson.rs`), the REAPER `lesson` scenario | F1l; PLY's predicates before F2l | 2–3.5 | **H** |
| **LCK** Lesson check | `tools/lesson/src/` (`check` with `--cross` and `--solve`, `coverage`, `new`, `pack`; `cargo xtask lesson-check`, `lesson-coverage`, `lesson new` and `pack` drive them), `lessons.yml`, the `gate.sh lane` hook for `content/lessons/` | F1l, PLY, CLS | 1–2 | L |
| **C1** Basics pack | `content/lessons/basics/` (16 lessons; the tour's six first, 2.5 after fu1-drv) | Drafts from PLAN.md's cards in wave 2; F1l to translate; LCK to verify | 2–3 | L, Opus |
| **LUI** Learn view | `enth-ui/src/editor/learn/*`, `learn.json`, the overlay's highlight and lock (additive in `overlay.rs`), the lesson browser and its Review entry, `sizes/learn.rs` | F2l | 2–3.5 | **H** |
| **DEC** Deconstruct and studies | `enth-learn/src/deconstruct.rs`, `tools/lesson/src/gen.rs` (`lesson-gen deconstruct`), the runtime job for user presets, `content/lessons/studies/` | F2l, CLS, fu1-drv | 1–2 | L |
| **C2, C3** Shaping; space and dynamics | `content/lessons/shaping/`, `content/lessons/space-and-dynamics/` (tour lessons first) | LCK; CV2's phrases | 2–3 each | L, Opus |
| **EV3** Agent lesson track | The lesson track in `content/agent-evals/`, a `follower` floor; the rest of `enth lesson` (`list`, `hint`, `closest`), additive in SKL's `lesson.rs` | SKL's CLI, CLS | 0.75–1.5 | L |
| **REV** Review | `enth-learn/src/review.rs` (Leitner boxes per skill: module kind + parameter, fed by Recreate's rings), progress's review field (additive), the session's review source (seeded sibling rounds) | LUI | 1–2 (this plan's estimate) | L |
| **INT2** R2 integration | `docs/r2-gates.md`, owner sheet 4 (`docs/r2-owner.md`), the friend sheet, `cargo xtask gr2-report` (`xtask/src/gr2.rs`), a friend build with packs | Everything | 1.5–2.5 | **H** |
| **DL** Download (cut list) | A non-default `learn-download` feature (`ureq` + `rustls`), or import only | Your networking decision, after GR1 | 1–2 | H |

**R3 — Sound → patch**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **S0s** Workspace prep (→ F0s) | Stubs for every R3 file below (solver `create/`, `propose/`; analysis `intake/` and the robust distance; session `create.rs`; plugin `create.rs`; CLI `cmd/create.rs`; UI `editor/create/`, `sizes/create.rs` and the Create header hook; `tools/listen/src/gr3.rs`; `xtask/src/gr3/`; `tools/datagen` (`enth-datagen`, an exempt package that also holds TP's bank builder, `bank.rs`, which `cargo xtask bank` drives); an `enth-plugin/create` feature (the decoders and `rayon`, optional) and an `enth-ui/create` feature); the `ml/` skeleton (uv); `content/gr3/` | S0l merged; the coordinator's pins | 0.75–1.5 | H once |
| **DS** Held-out sets | `content/gr3/{synth,real,degraded}.toml`, `hashes.txt`, `exclude-*.txt` | S0s; your picks or the default | 0.5–1 | L |
| **DEG** Degradation chain | `ml/enth_ml/degrade/` (seeded; held-out pools disjoint from training's) | S0s | 0.75–1.5 | L |
| **LT** GR3 listening | `tools/listen/src/gr3.rs` (`enth-listen gr3`, `--export-kit`, `--analyze`), its section of [listening.md](listening.md) | S0s | 1–1.5 | L |
| **FE** Intake (→ F1s) | `enth-analysis/src/intake/{trim,floor,bandwidth,clip,perf,resample}.rs` (pure, wasm32) | F0s | 1.5–2.5 | L |
| **OBJ** Robust objective | The degradation-aware distance in `enth-analysis`, under a new id (`sound/1` unchanged) | FE | 1–2 | L |
| **TP** Portfolio (→ F1s) | `enth-solver/src/create/{portfolio,schedule,candidate}.rs`, `nearest.rs` (k-NN retrieval), the bank builder (`tools/datagen/src/bank.rs`, driven by `cargo xtask bank`); the per-evaluation cost on R1's engine with effects | F0s | 1.5–2.5 | L |
| **EVK** GR3 kit | `xtask/src/gr3/`, `content/gr3/runs.jsonl`, `docs/gr3.md` (planned); the arm-A baseline | TP, DS | 1–2 | L |
| **F1S** The F1s judge's findings | Its must-fix items, in the files they name | The judge | 0.5–1.5 (this plan's estimate) | L |
| **FK3** Fork patch 11: file drops | `vendor/egui-baseview` (baseview's drag events mapped to egui's hovered and dropped files, `AcceptDrop(Copy)`), `vendor/README.md`, the ADR-0001 note | fu1-fk merged; F0s (for the slot) | 0.75–1.5 (this plan's estimate) | **H** |
| **SK3** Create skeleton (→ F2s) | `enth-cli/src/cmd/create.rs`; `enth-plugin/src/create.rs` (the `enth-create` thread, the per-job pool, cancel, decoders); `enth-session/src/create.rs` (jobs, candidates, "Use this" as one undo step, no host writes) | F1s | 2–3.5 | **H** |
| **CLC** CLI for R3 | `enth create`'s flags (`--perf` override), schema snapshots, [agent-cli.md](agent-cli.md) | SK3 | 0.75–1.5 | L |
| **PUI** Create UI | `enth-ui/src/editor/create/*` (drop zone, folder browser, K tiles, audition and A/B, progress, cancel) | F2s, FK3 | 2–3.5 | **H** |
| **INTa** R3a ships | The R3a table in `docs/r3-gates.md`, [r3.md](r3.md)'s status, owner sheet 5 (`docs/r3-owner.md`) | All R3a lanes | 1.5–2.5 | **H** |
| **ONX** Inference | `enth-solver/src/propose/{ops,net}.rs` (our scalar kernels; bytes passed in; F3C and F3S wrote `pack`, `features` and `decode`, the F3s contract, which ONX edits only additively), `tools/model-check` (exempt; parity against ONNX Runtime and `tract-nnef`), the kernel-hash step on every CI platform | F3s | 1.5–3 | **H** |
| **DG** Dataset generator | `tools/datagen/` (exempt): `ds1.rs`, `shards.rs`, `exclude.rs` (`bank.rs` is TP's) | F3s | 1–2 | L |
| **AUG** Augmentation | `ml/enth_ml/augment/`, the data manifest | DEG | 1–2 | L |
| **TR** Training | `ml/enth_ml/{model,train,export}/` | DG, AUG, F3s | 2–4, plus 1–3 days of runs | Runtime-heavy |
| **MS** Model + search | Model proposals in `create/schedule.rs` (successive halving over the K hypotheses) | ONX, TP | 1–2 | L |
| **EVb** GR3 runs | `content/gr3/runs.jsonl`, `docs/gr3.md` | MS, EVK | 1–2, plus about 11 h of machine time | L |
| **PUIb** Model in the plugin | Model-file lookup and loading, branches from the model | MS | 1–2 | **H** |
| **INTb** R3b integration | GR3's table in `docs/r3-gates.md`, the GR3 owner sheet | All | 1.5–2.5 | **H** |

**Totals:** R2 about 22–40 agent-hours of code plus 6–9 of content (REV adds about 1–2); R3a about 14–18 expected, R3b about 11–16 plus training; FK3 and F1S add about 1.25–3.

## Waves

**Two heavy slots: L for R2 and S for R3.** A slot holds at most one heavy lane, and a slot its milestone leaves idle never takes a third heavy build; it may hold two light lanes instead (a heavy build uses about the RAM of two light ones). Otherwise at most four light lanes build at once. Content, docs and Python-only lanes (C1–C3, DOCS3, DS, DEG, AUG) build no workspace crates beyond `lesson-check` and don't count. When a wave has more light lanes than room, the freeze-path lanes go first (in wave 2: LM, CV2, SL, FE and TP, in idle slot L's room; then OBJ and EVK). The two milestones advance on their own triggers, so a row's two halves need not start together.

| Wave | Slot L (R2) | R2 light | Slot S (R3) | R3 light | Starts when |
| --- | --- | --- | --- | --- | --- |
| 0 | — | DOCS3; CUR (merged) | — | DOCS3 | Now, while the fu1 lanes finish |
| 1 | S0l (→ F0l) | CLS, once F0l | S0s, once S0l has merged | DS (→ F0s with S0s), DEG, LT, once S0s has merged | **fu1-hint and fu1-fk merged**; the coordinator's pins for both milestones first |
| 2 | — | LM, CV2, SL (→ F1l, judged); C1 drafting from PLAN.md | FK3 | FE, TP (→ F1s, judged); OBJ; EVK | F0l; F0s |
| 3 | SKL (→ F2l) | F1L, PLY (first: SKL needs its predicates), LCK; C1 verified; your taste check | SK3 (→ F2s) | F1S, CLC; EVK on dev | F1l; F1s |
| 4 | LUI | DEC, C2, C3, EV3; REV after LUI | PUI | The arm-A baseline overnight (EVK) | F2l; F2s |
| 5 | INT2, then owner build 4, a friend round, GR2 | — | INTa, then your R3a check; **R3a ships** | — | All R2 merged; all R3a merged |
| 6 | DL, after your OK (cut list) | — | ONX | F3s (judged); DG, AUG; a pilot TR with a 10-sound pilot listen | R3a shipped |
| 7 | — | — | — | TR, MS; EVb on dev | ONX, DG |
| 8 | — | — | PUIb | EVb's test run; the listening kits | MS |
| 9 | — | — | INTb | GR3 listening; the report | PUIb |

- **The machine alone:** a timed run (the arm-A baseline, EVb's clause-1 runs) holds `flock -x /tmp/enth-quiet.lock` with no lane building, overnight; a training run (TR) uses the laptop's GPU overnight with no heavy build in either slot, attended the first time, under `systemd-run --user -p MemoryMax=6G`.
- **INT2 and INTa** never run REAPER, the soak or a timing step at the same time: both take `flock -x` for those steps.
- **Critical paths:** R2's S0l (0.5–1) → LM + CV2 + SL (1–1.5) → judge and F1L (0.5–1.5) → SKL (2–3.5) → LUI (2–3.5) → INT2 (1.5–2.5), about 7.5–13.5 agent-hours. R3a's S0s (0.75–1.5) → TP (1.5–2.5) → SK3 (2–3.5) → PUI (2–3.5) → INTa (1.5–2.5), about 8–11 h of wall time after S0l. Your gates and your friends' sittings come on top, and they usually set the end date.

## Acceptance per lane

Each lane stops at its own criteria and reports against them.

| Lane | Acceptance |
| --- | --- |
| **DOCS3** | Every "Docs to change" row of both summaries applied; every relative link and anchor it wrote resolves; file paths it cites exist or are marked planned |
| **S0l** | The Done-means gate is green. `wasm-check` covers `enth-learn` with `pulldown-cmark` and `rawzip`. Every stub compiles. The v1 state and challenge fixtures are unchanged |
| **CLS** | 1 and 8 workers give the same result. Renders and seconds for 1–4 free parameters on 10 real lesson starts |
| **LM** | Pack, lesson and progress fixtures. Refusals carry hints (a newer pack refused and named). `Progress::merge`'s laws as proptests (union of steps, max of bests, never shrinks) |
| **CV2** | v1 fixtures read as v2. The golden challenges and `enth compare --target` are byte-identical. New phrase ids with their invariants; generator v1's draws unchanged |
| **SL** | Lifecycle, lock, stash and budget tests: every op kind on a locked address refused (`SessionError::Locked`); one plan per step load; the stash survives a mid-lesson save; no audio-thread contact |
| **F1L, F1S** | Every must-fix closed |
| **PLY** | The player's properties pass (no panic; no pass without a passing result for the current revision; progress never shrinks; locks hold). Every CLI unit form parses in a predicate |
| **SKL** | The F2l row |
| **LCK** | Catches a seeded bad lesson of each kind (a start that passes, a witness that fails or touches a locked control, a dead link, an inaudible choice). Under 10 minutes in CI |
| **C1, C2, C3** | `lesson-check --solve` green on every lesson; coverage complete across C1–C3; C1 after your taste check |
| **LUI** | Two lessons by label in kittest; overlay snapshots; no-peek in lesson Recreate steps; the overflow report at 1200 × 800; 0 idle frames; `leak-test` |
| **DEC** | Lessons from 5 factory presets pass `lesson-check`; ablation cost and step quality reported on Acid Snap and Glass Pluck |
| **EV3** | `enth lesson list\|hint\|closest` with insta snapshots; the follower floor recorded with model, date and cost (paid runs only with your OK) |
| **REV** | Leitner moves as proptests; a skill's box follows its rings; the due queue is deterministic for a given progress file |
| **INT2** | The full gate, REAPER, leak and soak green. The GR2 table filled except your and your friends' rows. Owner sheet 4 written |
| **DL** | `cargo deny`; three OS legs; no request without a click |
| **S0s** | The Done-means gate is green. `ml/` in no cargo or CI build. Every stub compiles |
| **DS** | Sizes as [r3.md](r3.md#gr3-and-its-hygiene) says; licences read from the source; hashes match; exclusion lists committed |
| **DEG** | The same seed gives the same bytes; held-out pools are disjoint from training's (tested) |
| **LT** | A dry-run report; the bootstrap checked against a Python reference |
| **FE** | Trim within 5 ms; bandwidth within 1/3 octave; the note right on ≥ 95 % of pitched clips; a chord warning; short clips kept |
| **OBJ** | On 40 dev targets × 4 degradations, the median `sound/1` against the clean target improves on v1.1, and clean inputs stay within 1 point |
| **TP** | K branches; identical results for 1 and 8 workers; the schedule does not depend on wall time; the per-evaluation cost table |
| **EVK** | Four arms; statistics checked against hand values; wall-clock and render modes; the arm-A baseline recorded |
| **FK3** | A drop reaches egui as `dropped_files` in a test under Xvfb; the leak gate unchanged; [vendor/README.md](../vendor/README.md) lists patch 11 |
| **SK3** | The F2s row |
| **CLC** | A versioned schema (`enth.create/1`); insta snapshots; `--perf` overrides the inferred performance; errors carry a `hint` |
| **PUI** | The kittest flow (drop or browse, branches, audition, Use this, undo); 0 idle frames; the leak gate unchanged |
| **INTa** | The full gate, REAPER, leak green; the arm-A baseline in; your 5-sound check |
| **ONX** | `wasm-check`; output hashes identical on Linux, Windows, macOS arm64 and wasm32; parity with ONNX Runtime within 1e-5 on exported fixtures |
| **DG** | Deterministic shards; exclusions asserted; throughput reported |
| **AUG** | Licences recorded per source; no NC or SA data in shipped-model training |
| **TR** | Parity between PyTorch and our kernels on fixtures; a model card |
| **MS** | Deterministic given its proposals; a gain on dev |
| **EVb** | Clause 1 by its rule; the kits made |
| **PUIb, INTb** | Branches from the model; GR3's rows filled |

## Your part

| What | Milestone | Time | When | Default if you don't answer |
| --- | --- | --- | --- | --- |
| Read the two summaries | Both | 40 min | Now | — |
| The content licence: lesson text CC BY 4.0, patches and codes CC0, code MIT | R2 | 2 min | Before C1 | Proceed on it |
| Immutable releases (a repository setting) | R2 | 2 min | Any time | Releases unchanged |
| In-plugin download: `ring` needs a C toolchain, `graviola` is new | R2 | 2 min | After GR1 | No network code: bundled packs, the Packs folder, a browser button |
| Taste check: lessons 1.1 and 2.1, as markdown and through `enth lesson` | R2 | 20–30 min | Waves 2–3, before C1's bulk | Proceed on report 01's rubric |
| Owner build 4: the Learn view, highlight and lock, two lessons | R2 | 45–60 min | After INT2 | GR2 waits |
| GR2: pre-test, the path from placement (or the tour), post-test; the pass criteria | R2 | 1–2 h, 2–3 sittings | After owner build 4 | GR2 waits; report 01's proposed criteria |
| The degradation policy | R3 | 2 min | Before OBJ and AUG | Codec damage, hum, noise and device EQ are damage; for room reverb and compression the K candidates include a dry patch and one using the patch's own reverb or compressor |
| The inference path | R3 | 2 min | Before F3s | Our own kernels (`tract-nnef` behind a feature is the fallback, and needs your OK for `cc`) |
| MPL-2.0 for `symphonia` (adds M4A) | R3 | 2 min | Before SK3 | No MPL, so no M4A |
| A learned neural proxy in `ml/` | R3 | 2 min | Before TR | No |
| Renting a GPU (your account) | R3 | 2 min | Before TR | The laptop only |
| Real picks: 10 sounds you care about; OK the agent's CC0 picks | R3 | 30–45 min | Wave 1 (DS) | 40 CC0 picks by an agent blind to results |
| About 30 phone re-recordings | R3 | 30 min | Before the first training run | Synthetic phone EQ only |
| The first laptop GPU run, attended | R3 | 30 min | Wave 6 | CPU, or wait |
| R3a check: create from 5 of your sounds in the plugin | R3 | 20–30 min | After INTa | Ship on the kit's numbers |
| GR3 listening (66 trials, resumable); two friend kits | R3 | 25 min + 10 min | Wave 9 | GR3 waits; owner-only |

About 3–4 h new for R2 and 2.5–3.5 h for R3, after your earlier sheets ([owner sheet 3](r1s1-owner.md) and GR1's rounds). **Your friends:** GR2's tour with per-lesson votes; GR3's 30-reference kit (about 15 min) for two of them. Each sends files back by choice; nothing leaves anyone's machine on its own.

Until you say otherwise: **packs are bundled and R2 has no network code; the content licence is CC BY 4.0 for text and CC0 for patches; R3 runs on our own kernels with no MPL decoder, no neural proxy and no rented GPU; the degradation policy is the one above.**

## The GR2 and GR3 checks

The tables of record will be `docs/r2-gates.md` (INT2) and `docs/r3-gates.md` (INTa, INTb), both planned. The protocols are in [learn.md](learn.md#gr2) and [r3.md](r3.md#gr3-and-its-hygiene).

| Gate | Check | How | Who | When |
| --- | --- | --- | --- | --- |
| GR2 | The lessons cover every P1 module | `lesson-coverage`: every module and Basic-tier parameter has a step and a non-patch check; "E only" and "taught, not checked" listed apart | LCK, C1–C3 | CI from wave 3 |
| GR2 | Every lesson is honest | `lesson-check --solve`: the start fails, the witness passes with a margin ≥ 1e-3, the solver reaches the pass, choices are audible, `--cross` holds | LCK | Every content commit (`lessons.yml`) |
| GR2 | You and friends learn something new | Parallel-form pre- and post-tests of challenge codes, a staggered mid-test, "name up to three things you learned", reported per person. Proposed criteria (report 01 §7, yours to set): every participant names a specific new thing in at least 3 lessons; the median participant improves on at least 3 of 10 families and no family gets worse on average; you name something new and pass a post-test item you failed before | INT2's `gr2-report`; **you and your friends** | After owner build 4 |
| GR2 | A build friends can run, with packs | The friend build with bundled packs | INT2 | Wave 5 |
| GR3 | Clause 1: model + search beats the best search alone at equal time | About 120 held-out synth targets; arm A is R3a's best tuned search; equal wall-clock with inference charged, at 60 s (primary) and to 300 s; McNemar on pass rate, Wilcoxon on JND distance; one metric no arm optimises | EVK, EVb | Wave 4 (arm A), waves 7–8 |
| GR3 | Clause 2: a blind listening test prefers it | Forced choice "which is closer to the reference?", level-matched, top candidates only, catch trials; 60 references, 20 of them degraded and **judged against the clean original**; a cluster-bootstrap rule fixed in advance | LT, EVb; **you**, two friends | Wave 9 |
| GR3 | Clause 3: every result in under 5 minutes | A cap guarantees it; gated instead on time to the first candidate (≤ 15 s) and no xruns while a search runs in the DAW (`process-stats --gate g1`) | SK3, INTa | F2s; INTa |
| GR3 | Hygiene | Held-out sets frozen and hashed before any training; no priors fitted on test presets; every test run appended to `runs.jsonl` | DS, EVK, DG | From wave 1 |
| R3a | Ships | The full gate, REAPER, leak; the arm-A baseline; your 5-sound check | INTa; **you** | Wave 5 |

## Rules for every lane

- Branch from the latest freeze of your milestone, and write only the files your lane owns. One owner per doc: [learn.md](learn.md) (DOCS3, then a section each for LM, SL, PLY, LCK, LUI, DEC and REV; INT2 its status), [challenge.md](challenge.md)'s v2 section (CV2), [session.md](session.md)'s Learn (SL) and Create (SK3) sections, [editor.md](editor.md)'s Learn (LUI) and Create (PUI) sections, [plugin.md](plugin.md)'s Learn (SKL) and Create (SK3) sections, [agent-cli.md](agent-cli.md) (SKL's and EV3's `enth lesson`, CLC's `enth create`), [r3.md](r3.md) (DOCS3, then FE, OBJ, TP, ONX, DG and TR sections; INTa and INTb its status), [listening.md](listening.md)'s GR3 section (LT), `docs/gr3.md` (EVK, EVb), the gate tables and owner sheets (INT2, INTa, INTb). Report against your acceptance criteria when you stop.
- **Privacy.** Never put your email, name or other personal details in any request, header, URL or payload (crates.io, GitHub and dataset sites included); use `enth-research/0.1`. A dataset that needs an account (Freesound's originals) is fetched by you, not by an agent.
- **No windows on the real display** (Xvfb, as [AGENTS.md](../AGENTS.md#ui) says), and **never play audio**: the listening tools are tested with `--dry-run`.
- **Foreground commands** with a timeout above their measured length; a backgrounded agent can go idle and never resume. Keep a running `notes.md` in persistent scratch (`~/code/enth-scratch/r23/<lane>/`).
- **Concurrency.** Two heavy slots, one per milestone ([Waves](#waves)); 15 GiB of RAM is shared by every lane. `CARGO_BUILD_JOBS=3`, a separate `CARGO_TARGET_DIR` per lane on ext4, `flock -s` around builds and `flock -x` around timing runs. Build only the crates you need until the final gate. Check `df -h /home` before a big build; a training cache stays under about 70 GB.
- **A two-tier gate, judged by exit status only** (no `cmd | tail && echo ok`): `scripts/gate.sh lane <worktree>` before a lane reports, `gate.sh full` (the coordinator's) before a merge. A docs-only lane checks its links and anchors instead (as DOCS3 does), and a Python-only lane runs `ml/`'s own tests; the coordinator's full gate covers the rest.
- **No engine work.** Neither milestone needs any: no lane edits `enth-dsp`, `enth-engine` or `enth-render`, and no engine golden may change (fu1-drv's Drive fix is the coordinator's; C1's lesson 2.5, UT5's seeds and DEC's studies wait for it).
- **ML rules** (R3). `ml/` is in no cargo or CI build and never ships. Only MIT or Apache code goes into it (report 01: the copy rule holds there too, although it never ships; MPL code only with your OK); LGPL and GPL tools (LAME, ffmpeg) run only as subprocesses there; pedalboard, gpuRIR and fdk-aac are never imported. Shipped weights train only on our renders plus CC0, CC BY or Apache data; NC, SA and licence-less sets stay evaluation-only. Features are computed in Rust (`enth-analysis`), never re-implemented in Python. Logs stay local.
- **Held-out hygiene** (R3). The GR3 sets freeze with their hashes before any training; tuning happens on dev only; every test run is appended to `content/gr3/runs.jsonl` and none is dropped; the listening key stays closed until every listener is done; whoever prepares stimuli does not audition them.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshots per layout.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `Cargo.lock`, `[workspace.dependencies]`, `[patch.crates-io]` | The coordinator's pins (both milestones: `pulldown-cmark`, `rawzip`, `miniz_oxide` with `=`, `toml` for xtask and the CLI; `claxon`, `lewton`, `nanomp3`, `rayon` in the plugin), then S0l, then S0s; afterwards the coordinator only |
  | `xtask/layers.toml` | S0l (`enth-learn = 5`), then S0s (`tools/datagen` exempt); `enth-py` gets its line when a lane creates it (open check) |
  | `enth-solver` | CLS adds `closest.rs` (R2); TP adds `create/` and fills `nearest.rs`; ONX adds `propose/`; MS edits only `create/schedule.rs`. `hint/` and `search.rs` are untouched; a needed `pub` is additive and reported |
  | `enth-plugin`'s threads | fu1-hint's parallel hint evaluator lands first. SKL adds Learn jobs to the game worker (`learn.rs`, and `JobKind::PrepareChallenge` answered there); SK3's create work runs on its own `enth-create` thread and pool and never touches `game.rs`. S0l and S0s pre-declare `learn.rs`, `create.rs` and their hooks in `lib.rs` and `editor.rs` |
  | `enth-ui`'s editor | S0l and S0s declare the Learn and Create header hooks and the `editor/learn/` and `editor/create/` seams; LUI and PUI fill their own. The overlay's highlight and lock are LUI's, additive (reuse fu1-hint's hint highlight if it landed one). Tokens only in `sizes/learn.rs` and `sizes/create.rs` |
  | `enth-session` | SL (`learn.rs`, `RoundSource::Challenge` in `game.rs`, additive), SK3 (`create.rs`); `lib.rs` exports pre-declared |
  | The state blob | S0l reserves the optional `learn` object (absent when idle; state version 1); SL fills it; a create job is not saved |
  | `enth-challenge` | CV2 only (v2, the migration, `prepare_patch_round`, phrases). `ds1` reads generator v1's public pieces from `tools/datagen`; a needed `pub` is additive and reported |
  | `enth-analysis` | FE (`intake/`) and OBJ (the robust distance); existing files untouched |
  | `xtask/src/` | S0l pre-declares `lesson/` (LCK; `gen.rs` is DEC's) and `gr2.rs` (INT2); S0s pre-declares `gr3.rs` with `gr3/` (EVK: `gr3-run`, `gr3-report`) and `bank.rs` (TP's thin driver of `enth-datagen`, whose `bank.rs` is the work); `main.rs` registrations go in with the stubs |
  | `content/` | One owner per directory: `lessons/<pack>/` (C1, C2, C3, DEC), `gr3/` (DS; `runs.jsonl` EVK and EVb) |
  | CI workflows | S0l stubs `lessons.yml`, LCK fills it; ONX adds the kernel-hash step; nothing builds `ml/` |
  | `scripts/gate.sh` | LCK adds the `lesson-check` hook for `content/lessons/`, reviewed by the coordinator |
  | `vendor/` | FK3 only (fork patch 11) |
  | `tools/listen` | LT (`gr3.rs`), additively |
  | REAPER and timing | INT2 and INTa take `flock -x`; timed GR3 runs and training use the machine alone |

- **Ownership crossings** are additive only and listed in the report. Both earlier phases grew an unplanned lane; F1L and F1S are planned up front.
- **Risks** (report 03 §10 and report 04 §4 have the full lists, each with its cheapest retirement):
  1. bland or wrong lessons, the real cost of R2 (your taste check before bulk content, the honesty audit, per-lesson votes);
  2. coverage holes where sound cannot check (ear and patch checks, `lesson-coverage` in CI);
  3. search fitting the degradation (FE and OBJ before R3a ships; clean references in GR3);
  4. the domain gap on real sounds (R3a measures the real baseline first; a 10-sound pilot listen after the first small model; per-stratum reports);
  5. leakage into the held-out sets (the wave-1 freeze, disjoint preset halves, Freesound-id dedupe, disjoint impulse-response and noise pools);
  6. real time and lifetime in the DAW (a separate cancellable pool, capped workers, `process-stats` under search);
  7. format sprawl across pack, lesson, challenge v2, progress, the `learn` field and `enth.create/1` (two judge reviews, fixtures, strict inputs and lenient outputs);
  8. your bandwidth, with earlier sheets pending (defaults everywhere, resumable listening, automatic logs, combined sittings).

## Cut lists

Agreed in advance (provisional until you confirm), in order: if a milestone runs late, the first item goes first.

- **R2:** (1) DL, the in-plugin download (import from a file and the browser button stay); (2) REV, the Review queue, to after GR2; (3) DEC's runtime job for user presets (the factory studies stay); (4) the `studies` pack, to after GR2; (5) the non-tour lessons of C2 and C3 (GR2 runs on the tour); (6) EV3's paid runs.
- **R3** (report 04 §5.5): (1) the friends' listening (owner-only GR3); (2) Opus and AAC augmentation; (3) drag-and-drop (the folder browser stays); (4) the MUSHRA diagnosis; (5) the playability check; (6) a model download. Topology search and text-to-patch stay out until GR3, as the roadmap says. R3b as a whole may slip: R3a ships alone.

## Open checks

Not settled by the research; each is owned by a lane, by you or by a later milestone.

| Check | Owner | When |
| --- | --- | --- |
| `pulldown-cmark` and `rawzip` pass `wasm-check` inside `enth-learn` | S0l | F0l |
| `closest`'s cost on real lesson starts (renders, seconds, 1 vs 8 workers) | CLS | Wave 1 |
| ~~The largest lesson plus its round fit the 1 MiB `enth` field~~ **Settled:** 690,713 of 1,048,576 bytes with every part at its cap ([session.md](session.md#learn-lane-sl-r2), "The budget"); the preset bookmark is capped at 4 KiB so the rest stays bounded | F1l's judge; F1L | F1l |
| ~~Whether Listen examples can share the Target ▶ loop's buffer slot~~ **Settled:** yes, by time, as a set of loops (SL) | SL | F1l |
| ~~How the tour is played across packs (Q1)~~ **Settled:** a track, `pack/lesson` lists in the manifest (LM) | LM | F1l |
| ~~Route depths in `Target::Draw` (Q4), the macro pin (Q5), the Listen options (Q7), the phrase table's split (Q8)~~ **Settled:** routes drawn as signed depths; `macros` on a sound check and the `macro1` form; `level_match`, `seed`, `curve` and `transpose` on an example; the table split, ids 4–7 lesson-only ([learn.md](learn.md#the-lesson-model-lane-lm), [challenge.md](challenge.md#challenge-v2-lane-cv2-r2)) | CV2, LM | F1l |
| **A pack from a newer build** that fails on something this build lacks (a newer engine's patch, a scorer, a phrase) should be refused as "needs a newer enth" whenever its recorded `engine` is newer than this build's, rather than as a damaged pack (F1l review should-fix 6) | PLY | Before F2l |
| **A resumed lesson forgets which steps passed:** `Player::resume` starts its visits fresh, so after a reload a passed step looks unpassed (and could record again) until the player reads the progress file back (F1l review should-fix 7). Default: the shell passes the lesson's `Progress` entry to the player on resume (an additive call) | SKL | F2l |
| `enth compare --phrase` offers only the generator's phrases 1–3; the lesson phrases 4–7 resolve everywhere else (F1l review nit 9, not in lane F1L's list) | SKL (the next lane in `enth-cli`) | F2l |
| How long a lesson takes; the pane's overflow on preset-sized patches | C1, LUI | Waves 3–4 |
| Ablation cost and text quality on Acid Snap and Glass Pluck | DEC | Wave 4 |
| Drive lessons (2.5), UT5's seeds and the studies wait for fu1-drv's merge | C1, DEC | After fu1-drv |
| Four packs or three (Q2); 0.1 opening the tour (Q3); a lesson on adding modules (Q10) | DEC, C1; you | Before C3 |
| Controls no ear question resolves become "taught, not checked" (Q9) | LCK; you at GR2 | GR2 |
| ~~**GR2's friend time:** report 01's protocol (intake, two 25-minute tests, the tour, the mid-test, the debrief) is about 2.5 h per friend; the summary's 60–90 min covers the tour alone~~ **Settled on the default** (INT2, [owner sheet 4](r2-owner.md#5-decide-gr2s-criteria-and-the-friend-round-cut-the-friend-build-20-min-plus-runner-time)): the full protocol where a friend agrees (about 3 h with the tour), otherwise the tour with per-lesson votes and the debrief (1½–2 h), with the pre/post rows from you and the full-protocol friends; yours to change | INT2; you | Owner sheet 4 |
| The degraded-match result on 40 targets × 4 degradations (not 1) | OBJ | Wave 2 |
| Per-evaluation cost on R1's engine with effects (sizes K and the budget) | TP | Wave 2 |
| ~~**The k-NN bank:** its size, features and delivery~~ **Settled at F1s:** 20,000 generator specs with a 112-byte pitch-relative log-mel embedding (`knn-logmel-rel/1`) in `enth-bank-v1.bin` beside the plugin, regenerated by `cargo xtask bank`; R3a runs without it. Its hygiene (F1s should-fix 7): near duplicates of GR3 targets at any phrase, and near renders in embedding space against every GR3 synth target, test and dev, presets included, are skipped and re-asserted by `bank check` ([r3.md](r3.md#what-lane-tp-built-f1s-2026-10-21)) | TP, F1S | F1s |
| ~~Whether the intake needs `resample.rs` at all~~ **Settled:** no; render and analysis run at the file's own rate, and `resample.rs` is deleted | FE | F1s |
| A create job's effect on `process()` p99 in the plugin | SK3 | F2s |
| Whether REAPER drags media items into a plugin window as files | SK3 or FK3 | Wave 3 |
| Whether `nanomp3` and `lewton` decode bit-identically across platforms (a result records the decoded audio's hash either way) | SK3 | F2s |
| ~~Whether create candidates should survive a project reload~~ **Settled by the F1s judge:** no, keep the default ("Use this" puts the patch in the project) | F1s's judge | F1s |
| High notes (f0 above about 1 kHz) hide a codec's cut-off: an optional codec and bitrate hint from the decoder in `IntakeCfg` (F1s should-fix 2) | SK3 | F2s |
| 192 kHz files have no path: accept 192 kHz, or decimate exactly 2:1 in the decoder, and say which in the hint (F1s should-fix 10) | SK3 | F2s |
| Slow attacks under noise surface 5–35 ms late: extend the onset back along the attack's rise (F1s should-fix 6) | OBJ, or an FE follow-up | Before R3a ships |
| `min_spread` against the bank's size; `first_rung` (m0, 50) and K against the budget; the scored window (2, 3 or 5 s; `create::SCORE_S` is 3) (F1s should-fix 9 and the coordinator's decisions) | EVK, on dev | Wave 3 |
| ~~The CLI's `--score-seconds` (`Target::from_intake_scored`)~~ **Settled by CLC:** `enth create --score-seconds` (0.1–5 s, default 3) builds `Target::from_intake_scored` and the new `RobustMetricTarget::from_intake_scored`, equal to `from_intake` bit for bit at 3 s; the plugin keeps the default window | CLC | With `enth create`'s flags |
| OBJ masks on a floor only when `floor_confirmed`, and never on `clipping` for soft clipping or limiting (FE decision 7) | OBJ | Wave 2–3 |
| EVK: 12 of the 112 clause-1 generator targets are two-note phrases, but `create` plays one note in every arm: report them as their own stratum (the F1s judge) | EVK | Arm A |
| `enth.create/1`'s JSON test lives in `tools/datagen/tests/create_record.rs`: moving it into `enth-solver` needs a `serde_json` dev-dependency, one line in `Cargo.lock`'s `enth-solver` entry (F1s nit 14) | The coordinator | Any time |
| ~~Where `ds1` lives (default `tools/datagen`, reading generator v1's public pieces) and whether features reach `ml/` through `enth-py` (PyO3, L8) or files `tools/datagen` writes~~ **Settled at F3s:** `tools/datagen` runs the product's intake on each clip and writes shards (`mel`, `context`, `label` [102], `mask` [102], ids, the layout hash); no `enth-py` for v0 (r3.md, "The model contract (F3s)") | DG | F3s |
| Our kernels bit-exact on macOS arm64 and MSVC (run the scalar path on those CI legs) | ONX | Wave 6 |
| GPU training throughput on the laptop; the first PyTorch-exported model through our kernels | TR | Wave 6 |
| ~~Licences of AMR, GSM, WHAM! and OpenSLR 28's sources; Freesound's terms on training~~ **Answered by AUG** (`ml/enth_ml/augment/manifest.toml`, each source's licence and use): AMR-NB/WB Apache-2.0 with a 3GPP no-patent note and GSM's TU Berlin notice, subprocess and held-out only; WHAM! CC BY-NC, excluded; OpenSLR 28 Apache-2.0 with its simulated RIRs and point-source noises for shipped training and its real RIRs (AIR, RWCP, REVERB) research-only; Freesound allows training within each clip's licence, bulk only by Data Packs. Left for you: AIR's MIT licence on the allowlist, the point-source noises as CC0, a Data Packs request, FSD50K's commercial note, a MUSAN per-file audit | DS, AUG; you | Waves 1, 6 |
| **The roadmap holds "ML of any kind" until GR1**, which is still open. R3a has no ML; R3b's wave 6 does. Default: R3b's lanes proceed on schedule, and no model reaches a build before INTb; say so if you want wave 6 to wait for GR1 | You | Before wave 6 |
