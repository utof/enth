"""``python -m enth_ml.train compare RUN_A RUN_B [--step N]``: is B's recipe better than A's? A paired, per-sample
comparison of two runs on the same dev samples (lane FTL; the statistics of the 2026-10-25 research session, 01 §5 and
02 §2.2; ml/README.md, "Fast loop"). numpy only: it never imports torch.

**Input.** Each run's per-sample dev file, ``dev-persample-<step>.npz`` (``persample_evals``), at ``--step`` or at
each run's newest; or an ``.npz`` path directly (``python -m enth_ml.train eval --ckpt C --persample F``). Samples pair
by their ``index`` (the dev stream's candidate index), so both files must come from the same dev set. A run may be a
comma-separated list of replicate runs (other seeds), paired by position: ``compare a-s1,a-s2 b-s1,b-s2``.

**What it prints.**

* For best-of-K (the decision metric), top-ranked and mean-K: the paired mean difference A - B (positive: B is
  better), its SE, a bootstrap 95 % CI, the Wilcoxon signed-rank test (normal approximation, ties averaged), the share
  of samples each run is better on, and the correlation of the two runs' per-sample losses;
* the discrete metrics (distinct joint choices among the K, the categorical oracle, presence recall, the confidence's
  top-1) from each run's ``metrics.jsonl`` at that step, each with a **jitter band**: twice the combined
  second-difference noise of the two runs' own dev curves over their last third (Rice's estimator, sd(Δ²x)/√6), and
  the per-sample distinct choices as a paired test;
* best-of-K per data source (generator v1, preset, uniform);
* **the verdict**, on best-of-K: "B better" or "A better" when the gap is over ``max(3 x paired SE, 2 x seed SD)``
  (and, with replicates, every seed's gap has the same sign), else "no decision at this tier". The seed SD is measured
  when replicates are given (the pooled within-arm SD of the runs' mean losses), else taken from ``--seed-sd`` (the
  calibration's number); without either it is left out and the verdict says so (dev-sample noise only).
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import re
import sys

import numpy as np

LOSSES = ("best", "top", "meank")
LOSS_NAMES = {"best": "best-of-K", "top": "top-ranked", "meank": "mean over K"}
SOURCES = {0: "generator v1", 1: "preset", 2: "uniform", 255: "unknown"}
# Discrete metrics read from metrics.jsonl's dev records: (label, path in the record).
DISCRETE = (
    ("distinct joint choices of K", ("discrete", "distinct_mean")),
    ("categorical oracle (mean)", ("discrete", "categorical_oracle_mean")),
    ("presence recall (best)", ("best", "presence_recall")),
    ("confidence top-1", ("conf_top1",)),
)
_FILE = re.compile(r"^dev-persample-(\d+)\.npz$")


# -- reading -----------------------------------------------------------------------------------------------------------


def persample_files(run: pathlib.Path) -> dict[int, pathlib.Path]:
    """A run directory's per-sample files by step."""
    return {int(m.group(1)): p for p in run.glob("dev-persample-*.npz") if (m := _FILE.match(p.name))}


def load(source: str | pathlib.Path, step: int | None = None) -> dict:
    """One side: a per-sample file's arrays, its step, its name and (for a run directory) the directory."""
    path = pathlib.Path(source)
    run = None
    if path.is_dir():
        run = path
        files = persample_files(path)
        if not files:
            raise FileNotFoundError(f"{path}: no dev-persample-<step>.npz (train with persample_evals = \"final\" or \"all\", or use eval --persample)")
        if step is None:
            step = max(files)
        if step not in files:
            raise FileNotFoundError(f"{path}: no per-sample file at step {step} (have {sorted(files)})")
        path = files[step]
    with np.load(path) as z:
        out = {k: z[k] for k in z.files if k not in ("meta",)}
        meta = json.loads(bytes(z["meta"]).decode()) if "meta" in z.files else {}
    out["step"] = int(out["step"]) if "step" in out else meta.get("step")
    out["name"] = run.name if run is not None else path.name
    out["run"] = run
    out["meta"] = meta
    for k in LOSSES:
        if k not in out:
            raise ValueError(f"{path}: no {k!r} array")
    return out


