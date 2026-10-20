# 03 — R2 Learn: architecture, code structure, risks and the lane plan

- **Date:** 2026-10-21. Sub-researcher 03 of the R2 Learn session ([brief.md](brief.md)).
- **Method:** the docs the brief names, the codebase-memory graph (project `enth`, index ready, no partial files) and grep on main at `9842936`, the git log for timings, the crates.io API for a few crates (User-Agent `enth-research/0.1`). Scratch: `~/code/enth-scratch/2026-10-21-r2/03/notes.md`.
- **Labels:** *measured* (read or run today), *documented* (a repo doc says so), *anecdotal*. Pack formats, rendering and delivery are report 02's; the curriculum and GR2's measurement are report 01's. This report decides where the code goes and in what order.

## 0. The answer in seven lines

1. **A new core crate, `enth-learn` (L5)**, holds the lesson model, the pure lesson player, the checks and the "deconstruct" planner. `enth-session::learn` (L6) binds it to the live patch. `enth-ui::editor::learn` (L7, feature `learn`) draws it. The plugin, CLI and xtask do the I/O.
2. **Challenge v1 cannot express a lesson** (*measured*): only `Target::Seed`, `Start::Template`, an empty `Constraints {}` and `AnswerKind::Patch`. **Challenge v2** is the persisted change the judge must review.
3. **A lesson's "recreate this" step should be a real Recreate round.** One additive `RoundSource::Challenge` gives it A/B, rings, the coach and the results drawer for free.
4. **Nearest-solution feedback is half built.** `enth-solver::hint` already is the smallest verified move for expectation steps. For patch targets, design §7.3's "closest passing patch" is unbuilt; `nearest.rs` is a different, R3-flavoured stub.
5. **Sound checks cannot cover every P1 module.** The compressor, reverb on held notes, resonance and macros have no usable descriptor (agent-evals findings 7–11). GR2 needs patch-state checks and ear questions too.
6. **Bundled packs are enough for GR2.** Downloading needs TLS, and both rustls crypto providers compile C (*measured*); the roadmap also defers networking until after GR1.
7. **Six waves, three freezes**, about 22–40 agent-hours plus 6–9 for content, a critical path of about 8–13 h. The owner's new time is about 3–4 h.

## 1. What exists today (*measured*, main `9842936`)

| Hook | Where | State | Gap for R2 |
| --- | --- | --- | --- |
| `LearnMeta { tier, desc }` | `enth-params/src/spec.rs` | Built; the randomizer uses the tier | None |
| `Overlay`: normal, dimmed, hidden, score ring | `enth-ui/src/editor/overlay.rs` | Built; Recreate dims and scores | **Highlight** and **locked**, additive |
| Reserved `lesson` region, `learn` feature | `editor/reserved.rs`, `enth-ui/Cargo.toml` | An empty panel; the feature is empty | A presenter, a Learn layout |
| `LessonPane`: headings, bold, figure, bullets | `widgets/lesson_pane.rs` | Gallery only | Fed by a markdown subset |
| The Recreate round: pure, jobs, stash, budgets | `enth-session/src/game.rs` (2,423 lines) | Built, judge-reviewed | The template for `learn.rs` |
| `Challenge` v1 | `enth-challenge/src/model.rs` | Recreate only; `prepare_challenge` refuses non-`Seed` | v2 (§3) |
| A "hand-made round" on a patch target | `enth-cli/src/cmd/compare.rs` (`moved_params`) | Built **in a shell** | Move it into `enth-challenge` |
| Coach, `hint`, `match` (CMA-ES) | `enth-solver` | Built | `closest` (§5) |
| `nearest.rs` | `enth-solver` (19 lines) | Stub, cut to R2; its design is "the nearest preset as a start for match" | Not R2's feature (§5) |
| The plugin's game worker | `enth-plugin/src/game.rs` | One lazy thread, `Serial` evaluator; fu1-hint is adding a parallel one | Learn jobs |
| `content/lessons/` | — | **Missing**, though architecture.md lists it | S0l creates it |
| CLI → session | `enth-cli/Cargo.toml` | No dependency on `enth-session` | Lesson logic must sit at ≤ L5 to be shared |

