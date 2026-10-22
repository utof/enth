"""Lane TR: the shard stream, the training loop, checkpoints, resume and evaluation, on tiny fake runs (CPU)."""

from __future__ import annotations

import json
import pathlib
import tempfile
import unittest

from train_support import needs_torch


@needs_torch
class Loop(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import numpy as np

        from train_support import write_run

        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = pathlib.Path(cls.tmp.name)
        cls.train_dir = write_run(np, cls.dir / "train", "r3/train", 6, 16, 1)
        cls.dev_dir = write_run(np, cls.dir / "dev", "r3/dev", 2, 16, 2)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def tiny(self, out: str, **kw):
        from enth_ml.model.net import ModelConfig
        from enth_ml.train.config import TrainConfig

        cfg = TrainConfig(
            train_dirs=[str(self.train_dir)],
            dev_dirs=[str(self.dev_dir)],
            out=str(self.dir / out),
            batch_size=8,
            workers=0,
            epochs=3,
            max_minutes=10.0,
            lr=3e-3,
            warmup_steps=5,
            log_every_steps=4,
            stats_shards=4,
            amp=False,
            channels_last=False,
            model=ModelConfig(stem=4, widths=(8, 8, 16, 16), blocks=(0, 1, 0, 1), embed=32, hyp_hidden=16, conf_hidden=16, hypotheses=4),
        )
        for k, v in kw.items():
            setattr(cfg, k, v)
        return cfg

    def test_stream_covers_each_sample_once_per_epoch(self) -> None:
        import numpy as np
        from torch.utils.data import DataLoader

        from enth_ml.train.data import ShardStream
        from enth_ml.train.shards import open_dataset

        ds = open_dataset([self.train_dir])
        for workers in (0, 2):
            s = ShardStream(ds.shards, batch_size=10, seed=3, buffer_shards=2)
            s.set_epoch(1)
            seen = []
            for b in DataLoader(s, batch_size=None, num_workers=workers):
                self.assertEqual(b["mel"].shape[0], 10)
                self.assertEqual(str(b["mel"].dtype), "torch.float16")
                seen.append(b["context"][:, 3].numpy())  # the hold is distinct enough to identify a sample
            seen = np.concatenate(seen)
            self.assertEqual(len(seen), len(np.unique(seen)), "no sample twice in an epoch")
            self.assertGreaterEqual(len(seen), s.batches_per_epoch(max(workers, 1)) * 10)
        a = [b["context"][:, 3].numpy().tolist() for b in ShardStream(ds.shards, 10, seed=3, buffer_shards=2)]
        s2 = ShardStream(ds.shards, 10, seed=3, buffer_shards=2)
        s2.set_epoch(1)
        b2 = [b["context"][:, 3].numpy().tolist() for b in s2]
        self.assertNotEqual(a, b2, "each epoch has its own order")

    def test_train_learns_checkpoints_and_resumes(self) -> None:
        from enth_ml.train.loop import evaluate_checkpoint, train

        cfg = self.tiny("run-a", epochs=4)
        s = train(cfg, device="cpu")
        out = pathlib.Path(cfg.out)
        for f in ("run.json", "metrics.jsonl", "ckpt-last.pt", "ckpt-best.pt", "summary.json"):
            self.assertTrue((out / f).exists(), f)
        recs = [json.loads(line) for line in (out / "metrics.jsonl").read_text().splitlines()]
        dev = [r for r in recs if r["kind"] == "dev"]
        self.assertEqual(len(dev), 4)
        self.assertLess(dev[-1]["loss_best"], dev[0]["loss_best"], "dev loss falls on a learnable fake")
        train_recs = [r for r in recs if r["kind"] == "train"]
        self.assertTrue(all(0.0 <= r["data_wait_frac"] <= 1.0 for r in train_recs))
        run = json.loads((out / "run.json").read_text())
        self.assertEqual(run["schema"], "enth.train-run/1")
        self.assertEqual(run["data"]["train"]["samples"], 96)
        self.assertNotIn(str(pathlib.Path.home()), json.dumps(run["environment"]), "no local path in the environment block")
        ev = dev[-1]
        for key in ("best", "top", "wta", "conf"):
            self.assertIn(key, ev)
        self.assertEqual(len(ev["wta"]["win_share"]), 4)
        self.assertIn("osc1.semi", ev["best"]["continuous_per_entry"])
        # Resume: the step count continues and one more epoch runs.
        cfg2 = self.tiny("run-a", epochs=5)
        s2 = train(cfg2, resume=True, device="cpu")
        self.assertGreater(s2["steps"], s["steps"])
        self.assertTrue(any(p.name.startswith("run-resume-") for p in out.iterdir()))
        import contextlib
        import io

        from enth_ml.train import report

        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            report.main([str(out)])
        self.assertIn("| Epoch |", buf.getvalue())
        self.assertIn("samples_per_s_median", buf.getvalue())
        res = evaluate_checkpoint(out / "ckpt-best.pt", device="cpu")
        res16 = evaluate_checkpoint(out / "ckpt-best.pt", device="cpu", mel_dtype="f2")
        self.assertLess(abs(res["loss_best"] - res16["loss_best"]), 0.05 * res["loss_best"] + 1e-3)

    def test_time_budget_ends_the_run(self) -> None:
        from enth_ml.train.loop import train

        cfg = self.tiny("run-b", epochs=50, max_minutes=0.05)
        s = train(cfg, device="cpu")
        self.assertLess(s["steps"], 50 * 12)

    def test_same_stream_is_refused(self) -> None:
        from enth_ml.train.loop import train

        cfg = self.tiny("run-c")
        cfg.dev_dirs = [str(self.train_dir)]
        with self.assertRaises(ValueError):
            train(cfg, device="cpu")


if __name__ == "__main__":
    unittest.main()
