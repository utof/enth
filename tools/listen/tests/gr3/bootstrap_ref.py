#!/usr/bin/env python3
"""Reference implementation of GR3's cluster bootstrap (the decision rule of `enth-listen gr3 --analyze`).

Standard library only. Run it isolated: `python3 -I bootstrap_ref.py ...`.

    bootstrap_ref.py boot FIXTURE.json [--replicates N]   print the bootstrap of the fixture as JSON
    bootstrap_ref.py synth [--seed N]                     print a synthetic fixture (owner + two friends, 60 items)

A fixture is {"schema": "enth.listen.gr3-judgements/1", "judgements": [...]}; each judgement has
    listener, item, cluster, stratum (strings), degraded (bool), b (bool: arm B's candidate was chosen).

The procedure is the one written in tools/listen/src/gr3/boot.rs, step by step, so that the two agree:

  * SplitMix64 from the fixed seed SEED (0x4752_3342_4F4F_5453, "GR3BOOTS"); `below(n)` is `next() % n`.
  * Clusters sorted by name; within a cluster, listeners sorted by name; per (cluster, listener): (wins, judgements).
  * One replicate: draw C cluster indices with replacement (C = number of clusters) -> the references-only share
    (the drawn clusters' wins over judgements); then, for each drawn cluster in draw order, draw as many of its
    listeners as it has, with replacement -> the two-stage share (the rule's).
  * The interval is the 2.5th and 97.5th percentile of the sorted replicates, linear interpolation (type 7).
  * Design effect = (sd of the references-only replicates)^2 / (p (1 - p) / n).
"""

import json
import math
import sys

MASK = (1 << 64) - 1
SEED = 0x4752_3342_4F4F_5453  # "GR3BOOTS"
REPLICATES = 10_000
TAIL = 0.025


class SplitMix64:
    def __init__(self, seed):
        self.s = seed & MASK

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & MASK
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def below(self, n):
        return self.next() % n

    def uniform(self):
        return (self.next() >> 11) / float(1 << 53)


def group(js):
    by = {}
    for j in js:
        cell = by.setdefault(j["cluster"], {}).setdefault(j["listener"], [0, 0])
        cell[0] += 1 if j["b"] else 0
        cell[1] += 1
    return [[tuple(by[c][l]) for l in sorted(by[c])] for c in sorted(by)]


def quantile(sorted_x, q):
    pos = q * (len(sorted_x) - 1)
    lo = int(math.floor(pos))
    frac = pos - lo
    if lo + 1 < len(sorted_x):
        return sorted_x[lo] + frac * (sorted_x[lo + 1] - sorted_x[lo])
    return sorted_x[lo]


def variance(x):
    # Left to right, like the Rust: no compensated sum, no fsum.
    n = float(len(x))
    s = 0.0
    for v in x:
        s += v
    mean = s / n
    t = 0.0
    for v in x:
        t += (v - mean) * (v - mean)
    return t / (n - 1.0)


def boot(js, replicates=REPLICATES, seed=SEED):
    if not js:
        return None
    groups = group(js)
    c = len(groups)
    wins = sum(1 for j in js if j["b"])
    n = len(js)
    listeners = len({j["listener"] for j in js})
    replicates = max(replicates, 2)
    rng = SplitMix64(seed)
    one, two = [], []
    for _ in range(replicates):
        drawn = [rng.below(c) for _ in range(c)]
        w1 = n1 = 0
        for g in drawn:
            for (w, k) in groups[g]:
                w1 += w
                n1 += k
        w2 = n2 = 0
        for g in drawn:
            ls = groups[g]
            for _ in range(len(ls)):
                (w, k) = ls[rng.below(len(ls))]
                w2 += w
                n2 += k
        one.append(w1 / n1)
        two.append(w2 / n2)
    se_ref = math.sqrt(variance(one))
    se = math.sqrt(variance(two))
    one.sort()
    two.sort()
    p = wins / n
    srs = p * (1.0 - p) / n

    def ratio(s):
        return (s * s / srs) if srs > 0.0 else None

    return {
        "n": n,
        "wins": wins,
        "clusters": c,
        "listeners": listeners,
        "replicates": replicates,
        "p": p,
        "lo": quantile(two, TAIL),
        "hi": quantile(two, 1.0 - TAIL),
        "se": se,
        "lo_ref": quantile(one, TAIL),
        "hi_ref": quantile(one, 1.0 - TAIL),
        "se_ref": se_ref,
        "deff": ratio(se_ref),
        "deff_two_stage": ratio(se),
    }


def report(js, replicates):
    """The same subsets the Rust report bootstraps: all, clean, degraded and each stratum."""
    strata = sorted({j["stratum"] for j in js})
    return {
        "all": boot(js, replicates),
        "clean": boot([j for j in js if not j["degraded"]], replicates),
        "degraded": boot([j for j in js if j["degraded"]], replicates),
        "strata": {s: boot([j for j in js if j["stratum"] == s], replicates) for s in strata},
    }


def synth(seed):
    """A fixture shaped like the real test: 40 clean references in four strata, 20 degraded copies that share their
    clean original's cluster, an owner who hears all 60 and two friends who hear 30 each; a latent per-reference
    effect makes the judgements of one reference agree more than chance (a design effect above 1)."""
    rng = SplitMix64(seed)

    def normal():
        u1 = max(rng.uniform(), 1e-12)
        u2 = rng.uniform()
        return math.sqrt(-2.0 * math.log(u1)) * math.cos(2.0 * math.pi * u2)

    names = ["subtractive", "fm-wavetable", "acoustic-percussive", "owner-picks"]
    items = []
    effect = {}
    for i in range(40):
        items.append((f"ref-{i:02d}", f"ref-{i:02d}", names[i // 10], False))
        effect[f"ref-{i:02d}"] = 0.7 * normal()
    for i in range(20):
        items.append((f"deg-{i:02d}", f"ref-{i:02d}", names[i // 10], True))
    order = list(range(60))
    for i in range(59, 0, -1):
        j = rng.below(i + 1)
        order[i], order[j] = order[j], order[i]
    halves = {"friend-1": set(order[:30]), "friend-2": set(order[30:])}
    base_clean = math.log(0.65 / 0.35)
    base_degraded = math.log(0.60 / 0.40)
    out = []
    for listener in ["owner", "friend-1", "friend-2"]:
        for idx, (item, cluster, stratum, degraded) in enumerate(items):
            if listener != "owner" and idx not in halves[listener]:
                continue
            logit = (base_degraded if degraded else base_clean) + effect[cluster]
            b = rng.uniform() < 1.0 / (1.0 + math.exp(-logit))
            out.append(
                {
                    "listener": listener,
                    "item": item,
                    "cluster": cluster,
                    "stratum": stratum,
                    "degraded": degraded,
                    "b": b,
                }
            )
    return {"schema": "enth.listen.gr3-judgements/1", "judgements": out}


def main(argv):
    if len(argv) >= 3 and argv[1] == "boot":
        replicates = REPLICATES
        if "--replicates" in argv:
            replicates = int(argv[argv.index("--replicates") + 1])
        with open(argv[2], encoding="utf-8") as f:
            fixture = json.load(f)
        print(json.dumps(report(fixture["judgements"], replicates), indent=1, sort_keys=True))
        return 0
    if len(argv) >= 2 and argv[1] == "synth":
        seed = 20261021
        if "--seed" in argv:
            seed = int(argv[argv.index("--seed") + 1])
        print(json.dumps(synth(seed), indent=1))
        return 0
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
