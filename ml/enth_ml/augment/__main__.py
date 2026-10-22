"""``python -m enth_ml.augment``: augment one WAV, print a plan, check the config and the manifest, or index a collection."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from ..degrade import ffmpeg, load_pools
from ..degrade.chain import SAMPLE_FORMATS
from ..degrade.pools import PoolError
from . import manifest as mf
from .chain import augment_file
from .config import ConfigError, check_config, load_config
from .plan import PARTITIONS, Availability, PlanError, make_plan, sample_seed
from .sources import SourceError, SourceStore, load_exclusions


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="python -m enth_ml.augment",
        description="Seeded augmentation: DEG's degradation chain plus recorded rooms and noises, mics, levels and layers.",
    )
    p.add_argument("--in", dest="src", type=Path, help="the clean input (WAV, or anything ffmpeg reads; mono or stereo)")
    p.add_argument("--out", type=Path, help="the augmented WAV to write")
    p.add_argument("--seed", type=int, default=0, help="the seed (default 0); the same seed gives the same bytes")
    p.add_argument("--config", type=Path, help="the augmentation config (default: the packaged augment.toml)")
    p.add_argument("--profile", help="force a named subset of stages (see --list), a DEG profile, or 'all'")
    p.add_argument("--layer", type=Path, action="append", default=[], help="a second render for the mix_in stage (repeatable)")
    p.add_argument("--partition", choices=PARTITIONS, help="which share of each recorded collection to draw from")
    p.add_argument("--data-dir", type=Path, help="where the collections are (default: $ENTH_DATA_DIR or the manifest's local_root)")
    p.add_argument("--record", type=Path, help="write the JSON record here (default: print it on stdout)")
    p.add_argument("--sample-fmt", choices=SAMPLE_FORMATS, default="s16", help="the output sample format (default s16)")
    p.add_argument("--work-dir", type=Path, help="keep the intermediate float files here (default: a removed temp dir)")
    what = p.add_argument_group("other actions (no audio)")
    what.add_argument("--plan", action="store_true", help="print the plan for --seed/--profile as JSON and exit")
    what.add_argument("--list", action="store_true", help="print the config's stages, profiles and sources as JSON and exit")
    what.add_argument("--check", action="store_true", help="check the manifest and the config; exit 1 and say why if either is unsound")
    what.add_argument("--manifest", action="store_true", help="print each source's licence, allowed use and whether it is on disk")
    what.add_argument("--index", metavar="REF", help="hash a collection (e.g. openslr-28/sim-small), write its index, print the digest")
    what.add_argument("--sample-seed", nargs="+", metavar=("RUN_SEED", "SAMPLE_ID"), help="RUN_SEED SAMPLE_ID [EPOCH]: print the sample's seed")
    return p


def _store(args: argparse.Namespace, manifest: mf.Manifest) -> SourceStore:
    return SourceStore(manifest, data_root=args.data_dir, exclusions=load_exclusions())


def main(argv: list[str] | None = None) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    try:
        if args.sample_seed:
            if len(args.sample_seed) not in (2, 3):
                parser.error("--sample-seed takes RUN_SEED SAMPLE_ID [EPOCH]")
            run, sid, *rest = args.sample_seed
            print(sample_seed(int(run), sid, int(rest[0]) if rest else 0))
            return 0
        manifest = mf.load_manifest()
        if args.check:
            cfg, pools = load_config(args.config), load_pools()
            problems = [f"manifest: {m}" for m in mf.check_manifest(manifest)] + [
                f"config: {m}" for m in check_config(cfg, manifest, pools)
            ]
            for line in problems:
                print(line)
            print(f"{'FAILED' if problems else 'ok'}: {len(manifest.sources)} sources, {len(manifest.tools)} tools; config {cfg.name} ({cfg.purpose})")
            return 1 if problems else 0
        store = _store(args, manifest)
        if args.manifest:
            rows = []
            for ref in manifest.all_refs():
                rows.append(
                    {
                        "ref": ref,
                        "use": manifest.effective_use(ref),
                        "licence": manifest.licence_of(ref),
                        "role": manifest.role_of(ref),
                        "on_disk": store.available(ref),
                    }
                )
            print(json.dumps({"manifest_sha256": manifest.sha256, "sources": rows}, indent=2))
            return 0
        if args.index:
            path, digest, count = store.write_index(args.index)
            print(json.dumps({"ref": args.index, "index": str(path), "files": count, "index_sha256": digest}, indent=2))
            return 0
        cfg, pools = load_config(args.config), load_pools()
        if args.list:
            print(
                json.dumps(
                    {
                        "config": {"name": cfg.name, "version": cfg.version, "sha256": cfg.sha256, "purpose": cfg.purpose,
                                   "degrade_pool": cfg.degrade_pool},
                        "stages": {s: {"p": d.p, "variants": sorted(d.variants)} for s, d in cfg.stages.items()},
                        "profiles": {n: {"stages": list(p.stages), "deg": p.deg} for n, p in cfg.profiles.items()},
                        "sources": {ref: {"used_by": users, "on_disk": store.available(ref)} for ref, users in cfg.sources().items()},
                        "degrade_profiles": sorted(pools.pool(cfg.degrade_pool).profiles),
                    },
                    indent=2,
                )
            )
            return 0
        if args.plan:
            plan = make_plan(
                cfg, pools, args.seed, args.profile,
                availability=Availability(store.available, layers=bool(args.layer)), partition=args.partition,
            )
            print(json.dumps(plan, indent=2))
            return 0
        if args.src is None or args.out is None:
            parser.error("--in and --out are required (or use --plan, --list, --check, --manifest, --index, --sample-seed)")
        record = augment_file(
            args.src, args.out, seed=args.seed, config=cfg, pools=pools, manifest=manifest, store=store,
            layers=args.layer, profile=args.profile, partition=args.partition, sample_fmt=args.sample_fmt,
            work_dir=args.work_dir,
        )
    except (ConfigError, PlanError, PoolError, mf.ManifestError, SourceError, ffmpeg.FfmpegError, OSError, ValueError) as e:
        print(f"enth_ml.augment: error: {e}", file=sys.stderr)
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
