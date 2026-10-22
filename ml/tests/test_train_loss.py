"""Lane TR: the masked JND-unit loss, the LFO swap, winner-takes-all and the confidence target."""

from __future__ import annotations

import unittest

from train_support import needs_torch


@needs_torch
class Loss(unittest.TestCase):
    def setUp(self) -> None:
        import numpy as np
        import torch

        from enth_ml.train.loss import LossConfig, ProposeLoss
        from train_support import fake_jnd, fake_samples

        self.torch = torch
        scale, table = fake_jnd(np)
        self.table = table
        self.loss = ProposeLoss(table, scale, LossConfig())
        a = fake_samples(np, 4, 5)
        self.label = torch.as_tensor(a["label"])
        self.mask = torch.as_tensor(a["mask"])

    def perfect(self, k: int = 8):
        """Hypotheses equal to the label, with confident logits for the discrete entries."""
        from enth_ml.model import layout as L

        t = self.torch
        h = self.label.clone()
        a, b = L.CATEGORICAL[0], L.PRESENCE[1]
        h[:, a:b] = (self.label[:, a:b] * 2 - 1) * 30.0
        return h.unsqueeze(1).repeat(1, k, 1)

    def test_jnd_interpolation(self) -> None:
        t = self.torch
        x = t.tensor([[0.0] * 33, [1.0] * 33, [0.5] * 33, [1.0 / 32] * 33])
        j = self.loss.jnd_at(x)
        self.assertAlmostEqual(float(j[0, 0]), float(self.table[0, 0]), places=6)
        self.assertAlmostEqual(float(j[1, 0]), float(self.table[0, 16]), places=6)
        self.assertAlmostEqual(float(j[2, 5]), float(self.table[5, 8]), places=6)
        self.assertAlmostEqual(float(j[3, 0]), float((self.table[0, 0] + self.table[0, 1]) / 2), places=6)

    def test_a_perfect_hypothesis_wins(self) -> None:
        t = self.torch
        h = self.perfect()
        h[:, :3, :33] = (h[:, :3, :33] + 0.2).clamp(0, 1)  # three heads off by about 10 JND
        h[:, 4:, :33] = (h[:, 4:, :33] - 0.1).clamp(0, 1)
        losses, swapped = self.loss.hypothesis_losses(h, self.label, self.mask)
        self.assertTrue(bool((losses.argmin(1) == 3).all()))
        self.assertLess(float(losses[:, 3].max()), 1e-6)
        r = self.loss(h, t.zeros(4, 8), self.label, self.mask)
        self.assertTrue(bool((r["winner"] == 3).all()))
        conf = t.zeros(4, 8)
        conf[:, 3] = 10.0
        r2 = self.loss(h, conf, self.label, self.mask)
        self.assertLess(float(r2["conf"]), float(r["conf"]), "the confidence target is the winner")

    def test_lfo_swap_is_free(self) -> None:
        from enth_ml.model import layout as L

        t = self.torch
        h = self.perfect(2)
        perm = t.tensor(L.swap_permutation())
        lab = self.label.clone()
        lab[:, 31] = 0.2
        lab[:, 32] = 0.8
        lab[:, 52:66] = 0.0
        lab[:, 52 + 1] = 1.0  # lfo1 triangle
        lab[:, 59 + 5] = 1.0  # lfo2 sample & hold
        h0 = h.clone()
        a, b = L.CATEGORICAL[0], L.PRESENCE[1]
        h0[:, :, :] = lab.unsqueeze(1)
        h0[:, :, a:b] = (lab.unsqueeze(1)[:, :, a:b] * 2 - 1) * 30.0
        h1 = h0[:, :, perm]  # the same sound with the LFOs exchanged
        l0, s0 = self.loss.hypothesis_losses(h0, lab, self.mask)
        l1, s1 = self.loss.hypothesis_losses(h1, lab, self.mask)
        self.assertTrue(t.allclose(l0, l1, atol=1e-6))
        self.assertFalse(bool(s0.any()))
        self.assertTrue(bool(s1.all()))

    def test_masked_entries_get_no_gradient(self) -> None:
        t = self.torch
        h = self.perfect(8).clone().requires_grad_(True)
        mask = self.mask.clone()
        mask[:, 10] = 0.0
        mask[:, 36:41] = 0.0  # osc1.wave unheard
        r = self.loss(h, t.zeros(4, 8), self.label, mask, temperature=0.5)
        r["total"].backward()
        self.assertEqual(float(h.grad[:, :, 10].abs().max()), 0.0)
        self.assertEqual(float(h.grad[:, :, 36:41].abs().max()), 0.0)
        self.assertEqual(float(h.grad[:, :, 0].abs().max()), 0.0, "voice.glide is masked in the fixture")

    def test_relaxed_wta_feeds_every_head(self) -> None:
        t = self.torch
        h = self.perfect(8)
        h[:, 1:, :33] = (h[:, 1:, :33] + 0.3).clamp(0, 1)
        h = h.clone().requires_grad_(True)
        r = self.loss(h, t.zeros(4, 8), self.label, self.mask, temperature=0.0)
        r["total"].backward()
        per_head = h.grad.abs().sum((0, 2))
        self.assertTrue(bool((per_head[1:] > 0).all()), "eps keeps the losing heads learning")

    def test_temperature_schedule(self) -> None:
        from enth_ml.train.loss import LossConfig

        c = LossConfig(t_start=1.0, t_end=0.01, anneal_frac=0.5)
        self.assertAlmostEqual(c.temperature(0.0), 1.0)
        self.assertAlmostEqual(c.temperature(0.25), 0.1, places=6)
        self.assertEqual(c.temperature(0.5), 0.0)
        self.assertEqual(c.temperature(0.9), 0.0)
        self.assertGreater(c.temperature(0.1), c.temperature(0.2))

    def test_annealing_softens_the_assignment(self) -> None:
        t = self.torch
        h = self.perfect(8)
        h[:, 1:, :33] = (h[:, 1:, :33] + 0.05).clamp(0, 1)
        self.loss.train()
        hot = self.loss(h, t.zeros(4, 8), self.label, self.mask, temperature=100.0)
        cold = self.loss(h, t.zeros(4, 8), self.label, self.mask, temperature=0.0)
        self.assertGreater(float(hot["hyp"]), float(cold["hyp"]), "a hot assignment spreads weight onto worse heads")


if __name__ == "__main__":
    unittest.main()
