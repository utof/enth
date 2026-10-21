# Research brief: R3 Sound → patch

- **Date:** 2026-10-21
- **Milestone:** R3 Sound → patch ([docs/roadmap.md](../../../docs/roadmap.md#r3--sound--patch-610-weeks)). "Create from sound": heuristic start + CMA-ES, then learned proposals (ONNX + `tract`) returning several candidates. Fully local. **Budget: under 5 minutes per sound on the developer's laptop; aim for about 60 seconds.** Robust to real-world audio: training renders go through realistic augmentation (low-bitrate codecs, resampling, low bit depth, clipping and limiting, reverb, noise and hum, EQ tilts, stereo-to-mono, trimmed or layered sounds, varied notes, velocities and lengths). Codec encoders such as LAME are LGPL: acceptable in the never-shipped `ml/` pipeline, never in the product.
- **Gate GR3:** on held-out sounds from our own synth, model + search beats search alone at equal time; on real samples, including a held-out set of degraded uploads (low-bitrate MP3, noisy, reverberant, clipped), a blind listening test prefers it; every result arrives in under 5 minutes.
- **Asked by:** the owner's standing request to keep advancing milestones in parallel (R3's research may run beside R2's). Docs are suggestions; only Rust and a native, non-WebView UI are firm.

## Where things stand (2026-10-21)

- **Solver pipeline** ([design.md §7–§8](../../../docs/design.md#8-solver-pipeline-sound--patch)): a heuristic start (pitch, centroid and tilt, envelope, noisiness), our own ask–tell CMA-ES with IPOP restarts (`enth-solver`), an order-stable parallel evaluator, a cap of 5,000 evaluations; `enth match` solves 12 of the 20 GR1 codes from the CMA-ES floor (the untouched start solves 0). Measured in Phase 0: one evaluation (2 s render + features) costs about 7.5 ms on one thread on the toy synth; eight workers give 4.3–4.7× on a 6-core laptop.
- **The metric:** `sound/1` (multi-resolution log-mel L1 with in-house BS.1770 loudness, metric v1.1 pooled mel, a pitch and waveform guard; ADR-0007) and `structure/1`.
- **Data sources we own:** the deterministic engine and render (`enth-render`), frozen generator v1 (`RandomSpec`) for random patches, 22 preset candidates, the agent-eval goals.
- **Not built:** any ML. design.md §8 plans `enth-py` renders, training in `ml/`, ONNX export, `tract` in `enth-solver`; a model maps audio to K candidate patches (generative, symmetry-aware), each refined by CMA-ES and shown as a branch (Synplant-style); structure later by embedding retrieval and topology search.

## Constraints

Rust and a native non-WebView UI in the product; the training pipeline (`ml/`, Python allowed) never ships; permissive licences in the product (MIT/Apache/BSD/ISC/Zlib/BSL-1.0; MPL-2.0 needs the owner's OK); dataset and model-weight licences checked from the source; real-time safety (inference and search run on a worker, never on the audio thread); determinism where it matters (the search is deterministic given its start; say whether inference must be bit-exact across platforms); wasm32 for core crates where feasible; fully local inference; no telemetry.

## Sub-researchers (run in parallel)

| # | Axis | Report |
| --- | --- | --- |
| 1 | **Prior art and models.** Synth parameter inference and sound matching, 2018 to today: InverSynth, Sound2Synth, DiffMoog, neural proxies and differentiable synths (DDSP), flow-matching and diffusion approaches, set prediction and permutation symmetry (oscillators, modulation slots), multi-candidate outputs, discrete plus continuous parameters, out-of-domain (real) sounds; commercial products (Synplant 2's Genopatch, others). What results are reported, on what data, and what transfers to real recordings. Open-source code and licences, checked today. A recommendation for our first model and why. | `01-prior-art-models.md` |
| 2 | **Data, augmentation and training.** Generating training data from our engine (sampling priors: generator v1 versus distributions fitted to presets; dataset size; dedup by perceptual distance), the augmentation chain above with each tool's licence (codecs: LAME, libopus, fdk-aac and ffmpeg builds; reverb impulse responses such as OpenAIR; noise datasets such as DEMAND, MUSAN, FSD50K, with their licences), unlabelled real recordings for the audio side (self-supervised pretraining, which datasets, licences), compute (what trains on one consumer laptop GPU or CPU versus a rented GPU, rough costs), the framework (PyTorch or JAX in `ml/`), reproducibility, and the held-out evaluation sets GR3 needs (our synth; real samples; degraded uploads). | `02-data-augmentation-training.md` |
| 3 | **Inference and integration in Rust.** `tract` (version, ONNX op coverage, CPU speed, wasm32) against `ort` (onnxruntime: native library, size, licence), `candle` and `burn`, checked today; model-size and latency budgets inside a plugin (bundled versus a downloaded pack); threading on the plugin worker beside CMA-ES; whether inference must be deterministic; the audio-input path (dropping a file on the plugin or the CLI: decoders such as `symphonia` or `hound`, their licences; resampling; trimming); and the UI (candidates as branches, progress, cancel). | `03-inference-integration.md` |
| 4 | **Evaluation, risks and the lane plan.** How to run GR3 honestly: model + search versus search alone at equal wall-clock time, the blind listening protocol for real samples (level-matched, the owner plus friends), the degraded-upload set; what could surprise us (domain gap, symmetric solutions, metric blind spots on real audio, licences of training data); a phased plan (R3a: "create from sound" with the existing heuristic and CMA-ES from a file, shippable alone; R3b: learned proposals), lanes in waves (at most two heavy plugin builds at once on this laptop), freezes and acceptance, and the owner's time. | `04-evaluation-risks-plan.md` |

## Rules

[docs/research-process.md](../../../docs/research-process.md) applies: date every claim and link it; verify licences from the actual file; mark claims *measured*, *documented* or *anecdotal*; say plainly when evidence is missing; reports of at most about 25 KB.

AGENTS.md also applies: Xvfb only; persistent scratch with `notes.md` (`~/code/enth-scratch/2026-10-21-r3/NN/`); `flock -s /tmp/enth-quiet.lock` for builds; never play audio; **no personal details in any web or API request** (User-Agent `enth-research/0.1`).

Reports are saved verbatim and never edited.
