# ml — R3b's Python side

The degradation chain, augmentation, the proposal model, training and the export to our model file ([docs/r3.md](../docs/r3.md), "R3b: the first model"). **It is never shipped and is in no cargo or CI build.** The lane plan and the rules below are in [docs/phase-3.md](../docs/phase-3.md); the owners are named in each package's docstring.

Lane S0s (R3, freeze point F0s) declared this skeleton and nothing else: `pyproject.toml` lists **no dependency**, none is installed or downloaded, and there is no `uv.lock`. The first lane that needs a package (DEG, AUG, TR) adds it to `pyproject.toml`, runs `uv sync` and commits the lock; the run manifests record its hash.

| Path | Holds | Lane |
| --- | --- | --- |
| `enth_ml/degrade/` | the seeded degradation chain; held-out pools disjoint from training's (tested); [DEG](#deg-the-degradation-chain) | DEG |
| `enth_ml/augment/` | augmentation and the data manifest (licences per source) | AUG |
| `enth_ml/model/` | the proposal network: K = 8 winner-takes-all hypotheses and a confidence head | TR |
| `enth_ml/train/` | training, with a manifest per run | TR |
| `enth_ml/export/` | ONNX and our model file (`enth-model-<id>.bin`), with a 20-input output golden | TR |
| `tests/` | `ml/`'s own tests (standard library only for the layout; lanes add theirs) | all |

Run every test with no installation: `python3 -m unittest discover -s tests` from `ml/` (the layout test needs `ml/` as the working directory; the DEG tests also run from the repository root with `-s ml/tests`). The DEG tests need the system `ffmpeg`, and skip without it.

## DEG: the degradation chain

A seeded chain that turns a clean WAV into a degraded WAV plus a JSON record of exactly what was applied (`enth_ml/degrade/`; lane DEG; design in [docs/r3.md](../docs/r3.md), "Augmentation and licences"). **Standard library plus the system `ffmpeg` binary (run as a subprocess; it is LGPL or GPL) and nothing else:** no package, no dataset, no impulse-response or noise file. Rooms, noises and hum are synthesised from seeds.

```
python3 -m enth_ml.degrade --in x.wav --out y.wav --seed 7 --pool heldout --record y.json
python3 -m enth_ml.degrade --in x.wav --out y.wav --seed 7 --pool heldout --profile mp3 --record y.json
python3 -m enth_ml.degrade --plan --pool heldout --profile phone-recording --seed 7   # the drawn parameters, no audio
python3 -m enth_ml.degrade --list                                                     # every pool, stage, variant, profile
```

