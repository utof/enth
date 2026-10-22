"""Helpers shared by the AUG tests: tiny synthetic "recorded" collections (no data files), and a manifest builder.

Standard library only; ``ffmpeg`` is the system binary (the tests that need it skip without it). Importing this module
puts ``ml/`` on ``sys.path`` (through ``degrade_support``) so the tests run from the repository root or from ``ml/``.
"""

from __future__ import annotations

import array
import math
import pathlib
import random
import tempfile
import textwrap

from degrade_support import HAVE_FFMPEG, ML, have_encoder, write_burst, write_impulse, write_pluck  # noqa: F401
from enth_ml.augment import manifest as mf
from enth_ml.degrade import wavio

__all__ = [
    "HAVE_FFMPEG",
    "ML",
    "have_encoder",
    "write_burst",
    "write_impulse",
    "write_pluck",
    "write_rir",
    "write_noise",
    "make_collection",
    "build_manifest",
    "source_block",
    "temp_dir",
    "manifest_text",
    "config_text",
    "recorded_stage",
]


def write_rir(path: pathlib.Path, *, sr: int = 16000, rt60: float = 0.5, seed: int = 1, dur: float | None = None,
              channels: int = 1, delay_ms: float = 3.0) -> None:
    """A synthetic "measured" impulse response: a direct spike after ``delay_ms`` and an exponentially decaying noise tail."""
    rng = random.Random(seed)
    dur = dur if dur is not None else rt60 * 1.3
    n = int(dur * sr)
    d = int(delay_ms / 1000.0 * sr)
    out = array.array("f", [0.0] * (n * channels))
    for c in range(channels):
        for i in range(n):
            if i == d:
                out[i * channels + c] = 1.0
            elif i > d + int(0.002 * sr):
                t = (i - d) / sr
                out[i * channels + c] = 0.12 * (2.0 * rng.random() - 1.0) * 10.0 ** (-3.0 * t / rt60)
    wavio.write_wav_f32(path, sr, channels, out)


def write_noise(path: pathlib.Path, *, sr: int = 16000, dur: float = 3.0, seed: int = 1, amp: float = 0.2) -> None:
    rng = random.Random(seed)
    n = int(dur * sr)
    y = 0.0
    out = array.array("f")
    for _ in range(n):
        y += 0.2 * ((2.0 * rng.random() - 1.0) - y)  # a low-passed hiss: not white, so a mix is not trivially like DEG's
        out.append(amp * y * 3.0)
    wavio.write_wav_f32(path, sr, 1, out)


def make_collection(root: pathlib.Path, kind: str, count: int, **kw) -> pathlib.Path:
    """``count`` files of ``kind`` (``"rir"`` or ``"noise"``) in ``root``; returns ``root``."""
    root.mkdir(parents=True, exist_ok=True)
    for i in range(count):
        if kind == "rir":
            write_rir(root / f"rir-{i:03d}.wav", seed=i + 1, rt60=kw.get("rt60", 0.3 + 0.1 * (i % 5)))
        else:
            write_noise(root / f"noise-{i:03d}.wav", seed=i + 1, dur=kw.get("dur", 3.0 + (i % 3)))
    return root


def manifest_text(sources: str = "", tools: str = "", shipped: str = '["CC0-1.0", "CC-BY-3.0", "CC-BY-4.0", "Apache-2.0"]') -> str:
    return (
        f'schema = "enth.data-manifest/1"\nversion = 1\nlocal_root = "/nonexistent"\n[policy]\nshipped_licences = {shipped}\n'
        + textwrap.dedent(sources)
        + textwrap.dedent(tools)
    )


def source_block(**kw: str) -> str:
    """One ``[[source]]`` table with sane defaults; ``kw`` overrides (values are written as TOML strings)."""
    base = {
        "id": "x",
        "name": "x",
        "kind": "noise",
        "role": "augmentation",
        "url": "https://example.test/x",
        "version": "1",
        "licence": "CC0-1.0",
        "licence_url": "https://example.test/licence",
        "licence_read_on": "2026-10-22",
        "licence_evidence": "read on the page",
        "use": "shipped-training",
        "attribution": "x by y",
    }
    base.update(kw)
    lines = ["[[source]]"] + [f'{k} = "{v}"' for k, v in base.items() if v is not None]
    return "\n".join(lines) + "\n"


def build_manifest(sources: str, tools: str = "") -> mf.Manifest:
    """Parse a manifest made of the given ``[[source]]``/``[[tool]]`` text (the refusal tests build bad ones this way)."""
    return mf.parse_manifest(manifest_text(sources, tools))


def temp_dir() -> tempfile.TemporaryDirectory:
    return tempfile.TemporaryDirectory(prefix="enth-augment-test-")


def recorded_stage(stage: str, ref: str, *, p: float = 1.0, **params: str) -> str:
    """A ``[stage.<stage>]`` table with one recorded variant drawing from ``ref`` (``bg_noise``, ``rir`` or ``mix_in``)."""
    extra = {"bg_noise": 'snr_db = [15.0, 50.0]', "rir": 'wet_db = [-20.0, 0.0]', "mix_in": 'rel_db = [-20.0, -6.0]\noffset_ms = [0.0, 100.0]'}[stage]
    kind = "layer-recorded" if stage == "mix_in" else "recorded"
    return f'[stage.{stage}]\np = {p}\n[stage.{stage}.variant.v]\nkind = "{kind}"\nsource = "{ref}"\n{extra}\n'


def config_text(*, purpose: str = "shipped-training", pool: str | None = None, stages: str = "", extra: str = "",
                name: str = "t", version: int = 1) -> str:
    """Augment-config text for a test: the header for ``purpose`` (the pool follows it unless given), ``stages`` and ``extra``."""
    pool = pool or ("heldout" if purpose == "eval" else "train")
    return (
        f'schema = "enth.augment-config/1"\nversion = {version}\nname = "{name}"\n'
        f'[config]\npurpose = "{purpose}"\ndegrade_pool = "{pool}"\n' + extra + stages
    )
