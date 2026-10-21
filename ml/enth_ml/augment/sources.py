"""Audio sources on disk: a hashed index per manifest reference, a train/dev split by path hash, and GR3's exclusions.

Recorded collections (room impulse responses, background noises, music beds) are fetched once, by hand, into a data
root outside the repository (``ml/data/fetch.py``; ``ENTH_DATA_DIR`` or the manifest's ``local_root``). This module reads
them without trusting them: it follows no symlink, hashes every file into an *index* (``relpath<TAB>sha256<TAB>bytes``,
sorted) and re-checks a file's hash when the augmentation picks it, so an index, not the file system's order, decides
what a seed selects.

* **Split.** ``partition_of`` puts a small share of each reference's files in ``dev`` by a hash of the path, so the
  validation augmentation draws impulse responses and noises training never saw. (The *held-out* collections of GR3, the
  MIT IR Survey and DEMAND, are other sources altogether: see ``manifest.toml``.)
* **GR3's exclusions.** ``content/gr3/exclude-*.txt`` list what training must never contain. ``Exclusions`` reads them
  and answers for an audio file (Freesound id, pack, SHA-256, path) and for a seed (the GR3 seed block).

Standard library only.
"""

from __future__ import annotations

import fnmatch
import hashlib
import os
import re
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

from .manifest import Manifest, ManifestError, load_manifest

REPO_ROOT = Path(__file__).resolve().parents[3]
GR3_DIR = REPO_ROOT / "content" / "gr3"
AUDIO_SUFFIXES = (".wav", ".flac", ".ogg", ".mp3", ".opus")
# The share of files, by hash of path, that the validation split gets.
DEV_PERCENT = 5
# GR3's seed block (content/gr3/exclude-*.txt: `seed-range` and `degrade-seed-range`): no augmentation seed lies in it.
# `tests/test_augment_disjoint.py` checks this constant against the files.
GR3_SEED_RANGES: tuple[tuple[int, int], ...] = ((770000000000, 770999999999),)


class SourceError(RuntimeError):
    """A source is missing, changed on disk, or excluded."""


@dataclass(frozen=True)
class IndexEntry:
    relpath: str
    sha256: str
    bytes: int

    def line(self) -> str:
        return f"{self.relpath}\t{self.sha256}\t{self.bytes}\n"


def index_text(entries: Iterable[IndexEntry]) -> str:
    """The canonical index text: sorted by path (bytewise), one ``relpath<TAB>sha256<TAB>bytes`` line each."""
    return "".join(e.line() for e in sorted(entries, key=lambda e: e.relpath.encode("utf-8")))


def index_digest(entries: Iterable[IndexEntry]) -> str:
    """The SHA-256 the manifest records as a part's ``index_sha256``."""
    return hashlib.sha256(index_text(entries).encode("utf-8")).hexdigest()


def parse_index(text: str) -> list[IndexEntry]:
    out = []
    for n, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) != 3 or not re.fullmatch(r"[0-9a-f]{64}", parts[1]) or not parts[2].isdigit():
            raise SourceError(f"index line {n}: expected relpath<TAB>sha256<TAB>bytes, got {line[:80]!r}")
        out.append(IndexEntry(parts[0], parts[1], int(parts[2])))
    return out


