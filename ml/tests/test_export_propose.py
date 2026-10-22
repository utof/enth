"""Lane EXP's exporter: propose v0 to ``ENTHMODL`` (``enth_ml.export.propose``). Needs torch and numpy (``uv sync
--group train``); skipped without them.

What these pin: the committed parity fixture is what the generator writes (the model byte for byte, PyTorch's outputs
within 1e-6); the file holds exactly the weights of the model PyTorch runs (read back and compared); BatchNorm is folded
and rounded once; the header's counts are the model's; and the provenance comes from the run manifest. The Rust side
(``crates/enth-solver/tests/propose_parity.rs``) checks the same fixture against our kernels on every CI leg.
"""

import hashlib
import json
import pathlib
import tempfile
import unittest

from train_support import needs_torch

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "crates/enth-solver/tests/fixtures/propose"


def run_json(layout_sha: str | None = None) -> bytes:
    """A run manifest with the fields the exporter reads (``run.json`` of lane TR's loop, abridged)."""
    from enth_ml.export import enthmodl

    sha = layout_sha or enthmodl.layout_v1_sha256().hex()
    return json.dumps(
        {
            "schema": "enth.train-run/1",
            "config": {"out": "/some/where/pilot-v0", "seed": 1, "model": {"stem": 4, "widths": [4, 4, 6, 8], "blocks": [1, 1, 2, 1], "embed": 16, "hyp_hidden": 4, "conf_hidden": 8, "hypotheses": 8}},
            "environment": {"torch": "2.14.1+cu132", "cuda": "13.2", "commit": "f6d055fde86cb1a4895aaf45e272d2dfefd1103d", "dirty": False, "uv_lock_sha256": "ab" * 32},
            "data": {
                "train": {"ds1": 1, "samples": 100, "runs": [{"stream": "r3/train", "run_seed": 1, "manifest_sha256": "cd" * 32, "engine": "0.3.0", "sources": {"v1": 60, "preset": 25, "uniform": 15}}]},
                "dev": {"ds1": 1, "samples": 10, "runs": [{"stream": "r3/dev", "run_seed": 1, "manifest_sha256": "ef" * 32}]},
            },
            "augmentation": "none (clean ds1 renders only)",
            "layout": {"id": 1, "sha256": sha, "width": 102},
        },
        indent=1,
    ).encode()


