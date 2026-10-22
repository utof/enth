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
    train_mel_dtype: str = "f4"  # what the loader ships to the GPU: f32, as inference sees it ("f2" halves the bytes)
    dev_limit: int | None = None
    dev_batch: int = 512
    epochs: int = 30  # an upper bound; the time budget usually ends the run first
    max_minutes: float = 180.0  # the GPU budget (wall clock, training and evaluation)
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
    schedule_steps: int = 0  # the LR schedule's length (0: from epochs or the time budget's estimate)
    model: ModelConfig = field(default_factory=ModelConfig)
    loss: LossConfig = field(default_factory=LossConfig)

    def to_dict(self) -> dict:
        d = asdict(self)
        return d

    @classmethod
    def from_dict(cls, d: dict) -> "TrainConfig":
        d = dict(d)
        model = ModelConfig.from_dict(d.pop("model", {}))
        loss = LossConfig(**d.pop("loss", {}))
        return cls(model=model, loss=loss, **d)

    @classmethod
    def load(cls, path: str | pathlib.Path) -> "TrainConfig":
        p = pathlib.Path(path)
        text = p.read_text(encoding="utf-8")
        d = tomllib.loads(text) if p.suffix == ".toml" else json.loads(text)
        return cls.from_dict(d)