def file_sha256(path: str | Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def build_index(
    root: str | Path, subdir: str = "", suffixes: tuple[str, ...] = AUDIO_SUFFIXES, prefix: str = ""
) -> list[IndexEntry]:
    """Hash every audio file under ``root/subdir`` whose name starts with ``prefix``.

    Symlinks are never followed; paths are relative to ``root/subdir``.
    """
    base = Path(root) / subdir
    if not base.is_dir():
        raise SourceError(f"{base}: not a directory")
    entries = []
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        dirnames.sort()
        for name in sorted(filenames):
            full = Path(dirpath) / name
            if full.is_symlink() or not full.is_file() or not name.lower().endswith(suffixes):
                continue
            if prefix and not name.startswith(prefix):
                continue
            rel = full.relative_to(base).as_posix()
            entries.append(IndexEntry(rel, file_sha256(full), full.stat().st_size))
    return sorted(entries, key=lambda e: e.relpath.encode("utf-8"))


def partition_of(ref: str, relpath: str, dev_percent: int = DEV_PERCENT) -> str:
    """``"train"`` or ``"dev"``: a stable split of one reference's files by a hash of the path."""
    h = hashlib.sha256(f"enth/augment/partition/v1:{ref}:{relpath}".encode()).digest()
    return "dev" if int.from_bytes(h[:4], "big") % 100 < dev_percent else "train"


# --- GR3's exclusions ----------------------------------------------------------------------------------------------

_KNOWN_TYPES = {
    "seed-range", "stream", "code", "fingerprint", "preset-stem", "preset-name", "file-sha256", "path",
    "freesound-id", "freesound-pack", "freesound-uploader", "audio-sha256",
    "degrade-pool", "degrade-variant", "degrade-profile", "degrade-seed-range", "pools-sha256",
}


@dataclass(frozen=True)
class Exclusions:
    """What ``content/gr3/exclude-*.txt`` say, parsed (a newer list with an unknown type is refused, never half-read)."""

    entries: tuple[tuple[str, str], ...]
    sha256: str  # of the files, in name order, so a run can record which lists it checked against

    def values(self, kind: str) -> list[str]:
        return [v for k, v in self.entries if k == kind]

    def seed_ranges(self) -> list[tuple[int, int]]:
        out = []
        for kind in ("seed-range", "degrade-seed-range"):
            for v in self.values(kind):
                lo, _, hi = v.partition("..")
                out.append((int(lo), int(hi)))
        return sorted(set(out))

    def seed_excluded(self, seed: int) -> bool:
        return any(lo <= seed <= hi for lo, hi in self.seed_ranges())

    def audio_excluded(
        self,
        *,
        freesound_id: int | None = None,
        freesound_pack: int | None = None,
        sha256: str | None = None,
        path: str | Path | None = None,
    ) -> str | None:
        """The first rule an audio file breaks, or None. Pass whatever the source records about the file."""
        if freesound_id is not None and str(freesound_id) in self.values("freesound-id"):
            return f"freesound-id {freesound_id}"
        if freesound_pack is not None and str(freesound_pack) in self.values("freesound-pack"):
            return f"freesound-pack {freesound_pack}"
        if sha256 is not None and sha256 in self.values("audio-sha256"):
            return f"audio-sha256 {sha256[:12]}"
        if path is not None:
            p = os.path.realpath(os.path.expanduser(str(path)))
            for pattern in self.values("path"):
                pat = os.path.expanduser(pattern)
                if pat.startswith("~") or not os.path.isabs(pat):
                    continue  # repo-relative patterns (content/gr3/**) concern synth files, not audio
                if fnmatch.fnmatch(p, pat.replace("**", "*")):
                    return f"path {pattern}"
        return None


def load_exclusions(directory: str | Path | None = None) -> Exclusions:
    """Read the three ``exclude-*.txt`` files of ``content/gr3`` (or another directory)."""
    d = Path(directory) if directory is not None else GR3_DIR
    h = hashlib.sha256()
    entries: list[tuple[str, str]] = []
    for name in ("exclude-degraded.txt", "exclude-real.txt", "exclude-synth.txt"):
        f = d / name
        if not f.is_file():
            raise SourceError(f"{f}: GR3's exclusion list is missing")
        raw = f.read_bytes()
        h.update(name.encode() + b"\0" + hashlib.sha256(raw).digest())
        for line in raw.decode("utf-8").splitlines():
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            kind, _, value = line.partition(" ")
            if kind not in _KNOWN_TYPES:
                raise SourceError(f"{f.name}: unknown entry type {kind!r}; the list is newer than this reader")
            entries.append((kind, value.strip()))
    return Exclusions(tuple(entries), h.hexdigest())


def seed_in_gr3_block(seed: int, ranges: Iterable[tuple[int, int]] = GR3_SEED_RANGES) -> bool:
    return any(lo <= seed <= hi for lo, hi in ranges)


# --- the store -----------------------------------------------------------------------------------------------------


def default_data_root(manifest: Manifest) -> Path:
    env = os.environ.get("ENTH_DATA_DIR")
    return Path(os.path.expanduser(env if env else manifest.local_root))


class SourceStore:
    """Where each manifest reference lives on disk, and its hashed index.

    ``dirs`` maps a reference to an already extracted directory (tests build small ones); otherwise a part lives at
    ``<data root>/<archive.extract_to>/<part.path>`` and its index is read from ``<data root>/_index/<ref>.tsv`` (written
    by ``python -m enth_ml.augment index``), or built in memory when there is no index file.
    """

    def __init__(
        self,
        manifest: Manifest | None = None,
        data_root: str | Path | None = None,
        dirs: Mapping[str, str | Path] | None = None,
        dev_percent: int = DEV_PERCENT,
    ) -> None:
        self.manifest = manifest or load_manifest()
        self.data_root = Path(data_root) if data_root is not None else default_data_root(self.manifest)
        self.dirs = {k: Path(v) for k, v in (dirs or {}).items()}
        self.dev_percent = dev_percent
        self._index: dict[str, list[IndexEntry]] = {}

    def directory(self, ref: str) -> Path:
        if ref in self.dirs:
            return self.dirs[ref]
        src, part = self.manifest.split_ref(ref)
        base = self.data_root / (src.archive.extract_to if src.archive and src.archive.extract_to else src.id)
        return base / part.path if part is not None else base

    def index_path(self, ref: str) -> Path:
        return self.data_root / "_index" / (ref.replace("/", ".") + ".tsv")

    def _prefix(self, ref: str) -> str:
        _, part = self.manifest.split_ref(ref) if ref not in self.dirs else (None, None)
        return part.prefix if part is not None else ""

    def index(self, ref: str) -> list[IndexEntry]:
        if ref not in self._index:
            if ref in self.dirs:
                self._index[ref] = build_index(self.dirs[ref])
            elif self.index_path(ref).is_file():
                self._index[ref] = parse_index(self.index_path(ref).read_text(encoding="utf-8"))
            else:
                d = self.directory(ref)
                self._index[ref] = build_index(d, prefix=self._prefix(ref)) if d.is_dir() else []
        return self._index[ref]

    def available(self, ref: str) -> bool:
        try:
            return bool(self.index(ref)) and self.directory(ref).is_dir()
        except (ManifestError, SourceError):
            return False

    def entries(self, ref: str, partition: str = "train") -> list[IndexEntry]:
        if partition not in ("train", "dev"):
            raise ValueError("partition is 'train' or 'dev'")
        return [e for e in self.index(ref) if partition_of(ref, e.relpath, self.dev_percent) == partition]

    def pick(self, ref: str, u: float, partition: str = "train") -> IndexEntry:
        """The file ``u`` in [0, 1) selects among ``ref``'s files of ``partition``."""
        entries = self.entries(ref, partition)
        if not entries:
            raise SourceError(f"{ref}: no {partition} files (not fetched, or not indexed)")
        return entries[min(int(u * len(entries)), len(entries) - 1)]

    def path_of(self, ref: str, entry: IndexEntry, verify: bool = True) -> Path:
        """The file's path; its SHA-256 is checked against the index unless ``verify`` is False."""
        d = self.directory(ref)
        path = d / entry.relpath
        if path.is_symlink() or not path.is_file():
            raise SourceError(f"{ref}: {entry.relpath} is missing or not a regular file")
        if not os.path.realpath(path).startswith(os.path.realpath(d) + os.sep):
            raise SourceError(f"{ref}: {entry.relpath} escapes its directory")
        if verify and file_sha256(path) != entry.sha256:
            raise SourceError(f"{ref}: {entry.relpath} does not match its indexed SHA-256")
        return path

    def write_index(self, ref: str) -> tuple[Path, str, int]:
        """Hash ``ref``'s directory, write the index under the data root and return (path, digest, file count)."""
        entries = build_index(self.directory(ref), prefix=self._prefix(ref))
        out = self.index_path(ref)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(index_text(entries), encoding="utf-8")
        self._index[ref] = entries
        return out, index_digest(entries), len(entries)
