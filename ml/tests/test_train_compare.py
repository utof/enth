"""Lane FTL: ``python -m enth_ml.train compare`` on synthetic per-sample arrays with a known answer (numpy only).

The statistics are checked against hand values (Wilcoxon with and without ties, Rice's jitter) and against numpy's
own formulas (the paired SE, the shares); the verdict against built-in gaps: a clear gap, no gap, the same gap under a
larger seed SD, replicates whose seeds agree and disagree; the pairing by sample index survives a shuffled file and
refuses another dev set; a run directory's discrete metrics come with their jitter bands.
"""

from __future__ import annotations

import contextlib
import io
import json
import math
import pathlib
import tempfile
import unittest

from train_support import needs_numpy

N = 9324


def _file(path: pathlib.Path, step: int, best, top=None, meank=None, index=None, source=None, distinct=None) -> pathlib.Path:
    import numpy as np

    n = len(best)
    arrays = {
        "best": np.asarray(best, dtype=np.float32),
        "top": np.asarray(top if top is not None else best, dtype=np.float32),
        "meank": np.asarray(meank if meank is not None else best, dtype=np.float32),
        "winner": np.zeros(n, dtype=np.uint8),
        "index": np.asarray(index if index is not None else np.arange(n), dtype=np.uint64),
        "source": np.asarray(source if source is not None else np.arange(n) % 3, dtype=np.uint8),
    }
    if distinct is not None:
        arrays["distinct"] = np.asarray(distinct, dtype=np.uint8)
    meta = np.frombuffer(json.dumps({"schema": "enth.dev-persample/1", "step": step}).encode(), dtype=np.uint8)
    np.savez(path, step=np.int64(step), meta=meta, **arrays)
    return path


