"""Lane ONX's ONNX Runtime parity fixtures and the ONNX converter (``enth_ml.export.parity``, ``onnx_import``).

Needs the ``parity`` group (``uv run --group parity python -m unittest discover -s tests``); skipped without it, as
the standard-library run of the suite has none. The fixtures the Rust test compiles in must be what this checkout's
generator writes: the models byte for byte (their weights come from the deterministic generator), ONNX Runtime's
outputs within 1e-6 (they may move in the last bits with its version or the CPU).
"""

import importlib.util
import pathlib
import unittest

HAVE = all(importlib.util.find_spec(m) for m in ("numpy", "onnx", "onnxruntime"))
FIXTURES = pathlib.Path(__file__).resolve().parents[2] / "crates/enth-solver/tests/fixtures/propose"


@unittest.skipUnless(HAVE, "needs the parity group: uv run --group parity")
class Parity(unittest.TestCase):
    def test_the_committed_fixtures_are_what_the_generator_writes(self) -> None:
        from enth_ml.export import parity, tensors

        for name in parity.FIXTURES:
            with self.subTest(fixture=name):
                model, expected = parity.fixture(name)
                self.assertEqual(model, (FIXTURES / f"{name}.bin").read_bytes(), "regenerate the fixtures")
                committed = tensors.read((FIXTURES / f"{name}.expected").read_bytes())
                fresh = tensors.read(expected)
                self.assertEqual([(n, d) for n, d, _ in fresh], [(n, d) for n, d, _ in committed])
                for (n, _, a), (_, _, b) in zip(fresh, committed):
                    worst = max(abs(x - y) for x, y in zip(a, b))
                    self.assertLessEqual(worst, 1e-6, f"{name}.{n}")

    def test_the_fixtures_are_small(self) -> None:
        total = sum(p.stat().st_size for p in FIXTURES.iterdir())
        self.assertLess(total, 80_000, "commit only small fixtures")

    def test_what_op_set_1_cannot_express_is_refused(self) -> None:
        import numpy as np
        from onnx import TensorProto, helper, numpy_helper

        from enth_ml.export.onnx_import import Unsupported, convert

        def model(node, inits=()):
            ins = [
                helper.make_tensor_value_info("mel", TensorProto.FLOAT, [1, 3, 128, 64]),
                helper.make_tensor_value_info("context", TensorProto.FLOAT, [1, 5]),
            ]
            out = [helper.make_tensor_value_info("y", TensorProto.FLOAT, None)]
            g = helper.make_graph([node], "t", ins, out, list(inits))
            m = helper.make_model(g, opset_imports=[helper.make_opsetid("", 20)])
            m.ir_version = 10
            return m

        cases = {
            "an op outside the set": helper.make_node("Exp", ["context"], ["y"]),
            "GELU's tanh form": helper.make_node("Gelu", ["context"], ["y"], approximate="tanh"),
            "pooling that counts padding": helper.make_node(
                "AveragePool", ["mel"], ["y"], kernel_shape=[3, 3], pads=[1, 1, 1, 1], count_include_pad=1
            ),
            "ceil mode": helper.make_node("MaxPool", ["mel"], ["y"], kernel_shape=[2, 2], ceil_mode=1),
            "a softmax over the batch axis": helper.make_node("Softmax", ["context"], ["y"], axis=0),
            "a transpose that moves the batch axis": helper.make_node("Transpose", ["context"], ["y"], perm=[1, 0]),
        }
        for why, node in cases.items():
            with self.subTest(case=why), self.assertRaises(Unsupported):
                convert(model(node), "t", 1)
        w = numpy_helper.from_array(np.ones((5, 2), dtype=np.float32), "w")
        with self.assertRaises(Unsupported):
            convert(model(helper.make_node("Gemm", ["context", "w"], ["y"], alpha=2.0), [w]), "t", 1)


if __name__ == "__main__":
    unittest.main()
