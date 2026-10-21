"""Helpers shared by the DEG tests: a synthetic input WAV (no data files), level measures, and a seed finder.

Standard library only; ``ffmpeg`` is the system binary. Importing this module puts ``ml/`` on ``sys.path`` so the tests
run from the repository root or from ``ml/``.
"""

from __future__ import annotations

import array
import math
import pathlib
import re
import shutil
import subprocess
import sys
import wave

ML = pathlib.Path(__file__).resolve().parents[1]
if str(ML) not in sys.path:
    sys.path.insert(0, str(ML))

from enth_ml.degrade import ffmpeg, load_pools, make_plan  # noqa: E402

HAVE_FFMPEG = shutil.which("ffmpeg") is not None


def have_encoder(name: str) -> bool:
    return HAVE_FFMPEG and name in ffmpeg.encoders()


def write_pluck(path: pathlib.Path, *, channels: int = 2, sr: int = 48000, dur: float = 0.6, f0: float = 220.0) -> None:
    """A decaying three-partial tone, panned a little to the left in stereo: stands in for a synth render."""
    n = int(dur * sr)
    pcm = array.array("h")
    for i in range(n):
        t = i / sr
        env = math.exp(-5.0 * t) * (1.0 - math.exp(-300.0 * t))
        v = env * (
            0.5 * math.sin(2 * math.pi * f0 * t)
            + 0.25 * math.sin(2 * math.pi * 2 * f0 * t + 0.3)
            + 0.1 * math.sin(2 * math.pi * 5 * f0 * t)
        )
        for c in range(channels):
            pcm.append(int(v * (1.0 if c == 0 else 0.8) * 30000))
    _write_s16(path, channels, sr, pcm)


def write_impulse(path: pathlib.Path, *, sr: int = 48000, dur: float = 0.5, at: int = 4000, amp: float = 0.5) -> None:
    pcm = array.array("h", [0] * int(dur * sr))
    pcm[at] = int(amp * 32767)
    _write_s16(path, 1, sr, pcm)


def write_burst(path: pathlib.Path, *, sr: int = 48000, dur: float = 1.0, start: float = 0.4, length: float = 0.03) -> None:
    """A stereo noise burst in silence: a codec's time shift shows as a shift of its onset."""
    from enth_ml.degrade.rng import Rng

    rng = Rng(99)
    n = int(dur * sr)
    pcm = array.array("h", [0] * (2 * n))
    for i in range(int(start * sr), int((start + length) * sr)):
        v = int((rng.random() - 0.5) * 2 * 20000)
        pcm[2 * i] = pcm[2 * i + 1] = v
    _write_s16(path, 2, sr, pcm)


def _write_s16(path: pathlib.Path, channels: int, sr: int, pcm: array.array) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm.tobytes())


def lufs(path: pathlib.Path) -> float:
    """Integrated loudness by ffmpeg's ebur128 (an independent run of the measure the chain normalises with)."""
    err = subprocess.run(
        ["ffmpeg", "-nostdin", "-hide_banner", "-i", str(path), "-af", "ebur128=framelog=quiet", "-f", "null", "-"],
        capture_output=True,
        text=True,
        check=True,
    ).stderr
    return float(re.findall(r"I:\s+(-?[\d.]+) LUFS", err)[-1])


def find_seed(pool: str, profile: str | None, stage: str, variant: str, limit: int = 2000) -> int:
    """The first seed whose plan draws ``variant`` for ``stage`` (plans need no audio, so this is cheap)."""
    pools = load_pools()
    for seed in range(limit):
        for e in make_plan(pools, pool, seed, profile)["stages"]:
            if e["stage"] == stage and e.get("active") and e["variant"].endswith("/" + variant):
                return seed
    raise AssertionError(f"no seed below {limit} draws {pool}/{stage}/{variant}")
