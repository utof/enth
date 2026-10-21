"""The chain: a plan drawn from a seed, then applied to a clean WAV, with a JSON record of exactly what was done.

``make_plan`` is a pure function of ``(pools.toml, pool, profile, seed)``: it needs no audio, so a manifest can store
the plan's inputs and anyone can rebuild it. ``degrade_file`` applies a plan to a file and returns the record.

Determinism: the same input, seed, pool and profile give the same output bytes on one machine with one ffmpeg build.
Parameters come from our own SplitMix64 streams (stable everywhere); the bytes of the audio also depend on ffmpeg's
floating-point code (its FFT and resamplers pick SIMD paths by CPU, and encoder versions change), so another ffmpeg
build or CPU may change them. Every record names the ffmpeg it ran with.
"""

from __future__ import annotations

import shutil
import tempfile
from pathlib import Path
from typing import Any

from . import ffmpeg, stages, wavio
from .pools import ALL_PROFILE, STAGE_ORDER, PoolError, PoolSet, load_pools
from .rng import Rng, derive, instance_seed

CHAIN_VERSION = 1
RECORD_SCHEMA = "enth.degrade-record/1"
SAMPLE_FORMATS = ("s16", "s24", "f32")


def make_plan(
    pools: PoolSet, pool_name: str, seed: int, profile: str | None = None
) -> dict[str, Any]:
    """Which stages run and with what parameters, for ``seed`` in ``pool_name`` (optionally forced by a profile)."""
    pool = pools.pool(pool_name)
    restrict: dict[str, tuple[str, ...]] = {}
    forced = None
    clean = False
    if profile == ALL_PROFILE:
        active = set(STAGE_ORDER)
    elif profile is not None:
        if profile not in pool.profiles:
            raise PoolError(f"pool {pool_name} has no profile {profile!r}; it has {sorted(pool.profiles)} and {ALL_PROFILE!r}")
        active = set(pool.profiles[profile].stages)
        restrict = dict(pool.profiles[profile].variants)
    else:
        rng = Rng(derive("plan", seed, pool_name))
        clean = rng.chance(pool.p_clean)
        coins = [rng.random() < pool.stages[s].p for s in STAGE_ORDER]  # always drawn: the stream never shifts
        active = set() if clean else {s for s, c in zip(STAGE_ORDER, coins, strict=True) if c}
        if not clean and not active:
            candidates = [s for s in STAGE_ORDER if pool.stages[s].p > 0.0]
            forced = rng.choice(candidates)
            active = {forced}

    entries: list[dict[str, Any]] = []
    for stage in STAGE_ORDER:
        if stage not in active:
            entries.append({"stage": stage, "active": False})
            continue
        sdef = pool.stages[stage]
        names = list(restrict.get(stage, sdef.variants))
        rng = Rng(derive("stage", seed, pool_name, stage))
        name = rng.weighted(names, [sdef.variants[n].weight for n in names])
        params = stages.sample_stage(
            stage,
            sdef.variants[name],
            rng,
            lambda label, stage=stage: instance_seed(seed, pool_name, pool.instance_parity, stage, label),
        )
        entries.append({"stage": stage, "active": True, "variant": pool.ref(stage, name), "params": params})
    return {
        "pool": pool_name,
        "profile": profile,
        "seed": seed,
        "clean": clean,
        "forced_stage": forced,
        "stages": entries,
        "target_lufs": pool.target_lufs,
    }


def _describe(path: Path) -> dict[str, Any]:
    info = wavio.read_info(path)
    return {
        "sha256": wavio.sha256_file(path),
        "sample_rate": info.sample_rate,
        "channels": info.channels,
        "frames": info.frames,
        "duration_s": round(info.duration_s, 6),
    }


def degrade_file(
    src: str | Path,
    dst: str | Path,
    *,
    seed: int,
    pool: str = "train",
    profile: str | None = None,
    sample_fmt: str = "s16",
    pools: PoolSet | None = None,
    work_dir: str | Path | None = None,
) -> dict[str, Any]:
    """Degrade ``src`` into the WAV ``dst`` and return the record of what was applied.

    ``work_dir`` keeps the intermediate float files (one per ffmpeg step) for inspection; by default they are removed.
    """
    if sample_fmt not in SAMPLE_FORMATS:
        raise ValueError(f"sample_fmt must be one of {SAMPLE_FORMATS}")
    pools = pools or load_pools()
    plan = make_plan(pools, pool, seed, profile)
    version = ffmpeg.version_line()
    src, dst = Path(src), Path(dst)

    own_dir = work_dir is None
    work = Path(tempfile.mkdtemp(prefix="enth-degrade-")) if own_dir else Path(work_dir)
    work.mkdir(parents=True, exist_ok=True)
    try:
        ctx = stages.Ctx(work, 0, 0, 0, work / "00-ingest.wav")
        # Ingest: the first audio stream as 32-bit float, whatever the input format (WAV, FLAC, MP3...).
        ctx.call(
            ["-i", "{in}", "-map", "0:a:0", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
            {"in": str(src), "out": str(ctx.cur)},
        )
        info = wavio.read_info(ctx.cur)
        if info.channels > 2:
            raise ffmpeg.FfmpegError(f"{src}: {info.channels} channels; the chain takes mono or stereo")
        if info.frames < 1:
            raise ffmpeg.FfmpegError(f"{src}: no samples")
        ctx.sr, ctx.channels, ctx.frames = info.sample_rate, info.channels, info.frames
        record_stages: list[dict[str, Any]] = [
            {"stage": "ingest", "calls": ctx.calls, "sha256": wavio.sha256_file(ctx.cur)}
        ]
        input_desc = {
            "sha256": wavio.sha256_file(src),
            "sample_rate": info.sample_rate,
            "channels": info.channels,
            "frames": info.frames,
            "duration_s": round(info.duration_s, 6),
        }

        for entry in plan["stages"]:
            rec = dict(entry)
            if entry["active"]:
                ctx.calls = []
                try:
                    result = stages.APPLY[entry["stage"]](ctx, entry["params"])
                    rec["applied"] = True
                    if result:
                        rec["result"] = result
                except stages.Skip as skip:
                    rec["applied"] = False
                    rec["skipped"] = str(skip)
                rec["calls"] = ctx.calls
                rec["sha256"] = wavio.sha256_file(ctx.cur)
                rec["channels"], rec["frames"] = ctx.channels, ctx.frames
            record_stages.append(rec)

        ctx.calls = []
        out_tmp = work / "99-final.wav"
        norm = stages.normalise_and_write(ctx, plan["target_lufs"], out_tmp, sample_fmt)
        norm_rec = {"stage": "normalise", "active": True, "params": {"target_lufs": plan["target_lufs"], "sample_fmt": sample_fmt},
                    "applied": True, "result": norm, "calls": ctx.calls}
        record_stages.append(norm_rec)
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(out_tmp, dst)
        output_desc = _describe(dst)
        output_desc["sample_fmt"] = sample_fmt
    finally:
        if own_dir:
            shutil.rmtree(work, ignore_errors=True)

    return {
        "schema": RECORD_SCHEMA,
        "chain_version": CHAIN_VERSION,
        "pool": pool,
        "profile": profile,
        "seed": seed,
        "clean": plan["clean"],
        "forced_stage": plan["forced_stage"],
        "pools": {"file": pools.path.name, "version": pools.version, "sha256": pools.sha256},
        "ffmpeg": version,
        "input": input_desc,
        "stages": record_stages,
        "output": output_desc,
    }