Run it from `ml/`. `--pool` is `train` (the default) or `heldout`; `--profile` forces a named set of stages (below), `all` forces every stage, and without it a random chain is drawn (train keeps about 15 % of samples clean; held-out never does). `--record` writes the JSON record (default: on stdout), `--sample-fmt s16|s24|f32` picks the output format (default s16, at the input's sample rate), `--work-dir` keeps the float working file of every ffmpeg step. The input may be anything ffmpeg reads, mono or stereo. From Python: `degrade_file(src, dst, seed=, pool=, profile=)` returns the record; `make_plan(load_pools(), pool, seed, profile)` returns the plan alone.

### Stages

In this order (a real signal path); each stage is one or two ffmpeg calls on a 32-bit float working file, and Python measures levels between them where a stage is defined against one. The layer or mix-in stage of report 02 needs a second clip and belongs to lane AUG.

| Stage | ffmpeg filters | What a variant draws |
| --- | --- | --- |
| `attack_cut` | `atrim=start_sample`, `asetpts` | cut 0-50 ms into the attack (at most 30 % of the clip) |
| `tail_trunc` | `atrim=end_sample`, `afade` | keep 0.3-1.5 s (at most 85 % of the clip), then fade out |
| `lead_pad` | `adelay` | 0-300 ms of leading silence |
| `room` | `apad`, `afir` | a synthetic impulse response (`ir.py`): the direct impulse, early reflections, and a decaying noise tail whose high band (above `damp_hz`) decays faster; `wet_db` is the reverberant-to-direct energy ratio; the clip rings on for 0.67 x RT60 |
| `eq` | `highpass`, `lowpass`, `equalizer`, `lowshelf`, `highshelf` | high-pass and low-pass (2 or 4 poles), 1-3 peaks, a tilt as a shelf pair 300 Hz and 3 kHz |
| `noise` | `anoisesrc` (its `seed`), `highpass`, `lowpass`, `tremolo`, `amix` | a colour at an SNR against the clip's RMS (independent per channel) |
| `hum` | `aevalsrc`, `amix` | 50 or 60 Hz and harmonics at an SNR against the same RMS (the same on every channel) |
| `dynamics` | `volume`, `alimiter`, `asoftclip`, `acompressor` | the clip is brought to 0 dBFS peak, then `gain_db` above it goes into a limiter at -1 dBFS or a hard, tanh, atan or cubic clip; or a compressor (our own, through `enth-py`, is lane AUG's) |
| `mono` | `pan` | `mono = 0.5 (gL L + r gR R)`, an imbalance in dB and a partial polarity cancellation `r` |
| `band_limit` | `aresample` down and back | resample to 4-24 kHz and back (skipped when the clip's rate is not above it) |
| `codec` | `libmp3lame`, `libshine`, `aac`, `libopus`, `libopencore_amrnb`, `libgsm`, then the decoder, `atrim`, `apad` | encode and decode back to the clip's rate, channels and length; ffmpeg compensates the gapless delay of LAME, AAC and Opus; AMR-NB and GSM run at 8 kHz mono and come back to the clip's channels |
| `bit_depth` | `aeval`, `anoisesrc` (TPDF dither), `pcm_mulaw`, `pcm_alaw` | quantise to 5-12 bits with or without dither, or 8-bit mu-law or A-law |
| normalise | `ebur128` (measure), `volume` | one linear gain to **-23 LUFS**, always; the peak is kept at or below -0.1 dBFS (recorded as `peak_limited` in the rare case that costs loudness) |

Short sounds are valid: nothing is filtered by duration. A cut or fade is clamped to a share of the clip, and a clip under 400 ms (no BS.1770 gating block) is measured looped to 1.5 s, which reads its mean power as enth's meter does.

### The pools

`enth_ml/degrade/pools.toml` is data: for each pool, each stage has a probability `p` and named **variants**, and a variant is a table of parameters (a constant, `[lo, hi]`, `{ log = [lo, hi] }` or `{ choice = [...] }`; every variant has a `kind`). Other lanes reference a setting by name: `heldout/codec/shine-cbr`, or `heldout/profile/mp3`. `--list` prints them all; the file's `sha256` is in every record.

| | `train` | `heldout` (evaluation only) |
| --- | --- | --- |
| Rooms | small, medium and large, RT60 0.15-0.35, 0.7-1.0, 1.5-1.9 s | living room, hall, cathedral, RT60 0.4-0.6, 1.1-1.4, 2.0-2.5 s; a **flutter echo** generator training never has |
| EQ | `device`: high-pass 80-400 Hz, low-pass 3-12 kHz, 1-3 peaks of +-9 dB, tilt +-3 dB/oct | `phone-speaker` (420-800 Hz high-pass, 2.4-3 kHz low-pass, a presence peak), `laptop-box`, tilts of 3.5-6 dB/oct |
| Noise | white, pink, brown at 15-50 dB SNR | blue, violet, velvet at 5-20 dB; an amplitude-modulated, band-limited pink "babble" at 5-14.9 dB |
| Hum | 2-4 harmonics falling as k^-0.8..2, 30-60 dB SNR | 6-12 harmonics, flat to k^-0.6, 15-29 dB SNR |
| Dynamics | limiter, hard clip, tanh clip at +3 to +12 dB; compressor at ratio 2-6 | limiter and hard clip at +13 to +18 dB; atan and cubic clips at +6 to +18 dB; compressor at ratio 8-20 |
| Mono | imbalance 1-6 dB, polarity 0.3-1 | the plain downmix; a downmix that cancels part of the centre |
| Band limit (the rate resampled to and back) | 8-24 kHz | 4-7 kHz |
| Codec | LAME CBR 32-128 kbps; AAC-LC (twoloop coder) 32-96; Opus (audio) 12-64 | **shine** CBR 32-96 kbps; LAME VBR (V5-V9) and ABR 32-96; AAC (fast coder) 24-64; Opus (voip) 6-24; **AMR-NB** 4.75-12.2 kbps; **GSM** |
| Bit depth | 8-12 bits, mu-law | 5-7 bits, A-law |
| Trim and pad | cut 0-50 ms, keep 0.3-1.5 s, pad 0-300 ms | cut 51-120 ms, keep 1.6-2.4 s, pad 320-700 ms |

**Disjoint, and tested** (`tests/test_degrade_pools.py`). Two variants of a stage are disjoint when some parameter they share has intervals or sets that do not meet; `kind` (the generator, the encoder and its mode, the noise colour) is such a parameter, so a held-out noise colour or encoder is disjoint from training's whatever the other numbers are. The test proves every held-out variant is disjoint from every training variant of its stage, that the checker catches an overlap (it is not vacuous), that every drawn parameter lies inside its variant's domain and outside every variant of the other pool, and that instance seeds (noise sources, impulse responses, dither) carry the pool in their lowest bit, so the pools cannot draw the same instance. This is a rule about **settings**, not about audio: a held-out RT60 range may interleave with training's, as long as the bands do not meet.

**Profiles** force a set of stages on, in stage order, and may restrict the variants. The same names exist in both pools, so a held-out result has an in-distribution control from the training pool's version. Single families: `mp3`, `noise-hum`, `reverb`, `clip-limit`, `eq`, `mono`, `trim`, `band-bit`. Compound chains, for uploads: `upload-noisy` (noise, hum, codec), `upload-room` (room, codec), `upload-loud` (dynamics, codec), `phone-recording` (room, phone-speaker EQ, mono, a phone codec). Held-out only: `eq-other`, `codec-other`, `phone-codec`. The held-out MP3 range follows docs/r3.md (32-96 kbps); report 04's four measured degradations are `mp3`, `noise-hum`, `reverb` and `clip-limit`.

### The record

`enth.degrade-record/1` (the chain's `CHAIN_VERSION` is 1; a change to a stage's behaviour bumps it): the seed, pool and profile, the `sha256` of `pools.toml`, **the first line of `ffmpeg -version`**, the input's and output's SHA-256, format and length, and per stage whether it was drawn, its variant, the exact drawn parameters, whether it ran (a mono stage on a mono input is recorded as skipped, with the reason), what was measured (SNR reference level, gains, the impulse response's SHA-256, coded size), the ffmpeg arguments of every call (paths replaced by `{in}`, `{out}`...) and the SHA-256 of the stage's output. No local path or personal detail is recorded.

### Determinism

The same input, seed, pool and profile give the same output bytes **on this machine with this ffmpeg build** (`tests/test_degrade_chain.py` runs a chain twice and compares SHA-256s, in-process and through the command line). Parameters come from SplitMix64 streams derived by SHA-256 from `(seed, pool, stage)`, so they are identical everywhere, and every drawn float is rounded to six significant digits before it is recorded and used. The audio bytes also depend on ffmpeg's float code: **a different ffmpeg build (an encoder or resampler version, or a CPU whose SIMD paths differ in the FFT) may change the bytes**, which is why every record names the build. ffmpeg runs with one thread and bit-exact output headers.

### Limits and open items

- Codec time alignment is checked, not exact: LAME, AAC and Opus land within a few samples; **shine** within about 10 ms at its lowest rates (its delay is not trimmed by ffmpeg); AMR-NB and GSM within about 20 ms (their look-ahead).
- ffmpeg's `acompressor` stands in for our own compressor; AUG swaps it when `enth-py` exposes one.
- The held-out chain has no rooms from real collections (the MIT IR Survey, DEMAND): synthetic rooms and noises only, by the no-download rule. The report 02 sets C can add them later as files beside this chain.
- One clip takes 0.3-1.7 s (up to about 20 ffmpeg calls, one or two per active stage). Baking about 100,000 codec variants for training needs batching (concatenate clips, or call the codec stage alone); that belongs to AUG.
- Tests: `python3 -m unittest discover -s tests` from `ml/` (about a minute).

## The ML rules

From [docs/phase-3.md](../docs/phase-3.md#rules-for-every-lane) ("ML rules" and "Held-out hygiene"); they bind every lane that writes here.

- **`ml/` is in no cargo or CI build and never ships.** The root `Cargo.toml` excludes it by name, `ci.yml` ignores changes to it (so a Python-only change runs no workflow), no workflow runs `uv`, and `tests/test_layout.py` checks all of it. A Python-only lane runs `ml/`'s own tests instead of `scripts/gate.sh`.
- **Only MIT or Apache code goes into it.** The copy rule holds here although nothing ships; MPL code only with the owner's OK.
- **LGPL and GPL tools run only as subprocesses.** LAME, shine and ffmpeg are never imported or linked. **pedalboard (GPL-3.0), gpuRIR (AGPL-3.0) and fdk-aac are never imported.**
- **Shipped weights train only on our renders plus CC0, CC BY or Apache data.** NC, SA and licence-less sets stay evaluation-only and local. A source's licence is read from the source and recorded in the data manifest.
- **Features are computed in Rust** (`enth-analysis`, through files `tools/datagen` writes), **never re-implemented in Python.**
- **Logs stay local.** No telemetry, no experiment tracker that uploads, no network code.
- **Held-out hygiene.** The GR3 sets (`content/gr3/`) freeze with their hashes before any training; tuning happens on dev only; every test run is appended to `content/gr3/runs.jsonl` and none is dropped.
- **Privacy.** Never put the owner's email, name or other personal details in any request, header, URL or payload; where an API asks for contact info use `enth-research/0.1`. A dataset that needs an account (Freesound's originals) is fetched by the owner, not by an agent.

Decisions that are the owner's, with the defaults ([phase-3.md](../docs/phase-3.md#your-part)): no learned neural proxy (it is close to the banned differentiable copy of the engine), no rented GPU (the laptop only), no MPL code.

## Compute

PyTorch with uv. The laptop's GPU runs training overnight with FP16 autocast, **attended the first time, never beside a heavy build, under a memory cap** (`systemd-run --user -p MemoryMax=6G`), and a training run uses the machine alone ([phase-3.md](../docs/phase-3.md#waves)). A training cache stays under about 70 GB and outside the repository (`.gitignore` keeps caches, checkpoints, audio and exported models out). Training is not bit-reproducible (GPU kernels): the artefact that must be reproducible is the hashed model file and its output golden.
