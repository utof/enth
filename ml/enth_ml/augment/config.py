"""The augmentation config (``augment.toml``): loading, validation, and the checks that tie it to the manifest and to DEG.

Same shape as DEG's ``pools.toml`` (stages with a probability and named variants whose parameters are constants,
``[lo, hi]``, ``{ log = [...] }`` or ``{ choice = [...] }``), so the same ``draw`` and ``variants_disjoint`` code serves
both. A config is versioned (``schema``, ``version``) and its SHA-256 goes into every record.

Three purposes, three rules (``check_config``):

* ``shipped-training``: composes with DEG's ``train`` pool and may name only sources the manifest allows for shipped
  training (CC0, CC BY or Apache; never NC, SA or licence-less, never a held-out collection).
* ``research``: also the ``train`` pool, any source that is not ``excluded``; every record says ``shipped_ok = false``.
* ``eval``: composes with the ``heldout`` pool and may name only held-out collections (``role = "heldout-eval"``).
"""

from __future__ import annotations

import functools
import hashlib
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ..degrade.pools import STAGE_ORDER as DEG_STAGES
from ..degrade.pools import PoolSet, PoolError, StageDef, Variant, _check_spec, _is_number
from .manifest import LicenceError, Manifest, ManifestError, load_manifest, require_shipped

CONFIG_PATH = Path(__file__).with_name("augment.toml")
SCHEMA = "enth.augment-config/1"
PURPOSES = ("shipped-training", "research", "eval")
ALL_PROFILE = "all"

# The stages this package adds, and the one merged order of both chains (a real signal path). In the random chain `rir`
# and `bg_noise` take the place of a share (their own `p`) of DEG's active `room` and `noise` stages (SLOTS), so the
# overall chance of a room or of noise stays DEG's; in the `all` profile they run beside them.
AUG_STAGES = ("mix_in", "rir", "mic", "bg_noise", "gain", "rate")
MERGED_ORDER = (
    "mix_in", "attack_cut", "tail_trunc", "lead_pad", "room", "rir", "mic", "eq", "noise", "bg_noise", "hum",
    "dynamics", "mono", "gain", "band_limit", "codec", "bit_depth", "rate",
)
SLOTS = {"rir": "room", "bg_noise": "noise"}
assert [s for s in MERGED_ORDER if s in DEG_STAGES] == list(DEG_STAGES), "DEG's stage order must survive the merge"
assert set(MERGED_ORDER) == set(DEG_STAGES) | set(AUG_STAGES)
META_KEYS = frozenset({"weight", "doc"})


class ConfigError(ValueError):
    """``augment.toml`` (or a request against it) is invalid."""


@dataclass(frozen=True)
class AugProfile:
    name: str
    stages: tuple[str, ...]  # AUG stages forced on
    deg: str | None  # a DEG profile of the pool whose stages are forced as well


@dataclass(frozen=True)
class AugmentConfig:
    name: str
    version: int
    sha256: str
    path: Path
    purpose: str
    degrade_pool: str
    dry_and_wet: tuple[str, ...]  # "room", "rir", "dynamics/compressor": stages (or stage/kind) that get a wet candidate
    label_policy_version: int
    stages: Mapping[str, StageDef]
    profiles: Mapping[str, AugProfile]

    def policy_of(self, stage: str, kind: str | None = None) -> str:
        """``"dry-and-wet"`` for a stage the degradation policy leaves ambiguous, else ``"damage"``."""
        if stage in self.dry_and_wet or (kind is not None and f"{stage}/{kind}" in self.dry_and_wet):
            return "dry-and-wet"
        return "damage"

    def ref(self, stage: str, variant: str) -> str:
        """The name a record uses for a setting of this config: ``augment/train-v1/rir/sim-small``."""
        return f"augment/{self.name}/{stage}/{variant}"

    def sources(self) -> dict[str, list[str]]:
        """Every manifest reference the config names, with the variants that name it (``stage/variant``)."""
        out: dict[str, list[str]] = {}
        for sname, sdef in self.stages.items():
            for vname, v in sdef.variants.items():
                ref = v.params.get("source")
                if isinstance(ref, str):
                    out.setdefault(ref, []).append(f"{sname}/{vname}")
        return out


def _load_variant(where: str, name: str, table: Mapping[str, Any]) -> Variant:
    if not isinstance(table.get("kind"), str):
        raise ConfigError(f"{where}/{name}: every variant needs a string `kind`")
    params = {k: v for k, v in table.items() if k not in META_KEYS}
    try:
        for k, v in params.items():
            _check_spec(f"{where}/{name}.{k}", v)
    except PoolError as e:
        raise ConfigError(str(e)) from e
    weight = table.get("weight", 1.0)
    if not _is_number(weight) or weight <= 0:
        raise ConfigError(f"{where}/{name}: weight must be a positive number")
    return Variant(name, float(weight), params)


