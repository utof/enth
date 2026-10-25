# 06 — What S2 Depth does to enth's frozen contracts, and how to run it in waves

- **Sub-researcher:** 06 (architecture impact), 2026-10-25. Based on the codebase; no web sources.
- **Method:** the codebase-memory graph (`home-vboxuser-code-enth`, 70,053 nodes, ready), then direct reads of the files cited. Nothing was built or benchmarked.
- **Labels:** *measured* means a number recorded in the repo (a bench comment, a doc table) or a count I took. *Documented* means a doc or code comment says so. *Inferred* means my own reasoning. References are `path:line` at `1c99e80`.

## TL;DR

1. **The patch layer is ready for many modules; three narrow encodings are not.** A new `ModuleKind` is appended at the end, and the exhaustive `match` turns a missing implementation into a compile error. It changes no existing hash, golden, code or model. The narrow places:
   - **R5's answer code, layout 1:** at most 16 kinds (7 used), 15 parameters per kind, and 8 modules per section (`answer.rs:111-129,291-311`).
   - **Generator v1's pin** (`random/catalogue-v1.txt`).
   - **R3b's layout-1 hash** (`r3.md`, "The layout hash").

   The last two pin the item lists of `osc.wave`, `lfo.shape`, `svf.mode`, `noise.color` and `voice.mode`. *(documented)*
2. **Rule for S2: add kinds and neutral-default parameters; never append items to a pinned enum.** A ladder filter is a new kind, not `svf.mode = ladder`. An MSEG is a new kind, not an LFO shape. An optional one-time refactor (lane V1F) lifts the rule: generator v1 and layout 1 would read frozen copies of the specs.
3. **A new parameter on an existing kind rehashes every patch of that kind.** Both patch hashes include `values.len()` and every value's bits (`canon.rs:203-226`). This rekeys render caches. Codes, rounds and duels are safe: they use key-based fingerprints that skip default values (`answer.rs:68-72`, `random.rs:40-44`).
4. **Today's engine has a fixed topology. Hard sync is the only audio-rate link between modules.**
   - The plan "is just ordered lists" (`plan.rs:7`).
   - Generators run one at a time over a whole 32-frame segment (`engine.rs:1193-1218`).
   - The voice bus is mono (`module.rs`, `VoiceBus`).
   - `SyncBus` is the template to copy: a whole-segment buffer written by an earlier generator and read by a later one.
5. **Recommended routing:**
   - **Audio routes are ordinary `Route`s** from a generator to an `audio`-flagged carrier parameter (`pm`, `am`).
   - They run in topological order over whole segments, through per-generator buffers. Feedback happens only inside a module. Cycles between modules are refused.
   - The voice sum stays in patch order, so all 73 goldens stay bit-exact.
   - **Unison lives inside the oscillator** in `wide` f32x8 lanes. A per-plan stereo flag gives it spread.
6. **The CPU budget is the wall.** 16 voices × 7 unison × a table read (107.9 instructions per voice-sample) is about 12,100 instructions per frame for one oscillator, above the 12,000 cap. The G1 patch is already at 6,154. *(measured inputs, inferred product)* S2 needs SIMD unison from the first lane and a G2 reference patch with its own budget.
7. **The model survives S2a unchanged.** v0-b keeps loading, because nothing outside layout 1's entries moves its hash. Create keeps working: S2 sounds enter through **templates**.
   - Layout 2 must be a new id; the contract forbids growing layout 1.
   - Build layout 2 as **layout 1's 102 entries as a prefix, plus appended segments, on a template 1 that renders exactly like template 0 when the new modules are off.** Then ds1 is relabelled rather than re-rendered, and v0-b can warm-start the new head.
8. **S2a: about 34–57 agent-hours; critical path about 12–21.** Freezes F0d, F1d (judged) and F2d. Most lanes are **light** builds (dsp, engine, patch), so they fit beside R4/R5. Only four lanes need heavy slot S.
9. **Silent holes the compiler won't catch:** the structure matcher's kind families (`score/matching.rs:495-508`), so a new kind never pairs and scores 0; the coach's and deconstruct's aspect maps; the CLI aliases. `lesson-coverage` flags each new kind as a hole, and it turns blocking at GR2.
10. **Decide before R5a's friend build ships:** widen answer layout 1 now (`KIND`, `PARAM`, `PLACE` bits), or plan answer layout 2 inside S2. Layout 1 is frozen at F1e but not released (SKD merged in `ece5d5f`; no `docs/r5-gates.md`).