@needs_numpy
class Compare(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import numpy as np

        cls.tmp = tempfile.TemporaryDirectory()
        cls.dir = pathlib.Path(cls.tmp.name)
        rng = np.random.default_rng(7)
        cls.base = rng.lognormal(0.2, 0.5, N)  # heavy-tailed per-sample losses, like dev best-of-8
        cls.noise_a = rng.normal(0, 0.3, N)
        cls.noise_b = rng.normal(0, 0.3, N)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def test_wilcoxon_hand_values(self) -> None:
        from enth_ml.train.compare import wilcoxon

        w = wilcoxon([1.0, 2.0, 3.0, -4.0, 5.0])  # ranks 1..5; W+ = 1 + 2 + 3 + 5
        self.assertEqual(w["w_plus"], 11.0)
        self.assertAlmostEqual(w["z"], (11 - 7.5) / math.sqrt(13.75), places=12)
        self.assertAlmostEqual(w["p"], math.erfc(w["z"] / math.sqrt(2)), places=12)
        # Ties: |d| = 1, 1, 2, 2, 3 -> ranks 1.5, 1.5, 3.5, 3.5, 5; zeros dropped; var 13.75 - (6 + 6) / 48.
        w = wilcoxon([1.0, -1.0, 2.0, 2.0, -3.0, 0.0])
        self.assertEqual((w["n"], w["w_plus"]), (5, 8.5))
        self.assertAlmostEqual(w["z"], 1.0 / math.sqrt(13.5), places=12)
        self.assertEqual(wilcoxon([0.0, 0.0])["p"], 1.0)

    def test_rice_jitter(self) -> None:
        import numpy as np

        from enth_ml.train.compare import rice_jitter

        x = [1.0, 0.9, 0.85, 0.83, 0.8, 0.79]
        want = float(np.diff(np.array(x), 2).std(ddof=1) / math.sqrt(6))
        self.assertAlmostEqual(rice_jitter(x), want, places=15)
        self.assertAlmostEqual(rice_jitter([0.1 * i for i in range(10)]), 0.0, places=12)  # a straight line: no jitter
        self.assertIsNone(rice_jitter([1.0, 2.0, 3.0]))

    def test_a_known_gap_is_found_and_b_wins(self) -> None:
        import numpy as np

        from enth_ml.train.compare import compare, format_report

        a = self.base + 0.03 + self.noise_a
        b = self.base
        fa = _file(self.dir / "gap-a.npz", 1000, a, distinct=np.full(N, 6))
        fb = _file(self.dir / "gap-b.npz", 1000, b, distinct=np.full(N, 5))
        rep = compare([str(fa)], [str(fb)], n_boot=1000)
        r = rep["losses"]["best"]
        d = a.astype(np.float32).astype(np.float64) - b.astype(np.float32).astype(np.float64)
        self.assertAlmostEqual(r["delta"], float(d.mean()), places=12)
        self.assertAlmostEqual(r["se"], float(d.std(ddof=1) / math.sqrt(N)), places=12)
        self.assertAlmostEqual(r["share_b_lower"], float((d > 0).mean()), places=12)
        self.assertLess(r["ci95"][0], r["delta"])
        self.assertGreater(r["ci95"][1], r["delta"])
        self.assertGreater(r["wilcoxon"]["z"], 3)
        self.assertAlmostEqual(r["delta"], 0.03, delta=4 * r["se"])
        self.assertEqual(rep["verdict"]["verdict"], "B better")
        self.assertEqual(rep["verdict"]["rule"], "3 x paired SE")
        self.assertIsNotNone(rep["verdict"]["note"], "seed noise is not included and the report says so")
        self.assertAlmostEqual(rep["distinct_paired"]["delta"], 1.0, places=12)
        # The other way round: the same gap, negated, and A wins.
        rev = compare([str(fb)], [str(fa)], n_boot=200)
        self.assertAlmostEqual(rev["losses"]["best"]["delta"], -r["delta"], places=12)
        self.assertEqual(rev["verdict"]["verdict"], "A better")
        # Per source: the breakdown's means are the subsets' means.
        src = np.arange(N) % 3
        for name, s in (("generator v1", 0), ("preset", 1), ("uniform", 2)):
            self.assertAlmostEqual(rep["by_source"][name]["delta"], float(d[src == s].mean()), places=10)
        self.assertIn("VERDICT: B better", format_report(rep))

    def test_no_gap_is_no_decision_and_seed_noise_raises_the_bar(self) -> None:
        from enth_ml.train.compare import compare

        fa = _file(self.dir / "null-a.npz", 500, self.base + self.noise_a)
        fb = _file(self.dir / "null-b.npz", 500, self.base + self.noise_b)
        rep = compare([str(fa)], [str(fb)], n_boot=500)
        self.assertEqual(rep["verdict"]["verdict"], "no decision at this tier")
        # A real gap of 0.03 (about 7 paired SE here) is no decision once one run's seed SD is 0.02: 2 x 0.02 > 0.03.
        fa2 = _file(self.dir / "gap2-a.npz", 500, self.base + 0.03 + self.noise_a)
        fb2 = _file(self.dir / "gap2-b.npz", 500, self.base)
        rep = compare([str(fa2)], [str(fb2)], seed_sd=0.02, n_boot=200)
        self.assertEqual((rep["verdict"]["verdict"], rep["verdict"]["rule"]), ("no decision at this tier", "2 x seed SD"))
        self.assertAlmostEqual(rep["verdict"]["threshold"], 0.04, places=12)
        self.assertIsNone(rep["verdict"]["note"])

    def test_replicates_measure_the_seed_noise(self) -> None:
        import numpy as np

        from enth_ml.train.compare import compare

        rng = np.random.default_rng(3)
        files = {}
        for arm, gap in (("a", 0.03), ("b", 0.0)):
            for s, offset in ((1, 0.002), (2, -0.002)):
                files[arm, s] = _file(self.dir / f"rep-{arm}{s}.npz", 1000, self.base + gap + offset + rng.normal(0, 0.1, N))
        rep = compare([str(files["a", 1]), str(files["a", 2])], [str(files["b", 1]), str(files["b", 2])], n_boot=300)
        self.assertEqual(rep["replicates"], 2)
        means_a = [float(np.load(files["a", s])["best"].mean()) for s in (1, 2)]
        means_b = [float(np.load(files["b", s])["best"].mean()) for s in (1, 2)]
        want_sd = math.sqrt((np.var(means_a, ddof=1) + np.var(means_b, ddof=1)) / 2)
        self.assertAlmostEqual(rep["seeds"]["seed_sd_measured"], want_sd, places=10)
        self.assertEqual(len(rep["seeds"]["delta_per_seed"]), 2)
        self.assertTrue(all(x > 0 for x in rep["seeds"]["delta_per_seed"]))
        self.assertEqual(rep["verdict"]["verdict"], "B better")
        self.assertEqual(rep["verdict"]["seed_sd_source"], "measured from the replicates")
        # Seeds that disagree on the sign: no decision, whatever the mean says.
        fa = _file(self.dir / "dis-a2.npz", 1000, self.base - 0.02)
        rep = compare([str(files["a", 1]), str(fa)], [str(files["b", 1]), str(files["b", 2])], n_boot=200)
        self.assertEqual(rep["verdict"]["verdict"], "no decision at this tier")
        self.assertIn("disagree", rep["verdict"]["rule"])

    def test_pairing_by_index(self) -> None:
        import numpy as np

        from enth_ml.train.compare import compare

        a = self.base + 0.03 + self.noise_a
        fa = _file(self.dir / "idx-a.npz", 10, a)
        perm = np.random.default_rng(1).permutation(N)
        fb = _file(self.dir / "idx-b.npz", 10, self.base[perm], index=np.arange(N)[perm], source=(np.arange(N) % 3)[perm])
        fb_sorted = _file(self.dir / "idx-b2.npz", 10, self.base)
        r1 = compare([str(fa)], [str(fb)], n_boot=100)["losses"]["best"]
        r2 = compare([str(fa)], [str(fb_sorted)], n_boot=100)["losses"]["best"]
        self.assertEqual((r1["delta"], r1["se"]), (r2["delta"], r2["se"]))
        other = _file(self.dir / "idx-c.npz", 10, self.base, index=np.arange(N) + 1)
        with self.assertRaises(ValueError):
            compare([str(fa)], [str(other)], n_boot=10)

    def test_run_directories_steps_and_discrete_bands(self) -> None:
        import numpy as np

        from enth_ml.train.compare import compare, main, rice_jitter

        curves = {"a": [6.0, 6.5, 6.4, 6.62, 6.6, 6.66], "b": [5.0, 4.9, 5.05, 4.98, 5.02, 5.0]}
        runs = {}
        for arm in ("a", "b"):
            run = self.dir / f"run-{arm}"
            run.mkdir()
            lines = []
            for i, v in enumerate(curves[arm]):
                step = 1000 * (i + 1)
                rec = {"kind": "dev", "step": step, "loss_best": 1.0, "conf_top1": 0.2, "discrete": {"distinct_mean": v, "categorical_oracle_mean": 0.9}, "best": {"presence_recall": 0.8}}
                lines.append(json.dumps(rec))
            (run / "metrics.jsonl").write_text("\n".join(lines) + "\n")
            off = 0.03 if arm == "a" else 0.0
            _file(run / "dev-persample-000003000.npz", 3000, self.base + off + 0.05)
            _file(run / "dev-persample-000006000.npz", 6000, self.base + off)
            runs[arm] = run
        rep = compare([str(runs["a"])], [str(runs["b"])], n_boot=100)
        self.assertEqual((rep["a"][0]["step"], rep["b"][0]["step"]), (6000, 6000), "the newest file by default")
        dm = rep["discrete"]["distinct joint choices of K"]
        self.assertAlmostEqual(dm["delta"], 6.66 - 5.0, places=12)
        ja, jb = rice_jitter(curves["a"]), rice_jitter(curves["b"])  # 6 evaluations: the last 5 (at least 5)
        ja5, jb5 = rice_jitter(curves["a"][-5:]), rice_jitter(curves["b"][-5:])
        self.assertAlmostEqual(dm["band"], 2 * math.sqrt(ja5**2 + jb5**2), places=12)
        self.assertNotEqual(ja, ja5)
        self.assertTrue(dm["beyond_band"])
        self.assertFalse(rep["discrete"]["categorical oracle (mean)"]["beyond_band"])
        early = compare([str(runs["a"])], [str(runs["b"])], step=3000, n_boot=100)
        self.assertEqual(early["a"][0]["step"], 3000)
        with self.assertRaises(FileNotFoundError):
            compare([str(runs["a"])], [str(runs["b"])], step=4000)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(main([str(runs["a"]), str(runs["b"]), "--json", "--boot", "50"]), 0)
        self.assertEqual(json.loads(buf.getvalue())["verdict"]["verdict"], "B better")
        self.assertTrue(np.isfinite(rep["losses"]["top"]["delta"]))


if __name__ == "__main__":
    unittest.main()
