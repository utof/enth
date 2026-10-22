"""The training loop: FP16 autocast with a gradient scaler, AdamW, warmup then cosine on the run's progress, WTA
annealing, dev evaluation, checkpoints and a run manifest. Logs stay local (JSONL and CSV in the run directory);
nothing is uploaded.

Progress is ``max(step / schedule_steps, elapsed / budget)``, so the learning rate and the WTA temperature follow
whichever budget (steps or wall-clock minutes) ends the run first; ``elapsed`` is training time (stops and waits for
data do not count).

**Stoppable and crash-safe (lane TR2).** A stop request (``control.py``: the ``stop`` command, a ``STOP`` file,
SIGTERM, SIGINT) ends the current step, writes a checkpoint and returns ``state = "stopped"``. Checkpoints
(``ckpt.py``) are written every ``ckpt_every_minutes`` (the newest two kept), plus ``best.pt`` at every new best dev
best-of-K, atomically and with checksums. A checkpoint holds everything the next step depends on: the model, the
optimiser, the gradient scaler, the WTA loss's running mean, the step and epoch (the LR schedule and the temperature
are functions of them), the epoch's shard list and the number of samples consumed in it (``data.EpochPlan``), the
RNG states, the log accumulators, the best dev loss and the early-stop counter. So a run stopped at step k and resumed
reaches step n with the same weights and the same logged losses as one that never stopped (on the CPU, where the
kernels are deterministic; ``tests/test_train_resume.py``), and a ``kill -9`` loses at most one interval.

**Data that grows.** With ``train_glob``, each epoch trains on the parts complete when it starts (``produce.py``
renders them meanwhile); ``max_repeat`` paces the run so samples seen stay within that many times the distinct samples
available, waiting at an epoch boundary for more parts (the wait is logged and does not count as training time).
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import math
import os
import pathlib
import platform
import random
import re
import signal
import subprocess
import sys
import time

import numpy as np
import torch
from torch.utils.data import DataLoader

from ..model import layout as L
from ..model.net import ProposeV0, count_flops, count_parameters, exported_parameter_count
from .ckpt import CheckpointStore, atomic_write_json
from .config import TrainConfig
from .control import StopFlag, write_status
from .data import DevSet, EpochPlan, batches_from, compute_standardisation
from .loss import ProposeLoss
from .metrics import DevMetrics
from .shards import Dataset, open_dataset, streams_disjoint

RUN_SCHEMA = "enth.train-run/1"
CKPT_RETRY_S = 10.0  # between attempts at a checkpoint write that failed
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


def _die_with_parent() -> None:
    """In a child process: get SIGTERM when the parent dies (a ``kill -9`` of the trainer leaves no orphan)."""
    try:
        import ctypes

        ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)  # PR_SET_PDEATHSIG
    except Exception:  # noqa: BLE001 - not Linux: the monitor is stopped on a normal exit anyway
        pass


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
                preexec_fn=_die_with_parent,
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


def _rng_state(device_t: torch.device) -> dict:
    st = {"torch": torch.get_rng_state(), "numpy": np.random.get_state(), "python": random.getstate()}
    if device_t.type == "cuda":
        st["cuda"] = torch.cuda.get_rng_state_all()
    return st


def _set_rng_state(st: dict, device_t: torch.device) -> None:
    torch.set_rng_state(st["torch"])
    np.random.set_state(st["numpy"])
    random.setstate(st["python"])
    if device_t.type == "cuda" and "cuda" in st:
        torch.cuda.set_rng_state_all(st["cuda"])


def _natural_key(p: pathlib.Path) -> list:
    return [int(t) if t.isdigit() else t for t in re.split(r"(\d+)", p.name)]


def _completed(p: pathlib.Path) -> tuple:
    """A part's place in the order parts were completed: its manifest's time (written last), then its name."""
    try:
        return (os.stat(p / "manifest.json").st_mtime, _natural_key(p))
    except OSError:
        return (float("inf"), _natural_key(p))


def scan_train(cfg: TrainConfig, cutoff_unix: float | None = None) -> Dataset:
    """The training set as it stands: ``train_dirs`` plus every complete part ``train_glob`` matches, in the order
    they were completed; with ``train_window_samples``, only the newest parts holding about that many samples.

    ``cutoff_unix`` (a replay): only the parts complete by then, in name order (the order the replayed run's scan used)."""
    dirs = [pathlib.Path(d) for d in cfg.train_dirs]
    if cfg.train_glob:
        g = pathlib.Path(cfg.train_glob)
        found = [p for p in g.parent.glob(g.name) if (p / "manifest.json").exists() and not p.name.endswith((".tmp", ".stage"))]
        if cutoff_unix is not None:
            found = sorted((p for p in found if os.stat(p / "manifest.json").st_mtime <= cutoff_unix), key=_natural_key)
        else:
            found = sorted(found, key=_completed)
        found = [p for p in found if p not in dirs]
        if cfg.train_window_samples > 0 and found:
            keep, total = [], 0
            for p in reversed(found):
                keep.append(p)
                total += int(json.loads((p / "manifest.json").read_text()).get("kept", 0))
                if total >= cfg.train_window_samples:
                    break
            found = list(reversed(keep))
        dirs += found
    if not dirs:
        raise FileNotFoundError("no training data: train_dirs is empty and train_glob matches no complete part")
    return open_dataset(dirs, "train")


class SafeLog:
    """``metrics.jsonl``, append-only; a failed write (seen: ENOMEM from WSL's 9P mount of a Windows drive under memory
    pressure) is retried once on a reopened file, then the record is dropped with a warning: never a crash."""

    def __init__(self, path: pathlib.Path) -> None:
        self.path = path
        self.fh = open(path, "a", encoding="utf-8")
        self.dropped = 0

    def _retry(self, s: str, err: OSError) -> None:
        print(f"warning: writing {self.path.name} failed ({err!r}); retrying once", flush=True)
        try:
            self.fh.close()
        except OSError:
            pass
        time.sleep(1.0)
        try:
            self.fh = open(self.path, "a", encoding="utf-8")
            self.fh.write(s)
            self.fh.flush()
        except OSError as err2:
            self.dropped += 1
            print(f"warning: {self.path.name}: a record was dropped ({err2!r})", flush=True)

    def write(self, s: str) -> None:
        try:
            self.fh.write(s)
            self.fh.flush()
        except OSError as err:
            self._retry(s, err)

    def flush(self) -> None:
        pass  # every write is flushed

    def close(self) -> None:
        try:
            self.fh.close()
        except OSError:
            pass


class Replay:
    """Another run's epochs, from its ``metrics.jsonl``: when each started, with how many parts and samples."""

    def __init__(self, run_dir: str) -> None:
        self.dir = pathlib.Path(run_dir)
        self.epochs: dict[int, dict] = {}
        for line in (self.dir / "metrics.jsonl").read_text().splitlines():
            r = json.loads(line)
            if r.get("kind") == "epoch":
                self.epochs[int(r["epoch"])] = r  # a resumed run logs an epoch once (its plan is in the checkpoint)
        if 0 not in self.epochs:
            raise ValueError(f"{run_dir}: no epoch records to replay")
        self.run = json.loads((self.dir / "run.json").read_text())

    def cutoff(self, epoch: int) -> float | None:
        r = self.epochs.get(epoch)
        return _dt.datetime.fromisoformat(r["utc"]).timestamp() + 0.999 if r else None

    def check(self, epoch: int, ds: Dataset) -> None:
        r = self.epochs.get(epoch)
        if r is not None and (len(ds.runs) != r["parts"] or ds.n != r["available"]):
            raise ValueError(f"replay of epoch {epoch}: {len(ds.runs)} parts / {ds.n} samples, the replayed run had {r['parts']} / {r['available']}")


def train(cfg: TrainConfig, resume: bool = True, device: str | None = None, allow_config_change: bool = False, stop_flag: StopFlag | None = None) -> dict:
    """Train, or resume from the newest valid checkpoint in ``cfg.out`` (``resume=False`` refuses a directory that has
    one). Returns a summary whose ``state`` is ``finished`` (budget, epochs or early stop) or ``stopped`` (on request:
    a checkpoint was written and ``run`` can carry on from it)."""
    dev_name = device or ("cuda" if torch.cuda.is_available() else "cpu")
    device_t = torch.device(dev_name)
    out = pathlib.Path(cfg.out)
    out.mkdir(parents=True, exist_ok=True)
    store = CheckpointStore(out, cfg.keep_checkpoints)
    stop = stop_flag or StopFlag(out, cfg.stop_poll_s)
    stop.install()
    status: dict = {"state": "starting", "schedule_steps": None}
    write_status(out, status)

    def log_line(msg: str) -> None:
        print(msg, flush=True)

    state, entry = (None, None)
    if store.has_checkpoint():
        if not resume:
            raise FileExistsError(f"{out} has checkpoints: resume it, or train into a new directory")
        state, entry = store.load_latest(log_line)
        if state is None:
            raise RuntimeError(f"{out}: no checkpoint passes its checksum and loads")
        diff = cfg.identity_diff(state["config"])
        if diff and not allow_config_change:
            raise ValueError(f"the config differs from the checkpoint's in {diff}; resume with the run's config.json (or --force-config)")
        log_line(f"resuming from {entry['file']} (step {state['step']}, epoch {state['epoch']}, {state['position']} samples into it)")
    if not (out / "config.json").exists():
        atomic_write_json(out / "config.json", cfg.to_dict())

    torch.manual_seed(cfg.seed)
    np.random.seed(cfg.seed % (2**32))
    random.seed(cfg.seed)
    torch.backends.cudnn.benchmark = True

    dev_ds = open_dataset(cfg.dev_dirs, "dev")

    replay = Replay(cfg.replay_from) if cfg.replay_from else None
    # The first training data: wait for min_train_samples (a producer may still be rendering the first parts).
    t_wait0 = time.perf_counter()
    wait_s = 0.0
    while True:
        try:
            train_ds = scan_train(cfg, replay.cutoff(0) if replay else None)
            if state is not None or train_ds.n >= cfg.min_train_samples:
                break
            have = train_ds.n
        except FileNotFoundError:
            have = 0
        status.update(state="waiting_for_data", phase=f"waiting for {cfg.min_train_samples} training samples ({have} so far)")
        write_status(out, status)
        if _sleep_or_stop(stop, 20.0):
            stop.restore()
            write_status(out, status | {"state": "stopped", "phase": "stopped while waiting for the first data"})
            return {"state": "stopped", "steps": 0}
    wait_s += time.perf_counter() - t_wait0
    if not streams_disjoint(train_ds, dev_ds):
        raise ValueError("train and dev share a stream key: splits are by stream, never by index range")
    if train_ds.jnd_table.tolist() != dev_ds.jnd_table.tolist():
        raise ValueError("train and dev disagree on the JND table")

    model = ProposeV0(cfg.model)
    if state is not None:
        stats = {k: np.asarray(v) for k, v in state["stats"].items()}
    else:
        t0 = time.perf_counter()
        stats = compute_standardisation(train_ds.shards, cfg.stats_shards, cfg.seed)
        log_line(f"standardisation from {int(stats['samples'])} samples in {time.perf_counter() - t0:.1f} s")
        if replay is not None:
            mine = _sha256_bytes(b"".join(np.ascontiguousarray(stats[k]).tobytes() for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std")))
            same = mine == replay.run.get("standardisation_sha256")
            log_line(f"replay of {replay.dir}: standardisation {'identical to' if same else 'DIFFERS from'} the replayed run's")
    model.set_standardisation(*(torch.as_tensor(stats[k]) for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std")))
    model.to(device_t)
    if cfg.channels_last:
        model = model.to(memory_format=torch.channels_last)
    loss_fn = ProposeLoss(train_ds.jnd_table, train_ds.jnd_scale, cfg.loss).to(device_t)
    opt = torch.optim.AdamW(_param_groups(model, cfg.weight_decay), lr=cfg.lr, betas=(0.9, cfg.beta2), fused=device_t.type == "cuda")
    use_amp = cfg.amp and device_t.type == "cuda"
    scaler = torch.amp.GradScaler("cuda", enabled=use_amp)

    if cfg.schedule_steps:
        schedule_steps = cfg.schedule_steps
    elif cfg.train_glob:
        raise ValueError("a run on growing data (train_glob) needs schedule_steps")
    else:
        schedule_steps = cfg.epochs * (train_ds.n // cfg.batch_size)
    budget_s = cfg.max_minutes * 60.0

    # The run's counters; a checkpoint holds all of them.
    step, epoch, position, elapsed0, best_dev, evals_since_best, samples_seen = 0, 0, 0, 0.0, float("inf"), 0, 0
    plan: EpochPlan | None = None
    acc = torch.zeros(4, device=device_t)  # total, hyp, conf, best since the last log line
    win_counts = torch.zeros(cfg.model.hypotheses, device=device_t)
    win_steps = 0
    ep_samples = 0
    if state is not None:
        model.load_state_dict(state["model"])
        opt.load_state_dict(state["opt"])
        scaler.load_state_dict(state["scaler"])
        loss_fn.running.copy_(torch.as_tensor(state["loss_running"]))
        loss_fn._started = bool(state.get("loss_started", True))
        step, epoch, position = state["step"], state["epoch"], state["position"]
        elapsed0, best_dev, evals_since_best = state["elapsed"], state["best_dev"], state["evals_since_best"]
        samples_seen, wait_s = state["samples_seen"], state.get("wait_s", 0.0) + wait_s
        schedule_steps = state["schedule_steps"]
        plan = EpochPlan.from_identity(state["plan"]) if state.get("plan") else None
        if plan is not None and not all(os.path.exists(p) for p, _ in plan.shards):
            # A rolling cache deleted a part this epoch used (only possible when resuming from an old checkpoint):
            # the epoch restarts on the data as it stands. Logged; every other resume is exact.
            log_line(f"the checkpoint's epoch {epoch} uses parts that are gone: the epoch restarts on the current data")
            plan, position = None, 0
        acc.copy_(torch.as_tensor(state["acc"]).to(device_t))
        win_counts.copy_(torch.as_tensor(state["win_counts"]).to(device_t))
        win_steps, ep_samples = state["win_steps"], state["ep_samples"]
        _set_rng_state(state["rng"], device_t)

    dev = DevSet(dev_ds.shards, cfg.dev_limit)
    run = {
        "schema": RUN_SCHEMA,
        "started": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds"),
        "config": cfg.to_dict(),
        "environment": environment(),
        "data": {"train": train_ds.describe(), "dev": dev_ds.describe() | {"in_ram": dev.n}},
        "augmentation": "pre-baked into parts of kind 'clips' (see each part's manifest)" if cfg.train_glob else "none (clean ds1 renders only)",
        "layout": {"id": L.LAYOUT_ID, "sha256": L.LAYOUT_SHA256, "width": L.WIDTH},
        "feature_set": {"id": L.FEATURE_SET_ID, "name": L.FEATURE_SET_NAME},
        "model": {
            "parameters_trainable": count_parameters(model),
            "parameters_exported": exported_parameter_count(model),
            "flops_per_inference": count_flops(model),
            "trunk_shape": list(model.trunk_shape),
        },
        "standardisation_sha256": _sha256_bytes(b"".join(np.ascontiguousarray(stats[k]).tobytes() for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std"))),
        "schedule_steps": schedule_steps,
        "resumed_from": entry if state is not None else None,
        "replay_of": {"run": replay.dir.name, "epochs": len(replay.epochs), "config_diff": cfg.identity_diff(replay.run["config"])} if replay else None,
    }
    (out / ("run.json" if state is None else f"run-resume-{step}.json")).write_text(json.dumps(run, indent=1))
    log_line(json.dumps(run["model"]))

    log = SafeLog(out / "metrics.jsonl")
    if state is not None:
        log.write(json.dumps({"kind": "resume", "step": step, "epoch": epoch, "position": position, "from": entry["file"], "utc": _utc()}) + "\n")
        log.flush()
    gpu = GpuMonitor(out / "gpu.csv") if device_t.type == "cuda" else None
    t_start = time.perf_counter() - elapsed0  # training wall clock: excludes stopped time and waits for data
    last_ckpt_t = time.perf_counter()
    last_ckpt_step = step
    last_status_t = 0.0
    rate_window: list[tuple[float, int]] = []
    final_eval = None
    finished_reason: str | None = None
    available = train_ds.n
    n_parts = len(train_ds.runs)

    def elapsed() -> float:
        return time.perf_counter() - t_start

    def progress_now() -> float:
        # max_minutes <= 0: no time budget, so progress (and the LR and temperature) is a function of the step alone,
        # which is what makes a resumed run's schedule identical to an uninterrupted one's.
        p_steps = step / max(schedule_steps, 1)
        return max(p_steps, elapsed() / budget_s) if budget_s > 0 else p_steps

    def snapshot() -> dict:
        return {
            "model": model.state_dict(),
            "opt": opt.state_dict(),
            "scaler": scaler.state_dict(),
            "loss_running": loss_fn.running.detach().cpu(),
            "loss_started": loss_fn._started,
            "step": step,
            "epoch": epoch,
            "position": position,
            "plan": plan.identity() if plan is not None else None,
            "elapsed": elapsed(),
            "wait_s": wait_s,
            "samples_seen": samples_seen,
            "schedule_steps": schedule_steps,
            "best_dev": best_dev,
            "evals_since_best": evals_since_best,
            "acc": acc.detach().cpu(),
            "win_counts": win_counts.detach().cpu(),
            "win_steps": win_steps,
            "ep_samples": ep_samples,
            "rng": _rng_state(device_t),
            "stats": {k: np.asarray(v) for k, v in stats.items()},
            "config": cfg.to_dict(),
        }

    def save(kind: str, reason: str, dev_loss: float | None = None, must: bool = False) -> dict | None:
        """Write a checkpoint. A failed write is retried; if it keeps failing, training goes on (a periodic one is
        tried again in two minutes) unless ``must`` (a stop), which raises after the retries."""
        nonlocal last_ckpt_t, last_ckpt_step
        t0 = time.perf_counter()
        d = dev_loss if dev_loss is not None else best_dev
        meta = {"step": step, "epoch": epoch, "position": position, "samples_seen": samples_seen, "reason": reason, "dev_best": None if math.isinf(d) else d}
        snap = snapshot()
        e = None
        for attempt in range(5 if must else 3):
            try:
                e = store.save(snap, kind, meta)
                break
            except OSError as err:
                log_line(f"checkpoint write failed ({err!r}), attempt {attempt + 1}; retrying in {CKPT_RETRY_S:.0f} s")
                log.write(json.dumps({"kind": "ckpt_failed", "step": step, "reason": reason, "error": repr(err)[:200], "utc": _utc()}) + "\n")
                time.sleep(CKPT_RETRY_S)
        if e is None:
            status["checkpoint_error"] = {"step": step, "utc": _utc(), "reason": reason}
            if must:
                raise OSError(f"could not write the {reason} checkpoint at step {step}")
            if kind != "best":
                last_ckpt_t = time.perf_counter() - max(cfg.ckpt_every_minutes * 60 - 120.0, 0.0)
            return None
        status.pop("checkpoint_error", None)
        secs = time.perf_counter() - t0
        if kind != "best":
            last_ckpt_t = time.perf_counter()
            last_ckpt_step = step
        log.write(json.dumps({"kind": "ckpt", "file": e["file"], "step": step, "epoch": epoch, "position": position, "reason": reason, "seconds": secs, "bytes": e["bytes"], "utc": e["utc"]}) + "\n")
        log.flush()
        log_line(f"checkpoint {e['file']} ({reason}) at step {step} in {secs:.1f} s")
        status["last_checkpoint"] = {"file": e["file"], "step": step, "utc": e["utc"], "reason": reason}
        return e

    def heartbeat(state_name: str, phase: str = "", force: bool = False) -> None:
        nonlocal last_status_t
        now = time.perf_counter()
        if not force and now - last_status_t < cfg.status_every_s:
            return
        last_status_t = now
        rate = None
        if len(rate_window) >= 2 and rate_window[-1][0] > rate_window[0][0]:
            rate = (rate_window[-1][1] - rate_window[0][1]) * cfg.batch_size / (rate_window[-1][0] - rate_window[0][0])
        eta = None
        if rate:
            eta = max(schedule_steps - step, 0) * cfg.batch_size / rate
            if budget_s > 0:
                eta = min(eta, max(budget_s - elapsed(), 0.0))
        status.update(
            state=state_name,
            phase=phase,
            step=step,
            schedule_steps=schedule_steps,
            epoch=epoch,
            epoch_position=position,
            epoch_samples=plan.samples if plan is not None else None,
            samples_seen=samples_seen,
            train_samples_available=available,
            parts=n_parts,
            samples_per_s=rate,
            best_dev=None if math.isinf(best_dev) else best_dev,
            evals_since_best=evals_since_best,
            elapsed_s=elapsed(),
            wait_s=wait_s,
            eta_s=eta,
            progress=progress_now(),
            device=str(device_t),
        )
        write_status(out, status)

    def do_eval(per_entry: bool) -> tuple[dict, bool]:
        nonlocal best_dev, evals_since_best
        ev = evaluate(model, loss_fn, dev, device_t, cfg, per_entry=per_entry)
        improved = ev["loss_best"] < best_dev - cfg.min_dev_improvement
        if improved:
            best_dev = ev["loss_best"]
            evals_since_best = 0
        else:
            evals_since_best += 1
        return ev, improved

    def early_stop_due() -> bool:
        return bool(cfg.early_stop_patience) and step >= cfg.early_stop_min_steps and evals_since_best >= cfg.early_stop_patience

    try:
        heartbeat("running", "starting", force=True)
        while epoch < cfg.epochs and finished_reason is None and not stop():
            if plan is None:
                # A new epoch: the data as it stands now, after pacing (wait for more parts if the run is ahead).
                t_w = time.perf_counter()
                last_growth = time.perf_counter()
                while True:
                    train_ds = scan_train(cfg, replay.cutoff(epoch) if replay else None)
                    if replay is not None:
                        replay.check(epoch, train_ds)
                    if train_ds.n > available:
                        last_growth = time.perf_counter()
                    available, n_parts = train_ds.n, len(train_ds.runs)
                    ep_n = available // cfg.batch_size * cfg.batch_size
                    paced = cfg.max_repeat > 0 and samples_seen + ep_n > cfg.max_repeat * available
                    complete = cfg.data_target_samples > 0 and available >= cfg.data_target_samples
                    gave_up = time.perf_counter() - last_growth > cfg.data_wait_max_minutes * 60
                    if not paced or complete or gave_up:
                        break
                    heartbeat("waiting_for_data", f"pacing: {samples_seen} seen + {ep_n} > {cfg.max_repeat} x {available} distinct", force=True)
                    if _sleep_or_stop(stop, 20.0):
                        break
                waited = time.perf_counter() - t_w
                if waited > 1.0:
                    wait_s += waited
                    t_start += waited
                    log.write(json.dumps({"kind": "wait", "step": step, "epoch": epoch, "seconds": waited, "available": available, "parts": n_parts}) + "\n")
                    log.flush()
                if stop():
                    break
                plan = EpochPlan(train_ds.shards, cfg.seed, epoch, cfg.buffer_shards, cfg.batch_size)
                position = 0
                ep_samples = 0
                log.write(json.dumps({"kind": "epoch", "epoch": epoch, "step": step, "samples": plan.samples, "parts": n_parts, "available": available, "utc": _utc()}) + "\n")
                log.flush()
            ep_t0 = time.perf_counter()
            win_t0 = time.perf_counter()
            win_data = 0.0
            batches = batches_from(plan, position, cfg.workers, cfg.prefetch, cfg.pin_memory and device_t.type == "cuda", cfg.train_mel_dtype)
            cut = False
            while True:
                td = time.perf_counter()
                b = next(batches, None)
                if b is None:
                    break
                win_data += time.perf_counter() - td
                p = progress_now()
                if p >= 1.0:
                    finished_reason = "budget"
                    cut = True
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
                position += cfg.batch_size
                samples_seen += cfg.batch_size
                ep_samples += cfg.batch_size
                now = time.perf_counter()
                rate_window.append((now, step))
                if len(rate_window) > 200:
                    del rate_window[:-200]
                if step % cfg.log_every_steps == 0:
                    a = (acc / max(win_steps, 1)).tolist()  # one host sync per log interval
                    wall = max(now - win_t0, 1e-9)
                    wc = win_counts / win_counts.sum().clamp_min(1)
                    n_win = max(win_steps, 1)
                    rec = {
                        "kind": "train",
                        "step": step,
                        "epoch": epoch,
                        "elapsed_min": elapsed() / 60.0,
                        "progress": p,
                        "loss": a[0],
                        "hyp": a[1],
                        "conf": a[2],
                        "best": a[3],
                        "lr": cfg.lr * f,
                        "temperature": temp,
                        "grad_norm": float(gn),
                        "scale": scaler.get_scale() if use_amp else 1.0,
                        "step_ms": 1000.0 * wall / n_win,
                        "samples_per_s": n_win * cfg.batch_size / wall,
                        "data_wait_frac": min(win_data / wall, 1.0),
                        "gpu_mem_alloc_mib": torch.cuda.max_memory_allocated() / 2**20 if device_t.type == "cuda" else 0,
                        "gpu_mem_reserved_mib": torch.cuda.max_memory_reserved() / 2**20 if device_t.type == "cuda" else 0,
                        "win_share_max": float(wc.max()),
                        "heads_winning_2pct": int((wc >= 0.02).sum()),
                        "samples_seen": samples_seen,
                    }
                    log.write(json.dumps(rec) + "\n")
                    log.flush()
                    log_line(
                        f"step {step} ep {epoch} {rec['elapsed_min']:.1f}min loss {a[0]:.4f} best {a[3]:.4f} conf {a[2]:.3f} "
                        f"lr {rec['lr']:.2e} T {temp:.3g} {rec['samples_per_s']:.0f} samp/s {rec['step_ms']:.0f} ms/step "
                        f"wait {100 * rec['data_wait_frac']:.1f}% heads {rec['heads_winning_2pct']}"
                    )
                    acc.zero_()
                    win_counts.zero_()
                    win_t0 = time.perf_counter()
                    win_data = 0.0
                    win_steps = 0
                if cfg.eval_every_steps and step % cfg.eval_every_steps == 0:
                    ev, improved = do_eval(per_entry=False)
                    log.write(json.dumps({"kind": "dev", "step": step, "epoch": epoch, "elapsed_min": elapsed() / 60.0, "samples_seen": samples_seen, "improved": improved, **ev}) + "\n")
                    log.flush()
                    log_line(f"== dev at step {step}: best-of-K {ev['loss_best']:.4f} top {ev['loss_top']:.4f} distinct {ev['discrete']['distinct_mean']:.2f}{' (new best)' if improved else ''}")
                    if improved:
                        save("best", "dev improved", ev["loss_best"])
                    if early_stop_due():
                        finished_reason = "early stop"
                if cfg.stop_at_step and step >= cfg.stop_at_step:
                    stop.request(f"stop_at_step {cfg.stop_at_step}")
                heartbeat("running")
                due_t = cfg.ckpt_every_minutes > 0 and time.perf_counter() - last_ckpt_t >= cfg.ckpt_every_minutes * 60
                due_s = cfg.ckpt_every_steps > 0 and step - last_ckpt_step >= cfg.ckpt_every_steps
                if due_t or due_s:
                    save("periodic", "interval")
                if finished_reason is not None or stop():
                    cut = True
                    break
            batches.close()
            if cut:
                break
            # The epoch is complete.
            ep_wall = time.perf_counter() - ep_t0
            if cfg.eval_at_epoch_end:
                ev, improved = do_eval(per_entry=True)
                final_eval = ev
                rec = {"kind": "dev", "step": step, "epoch": epoch, "epoch_complete": True, "epoch_seconds": ep_wall, "epoch_samples": ep_samples, "elapsed_min": elapsed() / 60.0, "samples_seen": samples_seen, "improved": improved, **ev}
                log.write(json.dumps(rec) + "\n")
                log.flush()
                log_line(
                    f"== epoch {epoch} done: {ep_samples} samples in {ep_wall:.0f} s; dev best-of-K {ev['loss_best']:.4f} "
                    f"top {ev['loss_top']:.4f} mean-K {ev['loss_mean_k']:.4f} conf top1 {ev['conf_top1']:.3f} heads {ev['wta']['heads_winning_2pct']} "
                    f"distinct {ev['discrete']['distinct_mean']:.2f} cont med {ev['best']['continuous_median_jnd']:.2f} JND (eval {ev['eval_seconds']:.0f} s)"
                )
                if improved:
                    save("best", "dev improved", ev["loss_best"])
                if early_stop_due():
                    finished_reason = "early stop"
            epoch += 1
            plan = None
            position = 0
        if finished_reason is None and epoch >= cfg.epochs:
            finished_reason = "epochs"
        stopped = finished_reason is None
        if stopped:
            heartbeat("stopping", f"stop requested ({stop.reason})", force=True)
        reason = f"stop ({stop.reason})" if stopped else f"end ({finished_reason})"
        if step != last_ckpt_step or not store.entries("periodic"):
            save("periodic", reason, must=True)
        if not stopped and final_eval is None:
            final_eval, _ = do_eval(per_entry=True)
            log.write(json.dumps({"kind": "dev", "step": step, "epoch": epoch, "final": True, "elapsed_min": elapsed() / 60.0, "samples_seen": samples_seen, **final_eval}) + "\n")
        log.write(json.dumps({"kind": "stop" if stopped else "end", "step": step, "epoch": epoch, "position": position, "reason": reason, "utc": _utc()}) + "\n")
        log.flush()
        heartbeat("stopped" if stopped else "finished", reason, force=True)
    except BaseException as err:
        status.update(state="failed", phase=repr(err)[:300])
        write_status(out, status)
        raise
    finally:
        log.close()
        if gpu is not None:
            gpu.stop()
        stop.restore()

    summary = {
        "state": "stopped" if stopped else "finished",
        "reason": reason,
        "steps": step,
        "epoch": epoch,
        "samples_seen": samples_seen,
        "elapsed_min": elapsed() / 60.0,
        "wait_min": wait_s / 60.0,
        "best_dev_loss": None if math.isinf(best_dev) else best_dev,
        "last_eval": final_eval,
    }
    if not stopped:
        (out / "summary.json").write_text(json.dumps(summary, indent=1))
    return summary


def _sleep_or_stop(stop: StopFlag, seconds: float) -> bool:
    """Sleep up to ``seconds``, waking for a stop request; True when one came."""
    t0 = time.monotonic()
    while time.monotonic() - t0 < seconds:
        if stop():
            return True
        time.sleep(min(1.0, seconds))
    return stop()


def _utc() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


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