def align(a: dict, b: dict) -> dict:
    """``b`` reordered to ``a``'s samples (by ``index``); refuses files from different dev sets."""
    n = len(a["best"])
    if "index" in a and "index" in b:
        if np.array_equal(a["index"], b["index"]):
            return b
        if sorted(a["index"].tolist()) != sorted(b["index"].tolist()):
            raise ValueError(f"{a['name']} and {b['name']} were evaluated on different dev samples")
        if len(set(a["index"].tolist())) != n:
            raise ValueError(f"{a['name']} and {b['name']}: repeated sample indices (a dev set of several runs?) in different orders: cannot pair them")
        pos = {int(v): i for i, v in enumerate(b["index"].tolist())}
        order = np.array([pos[int(v)] for v in a["index"].tolist()])
        return {k: (v[order] if isinstance(v, np.ndarray) and v.shape[:1] == (n,) else v) for k, v in b.items()}
    if len(b["best"]) != n:
        raise ValueError(f"{a['name']} has {n} samples, {b['name']} {len(b['best'])}")
    return b


# -- statistics --------------------------------------------------------------------------------------------------------


def wilcoxon(d: np.ndarray) -> dict:
    """The Wilcoxon signed-rank test of ``d`` against 0: zeros dropped, tied |d| given their average rank, the normal
    approximation with the tie correction. ``z > 0`` when the positive differences dominate."""
    d = np.asarray(d, dtype=np.float64)
    d = d[d != 0]
    n = len(d)
    if n == 0:
        return {"n": 0, "w_plus": 0.0, "z": 0.0, "p": 1.0}
    a = np.abs(d)
    order = np.argsort(a, kind="mergesort")
    _, inv, counts = np.unique(a[order], return_inverse=True, return_counts=True)
    ends = np.cumsum(counts)
    avg = (ends - counts + 1 + ends) / 2.0
    ranks = np.empty(n)
    ranks[order] = avg[inv]
    w_plus = float(ranks[d > 0].sum())
    mu = n * (n + 1) / 4.0
    var = n * (n + 1) * (2 * n + 1) / 24.0 - float((counts**3 - counts).sum()) / 48.0
    z = (w_plus - mu) / math.sqrt(var) if var > 0 else 0.0
    return {"n": n, "w_plus": w_plus, "z": z, "p": math.erfc(abs(z) / math.sqrt(2.0))}