## 2. Where Learn lives

| Piece | Home | Why |
| --- | --- | --- |
| Pack and lesson model, `LessonRef`, compatibility, progress and its merge, the **player**, check specs and evaluation (patch predicates; expectations through `enth-render` and `enth-analysis`), the markdown AST, the deconstruct planner, coverage | **`enth-learn`, L5, core** | Shared by session, CLI, xtask and later wasm. It embeds `Challenge` (L4), so it must sit above L4. `wasm-check` takes every layer ≤ 5 (*measured*, `xtask/src/main.rs`), so the I/O bans and the wasm build come free |
| Challenge v2, `prepare_patch_round` (moved down from `compare.rs`), new phrase ids (a chord, a long hold, staccato) | `enth-challenge` (L4) | Persisted challenge types; phrases are RG's table, new ids only |
| `closest` (design §7.3) | `enth-solver/src/closest.rs` | Leaves `nearest.rs` to R3 |
| Live binding: stash, step loads, **locks in the edit path**, jobs, read model, the `learn` object in the `enth` field | `enth-session/src/learn.rs` | Like `game.rs`: no clocks, threads or I/O |
| The Learn view | `enth-ui/src/editor/learn/` | The `editor/recreate/` seam |
| Pack I/O, the progress file, Learn jobs on the worker | `enth-plugin/src/learn.rs` | Shells translate; the rules come from `enth-learn` |
| `enth lesson list\|show\|check\|hint\|closest` | `enth-cli/src/cmd/lesson.rs` | Agent evals; stateless like `enth check` |
| `lesson-check`, `lesson-gen`, `lesson-coverage`, `gr2-report` | `xtask/src/lesson.rs` | Tooling |

**Rejected:**
- **Lessons inside `enth-challenge`.** architecture.md's L4 row still says "lessons", but the crate is 11.5k lines with five owners, and packs have a lifecycle challenges don't.
- **Session-only.** The CLI and xtask could not share the player.
- **An L7 I/O crate.** Only the plugin needs one until a standalone ships.

**Job payloads are solver-native**, so `enth-learn` never calls its same-layer sibling `enth-solver`, and "the worker composes nothing" still holds:
- an expectation hint is `{patch, perf, items, free}` → `enth_solver::hint`;
- a Recreate-step hint is the game's coach job;
- closest is `{round, patch, free, budget}` → `enth_solver::closest`.

## 3. The lesson model and the state machine

```rust
// enth-learn (sketch; report 02 owns the file format)
pub struct LessonRef { pack: PackId, pack_version: u32, lesson: LessonId, step: u16, content: [u8; 8] } // sha-256 prefix
pub enum StepKind { Read, Listen { examples: Vec<Example> }, Do { check: Check },
                    Recreate { challenge: Challenge }, Choose { options: Vec<Example>, answer: u8 } } // Choose: ear question
pub enum Check { Patch(Vec<Predicate>), Sound { phrase: PhraseId, expect: Vec<Expect> }, All(Vec<Check>) }
pub enum Predicate { Value(Addr, Cmp, Plain), Route(Src, Dst, Cmp, Pct), Has(ModuleKind), Enabled(Id, bool),
                     Touched { addr: Addr, min_span_pct: u8 } } // "turn it and listen", from the edit log
pub struct StepView { layout: Option<String>, start: Option<PatchRef>, highlight: Vec<AddrPat>, hide: Vec<ModuleSel>, lock: Vec<AddrPat> }
pub enum StepPhase { Intro, Working, Checking { job: JobId }, Passed, Shown, Skipped }
pub struct Progress { lessons: BTreeMap<LessonKey, LessonProgress> } // per user; merge = union of steps, max of bests
```

