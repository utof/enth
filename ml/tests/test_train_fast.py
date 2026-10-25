"""Lane FTL, the fast loop: schedules (WSD, anneal by steps, weight decay as a timescale) with the old path bit for bit,
cooldown branches that resume exactly, per-sample dev files, the final evaluation's best.pt, the RAM-resident slice,
config inheritance (proxy-v0.toml) and the T0 smoke (CPU, tiny fake runs).
"""

from __future__ import annotations

import json
import math
import pathlib
import tempfile
import unittest

from train_support import needs_numpy, needs_torch
from test_train_resume import final_weights, records, tiny_cfg

ML = pathlib.Path(__file__).resolve().parents[1]
CONFIGS = ML / "enth_ml" / "train"


# The schedule and the temperature as every run before lane FTL computed them (loop.py's lr_factor and LossConfig's
# temperature at main 1db51b0), copied verbatim: the new code must give the same floats for the old configs.
def old_lr_factor(progress: float, step: int, warmup: int, min_frac: float) -> float:
    if step < warmup:
        return (step + 1) / warmup
    p = min(max(progress, 0.0), 1.0)
    return min_frac + (1.0 - min_frac) * 0.5 * (1.0 + math.cos(math.pi * p))


def old_temperature(c, progress: float) -> float:
    if c.anneal_frac <= 0 or progress >= c.anneal_frac:
        return 0.0
    r = progress / c.anneal_frac
    return float(c.t_start * (c.t_end / c.t_start) ** r)


