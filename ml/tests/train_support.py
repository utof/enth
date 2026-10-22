"""Helpers for lane TR's tests: fake ds1 runs in lane DG's file format, with labels the network can learn.

The tests skip without numpy and torch (``uv sync --group train``); the rest of ``ml/``'s suite needs neither.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import unittest

HAVE_NUMPY = importlib.util.find_spec("numpy") is not None
HAVE_TORCH = importlib.util.find_spec("torch") is not None

needs_numpy = unittest.skipUnless(HAVE_NUMPY, "numpy is not installed (uv sync --group train)")
needs_torch = unittest.skipUnless(HAVE_NUMPY and HAVE_TORCH, "torch is not installed (uv sync --group train)")


def fake_jnd(np):
    """A plausible JND block: 0.02 for every continuous entry (a slope in the table so interpolation is exercised),
    0.025 for depths, 1 elsewhere."""
    from enth_ml.model import layout as L

    scale = np.ones(L.WIDTH, dtype=np.float32)
    scale[: L.CONTINUOUS[1]] = 0.02
    scale[L.DEPTH[0] : L.DEPTH[1]] = 0.025
    table = np.tile(np.linspace(0.015, 0.025, 17, dtype=np.float32), (L.CONTINUOUS[1], 1))
    return scale, table


def fake_samples(np, n: int, seed: int):
    """``n`` samples whose labels are functions of the features (so a model can fit them)."""
    from enth_ml.model import layout as L

    rng = np.random.default_rng(seed)
    mel = rng.normal(-9.0, 2.5, size=(n, *L.MEL_SHAPE)).astype(np.float32)
    context = np.stack(
        [rng.integers(-30, 30, n), rng.normal(0, 0.2, n), rng.integers(0, 2, n), rng.uniform(0.08, 4, n), np.zeros(n)], 1
    ).astype(np.float32)
    feat = mel.mean(axis=(2,))  # [n, 3, 64]
    label = np.zeros((n, L.WIDTH), dtype=np.float32)
    mask = np.ones((n, L.WIDTH), dtype=np.float32)
    z = (feat[:, 0, : L.CONTINUOUS[1]] + 9.0) / 0.3
    label[:, : L.CONTINUOUS[1]] = 1.0 / (1.0 + np.exp(-z))
    for g, (_, a, b) in enumerate(L.GROUPS):
        k = (feat[:, 1, g] > -9.0).astype(int) % (b - a)
        label[np.arange(n), a + k] = 1.0
    label[:, L.STRUCTURE[0] : L.STRUCTURE[1]] = (feat[:, 2, :4] > -9.0).astype(np.float32)
    pres = (feat[:, 2, 4:20] > -9.0).astype(np.float32)
    label[:, L.PRESENCE[0] : L.PRESENCE[1]] = pres
    label[:, L.DEPTH[0] : L.DEPTH[1]] = np.where(pres > 0, 0.5 + 0.4 * np.tanh(feat[:, 2, 20:36] + 9.0), 0.5)
    mask[:, L.DEPTH[0] : L.DEPTH[1]] = pres
    mask[:, 0] = 0.0  # voice.glide unheard, as under one note
    return {
        "mel": mel,
        "context": context,
        "label": label,
        "mask": mask,
        "index": np.arange(n, dtype=np.uint64),
        "seed": rng.integers(0, 2**62, n, dtype=np.uint64),
        "gen_seed": np.zeros(n, dtype=np.uint64),
        "rate": np.full(n, 48000, dtype=np.uint32),
        "source": (np.arange(n) % 3).astype(np.uint8),
    }


def write_run(np, path: pathlib.Path, stream: str, n_shards: int, per_shard: int, seed: int, layout_sha: str | None = None) -> pathlib.Path:
    """A run directory like ``enth-datagen ds1 gen`` writes: shards and ``manifest.json``."""
    from enth_ml.model import layout as L
    from enth_ml.train import shards as S

    path.mkdir(parents=True, exist_ok=True)
    scale, table = fake_jnd(np)
    stats = []
    for k in range(n_shards):
        arrays = fake_samples(np, per_shard, seed * 1000 + k)
        name = f"shard-{k:06d}.bin"
        meta = {"ds1": 1, "stream": stream, "run_seed": seed, "shard": k, "first_index": k * per_shard, "candidates": per_shard}
        size = S.write_shard(path / name, arrays, meta)
        stats.append({"file": name, "shard": k, "first_index": k * per_shard, "candidates": per_shard, "kept": per_shard, "bytes": size, "sha256": S.sha256_file(path / name), "skips": {}, "sources": {}})
    manifest = {
        "schema": S.MANIFEST_SCHEMA,
        "kind": "clean",
        "ds1": 1,
        "stream": stream,
        "role": stream.split("/")[-1],
        "run_seed": seed,
        "count": n_shards * per_shard,
        "shard_size": per_shard,
        "kept": n_shards * per_shard,
        "skips": {},
        "sources": {},
        "engine": "0.0.0-test",
        "feature_set": {"id": 1, "name": L.FEATURE_SET_NAME, "inputs": [{"name": "mel", "dims": list(L.MEL_SHAPE)}, {"name": "context", "dims": [5]}]},
        "layout": {"id": 1, "name": L.LAYOUT_NAME, "generator": 1, "template": 0, "width": L.WIDTH, "sha256": layout_sha or L.LAYOUT_SHA256, "entries": L.expected_entry_names()},
        "jnd": {"scale": scale.tolist(), "table": table.tolist()},
        "arrays": [],
        "sampler": {},
        "exclusions": {},
        "bridge": None,
        "warnings": [],
        "shards": stats,
    }
    (path / "manifest.json").write_text(json.dumps(manifest))
    return path
