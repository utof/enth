"""Dev metrics for propose v0: per-entry errors in JND units, accuracies, WTA statistics and the confidence's ranking.

Every metric is on the entries the mask keeps (what the sample's phrase reveals; an approximation of "on decoded
patches", since decoding canonicalises in Rust under the job's performance). Two hypotheses are scored per sample:

* ``best``: the oracle winner (the lowest loss of the Kₘ), what create gets when it renders all Kₘ and keeps the best;
* ``top``: the hypothesis the confidence head ranks first, what a single proposal would be.
"""

from __future__ import annotations

from collections import defaultdict

import numpy as np
import torch

from ..model import layout as L
from .loss import ProposeLoss


def _rank_desc(x: torch.Tensor) -> torch.Tensor:
    """Ranks (0 = largest) along the last axis."""
    order = x.argsort(dim=-1, descending=True)
    ranks = torch.empty_like(order)
    ranks.scatter_(-1, order, torch.arange(x.shape[-1], device=x.device).expand_as(order))
    return ranks


class DevMetrics:
    def __init__(self, loss: ProposeLoss, k: int) -> None:
        self.loss = loss
        self.k = k
        self.reset()

    def reset(self) -> None:
        self.n = 0
        self.sums: dict[str, float] = defaultdict(float)
        self.cont_err = {w: [[] for _ in range(L.CONTINUOUS[1])] for w in ("best", "top")}
        self.depth_err = {w: [] for w in ("best", "top")}
        self.cat = {w: np.zeros((len(L.GROUPS), 2)) for w in ("best", "top")}  # correct, count
        self.struct = {w: np.zeros((4, 2)) for w in ("best", "top")}
        self.pres = {w: np.zeros((16, 4)) for w in ("best", "top")}  # tp, fp, fn, tn
        self.win = np.zeros(self.k, dtype=np.int64)
        self.topk = np.zeros(self.k, dtype=np.int64)
        self.rank_of_winner = np.zeros(self.k, dtype=np.int64)
        self.spearman: list[float] = []
        self.distinct = 0.0  # summed over samples: distinct joint discrete choices among the K hypotheses
        self.cat_oracle = np.zeros((len(L.GROUPS), 2))  # some hypothesis right, count

    @torch.no_grad()
    def update(self, hyp: torch.Tensor, conf: torch.Tensor, label: torch.Tensor, mask: torch.Tensor) -> None:
        hyp, conf, label, mask = hyp.float(), conf.float(), label.float(), mask.float()
        losses, swapped = self.loss.hypothesis_losses(hyp, label, mask)
        b = hyp.shape[0]
        best_l, winner = losses.min(1)
        top = conf.argmax(1)
        ar = torch.arange(b, device=hyp.device)
        self.n += b
        self.sums["loss_best"] += best_l.sum().item()
        self.sums["loss_top"] += losses[ar, top].sum().item()
        self.sums["loss_mean_k"] += losses.mean(1).sum().item()
        self.sums["top_is_best"] += (top == winner).sum().item()
        self.win += np.bincount(winner.cpu().numpy(), minlength=self.k)
        self.topk += np.bincount(top.cpu().numpy(), minlength=self.k)
        ranks = _rank_desc(conf)
        self.rank_of_winner += np.bincount(ranks[ar, winner].cpu().numpy(), minlength=self.k)
        # Spearman between the confidence and -loss over the K hypotheses of each sample.
        rc = ranks.float()
        rl = _rank_desc(-losses).float()
        rc = rc - rc.mean(1, keepdim=True)
        rl = rl - rl.mean(1, keepdim=True)
        rho = (rc * rl).sum(1) / (rc.norm(dim=1) * rl.norm(dim=1)).clamp_min(1e-9)
        self.spearman.extend(rho.cpu().tolist())

        # Do the K hypotheses explore discrete alternatives? (TR2: the pilot's agreed on them.)
        s0, s1 = L.STRUCTURE
        p0, p1 = L.PRESENCE
        code = torch.cat(
            [torch.stack([hyp[..., a:bnd].argmax(-1) for _, a, bnd in L.GROUPS], -1), (hyp[..., s0:s1] > 0).long(), (hyp[..., p0:p1] > 0).long()], -1
        )  # [B, K, G + 4 + 16]
        eq = (code.unsqueeze(2) == code.unsqueeze(1)).all(-1)  # [B, K, K]
        earlier = torch.tril(torch.ones(self.k, self.k, dtype=torch.bool, device=hyp.device), diagonal=-1)
        self.distinct += float((~(eq & earlier).any(-1)).sum().item())
        for g, (_, a, bnd) in enumerate(L.GROUPS):
            gm = mask[:, a] > 0.5
            ok = (hyp[..., a:bnd].argmax(-1) == label[:, a:bnd].argmax(-1).unsqueeze(1)).any(1)
            self.cat_oracle[g, 0] += (ok & gm).sum().item()
            self.cat_oracle[g, 1] += gm.sum().item()

        perm = self.loss.perm
        for which, idx in (("best", winner), ("top", top)):
            sw = swapped[ar, idx]
            lab = torch.where(sw.unsqueeze(1), label[:, perm], label)
            msk = torch.where(sw.unsqueeze(1), mask[:, perm], mask)
            h = hyp[ar, idx]
            t = self.loss.terms(h.unsqueeze(1), lab, msk)
            e = t["e_cont"][:, 0].abs().cpu().numpy()
            m = t["m_cont"][:, 0].cpu().numpy() > 0.5
            for j in range(e.shape[1]):
                self.cont_err[which][j].append(e[m[:, j], j])
            ed = t["e_depth"][:, 0].abs().cpu().numpy()
            md = t["m_depth"][:, 0].cpu().numpy() > 0.5
            self.depth_err[which].append(ed[md])
            for g, (_, a, bnd) in enumerate(L.GROUPS):
                gm = msk[:, a] > 0.5
                ok = h[:, a:bnd].argmax(1) == lab[:, a:bnd].argmax(1)
                self.cat[which][g, 0] += (ok & gm).sum().item()
                self.cat[which][g, 1] += gm.sum().item()
            s0, s1 = L.STRUCTURE
            sm = msk[:, s0:s1] > 0.5
            sok = (h[:, s0:s1] > 0) == (lab[:, s0:s1] > 0.5)
            self.struct[which][:, 0] += (sok & sm).sum(0).cpu().numpy()
            self.struct[which][:, 1] += sm.sum(0).cpu().numpy()
            p0, p1 = L.PRESENCE
            pm = msk[:, p0:p1] > 0.5
            pp = h[:, p0:p1] > 0
            pt = lab[:, p0:p1] > 0.5
            for col, val in enumerate(((pp & pt), (pp & ~pt), (~pp & pt), (~pp & ~pt))):
                self.pres[which][:, col] += (val & pm).sum(0).cpu().numpy()

    def summary(self, per_entry: bool = True) -> dict:
        n = max(self.n, 1)
        out: dict = {
            "n": self.n,
            "loss_best": self.sums["loss_best"] / n,
            "loss_top": self.sums["loss_top"] / n,
            "loss_mean_k": self.sums["loss_mean_k"] / n,
            "conf_top1": self.sums["top_is_best"] / n,
        }
        win = self.win / max(self.win.sum(), 1)
        out["wta"] = {
            "win_share": [round(float(x), 4) for x in win],
            "heads_winning_2pct": int((win >= 0.02).sum()),
            "max_share": float(win.max()) if self.win.sum() else 0.0,
            "entropy_bits": float(-(win[win > 0] * np.log2(win[win > 0])).sum()),
            "top_share": [round(float(x), 4) for x in self.topk / max(self.topk.sum(), 1)],
        }
        r = self.rank_of_winner / max(self.rank_of_winner.sum(), 1)
        out["conf"] = {
            "top1": out["conf_top1"],
            "winner_rank_share": [round(float(x), 4) for x in r],
            "winner_in_top3": float(r[:3].sum()),
            "mean_winner_rank": float((r * np.arange(self.k)).sum()),
            "spearman": float(np.mean(self.spearman)) if self.spearman else 0.0,
            "regret": out["loss_top"] - out["loss_best"],
        }
        oracle = {name: (float(c / t) if t else None) for (name, _, _), (c, t) in zip(L.GROUPS, self.cat_oracle)}
        known = [v for v in oracle.values() if v is not None]
        out["discrete"] = {
            "distinct_mean": self.distinct / n,
            "categorical_oracle_acc": oracle,
            "categorical_oracle_mean": float(np.mean(known)) if known else None,
        }
        for which in ("best", "top"):
            errs = [np.concatenate(x) if x else np.zeros(0) for x in self.cont_err[which]]
            allc = np.concatenate(errs) if errs else np.zeros(0)
            dep = np.concatenate(self.depth_err[which]) if self.depth_err[which] else np.zeros(0)
            seg = {
                "continuous_mean_jnd": float(allc.mean()) if allc.size else None,
                "continuous_median_jnd": float(np.median(allc)) if allc.size else None,
                "continuous_within_1jnd": float((allc <= 1).mean()) if allc.size else None,
                "continuous_within_3jnd": float((allc <= 3).mean()) if allc.size else None,
                "depth_mean_jnd": float(dep.mean()) if dep.size else None,
                "depth_median_jnd": float(np.median(dep)) if dep.size else None,
                "categorical_acc": {
                    name: (float(c / t) if t else None) for (name, _, _), (c, t) in zip(L.GROUPS, self.cat[which])
                },
                "structure_acc": {
                    name: (float(c / t) if t else None) for name, (c, t) in zip(L.STRUCTURE_NAMES, self.struct[which])
                },
            }
            p = self.pres[which].sum(0)
            tp, fp, fn, tn = p
            seg["presence_acc"] = float((tp + tn) / max(p.sum(), 1))
            seg["presence_precision"] = float(tp / max(tp + fp, 1))
            seg["presence_recall"] = float(tp / max(tp + fn, 1))
            if per_entry:
                seg["continuous_per_entry"] = {
                    name: {
                        "mean": float(e.mean()) if e.size else None,
                        "median": float(np.median(e)) if e.size else None,
                        "n": int(e.size),
                    }
                    for name, e in zip(L.CONTINUOUS_NAMES, errs)
                }
            out[which] = seg
        return out
