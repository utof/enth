# 02: Data, augmentation and training (R3 Sound → patch)

- **Sub-researcher:** 02 of the R3 session · **Date:** 2026-10-21 · **Scratch:** `~/code/enth-scratch/2026-10-21-r3/02/` (`notes.md`, licence copies in `lic/`, timings in `bench/` and `codec/`)
- **Labels used below:** *measured* (run here today), *documented* (a primary page or repo doc, linked and dated), *anecdotal*, *estimate* (my arithmetic from the first two).

## Summary

1. **Generate the data from our own engine, keep the dataset as a manifest, and cache part of it.** Renders are bit-exact (design.md §5), so a dataset is a sampler version, a seed range and an augmentation config. Costs per 3 s clip on a loaded core (*measured*):
   - a 1-voice render: 10–28 ms dry, 130–175 ms with the reverb;
   - a codec round trip: 65–110 ms.

   The CPU, not the GPU, is the bottleneck.
2. **Don't train on generator v1 as it stands.** Level 1 has about 12 distinct sounds, phrases start on C3, envelope times stop at 1.5 s, and values are pulled towards the start patch. Write an R3 sampler (`ds1`) that reuses v1's keyed draws, priors and `observable` mask, and adds a phrase sampler. 22 presets are too few to fit priors: use them as perturbation anchors.
3. **The augmentation chain is buildable today.** This laptop's distro `ffmpeg` 6.1.1 already has LAME, shine, native AAC, Opus, Vorbis, AMR and GSM encoders (*measured*). Use it as a batched subprocess.
   - Skip fdk-aac.
   - Don't import pedalboard (GPL-3.0) or gpuRIR (AGPL-3.0) into MIT `ml/` code.
4. **Shipped weights train only on our renders plus CC0, CC BY or Apache data.** CC BY-NC, CC BY-SA and licence-less sets stay evaluation-only. OpenAIR is offline today.
5. **Real audio without labels:** search-distilled pseudo-labels first. Run model plus CMA-ES on real one-shots, keep the good fits, retrain. It needs no differentiable engine, which the roadmap lists as "Never".
6. **Compute:** the laptop's 6 GB Turing GPU (FP16; CUDA 13 wheels support it) overnight, outside build waves, because RAM is tight. Otherwise a rented RTX 4090 at $0.34–0.89/h (RunPod, 2026-10-21).
7. **Framework:** PyTorch with uv. JAX has no first-party ONNX export, and torchaudio has been in maintenance since 2.9.
8. **GR3 sets to freeze before the first training run:**
   - synth-clean;
   - synth-degraded, scored against the **clean** render;
   - real samples;
   - degraded uploads with *held-out* degradations, including phone re-recordings;
   - a separate dev split.
9. **Owner decision:** is the reverb, noise or compression on an upload part of the sound, or damage to ignore? The patch has its own reverb, noise and compressor.

## 1. What our engine gives us

- **Reproducibility** (*documented*, design.md §5): `render()` is a pure function of (patch, performance, rate, seed, engine version). A training set is fully described by its sampler version, seeds, phrase sampler and augmentation seeds. Clean audio never needs shipping or backing up.
- **Render cost** (*measured* 01:40 today, with the 2026-10-19 release `enth` binary, `enth bench --voices 1 --seconds 30`, under `flock -s /tmp/enth-quiet.lock`). Load average was 24.8 on 12 threads, so these are upper bounds.

| Template | Realtime × per core | 3 s clip |
| --- | --- | --- |
| `init` (osc + env) | 290–310 | ~10 ms |
| `pluck_1osc` / `bass_sub` | 107–131 / 153–185 | ~16–28 ms |
| `pad_2osc` (2 saws, LP, LFO, **reverb**) | 17–23 | ~130–175 ms |

  The reverb dominates: 2,362 instructions per frame against 192 for the compressor (*documented*, dsp/reverb.md, compressor.md). match.md reports 15–30 ms per scoring render with the reverb off. 8 workers give 4.3–4.7× on this 6-core laptop (design.md §7).
