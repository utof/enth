"""The data manifest (lane AUG, R3b): every source of audio the augmentation or training may touch, with its licence.

``manifest.toml`` (beside this file) is data. Each source records where its licence was read, the version, the archive's
size and SHA-256 once fetched, and one *allowed use*:

* ``shipped-training``: weights that ship (or are downloaded) may train on it. The rule (docs/phase-3.md, "ML rules"):
  our own renders plus **CC0, CC BY or Apache** data and nothing else.
* ``research-only``: evaluation and local experiments; no shipped weights. NC, SA and licence-less sets, and anything
  whose terms are unverified, belong here.
* ``excluded``: not used at all (unavailable, or incompatible).

A source may split into *parts* (a folder of an archive) that carry their own licence and use; a part can tighten the
source's use, never loosen it. ``check_manifest`` returns every way the file breaks the rules (a refused licence marked
for shipped training, a missing licence URL, a fetched archive without a hash...), and ``require_shipped`` is the gate
every shipped-training pipeline calls: it raises ``LicenceError`` naming the first reference that is not allowed.

Standard library only; no network code (the one fetch script is ``ml/data/fetch.py``, run by hand).
"""

from __future__ import annotations

import functools
import hashlib
import re
import tomllib
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

MANIFEST_PATH = Path(__file__).with_name("manifest.toml")
SCHEMA = "enth.data-manifest/1"
USES = ("shipped-training", "research-only", "excluded")
ROLES = ("augmentation", "real-audio", "heldout-eval", "own")
TOOL_USAGE = ("subprocess", "excluded")
_RANK = {"excluded": 0, "research-only": 1, "shipped-training": 2}
_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_HEX64 = re.compile(r"^[0-9a-f]{64}$")


class ManifestError(ValueError):
    """``manifest.toml`` (or a request against it) is invalid."""


class LicenceError(ManifestError):
    """A reference is not allowed for shipped-model training."""


# --- licences ------------------------------------------------------------------------------------------------------

# The licences shipped weights may train on, as the owner's rule reads (docs/phase-3.md): CC0, CC BY, Apache.
DEFAULT_SHIPPED_LICENCES = ("CC0-1.0", "CC-BY-3.0", "CC-BY-4.0", "Apache-2.0")
# Words that make a licence unusable for shipped training whatever else it says.
_REFUSED_WORDS = ("unknown", "unverified", "unlicensed", "unlicenced", "none", "not stated", "unstated", "research")
_TOKEN_SPLIT = re.compile(r"[\s_/,()+]+|-")


def canonical_licence(spec: str) -> str:
    """``"CC BY 4.0"`` -> ``"CC-BY-4.0"``; case and separators are normalised, nothing else."""
    return re.sub(r"[\s_]+", "-", spec.strip()).upper()


def licence_flags(spec: str) -> set[str]:
    """The restricting terms a licence string names: any of ``NC``, ``SA``, ``ND``, ``SAMPLING`` (Sampling+)."""
    up = spec.upper()
    tokens = {t for t in _TOKEN_SPLIT.split(up) if t}
    flags = {t for t in tokens if t in ("NC", "SA", "ND")}
    if "SAMPLING" in up:
        flags.add("SAMPLING")
    return flags


def licence_refusal(spec: str, allowed: Iterable[str] = DEFAULT_SHIPPED_LICENCES) -> str | None:
    """Why ``spec`` may not train shipped weights, or None when it may.

    Only single licences from ``allowed`` pass. ``AND`` needs every part to pass; ``OR`` is refused (record the one
    licence you rely on).
    """
    if not isinstance(spec, str) or not spec.strip():
        return "no licence is recorded"
    low = spec.lower()
    for w in _REFUSED_WORDS:
        if re.search(rf"\b{re.escape(w)}\b", low):
            return f"licence {spec!r} is {w!r}, not a CC0, CC BY or Apache licence"
    if re.search(r"\sOR\s", spec):
        return f"licence {spec!r} is an OR expression: record the one licence relied on"
    if re.search(r"\sAND\s", spec):
        for part in re.split(r"\sAND\s", spec):
            why = licence_refusal(part, allowed)
            if why:
                return why
        return None
    flags = licence_flags(spec)
    if flags:
        return f"licence {spec!r} carries {'/'.join(sorted(flags))} terms; shipped weights train on CC0, CC BY or Apache only"
    allowed_canon = {canonical_licence(a) for a in allowed}
    if canonical_licence(spec) not in allowed_canon:
        return f"licence {spec!r} is not one of {sorted(allowed_canon)}"
    return None


