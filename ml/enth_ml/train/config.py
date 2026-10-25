"""A training run's configuration: data, model, loss, optimiser, budget. Saved whole in the run manifest."""

from __future__ import annotations

import json
import pathlib
import tomllib
from dataclasses import asdict, dataclass, field

from ..model.net import ModelConfig
from .loss import LossConfig


@dataclass
class TrainConfig:
    train_dirs: list[str] = field(default_factory=list)  # ds1 run directories of the training stream
    dev_dirs: list[str] = field(default_factory=list)  # ds1 run directories of the dev stream
    out: str = ""  # the run directory (checkpoints, logs, manifest)
    seed: int = 1
    batch_size: int = 256
    workers: int = 6
    buffer_shards: int = 4
    prefetch: int = 4
    # Pin the loader's windows (whole windows of shards, ~50-100 MB each) for asynchronous copies. Off: under WSL2 the
    # pinned pool hit cuMemHostAlloc's limit on the desktop (CUDA out of memory in the pin thread); a pageable copy of
    # one batch costs a few ms a step.
    pin_memory: bool = False
    train_mel_dtype: str = "f4"  # what the loader ships to the GPU: f32, as inference sees it ("f2" halves the bytes)
    dev_limit: int | None = None
    dev_batch: int = 512
    epochs: int = 30  # an upper bound; the time budget usually ends the run first
    max_minutes: float = 180.0  # the GPU budget (training wall clock, evaluation included); 0: none, the schedule is by steps alone
    lr: float = 2e-3
    min_lr_frac: float = 0.02
    warmup_steps: int = 1000
    weight_decay: float = 0.05
    beta2: float = 0.99
    grad_clip: float = 5.0
    # FP16 autocast + GradScaler. Off by default: on the desktop's GTX 1660 Ti (Turing TU116, no tensor cores) cuDNN's
    # FP16 convolutions ran this network 2.4x slower than FP32 (848 against 2,029 samples/s, fwd + bwd + AdamW, batch
    # 256; lane TR's probe, 2026-10-22). Turn it on for a GPU with tensor cores.
    amp: bool = False
    channels_last: bool = False  # NCHW measured faster on the same GPU (1,754 against 2,026 samples/s in FP32)
    eval_every_steps: int = 0  # 0: once per epoch
    log_every_steps: int = 50
    stats_shards: int = 64
    early_stop_patience: int = 0  # evaluations without a new best dev loss before stopping (0: never)
    early_stop_min_steps: int = 0  # no early stop before this step
    min_dev_improvement: float = 0.0  # a new best must beat the old one by this much (dev best-of-K)
    eval_at_epoch_end: bool = True
    schedule_steps: int = 0  # the LR schedule's length (0: from epochs or the time budget's estimate)
    # -- lane TR2: data that grows while the run trains (produce.py writes parts) --------------------------------------
    train_glob: str = ""  # parts found at each epoch start (a part counts once its manifest.json exists), added to train_dirs
    min_train_samples: int = 0  # wait for this many training samples before the first step
    # A rolling cache: each epoch trains on the newest parts (in the order they were completed) holding about this many
    # samples (0: every part). On a disk that cannot stream the whole set (the desktop's D:, an HDD read through 9P),
    # the newest parts are the ones still in the host's file cache, having just been written.
    train_window_samples: int = 0
    # Replay another run's data (its run directory): epoch e trains on exactly the parts that run's epoch e had (the parts
    # complete at that epoch's start, in its order), so two arms differ only in their config, not in their data.
    replay_from: str = ""
    max_repeat: float = 0.0  # pacing: an epoch starts only if samples seen stay <= max_repeat x distinct samples (0: off)
    data_target_samples: int = 0  # the producer's planned total; once available, pacing stops waiting
    data_wait_max_minutes: float = 45.0  # pacing gives up waiting after this long without a new part
    # -- lane TR2: stop, checkpoints, resume -------------------------------------------------------------------------
    ckpt_every_minutes: float = 25.0  # periodic checkpoints (training wall clock)
    ckpt_every_steps: int = 0  # also every N steps (0: off; tests)
    keep_checkpoints: int = 2  # periodic checkpoints kept (best.pt is kept apart, never rotated)
    status_every_s: float = 5.0  # how often status.json is rewritten
    stop_poll_s: float = 2.0  # how often the STOP file is looked for
    stop_at_step: int = 0  # request a stop (checkpoint, exit) once this step is done (0: never; tests and planned pauses)
    # Also keep a checkpoint every N steps, never rotated (ckpt/keep-<step>.pt): the stable-phase points a cooldown
    # branches from (python -m enth_ml.train cooldown). 0: off.
    ckpt_keep_every_steps: int = 0
    # -- lane FTL: the fast loop (schedule.py; ml/README.md, "Fast loop") -------------------------------------------------
    lr_schedule: str = "cosine"  # "cosine" (every run before FTL) or "wsd": warmup, a constant LR, then a decay
    decay_frac: float = 0.2  # wsd: the share of schedule_steps the decay takes (the last 20 %)
    decay_steps: int = 0  # wsd: the decay's length in steps instead (0: from decay_frac)
    decay_shape: str = "1-sqrt"  # the shape of a wsd decay and of a cooldown: "1-sqrt", "linear" or "cosine"
    # A cooldown branch (written by `cooldown`, never by hand): the parent's schedule up to cooldown_from, then a decay
    # to min_lr_frac over cooldown_steps, and the run ends.
    cooldown_from: int = 0
    cooldown_steps: int = 0
    # > 0: AdamW's weight decay is set so that the decayed weights' timescale at the peak LR, 1 / (lr * wd) steps, is
    # this many samples (wd = batch_size / (tau * lr)); 0: weight_decay as written (full-v0-b: 2.56 M samples).
    weight_decay_tau_samples: float = 0.0
    # Per-sample dev losses for paired comparisons (`compare`): "final" writes dev-persample-<step>.npz at the end,
    # "all" at every evaluation, "" never.
    persample_evals: str = ""
    # Hold the training shards in RAM (as the loader ships them: f16 mel ~50 KB a sample, so 240 k samples ~12 GB) and
    # cut batches from there: no loader workers, no disk reads. The same samples in the same order as from disk.
    train_in_ram: bool = False
    train_in_ram_max_gb: float = 14.0  # refuse to start when the slice needs more (or more than 85 % of MemAvailable)
    model: ModelConfig = field(default_factory=ModelConfig)
    loss: LossConfig = field(default_factory=LossConfig)

    # Keys a resume may change (operational); every other key must match the checkpoint's config.
    OPERATIONAL = (
        "workers", "prefetch", "pin_memory", "dev_batch", "epochs", "max_minutes", "log_every_steps", "early_stop_patience",
        "early_stop_min_steps", "min_dev_improvement", "min_train_samples", "train_window_samples", "max_repeat", "data_target_samples",
        "data_wait_max_minutes", "ckpt_every_minutes", "ckpt_every_steps", "keep_checkpoints", "status_every_s",
        "stop_poll_s", "stop_at_step", "out", "train_dirs", "dev_dirs", "train_glob", "eval_every_steps", "eval_at_epoch_end",
        "ckpt_keep_every_steps", "persample_evals", "train_in_ram", "train_in_ram_max_gb",
    )

    def to_dict(self) -> dict:
        d = asdict(self)
        return d

    def identity_diff(self, other: dict) -> list[str]:
        """Keys (outside ``OPERATIONAL``) whose values differ from ``other`` (a checkpoint's config)."""
        mine = json.loads(json.dumps(self.to_dict()))
        theirs = json.loads(json.dumps(TrainConfig.from_dict(other).to_dict()))
        return sorted(k for k in set(mine) | set(theirs) if k not in self.OPERATIONAL and mine.get(k) != theirs.get(k))

    @classmethod
    def from_dict(cls, d: dict) -> "TrainConfig":
        d = dict(d)
        model = ModelConfig.from_dict(d.pop("model", {}))
        loss = LossConfig(**d.pop("loss", {}))
        return cls(model=model, loss=loss, **d)

    @staticmethod
    def load_dict(path: str | pathlib.Path, _depth: int = 0) -> dict:
        """A config file as a dict. ``base = "other.toml"`` (a path relative to this file) inherits that file's keys,
        which this file's override; tables (``[model]``, ``[loss]``) merge key by key."""
        p = pathlib.Path(path)
        text = p.read_text(encoding="utf-8")
        d = tomllib.loads(text) if p.suffix == ".toml" else json.loads(text)
        base = d.pop("base", None)
        if base:
            if _depth >= 8:
                raise ValueError(f"{p}: base files nest more than 8 deep (a cycle?)")
            d = _merge(TrainConfig.load_dict(p.parent / base, _depth + 1), d)
        return d

    @classmethod
    def load(cls, path: str | pathlib.Path) -> "TrainConfig":
        return cls.from_dict(cls.load_dict(path))


def set_key(cfg_dict: dict, assignment: str) -> None:
    """Apply ``key=value`` (a dotted key; the value as JSON, else as a string) to a config dict: ``--set``."""
    key, _, value = assignment.partition("=")
    try:
        v = json.loads(value)
    except json.JSONDecodeError:
        v = value
    d = cfg_dict
    parts = key.split(".")
    for part in parts[:-1]:
        d = d.setdefault(part, {})
    d[parts[-1]] = v


def _merge(base: dict, over: dict) -> dict:
    out = dict(base)
    for k, v in over.items():
        out[k] = _merge(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out
