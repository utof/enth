# ml — R3b's Python side

The degradation chain, augmentation, the proposal model, training and the export to our model file ([docs/r3.md](../docs/r3.md), "R3b: the first model"). **It is never shipped and is in no cargo or CI build.** The lane plan and the rules below are in [docs/phase-3.md](../docs/phase-3.md); the owners are named in each package's docstring.

Lane S0s (R3, freeze point F0s) declared this skeleton and nothing else: `pyproject.toml` lists **no dependency**, none is installed or downloaded, and there is no `uv.lock`. The first lane that needs a package (DEG, AUG, TR) adds it to `pyproject.toml`, runs `uv sync` and commits the lock; the run manifests record its hash.

| Path | Holds | Lane |
| --- | --- | --- |
| `enth_ml/degrade/` | the seeded degradation chain; held-out pools disjoint from training's (tested) | DEG |
| `enth_ml/augment/` | augmentation and the data manifest (licences per source) | AUG |
| `enth_ml/model/` | the proposal network: K = 8 winner-takes-all hypotheses and a confidence head | TR |
| `enth_ml/train/` | training, with a manifest per run | TR |
| `enth_ml/export/` | ONNX and our model file (`enth-model-<id>.bin`), with a 20-input output golden | TR |
| `tests/` | `ml/`'s own tests (standard library only for the layout; lanes add theirs) | all |

Run the layout test with no installation: `python3 -m unittest discover -s tests` from `ml/`.

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
