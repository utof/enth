"""Lane TR: reading lane DG's ds1 shards and run manifests (numpy only)."""

from __future__ import annotations

import json
import pathlib
import struct
import tempfile
import unittest

from train_support import needs_numpy


@needs_numpy
class Shards(unittest.TestCase):
    def setUp(self) -> None:
        import numpy as np

        self.np = np
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_layout_names_and_swap(self) -> None:
        from enth_ml.model import layout as L

        names = L.expected_entry_names()
        self.assertEqual(len(names), 102)
        self.assertEqual(names[31], "lfo1.rate")
        self.assertEqual(names[66], "osc2.enabled")
        self.assertEqual(names[70], "route env2->osc1.fine present")
        self.assertEqual(names[101], "route velocity->filter1.cutoff depth")
        perm = L.swap_permutation()
        self.assertEqual([perm[perm[i]] for i in range(102)], list(range(102)), "the swap is an involution")
        for a, b in [(31, 32), (52, 59), (58, 65), (74, 78), (77, 81), (90, 94), (93, 97)]:
            self.assertEqual((perm[a], perm[b]), (b, a))
            self.assertEqual(names[a].replace("lfo1", "lfoX"), names[b].replace("lfo2", "lfoX"))
        moved = [i for i in range(102) if perm[i] != i]
        self.assertEqual(len(moved), 2 * (1 + 7 + 4 + 4))

    def test_round_trip_and_offsets(self) -> None:
        from enth_ml.train import shards as S
        from train_support import fake_samples

        a = fake_samples(self.np, 5, 3)
        p = self.dir / "s.bin"
        size = S.write_shard(p, a, {"ds1": 1, "stream": "r3/train", "run_seed": 1, "shard": 0, "first_index": 0, "candidates": 5})
        raw = p.read_bytes()
        self.assertEqual(raw[:8], b"ENTHSHRD")
        self.assertEqual(struct.unpack_from("<I", raw, 8)[0], 1)
        h = S.read_header(p)
        # DG's layout: arrays from byte 1024, each at a multiple of 64, in the listed order, the file ending at the last.
        offs = [x["offset"] for x in h["arrays"]]
        self.assertEqual(offs[0], 1024)
        self.assertTrue(all(o % 64 == 0 for o in offs))
        self.assertEqual(size, h["arrays"][-1]["offset"] + h["arrays"][-1]["bytes"])
        self.assertEqual(h["arrays"][0]["bytes"], 5 * 3 * 128 * 64 * 4)
        got = S.read_shard(p)
        for k in a:
            self.np.testing.assert_array_equal(got[k], a[k])
        half = S.read_shard(p, ("mel",), mel_dtype="f2")
        self.assertEqual(half["mel"].dtype, self.np.float16)
        self.assertEqual(set(half), {"mel"})

    def test_refusals(self) -> None:
        from enth_ml.train import shards as S
        from train_support import fake_samples

        p = self.dir / "s.bin"
        S.write_shard(p, fake_samples(self.np, 2, 1))
        raw = bytearray(p.read_bytes())
        bad = self.dir / "bad.bin"
        bad.write_bytes(b"NOTSHARD" + bytes(raw[8:]))
        with self.assertRaises(S.ShardError):
            S.read_header(bad)
        raw2 = bytearray(raw)
        raw2[8:12] = struct.pack("<I", 2)
        bad.write_bytes(bytes(raw2))
        with self.assertRaises(S.ShardError):
            S.read_header(bad)
        bad.write_bytes(bytes(raw[:2000]))
        with self.assertRaises(S.ShardError):
            S.read_shard(bad)

    def test_run_manifest_checks(self) -> None:
        from enth_ml.model.layout import LayoutMismatch
        from enth_ml.train import shards as S
        from train_support import write_run

        good = write_run(self.np, self.dir / "train", "r3/train", 2, 3, 1)
        run = S.load_run(good)
        self.assertEqual(run.kept, 6)
        self.assertEqual(len(run.shards), 2)
        bad = write_run(self.np, self.dir / "other", "r3/train", 1, 2, 2, layout_sha="0" * 64)
        with self.assertRaises(LayoutMismatch):
            S.load_run(bad)
        m = json.loads((good / "manifest.json").read_text())
        m["layout"]["entries"][31], m["layout"]["entries"][32] = m["layout"]["entries"][32], m["layout"]["entries"][31]
        moved = self.dir / "moved"
        moved.mkdir()
        (moved / "manifest.json").write_text(json.dumps(m))
        with self.assertRaises(LayoutMismatch):
            S.load_run(moved)

    def test_f16_cache_reads_like_the_run(self) -> None:
        from enth_ml.train import shards as S
        from enth_ml.train.cache import make_cache
        from train_support import write_run

        src = write_run(self.np, self.dir / "src", "r3/train", 3, 4, 1)
        r = make_cache(src, self.dir / "cache", workers=2)
        self.assertEqual(r["converted"], 3)
        a = S.open_dataset([src])
        b = S.open_dataset([self.dir / "cache"])
        self.assertEqual(a.n, b.n)
        self.assertEqual(b.runs[0].manifest["tr_cache"]["source_manifest_sha256"], a.runs[0].manifest_sha256)
        x = S.read_shard(a.shards[1][0])
        y = S.read_shard(b.shards[1][0], mel_dtype="f2")
        self.assertEqual(y["mel"].dtype, self.np.float16)
        self.np.testing.assert_array_equal(y["mel"], x["mel"].astype(self.np.float16))
        self.np.testing.assert_array_equal(y["label"], x["label"])
        self.assertEqual(S.read_shard(b.shards[0][0], mel_dtype="f4")["mel"].dtype, self.np.float32)
        self.assertEqual(make_cache(src, self.dir / "cache", workers=2)["converted"], 0, "a rerun skips what is there")

    def test_streams_must_differ(self) -> None:
        from enth_ml.train import shards as S
        from train_support import write_run

        t = S.open_dataset([write_run(self.np, self.dir / "t", "r3/train", 1, 2, 1)])
        d = S.open_dataset([write_run(self.np, self.dir / "d", "r3/dev", 1, 2, 1)])
        t2 = S.open_dataset([write_run(self.np, self.dir / "t2", "r3/train", 1, 2, 9)])
        self.assertTrue(S.streams_disjoint(t, d))
        self.assertFalse(S.streams_disjoint(t, t2))
        both = S.open_dataset([self.dir / "t", self.dir / "t2"])
        self.assertEqual(both.n, 4)
        self.assertEqual(both.describe()["samples"], 4)


if __name__ == "__main__":
    unittest.main()