**The lifecycle** (pure, `&mut self`, times from the shell):
1. `start` stashes the player's patch and loads the step's start.
2. While `Working`, patch predicates re-run after each delivered edit (microseconds). Sound checks run as a job once the patch settles (`SETTLE_MS`, 300 ms, as the game's loudness job does). A result for an older revision is refused.
3. `hint` gives the authored hints first, then measured ones. `show` applies the witness's next op as one undo step. Both mark the step assisted.
4. `next` and `back` move between steps; `end(Restore | Keep)` works as in the game.

**Recreate steps.** A `Recreate` step starts a game round through `RoundSource::Challenge(Box<Challenge>)`, additive, plus `JobKind::PrepareChallenge`, which the worker answers with `prepare_challenge`. The step passes when the round passes. Its log line carries `origin: lesson`, so `gr1-report` can leave it out.

**Per user vs per project:**
- **Per user:** progress, the GR2 votes and a later review schedule, in a file the shell owns (e.g. `<Documents>/enth/Learn/progress.json`). This follows `presets_dir` and `round_log_path`, with an `ENTH_LEARN_DIR` override.
- **Per project:** the `enth` field's optional `learn` object (absent when idle; state version 1, like `game`) holds only the in-flight `LessonRef`, its step and the stash. A reload resumes the lesson and brings your own patch back.
- **The budget** (*documented*): the field is 1 MiB, and a round already reserves 768 KiB, a 384 KiB stash included. So **lesson rounds never stash**: the `learn` object owns the only stash. F1l's judge checks that the largest lesson plus its round fit.

**Locks are the session's rule.** `deliver` refuses an op that touches a locked address or module (`SessionError::Locked`, with a hint), so every shell and test gets the rule. Policy to settle at F1l:
- host automation of a locked parameter folds, but never counts toward a check;
- undo never leaves the step;
- a preset load asks Restore or Keep.

## 4. Which checks work for which P1 module

Evidence: agent-evals.md's findings (*documented*, eval version 3), ADR-0007's note (short attacks move metric v1 by at most 0.003, *documented*), the phrase table (3 phrases, *measured*).

| Module | Sound (expectations) | Recreate score | Patch state | Ear question |
| --- | --- | --- | --- | --- |
| Oscillator wave, pitch | Yes; pulse and saw read alike (finding 9) | Yes | Yes | Pulse vs saw |
| Noise, filter cutoff | Yes (noisiness, brightness) | Yes | Yes | — |
| Resonance, drive | No resonance descriptor (finding 11); Drive loses level (fu1-drv fixing) | Partly | Yes | **Needed** |
| ADSR | Yes (beat-aware) | **Weak** for short attacks | Yes | — |
| LFO, routes | Only after two or more cycles (finding 8); not on pink noise (finding 1) | Yes | Yes | — |
| Reverb | Only on decaying sounds (finding 7) | Partly | Yes | **Needed** |
| Compressor | **No descriptor** (finding 10) | Weak | Yes | **Needed** |
| Voices, glide | Needs a chord or legato phrase (only `LEGATO_PAIR` exists) | With new phrases | Yes | Mono vs poly |
| Macros | No | No | Yes | Yes |

**GR2's "every P1 module" needs all four kinds.** `lesson-coverage` makes it mechanical: every module, its Basic-tier parameters, and at least one non-patch check. Ear questions (`AnswerKind::Choice`) are R4's shape (design §6), so they are no detour.

## 5. Nearest-solution knob feedback

- **Expectation steps:** `hint` returns a bounded Gauss-Newton move from the player's patch over every failing item. It is verified by renders, sparsified, and `blocked` when a bound stops it (*documented*: 10–16 renders a hint). That **is** nearest-solution feedback for these steps. Show it graded, as the coach does: aspect, then module, then parameter, then range.
- **Recreate steps:**
  - structure rings are instant, with no render;
  - design §7.3's closest passing patch (CMA-ES from the player's patch, penalising distance, per-parameter deltas) becomes `closest.rs`, with the free set limited to the step's editable parameters (usually 1–4).
- **Cost** (*documented*, dsp/match.md): a scoring render takes 15–30 ms of one worker, and level-2 `match` runs took a median of 423 renders. On the serial plugin worker, 100–400 renders is 1.5–12 s. So it is a **"How close am I?" button** with progress and cancel, not live. Lane CLS measures it on real lesson starts first.
- **Naming:** R2's feature is `closest`. `nearest` stays R3's "nearest preset as a start"; otherwise two sessions build different things into one file.

