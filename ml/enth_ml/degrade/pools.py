"""The pool definitions (``pools.toml``): loading, validation, drawing, and the disjointness check.

A *pool* (``train``, ``heldout``) holds, per stage, a probability and named *variants*; a variant is a table of
parameters, each a constant, a ``[lo, hi]`` range, ``{ log = [lo, hi] }`` or ``{ choice = [...] }``. Everything here is
plain data in, plain data out, so lane DS's manifests can name a setting (``heldout/codec/shine-cbr``) and a test can
prove the pools never meet.
"""

from __future__ import annotations

import functools
import hashlib
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .rng import Rng, sig6

POOLS_PATH = Path(__file__).with_name("pools.toml")
SCHEMA = "enth.degrade-pools/1"

# The chain's order, which is a real signal path (docs/r3.md, "Augmentation and licences"). Normalisation follows.
STAGE_ORDER = (
    "attack_cut",
    "tail_trunc",
    "lead_pad",
    "room",
    "eq",
    "noise",
    "hum",
    "dynamics",
    "mono",
    "band_limit",
    "codec",
    "bit_depth",
)
# Keys of a variant that are not parameters.
META_KEYS = frozenset({"weight", "doc"})
ALL_PROFILE = "all"  # built in: every stage on, no restriction on variants


class PoolError(ValueError):
    """``pools.toml`` (or a request against it) is invalid."""


@dataclass(frozen=True)
class Variant:
    name: str
    weight: float
    params: Mapping[str, Any]  # raw parameter specs, META_KEYS removed

    @property
    def kind(self) -> str:
        return str(self.params["kind"])


@dataclass(frozen=True)
class StageDef:
    name: str
    p: float
    variants: Mapping[str, Variant]


@dataclass(frozen=True)
class Profile:
    name: str
    stages: tuple[str, ...]
    variants: Mapping[str, tuple[str, ...]]  # stage -> allowed variant names (absent = all)


@dataclass(frozen=True)
class Pool:
    name: str
    description: str
    instance_parity: int
    p_clean: float
    target_lufs: float
    stages: Mapping[str, StageDef]
    profiles: Mapping[str, Profile]

    def ref(self, stage: str, variant: str) -> str:
        """The name other files use for a setting: ``heldout/codec/shine-cbr``."""
        return f"{self.name}/{stage}/{variant}"


@dataclass(frozen=True)
class PoolSet:
    version: int
    pools: Mapping[str, Pool]
    sha256: str
    path: Path

    def pool(self, name: str) -> Pool:
        try:
            return self.pools[name]
        except KeyError:
            raise PoolError(f"unknown pool {name!r}; the pools are {sorted(self.pools)}") from None

    def names(self) -> dict[str, Any]:
        """Every name a manifest may reference, as plain data (``--list`` prints it)."""
        return {
            name: {
                "stages": {s: {"p": sd.p, "variants": sorted(sd.variants)} for s, sd in pool.stages.items()},
                "profiles": {p: {"stages": list(pr.stages), "variants": {k: list(v) for k, v in pr.variants.items()}}
                             for p, pr in pool.profiles.items()},
            }
            for name, pool in self.pools.items()
        }


# --- parameter domains --------------------------------------------------------------------------------------------


def _is_number(x: Any) -> bool:
    return isinstance(x, (int, float)) and not isinstance(x, bool)


def _check_spec(where: str, spec: Any) -> None:
    if isinstance(spec, (str, bool)) or _is_number(spec):
        return
    if isinstance(spec, list):
        if len(spec) == 2 and all(_is_number(v) for v in spec) and spec[0] <= spec[1]:
            return
        raise PoolError(f"{where}: a list must be [lo, hi] with lo <= hi, got {spec!r}")
    if isinstance(spec, dict) and len(spec) == 1:
        (key, val), = spec.items()
        if key == "log":
            if isinstance(val, list) and len(val) == 2 and all(_is_number(v) for v in val) and 0 < val[0] <= val[1]:
                return
            raise PoolError(f"{where}: log needs [lo, hi] with 0 < lo <= hi, got {val!r}")
        if key == "choice":
            if isinstance(val, list) and val:
                return
            raise PoolError(f"{where}: choice needs a non-empty list, got {val!r}")
    raise PoolError(f"{where}: cannot read parameter {spec!r}")


def draw(spec: Any, rng: Rng) -> Any:
    """One value from a parameter spec. Floats are rounded to 6 significant digits."""
    if isinstance(spec, list):
        lo, hi = spec
        if isinstance(lo, int) and isinstance(hi, int):
            return rng.randint(lo, hi)
        return sig6(rng.uniform(lo, hi))
    if isinstance(spec, dict):
        if "log" in spec:
            lo, hi = spec["log"]
            return sig6(rng.log_uniform(lo, hi))
        return rng.choice(spec["choice"])
    return spec


def draw_all(variant: Variant, rng: Rng) -> dict[str, Any]:
    """Every parameter of a variant, drawn in sorted key order (so the file's key order never matters)."""
    return {k: draw(variant.params[k], rng) for k in sorted(variant.params)}


def domain(spec: Any) -> tuple[str, Any, Any]:
    """The set of values a spec can produce: ``("range", lo, hi)`` or ``("set", frozenset, None)``."""
    if isinstance(spec, list):
        return ("range", spec[0], spec[1])
    if isinstance(spec, dict):
        if "log" in spec:
            return ("range", spec["log"][0], spec["log"][1])
        return ("set", frozenset(spec["choice"]), None)
    if _is_number(spec):
        return ("range", spec, spec)
    return ("set", frozenset([spec]), None)


