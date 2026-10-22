"""The merged plan: DEG's degradation plan and this package's stages, drawn from one seed.

``make_plan`` is pure in ``(config, pools, seed, profile, availability)``: it touches no audio and no disk. DEG's
streams are not touched: the degradation half of the plan is exactly what ``degrade.make_plan`` returns for the same
seed, except that a ``room`` or ``noise`` stage the recorded ``rir`` or ``bg_noise`` replaces is switched off. Each
stage of this package draws from its own stream (``derive("augment", "stage", seed, pool, stage)``), so adding a stage or
a variant never moves another stage's numbers. A variant whose source is not on disk is dropped before the draw, so a
checkout with only some collections fetched still runs (the plan lists what was dropped); naming a stage by profile when
none of its variants can run is an error.

Recorded files are chosen by fractions, not names (``u_file`` in [0, 1)): the store maps a fraction to a file of its
hashed index when the stage is applied, and the record names the file and its SHA-256.

Label policy (docs/r3.md): every stage is *damage* (the label is the clean render's patch) except the engine's own
effects, a reverb or a compressor, which are ambiguous. A sample that went through one carries ``label.policy =
"dry-and-wet"`` and lists it under ``wet_candidates``; the record adds the reverb's or compressor's measured values.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from ..degrade import chain
from ..degrade.pools import ALL_PROFILE, STAGE_ORDER, PoolError, PoolSet, draw_all
from ..degrade.rng import Rng, derive
from .config import AUG_STAGES, MERGED_ORDER, SLOTS, AugmentConfig
from .sources import GR3_SEED_RANGES, seed_in_gr3_block

PLAN_SCHEMA = "enth.augment-plan/1"
SEED_LIMIT = 1 << 63
PARTITIONS = ("train", "dev", "all")
# Variant kinds that need a second render from the caller, not a recorded source.
LAYER_KINDS = frozenset({"layer"})


class PlanError(ValueError):
    """A plan cannot be made: a forbidden seed, an unknown profile, a forced stage with nothing to run."""


def sample_seed(run_seed: int, sample_id: str, epoch: int = 0) -> int:
    """The seed of one training sample: a pure function of the run's seed, the sample's id and the epoch.

    The same sample gets a different augmentation every epoch and the same one on every re-run. The result is below 2**63
    and never inside GR3's seed block (those seeds are reserved for the held-out set; a hash that lands there is hashed
    again).
    """
    s = derive("augment/sample", run_seed, sample_id, epoch) % SEED_LIMIT
    while seed_in_gr3_block(s):
        s = derive("augment/sample-remap", s) % SEED_LIMIT
    return s


@dataclass(frozen=True)
class Availability:
    """What the plan may rely on: which recorded sources are on disk, and whether the caller supplies second renders."""

    sources: Callable[[str], bool] | None = None  # None: assume every source is there (documentation, tests)
    layers: bool = True

    def has(self, ref: str) -> bool:
        return True if self.sources is None else bool(self.sources(ref))


def _inactive_degradation(pools: PoolSet, pool_name: str, seed: int) -> dict[str, Any]:
    pool = pools.pool(pool_name)
    return {
        "pool": pool_name,
        "profile": None,
        "seed": seed,
        "clean": False,
        "forced_stage": None,
        "stages": [{"stage": s, "active": False} for s in STAGE_ORDER],
        "target_lufs": pool.target_lufs,
    }


def _usable(cfg: AugmentConfig, stage: str, av: Availability) -> tuple[list[str], list[str]]:
    """(variants of ``stage`` that can run, ``stage/variant`` names dropped for a missing source or missing layers)."""
    keep, dropped = [], []
    for name, v in cfg.stages[stage].variants.items():
        ref = v.params.get("source")
        missing = (v.kind in LAYER_KINDS and not av.layers) or (isinstance(ref, str) and not av.has(ref))
        if missing:
            dropped.append(f"{stage}/{name}")
        else:
            keep.append(name)
    return keep, dropped


def _unit(rng: Rng) -> float:
    return min(round(rng.random(), 9), 0.999999999)


def _sample_stage(cfg: AugmentConfig, stage: str, name: str, rng: Rng) -> dict[str, Any]:
    variant = cfg.stages[stage].variants[name]
    p = draw_all(variant, rng)
    if variant.kind in LAYER_KINDS:
        p["u_layer"] = _unit(rng)
    elif isinstance(variant.params.get("source"), str):
        p["u_file"], p["u_chan"], p["u_start"] = _unit(rng), _unit(rng), _unit(rng)
    return p


def make_plan(
    cfg: AugmentConfig,
    pools: PoolSet,
    seed: int,
    profile: str | None = None,
    *,
    availability: Availability | None = None,
    partition: str | None = None,
) -> dict[str, Any]:
    """Which stages run and with what parameters, for ``seed`` (optionally forced by a profile).

    ``profile`` is one of this config's profiles, a DEG profile of the pool, or ``"all"`` (every stage of both chains).
    ``partition`` is which share of each recorded collection to draw from; it defaults to ``"all"`` for an ``eval``
    config (its collections are held out as a whole) and ``"train"`` otherwise.
    """
    if not isinstance(seed, int) or isinstance(seed, bool) or not 0 <= seed < SEED_LIMIT:
        raise PlanError(f"seed must be an integer in [0, 2**63), got {seed!r}")
    if seed_in_gr3_block(seed):
        lo, hi = next((lo, hi) for lo, hi in GR3_SEED_RANGES if lo <= seed <= hi)
        raise PlanError(
            f"seed {seed} lies in GR3's reserved block {lo}..{hi}: those seeds belong to the held-out set. "
            "Use augment.sample_seed(run_seed, sample_id, epoch), which never lands there"
        )
    partition = partition or ("all" if cfg.purpose == "eval" else "train")
    if partition not in PARTITIONS:
        raise PlanError(f"partition must be one of {PARTITIONS}, got {partition!r}")
    av = availability or Availability()
    pool = pools.pool(cfg.degrade_pool)

    forced: tuple[str, ...] = ()
    random_mode = profile is None
    if profile is None:
        deg = chain.make_plan(pools, cfg.degrade_pool, seed, None)
    elif profile == ALL_PROFILE:
        deg = chain.make_plan(pools, cfg.degrade_pool, seed, ALL_PROFILE)
        forced = tuple(s for s in AUG_STAGES if s in cfg.stages)
    elif profile in cfg.profiles:
        ap = cfg.profiles[profile]
        deg = chain.make_plan(pools, cfg.degrade_pool, seed, ap.deg) if ap.deg else _inactive_degradation(
            pools, cfg.degrade_pool, seed
        )
        deg["profile"] = profile
        forced = ap.stages
    elif profile in pool.profiles:
        deg = chain.make_plan(pools, cfg.degrade_pool, seed, profile)
    else:
        raise PlanError(
            f"no profile {profile!r}: this config has {sorted(cfg.profiles)}, pool {pool.name} has "
            f"{sorted(pool.profiles)}, and {ALL_PROFILE!r} is built in"
        )

    # Every coin is drawn, whatever the config defines, so the stream never shifts.
    rng = Rng(derive("augment", "plan", seed, cfg.degrade_pool))
    coins = {s: rng.random() for s in AUG_STAGES}
    deg_active = {e["stage"]: e["active"] for e in deg["stages"]}

    dropped: list[str] = []
    usable: dict[str, list[str]] = {}
    active_aug: dict[str, bool] = {}
    replaced: dict[str, str] = {}  # DEG stage -> the AUG stage that takes its place
    for stage in AUG_STAGES:
        if stage not in cfg.stages:
            continue
        names, gone = _usable(cfg, stage, av)
        dropped += gone
        usable[stage] = names
        if random_mode:
            if deg["clean"] or coins[stage] >= cfg.stages[stage].p:
                on = False
            elif stage in SLOTS:
                on = deg_active[SLOTS[stage]]  # a share of DEG's active room / noise stages, not a new chance
            else:
                on = True
            on = on and bool(names)
            if on and stage in SLOTS:
                replaced[SLOTS[stage]] = stage
        else:
            on = stage in forced
            if on and not names:
                if profile == ALL_PROFILE:
                    on = False
                else:
                    raise PlanError(
                        f"profile {profile!r} forces stage {stage!r}, but none of its variants can run here "
                        f"(unavailable: {sorted(g for g in gone if g.startswith(stage + '/'))})"
                    )
        active_aug[stage] = on

    deg_by_stage = {}
    for e in deg["stages"]:
        deg_by_stage[e["stage"]] = {"stage": e["stage"], "active": False, "replaced_by": replaced[e["stage"]]} if (
            e["stage"] in replaced
        ) else e

    entries: list[dict[str, Any]] = []
    for stage in MERGED_ORDER:
        if stage in deg_by_stage:
            entries.append(deg_by_stage[stage])
        elif not active_aug.get(stage, False):
            entries.append({"stage": stage, "active": False})
        else:
            srng = Rng(derive("augment", "stage", seed, cfg.degrade_pool, stage))
            names = usable[stage]
            name = srng.weighted(names, [cfg.stages[stage].variants[n].weight for n in names])
            entries.append(
                {
                    "stage": stage,
                    "active": True,
                    "variant": cfg.ref(stage, name),
                    "params": _sample_stage(cfg, stage, name, srng),
                }
            )

    wet = [
        {"stage": e["stage"], "variant": e["variant"]}
        for e in entries
        if e["active"] and cfg.policy_of(e["stage"], (e.get("params") or {}).get("kind")) == "dry-and-wet"
    ]
    return {
        "schema": PLAN_SCHEMA,
        "config": {"name": cfg.name, "version": cfg.version},
        "pool": cfg.degrade_pool,
        "profile": profile,
        "seed": seed,
        "partition": partition,
        "clean": deg["clean"],
        "forced_stage": deg["forced_stage"],
        "stages": entries,
        "target_lufs": deg["target_lufs"],
        "unavailable": sorted(set(dropped)),
        "label": {
            "policy_version": cfg.label_policy_version,
            "policy": "dry-and-wet" if wet else "damage",
            "wet_candidates": wet,
        },
    }


def plan_digest(plan: dict[str, Any]) -> str:
    """A short digest of a plan (its canonical JSON), for tests and logs."""
    return hashlib.sha256(json.dumps(plan, sort_keys=True, separators=(",", ":")).encode()).hexdigest()[:16]