@needs_torch
class Exporter(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import torch

        from enth_ml.export import propose

        cls.torch, cls.propose = torch, propose
        cls.model = propose.tiny_model()

    def test_the_committed_fixture_is_what_the_generator_writes(self) -> None:
        from enth_ml.export import tensors

        model, expected = self.propose.fixture()
        self.assertEqual(model, (FIXTURES / "exp_tiny.bin").read_bytes(), "regenerate the fixture")
        fresh = tensors.read_arrays(expected)
        committed = tensors.read_arrays((FIXTURES / "exp_tiny.expected").read_bytes())
        self.assertEqual(sorted(fresh), sorted(committed))
        for name in fresh:
            self.assertEqual(fresh[name].shape, committed[name].shape)
            self.assertLessEqual(float(abs(fresh[name] - committed[name]).max()), 1e-6, name)

    def test_the_fixture_is_small(self) -> None:
        total = sum((FIXTURES / n).stat().st_size for n in ("exp_tiny.bin", "exp_tiny.expected"))
        self.assertLess(total, 45_000, "commit only small fixtures")

    def test_the_model_is_built_from_the_deterministic_generator(self) -> None:
        a, b = self.propose.tiny_model(), self.propose.tiny_model()
        for (n, x), (_, y) in zip(a.state_dict().items(), b.state_dict().items()):
            self.assertTrue(bool((x == y).all()), n)

    def test_folding_alone_does_not_move_the_outputs_and_f16_stays_close(self) -> None:
        torch, propose = self.torch, self.propose
        import numpy as np

        from enth_ml.export import synthetic

        mel = np.array([synthetic.synthetic_input(5)[0]], dtype=np.float32).reshape(1, 3, 128, 64)
        ctx = np.array([synthetic.synthetic_input(5)[1]], dtype=np.float32)
        base = propose.run_reference(self.model, mel, ctx)
        f32 = propose.run_reference(propose.as_stored(self.model, "f32"), mel, ctx)
        f16 = propose.run_reference(propose.as_stored(self.model, "f16"), mel, ctx)
        for a, b in zip(base, f32):
            self.assertLessEqual(float(abs(a - b).max()), 2e-5, "BatchNorm folding in f32")
        for a, b in zip(base, f16):
            self.assertLessEqual(float(abs(a - b).max()), 5e-2, "f16 rounding of the matrices")

    def test_stored_matrices_are_f16_values_and_the_rest_f32(self) -> None:
        torch, propose = self.torch, self.propose
        m = propose.as_stored(self.model, "f16")
        names = {n for n, _ in m.named_parameters()} | {n for n, _ in m.named_buffers()}
        self.assertIn("stem.conv.bias", names)
        self.assertFalse(any("running" in n or n.endswith("bn.weight") for n in names), "BatchNorm is folded away")
        for n, t in list(m.named_parameters()) + list(m.named_buffers()):
            if propose.storage_dtype(n) == "f16":
                self.assertTrue(bool((t == t.half().float()).all()), f"{n} is not an f16 value")
            else:
                self.assertTrue(bool(((t == 0) | (t.abs() >= 2.0**-126)).all()), f"{n} holds a subnormal")

    def test_the_file_holds_the_weights_the_model_runs(self) -> None:
        from enth_ml.export import enthmodl as E
        from enth_ml.model.net import count_flops, exported_parameter_count

        propose = self.propose
        for weights in ("f16", "f32"):
            with self.subTest(weights=weights):
                m = propose.as_stored(self.model, weights)
                w = propose.build_writer(m, "exp-test", weights=weights, engine="0.3.0", ds1=1, manifest_sha256=b"\x07" * 32)
                data = w.build()
                n = propose.check_round_trip(data, m, weights)
                parsed = E.read(data)
                self.assertEqual(n, len(parsed.tensors))
                h = parsed.header
                self.assertEqual((h["parameters"], h["flops"]), (exported_parameter_count(m), count_flops(m)))
                self.assertEqual((h["hypotheses"], h["width"], h["ds1"], h["engine"]), (8, 102, 1, "0.3.0"))
                self.assertEqual(h["manifest_sha256"], b"\x07" * 32)
                self.assertEqual(h["layout_sha256"], E.layout_v1_sha256())
                self.assertEqual([o[0] for o in parsed.outputs], ["hypotheses", "confidence"])
                self.assertEqual([o[2] for o in parsed.outputs], [(8, 102), (8,)])
                dtypes = {name: dtype for name, dtype, *_ in parsed.tensors}
                self.assertEqual(dtypes["hyp_w"], E.F16 if weights == "f16" else E.F32)
                self.assertEqual(dtypes["hyp_b"], E.F32)
                self.assertEqual(dtypes["mel_scale"], E.F32)

    def test_a_changed_weight_is_caught_by_the_read_back(self) -> None:
        propose = self.propose
        m = propose.as_stored(self.model, "f16")
        data = propose.build_writer(m, "exp-test").build()
        with self.torch.no_grad():
            m.proj.weight[0, 0] += 0.5
        with self.assertRaises(ValueError):
            propose.check_round_trip(data, m)

    def test_the_graph_uses_op_set_1_only_and_the_node_counts_follow_the_network(self) -> None:
        from enth_ml.export import enthmodl as E

        m = self.propose.as_stored(self.model, "f16")
        parsed = E.read(self.propose.build_writer(m, "exp-test").build())
        ops = [op for op, _, _ in parsed.nodes]
        self.assertTrue(set(ops) <= set(E.OPS))
        blocks = sum(self.model.cfg.blocks)
        stages = len(self.model.cfg.widths)
        self.assertEqual(ops.count("Conv2d"), 1 + stages + 2 * blocks)
        self.assertEqual(ops.count("AvgPool2d"), 1)
        self.assertEqual(ops.count("MatMul"), 1)

    def test_provenance_comes_from_the_run_manifest(self) -> None:
        propose = self.propose
        raw = run_json()
        state = {"epoch": 39, "step": 28995, "best_dev": 1.7736472788924875}
        ds1, engine, manifest, info = propose.provenance(state, raw)
        self.assertEqual((ds1, engine), (1, "0.3.0"))
        self.assertEqual(manifest, hashlib.sha256(raw).digest())
        d = dict(info)
        self.assertEqual(d["train.run"], "pilot-v0")
        self.assertEqual(d["train.checkpoint"], "best, epoch 40, step 28995")
        self.assertEqual(d["train.commit"], "f6d055fde86cb1a4895aaf45e272d2dfefd1103d")
        self.assertIn("r3/train seed 1, 100 samples", d["data.train"])
        self.assertTrue(all(len(v) <= 1024 and v.isascii() and v.isprintable() for v in d.values()))
        self.assertEqual(len(d), len(info), "keys are unique")
        with self.assertRaises(ValueError):
            propose.provenance(state, run_json(layout_sha="00" * 32))

    def test_no_personal_detail_reaches_the_file(self) -> None:
        propose = self.propose
        _, _, _, info = propose.provenance({"epoch": 0, "step": 1, "best_dev": 1.0}, run_json())
        text = " ".join(f"{k} {v}" for k, v in info)
        for bad in ("@", "/home", "/mnt", "C:\\", "some/where"):
            self.assertNotIn(bad, text)

    def test_dev_inputs_are_picked_evenly_and_in_parts(self) -> None:
        import numpy as np

        from train_support import write_run

        propose = self.propose
        with tempfile.TemporaryDirectory() as d:
            run = write_run(np, pathlib.Path(d) / "dev", "r3/dev", 3, 10, 5)
            whole = propose.pick_dev_inputs([str(run)], 0)
            self.assertEqual(whole["mel"].shape, (30, 3, 128, 64))
            self.assertEqual(whole["index"].tolist(), list(range(30)))
            some = propose.pick_dev_inputs([str(run)], 7)
            self.assertEqual(some["index"].tolist(), [0, 5, 10, 14, 19, 24, 29])
            self.assertTrue(bool((some["mel"][2] == whole["mel"][10]).all()))
            parts = [propose.pick_dev_inputs([str(run)], 7, (i, 3)) for i in range(3)]
            self.assertEqual(sum((p["index"].tolist() for p in parts), []), some["index"].tolist())

    def test_the_tensor_file_of_arrays_matches_the_list_writer(self) -> None:
        import numpy as np

        from enth_ml.export import tensors

        a = np.arange(24, dtype=np.float32).reshape(2, 3, 4)
        data = tensors.write_arrays([("a", a), ("b", np.array([1.5], dtype=np.float64))])
        self.assertEqual(data, tensors.write([("a", (2, 3, 4), a.reshape(-1).tolist()), ("b", (1,), [1.5])]))
        back = tensors.read_arrays(data)
        self.assertTrue(bool((back["a"] == a).all()))
        self.assertEqual(back["b"].shape, (1,))


if __name__ == "__main__":
    unittest.main()