## 6. The editor's Learn view

- **Seam**, copied from Recreate: `Editor::set_lesson(LearnEdit::lesson())` before the frame, `Intent::Learn(LearnIntent)`, `Editor::apply_learn(&mut dyn LearnEdit)` after. A lesson's round uses the Recreate UI unchanged.
- **Overlay additions:**
  - `Emphasis::Highlighted`: a static accent outline, so no repaint timer;
  - a `locked` flag: a lock glyph, with the refusal on the status line as drag-to-modulate shows it;
  - markdown links such as `[Cutoff](enth:filter1.cutoff)` highlight their control on hover, and `lesson-check` verifies they resolve.
- **Layout** (*documented*, editor.md): a 320 px pane fits 1200 × 800 only when the step hides unused modules (the G1 patch overflows by 276–732 px). Full-preset deconstruct steps will overflow, so LUI needs a collapsible pane or a focus card region, with an overflow report.
- **Markdown:** a block AST from `enth-learn`, drawn through `LessonPane` with tokens only. `egui_commonmark`'s own styling would fight the token rule (report 02 judges it). `pulldown-cmark` 0.13.4 was released 2026-05-20 (*measured*, crates.io).
- **Repaint:** Learn asks for no frames of its own. Job results wake the editor, as `wake_at_ms` and the worker's `request_repaint` do for rounds.

## 7. Packs, I/O and the light build

- **Features:**
  - `enth-ui/learn` (exists) and a new `enth-plugin/learn` gate the view, the I/O and the bundled content;
  - a non-default `learn-download` gates any network code;
  - `enth-learn` and `session::learn` always compile, as the round's worker does "so a saved round restores in any build".
- **Bundle first.** Targets render on the fly (design §5), so packs are markdown, patches and codes: small. Report 02 has the sizes.
- **Download risk** (*measured* 2026-10-21, crates.io):
  - `rustls` 0.23.46's default features enable `aws_lc_rs` (`aws-lc-sys`, a C build);
  - the `ring` 0.17.14 alternative has `cc` as a build dependency;
  - either way that is a C toolchain on three OS legs, which needs the owner's OK (AGENTS.md);
  - `deny.toml` bans OpenSSL ("use rustls if networking is ever needed");
  - roadmap.md defers networking until GR1, which is pending.
- **Cheapest path:** "Import pack…" from a file, plus "Open release page" in the browser. No network code, and a download still happens only on the user's click.

## 8. Tests and verification

