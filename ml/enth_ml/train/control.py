"""Stopping a run gracefully and asking how it is doing (lane TR2).

A run stops at the end of its current step, writes a checkpoint and exits with ``EXIT_STOPPED`` when any of these
happens: ``python -m enth_ml.train stop RUN_DIR`` (which does both of the next two), a ``RUN_DIR/STOP`` file, or a
SIGTERM or SIGINT to the training process. Starting or resuming a run removes a stale ``STOP`` file.

The training process rewrites ``RUN_DIR/status.json`` (atomically) every few seconds: its PID, the step, epoch,
samples seen, the last checkpoint, dev best, throughput and an ETA. ``python -m enth_ml.train status RUN_DIR`` prints it
with whether the process is alive (the PID exists and is a training process, and the heartbeat is recent).
"""

from __future__ import annotations

import json
import os
import pathlib
import signal
import sys
import time

from .ckpt import CheckpointStore, atomic_write_json

STOP_FILE = "STOP"
STATUS_FILE = "status.json"
EXIT_STOPPED = 75  # EX_TEMPFAIL: stopped on request, resumable
HEARTBEAT_STALE_S = 180.0


class StopFlag:
    """Set by SIGTERM, SIGINT or the run directory's STOP file (checked at most every ``poll_s`` seconds)."""

    def __init__(self, run_dir: pathlib.Path, poll_s: float = 2.0) -> None:
        self.path = pathlib.Path(run_dir) / STOP_FILE
        self.poll_s = poll_s
        self.reason: str | None = None
        self._last = 0.0
        self._old: dict = {}

    def install(self) -> None:
        self.path.unlink(missing_ok=True)
        for sig in (signal.SIGTERM, signal.SIGINT):
            try:
                self._old[sig] = signal.signal(sig, self._on_signal)
            except ValueError:  # not the main thread (tests that train in a thread): the STOP file still works
                pass

    def restore(self) -> None:
        for sig, h in self._old.items():
            signal.signal(sig, h)
        self._old.clear()

    def _on_signal(self, signum, frame) -> None:
        self.reason = self.reason or signal.Signals(signum).name

    def request(self, reason: str) -> None:
        self.reason = self.reason or reason

    def __call__(self) -> bool:
        if self.reason is None:
            now = time.monotonic()
            if now - self._last >= self.poll_s:
                self._last = now
                if self.path.exists():
                    self.reason = "STOP file"
        return self.reason is not None


def write_status(run_dir: pathlib.Path, status: dict) -> None:
    atomic_write_json(pathlib.Path(run_dir) / STATUS_FILE, status | {"pid": os.getpid(), "heartbeat_unix": time.time()})


def _proc_cmdline(pid: int) -> str:
    try:
        return pathlib.Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
    except OSError:
        return ""