# --- the model -----------------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Archive:
    url: str
    bytes: int
    sha256: str
    fetched: bool
    fetched_on: str
    local: str  # path under the data root
    extract_to: str  # directory under the data root (empty: used as is)
    mirrors: tuple[str, ...]
    note: str


@dataclass(frozen=True)
class Part:
    id: str
    path: str  # directory inside the extracted archive
    prefix: str  # only files whose name starts with this (parts that share a directory)
    kind: str
    licence: str
    use: str
    reason: str
    files: int
    index_sha256: str  # SHA-256 of the sorted "relpath<TAB>sha256<TAB>bytes" index lines (sources.index_text)
    note: str


@dataclass(frozen=True)
class Source:
    id: str
    name: str
    kind: str
    role: str
    url: str
    version: str
    licence: str
    licence_url: str
    licence_read_on: str
    licence_evidence: str
    use: str
    reason: str
    attribution: str
    obligations: tuple[str, ...]
    archive: Archive | None
    parts: Mapping[str, Part]
    note: str


@dataclass(frozen=True)
class Tool:
    id: str
    name: str
    licence: str
    version: str
    usage: str
    url: str
    evidence: str
    note: str


@dataclass(frozen=True)
class Manifest:
    version: int
    sha256: str
    path: Path
    local_root: str
    shipped_licences: tuple[str, ...]
    sources: Mapping[str, Source]
    tools: Mapping[str, Tool]

    def split_ref(self, ref: str) -> tuple[Source, Part | None]:
        """``"openslr-28/sim-small"`` -> (source, part); ``"nsynth"`` -> (source, None)."""
        sid, _, pid = ref.partition("/")
        try:
            src = self.sources[sid]
        except KeyError:
            raise ManifestError(f"unknown source {sid!r}; the manifest has {sorted(self.sources)}") from None
        if not pid:
            return src, None
        try:
            return src, src.parts[pid]
        except KeyError:
            raise ManifestError(f"source {sid!r} has no part {pid!r}; it has {sorted(src.parts)}") from None

    def effective_use(self, ref: str) -> str:
        """The allowed use of a reference: a part's use (never above its source's), or the least of a source's parts."""
        src, part = self.split_ref(ref)
        if part is not None:
            return part.use if _RANK[part.use] <= _RANK[src.use] else src.use
        if src.parts:
            return min((self.effective_use(f"{src.id}/{p}") for p in src.parts), key=_RANK.__getitem__)
        return src.use

    def licence_of(self, ref: str) -> str:
        src, part = self.split_ref(ref)
        return part.licence if part is not None and part.licence else src.licence

    def role_of(self, ref: str) -> str:
        return self.split_ref(ref)[0].role

    def all_refs(self) -> list[str]:
        out: list[str] = []
        for sid, src in self.sources.items():
            out.append(sid)
            out.extend(f"{sid}/{pid}" for pid in src.parts)
        return out

    def shipped_refs(self) -> list[str]:
        """Every reference (source or part) that may train shipped weights."""
        return [r for r in self.all_refs() if self.effective_use(r) == "shipped-training"]


def _s(table: Mapping[str, Any], key: str, default: str = "") -> str:
    v = table.get(key, default)
    return v if isinstance(v, str) else str(v)


def _load_archive(table: Mapping[str, Any]) -> Archive:
    return Archive(
        url=_s(table, "url"),
        bytes=int(table.get("bytes", 0)),
        sha256=_s(table, "sha256"),
        fetched=bool(table.get("fetched", False)),
        fetched_on=_s(table, "fetched_on"),
        local=_s(table, "local"),
        extract_to=_s(table, "extract_to"),
        mirrors=tuple(table.get("mirrors", ())),
        note=_s(table, "note"),
    )