| Test | Where |
| --- | --- |
| Pack, lesson, challenge-v2 and progress fixtures; a newer pack refused with a hint; a newer progress file read leniently | `enth-learn/tests`, `enth-challenge/tests/fixtures/challenge/v2/` |
| The player under random intents, edits, ticks, results (right, stale, failed), saves and loads: no panic; no pass without a passing result for the current revision; progress never shrinks; locks hold (proptest, fixed seed) | `enth-learn/tests/player_props.rs` |
| Every op kind on a locked address refused; one plan per step load; the stash survives a mid-lesson save; the budget holds | `enth-session/tests/learn.rs` |
| **Scripted player:** every step of every lesson through a real `Session`, jobs run inline between frames (as `tests/recreate.rs` does): start fails, witness passes, Show-me reaches the pass | `cargo xtask lesson-check` |
| kittest: two lessons by label; overlay snapshots; no-peek in lesson Recreate steps; 0 idle frames (lavapipe) | `enth-ui/tests/learn.rs` |
| REAPER `lesson`: a step, then a save and reload mid-lesson | `xtask/reaper` |
| An agent completes lessons through `enth lesson` from the text alone; a `follower` floor | the agent-eval lesson track (paid only with the owner's OK) |

**CI gap** (*measured*):
- `ci.yml` ignores `**/*.md`, so lesson edits would run nothing. Add `lessons.yml` on `content/lessons/**`, `enth-learn` and the DSP, engine, patch and challenge crates: a DSP change can break a lesson (design §5).
- Make `gate.sh lane` run `lesson-check` when `content/lessons/` changes; `xtask affected` maps no crate to it today.
- Fail witness margins under 1e-3, as the golden challenges do.

## 9. An LLM-assisted authoring pipeline the solver verifies

1. **Scaffold:** `cargo xtask lesson new`. An authoring agent (opus) drafts from report 01's curriculum and the catalogue (`enth params`, `enth explain`), building start patches and witnesses with `enth new | apply | check | compare`, as EV2 wrote its goals.
2. **Honesty** (`lesson-check`, deterministic, no spend; agent-evals' rules for lessons):
   - the start fails; the witness passes with no `thin` item;
   - the witness touches only editable parameters;
   - `--cross`: a check does not pass its neighbours' witnesses;
   - links resolve; coverage is complete.
3. **Reachability** (`--solve`):
   - the hint follower reaches the pass from the start within the editable set. If it cannot, the step must teach structure, and its witness must show it;
   - `match` passes Recreate steps within a budget;
   - ear-question options are pairwise at least k JND apart under `sound/1`'s calibration.
4. **Review:** a second agent checks the prose against `enth explain`'s facts. It reports; it does not gate. Then the owner and friends (GR2, with report 01's per-lesson vote).

**Deconstruct lessons use the same pipe.** `deconstruct::plan(preset, ranks)` orders modules and routes by the sensitivity `reach` the caller measured, so the planner stays pure. Each step restores one element: a patch-target step, with text from `LearnMeta.desc` and the `explain` summaries. `lesson-gen deconstruct` writes factory-preset lessons at authoring time; the plugin worker runs the same planner on user presets.

## 10. Risks and unknowns

| # | Risk | Cheapest retirement |
| --- | --- | --- |
| 1 | **Coverage holes:** the compressor, reverb, resonance and macros are uncheckable by sound (§4) | Ear and patch checks; `lesson-coverage` in CI; descriptor follow-ups (findings 9–11) |
| 2 | **Bland or wrong LLM lessons**; content is the real cost | The owner tastes 2 lessons before bulk; the honesty audit; report 01's rubric; the friends' votes |
| 3 | **Engine drift breaks lessons** (Drive is changing now) | Packs pin `engine`; `lessons.yml`; Drive lessons after fu1-drv |
| 4 | **Format sprawl**: pack, lesson, challenge v2, progress, the `learn` field, the round's `origin` | One judge review at F1l; fixtures; strict inputs, lenient outputs (challenge.md's rule) |
| 5 | **The 1 MiB `enth` field** with a lesson and its round at once | One stash; a budget test |
| 6 | **Lost progress** when two instances write the per-user file | Re-read, `Progress::merge`, temp file and rename; a two-writer test |
| 7 | **The pane overflows** on big patches | Hide unused modules; a collapsible pane; an overflow report |
| 8 | **Closest is slow** on the serial worker | A small free set; a button; fu1-hint's parallel evaluator; CLS measures first |
| 9 | **TLS pulls C**, and networking breaks the GR1 rule | Bundle; import from a file; download only after the owner's OK |
| 10 | **Collisions** with R3 (solver, worker, editor header, `Cargo.lock`) and the four fu1 lanes (`hint` edits `enth-plugin/src/game.rs`) | `closest.rs`; S0l pre-declares every file and the hook; SKL after fu1-hint; the coordinator alone edits `Cargo.lock` |
| 11 | **Unfair envelope Recreate steps** (short attacks barely move `sound/1`) | Expectation checks for envelopes |
| 12 | **The owner's bandwidth**: about 6 h of earlier sheets are pending (handoff) | GR2 rides on owner build 4; defaults everywhere; automatic logs |

**Unknowns:**
- whether Listen examples can share the Target ▶ loop's single buffer slot;
- how many minutes a lesson takes;
- whether `toml` 1.1.8 and a markdown crate pass `wasm-check` inside `enth-learn` (S0l proves it).

## 11. The lane plan

**Freezes:**

