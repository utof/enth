"""Feeding shards to the GPU.

* ``ShardStream``: an ``IterableDataset`` for training. Each epoch the shard order is shuffled from ``(seed, epoch)``;
  worker ``i`` of ``W`` takes every ``W``-th shard, reads a few whole shards at a time (one ``numpy.fromfile`` per array,
  which is what a 9P- or network-mounted disk serves fastest), shuffles them together with the previous remainder and
  yields whole batches. ``ds1``'s samples are independent draws per index, so the shuffle is for epoch-to-epoch variety,
  not to break correlations.
* ``EpochPlan``, ``WindowDataset`` and ``batches_from``: the resumable stream (lane TR2). An epoch's samples are a
  fixed sequence: the shards in an order drawn from ``(seed, epoch)``, cut into windows of ``buffer_shards``
  consecutive shards, each window's samples permuted from ``(seed, epoch, window)``; batches are consecutive slices of
  that sequence (the last partial batch is dropped). A DataLoader worker reads and permutes one whole window at a
  time (whole-shard reads, which a 9P or network disk serves fastest), the main process cuts batches, carrying the
  remainder of one window into the next. The sequence depends on nothing else (not on the number of workers), so a
  checkpoint holds the epoch's shard list and the number of samples consumed, and a resume continues **at the exact
  sample**: no batch repeated or skipped.
* ``RamSlice`` and ``batches_from_ram`` (lane FTL, ``train_in_ram``): a fixed training slice held in RAM, its batches
  cut from there in exactly the order ``batches_from`` reads them from disk (the same ``EpochPlan``), with no loader
  workers and no disk reads. For proxies on a slice that fits (240 k samples at about 50 KB each, about 12 GB).
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


class EpochPlan:
    """One epoch's sample order over ``shards`` (a list of ``(path, n)``), from ``(seed, epoch)``."""

    def __init__(self, shards: list[tuple[pathlib.Path | str, int]], seed: int, epoch: int, buffer_shards: int, batch_size: int) -> None:
        if not shards:
            raise ValueError("no shard to plan an epoch over")
        self.shards = [(str(p), int(n)) for p, n in shards]
        self.seed = int(seed)
        self.epoch = int(epoch)
        self.buffer_shards = max(1, int(buffer_shards))
        self.batch_size = int(batch_size)
        order = np.random.default_rng([self.seed, self.epoch, 11]).permutation(len(self.shards))
        self.windows = [order[i : i + self.buffer_shards].tolist() for i in range(0, len(order), self.buffer_shards)]
        sizes = [sum(self.shards[j][1] for j in w) for w in self.windows]
        self.starts = np.concatenate([[0], np.cumsum(sizes)]).astype(np.int64)
        self.n = int(self.starts[-1])
        self.n_batches = self.n // self.batch_size

    @property
    def samples(self) -> int:
        """Samples the epoch trains on (whole batches only)."""
        return self.n_batches * self.batch_size

    def window_perm(self, w: int) -> np.ndarray:
        return np.random.default_rng([self.seed, self.epoch, int(w), 12]).permutation(int(self.starts[w + 1] - self.starts[w]))

    def locate(self, position: int) -> tuple[int, int]:
        """The window holding sample ``position`` of the sequence and the offset in it."""
        w = int(np.searchsorted(self.starts, position, side="right") - 1)
        return w, int(position - self.starts[w])

    def identity(self) -> dict:
        """What a checkpoint stores to rebuild the plan exactly."""
        return {"shards": [[p, n] for p, n in self.shards], "seed": self.seed, "epoch": self.epoch, "buffer_shards": self.buffer_shards, "batch_size": self.batch_size}

    @classmethod
    def from_identity(cls, d: dict) -> "EpochPlan":
        return cls([(p, n) for p, n in d["shards"]], d["seed"], d["epoch"], d["buffer_shards"], d["batch_size"])


