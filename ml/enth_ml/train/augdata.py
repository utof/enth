"""Augmented training parts (lane TR2): clean renders through lane AUG's chain, back through the product's intake.

Part ``k`` (run seed ``base + k``, so its samples are disjoint from every clean part's):

1. ``enth-datagen ds1 render --count C --seed S`` writes the clean clips and ``samples.jsonl`` (a local staging disk);
2. each clip goes through ``enth_ml.augment.augment_file`` (AUG's ``augment.toml``: purpose ``shipped-training``, DEG's
   ``train`` pool, recorded rooms and noises from the collections on disk; ffmpeg as a subprocess) with the sample's
   seed ``sample_seed(run_seed, index)``; a second clip of the same part is offered as the ``mix_in`` layer (a render of
   ours). A sample whose record is not ``shipped_ok`` is left out (a shipped-weights run trains on nothing else);
3. ``enth-datagen shards features`` runs the product's intake on each degraded clip and writes shards whose labels are
   the clean render's patch relative to the key the intake read from **that** clip (the label policy: degradation is
   damage; a room or a compressor is trained on the dry label, i.e. the model is asked for the instrument);
4. TR's ``f16`` cache of the shards becomes ``OUT/train-aug-s<seed>-f16`` (renamed into place, manifest last), with the
   augmentation records' summary in ``aug.json``; the staging copy is removed.

    python -m enth_ml.train augproduce --bin BIN --gr3 GR3 --staging DIR --out DIR --seed-base 1100 --parts 10 \
        [--count 4096] [--workers 8] [--data-dir AUG_DATA] [--stream r3/train]

``OUT/AUGPRODUCE-STOP`` stops it between parts; SIGTERM at once (a part is redone from its start).
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor

STOP_FILE = "AUGPRODUCE-STOP"

_STATE: dict = {}


def _init(data_dir: str | None) -> None:
    from ..augment.chain import augment_file  # noqa: F401 - imported once per worker
    from ..augment.config import load_config
    from ..augment.manifest import load_manifest
    from ..augment.sources import SourceStore, load_exclusions
    from ..degrade.pools import load_pools

    signal.signal(signal.SIGINT, signal.SIG_IGN)
    manifest = load_manifest()
    _STATE["config"] = load_config()
    _STATE["pools"] = load_pools()
    _STATE["manifest"] = manifest
    _STATE["store"] = SourceStore(manifest, data_root=data_dir, exclusions=load_exclusions())


def _augment_one(job: tuple[str, str, int, str | None]) -> tuple[str, dict | None, str | None, float]:
    from ..augment.chain import augment_file

    src, dst, seed, layer = job
    t0 = time.perf_counter()
    try:
        rec = augment_file(
            src, dst, seed=seed, config=_STATE["config"], pools=_STATE["pools"], manifest=_STATE["manifest"], store=_STATE["store"],
            layers=[layer] if layer else None, sample_fmt="f32",
        )
        return dst, rec, None, time.perf_counter() - t0
    except Exception as e:  # noqa: BLE001 - one clip that fails is left out and counted
        return dst, None, repr(e)[:300], time.perf_counter() - t0


def make_part(a, seed: int, out: pathlib.Path, staging: pathlib.Path, stopping) -> dict | None:
    from ..augment.plan import sample_seed
    from .produce import convert_part

    name = f"train-aug-s{seed}"
    final = out / f"{name}-f16"
    if (final / "manifest.json").exists():
        return None
    work = staging / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    rec: dict = {"seed": seed, "count": a.count, "stream": a.stream}
    t0 = time.perf_counter()
    r = subprocess.run([a.bin, "ds1", "render", "--count", str(a.count), "--seed", str(seed), "--out", str(work), "--stream", a.stream,
                        "--workers", str(a.workers), "--gr3", a.gr3], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"ds1 render: {r.stderr[-2000:]}")
    rec["render_seconds"] = time.perf_counter() - t0
    lines = [json.loads(x) for x in (work / "samples.jsonl").read_text().splitlines() if x.strip()]
    (work / "aug").mkdir()
    jobs = []
    for i, ln in enumerate(lines):
        layer = str(work / lines[(i + 1) % len(lines)]["clip"]) if len(lines) > 1 else None
        jobs.append((str(work / ln["clip"]), str(work / "aug" / f"{ln['index']:010d}.wav"), sample_seed(seed, str(ln["index"])), layer))
    t1 = time.perf_counter()
    out_lines, failed, not_shipped = [], 0, 0
    stages: collections.Counter = collections.Counter()
    secs = []
    by_dst = {j[1]: ln for j, ln in zip(jobs, lines)}
    with ProcessPoolExecutor(max_workers=a.workers, initializer=_init, initargs=(a.data_dir,)) as ex:
        for dst, record, err, dt in ex.map(_augment_one, jobs, chunksize=4):
            secs.append(dt)
            if stopping():
                ex.shutdown(wait=False, cancel_futures=True)
                return None
            if record is None:
                failed += 1
                continue
            if not record.get("shipped_ok"):
                not_shipped += 1
                continue
            for st in record.get("stages", []):
                if st.get("applied"):
                    stages[st["stage"]] += 1
            ln = by_dst[dst]
            out_lines.append(json.dumps({"stream": ln["stream"], "run_seed": ln["run_seed"], "index": ln["index"], "clip": f"aug/{ln['index']:010d}.wav", "degrade": record}))
    rec["augment_seconds"] = time.perf_counter() - t1
    rec["augment_cpu_seconds_per_clip"] = sum(secs) / max(len(secs), 1)
    rec.update(clips=len(lines), augmented=len(out_lines), failed=failed, not_shipped_ok=not_shipped, stages_ran=dict(stages))
    (work / "aug-samples.jsonl").write_text("\n".join(out_lines) + "\n")
    shards = work / "shards"
    t2 = time.perf_counter()
    r = subprocess.run([a.bin, "shards", "features", "--samples", str(work / "aug-samples.jsonl"), "--clips", str(work), "--out", str(shards),
                        "--shard-size", "256", "--workers", str(a.workers), "--gr3", a.gr3], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"shards features: {r.stderr[-2000:]}")
    rec["features_seconds"] = time.perf_counter() - t2
    m = json.loads((shards / "manifest.json").read_text())
    rec["kept"] = m["kept"]
    rec["skips"] = m.get("skips")
    tmp_final = out / f"{name}-f16.stage"
    if tmp_final.exists():
        shutil.rmtree(tmp_final)
    conv = convert_part(shards, tmp_final, 3)
    (tmp_final / "aug.json").write_text(json.dumps(rec, indent=1))
    os.replace(tmp_final, final)
    shutil.rmtree(work, ignore_errors=True)
    rec["convert_seconds"] = conv["convert_seconds"]
    rec["total_seconds"] = time.perf_counter() - t0
    return rec


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m enth_ml.train augproduce", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--gr3", required=True)
    ap.add_argument("--staging", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed-base", type=int, required=True)
    ap.add_argument("--parts", type=int, required=True)
    ap.add_argument("--first", type=int, default=1)
    ap.add_argument("--count", type=int, default=4096)
    ap.add_argument("--workers", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--data-dir", default=None, help="where AUG's collections are (default: ENTH_DATA_DIR or the manifest's local_root)")
    ap.add_argument("--stream", default="r3/train")
    a = ap.parse_args(argv)
    out, staging = pathlib.Path(a.out), pathlib.Path(a.staging)
    out.mkdir(parents=True, exist_ok=True)
    staging.mkdir(parents=True, exist_ok=True)
    flag = {"stop": False}

    def _stop(signum, frame):
        flag["stop"] = True

    signal.signal(signal.SIGTERM, _stop)
    signal.signal(signal.SIGINT, _stop)

    def stopping() -> bool:
        return flag["stop"] or (out / STOP_FILE).exists()

    log = open(out / "augproduce.jsonl", "a", encoding="utf-8")
    for k in range(a.first, a.first + a.parts):
        if stopping():
            print("stop requested", flush=True)
            break
        seed = a.seed_base + k
        print(f"aug part {seed}: {a.count} clips on {a.workers} workers", flush=True)
        rec = make_part(a, seed, out, staging, stopping)
        if rec is None:
            continue
        rec["done_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        log.write(json.dumps(rec) + "\n")
        log.flush()
        print(f"aug part {seed}: kept {rec['kept']} of {rec['clips']} clips in {rec['total_seconds']:.0f} s "
              f"(render {rec['render_seconds']:.0f}, augment {rec['augment_seconds']:.0f} = {rec['augment_cpu_seconds_per_clip']:.2f} s/clip, "
              f"features {rec['features_seconds']:.0f}); failed {rec['failed']}, not shipped_ok {rec['not_shipped_ok']}", flush=True)
    log.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
