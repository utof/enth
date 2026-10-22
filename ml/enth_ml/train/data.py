"""Feeding shards to the GPU.

* ``ShardStream``: an ``IterableDataset`` for training. Each epoch the shard order is shuffled from ``(seed, epoch)``;
  worker ``i`` of ``W`` takes every ``W``-th shard, reads a few whole shards at a time (one ``numpy.fromfile`` per array,
  which is what a 9P- or network-mounted disk serves fastest), shuffles them together with the previous remainder and
  yields whole batches. ``ds1``'s samples are independent draws per index, so the shuffle is for epoch-to-epoch variety,
  not to break correlations.
* ``DevSet``: the dev stream in RAM, in ``f32`` as inference sees it, read in order.
* ``compute_standardisation``: per-(resolution, band) and per-context means and deviations over training shards; the
  model holds them as ``Mul`` and ``Add`` constants.
"""

from __future__ import annotations

import pathlib

import numpy as np
import torch
from torch.utils.data import IterableDataset, get_worker_info

from .shards import read_shard

TRAIN_ARRAYS = ("mel", "context", "label", "mask")


class ShardStream(IterableDataset):
    def __init__(
        self,
        shards: list[tuple[pathlib.Path, int]],
        batch_size: int,
        seed: int = 0,
        buffer_shards: int = 4,
        mel_dtype: str = "f2",
    ) -> None:
        super().__init__()
        if not shards:
            raise ValueError("no shard to stream")
        self.shards = [(pathlib.Path(p), int(n)) for p, n in shards]
        self.batch_size = int(batch_size)
        self.seed = int(seed)
        self.buffer_shards = max(1, int(buffer_shards))
        self.mel_dtype = mel_dtype
        self.epoch = 0
        self.n = sum(n for _, n in self.shards)

    def set_epoch(self, epoch: int) -> None:
        self.epoch = int(epoch)

    def batches_per_epoch(self, workers: int) -> int:
        """A lower bound on the batches one epoch yields (each worker drops under one batch at the end)."""
        w = max(1, workers)
        return max(0, (self.n - w * (self.batch_size - 1)) // self.batch_size)

    def _batches(self, pending: dict[str, np.ndarray], rng: np.random.Generator, final: bool):
        n = len(pending["label"])
        order = rng.permutation(n)
        full = n // self.batch_size
        for i in range(full):
            idx = np.sort(order[i * self.batch_size : (i + 1) * self.batch_size])
            yield {k: torch.from_numpy(np.ascontiguousarray(v[idx])) for k, v in pending.items()}
        rest = order[full * self.batch_size :]
        return {k: v[np.sort(rest)] for k, v in pending.items()} if (len(rest) and not final) else None

    def __iter__(self):
        info = get_worker_info()
        wid, nw = (info.id, info.num_workers) if info is not None else (0, 1)
        order = np.random.default_rng([self.seed, self.epoch]).permutation(len(self.shards))
        mine = [self.shards[i] for i in order[wid::nw]]
        rng = np.random.default_rng([self.seed, self.epoch, wid, 1])
        leftover: dict[str, np.ndarray] | None = None
        for start in range(0, len(mine), self.buffer_shards):
            parts = [read_shard(p, TRAIN_ARRAYS, mel_dtype=self.mel_dtype) for p, _ in mine[start : start + self.buffer_shards]]
            if leftover is not None:
                parts.insert(0, leftover)
            pending = {k: np.concatenate([p[k] for p in parts]) for k in TRAIN_ARRAYS}
            final = start + self.buffer_shards >= len(mine)
            leftover = yield from self._batches(pending, rng, final)


class DevSet:
    """The dev shards in RAM (``f32``), in batches, in file order."""

    def __init__(self, shards: list[tuple[pathlib.Path, int]], limit: int | None = None) -> None:
        parts = []
        total = 0
        for p, _ in shards:
            parts.append(read_shard(p, TRAIN_ARRAYS))
            total += len(parts[-1]["label"])
            if limit is not None and total >= limit:
                break
        self.arrays = {k: np.concatenate([p[k] for p in parts]) for k in TRAIN_ARRAYS}
        if limit is not None:
            self.arrays = {k: v[:limit] for k, v in self.arrays.items()}
        self.n = len(self.arrays["label"])

    def batches(self, batch_size: int, mel_dtype: str = "f4"):
        for i in range(0, self.n, batch_size):
            b = {k: v[i : i + batch_size] for k, v in self.arrays.items()}
            if mel_dtype == "f2":
                b["mel"] = b["mel"].astype(np.float16).astype(np.float32)
            yield {k: torch.from_numpy(np.ascontiguousarray(v)) for k, v in b.items()}


def compute_standardisation(shards: list[tuple[pathlib.Path, int]], max_shards: int = 64, seed: int = 0) -> dict[str, np.ndarray]:
    """Means and deviations of ``mel`` per (resolution, band), over samples and time, and of ``context`` per entry, from
    up to ``max_shards`` training shards picked by ``seed``."""
    pick = np.random.default_rng([seed, 7]).permutation(len(shards))[:max_shards]
    s1 = s2 = None
    c1 = c2 = None
    n_mel = 0
    n_ctx = 0
    for i in pick:
        a = read_shard(shards[i][0], ("mel", "context"))
        mel = a["mel"].astype(np.float64)  # [n, 3, 128, 64]
        ctx = a["context"].astype(np.float64)
        s1 = mel.sum((0, 2)) if s1 is None else s1 + mel.sum((0, 2))
        s2 = (mel**2).sum((0, 2)) if s2 is None else s2 + (mel**2).sum((0, 2))
        n_mel += mel.shape[0] * mel.shape[2]
        c1 = ctx.sum(0) if c1 is None else c1 + ctx.sum(0)
        c2 = (ctx**2).sum(0) if c2 is None else c2 + (ctx**2).sum(0)
        n_ctx += ctx.shape[0]
    mel_mean = s1 / n_mel
    mel_std = np.sqrt(np.maximum(s2 / n_mel - mel_mean**2, 0.0))
    ctx_mean = c1 / n_ctx
    ctx_std = np.sqrt(np.maximum(c2 / n_ctx - ctx_mean**2, 0.0))
    return {
        "mel_mean": mel_mean.astype(np.float32),  # [3, 64]
        "mel_std": mel_std.astype(np.float32),
        "ctx_mean": ctx_mean.astype(np.float32),  # [5]
        "ctx_std": ctx_std.astype(np.float32),
        "samples": np.int64(n_ctx),
    }