class WindowDataset(torch.utils.data.Dataset):
    """Item ``w``: window ``w`` of an ``EpochPlan``, its shards read whole, concatenated and permuted."""

    def __init__(self, plan: EpochPlan, mel_dtype: str = "f2") -> None:
        self.plan = plan
        self.mel_dtype = mel_dtype

    def __len__(self) -> int:
        return len(self.plan.windows)

    def __getitem__(self, w: int) -> dict[str, torch.Tensor]:
        parts = [read_shard(self.plan.shards[j][0], TRAIN_ARRAYS, mel_dtype=self.mel_dtype) for j in self.plan.windows[w]]
        for (path, n), part in zip((self.plan.shards[j] for j in self.plan.windows[w]), parts):
            if len(part["label"]) != n:
                raise ValueError(f"{path}: {len(part['label'])} samples, the manifest says {n}")
        perm = self.plan.window_perm(w)
        return {k: torch.from_numpy(np.ascontiguousarray(np.concatenate([p[k] for p in parts])[perm])) for k in TRAIN_ARRAYS}


def _ignore_sigint(_worker_id: int) -> None:
    import signal

    signal.signal(signal.SIGINT, signal.SIG_IGN)  # a Ctrl-C reaches the main process, which stops cleanly


def batches_from(plan: EpochPlan, position: int, workers: int, prefetch: int = 2, pin_memory: bool = False, mel_dtype: str = "f2"):
    """The epoch's batches from sample ``position`` (a multiple of the batch size) on, as dicts of tensors."""
    bs = plan.batch_size
    if position % bs:
        raise ValueError("a resume position is a whole number of batches")
    if position >= plan.samples:
        return
    w0, off = plan.locate(position)
    ds = WindowDataset(plan, mel_dtype)
    loader = torch.utils.data.DataLoader(
        ds,
        batch_size=None,
        sampler=list(range(w0, len(plan.windows))),
        num_workers=workers,
        pin_memory=pin_memory,
        prefetch_factor=prefetch if workers > 0 else None,
        worker_init_fn=_ignore_sigint if workers > 0 else None,
        persistent_workers=False,
    )
    left = plan.samples - position
    carry: dict[str, torch.Tensor] | None = None
    first = True
    try:
        for win in loader:
            if first:
                win = {k: v[off:] for k, v in win.items()}
                first = False
            n = len(win["label"])
            i = 0
            if carry is not None:  # the remainder of the previous window, completed from the head of this one
                need = bs - len(carry["label"])
                if n < need:
                    carry = {k: torch.cat([carry[k], win[k]]) for k in TRAIN_ARRAYS}
                    continue
                yield {k: torch.cat([carry[k], win[k][:need]]) for k in TRAIN_ARRAYS}
                carry = None
                i = need
                left -= bs
                if left <= 0:
                    return
            while i + bs <= n and left > 0:
                yield {k: v[i : i + bs] for k, v in win.items()}
                i += bs
                left -= bs
            if left <= 0:
                return
            if i < n:
                carry = {k: v[i:] for k, v in win.items()}
    finally:
        del loader


def sample_bytes(mel_dtype: str = "f2") -> int:
    """Bytes one training sample takes in RAM as the loader ships it (mel in ``mel_dtype``; the rest ``f32``)."""
    from ..model import layout as L

    mel = int(np.prod(L.MEL_SHAPE)) * (2 if mel_dtype == "f2" else 4)
    return mel + 4 * (L.CONTEXT_LEN + 2 * L.WIDTH)


def mem_available_bytes() -> int | None:
    """``MemAvailable`` from ``/proc/meminfo`` (None where there is none)."""
    try:
        for line in pathlib.Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