@needs_numpy
class Schedules(unittest.TestCase):
    def cfg(self, **kw):
        from enth_ml.train.config import TrainConfig

        c = TrainConfig(warmup_steps=10, min_lr_frac=0.02)
        for k, v in kw.items():
            setattr(c, k, v)
        return c

    def test_existing_configs_keep_their_schedule_exactly(self) -> None:
        from enth_ml.train import schedule as S
        from enth_ml.train.config import TrainConfig

        for name in ("full-v0-a.toml", "full-v0-b.toml", "pilot-v0.toml"):
            cfg = TrainConfig.load(CONFIGS / name)
            self.assertEqual((cfg.lr_schedule, cfg.cooldown_steps, cfg.loss.anneal_steps, cfg.weight_decay_tau_samples), ("cosine", 0, 0, 0.0))
            self.assertEqual(S.weight_decay(cfg), cfg.weight_decay)
            total = cfg.schedule_steps or 20000  # the pilot's schedule follows its time budget: any progress grid
            for step in range(total + 1):
                p = step / total
                self.assertEqual(S.lr_factor(cfg, p, step, total), old_lr_factor(p, step, cfg.warmup_steps, cfg.min_lr_frac), f"{name} step {step}")
                self.assertEqual(cfg.loss.temperature(p, step), old_temperature(cfg.loss, p), f"{name} step {step}")
            for p in (0.0, 0.1234567, 0.3, 0.29999999, 0.5, 0.9999, 1.0, 1.3):  # a wall-clock progress, any value
                self.assertEqual(S.lr_factor(cfg, p, 5000, total), old_lr_factor(p, 5000, cfg.warmup_steps, cfg.min_lr_frac))
                self.assertEqual(cfg.loss.temperature(p, 5000), old_temperature(cfg.loss, p))

    def test_wsd_shape(self) -> None:
        from enth_ml.train import schedule as S

        c = self.cfg(lr_schedule="wsd", decay_frac=0.2)
        total = 100
        f = [S.lr_factor(c, s / total, s, total) for s in range(total + 1)]
        self.assertEqual(f[:10], [(s + 1) / 10 for s in range(10)], "the warmup is the cosine's")
        self.assertTrue(all(x == 1.0 for x in f[10:81]), "constant through the stable phase, up to the decay's start at 80")
        self.assertAlmostEqual(f[100], 0.02, places=12)
        self.assertAlmostEqual(f[90], 0.02 + 0.98 * (1 - math.sqrt(0.5)), places=12, msg="1 - sqrt at the decay's middle")
        self.assertTrue(all(a > b for a, b in zip(f[80:], f[81:])), "decreasing over the decay")
        lin = self.cfg(lr_schedule="wsd", decay_frac=0.2, decay_shape="linear")
        self.assertAlmostEqual(S.lr_factor(lin, 0.9, 90, total), 0.02 + 0.98 * 0.5, places=12)
        cos = self.cfg(lr_schedule="wsd", decay_steps=40, decay_shape="cosine")
        self.assertEqual(S.lr_factor(cos, 0.6, 60, total), 1.0)
        self.assertAlmostEqual(S.lr_factor(cos, 0.8, 80, total), 0.02 + 0.98 * 0.5, places=12)
        with self.assertRaises(ValueError):
            S.check(self.cfg(lr_schedule="step"))
        with self.assertRaises(ValueError):
            S.check(self.cfg(decay_shape="exp"))

    def test_cooldown_factor(self) -> None:
        from enth_ml.train import schedule as S

        c = self.cfg(lr_schedule="wsd", decay_frac=0.2, cooldown_from=40, cooldown_steps=20)
        total = 100
        self.assertEqual(S.lr_factor(c, 0.39, 39, total), 1.0)
        self.assertEqual(S.lr_factor(c, 0.4, 40, total), 1.0)
        self.assertAlmostEqual(S.lr_factor(c, 0.5, 50, total), 0.02 + 0.98 * (1 - math.sqrt(0.5)), places=12)
        self.assertAlmostEqual(S.lr_factor(c, 0.6, 60, total), 0.02, places=12)
        self.assertEqual(S.end_step(c), 60)
        # From a cosine parent: the decay starts at the parent's own factor at the branch.
        cc = self.cfg(cooldown_from=50, cooldown_steps=10, decay_shape="linear")
        f0 = old_lr_factor(0.5, 50, 10, 0.02)
        self.assertAlmostEqual(S.lr_factor(cc, 0.55, 55, total), 0.02 + (f0 - 0.02) * 0.5, places=12)

    def test_anneal_by_steps(self) -> None:
        from enth_ml.train.loss import LossConfig

        c = LossConfig(t_start=1.0, t_end=0.01, anneal_frac=0.3, anneal_steps=1000)
        self.assertEqual(c.temperature(0.9, 0), 1.0, "by step, whatever the progress")
        self.assertAlmostEqual(c.temperature(0.0, 500), 0.1, places=12)
        self.assertEqual(c.temperature(0.0, 1000), 0.0)
        self.assertEqual(c.temperature(0.0, 5000), 0.0)
        self.assertAlmostEqual(c.temperature(0.15), 0.1, places=12, msg="without a step the share of the run still rules")

    def test_weight_decay_timescale(self) -> None:
        from enth_ml.train import schedule as S

        c = self.cfg(lr=2e-3, batch_size=256, weight_decay=0.05)
        self.assertAlmostEqual(S.weight_decay_tau_samples(c), 2.56e6, places=3)
        t = self.cfg(lr=2e-3, batch_size=256, weight_decay=0.9, weight_decay_tau_samples=2.56e6)
        self.assertAlmostEqual(S.weight_decay(t), 0.05, places=15)
        # A proxy at another batch and LR keeps the same timescale in samples.
        t2 = self.cfg(lr=1e-3, batch_size=512, weight_decay_tau_samples=2.56e6)
        self.assertAlmostEqual(S.weight_decay_tau_samples(t2), 2.56e6, places=3)
        self.assertAlmostEqual(S.weight_decay(t2), 0.2, places=15)

    def test_base_inheritance_and_proxy_v0(self) -> None:
        from enth_ml.train.config import TrainConfig

        proxy = TrainConfig.load(CONFIGS / "proxy-v0.toml")
        full = TrainConfig.load(CONFIGS / "full-v0-b.toml")
        self.assertEqual((proxy.schedule_steps, proxy.warmup_steps, proxy.eval_every_steps), (12500, 500, 1000))
        self.assertEqual((proxy.early_stop_patience, proxy.max_repeat, proxy.train_glob, proxy.min_train_samples, proxy.ckpt_every_minutes), (0, 0.0, "", 0, 10.0))
        self.assertEqual(proxy.persample_evals, "all")
        self.assertEqual(proxy.model, full.model)
        self.assertEqual(proxy.loss, full.loss)
        diff = sorted(k for k, v in proxy.to_dict().items() if full.to_dict()[k] != v)
        self.assertEqual(diff, ["ckpt_every_minutes", "data_target_samples", "early_stop_patience", "eval_every_steps", "min_train_samples", "persample_evals", "schedule_steps", "warmup_steps"])
        with tempfile.TemporaryDirectory() as t:
            d = pathlib.Path(t)
            (d / "a.toml").write_text('lr = 1e-3\nseed = 4\n[loss]\neps = 0.1\nt_end = 0.02\n')
            (d / "sub").mkdir()
            (d / "sub" / "b.toml").write_text('base = "../a.toml"\nseed = 5\n[loss]\neps = 0.2\n')
            c = TrainConfig.load(d / "sub" / "b.toml")
            self.assertEqual((c.lr, c.seed, c.loss.eps, c.loss.t_end), (1e-3, 5, 0.2, 0.02))
            (d / "loop.toml").write_text('base = "loop.toml"\n')
            with self.assertRaises(ValueError):
                TrainConfig.load(d / "loop.toml")