def _load_source(table: Mapping[str, Any]) -> Source:
    parts = {}
    for pt in table.get("part", ()):
        p = Part(
            id=_s(pt, "id"),
            path=_s(pt, "path"),
            prefix=_s(pt, "prefix"),
            kind=_s(pt, "kind"),
            licence=_s(pt, "licence"),
            use=_s(pt, "use"),
            reason=_s(pt, "reason"),
            files=int(pt.get("files", 0)),
            index_sha256=_s(pt, "index_sha256"),
            note=_s(pt, "note"),
        )
        if p.id in parts:
            raise ManifestError(f"source {table.get('id')!r}: part {p.id!r} is listed twice")
        parts[p.id] = p
    archive = _load_archive(table["archive"]) if "archive" in table else None
    return Source(
        id=_s(table, "id"),
        name=_s(table, "name"),
        kind=_s(table, "kind"),
        role=_s(table, "role"),
        url=_s(table, "url"),
        version=_s(table, "version"),
        licence=_s(table, "licence"),
        licence_url=_s(table, "licence_url"),
        licence_read_on=_s(table, "licence_read_on"),
        licence_evidence=_s(table, "licence_evidence"),
        use=_s(table, "use"),
        reason=_s(table, "reason"),
        attribution=_s(table, "attribution"),
        obligations=tuple(table.get("obligations", ())),
        archive=archive,
        parts=parts,
        note=_s(table, "note"),
    )


def _load_tool(table: Mapping[str, Any]) -> Tool:
    return Tool(
        id=_s(table, "id"),
        name=_s(table, "name"),
        licence=_s(table, "licence"),
        version=_s(table, "version"),
        usage=_s(table, "usage"),
        url=_s(table, "url"),
        evidence=_s(table, "evidence"),
        note=_s(table, "note"),
    )


@functools.cache
def _load_cached(path: str) -> Manifest:
    raw = Path(path).read_bytes()
    return parse_manifest(raw.decode("utf-8"), Path(path), hashlib.sha256(raw).hexdigest())


def parse_manifest(text: str, path: Path | None = None, sha256: str | None = None) -> Manifest:
    """Parse manifest text (the tests build bad manifests this way). Structure errors raise; rule breaks are
    returned by ``check_manifest``."""
    doc = tomllib.loads(text)
    if doc.get("schema") != SCHEMA:
        raise ManifestError(f"schema must be {SCHEMA!r}, got {doc.get('schema')!r}")
    policy = doc.get("policy", {})
    shipped = tuple(policy.get("shipped_licences", DEFAULT_SHIPPED_LICENCES))
    sources: dict[str, Source] = {}
    for t in doc.get("source", ()):
        s = _load_source(t)
        if not s.id:
            raise ManifestError("a source has no id")
        if s.id in sources:
            raise ManifestError(f"source {s.id!r} is listed twice")
        sources[s.id] = s
    tools: dict[str, Tool] = {}
    for t in doc.get("tool", ()):
        tl = _load_tool(t)
        if tl.id in tools:
            raise ManifestError(f"tool {tl.id!r} is listed twice")
        tools[tl.id] = tl
    digest = sha256 or hashlib.sha256(text.encode("utf-8")).hexdigest()
    m = Manifest(
        version=int(doc.get("version", 0)),
        sha256=digest,
        path=path or Path("<text>"),
        local_root=str(doc.get("local_root", "")),
        shipped_licences=shipped,
        sources=sources,
        tools=tools,
    )
    return m


def load_manifest(path: str | Path | None = None) -> Manifest:
    """Load ``manifest.toml`` (or another file of the same schema)."""
    return _load_cached(str(path or MANIFEST_PATH))


# --- the rules -----------------------------------------------------------------------------------------------------

