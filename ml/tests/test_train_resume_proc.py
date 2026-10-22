"""Lane TR2: real training processes (CPU, a tiny model): kill -9, the stop command, the STOP file, signals.

(b) ``kill -9`` mid-run: the resume starts from the last periodic checkpoint (at most one interval lost), leaves no
    orphan process (DataLoader workers included), and ends with the same weights as a run that was never killed;
(e) ``python -m enth_ml.train stop``, a ``RUN_DIR/STOP`` file, SIGTERM and SIGINT each stop a run within the bound,
    with a checkpoint and exit status 75; ``status`` reports the run (alive while it runs, not after).
"""

from __future__ import annotations

import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest

from train_support import needs_torch

ML = pathlib.Path(__file__).resolve().parents[1]
STOP_BOUND_S = 60.0  # the bound the coordinator relies on ("within about a minute"); a tiny CPU run takes ~1 s


def _env() -> dict:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(ML) + os.pathsep + env.get("PYTHONPATH", "")
    env["OMP_NUM_THREADS"] = "2"  # the same threads in every process: CPU kernels give the same bits
    return env


def _cmd(*args: str) -> list[str]:
    return [sys.executable, "-m", "enth_ml.train", *args]


def _wait_for(pred, timeout: float, what: str):
    t0 = time.monotonic()
    while time.monotonic() - t0 < timeout:
        v = pred()
        if v:
            return v
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


def _status(run: pathlib.Path) -> dict:
    try:
        return json.loads((run / "status.json").read_text())
    except (OSError, ValueError):
        return {}