def parse_config(text: str, path: Path | None = None, sha256: str | None = None) -> AugmentConfig:
    """Parse config text (the tests build crafted configs this way); structure errors raise ``ConfigError``."""
    doc = tomllib.loads(text)
    if doc.get("schema") != SCHEMA:
        raise ConfigError(f"schema must be {SCHEMA!r}, got {doc.get('schema')!r}")
    cfg = doc.get("config", {})
    purpose, pool = cfg.get("purpose"), cfg.get("degrade_pool")
    if purpose not in PURPOSES:
        raise ConfigError(f"config.purpose must be one of {PURPOSES}, got {purpose!r}")
    if pool not in ("train", "heldout"):
        raise ConfigError(f"config.degrade_pool must be 'train' or 'heldout', got {pool!r}")
    if (purpose == "eval") != (pool == "heldout"):
        raise ConfigError(
            f"purpose {purpose!r} composes with the {'heldout' if purpose == 'eval' else 'train'} pool, not {pool!r}: "
            "training never draws from the held-out pool and evaluation never from the training one"
        )
    unknown = set(doc.get("stage", {})) - set(AUG_STAGES)
    if unknown:
        raise ConfigError(f"unknown stages {sorted(unknown)}; this package adds {AUG_STAGES}")
    stages: dict[str, StageDef] = {}
    for sname in AUG_STAGES:
        raw = doc.get("stage", {}).get(sname)
        if raw is None:
            continue
        p = raw.get("p")
        if not _is_number(p) or not 0.0 <= p <= 1.0:
            raise ConfigError(f"stage {sname}: p must be a number in [0, 1]")
        variants = {vn: _load_variant(f"stage.{sname}", vn, vt) for vn, vt in raw.get("variant", {}).items()}
        if not variants:
            raise ConfigError(f"stage {sname}: needs at least one variant")
        stages[sname] = StageDef(sname, float(p), variants)
    policy = doc.get("label_policy", {})
    dry_and_wet = tuple(policy.get("dry_and_wet", ()))
    for item in dry_and_wet:
        stage, _, kind = item.partition("/")
        if stage not in MERGED_ORDER:
            raise ConfigError(f"label_policy.dry_and_wet: unknown stage {item!r}")
    profiles: dict[str, AugProfile] = {}
    for pn, pt in doc.get("profile", {}).items():
        if pn == ALL_PROFILE:
            raise ConfigError(f"profile name {ALL_PROFILE!r} is built in")
        forced = tuple(pt.get("stages", ()))
        if not forced or any(s not in stages for s in forced):
            raise ConfigError(f"profile {pn}: `stages` must name stages defined here, got {forced}")
        profiles[pn] = AugProfile(pn, forced, pt.get("deg"))
    raw_bytes = text.encode("utf-8")
    return AugmentConfig(
        name=str(doc.get("name", "")),
        version=int(doc.get("version", 0)),
        sha256=sha256 or hashlib.sha256(raw_bytes).hexdigest(),
        path=path or Path("<text>"),
        purpose=purpose,
        degrade_pool=pool,
        dry_and_wet=dry_and_wet,
        label_policy_version=int(policy.get("version", 1)),
        stages=stages,
        profiles=profiles,
    )


@functools.cache
def _load_cached(path: str) -> AugmentConfig:
    raw = Path(path).read_bytes()
    return parse_config(raw.decode("utf-8"), Path(path), hashlib.sha256(raw).hexdigest())


def load_config(path: str | Path | None = None) -> AugmentConfig:
    """Load ``augment.toml`` (or another file of the same schema)."""
    return _load_cached(str(path or CONFIG_PATH))


def check_config(cfg: AugmentConfig, manifest: Manifest | None = None, pools: PoolSet | None = None) -> list[str]:
    """Every way ``cfg`` breaks the rules for its purpose; empty when it is sound.

    Checks that every named source exists in the manifest and is allowed for the purpose, that a shipped-training config
    names nothing held out, and (given the pools) that profile names reference real DEG profiles of the pool.
    """
    m = manifest or load_manifest()
    problems: list[str] = []
    if cfg.version < 1:
        problems.append("version must be 1 or more")
    if not cfg.name:
        problems.append("a config needs a name")
    for ref, users in sorted(cfg.sources().items()):
        try:
            use = m.effective_use(ref)
            role = m.role_of(ref)
        except ManifestError as e:
            problems.append(f"{', '.join(users)}: {e}")
            continue
        if cfg.purpose == "shipped-training":
            try:
                require_shipped([ref], m)  # the same gate lane TR's training uses: use, licence allowlist, not held out
            except LicenceError as e:
                problems.append(f"{', '.join(users)}: {e}")
        elif cfg.purpose == "research":
            if use == "excluded":
                problems.append(f"{', '.join(users)}: source {ref} is excluded")
            if role == "heldout-eval":
                problems.append(f"{', '.join(users)}: source {ref} is held out; only an 'eval' config may draw from it")
        else:  # eval
            if use == "excluded":
                problems.append(f"{', '.join(users)}: source {ref} is excluded")
            if role != "heldout-eval":
                problems.append(f"{', '.join(users)}: source {ref} is a training source; an 'eval' config may name held-out collections only")
    if pools is not None:
        pool = pools.pool(cfg.degrade_pool)
        for p in cfg.profiles.values():
            if p.deg is not None and p.deg not in pool.profiles:
                problems.append(f"profile {p.name}: pool {pool.name} has no DEG profile {p.deg!r}")
            if p.name in pool.profiles:
                problems.append(f"profile {p.name}: pool {pool.name} already has a DEG profile of that name")
    return problems
