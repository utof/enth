"""The training loop: FP16 autocast with a gradient scaler, AdamW, warmup then cosine on the run's progress, WTA
annealing, per-epoch dev evaluation, checkpoints and a run manifest. Logs stay local (JSONL and CSV in the run
directory); nothing is uploaded.

Progress is ``max(step / schedule_steps, elapsed / budget)``, so the learning rate and the WTA temperature follow
whichever budget (steps or wall-clock minutes) ends the run first.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import math
import os
import pathlib
import platform
import subprocess
import sys
import time

import numpy as np
import torch
from torch.utils.data import DataLoader

from ..model import layout as L
from ..model.net import ProposeV0, count_flops, count_parameters, exported_parameter_count
from .config import TrainConfig
from .data import DevSet, ShardStream, compute_standardisation
from .loss import ProposeLoss
from .metrics import DevMetrics
from .shards import open_dataset, streams_disjoint

RUN_SCHEMA = "enth.train-run/1"
ML = pathlib.Path(__file__).resolve().parents[2]


def _sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def _git(*args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(ML), *args], capture_output=True, text=True, timeout=20).stdout.strip()
    except Exception:  # noqa: BLE001 - provenance is best effort
        return ""


def environment() -> dict:
    """What the manifest records of the software and the machine (no host name, user or path)."""
    env = {
        "python": platform.python_version(),
        "torch": torch.__version__,
        "numpy": np.__version__,
        "cuda": torch.version.cuda,
        "cudnn": torch.backends.cudnn.version() if torch.backends.cudnn.is_available() else None,
        "cpus": os.cpu_count(),
        "platform": platform.platform(terse=True),
    }
    if torch.cuda.is_available():
        p = torch.cuda.get_device_properties(0)
        env["gpu"] = {"name": p.name, "memory_mib": p.total_memory // (1 << 20), "capability": f"{p.major}.{p.minor}"}
    lock = ML / "uv.lock"
    env["uv_lock_sha256"] = _sha256_bytes(lock.read_bytes()) if lock.exists() else None
    env["pyproject_sha256"] = _sha256_bytes((ML / "pyproject.toml").read_bytes())
    env["commit"] = _git("rev-parse", "HEAD")
    env["dirty"] = bool(_git("status", "--porcelain", "--", "."))
    return env


class GpuMonitor:
    """``nvidia-smi`` sampled every few seconds into ``gpu.csv`` (utilisation, memory, power, temperature)."""

    def __init__(self, path: pathlib.Path, every_s: int = 5) -> None:
        self.proc = None
        try:
            self.fh = open(path, "a", encoding="utf-8")
            self.proc = subprocess.Popen(
                [
                    "nvidia-smi",
                    "--query-gpu=timestamp,utilization.gpu,utilization.memory,memory.used,power.draw,temperature.gpu,clocks.sm",
                    "--format=csv,noheader,nounits",
                    f"-l={every_s}",
                ],
                stdout=self.fh,
                stderr=subprocess.DEVNULL,
            )
        except (OSError, FileNotFoundError):
            self.proc = None

    def stop(self) -> None:
        if self.proc is not None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
            self.fh.close()


def _save(obj: dict, path: pathlib.Path) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    torch.save(obj, tmp)
    os.replace(tmp, path)


def _param_groups(model: torch.nn.Module, weight_decay: float) -> list[dict]:
    decay, no_decay = [], []
    for name, p in model.named_parameters():
        if not p.requires_grad:
            continue
        # Weight decay on convolution and linear weights only; never on biases or BatchNorm's affine vectors.
        (decay if (p.ndim >= 2 and "bn" not in name) else no_decay).append(p)
    return [{"params": decay, "weight_decay": weight_decay}, {"params": no_decay, "weight_decay": 0.0}]


def lr_factor(progress: float, step: int, warmup: int, min_frac: float) -> float:
    if step < warmup:
        return (step + 1) / warmup
    p = min(max(progress, 0.0), 1.0)
    return min_frac + (1.0 - min_frac) * 0.5 * (1.0 + math.cos(math.pi * p))


@torch.no_grad()
def evaluate(model: ProposeV0, loss_fn: ProposeLoss, dev: DevSet, device: torch.device, cfg: TrainConfig, mel_dtype: str = "f4", per_entry: bool = True) -> dict:
    model.eval()
    loss_fn.eval()
    m = DevMetrics(loss_fn, cfg.model.hypotheses)
    t0 = time.perf_counter()
    for b in dev.batches(cfg.dev_batch, mel_dtype=mel_dtype):
        mel = b["mel"].to(device, non_blocking=True).float()
        if cfg.channels_last:
            mel = mel.contiguous(memory_format=torch.channels_last)
        ctx = b["context"].to(device, non_blocking=True)
        with torch.autocast(device_type=device.type, dtype=torch.float16, enabled=cfg.amp and device.type == "cuda"):
            hyp, conf = model(mel, ctx)
        m.update(hyp, conf, b["label"].to(device), b["mask"].to(device))
    out = m.summary(per_entry=per_entry)
    out["eval_seconds"] = time.perf_counter() - t0
    model.train()
    loss_fn.train()
    return out


def train(cfg: TrainConfig, resume: bool = False, device: str | None = None) -> dict:
    dev_name = device or ("cuda" if torch.cuda.is_available() else "cpu")
    device_t = torch.device(dev_name)
    out = pathlib.Path(cfg.out)
    out.mkdir(parents=True, exist_ok=True)
    torch.manual_seed(cfg.seed)
    np.random.seed(cfg.seed % (2**32))
    torch.backends.cudnn.benchmark = True

    train_ds = open_dataset(cfg.train_dirs, "train")
    dev_ds = open_dataset(cfg.dev_dirs, "dev")
    if not streams_disjoint(train_ds, dev_ds):
        raise ValueError("train and dev share a stream key: splits are by stream, never by index range")
    if train_ds.jnd_table.tolist() != dev_ds.jnd_table.tolist():
        raise ValueError("train and dev disagree on the JND table")

    ckpt_path = out / "ckpt-last.pt"
    state = None
    if resume and ckpt_path.exists():
        state = torch.load(ckpt_path, map_location="cpu", weights_only=False)

    model = ProposeV0(cfg.model)
    if state is not None:
        stats = {k: np.asarray(v) for k, v in state["stats"].items()}
    else:
        t0 = time.perf_counter()
        stats = compute_standardisation(train_ds.shards, cfg.stats_shards, cfg.seed)
        print(f"standardisation from {int(stats['samples'])} samples in {time.perf_counter() - t0:.1f} s", flush=True)
    model.set_standardisation(*(torch.as_tensor(stats[k]) for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std")))
    model.to(device_t)
    if cfg.channels_last:
        model = model.to(memory_format=torch.channels_last)
    loss_fn = ProposeLoss(train_ds.jnd_table, train_ds.jnd_scale, cfg.loss).to(device_t)
    opt = torch.optim.AdamW(_param_groups(model, cfg.weight_decay), lr=cfg.lr, betas=(0.9, cfg.beta2), fused=device_t.type == "cuda")
    use_amp = cfg.amp and device_t.type == "cuda"
    scaler = torch.amp.GradScaler("cuda", enabled=use_amp)

    stream = ShardStream(train_ds.shards, cfg.batch_size, cfg.seed, cfg.buffer_shards, cfg.train_mel_dtype)
    per_epoch = stream.batches_per_epoch(cfg.workers)
    schedule_steps = cfg.schedule_steps or cfg.epochs * per_epoch
    budget_s = cfg.max_minutes * 60.0

    step, epoch0, elapsed0, best_dev, evals_since_best = 0, 0, 0.0, float("inf"), 0
    if state is not None:
        model.load_state_dict(state["model"])
        opt.load_state_dict(state["opt"])
        scaler.load_state_dict(state["scaler"])
        loss_fn.running.copy_(torch.as_tensor(state["loss_running"]))
        loss_fn._started = True
        step, epoch0, elapsed0 = state["step"], state["epoch"], state["elapsed"]
        best_dev, evals_since_best = state["best_dev"], state.get("evals_since_best", 0)
        print(f"resumed at step {step}, epoch {epoch0}, {elapsed0 / 60:.1f} min", flush=True)

    dev = DevSet(dev_ds.shards, cfg.dev_limit)
    run = {
        "schema": RUN_SCHEMA,
        "started": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
        "config": cfg.to_dict(),
        "environment": environment(),
        "data": {"train": train_ds.describe(), "dev": dev_ds.describe() | {"in_ram": dev.n}},
        "augmentation": "none (clean ds1 renders only)",
        "layout": {"id": L.LAYOUT_ID, "sha256": L.LAYOUT_SHA256, "width": L.WIDTH},
        "feature_set": {"id": L.FEATURE_SET_ID, "name": L.FEATURE_SET_NAME},
        "model": {
            "parameters_trainable": count_parameters(model),
            "parameters_exported": exported_parameter_count(model),
            "flops_per_inference": count_flops(model),
            "trunk_shape": list(model.trunk_shape),
        },
        "standardisation_sha256": _sha256_bytes(b"".join(np.ascontiguousarray(stats[k]).tobytes() for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std"))),
        "batches_per_epoch": per_epoch,
        "schedule_steps": schedule_steps,
        "resumed_from_step": step if state is not None else None,
    }
    (out / ("run.json" if state is None else f"run-resume-{step}.json")).write_text(json.dumps(run, indent=1))
    print(json.dumps(run["model"]), flush=True)

    log = open(out / "metrics.jsonl", "a", encoding="utf-8")
    gpu = GpuMonitor(out / "gpu.csv") if device_t.type == "cuda" else None
    t_start = time.perf_counter() - elapsed0
    stop = False
    final_eval = None

    def progress_now() -> float:
        return max(step / max(schedule_steps, 1), (time.perf_counter() - t_start) / budget_s)

    def checkpoint(epoch: int, tag: str = "last") -> None:
        _save(
            {
                "model": model.state_dict(),
                "opt": opt.state_dict(),
                "scaler": scaler.state_dict(),
                "loss_running": loss_fn.running.detach().cpu(),
                "step": step,
                "epoch": epoch,
                "elapsed": time.perf_counter() - t_start,
                "best_dev": best_dev,
                "evals_since_best": evals_since_best,
                "stats": {k: np.asarray(v) for k, v in stats.items()},
                "config": cfg.to_dict(),
            },
            out / f"ckpt-{tag}.pt",
        )

    try:
        for epoch in range(epoch0, cfg.epochs):
            stream.set_epoch(epoch)
            loader = DataLoader(
                stream,
                batch_size=None,
                num_workers=cfg.workers,
                pin_memory=device_t.type == "cuda",
                prefetch_factor=cfg.prefetch if cfg.workers > 0 else None,
                persistent_workers=False,
            )
            it = iter(loader)
            ep_t0 = time.perf_counter()
            ep_samples = 0
            win_t0 = time.perf_counter()
            win_data = 0.0
            win_steps = 0
            acc = torch.zeros(4, device=device_t)  # total, hyp, conf, best
            win_counts = torch.zeros(cfg.model.hypotheses, device=device_t)
            last_ckpt = time.perf_counter()
            while True:
                td = time.perf_counter()
                try:
                    b = next(it)
                except StopIteration:
                    break
                win_data += time.perf_counter() - td
                p = progress_now()
                if p >= 1.0:
                    stop = True
                    break
                f = lr_factor(p, step, cfg.warmup_steps, cfg.min_lr_frac)
                for g in opt.param_groups:
                    g["lr"] = cfg.lr * f
                temp = cfg.loss.temperature(p)
                mel = b["mel"].to(device_t, non_blocking=True).float()
                if cfg.channels_last:
                    mel = mel.contiguous(memory_format=torch.channels_last)
                ctx = b["context"].to(device_t, non_blocking=True)
                label = b["label"].to(device_t, non_blocking=True)
                mask = b["mask"].to(device_t, non_blocking=True)
                with torch.autocast(device_type=device_t.type, dtype=torch.float16, enabled=use_amp):
                    hyp, conf = model(mel, ctx)
                r = loss_fn(hyp, conf, label, mask, temperature=temp)
                opt.zero_grad(set_to_none=True)
                scaler.scale(r["total"]).backward()
                scaler.unscale_(opt)
                gn = torch.nn.utils.clip_grad_norm_(model.parameters(), cfg.grad_clip)
                scaler.step(opt)
                scaler.update()
                acc += torch.stack([r["total"].detach(), r["hyp"].detach(), r["conf"].detach(), r["best"]])
                win_counts += torch.bincount(r["winner"], minlength=cfg.model.hypotheses).float()
                step += 1
                win_steps += 1
                ep_samples += mel.shape[0]
                if step % cfg.log_every_steps == 0:
                    a = (acc / win_steps).tolist()  # one host sync per log interval
                    wall = time.perf_counter() - win_t0
                    wc = win_counts / win_counts.sum()
                    rec = {
                        "kind": "train",
                        "step": step,
                        "epoch": epoch,
                        "elapsed_min": (time.perf_counter() - t_start) / 60.0,
                        "progress": p,
                        "loss": a[0],
                        "hyp": a[1],
                        "conf": a[2],
                        "best": a[3],
                        "lr": cfg.lr * f,
                        "temperature": temp,
                        "grad_norm": float(gn),
                        "scale": scaler.get_scale() if use_amp else 1.0,
                        "step_ms": 1000.0 * wall / win_steps,
                        "samples_per_s": win_steps * cfg.batch_size / wall,
                        "data_wait_frac": win_data / wall,
                        "gpu_mem_alloc_mib": torch.cuda.max_memory_allocated() / 2**20 if device_t.type == "cuda" else 0,
                        "gpu_mem_reserved_mib": torch.cuda.max_memory_reserved() / 2**20 if device_t.type == "cuda" else 0,
                        "win_share_max": float(wc.max()),
                        "heads_winning_2pct": int((wc >= 0.02).sum()),
                    }
                    log.write(json.dumps(rec) + "\n")
                    log.flush()
                    print(
                        f"step {step} ep {epoch} {rec['elapsed_min']:.1f}min loss {a[0]:.4f} best {a[3]:.4f} conf {a[2]:.3f} "
                        f"lr {rec['lr']:.2e} T {temp:.3g} {rec['samples_per_s']:.0f} samp/s {rec['step_ms']:.0f} ms/step "
                        f"wait {100 * rec['data_wait_frac']:.1f}% heads {rec['heads_winning_2pct']}",
                        flush=True,
                    )
                    acc.zero_()
                    win_counts.zero_()
                    win_t0 = time.perf_counter()
                    win_data = 0.0
                    win_steps = 0
                if cfg.eval_every_steps and step % cfg.eval_every_steps == 0:
                    ev = evaluate(model, loss_fn, dev, device_t, cfg, per_entry=False)
                    log.write(json.dumps({"kind": "dev", "step": step, "epoch": epoch, "elapsed_min": (time.perf_counter() - t_start) / 60.0, **ev}) + "\n")
                    log.flush()
                if time.perf_counter() - last_ckpt > 20 * 60:
                    checkpoint(epoch)  # mid-epoch: a resume restarts this epoch's data order
                    last_ckpt = time.perf_counter()
            del it, loader
            ep_wall = time.perf_counter() - ep_t0
            ev = evaluate(model, loss_fn, dev, device_t, cfg, per_entry=True)
            final_eval = ev
            improved = ev["loss_best"] < best_dev
            if improved:
                best_dev = ev["loss_best"]
                evals_since_best = 0
            else:
                evals_since_best += 1
            rec = {
                "kind": "dev",
                "step": step,
                "epoch": epoch,
                "epoch_complete": not stop,
                "epoch_seconds": ep_wall,
                "epoch_samples": ep_samples,
                "elapsed_min": (time.perf_counter() - t_start) / 60.0,
                **ev,
            }
            log.write(json.dumps(rec) + "\n")
            log.flush()
            print(
                f"== epoch {epoch} {'done' if not stop else 'cut'}: {ep_samples} samples in {ep_wall:.0f} s; dev best-of-K {ev['loss_best']:.4f} "
                f"top {ev['loss_top']:.4f} mean-K {ev['loss_mean_k']:.4f} conf top1 {ev['conf_top1']:.3f} heads {ev['wta']['heads_winning_2pct']} "
                f"cont med {ev['best']['continuous_median_jnd']:.2f} JND (eval {ev['eval_seconds']:.0f} s)",
                flush=True,
            )
            checkpoint(epoch + 1)
            if improved:
                checkpoint(epoch + 1, "best")
            if stop or (cfg.early_stop_patience and evals_since_best >= cfg.early_stop_patience):
                break
    finally:
        log.close()
        if gpu is not None:
            gpu.stop()

    summary = {
        "steps": step,
        "elapsed_min": (time.perf_counter() - t_start) / 60.0,
        "best_dev_loss": best_dev,
        "last_eval": final_eval,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    return summary


def evaluate_checkpoint(path: str | pathlib.Path, dev_dirs: list[str] | None = None, device: str | None = None, mel_dtype: str = "f4") -> dict:
    """Load a checkpoint and evaluate it on dev (``mel_dtype="f2"``: the features rounded to f16 first, the storage
    question)."""
    state = torch.load(path, map_location="cpu", weights_only=False)
    cfg = TrainConfig.from_dict(state["config"])
    device_t = torch.device(device or ("cuda" if torch.cuda.is_available() else "cpu"))
    dev_ds = open_dataset(dev_dirs or cfg.dev_dirs, "dev")
    model = ProposeV0(cfg.model)
    model.load_state_dict(state["model"])
    model.to(device_t)
    if cfg.channels_last:
        model = model.to(memory_format=torch.channels_last)
    loss_fn = ProposeLoss(dev_ds.jnd_table, dev_ds.jnd_scale, cfg.loss).to(device_t)
    dev = DevSet(dev_ds.shards, cfg.dev_limit)
    return evaluate(model, loss_fn, dev, device_t, cfg, mel_dtype=mel_dtype)