def domains_meet(a: tuple[str, Any, Any], b: tuple[str, Any, Any]) -> bool:
    """Do two parameter domains share a value?"""
    if a[0] == "set" and b[0] == "set":
        return bool(a[1] & b[1])
    if a[0] == "range" and b[0] == "range":
        return max(a[1], b[1]) <= min(a[2], b[2])
    s, r = (a, b) if a[0] == "set" else (b, a)
    return any(_is_number(v) and r[1] <= v <= r[2] for v in s[1])


def variants_disjoint(a: Variant, b: Variant) -> bool:
    """True when some parameter both variants define has non-meeting domains.

    A parameter only one variant defines does not separate them, which is why every variant carries a ``kind``: two
    variants of different kinds are always disjoint, and two of the same kind must differ in a shared range or set.
    """
    shared = set(a.params) & set(b.params)
    return any(not domains_meet(domain(a.params[k]), domain(b.params[k])) for k in sorted(shared))


def check_disjoint(a: Pool, b: Pool) -> list[str]:
    """Every way ``a`` and ``b`` fail to be disjoint; empty when they are."""
    problems: list[str] = []
    if a.instance_parity == b.instance_parity:
        problems.append(f"{a.name} and {b.name} share instance_parity {a.instance_parity}: instances can collide")
    for stage in STAGE_ORDER:
        sa, sb = a.stages[stage], b.stages[stage]
        for na, va in sa.variants.items():
            for nb, vb in sb.variants.items():
                if na == nb:
                    problems.append(f"{stage}: variant name {na!r} is in both pools")
                if not variants_disjoint(va, vb):
                    problems.append(
                        f"{stage}: {a.name}/{na} and {b.name}/{nb} (kind {va.kind!r} / {vb.kind!r}) "
                        "overlap in every shared parameter"
                    )
    return problems


# --- loading ------------------------------------------------------------------------------------------------------


def _load_variant(where: str, name: str, table: Mapping[str, Any]) -> Variant:
    if "kind" not in table or not isinstance(table["kind"], str):
        raise PoolError(f"{where}/{name}: every variant needs a string `kind`")
    params = {k: v for k, v in table.items() if k not in META_KEYS}
    for k, v in params.items():
        _check_spec(f"{where}/{name}.{k}", v)
    weight = table.get("weight", 1.0)
    if not _is_number(weight) or weight <= 0:
        raise PoolError(f"{where}/{name}: weight must be a positive number")
    return Variant(name, float(weight), params)


def _load_pool(name: str, table: Mapping[str, Any]) -> Pool:
    stages: dict[str, StageDef] = {}
    raw_stages = table.get("stage", {})
    unknown = set(raw_stages) - set(STAGE_ORDER)
    if unknown:
        raise PoolError(f"pool {name}: unknown stages {sorted(unknown)}")
    for stage in STAGE_ORDER:
        if stage not in raw_stages:
            raise PoolError(f"pool {name}: stage {stage!r} is not defined")
        raw = raw_stages[stage]
        p = raw.get("p")
        if not _is_number(p) or not 0.0 <= p <= 1.0:
            raise PoolError(f"pool {name}/{stage}: p must be a number in [0, 1]")
        variants = {
            vn: _load_variant(f"{name}/{stage}", vn, vt) for vn, vt in raw.get("variant", {}).items()
        }
        if not variants:
            raise PoolError(f"pool {name}/{stage}: needs at least one variant")
        stages[stage] = StageDef(stage, float(p), variants)
    profiles: dict[str, Profile] = {}
    for pn, pt in table.get("profile", {}).items():
        if pn == ALL_PROFILE:
            raise PoolError(f"pool {name}: profile name {ALL_PROFILE!r} is built in")
        pstages = tuple(pt.get("stages", ()))
        if not pstages or any(s not in stages for s in pstages):
            raise PoolError(f"pool {name} profile {pn}: `stages` must name stages of the chain")
        restrict = {k: tuple(v) for k, v in pt.get("variants", {}).items()}
        for stage, names in restrict.items():
            if stage not in pstages:
                raise PoolError(f"pool {name} profile {pn}: restricts {stage!r}, which it does not force on")
            missing = [v for v in names if v not in stages[stage].variants]
            if missing or not names:
                raise PoolError(f"pool {name} profile {pn}: unknown variants {missing} of {stage!r}")
        profiles[pn] = Profile(pn, pstages, restrict)
    parity = table.get("instance_parity")
    if parity not in (0, 1):
        raise PoolError(f"pool {name}: instance_parity must be 0 or 1")
    p_clean, lufs = table.get("p_clean"), table.get("target_lufs")
    if not _is_number(p_clean) or not 0.0 <= p_clean < 1.0 or not _is_number(lufs):
        raise PoolError(f"pool {name}: p_clean in [0, 1) and target_lufs are required")
    return Pool(name, str(table.get("description", "")), parity, float(p_clean), float(lufs), stages, profiles)


@functools.cache
def _load_cached(path: str) -> PoolSet:
    raw = Path(path).read_bytes()
    doc = tomllib.loads(raw.decode("utf-8"))
    if doc.get("schema") != SCHEMA:
        raise PoolError(f"{path}: schema must be {SCHEMA!r}, got {doc.get('schema')!r}")
    pools = {name: _load_pool(name, t) for name, t in doc.get("pool", {}).items()}
    if set(pools) != {"train", "heldout"}:
        raise PoolError(f"{path}: the pools are exactly 'train' and 'heldout', got {sorted(pools)}")
    return PoolSet(int(doc.get("version", 0)), pools, hashlib.sha256(raw).hexdigest(), Path(path))


def load_pools(path: str | Path | None = None) -> PoolSet:
    """Load and validate ``pools.toml`` (or another file of the same schema)."""
    return _load_cached(str(path or POOLS_PATH))
