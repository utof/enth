"""A run's learning curves and throughput as Markdown tables, from its ``metrics.jsonl`` and ``gpu.csv``.

    python -m enth_ml.train report RUN_DIR [--every N]
"""

from __future__ import annotations

import json
import pathlib
import statistics


def _fmt(x, nd: int = 3) -> str:
    if x is None:
        return "-"
    if isinstance(x, float):
        return f"{x:.{nd}f}"
    return str(x)


def load(run: pathlib.Path) -> tuple[list[dict], list[dict]]:
    recs = [json.loads(line) for line in (run / "metrics.jsonl").read_text().splitlines() if line.strip()]
    return [r for r in recs if r["kind"] == "train"], [r for r in recs if r["kind"] == "dev" and "epoch_seconds" in r]


def gpu_stats(run: pathlib.Path) -> dict:
    p = run / "gpu.csv"
    if not p.exists():
        return {}
    util, mem, power = [], [], []
    for line in p.read_text().splitlines():
        parts = [x.strip() for x in line.split(",")]
        if len(parts) < 6:
            continue
        try:
            util.append(float(parts[1]))
            mem.append(float(parts[3]))
            power.append(float(parts[4]))
        except ValueError:
            continue
    if not util:
        return {}
    return {
        "samples": len(util),
        "util_mean": statistics.mean(util),
        "util_median": statistics.median(util),
        "mem_used_max_mib": max(mem),
        "power_mean_w": statistics.mean(power),
    }


def epoch_table(train: list[dict], dev: list[dict], every: int = 1) -> str:
    rows = [
        "| Epoch | Min | Steps | Train loss | Dev best-of-K | Dev top-conf | Dev mean-K | Conf top-1 | Heads ≥2 % | Cont. median JND (best) | Samples/s | Wait % |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for i, d in enumerate(dev):
        if i % every and i != len(dev) - 1:
            continue
        tr = [t for t in train if t["epoch"] == d["epoch"]]
        loss = statistics.mean(t["loss"] for t in tr) if tr else None
        sps = statistics.mean(t["samples_per_s"] for t in tr) if tr else None
        wait = 100 * statistics.mean(t["data_wait_frac"] for t in tr) if tr else None
        rows.append(
            f"| {d['epoch']} | {d['elapsed_min']:.0f} | {d['step']} | {_fmt(loss)} | {_fmt(d['loss_best'])} | {_fmt(d['loss_top'])} | "
            f"{_fmt(d['loss_mean_k'])} | {_fmt(d['conf_top1'])} | {d['wta']['heads_winning_2pct']} | "
            f"{_fmt(d['best']['continuous_median_jnd'], 2)} | {_fmt(sps, 0)} | {_fmt(wait, 1)} |"
        )
    return "\n".join(rows)


def main(argv: list[str]) -> int:
    import argparse

    ap = argparse.ArgumentParser(prog="python -m enth_ml.train report")
    ap.add_argument("run")
    ap.add_argument("--every", type=int, default=1)
    a = ap.parse_args(argv)
    run = pathlib.Path(a.run)
    train, dev = load(run)
    print(epoch_table(train, dev, a.every))
    if train:
        steady = train[len(train) // 10 :] or train
        print()
        print(
            json.dumps(
                {
                    "steps": train[-1]["step"],
                    "samples_per_s_median": statistics.median(t["samples_per_s"] for t in steady),
                    "step_ms_median": statistics.median(t["step_ms"] for t in steady),
                    "data_wait_pct_mean": 100 * statistics.mean(t["data_wait_frac"] for t in steady),
                    "data_wait_pct_max": 100 * max(t["data_wait_frac"] for t in steady),
                    "gpu_mem_alloc_mib_max": max(t["gpu_mem_alloc_mib"] for t in train),
                    "gpu_mem_reserved_mib_max": max(t["gpu_mem_reserved_mib"] for t in train),
                    "epoch_seconds_median": statistics.median(d["epoch_seconds"] for d in dev if d.get("epoch_complete")) if any(d.get("epoch_complete") for d in dev) else None,
                    "eval_seconds_median": statistics.median(d["eval_seconds"] for d in dev) if dev else None,
                    "gpu": gpu_stats(run),
                },
                indent=1,
            )
        )
    return 0
