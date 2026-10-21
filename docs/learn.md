# Learn (R2): lessons, checks and packs

R2 teaches sound design on the instrument you keep: lessons of 5–10 minutes, one control or idea each, played inside the plugin, from "what is a saw wave" to sound-design studies. A lesson's "recreate this" step is a real Recreate round, so A/B, the rings, the coach and the results drawer come free. Lessons ship as **content packs** (markdown, patches and challenges, no audio), bundled in the build. This page is the design of record; the lane plan is [phase-3.md](phase-3.md), the curriculum is lane CUR's [lesson plan](../content/lessons/PLAN.md), and the evidence is the [R2 research session](../_local/sessions/2026-10-21-r2-learn/summary.md) (reports 01 pedagogy, 02 packs, 03 architecture; the summary wins where it corrects a report).

**Status (2026-10-21): planned; nothing is built.** Every decision here is provisional until you steer; work proceeds on it meanwhile. Each lane updates its own section as it lands ([phase-3.md](phase-3.md#rules-for-every-lane)).

| Piece | Home | Lane | State |
| --- | --- | --- | --- |
| Lesson and pack model, player, checks, progress, markdown AST | `enth-learn` (L5) | LM, PLY | Planned |
| Challenge v2 | `enth-challenge` (L4) | CV2 | Planned; judged at F1l |
| Live binding, locks, the `learn` field | `enth-session::learn` (L6) | SL | Planned; judged at F1l |
| `closest` (the nearest passing patch) | `enth-solver::closest` (L5) | CLS | Planned |
| One lesson end to end in the plugin; `enth lesson` | `enth-plugin`, `enth-cli` (L8) | SKL, EV3 | Planned (F2l) |
| `lesson-check`, coverage, `pack`, `lessons.yml` | `xtask` | LCK | Planned |
| The Learn view | `enth-ui::editor::learn` (L7) | LUI | Planned |
| Deconstruct lessons, the studies | `enth-learn::deconstruct`, `xtask` | DEC | Planned |
| Review (Leitner boxes) | `enth-learn::review` | REV | Planned, later |
| The packs | `content/lessons/<pack>/` | C1, C2, C3, DEC | Planned ([PLAN.md](../content/lessons/PLAN.md)) |
| GR2 | `docs/r2-gates.md`, `gr2-report` (planned) | INT2 | Planned |

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
| `lesson-check`, `lesson-coverage`, `lesson new`, `lesson-gen`, `pack`, `gr2-report` | `tools/lesson/src/` (the package `enth-lesson-tool`, exempt tooling); `xtask/src/lesson/` and `xtask/src/gr2.rs` are thin drivers that run it, so xtask carries no engine crate | Tooling |

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

**Locks are the session's rule, not the overlay's.** `deliver` refuses an op that touches a locked address or module (`SessionError::Locked`, with a hint), so every shell and test gets the rule; the overlay only draws the lock. Policy to settle at F1l: host automation of a locked parameter folds into the patch but never counts toward a check; undo never leaves the step; a preset load asks Restore or Keep.

## The lesson model (lane LM)

**Built (2026-10-21, lane LM, to freeze F1l; the F1l judge reviews it with Challenge v2 and the session's Learn API).** `enth-learn` holds the frozen formats, the `.enthpack` reader, the player's API and a basic body, and progress v1. The code is the reference (`crates/enth-learn/src/`, one file per piece); the fixtures under `crates/enth-learn/tests/fixtures/<kind>/v1/` pin the bytes. The sketch above is superseded where they differ.

| Format | Schema | Reading | Where | Fixtures |
| --- | --- | --- | --- | --- |
| Pack manifest | `enth.pack/1` (`pack.json`; `pack.toml` when authoring) | **Strict**: unknown fields refused, a newer schema refused and named (`pack_too_new`) | `pack.rs` | `pack/v1/pack.json`, `example.enthpack.sha256` |
| Lesson | `enth.lesson/1` (`lessons/<id>/lesson.json`; `lesson.toml`) | **Strict**: `lesson_too_new`, unknown fields, and a field the step's kind does not have, refused | `lesson.rs`, `check.rs` | `lesson/v1/cutoff.json`, `every-field.json` |
| Progress | `enth.progress/1` (`<Documents>/enth/Learn/progress.json`) | **Lenient**: a newer version reads; unknown fields and undecodable entries are kept verbatim and written back | `progress.rs` | `progress/v1/progress.json`, `newer.json` (input), `newer.rewritten.json` |
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
| `requires` | `lesson` and `patch` (the patch **file schema**, 1 today, not the engine), and `challenge`, `generator`, `scorers` when a lesson has a Recreate step. The pack loads only if this build has them all (`requires_unmet`), and **a pack whose lessons use more than it declares is refused**, so `requires` never lies |
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

**`view`** (`StepView`): `layout` (`full`, `focus`), `start` (a pack patch; none: the patch carries over), `highlight`, `hide`, `editable` (**advisory**: the UI, the measured hint's free set, `closest`; empty means no restriction) and `lock` (**enforced** by the session). Selectors are `enth_challenge::v2::Selector`'s text: `filter1.cutoff`, `voice.glide`, `filter1.*` (the module, its parameters and its routes), `macro3`, `lfo1 -> filter1.cutoff`. **A Recreate step's start and locks are its challenge's**, so its view may not set `start`, `editable` or `lock`.

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
| `show(&patch)` | A Do step: the witness's next op (`ops_between`, an editable control first) as `Apply(op)`, one undo step through the session's edit path; a choose step: `Reveal` and a pass. Marks the step assisted |
| `choose(i)`, `round_ended(RoundEnd{passed, assisted, best_sound_pct})` | Ear questions; a Recreate step's round, at its first passing submit, a reveal, or when the step is left |
| `next()`, `back()`, `end(Restore \| Keep)` | Next passes a Read or Listen step and skips an unpassed one; leaving an unpassed round emits `EndRound`; the last Next emits `Finished` |
| `lesson()`, `step()`, `steps()`, `phase()`, `lesson_ref()` | Read accessors; the session saves `lesson_ref()` after every step change |

Effects: `LoadPatch` (**at most one per step load**), `StartRound(challenge)` (a Recreate step, instead of `LoadPatch`), `EndRound`, `View` (the **effective locks** every step; a Recreate step's are its challenge's `Constraints.locked`), `Examples` (resolved patches), `Check`, `HintJob`, `Hint`, `Measured`, `Apply`, `Chose`, `Reveal`, `Passed`, `Record` (merge it into progress: passing is reported at once), `Finished`, `End`. A passed step stays passed. Phases: `Working`, `Checking{revision}`, `Passed`.

**`LessonRef`**: `{"pack":"basics","version":"1.2.0","lesson":"cutoff","step":1,"content":"0123456789abcdef"}`, the first 8 bytes of the lesson file's SHA-256. On load the session resumes at `step`, or at 0 when `content` differs; the stash is kept either way. It is under 200 bytes with the longest ids, so it costs the 1 MiB `enth` field nothing next to the stash.

### Progress v1

`{"schema":"enth.progress/1","lessons":{"basics/cutoff":{"steps":{"try":{"done":true,"clean":true,"best":97}}}}}`. A step is `done` (passed or read), `clean` (passed once with no hint, Show, reveal or wrong pick) and has a `best` (a Recreate step's best sound %). **`Progress::merge` is a join**: the union of lessons and steps, `done` and `clean` or-ed, the maximum of bests; commutative, associative, idempotent, never shrinking (proptests, fixed seed). Fields and entries this build cannot read are kept and written back; between two differing kept values the greater canonical JSON text wins (still a join), and a readable entry wins over an unreadable one. `record` is a one-step merge. Only text that is not a progress file is refused, and the shell keeps such a file aside. GR2's votes and REV's boxes join additively as new fields.

### Decisions for the F1l judge

| Decision | Why |
| --- | --- |
| Ear questions are `StepKind::Choose`; Challenge v2 has no `AnswerKind::Choice` (CV2 agreed) | One representation; the player judges an index with no render |
| Sound checks are `Check::Sound` in the lesson; Challenge v2 has no `Target::Expectations` (coordinator) | One representation of an expectation list |
| A Recreate step embeds a whole `Challenge`, read by its own decoder (v1 migrates once CV2 lands) | A challenge stays self-contained (inline patches, CV2) |
| Lock selectors are CV2's `Selector`; `lock` enforced, `editable` advisory (coordinator) | One matcher in the session for steps and rounds |
| Predicates are structured objects with text atoms, not one-line strings | Frozen now without PLY's grammar; addresses, units and expectations reuse existing parsers |
| Steps are flat objects with a `kind` tag and kind-specific fields, a foreign field refused | Natural TOML (`[[steps]]`, `[steps.check]`), and nothing silently ignored |
| `requires` must cover what the lessons use | A pack cannot under-declare and then fail inside a lesson |
| Progress keeps what it cannot read | An older build that merges a newer file loses nothing |
| `Example.transpose` added beside Q7's three fields | The cards' "at C2 and C4" |

**Until CV2 merges** (`interim.rs`, deleted then): `Selector`, `Touch`, `score::is_known` and the lesson-only phrase lookup are stand-ins with the same names and text form, and a Recreate step's effective locks are empty because Challenge v1's `Constraints` is; the F1l merge makes each a one-line `pub use` and `challenge.constraints.locked`.

## Challenge v2 (planned; judged at F1l)

Challenge v1 has only `Target::Seed`, `Start::Template`, an empty `Constraints {}` and `AnswerKind::Patch` ([challenge.md](challenge.md#challenge-v1)), so it cannot express a lesson. Lane CV2 adds, additively and with a v1 migration (v1 reads as v2):

| Piece | What | Lessons that need it ([PLAN.md §8.2](../content/lessons/PLAN.md#82-challenge-v2-and-the-other-learn-pieces)) |
| --- | --- | --- |
| `Target::Draw` | A seeded draw over a parameter list with value sets, drawn in parameter space with design §9's keyed streams and just-noticeable steps, so a new seed gives a sibling for a redo or a parallel test form. Route depths and their sign are drawn too (Q4, provisional) | 32 core lessons |
| `Target::Patch` | An explicit target patch (hand-made or generated) | 8.1's sibling, every study step |
| `Target::Expectations` | An expectation list, as `enth check` reads it | 33 lessons with a sound check, and every Try that uses one |
| `Start::Patch` | The step's own start patch, not a template | Every Do |
| `Constraints { editable, locked }` | What the player may change; what the session refuses | Every Do; `locked` in 19 cards |
| `AnswerKind::Choice` | An ear question: options rendered live, pairwise at least 2 just-noticeable steps apart | All 42 |
| New phrase ids | `CHORD`, `LONG_HOLD`, `STACCATO`, `STACCATO_RUN`; the table split into generator phrases (ids 1–3, generator v1 untouched) and lesson-only phrases with their own invariants (Q8, provisional) | 6.1, 8.2, 5.3, 5.4, 7.1–7.4 |

`prepare_patch_round` (the "hand-made round on a patch target" that lives in the CLI's `compare.rs` today) moves down into `enth-challenge`, so the session and xtask share it; `enth compare --target` and the golden challenges stay byte-identical. The judge reviews v2 at F1l together with the lesson model and the session's Learn API, because all are persisted.

## Checks: four kinds

Sound alone cannot check every P1 module (report 03 §4, from the [agent-evals findings](agent-evals.md#findings-for-the-descriptor-and-cli-owners)), so a step's goal is one of four kinds:

| Kind | Passes when | Works for |
| --- | --- | --- |
| **P** patch state (`Check::Patch`: `Value`, `Route`, `Has`, `Enabled`, `Touched`) | The live patch satisfies every predicate, checked after each delivered edit | Every control; the only kind that does |
| **S** sound expectation (`Check::Sound { phrase, expect }`, the expectation grammar of `enth-analysis`) | The rendered phrase meets every item with a margin | Pitch, filter, noise, envelopes, LFO rate and depth, master level |
| **R** Recreate score (a real round with `Constraints.editable`) | `structure/1` and `sound/1` reach the pass mark (90 %) | Oscillator, filter, noise, voices with the right phrase; weak for short envelopes and effects; never gauges or nuisances |
| **E** ear question (`AnswerKind::Choice`) | The player picks the right option | Everything; the only reliable check for resonance, reverb, the compressor and macros |

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

## Knob feedback

- **Expectation steps:** `enth-solver::hint` returns a bounded Gauss-Newton move from your patch over every failing item, verified by renders (10–16 a hint), sparsified, and `blocked` when a bound stops it. That *is* nearest-solution feedback for these steps; it is shown graded like the coach: aspect, then module, then parameter, then range.
- **Recreate steps:** structure rings are instant, with no render. Design §7's closest passing patch (CMA-ES from your patch, penalising distance, per-parameter deltas) becomes `enth-solver::closest`, with the free set limited to the step's editable parameters (usually 1–4). A scoring render costs 15–30 ms, and level-2 `match` runs took a median of 423 renders, so it is a **"How close am I?" button** with progress and cancel, never live; lane CLS measures it on real lesson starts first. Using it triggers a sibling redo, like a Reveal.
- **Naming:** R2's feature is `closest`. `nearest` is R3's ("the nearest preset as a start"), filled by R3a's k-NN retrieval ([r3.md](r3.md#r3a-create-from-sound-with-search-alone)).

## The Learn view

- **The seam,** copied from Recreate: `Editor::set_lesson(LearnEdit::lesson())` before the frame, `Intent::Learn(LearnIntent)`, `Editor::apply_learn(&mut dyn LearnEdit)` after. A lesson's round uses the Recreate UI unchanged.
- **Overlay additions:** `Emphasis::Highlighted` (a static accent outline, so no repaint timer) and a `locked` flag (a lock glyph, with the refusal on the status line). Markdown links such as `[Cutoff](enth:filter1.cutoff)` highlight their control on hover, and `lesson-check` verifies they resolve.
- **Layout:** a 320 px pane fits 1200 × 800 only when the step hides unused modules (the G1 patch overflows by 276–732 px), so LUI needs a collapsible pane or a focus-card region, with an overflow report.
- **Markdown:** our own small renderer. `enth-learn` parses each step into a block AST with `pulldown-cmark` (MIT), and `LessonPane` draws it with design tokens only. `egui_commonmark` was rejected for now: +875 KB, hard-coded colours and emoji, one maintainer, and releases tied to egui's. The lint bans HTML, images and code blocks and allows `https:` links only. Revisit if lessons need tables or footnotes.
- **Repaint:** Learn asks for no frames of its own; job results wake the editor, as `wake_at_ms` and the worker's `request_repaint` do for rounds.

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

## Review (lane REV, later)

Every Recreate round's Good, Near and Bad rings give a free signal per skill (a module kind and parameter key, such as `svf.cutoff`). A Leitner box per skill in the progress file, and a Review that serves the weakest due skills as seeded sibling rounds and quiz items, give weakness-targeted drilling with no model to train. No `fsrs` (it pulls in ndarray, rayon and rand for about 50 skills). REV is on the cut list: it may move after GR2.

## Deconstruct lessons (lane DEC)

"Deconstruct this preset" lessons are generated by ablation (report 01 §6):

1. **Elements:** each enabled module, each route, and each group of parameters that differ from Init.
2. **Ablation:** render the preset without each element and measure the `sound/1` distance plus the coach's descriptor changes.
3. **Prune:** an element below one just-noticeable step becomes "fine detail" and gets no step.
4. **Order:** by signal flow (source, filter, amp envelope, modulators, effects, macros), then by contribution; step *k*'s target is Init plus elements 1 to *k*, because elements interact.
5. **Each step:** hear the target with and without the element; rebuild it with only that element editable (a `Target::Patch` step); the coach is on.
6. **Text:** a template from `learn.desc` and the measured changes; factory studies get human-edited text, your own presets get automatic lessons.
7. **Close:** "What disappeared?" (pick 1 of 4), then a variation goal checked by expectations derived from the preset.

At most about 8 steps a lesson. The planner, `deconstruct::plan(preset, ranks)`, is pure: the caller measures the sensitivity `reach` that orders it. `lesson-gen deconstruct` writes the factory studies at authoring time, and the plugin's worker runs the same planner on your presets. DEC prototypes on Acid Snap and Glass Pluck before the studies, and waits for fu1-drv (every candidate carries a Drive macro).

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

## Features and the light build

- `enth-ui/learn` (it exists, empty) and a new `enth-plugin/learn` gate the view, the I/O and the bundled content; a non-default `learn-download` would gate any network code.
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

## Decisions (provisional)

| Area | Decision | Evidence | Status |
| --- | --- | --- | --- |
| Lesson shape | Hear → Name → Try → Do → Check → Review; one idea per lesson; scaffolds fade; no spectrum during Do | Report 01 §1, §3 | Adopted |
| Curriculum | 9 units + studies in the generator's order; the 12-lesson tour first | Report 01 §4, PLAN.md | Adopted |
| Crate | `enth-learn` at L5 | Report 03 §2 | Adopted |
| Challenge v2 | `Target::Draw`, `Target::Patch`, `Target::Expectations`, `Start::Patch`, `Constraints { editable, locked }`, `AnswerKind::Choice`; v1 reads as v2 | Reports 01 §5, 03 §3 | Judge at F1l |
| Locks | Enforced in the session's edit path (`SessionError::Locked`); the overlay only draws them | Report 03 §3 | Adopted |
| Recreate steps | Real rounds (`RoundSource::Challenge`), `origin: lesson`, left out of `gr1-report`, never stashing | Report 03 §3 | Adopted |
| Knob feedback | `hint` for expectation steps; `closest` (a button) for patch targets; `nearest` stays R3's | Report 03 §5 | Adopted |
| Pack format | `.enthpack`; `enth.pack/1` built from TOML; SHA-256 per file; newer schema refused and named; no audio, no images in v1; the engine recorded, not a gate | Report 02 §2 | Adopted |
| Pack reader | `rawzip` + `miniz_oxide` + `sha2` + `serde_json`, pure and wasm-clean | Report 02 §5 | Adopted (coordinator pins) |
| Markdown | Our own renderer on `pulldown-cmark`, through `LessonPane` | Reports 02 §3, 03 §6 | Coordinator's call |
| Delivery | Bundled + Packs folder + a browser button; no network code in R2 | Reports 02 §4, 03 §7 | Adopted; download is yours to decide after GR1 |
| Progress | Per user, merged on write; per project only the in-flight lesson | Report 03 §3 | Adopted |
| Review | Leitner boxes per skill, fed by the rings; no streaks, no `fsrs` | Report 01 §5 | Adopted, later lane |
| Content licence | Text CC BY 4.0, patches and codes CC0, code MIT | Report 02 §9 | Yours to decide; proceed on it |
| Immutable releases | A repository setting | Report 02 §4.3 | Yours to decide |