def _procs_with(marker: str) -> list[int]:
    pids = []
    for d in pathlib.Path("/proc").iterdir():
        if d.name.isdigit() and int(d.name) != os.getpid():
            try:
                cmd = (d / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
                state = (d / "stat").read_text().rsplit(")", 1)[1].split()[0]
            except (OSError, IndexError):
                continue
            if marker in cmd and state != "Z":
                pids.append(int(d.name))
    return pids


@needs_torch
@unittest.skipUnless(sys.platform.startswith("linux"), "uses /proc and POSIX signals")
class Processes(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import numpy as np

        from test_train_resume import tiny_cfg
        from train_support import write_run

        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = pathlib.Path(cls.tmp.name)
        cls.train_dir = write_run(np, cls.dir / "train", "r3/train", 8, 15, 1)
        cls.dev_dir = write_run(np, cls.dir / "dev", "r3/dev", 2, 16, 2)
        cls.tiny_cfg = staticmethod(tiny_cfg)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def write_cfg(self, name: str, **kw) -> pathlib.Path:
        cfg = self.tiny_cfg(self.train_dir, self.dev_dir, self.dir / name, **kw)
        path = self.dir / f"{name}.json"
        path.write_text(json.dumps(cfg.to_dict()))
        return path

    def start(self, cfg_path: pathlib.Path, log: str) -> subprocess.Popen:
        fh = open(self.dir / log, "ab")
        self.addCleanup(fh.close)
        p = subprocess.Popen(_cmd("run", "--config", str(cfg_path), "--device", "cpu"), cwd=ML, env=_env(), stdout=fh, stderr=subprocess.STDOUT)
        self.addCleanup(lambda: p.poll() is None and p.kill())
        return p

    def final_model(self, run: pathlib.Path) -> dict:
        from enth_ml.train.ckpt import CheckpointStore

        state, _ = CheckpointStore(run).load_latest(log=lambda m: None)
        return state

    # (b) -------------------------------------------------------------------------------------------------------------
    def test_b_kill9_resumes_from_last_checkpoint(self) -> None:
        import torch

        from enth_ml.train.ckpt import CheckpointStore

        interval = 5
        kw = dict(workers=2, epochs=4, ckpt_every_steps=interval)
        ref = self.start(self.write_cfg("k-ref", **kw), "k-ref.log")
        self.assertEqual(ref.wait(timeout=600), 0, (self.dir / "k-ref.log").read_text()[-3000:])

        run = self.dir / "k-run"
        cfg = self.write_cfg("k-run", **kw)
        p = self.start(cfg, "k-run.log")
        store = CheckpointStore(run)
        _wait_for(lambda: any(e.get("step", 0) >= 2 * interval for e in store.entries("periodic")), 300, "two checkpoint intervals")
        workers = [q for q in _procs_with(str(cfg)) if q != p.pid]
        os.kill(p.pid, signal.SIGKILL)
        p.wait(timeout=30)
        killed_at = _status(run).get("step")
        self.assertIsNotNone(killed_at)
        # No orphan: the DataLoader workers and the GPU monitor go with the trainer.
        _wait_for(lambda: not _procs_with(str(cfg)), 30, f"the trainer's children {workers} to exit")
        state, entry = store.load_latest(log=lambda m: None)
        self.assertLessEqual(killed_at - state["step"], interval, "at most one interval is lost")
        p2 = self.start(cfg, "k-run.log")
        self.assertEqual(p2.wait(timeout=600), 0, (self.dir / "k-run.log").read_text()[-3000:])
        recs = [json.loads(x) for x in (run / "metrics.jsonl").read_text().splitlines()]
        resumes = [r for r in recs if r["kind"] == "resume"]
        self.assertEqual([r["step"] for r in resumes], [state["step"]])
        a, b = self.final_model(self.dir / "k-ref"), self.final_model(run)
        self.assertEqual(a["step"], b["step"])
        for k, v in a["model"].items():
            self.assertTrue(torch.equal(v, b["model"][k]), f"{k} differs after kill -9 and resume")

    # (e) -------------------------------------------------------------------------------------------------------------
    def _stop_and_time(self, name: str, how: str) -> float:
        from enth_ml.train.ckpt import CheckpointStore

        run = self.dir / name
        cfg = self.write_cfg(name, epochs=10_000, workers=1)  # would run for hours
        p = self.start(cfg, f"{name}.log")
        _wait_for(lambda: _status(run).get("step", 0) >= 3, 300, "the run to take steps")
        st = subprocess.run(_cmd("status", str(run), "--json"), cwd=ML, env=_env(), capture_output=True, text=True, timeout=120)
        self.assertTrue(json.loads(st.stdout)["alive"], st.stdout)
        t0 = time.monotonic()
        if how == "command":
            r = subprocess.run(_cmd("stop", str(run), "--wait", "120"), cwd=ML, env=_env(), capture_output=True, text=True, timeout=300)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("stopped after", r.stdout)
        elif how == "file":
            (run / "STOP").write_text("stop\n")
        else:
            os.kill(p.pid, getattr(signal, how))
        rc = p.wait(timeout=STOP_BOUND_S + 30)
        dt = time.monotonic() - t0
        self.assertEqual(rc, 75, (self.dir / f"{name}.log").read_text()[-2000:])
        self.assertLess(dt, STOP_BOUND_S)
        newest = CheckpointStore(run).candidates()[0]
        self.assertTrue(newest["reason"].startswith("stop"), newest)
        self.assertEqual(newest["step"], _status(run)["step"])
        st = subprocess.run(_cmd("status", str(run), "--json"), cwd=ML, env=_env(), capture_output=True, text=True, timeout=120)
        js = json.loads(st.stdout)
        self.assertFalse(js["alive"])
        self.assertEqual(js["state"], "stopped")
        return dt

    def test_e_stop_command_file_and_signals(self) -> None:
        times = {how: self._stop_and_time(f"stop-{how.lower()}", how) for how in ("command", "file", "SIGTERM", "SIGINT")}
        print(f"\nstop latency (s): {json.dumps({k: round(v, 2) for k, v in times.items()})}", file=sys.stderr)
        # A resume of a stopped run removes the stale STOP file and carries on from the stop checkpoint.
        run = self.dir / "stop-file"
        before = _status(run)["step"]
        p = subprocess.Popen(_cmd("resume", str(run), "--device", "cpu", "--set", f"stop_at_step={before + 4}"), cwd=ML, env=_env(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.assertEqual(p.wait(timeout=300), 75)
        self.assertEqual(_status(run)["step"], before + 4)
        self.assertFalse((run / "STOP").exists())
        out = subprocess.run(_cmd("status", str(run)), cwd=ML, env=_env(), capture_output=True, text=True, timeout=120).stdout
        self.assertIn("not running: stopped", out)
        self.assertIn("periodic-", out)


if __name__ == "__main__":
    unittest.main()