def check_manifest(m: Manifest) -> list[str]:
    """Every way ``m`` breaks the rules; empty when it is sound. Pure data, no files are read."""
    problems: list[str] = []
    if m.version < 1:
        problems.append("version must be 1 or more")
    for sid, s in m.sources.items():
        where = f"source {sid}"
        for f in ("name", "kind", "url", "version", "licence", "licence_url", "licence_read_on", "licence_evidence"):
            if not getattr(s, f).strip():
                problems.append(f"{where}: `{f}` is required (the licence is read from the source and recorded)")
        if s.role not in ROLES:
            problems.append(f"{where}: role {s.role!r} is not one of {ROLES}")
        if s.use not in USES:
            problems.append(f"{where}: use {s.use!r} is not one of {USES}")
            continue
        if s.licence_read_on and not _DATE.match(s.licence_read_on):
            problems.append(f"{where}: licence_read_on must be YYYY-MM-DD")
        if s.licence_url and not s.licence_url.startswith(("http://", "https://")):
            problems.append(f"{where}: licence_url must be an http(s) URL")
        if s.use != "shipped-training" and not s.reason.strip():
            problems.append(f"{where}: use {s.use!r} needs a `reason`")
        if s.use == "shipped-training":
            why = licence_refusal(s.licence, m.shipped_licences)
            if why:
                problems.append(f"{where}: marked shipped-training but {why}")
            if s.role == "heldout-eval":
                problems.append(f"{where}: a held-out evaluation collection can never be shipped-training")
            if canonical_licence(s.licence).startswith("CC-BY") and not s.attribution.strip():
                problems.append(f"{where}: a CC BY source needs an `attribution` credit line for THIRD_PARTY_NOTICES.md")
        if s.archive is not None:
            problems += _check_archive(where, s.archive)
        for pid, p in s.parts.items():
            pwhere = f"{where}/{pid}"
            if not p.id or not p.path:
                problems.append(f"{pwhere}: a part needs an id and a path")
            if p.use not in USES:
                problems.append(f"{pwhere}: use {p.use!r} is not one of {USES}")
                continue
            if _RANK[p.use] > _RANK[s.use]:
                problems.append(f"{pwhere}: use {p.use!r} is looser than its source's {s.use!r}; a part can only tighten it")
            if p.use != "shipped-training" and not p.reason.strip():
                problems.append(f"{pwhere}: use {p.use!r} needs a `reason`")
            if p.use == "shipped-training":
                lic = p.licence or s.licence
                why = licence_refusal(lic, m.shipped_licences)
                if why:
                    problems.append(f"{pwhere}: marked shipped-training but {why}")
            if p.index_sha256 and not _HEX64.match(p.index_sha256):
                problems.append(f"{pwhere}: index_sha256 must be 64 lowercase hex digits")
    for tid, t in m.tools.items():
        where = f"tool {tid}"
        for f in ("name", "licence", "version", "url", "evidence"):
            if not getattr(t, f).strip():
                problems.append(f"{where}: `{f}` is required")
        if t.usage not in TOOL_USAGE:
            problems.append(f"{where}: usage {t.usage!r} is not one of {TOOL_USAGE} (a tool is never imported or linked)")
    return problems


def _check_archive(where: str, a: Archive) -> list[str]:
    out: list[str] = []
    if not a.url.startswith(("http://", "https://")):
        out.append(f"{where}: archive url must be an http(s) URL")
    if a.fetched:
        if not _HEX64.match(a.sha256):
            out.append(f"{where}: a fetched archive needs its sha256 (64 lowercase hex digits)")
        if a.bytes <= 0:
            out.append(f"{where}: a fetched archive needs its size in bytes")
        if not _DATE.match(a.fetched_on):
            out.append(f"{where}: a fetched archive needs `fetched_on` (YYYY-MM-DD)")
        if not a.local:
            out.append(f"{where}: a fetched archive needs `local`, its path under the data root")
    elif a.sha256 and not _HEX64.match(a.sha256):
        out.append(f"{where}: sha256 must be 64 lowercase hex digits (or empty before the first fetch)")
    return out


def require_shipped(refs: Sequence[str] | Iterable[str], m: Manifest) -> None:
    """The gate for shipped-model training: raise ``LicenceError`` unless every reference is ``shipped-training``.

    A reference may be a source (``"nsynth"``) or a part (``"openslr-28/sim-small"``); a source with parts passes only
    when every part does. Unknown references raise too.
    """
    for ref in refs:
        use = m.effective_use(ref)
        src, part = m.split_ref(ref)
        lic = m.licence_of(ref)
        if use != "shipped-training":
            reason = (part.reason if part is not None and part.reason else src.reason) or "see the manifest"
            raise LicenceError(f"{ref}: use is {use!r} ({lic}); shipped weights cannot train on it. {reason}")
        why = licence_refusal(lic, m.shipped_licences)
        if why:
            raise LicenceError(f"{ref}: {why}")
        if src.role == "heldout-eval":
            raise LicenceError(f"{ref}: a held-out evaluation collection can never train")


def refused_for_shipped(m: Manifest) -> dict[str, str]:
    """Every reference that carries NC, SA or ND terms (or no licence) with the reason it is kept out of shipped training."""
    out: dict[str, str] = {}
    for ref in m.all_refs():
        lic = m.licence_of(ref)
        if licence_refusal(lic, m.shipped_licences) is not None:
            out[ref] = lic
    return out
