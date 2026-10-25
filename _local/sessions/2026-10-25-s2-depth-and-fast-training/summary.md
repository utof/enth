# Summary: S2 Depth (a synth wide enough for dubstep) and a faster training loop

- **Date:** 2026-10-25
- **Brief:** [brief.md](brief.md)
- **Reports:** written by sub-researchers and kept verbatim. This summary wins where it corrects them.

| # | Report |
| --- | --- |
| 01 | [fast-feedback methods](01-fast-feedback-methods.md) |
| 02 | [our pipeline, measured](02-fast-feedback-our-pipeline.md) |
| 03 | [synth inventories](03-synth-inventories.md) |
| 04 | [dubstep anatomy](04-dubstep-anatomy.md) |
| 05 | [DSP algorithms and sources](05-dsp-algorithms-and-sources.md) |
| 06 | [architecture impact](06-architecture-impact.md) |

## Answers

### 1. Tighter feedback loops, so training no longer means hours

- **Rendering is the slow part, not training.**
  - One iteration on 2026-10-22 took about 4.3 h, of which 3.1 h was rendering 2.07 M samples. Training B on data that already existed took 51 min. (02, measured)
  - Rendering beside training cut GPU throughput by 40 %.
  - So recipe experiments reuse the existing 2.07 M samples. Renders happen only for a new layout or augmentation, and only overnight.
- **A short run can rank changes if it finishes its learning-rate schedule.**
  - A schedule compressed to a short run correlates 0.91 with full training.
  - An early checkpoint of a long run correlates only 0.75–0.79.
  - Ranking at one small scale picks the larger model's winner in about 80 % of pairs. (01, documented)
  - So we switch to a constant-LR schedule with cheap cooldowns (WSD), and decouple the two knobs our config ties to run length: the WTA anneal and the weight-decay timescale.
- **The tiered loop.** Both arms of a comparison run on the same machine, with paired seeds, data order and dev samples:

  | Tier | Budget | What runs | What it may decide |
  | --- | --- | --- | --- |
  | **T0** smoke | ≤ 5 min | Overfit one batch, export, kernel parity | Go or no-go |
  | **T1** proxy | ~11 min per arm on the 2080 Ti | Full width, 1/5 of the steps, decayed, on a 240–427 k slice | Kill changes worse by more than about 3 SE; promote the best |
  | **T2** confirm | ≤ 2 h | All existing data, plus create at 250/500 renders (~2 min per arm, against 11–22) | The recipe for T3 |
  | **T3** ship | Overnight | New renders, the full run, create at 4,000, GR3 | Ship |

- **The A/B gap was real against dev-set noise.**
  - A paired re-evaluation gives SE 0.0031, so the 0.0172 gap is 5.6 SE. (02, measured)
  - **Seed-to-seed noise is unmeasured.** Every threshold depends on it, so the first T1 job is a calibration: replay A against B at T1 with 2 seeds each, about 45 min of GPU time.
- **Smaller nets (muP) are not worth it at 3.8 M parameters.** The real width trains fast enough. (01)
- **When the synth grows, warm-start instead of training from scratch.**
  - 97 % of v0-b's weights don't depend on the layout. (02)
  - Model-growth papers report about 45 % less compute. (01)
  - Layout 2 should keep layout 1's 102 entries as a prefix (06), so the existing data can be relabelled instead of re-rendered.
- **The create eval is too small to see subtle effects.** The 30-target McNemar test has under 50 % power for them. Add 100–200 synthetic create targets; the 30 GR3 dev targets stay the gate.

### 2. A synth wide enough for the owner's sounds

- **The owner's question: Phase Plant has no sequencer and no arpeggiator.** (03, documented)
  - Its nearest features are the Trance Gate snapin, stepped drawing in its curve and LFO editors, and S&H/random modulators.
  - Serum 2 and Pigments 7 have both an arp and a sequencer; Vital and Surge XT have neither.
  - We keep the arp and sequencer out of S2 (the DAW supplies MIDI). A synced step-gate or volume MSEG covers the rhythmic part.
- **What all five reference synths share (tier A):**
  - **Sources:** classic and wavetable oscillators, noise, unison, hard sync, FM/PM and ring mod between oscillators.
  - **Filters:** a ladder-type filter and a comb.
  - **Distortion:** fold, clip and crush types.
  - **Modulators:** ADSR, LFO, drawable multi-point shapes, random/S&H, macros.
  - **Effects:** chorus, flanger, phaser, delay, reverb, compressor, EQ.

  **enth has 7 of these.**
