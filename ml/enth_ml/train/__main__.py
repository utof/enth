"""``python -m enth_ml.train``: train propose v0 on ds1 shards, evaluate a checkpoint, or describe a run of shards.

    python -m enth_ml.train run --config pilot.toml [--resume] [--set key=value ...]
    python -m enth_ml.train eval --ckpt RUN/ckpt-best.pt [--mel f2] [--dev DIR ...]
    python -m enth_ml.train info DIR [DIR ...]
"""

from __future__ import annotations

import argparse
import json
import sys


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
    r = sub.add_parser("run", help="train")
    r.add_argument("--config", required=True)
    r.add_argument("--resume", action="store_true")
    r.add_argument("--set", action="append", default=[], help="override a config key (dotted), value as JSON")
    r.add_argument("--device", default=None)
    e = sub.add_parser("eval", help="evaluate a checkpoint on dev")
    e.add_argument("--ckpt", required=True)
    e.add_argument("--mel", default="f4", choices=["f4", "f2"])
    e.add_argument("--dev", action="append", default=None)
    e.add_argument("--device", default=None)
    i = sub.add_parser("info", help="describe ds1 run directories")
    i.add_argument("dirs", nargs="+")
    args = ap.parse_args(argv)

    if args.cmd == "info":
        from .shards import open_dataset

        ds = open_dataset(args.dirs)
        print(json.dumps(ds.describe(), indent=1))
        return 0

    from .config import TrainConfig
    from .loop import evaluate_checkpoint, train

    if args.cmd == "run":
        import pathlib
        import tomllib

        p = pathlib.Path(args.config)
        d = tomllib.loads(p.read_text()) if p.suffix == ".toml" else json.loads(p.read_text())
        for s in args.set:
            _set(d, s)
        cfg = TrainConfig.from_dict(d)
        summary = train(cfg, resume=args.resume, device=args.device)
        print(json.dumps({k: v for k, v in summary.items() if k != "last_eval"}, indent=1))
        return 0
    if args.cmd == "eval":
        res = evaluate_checkpoint(args.ckpt, args.dev, args.device, args.mel)
        print(json.dumps(res, indent=1))
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
