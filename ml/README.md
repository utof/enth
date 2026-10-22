# ml — R3b's Python side

The degradation chain, augmentation, the proposal model, training and the export to our model file ([docs/r3.md](../docs/r3.md), "R3b: the first model"). **It is never shipped and is in no cargo or CI build.** The lane plan and the rules below are in [docs/phase-3.md](../docs/phase-3.md); the owners are named in each package's docstring.

Lane S0s (R3, freeze point F0s) declared this skeleton and nothing else: `pyproject.toml` lists **no dependency**, none is installed or downloaded, and there is no `uv.lock`. The first lane that needs a package (DEG, AUG, TR) adds it to `pyproject.toml`, runs `uv sync` and commits the lock; the run manifests record its hash.

| Path | Holds | Lane |
| --- | --- | --- |
| `enth_ml/degrade/` | the seeded degradation chain; held-out pools disjoint from training's (tested); [DEG](#deg-the-degradation-chain) | DEG |
| `enth_ml/augment/` | augmentation on top of DEG's chain (recorded rooms and noises, microphones, levels, rates, layers) and the data manifest (a licence per source, refused for NC and SA); [AUG](#aug-augmentation-and-the-data-manifest) | AUG |
| `data/fetch.py` | the one script that downloads a dataset named in the manifest, by hand: robots.txt, one request, hash, safe extraction | AUG |
| `enth_ml/model/` | the proposal network: K = 8 winner-takes-all hypotheses and a confidence head | TR |
| `enth_ml/train/` | training, with a manifest per run | TR |
| `enth_ml/export/` | ONNX and our model file (`enth-model-<id>.bin`), with a 20-input output golden | TR |
| `tests/` | `ml/`'s own tests (standard library only for the layout; lanes add theirs) | all |

