"""Lane TR2: a stopped run resumes exactly, a damaged checkpoint falls back, rotation keeps two plus the best (CPU).

(a) stop at step k, resume, run to the end: the same final weights and the same logged losses as a run that never
    stopped (mid-epoch, at an epoch boundary, twice, and with two DataLoader workers);
(c) a truncated or bit-flipped newest checkpoint: the resume falls back to the older one (and to best.pt when both
    periodic ones are damaged), and still ends identical;
(d) rotation keeps exactly two periodic checkpoints and best.pt, each with a checksum in the manifest.
The kill -9 and the stop command, STOP file and signals run real processes: test_train_resume_proc.py.
"""

from __future__ import annotations

import json
import pathlib
import tempfile
import unittest

from train_support import needs_torch

LOSS_KEYS = ("loss", "hyp", "conf", "best", "lr", "temperature", "grad_norm", "heads_winning_2pct", "samples_seen")


def tiny_cfg(train_dir, dev_dir, out, **kw):
    from enth_ml.model.net import ModelConfig
    from enth_ml.train.config import TrainConfig

    cfg = TrainConfig(
        train_dirs=[str(train_dir)],
        dev_dirs=[str(dev_dir)],
        out=str(out),
        batch_size=8,
        workers=0,
        buffer_shards=2,
        epochs=3,
        max_minutes=0.0,  # no time budget: the schedule follows steps only, so it is deterministic
        lr=3e-3,
        warmup_steps=5,
        log_every_steps=4,
        eval_every_steps=10,
        stats_shards=4,
        ckpt_every_minutes=0.0,
        status_every_s=0.0,
        stop_poll_s=0.0,
        amp=False,
        channels_last=False,
        model=ModelConfig(stem=4, widths=(8, 8, 16, 16), blocks=(0, 1, 0, 1), embed=32, hyp_hidden=16, conf_hidden=16, hypotheses=4),
    )
    for k, v in kw.items():
        setattr(cfg, k, v)
    return cfg


def records(out: pathlib.Path) -> tuple[dict, dict]:
    """The last train and dev record of each step (a resume after a crash logs some steps twice)."""
    tr, dv = {}, {}
    for line in (out / "metrics.jsonl").read_text().splitlines():
        r = json.loads(line)
        if r["kind"] == "train":
            tr[r["step"]] = {k: r[k] for k in LOSS_KEYS}
        elif r["kind"] == "dev":
            dv[(r["step"], bool(r.get("epoch_complete")))] = r["loss_best"]
    return tr, dv


def final_weights(out: pathlib.Path) -> dict:
    import torch

    from enth_ml.train.ckpt import CheckpointStore

    state, _ = CheckpointStore(out).load_latest()
    return {"model": state["model"], "opt": state["opt"]["state"], "step": state["step"], "running": torch.as_tensor(state["loss_running"])}


