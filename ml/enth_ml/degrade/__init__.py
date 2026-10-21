"""The degradation chain (lane DEG, R3): a seeded chain from a clean WAV to a degraded WAV and a JSON record.

The chain follows a real signal path (docs/r3.md, "Augmentation and licences"): trim and pad, a synthetic room, device
EQ, noise and hum, dynamics, stereo to mono, band limit, a codec, bit depth, then normalisation to -23 LUFS. The same
input, seed and pool give the same bytes (on one machine with one ffmpeg build), and the two pools in ``pools.toml``
(``train`` and ``heldout``) are disjoint, which a test asserts. LGPL and GPL encoders (LAME, shine, ffmpeg) run as
subprocesses only. Degraded synth inputs are scored against the clean render.

    python -m enth_ml.degrade --in x.wav --out y.wav --seed 7 --pool heldout --profile mp3 --record y.json

Standard library plus the system ``ffmpeg`` only. See ``ml/README.md`` ("DEG") for the stages, the pools and the names
lane DS's manifests reference.
"""

from .chain import CHAIN_VERSION, RECORD_SCHEMA, degrade_file, make_plan
from .pools import POOLS_PATH, check_disjoint, load_pools

__all__ = [
    "CHAIN_VERSION",
    "POOLS_PATH",
    "RECORD_SCHEMA",
    "check_disjoint",
    "degrade_file",
    "load_pools",
    "make_plan",
]
