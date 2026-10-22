# 02 — R4 Ear games and R5 Duels: architecture, code structure, risks and the lane plan

- **Date:** 2026-10-22. Sub-researcher 02 ([brief.md](brief.md)).
- **Method:** the docs the brief names; the codebase-memory graph (project `enth`, ready, no partial files) and grep on main `aacaa99`; the vendored nice-plug and egui-baseview sources; a read-only probe with the existing debug CLI (60 generated targets, levels 1–5 × 12 seeds; scripts in `~/code/enth-scratch/2026-10-22-r4r5/02/probe/`); a few dated web pages (end). Nothing built, no audio played.
- **Labels:** *measured* (read or run today), *documented* (a repo or vendor doc), *anecdotal*. Game choice is report 01's; chat UX, trust wording and P2P are report 03's.

## 0. The answer in eight lines

1. **R4 needs no engine change.** Every stimulus is an offline render of a `Performance` (any notes, any times) played through the existing Target ▶ examples slot (≤ 8 position-locked loops). Polyphony 1–16 and 12-TET pitch through pinned `libm` exist (*measured*).
2. **Homes:** a small pure crate **`enth-music` (L1)** (theory, rhythm, the frozen **ear generator v1**); **Challenge v3** in `enth-challenge` (`Target::Ear`, `AnswerKind::{Choice, Value, Taps}`, scorers); a **drill player** in `enth-learn` (staircase, ear skills in Review); `session::{ear, duel}`; `editor/ear/` and a duel card.
3. **Round types:** label choice, clip choice (today's `Choose`), A/B and 2AFC on a staircase, slider estimate, "match it" (a Recreate round on a v2 draw, already built), tap-back rhythm. Answer keys are seeded integers, so **ear duels verify without rendering**.
4. **EQ A/B:** a peaking and shelf biquad in `enth-dsp`, applied offline to rendered clips; S2's EQ module later wraps the same primitive.
5. **Rhythm timing is the real unknown:** the editor sees input once per frame (about 30 fps in Phase 0) and the fork leaves the keyboard to the host. So v1 is **call and response scored on inter-onset intervals** (latency cancels), taps stamped on arrival (fork patch) or taken as host MIDI at sample offsets, chosen by a measured spike.
6. **R5 result codes** take the reserved escape version 15: the challenge code, the engine and scorer set, the claim, time, assists, and **the answer as `ops_between(start, answer)` in binary** with a 32-bit answer fingerprint. That gives **median 51 / 72 / 141 / 165 / 254 symbols** at levels 1–5, about 1,100 at worst (*measured* on targets). Ear results are 40–60.
7. **Honest limits:** verification proves "this patch scores X on this build", never who found it or how fast. `enth match` passes 11–13 of 20 golden and GR1 codes in 6–12 s (*documented*).
8. **Plan:** four freezes, one heavy slot (R3b holds the other), **about 26–44 agent-hours**. **R5a (async duels on Recreate) ships first**, critical path about 6–11 h. Your new time is about 2–3 h.

## 1. What exists today (*measured*, main `aacaa99`)

| Hook | Where | Gap for R4/R5 |
| --- | --- | --- |
| `Performance { notes, automation, controls }`; onsets on the 32-frame grid (0.67 ms at 48 kHz) | `enth-render` | Stimuli from a generator, not a `PhraseId` table |
| 32 voice slots, `voice.polyphony` 1–16 (default 8), mono, legato, glide | `enth-engine/src/voice.rs` | None |
| `midi_to_hz` = 440·2^((n−69)/12) via `libm`; `Note.key` is `u8` | `enth-dsp` | Cents through `osc.fine` |
| Phrases 1–7, including a 4-note `chord`; lesson invariants (lowest C3, ≤ 8 at once, ≤ 10 s) | `phrases.rs`, `lesson_phrases.rs` | Ear invariants of their own |
| `Choose` (2–6 clip options), the Examples job, `ListenState`, `choice_audibility` (≥ 2 JND by `sound/1`) | `enth-learn`, `session/learn.rs` | Label options |
| `TargetLoop::examples`: ≤ 8 loops, `AtomicPtr` mailbox, never frees on the audio thread | `enth-plugin/src/game.rs` | A "play from the top" flag (additive) |
| Codes: Crockford base32, a GF(32) check; **version 15 reserved** for result codes; `MAX_SYMBOLS = 64` | `enth-challenge/src/code.rs` | A result kind (§5) |
| Challenge v2: draws, patch targets, locks; `AnswerKind::Choice` left "for R4's ear training" | `v2.rs` | Challenge v3 |
| `ops_between(a, b)`, checked, used by preset loads | `enth-patch/src/diff.rs` | The answer form |
| The round record keeps the **latest** `submitted_patch` | `session/game.rs` | The patch at the first pass (additive) |
| Leitner boxes per skill; "ear questions as Review items" listed as open | `enth-learn/src/review.rs` | Ear skills, drill items |
| Svf (voice FX): lp, bp, hp, notch, no gain; noise: white, pink | catalogue | No EQ |

## 2. Where R4 lives

| Piece | Home | Why |
| --- | --- | --- |
| Interval, chord, scale and progression tables (simple voice leading), a 16-step rhythm grid, a syncopation measure, note names; **ear generator v1** (spec → questions), frozen with a golden | **new `enth-music`, L1** (needs `enth-dsp`'s `derive` and `fnv1a64` only) | Pure and tiny; the UI and CLI get labels without the challenge crate; spares `enth-challenge` (15.8k lines) a third owner set. It cannot be L0, because same-layer edges are refused |
| Challenge v3 (`Target::Ear(EarSpec)`, `AnswerKind::{Choice, Value, Taps}`); `ear.rs` (question → timbre patch, `Performance`, optional post-EQ); timbre pool `timbres-v1/` (frozen, append-only); scorers `ear/1`, `ear-value/1`, `taps/1`; the ear-challenge code kind | `enth-challenge` (L4) | Persisted challenge types live here; strictness holds (an ear target requires `phrase` 0 and the template start) |
| Result codes (`result.rs`, `answer.rs`), the frozen `result-params-v1` table, **`verify(result, evaluator)`** | `enth-challenge` | One composition for the plugin, CLI and tests |
| A peaking and shelf biquad (RBJ form; coefficients through the `math` door) | `enth-dsp` (L0) | One primitive for the game EQ now and S2's EQ later |
| Drill player: N questions, the 2-down/1-up staircase, streaks, ear skills (`ear.interval.m3`, `ear.eq.1k`, `ear.param.svf.cutoff`) into Review's boxes; the progress field `drills` (additive); `StepKind::Drill` (additive, with `requires`) | `enth-learn` (L5) | Progress and Review live here |
| `ear.rs` (questions as jobs, answers and taps with times from the shell, nothing saved per project, like Create); `duel.rs` (build a result from a round, `Verify` job) | `enth-session` (L6) | The seams of `game`, `learn`, `create` |
| `editor/ear/` and the duel card, features `ear` and `duel`, header hooks, `sizes/{ear,duel}.rs` | `enth-ui` (L7) | A 320 px right column like Create's, so a parameter game keeps a card in view |
| Jobs, examples at the host's rate, clipboard; `enth ear …`, `enth duel result\|verify` | `enth-plugin`, `enth-cli` | Shells translate |
| `ear-check` (oracle, random and deaf players, audibility per level, the golden) | `tools/lesson`, driven by `cargo xtask ear-check` | As `lesson-check` |

**Rejected:**
- **All inside `enth-challenge`.** It is one crate fewer (take it if you prefer), but the result codes, v3 and the ladder tuning would collide.
- **An L4 `enth-ear`.** Challenge must embed `EarSpec`, and an L4 → L4 edge is refused.
- **Drills as `Choose` steps.** `Choose` options are clips, while drills are open-ended and use labels.

```rust
pub struct EarSpec { game: u8, generator: u8, level: u8, seed: u64 /*≤2^53-1*/, count: u8 }
pub struct Question { timbre: u8, notes: Vec<NoteSpec /*key,vel,start_ms,hold_ms*/>, length_ms: u32,
                      post: Option<EqBand /*integers*/>, answer: Answer, options: Options }
pub enum Answer { Choice(u8), Value { key: &'static str, bits: u32 }, Taps(Vec<u32 /*ms*/>) }
```

## 3. Round types and deterministic targets

| Round | Answer | Target | Scored | Reuses | New |
| --- | --- | --- | --- | --- | --- |
| **Label choice** (interval, chord, note, band, which knob changed) | index | generator | exact | examples slot | buttons, restart flag |
| **Clip choice** | index | generator or draw | exact | `Choose` as is | — |
| **A/B, same/different, 2AFC** (cents, cutoff ratio) | index | generator + staircase | exact; replayed from the answers | 2 locked loops | staircase |
| **Slider estimate** (attack? cutoff?) | value | 1-item draw | error in perceptual units (octaves, log ms) | knob, v2 draw | `ear-value/1` |
| **Match it** | patch | v2 draw, others locked | `sound/1` | **the whole Recreate round** | nothing |
| **Tap back** (rhythm, syncopation) | onsets | rhythm generator | IOIs after best-fit offset, ±10 % tempo, misses and extras | count-in in the clip | §4, `taps/1` |

What the engine and generator need:
- **Polyphony:** chords of up to 6 notes, and progressions with release tails. Timbre patches set polyphony to at least 8; no engine work.
- **Phrases:** generated, within C1–C7, ≤ 8 notes at once, ≤ 10 s, integers only. One render is about 15–30 ms (*documented*), so a 10-question set prepares in about 0.3 s.
- **Timbres:** a frozen pool of our own patches (sine, saw low-pass, pluck, organ table, pad, noise-burst click). Pitch drills rotate them, so the timbre is no cue.
- **EQ:** one band from the level's set (7 octave bands at ±12 dB, down to ⅓-octave at ±3 dB) on pink noise or a pad, played as 2 level-matched loops; integer parameters.
- **Parameter games** gate their options with `choice_audibility`. `sound/1`'s deaf spots (attacks under about 60 ms, slow beating, sustain, pluck resonance) fail items people hear, so those need a listening review or are left out.
- **Duel determinism:** answer keys are integer arithmetic, identical on every platform and engine version. Only a generator change moves them, and that is a new frozen version.

## 4. Rhythm input timing in a plugin

**Facts:**
- egui-baseview sets `RawInput.time` once per frame in `draw()`, and events from `on_event` wait for that frame (*measured*, source).
- egui 0.37's events are "in-order events received this frame" (*documented*).
- At Phase 0's ~30 fps, an editor tap is quantised to up to about 33 ms.
- Fork patch 4 gives keyboard focus only while a text field is edited, so keys reach the host unless the tap pad takes focus. Clicks always arrive.
- Host notes reach `process()` with `NoteEvent::timing()`, a sample offset within the buffer (CLAP's header `time`, *documented*).
- `Transport` comes per block.

**Recommendation:**
1. **No host transport:** stimuli carry their own tempo and count-in, so duels never depend on a project.
2. **Rhythm v1 is call and response.** `taps/1` scores inter-onset intervals after a best-fit offset, with a one-to-one DP match, so constant output, block and input latencies cancel.
3. **Two tap sources, picked by lane TAP's measurement:**
   - *(a)* **Editor taps stamped on arrival:** fork patch 12 (lane FK4) records an `Instant` per baseview event, plus a focus-taking tap pad. No audio-thread contact.
   - *(b)* **Host MIDI in the response window:** `Player` writes note-on sample positions into a preallocated `rtrb` ring (with an overflow test under `assert_no_alloc`). It is sample-exact on our side, but it is a new audio-thread path, against the brief's rule, so it needs your OK. Hosts trade input latency against jitter (Ableton, *documented*); REAPER's current live-input offsets are **unmeasured** (*anecdotal*, 2012-era test).
4. **"Tap along" later:** a calibration screen (16 clicks; the median offset kept in progress) absorbs at most one block of jitter.

**Missing evidence:** real jitter on your display, in REAPER, on Windows and on macOS.

## 5. R5 result codes

**Format** (escape version 15, a 5-bit kind, a 4-bit layout):

| Field | Bits | Note |
| --- | --- | --- |
| escape; kind (1 recreate result, 2 ear challenge, 3 ear result, 4 lesson sibling reserved); layout | 5 + 5 + 4 | Unknown → `code_too_new` |
| the challenge code's payload (short 64; long 169 + 7k) | 65–211 | One paste carries the challenge too |
| engine version; scorer set (index into a frozen table of scorer-id tuples) | 24 + 8 | `sound/2` will be a new set; unknown → `scorer_too_new` |
| claimed sound % and combined %, passed | 15 | Checked against the local rescore |
| time to pass (s), submits, assists (hints, reveal, closest, dim, meter) | 29 | Self-reported |
| answer form (ops, deflated patch, ear answers) and payload | 2 + n | Below |
| answer fingerprint prefix (`of_patch`) | 32 | Detects decoding drift or corruption (2⁻³² miss) |
| GF(32) check | 5 | As in v1 |

**The answer** is `ops_between(start, answer)`, checked. Each op is a 4-bit kind and a module reference (kind and ordinal, 8 bits). A `SetParam` adds a 4-bit parameter index and a value: enums by item count, toggles 1 bit, floats as exact `f32` bits. Indices go through the frozen, append-only `result-params-v1` table, as generator v1 freezes its catalogue snapshot.

**Privacy:** drop `SetMeta` (`PatchMeta` has `author`), labels, macro names and slots before encoding; the full-patch fallback strips the same.

**Size** (*measured* on targets as stand-ins for passing answers; real answers touch more knobs, likely 1.2–2× the ops, unmeasured):

| Level | Ops (median, max) | Op-form symbols (median, max) | With hyphens | Deflated full patch |
| --- | --- | --- | --- | --- |
| 1 | 1, 3 | 51, 62 | 63 | 924–946 |
| 2 | 4, 5 | 72, 81 | 90 | 928–952 |
| 3 | 11, 13 | 141, 154 | 176 | 999–1,028 |
| 4 | 14, 16 | 165, 186 | 206 | 1,031–1,079 |
| 5 | 25, 33 | 254, 309 | 317 | 1,055–1,117 |

- Changing every leaf of the 114-leaf starter is about 1,000 symbols. The fallback is used only when shorter (added modules).
- The op form fits Discord (2,000, or 4,000 with Nitro) and Telegram (4,096), but not one SMS (*documented*).
- Crockford's alphabet and `-` lie inside QR's 45-character alphanumeric set (*documented*).

**Verification** (`result::verify`: game worker, or `enth duel verify`):
1. decode;
2. regenerate the challenge;
3. apply the ops and check the fingerprint;
4. score with the named scorer set;
5. compare with the claim.

One prepare plus one scored render takes under about 1 s. Ear results need no render: the answers are replayed against the key, and staircases replay from the answers.

**Versions:**
- **Each side judges both patches on its own build.** Another engine version is a warning ("claimed 93 on 0.3.0, 92 here"). An unknown generator, kind or layout is `code_too_new`; an op on a module kind this build lacks is `answer_too_new`.
- **Old builds:** `decode` stops above `MAX_SYMBOLS = 64` with `bad_length` before it reads the version (*measured*). R1–R3 builds therefore call result codes "bad length". This cannot be fixed afterwards; the friend sheet says "update".
- Share text is one summary line plus the code as one token; the paste box tries each token, and a name never enters the code.

**What determinism can and cannot stop:**

| Stops | Cannot stop |
| --- | --- |
| Typos and mangling (the check and the fingerprint) | A self-reported time |
| Silent engine drift (version plus local rescore) | `enth match` or `challenge show --reveal` answers |
| Score disputes (same build, same bits on Linux, Windows, macOS, wasm) | Copying a friend's patch from their code |
| A fake "passed" (the receiver rescores) | False assist flags; ear keys anyone can compute |

The UI says **"verified score"**, never "verified player". It hides a friend's patch until you have played or given up (still decodable, since the source is open). Afterwards you can audition or load it (one undo step), and the duel becomes a lesson. **Daily challenge:** a seed formula of the UTC day, top 40 bits (*documented* in challenge.md), gives the Wordle pattern without a server. Live duels later send the same codes, with transport only in L8, after GR1.

## 6. Tests

| Test | Where |
| --- | --- |
| Theory tables; every generated question valid (proptest); **ear generator v1 golden** (1,000 specs → hash) on Linux, Windows and wasm | `enth-music` |
| v3 fixtures; v1 and v2 bytes unchanged; ear-target strictness | `enth-challenge` fixtures |
| Results: round trip (proptest); every single substitution and adjacent swap caught; frozen fixtures with a pinned SHA; a size budget test; no meta, label or name survives; refusals | `enth-challenge` |
| **The 25 golden challenges: target → result → verify gives the same score bits on Linux, Windows and wasm** | `tests/golden` |
| Staircase vs a scripted logistic listener converges near 70.7 % in 60 trials; progress merge laws | `enth-learn` |
| Ear and duel lifecycles; stale results refused; no audio-thread contact under `assert_no_alloc` | `enth-session` |
| `ear-check`: oracle 100 %, random and deaf at chance (binomial test), audibility per level | xtask, `ear.yml` |
| kittest: drills by label, a snapshot per round type, 0 idle frames, spoiler hiding | `enth-ui/tests` |
| REAPER `ear` and `duel` (two instances); MIDI taps at known offsets in the test host | `xtask/reaper`, `enth-test-host` |
| Agent probes: an agent with the CLI answers ear games and verifies duels (paid only with your OK) | `content/agent-evals` |

## 7. Risks and unknowns

| # | Risk | Retirement |
| --- | --- | --- |
| 1 | **Tap jitter** (frame-quantised editor; host focus; per-host MIDI offsets unknown) | IOI scoring; the TAP spike; your REAPER check |
| 2 | **Scope:** six round types × five families × ladders | R4a first (label, clip, A/B, estimate, match: intervals, chords, parameters, EQ); rhythm and progressions after TAP |
| 3 | **Parameter games auto-gate weakly** (`sound/1` deaf spots) | Listening review; "not gated" list |
| 4 | **Cheating is easy** | Honest wording, flags, hidden patch, the social model (report 03) |
| 5 | **Old builds say `bad_length`** | Friend sheet; per-kind limit in R5 builds |
| 6 | **Scorer drift** after GR1's `sound/2` | The scorer set in every code; old ids kept |
| 7 | **Format sprawl** (v3, three code kinds, the param table, drill field, `StepKind::Drill`) | One judge at F1e; fixtures |
| 8 | **One heavy slot** (R3b's ONX, PUIb, INTb; TR needs the machine alone) | R5a first; FK4 small; heavy lanes in series |
| 9 | **S2's EQ collides** with the biquad | BQ owns `biquad.rs`; S2 wraps it |
| 10 | **Your ears:** GR4 needs listening, and you cannot always listen | Mechanical half automated; short sittings; defaults |
| 11 | **Columns squeeze cards** at 1200 × 800 | Ear and Create columns exclusive; an overflow report |
| 12 | **Pedagogy fit** (adult absolute pitch is limited) | Report 01; honest copy |

**Unknowns:** whether the tap pad's focus grab works with fork patch 4; real answer sizes; session length (report 01); whether every REAPER setup runs FX with the transport stopped (Target ▶ already relies on it).

## 8. The lane plan

**Freezes:**

| Freeze | Merged | Gate |
| --- | --- | --- |
| **F0e** | S0e: `enth-music = 1` (layers.toml, AGENTS.md); stubs for §2; features `ear` and `duel`; hooks; `sizes/{ear,duel}.rs`; code kinds reserved; `ear.yml` stub | Done-means green by exit status; `wasm-check` covers `enth-music`; fixtures unchanged |
| **F1e** | MUS API, CV3, RC, DRL API, SE | **A judge agent (Opus)** reviews them together (all persisted, all go to friends). Named checks: size budget, privacy, refusals. Then F1E |
| **F2d** | SKD: copy a result after a pass, paste and verify in a second instance | REAPER `duel`; `validate`, `host-render`, `leak-test` |
| **F2e** | SKE: intervals end to end in the plugin and through `enth ear` | `ear-check` on it; REAPER `ear`; the same three gates |

After F1e the persisted forms change only additively; after F2d and F2e, the types too.

**Lanes** (agent-hours; H = heavy plugin, egui or vendor build):

| Lane | Owns | Depends | Size | H/L | Acceptance |
| --- | --- | --- | --- | --- | --- |
| **DOCS4** | `docs/phase-4.md`, `ear.md`, `duels.md`, the summary's rows | Summary | 0.75–1 | L | Links resolve |
| **S0e** | F0e | DOCS4; no root-manifest edits in flight | 0.5–1 | H | F0e row |
| **MUS** | `enth-music` + generator v1 and its golden | F0e | 1.5–3 | L | Proptests; golden on 3 targets |
| **CV3** | v3, `ear.rs`, timbres, 3 scorers, ear code | F0e, MUS API | 1.5–2 | L | Old bytes unchanged; v3 fixtures |
| **RC** | `result.rs`, `answer.rs`, the param table, `verify` | F0e | 1.5–2.5 | L | Round trip, mutations, size, privacy |
| **SE** | `session/{ear,duel}.rs`, the passing patch | F0e | 1.5–2 | L | Lifecycle; no audio contact |
| **DRL** | `drill.rs`, staircase, Review skills, `StepKind::Drill` | F0e | 1–2 | L | Staircase simulation; merge laws |
| **BQ** | `enth-dsp/src/biquad.rs`, `docs/dsp/biquad.md`, golden | F0e | 0.5–1 | L | Bits on 6 targets; within 0.1 dB of the cookbook |
| **F1E** | The judge's must-fix items | Judge | 0.5–1.5 | L | All closed |
| **SKD** | `plugin/duel.rs`, a minimal card, `enth duel`, REAPER `duel` | F1e | 1.5–2.5 | H | F2d row |
| **DUI** | `editor/duel/*` (share, paste, verify, side by side, spoilers) | F2d | 1–2 | H | kittest; 0 idle frames; leak |
| **INT5** | R5a ships: GR5 table, friend duel sheet, friend build | DUI | 1–1.5 | H | Full gate, REAPER, leak |
| **SKE** | `plugin/ear.rs` (restart flag), a minimal panel, `enth ear`, REAPER `ear` | F1e | 2–3.5 | H | F2e row |
| **FK4** | Fork patch 12: per-event arrival instants | F0e | 0.5–1 | H, small | Stamps under Xvfb; leak gate unchanged |
| **EUI** | `editor/ear/*` | F2e | 2–3.5 | H | Each round type by label; overflow; leak |
| **TAP** | Spike, then the chosen tap path | EUI, FK4 | 1.5–2.5 | H | Jitter table; exact MIDI offsets; your check |
| **ECK** | `ear-check`, `ear.yml` | F1e | 1–1.5 | L | Catches a seeded bad game per kind; under 10 min |
| **G1–G4** | Ladders: intervals and pitch; chords and progressions; EQ; parameters | ECK (G3 after BQ) | 1–1.5 each | L, Opus | `ear-check` green |
| **EV4** | Ear and duel agent tracks | SKE's CLI | 0.75–1.5 | L | Recorded |
| **INT4** | GR4 table, owner sheet 5, friend build | All | 1.5–2.5 | H | Full gate, REAPER, leak, soak |

**Waves** (slot L only, since slot S stays R3b's; at most four light lanes at once):

| Wave | Slot L | Light | Starts when |
| --- | --- | --- | --- |
| 0 | — | DOCS4 | The summary is sent (don't wait) |
| 1 | S0e (→F0e) | — | No R3b root-manifest edits in flight |
| 2 | FK4 | MUS, CV3, RC, SE (→F1e, judged); then DRL, BQ | F0e |
| 3 | SKD (→F2d) | F1E, ECK, ladder drafts | F1e |
| 4 | DUI, then INT5: **R5a ships** | G1, G4, EV4 (duel) | F2d |
| 5 | SKE (→F2e) | G2, G3 | INT5 (or F1e for R4 first) |
| 6 | EUI | EV4 (ear) | F2e |
| 7 | TAP | rhythm ladder | EUI, FK4 |
| 8 | INT4, then owner build 5 and a friend round | — | All merged |

**Size:**
- **About 26–44 agent-hours** (22–38 of code, 4–6 of ladders).
- **Critical paths:**
  - R5a: S0e → RC + SE → judge → SKD → DUI → INT5, 6–11 h.
  - R4: S0e → MUS + CV3 → judge → SKE → EUI → TAP → INT4, 9.5–17.5 h, behind R5a's heavy lanes in the one slot.
- **Calibration** (*measured*, git log): R2 was planned at 9–13 h of wall time and took **about 25 h**, from DOCS3 at 2026-10-21 03:15 to INT2 at 2026-10-22 04:02, alongside R3a. R1/S1 took 12 h against 8–11.
- So expect **about 12–18 h of wall time to R5a and 25–40 h to R4**. Your sittings and TAP set the end date.

**Your time:**

| What | Time | Default |
| --- | --- | --- |
| Read the summary; the §9 decisions | 30–40 min | §9 defaults |
| Optional taste check: one exported WAV set per game | 15 min | Report 01 + `ear-check` |
| **Duel check:** one async duel with a friend over your chat | 20–30 min (+ friend) | Two-instance REAPER test; GR5 waits |
| **Owner build 5:** Ear view, 4 games, taps in REAPER (keys, mouse, MIDI if you have it) | 45–60 min | GR4 waits; editor stamps |
| GR4 sessions, plus two friends | 3 × 10–15 min | GR4 waits |

**Proposed gates** (the roadmap has none):
- **GR4:** each R4a game passes `ear-check`; you and two friends play three sessions, and the staircase level rises on at least two games; you want more.
- **GR5:** three async duels finish with codes verified both ways, and any disagreement is explained by a flagged engine difference.

## 9. Open questions for you

1. **R4a games:** intervals, chords, synth parameters, EQ band ID; rhythm and progressions after TAP; perfect pitch as a light drill? *Default: yes, subject to report 01.*
2. **Tap source:** editor stamps (fork patch 12), or host MIDI through a new audio-thread ring? *Default: measure both, editor first; MIDI only with your OK.*
3. **EQ:** offline game EQ now, or wait for S2's module? *Default: now; S2 reuses the biquad.*
4. **`enth-music` (L1)**, or a module in `enth-challenge`? *Default: the crate.*
5. **Codes carry the patch** (about 50–310 symbols; about 1,100 worst), or the score only (about 40, unverifiable)? *Default: the patch.*
6. **R5a before R4?** *Default: yes.*
7. **Hide a friend's patch until you have played; a daily challenge?** *Default: yes to both.*
8. **GR4 and GR5** as proposed? *Default: yes, using report 01's criteria where they differ.*

## Sources (read 2026-10-22)

- egui 0.37.0 `RawInput`: https://docs.rs/egui/latest/egui/struct.RawInput.html — *documented*.
- CLAP `events.h` (`time` is the "sample offset within the buffer"; transport per block): https://raw.githubusercontent.com/free-audio/clap/main/include/clap/events.h — *documented*.
- Ableton Live 12 MIDI fact sheet (driver timestamps; latency against jitter): https://www.ableton.com/en/live-manual/12/midi-fact-sheet/ — *documented* (vendor). Old REAPER jitter test: https://www.synthtopia.com/?p=46149 — *anecdotal*.
- Discord limits 2,000 / 4,000: https://support.discord.com/hc/en-us/articles/33694251638295; Telegram text 1–4,096: https://core.telegram.org/bots/api (via https://docs.aiogram.dev/en/dev-3.x/api/methods/send_message.html) — *documented*.
- QR alphanumeric set and 4,296 characters at 40-L: https://www.keyence.com/ss/products/auto_id/codereader/basic_2d/qr.jsp — *documented*.
- Repo: `docs/{challenge,session,learn,editor}.md`, `docs/dsp/match.md`, `crates/enth-challenge/src/code.rs`, `crates/enth-plugin/src/game.rs`, `vendor/egui-baseview/src/window.rs`, `vendor/nice-plug/crates/nice-plug-core/src/{midi.rs,context/process.rs}`.