| Freeze | Merged | Gate |
| --- | --- | --- |
| **F0l** | S0l: `enth-learn` created (`enth-learn = 5`), every R2 file pre-declared, the `learn` field reserved, the Learn header hook, `sizes/learn.rs`, `content/lessons/` with one example, a `lessons.yml` stub | Done-means, judged by exit status |
| **F1l** | LM, CV2, SL: pack and lesson v1, the player API, progress v1, Challenge v2 with the v1 migration, the session's Learn API, `RoundSource::Challenge`, the `learn` field | **A judge agent (opus)** reviews all three together; then F1L fixes |
| **F2l** | SKL: one bundled lesson end to end in the real plugin | `lesson-check` green on it; kittest; REAPER `lesson`; `validate`, `host-render`, `leak-test` |

After F2l, the learn, challenge and session types change only additively.

**Lanes** (agent-hours; H is a heavy local build: nice-plug, egui or clack):

| Lane | Owns | Depends | Size | H/L | Acceptance |
| --- | --- | --- | --- | --- | --- |
| **DOCS3** | `docs/phase-3.md`, new `docs/learn.md`, the summary's doc rows | Summary | 0.75–1 | L | Links resolve; lane gate green |
| **CUR** | `content/lessons/PLAN.md`: lessons × check kinds × modules | Report 01 | 0.5–1 | L | Every module and Basic-tier parameter has a step and a non-patch check |
| **S0l** (→F0l) | New crate, stubs, hook, tokens; pins (coordinator) | fu1-hint and fu1-fk merged | 0.5–1 | H once | Done-means; `wasm-check` covers `enth-learn`; v1 fixtures unchanged |
| **CLS** | `enth-solver/src/closest.rs`, its tests, a cost table | F0l | 1–2 | L | 1 and 8 workers give the same result; renders and seconds for 1–4 free parameters on 10 starts |
| **LM** (→F1l) | `enth-learn`: pack, lesson, step, check types, player (API and a basic body), progress | F0l | 1–1.5 | L | Fixtures; refusals with hints; merge laws as proptests |
| **CV2** (→F1l) | `enth-challenge` model v2, the migration, `prepare_patch_round`, phrase ids, `challenge.md` | F0l | 1–1.5 | L | v1 fixtures read as v2; golden challenges and `compare --target` byte-identical |
| **SL** (→F1l) | `enth-session/src/learn.rs`, `RoundSource::Challenge`, the `learn` field, `session.md` §Learn | F0l | 1–1.5 | L | Lifecycle, lock, stash and budget tests; no audio-thread contact |
| **F1L** | The judge's findings | Judge | 0.5–1.5 | L | Every must-fix closed |
| **PLY** | Predicate grammar, sound and choice checks, markdown, player proptests | F1l | 1–2 | L | Properties pass; every CLI unit form parses |
| **SKL** (→F2l) | `enth-plugin/src/learn.rs`, a minimal panel, `enth lesson show\|check`, REAPER `lesson` | F1l, PLY's predicates | 2–3.5 | **H** | The F2l row |
| **LCK** | `xtask/src/lesson.rs` (`--cross`, `--solve`, coverage), `lessons.yml` | F1l, PLY, CLS | 1–2 | L | Catches a seeded bad lesson of each kind; under 10 min in CI |
| **C1** | `content/lessons/basics/` (about 10 lessons) | F1l; LCK to verify | 2–3 | L, opus | `--solve` green; the owner's taste check |
| **LUI** | `enth-ui/src/editor/learn/*`, `learn.json`, overlay highlight and lock, the browser | F2l | 2–3.5 | **H** | Two lessons by label; snapshots; overflow report; 0 idle frames; leak-test |
| **DEC** | `deconstruct.rs`, `lesson-gen`, the runtime job | F2l, CLS | 1–2 | L | Lessons from 5 factory presets pass `lesson-check` |
| **C2, C3** | `content/lessons/{shaping,space-and-dynamics}/` | LCK; Drive after fu1-drv | 2–3 each | L, opus | Coverage complete across C1–C3 |
| **EV3** | The lesson track, the follower floor | SKL's CLI | 0.75–1.5 | L | Floor recorded |
| **INT2** | `docs/r2-gates.md`, owner sheet 4, `gr2-report`, a friend build with packs | Everything | 1.5–2.5 | **H** | Full gate, REAPER, leak and soak green |
| **DL** (cut list) | `learn-download`, or import only | The owner's networking decision | 1–2 | H | `cargo deny`; three OS legs; no request without a click |