- **Dubstep is tier A used hard.** It is a mid-range bass whose timbre moves in tempo-locked patterns, over a separate clean mono sine sub. (04)
  - The movement is a tempo-synced, often hand-drawn LFO or MSEG driving three things: a filter (the classic wobble), the source itself (FM index, wavetable position), or serial distortion followed by OTT.
  - Formant, comb and notch filters add the vowels and the metal.
- **What we can make today covers about 3 of the 14 sound families:** sub, basic wobble, two-saw reese, sync screech.
- **The additions that matter, by coverage per unit of work** (04, with 05's costs and 06's routing):

  | # | Addition | Coverage of 14 families | Notes |
  | --- | --- | --- | --- |
  | 1 | **Tempo:** host BPM in the render context; LFO sync with dotted and triplet divisions; retrigger modes; a one-shot mode | ~25 % | No new audio DSP |
  | 2 | **Distortion:** soft, hard, fold, asymmetric, bitcrush, downsample; per-voice anti-aliasing (ADAA), optional 2× IIR oversampling; `mix` | ~45 % | 320–480 instructions per frame as a per-voice insert |
  | 3 | **PM/FM, AM and ring mod between generators**, with feedback | ~60 % | Needs per-generator buffers in a fixed order |
  | 4 | **OTT:** 3-band upward and downward compression | ~68 % | 700–1,000 per frame |
  | 5–6 | **Comb filter** (keytracked) and **formant filter** (vowel morph) | ~80 % | Cheap |
  | 7 | **A sub lane:** a generator can bypass the voice FX | Quality | Routing flag |
  | 8 | **Unison** (mono first, then stereo spread) and **chorus, flanger and phaser** on one modulated-delay core | ~90 % | Unison is the CPU risk |
  | 9 | **MSEG** with a fixed point count, EQ, synced delay, ladder filter, phase warps, `bend`, envelope times above 10 s | Polish and breadth | |

- **Two rules from the patch model** (04):
  - Every new effect gets a `mix` knob.
  - The voice FX chain accepts several instances of one kind.

  Together these give "resampling" chains without a sampler.
- **Beyond dubstep** (04, §6):
  - The same set plus unison, stereo, velocity routes, LFO fade-in and an ensemble covers pads, plucks (the comb filter doubles as Karplus-Strong), leads and basic FM keys.
  - Realism (piano, acoustic, voice) needs a sampler or granular engine: tier C, later.

### 3. What it costs the architecture (06, 05)

**Safe:**
- A new `ModuleKind` changes no existing hash, golden, challenge code or model. v0-b keeps loading.
- A missing implementation is a compile error.

**Narrow spots:**

| Spot | Limit | Consequence |
| --- | --- | --- |
| R5's answer code, layout 1 | 16 kinds (7 used), 15 parameters per kind (osc has 11), 8 modules per section | Too small for a bigger synth |
| Generator v1's pin and R3b's layout-1 hash | Both pin the items of `osc.wave`, `lfo.shape`, `svf.mode`, `noise.color` and `voice.mode` | **S2 adds kinds and neutral-default parameters, never items on a pinned enum.** A ladder filter is a new kind; an MSEG is a new kind |

**The engine's fixed shape changes in three places:**
- per-generator audio buffers in a DAG order, for FM/AM/RM;
- a stereo voice bus, for unison spread;
- tempo as a render input.

Everything else is an ordinary module.

**CPU is the wall:**
- 16 voices × 7-voice unison is about 4,700 instructions per frame on saws (05) and about 12,100 on wavetables (06). G1 is 6,154–6,274 of the 12,000 budget.
- So unison uses `wide` SIMD lanes from its first lane, and S2 gets a G2 reference patch with its own budget.
- `wide` 1.7.1 has determinism traps to ban first: `recip`/`recip_sqrt` are approximate on x86 SSE, and `exp`, `ln`, `sin_cos` and `pow` use `mul_add` internally.

**Latency stays zero:**
- no lookahead limiter (an instant-attack follower plus a soft clip instead);
- IIR oversampling, not FIR;
- a through-zero flanger with a fixed dry delay.

ADR-0006 stands.

**Every effect has a verified permissive source:** Mutable Instruments (MIT), Signalsmith (MIT / 0BSD), DaisySP, per-function Faust (STK-4.3), and Dexed's MSFA files (Apache-2.0, per file). Vital and Surge XT are study only.

## Corrections to sub-reports

