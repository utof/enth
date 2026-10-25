# Research brief: S2 Depth (a synth wide enough for dubstep) and a faster training loop

- **Date:** 2026-10-25
- **Milestones:**
  - **S2 Depth** ([docs/roadmap.md](../../../docs/roadmap.md#s2--depth-rolling)), pulled forward and re-aimed. The roadmap lists wavetable editing, FM/PM/AM between generators, unison, more effects (chorus, flanger, phaser, saturation, EQ, a limiter), multi-segment envelopes, random modulators, MPE, oversampling and SIMD voices.
  - **R3b iteration speed:** how to train the proposal model in short loops instead of multi-hour runs.
- **Asked by:** the owner, 2026-10-25. In their words, condensed:
  - Dispatch research into **tighter feedback loops**, so we don't have to train so much at once. Maybe smaller models can show early that we're heading for a good result. Any future training must not mean waiting hours for a result.
  - **The synth needs much more versatility.** Today it has a filter, a reverb and a compressor, and that's not enough. Look at what Phase Plant has (chorus, maybe a sequencer). We need the breadth and depth to recreate sounds.
  - **The owner loves dubstep sounds.** Find out what makes a dubstep sound: FM, AM, and so on.
- **Docs are suggestions.** Only Rust and a native, non-WebView UI are firm.

## Where things stand (2026-10-25)

### The engine's modules today

There are seven module kinds (`crates/enth-patch/src/catalogue/mod.rs`, `ModuleKind`), plus voices, a modulation matrix, macros, and goldens on six targets:

| Module | What it is |
| --- | --- |
| `Osc` | Classic waves (a Plaits variable-shape core with polyBLEP), hard sync, a wavetable mode |
| `Noise` | White or pink |
| `Svf` | Cytomic/Simper TPT state-variable filter (LP, BP, HP, notch), with drive |
| `Adsr` | Exponential, RC-style segments with exact segment times |
| `Lfo` | Per voice or global |
| `Reverb` | Stereo FDN, a port of Signalsmith Basics; global |
| `Comp` | Feed-forward, soft knee, make-up gain; global |

`enth-dsp` also has an offline biquad, used by the ear games. Missing today:
- FM, PM, AM and ring modulation between generators;
- unison;
- distortion beyond the filter's drive;
- chorus, flanger, phaser, delay, EQ, multiband, OTT-style compression, comb and formant filters;
- multi-segment envelopes and LFO shapes (MSEG), random modulators, a sequencer;
- oversampling.

### Everything that depends on the module set

Each of these is frozen or versioned, so new modules touch them:

| What | Where |
| --- | --- |
| Patch format v1, with migrations | ADR-0005 |
| The parameter catalogue and its hash | |
| Challenge codes and the frozen random generator v1 | `enth-challenge` |
| The `sound/1` and `structure/1` scorers | |
| Lessons, which cover every module | `content/lessons` |
| The editor's lanes | |
| The R3b model's output layout | `propose-layout/1`: 102 values per hypothesis, tied to the catalogue hash (docs/r3.md, "The model contract (F3s)") |

### The R3b model and its training today (docs/r3.md, ml/README.md)

**The network.** A CNN of 3.8 M parameters and 0.48 GFLOP:
- input: a log-mel [3, 128, 64] plus 5 context values;
- output: 8 winner-takes-all hypotheses plus a confidence head;
- loss: masked Huber in JND units, plus cross-entropy, plus LFO min-over-permutation.

**The data.** `ds1` is rendered by our own engine. Rendering 2 M samples takes about 3 h on 16 CPU threads, and the stored set is about 97 GB in f16. Augmentation costs about 50 CPU-hours per 10 % of 2 M samples.

**A training run.** About 75–80 min for 16 M samples on an RTX 2080 Ti, at 3.4–5.8 k samples/s with AMP. The pilot ran at 1.9 k samples/s on a GTX 1660 Ti.

**Results so far.** The run was data-limited: run A was still improving at the end.

| | Dev best-of-8 loss |
| --- | --- |
| Pilot | 1.774 |
| Run A | 1.389 |
| Run B (shipped as v0-b) | 1.372 |

In create, v0-b passes 16 of 30 GR3 dev targets at 250 renders, against 8 of 30 for search alone (p = 0.004). At 4,000 renders the two are level.

**One full iteration costs about 5–8 h end to end:** data, train, export, parity, then create evaluation.

**Machines:**
- this laptop: 12 threads, 15 GB RAM, Quadro RTX 3000; agents never open windows on its display;
- two remote GPU machines, reached over SSH, both stoppable at any time:
  - an RTX 2080 Ti 11 GB with 16 threads;
  - a GTX 1660 Ti 6 GB with 12 threads.

## Constraints

- Rust and a native, non-WebView UI.
- Licences:
  - permissive only: MIT, Apache, BSD, ISC, Zlib, BSL-1.0; MPL-2.0 needs the owner's OK;
  - **GPL code (Vital, Surge XT, Dexed's GUI, most of chowdsp's plugins) may be studied for ideas, never ported**;
  - check each licence from the actual LICENSE file or source header, per file in mixed repos.
- **Real-time safety:** no allocation, locks, I/O or panics on the audio thread.
- **Determinism:** bit-exact renders on Linux, Windows, macOS arm64 and wasm32. Core crates do no I/O, spawn no threads, read no clocks and use no global RNG.
- **CPU:** the 16-voice budget is at most 12,000 instructions per stereo frame on the scalar path, at least 10× realtime on one core (docs/roadmap.md).
- **No telemetry, no accounts, no server.** Everything runs locally.

## Sub-researchers (run in parallel, one report each in this folder)

| # | File | Axis |
| --- | --- | --- |
| 01 | `01-fast-feedback-methods.md` | Literature and practice: small proxies that predict big runs (scaling-law fits, muP-style hyperparameter transfer, learning-curve extrapolation, successive halving), sound-matching papers, growing a model when the synth grows, small-sample statistics for decisions |
| 02 | `02-fast-feedback-our-pipeline.md` | Our pipeline, measured: where an iteration's hours go, what the A, B and pilot logs say about when a ranking settles, a tiered loop (minutes → one hour → overnight) with the code changes it needs; a bounded proxy experiment on the 1660 Ti |
| 03 | `03-synth-inventories.md` | What Phase Plant, Serum 2, Vital, Pigments and Surge XT ship (generators, modulators, effects, routing), as one tiered union table |
| 04 | `04-dubstep-anatomy.md` | What makes dubstep sounds (wobble, growl, reese, riddim, FM metallic, yoi or formant, screech, sub, neuro), the synthesis, modulation and effects chain behind each, and the smallest module set that covers about 80 % of them; patch sketches in real units |
| 05 | `05-dsp-algorithms-and-sources.md` | The algorithms and permissively licensed sources for each candidate module (FM/PM/AM/RM, unison, wavetable warps, distortion with ADAA and oversampling, chorus, flanger, phaser, delay, EQ, multiband/OTT, comb, formant, frequency shifter, bitcrusher, MSEG, random, follower); costs, aliasing, determinism, effort |
| 06 | `06-architecture-impact.md` | What adding modules and routing does to the frozen contracts (patch v1, catalogue hash, generator v1, codes, scorers, lessons, model layout 1, editor, CPU budget); the S2 waves, freezes and versioning plan |

Then `summary.md`: answers, a decision table, corrections, open checks, and the docs to change.