def bootstrap_ci(d: np.ndarray, n_boot: int = 4000, seed: int = 0, level: float = 0.95) -> tuple[float, float]:
    """A percentile bootstrap CI of the mean of ``d`` (samples resampled with replacement; seeded, so reproducible)."""
    d = np.asarray(d, dtype=np.float64)
    n = len(d)
    rng = np.random.default_rng(seed)
    means = np.empty(n_boot)
    chunk = max(1, min(n_boot, 4_000_000 // max(n, 1)))
    for i in range(0, n_boot, chunk):
        k = min(chunk, n_boot - i)
        means[i : i + k] = d[rng.integers(0, n, size=(k, n))].mean(axis=1)
    lo, hi = np.percentile(means, [100 * (1 - level) / 2, 100 * (1 + level) / 2])
    return float(lo), float(hi)


def paired(x: np.ndarray, y: np.ndarray, n_boot: int = 4000, seed: int = 0) -> dict:
    """``x - y`` per sample: mean, SD, SE, t, bootstrap CI, Wilcoxon, the shares where each side is lower."""
    x = np.asarray(x, dtype=np.float64)
    y = np.asarray(y, dtype=np.float64)
    d = x - y
    n = len(d)
    sd = float(d.std(ddof=1)) if n > 1 else 0.0
    se = sd / math.sqrt(n) if n else 0.0
    mean = float(d.mean()) if n else 0.0
    lo, hi = bootstrap_ci(d, n_boot, seed) if n > 1 else (mean, mean)
    corr = float(np.corrcoef(x, y)[0, 1]) if n > 1 and x.std() > 0 and y.std() > 0 else None
    return {
        "n": n,
        "mean_a": float(x.mean()),
        "mean_b": float(y.mean()),
        "delta": mean,
        "sd": sd,
        "se": se,
        "t": mean / se if se > 0 else None,
        "ci95": [lo, hi],
        "wilcoxon": wilcoxon(d),
        "share_b_lower": float((d > 0).mean()),
        "share_a_lower": float((d < 0).mean()),
        "corr": corr,
    }


def rice_jitter(values: list[float]) -> float | None:
    """Rice's second-difference estimate of a curve's eval-to-eval noise: sd(x[i+1] - 2x[i] + x[i-1]) / √6."""
    v = np.asarray([x for x in values if x is not None and math.isfinite(x)], dtype=np.float64)
    if len(v) < 4:
        return None
    return float(np.diff(v, 2).std(ddof=1) / math.sqrt(6.0))


def _get(rec: dict, path: tuple[str, ...]):
    for k in path:
        if not isinstance(rec, dict) or k not in rec:
            return None
        rec = rec[k]
    return rec


def dev_records(run: pathlib.Path) -> list[dict]:
    """A run's dev records, one per step (the last written: a final evaluation over an interval one), by step."""
    by_step: dict[int, dict] = {}
    p = run / "metrics.jsonl"
    if not p.exists():
        return []
    for line in p.read_text().splitlines():
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if r.get("kind") == "dev":
            by_step[int(r["step"])] = r
    return [by_step[s] for s in sorted(by_step)]


def discrete_at(run: pathlib.Path | None, step: int | None) -> dict:
    """Each discrete metric at ``step`` and its jitter over the run's last third of evaluations (at least 5)."""
    if run is None:
        return {}
    recs = dev_records(run)
    if not recs:
        return {}
    at = [r for r in recs if int(r["step"]) == step] if step is not None else []
    rec = at[-1] if at else None
    tail = recs[-max(5, len(recs) // 3) :]
    out = {}
    for label, path in DISCRETE:
        out[label] = {"value": _get(rec, path) if rec else None, "jitter": rice_jitter([_get(r, path) for r in tail])}
    return out


# -- the comparison ----------------------------------------------------------------------------------------------------


def verdict(delta: float, se: float, seed_sd: float | None, per_seed: list[float] | None = None) -> tuple[str, float, str]:
    """``("A better" | "B better" | "no decision at this tier", threshold, why)`` for a best-of-K gap ``delta`` (A - B)."""
    thr = 3.0 * se
    why = "3 x paired SE"
    if seed_sd is not None and 2.0 * seed_sd > thr:
        thr = 2.0 * seed_sd
        why = "2 x seed SD"
    consistent = per_seed is None or all(x > 0 for x in per_seed) or all(x < 0 for x in per_seed)
    if abs(delta) > thr and consistent:
        return ("B better" if delta > 0 else "A better"), thr, why
    if not consistent:
        why += "; the seeds disagree on the sign"
    return "no decision at this tier", thr, why


def compare(a_sources: list[str], b_sources: list[str], step: int | None = None, seed_sd: float | None = None, n_boot: int = 4000) -> dict:
    """The comparison as a dict (``format_report`` prints it)."""
    A = [load(s, step) for s in a_sources]
    B = [load(s, step) for s in b_sources]
    ref = A[0]
    A = [ref] + [align(ref, x) for x in A[1:]]
    B = [align(ref, x) for x in B]
    k = min(len(A), len(B))
    rep: dict = {
        "a": [{"name": x["name"], "step": x["step"]} for x in A],
        "b": [{"name": x["name"], "step": x["step"]} for x in B],
        "n": len(ref["best"]),
        "replicates": k,
    }
    # The decision metric and its companions: averaged over the seeds when replicates are given.
    rep["losses"] = {}
    for m in LOSSES:
        xa = np.mean([x[m] for x in A[:k]], axis=0)
        xb = np.mean([x[m] for x in B[:k]], axis=0)
        rep["losses"][m] = paired(xa, xb, n_boot)
    per_seed = None
    measured_sd = None
    if k >= 2:
        per_seed = [float(np.mean(A[j]["best"].astype(np.float64) - B[j]["best"].astype(np.float64))) for j in range(k)]
        va = np.var([float(x["best"].mean()) for x in A], ddof=1) if len(A) >= 2 else None
        vb = np.var([float(x["best"].mean()) for x in B], ddof=1) if len(B) >= 2 else None
        pooled = [v for v in (va, vb) if v is not None]
        measured_sd = float(math.sqrt(sum(pooled) / len(pooled))) if pooled else None
        rep["seeds"] = {"delta_per_seed": per_seed, "seed_sd_measured": measured_sd, "a_means": [float(x["best"].mean()) for x in A], "b_means": [float(x["best"].mean()) for x in B]}
    sd_used = measured_sd if measured_sd is not None else seed_sd
    best = rep["losses"]["best"]
    v, thr, why = verdict(best["delta"], best["se"], sd_used, per_seed)
    rep["verdict"] = {
        "verdict": v,
        "threshold": thr,
        "rule": why,
        "seed_sd": sd_used,
        "seed_sd_source": "measured from the replicates" if measured_sd is not None else ("--seed-sd" if seed_sd is not None else None),
        "note": None if sd_used is not None else "seed noise not included (no replicates and no --seed-sd): dev-sample noise only; calibrate T1 first",
    }
    # Per-sample discrete: distinct joint choices among the K.
    if all("distinct" in x for x in A[:k] + B[:k]):
        da = np.mean([x["distinct"].astype(np.float64) for x in A[:k]], axis=0)
        db = np.mean([x["distinct"].astype(np.float64) for x in B[:k]], axis=0)
        rep["distinct_paired"] = paired(da, db, n_boot=1000)
    # Discrete metrics from the runs' logs, with jitter bands (the first replicate of each side).
    da_, db_ = discrete_at(A[0]["run"], A[0]["step"]), discrete_at(B[0]["run"], B[0]["step"])
    rep["discrete"] = {}
    for label, _ in DISCRETE:
        xa, xb = da_.get(label, {}), db_.get(label, {})
        va_, vb_ = xa.get("value"), xb.get("value")
        if va_ is None or vb_ is None:
            continue
        ja, jb = xa.get("jitter"), xb.get("jitter")
        band = 2.0 * math.sqrt((ja or 0.0) ** 2 + (jb or 0.0) ** 2) if (ja is not None and jb is not None) else None
        delta = float(va_) - float(vb_)
        rep["discrete"][label] = {"a": float(va_), "b": float(vb_), "delta": delta, "band": band, "beyond_band": None if band is None else abs(delta) > band}
    # Per source.
    if "source" in ref:
        rep["by_source"] = {}
        xa = np.mean([x["best"].astype(np.float64) for x in A[:k]], axis=0)
        xb = np.mean([x["best"].astype(np.float64) for x in B[:k]], axis=0)
        for s in sorted(set(ref["source"].tolist())):
            sel = ref["source"] == s
            d = xa[sel] - xb[sel]
            n = int(sel.sum())
            rep["by_source"][SOURCES.get(int(s), str(int(s)))] = {"n": n, "a": float(xa[sel].mean()), "b": float(xb[sel].mean()), "delta": float(d.mean()), "se": float(d.std(ddof=1) / math.sqrt(n)) if n > 1 else None}
    return rep


def _f(x, nd: int = 4, sign: bool = False) -> str:
    if x is None:
        return "-"
    return f"{x:+.{nd}f}" if sign else f"{x:.{nd}f}"


def format_report(rep: dict) -> str:
    a = ", ".join(f"{x['name']} (step {x['step']})" for x in rep["a"])
    b = ", ".join(f"{x['name']} (step {x['step']})" for x in rep["b"])
    lines = [f"A: {a}", f"B: {b}", f"{rep['n']} dev samples, paired by index; {rep['replicates']} replicate pair(s). Delta = A - B (positive: B lower, B better).", ""]
    lines.append("| Loss | A | B | Delta | SE | t | bootstrap 95 % CI | Wilcoxon z (p) | B lower on | corr |")
    lines.append("| --- | ---: | ---: | ---: | ---: | ---: | --- | --- | ---: | ---: |")
    for m in LOSSES:
        r = rep["losses"][m]
        w = r["wilcoxon"]
        lines.append(
            f"| {LOSS_NAMES[m]} | {_f(r['mean_a'])} | {_f(r['mean_b'])} | {_f(r['delta'], sign=True)} | {_f(r['se'])} | {_f(r['t'], 1, True)} | "
            f"[{_f(r['ci95'][0], sign=True)}, {_f(r['ci95'][1], sign=True)}] | {w['z']:+.1f} ({w['p']:.2g}) | {100 * r['share_b_lower']:.1f} % | {_f(r['corr'], 3)} |"
        )
    if rep.get("distinct_paired"):
        r = rep["distinct_paired"]
        lines += ["", f"Distinct joint choices of K, per sample: A {r['mean_a']:.3f}, B {r['mean_b']:.3f}, Delta {r['delta']:+.3f} (SE {r['se']:.3f}, Wilcoxon z {r['wilcoxon']['z']:+.1f})"]
    if rep.get("discrete"):
        lines += ["", "| Discrete metric (log) | A | B | Delta (A - B) | jitter band (2 sigma) | beyond it? |", "| --- | ---: | ---: | ---: | ---: | --- |"]
        for label, r in rep["discrete"].items():
            beyond = "-" if r["beyond_band"] is None else ("yes" if r["beyond_band"] else "no")
            lines.append(f"| {label} | {_f(r['a'])} | {_f(r['b'])} | {_f(r['delta'], sign=True)} | {_f(r['band'])} | {beyond} |")
    if rep.get("by_source"):
        lines += ["", "| Source (best-of-K) | n | A | B | Delta | SE |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
        for s, r in rep["by_source"].items():
            lines.append(f"| {s} | {r['n']} | {_f(r['a'])} | {_f(r['b'])} | {_f(r['delta'], sign=True)} | {_f(r['se'])} |")
    if rep.get("seeds"):
        s = rep["seeds"]
        lines += ["", f"Seeds: Delta per seed {', '.join(f'{x:+.4f}' for x in s['delta_per_seed'])}; seed SD (pooled within arms) {_f(s['seed_sd_measured'])} "
                  f"(A means {', '.join(f'{x:.4f}' for x in s['a_means'])}; B means {', '.join(f'{x:.4f}' for x in s['b_means'])})"]
    v = rep["verdict"]
    lines += ["", f"VERDICT: {v['verdict']} (|Delta| {abs(rep['losses']['best']['delta']):.4f} against the threshold {v['threshold']:.4f} = {v['rule']}"
              + (f"; seed SD {v['seed_sd']:.4f} {v['seed_sd_source']}" if v["seed_sd"] is not None else "") + ")"]
    if v["note"]:
        lines.append(f"note: {v['note']}")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.train compare", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("a", help="run directory or per-sample .npz (comma-separated: replicate seeds)")
    ap.add_argument("b", help="the same for B")
    ap.add_argument("--step", type=int, default=None, help="the evaluation step (default: each run's newest per-sample file)")
    ap.add_argument("--seed-sd", type=float, default=None, help="the seed SD of one run's dev best-of-K, from the T1 calibration")
    ap.add_argument("--boot", type=int, default=4000, help="bootstrap resamples")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)
    rep = compare(args.a.split(","), args.b.split(","), args.step, args.seed_sd, args.boot)
    print(json.dumps(rep, indent=1) if args.json else format_report(rep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
