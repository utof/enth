"""``python -m enth_ml.train smoke --config C --dev DIR``: tier T0 of the fast loop, go or no-go in minutes (lane FTL;
ml/README.md, "Fast loop"). It runs the config's own network and loss, on a few real dev samples, and checks four
things:

1. **Budgets:** the model contract's 2²² parameters and 2³¹ FLOPs per inference (counted as ``pack.rs`` counts them:
   ``net.exported_parameter_count`` and ``net.count_flops``), and op set 1 (``net.audit_ops``).
2. **It learns:** it overfits ``--samples`` dev samples for ``--steps`` steps (the config's LR after a short warmup,
   the WTA temperature annealed over the config's share of the smoke run) and requires the dev best-of-K on those
   samples to fall by at least 70 %, every loss to stay finite, and every one of the K heads to win at least once.
3. **It exports:** ``export/propose.py``'s path (BatchNorm folded, every tensor rounded to the stored dtype, the graph
   written), and the file read back and compared with the model exactly (``check_round_trip``).
4. **Our kernels agree** (with ``--model-check BIN``): ``enth-model-check run --tolerance 1e-4 --decode`` on 8 of the
   samples against PyTorch's outputs.

It writes ``smoke.json`` (each check, its numbers, the seconds per phase) to ``--out`` and exits 0 when every check
passes, 1 otherwise. Defaults: 256 samples, 300 steps in batches of 256 on a GPU; on a CPU 64 samples, 100 steps in
batches of 64, which fits the 5-minute budget on the laptop (measured: ml/README.md).
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

os.environ.setdefault("TORCH_DISABLE_NATIVE_JIT", "1")

SMOKE_SCHEMA = "enth.train-smoke/1"
MAX_PARAMETERS = 1 << 22
MAX_FLOPS = 1 << 31
BUDGET_S = 300.0
MIN_DROP = 0.70
PARITY_INPUTS = 8
PARITY_TOLERANCE = 1e-4


def _defaults(device: str) -> dict:
    return {"samples": 256, "steps": 300, "batch": 256} if device.startswith("cuda") else {"samples": 64, "steps": 100, "batch": 64}


def smoke(config: str, dev_dirs: list[str], out: str | pathlib.Path, device: str | None = None, samples: int | None = None, steps: int | None = None, batch: int | None = None, model_check: str | None = None, seed: int | None = None, log=print) -> dict:
    import numpy as np
    import torch

    from ..export import propose as P
    from ..model.net import ProposeV0, audit_ops, count_flops, exported_parameter_count
    from . import schedule as S
    from .config import TrainConfig
    from .data import DevSet, compute_standardisation
    from .loop import _param_groups, environment
    from .loss import ProposeLoss
    from .shards import open_dataset

    t_all = time.perf_counter()
    phases: dict[str, float] = {}
    checks: dict[str, dict] = {}
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    dev_name = device or ("cuda" if torch.cuda.is_available() else "cpu")
    device_t = torch.device(dev_name)
    d = _defaults(dev_name)
    samples = samples or d["samples"]
    steps = steps or d["steps"]
    batch = min(batch or d["batch"], samples)
    cfg = TrainConfig.load(config)
    S.check(cfg)
    seed = cfg.seed if seed is None else seed
    torch.manual_seed(seed)
    np.random.seed(seed % (2**32))

    # 1. Budgets.
    t0 = time.perf_counter()
    model = ProposeV0(cfg.model)
    params, flops, ops = exported_parameter_count(model), count_flops(model), audit_ops(model)
    largest = max(p.numel() for p in model.parameters())
    checks["budgets"] = {
        "pass": params <= MAX_PARAMETERS and flops <= MAX_FLOPS and largest <= MAX_PARAMETERS and not ops,
        "parameters": params,
        "max_parameters": MAX_PARAMETERS,
        "largest_tensor": largest,
        "flops": flops,
        "max_flops": MAX_FLOPS,
        "op_set_problems": ops,
    }
    log(f"budgets: {params:,} parameters (cap {MAX_PARAMETERS:,}), {flops:,} FLOPs (cap {MAX_FLOPS:,}){'; ' + '; '.join(ops) if ops else ''}")

    # 2. Overfit a few real dev samples.
    ds = open_dataset(dev_dirs, "dev")
    dev = DevSet(ds.shards, samples)
    n = dev.n
    covering, total = [], 0
    for s in ds.shards:
        covering.append(s)
        total += s[1]
        if total >= n:
            break
    stats = compute_standardisation(covering, len(covering), seed)
    model.set_standardisation(*(torch.as_tensor(stats[k]) for k in ("mel_mean", "mel_std", "ctx_mean", "ctx_std")))
    model.to(device_t)
    loss_fn = ProposeLoss(ds.jnd_table, ds.jnd_scale, cfg.loss).to(device_t)
    opt = torch.optim.AdamW(_param_groups(model, S.weight_decay(cfg)), lr=cfg.lr, betas=(0.9, cfg.beta2))
    use_amp = cfg.amp and device_t.type == "cuda"
    scaler = torch.amp.GradScaler("cuda", enabled=use_amp)
    arr = {k: torch.from_numpy(np.ascontiguousarray(v)).to(device_t) for k, v in dev.arrays.items()}
    k_heads = cfg.model.hypotheses
    phases["setup_s"] = time.perf_counter() - t0

    @torch.no_grad()
    def dev_best() -> tuple[float, list[int]]:
        model.eval()
        loss_fn.eval()
        best, wins = [], torch.zeros(k_heads, dtype=torch.long, device=device_t)
        for i in range(0, n, 256):
            with torch.autocast(device_type=device_t.type, dtype=torch.float16, enabled=use_amp):
                hyp, _ = model(arr["mel"][i : i + 256].float(), arr["context"][i : i + 256])
            losses, _ = loss_fn.hypothesis_losses(hyp.float(), arr["label"][i : i + 256], arr["mask"][i : i + 256])
            b, w = losses.min(1)
            best.append(b)
            wins += torch.bincount(w, minlength=k_heads)
        model.train()
        loss_fn.train()
        return float(torch.cat(best).mean()), wins.tolist()

    t0 = time.perf_counter()
    initial, initial_wins = dev_best()
    warm = max(1, steps // 20)
    train_wins = torch.zeros(k_heads, device=device_t)
    finite = True
    first_bad = None
    curve = []
    for s in range(steps):
        idx = torch.arange(s * batch, (s + 1) * batch, device=device_t) % n
        for g in opt.param_groups:
            g["lr"] = cfg.lr * min(1.0, (s + 1) / warm)
        temp = cfg.loss.temperature(s / steps)
        with torch.autocast(device_type=device_t.type, dtype=torch.float16, enabled=use_amp):
            hyp, conf = model(arr["mel"][idx].float(), arr["context"][idx])
        r = loss_fn(hyp, conf, arr["label"][idx], arr["mask"][idx], temperature=temp)
        opt.zero_grad(set_to_none=True)
        scaler.scale(r["total"]).backward()
        scaler.unscale_(opt)
        torch.nn.utils.clip_grad_norm_(model.parameters(), cfg.grad_clip)
        scaler.step(opt)
        scaler.update()
        train_wins += torch.bincount(r["winner"], minlength=k_heads).float()
        total_loss = float(r["total"].detach())
        if not np.isfinite(total_loss) and first_bad is None:
            finite, first_bad = False, s
        if (s + 1) % max(1, steps // 10) == 0 or s + 1 == steps:
            curve.append({"step": s + 1, "train_loss": total_loss, "train_best": float(r["best"]), "seconds": time.perf_counter() - t0})
            log(f"step {s + 1}/{steps}: loss {total_loss:.4f} best {float(r['best']):.4f} ({time.perf_counter() - t0:.0f} s)")
    final, final_wins = dev_best()
    phases["overfit_s"] = time.perf_counter() - t0
    drop = 1.0 - final / initial if initial > 0 else 0.0
    finite = finite and bool(np.isfinite(final)) and all(bool(torch.isfinite(p).all()) for p in model.parameters())
    tw = train_wins.long().tolist()
    checks["overfit"] = {
        "pass": finite and drop >= MIN_DROP and min(tw) >= 1,
        "samples": n,
        "steps": steps,
        "batch": batch,
        "best_of_k_initial": initial,
        "best_of_k_final": final,
        "drop": drop,
        "min_drop": MIN_DROP,
        "finite": finite,
        "first_non_finite_step": first_bad,
        "train_wins_per_head": tw,
        "dev_wins_per_head_initial": initial_wins,
        "dev_wins_per_head_final": final_wins,
        "samples_per_s": steps * batch / max(phases["overfit_s"], 1e-9),
        "curve": curve,
    }
    log(f"overfit: best-of-{k_heads} {initial:.4f} -> {final:.4f} ({100 * drop:.0f} % down; need {100 * MIN_DROP:.0f} %), finite {finite}, heads winning {sum(1 for w in tw if w)}/{k_heads}")

    # 3. Export and read back.
    t0 = time.perf_counter()
    model_path = out / "smoke-model.bin"
    try:
        stored = P.as_stored(model.cpu())
        data = P.build_writer(stored, f"smoke-{pathlib.Path(config).stem}", info=[("smoke.config", pathlib.Path(config).name)]).build()
        tensors_compared = P.check_round_trip(data, stored)
        model_path.write_bytes(data)
        checks["export"] = {"pass": True, "bytes": len(data), "tensors_compared": tensors_compared, "file": model_path.name}
        log(f"export: {len(data):,} bytes, {tensors_compared} tensors read back exactly")
    except Exception as err:  # noqa: BLE001 - a failed export is the check's result
        stored = None
        checks["export"] = {"pass": False, "error": repr(err)[:500]}
        log(f"export: FAILED {err!r}")
    phases["export_s"] = time.perf_counter() - t0

    # 4. Our kernels against PyTorch.
    if model_check:
        t0 = time.perf_counter()
        checks["parity"] = _parity(model_check, model_path, stored, dev, out, log) if stored is not None else {"pass": False, "error": "no model file (the export failed)"}
        phases["parity_s"] = time.perf_counter() - t0
    else:
        checks["parity"] = {"pass": None, "skipped": "no --model-check BIN"}

    total_s = time.perf_counter() - t_all
    ok = all(c["pass"] is not False for c in checks.values())
    env = environment()
    result = {
        "schema": SMOKE_SCHEMA,
        "config": pathlib.Path(config).name,
        "device": str(device_t),
        "threads": torch.get_num_threads(),
        "seed": seed,
        "pass": ok,
        "seconds": total_s,
        "budget_s": BUDGET_S,
        "within_budget": total_s <= BUDGET_S,
        "phases": phases,
        "checks": checks,
        "environment": {k: env.get(k) for k in ("python", "torch", "numpy", "cuda", "cpus", "gpu", "commit", "dirty", "uv_lock_sha256")},
    }
    (out / "smoke.json").write_text(json.dumps(result, indent=1))
    log(f"smoke: {'PASS' if ok else 'FAIL'} in {total_s:.0f} s (budget {BUDGET_S:.0f} s{'' if total_s <= BUDGET_S else ', OVER'}) -> {out / 'smoke.json'}")
    return result


def _parity(bin_path: str, model_path: pathlib.Path, stored, dev, out: pathlib.Path, log) -> dict:
    """``enth-model-check run`` on ``PARITY_INPUTS`` of the samples against PyTorch's outputs of the stored model."""
    import numpy as np

    from ..export import propose as P
    from ..export import tensors

    k = min(PARITY_INPUTS, dev.n)
    pick = np.unique(np.round(np.linspace(0, dev.n - 1, k)).astype(np.int64))
    mel = dev.arrays["mel"][pick].astype(np.float32)
    ctx = dev.arrays["context"][pick].astype(np.float32)
    hyp, conf = P.run_reference(stored, mel, ctx)
    inputs, expected = out / "smoke-inputs.tensors", out / "smoke-expected.tensors"
    inputs.write_bytes(tensors.write_arrays([("mel", mel), ("context", ctx)]))
    expected.write_bytes(tensors.write_arrays([("hypotheses", hyp), ("confidence", conf)]))
    cmd = [bin_path, "run", str(model_path), "--inputs", str(inputs), "--expected", str(expected), "--tolerance", str(PARITY_TOLERANCE), "--decode"]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    except (OSError, subprocess.TimeoutExpired) as err:
        log(f"parity: could not run {bin_path}: {err!r}")
        return {"pass": False, "error": repr(err)[:300]}
    tail = (r.stdout + r.stderr).strip().splitlines()[-12:]
    log(f"parity: enth-model-check exit {r.returncode}" + ("" if r.returncode == 0 else "\n  " + "\n  ".join(tail)))
    return {"pass": r.returncode == 0, "exit": r.returncode, "inputs": int(len(pick)), "tolerance": PARITY_TOLERANCE, "output_tail": tail}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.train smoke", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", required=True)
    ap.add_argument("--dev", action="append", required=True, help="a ds1 dev run directory (r3/dev)")
    ap.add_argument("--out", default=None, help="where smoke.json and the model file go (default: a new temporary directory)")
    ap.add_argument("--device", default=None)
    ap.add_argument("--samples", type=int, default=None)
    ap.add_argument("--steps", type=int, default=None)
    ap.add_argument("--batch", type=int, default=None)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--model-check", default=None, help="an enth-model-check binary: also check our kernels against PyTorch")
    args = ap.parse_args(argv)
    out = args.out or tempfile.mkdtemp(prefix="enth-smoke-")
    res = smoke(args.config, args.dev, out, args.device, args.samples, args.steps, args.batch, args.model_check, args.seed)
    return 0 if res["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
