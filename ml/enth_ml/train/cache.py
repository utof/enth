"""A training cache of a ds1 run with ``mel`` in ``float16``: half the bytes to read per epoch.

Lane DG's shards keep ``f32`` (the model contract). On the training desktop the shards live on a disk read through
WSL's 9P mount at about 100 MB/s when the file is not in Windows' cache, which fed the GPU only about 1,000 samples/s
(``f32``: 99 KB a sample) against the 2,000 it can train; a cache at 50 KB a sample doubles that and fits the host's file
cache. The rounding (``f16``'s spacing is 2^-7 nepers between 8 and 16, about 0.07 dB) is measured, not assumed:
``python -m enth_ml.train eval --mel f2`` scores dev on rounded features (docs/r3.md, the TR model card).

The cache is a directory with one ``.npz`` (uncompressed) per source shard (``mel`` ``f16``, ``context``, ``label``,
``mask`` ``f32``) and a ``manifest.json``: the source manifest with each shard's ``file`` pointing at its ``.npz`` and a
``tr_cache`` block (the source manifest's SHA-256, the dtype). ``read_shard`` reads either kind, so a cache is used
exactly like a run directory.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
from concurrent.futures import ProcessPoolExecutor

import numpy as np

from .shards import load_run, read_shard

CACHE_SCHEMA = "enth.tr-cache/1"


def _convert(args: tuple[str, str]) -> tuple[str, int]:
    src, dst = args
    a = read_shard(src, ("mel", "context", "label", "mask"))
    tmp = dst + ".tmp.npz"
    np.savez(tmp, mel=a["mel"].astype(np.float16), context=a["context"], label=a["label"], mask=a["mask"])
    pathlib.Path(tmp).replace(dst)
    return dst, pathlib.Path(dst).stat().st_size


def make_cache(run_dir: str | pathlib.Path, out_dir: str | pathlib.Path, workers: int = 8) -> dict:
    run = load_run(run_dir)
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    m = json.loads(json.dumps(run.manifest))
    jobs = []
    for s in m["shards"]:
        src = run.path / s["file"]
        name = pathlib.Path(s["file"]).with_suffix(".npz").name
        s["file"] = name
        s.pop("sha256", None)
        if int(s["kept"]) > 0 and not (out / name).exists():
            jobs.append((str(src), str(out / name)))
    total = 0
    with ProcessPoolExecutor(max_workers=workers) as ex:
        for _, size in ex.map(_convert, jobs, chunksize=4):
            total += size
    m["tr_cache"] = {
        "schema": CACHE_SCHEMA,
        "source_dir": run.path.name,
        "source_manifest_sha256": run.manifest_sha256,
        "mel_dtype": "<f2",
        "arrays": ["mel", "context", "label", "mask"],
    }
    (out / "manifest.json").write_text(json.dumps(m))
    return {"shards": len(m["shards"]), "converted": len(jobs), "bytes_written": total}


def main(argv: list[str]) -> int:
    import argparse
    import time

    ap = argparse.ArgumentParser(prog="python -m enth_ml.train cache")
    ap.add_argument("src")
    ap.add_argument("out")
    ap.add_argument("--workers", type=int, default=8)
    a = ap.parse_args(argv)
    t0 = time.perf_counter()
    r = make_cache(a.src, a.out, a.workers)
    r["seconds"] = time.perf_counter() - t0
    print(json.dumps(r))
    return 0