- **Python access:** `enth-py` (L8, pyo3 + maturin, MIT/Apache; rust-numpy BSD-2) should expose:
  - `ds1` sampling;
  - batched order-stable `render` on rayon, with the GIL released;
  - the `enth-analysis` front end;
  - the compressor as an effect;
  - `match`.

  A CLI process per clip costs ~100 ms in start-up alone under load (codec timings below).

## 2. Sampling priors

**Generator v1** (*documented*, design.md §9, challenge.md) brings:

- keyed draws, safety bounds and the `observable` mask;
- `musical`, which the docs call "R3's dataset knob" (0 of 200 targets silent at `musical` 100).

As a training source it has gaps:

- level 1 has about 12 sounds, so mixing levels floods the set with duplicates;
- `amounts` below 100 pull values towards the start patch, biasing labels towards the defaults;
- every phrase's lowest key is C3, with holds of at least 1.5 s;
- envelope times are capped at that hold and tail, so slow pads are missing;
- random LFO shapes are excluded.

**The literature** (*documented*):

- [Hayes et al., ISMIR 2025](https://arxiv.org/abs/2506.07199) (Surge XT):
  - 2 × 2 M pairs, sampled from uniform, permutation-invariant priors; they argue presets can break the invariance;
  - 4 s at 44.1 kHz, 128 mels; 1 M steps at batch 128;
  - out-of-domain NSynth and FSD50K tests are clearly worse than in-domain.
- [Bruford et al., DAFx 2024](https://arxiv.org/abs/2407.16643) (Massive): 1 M pairs over 16 parameters, drawn within the factory presets' parameter distributions; clips below −60 dB dropped.
- [arXiv 2609.29320](https://arxiv.org/abs/2609.29320) (2026-09-24; Surge XT, Dexed):
  - uniform sampling, rendered **online with no fixed dataset size**, near-silent clips rejected;
  - validation and test sets frozen by seed;
  - real one-shots train the audio marginal.

**Recommendation: `ds1`**, written in Rust beside generator v1. It is not a challenge-code generator, so v1's freeze rules don't apply, but it is versioned and pinned in every manifest.

- **Mixture** (a starting point to tune on dev, not evidence):
  - 60 % v1 priors at `amounts` 100 at level 5, structure included, with `musical` drawn from {100, 50, 0} at 0.6 / 0.25 / 0.15;
  - 25 % perturbed preset candidates (σ 0.05–0.2 in the normalised domain, plus a few categorical flips; held-out presets excluded);
  - 15 % uniform over the safe ranges.
- **Phrase sampler:**
  - key log-weighted over MIDI 24–84; velocity 30–127;
  - hold log-uniform from 0.08 to 4 s, with envelope caps tied to the hold;
  - 10 % octave or legato pairs, so keytrack, velocity and glide stay learnable;
  - a fixed 3–4 s window from the onset.
- **Symmetric slots** (LFOs, env 2, route order): permute at random or sort canonically, so the prior stays invariant (Hayes et al.). The loss is report 01's call.
- **Labels:**
  - 0–1 values through each `ParamSpec` scale (the ML boundary in AGENTS.md); one-hot categoricals;
  - a loss mask from `observable(patch, phrase)`: gauges, nuisances and `Dead(*)` items get no loss and are written as canonical defaults;
  - the note and `osc1.semi` are degenerate. Label the note nearest f0 with the smallest |semi|, and train with ±50 ct of note noise, since the heuristic pins pitch at inference (match.md).
- **Filters:** reject renders below −60 LUFS. Keep clipped renders. Never filter by duration (owner rule).

## 3. Size, dedup and splits

- **Size:** the literature uses 1–2 M pairs, or a stream, for 16–90 parameters. Ours draws 12–39 items per target (challenge.md). Plan on 1–2 M distinct renders consumed per run. At about 50 ms per render and 4.5× parallelism, the laptop makes about 300 k clips/h (*estimate*).
- **Cache plus rolling refresh** (*estimate*; /home has 174 GB free today, *measured*, and cargo targets once filled it):
  - 250 k clean 3 s clips as mono int16 ≈ 48 GB at 32 kHz (only if the model's mel stops at 16 kHz, like metric v1) or 72 GB at 48 kHz;
  - about 100 k pre-baked codec variants ≈ 19 GB;
  - 5–10 % of the shards re-rendered per epoch from fresh seeds, so each clip is seen 10–30× under different augmentations.
- **Dedup:**
  - first an exact hash of the masked, canonical labels;
  - then pooled log-mel embeddings (~128-d) in hnswlib (Apache-2.0) or FAISS (MIT), dropping pairs within about one `sound/1` just-noticeable step (ADR-0007), calibrated on 1 k pairs;
  - the same radius as a leakage guard against every dev and test item, re-run after each refresh.
- **Splits:** separate sampler stream keys (`r3/train`, `r3/dev`, `r3/test`), never index ranges, so keyed streams keep the sets disjoint by construction.

## 4. The augmentation chain

The order follows a real signal path. Probabilities are starting values. Keep 10–20 % of samples clean.

| # | Stage | How | Runs on | p |
| --- | --- | --- | --- | --- |
| 1 | Layer / mix-in | a second `ds1` render at −6 to −20 dB; a phrase or music bed (our renders, MUSAN music) at −10 to −25 dB | cache | 0.15 |
| 2 | Trim / pad | cut 0–50 ms of the attack (0.1); truncate the tail after 0.3 s with a fade (0.3); 0–300 ms of leading silence (0.3) | GPU | — |
| 3 | Room | a real RIR (BUT ReverbDB, OpenSLR 28) or a synthetic one (decaying noise; pyroomacoustics), wet −20 to 0 dB | GPU (FFT) | 0.3 |
| 4 | Device EQ | HPF 80–400 Hz, LPF 3–12 kHz, 1–3 peaks of ±9 dB, tilt ±3 dB/oct | GPU | 0.4 |
| 5 | Noise / hum | synthetic colours, MUSAN noise, FSD50K CC0/CC BY background classes, BUT room noise, at 15–50 dB SNR; hum at 50/60 Hz plus harmonics at −60 to −30 dB | GPU | 0.3 / 0.1 |
| 6 | Dynamics | **our compressor through `enth-py`**; a look-ahead limiter; hard or tanh clipping at +3 to +12 dB | CPU / GPU | 0.2 each |
| 7 | Stereo → mono | gain imbalance, partial polarity cancellation | GPU | 0.1 |
| 8 | Band-limit | resample to 8–24 kHz and back (julius) | GPU | 0.3 |
| 9 | Codec | MP3 32–128 kbps (LAME), AAC-LC 32–96 kbps (FFmpeg native), Opus 12–64 kbps; **pre-baked** | offline | 0.4 |
| 10 | Bit depth | 8–12 bit with or without dither; 8-bit µ-law | GPU | 0.1 |
| 11 | Normalise | −23 LUFS like metric v1 (in-house BS.1770 through `enth-py`, or pyloudnorm, MIT) | GPU / CPU | 1 |

**Codec cost** (*measured* ~01:50, distro `ffmpeg` 6.1.1, one thread, load average ~16):

- two `ffmpeg` processes per 3 s clip cost **200–300 ms**, mostly start-up;
- without the spawn, per 3 s clip: LAME 64 kbps 88 ms (encode 44×, decode 153× realtime), shine 65 ms, native AAC 98 ms, Opus 24 kbps 90 ms, AMR-NB 110 ms.

So bake the variants offline in batches (concatenate clips with known offsets, or call PyAV or lameenc in-process). Never encode inside the training loop.

**Held out for evaluation only:** shine (a different MP3 encoder), AMR and GSM (phone codecs), the MIT IR Survey's rooms and DEMAND's noise. Set C then measures generalisation, not memorised damage.

**Score degraded synth inputs against the clean render.** We want the instrument back. Metric v1 on degraded audio rewards reproducing the damage.

**Label ambiguity:** "a clean label for a reverberant input" teaches the model to strip room reverb. That is right for a phone recording and wrong for a pad sampled with its reverb. Proposal:

- treat codec damage, hum, noise and device EQ as damage;
- for room reverb and compression, let the K candidates include a dry patch and one using the patch's own reverb or compressor.

The owner decides.

## 5. Licences (verified today from the source file or page)

| Item | Licence (source) | Use |
| --- | --- | --- |
| LAME | LGPL-2.0 ([COPYING](https://sourceforge.net/p/lame/svn/HEAD/tree/trunk/lame/COPYING)) | `ml/` as a library or process; never in the product |
| shine | LGPL-2.0 ([COPYING](https://github.com/toots/shine/blob/main/COPYING)) | `ml/`, held-out encoder |
| libopus, libvorbis | BSD-3 ([opus](https://github.com/xiph/opus/blob/main/COPYING), [vorbis](https://github.com/xiph/vorbis/blob/main/COPYING)) | anywhere |
| FFmpeg | LGPL-2.1+; GPL-2+ with `--enable-gpl` (the distro build is); fdk + `--enable-nonfree` gives an "unredistributable" binary ([LICENSE.md](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md)) | `ml/`, subprocess only |
| fdk-aac | FDK licence, §3 "NO PATENT LICENSE" ([NOTICE](https://github.com/mstorsjo/fdk-aac/blob/master/NOTICE)) | skip |
| AMR (opencore, vo-amrwbenc), libgsm | **not verified today** | held-out evaluation only |
| pedalboard v0.9.26 (2026-10-07) / gpuRIR | **GPL-3.0** ([LICENSE](https://github.com/spotify/pedalboard/blob/master/LICENSE)) / **AGPL-3.0** ([LICENSE](https://github.com/DavidDiazGuerra/gpuRIR/blob/master/LICENSE)) | don't import |
| lameenc / fast-mp3-augment 0.2.0 (2025-12-15) / python-soxr v1.1.0 | LGPL-3.0 / LGPL-2.1 / LGPL-2.1+ | `ml/` only |
| audiomentations v0.43.1 (2025-09-13), torch-audiomentations v0.12.0 (2025-01-15, now `iver56/`), pyroomacoustics v0.10.1 (2026-05-01), julius, pyloudnorm v0.2.0, FAISS | MIT | `ml/` |
| resampy, librosa / libsamplerate / hnswlib | ISC / BSD-2 / Apache-2.0 | `ml/` |
| PyTorch v2.14.1 (2026-09-30), torchaudio v2.11.0, JAX v0.12.0 (2026-10-19), ONNX v1.23.2, onnxruntime | BSD-3, BSD-2, Apache-2.0, Apache-2.0, MIT | `ml/` |
| uv, PyO3, maturin / rust-numpy / python-soundfile (bundles libsndfile LGPL-2.1) / TensorBoard / minimp3 | MIT or Apache / BSD-2 / BSD-3 / Apache-2.0 / CC0 | `ml/` |
| **NSynth** | CC BY 4.0 ([Magenta](https://magenta.tensorflow.org/datasets/nsynth)): 305,979 notes, 1,006 instruments, **16 kHz**, 4 s, disjoint splits 289,205 / 12,678 / 4,096 | training, with attribution |
| **FSD50K** | dataset CC BY; clips CC0 19,873, CC BY 23,506, CC BY-NC 6,041, Sampling+ 1,777 ([Zenodo](https://zenodo.org/records/4060432)); commercial use: "contact" the authors | CC0 + CC BY subset |
| **MUSAN** | CC BY 4.0 ([OpenSLR 17](https://www.openslr.org/17/)); only public-domain or CC content that allows commercial use, per-folder LICENSE files ([paper](https://arxiv.org/abs/1510.08484)) | training |
| **OpenSLR 28** RIRS_NOISES | Apache-2.0 ([page](https://www.openslr.org/28/)); the terms of its RWCP, REVERB and AIR sources are not stated | training (check the sources) |
| **BUT ReverbDB** | "CC-BY 4.0" ([page](https://speech.fit.vut.cz/software/but-speech-fit-reverb-database)): 9 rooms, ~4.7 k RIRs | training |
| TinySOL | CC BY 4.0 ([Zenodo](https://zenodo.org/records/3685367)): 2,913 notes, 44.1 kHz | training, evaluation |
| DEMAND | conflict: the text says **CC BY-SA 3.0**, the metadata CC BY 4.0 ([Zenodo](https://zenodo.org/records/1227121)) | evaluation only |
| ESC-50 | CC BY-NC 3.0 ([LICENSE](https://github.com/karolpiczak/ESC-50/blob/master/LICENSE)) | evaluation only |
| MIT IR Survey | **no licence stated** ([page](https://mcdermottlab.mit.edu/Reverb/IR_Survey.html)) | local evaluation only |
| OpenAIR | per upload, CC or all rights reserved (search snippets); site shows "Account Suspended" (*measured*, 2026-10-20 22:32 UTC) | unavailable |
| WHAM! noise | not verifiable today | — |
| Freesound | per sound CC0 / CC BY / CC BY-NC; originals need OAuth2 (an owner account); 60 req/min, 2,000/day ([API](https://freesound.org/docs/api/overview.html)) | CC0 + CC BY |

**Rule:** shipped weights (MIT, bundled or downloaded) train only on our renders plus CC0, CC BY or Apache data. Credit CC BY sources in `THIRD_PARTY_NOTICES.md`: whether weights inherit attribution is unsettled. Pretrained encoders carry their own provenance: DAC's weights are MIT ([README](https://github.com/descriptinc/descript-audio-codec)), and LAION-CLAP's weights include AudioSet training. That choice belongs to reports 01 and 03.

## 6. Unlabelled real recordings

**Sources:**

- NSynth: CC BY. It is 16 kHz, so there is nothing above 8 kHz; band-limit our renders when mixing it in;
- FSD50K's CC0 and CC BY clips (~43 k across all classes);
- TinySOL;
- Freesound single-event one-shots, CC0 or CC BY (the 2026 paper filtered on the Audio Commons single-event descriptor);
- the owner's own samples, kept local.

**Uses, cheapest to engineer first:**

1. **Search-distilled pseudo-labels** (expert iteration).
   - Model plus `enth match` on real one-shots; keep fits under a `sound/1` threshold as labelled pairs; retrain.
   - Cost (*estimate*): 5,000 evaluations at 20–150 ms over 4.5× parallelism is 22–170 s per sound, so 2,000 sounds take 12–95 laptop hours. It gets cheaper as proposals improve.
   - Risk: it learns search's blind spots. Cap pseudo-labels at about 30 % and listen to each batch.
2. **Audio-marginal training** ([arXiv 2609.29320](https://arxiv.org/abs/2609.29320)): a joint audio–parameter flow trains its audio edge on unlabelled one-shots, and partially noising the reference improved real-audio inversion. Only applies if report 01 picks a joint flow.
3. **RL on a similarity reward** ([SynthRL, IJCAI 2025](https://www.ijcai.org/proceedings/2025/1129)): built for non-differentiable synths; it needs renders, which are cheap here.
4. **Encoder pretraining** with synth "doppelgänger" positives ([Cherep & Singh, ICLR 2025](https://arxiv.org/abs/2406.05923)) or BYOL-style on the real sets: a later ablation.

A learned neural proxy ([arXiv 2509.07635](https://arxiv.org/abs/2509.07635)) is not the banned "differentiable PyTorch copy", but it is close. Ask before building one.

## 7. Compute

**The laptop** (*measured*):

- 6 cores / 12 threads;
- 15 GiB RAM, ~6 GiB available at the check, 11 GiB of swap in use;
- a 6 GB Turing GPU (compute capability 7.5), with a current driver (version redacted by the coordinator).

CUDA 13.x needs driver 580 or later ([NVIDIA notes](https://docs.nvidia.com/cuda/cuda-toolkit-release-notes/index.html)). PyTorch's CUDA 13 wheels keep Turing after 2.15 ([dev-discuss, 2026-09-04](https://dev-discuss.pytorch.org/t/notice-cuda-12-6-wheels-will-no-longer-be-published-from-pytorch-2-15-drops-maxwell-pascal-volta/3432)). Turing has no BF16 tensor cores, so use FP16 autocast with a gradient scaler. `torch` isn't installed yet.

**Bottleneck** (*estimate*):

- fully online, the CPU needs 70–200 ms per sample-core (render, codec, features), which is 25–65 samples/s, or 45–220 h for 10–20 M samples;
- with the cache, GPU augmentation and pre-baked codecs, CPU work drops to a few ms and the GPU sets the pace.

GPU throughput for a 5–30 M-parameter mel encoder is **not measured**. Measure it in the first hour.

**RAM:**

- train overnight, outside heavy build waves, with `systemd-run --user -p MemoryMax=6G`;
- 4–6 loader workers;
- memory-mapped shards only.

**Renting** (*documented*, read 2026-10-21):

- [RunPod](https://www.runpod.io/pricing), community / secure, per hour: RTX 4090 $0.34 / $0.89, RTX 5090 $0.69 / $1.19, A100 80 GB PCIe $1.19 / $1.79, H100 PCIe $1.99 / $2.89;
- [Lambda](https://lambda.ai/pricing): A100 40 GB $1.99, H100 PCIe $3.29.

A first model plus ablations at about 20–40 GPU-hours is **$7–36** on a community 4090 (*estimate*). The pipeline is CPU-heavy, so pick 16 or more vCPUs, or upload pre-rendered shards (30–70 GB). Only the owner can open an account and pay. No personal data is involved.

## 8. Framework and reproducibility

- **PyTorch** (BSD-3), plain training loop. `torch.onnx.export` defaults to the `torch.export` path since 2.9. JAX's ONNX route is the third-party `jax2onnx`; its health is unchecked (GitHub API rate limit).
- **torchaudio** has been in maintenance since 2.9 ([docs](https://docs.pytorch.org/audio/2.9.0/torchaudio.html)), and its last release, 2.11.0 (2026-03-23), trails torch 2.14.1. Write the few ops ourselves (FFT convolution, biquads, mel); use julius for resampling.
- **One feature front end.** Train on the features `tract` will see: `enth-analysis` through `enth-py`, or a torch mel with a parity test against Rust (≤ 1e-4 max abs on 1 k clips). Pin the resampler and mel settings in ONNX metadata.
- **Manifest per run:**
  - commit, engine version, `ds1` version and stream keys;
  - phrase and augmentation configs;
  - per-sample augmentation seed `derive(run_seed, id, epoch)`;
  - `ffmpeg -version`;
  - cache hashes and the `uv.lock` hash.

  A 1,000-clip augmented golden (SHA-256) catches tooling drift.
- **Determinism:** deterministic flags (`torch.use_deterministic_algorithms`, `CUBLAS_WORKSPACE_CONFIG`) only for debugging runs. The artifact is the hashed ONNX file plus a 20-input output golden. Whether inference must be bit-exact is report 03's call.
- **Logging:** local only (CSV or TensorBoard), no cloud trackers.

## 9. Held-out sets for GR3 (frozen with SHA-256 manifests before any training)

| Set | What | Size | Scored by |
| --- | --- | --- | --- |
| **A synth-clean** | the `r3/test` stream (deduped) + GR1's 20 codes + the golden challenges + the 22 preset candidates, never in training or fitted priors | ~2,000 + 42 | `sound/1` and `structure/1`; equal wall-clock against search alone (report 04) |
| **A′ synth-degraded** | A through the held-out chain | 500 | `sound/1` against the **clean** target |
| **B real** | strata: synth one-shots (Freesound CC0/CC BY, the owner's synths); electronic and acoustic (the NSynth test split's electronic and synthetic sources, TinySOL); a "degrade gracefully" group (voice, drums) | ~150 | blind listening (report 04); `sound/1` as a diagnostic |
| **C degraded uploads** | B through held-out damage: shine MP3 64–128 kbps, LAME VBR, AAC 32–64 kbps, Opus 12–24 kbps, AMR-NB, DEMAND noise, MIT IR rooms, clipping at +6 and +12 dB; plus **20–30 phone re-recordings** through the laptop speakers (~30 minutes of the owner's time) | 60–80 | blind listening against the clean original |
| **dev** | separate streams and files for tuning | ~10 % | anything |

B and C record per-file URL, licence, attribution and hash. Only CC0 and CC BY files are committed. NC and licence-less files stay local.

## 10. Risks and gaps

- **Risks:**
  - the domain gap and the label policy (§4); Hayes et al.'s NSynth and FSD50K results show the drop (*documented*);
  - the reverb share of patches moves render cost tenfold (*measured*);
  - RAM and disk contention with build lanes.
- **Gaps (not measured or not verified today):**
  - GPU training throughput (no torch here; I didn't download it);
  - render cost on the current main (I used the 2026-10-19 binary under load);
  - AMR and GSM library licences, WHAM!'s licence, OpenAIR's status, the terms of OpenSLR 28's source databases;
  - whether Freesound's terms mention model training;
  - the 2026 paper's numbers (lost in text extraction);
  - jax2onnx's health;
  - a dataset-size ablation for a space like ours (plan 250 k vs 1 M vs streamed).

## 11. Recommendations

1. **Build `enth-py` first:** batched render, `ds1`, the front end, the compressor, `match`. Measure clips per second on the current main, with and without the reverb.
2. **Write `ds1`:** v1's keyed streams and priors at `amounts` 100, level 5, the §2 mixture, a phrase sampler, slot permutation, mask-based labels, note canonicalisation, a −60 LUFS reject, and the streams `r3/train`, `r3/dev` and `r3/test`. Versioned, with a hash golden.
3. **Freeze A, A′, B, C and dev** before training. Book the owner's ~30 minutes of phone re-recording.
4. **Data pipeline:**
   - a ≤ 70 GB clean cache with rolling refresh (check `df` first);
   - GPU-side stages 2–8 and 10–11;
   - codecs pre-baked offline by batched `ffmpeg` (LAME, native AAC, Opus);
   - shine, AMR, GSM, DEMAND and the MIT IRs held out for evaluation.
5. **Licences:** shipped weights train only on our renders plus NSynth, MUSAN, FSD50K's CC0 and CC BY subset, BUT ReverbDB, OpenSLR 28, TinySOL, and Freesound CC0 and CC BY. No pedalboard or gpuRIR imports, no fdk-aac. LGPL and GPL tools stay in `ml/`.
6. **Stack:** PyTorch 2.14 or 2.15 with CUDA 13.0 wheels, FP16 autocast, `uv.lock`, local logs, manifests, ONNX export plus a parity test.
7. **Where to train:** the laptop, overnight, under a memory cap. Rent a community RTX 4090 only if a run would exceed about 24 h locally.
8. **Real audio:** pseudo-labels from search first; audio-marginal training if report 01 picks a joint flow; listen to every batch.
9. **Owner decisions:**
   - the label policy for reverb, noise and compression on uploads;
   - GPL tools as subprocesses in `ml/`;
   - renting a GPU and its account;
   - whether a neural proxy counts as the banned "differentiable copy".