def process_alive(pid: int | None) -> bool:
    """The PID exists and is a training process (``enth_ml.train`` in its command line, where /proc tells)."""
    if not pid:
        return False
    try:
        os.kill(int(pid), 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:  # a zombie (exited, not yet reaped by its parent) is not alive
        if pathlib.Path(f"/proc/{int(pid)}/stat").read_text().rsplit(")", 1)[1].split()[0] == "Z":
            return False
    except (OSError, IndexError):
        pass
    cmd = _proc_cmdline(int(pid))
    return (not cmd) or ("enth_ml.train" in cmd)


def read_status(run_dir: pathlib.Path) -> dict:
    run_dir = pathlib.Path(run_dir)
    try:
        st = json.loads((run_dir / STATUS_FILE).read_text())
    except (OSError, ValueError):
        st = {}
    alive = process_alive(st.get("pid")) and st.get("state") not in ("finished", "stopped", "failed")
    age = time.time() - float(st.get("heartbeat_unix", 0)) if st else None
    store = CheckpointStore(run_dir)
    ck = store.candidates()
    st["alive"] = bool(alive)
    st["heartbeat_age_s"] = age
    if alive and age is not None and age > HEARTBEAT_STALE_S:
        st["warning"] = f"the process exists but its heartbeat is {age:.0f} s old (hung, or waiting on a disk)"
    st["checkpoints"] = [{k: e.get(k) for k in ("file", "kind", "step", "epoch", "utc", "reason", "dev_best")} for e in ck]
    st["stop_file"] = (run_dir / STOP_FILE).exists()
    return st


def _fmt_s(s: float | None) -> str:
    if s is None:
        return "-"
    s = int(s)
    return f"{s // 3600}h{(s % 3600) // 60:02d}m" if s >= 3600 else f"{s // 60}m{s % 60:02d}s"


def status_main(argv: list[str]) -> int:
    import argparse

    ap = argparse.ArgumentParser(prog="python -m enth_ml.train status")
    ap.add_argument("run")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    st = read_status(pathlib.Path(a.run))
    if a.json:
        print(json.dumps(st, indent=1))
        return 0
    if not st.get("pid"):
        print(f"{a.run}: no status yet (never started here?)")
    state = "RUNNING" if st["alive"] else ("not running: " + str(st.get("state", "unknown")))
    print(f"run       {a.run}")
    print(f"process   {state} (pid {st.get('pid')}, heartbeat {_fmt_s(st.get('heartbeat_age_s'))} ago)" + (f"  WARNING: {st['warning']}" if st.get("warning") else ""))
    if st.get("phase"):
        print(f"phase     {st['phase']}")
    print(f"step      {st.get('step')} of {st.get('schedule_steps')} scheduled  (epoch {st.get('epoch')}, {st.get('epoch_position', 0)}/{st.get('epoch_samples', '?')} samples into it)")
    print(f"seen      {st.get('samples_seen', 0) / 1e6:.2f} M samples; {st.get('train_samples_available', 0) / 1e6:.2f} M distinct available in {st.get('parts', '?')} parts")
    rate = st.get("samples_per_s")
    print(f"speed     {rate:.0f} samples/s" if rate else "speed     -")
    print(f"dev best  {st.get('best_dev')} (best-of-K; evals since best: {st.get('evals_since_best')})")
    print(f"elapsed   {_fmt_s(st.get('elapsed_s'))} training, ETA {_fmt_s(st.get('eta_s'))}")
    for c in st["checkpoints"]:
        print(f"ckpt      {c['file']:<26} step {c.get('step')} epoch {c.get('epoch')} {c.get('utc')} {c.get('reason') or ''} {('dev ' + format(c['dev_best'], '.4f')) if c.get('dev_best') is not None else ''}")
    if st["stop_file"]:
        print("STOP file present: the run stops (or stopped) at the end of its step")
    return 0


def stop_main(argv: list[str]) -> int:
    import argparse

    ap = argparse.ArgumentParser(prog="python -m enth_ml.train stop")
    ap.add_argument("run")
    ap.add_argument("--wait", type=float, default=180.0, help="seconds to wait for the process to exit (0: do not wait)")
    a = ap.parse_args(argv)
    run = pathlib.Path(a.run)
    (run / STOP_FILE).write_text(time.strftime("%Y-%m-%dT%H:%M:%SZ\n", time.gmtime()))
    st = read_status(run)
    pid = st.get("pid")
    t0 = time.monotonic()
    if st["alive"]:
        try:
            os.kill(int(pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
        print(f"stop requested: STOP file written, SIGTERM sent to pid {pid}", flush=True)
    else:
        print(f"no live training process (pid {pid}); STOP file written so a starting run stops at once", flush=True)
        return 0
    if a.wait <= 0:
        return 0
    while time.monotonic() - t0 < a.wait:
        if not process_alive(pid):
            st = read_status(run)
            last = st["checkpoints"][0] if st["checkpoints"] else None
            print(f"stopped after {time.monotonic() - t0:.1f} s; state {st.get('state')}; newest checkpoint {last}", flush=True)
            return 0
        time.sleep(0.5)
    print(f"still running after {a.wait:.0f} s (pid {pid}); the STOP file stays: it stops at the end of its step", flush=True)
    return 1


if __name__ == "__main__":
    sys.exit(status_main(sys.argv[1:]))
