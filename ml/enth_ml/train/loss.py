"""propose v0's loss (docs/r3.md, "The model: propose v0", and the model contract's TR row).

Per hypothesis k, the loss of a sample is the masked sum of

* continuous values (offsets 0-32): a Huber loss (delta 1) on the error **in JND units**: the error divided by the
  just-noticeable step at the label's value, interpolated linearly from the manifest's 33 x 17 table (computed in Rust;
  we never re-implement ``ParamSpec``);
* route depths (86-101): a Huber loss on the error in steps of the manifest's scale (0.025, one 5 % step);
* categoricals (33-65): cross-entropy per group;
* structure (66-69) and route presence (70-85): binary cross-entropy on the logits;

divided by the number of terms that carry a loss (the mask decides: ``observable()``, the 2 s window's releases, an
absent route's depth). The two LFOs are symmetric, so each hypothesis takes the **minimum over the two permutations**
(the contract's swap, masks swapped too).

Over the Kₘ hypotheses: **winner-takes-all**, annealed then relaxed. Each sample's hypotheses are weighted by
``(1 - eps) * softmax(-L_k / (T * s)) + eps / K`` (``s`` a running mean of the loss, so ``T`` has no units), with the
temperature ``T`` annealed to 0 (hard WTA: the arg-min) and a floor ``eps`` that keeps every head learning (relaxed WTA).
The confidence head learns the hard winner by cross-entropy.

Lane TR2's options for the full run (defaults off): a positive-class weight on route presence; the confidence on soft
targets ``softmax(-L_k / (tau * s))`` from the per-head losses; and a **WTA selection that counts discrete misses more**
(``select_discrete_boost``): the weights ``q`` (and the hard winner) come from a selection loss whose categorical,
structure and presence terms are multiplied by the boost, while the gradient still flows through the training loss, so
a head is made responsible for the samples whose discrete choices it gets right and the heads can specialise on
discrete alternatives instead of agreeing on them.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass

import torch
import torch.nn.functional as F
from torch import nn

from ..model import layout as L


@dataclass
class LossConfig:
    w_continuous: float = 1.0
    w_depth: float = 1.0
    w_categorical: float = 2.0
    w_structure: float = 2.0
    w_presence: float = 1.0
    huber_delta: float = 1.0  # in JND units
    conf_weight: float = 0.2
    eps: float = 0.05  # relaxed WTA floor
    t_start: float = 1.0  # annealing temperature at step 0 (in units of the running mean loss)
    t_end: float = 0.01
    anneal_frac: float = 0.3  # share of the run over which T falls geometrically; hard WTA after it
    # Lane TR2 (the full run; every default keeps the pilot's behaviour):
    presence_pos_weight: float = 1.0  # BCE weight of a present route (the pilot's presence recall was 0.55)
    conf_soft_tau: float = 0.0  # > 0: the confidence learns softmax(-L_k / (tau * s)) instead of the hard winner
    select_discrete_boost: float = 1.0  # > 1: the WTA winner is chosen with the discrete terms weighted this much more

    def to_dict(self) -> dict:
        return asdict(self)

    def temperature(self, progress: float) -> float:
        """T at ``progress`` in [0, 1] of the run: geometric from ``t_start`` to ``t_end``, then 0 (hard)."""
        if self.anneal_frac <= 0 or progress >= self.anneal_frac:
            return 0.0
        r = progress / self.anneal_frac
        return float(self.t_start * (self.t_end / self.t_start) ** r)


def huber(e: torch.Tensor, delta: float) -> torch.Tensor:
    a = e.abs()
    return torch.where(a <= delta, 0.5 * e * e, delta * (a - 0.5 * delta))


class ProposeLoss(nn.Module):
    """Per-hypothesis losses, the WTA weighting and the confidence loss."""

    def __init__(self, jnd_table, jnd_scale, cfg: LossConfig | None = None) -> None:
        super().__init__()
        self.cfg = cfg or LossConfig()
        table = torch.as_tensor(jnd_table, dtype=torch.float32)
        scale = torch.as_tensor(jnd_scale, dtype=torch.float32)
        if table.shape != (L.CONTINUOUS[1], 17) or scale.shape != (L.WIDTH,):
            raise ValueError(f"JND table {tuple(table.shape)} and scale {tuple(scale.shape)}")
        if not torch.all(table > 0) or not torch.all(scale > 0):
            raise ValueError("JND steps must be positive")
        self.register_buffer("table", table)
        self.register_buffer("depth_step", scale[L.DEPTH[0] : L.DEPTH[1]].clone())
        self.register_buffer("perm", torch.tensor(L.swap_permutation(), dtype=torch.long))
        self.register_buffer("running", torch.tensor(0.0))
        self._started = False
        p0, p1 = L.PRESENCE
        self.register_buffer("pos_weight", torch.full((p1 - p0,), float(self.cfg.presence_pos_weight)))
        groups = torch.zeros(len(L.GROUPS), L.WIDTH, dtype=torch.bool)
        for g, (_, a, b) in enumerate(L.GROUPS):
            groups[g, a:b] = True
        self.register_buffer("groups", groups)

    # -- JND --------------------------------------------------------------------------------------------------------
    def jnd_at(self, label_cont: torch.Tensor) -> torch.Tensor:
        """The JND step of each continuous entry at its label's value: ``label_cont`` [..., 33] -> [..., 33]."""
        q = label_cont.clamp(0.0, 1.0) * 16.0
        lo = q.floor().clamp(max=15.0)
        frac = q - lo
        lo = lo.long()
        n = label_cont.shape[-1]
        t = self.table  # [33, 17]
        idx = torch.arange(n, device=label_cont.device)
        a = t[idx, lo]  # advanced indexing broadcasts idx over the leading axes
        b = t[idx, lo + 1]
        return a + (b - a) * frac

    # -- per-term losses --------------------------------------------------------------------------------------------
    def terms(self, hyp: torch.Tensor, label: torch.Tensor, mask: torch.Tensor) -> dict[str, torch.Tensor]:
        """Unweighted per-entry terms for ``hyp`` [B, K, 102] against ``label``/``mask`` [B, 102].

        Returns continuous [B, K, 33] and depth [B, K, 16] errors in JND units (signed), categorical cross-entropy
        [B, K, 7], structure and presence BCE [B, K, 4] and [B, K, 16], and the matching masks [B, 1, n]."""
        c0, c1 = L.CONTINUOUS
        d0, d1 = L.DEPTH
        lab = label.unsqueeze(1)
        msk = mask.unsqueeze(1)
        jnd = self.jnd_at(label[:, c0:c1]).unsqueeze(1)
        e_cont = (hyp[..., c0:c1] - lab[..., c0:c1]) / jnd
        e_depth = (hyp[..., d0:d1] - lab[..., d0:d1]) / self.depth_step
        ce = []
        gmask = []
        for _, a, b in L.GROUPS:
            logits = hyp[..., a:b]
            target = lab[..., a:b].argmax(-1).expand(logits.shape[:-1])
            ce.append(F.cross_entropy(logits.reshape(-1, b - a), target.reshape(-1), reduction="none").reshape(target.shape))
            gmask.append(msk[..., a])
        s0, s1 = L.STRUCTURE
        p0, p1 = L.PRESENCE
        bce_s = F.binary_cross_entropy_with_logits(hyp[..., s0:s1], lab[..., s0:s1].expand_as(hyp[..., s0:s1]), reduction="none")
        pw = self.pos_weight if self.cfg.presence_pos_weight != 1.0 else None
        bce_p = F.binary_cross_entropy_with_logits(hyp[..., p0:p1], lab[..., p0:p1].expand_as(hyp[..., p0:p1]), reduction="none", pos_weight=pw)
        return {
            "e_cont": e_cont,
            "e_depth": e_depth,
            "ce": torch.stack(ce, -1),
            "bce_s": bce_s,
            "bce_p": bce_p,
            "m_cont": msk[..., c0:c1],
            "m_depth": msk[..., d0:d1],
            "m_cat": torch.stack(gmask, -1),
            "m_s": msk[..., s0:s1],
            "m_p": msk[..., p0:p1],
        }

    def per_hypothesis(self, hyp: torch.Tensor, label: torch.Tensor, mask: torch.Tensor, discrete_boost: float = 1.0) -> torch.Tensor:
        """The loss of each hypothesis against one labelling: [B, K]. ``discrete_boost`` multiplies the categorical,
        structure and presence terms (the WTA selection loss; 1 is the training loss)."""
        c = self.cfg
        t = self.terms(hyp, label, mask)
        cont = c.w_continuous * (huber(t["e_cont"], c.huber_delta) * t["m_cont"]).sum(-1) + c.w_depth * (huber(t["e_depth"], c.huber_delta) * t["m_depth"]).sum(-1)
        disc = c.w_categorical * (t["ce"] * t["m_cat"]).sum(-1) + c.w_structure * (t["bce_s"] * t["m_s"]).sum(-1) + c.w_presence * (t["bce_p"] * t["m_p"]).sum(-1)
        den = t["m_cont"].sum(-1) + t["m_depth"].sum(-1) + t["m_cat"].sum(-1) + t["m_s"].sum(-1) + t["m_p"].sum(-1)
        num = cont + disc if discrete_boost == 1.0 else cont + discrete_boost * disc
        return num / den.clamp_min(1.0)

    def hypothesis_losses(self, hyp: torch.Tensor, label: torch.Tensor, mask: torch.Tensor, discrete_boost: float = 1.0) -> tuple[torch.Tensor, torch.Tensor]:
        """[B, K] losses with the LFO swap's minimum taken, and which permutation won (0 as labelled, 1 swapped)."""
        a = self.per_hypothesis(hyp, label, mask, discrete_boost)
        b = self.per_hypothesis(hyp, label[:, self.perm], mask[:, self.perm], discrete_boost)
        swapped = b < a
        return torch.where(swapped, b, a), swapped

    def forward(self, hyp: torch.Tensor, conf: torch.Tensor, label: torch.Tensor, mask: torch.Tensor, temperature: float = 0.0) -> dict[str, torch.Tensor]:
        hyp = hyp.float()
        conf = conf.float()
        losses, swapped = self.hypothesis_losses(hyp, label.float(), mask.float())
        k = losses.shape[1]
        best, winner = losses.min(dim=1)
        with torch.no_grad():
            m = losses.mean()
            if self.training:
                # No .item(): a host sync every step would stall the GPU queue.
                self.running.copy_(0.99 * self.running + 0.01 * m if self._started else m)
                self._started = True
            s = self.running.clamp_min(1e-6)
            boost = self.cfg.select_discrete_boost
            sel = losses if boost == 1.0 else self.hypothesis_losses(hyp, label.float(), mask.float(), boost)[0]
            if temperature > 0.0:
                q = torch.softmax(-sel / (temperature * s * (1.0 if boost == 1.0 else boost)), dim=1)
            else:
                q = F.one_hot(sel.argmin(dim=1), k).float()
            w = (1.0 - self.cfg.eps) * q + self.cfg.eps / k
            conf_target = torch.softmax(-losses / (self.cfg.conf_soft_tau * s), dim=1) if self.cfg.conf_soft_tau > 0 else winner
        hyp_loss = (w * losses).sum(1).mean()
        conf_loss = F.cross_entropy(conf, conf_target)
        total = hyp_loss + self.cfg.conf_weight * conf_loss
        return {
            "total": total,
            "hyp": hyp_loss,
            "conf": conf_loss,
            "best": best.mean().detach(),
            "losses": losses.detach(),
            "winner": winner.detach(),
            "swapped": swapped.detach(),
        }
