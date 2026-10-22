"""``python -m enth_ml.train``: train propose v0 on ds1 shards, evaluate a checkpoint, or describe a run of shards.

    python -m enth_ml.train run --config pilot.toml [--set key=value ...]   # resumes when OUT has checkpoints
    python -m enth_ml.train resume RUN_DIR [--set key=value ...]            # carry on with the run's config.json
    python -m enth_ml.train stop RUN_DIR [--wait SECONDS]                   # checkpoint and exit at the end of a step
    python -m enth_ml.train status RUN_DIR [--json]                         # step, samples, checkpoints, ETA, alive?
    python -m enth_ml.train eval --ckpt RUN/ckpt-best.pt [--mel f2] [--dev DIR ...]
    python -m enth_ml.train info DIR [DIR ...]
    python -m enth_ml.train report RUN_DIR [--every N]
    python -m enth_ml.train cache SRC_RUN_DIR OUT_DIR [--workers N]   # an f16 training cache of a ds1 run
    python -m enth_ml.train produce --bin B --gr3 G --staging S --out O --seed-base N --parts P   # render parts (produce.py)
"""

from __future__ import annotations

import argparse
import json
import os
import sys

# torch 2.14 routes a few eager ops (a batched outer product, some reductions) to Triton kernels it compiles at first
# use, and Triton needs a C compiler to build its driver stub. The training desktop has none (no sudo to install one),
# so the native JIT is off and those ops run on cuBLAS/ATen as before. Set before torch is imported.
os.environ.setdefault("TORCH_DISABLE_NATIVE_JIT", "1")


def _set(cfg_dict: dict, assignment: str) -> None:
    key, _, value = assignment.partition("=")
    try:
        v = json.loads(value)
    except json.JSONDecodeError:
        v = value
    d = cfg_dict
    parts = key.split(".")
    for p in parts[:-1]:
        d = d.setdefault(p, {})
    d[parts[-1]] = v


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.train", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="train (resumes from OUT's newest valid checkpoint when there is one)")
    r.add_argument("--config", required=True)
    r.add_argument("--resume", action="store_true", help="the default; kept for old command lines")
    r.add_argument("--no-resume", action="store_true", help="refuse a directory that already has checkpoints")
    r.add_argument("--force-config", action="store_true", help="resume although the config's model, loss or optimiser keys changed")
    r.add_argument("--set", action="append", default=[], help="override a config key (dotted), value as JSON")
    r.add_argument("--device", default=None)
    rs = sub.add_parser("resume", help="carry on with RUN_DIR/config.json")
    rs.add_argument("run")
    rs.add_argument("--set", action="append", default=[], help="override an operational key (workers, max_minutes, ...)")
    rs.add_argument("--force-config", action="store_true")
    rs.add_argument("--device", default=None)
    sp = sub.add_parser("stop", help="stop a run at the end of its step, with a checkpoint")
    sp.add_argument("run")
    sp.add_argument("--wait", default="180")
    st = sub.add_parser("status", help="how a run is doing")
    st.add_argument("run")
    st.add_argument("--json", action="store_true")
    e = sub.add_parser("eval", help="evaluate a checkpoint on dev")
    e.add_argument("--ckpt", required=True)
    e.add_argument("--mel", default="f4", choices=["f4", "f2"])
    e.add_argument("--dev", action="append", default=None)
    e.add_argument("--device", default=None)
    i = sub.add_parser("info", help="describe ds1 run directories")
    i.add_argument("dirs", nargs="+")
    rp = sub.add_parser("report", help="a run's learning curves and throughput as Markdown")
    rp.add_argument("run")
    rp.add_argument("--every", default="1")
    cp = sub.add_parser("cache", help="an f16 training cache of a ds1 run")
    cp.add_argument("src")
    cp.add_argument("out")
    cp.add_argument("--workers", default="8")
    if argv is None:
        argv = sys.argv[1:]
    if argv and argv[0] == "produce":  # its own parser (produce.py); no torch needed
        from .produce import main as produce_main

        return produce_main(argv[1:])
    args = ap.parse_args(argv)

    if args.cmd == "status":
        from .control import status_main

        return status_main([args.run] + (["--json"] if args.json else []))
    if args.cmd == "stop":
        from .control import stop_main

        return stop_main([args.run, "--wait", args.wait])
    if args.cmd == "cache":
        from .cache import main as cache_main

        return cache_main([args.src, args.out, "--workers", args.workers])
    if args.cmd == "report":
        from .report import main as report_main

        return report_main([args.run, "--every", args.every])

    if args.cmd == "info":
        from .shards import open_dataset

        ds = open_dataset(args.dirs)
        print(json.dumps(ds.describe(), indent=1))
        return 0

    from .config import TrainConfig
    from .loop import evaluate_checkpoint, train

    if args.cmd in ("run", "resume"):
        import pathlib
        import tomllib

        from .control import EXIT_STOPPED

        # SIGTERM and SIGINT are the trainer's (control.StopFlag): it finishes the step, checkpoints and exits.
        if args.cmd == "run":
            p = pathlib.Path(args.config)
            d = tomllib.loads(p.read_text()) if p.suffix == ".toml" else json.loads(p.read_text())
        else:
            d = json.loads((pathlib.Path(args.run) / "config.json").read_text())
            d["out"] = str(pathlib.Path(args.run))
        for s in args.set:
            _set(d, s)
        cfg = TrainConfig.from_dict(d)
        summary = train(cfg, resume=not getattr(args, "no_resume", False), device=args.device, allow_config_change=args.force_config)
        print(json.dumps({k: v for k, v in summary.items() if k != "last_eval"}, indent=1))
        return EXIT_STOPPED if summary["state"] == "stopped" else 0
    if args.cmd == "eval":
        res = evaluate_checkpoint(args.ckpt, args.dev, args.device, args.mel)
        print(json.dumps(res, indent=1))
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