class RamSlice:
    """Training shards read once into one array per field (``mel`` in the dtype the loader would ship)."""

    def __init__(self, shards: list[tuple[pathlib.Path | str, int]], mel_dtype: str = "f2", max_gb: float = 14.0, threads: int = 8) -> None:
        from concurrent.futures import ThreadPoolExecutor

        self.shards = [(str(p), int(n)) for p, n in shards]
        self.mel_dtype = mel_dtype
        self.n = sum(n for _, n in self.shards)
        self.bytes = self.n * sample_bytes(mel_dtype)
        avail = mem_available_bytes()
        if self.bytes > max_gb * 2**30 or (avail is not None and self.bytes > 0.85 * avail):
            have = f", {avail / 2**30:.1f} GiB available" if avail is not None else ""
            raise MemoryError(
                f"train_in_ram: {self.n} samples need {self.bytes / 2**30:.1f} GiB (train_in_ram_max_gb {max_gb}{have}); "
                f"train on fewer parts (240 k samples is about 12 GiB) or set train_in_ram = false"
            )
        first = read_shard(self.shards[0][0], TRAIN_ARRAYS, mel_dtype=mel_dtype)
        self.arrays = {k: np.empty((self.n, *v.shape[1:]), dtype=v.dtype) for k, v in first.items()}
        self.starts = np.concatenate([[0], np.cumsum([n for _, n in self.shards])]).astype(np.int64)

        def fill(i: int) -> None:
            path, n = self.shards[i]
            part = first if i == 0 else read_shard(path, TRAIN_ARRAYS, mel_dtype=mel_dtype)
            if len(part["label"]) != n:
                raise ValueError(f"{path}: {len(part['label'])} samples, the manifest says {n}")
            a = int(self.starts[i])
            for k in TRAIN_ARRAYS:
                self.arrays[k][a : a + n] = part[k]

        with ThreadPoolExecutor(max(1, threads)) as ex:
            list(ex.map(fill, range(len(self.shards))))

    def covers(self, plan: "EpochPlan") -> bool:
        return plan.shards == self.shards

    def sequence(self, plan: EpochPlan) -> np.ndarray:
        """The epoch's sample order as indices into the slice: each window's shards concatenated, then permuted by
        ``plan.window_perm`` (what ``WindowDataset`` does), windows in order."""
        if plan.shards != self.shards:
            raise ValueError("the epoch plan is over other shards than the ones in RAM")
        parts = []
        for w, js in enumerate(plan.windows):
            idx = np.concatenate([np.arange(self.starts[j], self.starts[j + 1], dtype=np.int64) for j in js])
            parts.append(idx[plan.window_perm(w)])
        return np.concatenate(parts)


def batches_from_ram(plan: EpochPlan, position: int, ram: RamSlice, prefetch: int = 2):
    """The epoch's batches from sample ``position`` on, gathered from ``ram`` (a thread gathers ``prefetch`` ahead);
    the same batches as :func:`batches_from` on the same plan."""
    import queue
    import threading

    bs = plan.batch_size
    if position % bs:
        raise ValueError("a resume position is a whole number of batches")
    if position >= plan.samples:
        return
    seq = ram.sequence(plan)
    q: queue.Queue = queue.Queue(maxsize=max(1, prefetch))
    stop = threading.Event()

    def work() -> None:
        try:
            for i in range(position, plan.samples, bs):
                if stop.is_set():
                    return
                idx = seq[i : i + bs]
                q.put({k: torch.from_numpy(np.take(ram.arrays[k], idx, axis=0)) for k in TRAIN_ARRAYS})
            q.put(None)
        except BaseException as err:  # noqa: BLE001 - re-raised in the consumer
            q.put(err)

    t = threading.Thread(target=work, name="enth-ram-batches", daemon=True)
    t.start()
    try:
        while True:
            b = q.get()
            if b is None:
                return
            if isinstance(b, BaseException):
                raise b
            yield b
    finally:
        stop.set()
        while t.is_alive():  # unblock a producer waiting on a full queue
            try:
                q.get_nowait()
            except queue.Empty:
                pass
            t.join(timeout=0.05)


class DevSet:
    """The dev shards in RAM (``f32``), in batches, in file order; ``source`` and ``index`` per sample alongside (the
    per-sample dev files carry them, so two runs' files pair by sample)."""

    def __init__(self, shards: list[tuple[pathlib.Path, int]], limit: int | None = None) -> None:
        parts = []
        meta = []
        total = 0
        for p, _ in shards:
            parts.append(read_shard(p, TRAIN_ARRAYS))
            n = len(parts[-1]["label"])
            m = read_shard(p, ("source", "index"))  # a TR f16 cache (.npz) has neither: unknown source, file order
            meta.append({"source": m.get("source", np.full(n, 255, dtype=np.uint8)), "index": m.get("index", np.arange(total, total + n, dtype=np.uint64))})
            total += n
            if limit is not None and total >= limit:
                break
        self.arrays = {k: np.concatenate([p[k] for p in parts]) for k in TRAIN_ARRAYS}
        self.meta = {k: np.concatenate([m[k] for m in meta]) for k in ("source", "index")}
        if limit is not None:
            self.arrays = {k: v[:limit] for k, v in self.arrays.items()}
            self.meta = {k: v[:limit] for k, v in self.meta.items()}
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
