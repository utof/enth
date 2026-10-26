# Phase S2 — S2a: Depth for bass music

**Goal: modules for wobble, growl, reese and FM bass, with no existing hash, golden or code moved.** S2 (Depth) is pulled forward and re-aimed at bass music. A dubstep sound is a mid-range bass whose timbre moves in tempo-locked patterns over a clean mono sub, and enth makes about 3 of the 14 sound families today (sub, basic wobble, two-saw reese, sync screech). S2a adds, in this order, tempo and distortion; FM, PM, AM and ring mod between generators in a new wavetable oscillator `wt`; OTT, comb and formant filters and an MSEG; unison on SIMD, chorus, flanger, phaser, EQ and a synced delay; then templates, lessons and a friend build. Its gate GS2a ([below](#the-gs2a-checks)): nothing that exists moves, a G2 reference patch stays within its CPU budget, every module's aliasing is measured, [report 04](../_local/sessions/2026-10-25-s2-depth-and-fast-training/04-dubstep-anatomy.md)'s ten patch sketches are matched in create, and you make a wobble, a growl, a reese and an FM bass you like. **S2a takes heavy slot S** (R3b's local heavy lanes are done and its training runs on the remote GPUs), beside R4 and R5 in slot L ([phase-4.md](phase-4.md)).

Size: **about 35–63 agent-hours** (report 06 §5's 34–57 plus the unison split and the MSEG lane; its own rows add to 34–59). Critical path: S0d → RTE + CAT → F1M → WT → SKS → EDS → INT-S2a, about 11–20 agent-hours; R2 took about twice its plan in wall time, so expect **about 22–40 h of wall time**. Most lanes are light builds (dsp, engine, patch), so they fit beside R4 and R5; only S0d, SKS, EDS and INT-S2a are heavy.

**Status (2026-10-25): planned; nothing is built.** The research session is done, and its decisions are provisional until you steer; work proceeds on the defaults meanwhile: [`_local/sessions/2026-10-25-s2-depth-and-fast-training/`](../_local/sessions/2026-10-25-s2-depth-and-fast-training/summary.md) (the summary wins where it corrects a report; [report 06 §5](../_local/sessions/2026-10-25-s2-depth-and-fast-training/06-architecture-impact.md#5-s2a-lane-plan) is the lane plan this page adapts). [roadmap.md](roadmap.md) and [README.md](README.md) were updated the same day (lane DOCS-S2), and [phase-4.md](phase-4.md)'s open checks gained one row on the answer code. The faster R3b training loop (lane FTL) is separate: it updates [r3.md](r3.md) and `ml/README.md` when it lands, S2a does not wait for it, and **no full training run starts before layout 2** ([S2b](#s2b-outline)).

New in S2a: reserved module kinds in `enth-patch`'s catalogue, their `enth-dsp` cores and `enth-engine` modules; **three changes to the engine's fixed shape** (per-generator audio buffers in a topological order for FM, AM and RM; a stereo voice bus for unison spread; tempo as a render input); `wide` (the first SIMD dependency); and content in `enth-learn`, `enth-solver`, `enth-ui`, `enth-plugin` and `enth-cli`. No new crate, so no layer change. Nothing that exists changes: no hash, golden, challenge code, fixture or model, and ADR-0006 stands.

## What the research changed

Lanes should read these before the tables.

**What the synth needs**

- **Phase Plant has no sequencer and no arpeggiator** (its nearest features are the Trance Gate, stepped drawing in its curve and LFO editors, and S&H or random modulators); Serum 2 and Pigments 7 have both, Vital and Surge XT neither. S2 keeps both out because the DAW supplies MIDI; a synced step gate or a volume MSEG covers the rhythm (D4).
- **All five reference synths share a tier A core and enth has 7 of its parts.** Dubstep is that core used hard. The movement is a tempo-synced, often drawn LFO or MSEG on a filter (the wobble), on the source itself (FM index, wavetable position) or on serial distortion followed by OTT; formant, comb and notch filters add the vowels and the metal.
- **"Resampling" is not a module.** In a synth it is serial chains of distortion and filtering with dry and wet blends, so every effect gets `mix` and a chain takes several instances of one kind.
- **Coverage is judgment, not measured:** report 04 puts it at about 20 % of the 14 families today and about 25, 45, 60, 68, 80 and 90 % after tempo, distortion, PM, OTT, comb plus formant, and unison plus a modulation effect. Lane TPL checks it by matching 04's ten patch sketches after each module lands.

**What it costs**

- A new `ModuleKind` changes no existing hash, golden, code or model, and a missing implementation is a compile error. The narrow spots are generator v1's pin and R3b's layout-1 hash (both pin five enums, below). R5's answer code was one too (16 kinds, 15 parameters per kind, 8 modules per section) until D3 widened it on 2026-10-26 to 64 kinds, 63 parameters per owner (indices 0–62) and 16 modules per section, with compile-time asserts in `answer.rs` that fail the build when the catalogue or the patch model outgrows them.
- **CPU is the wall.** 16 voices × 7-voice unison is about 4,700 instructions per frame on saws and about 12,100 on wavetables, against G1's 6,154–6,274 of 12,000. So unison uses `wide` lanes from its first line, and S2a gets a G2 reference patch with its own budget.
- **Latency stays zero** (D10): no lookahead limiter, IIR oversampling instead of FIR, a through-zero flanger with a fixed dry delay.
- **Every effect has a verified permissive source** (Mutable Instruments, Signalsmith, DaisySP, Faust per function, Dexed's MSFA files per file); Vital, Surge XT and chowdsp's GPL modules are study only ([research.md](research.md), [ADR-0004](adr/0004-licensing.md)).

## The S2 rules (D2)

1. **New kinds and neutral-default parameters only; never items on a pinned enum.** Generator v1 (`random/catalogue-v1.txt`) and R3b's layout 1 ([r3.md](r3.md#the-layout-hash-the-catalogue-hash)) pin the items of these five, so a ladder filter is a new kind and not `svf.mode = ladder`, an MSEG is a new kind and not an LFO shape, and ring mod is not an `osc.wave` item:

   | Pinned enum | Items |
   | --- | --- |
   | `osc.wave` | sine, triangle, saw, pulse, table |
   | `lfo.shape` | sine, triangle, saw_up, saw_down, square, sample_hold, smooth_random |
   | `svf.mode` | lp, bp, hp, notch |
   | `noise.color` | white, pink |
   | `voice.mode` | poly, mono, legato |

   `osc.table`, `osc.phase_mode` and `voice.glide_mode` are not pinned.
2. **Prefer a new kind to a parameter on an old one.** A new parameter on an existing kind rehashes every patch of that kind (render caches and `enth sound`'s `patch_hash`; codes, rounds and duels use key-based fingerprints that skip defaults and are safe). So parameters added to an existing kind (`lfo`'s `sync` and `beats`, and whatever CAT decides for envelope times) go in **once, at F1m**. Nothing may change a field layout 1 reads (a range, scale, unit, item list or template-0 value of one of its 102 entries).
3. **Every effect gets `mix`** (at 0 it returns its input bit for bit), **and the voice-FX and FX chains take several instances of one kind** (two `dist` modules keep separate state, seeds and swap carry; each lane tests it). `MAX_VOICE_FX` and `MAX_FX` stay 4 unless the sketches need more ([open checks](#open-checks)).
4. **A kind lives in one section.** A distortion wanted both per voice and global is two kinds.
5. **The determinism rules of [AGENTS.md](../AGENTS.md) apply to every new line.** `libm` or polynomial shapers, a fixed summation order, no fused operations, flush denormals in code, seeds from the voice seed and the module id (a unison copy's phase comes from `derive(voice_seed, k)`, never from a parameter value), and **`wide`'s traps are banned** (`recip`, `recip_sqrt`, `exp`, `exp2`, `ln`, `sin_cos`, `pow`, `exp_m1`, `ln_1p`) beside the fused operations already in `clippy.toml`.
6. **Reserved kinds stay hidden** (S0d's flag: the Add menu, `enth modules` and the generator never list them) until INT-S2a clears it per kind, so `lesson-coverage`, which turns blocking at GR2, never meets a kind without a lesson.
7. **A module lane delivers** an `enth-dsp` core, an engine module, golden cases on six targets, a gungraun budget, `docs/dsp/<kind>.md` and a `THIRD_PARTY_NOTICES.md` entry per port ([AGENTS.md](../AGENTS.md#architecture-non-negotiable)); its lesson and agent eval come from LS2 and EV5.

## How the conflicts were resolved

The summary's corrections stand ([summary](../_local/sessions/2026-10-25-s2-depth-and-fast-training/summary.md#corrections-to-sub-reports)). This plan settles the rest, provisionally:

| Conflict | Resolution | By |
| --- | --- | --- |
| Report 03 puts unison first; 04 and 05 put distortion first | **Distortion and tempo first.** Unison needs the stereo bus and SIMD, and alone adds width more than new timbres, so it lands after the PM routing | The summary |
| Report 06's S2a has no MSEG; 04 calls it the biggest single dubstep modulator | **Lane MSEG**, a new kind with a fixed point count that CAT decides (variable-length data fits neither the catalogue nor a model layout) | The summary |
| 04 asks for envelope times above 10 s | An existing range cannot move (the catalogue's normalisation and the layout-1 hash), so **a new parameter or a new envelope kind**, decided by CAT, built by lane MSEG | The summary; this plan |
| Report 06 makes `wt` with PM and unison the F2m skeleton | `wt` with PM is F2m; **lane UNI lands unison mono-first after PM**, on `wide` lanes, so F2m never waits on SIMD. F2m's gate loses "G2 measured"; G2B measures it once unison and OTT exist | The summary; this plan |
| 06 puts tempo's default at 120 and the summary (D5) at 140 | **140**: only a synced module reads it and no patch before S2 has one. Code v1 is frozen, so tempo enters a challenge's render spec with the first version that can draw a synced module (generator v2, S2b) | The summary; this plan |
| The summary names SVP or WT for `wide`'s bans | **S0d pins `wide` and writes the bans** (root files are S0d's, [hotspots](#rules-for-every-lane)); UNI, `wide`'s first user, owns the six-target golden | This plan |
| Report 06's waves run MOD before OTT and have no MSEG or UNI lane of their own | D1's order: DST and WT, then OTT, FLT and MSEG, then UNI, MOD, EQ and DLY | The summary; this plan |
| 04's coverage percentages | Judgment; TPL and GS2a replace them with a count of the ten sketches matched | The summary |
| phase-4.md already has an **F2d** (R5a's duel skeleton) and a lane **G2** (the chord and rhythm ladders) | S2's freezes take an **m** (modules): F0m, F1m, F2m, F3m, and the judge-fix lane is F1M, so no freeze name means two things. "G2" here is the reference patch (after G1's); the ladder lane keeps its name in phase-4.md | This plan |
| The answer code has 9 free kinds and S2a reserves 11 | D3, [below](#the-answer-code-r5a-d3): decided 2026-10-26, widened | This plan |

## Freezes

After **F1m** the S2a catalogue (keys, units, ranges, scales, defaults, sections, flags) changes only through the coordinator, who names the lanes it moves; after **F2m** the routing contract too. Discriminants are fixed at **F0m**, so merge order never sets a persisted number.

| Freeze | What is merged | Gate before the fan-out |
| --- | --- | --- |
| **F0m** | S0d: every S2a kind appended as a hidden stub with its discriminant fixed, the arms in every exhaustive match, the hide flag, the `audio` parameter flag, `Performance::tempo_bpm`, the `wide` pin and bans, the aliasing harness, bench files, golden folders and `docs/dsp` stubs | The Done-means gate is green ([AGENTS.md](../AGENTS.md#done-means)), judged by exit status; **every golden, fixture, generator-v1 golden and `layout-v1.txt` unchanged, v0-b still loads**; G1 within budget |
| **F1m** | RTE, SVP, CAT and TMP: the S2a catalogue, the audio-route contract, the stereo flag, tempo and the context additions, "no items on pinned enums" written into `catalogue/mod.rs`, and D3's answer or the table rows it needs | **A judge agent (Opus)** reviews them together, because every persisted key goes to friends. Named checks: the v1 pin and layout hash unchanged; 73 goldens bit-exact; mono patches through the stereo-capable loop bit-exact; kind and parameter counts within the answer cap, or the widened one; the six points of the audio-route contract ([06 §3.1](../_local/sessions/2026-10-25-s2-depth-and-fast-training/06-architecture-impact.md#31-fmpmamrm-between-generators)); tempo's place in the render spec. Then F1M closes the must-fix items |
| **F2m** | SKS: `wt` with PM from another generator, end to end (engine, six targets, CLI, card, `host-render`, `cli-equal`, REAPER) | `cli-equal` on a `wt` golden; the REAPER `wt` scenario; `validate`, `host-render`, `leak-test`; G1 unchanged; `wt`'s budget recorded |
| **F3m** (S2b) | Generator v2, template 1, layout 2: the data freeze for the next model | The v1 golden unchanged; layout 2's first 102 entries equal layout 1's; template 1 with the new modules off renders bit-identically to template 0 |

## Lanes

Sizes are agent-hours (report 06 §5's; sizes this plan changed or added are marked). H = heavy local build (nice-plug, egui or a vendored crate), L = light. Kind names are working names until F0m merges and persisted after it.

**The spine**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **DOCS-S2** Docs | This page; the summary's doc rows in roadmap.md and README.md | The summary | 0.75–1 | L |
| **S0d** Workspace prep (→ F0m) | The reserved kinds and stubs (`catalogue/<kind>.rs`, `modules/<kind>.rs`, the arms in `mask.rs`, `present.rs` and `rate()`); the hide flag; the `audio` flag; `Performance::tempo_bpm`; one seam file each for RTE and SVP, so they edit different files; `benches/<kind>.rs`, golden folders and `docs/dsp/<kind>.md` stubs; the spectral aliasing harness the module lanes share; the `wide` pin in `[workspace.dependencies]` and its bans in `clippy.toml` | DOCS-S2; no root-manifest edit in flight | 1–2 (this plan adds up to 0.5 for `wide` and the harness) | **H** once |
| **RTE** Routing (→ F1m) | Audio routes: `validate.rs` (an `audio` parameter accepts only generator sources, and a generator source targets only `audio` parameters), the `canon.rs` check, `plan.rs` (topological order, ties in patch order, `audio_cycle`), per-generator pre-level buffers, `engine.rs` step 7, `VoiceCx::audio_in`, `docs/dsp/engine.md` | F0m | 2–3.5 | L |
| **SVP** Stereo voice path (→ F1m) | The per-plan `stereo` flag, `voice_fx` instantiated twice when it is set, the balance law; no-alloc and swap-carry tests | F0m | 1.5–3 | L |
| **CAT** Catalogue (→ F1m) | Every S2a `ParamSpec` table with the counts checked against the answer cap; the pinned-enum rule; the rows `answer.rs` needs, handed to ANS or RC (`answer.rs` is R5's). **Decides** the MSEG's fixed point count, the form of envelope times above 10 s, what `lfo` gains, and whether `wt`'s unison parameters are in F1m's list | F0m; reports 03–05 | 1–2 | L |
| **TMP** Tempo (→ F1m) | `Performance::tempo_bpm` and a tempo event applied at segment boundaries; the plugin maps nice-plug's per-block `Transport::tempo`; `enth render --bpm`; `lfo`'s `sync` and `beats` (divisions with dotted and triplet feels, stepped under modulation) and the retrigger and one-shot modes CAT adds | F0m | 0.75–1.5 | L |
| **V1F** (optional) | Generator v1 and layout 1 read frozen copies of the specs, which would lift rule 1. Not planned | F0m | 1–2 | L |
| **F1M** The F1m judge's findings | Its must-fix items, in the files they name | RTE, SVP, CAT, TMP | 0.5–1.5 | L, Opus |

**The modules.** Each delivers rule 7's list. Sources and costs: [report 05](../_local/sessions/2026-10-25-s2-depth-and-fast-training/05-dsp-algorithms-and-sources.md).

| Lane | Kind (section) | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **DST** | `dist` (voice_fx): soft, hard, sine and triangle fold, asymmetric, bitcrush, downsample; ADAA-1 per voice; zero-latency 2× or 4× IIR oversampling (`enth-dsp::resample`, coefficients designed from the paper); drive, bias, tone, `mix`. A global twin is a second kind | F1m | 1.5–2.5 | L |
| **WT** (→ F2m) | `wt` (generators): generated tables (FM-index sweeps, formant-swept saws, sparse metal spectra), phase warps (bend and the like), `pm`, `am` (bipolar is ring mod) and `feedback`, on phase-read waves only, so PM never meets a polyBLEP shape; at most 63 parameters (the answer code's cap since D3; UNI's unison parameters count) | F1m | 2–3.5 (this plan, from 3–5) | L |
| **FLT** | `comb` (keytracked), `formant` (a continuous vowel path) and last `ladder` (TPT Moog) (voice_fx) | F1m | 2–4 | L |
| **OTT** | `ott` (fx): three bands, upward and downward per band, on the compressor's detector | F1m | 2–3 | L |
| **MSEG** | `mseg` (modulators): a fixed point count, a curve per segment, a length synced or in ms, loop, one-shot and sustain-loop; and **envelope times above 10 s** in CAT's form (a parameter or a kind) | F1m, TMP | 1.5–2.5 (this plan) | L |
| **UNI** | Unison inside `wt`: mono first on `wide` f32x8 lanes with a fixed horizontal sum, detune and blend, then stereo spread through SVP's bus; the six-target golden for `wide`'s exact subset | WT, SVP, S0d's `wide` | 1.5–2.5 (this plan) | L |
| **MOD** | `chorus` (with a flanger mode) and `phaser` on one modulated-delay core (fx) | F1m | 2–3 | L |
| **EQ** | `eq` (fx) wrapping [`enth-dsp/src/biquad.rs`](dsp/biquad.md) (lane BQ owns it; per-segment coefficient ramps are EQ's to add) | F1m | 1–1.5 | L |
| **DLY** | `delay` (fx), synced in beats | F1m, TMP | 1–2 | L |

**Integration**

| Lane | Owns | Depends on | Size | H/L |
| --- | --- | --- | --- | --- |
| **SKS** (→ F2m) | `wt` and an audio route through the plugin and the editor; `host-render`, `cli-equal`; the REAPER scenario | WT | 1.5–2.5 | **H** |
| **MTC** Matcher holes | The `structure/1` matcher's kind families (`score/matching.rs`, so a new kind pairs instead of scoring 0), the coach, deconstruct and the CLI aliases | F1m | 1–1.5 | L |
| **G2B** G2 patch | The G2 reference patch, its budget (default 8 voices), the real-time row and the swap blocks | WT, UNI, DST, OTT | 1–1.5 | L |
| **EDS** Editor density | Dense cards (a focus region or a second layout), audio-route drag; kittest, the overflow report, `leak-test` | F2m; G2B's patch | 2–4 | **H** |
| **TPL** Templates | Create templates: growl, reese, wobble and FM bass | WT, DST, FLT | 1–2 | L |
| **LS2** Lessons | A lesson per user-facing kind | Each kind | 4–8 | L, Opus |
| **EV5** Agent evals | Agent-eval families (eval version 4) | SKS | 1–1.5 | L |
| **INT-S2a** S2a ships | The full gate, the GS2a table, owner sheet 8, a friend build, the hide flag cleared per kind | Everything | 2–3 | **H** |

**Totals:** about 35–63 agent-hours: the spine 7.5–14.5, the modules 14.5–24.5, integration 13.5–24 (V1F, if chosen, adds 1–2).

**Cut list**, agreed in advance: if the phase runs late, `ladder` (FLT's last item), then `delay`, `eq` and `phaser`, move to S2b with their lessons.

### The answer code (R5a, D3)

Not an S2a lane: it is **R5a's change**, because `answer.rs` is R5's and phase-4.md's F1e freezes it. Answer layout 1 allowed 16 kinds (`KIND_BITS` 4; 7 used), 15 parameters per kind (`PARAM_BITS` 4, where 15 ended a run) and 8 modules per section (`PLACE_BITS` 3). **S2a reserves 11 kinds against 9 free**, so two could not be named in an answer, and an MSEG of 8–16 points exceeded 15 parameters (report 06: 5 points × 3 values already reach it). Layout 1 was frozen at F1e but unreleased (SKD is merged and no R5a friend build exists), so widening it was free and later it would be a version.

- **D3 (decided 2026-10-26: widen only):** more bits for kind, parameter, place and target. The owner chose **no multi-part answers and no quantised answers**, so `bad_length` stays as [duels.md](duels.md#bad-length-on-touched-level-1-answers-found-by-the-coordinator-measured-by-lane-dui) describes (code v1's decoder refuses a code over 64 symbols before it reads a field; the card's note and the friend sheet's line stay). **Lane ANS built it** (one light lane, in `answer.rs`; layout 1 stays layout 1): `TARGET_BITS` 5→6 (62 modules), `PARAM_BITS` 4→6 (parameters 0–62 per owner, 63 ends a run), `PLACE_BITS` 3→4 (16 modules per section), `KIND_BITS` 4→6 (64 kinds, so `delay` 16 and `ladder` 17 can be written). Answers grew by 1–4 symbols at level 1 and 15–21 at level 5 ([duels.md](duels.md#lane-ans-the-answer-codes-fields-widened-d3) has the before and after). A commitment hashes the patch, so scorecards still match.
- **What CAT must keep, and the guards that check it** (compile-time asserts in `answer.rs`, with tests that name the offender): at most 64 kinds counting the hidden ones (18 today); at most 63 parameters per kind (indices 0–62; 63 ends a run), with a `result-params-v1` row for each listed kind; each section at most 16 modules and the section caps summing to at most 62 modules; `MAX_ROUTES` within 64. **CAT still adds the table rows for kinds 7–17** (until then, a code naming one is `answer_too_new` and the encoder refuses an answer that holds one). A change past a guard needs a new layout, so tell the judge.
- **If the owner had declined** (not taken): a patch using a kind or parameter past the old caps could not be sent as an answer, and CAT would have capped the MSEG at 15 parameters.
- Cross-referenced from [phase-4.md](phase-4.md#open-checks).

## Waves

**Slots.** S2a takes heavy slot S; R4 and R5 keep slot L, one heavy lane at a time. Otherwise at most four light lanes build at once, **shared with R4 and R5**, and the freeze-path lanes go first. Docs-only lanes (DOCS-S2) don't count, and R3b's training runs remotely and takes no slot.

| Wave | Slot S (heavy) | Light | Starts when |
| --- | --- | --- | --- |
| 0 | — | DOCS-S2 | The summary is sent |
| 1 | S0d (→ F0m) | — | DOCS-S2 merged; no root-manifest edit in flight |
| 2 | — | TMP, CAT, RTE, SVP (→ F1m); V1F if chosen | F0m |
| 3 | — | DST and WT first; then MTC, FLT | F1m |
| 4 | SKS (→ F2m) | OTT, MSEG, UNI; then MOD, EQ, DLY | WT merged for SKS and UNI |
| 5 | EDS | G2B, TPL, EV5; LS2 as kinds land | F2m; G2B before EDS's overflow check |
| 6 | INT-S2a, then listening and a friend build | LS2 | Everything merged |

- **The machine alone:** INT-S2a's REAPER, soak and timing steps and G2B's timing runs take `flock -x /tmp/enth-quiet.lock` and never overlap phase 4's INT lanes or TAP's timing runs.
- **Critical path:** S0d (1–2) → RTE + CAT (2–3.5) → F1M (0.5–1.5) → WT (2–3.5) → SKS (1.5–2.5) → EDS (2–4) → INT-S2a (2–3), about 11–20 agent-hours. UNI → G2B → INT-S2a is shorter. Your sittings come on top.

## Acceptance per lane

Each lane stops at its own criteria and reports against them.

| Lane | Acceptance |
| --- | --- |
| **DOCS-S2** | Every "Docs to change" row of the summary that is this lane's applied; every relative link and anchor it wrote resolves |
| **S0d** | The F0m row: every golden, fixture, v1 golden and layout hash unchanged, reserved kinds invisible, `wasm-check` green with `wide` |
| **RTE** | No-audio-route patches bit-exact (73 goldens); a non-`audio` destination and a cycle refused; apply and inverse proptest with audio routes; no-alloc over a swap that adds one |
| **SVP** | Mono bit-exact; stereo with spread 0 equals mono on both channels; swap carry; no-alloc |
| **CAT** | `const` checks; kind and parameter counts within the cap in force; every new parameter round-trips in an answer code; the decisions it owns written down |
| **TMP** | Bit-exact at host blocks 1, 64, 100 and 4096 with a tempo change; an unsynced patch unchanged |
| **Every module lane** | The same bits on six targets; FTZ-independent; zero subnormals after a 40 s tail; latency 0; budget + 10 %; two instances keep separate state; `mix` 0 passes the input; **aliasing measured spectrally** (a hash cannot see it: a swept or high-index render analysed with the lane's recorded bound, as [osc.md](dsp/osc.md) did) |
| **DST, OTT, FLT** | Plus the kind's defining response by a spectral or closed-form check (fold curves, per-band gain curves, comb teeth at the keytracked spacing, formant peaks at 04's vowel table) |
| **WT** | Plus the audio-route golden, an index and feedback sweep inside the aliasing bound |
| **UNI** | Lanes equal a scalar reference bit for bit on six targets; a sub-voice's seed never depends on a parameter value (a test); 16 voices × 7 inside G2's budget; spread 0 equals mono |
| **MSEG** | Loop, one-shot and sustain-loop; a synced length exact at host blocks 1, 64, 100 and 4096 across a tempo change; point edits apply and invert |
| **MOD, EQ, DLY** | The crossfade swap's copied buffers measured (the P1b p99 swap block was 797 µs against a 1,333 µs deadline); a delay time exact in beats; EQ within 0.1 dB of the cookbook |
| **SKS** | The F2m row; CLI = plugin for a `wt` golden |
| **MTC** | Each new kind pairs in `structure/1`; v1 golden challenges unchanged |
| **G2B** | G2 within its budget; p99 under 25 % of the 64-frame deadline with an edit every 250 ms |
| **EDS** | Nothing scrolls at 1200 × 800 for G2 in the chosen layout; 0 idle frames; `leak-test` |
| **TPL** | Each template beats `init` on its genre's dev targets in create |
| **LS2** | `lesson-check --cross --solve` green; no coverage hole for an S2a kind |
| **EV5** | The new families recorded with model, date and cost (paid runs only with your OK) |
| **INT-S2a** | The full gate; the GS2a table filled; owner sheet 8 written |

## The GS2a checks

The table of record will be `docs/s2a-gates.md` (INT-S2a), planned. Proposed; you set the criteria.

| Check | How | Who | When |
| --- | --- | --- | --- |
| Nothing moved | The 73 goldens on six targets, the generator-v1 golden, `layout-v1.txt`, every fixture unchanged; v0-b loads and create runs with "model layout 1: P1 modules only" recorded | S0d, RTE, INT-S2a | F0m; each merge |
| Real time | G2 within its budget; the p99 row with edits every 250 ms; the swap blocks | G2B | Wave 5 |
| Aliasing | Each module's spectral test inside its bound | Each module lane | At its merge |
| Coverage | Report 04's ten sketches (S1–S10) matched in create; the count replaces 04's judged percentages | TPL | Wave 5 |
| Lessons | `lesson-check --cross --solve` green; no hole for an S2a kind | LS2 | Wave 6 |
| Sounds you like | A wobble, a growl, a reese and an FM bass you like, made in the plugin (owner sheet 8) | **You** | After INT-S2a |

## Your part

| Decision | Default if you don't answer | When |
| --- | --- | --- |
| **D1** S2a starts now, re-aimed at bass music, in this page's order | Yes | Now |
| **D2** The S2 rules above | Yes | Now |
| **D3** Widen the answer code before R5a's friend build and fix `bad_length` in the same change: **decided 2026-10-26: widen only**; no multi-part and no quantised answers, so `bad_length` stays | Widen now (done: lane ANS) | Done |
| **D4** No arpeggiator or sequencer in S2; a synced step gate or volume MSEG | Yes | Now |
| **D5** BPM is a deterministic render input, default 140 | Yes | Now |
| **D6** No full training run until layout 2; the fast loop first (lane FTL); calibrate T1 when a GPU is idle | Yes | Now |
| **D7** Layout 2 is layout 1 as a prefix plus appended segments, warm-started from v0-b | Yes | Before S2b |
| **D8** Recipe experiments reuse the 2.07 M rendered samples; renders run overnight and never beside training | Yes | Now |
| **D9** Mono unison first, spread through the stereo bus, `wide` SIMD with the new bans | Yes | Now |
| **D10** Latency stays zero (ADR-0006 unchanged) | Yes | Now |

Sittings, this plan's estimate (about 1.5–2 h): read the summary and answer D3, 15–20 min; an optional taste check of the exported WAVs of DST and WT (agents export them and never play them), 15 min in wave 4; owner build 7 and the four sounds, 60–90 min after INT-S2a. Friends get the friend build; no friend round is planned for S2a.

Until you say otherwise: **tempo and distortion come first, FM/PM/AM/ring mod arrive through `wt`, unison follows PM, no kind appends an item to a pinned enum, every effect has `mix`, latency stays zero, the answer code is widened (done 2026-10-26), and S2b waits for F2m.**

## Rules for every lane

- Branch from the latest freeze, and write only the files your lane owns. One owner per doc: this page (DOCS-S2), `docs/dsp/<kind>.md` (each module lane), [dsp/engine.md](dsp/engine.md) (RTE), [dsp/catalogue.md](dsp/catalogue.md) (CAT regenerates it), [benchmarks.md](benchmarks.md) (G2B adds the G2 row), [editor.md](editor.md) (SKS, then EDS), the lesson plan (LS2), the gate table and owner sheet (INT-S2a). Report against your acceptance criteria when you stop.
- **Privacy.** Never put your email, name or other personal details in any request, header, URL or payload; use `enth-research/0.1`. **Web etiquette:** read `robots.txt` and the site's terms before a scripted fetch.
- **No windows on the real display** (Xvfb, as [AGENTS.md](../AGENTS.md#ui) says), and **never play audio**: a taste check's WAVs are exported for you.
- **Foreground commands** with a timeout above their measured length; a backgrounded agent can go idle and never resume. Keep a running `notes.md` in persistent scratch (`~/code/enth-scratch/s2/<lane>/`).
- **Concurrency.** One heavy slot, S ([Waves](#waves)); 15 GiB of RAM is shared with phase 4's lanes. `CARGO_BUILD_JOBS` as the coordinator's lane rules say, a separate `CARGO_TARGET_DIR` per lane on ext4, `flock -s` around builds and `flock -x` around timing runs. Build only the crates you need until the final gate.
- **A two-tier gate, judged by exit status only:** `scripts/gate.sh lane <worktree>` before a lane reports, `gate.sh full` (the coordinator's) before a merge. A docs-only lane checks its links and anchors too.
- **Licences, per file.** Write DSP from cited papers, manuals and permissive sources, never from memory of a GPL synth's code; Mutable Instruments' STM32 code is MIT (its AVR code is GPL; check each file), DaisySP's LGPL submodule is out, Faust functions only where declared MIT or STK-4.3 (never compiler output), HIIR's coefficients are designed from Valenzuela and Constantinides rather than copied, and every port records upstream, commit, licence and copyright in `THIRD_PARTY_NOTICES.md` and its `docs/dsp` page.
- **Composite artefacts belong to the coordinator:** composite goldens, bench budgets, the whole-editor snapshots per layout.
- **Merge hotspots are defused up front.**

  | Hotspot | Who edits it |
  | --- | --- |
  | `Cargo.lock`, `[workspace.dependencies]`, `[patch.crates-io]`, `clippy.toml` | S0d (the `wide` pin and bans); afterwards the coordinator only |
  | `enth-patch/src/catalogue/mod.rs` (`ModuleKind`, `CATALOGUE`, `ALL`) | S0d (every kind, discriminants fixed); CAT the rule text. Each `catalogue/<kind>.rs` table: CAT until F1m, then a change through the coordinator |
  | `canon.rs`, `validate.rs` | RTE |
  | `plan.rs`, `engine.rs`, `module.rs` | RTE (routing) and SVP (stereo) each through their own seam file; the call sites are the only lines both touch |
  | `enth-render`'s `Performance` | TMP, additive |
  | `enth-engine/src/modules/<kind>.rs`, `enth-dsp` cores, `benches/<kind>.rs`, `docs/dsp/<kind>.md`, golden folders | Each module lane, its own (pre-declared by S0d) |
  | `THIRD_PARTY_NOTICES.md` | Each module lane appends its own entry |
  | `answer.rs`, `result-params-v1` | Not S2a's: ANS, RC or the coordinator, with CAT's rows |
  | `random/` (generator v1), `layout-v1.txt` | Nobody; frozen |
  | `score/matching.rs`, `coach.rs`, `deconstruct/`, `aliases.rs` | MTC |
  | `enth-ui`'s editor, `sizes/` | SKS (the minimum), then EDS, tokens only in its own file |
  | `enth-plugin`, `enth-cli` | SKS; no module is a host parameter (the 72 ids stay) |
  | `content/lessons`, `lessons.yml` | LS2; `content/agent-evals` EV5; create templates TPL |
  | REAPER and timing | INT-S2a and G2B take `flock -x` |

- **Ownership crossings** are additive only and listed in the report.
- **Risks** (report 06 §6 has all 14, each with its mitigation):
  1. CPU blow-up (SIMD unison, the G2 budget, a voice-count rule);
  2. aliasing at high FM index (PM on phase-read waves only; spectral tests);
  3. determinism of new DSP (`libm` or polynomial shapers, a fixed sum order, six-target goldens);
  4. the answer code out of room (D3: widened on 2026-10-26, and guarded at compile time);
  5. a "harmless" enum item tripping v1 and layout 1 (rule 1);
  6. silent scoring holes (MTC in wave 3);
  7. no editor space (EDS);
  8. lesson debt while coverage turns blocking at GR2 (LS2, the hide flag);
  9. crossfade swaps copying effect buffers (minimal buffers, G2B measures);
  10. model churn (one layout per wave, a superset layout).

## S2b outline

S2b is the data freeze for the next model and is **not planned here**. It follows F2m and gets its own short plan (with a research note if the module set has moved) when S2a's modules are in. Outline, from the summary and [report 06 §4](../_local/sessions/2026-10-25-s2-depth-and-fast-training/06-architecture-impact.md#4-the-model): generator v2 (or a `ds2` sampler), **template 1** (template 0 plus the S2a modules switched off), mask and label rules for the new segments, and **layout 2 as layout 1's 102 entries plus appended segments**, so the 2 M ds1 clips are relabelled instead of re-rendered, only new structure is rendered (overnight), and v0-b warm-starts the new head; training runs through the tiered loop (T1 proxy, T2 confirm, T3 ship) on the remote GPUs; create keeps working because S2a sounds enter through templates. A kind that wants a variable-length input, an envelope follower and a global distortion twin are S2b candidates.

## Deferred

Beyond S2a: an arpeggiator and a sequencer (D4); `bend`, aftertouch, MPE and velocity routes, LFO fade-in (report 04 §6 says the plan generalises if these ship alongside the MSEG); a frequency shifter, a limiter, a transient shaper, an envelope follower; wavetable editing in the editor, the CloudSeedCore reverb; true Phase Plant lanes (schema 2); a sampler or granular engine (realism is tier C); S3 and S4 as before.

## Open checks

Not settled by the research; each is owned by a lane, by you or by a later milestone.

| Check | Owner | When |
| --- | --- | --- |
| The answer code's design: bits per kind, parameter and place; multi-part against quantised answers | DUI's report; lane ANS; your D3 | Done (2026-10-26: widened; neither multi-part nor quantised) |
| The MSEG's point count (8–16, against the 63 parameters per kind the answer code holds since D3; 16 points × time, level and curve is 48) and envelope times above 10 s (a parameter or a kind) | CAT | F1m |
| Whether `wt`'s unison parameters are in F1m's catalogue (inert until UNI, hidden with the kind) or UNI appends them to the still-unreleased kind | CAT | F1m |
| Where tempo lives in a challenge's render spec, since code v1 cannot grow | TMP, CAT; the F1m judge | F1m |
| Chain depth: `MAX_VOICE_FX` and `MAX_FX` are 4; 16 is the most `PLACE_BITS` allows (8 before D3) and older builds refuse more than 4 | TPL, from the ten sketches | Wave 5 |
| G2's voice count and budget (default 8 voices, at most 12,000 instructions per frame as G1) | G2B | Wave 5 |
| How module lanes measure aliasing (a hash cannot see it): the sweep and the bound | S0d's harness, each module lane | F0m; each merge |
| `wide`'s determinism: the bans and a six-target golden, and an arm runner's hash run before SIMD lands | S0d, UNI | F0m; UNI |
| OTT's defaults come from one or two blogs: read Ableton's preset or measure Xfer's | OTT, CAT | F1m |
| Per-file licences of each port (Dexed MSFA, Faust functions, Mutable Instruments files) | Each module lane | NOTICE entries, as for the reverb |
| Who clears each kind's hide flag, and when against GR2 | INT-S2a | Wave 6 |
| T1's seed noise and agreement with T2 (training, not S2a) | Lane FTL, then the first idle GPU | Not S2a's |
| The ten D defaults; silence means the defaults | You | Any time |