@needs_torch
class Resume(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import numpy as np

        from train_support import write_run

        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = pathlib.Path(cls.tmp.name)
        cls.train_dir = write_run(np, cls.dir / "train", "r3/train", 6, 15, 1)  # 90 samples: windows do not align with batches
        cls.dev_dir = write_run(np, cls.dir / "dev", "r3/dev", 2, 16, 2)
        cls.refs: dict = {}

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def cfg(self, out: str, **kw):
        return tiny_cfg(self.train_dir, self.dev_dir, self.dir / out, **kw)

    def reference(self, workers: int = 0):
        """The uninterrupted run (once per worker count)."""
        from enth_ml.train.loop import train

        if workers not in self.refs:
            out = self.dir / f"ref-w{workers}"
            s = train(self.cfg(out.name, workers=workers), device="cpu")
            self.assertEqual(s["state"], "finished")
            self.refs[workers] = (s, final_weights(out), records(out))
        return self.refs[workers]

    def assert_same(self, out: pathlib.Path, workers: int = 0) -> None:
        import torch

        s_ref, w_ref, (tr_ref, dv_ref) = self.reference(workers)
        w = final_weights(out)
        self.assertEqual(w["step"], w_ref["step"])
        for k, v in w_ref["model"].items():
            self.assertTrue(torch.equal(v, w["model"][k]), f"model tensor {k} differs")
        for pid, st in w_ref["opt"].items():
            for k, v in st.items():
                self.assertTrue(torch.equal(torch.as_tensor(v), torch.as_tensor(w["opt"][pid][k])), f"optimiser state {pid}/{k} differs")
        self.assertTrue(torch.equal(w["running"], w_ref["running"]))
        tr, dv = records(out)
        self.assertEqual(tr, tr_ref, "logged training losses differ")
        self.assertEqual(dv, dv_ref, "logged dev losses differ")

    def stop_then_resume(self, name: str, stops: list[int], workers: int = 0) -> pathlib.Path:
        from enth_ml.train.loop import train

        for k in stops:
            s = train(self.cfg(name, workers=workers, stop_at_step=k), device="cpu")
            self.assertEqual((s["state"], s["steps"]), ("stopped", k))
            st = json.loads((self.dir / name / "status.json").read_text())
            self.assertEqual((st["state"], st["step"]), ("stopped", k))
        s = train(self.cfg(name, workers=workers), device="cpu")
        self.assertEqual(s["state"], "finished")
        return self.dir / name

    # (a) -------------------------------------------------------------------------------------------------------------
    def test_a_stop_mid_epoch_resumes_identically(self) -> None:
        s_ref, _, _ = self.reference()
        self.assertEqual(s_ref["steps"], 3 * (90 // 8))
        self.assert_same(self.stop_then_resume("stop-17", [17]))

    def test_a_stop_at_epoch_boundary_and_twice(self) -> None:
        self.assert_same(self.stop_then_resume("stop-11", [11]))  # the last step of epoch 0 (before its dev eval)
        self.assert_same(self.stop_then_resume("stop-5-23", [5, 23]))

    def test_a_stop_with_workers(self) -> None:
        self.assert_same(self.stop_then_resume("stop-w2", [9], workers=2), workers=2)
        # The sample order does not depend on the number of workers: the 0-worker run gives the same weights.
        import torch

        w0 = self.reference(0)[1]["model"]
        w2 = self.reference(2)[1]["model"]
        self.assertTrue(all(torch.equal(w0[k], w2[k]) for k in w0))

    # (c) -------------------------------------------------------------------------------------------------------------
    def test_c_damaged_newest_checkpoint_falls_back(self) -> None:
        from enth_ml.train.ckpt import CheckpointStore
        from enth_ml.train.loop import train

        for damage in ("truncate", "flip"):
            name = f"damaged-{damage}"
            s = train(self.cfg(name, ckpt_every_steps=5, stop_at_step=17), device="cpu")
            self.assertEqual(s["steps"], 17)
            store = CheckpointStore(self.dir / name)
            steps = sorted(e["step"] for e in store.entries("periodic"))
            self.assertEqual(steps, [15, 17])
            newest = store.dir / "periodic-000000017.pt"
            data = bytearray(newest.read_bytes())
            if damage == "truncate":
                newest.write_bytes(bytes(data[: len(data) // 2]))  # a crash mid-write without the atomic rename
            else:
                data[len(data) // 2] ^= 0xFF
                newest.write_bytes(bytes(data))
            state, entry = store.load_latest(log=lambda m: None)
            self.assertEqual((entry["file"], state["step"]), ("periodic-000000015.pt", 15))
            s = train(self.cfg(name, ckpt_every_steps=5), device="cpu")
            self.assertEqual(s["state"], "finished")
            self.assert_same(self.dir / name)

    def test_c_both_periodic_damaged_falls_back_to_best(self) -> None:
        from enth_ml.train.ckpt import CheckpointStore
        from enth_ml.train.loop import train

        name = "damaged-both"
        train(self.cfg(name, ckpt_every_steps=5, stop_at_step=17), device="cpu")
        store = CheckpointStore(self.dir / name)
        for e in store.entries("periodic"):
            (store.dir / e["file"]).write_bytes(b"not a checkpoint")
        state, entry = store.load_latest(log=lambda m: None)
        self.assertEqual(entry["file"], "best.pt")
        self.assertIn(state["step"], (10, 11))  # the best dev evaluation so far (every 10 steps and at epoch ends)
        (store.dir / "manifest.json").write_text("{ torn")  # a lost manifest: the sidecars still hold the checksums
        state2, entry2 = store.load_latest(log=lambda m: None)
        self.assertEqual((entry2["file"], state2["step"]), ("best.pt", state["step"]))
        s = train(self.cfg(name, ckpt_every_steps=5), device="cpu")
        self.assertEqual(s["state"], "finished")
        self.assert_same(self.dir / name)

    # (d) -------------------------------------------------------------------------------------------------------------
    def test_d_rotation_keeps_two_and_the_best(self) -> None:
        from enth_ml.train.ckpt import CheckpointStore
        from enth_ml.train.loop import train

        name = "rotation"
        train(self.cfg(name, ckpt_every_steps=3), device="cpu")
        store = CheckpointStore(self.dir / name)
        files = sorted(p.name for p in store.dir.iterdir())
        periodic = [f for f in files if f.startswith("periodic-") and f.endswith(".pt")]
        self.assertEqual(periodic, ["periodic-000000030.pt", "periodic-000000033.pt"])
        self.assertIn("best.pt", files)
        self.assertEqual(len([f for f in files if f.endswith(".pt")]), 3)
        self.assertFalse([f for f in files if f.endswith(".tmp")])
        m = json.loads((store.dir / "manifest.json").read_text())
        self.assertEqual(sorted(e["file"] for e in m["entries"]), ["best.pt", *periodic])
        for e in m["entries"]:
            self.assertTrue(store.verify(e), e["file"])
            self.assertEqual(len(e["sha256"]), 64)
        dev = [json.loads(x) for x in (self.dir / name / "metrics.jsonl").read_text().splitlines() if '"kind": "dev"' in x]
        best = min(dev, key=lambda r: r["loss_best"])
        self.assertEqual(store.best()["step"], best["step"])
        self.assertAlmostEqual(store.best()["dev_best"], best["loss_best"])

    def test_growing_parts_and_pacing(self) -> None:
        """Parts found by train_glob at each epoch start; pacing waits are skipped once the target is met."""
        import numpy as np

        from enth_ml.train.loop import train
        from train_support import write_run

        parts = self.dir / "parts"
        write_run(np, parts / "train-s1-f16", "r3/train", 2, 15, 11)
        (parts / "train-s2-f16.tmp").mkdir(parents=True)  # a part still being written is ignored
        cfg = self.cfg("glob", train_dirs=[], train_glob=str(parts / "train-s*-f16"), schedule_steps=40, epochs=4, max_repeat=1e9)
        s = train(cfg, device="cpu")
        self.assertEqual(s["state"], "finished")
        epochs = [json.loads(x) for x in (self.dir / "glob" / "metrics.jsonl").read_text().splitlines() if '"kind": "epoch"' in x]
        self.assertEqual([e["available"] for e in epochs], [30] * 4)


if __name__ == "__main__":
    unittest.main()