## 1. Dependency inventory

Classes:
- **A** = additive: new ids only, nothing old moves.
- **V** = versioned: a new id beside the old (generator v2, layout 2, answer layout 2, scorer `/2`, schema 2, eval version 4).
- **B** = breaking: silently changes an old result unless someone acts.

The columns are the four changes S2 makes:
- **+kind:** a new `ModuleKind`.
- **+param:** a neutral-default parameter on an existing kind.
- **+item:** an enum item appended to an existing parameter.
- **DSP Δ:** a numeric change to an existing module or to the engine loop.

| # | Dependent | Where | +kind | +param | +item | DSP Δ |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `ModuleKind`, `CATALOGUE` (one section per kind), catalogue snapshot | `enth-patch/src/catalogue/mod.rs:119-258`, `tests/snapshots/catalogue__catalogue.snap` | A, appended (discriminants are hashed, :119); snapshot review | A; the snapshot forces "a schema step, or say why none" (`migrate/mod.rs`) | A (items stored by name) | — |
| 2 | Patch file v1, migrations | `file/mod.rs:69` (`SCHEMA = 1`), `migrate/mod.rs` | A. Older builds refuse with `unknown_kind`; the plugin keeps the text and re-saves it unchanged (`session.md:157`) | A, because input may be sparse | A | none; bumps `ENGINE_VERSION` (0.3.0) |
| 3 | Section limits 4/4/4/8, 64 routes | `model.rs:40-52` (used only through `Section::max_len`; grep) | — | — | — | raising one is A for files; answer layout 1 caps it at 8 |
| 4 | Validation: a route source must be a modulator | `validate.rs:16` (`not_a_source`) | A | A | A | — |
| 5 | `exact_hash`, `canonical_hash` | `canon.rs:200-226` | A (patches without the kind keep their hash) | **B for keys**: every patch of that kind rehashes (render caches, `enth sound`'s `patch_hash`) | A | — |
| 6 | Canonicalisation: generators and modulators commute, sync exception, 1-WL colours over routes | `canon.rs:22-55,595-601` | A; a kind that breaks commutativity needs a check like sync's | A | A | — |
| 7 | Engine dispatch, roles, contexts | `enth-engine/src/modules/mod.rs:104-140`, `module.rs` | A (compile-enforced) | A | A | — |
| 8 | Segment loop: 10 fixed steps, patch-order sum, mono voice bus | `engine.rs:1-31,1101-1334` | A for leaf modules; routing changes the loop (§3) | — | — | **B**: every golden and replay unless unused features keep the old path bit-exact |
| 9 | Goldens: 73 cases × six targets, sparse patches | `tests/golden/cases/`, `golden.yml:21` | A | A if neutral | A | V (`golden --update`; the commit names the cases) |
| 10 | CPU budgets | `enth-engine/benches/*.rs`; G1 limit 650 M, cap 12,000 per frame (`g1.rs:25-43`) | A (a bench per kind; a G2 patch) | — | — | re-base on purpose |
| 11 | Host params: 72 permanent ids | `enth-plugin/src/params.rs:1-11` | A (modules are never host params) | A | A | — |
| 12 | Presets and plugin state | `presets.md`, `session.md:157` | A | A | A | — |
| 13 | Editor: cards from `ParamSpec`s; Add menu by section; layout 1 | `enth-ui/src/editor/`, `editor.md:127-150,231-252` | A in code (`present.rs:62` match is exhaustive); **no space left at 1200 × 800** | A | A | — |
| 14 | Generator v1, priors v1, observability mask | `random.rs:46-52`, `random/catalogue-v1.txt` (43 lines), `random/mask.rs:350-400` | A (v1 never draws it; the mask's exhaustive match needs an arm) | A for keys outside the pin | **V** for the 5 pinned enums; A elsewhere (`osc.table` is not pinned) | target audio moves, fingerprint does not |
| 15 | Challenge codes v1 | `code.rs:1-48`, `challenge.md:485` | A | A | V if pinned | another engine version → `engine_differs` warning only |
| 16 | R5 answer code and scorecard (`result-params-v1`) | `answer.rs:9-60,100-129,291-330` | A up to **16 kinds** (9 left) | A up to **15 parameters per kind** (osc 11, reverb 10) | A within the field width (`lfo.shape` 1 left, `osc.wave` 3) | re-scored by the receiver |
| 17 | `sound/1` | `score/sound.rs` | A | A | A | scores follow renders |
| 18 | `structure/1` and the module matcher | `score/structure.rs:1-40`, `score/matching.rs:495-508` | **B, silent:** fixed kind arrays, so a new kind never pairs (counts as left over, structure 0). Old v1 challenges are unaffected | A | A | — |
| 19 | Knob distance / JND | `priors.rs:121-147` | A (unit-based fallback, ×1.5) | A | A | — |
| 20 | Lessons, `lesson-coverage` | `enth-learn/src/coverage.rs` (`Group::all()` = every kind), `lessons.yml:16,96-98` | **a hole per kind** until a lesson teaches it; report-only until GR2 | Basic-tier parameters are holes | — | every lesson re-runs |
| 21 | Coach hints, deconstruct | `enth-solver/src/coach.rs:269-276`, `enth-learn/src/deconstruct/build.rs:450-468` | B, silent (no aspect maps to the kind) | — | — | — |
| 22 | R4 ear games | `enth-challenge/src/ear/params.rs:1-30` (template 0) | A | A | A if not pinned | stimuli re-render; answer keys unchanged (`challenge.md:322`) |
| 23 | R3a create: free set, templates, k-NN bank | `create/mod.rs:329-365,385-389`, `nearest.rs:1-50` (`ENTHBANK` stores v1 specs) | A (searched only when a start contains the kind) | A | A | bank embeddings drift (heuristic) |
| 24 | R3b: feature set 1, layout 1, `ENTHMODL` 1 | `r3.md:840-1011`, `propose/decode.rs:84-96`, `layout-v1.txt` | A ("still load") | A outside layout entries | **V** (`model_mismatch`) for the 5 pinned enums | refit later |
| 25 | CLI `schema/modules/params`, aliases | `enth-cli/src/schema/mod.rs:31-35`, `aliases.rs:97+` | A (snapshots update; ids stay `enth.modules/1`, `enth.params/2`) | A | A | — |
| 26 | Agent evals (30 goals, eval version 3) | `content/agent-evals/`, `agent-evals.md:10` | A (new goals → version 4) | A | A | witnesses re-checked |
| 27 | `ModSource` (append-only) | `addr.rs:421-432`; answer `SOURCE_BITS` 3 | A up to 3 new built-in sources | — | — | — |

**Safe today:** new kinds; neutral parameters on keys outside the pin; items on unpinned enums, within the answer field widths; new golden cases, templates and lessons.

**Needs a version:** any field generator v1 or layout 1 reads; more than 16 kinds; more than 15 parameters on one kind; more than 8 modules in a section; a kind that lives in two sections (the answer decoder places an added module by `kind.section()`, `answer.rs:1074`).

## 2. What each S2 candidate touches

| Candidate | Home | Contracts touched |
| --- | --- | --- |
| FM/PM/AM/RM | audio routes (§3.1) into a new oscillator kind | validation, engine order, the canonicalisation check, a new `audio` flag (the pin's flag column stays 4 characters) |
| Unison | inside the new oscillator; stereo flag | CPU; the 15-parameter cap if added to `osc` |
| Wavetable warp | the new oscillator | none (not `osc.wave`, which is pinned) |
| Distortion | new `voice_fx` kind (before the filter, for growls) | zero latency (ADR-0006) |
| Chorus/flanger, phaser, delay | new `fx` kinds, `time_based` | crossfade swaps copy their buffers (R9) |
| EQ | `fx` kind wrapping `enth-dsp/src/biquad.rs` | none |
| OTT / multiband | one `fx` kind | CPU |
| Comb, formant, ladder | new `voice_fx` kinds | `svf.mode` is pinned |
| MSEG | new modulator kind, **S2b** | 5 points × 3 values already reach the 15-parameter cap |
| Random/S&H | exists as LFO shapes (`catalogue/lfo.rs:25-28`) | — |
| Envelope follower | modulator reading the previous segment's bus, S2b | additive context field |
| Tempo sync | `lfo` gains `sync` and `beats` (7 → 9 parameters) | **no tempo input today**: `Performance` holds notes, automation, controls and duration only |
| Serial chains in any order | exist (`voice_fx`, `fx`, `MoveModule`) | true lanes are design.md §2's schema-2 migration, later |
| Arp / sequencer | later | needs transport position |

## 3. Routing architecture

### 3.1 FM/PM/AM/RM between generators

| Option | Verdict |
| --- | --- |
| (a) A per-sample graph over all modules | Rejected: about 32× the dispatch cost, and it breaks "one call per module per segment" (`module.rs:15-17`) |
| (b) N-operator FM inside one kind | Fine for DX-style FM, but cannot do "FM wavetable A from B" |
| (c) **Audio routes as `Route`s:** source a generator, destination `<carrier>.pm` or `.am`, depth sets the index | **Recommended** |
| (d) A new `input` field on `ModuleInstance` | Rejected: changes the patch shape, the hash preimage, the answer format and the file |

Why (c):
- Routes already have ops with inverses, undo, editor drag and canonical sorting.
- The answer code already encodes them as `InsertRoute`: a module source plus a target and parameter (`answer.rs:31-50`).
- Layout 1's route grid lists no generator source.
- Older builds refuse such a patch cleanly with `not_a_source`.

**The F1d contract:**
1. **Destinations.** An `audio` parameter accepts only generator sources, and a generator source may target only `audio` parameters. These routes stay out of the control-rate matrix.
2. **Order.** Generators are sorted topologically by audio routes, with ties kept in patch order (stable), so a patch without audio routes runs exactly as today. A cycle between modules is `audio_cycle`. Hard sync keeps its rule.
3. **Buffers.** Each generator writes its **pre-level** output into its own segment buffer (4 × 32 voices × 32 frames × 4 B = 16 KB, allocated in `compile`). A carrier reads it per frame, scaled by the route depth.
   - Pre-level means a modulator at level 0 is not heard but still modulates.
   - **The voice sum stays in patch order.** That keeps every golden bit-exact.
4. **Feedback** is a parameter inside the module: a one-sample delay with two-sample averaging. There is no cross-module feedback: a one-segment delay (0.67 ms) would comb-filter the result.
5. **Carriers.** PM works on phase-read waves (sine and table, `Phased`, `docs/dsp/osc.md:33`). The polyBLEP shapes assume a known increment per sample, so PM on them aliases. Either mark PM dead on those shapes or oversample; report 05 picks.
6. **Masks.** The rule "a silent generator hides all but its level" (`mask.rs:351-357`) and the matcher's "silent ≡ absent" gain the clause "unless an audio route leaves it". v1 inputs have no audio routes, so v1 is unchanged.

AM and RM use the same route into an `am` parameter (bipolar mode = RM). Keep to at most two `audio` parameters per carrier because of the 15-parameter cap.

### 3.2 Unison

- **Inside the generator, not the voice manager.** There are 32 slots for polyphony 16 (`voice.rs:7-12`). Engine-voice unison would also change steals, `NOTE_END` and seeds.
- **Seeds.** Sub-voice `k`'s phase and jitter come from `derive(voice_seed, k)`, never from parameter values.
- **SIMD.** `wide` f32x8 lanes with no fused ops, then a horizontal sum in a fixed order. Phase 0 measured these lanes bit-identical to scalar code and 1.8–1.95× cheaper at 16 voices (`design.md` §3).
- **Stereo spread needs a stereo voice path.** Proposed: a per-plan `stereo` flag.
  - When it is set, the voice bus is L/R and each `voice_fx` module is instantiated twice (one parameter span, one module key), so the module API does not change.
  - Pan becomes a balance law.
  - When it is clear, today's mono path runs untouched. Switching it is a recompile.
  - Cost: the voice effects run twice (svf 28.4 → about 57 instructions per voice-sample, `docs/dsp/svf.md:126-131`).

Instructions per frame on the scalar path, inferred from measured per-voice costs (a table read is 107.9, `docs/dsp/osc.md:116-124`):

| Patch | Instructions per frame |
| --- | --- |
| G1 today, 16 voices (measured, `g1.rs:38-43`) | 6,154 |
| plus a table oscillator with 7-voice unison, 16 voices | about 18,200 (over the 12,000 cap) |
| the same with f32x8 unison | about 12,500 |
| 8 voices: 7-voice table unison, stereo svf, 2× distortion (about 100 per voice-sample, a guess), reverb, chorus, OTT, EQ | about 9,000–11,500 |

So S2 needs SIMD unison from the first lane, and a **G2 reference patch** whose voice count and budget the S2 summary sets. Bass music mostly plays mono, so 8 voices may be fair.

### 3.3 Chains, follower, tempo

- **Reorderable chains exist already.** Raise `MAX_FX` to 8 only if needed: 8 is the most `PLACE_BITS` allows, and older builds refuse more than 4 (`too_many`). Phase Plant lanes are schema 2: leave them for later.
- **Follower.** Modulators run before the voices (`engine.rs:13-19`), so a follower reads the previous segment's bus. That is a deterministic 0.67 ms delay, harmless for a follower. The contexts gain a read-only bus. There is no external sidechain (a synth has no second input).
- **Tempo.**
  - Add `Performance::tempo_bpm` (default 120) and a tempo event applied at segment boundaries.
  - The plugin maps nice-plug's `Transport::tempo`, which is per block (`vendor/nice-plug/crates/nice-plug-core/src/context/process.rs:145`). The CLI takes `--bpm`.
  - Synced rates are stored in beats. Challenge phrases assume 120.
  - "CLI = plugin" gains a condition: the same tempo events.
  - Transport position (arp, bar-locked phase) is a separate, later decision.

### 3.4 Freezes for S2

| Freeze | Content | Gate |
| --- | --- | --- |
| **F0d** | S0d appends every S2a kind as a stub, with **discriminants fixed now** so merge order never sets a persisted number. Also: the arms in every exhaustive match, bench files, golden folders, `docs/dsp` stubs, `Performance::tempo_bpm`, the `audio` flag, and a `ModuleSpec` flag that hides reserved kinds from the Add menu, `enth modules` and the generator | gate green; **every golden, fixture, generator golden and `layout-v1.txt` unchanged**; G1 within budget |
| **F1d** (Opus judge) | The S2a catalogue (keys, units, ranges, scales, defaults, sections, flags); the audio-route contract; the stereo flag; tempo; context additions; answer widening or layout 2; "no items on pinned enums" written into `catalogue/mod.rs` | v1 pin and layout hash unchanged; 73 goldens bit-exact; mono patches through the stereo-capable loop bit-exact; parameter counts within the answer cap |
| **F2d** | `wt` with PM from another generator and unison, end to end (engine, six targets, CLI, card, `host-render`, `cli-equal`, REAPER) | G2 measured; G1 unchanged |
| F3d (S2b) | generator v2, template 1, layout 2: the data freeze for the next model | the v1 golden unchanged; layout 2's prefix equals layout 1 |

## 4. The model

- **Today.** Layout 1's canonical text covers only its own entries, their specs, template 0's values and the label rules. "A new parameter, enum item, module kind or route source anywhere else → still load" (`r3.md`, "The layout hash"). So S2a leaves v0-b loading, as long as it touches neither the pinned enums, nor a layout entry's range, scale or unit, nor template 0. Template 0 is a frozen copy, `random/starter-v1.patch.json`.
- **Layout 1 cannot grow.** "More parameters" is listed under layout 2 (`r3.md:981`). Only outputs and info keys grow additively. Format 1 needs no bump: `OutputLayout::build(layout, generator, template)` already takes an id (`decode.rs:654`).
- **What layout 2 needs:**
  - generator v2 (or a `ds2` sampler);
  - **template 1**: template 0 plus the S2 modules, switched off;
  - mask rules for the new kinds;
  - label rules for the new segments: unison as categorical, detune, spread and FM index as continuous, an audio-route cell from osc2 to osc1, effect enables;
  - canonical-slot rules for the new symmetries.
- **Make it a strict superset:** layout 1's 102 entries first, then appended segments, on a template 1 that renders bit-identically to template 0 when its new modules are off. Disabled modules are left out of the plan (`plan.rs:27`), and the stereo flag and audio routes are off. *(inferred)* This buys three things:
  - ds1's 2 M clips stay valid layout-2 samples, with the new structure set to "off" and the new values masked. Only labels need re-encoding (a Rust pass over regenerated v1 specs), and only **new** renders are needed, not another 3 h × 2 M.
  - v0-b's trunk and its first 102 columns per hypothesis can initialise a widened head. That is for the warm-start researcher (01/02) to measure.
  - Decoding code is shared.
- **Create on targets that need unknown modules: no masking.**
  - The model "can only add candidates": the heuristic start is always kept, round 0 ranks by rendered distance, and the model's share of branches is capped (lane EVM).
  - S2 sounds enter through templates (`create/mod.rs:385-389`), and the free set searches whatever a start contains (`:329-365`). So S2a adds growl, reese and wobble templates.
  - `enth.create/1` records "model layout 1: P1 modules only" (additive).
  - The k-NN bank stays v1-space until a generator-v2 bank.
- **Churn.** A layout costs a full iteration (5–8 h plus renders), so batch: **one layout per S2 wave**, never one per module.

## 5. S2a lane plan

**Goal:** the modules for wobble, growl, reese and FM bass, with no existing hash, golden or code moved.

**Slots:**
- R4/R5 keep heavy slot L (phase-4.md).
- R3b's local heavy lanes are done and its training runs on the remote GPUs, so **S2a takes slot S**.
- At most 4 light lanes at once, **shared with R4/R5**; freeze-path lanes go first.
- dsp, engine, patch and challenge are light builds; editor and plugin are heavy.

**Spine**

| Lane | Owns | Depends on | Size (agent-h) | H/L |
| --- | --- | --- | --- | --- |
| **DOCS-S2** | the summary's doc rows; `docs/phase-s2.md` | the summary | 0.75–1 | L |
| **S0d** (→ F0d) | the reserved kinds and stubs (`catalogue/<kind>.rs`, `modules/<kind>.rs`), the arms in `mask.rs`, `present.rs`, `rate()`; the hide flag; benches; golden folders; `docs/dsp` stubs; `tempo_bpm`; the `audio` flag | DOCS-S2; no root-manifest edit in flight | 1–1.5 | **H** once |
| **RTE** routing (→ F1d) | `validate.rs`, the `canon.rs` check, `plan.rs` (topology, per-generator buffers), `engine.rs` step 7, `module.rs` (`VoiceCx::audio_in`), `docs/dsp/engine.md` | F0d | 2–3.5 | L |
| **SVP** stereo voice path (→ F1d) | the `stereo` flag, duplicated `voice_fx`, the balance law; noalloc and swap-carry tests | F0d | 1.5–3 | L |
| **CAT** catalogue (→ F1d) | every S2a `ParamSpec` table; the answer-table rows **through RC/the coordinator** (`answer.rs` is R5's); the pinned-enum rule | F0d; reports 03–05 | 1–2 | L |
| **TMP** tempo (→ F1d) | `Performance::tempo_bpm`, the tempo event, `--bpm`, `lfo` `sync`/`beats` | F0d | 0.75–1.5 | L |
| **V1F** (optional) | generator v1 and layout 1 on frozen spec copies | F0d | 1–2 | L |
| **F1D** | the judge's must-fixes | RTE, SVP, CAT, TMP | 0.5–1.5 | L, Opus |

**Modules.** Each lane delivers an `enth-dsp` core, an engine module, golden cases on six targets, a gungraun budget, `docs/dsp/<kind>.md` and notices for any port.

| Lane | Kind(s) | Section | Depends on | Size | H/L |
| --- | --- | --- | --- | --- | --- |
| **WT** (→ F2d) | `wt`: tables, warp, `pm`/`am`, `feedback`, f32x8 unison; at most 15 parameters | generators | F1d | 3–5 | L |
| **DST** | `dist`: ADAA, zero-latency 2×/4× oversampling | voice_fx | F1d | 1.5–2.5 | L |
| **MOD** | `chorus` (with a flanger mode), `phaser` | fx | F1d | 2–3 | L |
| **DLY** | `delay` (synced) | fx | F1d, TMP | 1–2 | L |
| **EQ** | `eq` on the biquad | fx | F1d | 1–1.5 | L |
| **OTT** | `ott` (3 bands) | fx | F1d | 2–3 | L |
| **FLT** | `ladder`, `comb`, `formant` | voice_fx | F1d | 2–4 | L |

**Integration**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **SKS** (→ F2d) | `wt` and an audio route through plugin and editor; `host-render`, `cli-equal`; REAPER | WT | 1.5–2.5 | **H** |
| **MTC** | the matcher's kind families, the coach, deconstruct, aliases | F1d | 1–1.5 | L |
| **G2B** | the G2 patch, its budget, the real-time row and swap blocks | WT, DST, OTT | 1–1.5 | L |
| **EDS** | dense cards (a focus region or a second layout), audio-route drag; kittest, overflow, `leak-test` | F2d | 2–4 | **H** |
| **TPL** | create templates: growl, reese, wobble, FM bass | WT, DST, FLT | 1–2 | L |
| **LS2** | a lesson per user-facing kind | each kind | 4–8 | L, Opus |
| **EV5** | agent-eval families (eval version 4) | SKS | 1–1.5 | L |
| **INT-S2a** | full gate, owner sheet, friend build | everything | 2–3 | **H** |

**Waves**

| Wave | Slot S | Light | Starts when |
| --- | --- | --- | --- |
| 0 | — | DOCS-S2 | the summary is sent |
| 1 | S0d (→ F0d) | — | DOCS-S2 merged |
| 2 | — | RTE, CAT, SVP, TMP (→ F1d); V1F if chosen | F0d |
| 3 | — | WT first; then DST, FLT, MOD; MTC | F1d |
| 4 | SKS (→ F2d) | DLY, EQ, OTT, G2B | WT |
| 5 | EDS | TPL, LS2, EV5 | F2d |
| 6 | INT-S2a, then listening and a friend build | LS2 | everything merged |

**Totals:** spine about 8–14, modules 12.5–21, integration 13.5–23.5: **about 34–57 agent-hours**.

**Critical path:** S0d (1–1.5) → RTE + CAT (2–3.5) → F1D (0.5–1.5) → WT (3–5) → SKS (1.5–2.5) → EDS (2–4) → INT-S2a (2–3), **about 12–21**. R2 took about twice its plan in wall time, so expect **about 25–45 h of wall time**.

**Acceptance**

| Lane | Acceptance |
| --- | --- |
| S0d | Every golden, fixture, v1 golden and layout hash unchanged; reserved kinds invisible |
| RTE | No-audio-route patches bit-exact (73 goldens); non-`audio` destinations and cycles refused; apply/inverse proptest with audio routes; noalloc over a swap that adds one |
| SVP | Mono bit-exact; stereo with spread 0 equals mono on both channels; swap carry; noalloc |
| CAT | `const` checks; at most 15 parameters (or the widened cap); every new parameter round-trips in an answer code |
| TMP | Bit-exact at host blocks 1, 64, 100, 4096 with a tempo change |
| Module lanes | Same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; budget + 10 %; **aliasing measured spectrally** (a hash cannot see aliasing) |
| SKS | The F2d row; CLI = plugin for a `wt` golden |
| MTC | Each new kind pairs in `structure/1`; v1 golden challenges unchanged |
| G2B | G2 within the summary's budget; p99 under 25 % of the 64-frame deadline with edits every 250 ms |
| EDS | Nothing scrolls at 1200 × 800 for G2 in the chosen layout; 0 idle frames; `leak-test` |
| TPL | Each template beats `init` on its genre's dev targets in create |
| LS2 | `lesson-check --cross --solve` green; no coverage hole for S2a kinds |
| INT-S2a | Full gate; S2a gate table; owner sheet |

## 6. Risks

| # | Risk | Evidence | Mitigation |
| --- | --- | --- | --- |
| R1 | CPU blow-up | §3.2 (inferred) | SIMD unison; G2 budget; a voice-count rule |
| R2 | Aliasing at high FM index; PM on polyBLEP | `docs/dsp/osc.md:18,33` | PM on phase-read waves only, or oversampling; spectral tests |
| R3 | Determinism of new DSP (shapers, oversampling filters, lane sums, denormals in feedback) | AGENTS.md DSP rules | `libm` or polynomial shapers; fixed sum order; flush per write; six-target goldens |
| R4 | Answer code out of room | `answer.rs:111-129` | TL;DR 10 |
| R5 | A "harmless" enum item trips v1 and layout 1 | `catalogue-v1.txt`, `r3.md` | The rule; optional V1F |
| R6 | Silent scoring holes | `matching.rs:495-508` | MTC in wave 3 |
| R7 | No editor space | `editor.md:139-150` | EDS (focus region or second layout) |
| R8 | Lesson debt; coverage blocking at GR2 | `lessons.yml:16` | LS2; keep kinds hidden until their lesson |
| R9 | A crossfade swap **copies** effect state; only instant swaps move it. The P1b p99 swap block was 797 µs with the reverb's 1.6 MB copied, against a 1,333 µs deadline (`design.md` §3). Delay and chorus buffers add to that | design.md §3 | Minimal buffers; measure G2 swaps in G2B |
| R10 | Model churn | 5–8 h per iteration | One layout per wave; superset layout |
| R11 | Hash churn from parameters on `osc`/`lfo` | `canon.rs:203-226` | Prefer new kinds; accept `lfo`'s tempo rekey |
| R12 | Tempo adds a "CLI = plugin" condition | design.md §5 | `host-render` cases with a tempo change |
| R13 | Discriminant races between lanes | `catalogue/mod.rs:119` | S0d reserves them all at once |
| R14 | Distortion wanted both per voice and global | `answer.rs:1074` | Two kinds, not a multi-section kind |

## 7. Gaps

- Nothing was built or benchmarked. §3.2's numbers are inferred from recorded per-voice costs. G1's numbers predate later skeleton savings (`docs/dsp/svf.md:133`).
- I did not run `check_index_coverage` per path. `index_status` shows one partial file (HTML under `_local/`). The negative claims ("no tempo", "limits used only in `model.rs`") rest on grep and the graph.
- Consumers I did not read in depth: `enth-ui` cards beyond the name match, `enth-solver` `hint/space.rs` and `search.rs`, the R4 drill player. Their classes are inferred from their `ModuleKind` uses.
- 1-WL canonicalisation with generator-sourced routes is reasoned from `canon.rs`, not tested.
- How audio-route depth events pass through the session's `apply::route` is not traced.
- Widening the answer code crosses R5's F1e freeze: the owner's and lane RC's call.
- Kind names are working names. The candidate list and the DSP choices belong to reports 03–05.

## 8. Sources

- Docs: `AGENTS.md`, `docs/{README,architecture,design,research-process,roadmap,phase-4,r3,editor,session,challenge,presets,agent-evals}.md`, `docs/adr/0005-*`, `docs/adr/0006-*`, `docs/dsp/{osc,svf,reverb,engine}.md`.
- Code: `crates/enth-patch/src/{catalogue/*, model.rs, addr.rs, canon.rs, validate.rs, migrate/mod.rs, file/mod.rs}`; `crates/enth-engine/src/{module.rs, modules/*, engine.rs, plan.rs, voice.rs, lib.rs}`, `benches/*`; `crates/enth-challenge/src/{random.rs, random/*, priors.rs, code.rs, answer.rs, score/*, ear/params.rs}`; `crates/enth-solver/src/{create/mod.rs, nearest.rs, coach.rs, propose/decode.rs}`; `crates/enth-learn/src/{coverage.rs, deconstruct/build.rs}`; `crates/enth-plugin/src/params.rs`; `crates/enth-cli/src/{schema/mod.rs, aliases.rs}`; `crates/enth-render/src` (`Performance`); `tests/golden/cases/`; `.github/workflows/{golden,lessons,engine-golden}.yml`; `ml/enth_ml/export/enthmodl.py`; `vendor/nice-plug/crates/nice-plug-core/src/context/process.rs`.
