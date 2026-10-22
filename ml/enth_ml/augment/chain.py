"""The merged chain: a plan drawn from one seed, applied to a clean WAV, and a JSON record of exactly what was done.

``augment_file`` is ``degrade_file`` with this package's stages in the signal path (``config.MERGED_ORDER``) and the
licence rules enforced: before anything runs the config is checked against the manifest and the purpose, a recorded
file is used only if its source is allowed for the purpose, and the record lists every recorded file with its SHA-256,
licence and use, and says whether the sample may be part of a shipped training set (``shipped_ok``).

Determinism is DEG's: the same input, seed, config, pools, source files and profile give the same output bytes on one
machine with one ffmpeg build; the record names the ffmpeg, the config, the pools and the manifest by SHA-256.
"""

from __future__ import annotations

import shutil
import tempfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from ..degrade import ffmpeg, stages as deg_stages, wavio
from ..degrade.chain import SAMPLE_FORMATS, _describe
from ..degrade.pools import PoolSet, load_pools
from .config import AUG_STAGES, AugmentConfig, ConfigError, check_config, load_config
from .manifest import Manifest, load_manifest
from .plan import Availability, make_plan
from .sources import SourceStore, load_exclusions
from .stages import APPLY_AUG, AugCtx, layers_from

CHAIN_VERSION = 1
RECORD_SCHEMA = "enth.augment-record/1"


def _wet_candidate(rec: dict[str, Any]) -> dict[str, Any] | None:
    """What a trainer needs to describe the reverb or the compressor a sample went through (the 'wet' label candidate)."""
    stage = rec["stage"]
    if not rec.get("applied"):
        return None
    params, result = rec.get("params", {}), rec.get("result", {})
    if stage == "room":
        return {
            "stage": stage, "variant": rec["variant"], "recorded": False,
            "rt60_s": params["rt60_s"], "wet_db": params["wet_db"], "predelay_ms": params["predelay_ms"],
        }
    if stage == "rir":
        return {
            "stage": stage, "variant": rec["variant"], "recorded": True,
            "rt60_s": result["rt60_s"], "wet_db": result["wet_db"], "tail_s": result["tail_s"],
        }
    if stage == "dynamics" and params.get("kind") == "compressor":
        keys = ("ratio", "thr_below_peak_db", "attack_ms", "release_ms", "makeup_db")
        return {"stage": stage, "variant": rec["variant"], **{k: params[k] for k in keys}}
    return None


def augment_file(
    src: str | Path,
    dst: str | Path,
    *,
    seed: int,
    config: AugmentConfig | None = None,
    pools: PoolSet | None = None,
    manifest: Manifest | None = None,
    store: SourceStore | None = None,
    layers: Sequence[str | Path] | None = None,
    profile: str | None = None,
    partition: str | None = None,
    sample_fmt: str = "s16",
    work_dir: str | Path | None = None,
) -> dict[str, Any]:
    """Augment ``src`` into the WAV ``dst`` and return the record of what was applied.

    ``layers`` are second renders the ``mix_in`` stage may put under the clip (without them that variant is dropped from
    the plan). ``store`` says where the recorded collections are; the default reads the manifest's data root and GR3's
    exclusion lists. ``work_dir`` keeps the intermediate float files; by default they are removed.
    """
    if sample_fmt not in SAMPLE_FORMATS:
        raise ValueError(f"sample_fmt must be one of {SAMPLE_FORMATS}")
    cfg = config or load_config()
    pools = pools or load_pools()
    manifest = manifest or load_manifest()
    problems = check_config(cfg, manifest, pools)
    if problems:
        raise ConfigError(f"config {cfg.name!r} (purpose {cfg.purpose}) is not allowed:\n  " + "\n  ".join(problems))
    store = store or SourceStore(manifest, exclusions=load_exclusions())
    layer_paths = layers_from(layers)
    plan = make_plan(
        cfg, pools, seed, profile, availability=Availability(store.available, layers=bool(layer_paths)), partition=partition
    )
    version = ffmpeg.version_line()
    src, dst = Path(src), Path(dst)
    if src.resolve() == dst.resolve():
        raise ValueError("the output would overwrite the input")

    own_dir = work_dir is None
    work = Path(tempfile.mkdtemp(prefix="enth-augment-")) if own_dir else Path(work_dir)
    work.mkdir(parents=True, exist_ok=True)
    try:
        ctx = AugCtx(
            work, 0, 0, 0, work / "00-ingest.wav",
            store=store, manifest=manifest, purpose=cfg.purpose, partition=plan["partition"], layers=layer_paths,
        )
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
                fn = APPLY_AUG[entry["stage"]] if entry["stage"] in AUG_STAGES else deg_stages.APPLY[entry["stage"]]
                try:
                    result = fn(ctx, entry["params"])  # type: ignore[arg-type]
                    rec["applied"] = True
                    if result:
                        rec["result"] = result
                except deg_stages.Skip as skip:
                    rec["applied"] = False
                    rec["skipped"] = str(skip)
                rec["calls"] = ctx.calls
                rec["sha256"] = wavio.sha256_file(ctx.cur)
                rec["channels"], rec["frames"], rec["sample_rate"] = ctx.channels, ctx.frames, ctx.sr
            record_stages.append(rec)

        ctx.calls = []
        out_tmp = work / "99-final.wav"
        norm = deg_stages.normalise_and_write(ctx, plan["target_lufs"], out_tmp, sample_fmt)
        record_stages.append(
            {
                "stage": "normalise",
                "active": True,
                "params": {"target_lufs": plan["target_lufs"], "sample_fmt": sample_fmt},
                "applied": True,
                "result": norm,
                "calls": ctx.calls,
            }
        )
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(out_tmp, dst)
        output_desc = _describe(dst)
        output_desc["sample_fmt"] = sample_fmt
        used = list(ctx.used)
    finally:
        if own_dir:
            shutil.rmtree(work, ignore_errors=True)

    wet = [c for c in (_wet_candidate(r) for r in record_stages if r["stage"] != "ingest") if c is not None]
    return {
        "schema": RECORD_SCHEMA,
        "chain_version": CHAIN_VERSION,
        "purpose": cfg.purpose,
        "pool": cfg.degrade_pool,
        "profile": profile,
        "seed": seed,
        "partition": plan["partition"],
        "clean": plan["clean"],
        "forced_stage": plan["forced_stage"],
        "unavailable": plan["unavailable"],
        "config": {"name": cfg.name, "version": cfg.version, "file": cfg.path.name, "sha256": cfg.sha256},
        "pools": {"file": pools.path.name, "version": pools.version, "sha256": pools.sha256},
        "manifest": {"version": manifest.version, "sha256": manifest.sha256},
        "exclusions_sha256": store.exclusions.sha256 if store.exclusions is not None else None,
        "ffmpeg": version,
        "input": input_desc,
        "layers_supplied": len(layer_paths),
        "stages": record_stages,
        "sources_used": used,
        # True when nothing but the clip itself went in, or only sources the manifest allows for shipped training.
        # The clip's own licence is the caller's: a render of ours is fine, a real recording needs its own check.
        "shipped_ok": cfg.purpose != "eval" and all(u["use"] == "shipped-training" for u in used),
        "label": {
            "policy": "dry-and-wet" if wet else "damage",
            "policy_version": plan["label"]["policy_version"],
            "dry": "the patch of the clean input render",
            "wet_candidates": wet,
        },
        "output": output_desc,
    }