| Where | Correction |
| --- | --- |
| 03 puts unison first; 05 and 04 put distortion first | **Distortion and tempo first.** Unison needs the stereo bus and SIMD (effort M/L), and on its own adds width more than new timbres. It lands after the PM routing |
| 06's S2a plan has no MSEG; 04 calls MSEG the biggest single dubstep modulator | **MSEG joins S2a as a new kind with a fixed point count** (05: variable-length data fits neither the catalogue nor a model layout). The point count, 8–16, is decided in lane CAT |
| 04 asks for envelope times above 10 s | Changing an existing range moves the catalogue's normalisation and the layout-1 hash. It must be a new parameter (e.g. a time scale, or time in beats) or a new envelope kind. CAT decides |
| 06 makes `wt` (PM + unison) the F2d skeleton | Keep `wt` with PM as F2d, but **land unison mono-first after PM**, so F2d doesn't wait on SIMD |
| 01's T1 is ≤ 30 min on 4 M samples; 02's is ~11 min on a 427 k slice | Use 02's concrete config, which uses files and keys that exist. Use 01's validation protocol: replay A/B, measure seed noise, and forward-calibrate the next 5 changes at T1 and T2 |
| 02's RAM slice (427 k × 50 KB = 21 GB) | Too big for the 2080 Ti machine's 20 GB WSL. Use 240 k (12 GB) or memory-map the parts |
| 04's coverage percentages | Judgment, not measured. Lane TPL checks them by matching 04's ten patch sketches after each module lands |

## Decisions (provisional until the owner steers; work proceeds on these defaults)

| # | Area | Decision | Evidence | Status |
| --- | --- | --- | --- | --- |
| D1 | S2 | **S2a starts now, re-aimed at bass music.** Order: spine (reserved kinds, routing, stereo flag, catalogue, tempo) → distortion, `wt` with PM/AM/RM → OTT, comb, formant, MSEG → unison (SIMD), chorus/flanger/phaser, EQ, delay → integration, templates, lessons | 03, 04, 05, 06 | Default |
| D2 | S2 rule | Add new kinds and neutral-default parameters only; never items on a pinned enum; every effect gets `mix`; several instances per chain | 06, 04 | Default |
| D3 | R5a | **Widen the answer code before R5a's friend build**, unreleased so it's free now. Fix "bad length" in the same change (multi-part or quantised answers; DUI sizes both) | 06; lane DUI | **Owner's call** (default: widen now) |
| D4 | Sequencer | No arp or sequencer in S2; a synced step-gate or volume MSEG instead | 03 | Default |
| D5 | Tempo | BPM is a deterministic render input (render spec, codes, training data), default 140 | 04, 05, 06 | Default |
| D6 | Training | **No full run until layout 2.** Build the fast loop now (lane FTL); calibrate T1 (A against B, 2 seeds) the first time a GPU is idle | 01, 02 | Default |
| D7 | Model growth | Layout 2 = layout 1 as a prefix, plus appended segments; warm-start from v0-b; one warm-vs-scratch check at T2 | 01, 02, 06 | Default |
| D8 | Data | Recipe experiments reuse the 2.07 M rendered samples; renders run overnight, before training rather than beside it; no on-the-fly rendering | 02 | Default |
| D9 | Unison | Mono unison first; stereo spread through the stereo voice bus; `wide` SIMD with new clippy bans | 05, 06 | Default |
| D10 | Latency | Stays zero (ADR-0006 unchanged) | 05 | Default |

## Open checks

| Check | Who | How |
| --- | --- | --- |
| Seed noise; T1 agrees with T2 ≥ 80 % for gaps > 2 SE | FTL, then the first idle GPU | 3 seeds at T1, 2 at T2; a decision log |
| Both GPUs were busy with other people's jobs all afternoon (16:03–16:38) | Coordinator | Schedule T1 runs when `nvidia-smi` is idle; never preempt |
| The answer-code widening design (bits per kind, parameter and place; multi-part vs quantised) | DUI's report → the R5a release step (INT5) or an ANS lane | Before the friend build |
| How the module lanes measure aliasing (a hash cannot see it) | S0d (the harness), each module lane | A spectral test, per 06's acceptance |
| The MSEG point count; envelope times above 10 s | CAT (F1d) | Judged at F1d |
| `wide`'s determinism bans | SVP or WT | clippy `disallowed_methods`; a six-target golden |
| Per-file licences of each port (Dexed MSFA, Faust functions) | Each module lane | NOTICE entries, as for the reverb |

## Docs to change (lane DOCS-S2)

- `docs/roadmap.md`: S2 pulled forward and re-aimed; the date line.
- **New `docs/phase-s2.md`:** the S2a lane plan (06 §5 with this summary's corrections), freezes F0d, F1d, F2d and F3d, ownership, acceptance.
- `docs/README.md`: the status line, and the reading-order row for phase-s2.md.
- `docs/r3.md` "Training" and `ml/README.md`: the tiered loop, once FTL lands.
- `docs/dsp/` stubs: S0d.