Lane ONX (R3b) added the first dependency group, `parity` (`onnx`, `onnxruntime`, `numpy`; [ONX](#onx-the-model-file-writer-and-onnx-runtime-parity) below), and committed `uv.lock`; `[project] dependencies` stays empty, so the standard-library run below needs nothing installed.

Run every test with no installation: `python3 -m unittest discover -s tests` from `ml/` (the layout test needs `ml/` as the working directory; the DEG tests also run from the repository root with `-s ml/tests`). The DEG and AUG tests need the system `ffmpeg`, and skip without it. The whole suite (262 tests) takes 3.5 to 6 minutes, depending on what else the machine is building; one lane's tests alone: `python3 -m unittest discover -s tests -p 'test_augment_*.py'`.

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

## AUG: augmentation and the data manifest

Lane AUG (`enth_ml/augment/`, `data/fetch.py`; design in [docs/r3.md](../docs/r3.md), "Augmentation and licences") puts DEG's chain under a config that adds what needs **recordings or a second clip**, and holds the **data manifest**: one entry per source with its licence, URL, version, hash and the use it allows. **Standard library plus the system `ffmpeg` binary**; the package has no network code (the one script that has is `data/fetch.py`, below), and the only data on this machine is OpenSLR 28 (1.3 GB, fetched on 2026-10-22).

```
python3 -m enth_ml.augment --in x.wav --out y.wav --seed 7 --record y.json                    # a random chain
python3 -m enth_ml.augment --in x.wav --out y.wav --seed 7 --profile recorded-room            # a recorded room, forced
python3 -m enth_ml.augment --in x.wav --out y.wav --seed 7 --profile all --layer other.wav    # every stage, a second render under it
python3 -m enth_ml.augment --plan --seed 7 --profile mic        # the drawn parameters, no audio (what is on disk decides the variants)
python3 -m enth_ml.augment --list | --check | --manifest        # stages and sources | are the manifest and config sound | licence table
python3 -m enth_ml.augment --index openslr-28/sim-small         # hash a collection, write its index, print the digest to pin
python3 -m enth_ml.augment --sample-seed RUN_SEED SAMPLE_ID [EPOCH]
python3 data/fetch.py --list | --verify openslr-28 | SOURCE --yes [--research] [--allow-large]
```

From Python: `augment_file(src, dst, seed=, config=, store=, layers=, profile=, partition=)` returns the record; `make_plan(config, load_pools(), seed, profile)` the plan alone; `sample_seed(run_seed, sample_id, epoch)` the seed of one training sample. `--data-dir` (or `ENTH_DATA_DIR`) says where the collections are; the default is the manifest's `local_root`.

| File | Holds |
| --- | --- |
| `manifest.toml`, `manifest.py` | the data manifest (`enth.data-manifest/1`) and the checks: `check_manifest`, `require_shipped` (the gate lane TR calls before it trains shipped weights) |
| `augment.toml`, `augment-eval.toml` | the training config (`enth.augment-config/1`, purpose `shipped-training`, DEG's `train` pool) and the held-out one (purpose `eval`, DEG's `heldout` pool); versioned, their SHA-256 is in every record |
| `config.py`, `plan.py` | loading and `check_config`; the merged seeded plan |
| `sources.py` | a hashed index per collection, the train/dev split by path hash, GR3's exclusion lists |
| `rir.py`, `stages.py`, `chain.py`, `__main__.py` | recorded impulse responses made ready for `afir`; the six stages; `augment_file`; the command line |
| `../data/fetch.py` | the downloader |

### How a sample is augmented

`make_plan` draws **DEG's plan untouched** (the same streams, the same clean 15 % of train samples) and adds this package's stages from streams of their own (`derive("augment", "stage", seed, pool, stage)`), so adding a stage or a variant never moves another stage's numbers. The merged order is a signal path: `mix_in`, `attack_cut`, `tail_trunc`, `lead_pad`, `room` | `rir`, `mic`, `eq`, `noise` | `bg_noise`, `hum`, `dynamics`, `mono`, `gain`, `band_limit`, `codec`, `bit_depth`, `rate`, then -23 LUFS. A clean sample stays clean.

| Stage | Draws | How |
| --- | --- | --- |
| `mix_in` (p 0.15) | a second sound 6-20 dB under the clip, 0-300 ms late: `layer` (a second render the caller passes, `--layer`) or `layer-recorded` (a real instrument note: TinySOL) | RMS ratio over the clip's length; resampled and channel-matched; the noise and hum that follow measure their SNR against the clip *with* its layer |
| `rir` (replaces half of DEG's active `room` stages, so the chance of a room stays 0.3) | a **recorded or simulated room**: OpenSLR 28's simulated RIRs (small, medium, large rooms), BUT ReverbDB; `wet_db` -20..0 | the recording becomes DEG's shape: the first sample reaching a quarter of the peak is the onset; the direct window (1 ms before to 2 ms after) is **replaced by a unit impulse** so the recording's own colour never filters the clip; the tail is cut where the energy-decay curve passes -45 dB (4 s at most), resampled if the rates differ (the direct part never is), and scaled to energy `wet_db` below the direct; RT60 is measured (T20, else T10) and recorded with its fit's R squared; one channel of a multichannel recording per clip channel, a mono one on both |
| `mic` (p 0.25) | a microphone's colour: high-pass 30-380 Hz, proximity shelf, a presence peak, an air shelf, low-pass 5-20 kHz, for four mic types | `highpass`, `lowshelf`, `equalizer`, `highshelf`, `lowpass`; every high-pass lies below the held-out speaker EQs' (420 Hz up); a component above 0.45 of the sample rate is left out |
| `bg_noise` (replaces 0.6 of DEG's active `noise` stages) | a **recorded background noise** (OpenSLR 28's point-source noises, the Freesound portion of MUSAN) at 15-50 dB SNR | a random excerpt (looped if the noise is shorter than the clip) at the exact SNR against the clip's RMS, measured on the files |
| `gain` (p 0.3) | the recording level: the clip's peak at -45..-3 dBFS before the band limit, codec and bit depth | so the quantiser, the codec and the final normalisation's boost act on a quiet or a hot recording |
| `rate` (p 0.15) | the file's own sample rate: 8, 11.025, 16, 22.05, 32, 44.1 or 96 kHz | a proper resample, pitch and duration kept; none lies in the held-out 4-7 kHz band-limit range |

`--profile` forces stages on (and DEG's off unless the profile names a DEG one): `layer`, `recorded-room`, `recorded-noise`, `mic`, `levels` (`gain` and `rate`), `recorded-room-mp3`, `recorded-noise-mp3`; any DEG profile passes through; `all` runs every stage of both chains, the recorded room and noise stacked on the synthetic ones. A profile whose stage has nothing to run (no collection fetched) is refused, naming why; the random chain just leaves that variant out and renormalises the weights, and the plan lists what it left out (`unavailable`).

**Recorded files are chosen by fraction, not name**: the plan holds `u_file`, `u_chan`, `u_start` in [0, 1) and the store maps `u_file` to a file of the collection's **hashed index** when the stage runs (so a new file in a folder changes nothing until the index is rewritten, and a changed file is refused: its SHA-256 is checked at every pick). About 5 % of each collection's files, by a hash of the path, are the `dev` partition, which a validation augmentation draws from and training never does (`--partition dev`); an `eval` config draws from all of a held-out collection. Files GR3 excludes (`content/gr3/exclude-*.txt`: by SHA-256, Freesound id or pack, path) are never picked.

**Seeds.** `sample_seed(run_seed, sample_id, epoch)` is a pure function: the same sample gets a new augmentation every epoch and the same one on every re-run. It is below 2**63 and **never inside GR3's seed block** (770000000000..770999999999: the held-out set's seeds); a hash that lands there is hashed again, and `make_plan` and `augment_file` refuse such a seed. A test holds the constant against `exclude-synth.txt`.

### Labels: damage, or dry and wet

Per the degradation policy (docs/r3.md, "Yours to decide", proceeded on): codec, hum, noise, device or microphone EQ, clipping and the rest are **damage**: the label is the clean render's patch. A **reverb or a compressor is ambiguous**, because the patch has its own, and a pad sampled with its reverb is not damaged. So a record whose chain applied a `room`, a `rir` or a compressor says `label.policy = "dry-and-wet"` and lists each under `label.wet_candidates` with its numbers (`rt60_s`, `wet_db`, `predelay_ms`, or `ratio`, `thr_below_peak_db`, `attack_ms`, `release_ms`, `makeup_db`; a recorded room adds `tail_s` and `rt60_fit_r2`). The dry label is the clean render's patch; lane TR decides, per its loss, whether to train on the dry label, on a wet one (the patch with that reverb or compressor set) or on either. Limiters and clippers are not the engine's effects and stay damage.

### The record

`enth.augment-record/1` (the chain's `CHAIN_VERSION` is 1) is DEG's record plus: `purpose`, `partition`, the config's name, version and SHA-256, the manifest's SHA-256, GR3's exclusion lists' SHA-256, `unavailable`, `layers_supplied`, per stage `sample_rate` (the `rate` stage changes it), `sources_used` (**every recorded file: its reference, path inside the collection, SHA-256, licence and allowed use**), the `label` block above, and **`shipped_ok`**: true when no source used is anything but `shipped-training` and the config is not an `eval` one. Lane TR keeps a sample out of a shipped-weights run when it is false. The clip's own licence is the caller's: a render of ours is fine, a real recording needs its own check. The record holds no path of this machine.

### The data manifest and what the licences say

`manifest.toml` has a **licence read at the source** for every collection (the date, the page, what it says), a URL, a version, the archive's size and SHA-256 once fetched, and a `use`: `shipped-training` (CC0, CC BY, Apache), `research-only` (evaluation or experiments, local) or `excluded`. A part of a source may tighten the source's use, never loosen it, and a source with parts is only as free as its least free part. `check_manifest` refuses a `shipped-training` entry whose licence is NC, SA, ND, Sampling+, unknown, unstated or not on the allowlist; `require_shipped(refs, manifest)` is the gate (it raises `LicenceError`); `check_config` runs it over every source a training config names, and `pick_source` runs it again at every file read. `python3 -m enth_ml.augment --manifest` prints the table. Findings that closed the open check (docs/phase-3.md):

| Collection | Licence, as read | Use |
| --- | --- | --- |
| [OpenSLR 28](https://www.openslr.org/28/) (RIRS_NOISES, 1.3 GB, **fetched**) | Apache-2.0 for the archive. Its parts: the 3 x 20,000 **simulated RIRs** are the authors' own; the 843 **point-source noises** are MUSAN's Freesound portion, whose `LICENSE` says all were marked Public Domain on Freesound (recorded as CC0; the owner may veto that reading); the **real RIRs** come from three databases with their own terms | simulated RIRs and point-source noises: `shipped-training`. Real RIRs: AIR (MIT licence file, outside the CC0/CC BY/Apache allowlist), RWCP ("research and development purposes only" at NII and OpenSLR 13) and REVERB 2014 ("available only through LDC"): `research-only` |
| [BUT ReverbDB](https://speech.fit.vut.cz/software/but-speech-fit-reverb-database) | CC BY 4.0 (page; `read_me.txt` header Apache-2.0) | `shipped-training`; the RIR-only archive is 9.3 GB, over the 3 GB budget: `fetch.py` is ready, `--allow-large` |
| [MUSAN](https://www.openslr.org/17/) | CC BY 4.0 for the archive; per-folder `LICENSE` files (paper: only public-domain or commercial-use CC material) | `noise-free-sound` (Public Domain) `shipped-training`; `noise-sound-bible` and `music` wait for a per-file audit: `research-only` |
| [NSynth](https://magenta.tensorflow.org/datasets/nsynth) | CC BY 4.0 | `shipped-training` |
| [FSD50K](https://zenodo.org/records/4060432) | dataset CC BY, **per-clip licences**: CC0 and CC BY clips ship; CC BY-NC (6,041) and Sampling+ (1,777) do not. The record says to contact the authors for commercial use | split by clip licence; clip audio not fetched |
| [TinySOL](https://zenodo.org/records/3685367) | CC BY 4.0 | `shipped-training` |
| [Freesound](https://freesound.org/help/faq/) | per-sound CC. **Training terms** (FAQ, read 2026-10-22): training is fine "as long as individual sounds' Creative Commons licenses are respected": credit CC BY sounds, disclose the training set for CC BY, leave CC BY-NC out of a commercial model; bulk access is the Data Packs Portal (a contact form), not the API, whose terms (4f) bar robots and scraping, and `robots.txt` disallows `/search/`, `/api/`, `/apiv2/` | CC0 and CC BY: `shipped-training`; NC and Sampling+: `research-only`; nothing downloaded; GR3's 48 references are excluded |
| [MIT IR Survey](https://mcdermottlab.mit.edu/Reverb/IR_Survey.html), [DEMAND](https://zenodo.org/records/1227121), [ESC-50](https://github.com/karolpiczak/ESC-50) | no licence stated; CC BY-SA 3.0 (the description; the metadata says CC BY 4.0, read as SA); CC BY-NC 3.0 | held-out evaluation collections, `research-only`: never training, and the only recordings `augment-eval.toml` may name |
| [WHAM!](https://web.archive.org/web/20250114184610/http://wham.whisper.ai/) noise, [OpenAIR](https://www.openairlib.net/) | CC BY-NC 4.0; per-upload (some all rights reserved), and the site was unreachable | `excluded`: not fetched, not used, not even for evaluation |
| AMR-NB (`libopencore-amrnb`), GSM (`libgsm`), AMR-WB (`libvo-amrwbenc`) | Apache-2.0 with a 3GPP note and "not a grant of patent rights"; a permissive TU Berlin notice; Apache-2.0 (read from the installed Debian packages' `copyright`, since SourceForge's `robots.txt` disallows the project pages) | used as subprocesses, in the held-out pool only (`heldout/codec/amr-nb`, `gsm`): no shipped-training sample passes through them; AMR-WB is not drawn |
| LAME, shine, Opus, ffmpeg; pedalboard, gpuRIR, fdk-aac | LGPL-2+, LGPL-2, BSD-3, GPL-2+ (as built); GPL-3.0, AGPL-3.0, no patent licence | subprocesses only; the last three are never imported or used |

### Disjoint from GR3's held-out set, and tested

`tests/test_augment_disjoint.py` extends DEG's check to everything this package adds. (1) Each variant of `augment.toml` is disjoint, by DEG's own rule, from every related held-out variant (`rir` and `room`, `mic` and `eq`, `bg_noise` and `noise`, `rate` and `band_limit`), and the microphones and rates also differ from the held-out EQs and band limits **by a number in a parameter both define** (a stricter rule than `kind`; its test has teeth). (2) `augment-eval.toml` differs from `augment.toml` the same way, stage by stage. (3) No training config can name a collection the manifest holds out (it is refused, for every held-out reference, under every training purpose), and an `eval` config can name nothing else. (4) Held against the plans: no training plan, over thousands of seeds and every profile, shows a pool, variant or profile that `content/gr3/exclude-degraded.txt` lists, or a seed in a reserved range; instance seeds keep each pool's parity bit; DEG's draws are unchanged. A recorded room is a new generator (`kind = "recorded"`), disjoint from the held-out *synthetic* rooms by DEG's rule, but its RT60 is whatever the recording has, including inside the held-out bands: the held-out room test measures a new generator, not a new RT60.

### `data/fetch.py`: the one script with network code

`python3 data/fetch.py SOURCE --yes` downloads one archive named in the manifest. It refuses an excluded source, a source that cannot train shipped weights (unless `--research`), an archive over 3 GB (unless `--allow-large`), and anything without `--yes` (it prints the licence and where it was read first). It reads the host's `robots.txt` before anything else (own RFC 9309 parser; obeys `Disallow` and `Crawl-delay`; re-checks every redirect), sends `User-Agent: enth-research/0.1` and no other identifying header, makes one request at a time at least 2 s apart, resumes a `.part` with `Range`, never retries after 403, 404, 429 or 5xx, checks size and SHA-256 against the manifest (a first fetch with no pinned hash prints the digest and stops), extracts with no absolute path, no `..` and no link or device, capped in size and file count, into a fresh directory, deletes nothing, and prints each part's index digest next to the pinned one. `--verify` re-hashes without the network. Its tests (`tests/test_augment_fetch.py`) run it against a server on 127.0.0.1 only. Downloaded data is untrusted: nothing in `enth_ml` opens an archive; read extracted files with `python3 -I` from another directory.

### Limits and open items

- **`enth-py` has no compressor yet**, so the `dynamics` compressor is still ffmpeg's `acompressor` (DEG's stand-in). AUG swaps it when `enth-py` exposes one; the wet candidate already carries its parameters.
- **Not fetched** (budget, or a person must): BUT ReverbDB (9.3 GB), MUSAN (11 GB), NSynth, TinySOL, FSD50K, Freesound, and the held-out MIT IR Survey and DEMAND. Their variants drop out of the plan until the collection is on disk; a run with only OpenSLR 28 uses the simulated rooms and the point-source noises.
- **Owner questions**: whether to admit the MIT licence (AIR) to the allowlist (it is kept `research-only`); the reading of the point-source noises as CC0; whether to ask Freesound for a Data Packs download and FSD50K's authors about commercial use; a per-file audit of MUSAN's other folders.
- `rate` makes the output file's sample rate vary (8-96 kHz); the trainer decides whether to feed that to the features as is or resample back.
- Tests: `python3 -m unittest discover -s tests -p 'test_augment_*.py'` (a little over half of the suite's time).

## ONX: the model file writer and ONNX Runtime parity

Lane ONX (R3b, our scalar kernels; docs/r3.md, "Our own scalar kernels (lane ONX)") added four modules to `enth_ml/export/` and one script:

| Module | What | Needs |
| --- | --- | --- |
| `enthmodl.py` | `ModelWriter`: `ENTHMODL` format 1 exactly as `pack.rs` reads it (header, tables, tensors at multiples of 64, the SHA-256). `f32` weights below the smallest normal are flushed to 0, `f16` tensors are rounded to nearest even. It reproduces `pack.rs`'s pinned format-1 fixture byte for byte, and its op table is checked against `pack.rs`'s | standard library |
| `tensors.py` | `ENTHTNS1`, a raw tensor file: expected outputs, and inputs for `tools/model-check --inputs` | standard library |
| `synthetic.py` | deterministic inputs and weights from one SplitMix64 stream, bit for bit as `propose::net::synthetic_input`, so a fixture records a seed instead of a 98 KB input | standard library |
| `onnx_import.py` | ONNX (batch axis 1) to `ENTHMODL`, op by op (the table in its docstring); folds a `BatchNormalization` into the convolution it follows (the folded weight stays `f32`) and turns `Mul(x, Sigmoid(x))` into `Silu`; refuses anything outside op set 1 | `parity` group |
| `parity.py` | the parity fixtures: four ONNX graphs built with `onnx.helper` (every op of op set 1, and a CNN with an attention block), run through ONNX Runtime on one thread with graph optimisation off, written as `<name>.bin` and `<name>.expected` | `parity` group |

```
uv run --group parity python -m enth_ml.export.parity --out ../crates/enth-solver/tests/fixtures/propose
uv run --group parity python -m unittest discover -s tests -p 'test_export_*.py'
```

The fixtures are committed (about 55 KB in all); `enth-solver`'s `tests/propose_parity.rs` compiles them in and checks our kernels against ONNX Runtime within 1e-5 on every CI leg (measured: at most 3.6e-7), and `tools/model-check run <name>.bin --expected <name>.expected` does the same by hand for any model. Licences: `onnx` Apache-2.0, `onnxruntime` MIT, `numpy` BSD-3-Clause (its wheels bundle OpenBLAS, BSD-3-Clause, and the GCC runtime under its library exception), and the transitive `protobuf` (BSD-3-Clause), `flatbuffers`, `ml-dtypes` and `packaging` (Apache-2.0) and `typing-extensions` (PSF-2.0): all permissive, imported here only, never shipped; nothing is copied into `ml/`.

**For TR:** export through `ModelWriter` (or check your writer against `tests/test_export_enthmodl.py`'s pinned fixture). Run parity against the weights as the file holds them: `f16`-rounded, and a folded `BatchNorm` rounded **after** folding (rounding before folding and again after cost 3e-5 here, three times the tolerance, until the converter kept folded weights in `f32`).

## The ML rules

From [docs/phase-3.md](../docs/phase-3.md#rules-for-every-lane) ("ML rules" and "Held-out hygiene"); they bind every lane that writes here.

- **`ml/` is in no cargo or CI build and never ships.** The root `Cargo.toml` excludes it by name, `ci.yml` ignores changes to it (so a Python-only change runs no workflow), no workflow runs `uv`, and `tests/test_layout.py` checks all of it. A Python-only lane runs `ml/`'s own tests instead of `scripts/gate.sh`.
- **Only MIT or Apache code goes into it.** The copy rule holds here although nothing ships; MPL code only with the owner's OK.
- **LGPL and GPL tools run only as subprocesses.** LAME, shine and ffmpeg are never imported or linked. **pedalboard (GPL-3.0), gpuRIR (AGPL-3.0) and fdk-aac are never imported.**
- **Shipped weights train only on our renders plus CC0, CC BY or Apache data.** NC, SA and licence-less sets stay evaluation-only and local. A source's licence is read from the source and recorded in the data manifest.
- **Features are computed in Rust** (`enth-analysis`, through files `tools/datagen` writes), **never re-implemented in Python.**
- **Logs stay local.** No telemetry, no experiment tracker that uploads, no network code in the package. The one exception is `data/fetch.py`, a by-hand downloader of public datasets (it is never imported, and only a person starts it; see AUG above).
- **Held-out hygiene.** The GR3 sets (`content/gr3/`) freeze with their hashes before any training; tuning happens on dev only; every test run is appended to `content/gr3/runs.jsonl` and none is dropped.
- **Privacy.** Never put the owner's email, name or other personal details in any request, header, URL or payload; where an API asks for contact info use `enth-research/0.1`. A dataset that needs an account (Freesound's originals) is fetched by the owner, not by an agent.

Decisions that are the owner's, with the defaults ([phase-3.md](../docs/phase-3.md#your-part)): no learned neural proxy (it is close to the banned differentiable copy of the engine), no rented GPU (the laptop only), no MPL code.

## Compute

PyTorch with uv. The laptop's GPU runs training overnight with FP16 autocast, **attended the first time, never beside a heavy build, under a memory cap** (`systemd-run --user -p MemoryMax=6G`), and a training run uses the machine alone ([phase-3.md](../docs/phase-3.md#waves)). A training cache stays under about 70 GB and outside the repository (`.gitignore` keeps caches, checkpoints, audio and exported models out). Training is not bit-reproducible (GPU kernels): the artefact that must be reproducible is the hashed model file and its output golden.
