"""Lane TR: propose v0's shapes, budgets and op set (crates/enth-solver/src/propose/pack.rs, op set 1)."""

from __future__ import annotations

import unittest

from train_support import needs_torch


@needs_torch
class Model(unittest.TestCase):
    def setUp(self) -> None:
        import torch

        from enth_ml.model.net import ModelConfig, ProposeV0

        torch.manual_seed(0)
        self.torch = torch
        self.model = ProposeV0(ModelConfig())

    def test_shapes_and_ranges(self) -> None:
        from enth_ml.model import layout as L

        t = self.torch
        mel = t.randn(3, 3, 128, 64) * 2.5 - 9.0
        ctx = t.randn(3, 5)
        hyp, conf = self.model.eval()(mel, ctx)
        self.assertEqual(tuple(hyp.shape), (3, 8, 102))
        self.assertEqual(tuple(conf.shape), (3, 8))
        for a, b in L.SIGMOID_RANGES:
            self.assertTrue(bool(((hyp[..., a:b] >= 0) & (hyp[..., a:b] <= 1)).all()))
        self.assertTrue(bool(t.isfinite(hyp).all()) and bool(t.isfinite(conf).all()))
        e = self.model.embed(mel, ctx)
        self.assertEqual(tuple(e.shape), (3, self.model.cfg.embed))
        # The K heads start apart (their biases differ), so WTA has something to choose between.
        self.assertGreater(float((hyp[:, 0] - hyp[:, 1]).abs().mean()), 1e-3)

    def test_budgets(self) -> None:
        from enth_ml.model.net import count_flops, exported_parameter_count

        params = exported_parameter_count(self.model)
        flops = count_flops(self.model)
        self.assertLessEqual(params, 2**22, "pack.rs: MAX_PARAMETERS")
        self.assertLessEqual(flops, 2**31, "pack.rs: MAX_FLOPS")
        self.assertGreater(params, 1_000_000)
        self.assertLess(abs(flops / 0.5e9 - 1.0), 0.25, f"propose v0 aims at about 0.5 GFLOP, has {flops / 1e9:.3f}")

    def test_op_audit(self) -> None:
        from torch import nn

        from enth_ml.model.net import audit_ops

        self.assertEqual(audit_ops(self.model), [])
        bad = self.model
        bad.extra = nn.GroupNorm(2, 4)
        bad.pool = nn.AvgPool2d(3, padding=1)  # counts padding by default: op set 1 does not
        problems = audit_ops(bad)
        self.assertTrue(any("GroupNorm" in p for p in problems), problems)
        self.assertTrue(any("counts padding" in p for p in problems), problems)

    def test_functions_called_are_op_set_1(self) -> None:
        """Record every torch function the folded model calls and hold them to op set 1's meanings."""
        from torch.overrides import TorchFunctionMode

        from enth_ml.model.fold import fold_batchnorm

        seen: set[str] = set()

        class Record(TorchFunctionMode):
            def __torch_function__(self, func, types, args=(), kwargs=None):
                seen.add(getattr(func, "__name__", str(func)))
                return func(*args, **(kwargs or {}))

        m = fold_batchnorm(self.model)
        mel, ctx = self.torch.randn(2, 3, 128, 64), self.torch.randn(2, 5)
        with Record():
            m(mel, ctx)
        allowed = {
            # Mul, Add (standardisation, residuals), Conv2d, Silu, AvgPool2d, Reshape, Linear, Concat, MatMul, Slice, Sigmoid
            "__mul__", "mul", "__add__", "add", "conv2d", "silu", "avg_pool2d", "reshape", "linear", "cat", "matmul",
            "__getitem__", "sigmoid",
            # bookkeeping that computes nothing
            "to", "float", "dim", "size", "__get__", "device", "shape", "type", "is_floating_point",
        }  # fmt: skip
        self.assertEqual(seen - allowed, set(), f"calls outside op set 1: {sorted(seen - allowed)}")
        self.assertNotIn("batch_norm", seen, "BatchNorm is folded")

    def test_fold_batchnorm_matches(self) -> None:
        from enth_ml.model.fold import fold_batchnorm

        t = self.torch
        m = self.model
        # Give the BatchNorms non-trivial running statistics and affine values.
        with t.no_grad():
            for mod in m.modules():
                if isinstance(mod, t.nn.BatchNorm2d):
                    mod.running_mean.uniform_(-0.5, 0.5)
                    mod.running_var.uniform_(0.5, 2.0)
                    mod.weight.uniform_(0.5, 1.5)
                    mod.bias.uniform_(-0.2, 0.2)
        m.eval()
        mel = t.randn(2, 3, 128, 64) * 2.5 - 9.0
        ctx = t.randn(2, 5)
        f = fold_batchnorm(m)
        with t.no_grad():
            a = m(mel, ctx)
            b = f(mel, ctx)
        for x, y in zip(a, b):
            self.assertLess(float((x - y).abs().max()), 1e-4)
        self.assertFalse(any(isinstance(mod, t.nn.BatchNorm2d) for mod in f.modules()))

    def test_standardisation_is_in_the_graph(self) -> None:
        t = self.torch
        mean = t.full((3, 64), -9.0)
        std = t.full((3, 64), 2.0)
        cm = t.tensor([0.0, 0.0, 0.5, 1.0, 0.0])
        cs = t.tensor([20.0, 0.3, 0.5, 1.0, 0.2])
        self.model.set_standardisation(mean, std, cm, cs)
        x = t.full((1, 3, 128, 64), -9.0)
        self.assertEqual(float((x * self.model.mel_scale + self.model.mel_shift).abs().max()), 0.0)
        c = t.tensor([[20.0, 0.3, 1.0, 2.0, 0.2]])
        self.assertTrue(t.allclose(c * self.model.ctx_scale + self.model.ctx_shift, t.ones(1, 5)))


if __name__ == "__main__":
    unittest.main()
