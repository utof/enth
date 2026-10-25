"""``python -m enth_ml.train cooldown RUN_DIR --from-step N --steps M --out OUT``: a decayed copy of a run, branched
from one of its checkpoints (lane FTL; docs/r3.md, "Training"; ml/README.md, "Fast loop").

A WSD run (``lr_schedule = "wsd"``) holds its peak LR through the stable phase; a cooldown branch takes the state at
step N (a ``keep-<step>.pt`` from ``ckpt_keep_every_steps``, or any checkpoint at that step) and trains M more steps
with the LR decaying from the parent's value at N to ``min_lr_frac`` (``decay_shape``), then stops. So one long run
gives finished, comparable models at 20, 40 and 60 % of its length for the price of short cooldowns, and a run can be
extended without restarting.

**Exact.** The branch starts from TR2's resume state (model, optimiser, scaler, WTA running mean, the epoch's shard
list and position, the RNG states, the counters), with the parent's config plus two keys, ``cooldown_from`` and
``cooldown_steps``; nothing before step N changes, so the branch equals a run that had those keys from its first step
(tested on the CPU, bit for bit). The schedule keys are identity keys (``TrainConfig.identity_diff``), which is why
this is a command and not a ``--set``.

``OUT`` gets ``branch.json`` (``enth.train-branch/1``: the parent, the checkpoint and its SHA-256, N, M), the starting
checkpoint, and then a run directory like any other (``status``, ``stop``, ``resume``, ``compare`` and the export work
on it). Run it again on the same ``OUT`` to resume a stopped branch.
"""

from __future__ import annotations

import datetime as _dt
import json
import pathlib

from . import schedule as S
from .ckpt import CheckpointStore, atomic_write_json
from .config import TrainConfig, set_key as _set

BRANCH_SCHEMA = "enth.train-branch/1"


def plan_branch(run_dir: str | pathlib.Path, from_step: int, steps: int, out: str | pathlib.Path, sets: list[str] = ()) -> tuple[TrainConfig, dict, dict]:
    """The branch's config, its starting state and its ``branch.json``, from the parent's checkpoint at ``from_step``.
    Refuses a step with no checkpoint, a parent on a wall-clock budget, a branch of a branch, and a WSD parent already
    in its decay."""
    run_dir = pathlib.Path(run_dir)
    if steps <= 0:
        raise ValueError("--steps: a cooldown needs at least one step")
    store = CheckpointStore(run_dir)
    entries = store.at_step(from_step)
    if not entries:
        have = sorted({int(e.get("step", -1)) for e in store.entries()})
        raise ValueError(f"{run_dir}: no checkpoint at step {from_step} (checkpoints at {have}); keep stable-phase ones with ckpt_keep_every_steps")
    state, entry, errors = None, None, []
    for e in entries:
        try:
            state, entry = store.load(e), e
            break
        except Exception as err:  # noqa: BLE001 - the next checkpoint at the same step may do
            errors.append(f"{e['file']}: {err}")
    if state is None:
        raise ValueError(f"{run_dir}: no checkpoint at step {from_step} loads ({'; '.join(errors)})")
    parent = TrainConfig.from_dict(state["config"])
    if parent.max_minutes > 0:
        raise ValueError("the parent's schedule follows the wall clock (max_minutes > 0): a branch could not reproduce it")
    if parent.cooldown_steps:
        raise ValueError("the parent is itself a cooldown branch: branch from its parent instead")
    schedule_steps = int(state["schedule_steps"])
    if parent.lr_schedule == "wsd" and from_step > schedule_steps - S.wsd_decay_steps(parent, schedule_steps):
        raise ValueError(f"step {from_step} is inside the parent's own decay (from step {schedule_steps - S.wsd_decay_steps(parent, schedule_steps)}): branch from the stable phase")
    d = parent.to_dict()
    d.update(cooldown_from=int(from_step), cooldown_steps=int(steps), out=str(out))
    # Operational defaults for a branch (each can be --set): its own per-sample file at the end, no early stop, no
    # kept checkpoints of its own.
    if not d.get("persample_evals"):
        d["persample_evals"] = "final"
    d["early_stop_patience"] = 0
    d["ckpt_keep_every_steps"] = 0
    d["stop_at_step"] = 0
    for s in sets:
        _set(d, s)
    cfg = TrainConfig.from_dict(d)
    S.check(cfg)
    diff = cfg.identity_diff(state["config"])
    extra = [k for k in diff if k not in ("cooldown_from", "cooldown_steps", "decay_shape", "min_lr_frac")]
    if extra:
        raise ValueError(f"a branch may change only the decay (decay_shape, min_lr_frac) and operational keys; --set changed {extra}")
    state = dict(state)
    state["config"] = cfg.to_dict()
    branch = {
        "schema": BRANCH_SCHEMA,
        "parent": run_dir.name,
        "checkpoint": entry["file"],
        "checkpoint_sha256": entry.get("sha256"),
        "from_step": int(from_step),
        "steps": int(steps),
        "end_step": int(from_step) + int(steps),
        "decay_shape": cfg.decay_shape,
        "min_lr_frac": cfg.min_lr_frac,
        "parent_schedule": {"lr_schedule": parent.lr_schedule, "schedule_steps": schedule_steps, "lr_factor_at_branch": S.base_factor(parent, from_step / max(schedule_steps, 1), from_step, schedule_steps)},
        "utc": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
    }
    return cfg, state, branch


def cooldown(run_dir: str | pathlib.Path, from_step: int, steps: int, out: str | pathlib.Path, sets: list[str] = (), device: str | None = None) -> dict:
    """Branch (or resume a stopped branch in ``out``) and train it to its end; returns ``train``'s summary."""
    from .loop import train

    out = pathlib.Path(out)
    store = CheckpointStore(out)
    if store.has_checkpoint():
        b = json.loads((out / "branch.json").read_text()) if (out / "branch.json").exists() else None
        if b is None or b["from_step"] != int(from_step) or b["steps"] != int(steps) or b["parent"] != pathlib.Path(run_dir).name:
            raise FileExistsError(f"{out} holds another run: branch into a new directory")
        d = json.loads((out / "config.json").read_text())
        d["out"] = str(out)
        for s in sets:
            _set(d, s)
        return train(TrainConfig.from_dict(d), resume=True, device=device)
    cfg, state, branch = plan_branch(run_dir, from_step, steps, out, sets)
    out.mkdir(parents=True, exist_ok=True)
    atomic_write_json(out / "branch.json", branch)
    meta = {
        "step": int(state["step"]),
        "epoch": int(state["epoch"]),
        "position": int(state["position"]),
        "samples_seen": int(state["samples_seen"]),
        "reason": f"branched from {branch['parent']} {branch['checkpoint']}",
        "dev_best": None if state.get("best_dev") in (None, float("inf")) else state["best_dev"],
    }
    store.save(state, "periodic", meta)
    atomic_write_json(out / "config.json", cfg.to_dict())
    return train(cfg, resume=True, device=device)