**Waves** (two heavy slots, shared with R3 and the fu1 lanes):

| Wave | Heavy | Light | Starts when |
| --- | --- | --- | --- |
| 0 | — (fu1 lanes finish) | DOCS3, CUR | The owner has the summary |
| 1 | S0l (→F0l) | CLS | fu1-hint and fu1-fk merged |
| 2 | *free for R3 or S2* | LM, CV2, SL (→F1l, judge); C1 drafting | F0l |
| 3 | SKL (→F2l) | F1L, PLY, LCK, C1 verified | F1l |
| 4 | LUI | DEC, C2, C3, EV3 | F2l |
| 5 | INT2 | — | All merged; then owner build 4, a friend round, GR2 |
| Later | DL | — | The owner's OK |

**Size and calibration:**
- About 22–40 agent-hours for code, plus 6–9 for content.
- **Critical path:** S0l → LM + CV2 + SL → judge → SKL → LUI → INT2, about 7.5–13.5 agent-hours.
- R1/S1 ran from DOCS2's merge (06:50) to INT's (18:49) on 2026-10-20: **about 12 h of wall time** against a planned 8–11 (*measured*, git log). P1a ran at about a quarter of its estimate, and P1b at about 20 h against 27–51 (*documented*).
- So expect about 9–13 h of wall time. **Content and the owner's sittings set the end date.**

**The owner's time:**

| Gate | Time | Default if no answer |
| --- | --- | --- |
| Read the summary | 20–30 min | — |
| Decide: the content licence (report 02), networking and TLS, new phrase ids | 5 min | No network code; phrases added |
| Taste check: two C1 lessons, as markdown and through the CLI | 20–30 min | Proceed on report 01's rubric |
| Owner build 4 on the real display: the Learn view, highlight and lock, two lessons | 45–60 min | GR2 waits |
| GR2: the basics pack, with per-lesson votes logged automatically | 60–120 min, 2–3 sittings | GR2 waits |

Each friend's time is about 60–90 min.

## 12. Gaps

- No lesson has been timed, and no Learn layout has been measured with lesson patches.
- `closest`'s cost on real lesson starts is unmeasured.
- R1/S1 recorded no agent-hours per lane, so the calibration uses wall time.
- Nothing was built (per the brief), so `wasm-check` for `toml` and a markdown crate inside `enth-learn` is unproven.
- Reports 01 and 02 were not available while I wrote, so the curriculum size and the pack format here are placeholders.

## 13. Recommendations

1. Create **`enth-learn` at L5**, with `session::learn` at L6 and `editor::learn` at L7. Correct architecture.md's L4 row, which still lists "lessons".
2. Make **Challenge v2** (patch target, patch start, editable and locked sets, choice answers) and the session's Learn API **one judge-reviewed freeze (F1l)**, with the `enth`-field budget as a named check.
3. Run lesson **Recreate steps as real rounds** (`RoundSource::Challenge`, `origin: lesson`, left out of `gr1-report`).
4. **Enforce locks in the session's edit path**; the overlay only draws them.
5. Build knob feedback from **`hint` (expectation steps) plus a new `closest.rs` (patch targets)**, as a button. Leave `nearest.rs` to R3.
6. Use **four check kinds**, and run **`lesson-coverage`** in CI as the mechanical half of GR2.
7. **Bundle packs; no network code in R2.** Offer "Import pack…" and "Open release page". Downloading waits for the owner's OK (TLS needs C; networking comes after GR1).
8. Define an honest lesson as **`lesson-check --solve`** passing (start fails, witness passes, solver reaches, choices audible, margin ≥ 1e-3). Add a `lessons.yml` path filter, because Markdown runs no CI.
9. Generate **deconstruct lessons at authoring time** with a pure planner fed measured sensitivity, and reuse the planner at run time for user presets.
10. **Schedule around the fu1 lanes and R3:** S0l after fu1-hint and fu1-fk; wave 2's heavy slot to R3; Drive lessons after fu1-drv.
11. **Get the owner's taste check on two lessons before bulk content**: content quality is the top cost and risk.