@needs_torch
class Fast(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import numpy as np

        from train_support import write_run

        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = pathlib.Path(cls.tmp.name)
        cls.train_dir = write_run(np, cls.dir / "train", "r3/train", 6, 15, 1)
        cls.dev_dir = write_run(np, cls.dir / "dev", "r3/dev", 2, 16, 2)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def cfg(self, out: str, **kw):
        return tiny_cfg(self.train_dir, self.dev_dir, self.dir / out, **kw)

    def assert_same_weights(self, a: pathlib.Path, b: pathlib.Path) -> None:
        import torch

        wa, wb = final_weights(a), final_weights(b)
        self.assertEqual(wa["step"], wb["step"])
        for k, v in wa["model"].items():
            self.assertTrue(torch.equal(v, wb["model"][k]), f"model tensor {k} differs")
        for pid, st in wa["opt"].items():
            for k, v in st.items():
                self.assertTrue(torch.equal(torch.as_tensor(v), torch.as_tensor(wb["opt"][pid][k])), f"optimiser state {pid}/{k} differs")

    def test_old_schedule_path_trains_bit_for_bit(self) -> None:
        """The default config through the new code against the same loop with the old formulas patched in."""
        from unittest import mock

        from enth_ml.train import loop
        from enth_ml.train import schedule as S
        from enth_ml.train.loss import LossConfig

        loop.train(self.cfg("old-new"), device="cpu")
        with mock.patch.object(S, "lr_factor", lambda cfg, p, step, total: old_lr_factor(p, step, cfg.warmup_steps, cfg.min_lr_frac)), mock.patch.object(
            LossConfig, "temperature", lambda self, p, step=None: old_temperature(self, p)
        ), mock.patch.object(S, "weight_decay", lambda cfg: cfg.weight_decay):
            loop.train(self.cfg("old-ref"), device="cpu")
        self.assert_same_weights(self.dir / "old-new", self.dir / "old-ref")
        tr_new, dv_new = records(self.dir / "old-new")
        tr_ref, dv_ref = records(self.dir / "old-ref")
        self.assertEqual(tr_new, tr_ref)
        self.assertEqual(dv_new, dv_ref)
        run = json.loads((self.dir / "old-new" / "run.json").read_text())
        self.assertEqual(run["schedule"]["lr_schedule"], "cosine")
        self.assertAlmostEqual(run["schedule"]["weight_decay"], 0.05)

    def test_cooldown_branch_equals_a_run_with_the_keys_from_the_start(self) -> None:
        from enth_ml.train import branch
        from enth_ml.train.ckpt import CheckpointStore
        from enth_ml.train.loop import train

        common = dict(epochs=20, schedule_steps=40, lr_schedule="wsd", decay_frac=0.25, eval_every_steps=10)
        common["loss"] = self.cfg("x").loss
        common["loss"].anneal_steps = 8
        parent = self.cfg("wsd-parent", ckpt_keep_every_steps=10, **common)
        s = train(parent, device="cpu")
        self.assertEqual((s["state"], s["steps"]), ("finished", 40))
        keeps = sorted(e["step"] for e in CheckpointStore(self.dir / "wsd-parent").entries("keep"))
        self.assertEqual(keeps, [10, 20, 30, 40])
        tr_parent, _ = records(self.dir / "wsd-parent")
        self.assertEqual(tr_parent[28]["lr"], parent.lr, "the parent is still in its stable phase at step 28")
        # The branch: from the stable phase at step 20, 8 steps of decay.
        bs = branch.cooldown(self.dir / "wsd-parent", 20, 8, self.dir / "wsd-branch")
        self.assertEqual((bs["state"], bs["steps"], bs["reason"]), ("finished", 28, "end (cooldown)"))
        b = json.loads((self.dir / "wsd-branch" / "branch.json").read_text())
        self.assertEqual((b["schema"], b["parent"], b["from_step"], b["end_step"]), ("enth.train-branch/1", "wsd-parent", 20, 28))
        self.assertTrue((self.dir / "wsd-branch" / "run.json").exists())
        self.assertTrue((self.dir / "wsd-branch" / "dev-persample-000000028.npz").exists(), "a branch writes its final per-sample file")
        # The reference: the branch's two keys from the first step, never stopped.
        ref = self.cfg("wsd-ref", cooldown_from=20, cooldown_steps=8, **common)
        rs = train(ref, device="cpu")
        self.assertEqual(rs["steps"], 28)
        self.assert_same_weights(self.dir / "wsd-branch", self.dir / "wsd-ref")
        tr_b, _ = records(self.dir / "wsd-branch")
        tr_r, _ = records(self.dir / "wsd-ref")
        self.assertEqual({k: v for k, v in tr_r.items() if k <= 20}, {k: v for k, v in tr_parent.items() if k <= 20}, "before the branch: the parent")
        self.assertEqual({k: v for k, v in tr_r.items() if k > 20}, tr_b, "after it: the branch")
        lrs = [tr_b[k]["lr"] for k in sorted(tr_b)]
        self.assertTrue(all(x > y for x, y in zip(lrs, lrs[1:])), "the LR decays over the branch")
        # Refusals: no checkpoint at that step, a step inside the parent's own decay, an occupied directory.
        with self.assertRaises(ValueError):
            branch.cooldown(self.dir / "wsd-parent", 25, 5, self.dir / "wsd-b2")
        with self.assertRaises(ValueError):
            branch.cooldown(self.dir / "wsd-parent", 40, 5, self.dir / "wsd-b3")
        with self.assertRaises(FileExistsError):
            branch.cooldown(self.dir / "wsd-parent", 10, 5, self.dir / "wsd-branch")
        with self.assertRaises(ValueError):
            branch.plan_branch(self.dir / "wsd-parent", 20, 8, self.dir / "wsd-b4", ["lr=0.1"])

    def test_persample_files_and_the_final_best(self) -> None:
        import numpy as np

        from enth_ml.train.ckpt import CheckpointStore
        from enth_ml.train.loop import train

        # 37 steps: evaluations at 10, 20, 30 and the final one at 37; the fake keeps learning, so the final one improves.
        cfg = self.cfg("per-all", epochs=20, schedule_steps=37, eval_every_steps=10, eval_at_epoch_end=False, persample_evals="all")
        train(cfg, device="cpu")
        out = self.dir / "per-all"
        files = sorted(p.name for p in out.glob("dev-persample-*.npz"))
        self.assertEqual(files, [f"dev-persample-{s:09d}.npz" for s in (10, 20, 30, 37)])
        devs = {r["step"]: r for r in map(json.loads, (out / "metrics.jsonl").read_text().splitlines()) if r["kind"] == "dev"}
        with np.load(out / "dev-persample-000000037.npz") as z:
            self.assertEqual(int(z["step"]), 37)
            self.assertEqual(len(z["best"]), 32)
            self.assertEqual(z["index"].tolist(), list(range(16)) * 2, "the shards' sample indices, in dev order")
            self.assertEqual(z["source"].tolist(), [i % 3 for i in range(16)] * 2)
            self.assertAlmostEqual(float(z["best"].astype(np.float64).mean()), devs[37]["loss_best"], places=5)
            self.assertTrue(np.all(z["top"] >= z["best"]) and np.all(z["meank"] >= z["best"]))
            self.assertLessEqual(int(z["distinct"].max()), 4)
            self.assertAlmostEqual(float(z["distinct"].mean()), devs[37]["discrete"]["distinct_mean"], places=6)
            meta = json.loads(bytes(z["meta"]).decode())
            self.assertEqual((meta["schema"], meta["run"]), ("enth.dev-persample/1", "per-all"))
        self.assertTrue(devs[37]["final"])
        self.assertTrue(devs[37]["improved"], "the fake keeps learning: the final evaluation is the best")
        self.assertEqual(CheckpointStore(out).best()["step"], 37, "and best.pt is rewritten with it (EVM's end-of-run quirk)")
        # "final": only the last evaluation's file.
        train(self.cfg("per-final", epochs=20, schedule_steps=37, eval_every_steps=10, eval_at_epoch_end=False, persample_evals="final"), device="cpu")
        self.assertEqual([p.name for p in (self.dir / "per-final").glob("dev-persample-*.npz")], ["dev-persample-000000037.npz"])
        # Epoch-end evaluations (eval_at_epoch_end), the run ending on an epoch's last step: the file is still written.
        train(self.cfg("per-epoch", epochs=2, persample_evals="final"), device="cpu")
        self.assertEqual(len(list((self.dir / "per-epoch").glob("dev-persample-*.npz"))), 1)

    def test_eval_command_writes_a_persample_file(self) -> None:
        import numpy as np

        from enth_ml.train.loop import evaluate_checkpoint, train

        train(self.cfg("per-eval", epochs=1), device="cpu")
        res = evaluate_checkpoint(self.dir / "per-eval" / "ckpt" / "best.pt", device="cpu", persample_out=self.dir / "per-eval.npz")
        with np.load(self.dir / "per-eval.npz") as z:
            self.assertAlmostEqual(float(z["best"].astype(np.float64).mean()), res["loss_best"], places=5)

    def test_ram_slice_gives_the_disks_batches(self) -> None:
        import torch

        from enth_ml.train.data import EpochPlan, RamSlice, batches_from, batches_from_ram
        from enth_ml.train.shards import open_dataset

        ds = open_dataset([self.train_dir])
        ram = RamSlice(ds.shards, "f2")
        self.assertEqual(ram.n, 90)
        for epoch, position in ((0, 0), (1, 8), (2, 40)):
            plan = EpochPlan(ds.shards, 3, epoch, 2, 8)
            disk = list(batches_from(plan, position, 0, 2, False, "f2"))
            mem = list(batches_from_ram(plan, position, ram, 2))
            self.assertEqual(len(disk), len(mem))
            for a, b in zip(disk, mem):
                for k in a:
                    self.assertEqual(a[k].dtype, b[k].dtype)
                    self.assertTrue(torch.equal(a[k], b[k]), k)
        with self.assertRaises(MemoryError):
            RamSlice(ds.shards, "f2", max_gb=1e-6)

    def test_train_in_ram_trains_identically(self) -> None:
        from enth_ml.train.loop import train

        train(self.cfg("ram-disk", train_mel_dtype="f2"), device="cpu")
        train(self.cfg("ram-ram", train_mel_dtype="f2", train_in_ram=True), device="cpu")
        self.assert_same_weights(self.dir / "ram-disk", self.dir / "ram-ram")
        recs = [json.loads(line) for line in (self.dir / "ram-ram" / "metrics.jsonl").read_text().splitlines()]
        self.assertEqual(sum(r["kind"] == "ram" for r in recs), 1, "the fixed slice is read once")

    def test_smoke_passes_and_fails(self) -> None:
        from enth_ml.train.smoke import smoke

        toml = self.dir / "tiny-smoke.toml"
        toml.write_text(
            'base = "%s"\nlr = 3e-3\n[model]\nstem = 4\nwidths = [8, 8, 16, 16]\nblocks = [0, 1, 0, 1]\nembed = 32\nhyp_hidden = 16\nconf_hidden = 16\nhypotheses = 4\n'
            % (CONFIGS / "full-v0-b.toml").as_posix()
        )
        res = smoke(str(toml), [str(self.dev_dir)], self.dir / "smoke-ok", device="cpu", samples=32, steps=150, batch=32, log=lambda *_: None)
        self.assertTrue(res["pass"], json.dumps(res["checks"], indent=1))
        saved = json.loads((self.dir / "smoke-ok" / "smoke.json").read_text())
        self.assertEqual(saved["schema"], "enth.train-smoke/1")
        self.assertGreaterEqual(saved["checks"]["overfit"]["drop"], 0.7)
        self.assertEqual(len(saved["checks"]["overfit"]["train_wins_per_head"]), 4)
        self.assertTrue(saved["checks"]["export"]["pass"])
        self.assertIsNone(saved["checks"]["parity"]["pass"], "no --model-check: skipped, not failed")
        self.assertTrue((self.dir / "smoke-ok" / "smoke-model.bin").exists())
        self.assertNotIn(str(pathlib.Path.home()), json.dumps(saved), "no local path in smoke.json")
        # A network that cannot learn (LR 0) fails the overfit check, and the run says so.
        frozen = self.dir / "frozen.toml"
        frozen.write_text(toml.read_text().replace("lr = 3e-3", "lr = 0.0"))
        bad = smoke(str(frozen), [str(self.dev_dir)], self.dir / "smoke-bad", device="cpu", samples=32, steps=10, batch=32, log=lambda *_: None)
        self.assertFalse(bad["pass"])
        self.assertFalse(bad["checks"]["overfit"]["pass"])
        self.assertTrue(bad["checks"]["budgets"]["pass"])


if __name__ == "__main__":
    unittest.main()
