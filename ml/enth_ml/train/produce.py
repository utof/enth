"""Rendering a training set in parts while a run trains on it (lane TR2, the full run).

``ds1`` is a pure function of ``(version, stream, run seed, index)``, so a large training set is a sequence of **parts**:
part ``k`` is ``enth-datagen ds1 gen --count C --seed base + k`` into a staging directory (a local disk), then TR's
``f16`` cache of it (``cache.py``) under ``OUT/train-s<seed>-f16``. A part is complete when its cache's
``manifest.json`` exists (written last, after a rename of the whole directory), which is what the trainer's part scan
looks for (``loop.py``, ``train_glob``). Different machines use different seed bases, so their data is disjoint.

    python -m enth_ml.train produce --bin BIN --gr3 GR3 --staging DIR --out DIR --seed-base 100 --parts 30 \
        [--count 65536] [--workers 12] [--shard-size 256] [--convert-workers 3] [--stream r3/train]

* Rendering part ``k + 1`` overlaps the conversion of part ``k`` (a thread with its own process pool).
* ``OUT/produce-workers`` (a number in a file), re-read before each part, changes the render workers while it runs
  (fewer when a training run needs CPU for its loader).
* ``OUT/PRODUCE-STOP`` stops it between parts; SIGTERM or SIGINT stops it at once (the running ``gen`` is terminated;
  ``gen`` resumes a part from its finished shards' records, so nothing rendered is lost).
* ``OUT/produce.jsonl`` gets one line per part: seed, candidates, kept, render seconds and rate, bytes.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import threading
import time

STOP_FILE = "PRODUCE-STOP"
WORKERS_FILE = "produce-workers"


def part_name(seed: int) -> str:
    return f"train-s{seed}"


def _workers(out: pathlib.Path, default: int) -> int:
    try:
        return max(1, int((out / WORKERS_FILE).read_text().strip()))
    except (OSError, ValueError):
        return default


def convert_part(staging_dir: pathlib.Path, final: pathlib.Path, workers: int) -> dict:
    """``staging_dir`` (DG's ``f32`` shards) -> ``final`` (TR's ``f16`` cache), atomically: written under a temporary
    name, renamed, then the staging copy is removed."""
    from .cache import make_cache

    tmp = final.with_name(final.name + ".tmp")
    if tmp.exists():
        shutil.rmtree(tmp)
    t0 = time.perf_counter()
    r = make_cache(staging_dir, tmp, workers)
    os.replace(tmp, final)
    shutil.rmtree(staging_dir, ignore_errors=True)
    r["convert_seconds"] = time.perf_counter() - t0
    return r


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.train produce", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", required=True, help="the enth-datagen binary")
    ap.add_argument("--gr3", required=True, help="a copy of content/gr3 (the exclusion lists and references)")
    ap.add_argument("--staging", required=True, help="where gen writes f32 shards (a fast local disk)")
    ap.add_argument("--out", required=True, help="where the f16 parts go")
    ap.add_argument("--seed-base", type=int, required=True)
    ap.add_argument("--parts", type=int, required=True)
    ap.add_argument("--first", type=int, default=1, help="the first part number (seed = base + part)")
    ap.add_argument("--count", type=int, default=65536)
    ap.add_argument("--workers", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--shard-size", type=int, default=256)
    ap.add_argument("--convert-workers", type=int, default=3)
    ap.add_argument("--stream", default="r3/train")
    a = ap.parse_args(argv)

    out = pathlib.Path(a.out)
    staging = pathlib.Path(a.staging)
    out.mkdir(parents=True, exist_ok=True)
    staging.mkdir(parents=True, exist_ok=True)
    log = open(out / "produce.jsonl", "a", encoding="utf-8")
    current: dict = {"proc": None}
    stopping = threading.Event()

    def _stop(signum, frame):
        stopping.set()
        p = current["proc"]
        if p is not None and p.poll() is None:
            p.terminate()

    signal.signal(signal.SIGTERM, _stop)
    signal.signal(signal.SIGINT, _stop)

    conv_thread: threading.Thread | None = None
    conv_err: list[BaseException] = []

    def _convert(src: pathlib.Path, final: pathlib.Path, rec: dict) -> None:
        try:
            r = convert_part(src, final, a.convert_workers)
            rec.update(r)
            rec["done_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            log.write(json.dumps(rec) + "\n")
            log.flush()
            print(f"part {rec['seed']}: converted in {r['convert_seconds']:.0f} s ({r['bytes_written'] / 1e9:.2f} GB)", flush=True)
        except BaseException as e:  # noqa: BLE001 - reported by the main thread
            conv_err.append(e)

    rc = 0
    for k in range(a.first, a.first + a.parts):
        seed = a.seed_base + k
        name = part_name(seed)
        final = out / f"{name}-f16"
        if (final / "manifest.json").exists():
            continue
        if stopping.is_set() or (out / STOP_FILE).exists():
            print("stop requested: stopping between parts", flush=True)
            break
        src = staging / name
        w = _workers(out, a.workers)
        cmd = [a.bin, "ds1", "gen", "--count", str(a.count), "--seed", str(seed), "--out", str(src), "--stream", a.stream,
               "--workers", str(w), "--shard-size", str(a.shard_size), "--gr3", a.gr3]
        t0 = time.perf_counter()
        print(f"part {seed}: rendering {a.count} candidates on {w} workers", flush=True)
        with open(staging / f"{name}.gen.log", "a", encoding="utf-8") as glog:
            current["proc"] = subprocess.Popen(cmd, stdout=glog, stderr=subprocess.STDOUT)
            ret = current["proc"].wait()
            current["proc"] = None
        secs = time.perf_counter() - t0
        if ret != 0 or not (src / "manifest.json").exists():
            print(f"part {seed}: gen exited {ret}", flush=True)
            rc = 0 if stopping.is_set() else 1
            break
        m = json.loads((src / "manifest.json").read_text())
        rec = {"seed": seed, "part": k, "stream": a.stream, "count": a.count, "kept": m["kept"], "render_seconds": secs,
               "candidates_per_s": a.count / secs, "workers": w, "skips": m.get("skips")}
        print(f"part {seed}: kept {m['kept']} of {a.count} in {secs:.0f} s ({a.count / secs:.1f} candidates/s)", flush=True)
        if conv_thread is not None:
            conv_thread.join()
        if conv_err:
            raise conv_err[0]
        conv_thread = threading.Thread(target=_convert, args=(src, final, rec))
        conv_thread.start()
    if conv_thread is not None:
        conv_thread.join()
    log.close()
    if conv_err:
        print(f"conversion failed: {conv_err[0]!r}", flush=True)
        return 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
