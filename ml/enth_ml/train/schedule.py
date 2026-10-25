"""The learning-rate schedule and the weight decay (lane FTL, the fast loop; docs/r3.md, "Training", and ml/README.md,
"Fast loop").

* ``lr_schedule = "cosine"`` (the default, every run before FTL): a linear warmup over ``warmup_steps``, then a cosine
  from the peak to ``min_lr_frac`` of it over the run's progress. :func:`lr_factor_cosine` is that formula, unchanged,
  so the existing configs (``full-v0-a``, ``full-v0-b``, ``pilot-v0``) train exactly as before (tested bit for bit).
* ``lr_schedule = "wsd"`` (warmup, stable, decay): the same warmup, then the peak held constant, then a decay to
  ``min_lr_frac`` over the last ``decay_frac`` of the schedule (``decay_steps`` when given) with the shape
  ``decay_shape`` (``1-sqrt``, the default, beat ``linear`` in the WSD literature; ``cosine`` also offered). A
  stable-phase checkpoint is then a model a cheap cooldown can finish.
* **A cooldown branch** (``cooldown_from`` and ``cooldown_steps``, set by ``python -m enth_ml.train cooldown``): every
  step before ``cooldown_from`` follows the parent's schedule exactly; from there the factor decays from the parent's
  value at that step to ``min_lr_frac`` over ``cooldown_steps``, with ``decay_shape``, and the run ends after them.
* **Weight decay as a timescale** (``weight_decay_tau_samples``): PyTorch's AdamW shrinks the decayed weights by
  ``lr_t * wd`` a step, so their timescale at the peak LR is ``1 / (lr * wd)`` steps, ``batch_size / (lr * wd)``
  samples. Given in samples it stays put when a proxy changes the run's length, its batch or its LR (0 keeps
  ``weight_decay`` as written; ``full-v0-b``'s 0.05 at LR 2e-3 and batch 256 is 2.56 M samples).
"""

from __future__ import annotations

import math

SCHEDULES = ("cosine", "wsd")
DECAY_SHAPES = ("1-sqrt", "linear", "cosine")


def lr_factor_cosine(progress: float, step: int, warmup: int, min_frac: float) -> float:
    """The schedule every run before lane FTL trained with (``loop.lr_factor`` then): warmup, then a cosine on the
    progress."""
    if step < warmup:
        return (step + 1) / warmup
    p = min(max(progress, 0.0), 1.0)
    return min_frac + (1.0 - min_frac) * 0.5 * (1.0 + math.cos(math.pi * p))


def decay_shape(r: float, shape: str) -> float:
    """The share of the way still to go at ``r`` in [0, 1] of a decay: 1 at its start, 0 at its end."""
    r = min(max(r, 0.0), 1.0)
    if shape == "1-sqrt":
        return 1.0 - math.sqrt(r)
    if shape == "linear":
        return 1.0 - r
    if shape == "cosine":
        return 0.5 * (1.0 + math.cos(math.pi * r))
    raise ValueError(f"decay_shape {shape!r}: one of {', '.join(DECAY_SHAPES)}")


def wsd_decay_steps(cfg, schedule_steps: int) -> int:
    """How many steps a WSD schedule decays over: ``decay_steps``, else ``decay_frac`` of the schedule."""
    if cfg.decay_steps > 0:
        return min(int(cfg.decay_steps), int(schedule_steps))
    return min(int(round(cfg.decay_frac * schedule_steps)), int(schedule_steps))


def base_factor(cfg, progress: float, step: int, schedule_steps: int) -> float:
    """The run's own schedule (no cooldown) at ``step``, ``progress`` being ``loop.progress_now``'s."""
    if cfg.lr_schedule == "cosine":
        return lr_factor_cosine(progress, step, cfg.warmup_steps, cfg.min_lr_frac)
    if cfg.lr_schedule != "wsd":
        raise ValueError(f"lr_schedule {cfg.lr_schedule!r}: one of {', '.join(SCHEDULES)}")
    if step < cfg.warmup_steps:
        return (step + 1) / cfg.warmup_steps
    d = wsd_decay_steps(cfg, schedule_steps)
    if d <= 0:
        return 1.0
    # By progress, so a time budget (max_minutes) still ends the decay with the run; with max_minutes = 0 the
    # progress is step / schedule_steps and the decay starts at exactly schedule_steps - d.
    start = 1.0 - d / max(schedule_steps, 1)
    p = min(max(progress, 0.0), 1.0)
    if p <= start:
        return 1.0
    r = (p - start) / (1.0 - start)
    return cfg.min_lr_frac + (1.0 - cfg.min_lr_frac) * decay_shape(r, cfg.decay_shape)


def lr_factor(cfg, progress: float, step: int, schedule_steps: int) -> float:
    """The LR multiplier at ``step``: the run's schedule, or a cooldown branch's decay after ``cooldown_from``."""
    if cfg.cooldown_steps > 0 and step >= cfg.cooldown_from:
        n = cfg.cooldown_from
        f0 = base_factor(cfg, min(n / max(schedule_steps, 1), 1.0), n, schedule_steps)
        r = (step - n) / cfg.cooldown_steps
        return cfg.min_lr_frac + (f0 - cfg.min_lr_frac) * decay_shape(r, cfg.decay_shape)
    return base_factor(cfg, progress, step, schedule_steps)


def end_step(cfg) -> int:
    """The step a cooldown branch ends at (0: not a branch, the schedule or the budget ends the run)."""
    return cfg.cooldown_from + cfg.cooldown_steps if cfg.cooldown_steps > 0 else 0


def weight_decay(cfg) -> float:
    """AdamW's weight decay: ``weight_decay``, or the one whose timescale at the peak LR is ``weight_decay_tau_samples``."""
    if cfg.weight_decay_tau_samples > 0:
        return cfg.batch_size / (cfg.weight_decay_tau_samples * cfg.lr)
    return cfg.weight_decay


def weight_decay_tau_samples(cfg) -> float | None:
    """The decayed weights' timescale at the peak LR, in samples (``None`` without weight decay)."""
    wd = weight_decay(cfg)
    return cfg.batch_size / (cfg.lr * wd) if wd > 0 and cfg.lr > 0 else None


def check(cfg) -> None:
    """Refuse a schedule that cannot run."""
    if cfg.lr_schedule not in SCHEDULES:
        raise ValueError(f"lr_schedule {cfg.lr_schedule!r}: one of {', '.join(SCHEDULES)}")
    if cfg.decay_shape not in DECAY_SHAPES:
        raise ValueError(f"decay_shape {cfg.decay_shape!r}: one of {', '.join(DECAY_SHAPES)}")
    if not 0.0 <= cfg.decay_frac <= 1.0:
        raise ValueError(f"decay_frac {cfg.decay_frac}: between 0 and 1")
    if cfg.cooldown_steps < 0 or cfg.cooldown_from < 0:
        raise ValueError("cooldown_from and cooldown_steps are step counts (>= 0)")
    if cfg.weight_decay_tau_samples < 0:
        raise ValueError("weight_decay_tau_samples is a sample count (>= 0)")
