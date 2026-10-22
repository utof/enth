# Learn (R2): lessons, checks and packs

R2 teaches sound design on the instrument you keep: lessons of 5–10 minutes, one control or idea each, played inside the plugin, from "what is a saw wave" to sound-design studies. A lesson's "recreate this" step is a real Recreate round, so A/B, the rings, the coach and the results drawer come free. Lessons ship as **content packs** (markdown, patches and challenges, no audio), bundled in the build. This page is the design of record; the lane plan is [phase-3.md](phase-3.md), the curriculum is lane CUR's [lesson plan](../content/lessons/PLAN.md), and the evidence is the [R2 research session](../_local/sessions/2026-10-21-r2-learn/summary.md) (reports 01 pedagogy, 02 packs, 03 architecture; the summary wins where it corrects a report).

**Status (2026-10-22, lane INT2): every R2 lane is built and merged; GR2 waits for you and your friends.** The lesson model and player (LM, PLY), Challenge v2 (CV2) and the session's Learn API (SL) were judged together at F1l (lane F1L closed the findings), and one lesson ran end to end in the plugin at F2l (SKL); the Learn view, the lesson tools, the four packs (42 lessons), the Review queue and the agent lesson track followed. After F1l, learn, challenge and session types change only additively ([phase-3.md](phase-3.md#freezes)). The automated GR2 checks pass ([r2-gates.md](r2-gates.md)); your steps (the taste check, owner build 4, the friend round and GR2's pre- and post-tests) are on [owner sheet 4](r2-owner.md), the friends' on [gr2-friends.md](gr2-friends.md). Every decision here is provisional until you steer. Each lane updated its own section as it landed ([phase-3.md](phase-3.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| Lesson and pack model, player, checks, progress, markdown AST | `enth-learn` (L5) | LM, PLY | Built by LM and judged at F1l ([below](#the-lesson-model-lane-lm)); PLY's predicates, sound and choice evaluation and the markdown AST built ([below](#player-and-checks-lane-ply)) |
| Challenge v2 | `enth-challenge` (L4) | CV2 | Built and judged at F1l ([challenge.md](challenge.md#challenge-v2-lane-cv2-r2)) |
| Live binding, locks, the `learn` field | `enth-session::learn` (L6) | SL | Built and judged at F1l, on LM's player and CV2's selectors since lane F1L's swap ([session.md](session.md#learn-lane-sl-r2)) |
| `closest` (the nearest passing patch) | `enth-solver::closest` (L5) | CLS | Built ([match.md](dsp/match.md#closest-closestrs-r2-lane-cls)); the "How close am I?" button on it built by lane LFU ([below](#learn-follow-ups-lane-lfu)) |
| One lesson end to end in the plugin; `enth lesson` | `enth-plugin`, `enth-cli` (L8) | SKL, EV3 | SKL's part built (F2l): [plugin.md](plugin.md#learn-lane-skl-r2), [agent-cli.md](agent-cli.md#lessons-on-the-command-line-enth-lesson); EV3's `list`, `start`, `hint`, `closest` and the agent lesson track built ([agent-evals.md](agent-evals.md#the-lesson-track-r2-lane-ev3)) |
| `lesson-check`, coverage, `pack`, `lesson new`, `lessons.yml` | `tools/lesson` (`enth-lesson-tool`), `enth-learn::coverage`, xtask drivers | LCK | Built ([below](#lesson-check-lane-lck)) |
| The Learn view | `enth-ui::editor::learn` (L7) | LUI, LFU | Built ([editor.md](editor.md#the-learn-view-lane-lui)); LFU's follow-ups [below](#learn-follow-ups-lane-lfu) |
| Deconstruct lessons, the studies | `enth-learn::deconstruct`, `tools/lesson` (`gen`), `content/lessons/studies/` | DEC | Built ([below](#deconstruct-lessons-lane-dec)) |
| Review (Leitner boxes) | `enth-learn::review`, `enth-session::review` | REV | Built ([below](#review-lane-rev)) |
| Ear skills, bursts and drills (R4) | `enth-learn::drill`, additive in `review`, `lesson` and `progress` | DRL | Built ([below](#ear-skills-bursts-and-drills-r4-lane-drl), [ear.md](ear.md)) |
| The packs | `content/lessons/<pack>/` | C1, C2, C3, DEC | All four built: `basics` (C1, 16 lessons, bundled in the plugin by LFU), `shaping` (C2, 13), `space-and-dynamics` (C3, 7), `studies` (DEC, 6); the last three install from the Packs folder (the friend zip carries them) |
| GR2 | [r2-gates.md](r2-gates.md), `cargo xtask gr2-report` (`xtask/src/gr2.rs`), `content/gr2/` | INT2 | Built: the automated rows pass; the human half waits for you and your friends ([owner sheet 4](r2-owner.md), [below](#gr2)) |

## The lesson shape

A lesson takes 5–10 minutes and teaches **one new control or idea**: Hear → Name → Try → Do → Check, then Review in later sessions (report 01 §3). Hearing a contrast before the explanation has better support than explaining first (Schwartz & Bransford 1998; variation theory), so Hear comes first.

| Step | What happens | Built from | Time |
| --- | --- | --- | --- |
| **Hear** | 2–4 A/B pairs that change only the new control, on at least two base sounds or notes; one "which is brighter?" item; a written listening cue | Live deterministic renders of the pack's patches, level-matched | 30–60 s |
| **Name** | At most about 80 words in perceptual terms, with `ParamSpec.learn.desc` as the canonical sentence and an optional link for depth; visuals only for modulator shapes | `LearnMeta`, the scope | 30 s |
| **Try** | Only the new control shows; a goal such as "make it darker than this", checked live | A patch or sound check; `enth-solver::hint` | 1 min |
| **Do** | 2–3 hidden-target challenges; the new and earlier controls editable, earlier ones dimmed. Early lessons draw 3–5 values at least 2 just-noticeable steps apart; later ones continuous values | A constrained Recreate round: the scorers, the rings, the coach | 2–5 min |
| **Check** | Mastery: 2 of 3 Do challenges passed with no level-4 hint or Reveal, plus 1–2 mixed "what changed?" items; a "new to me / knew it" tap; an optional "what did you learn?" | The round log, choice items | 30 s |
| **Review** (later) | 3–5 items from passed lessons, by Leitner box and weakest skill; no hints on the first try | Seeded rounds, the ring history | 3–5 min |

Rules:

- **Scaffolds fade within a unit:** controls go hidden, then dimmed, then normal, and each unit's last lesson uses the full editor.
- **Hints by tier** (through `hint_budget`): Basic lessons open all four coach levels; Intermediate open levels 3–4 after a first Submit; Advanced lessons, Review and unit tests give no hint on the first attempt. Any hint marks the attempt assisted.
- **A redo after a Reveal, a level-4 hint or "How close am I?":** a sibling challenge with the same parameters and a new seed, so a worked example is always followed by a problem.
- **No spectrum in Do or Review** (it would mark the round assisted); it stays available in Hear and Name.
- **No streaks**, which show engagement, not learning. Feedback talks about the sound and the knob ("darker than the target"), never the person. Text uses the timbre words the coach already uses (brighter, plucky, thicker).

**The curriculum** follows the Recreate generator's order (sources, filter, amp envelope, envelopes as modulators, LFOs, voices, effects, macros), so a unit test is three Recreate rounds at the unit's generator level, and a placement test of two per unit lets an experienced player skip. [PLAN.md](../content/lessons/PLAN.md) has 42 lessons: 36 core lessons in units U0–U8 and 6 studies (U9), in four packs (`basics` 16, `shaping` 13, `space-and-dynamics` 7, `studies` 6), covering all 54 parameters of the P1 modules. **The 12-lesson tour** (one lesson per module, about 1.5 h; 1.1, 1.4, 1.5, 2.1, 2.5, 3.2, 4.1, 5.1, 6.2, 7.1, 7.3, 8.1) is GR2's friend track and is written first in each pack. Provisional (CUR's open questions, [phase-3.md](phase-3.md#how-the-conflicts-were-resolved)): the tour is played as a track across installed packs; 0.1 opens it as lesson 0, outside the count; the studies are a fourth pack.

## Where Learn lives

| Piece | Home | Why |
| --- | --- | --- |
| Pack and lesson model, `LessonRef`, compatibility, progress and its merge, the **player**, check specs and their evaluation (patch predicates; expectations through `enth-render` and `enth-analysis`), the markdown AST, the deconstruct planner, coverage, review | **`enth-learn`, L5, core** | Shared by the session, the CLI and xtask (and later wasm). It embeds `Challenge` (L4), so it sits above L4; `wasm-check` covers every layer up to 5, so the I/O bans and the wasm build come free |
| Challenge v2, `prepare_patch_round` (moved down from `enth-cli`'s `compare.rs`), new phrase ids | `enth-challenge` (L4) | Persisted challenge types; phrases are append-only |
| `closest` ([design.md §7](design.md#7-how-far-were-your-knobs-when-many-answers-are-right), layer 3) | `enth-solver/src/closest.rs` | Leaves `nearest.rs` to R3, whose k-NN retrieval fills it |
| Live binding: stash, step loads, **locks in the edit path**, jobs, the read model, the `learn` object in the `enth` field | `enth-session/src/learn.rs` | Like `game.rs`: no clocks, threads or I/O |
| The Learn view | `enth-ui/src/editor/learn/` | The `editor/recreate/` seam |
| Pack I/O, the progress file, Learn jobs on the worker | `enth-plugin/src/learn.rs` | Shells translate; the rules come from `enth-learn` |
| `enth lesson list\|show\|check\|hint\|closest` | `enth-cli/src/cmd/lesson.rs` | Agent evals; stateless like `enth check` |
| `lesson-check`, `lesson-coverage`, `lesson new`, `lesson-gen`, `pack`, `gr2-report` | `tools/lesson/src/` (the package `enth-lesson-tool`, exempt tooling); `xtask/src/lesson/` are thin drivers that run it, so xtask carries no engine crate; `gr2-report` reads plain files and lives in xtask itself (`xtask/src/gr2.rs`, lane INT2) | Tooling |

Rejected: lessons inside `enth-challenge` (11.5k lines with five owners, and packs have a lifecycle challenges don't); session-only lessons (the CLI and the lesson tool could not share the player); an L7 I/O crate (only the plugin needs one until a standalone ships).

**Job payloads are solver-native**, so `enth-learn` never calls its same-layer sibling `enth-solver`, and the worker composes nothing: an expectation hint is `{patch, perf, items, free}` → `enth_solver::hint`; a Recreate step's hint is the game's coach job; closest is `{round, patch, free, budget}` → `enth_solver::closest`.

## The lesson model (sketch)

Report 03 §3's sketch; lane LM wrote the real types ([below](#the-lesson-model-lane-lm)) and the F1l judge reviews them. The file format is report 02's (below).

```rust
// enth-learn (sketch)
pub struct LessonRef { pack: PackId, pack_version: u32, lesson: LessonId, step: u16, content: [u8; 8] } // sha-256 prefix
pub enum StepKind { Read, Listen { examples: Vec<Example> }, Do { check: Check },
                    Recreate { challenge: Challenge }, Choose { options: Vec<Example>, answer: u8 } } // Choose: an ear question
pub enum Check { Patch(Vec<Predicate>), Sound { phrase: PhraseId, expect: Vec<Expect> }, All(Vec<Check>) }
pub enum Predicate { Value(Addr, Cmp, Plain), Route(Src, Dst, Cmp, Pct), Has(ModuleKind), Enabled(Id, bool),
                     Touched { addr: Addr, min_span_pct: u8 } } // "turn it and listen", from the edit log
pub struct StepView { layout: Option<String>, start: Option<PatchRef>, highlight: Vec<AddrPat>, hide: Vec<ModuleSel>, lock: Vec<AddrPat> }
pub enum StepPhase { Intro, Working, Checking { job: JobId }, Passed, Shown, Skipped }
pub struct Progress { lessons: BTreeMap<LessonKey, LessonProgress> } // per user; merge = union of steps, max of bests
```

Provisional additions from [PLAN.md](../content/lessons/PLAN.md#9-open-questions): a Listen example carries optional fields for turning the level match off, a render seed and a control curve (Q7); a sound check may pin macro positions, and `macro1` is the predicate form for the knob (Q5).

**The lifecycle** (pure, `&mut self`, times from the shell):

1. `start` stashes your patch and loads the step's start.
2. While `Working`, patch predicates re-run after each delivered edit (microseconds). Sound checks run as a job once the patch settles (300 ms, as the game's loudness job does). A result for an older revision is refused.
3. `hint` gives the authored hints first, then measured ones. `show` applies the witness's next op as one undo step. Both mark the step assisted.
4. `next` and `back` move between steps; `end(Restore | Keep)` works as in the game.

**Recreate steps are real rounds.** A `Recreate` step starts a game round through an additive `RoundSource::Challenge(Box<Challenge>)` and a `JobKind::PrepareChallenge` the worker answers with `prepare_challenge`. The step passes when the round passes. Its round-log line carries `origin: lesson`, so `gr1-report` leaves it out. Lesson rounds never stash: the `learn` object holds the only stash.

**Locks are the session's rule, not the overlay's.** `Session::edit` refuses, before anything changes, a transaction with any op that touches a locked control (`SessionError::Locked`, with a hint), so every shell and test gets the rule; the overlay only draws the lock. What an op touches is one table, `enth_challenge::v2::touches`, which the challenge's own lock check and the player's Show use too. The policy, settled at F1l ([session.md](session.md#locks)): host automation of a locked control folds into the patch but never counts toward a check, nor keeps one from settling; undo never leaves the step; a preset load asks Restore or Keep.

## The lesson model (lane LM)

**Built (2026-10-21, lane LM, to freeze F1l; judged with Challenge v2 and the session's Learn API, and the judge's findings closed by lane F1L).** `enth-learn` holds the frozen formats, the `.enthpack` reader, the player's API and a basic body, and progress v1. The code is the reference (`crates/enth-learn/src/`, one file per piece); the fixtures under `crates/enth-learn/tests/fixtures/<kind>/v1/` pin the bytes. The sketch above is superseded where they differ.

| Format | Schema | Reading | Where | Fixtures |
| --- | --- | --- | --- | --- |
| Pack manifest | `enth.pack/1` (`pack.json`; `pack.toml` when authoring) | **Strict**: unknown fields refused, a newer schema refused and named (`pack_too_new`) | `pack.rs` | `pack/v1/pack.json`, `example.enthpack.sha256` |
| Lesson | `enth.lesson/1`; `enth.lesson/2` when a lesson has a drill step (R4), written at the lowest version that holds it (`lessons/<id>/lesson.json`; `lesson.toml`) | **Strict**: `lesson_too_new`, unknown fields, and a field the step's kind does not have, refused; a v1 file holding a drill step refused | `lesson.rs`, `check.rs` | `lesson/v1/cutoff.json`, `every-field.json`, `recreate-v2.json` (an embedded `enth.challenge/2` draw with `editable` and `locked`); `lesson/v2/drill.json` |
| Progress | `enth.progress/1` (`<Documents>/enth/Learn/progress.json`) | **Lenient**: a newer version reads; unknown fields and undecodable entries are kept verbatim and written back, with the highest version read | `progress.rs` | `progress/v1/progress.json`, `newer.json` (input), `newer.rewritten.json` |
| `LessonRef` | inside the session's `learn` object | Tolerant (saved state): an unknown field is ignored | `pack.rs` | `lesson-ref/v1/ref.json` |

TOML and JSON are one set of serde types (`Manifest::from_toml`, `Lesson::from_toml` behind the `toml` feature), so the two forms cannot drift; `pack.toml` holds every field but `files`.

### Packs

```json
{"schema":"enth.pack/1","id":"basics","version":"1.2.0","title":"Synth basics","authors":["enth contributors"],
 "license":{"text":"CC-BY-4.0","patches":"CC0-1.0"},
 "requires":{"lesson":1,"challenge":2,"patch":1,"generator":1,"scorers":["combined/1","sound/1","structure/1","time/1"]},
 "engine":"0.3.0","covers":["osc","svf"],"lessons":["waveforms","cutoff"],
 "tracks":[{"id":"tour","title":"The tour","lessons":["basics/how-a-lesson-works","basics/waveforms","shaping/vibrato"]}],
 "files":[{"path":"lessons/cutoff/lesson.json","size":871,"sha256":"…"}]}
```

| Field | Rule |
| --- | --- |
| `id` | `[a-z][a-z0-9-]*`, at most 32 bytes; progress is keyed by it |
| `version` | Semver core `MAJOR.MINOR.PATCH`; the newer of two packs with one id is kept (a tie: the bundled one) |
| `requires` | `lesson` and `patch` (the patch **file schema**, 1 today, not the engine), and `challenge`, `generator`, `scorers` when a lesson has a Recreate step. The pack loads only if this build has them all (`requires_unmet`), and **a pack whose lessons use more than it declares is refused** (`bad_field`, naming `requires`), so `requires` never lies. `challenge` must be at least the highest `Challenge::version` its Recreate steps use: 1 for generated rounds, 2 for any v2 piece (F1l review M3). An under-declared pack would fail inside a lesson on an older build, whose reader refuses the newer embedded challenge; refusing it here, with the authoring error named, is why the code is `bad_field` and not `requires_unmet` (whose hint, "update enth", would be wrong) |
| `engine` | The engine it was checked on, recorded, never a gate; `Manifest::seal` stamps it |
| `covers` | Module-kind keys (`osc`, `svf`, …), for the browser |
| `lessons` | The order the app offers them; each lives in `lessons/<id>/` (the directory is the id) |
| `tracks` (Q1) | Ordered lesson lists across packs (`pack/lesson`): the tour is a track the pack that starts it carries; a missing pack's lesson shows as missing. 0.1 opening the tour (Q3) is the track's first entry |
| `files` | Every file but `pack.json`, with size and SHA-256; `Manifest::seal` computes it |

**Paths.** Relative, `/`-separated ASCII (`[A-Za-z0-9._-]`), no `.`, `..` or hidden segments, at most 160 bytes, and only `CREDITS.md` (required), `lessons/<id>/lesson.json`, `lessons/<id>/<text>.md` and `patches/<name>.patch.json`. `media/` is refused in v1. Limits: 1 MiB a file, 16 MiB a pack, 1,024 entries, 256 lessons, 32 MiB an archive.

**The reader** (`reader.rs`) reads `pack.json` first, then inflates each listed file **capped at its listed size**, and refuses, each with a code and a hint: `renamed_entry` (a normalised name differs from the raw one, or the local header names another file), `duplicate_entry`, `too_many_entries`, `too_large`, `unsupported_compression` (only stored and deflate), `encrypted_entry`, `unlisted_file`, `missing_file`, `size_mismatch`, `inflate_overflow`, `corrupt_entry` (CRC), `hash_mismatch`, `bad_path`, `bad_content` (a patch that does not load, a text not UTF-8). Directory entries are skipped. A folder pack goes through the same checks (`Pack::from_files`). **The canonical writer** (`write_archive`): `pack.json` first, then path order, deflate level 9 (stored when that does not shrink), no timestamps, permissions or comments, so a pack builds to the same bytes on every platform (the fixture pins its SHA-256). `cargo xtask pack` (LCK) is TOML → JSON, `Manifest::seal`, `write_archive`.

**The store** (`store.rs`): `PackStore { scan, read, write_user }` with `MemStore`; `install` verifies before it writes `<id>.enthpack`; `load_library` reads every pack through the reader, keeps one per id and lists every refusal with its reason (a newer pack is named, never hidden).

### Lessons and steps

A lesson: `id`, `title`, `tier` (`basic`, `intermediate`, `advanced`), `objective`, `teaches` (catalogue keys: `svf.cutoff`, `voice.glide`, `macro.value`), and 1–32 `steps`. A step: `id` (append-only; progress is keyed by it), `kind`, `text` (a `.md` beside `lesson.json`), `view`, `hints` (at most 8, shown before any measured one), and its kind's fields, flat:

| `kind` | Fields | Passes when |
| --- | --- | --- |
| `read` | (`text` required) | Next |
| `listen` | `examples`: 1–8 | Next |
| `do` | `check`, `witness` (a pack patch that passes it; Show's source) | the check passes for the current revision |
| `recreate` | `challenge` (a whole `enth_challenge::Challenge`, read by its own version-tolerant decoder), optional `witness` | its round passes |
| `choose` | `options`: 2–6 examples, `answer` (0-based) | the right pick (clean only with no wrong pick and no help) |

**`view`** (`StepView`): `layout` (`full`, `focus`), `start` (a pack patch; none: the patch carries over), `highlight`, `hide`, `editable` (**advisory**: the UI, the measured hint's free set, `closest`; empty means no restriction) and `locked` (**enforced** by the session). **The F1l naming decision:** the lesson's `lock` became `locked`, so `editable` and `locked` are named as the challenge's `Constraints` names them (the fixtures and the example pack follow; nothing had shipped). Selectors are `enth_challenge::v2::Selector`'s text, strict and canonical: `filter1.cutoff`, `voice.glide`, `filter1.*` (the module, its parameters and its routes), `macro3`, `lfo1 -> filter1.cutoff`. **Each is checked against the step's start when the pack loads** (`bad_field`: it must name something the start has, and no selector may appear twice in a list; F1l review should-fix 3); a step with no start of its own carries the patch over and has nothing to check against, and a Recreate step's view is checked against its challenge's start. **A Recreate step's start, `editable` and `locked` are its challenge's**, so its view may not set them, and the player copies the challenge's constraints into the step's view (M4).

**`Example`** (Q7, decided): `label`, `patch`, and the optional `phrase` (default 1, `HELD`), `transpose` (−36 to 36 st, for "at C2 and C4"), `level_match` (default on; off in 0.1 and 7.3), `seed` (a render seed; 5.6, 8.3) and `curve` (`{"source":"modwheel","points":[{"ms":0,"pct":0},…]}`, the mod wheel or a macro, linear between points; 6.3). Examples share the Target ▶ slot by time (SL).

**`Check`** (structured, `kind` first; every atom is text the project already reads, so the shape freezes without PLY's one-line grammar, which may later read `filter1.cutoff < 1kHz` into the same value):

```json
{"kind":"all","checks":[
  {"kind":"patch","predicates":[
    {"kind":"value","knob":"filter1.cutoff","cmp":"<","value":"1kHz"},
    {"kind":"within","knob":"master.level","lo":"-24dB","hi":"0"},
    {"kind":"route","route":"macro1 -> filter1.cutoff","cmp":">","depth":"20%"},
    {"kind":"has","module_kind":"svf"},
    {"kind":"enabled","module":"osc2","enabled":true},
    {"kind":"touched","knob":"filter1.resonance","min_span_pct":30}]},
  {"kind":"sound","phrase":1,"expect":["brightness<500Hz"],"macros":[{"macro":"macro1","value":"1"}]}]}
```

Values are read by the control's `ParamSpec::parse` (every CLI unit, enum keys, `on`/`off`; a TOML number reads as its text); `==` is exact, for enums and toggles. An absent route has depth 0; an absent module fails every comparison and counts as not enabled. `touched` spans the normalised range the player's own edits covered during the step. Expectations must parse at load. **Q5, decided:** a sound check pins macro positions (`macros`), and the knob's predicate form is `macro1` (0–1). The patch evaluation here is LM's basic body; PLY owns its refinement, the sound and choice evaluation and the one-line grammar.

### The player

`Player` (`player.rs`) is pure: `&mut self`, times from the shell. **The session owns** the live patch, the stash, the undo history per step, the job ids and the saved state; every mutating call returns `Vec<Effect>` in order (seam agreed with SL through the coordinator):

| Call | Does |
| --- | --- |
| `new(bundle)`, `start()` | Step 0's load |
| `resume(bundle, step)` | After a reload: `View` and `Examples` only, never `LoadPatch` or `StartRound` |
| `edited(now, revision, &patch, &ops, source)` | `source`: `Player` (editor, undo, redo, Show's op; counts toward `touched`), `Host` (folded, locked parameters pinned back by the session), `Load` (a step's start or a state load). Patch predicates re-run at once |
| `tick(now)` | After `SETTLE_MS` (300 ms) without an edit, a step whose patch part holds asks `Check(CheckRequest{revision, patch, sounds})`, once per revision |
| `check_done(revision, &verdict)` | **Refused (`stale_result`) unless it answers the check out for the current revision**; the session refuses stale job ids first |
| `hint()` / `hint_done(revision, MeasuredHint)` | Authored hints in order, then one `HintJob` per revision (`{patch, phrase, expect, free}`, solver-native). Marks the step assisted. Recreate steps: the round's coach |
| `show(&patch)` | A Do step: the witness's next op (`ops_between`) as `Apply(op)`, one undo step through the session's edit path: the first whose every touch the view allows (by `v2::touches`, the session's table), else the first that touches no lock; never an op the session refuses (a witness only a locked edit reaches is `no_witness_path`, which `lesson-check` catches); the knob's value before the op is noted for `touched` (lane LFU), so one jump counts as a turn. A choose step: `Reveal` and a pass. Marks the step assisted |
| `choose(i)`, `round_ended(RoundEnd{passed, assisted, best_sound_pct})` | Ear questions; a Recreate step's round, at its first passing submit, a reveal, or when the step is left |
| `next()`, `back()`, `end(Restore \| Keep)` | Next passes a Read or Listen step and skips an unpassed one; leaving an unpassed round emits `EndRound`; the last Next emits `Finished` |
| `lesson()`, `step()`, `steps()`, `step_count()`, `phase()`, `lesson_ref()`, `round_challenge()` | Read accessors; the session saves `lesson_ref()` after every step change, and prepares a resumed Recreate step's round again from `round_challenge()` |

Effects: `LoadPatch` (**at most one per step load**), `StartRound(challenge)` (a Recreate step, instead of `LoadPatch`), `EndRound`, `View` (the **effective view** every step: a Recreate step's `locked` and `editable` are its challenge's `Constraints`), `Examples` (resolved patches), `Check`, `HintJob`, `Hint`, `Measured`, `Apply`, `Chose`, `Reveal`, `Passed`, `Record` (merge it into progress: passing is reported at once), `Finished`, `End`. A passed step stays passed. Phases: `Working` (the default), `Checking{revision}`, `Passed`. **The session holds it behind its `LessonPlayer` trait**, which `Player` implements (lane F1L), and `enth-session`'s `tests/learn_player.rs` plays a lesson with it.

**`LessonRef`**: `{"pack":"basics","version":"1.2.0","lesson":"cutoff","step":1,"content":"0123456789abcdef"}`. `content` is the lesson's content id (`ContentId::of_lesson`): the first 8 bytes of a SHA-256 over one `<path> <sha256>` line per file, in path order, for `lesson.json` and every patch and text the lesson names, each with its manifest hash (F1l review should-fix 8), so an update that only edits a step's start patch or its text restarts a saved lesson too. On load the session resumes at `step`, or at 0 when `content` differs; the stash is kept either way. It is under 200 bytes with the longest ids, so it costs the 1 MiB `enth` field nothing next to the stash.

### Progress v1

`{"schema":"enth.progress/1","lessons":{"basics/cutoff":{"steps":{"try":{"done":true,"clean":true,"best":97}}}}}`. A step is `done` (passed or read), `clean` (passed once with no hint, Show, reveal or wrong pick) and has a `best` (a Recreate step's best sound %). **`Progress::merge` is a join**: the union of lessons and steps, `done` and `clean` or-ed, the maximum of bests; commutative, associative, idempotent, never shrinking (proptests, fixed seed). Fields and entries this build cannot read are kept and written back; between two differing kept values the greater canonical JSON text wins (still a join), and a readable entry wins over an unreadable one. `record` is a one-step merge. Only text that is not a progress file is refused, and the shell keeps such a file aside. **The highest version read is written back** (`Progress::version`, joined by the merge), so an older build that rewrites a newer file keeps its number (F1l review should-fix 2). **Progress changes are additive only:** a later version may add fields and entries, never change what an existing one means or how it is written; that is what makes keeping the unknown and writing the newer number back correct. GR2's votes and REV's boxes join additively as new fields.

### Decisions for the F1l judge

| Decision | Why |
| --- | --- |
| Ear questions are `StepKind::Choose`; Challenge v2 has no `AnswerKind::Choice` (CV2 agreed) | One representation; the player judges an index with no render |
| Sound checks are `Check::Sound` in the lesson; Challenge v2 has no `Target::Expectations` (coordinator) | One representation of an expectation list |
| A Recreate step embeds a whole `Challenge`, read by its own decoder (v1 migrates; a repeated key is refused) | A challenge stays self-contained (inline patches, CV2) |
| Lock selectors are CV2's `Selector`; `locked` enforced, `editable` advisory (coordinator); what an op touches is CV2's `touches`, shared with the session (F1l review M1) | One matcher and one table for steps, rounds and Show |
| Predicates are structured objects with text atoms, not one-line strings | Frozen now without PLY's grammar; addresses, units and expectations reuse existing parsers |
| Steps are flat objects with a `kind` tag and kind-specific fields, a foreign field refused | Natural TOML (`[[steps]]`, `[steps.check]`), and nothing silently ignored |
| `requires` must cover what the lessons use | A pack cannot under-declare and then fail inside a lesson |
| Progress keeps what it cannot read, writes back the highest version read, and changes only additively | An older build that merges a newer file loses nothing, its version included |
| `Example.transpose` added beside Q7's three fields | The cards' "at C2 and C4" |

**The F1L swap** (done): `interim.rs` is gone; `Selector`, `Touch` and `touches` are `enth_challenge::v2`'s, the scorer check is `score::is_known` and the phrase lookup `phrases::phrase`, and a Recreate step's view carries its challenge's `locked` and `editable`.

<a id="challenge-v2-planned-judged-at-f1l"></a>

## Challenge v2

**Built by lane CV2 and judged at F1l; the reference is [challenge.md](challenge.md#challenge-v2-lane-cv2-r2).** Challenge v1 had only a seed target, a template start, no constraints and patch answers, so it could not express a lesson. v2 adds, additively and with a v1 migration (v1 reads as v2, and a v1-expressible challenge is still written as v1): `Target::Draw` (a seeded draw over parameters and routes with value sets or ranges; a new seed gives a sibling; route depths and their sign are drawn, Q4), `Target::Patch` and `Start::Patch` (whole patches inline), `Constraints { editable, locked }` over one `Selector` type, and the lesson phrase ids 4–7 in their own table (Q8). `prepare_patch_round` moved down from the CLI, so `enth compare --target` and the golden challenges kept their bytes.

**Planned, then left out** (coordinator ruling at F1l): `Target::Expectations` and `AnswerKind::Choice`. A lesson checks sound with its own `Check::Sound { phrase, expect }` and asks ear questions with `StepKind::Choose`, so a challenge variant would be a second representation of each, with nothing to play it.

**What the F1l review changed** (lane F1L): one `v2::touches` table for every lock check (the challenge, the session and the player's Show); no lock on a seed target (`with_seed` regenerates a sibling, so a lock safe for one seed may not be for another: lock with a draw target); `requires.challenge` covers the highest version a pack's Recreate steps use; repeated keys refused in an embedded challenge; the draws pinned by a 1,000-line golden.

## Checks: four kinds

Sound alone cannot check every P1 module (report 03 §4, from the [agent-evals findings](agent-evals.md#findings-for-the-descriptor-and-cli-owners)), so a step's goal is one of four kinds:

| Kind | Passes when | Works for |
| --- | --- | --- |
| **P** patch state (`Check::Patch`: `Value`, `Route`, `Has`, `Enabled`, `Touched`) | The live patch satisfies every predicate, checked after each delivered edit | Every control; the only kind that does |
| **S** sound expectation (`Check::Sound { phrase, expect }`, the expectation grammar of `enth-analysis`) | The rendered phrase meets every item with a margin | Pitch, filter, noise, envelopes, LFO rate and depth, master level |
| **R** Recreate score (a real round with `Constraints.editable`) | `structure/1` and `sound/1` reach the pass mark (90 %) | Oscillator, filter, noise, voices with the right phrase; weak for short envelopes and effects; never gauges or nuisances |
| **E** ear question (`StepKind::Choose`) | The player picks the right option | Everything; the only reliable check for resonance, reverb, the compressor and macros |

| Module | S | R | P | E |
| --- | --- | --- | --- | --- |
| Oscillator wave, pitch | Yes (pulse and saw read alike) | Yes | Yes | Pulse vs saw |
| Noise, filter cutoff | Yes (noisiness, brightness) | Yes | Yes | — |
| Resonance, drive | No resonance descriptor; Drive loses level until fu1-drv | Partly | Yes | **Needed** |
| ADSR | Yes (beat-aware) | **Weak** for short attacks | Yes | — |
| LFO, routes | Only after two or more cycles | Yes | Yes | — |
| Reverb | Only on decaying sounds | Partly | Yes | **Needed** |
| Compressor | **No descriptor** | Weak | Yes | **Needed** |
| Voices, glide | Needs a chord or legato phrase | With new phrases | Yes | Mono vs poly |
| Macros | No | No | Yes | Yes |

GR2's "every P1 module" needs all four kinds, and an ear question counts as a non-patch check (Q6, decided by the summary). `lesson-coverage` makes it mechanical: every module, its Basic-tier parameters, and at least one non-patch check, with "E only" reported separately. A control no ear question resolves (early reflections, detune, damping, the knee, phase) becomes "taught, not checked" and is listed for you to accept at GR2 (Q9, provisional). Numbers in sound checks keep a margin of at least 1e-3 against the start and the witness, as the golden challenges do.

## Player and checks (lane PLY)

**Built (2026-10-21, lane PLY, after F1l; additive only).** The predicate grammar and its tolerances, sound checks, the ear question's audibility gate, the markdown block AST and its lint, the player's properties, and the "needs a newer enth" refusal (F1l review should-fix 6). The code is the reference: `crates/enth-learn/src/predicate.rs`, `check/sound.rs`, `check/choice.rs`, `markdown.rs`, and the tests `tests/predicate.rs`, `checks.rs`, `player_props.rs` and `newer_pack.rs`. No persisted shape changed: a predicate is still LM's structured object, and the grammar reads into it.

### The predicate grammar (lane PLY)

One line per predicate, read strictly into the same [`Predicate`](#the-lesson-model-lane-lm) value as the object, and printed canonically. It follows `enth check`'s expectation grammar (`feature op value`, `feature in [lo, hi]`):

```text
predicate := value | within | route | has | enabled | touched
value     := knob cmp plain                       filter1.cutoff < 1kHz    osc1.wave == saw    macro1 >= 0.5
within    := knob "in" "[" plain "," plain "]"     master.level in [-24dB, 0dB]
route     := source "->" param cmp depth            macro1 -> filter1.cutoff > 20%    env2 -> filter1.cutoff >= +1oct
has       := "has" kind                             has svf
enabled   := ("enabled" | "disabled") module        enabled osc2    disabled noise1
touched   := "touched" knob ">=" span               touched filter1.resonance >= 30%
knob      := module "." param | "voice." param | "master." param | "macro" 1–8
cmp       := "<" | "<=" | ">" | ">=" | "==" | "!="  ("=" reads as "==")
```

- **Values** are the rest of the line, read by the control's `ParamSpec::parse`: every unit form the CLI accepts (`800Hz`, `1.2kHz`, `A4`, `0.5s`, `-6dB`, `+7st`, `50ct`, `+1oct`, `30%`, `4:1`, `90deg`, `−12 st` with the display's minus), enum keys and `on`/`off`. **Route depths** read as the CLI reads them: percent (`20`, `20%`), or an interval on the destination (`+1oct`, `-12st`, `+200Hz`), converted at its current value (`ParamSpec::depth_from_text`). A **span** is 1–100 % of the control's normalised travel. PLAN.md's "span ≥ 6 dB" is written as a percentage: 6 dB of `master.level`'s 48 dB is `touched master.level >= 13%`.
- **Strict:** every atom must parse (an address, a kind, a value that parses for some parameter the knob can name, a depth within ±100 %, a whole-number span), or the line is refused (`bad_predicate`, with a hint). Spaces around operators are optional (`osc1.pw<30`), and `→` reads as `->`. **Canonical print:** single spaces, `==`, ` -> `, a span with `%`, values as written. `parse(print(p)) == p` for every valid predicate (a proptest), and any text either parses or is refused, never a panic.
- **In a lesson,** a predicate may be written as the line wherever it is written as the object: `predicates = ["filter1.cutoff < 1kHz", "touched filter1.resonance >= 30%"]` in `lesson.toml`. It is always written back as the object, so a built pack's `lesson.json` holds only the frozen shape and an F1l build reads it. **`cargo xtask pack` (LCK) must write `lesson.json` through `Lesson::to_json`**, as the tests' pack builder does, never by copying the authoring text.

**Tolerances.** No exact `==` on a real value. This supersedes the basic evaluation's exact `==` described in [Lessons and steps](#lessons-and-steps):

| Comparison | Continuous control | Enum, integer, toggle |
| --- | --- | --- |
| `<`, `<=`, `>`, `>=` | Exact thresholds on the plain value. `lesson-check` keeps the start and the witness at least a step away | Exact |
| `==` | Within one just-noticeable step of the value: `sound/1`'s calibration step (`enth_challenge::calibrate::step`: 10 % for Hz and ms, 2 dB, 10 ct, 0.1 st, 5 points of a percentage, 2 dB on levels, 10 % for ratios and unitless values, 10°), and never narrower than 1 % of the normalised travel (`EQ_FLOOR_NORM`, because a ratio step has no width at 0, e.g. `macro1 == 0`) | Exact |
| `!=` | Not `==` | Exact |
| `in [lo, hi]` | Both ends included: the explicit tolerance | Exact |
| Route `==`, `!=` | Within 2.5 points of depth (`sound/1`'s route step) | — |

The bands are `predicate::equal_band(spec, value)`, so the editor can draw what `==` accepts. Acceptance: `tests/predicate.rs` parses every unit form of every catalogue parameter (with long suffixes, spaces, scaled forms, note names and the typographic minus) in a predicate, and checks it reads the value `ParamSpec::parse` reads.

### Sound and choice checks (lane PLY)

**Sound checks: the seam agreed with SKL (through the coordinator).** `enth-render` and `enth-analysis` sit at L3, below `enth-learn`, so the render happens here, and the plugin's worker, `enth lesson check` and `lesson-check` all reach the same verdict:

| Item | What it does |
| --- | --- |
| `check::evaluate_sounds(&[SoundCheck], &Patch) -> Result<CheckVerdict, SoundError>` | For each sound check, renders `sound.pinned(patch)` (macro pins applied) playing its phrase at `SCORING_SAMPLE_RATE` and `SCORING_SEED`, describes the mid signal with the first note's timing and only the groups its items need, and judges with `enth_analysis::check`. These are `enth_solver::measure`'s steps, so a check reads what `enth check --phrase N` reads. Passes when every item of every check holds; the items come in order, as written, with their signed margins. One render per check (about 15–30 ms) |
| `check::evaluate_sounds_report(sounds, patch, SoundOptions { nuisance })` | The full `enth_analysis::CheckReport` per check. With `nuisance` it also renders at `enth check`'s two nuisance seeds, so each item carries its noise and `thin` verdict (`lesson-check`'s margin rule); the verdict stays the judged render's |
| `SoundError` | `unknown_phrase`, `bad_expect` (both refused when the pack loads), `render_failed`, `analysis_failed`, `score_failed`. **On an error the worker still answers the player, with a failed verdict** (`CheckVerdict::default()`), so the step does not wait in `Checking` |

The session wraps `Effect::Check(CheckRequest)` in a job; the worker calls `evaluate_sounds(&request.sounds, &request.patch)` and hands the verdict back with the request's revision.

**Choice checks: the audibility gate** (for LCK's `--solve`). `check::choice_audibility(&[Audition]) -> Audibility` measures every pair of an ear question's options with `sound/1`, as `enth compare --target a --phrase N b` scores it, in both directions, and keeps the smaller distance in JND units. When both options play without the level match (0.1, 7.3), their loudness difference is added in 2 dB steps (`LEVEL_STEP_DB`). `Audibility::passes(MIN_JND)` is the gate: every pair measured and at least 2 JND apart (PLAN.md §1.2). `failing(k)` and `closest()` name the problem pairs. `check::auditions(bundle, examples)` resolves a step's options. `sound/1` renders a table phrase at the scoring seed, untransposed and with no curve, so **a pair whose options differ in phrase, or where either has a transpose, a render seed or a control curve, is `NotMeasured` with the reason**, and such a step fails the gate on its own (a listening review, or "taught, not checked", Q9). Measured on Init: saw against sine 73 JND, two identical options 0, a 12 dB level change 0 when level-matched and 6 when not, a 20 ct detune 1.07. The cost is about 19 renders an option and one a pair: an authoring-time check.

### The markdown block AST (lane PLY)

`markdown::parse(text) -> Parsed { doc, issues }`, on `pulldown-cmark` with no extensions. `LessonPane` (LUI) draws the `Doc` with design tokens and never sees markdown:

| Markdown | AST |
| --- | --- |
| `#`, `##`, `###` | `Block::Heading { level: 1–3, inlines }` |
| Paragraphs; `-` and `1.` lists, nested | `Block::Paragraph(inlines)`; `Block::List { start, items }` (each item a list of blocks) |
| `*em*`, `**strong**`, `` `code` ``, hard breaks | `Inline::Emphasis`, `Strong`, `Code` (a monospaced chip), `LineBreak`; text runs are merged, soft breaks read as a space |
| `[Cutoff](enth:filter1.cutoff)` | `Inline::Link` to `LinkTarget::Control(Selector)`: highlights on hover. Any selector (`filter1.*`, `macro3`); a route may be written compactly, `enth:lfo1->filter1.cutoff` |
| `[hear it](enth:play/2)` | `LinkTarget::Play(2)`: plays the step's example or option 2 (1-based) |
| `[more](https://…)` | `LinkTarget::Web` |
| A fenced block with the info string `enth` | `Block::Widgets`, one per line: `play 2`, `control filter1.cutoff` (the live control in the text), `scope`, `shape env2` (a modulator's shape) |

**The lint** (`markdown::lint(text, budget)`): `html`, `image`, `code_block` (any fenced or indented block but `enth`), `quote`, `rule`, `link_scheme` (only `https:` and `enth:`; no e-mail), `bad_enth_link`, `bad_widget`, `heading_level` (deeper than 3), `too_long` (more than `STEP_WORD_BUDGET`, 120 words, a Name step's 80 plus a cue and a link; `lesson-check` may pass a tighter budget). `Parsed::resolve(start, examples)` adds `unresolved` for a link or widget that names a control, route, macro or module the step's start lacks, or an example the step does not have. Each `Issue` has its byte range and `line()`. The parse is lenient, so the pane always draws: refused content is dropped (HTML, code, images, rules) or kept as plain content (a link to another scheme keeps its text, a quote its blocks). `Doc::plain()` gives the visible text (for word counts and for agents reading a lesson through the CLI). The example pack's three texts lint clean and resolve.

### The player's properties (lane PLY)

`tests/player_props.rs` drives a lesson with every step and check kind (patch, touched, sound, all, choose, a v2 draw round with locks) through a harness that plays the session's part: it applies loads and Show's op, refuses a player's edit that touches a lock, and counts revisions. The intents are random: edits from every source, ticks, right, stale and failed results, hints, Show, picks, round ends, Next, Back, End and resumes. After every call it checks four properties:

1. **No panic**; a refused call names its code and hint, and after End every call is `lesson_ended`.
2. **No pass without a passing result for the current revision.** A Do step with a sound check passes exactly on `check_done` with a passing verdict for the check out for the latest revision; every other result is `stale_result`. A check is asked only while the step's patch part holds (so an `all` cannot pass on sound alone). A patch-only Do step passes exactly on an edit whose patch satisfies its predicates. Choose passes on the right pick or Show, Recreate on a passing round, Read and Listen on Next, and nothing else passes.
3. **Progress never shrinks:** each `Record` comes with its `Passed`, merging records never lowers `done`, `clean` or `best`, and a passed step stays passed.
4. **Locks hold:** Show never asks for an op that touches a lock of the step's effective view, a step load sends at most one plan, and a Recreate step's view carries its challenge's locks.

It is green at the gate's fixed seed, at 4,000 cases and at another seed. Five deliberate mutations of the player each fail it: accepting a stale result, Show ignoring locks, forgetting a pass on re-entry, a patch pass without its predicates, and asking for sound while the patch part fails.

### A newer pack (F1l review should-fix 6)

A pack whose recorded `engine` is newer than this build's, and that fails on its content, is refused as `needs_newer_enth` (`LearnError::NeedsNewer { name, engine, cause }`, hint "update enth"), never as `bad_lesson`, `bad_field` or `bad_content`. Content failures include an unknown module kind, parameter key, phrase or expectation feature, a newer patch file or inline patch schema, and a manifest that does not decode. The refusal it met is kept as `cause` for `lesson-check` and logs. Integrity failures (sizes, hashes, the archive), `requires` (already "needs a newer enth") and text that is not UTF-8 keep their own codes. `LearnError::needs_newer_enth()` groups it with `pack_too_new`, `lesson_too_new` and `requires_unmet`, so the browser lists all four the same way. `pack::engine_is_newer` compares the stamps.

## Knob feedback

- **Expectation steps:** `enth-solver::hint` returns a bounded Gauss-Newton move from your patch over every failing item, verified by renders (10–16 a hint), sparsified, and `blocked` when a bound stops it. That *is* nearest-solution feedback for these steps; it is shown graded like the coach: aspect, then module, then parameter, then range.
- **Recreate steps:** structure rings are instant, with no render. Design §7's closest passing patch (CMA-ES from your patch, penalising distance, per-parameter deltas) becomes `enth-solver::closest`, with the free set limited to the step's editable parameters (usually 1–4). A scoring render costs 15–30 ms, and level-2 `match` runs took a median of 423 renders, so it is a **"How close am I?" button** with progress and cancel, never live; lane CLS measured it on lesson-like starts (40–75 renders, about 1 s on eight workers). Using it triggers a sibling redo, like a Reveal. Built by lane LFU ([below](#learn-follow-ups-lane-lfu)).
- **Naming:** R2's feature is `closest`. `nearest` is R3's ("the nearest preset as a start"), filled by R3a's k-NN retrieval ([r3.md](r3.md#r3a-create-from-sound-with-search-alone)).

## The Learn view

- **The seam,** copied from Recreate: `Editor::set_lesson(LearnEdit::lesson())` before the frame, `Intent::Learn(LearnIntent)`, `Editor::apply_learn(&mut dyn LearnEdit)` after. A lesson's round uses the Recreate UI unchanged.
- **Overlay additions:** `Emphasis::Highlighted` (a static accent outline, so no repaint timer) and a `locked` flag (a lock glyph, with the refusal on the status line). Markdown links such as `[Cutoff](enth:filter1.cutoff)` highlight their control on hover, and `lesson-check` verifies they resolve.
- **Layout:** a 320 px pane fits 1200 × 800 only when the step hides unused modules (the G1 patch overflows by 276–732 px), so LUI needs a collapsible pane or a focus-card region, with an overflow report.
- **Markdown:** our own small renderer. `enth-learn` parses each step into a block AST with `pulldown-cmark` (MIT), and `LessonPane` draws it with design tokens only. `egui_commonmark` was rejected for now: +875 KB, hard-coded colours and emoji, one maintainer, and releases tied to egui's. The lint bans HTML, images and code blocks and allows `https:` links only. Revisit if lessons need tables or footnotes.
- **Repaint:** Learn asks for no frames of its own; job results wake the editor, as `wake_at_ms` and the worker's `request_repaint` do for rounds.
- **Built (2026-10-21, lane LUI):** the pane, the browser (the tour first, progress per lesson, a disabled Review entry for REV), the layout `learn` (the pane right of the cards, the overview across the window), hiding the pane as the collapse, focus steps, the overlay's `Emphasis::Highlighted` and lock glyph, links and widgets; two basics lessons pass by label in kittest, and the overflow report says which steps fit beside the pane. The reference is [editor.md](editor.md#the-learn-view-lane-lui). "How close am I?", the browser's two buttons and the bundled basics pack followed (lane LFU, [below](#learn-follow-ups-lane-lfu)).

## Packs

**The format** (report 02 §2): a zip named `.enthpack` (deflate or stored), or a folder with the same layout. Not `.zip`, because Safari unpacks zips and discards the archive.

```
basics.enthpack
├── pack.json              # manifest enth.pack/1 (built from pack.toml by `cargo xtask pack`)
├── lessons/01-saw/lesson.json, step-1.md, …
├── patches/*.patch.json   # exactly Patch::to_json()
├── media/                 # reserved, empty in v1
└── CREDITS.md             # authors and licences, shown in the app
```

```json
{"schema":"enth.pack/1","id":"basics","version":"1.2.0","title":"Synth basics",
 "authors":["enth contributors"],"license":{"text":"CC-BY-4.0","patches":"CC0-1.0"},
 "requires":{"lesson":1,"challenge":2,"patch":1,"generator":1,"scorers":["sound/1","structure/1"]},
 "covers":["osc","filter","adsr"],
 "files":[{"path":"lessons/01-saw/step-1.md","size":2113,"sha256":"…"}]}
```

(A sketch: the frozen fields are in [The lesson model (lane LM)](#the-lesson-model-lane-lm); `patch` is the patch file schema, 1.)

| Field | Rule |
| --- | --- |
| `schema` | A newer one is refused as `pack_too_new` ("needs a newer enth") and listed, not hidden; unknown fields are refused, as for `enth.challenge/1` |
| `requires.*` | The highest lesson, challenge and patch schemas, generator and scorer ids the pack uses. It loads only if this build has them all; older patches migrate |
| Engine version | **Recorded, not a gate.** A pack notes the engine it was checked on and loads on any: checks lean on structure, sound checks keep a margin, and CI re-runs `lesson-check` on every bundled pack after any DSP change. (Report 03 said packs "pin" the engine; report 02's rule wins, [design.md §5](design.md#5-determinism)) |
| `version` (semver) | A newer install replaces the older one with the same `id`. Progress is keyed by pack, lesson and step ids, which are append-only, so updates never reset it |

- **No pre-rendered audio of our own patches** (targets render deterministically, as Target ▶ does) and **no images in v1** (live diagrams drawn by `enth-ui` teach better and follow the theme).
- **Size:** 60 lessons are about 0.5 MB raw, 0.2 MB zipped, against an 11 MB release `.clap`.
- **Authoring vs shipping:** authors write TOML in the repo (comments, multi-line text); `cargo xtask pack` writes canonical JSON into a reproducible zip (sorted entries, 1980 timestamps, a fixed deflate level, `miniz_oxide` pinned with `=`), so the plugin needs only `serde_json`.

**Integrity and the reader** (report 02 §5): SHA-256 and size per file in the manifest; files the manifest does not list are refused; each build pins the hashes of the packs it bundles. The reader is pure and wasm-clean (`rawzip` + `miniz_oxide` + `sha2` + `serde_json`, pinned by the coordinator), reads in memory and never extracts. It refuses an entry whose normalised name differs from its raw name (`rawzip` silently turns `../../evil.txt` into `evil.txt`), duplicate entries, too many entries, oversize entries and inflation past the listed size. I/O goes through a `PackStore` seam shaped like `PresetStore` (`scan`, `read`, `install`), with a memory store for tests.

**Where packs live:**

| What | Where |
| --- | --- |
| Bundled packs | Inside the binary (`include_bytes!` of the built `.enthpack`), read through the same reader as any other |
| Your packs | `<Documents>/enth/Packs/` (`ENTH_PACKS_DIR` overrides), next to Presets and Crashes; the view rescans when it opens |
| Progress | `<Documents>/enth/Learn/progress.json` (`ENTH_LEARN_DIR` overrides; below) |

**Delivery: bundle everything; no network code in R2.** The plugin offers "Open packs folder" and "Get more lessons", which opens the project's GitHub releases page in the browser through egui's `OpenUrl` (the vendored egui-baseview already serves it with the `open` crate, so no new dependency). It is the same stage as "Import pack…" from a file. **Downloading inside the plugin waits** for GR1 and your OK: `ureq` + `rustls` works, but `ring` compiles C (and breaks the per-push cross-target guard) and `graviola` is new; it would sit behind a non-default `learn-download` feature in L8 only, and send one GET on a click with no cookies and no query string (lane DL, cut list). Report 02 also proposes turning on GitHub's immutable releases (yours to decide) and, for a later online catalogue, a minisign signature.

**Content licence** (yours to decide; the default): lesson text CC BY 4.0, patches, factory presets and codes CC0-1.0, code MIT. `deny.toml` covers crates only, so `lesson-check` enforces the pack's `license` ids, and a `content/lessons/LICENSE` file states the terms for contributions.

## Progress and the saved state

- **Per user:** progress, the GR2 votes and the review schedule live in `<Documents>/enth/Learn/progress.json`, a file the shell owns (like `presets_dir` and `round_log_path`). Writes merge: re-read, `Progress::merge` (the union of steps, the max of bests), a temporary file and a rename, so two instances never lose each other's progress (a two-writer test). A newer progress file is read leniently.
- **Per project:** the `enth` field's optional `learn` object (absent when idle, within state version 1 like `game`) holds only the in-flight `LessonRef`, its step and the stash. A reload resumes the lesson and brings your own patch back.
- **The budget:** the field is 1 MiB, and a round already reserves 768 KiB, a 384 KiB stash included ([session.md](session.md#what-a-round-saves)). So lesson rounds never stash, and the F1l judge checks that the largest lesson plus its round fit.

## Review (lane REV)

Every Recreate round's Good, Near and Bad rings give a free signal per skill (a module kind and parameter key, such as `svf.cutoff`). A Leitner box per skill in the progress file, and a Review that serves the weakest due skills as seeded sibling rounds, give weakness-targeted drilling with no model to train. No `fsrs` (it pulls in ndarray, rayon and rand for about 50 skills). REV is on the cut list: it may move after GR2.

**Built (2026-10-21, lane REV; additive only, no schema bump).** The code is the reference: `crates/enth-learn/src/review.rs` (skills, boxes, the field, the queue), `crates/enth-session/src/review.rs` (the review source), the browser's Review in `crates/enth-ui/src/editor/learn/browser.rs`, and `ProgressFile::observe` in `crates/enth-plugin/src/learn.rs`. Tests: `enth-learn/tests/review.rs` (the moves and "a skill's box follows its rings" as proptests, the merge laws, the fixture `tests/fixtures/progress/v1/review.json`, the queue on a built pack and on `basics`), `enth-session/tests/review.rs`, `enth-ui/tests/learn_review.rs` (snapshot `learn_review.png`) and `enth-plugin/src/tests_review.rs`.

- **A skill** is read off the round's **target** patch: its module's kind and the parameter key (`svf.cutoff`, `osc.wave`; `voice.glide` and `master.level` for the built-ins), so the player's module ids never matter. Route and enable-flag rows are not skills (open).
- **The boxes:** 1 to 5, due 1, 2, 4, 8 and 16 days after the round that last rang the skill. **Good moves a box up, Near keeps it, Bad moves it down one** (never below 1). A skill no round has rung counts as box 1, due.
- **Which rings:** the round's **first submit** (its saved `first_report`): after it the rings show live, so later submits are feedback, not evidence. Several rows on one skill (two oscillators' `wave`) move it by the worst. **An assisted round never moves a box up:** the coach's level 4 (it gives the move), a Reveal, or "How close am I?" (LFU's `closest`), the same triggers as the sibling redo, whose fresh round can then raise the box; levels 1–3 and the dim assist name no value and hold nothing back. A row the target did not randomize can hold or lower a box, never raise it.
- **When:** once per played round of any origin (the game, a lesson, a Review item), when it closes, as the session's `LearnRequest::Rings`; a close by a host load is skipped, since the host may close and reopen the same round (REAPER's undo). The shell applies it to the progress file with today's day (UTC days since 1970-01-01), under the same lock, re-read and rename as a step's record.
- **The field:** `"review":{"skills":{"svf.cutoff":{"box":2,"day":20747,"rounds":3}}}`, after `lessons`, still `enth.progress/1`, and omitted while empty, so a file without boxes keeps its bytes. It is read as leniently as the rest (unknown fields at any level kept, an unreadable entry kept verbatim, a `review` value that is not an object kept until a readable one replaces it). **Its merge is a join:** per skill the entry with more `rounds` wins, then the later day, the higher box, the greater kept text. LM's progress tests used `review` as their example of an unknown key; they now use `votes`.
- **The queue** (`review::queue(progress, lessons, day)`, pure): candidates are the Recreate steps of **completed** lessons with a draw or seed target (a patch target has no seed, so no sibling); a draw's skills are its parameter items, a seed target's the generated parameters. Skills are ordered due first, then by box, then by due day, then by a keyed draw of the day and the skill (equal skills take turns from day to day, not by name). Each skill takes one candidate that exercises it, no step twice, up to 5 items; when fewer than 3 skills are due the weakest others top the queue up to 3. Each item is the step's challenge with `with_seed(derive(derive(day, fnv(lesson#step)), fnv(skill)) & MAX_SEED)`, never the step's own seed. **The same progress file on the same day gives the same queue**, seeds included.
- **The session** (`LearnEdit::review_start`, a default body for fakes): the item plays as a round of the new origin `review` (`RoundOrigin::Review`; `gr1-report` leaves it out). It stashes like a game round, and the next item takes over a live Review round and its stash; it is refused while a lesson is live (`round_live` over a game round of your own). **No hint on the first try:** a Review round refuses a hint before its first submit (`first_try`); after it the coach opens as in any round.
- **The view:** the shell gives the day (`Editor::set_review_day`) and hands the progress file back after each write (`Editor::set_progress`, when the process's write count moves); the browser's **Review** shows "Review • N due" (its accessible name stays "Review"), plays the queue's head item, and is disabled while a lesson is live and until a lesson with a Recreate round is passed. Each press plays one item; the queue follows the file, so the next press plays the next weakest skill.

Open: route and enable skills; a Review round does not survive a project reload (a draw target has no code: it restores broken, its stash kept, and the queue offers the item again); the day is UTC, so it turns at local midnight only in UTC. ~~Ear questions (Choose steps) as Review items~~ **closed by R4's design** (below).

### Ear skills, bursts and drills (R4, lane DRL)

**Built (2026-10-22, lane DRL; additive, still `enth.progress/1`; a lesson with a drill step is `enth.lesson/2` (the F1e review's M1, lane F1E); judged at freeze F1e, [phase-4.md](phase-4.md#freezes)).** R4's ear games feed the same boxes, so the skill is the unit of spacing and the game only its vehicle. The design of record is [ear.md](ear.md#the-drill-player); the code is the reference: `crates/enth-learn/src/drill.rs`, `drill/staircase.rs`, `drill/record.rs` and `workout.rs`, additive in `review.rs`, `lesson.rs`, `pack.rs`, `player.rs` and `progress.rs`, and the tests `drill.rs`, `drills.rs`, `workout.rs`, `drill_step.rs` and `ear_skills.rs`. Everything is pure: no clock, no global RNG, no I/O; the day comes from the shell (days since 1970-01-01, UTC, as Review's).

| Piece | Where | What it is |
| --- | --- | --- |
| Ear skills | `review::Skill::ear`, `Skill::parse` | `ear.interval.M3`: the `ear` kind and a dotted path of one to three parts of `[A-Za-z0-9_-]{1,32}`, **case kept** (`M3` and `m3` are two skills). `svf.cutoff` keeps its grammar (`svf.cut.off` is still no skill). `Skill::is_ear`, a label that needs no table (`Interval M3`), serde as a string |
| Bursts | `review::{Burst, burst_verdict, Review::observe_burst, burst_standings}` | 5 items on one skill: 4 or 5 right is Good (up a box), 3 is Near (holds), 2 or fewer is Bad (down). One answer never moves a box. `burst_standings` lists the due skills with a box that the Recreate queue does not hold |
| The staircase | `drill::Staircase` | 3 right in a row make the next item one level harder, 1 wrong one level easier; the run counter is private |
| A drill session | `drill::{DrillSpec, Drill, Item, Answer, Summary}` | a spec and an answer log (`Drill::replay` rebuilds it); bursts finish at 5 answers on a skill, once per skill per session |
| The `drills` field | `drill::{Drills, GameDrill, SkillDrill, Point}` | per game the level the staircase ended at; per skill accuracy and thresholds over time in physical units; the days practised |
| `StepKind::Drill` | `lesson` (an `enth.lesson/2` piece), `pack` (`requires.lesson = 2`), `player` (`Effect::StartDrill`, `drill_ended`) | a lesson step that plays a drill |
| The workout composer | `workout::{Catalog, compose, one_more}` | 3 games chosen by due skills, the same for the same file and day, ending in "done for today" |

- **Skill keys:** `module.param` (`svf.cutoff`) and `ear.*` for what has no parameter (`ear.interval.m3`, `ear.chord.min`, `ear.band.1k`). The progress file keeps unknown keys, so an older build keeps them untouched.
- **Bursts:** a drill counts its answers per skill (the question's own skill, which may differ from the focus the item asked for). The fifth answer on a skill ends its burst: a `Burst { skill, right }` that `Progress::observe_drill` applies with Review's own `moved(level, verdict, true)`, one box at most, the day and one more round. A burst short of five answers is dropped, since fewer answers are too guessable. The first 5 × focus items go round the focus skills in turn so the bursts finish together; the rest are free.
- **The staircase** is 3-down/1-up (a right answer about 79 % of the time: `0.5^(1/3)`), not Recreate's 2-down/1-up. A level runs from 1 (easiest) to the ladder's length; a move the ends of the ladder swallow holds the level and resets the run. A **reversal** is the level where the track turned. The **threshold estimate** is the mean level of the last reversals (an even number, at most 12, at least 4; a 40-trial track turns about ten times). `Ladder` turns the fraction of a level into semitones, cents, ms, dB, octaves, a ratio or a percentage by interpolating the ladder's own numbers.
- **How well it measures** (`tests/drill.rs`, simulated listeners: a soft-sign psychometric curve over a 12-level ladder, chance 1/2, 1/3 or 1/4, lapse 0 or 3 %, a keyed `Rng` per run, 300 runs of 40 trials each):

  | Claim | Result |
  | --- | --- |
  | Trained at its threshold, success rate | 77.3 % to 79.3 % over 90 listeners (inside 75–85 %) |
  | Threshold within one level, steep ladders (a level is a JND or more: 0.5 and 0.8 levels per logistic unit; 60 listeners, each started at, one level too hard and one too easy) | 92.2 % of all runs; the worst single listener-and-start 77.3 % (3-option, 3 % lapse, starting a level too easy) |
  | The same, flat ladders (half a JND a level: 1.6) | 79 % within one level, 97 % within two |
  | Cold start from level 1 (a new player), low thresholds | 94.9 % of runs end within 2 levels of the threshold; 0.5 % overshoot it by more than 3 |

  **The F1e judge's ruling changed the criterion, not the rule:** pooled over the simulated listeners, at least 90 % within one level on ladders of about one JND or more per level; every listener-and-start cell at least 75 %; flat ladders at least 90 % within two levels; success at threshold 75–85 %. The numbers above meet it (92.2 % pooled, worst cell 77.3 %, flat 97 % within two, success 77.3–79.3 %). The original sentence "within one step in 90 % of 40-trial runs" held only in the pooled sense for steep ladders, not in every cell and not for flat ladders. The limit is information, not the rule: at half a JND per level, 40 answers carry too little to place a threshold within a level in nine runs in ten (a maximum-likelihood fit of the true curve would reach about 89 % there, before chance guessing costs more). A bias correction and dropping the first reversal were tried and gained under two points; an estimate from a PAV fit and the mean of the track were worse. Ladders should therefore be built with steps of about a JND (ear.md places items by JND distance for the same reason); the Ear view shows a threshold as a trend over sessions, never as one exact number.
- **A game's start level** (`drill::start_level`) is the level its staircase ended at last, or one below after more than 7 days without it (a warm-up, silent: nothing counts the days missed), and 1 for a game never played. There is no placement rule: a new player climbs one level per three right answers.
- **The `drills` field:** `"drills":{"games":{"5":{"level":3,"best":4,"day":20747,"sessions":4,"items":57,"right":44}},"skills":{"svf.cutoff":{"game":3,"day":20747,"sessions":2,"items":80,"right":62,"thresholds":[[20740,"st",2400],[20747,"st",2100]]}},"days":[20740,20747],"workout":20747}`, after `review`, omitted while empty, so a file without drills keeps its bytes (`tests/drills.rs` checks every existing fixture). Games are keyed by `EarSpec::game`'s plain number; thresholds are one point a day, `[day, unit, thousandths]`, the better of a day's measures, the latest 64 days kept; `days` is the cumulative calendar; `workout` is the last day a workout was played to its end. It is read as leniently as the rest: unknown fields at any level kept, an entry this build cannot decode kept verbatim, a part of the wrong shape kept until a readable one replaces it. **Its merge is a join** (commutative, associative, idempotent; proved as properties alone and inside `Progress::merge`): per game and per skill the entry with more sessions wins (then the later day, and so on down to the kept text), the threshold series unite (the better measure a day, then the latest 64), the days unite and `workout` takes the later day. No streak is stored or computed anywhere: a calendar of days practised cannot be broken.
- **`StepKind::Drill`** (`"kind":"drill"`, fields `game`, `level` (default 1), `items` (default 10) and an optional `skill`) is a lesson step that plays a drill. Playing it to its end passes the step **whatever the score**: no content is gated on chords, intervals or note naming. **A drill step is an `enth.lesson/2` piece** (the F1e review's M1; `LESSON_VERSION` = 2, `DRILL_LESSON_VERSION` = 2). A lesson is written at the lowest version that holds it, as a challenge is (`Lesson::version`): a lesson with no drill step keeps `enth.lesson/1` and its bytes, and a v1 file holding a drill step is refused (`bad_field`, naming the step). A pack with a drill step therefore says `requires.lesson = 2`, and a pack that says less is refused as one whose `requires` lies. **Every R2 and R3 build already checks `requires.lesson` by name and answers "needs a newer enth" (`requires_unmet`) before it reads a lesson** (`tests/drill_step.rs` mirrors R3's `Requires` to prove it). DRL first declared drills in a new `requires.drill` key, but R2 and R3 decode `Requires` with unknown fields refused, and R4 changes no engine stamp, so those builds would have listed a drill pack as damaged; the key and `Need::Drill` are gone. `tests/reader.rs` uses `enth.lesson/3` as "too new". The player sends `Effect::StartDrill(DrillStep)` on entering the step (a resumed step has `Player::drill_step`), and the shell reports the end with `Player::drill_ended`.
- **The workout composer** (`workout::compose(progress, catalog, today)`, pure): due skills, in Review's order (due first, then the lower box, the earlier due day, a keyed draw of the day and the skill), each get a **vehicle**, one of the games that can drill them, in a fixed rotation by how many sessions the skill has had and never the game that drilled it last, so a due `svf.cutoff` comes back as Spot the change one day, Dial it in another and Frequency finder later. A game takes up to 3 skills (five items each, so a segment is at least 15 items) and a workout up to 3 games. **Each category is met alone before confusable ones mix**: two skills the `Catalog` calls confusable share a segment only after each has been rung or drilled once. **Note naming is opt-in and never in the default workout.** After a break a segment starts one level below where its staircase ended (`Segment::warm_up`, shown only as the start). Every segment's seed is a keyed draw of the day, the game and the skills, so **the same progress file, catalogue and day give the same workout, seeds included**. A workout ends: `Progress::finish_workout(today)` marks the day, and `compose` then gives no segments (`Workout::done_for_today`) until tomorrow, as it does when nothing is due; `workout::one_more` is the optional extra segment from the weakest skills.

**The shell's loop** (SE, EUI and SKE; the session takes no `enth-learn` type, the seam is data):
1. Build a `workout::Catalog` once from the build's tables: MUS's `Game::{id, title, max_level}` and the theory tables' skill slugs for the music games, and the parameter games' observable controls (as `module.param` skills the learner has met, since the composer drills every skill the catalogue lists that is due). Mark confusable pairs with `Catalog::confusing`.
2. `workout::compose(&progress, &catalog, today)`; for each segment `Drill::new(segment.drill_spec(generator))`.
3. For each `drill.next_item()`: ask the session for one question through **the one mapping** (should-fix S3, lane F1E): `let (spec, first) = item.plan()` (a one-question `EarSpec` of the item's game, generator and level, seeded `item.run_seed()`, starting at the first item that exercises the focus skill, found by `enth_challenge::ear::find_item`, or at 0), then `EarPlan::new(spec).starting_at(first)`; give the judged result back as `Answer::new(item.index, Skill::parse(&skill), correct)` (a one-question run's result `item` is always 0). An Ear check on a chosen control is `params::ear_check_on`, not a plan. `drill.answer` returns the move for the replay line, never a count.
4. When the drill is done (or abandoned: `summary()` counts what was answered), optionally `summary().measuring(skill, &ladder)` for an Ear check, then under the progress lock: re-read the file, `observe_drill(&summary, today)`, write it back. After the last segment, `finish_workout(today)`.

Open: placement (a 1-down rule until the first wrong answer would shorten a new player's climb; not built); the catalogue is the shell's to filter, since the composer cannot know which parameter lessons are passed; the ladders' physical numbers are G1–G4's; a Drill step's generator and ladder come from the shell as well; the day is UTC, as Review's. ~~Ear questions (Choose steps) as Review items~~ closed: a due skill with no Recreate step plays as a burst in a game.

## Deconstruct lessons (lane DEC)

**Built (2026-10-21, lane DEC).** "Deconstruct this preset" lessons are generated by ablation (report 01 §6, report 03 §9): take each element out of the preset, measure what the sound loses, leave the inaudible in as fine detail, and rebuild the rest in signal-flow order, each step heard without and with its element and rebuilt by ear. The code is the reference: `crates/enth-learn/src/deconstruct.rs` and `deconstruct/` (one file per piece), `tools/lesson/src/gen.rs`, and the tests `deconstruct.rs` and `studies_pack.rs`.

| Piece | Where | Pure? |
| --- | --- | --- |
| Elements, the preset's base, projection | `deconstruct::element` | parameter space only |
| The planner, `plan(preset, ranks)` | `deconstruct::plan` | yes: the caller measures the ranks |
| The measurements: ablations, steps in context, sound items, "What disappeared?" | `deconstruct::measure` | renders through `enth-render`; no I/O, clock or thread |
| The lesson, its patches and automatic text | `deconstruct::build` (`generate`) | as `measure` |
| The authoring `lesson.toml` (sparse inline patches, measurements as comments) | `deconstruct::toml_out` | yes |
| The runtime job for a user's preset | `deconstruct::auto_lesson` (`generate`, then `pack_of`) | as `measure` |
| `lesson-gen deconstruct <preset>`, `lesson-gen studies` | `tools/lesson/src/gen.rs` | tooling |

**Elements and the base.** The base is the preset's own Init: the same modules and ids, each at its catalogue defaults, with the first generator that sounds switched on (a noise patch starts from noise) and every other generator and effect off, no routes, the voice at its defaults, and the preset's master level and macro positions (a gauge and a performance, never an element). The elements partition the differences: each generator (its switch and values), a filter's tone, key tracking and drive as three groups, the amplitude envelope, each route with its source modulator's values (shared by that modulator's routes), the voice mode with glide, the voice pan, and each effect. Every patch a lesson uses is a projection of the preset (the excluded parts turned back to the base through ops), so step *k*'s target is the start plus elements 1 to *k* and the last target is the preset itself, routes in its own order.

**Measuring and pruning.** One `sound/1` round per phrase on the full preset (as `enth compare --target` prepares it), and each ablation scored against it; plus a description of each ablation against the preset's, where a descriptor counts when it moves at least two tolerances and three times its nuisance noise (`enth check`'s two nuisance seeds). `sound/1` alone would under-rank exactly what C1 found it deaf to: it measured **0.00 JND** for Glass Pluck's octave pulse, **0.79** for Acid Snap's slide and **1.04** for its snap, all audible and all caught by the descriptors. An element is measured through the first phrase RG's mask says reveals it (the held note, else the legato pair for glide and voice mode, the octave pair for key tracking, the long hold first for an LFO slower than 2 Hz). Below one JND with no descriptor move, an element is fine detail and folds into the start; at most 8 are taught (report 01 §6).

**How a step is taught.** Each step is a Listen step (without, with) and a build step. The build step is a real Recreate round on a patch target when `sound/1` scores its start at most 85 %, it changes parameters only (closest moves parameters, not switches or routes) and `closest` reaches the pass within lesson-check's 300 renders (`lesson-gen` checks and regenerates a miss as a Do step). Otherwise it is a Do step: its verified sound items (each failing at the start and passing at the witness by at least 1e-3, never thin), with the parts too when it switches a module on; with no item that verifies, its parts as loose patch predicates; plus context predicates on the locked earlier parts, which tell its goal apart from a neighbouring lesson's (`--cross`). A step inaudible in its context (a detune that only beats against the next oscillator) rides along with the next step. The editable set covers the witness's touches (a route's destination included); earlier parts are locked unless the witness touches them.

**The close.** "What disappeared?" offers up to four ablations pairwise at least 2 JND apart under `sound/1` (the audibility gate), after a Listen step with the whole sound and the missing one. The variation goal starts from the whole preset: its first filter an octave up (down when already open), checked by a brightness item, a ceiling half an octave past the witness, the items that keep its character (plucky, slow to swell, quick to speak) and its amplitude envelope kept loosely. Then a recap names the parts and the fine detail.

**The runtime job.** `auto_lesson(preset, pack_id, opts)` measures, plans and builds a one-lesson pack (default id `my-presets`) in memory, verified by `Pack::from_files`, so the session plays it as it plays a bundled lesson (`pack.bundle(id)`, then the player). The text is the automatic template (each part's sound and values, `learn.desc` and the measured changes, filtered to what the part can do). It costs a few hundred renders, so it is a background job with progress: the plugin's worker calls it (lane LUI's view and SKL's worker add the job; this lane edits no UI). It is on the cut list (phase-3.md).

**Cost and step quality** (release build; the render counts are exact and deterministic, the wall times were taken at a load average near 40 and are high):

| | Acid Snap | Glass Pluck |
| --- | --- | --- |
| Elements; taught; fine detail | 10; 6; 4 (three macro routes, the compressor) | 13; 4; 9 (six macro routes, the filter, its key tracking, the compressor) |
| Renders: ablation, steps, "What disappeared?", variation | 80, 168, 88, 20 = 356 | 58, 108, 88, 26 = 280 |
| Wall time (CPU) at load 40 | 196 s (85 s), with the closest check and one regeneration | 101 s (44 s) |
| Steps (sound/1 of the start against the target) | filter (17 %, Do: closest reached 87.8 %), key tracking (82 %, round), drive (95 %, Do, predicates), amp envelope (89 %, Do, release item), snap (89 %, Do, attack item), slide (92 %, Do, three items) | octave pulse (100 %, Do, predicates), short decay (57 %, round), filter envelope (100 %, Do, brightness item), room (81 %, Do: a switch) |
| What disappeared? | filter, snap (19.3 JND apart) | short decay, room (4.9 JND) |
| lesson-check --cross --solve | passes; 2 notes (the snap and the slide teach structure the hints cannot move) | passes; 1 note (the room) |

An earlier, lighter-loaded run measured 33.7 ms a render (332 renders in 11.2 s for Acid Snap).

**The studies** (`content/lessons/studies/`, PLAN.md U9; four packs, Q2's default): bass (Acid Snap), plucks (Glass Pluck), pads (Velvet Pad), leads (Breath Flute), percussion (Thump Kick), textures (Wind Tunnel), each regenerated by `cargo xtask lesson-gen studies` with its part names and hints from `gen.rs`'s table and its text edited by hand (a regeneration keeps the texts). `cargo xtask lesson-check --cross --solve content/lessons/studies`: 6 lessons, 94 steps, 0 failures, 8 notes; `pack --check` builds it reproducibly. Acid Snap and Breath Flute are GR3 test-half presets: they may appear in lessons, but this pack's patches never feed R3's training, priors or bank (`pack.toml` says so; the full presets are byte copies of the candidates, which `exclude-synth.txt` lists by hash).

**Known limits.** sound/1 cannot hear several elements, and closest cannot switch modules or add routes, so 20 of the 30 build steps are Do steps rather than rounds. A route step's destination is editable (a route edit touches it), so a brightness item for a filter envelope can also be met by raising the cutoff; the step's text and Show me teach the route. "What disappeared?" is often two options, because the gate needs every pair 2 JND apart under `sound/1`; Thump Kick has none.

## `lesson-check` and the authoring loop

An honest lesson is one `cargo xtask lesson-check --solve` passes. It runs over every pack in `content/lessons/`, deterministically and with no spend:

1. **Schemas:** unknown fields refused; every listed file present at its size, nothing unlisted; patches validate and equal `to_json()` byte for byte; codes decode; generator, template and scorer ids exist in this build.
2. **Text:** every `enth:` link resolves; no HTML or images; `https:` only; a length budget per step.
3. **Playthrough:** from each step's start, the reference solution (the witness) is applied through a headless `Session`, with jobs run inline between frames. Every check **fails before** it and **passes after** it, with no `thin` item and a margin of at least 1e-3; the witness touches only editable controls.
4. **`--cross`:** a check does not pass its neighbours' witnesses.
5. **`--solve`, reachability:** the hint follower reaches the pass from the start within the editable set (if it cannot, the step must teach structure, and its witness shows it); `match` passes Recreate steps within a budget; ear-question options are pairwise at least k just-noticeable steps apart under `sound/1`'s calibration (the audibility gate).
6. **Coverage** (`lesson-coverage`): the module kinds and Basic-tier parameters each lesson touches; it fails if any P1 module has no lesson or no non-patch check.
7. **Hash pinning:** `cargo xtask pack` builds reproducibly, and each bundled pack's hash matches the build's pin.

**The pipeline:** `cargo xtask lesson new` scaffolds; an authoring agent (Opus) drafts from the plan and the catalogue (`enth params`, `enth explain`), building start patches and witnesses with `enth new | apply | check | compare`; `lesson-check` proves honesty and reachability; a second agent checks the prose against `enth explain`'s facts (it reports, it does not gate); then you taste two lessons (1.1 and 2.1) before bulk content, and GR2's per-lesson votes judge the rest.

**CI:** `ci.yml` ignores `**/*.md`, so lesson edits run nothing today. `lessons.yml` runs `lesson-check` on `content/lessons/**`, `enth-learn`, xtask and the DSP, engine, patch and challenge crates (a DSP change can break a lesson). `gate.sh lane` runs `lesson-check` when `content/lessons/` changes.

## Lesson check (lane LCK)

**Built (2026-10-21, lane LCK).** The authoring loop's tools: `enth-lesson-tool` (`tools/lesson/src/`, one file per command, `check/` for the check's parts), driven by `cargo xtask lesson-check`, `lesson-coverage`, `lesson new` and `pack` (release builds, as `golden` drives `enth-golden`); the coverage computation is `enth_learn::coverage`, pure. The code is the reference; the tests are `tools/lesson/tests/` (`check.rs` with the seeded-bad fixtures in `tests/fixtures/`, `commands.rs`).

| Command | What it proves | Fails when |
| --- | --- | --- |
| `lesson-check [PACK...]` | **Files:** the authoring layout builds (a pack directory holds only `pack.toml`, `CREDITS.md`, `lessons/<id>/lesson.toml` and `.md` texts, `patches/*.patch.json`, and every text and patch is named by a lesson); every TOML decodes strictly; `requires` is met and covers what the lessons use; the built archive reads back through the real reader at every size and SHA-256; the pins match; every patch file is `Patch::to_json()` byte for byte; licences are `CC-BY-4.0` or `CC0-1.0`; `covers` names exactly the module kinds taught; track entries exist (a pack not written yet is noted); the markdown lint; no `TODO` left. **Playthrough:** every lesson through a headless `Session` and the real `Player`, jobs inline: each Do step's check fails after its start settles and passes after its witness, applied op by op by Show me through the session's edit path, a `touched` control turned across its span and back; sound items printed at the start and the witness with their margins at the scoring seed and `enth check`'s two nuisance seeds; a Recreate step's start does not pass its round and its witness (or its target) does, submitted; an ear question's wrong picks leave it open and its answer passes; every `enth:` link and widget resolves against the step's start; a generated round's code decodes to its spec | a start passes; a witness fails; a witness op touches a lock or a control outside `editable`; a sound item passes the witness by less than 1e-3, or is `thin`, or no item fails the start by 1e-3; a dead link; a stray, unused or non-canonical file; a pin differs |
| `--cross` | No Do step's check passes another Do step's witness (in its lesson and the lessons on either side) unless that step's start already passes it (a step that builds on another inherits its result and is not flagged); a `touched` predicate reads the straight turn from that start | two checks cannot tell their goals apart |
| `--solve` | **Sound steps:** the hint follower, from the step's start: the real player's measured hint requests (after the authored hints), `enth_solver::hint` within the step's free set, the moves applied, up to 6 times. **Rounds:** `closest` from the round's start over the editable (else drawn) unlocked parameters, within `--budget` renders (300). **Ear questions:** the audibility gate (below) | the follower stops short and the witness moves nothing the hints cannot (a waveform, a switch, a route or a transposition is structure, noted); the measured hint stops where the sound passes while a patch predicate fails (the player is told it already sounds right); `closest` does not pass; an inaudible choice |
| `lesson-coverage [PACK...]` | GR2's mechanical half over every pack but `example` (a layout example; `--all` or naming it includes it): the ten tables (seven module kinds, `voice`, `master`, the macro knob) and 54 parameters with their lessons and kinds of check (P S R E, from what each step changes by `v2::touches`), E only, taught-not-checked (Q9), and ear questions `sound/1` cannot compare (counted as no check) | a table with no lesson or no non-patch check; a Basic-tier parameter no lesson teaches |
| `pack [--out DIR] [--check] [PACK...]` | The canonical `.enthpack` (`lesson.json` through `Lesson::to_json`, the sealed manifest, `reader::write_archive`: `pack.json` first, path order, deflate 9, DOS-zero timestamps, the zip epoch's 1980), read back through the reader, then compared with its pins: the plugin's bundled archive `crates/enth-plugin/packs/<id>.enthpack` (same bytes) and, once a release has one, `content/packs/catalogue.sha256` (`sha256sum` lines `<sha256>  <id>-<version>.enthpack`). The example builds to the SHA-256 lane LM's fixture pins. `--out crates/enth-plugin/packs` rebuilds a bundled pin on purpose | a pin differs |
| `lesson new [--in DIR] <pack> <id>` | A scaffold from the starter (Hear, Name, Try, an ear question; a start and a witness patch) appended to `pack.toml`'s `lessons`, a new pack's `pack.toml` and `CREDITS.md` when needed. It plays honestly; only its `TODO`s fail | — |

`--jobs N` (default `ENTH_LESSON_JOBS`, else the cores up to 4) runs lessons in parallel; the report keeps their order and does not depend on it. `--lesson ID` plays one lesson (an author's loop).

**Decisions (provisional):**

- **The audibility gate gates the answer:** a question is answerable by ear when the answer stands at least `MIN_JND` (2) from each distractor under `sound/1`; two distractors may sound alike (an odd one out repeats a sound), which is noted. When `sound/1` puts the answer closer (it is blind to a beat rate, a beat depth, an asymmetric decay), the **descriptors** decide: both options rendered at the scoring and nuisance seeds, and the pair passes, noted, when pitch, brightness, noisiness, the envelope, beating or its depth, tremolo or a filter sweep differs by at least 2 steps of about one just-noticeable difference (10 % in Hz and ms, 2 dB, 10 cents) and by more than twice the readings' noise, or is present in one option and absent in the other on every render. Options `sound/1` cannot compare as played (another phrase, a transpose, a seed, a curve) are noted for a listening review and count as no check in coverage (Q9).
- **The hint follower is the player's:** it asks the real player for its measured hints, so it sees what a player following them sees; a `touched` control is turned first, as the step's text asks.
- **Witnesses stay inside `editable`** (advisory in the session, a rule here), and `--cross` compares Do steps only (a round's scorer is its own check).
- **Unused and stray files fail** (lane C1's rule, enforced): what a pack ships, a lesson shows.

**On main (2026-10-21, `950ed99`):** `cargo xtask lesson-check` passes both packs (17 lessons, 148 steps, about 16 s), and the bundled example archive matches its pin. With `--cross --solve`, `example` passes and `basics` fails 6 steps, reported to the coordinator for C1: four ear questions whose answer is under 2 JND from a distractor with no descriptor apart either (`pulse-width#check-narrowest` 1.94, `pulse-width#check-saw` 1.19, `hard-sync#check-synced` 1.50, `velocity#check-responds` 1.82, whose difference, the second note's level, the level-matched descriptors do not see), and two Do steps whose measured hints stop where the sound passes while the predicate still fails (`resonance#try`, `filter1.resonance > 70%`; `drive#try`, `filter1.drive > 10dB`). Passing on the new rules: `how-a-lesson-works#check-different` (an odd one out, its distractors deliberately alike) and, on their descriptors, 1.4's three beat questions (`detune.beating_hz`: 0.91 against 3.84 Hz, or beating in one option only), 3.1's two swell questions (`envelope.attack_ms`) and 3.2's `check-decay` (`envelope.decay_ms`, 37 against 400 ms); 1.2's interval question is noted for a listening review (its options play different phrases), as are the transposed options of 1.1 and 3.2. All 45 rounds reach a pass with `closest`, the raised pass marks of 1.4, 3.1, 3.2 and 3.3 and the range rounds included; seven sound steps teach structure (a waveform, a transposition, a module switched on, a mode), noted. `lesson-coverage` fails, as expected until C2 and C3: 20 holes, no lesson yet for `lfo`, `reverb`, `comp`, `voice` and the macro knob and their 10 Basic-tier parameters; `osc`, `noise`, `svf`, `adsr` and `master` are covered with every kind of check, none E only, nothing taught but unchecked.

**Time:** `--cross --solve` over both packs takes 75 s with 4 jobs (317 % CPU, beside other lanes' builds; one job alone about four times that), the playthrough about 3 s a lesson and `--solve` 5–30 s (the audibility gate's `sound/1` preparation and `closest`, which stops at its first passing batch). The 42 planned lessons should take about 3 minutes on CI's 4 cores, plus the build: under the 10-minute budget.

**CI** (`.github/workflows/lessons.yml`): `lesson-check --cross --solve`, `lesson-coverage` (a report, `continue-on-error`, until GR2) and `pack --check`, on `content/lessons/**`, the tool, xtask, every crate a verdict depends on (`enth-learn`, `-session`, `-solver`, `-challenge`, `-analysis`, `-render`, `-engine`, `-patch`, `-params`, `-dsp`), the bundled packs, the lock file and the toolchain. **`scripts/gate.sh lane`** runs `lesson-check` on every pack when the change touches `content/lessons/`, then `--cross --solve` on each pack it touches.

## Learn follow-ups (lane LFU)

**Built (2026-10-21, lane LFU, after LUI and LCK; additive only).** The open items of LUI's and SKL's reports:

| Item | What changed | Where |
| --- | --- | --- |
| **The basics pack is bundled** | The plugin's `BUNDLED` is `basics` (16 lessons, the tour's first seven), built by `cargo xtask pack --out crates/enth-plugin/packs basics` (160,022 bytes, SHA-256 `f2b86e97…`) and pinned by `pack --check` and `the_bundled_packs_are_the_content_built`. The example pack stays built beside it as the tests' small fixture (`Lessons::with_bundled`), pinned the same way and not bundled | `enth-plugin/src/learn.rs`, `packs/` |
| **`learn` on by default** | The default plugin has the Learn view, pack I/O and the basics pack; `reaper-test`'s `lesson` scenario runs on the default bundle and plays the tour's first lesson. **Size** (release `.clap`, Linux x86-64, this branch): 19,504,816 bytes with `learn` against 16,626,216 without (`--no-default-features --features recreate`), +2.88 MB (+17 %); stripped, 15.06 MB against 12.77 MB, +2.29 MB. The pack is 0.16 MB of it; the rest is code: by symbol size, the serde code of the lesson, pack and progress types (about 0.5 MB with its `core` and `alloc` instances), `enth-learn` 0.2 MB, `pulldown-cmark` 0.1 MB, the Learn view, `closest` and the pack reader's `rawzip`, `miniz_oxide` and `sha2`. Report 02 put a TLS stack at 2.2 MB against an 11 MB plugin; Learn costs about as much, and is the feature | `enth-plugin/Cargo.toml`, `xtask/src/reaper.rs`, `xtask/reaper/lesson.lua` |
| **"How close am I?"** | A Learn job, `LearnJobKind::Closest`, with progress, cancel and stale results refused ([session.md](session.md#jobs-1)); the game worker runs `enth_solver::closest` on the round it holds, on scoped threads (`RoundPool`, the hint evaluator's count, the serial answer for any count), with a hard budget of 300 renders or 1,000 to **look harder** (CLS's long run: `patience` 15, `tol` 0), stopping within one batch on a cancel or when the plugin closes. In the pane, on a lesson's Recreate step after a Submit that did not pass: the button, its progress and ■ Stop, then "N moves away: …" in words (each control, which way, a touch, a little or a lot) and rings with a pointer on the cards; exact values only once the round is revealed (no peeking). A passing answer marks the round assisted, like a reveal, and **Try a new one** starts the sibling redo, which also follows a reveal or the coach's last hint ([session.md](session.md#jobs-1)) | `enth-session/src/learn.rs`, `game.rs`; `enth-plugin/src/learn.rs`, `game.rs`; `enth-ui/src/editor/learn/` |
| **Open packs folder, Get more lessons** | At the browser's bottom, both through egui's `OpenUrl`: the first asks the plugin's glue, which makes the Packs folder if it is missing and opens it in the file manager; the second opens `https://github.com/utof/enth/releases`. No network code | `enth-ui/src/editor/learn/browser.rs`, `enth-plugin/src/editor.rs` |
| **The overflow at 1200 × 800** | The 28 basics ear questions that follow a round carried the round's whole starter over and overflowed beside the pane; each now has `layout = "focus"` with nothing highlighted, so no card shows. `lesson-check --cross --solve` passes (17 lessons, 148 steps) | `content/lessons/basics/` |
| **Show me and `touched`** | One Show me could never satisfy a `touched` predicate (the touch log began at the first edit's value). `Player::show` now notes the knob's value before its op, so the jump counts as a turn across the travel it moves, as a drag would; the player's property harness mirrors it. How a lesson works' Try step now passes by one Show me | `enth-learn/src/player.rs` |

## Features and the light build

- `enth-ui/learn` and `enth-plugin/learn` gate the view, the I/O and the bundled content; **`enth-plugin/learn` is on by default since lane LFU**. A non-default `learn-download` would gate any network code.
- `enth-learn` and `session::learn` always compile, as the round's worker does, so a saved lesson restores in any build.
- Nothing in the engine or the patch format depends on these features ([architecture.md](architecture.md#light-and-heavy-builds-content-packs-and-features)).

## Tests

All planned (report 03 §8); the files are created by S0l or the lane that owns them.

| Test | Where |
| --- | --- |
| Pack, lesson, challenge-v2 and progress fixtures; a newer pack refused with a hint; a newer progress file read leniently | `enth-learn/tests`, `enth-challenge/tests/fixtures/challenge/v2/` |
| The player under random intents, edits, ticks and results (right, stale, failed), saves and loads: no panic; no pass without a passing result for the current revision; progress never shrinks; locks hold (proptest, fixed seed) | `enth-learn/tests/player_props.rs` |
| Every op kind on a locked address refused; one plan per step load; the stash survives a mid-lesson save; the budget holds | `enth-session/tests/learn.rs` |
| The scripted player: every step of every lesson through a real `Session` | `cargo xtask lesson-check` |
| kittest: two lessons by label; overlay snapshots; no-peek in lesson Recreate steps; 0 idle frames (lavapipe) | `enth-ui/tests/learn.rs` |
| REAPER `lesson`: a step, then a save and reload mid-lesson | `xtask/reaper` |
| An agent completes lessons through `enth lesson` from the text alone; a `follower` floor | The agent-eval lesson track (paid runs only with your OK) |

## GR2

**The gate** ([roadmap.md](roadmap.md#r2--learn-ongoing)): the lessons cover every P1 module, and you and your friends learn something new from them.

**The mechanical half** is `lesson-coverage` and `lesson-check --solve` in CI ([above](#lesson-check-and-the-authoring-loop)).

**The human half** (report 01 §7), everything local, reported per person:

| Part | What | Time |
| --- | --- | --- |
| Intake | Years with synths, which synths, confidence 1–5 per module family | 3 min |
| Pre-test (form A or B, counterbalanced) | 10 challenge codes, one per family, at fixed settings: pass within 3 min without hints, plus structure %; 12 mixed "what changed?" items | 25 min |
| Learning | The tour (friends) or the full path from placement (you), over 1–2 weeks, with per-lesson votes logged automatically | About 1.5 h |
| Mid-test | 5 items over all families, after the tour's first third | 8 min |
| Post-test | The other form: the same specs, other seeds | 25 min |
| Delayed (optional) | 5 items a week later | 8 min |
| Debrief | "Name up to three things you learned"; too easy or too hard | 5 min |

- **Parallel forms** rule out memorised targets. **The staggered mid-test** is a multiple baseline across skills: a family should improve only after its lesson, which separates the lessons' effect from practice on the tests.
- **Proposed pass criteria** (yours to set): every participant names a specific new thing in at least 3 lessons, coded by an agent against each lesson's objective; the median participant improves on at least 3 of 10 families, and no family gets worse on average; you name something new and pass a post-test item you failed before. With N = 5, "all improve" gives a one-sided sign test p ≈ 0.03: describe it, don't lean on it.
- **Privacy:** the progress file and the round log stay local; participants send them by choice, as GR1 works. `cargo xtask gr2-report` (INT2) reads them.
- **Time:** the full protocol is about 2.5 h per friend; the summary budgets 60–90 min (the tour alone). Which one friends do is an open check for INT2 ([phase-3.md](phase-3.md#open-checks)). Pilot it with you first.

**As built (lane INT2, 2026-10-22).** [Owner sheet 4](r2-owner.md) is the protocol as you run it, [gr2-friends.md](gr2-friends.md) the friends', and [r2-gates.md](r2-gates.md) the table of record. The friend time is settled on phase-3's default: the full protocol where a friend agrees (about 3 h), otherwise the tour with per-lesson votes and the debrief (1½–2 h), with the pre/post rows from you and the full-protocol friends. Three things differ from the table above, each because the plugin cannot do it yet: the tests are **six families**, one long challenge code per family and form (A and B for pre and post, M for the mid-test after lesson 5, D delayed), each randomizing one generator category, played as ordinary Recreate rounds and read from the round log ([content/gr2/](../content/gr2/README.md)); there are **no "what changed?" items** (ear questions play only inside lessons); and **the per-lesson votes and notes are typed** into an answers file (`enth.gr2-answers/1`). `cargo xtask gr2-report <dir>` reads each person's answers, progress file and round log and prints the items, each family better, unchanged or worse, the lessons' votes and the proposed criteria (criterion 2 scaled to 2 of 6 families).

## Decisions (provisional)

| Area | Decision | Evidence | Status |
| --- | --- | --- | --- |
| Lesson shape | Hear → Name → Try → Do → Check → Review; one idea per lesson; scaffolds fade; no spectrum during Do | Report 01 §1, §3 | Adopted |
| Curriculum | 9 units + studies in the generator's order; the 12-lesson tour first | Report 01 §4, PLAN.md | Adopted |
| Crate | `enth-learn` at L5 | Report 03 §2 | Adopted |
| Challenge v2 | `Target::Draw`, `Target::Patch`, `Start::Patch`, `Constraints { editable, locked }`, phrase ids 4–7; v1 reads as v2. `Target::Expectations` and `AnswerKind::Choice` left out (a lesson's `Check::Sound` and `StepKind::Choose` instead) | Reports 01 §5, 03 §3; the F1l ruling | Built; judged at F1l, findings closed (F1L) |
| Locks | Enforced in the session's edit path (`SessionError::Locked`); the overlay only draws them | Report 03 §3 | Adopted |
| Recreate steps | Real rounds (`RoundSource::Challenge`), `origin: lesson`, left out of `gr1-report`, never stashing | Report 03 §3 | Adopted |
| Knob feedback | `hint` for expectation steps; `closest` (a button) for patch targets; `nearest` stays R3's | Report 03 §5 | Adopted |
| Pack format | `.enthpack`; `enth.pack/1` built from TOML; SHA-256 per file; newer schema refused and named; no audio, no images in v1; the engine recorded, not a gate | Report 02 §2 | Adopted |
| Pack reader | `rawzip` + `miniz_oxide` + `sha2` + `serde_json`, pure and wasm-clean | Report 02 §5 | Adopted (coordinator pins) |
| Markdown | Our own renderer on `pulldown-cmark`, through `LessonPane` | Reports 02 §3, 03 §6 | Coordinator's call |
| Delivery | Bundled + Packs folder + a browser button; no network code in R2 | Reports 02 §4, 03 §7 | Adopted; download is yours to decide after GR1 |
| Progress | Per user, merged on write; per project only the in-flight lesson | Report 03 §3 | Adopted |
| Review | Leitner boxes per skill, fed by the rings; no streaks, no `fsrs` | Report 01 §5 | Adopted, later lane |
| Ear skills and drills (R4) | `ear.*` skills beside `module.param`; 5-item bursts move a box; `StepKind::Drill` and a `drills` progress field, additive; 3-down/1-up | [R4 research](../_local/sessions/2026-10-22-r4-r5-ear-games-and-duels/summary.md), report 01 §3 | Built (DRL, judged at F1e) |
| Content licence | Text CC BY 4.0, patches and codes CC0, code MIT | Report 02 §9 | Yours to decide; proceed on it |
| Immutable releases | A repository setting | Report 02 §4.3 | Yours to decide |
