"""``python -m enth_ml.degrade``: degrade one WAV, print a plan, or list the pools' names."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from . import chain, ffmpeg, pools


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="python -m enth_ml.degrade",
        description="Seeded degradation chain: a clean WAV in, a degraded WAV and a JSON record out.",
    )
    p.add_argument("--in", dest="src", type=Path, help="the clean input (WAV, or anything ffmpeg reads; mono or stereo)")
    p.add_argument("--out", type=Path, help="the degraded WAV to write")
    p.add_argument("--seed", type=int, default=0, help="the seed (default 0); the same seed gives the same bytes")
    p.add_argument("--pool", choices=("train", "heldout"), default="train", help="which pool to draw from (default train)")
    p.add_argument("--profile", help="force a named subset of stages (see --list), or 'all' for every stage")
    p.add_argument("--record", type=Path, help="write the JSON record here (default: print it on stdout)")
    p.add_argument("--sample-fmt", choices=chain.SAMPLE_FORMATS, default="s16", help="the output sample format (default s16)")
    p.add_argument("--work-dir", type=Path, help="keep the intermediate float files here (default: a removed temp dir)")
    p.add_argument("--list", action="store_true", help="print the pools' stages, variants and profiles as JSON and exit")
    p.add_argument("--plan", action="store_true", help="print the plan for --seed/--pool/--profile as JSON (no audio) and exit")
    return p


def main(argv: list[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        ps = pools.load_pools()
        if args.list:
            print(json.dumps({"pools_sha256": ps.sha256, "pools": ps.names()}, indent=2))
            return 0
        if args.plan:
            print(json.dumps(chain.make_plan(ps, args.pool, args.seed, args.profile), indent=2))
            return 0
        if args.src is None or args.out is None:
            parser.error("--in and --out are required (or use --list or --plan)")
        record = chain.degrade_file(
            args.src,
            args.out,
            seed=args.seed,
            pool=args.pool,
            profile=args.profile,
            sample_fmt=args.sample_fmt,
            pools=ps,
            work_dir=args.work_dir,
        )
    except (pools.PoolError, ffmpeg.FfmpegError, OSError, ValueError) as e:
        print(f"enth_ml.degrade: error: {e}", file=sys.stderr)
        return 1
    text = json.dumps(record, indent=2) + "\n"
    if args.record is not None:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
